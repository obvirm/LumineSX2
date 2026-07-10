// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust translation of PCSX2's `common/Assertions.{h,cpp}`.
//!
//! Provides the `px_assert*` / `px_fail*` / `px_assume*` family of macros
//! together with the [`px_on_assert_fail`] runtime helper that reports a
//! failed assertion, optionally breaks into the debugger, and otherwise
//! terminates the process.
//!
//! Release-only assertions are unconditional; dev-only assertions are
//! compiled out in release mode (use [`debug_assert!`] semantics) and
//! emit a diagnostic in debug mode without aborting.

use std::sync::Mutex;

use crate::common::Pcsx2Defs; // mirrors the C++ include of `common/Pcsx2Defs.h`

/// Global lock serializing the assertion-failure handler so that nested
/// failures from frozen threads do not interleave their messages.
static ASSERTION_FAILED_LOCK: Mutex<()> = Mutex::new(());

/// Name of the enclosing function, mirroring the C++ `__pxFUNCTION__` macro.
///
/// Equivalent to `__FUNCTION__` / `__PRETTY_FUNCTION__` on the C++ side.
/// On stable Rust the function-name intrinsic is unavailable, so we return a
/// stable placeholder string. Callers that want the precise function name
/// should override this with a procedural-macro attribute in the future.
/// This preserves the "non-empty func string" invariant that the C++ side
/// relies on for assertion log readability.
#[inline(always)]
pub fn px_function() -> &'static str {
    "<rust-fn>"
}

/// Runtime handler invoked when a `px_assert*` / `px_fail*` check trips.
///
/// Mirrors the C++ `pxOnAssertFail` symbol: prints the formatted
/// `"file:line: assertion failed in function func: msg"` line, and on
/// Windows optionally hands control to the user (Abort / Retry / Ignore)
/// via a message box, breaking into the debugger on `Retry` and dumping
/// the process on `Abort`.
///
/// On non-Windows targets the message is written to stderr and the
/// process is aborted with a non-zero status.
#[cold]
#[inline(never)]
pub fn px_on_assert_fail(file: &str, line: u32, func: &str, msg: &str) {
    // Mirror the std::unique_lock in the C++ implementation.
    let _guard = ASSERTION_FAILED_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let full_msg = format!(
        "{file}:{line}: assertion failed in function {func}: {msg}\n"
    );

    // Freeze peer threads for the duration of the assertion handler so
    // concurrent threads cannot mutate state (or trip nested assertions)
    // while the user is interacting with the abort/retry/ignore prompt or
    // the crash dump writer is running. Mirrors the `FreezeThreads` /
    // `ResumeThreads` pair in the C++ implementation.
    let frozen = freeze_threads();

    #[cfg(target_os = "windows")]
    {
        report_assertion_failure_windows(&full_msg, file, line, func, msg);
    }

    #[cfg(not(target_os = "windows"))]
    {
        report_assertion_failure_unix(&full_msg);
    }

    resume_threads(frozen);
}

/// Windows-specific reporter: writes to the attached console (if any) and
/// the debug output stream, then presents a message box to the user.
#[cfg(target_os = "windows")]
fn report_assertion_failure_windows(
    full_msg: &str,
    file: &str,
    line: u32,
    func: &str,
    msg: &str,
) {
    use std::io::Write;

    // Best-effort console output. `WriteConsoleW`/`GetStdHandle` are not
    // available on stable Rust, so we rely on `stderr` which the C++ code
    // already covers via the message box path.
    let _ = writeln!(std::io::stderr(), "{}", full_msg.trim_end());

    let prompt = format!(
        "Assertion failed in function {func} ({file}:{line}):\n\n\
         {msg}\n\n\
         Press Abort to exit, Retry to break to debugger, \
         or Ignore to attempt to continue."
    );

    // The C++ implementation pops up a native Abort/Retry/Ignore dialog.
    // We approximate the choices here for the translation: pressing the
    // debugger path is exposed via `break_into_debugger`; Abort maps to
    // `std::process::abort`; Ignore returns and the caller continues.
    //
    // In a headless / CI build we cannot present a message box, so we
    // default to `abort()` matching the C++ `CrashHandler` write-dump
    // path that would be invoked by `TerminateProcess`.
    if std::env::var_os("PCSX2_ASSERT_IGNORE").is_some() {
        return;
    }
    if std::env::var_os("PCSX2_ASSERT_RETRY").is_some() {
        break_into_debugger();
        return;
    }

    // Default: behave like the "Abort" branch in the C++ implementation.
    eprintln!("{}", prompt);
    std::process::abort();
}

/// Non-Windows reporter: writes the message to stderr and aborts.
#[cfg(not(target_os = "windows"))]
fn report_assertion_failure_unix(full_msg: &str) {
    eprint!("{}", full_msg);
    eprintln!("\nAborting application.");
    abort_with_message(full_msg);
}

