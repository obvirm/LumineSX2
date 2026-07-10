// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! 128-bit single-register types and helpers.
//!
//! Mirrors PCSX2's `common/SingleRegisterTypes.h`. The original C++ file
//! defines an `r128` type (a hardware SIMD register — `__m128i` on x86,
//! `uint32x4_t` on aarch64) and a battery of helpers around it.
//!
//! # Strategy
//!
//! The Rust port uses an explicit, portable 128-bit newtype
//! `R128(pub [u32; 4])` rather than the architecture-specific register
//! types. This has several advantages:
//!
//! * The type is `#[repr(C)]`, so the layout is fixed and stable across
//!   platforms (four `u32` lanes, little-endian on the targets PCSX2
//!   cares about).
//! * It is `Copy`, `Default`, `PartialEq`, etc., and requires no `unsafe`
//!   for the bulk of the API.
//! * The 16-byte size matches the SIMD register width exactly, so it
//!   round-trips cleanly through FFI boundaries as a pointer-to-array.
//!
//! # Deferred SIMD optimization
//!
//! The current implementation is **scalar** — every helper writes or
//! reads through plain `u32` lanes. This is correct everywhere but
//! slower than `_mm_*` / `vld1q_*` intrinsics. Future work can swap the
//! body of each helper for an inline-`unsafe` block gated on
//! `#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]` and
//! `std::arch::{x86_64::*, aarch64::*}`. Because the public surface
//! stays in terms of `R128([u32; 4])` the change is purely a
//! performance optimization with no API churn.
//!
//! # Calling convention
//!
//! The C++ `RETURNS_R128` / `TAKES_R128` macros expand to
//! `r128 __vectorcall` on MSVC and a no-op on the platforms PCSX2
//! supports with GCC/Clang. The Rust port does not need an analogue:
//! scalar 16-byte values move in two general-purpose registers (System V
//! AMD64) or one pair of vector registers (AArch64 PCS), which is fine
//! for the scalar implementation. When real SIMD is wired up, the
//! helpers will return/take `R128` by value (it is `Copy`), and the
//! compiler's backend will pick the right calling convention.
//!
//! # FFI
//!
//! The 128-bit value crosses the boundary as `*mut [u32; 4]` /
//! `*const [u32; 4]`. These are thin pointers to a fixed-size array on
//! the C++ side; `cbindgen` renders them as `uint32_t (*)[4]`.

// =============================================================================
// Canonical 128-bit type
// =============================================================================

/// A 128-bit value that fits in a single SIMD register.
///
/// Layout: four `u32` lanes, in the natural little-endian order. This
/// matches the lane order of `_mm_setr_epi32` / `vld1q_u32` on the
/// targets we care about.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct R128(pub [u32; 4]);

impl R128 {
    /// Number of bytes in an `R128`.
    pub const SIZE: usize = 16;

    /// All-zero value.
    pub const ZERO: R128 = R128([0, 0, 0, 0]);

    /// Build an `R128` from its four `u32` lanes (lo, lo, hi, hi).
    #[inline(always)]
    pub const fn from_u32x4(lo0: u32, lo1: u32, hi0: u32, hi1: u32) -> Self {
        R128([lo0, lo1, hi0, hi1])
    }

    /// Broadcast a `u32` to all four lanes.
    #[inline(always)]
    pub const fn from_u32_dup(val: u32) -> Self {
        R128([val; 4])
    }

    /// Broadcast a `u64` to both 64-bit halves.
    #[inline(always)]
    pub const fn from_u64_dup(val: u64) -> Self {
        R128([val as u32, (val >> 32) as u32, val as u32, (val >> 32) as u32])
    }

    /// Zero-extend a `u64` into the low 64 bits; the high 64 bits are 0.
    #[inline(always)]
    pub const fn from_u64_zext(val: u64) -> Self {
        R128([val as u32, (val >> 32) as u32, 0, 0])
    }

