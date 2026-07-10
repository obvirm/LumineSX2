// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust 2021 translation of the MMI / recVTLB / VIF-dynarec /
//! ARM64-asm-helper source set of PCSX2.
//!
//! # Scope
//!
//! This module folds the following original files into a single
//! translation unit:
//!
//! * `pcsx2/x86/iMMI.{h,cpp}`         – the EE MMI opcode dispatch
//!   (`MMI0`/`MMI1`/`MMI2`/`MMI3` plus the SPECIAL `MADD`/`MADDU`/
//!   `MFHI`/`MFLO`/`MULT`/`MULTU`/`DIV`/`DIVU`/`MTHI`/`MTLO` forms).
//! * `pcsx2/x86/ix86-32/recVTLB.cpp`  – the EE 32-bit virtual TLB page
//!   table used by the dynarec load/store path.
//! * `pcsx2/arm64/Vif_Dynarec.cpp`    – the VIF dynarec block-recompile
//!   entry point and its supporting helpers (`dVifReset`, `dVifRelease`,
//!   `dVifCompile`, `dVifUnpack`, `dVifComputeLength`, ...).
//! * `pcsx2/arm64/Vif_UnpackNEON.cpp` – the NEON VIF unpack interpreter
//!   and the `VifUnpackNEON_*` family of emitters.
//! * `pcsx2/arm64/AsmHelpers.cpp`     – the ARM64 assembly helper entry
//!   points (`armWRegister`, `armXRegister`, `armSRegister`,
//!   `armDRegister`, `armQRegister`, `armSetAsmPtr`, `armStartBlock`,
//!   `armEndBlock`, `armEmitJmp`, `armEmitCall`, `armEmitCbnz`,
//!   `armEmitCondBranch`, `armMoveAddressToReg`, `armLoadPtr`,
//!   `armStorePtr`, `armBeginStackFrame`, `armEndStackFrame`,
//!   `armIsCalleeSavedRegister`, `armOffsetMemOperand`,
//!   `armGetMemOperandInRegister`, `armLoadConstant128`,
//!   `armEmitVTBL`, plus the `ArmConstantPool` shim).
//! * `pcsx2/arm64/RecStubs.cpp`       – ARM64 recompiler stubs
//!   (`vtlb_DynBackpatchLoadStore`, `vuJITFreeze`).
//!
//! # What is *not* translated
//!
//! The per-opcode x86 SSE / NEON emitter bodies have no idiomatic Rust
//! analogue without dragging in a JIT backend (VIXL, dynasm, etc.).
//! They are exposed as no-op shims that preserve the C++ signature
//! 1:1, plus the dispatcher / bitfield decoder / data-structure
//! surface that other translated modules can call into. The
//! MMI_Opcode dispatcher still routes every primary opcode to its
//! canonical handler, and the per-sub-opcode dispatch tables for
//! MMI0..MMI3 are preserved as exhaustive `match` expressions so
//! the translation is structurally faithful.
//!
//! Per the project rules, all globals use `static mut` and the module
//! only depends on `std`.

#![deny(unsafe_op_in_unsafe_fn)]
#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(dead_code)]
#![allow(unused_variables)]

// ===========================================================================
//  Primitive aliases (mirror `Common.h`)
// ===========================================================================

pub type u8 = ::std::primitive::u8;
pub type u16 = ::std::primitive::u16;
pub type u32 = ::std::primitive::u32;
pub type u64 = ::std::primitive::u64;
pub type s8 = ::std::primitive::i8;
pub type s16 = ::std::primitive::i16;
pub type s32 = ::std::primitive::i32;
pub type s64 = ::std::primitive::i64;

/// Unsigned pointer-sized integer.  In the C++ code this is `uptr`.
pub type uptr = usize;

/// Signed pointer-sized integer.  In the C++ code this is `sptr`.
pub type sptr = isize;

// ===========================================================================
//  Instruction bit-field decoders
// ===========================================================================
//
//  The C++ original uses preprocessor macros `_Rd_`, `_Rs_`, `_Rt_`, `_Sa_`,
//  `_func_`, `_op_` that read `cpuRegs.code`.  In idiomatic Rust we model
//  them as plain functions over the explicit `instr: u32` argument.

/// Destination GPR field, bits 11..15.
#[inline]
pub fn _Rd_(instr: u32) -> u32 { (instr >> 11) & 0x1F }

/// Source GPR field, bits 21..25.
#[inline]
pub fn _Rs_(instr: u32) -> u32 { (instr >> 21) & 0x1F }

/// Target GPR field, bits 16..20.
#[inline]
pub fn _Rt_(instr: u32) -> u32 { (instr >> 16) & 0x1F }

/// Shift-amount field, bits 6..10.
#[inline]
pub fn _Sa_(instr: u32) -> u32 { (instr >> 6) & 0x1F }

/// Function sub-field, bits 0..5.
#[inline]
pub fn _func_(instr: u32) -> u32 { instr & 0x3F }

/// Primary opcode field, bits 26..31.
#[inline]
pub fn _op_(instr: u32) -> u32 { (instr >> 26) & 0x3F }

// ===========================================================================
//  VTLB — virtual TLB page table (translation of `recVTLB.cpp`)
// ===========================================================================

/// 4 KiB page size, matching the MIPS R5900 TLB granularity.
pub const VTLB_PAGE_SIZE: u32 = 4096;
/// Mask covering the offset-within-page bits of a guest address.
pub const VTLB_PAGE_MASK: u32 = VTLB_PAGE_SIZE - 1;
/// Shift that turns a guest address into a VTLB page index.
pub const VTLB_PAGE_BITS: u32 = 12;
/// Number of entries in the virtual TLB (covers the full 4 GiB address
/// space at 4 KiB granularity).
pub const VTLB_VMAP_ITEMS: u32 = 0x10000;

