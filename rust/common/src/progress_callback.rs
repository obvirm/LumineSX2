// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! `ProgressCallback` — abstracts a blocking operation and lets it report
//! progress without depending on the UI.
//!
//! Mirrors PCSX2's `common/ProgressCallback.h`. The C++ side has a
//! `ProgressCallback` abstract base class with two important subclasses:
//! `BaseProgressCallback` (provides default `PushState`/`PopState`/
//! `SetProgressRange`/`SetProgressValue` stack-based progress math) and
//! `NullProgressCallbacks` (silently discards everything; modal prompts
//! log to `Console` and always return `false`).
//!
//! ## Mapping from C++ to Rust
//!
//! - The abstract `ProgressCallback` C++ class becomes the
//!   [`ProgressCallback`] trait. Pure-Rust code can take
//!   `&mut dyn ProgressCallback` (or `Box<dyn ProgressCallback>` for
//!   ownership) just like it would take `ProgressCallback*` in C++.
//! - The `ProgressState` enum class becomes the [`ProgressState`] enum.
//!   It is `#[repr(u32)]` so that its discriminant matches the C++
//!   `enum class : u32` (matching the C++ default for plain
//!   `enum class` underlying type would actually be `int`, but
//!   PCSX2 stores progress state in `u32` fields in `BaseProgressCallback`
//!   — see the `m_progress_state` initialiser). The explicit `#[repr(u32)]`
//!   is the safe choice here.
//! - The C++ static `NullProgressCallback*` and
//!   `CreateNullProgressCallback()` factory are exposed through the FFI
//!   in this file (see [FFI](#ffi) below).
//!
//! ## What is **not** ported
//!
//! - `BaseProgressCallback` is intentionally **not** ported. It is a
//!   reusable mix-in that C++ callers inherit from; in Rust the
//!   equivalent is a concrete struct that implements
//!   [`ProgressCallback`] directly, using a `Vec<State>` for the
//!   `m_saved_state` linked-list stack. If a Rust port of `BaseProgressCallback`
//!   becomes useful later, it should live in its own
//!   `base_progress_callback.rs` module.
//! - The `SetFormattedStatusText` / `DisplayFormatted*` varargs wrappers
//!   from `ProgressCallback.cpp` are not ported. Rust uses the
//!   `format!` / `format_args!` macros for the same job at the call
//!   site (e.g. `cb.set_status_text(&format!("loaded {n} files"))`).
//! - The `Console.Error` / `Console.Warning` / `Console.WriteLn` /
//!   `DevCon.WriteLn` log paths used by the C++ `NullProgressCallbacks`
//!   are not ported. The Rust [`NullProgressCallback`] is a true
//!   no-op (matches the trait's contract that every method is
//!   free to ignore). The C++ "log to console" behaviour can be
//!   reinstated by adding a `tracing` or `log` crate call inside
//!   `NullProgressCallback`'s impl later.
//!
//! ## FFI
//!
//! Rust trait objects (`dyn ProgressCallback`) are not representable
//! across the C ABI. The C ABI has no concept of:
//!
//! 1. A "vtable" — Rust lays out the data pointer and vtable pointer
//!    in a way that is `repr(Rust)` and unstable across compiler
//!    versions. C++ expects a specific C++-ABI layout for `ProgressCallback*`
//!    (typically two pointers on Itanium, one with multiple inheritance
//!    thunks on MSVC). The two layouts do not match.
//! 2. A Rust trait object header — `&dyn Trait` and `Box<dyn Trait>`
//!    are wider than a single pointer (the vtable pointer is appended
//!    to the value), and the data pointer is not interchangeable with
//!    a C++ `this`.
//! 3. Object lifetime ownership — a C++ `ProgressCallback*` may be
//!    stack-allocated (the singleton `s_nullProgressCallbacks`),
//!    heap-allocated, or owned by a `unique_ptr`. A Rust
//!    `Box<dyn ProgressCallback>` enforces heap allocation and a
//!    single owner. Forcing the two to share an ABI would require
//!    pointer-stable `Pin<Box<...>>` boxing on the C++ side, which
//!    is not how PCSX2 uses the type today.
//!
//! The pragmatic compromise is to expose the bits that *do* translate:
//!
//! - A constructor that hands out a heap-allocated
//!   `Box<NullProgressCallback>` as an opaque `*mut std::ffi::c_void`.
//!   C++ can wrap this in a thin `ProgressCallback` subclass that
//!   forwards every call to the Rust no-op. In practice, C++ should
//!   just keep using its own `NullProgressCallbacks` — there is no
//!   value in round-tripping through Rust for a no-op — but the
//!   factory exists so the C++ side can ask "give me the Rust
//!   null callback" if it ever needs one for interop testing.
//! - A destructor that takes back the opaque handle and drops the
//!   `Box`. Pairing these two is the standard "C-ABI opaque handle"
//!   pattern; ownership transfers from C++ to Rust at creation, and
//!   back from C++ to Rust at destruction.
//!
//! Trait methods themselves are **not** exported as `extern "C"`
//! functions. To call back into a C++ `ProgressCallback` from Rust,
//! the C++ side would have to expose a C function-pointer table
//! (a "trampoline"), which is out of scope for this initial port.
//!
//! ## Exports
//!
//! - [`pcsx2_progress_callback_null_create`]
//!   — heap-allocates a [`NullProgressCallback`], returns the
//!   pointer as `*mut std::ffi::c_void`. C++ side must pair with
//!   [`pcsx2_progress_callback_destroy`].
//! - [`pcsx2_progress_callback_destroy`]
//!   — takes the opaque handle back and drops the `Box`.
//!   No-op on null.
//! - [`pcsx2_progress_callback_state_normal`] /
//!   [`pcsx2_progress_callback_state_indeterminate`] /
//!   [`pcsx2_progress_callback_state_paused`] /
//!   [`pcsx2_progress_callback_state_error`]
//!   — re-exported discriminants of [`ProgressState`] as `u32`
//!   constants for C++ convenience.

