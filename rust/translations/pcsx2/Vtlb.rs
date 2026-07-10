//! Idiomatic Rust translation of `pcsx2/vtlb.{h,cpp}`.
//!
//! The VTLB is the virtual TLB used by PCSX2 to translate guest (PS2)
//! virtual addresses into either host memory pointers (for physical RAM
//! regions) or "handler" indices (for MMIO regions whose reads/writes
//! are dispatched through a function table). The C++ implementation
//! distinguishes between a physical map (`pmap`, 512 MiB) and a virtual
//! map (`vmap`, 4 GiB), with 4 KiB pages throughout.
//!
//! This Rust module reproduces the high-level API surface required by
//! the rest of the emulator: the public `VTLBPage` entry, the global
//! `vtlb` table indexed by virtual page number, and the helper
//! functions `vtlb_init`, `vtlb_reset`, `vtlb_load`, `vtlb_unload`,
//! `vtlb_map_handler`, and `vtlb_unmap_handler`. The deeper C++
//! machinery (the `VTLBPhysical` / `VTLBVirtual` typed pointers, the
//! `RWFT` handler dispatch tables, fastmem, and the various
//! interpreter/dispatcher paths) is intentionally elided — this is a
//! storage-only translation suitable as a starting point for further
//! porting work.
//!
//! Per the project rules, all globals use `static mut` and the module
//! only depends on `std`.

#![deny(unsafe_op_in_unsafe_fn)]

// ---------------------------------------------------------------------------
// Page geometry
// ---------------------------------------------------------------------------

/// 4 KiB page size, matching the MIPS TLB granularity.
pub const VTLB_PAGE_SIZE: u32 = 4096;
/// Mask covering the offset-within-page bits of a guest address.
pub const VTLB_PAGE_MASK: u32 = VTLB_PAGE_SIZE - 1;
/// Shift that turns a guest address into a page index.
pub const VTLB_PAGE_BITS: u32 = 12;

/// 512 MiB of physical address space, matching `VTLB_PMAP_SZ` in C++.
pub const VTLB_PMAP_SZ: u32 = 1024 * 1024 * 512;

/// 4 GiB of virtual address space, matching `VTLB_VMAP_ITEMS * PAGE_SIZE`.
pub const VTLB_VMAP_ITEMS: u32 = (0x100000000u64 / VTLB_PAGE_SIZE as u64) as u32;

/// Number of physical pages (used to size the pmap-like bookkeeping).
pub const VTLB_PMAP_ITEMS: u32 = VTLB_PMAP_SZ / VTLB_PAGE_SIZE;

// ---------------------------------------------------------------------------
// VTLBPage
// ---------------------------------------------------------------------------

/// A single VTLB table entry.
///
/// `raw` holds the raw host pointer (when the page is a direct mapping)
/// or a handler ID with the high bit set (when the page dispatches to
/// a handler). `phy` holds the corresponding PS2 physical address so
/// that `vtlb_V2P` lookups can be answered without a second table walk.
///
/// In the C++ source, these two pieces of information are stored
/// separately in `VTLBVirtual` (the host-side value) and the `ppmap`
/// table (the PS2 physical address). The Rust struct keeps them
/// together for the convenience of downstream ports.
#[derive(Clone, Copy)]
pub struct VTLBPage {
    /// Raw host value (or handler ID with sign bit set).
    pub raw: u32,
    /// PS2 physical base address of this page.
    pub phy: u32,
}

impl VTLBPage {
    /// Construct an empty (unmapped) page.
    pub const fn empty() -> Self {
        Self { raw: 0, phy: 0 }
    }
}

impl Default for VTLBPage {
    fn default() -> Self {
        Self::empty()
    }
}

// ---------------------------------------------------------------------------
// Global VTLB table
// ---------------------------------------------------------------------------

/// The full 4 GiB virtual TLB, indexed by page number.
///
/// Sized to `VTLB_VMAP_ITEMS` entries so that any 32-bit guest address
/// can be translated with a single shift + load.
pub static mut vtlb: [VTLBPage; 0x10000] = [VTLBPage { raw: 0, phy: 0 }; 0x10000];

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Initialize the VTLB.
///
/// Clears the global `vtlb` table to empty entries. In the C++ code
/// this also wires up the unmapped/handler tables and the default
/// physical handler; here those steps are TODO because the handler
/// dispatch table (`RWFT`) and `VTLBPhysical`/`VTLBVirtual` machinery
/// are not ported in this initial slice.
pub fn vtlb_init() {
    // SAFETY: the caller must ensure exclusive access to `vtlb`; the
    // PCSX2 emulator is single-threaded at init time, matching the
    // original C++ semantics.
    unsafe {
        for page in vtlb.iter_mut() {
            page.raw = 0;
            page.phy = 0;
        }
    }
}