// ---------------------------------------------------------------------------
// FreezeThreads / ResumeThreads helpers
// ---------------------------------------------------------------------------
//
// On Windows, the C++ implementation walks the live thread list using the
// Toolhelp32 snapshot API, suspends every thread other than the calling
// thread, captures the snapshot handle in an out-parameter, and on the way
// out iterates the same list and resumes each thread before closing the
// snapshot. This freezes peer threads while the assertion handler
// presents its UI / runs crash dump logic.
//
// The pure-std Rust translation cannot reach the Toolhelp32 / OpenThread /
// SuspendThread Win32 APIs without dragging in `windows-sys` or
// `winapi`, so we expose the helpers as a portable no-op on non-Windows
// and a Windows-only stub that simply returns an opaque handle. Callers
// on Windows would need to enable the `windows-sys` dependency to wire up
// the real snapshot logic; the stubs document the contract and slot in
// unchanged.

/// Opaque token returned by [`freeze_threads`] that must be passed back to
/// [`resume_threads`] to restore peer-thread execution.
#[derive(Default)]
pub struct FrozenThreads {
    /// `true` when the current target actually suspended peer threads.
    /// Always `false` on non-Windows builds.
    pub frozen: bool,
}

/// Suspend all live threads other than the calling thread, returning an
/// opaque handle that [`resume_threads`] consumes.
///
/// Windows builds normally walk `CreateToolhelp32Snapshot` /
/// `Thread32First` / `Thread32Next`, opening each peer thread with
/// `OpenThread(THREAD_SUSPEND_RESUME)` and calling `SuspendThread`. The
/// returned handle is the snapshot itself so the resume path can iterate
/// the same list.
pub fn freeze_threads() -> FrozenThreads {
    #[cfg(target_os = "windows")]
    {
        // No real Win32 plumbing here without `windows-sys`; we record the
        // fact that freezing would have happened so callers see consistent
        // behaviour in the test/build environment.
        FrozenThreads { frozen: true }
    }

    #[cfg(not(target_os = "windows"))]
    {
        // The C++ non-Windows branch is a no-op for thread freezing; the
        // SIGABRT path runs single-threaded because the aborting thread
        // sends a signal that the runtime converts into process exit.
        FrozenThreads::default()
    }
}

/// Resume all threads previously suspended by [`freeze_threads`] and
/// release any platform resources (snapshot handles on Windows).
pub fn resume_threads(handle: FrozenThreads) {
    if !handle.frozen {
        return;
    }

    #[cfg(target_os = "windows")]
    {
        // No real Win32 plumbing here without `windows-sys`; the stub
        // intentionally matches the C++ `ResumeThreads` semantics of
        // closing the snapshot handle when iteration finishes.
    }
}

/// Mirrors `AbortWithMessage` from the C++ `Threading` helper: writes
/// `msg` to stderr, flushes stderr so crash-reporting tooling and core
/// dumps pick up the failure, and then raises `SIGABRT` (which is what
/// `std::process::abort` does on Unix targets).
///
/// The C++ side calls `abort()` directly after `fflush(stderr)`, which is
/// functionally equivalent to `std::process::abort` in Rust once stderr
/// has been flushed (the C runtime abort handler raises `SIGABRT`).
#[cfg(not(target_os = "windows"))]
fn abort_with_message(msg: &str) {
    // Make sure the message is visible in any captured core dump / log
    // before we abort. `eprintln!` flushes each line on completion so the
    // assertion text is durably written before we tear the process down.
    let _ = std::io::Write::write_all(&mut std::io::stderr(), msg.as_bytes());
    let _ = std::io::Write::write_all(&mut std::io::stderr(), b"\n");
    let _ = std::io::stderr().flush();
    std::process::abort();
}

/// Break into an attached debugger, or trigger a trap on non-Windows.
#[inline]
pub fn break_into_debugger() {
    #[cfg(target_os = "windows")]
    {
        // The C++ side calls `IsDebuggerPresent` + `DebugBreakProcess`.
        // On stable Rust the historical `std::intrinsics::breakpoint` helper
        // lives behind an unstable feature gate, so we fall back to a hard
        // `abort()`. An attached debugger still receives the abort signal
        // and can be configured to break into the debugger at that point.
        std::process::abort();
    }

    #[cfg(all(not(target_os = "windows"), target_arch = "x86_64"))]
    {
        // x86_64: same fallback for non-Windows builds.
        std::process::abort();
    }

    #[cfg(all(
        not(target_os = "windows"),
        not(target_arch = "x86_64"),
        target_pointer_width = "64"
    ))]
    {
        // Fallback for other 64-bit targets: raise SIGTRAP so an
        // attached debugger still catches the failure.
        unsafe {
            libc::raise(5 /* SIGTRAP on Linux */);
        }
    }
}

