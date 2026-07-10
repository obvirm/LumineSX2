// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust translation of the C/C++ PCSX2 core.
//!
//! This module rolls up the small C/C++ translation units listed in the
//! rewrite brief (R3000A, R5900, Config, GameList, GameDatabase, Patch,
//! Memory, MTGS, MTVU, HW, Achievements) into a single Rust 2021 file with
//! `std` only.  The goal is structural fidelity, not cycle-accurate
//! emulation — globals are exposed as `static mut` mirroring the C
//! originals, and dispatch tables are emitted as plain `static` arrays of
//! function pointers.

#![allow(dead_code)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]
#![allow(static_mut_refs)]
#![allow(unused_assignments)]
#![allow(unused_variables)]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Primitive aliases (mirror Pcsx2Defs.h)
// ---------------------------------------------------------------------------

pub type u8 = std::primitive::u8;
pub type u16 = std::primitive::u16;
pub type u32 = std::primitive::u32;
pub type u64 = std::primitive::u64;
pub type u128 = [u64; 2];
pub type s8 = std::primitive::i8;
pub type s16 = std::primitive::i16;
pub type s32 = std::primitive::i32;
pub type s64 = std::primitive::i64;
pub type s128 = [i64; 2];

pub type uptr = usize;

// ---------------------------------------------------------------------------
// R3000A — IOP (PlayStation 1) CPU state, interpreter, opcode tables.
// ---------------------------------------------------------------------------

#[repr(C)]
pub struct GPRRegs {
    pub r: [u32; 34], // 32 GPRs + lo(33) + hi(32)
}

impl Default for GPRRegs {
    fn default() -> Self { Self { r: [0; 34] } }
}

#[repr(C)]
#[derive(Default)]
pub struct CP0Regs {
    pub r: [u32; 32],
}

#[derive(Default)]
pub struct CP2Data {
    pub r: [u32; 32],
}

#[derive(Default)]
pub struct CP2Ctrl {
    pub r: [u32; 32],
}

/// All CPU-visible state for the IOP / R3000A.
#[derive(Default)]
pub struct R3000AState {
    pub GPR: GPRRegs,
    pub CP0: CP0Regs,
    pub CP2D: CP2Data,
    pub CP2C: CP2Ctrl,
    pub pc: u32,
    pub code: u32,
    pub cycle: u64,
    pub interrupt: u32,
    pub pcWriteback: u32,
    pub iopNextEventCycle: u64,
    pub iopBreak: s32,
    pub iopCycleEE: s32,
    pub iopCycleEECarry: u32,
    pub sCycle: [u64; 32],
    pub eCycle: [s32; 32],
}

pub static mut psxRegs: R3000AState = R3000AState {
    GPR: GPRRegs { r: [0; 34] },
    CP0: CP0Regs { r: [0; 32] },
    CP2D: CP2Data { r: [0; 32] },
    CP2C: CP2Ctrl { r: [0; 32] },
    pc: 0,
    code: 0,
    cycle: 0,
    interrupt: 0,
    pcWriteback: 0,
    iopNextEventCycle: 0,
    iopBreak: 0,
    iopCycleEE: 0,
    iopCycleEECarry: 0,
    sCycle: [0; 32],
    eCycle: [0; 32],
};

pub static mut iopEventAction: bool = false;
pub static mut iopEventTestIsActive: bool = false;
pub static mut iopIsDelaySlot: bool = false;

/// IOP clock (Hz) — PlayStation 1 bus clock.
pub static mut PSXCLK: u32 = 36_864_000;
/// EE clock (Hz) — Emotion Engine bus clock.
pub static mut PS2CLK: u32 = 294_912_000;
pub static mut psxNextDeltaCounter: s32 = 0;
pub static mut psxNextStartCounter: u64 = 0;

/// Generic CPU interface (Reset / ExecuteBlock / Clear / Shutdown).
pub struct R3000Acpu {
    pub reserve: Option<fn()>,
    pub reset: Option<fn()>,
    pub execute_block: Option<fn(s32) -> s32>,
    pub clear: Option<fn(u32, u32)>,
    pub shutdown: Option<fn()>,
}

pub static mut psxCpu: Option<&'static mut R3000Acpu> = None;
pub static mut psxInt: R3000Acpu = R3000Acpu {
    reserve: None,
    reset: None,
    execute_block: None,
    clear: None,
    shutdown: None,
};
pub static mut psxRec: R3000Acpu = R3000Acpu {
    reserve: None,
    reset: None,
    execute_block: None,
    clear: None,
    shutdown: None,
};

/// Reset the IOP / R3000A to its post-BIOS state (PC = 0xbfc00000).
pub fn psxReset() {
    unsafe {
        psxRegs = R3000AState {
            GPR: GPRRegs { r: [0; 34] },
            CP0: CP0Regs { r: [0; 32] },
            CP2D: CP2Data { r: [0; 32] },
            CP2C: CP2Ctrl { r: [0; 32] },
            pc: 0xbfc00000,
            code: 0,
            cycle: 0,
            interrupt: 0,
            pcWriteback: 0,
            iopNextEventCycle: 4,
            iopBreak: 0,
            iopCycleEE: -1,
            iopCycleEECarry: 0,
            sCycle: [0; 32],
            eCycle: [0; 32],
        };
        psxRegs.CP0.r[12] = 0x00400000; // Status: BEV = 1
        psxRegs.CP0.r[15] = 0x0000001f; // PRid
        PSXCLK = 36_864_000;
    }
}

/// One-shot IOP init — currently just delegates to reset.
pub fn psxInit() {
    psxReset();
}

/// Placeholder IOP execute block: drains the EE cycle budget by
/// accumulating into `iopCycleEE` and returns a synthetic remaining
/// cycle count.  A real interpreter lives in `R3000AInterpreter.cpp`;
/// this stub keeps the dispatch loop shape identical.
pub fn psxExecuteBlock(ee_cycles: s32) -> s32 {
    unsafe {
        psxRegs.iopBreak = 0;
        psxRegs.iopCycleEE = ee_cycles;
        let last = psxRegs.cycle;
        // The interpreter loop runs `while (iopCycleEE > 0)`.  We
        // simulate one chunk of work per call.
        if psxRegs.iopCycleEE > 0 {
            psxRegs.iopCycleEE -= (psxRegs.cycle - last) as s32 * 8;
        }
        psxRegs.iopBreak + psxRegs.iopCycleEE
    }
}

pub fn psxException(code: u32, bd: u32) {
    unsafe {
        psxRegs.CP0.r[13] = (psxRegs.CP0.r[13] & !0x7f) | (code & 0x7f);
        if bd != 0 {
            psxRegs.CP0.r[13] |= 0x80000000;
            psxRegs.CP0.r[14] = psxRegs.pc.wrapping_sub(4);
        } else {
            psxRegs.CP0.r[14] = psxRegs.pc;
        }
        psxRegs.pc = if (psxRegs.CP0.r[12] & 0x400000) != 0 {
            0xbfc00180
        } else {
            0x80000080
        };
        psxRegs.CP0.r[12] = (psxRegs.CP0.r[12] & !0x3f) | ((psxRegs.CP0.r[12] & 0xf) << 2);
    }
}

pub fn iopEventTest() {
    unsafe {
        psxRegs.iopNextEventCycle = psxRegs.cycle + 384;
    }
}

// --- PSX instruction implementations (R3000AInterpreter / OpcodeTables) ---

type PsxOp = fn() -> ();

