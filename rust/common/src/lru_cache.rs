//! LRU (least-recently-used) cache.
//!
//! Rust port of PCSX2's `common/LRUCache.h`. Provides a fixed-capacity
//! cache that evicts the least-recently-used entries when full. Every
//! successful [`LruCache::lookup_mut`] or [`LruCache::insert`] updates
//! the entry's monotonic `last_access` counter; [`LruCache::evict`]
//! then picks the entries with the lowest counters.
//!
//! C++ used `std::map<K, Item>` (or `StringMap<Item>` for string keys
//! to allow heterogeneous lookup). Rust's [`HashMap`] gives the same
//! semantics with `O(1)` average access; the per-eviction scan is
//! `O(n)`, which is unavoidable without a parallel priority queue.

use std::borrow::Borrow;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::hash::Hash;

/// One entry in the cache.
struct Item<V> {
    value: V,
    last_access: u64,
}

/// LRU cache with manual-eviction support.
///
/// * `K` — key type. Must be `Hash + Eq`; the methods that touch
///   several keys ([`evict`], [`insert`], etc.) additionally require
///   `K: Clone`.
/// * `V` — value type.
///
/// The `last_access` counter is a monotonic `u64` bumped on every
/// successful `lookup_mut` / `insert`; eviction picks the entry with
/// the smallest counter.
pub struct LruCache<K, V> {
    items: HashMap<K, Item<V>>,
    counter: u64,
    max_capacity: usize,
    manual_evict: bool,
}

impl<K, V> LruCache<K, V>
where
    K: Hash + Eq,
{
    /// Construct a cache that holds at most `max_capacity` entries.
    ///
    /// * `manual_evict = false` — entries are evicted automatically on
    ///   `insert` once the cap is reached (default C++ behaviour).
    /// * `manual_evict = true`  — the cache grows past the cap until
    ///   the caller invokes [`manual_evict`] or flips the flag via
    ///   [`set_manual_evict`](false).
    pub fn new(max_capacity: usize, manual_evict: bool) -> Self {
        Self {
            items: HashMap::with_capacity(max_capacity),
            counter: 0,
            max_capacity,
            manual_evict,
        }
    }

    /// Number of entries currently in the cache.
    #[inline]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// `true` if the cache holds no entries.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Configured maximum capacity.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.max_capacity
    }

    /// `true` if automatic eviction is currently enabled.
    #[inline]
    pub fn is_manual_evict(&self) -> bool {
        self.manual_evict
    }

    /// Drop all entries. Counters and capacity are preserved.
    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// Resize the cap. If the new cap is smaller than the current
    /// size, the oldest entries are evicted immediately.
    pub fn set_capacity(&mut self, capacity: usize)
    where
        K: Clone,
    {
        self.max_capacity = capacity;
        if self.items.len() > self.max_capacity {
            let extra = self.items.len() - self.max_capacity;
            self.evict(extra);
        }
    }

    /// Look up a key **without** mutating `last_access`. Use
    /// [`lookup_mut`](Self::lookup_mut) for the LRU touch.
    ///
    /// Heterogeneous: any `Q` that `K` can `Borrow` (e.g.
    /// `&str` for `K = String`) is accepted.
    pub fn peek<Q>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.items.get(key).map(|item| &item.value)
    }

    /// Look up a key and bump its `last_access` counter. Returns
    /// `None` if the key is not present.
    ///
    /// Note: returns `&mut V` (not `&V`) because bumping the counter
    /// requires `&mut self`. Callers that only want a shared view
    /// should use [`peek`](Self::peek).
    pub fn lookup_mut<Q>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        // Bump counter first to release the borrow on `self.items`.
        let access = self.next_counter();
        let item = self.items.get_mut(key)?;
        item.last_access = access;
        Some(&mut item.value)
    }

    /// Insert (or replace) a key/value pair, bump `last_access`, and
    /// return a mutable reference to the stored value.
    ///
    /// If the cache is at capacity and auto-eviction is enabled, one
    /// entry is evicted first to make room.
    pub fn insert(&mut self, key: K, value: V) -> &mut V
    where
        K: Clone,
    {
        self.shrink_for_new_item();
        self.counter += 1;
        let last_access = self.counter;
        let item = match self.items.entry(key) {
            Entry::Occupied(mut o) => {
                let slot = o.get_mut();
                slot.value = value;
                slot.last_access = last_access;
                o.into_mut()
            }
            Entry::Vacant(v) => v.insert(Item { value, last_access }),
        };
        &mut item.value
    }

    /// Evict up to `count` least-recently-used entries. Stops early
    /// if the cache runs out of items.
    pub fn evict(&mut self, mut count: usize)
    where
        K: Clone,
    {
        while count > 0 {
            // Pick the entry with the smallest `last_access`; clone the
            // key out so the borrow on `self.items` is released before
            // we call `remove`.
            let Some(key) = self
                .items
                .iter()
                .min_by_key(|(_, item)| item.last_access)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            self.items.remove(&key);
            count -= 1;
        }
    }

    /// Remove the entry for `key`. Returns `true` if a key was removed.
    pub fn remove<Q>(&mut self, key: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.items.remove(key).is_some()
    }

    /// Toggle automatic eviction. When transitioning from `true` to
    /// `false`, runs [`manual_evict`](Self::manual_evict) to enforce
    /// the cap immediately.
    pub fn set_manual_evict(&mut self, block: bool)
    where
        K: Clone,
    {
        self.manual_evict = block;
        if !self.manual_evict {
            self.run_manual_evict();
        }
    }

    /// Evict down to the configured cap if currently over.
    pub fn manual_evict(&mut self)
    where
        K: Clone,
    {
        self.run_manual_evict();
    }

    fn run_manual_evict(&mut self)
    where
        K: Clone,
    {
        while self.items.len() > self.max_capacity {
            let extra = self.items.len() - self.max_capacity;
            self.evict(extra);
        }
    }

    fn shrink_for_new_item(&mut self)
    where
        K: Clone,
    {
        if self.items.len() < self.max_capacity {
            return;
        }
        // Cache is at or above the cap: evict one slot's worth of the
        // oldest entries so the new item fits. Mirrors C++'s
        // `m_items.size() - (m_max_capacity - 1)`.
        let to_evict = self.items.len() - (self.max_capacity - 1);
        self.evict(to_evict);
    }

    #[inline]
    fn next_counter(&mut self) -> u64 {
        self.counter += 1;
        self.counter
    }
}

