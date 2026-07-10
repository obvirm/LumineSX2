// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Linux host-system facilities.
//!
//! Idiomatic Rust 2021 translation of PCSX2's `LnxHostSys.cpp`, `LnxMisc.cpp`
//! and `LnxThreads.cpp`. The entire body is gated on `target_os = "linux"`;
//! non-Linux targets get empty stubs so consumers can call the API uniformly.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

// ---------------------------------------------------------------------------
// Linux implementation
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
mod imp {
    use std::env;
    use std::ffi::CString;
    use std::fs;
    use std::io::{BufRead, BufReader};
    use std::os::raw::{c_char, c_int, c_uint};
    use std::path::PathBuf;
    use std::sync::OnceLock;
    use std::thread;
    use std::time::Duration;

    // -----------------------------------------------------------------------
    // libc bindings we need. Kept minimal so the module is self-contained.
    // -----------------------------------------------------------------------

    const PR_SET_NAME: c_int = 15;
    const PR_GET_NAME: c_int = 16;
    const PATH_MAX: usize = 4096;

    extern "C" {
        fn prctl(option: c_int, ...) -> c_int;
        fn getpid() -> c_int;
        fn usleep(usec: u32) -> c_int;
        fn sched_yield() -> c_int;
    }

    // -----------------------------------------------------------------------
    // Path helpers (XDG-based)
    // -----------------------------------------------------------------------

    fn read_env(var: &str) -> Option<PathBuf> {
        env::var_os(var).map(PathBuf::from).filter(|p| !p.as_os_str().is_empty())
    }

    fn join_or(base: &str, sub: &str) -> PathBuf {
        let mut p = PathBuf::from(base);
        p.push(sub);
        p
    }

    /// Path of the running executable.
    pub fn get_program_path() -> PathBuf {
        static CACHE: OnceLock<PathBuf> = OnceLock::new();
        CACHE
            .get_or_init(|| {
                // Prefer /proc/self/exe (Linux-specific reliable answer).
                if let Ok(target) = fs::read_link("/proc/self/exe") {
                    return target;
                }
                // Fall back to argv[0]; this is only an approximation.
                env::args().next().map(PathBuf::from).unwrap_or_default()
            })
            .clone()
    }

    /// Read-only resources shipped with the program (next to the binary).
    pub fn get_resources_path() -> PathBuf {
        get_program_path()
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_default()
    }

    fn xdg(env_var: &str, default_subdir: &str) -> PathBuf {
        if let Some(p) = read_env(env_var) {
            return p;
        }
        if let Some(home) = read_env("HOME") {
            return join_or(&home.to_string_lossy(), default_subdir);
        }
        PathBuf::from(default_subdir)
    }

    /// `$XDG_DATA_HOME` or `$HOME/.local/share`.
    pub fn get_data_path() -> PathBuf {
        xdg("XDG_DATA_HOME", ".local/share")
    }

    /// `$XDG_DATA_HOME` — same as [`get_data_path`] on Linux, no roaming.
    pub fn get_data_local_path() -> PathBuf {
        get_data_path()
    }

    /// `$XDG_CACHE_HOME` or `$HOME/.cache`.
    pub fn get_cache_path() -> PathBuf {
        xdg("XDG_CACHE_HOME", ".cache")
    }

    /// `$XDG_CONFIG_HOME` or `$HOME/.config`.
    pub fn get_config_path() -> PathBuf {
        xdg("XDG_CONFIG_HOME", ".config")
    }

    /// `$HOME` directory.
    pub fn get_userprofile_path() -> PathBuf {
        read_env("HOME").unwrap_or_default()
    }

    // -----------------------------------------------------------------------
    // Process / command-line
    // -----------------------------------------------------------------------

    /// Placeholder for the global "parsed command line" handle. The C++ side
    /// uses a small struct; we keep a process id for symmetry.
    #[derive(Clone, Debug, Default)]
    pub struct ProcessHandle {
        pub pid: i32,
    }

