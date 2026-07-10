// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Linux host-system facilities.
//!
//! Idiomatic Rust 2021 translation of PCSX2's
//! `common/Linux/LnxHostSys.cpp`. The Linux-specific implementation
//! lives behind `#[cfg(target_os = "linux")]`; non-Linux targets
//! compile to no-op stubs so callers can use the API uniformly across
//! platforms.
//!
//! Scope (mirrors the C++ side):
//!
//! - Memory protection via `mprotect(2)`.
//! - Process-private shared memory creation via `shm_open(2)` +
//!   `ftruncate(2)`, returned as a file-descriptor-as-pointer (matching
//!   the C++ convention).
//! - Shared-memory region reservation via `mmap(2)` / `munmap(2)`
//!   (the `SharedMemoryMappingArea` analogue).
//! - Runtime page size, cache line size, tick frequency / counter
//!   (`sysconf`, `clock_gettime`).
//! - Physical / available memory via `sysinfo(2)`.
//! - OS version string via `uname(2)`.
//! - CPU info via `/proc/cpuinfo`.
//!
//! # FFI surface
//!
//! `pcsx2_host_*` entry points are `#[no_mangle] pub extern "C"` and
//! are picked up by `cbindgen` from this module.

#![cfg_attr(
    not(target_os = "linux"),
    allow(dead_code, unused_imports, unused_variables)
)]

use std::ffi::CString;
use std::fs;
use std::mem::MaybeUninit;
use std::os::raw::c_char;
use std::ptr;

// ============================================================================
// Linux implementation
// ============================================================================

#[cfg(target_os = "linux")]
mod linux {
    use super::*;

    // -----------------------------------------------------------------------
    // Local constants
    //
    // The `libc` crate exposes most of these but not always in a
    // way that's `const`-friendly on every tier-1 target. We redeclare
    // the values we use explicitly to keep this module self-contained.
    // -----------------------------------------------------------------------

    // mmap / mprotect protection bits.
    const PROT_NONE: libc::c_int = 0;
    #[allow(dead_code)]
    const PROT_READ: libc::c_int = 1;
    #[allow(dead_code)]
    const PROT_WRITE: libc::c_int = 2;
    #[allow(dead_code)]
    const PROT_EXEC: libc::c_int = 4;

    // mmap flags.
    #[allow(dead_code)]
    const MAP_SHARED: libc::c_int = 0x01;
    #[allow(dead_code)]
    const MAP_PRIVATE: libc::c_int = 0x02;
    #[allow(dead_code)]
    const MAP_FIXED: libc::c_int = 0x10;
    #[allow(dead_code)]
    const MAP_ANONYMOUS: libc::c_int = 0x20;

    // shm_open / open flags.
    #[allow(dead_code)]
    const O_RDWR: libc::c_int = 0x2;
    #[allow(dead_code)]
    const O_CREAT: libc::c_int = 0x40;
    #[allow(dead_code)]
    const O_EXCL: libc::c_int = 0x80;

    // File mode bits for the created shared-memory inode.
    const SHM_MODE: libc::mode_t = 0o600;

    // FFI bit-to-PROT_* mapping (must match the C++ side's `prot` arg).
    const FPROT_READ: u32 = 1 << 0;
    const FPROT_WRITE: u32 = 1 << 1;
    const FPROT_EXEC: u32 = 1 << 2;

    // -----------------------------------------------------------------------
    // Memory protection
    // -----------------------------------------------------------------------

    /// Translate (read, write, exec) booleans into a `mprotect(2)`
    /// protection mask.
    ///
    /// The Linux side-effect of mapping `exec` to `PROT_READ|PROT_EXEC`
    /// matches the C++ original's behaviour: an executable region is
    /// always readable on Linux, because i386 / x86_64 cannot execute
    /// a non-readable page.
    fn prot_from_rwx(read: bool, write: bool, exec: bool) -> libc::c_int {
        let mut prot = PROT_NONE;
        if read {
            prot |= PROT_READ;
        }
        if write {
            prot |= PROT_WRITE;
        }
        if exec {
            // C++ LinuxProt() forces PROT_READ on for any PROT_EXEC
            // mapping. Preserve that here.
            prot |= PROT_READ | PROT_EXEC;
        }
        prot
    }