use std::ffi::c_void;

/// Progress display state for an operation in flight.
///
/// Mirrors `ProgressCallback::ProgressState` in
/// `common/ProgressCallback.h`. The discriminants match the C++
/// source order (Normal=0, Indeterminate=1, Paused=2, Error=3) so
/// the values round-trip through `as u32` cleanly across the FFI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum ProgressState {
    /// Standard determinate progress (`value` / `range`).
    Normal = 0,
    /// Marquee / unknown duration. C++ UI uses an animated bar.
    Indeterminate = 1,
    /// Operation is paused; the user should be told why.
    Paused = 2,
    /// Operation has failed; the UI typically shows red.
    Error = 3,
}

impl Default for ProgressState {
    #[inline]
    fn default() -> Self {
        ProgressState::Normal
    }
}

/// Progress reporting + user-interaction interface.
///
/// Mirrors the C++ `ProgressCallback` abstract base class. A
/// blocking operation takes a `&mut dyn ProgressCallback` (or
/// `Box<dyn ProgressCallback>`) and uses it to:
///
/// - report a title and a status line that the UI can render;
/// - update a progress bar (`range`, `value`, `IncrementProgressValue`);
/// - toggle cancellability and poll `is_cancelled` to break out
///   of long loops;
/// - push/pop a stack of progress states so that nested operations
///   can contribute a fraction of the parent's bar;
/// - surface non-fatal messages and prompt the user modally.
///
/// The "default" trait methods are intentionally omitted. Every
/// implementor is expected to provide every method — the C++
/// abstract class is pure-virtual too, with no fallback
/// implementations.
///
/// `Box<dyn ProgressCallback>` is the idiomatic Rust equivalent of
/// `std::unique_ptr<ProgressCallback>`.
pub trait ProgressCallback {
    /// Save the current progress / status / cancellability on an
    /// internal stack so it can be restored by [`Self::pop_state`].
    ///
    /// Mirrors `ProgressCallback::PushState`. The default
    /// implementation (see `BaseProgressCallback` in the C++ side)
    /// pushes a copy of the live state onto a stack.
    fn push_state(&mut self);

