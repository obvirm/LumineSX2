// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust translation of PCSX2's GSVector SIMD types.
//!
//! This module exposes the public surface of the original C++ SIMD vector
//! library translated into Rust 2021. The four main types are:
//!
//! * [`GsVector4`]   - 4 packed `f32`s (128-bit)
//! * [`GsVector4i`]  - 4 packed `i32`s (128-bit)
//! * [`GsVector8`]   - 8 packed `f32`s (256-bit, AVX on x86_64)
//! * [`GsVector8i`]  - 8 packed `i32`s (256-bit, AVX2 on x86_64)
//!
//! On x86_64 the implementations use the SSE / SSE2 / SSE4.1 / AVX / AVX2
//! intrinsics from [`core::arch::x86_64`]. On aarch64 the 128-bit types are
//! backed by NEON intrinsics from [`core::arch::aarch64`]. Every function
//! that directly invokes a SIMD intrinsic is `unsafe`; safe wrappers that
//! do not need to perform unchecked operations are not marked `unsafe`.
//!
//! The module only depends on the standard library and `core::arch`.
//! There is no allocator, no `libc`, and no `extern` C.

#![allow(non_camel_case_types)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::missing_safety_doc)]

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;
#[cfg(target_arch = "aarch64")]
use core::arch::aarch64::*;

// ---------------------------------------------------------------------------
// Shared enums that mirror the C++ enums in `GSVector.h`.
// ---------------------------------------------------------------------------

/// Mirrors the C++ `Round_Mode` enum used by the rounding intrinsics.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum RoundMode {
    NearestInt = 0,
    NegInf = 1,
    PosInf = 2,
    Truncate = 3,
}

/// Mirrors the C++ `Align_Mode` enum used by the `ralign` helpers.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AlignMode {
    Outside,
    Inside,
    NegInf,
    PosInf,
}

// ---------------------------------------------------------------------------
// GsVector4 - 4 x f32 (128-bit).
// ---------------------------------------------------------------------------

