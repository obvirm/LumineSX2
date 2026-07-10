// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust translation of `pcsx2/x86/ix86-32/iCore.cpp` and
//! `pcsx2/x86/ix86-32/iR5900.cpp` — the IA-32 portion of the EE
//! recompiler.
//!
//! Together these two files form the heart of PCSX2's R5900 (EE)
//! dynamic-recompiler backend: an x86 host-side register allocator
//! layered on top of the `x86Emitter`, plus the dispatcher and
//! block-recompilation entry points that translate EE basic blocks
//! into i386 machine code.
//!
//! # Module layout
//!
//! 1. **Primitive aliases** — `u8` / `u32` / `uptr` etc. that match the
//!    C++ `Common.h` typedefs so the rest of the file reads 1:1 with the
//!    source.
//! 2. **Recompiler types** — the `X86Type` enum, `X86Reg` / `XmmReg`
//!    descriptors, allocation mode flags, and `FLUSH_*` flush flags.
//! 3. **CPU register-file stubs** — `EeRegFile`, `FpuRegFile`,
//!    `VuRegFile`, `PsxRegFile`, plus the union-style 128-bit
//!    `GprReg64` helpers the C++ code uses to peek at 32 / 64 / 128 bit
//!    views of an EE GPR.
//! 4. **x86 register allocation** — the `RegAlloc` struct and the
//!    `_initX86regs`, `_getFreeX86reg`, `_allocX86reg`, `_checkX86reg`,
//!    `_addNeededX86reg`, `_clearNeededX86regs`, `_freeX86reg`,
//!    `_freeX86regWithoutWriteback`, `_freeX86regs`, `_flushX86regs`
//!    entry points from `iCore.cpp`.
//! 5. **Constant-register flushing** — `_flushConstReg`, `_flushConstRegs`,
//!    `_eeMoveGPRtoR` and `_eeMoveGPRtoM`, which together form the EE
//!    constant-propagation engine.
//! 6. **Validation** — `_validateRegs`, the dev-build sanity check that
//!    no EE GPR/FPR is in write mode in two host registers at once.
//! 7. **EE 32-bit recompiler entry points** — `recReserve`,
//!    `recResetRaw`, `recShutdown`, `recStep`, `recExecute`,
//!    `recRecompile`, `recClear`, the `_DynGen_*` dispatchers
//!    (`_DynGen_DispatcherEvent`, `_DynGen_DispatcherReg`,
//!    `_DynGen_JITCompile`, `_DynGen_EnterRecompiledCode`,
//!    `_DynGen_DispatchBlockDiscard`, `_DynGen_DispatchPageReset`,
//!    `_DynGen_UnmappedRecLUTPage`), the `SetBranchReg` /
//!    `SetBranchImm` / `iBranchTest` helpers, the cycle-scaling
//!    helpers `scaleblockcycles_calculation` / `scaleblockcycles` /
//!    `scaleblockcycles_clear`, the
//!    `recompileNextInstruction` driver, the SYSCALL / BREAK
//!    opcode emitters, the COP2 timing / detection helpers
//!    (`cop2flags`, `COP2DivUnitTimings`, `COP2IsQOP`), the
//!    `iFlushCall` ABI-register-spiller, the branch-state
//!    `SaveBranchState` / `LoadBranchState` pair, the
//!    `recBranchCall` / `recCall` call-site helpers, the
//!    delay-slot swapper `TrySwapDelaySlot`, the breakpoints
//!    glue (`dynarecCheckBreakpoint`, `dynarecMemcheck`,
//!    `recMemcheck`, `encodeBreakpoint`, `encodeMemcheck`), the
//!    `recBeginThunk` / `recEndThunk` cache advance helpers, the
//!    `recSafeExitExecution` / `recExitExecution` longjmp wrapper,
//!    the manual protection routines
//!    `memory_protect_recompiled_code`, `dyna_block_discard`,
//!    `dyna_page_reset`, the special `skipMPEG_By_Pattern` and
//!    `recSkipTimeoutLoop` speedhacks, the EE register-file move
//!    helpers, the `recCpu` vtable, and finally the
//!    `_eeFlushAllDirty` top-level flush.
//!
//! # Scope
//!
//! The original C++ emits real x86 instructions through the
//! `x86Emitter` API (`xXOR`, `xMOV`, `xFastCall`, `xJMP`,
//! `xForwardJNZ32`, ...). Those calls are represented in this Rust
//! translation by thin **stub functions** with the same name and
//! signature as the C++ emitter functions, but that record their
//! intent rather than writing machine code. This keeps the module
//! `std`-only and free of unsafe code, while preserving the structure
//! of `iCore.cpp` / `iR5900.cpp` line-for-line where possible.
//!
//! In a real port, the stubs would be replaced by calls into the
//! emitter defined in `rust/translations/common/emitter/X86Emitter.rs`
//! and the data would live in thread-local state. The translation
//! here is the *bridge*: it keeps the C++ semantic surface but is
//! idiomatic Rust, ready to be wired to a real emitter or to be
//! inspected standalone.

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(clippy::too_many_arguments)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt;

// ---------------------------------------------------------------------------
// Primitive aliases
// ---------------------------------------------------------------------------
//
// The original C++ uses `u8`, `u16`, `u32`, `u64`, `s8`, `s16`, `s32`,
// `s64`, `uptr`, `sptr` from its `Common.h`. We keep the names so the
// port reads 1:1 with the source.

pub type u8 = ::std::primitive::u8;
pub type u16 = ::std::primitive::u16;
pub type u32 = ::std::primitive::u32;
pub type u64 = ::std::primitive::u64;
pub type s8 = ::std::primitive::i8;
pub type s16 = ::std::primitive::i16;
pub type s32 = ::std::primitive::i32;
pub type s64 = ::std::primitive::i64;

/// Unsigned pointer-sized integer. In the C++ code this is `uptr`.
pub type uptr = usize;

/// Signed pointer-sized integer. In the C++ code this is `sptr`.
pub type sptr = isize;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// One 64 KiB window (16-bit page address space).
pub const _64KB: usize = 0x10000;

/// Maximum number of host GPRs the IA-32 backend tracks. The C++
/// `iREGCNT_GPR` is 8 (eax, ecx, edx, ebx, esi, edi, ebp, esp used
/// only indirectly).
pub const IREG_COUNT_GPR: usize = 8;

/// Maximum number of host XMM registers the IA-32 backend tracks. The
/// C++ `iREGCNT_XMM` is 8 (xmm0..xmm7).
pub const IREG_COUNT_XMM: usize = 8;

/// Maximum size in instructions of an EE recompiled block. The C++
/// code only uses this in `pxAssert`s.
pub const MAX_BLOCK_INSTS: u32 = 0xffff;

// ---------------------------------------------------------------------------
// x86 register-allocation mode bits
// ---------------------------------------------------------------------------

/// Mark a host register as read by the current instruction.
pub const MODE_READ: u32 = 0x1;
/// Mark a host register as written by the current instruction.
pub const MODE_WRITE: u32 = 0x2;
/// The register's contents must survive a call (callee-saved).
pub const MODE_CALLEESAVED: u32 = 0x20;
/// The host register is reserved for COP2 (VU0) use.
pub const MODE_COP2: u32 = 0x40;

// ---------------------------------------------------------------------------
// Tag describing what a host x86 register is currently mapped to.
// ---------------------------------------------------------------------------
//
// Mirrors the C++ `enum x86type` from `iCore.h`. Every host register
// the IA-32 backend caches is tagged with one of these values so the
// next instruction knows which guest state lives where.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum X86Type {
    /// Host-owned temporary; no guest state.
    Temp = 0,
    /// Caches an EE GPR.
    Gpr = 1,
    /// Caches an FPU control register (FPCR / FPU CCR / FPU CSR).
    Fprc = 2,
    /// Caches a VU0 VI (integer) register.
    ViReg = 3,
    /// Caches the EE PC writeback address.
    PcWriteback = 4,
    /// Caches an IOP/R3000A GPR.
    Psx = 5,
    /// Caches the IOP PC writeback address.
    PsxPcWriteback = 6,
}

impl Default for X86Type {
    fn default() -> Self {
        X86Type::Temp
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
// x86 GPR / XMM register state
// ---------------------------------------------------------------------------

/// Cached IA-32 host GPR allocation entry. Mirrors the C++
/// `_x86regs` struct.
#[derive(Debug, Clone)]
pub struct X86Reg {
    pub in_use: bool,
    pub reg: s8,
    pub mode: u8,
    pub needed: bool,
    pub ty: X86Type,
    pub counter: u16,
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
        }
    }
}

/// Cached IA-32 host XMM allocation entry.
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
// 128-bit EE GPR / FPR register view
// ---------------------------------------------------------------------------
//
// In C++ these are laid out as a union: 16 bytes can be viewed as
// `UD[2]` (two u64), `SD[2]` (two i64), `US[4]` (four u16) and so on.
// We replicate the same memory layout with `MaybeUninit<u8>` to keep
// the file `std`-only and allocation-free.

/// Helper: a 16-byte storage that the EE recompiler overlays with
/// various integer views. We expose the same accessors the C++ code
/// uses.
#[derive(Clone, Copy)]
pub struct GprReg64 {
    bytes: [u8; 16],
}

impl Default for GprReg64 {
    fn default() -> Self {
        GprReg64 { bytes: [0u8; 16] }
    }
}

impl fmt::Debug for GprReg64 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "GprReg64({:02x?})", &self.bytes[..])
    }
}

impl GprReg64 {
    /// Low 8 bytes as a `u64`.
    #[inline]
    pub fn ud0(&self) -> u64 {
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&self.bytes[..8]);
        u64::from_le_bytes(buf)
    }

    /// High 8 bytes as a `u64`.
    #[inline]
    pub fn ud1(&self) -> u64 {
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&self.bytes[8..]);
        u64::from_le_bytes(buf)
    }

    /// Low 8 bytes as an `i64` (signed view).
    #[inline]
    pub fn sd0(&self) -> i64 {
        self.ud0() as i64
    }

    /// High 8 bytes as an `i64` (signed view).
    #[inline]
    pub fn sd1(&self) -> i64 {
        self.ud1() as i64
    }

    /// Lowest 4 bytes as a `u32`.
    #[inline]
    pub fn ul0(&self) -> u32 {
        let mut buf = [0u8; 4];
        buf.copy_from_slice(&self.bytes[..4]);
        u32::from_le_bytes(buf)
    }

    /// 4 bytes at `[4..8]` as a `u32`.
    #[inline]
    pub fn ul1(&self) -> u32 {
        let mut buf = [0u8; 4];
        buf.copy_from_slice(&self.bytes[4..8]);
        u32::from_le_bytes(buf)
    }

    /// Lowest byte as a `u8`.
    #[inline]
    pub fn uc0(&self) -> u8 {
        self.bytes[0]
    }

    /// Set the low 8 bytes.
    #[inline]
    pub fn set_ud0(&mut self, v: u64) {
        self.bytes[..8].copy_from_slice(&v.to_le_bytes());
    }
}

// ---------------------------------------------------------------------------
// CPU register-file stubs
// ---------------------------------------------------------------------------
//
// The real definitions live in `cpuRegs.h`, `psxRegs.h`, `VURegs.h`,
// `IopGte.h`, etc. We only model the fields the dispatcher entry
// points actually touch.

/// EE GPR file. Mirrors `cpuRegs.GPR`.
#[derive(Debug, Clone)]
pub struct EeGprFile {
    pub r: [GprReg64; 34],
}

impl Default for EeGprFile {
    fn default() -> Self {
        EeGprFile {
            r: [GprReg64::default(); 34],
        }
    }
}

/// EE (R5900) register file. Mirrors `cpuRegs`.
#[derive(Debug, Default, Clone)]
pub struct EeRegFile {
    pub gpr: EeGprFile,
    pub hi: GprReg64,
    pub lo: GprReg64,
    pub sa: u32,
    pub pc: u32,
    pub cycle: u32,
    pub next_event_cycle: u32,
    pub code: u32,
    pub branch: i32,
    pub target: u32,
    pub pc_writeback: u32,
    pub gpr_dirty: u32,
    pub cp0_status: u32,
    pub cp0_cause: u32,
    pub cp0_epc: u32,
    /// EE-side CP0 configuration register.
    pub cp0_config: u32,
}

/// FPU control / FPR file. Mirrors `fpuRegs`.
#[derive(Debug, Default, Clone)]
pub struct FpuRegFile {
    pub fpr: [GprReg64; 32],
    pub fprc: [u32; 32],
    pub acc: GprReg64,
}

/// COP2 / VU register file (VU0 only — VU1 has its own state).
#[derive(Debug, Default, Clone)]
pub struct VuRegFile {
    pub vf: [GprReg64; 32],
    pub vi: [u16; 32],
    pub acc: GprReg64,
    pub q: GprReg64,
    pub p: GprReg64,
    pub idx: u32,
    pub mac_flag: [u32; 4],
    pub clip_flag: [u32; 4],
    pub stat_flag: [u32; 4],
}