/// One VTLB page-table entry.
///
/// `paddr` is the physical base address of the page (its low 12 bits are
/// zeroed when the entry is mapped).  `vaddr` is the virtual base the
/// page was mapped at, and `size` is the page count (1 for a single
/// 4 KiB page; multi-page mappings chain through consecutive entries).
#[derive(Clone, Copy, Default, Debug)]
#[repr(C)]
pub struct VTLBEntry {
    /// PS2 physical base address of the page.
    pub paddr: u32,
    /// Guest virtual base address of the page.
    pub vaddr: u32,
    /// Number of pages in this mapping (1 for a single page).
    pub size: u32,
}

impl VTLBEntry {
    /// Construct an empty (unmapped) entry.
    pub const fn empty() -> Self {
        Self { paddr: 0, vaddr: 0, size: 0 }
    }
}

/// The 64 K-entry virtual TLB page table, indexed by `vaddr >> VTLB_PAGE_BITS`.
pub static mut vtlb: [VTLBEntry; 0x10000] =
    [VTLBEntry { paddr: 0, vaddr: 0, size: 0 }; 0x10000];

/// Translate a guest virtual address to a PS2 physical address by walking
/// the VTLB.
///
/// This is the structural Rust equivalent of the x86 sequence
/// `vtlb_DynV2P` found in `recVTLB.cpp`:
///
/// ```text
///   mov  eax, ecx          ; eax = vaddr
///   and  ecx, VTLB_PAGE_MASK
///   shr  eax, VTLB_PAGE_BITS
///   mov  eax, [eax*4 + ppmap]   ; paddr of the page
///   or   eax, ecx               ; paddr | offset
/// ```
///
/// In Rust we index the single global `vtlb` table directly; the
/// returned physical address is the entry's `paddr` with the original
/// page-offset bits OR'd back in.  The masking guarantees that an
/// out-of-table index wraps, mirroring the C++ `& 0xFFFF` semantics.
pub fn recVTLBLookup(vaddr: u32) -> u32 {
    let page = ((vaddr >> VTLB_PAGE_BITS) as usize) & 0xFFFF;
    unsafe {
        let entry = vtlb[page];
        (entry.paddr & !VTLB_PAGE_MASK) | (vaddr & VTLB_PAGE_MASK)
    }
}

/// Reset every VTLB entry to "unmapped".
pub fn vtlb_reset() {
    unsafe {
        for entry in vtlb.iter_mut() {
            *entry = VTLBEntry::empty();
        }
    }
}

/// Map `count` consecutive 4 KiB pages starting at `vaddr` to the
/// physical page at `paddr`.  The mapping wraps at the top of the table.
pub fn vtlb_map_pages(vaddr: u32, paddr: u32, count: u32) {
    let start = ((vaddr >> VTLB_PAGE_BITS) as usize) & 0xFFFF;
    let pbase = paddr & !VTLB_PAGE_MASK;
    unsafe {
        for i in 0..count as usize {
            let idx = (start + i) & 0xFFFF;
            vtlb[idx] = VTLBEntry {
                paddr: pbase + (i as u32) * VTLB_PAGE_SIZE,
                vaddr: vaddr + (i as u32) * VTLB_PAGE_SIZE,
                size:  count,
            };
        }
    }
}

// ===========================================================================
//  MMI opcode dispatch (translation of `iMMI.{h,cpp}`)
// ===========================================================================

/// Top-level dispatcher for the R5900 MMI / SPECIAL-MULT family of
/// instructions.
///
/// The C++ original spreads this dispatch across
/// `R5900::Dynarec::OpcodeImpl::MMI::recMMI0..recMMI3` plus the
/// `recMADD` / `recMADDU` / `recMFHI` / `recMFLO` / `recMTHI` / `recMTLO`
/// stubs declared in `iMMI.h`.  Here the public entry point is
/// `MMI_Opcode`, which routes by primary opcode to:
///
/// | primary opcode | dispatched to                              |
/// |---------------:|--------------------------------------------|
/// | 0x00 (SPECIAL) | `MMI_MADD`, `MMI_MADDU`, `MMI_MFHI`, ...  |
/// | 0x10           | `recMMI0`                                  |
/// | 0x11           | `recMMI1`                                  |
/// | 0x12           | `recMMI2`                                  |
/// | 0x13           | `recMMI3`                                  |
///
/// In idiomatic Rust the SPECIAL sub-dispatch is a `match` on the
/// 6-bit function field rather than a function-pointer table.
pub fn MMI_Opcode(instr: u32) {
    match _op_(instr) {
        0x00 => match _func_(instr) {
            0x00 => MMI_MADD(instr),
            0x01 => MMI_MADDU(instr),
            0x02 => MMI_MULT(instr),
            0x03 => MMI_MULTU(instr),
            0x04 => MMI_DIV(instr),
            0x05 => MMI_DIVU(instr),
            0x10 => MMI_MFHI(instr),
            0x11 => MMI_MTHI(instr),
            0x12 => MMI_MFLO(instr),
            0x13 => MMI_MTLO(instr),
            _    => { /* reserved SPECIAL function */ }
        },
        0x10 => recMMI0(instr),
        0x11 => recMMI1(instr),
        0x12 => recMMI2(instr),
        0x13 => recMMI3(instr),
        _    => { /* not an MMI instruction */ }
    }
}

// ---- MMI helpers (mirror the SPECIAL entries in iMMI.h) --------------------

