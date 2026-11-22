//! The wire: where frames are sent and heard. Today the raw link on a named interface (Linux);
//! the capture file and the memory hub follow with listening (`docs/aranha-plan.md`, phase 4).

#[cfg(target_os = "linux")]
pub mod raw;