/// IOP (R3000A) register file. Mirrors `psxRegs`.
#[derive(Debug, Default, Clone)]
pub struct PsxRegFile {
    pub gpr: [u32; 32],
    pub pc: u32,
    pub code: u32,
    pub pc_writeback: u32,
    pub cycle: u32,
    pub iop_break: u32,
    pub iop_cycle_ee: u32,
    pub cp0_status: u32,
    pub cp0_cause: u32,
    pub cp0_epc: u32,
}

/// Process-wide CPU state. Mirrors the C++ globals (`cpuRegs`,
/// `psxRegs`, `VU0`, `fpuRegs`, etc.).
#[derive(Debug, Default)]
pub struct CpuState {
    pub ee: EeRegFile,
    pub fpu: FpuRegFile,
    pub vu0: VuRegFile,
    pub psx: PsxRegFile,
    /// EE constant-propagation cache (`g_cpuConstRegs`).
    pub cpu_const_regs: [GprReg64; 32],
    /// EE constant mask: which GPRs currently have a known constant
    /// (`g_cpuHasConstReg`).
    pub cpu_has_const: u32,
    /// EE constant-flushed mask (`g_cpuFlushedConstReg`).
    pub cpu_flushed_const: u32,
    /// IOP constant-propagation cache (`g_psxConstRegs`).
    pub psx_const_regs: [u32; 32],
    pub psx_has_const: u32,
    pub psx_flushed_const: u32,
    /// True while the EE interpreter or recompiler is running
    /// (`eeCpuExecuting`).
    pub ee_cpu_executing: bool,
    /// Pending reset flag (`eeRecNeedsReset`).
    pub ee_rec_needs_reset: bool,
    /// Set when the recompiler is currently compiling a delay-slot
    /// instruction (`g_recompilingDelaySlot`).
    pub recompiling_delay_slot: bool,
    /// Set once `cpuRegs.pc` has been written to memory
    /// (`g_cpuFlushedPC`).
    pub cpu_flushed_pc: bool,
    /// Set once `cpuRegs.code` has been written to memory
    /// (`g_cpuFlushedCode`).
    pub cpu_flushed_code: bool,
    /// Set when a recent instruction may have signalled an EE
    /// exception (`g_maySignalException`).
    pub may_signal_exception: bool,
    /// Pending `recSafeExitExecution` request (`eeRecExitRequested`).
    pub ee_rec_exit_requested: bool,
    /// Set when an EE event test is currently in progress
    /// (`eeEventTestIsActive`).
    pub ee_event_test_active: bool,
}

// ---------------------------------------------------------------------------
// Macro: helper for the C++ `GPR_IS_CONST1` / `PSX_IS_CONST1` checks.
// ---------------------------------------------------------------------------
//
// The C++ uses preprocessor bit tests. Here we keep them as `const
// fn`s so the same logic is available in normal Rust code.

#[inline]
pub const fn gpr_is_const1(mask: u32, gpr: usize) -> bool {
    (mask & (1u32 << gpr as u32)) != 0
}

#[inline]
pub const fn psx_is_const1(mask: u32, gpr: usize) -> bool {
    (mask & (1u32 << gpr as u32)) != 0
}

#[inline]
pub const fn gpr_is_dirty_const(state: &CpuState, gpr: usize) -> bool {
    gpr_is_const1(state.cpu_has_const, gpr) && !gpr_is_const1(state.cpu_flushed_const, gpr)
}

#[inline]
pub const fn psx_is_dirty_const(state: &CpuState, gpr: usize) -> bool {
    psx_is_const1(state.psx_has_const, gpr) && !psx_is_const1(state.psx_flushed_const, gpr)
}

/// Mirrors `GPR_DEL_CONST(reg)`: clear the constant flag for `gpr`.
#[inline]
pub fn gpr_del_const(state: &mut CpuState, gpr: usize) {
    state.cpu_has_const &= !(1u32 << gpr as u32);
}

/// Mirrors `PSX_DEL_CONST(reg)`.
#[inline]
pub fn psx_del_const(state: &mut CpuState, gpr: usize) {
    state.psx_has_const &= !(1u32 << gpr as u32);
}

// ---------------------------------------------------------------------------
// x86 emitter stubs
// ---------------------------------------------------------------------------
//
// The C++ code calls into `x86Emitter` (`xXOR`, `xMOV`, `xFastCall`,
// `xJMP`, ...). Those functions write real machine code. Here we
// provide stand-ins with the same names and signatures, so the
// translation reads 1:1 with the source but stays `std`-only.
//
// In a real port, replace these stubs with the corresponding emitter
// calls. Each stub records the call in `EMIT_LOG` so unit tests can
// verify ordering.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XReg(pub u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XmmRegId(pub u8);

/// A 16 / 32 / 64-bit host GPR reference (the C++ uses 32-bit
/// `xRegister32` and 64-bit `xRegister64` overloads — here we just
/// carry the host reg id).
#[derive(Debug, Clone, Copy)]
pub struct XRegister32(pub u8);

impl XRegister32 {
    pub fn id(&self) -> u8 {
        self.0
    }
    pub fn is_caller_saved(id: u8) -> bool {
        // In the 32-bit SysV ABI: eax, ecx, edx are caller-saved.
        matches!(id, 0 | 1 | 2)
    }
}

/// 64-bit host GPR reference.
#[derive(Debug, Clone, Copy)]
pub struct XRegister64(pub u8);

impl XRegister64 {
    pub fn id(&self) -> u8 {
        self.0
    }
    pub fn as_32(&self) -> XRegister32 {
        XRegister32(self.0)
    }
    pub fn is_caller_saved(id: u8) -> bool {
        XRegister32::is_caller_saved(id)
    }
}

/// 128-bit XMM reference.
#[derive(Debug, Clone, Copy)]
pub struct XRegisterSse(pub u8);

impl XRegisterSse {
    pub fn is_caller_saved(id: u8) -> bool {
        // x86-32 ABI: all XMM regs are caller-saved.
        true
    }
}

pub mod emitter {
    //! Stub x86 emitter surface. The C++ code calls into
    //! `x86Emitter::xXOR`, `xMOV`, ... — here those are no-op
    //! functions that record the call into a global log.
    use super::*;
    use crate::lazy_static_log;

    lazy_static_log! {
        /// Global log of every emitter call made by the stubs.
        pub static ref EMIT_LOG: Vec<String> = Vec::new();
    }

    fn record(s: String) {
        EMIT_LOG::instance().lock().unwrap().push(s);
    }

    // -- simple register-register / register-immediate -------------------
    pub fn x_xor(_a: XRegister32, _b: XRegister32) {
        record("xXOR r32, r32".into());
    }
    pub fn x_xor64(_a: XRegister64, _b: XRegister64) {
        record("xXOR r64, r64".into());
    }
    pub fn x_not(_a: XRegister32) {
        record("xNOT".into());
    }
    pub fn x_mov_r32_r32(_a: XRegister32, _b: XRegister32) {
        record("xMOV r32, r32".into());
    }
    pub fn x_mov_r64_r64(_a: XRegister64, _b: XRegister64) {
        record("xMOV r64, r64".into());
    }
    pub fn x_mov_r32_imm(_a: XRegister32, _imm: u32) {
        record("xMOV r32, imm32".into());
    }
    pub fn x_mov_r64_imm(_a: XRegister64, _imm: u64) {
        record("xMOV r64, imm64".into());
    }
    pub fn x_mov_r32_mem(_a: XRegister32, _addr: uptr) {
        record("xMOV r32, [mem]".into());
    }
    pub fn x_mov_r64_mem(_a: XRegister64, _addr: uptr) {
        record("xMOV r64, [mem]".into());
    }
    pub fn x_mov_mem_r32(_addr: uptr, _a: XRegister32) {
        record("xMOV [mem], r32".into());
    }
    pub fn x_mov_mem_r64(_addr: uptr, _a: XRegister64) {
        record("xMOV [mem], r64".into());
    }
    pub fn x_mov_mem_imm32(_addr: uptr, _imm: u32) {
        record("xMOV [mem], imm32".into());
    }
    pub fn x_mov_mem_imm64(_addr: uptr, _imm: u64) {
        record("xMOV [mem], imm64".into());
    }
    pub fn x_mov_r16_mem(_a: XRegister32, _addr: uptr) {
        record("xMOVZX r32, [word]".into());
    }
    pub fn x_movd_xmm_r32(_a: XRegister32, _b: XRegisterSse) {
        record("xMOVD r32, xmm".into());
    }
    pub fn x_movd_r64_xmm(_a: XRegister64, _b: XRegisterSse) {
        record("xMOVD r64, xmm".into());
    }
    pub fn x_movss_mem_xmm(_addr: uptr, _a: XRegisterSse) {
        record("xMOVSS [mem], xmm".into());
    }

    // -- arithmetic / logic ---------------------------------------------
    pub fn x_add_r32_r32(_a: XRegister32, _b: XRegister32) {
        record("xADD r32, r32".into());
    }
    pub fn x_add_r32_imm(_a: XRegister32, _imm: i32) {
        record("xADD r32, imm".into());
    }
    pub fn x_add_r64_r64(_a: XRegister64, _b: XRegister64) {
        record("xADD r64, r64".into());
    }
    pub fn x_add_r64_imm(_a: XRegister64, _imm: i32) {
        record("xADD r64, imm".into());
    }
    pub fn x_sub_r32_r32(_a: XRegister32, _b: XRegister32) {
        record("xSUB r32, r32".into());
    }
    pub fn x_sub_r64_r64(_a: XRegister64, _b: XRegister64) {
        record("xSUB r64, r64".into());
    }
    pub fn x_sub_rsp_imm(_imm: i32) {
        record("xSUB rsp, imm".into());
    }
    pub fn x_and_r32_r32(_a: XRegister32, _b: XRegister32) {
        record("xAND r32, r32".into());
    }
    pub fn x_and_r32_imm(_a: XRegister32, _imm: i32) {
        record("xAND r32, imm".into());
    }
    pub fn x_shr_r32(_a: XRegister32, _imm: u8) {
        record("xSHR r32, imm".into());
    }
    pub fn x_test_r32_r32(_a: XRegister32, _b: XRegister32) {
        record("xTEST r32, r32".into());
    }
    pub fn x_cmovs_r64_r64(_a: XRegister64, _b: XRegister64) {
        record("xCMOVS r64, r64".into());
    }
    pub fn x_cmp(_a: XRegister64, _b: XRegister64) {
        record("xCMP r64, r64".into());
    }
    pub fn x_cmp_imm(_a: XRegister32, _imm: i32) {
        record("xCMP r32, imm".into());
    }

    // -- control flow ---------------------------------------------------
    pub fn x_jmp_indirect(_target: uptr) {
        record("xJMP indirect".into());
    }
    pub fn x_jcc(_cc: i32) -> usize {
        record("xJcc".into());
        0
    }
    pub fn x_jcc32() -> usize {
        record("xJcc32".into());
        0
    }
    pub fn x_js(_target: uptr) {
        record("xJS".into());
    }
    pub fn x_jae(_target: uptr) {
        record("xJAE".into());
    }
    pub fn x_jne(_target: uptr) {
        record("xJNE".into());
    }
    pub fn x_jnz(_target: uptr) {
        record("xJNZ".into());
    }
    pub fn x_jc(_target: uptr) {
        record("xJC".into());
    }
    pub fn x_fast_call(_target: uptr) {
        record("xFastCall".into());
    }
    pub fn x_fast_call_arg(_target: uptr, _arg: XRegister32) {
        record("xFastCall arg".into());
    }

    /// `xForwardJNZ32` returns a "branch handle" that the C++ later
    /// resolves with `SetTarget()`. Here we just return a placeholder
    /// address.
    pub fn x_forward_jnz32() -> usize {
        record("xForwardJNZ32".into());
        0
    }
    pub fn x_forward_jge8() -> usize {
        record("xForwardJGE8".into());
        0
    }
    pub fn branch_set_target(_b: usize) {
        record("branch.setTarget".into());
    }

    // -- assembler control ----------------------------------------------
    pub fn x_get_ptr() -> uptr {
        0
    }
    pub fn x_get_aligned_call_target() -> uptr {
        0
    }
    pub fn x_set_ptr(_p: uptr) {}
    pub fn x_set_text_ptr(_p: uptr) {}
    pub fn x_load_far_addr(_a: XRegister64, _p: uptr) {
        record("xLoadFarAddr".into());
    }
    pub fn get_x86_ptr() -> uptr {
        0
    }
    pub fn set_x86_ptr(_p: uptr) {}
}

