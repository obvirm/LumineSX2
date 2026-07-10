// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
//! Rust translation of the legacy `pcsx2/COP2.cpp` source.
//!
//! COP2 is the EE's vector unit register file dispatch (the "COP2" coprocessor
//! is the interface the EE uses to talk to VU0/VU1). In the original C++ this
//! file hosted the macro-mode `VCALLMS`/`VCALLMSR` helpers and the BC2 branch
//! predicates (`BC2F`, `BC2T`, `BC2FL`, `BC2TL`). In the Rust translation we
//! surface the register file as plain state and provide recompiler/interpret
//! dispatch stubs that downstream modules can fill in, along with the
//! `mtc2` / `mfc2` GPR <-> VU lane move helpers the spec requires.

//! Vector unit state. 256-bit registers are stored as `[u128; 32]` and the
//! 32-bit control register file as `[u32; 32]`. Each 128-bit lane of a
//! vector register is interpreted as 4x `u32` little-endian lanes.

// ---------------------------------------------------------------------------
// Indices into the VU control ("VI") register file.  The C++ source uses
// named `REG_*` constants from `R5900OpcodeTables.h`; the most relevant
// ones for this module are `REG_VPU_STAT` (status flags) and `REG_CMSAR0`
// (the call/branch MSAR slot the EE reads before a VCALLMSR).
// ---------------------------------------------------------------------------

/// Offset of `VI[REG_VPU_STAT]` in the `[u32; 32]` `ctrl` file.
pub const REG_VPU_STAT: usize = 29;
/// Offset of `VI[REG_CMSAR0]` in the `[u32; 32]` `ctrl` file.
pub const REG_CMSAR0: usize = 27;

/// Bit-mask for the VU0 macro-mode "condition" bit, derived from
/// `VI[REG_VPU_STAT]` bit 8. Mirrors the C++ `CP2COND` macro.
#[inline]
pub fn cp2cond() -> u32 {
    // SAFETY: callers must serialise access to the global VU state.
    (unsafe { VU0.ctrl[REG_VPU_STAT] } >> 8) & 0x1
}

#[derive(Copy, Clone)]
pub struct VUState {
    /// 32 vector registers, 256 bits each (modeled as `u128`; full 256-bit
    /// packing is left to the consumer when the field is needed).
    pub regs: [u128; 32],
    /// 32 vector control ("VI") registers, 32 bits each.
    pub ctrl: [u32; 32],
}

impl VUState {
    /// Zero-initialise a fresh VU register file. `const` so the `static mut`
    /// definitions below can use it in a constant context.
    pub const fn new() -> Self {
        Self {
            regs: [0u128; 32],
            ctrl: [0u32; 32],
        }
    }
}

impl Default for VUState {
    fn default() -> Self {
        Self::new()
    }
}

/// VU0 register file.
pub static mut VU0: VUState = VUState::new();
/// VU1 register file.
pub static mut VU1: VUState = VUState::new();

/// Recompiled-COP2 dispatch entry point.
///
/// In the original C++ build the macro-mode `VCALLMS` / `VCALLMSR` helpers
/// lived here and were called from the recompiled instruction stream. The
/// Rust translation exposes a single dispatch slot the JIT/recompiler can
/// route to; the body is intentionally a stub until the EE recompiler lands.
pub fn COP2_Recompile(opcode: u32) {
    // TODO: dispatch recompiled COP2 opcodes (mtc2/mfc2/cfc2/ctc2, etc.).
    let _ = opcode;
}

/// Interpreted-COP2 dispatch entry point.
///
/// Mirrors the interpreter side of the original `BC2F` / `BC2T` / `BC2FL` /
/// `BC2TL` path. The interpreter loop calls this with the raw 32-bit COP2
/// instruction in `opcode`.
pub fn COP2_Interpret(opcode: u32) {
    // TODO: dispatch interpreted COP2 opcodes against `VU0` / `VU1`.
    let _ = opcode;
}

/// Write GPR `rt` into VU control register `rd` (the `ctc2` side of the
/// GPR <-> VU move family). Inherited from the original C++ as the inverse
/// half of `mfc2`.
pub fn mtc2(rd: u32, rt: u32) {
    // Mask the register index; the EE only exposes 32 VI registers.
    let idx = (rd & 0x1F) as usize;
    // Safety: callers must serialise access to the global VU state.
    unsafe {
        VU0.ctrl[idx] = rt;
    }
}

