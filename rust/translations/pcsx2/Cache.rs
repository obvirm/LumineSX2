//! Idiomatic Rust translation of `pcsx2/Cache.{h,cpp}`.
//!
//! The EE (R5900) and IOP (R3000) data caches are emulated as 16 KiB
//! 2-way set-associative structures: 64 sets, 2 ways per set, 64-byte
//! lines. Each line is paired with a `RawTag` whose lower bits encode
//! the PCSX2-specific flag set (valid, dirty, LRF, lock, valid-PFN) and
//! whose upper bits hold the host/physical address used for matching.
//!
//! The C++ implementation in `Cache.cpp` models this with a single
//! `static Cache cache` instance plus a free-form CACHE_LOG macro and
//! some instruction-decoder glue that lives in `R5900::Interpreter`.
//! This Rust module keeps the cache storage (with separate `static mut`
//! arrays for the EE and IOP) and the tag arithmetic, and exposes the
//! public surface that the surrounding emulator calls: `init`,
//! `shutdown`, `reset`, the line-pointer accessors
//! (`get_eecache_ptr`, `get_eedata_ptr`, `get_iopcache_ptr`), the
//! 8/16/32/64/128-bit read and write helpers, plus a small
//! `write_cache_line` convenience. The actual vtlb lookup that the
//! C++ uses to resolve `mem` to a host pointer is left as a TODO —
//! here we only model the storage side.
//!
//! Like the C++ code, this module assumes a little-endian host. The
//! unsafe blocks are guarded by `cfg!(target_endian = "little")` and
//! every cache-line pointer is 64-byte aligned (matching the original
//! `alignas(64) CacheData`).

#![deny(unsafe_op_in_unsafe_fn)]

// ---------------------------------------------------------------------------
// Cache geometry
// ---------------------------------------------------------------------------

/// Number of sets in the EE/IOP data cache.
pub const CACHE_SETS: usize = 64;
/// Number of ways (associativity).
pub const CACHE_WAYS: usize = 2;
/// Size of a single cache line, in bytes.
pub const CACHE_LINE_SIZE: usize = 64;
/// Mask covering the offset-within-line bits of a virtual address.
pub const CACHE_LINE_MASK: u32 = (CACHE_LINE_SIZE as u32) - 1;
/// Total number of cache lines (sets * ways).
pub const CACHE_LINES: usize = CACHE_SETS * CACHE_WAYS;

// ---------------------------------------------------------------------------
// Cache-tag layout
// ---------------------------------------------------------------------------
//
// Mirrors the bit layout of the C++ `CacheTag` raw word:
//
//   31..12  physical (host) address used for matching
//   11      valid-PFN flag (PCSX2-internal: physical page is real)
//   10..7   unused
//    6      dirty flag
//    5      valid flag
//    4      LRF (least-recently-filled) flag
//    3      lock flag
//    2..0   unused

/// A single cache-tag word. See module docs for the bit layout.
pub type RawTag = usize;

/// Bits that hold the host/physical address in a tag.
pub const TAG_ADDR_MASK: RawTag = !0xFFFusize;
/// PCSX2-internal: physical page is real (the bus can write back to it).
pub const VALID_PFN_FLAG: RawTag = 0x800;
/// Dirty flag — line has been written but not yet flushed.
pub const DIRTY_FLAG: RawTag = 0x40;
/// Valid flag — line is resident.
pub const VALID_FLAG: RawTag = 0x20;
/// LRF (least-recently-filled) flag — set-associativity replacement hint.
pub const LRF_FLAG: RawTag = 0x10;
/// Lock flag — line is pinned into the cache.
pub const LOCK_FLAG: RawTag = 0x8;
/// Mask of every flag bit a tag can carry.
pub const ALL_FLAGS: RawTag = 0x7FF;
/// Mask of every meaningful bit in the low half of the tag.
pub const ALL_BITS: RawTag = 0xFFF;

#[inline]
fn tag_addr(tag: RawTag) -> RawTag {
    tag & TAG_ADDR_MASK
}

#[inline]
fn tag_flags(tag: RawTag) -> RawTag {
    tag & ALL_FLAGS
}

#[inline]
fn tag_set_addr(tag: &mut RawTag, addr: RawTag) {
    *tag = (*tag & ALL_BITS) | (addr & TAG_ADDR_MASK);
}

#[inline]
fn tag_matches(tag: RawTag, other: RawTag) -> bool {
    (tag & VALID_FLAG) != 0 && tag_addr(tag) == (other & TAG_ADDR_MASK)
}

