// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Host OS abstraction layer.
//!
//! This module mirrors PCSX2's `common/HostSys.{h,cpp}` pair. It is a thin
//! portability shim that exposes a small set of primitives the JIT and
//! memory-mapping code relies on:
//!
//! - Reporting the host's page size and cache-line size.
//! - Allocating executable memory regions for the recompiler.
//! - Changing the protection on existing pages (`mprotect` / `VirtualProtect`).
//! - Flushing the instruction cache after writing freshly generated code
//!   (a no-op on x86 because the D/I caches are coherent, mandatory on
//!   aarch64).
//! - A coarse `PageProtection` enum covering the four meaningful R/W/X
//!   combinations the rest of the engine cares about.
//!
//! The original C++ exposes a `PageProtectionMode` builder class with
//! `.Read()/.Write()/.Execute()/.All()` plus a handful of named factories
//! (`PageAccess_None`, `PageAccess_ReadOnly`, ...). In this Rust port we
//! collapse that to a flat `enum` because the C++ builder was effectively
//! constructing one of four states anyway; the callers in PCSX2 only ever
//! ask for `None`, `ReadOnly`, `ReadWrite`, or `ReadWriteExecute` once JIT
//! code is ready to run.
//!
//! All functions in this module are `unsafe` because they operate on raw
//! pointers and rely on the caller respecting the host page-size
//! requirements; see the per-function docs for the exact contracts.

use std::io;
use std::ptr::{self, NonNull};

// --------------------------------------------------------------------------------------
//  PageProtection
// --------------------------------------------------------------------------------------

/// Coarse-grained page protection modes the JIT and shared-memory code use.
///
/// `CanExecute` on the original `PageProtectionMode` class also requires
/// `CanRead`; the variants below follow that rule (`ReadExecute` and
/// `ReadWriteExecute` are readable+executable, the others are not
/// executable at all).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PageProtection {
    /// `R--` — readable, not writable, not executable.
    ReadOnly,
    /// `RW-` — readable and writable, not executable.
    ReadWrite,
    /// `R-E` — readable and executable, not writable.
    ReadExecute,
    /// `RWE` — readable, writable, and executable.
    ReadWriteExecute,
}

impl PageProtection {
    /// Returns `true` when the protection allows reads.
    #[inline]
    pub const fn can_read(self) -> bool {
        // Every variant in the enum is readable; this mirrors the C++
        // `PageProtectionMode::CanRead()`.
        matches!(
            self,
            Self::ReadOnly | Self::ReadWrite | Self::ReadExecute | Self::ReadWriteExecute
        )
    }

    /// Returns `true` when the protection allows writes.
    #[inline]
    pub const fn can_write(self) -> bool {
        matches!(self, Self::ReadWrite | Self::ReadWriteExecute)
    }

    /// Returns `true` when the protection allows execution. Mirrors the
    /// C++ rule that `CanExecute()` also requires `CanRead()`.
    #[inline]
    pub const fn can_execute(self) -> bool {
        matches!(self, Self::ReadExecute | Self::ReadWriteExecute)
    }
}

// --------------------------------------------------------------------------------------
//  Platform imports
// --------------------------------------------------------------------------------------

#[cfg(target_os = "windows")]
mod platform {
    use std::io;
    use std::ptr::NonNull;

    /// Windows page-protection bits used by `VirtualAlloc` / `VirtualProtect`.
    ///
    /// We inline only the constants and prototypes we actually need rather
    /// than pulling in the `windows-sys` / `winapi` crate, matching the
    /// "only `std` deps" rule.
    pub const PAGE_NOACCESS: u32 = 0x01;
    pub const PAGE_READONLY: u32 = 0x02;
    pub const PAGE_READWRITE: u32 = 0x04;
    pub const PAGE_EXECUTE_READ: u32 = 0x20;
    pub const PAGE_EXECUTE_READWRITE: u32 = 0x40;

    pub const MEM_COMMIT: u32 = 0x1000;
    pub const MEM_RESERVE: u32 = 0x2000;
    pub const MEM_RELEASE: u32 = 0x8000;

