// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
// Rust translation of `common/VectorIntrin.h`.
//
// The C++ header is a thin polyfill that pulls in the appropriate SIMD
// intrinsic header for the target architecture (`<immintrin.h>` for x86,
// `<arm_neon.h>` for aarch64) and bails out at compile time if the build
// isn't targeting at least SSE 4.1 on x86. The Rust counterpart exposes
// the same small set of vector operations under one uniform interface
// (`VectorLoad`, `VectorStore`, `VectorAdd`, `VectorSub`, ...) using
// `core::arch` types that LLVM lowers to the matching native
// instructions. The module is gated per target so it only compiles on
// the architectures PCSX2 actually supports.

#![cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]

// =====================================================================
// x86_64 implementation. Wraps the SSE/AVX intrinsics in a Rust-friendly
// 128-bit vector alias. AVX/AVX2 is preferred when enabled at compile
// time so that the wider 256-bit types can be used by callers that opt
// in; for the common 128-bit path the operations are identical.
// =====================================================================

#[cfg(target_arch = "x86_64")]
pub mod x86_64 {
    use core::arch::x86_64::*;

    /// 128-bit SIMD vector. Equivalent to the `__m128` / `__m128i` /
    /// `__m128d` overloads of the C++ polyfill.
    pub type Vector128 = __m128i;

    /// 256-bit SIMD vector, available only when AVX is enabled.
    #[cfg(target_feature = "avx")]
    pub type Vector256 = __m256i;

    /// Load 16 bytes from an aligned pointer. Mirrors `_mm_load_si128`.
    #[inline]
    #[target_feature(enable = "sse2")]
    pub unsafe fn VectorLoad(ptr: *const u8) -> Vector128 {
        _mm_loadu_si128(ptr as *const __m128i)
    }

    /// Store 16 bytes to an aligned pointer. Mirrors `_mm_store_si128`.
    #[inline]
    #[target_feature(enable = "sse2")]
    pub unsafe fn VectorStore(ptr: *mut u8, value: Vector128) {
        _mm_storeu_si128(ptr as *mut __m128i, value);
    }

    /// Element-wise addition of two 128-bit vectors of 8-bit integers.
    /// Mirrors `_mm_add_epi8`.
    #[inline]
    #[target_feature(enable = "sse2")]
    pub unsafe fn VectorAdd8(value: Vector128, increment: Vector128) -> Vector128 {
        _mm_add_epi8(value, increment)
    }

    /// Element-wise addition of two 128-bit vectors of 16-bit integers.
    /// Mirrors `_mm_add_epi16`.
    #[inline]
    #[target_feature(enable = "sse2")]
    pub unsafe fn VectorAdd16(value: Vector128, increment: Vector128) -> Vector128 {
        _mm_add_epi16(value, increment)
    }

    /// Element-wise addition of two 128-bit vectors of 32-bit integers.
    /// Mirrors `_mm_add_epi32`.
    #[inline]
    #[target_feature(enable = "sse2")]
    pub unsafe fn VectorAdd32(value: Vector128, increment: Vector128) -> Vector128 {
        _mm_add_epi32(value, increment)
    }

    /// Element-wise addition of two 128-bit vectors of 32-bit floats.
    /// Mirrors `_mm_add_ps`.
    #[inline]
    #[target_feature(enable = "sse")]
    pub unsafe fn VectorAddF32(value: __m128, increment: __m128) -> __m128 {
        _mm_add_ps(value, increment)
    }

    /// Element-wise subtraction of two 128-bit vectors of 8-bit integers.
    /// Mirrors `_mm_sub_epi8`.
    #[inline]
    #[target_feature(enable = "sse2")]
    pub unsafe fn VectorSub8(value: Vector128, decrement: Vector128) -> Vector128 {
        _mm_sub_epi8(value, decrement)
    }

    /// Element-wise subtraction of two 128-bit vectors of 16-bit integers.
    /// Mirrors `_mm_sub_epi16`.
    #[inline]
    #[target_feature(enable = "sse2")]
    pub unsafe fn VectorSub16(value: Vector128, decrement: Vector128) -> Vector128 {
        _mm_sub_epi16(value, decrement)
    }

    /// Element-wise subtraction of two 128-bit vectors of 32-bit integers.
    /// Mirrors `_mm_sub_epi32`.
    #[inline]
    #[target_feature(enable = "sse2")]
    pub unsafe fn VectorSub32(value: Vector128, decrement: Vector128) -> Vector128 {
        _mm_sub_epi32(value, decrement)
    }

    /// Bitwise AND of two 128-bit vectors. Mirrors `_mm_and_si128`.
    #[inline]
    #[target_feature(enable = "sse2")]
    pub unsafe fn VectorAnd(value: Vector128, mask: Vector128) -> Vector128 {
        _mm_and_si128(value, mask)
    }

    /// Bitwise OR of two 128-bit vectors. Mirrors `_mm_or_si128`.
    #[inline]
    #[target_feature(enable = "sse2")]
    pub unsafe fn VectorOr(value: Vector128, mask: Vector128) -> Vector128 {
        _mm_or_si128(value, mask)
    }

    /// Bitwise XOR of two 128-bit vectors. Mirrors `_mm_xor_si128`.
    #[inline]
    #[target_feature(enable = "sse2")]
    pub unsafe fn VectorXor(value: Vector128, mask: Vector128) -> Vector128 {
        _mm_xor_si128(value, mask)
    }

