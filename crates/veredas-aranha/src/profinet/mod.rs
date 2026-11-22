//! The PROFINET codecs: frames as bytes and back, with no socket and nothing of the rest of the
//! crate (`docs/aranha-plan.md`, "Rules"). They move into core when the IO controller needs them
//! (P-018); until then the bench's PROFINET device speaks through them (`docs/questions.md` Q5).
//!
//! Every decoder is total on input: what it cannot read is kept with the reason, never a panic.
//! Every encoder refuses what the wire cannot carry (a value past its length field, a group
//! address where one device must be named), never cutting it to fit.

pub mod alarm;
pub mod dcp;
pub mod ether;
pub mod lldp;
pub mod mac;
pub mod record;
pub mod rpc;
pub mod rt;