    /// Translate the FFI `prot` bitmask to a Linux `mprotect` mask.
    fn prot_from_bits(prot: u32) -> libc::c_int {
        let mut bits = PROT_NONE;
        if prot & FPROT_READ != 0 {
            bits |= PROT_READ;
        }
        if prot & FPROT_WRITE != 0 {
            bits |= PROT_WRITE;
        }
        if prot & FPROT_EXEC != 0 {
            bits |= PROT_READ | PROT_EXEC;
        }
        bits
    }

    /// Apply memory protection to an existing region.
    ///
    /// Returns `true` on success, `false` if `mprotect(2)` failed.
    /// The caller is responsible for ensuring that `base` is page-aligned
    /// and that `size` is a multiple of the runtime page size.
    pub fn mem_protect(
        base: *mut u8,
        size: usize,
        read: bool,
        write: bool,
        exec: bool,
    ) -> bool {
        let prot = prot_from_rwx(read, write, exec);
        // SAFETY: `mprotect` is safe to call with any pointer and size
        // pair; the kernel validates alignment and permissions. A
        // failed call leaves the region untouched and returns -1.
        let ret = unsafe { libc::mprotect(base as *mut libc::c_void, size, prot) };
        ret == 0
    }

    // -----------------------------------------------------------------------
    // Shared memory
    // -----------------------------------------------------------------------

    /// Create a process-private shared memory region.
    ///
    /// The returned `*mut u8` is actually a file descriptor (matching the
    /// C++ convention of returning `intptr_t` as `void*`). The caller is
    /// expected to either `mmap` it themselves or pass it back to
    /// [`destroy_shared_memory`].
    ///
    /// The kernel-side name is removed via `shm_unlink` immediately, so
    /// the region is only reachable via the inherited fd.
    pub fn create_shared_memory(name: &str, size: usize) -> *mut u8 {
        let cname = match CString::new(name) {
            Ok(s) => s,
            Err(_) => return ptr::null_mut(),
        };

        // shm_open(name, O_CREAT | O_EXCL | O_RDWR, 0600)
        let fd = unsafe {
            libc::shm_open(
                cname.as_ptr(),
                O_CREAT | O_EXCL | O_RDWR,
                SHM_MODE,
            )
        };
        if fd < 0 {
            return ptr::null_mut();
        }

        // Process-private: remove the name immediately so only this
        // process can reach the inode via the fd.
        // (Failure is non-fatal — the fd is still valid.)
        unsafe {
            libc::shm_unlink(cname.as_ptr());
        }

        // ftruncate to the requested size.
        let trunc = unsafe { libc::ftruncate(fd, size as libc::off_t) };
        if trunc < 0 {
            // Close the fd on failure so we don't leak it.
            unsafe {
                libc::close(fd);
            }
            return ptr::null_mut();
        }

        fd as isize as *mut u8
    }

    /// Destroy a shared memory region previously created by
    /// [`create_shared_memory`].
    ///
    /// The `size` parameter is part of the public signature for parity
    /// with other platforms but is unused here: Linux just needs the
    /// file descriptor closed.
    pub fn destroy_shared_memory(ptr: *mut u8, _size: usize) {
        if ptr.is_null() {
            return;
        }
        // SAFETY: `ptr` carries a file descriptor created by
        // `create_shared_memory`; closing it once is well-defined.
        unsafe {
            libc::close(ptr as isize as libc::c_int);
        }
    }

