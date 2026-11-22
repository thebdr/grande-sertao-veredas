//! The emulated EtherCAT segment: SubDevices answering a master's frames on a raw network
//! interface, so an EtherCAT master meets its segment on the bench before it meets the plant.
//!
//! An [`EthercatConfig`] (read from a bench file) names the interface and the SubDevices in the
//! order the frames pass them: each its identity (vendor, product, revision, serial, name, alias)
//! and its process data (the output and input entries a master maps). [`start`] runs the segment on
//! a thread of its own (Linux, through aranha's raw link): every EtherCAT frame that arrives is
//! passed through the SubDevices in order, each an ESC whose registers answer every command
//! (auto-increment, configured, broadcast, logical through its FMMUs, read-multiple-write) with the
//! working counter the standard gives, its AL state machine, its SII EEPROM read through the
//! registers, its distributed clock latched and offset; then the frame goes back on the same wire,
//! as from the segment's far end. It is functional, never exact in time: a frame is answered when
//! the thread reads it (`docs/briefing/review.md`, entry 27). The handle reads the outputs a master
//! wrote, sets the inputs it reads, forces a device's AL state and silences the segment.

pub mod esc;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::Deserialize;

use esc::{Esc, Identity, PdoEntry};

/// One process-data entry, as the bench file writes it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntryConfig {
    pub index: u16,
    pub subindex: u8,
    pub bits: u8,
}

/// One SubDevice.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceConfig {
    /// The name the bench's handle reads and sets it by.
    pub name: String,
    #[serde(default)]
    pub vendor_id: u32,
    #[serde(default)]
    pub product_code: u32,
    #[serde(default)]
    pub revision: u32,
    #[serde(default)]
    pub serial: u32,
    /// The name its EEPROM gives a master.
    #[serde(default = "default_product_name")]
    pub product_name: String,
    #[serde(default)]
    pub alias: u16,
    #[serde(default)]
    pub outputs: Vec<EntryConfig>,
    #[serde(default)]
    pub inputs: Vec<EntryConfig>,
}

fn default_product_name() -> String {
    "veredas bench terminal".into()
}

/// The segment, as the bench file declares it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EthercatConfig {
    /// The network interface the master's frames arrive on, by name.
    pub interface: String,
    /// The SubDevices, in the order the frames pass them.
    #[serde(default)]
    pub devices: Vec<DeviceConfig>,
}

#[derive(Debug, thiserror::Error)]
pub enum EthercatError {
    #[error("{0}")]
    Config(String),
    #[error("the interface: {0}")]
    Link(String),
    #[error("spawn the segment thread: {0}")]
    Thread(std::io::Error),
    #[error("an EtherCAT segment runs on Linux only")]
    Unsupported,
    #[error("no device `{0}`")]
    NoDevice(String),
    #[error("device `{device}` holds {size} byte(s) of inputs, not {given}")]
    WrongSize { device: String, size: usize, given: usize },
}

/// The most process data a device holds each way, bytes.
pub const MAX_PROCESS_DATA: usize = 0x400;

fn bytes_of(entries: &[EntryConfig]) -> usize {
    entries.iter().map(|e| e.bits as usize).sum::<usize>().div_ceil(8)
}

/// Validate a configuration before anything opens, naming what is wrong.
pub fn validate(cfg: &EthercatConfig) -> Result<(), EthercatError> {
    let bad = |m: String| EthercatError::Config(m);
    if cfg.interface.trim().is_empty() {
        return Err(bad("ethercat.interface is empty".into()));
    }
    if cfg.devices.is_empty() {
        return Err(bad("ethercat.devices: a segment holds at least one device".into()));
    }
    for (i, d) in cfg.devices.iter().enumerate() {
        let place = format!("ethercat.devices[{i}]");
        if d.name.trim().is_empty() || d.name != d.name.trim() {
            return Err(bad(format!("{place}: a device's name is not empty and has no surrounding whitespace")));
        }
        if cfg.devices[..i].iter().any(|e| e.name == d.name) {
            return Err(bad(format!("{place}: two devices are named `{}`", d.name)));
        }
        let place = format!("{place} (`{}`)", d.name);
        if d.product_name.is_empty() || d.product_name.len() > 255 || !d.product_name.is_ascii() {
            return Err(bad(format!("{place}.product_name: 1 to 255 ASCII characters")));
        }
        for (kind, entries) in [("outputs", &d.outputs), ("inputs", &d.inputs)] {
            if let Some((j, e)) = entries.iter().enumerate().find(|(_, e)| e.bits == 0 || e.bits > 64) {
                return Err(bad(format!("{place}.{kind}[{j}]: {} bit(s), not 1 to 64", e.bits)));
            }
            if bytes_of(entries) > MAX_PROCESS_DATA {
                return Err(bad(format!("{place}.{kind}: {} bytes, over {MAX_PROCESS_DATA}", bytes_of(entries))));
            }
        }
    }
    Ok(())
}