    #[link(name = "kernel32")]
    extern "system" {
        pub fn VirtualAlloc(
            lpAddress: *mut core::ffi::c_void,
            dwSize: usize,
            flAllocationType: u32,
            flProtect: u32,
        ) -> *mut core::ffi::c_void;

        pub fn VirtualFree(
            lpAddress: *mut core::ffi::c_void,
            dwSize: usize,
            dwFreeType: u32,
        ) -> i32;

        pub fn VirtualProtect(
            lpAddress: *mut core::ffi::c_void,
            dwSize: usize,
            flNewProtect: u32,
            lpflOldProtect: *mut u32,
        ) -> i32;

        pub fn GetSystemInfo(lpSystemInfo: *mut SystemInfo);

        pub fn FlushInstructionCache(
            hProcess: *mut core::ffi::c_void,
            lpBaseAddress: *mut core::ffi::c_void,
            dwSize: usize,
        ) -> i32;
    }

    /// Minimal stand-in for the Win32 `SYSTEM_INFO` struct, just enough to
    /// read the page and cache-line sizes.
    #[repr(C)]
    pub struct SystemInfo {
        pub dwOemId: u32,
        pub dwPageSize: u32,
        pub lpMinimumApplicationAddress: *mut core::ffi::c_void,
        pub lpMaximumApplicationAddress: *mut core::ffi::c_void,
        pub dwActiveProcessorMask: usize,
        pub dwNumberOfProcessors: u32,
        pub dwProcessorType: u32,
        pub dwAllocationGranularity: u32,
        pub wProcessorLevel: u16,
        pub wProcessorRevision: u16,
    }

    /// Convert a `PageProtection` to a Win32 PAGE_* constant.
    pub const fn to_win32_prot(prot: super::PageProtection) -> u32 {
        match prot {
            super::PageProtection::ReadOnly => PAGE_READONLY,
            super::PageProtection::ReadWrite => PAGE_READWRITE,
            super::PageProtection::ReadExecute => PAGE_EXECUTE_READ,
            super::PageProtection::ReadWriteExecute => PAGE_EXECUTE_READWRITE,
        }
    }

    /// Best-effort mapping of a Win32 error code to an `io::Error`.
    pub fn last_error() -> io::Error {
        // We deliberately do not call `GetLastError` here; the caller
        // has the relevant context and most paths already have a
        // sensible `io::ErrorKind`. Keeping this helper minimal lets
        // the platform layer stay free of `windows-sys`.
        io::Error::last_os_error()
    }

    /// Ensure that a non-null, non-NULL pointer is returned or an error is built.
    pub fn ptr_or_err(p: *mut u8) -> io::Result<NonNull<u8>> {
        match NonNull::new(p) {
            Some(nn) => Ok(nn),
            None => Err(last_error()),
        }
    }
}

#[cfg(unix)]
mod platform {
    use std::io;
    use std::ptr::NonNull;

    /// POSIX `mprotect` protection bits. Inline the few values we need.
    pub const PROT_NONE: i32 = 0x0;
    pub const PROT_READ: i32 = 0x1;
    pub const PROT_WRITE: i32 = 0x2;
    pub const PROT_EXEC: i32 = 0x4;

    /// POSIX `mmap` flags. We only need `MAP_ANONYMOUS` + `MAP_PRIVATE`
    /// for anonymous private mappings, which is what the JIT code
    /// allocator wants on Linux/macOS.
    pub const MAP_ANONYMOUS: i32 = 0x20;
    pub const MAP_PRIVATE: i32 = 0x02;

    extern "C" {
        pub fn mmap(
            addr: *mut core::ffi::c_void,
            length: usize,
            prot: i32,
            flags: i32,
            fd: i32,
            offset: i64,
        ) -> *mut core::ffi::c_void;

        pub fn munmap(addr: *mut core::ffi::c_void, length: usize) -> i32;

        pub fn mprotect(
            addr: *mut core::ffi::c_void,
            len: usize,
            prot: i32,
        ) -> i32;

        pub fn sysconf(name: i32) -> i64;
    }

    /// `_SC_PAGESIZE` for `sysconf`.
    pub const SC_PAGESIZE: i32 = 30;
    /// `_SC_LEVEL1_DCACHE_LINESIZE` — used to read the cache-line size.
    /// Returns `-1` on platforms that do not define it (older glibc, etc.).
    pub const SC_LEVEL1_DCACHE_LINESIZE: i32 = 169;

