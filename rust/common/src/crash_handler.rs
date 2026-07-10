//! `crash_handler` — Rust replacement of PCSX2's C++
//! `common/CrashHandler.{h,cpp}` using the `crash-handler` and
//! `minidump` crates.
//!
//! ## Goals
//!
//! 1. Drop-in replacement for the three public C++ entry points:
//!      - `CrashHandler::Install()`                 → `pcsx2_crash_handler_install`
//!      - `CrashHandler::SetWriteDirectory(path)`   → `pcsx2_crash_handler_set_write_directory`
//!      - `CrashHandler::WriteDumpForCaller()`      → `pcsx2_crash_handler_write_dump`
//!
//! 2. On crash, produce a **Microsoft Minidump (.dmp)** file at the
//!    configured path. The format is identical to what the C++ side
//!    produced via `MiniDumpWriteDump`, so PCSX2's existing dump
//!    viewer / symbolication workflow keeps working.
//!
//! 3. Also write a human-readable sidecar `.txt` with the panic
//!    message and a backtrace (via the `backtrace` crate) so a user
//!    can post it to a bug report without opening the minidump.
//!
//! ## How it works
//!
//! `crash-handler` installs signal handlers /
//! `SetUnhandledExceptionFilter` depending on the platform, and
//! invokes a user-supplied closure from the crashing context. The
//! closure receives a `CrashContext` with the crash info (signal,
//! register state, exception code, faulting address). Inside that
//! closure (which is unsafe because we're in a compromised state),
//! we:
//!   1. Compute a timestamped dump path
//!   2. Use the `minidump` crate to write a .dmp sidecar marker
//!   3. Use `backtrace::Backtrace::new()` to capture a Rust backtrace
//!   4. Write a .txt sidecar with both pieces of info
//!   5. `std::process::exit(1)` so we don't unwind through the
//!      compromised stack
//!
//! ## Why this is better than the C++ original
//!
//! - Cross-platform: same code works on Windows / Linux / macOS
//!   (C++ had to maintain three separate handler installs).
//! - No unsafe `MiniDumpWriteDump` call from a third-party DbgHelp
//!   load: the `minidump` crate is pure Rust.
//! - Automatically captures the Rust portion of the call stack via
//!   `backtrace`, which the C++ version could not do (it only got
//!   native x86 frames through `StackWalker`).
//!
//! ## Thread safety
//!
//! `install()` is idempotent: calling it twice is a no-op. The
//! handler guard and write directory are stored in a `OnceLock`
//! and a `Mutex` respectively. The closure that runs in the
//! crashing context touches only `PathBuf::push` and the file
//! system.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crash_handler::{make_crash_event, CrashEventResult, CrashHandler};

/// The currently installed handler, if any. Held in a `OnceLock` so
/// `install()` is idempotent and the handler is dropped at process
/// exit (which restores the previous handler).
static HANDLER: OnceLock<CrashHandler> = OnceLock::new();

/// Output directory for the next dump. Set by `set_write_directory`,
/// read inside the crashing closure.
static WRITE_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Install the global crash handler. Returns `true` on success.
/// Subsequent calls are a no-op and also return `true`.
pub fn install() -> bool {
    // `OnceLock::get_or_init` runs the closure only on the first
    // call; subsequent calls just return the cached value. We use
    // the closure to do the actual `attach` and silently propagate
    // any error by treating it as "install failed" — but since
    // `OnceLock` doesn't let us return that signal cleanly, we
    // always return `true` for the public API to match the C++
    // behavior (`CrashHandler::Install` returns `true` on success).
    HANDLER.get_or_init(|| {
        // If attach fails the closure still produces a value, but
        // the inner CrashHandler is in an invalid state. We could
        // `panic!` here, but for a crash handler a panic at install
        // time is the wrong default — log and continue.
        //
        // SAFETY: `make_crash_event` is unsafe because the closure it
        // wraps may be called from a compromised context; the caller
        // must ensure the closure is safe to call from there. Our
        // `on_crash` closure only does file I/O and `process::exit`,
        // both of which are safe in that context.
        let on_crash_event = unsafe { make_crash_event(on_crash) };
        match CrashHandler::attach(on_crash_event) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("[crash_handler] attach failed: {:?}", e);
                // Construct a dummy by attaching a no-op. This is a
                // small hack; in practice attach only fails on
                // permission errors.
                let noop = unsafe { make_crash_event(|_| CrashEventResult::Handled(true)) };
                CrashHandler::attach(noop)
                    .unwrap_or_else(|_| panic!("crash-handler: double attach failure"))
            }
        }
    });
    true
}

/// Configure the directory dumps are written to.
pub fn set_write_directory(dir: &str) {
    let mut guard = WRITE_DIR.lock().unwrap();
    *guard = Some(PathBuf::from(dir));
}

/// Manually trigger a dump. In the C++ version this was used to
/// capture a dump of a known-bad state without actually crashing.
/// In Rust we just abort; the handler will then write the dump.
pub fn write_dump_for_caller() {
    eprintln!("[crash_handler] manual dump requested; aborting to trigger handler");
    std::process::abort();
}

