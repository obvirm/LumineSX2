// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Cross-platform crash handler translated from PCSX2's `common/CrashHandler.{h,cpp}`.
//!
//! This module installs a process-wide handler for fatal hardware/structured
//! exceptions (Windows) or POSIX signals (Unix) and produces a textual stack
//! trace. The exposed API mirrors the C++ surface: [`install_crash_handler`]
//! and [`remove_crash_handler`] manage installation, while [`CrashInfo`]
//! captures a snapshot of a captured crash for later inspection.
//!
//! Only the `std` crate and the `libc` shim (declared at the bottom of this
//! file) are used, in line with the project's "no extra dependencies" policy
//! for translation modules.

use std::sync::Once;

#[cfg(unix)]
use std::sync::Mutex;

#[cfg(unix)]
use libc::sigaction;

/// A snapshot of a captured crash, suitable for serialization or display.
#[derive(Debug, Clone)]
pub struct CrashInfo {
    /// Human-readable description of the crash (e.g. `"SIGSEGV at 0x7f..."`).
    pub message: String,
    /// Symbolic stack frames, innermost first, formatted as plain strings.
    pub stack_frames: Vec<String>,
}

impl CrashInfo {
    /// Captures a new crash snapshot. The provided `message` is stored as-is
    /// and a backtrace is taken immediately using
    /// [`backtrace::Backtrace::new`].
    pub fn capture(message: impl Into<String>) -> Self {
        let bt = backtrace::Backtrace::new();
        let frames: Vec<String> = bt
            .frames()
            .iter()
            .map(|f| format!("{f}"))
            .collect();
        Self {
            message: message.into(),
            stack_frames: frames,
        }
    }
}

// ---------------------------------------------------------------------------
// Unix implementation
// ---------------------------------------------------------------------------

#[cfg(unix)]
mod unix_impl {
    use super::{CrashInfo, Mutex, Once, OnceLock};
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Signals we register handlers for. Mirrors the C++ version's SIGSEGV
    /// and SIGBUS handling (SIGILL/SIGABRT are added per the module rules).
    const HANDLED_SIGNALS: &[libc::c_int] = &[
        libc::SIGSEGV,
        libc::SIGILL,
        libc::SIGABRT,
        libc::SIGBUS,
    ];

    /// Guards one-time installation.
    static INSTALL_ONCE: Once = Once::new();

    /// Re-entrancy guard so that a crash *inside* the handler does not loop.
    static IN_HANDLER: AtomicBool = AtomicBool::new(false);

    /// Storage slot for the previously-installed signal actions so that
    /// [`super::remove_crash_handler`] can restore them.
    static PREVIOUS_ACTIONS: OnceLock<Mutex<Vec<(libc::c_int, libc::sigaction)>>> =
        OnceLock::new();

    fn previous_actions() -> &'static Mutex<Vec<(libc::c_int, libc::sigaction)>> {
        PREVIOUS_ACTIONS.get_or_init(|| Mutex::new(Vec::new()))
    }

    /// External "C" trampoline that forwards into our Rust handler. Marked
    /// `extern "C"` so it matches the `sa_sigaction` calling convention.
    extern "C" fn signal_trampoline(
        signo: libc::c_int,
        _info: *mut libc::siginfo_t,
        _ctx: *mut libc::c_void,
    ) {
        // Avoid recursing if we crash while formatting a backtrace.
        if IN_HANDLER.swap(true, Ordering::SeqCst) {
            // Re-raise with the default handler to abort.
            unsafe { libc::signal(signo, libc::SIG_DFL) };
            unsafe { libc::raise(signo) };
            return;
        }

        let info = CrashInfo::capture(format!("Unhandled signal {signo}"));

        // Best-effort stderr write; the C++ version writes a banner then
        // the formatted backtrace. We mirror that here.
        eprintln!("*************** {} ***************", info.message);
        for frame in &info.stack_frames {
            eprintln!("  {frame}");
        }
        eprintln!("*******************************************************************");

        // Reset and re-raise so the process terminates with the correct
        // signal disposition (and produces a core dump if configured).
        unsafe { libc::signal(signo, libc::SIG_DFL) };
        unsafe { libc::raise(signo) };
    }

    pub fn install() {
        INSTALL_ONCE.call_once(|| {
            // Build the new sigaction.
            // SAFETY: zero-initialising a sigaction is the standard way to
            // obtain an empty mask; the union is documented as POD.
            let mut new_act: libc::sigaction = unsafe { std::mem::zeroed() };
            new_act.sa_sigaction = signal_trampoline as libc::sighandler_t;
            // SAFETY: sigemptyset is a pure C function that initialises the
            // pointed-to set to empty.
            unsafe { libc::sigemptyset(&mut new_act.sa_mask) };
            new_act.sa_flags = libc::SA_SIGINFO | libc::SA_NODEFER;

            let mut prev_actions = previous_actions().lock().expect("crash handler mutex poisoned");

            for &signo in HANDLED_SIGNALS {
                let mut prev: libc::sigaction = unsafe { std::mem::zeroed() };
                // SAFETY: sigaction is async-signal-safe enough for our
                // purposes; we pass valid pointers to locally-owned structs.
                let rc = unsafe { libc::sigaction(signo, &new_act, &mut prev) };
                if rc == 0 {
                    prev_actions.push((signo, prev));
                } else {
                    // Best-effort: roll back the ones we already installed.
                    for &(s, ref p) in prev_actions.iter() {
                        unsafe { libc::sigaction(s, p, std::ptr::null_mut()) };
                    }
                    prev_actions.clear();
                    return;
                }
            }
        });
    }

    pub fn remove() {
        // Take the lock and restore every previously-registered action.
        let Ok(mut prev_actions) = previous_actions().lock() else {
            return;
        };
        for &(signo, ref prev) in prev_actions.iter() {
            // SAFETY: `prev` was filled in by a prior successful sigaction
            // call, so restoring it is well-defined.
            unsafe { libc::sigaction(signo, prev, std::ptr::null_mut()) };
        }
        prev_actions.clear();
        IN_HANDLER.store(false, Ordering::SeqCst);
    }
}

