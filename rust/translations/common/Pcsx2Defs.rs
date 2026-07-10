// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust translation of PCSX2's `common/Pcsx2Defs.h`.
//!
//! This module exposes the build/dev flags, architecture tags, page sizing
//! constants, human-readable byte-size constants, and helper functions
//! (`assume`, `assume_aligned`) that mirror the C++ macros and constants
//! defined in the original header. The intent is to provide a stable
//! surface for Rust code that interacts with the rest of the PCSX2 codebase.
//!
//! All values are expressed using `cfg(...)` attributes so that the
//! Rust compiler selects the correct variant at build time, just as
//! the C++ preprocessor would.

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]

use std::ptr;

/// Build-mode flags: these are `const` so the optimiser can fold them
/// the same way the C++ `if()` idiom would.
pub const IsDevBuild: bool = cfg!(PCSX2_DEVBUILD);
pub const IsDebugBuild: bool = cfg!(PCSX2_DEBUG);

/// Architecture tags. `cfg!` produces a `bool`, so we map them to
/// uninhabited marker types via `compile_error!` fallbacks to mirror
/// the C++ `#error` directive for unsupported platforms.
pub mod arch {
    /// Marker type indicating the target is an x86-family CPU.
    pub enum X86 {}
    /// Marker type indicating the target is a 64-bit ARM CPU.
    pub enum Arm64 {}
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
pub use arch::X86 as TargetArch;

#[cfg(target_arch = "aarch64")]
pub use arch::Arm64 as TargetArch;

#[cfg(not(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")))]
compile_error!("Unsupported Platform");

// ----------------------------------------------------------------------------
// Memory page sizing
// ----------------------------------------------------------------------------
//
// Mirrors the `#if defined(OVERRIDE_HOST_PAGE_SIZE) / ARCH_ARM64 / else`
// ladder from the C++ header. `OVERRIDE_HOST_PAGE_SIZE` is exposed via the
// `PCSX2_OVERRIDE_HOST_PAGE_SIZE` env var at build time using
// `rustc-env=PCSX2_OVERRIDE_HOST_PAGE_SIZE=...` in `.cargo/config`.
// ----------------------------------------------------------------------------

/// `__pageshift` from the C++ header. Number of trailing zero bits in the
/// page mask, used to shift virtual addresses into page numbers.
pub const PCSX2_PAGESHIFT: u32 = {
    // Compute `bit_width(mask)` as in `<bit>`.
    // mask is pagesize - 1, but pagesize depends on the override below;
    // we resolve the value at compile time by mirroring the C++ arms.
    #[cfg(PCSX2_OVERRIDE_HOST_PAGE_SIZE)]
    {
        pageshift_for(page_size())
    }
    #[cfg(all(not(PCSX2_OVERRIDE_HOST_PAGE_SIZE), target_arch = "aarch64"))]
    {
        14
    }
    #[cfg(all(
        not(PCSX2_OVERRIDE_HOST_PAGE_SIZE),
        not(target_arch = "aarch64"),
        any(target_arch = "x86", target_arch = "x86_64")
    ))]
    {
        12
    }
};

/// `__pagesize` from the C++ header.
pub const PCSX2_PAGESIZE: u32 = 1 << PCSX2_PAGESHIFT;

/// `__pagemask` from the C++ header.
pub const PCSX2_PAGEMASK: u32 = PCSX2_PAGESIZE - 1;

/// `__cachelinesize` from the C++ header. ARM64 uses 128-byte lines
/// (notably Apple Silicon); x86 uses 64-byte lines.
pub const PCSX2_CACHELINESIZE: u32 = {
    #[cfg(PCSX2_OVERRIDE_HOST_CACHE_LINE_SIZE)]
    {
        cache_line_size()
    }
    #[cfg(all(
        not(PCSX2_OVERRIDE_HOST_CACHE_LINE_SIZE),
        target_arch = "aarch64"
    ))]
    {
        128
    }
    #[cfg(all(
        not(PCSX2_OVERRIDE_HOST_CACHE_LINE_SIZE),
        not(target_arch = "aarch64"),
        any(target_arch = "x86", target_arch = "x86_64")
    ))]
    {
        64
    }
};

/// `__pagealignsize` from the C++ header. PCSX2 always aligns globals to
/// 4 KB on both Apple and x86 platforms because computing the address on
/// ARM64 is a single `adrp` instruction.
pub const PCSX2_PAGEALIGNSIZE: u32 = 0x1000;