/// What the segment reports.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EthercatStatus {
    /// Frames passed through the segment and sent back.
    pub frames: u64,
    /// Datagrams processed.
    pub datagrams: u64,
    /// Frames dropped while the segment was silent, or unreadable.
    pub dropped: u64,
    pub unreadable: u64,
}

struct State {
    segment: Vec<Esc>,
    status: EthercatStatus,
}

#[allow(dead_code)]
struct Shared {
    state: Mutex<State>,
    silent: AtomicBool,
    stop: AtomicBool,
    started: std::time::Instant,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// A running segment. Dropping it stops it.
pub struct EthercatHandle {
    interface: String,
    names: Vec<String>,
    sizes: Vec<(usize, usize)>,
    shared: Arc<Shared>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl EthercatHandle {
    pub fn interface(&self) -> &str {
        &self.interface
    }

    fn index(&self, device: &str) -> Result<usize, EthercatError> {
        self.names.iter().position(|n| n == device).ok_or_else(|| EthercatError::NoDevice(device.to_string()))
    }

    /// The outputs a master wrote into a device's process RAM.
    pub fn outputs(&self, device: &str) -> Result<Vec<u8>, EthercatError> {
        let i = self.index(device)?;
        Ok(self.shared.lock().segment[i].read(esc::OUTPUTS_AT, self.sizes[i].0))
    }

    /// The inputs a master reads from a device, set from the plant side, all of them.
    pub fn set_inputs(&self, device: &str, bytes: &[u8]) -> Result<(), EthercatError> {
        let i = self.index(device)?;
        let (out, inp) = self.sizes[i];
        if bytes.len() != inp {
            return Err(EthercatError::WrongSize { device: device.to_string(), size: inp, given: bytes.len() });
        }
        let at = esc::inputs_at(out) as usize;
        self.shared.lock().segment[i].mem[at..at + inp].copy_from_slice(bytes);
        Ok(())
    }

    /// A device's AL state (1 INIT, 2 PRE-OP, 4 SAFE-OP, 8 OP), its error flag apart.
    pub fn al_state(&self, device: &str) -> Result<(u8, bool), EthercatError> {
        let i = self.index(device)?;
        let state = self.shared.lock();
        let s = state.segment[i].mem[esc::reg::AL_STATUS as usize];
        Ok((s & 0x0F, s & esc::state::ERROR_FLAG != 0))
    }

    /// The configured station address a master gave a device.
    pub fn station(&self, device: &str) -> Result<u16, EthercatError> {
        let i = self.index(device)?;
        Ok(self.shared.lock().segment[i].station())
    }

    /// A device falls back to SAFE-OP with an error and this AL status code, as a device whose
    /// watchdog expired or whose sync was lost does; the master acknowledges it to recover.
    pub fn fault(&self, device: &str, code: u16) -> Result<(), EthercatError> {
        let i = self.index(device)?;
        let mut state = self.shared.lock();
        let esc = &mut state.segment[i];
        esc.mem[esc::reg::AL_STATUS as usize] = esc::state::SAFE_OP | esc::state::ERROR_FLAG;
        esc.mem[esc::reg::AL_STATUS_CODE as usize..esc::reg::AL_STATUS_CODE as usize + 2].copy_from_slice(&code.to_le_bytes());
        Ok(())
    }

    /// Answer nothing (true), as a segment whose cable was pulled; false answers again.
    pub fn set_silent(&self, silent: bool) {
        self.shared.silent.store(silent, Ordering::SeqCst);
    }

    pub fn status(&self) -> EthercatStatus {
        self.shared.lock().status.clone()
    }

    fn stop_and_join(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }

    pub fn stop(mut self) {
        self.stop_and_join();
    }
}

impl Drop for EthercatHandle {
    fn drop(&mut self) {
        self.stop_and_join();
    }
}

/// The segment's ESCs, each with its EEPROM image and a local clock of its own (origins 1 µs
/// apart, as devices powered at slightly different instants).
fn segment(cfg: &EthercatConfig) -> Vec<Esc> {
    let n = cfg.devices.len();
    cfg.devices
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let id = Identity { vendor_id: d.vendor_id, product_code: d.product_code, revision: d.revision, serial: d.serial, name: d.product_name.clone(), alias: d.alias };
            let entries = |e: &[EntryConfig]| e.iter().map(|x| PdoEntry { index: x.index, subindex: x.subindex, bits: x.bits }).collect::<Vec<_>>();
            let mut esc = Esc::new(esc::sii_image(&id, &entries(&d.outputs), &entries(&d.inputs)), i as u64 * 1_000);
            esc.mem[esc::reg::STATION_ALIAS as usize..esc::reg::STATION_ALIAS as usize + 2].copy_from_slice(&d.alias.to_le_bytes());
            esc.set_link(i + 1 < n);
            esc
        })
        .collect()
}

