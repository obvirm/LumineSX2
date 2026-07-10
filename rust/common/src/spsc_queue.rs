// SPDX-FileCopyrightText: 2024 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
// Port of common/boost_spsc_queue.hpp (ringbuffer_base) to Rust.
// Lock-free single-producer single-consumer ringbuffer.
//
// Original C++ code (c) 2009-2013 Tim Blechmann, Boost Software License 1.0.
// This Rust port implements the same algorithm with a Rustic API.

use core::cell::UnsafeCell;
use core::fmt;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicUsize, Ordering};

/// Cache line size for x86_64 (64 bytes).
const CACHE_LINE_SIZE: usize = 64;

/// Pads to a full cache line to prevent false sharing between atomics.
#[repr(align(64))]
struct CachePad([u8; CACHE_LINE_SIZE]);

/// Lock-free single-producer single-consumer ringbuffer.
///
/// - `T`: element type
/// - `CAPACITY`: maximum number of elements (must be > 0)
///
/// # Thread Safety
/// - `push` — producer thread only
/// - `pop`, `front`, `front_mut`, `pop_front`, `consume_one` — consumer thread only
/// - Concurrent `push` (producer) + `pop` (consumer) is safe.
/// - All other combinations are **unsafe**.
pub struct RingBuffer<T, const CAPACITY: usize> {
    /// Modulo increment helper.
    /// Write index — only written by producer (relaxed), read by consumer (acquire).
    write_index: AtomicUsize,
    _pad1: CachePad,
    /// Read index — only written by consumer (release), read by producer (acquire).
    read_index: AtomicUsize,
    _pad2: CachePad,
    /// Cached read index for two-phase pop — consumer only, not atomic.
    pending_pop_read_index: UnsafeCell<usize>,
    /// Storage. Elements are constructed in-place (push) and destructed (pop).
    buffer: Box<[MaybeUninit<T>]>,
}

// SAFETY: as long as T: Send, the ringbuffer is Send (only one producer, one consumer).
unsafe impl<T: Send, const CAPACITY: usize> Send for RingBuffer<T, CAPACITY> {}
unsafe impl<T: Sync, const CAPACITY: usize> Sync for RingBuffer<T, CAPACITY> {}

impl<T, const CAPACITY: usize> RingBuffer<T, CAPACITY> {
    /// Creates a new empty ringbuffer.
    ///
    /// # Panics
    /// Panics if `CAPACITY == 0`.
    pub fn new() -> Self {
        assert!(CAPACITY > 0, "RingBuffer capacity must be > 0");
        let mut buf = Vec::with_capacity(CAPACITY);
        // SAFETY: all slots are MaybeUninit which is valid for any pattern.
        unsafe {
            buf.set_len(CAPACITY);
        }
        Self {
            write_index: AtomicUsize::new(0),
            _pad1: CachePad([0; CACHE_LINE_SIZE]),
            read_index: AtomicUsize::new(0),
            _pad2: CachePad([0; CACHE_LINE_SIZE]),
            pending_pop_read_index: UnsafeCell::new(0),
            buffer: buf.into_boxed_slice(),
        }
    }

    #[inline]
    fn next_index(index: usize) -> usize {
        (index + 1) % CAPACITY
    }

    /// Helper: raw pointer to element at `index`.
    fn elem_ptr(&self, index: usize) -> *mut T {
        unsafe {
            let base = self.buffer.as_ptr() as *mut MaybeUninit<T>;
            (*base.add(index)).as_mut_ptr()
        }
    }

    /// Helper: read (move) value out of slot at `index` without dropping.
    unsafe fn read_elem(&self, index: usize) -> T {
        self.elem_ptr(index).read()
    }

    /// Helper: write value into slot at `index` without dropping old value.
    unsafe fn write_elem(&self, index: usize, value: T) {
        self.elem_ptr(index).write(value);
    }

    /// Push `value` into the buffer.
    ///
    /// Returns `false` if the buffer is full.
    ///
    /// # Thread Safety
    /// Producer thread only.
    pub fn push(&self, value: T) -> bool {
        let write_index = self.write_index.load(Ordering::Relaxed);
        let next = Self::next_index(write_index);

        if next == self.read_index.load(Ordering::Acquire) {
            return false;
        }

        // SAFETY: write_index < CAPACITY guaranteed by next_index.
        unsafe { self.write_elem(write_index, value); }
        self.write_index.store(next, Ordering::Release);
        true
    }

    /// Pop a value from the buffer.
    ///
    /// Returns `None` if empty.
    ///
    /// # Thread Safety
    /// Consumer thread only.
    pub fn pop(&self) -> Option<T> {
        let write_index = self.write_index.load(Ordering::Acquire);
        let read_index = self.read_index.load(Ordering::Relaxed);

        if write_index == read_index {
            return None;
        }

        // SAFETY: read_index is valid; we read (move) the element out.
        let value = unsafe { self.read_elem(read_index) };
        let next = Self::next_index(read_index);
        self.read_index.store(next, Ordering::Release);
        Some(value)
    }

