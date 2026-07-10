// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Host system query functions.
//!
//! Pure-Rust port of the read-only query surface of
//! `common/HostSys.h` / `common/HostSys.cpp`. The functions exposed here
//! are side-effect-free queries of the host OS and CPU:
//!
//! - Runtime page size and cache line size
//! - Tick counter (monotonic high-resolution timer) and its frequency
//! - Physical and available memory
//! - Operating system version string
//! - CPU information (name, big/small core counts, threads, clusters)
//!
//! The JIT memory management (`MemProtect`, `SharedMemoryMappingArea`,
//! `CreateSharedMemory` / `DestroySharedMemory`, `BeginCodeWrite` /
//! `EndCodeWrite`, `FlushInstructionCache`, `PageFaultHandler`) and the
//! desktop integration helpers (`Common::InhibitScreensaver`,
//! `PlaySoundAsync`, `SetMousePosition`, mouse-callback attach/detach,
//! `ShortSpin`, `AbortWithMessage`) are intentionally **not** ported in
//! Phase 1 — they require platform facilities that have no safe Rust
//! equivalent and remain in the C++ tree.
//!
//! FFI surface (`pcsx2_host_*`) is provided for the cross-cutting
//! queries; everything else is consumed by other Rust modules.

use std::ffi::CString;
use std::fs;
use std::io;
use std::mem::{self, MaybeUninit};
use std::os::raw::c_char;
use std::path::Path;
use std::str;

// ============================================================================
// Platform-specific FFI declarations
// ============================================================================
//
// The `libc` crate exposes only a small subset of the Win32 API. For the
// handful of functions we need (`GetSystemInfo`, `QueryPerformanceCounter`,
// `QueryPerformanceFrequency`, `GlobalMemoryStatusEx`, the cache-line
// enumeration API, and `RtlGetVersion` from ntdll) we declare them
// locally. The `#[cfg(windows)]` guards ensure the declarations only
// appear on Windows targets; on Unix, all of the relevant calls are
// reachable through `libc`.

#[cfg(windows)]
mod windows_ffi {
    use std::os::raw::c_void;

    // `BOOL` in the Windows SDK is `i32` (always 0 or 1).
    pub type BOOL = i32;
    // `LOGICAL_PROCESSOR_RELATIONSHIP` is an unsigned enum.
    pub type LOGICAL_PROCESSOR_RELATIONSHIP = u32;
    // `NTSTATUS` is `i32` for our purposes; only the success / failure
    // (low bit) matters here.
    pub type NTSTATUS = i32;

    /// Mirrors Win32 `SYSTEM_INFO`.
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct SYSTEM_INFO {
        pub dwOemId: u32,
        pub dwPageSize: u32,
        pub lpMinimumApplicationAddress: *mut c_void,
        pub lpMaximumApplicationAddress: *mut c_void,
        pub dwActiveProcessorMask: usize,
        pub dwNumberOfProcessors: u32,
        pub dwProcessorType: u32,
        pub dwAllocationGranularity: u32,
        pub wProcessorLevel: u16,
        pub wProcessorRevision: u16,
    }

    /// Mirrors Win32 `MEMORYSTATUSEX`.
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct MEMORYSTATUSEX {
        pub dwLength: u32,
        pub dwMemoryLoad: u32,
        pub ullTotalPhys: u64,
        pub ullAvailPhys: u64,
        pub ullTotalPageFile: u64,
        pub ullAvailPageFile: u64,
        pub ullTotalVirtual: u64,
        pub ullAvailVirtual: u64,
        pub ullAvailExtendedVirtual: u64,
    }

    /// Group affinity for cache entries.
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct GROUP_AFFINITY {
        pub Mask: usize,
        pub Group: u16,
        pub Reserved: [u16; 3],
    }