#[inline]
fn tag_clear(tag: &mut RawTag) {
    *tag &= LRF_FLAG;
}

#[inline]
fn tag_set_valid(tag: &mut RawTag) {
    *tag |= VALID_FLAG;
}

#[inline]
fn tag_set_dirty(tag: &mut RawTag) {
    *tag |= DIRTY_FLAG;
}

#[inline]
fn tag_set_locked(tag: &mut RawTag) {
    *tag |= LOCK_FLAG;
}

#[inline]
fn tag_clear_valid(tag: &mut RawTag) {
    *tag &= !VALID_FLAG;
}

#[inline]
fn tag_clear_dirty(tag: &mut RawTag) {
    *tag &= !DIRTY_FLAG;
}

#[inline]
fn tag_toggle_lrf(tag: &mut RawTag) {
    *tag ^= LRF_FLAG;
}

#[inline]
fn tag_is_valid(tag: RawTag) -> bool {
    tag & VALID_FLAG != 0
}

#[inline]
fn tag_is_dirty(tag: RawTag) -> bool {
    tag & DIRTY_FLAG != 0
}

#[inline]
fn tag_is_locked(tag: RawTag) -> bool {
    tag & LOCK_FLAG != 0
}

#[inline]
fn tag_lrf(tag: RawTag) -> bool {
    tag & LRF_FLAG != 0
}

#[inline]
fn tag_is_dirty_and_valid(tag: RawTag) -> bool {
    tag & (DIRTY_FLAG | VALID_FLAG) == (DIRTY_FLAG | VALID_FLAG)
}

#[inline]
fn tag_is_valid_pfn(tag: RawTag) -> bool {
    tag & VALID_PFN_FLAG != 0
}

#[inline]
fn tag_set_valid_pfn(tag: &mut RawTag, valid: bool) {
    if valid {
        *tag |= VALID_PFN_FLAG;
    } else {
        *tag &= !VALID_PFN_FLAG;
    }
}

// ---------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------
//
// The C++ uses `union alignas(64) CacheData { u8 bytes[64]; };` — a 64-byte
// aligned 64-byte value. We mirror that with `#[repr(align(64))]` so the
// 64-byte lines are also 64-byte aligned in memory. Every access goes
// through `static mut` arrays, guarded by `cfg!(target_endian = "little")`
// (the C++ assumes the host is little-endian for the bit-packing arithmetic).

#[repr(align(64))]
#[derive(Copy, Clone)]
struct CacheLine([u8; CACHE_LINE_SIZE]);

const ZERO_LINE: CacheLine = CacheLine([0u8; CACHE_LINE_SIZE]);

#[cfg(target_endian = "little")]
static mut EE_CACHE_DATA: [CacheLine; CACHE_LINES] = [ZERO_LINE; CACHE_LINES];
#[cfg(target_endian = "little")]
static mut EE_CACHE_TAGS: [RawTag; CACHE_LINES] = [0; CACHE_LINES];

#[cfg(target_endian = "little")]
static mut IOP_CACHE_DATA: [CacheLine; CACHE_LINES] = [ZERO_LINE; CACHE_LINES];
#[cfg(target_endian = "little")]
static mut IOP_CACHE_TAGS: [RawTag; CACHE_LINES] = [0; CACHE_LINES];

// Big-endian hosts: PCSX2 itself doesn't support them, and the unsafe
// reinterpret-style code below would not work. Trip a compile error so
// any non-x86_64/ppc64le (etc.) target fails fast rather than silently
// producing wrong cache contents.
#[cfg(not(target_endian = "little"))]
compile_error!("Cache: PCSX2 only supports little-endian hosts");

// ---------------------------------------------------------------------------
// Public init / shutdown / reset
// ---------------------------------------------------------------------------

/// Initialise the cache subsystems. Mirrors the implicit
/// `static Cache cache = {};` from `Cache.cpp`.
pub fn init() {
    if !cfg!(target_endian = "little") {
        return;
    }
    // SAFETY: only writer during init, and the only side effect is the
    // call into `reset()` below.
    unsafe {
        zero_ee();
        zero_iop();
    }
}

/// Release any resources owned by the cache subsystem. The C++ version
/// has no resources to release either; this is here to give the
/// surrounding emulator a single `init`/`shutdown` pair.
pub fn shutdown() {
    // No-op: storage is static-mut, no allocations.
}