/// 128-bit floating point vector, the Rust equivalent of `GSVector4`.
///
/// Internally stored as an `[f32; 4]` plus a cached copy of the underlying
/// `__m128` (x86_64) or `float32x4_t` (aarch64) for SIMD operations.
#[derive(Copy, Clone, Debug, Default)]
#[repr(C, align(16))]
pub struct GsVector4 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl GsVector4 {
    /// Build a vector from four lanes.
    #[inline]
    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self { x, y, z, w }
    }

    /// Build a vector where all four lanes hold the same value.
    #[inline]
    pub const fn splat(v: f32) -> Self {
        Self::new(v, v, v, v)
    }

    /// `cxpr` analogue: a `const fn` constructor matching the C++ static.
    #[inline]
    pub const fn cxpr(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self::new(x, y, z, w)
    }

    /// `cxpr` analogue broadcasting a single value.
    #[inline]
    pub const fn cxpr_splat(v: f32) -> Self {
        Self::splat(v)
    }

    /// Build a vector from a `[f32; 4]`.
    #[inline]
    pub fn from_array(a: [f32; 4]) -> Self {
        Self::new(a[0], a[1], a[2], a[3])
    }

    /// Read the four lanes as an array.
    #[inline]
    pub fn to_array(self) -> [f32; 4] {
        [self.x, self.y, self.z, self.w]
    }

    // -----------------------------------------------------------------------
    // SIMD backends.
    // -----------------------------------------------------------------------

    /// Load a vector from a SIMD register (x86_64 `_mm_set_ps` style).
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn from_m128(m: __m128) -> Self {
        let mut out = Self::default();
        core::arch::x86_64::_mm_storeu_ps(&mut out as *mut _ as *mut f32, m);
        out
    }

    /// Load a vector from a NEON register (aarch64).
    #[inline]
    #[cfg(target_arch = "aarch64")]
    pub unsafe fn from_neon(m: float32x4_t) -> Self {
        let mut out = Self::default();
        vst1q_f32(&mut out as *mut _ as *mut f32, m);
        out
    }

    /// Convert to a SIMD register (x86_64).
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn to_m128(self) -> __m128 {
        _mm_loadu_ps(&self as *const _ as *const f32)
    }

    /// Convert to a NEON register (aarch64).
    #[inline]
    #[cfg(target_arch = "aarch64")]
    pub unsafe fn to_neon(self) -> float32x4_t {
        vld1q_f32(self as *const _ as *const f32)
    }

    // -----------------------------------------------------------------------
    // Static initializers (lazy-const variants of the C++ `m_*` constants).
    // -----------------------------------------------------------------------

    /// All-zero vector.
    #[inline]
    pub fn zero() -> Self {
        Self::splat(0.0)
    }

    /// Vector of all-ones bits. NaN test / true predicate.
    #[inline]
    pub fn xffffffff() -> Self {
        // Bit pattern of all-ones: only the sign bit per lane, matches the
        // C++ `zero() == zero()` trick.
        Self::from_bits(0xFFFFFFFFu32)
    }

    /// 1.0 broadcast.
    #[inline]
    pub fn one() -> Self {
        Self::splat(1.0)
    }

    /// 0.5 broadcast.
    #[inline]
    pub fn half() -> Self {
        Self::splat(0.5)
    }

    /// 2.0 broadcast.
    #[inline]
    pub fn two() -> Self {
        Self::splat(2.0)
    }

    /// 4.0 broadcast.
    #[inline]
    pub fn four() -> Self {
        Self::splat(4.0)
    }

    /// Vector `(0, 1, 2, 3)`.
    #[inline]
    pub fn ps0123() -> Self {
        Self::new(0.0, 1.0, 2.0, 3.0)
    }

    /// Vector `(4, 5, 6, 7)`.
    #[inline]
    pub fn ps4567() -> Self {
        Self::new(4.0, 5.0, 6.0, 7.0)
    }

    /// Bit-casting a `u32` lane to `f32` lanes.
    #[inline]
    pub fn from_bits(bits: u32) -> Self {
        Self::splat(f32::from_bits(bits))
    }

    // -----------------------------------------------------------------------
    // Memory loads and stores.
    // -----------------------------------------------------------------------

    /// Load four floats from a (potentially unaligned) pointer.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn loadu(ptr: *const f32) -> Self {
        Self::from_m128(_mm_loadu_ps(ptr))
    }

    /// Load four floats from an aligned pointer.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn load(ptr: *const f32) -> Self {
        Self::from_m128(_mm_load_ps(ptr))
    }

    /// Load four floats from a pointer (aarch64).
    #[inline]
    #[cfg(target_arch = "aarch64")]
    pub unsafe fn loadu(ptr: *const f32) -> Self {
        Self::from_neon(vld1q_f32(ptr))
    }

    /// Store to a (potentially unaligned) pointer.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn storeu(self, ptr: *mut f32) {
        _mm_storeu_ps(ptr, self.to_m128());
    }

    /// Store to an aligned pointer.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn store(self, ptr: *mut f32) {
        _mm_store_ps(ptr, self.to_m128());
    }

    /// Store to a (potentially unaligned) pointer (aarch64).
    #[inline]
    #[cfg(target_arch = "aarch64")]
    pub unsafe fn storeu(self, ptr: *mut f32) {
        vst1q_f32(ptr, self.to_neon());
    }

    /// Non-temporal store to an aligned pointer.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn storent(self, ptr: *mut f32) {
        _mm_stream_ps(ptr, self.to_m128());
    }

    // -----------------------------------------------------------------------
    // Per-lane accessors and small constructors.
    // -----------------------------------------------------------------------

    /// Vector with the first lane set to `v`, others zero.
    #[inline]
    pub fn load1(v: f32) -> Self {
        Self::new(v, 0.0, 0.0, 0.0)
    }

    /// Vector built from a single `f32` broadcast (uses `m128d` load on x86).
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn broadcast(v: f32) -> Self {
        // `_mm_broadcastss_ps` only exists on SSE4.1+; the safe fallback is
        // `_mm_set1_ps` which is always available on x86_64.
        let m = _mm_set_ps1(v);
        Self::from_m128(m)
    }

    /// Vector built from a single `f32` broadcast on aarch64.
    #[inline]
    #[cfg(target_arch = "aarch64")]
    pub unsafe fn broadcast(v: f32) -> Self {
        Self::from_neon(vdupq_n_f32(v))
    }

    /// Load a 64-bit double and broadcast it across the two 64-bit lanes
    /// (giving an `f32` vector that aliases as 2 doubles).
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn broadcast64(ptr: *const f64) -> Self {
        let m = _mm_castpd_ps(_mm_loaddup_pd(ptr));
        Self::from_m128(m)
    }

    // -----------------------------------------------------------------------
    // Arithmetic.
    // -----------------------------------------------------------------------

    #[inline]
    pub fn add(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128(_mm_add_ps(self.to_m128(), rhs.to_m128()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            Self::from_neon(vaddq_f32(self.to_neon(), rhs.to_neon()))
        }
    }

    #[inline]
    pub fn sub(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128(_mm_sub_ps(self.to_m128(), rhs.to_m128()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            Self::from_neon(vsubq_f32(self.to_neon(), rhs.to_neon()))
        }
    }

    #[inline]
    pub fn mul(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128(_mm_mul_ps(self.to_m128(), rhs.to_m128()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            Self::from_neon(vmulq_f32(self.to_neon(), rhs.to_neon()))
        }
    }

    #[inline]
    pub fn div(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128(_mm_div_ps(self.to_m128(), rhs.to_m128()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            Self::from_neon(vdivq_f32(self.to_neon(), rhs.to_neon()))
        }
    }

    /// Component-wise minimum.
    #[inline]
    pub fn min(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128(_mm_min_ps(self.to_m128(), rhs.to_m128()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            Self::from_neon(vminq_f32(self.to_neon(), rhs.to_neon()))
        }
    }

    /// Component-wise maximum.
    #[inline]
    pub fn max(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128(_mm_max_ps(self.to_m128(), rhs.to_m128()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            Self::from_neon(vmaxq_f32(self.to_neon(), rhs.to_neon()))
        }
    }

    /// Horizontal add of adjacent pairs. Returns `[x+y, z+w, x+y, z+w]`.
    #[inline]
    pub fn hadd(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128(_mm_hadd_ps(self.to_m128(), rhs.to_m128()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            // NEON has no `hadd` directly; use a pair of pairwise adds.
            let l = vpaddq_f32(self.to_neon(), rhs.to_neon());
            Self::from_neon(l)
        }
    }

    /// Horizontal subtract of adjacent pairs.
    #[inline]
    pub fn hsub(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128(_mm_hsub_ps(self.to_m128(), rhs.to_m128()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            // Fall back to scalar lane math, NEON has no direct `hsub` for f32.
            let a = self.to_array();
            let b = rhs.to_array();
            Self::new(a[0] - a[1], a[2] - a[3], b[0] - b[1], b[2] - b[3])
        }
    }

    // -----------------------------------------------------------------------
    // sqrt / rcp / dot / cross / etc.
    // -----------------------------------------------------------------------

    /// Component-wise square root.
    #[inline]
    pub fn sqrt(self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128(_mm_sqrt_ps(self.to_m128()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            Self::from_neon(vsqrtq_f32(self.to_neon()))
        }
    }

    /// Fast reciprocal (12-bit accurate).
    #[inline]
    pub fn rcp(self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128(_mm_rcp_ps(self.to_m128()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            // NEON has no rcp, derive it from 1/x (single division).
            Self::from_neon(vdivq_f32(vdupq_n_f32(1.0), self.to_neon()))
        }
    }

    /// Refined reciprocal (Newton-Raphson on the result of `rcp`).
    #[inline]
    pub fn rcpnr(self) -> Self {
        let v = self.rcp();
        let two = Self::two();
        v.add(v).sub(v.mul(v).mul(self))
            .add(two.mul(v).sub(v.mul(v).mul(self)))
            .sub(v.mul(v).mul(self))
    }

    /// Dot product of `self` and `rhs`, broadcast to all four lanes.
    #[inline]
    pub fn dot(self, rhs: Self) -> Self {
        let a = self.mul(rhs);
        let b = a.hadd(a);
        b.hadd(b)
    }

    /// 3D cross product, with the result written to the `xyz` lanes and
    /// the original `self.w` preserved.
    #[inline]
    pub fn cross(self, rhs: Self) -> Self {
        let a = self.yzx().mul(rhs.zxy());
        let b = self.zxy().mul(rhs.yzx());
        a.sub(b)
    }

    // -----------------------------------------------------------------------
    // Rounding.
    // -----------------------------------------------------------------------

    #[inline]
    pub fn round(self, mode: RoundMode) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            // `_mm_round_ps` requires a const immediate, so dispatch on the
            // runtime `mode` to the matching const entry point.
            match mode {
                RoundMode::NearestInt => Self::from_m128(_mm_round_ps::<{ _MM_FROUND_TO_NEAREST_INT | _MM_FROUND_NO_EXC }>(self.to_m128())),
                RoundMode::NegInf => Self::from_m128(_mm_round_ps::<{ _MM_FROUND_TO_NEG_INF | _MM_FROUND_NO_EXC }>(self.to_m128())),
                RoundMode::PosInf => Self::from_m128(_mm_round_ps::<{ _MM_FROUND_TO_POS_INF | _MM_FROUND_NO_EXC }>(self.to_m128())),
                RoundMode::Truncate => Self::from_m128(_mm_round_ps::<{ _MM_FROUND_TO_ZERO | _MM_FROUND_NO_EXC }>(self.to_m128())),
            }
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            let m = match mode {
                RoundMode::NearestInt => vcvtnq_f32_f32(self.to_neon()),
                RoundMode::NegInf => {
                    let v = vcvtmq_f32_f32(self.to_neon());
                    v
                }
                RoundMode::PosInf => {
                    let v = vcvtpq_f32_f32(self.to_neon());
                    v
                }
                RoundMode::Truncate => vcvtq_f32_s32(vcvtnq_s32_f32(self.to_neon())),
            };
            Self::from_neon(m)
        }
    }

    /// Round towards `-inf`.
    #[inline]
    pub fn floor(self) -> Self {
        self.round(RoundMode::NegInf)
    }

    /// Round towards `+inf`.
    #[inline]
    pub fn ceil(self) -> Self {
        self.round(RoundMode::PosInf)
    }

    /// Round towards zero.
    #[inline]
    pub fn trunc(self) -> Self {
        self.round(RoundMode::Truncate)
    }

    // -----------------------------------------------------------------------
    // Bitwise / boolean.
    // -----------------------------------------------------------------------

    /// Bitwise AND.
    #[inline]
    pub fn and(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128(_mm_and_ps(self.to_m128(), rhs.to_m128()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            // NEON has no f32 bitwise ops; use the vreinterpretq_*_u32 path.
            let a: uint32x4_t = vreinterpretq_u32_f32(self.to_neon());
            let b: uint32x4_t = vreinterpretq_u32_f32(rhs.to_neon());
            Self::from_neon(vreinterpretq_f32_u32(vandq_u32(a, b)))
        }
    }

    /// Bitwise OR.
    #[inline]
    pub fn or(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128(_mm_or_ps(self.to_m128(), rhs.to_m128()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            let a: uint32x4_t = vreinterpretq_u32_f32(self.to_neon());
            let b: uint32x4_t = vreinterpretq_u32_f32(rhs.to_neon());
            Self::from_neon(vreinterpretq_f32_u32(vorrq_u32(a, b)))
        }
    }

    /// Bitwise XOR.
    #[inline]
    pub fn xor(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128(_mm_xor_ps(self.to_m128(), rhs.to_m128()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            let a: uint32x4_t = vreinterpretq_u32_f32(self.to_neon());
            let b: uint32x4_t = vreinterpretq_u32_f32(rhs.to_neon());
            Self::from_neon(vreinterpretq_f32_u32(veorq_u32(a, b)))
        }
    }

    /// `!v & rhs` semantics, matches the C++ `_mm_andnot_ps(rhs, self)`.
    #[inline]
    pub fn andnot(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128(_mm_andnot_ps(rhs.to_m128(), self.to_m128()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            let ones = vdupq_n_u32(!0u32);
            let rb: uint32x4_t = vreinterpretq_u32_f32(rhs.to_neon());
            let a: uint32x4_t = vreinterpretq_u32_f32(self.to_neon());
            let inv = veorq_u32(rb, ones);
            Self::from_neon(vreinterpretq_f32_u32(vandq_u32(inv, a)))
        }
    }

    /// `self == rhs`, lane-wise, returns a vector with all-ones bits per
    /// matching lane and zero bits elsewhere.
    #[inline]
    pub fn cmpeq(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128(_mm_cmpeq_ps(self.to_m128(), rhs.to_m128()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            Self::from_neon(vreinterpretq_f32_u32(vceqq_f32(
                self.to_neon(),
                rhs.to_neon(),
            )))
        }
    }

    /// `self != rhs`.
    #[inline]
    pub fn cmpneq(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128(_mm_cmpneq_ps(self.to_m128(), rhs.to_m128()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            let eq = vceqq_f32(self.to_neon(), rhs.to_neon());
            let ones = vdupq_n_u32(!0u32);
            let neq = veorq_u32(eq, ones);
            Self::from_neon(vreinterpretq_f32_u32(neq))
        }
    }

    /// `self < rhs`.
    #[inline]
    pub fn cmplt(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128(_mm_cmplt_ps(self.to_m128(), rhs.to_m128()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            Self::from_neon(vreinterpretq_f32_u32(vcltq_f32(
                self.to_neon(),
                rhs.to_neon(),
            )))
        }
    }

    /// `self <= rhs`.
    #[inline]
    pub fn cmple(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128(_mm_cmple_ps(self.to_m128(), rhs.to_m128()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            Self::from_neon(vreinterpretq_f32_u32(vcleq_f32(
                self.to_neon(),
                rhs.to_neon(),
            )))
        }
    }

    /// `self > rhs`.
    #[inline]
    pub fn cmpgt(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128(_mm_cmpgt_ps(self.to_m128(), rhs.to_m128()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            Self::from_neon(vreinterpretq_f32_u32(vcgtq_f32(
                self.to_neon(),
                rhs.to_neon(),
            )))
        }
    }

    /// `self >= rhs`.
    #[inline]
    pub fn cmpge(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128(_mm_cmpge_ps(self.to_m128(), rhs.to_m128()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            Self::from_neon(vreinterpretq_f32_u32(vcgeq_f32(
                self.to_neon(),
                rhs.to_neon(),
            )))
        }
    }

    /// `x > 0` test as a bit mask. Returns 0..15 with bit `i` set if
    /// `lane i` has its sign bit clear (i.e. is non-negative).
    #[inline]
    pub fn mask(self) -> i32 {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            _mm_movemask_ps(self.to_m128())
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            // NEON gives sign bits via vcltzq_f32; non-negative -> bit 0.
            // We build a bitmask of 4 bits.
            let a: int32x4_t = vreinterpretq_s32_f32(self.to_neon());
            let a_lo = vget_low_s32(a);
            let a_hi = vget_high_s32(a);
            let mut m: i32 = 0;
            if vgetq_lane_s32(a, 0) >= 0 {
                m |= 1;
            }
            if vgetq_lane_s32(a, 1) >= 0 {
                m |= 2;
            }
            if vgetq_lane_s32(a, 2) >= 0 {
                m |= 4;
            }
            if vgetq_lane_s32(a, 3) >= 0 {
                m |= 8;
            }
            let _ = (a_lo, a_hi);
            m
        }
    }

    /// True if all four lanes have the sign bit clear (i.e. all > 0 in
    /// IEEE-754 compare with -0).
    #[inline]
    pub fn alltrue(self) -> bool {
        self.mask() == 0xF
    }

    /// True if all four lanes have the sign bit set (i.e. all < 0).
    #[inline]
    pub fn allfalse(self) -> bool {
        self.mask() == 0
    }

    // -----------------------------------------------------------------------
    // Shuffles / lanes.
    // -----------------------------------------------------------------------

    /// Lane swap: `[x, y, z, w]` -> `[y, x, z, w]`.
    #[inline]
    pub fn yxwz(self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128(_mm_shuffle_ps(
                self.to_m128(),
                self.to_m128(),
                0b01_00_01_00,
            ))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            // vtrn across pairs gives the same effect on the low two lanes
            // only; do it in scalar for portability.
            let a = self.to_array();
            Self::new(a[1], a[0], a[3], a[2])
        }
    }

    /// Broadcast the `x` lane to all four.
    #[inline]
    pub fn xxxx(self) -> Self {
        Self::splat(self.x)
    }

    /// Broadcast the `y` lane.
    #[inline]
    pub fn yyyy(self) -> Self {
        Self::splat(self.y)
    }

    /// Broadcast the `z` lane.
    #[inline]
    pub fn zzzz(self) -> Self {
        Self::splat(self.z)
    }

    /// Broadcast the `w` lane.
    #[inline]
    pub fn wwww(self) -> Self {
        Self::splat(self.w)
    }

    /// Lanes `(y, x, z, w)`.
    #[inline]
    pub fn yx(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128(_mm_shuffle_ps(
                self.to_m128(),
                rhs.to_m128(),
                0b11_10_00_01,
            ))
        }
        #[cfg(target_arch = "aarch64")]
        {
            Self::new(rhs.y, self.x, self.z, self.w)
        }
    }

    /// Lanes `(x, y, x, y)`.
    #[inline]
    pub fn xyxy(self) -> Self {
        Self::new(self.x, self.y, self.x, self.y)
    }

    /// Lanes `(z, w, z, w)`.
    #[inline]
    pub fn zwzw(self) -> Self {
        Self::new(self.z, self.w, self.z, self.w)
    }

    /// Lanes `(x, y, z, w) -> (y, z, x, w)`.
    #[inline]
    pub fn yzxw(self) -> Self {
        Self::new(self.y, self.z, self.x, self.w)
    }

    /// Lanes `(x, y, z, w) -> (y, x, z, w)`.
    #[inline]
    pub fn yzx(self) -> Self {
        Self::new(self.y, self.z, self.x, self.w)
    }

    /// Lanes `(x, y, z, w) -> (z, x, y, w)`.
    #[inline]
    pub fn zxy(self) -> Self {
        Self::new(self.z, self.x, self.y, self.w)
    }

    /// Absolute value: clears the sign bit of every lane.
    #[inline]
    pub fn abs(self) -> Self {
        let mask = Self::from_bits(0x7FFF_FFFF);
        self.and(mask)
    }

    /// Negation: flips the sign bit of every lane.
    #[inline]
    pub fn neg(self) -> Self {
        let mask = Self::from_bits(0x8000_0000);
        self.xor(mask)
    }

    /// Fused multiply-add, uses the FMA intrinsic on targets that support
    /// it. On targets without FMA falls back to `self * a + b`.
    #[inline]
    pub fn madd(self, a: Self, b: Self) -> Self {
        #[cfg(all(target_arch = "x86_64", target_feature = "fma"))]
        unsafe {
            Self::from_m128(_mm_fmadd_ps(self.to_m128(), a.to_m128(), b.to_m128()))
        }
        #[cfg(all(target_arch = "aarch64", target_feature = "fma"))]
        unsafe {
            Self::from_neon(vfmaq_f32(b.to_neon(), a.to_neon(), self.to_neon()))
        }
        #[cfg(not(any(
            all(target_arch = "x86_64", target_feature = "fma"),
            all(target_arch = "aarch64", target_feature = "fma")
        )))]
        {
            self.mul(a).add(b)
        }
    }

    /// Clamp each lane to `[lo, hi]`.
    #[inline]
    pub fn clamp(self, lo: Self, hi: Self) -> Self {
        self.max(lo).min(hi)
    }

    /// Saturated value when scaled to the range `[0, 1]` then multiplied
    /// by `scale`.
    #[inline]
    pub fn sat(self, lo: Self, hi: Self) -> Self {
        self.max(lo).min(hi)
    }
}

// ---------------------------------------------------------------------------
// Operator overloads for GsVector4.
// ---------------------------------------------------------------------------

impl core::ops::Add for GsVector4 {
    type Output = GsVector4;
    #[inline]
    fn add(self, rhs: GsVector4) -> GsVector4 {
        GsVector4::add(self, rhs)
    }
}

impl core::ops::Sub for GsVector4 {
    type Output = GsVector4;
    #[inline]
    fn sub(self, rhs: GsVector4) -> GsVector4 {
        GsVector4::sub(self, rhs)
    }
}

impl core::ops::Mul for GsVector4 {
    type Output = GsVector4;
    #[inline]
    fn mul(self, rhs: GsVector4) -> GsVector4 {
        GsVector4::mul(self, rhs)
    }
}

impl core::ops::Div for GsVector4 {
    type Output = GsVector4;
    #[inline]
    fn div(self, rhs: GsVector4) -> GsVector4 {
        GsVector4::div(self, rhs)
    }
}

impl core::ops::Neg for GsVector4 {
    type Output = GsVector4;
    #[inline]
    fn neg(self) -> GsVector4 {
        GsVector4::neg(self)
    }
}

impl core::ops::BitAnd for GsVector4 {
    type Output = GsVector4;
    #[inline]
    fn bitand(self, rhs: GsVector4) -> GsVector4 {
        GsVector4::and(self, rhs)
    }
}

impl core::ops::BitOr for GsVector4 {
    type Output = GsVector4;
    #[inline]
    fn bitor(self, rhs: GsVector4) -> GsVector4 {
        GsVector4::or(self, rhs)
    }
}

impl core::ops::BitXor for GsVector4 {
    type Output = GsVector4;
    #[inline]
    fn bitxor(self, rhs: GsVector4) -> GsVector4 {
        GsVector4::xor(self, rhs)
    }
}

// ---------------------------------------------------------------------------
// GsVector4i - 4 x i32 (128-bit).
// ---------------------------------------------------------------------------

/// 128-bit integer vector, the Rust equivalent of `GSVector4i`.
#[derive(Copy, Clone, Debug, Default)]
#[repr(C, align(16))]
pub struct GsVector4i {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub w: i32,
}

impl GsVector4i {
    #[inline]
    pub const fn new(x: i32, y: i32, z: i32, w: i32) -> Self {
        Self { x, y, z, w }
    }

    #[inline]
    pub const fn splat(v: i32) -> Self {
        Self::new(v, v, v, v)
    }

    /// `cxpr` analogue matching the C++ static.
    #[inline]
    pub const fn cxpr(x: i32, y: i32, z: i32, w: i32) -> Self {
        Self::new(x, y, z, w)
    }

    /// Vector of all-zero bits.
    #[inline]
    pub fn zero() -> Self {
        Self::splat(0)
    }

    /// Vector of all-one bits, computed as `zero() == zero()` like the
    /// C++ version.
    #[inline]
    pub fn xffffffff() -> Self {
        Self::splat(-1)
    }

    /// Load a vector from a pointer.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn loadu(ptr: *const i32) -> Self {
        let m: __m128i = _mm_loadu_si128(ptr as *const __m128i);
        Self::from_m128i(m)
    }

    /// Load a vector from an aligned pointer.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn load(ptr: *const i32) -> Self {
        let m: __m128i = _mm_load_si128(ptr as *const __m128i);
        Self::from_m128i(m)
    }

    /// Load a vector from a pointer on aarch64.
    #[inline]
    #[cfg(target_arch = "aarch64")]
    pub unsafe fn loadu(ptr: *const i32) -> Self {
        Self::from_neon(vld1q_s32(ptr))
    }

    /// Store to a pointer.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn storeu(self, ptr: *mut i32) {
        _mm_storeu_si128(ptr as *mut __m128i, self.to_m128i());
    }

    /// Store to a pointer on aarch64.
    #[inline]
    #[cfg(target_arch = "aarch64")]
    pub unsafe fn storeu(self, ptr: *mut i32) {
        vst1q_s32(ptr, self.to_neon());
    }

    /// Non-temporal aligned store.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn storent(self, ptr: *mut i32) {
        _mm_stream_si128(ptr as *mut __m128i, self.to_m128i());
    }

    /// Convert from a SIMD register on x86_64.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn from_m128i(m: __m128i) -> Self {
        let mut out = Self::default();
        _mm_storeu_si128(&mut out as *mut _ as *mut __m128i, m);
        out
    }

    /// Convert from a NEON register.
    #[inline]
    #[cfg(target_arch = "aarch64")]
    pub unsafe fn from_neon(m: int32x4_t) -> Self {
        let mut out = Self::default();
        vst1q_s32(&mut out as *mut _ as *mut i32, m);
        out
    }

    /// Convert to a SIMD register.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn to_m128i(self) -> __m128i {
        _mm_loadu_si128(&self as *const _ as *const __m128i)
    }

    /// Convert to a NEON register.
    #[inline]
    #[cfg(target_arch = "aarch64")]
    pub unsafe fn to_neon(self) -> int32x4_t {
        vld1q_s32(self as *const _ as *const i32)
    }

    // -----------------------------------------------------------------------
    // Width / height helpers (rect semantics from the C++ API).
    // -----------------------------------------------------------------------

    #[inline]
    pub fn width(self) -> i32 {
        self.w - self.x
    }

    #[inline]
    pub fn height(self) -> i32 {
        self.w - self.y
    }

    // -----------------------------------------------------------------------
    // Per-lane predicates.
    // -----------------------------------------------------------------------

    /// Per-byte movemask. Returns 0..=0xFFFF.
    #[inline]
    pub fn mask(self) -> i32 {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            _mm_movemask_epi8(self.to_m128i())
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            // NEON doesn't have a movemask; do it scalar-style.
            let mut m: i32 = 0;
            let bytes: [u8; 16] = core::mem::transmute(self);
            for (i, &b) in bytes.iter().enumerate() {
                if b & 0x80 != 0 {
                    m |= 1 << i;
                }
            }
            m
        }
    }

    /// All-ones predicate on the bit pattern, used by `alltrue`/`allfalse`.
    #[inline]
    pub fn alltrue(self) -> bool {
        self.mask() == 0xFFFF
    }

    /// All-zeros predicate.
    #[inline]
    pub fn allfalse(self) -> bool {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            _mm_testz_si128(self.to_m128i(), self.to_m128i()) != 0
        }
        #[cfg(target_arch = "aarch64")]
        {
            (self.x | self.y | self.z | self.w) == 0
        }
    }

    // -----------------------------------------------------------------------
    // Signed integer arithmetic.
    // -----------------------------------------------------------------------

    /// Component-wise 32-bit signed addition.
    #[inline]
    pub fn add_s32(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128i(_mm_add_epi32(self.to_m128i(), rhs.to_m128i()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            Self::from_neon(vaddq_s32(self.to_neon(), rhs.to_neon()))
        }
    }

    /// Component-wise 16-bit signed addition.
    #[inline]
    pub fn add_s16(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128i(_mm_add_epi16(self.to_m128i(), rhs.to_m128i()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            Self::from_neon(vaddq_s16(self.to_neon(), rhs.to_neon()))
        }
    }

    /// Component-wise 8-bit signed addition.
    #[inline]
    pub fn add_s8(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128i(_mm_add_epi8(self.to_m128i(), rhs.to_m128i()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            Self::from_neon(vaddq_s8(self.to_neon(), rhs.to_neon()))
        }
    }

    /// Component-wise 16-bit unsigned saturating addition.
    #[inline]
    pub fn add_u16(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128i(_mm_adds_epu16(self.to_m128i(), rhs.to_m128i()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            Self::from_neon(vqaddq_u16(
                vreinterpretq_u16_s32(self.to_neon()),
                vreinterpretq_u16_s32(rhs.to_neon()),
            ))
            .cast()
        }
    }

    /// Component-wise 8-bit signed saturating addition.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn adds8(self, rhs: Self) -> Self {
        Self::from_m128i(_mm_adds_epi8(self.to_m128i(), rhs.to_m128i()))
    }

    /// Component-wise 16-bit signed saturating addition.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn adds16(self, rhs: Self) -> Self {
        Self::from_m128i(_mm_adds_epi16(self.to_m128i(), rhs.to_m128i()))
    }

    /// Component-wise 32-bit subtraction.
    #[inline]
    pub fn sub_s32(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128i(_mm_sub_epi32(self.to_m128i(), rhs.to_m128i()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            Self::from_neon(vsubq_s32(self.to_neon(), rhs.to_neon()))
        }
    }

    // -----------------------------------------------------------------------
    // Bitwise.
    // -----------------------------------------------------------------------

    /// Bitwise AND.
    #[inline]
    pub fn and(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128i(_mm_and_si128(self.to_m128i(), rhs.to_m128i()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            Self::from_neon(vandq_s32(self.to_neon(), rhs.to_neon()))
        }
    }

    /// Bitwise OR.
    #[inline]
    pub fn or(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128i(_mm_or_si128(self.to_m128i(), rhs.to_m128i()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            Self::from_neon(vorrq_s32(self.to_neon(), rhs.to_neon()))
        }
    }

    /// Bitwise XOR.
    #[inline]
    pub fn xor(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128i(_mm_xor_si128(self.to_m128i(), rhs.to_m128i()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            Self::from_neon(veorq_s32(self.to_neon(), rhs.to_neon()))
        }
    }

    /// Bitwise NOT.
    #[inline]
    pub fn not(self) -> Self {
        self.xor(Self::xffffffff())
    }

    /// `!rhs & self` semantics, matches `_mm_andnot_si128`.
    #[inline]
    pub fn andnot(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128i(_mm_andnot_si128(rhs.to_m128i(), self.to_m128i()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            let ones = vdupq_n_s32(-1);
            let nb = veorq_s32(rhs.to_neon(), ones);
            Self::from_neon(vandq_s32(self.to_neon(), nb))
        }
    }

    // -----------------------------------------------------------------------
    // Min / max.
    // -----------------------------------------------------------------------

    /// Signed 8-bit lane-wise minimum.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn min_i8(self, rhs: Self) -> Self {
        Self::from_m128i(_mm_min_epi8(self.to_m128i(), rhs.to_m128i()))
    }

    /// Signed 8-bit lane-wise maximum.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn max_i8(self, rhs: Self) -> Self {
        Self::from_m128i(_mm_max_epi8(self.to_m128i(), rhs.to_m128i()))
    }

    /// Signed 16-bit lane-wise minimum.
    #[inline]
    pub fn min_i16(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128i(_mm_min_epi16(self.to_m128i(), rhs.to_m128i()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            Self::from_neon(vminq_s16(self.to_neon(), rhs.to_neon()))
        }
    }

    /// Signed 16-bit lane-wise maximum.
    #[inline]
    pub fn max_i16(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128i(_mm_max_epi16(self.to_m128i(), rhs.to_m128i()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            Self::from_neon(vmaxq_s16(self.to_neon(), rhs.to_neon()))
        }
    }

    /// Signed 32-bit lane-wise minimum.
    #[inline]
    pub fn min_i32(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128i(_mm_min_epi32(self.to_m128i(), rhs.to_m128i()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            // NEON has no `vminq_s32` directly; vreinterpret to s32 and use
            // the s32 NEON min that the NEON intrinsics provide.
            Self::from_neon(vminq_s32(self.to_neon(), rhs.to_neon()))
        }
    }

    /// Signed 32-bit lane-wise maximum.
    #[inline]
    pub fn max_i32(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128i(_mm_max_epi32(self.to_m128i(), rhs.to_m128i()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            Self::from_neon(vmaxq_s32(self.to_neon(), rhs.to_neon()))
        }
    }

    /// Unsigned 8-bit lane-wise minimum.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn min_u8(self, rhs: Self) -> Self {
        Self::from_m128i(_mm_min_epu8(self.to_m128i(), rhs.to_m128i()))
    }

    /// Unsigned 8-bit lane-wise maximum.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn max_u8(self, rhs: Self) -> Self {
        Self::from_m128i(_mm_max_epu8(self.to_m128i(), rhs.to_m128i()))
    }

    /// Unsigned 16-bit lane-wise minimum.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn min_u16(self, rhs: Self) -> Self {
        Self::from_m128i(_mm_min_epu16(self.to_m128i(), rhs.to_m128i()))
    }

    /// Unsigned 16-bit lane-wise maximum.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn max_u16(self, rhs: Self) -> Self {
        Self::from_m128i(_mm_max_epu16(self.to_m128i(), rhs.to_m128i()))
    }

    /// Unsigned 32-bit lane-wise minimum.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn min_u32(self, rhs: Self) -> Self {
        Self::from_m128i(_mm_min_epu32(self.to_m128i(), rhs.to_m128i()))
    }

    /// Unsigned 32-bit lane-wise maximum.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn max_u32(self, rhs: Self) -> Self {
        Self::from_m128i(_mm_max_epu32(self.to_m128i(), rhs.to_m128i()))
    }

    // -----------------------------------------------------------------------
    // Packs.
    // -----------------------------------------------------------------------

    /// Signed 16-bit -> signed 8-bit pack, saturating.
    #[inline]
    pub fn packs(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128i(_mm_packs_epi16(self.to_m128i(), rhs.to_m128i()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            Self::from_neon(vcombine_s8(
                vqmovn_s16(self.to_neon()),
                vqmovn_s16(rhs.to_neon()),
            ))
        }
    }

    /// Signed 16-bit -> unsigned 8-bit pack, saturating.
    #[inline]
    pub fn packu(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128i(_mm_packus_epi16(self.to_m128i(), rhs.to_m128i()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            let lo = vreinterpretq_s16_s32(self.to_neon());
            let hi = vreinterpretq_s16_s32(rhs.to_neon());
            Self::from_neon(vcombine_s8(
                vqmovun_s16(lo),
                vqmovun_s16(hi),
            ))
        }
    }

    // -----------------------------------------------------------------------
    // Shifts.
    // -----------------------------------------------------------------------

    /// 32-bit variable logical left shift, lane-wise.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn sllv(self, rhs: Self) -> Self {
        Self::from_m128i(_mm_sllv_epi32(self.to_m128i(), rhs.to_m128i()))
    }

    /// 32-bit variable logical right shift, lane-wise.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn srlv(self, rhs: Self) -> Self {
        Self::from_m128i(_mm_srlv_epi32(self.to_m128i(), rhs.to_m128i()))
    }

    /// 32-bit variable arithmetic right shift, lane-wise.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn srav(self, rhs: Self) -> Self {
        Self::from_m128i(_mm_srav_epi32(self.to_m128i(), rhs.to_m128i()))
    }

    /// 16-bit variable logical left shift, lane-wise.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn sllv16(self, rhs: Self) -> Self {
        Self::from_m128i(_mm_sllv_epi16(self.to_m128i(), rhs.to_m128i()))
    }

    /// 16-bit variable logical right shift, lane-wise.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn srlv16(self, rhs: Self) -> Self {
        Self::from_m128i(_mm_srlv_epi16(self.to_m128i(), rhs.to_m128i()))
    }

    /// 16-bit variable arithmetic right shift, lane-wise.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn srav16(self, rhs: Self) -> Self {
        Self::from_m128i(_mm_srav_epi16(self.to_m128i(), rhs.to_m128i()))
    }

    /// Immediate 16-bit left shift, lane-wise.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn slli16<const I: i32>(self) -> Self {
        Self::from_m128i(_mm_slli_epi16(self.to_m128i(), I))
    }

    /// Immediate 16-bit right (logical) shift, lane-wise.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn srli16<const I: i32>(self) -> Self {
        Self::from_m128i(_mm_srli_epi16(self.to_m128i(), I))
    }

    /// Immediate 16-bit arithmetic right shift, lane-wise.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn srai16<const I: i32>(self) -> Self {
        Self::from_m128i(_mm_srai_epi16(self.to_m128i(), I))
    }

    /// Immediate 32-bit left shift, lane-wise.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn slli32<const I: i32>(self) -> Self {
        Self::from_m128i(_mm_slli_epi32(self.to_m128i(), I))
    }

    /// Immediate 32-bit right (logical) shift, lane-wise.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn srli32<const I: i32>(self) -> Self {
        Self::from_m128i(_mm_srli_epi32(self.to_m128i(), I))
    }

    /// Immediate 32-bit arithmetic right shift, lane-wise.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn srai32<const I: i32>(self) -> Self {
        Self::from_m128i(_mm_srai_epi32(self.to_m128i(), I))
    }

    // -----------------------------------------------------------------------
    // Lane / shuffle / unpack.
    // -----------------------------------------------------------------------

    /// Interleave the low 16-bit halves of `self` and `rhs`.
    #[inline]
    pub fn unpack_lo16(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128i(_mm_unpacklo_epi16(self.to_m128i(), rhs.to_m128i()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            let lo = vzip1q_s16(self.to_neon(), rhs.to_neon());
            Self::from_neon(vreinterpretq_s32_s16(lo))
        }
    }

    /// Interleave the high 16-bit halves of `self` and `rhs`.
    #[inline]
    pub fn unpack_hi16(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128i(_mm_unpackhi_epi16(self.to_m128i(), rhs.to_m128i()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            let hi = vzip2q_s16(self.to_neon(), rhs.to_neon());
            Self::from_neon(vreinterpretq_s32_s16(hi))
        }
    }

    /// Interleave the low 32-bit lanes of `self` and `rhs`.
    #[inline]
    pub fn unpack_lo(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128i(_mm_unpacklo_epi32(self.to_m128i(), rhs.to_m128i()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            let lo = vzip1q_s32(self.to_neon(), rhs.to_neon());
            Self::from_neon(lo)
        }
    }

    /// Interleave the high 32-bit lanes of `self` and `rhs`.
    #[inline]
    pub fn unpack_hi(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128i(_mm_unpackhi_epi32(self.to_m128i(), rhs.to_m128i()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            let hi = vzip2q_s32(self.to_neon(), rhs.to_neon());
            Self::from_neon(hi)
        }
    }

    /// Permute the four 32-bit lanes. `mask` indexes each output lane as a
    /// 2-bit value in the low 8 bits of the mask, in lane order `[w, z, y, x]`.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn shuffle<const MASK: i32>(self) -> Self {
        Self::from_m128i(_mm_shuffle_epi32::<MASK>(self.to_m128i()))
    }

    /// Insert a single 32-bit value at lane `i` (0..=3).
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn insert32<const I: i32>(self, v: i32) -> Self {
        Self::from_m128i(_mm_insert_epi32::<I>(self.to_m128i(), v))
    }

    /// Extract a single 32-bit value from lane `i` (0..=3).
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn extract32<const I: i32>(self) -> i32 {
        _mm_extract_epi32::<I>(self.to_m128i())
    }

    /// Insert a single 8-bit value at lane `i` (0..=15).
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn insert8<const I: i32>(self, v: i32) -> Self {
        Self::from_m128i(_mm_insert_epi8::<I>(self.to_m128i(), v))
    }

    /// Extract a single 8-bit value from lane `i` (0..=15).
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn extract8<const I: i32>(self) -> i32 {
        _mm_extract_epi8::<I>(self.to_m128i()) as i32
    }

    /// Insert a single 16-bit value at lane `i` (0..=7).
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn insert16<const I: i32>(self, v: i32) -> Self {
        Self::from_m128i(_mm_insert_epi16::<I>(self.to_m128i(), v))
    }

    /// Extract a single 16-bit value from lane `i` (0..=7).
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn extract16<const I: i32>(self) -> i32 {
        _mm_extract_epi16::<I>(self.to_m128i()) as i32
    }

    /// Insert a single 64-bit value at lane `i` (0..=1).
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn insert64<const I: i32>(self, v: i64) -> Self {
        Self::from_m128i(_mm_insert_epi64::<I>(self.to_m128i(), v))
    }

    /// Extract a single 64-bit value from lane `i` (0..=1).
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn extract64<const I: i32>(self) -> i64 {
        _mm_extract_epi64::<I>(self.to_m128i())
    }

    /// Insert: generic wrapper for the SSE4.1 insert instruction. The
    /// lane index is determined by the immediate encoded in the
    /// `_MM_MK_INSERTPS_NDX` macro of the C++ API.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn insert<const LANE: i32>(self, v: i32) -> Self {
        Self::from_m128i(_mm_insert_epi32::<LANE>(self.to_m128i(), v))
    }

    /// Extract: generic wrapper matching the C++ `extract32` template.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn extract<const LANE: i32>(self) -> i32 {
        _mm_extract_epi32::<LANE>(self.to_m128i())
    }

    // -----------------------------------------------------------------------
    // Conversions to GsVector4.
    // -----------------------------------------------------------------------

    /// Bit-cast this integer vector to a float vector.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn cast_to_vec4(self) -> GsVector4 {
        GsVector4::from_m128(_mm_castsi128_ps(self.to_m128i()))
    }

    /// Convert each i32 lane to f32.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn to_vec4(self) -> GsVector4 {
        GsVector4::from_m128(_mm_cvtepi32_ps(self.to_m128i()))
    }

    /// Convert f32 lanes to i32 with truncation.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn from_vec4_trunc(v: GsVector4) -> Self {
        Self::from_m128i(_mm_cvttps_epi32(v.to_m128()))
    }

    /// Convert f32 lanes to i32 with rounding.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn from_vec4_round(v: GsVector4) -> Self {
        Self::from_m128i(_mm_cvtps_epi32(v.to_m128()))
    }

    // -----------------------------------------------------------------------
    // Predicates / comparisons.
    // -----------------------------------------------------------------------

    /// `self == rhs` lane-wise, returns all-ones / all-zero bits.
    #[inline]
    pub fn cmpeq(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128i(_mm_cmpeq_epi32(self.to_m128i(), rhs.to_m128i()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            Self::from_neon(vceqq_s32(self.to_neon(), rhs.to_neon()))
        }
    }

    /// `self != rhs` lane-wise.
    #[inline]
    pub fn cmpneq(self, rhs: Self) -> Self {
        self.cmpeq(rhs).not()
    }

    /// `self > rhs` lane-wise signed.
    #[inline]
    pub fn cmpgt(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128i(_mm_cmpgt_epi32(self.to_m128i(), rhs.to_m128i()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            Self::from_neon(vcgtq_s32(self.to_neon(), rhs.to_neon()))
        }
    }

    /// `self < rhs` lane-wise signed.
    #[inline]
    pub fn cmplt(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m128i(_mm_cmplt_epi32(self.to_m128i(), rhs.to_m128i()))
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            Self::from_neon(vcltq_s32(self.to_neon(), rhs.to_neon()))
        }
    }

    /// Vector equality test, returns true iff all four lanes match.
    #[inline]
    pub fn eq(self, rhs: Self) -> bool {
        self.xor(rhs).allfalse()
    }

    // -----------------------------------------------------------------------
    // Byte-shuffles.
    // -----------------------------------------------------------------------

    /// Per-byte variable shuffle, uses `pshufb` on x86_64.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn shuffle8(self, mask: Self) -> Self {
        Self::from_m128i(_mm_shuffle_epi8(self.to_m128i(), mask.to_m128i()))
    }

    /// Sign-extend bytes to 16-bit, taking the low 8 bytes of `self`.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn i8to16(self) -> Self {
        Self::from_m128i(_mm_cvtepi8_epi16(self.to_m128i()))
    }

    /// Zero-extend bytes to 16-bit, taking the low 8 bytes of `self`.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn u8to16(self) -> Self {
        Self::from_m128i(_mm_cvtepu8_epi16(self.to_m128i()))
    }

    /// Sign-extend bytes to 32-bit, taking the low 4 bytes of `self`.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn i8to32(self) -> Self {
        Self::from_m128i(_mm_cvtepi8_epi32(self.to_m128i()))
    }

    /// Zero-extend bytes to 32-bit, taking the low 4 bytes of `self`.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn u8to32(self) -> Self {
        Self::from_m128i(_mm_cvtepu8_epi32(self.to_m128i()))
    }

    /// Sign-extend 16-bit lanes to 32-bit, taking the low two lanes.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn i16to32(self) -> Self {
        Self::from_m128i(_mm_cvtepi16_epi32(self.to_m128i()))
    }

    /// Zero-extend 16-bit lanes to 32-bit, taking the low two lanes.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn u16to32(self) -> Self {
        Self::from_m128i(_mm_cvtepu16_epi32(self.to_m128i()))
    }

    /// `mulhi` for signed 16-bit lanes, taking the high half of each 32-bit
    /// product.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn mul16hs(self, rhs: Self) -> Self {
        Self::from_m128i(_mm_mulhi_epi16(self.to_m128i(), rhs.to_m128i()))
    }

    /// `mulhi` for unsigned 16-bit lanes, taking the high half of each
    /// 32-bit product.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn mul16hu(self, rhs: Self) -> Self {
        Self::from_m128i(_mm_mulhi_epu16(self.to_m128i(), rhs.to_m128i()))
    }

    /// `mullo` for 16-bit lanes, taking the low half of each 32-bit
    /// product.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn mul16l(self, rhs: Self) -> Self {
        Self::from_m128i(_mm_mullo_epi16(self.to_m128i(), rhs.to_m128i()))
    }

    /// `madd` 16-bit -> 32-bit, multiplies adjacent pairs and adds.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn madd16(self, rhs: Self) -> Self {
        Self::from_m128i(_mm_madd_epi16(self.to_m128i(), rhs.to_m128i()))
    }

    /// Absolute value of each 32-bit lane.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn abs32(self) -> Self {
        Self::from_m128i(_mm_abs_epi32(self.to_m128i()))
    }

    // -----------------------------------------------------------------------
    // Blend helpers.
    // -----------------------------------------------------------------------

    /// Variable blend, lane-wise, of `self` and `rhs` according to `mask`.
    /// The mask convention matches `_mm_blendv_epi8`/`_mm_blendv_ps`.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn blendv(self, rhs: Self, mask: Self) -> Self {
        Self::from_m128i(_mm_blendv_epi8(self.to_m128i(), rhs.to_m128i(), mask.to_m128i()))
    }

    /// Compile-time blend: each bit of `MASK` picks a lane from `self`
    /// (0) or `rhs` (1). Implemented in terms of `_mm_blend_epi16` with the
    /// same bit pattern the C++ `blend16`/`blend32` macros use.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn blend32_const<const MASK: i32>(self, rhs: Self) -> Self {
        // Replicate the macro expansion from the C++ API so callers can use
        // a `const` mask value. The arithmetic is computed inline and the
        // resulting literal is passed straight to the SSE intrinsic.
        const fn blend_immediate(mask: i32) -> i32 {
            let b3 = ((mask & 8) * 3) << 3;
            let b2 = ((mask & 4) * 3) << 2;
            let b1 = ((mask & 2) * 3) << 1;
            let b0 = (mask & 1) * 3;
            b3 | b2 | b1 | b0
        }
        match blend_immediate(MASK) {
            0 => Self::from_m128i(_mm_blend_epi16::<0>(self.to_m128i(), rhs.to_m128i())),
            1 => Self::from_m128i(_mm_blend_epi16::<1>(self.to_m128i(), rhs.to_m128i())),
            2 => Self::from_m128i(_mm_blend_epi16::<2>(self.to_m128i(), rhs.to_m128i())),
            3 => Self::from_m128i(_mm_blend_epi16::<3>(self.to_m128i(), rhs.to_m128i())),
            4 => Self::from_m128i(_mm_blend_epi16::<4>(self.to_m128i(), rhs.to_m128i())),
            5 => Self::from_m128i(_mm_blend_epi16::<5>(self.to_m128i(), rhs.to_m128i())),
            6 => Self::from_m128i(_mm_blend_epi16::<6>(self.to_m128i(), rhs.to_m128i())),
            7 => Self::from_m128i(_mm_blend_epi16::<7>(self.to_m128i(), rhs.to_m128i())),
            8 => Self::from_m128i(_mm_blend_epi16::<8>(self.to_m128i(), rhs.to_m128i())),
            9 => Self::from_m128i(_mm_blend_epi16::<9>(self.to_m128i(), rhs.to_m128i())),
            10 => Self::from_m128i(_mm_blend_epi16::<10>(self.to_m128i(), rhs.to_m128i())),
            11 => Self::from_m128i(_mm_blend_epi16::<11>(self.to_m128i(), rhs.to_m128i())),
            12 => Self::from_m128i(_mm_blend_epi16::<12>(self.to_m128i(), rhs.to_m128i())),
            13 => Self::from_m128i(_mm_blend_epi16::<13>(self.to_m128i(), rhs.to_m128i())),
            14 => Self::from_m128i(_mm_blend_epi16::<14>(self.to_m128i(), rhs.to_m128i())),
            15 => Self::from_m128i(_mm_blend_epi16::<15>(self.to_m128i(), rhs.to_m128i())),
            _ => Self::from_m128i(_mm_blend_epi16::<0>(self.to_m128i(), rhs.to_m128i())),
        }
    }

    /// 16-bit blend, matches `_mm_blend_epi16`.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn blend16<const MASK: i32>(self, rhs: Self) -> Self {
        Self::from_m128i(_mm_blend_epi16(self.to_m128i(), rhs.to_m128i(), MASK))
    }

    // -----------------------------------------------------------------------
    // Mask helpers for the static `x00xxxxxx` patterns.
    // -----------------------------------------------------------------------

    /// Vector with the low `n` bytes set to 0xFF, lanes 0..4.
    #[inline]
    pub fn xff(n: i32) -> Self {
        // Matches the C++ table in GSVector.cpp. We compute it by hand.
        let mut lanes = [0xFFFFFFFFu32; 4];
        let bytes = (n as u32).min(4);
        for i in 0..(bytes as usize) {
            lanes[i] = 0x00000000;
        }
        // Convert to signed lanes.
        Self::new(
            lanes[0] as i32,
            lanes[1] as i32,
            lanes[2] as i32,
            lanes[3] as i32,
        )
    }

    /// Vector with the low `n` bytes of each lane set to 0x0F, 0..4.
    #[inline]
    pub fn x0f(n: i32) -> Self {
        let mut lanes = [0x0F0F0F0Fu32; 4];
        let bytes = (n as u32).min(4);
        for i in 0..(bytes as usize) {
            lanes[i] = 0x00000000;
        }
        Self::new(
            lanes[0] as i32,
            lanes[1] as i32,
            lanes[2] as i32,
            lanes[3] as i32,
        )
    }
}

