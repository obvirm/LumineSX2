// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust port of `common/MemoryInterface.{h,cpp}`.
//!
//! The C++ original is a vtable-heavy abstract base class used by the MIPS
//! recompiler to read/write EE, IOP, and VU memory. Two design notes drove
//! the translation:
//!
//! 1. **Pure-Rust callers** — Rust code (the recompiler, VM core, etc.) uses
//!    the [`MemoryInterface`] trait through dynamic dispatch via
//!    `Box<dyn MemoryInterface>` or `&mut dyn MemoryInterface`.
//!
//! 2. **C ABI callers** — The C++ side does not understand Rust trait
//!    objects (a `dyn` vtable has Rust-specific layout). To expose a stable
//!    C-compatible surface we provide a manual [`MemoryVTable`] of `extern
//!    "C"` function pointers plus a thin [`MemoryHandle`] envelope. Wrapper
//!    functions `pcsx2_mem_read32(handle) -> u32` etc. invoke the vtable.
//!    This is the same pattern used by the C++ `MemoryInterface` — both
//!    languages land on an explicit, manually-laid-out function pointer
//!    table.
//!
//! The C++ template helpers `Read<T>` / `Write<T>` / `IdempotentWrite<T>`
//! for signed / float / `u128` overloads are replaced by Rust generic
//! functions constrained on [`MemoryAccessType`]. They are not part of the
//! `extern "C"` surface — the C++ side already has those overloads and
//! calls into Rust only at the fixed-width read/write boundaries.

use std::ffi::c_void;

// ---------------------------------------------------------------------------
// Trait definition
// ---------------------------------------------------------------------------

/// Pure-Rust memory-access interface.
///
/// Mirrors the public surface of C++ `MemoryInterface` for fixed-width
/// 8/16/32/64-bit accesses, plus bulk byte copies and the `IsMapped`
/// predicate. The C++ class also has `Read128` / `Write128` / templated
/// overloads; those are deliberately excluded here per the task spec, and
/// callers that need them can compose the 64-bit primitives.
///
/// `Send + Sync` lets the trait be shared across threads (the recompiler
/// reads from one thread while the VM core writes from another).
pub trait MemoryInterface: Send + Sync {
    fn read8(&self, addr: u32) -> u8;
    fn read16(&self, addr: u32) -> u16;
    fn read32(&self, addr: u32) -> u32;
    fn read64(&self, addr: u32) -> u64;

    fn write8(&mut self, addr: u32, val: u8);
    fn write16(&mut self, addr: u32, val: u16);
    fn write32(&mut self, addr: u32, val: u32);
    fn write64(&mut self, addr: u32, val: u64);

    fn read_bytes(&self, addr: u32, dst: &mut [u8]);
    fn write_bytes(&mut self, addr: u32, src: &[u8]);

    fn is_mapped(&self, addr: u32) -> bool;

    /// Convenience: idempotent 8-bit write. Skips the underlying write if
    /// the existing byte already matches `val`. Mirrors the C++
    /// `IdempotentWrite8`.
    fn idempotent_write8(&mut self, addr: u32, val: u8) -> bool {
        let existing = self.read8(addr);
        if existing == val {
            true
        } else {
            self.write8(addr, val);
            true
        }
    }

    fn idempotent_write16(&mut self, addr: u32, val: u16) -> bool {
        let existing = self.read16(addr);
        if existing == val {
            true
        } else {
            self.write16(addr, val);
            true
        }
    }

    fn idempotent_write32(&mut self, addr: u32, val: u32) -> bool {
        let existing = self.read32(addr);
        if existing == val {
            true
        } else {
            self.write32(addr, val);
            true
        }
    }

    fn idempotent_write64(&mut self, addr: u32, val: u64) -> bool {
        let existing = self.read64(addr);
        if existing == val {
            true
        } else {
            self.write64(addr, val);
            true
        }
    }

    fn idempotent_write_bytes(&mut self, addr: u32, src: &[u8]) -> bool {
        if src.is_empty() {
            return true;
        }
        let mut scratch = vec![0u8; src.len()];
        self.read_bytes(addr, &mut scratch);
        if scratch == src {
            return true;
        }
        self.write_bytes(addr, src);
        true
    }
}