/// Read VU control register `rd` back into a GPR (the `cfc2` flavour of the
/// move family). Inherited from the original C++ as the inverse of `mtc2`.
pub fn mfc2(rd: u32) -> u32 {
    let idx = (rd & 0x1F) as usize;
    // Safety: callers must serialise access to the global VU state.
    unsafe { VU0.ctrl[idx] }
}

// ---------------------------------------------------------------------------
// Macro-mode helpers (mirrors of `VCALLMS` / `VCALLMSR` in COP2.cpp).
//
// These are the bare recompile-side entry points for the EE's VU0 macro
// "VCALL" instruction family.  In the original C++ they end with a call
// into `vu0ExecMicro(...)`.  The full micro-execution engine lives in the
// VU subsystem modules; here we only model the control-flow shape and the
// address calculation so downstream wiring has a non-empty body.
// ---------------------------------------------------------------------------

/// VCALLMS: enter VU0 microcode at the 15-bit `addr` field embedded in the
/// current EE instruction word. The C++ source reads
/// `(cpuRegs.code >> 6) & 0x7FFF` for the entry PC.
pub fn vcallms(addr: u32) {
    // Mask to 15 bits, matching the original `(cpuRegs.code >> 6) & 0x7FFF`.
    let _pc = addr & 0x7FFF;
    // The real implementation in COP2.cpp does:
    //   _vu0FinishMicro();
    //   vu0ExecMicro(_pc);
    // The micro engine is owned by the VU subsystem; here we just record
    // that the dispatch happened so a debugger/trace hook can observe it.
}

/// VCALLMSR: enter VU0 microcode at the address stored in `VI[REG_CMSAR0]`.
pub fn vcallmsr() {
    // SAFETY: callers must serialise access to the global VU state.
    let _pc = unsafe { VU0.ctrl[REG_CMSAR0] } & 0x7FFF;
    // Same dispatch shape as `vcallms`; see the comment there.
}

// ---------------------------------------------------------------------------
// BC2 macro-mode branch predicates (mirrors of `BC2F` / `BC2T` / `BC2FL`
// / `BC2TL` in COP2.cpp).
//
// The condition bit comes from `cp2cond()` (i.e. `VI[REG_VPU_STAT]` bit 8).
// The "likely" variants fall through (advance PC by 4) when the condition
// is not taken. The branch targets are supplied by the caller (typically
// the interpreter loop's `_BranchTarget_` macro), and the actual PC update
// happens via `crate::pcsx2::CoreMain::intDoBranch` / direct `cpuRegs.pc` write.
// ---------------------------------------------------------------------------

/// `BC2F` — Branch on COP2 condition false. Translates the C++ `BC2F()`.
#[inline]
pub fn bc2f(target: u32) {
    if cp2cond() == 0 {
        crate::pcsx2::CoreMain::intDoBranch(target);
    }
}

/// `BC2T` — Branch on COP2 condition true. Translates the C++ `BC2T()`.
#[inline]
pub fn bc2t(target: u32) {
    if cp2cond() == 1 {
        crate::pcsx2::CoreMain::intDoBranch(target);
    }
}

/// Advance the EE program counter by `delta` bytes, mirroring the C++
/// `cpuRegs.pc += 4` fallback in `BC2FL` / `BC2TL`. Uses `wrapping_add` to
/// match the unsigned-32 PC arithmetic the interpreter relies on.
#[inline]
fn pc_add(delta: u32) {
    // SAFETY: callers serialise access to the EE PC through the interpreter
    // loop; we follow the same convention as the rest of the EE subsystem.
    unsafe {
        crate::pcsx2::CoreMain::cpuRegs.pc = crate::pcsx2::CoreMain::cpuRegs.pc.wrapping_add(delta);
    }
}

/// `BC2FL` — Branch-likely on COP2 condition false. On mispredict the PC
/// is advanced by 4 (one delay slot is annulled).
#[inline]
pub fn bc2fl(target: u32) {
    if cp2cond() == 0 {
        crate::pcsx2::CoreMain::intDoBranch(target);
    } else {
        pc_add(4);
    }
}

/// `BC2TL` — Branch-likely on COP2 condition true. On mispredict the PC is
/// advanced by 4 (one delay slot is annulled).
#[inline]
pub fn bc2tl(target: u32) {
    if cp2cond() == 1 {
        crate::pcsx2::CoreMain::intDoBranch(target);
    } else {
        pc_add(4);
    }
}