    /// Lazily-initialised handle to the running process.
    pub fn parent_process_handle() -> ProcessHandle {
        // SAFETY: getpid is async-signal-safe and has no preconditions.
        let pid = unsafe { getpid() };
        ProcessHandle { pid }
    }

    /// Marker for command-line options that get attached to the global state
    /// at start-up. The original C++ side uses a verbose `std::optional`-like
    /// wrapper; here we just expose a builder-style helper that records the
    /// argument and returns `true` so the caller's control-flow matches.
    #[derive(Clone, Debug, Default)]
    pub struct CommandLineOption {
        pub key: String,
        pub value: Option<String>,
    }

    impl CommandLineOption {
        pub fn new(key: impl Into<String>, value: Option<String>) -> Self {
            Self {
                key: key.into(),
                value,
            }
        }
    }

    /// Attach a parsed command-line option. Returns `true` on success.
    pub fn attach_command_line_option(opt: CommandLineOption) -> bool {
        // No persistent store in this translation; the C++ side stores into
        // a global `std::optional` value. We simply validate the input.
        !opt.key.is_empty()
    }

    // -----------------------------------------------------------------------
    // Thread-name helpers (prctl)
    // -----------------------------------------------------------------------

    /// Set the name of the current thread via `prctl(PR_SET_NAME, ...)`.
    ///
    /// The kernel truncates the name to 15 bytes (plus a NUL).
    pub fn set_thread_name(name: &str) {
        let truncated = if name.len() > 15 { &name[..15] } else { name };
        let cname = CString::new(truncated).unwrap_or_else(|_| CString::new("rust").unwrap());
        // SAFETY: PR_SET_NAME takes a NUL-terminated string of at most 16 bytes.
        unsafe {
            prctl(PR_SET_NAME, cname.as_ptr(), 0u64, 0u64, 0u64);
        }
    }

    /// Read the current thread's name via `prctl(PR_GET_NAME, ...)`.
    pub fn get_thread_name() -> String {
        let mut buf = [0u8; 16];
        // SAFETY: PR_GET_NAME writes up to 16 bytes (including NUL) into buf.
        let ret = unsafe {
            prctl(
                PR_GET_NAME,
                buf.as_mut_ptr() as *mut c_char,
                0u64,
                0u64,
                0u64,
            )
        };
        if ret < 0 {
            return String::new();
        }
        let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        String::from_utf8_lossy(&buf[..len]).into_owned()
    }

    // -----------------------------------------------------------------------
    // Misc utilities translated from LnxMisc.cpp
    // -----------------------------------------------------------------------

    /// Total physical RAM in bytes. `0` on failure.
    pub fn get_physical_memory() -> u64 {
        // _SC_PHYS_PAGES is in the libc crate, but we keep this self-contained
        // by reading `/proc/meminfo` instead — same data, no extra deps.
        read_meminfo_kb("MemTotal").saturating_mul(1024)
    }

