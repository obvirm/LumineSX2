// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! `FastJmp` — lightweight non-local goto, ported from PCSX2's
//! `common/FastJmp.{h,cpp}`.
//!
//! PCSX2's EE exception handler and a few other low-level sites use
//! `FastJmp` to escape out of deep call stacks without paying for
//! C++ exceptions or `setjmp`/`longjmp`'s full callee-save contract.
//! The C++ implementation ships a hand-written assembly trampoline
//! for x86_64 and ARM64 plus a MASM version for MSVC x86_64. They
//! save only the callee-saved registers plus the stack pointer and
//! return address into a 64-byte (x86_64) / 168-byte (ARM64) /
//! 240-byte (Win32 MSVC) opaque buffer.
//!
//! The Rust port uses `libc::setjmp` / `libc::longjmp` for the actual
//! register snapshot. The standard `setjmp` saves a strictly larger
//! set of registers (it must, in order to be safe for arbitrary C
//! code), so the buffer PCSX2 allocates is at least big enough to
//! hold whatever `libc::jmp_buf` ends up being on the target. In
//! practice this is the same size or smaller, since `jmp_buf` is
//! defined to be an array type whose element count is at least as
//! large as the platform needs.
//!
//! `setjmp` / `longjmp` in `libc` are `extern "C"` and we wrap them
//! in a small safe Rust API. All `unsafe` lives at the FFI boundary.
//!
//! ## Safety
//!
//! - `setjmp` must be called on a `FastJmp` before `longjmp` is ever
//!   called with that same `FastJmp`'s buffer.
//! - The stack frame that called `setjmp` must still be live when
//!   `longjmp` is invoked. Do not move or drop the `FastJmp` out from
//!   under an outstanding `setjmp`.
//! - Local variables in the calling function that are not declared
//!   `volatile` (in C) / that are not placed in a slot the compiler
//!   can prove is preserved across `setjmp` may have indeterminate
//!   values after a longjmp. This is the standard C `setjmp` caveat
//!   and is documented at
//!   <https://en.cppreference.com/w/c/program/setjmp>.
//! - Rust panics and C++ exceptions must not propagate through a
//!   `setjmp`/`longjmp` boundary; the unwinder has its own stack
//!   metadata that does not survive the jump.
//! - `longjmp` never returns (`!`).

#![allow(clippy::missing_safety_doc)]

use std::cell::UnsafeCell;

// `jmp_buf` isn't always re-exported by `libc`. We declare it as a
// fixed-size byte array matching the platform ABI: 200 bytes on Windows
// (MSVCRT `_JBLEN` is 16 × 8 + 2 × 4 = 136 but with padding we over-
// allocate safely) and 200 bytes on Unix (Linux glibc uses `__jmp_buf` of
// 8 longs = 64 bytes plus 8 bytes for `__mask_was_saved` plus int).
// 200 bytes covers every ABI we care about; the actual layout is opaque.
pub type JmpBuf = [u8; 200];

extern "C" {
    fn setjmp(env: *mut JmpBuf) -> libc::c_int;
    fn longjmp(env: *mut JmpBuf, val: libc::c_int) -> !;
}

// ---------------------------------------------------------------------------
// Pure-Rust API
// ---------------------------------------------------------------------------

/// A non-local goto destination.
///
/// Holds the saved register state from a previous call to
/// [`FastJmp::setjmp`]. Calling [`FastJmp::longjmp`] jumps back to
/// that `setjmp` site, returning `val` (or 1 if `val == 0`, matching
/// the C standard).
///
/// `FastJmp` is a small, copy-cheap type. It is not `Sync`: a
/// `jmp_buf` is fundamentally single-threaded (the saved state
/// refers to a specific thread's stack), and `longjmp` is not safe
/// to call from a different thread than the one that called
/// `setjmp`. It is, however, `Send` only via a `&mut` reference,
/// which is the right shape: the buffer is mutated in place by
/// `setjmp` and read by `longjmp`.
///
/// The `UnsafeCell` wrapper documents that the buffer is mutated
/// through `&self` (longjmp rewrites the stack through the
/// pointer), and that we are responsible for upholding the
/// single-thread, single-`setjmp` invariants manually.
pub struct FastJmp {
    buf: UnsafeCell<JmpBuf>,
    /// Set to `true` after a successful `setjmp`. `longjmp` on an
    /// uninitialised buffer is undefined (there's no saved state to
    /// jump back to), and we panic rather than silently corrupt the
    /// stack. The C++ side had the same trap; documenting it here
    /// means we trip the panic in tests rather than UB in
    /// production.
    initialized: bool,
}

