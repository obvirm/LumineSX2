// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! IOP (R3000) physical memory layout and bus accessors.
//!
//! This module is the Rust port of PCSX2's `pcsx2/IopMem.{h,cpp}` pair. It exposes the
//! 2 MiB IOP main RAM (`iopMem`) and the helper accessors the rest of the IOP code
//! uses to read and write through the bus (`iopMemRead8/16/32`, `iopMemWrite8/16/32`,
//! plus the safe block helpers `iopMemSafeReadBytes`/`iopMemSafeWriteBytes`/
//! `iopMemSafeCmpBytes` and the convenience string reader `iopMemReadString`).
//!
//! The original C++ module is mostly a thin wrapper around two lookup tables
//! (`psxMemWLUT` / `psxMemRLUT`) and a `IopVM_MemoryAllocMess` aggregate that
//! bundles `Main`, `P`, and `Sif` buffers. Those buffers live in the big
//! `SysMemory::GetIOPMem()` allocation produced by PCSX2's memmap. In this Rust
//! port the user-visible buffer is just [`iopMem`]: a single 2 MiB array, which is
//! all the public API needs. The hardware dispatch paths that the C++ code routes
//! to (`iopHwRead*_PageN`, `DEV9read*`, `SPU2read`, `psxHw4Read8`, ...) are out of
//! scope for this module — we collapse the address-space switch into a single
//! bounds check against [`IOP_MEM_SIZE`], matching the only region this module
//! actually owns. Out-of-range addresses behave as the C++ version does in
//! the "unmapped" case: reads return 0 and writes are silently dropped.

use std::ptr;

/// Total size of the IOP main RAM exposed to the bus, in bytes (2 MiB).
pub const IOP_MEM_SIZE: usize = 0x0020_0000;

/// Page shift used when building the WLUT/RLUT lookups in the C++ version.
/// Kept here for callers that want to derive the page size (`1 << IOP_PAGESHIFT`).
pub const IOP_PAGESHIFT: u32 = 12;

/// Page size in bytes, derived from [`IOP_PAGESHIFT`].
pub const IOP_PAGESIZE: usize = 1usize << IOP_PAGESHIFT;

/// Mask equivalent to `addr % IOP_PAGESIZE`, useful for page-relative offsets.
pub const IOP_PAGEMASK: u32 = (IOP_PAGESIZE as u32) - 1;

/// 2 MiB of IOP physical memory. Mirrors `iopMem->Main` from the C++ side and is
/// the only buffer the read/write helpers below touch directly.
pub static mut iopMem: [u8; IOP_MEM_SIZE] = [0u8; IOP_MEM_SIZE];

// ---------------------------------------------------------------------------
// Init / reset / shutdown
// ---------------------------------------------------------------------------

/// Reset the IOP memory state.
///
/// Mirrors `iopMemAlloc` + `iopMemReset` from the C++ side. The C++ version
/// allocates the WLUT/RLUT lookup tables through `_aligned_malloc` and rebuilds
/// the page mappings; this port does not need a separate lookup table because
/// [`iopMemRead8`] / [`iopMemWrite8`] check the address against
/// [`IOP_MEM_SIZE`] directly. We keep the same three-phase lifecycle
/// (`Init` -> `Reset` -> `Shutdown`) so callers can swap implementations
/// without changing their bookkeeping.
pub fn iopMemInit() {
    // The C++ version's lookup tables would be built here. There is nothing to
    // allocate on the Rust side: `iopMem` is a `static` array and is always
    // live. We still call `iopMemReset` so a freshly-initialised buffer starts
    // in a known state.
    iopMemReset();
}

/// Zero the IOP main RAM. This is the Rust equivalent of the
/// `std::memset(iopMem, 0, sizeof(*iopMem))` call in the C++ `iopMemReset`.
pub fn iopMemReset() {
    // Safety: `iopMem` is a `static mut` array; the only writer is this module
    // and the function has no other outstanding references.
    unsafe {
        ptr::write_bytes(iopMem.as_mut_ptr(), 0u8, IOP_MEM_SIZE);
    }
}

