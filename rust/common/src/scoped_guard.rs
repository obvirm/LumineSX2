//! `ScopedGuard` — RAII guard that runs a closure on drop.
//!
//! This is a Rust port of PCSX2's C++ `common/ScopedGuard.h`. Unlike the
//! upstream C++ class, this Rust version does not need a third-party
//! `scopeguard` crate: Rust's native `Drop` trait plus an `Option<F>`
//! give us exactly the same semantics in a few lines.
//!
//! ## Semantics
//!
//! - `ScopedGuard::new(f)` constructs a guard holding `f`.
//! - When the guard is dropped, the held closure runs **unless** it has
//!   already been consumed by [`ScopedGuard::run`] or disarmed by
//!   [`ScopedGuard::cancel`].
//! - [`ScopedGuard::run`] consumes `self` and runs the closure immediately,
//!   preventing it from running again at drop time.
//! - [`ScopedGuard::cancel`] consumes `self` and discards the closure
//!   without running it.
//!
//! The guard is `!Clone` and not `Copy`; closures generally aren't.
//!
//! ## Macro
//!
//! [`pcsx2_scoped_guard!`] is provided as a convenience that captures
//! the body as a closure and returns a [`ScopedGuard`]:
//!
//! ```
//! use pcsx2_common_rs::{pcsx2_scoped_guard, ScopedGuard};
//!
//! let _g: ScopedGuard<Box<dyn FnOnce()>> =
//!     pcsx2_scoped_guard!(println!("ran"));
//! ```
//!
//! ## FFI
//!
//! `ScopedGuard` is a Rust-internal convenience. The C++ side has its
//! own `ScopedGuard<T>` class and never crosses the FFI boundary with
//! a Rust closure — Rust closures are not `extern "C"` representable,
//! so any FFI export would be vacuous. We expose an opaque zero-sized
//! type (`Pcsx2ScopedGuard`) for completeness in case future work
//! wants to coordinate lifetimes across the boundary, but no `Run`
//! style helper makes sense: there is nothing to call from C.

#![allow(clippy::needless_pass_by_value)]

/// RAII guard that runs the contained closure when dropped.
///
/// See the [module-level documentation](self) for full semantics.
pub struct ScopedGuard<F: FnOnce()> {
    func: Option<F>,
}

impl<F: FnOnce()> ScopedGuard<F> {
    /// Construct a guard that will run `f` when dropped.
    #[inline]
    pub fn new(f: F) -> Self {
        Self { func: Some(f) }
    }

    /// Run the contained closure now, consuming `self`. The closure
    /// will **not** run again on drop.
    #[inline]
    pub fn run(mut self) {
        if let Some(f) = self.func.take() {
            f();
        }
    }

    /// Cancel the contained closure without running it, consuming `self`.
    /// The closure will **not** run on drop.
    #[inline]
    pub fn cancel(mut self) {
        self.func = None;
    }

    /// Returns `true` if the guard still holds a closure to run.
    #[inline]
    pub fn is_active(&self) -> bool {
        self.func.is_some()
    }
}

impl<F: FnOnce()> Drop for ScopedGuard<F> {
    #[inline]
    fn drop(&mut self) {
        if let Some(f) = self.func.take() {
            f();
        }
    }
}

/// Convenience macro: `pcsx2_scoped_guard!(expr)` returns a
/// [`ScopedGuard`] that will run the expression when the guard is dropped.
///
/// The argument is captured as a single expression and wrapped in a
/// `|| { ... }` closure that captures by **shared reference** by default.
/// That matches how PCSX2's C++ `MAKE_SCOPED_GUARD` lambda is typically
/// used (the body reads from the surrounding scope). If the body needs
/// ownership, capture explicitly with a `move || { ... }` expression:
///
/// ```
/// # use pcsx2_common_rs::{pcsx2_scoped_guard, ScopedGuard};
/// // Bare expression body (captures by ref):
/// let _g: ScopedGuard<Box<dyn FnOnce()>> =
///     pcsx2_scoped_guard!(println!("ran"));
///
/// // Block body (a `{ ... }` is itself an expression):
/// let _g: ScopedGuard<Box<dyn FnOnce()>> =
///     pcsx2_scoped_guard!({ println!("ran"); });
///
/// // `move` closure when the body needs owned captures:
/// let owned = String::from("hello");
/// let _g: ScopedGuard<Box<dyn FnOnce()>> =
///     pcsx2_scoped_guard!(move || drop(owned));
/// ```
///
/// Mirrors PCSX2's C++ `MAKE_SCOPED_GUARD` macro (which lives elsewhere
/// in the C++ tree) without requiring a third-party `scopeguard`
/// crate — `Drop` + `Option` is the entire implementation.
#[macro_export]
macro_rules! pcsx2_scoped_guard {
    ($body:expr) => {
        $crate::scoped_guard::ScopedGuard::new(|| { $body })
    };
}