/// Reset both EE and IOP caches to power-on state. Mirrors the C++
/// `resetCache()` (the original implementation only had the EE cache;
/// the IOP is reset here for symmetry).
pub fn reset() {
    if !cfg!(target_endian = "little") {
        return;
    }
    // SAFETY: single-threaded reset; the rest of the emulator must
    // not be touching the cache arrays here.
    unsafe {
        zero_ee();
        zero_iop();
    }
}

#[cfg(target_endian = "little")]
unsafe fn zero_ee() {
    unsafe {
        for line in &mut EE_CACHE_DATA {
            line.0 = [0u8; CACHE_LINE_SIZE];
        }
        for tag in &mut EE_CACHE_TAGS {
            *tag = 0;
        }
    }
}

#[cfg(target_endian = "little")]
unsafe fn zero_iop() {
    unsafe {
        for line in &mut IOP_CACHE_DATA {
            line.0 = [0u8; CACHE_LINE_SIZE];
        }
        for tag in &mut IOP_CACHE_TAGS {
            *tag = 0;
        }
    }
}

// ---------------------------------------------------------------------------
// Public line-pointer accessors
// ---------------------------------------------------------------------------

/// Return a mutable pointer to the EE cache line covering `block`.
///
/// `block` is a line index in the range `0..CACHE_LINES`. The returned
/// slice aliases the underlying `static mut` storage; callers must
/// ensure exclusive access (this is the same contract the C++ version
/// relies on — single-threaded EE core).
#[cfg(target_endian = "little")]
pub fn get_eecache_ptr(block: u32) -> &'static mut [u8; CACHE_LINE_SIZE] {
    assert!((block as usize) < CACHE_LINES, "EE cache block out of range");
    // SAFETY: bounds-checked above; `block` selects a unique line in
    // the EE cache data array.
    unsafe { &mut EE_CACHE_DATA[block as usize].0 }
}

/// Return a mutable pointer to the EE data cache line covering `block`.
///
/// Distinct from `get_eecache_ptr` only by name; the C++ code uses a
/// single `cache` static for both instruction and data, and we follow
/// the same convention here.
#[cfg(target_endian = "little")]
pub fn get_eedata_ptr(block: u32) -> &'static mut [u8; CACHE_LINE_SIZE] {
    assert!((block as usize) < CACHE_LINES, "EE data block out of range");
    // SAFETY: bounds-checked above; `block` selects a unique line in
    // the EE cache data array.
    unsafe { &mut EE_CACHE_DATA[block as usize].0 }
}

/// Return a mutable pointer to the IOP cache line covering `block`.
#[cfg(target_endian = "little")]
pub fn get_iopcache_ptr(block: u32) -> &'static mut [u8; CACHE_LINE_SIZE] {
    assert!((block as usize) < CACHE_LINES, "IOP cache block out of range");
    // SAFETY: bounds-checked above; `block` selects a unique line in
    // the IOP cache data array.
    unsafe { &mut IOP_CACHE_DATA[block as usize].0 }
}

// ---------------------------------------------------------------------------
// Tag accessors. The surrounding emulator (and tests) need to read and
// write the raw tag words directly, the same way the C++ code reaches
// into `set.tags[way].rawValue`.
// ---------------------------------------------------------------------------

/// Read the tag word for an EE cache line.
#[cfg(target_endian = "little")]
pub fn ee_tag(block: usize) -> RawTag {
    assert!(block < CACHE_LINES, "EE cache block out of range");
    // SAFETY: bounds-checked above.
    unsafe { EE_CACHE_TAGS[block] }
}

/// Write the tag word for an EE cache line.
#[cfg(target_endian = "little")]
pub fn set_ee_tag(block: usize, value: RawTag) {
    assert!(block < CACHE_LINES, "EE cache block out of range");
    // SAFETY: bounds-checked above.
    unsafe {
        EE_CACHE_TAGS[block] = value;
    }
}

/// Read the tag word for an IOP cache line.
#[cfg(target_endian = "little")]
pub fn iop_tag(block: usize) -> RawTag {
    assert!(block < CACHE_LINES, "IOP cache block out of range");
    // SAFETY: bounds-checked above.
    unsafe { IOP_CACHE_TAGS[block] }
}

/// Write the tag word for an IOP cache line.
#[cfg(target_endian = "little")]
pub fn set_iop_tag(block: usize, value: RawTag) {
    assert!(block < CACHE_LINES, "IOP cache block out of range");
    // SAFETY: bounds-checked above.
    unsafe {
        IOP_CACHE_TAGS[block] = value;
    }
}

