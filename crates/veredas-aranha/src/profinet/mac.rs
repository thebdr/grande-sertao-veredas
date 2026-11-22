//! A MAC address: the identity of everything a scan sees (a device is its MAC; an address is
//! secondary; a name is never a key).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Six octets, in the order they travel.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Mac(pub [u8; 6]);

impl Mac {
    /// The group address DCP's Identify requests and Hellos go to.
    pub const DCP_MULTICAST: Mac = Mac([0x01, 0x0E, 0xCF, 0x00, 0x00, 0x00]);
    /// The nearest-bridge group address LLDP goes to: no bridge forwards it, so an LLDP frame
    /// heard names the far end of the cable it came over.
    pub const LLDP_MULTICAST: Mac = Mac([0x01, 0x80, 0xC2, 0x00, 0x00, 0x0E]);
    pub const BROADCAST: Mac = Mac([0xFF; 6]);

    /// The first six bytes of `b`, or `None` when there are fewer.
    pub fn from_slice(b: &[u8]) -> Option<Mac> {
        let arr: [u8; 6] = b.get(..6)?.try_into().ok()?;
        Some(Mac(arr))
    }

    /// The group bit: a multicast or the broadcast address, never one device.
    pub fn is_multicast(&self) -> bool {
        self.0[0] & 1 == 1
    }

    /// The locally administered bit: an address assigned by its owner, not taken from a vendor's
    /// block (the range the suite's fixtures use).
    pub fn is_local(&self) -> bool {
        self.0[0] & 2 == 2
    }

    /// The first three octets: the key of the IEEE registry of organisation identifiers.
    pub fn oui(&self) -> [u8; 3] {
        [self.0[0], self.0[1], self.0[2]]
    }
}

impl fmt::Display for Mac {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let b = self.0;
        write!(f, "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}", b[0], b[1], b[2], b[3], b[4], b[5])
    }
}

impl fmt::Debug for Mac {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Mac({self})")
    }
}

/// A text that is not one of the three spellings of a MAC address.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("not a MAC address: {0:?} (six pairs of hex digits joined by ':' or '-', or three groups of four joined by '.')")]
pub struct MacParseError(pub String);

impl FromStr for Mac {
    type Err = MacParseError;

    /// Exactly one of `aa:bb:cc:dd:ee:ff`, `aa-bb-cc-dd-ee-ff` and `aabb.ccdd.eeff`, in either
    /// case. Nothing else: no sign, no space, no mixed separators, no short groups.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let refuse = || MacParseError(s.to_string());
        let (sep, count, width) = match (s.contains(':'), s.contains('-'), s.contains('.')) {
            (true, false, false) => (':', 6, 2),
            (false, true, false) => ('-', 6, 2),
            (false, false, true) => ('.', 3, 4),
            _ => return Err(refuse()),
        };
        let groups: Vec<&str> = s.split(sep).collect();
        if groups.len() != count {
            return Err(refuse());
        }
        let mut hex = String::with_capacity(12);
        for g in &groups {
            if g.len() != width || !g.bytes().all(|c| c.is_ascii_hexdigit()) {
                return Err(refuse());
            }
            hex.push_str(g);
        }
        let mut out = [0u8; 6];
        for (i, o) in out.iter_mut().enumerate() {
            *o = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).map_err(|_| refuse())?;
        }
        Ok(Mac(out))
    }
}

impl Serialize for Mac {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Mac {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}
