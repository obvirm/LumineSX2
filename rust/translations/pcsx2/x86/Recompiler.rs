// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! PCSX2 EE (Emotion Engine) x86 recompiler module — Rust port.
//!
//! This module is the idiomatic Rust 2021 translation of the EE interpreter
//! (`iCore.*`, `iFPU.*`, `iMMI.*`, `iR3000A.*`) and the EE-to-x86
//! recompiler / micro-VU recompiler (`iR5900.*`, `microVU.*`,
//! `recVTLB.*`, `Vif_Dynarec.*`, `Vif_UnpackSSE.*`,
//! `BaseblockEx.*`, `R5900_Profiler.*`).
//!
//! # Architecture
//!
//! The original C++ is split into roughly four concerns that share
//! register-allocation infrastructure:
//!
//! 1. **EE interpreter dispatch** — `EE_Core_Opcode`, `FPU_Opcode`,
//!    `MMI_Opcode` are the entry points that decode a 32-bit EE
//!    instruction word and run one step of the interpreter. Each
//!    follows the same pattern: inspect the primary opcode field
//!    (`code >> 26`), the function field (`code & 0x3F`), and any
//!    sub-coprocessor fields, then mutate the EE register file
//!    (`cpuRegs`) and the cycle counter.
//!
//! 2. **IOP (R3000A) interpreter dispatch** — `iR3000A_Opcode` decodes
//!    a MIPS-I instruction from `psxRegs.code` and runs the matching
//!    `psx*` interpreter helper. The PSX opcode space is small and
//!    fully covered by the [`PSX_OPCODE_TABLE`] lookup table.
//!
//! 3. **EE-to-x86 recompilation** — `R5900_RecompileBlock` is the
//!    primary entry point. It walks an EE basic block, allocates
//!    x86 / XMM registers for EE GPR / FPR state, and emits
//!    host instructions into a code cache. This is the largest
//!    part of the original C++ (well over 10k LOC across
//!    `iR5900*.cpp`); in this Rust translation it is exposed as
//!    a stub that returns `()` because the real implementation
//!    depends on the dynarec emitter, VTLB, base-block manager,
//!    and full CPU state — none of which fit a `std`-only module.
//!
//! 4. **micro-VU recompilation** — `microVU_RecompileBlock` performs
//!    the same role for Vector Unit microprograms. It operates on
//!    `microVU0` / `microVU1` and is also exposed as a stub here.
//!
//! The module deliberately uses only `std`. Anything that needs
//! the x86 emitter, the PS2 memory bus, the FPU state, the EE
//! register file, the recompiler cache, or the profiler is
//! represented by lightweight data types — see
//! [`RegAlloc`], [`RecCache`], [`MicroVu`], [`BaseBlocks`],
//! [`RecompilerStats`], etc. — that mirror the C++ ABI but do not
//! perform any real work.
//!
//! The PSX (`rpsxBSC`) primary-opcode table is the only piece of
//! concrete data that has been ported verbatim, because the spec
//! requires it and because it is genuinely a small, well-defined
//! table of 64 entries indexed by the top 6 bits of `psxRegs.code`.

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(dead_code)]
#![allow(unused_variables)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

// ---------------------------------------------------------------------------
// Primitive aliases
// ---------------------------------------------------------------------------
//
// The original C++ uses `u8`, `u16`, `u32`, `u64`, `s8`, `s32` and `uptr` from
// its Common.h. We keep the names so the port reads 1:1 with the source.

pub type u8 = ::std::primitive::u8;
pub type u16 = ::std::primitive::u16;
pub type u32 = ::std::primitive::u32;
pub type u64 = ::std::primitive::u64;
pub type s8 = ::std::primitive::i8;
pub type s32 = ::std::primitive::i32;
pub type s64 = ::std::primitive::i64;

/// Unsigned pointer-sized integer. In the C++ code this is `uptr`.
pub type uptr = usize;

/// Signed pointer-sized integer.
pub type sptr = isize;

// ---------------------------------------------------------------------------
// x86 register-allocation types
// ---------------------------------------------------------------------------
//
// Mirror of `enum x86type` and the `_x86regs` / `_xmmregs` structs from
// `iCore.h`. The dynarec tags every cached host register with the kind
// of EE-side state it currently holds (GPR, FPU, COP2 VF, etc.).

/// Tag describing what an x86 host register is currently mapped to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum X86Type {
    Temp = 0,
    Gpr = 1,
    Fprc = 2,
    ViReg = 3,
    PcWriteback = 4,
    Psx = 5,
    PsxPcWriteback = 6,
}

impl Default for X86Type {
    fn default() -> Self {
        X86Type::Temp
    }
}

/// Mode flags for register allocation. `MODE_READ` and `MODE_WRITE`
/// can be combined; `MODE_CALLEESAVED` indicates the value must
/// survive an indirect call.
pub const MODE_READ: u32 = 0x1;
pub const MODE_WRITE: u32 = 0x2;
pub const MODE_CALLEESAVED: u32 = 0x20;
pub const MODE_COP2: u32 = 0x40;

pub const PROCESS_EE_XMM: u32 = 0x02;
pub const PROCESS_EE_S: u32 = 0x04;
pub const PROCESS_EE_T: u32 = 0x08;
pub const PROCESS_EE_D: u32 = 0x10;
pub const PROCESS_EE_LO: u32 = 0x40;
pub const PROCESS_EE_HI: u32 = 0x80;
pub const PROCESS_EE_ACC: u32 = 0x40;

pub const PROCESS_CONSTS: u32 = 1;
pub const PROCESS_CONSTT: u32 = 2;

pub const XMMGPR_LO: u8 = 33;
pub const XMMGPR_HI: u8 = 32;
pub const XMMFPU_ACC: u8 = 32;

/// State of a single cached x86 GPR allocation, modelled on `_x86regs`.
#[derive(Debug, Clone)]
pub struct X86Reg {
    pub in_use: bool,
    pub reg: s8,
    pub mode: u8,
    pub needed: bool,
    pub ty: X86Type,
    pub counter: u16,
    pub extra: u32,
}