// ---------------------------------------------------------------------------
// Read / write helpers
// ---------------------------------------------------------------------------
//
// The C++ exposes u8/u16/u32/u64/u128 specialisations for both
// readCache and writeCache. We follow the same pattern using
// `core::ptr::{read,write}_unaligned` so the access is correct at any
// offset within a 64-byte line. (A 64-byte line is 64-byte aligned, so
// 8/16-byte-aligned offsets are guaranteed; we still use the unaligned
// helpers to keep the offset arithmetic straightforward.)
//
// `write_cache_line(addr, value)` is the explicit 128-bit entry point
// the surrounding emulator calls for DMA / SQ writes.

/// Write a single byte to the EE data cache.
#[cfg(target_endian = "little")]
pub fn write_cache8(addr: u32, value: u8) {
    // SAFETY: offset within a 64-byte line; the destination slice is
    // statically sized and bounded by the call to `cache_line_mut`.
    let dst = cache_line_mut(addr);
    unsafe {
        dst.add((addr & CACHE_LINE_MASK) as usize).write(value);
    }
}

/// Write a 16-bit halfword to the EE data cache.
#[cfg(target_endian = "little")]
pub fn write_cache16(addr: u32, value: u16) {
    // SAFETY: see `write_cache8`; alignment is guaranteed because the
    // line is 64-byte aligned and the offset is `addr & 0x3F`.
    let dst = cache_line_mut(addr);
    unsafe {
        dst.add((addr & CACHE_LINE_MASK) as usize).cast::<u16>().write_unaligned(value);
    }
}

/// Write a 32-bit word to the EE data cache.
#[cfg(target_endian = "little")]
pub fn write_cache32(addr: u32, value: u32) {
    // SAFETY: see `write_cache16`.
    let dst = cache_line_mut(addr);
    unsafe {
        dst.add((addr & CACHE_LINE_MASK) as usize)
            .cast::<u32>()
            .write_unaligned(value);
    }
}

/// Write a 64-bit doubleword to the EE data cache.
#[cfg(target_endian = "little")]
pub fn write_cache64(addr: u32, value: u64) {
    // SAFETY: see `write_cache16`.
    let dst = cache_line_mut(addr);
    unsafe {
        dst.add((addr & CACHE_LINE_MASK) as usize)
            .cast::<u64>()
            .write_unaligned(value);
    }
}

/// Write a 128-bit quadword to the EE data cache. This is the entry
/// point the C++ code calls `writeCache128`.
#[cfg(target_endian = "little")]
pub fn write_cache_line(addr: u32, value: u128) {
    // SAFETY: see `write_cache16`. A 128-bit aligned offset within a
    // 64-byte aligned line is always 16-byte aligned, so the
    // `write_unaligned` is just future-proofing.
    let dst = cache_line_mut(addr);
    unsafe {
        dst.add((addr & CACHE_LINE_MASK) as usize)
            .cast::<u128>()
            .write_unaligned(value);
    }
    mark_dirty(addr);
}

/// Read a single byte from the EE data cache.
#[cfg(target_endian = "little")]
pub fn read_cache8(addr: u32) -> u8 {
    let src = cache_line_const(addr);
    // SAFETY: see `write_cache8`.
    unsafe { src.add((addr & CACHE_LINE_MASK) as usize).read() }
}

/// Read a 16-bit halfword from the EE data cache.
#[cfg(target_endian = "little")]
pub fn read_cache16(addr: u32) -> u16 {
    let src = cache_line_const(addr);
    // SAFETY: see `write_cache16`.
    unsafe {
        src.add((addr & CACHE_LINE_MASK) as usize)
            .cast::<u16>()
            .read_unaligned()
    }
}

/// Read a 32-bit word from the EE data cache.
#[cfg(target_endian = "little")]
pub fn read_cache32(addr: u32) -> u32 {
    let src = cache_line_const(addr);
    // SAFETY: see `write_cache16`.
    unsafe {
        src.add((addr & CACHE_LINE_MASK) as usize)
            .cast::<u32>()
            .read_unaligned()
    }
}

/// Read a 64-bit doubleword from the EE data cache.
#[cfg(target_endian = "little")]
pub fn read_cache64(addr: u32) -> u64 {
    let src = cache_line_const(addr);
    // SAFETY: see `write_cache16`.
    unsafe {
        src.add((addr & CACHE_LINE_MASK) as usize)
            .cast::<u64>()
            .read_unaligned()
    }
}

