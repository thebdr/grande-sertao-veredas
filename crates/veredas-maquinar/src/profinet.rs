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
//! interval. At the IP suite it holds it answers ARP and the connectionless record read of its
//! I&M0 and its port's real data ([`ip`]), the neighbour its LLDP reception heard among the latter.
//! The handle reads the identity as it stands, sets it from the plant side, mutes the device and
//! counts what it did.

pub mod ip;

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
use veredas_aranha::profinet::record::Peer;
#[cfg(target_os = "linux")]
use veredas_aranha::profinet::record::{Im0, LinkState, PortDataReal, SoftwareRevision};
#[cfg(target_os = "linux")]
use veredas_aranha::profinet::rpc::{PNIO_PORT, PnioStatus, ReadRequest};
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
    /// I&M0's order id, 1 to 20 visible characters (padded with spaces on the wire).
    #[serde(default = "default_order_id")]
    pub order_id: String,
    /// I&M0's serial number, up to 16 visible characters; empty, the MAC's twelve hex digits.
    #[serde(default)]
    pub serial_number: String,
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
            order_id: default_order_id(),
            serial_number: String::new(),
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
fn default_order_id() -> String {
    "veredas bench".into()
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
    if !is_port_name(&cfg.port) {
        return Err(bad(format!("profinet.port `{}`: a port is named port-xyz or port-xyz-abcde (xyz 001 to 255, abcde five digits)", cfg.port)));
    }
    if let Some(alias) = &cfg.alias {
        // An alias is a neighbour's port and station, as LLDP names them: `port-xyz.station`.
        let ok = alias.split_once('.').is_some_and(|(port, station)| is_port_name(port) && dcp::check_station_name(station).is_ok());
        if !ok {
            return Err(bad(format!("profinet.alias `{alias}`: an alias is a port name, a dot and a station name (port-001.switch-1)")));
        }
    }
    if cfg.vendor.is_empty() || cfg.vendor.len() > 255 {
        return Err(bad("profinet.vendor: 1 to 255 characters".into()));
    }
    let visible = |t: &str| t.bytes().all(|b| (0x20..=0x7E).contains(&b));
    if cfg.order_id.is_empty() || cfg.order_id.len() > 20 || !visible(&cfg.order_id) {
        return Err(bad(format!("profinet.order_id `{}`: 1 to 20 visible characters", cfg.order_id)));
    }
    if cfg.serial_number.len() > 16 || !visible(&cfg.serial_number) {
        return Err(bad(format!("profinet.serial_number `{}`: up to 16 visible characters", cfg.serial_number)));
    }
    Ok(())
}

/// `port-xyz` or `port-xyz-abcde`: xyz 001 to 255, abcde five digits (IEC 61158-6-10's port names).
pub fn is_port_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("port-") else { return false };
    let (xyz, abcde) = match rest.split_once('-') {
        Some((x, a)) => (x, Some(a)),
        None => (rest, None),
    };
    let digits = |t: &str, n: usize| t.len() == n && t.bytes().all(|b| b.is_ascii_digit());
    digits(xyz, 3) && (1..=255).contains(&xyz.parse::<u16>().unwrap_or(0)) && abcde.is_none_or(|a| digits(a, 5))
}

/// What the device reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfinetStatus {
    /// The interface's MAC, the device's.
    pub mac: Mac,
    pub hellos: u64,
    /// Identify answers sent, counted as each goes out (an answer dropped by a mute or a stop is
    /// not one).
    pub identify_answered: u64,
    /// Identify requests whose filter it did not match.
    pub identify_passed: u64,
    pub sets: u64,
    pub gets: u64,
    pub blinks: u64,
    pub lldp_sent: u64,
    /// DCP frames it could not read.
    pub undecodable: u64,
    /// ARP requests for its address answered.
    pub arp_answered: u64,
    /// Record reads answered with the record.
    pub reads: u64,
    /// Record reads answered with an error status.
    pub reads_refused: u64,
}

impl ProfinetStatus {
    /// Nothing done yet, by the device of this MAC.
    pub fn new(mac: Mac) -> ProfinetStatus {
        ProfinetStatus { mac, hellos: 0, identify_answered: 0, identify_passed: 0, sets: 0, gets: 0, blinks: 0, lldp_sent: 0, undecodable: 0, arp_answered: 0, reads: 0, reads_refused: 0 }
    }
}

struct State {
    identity: DeviceIdentity,
    status: ProfinetStatus,
    /// The neighbour its LLDP reception heard last on its port, gone with a TTL of 0.
    neighbour: Option<Peer>,
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