/// Release any resources owned by the IOP memory subsystem.
///
/// The Rust port has nothing to free (`iopMem` is a `static` array and the C++
/// `_aligned_malloc`'d lookup tables are gone), so this is a no-op. It exists
/// so callers can pair it with [`iopMemInit`] symmetrically.
pub fn iopMemShutdown() {
    // No-op: `iopMem` is statically allocated.
}

// ---------------------------------------------------------------------------
// Bus reads
// ---------------------------------------------------------------------------

/// Read a single byte from the IOP bus.
///
/// In the C++ version this routes through `psxMemRLUT` and dispatches to the
/// hardware handlers for `0x1F80*`, `0x1F40*`, `0x1F90*`, and `0x1000*`. This
/// module only owns the `Main` window, so any address that falls inside
/// [`IOP_MEM_SIZE`] is read directly; everything else returns 0, matching the
/// "unmapped" branch of the C++ code.
pub fn iopMemRead8(addr: u32) -> u8 {
    let phys = (addr & 0x1FFF_FFFF) as usize;
    if phys < IOP_MEM_SIZE {
        // Safety: bounds-checked above; the only writer to `iopMem` is this
        // module's write helpers, both of which go through the same check.
        unsafe { *iopMem.as_ptr().add(phys) }
    } else {
        0
    }
}

/// Read a little-endian half-word from the IOP bus.
pub fn iopMemRead16(addr: u32) -> u16 {
    let phys = (addr & 0x1FFF_FFFF) as usize;
    if phys + 1 < IOP_MEM_SIZE {
        // Safety: bounds-checked above; the buffer is 2 MiB and `phys + 1` is
        // known to be in-range.
        let bytes = unsafe { *(iopMem.as_ptr().add(phys) as *const [u8; 2]) };
        u16::from_le_bytes(bytes)
    } else {
        0
    }
}