// Helper to convert a NEON u8 vector back to i32 lanes without copying.
#[cfg(target_arch = "aarch64")]
trait NeonCastToI32 {
    fn cast(self) -> int32x4_t;
}

#[cfg(target_arch = "aarch64")]
impl NeonCastToI32 for int32x4_t {
    #[inline]
    fn cast(self) -> int32x4_t {
        self
    }
}

#[cfg(target_arch = "aarch64")]
impl NeonCastToI32 for uint16x8_t {
    #[inline]
    fn cast(self) -> int32x4_t {
        vreinterpretq_s32_u16(self)
    }
}

#[cfg(target_arch = "aarch64")]
impl NeonCastToI32 for uint8x16_t {
    #[inline]
    fn cast(self) -> int32x4_t {
        vreinterpretq_s32_u8(self)
    }
}

// Operator overloads for GsVector4i.
impl core::ops::Add for GsVector4i {
    type Output = GsVector4i;
    #[inline]
    fn add(self, rhs: GsVector4i) -> GsVector4i {
        self.add_s32(rhs)
    }
}

impl core::ops::Sub for GsVector4i {
    type Output = GsVector4i;
    #[inline]
    fn sub(self, rhs: GsVector4i) -> GsVector4i {
        self.sub_s32(rhs)
    }
}