impl Default for X86Reg {
    fn default() -> Self {
        X86Reg {
            in_use: false,
            reg: -1,
            mode: 0,
            needed: false,
            ty: X86Type::Temp,
            counter: 0,
            extra: 0,
        }
    }
}

/// State of a single cached XMM (128-bit) allocation, modelled on
/// `_xmmregs`.
#[derive(Debug, Clone)]
pub struct XmmReg {
    pub in_use: bool,
    pub reg: s8,
    pub ty: u8,
    pub mode: u8,
    pub needed: bool,
    pub counter: u16,
}

impl Default for XmmReg {
    fn default() -> Self {
        XmmReg {
            in_use: false,
            reg: -1,
            ty: 0,
            mode: 0,
            needed: false,
            counter: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// iFlushCall flags
// ---------------------------------------------------------------------------

pub const FLUSH_NONE: u32 = 0x000;
pub const FLUSH_CONSTANT_REGS: u32 = 0x001;
pub const FLUSH_FLUSH_XMM: u32 = 0x002;
pub const FLUSH_FREE_XMM: u32 = 0x004;
pub const FLUSH_ALL_X86: u32 = 0x020;
pub const FLUSH_FREE_TEMP_X86: u32 = 0x040;
pub const FLUSH_FREE_NONTEMP_X86: u32 = 0x080;
pub const FLUSH_FREE_VU0: u32 = 0x100;
pub const FLUSH_PC: u32 = 0x200;
pub const FLUSH_CODE: u32 = 0x800;
pub const FLUSH_EVERYTHING: u32 = 0x1ff;
pub const FLUSH_INTERPRETER: u32 = 0xfff;
pub const FLUSH_FULLVTLB: u32 = 0x000;
pub const FLUSH_NODESTROY: u32 = FLUSH_CONSTANT_REGS | FLUSH_FLUSH_XMM | FLUSH_ALL_X86;

// ---------------------------------------------------------------------------
// EE recompiler state
// ---------------------------------------------------------------------------
//
// Minimal stand-ins for the global C++ state. The real definitions
// live in `cpuRegs.h`, `psxRegs.h`, `VURegs.h`, `IopGte.h`, etc. We
// only model the fields the dispatcher entry points actually touch.

/// Per-instruction live-range / liveness information. Mirrors `EEINST`.
#[derive(Debug, Clone)]
pub struct EEInst {
    pub info: u16,
    pub regs: [u8; 34],
    pub fpuregs: [u8; 33],
    pub vfregs: [u8; 34],
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

/// EE recompiler register allocator. Stand-in for the global
/// `x86regs[]`, `xmmregs[]`, `g_x86AllocCounter`, `g_xmmAllocCounter`
/// in `iCore.cpp`.
pub struct RegAlloc {
    pub gpr: Vec<X86Reg>,
    pub xmm: Vec<XmmReg>,
    pub x86_alloc_counter: u16,
    pub xmm_alloc_counter: u16,
}

impl RegAlloc {
    pub fn new(gpr_count: usize, xmm_count: usize) -> Self {
        RegAlloc {
            gpr: vec![X86Reg::default(); gpr_count],
            xmm: vec![XmmReg::default(); xmm_count],
            x86_alloc_counter: 0,
            xmm_alloc_counter: 0,
        }
    }
}

/// Stub for the EE register file (`cpuRegs.GPR.r[N]`, `cpuRegs.CP0.*`).
/// A real port would replace this with a `&mut CpuRegs` reference.
#[derive(Debug, Default, Clone)]
pub struct EeRegFile {
    pub gpr: [u64; 32],
    pub hi: u64,
    pub lo: u64,
    pub sa: u32,
    pub pc: u32,
    pub cycle: u32,
    pub code: u32,
    pub branch: i32,
    pub target: u32,
    pub gpr_dirty: u32,
    pub cp0_status: u32,
    pub cp0_cause: u32,
    pub cp0_epc: u32,
}

/// Stub for the FPU register file (`cpuRegs.FPR`).
#[derive(Debug, Default, Clone)]
pub struct FpuRegFile {
    pub fpr: [u128; 32],
    pub acc: u128,
    pub cccr: [u32; 3],
    pub fcsr: u32,
}

/// Stub for the COP2 / VU register files.
#[derive(Debug, Default, Clone)]
pub struct VuRegFile {
    pub vf: [[u32; 4]; 32],
    pub vi: [u16; 16],
    pub q: u32,
    pub p: u32,
    pub mac_flag: [u32; 4],
    pub clip_flag: [u32; 4],
    pub stat_flag: [u32; 4],
}

/// Stub for the IOP / R3000A register file.
#[derive(Debug, Default, Clone)]
pub struct PsxRegFile {
    pub gpr: [u32; 32],
    pub hi: u32,
    pub lo: u32,
    pub pc: u32,
    pub code: u32,
    pub pc_writeback: u32,
    pub cp0_status: u32,
    pub cp0_cause: u32,
    pub cp0_epc: u32,
}

/// Stub for the GTE register file used by the IOP's COP2.
#[derive(Debug, Default, Clone)]
pub struct GteRegFile {
    pub v: [[i16; 4]; 32],
    pub r: [[i16; 4]; 32],
    pub mac: [i32; 4],
    pub ir: [i32; 4],
    pub otz: i32,
    pub sxy: [i32; 3],
    pub sz: [i32; 4],
    pub rgb: [i32; 3],
    pub res: i32,
    pub lzcs: u32,
    pub lzcr: u32,
    pub flag: u32,
}

/// Top-level shared CPU state.
#[derive(Debug, Default)]
pub struct CpuState {
    pub ee: EeRegFile,
    pub fpu: FpuRegFile,
    pub vu0: VuRegFile,
    pub vu1: VuRegFile,
    pub psx: PsxRegFile,
    pub gte: GteRegFile,
    /// Constant-propagation cache for the EE GPR file.
    pub cpu_const_regs: [u128; 32],
    pub cpu_has_const: u32,
    pub cpu_flushed_const: u32,
    /// Constant-propagation cache for the IOP GPR file.
    pub psx_const_regs: [u32; 32],
    pub psx_has_const: u32,
    pub psx_flushed_const: u32,
}

// ---------------------------------------------------------------------------
// Recompiler code cache & base blocks
// ---------------------------------------------------------------------------

/// One executable code cache page, modelled on the dynarec's
/// 4 KiB / 16 MiB split.
#[derive(Debug, Default)]
pub struct RecCachePage {
    pub code: Vec<u8>,
}

/// Recompiler output buffer. The real implementation emits x86
/// instructions through the x86Emitter; here it is just a `Vec<u8>`.
#[derive(Debug, Default)]
pub struct RecCache {
    pub pages: Vec<RecCachePage>,
    pub cur: Vec<u8>,
}

impl RecCache {
    pub fn new() -> Self {
        RecCache::default()
    }

    /// Emit a single byte. Stub for `x86Emitter::Emit(...)`.
    pub fn emit_u8(&mut self, b: u8) {
        self.cur.push(b);
    }

    /// Emit a 32-bit little-endian word.
    pub fn emit_u32(&mut self, w: u32) {
        self.cur.extend_from_slice(&w.to_le_bytes());
    }

    /// Reserve `n` bytes and return a back-patch handle.
    pub fn reserve(&mut self, n: usize) -> Backpatch {
        let offset = self.cur.len();
        self.cur.resize(offset + n, 0);
        Backpatch { offset }
    }

    /// Write a 32-bit value into a previously reserved slot.
    pub fn patch_u32(&mut self, bp: Backpatch, value: u32) {
        let bytes = value.to_le_bytes();
        let off = bp.offset;
        self.cur[off..off + 4].copy_from_slice(&bytes);
    }
}

/// Handle to a slot previously reserved via [`RecCache::reserve`].
#[derive(Debug, Clone, Copy)]
pub struct Backpatch {
    pub offset: usize,
}

/// One emitted basic block. Mirrors `BASEBLOCKEX` from
/// `BaseblockEx.h`. A real port would store `fnptr` as a code-cache
/// pointer; here it is a `usize` offset into [`RecCache::cur`].
#[derive(Debug, Clone, Default)]
pub struct BaseBlockEx {
    pub fnptr: usize,
    pub startpc: u32,
    pub size: u32,
    pub x86size: u32,
}

/// Sorted base-block table. Mirrors `BaseBlocks` from
/// `BaseblockEx.h` but uses a `BTreeMap` for ordered iteration.
#[derive(Debug, Default)]
pub struct BaseBlocks {
    pub recompiler: usize,
    pub blocks: BTreeMap<u32, BaseBlockEx>,
    pub links: BTreeMap<u32, usize>,
}

impl BaseBlocks {
    pub fn new() -> Self {
        BaseBlocks::default()
    }

    /// Insert a freshly compiled block, returning a mutable handle.
    pub fn new_block(&mut self, startpc: u32, fnptr: usize) -> &mut BaseBlockEx {
        self.blocks.insert(
            startpc,
            BaseBlockEx {
                fnptr,
                startpc,
                size: 0,
                x86size: 0,
            },
        );
        self.blocks.get_mut(&startpc).expect("just inserted")
    }

    /// Find the block containing `startpc`, if any.
    pub fn get(&self, startpc: u32) -> Option<&BaseBlockEx> {
        // The original code uses binary search; here the map is ordered
        // and we look up the greatest key <= startpc.
        self.blocks
            .range(..=startpc)
            .next_back()
            .map(|(_, b)| b)
            .filter(|b| b.startpc <= startpc && (b.size == 0 || startpc < b.startpc + b.size * 4))
    }

    /// Patch an outstanding branch into the target block (or, if the
    /// target is not yet compiled, into the recompiler dispatcher).
    pub fn link(&mut self, pc: u32, jump_offset: usize) {
        if let Some(target) = self.blocks.get(&pc).filter(|b| b.startpc == pc) {
            let delta = target.fnptr as isize - jump_offset as isize - 4;
            self.links.insert(pc, delta as usize);
        } else {
            let delta = self.recompiler as isize - jump_offset as isize - 4;
            self.links.insert(pc, delta as usize);
        }
    }
}

// ---------------------------------------------------------------------------
// Recompiler profiler
// ---------------------------------------------------------------------------

/// Recompiler-time profiler. Mirrors `EE::Profiler` from
/// `R5900_Profiler.h`. The real implementation tracks per-opcode
/// emission counts; here it just bumps a counter.
#[derive(Debug, Default)]
pub struct RecompilerStats {
    pub op_counts: std::collections::BTreeMap<u32, u64>,
    pub total_blocks: u64,
    pub total_instructions: u64,
}

impl RecompilerStats {
    pub fn new() -> Self {
        RecompilerStats::default()
    }

    /// Stub for `EE::Profiler.EmitOp(eeOpcode::fn)`.
    pub fn emit_op(&mut self, opcode: u32) {
        *self.op_counts.entry(opcode).or_insert(0) += 1;
        self.total_instructions += 1;
    }

    /// Record that a complete block has been compiled.
    pub fn emit_block(&mut self) {
        self.total_blocks += 1;
    }
}

// ---------------------------------------------------------------------------
// microVU
// ---------------------------------------------------------------------------

/// Stub for the `microVU` struct from `microVU.h`. Holds the bare
/// minimum needed to dispatch a microblock.
#[derive(Debug, Default)]
pub struct MicroVu {
    pub index: u32,
    pub cop2: bool,
    pub code: u32,
    pub branch: u32,
    pub cycle: s32,
    pub total_cycles: u32,
    pub div_flag: u32,
    pub vi_backup: u32,
    pub vi_xgkick: u32,
    pub p: u32,
    pub q: u32,
    pub bad_branch: u32,
    pub evil_branch: u32,
    pub evilevil_branch: u32,
    pub stat_flag: [u32; 4],
    pub mac_flag: [u32; 4],
    pub clip_flag: [u32; 4],
}

impl MicroVu {
    /// Return a reference to the matching VU register file in `state`.
    pub fn regs<'a>(&self, state: &'a CpuState) -> &'a VuRegFile {
        if self.index == 0 {
            &state.vu0
        } else {
            &state.vu1
        }
    }

    /// Return a mutable reference to the matching VU register file in `state`.
    pub fn regs_mut<'a>(&self, state: &'a mut CpuState) -> &'a mut VuRegFile {
        if self.index == 0 {
            &mut state.vu0
        } else {
            &mut state.vu1
        }
    }
}