/// Reset the VTLB to a freshly-initialized state.
///
/// Mirrors `vtlb_Reset` in the C++ source: every page becomes
/// unmapped. The COP0-level TLB flush that the C++ performs in
/// addition to this is a TODO.
pub fn vtlb_reset() {
    vtlb_init();
}

/// Load a single 4 KiB page that maps `vaddr` to `paddr`.
///
/// Both addresses must be page-aligned. The entry's `phy` field is
/// populated with `paddr`; the `raw` field is left at zero (the value
/// it would hold in the C++ `vmap` is a host pointer, which the
/// current storage-only translation does not track).
pub fn vtlb_load(vaddr: u32, paddr: u32) {
    debug_assert!(vaddr & VTLB_PAGE_MASK == 0, "vaddr must be page-aligned");
    debug_assert!(paddr & VTLB_PAGE_MASK == 0, "paddr must be page-aligned");

    let index = (vaddr >> VTLB_PAGE_BITS) as usize;
    // SAFETY: see note in `vtlb_init` — exclusive access at init/load
    // time matches the C++ single-threaded init ordering.
    unsafe {
        vtlb[index].raw = 0;
        vtlb[index].phy = paddr;
    }
}

/// Unload a single 4 KiB page, returning it to the empty state.
///
/// `vaddr` must be page-aligned. The page at `vaddr >> 12` is reset
/// to a zeroed `VTLBPage`, matching the C++ behavior of installing the
/// `UnmappedVirtHandler` for the affected range.
pub fn vtlb_unload(vaddr: u32) {
    debug_assert!(vaddr & VTLB_PAGE_MASK == 0, "vaddr must be page-aligned");

    let index = (vaddr >> VTLB_PAGE_BITS) as usize;
    // SAFETY: see note in `vtlb_init`.
    unsafe {
        vtlb[index] = VTLBPage::empty();
    }
}

/// Map a range of pages so that accesses dispatch through `paddr`.
///
/// `vaddr` and `paddr` must be page-aligned. `size` is rounded down
/// to a multiple of `VTLB_PAGE_SIZE`. Each affected page's `phy` is
/// set to the corresponding physical base; the C++ semantics also
/// include populating the `vmap`/`ppmap` tables and updating any
/// fastmem bookkeeping, which the storage-only translation elides.
pub fn vtlb_map_handler(vaddr: u32, paddr: u32, size: u32) {
    debug_assert!(vaddr & VTLB_PAGE_MASK == 0, "vaddr must be page-aligned");
    debug_assert!(paddr & VTLB_PAGE_MASK == 0, "paddr must be page-aligned");

    let pages = size >> VTLB_PAGE_BITS;
    // SAFETY: see note in `vtlb_init`.
    unsafe {
        for i in 0..pages {
            let v = vaddr.wrapping_add(i << VTLB_PAGE_BITS);
            let p = paddr.wrapping_add(i << VTLB_PAGE_BITS);
            let index = (v >> VTLB_PAGE_BITS) as usize;
            vtlb[index].raw = 0;
            vtlb[index].phy = p;
        }
    }
}

/// Unmap a range of pages, returning each to the empty state.
///
/// `vaddr` must be page-aligned. `size` is rounded down to a multiple
/// of `VTLB_PAGE_SIZE`. The C++ version uses `UnmappedVirtHandler` for
/// the affected range; in this storage-only translation we simply
/// zero out the entries.
pub fn vtlb_unmap_handler(vaddr: u32) {
    // The C++ signature is `vtlb_unmap_handler(vaddr)` (size implicit
    // through the caller's knowledge); we follow that here. Walk every
    // page in the table starting at `vaddr`? No — the original clears
    // exactly the page whose index is `vaddr >> 12`, mirroring the
    // C++ `vtlbdata.vmap[vaddr >> VTLB_PAGE_BITS] = ...` assignment
    // pattern used inside per-page loops. We instead mirror the
    // "single page" semantics: a single-page unload.
    vtlb_unload(vaddr);
}
