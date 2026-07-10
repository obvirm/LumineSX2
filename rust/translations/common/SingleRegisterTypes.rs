//! Types that are guaranteed to fit in one register.
//!
//! Translation of `common/SingleRegisterTypes.h` from PCSX2. Provides the
//! `r128` SIMD register type alias and the helpers used to load, store,
//! construct and convert such values. The original C++ ties this to SSE2
//! on x86 and NEON on aarch64 and rejects every other architecture at
//! compile time; the same gate is preserved here.
//!
//! Recompilers rely on some of these types and the registers they allocate
//! to, so be careful if you want to change them.

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
compile_error!("Unknown architecture.");

/// `RETURNS_R128 r128 __vectorcall` return-type marker from the C++ side.
/// Rust has no `__vectorcall` keyword, so this just expands to a plain
/// `r128` return type.
#[macro_export]
macro_rules! RETURNS_R128 {
    ($($tail:tt)*) => { r128 $($tail)* };
}

/// `__vectorcall` parameter-list marker from the C++ side. No-op in Rust.
#[macro_export]
macro_rules! TAKES_R128 {
    ($($tail:tt)*) => { $($tail)* };
}

#[cfg(target_arch = "x86_64")]
mod imp {
    use core::arch::x86_64::*;

    /// 128-bit SIMD register (SSE2 `__m128i`).
    pub type r128 = __m128i;

    /// Load an aligned 16-byte value from `ptr` into an `r128`.
    #[inline(always)]
    pub unsafe fn r128_load(ptr: *const u8) -> r128 {
        // Safety: caller guarantees `ptr` is 16-byte aligned and points to
        // at least 16 readable bytes.
        _mm_load_si128(ptr as *const __m128i)
    }

    /// Store an `r128` to the 16-byte-aligned address `ptr`.
    #[inline(always)]
    pub unsafe fn r128_store(ptr: *mut u8, val: r128) {
        // Safety: caller guarantees `ptr` is 16-byte aligned and points to
        // at least 16 writable bytes.
        _mm_store_si128(ptr as *mut __m128i, val)
    }

    /// Store an `r128` to the (possibly unaligned) address `ptr`.
    #[inline(always)]
    pub unsafe fn r128_store_unaligned(ptr: *mut u8, val: r128) {
        // Safety: caller guarantees `ptr` points to at least 16 writable
        // bytes.
        _mm_storeu_si128(ptr as *mut __m128i, val)
    }

    /// Return an `r128` with all bits cleared.
    #[inline(always)]
    pub fn r128_zero() -> r128 {
        unsafe { _mm_setzero_si128() }
    }

    /// Broadcast a `u64` to both 64-bit lanes of an `r128`.
    /// Expects that the `u64` came from r64-handling code, and not from a
    /// recompiler or something.
    #[inline(always)]
    pub fn r128_from_u64_dup(val: u64) -> r128 {
        unsafe { _mm_set1_epi64x(val as i64) }
    }

    /// Zero-extend a `u64` into the low lane of an `r128` (high lane zero).
    #[inline(always)]
    pub fn r128_from_u64_zext(val: u64) -> r128 {
        unsafe { _mm_set_epi64x(0, val as i64) }
    }

    /// Broadcast a `u32` to all four 32-bit lanes of an `r128`.
    #[inline(always)]
    pub fn r128_from_u32_dup(val: u32) -> r128 {
        unsafe { _mm_set1_epi32(val as i32) }
    }

    /// Build an `r128` from four `u32` values in lane order
    /// (`lo0`, `lo1`, `hi0`, `hi1`).
    #[inline(always)]
    pub fn r128_from_u32x4(lo0: u32, lo1: u32, hi0: u32, hi1: u32) -> r128 {
        unsafe { _mm_setr_epi32(lo0 as i32, lo1 as i32, hi0 as i32, hi1 as i32) }
    }

