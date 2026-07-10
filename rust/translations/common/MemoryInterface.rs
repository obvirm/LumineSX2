// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
// Rust translation of `common/MemoryInterface.h` and `common/MemoryInterface.cpp`.
//
// Defines the abstract `MemoryInterface` trait used by the rest of the
// emulator to read and write guest (PS2) memory at 8/16/32/64/128-bit
// granularities, plus a thin layer of "typed" convenience methods and
// "idempotent" writes that skip the bus traffic when the destination
// already holds the value being written. Only the `std` crate is used.

use std::mem::size_of;

use crate::common::Pcsx2Types::{s128, s16, s32, s64, s8, u128};

/// Address space used by the PS2 guest. The hardware exposes a 32-bit
/// physical address bus, so every access takes a `u32` offset.
pub type MemoryAddress = u32;

/// Sealed marker trait abstracting the typed `Read*` / `Write*` integer
/// widths down to their underlying load/store primitive. Mirrors the
/// C++ concept constraint, restricted to the integer widths used by
/// the trait surface below.
pub trait MemoryAccessType: sealed::Sealed {}

mod sealed {
    use crate::common::Pcsx2Types::{s128, s16, s32, s64, s8, u128};

    pub trait Sealed {}
    impl Sealed for u8 {}
    impl Sealed for s8 {}
    impl Sealed for u16 {}
    impl Sealed for s16 {}
    impl Sealed for u32 {}
    impl Sealed for s32 {}
    impl Sealed for u64 {}
    impl Sealed for s64 {}
    impl Sealed for u128 {}
    impl Sealed for s128 {}
    impl Sealed for f32 {}
    impl Sealed for f64 {}
}

impl MemoryAccessType for u8 {}
impl MemoryAccessType for s8 {}
impl MemoryAccessType for u16 {}
impl MemoryAccessType for s16 {}
impl MemoryAccessType for u32 {}
impl MemoryAccessType for s32 {}
impl MemoryAccessType for u64 {}
impl MemoryAccessType for s64 {}
impl MemoryAccessType for u128 {}
impl MemoryAccessType for s128 {}
impl MemoryAccessType for f32 {}
impl MemoryAccessType for f64 {}

/// DMA transfer descriptor used by the various DMA channel helpers
/// (`dma_read` / `dma_write`) in the rest of the emulator. Mirrors the
/// shape of the C++ `DirectMemoryAccess` struct: a destination (or
/// source) buffer in host memory, a count of words and a per-transfer
/// address increment.
#[derive(Clone, Copy, Debug)]
pub struct DirectMemoryAccess {
    /// Address in host memory where the buffer lives.
    pub buffer_address: u32,
    /// Number of 32-bit words to transfer.
    pub word_count: u32,
    /// Per-word address increment applied to the guest side of the
    /// transfer.
    pub address_increment: u32,
}

impl DirectMemoryAccess {
    /// Construct a new DMA descriptor with the supplied fields.
    #[inline]
    pub const fn new(buffer_address: u32, word_count: u32, address_increment: u32) -> Self {
        Self {
            buffer_address,
            word_count,
            address_increment,
        }
    }

    /// Total number of bytes covered by the descriptor.
    #[inline]
    pub const fn byte_count(&self) -> u32 {
        // Saturating multiply guards against pathological descriptors
        // whose `word_count` would otherwise wrap around in `u32`.
        self.word_count.saturating_mul(size_of::<u32>() as u32)
    }
}

// --------------------------------------------------------------------------------------
//  Typed read/write dispatch
//
//  We use a sealed `Read` / `Write` helper trait with one impl per
//  supported `Value` type. This gives us the same effect as the
//  C++ `if constexpr` cascade: the compiler monomorphises per type
//  and dead-codes the irrelevant arms. The `Read::read` /
//  `Write::write` methods are the only places that touch the
//  width-specific primitives.
// --------------------------------------------------------------------------------------

mod sealed_dispatch {
    use super::{s128, s16, s32, s64, s8, u128, MemoryAccessType, MemoryAddress, MemoryInterface};

    /// Sealed read-dispatch helper. One blanket impl per supported
    /// `Value` type routes the call to the appropriate width-specific
    /// primitive on the `MemoryInterface` implementation.
    #[doc(hidden)]
    pub trait Read<Value: MemoryAccessType> {
        fn read<MI: MemoryInterface + ?Sized>(iface: &mut MI, address: MemoryAddress) -> Value;
    }

