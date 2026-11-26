//! The tests' allocator: the system's, refusing any one allocation past a ceiling. A regression that lays out what a
//! program or a host names, past the loader's checks, then aborts its test binary (a failed run), and never fills the
//! machine the tests run on (`docs/orcl/README.md`, "Traps").
#![allow(dead_code)]
use std::alloc::{GlobalAlloc, Layout, System};

/// One allocation's ceiling: 1 GiB, far past what any test here lays out on purpose.
pub const CEILING: usize = 1 << 30;

pub struct Capped;

// SAFETY: every call within the ceiling is handed to the system's allocator unchanged; one past it is refused with a
// null pointer, which the standard library's collections answer with `handle_alloc_error`, an abort.
unsafe impl GlobalAlloc for Capped {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if layout.size() > CEILING {
            return std::ptr::null_mut();
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if new_size > CEILING {
            return std::ptr::null_mut();
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}