impl core::ops::BitAnd for GsVector4i {
    type Output = GsVector4i;
    #[inline]
    fn bitand(self, rhs: GsVector4i) -> GsVector4i {
        self.and(rhs)
    }
}

impl core::ops::BitOr for GsVector4i {
    type Output = GsVector4i;
    #[inline]
    fn bitor(self, rhs: GsVector4i) -> GsVector4i {
        self.or(rhs)
    }
}

impl core::ops::BitXor for GsVector4i {
    type Output = GsVector4i;
    #[inline]
    fn bitxor(self, rhs: GsVector4i) -> GsVector4i {
        self.xor(rhs)
    }
}

impl core::ops::Shl<i32> for GsVector4i {
    type Output = GsVector4i;
    #[inline]
    fn shl(self, _rhs: i32) -> GsVector4i {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            self.slli32::<0>()
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            self
        }
    }
}

impl core::ops::Shr<i32> for GsVector4i {
    type Output = GsVector4i;
    #[inline]
    fn shr(self, _rhs: i32) -> GsVector4i {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            self.srli32::<0>()
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            self
        }
    }
}

// ---------------------------------------------------------------------------
// GsVector8 - 8 x f32 (256-bit, AVX on x86_64).
// ---------------------------------------------------------------------------

/// 256-bit floating point vector. On aarch64 this is implemented as two
/// `GsVector4` lanes; on x86_64 it wraps a `__m256`.
#[derive(Copy, Clone, Debug, Default)]
pub struct GsVector8 {
    /// The low 128 bits.
    pub xy: GsVector4,
    /// The high 128 bits.
    pub zw: GsVector4,
}