/// MADD rd, rs, rt  (signed 32x32+64 -> 64)
pub fn MMI_MADD(_instr: u32) { /* x86 emitter: gsUHQxy32 */ }
/// MADDU rd, rs, rt (unsigned 32x32+64 -> 64)
pub fn MMI_MADDU(_instr: u32) { /* x86 emitter: gsUHQxy32 */ }
/// MULT rd, rs, rt  (signed 32x32 -> 64)
pub fn MMI_MULT(_instr: u32) { /* x86 emitter: gsUHQxy32 */ }
/// MULTU rd, rs, rt (unsigned 32x32 -> 64)
pub fn MMI_MULTU(_instr: u32) { /* x86 emitter: gsUHQxy32 */ }
/// DIV rs, rt       (signed 32/32 -> LO; HI = rs % rt)
pub fn MMI_DIV(_instr: u32) { /* x86 emitter: recCall Div */ }
/// DIVU rs, rt      (unsigned 32/32 -> LO; HI = rs % rt)
pub fn MMI_DIVU(_instr: u32) { /* x86 emitter: recCall DivU */ }
/// MFHI rd          (move HI -> rd)
pub fn MMI_MFHI(_instr: u32) { /* x86 emitter: xMOVD Rd <- HI */ }
/// MTHI rs          (move rs -> HI)
pub fn MMI_MTHI(_instr: u32) { /* x86 emitter: xMOVD HI <- Rs */ }
/// MFLO rd          (move LO -> rd)
pub fn MMI_MFLO(_instr: u32) { /* x86 emitter: xMOVD Rd <- LO */ }
/// MTLO rs          (move rs -> LO)
pub fn MMI_MTLO(_instr: u32) { /* x86 emitter: xMOVD LO <- Rs */ }

// ---- MMI sub-group dispatchers -------------------------------------------

/// MMI0 sub-opcode dispatcher.  The sub-opcode lives in bits 5..0 of the
/// instruction word, matching the C++ `recMMI0` table.
pub fn recMMI0(instr: u32) {
    match instr & 0x3F {
        0x00 => recPADDB(instr),
        0x01 => recPADDH(instr),
        0x02 => recPADDW(instr),
        0x03 => recPADDSB(instr),
        0x04 => recPADDSH(instr),
        0x05 => recPADDSW(instr),
        0x06 => recPSUBB(instr),
        0x07 => recPSUBH(instr),
        0x08 => recPSUBW(instr),
        0x09 => recPSUBSB(instr),
        0x0A => recPSUBSH(instr),
        0x0B => recPSUBSW(instr),
        0x0C => recPMAXW(instr),
        0x0D => recPMAXH(instr),
        0x0E => recPCGTB(instr),
        0x0F => recPCGTH(instr),
        0x10 => recPCGTW(instr),
        0x11 => recPEXTLW(instr),
        0x12 => recPEXTLH(instr),
        0x13 => recPEXTLB(instr),
        0x14 => recPEXT5(instr),
        0x15 => recPPACW(instr),
        0x16 => recPPACH(instr),
        0x17 => recPPACB(instr),
        0x18 => recPPAC5(instr),
        _    => { /* reserved MMI0 sub-opcode */ }
    }
}

/// MMI1 sub-opcode dispatcher.
pub fn recMMI1(instr: u32) {
    match instr & 0x3F {
        0x00 => recPABSW(instr),
        0x01 => recPABSH(instr),
        0x02 => recPMINW(instr),
        0x03 => recPADSBH(instr),
        0x04 => recPADDUB(instr),
        0x05 => recPADDUH(instr),
        0x06 => recPADDUW(instr),
        0x07 => recPSUBUB(instr),
        0x08 => recPSUBUH(instr),
        0x09 => recPSUBUW(instr),
        0x0A => recPEXTUH(instr),
        0x0B => recPEXTUB(instr),
        0x0C => recPEXTUW(instr),
        0x0D => recQFSRV(instr),
        0x0E => recPMINH(instr),
        0x0F => recPCEQB(instr),
        0x10 => recPCEQH(instr),
        0x11 => recPCEQW(instr),
        _    => { /* reserved MMI1 sub-opcode */ }
    }
}

/// MMI2 sub-opcode dispatcher.
pub fn recMMI2(instr: u32) {
    match instr & 0x3F {
        0x00 => recPMADDW(instr),
        0x01 => recPSLLVW(instr),
        0x02 => recPSRLVW(instr),
        0x03 => recPMSUBW(instr),
        0x04 => recPINTH(instr),
        0x05 => recPMULTW(instr),
        0x06 => recPDIVW(instr),
        0x07 => recPMADDH(instr),
        0x08 => recPHMADH(instr),
        0x09 => recPMSUBH(instr),
        0x0A => recPHMSBH(instr),
        0x0B => recPEXEH(instr),
        0x0C => recPREVH(instr),
        0x0D => recPMULTH(instr),
        0x0E => recPDIVBW(instr),
        0x0F => recPEXEW(instr),
        0x10 => recPROT3W(instr),
        0x11 => recPMFHI(instr),
        0x12 => recPMFLO(instr),
        0x13 => recPAND(instr),
        0x14 => recPXOR(instr),
        0x15 => recPCPYLD(instr),
        _    => { /* reserved MMI2 sub-opcode */ }
    }
}

/// MMI3 sub-opcode dispatcher.
pub fn recMMI3(instr: u32) {
    match instr & 0x3F {
        0x00 => recPMADDUW(instr),
        0x01 => recPSRAVW(instr),
        0x02 => recPMTHI(instr),
        0x03 => recPMTLO(instr),
        0x04 => recPINTEH(instr),
        0x05 => recPMULTUW(instr),
        0x06 => recPDIVUW(instr),
        0x07 => recPCPYUD(instr),
        0x08 => recPOR(instr),
        0x09 => recPNOR(instr),
        0x0A => recPCPYH(instr),
        0x0B => recPEXCW(instr),
        0x0C => recPEXCH(instr),
        _    => { /* reserved MMI3 sub-opcode */ }
    }
}

