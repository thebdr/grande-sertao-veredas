//! The PROFINET device: a device's discovery and naming on a raw network interface, so a
//! controller or a scanner meets its device on the bench before it meets the plant.
//!
//! A [`ProfinetConfig`] (read from a bench file) gives the device's identity as DCP states it (its
//! station name, which may be empty, as a device not yet named; its alias; its vendor's text,
//! vendor and device ids; its IP suite) and its port's name. [`start`] runs it on a thread of its
//! own on the named interface, through aranha's raw link and codecs (`docs/questions.md` Q5): it
//! announces itself with a DCP Hello, answers a DCP Identify whose filter it matches (all, its
//! station name, its alias) after a spread within the request's response delay, applies a DCP Set
//! (its name, its IP suite, the identification signal) and answers each block with the three-byte
//! control response, answers a DCP Get, and sends LLDP on its port with the PROFINET fields on an
//! interval. The handle reads the identity as it stands, sets it from the plant side, mutes the
//! device and counts what it did.

use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
#[cfg(target_os = "linux")]
use std::time::{Duration, Instant};

use serde::Deserialize;
use veredas_aranha::profinet::dcp::{self, DeviceIdentity, IpStatus};
#[cfg(target_os = "linux")]
use veredas_aranha::profinet::dcp::{Block, BlockBody, DcpPdu, IpSuite, Service, ServiceType};
#[cfg(target_os = "linux")]
use veredas_aranha::profinet::ether;
#[cfg(target_os = "linux")]
use veredas_aranha::profinet::lldp::{self, ChassisId, LldpPdu, PortId};
use veredas_aranha::profinet::mac::Mac;
#[cfg(target_os = "linux")]
use veredas_aranha::wire::raw::{Options, RawLink};
#[cfg(target_os = "linux")]
use veredas_core::rng::Pcg32;

/// The device role this bench plays: an IO device.
pub const ROLE_IO_DEVICE: u8 = 0x01;

/// DCP control response errors.
pub mod error {
    pub const NONE: u8 = 0x00;
    pub const OPTION_UNSUPPORTED: u8 = 0x01;
    pub const SUBOPTION_UNSUPPORTED: u8 = 0x02;
    pub const SET_NOT_POSSIBLE: u8 = 0x05;
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IpConfig {
    pub address: Ipv4Addr,
    pub mask: Ipv4Addr,
    #[serde(default = "unspecified")]
    pub gateway: Ipv4Addr,
}

fn unspecified() -> Ipv4Addr {
    Ipv4Addr::UNSPECIFIED
}

/// The device, as the bench file declares it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfinetConfig {
    /// The network interface it answers on, by name.
    pub interface: String,
    /// Its NameOfStation; empty for a device not yet named.
    #[serde(default)]
    pub station: String,
    /// Its AliasName; none when absent.
    #[serde(default)]
    pub alias: Option<String>,
    /// The vendor's text for the device's type (DeviceVendorValue).
    #[serde(default = "default_vendor")]
    pub vendor: String,
    #[serde(default)]
    pub vendor_id: u16,
    #[serde(default)]
    pub device_id: u16,
    /// Its IP suite; absent, 0.0.0.0 and not set.
    #[serde(default)]
    pub ip: Option<IpConfig>,
    /// A Hello at start.
    #[serde(default = "yes")]
    pub hello: bool,
    /// Milliseconds between LLDP frames; 0 sends none.
    #[serde(default = "default_lldp")]
    pub lldp_interval_ms: u64,
    /// The port's name, as LLDP's port id states it.
    #[serde(default = "default_port")]
    pub port: String,
    /// The seed of the response delay's spread; the clock when absent.
    #[serde(default)]
    pub seed: Option<u64>,
}

impl ProfinetConfig {
    /// A device on `interface` with every other key at its default.
    pub fn new(interface: &str) -> ProfinetConfig {
        ProfinetConfig {
            interface: interface.to_string(),
            station: String::new(),
            alias: None,
            vendor: default_vendor(),
            vendor_id: 0,
            device_id: 0,
            ip: None,
            hello: true,
            lldp_interval_ms: default_lldp(),
            port: default_port(),
            seed: None,
        }
    }
}

fn default_vendor() -> String {
    "veredas bench device".into()
}
fn yes() -> bool {
    true
}
fn default_lldp() -> u64 {
    5000
}
fn default_port() -> String {
    "port-001".into()
}