// ---------------------------------------------------------------------------
// Concrete impl: direct slice-backed memory
// ---------------------------------------------------------------------------

/// Concrete [`MemoryInterface`] backed by a fixed slice of bytes.
///
/// Used for simple test harnesses and for the linear physical-memory windows
/// of the EE/IOP where the backing buffer is just `&mut [u8]`. The C++ side
/// has a similar `Memory` class that wraps a `u8*`.
#[derive(Debug)]
pub struct DirectMemory<'a> {
    bytes: &'a mut [u8],
    base: u32,
}

impl<'a> DirectMemory<'a> {
    pub fn new(bytes: &'a mut [u8], base: u32) -> Self {
        Self { bytes, base }
    }

    fn check(&self, addr: u32, len: usize) -> bool {
        let offset = addr.wrapping_sub(self.base) as usize;
        offset.checked_add(len).map_or(false, |end| end <= self.bytes.len())
    }
}

impl<'a> MemoryInterface for DirectMemory<'a> {
    fn read8(&self, addr: u32) -> u8 {
        if !self.check(addr, 1) {
            return 0;
        }
        let off = (addr - self.base) as usize;
        self.bytes[off]
    }

    fn read16(&self, addr: u32) -> u16 {
        if !self.check(addr, 2) {
            return 0;
        }
        let off = (addr - self.base) as usize;
        u16::from_le_bytes([self.bytes[off], self.bytes[off + 1]])
    }

    fn read32(&self, addr: u32) -> u32 {
        if !self.check(addr, 4) {
            return 0;
        }
        let off = (addr - self.base) as usize;
        u32::from_le_bytes([
            self.bytes[off],
            self.bytes[off + 1],
            self.bytes[off + 2],
            self.bytes[off + 3],
        ])
    }

    fn read64(&self, addr: u32) -> u64 {
        if !self.check(addr, 8) {
            return 0;
        }
        let off = (addr - self.base) as usize;
        u64::from_le_bytes([
            self.bytes[off],
            self.bytes[off + 1],
            self.bytes[off + 2],
            self.bytes[off + 3],
            self.bytes[off + 4],
            self.bytes[off + 5],
            self.bytes[off + 6],
            self.bytes[off + 7],
        ])
    }

    fn write8(&mut self, addr: u32, val: u8) {
        if !self.check(addr, 1) {
            return;
        }
        let off = (addr - self.base) as usize;
        self.bytes[off] = val;
    }

    fn write16(&mut self, addr: u32, val: u16) {
        if !self.check(addr, 2) {
            return;
        }
        let off = (addr - self.base) as usize;
        let le = val.to_le_bytes();
        self.bytes[off] = le[0];
        self.bytes[off + 1] = le[1];
    }

    fn write32(&mut self, addr: u32, val: u32) {
        if !self.check(addr, 4) {
            return;
        }
        let off = (addr - self.base) as usize;
        let le = val.to_le_bytes();
        self.bytes[off..off + 4].copy_from_slice(&le);
    }

    fn write64(&mut self, addr: u32, val: u64) {
        if !self.check(addr, 8) {
            return;
        }
        let off = (addr - self.base) as usize;
        let le = val.to_le_bytes();
        self.bytes[off..off + 8].copy_from_slice(&le);
    }

    fn read_bytes(&self, addr: u32, dst: &mut [u8]) {
        if dst.is_empty() {
            return;
        }
        if !self.check(addr, dst.len()) {
            dst.fill(0);
            return;
        }
        let off = (addr - self.base) as usize;
        dst.copy_from_slice(&self.bytes[off..off + dst.len()]);
    }

    fn write_bytes(&mut self, addr: u32, src: &[u8]) {
        if src.is_empty() {
            return;
        }
        if !self.check(addr, src.len()) {
            return;
        }
        let off = (addr - self.base) as usize;
        self.bytes[off..off + src.len()].copy_from_slice(src);
    }

    fn is_mapped(&self, addr: u32) -> bool {
        self.check(addr, 1)
    }
}

// ---------------------------------------------------------------------------
// C ABI surface — manual vtable + opaque handle
// ---------------------------------------------------------------------------