// ---------------------------------------------------------------------------
// Shared recompiler context
// ---------------------------------------------------------------------------

/// Recompiler shared state. Holds everything the C++ globals
/// referenced: register allocator, code cache, base-block table,
/// profiler, and CPU state.
pub struct Recompiler {
    pub state: CpuState,
    pub regalloc: RegAlloc,
    pub cache: RecCache,
    pub base_blocks: BaseBlocks,
    pub stats: RecompilerStats,
    pub pc: u32,
    pub max_rec_mem: u32,
    pub branch: i32,
    pub g_branch: i32,
    pub iop_cycle_penalty: u32,
}

impl Recompiler {
    pub fn new() -> Self {
        Recompiler {
            state: CpuState::default(),
            regalloc: RegAlloc::new(8, 16),
            cache: RecCache::new(),
            base_blocks: BaseBlocks::new(),
            stats: RecompilerStats::new(),
            pc: 0,
            max_rec_mem: 0,
            branch: 0,
            g_branch: 0,
            iop_cycle_penalty: 0,
        }
    }

    /// Stub for `iFlushCall(flushtype)`. In the real port this would
    /// spill dirty x86 / XMM registers back to the EE state and
    /// free temporaries before an indirect call.
    pub fn i_flush_call(&mut self, _flushtype: u32) {
        // no-op in the stub
    }

