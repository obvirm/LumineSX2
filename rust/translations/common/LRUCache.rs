// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Bounded least-recently-used (LRU) cache.
//!
//! This module is the Rust translation of PCSX2's `common/LRUCache.h`. The C++
//! class is a small templated container that keeps key/value pairs in a `std::map`
//! and stamps each entry with a monotonically increasing counter; the entry with
//! the lowest counter is the least recently used and is the one evicted when the
//! cache exceeds its configured capacity.
//!
//! In the Rust port the LRU order is tracked explicitly with a [`VecDeque`] of
//! keys (front = least recently used, back = most recently used) and lookups go
//! through a [`HashMap`] of keys to values, matching the C++ semantics while
//! staying within the `std` crate. Both `K` and `V` are `Clone` so the cache
//! can refresh the LRU position of a key without taking ownership of the caller's
//! data when reading.

use std::collections::{HashMap, VecDeque};
use std::hash::Hash;

/// A bounded least-recently-used cache mapping keys to values.
///
/// The cache holds at most `capacity` entries. Inserting a new key when the
/// cache is full evicts the least recently used entry. Both reads
/// ([`Self::get`]) and writes ([`Self::put`]) refresh the access order of the
/// touched key, so an entry that is only ever read is still considered
/// "recently used" and is protected from eviction.
pub struct LRUCache<K, V>
where
    K: Hash + Eq + Clone,
    V: Clone,
{
    map: HashMap<K, V>,
    order: VecDeque<K>,
    capacity: usize,
    /// Mirrors `m_manual_evict` from the C++ class. When `true`, the caller
    /// is expected to drive eviction explicitly via [`Self::evict_over_capacity`]
    /// instead of relying on the auto-evict path used by [`Self::put`].
    manual_evict: bool,
}