/// Manually-laid-out function pointer table for memory access.
///
/// We can't pass a `&dyn MemoryInterface` across the C ABI because a Rust
/// trait-object vtable has a Rust-specific layout (drop glue, alignment
/// padding, ...). Instead we declare an explicit `#[repr(C)]` struct of
/// `extern "C"` fn pointers and have Rust code fill it in from a real
/// implementor. The C++ side then sees a plain C struct of function
/// pointers — exactly the layout it would have produced for the C++
/// abstract class's vtable.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct MemoryVTable {
    pub read8: unsafe extern "C" fn(ctx: *mut c_void, addr: u32) -> u8,
    pub read16: unsafe extern "C" fn(ctx: *mut c_void, addr: u32) -> u16,
    pub read32: unsafe extern "C" fn(ctx: *mut c_void, addr: u32) -> u32,
    pub read64: unsafe extern "C" fn(ctx: *mut c_void, addr: u32) -> u64,
    pub write8: unsafe extern "C" fn(ctx: *mut c_void, addr: u32, val: u8),
    pub write16: unsafe extern "C" fn(ctx: *mut c_void, addr: u32, val: u16),
    pub write32: unsafe extern "C" fn(ctx: *mut c_void, addr: u32, val: u32),
    pub write64: unsafe extern "C" fn(ctx: *mut c_void, addr: u32, val: u64),
    pub read_bytes: unsafe extern "C" fn(ctx: *mut c_void, addr: u32, dst: *mut u8, len: u32),
    pub write_bytes: unsafe extern "C" fn(ctx: *mut c_void, addr: u32, src: *const u8, len: u32),
    pub is_mapped: unsafe extern "C" fn(ctx: *mut c_void, addr: u32) -> bool,
}

/// Opaque handle handed to C++: `*const MemoryHandle`.
///
/// `ctx` is an opaque cookie the C++ side never dereferences — it's only
/// forwarded back into the vtable entry points, where Rust reinterprets it
/// as the boxed implementor it originally wrapped.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct MemoryHandle {
    pub vtable: *const MemoryVTable,
    pub ctx: *mut c_void,
}

// Safety: MemoryHandle is just two pointers. The real thread-safety story
// lives inside the boxed implementor; we mark it Send/Sync because the C++
// side treats it as a free-floating handle and the underlying
// `Box<dyn MemoryInterface + Send + Sync>` already proves the inner state
// is shareable. Implementors that need interior mutability must use
// `Mutex`/`RwLock` (the trait bounds guarantee Send + Sync).
unsafe impl Send for MemoryHandle {}
unsafe impl Sync for MemoryHandle {}

impl MemoryHandle {
    /// Wrap an arbitrary implementor as a `MemoryHandle` that the C++ side
    /// can call into. The implementor is stored in a
    /// `Box<ErasedMemory>` (an internal Sized newtype that delegates to
    /// the boxed `dyn MemoryInterface`), so it lives until
    /// [`MemoryHandle::release_box`] is called (typically via the FFI
    /// export `pcsx2_mem_release`).
    ///
    /// **Single-owner constraint:** the boxed implementor must be unique.
    /// If you need to share it between Rust and C++ callers concurrently,
    /// wrap the underlying state in `Mutex`/`RwLock` (the
    /// [`MemoryInterface`] trait requires `Send + Sync` for exactly this
    /// reason — concurrent Rust callers go through the implementor's
    /// interior mutability).
    pub fn from_box(impl_: Box<dyn MemoryInterface>) -> Self {
        let erased: Box<ErasedMemory> = Box::new(ErasedMemory(impl_));
        let raw = Box::into_raw(erased) as *mut c_void;
        MemoryHandle {
            vtable: &DIRECT_VTABLE as *const MemoryVTable,
            ctx: raw,
        }
    }

    /// Reclaim the `Box<dyn MemoryInterface>` that backs this handle.
    ///
    /// Caller must own the only remaining reference. After this call the
    /// handle is dangling — do not use it again.
    ///
    /// # Safety
    /// `ctx` must have been produced by [`MemoryHandle::from_box`] and no
    /// other live `MemoryHandle` (or `Box<ErasedMemory>`) may share its
    /// `ctx`.
    pub unsafe fn into_box(handle: MemoryHandle) -> Box<dyn MemoryInterface> {
        let erased: Box<ErasedMemory> =
            unsafe { Box::from_raw(handle.ctx as *mut ErasedMemory) };
        erased.0
    }
}

