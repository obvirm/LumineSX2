// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Rust reimplementation of PCSX2's `common/Pcsx2Defs.h`.
//!
//! Mirrors the C++ header's compile-time constants and macro
//! definitions, exposing them as idiomatic Rust `pub const` values
//! alongside `#[no_mangle] pub static` FFI exports that the C++ side
//! can consume directly.
//!
//! Sections mirrored here:
//! - Build / debug flags (`IsDevBuild`, `IsDebugBuild`)
//! - Architecture detection (`ARCH_X86`, `ARCH_ARM64`)
//! - Memory-page / cache-line / page-alignment sizes
//! - Compiler intrinsic replacements (`__forceinline`, `__noinline`,
//!   `__noreturn`, `ASSUME`, `RESTRICT`, `__fi`, `__ri`)
//! - Human-readable byte-size constants (`_1kb` ... `_4gb`)
//!
//! Skipped on the Rust side (no equivalent needed):
//! - `safe_delete` / `safe_delete_array` / `safe_free` — Rust's `Drop`
//!   trait makes these unnecessary.
//! - `DeclareNoncopyableObject` — Rust's default move-only semantics
//!   give the same guarantee without a marker.
//!
//! The C++ side receives the FFI surface through cbindgen's auto-generated
//! `pcsx2_common_rs.h` header. See `cbindgen.toml` for the prefix /
//! export configuration.

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    dead_code,
    unused_imports,
    unused_variables,
    clippy::all,
)]

// =========================================================================
// Build conditionals
// =========================================================================
//
// In C++ these come from `#ifdef PCSX2_DEVBUILD` / `#ifdef PCSX2_DEBUG`.
// On the Rust side the build system (CMake / cargo) toggles them via
// cargo features `--features dev-build` and `--features debug-build`.
// `cfg!()` resolves to a compile-time literal, so the result can be
// used in `pub const` initialisers.

/// `true` when this crate is built as a development build (C++'s
/// `PCSX2_DEVBUILD` is defined). Defaults to `false`; enable via
/// `--features dev-build`.
pub const IsDevBuild: bool = cfg!(feature = "dev-build");

/// `true` when this crate is built as a debug build (C++'s
/// `PCSX2_DEBUG` is defined). Defaults to `false`; enable via
/// `--features debug-build`.
pub const IsDebugBuild: bool = cfg!(feature = "debug-build");

// =========================================================================
// Architecture detection
// =========================================================================
//
// `cfg!()` returns a `bool` literal at compile time. The two values are
// mutually exclusive on the targets PCSX2 supports (x86, x86_64,
// aarch64); for any other target both resolve to `false` and the build
// will fall back to the C++ `#error Unsupported Platform` check on the
// C++ side.

/// `true` when the host is x86 or x86_64 (equivalent to C++'s
/// `#ifdef ARCH_X86`).
pub const ARCH_X86: bool = cfg!(any(target_arch = "x86", target_arch = "x86_64"));

/// `true` when the host is AArch64 (equivalent to C++'s
/// `#ifdef ARCH_ARM64`).
pub const ARCH_ARM64: bool = cfg!(target_arch = "aarch64");

// =========================================================================
// Page size, cache-line size, page alignment
// =========================================================================
//
// The C++ side selects between 4 KiB / 64-byte (x86) and 16 KiB /
// 128-byte (ARM64) based on `#ifdef ARCH_*` at compile time. On the
// Rust side `cfg!()` plus `const if` collapses the same selection
// without any macro indirection.

/// Memory-page size in bytes. 4 KiB on x86, 16 KiB on ARM64.
pub const __pagesize: u32 = if ARCH_ARM64 { 0x4000 } else { 0x1000 };

/// `__pagesize - 1`. Useful for rounding an address down to a page
/// boundary: `addr & !__pagemask` -> aligned, `addr & __pagemask` ->
/// intra-page offset.
pub const __pagemask: u32 = __pagesize - 1;

/// Number of low zero bits in a page-aligned address. Equivalent to
/// C++'s `std::bit_width(__pagemask)`.
pub const __pageshift: u32 = if ARCH_ARM64 { 14 } else { 12 };

/// Cache line size in bytes. 64 on x86, 128 on ARM64.
pub const __cachelinesize: u32 = if ARCH_ARM64 { 128 } else { 64 };