    fn read_meminfo_kb(key: &str) -> u64 {
        let file = match fs::File::open("/proc/meminfo") {
            Ok(f) => f,
            Err(_) => return 0,
        };
        let prefix = format!("{}:", key);
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            if let Some(rest) = line.strip_prefix(&prefix) {
                // e.g. " 16384000 kB"
                let mut it = rest.split_whitespace();
                if let Some(value) = it.next() {
                    if let Ok(n) = value.parse::<u64>() {
                        return n;
                    }
                }
            }
        }
        0
    }

    /// Sleep the current thread for `ms` milliseconds.
    pub fn sleep_ms(ms: u32) {
        // SAFETY: usleep is async-signal-safe; argument is in microseconds.
        unsafe {
            usleep(ms.saturating_mul(1000));
        }
    }

    /// Yield the current timeslice to other runnable threads.
    pub fn timeslice() {
        // SAFETY: sched_yield is always safe to call.
        unsafe {
            sched_yield();
        }
    }

    /// Busy-wait / hint the CPU that we're in a spin loop.
    #[inline]
    pub fn spin_wait() {
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        {
            // SAFETY: the `pause` instruction has no preconditions.
            unsafe {
                std::arch::asm!("pause", options(nomem, preserves_flags));
            }
        }
        #[cfg(target_arch = "aarch64")]
        {
            // SAFETY: the `isb` instruction has no preconditions.
            unsafe {
                std::arch::asm!("isb", options(nomem, preserves_flags));
            }
        }
        #[cfg(not(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")))]
        {
            thread::yield_now();
        }
    }

    /// Cheap delay between polling attempts.
    pub fn sleep_and_spin_wait(ms: u32) {
        sleep_ms(ms);
        spin_wait();
    }

    /// Read a human-readable OS version string from `/etc/os-release`.
    pub fn os_version_string() -> String {
        let Ok(file) = fs::File::open("/etc/os-release") else {
            return "Linux".to_string();
        };
        let mut name = String::new();
        let mut version = String::new();
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            if let Some(v) = line.strip_prefix("NAME=") {
                name = strip_quotes(v);
            } else if line.starts_with("VERSION_ID=") {
                version = strip_quotes(line.trim_start_matches("VERSION_ID="));
            } else if line.starts_with("BUILD_ID=") && version.is_empty() {
                version = strip_quotes(line.trim_start_matches("BUILD_ID="));
            }
        }
        if name.is_empty() {
            return "Linux".to_string();
        }
        if version.is_empty() {
            return name;
        }
        format!("{name} {version}")
    }

    fn strip_quotes(s: &str) -> String {
        let s = s.trim();
        let s = s.trim_matches('"');
        s.to_string()
    }

    // -----------------------------------------------------------------------
    // Re-exports
    // -----------------------------------------------------------------------

    pub use std::path::PathBuf as Path;
}

// ---------------------------------------------------------------------------
// Public re-exports (Linux)
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
pub use imp::*;

// ---------------------------------------------------------------------------
// Non-Linux stubs
// ---------------------------------------------------------------------------

#[cfg(not(target_os = "linux"))]
mod imp {
    use std::path::PathBuf;
    use std::time::Duration;

    pub fn get_program_path() -> PathBuf {
        PathBuf::new()
    }
    pub fn get_resources_path() -> PathBuf {
        PathBuf::new()
    }
    pub fn get_data_path() -> PathBuf {
        PathBuf::new()
    }
    pub fn get_data_local_path() -> PathBuf {
        PathBuf::new()
    }
    pub fn get_cache_path() -> PathBuf {
        PathBuf::new()
    }
    pub fn get_config_path() -> PathBuf {
        PathBuf::new()
    }
    pub fn get_userprofile_path() -> PathBuf {
        PathBuf::new()
    }

    pub fn set_thread_name(_name: &str) {}
    pub fn get_thread_name() -> String {
        String::new()
    }

    #[derive(Clone, Debug, Default)]
    pub struct ProcessHandle {
        pub pid: i32,
    }
    pub fn parent_process_handle() -> ProcessHandle {
        ProcessHandle::default()
    }

    #[derive(Clone, Debug, Default)]
    pub struct CommandLineOption {
        pub key: String,
        pub value: Option<String>,
    }
    impl CommandLineOption {
        pub fn new(key: impl Into<String>, value: Option<String>) -> Self {
            Self {
                key: key.into(),
                value,
            }
        }
    }
    pub fn attach_command_line_option(_opt: CommandLineOption) -> bool {
        false
    }

    pub fn get_physical_memory() -> u64 {
        0
    }
    pub fn sleep_ms(_ms: u32) {
        // Best-effort fallback using std.
        std::thread::sleep(Duration::from_millis(0));
    }
    pub fn timeslice() {
        std::thread::yield_now();
    }
    pub fn spin_wait() {
        std::hint::spin_loop();
    }
    pub fn sleep_and_spin_wait(ms: u32) {
        std::thread::sleep(Duration::from_millis(ms as u64));
    }
    pub fn os_version_string() -> String {
        String::new()
    }
}

#[cfg(not(target_os = "linux"))]
pub use imp::*;