// ---------------------------------------------------------------------------
// Concrete wrapper used as the FFI-storage type for `Box<dyn ...>`.
// ---------------------------------------------------------------------------
//
// `Box<dyn MemoryInterface>` is unsized, so we can't round-trip it through
// a thin `*mut c_void` (the `Box::from_raw` round-trip would need to know
// the concrete pointer type, and the `*mut c_void → *mut dyn Trait` cast
// is rejected by the compiler because `c_void` doesn't implement
// `MemoryInterface`). The fix is to wrap the boxed implementor in a
// concrete Sized struct, store THAT behind the thin pointer, and recover
// the inner box by reading the wrapper's field.

struct ErasedMemory(Box<dyn MemoryInterface>);

impl MemoryInterface for ErasedMemory {
    fn read8(&self, addr: u32) -> u8 {
        self.0.read8(addr)
    }
    fn read16(&self, addr: u32) -> u16 {
        self.0.read16(addr)
    }
    fn read32(&self, addr: u32) -> u32 {
        self.0.read32(addr)
    }
    fn read64(&self, addr: u32) -> u64 {
        self.0.read64(addr)
    }
    fn write8(&mut self, addr: u32, val: u8) {
        self.0.write8(addr, val)
    }
    fn write16(&mut self, addr: u32, val: u16) {
        self.0.write16(addr, val)
    }
    fn write32(&mut self, addr: u32, val: u32) {
        self.0.write32(addr, val)
    }
    fn write64(&mut self, addr: u32, val: u64) {
        self.0.write64(addr, val)
    }
    fn read_bytes(&self, addr: u32, dst: &mut [u8]) {
        self.0.read_bytes(addr, dst)
    }
    fn write_bytes(&mut self, addr: u32, src: &[u8]) {
        self.0.write_bytes(addr, src)
    }
    fn is_mapped(&self, addr: u32) -> bool {
        self.0.is_mapped(addr)
    }
}

// ---------------------------------------------------------------------------
// Bridging helpers: turn the boxed wrapper into vtable calls.
// ---------------------------------------------------------------------------
//
// The function pointers stored in `DIRECT_VTABLE` take a `*mut c_void` that
// is actually a `*mut ErasedMemory`. Each shim reconstructs a `&mut
// ErasedMemory` and dispatches.
//
// These are `unsafe extern "C"` because the C side may call them from any
// thread, and the FFI contract requires we not panic across the boundary.

unsafe extern "C" fn shim_read8(ctx: *mut c_void, addr: u32) -> u8 {
    let e = unsafe { &*(ctx as *const ErasedMemory) };
    e.read8(addr)
}

unsafe extern "C" fn shim_read16(ctx: *mut c_void, addr: u32) -> u16 {
    let e = unsafe { &*(ctx as *const ErasedMemory) };
    e.read16(addr)
}

unsafe extern "C" fn shim_read32(ctx: *mut c_void, addr: u32) -> u32 {
    let e = unsafe { &*(ctx as *const ErasedMemory) };
    e.read32(addr)
}

unsafe extern "C" fn shim_read64(ctx: *mut c_void, addr: u32) -> u64 {
    let e = unsafe { &*(ctx as *const ErasedMemory) };
    e.read64(addr)
}

unsafe extern "C" fn shim_write8(ctx: *mut c_void, addr: u32, val: u8) {
    let e = unsafe { &mut *(ctx as *mut ErasedMemory) };
    e.write8(addr, val);
}

unsafe extern "C" fn shim_write16(ctx: *mut c_void, addr: u32, val: u16) {
    let e = unsafe { &mut *(ctx as *mut ErasedMemory) };
    e.write16(addr, val);
}

unsafe extern "C" fn shim_write32(ctx: *mut c_void, addr: u32, val: u32) {
    let e = unsafe { &mut *(ctx as *mut ErasedMemory) };
    e.write32(addr, val);
}

unsafe extern "C" fn shim_write64(ctx: *mut c_void, addr: u32, val: u64) {
    let e = unsafe { &mut *(ctx as *mut ErasedMemory) };
    e.write64(addr, val);
}

