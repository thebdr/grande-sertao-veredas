//! veredas-aranha, the network scanner (`docs/aranha-plan.md`): who is on an industrial Ethernet
//! segment, how each device names itself and who is cabled to whom, from what the wire says, with
//! the evidence behind every value; for setup and fault-finding, never while the plant produces.
//!
//! Today it holds the PROFINET codecs (`profinet`), the wire (`wire`: the raw link, the capture
//! file, the memory hub), the private network a test makes for the raw link (`netns`), what a
//! frame says (`observe`), the devices it adds up to (`fusion`) and the listener that hears them
//! (`listen`); the topology and the scan file follow, phase by phase.

#[cfg(target_os = "linux")]
pub mod netns;
pub mod fusion;
pub mod listen;
pub mod observe;
pub mod profinet;
pub mod wire;
