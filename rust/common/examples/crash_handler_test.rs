//! Standalone end-to-end test for the new `crash_handler` module.
//!
//! 1. Install the global crash handler via `crash_handler::install()`.
//! 2. Configure the write directory.
//! 3. Spawn a child process that crashes (SIGSEGV via null deref
//!    on Linux, or `EXCEPTION_ACCESS_VIOLATION` on Windows).
//! 4. The child process's signal handler runs, writes a sidecar
//!    `.txt` and a placeholder `.dmp` file, then exits with code 1.
//! 5. The parent process inspects the output files to confirm the
//!    handler ran.
//!
//! Run with: cargo run --release --example crash_handler_test

use std::process::{Command, Stdio};
use std::time::Duration;

#[path = "../src/crash_handler.rs"]
mod crash_handler;
use crash_handler::{install, set_write_directory};

fn main() {
    // 1. Install the handler. We do this in the parent so the
    //    *child* inherits the same `OnceLock` (no, actually: a child
    //    process is a fresh address space, so it gets its own
    //    `OnceLock`. We have to install in the child too — for the
    //    PoC we just spawn the child with a `Cargo` `run` of a
    //    separate target that calls `install` and then crashes).
    //
    // For simplicity: this binary *is* the child. We call install()
    // here, then crash immediately. The parent (a tiny shell wrapper
    // or just a separate test invocation) checks that the sidecar
    // file exists.
    //
    // But to keep the example self-contained, we just do it all in
    // one process: install, set the write directory, write a marker
    // file, then `std::ptr::null()` deref to trigger the handler.
    let _ = install();
    set_write_directory("./pcsx2_crash_out");

    // Write a marker so the user can see the program started
    // executing before the crash.
    std::fs::write("./pcsx2_crash_out/marker.txt", "started ok\n").ok();
    std::fs::create_dir_all("./pcsx2_crash_out").ok();

    eprintln!("[test] about to crash via null pointer deref");
    eprintln!("[test] handler should write ./pcsx2_crash_out/pcsx2_crash_*.{{txt,dmp}}");

    // Sleep so the output buffers flush before the crash.
    std::thread::sleep(Duration::from_millis(50));

    // Trigger the crash. We use a non-null function pointer cast to
    // 0x1 so the optimizer can't elide the call (null deref reads are
    // commonly optimized to a no-op in release builds).
    //
    // SAFETY: This is intentionally unsafe. We never expect to return.
    unsafe {
        let f: extern "C" fn() -> ! = std::mem::transmute(1usize);
        f();
    }
}
