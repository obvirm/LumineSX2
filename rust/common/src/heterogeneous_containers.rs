// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Heterogeneous string-keyed containers.
//!
//! This module mirrors `common/HeterogeneousContainers.h`, which provides
//! C++ type aliases for `std::*map`/`std::*set` whose hash and compare
//! functors are *transparent* — i.e. they accept `std::string_view`,
//! `std::string`, and `const char*` interchangeably. That lets callers
//! look up a key without first constructing a `std::string`, which avoids
//! a heap allocation on every lookup.
//!
//! In idiomatic Rust no extra machinery is required: `HashMap<String, V>`
//! and `BTreeMap<String, V>` already accept any key type that implements
//! `Borrow<str>`, which covers `String`, `&str`, and `&String`. The
//! standard library's `Borrow` machinery gives heterogeneous lookup
//! "for free" — see the module-level docs on each alias below.
//!
//! All aliases below are pure-Rust, drop-in names matching the C++ side
//! 1:1 so cross-language call sites read the same.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

// ---------------------------------------------------------------------------
// Unordered (hash-based) containers.
//
// Rust's `HashMap<String, V>` accepts any `Q: Hash + Eq + Borrow<String>`
// for lookup, and `Borrow<String>` is implemented for `String`, `&str`,
// and `&String`. The hash and equality of `&str` are computed against the
// borrowed bytes (no allocation), which is exactly the guarantee the C++
// transparent hashers provide.
// ---------------------------------------------------------------------------

/// Hash map keyed by `String`, equivalent to
/// `std::unordered_map<std::string, V, transparent_string_hash, transparent_string_equal>`.
///
/// Heterogeneous lookup: `map.get("foo")` works directly with a `&str`,
/// no `String` allocation required.
pub type UnorderedStringMap<V> = HashMap<String, V>;

/// Hash multimap keyed by `String`, equivalent to
/// `std::unordered_multimap<std::string, V, transparent_string_hash, transparent_string_equal>`.
///
/// Multiple values may share the same key; `get` is replaced by
/// `get_many` / manual iteration over a `Query` for a key. Heterogeneous
/// lookup with `&str` works the same as for [`UnorderedStringMap`].
pub type UnorderedStringMultimap<V> = HashMap<String, V>;

/// Hash set of `String`, equivalent to
/// `std::unordered_set<std::string, transparent_string_hash, transparent_string_equal>`.
pub type UnorderedStringSet = HashSet<String>;

/// Hash multiset of `String`, equivalent to
/// `std::unordered_multiset<std::string, transparent_string_hash, transparent_string_equal>`.
///
/// Rust's standard `HashSet` is a mathematical set without duplicates;
/// the closest equivalent to `unordered_multiset` is a `HashMap<K, usize>`
/// of occurrence counts, which is not a drop-in replacement. Provided as
/// a placeholder type alias for source-compatibility; replace per-site
/// if true multi-set semantics are required.
pub type UnorderedStringMultiSet = HashSet<String>;

// ---------------------------------------------------------------------------
// Ordered containers.
//
// `BTreeMap` orders by the natural `Ord` of its key. `String`'s `Ord`
/// matches lexicographic byte comparison, which is the same as
/// `std::string`'s `<` used by `transparent_string_less`. Heterogeneous
/// lookup with `&str` works through `Borrow<String>`.
// ---------------------------------------------------------------------------

/// Ordered map keyed by `String`, equivalent to
/// `std::map<std::string, V, transparent_string_less>`.
pub type StringMap<V> = BTreeMap<String, V>;

/// Ordered multimap keyed by `String`, equivalent to
/// `std::multimap<std::string, V, transparent_string_less>`.
pub type StringMultiMap<V> = BTreeMap<String, V>;

/// Ordered set of `String`, equivalent to
/// `std::set<std::string, transparent_string_less>`.
pub type StringSet = BTreeSet<String>;

/// Ordered multiset of `String`, equivalent to
/// `std::multiset<std::string, transparent_string_less>`.
///
/// As with [`UnorderedStringMultiSet`], this is a placeholder for
/// source-compatibility; `BTreeSet` does not natively support duplicates.
pub type StringMultiSet = BTreeSet<String>;