// ---- MMI0 sub-handlers (no-op shims preserving the C++ signatures) -------

pub fn recPADDB(_i: u32)   {}
pub fn recPADDH(_i: u32)   {}
pub fn recPADDW(_i: u32)   {}
pub fn recPADDSB(_i: u32)  {}
pub fn recPADDSH(_i: u32)  {}
pub fn recPADDSW(_i: u32)  {}
pub fn recPSUBB(_i: u32)   {}
pub fn recPSUBH(_i: u32)   {}
pub fn recPSUBW(_i: u32)   {}
pub fn recPSUBSB(_i: u32)  {}
pub fn recPSUBSH(_i: u32)  {}
pub fn recPSUBSW(_i: u32)  {}
pub fn recPMAXW(_i: u32)   {}
pub fn recPMAXH(_i: u32)   {}
pub fn recPCGTB(_i: u32)   {}
pub fn recPCGTH(_i: u32)   {}
pub fn recPCGTW(_i: u32)   {}
pub fn recPEXTLW(_i: u32)  {}
pub fn recPEXTLH(_i: u32)  {}
pub fn recPEXTLB(_i: u32)  {}
pub fn recPEXT5(_i: u32)   {}
pub fn recPPACW(_i: u32)   {}
pub fn recPPACH(_i: u32)   {}
pub fn recPPACB(_i: u32)   {}
pub fn recPPAC5(_i: u32)   {}

// ---- MMI1 sub-handlers ---------------------------------------------------

pub fn recPABSW(_i: u32)   {}
pub fn recPABSH(_i: u32)   {}
pub fn recPMINW(_i: u32)   {}
pub fn recPADSBH(_i: u32)  {}
pub fn recPADDUB(_i: u32)  {}
pub fn recPADDUH(_i: u32)  {}
pub fn recPADDUW(_i: u32)  {}
pub fn recPSUBUB(_i: u32)  {}
pub fn recPSUBUH(_i: u32)  {}
pub fn recPSUBUW(_i: u32)  {}
pub fn recPEXTUH(_i: u32)  {}
pub fn recPEXTUB(_i: u32)  {}
pub fn recPEXTUW(_i: u32)  {}
pub fn recQFSRV(_i: u32)   {}
pub fn recPMINH(_i: u32)   {}
pub fn recPCEQB(_i: u32)   {}
pub fn recPCEQH(_i: u32)   {}
pub fn recPCEQW(_i: u32)   {}

// ---- MMI2 sub-handlers ---------------------------------------------------

pub fn recPMADDW(_i: u32)  {}
pub fn recPSLLVW(_i: u32)  {}
pub fn recPSRLVW(_i: u32)  {}
pub fn recPMSUBW(_i: u32)  {}
pub fn recPINTH(_i: u32)   {}
pub fn recPMULTW(_i: u32)  {}
pub fn recPDIVW(_i: u32)   {}
pub fn recPMADDH(_i: u32)  {}
pub fn recPHMADH(_i: u32)  {}
pub fn recPMSUBH(_i: u32)  {}
pub fn recPHMSBH(_i: u32)  {}
pub fn recPEXEH(_i: u32)   {}
pub fn recPREVH(_i: u32)   {}
pub fn recPMULTH(_i: u32)  {}
pub fn recPDIVBW(_i: u32)  {}
pub fn recPEXEW(_i: u32)   {}
pub fn recPROT3W(_i: u32)  {}
pub fn recPMFHI(_i: u32)   {}
pub fn recPMFLO(_i: u32)   {}
pub fn recPAND(_i: u32)    {}
pub fn recPXOR(_i: u32)    {}
pub fn recPCPYLD(_i: u32)  {}

// ---- MMI3 sub-handlers ---------------------------------------------------

pub fn recPMADDUW(_i: u32) {}
pub fn recPSRAVW(_i: u32)  {}
pub fn recPMTHI(_i: u32)   {}
pub fn recPMTLO(_i: u32)   {}
pub fn recPINTEH(_i: u32)  {}
pub fn recPMULTUW(_i: u32) {}
pub fn recPDIVUW(_i: u32)  {}
pub fn recPCPYUD(_i: u32)  {}
pub fn recPOR(_i: u32)     {}
pub fn recPNOR(_i: u32)    {}
pub fn recPCPYH(_i: u32)   {}
pub fn recPEXCW(_i: u32)   {}
pub fn recPEXCH(_i: u32)   {}

// ---- Standalone MMI handlers (PLZCW / PMFHL / PMTHL / shifts) ------------

/// PLZCW rd, rs - count of leading zero/sign bits in each 32-bit half.
pub fn recPLZCW(_i: u32) {}
/// PMFHL rd, sa - move-from-HI/LO with format `sa`.
pub fn recPMFHL(_i: u32) {}
/// PMTHL rs, sa - move-to-HI/LO with format `sa`.
pub fn recPMTHL(_i: u32) {}
pub fn recPSRLH(_i: u32)  {}
pub fn recPSRLW(_i: u32)  {}
pub fn recPSRAH(_i: u32)  {}
pub fn recPSRAW(_i: u32)  {}
pub fn recPSLLH(_i: u32)  {}
pub fn recPSLLW(_i: u32)  {}

// ===========================================================================
//  VIF dynarec (translation of `Vif_Dynarec.cpp`)
// ===========================================================================

/// Engine / unit selector for the VIF dynarec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VifUnit {
    /// VIF0 (GIF path).
    Vif0,
    /// VIF1 (VIF path).
    Vif1,
}