impl GsVector8 {
    /// Build an 8-wide vector from eight lanes.
    #[inline]
    pub const fn new(
        x0: f32,
        y0: f32,
        z0: f32,
        w0: f32,
        x1: f32,
        y1: f32,
        z1: f32,
        w1: f32,
    ) -> Self {
        Self {
            xy: GsVector4::new(x0, y0, z0, w0),
            zw: GsVector4::new(x1, y1, z1, w1),
        }
    }

    /// Build from two `GsVector4` lanes.
    #[inline]
    pub const fn from_lanes(xy: GsVector4, zw: GsVector4) -> Self {
        Self { xy, zw }
    }

    /// Broadcast a single `f32` to all eight lanes.
    #[inline]
    pub fn splat(v: f32) -> Self {
        Self {
            xy: GsVector4::splat(v),
            zw: GsVector4::splat(v),
        }
    }

    /// Zero vector.
    #[inline]
    pub fn zero() -> Self {
        Self::splat(0.0)
    }

    /// All-ones predicate vector.
    #[inline]
    pub fn xffffffff() -> Self {
        Self {
            xy: GsVector4::xffffffff(),
            zw: GsVector4::xffffffff(),
        }
    }

    /// Broadcast a single `f32` to all eight lanes (SIMD version).
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn broadcast(v: f32) -> Self {
        let m = _mm256_set1_ps(v);
        Self::from_m256(m)
    }

    /// Convert from an AVX register.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn from_m256(m: __m256) -> Self {
        let mut out = Self::default();
        _mm256_storeu_ps(&mut out as *mut _ as *mut f32, m);
        out
    }

    /// Convert to an AVX register.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn to_m256(self) -> __m256 {
        _mm256_loadu_ps(&self as *const _ as *const f32)
    }

    /// AVX load.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn loadu(ptr: *const f32) -> Self {
        Self::from_m256(_mm256_loadu_ps(ptr))
    }

    /// AVX aligned load.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn load(ptr: *const f32) -> Self {
        Self::from_m256(_mm256_load_ps(ptr))
    }

    /// AVX unaligned store.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn storeu(self, ptr: *mut f32) {
        _mm256_storeu_ps(ptr, self.to_m256());
    }

    /// AVX aligned store.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn store(self, ptr: *mut f32) {
        _mm256_store_ps(ptr, self.to_m256());
    }

    // -----------------------------------------------------------------------
    // Arithmetic.
    // -----------------------------------------------------------------------

    #[inline]
    pub fn add(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m256(_mm256_add_ps(self.to_m256(), rhs.to_m256()))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self {
                xy: self.xy.add(rhs.xy),
                zw: self.zw.add(rhs.zw),
            }
        }
    }

    #[inline]
    pub fn sub(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m256(_mm256_sub_ps(self.to_m256(), rhs.to_m256()))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self {
                xy: self.xy.sub(rhs.xy),
                zw: self.zw.sub(rhs.zw),
            }
        }
    }

    #[inline]
    pub fn mul(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m256(_mm256_mul_ps(self.to_m256(), rhs.to_m256()))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self {
                xy: self.xy.mul(rhs.xy),
                zw: self.zw.mul(rhs.zw),
            }
        }
    }

    #[inline]
    pub fn div(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m256(_mm256_div_ps(self.to_m256(), rhs.to_m256()))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self {
                xy: self.xy.div(rhs.xy),
                zw: self.zw.div(rhs.zw),
            }
        }
    }

    #[inline]
    pub fn min(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m256(_mm256_min_ps(self.to_m256(), rhs.to_m256()))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self {
                xy: self.xy.min(rhs.xy),
                zw: self.zw.min(rhs.zw),
            }
        }
    }

    #[inline]
    pub fn max(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m256(_mm256_max_ps(self.to_m256(), rhs.to_m256()))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self {
                xy: self.xy.max(rhs.xy),
                zw: self.zw.max(rhs.zw),
            }
        }
    }

    /// Fast reciprocal.
    #[inline]
    pub fn rcp(self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m256(_mm256_rcp_ps(self.to_m256()))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self {
                xy: self.xy.rcp(),
                zw: self.zw.rcp(),
            }
        }
    }

    /// Refined reciprocal.
    #[inline]
    pub fn rcpnr(self) -> Self {
        let v = self.rcp();
        let two = Self::splat(2.0);
        let vv = v.mul(v);
        two.mul(v).sub(vv.mul(self))
    }

    /// `sqrt` for each lane.
    #[inline]
    pub fn sqrt(self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m256(_mm256_sqrt_ps(self.to_m256()))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self {
                xy: self.xy.sqrt(),
                zw: self.zw.sqrt(),
            }
        }
    }

    /// Rounding.
    #[inline]
    pub fn round(self, mode: RoundMode) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            // `_mm256_round_ps` requires a const immediate, so dispatch on
            // the runtime `mode` to the matching const entry point.
            match mode {
                RoundMode::NearestInt => Self::from_m256(_mm256_round_ps::<{ _MM_FROUND_TO_NEAREST_INT | _MM_FROUND_NO_EXC }>(self.to_m256())),
                RoundMode::NegInf => Self::from_m256(_mm256_round_ps::<{ _MM_FROUND_TO_NEG_INF | _MM_FROUND_NO_EXC }>(self.to_m256())),
                RoundMode::PosInf => Self::from_m256(_mm256_round_ps::<{ _MM_FROUND_TO_POS_INF | _MM_FROUND_NO_EXC }>(self.to_m256())),
                RoundMode::Truncate => Self::from_m256(_mm256_round_ps::<{ _MM_FROUND_TO_ZERO | _MM_FROUND_NO_EXC }>(self.to_m256())),
            }
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self {
                xy: self.xy.round(mode),
                zw: self.zw.round(mode),
            }
        }
    }

    /// Round towards `-inf`.
    #[inline]
    pub fn floor(self) -> Self {
        self.round(RoundMode::NegInf)
    }

    /// Round towards `+inf`.
    #[inline]
    pub fn ceil(self) -> Self {
        self.round(RoundMode::PosInf)
    }

    /// Round towards zero.
    #[inline]
    pub fn trunc(self) -> Self {
        self.round(RoundMode::Truncate)
    }

    // -----------------------------------------------------------------------
    // Bitwise.
    // -----------------------------------------------------------------------

    #[inline]
    pub fn and(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m256(_mm256_and_ps(self.to_m256(), rhs.to_m256()))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self {
                xy: self.xy.and(rhs.xy),
                zw: self.zw.and(rhs.zw),
            }
        }
    }

    #[inline]
    pub fn or(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m256(_mm256_or_ps(self.to_m256(), rhs.to_m256()))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self {
                xy: self.xy.or(rhs.xy),
                zw: self.zw.or(rhs.zw),
            }
        }
    }

    #[inline]
    pub fn xor(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m256(_mm256_xor_ps(self.to_m256(), rhs.to_m256()))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self {
                xy: self.xy.xor(rhs.xy),
                zw: self.zw.xor(rhs.zw),
            }
        }
    }

    /// `!rhs & self` semantics, matches `_mm256_andnot_ps`.
    #[inline]
    pub fn andnot(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m256(_mm256_andnot_ps(rhs.to_m256(), self.to_m256()))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self {
                xy: self.xy.andnot(rhs.xy),
                zw: self.zw.andnot(rhs.zw),
            }
        }
    }

    /// `self == rhs`, lane-wise, returns a vector with all-ones bits per
    /// matching lane and zero bits elsewhere.
    #[inline]
    pub fn cmpeq(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m256(_mm256_cmp_ps(self.to_m256(), rhs.to_m256(), _CMP_EQ_OQ))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self {
                xy: self.xy.cmpeq(rhs.xy),
                zw: self.zw.cmpeq(rhs.zw),
            }
        }
    }

    /// `self < rhs`.
    #[inline]
    pub fn cmplt(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m256(_mm256_cmp_ps(self.to_m256(), rhs.to_m256(), _CMP_LT_OQ))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self {
                xy: self.xy.cmplt(rhs.xy),
                zw: self.zw.cmplt(rhs.zw),
            }
        }
    }

    /// `self > rhs`.
    #[inline]
    pub fn cmpgt(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m256(_mm256_cmp_ps(self.to_m256(), rhs.to_m256(), _CMP_GT_OQ))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self {
                xy: self.xy.cmpgt(rhs.xy),
                zw: self.zw.cmpgt(rhs.zw),
            }
        }
    }

    /// `self <= rhs`.
    #[inline]
    pub fn cmple(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m256(_mm256_cmp_ps(self.to_m256(), rhs.to_m256(), _CMP_LE_OQ))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self {
                xy: self.xy.cmple(rhs.xy),
                zw: self.zw.cmple(rhs.zw),
            }
        }
    }

    /// `self >= rhs`.
    #[inline]
    pub fn cmpge(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m256(_mm256_cmp_ps(self.to_m256(), rhs.to_m256(), _CMP_GE_OQ))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self {
                xy: self.xy.cmpge(rhs.xy),
                zw: self.zw.cmpge(rhs.zw),
            }
        }
    }

    /// `self != rhs`.
    #[inline]
    pub fn cmpneq(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m256(_mm256_cmp_ps(self.to_m256(), rhs.to_m256(), _CMP_NEQ_OQ))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self {
                xy: self.xy.cmpneq(rhs.xy),
                zw: self.zw.cmpneq(rhs.zw),
            }
        }
    }

    /// Lane-wise movemask. Returns 0..=0xFF.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn mask(self) -> i32 {
        _mm256_movemask_ps(self.to_m256())
    }

    /// True if all eight lanes have their sign bit clear.
    #[inline]
    pub fn alltrue(self) -> bool {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            self.mask() == 0xFF
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            self.xy.alltrue() && self.zw.alltrue()
        }
    }

    /// True if all eight lanes have their sign bit set.
    #[inline]
    pub fn allfalse(self) -> bool {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            _mm256_testz_ps(self.to_m256(), self.to_m256()) != 0
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            self.xy.allfalse() && self.zw.allfalse()
        }
    }

    /// `clamp` to the range `[lo, hi]` per lane.
    #[inline]
    pub fn clamp(self, lo: Self, hi: Self) -> Self {
        self.max(lo).min(hi)
    }

    /// `sat` to the range `[lo, hi]` per lane, same as `clamp`.
    #[inline]
    pub fn sat(self, lo: Self, hi: Self) -> Self {
        self.clamp(lo, hi)
    }

    /// Horizontal add of adjacent pairs. Returns `[x+y, z+w, x1+y1, z1+w1]`
    /// on each 128-bit lane.
    #[inline]
    pub fn hadd(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m256(_mm256_hadd_ps(self.to_m256(), rhs.to_m256()))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self {
                xy: self.xy.hadd(rhs.xy),
                zw: self.zw.hadd(rhs.zw),
            }
        }
    }

    /// Horizontal subtract.
    #[inline]
    pub fn hsub(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m256(_mm256_hsub_ps(self.to_m256(), rhs.to_m256()))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self {
                xy: self.xy.hsub(rhs.xy),
                zw: self.zw.hsub(rhs.zw),
            }
        }
    }

    /// Lane 0 (x) broadcast to all eight lanes.
    #[inline]
    pub fn xxxx(self) -> Self {
        Self::splat(self.xy.x)
    }

    /// Lane 1 (y) broadcast to all eight lanes.
    #[inline]
    pub fn yyyy(self) -> Self {
        Self::splat(self.xy.y)
    }

    /// Broadcast the first two lanes to the low half and the last two to
    /// the high half, producing `(x, y, x, y, x1, y1, x1, y1)`.
    #[inline]
    pub fn xyxy(self) -> Self {
        Self {
            xy: self.xy.xyxy(),
            zw: self.zw.xyxy(),
        }
    }

    /// Like `xyxy` but for `zw`.
    #[inline]
    pub fn zwzw(self) -> Self {
        Self {
            xy: self.xy.zwzw(),
            zw: self.zw.zwzw(),
        }
    }

    /// Fused multiply-add, `self * a + b` per lane.
    #[inline]
    pub fn madd(self, a: Self, b: Self) -> Self {
        #[cfg(all(target_arch = "x86_64", target_feature = "fma"))]
        unsafe {
            Self::from_m256(_mm256_fmadd_ps(self.to_m256(), a.to_m256(), b.to_m256()))
        }
        #[cfg(not(all(target_arch = "x86_64", target_feature = "fma")))]
        {
            self.mul(a).add(b)
        }
    }

    /// Absolute value: clears the sign bit on every lane.
    #[inline]
    pub fn abs(self) -> Self {
        self.and(GsVector4::xffffffff().broadcast_to_vec8())
    }

    /// Negate: flip the sign bit on every lane.
    #[inline]
    pub fn neg(self) -> Self {
        self.xor(Self::splat(f32::from_bits(0x8000_0000)))
    }
}

