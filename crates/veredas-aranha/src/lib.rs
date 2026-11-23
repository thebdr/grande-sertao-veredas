//! veredas-aranha, the network scanner (`docs/aranha-plan.md`): who is on an industrial Ethernet
//! segment, how each device names itself and who is cabled to whom, from what the wire says, with
//! the evidence behind every value; for setup and fault-finding, never while the plant produces.
//!
//! Today it holds the PROFINET codecs (`profinet`), the wire (`wire`: the raw link, the capture
//! file, the memory hub) and the private network a test makes for the raw link (`netns`); the
//! listener, fusion, the topology and the scan file follow, phase by phase.

#[cfg(target_os = "linux")]
pub mod netns;
pub mod profinet;
pub mod wire;