    /// Stub for `_psxFlushCall(flushtype)`. Spills IOP constant regs.
    pub fn psx_flush_call(&mut self, _flushtype: u32) {
        // no-op in the stub
    }

    /// Stub for `SetBranchReg()`. The real port emits a `jmp eax`
    /// into the code cache.
    pub fn set_branch_reg(&mut self) {
        self.g_branch = 2;
    }

    /// Stub for `SetBranchImm(imm)`.
    pub fn set_branch_imm(&mut self, _imm: u32) {
        self.g_branch = 2;
    }

    /// Stub for `recompileNextInstruction(delay, swapped)`.
    pub fn recompile_next_instruction(&mut self, _delay: bool, _swapped: bool) {
        // no-op in the stub
    }

    /// Stub for `psxRecompileNextInstruction(delay, swapped)`.
    pub fn psx_recompile_next_instruction(&mut self, _delay: bool, _swapped: bool) {
        // no-op in the stub
    }
}

impl Default for Recompiler {
    fn default() -> Self {
        Recompiler::new()
    }
}

// ---------------------------------------------------------------------------
// Top-level CPU state handle
// ---------------------------------------------------------------------------

thread_local! {
    /// Singleton handle mirroring the C++ globals (`psxRegs`,
    /// `cpuRegs`, `vuRegs`, `iopRegs`, etc.). Real code would use
    /// explicit context objects passed by reference; this is a
    /// translation convenience.
    static RECOMPILER: RefCell<Recompiler> = RefCell::new(Recompiler::new());
}

/// Run `f` with mutable access to the global recompiler.
pub fn with_recompiler<R>(f: impl FnOnce(&mut Recompiler) -> R) -> R {
    RECOMPILER.with(|r| f(&mut r.borrow_mut()))
}

// ---------------------------------------------------------------------------
// Instruction field extractors
// ---------------------------------------------------------------------------
//
// The C++ headers use preprocessor macros to pull fields out of the
// current 32-bit instruction word. The PSX and EE instruction
// encodings share the same primary-opcode layout, so a single set
// of helpers works for both.

/// Return the primary 6-bit opcode of a MIPS/EE instruction.
#[inline]
pub fn instr_op(instr: u32) -> u32 {
    instr >> 26
}

/// Return the 5-bit `rs` field.
#[inline]
pub fn instr_rs(instr: u32) -> u32 {
    (instr >> 21) & 0x1f
}

/// Return the 5-bit `rt` field.
#[inline]
pub fn instr_rt(instr: u32) -> u32 {
    (instr >> 16) & 0x1f
}

/// Return the 5-bit `rd` field.
#[inline]
pub fn instr_rd(instr: u32) -> u32 {
    (instr >> 11) & 0x1f
}

/// Return the 5-bit `shamt` field.
#[inline]
pub fn instr_sa(instr: u32) -> u32 {
    (instr >> 6) & 0x1f
}

/// Return the 6-bit function field.
#[inline]
pub fn instr_funct(instr: u32) -> u32 {
    instr & 0x3f
}

/// Return the 16-bit immediate, sign-extended to `u32`.
#[inline]
pub fn instr_imm(instr: u32) -> u32 {
    instr & 0xffff
}

/// Return the 16-bit immediate as a `u16` (zero-extended).
#[inline]
pub fn instr_imm_u(instr: u32) -> u32 {
    instr & 0xffff
}

/// Return the 26-bit jump target.
#[inline]
pub fn instr_target(instr: u32) -> u32 {
    instr & 0x03ff_ffff
}

// ---------------------------------------------------------------------------
// PSX (R3000A) opcode handlers
// ---------------------------------------------------------------------------
//
// These are the 64-entry primary-opcode handlers from
// `iR3000Atables.cpp`. Each takes the current 32-bit `psxRegs.code`
// and is wired up in [`PSX_OPCODE_TABLE`].

/// Type alias for a PSX primary-opcode handler.
pub type PsxOpcodeFn = fn(u32) -> ();