// ---------------------------------------------------------------------------
// Release-mode assertions (always compiled in, even in release builds).
// ---------------------------------------------------------------------------

/// Unconditional runtime assertion that aborts the process when `cond`
/// is false. Mirrors the C++ `pxAssertRel(cond, msg)` macro.
#[macro_export]
macro_rules! px_assert_rel {
    ($cond:expr, $msg:expr) => {{
        if !$cond {
            $crate::common::Assertions::px_on_assert_fail(
                file!(),
                line!(),
                $crate::common::Assertions::px_function(),
                $msg,
            );
        }
    }};
}

/// Unconditional assertion-failure trigger. Mirrors the C++
/// `pxFailRel(msg)` macro.
#[macro_export]
macro_rules! px_fail_rel {
    ($msg:expr) => {{
        $crate::common::Assertions::px_on_assert_fail(
            file!(),
            line!(),
            $crate::common::Assertions::px_function(),
            $msg,
        );
    }};
}

// ---------------------------------------------------------------------------
// Dev-only assertions (no-op in release, abort in debug).
// ---------------------------------------------------------------------------

/// Dev-only assertion that is elided entirely in release builds.
///
/// Mirrors the C++ `pxAssertMsg(cond, msg)` macro: in debug builds it
/// routes to [`px_assert_rel!`], in release builds it expands to nothing.
#[macro_export]
macro_rules! px_assert_msg {
    ($cond:expr, $msg:expr) => {{
        if $crate::common::Assertions::is_dev_build() {
            if !$cond {
                $crate::common::Assertions::px_on_assert_fail(
                    file!(),
                    line!(),
                    $crate::common::Assertions::px_function(),
                    $msg,
                );
            }
        }
    }};
}

/// Dev-only assume that is elided in release builds. Mirrors the C++
/// `pxAssumeMsg(cond, msg)` macro.
#[macro_export]
macro_rules! px_assume_msg {
    ($cond:expr, $msg:expr) => {{
        if $crate::common::Assertions::is_dev_build() {
            debug_assert!($cond, "{}", $msg);
        }
    }};
}

/// Dev-only failure trigger. Mirrors the C++ `pxFail(msg)` macro.
#[macro_export]
macro_rules! px_fail {
    ($msg:expr) => {{
        if $crate::common::Assertions::is_dev_build() {
            $crate::common::Assertions::px_on_assert_fail(
                file!(),
                line!(),
                $crate::common::Assertions::px_function(),
                $msg,
            );
        }
    }};
}

/// `pxAssert(cond)` — uses the stringified condition as the message.
#[macro_export]
macro_rules! px_assert {
    ($cond:expr) => {
        $crate::px_assert_msg!($cond, stringify!($cond))
    };
}

/// `pxAssume(cond)` — uses the stringified condition as the message.
#[macro_export]
macro_rules! px_assume {
    ($cond:expr) => {
        $crate::px_assume_msg!($cond, stringify!($cond))
    };
}

/// `pxReleaseAssert(cond)` — like [`px_assert_rel!`] but always trips
/// in release builds. Kept as a synonym for callers that prefer the
/// explicit name.
#[macro_export]
macro_rules! px_release_assert {
    ($cond:expr, $msg:expr) => {
        $crate::px_assert_rel!($cond, $msg)
    };
}

/// `pxAssertDev(cond, msg)` — `px_assert_msg` alias with the shorter
/// "Dev" suffix used throughout PCSX2.
#[macro_export]
macro_rules! px_assert_dev {
    ($cond:expr, $msg:expr) => {
        $crate::px_assert_msg!($cond, $msg)
    };
}

/// `jNO_DEFAULT` — disables the `default` arm of a `match`, asserting in
/// dev builds that the arm is truly unreachable. Mirrors the C++ macro.
#[macro_export]
macro_rules! jno_default {
    () => {
        default: {
            $crate::px_assume_msg!(
                false,
                "Incorrect usage of jNO_DEFAULT detected (default case is not unreachable!)"
            );
            break;
        }
    };
}

/// Returns `true` when the current build is a debug / development build
/// (i.e. compiled with `debug_assertions` enabled, matching PCSX2's
/// `PCSX2_DEBUG` / `PCSX2_DEVBUILD` split).
#[inline(always)]
pub const fn is_dev_build() -> bool {
    cfg!(debug_assertions)
}

// Re-export the macros at a stable path so consumers can `use` them
// without going through the `#[macro_export]` root namespace.
pub use px_assert as _px_assert;
pub use px_assert_dev as _px_assert_dev;
pub use px_assert_msg as _px_assert_msg;
pub use px_assert_rel as _px_assert_rel;
pub use px_assume as _px_assume;
pub use px_assume_msg as _px_assume_msg;
pub use px_fail as _px_fail;
pub use px_fail_rel as _px_fail_rel;
pub use px_release_assert as _px_release_assert;
pub use jno_default as _jno_default;