/// Read a little-endian word from the IOP bus.
pub fn iopMemRead32(addr: u32) -> u32 {
    let phys = (addr & 0x1FFF_FFFF) as usize;
    if phys + 3 < IOP_MEM_SIZE {
        // Safety: bounds-checked above; same reasoning as the 16-bit version.
        let bytes = unsafe { *(iopMem.as_ptr().add(phys) as *const [u8; 4]) };
        u32::from_le_bytes(bytes)
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// Bus writes
// ---------------------------------------------------------------------------

/// Write a single byte to the IOP bus.
///
/// Mirrors `iopMemWrite8`: addresses inside the owned 2 MiB window are stored
/// directly; everything else is dropped, which is the same behaviour the C++
/// code falls into once the WLUT/RLUT entries are exhausted.
pub fn iopMemWrite8(addr: u32, value: u8) {
    let phys = (addr & 0x1FFF_FFFF) as usize;
    if phys < IOP_MEM_SIZE {
        // Safety: bounds-checked above.
        unsafe {
            *iopMem.as_mut_ptr().add(phys) = value;
        }
    }
}

/// Write a little-endian half-word to the IOP bus.
pub fn iopMemWrite16(addr: u32, value: u16) {
    let phys = (addr & 0x1FFF_FFFF) as usize;
    if phys + 1 < IOP_MEM_SIZE {
        // Safety: bounds-checked above; the buffer is large enough to hold the
        // 2-byte write at `phys`.
        unsafe {
            let dst = iopMem.as_mut_ptr().add(phys) as *mut [u8; 2];
            *dst = value.to_le_bytes();
        }
    }
}

/// Write a little-endian word to the IOP bus.
pub fn iopMemWrite32(addr: u32, value: u32) {
    let phys = (addr & 0x1FFF_FFFF) as usize;
    if phys + 3 < IOP_MEM_SIZE {
        // Safety: bounds-checked above; the buffer is large enough to hold the
        // 4-byte write at `phys`.
        unsafe {
            let dst = iopMem.as_mut_ptr().add(phys) as *mut [u8; 4];
            *dst = value.to_le_bytes();
        }
    }
}

// ---------------------------------------------------------------------------
// Safe block helpers
// ---------------------------------------------------------------------------

/// Compare `size` bytes at IOP address `mem` against `src`.
///
/// Returns the first non-zero `memcmp` result, or 0 if the whole region
/// matched. Returns `-1` if the address falls outside the owned IOP window
/// (matching the C++ version, which returns `-1` when `iopVirtMemW` yields
/// `NULL`).
///
/// # Safety
///
/// `src` must point to at least `size` readable bytes.
pub unsafe fn iopMemSafeCmpBytes(mem: u32, src: *const u8, size: u32) -> i32 {
    if size == 0 {
        return 0;
    }
    // `src` is `*const u8`, so it can never be null in safe terms; the C++ code
    // has the same contract.
    let mut sptr = src;
    let sptr_end = src.add(size as usize);
    let mut cur = mem;
    while sptr < sptr_end {
        let phys = (cur & 0x1FFF_FFFF) as usize;
        if phys >= IOP_MEM_SIZE {
            return -1;
        }
        let remaining_in_page =
            core::cmp::min(IOP_PAGESIZE - (phys & IOP_PAGEMASK as usize), sptr_end as usize - sptr as usize);
        let dst = iopMem.as_ptr().add(phys);
        let res = libc_memcmp(sptr, dst, remaining_in_page);
        if res != 0 {
            return res;
        }
        sptr = sptr.add(remaining_in_page);
        cur = cur.wrapping_add(remaining_in_page as u32);
    }
    0
}

/// Copy `size` bytes from IOP address `mem` into `dst`.
///
/// Returns `true` on success, `false` if any byte of the transfer falls outside
/// the owned IOP window (mirrors the C++ `iopMemSafeReadBytes` contract).
///
/// # Safety
///
/// `dst` must point to at least `size` writable bytes.
pub unsafe fn iopMemSafeReadBytes(mem: u32, dst: *mut u8, size: u32) -> bool {
    if size == 0 {
        return true;
    }
    let mut dptr = dst;
    let dptr_end = dst.add(size as usize);
    let mut cur = mem;
    while dptr < dptr_end {
        let phys = (cur & 0x1FFF_FFFF) as usize;
        if phys >= IOP_MEM_SIZE {
            return false;
        }
        let remaining_in_page =
            core::cmp::min(IOP_PAGESIZE - (phys & IOP_PAGEMASK as usize), dptr_end as usize - dptr as usize);
        let src = iopMem.as_ptr().add(phys);
        ptr::copy_nonoverlapping(src, dptr, remaining_in_page);
        dptr = dptr.add(remaining_in_page);
        cur = cur.wrapping_add(remaining_in_page as u32);
    }
    true
}

/// Copy `size` bytes from `src` into IOP address `mem`.
///
/// Returns `true` on success, `false` if any byte of the transfer falls outside
/// the owned IOP window (mirrors the C++ `iopMemSafeWriteBytes` contract).
///
/// # Safety
///
/// `src` must point to at least `size` readable bytes.
pub unsafe fn iopMemSafeWriteBytes(mem: u32, src: *const u8, size: u32) -> bool {
    if size == 0 {
        return true;
    }
    let mut sptr = src;
    let sptr_end = src.add(size as usize);
    let mut cur = mem;
    while sptr < sptr_end {
        let phys = (cur & 0x1FFF_FFFF) as usize;
        if phys >= IOP_MEM_SIZE {
            return false;
        }
        let remaining_in_page =
            core::cmp::min(IOP_PAGESIZE - (phys & IOP_PAGEMASK as usize), sptr_end as usize - sptr as usize);
        let dst = iopMem.as_mut_ptr().add(phys);
        ptr::copy_nonoverlapping(sptr, dst, remaining_in_page);
        sptr = sptr.add(remaining_in_page);
        cur = cur.wrapping_add(remaining_in_page as u32);
    }
    true
}

/// Read a NUL-terminated ASCII string from IOP memory.
///
/// `maxlen` defaults to 64 KiB, matching the C++ default. Stops at the first
/// NUL byte, when `maxlen` bytes have been consumed, or when the read would
/// leave the owned window — whichever comes first. Behaviour matches
/// `std::string iopMemReadString(u32 mem, int maxlen = 65536);` in the C++
/// module.
pub fn iopMemReadString(mem: u32, maxlen: i32) -> String {
    let mut ret = String::new();
    if maxlen <= 0 {
        return ret;
    }
    let mut addr = mem;
    let mut remaining = maxlen as usize;
    loop {
        let b = iopMemRead8(addr);
        if b == 0 {
            break;
        }
        ret.push(b as char);
        addr = addr.wrapping_add(1);
        remaining -= 1;
        if remaining == 0 {
            break;
        }
    }
    ret
}

// ---------------------------------------------------------------------------
// IOP hardware page-specific accessors (IopMemory namespace)
//
// These are thin stubs that mirror the `IopMemory::iopHwRead*_PageN` and
// `IopMemory::iopHwWrite*_PageN` declarations in `pcsx2/IopMem.h` (lines
// 103-121). The real per-page dispatch lives in `IopHw.cpp` and is owned by
// the `IopHw` module in `HwEtcMain.rs`. We re-export page-relative accessors
// here that the bus read/write paths above conceptually call into; the body
// of each is a single delegation to the corresponding `iopHw*_generic`
// dispatcher so the type signatures stay in lock-step with the C++ header.
//
// Page numbers reflect the `t = mem & 0x1FFF_FFFF >> 16` switch in
// `IopMem.cpp`:
//   * Page 1 - 0x1F801000 .. 0x1F801FFF  (offset 0x1000)
//   * Page 3 - 0x1F803000 .. 0x1F803FFF  (offset 0x3000)
//   * Page 8 - 0x1F808000 .. 0x1F808FFF  (offset 0x8000)
// ---------------------------------------------------------------------------

/// IOP hardware page-specific accessors, mirroring `namespace IopMemory` in
/// `pcsx2/IopMem.h`. Each `*_PageN` function ultimately feeds
/// `iopHw{Read,Write}{8,16,32}_generic` from `HwEtcMain::IopHw`; the per-page
/// specialisations live in that module's dispatch tables.
pub mod IopMemory {
    use crate::pcsx2::HwEtcMain::IopHw::{
        iopHwRead16_generic, iopHwRead32_generic, iopHwRead8_generic,
        iopHwWrite16_generic, iopHwWrite32_generic, iopHwWrite8_generic,
    };

    // ----- 8-bit page-specific HW reads -----

    /// Read an 8-bit IOP HW register on Page 1 (0x1F801000-0x1F801FFF).
    pub fn iopHwRead8_Page1(addr: u32) -> u8 {
        iopHwRead8_generic(addr)
    }

    /// Read an 8-bit IOP HW register on Page 3 (0x1F803000-0x1F803FFF).
    pub fn iopHwRead8_Page3(addr: u32) -> u8 {
        iopHwRead8_generic(addr)
    }

    /// Read an 8-bit IOP HW register on Page 8 (0x1F808000-0x1F808FFF).
    pub fn iopHwRead8_Page8(addr: u32) -> u8 {
        iopHwRead8_generic(addr)
    }

    // ----- 16-bit page-specific HW reads -----

    /// Read a 16-bit IOP HW register on Page 1.
    pub fn iopHwRead16_Page1(addr: u32) -> u16 {
        iopHwRead16_generic(addr)
    }

    /// Read a 16-bit IOP HW register on Page 3.
    pub fn iopHwRead16_Page3(addr: u32) -> u16 {
        iopHwRead16_generic(addr)
    }

    /// Read a 16-bit IOP HW register on Page 8.
    pub fn iopHwRead16_Page8(addr: u32) -> u16 {
        iopHwRead16_generic(addr)
    }

    // ----- 32-bit page-specific HW reads -----

    /// Read a 32-bit IOP HW register on Page 1.
    pub fn iopHwRead32_Page1(addr: u32) -> u32 {
        iopHwRead32_generic(addr)
    }

    /// Read a 32-bit IOP HW register on Page 3.
    pub fn iopHwRead32_Page3(addr: u32) -> u32 {
        iopHwRead32_generic(addr)
    }

    /// Read a 32-bit IOP HW register on Page 8.
    pub fn iopHwRead32_Page8(addr: u32) -> u32 {
        iopHwRead32_generic(addr)
    }

    // ----- 8-bit page-specific HW writes -----

    /// Write an 8-bit IOP HW register on Page 1.
    pub fn iopHwWrite8_Page1(addr: u32, value: u8) {
        iopHwWrite8_generic(addr, value);
    }

    /// Write an 8-bit IOP HW register on Page 3.
    pub fn iopHwWrite8_Page3(addr: u32, value: u8) {
        iopHwWrite8_generic(addr, value);
    }

    /// Write an 8-bit IOP HW register on Page 8.
    pub fn iopHwWrite8_Page8(addr: u32, value: u8) {
        iopHwWrite8_generic(addr, value);
    }

    // ----- 16-bit page-specific HW writes -----

    /// Write a 16-bit IOP HW register on Page 1.
    pub fn iopHwWrite16_Page1(addr: u32, value: u16) {
        iopHwWrite16_generic(addr, value);
    }

    /// Write a 16-bit IOP HW register on Page 3.
    pub fn iopHwWrite16_Page3(addr: u32, value: u16) {
        iopHwWrite16_generic(addr, value);
    }

    /// Write a 16-bit IOP HW register on Page 8.
    pub fn iopHwWrite16_Page8(addr: u32, value: u16) {
        iopHwWrite16_generic(addr, value);
    }

    // ----- 32-bit page-specific HW writes -----

    /// Write a 32-bit IOP HW register on Page 1.
    pub fn iopHwWrite32_Page1(addr: u32, value: u32) {
        iopHwWrite32_generic(addr, value);
    }

    /// Write a 32-bit IOP HW register on Page 3.
    pub fn iopHwWrite32_Page3(addr: u32, value: u32) {
        iopHwWrite32_generic(addr, value);
    }

    /// Write a 32-bit IOP HW register on Page 8.
    pub fn iopHwWrite32_Page8(addr: u32, value: u32) {
        iopHwWrite32_generic(addr, value);
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Local `memcmp` that does not pull in `<libc>`. Returns the first non-zero
/// byte difference, or 0 if both slices match. Mirrors the behaviour of
/// `std::memcmp` for the well-defined equal-prefix case the safe byte helpers
/// care about.
///
/// # Safety
///
/// `a` and `b` must each point to at least `n` readable bytes.
unsafe fn libc_memcmp(a: *const u8, b: *const u8, n: usize) -> i32 {
    let mut i = 0;
    while i < n {
        let av = *a.add(i);
        let bv = *b.add(i);
        if av != bv {
            return (av as i32) - (bv as i32);
        }
        i += 1;
    }
    0
}

// ---------------------------------------------------------------------------
// IOPMemoryInterface
// ---------------------------------------------------------------------------
//
// Rust translation of the `IOPMemoryInterface` class declared at the bottom of
// `pcsx2/IopMem.h` (lines 124-142). The C++ class is a thin wrapper that
// implements the abstract `MemoryInterface` declared in
// `common/MemoryInterface.h`, re-routing every access through the
// `iopMemRead*` / `iopMemWrite*` / `iopMemSafe*` helpers in this module.
//
// In the Rust port the same job is done by `IOPMemoryInterface`, a zero-sized
// marker struct whose methods forward to the bus helpers above. We do not
// implement the full `MemoryInterface` trait here because that trait returns
// `bool` for writes and uses a `Pcsx2Types::u128` pair-struct, whereas the
// C++ class returns a native `u128`. The struct is therefore plain data and
// is meant to be constructed on the stack (`IOPMemoryInterface::new()`) and
// used as a handle into the IOP bus.

/// Handle into the IOP bus accessors exposed by this module.
///
/// The C++ `IOPMemoryInterface` is a stateless singleton — every method is
/// `const` and the only state it touches is the global `iopMem` array. The
/// Rust port mirrors that with a zero-sized marker so callers can use the
/// helper in a uniform way (and so a future revision that does need state
/// has somewhere to put it without breaking call sites).
#[derive(Clone, Copy, Debug, Default)]
pub struct IOPMemoryInterface {
    _private: (),
}

impl IOPMemoryInterface {
    /// Construct a new handle. Equivalent to the C++ default constructor.
    #[inline]
    pub const fn new() -> Self {
        Self { _private: () }
    }

    /// Read a single byte from the IOP bus at `addr`.
    #[inline]
    pub fn Read8(&self, addr: u32) -> u8 {
        iopMemRead8(addr)
    }

    /// Read a little-endian half-word from the IOP bus at `addr`.
    #[inline]
    pub fn Read16(&self, addr: u32) -> u16 {
        iopMemRead16(addr)
    }

    /// Read a little-endian word from the IOP bus at `addr`.
    #[inline]
    pub fn Read32(&self, addr: u32) -> u32 {
        iopMemRead32(addr)
    }

    /// Read a little-endian double-word from the IOP bus at `addr`.
    ///
    /// Built on top of two 32-bit reads so the address masking and
    /// out-of-range behaviour stay consistent with the bus helpers.
    #[inline]
    pub fn Read64(&self, addr: u32) -> u64 {
        let lo = iopMemRead32(addr) as u64;
        let hi = iopMemRead32(addr.wrapping_add(4)) as u64;
        lo | (hi << 32)
    }

    /// Read a little-endian quad-word from the IOP bus at `addr`.
    ///
    /// Built from two 64-bit reads; the two halves are little-endian
    /// individually, and the pair is laid out with the lower-address
    /// half in `lo` (matching the C++ `u128 { u64 lo, u64 hi; }` layout).
    #[inline]
    pub fn Read128(&self, addr: u32) -> u128 {
        let lo = self.Read64(addr);
        let hi = self.Read64(addr.wrapping_add(8));
        ((hi as u128) << 64) | (lo as u128)
    }

    /// Write a single byte to the IOP bus at `addr`.
    #[inline]
    pub fn Write8(&self, addr: u32, value: u8) {
        iopMemWrite8(addr, value);
    }

    /// Write a little-endian half-word to the IOP bus at `addr`.
    #[inline]
    pub fn Write16(&self, addr: u32, value: u16) {
        iopMemWrite16(addr, value);
    }

    /// Write a little-endian word to the IOP bus at `addr`.
    #[inline]
    pub fn Write32(&self, addr: u32, value: u32) {
        iopMemWrite32(addr, value);
    }

    /// Write a little-endian double-word to the IOP bus at `addr`.
    ///
    /// Split into two 32-bit writes so the address masking behaviour of
    /// `iopMemWrite32` is reused for both halves.
    #[inline]
    pub fn Write64(&self, addr: u32, value: u64) {
        iopMemWrite32(addr, value as u32);
        iopMemWrite32(addr.wrapping_add(4), (value >> 32) as u32);
    }

    /// Write a little-endian quad-word to the IOP bus at `addr`.
    ///
    /// Split into two 64-bit writes; see [`Self::Read128`] for the byte
    /// ordering.
    #[inline]
    pub fn Write128(&self, addr: u32, value: u128) {
        self.Write64(addr, value as u64);
        self.Write64(addr.wrapping_add(8), (value >> 64) as u64);
    }

    /// Copy `dst.len()` bytes from the IOP bus at `addr` into `dst`.
    ///
    /// Returns silently if `dst` is empty (matching the C++ early-out for
    /// `size == 0`). Returns silently leaving `dst` partially filled if
    /// any byte of the transfer falls outside the owned IOP window —
    /// mirroring the C++ `iopMemSafeReadBytes` contract that the caller
    /// is expected to check the buffer length.
    #[inline]
    pub fn ReadBytes(&self, addr: u32, dst: &mut [u8]) {
        if dst.is_empty() {
            return;
        }
        // Safety: `dst` is a `&mut [u8]`, so its pointer is valid for
        // `dst.len()` writes and is unique for the duration of the call.
        unsafe {
            let _ = iopMemSafeReadBytes(addr, dst.as_mut_ptr(), dst.len() as u32);
        }
    }

    /// Copy `src.len()` bytes from `src` into the IOP bus at `addr`.
    ///
    /// Returns silently if `src` is empty (matching the C++ early-out
    /// for `size == 0`). Bytes that fall outside the owned window are
    /// silently dropped — see [`Self::ReadBytes`] for the matching
    /// behaviour on the read side.
    #[inline]
    pub fn WriteBytes(&self, addr: u32, src: &[u8]) {
        if src.is_empty() {
            return;
        }
        // Safety: `src` is a `&[u8]`, so its pointer is valid for
        // `src.len()` reads for the duration of the call.
        unsafe {
            let _ = iopMemSafeWriteBytes(addr, src.as_ptr(), src.len() as u32);
        }
    }

    /// Compare `src.len()` bytes from the IOP bus at `addr` against `src`.
    ///
    /// Returns 0 if the two ranges are byte-identical, the first non-zero
    /// byte difference otherwise, or `-1` if any byte of the comparison
    /// falls outside the owned IOP window — matching the C++
    /// `iopMemSafeCmpBytes` contract.
    #[inline]
    pub fn CompareBytes(&self, addr: u32, src: &[u8]) -> i32 {
        if src.is_empty() {
            return 0;
        }
        // Safety: `src` is a `&[u8]`, so its pointer is valid for
        // `src.len()` reads for the duration of the call.
        unsafe { iopMemSafeCmpBytes(addr, src.as_ptr(), src.len() as u32) }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mem_size_is_two_mib() {
        assert_eq!(IOP_MEM_SIZE, 0x20_0000);
        assert_eq!(IOP_PAGESIZE, 4096);
        assert_eq!(IOP_PAGEMASK, 0xFFF);
    }

    #[test]
    fn reset_zips_memory() {
        // Pollute the buffer with a recognisable pattern, then reset.
        unsafe {
            iopMem[0] = 0xAB;
            iopMem[IOP_MEM_SIZE - 1] = 0xCD;
        }
        iopMemReset();
        unsafe {
            assert_eq!(iopMem[0], 0);
            assert_eq!(iopMem[IOP_MEM_SIZE - 1], 0);
        }
    }

    #[test]
    fn read_write_8_roundtrip() {
        iopMemReset();
        iopMemWrite8(0x0010_0000, 0x42);
        assert_eq!(iopMemRead8(0x0010_0000), 0x42);
    }

    #[test]
    fn read_write_16_roundtrip() {
        iopMemReset();
        iopMemWrite16(0x0010_0000, 0xBEEF);
        assert_eq!(iopMemRead16(0x0010_0000), 0xBEEF);
    }

    #[test]
    fn read_write_32_roundtrip() {
        iopMemReset();
        iopMemWrite32(0x0010_0000, 0xDEAD_BEEF);
        assert_eq!(iopMemRead32(0x0010_0000), 0xDEAD_BEEF);
    }

    #[test]
    fn out_of_range_reads_return_zero() {
        iopMemReset();
        assert_eq!(iopMemRead8(0x0020_0000), 0);
        assert_eq!(iopMemRead8(0xFFFF_FFFF), 0);
    }

    #[test]
    fn out_of_range_writes_are_dropped() {
        iopMemReset();
        iopMemWrite8(0x0020_0000, 0xFF);
        iopMemWrite32(0x0020_0000, 0xFFFF_FFFF);
        // Nothing should have landed in the buffer.
        for (i, b) in unsafe { &iopMem[..] }.iter().enumerate() {
            assert_eq!(*b, 0, "non-zero byte at offset {:#x}", i);
        }
    }

    #[test]
    fn address_masking_keeps_low_29_bits() {
        iopMemReset();
        // High bits must be masked off: writing to 0xE000_0000 is the same as
        // writing to 0x0000_0000.
        iopMemWrite8(0xE000_0000, 0x77);
        assert_eq!(iopMemRead8(0x0000_0000), 0x77);
    }

    #[test]
    fn read_string_stops_at_nul() {
        iopMemReset();
        // Plant a "hello\0" string at 0x100.
        for (i, b) in b"hello\0".iter().enumerate() {
            iopMemWrite8(0x100 + i as u32, *b);
        }
        assert_eq!(iopMemReadString(0x100, 1024), "hello");
    }
}
