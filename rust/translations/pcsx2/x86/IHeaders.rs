// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! PCSX2 x86 recompiler headers — Rust port.
//!
//! Idiomatic Rust 2021 translation of the C++ x86-side recompiler headers
//! that drive EE/IOP code generation: register-allocation tables, the
//! per-instruction liveness descriptor (`EEINST`), constant-propagation
//! state, dispatch helper traits, and the small `Xbyak`-style data
//! structures (`XmmRegister`, `recRegisterId`) used to name host
//! registers symbolically inside the dynarec.
//!
//! These types are *data only*; no code is generated, no `Xbyak`
//! dependency is pulled in, and no allocations are performed. They are
//! safe to share between the interpreter and the dynarec translation
//! passes. The dispatch helpers are exposed as traits so the existing
//! FFI/inline-asm code paths can keep their current call shapes while
//! the surrounding bookkeeping moves to safe Rust.
//!
//! The module keeps the original C++ naming where it makes sense
//! (`EEINST`, `_x86regs`, `_xmmregs`, `recRegisterId`, ...) and adds
//! idiomatic Rust wrappers (`RecompilerState`, `OpcodeDispatch`,
//! `RegisterAllocator`, ...).

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(clippy::upper_case_acronyms)]

use std::cell::RefCell;
use std::ptr;

// ---------------------------------------------------------------------------
// Fixed-width aliases matching the PCSX2 `<common/...>` typedefs used in the
// original headers. These keep the byte-for-byte layout that the dynarec
// expects when it reads constant tables and per-thread jump patches.
// ---------------------------------------------------------------------------

pub type u8 = core::primitive::u8;
pub type u16 = core::primitive::u16;
pub type u32 = core::primitive::u32;
pub type u64 = core::primitive::u64;
pub type s8 = core::primitive::i8;
pub type s32 = core::primitive::i32;
pub type uptr = usize;

// ---------------------------------------------------------------------------
// Constants: counts, register-class tags, opcode info bitmasks.
// Mirrors the `#define` block in `iCore.h` and `iR3000A.h`.
// ---------------------------------------------------------------------------

/// Number of allocatable GPR slots on the x86 backend. Matches
/// `iREGCNT_GPR` from the x86 emitter.
pub const IREGCNT_GPR: usize = 8;
/// Number of allocatable XMM slots on the x86 backend. Matches
/// `iREGCNT_XMM` from the x86 emitter.
pub const IREGCNT_XMM: usize = 16;
/// Deprecated jump-patch slot counts. The originals were 32-element
/// per-thread arrays; we keep the same capacity for layout compatibility.
pub const JUMP_PATCH_SLOTS: usize = 32;

// ---------------------------------------------------------------------------
// Register-allocation access modes (from `iCore.h`).
// ---------------------------------------------------------------------------

pub const MODE_READ: i32 = 1;
pub const MODE_WRITE: i32 = 2;
pub const MODE_CALLEESAVED: i32 = 0x20;
pub const MODE_COP2: i32 = 0x40;

// Per-instruction "reg is valid" bits packed into `info` (EE-style).
pub const PROCESS_EE_XMM: i32 = 0x02;
pub const PROCESS_EE_S: i32 = 0x04;
pub const PROCESS_EE_T: i32 = 0x08;
pub const PROCESS_EE_D: i32 = 0x10;
pub const PROCESS_EE_LO: i32 = 0x40;
pub const PROCESS_EE_HI: i32 = 0x80;
pub const PROCESS_EE_ACC: i32 = 0x40;

/// "Special info" tag bits: the source register is a propagated constant.
pub const PROCESS_CONSTS: i32 = 1;
/// "Special info" tag bits: the target register is a propagated constant.
pub const PROCESS_CONSTT: i32 = 2;

// Accessors for the per-instruction register fields packed into `info`.
// These replace the C macros `EEREC_S`, `EEREC_T`, ... used throughout the
// dynarec.
#[inline]
pub const fn eerec_s(info: i32) -> i32 {
    (info >> 8) & 0x0f
}
#[inline]
pub const fn eerec_t(info: i32) -> i32 {
    (info >> 12) & 0x0f
}
#[inline]
pub const fn eerec_d(info: i32) -> i32 {
    (info >> 16) & 0x0f
}
#[inline]
pub const fn eerec_lo(info: i32) -> i32 {
    (info >> 20) & 0x0f
}
#[inline]
pub const fn eerec_hi(info: i32) -> i32 {
    (info >> 24) & 0x0f
}
#[inline]
pub const fn eerec_acc(info: i32) -> i32 {
    (info >> 20) & 0x0f
}