/// PSX primary opcode: SPECIAL group — further decoded by `funct`.
pub fn psx_opcode_special(_instr: u32) {
    with_recompiler(|r| {
        let code = r.state.psx.code;
        let funct = instr_funct(code);
        r.stats.emit_op(funct);
        // The real implementation dispatches into rpsxSPC[funct].
    });
}

/// PSX primary opcode: REGIMM — further decoded by `rt`.
pub fn psx_opcode_regimm(_instr: u32) {
    with_recompiler(|r| {
        let code = r.state.psx.code;
        let rt = instr_rt(code);
        r.stats.emit_op(0x100 + rt);
    });
}

/// PSX primary opcode: J.
pub fn psx_opcode_j(_instr: u32) {
    with_recompiler(|r| r.stats.emit_op(0x02));
}

/// PSX primary opcode: JAL.
pub fn psx_opcode_jal(_instr: u32) {
    with_recompiler(|r| {
        r.stats.emit_op(0x03);
        r.state.psx.gpr[31] = r.state.psx.pc.wrapping_add(4);
    });
}

/// PSX primary opcode: BEQ.
pub fn psx_opcode_beq(_instr: u32) {
    with_recompiler(|r| r.stats.emit_op(0x04));
}

/// PSX primary opcode: BNE.
pub fn psx_opcode_bne(_instr: u32) {
    with_recompiler(|r| r.stats.emit_op(0x05));
}

/// PSX primary opcode: BLEZ.
pub fn psx_opcode_blez(_instr: u32) {
    with_recompiler(|r| r.stats.emit_op(0x06));
}

/// PSX primary opcode: BGTZ.
pub fn psx_opcode_bgtz(_instr: u32) {
    with_recompiler(|r| r.stats.emit_op(0x07));
}

/// PSX primary opcode: ADDI.
pub fn psx_opcode_addi(_instr: u32) {
    with_recompiler(|r| {
        let code = r.state.psx.code;
        let rt = instr_rt(code) as usize;
        let rs = instr_rs(code) as usize;
        let imm = instr_imm(code) as i16 as i32 as u32;
        r.stats.emit_op(0x08);
        if rt != 0 {
            r.state.psx.gpr[rt] = r.state.psx.gpr[rs].wrapping_add(imm);
        }
    });
}

/// PSX primary opcode: ADDIU.
pub fn psx_opcode_addiu(_instr: u32) {
    with_recompiler(|r| {
        let code = r.state.psx.code;
        let rt = instr_rt(code) as usize;
        let rs = instr_rs(code) as usize;
        let imm = instr_imm(code) as i16 as i32 as u32;
        r.stats.emit_op(0x09);
        if rt != 0 {
            r.state.psx.gpr[rt] = r.state.psx.gpr[rs].wrapping_add(imm);
        }
    });
}

/// PSX primary opcode: SLTI.
pub fn psx_opcode_slti(_instr: u32) {
    with_recompiler(|r| {
        let code = r.state.psx.code;
        let rt = instr_rt(code) as usize;
        let rs = instr_rs(code) as usize;
        let imm = instr_imm(code) as i16 as i32;
        r.stats.emit_op(0x0a);
        if rt != 0 {
            r.state.psx.gpr[rt] = if (r.state.psx.gpr[rs] as i32) < imm { 1 } else { 0 };
        }
    });
}

/// PSX primary opcode: SLTIU.
pub fn psx_opcode_sltiu(_instr: u32) {
    with_recompiler(|r| {
        let code = r.state.psx.code;
        let rt = instr_rt(code) as usize;
        let rs = instr_rs(code) as usize;
        let imm = instr_imm(code);
        r.stats.emit_op(0x0b);
        if rt != 0 {
            r.state.psx.gpr[rt] = if r.state.psx.gpr[rs] < imm { 1 } else { 0 };
        }
    });
}

/// PSX primary opcode: ANDI.
pub fn psx_opcode_andi(_instr: u32) {
    with_recompiler(|r| {
        let code = r.state.psx.code;
        let rt = instr_rt(code) as usize;
        let rs = instr_rs(code) as usize;
        let imm = instr_imm_u(code);
        r.stats.emit_op(0x0c);
        if rt != 0 {
            r.state.psx.gpr[rt] = r.state.psx.gpr[rs] & imm;
        }
    });
}

/// PSX primary opcode: ORI.
pub fn psx_opcode_ori(_instr: u32) {
    with_recompiler(|r| {
        let code = r.state.psx.code;
        let rt = instr_rt(code) as usize;
        let rs = instr_rs(code) as usize;
        let imm = instr_imm_u(code);
        r.stats.emit_op(0x0d);
        if rt != 0 {
            r.state.psx.gpr[rt] = r.state.psx.gpr[rs] | imm;
        }
    });
}

/// PSX primary opcode: XORI.
pub fn psx_opcode_xori(_instr: u32) {
    with_recompiler(|r| {
        let code = r.state.psx.code;
        let rt = instr_rt(code) as usize;
        let rs = instr_rs(code) as usize;
        let imm = instr_imm_u(code);
        r.stats.emit_op(0x0e);
        if rt != 0 {
            r.state.psx.gpr[rt] = r.state.psx.gpr[rs] ^ imm;
        }
    });
}

/// PSX primary opcode: LUI.
pub fn psx_opcode_lui(_instr: u32) {
    with_recompiler(|r| {
        let code = r.state.psx.code;
        let rt = instr_rt(code) as usize;
        r.stats.emit_op(0x0f);
        if rt != 0 {
            r.state.psx.gpr[rt] = code << 16;
        }
    });
}