/// Page-alignment size used for globals. PCSX2 uses a fixed 4 KiB
/// alignment on both x86 and ARM64 because the latter can compute the
/// page address with a single `adrp` instruction.
pub const __pagealignsize: u32 = 0x1000;

// =========================================================================
// Compiler intrinsic replacements
// =========================================================================
//
// C++ exposes these as preprocessor macros. In Rust the closest
// equivalents are attributes and standard-library items; they cannot
// be "exported" across FFI because attributes are part of the source
// language, not the runtime ABI. We document the mapping below and
// provide Rust-callable wrappers where one exists.

// -- __forceinline --------------------------------------------------------
//
// Apply `#[inline(always)]` to the function declaration. Rust has no
// first-class macro for forcing inlining; the attribute is the
// canonical form.
//
//     #[inline(always)]
//     pub fn hot_path(x: u32) -> u32 { x + 1 }
//
// (No FFI surface — attributes are not part of the C ABI.)

// -- __noinline ----------------------------------------------------------
//
// Apply `#[inline(never)]` to the function declaration.

// -- __noreturn ----------------------------------------------------------
//
// Declare the function with `-> !` return type, or call
// `std::process::abort()` / `std::hint::unreachable_unchecked()` from
// inside it. Rust has no separate `#[noreturn]` attribute; functions
// whose body diverges are already considered `noreturn` by the
// optimiser.

// -- ASSUME --------------------------------------------------------------
//
// Mirrors C++'s
//
//     #define ASSUME(x) do { if (!(x)) __builtin_unreachable(); } while (0)
//
// by branching to `std::hint::unreachable_unchecked()` when `x` is
// false. Behaviour is undefined if `x` is false at runtime — the
// optimiser is free to delete downstream code that becomes dead
// under the assumption.

/// `ASSUME(x)` as a function. The branch to `unreachable_unchecked`
/// is marked `unsafe` because the caller is contractually required to
/// guarantee `x` is true at runtime.
#[inline(always)]
pub fn assume(x: bool) {
    if !x {
        // SAFETY: the caller of `assume` promises that `x` is true.
        // If that contract is broken, behaviour is undefined —
        // matching C++'s `ASSUME` semantics.
        unsafe {
            std::hint::unreachable_unchecked();
        }
    }
}

/// `ASSUME(x)` as a macro. Use this when the condition is an
/// expression that should be evaluated at the call site (e.g. an
/// inline check or a complex predicate).
///
///     pcsx2_assume!(index < len);
#[macro_export]
macro_rules! pcsx2_assume {
    ($x:expr) => {
        if !$x {
            // SAFETY: by the contract of `pcsx2_assume!`, `$x` is
            // true at runtime. Violating this is UB.
            unsafe {
                ::core::hint::unreachable_unchecked();
            }
        }
    };
}

// -- RESTRICT ------------------------------------------------------------
//
// C++'s `__restrict` / `__restrict__` is a contract with the compiler
// that the pointer is not aliased. Rust's borrow checker enforces the
// same property structurally (the `&mut T` exclusivity invariant);
// there is no user-facing marker. Mark the parameter as `&mut T` or
// by-value as appropriate.

// -- __fi / __ri ---------------------------------------------------------
//
// `__fi` is `__forceinline`; `__ri` is `__forceinline` in release
// builds and empty in dev builds (so dev builds keep debuggable stack
// traces). On the Rust side, prefer `#[inline]` (let LLVM decide) and
// only escalate to `#[inline(always)]` when profiling proves it
// matters. The C++ conditional-inline macro has no direct equivalent
// in stable Rust; use `#[cfg_attr(not(debug_assertions), inline(always))]`
// when the conditional behaviour is required.

// =========================================================================
// Human-readable byte-size constants
// =========================================================================
//
// C++ exposes these as `static constexpr sptr` (sub-MB) and `s64`
// (MB+). The Rust equivalents use `isize` and `i64` so that
// arithmetic at the original types is preserved.

// -- Sub-megabyte --------------------------------------------------------