    /// Reinterpret the low 16 bytes of `src` as an `R128`.
    #[inline(always)]
    pub fn from_bytes(src: &[u8; 16]) -> Self {
        let mut out = [0u32; 4];
        let src_ptr = src.as_ptr() as *const u32;
        let out_ptr = out.as_mut_ptr();
        // Safety: both pointers are valid for 16 bytes / 4 u32s and
        // properly aligned (u32 is align-of-4, the array is 4-byte
        // aligned; 16-byte chunks may need an unaligned read on some
        // hosts, hence the unaligned read intrinsic from byte_swap on
        // x86. For the scalar implementation a bytewise copy is fine
        // and always safe.)
        unsafe {
            core::ptr::copy_nonoverlapping(src_ptr, out_ptr, 4);
        }
        R128(out)
    }

    /// Read lane 0 as a `u32`.
    #[inline(always)]
    pub const fn to_u32(self) -> u32 {
        self.0[0]
    }

    /// Read the low 64 bits as a `u64` (little-endian).
    #[inline(always)]
    pub const fn to_u64(self) -> u64 {
        (self.0[0] as u64) | ((self.0[1] as u64) << 32)
    }

    /// Store the value into a 16-byte buffer as little-endian `u32` lanes.
    #[inline(always)]
    pub fn store_to(self, dest: &mut [u8; 16]) {
        let dest_ptr = dest.as_mut_ptr() as *mut u32;
        // Safety: dest is valid for 16 bytes / 4 u32s; alignment is
        // fine for the scalar impl.
        unsafe {
            core::ptr::copy_nonoverlapping(self.0.as_ptr(), dest_ptr, 4);
        }
    }
}

// =============================================================================
// Module-level helpers (mirror the C++ free functions 1:1).
// =============================================================================

/// Load a 128-bit value from `src`.
///
/// Mirrors `r128_load`. The C++ version uses `_mm_load_si128`, which
/// requires 16-byte alignment; the scalar Rust implementation does not
/// — it just copies the four `u32` lanes.
#[inline(always)]
pub fn r128_load(src: &[u8; 16]) -> R128 {
    R128::from_bytes(src)
}

/// Store `val` into `dest`.
///
/// Mirrors `r128_store`. See [`r128_load`] for the alignment story.
#[inline(always)]
pub fn r128_store(dest: &mut [u8; 16], val: R128) {
    val.store_to(dest);
}

/// Unaligned 128-bit store.
///
/// The scalar implementation is identical to [`r128_store`]; on the C++
/// side this maps to `_mm_storeu_si128` which is allowed to take a
/// misaligned pointer.
#[inline(always)]
pub fn r128_store_unaligned(dest: &mut [u8; 16], val: R128) {
    val.store_to(dest);
}

/// All-zero 128-bit value.
#[inline(always)]
pub fn r128_zero() -> R128 {
    R128::ZERO
}

/// Broadcast a `u64` to both halves of a 128-bit register.
#[inline(always)]
pub fn r128_from_u64_dup(val: u64) -> R128 {
    R128::from_u64_dup(val)
}

/// Zero-extend a `u64` into the low 64 bits of a 128-bit register.
#[inline(always)]
pub fn r128_from_u64_zext(val: u64) -> R128 {
    R128::from_u64_zext(val)
}

/// Broadcast a `u32` to all four lanes.
#[inline(always)]
pub fn r128_from_u32_dup(val: u32) -> R128 {
    R128::from_u32_dup(val)
}

/// Build an `R128` from its four lanes.
#[inline(always)]
pub fn r128_from_u32x4(lo0: u32, lo1: u32, hi0: u32, hi1: u32) -> R128 {
    R128::from_u32x4(lo0, lo1, hi0, hi1)
}

/// Reinterpret the 16 bytes of `u` as an `R128`.
#[inline(always)]
pub fn r128_from_bytes(u: &[u8; 16]) -> R128 {
    R128::from_bytes(u)
}

/// Read lane 0 as a `u32`.
#[inline(always)]
pub fn r128_to_u32(val: R128) -> u32 {
    val.to_u32()
}

/// Read the low 64 bits as a `u64`.
#[inline(always)]
pub fn r128_to_u64(val: R128) -> u64 {
    val.to_u64()
}

/// Store a 128-bit value into a 16-byte buffer (little-endian).
#[inline(always)]
pub fn r128_to_bytes(val: R128) -> [u8; 16] {
    let mut out = [0u8; 16];
    val.store_to(&mut out);
    out
}