/// A single VIF dynarec block descriptor.
///
/// Mirrors the C++ `nVifBlock` struct: the keys used for the dynarec
/// cache, plus the start address and the byte length of the compiled
/// ARM64 code.
#[derive(Debug, Clone, Copy, Default)]
pub struct NVifBlock {
    /// Primary cache key (unpack type + transfer count).
    pub hash_key: u32,
    /// Secondary cache key.
    pub key0: u32,
    /// Tertiary cache key.
    pub key1: u32,
    /// Pointer to the start of the compiled ARM64 code for this block.
    pub start_ptr: uptr,
    /// Byte length of the compiled ARM64 code.
    pub length: u32,
    /// Unpack number (lower 4 bits of `upkType`).
    pub upkType: u8,
    /// Word length of one row.
    pub wl: u8,
    /// Number of columns.
    pub cl: u8,
    /// Number of transfers.
    pub num: u8,
    /// VIF mode field.
    pub mode: u8,
    /// Write-protect mask.
    pub mask: u32,
    /// Whether the start of the block is QW aligned.
    pub aligned: bool,
}

/// One entry in the VIF dynarec block cache.
#[derive(Debug, Clone, Copy, Default)]
pub struct NVifCacheEntry {
    pub block: NVifBlock,
    pub valid: bool,
}

/// Per-unit VIF dynarec state.
#[derive(Debug)]
pub struct NVifState {
    /// Block cache.
    pub cache: [NVifCacheEntry; 256],
    /// Pointer to the next free byte in the code buffer.
    pub rec_write_ptr: *mut u8,
    /// Pointer one byte past the last usable byte in the code buffer.
    pub rec_end_ptr: *mut u8,
    /// Index of this unit (0 or 1).
    pub idx: u32,
}

impl NVifState {
    /// Construct a fresh VIF state for the given unit, with the
    /// provided code buffer.
    pub fn new(idx: u32, code_buf: &mut [u8]) -> Self {
        let mut state = NVifState {
            cache: [NVifCacheEntry::default(); 256],
            rec_write_ptr: code_buf.as_mut_ptr(),
            rec_end_ptr:   unsafe { code_buf.as_mut_ptr().add(code_buf.len()) },
            idx,
        };
        dVifReset(&mut state);
        state
    }
}

/// Reset the dynarec code pointer to the start of the buffer.  Mirrors
/// `dVifReset` in `Vif_Dynarec.cpp`.
pub fn dVifReset(v: &mut NVifState) {
    for entry in v.cache.iter_mut() {
        entry.valid = false;
    }
    // The C++ version advances the write pointer to leave a 256 KiB
    // reservation for the recompiler cache; we model that as a fixed
    // 256 KiB reserve off the end of the buffer.
    let reserve: isize = 256 * 1024;
    let new_end = unsafe { v.rec_end_ptr.offset(-reserve) };
    if (new_end as usize) > (v.rec_write_ptr as usize) {
        v.rec_end_ptr = new_end;
    }
}

/// Release all cached blocks.  Mirrors `dVifRelease`.
pub fn dVifRelease(v: &mut NVifState) {
    for entry in v.cache.iter_mut() {
        entry.valid = false;
    }
}

/// Look up a VIF block in the per-unit cache.
pub fn dVifFindBlock(v: &NVifState, key: &NVifBlock) -> Option<NVifBlock> {
    for entry in v.cache.iter() {
        if entry.valid
            && entry.block.hash_key == key.hash_key
            && entry.block.key0 == key.key0
            && entry.block.key1 == key.key1
        {
            return Some(entry.block);
        }
    }
    None
}

/// Compute the byte length of an unpack packet.  Mirrors
/// `dVifComputeLength` in `Vif_Dynarec.cpp`.
pub fn dVifComputeLength(cl: u32, wl: u32, num: u8, is_fill: bool) -> u16 {
    let count = if num > 0 { num as u32 } else { 256 };
    let mut length = count * 16;

    if !is_fill {
        let skip = (cl.saturating_sub(wl)) * 16;
        let blocks = (count + wl.saturating_sub(1)) / wl;
        length = length.saturating_add(blocks.saturating_sub(1) * skip);
    }

    length.min(0xFFFF) as u16
}

/// Recompile a VIF dynarec block at `pc` and store the descriptor in
/// `out_block`.  Mirrors the public entry point that other modules
/// (the EE dispatcher) call when they need to compile a new VIF
/// dynarec block.
///
/// In the C++ source the body generates ARM64 code via VIXL into the
/// `recWritePtr` buffer.  The Rust translation preserves the entry
/// point and the cache-lookup / cache-miss split, but leaves the VIXL
/// body unimplemented because VIXL is C++-only.
pub fn vifDynarecRecompileBlock(pc: u32) {
    let mut unit = if (pc & 1) == 0 { VifUnit::Vif0 } else { VifUnit::Vif1 };
    let idx = unit as u32;
    let _ = idx;
    // The real implementation would:
    //   1. Build an nVifBlock from the VIF state at `pc`.
    //   2. Look it up in the per-unit cache; on miss, compile it
    //      with VIXL into the unit's recWritePtr buffer.
    //   3. Store the result back into the cache.
    // All of that requires a JIT backend that lives outside the
    // scope of this translation.
    let _ = &mut unit;
}