/// 1 KiB.
pub const _1kb: isize = 1024;
/// 4 KiB.
pub const _4kb: isize = _1kb * 4;
/// 16 KiB.
pub const _16kb: isize = _1kb * 16;
/// 32 KiB.
pub const _32kb: isize = _1kb * 32;
/// 64 KiB.
pub const _64kb: isize = _1kb * 64;
/// 128 KiB.
pub const _128kb: isize = _1kb * 128;
/// 256 KiB.
pub const _256kb: isize = _1kb * 256;

// -- Megabyte ------------------------------------------------------------

/// 1 MiB.
pub const _1mb: i64 = 1024 * 1024;
/// 8 MiB.
pub const _8mb: i64 = _1mb * 8;
/// 16 MiB.
pub const _16mb: i64 = _1mb * 16;
/// 32 MiB.
pub const _32mb: i64 = _1mb * 32;
/// 64 MiB.
pub const _64mb: i64 = _1mb * 64;
/// 256 MiB.
pub const _256mb: i64 = _1mb * 256;
/// 1 GiB.
pub const _1gb: i64 = _1mb * 1024;
/// 4 GiB.
pub const _4gb: i64 = _1gb * 4;

// =========================================================================
// FFI exports
// =========================================================================
//
// Each `pub const` above has a matching `#[no_mangle] pub static` so
// that the C++ side can `#include "pcsx2_common_rs.h"` and read the
// values directly. The naming follows the `pcsx2_<UPPER_SNAKE>` style
// already used by the rest of this crate's FFI surface.

// ---- Build flags -------------------------------------------------------

#[no_mangle]
pub static pcsx2_IS_DEV_BUILD: u8 = IsDevBuild as u8;

#[no_mangle]
pub static pcsx2_IS_DEBUG_BUILD: u8 = IsDebugBuild as u8;

// ---- Architecture flags ------------------------------------------------

#[no_mangle]
pub static pcsx2_ARCH_X86: u8 = ARCH_X86 as u8;

#[no_mangle]
pub static pcsx2_ARCH_ARM64: u8 = ARCH_ARM64 as u8;

// ---- Page size / cache line --------------------------------------------

#[no_mangle]
pub static pcsx2_PAGE_SIZE: u32 = __pagesize;

#[no_mangle]
pub static pcsx2_PAGE_MASK: u32 = __pagemask;

#[no_mangle]
pub static pcsx2_PAGE_SHIFT: u32 = __pageshift;

#[no_mangle]
pub static pcsx2_PAGE_SIZE_X86: u32 = 0x1000;

#[no_mangle]
pub static pcsx2_PAGE_SIZE_ARM64: u32 = 0x4000;

#[no_mangle]
pub static pcsx2_CACHE_LINE_SIZE: u32 = __cachelinesize;

#[no_mangle]
pub static pcsx2_CACHE_LINE_SIZE_X86: u32 = 64;

#[no_mangle]
pub static pcsx2_CACHE_LINE_SIZE_ARM64: u32 = 128;

#[no_mangle]
pub static pcsx2_PAGE_ALIGN_SIZE: u32 = __pagealignsize;

// ---- Byte-size constants ----------------------------------------------

#[no_mangle]
pub static pcsx2_1KB: isize = _1kb;

#[no_mangle]
pub static pcsx2_4KB: isize = _4kb;

#[no_mangle]
pub static pcsx2_16KB: isize = _16kb;

#[no_mangle]
pub static pcsx2_32KB: isize = _32kb;

#[no_mangle]
pub static pcsx2_64KB: isize = _64kb;

#[no_mangle]
pub static pcsx2_128KB: isize = _128kb;

#[no_mangle]
pub static pcsx2_256KB: isize = _256kb;

#[no_mangle]
pub static pcsx2_1MB: i64 = _1mb;

#[no_mangle]
pub static pcsx2_8MB: i64 = _8mb;

#[no_mangle]
pub static pcsx2_16MB: i64 = _16mb;

#[no_mangle]
pub static pcsx2_32MB: i64 = _32mb;

#[no_mangle]
pub static pcsx2_64MB: i64 = _64mb;

#[no_mangle]
pub static pcsx2_256MB: i64 = _256mb;

#[no_mangle]
pub static pcsx2_1GB: i64 = _1gb;

#[no_mangle]
pub static pcsx2_4GB: i64 = _4gb;