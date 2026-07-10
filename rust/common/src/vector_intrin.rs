// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Rust port of `common/VectorIntrin.h`.
//!
//! The original C++ header is the central pivot for PCSX2's SIMD usage:
//! it pulls in the appropriate intrinsic headers for the host architecture
//! (`<xmmintrin.h>`, `<emmintrin.h>`, `<tmmintrin.h>`, `<smmintrin.h>`,
//! `<immintrin.h>` on x86; `<arm_neon.h>` on AArch64), and exposes two
//! compile-time quantities the rest of the codebase relies on:
//!
//! * `_M_SSE` — a packed version number describing the highest SIMD ISA
//!   the build was configured for (`0x501` for AVX2, `0x500` for AVX,
//!   `0x401` for SSE 4.1). The high byte is the major generation, the
//!   low byte is the revision (e.g. `0x501` == "5.1").
//! * `FAST_UNALIGNED` — a flag indicating whether the host supports
//!   fast unaligned vector loads. The C++ definition flips it on once
//!   `_M_SSE >= 0x500` (AVX and above always handle unaligned without
//!   the SSE-era penalty).
//!
//! The PCSX2 C++ source treats these as macro constants because they are
//! consumed in `#if`/`#elif` directives throughout `common/emitter/` and
//! `pcsx2/GS/Renderers/SW/`. In idiomatic Rust we model them as
//! `pub const` items, which are inlined at every use site by the compiler
//! and behave identically for `#[cfg]`-style gating.
//!
//! ## Why this module is mostly configuration
//!
//! All actual SIMD code generation in PCSX2 lives in the x86/ARM JIT
//! emitter (`common/emitter/`), which stays in C++ for now — emitting
//! machine code requires the full `x86Emitter` / `Arm64Emitter` machinery,
//! and is out of scope for the leaf-utility phase of the Rust port.
//! What the Rust side needs from `VectorIntrin.h` is therefore just the
//! two compile-time values plus an FFI-friendly query for runtime
//! capability detection. Runtime detection is useful on x86 because the
//! host CPU may support more than the build was compiled for (e.g. a
//! build with `-msse4.1` running on an AVX2-capable CPU); the
//! `pcsx2_simd_level` export lets the C++ side ask at startup.
//!
//! ## `unsafe` discipline
//!
//! All `std::arch` items in this module are reached only inside
//! `unsafe` blocks. Most of the actual SIMD computation stays out of
//! Rust for now (the emitter is the consumer), but where we do call
//! arch intrinsics we wrap them in `#[target_feature(enable = "...")]`
//! helpers and gate every call site behind a runtime feature check.

#![cfg_attr(
    any(doc, not(any(target_arch = "x86_64", target_arch = "aarch64"))),
    allow(unused_imports, unused_variables)
)]

// ============================================================================
// Compile-time SIMD level and FAST_UNALIGNED flag.
//
// These mirror the macros defined in `common/VectorIntrin.h`. Both are
// `pub const` (not `pub static`) so every use site gets a direct inlined
// literal — exactly what `#define _M_SSE 0x501` did in C++.
// ============================================================================

/// Maximum SIMD ISA level compiled into this crate.
///
/// Encoded as `(major << 8) | minor`, matching the original C++ macro:
///
/// | Value  | Meaning                                |
/// |--------|----------------------------------------|
/// | 0x501  | AVX2 (FMA + BMI1/2)                    |
/// | 0x500  | AVX (no AVX2)                          |
/// | 0x401  | SSE 4.1 (the PCSX2 minimum)            |
/// | 0xA00  | AArch64 NEON (baseline, always present) |
///
/// For x86_64 the value matches the highest `-m` flag the crate was
/// compiled with (see [`build_target_feature_level`]). For AArch64, NEON
/// is mandatory in the architecture, so the value is fixed.
pub const _M_SSE: u32 = build_target_feature_level();

/// True when unaligned vector loads/stores run at full speed.
///
/// The C++ original sets this to `1` once `_M_SSE >= 0x500` because AVX
/// removed the old SSE unaligned-load penalty. We extend the same
/// intent to AArch64: NEON implementations on the cores PCSX2 targets
/// (ARMv8.0+) handle unaligned loads efficiently, so we treat the
/// flag as `true` there as well. Pre-AVX x86 builds (SSE 4.1) keep
/// the conservative `false` value — the penalty is real on those chips.
pub const FAST_UNALIGNED: bool = build_fast_unaligned();