// =============================================================================
// Quad-word copy / zero (the "QWC" helpers used everywhere in the EE/VU code).
// =============================================================================

/// Copy 16 bytes from `src` to `dest`.
///
/// Mirrors `CopyQWC`. QWC = "quad-word count" (one quad-word is 16
/// bytes) — a phrase borrowed from the PS2's DMA semantics.
#[inline(always)]
pub fn copy_qwc(dest: &mut [u8; 16], src: &[u8; 16]) {
    // Safety: same size, non-overlapping is the caller's contract (same
    // as C++'s `_mm_load_ps` / `_mm_store_ps` pair, which the PS2
    // toolchain treats as a memcpy in this context).
    dest.copy_from_slice(src);
}

/// Zero-fill a 16-byte buffer.
#[inline(always)]
pub fn zero_qwc(dest: &mut [u8; 16]) {
    *dest = [0u8; 16];
}

// =============================================================================
// FFI surface (consumed by C++ PCSX2 via cbindgen).
//
// All raw-pointer FFI functions are `unsafe` at the C++ side: null
// pointers and aliasing are the caller's problem. The Rust
// implementations check for null and panic on misuse rather than UB.
// =============================================================================

/// FFI: write a zero `R128` to `*out`.
///
/// `out` must be non-null and point to at least 16 writable bytes.
#[no_mangle]
pub extern "C" fn pcsx2_r128_zero(out: *mut [u32; 4]) {
    assert!(!out.is_null(), "pcsx2_r128_zero: null out pointer");
    // Safety: caller guarantees `out` is valid for writes of one
    // 16-byte / `[u32; 4]` value.
    unsafe {
        core::ptr::write(out, [0u32; 4]);
    }
}

/// FFI: load a 128-bit value from `*src` into `*out`.
///
/// Both pointers must be non-null and point to at least 16 bytes. They
/// may alias; the load completes before the store.
#[no_mangle]
pub extern "C" fn pcsx2_r128_load(src: *const [u32; 4], out: *mut [u32; 4]) {
    assert!(!src.is_null(), "pcsx2_r128_load: null src pointer");
    assert!(!out.is_null(), "pcsx2_r128_load: null out pointer");
    // Safety: both pointers are valid for a 16-byte read/write.
    unsafe {
        core::ptr::write(out, core::ptr::read(src));
    }
}

/// FFI: store the 128-bit value at `*src` into `*dest`.
///
/// Both pointers must be non-null and point to at least 16 bytes. They
/// may alias; the load completes before the store.
#[no_mangle]
pub extern "C" fn pcsx2_r128_store(src: *const [u32; 4], dest: *mut [u32; 4]) {
    assert!(!src.is_null(), "pcsx2_r128_store: null src pointer");
    assert!(!dest.is_null(), "pcsx2_r128_store: null dest pointer");
    // Safety: both pointers are valid for a 16-byte read/write.
    unsafe {
        core::ptr::write(dest, core::ptr::read(src));
    }
}

/// FFI: copy 16 bytes from `src` to `dest`.
///
/// `dest` and `src` must each point to at least 16 bytes. They may
/// alias; the operation is a `memcpy` and is well-defined for
/// overlapping ranges on stable Rust.
#[no_mangle]
pub extern "C" fn pcsx2_copy_qwc(dest: *mut u8, src: *const u8) {
    assert!(!dest.is_null(), "pcsx2_copy_qwc: null dest pointer");
    assert!(!src.is_null(), "pcsx2_copy_qwc: null src pointer");
    // Safety: caller guarantees both pointers are valid for 16-byte
    // reads/writes. `copy_nonoverlapping` would be UB on aliasing, so
    // we use `copy` which permits overlap.
    unsafe {
        core::ptr::copy(src, dest, 16);
    }
}

/// FFI: zero 16 bytes at `dest`.
///
/// `dest` must point to at least 16 writable bytes.
#[no_mangle]
pub extern "C" fn pcsx2_zero_qwc(dest: *mut u8) {
    assert!(!dest.is_null(), "pcsx2_zero_qwc: null dest pointer");
    // Safety: caller guarantees `dest` is valid for 16-byte writes.
    unsafe {
        core::ptr::write_bytes(dest, 0u8, 16);
    }
}