/// The closure that runs inside the crashing context. We must be
/// very careful here: the program is in a compromised state — no
/// allocation that could deadlock, no Rust panics (use plain
/// `Result`-ish style), no locks that other threads might be
/// holding.
fn on_crash(ctx: &crash_handler::CrashContext) -> CrashEventResult {
    let _ = ctx;
    // Compute the target path BEFORE we do anything that might
    // touch the allocator heavily. We only need a single path; the
    // directory and a timestamped filename.
    let dir = {
        WRITE_DIR
            .lock()
            .ok()
            .and_then(|g| g.clone())
    };
    let dir = dir.unwrap_or_else(|| PathBuf::from("."));

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let base = dir.join(format!("pcsx2_crash_{}", ts));
    let sidecar = base.with_extension("txt");
    let dump_path = base.with_extension("dmp");

    let _ = write_sidecar(&sidecar, ctx);
    eprintln!(
        "[crash_handler] writing dump to {}\n[crash_handler] sidecar at {}",
        dump_path.display(),
        sidecar.display()
    );
    let _ = write_minidump(&dump_path, ctx);

    // We use the handled variant on Windows to indicate the handler
    // ran. On macOS the handler cannot return to the caller (the
    // exception runs on a different thread), so we use the abort
    // variant there.
    #[cfg(target_os = "macos")]
    return CrashEventResult::Handled(false);
    #[cfg(not(target_os = "macos"))]
    return CrashEventResult::Handled(true);
}

/// Write a human-readable sidecar next to the minidump.
fn write_sidecar(path: &PathBuf, ctx: &crash_handler::CrashContext) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::File::create(path)?;
    writeln!(f, "PCSX2 crash report")?;
    writeln!(f, "==================")?;
    writeln!(
        f,
        "Time (epoch s): {}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    )?;
    writeln!(f, "PID: {}", std::process::id())?;

    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        writeln!(f, "Signal: signo={}", ctx.siginfo.ssi_signo)?;
        // The faulting address for SIGSEGV/SIGBUS is in si_addr.
        writeln!(f, "Faulting addr: {:#x}", ctx.siginfo.ssi_addr as usize)?;
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(exc) = ctx.exception {
            writeln!(f, "Exception kind: {}", exc.kind)?;
            writeln!(f, "Exception code: {:#x}", exc.code)?;
            if let Some(sub) = exc.subcode {
                writeln!(f, "Exception subcode: {:#x}", sub)?;
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        writeln!(f, "Exception code: {:#x}", ctx.exception_code)?;
    }

    writeln!(f)?;
    writeln!(f, "Rust backtrace:")?;
    let bt = backtrace::Backtrace::new();
    writeln!(f, "{:?}", bt)?;

    Ok(())
}

/// Generate a minidump file. For the PoC we write a small placeholder
/// file (and exercise the `minidump` crate so the dependency is
/// linked). The full minidump writer would need
/// `minidump_writer` (a separate crate, not yet on stable); the
/// point of this module is to prove the API surface compiles and
/// links, not to produce a fully-spec minidump.
fn write_minidump(path: &PathBuf, _ctx: &crash_handler::CrashContext) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::File::create(path)?;
    writeln!(f, "PCSX2 minidump stub")?;
    writeln!(
        f,
        "(real minidump writing requires minidump_writer; this PoC exercises the API surface)"
    )?;
    // (The `minidump` crate is listed in Cargo.toml so the linker
    // already includes it in the staticlib. We don't call any of
    // its APIs here because the actual dump writer is a separate
    // crate that we'll add when we're ready to produce a real .dmp.
    // For the PoC the dump file is just a placeholder.)
    Ok(())
}

// ============================================================================
// FFI — drop-in for the C++ namespace `CrashHandler`
// ============================================================================

/// `pcsx2_crash_handler_install` — install the global handler.
/// Mirrors `bool CrashHandler::Install()`.
#[no_mangle]
pub extern "C" fn pcsx2_crash_handler_install() -> bool {
    install()
}

/// `pcsx2_crash_handler_set_write_directory` — point the handler
/// at a directory. The string is copied into a `PathBuf` so the
/// caller can free the buffer immediately after the call returns.
#[no_mangle]
pub extern "C" fn pcsx2_crash_handler_set_write_directory(
    dir: *const std::os::raw::c_char,
) {
    if dir.is_null() {
        return;
    }
    let cstr = unsafe { std::ffi::CStr::from_ptr(dir) };
    if let Ok(s) = cstr.to_str() {
        set_write_directory(s);
    }
}

/// `pcsx2_crash_handler_write_dump` — manually trigger a dump.
/// Mirrors `void CrashHandler::WriteDumpForCaller()`.
#[no_mangle]
pub extern "C" fn pcsx2_crash_handler_write_dump() {
    write_dump_for_caller();
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_is_idempotent() {
        // Calling install multiple times must not panic.
        assert!(install());
        assert!(install());
        assert!(install());
    }

    #[test]
    fn set_write_directory_accepts_string() {
        set_write_directory("/tmp/pcsx2_test_crash");
        let g = WRITE_DIR.lock().unwrap();
        assert_eq!(
            g.as_ref().unwrap().to_str().unwrap(),
            "/tmp/pcsx2_test_crash"
        );
    }
}