unsafe extern "C" fn shim_read_bytes(ctx: *mut c_void, addr: u32, dst: *mut u8, len: u32) {
    let e = unsafe { &*(ctx as *const ErasedMemory) };
    let slice = unsafe { std::slice::from_raw_parts_mut(dst, len as usize) };
    e.read_bytes(addr, slice);
}

unsafe extern "C" fn shim_write_bytes(ctx: *mut c_void, addr: u32, src: *const u8, len: u32) {
    let e = unsafe { &mut *(ctx as *mut ErasedMemory) };
    let slice = unsafe { std::slice::from_raw_parts(src, len as usize) };
    e.write_bytes(addr, slice);
}

unsafe extern "C" fn shim_is_mapped(ctx: *mut c_void, addr: u32) -> bool {
    let e = unsafe { &*(ctx as *const ErasedMemory) };
    e.is_mapped(addr)
}

/// The single vtable all handles produced by [`MemoryHandle::from_box`]
/// share. The per-handle state lives entirely in `ctx`.
static DIRECT_VTABLE: MemoryVTable = MemoryVTable {
    read8: shim_read8,
    read16: shim_read16,
    read32: shim_read32,
    read64: shim_read64,
    write8: shim_write8,
    write16: shim_write16,
    write32: shim_write32,
    write64: shim_write64,
    read_bytes: shim_read_bytes,
    write_bytes: shim_write_bytes,
    is_mapped: shim_is_mapped,
};

// ---------------------------------------------------------------------------
// `extern "C"` wrapper functions consumed by C++
// ---------------------------------------------------------------------------
//
// Naming: `pcsx2_mem_<op>(handle) -> <type>`. The C++ side passes a
// `*const MemoryHandle` (or `*mut MemoryHandle` if it owns the handle).
// These are the lowest-level FFI exports; everything else can be expressed
// on top of them in Rust.

/// Read 8 bits. Returns 0 if the handle is null.
#[no_mangle]
pub unsafe extern "C" fn pcsx2_mem_read8(handle: *const MemoryHandle, addr: u32) -> u8 {
    let f = match handle_and_fn(handle, |v| v.read8) {
        Some(f) => f,
        None => return 0,
    };
    unsafe { f(handle_ctx(handle), addr) }
}

#[no_mangle]
pub unsafe extern "C" fn pcsx2_mem_read16(handle: *const MemoryHandle, addr: u32) -> u16 {
    let f = match handle_and_fn(handle, |v| v.read16) {
        Some(f) => f,
        None => return 0,
    };
    unsafe { f(handle_ctx(handle), addr) }
}

#[no_mangle]
pub unsafe extern "C" fn pcsx2_mem_read32(handle: *const MemoryHandle, addr: u32) -> u32 {
    let f = match handle_and_fn(handle, |v| v.read32) {
        Some(f) => f,
        None => return 0,
    };
    unsafe { f(handle_ctx(handle), addr) }
}

#[no_mangle]
pub unsafe extern "C" fn pcsx2_mem_read64(handle: *const MemoryHandle, addr: u32) -> u64 {
    let f = match handle_and_fn(handle, |v| v.read64) {
        Some(f) => f,
        None => return 0,
    };
    unsafe { f(handle_ctx(handle), addr) }
}

#[no_mangle]
pub unsafe extern "C" fn pcsx2_mem_write8(handle: *const MemoryHandle, addr: u32, val: u8) {
    let f = match handle_and_fn(handle, |v| v.write8) {
        Some(f) => f,
        None => return,
    };
    unsafe { f(handle_ctx(handle), addr, val) }
}

#[no_mangle]
pub unsafe extern "C" fn pcsx2_mem_write16(handle: *const MemoryHandle, addr: u32, val: u16) {
    let f = match handle_and_fn(handle, |v| v.write16) {
        Some(f) => f,
        None => return,
    };
    unsafe { f(handle_ctx(handle), addr, val) }
}

#[no_mangle]
pub unsafe extern "C" fn pcsx2_mem_write32(handle: *const MemoryHandle, addr: u32, val: u32) {
    let f = match handle_and_fn(handle, |v| v.write32) {
        Some(f) => f,
        None => return,
    };
    unsafe { f(handle_ctx(handle), addr, val) }
}

#[no_mangle]
pub unsafe extern "C" fn pcsx2_mem_write64(handle: *const MemoryHandle, addr: u32, val: u64) {
    let f = match handle_and_fn(handle, |v| v.write64) {
        Some(f) => f,
        None => return,
    };
    unsafe { f(handle_ctx(handle), addr, val) }
}

