// perf.rs — Stub implementation. All profiler registration is no-op.
// Real implementation deferred to platform-specific backends.

use std::sync::OnceLock;
use std::time::Instant;

/// Predefined profiler groups matching C++ Perf::Group enum.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PerfGroup {
    Any = 0,
    EE = 1,
    IOP = 2,
    VU0 = 3,
    VU1 = 4,
    VIF = 5,
}

/// RAII guard that records a duration when dropped.
pub struct PerfScope {
    _start: Instant,
    _group: PerfGroup,
}

impl PerfScope {
    pub fn new(group: PerfGroup) -> Self {
        Self {
            _start: Instant::now(),
            _group: group,
        }
    }
}

impl Drop for PerfScope {
    fn drop(&mut self) {
        // no-op stub
    }
}

/// Initialize perf subsystem. Returns false (no profiler active).
pub fn perf_init() -> bool {
    false
}

/// Shutdown perf subsystem.
pub fn perf_shutdown() {}

/// Register a code region with a human-readable symbol name.
pub fn perf_register(_group: PerfGroup, _addr: *const std::ffi::c_void, _size: u64, _name: *const std::ffi::c_char) {}

/// Register a code region with a hex PC label.
pub fn perf_register_pc(_group: PerfGroup, _addr: *const std::ffi::c_void, _size: u64, _pc: u32) {}

/// Register a code region with a 64-bit key.
pub fn perf_register_key(_group: PerfGroup, _addr: *const std::ffi::c_void, _size: u64, _key: u64, _name: *const std::ffi::c_char) {}

// ── FFI exports ──────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn pcsx2_perf_init() -> bool {
    perf_init()
}

#[no_mangle]
pub extern "C" fn pcsx2_perf_shutdown() {
    perf_shutdown()
}

#[no_mangle]
pub extern "C" fn pcsx2_perf_group_register(group: i32, addr: *const std::ffi::c_void, size: u64, name: *const std::ffi::c_char) {
    perf_register(unsafe { std::mem::transmute(group) }, addr, size, name);
}

#[no_mangle]
pub extern "C" fn pcsx2_perf_group_register_pc(group: i32, addr: *const std::ffi::c_void, size: u64, pc: u32) {
    perf_register_pc(unsafe { std::mem::transmute(group) }, addr, size, pc);
}

#[no_mangle]
pub extern "C" fn pcsx2_perf_group_register_key(group: i32, addr: *const std::ffi::c_void, size: u64, key: u64, name: *const std::ffi::c_char) {
    perf_register_key(unsafe { std::mem::transmute(group) }, addr, size, key, name);
}
