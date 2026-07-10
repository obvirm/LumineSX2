// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Thin Rust port of `common/HeapArray.h`.
//!
//! This module is largely a compatibility shim. Both C++ classes in the
//! original header — `FixedHeapArray<T, SIZE, ALIGNMENT>` and
//! `DynamicHeapArray<T, alignment>` — are RAII wrappers over a heap-allocated
//! buffer. In Rust the standard library already provides exactly that:
//! [`Vec<T>`] for the resizable case and [`Box<[T; SIZE]>`] for the fixed-size
//! case. Rather than reimplement the allocator logic, we expose those types
//! under familiar names and provide the small handful of methods the PCSX2
//! codebase uses (`data`, `size`, indexing, `fill`, `swap`, `resize`, ...).
//!
//! ## Caveat: custom alignment
//!
//! The C++ versions accepted an `ALIGNMENT` template parameter so that
//! callers could request over-aligned buffers (typically for SIMD). Stable
//! Rust's [`Vec<T>`] only guarantees `align_of::<T>()` alignment; it does not
//! expose a portable way to request a larger alignment without nightly
//! `allocator_api` or an external crate such as `aligned_vec`. The aliases
//! below therefore do **not** preserve the original alignment guarantee.
//! Code paths that relied on SIMD-friendly alignment will need to be updated
//! separately (e.g. by switching to an aligned allocation primitive).
//!
//! ## FFI
//!
//! No `extern "C"` exports are provided. The original C++ types had no
//! stable ABI — their only observable surface was a raw `T*` plus a
//! `size_t`, which is exactly what [`Vec::as_ptr`] / [`Vec::as_mut_ptr`]
//! already give us for `Vec<T>` (and what `Box<[T; SIZE]>::as_ptr` gives
//! us for the fixed case). Rust code that needs to hand a buffer to C
//! can do so inline with `as_mut_ptr()` + the length, without coupling
//! the two languages' ABIs through this module.

use std::mem;
use std::ops::{Deref, DerefMut};

/// Resizable heap-allocated array, equivalent to `std::vector<T>`.
///
/// This is a type alias for [`Vec<T>]. See the [module-level docs](self) for
/// the alignment caveat.
pub type DynamicHeapArray<T> = Vec<T>;

/// Fixed-size heap-allocated array, equivalent to `std::array<T, SIZE>` but
/// with the buffer living on the heap.
///
/// Wraps [`Box<[T; SIZE]>`]. Note that, unlike the C++ version, this struct
/// always default-constructs every element — `Box<[T; SIZE]>` is a boxed
/// array, so dropping it runs `T`'s destructor for every element.
#[derive(Debug)]
pub struct FixedHeapArray<T, const SIZE: usize> {
    data: Box<[T; SIZE]>,
}

impl<T, const SIZE: usize> FixedHeapArray<T, SIZE> {
    /// Construct a new fixed heap array with all elements default-initialised.
    pub fn new() -> Self
    where
        T: Default,
    {
        // `Box::new` for an array requires `T: Default` on stable Rust. The
        // C++ version used raw `malloc` and left elements uninitialised; we
        // deliberately diverge here because reading uninitialised memory in
        // safe Rust is unsound. Callers that need an uninitialised buffer
        // should use `Vec::with_capacity` or `MaybeUninit` instead.
        Self {
            data: Box::new(std::array::from_fn(|_| T::default())),
        }
    }

    /// Number of elements (always equal to `SIZE`).
    #[inline]
    pub fn size(&self) -> usize {
        SIZE
    }

    /// Allocated capacity (always equal to `SIZE` for a fixed array).
    #[inline]
    pub fn capacity(&self) -> usize {
        SIZE
    }

    /// A fixed array is never empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Pointer to the first element, as a raw `*const T`. Suitable for FFI.
    #[inline]
    pub fn as_ptr(&self) -> *const T {
        self.data.as_ptr()
    }

    /// Mutable pointer to the first element, as a raw `*mut T`. Suitable for FFI.
    #[inline]
    pub fn as_mut_ptr(&mut self) -> *mut T {
        self.data.as_mut_ptr()
    }

    /// Borrow the underlying boxed array.
    #[inline]
    pub fn as_array(&self) -> &[T; SIZE] {
        &self.data
    }

    /// Mutably borrow the underlying boxed array.
    #[inline]
    pub fn as_mut_array(&mut self) -> &mut [T; SIZE] {
        &mut self.data
    }

    /// Borrow the buffer as a slice.
    #[inline]
    pub fn as_slice(&self) -> &[T] {
        self.data.as_slice()
    }