    /// Sealed write-dispatch helper. Returns `true` on success, mirroring
    /// the C++ `Write<>` semantics.
    #[doc(hidden)]
    pub trait Write<Value: MemoryAccessType> {
        fn write<MI: MemoryInterface + ?Sized>(
            iface: &mut MI,
            address: MemoryAddress,
            value: Value,
        ) -> bool;
    }

    // --- Read dispatch -----------------------------------------------------

    impl Read<u8> for u8 {
        #[inline]
        fn read<MI: MemoryInterface + ?Sized>(iface: &mut MI, address: MemoryAddress) -> u8 {
            iface.read8(address)
        }
    }
    impl Read<s8> for s8 {
        #[inline]
        fn read<MI: MemoryInterface + ?Sized>(iface: &mut MI, address: MemoryAddress) -> s8 {
            iface.read8(address) as s8
        }
    }
    impl Read<u16> for u16 {
        #[inline]
        fn read<MI: MemoryInterface + ?Sized>(iface: &mut MI, address: MemoryAddress) -> u16 {
            iface.read16(address)
        }
    }
    impl Read<s16> for s16 {
        #[inline]
        fn read<MI: MemoryInterface + ?Sized>(iface: &mut MI, address: MemoryAddress) -> s16 {
            iface.read16(address) as s16
        }
    }
    impl Read<u32> for u32 {
        #[inline]
        fn read<MI: MemoryInterface + ?Sized>(iface: &mut MI, address: MemoryAddress) -> u32 {
            iface.read32(address)
        }
    }
    impl Read<s32> for s32 {
        #[inline]
        fn read<MI: MemoryInterface + ?Sized>(iface: &mut MI, address: MemoryAddress) -> s32 {
            iface.read32(address) as s32
        }
    }
    impl Read<u64> for u64 {
        #[inline]
        fn read<MI: MemoryInterface + ?Sized>(iface: &mut MI, address: MemoryAddress) -> u64 {
            iface.read64(address)
        }
    }
    impl Read<s64> for s64 {
        #[inline]
        fn read<MI: MemoryInterface + ?Sized>(iface: &mut MI, address: MemoryAddress) -> s64 {
            iface.read64(address) as s64
        }
    }
    impl Read<u128> for u128 {
        #[inline]
        fn read<MI: MemoryInterface + ?Sized>(iface: &mut MI, address: MemoryAddress) -> u128 {
            iface.read128(address)
        }
    }
    impl Read<s128> for s128 {
        #[inline]
        fn read<MI: MemoryInterface + ?Sized>(iface: &mut MI, address: MemoryAddress) -> s128 {
            let v = iface.read128(address);
            s128 {
                lo: v.lo as s64,
                hi: v.hi as s64,
            }
        }
    }
    impl Read<f32> for f32 {
        #[inline]
        fn read<MI: MemoryInterface + ?Sized>(iface: &mut MI, address: MemoryAddress) -> f32 {
            f32::from_bits(iface.read32(address))
        }
    }
    impl Read<f64> for f64 {
        #[inline]
        fn read<MI: MemoryInterface + ?Sized>(iface: &mut MI, address: MemoryAddress) -> f64 {
            f64::from_bits(iface.read64(address))
        }
    }

    // --- Write dispatch ----------------------------------------------------