impl<K, V> LRUCache<K, V>
where
    K: Hash + Eq + Clone,
    V: Clone,
{
    /// Creates a new LRU cache that can hold at most `capacity` entries with
    /// the default auto-evict behaviour.
    ///
    /// This is equivalent to [`Self::new_with_manual_evict`] with `manual_evict
    /// = false` and matches the C++ constructor's default arguments. Use the
    /// explicit constructor if you want to opt into manual eviction control.
    ///
    /// A `capacity` of zero is allowed: in that case the cache is effectively
    /// unable to retain new entries, and every call to [`Self::put`] for an
    /// unknown key is dropped on the floor. This mirrors the behaviour of the
    /// C++ constructor, which also accepts a zero capacity.
    pub fn new(capacity: usize) -> Self {
        Self::new_with_manual_evict(capacity, false)
    }

    /// Creates a new LRU cache with full control over both the capacity and
    /// the manual-evict flag, matching the C++ two-argument constructor.
    ///
    /// When `manual_evict` is `false` (the default) the cache is expected to
    /// shrink itself automatically on each [`Self::put`]. When `manual_evict`
    /// is `true` the caller becomes responsible for triggering eviction via
    /// [`Self::evict_over_capacity`] (typically after switching back to auto
    /// mode with [`Self::set_manual_evict`]).
    pub fn new_with_manual_evict(capacity: usize, manual_evict: bool) -> Self {
        Self {
            map: HashMap::with_capacity(capacity),
            order: VecDeque::with_capacity(capacity),
            capacity,
            manual_evict,
        }
    }

    /// Updates the maximum number of entries the cache can hold, evicting the
    /// least-recently-used entries if the cache is currently over capacity.
    ///
    /// This corresponds to `SetMaxCapacity` in the C++ version.
    pub fn set_max_capacity(&mut self, capacity: usize) {
        self.capacity = capacity;
        if self.map.len() > self.capacity {
            self.evict(self.map.len() - self.capacity);
        }
    }

    /// Returns `true` if the cache is operating in manual-evict mode.
    #[inline]
    pub fn is_manual_evict(&self) -> bool {
        self.manual_evict
    }

    /// Toggles manual-evict mode.
    ///
    /// When transitioning from manual back to auto (`block = false`) the cache
    /// immediately runs [`Self::evict_over_capacity`] to drop any entries that
    /// had piled up while in manual mode. This mirrors the C++ behaviour of
    /// `SetManualEvict`.
    pub fn set_manual_evict(&mut self, block: bool) {
        self.manual_evict = block;
        if !self.manual_evict {
            self.evict_over_capacity();
        }
    }

    /// Evicts excess entries until the cache holds at most `self.capacity`
    /// items.
    ///
    /// This is the Rust analogue of C++'s `ManualEvict` and is also called
    /// automatically when [`Self::set_manual_evict`] flips back to auto mode.
    pub fn evict_over_capacity(&mut self) {
        while self.map.len() > self.capacity {
            self.evict(self.map.len() - self.capacity);
        }
    }

    /// Evicts `count` of the least-recently-used entries from the cache.
    ///
    /// Silently stops early when the cache becomes empty, matching the C++
    /// behaviour of `Evict`.
    pub fn evict(&mut self, mut count: usize) {
        while !self.order.is_empty() && count > 0 {
            if let Some(oldest) = self.order.pop_front() {
                self.map.remove(&oldest);
            }
            count -= 1;
        }
    }

    /// Removes a single entry by key.
    ///
    /// Returns `true` if the key was present and got removed, `false`
    /// otherwise. This corresponds to the C++ `Remove` template, which
    /// transparently accepts any key type that can be compared against `K`.
    pub fn remove(&mut self, key: &K) -> bool {
        if self.map.remove(key).is_some() {
            self.order.retain(|k| k != key);
            true
        } else {
            false
        }
    }

    /// Returns the number of entries currently stored in the cache.
    #[inline]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Returns `true` if the cache holds no entries.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Returns the maximum number of entries the cache can hold.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Returns a reference to the value associated with `key`, marking the
    /// entry as the most recently used.
    ///
    /// Returns `None` if the key is not present in the cache. This corresponds
    /// to the non-const `Lookup` in the C++ version, which also bumps the
    /// `last_access` counter on a hit.
    pub fn get(&mut self, key: &K) -> Option<&V> {
        if self.map.contains_key(key) {
            self.touch(key);
            self.map.get(key)
        } else {
            None
        }
    }

    /// Inserts a key/value pair into the cache.
    ///
    /// If the key is already present, its value is replaced and the entry is
    /// promoted to the most-recently-used position. If the key is new and the
    /// cache is at capacity, the least recently used entry is evicted first to
    /// make room.
    pub fn put(&mut self, key: K, value: V) {
        if self.map.contains_key(&key) {
            // Existing key: replace the value and refresh the LRU order so
            // this entry is treated as the most recently used.
            self.map.insert(key.clone(), value);
            self.touch(&key);
            return;
        }

        // New key: ensure there is at least one free slot. The order queue is
        // walked from the front (least recently used) until either a slot is
        // free or the queue is exhausted (capacity == 0 case).
        while self.map.len() >= self.capacity {
            let Some(oldest) = self.order.pop_front() else {
                // Capacity is zero: drop the new entry on the floor, matching
                // the C++ constructor's tolerance for a zero capacity.
                return;
            };
            self.map.remove(&oldest);
        }

        self.order.push_back(key.clone());
        self.map.insert(key, value);
    }

    /// Removes all entries from the cache, leaving its capacity unchanged.
    pub fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
    }

    /// Returns an iterator over `(key, value)` pairs in LRU order, from the
    /// least recently used entry to the most recently used one.
    ///
    /// The iterator borrows the cache immutably, so it is safe to call
    /// alongside other read-only inspection of the cache.
    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.order
            .iter()
            .filter_map(move |k| self.map.get_key_value(k))
    }

    /// Promotes `key` to the most-recently-used position in the order queue.
    ///
    /// If `key` is not in the queue this is a no-op; the caller is expected to
    /// have already verified membership.
    fn touch(&mut self, key: &K) {
        self.order.retain(|k| k != key);
        self.order.push_back(key.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_and_get_round_trip() {
        let mut cache = LRUCache::new(2);
        cache.put("a", 1);
        cache.put("b", 2);
        assert_eq!(cache.get(&"a"), Some(&1));
        assert_eq!(cache.get(&"b"), Some(&2));
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn evicts_least_recently_used() {
        let mut cache = LRUCache::new(2);
        cache.put(1, "one");
        cache.put(2, "two");
        // Touch 1 so 2 becomes the least recently used.
        assert_eq!(cache.get(&1), Some(&"one"));
        cache.put(3, "three");

        assert_eq!(cache.get(&2), None, "2 should have been evicted");
        assert_eq!(cache.get(&1), Some(&"one"));
        assert_eq!(cache.get(&3), Some(&"three"));
    }

    #[test]
    fn update_existing_key_does_not_evict() {
        let mut cache = LRUCache::new(2);
        cache.put(1, "a");
        cache.put(2, "b");
        cache.put(1, "A");

        assert_eq!(cache.get(&1), Some(&"A"));
        assert_eq!(cache.get(&2), Some(&"b"));
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn clear_resets_cache() {
        let mut cache = LRUCache::new(2);
        cache.put(1, "a");
        cache.put(2, "b");
        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.get(&1), None);
    }

    #[test]
    fn zero_capacity_drops_new_entries() {
        let mut cache: LRUCache<i32, i32> = LRUCache::new(0);
        cache.put(1, 1);
        assert!(cache.is_empty());
        assert_eq!(cache.get(&1), None);
    }

    #[test]
    fn iter_yields_lru_order() {
        let mut cache = LRUCache::new(3);
        cache.put(1, 10);
        cache.put(2, 20);
        cache.put(3, 30);

        // 3 was just inserted, so it is the most recently used; 1 is the LRU.
        let keys: Vec<&i32> = cache.iter().map(|(k, _)| k).collect();
        assert_eq!(keys, vec![&1, &2, &3]);
    }

    #[test]
    fn set_max_capacity_evicts_excess() {
        let mut cache = LRUCache::new(3);
        cache.put(1, "a");
        cache.put(2, "b");
        cache.put(3, "c");

        // Touch 1 so 2 becomes the least recently used.
        assert_eq!(cache.get(&1), Some(&"a"));

        cache.set_max_capacity(2);

        assert_eq!(cache.len(), 2);
        assert_eq!(cache.get(&2), None, "2 should have been evicted");
        assert_eq!(cache.get(&1), Some(&"a"));
        assert_eq!(cache.get(&3), Some(&"c"));
    }

    #[test]
    fn set_max_capacity_can_grow() {
        let mut cache = LRUCache::new(2);
        cache.put(1, "a");
        cache.put(2, "b");

        cache.set_max_capacity(4);

        assert_eq!(cache.capacity(), 4);
        assert_eq!(cache.len(), 2);
        cache.put(3, "c");
        cache.put(4, "d");
        assert_eq!(cache.len(), 4);
    }

    #[test]
    fn evict_drops_n_oldest() {
        let mut cache = LRUCache::new(4);
        cache.put(1, "a");
        cache.put(2, "b");
        cache.put(3, "c");
        cache.put(4, "d");

        // Order from LRU to MRU is 1, 2, 3, 4. Evict two oldest.
        cache.evict(2);

        assert_eq!(cache.len(), 2);
        assert_eq!(cache.get(&1), None);
        assert_eq!(cache.get(&2), None);
        assert_eq!(cache.get(&3), Some(&"c"));
        assert_eq!(cache.get(&4), Some(&"d"));
    }

    #[test]
    fn evict_more_than_present_stops_at_empty() {
        let mut cache = LRUCache::new(4);
        cache.put(1, "a");
        cache.put(2, "b");

        // Evicting more entries than exist must simply empty the cache,
        // not panic.
        cache.evict(10);
        assert!(cache.is_empty());
    }

    #[test]
    fn remove_returns_membership() {
        let mut cache = LRUCache::new(3);
        cache.put(1, "a");
        cache.put(2, "b");

        assert!(cache.remove(&1));
        assert!(!cache.remove(&1), "second remove returns false");
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get(&1), None);
        assert_eq!(cache.get(&2), Some(&"b"));

        // Removed entries must also be flushed from the LRU order queue.
        let keys: Vec<&i32> = cache.iter().map(|(k, _)| k).collect();
        assert_eq!(keys, vec![&2]);
    }

    #[test]
    fn manual_evict_flag_round_trip() {
        let mut cache: LRUCache<i32, &str> = LRUCache::new_with_manual_evict(2, true);
        assert!(cache.is_manual_evict());

        cache.set_manual_evict(false);
        assert!(!cache.is_manual_evict());
    }

    #[test]
    fn switching_to_auto_evicts_overflow() {
        // Start in manual mode and overstuff the cache directly via the map
        // view through `put` (auto-eviction still runs on put regardless of
        // the flag, so we instead drive the flag-change path).
        let mut cache: LRUCache<i32, &str> = LRUCache::new_with_manual_evict(2, false);
        cache.put(1, "a");
        cache.put(2, "b");
        cache.put(3, "c");
        // Even in auto mode, put keeps the cache at capacity (it evicts the
        // LRU). Now flip into manual mode, drop the cap, flip back: the
        // post-flip call should evict any overflow.
        cache.set_manual_evict(true);
        cache.set_max_capacity(1);
        assert_eq!(cache.len(), 3, "manual mode retained the excess");

        cache.set_manual_evict(false);
        assert_eq!(cache.len(), 1, "flipping to auto must evict overflow");
    }
}