    /// Its DCP answers nothing (true), as a device whose DCP is off; LLDP, ARP and the record read
    /// go on. An Identify answer waiting out its delay is dropped.
    pub fn set_muted(&self, muted: bool) {
        self.shared.muted.store(muted, Ordering::SeqCst);
    }

    pub fn status(&self) -> ProfinetStatus {
        self.shared.lock().status.clone()
    }

    /// The neighbour its LLDP reception heard on its port, as its port data states it.
    pub fn neighbour(&self) -> Option<Peer> {
        self.shared.lock().neighbour.clone()
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
    // Every EtherType: DCP, LLDP from its neighbour, ARP and IPv4 for its record read.
    let link = RawLink::open_with(&cfg.interface, Options { ethertype: None, promiscuous: false, outgoing: false })
        .map_err(|e| ProfinetError::Link(e.to_string()))?;
    let mac = link.mac();
    let shared = Arc::new(Shared {
        state: Mutex::new(State { identity: identity_of(cfg, mac), status: ProfinetStatus::new(mac), neighbour: None }),
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
    // An unnamed device has no NameOfStation to give: its chassis is its MAC.
    let (chassis, port) = if station.is_empty() { (ChassisId::Mac { mac }, cfg.port.clone()) } else { (ChassisId::Local { id: station.clone() }, format!("{}.{station}", cfg.port)) };
    let mut pdu = LldpPdu::new(chassis, PortId::Local { id: port }, ttl);
    // A shutdown frame carries the chassis, the port and the TTL alone (802.1AB).
    if ttl > 0 {
        pdu.pn.chassis_mac = Some(mac);
    }
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
    // The RPC server's boot time: the start, in seconds since the epoch.
    let server_boot = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as u32).unwrap_or(1);
    let mut ip_id: u16 = 0;
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
        if shared.muted.load(Ordering::SeqCst) {
            pending.clear();
        }
        pending.retain(|(due, frame)| {
            if *due <= now {
                // Counted as it goes out, so whoever reads the answer finds it counted; a send that
                // fails takes it back.
                shared.lock().status.identify_answered += 1;
                if link.send(frame).is_err() {
                    shared.lock().status.identify_answered -= 1;
                }
                false
            } else {
                true
            }
        });
        let Ok(Some(received)) = link.recv(Duration::from_millis(5)) else { continue };
        let Ok(frame) = ether::parse(&received.bytes) else { continue };
        match frame.ethertype {
            ether::ETHERTYPE_LLDP => {
                if let Ok(pdu) = lldp::decode(frame.payload) {
                    let heard = neighbour_of(&pdu, frame.src);
                    let mut state = shared.lock();
                    if pdu.ttl > 0 {
                        state.neighbour = Some(heard);
                    } else if state.neighbour.as_ref().is_some_and(|n| (&n.chassis_id, &n.port_id) == (&heard.chassis_id, &heard.port_id)) {
                        // Its own shutdown frame (802.1AB names a neighbour by chassis and port):
                        // another station's leaves it.
                        state.neighbour = None;
                    }
                }
                continue;
            }
            ip::ETHERTYPE_ARP | ip::ETHERTYPE_IPV4 => {
                let address = shared.lock().identity.ip.as_ref().map(|s| s.ip).unwrap_or(Ipv4Addr::UNSPECIFIED);
                if let Some(reply) = ip::arp_reply(&received.bytes, mac, address) {
                    shared.lock().status.arp_answered += 1;
                    let _ = link.send(&reply);
                } else if let Some((peer_mac, peer_ip, peer_port, payload)) = ip::udp_to(&received.bytes, address, PNIO_PORT) {
                    let answered = std::cell::Cell::new(false);
                    let records = |read: &ReadRequest| {
                        let r = record(&shared.lock(), &cfg, mac, read);
                        answered.set(r.is_ok());
                        r
                    };
                    if let Some((_, answer)) = ip::answer(&payload, &records, server_boot) {
                        {
                            let mut state = shared.lock();
                            if answered.get() {
                                state.status.reads += 1;
                            } else {
                                state.status.reads_refused += 1;
                            }
                        }
                        ip_id = ip_id.wrapping_add(1);
                        let _ = link.send(&ip::udp_frame((peer_mac, peer_ip, peer_port), (mac, address, PNIO_PORT), ip_id, &answer));
                    }
                }
                continue;
            }
            ether::ETHERTYPE_PROFINET if frame.dst == mac || frame.dst == Mac::DCP_MULTICAST => {}
            _ => continue,
        }
        if shared.muted.load(Ordering::SeqCst) {
            continue;
        }
        let Ok(pdu) = dcp::decode(frame.payload) else {
            shared.lock().status.undecodable += 1;
            continue;
        };
        // A request under another service's frame id is not this device's to answer.
        let frame_id = match pdu.service {
            Service::Identify => dcp::FRAME_ID_IDENTIFY_REQ,
            _ => dcp::FRAME_ID_GET_SET,
        };
        if pdu.service_type != ServiceType::Request || pdu.frame_id != frame_id {
            continue;
        }
        match pdu.service {
            Service::Identify => {
                let mut state = shared.lock();
                if !matches(&state.identity, &pdu.blocks) {
                    state.status.identify_passed += 1;
                    continue;
                }
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
                let held = state.identity.blocks();
                drop(state);
                // Each pair asked, in the order asked: the block, or a control response saying the
                // suboption is not there (0x02, "suboption not supported or no DataSet available").
                let blocks: Vec<Block> = pdu
                    .blocks
                    .iter()
                    .map(|w| {
                        held.iter().find(|b| (b.option, b.suboption) == (w.option, w.suboption)).cloned().unwrap_or(Block {
                            option: 5,
                            suboption: 4,
                            info: None,
                            body: BlockBody::ControlResponse { option: w.option, suboption: w.suboption, error: error::SUBOPTION_UNSUPPORTED },
                        })
                    })
                    .collect();
                let answer = DcpPdu { frame_id: dcp::FRAME_ID_GET_SET, service: Service::Get, service_type: ServiceType::Success, xid: pdu.xid, response_delay: 0, blocks, trailing: Vec::new() };
                if let Ok(f) = answer.frame(frame.src, mac) {
                    let _ = link.send(&f);
                }
            }
            _ => {}
        }
    }
}

/// The slot and subslots the device has: the access point's (slot 0, subslot 1) and its port's.
#[cfg(target_os = "linux")]
pub const SUBSLOT_DAP: u16 = 0x0001;
#[cfg(target_os = "linux")]
pub const SUBSLOT_PORT: u16 = 0x8001;

/// A record the device holds, or the status that refuses the read: I&M0 at the access point,
/// the port's real data at the port; an index it holds at another place refused as a slot it
/// lacks, any other index as an index it lacks.
#[cfg(target_os = "linux")]
fn record(state: &State, cfg: &ProfinetConfig, mac: Mac, read: &ReadRequest) -> Result<Vec<u8>, PnioStatus> {
    use veredas_aranha::profinet::record::{INDEX_IM0, INDEX_PORT_DATA_REAL};
    let at = (read.slot, read.subslot);
    let block = match read.index {
        INDEX_IM0 | INDEX_PORT_DATA_REAL if read.api != 0 => return Err(ip::INVALID_API),
        INDEX_IM0 if at == (0, SUBSLOT_DAP) => im0(state, cfg, mac).block(),
        INDEX_PORT_DATA_REAL if at == (0, SUBSLOT_PORT) => port_data(state, cfg).block(),
        INDEX_IM0 | INDEX_PORT_DATA_REAL => return Err(ip::INVALID_SLOT),
        _ => return Err(ip::INVALID_INDEX),
    };
    // The texts are made to fit (`text_of`): a block that still will not encode is a read error,
    // never an index the device lacks.
    block.ok().and_then(|b| b.encode().ok()).ok_or(ip::READ_ERROR)
}

/// I&M0 as the file states it: the vendor id, the order id and serial number padded with spaces
/// (the serial, when the file gives none, the MAC's twelve hex digits), hardware revision 1,
/// software V1.0.0, I&M version 1.1, no other I&M record.
#[cfg(target_os = "linux")]
fn im0(state: &State, cfg: &ProfinetConfig, mac: Mac) -> Im0 {
    let serial = if cfg.serial_number.is_empty() { mac.0.iter().map(|b| format!("{b:02X}")).collect() } else { cfg.serial_number.clone() };
    Im0 {
        vendor_id: state.identity.vendor_id.unwrap_or(cfg.vendor_id),
        order_id: format!("{:<20}", cfg.order_id),
        serial_number: format!("{serial:<16}"),
        hardware_revision: 1,
        software_revision: SoftwareRevision { prefix: b'V', functional: 1, bugfix: 0, internal: 0 },
        revision_counter: 0,
        profile_id: 0,
        profile_specific_type: 0,
        version: (1, 1),
        supported: 0,
    }
}

/// The port's real data: its name, the neighbour LLDP heard (none before one is heard), copper
/// at 100 Mbit/s full duplex, the link up.
#[cfg(target_os = "linux")]
fn port_data(state: &State, cfg: &ProfinetConfig) -> PortDataReal {
    PortDataReal {
        slot: 0,
        subslot: SUBSLOT_PORT,
        own_port_id: cfg.port.clone(),
        peers: state.neighbour.iter().cloned().collect(),
        mau_type: 0x0010,
        domain_boundary: 0,
        multicast_boundary: 0,
        link_state: LinkState { port: 0, link: 1 },
        media_type: 1,
        extra: Vec::new(),
    }
}

/// The most a port data's id holds, in visible characters.
#[cfg(target_os = "linux")]
const MAX_ID: usize = 255;

/// A text as the port data carries it: visible ASCII kept, every other byte written `\xHH`, cut
/// to 255 characters.
#[cfg(target_os = "linux")]
fn text_of(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if (0x20..=0x7E).contains(&b) && b != b'\\' {
            out.push(b as char);
        } else {
            out.push_str(&format!("\\x{b:02x}"));
        }
    }
    out.chars().take(MAX_ID).collect()
}