/// Convenience: `lazy_static_log! { ... }` declares a `Mutex`-backed
/// static without depending on `lazy_static`. It is just a thin
/// wrapper around `OnceLock` that auto-wraps the initializer in a
/// `Mutex`.
#[macro_export]
macro_rules! lazy_static_log {
    {
        $(#[$attr:meta])*
        pub static ref $name:ident: $ty:ty = $init:expr;
    } => {
        $(#[$attr])*
        pub mod $name {
            use super::*;
            use ::std::sync::OnceLock;
            static INNER: OnceLock<::std::sync::Mutex<$ty>> = OnceLock::new();
            pub fn instance() -> &'static ::std::sync::Mutex<$ty> {
                INNER.get_or_init(|| ::std::sync::Mutex::new($init))
            }
        }
    };
}

// ---------------------------------------------------------------------------
// x86 register allocator
// ---------------------------------------------------------------------------

/// The full x86 GPR / XMM allocator. Stand-in for the C++ globals
/// `x86regs[]`, `xmmregs[]`, `g_x86AllocCounter`, `g_xmmAllocCounter`.
#[derive(Debug, Clone)]
pub struct RegAlloc {
    pub gpr: Vec<X86Reg>,
    pub xmm: Vec<XmmReg>,
    pub x86_alloc_counter: u32,
    pub xmm_alloc_counter: u32,
}

impl Default for RegAlloc {
    fn default() -> Self {
        RegAlloc::new(IREG_COUNT_GPR, IREG_COUNT_XMM)
    }
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

    /// Mirrors the C++ `_isAllocatableX86reg(i)`. Host reg 4 (esp) is
    /// never allocatable.
    pub fn is_allocatable_x86(&self, i: usize) -> bool {
        i != 4
    }

    /// Mirrors `mVUIsReservedCOP2(reg)` — see iCore.cpp: any host reg
    /// currently used by VU0 macro-mode is reserved. Here we always
    /// say no, so the caller gets to allocate normally.
    pub fn mvu_is_reserved_cop2(&self, _reg: usize) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// XMM register allocator stub
// ---------------------------------------------------------------------------
//
// The C++ code uses a separate allocator for XMM (`xmmregs`). The
// `_allocGPRtoXMMreg` / `_checkXMMreg` / `_freeXMMreg` entry points
// are not in our source files but are referenced. We declare stubs
// so the signatures compile.

#[derive(Debug, Default, Clone)]
pub struct XmmRegAlloc {
    pub regs: Vec<XmmReg>,
}

impl XmmRegAlloc {
    pub fn new(count: usize) -> Self {
        XmmRegAlloc {
            regs: vec![XmmReg::default(); count],
        }
    }
}

// ---------------------------------------------------------------------------
// Recompiler cache & base blocks
// ---------------------------------------------------------------------------
//
// Mirror of `BASEBLOCK` / `BASEBLOCKEX` from `BaseblockEx.h`, plus
// the `recLUT` page table and the `recRAM` / `recROM` block pools.

/// One base-block descriptor. Mirrors `BASEBLOCK`.
#[derive(Debug, Clone, Default)]
pub struct BaseBlock {
    pub fnptr: uptr,
}

/// Extended base-block descriptor. Mirrors `BASEBLOCKEX`.
#[derive(Debug, Clone, Default)]
pub struct BaseBlockEx {
    pub fnptr: uptr,
    pub startpc: u32,
    pub size: u32,
    pub x86size: u32,
}

/// Sorted base-block table. Mirrors `BaseBlocks`.
#[derive(Debug, Default, Clone)]
pub struct BaseBlocks {
    pub recompiler: uptr,
    pub blocks: BTreeMap<u32, BaseBlockEx>,
    pub links: BTreeMap<u32, usize>,
}

impl BaseBlocks {
    pub fn new() -> Self {
        BaseBlocks::default()
    }

    /// Set the JIT-compile trampoline. Mirrors `SetJITCompile`.
    pub fn set_jit_compile(&mut self, p: uptr) {
        self.recompiler = p;
    }

    /// Reset the table.
    pub fn reset(&mut self) {
        self.blocks.clear();
        self.links.clear();
    }

    /// Insert a freshly compiled block.
    pub fn new_block(&mut self, startpc: u32, fnptr: uptr) -> &mut BaseBlockEx {
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

    /// Find the block whose range contains `pc`. Mirrors `Get`.
    pub fn get(&self, pc: u32) -> Option<&BaseBlockEx> {
        self.blocks
            .range(..=pc)
            .next_back()
            .map(|(_, b)| b)
            .filter(|b| pc >= b.startpc && (b.size == 0 || pc < b.startpc + b.size * 4))
    }

    /// Find the block whose range contains `pc` and is not equal to
    /// `self`. Mirrors the C++ `[]` operator overload semantics.
    pub fn lookup(&self, pc: u32) -> Option<&BaseBlockEx> {
        self.get(pc)
    }

    /// Last index whose block contains or precedes `pc`. Mirrors
    /// `LastIndex`.
    pub fn last_index(&self, pc: u32) -> i32 {
        self.blocks
            .range(..=pc)
            .next_back()
            .map(|(k, _)| *k as i32)
            .unwrap_or(-1)
    }

    /// Patch an outstanding branch into a target block. Mirrors
    /// `Link`.
    pub fn link(&mut self, pc: u32, jump_offset: usize) {
        if let Some(target) = self.blocks.get(&pc) {
            let delta = (target.fnptr as isize) - (jump_offset as isize) - 4;
            self.links.insert(pc, delta as usize);
        } else {
            let delta = (self.recompiler as isize) - (jump_offset as isize) - 4;
            self.links.insert(pc, delta as usize);
        }
    }

    /// Remove a range of blocks. Mirrors `Remove`.
    pub fn remove(&mut self, _from: i32, _to: i32) {
        // Stub: in a real port we'd delete keys in the inclusive range.
    }
}

// ---------------------------------------------------------------------------
// Recompiler code cache
// ---------------------------------------------------------------------------

/// Recompiler output buffer. The real implementation emits x86
/// instructions through the x86Emitter; here it is just a `Vec<u8>`.
#[derive(Debug, Default, Clone)]
pub struct RecCache {
    pub cur: Vec<u8>,
}

impl RecCache {
    pub fn new() -> Self {
        RecCache::default()
    }

    /// Mirrors `xGetPtr()`.
    pub fn get_ptr(&self) -> uptr {
        self.cur.as_ptr() as uptr + self.cur.len()
    }

    /// Mirrors `xGetAlignedCallTarget()`.
    pub fn get_aligned_call_target(&mut self) -> uptr {
        // For the stub we don't worry about alignment.
        self.get_ptr()
    }

    /// Mirrors `xSetPtr(p)`.
    pub fn set_ptr(&mut self, _p: uptr) {}
}

// ---------------------------------------------------------------------------
// Opcode helpers
// ---------------------------------------------------------------------------
//
// Field extractors for a 32-bit EE instruction word. The C++ code
// uses preprocessor macros (`_Opcode_`, `_Funct_`, `_Rs_`, `_Rt_`,
// `_Rd_`, `_Imm_`, `_InstrucTarget_`).

#[inline]
pub fn opcode_of(code: u32) -> u32 {
    code >> 26
}

#[inline]
pub fn funct_of(code: u32) -> u32 {
    code & 0x3F
}

#[inline]
pub fn rs_of(code: u32) -> u32 {
    (code >> 21) & 0x1F
}

#[inline]
pub fn rt_of(code: u32) -> u32 {
    (code >> 16) & 0x1F
}

#[inline]
pub fn rd_of(code: u32) -> u32 {
    (code >> 11) & 0x1F
}

#[inline]
pub fn sa_of(code: u32) -> u32 {
    (code >> 6) & 0x1F
}

#[inline]
pub fn imm_of(code: u32) -> i16 {
    (code & 0xFFFF) as i16
}

#[inline]
pub fn instruc_target_of(code: u32) -> u32 {
    code & 0x03FF_FFFF
}

// ---------------------------------------------------------------------------
// Branch / cycle state
// ---------------------------------------------------------------------------

/// State of the in-flight recompilation. Mirrors the C++ globals
/// `pc`, `g_branch`, `s_branchTo`, `s_nEndBlock`, `s_nBlockCycles`,
/// `s_nBlockInterlocked`, `s_nBlockFF`.
#[derive(Debug, Default, Clone)]
pub struct RecState {
    pub pc: u32,
    pub branch: i32,
    pub branch_to: u32,
    pub end_block: u32,
    pub block_cycles: u32,
    pub block_interlocked: bool,
    pub block_ff: bool,
    pub will_branch3: u32,
    pub max_recmem: u32,
    pub rec_ptr: uptr,
    pub rec_ptr_end: uptr,
    pub rec_text_ptr: uptr,
    pub eeload_main: u32,
    pub eeload_exec: u32,
}

// ---------------------------------------------------------------------------
// Recompiler CPU vtable
// ---------------------------------------------------------------------------
//
// Mirrors the C++ `R5900cpu` vtable the dynarec registers with the
// EE interpreter.

#[derive(Debug, Clone, Copy)]
pub struct R5900Cpu {
    pub reserve: fn(&mut RecContext),
    pub shutdown: fn(&mut RecContext),
    pub reset_ee: fn(&mut RecContext),
    pub step: fn(&mut RecContext),
    pub execute: fn(&mut RecContext),
    pub safe_exit: fn(&mut RecContext),
    pub cancel: fn(&mut RecContext),
    pub clear: fn(&mut RecContext, u32, u32),
}

// ---------------------------------------------------------------------------
// Recompiler context (the `Recompiler` aggregator)
// ---------------------------------------------------------------------------

/// Top-level recompiler context. Holds the global CPU state, the
/// register allocator, the code cache, the base-block table and the
/// per-compile state.
#[derive(Debug, Default)]
pub struct RecContext {
    pub state: CpuState,
    pub regalloc: RegAlloc,
    pub xmm: XmmRegAlloc,
    pub cache: RecCache,
    pub base_blocks: BaseBlocks,
    pub rec: RecState,
    pub ram_copy: Vec<u8>,
    pub rec_lut: Vec<uptr>,
    pub hw_lut: Vec<u32>,
    pub lut_reserve: Vec<BaseBlock>,
    pub lut_unmapped: Vec<BaseBlock>,
    pub lut_entries: usize,
    pub extra_ram: bool,
    pub ram: Vec<BaseBlock>,
    pub rom: Vec<BaseBlock>,
    pub rom1: Vec<BaseBlock>,
    pub rom2: Vec<BaseBlock>,
    pub inst_cache: Vec<EeInst>,
    pub inst_cache_size: u32,
    pub cur_block: Option<BaseBlock>,
    pub cur_block_ex: Option<BaseBlockEx>,
    pub ee_event_test_is_active: bool,
    pub s_save_const_regs: [GprReg64; 32],
    pub s_save_has_const: u32,
    pub s_save_flushed_const: u32,
    pub s_save_block_cycles: u32,
    pub g_reset_ee_scaling_stats: bool,
}

impl RecContext {
    pub fn new() -> Self {
        let mut me = RecContext {
            regalloc: RegAlloc::new(IREG_COUNT_GPR, IREG_COUNT_XMM),
            xmm: XmmRegAlloc::new(IREG_COUNT_XMM),
            cache: RecCache::new(),
            base_blocks: BaseBlocks::new(),
            ..Default::default()
        };
        me.state.cpu_const_regs = [GprReg64::default(); 32];
        me.state.psx_const_regs = [0u32; 32];
        me.s_save_const_regs = [GprReg64::default(); 32];
        me
    }
}

/// Per-instruction live-range / liveness information. Mirrors
/// `EEINST`.
#[derive(Debug, Clone)]
pub struct EeInst {
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

impl Default for EeInst {
    fn default() -> Self {
        EeInst {
            info: 0,
            regs: [0u8; 34],
            fpuregs: [0u8; 33],
            vfregs: [0u8; 34],
            viregs: [0u8; 16],
            write_type: [0u8; 3],
            write_reg: [0u8; 3],
            read_type: [0u8; 4],
            read_reg: [0u8; 4],
        }
    }
}

// ---------------------------------------------------------------------------
// Thread-local access to the global recompiler context
// ---------------------------------------------------------------------------

thread_local! {
    /// Singleton handle mirroring the C++ globals. Real code would
    /// use explicit context objects passed by reference; this is a
    /// translation convenience.
    static REC: RefCell<RecContext> = RefCell::new(RecContext::new());
}

/// Run `f` with mutable access to the global recompiler.
pub fn with_rec<R>(f: impl FnOnce(&mut RecContext) -> R) -> R {
    REC.with(|r| f(&mut r.borrow_mut()))
}

// ===========================================================================
//  iCore.cpp — register allocator
// ===========================================================================
//
// The functions in this section correspond 1:1 to the symbols in
// `pcsx2/x86/ix86-32/iCore.cpp`. They are recreated here using the
// `RegAlloc` and `CpuState` data structures above.

/// `_initX86regs` — reset all host GPR / XMM allocation state.
pub fn init_x86regs(ra: &mut RegAlloc, xmm: &mut XmmRegAlloc) {
    for r in &mut ra.gpr {
        r.in_use = false;
        r.mode = 0;
        r.needed = false;
        r.ty = X86Type::Temp;
        r.counter = 0;
    }
    for r in &mut xmm.regs {
        r.in_use = false;
        r.mode = 0;
        r.needed = false;
        r.counter = 0;
    }
    ra.x86_alloc_counter = 0;
    ra.xmm_alloc_counter = 0;
}

/// `_getFreeX86reg` — pick a free or evictable host GPR. Returns
/// the index or `-1` on failure.
pub fn get_free_x86reg(ra: &RegAlloc, mode: u32) -> i32 {
    // Pass 1: a completely free, allocatable register.
    let n = ra.gpr.len();
    for i in 0..n {
        if ra.gpr[i].in_use || !ra.is_allocatable_x86(i) {
            continue;
        }
        if (mode & MODE_CALLEESAVED) != 0 && XRegister32::is_caller_saved(i as u8) {
            continue;
        }
        if (mode & MODE_COP2) != 0 && ra.mvu_is_reserved_cop2(i) {
            continue;
        }
        return i as i32;
    }
    // Pass 2: pick a non-temp with the lowest allocation counter
    // (LRU eviction).
    let mut best: i32 = -1;
    let mut best_count: u32 = u32::MAX;
    for i in 0..n {
        if !ra.is_allocatable_x86(i) {
            continue;
        }
        if (mode & MODE_CALLEESAVED) != 0 && XRegister32::is_caller_saved(i as u8) {
            continue;
        }
        if (mode & MODE_COP2) != 0 && ra.mvu_is_reserved_cop2(i) {
            continue;
        }
        if ra.gpr[i].needed {
            continue;
        }
        if ra.gpr[i].ty == X86Type::Temp {
            return i as i32;
        }
        if u32::from(ra.gpr[i].counter) < best_count {
            best = i as i32;
            best_count = u32::from(ra.gpr[i].counter);
        }
    }
    best
}

/// `_allocX86reg` — bind a host GPR to a (type, reg) guest slot.
pub fn alloc_x86reg(ra: &mut RegAlloc, state: &mut CpuState, ty: X86Type, reg: i32, mode: u32) -> i32 {
    if matches!(ty, X86Type::Gpr | X86Type::Psx) {
        assert!((reg as i32) >= 0 && (reg as i32) < 34);
    }

    if ty != X86Type::Temp {
        for i in 0..ra.gpr.len() {
            if !ra.gpr[i].in_use || ra.gpr[i].ty != ty || ra.gpr[i].reg as i32 != reg {
                continue;
            }
            assert!(
                !(ty == X86Type::Gpr
                    && gpr_is_const1(state.cpu_has_const, reg as usize)
                    && !gpr_is_const1(state.cpu_flushed_const, reg as usize)),
                "dirty constant cached in a host GPR"
            );
            // can't go from write to read
            let prev = ra.gpr[i].mode as u32;
            assert!(
                !((prev & (MODE_READ | MODE_WRITE)) == MODE_WRITE
                    && (mode & (MODE_READ | MODE_WRITE)) == MODE_READ),
                "x86 host reg going from write to read"
            );
            if (mode & MODE_WRITE) != 0 {
                if ty == X86Type::Gpr && gpr_is_const1(state.cpu_has_const, reg as usize) {
                    gpr_del_const(state, reg as usize);
                }
                if ty == X86Type::Psx && psx_is_const1(state.psx_has_const, reg as usize) {
                    psx_del_const(state, reg as usize);
                }
            }
            ra.gpr[i].counter = (ra.x86_alloc_counter & 0xFFFF) as u16;
            ra.x86_alloc_counter = ra.x86_alloc_counter.wrapping_add(1);
            ra.gpr[i].mode |= (mode & !MODE_CALLEESAVED) as u8;
            ra.gpr[i].needed = true;
            return i as i32;
        }
    }

    let host = get_free_x86reg(ra, mode);
    if host < 0 {
        return -1;
    }
    let host = host as usize;
    ra.gpr[host].ty = ty;
    ra.gpr[host].reg = reg as s8;
    ra.gpr[host].mode = (mode & !MODE_CALLEESAVED) as u8;
    ra.gpr[host].counter = (ra.x86_alloc_counter & 0xFFFF) as u16;
    ra.x86_alloc_counter = ra.x86_alloc_counter.wrapping_add(1);
    ra.gpr[host].needed = true;
    ra.gpr[host].in_use = true;

    if ty == X86Type::Gpr && (mode & MODE_WRITE) != 0 {
        if (reg as i32) < 32 && gpr_is_const1(state.cpu_has_const, reg as usize) {
            gpr_del_const(state, reg as usize);
        }
    } else if ty == X86Type::Psx && (mode & MODE_WRITE) != 0 {
        if (reg as i32) < 32 && psx_is_const1(state.psx_has_const, reg as usize) {
            psx_del_const(state, reg as usize);
        }
    }

    if (mode & MODE_READ) != 0 {
        if ty == X86Type::Gpr {
            if reg == 0 {
                emitter::x_xor(XRegister32(host as u8), XRegister32(host as u8));
            } else if gpr_is_const1(state.cpu_has_const, reg as usize) {
                let v = state.cpu_const_regs[reg as usize].ud0();
                emitter::x_mov_r64_imm(XRegister64(host as u8), v);
                state.cpu_flushed_const |= 1u32 << reg as u32;
                ra.gpr[host].mode |= MODE_WRITE as u8;
            } else {
                let addr = &state.ee.gpr.r[reg as usize] as *const _ as uptr;
                emitter::x_mov_r64_mem(XRegister64(host as u8), addr);
            }
        } else if ty == X86Type::Fprc {
            let addr = &state.fpu.fprc[reg as usize] as *const _ as uptr;
            emitter::x_mov_r32_mem(XRegister32(host as u8), addr);
        } else if ty == X86Type::Psx {
            if reg == 0 {
                emitter::x_xor(XRegister32(host as u8), XRegister32(host as u8));
            } else if psx_is_const1(state.psx_has_const, reg as usize) {
                emitter::x_mov_r32_imm(XRegister32(host as u8), state.psx_const_regs[reg as usize]);
                state.psx_flushed_const |= 1u32 << reg as u32;
                ra.gpr[host].mode |= MODE_WRITE as u8;
            } else {
                let addr = &state.psx.gpr[reg as usize] as *const _ as uptr;
                emitter::x_mov_r32_mem(XRegister32(host as u8), addr);
            }
        } else if ty == X86Type::ViReg {
            let addr = &state.vu0.vi[reg as usize] as *const _ as uptr;
            emitter::x_mov_r16_mem(XRegister32(host as u8), addr);
        }
    }

    host as i32
}

/// `_checkX86reg` — return the host GPR currently bound to
/// `(type, reg)` if any, else `-1`.
pub fn check_x86reg(ra: &mut RegAlloc, state: &mut CpuState, ty: X86Type, reg: i32, mode: u32) -> i32 {
    for i in 0..ra.gpr.len() {
        if !ra.gpr[i].in_use || ra.gpr[i].reg as i32 != reg || ra.gpr[i].ty != ty {
            continue;
        }
        assert!(!gpr_is_dirty_const(state, reg as usize) || ty != X86Type::Gpr);
        assert!(!psx_is_dirty_const(state, reg as usize) || ty != X86Type::Psx);

        if (mode & MODE_WRITE) != 0 {
            if ty == X86Type::Gpr {
                // go through alloc, in case we need to invalidate an
                // XMM co-allocation
                return alloc_x86reg(ra, state, X86Type::Gpr, reg, mode);
            } else if ty == X86Type::Psx {
                assert!(!psx_is_dirty_const(state, reg as usize));
                psx_del_const(state, reg as usize);
            }
        }
        ra.gpr[i].mode |= mode as u8;
        ra.gpr[i].counter = (ra.x86_alloc_counter & 0xFFFF) as u16;
        ra.x86_alloc_counter = ra.x86_alloc_counter.wrapping_add(1);
        ra.gpr[i].needed = true;
        return i as i32;
    }
    -1
}

/// `_addNeededX86reg` — mark an existing (type, reg) binding as
/// "needed" so the next call to `_clearNeededX86regs` keeps it.
pub fn add_needed_x86reg(ra: &mut RegAlloc, ty: X86Type, reg: i32) {
    for i in 0..ra.gpr.len() {
        if !ra.gpr[i].in_use || ra.gpr[i].reg as i32 != reg || ra.gpr[i].ty != ty {
            continue;
        }
        ra.gpr[i].counter = (ra.x86_alloc_counter & 0xFFFF) as u16;
        ra.x86_alloc_counter = ra.x86_alloc_counter.wrapping_add(1);
        ra.gpr[i].needed = true;
    }
}

/// `_clearNeededX86regs` — clear the `needed` flag, demoting any
/// write-mode entries to read-mode.
pub fn clear_needed_x86regs(ra: &mut RegAlloc) {
    for i in 0..ra.gpr.len() {
        if ra.gpr[i].needed {
            if ra.gpr[i].in_use && (ra.gpr[i].mode as u32 & MODE_WRITE) != 0 {
                ra.gpr[i].mode |= MODE_READ as u8;
            }
        }
        ra.gpr[i].needed = false;
    }
}

/// `_freeX86regWithoutWriteback` — release a host GPR.
pub fn free_x86reg_without_writeback(ra: &mut RegAlloc, x86reg: usize) {
    if ra.gpr[x86reg].ty == X86Type::ViReg {
        // mVUFreeCOP2GPR(x86reg) — stub.
    }
    ra.gpr[x86reg].in_use = false;
}

/// `_writebackX86Reg` — spill a dirty host GPR back to guest state.
pub fn writeback_x86_reg(ra: &RegAlloc, state: &mut CpuState, x86reg: usize) {
    let entry = &ra.gpr[x86reg];
    match entry.ty {
        X86Type::Gpr => {
            let addr = &mut state.ee.gpr.r[entry.reg as usize] as *mut _ as uptr;
            emitter::x_mov_mem_r64(addr, XRegister64(x86reg as u8));
        }
        X86Type::Fprc => {
            let addr = &mut state.fpu.fprc[entry.reg as usize] as *mut _ as uptr;
            emitter::x_mov_mem_r32(addr, XRegister32(x86reg as u8));
        }
        X86Type::ViReg => {
            let addr = &mut state.vu0.vi[entry.reg as usize] as *mut _ as uptr;
            emitter::x_mov_mem_r32(addr, XRegister32(x86reg as u8));
        }
        X86Type::PcWriteback => {
            let addr = &mut state.ee.pc_writeback as *mut _ as uptr;
            emitter::x_mov_mem_r32(addr, XRegister32(x86reg as u8));
        }
        X86Type::Psx => {
            let addr = &mut state.psx.gpr[entry.reg as usize] as *mut _ as uptr;
            emitter::x_mov_mem_r32(addr, XRegister32(x86reg as u8));
        }
        X86Type::PsxPcWriteback => {
            let addr = &mut state.psx.pc_writeback as *mut _ as uptr;
            emitter::x_mov_mem_r32(addr, XRegister32(x86reg as u8));
        }
        _ => {}
    }
}

/// `_freeX86reg` — release a host GPR after spilling any dirty data.
pub fn free_x86reg(ra: &mut RegAlloc, state: &mut CpuState, x86reg: usize) {
    if ra.gpr[x86reg].in_use && (ra.gpr[x86reg].mode as u32 & MODE_WRITE) != 0 {
        writeback_x86_reg(ra, state, x86reg);
        ra.gpr[x86reg].mode &= !MODE_WRITE as u8;
    }
    free_x86reg_without_writeback(ra, x86reg);
}

/// `_freeX86regs` — release every host GPR.
pub fn free_x86regs(ra: &mut RegAlloc, state: &mut CpuState) {
    for i in 0..ra.gpr.len() {
        free_x86reg(ra, state, i);
    }
}

/// `_flushX86regs` — spill every dirty host GPR but keep the
/// allocation.
pub fn flush_x86regs(ra: &mut RegAlloc, state: &mut CpuState) {
    for i in 0..ra.gpr.len() {
        if ra.gpr[i].in_use && (ra.gpr[i].mode as u32 & MODE_WRITE) != 0 {
            assert!(!(ra.gpr[i].ty == X86Type::Gpr && gpr_is_dirty_const(state, ra.gpr[i].reg as usize)));
            writeback_x86_reg(ra, state, i);
            ra.gpr[i].mode = (ra.gpr[i].mode & !MODE_WRITE as u8) | MODE_READ as u8;
        }
    }
}

// ===========================================================================
//  iCore.cpp — constant register flush / move helpers
// ===========================================================================

/// `_flushConstReg` — flush a single EE constant register to memory
/// if it is dirty.
pub fn flush_const_reg(state: &mut CpuState, reg: usize) {
    if gpr_is_const1(state.cpu_has_const, reg) && !gpr_is_const1(state.cpu_flushed_const, reg) {
        let v = state.cpu_const_regs[reg].sd0();
        let addr = &mut state.ee.gpr.r[reg] as *mut _ as uptr;
        // xWriteImm64ToMem(addr, rax, v) — stub: write the low 8 bytes
        // as a u64; the high 8 bytes are kept zeroed by GPR semantics.
        let v_u64 = v as u64;
        emitter::x_mov_mem_imm64(addr, v_u64);
        state.cpu_flushed_const |= 1u32 << reg as u32;
        if reg == 0 {
            // DevCon.Warning("Flushing r0!") — skipped in the stub.
        }
    }
}

/// `_flushConstRegs` — flush every dirty EE constant. If
/// `delete_const` is true, also clear the constant-tracking bit.
pub fn flush_const_regs(state: &mut CpuState, delete_const: bool) {
    let mut zero_count = 0;
    let mut minus_one_count = 0;
    for i in 0..32 {
        if !gpr_is_const1(state.cpu_has_const, i) || gpr_is_const1(state.cpu_flushed_const, i) {
            continue;
        }
        match state.cpu_const_regs[i].sd0() {
            0 => zero_count += 1,
            -1 => minus_one_count += 1,
            _ => {}
        }
    }

    let mut rax_is_zero = false;
    if zero_count > 1 {
        emitter::x_xor(XRegister32(0), XRegister32(0));
        for i in 0..32 {
            if !gpr_is_const1(state.cpu_has_const, i) || gpr_is_const1(state.cpu_flushed_const, i) {
                continue;
            }
            if state.cpu_const_regs[i].sd0() == 0 {
                let addr = &mut state.ee.gpr.r[i] as *mut _ as uptr;
                emitter::x_mov_mem_r64(addr, XRegister64(0));
                state.cpu_flushed_const |= 1u32 << i;
                if delete_const {
                    gpr_del_const(state, i);
                }
            }
        }
        rax_is_zero = true;
    }

    if minus_one_count > 1 {
        if !rax_is_zero {
            emitter::x_mov_r64_imm(XRegister64(0), 0xFFFF_FFFF_FFFF_FFFFu64);
        } else {
            emitter::x_not(XRegister32(0));
        }
        for i in 0..32 {
            if !gpr_is_const1(state.cpu_has_const, i) || gpr_is_const1(state.cpu_flushed_const, i) {
                continue;
            }
            if state.cpu_const_regs[i].sd0() == -1 {
                let addr = &mut state.ee.gpr.r[i] as *mut _ as uptr;
                emitter::x_mov_mem_r64(addr, XRegister64(0));
                state.cpu_flushed_const |= 1u32 << i;
                if delete_const {
                    gpr_del_const(state, i);
                }
            }
        }
    }

    for i in 0..32 {
        if !gpr_is_const1(state.cpu_has_const, i) || gpr_is_const1(state.cpu_flushed_const, i) {
            continue;
        }
        let v = state.cpu_const_regs[i].ud0();
        let addr = &mut state.ee.gpr.r[i] as *mut _ as uptr;
        emitter::x_mov_mem_imm64(addr, v);
        state.cpu_flushed_const |= 1u32 << i;
        if delete_const {
            gpr_del_const(state, i);
        }
    }
}

/// `_eeMoveGPRtoR(to, fromgpr, allow_preload)` — 32-bit overload.
pub fn ee_move_gpr_to_r32(
    ra: &mut RegAlloc,
    state: &mut CpuState,
    to: XRegister32,
    fromgpr: i32,
    _allow_preload: bool,
) {
    if fromgpr == 0 {
        emitter::x_xor(to, to);
    } else if gpr_is_const1(state.cpu_has_const, fromgpr as usize) {
        emitter::x_mov_r32_imm(to, state.cpu_const_regs[fromgpr as usize].ul0());
    } else {
        let x86reg = check_x86reg(ra, state, X86Type::Gpr, fromgpr, MODE_READ);
        if x86reg >= 0 {
            emitter::x_mov_r32_r32(to, XRegister32(x86reg as u8));
        } else {
            let addr = &state.ee.gpr.r[fromgpr as usize] as *const _ as uptr;
            emitter::x_mov_r32_mem(to, addr);
        }
    }
}

/// `_eeMoveGPRtoR(to, fromgpr, allow_preload)` — 64-bit overload.
pub fn ee_move_gpr_to_r64(
    ra: &mut RegAlloc,
    state: &mut CpuState,
    to: XRegister64,
    fromgpr: i32,
    _allow_preload: bool,
) {
    if fromgpr == 0 {
        emitter::x_xor(to.as_32(), to.as_32());
    } else if gpr_is_const1(state.cpu_has_const, fromgpr as usize) {
        emitter::x_mov_r64_imm(to, state.cpu_const_regs[fromgpr as usize].ud0());
    } else {
        let x86reg = check_x86reg(ra, state, X86Type::Gpr, fromgpr, MODE_READ);
        if x86reg >= 0 {
            emitter::x_mov_r64_r64(to, XRegister64(x86reg as u8));
        } else {
            let addr = &state.ee.gpr.r[fromgpr as usize] as *const _ as uptr;
            emitter::x_mov_r64_mem(to, addr);
        }
    }
}

/// `_eeMoveGPRtoM(to, fromgpr)` — move a GPR to a memory location.
pub fn ee_move_gpr_to_m(
    ra: &mut RegAlloc,
    state: &mut CpuState,
    to: uptr,
    fromgpr: i32,
) {
    if gpr_is_const1(state.cpu_has_const, fromgpr as usize) {
        emitter::x_mov_mem_imm32(to, state.cpu_const_regs[fromgpr as usize].ul0());
        return;
    }
    let x86reg = check_x86reg(ra, state, X86Type::Gpr, fromgpr, MODE_READ);
    if x86reg >= 0 {
        emitter::x_mov_mem_r32(to, XRegister32(x86reg as u8));
    } else {
        let addr = &state.ee.gpr.r[fromgpr as usize] as *const _ as uptr;
        emitter::x_mov_r32_mem(XRegister32(0), addr);
        emitter::x_mov_mem_r32(to, XRegister32(0));
    }
}

// ===========================================================================
//  iCore.cpp — register validation
// ===========================================================================

/// `_validateRegs` — dev-build check that no EE GPR is in
/// write mode in both a host GPR and a host XMM. (Stub: noop in
/// release, but we keep the signature so callers stay 1:1.)
pub fn validate_regs(ra: &RegAlloc, xmm: &XmmRegAlloc) {
    let _ = ra;
    let _ = xmm;
    // In a dev build this would scan all GPR/XMM allocations and
    // assert that for each guest reg, the (gpr_mode | fpr_mode) does
    // not have both bits of MODE_WRITE set.
}

// ===========================================================================
//  iR5900.cpp — recompiler state & globals (stub)
// ===========================================================================

/// `_eeFlushAllDirty` — top-level flush used between recompiled
/// blocks and after longjmp exits.
pub fn ee_flush_all_dirty(ra: &mut RegAlloc, state: &mut CpuState) {
    flush_x86regs(ra, state);
    flush_const_regs(state, false);
    let _ = (); // XMM flush is a no-op in the stub.
}

// ===========================================================================
//  iR5900.cpp — cycle scaling
// ===========================================================================

/// `scaleblockcycles_calculation` — the inner scaling math used by
/// the EE recompiler's speedhacks.
pub fn scaleblockcycles_calculation(s_nBlockCycles: u32, cyclerate: i8) -> u32 {
    let lowcycles = s_nBlockCycles <= 40;
    let mut scale_cycles: u32 = 0;
    if cyclerate == 0 || lowcycles || cyclerate < -99 || cyclerate > 3 {
        scale_cycles = s_nBlockCycles >> 3;
    } else if cyclerate > 1 {
        scale_cycles = s_nBlockCycles >> (2 + cyclerate) as u32;
    } else if cyclerate == 1 {
        let base = s_nBlockCycles >> 3;
        scale_cycles = ((base as f32) / 1.3f32) as u32;
    } else if cyclerate == -1 {
        // mildest value
        let factor: u32 = if s_nBlockCycles <= 80 || s_nBlockCycles > 168 { 5 } else { 7 };
        scale_cycles = factor * s_nBlockCycles / 32;
    } else {
        let n = (5 + (-2 * (cyclerate as i32 + 1))) as u32;
        scale_cycles = (n * s_nBlockCycles) >> 5;
    }
    if scale_cycles < 1 { 1 } else { scale_cycles }
}

/// `scaleblockcycles` — public scaling entry. Returns the scaled
/// block-cycle count for the current block.
pub fn scaleblockcycles(s_nBlockCycles: u32, cyclerate: i8) -> u32 {
    scaleblockcycles_calculation(s_nBlockCycles, cyclerate)
}

/// `scaleblockcycles_clear` — like `scaleblockcycles`, but also
/// truncates `s_nBlockCycles` to a smaller mask based on the
/// speedhack.
pub fn scaleblockcycles_clear(s_nBlockCycles: &mut u32, cyclerate: i8) -> u32 {
    let scaled = scaleblockcycles_calculation(*s_nBlockCycles, cyclerate);
    let lowcycles = *s_nBlockCycles <= 40;
    if !lowcycles && cyclerate > 1 {
        *s_nBlockCycles &= (1u32 << (cyclerate as u32 + 2)) - 1;
    } else {
        *s_nBlockCycles &= 0x7;
    }
    scaled
}

// ===========================================================================
//  iR5900.cpp — COP2 timing helpers
// ===========================================================================

/// `cop2flags(code)` — returns a bitmask of COP2 status flags the
/// given EE instruction modifies.
pub fn cop2flags(code: u32) -> i32 {
    if (code >> 26) != 0o22 {
        return 0;
    }
    if ((code >> 25) & 1) == 0 {
        return 0;
    }
    match (code >> 2) & 0xF {
        0o15 => {
            match (code >> 6) & 0x1F {
                4 | 5 | 12 | 13 | 15 | 16 => 0,
                7 => {
                    if (code & 3) == 1 {
                        0
                    } else if (code & 3) == 3 {
                        4
                    } else {
                        3
                    }
                }
                11 => {
                    if (code & 3) == 3 { 0 } else { 3 }
                }
                14 => {
                    if (code & 3) == 3 { 0 } else { 1 }
                }
                _ => 3,
            }
        }
        4 | 5 | 12 | 13 | 14 => 0,
        7 => {
            if (code & 1) == 1 { 0 } else { 3 }
        }
        10 => {
            if (code & 3) == 3 { 0 } else { 3 }
        }
        11 => {
            if (code & 3) == 3 { 0 } else { 3 }
        }
        _ => 3,
    }
}

/// `COP2DivUnitTimings(code)` — return the COP2 DIV/SQRT/RSQRT
/// latency in EE cycles.
pub fn cop2_div_unit_timings(code: u32) -> i32 {
    match code & 0x3FF {
        0x3BC | 0x3BD => 6,
        0x3BE => 12,
        _ => 0,
    }
}

/// `COP2IsQOP(code)` — true if the COP2 instruction writes Q.
pub fn cop2_is_qop(code: u32) -> bool {
    if (code >> 26) != 0o22 {
        return false;
    }
    matches!(
        code & 0x7FF,
        0x20 | 0x21 | 0x24 | 0x25 | 0x1C | 0x1FC | 0x23C | 0x23D | 0x27C | 0x27D
    )
}

// ===========================================================================
//  iR5900.cpp — EE recompiler entry points (32-bit)
// ===========================================================================

/// `recReserve` — allocate the EE recompiler code cache, base-block
/// tables, and instruction cache.
pub fn rec_reserve(ctx: &mut RecContext) {
    ctx.rec.rec_ptr = 0;
    ctx.rec.rec_ptr_end = 0;
    rec_reserve_ram(ctx);
    if !ctx.inst_cache.is_empty() {
        return;
    }
    ctx.inst_cache_size = 128;
    ctx.inst_cache = vec![EeInst::default(); ctx.inst_cache_size as usize];
}

/// `recReserveRAM` — pre-fill the LUT, the unmapped page and the
/// RAM/ROM/ROM1/ROM2 block pools.
pub fn rec_reserve_ram(ctx: &mut RecContext) {
    // One entry per possible call target.
    let lut_entries = 0x0200_0000 / 4; // stand-in: 32 MiB
    ctx.lut_entries = lut_entries;
    ctx.ram_copy = vec![0u8; 0x0200_0000];
    ctx.lut_reserve = vec![BaseBlock::default(); lut_entries];
    ctx.lut_unmapped = vec![BaseBlock::default(); _64KB / 4];

    let mut basepos = 0usize;
    ctx.ram = ctx.lut_reserve[basepos..basepos + 0x0200_0000 / 4].to_vec();
    basepos += 0x0200_0000 / 4;
    let _ = basepos;

    // Mark all LUT pages as initially unmapped.
    for entry in &mut ctx.lut_unmapped {
        entry.fnptr = ctx.base_blocks.recompiler;
    }
}

/// `recResetRaw` — clear code cache, regalloc, inst cache, LUT.
pub fn rec_reset_raw(ctx: &mut RecContext) {
    if ctx.extra_ram {
        rec_reserve_ram(ctx);
        ctx.extra_ram = false;
    }
    ctx.cache = RecCache::new();
    dyn_gen_dispatchers(ctx);
    ctx.rec.rec_ptr = ctx.cache.get_ptr();

    for blk in &mut ctx.lut_reserve {
        blk.fnptr = ctx.base_blocks.recompiler;
    }
    for blk in &mut ctx.lut_unmapped {
        blk.fnptr = 0; // UnmappedRecLUTPage
    }
    ctx.ram_copy.fill(0);
    ctx.rec.max_recmem = 0;
    if !ctx.inst_cache.is_empty() {
        for inst in &mut ctx.inst_cache {
            *inst = EeInst::default();
        }
    }
    ctx.base_blocks.reset();
    ctx.rec.branch = 0;
    ctx.g_reset_ee_scaling_stats = true;
}

/// `recShutdown` — release all the recompiler resources.
pub fn rec_shutdown(ctx: &mut RecContext) {
    ctx.ram_copy.clear();
    ctx.lut_reserve.clear();
    ctx.base_blocks.reset();
    ctx.ram.clear();
    ctx.rom.clear();
    ctx.rom1.clear();
    ctx.rom2.clear();
    ctx.inst_cache.clear();
    ctx.inst_cache_size = 0;
    ctx.rec.rec_ptr = 0;
    ctx.rec.rec_ptr_end = 0;
}

/// `recStep` — single-step entry (no-op in the EE 32-bit dynarec).
pub fn rec_step(_ctx: &mut RecContext) {}

/// `recExecute` — start running recompiled EE code.
pub fn rec_execute(ctx: &mut RecContext) {
    if ctx.state.ee_rec_needs_reset {
        ctx.state.ee_rec_needs_reset = false;
        rec_reset_raw(ctx);
    }
    let entered = with_setjmp_state(|state| !fastjmp_set(state));
    if entered {
        ctx.state.ee_cpu_executing = true;
        // ((void (*)())EnterRecompiledCode)();
        let _ = enter_recompiled_code(ctx);
    }
    ctx.state.ee_cpu_executing = false;
}

/// `recCancelInstruction` — stub.
pub fn rec_cancel_instruction(_ctx: &mut RecContext) {
    // Never called in normal operation.
}

/// `recSafeExitExecution` — request an exit from the recompiler.
pub fn rec_safe_exit_execution(ctx: &mut RecContext) {
    ctx.state.ee_rec_exit_requested = true;
    if !ctx.ee_event_test_is_active {
        ctx.state.ee.next_event_cycle = 0;
    } else if ctx.state.psx.iop_cycle_ee > 0 {
        ctx.state.psx.iop_break += ctx.state.psx.iop_cycle_ee;
        ctx.state.psx.iop_cycle_ee = 0;
    }
}

/// `recExitExecution` — actually longjmp out of the recompiler.
pub fn rec_exit_execution() {
    with_setjmp_state(|state| fastjmp_jmp(state, 1));
}

/// `recResetEE` — schedule a recompiler reset.
pub fn rec_reset_ee(ctx: &mut RecContext) {
    if ctx.state.ee_cpu_executing {
        ctx.state.ee_rec_needs_reset = true;
        rec_safe_exit_execution(ctx);
        return;
    }
    rec_reset_raw(ctx);
}

// ---------------------------------------------------------------------------
// setjmp / longjmp stubs
// ---------------------------------------------------------------------------
//
// The C++ uses `fastjmp_set` / `fastjmp_jmp` (a `setjmp`-like
// primitive). We model that as a `RefCell<Option<isize>>` for the
// stub.

thread_local! {
    static M_SETJMP_STATE_CHECK: RefCell<isize> = RefCell::new(0);
}

pub fn get_setjmp_state() -> isize {
    M_SETJMP_STATE_CHECK.with(|c| *c.borrow())
}

/// Apply `f` to the current setjmp state. We return a boxed `&mut
/// isize` because the underlying value lives in a thread-local
/// `RefCell`, which can't give us a stable raw mutable reference
/// across the call boundary.
fn with_setjmp_state<R>(f: impl FnOnce(&mut isize) -> R) -> R {
    M_SETJMP_STATE_CHECK.with(|c| f(&mut c.borrow_mut()))
}

fn fastjmp_set(_state: &mut isize) -> bool {
    // In the real implementation this returns the second `longjmp`
    // argument; here we always claim to be in the first call.
    false
}

fn fastjmp_jmp(state: &mut isize, val: isize) {
    *state = val;
}

// ---------------------------------------------------------------------------
// Dispatcher / trampoline generation
// ---------------------------------------------------------------------------

/// `_DynGen_DispatcherEvent` — fall through to `recEventTest`.
pub fn dyn_gen_dispatcher_event(_ctx: &mut RecContext) -> uptr {
    // xFastCall((const void*)recEventTest);
    let ptr = emitter::x_get_ptr();
    emitter::x_fast_call(ptr); // stub
    ptr
}

/// `_DynGen_DispatcherReg` — dispatch to `cpuRegs.pc`.
pub fn dyn_gen_dispatcher_reg(_ctx: &mut RecContext) -> uptr {
    let ptr = emitter::x_get_ptr();
    // Stub: real implementation emits a jmp through recLUT[pc>>16].
    let _ = ptr;
    ptr
}

/// `_DynGen_JITCompile` — first entry; recompiles the current PC.
pub fn dyn_gen_jitcompile(_ctx: &mut RecContext) -> uptr {
    let ptr = emitter::x_get_aligned_call_target();
    // xFastCall((const void*)recRecompile, ptr32[&cpuRegs.pc]);
    emitter::x_fast_call(ptr);
    ptr
}

/// `_DynGen_EnterRecompiledCode` — entry point for the
/// recompiler. Aligns the stack, optionally loads the FASTMEM
/// base, and jumps to `DispatcherReg`.
pub fn dyn_gen_enter_recompiled_code(ctx: &mut RecContext) -> uptr {
    let ptr = emitter::x_get_aligned_call_target();
    // xSUB(rsp, 32 + 8);  // Win32 shadow space
    emitter::x_sub_rsp_imm(40);
    if let Some(p) = Some(emitter::x_get_ptr()) {
        emitter::x_load_far_addr(XRegister64(0), p);
    }
    emitter::x_jmp_indirect(ctx.base_blocks.recompiler);
    ptr
}

/// `_DynGen_DispatchBlockDiscard` — fallback for a manual-protection
/// discard.
pub fn dyn_gen_dispatch_block_discard(_ctx: &mut RecContext) -> uptr {
    let ptr = emitter::x_get_ptr();
    emitter::x_fast_call(ptr); // dyna_block_discard
    emitter::x_jmp_indirect(0);
    ptr
}

/// `_DynGen_DispatchPageReset` — fallback for a manual-protection
/// page reset.
pub fn dyn_gen_dispatch_page_reset(_ctx: &mut RecContext) -> uptr {
    let ptr = emitter::x_get_ptr();
    emitter::x_fast_call(ptr); // dyna_page_reset
    emitter::x_jmp_indirect(0);
    ptr
}

/// `_DynGen_UnmappedRecLUTPage` — error trampoline.
pub fn dyn_gen_unmapped_rec_lut_page(_ctx: &mut RecContext) -> uptr {
    let ptr = emitter::x_get_ptr();
    emitter::x_fast_call_arg(ptr, XRegister32(0));
    ptr
}

/// `_DynGen_Dispatchers` — emit all of the above in a single batch.
pub fn dyn_gen_dispatchers(ctx: &mut RecContext) {
    let start = emitter::x_get_aligned_call_target();
    let _ = dyn_gen_dispatcher_event(ctx);
    let dispatcher_reg = dyn_gen_dispatcher_reg(ctx);
    let jit_compile = dyn_gen_jitcompile(ctx);
    let _enter = dyn_gen_enter_recompiled_code(ctx);
    let _block_discard = dyn_gen_dispatch_block_discard(ctx);
    let _page_reset = dyn_gen_dispatch_page_reset(ctx);
    let _unmapped = dyn_gen_unmapped_rec_lut_page(ctx);

    ctx.base_blocks.set_jit_compile(jit_compile);
    let _ = dispatcher_reg;
    let _ = start;
}

// ---------------------------------------------------------------------------
// `enter_recompiled_code` — high-level entry (modelled on the C++
// fastjmp loop in `recExecute`).
// ---------------------------------------------------------------------------

fn enter_recompiled_code(_ctx: &mut RecContext) -> uptr {
    0
}

// ---------------------------------------------------------------------------
// `recError` — async error reporting.
// ---------------------------------------------------------------------------

/// `recError(code)` — 0 = unmapped recLUT page, 1 = unaligned jump.
pub fn rec_error(ctx: &mut RecContext, code: u32) {
    let _ = ctx;
    let _ = code;
    // In the real implementation, this calls `Host::ReportErrorAsync`
    // and `VMManager::SetPaused(true)`. The stub leaves the message
    // to the surrounding layer.
    rec_exit_execution();
}

// ---------------------------------------------------------------------------
// `ClearRecLUT` — re-fill a range of base-blocks with the JIT-compile
// trampoline.
// ---------------------------------------------------------------------------

pub fn clear_rec_lut(base: &mut [BaseBlock], memsize: usize, jit_compile: uptr) {
    for blk in base.iter_mut().take(memsize / 4) {
        blk.fnptr = jit_compile;
    }
}

// ---------------------------------------------------------------------------
// `recClear` — discard any compiled blocks overlapping
// `[addr, addr + size * 4)`.
// ---------------------------------------------------------------------------

pub fn rec_clear(ctx: &mut RecContext, addr: u32, size: u32) {
    if addr >= ctx.rec.max_recmem {
        return;
    }
    let last = (ctx.base_blocks.last_index(addr + size * 4 - 4)) as i32;
    if last < 0 {
        return;
    }
    let _ = last; // In a real port we walk the table and patch fnptrs.
}

// ---------------------------------------------------------------------------
// Branch helpers
// ---------------------------------------------------------------------------

/// `SetBranchReg` — dispatch through `eax` (the C++ expects the
/// branch target to be in `eax` at this point).
pub fn set_branch_reg(ctx: &mut RecContext) {
    ctx.rec.branch = 1;
    let addr = &mut ctx.state.ee.pc as *mut _ as uptr;
    emitter::x_mov_mem_r32(addr, XRegister32(0));
    emitter::x_test_r32_r32(XRegister32(0), XRegister32(0));
    let unaligned = emitter::x_forward_jnz32();
    i_flush_call(ctx, FLUSH_EVERYTHING);
    i_branch_test(ctx, 0xFFFF_FFFF);
    emitter::branch_set_target(unaligned);
    emitter::x_fast_call_arg(0, XRegister32(0)); // recError, 1
}

/// `SetBranchImm(imm)` — dispatch to a known PC.
pub fn set_branch_imm(ctx: &mut RecContext, imm: u32) {
    ctx.rec.branch = 1;
    assert!(imm != 0);
    i_flush_call(ctx, FLUSH_EVERYTHING);
    let addr = &mut ctx.state.ee.pc as *mut _ as uptr;
    emitter::x_mov_mem_imm32(addr, imm);
    i_branch_test(ctx, imm);
}

/// `recBeginThunk` / `recEndThunk` — bump the cache write pointer.
pub fn rec_begin_thunk(ctx: &mut RecContext) -> uptr {
    if ctx.rec.rec_ptr >= ctx.rec.rec_ptr_end {
        ctx.state.ee_rec_needs_reset = true;
    }
    ctx.rec.rec_text_ptr = ctx.rec.rec_ptr;
    ctx.cache.set_ptr(ctx.rec.rec_ptr);
    let aligned = ctx.cache.get_aligned_call_target();
    ctx.rec.rec_ptr = aligned;
    set_x86_ptr(aligned);
    aligned
}

pub fn rec_end_thunk(ctx: &mut RecContext) -> uptr {
    let end = get_x86_ptr();
    ctx.rec.rec_ptr = end;
    end
}

fn set_x86_ptr(_p: uptr) {}
fn get_x86_ptr() -> uptr { 0 }

// ---------------------------------------------------------------------------
// `iBranchTest` — emit the cycle-add + event-test sequence at the
// end of a block.
// ---------------------------------------------------------------------------

pub fn i_branch_test(ctx: &mut RecContext, newpc: u32) {
    let scaled = scaleblockcycles(ctx.rec.block_cycles, 0);
    let cycle_addr = &ctx.state.ee.cycle as *const _ as uptr;
    let next_addr = &ctx.state.ee.next_event_cycle as *const _ as uptr;
    emitter::x_mov_r64_mem(XRegister64(0), cycle_addr);
    emitter::x_add_r64_imm(XRegister64(0), scaled as i32);
    emitter::x_mov_mem_r64(cycle_addr, XRegister64(0));
    emitter::x_sub_r64_r64(XRegister64(0), XRegister64(1));
    let _ = next_addr;
    if newpc == 0xFFFF_FFFF {
        emitter::x_js(ctx.base_blocks.recompiler);
    } else {
        let _offset = emitter::x_jcc32();
        // recBlocks.Link(...)
    }
    emitter::x_jmp_indirect(ctx.base_blocks.recompiler);
}

// ---------------------------------------------------------------------------
// `iFlushCall` — spill / free host regs before an indirect call.
// ---------------------------------------------------------------------------

pub fn i_flush_call(ctx: &mut RecContext, flushtype: u32) {
    for i in 0..ctx.regalloc.gpr.len() {
        if !ctx.regalloc.gpr[i].in_use {
            continue;
        }
        let is_caller = XRegister64::is_caller_saved(i as u8);
        let drop_vi = (flushtype & FLUSH_FREE_VU0) != 0 && ctx.regalloc.gpr[i].ty == X86Type::ViReg;
        let drop_non_temp =
            (flushtype & FLUSH_FREE_NONTEMP_X86) != 0 && ctx.regalloc.gpr[i].ty != X86Type::Temp;
        let drop_temp =
            (flushtype & FLUSH_FREE_TEMP_X86) != 0 && ctx.regalloc.gpr[i].ty == X86Type::Temp;
        if is_caller || drop_vi || drop_non_temp || drop_temp {
            free_x86reg(&mut ctx.regalloc, &mut ctx.state, i);
        }
    }
    if (flushtype & FLUSH_ALL_X86) != 0 {
        flush_x86regs(&mut ctx.regalloc, &mut ctx.state);
    }
    if (flushtype & FLUSH_CONSTANT_REGS) != 0 {
        flush_const_regs(&mut ctx.state, true);
    }
    if (flushtype & FLUSH_PC) != 0 && !ctx.state.cpu_flushed_pc {
        let addr = &mut ctx.state.ee.pc as *mut _ as uptr;
        emitter::x_mov_mem_imm32(addr, ctx.rec.pc);
        ctx.state.cpu_flushed_pc = true;
    }
    if (flushtype & FLUSH_CODE) != 0 && !ctx.state.cpu_flushed_code {
        let addr = &mut ctx.state.ee.code as *mut _ as uptr;
        emitter::x_mov_mem_imm32(addr, ctx.state.ee.code);
        ctx.state.cpu_flushed_code = true;
    }
}

// ---------------------------------------------------------------------------
// `SaveBranchState` / `LoadBranchState` — branch shadowing.
// ---------------------------------------------------------------------------

pub fn save_branch_state(ctx: &mut RecContext) {
    ctx.s_save_block_cycles = ctx.rec.block_cycles;
    ctx.s_save_const_regs = ctx.state.cpu_const_regs;
    ctx.s_save_has_const = ctx.state.cpu_has_const;
    ctx.s_save_flushed_const = ctx.state.cpu_flushed_const;
    // g_pCurInstInfo and xmm save are stubs here.
}

pub fn load_branch_state(ctx: &mut RecContext) {
    ctx.rec.block_cycles = ctx.s_save_block_cycles;
    ctx.state.cpu_const_regs = ctx.s_save_const_regs;
    ctx.state.cpu_has_const = ctx.s_save_has_const;
    ctx.state.cpu_flushed_const = ctx.s_save_flushed_const;
}

// ---------------------------------------------------------------------------
// `recCall` / `recBranchCall` — call into an interpreter helper.
// ---------------------------------------------------------------------------

pub fn rec_call(ctx: &mut RecContext, target: uptr) {
    i_flush_call(ctx, FLUSH_INTERPRETER);
    emitter::x_fast_call(target);
}

pub fn rec_branch_call(ctx: &mut RecContext, target: uptr) {
    let cycle_addr = &ctx.state.ee.cycle as *const _ as uptr;
    let next_addr = &mut ctx.state.ee.next_event_cycle as *mut _ as uptr;
    emitter::x_mov_r64_mem(XRegister64(0), cycle_addr);
    emitter::x_mov_mem_r64(next_addr, XRegister64(0));
    rec_call(ctx, target);
    ctx.rec.branch = 2;
}

// ---------------------------------------------------------------------------
// `TrySwapDelaySlot` — best-effort delay-slot swap.
// ---------------------------------------------------------------------------

/// `TrySwapDelaySlot(rs, rt, rd, allow_loadstore)`. Returns `true`
/// if the next instruction was successfully swapped into the
/// delay slot.
pub fn try_swap_delay_slot(
    ctx: &mut RecContext,
    rs: u32,
    rt: u32,
    rd: u32,
    allow_loadstore: bool,
) -> bool {
    if ctx.state.recompiling_delay_slot {
        return false;
    }
    let opcode_encoded = ctx.state.ee.code;
    if opcode_encoded == 0 {
        return true; // NOP — always safe
    }
    let opcode_rs = (opcode_encoded >> 21) & 0x1F;
    let opcode_rt = (opcode_encoded >> 16) & 0x1F;
    let opcode_rd = (opcode_encoded >> 11) & 0x1F;

    let primary = opcode_encoded >> 26;
    match primary {
        8 | 9 | 10 | 11 | 12 | 13 | 14 | 24 | 25 => {
            if (rs != 0 && rs == opcode_rt)
                || (rt != 0 && rt == opcode_rt)
                || (rd != 0 && (rd == opcode_rs || rd == opcode_rt))
            {
                return false;
            }
        }
        26 | 27 | 30 | 31 | 32 | 33 | 34 | 35 | 36 | 37 | 38 | 39 | 40 | 41 | 42 | 43 | 44
        | 45 | 46 | 55 | 63 => {
            if !allow_loadstore
                || (rs != 0 && rs == opcode_rt)
                || (rt != 0 && rt == opcode_rt)
                || (rd != 0 && (rd == opcode_rs || rd == opcode_rt))
            {
                return false;
            }
        }
        15 => {
            if (rs != 0 && rs == opcode_rt)
                || (rt != 0 && rt == opcode_rt)
                || (rd != 0 && rd == opcode_rt)
            {
                return false;
            }
        }
        49 | 57 | 54 | 62 => {
            // LWC1/SWC1/LQC2/SQC2 — always safe
        }
        0 => {
            let funct = opcode_encoded & 0x3F;
            match funct {
                0 | 2 | 3 | 4 | 6 | 7 | 10 | 11 | 20 | 22 | 23 | 24 | 25 | 32 | 33 | 34 | 35
                | 36 | 37 | 38 | 39 | 42 | 43 | 44 | 45 | 46 | 47 | 56 | 58 | 59 | 60 | 62
                | 64 => {
                    if (rs != 0 && rs == opcode_rd)
                        || (rt != 0 && rt == opcode_rd)
                        || (rd != 0 && (rd == opcode_rs || rd == opcode_rt))
                    {
                        return false;
                    }
                }
                15 | 26 | 27 => {
                    // SYNC, DIV, DIVU — safe
                }
                _ => return false,
            }
        }
        16 => match (opcode_encoded >> 21) & 0x1F {
            0 | 2 => {
                if (rs != 0 && rs == opcode_rt)
                    || (rt != 0 && rt == opcode_rt)
                    || (rd != 0 && rd == opcode_rt)
                {
                    return false;
                }
            }
            4 | 6 => {
                // MTC0 / CTC0 — always safe
            }
            _ => return false,
        },
        17 => match (opcode_encoded >> 21) & 0x1F {
            0 | 2 => {
                if (rs != 0 && rs == opcode_rt)
                    || (rt != 0 && rt == opcode_rt)
                    || (rd != 0 && rd == opcode_rt)
                {
                    return false;
                }
            }
            4 | 6 | 16 | 20 => {
                let funct = opcode_encoded & 0x3F;
                if funct == 50 || funct == 52 || funct == 54 {
                    return false;
                }
            }
            _ => return false,
        },
        18 => match (opcode_encoded >> 21) & 0x1F {
            8 => return false,
            1 | 2 => {
                if (rs != 0 && rs == opcode_rt)
                    || (rt != 0 && rt == opcode_rt)
                    || (rd != 0 && rd == opcode_rt)
                {
                    return false;
                }
            }
            _ => {}
        },
        28 => {
            // MMI
            let funct = opcode_encoded & 0x3F;
            match funct {
                8 | 9 | 10 | 40 | 41 | 52 | 54 | 55 | 60 | 62 | 63 => {
                    if (rs != 0 && rs == opcode_rd)
                        || (rt != 0 && rt == opcode_rd)
                        || (rd != 0 && rd == opcode_rd)
                    {
                        return false;
                    }
                }
                _ => return false,
            }
        }
        _ => return false,
    }
    true
}

// ---------------------------------------------------------------------------
// `recompileNextInstruction` — driver called once per EE instruction.
// ---------------------------------------------------------------------------

/// `recompileNextInstruction(delayslot, swapped_delay_slot)`.
pub fn recompile_next_instruction(ctx: &mut RecContext, delayslot: bool, swapped_delay_slot: bool) {
    if !delayslot {
        // encodeBreakpoint / encodeMemcheck — stubs.
    } else {
        clear_needed_x86regs(&mut ctx.regalloc);
        // _clearNeededXMMregs — also a stub.
    }

    let _ = ctx.state.ee.code;

    let old_code = ctx.state.ee.code;
    let _ = old_code;

    if !delayslot {
        ctx.rec.pc = ctx.rec.pc.wrapping_add(4);
        ctx.state.cpu_flushed_pc = false;
        ctx.state.cpu_flushed_code = false;
    } else {
        ctx.state.recompiling_delay_slot = true;
    }

    // (liveness pre-pass and COP2 register flush are stubs.)

    let _ = opcode_of(ctx.state.ee.code);
    let _ = funct_of(ctx.state.ee.code);
    let _ = rs_of(ctx.state.ee.code);
    let _ = rt_of(ctx.state.ee.code);

    // No-op NOP fast path:
    if ctx.state.ee.code == 0 {
        ctx.rec.block_cycles += 9 * (2 - ((ctx.state.ee.cp0_config >> 18) & 0x1));
    } else {
        ctx.rec.block_cycles += 4;
        // opcode.recompile() — stub: in a real port this would call
        // into the per-opcode recompiler.
    }

    if !swapped_delay_slot {
        clear_needed_x86regs(&mut ctx.regalloc);
    }
    validate_regs(&ctx.regalloc, &ctx.xmm);

    if delayslot {
        ctx.rec.pc = ctx.rec.pc.wrapping_add(4);
        ctx.state.cpu_flushed_pc = false;
        ctx.state.cpu_flushed_code = false;
        if ctx.state.may_signal_exception {
            // xAND ptr32 [&cpuRegs.CP0.n.Cause], !(1 << 31) — stub.
        }
        ctx.state.recompiling_delay_slot = false;
    }
    ctx.state.may_signal_exception = false;
}

// ---------------------------------------------------------------------------
// `dynarecCheckBreakpoint` / `dynarecMemcheck` / `recMemcheck` —
// breakpoint glue. Stubs in this translation.
// ---------------------------------------------------------------------------

pub fn dynarec_check_breakpoint(_ctx: &mut RecContext) {}
pub fn dynarec_memcheck(_ctx: &mut RecContext, _i: usize) {}
pub fn rec_memcheck(_ctx: &mut RecContext, _op: u32, _bits: u32, _store: bool) {}
pub fn encode_breakpoint(_ctx: &mut RecContext) -> bool { false }
pub fn encode_memcheck(_ctx: &mut RecContext) -> bool { false }

// ---------------------------------------------------------------------------
// SYSCALL / BREAK opcode emitters.
// ---------------------------------------------------------------------------

/// `recSYSCALL` — emit a SYSCALL.
pub fn rec_syscall(ctx: &mut RecContext) {
    if gpr_is_const1(ctx.state.cpu_has_const, 3)
        && matches!(ctx.state.cpu_const_regs[3].uc0(), 0x64 | 0x68)
    {
        // Skip FlushCache in JIT.
        ctx.rec.block_cycles += 5650;
        return;
    }
    rec_call(ctx, 0); // R5900::Interpreter::OpcodeImpl::SYSCALL
    ctx.rec.branch = 2;
}

/// `recBREAK` — emit a BREAK.
pub fn rec_break(ctx: &mut RecContext) {
    rec_call(ctx, 0); // R5900::Interpreter::OpcodeImpl::BREAK
    ctx.rec.branch = 2;
}

// ---------------------------------------------------------------------------
// Manual protection / speedhacks.
// ---------------------------------------------------------------------------

/// `memory_protect_recompiled_code(startpc, size)` — decide which
/// protection mode the recompiled block should use, and emit the
/// appropriate entry-time checks. Stub here.
pub fn memory_protect_recompiled_code(_ctx: &mut RecContext, _startpc: u32, _size: u32) {}

/// `dyna_block_discard(start, sz)` — discard a block whose source
/// memory was modified.
pub fn dyna_block_discard(ctx: &mut RecContext, start: u32, sz: u32) {
    rec_clear(ctx, start, sz);
}

/// `dyna_page_reset(start, sz)` — re-protect a counted page.
pub fn dyna_page_reset(ctx: &mut RecContext, start: u32, sz: u32) {
    rec_clear(ctx, start & !0xfff, sz);
}

/// `skipMPEG_By_Pattern` — God of War-style sceMpegIsEnd pattern
/// detector. Returns `true` if a hand-coded fast-path was emitted.
pub fn skip_mpeg_by_pattern(_ctx: &mut RecContext, _sPC: u32) -> bool {
    false
}

/// `recSkipTimeoutLoop` — emit the WaitLoop speedhack. Returns
/// `true` if a fast-path was emitted.
pub fn rec_skip_timeout_loop(_ctx: &mut RecContext, _reg: i32, _is_timeout_loop: bool) -> bool {
    false
}

// ---------------------------------------------------------------------------
// Top-level EE recompilation entry point.
// ---------------------------------------------------------------------------

/// `recRecompile(startpc)` — recompile one EE basic block.
pub fn rec_recompile(ctx: &mut RecContext, startpc: u32) {
    assert!(startpc != 0);
    if ctx.rec.rec_ptr >= ctx.rec.rec_ptr_end {
        ctx.state.ee_rec_needs_reset = true;
    }
    if ctx.state.ee_rec_needs_reset {
        ctx.state.ee_rec_needs_reset = false;
        rec_reset_raw(ctx);
    }

    ctx.cache.set_ptr(ctx.rec.rec_ptr);
    let aligned = ctx.cache.get_aligned_call_target();
    ctx.rec.rec_ptr = aligned;

    // The "current block" is the BASEBLOCK entry for startpc.
    ctx.cur_block = Some(BaseBlock { fnptr: ctx.base_blocks.recompiler });
    ctx.cur_block_ex = Some(ctx.base_blocks.new_block(startpc, aligned).clone());

    if startpc == 0x1FC0_0000 {
        // EELOAD_START — would copy g_eeloadMain. Stub.
    }

    ctx.rec.branch = 0;
    ctx.rec.block_cycles = 0;
    ctx.rec.block_interlocked = false;
    ctx.rec.pc = startpc;
    ctx.state.cpu_has_const = 1;
    ctx.state.cpu_flushed_const = 1;
    assert!(ctx.state.cpu_const_regs[0].ud0() == 0);

    init_x86regs(&mut ctx.regalloc, &mut ctx.xmm);

    // Walk until the next branch.
    let mut i = startpc;
    ctx.rec.end_block = 0xFFFF_FFFF;
    ctx.rec.branch_to = u32::MAX;

    let mut will_branch3 = 0u32;
    let mut is_timeout_loop = true;
    let mut timeout_reg: i32 = -1;

    loop {
        if i != startpc {
            if (i & 0xffc) == 0 {
                will_branch3 = 1;
                ctx.rec.end_block = i;
                break;
            }
            // if pblock->GetFnptr() != JITCompile: stop — stub.
        }

        ctx.state.ee.code = i; // stand-in for *(int*)PSM(i)

        if is_timeout_loop {
            match ctx.state.ee.code >> 26 {
                8 | 9 => {
                    if timeout_reg >= 0
                        || rs_of(ctx.state.ee.code) != rt_of(ctx.state.ee.code)
                        || (imm_of(ctx.state.ee.code) as i32) >= 0
                    {
                        is_timeout_loop = false;
                    } else {
                        timeout_reg = rs_of(ctx.state.ee.code) as i32;
                    }
                }
                5 => {
                    if timeout_reg != rs_of(ctx.state.ee.code) as i32
                        || rt_of(ctx.state.ee.code) != 0
                    {
                        is_timeout_loop = false;
                    }
                }
                _ if ctx.state.ee.code != 0 => {
                    is_timeout_loop = false;
                }
                _ => {}
            }
        }

        match ctx.state.ee.code >> 26 {
            0 => match funct_of(ctx.state.ee.code) {
                8 | 9 => {
                    ctx.rec.end_block = i + 8;
                    break;
                }
                12 | 13 => {
                    ctx.rec.end_block = i + 4;
                    break;
                }
                _ => {}
            },
            1 => {
                if rt_of(ctx.state.ee.code) < 4 || (rt_of(ctx.state.ee.code) >= 16 && rt_of(ctx.state.ee.code) < 20) {
                    ctx.rec.branch_to = (imm_of(ctx.state.ee.code) as i32 as u32) * 4 + i + 4;
                    if ctx.rec.branch_to > startpc && ctx.rec.branch_to < i {
                        ctx.rec.end_block = ctx.rec.branch_to;
                    } else {
                        ctx.rec.end_block = i + 8;
                    }
                    break;
                }
            }
            2 | 3 => {
                ctx.rec.branch_to = (instruc_target_of(ctx.state.ee.code) << 2) | ((i + 4) & 0xF000_0000);
                ctx.rec.end_block = i + 8;
                break;
            }
            4 | 5 | 6 | 7 | 20 | 21 | 22 | 23 => {
                ctx.rec.branch_to = (imm_of(ctx.state.ee.code) as i32 as u32) * 4 + i + 4;
                if ctx.rec.branch_to > startpc && ctx.rec.branch_to < i {
                    ctx.rec.end_block = ctx.rec.branch_to;
                } else {
                    ctx.rec.end_block = i + 8;
                }
                break;
            }
            16 | 17 | 18 => {
                if rs_of(ctx.state.ee.code) == 8 {
                    ctx.rec.branch_to = (imm_of(ctx.state.ee.code) as i32 as u32) * 4 + i + 4;
                    if ctx.rec.branch_to > startpc && ctx.rec.branch_to < i {
                        ctx.rec.end_block = ctx.rec.branch_to;
                    } else {
                        ctx.rec.end_block = i + 8;
                    }
                    break;
                }
            }
            _ => {}
        }
        i += 4;
    }

    // s_nBlockFF: fast-forward detection (loop without side effects).
    ctx.rec.block_ff = false;
    if ctx.rec.branch_to == startpc {
        ctx.rec.block_ff = true;
        let mut reads: u32 = 0;
        let mut loads: u32 = 1;
        let mut j = startpc;
        while j < ctx.rec.end_block {
            if j == ctx.rec.end_block - 8 {
                j += 4;
                continue;
            }
            let _ = j;
            let code = j; // stand-in for *(u32*)PSM(j)
            if code == 0 {
                j += 4;
                continue;
            }
            // (the full C++ pattern scan is preserved as a stub.)
            j += 4;
            let _ = (reads, loads);
        }
    } else {
        is_timeout_loop = false;
    }
    let _ = is_timeout_loop;
    let _ = timeout_reg;

    // Pass 1 — back-propagate block info.
    if (ctx.inst_cache_size as u32) < (ctx.rec.end_block - startpc) / 4 + 1 {
        ctx.inst_cache = vec![EeInst::default(); ((ctx.rec.end_block - startpc) / 4 + 10) as usize];
        ctx.inst_cache_size = (ctx.rec.end_block - startpc) / 4 + 10;
        assert!(!ctx.inst_cache.is_empty());
    }

    // Pass 2 — emit x86 for each EE instruction.
    ctx.state.ee.code = startpc; // g_pCurInstInfo
    while ctx.rec.branch == 0 && ctx.rec.pc < ctx.rec.end_block {
        recompile_next_instruction(ctx, false, false);
    }

    if ctx.rec.branch == 2 {
        i_flush_call(ctx, FLUSH_EVERYTHING);
        i_branch_test(ctx, 0xFFFF_FFFF);
    } else if will_branch3 != 0 || ctx.rec.branch == 0 {
        let numinsts = (ctx.rec.pc.wrapping_sub(startpc)) / 4;
        if numinsts > 6 {
            set_branch_imm(ctx, ctx.rec.pc);
        } else {
            let addr = &mut ctx.state.ee.pc as *mut _ as uptr;
            emitter::x_mov_mem_imm32(addr, ctx.rec.pc);
            emitter::x_add_r64_imm(XRegister64(0), scaleblockcycles(ctx.rec.block_cycles, 0) as i32);
            let _ = emitter::x_jcc32();
        }
    }
    ctx.rec.rec_ptr = ctx.cache.get_ptr();
    ctx.cur_block = None;
    ctx.cur_block_ex = None;
}

// ---------------------------------------------------------------------------
// `R5900cpu` vtable — the EE 32-bit recompiler registers itself with
// the rest of PCSX2 via this struct.
// ---------------------------------------------------------------------------

/// The EE-32 recompiler vtable, mirroring the C++ `R5900cpu` struct.
pub const REC_CPU: R5900Cpu = R5900Cpu {
    reserve: rec_reserve_trampoline,
    shutdown: rec_shutdown_trampoline,
    reset_ee: rec_reset_ee_trampoline,
    step: rec_step_trampoline,
    execute: rec_execute_trampoline,
    safe_exit: rec_safe_exit_execution_trampoline,
    cancel: rec_cancel_instruction_trampoline,
    clear: rec_clear_trampoline,
};

fn rec_reserve_trampoline(ctx: &mut RecContext) { rec_reserve(ctx); }
fn rec_shutdown_trampoline(ctx: &mut RecContext) { rec_shutdown(ctx); }
fn rec_reset_ee_trampoline(ctx: &mut RecContext) { rec_reset_ee(ctx); }
fn rec_step_trampoline(ctx: &mut RecContext) { rec_step(ctx); }
fn rec_execute_trampoline(ctx: &mut RecContext) { rec_execute(ctx); }
fn rec_safe_exit_execution_trampoline(ctx: &mut RecContext) { rec_safe_exit_execution(ctx); }
fn rec_cancel_instruction_trampoline(ctx: &mut RecContext) { rec_cancel_instruction(ctx); }
fn rec_clear_trampoline(ctx: &mut RecContext, addr: u32, size: u32) { rec_clear(ctx, addr, size); }

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cop2flags_basic() {
        // Not COP2 -> 0
        assert_eq!(cop2flags(0x0000_0000), 0);
        // COP2, but bit 25 clear (transfer/branch) -> 0
        assert_eq!(cop2flags(0x0400_0000), 0);
        // VADDq -> 3 (default COP2 write to status)
        assert_eq!(cop2flags(0x4A00_0020) & 3, 3);
        // WAITQ -> 0
        assert_eq!(cop2flags(0x4A00_03BF) & 3, 0);
    }

    #[test]
    fn cop2_div_timings() {
        assert_eq!(cop2_div_unit_timings(0x4A00_03BC), 6); // DIV
        assert_eq!(cop2_div_unit_timings(0x4A00_03BD), 6); // SQRT
        assert_eq!(cop2_div_unit_timings(0x4A00_03BE), 12); // RSQRT
        assert_eq!(cop2_div_unit_timings(0x4A00_03BF), 0); // WAITQ
    }

    #[test]
    fn cop2_is_qop_recognises() {
        // VADDq
        assert!(cop2_is_qop(0x4A00_0020));
        // VMADDq
        assert!(cop2_is_qop(0x4A00_0021));
        // VMULAq (full 0x7FF)
        assert!(cop2_is_qop(0x4BC0_001C));
        // WAITQ — not Q
        assert!(!cop2_is_qop(0x4A00_03BF));
    }

    #[test]
    fn scaleblockcycles_is_at_least_one() {
        assert!(scaleblockcycles(0, 0) >= 1);
        assert!(scaleblockcycles(0, 1) >= 1);
        assert!(scaleblockcycles(40, 0) >= 1);
    }

    #[test]
    fn alloc_x86reg_assigns_and_finds_free() {
        let mut ctx = RecContext::new();
        let host = alloc_x86reg(&mut ctx.regalloc, &mut ctx.state, X86Type::Gpr, 5, MODE_READ);
        assert!(host >= 0);
        // The same reg is now findable via check.
        let found = check_x86reg(&mut ctx.regalloc, &mut ctx.state, X86Type::Gpr, 5, MODE_READ);
        assert_eq!(found, host);
    }

    #[test]
    fn flush_const_regs_advances_mask() {
        let mut ctx = RecContext::new();
        ctx.state.cpu_const_regs[1].set_ud0(42);
        ctx.state.cpu_has_const = 1u32 << 1;
        ctx.state.cpu_flushed_const = 0;
        flush_const_regs(&mut ctx.state, true);
        // After flushing, the constant is gone.
        assert_eq!(ctx.state.cpu_has_const & (1u32 << 1), 0);
    }

    #[test]
    fn try_swap_delay_slot_handles_nop() {
        let mut ctx = RecContext::new();
        ctx.state.ee.code = 0; // NOP
        assert!(try_swap_delay_slot(&mut ctx, 1, 2, 3, false));
    }
}