pub fn psxADDI() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; let rt = ((psxRegs.code >> 16) & 0x1f) as usize; if rt != 0 { let imm = (psxRegs.code & 0xffff) as i16 as i32 as u32; psxRegs.GPR.r[rt] = psxRegs.GPR.r[rs].wrapping_add(imm); } } }
pub fn psxADDIU() { psxADDI(); }
pub fn psxANDI() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; let rt = ((psxRegs.code >> 16) & 0x1f) as usize; if rt != 0 { psxRegs.GPR.r[rt] = psxRegs.GPR.r[rs] & (psxRegs.code & 0xffff); } } }
pub fn psxORI() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; let rt = ((psxRegs.code >> 16) & 0x1f) as usize; if rt != 0 { psxRegs.GPR.r[rt] = psxRegs.GPR.r[rs] | (psxRegs.code & 0xffff); } } }
pub fn psxXORI() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; let rt = ((psxRegs.code >> 16) & 0x1f) as usize; if rt != 0 { psxRegs.GPR.r[rt] = psxRegs.GPR.r[rs] ^ (psxRegs.code & 0xffff); } } }
pub fn psxSLTI() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; let rt = ((psxRegs.code >> 16) & 0x1f) as usize; if rt != 0 { let imm = (psxRegs.code & 0xffff) as i16 as i32; psxRegs.GPR.r[rt] = ((psxRegs.GPR.r[rs] as i32) < imm) as u32; } } }
pub fn psxSLTIU() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; let rt = ((psxRegs.code >> 16) & 0x1f) as usize; if rt != 0 { let imm = (psxRegs.code & 0xffff) as i16 as i32 as u32; psxRegs.GPR.r[rt] = (psxRegs.GPR.r[rs] < imm) as u32; } } }
pub fn psxADD() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; let rt = ((psxRegs.code >> 16) & 0x1f) as usize; let rd = ((psxRegs.code >> 11) & 0x1f) as usize; if rd != 0 { psxRegs.GPR.r[rd] = psxRegs.GPR.r[rs].wrapping_add(psxRegs.GPR.r[rt]); } } }
pub fn psxADDU() { psxADD(); }
pub fn psxSUB() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; let rt = ((psxRegs.code >> 16) & 0x1f) as usize; let rd = ((psxRegs.code >> 11) & 0x1f) as usize; if rd != 0 { psxRegs.GPR.r[rd] = psxRegs.GPR.r[rs].wrapping_sub(psxRegs.GPR.r[rt]); } } }
pub fn psxSUBU() { psxSUB(); }
pub fn psxAND() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; let rt = ((psxRegs.code >> 16) & 0x1f) as usize; let rd = ((psxRegs.code >> 11) & 0x1f) as usize; if rd != 0 { psxRegs.GPR.r[rd] = psxRegs.GPR.r[rs] & psxRegs.GPR.r[rt]; } } }
pub fn psxOR() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; let rt = ((psxRegs.code >> 16) & 0x1f) as usize; let rd = ((psxRegs.code >> 11) & 0x1f) as usize; if rd != 0 { psxRegs.GPR.r[rd] = psxRegs.GPR.r[rs] | psxRegs.GPR.r[rt]; } } }
pub fn psxXOR() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; let rt = ((psxRegs.code >> 16) & 0x1f) as usize; let rd = ((psxRegs.code >> 11) & 0x1f) as usize; if rd != 0 { psxRegs.GPR.r[rd] = psxRegs.GPR.r[rs] ^ psxRegs.GPR.r[rt]; } } }
pub fn psxNOR() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; let rt = ((psxRegs.code >> 16) & 0x1f) as usize; let rd = ((psxRegs.code >> 11) & 0x1f) as usize; if rd != 0 { psxRegs.GPR.r[rd] = !(psxRegs.GPR.r[rs] | psxRegs.GPR.r[rt]); } } }
pub fn psxSLT() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; let rt = ((psxRegs.code >> 16) & 0x1f) as usize; let rd = ((psxRegs.code >> 11) & 0x1f) as usize; if rd != 0 { psxRegs.GPR.r[rd] = ((psxRegs.GPR.r[rs] as i32) < (psxRegs.GPR.r[rt] as i32)) as u32; } } }
pub fn psxSLTU() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; let rt = ((psxRegs.code >> 16) & 0x1f) as usize; let rd = ((psxRegs.code >> 11) & 0x1f) as usize; if rd != 0 { psxRegs.GPR.r[rd] = (psxRegs.GPR.r[rs] < psxRegs.GPR.r[rt]) as u32; } } }

pub fn psxDIV() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; let rt = ((psxRegs.code >> 16) & 0x1f) as usize; let r = psxRegs.GPR.r[rs] as i32; let d = psxRegs.GPR.r[rt] as i32; if d == 0 { psxRegs.GPR.r[33] = if r < 0 { 1 } else { 0xFFFF_FFFF }; psxRegs.GPR.r[32] = psxRegs.GPR.r[rs]; } else if r == i32::MIN && d == -1 { psxRegs.GPR.r[33] = 0x8000_0000; psxRegs.GPR.r[32] = 0; } else { psxRegs.GPR.r[33] = (r / d) as u32; psxRegs.GPR.r[32] = (r % d) as u32; } } }
pub fn psxDIVU() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; let rt = ((psxRegs.code >> 16) & 0x1f) as usize; if psxRegs.GPR.r[rt] == 0 { psxRegs.GPR.r[33] = 0xFFFF_FFFF; psxRegs.GPR.r[32] = psxRegs.GPR.r[rs]; } else { psxRegs.GPR.r[33] = psxRegs.GPR.r[rs] / psxRegs.GPR.r[rt]; psxRegs.GPR.r[32] = psxRegs.GPR.r[rs] % psxRegs.GPR.r[rt]; } } }
pub fn psxMULT() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; let rt = ((psxRegs.code >> 16) & 0x1f) as usize; let r: i64 = (psxRegs.GPR.r[rs] as i32 as i64) * (psxRegs.GPR.r[rt] as i32 as i64); psxRegs.GPR.r[33] = r as u32; psxRegs.GPR.r[32] = (r >> 32) as u32; } }
pub fn psxMULTU() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; let rt = ((psxRegs.code >> 16) & 0x1f) as usize; let r: u64 = (psxRegs.GPR.r[rs] as u64) * (psxRegs.GPR.r[rt] as u64); psxRegs.GPR.r[33] = r as u32; psxRegs.GPR.r[32] = (r >> 32) as u32; } }

pub fn psxSLL() { unsafe { let rt = ((psxRegs.code >> 16) & 0x1f) as usize; let rd = ((psxRegs.code >> 11) & 0x1f) as usize; let sa = ((psxRegs.code >> 6) & 0x1f) as u32; if rd != 0 { psxRegs.GPR.r[rd] = psxRegs.GPR.r[rt] << sa; } } }
pub fn psxSRA() { unsafe { let rt = ((psxRegs.code >> 16) & 0x1f) as usize; let rd = ((psxRegs.code >> 11) & 0x1f) as usize; let sa = ((psxRegs.code >> 6) & 0x1f) as u32; if rd != 0 { psxRegs.GPR.r[rd] = ((psxRegs.GPR.r[rt] as i32) >> sa) as u32; } } }
pub fn psxSRL() { unsafe { let rt = ((psxRegs.code >> 16) & 0x1f) as usize; let rd = ((psxRegs.code >> 11) & 0x1f) as usize; let sa = ((psxRegs.code >> 6) & 0x1f) as u32; if rd != 0 { psxRegs.GPR.r[rd] = psxRegs.GPR.r[rt] >> sa; } } }
pub fn psxSLLV() { unsafe { let rt = ((psxRegs.code >> 16) & 0x1f) as usize; let rd = ((psxRegs.code >> 11) & 0x1f) as usize; let rs = ((psxRegs.code >> 21) & 0x1f) as usize; if rd != 0 { psxRegs.GPR.r[rd] = psxRegs.GPR.r[rt] << (psxRegs.GPR.r[rs] & 0x1f); } } }
pub fn psxSRAV() { unsafe { let rt = ((psxRegs.code >> 16) & 0x1f) as usize; let rd = ((psxRegs.code >> 11) & 0x1f) as usize; let rs = ((psxRegs.code >> 21) & 0x1f) as usize; if rd != 0 { psxRegs.GPR.r[rd] = ((psxRegs.GPR.r[rt] as i32) >> (psxRegs.GPR.r[rs] & 0x1f)) as u32; } } }
pub fn psxSRLV() { unsafe { let rt = ((psxRegs.code >> 16) & 0x1f) as usize; let rd = ((psxRegs.code >> 11) & 0x1f) as usize; let rs = ((psxRegs.code >> 21) & 0x1f) as usize; if rd != 0 { psxRegs.GPR.r[rd] = psxRegs.GPR.r[rt] >> (psxRegs.GPR.r[rs] & 0x1f); } } }
pub fn psxLUI() { unsafe { let rt = ((psxRegs.code >> 16) & 0x1f) as usize; if rt != 0 { psxRegs.GPR.r[rt] = psxRegs.code << 16; } } }
pub fn psxMFHI() { unsafe { let rd = ((psxRegs.code >> 11) & 0x1f) as usize; if rd != 0 { psxRegs.GPR.r[rd] = psxRegs.GPR.r[32]; } } }
pub fn psxMFLO() { unsafe { let rd = ((psxRegs.code >> 11) & 0x1f) as usize; if rd != 0 { psxRegs.GPR.r[rd] = psxRegs.GPR.r[33]; } } }
pub fn psxMTHI() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; psxRegs.GPR.r[32] = psxRegs.GPR.r[rs]; } }
pub fn psxMTLO() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; psxRegs.GPR.r[33] = psxRegs.GPR.r[rs]; } }
pub fn psxBREAK() { unsafe { psxRegs.pc = psxRegs.pc.wrapping_sub(4); psxException(0x24, iopIsDelaySlot as u32); } }
pub fn psxSYSCALL() { unsafe { psxRegs.pc = psxRegs.pc.wrapping_sub(4); psxException(0x20, iopIsDelaySlot as u32); } }
pub fn psxRFE() { unsafe { psxRegs.CP0.r[12] = (psxRegs.CP0.r[12] & 0xfffffff0) | ((psxRegs.CP0.r[12] & 0x3c) >> 2); } }

pub fn psxNULL() { /* unimplemented opcode */ }
pub fn psxSPECIAL() { unsafe { let f = (psxRegs.code & 0x3f) as usize; PSX_SPC[f](); } }
pub fn psxREGIMM() { unsafe { let r = ((psxRegs.code >> 16) & 0x1f) as usize; PSX_REG[r](); } }
pub fn psxCOP0() { unsafe { let r = ((psxRegs.code >> 21) & 0x1f) as usize; PSX_CP0[r](); } }
pub fn psxCOP2() { unsafe { let f = (psxRegs.code & 0x3f) as usize; PSX_CP2[f](); } }
pub fn psxBASIC() { unsafe { let r = ((psxRegs.code >> 21) & 0x1f) as usize; PSX_CP2BSC[r](); } }

pub fn psxMFC0() { unsafe { let rt = ((psxRegs.code >> 16) & 0x1f) as usize; let rd = ((psxRegs.code >> 11) & 0x1f) as usize; if rt != 0 { psxRegs.GPR.r[rt] = psxRegs.CP0.r[rd]; } } }
pub fn psxMTC0() { unsafe { let rt = ((psxRegs.code >> 16) & 0x1f) as usize; let rd = ((psxRegs.code >> 11) & 0x1f) as usize; psxRegs.CP0.r[rd] = psxRegs.GPR.r[rt]; } }
pub fn psxCFC0() { psxMFC0(); }
pub fn psxCTC0() { psxMTC0(); }
pub fn psxCTC2() { unsafe { let rt = ((psxRegs.code >> 16) & 0x1f) as usize; let rd = ((psxRegs.code >> 11) & 0x1f) as usize; psxRegs.CP2D.r[rd] = psxRegs.GPR.r[rt]; } }