    /// Load an `r128` from a `u128` reference.
    #[inline(always)]
    pub fn r128_from_u128(u: &u128) -> r128 {
        // Safety: `u` is a valid `u128` reference; the underlying load is
        // unaligned and tolerates any byte alignment.
        unsafe { _mm_loadu_si128(u as *const u128 as *const __m128i) }
    }

    /// Extract the low 32-bit lane of an `r128`.
    #[inline(always)]
    pub fn r128_to_u32(val: r128) -> u32 {
        // Safety: `_mm_cvtsi128_si32` only touches the low lane; no
        // alignment or pointer requirements.
        unsafe { _mm_cvtsi128_si32(val) as u32 }
    }

    /// Extract the low 64-bit lane of an `r128`.
    #[inline(always)]
    pub fn r128_to_u64(val: r128) -> u64 {
        // Safety: `_mm_cvtsi128_si64` only touches the low lane.
        unsafe { _mm_cvtsi128_si64(val) as u64 }
    }

    /// Store an `r128` into a 16-byte-aligned `u128` value.
    #[inline(always)]
    pub fn r128_to_u128(val: r128) -> u128 {
        let mut ret: u128 = 0;
        // Safety: `&mut ret` is a valid 16-byte-aligned reference; we
        // perform a 16-byte aligned store.
        unsafe { _mm_store_si128(&mut ret as *mut u128 as *mut __m128i, val) }
        ret
    }

    /// Copy 16 bytes (one quadword chunk) from `src` to `dest`.
    #[inline(always)]
    pub unsafe fn copy_qwc(dest: *mut u8, src: *const u8) {
        // Safety: caller guarantees both pointers are 16-byte aligned and
        // point to at least 16 valid bytes.
        _mm_store_ps(
            dest as *mut f32,
            _mm_load_ps(src as *const f32),
        )
    }

    /// Zero 16 bytes (one quadword chunk) at `dest`.
    #[inline(always)]
    pub unsafe fn zero_qwc(dest: *mut u8) {
        // Safety: caller guarantees `dest` is 16-byte aligned and points
        // to at least 16 writable bytes.
        _mm_store_ps(dest as *mut f32, _mm_setzero_ps())
    }

    /// Zero a 16-byte-aligned `u128` in place.
    #[inline(always)]
    pub fn zero_qwc_u128(dest: &mut u128) {
        // Safety: `&mut dest` is a valid 16-byte-aligned reference.
        unsafe { _mm_store_ps(dest as *mut u128 as *mut f32, _mm_setzero_ps()) }
    }
}

#[cfg(target_arch = "aarch64")]
mod imp {
    use core::arch::aarch64::*;

    /// 128-bit SIMD register (NEON `uint32x4_t`).
    pub type r128 = uint32x4_t;

    /// Load a 16-byte value from `ptr` into an `r128`.
    #[inline(always)]
    pub unsafe fn r128_load(ptr: *const u8) -> r128 {
        // Safety: caller guarantees `ptr` points to at least 16 readable
        // bytes. NEON `vld1q_u32` does not require alignment.
        vld1q_u32(ptr as *const u32)
    }

    /// Store an `r128` to `ptr` (16 bytes, no alignment required).
    #[inline(always)]
    pub unsafe fn r128_store(ptr: *mut u8, val: r128) {
        // Safety: caller guarantees `ptr` points to at least 16 writable
        // bytes.
        vst1q_u32(ptr as *mut u32, val)
    }

    /// Store an `r128` to `ptr` (16 bytes, no alignment required).
    #[inline(always)]
    pub unsafe fn r128_store_unaligned(ptr: *mut u8, val: r128) {
        // Safety: caller guarantees `ptr` points to at least 16 writable
        // bytes.
        vst1q_u32(ptr as *mut u32, val)
    }

    /// Return an `r128` with all bits cleared.
    #[inline(always)]
    pub fn r128_zero() -> r128 {
        vdupq_n_u32(0)
    }