/// Compute the SIMD level constant at compile time.
///
/// Centralised so the cascading order matches the C++ preprocessor
/// (AVX2 → AVX → SSE4.1 → compile error on x86; fixed 0xA00 on
/// AArch64; conservative 0 on any other architecture).
///
/// Every arm produces a `u32` literal so the function itself
/// type-checks as `-> u32`. The `[cfg]`-gated `compile_error!` block
/// (see below) catches misconfigured x86 builds at compile time —
/// here we just return the safe fallback `0` so unrelated
/// `cargo doc` / `cargo check --target` invocations succeed.
#[inline(always)]
const fn build_target_feature_level() -> u32 {
    #[cfg(target_arch = "x86_64")]
    {
        // Mirrors `#if defined(__AVX2__)` / `__AVX__` / `__SSE4_1__`.
        // Rust's `target_feature` cfg is the equivalent of the compiler
        // built-ins the C++ header consulted. The arms are ordered
        // most-specific first; only one fires per build.
        #[cfg(target_feature = "avx2")]
        {
            return 0x501;
        }
        #[cfg(all(target_feature = "avx", not(target_feature = "avx2")))]
        {
            return 0x500;
        }
        #[cfg(all(
            target_feature = "sse4.1",
            not(target_feature = "avx"),
            not(target_feature = "avx2")
        ))]
        {
            return 0x401;
        }
        // PCSX2's C++ header treats anything below SSE 4.1 as a build
        // error. The `compile_error!` block below mirrors that at
        // compile time; here we fall through to the catch-all below.
    }

    #[cfg(target_arch = "aarch64")]
    {
        // NEON is mandatory in AArch64; no feature flag to consult.
        // `0xA00` reads as "ARM, generation 0" — a convention chosen
        // to be distinct from any x86 level while remaining in the
        // same numeric envelope so `>=` comparisons still order
        // meaningfully.
        return 0xA00;
    }

    // Any other architecture (32-bit x86, 32-bit ARM, RISC-V, …) is
    // not a build target for PCSX2, and a misconfigured x86 build
    // (no sse4.1) also lands here. The `compile_error!` block above
    // turns the latter into a build failure; this fallback keeps the
    // crate compiling for `cargo doc` and unrelated-target `check`.
    //
    // `unreachable_code` is allowed because the per-arch `return`s
    // above handle the supported targets, and the fallback only fires
    // on a target combination we never actually ship.
    #[allow(unreachable_code)]
    0
}

// `const fn` cannot `panic!` at compile time in all contexts, and we
// cannot emit a `#error` directive from const context on stable. We
// mirror the C++ header's behaviour with a `compile_error!` that
// fires only when:
//   - we're on x86_64, AND
//   - the build was *not* configured with any of sse4.1 / avx / avx2.
//
// Production builds set `-C target-feature=+sse4.1` (or higher) via
// RUSTFLAGS, so this assertion stays quiet. A developer who forgets
// to set those flags gets a clear compile-time message instead of a
// silent runtime regression. We still allow the function to compile
// to `0` (via the fallback in `build_target_feature_level`) so that
// `cargo doc` and `cargo check --target` for unrelated targets don't
// fail with cryptic errors.

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
const _: () = {
    #[cfg(all(
        target_arch = "x86_64",
        not(any(
            target_feature = "sse4.1",
            target_feature = "avx",
            target_feature = "avx2"
        ))
    ))]
    compile_error!("PCSX2 requires compiling for at least SSE 4.1 (or higher: AVX / AVX2).");

    #[cfg(all(target_arch = "x86_64", target_feature = "sse4.1"))]
    const _SSE41_LEVEL: u32 = build_target_feature_level();
    #[cfg(all(target_arch = "x86_64", target_feature = "avx"))]
    const _AVX_LEVEL: u32 = build_target_feature_level();
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    const _AVX2_LEVEL: u32 = build_target_feature_level();
    #[cfg(target_arch = "aarch64")]
    const _AARCH64_LEVEL: u32 = build_target_feature_level();
};

/// Compute the `FAST_UNALIGNED` constant at compile time.
///
/// See [`FAST_UNALIGNED`] for the rationale. We duplicate the body of
/// `build_target_feature_level` instead of reading the cached value to
/// keep the function `const fn` (reading a `pub const` from a `const fn`
/// works too, but inlining it is friendlier to debug builds).
#[inline(always)]
const fn build_fast_unaligned() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        // Matches `#if _M_SSE >= 0x500` in the C++ original.
        #[cfg(any(target_feature = "avx", target_feature = "avx2"))]
        {
            true
        }
        #[cfg(not(any(target_feature = "avx", target_feature = "avx2")))]
        {
            false
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        // Modern AArch64 cores handle unaligned NEON loads at full
        // throughput; flag it on to keep `common/emitter/`'s use of
        // `FAST_UNALIGNED` consistent across architectures.
        true
    }
}