    /// Returns a shared reference to the front element.
    ///
    /// # Panics
    /// Panics if empty.
    ///
    /// # Thread Safety
    /// Consumer thread only.
    pub fn front(&self) -> &T {
        let read_index = self.read_index.load(Ordering::Relaxed);
        // SAFETY: caller ensures not empty, so read_index is valid.
        unsafe { &*self.elem_ptr(read_index) }
    }

    /// Returns a mutable reference to the front element, remembering the
    /// read index for a subsequent `pop_front()`.
    ///
    /// # Panics
    /// Panics if empty.
    ///
    /// # Safety
    /// This uses raw pointer manipulation to obtain &mut T from &self.
    /// This is safe because:
    /// - Consumer thread has exclusive access to both read_index and the element
    /// - Producer thread will never access this slot until after consumer advances read_index
    ///
    /// # Thread Safety
    /// Consumer thread only.
    pub fn front_mut(&self) -> &mut T {
        let read_index = self.read_index.load(Ordering::Relaxed);
        unsafe {
            *self.pending_pop_read_index.get() = read_index;
            // Get a raw pointer to the buffer element, then deref as &mut T
            let base_ptr = self.buffer.as_ptr() as *mut MaybeUninit<T>;
            let elem_ptr = base_ptr.add(read_index);
            &mut *(*elem_ptr).as_mut_ptr()
        }
    }

    /// Completes a two-phase pop started by `front_mut()`.
    ///
    /// # Thread Safety
    /// Consumer thread only — must be called after `front_mut()`.
    pub fn pop_front(&self) {
        let pending = unsafe { *self.pending_pop_read_index.get() };
        // SAFETY: read (move + drop) the element at pending index.
        unsafe { self.read_elem(pending); }
        let next = Self::next_index(pending);
        self.read_index.store(next, Ordering::Release);
    }

    /// Consumes the front element by passing it to `f`.
    ///
    /// Returns `false` if empty.
    ///
    /// # Thread Safety
    /// Consumer thread only.
    pub fn consume_one<F>(&self, f: F) -> bool
    where
        F: FnOnce(&T),
    {
        let write_index = self.write_index.load(Ordering::Acquire);
        let read_index = self.read_index.load(Ordering::Relaxed);

        if write_index == read_index {
            return false;
        }

        unsafe {
            f(&*self.elem_ptr(read_index));
            // Read (move + drop) the element.
            self.read_elem(read_index);
        }

        let next = Self::next_index(read_index);
        self.read_index.store(next, Ordering::Release);
        true
    }

    /// Resets the buffer to an empty state.
    ///
    /// **Not thread-safe** — must not be called concurrently with push/pop.
    pub fn reset(&self) {
        // Drain remaining items.
        while self.pop().is_some() {}
        self.write_index.store(0, Ordering::Relaxed);
        self.read_index.store(0, Ordering::Release);
    }

    /// Returns `true` if the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.write_index.load(Ordering::Relaxed) == self.read_index.load(Ordering::Relaxed)
    }

    /// Returns `true` if the buffer is full.
    pub fn is_full(&self) -> bool {
        let w = self.write_index.load(Ordering::Relaxed);
        let r = self.read_index.load(Ordering::Relaxed);
        Self::next_index(w) == r
    }

    /// Returns `true` if the implementation is lock-free.
    /// On x86_64, `AtomicUsize` is always lock-free.
    pub fn is_lock_free() -> bool {
        true
    }

    /// Returns the number of elements currently stored.
    pub fn len(&self) -> usize {
        let w = self.write_index.load(Ordering::Relaxed);
        let r = self.read_index.load(Ordering::Relaxed);
        if r > w {
            (w + CAPACITY) - r
        } else {
            w - r
        }
    }

    /// Returns the maximum capacity.
    pub const fn capacity(&self) -> usize {
        CAPACITY
    }
}

impl<T, const CAPACITY: usize> Drop for RingBuffer<T, CAPACITY> {
    fn drop(&mut self) {
        while self.pop().is_some() {}
    }
}

impl<T, const CAPACITY: usize> fmt::Debug for RingBuffer<T, CAPACITY> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RingBuffer")
            .field("capacity", &CAPACITY)
            .field("len", &self.len())
            .field("empty", &self.is_empty())
            .field("full", &self.is_full())
            .finish()
    }
}

impl<T, const CAPACITY: usize> Default for RingBuffer<T, CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_pop_simple() {
        let rb: RingBuffer<i32, 4> = RingBuffer::new();
        assert!(rb.is_empty());
        assert!(!rb.is_full());

        assert!(rb.push(1));
        assert!(rb.push(2));
        assert!(rb.push(3));
        assert!(rb.push(4));
        assert!(rb.is_full());
        assert!(!rb.push(5)); // full