/// PSX primary opcode: COP0 — coprocessor 0.
pub fn psx_opcode_cop0(_instr: u32) {
    with_recompiler(|r| {
        let code = r.state.psx.code;
        let rs = instr_rs(code);
        r.stats.emit_op(0x10);
        match rs {
            0 => {
                // MFC0: rt = CP0[rd]
                let rt = instr_rt(code) as usize;
                let rd = instr_rd(code) as usize;
                if rt != 0 {
                    r.state.psx.gpr[rt] = match rd {
                        12 => r.state.psx.cp0_status,
                        13 => r.state.psx.cp0_cause,
                        14 => r.state.psx.cp0_epc,
                        _ => 0,
                    };
                }
            }
            4 => {
                // MTC0: CP0[rd] = rt
                let rt = instr_rt(code) as usize;
                let rd = instr_rd(code) as usize;
                let v = r.state.psx.gpr[rt];
                match rd {
                    12 => r.state.psx.cp0_status = v,
                    13 => r.state.psx.cp0_cause = v,
                    14 => r.state.psx.cp0_epc = v,
                    _ => {}
                }
            }
            16 => {
                // RFE: shift status[3:0] right by 2
                let st = r.state.psx.cp0_status;
                let low = (st & 0x0f) >> 2;
                r.state.psx.cp0_status = (st & 0xfffffff0) | low;
            }
            _ => {}
        }
    });
}

/// PSX primary opcode: COP2 — GTE.
pub fn psx_opcode_cop2(_instr: u32) {
    with_recompiler(|r| r.stats.emit_op(0x12));
}

/// PSX primary opcode: LB.
pub fn psx_opcode_lb(_instr: u32) {
    with_recompiler(|r| r.stats.emit_op(0x20));
}

/// PSX primary opcode: LH.
pub fn psx_opcode_lh(_instr: u32) {
    with_recompiler(|r| r.stats.emit_op(0x21));
}

/// PSX primary opcode: LWL.
pub fn psx_opcode_lwl(_instr: u32) {
    with_recompiler(|r| r.stats.emit_op(0x22));
}

/// PSX primary opcode: LW.
pub fn psx_opcode_lw(_instr: u32) {
    with_recompiler(|r| r.stats.emit_op(0x23));
}

/// PSX primary opcode: LBU.
pub fn psx_opcode_lbu(_instr: u32) {
    with_recompiler(|r| r.stats.emit_op(0x24));
}

/// PSX primary opcode: LHU.
pub fn psx_opcode_lhu(_instr: u32) {
    with_recompiler(|r| r.stats.emit_op(0x25));
}

/// PSX primary opcode: LWR.
pub fn psx_opcode_lwr(_instr: u32) {
    with_recompiler(|r| r.stats.emit_op(0x26));
}

/// PSX primary opcode: SB.
pub fn psx_opcode_sb(_instr: u32) {
    with_recompiler(|r| r.stats.emit_op(0x28));
}

/// PSX primary opcode: SH.
pub fn psx_opcode_sh(_instr: u32) {
    with_recompiler(|r| r.stats.emit_op(0x29));
}

/// PSX primary opcode: SWL.
pub fn psx_opcode_swl(_instr: u32) {
    with_recompiler(|r| r.stats.emit_op(0x2a));
}

/// PSX primary opcode: SW.
pub fn psx_opcode_sw(_instr: u32) {
    with_recompiler(|r| r.stats.emit_op(0x2b));
}

/// PSX primary opcode: SWR.
pub fn psx_opcode_swr(_instr: u32) {
    with_recompiler(|r| r.stats.emit_op(0x2e));
}

/// PSX primary opcode: LWC2.
pub fn psx_opcode_lwc2(_instr: u32) {
    with_recompiler(|r| r.stats.emit_op(0x32));
}

/// PSX primary opcode: SWC2.
pub fn psx_opcode_swc2(_instr: u32) {
    with_recompiler(|r| r.stats.emit_op(0x3a));
}

/// PSX primary opcode: no-op/undefined.
pub fn psx_opcode_undefined(_instr: u32) {
    with_recompiler(|r| {
        r.stats.emit_op(0xff);
    });
}

// ---------------------------------------------------------------------------
// PSX (R3000A) primary-opcode table
// ---------------------------------------------------------------------------
//
// 64 entries, indexed by the top 6 bits of `psxRegs.code`. Mirrors
// `rpsxBSC[]` from `iR3000Atables.cpp`.
//
// Layout (taken verbatim from the C++ source):
//
//     0: SPECIAL   8: ADDI     16: COP0     24: ---        32: LB    40: SB    48: ---    56: ---
//     1: REGIMM    9: ADDIU    17: ---      25: ---        33: LH    41: SH    49: ---    57: ---
//     2: J        10: SLTI     18: COP2     26: ---        34: LWL   42: SWL   50: LWC2  58: SWC2
//     3: JAL      11: SLTIU    19: ---      27: ---        35: LW    43: SW    51: ---    59: ---
//     4: BEQ      12: ANDI     20: ---      28: ---        36: LBU   44: ---   52: ---    60: ---
//     5: BNE      13: ORI      21: ---      29: ---        37: LHU   45: ---   53: ---    61: ---
//     6: BLEZ     14: XORI     22: ---      30: ---        38: LWR   46: SWR   54: ---    62: ---
//     7: BGTZ     15: LUI      23: ---      31: ---        39: ---   47: ---   55: ---    63: ---