    /// Map a shared memory fd into an address range previously reserved
    /// by [`SharedMemoryMappingArea::create`].
    ///
    /// Returns the mapped base pointer, or null on failure.
    pub fn map_shared_memory(
        map_base: *mut u8,
        map_size: usize,
        file_handle: *mut u8,
        file_offset: usize,
        read: bool,
        write: bool,
        exec: bool,
    ) -> *mut u8 {
        let prot = prot_from_rwx(read, write, exec);
        let ret = if !file_handle.is_null() {
            let fd = file_handle as isize as libc::c_int;
            // SAFETY: `map_base`/`map_size` describe a reservation made
            // with MAP_FIXED; `mmap` with MAP_FIXED | MAP_SHARED
            // replaces that range atomically.
            unsafe {
                libc::mmap(
                    map_base as *mut libc::c_void,
                    map_size,
                    prot,
                    MAP_SHARED | MAP_FIXED,
                    fd,
                    file_offset as libc::off_t,
                )
            }
        } else {
            // No backing fd: just `mprotect` the reserved range to the
            // requested mode (matches the macOS MAP_JIT path).
            // SAFETY: see above.
            unsafe { libc::mprotect(map_base as *mut libc::c_void, map_size, prot) }
        };
        if ret == libc::MAP_FAILED || (file_handle.is_null() && ret != 0) {
            return ptr::null_mut();
        }
        map_base
    }

    /// Unmap a previously-mapped slice of a shared-memory reservation,
    /// replacing it with a PROT_NONE anonymous mapping so the address
    /// range stays reserved but inaccessible.
    pub fn unmap_shared_memory(map_base: *mut u8, map_size: usize) -> bool {
        // SAFETY: `map_base`/`map_size` describe a region we previously
        // mapped; replacing it with PROT_NONE anonymous preserves the
        // reservation.
        let ret = unsafe {
            libc::mmap(
                map_base as *mut libc::c_void,
                map_size,
                PROT_NONE,
                MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED,
                -1,
                0,
            )
        };
        ret != libc::MAP_FAILED
    }

    // -----------------------------------------------------------------------
    // SharedMemoryMappingArea analogue
    // -----------------------------------------------------------------------

    /// A reserved address range used to back process-shared mappings.
    ///
    /// Mirrors the C++ `SharedMemoryMappingArea` class. The region is
    /// allocated with `mmap(..., PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS)`
    /// and torn down with `munmap`.
    pub struct SharedMemoryMappingArea {
        base: *mut u8,
        size: usize,
        /// Number of live sub-mappings; the destructor asserts zero.
        num_mappings: u32,
    }

    // SAFETY: The kernel object the pointer references is process-local
    // (anonymous mmap). We synchronise via the raw pointer only, and
    // Rust's `Send` / `Sync` requirements match the C++ side's implicit
    // (no) requirements: callers must coordinate access.
    unsafe impl Send for SharedMemoryMappingArea {}
    unsafe impl Sync for SharedMemoryMappingArea {}

    impl SharedMemoryMappingArea {
        /// Create and reserve a new shared-memory mapping area of
        /// `size` bytes. Returns `None` if the reservation fails.
        pub fn create(size: usize) -> Option<Self> {
            // SAFETY: anonymous PROT_NONE mapping; the kernel returns
            // MAP_FAILED on failure.
            let alloc = unsafe {
                libc::mmap(
                    ptr::null_mut(),
                    size,
                    PROT_NONE,
                    MAP_PRIVATE | MAP_ANONYMOUS,
                    -1,
                    0,
                )
            };
            if alloc == libc::MAP_FAILED {
                return None;
            }
            Some(Self {
                base: alloc as *mut u8,
                size,
                num_mappings: 0,
            })
        }

        /// Base pointer of the reservation.
        pub fn base_ptr(&self) -> *mut u8 {
            self.base
        }

        /// Total size of the reservation, in bytes.
        pub fn size(&self) -> usize {
            self.size
        }