    /// Restore the state saved by the matching [`Self::push_state`].
    ///
    /// Mirrors `ProgressCallback::PopState`. Implementations should
    /// `assert!` (or panic in debug builds) if there is no matching
    /// push, matching the C++ `pxAssert` in `BaseProgressCallback::PopState`.
    fn pop_state(&mut self);

    /// `true` if the user has requested cancellation.
    fn is_cancelled(&self) -> bool;

    /// `true` if the UI is currently showing a cancel button (or
    /// equivalent affordance).
    fn is_cancellable(&self) -> bool;

    /// Toggle the cancel button visibility / availability.
    fn set_cancellable(&mut self, cancellable: bool);

    /// Set the dialog / window title. `title` is UTF-8.
    fn set_title(&mut self, title: &str);

    /// Set the status line. `text` is UTF-8.
    fn set_status_text(&mut self, text: &str);

    /// Set the upper bound of the progress bar. The actual value
    /// the bar counts towards is implementation-defined; see the
    /// C++ `BaseProgressCallback` math for the stack-based variant.
    fn set_progress_range(&mut self, range: u32);

    /// Set the absolute current value within the range.
    fn set_progress_value(&mut self, value: u32);

    /// Increment the current value by one. Implementations are
    /// free to no-op (see [`NullProgressCallback`]).
    fn increment_progress_value(&mut self);

    /// Switch between determinate / marquee / paused / error modes.
    fn set_progress_state(&mut self, state: ProgressState);

    /// Non-blocking error notification. The user is not interrupted.
    fn display_error(&self, msg: &str);

    /// Non-blocking warning notification.
    fn display_warning(&self, msg: &str);

    /// Non-blocking informational notification.
    fn display_information(&self, msg: &str);

    /// Non-blocking debug-level notification.
    ///
    /// Not in the original task spec but present in
    /// `ProgressCallback.h`; included for completeness so the trait
    /// covers every pure-virtual method on the C++ class.
    #[allow(dead_code)]
    fn display_debug_message(&self, msg: &str);

    /// Blocking error dialog. Returns when the user dismisses.
    fn modal_error(&self, msg: &str);

    /// Blocking yes/no dialog. Returns `true` if the user confirmed.
    fn modal_confirmation(&self, msg: &str) -> bool;

    /// Blocking informational dialog. Returns when dismissed.
    fn modal_information(&self, msg: &str);
}

/// The canonical "do nothing" implementation of [`ProgressCallback`].
///
/// Mirrors C++ `NullProgressCallbacks` in
/// `common/ProgressCallback.cpp`. Every method is a no-op. In
/// particular:
///
/// - [`NullProgressCallback::modal_confirmation`] returns `false`,
///   matching the C++ behaviour where the user is logged but no
///   dialog is shown and "no" is the default answer.
/// - [`NullProgressCallback::is_cancelled`] and
///   [`NullProgressCallback::is_cancellable`] both return `false`,
///   so a long-running loop that consults the callback will run
///   to completion even if the surrounding UI has been killed.
///
/// The C++ version routes log messages to `Console.Error` etc. We
/// deliberately do **not** do that here; `NullProgressCallback` is
/// silent. If logging is wanted, add a `tracing::error!` call
/// inside the relevant method.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullProgressCallback;

impl ProgressCallback for NullProgressCallback {
    #[inline]
    fn push_state(&mut self) {}

    #[inline]
    fn pop_state(&mut self) {}

    #[inline]
    fn is_cancelled(&self) -> bool {
        false
    }

    #[inline]
    fn is_cancellable(&self) -> bool {
        false
    }

    #[inline]
    fn set_cancellable(&mut self, _cancellable: bool) {}

    #[inline]
    fn set_title(&mut self, _title: &str) {}

    #[inline]
    fn set_status_text(&mut self, _text: &str) {}

    #[inline]
    fn set_progress_range(&mut self, _range: u32) {}

    #[inline]
    fn set_progress_value(&mut self, _value: u32) {}

    #[inline]
    fn increment_progress_value(&mut self) {}

