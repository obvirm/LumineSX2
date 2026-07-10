//! HeapArray - a heap-allocated array with explicit size management.
//!
//! This is the Rust translation of PCSX2's C++ `HeapArray` family
//! (`FixedHeapArray` and `DynamicHeapArray`). It mirrors the
//! `std::vector`-like ergonomics of the original API, with an explicit
//! `set_size` method that parallels the C++ `resize`.

use std::slice;

/// A `HeapArray<T>` is similar to `Vec<T>`, but stores its backing storage
/// in a `Box<[T]>` and tracks the logical length separately. This layout
/// lets `set_size` grow or shrink the array in a way that parallels the
/// C++ `DynamicHeapArray::resize` semantics, where the heap pre-allocation
/// was explicit.
///
/// The C++ version constrained `T` to be trivially copyable and standard
/// layout; idiomatic Rust expresses these requirements via the [`Clone`]
/// bound on the methods that need to copy elements.
pub struct HeapArray<T> {
    data: Box<[T]>,
    len: usize,
}

impl<T> HeapArray<T> {
    /// Creates a new, empty `HeapArray` with a zero-length backing slice.
    pub fn new() -> Self {
        Self {
            data: Box::from([]),
            len: 0,
        }
    }

    /// Creates a new `HeapArray` with `sz` elements, each initialised to
    /// `T::default()`. Mirrors `DynamicHeapArray(size_t size)`.
    pub fn with_size(sz: usize) -> Self
    where
        T: Default,
    {
        let data: Box<[T]> = std::iter::repeat_with(T::default)
            .take(sz)
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self { data, len: sz }
    }

    /// Resizes the array to `new_size`. Newly added elements are filled with
    /// `new_value`; any elements past the new length are dropped. Mirrors
    /// `DynamicHeapArray::resize`.
    pub fn set_size(&mut self, new_size: usize, new_value: T)
    where
        T: Clone,
    {
        if new_size == self.len {
            return;
        }

        let mut v: Vec<T> = self.as_slice().to_vec();
        v.resize(new_size, new_value);
        self.data = v.into_boxed_slice();
        self.len = new_size;
    }

    /// Returns an iterator over shared references to the live elements.
    pub fn iter(&self) -> slice::Iter<'_, T> {
        self.as_slice().iter()
    }

    /// Returns an iterator over mutable references to the live elements.
    pub fn iter_mut(&mut self) -> slice::IterMut<'_, T> {
        self.as_mut_slice().iter_mut()
    }

    /// Returns the number of elements currently considered "live" in the array.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns a shared slice view of the live elements.
    pub fn as_slice(&self) -> &[T] {
        &self.data[..self.len]
    }

    /// Returns a mutable slice view of the live elements.
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.data[..self.len]
    }
}

impl<T> Default for HeapArray<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Clone> Clone for HeapArray<T> {
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
            len: self.len,
        }
    }
}