        /// Map `file_handle` (or just change protection if null) into
        /// a sub-range of this reservation.
        pub fn map(
            &mut self,
            map_base: *mut u8,
            map_size: usize,
            file_handle: *mut u8,
            file_offset: usize,
            read: bool,
            write: bool,
            exec: bool,
        ) -> Option<*mut u8> {
            assert!(
                map_base >= self.base && map_base < unsafe { self.base.add(self.size) },
                "map_base outside reservation",
            );
            let ptr = map_shared_memory(map_base, map_size, file_handle, file_offset, read, write, exec);
            if !ptr.is_null() {
                self.num_mappings = self.num_mappings.saturating_add(1);
            }
            if ptr.is_null() {
                None
            } else {
                Some(ptr)
            }
        }

        /// Unmap a previously-mapped sub-range.
        pub fn unmap(&mut self, map_base: *mut u8, map_size: usize) -> bool {
            assert!(
                map_base >= self.base && map_base < unsafe { self.base.add(self.size) },
                "map_base outside reservation",
            );
            let ok = unmap_shared_memory(map_base, map_size);
            if ok {
                self.num_mappings = self.num_mappings.saturating_sub(1);
            }
            ok
        }
    }

    impl Drop for SharedMemoryMappingArea {
        fn drop(&mut self) {
            debug_assert_eq!(self.num_mappings, 0, "SharedMemoryMappingArea dropped with live mappings");
            // SAFETY: the reservation was made by `create` and not
            // yet released.
            unsafe {
                libc::munmap(self.base as *mut libc::c_void, self.size);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Page / cache line size
    // -----------------------------------------------------------------------

    /// Returns the runtime memory page size of the host, in bytes.
    pub fn get_runtime_page_size() -> usize {
        // SAFETY: `sysconf` is safe to call with any valid `_SC_*`
        // constant; on error it returns -1 and sets `errno`.
        let pages = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if pages < 0 {
            4096
        } else {
            pages as usize
        }
    }

    /// Returns the L1 data cache line size of the host, in bytes.
    ///
    /// Reads `sysconf(_SC_LEVEL1_DCACHE_LINESIZE)` first; if that's
    /// unavailable, walks `/sys/devices/system/cpu/cpu0/cache/indexN/`
    /// and returns the largest `coherency_line_size` reported, falling
    /// back to 64 bytes (the x86_64 / aarch64 baseline).
    pub fn get_runtime_cache_line_size() -> usize {
        // SAFETY: see `get_runtime_page_size`.
        let l1d = unsafe { libc::sysconf(libc::_SC_LEVEL1_DCACHE_LINESIZE) };
        let l1i = unsafe { libc::sysconf(libc::_SC_LEVEL1_ICACHE_LINESIZE) };
        let mut best = if l1d > l1i { l1d } else { l1i };
        if best <= 0 {
            best = 0;
        }

        // Walk /sys/devices/system/cpu/cpu0/cache/indexN looking for
        // larger coherency_line_size values. This mirrors the C++
        // behaviour exactly.
        for index in 0..16 {
            let path = format!(
                "/sys/devices/system/cpu/cpu0/cache/index{index}/coherency_line_size"
            );
            let Ok(contents) = fs::read_to_string(&path) else {
                break;
            };
            if let Ok(val) = contents.trim().parse::<i32>() {
                if val > best {
                    best = val;
                }
            }
        }

        if best > 0 {
            best as usize
        } else {
            64
        }
    }

    // -----------------------------------------------------------------------
    // Tick counter / frequency
    // -----------------------------------------------------------------------

    /// Returns the frequency of the monotonic tick counter, in ticks
    /// per second.
    ///
    /// Backing counter is `clock_gettime(CLOCK_MONOTONIC_RAW, ...)`,
    /// whose unit is exactly one nanosecond.
    pub fn get_tick_frequency() -> u64 {
        1_000_000_000
    }

    /// Returns the current value of the monotonic tick counter, in
    /// ticks (nanoseconds since an unspecified epoch).
    pub fn get_cpu_ticks() -> u64 {
        // SAFETY: `clock_gettime` only writes through the pointer; the
        // backing storage is a freshly-zeroed `MaybeUninit`.
        let mut ts = MaybeUninit::<libc::timespec>::zeroed();
        let ok = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC_RAW, ts.as_mut_ptr()) };
        if ok != 0 {
            return 0;
        }
        // SAFETY: `clock_gettime` returned 0, so `ts` was fully
        // initialised.
        let ts = unsafe { ts.assume_init() };
        let secs = ts.tv_sec.max(0) as u64;
        let nanos = ts.tv_nsec.max(0) as u64;
        secs.saturating_mul(1_000_000_000).saturating_add(nanos)
    }