pub fn psxLB() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; let rt = ((psxRegs.code >> 16) & 0x1f) as usize; let imm = (psxRegs.code & 0xffff) as i16 as i32 as u32; let addr = psxRegs.GPR.r[rs].wrapping_add(imm); let v = memRead8_iop(addr); if rt != 0 { psxRegs.GPR.r[rt] = v as i8 as i32 as u32; } } }
pub fn psxLBU() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; let rt = ((psxRegs.code >> 16) & 0x1f) as usize; let imm = (psxRegs.code & 0xffff) as i16 as i32 as u32; let addr = psxRegs.GPR.r[rs].wrapping_add(imm); let v = memRead8_iop(addr); if rt != 0 { psxRegs.GPR.r[rt] = v as u32; } } }
pub fn psxLH() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; let rt = ((psxRegs.code >> 16) & 0x1f) as usize; let imm = (psxRegs.code & 0xffff) as i16 as i32 as u32; let addr = psxRegs.GPR.r[rs].wrapping_add(imm); let v = memRead16_iop(addr); if rt != 0 { psxRegs.GPR.r[rt] = v as i16 as i32 as u32; } } }
pub fn psxLHU() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; let rt = ((psxRegs.code >> 16) & 0x1f) as usize; let imm = (psxRegs.code & 0xffff) as i16 as i32 as u32; let addr = psxRegs.GPR.r[rs].wrapping_add(imm); let v = memRead16_iop(addr); if rt != 0 { psxRegs.GPR.r[rt] = v as u32; } } }
pub fn psxLW() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; let rt = ((psxRegs.code >> 16) & 0x1f) as usize; let imm = (psxRegs.code & 0xffff) as i16 as i32 as u32; let addr = psxRegs.GPR.r[rs].wrapping_add(imm); let v = memRead32_iop(addr); if rt != 0 { psxRegs.GPR.r[rt] = v; } } }
pub fn psxSB() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; let rt = ((psxRegs.code >> 16) & 0x1f) as usize; let imm = (psxRegs.code & 0xffff) as i16 as i32 as u32; let addr = psxRegs.GPR.r[rs].wrapping_add(imm); memWrite8_iop(addr, psxRegs.GPR.r[rt] as u8); } }
pub fn psxSH() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; let rt = ((psxRegs.code >> 16) & 0x1f) as usize; let imm = (psxRegs.code & 0xffff) as i16 as i32 as u32; let addr = psxRegs.GPR.r[rs].wrapping_add(imm); memWrite16_iop(addr, psxRegs.GPR.r[rt] as u16); } }
pub fn psxSW() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; let rt = ((psxRegs.code >> 16) & 0x1f) as usize; let imm = (psxRegs.code & 0xffff) as i16 as i32 as u32; let addr = psxRegs.GPR.r[rs].wrapping_add(imm); memWrite32_iop(addr, psxRegs.GPR.r[rt]); } }

fn memRead8_iop(addr: u32) -> u8 { unsafe { Memory.mem[addr as usize & (MemoryState::size - 1)] } }
fn memRead16_iop(addr: u32) -> u16 { let off = addr as usize & (MemoryState::size - 1); unsafe { u16::from_le_bytes([Memory.mem[off], Memory.mem[(off + 1) & (MemoryState::size - 1)]]) } }
fn memRead32_iop(addr: u32) -> u32 { let off = addr as usize & (MemoryState::size - 1); unsafe { u32::from_le_bytes([Memory.mem[off], Memory.mem[(off + 1) & (MemoryState::size - 1)], Memory.mem[(off + 2) & (MemoryState::size - 1)], Memory.mem[(off + 3) & (MemoryState::size - 1)]]) } }
fn memWrite8_iop(addr: u32, val: u8) { unsafe { Memory.mem[addr as usize & (MemoryState::size - 1)] = val; } }
fn memWrite16_iop(addr: u32, val: u16) { let bytes = val.to_le_bytes(); unsafe { Memory.mem[addr as usize & (MemoryState::size - 1)] = bytes[0]; Memory.mem[(addr as usize + 1) & (MemoryState::size - 1)] = bytes[1]; } }
fn memWrite32_iop(addr: u32, val: u32) { let bytes = val.to_le_bytes(); let off = addr as usize & (MemoryState::size - 1); unsafe { Memory.mem[off] = bytes[0]; Memory.mem[(off + 1) & (MemoryState::size - 1)] = bytes[1]; Memory.mem[(off + 2) & (MemoryState::size - 1)] = bytes[2]; Memory.mem[(off + 3) & (MemoryState::size - 1)] = bytes[3]; } }

pub fn psxBGEZ() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; if (psxRegs.GPR.r[rs] as i32) >= 0 { let t = ((psxRegs.code & 0xffff) as i16 as i32) * 4 + psxRegs.pc as i32; psxRegs.pc = t as u32; } } }
pub fn psxBGEZAL() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; psxRegs.GPR.r[31] = psxRegs.pc.wrapping_add(4); if (psxRegs.GPR.r[rs] as i32) >= 0 { let t = ((psxRegs.code & 0xffff) as i16 as i32) * 4 + psxRegs.pc as i32; psxRegs.pc = t as u32; } } }
pub fn psxBGTZ() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; if (psxRegs.GPR.r[rs] as i32) > 0 { let t = ((psxRegs.code & 0xffff) as i16 as i32) * 4 + psxRegs.pc as i32; psxRegs.pc = t as u32; } } }
pub fn psxBLEZ() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; if (psxRegs.GPR.r[rs] as i32) <= 0 { let t = ((psxRegs.code & 0xffff) as i16 as i32) * 4 + psxRegs.pc as i32; psxRegs.pc = t as u32; } } }
pub fn psxBLTZ() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; if (psxRegs.GPR.r[rs] as i32) < 0 { let t = ((psxRegs.code & 0xffff) as i16 as i32) * 4 + psxRegs.pc as i32; psxRegs.pc = t as u32; } } }
pub fn psxBLTZAL() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; psxRegs.GPR.r[31] = psxRegs.pc.wrapping_add(4); if (psxRegs.GPR.r[rs] as i32) < 0 { let t = ((psxRegs.code & 0xffff) as i16 as i32) * 4 + psxRegs.pc as i32; psxRegs.pc = t as u32; } } }
pub fn psxBEQ() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; let rt = ((psxRegs.code >> 16) & 0x1f) as usize; if psxRegs.GPR.r[rs] == psxRegs.GPR.r[rt] { let t = ((psxRegs.code & 0xffff) as i16 as i32) * 4 + psxRegs.pc as i32; psxRegs.pc = t as u32; } } }
pub fn psxBNE() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; let rt = ((psxRegs.code >> 16) & 0x1f) as usize; if psxRegs.GPR.r[rs] != psxRegs.GPR.r[rt] { let t = ((psxRegs.code & 0xffff) as i16 as i32) * 4 + psxRegs.pc as i32; psxRegs.pc = t as u32; } } }
pub fn psxJ() { unsafe { let target = (psxRegs.code & 0x03ffffff) << 2; let pc_top = psxRegs.pc & 0xf0000000; psxRegs.pc = pc_top | target; } }
pub fn psxJAL() { unsafe { psxRegs.GPR.r[31] = psxRegs.pc.wrapping_add(4); let target = (psxRegs.code & 0x03ffffff) << 2; let pc_top = psxRegs.pc & 0xf0000000; psxRegs.pc = pc_top | target; } }
pub fn psxJR() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; psxRegs.pc = psxRegs.GPR.r[rs]; } }
pub fn psxJALR() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; let rd = ((psxRegs.code >> 11) & 0x1f) as usize; if rd != 0 { psxRegs.GPR.r[rd] = psxRegs.pc.wrapping_add(4); } psxRegs.pc = psxRegs.GPR.r[rs]; } }

// GTE / GTE opcodes referenced from the dispatch table are stubbed.
pub fn gteLWC2() {}
pub fn gteSWC2() {}
pub fn gteMFC2() {}
pub fn gteCFC2() {}
pub fn gteMTC2() {}
pub fn gteCTC2() {}
pub fn gteRTPS() {}
pub fn gteNCLIP() {}
pub fn gteOP() {}
pub fn gteDPCS() {}
pub fn gteINTPL() {}
pub fn gteMVMVA() {}
pub fn gteNCDS() {}
pub fn gteCDP() {}
pub fn gteNCDT() {}
pub fn gteNCCS() {}
pub fn gteCC() {}
pub fn gteNCS() {}
pub fn gteNCT() {}
pub fn gteSQR() {}
pub fn gteDCPL() {}
pub fn gteDPCT() {}
pub fn gteAVSZ3() {}
pub fn gteAVSZ4() {}
pub fn gteRTPT() {}
pub fn gteGPF() {}
pub fn gteGPL() {}
pub fn gteNCCT() {}

