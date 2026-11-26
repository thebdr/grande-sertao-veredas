//! veredas-aranha, the network scanner (`docs/aranha-plan.md`): who is on an industrial Ethernet
//! segment, how each device names itself and who is cabled to whom, from what the wire says, with
//! the evidence behind every value; for setup and fault-finding, never while the plant produces.
//!
//! Today it holds the PROFINET codecs (`profinet`), the wire (`wire`: the raw link, the capture
//! file, the memory hub), the private network a test makes for the raw link (`netns`), what a
//! frame says (`observe`), the devices it adds up to (`fusion`), the listener that hears them
//! (`listen`), the Identify asked through it (`ask`), the topology they make (`topology`), the
//! scan file that keeps it all (`scan`), the diff of a scan against a baseline (`diff`), and the
//! first of deep identity: a device's records read with no connection (`read`) in UDP datagrams
//! over IPv4 (`ip`), its ports' neighbours among them.

#[cfg(target_os = "linux")]
pub mod netns;
pub mod ask;
pub mod diff;
pub mod fusion;
pub mod ip;
pub mod listen;
pub mod observe;
pub mod profinet;
pub mod read;
pub mod scan;
pub mod topology;
pub mod wire;