    // -----------------------------------------------------------------------
    // Physical memory (sysinfo)
    // -----------------------------------------------------------------------

    /// Returns the total physical memory in the host, in bytes.
    pub fn get_physical_memory() -> u64 {
        // SAFETY: `sysinfo` writes through the pointer; the storage is
        // a freshly-zeroed `MaybeUninit`.
        let mut info = MaybeUninit::<libc::sysinfo>::zeroed();
        let ok = unsafe { libc::sysinfo(info.as_mut_ptr()) };
        if ok != 0 {
            return 0;
        }
        // SAFETY: `sysinfo` returned 0; the struct is fully initialised.
        let info = unsafe { info.assume_init() };
        let unit = info.mem_unit.max(1) as u64;
        (info.totalram as u64).saturating_mul(unit)
    }

    /// Returns the available physical memory in the host, in bytes.
    pub fn get_available_memory() -> u64 {
        // SAFETY: see `get_physical_memory`.
        let mut info = MaybeUninit::<libc::sysinfo>::zeroed();
        let ok = unsafe { libc::sysinfo(info.as_mut_ptr()) };
        if ok != 0 {
            return 0;
        }
        // SAFETY: see `get_physical_memory`.
        let info = unsafe { info.assume_init() };
        let unit = info.mem_unit.max(1) as u64;
        (info.freeram as u64).saturating_mul(unit)
    }

    // -----------------------------------------------------------------------
    // OS version string
    // -----------------------------------------------------------------------

    /// Returns a human-readable description of the host OS.
    ///
    /// Built from `uname(2)`'s `sysname`, `release`, `version`, and
    /// `machine` fields. Mirrors the C++ `GetOSVersionString()` output
    /// format.
    pub fn get_os_version_string() -> String {
        // SAFETY: `uname` writes through the pointer; the storage is a
        // freshly-zeroed `MaybeUninit`.
        let mut uts = MaybeUninit::<libc::utsname>::zeroed();
        let ok = unsafe { libc::uname(uts.as_mut_ptr()) };
        if ok != 0 {
            return String::from("Unknown Linux");
        }
        // SAFETY: `uname` returned 0.
        let uts = unsafe { uts.assume_init() };
        let sysname = cstr_array_to_str(&uts.sysname);
        let release = cstr_array_to_str(&uts.release);
        let version = cstr_array_to_str(&uts.version);
        let machine = cstr_array_to_str(&uts.machine);
        if version.is_empty() {
            format!("{sysname} {release} ({machine})")
        } else {
            format!("{sysname} {release} {version} ({machine})")
        }
    }

    // -----------------------------------------------------------------------
    // CPU information
    // -----------------------------------------------------------------------

    /// Description of the host CPU.
    ///
    /// Mirrors the C++ `CPUInfo` struct. The big/small core distinction
    /// is left at zero on Linux (a future revision can derive it from
    /// `/sys/devices/system/cpu/cpu*/cpufreq`).
    #[derive(Clone, Debug, Default)]
    pub struct CpuInfo {
        /// Brand / model string from `model name` in `/proc/cpuinfo`.
        pub name: String,
        /// Number of cores in the highest-frequency cluster.
        pub num_big_cores: u32,
        /// Number of cores in all lower-frequency clusters.
        pub num_small_cores: u32,
        /// Total number of logical processors.
        pub num_threads: u32,
        /// Number of frequency-distinct clusters.
        pub num_clusters: u32,
    }