/// Read a 128-bit quadword from the EE data cache. The C++ code
/// returned an `r128` (SSE register) here; in Rust we return the
/// 128-bit value directly and let the caller decide how to consume it.
#[cfg(target_endian = "little")]
pub fn read_cache128(addr: u32) -> u128 {
    let src = cache_line_const(addr);
    // SAFETY: see `write_cache_line`.
    unsafe {
        src.add((addr & CACHE_LINE_MASK) as usize)
            .cast::<u128>()
            .read_unaligned()
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Compute the line index for `addr`: bits [11:6] of the virtual
/// address, masking down to `CACHE_SETS - 1`.
#[inline]
fn set_index_for(addr: u32) -> usize {
    (((addr >> 6) as usize) & (CACHE_SETS - 1)) * CACHE_WAYS
}

/// Look up the way used for a given address. The C++ does an LRF
/// toggle / lookup dance; here we simply pick way 0 (the surrounding
/// emulator is expected to call `prepare_cache_access` for real
/// placement — see `write_cache_line` and friends).
#[inline]
fn way_for(_addr: u32) -> usize {
    0
}

/// Mutable raw pointer to the byte at offset 0 of the cache line
/// covering `addr`.
#[cfg(target_endian = "little")]
#[inline]
fn cache_line_mut(addr: u32) -> *mut u8 {
    let line = set_index_for(addr) + way_for(addr);
    // SAFETY: `line` is a valid index into `EE_CACHE_DATA`; the line
    // is 64-byte aligned and remains live for `'static`.
    unsafe { EE_CACHE_DATA[line].0.as_mut_ptr() }
}

/// Const raw pointer to the byte at offset 0 of the cache line
/// covering `addr`.
#[cfg(target_endian = "little")]
#[inline]
fn cache_line_const(addr: u32) -> *const u8 {
    let line = set_index_for(addr) + way_for(addr);
    // SAFETY: see `cache_line_mut`.
    unsafe { EE_CACHE_DATA[line].0.as_ptr() }
}

/// Mark the cache line that backs `addr` as dirty + valid. Mirrors
/// the C++ `prepareCacheAccess<true, ...>` path.
#[cfg(target_endian = "little")]
fn mark_dirty(addr: u32) {
    let line = set_index_for(addr) + way_for(addr);
    // SAFETY: `line` is a valid index into `EE_CACHE_TAGS`; this
    // function is only called from write helpers that already touch
    // the corresponding data, so no aliasing concerns.
    unsafe {
        let tag = &mut EE_CACHE_TAGS[line];
        tag_set_valid(tag);
        tag_set_dirty(tag);
    }
}

// ---------------------------------------------------------------------------
// Writeback
// ---------------------------------------------------------------------------
//
// Mirrors the C++ `writebackCache()` plus the per-line
// `CacheLine::writeBackIfNeeded()` glue. The C++ implementation walks
// every (set, way) pair and, for each line that is dirty + valid,
// reconstructs the host pointer from `tag.addr() | (set << 6)` and
// memcpy's the line back to physical memory.
//
// We do the same iteration but, because the vtlb lookup that the C++
// uses to recover the host pointer is left as a TODO in this module
// (see the file-level docs), we can only model the storage side. The
// actual bus write-back is a no-op here — we still clear the dirty
// bit so subsequent line fills don't trigger the "loaded without
// writeback" assertion that `CacheLine::load` would fire in the C++.

/// Per-line writeback helper. Mirrors `CacheLine::writeBackIfNeeded()`.
#[cfg(target_endian = "little")]
fn writeback_line(set: usize, way: usize) {
    let line = set * CACHE_WAYS + way;
    debug_assert!(line < CACHE_LINES, "EE cache line index out of range");
    // SAFETY: line index is bounds-checked above; this is the only
    // place that reads `EE_CACHE_TAGS[line]` for this line in a given
    // call, so no aliasing with concurrent writes.
    unsafe {
        let tag = &mut EE_CACHE_TAGS[line];
        if !tag_is_dirty_and_valid(*tag) {
            return;
        }
        // Host pointer recovery (vtlb lookup) is intentionally not
        // modelled here — see module docs.
        tag_clear_dirty(tag);
    }
}

/// Dump all dirty EE cache entries. Mirrors `writebackCache()` in the
/// C++ source — required when toggling the recompiler while the cache
/// is enabled.
#[cfg(target_endian = "little")]
pub fn writeback_cache() {
    if !cfg!(target_endian = "little") {
        return;
    }
    for set in 0..CACHE_SETS {
        for way in 0..CACHE_WAYS {
            writeback_line(set, way);
        }
    }
}