    /// Broadcast a `u64` to both 64-bit lanes of an `r128`.
    /// Expects that the `u64` came from r64-handling code, and not from a
    /// recompiler or something.
    #[inline(always)]
    pub fn r128_from_u64_dup(val: u64) -> r128 {
        // Safety: `vreinterpretq_u32_u64` is a pure register reinterpret
        // with no memory access.
        unsafe { vreinterpretq_u32_u64(vdupq_n_u64(val)) }
    }

    /// Zero-extend a `u64` into the low lane of an `r128` (high lane zero).
    #[inline(always)]
    pub fn r128_from_u64_zext(val: u64) -> r128 {
        // Safety: `vcreate_u64` and `vcombine_u64` are pure data
        // manipulation; no memory access.
        let lo = unsafe { vcreate_u64(val) };
        let hi = unsafe { vcreate_u64(0) };
        unsafe { vreinterpretq_u32_u64(vcombine_u64(lo, hi)) }
    }

    /// Broadcast a `u32` to all four 32-bit lanes of an `r128`.
    #[inline(always)]
    pub fn r128_from_u32_dup(val: u32) -> r128 {
        vdupq_n_u32(val)
    }

    /// Build an `r128` from four `u32` values in lane order
    /// (`lo0`, `lo1`, `hi0`, `hi1`).
    #[inline(always)]
    pub fn r128_from_u32x4(lo0: u32, lo1: u32, hi0: u32, hi1: u32) -> r128 {
        let values = [lo0, lo1, hi0, hi1];
        // Safety: `values` is a valid 4-element `u32` array on the stack.
        unsafe { vld1q_u32(values.as_ptr()) }
    }

    /// Load an `r128` from a `u128` reference.
    #[inline(always)]
    pub fn r128_from_u128(u: &u128) -> r128 {
        // Safety: `u` is a valid `u128` reference.
        unsafe { vld1q_u32(u as *const u128 as *const u32) }
    }

    /// Extract the low 32-bit lane of an `r128`.
    #[inline(always)]
    pub fn r128_to_u32(val: r128) -> u32 {
        // Safety: lane read with no side effects.
        unsafe { vgetq_lane_u32(val, 0) }
    }

    /// Extract the low 64-bit lane of an `r128`.
    #[inline(always)]
    pub fn r128_to_u64(val: r128) -> u64 {
        // Safety: `vreinterpretq_u64_u32` is a pure register reinterpret
        // and `vgetq_lane_u64` reads a specific lane.
        unsafe { vgetq_lane_u64(vreinterpretq_u64_u32(val), 0) }
    }

    /// Store an `r128` into a `u128` value.
    #[inline(always)]
    pub fn r128_to_u128(val: r128) -> u128 {
        let mut ret: u128 = 0;
        // Safety: `&mut ret` is a valid 16-byte-aligned reference.
        unsafe { vst1q_u32(&mut ret as *mut u128 as *mut u32, val) }
        ret
    }

    /// Copy 16 bytes (one quadword chunk) from `src` to `dest`.
    #[inline(always)]
    pub unsafe fn copy_qwc(dest: *mut u8, src: *const u8) {
        // Safety: caller guarantees 16 valid bytes for both pointers.
        vst1q_u8(dest, vld1q_u8(src))
    }

    /// Zero 16 bytes (one quadword chunk) at `dest`.
    #[inline(always)]
    pub unsafe fn zero_qwc(dest: *mut u8) {
        // Safety: caller guarantees 16 writable bytes at `dest`.
        vst1q_u8(dest, vmovq_n_u8(0))
    }

    /// Zero a `u128` in place.
    #[inline(always)]
    pub fn zero_qwc_u128(dest: &mut u128) {
        // Safety: `&mut dest` is a valid 16-byte-aligned reference.
        unsafe { vst1q_u8(dest as *mut u128 as *mut u8, vmovq_n_u8(0)) }
    }
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub use imp::*;