// LWL / LWR / SWL / SWR (unaligned word ops).
pub fn psxLWL() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; let rt = ((psxRegs.code >> 16) & 0x1f) as usize; let imm = (psxRegs.code & 0xffff) as i16 as i32 as u32; let addr = psxRegs.GPR.r[rs].wrapping_add(imm); let shift = (addr & 3) << 3; let m = memRead32_iop(addr & !3); if rt != 0 { psxRegs.GPR.r[rt] = (psxRegs.GPR.r[rt] & (0x00ff_ffff >> shift)) | (m << (24 - shift)); } } }
pub fn psxLWR() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; let rt = ((psxRegs.code >> 16) & 0x1f) as usize; let imm = (psxRegs.code & 0xffff) as i16 as i32 as u32; let addr = psxRegs.GPR.r[rs].wrapping_add(imm); let shift = (addr & 3) << 3; let m = memRead32_iop(addr & !3); if rt != 0 { psxRegs.GPR.r[rt] = (psxRegs.GPR.r[rt] & (0xffff_ff00 << (24 - shift))) | (m >> shift); } } }
pub fn psxSWL() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; let rt = ((psxRegs.code >> 16) & 0x1f) as usize; let imm = (psxRegs.code & 0xffff) as i16 as i32 as u32; let addr = psxRegs.GPR.r[rs].wrapping_add(imm); let shift = (addr & 3) << 3; let m = memRead32_iop(addr & !3); memWrite32_iop(addr & !3, (psxRegs.GPR.r[rt] >> (24 - shift)) | (m & (0xffff_ff00 << shift))); } }
pub fn psxSWR() { unsafe { let rs = ((psxRegs.code >> 21) & 0x1f) as usize; let rt = ((psxRegs.code >> 16) & 0x1f) as usize; let imm = (psxRegs.code & 0xffff) as i16 as i32 as u32; let addr = psxRegs.GPR.r[rs].wrapping_add(imm); let shift = (addr & 3) << 3; let m = memRead32_iop(addr & !3); memWrite32_iop(addr & !3, (psxRegs.GPR.r[rt] << shift) | (m & (0x00ff_ffff >> (24 - shift)))); } }

// ---------------------------------------------------------------------------
// PSX dispatch tables
// ---------------------------------------------------------------------------

pub static PSX_BSC: [fn(); 64] = [
    psxSPECIAL, psxREGIMM, psxJ, psxJAL, psxBEQ, psxBNE, psxBLEZ, psxBGTZ,
    psxADDI, psxADDIU, psxSLTI, psxSLTIU, psxANDI, psxORI, psxXORI, psxLUI,
    psxCOP0, psxNULL, psxCOP2, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL,
    psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL,
    psxLB, psxLH, psxLWL, psxLW, psxLBU, psxLHU, psxLWR, psxNULL,
    psxSB, psxSH, psxSWL, psxSW, psxNULL, psxNULL, psxSWR, psxNULL,
    psxNULL, psxNULL, gteLWC2, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL,
    psxNULL, psxNULL, gteSWC2, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL,
];

pub static PSX_SPC: [fn(); 64] = [
    psxSLL, psxNULL, psxSRL, psxSRA, psxSLLV, psxNULL, psxSRLV, psxSRAV,
    psxJR, psxJALR, psxNULL, psxNULL, psxSYSCALL, psxBREAK, psxNULL, psxNULL,
    psxMFHI, psxMTHI, psxMFLO, psxMTLO, psxNULL, psxNULL, psxNULL, psxNULL,
    psxMULT, psxMULTU, psxDIV, psxDIVU, psxNULL, psxNULL, psxNULL, psxNULL,
    psxADD, psxADDU, psxSUB, psxSUBU, psxAND, psxOR, psxXOR, psxNOR,
    psxNULL, psxNULL, psxSLT, psxSLTU, psxNULL, psxNULL, psxNULL, psxNULL,
    psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL,
    psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL,
];

pub static PSX_REG: [fn(); 32] = [
    psxBLTZ, psxBGEZ, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL,
    psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL,
    psxBLTZAL, psxBGEZAL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL,
    psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL,
];

pub static PSX_CP0: [fn(); 32] = [
    psxMFC0, psxNULL, psxCFC0, psxNULL, psxMTC0, psxNULL, psxCTC0, psxNULL,
    psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL,
    psxRFE, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL,
    psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL,
];

pub static PSX_CP2: [fn(); 64] = [
    psxBASIC, gteRTPS, psxNULL, psxNULL, psxNULL, psxNULL, gteNCLIP, psxNULL,
    psxNULL, psxNULL, psxNULL, psxNULL, gteOP, psxNULL, psxNULL, psxNULL,
    gteDPCS, gteINTPL, gteMVMVA, gteNCDS, gteCDP, psxNULL, gteNCDT, psxNULL,
    psxNULL, psxNULL, psxNULL, gteNCCS, gteCC, psxNULL, gteNCS, psxNULL,
    gteNCT, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL,
    gteSQR, gteDCPL, gteDPCT, psxNULL, psxNULL, gteAVSZ3, gteAVSZ4, psxNULL,
    gteRTPT, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL,
    psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, gteGPF, gteGPL, gteNCCT,
];

pub static PSX_CP2BSC: [fn(); 32] = [
    gteMFC2, psxNULL, gteCFC2, psxNULL, gteMTC2, psxNULL, psxCTC2, psxNULL,
    psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL,
    psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL,
    psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL, psxNULL,
];

// ---------------------------------------------------------------------------
// R5900 — Emotion Engine state + dispatch + interpreter
// ---------------------------------------------------------------------------

#[derive(Default, Clone, Copy)]
pub struct GprReg(pub [u64; 2]); // 128-bit
#[derive(Default, Clone, Copy)]
pub struct FprReg(pub u32);

#[derive(Default)]
pub struct GprRegs {
    pub r: [GprReg; 32],
}
#[derive(Default)]
pub struct FpuRegs {
    pub fpr: [FprReg; 32],
    pub fprc: [u32; 32],
    pub acc: FprReg,
    pub acc_flag: u32,
}

/// MIPS-style COP0 registers, kept as the same `u32` array layout that
/// the rest of the source base assumes (the Status bitfield lives at
/// index 12, Cause at 13, EPC at 14, etc.).
#[derive(Default)]
pub struct Cp0Regs {
    pub r: [u32; 32],
}

/// Performance counter / event configuration block (R5900 has two PCRs).
#[derive(Default)]
pub struct PerfRegs {
    pub r: [u32; 4],
}

#[derive(Default)]
pub struct R5900State {
    pub GPR: GprRegs,
    pub HI: GprReg,
    pub LO: GprReg,
    pub CP0: Cp0Regs,
    pub sa: u32,
    pub is_delay_slot: u32,
    pub pc: u32,
    pub code: u32,
    pub PERF: PerfRegs,
    pub eCycle: [u32; 32],
    pub sCycle: [u64; 32],
    pub cycle: u64,
    pub interrupt: u32,
    pub branch: s32,
    pub opmode: s32,
    pub tempcycles: u32,
    pub dmastall: u32,
    pub pcWriteback: u32,
    pub nextEventCycle: u64,
    pub lastEventCycle: u64,
    pub lastCOP0Cycle: u64,
    pub lastPERFCycle: [u64; 2],
    pub FPU: FpuRegs,
}

pub static mut cpuRegs: R5900State = R5900State {
    GPR: GprRegs { r: [GprReg([0, 0]); 32] },
    HI: GprReg([0, 0]),
    LO: GprReg([0, 0]),
    CP0: Cp0Regs { r: [0; 32] },
    sa: 0,
    is_delay_slot: 0,
    pc: 0,
    code: 0,
    PERF: PerfRegs { r: [0; 4] },
    eCycle: [0; 32],
    sCycle: [0; 32],
    cycle: 0,
    interrupt: 0,
    branch: 0,
    opmode: 0,
    tempcycles: 0,
    dmastall: 0,
    pcWriteback: 0,
    nextEventCycle: 0,
    lastEventCycle: 0,
    lastCOP0Cycle: 0,
    lastPERFCycle: [0, 0],
    FPU: FpuRegs { fpr: [FprReg(0); 32], fprc: [0; 32], acc: FprReg(0), acc_flag: 0 },
};

pub static mut EEsCycle: s32 = 0;
pub static mut EEoCycle: u64 = 0;
pub static mut eeEventTestIsActive: bool = false;
pub static mut eeWaitCycles: u32 = 3072;

/// Generic R5900 CPU interface (reserve / reset / step / execute …).
pub struct R5900cpu {
    pub reserve: Option<fn()>,
    pub shutdown: Option<fn()>,
    pub reset: Option<fn()>,
    pub step: Option<fn()>,
    pub execute: Option<fn()>,
    pub exit_execution: Option<fn()>,
    pub cancel_instruction: Option<fn()>,
    pub clear: Option<fn(u32, u32)>,
}

pub static mut Cpu: Option<&'static mut R5900cpu> = None;
pub static mut intCpu: R5900cpu = R5900cpu {
    reserve: None,
    shutdown: None,
    reset: None,
    step: None,
    execute: None,
    exit_execution: None,
    cancel_instruction: None,
    clear: None,
};
pub static mut recCpu: R5900cpu = R5900cpu {
    reserve: None,
    shutdown: None,
    reset: None,
    step: None,
    execute: None,
    exit_execution: None,
    cancel_instruction: None,
    clear: None,
};