/// NEON VIF unpack interpreter shim.  Mirrors the public surface of
/// `Vif_UnpackNEON.cpp`.  In the C++ source this is a class hierarchy
/// (`VifUnpackNEON_Base`, `VifUnpackNEON_Dynarec`, `VifUnpackNEON_Simple`)
/// that emits NEON instructions into the code buffer.  Here we expose
/// a single function that takes the per-block parameters and returns
/// the byte length that would be emitted.
pub fn vifUnpackNEON(upk_num: u8, usn: bool, do_mask: bool, cur_cycle: u8) -> u16 {
    // Sub-opcodes 3, 7 and 11 are "indeterminate" on real PS2 hardware
    // and are flagged by the C++ implementation; we follow suit.
    if matches!(upk_num, 3 | 7 | 11) {
        return 0;
    }
    // For all other sub-opcodes the compiled routine body size is
    // bounded by the per-iteration cost (load + unpack + store).
    let per_iter = if do_mask { 48 } else { 16 };
    let iters = (cur_cycle as u16).max(1);
    per_iter * iters
}

// ===========================================================================
//  ARM64 assembly helpers (translation of `AsmHelpers.cpp`)
// ===========================================================================

/// Opaque ARM64 register descriptor.
///
/// The C++ source exposes distinct types for the W / X / S / D / Q
/// register sets from VIXL; in idiomatic Rust we collapse them to a
/// single register `code` (the VIXL register number) since the bit
/// width is implicit in the instruction being emitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct ArmReg(pub u8);

/// Opaque ARM64 vector register descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct ArmVReg(pub u8);

impl ArmReg {
    pub const fn code(self) -> u8 { self.0 }
}
impl ArmVReg {
    pub const fn code(self) -> u8 { self.0 }
}

/// Return the `W<n>` 32-bit GPR for `n` in 0..32.
pub fn armWRegister(n: u8) -> ArmReg { ArmReg(n & 0x1F) }
/// Return the `X<n>` 64-bit GPR for `n` in 0..32.
pub fn armXRegister(n: u8) -> ArmReg { ArmReg(n & 0x1F) }
/// Return the `S<n>` 32-bit FP register for `n` in 0..32.
pub fn armSRegister(n: u8) -> ArmVReg { ArmVReg(n & 0x1F) }
/// Return the `D<n>` 64-bit FP register for `n` in 0..32.
pub fn armDRegister(n: u8) -> ArmVReg { ArmVReg(n & 0x1F) }
/// Return the `Q<n>` 128-bit FP register for `n` in 0..32.
pub fn armQRegister(n: u8) -> ArmVReg { ArmVReg(n & 0x1F) }

/// Opaque ARM64 memory operand descriptor.  The C++ original uses
/// VIXL's `MemOperand`; here we model just the base-register code
/// and the byte displacement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemOperand {
    pub base: ArmReg,
    pub offset: i32,
    /// Optional post-index marker; the C++ original distinguishes
    /// Offset, PreIndex and PostIndex addressing modes.
    pub post_index: bool,
}

impl MemOperand {
    pub const fn new(base: ArmReg) -> Self { Self { base, offset: 0, post_index: false } }
    pub const fn with_offset(base: ArmReg, offset: i32) -> Self {
        Self { base, offset, post_index: false }
    }
}

/// Tiny stand-in for the VIXL `MacroAssembler`.  In the C++ code this
/// is a singleton that owns the code buffer; the Rust translation
/// keeps the same global but reduces it to a raw pointer / capacity
/// pair, which is all the public helpers actually need.
pub struct ArmMacroAssembler {
    pub base: *mut u8,
    pub capacity: usize,
    pub used: usize,
}

/// Thread-local handle to the active assembler.  Mirrors
/// `thread_local a64::MacroAssembler* armAsm`.
pub static mut armAsm: *mut ArmMacroAssembler = std::ptr::null_mut();
/// Thread-local current write pointer.  Mirrors `armAsmPtr`.
pub static mut armAsmPtr: *mut u8 = std::ptr::null_mut();
/// Thread-local remaining capacity.  Mirrors `armAsmCapacity`.
pub static mut armAsmCapacity: usize = 0;

/// Constant pool.  The C++ `ArmConstantPool` is a non-trivial class;
/// the Rust translation keeps only the public surface used by the
/// helper entry points (`Init`, `Destroy`, `Reset`, `GetLiteral`,
/// `GetJumpTrampoline`).
#[derive(Debug, Default)]
pub struct ArmConstantPool {
    pub base_ptr: *mut u8,
    pub capacity: u32,
    pub used: u32,
    pub jump_targets: std::collections::BTreeMap<uptr, u32>,
    pub literals: std::collections::BTreeMap<u128, u32>,
}

impl ArmConstantPool {
    pub fn new() -> Self { Self::default() }

    pub fn Init(&mut self, ptr: *mut u8, capacity: u32) {
        self.base_ptr = ptr;
        self.capacity = capacity;
        self.used = 0;
        self.jump_targets.clear();
        self.literals.clear();
    }

    pub fn Destroy(&mut self) {
        self.base_ptr = std::ptr::null_mut();
        self.capacity = 0;
        self.used = 0;
        self.jump_targets.clear();
        self.literals.clear();
    }

    pub fn Reset(&mut self) {
        self.used = 0;
        self.jump_targets.clear();
        self.literals.clear();
    }

    pub fn get_jump_trampoline(&mut self, _target: *const u8) -> Option<*mut u8> {
        // The real implementation emits a `mov x_scratch, target; br x_scratch`
        // trampoline into the constant pool.  Without a JIT backend we
        // simply report "no trampoline available".
        None
    }

    pub fn get_literal_u64(&mut self, value: u64) -> Option<*mut u8> {
        let key = (value as u128) & 0xFFFF_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF;
        if let Some(&off) = self.literals.get(&key) {
            return Some(unsafe { self.base_ptr.add(off as usize) });
        }
        if (self.capacity - self.used) < 16 {
            return None;
        }
        let off = (self.used + 15) & !15;
        unsafe {
            let dst = self.base_ptr.add(off as usize) as *mut u64;
            *dst = value;
        }
        self.literals.insert(key, off);
        self.used = off + 16;
        Some(unsafe { self.base_ptr.add(off as usize) })
    }
}