#[derive(Debug, thiserror::Error)]
pub enum ProfinetError {
    #[error("{0}")]
    Config(String),
    #[error("the interface: {0}")]
    Link(String),
    #[error("spawn the device thread: {0}")]
    Thread(std::io::Error),
    /// The device answers raw frames through aranha's link, which is Linux's.
    #[error("a PROFINET device runs on Linux only")]
    Unsupported,
}

/// Validate a configuration before anything opens.
pub fn validate(cfg: &ProfinetConfig) -> Result<(), ProfinetError> {
    let bad = |m: String| ProfinetError::Config(m);
    if cfg.interface.trim().is_empty() {
        return Err(bad("profinet.interface is empty".into()));
    }
    if !cfg.station.is_empty() {
        dcp::check_station_name(&cfg.station).map_err(|e| bad(format!("profinet.station `{}`: {e}", cfg.station)))?;
    }
    if !cfg.port.starts_with("port-") {
        return Err(bad(format!("profinet.port `{}`: a port is named port-xyz", cfg.port)));
    }
    if cfg.vendor.is_empty() || cfg.vendor.len() > 255 {
        return Err(bad("profinet.vendor: 1 to 255 characters".into()));
    }
    Ok(())
}

/// What the device reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfinetStatus {
    /// The interface's MAC, the device's.
    pub mac: Mac,
    pub hellos: u64,
    pub identify_answered: u64,
    /// Identify requests whose filter it did not match.
    pub identify_passed: u64,
    pub sets: u64,
    pub gets: u64,
    pub blinks: u64,
    pub lldp_sent: u64,
    /// DCP frames it could not read.
    pub undecodable: u64,
}

impl ProfinetStatus {
    /// Nothing done yet, by the device of this MAC.
    pub fn new(mac: Mac) -> ProfinetStatus {
        ProfinetStatus { mac, hellos: 0, identify_answered: 0, identify_passed: 0, sets: 0, gets: 0, blinks: 0, lldp_sent: 0, undecodable: 0 }
    }
}

struct State {
    identity: DeviceIdentity,
    status: ProfinetStatus,
}