/// Reset the EE to its post-BIOS state.  Mirrors `cpuReset()` from
/// R5900.cpp: PC = 0xbfc00000, BEV=1, PRid set to 0x2e20.
pub fn eeReset() {
    unsafe {
        *(&mut cpuRegs) = R5900State::default();
        cpuRegs.pc = 0xbfc00000;
        cpuRegs.CP0.r[16] = 0x440;       // Config
        cpuRegs.CP0.r[12] = 0x70400004;  // Status: BEV=1, TS=1, COP0 enabled
        cpuRegs.CP0.r[15] = 0x00002e20;  // PRid
        cpuRegs.FPU.fprc[0] = 0x00002e30;
        cpuRegs.FPU.fprc[31] = 0x01000001;
        cpuRegs.nextEventCycle = cpuRegs.cycle + 4;
        EEsCycle = 0;
        EEoCycle = cpuRegs.cycle;
        psxReset();
    }
}

pub fn eeInit() {
    eeReset();
}

/// One-shot EE execute block.  Mirrors the `intExecute` outer loop
/// shape (run a single `execI` per call).  A full interpreter reads
/// `cpuRegs.code`, decodes via `EE_Standard`, and dispatches.
pub fn eeExecuteBlock() {
    unsafe {
        // The real interpreter would loop `while (true) execI();` —
        // we expose the same entry point and let the caller drive it.
        let opcode_idx = ((cpuRegs.code >> 26) & 0x3f) as usize;
        EE_STANDARD[opcode_idx]();
    }
}

pub fn cpuException(code: u32, bd: u32) {
    unsafe {
        cpuRegs.branch = 0;
        cpuRegs.CP0.r[13] = code & 0xffff;
        if bd != 0 {
            cpuRegs.CP0.r[13] |= 0x80000000;
            cpuRegs.CP0.r[14] = cpuRegs.pc.wrapping_sub(4);
        } else {
            cpuRegs.CP0.r[14] = cpuRegs.pc;
            cpuRegs.CP0.r[13] &= !0x80000000;
        }
        let erl = (cpuRegs.CP0.r[12] >> 2) & 1;
        let bev = (cpuRegs.CP0.r[12] >> 22) & 1;
        if erl == 0 {
            cpuRegs.pc = if bev == 0 { 0x80000000 } else { 0xBFC00200 };
        } else {
            cpuRegs.pc = 0xBFC00000;
        }
    }
}

pub fn cpuSetNextEvent(_start: u64, _delta: s32) {}
pub fn cpuSetNextEventDelta(_delta: s32) {}
pub fn cpuTestCycle(_start: u64, _delta: s32) -> i32 { 0 }
pub fn cpuSetEvent() {}
pub fn cpuClearInt(_n: u32) { unsafe { cpuRegs.interrupt &= !(1 << _n); cpuRegs.dmastall &= !(1 << _n); } }
pub fn cpuTestHwInts() {}
pub fn cpuTlbMissR(_addr: u32, _bd: u32) {}
pub fn cpuTlbMissW(_addr: u32, _bd: u32) {}
pub fn intUpdateCPUCycles() {}
pub fn intEventTest() {}
pub fn intSetBranch() { unsafe { cpuRegs.branch = 1; } }
pub fn intDoBranch(_target: u32) { unsafe { cpuRegs.pc = _target; } }

// ---------------------------------------------------------------------------
// EE instruction dispatch tables (R5900OpcodeTables)
// ---------------------------------------------------------------------------

pub fn ee_unknown() {}
pub fn ee_special() { unsafe { let f = (cpuRegs.code & 0x3f) as usize; EE_SPECIAL[f](); } }
pub fn ee_regimm() { unsafe { let r = ((cpuRegs.code >> 16) & 0x1f) as usize; EE_REGIMM[r](); } }
pub fn ee_cop0() { unsafe { let r = ((cpuRegs.code >> 21) & 0x1f) as usize; EE_COP0[r](); } }
pub fn ee_cop1() { unsafe { let r = ((cpuRegs.code >> 21) & 0x1f) as usize; EE_COP1[r](); } }
pub fn ee_cop2() {}
pub fn ee_mmi() { unsafe { let f = (cpuRegs.code & 0x3f) as usize; EE_MMI[f](); } }
pub fn ee_mmi0() { unsafe { let f = ((cpuRegs.code >> 6) & 0x1f) as usize; EE_MMI0[f](); } }
pub fn ee_mmi1() { unsafe { let f = ((cpuRegs.code >> 6) & 0x1f) as usize; EE_MMI1[f](); } }
pub fn ee_mmi2() { unsafe { let f = ((cpuRegs.code >> 6) & 0x1f) as usize; EE_MMI2[f](); } }
pub fn ee_mmi3() { unsafe { let f = ((cpuRegs.code >> 6) & 0x1f) as usize; EE_MMI3[f](); } }
pub fn ee_beql() {}
pub fn ee_bnel() {}
pub fn ee_blezl() {}
pub fn ee_bgtzl() {}
pub fn ee_addi() {}
pub fn ee_addiu() {}
pub fn ee_slti() {}
pub fn ee_sltiu() {}
pub fn ee_andi() {}
pub fn ee_ori() {}
pub fn ee_xori() {}
pub fn ee_lui() {}
pub fn ee_daddi() {}
pub fn ee_daddiu() {}
pub fn ee_ldl() {}
pub fn ee_ldr() {}
pub fn ee_lq() {}
pub fn ee_sq() {}
pub fn ee_lb() {}
pub fn ee_lh() {}
pub fn ee_lwl() {}
pub fn ee_lw() {}
pub fn ee_lbu() {}
pub fn ee_lhu() {}
pub fn ee_lwr() {}
pub fn ee_lwu() {}
pub fn ee_sb() {}
pub fn ee_sh() {}
pub fn ee_swl() {}
pub fn ee_sw() {}
pub fn ee_sdl() {}
pub fn ee_sdr() {}
pub fn ee_swr() {}
pub fn ee_ld() {}
pub fn ee_sd() {}
pub fn ee_lwc1() {}
pub fn ee_swc1() {}
pub fn ee_lqc2() {}
pub fn ee_sqc2() {}
pub fn ee_cache() {}
pub fn ee_pref() {}
pub fn ee_break() {}
pub fn ee_sync() {}
pub fn ee_syscall() {}
pub fn ee_teqi() {}
pub fn ee_tgei() {}
pub fn ee_tgeiu() {}
pub fn ee_tlti() {}
pub fn ee_tltiu() {}
pub fn ee_tnei() {}

pub fn ee_mfc0() {}
pub fn ee_mtc0() {}
pub fn ee_bc0f() {}
pub fn ee_bc0t() {}
pub fn ee_bc0fl() {}
pub fn ee_bc0tl() {}
pub fn ee_tlbr() {}
pub fn ee_tlbwi() {}
pub fn ee_tlbwr() {}
pub fn ee_tlbp() {}
pub fn ee_eret() {}
pub fn ee_di() {}
pub fn ee_ei() {}

pub fn ee_mfc1() {}
pub fn ee_mtc1() {}
pub fn ee_cfc1() {}
pub fn ee_ctc1() {}
pub fn ee_bc1f() {}
pub fn ee_bc1t() {}
pub fn ee_bc1fl() {}
pub fn ee_bc1tl() {}

pub static EE_STANDARD: [fn(); 64] = [
    ee_special, ee_regimm, ee_unknown /*J*/, ee_unknown /*JAL*/,
    ee_unknown /*BEQ*/, ee_unknown /*BNE*/, ee_unknown /*BLEZ*/, ee_unknown /*BGTZ*/,
    ee_addi, ee_addiu, ee_slti, ee_sltiu, ee_andi, ee_ori, ee_xori, ee_lui,
    ee_cop0, ee_cop1, ee_cop2, ee_unknown,
    ee_beql, ee_bnel, ee_blezl, ee_bgtzl,
    ee_daddi, ee_daddiu, ee_ldl, ee_ldr,
    ee_mmi, ee_unknown, ee_lq, ee_sq,
    ee_lb, ee_lh, ee_lwl, ee_lw,
    ee_lbu, ee_lhu, ee_lwr, ee_lwu,
    ee_sb, ee_sh, ee_swl, ee_sw,
    ee_sdl, ee_sdr, ee_swr, ee_cache,
    ee_unknown, ee_lwc1, ee_unknown, ee_pref,
    ee_unknown, ee_unknown, ee_lqc2, ee_ld,
    ee_unknown, ee_swc1, ee_unknown, ee_unknown,
    ee_unknown, ee_unknown, ee_sqc2, ee_sd,
];

pub static EE_SPECIAL: [fn(); 64] = [
    ee_unknown /*SLL*/, ee_unknown, ee_unknown /*SRL*/, ee_unknown /*SRA*/,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown,
    ee_unknown /*JR*/, ee_unknown /*JALR*/, ee_unknown, ee_unknown,
    ee_syscall, ee_break, ee_unknown, ee_sync,
    ee_unknown /*MFHI*/, ee_unknown /*MTHI*/, ee_unknown /*MFLO*/, ee_unknown /*MTLO*/,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown,
    ee_unknown /*MULT*/, ee_unknown /*MULTU*/, ee_unknown /*DIV*/, ee_unknown /*DIVU*/,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown,
    ee_unknown /*ADD*/, ee_unknown /*ADDU*/, ee_unknown /*SUB*/, ee_unknown /*SUBU*/,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown,
];

pub static EE_REGIMM: [fn(); 32] = [
    ee_unknown /*BLTZ*/, ee_unknown /*BGEZ*/, ee_unknown /*BLTZL*/, ee_unknown /*BGEZL*/,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown,
    ee_tgei, ee_tgeiu, ee_tlti, ee_tltiu, ee_teqi, ee_unknown, ee_tnei, ee_unknown,
    ee_unknown /*BLTZAL*/, ee_unknown /*BGEZAL*/, ee_unknown, ee_unknown,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown,
];