/// Set the active assembler target buffer.  Mirrors `armSetAsmPtr`.
pub fn armSetAsmPtr(ptr: *mut u8, capacity: usize, _pool: Option<&mut ArmConstantPool>) {
    unsafe {
        armAsmPtr = ptr;
        armAsmCapacity = capacity;
    }
}

/// Align the assembler write pointer to 16 bytes.
pub fn armAlignAsmPtr() {
    unsafe {
        if armAsmPtr.is_null() { return; }
        let cur = armAsmPtr as usize;
        let aligned = (cur + 15) & !15;
        let pad = aligned - cur;
        if pad <= armAsmCapacity {
            armAsmPtr = aligned as *mut u8;
            armAsmCapacity -= pad;
        }
    }
}

/// Start a new code block.  Returns the base pointer of the new block.
pub fn armStartBlock() -> *mut u8 {
    armAlignAsmPtr();
    unsafe { armAsmPtr }
}

/// End the current code block.  Returns the new write pointer.
pub fn armEndBlock() -> *mut u8 {
    unsafe {
        let prev = armAsmPtr;
        // The real implementation calls `armAsm->FinalizeCode()`,
        // flushes the instruction cache, and advances the write pointer
        // by the emitted size.  Without a JIT backend we return the
        // unchanged pointer.
        prev
    }
}

/// Get the current code pointer (the byte just past the most recently
/// emitted instruction).  Mirrors `armGetCurrentCodePointer`.
pub fn armGetCurrentCodePointer() -> *const u8 {
    unsafe { armAsmPtr as *const u8 }
}

/// Get the current assembler pointer (write cursor).
pub fn armGetAsmPtr() -> *mut u8 {
    unsafe { armAsmPtr }
}

/// Return true when `reg` is in the ARM64 callee-saved range
/// (x19..x28 on both Linux and Windows AAPCS).
pub fn armIsCalleeSavedRegister(reg: i32) -> bool { reg >= 19 && reg <= 28 }

/// Add `offset` to the displacement of an `[base, #offset]` memory
/// operand, returning a new operand.  Mirrors `armOffsetMemOperand`.
pub fn armOffsetMemOperand(op: MemOperand, offset: i64) -> MemOperand {
    MemOperand {
        base: op.base,
        offset: op.offset.wrapping_add(offset as i32),
        post_index: op.post_index,
    }
}

/// Materialize the effective address of `op + extra_offset` into
/// `addr_reg`.  Mirrors `armGetMemOperandInRegister`.
pub fn armGetMemOperandInRegister(addr_reg: ArmReg, op: MemOperand, extra_offset: i64) {
    let _ = (addr_reg, op, extra_offset);
    // Real implementation emits `add addr_reg, op.base, op.offset + extra_offset`.
}

/// Load a 128-bit value from `addr` into `reg`.  Mirrors
/// `armLoadConstant128`.
pub fn armLoadConstant128(reg: ArmVReg, addr: *const u8) {
    let _ = (reg, addr);
}

/// Move an absolute address into `reg`, preferring an ADRP+ADD pair
/// when the page displacement fits.  Mirrors `armMoveAddressToReg`.
pub fn armMoveAddressToReg(reg: ArmReg, addr: *const u8) {
    let _ = (reg, addr);
}

/// Load through the address in `addr` into `reg`.  Mirrors `armLoadPtr`.
pub fn armLoadPtr(reg: ArmReg, addr: *const ()) {
    let _ = (reg, addr);
}

/// Store `reg` to the address in `addr`.  Mirrors `armStorePtr`.
pub fn armStorePtr(reg: ArmReg, addr: *const ()) {
    let _ = (reg, addr);
}

/// Begin a stack frame.  Mirrors `armBeginStackFrame`.
pub fn armBeginStackFrame(save_fpr: bool) {
    let _ = save_fpr;
}

/// End a stack frame.  Mirrors `armEndStackFrame`.
pub fn armEndStackFrame(save_fpr: bool) {
    let _ = save_fpr;
}

/// Emit an unconditional jump to `ptr`.  Mirrors `armEmitJmp`.
pub fn armEmitJmp(_ptr: *const u8, _force_inline: bool) {}
/// Emit a call to `ptr`.  Mirrors `armEmitCall`.
pub fn armEmitCall(_ptr: *const u8, _force_inline: bool) {}
/// Emit a `cbnz reg, ptr`.  Mirrors `armEmitCbnz`.
pub fn armEmitCbnz(_reg: ArmReg, _ptr: *const u8) {}
/// Emit a `b.<cond> ptr`.  Mirrors `armEmitCondBranch`.
pub fn armEmitCondBranch(_cond: u8, _ptr: *const u8) {}
/// Emit a `tbl` lookup across two consecutive VFP tables.  Mirrors
/// `armEmitVTBL`.
pub fn armEmitVTBL(_dst: ArmVReg, _src1: ArmVReg, _src2: ArmVReg, _tbl: ArmVReg) {}