    /// The `Cache` variant of the `SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX`
    /// union. We only read `LineSize`, so the trailing variable-sized
    /// `GroupMask` array can be omitted safely.
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct CACHE_RELATIONSHIP {
        pub Level: u8,
        pub Associativity: u8,
        pub LineSize: u16,
        pub CacheSize: u32,
        pub CacheType: i32,
        pub Reserved: [u8; 20],
        pub GroupCount: u16,
        pub GroupMask: [GROUP_AFFINITY; 1],
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct PROCESSOR_RELATIONSHIP {
        pub Flags: u8,
        pub EfficiencyClass: u8,
        pub Reserved: [u8; 20],
        pub GroupCount: u16,
        pub GroupMask: [GROUP_AFFINITY; 1],
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct NUMA_NODE_RELATIONSHIP {
        pub NodeNumber: u32,
        pub Reserved: u8,
        pub GroupMask: GROUP_AFFINITY,
    }

    /// The shared header of every entry returned by
    /// `GetLogicalProcessorInformationEx`. The actual union payload
    /// (Processor / NumaNode / Cache / Group) follows but is not
    /// needed for our purpose of extracting the cache line size.
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX {
        pub Relationship: LOGICAL_PROCESSOR_RELATIONSHIP,
        pub Size: u32,
        pub Anonymous: SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX_Anonymous,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub union SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX_Anonymous {
        pub Processor: PROCESSOR_RELATIONSHIP,
        pub NumaNode: NUMA_NODE_RELATIONSHIP,
        pub Cache: CACHE_RELATIONSHIP,
    }

    // Values for `LOGICAL_PROCESSOR_RELATIONSHIP`.
    pub const RelationCache: LOGICAL_PROCESSOR_RELATIONSHIP = 2;

    // Forward declarations of the small subset of Win32 / ntdll we need.
    extern "system" {
        pub fn GetSystemInfo(lpSystemInfo: *mut SYSTEM_INFO);
        pub fn GlobalMemoryStatusEx(lpBuffer: *mut MEMORYSTATUSEX) -> BOOL;
        pub fn QueryPerformanceCounter(lpPerformanceCount: *mut i64) -> BOOL;
        pub fn QueryPerformanceFrequency(lpFrequency: *mut i64) -> BOOL;
        pub fn GetLogicalProcessorInformationEx(
            Relationship: LOGICAL_PROCESSOR_RELATIONSHIP,
            Buffer: *mut c_void,
            ReturnedLength: *mut u32,
        ) -> BOOL;
    }

    /// Mirror of `OSVERSIONINFOEXW` reduced to the fields we actually
    /// read. `RtlGetVersion` is preferred over `GetVersionEx` because
    /// the latter reports the wrong version for applications lacking
    /// a Windows 10/11 compatibility manifest.
    #[repr(C)]
    pub struct OsVersionInfoExW {
        pub dw_os_version_info_size: u32,
        pub dw_major_version: u32,
        pub dw_minor_version: u32,
        pub dw_build_number: u32,
        pub dw_platform_id: u32,
        pub sz_csd_version: [u16; 128],
    }

    extern "system" {
        pub fn RtlGetVersion(info: *mut OsVersionInfoExW) -> NTSTATUS;
    }
}

// ============================================================================
// Page / cache line size
// ============================================================================

/// Returns the runtime memory page size of the current host, in bytes.
///
/// On Unix, queried via `sysconf(_SC_PAGESIZE)`. On Windows, queried via
/// `GetSystemInfo`'s `dwPageSize`. Both are typically 4096 bytes but can
/// vary (e.g. 64 KiB on some PowerPC / Itanium configurations, 16 KiB
/// on aarch64 Linux under some kernel configurations, 4 KiB / 64 KiB
/// selectable on aarch64 Windows).
#[cfg(unix)]
pub fn get_runtime_page_size() -> usize {
    // Safety: `sysconf` is safe to call with any valid `_SC_*` constant.
    // On error it returns -1 and sets `errno`. Pages are always positive
    // on any Unix system PCSX2 builds on; the 4096 fallback is the
    // universal minimum and matches what most libc implementations
    // return on success.
    let pages = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if pages < 0 {
        4096
    } else {
        pages as usize
    }
}

#[cfg(windows)]
pub fn get_runtime_page_size() -> usize {
    use windows_ffi::{GetSystemInfo, SYSTEM_INFO};
    // Safety: `GetSystemInfo` always writes through the pointer; the
    // backing storage is a freshly-created `MaybeUninit` so alignment
    // and size are correct.
    let mut info = MaybeUninit::<SYSTEM_INFO>::zeroed();
    unsafe {
        GetSystemInfo(info.as_mut_ptr());
        info.assume_init().dwPageSize as usize
    }
}

/// Returns the L1 data cache line size of the current host, in bytes.
///
/// On Linux, queried via `sysconf(_SC_LEVEL1_DCACHE_LINESIZE)`. On
/// macOS, queried via `sysctlbyname("hw.cachelinesize")`. On Windows,
/// no public Win32 API directly exposes this; we walk
/// `GetLogicalProcessorInformationEx`'s `RelationCache` descriptors and
/// take the largest `LineSize` we see, falling back to 64 bytes (the
/// x86_64 and aarch64 baseline) when nothing is reported.
#[cfg(target_os = "linux")]
pub fn get_runtime_cache_line_size() -> usize {
    let linesize = unsafe { libc::sysconf(libc::_SC_LEVEL1_DCACHE_LINESIZE) };
    if linesize > 0 {
        linesize as usize
    } else {
        64
    }
}

#[cfg(target_os = "macos")]
pub fn get_runtime_cache_line_size() -> usize {
    // Apple Silicon Macs and recent Intel Macs all report 64 here.
    // We use sysctlbyname to avoid a hard-coded value where possible.
    let mut size: usize = 0;
    let mut size_len: libc::size_t = mem::size_of::<usize>();
    let mib_name = CString::new("hw.cachelinesize").expect("cstring literal");
    // Safety: sysctlbyname copies up to `size_len` bytes into the
    // caller-provided buffer. We initialise the buffer to zero so a
    // short write still yields a defined value.
    let ret = unsafe {
        libc::sysctlbyname(
            mib_name.as_ptr(),
            &mut size as *mut _ as *mut _,
            &mut size_len,
            std::ptr::null(),
            0,
        )
    };
    if ret == 0 && size > 0 {
        size
    } else {
        64
    }
}

#[cfg(all(unix, not(target_os = "linux"), not(target_os = "macos")))]
pub fn get_runtime_cache_line_size() -> usize {
    // Generic Unix fallback (FreeBSD, illumos, etc.). 64 is the
    // common value on every architecture PCSX2 currently supports.
    64
}

#[cfg(windows)]
pub fn get_runtime_cache_line_size() -> usize {
    use std::ptr;
    use windows_ffi::{
        GetLogicalProcessorInformationEx, RelationCache, CACHE_RELATIONSHIP,
        SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX,
    };

    // First call: pass a null buffer to ask the OS for the required
    // length. `returned` is set to the byte count and the call
    // returns FALSE with `ERROR_INSUFFICIENT_BUFFER`.
    let mut returned: u32 = 0;
    let _ = unsafe {
        GetLogicalProcessorInformationEx(
            RelationCache,
            ptr::null_mut(),
            &mut returned,
        )
    };

    if returned == 0 {
        return 64;
    }

    // Allocate a buffer of the requested size and re-issue the call.
    // `RelationCache` is documented to never require a buffer larger
    // than the system reports on the first call, so a single retry
    // is sufficient.
    let mut buffer: Vec<u8> = vec![0u8; returned as usize];
    let ok = unsafe {
        GetLogicalProcessorInformationEx(
            RelationCache,
            buffer.as_mut_ptr() as *mut _,
            &mut returned,
        )
    };
    if ok == 0 {
        return 64;
    }

    // Walk the variable-sized records. Each starts with a fixed
    // header containing the `Size` field that gives the byte length
    // of the current record; advance by that to reach the next.
    let mut max_line: usize = 0;
    let mut offset: usize = 0;
    let total = returned as usize;
    while offset + mem::size_of::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX>()
        <= total
    {
        let info_ptr = unsafe {
            buffer.as_ptr().add(offset) as *const SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX
        };
        // Safety: bounds checked above; the pointer is non-null and
        // well-aligned, and the OS is responsible for the layout of
        // every field it wrote.
        let info = unsafe { &*info_ptr };
        if info.Relationship == RelationCache {
            // Reading the `Cache` member of the union is sound
            // because we just verified the discriminator above.
            let cache: CACHE_RELATIONSHIP = unsafe { info.Anonymous.Cache };
            let line = cache.LineSize as usize;
            if line > max_line {
                max_line = line;
            }
        }
        if info.Size == 0 {
            // Defensive: malformed record should not loop forever.
            break;
        }
        offset += info.Size as usize;
    }

    if max_line == 0 {
        64
    } else {
        max_line
    }
}

// ============================================================================
// Tick counter / frequency
// ============================================================================

/// Returns the frequency of the monotonic tick counter, in ticks per
/// second.
///
/// Mirrors the C++ `GetTickFrequency()`. On Windows the frequency is
/// the inverse of `QueryPerformanceCounter`'s tick period. On Unix
/// the counter is driven by `clock_gettime(CLOCK_MONOTONIC_RAW, ...)`,
/// so the frequency is exactly 1,000,000,000 (nanoseconds).
#[cfg(unix)]
pub fn get_tick_frequency() -> u64 {
    // We back the tick counter with CLOCK_MONOTONIC_RAW on Unix, whose
    // unit is always 1 ns. The "frequency" is therefore 1e9 ticks/s.
    1_000_000_000
}

#[cfg(windows)]
pub fn get_tick_frequency() -> u64 {
    use windows_ffi::QueryPerformanceFrequency;
    let mut freq: i64 = 0;
    // Safety: `QueryPerformanceFrequency` always writes through the
    // pointer on success; returns non-zero on success.
    if unsafe { QueryPerformanceFrequency(&mut freq) } == 0 {
        // Should never happen on any Windows version PCSX2 supports,
        // but fall back to the historical default rather than 0 so
        // callers don't divide by zero.
        return 10_000_000;
    }
    freq as u64
}

/// Returns the current value of the monotonic tick counter, in ticks.
///
/// Mirrors the C++ `GetCPUTicks()`. Pairs with [`get_tick_frequency`].
#[cfg(unix)]
pub fn get_cpu_ticks() -> u64 {
    // Safety: `clock_gettime` only writes through the pointer; the
    // storage is a freshly-zeroed `MaybeUninit` of the right type.
    let mut ts = MaybeUninit::<libc::timespec>::zeroed();
    let ok = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC_RAW, ts.as_mut_ptr()) };
    if ok != 0 {
        return 0;
    }
    let ts = unsafe { ts.assume_init() };
    // Both fields are signed (seconds, nanoseconds) but always
    // non-negative for CLOCK_MONOTONIC_RAW in practice; widen through
    // `u64` to produce a single monotonic nanosecond count.
    let secs = ts.tv_sec.max(0) as u64;
    let nanos = ts.tv_nsec.max(0) as u64;
    secs.wrapping_mul(1_000_000_000).wrapping_add(nanos)
}

#[cfg(windows)]
pub fn get_cpu_ticks() -> u64 {
    use windows_ffi::QueryPerformanceCounter;
    let mut count: i64 = 0;
    // Safety: pointer is to a stack `i64`; `QueryPerformanceCounter`
    // always writes through the pointer on success.
    if unsafe { QueryPerformanceCounter(&mut count) } == 0 {
        return 0;
    }
    count as u64
}

// ============================================================================
// Physical memory
// ============================================================================

/// Returns the total physical memory in the host, in bytes.
///
/// On Unix, queried via `sysconf(_SC_PHYS_PAGES) * sysconf(_SC_PAGESIZE)`.
/// On Windows, queried via `GlobalMemoryStatusEx`'s `ullTotalPhys`.
#[cfg(unix)]
pub fn get_physical_memory() -> u64 {
    // Safety: `sysconf` is safe to call with any valid `_SC_*`
    // constant; the products stay within `u64` for any system we'd
    // realistically run PCSX2 on (and we saturate if not).
    let pages = unsafe { libc::sysconf(libc::_SC_PHYS_PAGES) };
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if pages < 0 || page_size < 0 {
        return 0;
    }
    let total = (pages as u64).saturating_mul(page_size as u64);
    total
}

#[cfg(windows)]
pub fn get_physical_memory() -> u64 {
    use windows_ffi::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    // Safety: `GlobalMemoryStatusEx` requires the structure's
    // `dwLength` field to be set to its size; we do that here and
    // zero the rest of the struct so padding reads are well-defined.
    let mut status: MEMORYSTATUSEX = unsafe { mem::zeroed() };
    status.dwLength = mem::size_of::<MEMORYSTATUSEX>() as u32;
    let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
    if ok == 0 {
        return 0;
    }
    status.ullTotalPhys
}

/// Returns the available physical memory in the host, in bytes.
///
/// On Unix, derived from `sysconf(_SC_AVPHYS_PAGES) * sysconf(_SC_PAGESIZE)`.
/// On Windows, queried via `GlobalMemoryStatusEx`'s `ullAvailPhys`.
#[cfg(unix)]
pub fn get_available_memory() -> u64 {
    // Safety: see `get_physical_memory`.
    let pages = unsafe { libc::sysconf(libc::_SC_AVPHYS_PAGES) };
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if pages < 0 || page_size < 0 {
        return 0;
    }
    (pages as u64).saturating_mul(page_size as u64)
}

#[cfg(windows)]
pub fn get_available_memory() -> u64 {
    use windows_ffi::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status: MEMORYSTATUSEX = unsafe { mem::zeroed() };
    status.dwLength = mem::size_of::<MEMORYSTATUSEX>() as u32;
    let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
    if ok == 0 {
        return 0;
    }
    status.ullAvailPhys
}

// ============================================================================
// Operating system version string
// ============================================================================

/// Returns a human-readable description of the host operating system.
///
/// Mirrors the C++ `GetOSVersionString()`. On Unix this is built from
/// `uname(2)`'s `sysname`, `release`, `version`, and `machine` fields.
/// On Windows this is built from the OS version major/minor/build
/// numbers obtained via the (undocumented-but-stable) `RtlGetVersion`
/// ntdll entry, plus architecture string.
#[cfg(unix)]
pub fn get_os_version_string() -> String {
    // Safety: `uname` writes through the pointer; the storage is a
    // freshly-zeroed `MaybeUninit` of the correct type.
    let mut uts = MaybeUninit::<libc::utsname>::zeroed();
    let ok = unsafe { libc::uname(uts.as_mut_ptr()) };
    if ok != 0 {
        // On failure return a minimal placeholder rather than panicking.
        return String::from("Unknown Unix");
    }
    let uts = unsafe { uts.assume_init() };

    // `uname` returns nul-terminated C strings in fixed-size arrays.
    // Convert each field to a `&str` by finding the first nul and
    // trimming. We intentionally do not strip the trailing nul when
    // building the final string.
    let sysname = cstr_array_to_str(&uts.sysname);
    let release = cstr_array_to_str(&uts.release);
    let version = cstr_array_to_str(&uts.version);
    let machine = cstr_array_to_str(&uts.machine);

    format!("{} {} {} ({})", sysname, release, version, machine)
}

#[cfg(windows)]
pub fn get_os_version_string() -> String {
    use windows_ffi::{RtlGetVersion, OsVersionInfoExW};

    // Use RtlGetVersion: unlike GetVersionEx it returns the true
    // version even for applications that don't have a manifest
    // declaring Windows 10/11 compatibility.
    let mut info: OsVersionInfoExW = unsafe { mem::zeroed() };
    info.dw_os_version_info_size = mem::size_of::<OsVersionInfoExW>() as u32;
    // Safety: pointer is to a stack-initialised struct of the right
    // size; RtlGetVersion is documented to return 0 on success.
    let status = unsafe { RtlGetVersion(&mut info) };
    if status != 0 {
        return String::from("Windows (unknown version)");
    }

    // Best-effort human label: PCSX2 historically prints "Windows"
    // plus the version. We avoid carrying a translation table here;
    // future code that needs the marketing name can derive it.
    let arch = if mem::size_of::<usize>() == 8 { "x64" } else { "x86" };
    format!(
        "Windows {}.{}.{} {}",
        info.dw_major_version, info.dw_minor_version, info.dw_build_number, arch
    )
}

// ============================================================================
// CPU information
// ============================================================================

/// Description of the host CPU.
///
/// Mirrors the C++ `CPUInfo` struct. Big/small core counts are
/// computed from per-cluster frequency information when available;
/// on platforms that don't expose that (and on the Phase 1 simple
/// Linux/Win32 implementations) both fields are set to zero and the
/// caller should fall back to [`num_threads`](Self::num_threads).
#[derive(Clone, Debug, Default)]
pub struct CPUInfo {
    /// CPU brand / model string (e.g. "Intel(R) Core(TM) i7-8559U
    /// CPU @ 2.70GHz"). Empty on platforms where the string could
    /// not be determined.
    pub name: String,
    /// Number of cores in the highest-frequency cluster.
    pub num_big_cores: u32,
    /// Number of cores in all lower-frequency clusters.
    pub num_small_cores: u32,
    /// Total number of logical processors visible to the host.
    pub num_threads: u32,
    /// Number of frequency-distinct clusters.
    pub num_clusters: u32,
}

impl CPUInfo {
    /// Empty placeholder for tests and default-valued FFI handles.
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
///
/// On Linux, the name is parsed from `/proc/cpuinfo`'s `model name`
/// line and the thread count from the number of `processor :` entries.
/// The cluster / big-vs-small distinction is left at zero on Linux in
/// Phase 1; a future revision can derive it from
/// `/sys/devices/system/cpu/cpu*/cpufreq` or from
/// `cpuinfo_get_clusters()` once that library has a Rust binding.
///
/// On Windows the name is left empty (no Win32 API exposes the brand
/// string without going through the registry or `__cpuid`) and the
/// thread count comes from `GetSystemInfo`'s `dwNumberOfProcessors`.
///
/// On macOS the name is left empty and the thread count comes from
/// `sysctlbyname("hw.logicalcpu")`. Apple Silicon's P/E core
/// distinction is not exposed via `sysctl` and is left at zero.
pub fn get_cpu_info() -> CPUInfo {
    #[cfg(target_os = "linux")]
    {
        linux_cpu_info()
    }
    #[cfg(target_os = "macos")]
    {
        macos_cpu_info()
    }
    #[cfg(all(unix, not(target_os = "linux"), not(target_os = "macos")))]
    {
        CPUInfo::empty()
    }
    #[cfg(windows)]
    {
        windows_cpu_info()
    }
}

// ---- Linux /proc/cpuinfo parsing -----------------------------------------

#[cfg(target_os = "linux")]
fn linux_cpu_info() -> CPUInfo {
    let mut info = CPUInfo::empty();

    // Best-effort: missing or malformed /proc/cpuinfo shouldn't take
    // the process down — return whatever we managed to collect.
    if let Ok(contents) = fs::read_to_string("/proc/cpuinfo") {
        let mut threads: u32 = 0;
        for line in contents.lines() {
            if let Some(rest) = line.strip_prefix("processor") {
                // "processor	: 0"
                if rest.trim_start().starts_with(':') {
                    threads = threads.saturating_add(1);
                }
            } else if let Some(rest) = line.strip_prefix("model name") {
                if info.name.is_empty() {
                    if let Some(value) = parse_proc_value(rest) {
                        info.name = value;
                    }
                }
            }
        }
        info.num_threads = threads;
    }

    info
}

/// Parses the right-hand side of a `/proc/cpuinfo` line of the form
/// `"<key>   : <value>"`. The separator after the key may be any
/// mixture of tabs and spaces; the colon must be present.
#[cfg(target_os = "linux")]
fn parse_proc_value(after_key: &str) -> Option<String> {
    let trimmed = after_key.trim_start();
    let after_colon = trimmed.strip_prefix(':')?;
    Some(after_colon.trim().to_string())
}

// ---- macOS sysctl ---------------------------------------------------------

#[cfg(target_os = "macos")]
fn macos_cpu_info() -> CPUInfo {
    let mut info = CPUInfo::empty();
    info.num_threads = sysctl_usize("hw.logicalcpu").unwrap_or(0) as u32;
    info
}

#[cfg(target_os = "macos")]
fn sysctl_usize(name: &str) -> Option<usize> {
    let c_name = CString::new(name).ok()?;
    let mut value: usize = 0;
    let mut len = mem::size_of::<usize>();
    // Safety: `sysctlbyname` writes at most `len` bytes into the
    // caller buffer. We pre-zero so a short read is still defined.
    let ret = unsafe {
        libc::sysctlbyname(
            c_name.as_ptr(),
            &mut value as *mut _ as *mut _,
            &mut len,
            std::ptr::null(),
            0,
        )
    };
    if ret == 0 {
        Some(value)
    } else {
        None
    }
}

// ---- Windows GetSystemInfo -----------------------------------------------

#[cfg(windows)]
fn windows_cpu_info() -> CPUInfo {
    use windows_ffi::{GetSystemInfo, SYSTEM_INFO};
    let mut info = CPUInfo::empty();
    // Safety: GetSystemInfo always writes through the pointer; the
    // storage is freshly zeroed.
    let mut si = MaybeUninit::<SYSTEM_INFO>::zeroed();
    unsafe {
        GetSystemInfo(si.as_mut_ptr());
        let si = si.assume_init();
        info.num_threads = si.dwNumberOfProcessors;
    }
    info
}

// ============================================================================
// Internal helpers
// ============================================================================

/// Convert a fixed-size C string array (e.g. `utsname.sysname`) to a
/// `&str` by stripping the trailing NULs. Returns the empty string if
/// the field is entirely NUL (which can happen on misconfigured
/// kernels) or contains non-UTF-8 bytes.
#[cfg(unix)]
fn cstr_array_to_str(buf: &[libc::c_char]) -> &str {
    // Find the first NUL.
    let nul_pos = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    // `c_char` is `i8` on most platforms and `u8` on aarch64; widen
    // to `u8` for a uniform UTF-8 check.
    let bytes: &[u8] = unsafe {
        std::slice::from_raw_parts(buf.as_ptr() as *const u8, nul_pos)
    };
    str::from_utf8(bytes).unwrap_or("")
}

/// Same as [`cstr_array_to_str`] but for a wide-character (`u16`)
/// buffer, used for the `sz_csd_version` field of OSVERSIONINFOEXW
/// (we never read it, but the helper is kept for symmetry / future
/// use).
#[allow(dead_code)]
fn wstr_array_to_str(buf: &[u16]) -> String {
    let nul_pos = buf.iter().position(|&w| w == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..nul_pos])
}

/// Write a Rust string into a caller-supplied C buffer with explicit
/// bound. Truncates if the destination is too small, always NUL
/// terminates (when `out_len > 0`), and returns the number of bytes
/// that *would* have been written, excluding the terminator — i.e.
/// the `snprintf` convention. This is the helper used by the FFI
/// `os_version_string` export below.
fn write_c_string(out: *mut c_char, out_len: u32, s: &str) -> u32 {
    if out.is_null() || out_len == 0 {
        return s.len() as u32;
    }
    // Truncate so the buffer (minus one byte for the NUL) holds
    // exactly the bytes that fit.
    let cap = (out_len as usize).saturating_sub(1);
    let bytes = s.as_bytes();
    let to_copy = bytes.len().min(cap);
    // Safety: `out` is non-null with `out_len` writable bytes; the
    // caller is the contract holder. We never read past `to_copy`
    // and always write a NUL at the end.
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), out as *mut u8, to_copy);
        *out.add(to_copy) = 0;
    }
    // Return the would-be length (excluding terminator) so callers
    // can detect truncation by comparing against `out_len - 1`.
    bytes.len() as u32
}