pub static EE_COP0: [fn(); 32] = [
    ee_mfc0, ee_unknown, ee_unknown, ee_unknown,
    ee_mtc0, ee_unknown, ee_unknown, ee_unknown,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown,
    ee_cop0 /*=tlbr..eret etc.*/, ee_unknown, ee_unknown, ee_unknown,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown,
];

pub static EE_COP1: [fn(); 32] = [
    ee_mfc1, ee_unknown, ee_cfc1, ee_unknown,
    ee_mtc1, ee_unknown, ee_ctc1, ee_unknown,
    ee_bc1f, ee_unknown, ee_unknown, ee_unknown,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown,
    ee_cop1 /*=S*/, ee_unknown, ee_unknown, ee_unknown,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown,
];

pub static EE_MMI: [fn(); 64] = [
    ee_unknown /*MADD*/, ee_unknown /*MADDU*/, ee_unknown, ee_unknown,
    ee_unknown /*PLZCW*/, ee_unknown, ee_unknown, ee_unknown,
    ee_mmi0, ee_mmi2, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown,
    ee_unknown /*MULT1*/, ee_unknown /*MULTU1*/, ee_unknown /*DIV1*/, ee_unknown /*DIVU1*/,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown,
    ee_unknown /*MADD1*/, ee_unknown /*MADDU1*/, ee_unknown, ee_unknown,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown,
    ee_mmi1, ee_mmi3, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown,
];

pub static EE_MMI0: [fn(); 32] = [
    ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown,
];

pub static EE_MMI1: [fn(); 32] = [
    ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown,
];

pub static EE_MMI2: [fn(); 32] = [
    ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown,
];

pub static EE_MMI3: [fn(); 32] = [
    ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown,
    ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown, ee_unknown,
];

// ---------------------------------------------------------------------------
// Config — Pcsx2Config with the full set of options.
// ---------------------------------------------------------------------------

#[derive(Default, Clone)]
pub struct SpeedhackOptions {
    pub eecycle_rate: i8,
    pub eecycle_skip: u8,
    pub bits: u32, // fastCDVD | IntcStat | WaitLoop | vuFlagHack | vuThread | vu1Instant
}

#[derive(Default, Clone)]
pub struct GamefixOptions {
    pub bits: u32, // bitfield of GamefixId flags
}

#[derive(Default, Clone)]
pub struct EmulationSpeedOptions {
    pub sync_to_host_refresh_rate: bool,
    pub use_vsync_for_timing: bool,
    pub nominal_scalar: f32,
    pub turbo_scalar: f32,
    pub slomo_scalar: f32,
}

#[derive(Default, Clone)]
pub struct FilenameOptions {
    pub bios: String,
}

#[derive(Default, Clone)]
pub struct SavestateOptions {
    pub compression_type: u8,
    pub compression_ratio: u8,
}

#[derive(Default, Clone)]
pub struct SPU2Options {
    pub standard_volume: u32,
    pub fast_forward_volume: u32,
    pub output_muted: bool,
    pub backend: u32,
    pub sync_mode: u8,
    pub driver_name: String,
    pub device_name: String,
}

#[derive(Default, Clone)]
pub struct DEV9Options {
    pub eth_enable: bool,
    pub hdd_enable: bool,
    pub eth_device: String,
    pub hdd_file: String,
    pub intercept_dhcp: bool,
}

#[derive(Default, Clone)]
pub struct UsbOptions {
    pub ports: [i32; 2],
    pub port_subtype: [u32; 2],
}

#[derive(Default, Clone)]
pub struct PadOptions {
    pub port_type: [u8; 8],
    pub multitap_port0: bool,
    pub multitap_port1: bool,
}

#[derive(Default, Clone)]
pub struct McdOptions {
    pub filename: String,
    pub enabled: bool,
    pub ty: u8, // MemoryCardType
}

#[derive(Default, Clone)]
pub struct GsOptions {
    pub bitsets: [u64; 2],
    pub vsync_queue_size: i32,
    pub framerate_ntsc: f32,
    pub framerate_pal: f32,
    pub aspect_ratio: u8,
    pub fmv_aspect_ratio: u8,
    pub interlace_mode: u8,
    pub linear_present: u8,
    pub stretch_y: f32,
    pub crop: [i32; 4],
    pub osd_scale: f32,
    pub osd_margin: f32,
    pub osd_font_path: String,
    pub osd_messages_pos: u8,
    pub osd_performance_pos: u8,
    pub renderer: i8,
    pub upscale_multiplier: f32,
    pub accurate_blending_unit: u8,
    pub texture_filtering: u8,
    pub texture_preloading: u8,
    pub dump_compression: u8,
    pub hw_download_mode: u8,
    pub cas_mode: u8,
    pub dithering: u8,
    pub max_anisotropy: u8,
    pub tv_shader: u8,
    pub adapter: String,
    pub hw_dump_directory: String,
    pub sw_dump_directory: String,
}

#[derive(Default, Clone)]
pub struct RecompilerOptions {
    pub bits: u32, // EE/IOP/VU enables + FPU clamps + cache + fastmem
    pub ee_clamp_mode: u32,
    pub vu_clamp_mode: u32,
}

#[derive(Default, Clone)]
pub struct CpuOptions {
    pub recompiler: RecompilerOptions,
    pub extra_memory: bool,
}

#[derive(Default, Clone)]
pub struct ProfilerOptions {
    pub bits: u32, // Enabled + RecBlocks_EE/IOP/VU0/VU1
}

#[derive(Default, Clone)]
pub struct DebugSymbolSource {
    pub name: String,
    pub clear_during_analysis: bool,
}

#[derive(Default, Clone)]
pub struct DebugExtraSymbolFile {
    pub path: String,
    pub base_address: String,
    pub condition: String,
}

#[derive(Default, Clone)]
pub struct DebugAnalysisOptions {
    pub run_condition: u8,
    pub generate_symbols_for_irx_exports: bool,
    pub automatically_select_symbols_to_clear: bool,
    pub symbol_sources: Vec<DebugSymbolSource>,
    pub import_symbols_from_elf: bool,
    pub import_sym_file_from_default_location: bool,
    pub demangle_symbols: bool,
    pub demangle_parameters: bool,
    pub extra_symbol_files: Vec<DebugExtraSymbolFile>,
    pub function_scan_mode: u8,
    pub custom_function_scan_range: bool,
    pub function_scan_start_address: String,
    pub function_scan_end_address: String,
    pub generate_function_hashes: bool,
}

#[derive(Default, Clone)]
pub struct TraceLogsEE {
    pub bits: u32,
}
#[derive(Default, Clone)]
pub struct TraceLogsIOP {
    pub bits: u32,
}
#[derive(Default, Clone)]
pub struct TraceLogsMISC {
    pub bits: u32,
}

#[derive(Default, Clone)]
pub struct TraceLogFilters {
    pub enabled: bool,
    pub ee: TraceLogsEE,
    pub iop: TraceLogsIOP,
    pub misc: TraceLogsMISC,
}

#[derive(Default, Clone)]
pub struct AchievementsOptions {
    pub bits: u32,
    pub notifications_duration: u32,
    pub leaderboards_duration: u32,
    pub overlay_position: u8,
    pub notification_position: u8,
    pub info_sound_name: String,
    pub unlock_sound_name: String,
    pub lb_submit_sound_name: String,
}

#[derive(Default, Clone)]
pub struct Pcsx2Config {
    pub cpu: CpuOptions,
    pub gs: GsOptions,
    pub speedhacks: SpeedhackOptions,
    pub gamefixes: GamefixOptions,
    pub profiler: ProfilerOptions,
    pub debugger_analysis: DebugAnalysisOptions,
    pub emulation_speed: EmulationSpeedOptions,
    pub savestate: SavestateOptions,
    pub spu2: SPU2Options,
    pub dev9: DEV9Options,
    pub usb: UsbOptions,
    pub pad: PadOptions,
    pub trace: TraceLogFilters,
    pub base_filenames: FilenameOptions,
    pub achievements: AchievementsOptions,
    pub mcd: [McdOptions; 8],
    pub gzip_iso_index_template: String,
    pub pine_slot: i32,
    pub rtc_year: i32,
    pub rtc_month: i32,
    pub rtc_day: i32,
    pub rtc_hour: i32,
    pub rtc_minute: i32,
    pub rtc_second: i32,
    pub current_blockdump: String,
    pub current_irx: String,
    pub current_game_args: String,
    pub custom_data_path: String,
    pub current_aspect_ratio: u8,
    pub current_custom_aspect_ratio: f32,
    pub is_portable_mode: bool,
    pub top_level_bits: u32, // CdvdVerboseReads, EnablePatches, EnableCheats, EnableWideScreenPatches …
}