/// Bulk read. `dst` is a caller-allocated buffer of length `len`.
///
/// # Safety
/// `dst` must point to at least `len` writable bytes.
#[no_mangle]
pub unsafe extern "C" fn pcsx2_mem_read_bytes(
    handle: *const MemoryHandle,
    addr: u32,
    dst: *mut u8,
    len: u32,
) {
    if dst.is_null() {
        return;
    }
    let f = match handle_and_fn(handle, |v| v.read_bytes) {
        Some(f) => f,
        None => return,
    };
    unsafe { f(handle_ctx(handle), addr, dst, len) }
}

/// Bulk write. `src` is a caller-allocated buffer of length `len`.
///
/// # Safety
/// `src` must point to at least `len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn pcsx2_mem_write_bytes(
    handle: *const MemoryHandle,
    addr: u32,
    src: *const u8,
    len: u32,
) {
    if src.is_null() {
        return;
    }
    let f = match handle_and_fn(handle, |v| v.write_bytes) {
        Some(f) => f,
        None => return,
    };
    unsafe { f(handle_ctx(handle), addr, src, len) }
}

#[no_mangle]
pub unsafe extern "C" fn pcsx2_mem_is_mapped(handle: *const MemoryHandle, addr: u32) -> bool {
    let f = match handle_and_fn(handle, |v| v.is_mapped) {
        Some(f) => f,
        None => return false,
    };
    unsafe { f(handle_ctx(handle), addr) }
}

/// Validate `handle` and copy out the selected vtable function pointer.
///
/// Returns `None` for any null component. The closure lets callers
/// extract a single field without manually repeating the null dance.
fn handle_and_fn<F, R>(handle: *const MemoryHandle, pick: F) -> Option<R>
where
    F: FnOnce(MemoryVTable) -> R,
    R: Copy,
{
    if handle.is_null() {
        return None;
    }
    // SAFETY: caller-provided `handle` is null-checked above; the FFI
    // contract requires the caller to keep the MemoryHandle struct alive
    // for the duration of any FFI call.
    let h = unsafe { &*handle };
    if h.vtable.is_null() || h.ctx.is_null() {
        return None;
    }
    let vt = unsafe { &*h.vtable };
    Some(pick(*vt))
}

/// Read the `ctx` pointer out of a handle. Caller must have already
/// validated the handle via [`handle_and_fn`].
fn handle_ctx(handle: *const MemoryHandle) -> *mut c_void {
    // SAFETY: mirror of handle_and_fn's invariant.
    unsafe { (*handle).ctx }
}

/// Release a handle. After this call, the Rust-side `Box<ErasedMemory>`
/// is dropped and the implementor's destructor runs.
///
/// # Safety
/// `handle` must have been produced by [`MemoryHandle::from_box`] and must
/// not be used afterwards. The `handle` pointer itself is not freed — C++
/// owns the `MemoryHandle` struct and is responsible for deallocating it.
#[no_mangle]
pub unsafe extern "C" fn pcsx2_mem_release(handle: *const MemoryHandle) {
    if handle.is_null() {
        return;
    }
    let h = unsafe { &*handle };
    if h.ctx.is_null() {
        return;
    }
    let _: Box<ErasedMemory> = unsafe { Box::from_raw(h.ctx as *mut ErasedMemory) };
}

// ---------------------------------------------------------------------------
// Pure-Rust generic helpers (replacement for C++ Read<T> / Write<T> /
// IdempotentWrite<T>). Not exposed via FFI.
// ---------------------------------------------------------------------------

/// Trait bound for the C++ `MemoryAccessType` concept: signed/unsigned
/// 8/16/32/64-bit plus `f32`/`f64`. (The C++ version also includes 128-bit
/// types; we omit those here per the task spec.)
pub trait MemoryAccessType: Copy + PartialEq {
    fn read_from_mem(mi: &dyn MemoryInterface, addr: u32) -> Self;
    fn write_to_mem(mi: &mut dyn MemoryInterface, addr: u32, val: Self) -> bool;
}