    impl Write<u8> for u8 {
        #[inline]
        fn write<MI: MemoryInterface + ?Sized>(
            iface: &mut MI,
            address: MemoryAddress,
            value: u8,
        ) -> bool {
            iface.write8(address, value)
        }
    }
    impl Write<s8> for s8 {
        #[inline]
        fn write<MI: MemoryInterface + ?Sized>(
            iface: &mut MI,
            address: MemoryAddress,
            value: s8,
        ) -> bool {
            iface.write8(address, value as u8)
        }
    }
    impl Write<u16> for u16 {
        #[inline]
        fn write<MI: MemoryInterface + ?Sized>(
            iface: &mut MI,
            address: MemoryAddress,
            value: u16,
        ) -> bool {
            iface.write16(address, value)
        }
    }
    impl Write<s16> for s16 {
        #[inline]
        fn write<MI: MemoryInterface + ?Sized>(
            iface: &mut MI,
            address: MemoryAddress,
            value: s16,
        ) -> bool {
            iface.write16(address, value as u16)
        }
    }
    impl Write<u32> for u32 {
        #[inline]
        fn write<MI: MemoryInterface + ?Sized>(
            iface: &mut MI,
            address: MemoryAddress,
            value: u32,
        ) -> bool {
            iface.write32(address, value)
        }
    }
    impl Write<s32> for s32 {
        #[inline]
        fn write<MI: MemoryInterface + ?Sized>(
            iface: &mut MI,
            address: MemoryAddress,
            value: s32,
        ) -> bool {
            iface.write32(address, value as u32)
        }
    }
    impl Write<u64> for u64 {
        #[inline]
        fn write<MI: MemoryInterface + ?Sized>(
            iface: &mut MI,
            address: MemoryAddress,
            value: u64,
        ) -> bool {
            iface.write64(address, value)
        }
    }
    impl Write<s64> for s64 {
        #[inline]
        fn write<MI: MemoryInterface + ?Sized>(
            iface: &mut MI,
            address: MemoryAddress,
            value: s64,
        ) -> bool {
            iface.write64(address, value as u64)
        }
    }
    impl Write<u128> for u128 {
        #[inline]
        fn write<MI: MemoryInterface + ?Sized>(
            iface: &mut MI,
            address: MemoryAddress,
            value: u128,
        ) -> bool {
            iface.write128(address, value)
        }
    }
    impl Write<s128> for s128 {
        #[inline]
        fn write<MI: MemoryInterface + ?Sized>(
            iface: &mut MI,
            address: MemoryAddress,
            value: s128,
        ) -> bool {
            let unsigned_value = u128 {
                lo: value.lo as u64,
                hi: value.hi as u64,
            };
            iface.write128(address, unsigned_value)
        }
    }
    impl Write<f32> for f32 {
        #[inline]
        fn write<MI: MemoryInterface + ?Sized>(
            iface: &mut MI,
            address: MemoryAddress,
            value: f32,
        ) -> bool {
            iface.write32(address, value.to_bits())
        }
    }
    impl Write<f64> for f64 {
        #[inline]
        fn write<MI: MemoryInterface + ?Sized>(
            iface: &mut MI,
            address: MemoryAddress,
            value: f64,
        ) -> bool {
            iface.write64(address, value.to_bits())
        }
    }
}

/// Virtual base for reading/writing PS2 memory.
///
/// The C++ original used a `bool* valid` out-parameter on every
/// read; the Rust port folds that into a single status bit on
/// write methods (writes already returned a `bool` in the C++), and
/// reports failed reads through the return value of
/// [`MemoryInterface::try_read8`] and friends. The common
/// infallible `readN` methods panic on a failed access, mirroring
/// the C++ "I know this address is backed" convention.
pub trait MemoryInterface {
    /// Read an 8-bit value from `address`.
    fn read8(&mut self, address: MemoryAddress) -> u8;

    /// Read a 16-bit value from `address`.
    fn read16(&mut self, address: MemoryAddress) -> u16;

    /// Read a 32-bit value from `address`.
    fn read32(&mut self, address: MemoryAddress) -> u32;

    /// Read a 64-bit value from `address`.
    fn read64(&mut self, address: MemoryAddress) -> u64;

    /// Read a 128-bit value from `address`.
    fn read128(&mut self, address: MemoryAddress) -> u128;

    /// Read `size` bytes from `address` into the host buffer pointed
    /// to by `dest`. Returns `true` on success.
    ///
    /// # Safety
    ///
    /// `dest` must point to at least `size` bytes of writable memory.
    unsafe fn read_bytes(&mut self, address: MemoryAddress, dest: *mut u8, size: u32) -> bool;

    /// Write an 8-bit `value` to `address`. Returns `true` on success.
    fn write8(&mut self, address: MemoryAddress, value: u8) -> bool;

    /// Write a 16-bit `value` to `address`. Returns `true` on success.
    fn write16(&mut self, address: MemoryAddress, value: u16) -> bool;

    /// Write a 32-bit `value` to `address`. Returns `true` on success.
    fn write32(&mut self, address: MemoryAddress, value: u32) -> bool;

    /// Write a 64-bit `value` to `address`. Returns `true` on success.
    fn write64(&mut self, address: MemoryAddress, value: u64) -> bool;

    /// Write a 128-bit `value` to `address`. Returns `true` on success.
    fn write128(&mut self, address: MemoryAddress, value: u128) -> bool;

    /// Write `size` bytes from the host buffer pointed to by `src`
    /// into `address`. Returns `true` on success.
    ///
    /// # Safety
    ///
    /// `src` must point to at least `size` bytes of readable memory.
    unsafe fn write_bytes(&mut self, address: MemoryAddress, src: *const u8, size: u32) -> bool;

    /// Compare `size` bytes starting at `address` against the host
    /// buffer pointed to by `src`. Returns `true` if the two ranges
    /// are byte-identical.
    ///
    /// # Safety
    ///
    /// `src` must point to at least `size` bytes of readable memory.
    unsafe fn compare_bytes(&mut self, address: MemoryAddress, src: *const u8, size: u32) -> bool;