/// Start the segment on its interface (Linux only).
#[cfg(not(target_os = "linux"))]
pub fn start(cfg: &EthercatConfig) -> Result<EthercatHandle, EthercatError> {
    validate(cfg)?;
    Err(EthercatError::Unsupported)
}

/// Start the segment on its interface (Linux only). Returns once the link is open.
#[cfg(target_os = "linux")]
pub fn start(cfg: &EthercatConfig) -> Result<EthercatHandle, EthercatError> {
    use veredas_aranha::wire::raw::{Options, RawLink};
    validate(cfg)?;
    let link = RawLink::open_with(&cfg.interface, Options { ethertype: Some(esc::ETHERTYPE), promiscuous: true, outgoing: false })
        .map_err(|e| EthercatError::Link(e.to_string()))?;
    let shared = Arc::new(Shared {
        state: Mutex::new(State { segment: segment(cfg), status: EthercatStatus::default() }),
        silent: AtomicBool::new(false),
        stop: AtomicBool::new(false),
        started: std::time::Instant::now(),
    });
    let worker = Arc::clone(&shared);
    let thread = std::thread::Builder::new().name("maquinar-ethercat".into()).spawn(move || run(link, worker)).map_err(EthercatError::Thread)?;
    Ok(EthercatHandle {
        interface: cfg.interface.clone(),
        names: cfg.devices.iter().map(|d| d.name.clone()).collect(),
        sizes: cfg.devices.iter().map(|d| (bytes_of(&d.outputs), bytes_of(&d.inputs))).collect(),
        shared,
        thread: Some(thread),
    })
}

#[cfg(target_os = "linux")]
fn run(link: veredas_aranha::wire::raw::RawLink, shared: Arc<Shared>) {
    use std::time::Duration;
    while !shared.stop.load(Ordering::SeqCst) {
        let Ok(Some(received)) = link.recv(Duration::from_millis(5)) else { continue };
        let frame = received.bytes;
        // The Ethernet header, untagged: an EtherCAT master sends none.
        if frame.len() < 14 + 2 || u16::from_be_bytes([frame[12], frame[13]]) != esc::ETHERTYPE {
            continue;
        }
        if shared.silent.load(Ordering::SeqCst) {
            shared.lock().status.dropped += 1;
            continue;
        }
        let payload = &frame[14..];
        let Some(mut datagrams) = esc::parse(payload) else {
            shared.lock().status.unreadable += 1;
            continue;
        };
        let now = shared.started.elapsed().as_nanos() as u64;
        let out = {
            let mut state = shared.lock();
            esc::process(&mut state.segment, &mut datagrams, now);
            state.status.frames += 1;
            state.status.datagrams += datagrams.len() as u64;
            let mut out = frame[..14].to_vec();
            // Back from the segment: the source's locally administered bit set, as the first
            // device sets it on the way back.
            out[6] |= 0x02;
            out.extend(esc::encode(&datagrams, payload.len()));
            out
        };
        let _ = link.send(&out);
    }
}