// Setters for the same fields. Use the `PROCESS_EE_SET_*` macros by name.
#[inline]
pub const fn process_ee_set_s(reg: i32) -> i32 {
    (reg << 8) | PROCESS_EE_S
}
#[inline]
pub const fn process_ee_set_t(reg: i32) -> i32 {
    (reg << 12) | PROCESS_EE_T
}
#[inline]
pub const fn process_ee_set_d(reg: i32) -> i32 {
    (reg << 16) | PROCESS_EE_D
}
#[inline]
pub const fn process_ee_set_lo(reg: i32) -> i32 {
    (reg << 20) | PROCESS_EE_LO
}
#[inline]
pub const fn process_ee_set_hi(reg: i32) -> i32 {
    (reg << 24) | PROCESS_EE_HI
}
#[inline]
pub const fn process_ee_set_acc(reg: i32) -> i32 {
    (reg << 20) | PROCESS_EE_ACC
}

// ---------------------------------------------------------------------------
// XMM caching info bitmask (`enum xmminfo : u16`).
// ---------------------------------------------------------------------------

#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XmmInfo {
    ReadLo = 0x001,
    ReadHi = 0x002,
    WriteLo = 0x004,
    WriteHi = 0x008,
    WriteD = 0x010,
    ReadD = 0x020,
    ReadS = 0x040,
    ReadT = 0x080,
    ReadAcc = 0x200,
    WriteAcc = 0x400,
    WriteT = 0x800,
    Bit64Op = 0x1000,
    ForceRegS = 0x2000,
    ForceRegT = 0x4000,
    NoRename = 0x8000,
}

impl XmmInfo {
    #[inline]
    pub const fn bits(self) -> u16 {
        self as u16
    }
}

// Bitwise-OR helpers. The original C++ used `|` on the underlying
// `u16`; we expose the same via `From`/`BitOr` to keep call sites short.
impl core::ops::BitOr for XmmInfo {
    type Output = u16;
    #[inline]
    fn bitor(self, rhs: XmmInfo) -> u16 {
        (self as u16) | (rhs as u16)
    }
}

// ---------------------------------------------------------------------------
// X86 (32-bit) register class tags.
// ---------------------------------------------------------------------------

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum X86Type {
    Temp = 0,
    Gpr = 1,
    FpRc = 2,
    ViReg = 3,
    PcWriteback = 4,
    Psx = 5,
    PsxPcWriteback = 6,
}

impl X86Type {
    #[inline]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Temp,
            1 => Self::Gpr,
            2 => Self::FpRc,
            3 => Self::ViReg,
            4 => Self::PcWriteback,
            5 => Self::Psx,
            6 => Self::PsxPcWriteback,
            _ => Self::Temp,
        }
    }
}

// ---------------------------------------------------------------------------
// X86 (32-bit) register-allocation table (`_x86regs`).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct X86Reg {
    pub in_use: u8,
    pub reg: i8,
    pub mode: u8,
    pub needed: u8,
    pub kind: X86Type,
    pub counter: u16,
    pub extra: u32,
}