pub static mut EmuConfig: Pcsx2Config = Pcsx2Config {
    cpu: CpuOptions { recompiler: RecompilerOptions { bits: 0, ee_clamp_mode: 0, vu_clamp_mode: 0 }, extra_memory: false },
    gs: GsOptions {
        bitsets: [0, 0],
        vsync_queue_size: 2,
        framerate_ntsc: 59.94,
        framerate_pal: 50.0,
        aspect_ratio: 1, // RAuto4_3_3_2
        fmv_aspect_ratio: 0,
        interlace_mode: 0, // Automatic
        linear_present: 1, // BilinearSmooth
        stretch_y: 100.0,
        crop: [0; 4],
        osd_scale: 100.0,
        osd_margin: 10.0,
        osd_font_path: String::new(),
        osd_messages_pos: 1,  // TopLeft
        osd_performance_pos: 3, // TopRight
        renderer: -1,         // Auto
        upscale_multiplier: 1.0,
        accurate_blending_unit: 1, // Basic
        texture_filtering: 2,       // PS2
        texture_preloading: 2,      // Full
        dump_compression: 2,        // Zstandard
        hw_download_mode: 0,        // Enabled
        cas_mode: 0,                // Disabled
        dithering: 2,
        max_anisotropy: 0,
        tv_shader: 0,
        adapter: String::new(),
        hw_dump_directory: String::new(),
        sw_dump_directory: String::new(),
    },
    speedhacks: SpeedhackOptions { eecycle_rate: 0, eecycle_skip: 0, bits: 0 },
    gamefixes: GamefixOptions { bits: 0 },
    profiler: ProfilerOptions { bits: 0 },
    debugger_analysis: DebugAnalysisOptions {
        run_condition: 1, // IF_DEBUGGER_IS_OPEN
        generate_symbols_for_irx_exports: true,
        automatically_select_symbols_to_clear: true,
        symbol_sources: Vec::new(),
        import_symbols_from_elf: true,
        import_sym_file_from_default_location: true,
        demangle_symbols: true,
        demangle_parameters: true,
        extra_symbol_files: Vec::new(),
        function_scan_mode: 0, // SCAN_ELF
        custom_function_scan_range: false,
        function_scan_start_address: String::new(),
        function_scan_end_address: String::new(),
        generate_function_hashes: true,
    },
    emulation_speed: EmulationSpeedOptions {
        sync_to_host_refresh_rate: false,
        use_vsync_for_timing: false,
        nominal_scalar: 1.0,
        turbo_scalar: 2.0,
        slomo_scalar: 0.5,
    },
    savestate: SavestateOptions { compression_type: 2, compression_ratio: 1 }, // Zstandard / Medium
    spu2: SPU2Options { standard_volume: 100, fast_forward_volume: 100, output_muted: false, backend: 0, sync_mode: 1, driver_name: String::new(), device_name: String::new() },
    dev9: DEV9Options { eth_enable: false, hdd_enable: false, eth_device: String::new(), hdd_file: String::new(), intercept_dhcp: false },
    usb: UsbOptions { ports: [0; 2], port_subtype: [0; 2] },
    pad: PadOptions { port_type: [0; 8], multitap_port0: false, multitap_port1: false },
    trace: TraceLogFilters { enabled: false, ee: TraceLogsEE { bits: 0 }, iop: TraceLogsIOP { bits: 0 }, misc: TraceLogsMISC { bits: 0 } },
    base_filenames: FilenameOptions { bios: String::new() },
    achievements: AchievementsOptions {
        bits: 0,
        notifications_duration: 5,
        leaderboards_duration: 10,
        overlay_position: 8, // BottomRight
        notification_position: 1,
        info_sound_name: String::new(),
        unlock_sound_name: String::new(),
        lb_submit_sound_name: String::new(),
    },
    mcd: [
        McdOptions { filename: String::new(), enabled: false, ty: 0 },
        McdOptions { filename: String::new(), enabled: false, ty: 0 },
        McdOptions { filename: String::new(), enabled: false, ty: 0 },
        McdOptions { filename: String::new(), enabled: false, ty: 0 },
        McdOptions { filename: String::new(), enabled: false, ty: 0 },
        McdOptions { filename: String::new(), enabled: false, ty: 0 },
        McdOptions { filename: String::new(), enabled: false, ty: 0 },
        McdOptions { filename: String::new(), enabled: false, ty: 0 },
    ],
    gzip_iso_index_template: String::new(),
    pine_slot: 0,
    rtc_year: 0,
    rtc_month: 0,
    rtc_day: 0,
    rtc_hour: 0,
    rtc_minute: 0,
    rtc_second: 0,
    current_blockdump: String::new(),
    current_irx: String::new(),
    current_game_args: String::new(),
    custom_data_path: String::new(),
    current_aspect_ratio: 1,
    current_custom_aspect_ratio: 0.0,
    is_portable_mode: false,
    top_level_bits: 0,
};

// ---------------------------------------------------------------------------
// GameList — minimal port of the game list + directory scan.
// ---------------------------------------------------------------------------

#[derive(Default, Clone)]
pub struct GameEntry {
    pub path: String,
    pub serial: String,
    pub title: String,
    pub title_en: String,
    pub region: u8, // Region enum
    pub total_size: u64,
    pub last_modified: u64,
    pub last_played: u64,
    pub total_played: u64,
    pub crc: u32,
    pub entry_type: u8, // EntryType
    pub compatibility: u8,
}

pub struct GameList {
    pub entries: Vec<GameEntry>,
}

impl GameList {
    pub fn new() -> Self { Self { entries: Vec::new() } }

    /// Append a single game-list entry.
    pub fn add_entry(&mut self, entry: GameEntry) { self.entries.push(entry); }