impl<K, V> Default for LruCache<K, V>
where
    K: Hash + Eq,
{
    fn default() -> Self {
        // Match the C++ default of `LRUCache(max_capacity=16, manual_evict=false)`.
        Self::new(16, false)
    }
}

// ─── FFI surface ─────────────────────────────────────────────────────
//
// PCSX2's C++ side uses `LRUCache` internally; this section exposes a
// C-ABI handle for the most common shape (string key, u64 value) so
// callers can interop without dragging in a templated header.
//
// All `unsafe` is contained below; the idiomatic Rust API above is
// sound. Null pointers are treated as a no-op rather than UB to keep
// the C++ side forgiving during teardown.

/// Opaque handle to a `LruCache<String, u64>`.
///
/// Returned by [`pcsx2_lru_cache_u64_create`] and consumed by
/// [`pcsx2_lru_cache_u64_destroy`]. The C++ side treats this as an
/// opaque pointer; its layout is `#[repr(C)]` but its fields are
/// intentionally inaccessible from C++.
#[repr(C)]
pub struct LruCacheStringU64 {
    _private: [u8; 0],
}

/// Create a new `LruCache<String, u64>` with the given capacity and
/// `manual_evict = false`. Returns a non-null handle on success, or
/// a null pointer on allocation failure.
#[no_mangle]
pub extern "C" fn pcsx2_lru_cache_u64_create(capacity: u32) -> *mut LruCacheStringU64 {
    let cache = Box::new(LruCache::<String, u64>::new(capacity as usize, false));
    // SAFETY: `LruCacheStringU64` is a zero-sized marker; the pointer
    // returned is the same as the `LruCache` pointer modulo the cast.
    let raw = Box::into_raw(cache) as *mut LruCacheStringU64;
    raw
}

/// Free a cache previously returned by [`pcsx2_lru_cache_u64_create`].
/// Safe to call with a null pointer (no-op).
#[no_mangle]
pub extern "C" fn pcsx2_lru_cache_u64_destroy(cache: *mut LruCacheStringU64) {
    if cache.is_null() {
        return;
    }
    // SAFETY: caller promises the pointer was produced by
    // `pcsx2_lru_cache_u64_create` (or is null) and is not aliased.
    unsafe {
        let raw = cache as *mut LruCache<String, u64>;
        drop(Box::from_raw(raw));
    }
}

/// Insert `(key, value)` into the cache. Bumps `last_access` on the
/// new entry and evicts the oldest if the cap is exceeded.
///
/// `key` must be a NUL-terminated C string. Invalid UTF-8 is silently
/// dropped (the call is a no-op). Null `cache` or `key` is a no-op.
#[no_mangle]
pub extern "C" fn pcsx2_lru_cache_u64_insert(
    cache: *mut LruCacheStringU64,
    key: *const std::os::raw::c_char,
    value: u64,
) {
    if cache.is_null() || key.is_null() {
        return;
    }
    // SAFETY: `cache` is a live `LruCache<String, u64>` handle from
    // `pcsx2_lru_cache_u64_create`; `key` is a valid NUL-terminated
    // C string for the duration of this call.
    unsafe {
        let cstr = std::ffi::CStr::from_ptr(key);
        let Ok(key) = cstr.to_str() else { return };
        let raw = cache as *mut LruCache<String, u64>;
        let cache = &mut *raw;
        cache.insert(key.to_owned(), value);
    }
}

/// Look up `key` in the cache. Bumps `last_access` on hit.
///
/// Returns `0` if the key was **not** present, `1` if it was. (The
/// 0/1 boolean is intentional: a `u64` value of 0 is a legitimate
/// cached value, so it cannot double as a "not found" sentinel. If
/// the C++ side needs the value, it should track it itself or extend
/// the API with a dedicated getter.)
#[no_mangle]
pub extern "C" fn pcsx2_lru_cache_u64_lookup(
    cache: *mut LruCacheStringU64,
    key: *const std::os::raw::c_char,
) -> u64 {
    if cache.is_null() || key.is_null() {
        return 0;
    }
    // SAFETY: see `pcsx2_lru_cache_u64_insert`.
    unsafe {
        let cstr = std::ffi::CStr::from_ptr(key);
        let Ok(key) = cstr.to_str() else { return 0 };
        let raw = cache as *mut LruCache<String, u64>;
        let cache = &mut *raw;
        if cache.lookup_mut(key).is_some() { 1 } else { 0 }
    }
}
