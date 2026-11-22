//! The raw link: whole Ethernet frames sent and heard on one named interface, through an
//! `AF_PACKET` socket (Linux). The seam the scanner listens and asks through, and the one the
//! bench's PROFINET device and core's IO controller will send through.
//!
//! The socket needs `CAP_NET_RAW` in the network namespace of the interface: on a real network
//! card an install step (`setcap cap_net_raw+ep` on the executable, `docs/aranha-plan.md`,
//! "Traps"); in a test, the private network a test makes for itself (`crate::netns`), where it
//! holds it. Without it, opening the link is refused with a reason that names the capability.
//!
//! The link hears every frame on its interface (or those of one EtherType, the kernel filtering),
//! those this host's other sockets send on it included (marked outgoing, or left out on request);
//! a socket never hears its own frames back. It joins every group address (`PACKET_MR_ALLMULTI`),
//! so DCP's and LLDP's multicast frames arrive whatever the card filters; promiscuous mode, for a
//! mirror port, is asked for apart. Every call is bounded: a receive waits at most its timeout, a
//! send never blocks.
//!
//! A frame is handed back as it was on the wire. The kernel takes an 802.1Q tag out of a frame
//! before a packet socket reads it (a veth pair does, as most cards do), keeping it beside the
//! frame; the link asks for it (`PACKET_AUXDATA`) and puts it back where it was, so a cyclic
//! frame's priority 6 tag is read as it was sent. An EtherType filter matches the type inside the
//! tag: it is a filter program the kernel runs on every frame (classic BPF, as libpcap's), because
//! a socket bound to one EtherType is handed its frames after the kernel has dropped a priority
//! tag for good (measured: the tag of a VLAN 0 frame was gone from the aux data too).

use std::ffi::CString;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::{Duration, SystemTime};

use crate::profinet::mac::Mac;

/// The longest frame the link reads; a longer one is reported cut, never handed back as whole.
pub const MAX_FRAME: usize = 65_536;

/// One frame heard: its bytes as they arrived (no check sequence), when the link read it, and
/// whether this host sent it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Received {
    pub bytes: Vec<u8>,
    pub at: SystemTime,
    pub outgoing: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum RawError {
    #[error("no network interface is named {0:?}")]
    NoSuchInterface(String),
    #[error("an interface name is 1 to 15 bytes with no NUL; {0:?} is not one")]
    BadName(String),
    #[error(
        "opening a raw link on {iface:?} needs CAP_NET_RAW in its network namespace ({source}): \
         `setcap cap_net_raw+ep` on the executable, or run it in a namespace of its own"
    )]
    Privilege { iface: String, source: io::Error },
    #[error("{op} on {iface:?}: {source}")]
    Io { op: &'static str, iface: String, source: io::Error },
    #[error("a frame of {0} bytes is past what the link sends")]
    TooLong(usize),
    #[error("a frame of {0} bytes is shorter than an Ethernet header")]
    TooShort(usize),
}

/// How a link opens: one EtherType or every one, promiscuous or not, and whether frames this
/// host's other sockets send on the interface are handed back (marked outgoing).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub ethertype: Option<u16>,
    pub promiscuous: bool,
    pub outgoing: bool,
}

impl Default for Options {
    /// Every EtherType, not promiscuous, outgoing frames handed back.
    fn default() -> Self {
        Options { ethertype: None, promiscuous: false, outgoing: true }
    }
}

/// A raw link on one interface. Dropping it closes the socket and leaves the interface's
/// memberships as they were.
#[derive(Debug)]
pub struct RawLink {
    fd: OwnedFd,
    iface: String,
    index: i32,
    mac: Mac,
    options: Options,
}

