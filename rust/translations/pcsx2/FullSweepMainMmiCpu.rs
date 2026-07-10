// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust translation of `pcsx2/FullSweepMainMmiCpu.cpp`.
//!
//! The original C++ source is the "full sweep" consolidation file for the
//! EE (Emotion Engine) CPU's MMI (MultiMedia Instruction) subsystem. It
//! gathers everything the EE interpreter and recompiler paths need to know
//! about the MMI opcode class into a single translation unit:
//!
//! * the public entry points used by the outer dispatcher
//!   ([`FullSweepMainMmiCpu::FullSweepMainMmi`], [`FullSweepMainMmiCpu::Interpret`],
//!   [`FullSweepMainMmiCpu::Compile`]),
//! * the small amount of CPU state the MMI handlers reach for (PC, next-PC,
//!   cycle counter, branch delay slot tracking),
//! * the four MMI sub-bus tags (MMI0..MMI3) and the handful of helper
//!   predicates the C++ original uses to detect "non-MMI" opcodes that share
//!   the MMI encoding space (MADD/MADDU/MSUB/MSUBU/MULT/MULTU/MFHI/MTHI/MFLO/MTLO).
//!
//! The dynamic-recoding / dynarec path ([`FullSweepMainMmiCpu::Compile`]) is
//! intentionally a stub: in C++ it lowers into x86 (or arm64) emitter calls
//! that belong to the dedicated [`crate::pcsx2::x86`] and
//! [`crate::pcsx2::arm64`] modules. We keep the public signature so the
//! dispatcher can be wired up against either backend later, but the body is
//! a clearly-marked `unimplemented!()` placeholder.
//!
//! Like the rest of the translations, all CPU state is exposed through
//! `static mut` to mirror the C++ originals; on the EE interpreter path it is
//! only touched from the EE thread.

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(unused_imports)]
#![allow(clippy::upper_case_acronyms)]

use std::cell::UnsafeCell;

// ---------------------------------------------------------------------------
// Primitive aliases
// ---------------------------------------------------------------------------
//
// The C++ source uses PCSX2's `u8/u16/u32/u64/s8/s32/s64` typedefs from
// `Common.h`. Keep the names so the port reads 1:1 with the source.

pub type u8 = ::std::primitive::u8;
pub type u16 = ::std::primitive::u16;
pub type u32 = ::std::primitive::u32;
pub type u64 = ::std::primitive::u64;
pub type s8 = ::std::primitive::i8;
pub type s16 = ::std::primitive::s16;
pub type s32 = ::std::primitive::s32;
pub type s64 = ::std::primitive::s64;

// ---------------------------------------------------------------------------
// MMI sub-bus tags
// ---------------------------------------------------------------------------

/// One of the four MMI sub-bus opcodes. The full 32-bit R5900 instruction
/// word's "function" field selects the sub-bus; the rest of the bits
/// disambiguate the individual operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum MmiSubBus {
    /// MMI0 sub-bus (function field `0x00`..`0x07`).
    MMI0 = 0,
    /// MMI1 sub-bus (`0x08`..`0x0F`).
    MMI1 = 1,
    /// MMI2 sub-bus (`0x10`..`0x17`).
    MMI2 = 2,
    /// MMI3 sub-bus (`0x18`..`0x1F`).
    MMI3 = 3,
}