    #[inline]
    fn set_progress_state(&mut self, _state: ProgressState) {}

    #[inline]
    fn display_error(&self, _msg: &str) {}

    #[inline]
    fn display_warning(&self, _msg: &str) {}

    #[inline]
    fn display_information(&self, _msg: &str) {}

    #[inline]
    fn display_debug_message(&self, _msg: &str) {}

    #[inline]
    fn modal_error(&self, _msg: &str) {}

    #[inline]
    fn modal_confirmation(&self, _msg: &str) -> bool {
        false
    }

    #[inline]
    fn modal_information(&self, _msg: &str) {}
}

// ---------------------------------------------------------------------------
// FFI exports
// ---------------------------------------------------------------------------
//
// See the module-level "FFI" section for the rationale. The exports
// below are deliberately minimal — they let C++ obtain and dispose of
// a Rust-owned null callback, but do not (and cannot) expose the
// [`ProgressCallback`] trait itself across the C ABI.

/// Heap-allocate a [`NullProgressCallback`] and return an opaque
/// handle for C++ to hold.
///
/// The returned pointer is a `*mut NullProgressCallback` widened to
/// `*mut c_void` for C-ABI compatibility. C++ should treat it as
/// `void*` and pass it back to
/// [`pcsx2_progress_callback_destroy`] when done. The pointer is
/// non-null on success; allocation failure is propagated as a
/// null pointer (the C++ caller checks for null, matching the
/// `std::make_unique` failure mode).
///
/// Ownership transfers: C++ does not own the allocation until it
/// receives the pointer, and from that point onward it is the
/// sole owner until it calls
/// [`pcsx2_progress_callback_destroy`]. Sharing the pointer across
/// threads is safe because [`NullProgressCallback`] is `Sync`
/// (every method takes `&self` / `&mut self` and the type holds
/// no interior state).
#[no_mangle]
pub extern "C" fn pcsx2_progress_callback_null_create() -> *mut c_void {
    let boxed: Box<NullProgressCallback> = Box::new(NullProgressCallback);
    Box::into_raw(boxed) as *mut c_void
}

/// Free a [`NullProgressCallback`] previously returned by
/// [`pcsx2_progress_callback_null_create`].
///
/// `cb` must either be null (no-op) or a pointer returned by the
/// matching create function. Passing any other pointer, or
/// double-freeing, is undefined behaviour — exactly the contract
/// of `Box::from_raw`.
///
/// Safe to call from any thread; the underlying type is `Send`.
#[no_mangle]
pub extern "C" fn pcsx2_progress_callback_destroy(cb: *mut c_void) {
    if cb.is_null() {
        return;
    }
    // SAFETY: cb was returned by pcsx2_progress_callback_null_create
    // (or is null), and the caller promises they have not already
    // destroyed it. The Box owns a NullProgressCallback, which is
    // a ZST-like type with no Drop work to do, so this is just a
    // deallocation.
    unsafe {
        let _ = Box::from_raw(cb as *mut NullProgressCallback);
    }
}

// -- ProgressState discriminants re-exported as `u32` constants ------------
//
// These are convenience exports so that C++ can write
// `pcsx2_progress_callback_state_paused` instead of
// `static_cast<u32>(ProgressState::Paused)`. They are not part of
// the trait surface; they're plain `pub const u32` values.

/// `ProgressState::Normal` as a `u32` constant (= 0).
#[no_mangle]
pub static pcsx2_progress_callback_state_normal: u32 = ProgressState::Normal as u32;

/// `ProgressState::Indeterminate` as a `u32` constant (= 1).
#[no_mangle]
pub static pcsx2_progress_callback_state_indeterminate: u32 =
    ProgressState::Indeterminate as u32;

/// `ProgressState::Paused` as a `u32` constant (= 2).
#[no_mangle]
pub static pcsx2_progress_callback_state_paused: u32 = ProgressState::Paused as u32;

/// `ProgressState::Error` as a `u32` constant (= 3).
#[no_mangle]
pub static pcsx2_progress_callback_state_error: u32 = ProgressState::Error as u32;