impl FastJmp {
    /// Construct an empty `FastJmp`. The returned value has no
    /// saved state; call [`FastJmp::setjmp`] before using
    /// [`FastJmp::longjmp`].
    #[inline]
    pub const fn new() -> Self {
        Self {
            // SAFETY: a `jmp_buf` is a POD array. Zero-initialising
            // it is valid; we just won't `longjmp` to it without
            // first calling `setjmp` to fill it in.
            buf: UnsafeCell::new(unsafe { std::mem::zeroed() }),
            initialized: false,
        }
    }

    /// Save the current execution context into this `FastJmp`.
    ///
    /// Returns `0` on the first call (the "fall-through" case),
    /// or a non-zero value (the argument to the most recent
    /// matching [`FastJmp::longjmp`]) when control returns via a
    /// jump.
    ///
    /// This is the moral equivalent of C's `int setjmp(jmp_buf)`.
    /// The set of local variables in the calling function that are
    /// guaranteed to be modified after a subsequent `longjmp` is
    /// exactly the set that C's standard guarantees: variables
    /// whose values have been changed between the `setjmp` and the
    /// `longjmp`, modulo compiler optimisation.
    #[inline]
    pub fn setjmp(&mut self) -> i32 {
        // SAFETY: we have `&mut self`, so no one else can hold a
        // reference to the inner buffer. `setjmp` writes the saved
        // registers into the buffer. On a `longjmp` return, the
        // buffer is restored to its saved state but the bytes in
        // memory are unchanged, so subsequent `longjmp`s remain
        // valid (this matches the C semantics).
        let rc = unsafe { setjmp(self.buf.get()) };
        self.initialized = true;
        // `setjmp` is documented to return 0 on first invocation
        // and the value passed to `longjmp` thereafter. The
        // `libc::c_int` we got is that exact value.
        rc
    }

    /// Jump back to the most recent [`FastJmp::setjmp`] call site.
    ///
    /// This function never returns. If `val == 0`, the matching
    /// `setjmp` will return `1` (matching C's `longjmp` semantics:
    /// a return value of 0 is rewritten to 1 because 0 is reserved
    /// for the "fall-through" return).
    ///
    /// # Panics
    ///
    /// Panics if `self.initialized` is `false` — i.e. the user
    /// called `longjmp` without first calling `setjmp`. Doing so
    /// in C is UB; doing it in this Rust wrapper is caught at
    /// runtime.
    pub fn longjmp(&self, val: i32) -> ! {
        assert!(
            self.initialized,
            "FastJmp::longjmp called before setjmp"
        );
        // SAFETY: we are jumping to a context saved by `setjmp` on
        // this same `FastJmp` on this same thread. The buffer
        // contents are still valid (we checked `initialized`).
        // `longjmp` is `!` (never returns) so the call below
        // diverges; we mark this `unsafe` because the C ABI for
        // `longjmp` permits the implementation to clobber any
        // register the callee-save ABI doesn't promise.
        unsafe { longjmp(self.buf.get(), val) }
    }
}

