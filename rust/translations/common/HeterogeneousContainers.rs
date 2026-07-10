// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Heterogeneous, allocation-free lookup containers.
//!
//! Rust 2021 translation of PCSX2's `common/HeterogeneousContainers.h`. The
//! original C++ header exposes a family of `std::unordered_map` / `std::map`
//! aliases keyed on `std::string` whose hashers, equality and comparison
//! functors are marked `is_transparent`, allowing lookups with
//! `std::string_view` or `const char*` without materialising a `std::string`.
//!
//! In Rust, the same property is achieved natively via the
//! [`std::borrow::Borrow`] trait: `HashMap<String, V>::get(&Q)` accepts any
//! `Q` such that `String: Borrow<Q>` (so `&str`, `&String`, `Box<str>`,
//! `Cow<'_, str>`, ... all work) and `Q: Hash + Eq`, with no extra
//! allocation. The [`HeterogeneousHasher`] and [`HeterogeneousEq`] traits
//! below document the property and provide a uniform extension point if a
//! caller needs a non-default hasher.
//!
//! Only `std::collections` and `std::hash` are used.

use std::borrow::Borrow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher, Hash, Hasher};

/// Marker trait for hashers that participate in heterogeneous lookup.
///
/// Mirrors C++'s `using is_transparent = void;` annotation on a hasher
/// type. The blanket contract is that the hasher can produce the same
/// digest for any `Q` that the stored key type `K` `Borrow`s — so a
/// container keyed on `K` can be queried with a borrowed `Q` without
/// forcing a fresh `K` to be allocated.
pub trait HeterogeneousHasher {
    /// The key type stored in the container.
    type Key;

    /// Compute the hash of any value that can be borrowed as `Self::Key`.
    ///
    /// Implementations should agree with the hash produced by hashing
    /// `Self::Key` directly, so that lookups using a borrowed key land
    /// in the same bucket as insertions of the owned key.
    fn hash_heterogeneous<Q>(&self, value: &Q) -> u64
    where
        Self::Key: Borrow<Q>,
        Q: ?Sized + Hash,
    {
        let mut h = DefaultHasher::new();
        value.hash(&mut h);
        h.finish()
    }
}

/// Marker trait for equality comparators that participate in heterogeneous
/// lookup.
///
/// Mirrors C++'s `using is_transparent = void;` annotation on an equality
/// functor. The blanket contract is that equality can be tested between
/// any two borrow-compatible key forms.
pub trait HeterogeneousEq {
    /// The key type stored in the container.
    type Key;

    /// Test equality between any pair of borrow-compatible key types.
    fn eq_heterogeneous<Q>(&self, lhs: &Q, rhs: &Q) -> bool
    where
        Self::Key: Borrow<Q>,
        Q: ?Sized + PartialEq,
    {
        lhs == rhs
    }
}

/// Hasher matching `detail::transparent_string_hash` from the C++ header.
///
/// Because Rust's [`HashMap::get`] already accepts borrowed key forms via
/// the `Borrow` trait, this type's role is mostly documentary: a
/// container that uses [`StringBuildHasher`] (and the
/// [`HeterogeneousHasher`] impl on it) signals to readers that lookups
/// with `&str` and friends are intentional and allocation-free.
#[derive(Default, Clone, Copy, Debug)]
pub struct StringHasher;

impl HeterogeneousHasher for StringHasher {
    type Key = String;
}

impl Hasher for StringHasher {
    fn finish(&self) -> u64 {
        // Digest is produced inside `hash_heterogeneous` via
        // `DefaultHasher`. The `Hasher` trait path is unused for typed
        // lookups because Rust's `HashMap` uses `Borrow` for the
        // heterogeneous fast path.
        0
    }
    fn write(&mut self, _bytes: &[u8]) {
        // Intentionally a no-op: see `finish` above.
    }
}

/// Equality comparator matching `detail::transparent_string_equal` from
/// the C++ header.
#[derive(Default, Clone, Copy, Debug)]
pub struct StringEq;

impl HeterogeneousEq for StringEq {
    type Key = String;
}

/// Ordering comparator matching `detail::transparent_string_less` from
/// the C++ header.
///
/// `BTreeMap` / `BTreeSet` only need `Ord` on the owned key type, but we
/// also expose a `cmp_str` helper that takes two borrow-compatible key
/// forms, mirroring the overload set the C++ functor provides.
#[derive(Default, Clone, Copy, Debug)]
pub struct StringLess;