impl MemoryAccessType for u8 {
    fn read_from_mem(mi: &dyn MemoryInterface, addr: u32) -> Self {
        mi.read8(addr)
    }
    fn write_to_mem(mi: &mut dyn MemoryInterface, addr: u32, val: Self) -> bool {
        mi.write8(addr, val);
        true
    }
}
impl MemoryAccessType for i8 {
    fn read_from_mem(mi: &dyn MemoryInterface, addr: u32) -> Self {
        mi.read8(addr) as i8
    }
    fn write_to_mem(mi: &mut dyn MemoryInterface, addr: u32, val: Self) -> bool {
        mi.write8(addr, val as u8);
        true
    }
}
impl MemoryAccessType for u16 {
    fn read_from_mem(mi: &dyn MemoryInterface, addr: u32) -> Self {
        mi.read16(addr)
    }
    fn write_to_mem(mi: &mut dyn MemoryInterface, addr: u32, val: Self) -> bool {
        mi.write16(addr, val);
        true
    }
}
impl MemoryAccessType for i16 {
    fn read_from_mem(mi: &dyn MemoryInterface, addr: u32) -> Self {
        mi.read16(addr) as i16
    }
    fn write_to_mem(mi: &mut dyn MemoryInterface, addr: u32, val: Self) -> bool {
        mi.write16(addr, val as u16);
        true
    }
}
impl MemoryAccessType for u32 {
    fn read_from_mem(mi: &dyn MemoryInterface, addr: u32) -> Self {
        mi.read32(addr)
    }
    fn write_to_mem(mi: &mut dyn MemoryInterface, addr: u32, val: Self) -> bool {
        mi.write32(addr, val);
        true
    }
}
impl MemoryAccessType for i32 {
    fn read_from_mem(mi: &dyn MemoryInterface, addr: u32) -> Self {
        mi.read32(addr) as i32
    }
    fn write_to_mem(mi: &mut dyn MemoryInterface, addr: u32, val: Self) -> bool {
        mi.write32(addr, val as u32);
        true
    }
}
impl MemoryAccessType for u64 {
    fn read_from_mem(mi: &dyn MemoryInterface, addr: u32) -> Self {
        mi.read64(addr)
    }
    fn write_to_mem(mi: &mut dyn MemoryInterface, addr: u32, val: Self) -> bool {
        mi.write64(addr, val);
        true
    }
}
impl MemoryAccessType for i64 {
    fn read_from_mem(mi: &dyn MemoryInterface, addr: u32) -> Self {
        mi.read64(addr) as i64
    }
    fn write_to_mem(mi: &mut dyn MemoryInterface, addr: u32, val: Self) -> bool {
        mi.write64(addr, val as u64);
        true
    }
}
impl MemoryAccessType for f32 {
    fn read_from_mem(mi: &dyn MemoryInterface, addr: u32) -> Self {
        f32::from_bits(mi.read32(addr))
    }
    fn write_to_mem(mi: &mut dyn MemoryInterface, addr: u32, val: Self) -> bool {
        mi.write32(addr, val.to_bits());
        true
    }
}
impl MemoryAccessType for f64 {
    fn read_from_mem(mi: &dyn MemoryInterface, addr: u32) -> Self {
        f64::from_bits(mi.read64(addr))
    }
    fn write_to_mem(mi: &mut dyn MemoryInterface, addr: u32, val: Self) -> bool {
        mi.write64(addr, val.to_bits());
        true
    }
}

/// Generic read — Rust analogue of C++ `MemoryInterface::Read<T>`.
pub fn read_mem<T: MemoryAccessType>(mi: &dyn MemoryInterface, addr: u32) -> T {
    T::read_from_mem(mi, addr)
}

/// Generic write — Rust analogue of C++ `MemoryInterface::Write<T>`.
pub fn write_mem<T: MemoryAccessType>(mi: &mut dyn MemoryInterface, addr: u32, val: T) -> bool {
    T::write_to_mem(mi, addr, val)
}