/// Bytes as hex, cut to 255 characters.
#[cfg(target_os = "linux")]
fn hex_of(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect::<String>().chars().take(MAX_ID).collect()
}

/// A network address as LLDP carries one (its family byte, then the address): IPv4 and IPv6 as
/// their text, any other as hex.
#[cfg(target_os = "linux")]
fn address_of(data: &[u8]) -> String {
    match data {
        [1, a, b, c, d] => Ipv4Addr::new(*a, *b, *c, *d).to_string(),
        [2, rest @ ..] if rest.len() == 16 => std::net::Ipv6Addr::from(<[u8; 16]>::try_from(rest).unwrap_or([0; 16])).to_string(),
        _ => hex_of(data),
    }
}

/// Bytes of a subtype no text names: as text when they are visible ASCII, else as hex.
#[cfg(target_os = "linux")]
fn other_of(data: &[u8]) -> String {
    match std::str::from_utf8(data) {
        Ok(t) if !t.is_empty() && t.bytes().all(|b| (0x20..=0x7E).contains(&b)) => text_of(t),
        _ => hex_of(data),
    }
}

/// A neighbour as its LLDP frame names it: the port id and the chassis id as text by their
/// subtype (a MAC as its address, a network address as its text, a name as itself), each at most
/// 255 visible characters; its source MAC; no measured delay.
#[cfg(target_os = "linux")]
fn neighbour_of(pdu: &LldpPdu, src: Mac) -> Peer {
    let chassis = match &pdu.chassis {
        ChassisId::Local { id } => text_of(id),
        ChassisId::Mac { mac } => mac.to_string(),
        ChassisId::IfName { name } => text_of(name),
        ChassisId::Network { data } => address_of(data),
        ChassisId::Other { data, .. } => other_of(data),
    };
    let port = match &pdu.port {
        PortId::Local { id } => text_of(id),
        PortId::IfName { name } | PortId::IfAlias { name } => text_of(name),
        PortId::Mac { mac } => mac.to_string(),
        PortId::Other { subtype: 4, data } => address_of(data),
        PortId::Other { data, .. } => other_of(data),
    };
    Peer { port_id: port, chassis_id: chassis, line_delay: 0, mac: src }
}