    impl CpuInfo {
        /// Empty placeholder.
        pub const fn empty() -> Self {
            Self {
                name: String::new(),
                num_big_cores: 0,
                num_small_cores: 0,
                num_threads: 0,
                num_clusters: 0,
            }
        }
    }

    /// Returns information about the host CPU.
    pub fn get_cpu_info() -> CpuInfo {
        let mut info = CpuInfo::empty();
        // /proc/cpuinfo may be missing or unreadable on some sandboxes;
        // fail soft and return what we have.
        let Ok(contents) = fs::read_to_string("/proc/cpuinfo") else {
            return info;
        };
        let mut threads: u32 = 0;
        for line in contents.lines() {
            if let Some(rest) = line.strip_prefix("processor") {
                // "processor\t: 0"
                if rest.trim_start().starts_with(':') {
                    threads = threads.saturating_add(1);
                }
            } else if info.name.is_empty() {
                if let Some(value) = line.strip_prefix("model name") {
                    if let Some(value) = parse_proc_kv(value) {
                        info.name = value;
                    }
                } else if let Some(value) = line.strip_prefix("Hardware") {
                    // aarch64 fallback: model name isn't always present.
                    if let Some(value) = parse_proc_kv(value) {
                        info.name = value;
                    }
                }
            }
        }
        info.num_threads = threads;
        info
    }

    fn parse_proc_kv(after_key: &str) -> Option<String> {
        let trimmed = after_key.trim_start();
        let after_colon = trimmed.strip_prefix(':')?;
        Some(after_colon.trim().to_string())
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Convert a fixed-size C string array (e.g. `utsname.sysname`) to
    /// `&str` by stripping trailing NULs.
    fn cstr_array_to_str(buf: &[libc::c_char]) -> &str {
        let nul_pos = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, nul_pos) };
        std::str::from_utf8(bytes).unwrap_or("")
    }

    /// Write a Rust string into a caller-supplied C buffer with
    /// explicit bound. Truncates if the destination is too small,
    /// always NUL-terminates (when `out_len > 0`), and returns the
    /// number of bytes that *would* have been written (excluding the
    /// terminator) — the `snprintf` convention.
    pub(crate) fn write_c_string(out: *mut c_char, out_len: u32, s: &str) -> u32 {
        if out.is_null() || out_len == 0 {
            return s.len() as u32;
        }
        let cap = (out_len as usize).saturating_sub(1);
        let bytes = s.as_bytes();
        let to_copy = bytes.len().min(cap);
        // SAFETY: caller guarantees `out` is non-null and `out_len`
        // bytes are writable; we never read past `to_copy` and always
        // write a NUL terminator.
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), out as *mut u8, to_copy);
            *out.add(to_copy) = 0;
        }
        bytes.len() as u32
    }

    // -----------------------------------------------------------------------
    // FFI exports
    // -----------------------------------------------------------------------

    /// FFI: change protection on a region.
    ///
    /// `prot` is a bitmask: bit 0 = read, bit 1 = write, bit 2 = exec.
    #[no_mangle]
    pub extern "C" fn pcsx2_host_mem_protect(
        base: *mut u8,
        size: usize,
        prot: u32,
    ) -> bool {
        let read = prot & FPROT_READ != 0;
        let write = prot & FPROT_WRITE != 0;
        let exec = prot & FPROT_EXEC != 0;
        mem_protect(base, size, read, write, exec)
    }

    /// FFI: runtime page size, in bytes.
    #[no_mangle]
    pub extern "C" fn pcsx2_host_page_size() -> u32 {
        get_runtime_page_size() as u32
    }

    /// FFI: L1 data cache line size, in bytes.
    #[no_mangle]
    pub extern "C" fn pcsx2_host_cache_line_size() -> u32 {
        get_runtime_cache_line_size() as u32
    }

    /// FFI: tick frequency, in ticks per second.
    #[no_mangle]
    pub extern "C" fn pcsx2_host_tick_frequency() -> u64 {
        get_tick_frequency()
    }

    /// FFI: current value of the monotonic tick counter.
    #[no_mangle]
    pub extern "C" fn pcsx2_host_cpu_ticks() -> u64 {
        get_cpu_ticks()
    }

    /// FFI: total physical memory, in bytes.
    #[no_mangle]
    pub extern "C" fn pcsx2_host_physical_memory() -> u64 {
        get_physical_memory()
    }

    /// FFI: available physical memory, in bytes.
    #[no_mangle]
    pub extern "C" fn pcsx2_host_available_memory() -> u64 {
        get_available_memory()
    }

    /// FFI: write the OS version string into a caller-supplied buffer.
    ///
    /// Returns the number of bytes that *would* have been written
    /// (excluding the NUL). A return value `>= out_len` indicates
    /// truncation.
    #[no_mangle]
    pub extern "C" fn pcsx2_host_os_version_string(out: *mut c_char, out_len: u32) -> u32 {
        write_c_string(out, out_len, &get_os_version_string())
    }
}

