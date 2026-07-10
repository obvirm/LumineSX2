// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
// Rust translation of `common/HashCombine.h`.
//
// Provides a `HashCombine` mixing helper (based on the boost::hash_combine
// formulation) that folds an arbitrary sequence of hashable values into a
// single `u64` seed, plus a small `BuildHasher` shim that exposes the
// combinator to anything in `std` that wants a `Hasher`. Only the `std`
// crate is used.

use std::hash::{BuildHasher, Hash, Hasher};

/// Folds the hash of `v` into `seed` in place using the classic
/// `boost::hash_combine` mixing step.
///
/// Equivalent to the C++ expression
/// ```text
/// seed ^= std::hash<T>{}(v) + 0x9e3779b9u + (seed << 6) + (seed >> 2);
/// ```
/// and is safe to call in `const` contexts.
#[inline]
pub const fn hash_combine_one(seed: &mut u64, v: u64) {
    *seed ^= v.wrapping_add(0x9e37_79b9).wrapping_add(*seed << 6).wrapping_add(*seed >> 2);
}

/// Recursively folds the hashes of `v` and the rest of `rest` into `seed`.
///
/// This mirrors the variadic C++ `HashCombine` template; each value is
/// hashed (via its [`Hash`] impl) and the result is mixed in with the
/// standard golden-ratio constant.
#[inline]
pub fn hash_combine<T, Rest>(seed: &mut u64, v: T, rest: Rest)
where
    T: Hash,
    Rest: HashCombineArgs,
{
    v.hash_one(seed);
    rest.combine(seed);
}

/// Convenience wrapper that returns the mixed seed rather than mutating
/// one in place.
#[inline]
pub fn HashCombine<T: Hash>(v: T) -> u64 {
    let mut seed: u64 = 0;
    v.hash_one(&mut seed);
    seed
}

/// Trait used to peel a variadic argument list for [`hash_combine`].
///
/// Models a "tuple" of hashable values, recursively mixing each one
/// into the running seed. Implemented for unit (the base case) and for
/// `(T, Rest)` where `T: Hash` and `Rest: HashCombineArgs`.
pub trait HashCombineArgs {
    /// Mixes every value in `self` into `seed`.
    fn combine(self, seed: &mut u64);
}

impl HashCombineArgs for () {
    #[inline]
    fn combine(self, _seed: &mut u64) {}
}

impl<T, Rest> HashCombineArgs for (T, Rest)
where
    T: Hash,
    Rest: HashCombineArgs,
{
    #[inline]
    fn combine(self, seed: &mut u64) {
        let (v, rest) = self;
        v.hash_one(seed);
        rest.combine(seed);
    }
}

/// Small helper extension that lets a `Hash` value contribute to a `u64`
/// seed without needing to construct a full [`Hasher`].
trait HashOne {
    /// Mixes `self` into `seed` using [`hash_combine_one`].
    fn hash_one(self, seed: &mut u64);
}

impl<T: Hash> HashOne for T {
    #[inline]
    fn hash_one(self, seed: &mut u64) {
        let mut h = HashCombineHasher::new();
        self.hash(&mut h);
        h.finish_one(seed);
    }
}

/// A `Hasher` that accumulates a `u64` seed using the same mixing rule
/// as [`hash_combine_one`], so that any `T: Hash` can be folded into a
/// `u64` with one call.
#[derive(Default)]
pub struct HashCombineHasher {
    seed: u64,
}

impl HashCombineHasher {
    /// Creates a fresh hasher starting from a zero seed.
    #[inline]
    pub const fn new() -> Self {
        Self { seed: 0 }
    }

    /// Creates a fresh hasher starting from `seed`.
    #[inline]
    pub const fn with_seed(seed: u64) -> Self {
        Self { seed }
    }

    /// Folds the bytes already written into this hasher into `out_seed`
    /// using the standard mixing step and returns the resulting value.
    #[inline]
    pub fn finish_one(mut self, out_seed: &mut u64) {
        let v = self.finish();
        hash_combine_one(out_seed, v);
    }
}

impl Hasher for HashCombineHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.seed
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        // Mirror `std::collections::hash_map::DefaultHasher`'s policy of
        // mixing the length into the seed so that variable-length inputs
        // don't trivially collide.
        hash_combine_one(&mut self.seed, bytes.len() as u64);
        let mut buf = [0u8; 8];
        let mut chunks = bytes.chunks_exact(8);
        for chunk in &mut chunks {
            buf.copy_from_slice(chunk);
            hash_combine_one(&mut self.seed, u64::from_le_bytes(buf));
        }
        let rem = chunks.remainder();
        if !rem.is_empty() {
            buf[..rem.len()].copy_from_slice(rem);
            hash_combine_one(&mut self.seed, u64::from_le_bytes(buf));
        }
    }

    #[inline]
    fn write_u8(&mut self, n: u8) {
        hash_combine_one(&mut self.seed, n as u64);
    }

    #[inline]
    fn write_u16(&mut self, n: u16) {
        hash_combine_one(&mut self.seed, n as u64);
    }

    #[inline]
    fn write_u32(&mut self, n: u32) {
        hash_combine_one(&mut self.seed, n as u64);
    }

    #[inline]
    fn write_u64(&mut self, n: u64) {
        hash_combine_one(&mut self.seed, n);
    }

    #[inline]
    fn write_usize(&mut self, n: usize) {
        hash_combine_one(&mut self.seed, n as u64);
    }

    #[inline]
    fn write_i8(&mut self, n: i8) {
        hash_combine_one(&mut self.seed, n as u64);
    }

    #[inline]
    fn write_i16(&mut self, n: i16) {
        hash_combine_one(&mut self.seed, n as u64);
    }

    #[inline]
    fn write_i32(&mut self, n: i32) {
        hash_combine_one(&mut self.seed, n as u64);
    }

    #[inline]
    fn write_i64(&mut self, n: i64) {
        hash_combine_one(&mut self.seed, n as u64);
    }

    #[inline]
    fn write_isize(&mut self, n: isize) {
        hash_combine_one(&mut self.seed, n as u64);
    }
}

/// `BuildHasher` factory for [`HashCombineHasher`].
#[derive(Clone, Default)]
pub struct HashCombineBuildHasher;

impl BuildHasher for HashCombineBuildHasher {
    type Hasher = HashCombineHasher;

    #[inline]
    fn build_hasher(&self) -> Self::Hasher {
        HashCombineHasher::new()
    }
}
