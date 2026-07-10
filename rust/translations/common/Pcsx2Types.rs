// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
// Idiomatic Rust translation of `common/Pcsx2Types.h`.
//
// This module provides the fixed-width integer aliases used throughout
// PCSX2, the pointer-sized aliases (`uptr` / `sptr`), and the
// non-SSE 128-bit helpers (`u128` / `s128`) that the EE/VU/IOP cores
// rely on for 128-bit GPR/VF register storage and arithmetic.

#![allow(non_camel_case_types)]

// --------------------------------------------------------------------------------------
//  Basic Atomic Types
// --------------------------------------------------------------------------------------

pub type s8 = i8;
pub type s16 = i16;
pub type s32 = i32;
pub type s64 = i64;

pub type u8 = ::core::primitive::u8;
pub type u16 = ::core::primitive::u16;
pub type u32 = ::core::primitive::u32;
pub type u64 = ::core::primitive::u64;

pub type uptr = usize;
pub type sptr = isize;

pub type uint = u32;

// --------------------------------------------------------------------------------------
//  u128 / s128 - A rough-and-ready cross platform 128-bit datatype, Non-SSE style.
// --------------------------------------------------------------------------------------
//
// The C++ original is a union that overlays a `lo`/`hi` pair with arrays
// of smaller widths.  In Rust we represent the same storage as a single
// `#[repr(C, align(16))]` struct so the layout matches the original
// union's storage class (and so `bytemuck`-style casting remains an
// option downstream).  Conversions from narrower types, comparison
// operators, and the casts to `u8` / `u16` / `u32` are preserved.

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
#[repr(C)]
pub struct u128 {
    pub lo: u64,
    pub hi: u64,
}

impl u128 {
    /// All-zero 128-bit value.
    pub const ZERO: Self = Self { lo: 0, hi: 0 };

    /// Construct a `u128` from its low and high 64-bit halves.
    #[inline]
    pub const fn new(lo: u64, hi: u64) -> Self {
        Self { lo, hi }
    }

    /// Explicit conversion from `u64`. Zero-extends the source through 128 bits.
    #[inline]
    pub const fn from_u64(src: u64) -> Self {
        Self { lo: src, hi: 0 }
    }

    /// Explicit conversion from `u32`. Zero-extends the source through 128 bits.
    #[inline]
    pub const fn from_u32(src: u32) -> Self {
        Self { lo: src as u64, hi: 0 }
    }

    /// View the value as 16 little-endian bytes.
    #[inline]
    pub const fn to_le_bytes(self) -> [u8; 16] {
        let mut out = [0u8; 16];
        let mut i = 0;
        while i < 8 {
            out[i] = (self.lo >> (i * 8)) as u8;
            out[i + 8] = (self.hi >> (i * 8)) as u8;
            i += 1;
        }
        out
    }

    /// Reconstruct a `u128` from 16 little-endian bytes.
    #[inline]
    pub const fn from_le_bytes(bytes: [u8; 16]) -> Self {
        let mut lo: u64 = 0;
        let mut hi: u64 = 0;
        let mut i = 0;
        while i < 8 {
            lo |= (bytes[i] as u64) << (i * 8);
            hi |= (bytes[i + 8] as u64) << (i * 8);
            i += 1;
        }
        Self { lo, hi }
    }
}

impl PartialEq<u32> for u128 {
    #[inline]
    fn eq(&self, other: &u32) -> bool {
        self.lo as u32 == *other && self.hi == 0
    }
}

impl PartialEq<u16> for u128 {
    #[inline]
    fn eq(&self, other: &u16) -> bool {
        self.lo as u16 == *other && self.hi == 0 && (self.lo >> 16) == 0
    }
}

impl PartialEq<u8> for u128 {
    #[inline]
    fn eq(&self, other: &u8) -> bool {
        self.lo as u8 == *other && self.hi == 0 && (self.lo >> 8) == 0
    }
}

impl From<u64> for u128 {
    #[inline]
    fn from(v: u64) -> Self {
        Self::from_u64(v)
    }
}

impl From<u32> for u128 {
    #[inline]
    fn from(v: u32) -> Self {
        Self::from_u32(v)
    }
}

impl From<[u8; 16]> for u128 {
    #[inline]
    fn from(bytes: [u8; 16]) -> Self {
        u128::from_le_bytes(bytes)
    }
}

impl From<u128> for [u8; 16] {
    #[inline]
    fn from(v: u128) -> Self {
        v.to_le_bytes()
    }
}

impl From<u128> for u32 {
    #[inline]
    fn from(v: u128) -> Self {
        v.lo as u32
    }
}

impl From<u128> for u16 {
    #[inline]
    fn from(v: u128) -> Self {
        v.lo as u16
    }
}

impl From<u128> for u8 {
    #[inline]
    fn from(v: u128) -> Self {
        v.lo as u8
    }
}

/// Signed 128-bit value, again matching the C++ `s128` layout: a pair
/// of `s64` halves with explicit sign-extending constructors and
/// narrow casts to the built-in primitive types.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
#[repr(C)]
pub struct s128 {
    pub lo: s64,
    pub hi: s64,
}

impl s128 {
    /// All-zero 128-bit value.
    pub const ZERO: Self = Self { lo: 0, hi: 0 };

    /// Construct a `s128` from its low and high 64-bit halves.
    #[inline]
    pub const fn new(lo: s64, hi: s64) -> Self {
        Self { lo, hi }
    }

    /// Explicit conversion from `s64`, with sign extension.
    #[inline]
    pub const fn from_s64(src: s64) -> Self {
        Self {
            lo: src,
            hi: if src < 0 { -1 } else { 0 },
        }
    }

    /// Explicit conversion from `s32`, with sign extension.
    #[inline]
    pub const fn from_s32(src: s32) -> Self {
        let lo = src as s64;
        Self {
            lo,
            hi: if src < 0 { -1 } else { 0 },
        }
    }

    /// View the value as 16 little-endian bytes (bit pattern of the two's
    /// complement representation).
    #[inline]
    pub const fn to_le_bytes(self) -> [u8; 16] {
        u128 {
            lo: self.lo as u64,
            hi: self.hi as u64,
        }
        .to_le_bytes()
    }

    /// Reconstruct a `s128` from 16 little-endian bytes.
    #[inline]
    pub const fn from_le_bytes(bytes: [u8; 16]) -> Self {
        let bits = u128::from_le_bytes(bytes);
        Self {
            lo: bits.lo as s64,
            hi: bits.hi as s64,
        }
    }
}

impl From<s64> for s128 {
    #[inline]
    fn from(v: s64) -> Self {
        Self::from_s64(v)
    }
}

impl From<s32> for s128 {
    #[inline]
    fn from(v: s32) -> Self {
        Self::from_s32(v)
    }
}

impl From<[u8; 16]> for s128 {
    #[inline]
    fn from(bytes: [u8; 16]) -> Self {
        s128::from_le_bytes(bytes)
    }
}

impl From<s128> for [u8; 16] {
    #[inline]
    fn from(v: s128) -> Self {
        v.to_le_bytes()
    }
}

impl From<s128> for u32 {
    #[inline]
    fn from(v: s128) -> Self {
        v.lo as u32
    }
}

impl From<s128> for u16 {
    #[inline]
    fn from(v: s128) -> Self {
        v.lo as u16
    }
}

impl From<s128> for u8 {
    #[inline]
    fn from(v: s128) -> Self {
        v.lo as u8
    }
}