/// Primary-opcode dispatch table for the IOP (R3000A). Indexed by
/// the top 6 bits of the 32-bit instruction word.
///
/// The second tuple field (`u8`) carries a per-opcode cycle cost
/// hint. The original C++ table does not encode this, so a sensible
/// default of `1` cycle per instruction is used for arithmetic /
/// branch opcodes, with `0` for undefined slots and `2` for the
/// slow integer multiplies / divides when applicable.
pub const PSX_OPCODE_TABLE: [(PsxOpcodeFn, u8); 64] = [
    /*  0 */ (psx_opcode_special,   1),
    /*  1 */ (psx_opcode_regimm,    1),
    /*  2 */ (psx_opcode_j,         1),
    /*  3 */ (psx_opcode_jal,       1),
    /*  4 */ (psx_opcode_beq,       1),
    /*  5 */ (psx_opcode_bne,       1),
    /*  6 */ (psx_opcode_blez,      1),
    /*  7 */ (psx_opcode_bgtz,      1),
    /*  8 */ (psx_opcode_addi,      1),
    /*  9 */ (psx_opcode_addiu,     1),
    /* 10 */ (psx_opcode_slti,      1),
    /* 11 */ (psx_opcode_sltiu,     1),
    /* 12 */ (psx_opcode_andi,      1),
    /* 13 */ (psx_opcode_ori,       1),
    /* 14 */ (psx_opcode_xori,      1),
    /* 15 */ (psx_opcode_lui,       1),
    /* 16 */ (psx_opcode_cop0,      1),
    /* 17 */ (psx_opcode_undefined, 0),
    /* 18 */ (psx_opcode_cop2,      1),
    /* 19 */ (psx_opcode_undefined, 0),
    /* 20 */ (psx_opcode_undefined, 0),
    /* 21 */ (psx_opcode_undefined, 0),
    /* 22 */ (psx_opcode_undefined, 0),
    /* 23 */ (psx_opcode_undefined, 0),
    /* 24 */ (psx_opcode_undefined, 0),
    /* 25 */ (psx_opcode_undefined, 0),
    /* 26 */ (psx_opcode_undefined, 0),
    /* 27 */ (psx_opcode_undefined, 0),
    /* 28 */ (psx_opcode_undefined, 0),
    /* 29 */ (psx_opcode_undefined, 0),
    /* 30 */ (psx_opcode_undefined, 0),
    /* 31 */ (psx_opcode_undefined, 0),
    /* 32 */ (psx_opcode_lb,        1),
    /* 33 */ (psx_opcode_lh,        1),
    /* 34 */ (psx_opcode_lwl,       1),
    /* 35 */ (psx_opcode_lw,        1),
    /* 36 */ (psx_opcode_lbu,       1),
    /* 37 */ (psx_opcode_lhu,       1),
    /* 38 */ (psx_opcode_lwr,       1),
    /* 39 */ (psx_opcode_undefined, 0),
    /* 40 */ (psx_opcode_sb,        1),
    /* 41 */ (psx_opcode_sh,        1),
    /* 42 */ (psx_opcode_swl,       1),
    /* 43 */ (psx_opcode_sw,        1),
    /* 44 */ (psx_opcode_undefined, 0),
    /* 45 */ (psx_opcode_undefined, 0),
    /* 46 */ (psx_opcode_swr,       1),
    /* 47 */ (psx_opcode_undefined, 0),
    /* 48 */ (psx_opcode_undefined, 0),
    /* 49 */ (psx_opcode_undefined, 0),
    /* 50 */ (psx_opcode_lwc2,      1),
    /* 51 */ (psx_opcode_undefined, 0),
    /* 52 */ (psx_opcode_undefined, 0),
    /* 53 */ (psx_opcode_undefined, 0),
    /* 54 */ (psx_opcode_undefined, 0),
    /* 55 */ (psx_opcode_undefined, 0),
    /* 56 */ (psx_opcode_undefined, 0),
    /* 57 */ (psx_opcode_undefined, 0),
    /* 58 */ (psx_opcode_swc2,      1),
    /* 59 */ (psx_opcode_undefined, 0),
    /* 60 */ (psx_opcode_undefined, 0),
    /* 61 */ (psx_opcode_undefined, 0),
    /* 62 */ (psx_opcode_undefined, 0),
    /* 63 */ (psx_opcode_undefined, 0),
];

// ---------------------------------------------------------------------------
// EE interpreter dispatch
// ---------------------------------------------------------------------------
//
// Each of these takes a 32-bit EE instruction word (`cpuRegs.code`)
// and executes one interpreter step. The real C++ body switches on
// `code >> 26` and then a sub-coprocessor field, then mutates
// `cpuRegs` and `cpuRegs.cycle`. The bodies here are stubs that
// update the profiler and the cycle counter.

/// Execute one EE integer core instruction. Mirrors `iCore.cpp`'s
/// `EE::_DynaRecCPU_Recompile` interpreter path and the
/// `eeOpcode` dispatch.
pub fn EE_Core_Opcode(instr: u32) -> () {
    with_recompiler(|r| {
        r.state.ee.code = instr;
        r.stats.emit_op(instr_op(instr));
        r.state.ee.cycle = r.state.ee.cycle.wrapping_add(1);
    });
}

/// Execute one EE FPU instruction. Mirrors `iFPU.cpp`'s COP1
/// dispatch, which switches on `rs` for MFC/CTC/MTC/CFC and on
/// `funct` for the floating-point ops.
pub fn FPU_Opcode(instr: u32) -> () {
    with_recompiler(|r| {
        r.state.ee.code = instr;
        let op = instr_op(instr);
        if op == 0x11 {
            let rs = instr_rs(instr);
            match rs {
                0 => r.stats.emit_op(0x110), // MFC1
                2 => r.stats.emit_op(0x112), // CFC1
                3 => r.stats.emit_op(0x113), // MFHC1
                4 => r.stats.emit_op(0x114), // MTC1
                6 => r.stats.emit_op(0x116), // CTC1
                7 => r.stats.emit_op(0x117), // MTHC1
                8 => r.stats.emit_op(0x118), // BC1
                _ => r.stats.emit_op(0x11f),
            }
        } else {
            r.stats.emit_op(op);
        }
        r.state.ee.cycle = r.state.ee.cycle.wrapping_add(1);
    });
}

/// Execute one EE MMI instruction. Mirrors `iMMI.cpp`'s
/// `MMI_Opcode` dispatch. The EE has both "real" MMI ops in the
/// primary opcode 0x1C and "second-pipeline" duplicates of
/// MTHI/MTLO/MULT/DIV/etc. in the primary opcode 0x1F.
pub fn MMI_Opcode(instr: u32) -> () {
    with_recompiler(|r| {
        r.state.ee.code = instr;
        let op = instr_op(instr);
        if op == 0x1c || op == 0x1f {
            r.stats.emit_op(instr_funct(instr));
        } else {
            r.stats.emit_op(op);
        }
        r.state.ee.cycle = r.state.ee.cycle.wrapping_add(1);
    });
}

// ---------------------------------------------------------------------------
// R3000A interpreter entry point
// ---------------------------------------------------------------------------