    /// 256-bit load, available only with AVX.
    #[cfg(target_feature = "avx")]
    #[inline]
    #[target_feature(enable = "avx")]
    pub unsafe fn VectorLoad256(ptr: *const u8) -> Vector256 {
        _mm256_loadu_si256(ptr as *const __m256i)
    }

    /// 256-bit store, available only with AVX.
    #[cfg(target_feature = "avx")]
    #[inline]
    #[target_feature(enable = "avx")]
    pub unsafe fn VectorStore256(ptr: *mut u8, value: Vector256) {
        _mm256_storeu_si256(ptr as *mut __m256i, value);
    }
}

// =====================================================================
// aarch64 implementation. NEON is mandatory on AArch64 targets so the
// wrappers are always available. The vector width is fixed at 128 bits
// (matching `int8x16_t`, `int16x8_t`, `int32x4_t`, `float32x4_t`).
// =====================================================================

#[cfg(target_arch = "aarch64")]
pub mod aarch64 {
    use core::arch::aarch64::*;

    /// Load 16 bytes from a (possibly unaligned) pointer. Mirrors
    /// `vld1q_s8` / `vld1q_u8`.
    #[inline]
    pub unsafe fn VectorLoad(ptr: *const u8) -> int8x16_t {
        vld1q_s8(ptr as *const i8)
    }

    /// Store 16 bytes to a (possibly unaligned) pointer. Mirrors
    /// `vst1q_s8` / `vst1q_u8`.
    #[inline]
    pub unsafe fn VectorStore(ptr: *mut u8, value: int8x16_t) {
        vst1q_s8(ptr as *mut i8, value);
    }

    /// Element-wise addition of two 128-bit vectors of 8-bit integers.
    /// Mirrors `vaddq_s8`.
    #[inline]
    pub unsafe fn VectorAdd8(value: int8x16_t, increment: int8x16_t) -> int8x16_t {
        vaddq_s8(value, increment)
    }

    /// Element-wise addition of two 128-bit vectors of 16-bit integers.
    /// Mirrors `vaddq_s16`.
    #[inline]
    pub unsafe fn VectorAdd16(value: int16x8_t, increment: int16x8_t) -> int16x8_t {
        vaddq_s16(value, increment)
    }

    /// Element-wise addition of two 128-bit vectors of 32-bit integers.
    /// Mirrors `vaddq_s32`.
    #[inline]
    pub unsafe fn VectorAdd32(value: int32x4_t, increment: int32x4_t) -> int32x4_t {
        vaddq_s32(value, increment)
    }

    /// Element-wise addition of two 128-bit vectors of 32-bit floats.
    /// Mirrors `vaddq_f32`.
    #[inline]
    pub unsafe fn VectorAddF32(value: float32x4_t, increment: float32x4_t) -> float32x4_t {
        vaddq_f32(value, increment)
    }

    /// Element-wise subtraction of two 128-bit vectors of 8-bit integers.
    /// Mirrors `vsubq_s8`.
    #[inline]
    pub unsafe fn VectorSub8(value: int8x16_t, decrement: int8x16_t) -> int8x16_t {
        vsubq_s8(value, decrement)
    }

    /// Element-wise subtraction of two 128-bit vectors of 16-bit integers.
    /// Mirrors `vsubq_s16`.
    #[inline]
    pub unsafe fn VectorSub16(value: int16x8_t, decrement: int16x8_t) -> int16x8_t {
        vsubq_s16(value, decrement)
    }

    /// Element-wise subtraction of two 128-bit vectors of 32-bit integers.
    /// Mirrors `vsubq_s32`.
    #[inline]
    pub unsafe fn VectorSub32(value: int32x4_t, decrement: int32x4_t) -> int32x4_t {
        vsubq_s32(value, decrement)
    }

    /// Bitwise AND of two 128-bit vectors. Mirrors `vandq_s8`.
    #[inline]
    pub unsafe fn VectorAnd(value: int8x16_t, mask: int8x16_t) -> int8x16_t {
        vandq_s8(value, mask)
    }

    /// Bitwise OR of two 128-bit vectors. Mirrors `vorrq_s8`.
    #[inline]
    pub unsafe fn VectorOr(value: int8x16_t, mask: int8x16_t) -> int8x16_t {
        vorrq_s8(value, mask)
    }

    /// Bitwise XOR of two 128-bit vectors. Mirrors `veorq_s8`.
    #[inline]
    pub unsafe fn VectorXor(value: int8x16_t, mask: int8x16_t) -> int8x16_t {
        veorq_s8(value, mask)
    }
}

// =====================================================================
// Unified facade. Re-exports the per-architecture module under a single
// `arch` namespace so callers can write `VectorIntrin::arch::VectorLoad`
// without conditional compilation on the call site.
// =====================================================================

/// Per-architecture SIMD intrinsics. The contents of this module depend
/// on the compilation target.
#[cfg(target_arch = "x86_64")]
pub use self::x86_64 as arch;

#[cfg(target_arch = "aarch64")]
pub use self::aarch64 as arch;

// `alloca` is exposed on the C++ side via `<stdlib.h>` / `<malloc.h>`
// regardless of the SIMD backend. Rust callers should reach for
// `std::mem` (e.g. `Vec` or `MaybeUninit`) instead, so no equivalent is
// provided here; the previous C++ `<alloca>` requirement was a build-
// environment shim rather than part of the vector API.