// ---------------------------------------------------------------------------
// FFI surface
// ---------------------------------------------------------------------------
//
// `ScopedGuard` carries a Rust closure (`FnOnce()`), which is **not**
// `extern "C"` representable — closures carry captured environments
// that have no stable C ABI. The C++ side already has its own
// `ScopedGuard<T>` class and continues to use it; there is no need
// to ferry Rust guards across the boundary. We therefore export only
// an opaque type handle, mirroring the shape PCSX2's existing
// `Pcsx2Defs.h` machinery uses for similar "mark a lifetime" tokens.
//
// If a future porting step ever needs to coordinate a guard with C++,
// the right tool is to create the guard on whichever side owns the
// resource and to run a C-callable teardown function from the
// opposite side. That requires a `fn() extern "C" + Send`, not a
// Rust closure, and is out of scope for this module.

/// Opaque handle type for FFI use. Zero-sized; the real guard lives
/// on the Rust side and is never visible to C/C++.
#[repr(C)]
pub struct Pcsx2ScopedGuard {
    _private: [u8; 0],
}

impl Pcsx2ScopedGuard {
    /// Marker constructor. The returned pointer is a non-null,
    /// well-aligned "token" that C/C++ code may store to indicate
    /// "a Rust-side guard exists"; it must not be dereferenced.
    #[inline]
    #[allow(dead_code)]
    pub fn token() -> *mut Pcsx2ScopedGuard {
        // A dangling but well-aligned non-null pointer. C/C++ must
        // treat this as opaque; nothing is ever read or written
        // through it on the C side.
        1usize as *mut Pcsx2ScopedGuard
    }
}

// Compile-time assertion: the handle really is ZST. This catches
// accidental field additions that would silently change the C ABI.
const _: [(); 0] = [(); std::mem::size_of::<Pcsx2ScopedGuard>()];

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn runs_on_drop() {
        let fired = Cell::new(false);
        {
            let _g = ScopedGuard::new(|| fired.set(true));
        }
        assert!(fired.get(), "guard should have run on drop");
    }

    #[test]
    fn run_consumes_and_prevents_re_run() {
        let fired = Cell::new(0u32);
        let g = ScopedGuard::new(|| fired.set(fired.get() + 1));
        g.run();
        assert_eq!(fired.get(), 1, "run should fire exactly once");
    }

    #[test]
    fn cancel_skips_run() {
        let fired = Cell::new(false);
        let g = ScopedGuard::new(|| fired.set(true));
        g.cancel();
        assert!(!fired.get(), "cancel should prevent the drop call");
    }

    #[test]
    fn is_active_reports_state() {
        let g = ScopedGuard::new(|| {});
        assert!(g.is_active());
        g.cancel();
    }

    #[test]
    fn macro_runs_on_drop() {
        let fired = std::rc::Rc::new(Cell::new(false));
        {
            let _g = pcsx2_scoped_guard!(fired.set(true));
        }
        assert!(fired.get(), "macro-produced guard should run on drop");
    }

    #[test]
    fn macro_supports_block_body() {
        let fired = std::rc::Rc::new(Cell::new(0u32));
        {
            let _g = pcsx2_scoped_guard!({
                fired.set(fired.get() + 1);
                fired.set(fired.get() + 1);
            });
        }
        assert_eq!(fired.get(), 2);
    }
}