/// A classic BPF program keeping the frames of one EtherType: the type at byte 12 (a tag the
/// kernel took out is no longer there), or behind a tag still in the frame, at byte 16.
fn ethertype_filter(ethertype: u16) -> Vec<libc::sock_filter> {
    let op = |code: u32, jt: u8, jf: u8, k: u32| libc::sock_filter { code: code as u16, jt, jf, k };
    let t = u32::from(ethertype);
    vec![
        op(libc::BPF_LD | libc::BPF_H | libc::BPF_ABS, 0, 0, 12),
        op(libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K, 3, 0, t),
        op(libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K, 0, 3, 0x8100),
        op(libc::BPF_LD | libc::BPF_H | libc::BPF_ABS, 0, 0, 16),
        op(libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K, 0, 1, t),
        op(libc::BPF_RET | libc::BPF_K, 0, 0, MAX_FRAME as u32),
        op(libc::BPF_RET | libc::BPF_K, 0, 0, 0),
    ]
}

fn last_error() -> io::Error {
    io::Error::last_os_error()
}

fn interface_index(name: &str) -> Result<(CString, i32), RawError> {
    if name.is_empty() || name.len() >= libc::IFNAMSIZ {
        return Err(RawError::BadName(name.to_string()));
    }
    let c = CString::new(name).map_err(|_| RawError::BadName(name.to_string()))?;
    // SAFETY: `c` is a valid NUL-terminated string for the duration of the call.
    let index = unsafe { libc::if_nametoindex(c.as_ptr()) };
    if index == 0 {
        return Err(RawError::NoSuchInterface(name.to_string()));
    }
    let index = i32::try_from(index).map_err(|_| RawError::NoSuchInterface(name.to_string()))?;
    Ok((c, index))
}

impl RawLink {
    /// Open a raw link on `iface`, hearing every EtherType. `promiscuous` also hears frames to
    /// other hosts' unicast addresses (a mirror port, a hub).
    pub fn open(iface: &str, promiscuous: bool) -> Result<RawLink, RawError> {
        RawLink::open_with(iface, Options { promiscuous, ..Options::default() })
    }

    /// Open a raw link on `iface` as `options` say.
    pub fn open_with(iface: &str, options: Options) -> Result<RawLink, RawError> {
        let promiscuous = options.promiscuous;
        let (cname, index) = interface_index(iface)?;
        let io_err = |op: &'static str| RawError::Io { op, iface: iface.to_string(), source: last_error() };
        let all = (libc::ETH_P_ALL as u16).to_be();
        // Created with protocol 0, it hears nothing until bound: the filter is in place first, so
        // no frame of another type is queued before it.
        // SAFETY: plain socket creation; the descriptor is owned at once.
        let fd = unsafe { libc::socket(libc::AF_PACKET, libc::SOCK_RAW | libc::SOCK_CLOEXEC, 0) };
        if fd < 0 {
            let source = last_error();
            return Err(match source.raw_os_error() {
                Some(libc::EPERM) | Some(libc::EACCES) => RawError::Privilege { iface: iface.to_string(), source },
                _ => RawError::Io { op: "socket", iface: iface.to_string(), source },
            });
        }
        // SAFETY: `fd` was just returned by socket() and is owned by nothing else.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        if let Some(ethertype) = options.ethertype {
            let mut program = ethertype_filter(ethertype);
            let fprog = libc::sock_fprog { len: program.len() as u16, filter: program.as_mut_ptr() };
            // SAFETY: `fprog` points at `program`, which outlives the call; the kernel copies it.
            let rc = unsafe {
                libc::setsockopt(
                    fd.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_ATTACH_FILTER,
                    (&raw const fprog).cast(),
                    std::mem::size_of::<libc::sock_fprog>() as libc::socklen_t,
                )
            };
            if rc < 0 {
                return Err(io_err("filter"));
            }
        }

        // SAFETY: an all-zero sockaddr_ll is a valid value, then filled in.
        let mut addr: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
        addr.sll_family = libc::AF_PACKET as u16;
        addr.sll_protocol = all;
        addr.sll_ifindex = index;
        // SAFETY: `addr` is a valid sockaddr_ll of the size given.
        let rc = unsafe {
            libc::bind(fd.as_raw_fd(), (&raw const addr).cast(), std::mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t)
        };
        if rc < 0 {
            return Err(io_err("bind"));
        }