impl X86Reg {
    pub const fn uninit() -> Self {
        Self {
            in_use: 0,
            reg: 0,
            mode: 0,
            needed: 0,
            kind: X86Type::Temp,
            counter: 0,
            extra: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// XMM (128-bit) register-class tags.
// ---------------------------------------------------------------------------

pub const XMMTYPE_TEMP: u8 = 0;
pub const XMMTYPE_GPRREG: u8 = X86Type::Gpr as u8;
pub const XMMTYPE_FPREG: u8 = 6;
pub const XMMTYPE_FPACC: u8 = 7;
pub const XMMTYPE_VFREG: u8 = 8;

// HI/LO aliases for XMM-backed GPR pairs.
pub const XMMGPR_LO: i32 = 33;
pub const XMMGPR_HI: i32 = 32;
pub const XMMFPU_ACC: i32 = 32;

// Delete-register policies.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteReg {
    Free = 0,
    Flush = 1,
    FlushAndFree = 2,
    FreeNoWriteback = 3,
}

// XMM register-allocation table.
#[derive(Debug, Clone, Copy)]
pub struct XmmReg {
    pub in_use: u8,
    pub reg: i8,
    pub kind: u8,
    pub mode: u8,
    pub needed: u8,
    pub counter: u16,
}

impl XmmReg {
    pub const fn uninit() -> Self {
        Self {
            in_use: 0,
            reg: 0,
            kind: XMMTYPE_TEMP,
            mode: 0,
            needed: 0,
            counter: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Per-instruction liveness descriptor (`EEINST`).
// ---------------------------------------------------------------------------

/// Per-instruction liveness info. See the long comment in `iCore.h` for
/// the meaning of the `EEINST_*` flag bits. Slot 32 is HI, slot 33 is LO
/// in `regs`; slot 32 in `fpuregs` is the FPU ACC; slot 32 in `vfregs`
/// is the VU ACC, slot 33 is the VU I register.
#[derive(Debug, Clone, Copy)]
pub struct EEInst {
    pub info: u16,
    /// 34 entries: GPR[0..32], HI=32, LO=33.
    pub regs: [u8; 34],
    /// 33 entries: FPR[0..32], ACC=32.
    pub fpuregs: [u8; 33],
    /// 34 entries: VFR[0..32], ACC=32, I=33.
    pub vfregs: [u8; 34],
    /// 16 VI regs.
    pub viregs: [u8; 16],
    pub write_type: [u8; 3],
    pub write_reg: [u8; 3],
    pub read_type: [u8; 4],
    pub read_reg: [u8; 4],
}

impl Default for EEInst {
    fn default() -> Self {
        Self {
            info: 0,
            regs: [0; 34],
            fpuregs: [0; 33],
            vfregs: [0; 34],
            viregs: [0; 16],
            write_type: [0; 3],
            write_reg: [0; 3],
            read_type: [0; 4],
            read_reg: [0; 4],
        }
    }
}

impl EEInst {
    /// Mirrors C++ `_recClearInst(EEINST*)`: zero out all liveness bytes.
    pub fn clear(&mut self) {
        self.regs = [0; 34];
        self.fpuregs = [0; 33];
        self.vfregs = [0; 34];
        self.viregs = [0; 16];
        self.write_type = [0; 3];
        self.write_reg = [0; 3];
        self.read_type = [0; 4];
        self.read_reg = [0; 4];
    }
}

// Liveness flag bits (from `iCore.h`).
pub const EEINST_LIVE: u8 = 0x01;
pub const EEINST_LASTUSE: u8 = 0x08;
pub const EEINST_XMM: u8 = 0x20;
pub const EEINST_USED: u8 = 0x40;
pub const EEINST_COP2_DENORMALIZE_STATUS_FLAG: u16 = 0x100;
pub const EEINST_COP2_NORMALIZE_STATUS_FLAG: u16 = 0x200;
pub const EEINST_COP2_STATUS_FLAG: u16 = 0x400;
pub const EEINST_COP2_MAC_FLAG: u16 = 0x800;
pub const EEINST_COP2_CLIP_FLAG: u16 = 0x1000;
pub const EEINST_COP2_SYNC_VU0: u16 = 0x2000;
pub const EEINST_COP2_FINISH_VU0: u16 = 0x4000;
pub const EEINST_COP2_FLUSH_VU0_REGISTERS: u16 = 0x8000;

/// If unset, values which are not live will not be written back to memory.
/// Tends to break stuff at the moment.
pub const EE_WRITE_DEAD_VALUES: bool = true;

// ---------------------------------------------------------------------------
// EELiveness, FpuLiveness — bit-test helpers. The originals were
// `static __fi` inline functions in `iCore.h`; here they become `const fn`
// over an explicit `&EEInst` so they can be reused in safe code.
// ---------------------------------------------------------------------------

#[inline]
pub const fn eeinst_used_test(pinst: &EEInst, reg: usize) -> bool {
    (pinst.regs[reg] & (EEINST_USED | EEINST_LASTUSE)) == EEINST_USED
}

#[inline]
pub const fn eeinst_xmm_used_test(pinst: &EEInst, reg: usize) -> bool {
    (pinst.regs[reg] & (EEINST_USED | EEINST_XMM | EEINST_LASTUSE))
        == (EEINST_USED | EEINST_XMM)
}

#[inline]
pub const fn eeinst_vf_used_test(pinst: &EEInst, reg: usize) -> bool {
    (pinst.vfregs[reg] & (EEINST_USED | EEINST_LASTUSE)) == EEINST_USED
}

#[inline]
pub const fn eeinst_vi_used_test(pinst: &EEInst, reg: usize) -> bool {
    (pinst.viregs[reg] & (EEINST_USED | EEINST_LASTUSE)) == EEINST_USED
}

#[inline]
pub const fn eeinst_live_test(pinst: &EEInst, reg: usize) -> bool {
    EE_WRITE_DEAD_VALUES || (pinst.regs[reg] & EEINST_LIVE) != 0
}

#[inline]
pub const fn eeinst_rename_test(pinst: &EEInst, reg: usize) -> bool {
    reg == 0 || !eeinst_used_test(pinst, reg) || !eeinst_live_test(pinst, reg)
}

#[inline]
pub const fn fpuinst_is_live(pinst: &EEInst, reg: usize) -> bool {
    (pinst.fpuregs[reg] & EEINST_LIVE) != 0
}

#[inline]
pub const fn fpuinst_last_use(pinst: &EEInst, reg: usize) -> bool {
    (pinst.fpuregs[reg] & EEINST_LASTUSE) != 0
}

#[inline]
pub const fn fpuinst_used_test(pinst: &EEInst, reg: usize) -> bool {
    (pinst.fpuregs[reg] & (EEINST_USED | EEINST_LASTUSE)) == EEINST_USED
}

#[inline]
pub const fn fpuinst_live_test(pinst: &EEInst, reg: usize) -> bool {
    EE_WRITE_DEAD_VALUES || fpuinst_is_live(pinst, reg)
}

#[inline]
pub const fn fpuinst_rename_test(pinst: &EEInst, reg: usize) -> bool {
    !eeinst_used_test(pinst, reg) || !eeinst_live_test(pinst, reg)
}

// ---------------------------------------------------------------------------
// Flush-call policy bitmask (iFlushCall / _psxFlushCall parameters).
// ---------------------------------------------------------------------------

pub const FLUSH_NONE: i32 = 0x000;
pub const FLUSH_CONSTANT_REGS: i32 = 0x001;
pub const FLUSH_FLUSH_XMM: i32 = 0x002;
pub const FLUSH_FREE_XMM: i32 = 0x004;
pub const FLUSH_ALL_X86: i32 = 0x020;
pub const FLUSH_FREE_TEMP_X86: i32 = 0x040;
pub const FLUSH_FREE_NONTEMP_X86: i32 = 0x080;
pub const FLUSH_FREE_VU0: i32 = 0x100;
pub const FLUSH_PC: i32 = 0x200;
pub const FLUSH_CODE: i32 = 0x800;
pub const FLUSH_EVERYTHING: i32 = 0x1ff;
pub const FLUSH_INTERPRETER: i32 = 0xfff;
pub const FLUSH_FULLVTLB: i32 = 0x000;
pub const FLUSH_NODESTROY: i32 =
    FLUSH_CONSTANT_REGS | FLUSH_FLUSH_XMM | FLUSH_ALL_X86;

// ---------------------------------------------------------------------------
// FCR31 flag bits (`iFPUd.cpp`).
// ---------------------------------------------------------------------------

pub const FPUFLAG_C: u32 = 0x0080_0000;
pub const FPUFLAG_I: u32 = 0x0002_0000;
pub const FPUFLAG_D: u32 = 0x0001_0000;
pub const FPUFLAG_O: u32 = 0x0000_8000;
pub const FPUFLAG_U: u32 = 0x0000_4000;
pub const FPUFLAG_SI: u32 = 0x0000_0040;
pub const FPUFLAG_SD: u32 = 0x0000_0020;
pub const FPUFLAG_SO: u32 = 0x0000_0010;
pub const FPUFLAG_SU: u32 = 0x0000_0008;

// FPU behavior switches (from the `FPU_*` defines in `iFPUd.cpp`).
pub const FPU_FLAGS_OVERFLOW: bool = true;
pub const FPU_FLAGS_UNDERFLOW: bool = true;
pub const FPU_RESULT: bool = true;
pub const FPU_FLAGS_ID: bool = true;
pub const FPU_CORRECT_ADD_SUB: bool = true;

// ---------------------------------------------------------------------------
// IOP cycle penalties (from `iR3000A.h`).
// ---------------------------------------------------------------------------

pub const PSX_INST_CYCLES_MULT: i32 = 7;
pub const PSX_INST_CYCLES_DIV: i32 = 40;
pub const PSX_INST_CYCLES_PEEPHOLE_STORE: i32 = 0;
pub const PSX_INST_CYCLES_STORE: i32 = 0;
pub const PSX_INST_CYCLES_LOAD: i32 = 0;

/// IOP mirror of the EE's HI/LO XMM slot indices.
pub const PSX_HI: i32 = XMMGPR_HI;
pub const PSX_LO: i32 = XMMGPR_LO;

// ---------------------------------------------------------------------------
// `recRegisterId` — symbolic name for a host GPR slot used by the dynarec
// emitter. The C++ enum stored the names `EAX`, `ECX`, `EDX`, `EBX`,
// `ESP`, `EBP`, `ESI`, `EDI`; in Rust we use the standard `RegId::*`
// variant name and an explicit `code` field for ABI stability.
// ---------------------------------------------------------------------------

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RegId {
    EAX = 0,
    ECX = 1,
    EDX = 2,
    EBX = 3,
    ESP = 4,
    EBP = 5,
    ESI = 6,
    EDI = 7,
}

impl RegId {
    /// Returns the 3-bit x86 ModRM encoding for the register.
    #[inline]
    pub const fn code(self) -> u8 {
        self as u8
    }

    /// Convert from the raw 3-bit x86 encoding.
    #[inline]
    pub const fn from_code(code: u8) -> Self {
        match code & 0x07 {
            0 => Self::EAX,
            1 => Self::ECX,
            2 => Self::EDX,
            3 => Self::EBX,
            4 => Self::ESP,
            5 => Self::EBP,
            6 => Self::ESI,
            7 => Self::EDI,
            _ => Self::EAX,
        }
    }
}

// ---------------------------------------------------------------------------
// `XmmRegister` — symbolic name for a host XMM slot. Mirrors the
// `xRegisterSSE` enum from the x86 emitter but kept dependency-free.
// ---------------------------------------------------------------------------

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum XmmRegister {
    XMM0 = 0,
    XMM1 = 1,
    XMM2 = 2,
    XMM3 = 3,
    XMM4 = 4,
    XMM5 = 5,
    XMM6 = 6,
    XMM7 = 7,
    XMM8 = 8,
    XMM9 = 9,
    XMM10 = 10,
    XMM11 = 11,
    XMM12 = 12,
    XMM13 = 13,
    XMM14 = 14,
    XMM15 = 15,
}

impl XmmRegister {
    #[inline]
    pub const fn code(self) -> u8 {
        self as u8
    }

    #[inline]
    pub const fn from_code(code: u8) -> Self {
        match code & 0x0f {
            0 => Self::XMM0,
            1 => Self::XMM1,
            2 => Self::XMM2,
            3 => Self::XMM3,
            4 => Self::XMM4,
            5 => Self::XMM5,
            6 => Self::XMM6,
            7 => Self::XMM7,
            8 => Self::XMM8,
            9 => Self::XMM9,
            10 => Self::XMM10,
            11 => Self::XMM11,
            12 => Self::XMM12,
            13 => Self::XMM13,
            14 => Self::XMM14,
            15 => Self::XMM15,
            _ => Self::XMM0,
        }
    }

    /// Returns the x86 emitter's "is high XMM" predicate. The original
    /// code needed this when AVX-style 3-operand VEX encoding required
    /// the operand XMM to live in xmm8..xmm15.
    #[inline]
    pub const fn is_high(self) -> bool {
        (self as u8) >= 8
    }
}

// ---------------------------------------------------------------------------
// `Xbyak`-style wrapper for `XmmRegister`. The dynarec frequently wants to
// talk about an XMM slot as both a "host register" and as a numeric index
// into the alloc table; the wrapper is layout-compatible with the C++
// `xRegisterSSE` POD.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct XbyakXmm(pub XmmRegister);

impl XbyakXmm {
    pub const fn new(reg: XmmRegister) -> Self {
        Self(reg)
    }
    #[inline]
    pub const fn id(self) -> u8 {
        self.0.code()
    }
}

impl From<XmmRegister> for XbyakXmm {
    #[inline]
    fn from(reg: XmmRegister) -> Self {
        Self(reg)
    }
}

// ---------------------------------------------------------------------------
// `Xbyak`-style wrapper for `RegId`. Same idea as the XMM wrapper but for
// the 32-bit GPR class.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct XbyakGpr(pub RegId);

impl XbyakGpr {
    pub const fn new(reg: RegId) -> Self {
        Self(reg)
    }
    #[inline]
    pub const fn id(self) -> u8 {
        self.0.code()
    }
}

impl From<RegId> for XbyakGpr {
    #[inline]
    fn from(reg: RegId) -> Self {
        Self(reg)
    }
}

// ---------------------------------------------------------------------------
// `Recompiler` — a host register reference. Many of the helpers in
// `iR5900.h` and `iCore.h` take a `const x86Emitter::xRegister32&` or
// `const x86Emitter::xRegister64&`; we model those as small newtype
// wrappers that the dynarec passes around by value.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Recompiler {
    id: RegId,
    /// `true` for 64-bit reference (e.g. `xRegister64`), `false` for 32-bit.
    is_64: bool,
}

impl Recompiler {
    pub const fn gpr32(id: RegId) -> Self {
        Self { id, is_64: false }
    }
    pub const fn gpr64(id: RegId) -> Self {
        Self { id, is_64: true }
    }
    #[inline]
    pub const fn id(self) -> RegId {
        self.id
    }
    #[inline]
    pub const fn is_64(self) -> bool {
        self.is_64
    }
}

// ---------------------------------------------------------------------------
// `RecompilerState` — the per-thread bookkeeping for the EE/IOP dynarec.
// Mirrors the `extern` globals in `iCore.h` and `iR3000A.h`.
// ---------------------------------------------------------------------------

/// Per-thread state for the EE (R5900) dynarec. All fields are simple
/// POD equivalents of the `extern` C++ symbols, with the same names.
#[derive(Debug)]
pub struct RecompilerState {
    /// Whether the dynarec is currently compiling an instruction that
    /// lives in a branch delay slot. Mirrors `g_recompilingDelaySlot`.
    pub recompiling_delay_slot: bool,
    /// The PC the dynarec is currently emitting code for.
    pub pc: u32,
    /// Set to a non-zero value when the current instruction is a branch.
    pub branch: i32,
    /// Branch target for the current instruction.
    pub target: u32,
    /// Cycles of the current block being recompiled.
    pub block_cycles: u32,
    /// `true` if the current block has VU0 interlocking.
    pub block_interlocked: bool,
    /// Maximum address we allow the dynarec to emit up to.
    pub maxrecmem: u32,
    /// Bitmask of GPRs that are propagated constants.
    pub cpu_has_const_reg: u32,
    /// Bitmask of GPR constants that have been flushed to memory.
    pub cpu_flushed_const_reg: u32,
    /// 32 propagated GPR values, parallel to `cpuRegs.GPR.r[]`.
    pub cpu_const_regs: [GprReg64; 32],
    /// X86 GPR allocation table.
    pub x86regs: Vec<X86Reg>,
    /// Saved X86 GPR allocation table.
    pub save_x86regs: Vec<X86Reg>,
    /// XMM allocation table.
    pub xmmregs: Vec<XmmReg>,
    /// Saved XMM allocation table.
    pub save_xmmregs: Vec<XmmReg>,
    /// X86 allocator epoch counter.
    pub x86_alloc_counter: u16,
    /// XMM allocator epoch counter.
    pub xmm_alloc_counter: u16,
    /// Deprecated 8-bit jump-patch slots.
    pub j8_ptr: [*mut u8; JUMP_PATCH_SLOTS],
    /// Deprecated 32-bit jump-patch slots.
    pub j32_ptr: [*mut u32; JUMP_PATCH_SLOTS],
    /// Pointer to the instruction currently being analysed.
    pub cur_inst: Option<EEInst>,
}

impl RecompilerState {
    /// Construct a default-initialised state. Allocation tables are sized
    /// from the `IREGCNT_*` constants.
    pub fn new() -> Self {
        Self {
            recompiling_delay_slot: false,
            pc: 0,
            branch: 0,
            target: 0,
            block_cycles: 0,
            block_interlocked: false,
            maxrecmem: 0,
            cpu_has_const_reg: 0,
            cpu_flushed_const_reg: 0,
            cpu_const_regs: [GprReg64 { ul: [0, 0] }; 32],
            x86regs: vec![X86Reg::uninit(); IREGCNT_GPR],
            save_x86regs: vec![X86Reg::uninit(); IREGCNT_GPR],
            xmmregs: vec![XmmReg::uninit(); IREGCNT_XMM],
            save_xmmregs: vec![XmmReg::uninit(); IREGCNT_XMM],
            x86_alloc_counter: 0,
            xmm_alloc_counter: 0,
            j8_ptr: [ptr::null_mut(); JUMP_PATCH_SLOTS],
            j32_ptr: [ptr::null_mut(); JUMP_PATCH_SLOTS],
            cur_inst: None,
        }
    }
}

impl Default for RecompilerState {
    fn default() -> Self {
        Self::new()
    }
}

// Mirror of the C++ `GPR_reg64` POD (see `iR5900.h`).
#[derive(Debug, Clone, Copy, Default)]
pub struct GprReg64 {
    pub ul: [u64; 2],
}

impl GprReg64 {
    pub const fn zero() -> Self {
        Self { ul: [0, 0] }
    }
}

// Mirror of the C++ `FPControlRegister` POD used by `iFPUd.cpp` for the
// transient `roundmode_nearest` save/restore around `recSQRT_S_xmm`.
#[derive(Debug, Clone, Copy, Default)]
pub struct FpControlRegister {
    pub bitmask: u32,
}

// ---------------------------------------------------------------------------
// Constants for the double-precision FPU helpers (`iFPUd.cpp`).
// These are the well-known double-precision bit patterns for the
// IEEE-754 boundary values used by `ToDouble` / `ToPS2FPU_Full`.
// ---------------------------------------------------------------------------

/// Bit pattern for the boundary between "normal" and PS2-overflow doubles.
/// Equivalent to `DOUBLE(0, 1151, 0)` from the C++.
pub const DBL_CVT_OVERFLOW_BITS: u64 = 0x47F0_0000_0000_0000_u64;
/// Bit pattern for the boundary above which doubles must be clamped.
/// Equivalent to `DOUBLE(0, 1152, 0)` from the C++.
pub const DBL_PS2_OVERFLOW_BITS: u64 = 0x4800_0000_0000_0000_u64;
/// Bit pattern for the boundary below which doubles underflow to zero.
/// Equivalent to `DOUBLE(0, 897, 0)` from the C++.
pub const DBL_UNDERFLOW_BITS: u64 = 0x3810_0000_0000_0000_u64;

// Single-precision min/max vals referenced by `ToPS2FPU` when `FPU_RESULT`
// is unset. From `iFPU.h` (`g_minvals`, `g_maxvals`).
pub const FPU_G_MINVALS: [u32; 4] = [0xFF7F_FFFF, 0xFFFF_FFFF, 0xFFFF_FFFF, 0xFFFF_FFFF];
pub const FPU_G_MAXVALS: [u32; 4] = [0x7F7F_FFFF, 0xFFFF_FFFF, 0xFFFF_FFFF, 0xFFFF_FFFF];

// ---------------------------------------------------------------------------
// `OpcodeDispatch` — the trait used by the EE dynarec to find the
// recompiler entry point for a given opcode. The C++ version was a
// per-coprocessor function table; we expose the same surface as a trait
// so the EE/COP1/COP2/MMI dispatchers can plug in their own tables
// independently.
// ---------------------------------------------------------------------------

/// A recompiled opcode is a function pointer with the standard EE
/// calling convention: no args, no return. The dynarec stores pointers
/// to these in the opcode table and tail-calls them.
pub type R5900FnPtr = fn();

/// Variant that takes the packed `info` (S/T/D reg indices + flags).
pub type R5900FnPtrInfo = fn(info: i32);

/// Variant used by the IOP (R3000A) dispatch.
pub type R3000AFnPtr = fn();
pub type R3000AFnPtrInfo = fn(info: i32);

/// Behaviour every opcode-dispatch table exposes. Concrete tables
/// (`EeDispatch`, `Cop1Dispatch`, `Cop0Dispatch`, ...) live in the
/// per-coprocessor modules; this trait only fixes the shape.
pub trait OpcodeDispatch {
    /// Look up a dispatcher by raw 6-bit primary opcode.
    fn dispatch_primary(&self, op: u32) -> Option<R5900FnPtr>;
    /// Look up a dispatcher by 6-bit function sub-opcode (after the
    /// primary opcode has been decoded).
    fn dispatch_function(&self, op: u32) -> Option<R5900FnPtr>;
    /// Look up a single full 32-bit instruction word.
    fn dispatch_full(&self, code: u32) -> Option<R5900FnPtrInfo>;
}

// ---------------------------------------------------------------------------
// `RecompileCodeRC0` etc. — function-pointer signatures that the
// `eeRecompileCodeRC0` helper expects. These mirror the C++ `R5900FNPTR`
// and `R5900FNPTR_INFO` typedefs.
// ---------------------------------------------------------------------------

/// `(rd, rs, rt) -> void` — register-arithmetic helper dispatcher.
pub type RecompileRc0Fn = fn();

/// `(info) -> void` — same shape but takes the packed reg info.
pub type RecompileRc0InfoFn = fn(info: i32);

/// `(rt, rs, imm) -> void` — register-arithmetic-with-immediate.
pub type RecompileRc1Fn = fn();
pub type RecompileRc1InfoFn = fn(info: i32);

/// `(rd, rt, sa) -> void` — shift-by-constant.
pub type RecompileRc2Fn = fn();
pub type RecompileRc2InfoFn = fn(info: i32);

/// Trait covering the `eeRecompileCodeRC*` family of dispatch helpers.
pub trait RegisterArithDispatch {
    /// `rd = rs OP rt` (R-type arithmetic).
    fn recompile_code_rc0(
        &mut self,
        constcode: RecompileRc0Fn,
        constscode: RecompileRc0InfoFn,
        consttcode: RecompileRc0InfoFn,
        noconstcode: RecompileRc0InfoFn,
        xmm_info: i32,
    );
    /// `rt = rs OP imm16` (I-type arithmetic).
    fn recompile_code_rc1(
        &mut self,
        constcode: RecompileRc1Fn,
        noconstcode: RecompileRc1InfoFn,
        xmm_info: i32,
    );
    /// `rd = rt OP sa` (shift-by-constant).
    fn recompile_code_rc2(
        &mut self,
        constcode: RecompileRc2Fn,
        noconstcode: RecompileRc2InfoFn,
        xmm_info: i32,
    );
}

// ---------------------------------------------------------------------------
// `RegisterAllocator` — trait modelling the `_allocX86reg`,
// `_allocXMMreg`, `_allocTempXMMreg` family. The dynarec asks the
// allocator for a host slot; the allocator returns an index into the
// matching `X86Reg` / `XmmReg` table. A negative return means "no slot
// available, flush the caller first".
// ---------------------------------------------------------------------------

pub trait RegisterAllocator {
    /// Allocate a temp x86 GPR. Returns the index of the new entry, or
    /// `-1` if none was free.
    fn alloc_temp_gpr(&mut self, mode: i32) -> i32;

    /// Allocate a specific EE GPR into a host GPR slot.
    fn alloc_gpr_to_x86(&mut self, gprreg: i32, mode: i32) -> i32;

    /// Allocate a temp XMM slot.
    fn alloc_temp_xmm(&mut self, kind: u8) -> i32;

    /// Allocate a specific FPU register into an XMM slot.
    fn alloc_fpr_to_xmm(&mut self, fprreg: i32, mode: i32) -> i32;

    /// Allocate a specific EE GPR into an XMM slot.
    fn alloc_gpr_to_xmm(&mut self, gprreg: i32, mode: i32) -> i32;

    /// Free a host x86 GPR slot.
    fn free_x86(&mut self, slot: i32);

    /// Free a host XMM slot.
    fn free_xmm(&mut self, slot: i32);

    /// Flush all dirty host registers to memory. Used before a call.
    fn flush_dirty(&mut self);
}

// ---------------------------------------------------------------------------
// `AnalysisPass` — minimal mirror of the C++ `R5900::AnalysisPass` base
// class. Subclasses in the dynarec override `run`; we use a small
// `dyn FnMut` so the trait stays object-safe and easy to drive from a
// dataflow walker.
// ---------------------------------------------------------------------------

pub trait AnalysisPass {
    /// Human-readable name of the pass; useful for the dynarec log.
    fn name(&self) -> &'static str;

    /// Run the pass over the cached instruction block.
    fn run(&mut self, start: u32, end: u32, inst_cache: &mut [EEInst]);
}

// ---------------------------------------------------------------------------
// `recBackpropBSC` — a back-propagation hook used by the EE branch-swap
// analysis. Modelled as a plain function pointer so the dynarec can
// re-use the FFI/inline-asm implementation that already exists in C++.
// ---------------------------------------------------------------------------

pub type RecBackpropBscFn = fn(code: u32, prev: &mut EEInst, pinst: &mut EEInst);

// ---------------------------------------------------------------------------
// Convenience: a small `RefCell`-backed `RecompilerState` that can be
// stored in a `thread_local!`. The C++ code used plain `extern`
// globals; this is the closest safe-Rust equivalent.
// ---------------------------------------------------------------------------

thread_local! {
    static EE_RECOMPILER_STATE: RefCell<RecompilerState> =
        RefCell::new(RecompilerState::new());
}

/// Returns a handle to the per-thread EE recompiler state. Panics if the
/// state is already mutably borrowed.
pub fn with_ee_recompiler<R>(f: impl FnOnce(&mut RecompilerState) -> R) -> R {
    EE_RECOMPILER_STATE.with(|cell| f(&mut cell.borrow_mut()))
}

thread_local! {
    static PSX_RECOMPILER_STATE: RefCell<RecompilerState> =
        RefCell::new(RecompilerState::new());
}

/// Returns a handle to the per-thread IOP recompiler state. Panics if
/// the state is already mutably borrowed.
pub fn with_psx_recompiler<R>(f: impl FnOnce(&mut RecompilerState) -> R) -> R {
    PSX_RECOMPILER_STATE.with(|cell| f(&mut cell.borrow_mut()))
}