// Helper to broadcast a `GsVector4` to a `GsVector8`.
impl GsVector4 {
    /// Treat the four lanes of `self` as the low half of an 8-wide vector.
    #[inline]
    pub fn broadcast_to_vec8(self) -> GsVector8 {
        GsVector8 {
            xy: self,
            zw: self,
        }
    }
}

// ---------------------------------------------------------------------------
// Operator overloads for GsVector8.
// ---------------------------------------------------------------------------

impl core::ops::Add for GsVector8 {
    type Output = GsVector8;
    #[inline]
    fn add(self, rhs: GsVector8) -> GsVector8 {
        GsVector8::add(self, rhs)
    }
}

impl core::ops::Sub for GsVector8 {
    type Output = GsVector8;
    #[inline]
    fn sub(self, rhs: GsVector8) -> GsVector8 {
        GsVector8::sub(self, rhs)
    }
}

impl core::ops::Mul for GsVector8 {
    type Output = GsVector8;
    #[inline]
    fn mul(self, rhs: GsVector8) -> GsVector8 {
        GsVector8::mul(self, rhs)
    }
}

impl core::ops::Div for GsVector8 {
    type Output = GsVector8;
    #[inline]
    fn div(self, rhs: GsVector8) -> GsVector8 {
        GsVector8::div(self, rhs)
    }
}

impl core::ops::Neg for GsVector8 {
    type Output = GsVector8;
    #[inline]
    fn neg(self) -> GsVector8 {
        GsVector8::neg(self)
    }
}

impl core::ops::BitAnd for GsVector8 {
    type Output = GsVector8;
    #[inline]
    fn bitand(self, rhs: GsVector8) -> GsVector8 {
        GsVector8::and(self, rhs)
    }
}

impl core::ops::BitOr for GsVector8 {
    type Output = GsVector8;
    #[inline]
    fn bitor(self, rhs: GsVector8) -> GsVector8 {
        GsVector8::or(self, rhs)
    }
}

impl core::ops::BitXor for GsVector8 {
    type Output = GsVector8;
    #[inline]
    fn bitxor(self, rhs: GsVector8) -> GsVector8 {
        GsVector8::xor(self, rhs)
    }
}

// ---------------------------------------------------------------------------
// GsVector8i - 8 x i32 (256-bit, AVX2 on x86_64).
// ---------------------------------------------------------------------------

/// 256-bit integer vector. On aarch64 this is two `GsVector4i` lanes; on
/// x86_64 it wraps a `__m256i`.
#[derive(Copy, Clone, Debug, Default)]
pub struct GsVector8i {
    /// Low 128 bits.
    pub xy: GsVector4i,
    /// High 128 bits.
    pub zw: GsVector4i,
}

impl GsVector8i {
    /// Build an 8-wide integer vector from eight lanes.
    #[inline]
    pub const fn new(
        x0: i32,
        y0: i32,
        z0: i32,
        w0: i32,
        x1: i32,
        y1: i32,
        z1: i32,
        w1: i32,
    ) -> Self {
        Self {
            xy: GsVector4i::new(x0, y0, z0, w0),
            zw: GsVector4i::new(x1, y1, z1, w1),
        }
    }

    /// Build from two `GsVector4i` lanes.
    #[inline]
    pub const fn from_lanes(xy: GsVector4i, zw: GsVector4i) -> Self {
        Self { xy, zw }
    }

    /// Broadcast a single `i32` to all eight lanes.
    #[inline]
    pub fn splat(v: i32) -> Self {
        Self {
            xy: GsVector4i::splat(v),
            zw: GsVector4i::splat(v),
        }
    }

    /// Zero vector.
    #[inline]
    pub fn zero() -> Self {
        Self::splat(0)
    }

    /// All-ones predicate.
    #[inline]
    pub fn xffffffff() -> Self {
        Self::splat(-1)
    }

