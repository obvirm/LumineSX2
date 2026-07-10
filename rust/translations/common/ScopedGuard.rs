//! RAII guard that runs a cleanup closure when dropped.
//!
//! Idiomatic Rust translation of the C++ `ScopedGuard<T>` template
//! from `common/ScopedGuard.h`. The guard stores a `FnOnce()`
//! closure in an `Option<F>` and invokes it when the guard is
//! dropped - including on early `return` or panic - unless the
//! closure has already been consumed by [`ScopedGuard::run`] or
//! suppressed by [`ScopedGuard::cancel`].
//!
//! Use [`scopeguard!`] to bind a guard to a named identifier, or
//! [`defer!`] for an unnamed one whose only purpose is to fire on
//! scope exit. [`ScopeExit`] is a newtype wrapper that conveys
//! "run on scope exit" intent at the call site.
//!
//! # Example
//!
//! ```ignore
//! use translations::common::{defer, scopeguard};
//!
//! let mut log = String::new();
//! {
//!     scopeguard!(enter, { log.push_str("enter\n"); });
//!     defer!({ log.push_str("exit\n"); });
//!     log.push_str("body\n");
//! }
//! // log == "enter\nbody\nexit\n"
//! ```

/// RAII guard which invokes a stored closure when dropped.
///
/// This is the direct Rust translation of the C++ `ScopedGuard<T>`
/// class. The closure is held in an `Option<F>` so it can be moved
/// out (via [`run`](Self::run)) or discarded (via
/// [`cancel`](Self::cancel)) before the guard drops.
///
/// Drop order matches the C++ version: when the guard is dropped,
/// any unconsumed closure is invoked exactly once.
pub struct ScopedGuard<F: FnOnce()> {
    func: Option<F>,
}

impl<F: FnOnce()> ScopedGuard<F> {
    /// Creates a new guard wrapping the supplied closure.
    ///
    /// Mirrors the C++ rvalue constructor: the closure is taken
    /// by value and the guard assumes ownership of it.
    #[inline]
    pub const fn new(func: F) -> Self {
        Self { func: Some(func) }
    }

    /// Runs the stored closure immediately, if it has not already
    /// been consumed.
    ///
    /// After `run` returns the guard is "spent" and the
    /// destructor becomes a no-op. This corresponds to the C++
    /// `Run()` method, which executes the destructor function
    /// early and nulls it out.
    #[inline]
    pub fn run(&mut self) {
        if let Some(f) = self.func.take() {
            f();
        }
    }

    /// Cancels the guard, preventing the closure from being
    /// invoked when the guard is dropped.
    ///
    /// Equivalent to the C++ `Cancel()` method: the stored
    /// closure is dropped without being called.
    #[inline]
    pub fn cancel(&mut self) {
        let _ = self.func.take();
    }
}

impl<F: FnOnce()> Drop for ScopedGuard<F> {
    #[inline]
    fn drop(&mut self) {
        self.run();
    }
}

/// Newtype wrapper that signals "run on scope exit" intent.
///
/// Behaves identically to [`ScopedGuard`], but the name makes
/// the guard's purpose explicit at the call site: the closure is
/// intended to run on scope exit (cleanup, logging, releasing a
/// resource) rather than to be cancelled or consumed early.
pub struct ScopeExit<F: FnOnce()>(pub ScopedGuard<F>);

impl<F: FnOnce()> ScopeExit<F> {
    /// Creates a new `ScopeExit` wrapping the supplied closure.
    #[inline]
    pub const fn new(func: F) -> Self {
        Self(ScopedGuard::new(func))
    }
}

impl<F: FnOnce()> Drop for ScopeExit<F> {
    #[inline]
    fn drop(&mut self) {
        self.0.run();
    }
}

/// Declares a [`ScopedGuard`] bound to the given identifier.
///
/// The closure body is a block expression; a trailing semicolon
/// is optional. The macro must be called from a scope where
/// `let` bindings are accepted.
///
/// The guard is bound to `$name` with `let`, so it is dropped at
/// the end of the enclosing scope (or earlier, if the binding is
/// shadowed or the scope is left via `?` / `return`).
///
/// # Example
///
/// ```ignore
/// scopeguard!(cleanup, {
///     // runs at end of scope
/// });
/// ```
#[macro_export]
macro_rules! scopeguard {
    ($name:ident, $body:block) => {
        let $name = $crate::ScopedGuard::new(move || $body);
    };
}

/// Unnamed scope-exit guard. Equivalent to
/// `scopeguard!(__defer_guard, { ... })`.
///
/// The binding name is mangled so that multiple `defer!`s in the
/// same scope do not collide. The closure runs when the
/// surrounding scope exits, including early `return`.
///
/// # Example
///
/// ```ignore
/// defer!({
///     // runs at end of scope
/// });
/// ```
#[macro_export]
macro_rules! defer {
    ($body:block) => {
        let __defer_guard = $crate::ScopedGuard::new(move || $body);
    };
}