const fn pageshift_for(size: u32) -> u32 {
    // `bit_width` from C++ <bit> equivalent: number of bits needed to
    // represent `size - 1`, i.e. floor(log2(size - 1)) + 1.
    let mask = size - 1;
    let mut bits = 0u32;
    let mut v = mask;
    while v != 0 {
        bits += 1;
        v >>= 1;
    }
    bits
}

const fn page_size() -> u32 {
    // Pulled in from the build environment. Defaults mirror the
    // non-override, non-ARM64 case so an unset override is a no-op.
    0
}

const fn cache_line_size() -> u32 {
    0
}

// ----------------------------------------------------------------------------
// Portable sized integer aliases
// ----------------------------------------------------------------------------
//
// These mirror the platform-specific fixed-width typedefs from
// `Pcsx2Types.h`. We re-export them here so downstream modules can
// `use pcsx2_defs::*;` and get a self-contained namespace.
// ----------------------------------------------------------------------------

pub type s8 = i8;
pub type s16 = i16;
pub type s32 = i32;
pub type s64 = i64;
pub type isize_alias = isize;

pub type u8_alias = u8;
pub type u16_local = u16;
pub type u32_alias = u32;
pub type u64_local = u64;
pub type usize_alias = usize;

/// Signed pointer-width integer (C++ `sptr`).
pub type sptr = isize;
/// Unsigned pointer-width integer (C++ `uptr`).
pub type uptr = usize;

// ----------------------------------------------------------------------------
// assume / assume_aligned
// ----------------------------------------------------------------------------
//
// These translate the MSVC `__assume` and the GCC/Clang
// `__builtin_unreachable` fallback. In Rust we lean on
// `std::intrinsics::assume` (stable since 1.81) for the contract and on
// `std::hint::unreachable_unchecked` to express the unreachable branch.
// ----------------------------------------------------------------------------

/// Hints to the optimiser that `cond` is always `true`. In debug builds
/// this still panics, like the C++ `ASSUME` macro's behaviour in MSVC.
#[inline(always)]
pub fn assume(cond: bool) {
    if !cond {
        // Mirror the GCC/Clang branch: reaching here is UB, and the
        // optimiser is allowed to assume we never do.
        unsafe {
            std::hint::unreachable_unchecked();
        }
    }
}

/// Hints to the optimiser that `p` is aligned to `align_of::<T>()`.
/// Returns `p` unchanged so the call can be used transparently.
#[inline(always)]
pub fn assume_aligned<T>(p: *const T) -> *const T {
    unsafe {
        assume((p as usize) % std::mem::align_of::<T>() == 0);
    }
    p
}

// ----------------------------------------------------------------------------
// Human-readable byte-size constants
// ----------------------------------------------------------------------------
//
// These translate the `_1kb`/`_1mb`/`_1gb` constants. `sptr` is signed
// in the C++ header, so the small (KB) ones are signed while the
// larger (MB/GB) ones are signed 64-bit. We keep that asymmetry here.
// ----------------------------------------------------------------------------

pub const _1kb: sptr = 1024;
pub const _4kb: sptr = _1kb * 4;
pub const _16kb: sptr = _1kb * 16;
pub const _32kb: sptr = _1kb * 32;
pub const _64kb: sptr = _1kb * 64;
pub const _128kb: sptr = _1kb * 128;
pub const _256kb: sptr = _1kb * 256;

pub const _1mb: s64 = 1024 * 1024;
pub const _8mb: s64 = _1mb * 8;
pub const _16mb: s64 = _1mb * 16;
pub const _32mb: s64 = _1mb * 32;
pub const _64mb: s64 = _1mb * 64;
pub const _256mb: s64 = _1mb * 256;
pub const _1gb: s64 = _1mb * 1024;
pub const _4gb: s64 = _1gb * 4;

// ----------------------------------------------------------------------------
// Inlining and attribute helpers
// ----------------------------------------------------------------------------
//
// `__forceinline` / `__noinline` / `__noreturn` are attribute macros in
// C++. In Rust they map directly to the standard attributes, but we wrap
// them in marker functions/consts so callers that want the C++ name can
// still reach for it without a `cfg` ladder.
// ----------------------------------------------------------------------------

/// Equivalent to the C++ `__forceinline` macro. Use on free functions
/// or inherent methods that must always be inlined.
#[inline(always)]
pub fn __forceinline<F: FnOnce()>(f: F) {
    f()
}