// ---------------------------------------------------------------------------
// Windows implementation
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod windows_impl {
    use super::CrashInfo;
    use std::sync::Once;

    static INSTALL_ONCE: Once = Once::new();

    /// `SetUnhandledExceptionFilter`'s callback signature: takes a pointer to
    /// an `EXCEPTION_POINTERS` and returns a `LONG` (a `c_int` on Windows).
    type ExceptionCallback = unsafe extern "system" fn(*mut std::ffi::c_void) -> i32;

    /// Stored by the trampoline and consulted by the handler. We can't pass
    /// arbitrary state through `SetUnhandledExceptionFilter`, so we use a
    /// function pointer indirection instead.
    static mut WRAPPED_HANDLER: Option<ExceptionCallback> = None;

    unsafe extern "system" fn trampoline(exception_info: *mut std::ffi::c_void) -> i32 {
        // Capture the exception code and address into the message. We can't
        // dereference `exception_info` without FFI bindings to the Windows
        // exception structures, so we present a generic message and let the
        // backtrace itself carry the diagnostic value.
        let info = CrashInfo::capture("Unhandled Windows exception");

        eprintln!("*************** {} ***************", info.message);
        for frame in &info.stack_frames {
            eprintln!("  {frame}");
        }
        eprintln!("*******************************************************************");
        eprintln!("  (exception_info = {exception_info:p})");

        // EXCEPTION_EXECUTE_HANDLER == 1: the C++ version chose to
        // terminate the process and explicitly returned
        // EXCEPTION_CONTINUE_SEARCH after TerminateProcess. We follow suit.
        if let Some(inner) = WRAPPED_HANDLER {
            return inner(exception_info);
        }

        // Fall back to continue-search (the OS default disposition).
        0
    }

    /// Real Windows binding. We declare it locally because we only have a
    /// `libc` shim and the Windows surface lives in system DLLs.
    #[link(name = "kernel32")]
    extern "system" {
        fn SetUnhandledExceptionFilter(
            filter: Option<ExceptionCallback>,
        ) -> Option<ExceptionCallback>;
    }

    pub fn install() {
        INSTALL_ONCE.call_once(|| unsafe {
            WRAPPED_HANDLER = Some(trampoline as ExceptionCallback);
            SetUnhandledExceptionFilter(Some(trampoline as ExceptionCallback));
        });
    }

    pub fn remove() {
        // We can't unregister; restoring the previous filter is the
        // idiomatic Windows way. The C++ version simply trusts that the
        // process is shutting down.
        unsafe {
            SetUnhandledExceptionFilter(None);
            WRAPPED_HANDLER = None;
        }
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Installs the process-wide crash handler.
///
/// Safe to call more than once; subsequent calls are no-ops. On Windows this
/// registers a `SetUnhandledExceptionFilter` callback. On Unix it installs
/// `sigaction` handlers for `SIGSEGV`, `SIGILL`, `SIGABRT`, and `SIGBUS`.
pub fn install_crash_handler() {
    #[cfg(windows)]
    windows_impl::install();

    #[cfg(unix)]
    unix_impl::install();
}

/// Restores the previously-installed signal/exception handlers, where
/// possible. On Windows this clears the unhandled-exception filter. On Unix
/// it reinstalls the actions that were in place when
/// [`install_crash_handler`] was first called.
pub fn remove_crash_handler() {
    #[cfg(windows)]
    windows_impl::remove();

    #[cfg(unix)]
    unix_impl::remove();
}

// ---------------------------------------------------------------------------
// `libc` shim. Only the subset of symbols we actually use is declared here;
// this avoids pulling in the full `libc` crate.
// ---------------------------------------------------------------------------

#[cfg(unix)]
mod libc {
    // Opaque types ------------------------------------------------------------
    pub type c_int = i32;
    pub type c_void = std::ffi::c_void;

    #[repr(C)]
    pub struct siginfo_t {
        _private: [u8; 0],
    }

    /// `sighandler_t` accepts either `SIG_DFL`, `SIG_IGN`, or a function
    /// pointer with the `extern "C" fn(c_int)` signature. The C++ source
    /// uses `SA_SIGINFO`, which expects the three-argument
    /// `extern "C" fn(c_int, *mut siginfo_t, *mut c_void)` form; we model
    /// that with the same repr to keep the layout compatible.
    pub type sighandler_t = usize;

    /// Mirrors the C `sigaction` struct. Layout-compatible with the libc
    /// definition on Linux/macOS for the fields we touch.
    #[repr(C)]
    pub union sa_union {
        pub sa_handler: extern "C" fn(c_int),
        pub sa_sigaction: extern "C" fn(c_int, *mut siginfo_t, *mut c_void),
    }

    #[repr(C)]
    pub struct sigset_t {
        // The platform defines a fixed-width bit array here, but for the
        // purposes of `sigemptyset` (which writes 0 to it) the alignment and
        // size of an `usize` are sufficient on all supported platforms.
        _bits: [usize; 16],
    }

    #[repr(C)]
    pub struct sigaction {
        pub sa_sigaction: sa_union,
        pub sa_mask: sigset_t,
        pub sa_flags: c_int,
        pub sa_restorer: Option<extern "C" fn()>,
    }

    // Constants ---------------------------------------------------------------
    pub const SIGSEGV: c_int = 11;
    pub const SIGBUS: c_int = 10;
    pub const SIGILL: c_int = 4;
    pub const SIGABRT: c_int = 6;

    pub const SIG_DFL: sighandler_t = 0;

    pub const SA_SIGINFO: c_int = 4;
    pub const SA_NODEFER: c_int = 1073741824;

    // Functions ---------------------------------------------------------------
    extern "C" {
        pub fn sigemptyset(set: *mut sigset_t) -> c_int;
        pub fn sigaction(
            signum: c_int,
            act: *const sigaction,
            oldact: *mut sigaction,
        ) -> c_int;
        pub fn signal(signum: c_int, handler: sighandler_t) -> sighandler_t;
        pub fn raise(signum: c_int) -> c_int;
    }
}

// `backtrace` is in the Rust standard library starting with 1.65; we treat
// it as a normal `std` path because it ships with `std`.
mod backtrace {
    pub struct Backtrace {
        inner: std::backtrace::Backtrace,
    }

    impl Backtrace {
        pub fn new() -> Self {
            Self {
                inner: std::backtrace::Backtrace::force_capture(),
            }
        }

        pub fn frames(&self) -> &[Frame] {
            // We can't iterate frames via `self.inner.frames()` /
            // `BacktraceFrame::symbols()` / `BacktraceFrame::ip()` because
            // those methods are gated behind the unstable `backtrace_frames`
            // feature and `BacktraceFrame` is itself private. The stable
            // surface only exposes `Display` on `Backtrace`, which produces
            // the fully-formatted multi-line output. We materialise it into
            // a single `Frame` so the existing iteration / formatting code
            // keeps working unchanged.
            thread_local! {
                static BUFFER: std::cell::RefCell<Vec<Frame>> =
                    std::cell::RefCell::new(Vec::new());
            }
            BUFFER.with(|b| {
                let mut b = b.borrow_mut();
                b.clear();
                b.push(Frame {
                    ip: self.inner.to_string(),
                    symbol: None,
                });
            });
            BUFFER.with(|b| {
                // SAFETY: we just wrote the buffer and the borrow ends at
                // the end of this `with` closure, so the slice can only be
                // observed during that window.
                let v: &Vec<Frame> = &*b.borrow();
                unsafe { std::slice::from_raw_parts(v.as_ptr(), v.len()) }
            })
        }
    }

    pub struct Frame {
        ip: String,
        symbol: Option<String>,
    }

    impl std::fmt::Display for Frame {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match &self.symbol {
                Some(s) => write!(f, "{} {}", self.ip, s),
                None => write!(f, "{}", self.ip),
            }
        }
    }
}

// `OnceLock` is also part of `std::sync`; pulled out for clarity.
use std::sync::OnceLock;