        assert_eq!(rb.pop(), Some(1));
        assert_eq!(rb.pop(), Some(2));
        assert_eq!(rb.pop(), Some(3));
        assert_eq!(rb.pop(), Some(4));
        assert_eq!(rb.pop(), None);
        assert!(rb.is_empty());
    }

    #[test]
    fn test_wrap_around() {
        let rb: RingBuffer<i32, 3> = RingBuffer::new();
        assert!(rb.push(1));
        assert!(rb.push(2));
        assert_eq!(rb.pop(), Some(1));
        assert!(rb.push(3));
        assert_eq!(rb.pop(), Some(2));
        assert!(rb.push(4));
        assert_eq!(rb.pop(), Some(3));
        assert_eq!(rb.pop(), Some(4));
        assert!(rb.is_empty());
    }

    #[test]
    fn test_consume_one() {
        let rb: RingBuffer<i32, 4> = RingBuffer::new();
        rb.push(10);
        rb.push(20);

        let mut sum = 0;
        assert!(rb.consume_one(|v| sum += *v));
        assert_eq!(sum, 10);
        assert!(rb.consume_one(|v| sum += *v));
        assert_eq!(sum, 30);
        assert!(!rb.consume_one(|_| {}));
    }

    #[test]
    fn test_front_and_pop_front() {
        let rb: RingBuffer<String, 4> = RingBuffer::new();
        rb.push("hello".to_string());
        rb.push("world".to_string());

        {
            let front = rb.front_mut();
            front.push_str("!!!");
        }
        rb.pop_front();

        assert_eq!(rb.pop(), Some("world".to_string()));
        assert_eq!(rb.pop(), None);
    }

    #[test]
    fn test_reset() {
        let rb: RingBuffer<i32, 4> = RingBuffer::new();
        rb.push(1);
        rb.push(2);
        rb.push(3);
        rb.reset();
        assert!(rb.is_empty());
        assert_eq!(rb.pop(), None);
    }

    #[test]
    fn test_empty_after_full_drain() {
        let rb: RingBuffer<i32, 8> = RingBuffer::new();
        for i in 0..8 {
            assert!(rb.push(i));
        }
        for _ in 0..8 {
            assert!(rb.pop().is_some());
        }
        assert!(rb.is_empty());
        assert_eq!(rb.pop(), None);
    }

    #[test]
    fn test_string_type() {
        let rb: RingBuffer<String, 4> = RingBuffer::new();
        rb.push("foo".to_string());
        rb.push("bar".to_string());
        assert_eq!(rb.pop(), Some("foo".to_string()));
        assert_eq!(rb.pop(), Some("bar".to_string()));
    }

    #[test]
    fn test_capacity_and_len() {
        let rb: RingBuffer<i32, 16> = RingBuffer::new();
        assert_eq!(rb.capacity(), 16);
        assert_eq!(rb.len(), 0);
        rb.push(42);
        assert_eq!(rb.len(), 1);
        rb.pop();
        assert_eq!(rb.len(), 0);
    }

    #[test]
    fn test_debug() {
        let rb: RingBuffer<i32, 4> = RingBuffer::new();
        let d = format!("{:?}", rb);
        assert!(d.contains("capacity"));
        assert!(d.contains("len"));
    }

    #[test]
    fn test_default() {
        let rb: RingBuffer<i32, 8> = RingBuffer::default();
        assert!(rb.is_empty());
        assert_eq!(rb.capacity(), 8);
    }

    #[test]
    fn test_multi_thread() {
        use std::sync::Arc;
        use std::thread;

        const N: usize = 10000;
        let rb = Arc::new(RingBuffer::<i32, 128>::new());

        let producer = {
            let rb = rb.clone();
            thread::spawn(move || {
                for i in 0..N {
                    while !rb.push(i as i32) {
                        thread::yield_now();
                    }
                }
            })
        };

        let consumer = {
            let rb = rb.clone();
            thread::spawn(move || {
                let mut sum = 0i64;
                let mut count = 0;
                while count < N {
                    if let Some(v) = rb.pop() {
                        sum += v as i64;
                        count += 1;
                    } else {
                        thread::yield_now();
                    }
                }
                sum
            })
        };

        producer.join().unwrap();
        let sum = consumer.join().unwrap();
        // Sum of 0..N-1 = N*(N-1)/2
        let expected = ((N - 1) * N / 2) as i64;
        assert_eq!(sum, expected);
    }

    #[test]
    fn test_multi_thread_stress() {
        use std::sync::Arc;
        use std::thread;

        const N: usize = 50000;
        let rb = Arc::new(RingBuffer::<u64, 256>::new());

        let p = {
            let rb = rb.clone();
            thread::spawn(move || {
                for i in 0u64..N as u64 {
                    while !rb.push(i) {
                        thread::yield_now();
                    }
                }
            })
        };

        let c = {
            let rb = rb.clone();
            thread::spawn(move || {
                let mut last: i64 = -1;
                let mut count = 0;
                while count < N {
                    if let Some(v) = rb.pop() {
                        assert!((v as i64) > last, "out of order: {} <= {}", v, last);
                        last = v as i64;
                        count += 1;
                    } else {
                        thread::yield_now();
                    }
                }
            })
        };

        p.join().unwrap();
        c.join().unwrap();
    }
}