        // The tag the kernel takes out of a frame, handed beside it.
        let on: libc::c_int = 1;
        // SAFETY: `on` is a valid int of the size given.
        let rc = unsafe {
            libc::setsockopt(
                fd.as_raw_fd(),
                libc::SOL_PACKET,
                libc::PACKET_AUXDATA,
                (&raw const on).cast(),
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            )
        };
        if rc < 0 {
            return Err(io_err("ask for the frame's tag"));
        }

        let mut kinds = vec![libc::PACKET_MR_ALLMULTI];
        if promiscuous {
            kinds.push(libc::PACKET_MR_PROMISC);
        }
        for kind in kinds {
            // SAFETY: an all-zero packet_mreq is valid, then filled in.
            let mut mreq: libc::packet_mreq = unsafe { std::mem::zeroed() };
            mreq.mr_ifindex = index;
            mreq.mr_type = kind as u16;
            // SAFETY: `mreq` is a valid packet_mreq of the size given.
            let rc = unsafe {
                libc::setsockopt(
                    fd.as_raw_fd(),
                    libc::SOL_PACKET,
                    libc::PACKET_ADD_MEMBERSHIP,
                    (&raw const mreq).cast(),
                    std::mem::size_of::<libc::packet_mreq>() as libc::socklen_t,
                )
            };
            if rc < 0 {
                return Err(io_err("join"));
            }
        }