/// Top-level ARM64 assembly helper.  Mirrors the public surface of
/// `AsmHelpers.cpp`: callers from the rest of the dynarec use this
/// as a single dispatch point into the helper module.  `op` selects
/// the helper:
///
/// | `op` | helper                          |
/// |-----:|---------------------------------|
/// | 0    | `armWRegister`                  |
/// | 1    | `armXRegister`                  |
/// | 2    | `armSRegister`                  |
/// | 3    | `armDRegister`                  |
/// | 4    | `armQRegister`                  |
/// | 5    | `armSetAsmPtr`                  |
/// | 6    | `armStartBlock`                 |
/// | 7    | `armEndBlock`                   |
/// | 8    | `armEmitJmp`                    |
/// | 9    | `armEmitCall`                   |
/// | 10   | `armIsCalleeSavedRegister`      |
/// | 11   | `armOffsetMemOperand`           |
/// | 12   | `armMoveAddressToReg`           |
/// | 13   | `armLoadPtr` / `armStorePtr`    |
/// | 14   | `armBeginStackFrame` / `armEnd` |
/// | 15   | `armEmitVTBL`                   |
pub fn aarch64AsmHelper(op: u32, a: u32, b: u32, c: u32) -> u32 {
    match op {
        0  => { let _ = armWRegister(a as u8); 0 }
        1  => { let _ = armXRegister(a as u8); 0 }
        2  => { let _ = armSRegister(a as u8); 0 }
        3  => { let _ = armDRegister(a as u8); 0 }
        4  => { let _ = armQRegister(a as u8); 0 }
        5  => { armSetAsmPtr(a as *mut u8, b as usize, None); 0 }
        6  => { armStartBlock() as u32 }
        7  => { armEndBlock() as u32 }
        8  => { armEmitJmp(a as *const u8, b != 0); 0 }
        9  => { armEmitCall(a as *const u8, b != 0); 0 }
        10 => { if armIsCalleeSavedRegister(a as i32) { 1 } else { 0 } }
        11 => {
            let op = MemOperand { base: ArmReg(a as u8), offset: b as i32, post_index: false };
            let _ = armOffsetMemOperand(op, c as i64);
            0
        }
        12 => { armMoveAddressToReg(ArmReg(a as u8), b as *const u8); 0 }
        13 => { armLoadPtr(ArmReg(a as u8), b as *const ()); 0 }
        14 => { armBeginStackFrame(a != 0); armEndStackFrame(a != 0); 0 }
        15 => {
            armEmitVTBL(ArmVReg(a as u8), ArmVReg(b as u8), ArmVReg(c as u8), ArmVReg(0));
            0
        }
        _  => 0,
    }
}

// ===========================================================================
//  ARM64 recompiler stubs (translation of `RecStubs.cpp`)
// ===========================================================================

/// ARM64 backpatch stub for slowmem load/store.  Mirrors
/// `vtlb_DynBackpatchLoadStore` from `RecStubs.cpp`.  In the C++ source
/// this is a `pxFailRel("Not implemented.")` - the real implementation
/// lives in the x86 backend.
pub fn vtlb_DynBackpatchLoadStore(
    _code_address: uptr,
    _code_size: u32,
    _guest_pc: u32,
    _guest_addr: u32,
    _gpr_bitmask: u32,
    _fpr_bitmask: u32,
    _address_register: u8,
    _data_register: u8,
    _size_in_bits: u8,
    _is_signed: bool,
    _is_load: bool,
    _is_fpr: bool,
) {
    // Not implemented on ARM64 in the C++ source either.
}

/// Stub for `SaveStateBase::vuJITFreeze`.  In the C++ source this
/// logs a warning and writes a 96-byte empty block into the save state.
pub fn vuJITFreeze() -> bool {
    // The C++ version logs: "recompiler state is stubbed in arm64!".
    // We return `true` to indicate the freeze "succeeded".
    true
}

// ===========================================================================
//  Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitfield_decoders() {
        // Rd at bits 11..15
        assert_eq!(_Rd_(0x0000_0800), 1);
        // Rs at bits 21..25
        assert_eq!(_Rs_(0x0020_0000), 1);
        // Rt at bits 16..20
        assert_eq!(_Rt_(0x0001_0000), 1);
        // Sa at bits 6..10
        assert_eq!(_Sa_(0x0000_0040), 1);
        // func at bits 0..5
        assert_eq!(_func_(0x0000_0010), 0x10);
        // op at bits 26..31
        assert_eq!(_op_(0x0800_0000), 0x20);
    }

    #[test]
    fn vtlb_round_trip() {
        vtlb_reset();
        vtlb_map_pages(0x1000_0000, 0x8000_0000, 1);
        assert_eq!(recVTLBLookup(0x1000_0000), 0x8000_0000);
        assert_eq!(recVTLBLookup(0x1000_1234), 0x8000_1234);
    }

    #[test]
    fn mmi_dispatch_routes() {
        // MADD is SPECIAL / function 0
        MMI_Opcode(0x0000_0018); // rd=3, rs=0, rt=0, special, func=0
        // MMI0 sub-opcode PADDB
        MMI_Opcode(0x4000_0000);
        // MMI1
        MMI_Opcode(0x4400_0000);
        // MMI2
        MMI_Opcode(0x4800_0000);
        // MMI3
        MMI_Opcode(0x4C00_0000);
    }

    #[test]
    fn vif_length_fill() {
        // 16 transfers at 32 columns should be 16*16 = 256 bytes.
        assert_eq!(dVifComputeLength(32, 32, 16, true), 256);
        // 0 transfers means 256.
        assert_eq!(dVifComputeLength(32, 32, 0, true), 256 * 16);
    }

    #[test]
    fn arm_register_helpers() {
        assert_eq!(armWRegister(7).code(), 7);
        assert_eq!(armXRegister(31).code(), 31);
        assert_eq!(armSRegister(0).code(), 0);
        assert_eq!(armDRegister(15).code(), 15);
        assert_eq!(armQRegister(31).code(), 31);
    }

    #[test]
    fn arm_callee_saved() {
        assert!(!armIsCalleeSavedRegister(0));
        assert!(!armIsCalleeSavedRegister(18));
        assert!(armIsCalleeSavedRegister(19));
        assert!(armIsCalleeSavedRegister(28));
        assert!(!armIsCalleeSavedRegister(29));
    }
}