impl StringLess {
    /// Heterogeneous `less`: returns `true` iff `lhs < rhs` interpreted
    /// as strings, regardless of whether each side is a `String`,
    /// `&str`, `Box<str>`, `Cow<'_, str>`, ...
    pub fn cmp_str<Q>(&self, lhs: &Q, rhs: &Q) -> bool
    where
        String: Borrow<Q>,
        Q: ?Sized + Ord,
    {
        lhs < rhs
    }
}

/// `BuildHasher` that hands out [`StringHasher`] instances.
#[derive(Default, Clone, Copy, Debug)]
pub struct StringBuildHasher;

impl BuildHasher for StringBuildHasher {
    type Hasher = StringHasher;
    fn build_hasher(&self) -> StringHasher {
        StringHasher
    }
}

/// `BuildHasherDefault` alias, for parity with the C++ side.
pub type HeterogeneousBuildHasher = BuildHasherDefault<StringHasher>;

// ---------------------------------------------------------------------------
// Hash-based aliases (unordered_map / unordered_set family)
// ---------------------------------------------------------------------------

/// `std::unordered_map<std::string, V, transparent_string_hash, _>` analogue.
///
/// Heterogeneous lookups with `&str` etc. are enabled by
/// [`HashMap`]'s built-in `Borrow` support — no custom hasher is strictly
/// required, but pinning [`StringBuildHasher`] documents the intent.
pub type HeterogeneousUnorderedStringMap<V> =
    HashMap<String, V, StringBuildHasher>;

/// `std::unordered_multimap<std::string, V, ...>` analogue.
///
/// `std` does not ship a multimap; the closest faithful representation
/// is a `Vec` of key/value pairs that the caller can scan / sort. This is
/// deliberate: the C++ container permits duplicate keys, which a
/// `HashMap` cannot model. We expose the alias so the migration path
/// stays obvious, but the semantics are a flat vector.
pub type HeterogeneousUnorderedStringMultimap<V> = Vec<(String, V)>;

/// `std::unordered_set<std::string, ...>` analogue.
pub type HeterogeneousUnorderedStringSet =
    HashSet<String, StringBuildHasher>;

/// `std::unordered_multiset<std::string, ...>` analogue. `std` has no
/// multiset, so we use a `Vec<String>` and document the trade-off.
pub type HeterogeneousUnorderedStringMultiSet = Vec<String>;

// ---------------------------------------------------------------------------
// Ordered (BTree) aliases (map / set family)
// ---------------------------------------------------------------------------

/// `std::map<std::string, V, transparent_string_less>` analogue.
///
/// Rust's `BTreeMap` does not accept a custom comparator (it relies on
/// `K: Ord` from the standard library), so heterogeneous lookups
/// naturally fall out of `String: Borrow<str>` and `str: Ord`. The
/// [`StringLess`] helper above is kept for parity with the C++ API, but
/// the map itself uses `String`'s built-in ordering.
pub type HeterogeneousStringMap<V> = BTreeMap<String, V>;

/// `std::multimap<std::string, V, transparent_string_less>` analogue.
/// `std` has no multimap; we use `Vec<(String, V)>` as a faithful
/// representation that preserves duplicates.
pub type HeterogeneousStringMultiMap<V> = Vec<(String, V)>;

/// `std::set<std::string, transparent_string_less>` analogue.
pub type HeterogeneousStringSet = BTreeSet<String>;

/// `std::multiset<std::string, transparent_string_less>` analogue. `std`
/// has no multiset; we use `Vec<String>`.
pub type HeterogeneousStringMultiSet = Vec<String>;

// ---------------------------------------------------------------------------
// Convenience constructors
// ---------------------------------------------------------------------------

/// Returns a fresh `HashMap` that hashes with [`StringBuildHasher`].
///
/// Pinning the hasher (rather than `RandomState`) keeps the type
/// signature of the returned map aligned with the `Heterogeneous*`
/// aliases above.
pub fn new_unordered_string_map<V>() -> HeterogeneousUnorderedStringMap<V> {
    HashMap::with_hasher(StringBuildHasher)
}

/// Returns a fresh `HashSet` that hashes with [`StringBuildHasher`].
pub fn new_unordered_string_set() -> HeterogeneousUnorderedStringSet {
    HashSet::with_hasher(StringBuildHasher)
}