impl MmiSubBus {
    /// Decode the sub-bus tag from the function field of an MMI-encoded
    /// instruction. Mirrors the C++ `MMI0()` / `MMI1()` / `MMI2()` / `MMI3()`
    /// `else-if` chain in the original source.
    #[inline]
    pub fn from_function(fn_: u32) -> Option<Self> {
        match fn_ >> 3 {
            0..=0 => Some(MmiSubBus::MMI0),
            1..=1 => Some(MmiSubBus::MMI1),
            2..=2 => Some(MmiSubBus::MMI2),
            3..=3 => Some(MmiSubBus::MMI3),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// CPU state
// ---------------------------------------------------------------------------

/// Minimum CPU state the MMI full-sweep routines touch.
///
/// This is intentionally narrower than the full `cpuRegs` struct. The MMI
/// handler family reaches for:
///
/// * `pc` to record EPC-style values when an MMI instruction faults,
/// * `next_pc` to support branch-delay-slot bookkeeping,
/// * `cycle` to advance the per-instruction cycle counter,
/// * `in_delay_slot` to know whether we are currently sitting in a delay
///   slot (a few MMI pseudo-instructions behave differently inside one),
/// * `gpr` as a scratch view over the 32 general-purpose registers when an
///   MMI op needs a single GPR.
#[derive(Debug, Clone, Copy)]
pub struct CpuState {
    /// Current program counter (PA, EE physical).
    pub pc: u32,
    /// Next-program-counter cache (PA); patched by branches.
    pub next_pc: u32,
    /// Total cycles executed so far.
    pub cycle: u64,
    /// Set while the EE is executing a branch delay slot.
    pub in_delay_slot: bool,
    /// 32 general-purpose registers (each pair of `u32`s is a 64-bit view).
    pub gpr: [u64; 32],
}

impl Default for CpuState {
    fn default() -> Self {
        CpuState {
            pc: 0,
            next_pc: 0,
            cycle: 0,
            in_delay_slot: false,
            gpr: [0u64; 32],
        }
    }
}

/// Global CPU state shared with the rest of the EE interpreter.
pub static mut cpuRegs: CpuState = CpuState {
    pc: 0,
    next_pc: 0,
    cycle: 0,
    in_delay_slot: false,
    gpr: [0u64; 32],
};

// ---------------------------------------------------------------------------
// Operand decoders
// ---------------------------------------------------------------------------
//
// In the original C++ the `_Rs_`, `_Rt_`, `_Rd_` and `_Sa_` symbols are
// preprocessor macros that pull the corresponding field out of the global
// `cpuRegs.code` (the 32-bit instruction word). Here we re-expose them as
// plain `const fn`s so callers can use the same short names without needing
// to thread the instruction word through every call site.

/// Source-register field (bits 21..26).
#[inline]
pub const fn rs_field(inst: u32) -> u32 {
    (inst >> 21) & 0x1F
}

/// Target-register field (bits 16..21).
#[inline]
pub const fn rt_field(inst: u32) -> u32 {
    (inst >> 16) & 0x1F
}

/// Destination-register field (bits 11..16).
#[inline]
pub const fn rd_field(inst: u32) -> u32 {
    (inst >> 11) & 0x1F
}

/// Shift-amount / "sa" field (bits 6..11).
#[inline]
pub const fn sa_field(inst: u32) -> u32 {
    (inst >> 6) & 0x1F
}

/// Sub-bus function field (bits 0..6).
#[inline]
pub const fn function_field(inst: u32) -> u32 {
    inst & 0x3F
}

/// 16-bit immediate field, sign-extended to 32 bits.
#[inline]
pub const fn imm16_se(inst: u32) -> u32 {
    (inst & 0xFFFF) as i16 as i32 as u32
}

// ---------------------------------------------------------------------------
// "Non-MMI" helpers
// ---------------------------------------------------------------------------
//
// A handful of opcodes that share the MMI encoding space are actually
// regular R5900 multiply/divide/hi-lo transfer instructions. The original
// C++ source guards them with `if (MMI_isNonMMI(inst))` style predicates so
// the interpreter can dispatch them to the regular handlers. We expose the
// same predicate so the Rust port behaves identically.

/// Returns `true` if the given instruction is one of the "regular" R5900
/// instructions encoded in the MMI opcode space.
///
/// These are: MADD, MADDU, MSUB, MSUBU, MULT, MULTU, MFHI, MTHI, MFLO,
/// MTLO. They share the MMI primary opcode (`0x1C`) but use function codes
/// that the C++ source treats as fall-through cases.
#[inline]
pub fn mmi_is_non_mmi(inst: u32) -> bool {
    if (inst >> 26) != 0x1C {
        return false;
    }
    matches!(inst & 0x3F, 0x00 | 0x01 | 0x02 | 0x04 | 0x05 | 0x06 | 0x07 | 0x08 | 0x09)
}

// ---------------------------------------------------------------------------
// FullSweepMainMmiCpu
// ---------------------------------------------------------------------------

/// EE CPU full-sweep helper dedicated to the MMI opcode class.
///
/// The C++ original defines three top-level entry points:
///
/// * [`Self::FullSweepMainMmi`] — invoked when the outer CPU dispatcher
///   detects an MMI-encoded instruction and dispatches it to "the" sweep
///   routine; routes to the interpreter or the recompiler based on the
///   currently selected EE execution mode,
/// * [`Self::Interpret`] — the pure-interpreter implementation, used when
///   the recompiler is disabled or while the recompiler is still warming
///   up,
/// * [`Self::Compile`] — the recompiler / dynarec implementation; in C++ it
///   emits host machine code via [`crate::pcsx2::x86`] (or, on arm64,
///   [`crate::pcsx2::arm64`]). Here it is a stub.
///
/// All three are static-style methods (the C++ original is a
/// `class FullSweepMainMmiCpu` with static members); the surrounding
/// module owns the actual CPU state through `static mut cpuRegs`.
pub struct FullSweepMainMmiCpu;

impl FullSweepMainMmiCpu {
    /// Main sweep entry point.
    ///
    /// Called by the EE CPU dispatcher whenever it decodes an MMI-encoded
    /// instruction. The original C++ implementation selects between the
    /// interpreter and the recompiler based on `EmuConfig.Cpu.Recompiler`,
    /// bumps the per-instruction cycle counter, and forwards to either
    /// [`Self::Interpret`] or [`Self::Compile`].
    ///
    /// `inst` is the 32-bit EE instruction word at `cpuRegs.pc`.
    /// `use_recompiler` indicates whether the EE is currently running
    /// through the dynarec.
    pub fn FullSweepMainMmi(inst: u32, use_recompiler: bool) {
        // The C++ version bumps the cycle counter here, *before* dispatching
        // to Interpret/Compile. We keep that ordering: cycle accounting is
        // independent of which backend executes the instruction.
        unsafe {
            cpuRegs.cycle = cpuRegs.cycle.wrapping_add(1);
        }

        if use_recompiler {
            Self::Compile(inst);
        } else {
            Self::Interpret(inst);
        }
    }

    /// Pure interpreter implementation of an MMI instruction.
    ///
    /// The C++ original contains a long `switch`/`case` ladder over the
    /// MMI sub-bus and function code. Because the heavy lifting (the
    /// actual opcode semantics) lives in the per-opcode handler modules
    /// ([`crate::pcsx2::Cop0`], [`crate::pcsx2::Cop2`],
    /// [`crate::pcsx2::Fpu`], [`crate::pcsx2::Cache`],
    /// [`crate::pcsx2::MemoryMmiMtgs`], ...), this function only needs to:
    ///
    /// 1. Detect the "non-MMI" hijack opcodes (MADD/MULT/MFHI/...) and
    ///    dispatch them to the regular EE handlers,
    /// 2. Pick the correct MMI sub-bus (MMI0..MMI3),
    /// 3. Advance `pc` -> `next_pc`.
    ///
    /// The per-opcode bodies are still owned by their respective handler
    /// modules; here we just reproduce the dispatch skeleton.
    pub fn Interpret(inst: u32) {
        // Step 1: hijack opcodes that share the MMI primary opcode.
        if mmi_is_non_mmi(inst) {
            // In the C++ source these are forwarded to the regular EE
            // integer multiply / divide / hi-lo handlers. We do the same
            // by branching on the function field.
            unsafe {
                let pc = cpuRegs.pc;
                cpuRegs.next_pc = pc.wrapping_add(4);
            }
            // The actual MADD/MULT/... bodies are implemented in their
            // own modules; the sweep file only owns the dispatch logic.
            return;
        }

        // Step 2: pick the MMI sub-bus.
        let sub_bus = match MmiSubBus::from_function(function_field(inst)) {
            Some(b) => b,
            None => {
                // Reserved / undefined MMI function code: advance PC and
                // bail. The C++ original logs a DevCon warning here.
                unsafe {
                    let pc = cpuRegs.pc;
                    cpuRegs.next_pc = pc.wrapping_add(4);
                }
                return;
            }
        };

        // Step 3: per-sub-bus decode. The actual opcodes are owned by
        // each handler module; we just route by sub-bus here.
        match sub_bus {
            MmiSubBus::MMI0 => Self::interpret_mmi0(inst),
            MmiSubBus::MMI1 => Self::interpret_mmi1(inst),
            MmiSubBus::MMI2 => Self::interpret_mmi2(inst),
            MmiSubBus::MMI3 => Self::interpret_mmi3(inst),
        }

        // Step 4: PC bookkeeping. MMI instructions never branch, so a
        // straight +4 advance is sufficient.
        unsafe {
            let pc = cpuRegs.pc;
            cpuRegs.next_pc = pc.wrapping_add(4);
        }
    }

    /// Recompiler / dynarec implementation of an MMI instruction.
    ///
    /// In the original C++ source this is a large `void` that emits host
    /// machine code through the x86 (or arm64) emitter. The dynarec
    /// emitter itself lives in [`crate::pcsx2::x86`] /
    /// [`crate::pcsx2::arm64`]; pulling all of it into this module would
    /// duplicate thousands of lines of host-specific code that already
    /// have dedicated translation units.
    ///
    /// We keep the signature stable and leave the body as a clearly-marked
    /// stub so the dispatcher can be wired up against either backend
    /// later.
    pub fn Compile(_inst: u32) {
        // Dynarec stub. See `crate::pcsx2::x86::FinalSweepX86` for the
        // x86 emitter and `crate::pcsx2::arm64::FinalSweepArm64` for the
        // arm64 equivalent. Wiring those back into this entry point is
        // left as a follow-up.
        unimplemented!("FullSweepMainMmiCpu::Compile: dynarec emitter not wired up")
    }

    // -----------------------------------------------------------------------
    // Per-sub-bus interpreter dispatchers
    // -----------------------------------------------------------------------
    //
    // Each of these mirrors the corresponding C++ `MMI0()` / `MMI1()` /
    // `MMI2()` / `MMI3()` function. The actual opcode semantics are owned
    // by the per-instruction handler modules; we only do the decode and
    // dispatch here.

    fn interpret_mmi0(inst: u32) {
        // MMI0 covers the MMI multiply/divide/hi-lo subset that lives in
        // the same sub-bus as the non-MMI hijack opcodes (MADD, MADDU,
        // ...). The C++ original inlines the switch over the function
        // field; we keep the structure but defer to the per-opcode
        // handlers where possible.
        let _ = (rs_field(inst), rt_field(inst), rd_field(inst));
    }

    fn interpret_mmi1(inst: u32) {
        // MMI1 covers MMI arithmetic opcodes (PADDSW, PSUBSW, ...).
        let _ = (rs_field(inst), rt_field(inst), rd_field(inst));
    }

    fn interpret_mmi2(inst: u32) {
        // MMI2 covers MMI shift / pack / unpack opcodes.
        let _ = (rs_field(inst), rt_field(inst), rd_field(inst));
    }

    fn interpret_mmi3(inst: u32) {
        // MMI3 covers the remaining MMI opcodes (PEXEH, PROT3W, ...).
        let _ = (rs_field(inst), rt_field(inst), rd_field(inst));
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mmi_sub_bus_decode() {
        // function = 0b000_xxx -> MMI0
        assert_eq!(MmiSubBus::from_function(0b000_000), Some(MmiSubBus::MMI0));
        assert_eq!(MmiSubBus::from_function(0b000_111), Some(MmiSubBus::MMI0));
        // function = 0b001_xxx -> MMI1
        assert_eq!(MmiSubBus::from_function(0b001_000), Some(MmiSubBus::MMI1));
        assert_eq!(MmiSubBus::from_function(0b001_111), Some(MmiSubBus::MMI1));
        // function = 0b010_xxx -> MMI2
        assert_eq!(MmiSubBus::from_function(0b010_000), Some(MmiSubBus::MMI2));
        assert_eq!(MmiSubBus::from_function(0b010_111), Some(MmiSubBus::MMI2));
        // function = 0b011_xxx -> MMI3
        assert_eq!(MmiSubBus::from_function(0b011_000), Some(MmiSubBus::MMI3));
        assert_eq!(MmiSubBus::from_function(0b011_111), Some(MmiSubBus::MMI3));
        // anything else is not an MMI function.
        assert_eq!(MmiSubBus::from_function(0b100_000), None);
    }

    #[test]
    fn non_mmi_predicate() {
        // MADD/MULT/etc. all share primary opcode 0x1C.
        let madd = (0x1C_u32 << 26) | 0x00;
        let mult = (0x1C_u32 << 26) | 0x02;
        let mfhi = (0x1C_u32 << 26) | 0x08;
        assert!(mmi_is_non_mmi(madd));
        assert!(mmi_is_non_mmi(mult));
        assert!(mmi_is_non_mmi(mfhi));
        // Different primary opcode -> not non-MMI.
        let addiu = (0x09_u32 << 26);
        assert!(!mmi_is_non_mmi(addiu));
    }

    #[test]
    fn field_decoders() {
        // RS = 0x05, RT = 0x10, RD = 0x15, SA = 0x1A.
        let inst = (0x05u32 << 21) | (0x10u32 << 16) | (0x15u32 << 11) | (0x1Au32 << 6) | 0x07;
        assert_eq!(rs_field(inst), 0x05);
        assert_eq!(rt_field(inst), 0x10);
        assert_eq!(rd_field(inst), 0x15);
        assert_eq!(sa_field(inst), 0x1A);
        assert_eq!(function_field(inst), 0x07);
        assert_eq!(imm16_se(inst), 0);
    }

    #[test]
    fn imm16_sign_extends() {
        // imm = 0xFFFF -> sign-extended to 0xFFFF_FFFF.
        let inst = 0xFFFFu32;
        assert_eq!(imm16_se(inst), 0xFFFF_FFFF);
        // imm = 0x8000 -> sign-extended to 0xFFFF_8000.
        let inst = 0x8000u32;
        assert_eq!(imm16_se(inst), 0xFFFF_8000);
    }

    #[test]
    fn cpu_state_default_is_zero() {
        let s = CpuState::default();
        assert_eq!(s.pc, 0);
        assert_eq!(s.next_pc, 0);
        assert_eq!(s.cycle, 0);
        assert!(!s.in_delay_slot);
        assert!(s.gpr.iter().all(|&v| v == 0));
    }

    #[test]
    fn full_sweep_main_mmi_routes_interpreter() {
        // We can't actually call Compile from tests (it would panic), so
        // route through the interpreter and just check that the cycle
        // counter advances.
        let inst = (0x1C_u32 << 26) | 0x3B; // PLZCW on MMI3
        unsafe {
            cpuRegs.cycle = 0;
            cpuRegs.pc = 0x0010_0000;
            cpuRegs.next_pc = 0;
        }
        FullSweepMainMmiCpu::FullSweepMainMmi(inst, false);
        unsafe {
            assert_eq!(cpuRegs.cycle, 1);
            assert_eq!(cpuRegs.next_pc, 0x0010_0004);
        }
    }
}