    /// Convert a `PageProtection` to a POSIX `PROT_*` bitmask.
    pub const fn to_posix_prot(prot: super::PageProtection) -> i32 {
        let mut bits = PROT_READ;
        if prot.can_write() {
            bits |= PROT_WRITE;
        }
        if prot.can_execute() {
            bits |= PROT_EXEC;
        }
        bits
    }

    pub fn ptr_or_err(p: *mut u8) -> io::Result<NonNull<u8>> {
        match NonNull::new(p) {
            Some(nn) => Ok(nn),
            None => Err(io::Error::last_os_error()),
        }
    }
}

// --------------------------------------------------------------------------------------
//  Public API
// --------------------------------------------------------------------------------------

/// Returns the host's page size in bytes, as reported by the OS.
///
/// On Windows this is `GetSystemInfo::dwPageSize`; on POSIX platforms
/// this is `sysconf(_SC_PAGESIZE)`. PCSX2 treats this as a constant at
/// runtime — the C++ version is named `GetRuntimePageSize` and the
/// caller caches the result.
pub unsafe fn os_page_size() -> usize {
    #[cfg(target_os = "windows")]
    {
        let mut info = core::mem::MaybeUninit::<platform::SystemInfo>::uninit();
        platform::GetSystemInfo(info.as_mut_ptr());
        let info = info.assume_init();
        info.dwPageSize as usize
    }
    #[cfg(unix)]
    {
        let p = platform::sysconf(platform::SC_PAGESIZE);
        if p <= 0 {
            // The C++ code asserts here, but in this port we degrade
            // to a 4 KiB default so callers can still recover.
            4096
        } else {
            p as usize
        }
    }
}

/// Returns the host's L1 data-cache line size in bytes.
///
/// On Windows we approximate it with the page size (Win32 does not
/// expose a portable cache-line query without going through
/// `GetLogicalProcessorInformationEx`); on POSIX we use
/// `sysconf(_SC_LEVEL1_DCACHE_LINESIZE)`, falling back to 64 bytes.
pub unsafe fn cache_line_size() -> usize {
    #[cfg(target_os = "windows")]
    {
        // No portable Win32 API for this. The C++ version returns 0
        // for Windows, and callers tolerate that. We use the page size
        // as a conservative upper bound.
        os_page_size()
    }
    #[cfg(unix)]
    {
        let c = platform::sysconf(platform::SC_LEVEL1_DCACHE_LINESIZE);
        if c <= 0 {
            64
        } else {
            c as usize
        }
    }
}

/// Allocates a region of `size` bytes of memory that is suitable for
/// storing JIT-emitted code. The returned region is initially readable
/// and writable; use [`set_page_protection`] to flip it to executable
/// once the code is in place.
///
/// On Windows this calls `VirtualAlloc(MEM_COMMIT | MEM_RESERVE,
/// PAGE_READWRITE)`. On POSIX this calls `mmap(NULL, size,
/// PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0)`. The
/// `size` is rounded up to a page boundary by the OS in both cases.
///
/// Returns `None` if the OS allocator failed; the caller is responsible
/// for deciding whether to retry, fall back to a smaller size, or
/// abort.
pub unsafe fn allocate_code_space(size: usize) -> Option<NonNull<u8>> {
    #[cfg(target_os = "windows")]
    {
        let p = platform::VirtualAlloc(
            ptr::null_mut(),
            size,
            platform::MEM_COMMIT | platform::MEM_RESERVE,
            platform::PAGE_READWRITE,
        );
        platform::ptr_or_err(p as *mut u8).ok()
    }
    #[cfg(unix)]
    {
        let p = platform::mmap(
            ptr::null_mut(),
            size,
            platform::PROT_READ | platform::PROT_WRITE,
            platform::MAP_PRIVATE | platform::MAP_ANONYMOUS,
            -1,
            0,
        );
        if p == platform::MAP_FAILED as *mut _ {
            None
        } else {
            NonNull::new(p as *mut u8)
        }
    }
}