    /// Scan a directory and add all `.iso` / `.bin` / `.elf` files as
    /// stub entries.  Mirrors `ScanDirectory` in GameList.cpp.
    pub fn scan_dir(&mut self, dir: &str) {
        let entries = match std::fs::read_dir(dir) {
            Ok(it) => it,
            Err(_) => return,
        };
        for ent in entries.flatten() {
            let p = ent.path();
            if p.is_file() {
                if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
                    let ext = ext.to_ascii_lowercase();
                    if matches!(ext.as_str(), "iso" | "bin" | "elf" | "cso" | "chd") {
                        let meta = p.metadata().ok();
                        let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                        let mtime = meta
                            .and_then(|m| m.modified().ok())
                            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        let entry = GameEntry {
                            path: p.to_string_lossy().to_string(),
                            serial: String::new(),
                            title: p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
                            title_en: String::new(),
                            region: 0,
                            total_size: size,
                            last_modified: mtime,
                            last_played: 0,
                            total_played: 0,
                            crc: 0,
                            entry_type: if ext == "elf" { 2 } else { 0 },
                            compatibility: 0,
                        };
                        self.add_entry(entry);
                    }
                }
            }
        }
    }

    /// Re-scan a single entry.  In the C++ source this re-parses ISO
    /// headers; here we just refresh the metadata.
    pub fn refresh(&mut self, idx: usize) {
        if let Some(entry) = self.entries.get_mut(idx) {
            if let Ok(meta) = std::fs::metadata(&entry.path) {
                entry.total_size = meta.len();
                if let Ok(t) = meta.modified() {
                    if let Ok(d) = t.duration_since(UNIX_EPOCH) {
                        entry.last_modified = d.as_secs();
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// GameDatabase — parsed YAML/INI compatibility database stub.
// ---------------------------------------------------------------------------

#[derive(Default, Clone)]
pub struct GameDatabaseEntry {
    pub serial: String,
    pub name: String,
    pub region: String,
    pub compat: u8,
    pub ee_clamp_mode: i8,
    pub vu0_clamp_mode: i8,
    pub vu1_clamp_mode: i8,
    pub game_fixes: Vec<u32>,
    pub speed_hacks: Vec<(u32, i32)>,
    pub memcard_filters: Vec<String>,
    pub patches: Vec<(u32, String)>,
}

pub struct GameDatabase {
    pub entries: Vec<GameDatabaseEntry>,
    pub loaded: bool,
}

impl GameDatabase {
    pub fn new() -> Self { Self { entries: Vec::new(), loaded: false } }

    /// Walk the on-disk GameIndex.yaml and pull per-serial entries.
    /// For the rewrite we treat the parse as a line-based scan: any
    /// line beginning with `serial: ` becomes an entry.
    pub fn parse(&mut self, path: &str) {
        self.entries.clear();
        if let Ok(data) = std::fs::read_to_string(path) {
            for line in data.lines() {
                let trimmed = line.trim();
                if let Some(rest) = trimmed.strip_prefix("serial:") {
                    let serial = rest.trim().trim_matches('"').to_string();
                    self.entries.push(GameDatabaseEntry { serial, ..Default::default() });
                }
            }
        }
        self.loaded = true;
    }

    pub fn find(&self, serial: &str) -> Option<&GameDatabaseEntry> {
        self.entries.iter().find(|e| e.serial == serial)
    }
}

// ---------------------------------------------------------------------------
// Patch — pnach-style cheat / patch engine.
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct PatchCommand {
    pub place: u8,  // patch_place_type
    pub cpu: u8,    // patch_cpu_type
    pub ty: u8,     // patch_data_type
    pub addr: u32,
    pub data: u64,
}

pub struct PatchEngine {
    pub patches: Vec<PatchCommand>,
    pub applied_count: u32,
}

impl PatchEngine {
    pub fn new() -> Self { Self { patches: Vec::new(), applied_count: 0 } }

    /// Append a patch line.
    pub fn add(&mut self, cmd: PatchCommand) { self.patches.push(cmd); }

    /// Apply a single 32-bit value to the EE memory buffer.
    pub fn apply(&mut self, addr: u32, value: u32) {
        // In a full port this writes through EE memory; here we keep
        // a side counter so the embedder can verify ordering.
        self.applied_count += 1;
        let cmd = PatchCommand { place: 0, cpu: 0, ty: 2, addr, data: value as u64 };
        self.patches.push(cmd);
    }
}

// ---------------------------------------------------------------------------
// Memory — flat memory buffer + typed read/write API.
// ---------------------------------------------------------------------------

pub struct MemoryState {
    /// 4 MiB of IOP-visible RAM.  Sized to a power of two so mask-based
    /// address folding is well-defined (matching the C++ `& size - 1`
    /// idiom in the PSX load/store helpers).
    pub mem: [u8; 4 * 1024 * 1024],
}

impl MemoryState {
    pub const fn new() -> Self { Self { mem: [0; 4 * 1024 * 1024] } }
    pub const size: usize = 4 * 1024 * 1024;

    pub fn memInit(&mut self) { self.mem.fill(0); }
    pub fn memReset(&mut self) { self.memInit(); }

    pub fn memRead8(&self, addr: u32) -> u8 { self.mem[addr as usize & (Self::size - 1)] }
    pub fn memRead16(&self, addr: u32) -> u16 {
        let off = addr as usize & (Self::size - 1);
        u16::from_le_bytes([self.mem[off], self.mem[(off + 1) & (Self::size - 1)]])
    }
    pub fn memRead32(&self, addr: u32) -> u32 {
        let off = addr as usize & (Self::size - 1);
        u32::from_le_bytes([
            self.mem[off],
            self.mem[(off + 1) & (Self::size - 1)],
            self.mem[(off + 2) & (Self::size - 1)],
            self.mem[(off + 3) & (Self::size - 1)],
        ])
    }
    pub fn memRead64(&self, addr: u32) -> u64 {
        let lo = self.memRead32(addr) as u64;
        let hi = self.memRead32(addr.wrapping_add(4)) as u64;
        lo | (hi << 32)
    }
    pub fn memRead128(&self, addr: u32) -> u128 {
        [self.memRead64(addr), self.memRead64(addr.wrapping_add(8))]
    }

    pub fn memWrite8(&mut self, addr: u32, val: u8) {
        self.mem[addr as usize & (Self::size - 1)] = val;
    }
    pub fn memWrite16(&mut self, addr: u32, val: u16) {
        let bytes = val.to_le_bytes();
        let off = addr as usize & (Self::size - 1);
        self.mem[off] = bytes[0];
        self.mem[(off + 1) & (Self::size - 1)] = bytes[1];
    }
    pub fn memWrite32(&mut self, addr: u32, val: u32) {
        let bytes = val.to_le_bytes();
        let off = addr as usize & (Self::size - 1);
        self.mem[off] = bytes[0];
        self.mem[(off + 1) & (Self::size - 1)] = bytes[1];
        self.mem[(off + 2) & (Self::size - 1)] = bytes[2];
        self.mem[(off + 3) & (Self::size - 1)] = bytes[3];
    }
    pub fn memWrite64(&mut self, addr: u32, val: u64) {
        self.memWrite32(addr, val as u32);
        self.memWrite32(addr.wrapping_add(4), (val >> 32) as u32);
    }
    pub fn memWrite128(&mut self, addr: u32, val: u128) {
        self.memWrite64(addr, val[0]);
        self.memWrite64(addr.wrapping_add(8), val[1]);
    }
}

pub static mut Memory: MemoryState = MemoryState { mem: [0; 4 * 1024 * 1024] };

// ---------------------------------------------------------------------------
// MTGS — graphics thread manager stub.
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct MtgsState {
    pub thread_handle: usize,
    pub ring_size: u32,
    pub ring_mask: u32,
    pub open: bool,
    pub queue_count: i32,
}

impl MtgsState {
    pub const fn new() -> Self {
        Self {
            thread_handle: 0,
            ring_size: 1 << 19, // 8 MiB ring buffer
            ring_mask: (1 << 19) - 1,
            open: false,
            queue_count: 0,
        }
    }
    pub fn start_thread(&mut self) { self.open = true; }
    pub fn shutdown_thread(&mut self) { self.open = false; }
    pub fn post_vsync(&mut self) { self.queue_count += 1; }
    pub fn run_on_gs<F: FnOnce()>(&self, _f: F) { /* would queue onto MTGS thread */ }
    pub fn is_open(&self) -> bool { self.open }
}

pub static mut Mtgs: MtgsState = MtgsState::new();

// ---------------------------------------------------------------------------
// MTVU — micro VU thread manager stub.
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct MtvuState {
    pub thread_handle: usize,
    pub open: bool,
    pub buffer_size: u32,
    pub read_pos: i32,
    pub write_pos: i32,
    pub shutdown_flag: bool,
    pub interrupts: u32,
    pub gs_label: u64,
    pub gs_signal: u64,
}

impl MtvuState {
    pub const fn new() -> Self {
        Self {
            thread_handle: 0,
            open: false,
            buffer_size: 16 * 1024 * 1024 / 4, // 16 MiB in u32 units
            read_pos: 0,
            write_pos: 0,
            shutdown_flag: false,
            interrupts: 0,
            gs_label: 0,
            gs_signal: 0,
        }
    }
    pub fn open(&mut self) { self.open = true; }
    pub fn close(&mut self) { self.open = false; self.shutdown_flag = true; }
    pub fn reset(&mut self) { self.read_pos = 0; self.write_pos = 0; self.interrupts = 0; }
    pub fn kick_start(&mut self) {}
    pub fn is_done(&self) -> bool { self.read_pos == self.write_pos }
    pub fn wait_vu(&self) {}
    pub fn get_changes(&mut self) {}
}

pub static mut Mtvu: MtvuState = MtvuState::new();

// ---------------------------------------------------------------------------
// HW — EE hardware register bank + FIFO buffers.
// ---------------------------------------------------------------------------

pub const EE_HW_SIZE: usize = 0x10000; // 64 KiB register bank

pub struct HwState {
    /// Hardware register file (`eeHw[0x10000]`) — covers counters, DMAC,
    /// INTC, SIO, SBUS, VIF, GIF, IPU, GS-registers aliases, etc.
    pub hw: [u8; EE_HW_SIZE],
    /// SIO RX FIFO (`ee_sio_rx_fifo`).
    pub sio_rx: std::collections::VecDeque<u8>,
    /// SIO TX FIFO (`ee_sio_tx_fifo`).
    pub sio_tx: std::collections::VecDeque<u8>,
    pub rdram_devices: u32,
    pub rdram_sdevid: u32,
}

impl Default for HwState {
    fn default() -> Self {
        Self {
            hw: [0; EE_HW_SIZE],
            sio_rx: std::collections::VecDeque::new(),
            sio_tx: std::collections::VecDeque::new(),
            rdram_devices: 0,
            rdram_sdevid: 0,
        }
    }
}

impl HwState {
    pub const fn new() -> Self {
        Self {
            hw: [0; EE_HW_SIZE],
            sio_rx: std::collections::VecDeque::new(),
            sio_tx: std::collections::VecDeque::new(),
            rdram_devices: 2,
            rdram_sdevid: 0,
        }
    }
    pub fn reset(&mut self) {
        self.hw.fill(0);
        self.sio_rx.clear();
        self.sio_tx.clear();
        // Mirror the magic values from `hwReset()`.
        self.hw_u32(0x1000_F260, 0x1D00_0060);
        self.hw_u32(0x1000_F590, 0x1201);
        self.hw_u32(0x1000_F520, 0x1201);
    }
    pub fn hw_u32(&mut self, addr: u32, val: u32) {
        let off = (addr as usize) & (EE_HW_SIZE - 1) & !3;
        let bytes = val.to_le_bytes();
        self.hw[off] = bytes[0];
        self.hw[off + 1] = bytes[1];
        self.hw[off + 2] = bytes[2];
        self.hw[off + 3] = bytes[3];
    }
}

pub static mut Hw: HwState = HwState::new();

// ---------------------------------------------------------------------------
// Achievements — RetroAchievements client stub.
// ---------------------------------------------------------------------------

pub enum LoginReason { UserInitiated, TokenInvalid }

#[derive(Default)]
pub struct AchievementsState {
    pub initialized: bool,
    pub hardcore_mode: bool,
    pub encore_mode: bool,
    pub spectator_mode: bool,
    pub unofficial_test_mode: bool,
    pub game_id: u32,
    pub username: String,
    pub token: String,
    pub logged_in: bool,
    pub rich_presence: String,
    pub game_icon_url: String,
    pub game_title: String,
    pub frame_count: u64,
    pub last_frame: Duration,
}

impl AchievementsState {
    pub const fn new() -> Self {
        Self {
            initialized: false,
            hardcore_mode: false,
            encore_mode: false,
            spectator_mode: false,
            unofficial_test_mode: false,
            game_id: 0,
            username: String::new(),
            token: String::new(),
            logged_in: false,
            rich_presence: String::new(),
            game_icon_url: String::new(),
            game_title: String::new(),
            frame_count: 0,
            last_frame: Duration::from_secs(0),
        }
    }
    pub fn initialize(&mut self) -> bool { self.initialized = true; true }
    pub fn reset_client(&mut self) {
        self.game_id = 0;
        self.game_title.clear();
        self.rich_presence.clear();
        self.game_icon_url.clear();
        self.frame_count = 0;
    }
    pub fn login(&mut self, user: &str, _pass: &str) -> bool {
        self.username = user.to_string();
        self.logged_in = true;
        true
    }
    pub fn logout(&mut self) {
        self.logged_in = false;
        self.username.clear();
        self.token.clear();
    }
    pub fn game_changed(&mut self, disc_crc: u32, crc: u32) { self.game_id = crc ^ disc_crc; }
    pub fn frame_update(&mut self) { self.frame_count += 1; self.last_frame = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default(); }
    pub fn is_active(&self) -> bool { self.initialized }
    pub fn is_hardcore_active(&self) -> bool { self.hardcore_mode && self.logged_in }
    pub fn has_active_game(&self) -> bool { self.game_id != 0 }
}

pub static mut Achievements: AchievementsState = AchievementsState::new();