    /// Mutably borrow the buffer as a slice.
    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        self.data.as_mut_slice()
    }

    /// Fill every element with `value`.
    pub fn fill(&mut self, value: T)
    where
        T: Clone,
    {
        for slot in self.data.iter_mut() {
            *slot = value.clone();
        }
    }

    /// Swap the underlying buffers of two `FixedHeapArray`s.
    pub fn swap(&mut self, other: &mut Self) {
        mem::swap(&mut self.data, &mut other.data);
    }
}

impl<T, const SIZE: usize> Default for FixedHeapArray<T, SIZE>
where
    T: Default,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<T, const SIZE: usize> Deref for FixedHeapArray<T, SIZE> {
    type Target = [T; SIZE];

    #[inline]
    fn deref(&self) -> &[T; SIZE] {
        &self.data
    }
}

impl<T, const SIZE: usize> DerefMut for FixedHeapArray<T, SIZE> {
    #[inline]
    fn deref_mut(&mut self) -> &mut [T; SIZE] {
        &mut self.data
    }
}

impl<T, const SIZE: usize> Clone for FixedHeapArray<T, SIZE>
where
    T: Clone,
{
    fn clone(&self) -> Self {
        // Allocate a fresh boxed array and copy element-wise. We could use
        // `Box::clone` after `Box::new([self.data.clone()])`, but writing it
        // explicitly keeps the dependency surface tiny.
        let mut out: Box<[T; SIZE]> = Box::new(std::array::from_fn(|_| unreachable!()));
        for (dst, src) in out.iter_mut().zip(self.data.iter()) {
            *dst = src.clone();
        }
        Self { data: out }
    }
}

impl<T, const SIZE: usize> PartialEq for FixedHeapArray<T, SIZE>
where
    T: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        self.data[..] == other.data[..]
    }
}

impl<T, const SIZE: usize> Eq for FixedHeapArray<T, SIZE> where T: Eq {}

impl<T, const SIZE: usize> std::hash::Hash for FixedHeapArray<T, SIZE>
where
    T: std::hash::Hash,
{
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.data[..].hash(state);
    }
}

impl<T, const SIZE: usize> From<[T; SIZE]> for FixedHeapArray<T, SIZE> {
    fn from(arr: [T; SIZE]) -> Self {
        Self { data: Box::new(arr) }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_basic_size_and_indexing() {
        let mut a: FixedHeapArray<u32, 4> = FixedHeapArray::new();
        assert_eq!(a.size(), 4);
        assert_eq!(a.capacity(), 4);
        assert!(!a.is_empty());

        a[0] = 10;
        a[1] = 20;
        a[2] = 30;
        a[3] = 40;
        assert_eq!(a[0], 10);
        assert_eq!(a[3], 40);

        // Slice view works through Deref.
        assert_eq!(&a[..], &[10, 20, 30, 40][..]);
    }

    #[test]
    fn fixed_fill_and_swap() {
        let mut a: FixedHeapArray<u8, 3> = FixedHeapArray::new();
        let mut b: FixedHeapArray<u8, 3> = FixedHeapArray::new();

        a.fill(7);
        b.fill(9);
        a.swap(&mut b);

        assert_eq!(a[..], [9, 9, 9]);
        assert_eq!(b[..], [7, 7, 7]);
    }

    #[test]
    fn fixed_clone_and_eq() {
        let mut a: FixedHeapArray<i32, 2> = FixedHeapArray::new();
        a[0] = -1;
        a[1] = 2;
        let b = a.clone();
        assert_eq!(a, b);

        let mut c: FixedHeapArray<i32, 2> = FixedHeapArray::new();
        c[0] = 99;
        c[1] = 99;
        assert_ne!(a, c);
    }

    #[test]
    fn fixed_from_array() {
        let a: FixedHeapArray<u16, 2> = [1u16, 2u16].into();
        assert_eq!(a[0], 1);
        assert_eq!(a[1], 2);
    }

    #[test]
    fn fixed_ffi_pointers() {
        let mut a: FixedHeapArray<u32, 2> = FixedHeapArray::new();
        a[0] = 0xAA;
        a[1] = 0xBB;

        // Verify pointer/length contract that C code would consume.
        let ptr = a.as_ptr();
        let len = a.size();
        assert!(!ptr.is_null());
        assert_eq!(len, 2);
        unsafe {
            assert_eq!(*ptr, 0xAA);
            assert_eq!(*ptr.add(1), 0xBB);
        }
    }

    #[test]
    fn dynamic_is_vec() {
        // The whole point of the alias: a DynamicHeapArray *is* a Vec.
        let mut v: DynamicHeapArray<u32> = DynamicHeapArray::with_capacity(4);
        v.push(1);
        v.push(2);
        v.push(3);
        assert_eq!(v.len(), 3);
        assert_eq!(v[0], 1);
        assert_eq!(v[2], 3);

        v.resize(5, 0);
        assert_eq!(v.len(), 5);
        assert_eq!(v[3], 0);
        assert_eq!(v[4], 0);
    }
}