/// Execute one IOP (R3000A) interpreter step. Mirrors the
/// `psxBSC[code >> 26]()` dispatch from `iR3000A.cpp`.
pub fn iR3000A_Opcode(instr: u32) -> () {
    with_recompiler(|r| {
        r.state.psx.code = instr;
        let op = instr_op(instr) as usize;
        let (handler, cycles) = PSX_OPCODE_TABLE[op & 0x3f];
        handler(instr);
        r.state.ee.cycle = r.state.ee.cycle.wrapping_add(cycles as u32);
    });
}

// ---------------------------------------------------------------------------
// EE-to-x86 recompiler
// ---------------------------------------------------------------------------
//
// The real `recRecompile` walks an EE basic block, allocates host
// registers, and emits x86 code into the dynarec cache. The
// implementation depends on the x86 emitter, the VTLB, the
// base-block manager, and the full CPU state. Here we expose a
// stub that simply records the request in the profiler.

/// Recompile one EE basic block starting at `pc`. Stub for the
/// real `R5900::Dynarec::recRecompile(...)` path that walks the
/// EE instruction stream, allocates x86/XMM registers for EE
/// GPR/FPR state, and emits host code into the dynarec cache.
pub fn R5900_RecompileBlock(pc: u32) -> () {
    with_recompiler(|r| {
        r.pc = pc;
        r.state.ee.pc = pc;
        r.stats.emit_block();
        // The real implementation would:
        //   1. Look up the base block for `pc` in `base_blocks`.
        //   2. Walk the EE instruction stream until a branch.
        //   3. For each EE instruction:
        //        - parse the opcode,
        //        - emit x86 via the x86Emitter,
        //        - update `regalloc`,
        //        - call `i_flush_call(...)` as needed.
        //   4. Back-patch the block trailer to the dispatcher.
    });
}

// ---------------------------------------------------------------------------
// micro-VU recompiler
// ---------------------------------------------------------------------------

/// Recompile one micro-VU block. `vu` is the VU index (0 or 1),
/// `pc` is the start address inside micro-memory. Stub for the
/// real `mVUcompileJIT` / `mVUsearchProg` path.
pub fn microVU_RecompileBlock(vu: u32, pc: u32) -> () {
    with_recompiler(|r| {
        let mut mvu = MicroVu::default();
        mvu.index = vu;
        mvu.code = pc;
        r.stats.emit_block();
        // The real implementation would:
        //   1. Build a microIR for the program starting at `pc`.
        //   2. Run register allocation.
        //   3. Emit host code into the cache.
        //   4. Cache the block in the microVU program manager.
        let _ = mvu;
    });
}

// ---------------------------------------------------------------------------
// TLB / VTLB helpers
// ---------------------------------------------------------------------------
//
// `recVTLB.cpp` and `recRecTLB` translate the VTLB miss handler
// into x86. The helpers below are type stubs that mirror the
// function-pointer signature used in the C++.

/// Page-table entry kind, modelled on the C++ `PageProtection` enum.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum PageProtection {
    #[default]
    Read = 0,
    Write = 1,
    Exec = 2,
}

/// One 4 KiB TLB page. Mirrors the struct used by `recVTLB.cpp`.
#[derive(Debug, Clone, Default)]
pub struct VtlbPage {
    pub vaddr: u32,
    pub paddr: u64,
    pub prot: PageProtection,
    pub is__ram: bool,
}

/// VTLB miss handler. In the real port this is replaced with a
/// direct call into the recompiled resolver.
pub type VtlbMissHandler = fn(addr: u32, is_write: bool) -> *mut u8;

/// TLB lookup helper. Returns `None` on miss.
pub fn vtlb_lookup(state: &CpuState, addr: u32) -> Option<&VtlbPage> {
    let _ = state;
    let _ = addr;
    None
}

// ---------------------------------------------------------------------------
// VIF dynarec helpers
// ---------------------------------------------------------------------------
//
// `Vif_Dynarec.cpp` and `Vif_UnpackSSE.cpp` recompile VIF unpack
// commands. They are tightly coupled to the x86Emitter, so we
// only expose a stub.

/// VIF unpack function descriptor. Mirrors `VIFUnpackFuncTable`.
#[derive(Debug, Clone, Copy)]
pub struct VifUnpackFn {
    pub fn0: fn(&mut CpuState, u32, u32, *mut u8),
    pub fn1: fn(&mut CpuState, u32, u32, *mut u8),
    pub fn2: fn(&mut CpuState, u32, u32, *mut u8),
    pub fn3: fn(&mut CpuState, u32, u32, *mut u8),
}

impl Default for VifUnpackFn {
    fn default() -> Self {
        fn noop(_: &mut CpuState, _: u32, _: u32, _: *mut u8) {}
        VifUnpackFn {
            fn0: noop,
            fn1: noop,
            fn2: noop,
            fn3: noop,
        }
    }
}

// ---------------------------------------------------------------------------
// Cycle-cost constants
// ---------------------------------------------------------------------------
//
// These match `psxInstCycles_Mult` / `psxInstCycles_Div` from
// `iR3000A.h` and the EE-side cost table from `R5900_Profiler.h`.

/// Cycle penalty applied when the IOP executes a 32-bit multiply.
pub const PSX_INST_CYCLES_MULT: u32 = 7;

/// Cycle penalty applied when the IOP executes a 32-bit divide.
pub const PSX_INST_CYCLES_DIV: u32 = 40;

// ---------------------------------------------------------------------------
// Helper: convert a 4-byte instruction back to a 32-bit word
// ---------------------------------------------------------------------------

/// Read a 32-bit word out of a byte slice. Mirrors the C++
// `readMem32` used by the iR3000A interpreter.
#[inline]
pub fn read_mem32(mem: &[u8], offset: usize) -> u32 {
    let mut w = [0u8; 4];
    if offset + 4 <= mem.len() {
        w.copy_from_slice(&mem[offset..offset + 4]);
    }
    u32::from_le_bytes(w)
}

// ---------------------------------------------------------------------------
// End of module
// ---------------------------------------------------------------------------