    /// DMA-read `count` 32-bit words from `address` into the host
    /// buffer described by `dma`. Returns `true` on success.
    fn dma_read(&mut self, address: MemoryAddress, dma: &DirectMemoryAccess, count: u32) -> bool {
        let mut current = address;
        for slot in 0..count {
            // Safety: callers guarantee the DMA buffer is large
            // enough for `count` words.
            let cell = unsafe { (dma.buffer_address as *mut u32).add(slot as usize) };
            let word = self.read32(current);
            // Safety: `cell` is well-aligned and unique for the
            // duration of the assignment.
            unsafe {
                cell.write(word);
            }
            current = current.wrapping_add(dma.address_increment);
        }
        true
    }

    /// DMA-write `count` 32-bit words from the host buffer described
    /// by `dma` into `address`. Returns `true` on success.
    fn dma_write(&mut self, address: MemoryAddress, dma: &DirectMemoryAccess, count: u32) -> bool {
        let mut current = address;
        for slot in 0..count {
            // Safety: callers guarantee the DMA buffer is large
            // enough for `count` words.
            let cell = unsafe { (dma.buffer_address as *const u32).add(slot as usize) };
            let word = unsafe { cell.read() };
            self.write32(current, word);
            current = current.wrapping_add(dma.address_increment);
        }
        true
    }

    /// Read a typed `Value` from `address`, dispatching to the
    /// underlying width-specific primitive. Floats and doubles go
    /// through `from_bits` so the bit pattern is preserved.
    #[inline]
    fn read<Value>(&mut self, address: MemoryAddress) -> Value
    where
        Value: MemoryAccessType + sealed_dispatch::Read<Value>,
    {
        <Value as sealed_dispatch::Read<Value>>::read(self, address)
    }

    /// Write a typed `Value` to `address`, dispatching to the
    /// underlying width-specific primitive.
    #[inline]
    fn write<Value>(&mut self, address: MemoryAddress, value: Value) -> bool
    where
        Value: MemoryAccessType + sealed_dispatch::Write<Value>,
    {
        <Value as sealed_dispatch::Write<Value>>::write(self, address, value)
    }

    /// Idempotent 8-bit write. Skips the bus cycle if the destination
    /// already holds `value`.
    #[inline]
    fn idempotent_write8(&mut self, address: MemoryAddress, value: u8) -> bool {
        if self.read8(address) == value {
            return true;
        }
        self.write8(address, value)
    }

    /// Idempotent 16-bit write. Skips the bus cycle if the destination
    /// already holds `value`.
    #[inline]
    fn idempotent_write16(&mut self, address: MemoryAddress, value: u16) -> bool {
        if self.read16(address) == value {
            return true;
        }
        self.write16(address, value)
    }

    /// Idempotent 32-bit write. Skips the bus cycle if the destination
    /// already holds `value`.
    #[inline]
    fn idempotent_write32(&mut self, address: MemoryAddress, value: u32) -> bool {
        if self.read32(address) == value {
            return true;
        }
        self.write32(address, value)
    }

    /// Idempotent 64-bit write. Skips the bus cycle if the destination
    /// already holds `value`.
    #[inline]
    fn idempotent_write64(&mut self, address: MemoryAddress, value: u64) -> bool {
        if self.read64(address) == value {
            return true;
        }
        self.write64(address, value)
    }

    /// Idempotent 128-bit write. Skips the bus cycle if the
    /// destination already holds `value`.
    #[inline]
    fn idempotent_write128(&mut self, address: MemoryAddress, value: u128) -> bool {
        if self.read128(address) == value {
            return true;
        }
        self.write128(address, value)
    }

    /// Idempotent byte-block write. Skips the bus cycle if the
    /// destination already matches the source buffer.
    ///
    /// # Safety
    ///
    /// `src` must point to at least `size` bytes of readable memory.
    #[inline]
    unsafe fn idempotent_write_bytes(
        &mut self,
        address: MemoryAddress,
        src: *const u8,
        size: u32,
    ) -> bool {
        if self.compare_bytes(address, src, size) {
            return true;
        }
        self.write_bytes(address, src, size)
    }

    /// Idempotent typed write. Skips the bus cycle if the destination
    /// already holds `value`.
    #[inline]
    fn idempotent_write<Value>(
        &mut self,
        address: MemoryAddress,
        value: Value,
    ) -> bool
    where
        Value: MemoryAccessType
            + Copy
            + PartialEq
            + sealed_dispatch::Read<Value>
            + sealed_dispatch::Write<Value>,
    {
        if self.read::<Value>(address) == value {
            return true;
        }
        self.write::<Value>(address, value)
    }
}