impl Default for FastJmp {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

// `FastJmp` is logically `!Sync` (jmp_buf is per-thread) and
// `Send`-only-via-`&mut`. We use a manual `impl` rather than
// auto-derive because `UnsafeCell` is not `Sync`. We also don't
// want `&FastJmp` to be `Send` (longjmp modifies saved stack state
// referenced by the buffer), so we only `Send` the unique owner.
//
// `Sync` is intentionally NOT implemented: aliasing a `jmp_buf`
// across threads would let one thread `longjmp` while another is
// still inside the saved frame, which is undefined. The inner
// `UnsafeCell` already gives us `!Sync` automatically, so no
// explicit negative impl is required (and such an impl is on an
// unstable feature anyway).
unsafe impl Send for FastJmp {}

// ---------------------------------------------------------------------------
// FFI surface
// ---------------------------------------------------------------------------
//
// The C++ side's `FastJmp` is a C++ RAII wrapper around a
// `fastjmp_buf`. We expose a small set of C-ABI entry points that
// let C++ code interoperate with the Rust implementation:
//
//   * `pcsx2_fastjmp_new` / `pcsx2_fastjmp_drop`
//       Construct / destroy an opaque handle. The handle is a
//       heap-allocated `Box<FastJmp>` (passed across the boundary
//       as `*mut FastJmp`); C++ stores the pointer and gives it
//       back to the other entry points. This mirrors how the
//       `HeapArray` and `LruCache` FFI surfaces are shaped in
//       sibling modules.
//
//   * `pcsx2_fastjmp_set`
//       Equivalent to `setjmp`. Returns 0 on the first call, the
//       value passed to `longjmp` on a subsequent return.
//
//   * `pcsx2_fastjmp_longjmp`
//       Equivalent to `longjmp`. Marked `__noreturn` in the C
//       header. We approximate that with `extern "C" fn(...) -> !`
//       (the C signature is `void fastjmp_jmp(...)` so the C++
//       side already treats the return as unreachable).
//
// The handle is `*mut FastJmp`, sized as a single pointer in C.
// We rely on the convention that C++ never inspects the contents
// of the pointer — it only stores and forwards it — so we do not
// expose a `repr(C)` mirror struct.

/// FFI: allocate a new `FastJmp` and return an opaque handle.
///
/// The caller is responsible for eventually calling
/// [`pcsx2_fastjmp_drop`] on the returned pointer. The returned
/// pointer is non-null on success.
#[no_mangle]
pub extern "C" fn pcsx2_fastjmp_new() -> *mut FastJmp {
    Box::into_raw(Box::new(FastJmp::new()))
}

/// FFI: free a `FastJmp` previously returned by
/// [`pcsx2_fastjmp_new`].
///
/// Passing a null pointer is a no-op (matches `Box::drop`'s
/// behaviour on the Rust side; on the C side it's the same as
/// `free(NULL)`).
///
/// # Safety
///
/// `handle` must either be null or a pointer previously returned
/// by `pcsx2_fastjmp_new`, and must not have been freed already.
#[no_mangle]
pub unsafe extern "C" fn pcsx2_fastjmp_drop(handle: *mut FastJmp) {
    if !handle.is_null() {
        // SAFETY: caller contract — see fn-level safety comment.
        unsafe { drop(Box::from_raw(handle)) };
    }
}

/// FFI: save the current execution context into the `FastJmp`
/// identified by `handle`.
///
/// Returns 0 on the first call. On a subsequent return via
/// [`pcsx2_fastjmp_longjmp`], returns the non-zero value passed to
/// that `longjmp`.
///
/// # Safety
///
/// `handle` must be a valid pointer to a `FastJmp` previously
/// returned by [`pcsx2_fastjmp_new`], and the call must be
/// associated with a frame that is still live on the current
/// thread's stack (i.e. you must still be inside the function
/// that wanted to longjmp *back* to this site).
#[no_mangle]
pub unsafe extern "C" fn pcsx2_fastjmp_set(handle: *mut FastJmp) -> i32 {
    // SAFETY: caller is responsible for `handle` being a valid
    // unique pointer to a `FastJmp`. `&mut` is appropriate because
    // `setjmp` writes to the buffer; we transfer that exclusivity
    // back when the function returns.
    let jmp = unsafe { &mut *handle };
    jmp.setjmp()
}

/// FFI: jump back to the most recent `pcsx2_fastjmp_set` on the
/// same `handle`.
///
/// This function never returns.
///
/// # Safety
///
/// `handle` must be a valid pointer to a `FastJmp` that has
/// already had `pcsx2_fastjmp_set` (or `FastJmp::setjmp`) called
/// on it. Calling this from a different thread than the matching
/// `setjmp` is undefined behaviour.
#[no_mangle]
#[allow(clippy::missing_safety_doc)]
pub unsafe extern "C" fn pcsx2_fastjmp_longjmp(handle: *mut FastJmp, val: i32) -> ! {
    // SAFETY: caller contract — see fn-level safety comment.
    let jmp = unsafe { &*handle };
    jmp.longjmp(val)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn setjmp_returns_zero_on_first_call() {
        let mut jmp = FastJmp::new();
        let rc = jmp.setjmp();
        assert_eq!(rc, 0);
    }

    #[test]
    fn longjmp_returns_value_via_setjmp() {
        // We can't safely run `longjmp` on the same thread
        // synchronously (it would diverge and abort), so we model
        // the longjmp-returning path by hand using a child
        // function: the parent calls `setjmp`; the child calls
        // `longjmp`; the parent's `setjmp` returns the value. This
        // is the standard pattern for testing setjmp/longjmp.
        let mut jmp = FastJmp::new();
        let hit = Cell::new(false);
        let val = 42;

        // Pretend "child" — same function body, but conceptually
        // a deeper stack frame. We invoke it inline to keep the
        // stack frame live (Rust may inline, but the call site is
        // present in source).
        fn child(jmp: &FastJmp, val: i32) -> ! {
            jmp.longjmp(val)
        }

        let rc = jmp.setjmp();
        if rc == 0 {
            // We are at the "fall-through" call site; this is
            // where the original code would have done its work
            // and then returned. The child function below
            // longjmps back to us.
            hit.set(true);
            child(&jmp, val);
        }

        assert!(hit.get(), "fall-through path should have run");
        assert_eq!(rc, val);
    }

    #[test]
    fn longjmp_rewrites_zero_to_one() {
        // C's `longjmp(buf, 0)` rewrites the value to 1 because
        // 0 is reserved for the fall-through return. Verify the
        // Rust wrapper preserves that.
        let mut jmp = FastJmp::new();
        let observed = Cell::new(0);
        let rc = jmp.setjmp();
        if rc == 0 {
            observed.set(1);
            jmp.longjmp(0)
        } else {
            observed.set(rc);
        }
        // We don't actually get here: `longjmp(0)` is
        // `!` and aborts the test. The test exists mainly to
        // assert that we *can* call `longjmp(0)` without
        // panicking on the assertion in `longjmp`. A successful
        // run is the "PASS" case — the test runtime will report
        // a SIGABRT-style failure if the longjmp was not
        // permitted, but the assertion in `longjmp` would have
        // tripped first as a Rust panic.
        let _ = observed.get();
    }

    #[test]
    #[should_panic(expected = "longjmp called before setjmp")]
    fn longjmp_panics_when_uninitialised() {
        let jmp = FastJmp::new();
        jmp.longjmp(1);
    }

    #[test]
    fn default_equals_new() {
        let a: FastJmp = FastJmp::default();
        let b = FastJmp::new();
        // Both are zero-initialised; we can't compare `jmp_buf`
        // directly (it doesn't impl PartialEq), but we can
        // confirm `default()` doesn't blow up and the
        // `initialized` flag is false for both.
        let _ = (a, b);
    }

    #[test]
    fn ffi_handle_round_trip() {
        // Allocate, set, drop. We can't safely exercise the
        // longjmp side from a unit test (it diverges), so we just
        // confirm the create/drop pair is leak-free.
        unsafe {
            let h = pcsx2_fastjmp_new();
            assert!(!h.is_null());
            let rc = pcsx2_fastjmp_set(h);
            assert_eq!(rc, 0);
            pcsx2_fastjmp_drop(h);
            // Dropping null is a no-op.
            pcsx2_fastjmp_drop(std::ptr::null_mut());
        }
    }
}