/// Whether every filter block of an Identify request matches the device.
#[cfg(target_os = "linux")]
fn matches(id: &DeviceIdentity, filters: &[Block]) -> bool {
    !filters.is_empty()
        && filters.iter().all(|b| match &b.body {
            BlockBody::AllSelector => true,
            BlockBody::NameOfStation { name } => id.name.as_deref().is_some_and(|n| !n.is_empty() && n == name),
            BlockBody::AliasName { name } => id.alias.as_deref() == Some(name.as_str()),
            BlockBody::DeviceVendor { value } => id.vendor.as_deref() == Some(value.as_str()),
            BlockBody::DeviceId { vendor_id, device_id } => (id.vendor_id, id.device_id) == (Some(*vendor_id), Some(*device_id)),
            BlockBody::DeviceRole { role, .. } => id.role.is_some_and(|r| r & role != 0),
            BlockBody::IpParameter { suite } => id.ip.as_ref() == Some(suite),
            BlockBody::MacAddress { mac } => id.mac == Some(*mac),
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
            // The identification signal: flash once (0x0100), the one value the standard defines.
            BlockBody::ControlSignal { value: 0x0100 } => {
                state.status.blinks += 1;
                error::NONE
            }
            BlockBody::ControlSignal { .. } => error::SET_NOT_POSSIBLE,
            // A name block it holds but cannot read (not UTF-8, cut short): not possible.
            _ if (b.option, b.suboption) == (2, 2) => error::SET_NOT_POSSIBLE,
            _ if b.option == 1 || b.option == 2 || b.option == 5 => error::SUBOPTION_UNSUPPORTED,
            _ => error::OPTION_UNSUPPORTED,
        };
        answers.push((b.option, b.suboption, e));
    }
    answers
}