// ============================================================================
// Non-Linux stubs
// ============================================================================

#[cfg(not(target_os = "linux"))]
mod linux {
    use super::*;

    /// Always returns `false` on non-Linux targets.
    pub fn mem_protect(
        _base: *mut u8,
        _size: usize,
        _read: bool,
        _write: bool,
        _exec: bool,
    ) -> bool {
        false
    }

    /// Always returns null on non-Linux targets.
    pub fn create_shared_memory(_name: &str, _size: usize) -> *mut u8 {
        ptr::null_mut()
    }

    /// No-op on non-Linux targets.
    pub fn destroy_shared_memory(_ptr: *mut u8, _size: usize) {}

    /// Returns a fixed 4096-byte page size.
    pub fn get_runtime_page_size() -> usize {
        4096
    }

    /// Returns the conservative 64-byte baseline.
    pub fn get_runtime_cache_line_size() -> usize {
        64
    }

    /// 1 GHz nominal tick frequency.
    pub fn get_tick_frequency() -> u64 {
        1_000_000_000
    }

    /// Returns zero ticks (no monotonic source on the stub path).
    pub fn get_cpu_ticks() -> u64 {
        0
    }

    /// Returns zero on non-Linux targets.
    pub fn get_physical_memory() -> u64 {
        0
    }

    /// Returns zero on non-Linux targets.
    pub fn get_available_memory() -> u64 {
        0
    }

    /// Empty OS version string.
    pub fn get_os_version_string() -> String {
        String::new()
    }

    /// Empty CpuInfo.
    #[derive(Clone, Debug, Default)]
    pub struct CpuInfo {
        pub name: String,
        pub num_big_cores: u32,
        pub num_small_cores: u32,
        pub num_threads: u32,
        pub num_clusters: u32,
    }

    impl CpuInfo {
        pub const fn empty() -> Self {
            Self {
                name: String::new(),
                num_big_cores: 0,
                num_small_cores: 0,
                num_threads: 0,
                num_clusters: 0,
            }
        }
    }

    pub fn get_cpu_info() -> CpuInfo {
        CpuInfo::empty()
    }

    pub(crate) fn write_c_string(out: *mut c_char, out_len: u32, s: &str) -> u32 {
        if out.is_null() || out_len == 0 {
            return s.len() as u32;
        }
        let cap = (out_len as usize).saturating_sub(1);
        let bytes = s.as_bytes();
        let to_copy = bytes.len().min(cap);
        // SAFETY: see Linux impl.
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), out as *mut u8, to_copy);
            *out.add(to_copy) = 0;
        }
        bytes.len() as u32
    }

    // ---- FFI exports -------------------------------------------------------

    #[no_mangle]
    pub extern "C" fn pcsx2_host_mem_protect(
        _base: *mut u8,
        _size: usize,
        _prot: u32,
    ) -> bool {
        false
    }

    #[no_mangle]
    pub extern "C" fn pcsx2_host_page_size() -> u32 {
        4096
    }

    #[no_mangle]
    pub extern "C" fn pcsx2_host_cache_line_size() -> u32 {
        64
    }

    #[no_mangle]
    pub extern "C" fn pcsx2_host_tick_frequency() -> u64 {
        1_000_000_000
    }

    #[no_mangle]
    pub extern "C" fn pcsx2_host_cpu_ticks() -> u64 {
        0
    }

    #[no_mangle]
    pub extern "C" fn pcsx2_host_physical_memory() -> u64 {
        0
    }

    #[no_mangle]
    pub extern "C" fn pcsx2_host_available_memory() -> u64 {
        0
    }

    #[no_mangle]
    pub extern "C" fn pcsx2_host_os_version_string(out: *mut c_char, out_len: u32) -> u32 {
        write_c_string(out, out_len, "")
    }
}