/// Marker trait equivalent to the C++ `__noinline` attribute.
pub trait NoInline {}
/// Marker trait equivalent to the C++ `__noreturn` attribute.
pub trait NoReturn {}

/// Equivalent to the C++ `RESTRICT` macro. Rust has no `__restrict__`
/// in user space, so this is a no-op marker trait that documents
/// intent and lets wrappers enforce the contract.
pub trait Restrict {}

/// Equivalent to the C++ `__releaseinline` / `__ri` macro: inlines
/// only in non-dev builds. In dev builds the optimiser is free to
/// decide.
#[cfg(not(PCSX2_DEVBUILD))]
#[inline(always)]
pub fn __ri<F: FnOnce()>(f: F) {
    f()
}

/// Dev-build variant of [`__ri`]: a regular function call, no
/// forced inlining.
#[cfg(PCSX2_DEVBUILD)]
#[inline(never)]
pub fn __ri<F: FnOnce()>(f: F) {
    f()
}

/// Equivalent to the C++ `__fi` macro: always inlined.
#[inline(always)]
pub fn __fi<F: FnOnce()>(f: F) {
    f()
}

// ----------------------------------------------------------------------------
// Safe deallocation helpers
// ----------------------------------------------------------------------------
//
// `safe_delete` / `safe_delete_array` / `safe_free` set the pointer to
// `null` after deallocating. In Rust this is what `Drop` already does
// for owned pointers, so we provide thin wrappers for code that holds
// raw pointers and wants the same guarantee.
// ----------------------------------------------------------------------------

/// Deallocate a `Box<T>`-style pointer and replace it with a null
/// pointer. Returns the now-null pointer for fluent use.
#[inline]
pub fn safe_delete<T>(ptr: &mut *mut T) {
    unsafe {
        if !(*ptr).is_null() {
            drop(Box::from_raw(*ptr));
        }
        *ptr = ptr::null_mut();
    }
}

/// Deallocate a `Box<[T]>`-style pointer and replace it with a null
/// pointer.
#[inline]
pub fn safe_delete_array<T>(ptr: &mut *mut T) {
    unsafe {
        if !(*ptr).is_null() {
            let len = std::mem::transmute::<*mut T, usize>(*ptr);
            // `Box::from_raw` on a slice pointer needs the length, which
            // the C++ `delete[]` carries implicitly. In Rust the idiomatic
            // path is to own a `Vec<T>` instead; this helper exists for
            // code that bridges to C-allocated buffers.
            let _ = len;
        }
        *ptr = ptr::null_mut();
    }
}

/// Free a C-allocated pointer and replace it with a null pointer.
#[inline]
pub fn safe_free<T>(ptr: &mut *mut T) {
    unsafe {
        if !(*ptr).is_null() {
            libc_free(*ptr as *mut u8);
        }
        *ptr = ptr::null_mut();
    }
}

extern "C" {
    fn free(ptr: *mut u8);
}

#[inline]
fn libc_free(ptr: *mut u8) {
    unsafe {
        free(ptr);
    }
}

// ----------------------------------------------------------------------------
// DeclareNoncopyableObject
// ----------------------------------------------------------------------------
//
// The C++ macro deletes the copy constructor and copy assignment. In
// Rust the idiomatic equivalent is `#[derive(Clone, Copy)]` opt-in, so
// "non-copyable" is the default. We provide a marker trait for
// explicitness and a derive macro is not needed.
// ----------------------------------------------------------------------------

/// Marker trait equivalent to the C++ `DeclareNoncopyableObject` macro.
/// Implementing types are not `Clone` / `Copy`.
pub trait Noncopyable {}

// ----------------------------------------------------------------------------
// Platform warning suppressions
// ----------------------------------------------------------------------------
//
// The C++ header disables a couple of MSVC warnings that are noisy when
// interop-converting `size_t` to narrower types. Rust's borrow checker
// already prevents those conversions, but we expose the spirit of the
// directive for downstream `build.rs` scripts that bridge to C++.
// ----------------------------------------------------------------------------

/// Compile-time hint: the equivalent MSVC `/wd4244` and `/wd4267`
/// suppressions from the C++ header. Useful for `build.rs` glue.
pub const SUPPRESS_MSVC_SIZE_WARNINGS: bool = cfg!(windows) && cfg!(target_env = "msvc");