struct Shared {
    state: Mutex<State>,
    muted: AtomicBool,
    stop: AtomicBool,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// A running device. Dropping it stops it (an LLDP frame with a TTL of 0 says it is leaving).
pub struct ProfinetHandle {
    pub mac: Mac,
    interface: String,
    shared: Arc<Shared>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ProfinetHandle {
    /// The interface it answers on.
    pub fn interface(&self) -> &str {
        &self.interface
    }

    pub fn identity(&self) -> DeviceIdentity {
        self.shared.lock().identity.clone()
    }

    pub fn station(&self) -> String {
        self.shared.lock().identity.name.clone().unwrap_or_default()
    }

    /// Name the device from the plant side, as an engineer's tool would.
    pub fn set_station(&self, name: &str) -> Result<(), ProfinetError> {
        if !name.is_empty() {
            dcp::check_station_name(name).map_err(|e| ProfinetError::Config(format!("`{name}`: {e}")))?;
        }
        self.shared.lock().identity.name = Some(name.to_string());
        Ok(())
    }

    /// Flag an address conflict the device detected itself, or clear it.
    pub fn set_ip_conflict(&self, conflict: bool) {
        let mut state = self.shared.lock();
        let set = state.identity.ip_status.map(|s| s.set).unwrap_or(0);
        state.identity.ip_status = Some(IpStatus { set, conflict });
    }

    /// Answer nothing (true), as a device whose DCP is off; LLDP goes on.
    pub fn set_muted(&self, muted: bool) {
        self.shared.muted.store(muted, Ordering::SeqCst);
    }

    pub fn status(&self) -> ProfinetStatus {
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

impl Drop for ProfinetHandle {
    fn drop(&mut self) {
        self.stop_and_join();
    }
}

#[cfg(target_os = "linux")]
fn identity_of(cfg: &ProfinetConfig, mac: Mac) -> DeviceIdentity {
    let (ip, ip_status) = match &cfg.ip {
        Some(ip) => (IpSuite { ip: ip.address, mask: ip.mask, gateway: ip.gateway }, IpStatus { set: 1, conflict: false }),
        None => (IpSuite { ip: Ipv4Addr::UNSPECIFIED, mask: Ipv4Addr::UNSPECIFIED, gateway: Ipv4Addr::UNSPECIFIED }, IpStatus { set: 0, conflict: false }),
    };
    DeviceIdentity {
        name: Some(cfg.station.clone()),
        alias: cfg.alias.clone(),
        ip: Some(ip),
        ip_status: Some(ip_status),
        vendor_id: Some(cfg.vendor_id),
        device_id: Some(cfg.device_id),
        vendor: Some(cfg.vendor.clone()),
        role: Some(ROLE_IO_DEVICE),
        instance: None,
        oem: None,
        options: vec![(1, 1), (1, 2), (2, 1), (2, 2), (2, 3), (2, 4), (2, 5), (5, 3), (5, 4)],
        mac: Some(mac),
        initiative: None,
    }
}

/// Start the device on its interface (Linux only). Returns once the link is open and the Hello is
/// sent.
#[cfg(not(target_os = "linux"))]
pub fn start(cfg: &ProfinetConfig) -> Result<ProfinetHandle, ProfinetError> {
    validate(cfg)?;
    Err(ProfinetError::Unsupported)
}

/// Start the device on its interface (Linux only). Returns once the link is open and the Hello is
/// sent.
#[cfg(target_os = "linux")]
pub fn start(cfg: &ProfinetConfig) -> Result<ProfinetHandle, ProfinetError> {
    validate(cfg)?;
    let link = RawLink::open_with(&cfg.interface, Options { ethertype: Some(ether::ETHERTYPE_PROFINET), promiscuous: false, outgoing: false })
        .map_err(|e| ProfinetError::Link(e.to_string()))?;
    let mac = link.mac();
    let shared = Arc::new(Shared {
        state: Mutex::new(State { identity: identity_of(cfg, mac), status: ProfinetStatus::new(mac) }),
        muted: AtomicBool::new(false),
        stop: AtomicBool::new(false),
    });
    if cfg.hello {
        let id = shared.lock().identity.clone();
        if let Ok(frame) = dcp::hello(mac, 1, &id) {
            link.send(&frame).map_err(|e| ProfinetError::Link(e.to_string()))?;
            shared.lock().status.hellos += 1;
        }
    }
    let cfg_interface = cfg.interface.clone();
    let cfg = cfg.clone();
    let worker = Arc::clone(&shared);
    let thread = std::thread::Builder::new()
        .name("maquinar-profinet".into())
        .spawn(move || run(link, cfg, worker))
        .map_err(ProfinetError::Thread)?;
    Ok(ProfinetHandle { mac, interface: cfg_interface, shared, thread: Some(thread) })
}

#[cfg(target_os = "linux")]
fn lldp_frame(mac: Mac, cfg: &ProfinetConfig, id: &DeviceIdentity, ttl: u16) -> Option<Vec<u8>> {
    let station = id.name.clone().unwrap_or_default();
    let port = if station.is_empty() { cfg.port.clone() } else { format!("{}.{station}", cfg.port) };
    let mut pdu = LldpPdu::new(ChassisId::Local { id: station }, PortId::Local { id: port }, ttl);
    pdu.pn.chassis_mac = Some(mac);
    lldp::frame(mac, &pdu).ok()
}

#[cfg(target_os = "linux")]
fn run(link: RawLink, cfg: ProfinetConfig, shared: Arc<Shared>) {
    let mut rng = match cfg.seed {
        Some(s) => Pcg32::new(s, 0x0D0C),
        None => Pcg32::from_entropy(0x0D0C),
    };
    let mac = link.mac();
    let mut pending: Vec<(Instant, Vec<u8>)> = Vec::new();
    let every = Duration::from_millis(cfg.lldp_interval_ms);
    let mut next_lldp = Instant::now();
    loop {
        if shared.stop.load(Ordering::SeqCst) {
            if cfg.lldp_interval_ms > 0 {
                let id = shared.lock().identity.clone();
                if let Some(f) = lldp_frame(mac, &cfg, &id, 0) {
                    let _ = link.send(&f);
                }
            }
            return;
        }
        let now = Instant::now();
        if cfg.lldp_interval_ms > 0 && now >= next_lldp {
            let id = shared.lock().identity.clone();
            if let Some(f) = lldp_frame(mac, &cfg, &id, (cfg.lldp_interval_ms * 4 / 1000).clamp(1, 65535) as u16)
                && link.send(&f).is_ok()
            {
                shared.lock().status.lldp_sent += 1;
            }
            while next_lldp <= now {
                next_lldp += every;
            }
        }
        pending.retain(|(due, frame)| {
            if *due <= now {
                let _ = link.send(frame);
                false
            } else {
                true
            }
        });
        let Ok(Some(received)) = link.recv(Duration::from_millis(5)) else { continue };
        let Ok(frame) = ether::parse(&received.bytes) else { continue };
        if frame.ethertype != ether::ETHERTYPE_PROFINET || !(frame.dst == mac || frame.dst == Mac::DCP_MULTICAST) {
            continue;
        }
        if shared.muted.load(Ordering::SeqCst) {
            continue;
        }
        let Ok(pdu) = dcp::decode(frame.payload) else {
            shared.lock().status.undecodable += 1;
            continue;
        };
        if pdu.service_type != ServiceType::Request {
            continue;
        }
        match pdu.service {
            Service::Identify => {
                let mut state = shared.lock();
                if !matches(&state.identity, &pdu.blocks) {
                    state.status.identify_passed += 1;
                    continue;
                }
                state.status.identify_answered += 1;
                let Ok(answer) = dcp::identify_response(frame.src, mac, pdu.xid, &state.identity) else { continue };
                drop(state);
                // Spread over the request's response delay: a factor of 1 (or 0) answers at once,
                // a factor of n anywhere within (n - 1) steps of 10 ms.
                let spread = if pdu.response_delay > 1 { (rng.next_u32() as u64) % (u64::from(pdu.response_delay - 1) * 10_000 + 1) } else { 0 };
                pending.push((Instant::now() + Duration::from_micros(spread), answer));
            }
            Service::Set if frame.dst == mac => {
                let answers = set(&shared, &pdu);
                if let Ok(answer) = dcp::set_response(frame.src, mac, pdu.xid, &answers) {
                    let _ = link.send(&answer);
                }
            }
            Service::Get if frame.dst == mac => {
                let mut state = shared.lock();
                state.status.gets += 1;
                let wanted: Vec<(u8, u8)> = pdu.blocks.iter().map(|b| (b.option, b.suboption)).collect();
                let blocks: Vec<Block> = state.identity.blocks().into_iter().filter(|b| wanted.contains(&(b.option, b.suboption))).collect();
                drop(state);
                let answer = DcpPdu { frame_id: dcp::FRAME_ID_GET_SET, service: Service::Get, service_type: ServiceType::Success, xid: pdu.xid, response_delay: 0, blocks, trailing: Vec::new() };
                if let Ok(f) = answer.frame(frame.src, mac) {
                    let _ = link.send(&f);
                }
            }
            _ => {}
        }
    }
}

/// Whether every filter block of an Identify request matches the device.
#[cfg(target_os = "linux")]
fn matches(id: &DeviceIdentity, filters: &[Block]) -> bool {
    !filters.is_empty()
        && filters.iter().all(|b| match &b.body {
            BlockBody::AllSelector => true,
            BlockBody::NameOfStation { name } => id.name.as_deref().is_some_and(|n| !n.is_empty() && n == name),
            BlockBody::AliasName { name } => id.alias.as_deref() == Some(name.as_str()),
            _ => false,
        })
}

/// Apply a Set's blocks; each answered (option, suboption, error).
#[cfg(target_os = "linux")]
fn set(shared: &Shared, pdu: &DcpPdu) -> Vec<(u8, u8, u8)> {
    let mut state = shared.lock();
    state.status.sets += 1;
    let mut answers = Vec::new();
    for b in &pdu.blocks {
        let e = match &b.body {
            BlockBody::NameOfStation { name } => {
                if name.is_empty() || dcp::check_station_name(name).is_ok() {
                    state.identity.name = Some(name.clone());
                    error::NONE
                } else {
                    error::SET_NOT_POSSIBLE
                }
            }
            BlockBody::IpParameter { suite } => {
                state.identity.ip = Some(suite.clone());
                let set = if suite.ip.is_unspecified() { 0 } else { 1 };
                let conflict = state.identity.ip_status.map(|s| s.conflict).unwrap_or(false);
                state.identity.ip_status = Some(IpStatus { set, conflict });
                error::NONE
            }
            BlockBody::ControlSignal { .. } => {
                state.status.blinks += 1;
                error::NONE
            }
            _ if b.option == 1 || b.option == 2 || b.option == 5 => error::SUBOPTION_UNSUPPORTED,
            _ => error::OPTION_UNSUPPORTED,
        };
        answers.push((b.option, b.suboption, e));
    }
    answers
}