        // The interface's own MAC, asked of the kernel in this namespace (sysfs may show another).
        // SAFETY: an all-zero ifreq is valid; the name fits (checked above) and is copied in.
        let mut req: libc::ifreq = unsafe { std::mem::zeroed() };
        for (dst, src) in req.ifr_name.iter_mut().zip(cname.as_bytes()) {
            *dst = *src as libc::c_char;
        }
        // SAFETY: SIOCGIFHWADDR fills `req.ifr_ifru.ifru_hwaddr`; `req` outlives the call.
        let rc = unsafe { libc::ioctl(fd.as_raw_fd(), libc::SIOCGIFHWADDR, &raw mut req) };
        if rc < 0 {
            return Err(io_err("read the interface's MAC"));
        }
        // SAFETY: after a successful SIOCGIFHWADDR the union holds `ifru_hwaddr`.
        let hw = unsafe { req.ifr_ifru.ifru_hwaddr };
        let mut mac = [0u8; 6];
        for (m, b) in mac.iter_mut().zip(hw.sa_data.iter()) {
            *m = *b as u8;
        }
        Ok(RawLink { fd, iface: iface.to_string(), index, mac: Mac(mac), options })
    }

    pub fn iface(&self) -> &str {
        &self.iface
    }

    pub fn index(&self) -> i32 {
        self.index
    }

    /// The interface's own MAC, as the kernel holds it.
    pub fn mac(&self) -> Mac {
        self.mac
    }

    /// Send one whole frame (header included, as `profinet::ether` writes it). It never blocks:
    /// a full queue is an error, not a wait.
    pub fn send(&self, frame: &[u8]) -> Result<(), RawError> {
        if frame.len() < 14 {
            return Err(RawError::TooShort(frame.len()));
        }
        // SAFETY: `frame` is a valid buffer of the length given.
        let n = unsafe { libc::send(self.fd.as_raw_fd(), frame.as_ptr().cast(), frame.len(), libc::MSG_DONTWAIT) };
        if n < 0 {
            let source = last_error();
            if source.raw_os_error() == Some(libc::EMSGSIZE) {
                return Err(RawError::TooLong(frame.len()));
            }
            return Err(RawError::Io { op: "send", iface: self.iface.clone(), source });
        }
        if n as usize != frame.len() {
            return Err(RawError::Io {
                op: "send",
                iface: self.iface.clone(),
                source: io::Error::new(io::ErrorKind::WriteZero, format!("{n} of {} bytes sent", frame.len())),
            });
        }
        Ok(())
    }

    /// The next frame heard, or `None` when none arrived within `timeout`, its 802.1Q tag put back
    /// where the kernel took it from. A frame longer than `MAX_FRAME` is an error, never handed
    /// back cut. With `outgoing` off, frames this host sent are passed over within the same wait.
    pub fn recv(&self, timeout: Duration) -> Result<Option<Received>, RawError> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match self.recv_one(left)? {
                Some(r) if r.outgoing && !self.options.outgoing => {
                    if left.is_zero() {
                        return Ok(None);
                    }
                }
                other => return Ok(other),
            }
        }
    }

    fn recv_one(&self, timeout: Duration) -> Result<Option<Received>, RawError> {
        let io_err = |op: &'static str, source: io::Error| RawError::Io { op, iface: self.iface.clone(), source };
        let mut pfd = libc::pollfd { fd: self.fd.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        let ms = i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX);
        // SAFETY: one valid pollfd.
        let rc = unsafe { libc::poll(&raw mut pfd, 1, ms) };
        if rc < 0 {
            let source = last_error();
            if source.kind() == io::ErrorKind::Interrupted {
                return Ok(None);
            }
            return Err(io_err("poll", source));
        }
        if rc == 0 {
            return Ok(None);
        }
        let mut buf = vec![0u8; MAX_FRAME];
        // SAFETY: all-zero sockaddr_ll and msghdr are valid; they are filled in below.
        let mut from: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
        let mut iov = libc::iovec { iov_base: buf.as_mut_ptr().cast(), iov_len: buf.len() };
        // Room for the aux data message, aligned as a cmsghdr wants.
        let mut control = [0u64; 16];
        // SAFETY: as above.
        let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
        msg.msg_name = (&raw mut from).cast();
        msg.msg_namelen = std::mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t;
        msg.msg_iov = &raw mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = control.as_mut_ptr().cast();
        msg.msg_controllen = std::mem::size_of_val(&control);
        // SAFETY: every buffer `msg` points at lives across the call, with the sizes given.
        let n = unsafe { libc::recvmsg(self.fd.as_raw_fd(), &raw mut msg, libc::MSG_DONTWAIT | libc::MSG_TRUNC) };
        if n < 0 {
            let source = last_error();
            if source.kind() == io::ErrorKind::WouldBlock || source.kind() == io::ErrorKind::Interrupted {
                return Ok(None);
            }
            return Err(io_err("receive", source));
        }
        let n = n as usize;
        if n > buf.len() {
            return Err(RawError::TooLong(n));
        }
        buf.truncate(n);
        // SAFETY: CMSG_FIRSTHDR and CMSG_NXTHDR walk the control buffer recvmsg filled, within
        // `msg_controllen`; the aux data is read unaligned from its message's data.
        let mut cmsg = unsafe { libc::CMSG_FIRSTHDR(&raw const msg) };
        while !cmsg.is_null() {
            let (level, kind) = unsafe { ((*cmsg).cmsg_level, (*cmsg).cmsg_type) };
            if level == libc::SOL_PACKET && kind == libc::PACKET_AUXDATA {
                let aux: libc::tpacket_auxdata = unsafe { std::ptr::read_unaligned(libc::CMSG_DATA(cmsg).cast()) };
                if aux.tp_status & libc::TP_STATUS_VLAN_VALID != 0 && buf.len() >= 12 {
                    let tpid = if aux.tp_status & libc::TP_STATUS_VLAN_TPID_VALID != 0 { aux.tp_vlan_tpid } else { 0x8100 };
                    let tag = [tpid.to_be_bytes(), aux.tp_vlan_tci.to_be_bytes()].concat();
                    buf.splice(12..12, tag);
                }
            }
            cmsg = unsafe { libc::CMSG_NXTHDR(&raw const msg, cmsg) };
        }
        Ok(Some(Received { bytes: buf, at: SystemTime::now(), outgoing: from.sll_pkttype == libc::PACKET_OUTGOING }))
    }
}
