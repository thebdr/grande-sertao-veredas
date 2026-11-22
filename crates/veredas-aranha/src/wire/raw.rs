//! The raw link: whole Ethernet frames sent and heard on one named interface, through an
//! `AF_PACKET` socket (Linux). The seam the scanner listens and asks through, and the one the
//! bench's PROFINET device and core's IO controller will send through.
//!
//! The socket needs `CAP_NET_RAW` in the network namespace of the interface: on a real network
//! card an install step (`setcap cap_net_raw+ep` on the executable, `docs/aranha-plan.md`,
//! "Traps"); in a test, the private network a test makes for itself (`crate::netns`), where it
//! holds it. Without it, opening the link is refused with a reason that names the capability.
//!
//! The link hears every frame on its interface, those it sent itself included (marked outgoing),
//! and joins every group address (`PACKET_MR_ALLMULTI`), so DCP's and LLDP's multicast frames
//! arrive whatever the card filters; promiscuous mode, for a mirror port, is asked for apart.
//! Every call is bounded: a receive waits at most its timeout, a send never blocks.

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

/// A raw link on one interface. Dropping it closes the socket and leaves the interface's
/// memberships as they were.
#[derive(Debug)]
pub struct RawLink {
    fd: OwnedFd,
    iface: String,
    index: i32,
    mac: Mac,
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
        let (cname, index) = interface_index(iface)?;
        let io_err = |op: &'static str| RawError::Io { op, iface: iface.to_string(), source: last_error() };
        let all = (libc::ETH_P_ALL as u16).to_be();
        // SAFETY: plain socket creation; the descriptor is owned at once.
        let fd = unsafe { libc::socket(libc::AF_PACKET, libc::SOCK_RAW | libc::SOCK_CLOEXEC, i32::from(all)) };
        if fd < 0 {
            let source = last_error();
            return Err(match source.raw_os_error() {
                Some(libc::EPERM) | Some(libc::EACCES) => RawError::Privilege { iface: iface.to_string(), source },
                _ => RawError::Io { op: "socket", iface: iface.to_string(), source },
            });
        }
        // SAFETY: `fd` was just returned by socket() and is owned by nothing else.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };

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
        Ok(RawLink { fd, iface: iface.to_string(), index, mac: Mac(mac) })
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

    /// The next frame heard, or `None` when none arrived within `timeout`. A frame longer than
    /// `MAX_FRAME` is an error, never handed back cut.
    pub fn recv(&self, timeout: Duration) -> Result<Option<Received>, RawError> {
        let mut pfd = libc::pollfd { fd: self.fd.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        let ms = i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX);
        // SAFETY: one valid pollfd.
        let rc = unsafe { libc::poll(&raw mut pfd, 1, ms) };
        if rc < 0 {
            let source = last_error();
            if source.kind() == io::ErrorKind::Interrupted {
                return Ok(None);
            }
            return Err(RawError::Io { op: "poll", iface: self.iface.clone(), source });
        }
        if rc == 0 {
            return Ok(None);
        }
        let mut buf = vec![0u8; MAX_FRAME];
        // SAFETY: an all-zero sockaddr_ll is valid; recvfrom fills it within the size given.
        let mut from: libc::sockaddr_ll = unsafe { std::mem::zeroed() };
        let mut len = std::mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t;
        // SAFETY: `buf` and `from` are valid for the sizes given.
        let n = unsafe {
            libc::recvfrom(
                self.fd.as_raw_fd(),
                buf.as_mut_ptr().cast(),
                buf.len(),
                libc::MSG_DONTWAIT | libc::MSG_TRUNC,
                (&raw mut from).cast(),
                &raw mut len,
            )
        };
        if n < 0 {
            let source = last_error();
            if source.kind() == io::ErrorKind::WouldBlock || source.kind() == io::ErrorKind::Interrupted {
                return Ok(None);
            }
            return Err(RawError::Io { op: "receive", iface: self.iface.clone(), source });
        }
        let n = n as usize;
        if n > buf.len() {
            return Err(RawError::TooLong(n));
        }
        buf.truncate(n);
        Ok(Some(Received { bytes: buf, at: SystemTime::now(), outgoing: from.sll_pkttype == libc::PACKET_OUTGOING }))
    }
}