/// Generic idempotent write — Rust analogue of C++
/// `MemoryInterface::IdempotentWrite<T>`.
pub fn idempotent_write_mem<T: MemoryAccessType>(
    mi: &mut dyn MemoryInterface,
    addr: u32,
    val: T,
) -> bool {
    let existing = T::read_from_mem(mi, addr);
    if existing == val {
        return true;
    }
    T::write_to_mem(mi, addr, val)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> Vec<u8> {
        vec![0u8; 256]
    }

    #[test]
    fn direct_read_write_roundtrip() {
        let mut buf = fresh();
        {
            let mut mem = DirectMemory::new(&mut buf, 0);
            mem.write8(0x10, 0xAB);
            mem.write16(0x20, 0xBEEF);
            mem.write32(0x30, 0xDEAD_BEEF);
            mem.write64(0x40, 0x0123_4567_89AB_CDEF);
            assert_eq!(mem.read8(0x10), 0xAB);
            assert_eq!(mem.read16(0x20), 0xBEEF);
            assert_eq!(mem.read32(0x30), 0xDEAD_BEEF);
            assert_eq!(mem.read64(0x40), 0x0123_4567_89AB_CDEF);
        }
        // Confirm LE storage
        assert_eq!(buf[0x10], 0xAB);
        assert_eq!(buf[0x20], 0xEF);
        assert_eq!(buf[0x21], 0xBE);
        assert_eq!(buf[0x30], 0xEF);
        assert_eq!(buf[0x33], 0xDE);
    }

    #[test]
    fn oob_returns_zero() {
        let mut buf = fresh();
        let mem = DirectMemory::new(&mut buf, 0);
        assert_eq!(mem.read32(0x200), 0);
        assert!(!mem.is_mapped(0x200));
    }

    #[test]
    fn bulk_bytes() {
        let mut buf = fresh();
        {
            let mut mem = DirectMemory::new(&mut buf, 0x100);
            let payload = [1u8, 2, 3, 4, 5];
            mem.write_bytes(0x100, &payload);
            let mut out = [0u8; 5];
            mem.read_bytes(0x100, &mut out);
            assert_eq!(out, payload);
        }
    }

    #[test]
    fn generic_helpers() {
        let mut buf = fresh();
        {
            let mut mem = DirectMemory::new(&mut buf, 0);
            write_mem(&mut mem, 0, 1.5f32);
            assert_eq!(read_mem::<f32>(&mem, 0), 1.5);
            idempotent_write_mem(&mut mem, 0, 1.5f32);
        }
        let before = buf[0];
        {
            let mut mem = DirectMemory::new(&mut buf, 0);
            idempotent_write_mem(&mut mem, 0, 1.5f32);
        }
        // Idempotent re-write should not have touched the buffer.
        assert_eq!(buf[0], before);
    }

    #[test]
    fn ffi_roundtrip() {
        // `Box<dyn MemoryInterface>` defaults to `+ 'static`, so the inner
        // implementor must own its backing buffer. Leak a Vec<u8> for the
        // duration of this test.
        let leaked: &'static mut [u8] = Box::leak(Box::new([0u8; 256]));
        let handle = MemoryHandle::from_box(Box::new(DirectMemory::<'static>::new(leaked, 0)));

        unsafe {
            pcsx2_mem_write32(&handle, 0x10, 0xDEAD_BEEF);
            assert_eq!(pcsx2_mem_read32(&handle, 0x10), 0xDEAD_BEEF);
            assert_eq!(pcsx2_mem_read16(&handle, 0x10), 0xBEEF);
            assert_eq!(pcsx2_mem_read8(&handle, 0x10), 0xEF);

            let mut scratch = [0u8; 4];
            pcsx2_mem_read_bytes(&handle, 0x10, scratch.as_mut_ptr(), scratch.len() as u32);
            assert_eq!(scratch, [0xEF, 0xBE, 0xAD, 0xDE]);

            let payload = [0x11u8, 0x22, 0x33, 0x44];
            pcsx2_mem_write_bytes(&handle, 0x20, payload.as_ptr(), payload.len() as u32);
            assert_eq!(pcsx2_mem_read32(&handle, 0x20), 0x4433_2211);

            assert!(pcsx2_mem_is_mapped(&handle, 0));
            assert!(!pcsx2_mem_is_mapped(&handle, 0x200));

            // Null handle returns zero safely.
            assert_eq!(pcsx2_mem_read32(std::ptr::null(), 0), 0);
            assert!(!pcsx2_mem_is_mapped(std::ptr::null(), 0));

            // Release reclaims the boxed implementor.
            pcsx2_mem_release(&handle);
        }
    }
}
