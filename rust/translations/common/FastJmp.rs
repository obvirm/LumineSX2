// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Cooperative context-switch primitive translated from PCSX2's
//! `common/FastJmp.{h,cpp}`.
//!
//! The original C/C++ implementation provides a small, register-only
//! `setjmp`/`longjmp`-style facility used to bounce control between coroutine
//! stacks without the cost or full semantics of the standard C library
//! routines (notably, signal mask preservation is intentionally omitted).
//!
//! This Rust module exposes a [`FastJmpBuf`] storage type and a [`Context`]
//! trait offering `set` and `long_jump` operations. Saving the current
//! context is `unsafe` (matching the C ABI, which can clobber the caller's
//! non-preserved registers), and the implementation is gated to Unix
//! platforms, where it is backed by `libc::setjmp` / `libc::longjmp`.

use std::pin::Pin;

#[cfg(unix)]
use std::mem::size_of;

/// Number of bytes required to store a saved execution context on the current
/// platform. Mirrors `fastjmp_buf::BUF_SIZE` from the C header.
///
/// The value matches the platform's `jmp_buf` footprint:
/// - macOS / Linux x86_64: 200 bytes (matches glibc/BSD `jmp_buf`)
/// - other Unix: 200 bytes (conservative upper bound for the SysV layout)
#[cfg(unix)]
pub const FASTJMP_BUF_SIZE: usize = size_of::<libc::jmp_buf>();

/// Aligned, fixed-size storage for a saved execution context.
///
/// The buffer is plain old data: the caller places it in some stable
/// location (stack, heap, or part of a coroutine frame) and pins it before
/// passing a `Pin<&mut Self>` to [`Context::set`].
#[cfg(unix)]
#[repr(C)]
#[repr(align(16))]
pub struct FastJmpBuf {
    pub buf: [u8; FASTJMP_BUF_SIZE],
}

#[cfg(unix)]
impl FastJmpBuf {
    /// Returns a zero-initialised buffer. A zeroed buffer is not a valid
    /// saved context; it must be populated by [`Context::set`] before use.
    pub const fn new() -> Self {
        Self {
            buf: [0u8; FASTJMP_BUF_SIZE],
        }
    }
}

#[cfg(unix)]
impl Default for FastJmpBuf {
    fn default() -> Self {
        Self::new()
    }
}

/// Cooperative context-switch API, modelled directly on the C
/// `fastjmp_set` / `fastjmp_jmp` pair.
#[cfg(unix)]
pub trait Context {
    /// Saves the current execution context into `buf` and returns `0` on
    /// the first call.
    ///
    /// When the same buffer is later restored via [`Context::long_jump`],
    /// this function "returns" a second time with the value passed to
    /// `long_jump` (i.e. it behaves like C `setjmp`).
    ///
    /// # Safety
    ///
    /// The caller must ensure that `buf` outlives the saved context and is
    /// not aliased mutably for the duration. Specifically:
    /// - `buf` must be pinned ([`Pin`]).
    /// - The location referenced by `buf` must remain valid until either
    ///   [`Context::long_jump`] is invoked from this stack or the buffer is
    ///   discarded without ever being jumped back to.
    /// - The stack pointer captured at save time must still be live when
    ///   `long_jump` restores it; in practice this means the calling frame
    ///   (and anything it relies on, including local variables used after
    ///   the call) must not have returned.
    unsafe fn set(buf: Pin<&mut FastJmpBuf>) -> i32;

    /// Restores the execution context previously saved in `buf`, causing
    /// the corresponding [`Context::set`] call to "return" with `val`.
    ///
    /// This function never returns on success.
    ///
    /// # Safety
    ///
    /// - `buf` must have been populated by a prior call to
    ///   [`Context::set`] that has not yet been long-jumped to.
    /// - The stack frame active at the original `set` call must still be
    ///   live (in particular, the calling function must not have returned).
    /// - All values live across the jump (local variables still in use,
    ///   references captured by closures, etc.) must be `Unpin` in spirit
    ///   or otherwise safe to abandon, because Rust's borrow checker
    ///   cannot reason about the post-jump stack.
    unsafe fn long_jump(buf: &FastJmpBuf, val: i32) -> !;
}

/// Unix implementation backed by the platform `setjmp` / `longjmp` from
/// `libc`. This matches the spirit of the C++ side, which uses the same
/// routines on Linux and macOS and platform-specific asm on Windows.
#[cfg(unix)]
pub struct UnixContext;

#[cfg(unix)]
impl Context for UnixContext {
    unsafe fn set(buf: Pin<&mut FastJmpBuf>) -> i32 {
        // `setjmp` returns 0 on the initial return and the value passed to
        // `longjmp` on the second return. We use the C `setjmp` (not
        // `sigsetjmp`) because the original PCSX2 implementation does not
        // preserve the signal mask either.
        unsafe { libc::setjmp(buf.get_unchecked_mut().buf.as_mut_ptr().cast()) }
    }

    unsafe fn long_jump(buf: &FastJmpBuf, val: i32) -> ! {
        // `longjmp` is `extern "C"` and marked `noreturn`; the `!` return
        // type on this wrapper preserves that for the Rust caller.
        unsafe { libc::longjmp(buf.buf.as_ptr().cast(), val) }
    }
}

/// Default context-switch implementation used by the convenience
/// functions on this module.
#[cfg(unix)]
pub type DefaultContext = UnixContext;

/// Saves the current execution context using [`DefaultContext`].
///
/// See [`Context::set`] for safety requirements.
#[cfg(unix)]
pub unsafe fn set(buf: Pin<&mut FastJmpBuf>) -> i32 {
    unsafe { <DefaultContext as Context>::set(buf) }
}

/// Restores the execution context previously saved in `buf` using
/// [`DefaultContext`].
///
/// See [`Context::long_jump`] for safety requirements.
#[cfg(unix)]
pub unsafe fn long_jump(buf: &FastJmpBuf, val: i32) -> ! {
    unsafe { <DefaultContext as Context>::long_jump(buf, val) }
}