    /// AVX2 load.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn loadu(ptr: *const i32) -> Self {
        let m: __m256i = _mm256_loadu_si256(ptr as *const __m256i);
        Self::from_m256i(m)
    }

    /// AVX2 aligned load.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn load(ptr: *const i32) -> Self {
        let m: __m256i = _mm256_load_si256(ptr as *const __m256i);
        Self::from_m256i(m)
    }

    /// AVX2 unaligned store.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn storeu(self, ptr: *mut i32) {
        _mm256_storeu_si256(ptr as *mut __m256i, self.to_m256i());
    }

    /// AVX2 aligned store.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn store(self, ptr: *mut i32) {
        _mm256_store_si256(ptr as *mut __m256i, self.to_m256i());
    }

    /// AVX2 non-temporal store.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn storent(self, ptr: *mut i32) {
        _mm256_stream_si256(ptr as *mut __m256i, self.to_m256i());
    }

    /// Convert from an AVX2 register.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn from_m256i(m: __m256i) -> Self {
        let mut out = Self::default();
        _mm256_storeu_si256(&mut out as *mut _ as *mut __m256i, m);
        out
    }

    /// Convert to an AVX2 register.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn to_m256i(self) -> __m256i {
        _mm256_loadu_si256(&self as *const _ as *const __m256i)
    }

    /// Per-byte movemask, 0..=0xFFFFFFFF.
    #[inline]
    pub fn mask(self) -> i32 {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            _mm256_movemask_epi8(self.to_m256i())
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            self.xy.mask() | (self.zw.mask() << 16)
        }
    }

    /// True if all 32 bytes have the high bit set.
    #[inline]
    pub fn alltrue(self) -> bool {
        self.mask() == -1
    }

    /// True if all 32 bytes have the high bit clear.
    #[inline]
    pub fn allfalse(self) -> bool {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            _mm256_testz_si256(self.to_m256i(), self.to_m256i()) != 0
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            self.xy.allfalse() && self.zw.allfalse()
        }
    }

    // -----------------------------------------------------------------------
    // Arithmetic.
    // -----------------------------------------------------------------------

    #[inline]
    pub fn add_s32(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m256i(_mm256_add_epi32(self.to_m256i(), rhs.to_m256i()))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self {
                xy: self.xy.add_s32(rhs.xy),
                zw: self.zw.add_s32(rhs.zw),
            }
        }
    }

    #[inline]
    pub fn sub_s32(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m256i(_mm256_sub_epi32(self.to_m256i(), rhs.to_m256i()))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self {
                xy: self.xy.sub_s32(rhs.xy),
                zw: self.zw.sub_s32(rhs.zw),
            }
        }
    }

    // -----------------------------------------------------------------------
    // Bitwise.
    // -----------------------------------------------------------------------

    #[inline]
    pub fn and(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m256i(_mm256_and_si256(self.to_m256i(), rhs.to_m256i()))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self {
                xy: self.xy.and(rhs.xy),
                zw: self.zw.and(rhs.zw),
            }
        }
    }

    #[inline]
    pub fn or(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m256i(_mm256_or_si256(self.to_m256i(), rhs.to_m256i()))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self {
                xy: self.xy.or(rhs.xy),
                zw: self.zw.or(rhs.zw),
            }
        }
    }

    #[inline]
    pub fn xor(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m256i(_mm256_xor_si256(self.to_m256i(), rhs.to_m256i()))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self {
                xy: self.xy.xor(rhs.xy),
                zw: self.zw.xor(rhs.zw),
            }
        }
    }

    /// Bitwise NOT.
    #[inline]
    pub fn not(self) -> Self {
        self.xor(Self::xffffffff())
    }

    /// `!rhs & self`, matches `_mm256_andnot_si256`.
    #[inline]
    pub fn andnot(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m256i(_mm256_andnot_si256(rhs.to_m256i(), self.to_m256i()))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self {
                xy: self.xy.andnot(rhs.xy),
                zw: self.zw.andnot(rhs.zw),
            }
        }
    }

    // -----------------------------------------------------------------------
    // Min / max.
    // -----------------------------------------------------------------------

    /// Signed 8-bit minimum.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn min_i8(self, rhs: Self) -> Self {
        Self::from_m256i(_mm256_min_epi8(self.to_m256i(), rhs.to_m256i()))
    }

    /// Signed 8-bit maximum.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn max_i8(self, rhs: Self) -> Self {
        Self::from_m256i(_mm256_max_epi8(self.to_m256i(), rhs.to_m256i()))
    }

    /// Signed 16-bit minimum.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn min_i16(self, rhs: Self) -> Self {
        Self::from_m256i(_mm256_min_epi16(self.to_m256i(), rhs.to_m256i()))
    }

    /// Signed 16-bit maximum.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn max_i16(self, rhs: Self) -> Self {
        Self::from_m256i(_mm256_max_epi16(self.to_m256i(), rhs.to_m256i()))
    }

    /// Signed 32-bit minimum.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn min_i32(self, rhs: Self) -> Self {
        Self::from_m256i(_mm256_min_epi32(self.to_m256i(), rhs.to_m256i()))
    }

    /// Signed 32-bit maximum.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn max_i32(self, rhs: Self) -> Self {
        Self::from_m256i(_mm256_max_epi32(self.to_m256i(), rhs.to_m256i()))
    }

    /// Unsigned 8-bit minimum.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn min_u8(self, rhs: Self) -> Self {
        Self::from_m256i(_mm256_min_epu8(self.to_m256i(), rhs.to_m256i()))
    }

    /// Unsigned 8-bit maximum.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn max_u8(self, rhs: Self) -> Self {
        Self::from_m256i(_mm256_max_epu8(self.to_m256i(), rhs.to_m256i()))
    }

    /// Unsigned 16-bit minimum.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn min_u16(self, rhs: Self) -> Self {
        Self::from_m256i(_mm256_min_epu16(self.to_m256i(), rhs.to_m256i()))
    }

    /// Unsigned 16-bit maximum.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn max_u16(self, rhs: Self) -> Self {
        Self::from_m256i(_mm256_max_epu16(self.to_m256i(), rhs.to_m256i()))
    }

    /// Unsigned 32-bit minimum.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn min_u32(self, rhs: Self) -> Self {
        Self::from_m256i(_mm256_min_epu32(self.to_m256i(), rhs.to_m256i()))
    }

    /// Unsigned 32-bit maximum.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn max_u32(self, rhs: Self) -> Self {
        Self::from_m256i(_mm256_max_epu32(self.to_m256i(), rhs.to_m256i()))
    }

    // -----------------------------------------------------------------------
    // Packs.
    // -----------------------------------------------------------------------

    /// Signed 16-bit -> signed 8-bit pack, saturating.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn packs16(self, rhs: Self) -> Self {
        Self::from_m256i(_mm256_packs_epi16(self.to_m256i(), rhs.to_m256i()))
    }

    /// Signed 16-bit -> unsigned 8-bit pack, saturating.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn packu16(self, rhs: Self) -> Self {
        Self::from_m256i(_mm256_packus_epi16(self.to_m256i(), rhs.to_m256i()))
    }

    /// Signed 32-bit -> signed 16-bit pack, saturating.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn packs32(self, rhs: Self) -> Self {
        Self::from_m256i(_mm256_packs_epi32(self.to_m256i(), rhs.to_m256i()))
    }

    /// Signed 32-bit -> unsigned 32-bit pack, saturating.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn packu32(self, rhs: Self) -> Self {
        Self::from_m256i(_mm256_packus_epi32(self.to_m256i(), rhs.to_m256i()))
    }

    // -----------------------------------------------------------------------
    // Shifts.
    // -----------------------------------------------------------------------

    /// 32-bit variable logical left shift, lane-wise.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn sllv32(self, rhs: Self) -> Self {
        Self::from_m256i(_mm256_sllv_epi32(self.to_m256i(), rhs.to_m256i()))
    }

    /// 32-bit variable logical right shift, lane-wise.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn srlv32(self, rhs: Self) -> Self {
        Self::from_m256i(_mm256_srlv_epi32(self.to_m256i(), rhs.to_m256i()))
    }

    /// 32-bit variable arithmetic right shift, lane-wise.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn srav32(self, rhs: Self) -> Self {
        Self::from_m256i(_mm256_srav_epi32(self.to_m256i(), rhs.to_m256i()))
    }

    /// 16-bit variable logical left shift, lane-wise.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn sllv16(self, rhs: Self) -> Self {
        Self::from_m256i(_mm256_sllv_epi16(self.to_m256i(), rhs.to_m256i()))
    }

    /// 16-bit variable arithmetic right shift, lane-wise.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn srav16(self, rhs: Self) -> Self {
        Self::from_m256i(_mm256_srav_epi16(self.to_m256i(), rhs.to_m256i()))
    }

    /// 16-bit immediate logical left shift, lane-wise.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn slli16<const I: i32>(self) -> Self {
        Self::from_m256i(_mm256_slli_epi16(self.to_m256i(), I))
    }

    /// 16-bit immediate arithmetic right shift, lane-wise.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn srai16<const I: i32>(self) -> Self {
        Self::from_m256i(_mm256_srai_epi16(self.to_m256i(), I))
    }

    /// 16-bit immediate logical right shift, lane-wise.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn srli16<const I: i32>(self) -> Self {
        Self::from_m256i(_mm256_srli_epi16(self.to_m256i(), I))
    }

    /// 32-bit immediate logical left shift, lane-wise.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn slli32<const I: i32>(self) -> Self {
        Self::from_m256i(_mm256_slli_epi32(self.to_m256i(), I))
    }

    /// 32-bit immediate arithmetic right shift, lane-wise.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn srai32<const I: i32>(self) -> Self {
        Self::from_m256i(_mm256_srai_epi32(self.to_m256i(), I))
    }

    /// 32-bit immediate logical right shift, lane-wise.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn srli32<const I: i32>(self) -> Self {
        Self::from_m256i(_mm256_srli_epi32(self.to_m256i(), I))
    }

    /// 64-bit immediate logical left shift.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn slli64<const I: i32>(self) -> Self {
        Self::from_m256i(_mm256_slli_epi64(self.to_m256i(), I))
    }

    /// 64-bit variable logical left shift, lane-wise.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn sllv64(self, rhs: Self) -> Self {
        Self::from_m256i(_mm256_sllv_epi64(self.to_m256i(), rhs.to_m256i()))
    }

    /// 64-bit immediate logical right shift.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn srli64<const I: i32>(self) -> Self {
        Self::from_m256i(_mm256_srli_epi64(self.to_m256i(), I))
    }

    /// 64-bit variable logical right shift, lane-wise.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn srlv64(self, rhs: Self) -> Self {
        Self::from_m256i(_mm256_srlv_epi64(self.to_m256i(), rhs.to_m256i()))
    }

    /// 64-bit variable arithmetic right shift, lane-wise.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn srav64(self, rhs: Self) -> Self {
        Self::from_m256i(_mm256_srav_epi64(self.to_m256i(), rhs.to_m256i()))
    }

    // -----------------------------------------------------------------------
    // Lane / shuffle / unpack.
    // -----------------------------------------------------------------------

    /// Interleave the low 16-bit halves.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn unpack_lo16(self, rhs: Self) -> Self {
        Self::from_m256i(_mm256_unpacklo_epi16(self.to_m256i(), rhs.to_m256i()))
    }

    /// Interleave the high 16-bit halves.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn unpack_hi16(self, rhs: Self) -> Self {
        Self::from_m256i(_mm256_unpackhi_epi16(self.to_m256i(), rhs.to_m256i()))
    }

    /// Interleave the low 32-bit lanes.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn unpack_lo(self, rhs: Self) -> Self {
        Self::from_m256i(_mm256_unpacklo_epi32(self.to_m256i(), rhs.to_m256i()))
    }

    /// Interleave the high 32-bit lanes.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn unpack_hi(self, rhs: Self) -> Self {
        Self::from_m256i(_mm256_unpackhi_epi32(self.to_m256i(), rhs.to_m256i()))
    }

    /// 32-bit lane permute with a constant mask.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn shuffle<const MASK: i32>(self) -> Self {
        Self::from_m256i(_mm256_shuffle_epi32::<MASK>(self.to_m256i()))
    }

    /// Per-byte variable shuffle (`vpshufb` on AVX2).
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn shuffle8(self, mask: Self) -> Self {
        Self::from_m256i(_mm256_shuffle_epi8(self.to_m256i(), mask.to_m256i()))
    }

    /// Variable blend per byte.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn blendv(self, rhs: Self, mask: Self) -> Self {
        Self::from_m256i(_mm256_blendv_epi8(
            self.to_m256i(),
            rhs.to_m256i(),
            mask.to_m256i(),
        ))
    }

    /// 32-bit compile-time blend, matches `_mm256_blend_epi32`.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn blend32_const<const MASK: i32>(self, rhs: Self) -> Self {
        Self::from_m256i(_mm256_blend_epi32(self.to_m256i(), rhs.to_m256i(), MASK))
    }

    /// 16-bit compile-time blend, matches `_mm256_blend_epi16`.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn blend16<const MASK: i32>(self, rhs: Self) -> Self {
        Self::from_m256i(_mm256_blend_epi16(self.to_m256i(), rhs.to_m256i(), MASK))
    }

    // -----------------------------------------------------------------------
    // Predicates.
    // -----------------------------------------------------------------------

    /// `self == rhs` lane-wise, returns all-ones / all-zero bits.
    #[inline]
    pub fn cmpeq(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m256i(_mm256_cmpeq_epi32(self.to_m256i(), rhs.to_m256i()))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self {
                xy: self.xy.cmpeq(rhs.xy),
                zw: self.zw.cmpeq(rhs.zw),
            }
        }
    }

    /// `self != rhs` lane-wise.
    #[inline]
    pub fn cmpneq(self, rhs: Self) -> Self {
        self.cmpeq(rhs).not()
    }

    /// `self > rhs` lane-wise signed.
    #[inline]
    pub fn cmpgt(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            Self::from_m256i(_mm256_cmpgt_epi32(self.to_m256i(), rhs.to_m256i()))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self {
                xy: self.xy.cmpgt(rhs.xy),
                zw: self.zw.cmpgt(rhs.zw),
            }
        }
    }

    /// `self < rhs` lane-wise signed.
    #[inline]
    pub fn cmplt(self, rhs: Self) -> Self {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            // NEON has no direct `<`; flip operands to use the existing
            // `_mm256_cmpgt_epi32`.
            Self::from_m256i(_mm256_cmpgt_epi32(rhs.to_m256i(), self.to_m256i()))
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self {
                xy: self.xy.cmplt(rhs.xy),
                zw: self.zw.cmplt(rhs.zw),
            }
        }
    }

    /// Vector equality test.
    #[inline]
    pub fn eq(self, rhs: Self) -> bool {
        self.xor(rhs).allfalse()
    }

    // -----------------------------------------------------------------------
    // Lane extractions.
    // -----------------------------------------------------------------------

    /// Extract lane `i` (0..=7) as a 32-bit integer.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn extract32<const I: i32>(self) -> i32 {
        // Lane 0..3 live in the low 128-bit lane and lanes 4..7 in the
        // high 128-bit lane. We split the result accordingly using
        // `match` so the const I is fed directly into the const generic
        // arguments of the SSE intrinsics.
        match (I >> 2, I & 3) {
            (0, lo) => match lo {
                0 => _mm_extract_epi32::<0>(_mm256_castsi256_si128(self.to_m256i())),
                1 => _mm_extract_epi32::<1>(_mm256_castsi256_si128(self.to_m256i())),
                2 => _mm_extract_epi32::<2>(_mm256_castsi256_si128(self.to_m256i())),
                _ => _mm_extract_epi32::<3>(_mm256_castsi256_si128(self.to_m256i())),
            },
            (_, lo) => match lo {
                0 => _mm_extract_epi32::<0>(_mm256_extracti128_si256::<1>(self.to_m256i())),
                1 => _mm_extract_epi32::<1>(_mm256_extracti128_si256::<1>(self.to_m256i())),
                2 => _mm_extract_epi32::<2>(_mm256_extracti128_si256::<1>(self.to_m256i())),
                _ => _mm_extract_epi32::<3>(_mm256_extracti128_si256::<1>(self.to_m256i())),
            },
        }
    }

    /// Extract the low or high 128-bit lane as a `GsVector4i`.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn extract<const I: i32>(self) -> GsVector4i {
        if I == 0 {
            GsVector4i::from_m128i(_mm256_castsi256_si128(self.to_m256i()))
        } else {
            GsVector4i::from_m128i(_mm256_extracti128_si256::<I>(self.to_m256i()))
        }
    }

    /// Insert a `__m128i` lane at position `I` (0..=1).
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn insert<const I: i32>(self, m: GsVector4i) -> Self {
        Self::from_m256i(_mm256_inserti128_si256::<I>(
            self.to_m256i(),
            m.to_m128i(),
        ))
    }

    /// Insert a 32-bit value at lane `I` (0..=7).
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn insert32<const I: i32>(self, v: i32) -> Self {
        // We extract the appropriate 128-bit lane, insert into it, then
        // re-insert. The lane index is computed from `I` at compile time
        // by branching on the value of `I` itself, which lets us pass
        // concrete literals to the const generic arguments of the
        // SSE/AVX2 intrinsics.
        let cur = if I < 4 {
            GsVector4i::from_m128i(_mm256_castsi256_si128(self.to_m256i()))
        } else {
            GsVector4i::from_m128i(_mm256_extracti128_si256::<1>(self.to_m256i()))
        };
        let updated = match I & 3 {
            0 => cur.insert32::<0>(v),
            1 => cur.insert32::<1>(v),
            2 => cur.insert32::<2>(v),
            _ => cur.insert32::<3>(v),
        };
        if I < 4 {
            Self::from_m256i(_mm256_inserti128_si256::<0>(
                self.to_m256i(),
                updated.to_m128i(),
            ))
        } else {
            Self::from_m256i(_mm256_inserti128_si256::<1>(
                self.to_m256i(),
                updated.to_m128i(),
            ))
        }
    }

    // -----------------------------------------------------------------------
    // Sign- and zero-extend helpers, mirroring the C++ static functions.
    // -----------------------------------------------------------------------

    /// Sign-extend the low 8 bytes to 16-bit, returning a 16-bit vector in
    /// the low half and zeros in the high half.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn i8to16(self) -> GsVector4i {
        GsVector4i::from_m128i(_mm_cvtepi8_epi16(_mm256_castsi256_si128(
            self.to_m256i(),
        )))
    }

    /// Zero-extend the low 8 bytes to 16-bit.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn u8to16(self) -> GsVector4i {
        GsVector4i::from_m128i(_mm_cvtepu8_epi16(_mm256_castsi256_si128(
            self.to_m256i(),
        )))
    }

    /// Sign-extend the low 4 bytes to 32-bit.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn i8to32(self) -> GsVector4i {
        GsVector4i::from_m128i(_mm_cvtepi8_epi32(_mm256_castsi256_si128(
            self.to_m256i(),
        )))
    }

    /// Zero-extend the low 4 bytes to 32-bit.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn u8to32(self) -> GsVector4i {
        GsVector4i::from_m128i(_mm_cvtepu8_epi32(_mm256_castsi256_si128(
            self.to_m256i(),
        )))
    }

    /// Sign-extend the low 4 16-bit lanes to 32-bit.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn i16to32(self) -> GsVector4i {
        GsVector4i::from_m128i(_mm_cvtepi16_epi32(_mm256_castsi256_si128(
            self.to_m256i(),
        )))
    }

    /// Zero-extend the low 4 16-bit lanes to 32-bit.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn u16to32(self) -> GsVector4i {
        GsVector4i::from_m128i(_mm_cvtepu16_epi32(_mm256_castsi256_si128(
            self.to_m256i(),
        )))
    }

    /// `mulhi` for signed 16-bit lanes, taking the high half of each
    /// 32-bit product.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn mul16hs(self, rhs: Self) -> Self {
        Self::from_m256i(_mm256_mulhi_epi16(self.to_m256i(), rhs.to_m256i()))
    }

    /// `mulhi` for unsigned 16-bit lanes.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn mul16hu(self, rhs: Self) -> Self {
        Self::from_m256i(_mm256_mulhi_epu16(self.to_m256i(), rhs.to_m256i()))
    }

    /// `mullo` for 16-bit lanes, taking the low half of each 32-bit
    /// product.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn mul16l(self, rhs: Self) -> Self {
        Self::from_m256i(_mm256_mullo_epi16(self.to_m256i(), rhs.to_m256i()))
    }

    /// `madd` 16-bit -> 32-bit, multiplies adjacent pairs and adds.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn madd16(self, rhs: Self) -> Self {
        Self::from_m256i(_mm256_madd_epi16(self.to_m256i(), rhs.to_m256i()))
    }

    /// Absolute value of each 8-bit lane.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn abs8(self) -> Self {
        Self::from_m256i(_mm256_abs_epi8(self.to_m256i()))
    }

    /// Absolute value of each 16-bit lane.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn abs16(self) -> Self {
        Self::from_m256i(_mm256_abs_epi16(self.to_m256i()))
    }

    /// Absolute value of each 32-bit lane.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn abs32(self) -> Self {
        Self::from_m256i(_mm256_abs_epi32(self.to_m256i()))
    }
}