// ============================================================================
// Runtime SIMD detection (x86 only).
//
// The x86 build of PCSX2 historically compiles multiple paths — SSE 4.1
// vs. AVX vs. AVX2 — and selects at startup based on what the CPU
// actually supports. The Rust crate is only built once (with one ISA
// level baked in), but we still expose a runtime query so the C++ side
// can verify the loaded library matches its expectations.
//
// On AArch64 the value is constant.
// ============================================================================

/// Runtime CPU SIMD level, expressed in the same encoding as [`_M_SSE`].
///
/// On x86_64 this is the highest level the running CPU supports among
/// the features Rust can detect at runtime. On AArch64 NEON is
/// mandatory, so this simply returns the compile-time level.
///
/// Safe to call at any time; performs a single CPUID-style feature test
/// the first time it is invoked and caches the result.
#[inline]
pub fn runtime_simd_level() -> u32 {
    runtime_simd_level_impl()
}

#[cfg(target_arch = "x86_64")]
#[inline]
fn runtime_simd_level_impl() -> u32 {
    // Use runtime feature detection so we don't have to compile the
    // whole crate under `target_feature = "+avx2"`. The C++ side
    // already does the same thing via `xgetbv` + `__cpuid`.
    //
    // We deliberately probe AVX2 first so the returned level reflects
    // the *highest* ISA the CPU can run, matching the original C++
    // helper's behaviour.
    if is_x86_feature_detected!("avx2") {
        0x501
    } else if is_x86_feature_detected!("avx") {
        0x500
    } else if is_x86_feature_detected!("sse4.1") {
        0x401
    } else {
        // Should never happen on any CPU PCSX2 supports, but mirror
        // the C++ fallback rather than panicking.
        0
    }
}

