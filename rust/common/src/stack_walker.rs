// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! `stack_walker` — cross-platform stack trace capture backed by the
//! [`backtrace`] crate.
//!
//! Rust port of PCSX2's C++ `common/StackWalker.{h,cpp}`.
//!
//! The original C++ implementation used Win32 `DbgHelp` API directly
//! (`StackWalk64`, `SymFromAddr`, `SymGetLineFromAddr64`). This Rust
//! version uses the pure-Rust `backtrace` crate which wraps the best
//! available platform mechanism:
//!
//! - **Windows:** `CaptureStackBackTrace` + `DbgHelp` (via `backtrace-rs`).
//! - **Linux:** `libgcc` unwind + `dladdr` (via `backtrace-rs`).
//! - **macOS:** `libunwind` (via `backtrace-rs`).
//!
//! This eliminates the dependency on the `windows` crate for the
//! stack-walker module, which was the reason it was previously disabled
//! (the pinned `windows` crate version had API mismatches).

use std::ffi::c_void;
use std::ptr;

// ---------------------------------------------------------------------------
// Public data type
// ---------------------------------------------------------------------------

/// A single resolved frame in a captured stack trace.
///
/// Mirrors a subset of the C++ `CallstackEntry` struct: we capture the
/// address, the demangled symbol name (if available), and optionally
/// file + line number.
#[repr(C)]
#[derive(Debug, Clone)]
pub struct StackFrame {
    /// Absolute instruction pointer for this frame.
    pub address: u64,
    /// Best-effort demangled symbol name. Empty if not resolvable.
    pub symbol: String,
    /// Source file path, if known.
    pub file: Option<String>,
    /// 1-based line number, if known.
    pub line: Option<u32>,
}

// ---------------------------------------------------------------------------
// Pure-Rust API
// ---------------------------------------------------------------------------

/// Capture up to `max_frames` stack frames from the current call site.
///
/// Returns a `Vec<StackFrame>` ordered from innermost (current) frame to
/// outermost. The first frame is the caller of this function.
///
/// Uses the [`backtrace`] crate internally, which resolves symbol names
/// on all platforms. File/line information is populated when available
/// (requires debug symbols).
pub fn capture_stack_trace(max_frames: usize) -> Vec<StackFrame> {
    let mut bt = backtrace::Backtrace::new_unresolved();
    bt.resolve();

    let mut frames: Vec<StackFrame> = Vec::with_capacity(max_frames);
    for frame in bt.frames().iter().take(max_frames) {
        let addr = frame.ip() as u64;

        // Symbol name: prefer demangled, fall back to the raw name.
        let symbol = frame
            .symbols()
            .iter()
            .find_map(|s| s.name().map(|n| n.to_string()))
            .unwrap_or_default();

        // File/line: backtrace::BacktraceSymbol provides filename()
        // and lineno() when debug info is available.
        let (file, line) = frame
            .symbols()
            .iter()
            .find_map(|s| {
                let f = s.filename().map(|p| p.to_string_lossy().to_string());
                let l = s.lineno();
                if f.is_some() || l.is_some() {
                    Some((f, l))
                } else {
                    None
                }
            })
            .unwrap_or((None, None));

        frames.push(StackFrame {
            address: addr,
            symbol,
            file,
            line,
        });
    }

    frames
}

// ---------------------------------------------------------------------------
// FFI surface
// ---------------------------------------------------------------------------
//
// The C++ side expects two functions:
//   - `pcsx2_stack_capture(max_frames, out_frames, out_count) -> bool`
//   - `pcsx2_stack_free(frames, count)`
//
// `StackFrame` contains a `String`, so it is NOT trivial to copy across
// the FFI boundary inline. Instead we heap-allocate a buffer of
// `count` `StackFrame` records and pass the pointer + count to C++.
// The C++ side then reads them and must eventually call
// `pcsx2_stack_free` to release the memory.

/// Capture up to `max_frames` stack frames and write them to a
/// heap-allocated buffer.
///
/// On success, allocates `count` `StackFrame` records, writes the
/// pointer into `*out_frames`, writes the count into `*out_count`,
/// and returns `true`.
///
/// The caller MUST release the buffer with [`pcsx2_stack_free`].
///
/// Returns `false` on invalid parameters or allocation failure.
#[no_mangle]
pub extern "C" fn pcsx2_stack_capture(
    max_frames: u32,
    out_frames: *mut *mut StackFrame,
    out_count: *mut u32,
) -> bool {
    if max_frames == 0 || out_frames.is_null() || out_count.is_null() {
        return false;
    }

    let frames = capture_stack_trace(max_frames as usize);
    if frames.is_empty() {
        unsafe {
            *out_frames = ptr::null_mut();
            *out_count = 0;
        }
        return true;
    }

    let count = frames.len();

    // Allocate a buffer large enough for all StackFrame records.
    let layout = match std::alloc::Layout::array::<StackFrame>(count) {
        Ok(l) => l,
        Err(_) => return false,
    };

    let raw = unsafe { std::alloc::alloc(layout) as *mut StackFrame };
    if raw.is_null() {
        return false; // OOM
    }

    // Move each frame into the heap buffer. We use ptr::write to
    // transfer ownership of the String fields, then forget the Vec
    // to prevent double-free on drop.
    for (i, frame) in frames.into_iter().enumerate() {
        unsafe {
            ptr::write(raw.add(i), frame);
        }
    }

    unsafe {
        *out_frames = raw;
        *out_count = count as u32;
    }
    true
}

/// Free a buffer previously returned by [`pcsx2_stack_capture`].
///
/// Drops each `StackFrame` (freeing the inner `String` fields) and
/// deallocates the buffer. Passing `null` or `count == 0` is a no-op.
///
/// # Safety
///
/// `frames` must be a valid pointer from `pcsx2_stack_capture`, or null.
#[no_mangle]
pub extern "C" fn pcsx2_stack_free(frames: *mut StackFrame, count: u32) {
    if frames.is_null() || count == 0 {
        return;
    }
    let count = count as usize;
    unsafe {
        // Drop each frame in place to free the String fields.
        for i in 0..count {
            ptr::drop_in_place(frames.add(i));
        }
        // Deallocate the buffer.
        if let Ok(layout) = std::alloc::Layout::array::<StackFrame>(count) {
            std::alloc::dealloc(frames as *mut u8, layout);
        }
    }
}