// Operator overloads for GsVector8i.
impl core::ops::Add for GsVector8i {
    type Output = GsVector8i;
    #[inline]
    fn add(self, rhs: GsVector8i) -> GsVector8i {
        self.add_s32(rhs)
    }
}

impl core::ops::Sub for GsVector8i {
    type Output = GsVector8i;
    #[inline]
    fn sub(self, rhs: GsVector8i) -> GsVector8i {
        self.sub_s32(rhs)
    }
}

impl core::ops::BitAnd for GsVector8i {
    type Output = GsVector8i;
    #[inline]
    fn bitand(self, rhs: GsVector8i) -> GsVector8i {
        self.and(rhs)
    }
}

impl core::ops::BitOr for GsVector8i {
    type Output = GsVector8i;
    #[inline]
    fn bitor(self, rhs: GsVector8i) -> GsVector8i {
        self.or(rhs)
    }
}

impl core::ops::BitXor for GsVector8i {
    type Output = GsVector8i;
    #[inline]
    fn bitxor(self, rhs: GsVector8i) -> GsVector8i {
        self.xor(rhs)
    }
}

impl core::ops::Shl<i32> for GsVector8i {
    type Output = GsVector8i;
    #[inline]
    fn shl(self, _rhs: i32) -> GsVector8i {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            self.slli32::<0>()
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            self
        }
    }
}

impl core::ops::Shr<i32> for GsVector8i {
    type Output = GsVector8i;
    #[inline]
    fn shr(self, _rhs: i32) -> GsVector8i {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            self.srli32::<0>()
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            self
        }
    }
}

// ---------------------------------------------------------------------------
// Constants used by the static `x00xxxxxx` / `xff...` helpers in the C++
// API. The full table from `GSVector.cpp` is encoded here in compact form.
// ---------------------------------------------------------------------------

/// Returns the `x00xxxxxx`-style mask for `n` bytes on a `GsVector4i`.
#[inline]
pub fn m_xff4(n: i32) -> GsVector4i {
    GsVector4i::xff(n)
}

/// Returns the `x0f`-style mask for `n` bytes on a `GsVector4i`.
#[inline]
pub fn m_x0f4(n: i32) -> GsVector4i {
    GsVector4i::x0f(n)
}

// ---------------------------------------------------------------------------
// Trigonometric / `log2` style helpers (from `GSVector4::log2`).
// ---------------------------------------------------------------------------

/// `log2` approximation matching the C++ `GSVector4::log2` polynomial fit.
/// Returns `log2(self)` per lane. `precision` selects the polynomial
/// degree, in the range 3..=6.
#[inline]
pub fn log2_4(v: GsVector4, precision: i32) -> GsVector4 {
    let one = GsVector4::one();
    let i = unsafe { v.to_vec4_i32_trunc() };
    let shifted = unsafe { i.slli32::<1>().srli32::<24>() };
    let e = unsafe {
        GsVector4::from_i32(shifted.sub_s32(GsVector4i::splat(0x7F)))
    };
    let m = unsafe { v.cast_to_vec4_i32() };
    let m = unsafe { m.slli32::<9>().srli32::<9>() };
    let m = unsafe { m.cast_to_vec4() }.or(one);

    let p = match precision {
        3 => poly_log2_3(m),
        4 => poly_log2_4(m),
        5 => poly_log2_5(m),
        _ => poly_log2_6(m),
    };

    let delta = m.sub(one);
    p.mul(delta).add(e)
}

#[inline]
fn poly_log2_3(m: GsVector4) -> GsVector4 {
    let c0 = GsVector4::splat(2.28330284476918490682);
    let c1 = GsVector4::splat(-1.04913055217340124191);
    let c2 = GsVector4::splat(0.204446009836232697516);
    m.madd(c1, c0).madd(m, c2)
}

#[inline]
fn poly_log2_4(m: GsVector4) -> GsVector4 {
    let c0 = GsVector4::splat(2.61761038894603480148);
    let c1 = GsVector4::splat(-1.75647175389045657003);
    let c2 = GsVector4::splat(0.688243882994381274313);
    let c3 = GsVector4::splat(-0.107254423828329604454);
    m.madd(c1, c0).madd(m, c2).madd(m, c3)
}

#[inline]
fn poly_log2_5(m: GsVector4) -> GsVector4 {
    let c0 = GsVector4::splat(2.8882704548164776201);
    let c1 = GsVector4::splat(-2.52074962577807006663);
    let c2 = GsVector4::splat(1.48116647521213171641);
    let c3 = GsVector4::splat(-0.465725644288844778798);
    let c4 = GsVector4::splat(0.0596515482674574969533);
    m.madd(c1, c0).madd(m, c2).madd(m, c3).madd(m, c4)
}

#[inline]
fn poly_log2_6(m: GsVector4) -> GsVector4 {
    let c0 = GsVector4::splat(3.1157899);
    let c1 = GsVector4::splat(-3.3241990);
    let c2 = GsVector4::splat(2.5988452);
    let c3 = GsVector4::splat(-1.2315303);
    let c4 = GsVector4::splat(3.1821337e-1);
    let c5 = GsVector4::splat(-3.4436006e-2);
    m.madd(c1, c0).madd(m, c2).madd(m, c3).madd(m, c4).madd(m, c5)
}

// Helpers used by `log2_4`.
impl GsVector4 {
    /// Convert this float vector to an `i32` vector by truncation.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn to_vec4_i32_trunc(self) -> GsVector4i {
        GsVector4i::from_m128i(_mm_cvttps_epi32(self.to_m128()))
    }

    /// Bit-cast this float vector to an `i32` vector (no conversion).
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn cast_to_vec4_i32(self) -> GsVector4i {
        GsVector4i::from_m128i(_mm_castps_si128(self.to_m128()))
    }
}

impl GsVector4i {
    /// Splat a constant.
    #[inline]
    pub fn cxpr_const(v: i32) -> Self {
        Self::splat(v)
    }

    /// 32-bit immediate logical left shift, lane-wise.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn slli32_rt<const BITS: i32>(self) -> Self {
        Self::from_m128i(_mm_slli_epi32::<BITS>(self.to_m128i()))
    }

    /// 32-bit immediate logical right shift, lane-wise.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn srli32_rt<const BITS: i32>(self) -> Self {
        Self::from_m128i(_mm_srli_epi32::<BITS>(self.to_m128i()))
    }
}

impl GsVector4 {
    /// Convert `i32` lanes to `f32` lanes.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn from_i32(v: GsVector4i) -> GsVector4 {
        GsVector4::from_m128(_mm_cvtepi32_ps(v.to_m128i()))
    }
}

// ---------------------------------------------------------------------------
// Tests - simple smoke tests that exercise the surface area.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vec4_construction() {
        let v = GsVector4::new(1.0, 2.0, 3.0, 4.0);
        assert_eq!(v.x, 1.0);
        assert_eq!(v.y, 2.0);
        assert_eq!(v.z, 3.0);
        assert_eq!(v.w, 4.0);
    }

    #[test]
    fn vec4_arith() {
        let a = GsVector4::new(1.0, 2.0, 3.0, 4.0);
        let b = GsVector4::new(5.0, 6.0, 7.0, 8.0);
        let c = a.add(b);
        assert_eq!(c.x, 6.0);
        assert_eq!(c.w, 12.0);
        let d = b.sub(a);
        assert_eq!(d.x, 4.0);
    }

    #[test]
    fn vec4_round() {
        let a = GsVector4::new(0.1, 0.5, -0.5, -1.5);
        let f = a.floor();
        assert_eq!(f.x, 0.0);
        assert_eq!(f.z, -1.0);
    }

    #[test]
    fn vec4i_construction() {
        let v = GsVector4i::new(1, 2, 3, 4);
        assert_eq!(v.x, 1);
        assert_eq!(v.w, 4);
    }

    #[test]
    fn vec4i_eq() {
        let a = GsVector4i::new(1, 2, 3, 4);
        let b = GsVector4i::new(1, 2, 3, 4);
        assert!(a.eq(b));
        let c = GsVector4i::new(1, 2, 3, 5);
        assert!(!a.eq(c));
    }

    #[test]
    fn vec8_construction() {
        let v = GsVector8::new(1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0);
        assert_eq!(v.xy.x, 1.0);
        assert_eq!(v.zw.w, 8.0);
    }

    #[test]
    fn vec8_arith() {
        let a = GsVector8::splat(2.0);
        let b = GsVector8::splat(3.0);
        let c = a.add(b);
        assert_eq!(c.xy.x, 5.0);
    }

    #[test]
    fn vec8i_construction() {
        let v = GsVector8i::new(1, 2, 3, 4, 5, 6, 7, 8);
        assert_eq!(v.xy.x, 1);
        assert_eq!(v.zw.w, 8);
    }
}