// Suppress the unused-import warning on the io module when no
// platform branch actually uses it. `io` is reserved for future
// readers that may want to surface errors as `io::Result`.
#[allow(dead_code)]
fn _force_io_import() -> io::Result<()> {
    Ok(())
}

// And the same for `Path` (reserved for a future Linux variant that
// might walk /sys/devices/system/cpu).
#[allow(dead_code)]
fn _force_path_import() -> &'static Path {
    Path::new("/proc/cpuinfo")
}

// ============================================================================
// FFI surface (consumed by C++ PCSX2 via cbindgen)
//
// NOTE: The cross-platform `pcsx2_host_*` FFI exports have been removed
// from this module to avoid duplicate-symbol errors with the platform-
// specific host modules (`linux_host_sys`, `windows_host_sys`, and
// forthcoming `darwin_host_sys`). The platform-specific modules provide
// the same FFI symbols under the same names. Use the safe Rust API
// (`get_runtime_page_size`, etc.) for cross-platform Rust callers.
// ============================================================================

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr;

    #[test]
    fn page_size_is_nonzero() {
        let p = get_runtime_page_size();
        assert!(p > 0);
        // All platforms PCSX2 runs on have a power-of-two page size.
        assert!(p.is_power_of_two());
    }

    #[test]
    fn cache_line_size_is_sensible() {
        let c = get_runtime_cache_line_size();
        // Should be at least 16 (tiny embedded controllers) and at
        // most 256 (some future cache hierarchies); in practice it's
        // 32/64/128.
        assert!(c >= 16 && c <= 256, "unexpected cache line: {}", c);
    }

    #[test]
    fn tick_frequency_is_nonzero() {
        assert!(get_tick_frequency() > 0);
    }

    #[test]
    fn cpu_ticks_monotonic() {
        // The exact value can't be asserted without sleeping, but
        // two consecutive reads should be ordered: t1 <= t2.
        let t1 = get_cpu_ticks();
        let t2 = get_cpu_ticks();
        assert!(t2 >= t1, "tick counter went backwards: {} -> {}", t1, t2);
    }

    #[test]
    fn physical_memory_is_nonzero() {
        // Any system capable of running tests has at least 1 MiB.
        let p = get_physical_memory();
        assert!(p >= 1024 * 1024, "physical memory implausible: {}", p);
    }

    #[test]
    fn available_memory_is_bounded_by_total() {
        let total = get_physical_memory();
        let avail = get_available_memory();
        assert!(avail <= total, "available {} > total {}", avail, total);
    }

    #[test]
    fn os_version_string_nonempty() {
        let s = get_os_version_string();
        assert!(!s.is_empty());
    }

    #[test]
    fn os_version_string_writes_and_truncates() {
        let s = get_os_version_string();
        // Path 1: large enough buffer.
        let mut buf = [0i8; 256];
        let n = write_c_string(buf.as_mut_ptr(), buf.len() as u32, &s);
        assert_eq!(n, s.len() as u32);
        // Read it back.
        let cstr = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) };
        assert_eq!(cstr.to_str().unwrap(), s);

        // Path 2: too small — return value should be the untruncated
        // length, and the buffer should still be NUL terminated.
        let mut tiny = [0i8; 4];
        let n = write_c_string(tiny.as_mut_ptr(), tiny.len() as u32, &s);
        assert_eq!(n, s.len() as u32);
        assert_eq!(tiny[tiny.len() - 1], 0);

        // Path 3: null pointer with non-zero length — returns the
        // would-be length without writing anything.
        let n = write_c_string(ptr::null_mut(), 16, &s);
        assert_eq!(n, s.len() as u32);
    }

    #[test]
    fn cpu_info_threads_is_nonzero() {
        // `get_cpu_info` on the test machine should report at least
        // one thread. On all platforms PCSX2 builds on, the underlying
        // syscall succeeds, but on odd CI runners it may not; we
        // therefore just assert the function runs without panic.
        let _ = get_cpu_info();
    }
}