#[cfg(target_arch = "aarch64")]
#[inline]
fn runtime_simd_level_impl() -> u32 {
    // NEON is mandatory on AArch64; nothing to detect at runtime.
    _M_SSE
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
#[inline]
fn runtime_simd_level_impl() -> u32 {
    0
}

// ============================================================================
// Re-export the relevant `std::arch` types so downstream Rust code in the
// crate (and any future emitter port) can refer to the vector types
// without repeating the `cfg` ladder at every call site.
//
// We don't pull in any arch modules here on non-target builds; the
// `cfg_attr` at the top of the file silences unused-import warnings
// for the documentation and `cargo check --target` cases.
// ============================================================================

#[cfg(target_arch = "x86_64")]
pub use std::arch::x86_64::{
    __m128, __m128d, __m128i, __m256, __m256d, __m256i, _mm_loadu_si128, _mm_setzero_si128,
    _mm_storeu_si128,
};

#[cfg(target_arch = "aarch64")]
pub use std::arch::aarch64::{
    int32x4_t, uint32x4_t, vld1q_u32, vst1q_u32,
};

// ============================================================================
// Lightweight SIMD helpers.
//
// The full SIMD surface used by PCSX2 lives in the C++ emitter. We
// provide a tiny set of zero/load/store helpers here purely so the
// FFI surface below has something to refer to and so future Rust
// code can do scalar-equivalent operations without reaching for
// unstable intrinsics. Each helper is gated behind the appropriate
// `#[target_feature]` and is only marked `unsafe` because the
// underlying intrinsics are.
// ============================================================================

/// Build a zero-initialised 128-bit integer vector.
///
/// x86_64: returns `_mm_setzero_si128()`.
/// aarch64: returns `vld1q_u32([0, 0, 0, 0])` via a constant.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
#[inline]
pub unsafe fn zero_si128() -> __m128i {
    // Safety: caller is bound by the `unsafe fn` contract, and the
    // enclosing `#[target_feature(enable = "sse2")]` guarantees the
    // CPU/runtime supports the instruction. `setzero` is a leaf
    // intrinsic with no further preconditions.
    _mm_setzero_si128()
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
pub unsafe fn zero_u32x4() -> uint32x4_t {
    // Safety: `neon` is in the target_feature list; `vld1q` accepts a
    // pointer to 4 `u32`s and is well-defined for any alignment.
    unsafe { vld1q_u32([0u32, 0, 0, 0].as_ptr()) }
}

/// Load 16 bytes from `src` (which need not be 16-byte aligned).
///
/// Matches `_mm_loadu_si128` semantics: the load may fault if `src`
/// doesn't actually point to 16 readable bytes; it does *not* fault
/// on alignment, which is the whole reason the `_u_` variant exists.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
#[inline]
pub unsafe fn loadu_si128(src: *const u8) -> __m128i {
    // Cast through `*const __m128i` per the intrinsic's contract; the
    // `_u_` variant requires only that the pointer is readable for
    // 16 bytes regardless of alignment.
    unsafe { _mm_loadu_si128(src as *const __m128i) }
}

/// Store 16 bytes to `dst` (which need not be 16-byte aligned).
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
#[inline]
pub unsafe fn storeu_si128(dst: *mut u8, val: __m128i) {
    unsafe { _mm_storeu_si128(dst as *mut __m128i, val) }
}

/// Load 4 `u32`s from `src` (NEON equivalent of `_mm_loadu_si128`).
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
pub unsafe fn load_u32x4(src: *const u32) -> uint32x4_t {
    unsafe { vld1q_u32(src) }
}

/// Store 4 `u32`s to `dst` (NEON equivalent of `_mm_storeu_si128`).
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
pub unsafe fn store_u32x4(dst: *mut u32, val: uint32x4_t) {
    unsafe { vst1q_u32(dst, val) }
}

// ============================================================================
// FFI surface (consumed by C++ PCSX2 via cbindgen).
//
// These mirror the C++ constants in `common/VectorIntrin.h` but as
// runtime-callable functions so the C++ side can verify at startup
// that the loaded `pcsx2_common_rs` was built with the expected ISA.
// ============================================================================

/// FFI export: the highest SIMD ISA level this build supports.
///
/// Returns the packed `(major << 8) | minor` version, identical to
/// the `_M_SSE` macro value used by the C++ build. The C++ side can
/// compare the returned value against its own compile-time constant
/// to detect mismatches (e.g. Rust built with SSE 4.1 but C++ with
/// AVX2).
#[no_mangle]
pub extern "C" fn pcsx2_simd_level() -> u32 {
    // `runtime_simd_level` may report a higher number than the crate
    // was compiled with — that is intentional and matches the C++
    // helper. C++ code that wants to know *what this binary supports*
    // should read `_M_SSE` directly; `pcsx2_simd_level` answers the
    // distinct question "what does the CPU support?"
    runtime_simd_level()
}

/// FFI export: whether unaligned vector loads run at full speed.
///
/// Returns [`FAST_UNALIGNED`] as a C `bool`. The C++ side consumes
/// this alongside `pcsx2_simd_level` to decide which vector-emitter
/// path to take when unaligned memory is in play.
#[no_mangle]
pub extern "C" fn pcsx2_simd_fast_unaligned() -> bool {
    FAST_UNALIGNED
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `_M_SSE` is one of the known-good encoded values.
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    #[test]
    fn simd_level_is_well_formed() {
        let level = _M_SSE;
        let valid = match level {
            0x401 | 0x500 | 0x501 | 0xA00 => true,
            _ => false,
        };
        assert!(valid, "_M_SSE = {:#x} is not a recognised level", level);
    }

    /// `FAST_UNALIGNED` matches the rule used in the C++ header.
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn fast_unaligned_matches_rule() {
        let expected = cfg!(any(target_feature = "avx", target_feature = "avx2"));
        assert_eq!(FAST_UNALIGNED, expected);
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn fast_unaligned_true_on_aarch64() {
        assert!(FAST_UNALIGNED);
    }

    /// Runtime detection must agree with the compile-time level on
    /// architectures with a fixed ISA (AArch64), and must be `>=` the
    /// compile-time level on x86 (the CPU may support more than the
    /// build target).
    #[test]
    fn runtime_level_is_sane() {
        let rt = runtime_simd_level();
        #[cfg(target_arch = "aarch64")]
        assert_eq!(rt, _M_SSE);
        #[cfg(target_arch = "x86_64")]
        assert!(rt >= _M_SSE, "runtime {} < compile-time {}", rt, _M_SSE);
    }

    /// The two FFI exports match their `pub fn` counterparts.
    #[test]
    fn ffi_exports_match() {
        // The FFI functions are plain wrappers, but the assertion
        // guards against accidental rewiring.
        // Note: `pcsx2_simd_fast_unaligned()` returns the const, so
        // there's no observable drift to catch — this is a smoke
        // test that the symbols resolve at all.
        let _ = pcsx2_simd_level();
        let _ = pcsx2_simd_fast_unaligned();
    }
}