// ============================================================================
// Public re-exports
// ============================================================================

pub use linux::{
    create_shared_memory, destroy_shared_memory, get_available_memory, get_cpu_info, get_cpu_ticks,
    get_os_version_string, get_physical_memory, get_runtime_cache_line_size, get_runtime_page_size,
    get_tick_frequency, mem_protect, CpuInfo,
};

#[cfg(target_os = "linux")]
pub use linux::{
    map_shared_memory, unmap_shared_memory, SharedMemoryMappingArea,
};

// ============================================================================
// Tests (Linux-only — the non-Linux stubs are constant-returning)
// ============================================================================

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::ptr;

    #[test]
    fn page_size_is_nonzero() {
        let p = get_runtime_page_size();
        assert!(p > 0);
        assert!(p.is_power_of_two());
    }

    #[test]
    fn cache_line_size_is_sensible() {
        let c = get_runtime_cache_line_size();
        assert!((16..=256).contains(&c), "unexpected cache line: {c}");
    }

    #[test]
    fn tick_frequency_is_nonzero() {
        assert!(get_tick_frequency() > 0);
    }

    #[test]
    fn cpu_ticks_monotonic() {
        let t1 = get_cpu_ticks();
        let t2 = get_cpu_ticks();
        assert!(t2 >= t1, "tick counter went backwards: {t1} -> {t2}");
    }

    #[test]
    fn available_memory_is_bounded_by_total() {
        let total = get_physical_memory();
        let avail = get_available_memory();
        assert!(avail <= total, "available {avail} > total {total}");
    }

    #[test]
    fn os_version_string_nonempty() {
        let s = get_os_version_string();
        assert!(!s.is_empty());
    }

    #[test]
    fn cpu_info_runs() {
        let _ = get_cpu_info();
    }

    #[test]
    fn shared_memory_create_destroy_roundtrip() {
        let pid = unsafe { libc::getpid() };
        let name = format!("pcsx2_test_shm_{pid}");
        let ptr = create_shared_memory(&name, 4096);
        assert!(!ptr.is_null(), "create_shared_memory failed");
        destroy_shared_memory(ptr, 4096);
    }

    #[test]
    fn mapping_area_create_drop() {
        let area = SharedMemoryMappingArea::create(64 * 1024).expect("create");
        assert!(!area.base_ptr().is_null());
        assert_eq!(area.size(), 64 * 1024);
    }

    #[test]
    fn write_c_string_roundtrip() {
        let s = "hello";
        let mut buf = [0i8; 16];
        let n = linux::write_c_string(buf.as_mut_ptr(), buf.len() as u32, s);
        assert_eq!(n, s.len() as u32);
        let cstr = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) };
        assert_eq!(cstr.to_str().unwrap(), s);
    }

    #[test]
    fn write_c_string_truncates() {
        let mut tiny = [0i8; 4];
        let n = linux::write_c_string(tiny.as_mut_ptr(), tiny.len() as u32, "abcdef");
        assert_eq!(n, 6);
        assert_eq!(tiny[tiny.len() - 1], 0);
    }

    #[test]
    fn write_c_string_null_pointer() {
        let n = linux::write_c_string(ptr::null_mut(), 16, "abc");
        assert_eq!(n, 3);
    }
}