/// Releases a region previously obtained from [`allocate_code_space`].
///
/// On Windows the entire region is released with `VirtualFree`
/// (`MEM_RELEASE`, which requires `dwSize == 0`). On POSIX the matching
/// `munmap` is invoked with the same size the allocation used.
///
/// The caller is responsible for ensuring that `ptr` was produced by
/// `allocate_code_space` with the same `size`, and that no other
/// thread is using the region at the time of the call.
pub unsafe fn release_code_space(ptr: NonNull<u8>, size: usize) -> io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        let ok = platform::VirtualFree(ptr.as_ptr() as *mut _, 0, platform::MEM_RELEASE);
        if ok == 0 {
            Err(platform::last_error())
        } else {
            Ok(())
        }
    }
    #[cfg(unix)]
    {
        let rc = platform::munmap(ptr.as_ptr() as *mut _, size);
        if rc != 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

/// Changes the protection on `[addr, addr + size)` to `prot`.
///
/// Both `addr` and `size` must be aligned to the host page size. On
/// Windows this calls `VirtualProtect`; on POSIX it calls `mprotect`.
/// Returns an `io::Error` describing the OS error on failure.
pub unsafe fn set_page_protection(
    addr: *mut u8,
    size: usize,
    prot: PageProtection,
) -> io::Result<()> {
    if addr.is_null() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "set_page_protection: null address",
        ));
    }
    if size == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "set_page_protection: zero size",
        ));
    }

    #[cfg(target_os = "windows")]
    {
        let mut _old: u32 = 0;
        let ok = platform::VirtualProtect(
            addr as *mut _,
            size,
            platform::to_win32_prot(prot),
            &mut _old,
        );
        if ok == 0 {
            Err(platform::last_error())
        } else {
            Ok(())
        }
    }
    #[cfg(unix)]
    {
        let rc = platform::mprotect(addr as *mut _, size, platform::to_posix_prot(prot));
        if rc != 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

/// Flushes the instruction cache for the given range.
///
/// On x86 the data and instruction caches are coherent, so this is a
/// no-op there (the C++ version compiles to an empty inline function
/// under `ARCH_X86`). On aarch64 we must explicitly invalidate the
/// affected lines; the standard idiom is `__clear_cache`, which is
/// provided by the MSVC CRT on Windows and by libgcc/compiler-rt on
/// GCC/Clang. On other targets we conservatively emit a memory barrier
/// via `compiler_fence`; that is not strictly correct for arbitrary
/// aarch64 toolchains but matches what the original C++ does for
/// "everything that is not x86 and not the platforms we explicitly
/// handle" (the file uses `#error Unknown architecture` for those, so
/// the Rust port likewise restricts itself to the two architectures
/// the C++ supports).
pub unsafe fn flush_icache(addr: *mut u8, size: usize) {
    if addr.is_null() || size == 0 {
        return;
    }

    #[cfg(target_os = "windows")]
    {
        // FlushInstructionCache on the current process. The first
        // argument is `HANDLE`; `-1` is the pseudo-handle for the
        // current process.
        platform::FlushInstructionCache(
            -1isize as *mut _,
            addr as *mut _,
            size,
        );
        return;
    }

    #[cfg(all(unix, target_arch = "aarch64"))]
    {
        // libgcc / compiler-rt provides `__clear_cache` on aarch64
        // Linux. Some toolchains expose it under a slightly different
        // name, so we declare it here and let the linker resolve it.
        extern "C" {
            fn __clear_cache(begin: *mut u8, end: *mut u8);
        }
        let end = addr.add(size);
        __clear_cache(addr, end);
        return;
    }

    #[cfg(all(unix, target_arch = "x86_64"))]
    {
        // x86 has coherent D/I caches; nothing to do.
        let _ = (addr, size);
        return;
    }

    #[cfg(all(unix, not(any(target_arch = "aarch64", target_arch = "x86_64"))))]
    {
        // Other architectures: best-effort compiler fence so the
        // optimizer does not reorder the writes that preceded this
        // call past it. The C++ uses `#error Unknown architecture.`
        // for this case; we compile but emit a no-op fence so the
        // rest of the port can still be exercised.
        core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
    }
}
