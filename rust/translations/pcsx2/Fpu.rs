// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
//! Rust translation of the legacy `pcsx2/FPU.cpp` source.
//!
//! COP1 is the EE's floating-point unit (FPU). In the original C++ this file
//! hosted the COP1 opcode dispatch (`ABS_S`, `ADD_S`, `MUL_S`, `C_EQ`, ...),
//! the conditional-branch macros (`BC1F` / `BC1T` / `BC1FL` / `BC1TL`), the
//! FCR31 control register accessors (`CFC1` / `CTC1`), the convert opcodes
//! (`CVT_S` / `CVT_W`), and the FPU load/store helpers (`LWC1` / `SWC1`).
//!
//! The Rust translation focuses on the COP1 opcode surface. State and the
//! basic arithmetic primitives (`add_s`, `sub_s`, `mul_s`, `div_s`, `sqrt_s`,
//! `abs_s`, `neg_s`, `mov_s`, `fpuDouble`) already live in the sibling
//! [`FpuFifo`](crate::FpuFifo) module, so the functions here reference those
//! statics directly instead of duplicating them.
//!
//! The interpreter-side opcode dispatch in the C++ source uses a handful of
//! global accessors:
//!
//! * `cpuRegs.code` — the raw 32-bit COP1 instruction word
//! * `cpuRegs.GPR.r[_Rt_]` / `_Rs_` / `_Rd_` — the GPR file
//! * `fpuRegs.fpr[_Fs_|_Ft_|_Fd_]` — the 32-bit FPU register file
//! * `fpuRegs.fprc[31]` — FCR31
//! * `fpuRegs.ACC` — the FPU accumulator (used by MADD/MSUB)
//!
//! In the Rust port the GPR/PC state lives in [`CoreMain`](crate::CoreMain)
//! and the FPU register file in [`FpuFifo`](crate::FpuFifo); the COP1
//! opcode functions below operate on those globals directly, matching the
//! C++ implementation's behaviour.

use crate::pcsx2::CoreMain;
use crate::pcsx2::FpuFifo;

// =====================================================================
// Operand extraction helpers
// =====================================================================
//
// The C++ source uses preprocessor macros (_Fs_, _Ft_, _Fd_, _Rt_, _Rs_,
// _Rd_, _ContVal_) that all read out of the current COP1 instruction word
// in `cpuRegs.code`. In Rust we expose small `#[inline]` helpers that
// pull those fields out of the same word, keeping the opcode bodies
// close to the original.

// Bits 21-25: Fs (source FP register).
#[inline]
fn fs(code: u32) -> usize {
    ((code >> 11) & 0x1F) as usize
}

// Bits 16-20: Ft (target FP register).
#[inline]
fn ft(code: u32) -> usize {
    ((code >> 16) & 0x1F) as usize
}

// Bits 6-10: Fd (destination FP register).
#[inline]
fn fd(code: u32) -> usize {
    ((code >> 6) & 0x1F) as usize
}

// Bits 16-20: Rt (GPR target for CFC1/MFC1/LWC1/SWC1).
#[inline]
fn rt(code: u32) -> usize {
    ((code >> 16) & 0x1F) as usize
}

// Bits 21-25: Rs (GPR base for LWC1/SWC1).
#[inline]
fn rs(code: u32) -> usize {
    ((code >> 21) & 0x1F) as usize
}

// Sign-extended 16-bit immediate (used by LWC1/SWC1).
#[inline]
fn simm16(code: u32) -> u32 {
    (code & 0xFFFF) as i16 as u32
}

// =====================================================================
// FPU helper functions
// =====================================================================
//
// These are the helper routines that the COP1 opcode bodies call into.
// `checkOverflow`, `checkUnderflow`, and `checkDivideByZero` mutate the
// destination register in place and update FCR31; `fp_max` / `fp_min` are
// the IEEE-754-aware MAX/MIN.S primitives that follow the PS2 spec's
// "negative-max / positive-min" rule for IEEE-negative operands.
//
// `FpuFifo::FPUACC` is typed as native `u128` (the FpuFifo module lives
// outside the `CoreMain` scope where the crate-wide `u128 = [u64; 2]`
// alias is in effect), so the accumulator helpers below refer to the
// native type via `std::primitive::u128` to avoid the shadowing.

/// Write `bits` to the low 32 bits of `FPUACC`, leaving the high 96 bits
/// untouched. Mirrors the C++ pattern of clearing the destination's
/// low half and OR-ing the new value in.
#[inline]
fn fpuacc_write_lo(bits: u32) {
    // SAFETY: callers must hold the FPU dispatch lock; the FPU is
    // single-threaded in the interpreter path.
    unsafe {
        FpuFifo::FPUACC = (FpuFifo::FPUACC
            & (0xFFFF_FFFF_FFFF_FFFF_FFFF_FFFF_0000_0000u128 as std::primitive::u128))
            | (bits as std::primitive::u128);
    }
}

/// Read the low 32 bits of `FPUACC` as a `u32`.
#[inline]
fn fpuacc_read_lo() -> u32 {
    // SAFETY: see `fpuacc_write_lo`.
    unsafe { FpuFifo::FPUACC as u32 }
}

/// IEEE 754 +infinity.
const POS_INFINITY: u32 = 0x7F80_0000;
/// IEEE 754 -infinity.
const NEG_INFINITY: u32 = 0xFF80_0000;
/// Largest finite positive float.
const POS_FMAX: u32 = 0x7F7F_FFFF;
/// Largest finite negative float.
const NEG_FMAX: u32 = 0xFF7F_FFFF;

/// If `xReg` is +/-infinity, clamp it to +/-fmax and set the overflow
/// flags in FCR31. Returns `true` if the clamp happened.
pub fn check_overflow(x_reg: &mut u32, c_flags_to_set: u32) -> bool {
    // SAFETY: FCR31 (`FPUcs`) is a `static mut` updated through the
    // FpuFifo module; the COP1 opcode dispatch is single-threaded.
    unsafe {
        if (*x_reg & !0x8000_0000) == POS_INFINITY {
            *x_reg = (*x_reg & 0x8000_0000) | POS_FMAX;
            FpuFifo::FPUcs |= c_flags_to_set;
            true
        } else if c_flags_to_set & FpuFifo::FPU_FLAG_O != 0 {
            FpuFifo::FPUcs &= !FpuFifo::FPU_FLAG_O;
            false
        } else {
            false
        }
    }
}

/// If `xReg` is a denormal, flush it to +/-0 and set the underflow
/// flags in FCR31. Returns `true` if the flush happened.
pub fn check_underflow(x_reg: &mut u32, c_flags_to_set: u32) -> bool {
    // SAFETY: see `check_overflow`.
    unsafe {
        if ((*x_reg & 0x7F80_0000) == 0) && ((*x_reg & 0x007F_FFFF) != 0) {
            *x_reg &= 0x8000_0000;
            FpuFifo::FPUcs |= c_flags_to_set;
            true
        } else if c_flags_to_set & FpuFifo::FPU_FLAG_U != 0 {
            FpuFifo::FPUcs &= !FpuFifo::FPU_FLAG_U;
            false
        } else {
            false
        }
    }
}

/// MAX.S helper. Both negative -> min; otherwise -> max.
#[inline]
pub fn fp_max(a: u32, b: u32) -> u32 {
    if (a as i32) < 0 && (b as i32) < 0 {
        a.min(b)
    } else {
        a.max(b)
    }
}

/// MIN.S helper. Both negative -> max; otherwise -> min.
#[inline]
pub fn fp_min(a: u32, b: u32) -> u32 {
    if (a as i32) < 0 && (b as i32) < 0 {
        a.max(b)
    } else {
        a.min(b)
    }
}

/// Detect a divide-by-zero and set the appropriate sticky flags.
/// Returns `true` if a divide-by-zero was detected and `x_reg` was
/// clamped to +/-fmax; the caller should then skip the actual divide.
pub fn check_divide_by_zero(
    x_reg: &mut u32,
    y_divisor: u32,
    z_dividend: u32,
    c_flags_to_set_1: u32,
    c_flags_to_set_2: u32,
) -> bool {
    if (y_divisor & 0x7F80_0000) == 0 {
        // SAFETY: see `check_overflow`.
        unsafe {
            FpuFifo::FPUcs |= if (z_dividend & 0x7F80_0000) == 0 {
                c_flags_to_set_2
            } else {
                c_flags_to_set_1
            };
        }
        *x_reg = ((y_divisor ^ z_dividend) & 0x8000_0000) | POS_FMAX;
        true
    } else {
        false
    }
}

/// Clear the FCR31 "Cause" flags. Mirrors the C++ `clearFPUFlags` macro.
#[inline]
fn clear_fpu_flags(c_flags: u32) {
    // SAFETY: FCR31 is a `static mut` updated through the FpuFifo
    // module; the COP1 opcode dispatch is single-threaded.
    unsafe {
        FpuFifo::FPUcs &= !c_flags;
    }
}

/// C.cond.S — compare and set FCR31.C. Used by the four C_EQ/C_F/C_LE/C_LT
/// handlers below. `cmp` is a closure that returns `true` when the
/// condition is satisfied.
#[inline]
fn c_cond_s(a: u32, b: u32, cmp: impl Fn(f32, f32) -> bool) {
    // SAFETY: see `clear_fpu_flags`.
    unsafe {
        FpuFifo::FPUcs = if cmp(FpuFifo::fpuDouble(a), FpuFifo::fpuDouble(b)) {
            FpuFifo::FPUcs | FpuFifo::FPU_FLAG_C
        } else {
            FpuFifo::FPUcs & !FpuFifo::FPU_FLAG_C
        };
    }
}

// =====================================================================
// COP1 arithmetic opcodes
// =====================================================================

/// `ADD.S` — `fd = fs + ft`.
pub fn add_s(code: u32) {
    // SAFETY: fpuRegs is a `static mut` updated through the FpuFifo
    // module; the COP1 opcode dispatch is single-threaded.
    unsafe {
        let fs_idx = fs(code);
        let ft_idx = ft(code);
        let fd_idx = fd(code);
        let result =
            FpuFifo::fpuDouble(FpuFifo::fpuRegs[fs_idx].to_bits()) + FpuFifo::fpuDouble(FpuFifo::fpuRegs[ft_idx].to_bits());
        FpuFifo::fpuRegs[fd_idx] = result;
        let mut bits = FpuFifo::fpuRegs[fd_idx].to_bits();
        if check_overflow(&mut bits, FpuFifo::FPU_FLAG_O | FpuFifo::FPU_FLAG_SO) {
            FpuFifo::fpuRegs[fd_idx] = f32::from_bits(bits);
            return;
        }
        check_underflow(&mut bits, FpuFifo::FPU_FLAG_U | FpuFifo::FPU_FLAG_SU);
        FpuFifo::fpuRegs[fd_idx] = f32::from_bits(bits);
    }
}

/// `ADDA.S` — `ACC = fs + ft`.
pub fn adda_s(code: u32) {
    // SAFETY: see `add_s`. The result lands in the low 32 bits of
    // `FPUACC`; we use `fpuacc_write_lo` / `fpuacc_read_lo` to avoid
    // the crate-wide `u128 = [u64; 2]` alias shadowing native `u128`.
    unsafe {
        let fs_idx = fs(code);
        let ft_idx = ft(code);
        let result = FpuFifo::fpuDouble(FpuFifo::fpuRegs[fs_idx].to_bits())
            + FpuFifo::fpuDouble(FpuFifo::fpuRegs[ft_idx].to_bits());
        fpuacc_write_lo(result.to_bits());
        let mut bits = fpuacc_read_lo();
        if check_overflow(&mut bits, FpuFifo::FPU_FLAG_O | FpuFifo::FPU_FLAG_SO) {
            fpuacc_write_lo(bits);
            return;
        }
        check_underflow(&mut bits, FpuFifo::FPU_FLAG_U | FpuFifo::FPU_FLAG_SU);
        fpuacc_write_lo(bits);
    }
}

/// `DIV.S` — `fd = fs / ft`.
pub fn div_s(code: u32) {
    // SAFETY: see `add_s`.
    unsafe {
        let fs_idx = fs(code);
        let ft_idx = ft(code);
        let fd_idx = fd(code);
        let mut x_bits = FpuFifo::fpuRegs[fd_idx].to_bits();
        if check_divide_by_zero(
            &mut x_bits,
            FpuFifo::fpuRegs[ft_idx].to_bits(),
            FpuFifo::fpuRegs[fs_idx].to_bits(),
            FpuFifo::FPU_FLAG_D | FpuFifo::FPU_FLAG_SD,
            FpuFifo::FPU_FLAG_I | FpuFifo::FPU_FLAG_SI,
        ) {
            FpuFifo::fpuRegs[fd_idx] = f32::from_bits(x_bits);
            return;
        }
        let result = FpuFifo::fpuDouble(FpuFifo::fpuRegs[fs_idx].to_bits())
            / FpuFifo::fpuDouble(FpuFifo::fpuRegs[ft_idx].to_bits());
        FpuFifo::fpuRegs[fd_idx] = result;
        let mut bits = FpuFifo::fpuRegs[fd_idx].to_bits();
        if check_overflow(&mut bits, 0) {
            FpuFifo::fpuRegs[fd_idx] = f32::from_bits(bits);
            return;
        }
        check_underflow(&mut bits, 0);
        FpuFifo::fpuRegs[fd_idx] = f32::from_bits(bits);
    }
}

/// `MUL.S` — `fd = fs * ft`.
pub fn mul_s(code: u32) {
    // SAFETY: see `add_s`.
    unsafe {
        let fs_idx = fs(code);
        let ft_idx = ft(code);
        let fd_idx = fd(code);
        let result = FpuFifo::fpuDouble(FpuFifo::fpuRegs[fs_idx].to_bits())
            * FpuFifo::fpuDouble(FpuFifo::fpuRegs[ft_idx].to_bits());
        FpuFifo::fpuRegs[fd_idx] = result;
        let mut bits = FpuFifo::fpuRegs[fd_idx].to_bits();
        if check_overflow(&mut bits, FpuFifo::FPU_FLAG_O | FpuFifo::FPU_FLAG_SO) {
            FpuFifo::fpuRegs[fd_idx] = f32::from_bits(bits);
            return;
        }
        check_underflow(&mut bits, FpuFifo::FPU_FLAG_U | FpuFifo::FPU_FLAG_SU);
        FpuFifo::fpuRegs[fd_idx] = f32::from_bits(bits);
    }
}

/// `MULA.S` — `ACC = fs * ft`.
pub fn mula_s(code: u32) {
    // SAFETY: see `adda_s`.
    unsafe {
        let fs_idx = fs(code);
        let ft_idx = ft(code);
        let result = FpuFifo::fpuDouble(FpuFifo::fpuRegs[fs_idx].to_bits())
            * FpuFifo::fpuDouble(FpuFifo::fpuRegs[ft_idx].to_bits());
        fpuacc_write_lo(result.to_bits());
        let mut bits = fpuacc_read_lo();
        if check_overflow(&mut bits, FpuFifo::FPU_FLAG_O | FpuFifo::FPU_FLAG_SO) {
            fpuacc_write_lo(bits);
            return;
        }
        check_underflow(&mut bits, FpuFifo::FPU_FLAG_U | FpuFifo::FPU_FLAG_SU);
        fpuacc_write_lo(bits);
    }
}

/// `MADD.S` — `fd = ACC + (fs * ft)`.
pub fn madd_s(code: u32) {
    // SAFETY: see `adda_s`.
    unsafe {
        let fs_idx = fs(code);
        let ft_idx = ft(code);
        let fd_idx = fd(code);
        let acc_bits = FpuFifo::FPUACC;
        let acc_lo = (acc_bits & 0xFFFF_FFFF) as u32;
        let product = FpuFifo::fpuDouble(FpuFifo::fpuRegs[fs_idx].to_bits())
            * FpuFifo::fpuDouble(FpuFifo::fpuRegs[ft_idx].to_bits());
        let result = FpuFifo::fpuDouble(acc_lo) + FpuFifo::fpuDouble(product.to_bits());
        FpuFifo::fpuRegs[fd_idx] = result;
        let mut bits = FpuFifo::fpuRegs[fd_idx].to_bits();
        if check_overflow(&mut bits, FpuFifo::FPU_FLAG_O | FpuFifo::FPU_FLAG_SO) {
            FpuFifo::fpuRegs[fd_idx] = f32::from_bits(bits);
            return;
        }
        check_underflow(&mut bits, FpuFifo::FPU_FLAG_U | FpuFifo::FPU_FLAG_SU);
        FpuFifo::fpuRegs[fd_idx] = f32::from_bits(bits);
    }
}

/// `MADDA.S` — `ACC = ACC + (fs * ft)`.
pub fn madda_s(code: u32) {
    // SAFETY: see `adda_s`.
    unsafe {
        let fs_idx = fs(code);
        let ft_idx = ft(code);
        let acc_lo = fpuacc_read_lo();
        let product = FpuFifo::fpuDouble(FpuFifo::fpuRegs[fs_idx].to_bits())
            * FpuFifo::fpuDouble(FpuFifo::fpuRegs[ft_idx].to_bits());
        let result = FpuFifo::fpuDouble(acc_lo) + FpuFifo::fpuDouble(product.to_bits());
        fpuacc_write_lo(result.to_bits());
        let mut bits = fpuacc_read_lo();
        if check_overflow(&mut bits, FpuFifo::FPU_FLAG_O | FpuFifo::FPU_FLAG_SO) {
            fpuacc_write_lo(bits);
            return;
        }
        check_underflow(&mut bits, FpuFifo::FPU_FLAG_U | FpuFifo::FPU_FLAG_SU);
        fpuacc_write_lo(bits);
    }
}

/// `MSUB.S` — `fd = ACC - (fs * ft)`.
pub fn msub_s(code: u32) {
    // SAFETY: see `adda_s`.
    unsafe {
        let fs_idx = fs(code);
        let ft_idx = ft(code);
        let fd_idx = fd(code);
        let acc_bits = FpuFifo::FPUACC;
        let acc_lo = (acc_bits & 0xFFFF_FFFF) as u32;
        let product = FpuFifo::fpuDouble(FpuFifo::fpuRegs[fs_idx].to_bits())
            * FpuFifo::fpuDouble(FpuFifo::fpuRegs[ft_idx].to_bits());
        let result = FpuFifo::fpuDouble(acc_lo) - FpuFifo::fpuDouble(product.to_bits());
        FpuFifo::fpuRegs[fd_idx] = result;
        let mut bits = FpuFifo::fpuRegs[fd_idx].to_bits();
        if check_overflow(&mut bits, FpuFifo::FPU_FLAG_O | FpuFifo::FPU_FLAG_SO) {
            FpuFifo::fpuRegs[fd_idx] = f32::from_bits(bits);
            return;
        }
        check_underflow(&mut bits, FpuFifo::FPU_FLAG_U | FpuFifo::FPU_FLAG_SU);
        FpuFifo::fpuRegs[fd_idx] = f32::from_bits(bits);
    }
}

/// `MSUBA.S` — `ACC = ACC - (fs * ft)`.
pub fn msuba_s(code: u32) {
    // SAFETY: see `adda_s`.
    unsafe {
        let fs_idx = fs(code);
        let ft_idx = ft(code);
        let acc_lo = fpuacc_read_lo();
        let product = FpuFifo::fpuDouble(FpuFifo::fpuRegs[fs_idx].to_bits())
            * FpuFifo::fpuDouble(FpuFifo::fpuRegs[ft_idx].to_bits());
        let result = FpuFifo::fpuDouble(acc_lo) - FpuFifo::fpuDouble(product.to_bits());
        fpuacc_write_lo(result.to_bits());
        let mut bits = fpuacc_read_lo();
        if check_overflow(&mut bits, FpuFifo::FPU_FLAG_O | FpuFifo::FPU_FLAG_SO) {
            fpuacc_write_lo(bits);
            return;
        }
        check_underflow(&mut bits, FpuFifo::FPU_FLAG_U | FpuFifo::FPU_FLAG_SU);
        fpuacc_write_lo(bits);
    }
}

/// `SUB.S` — `fd = fs - ft`.
pub fn sub_s(code: u32) {
    // SAFETY: see `add_s`.
    unsafe {
        let fs_idx = fs(code);
        let ft_idx = ft(code);
        let fd_idx = fd(code);
        let result = FpuFifo::fpuDouble(FpuFifo::fpuRegs[fs_idx].to_bits())
            - FpuFifo::fpuDouble(FpuFifo::fpuRegs[ft_idx].to_bits());
        FpuFifo::fpuRegs[fd_idx] = result;
        let mut bits = FpuFifo::fpuRegs[fd_idx].to_bits();
        if check_overflow(&mut bits, FpuFifo::FPU_FLAG_O | FpuFifo::FPU_FLAG_SO) {
            FpuFifo::fpuRegs[fd_idx] = f32::from_bits(bits);
            return;
        }
        check_underflow(&mut bits, FpuFifo::FPU_FLAG_U | FpuFifo::FPU_FLAG_SU);
        FpuFifo::fpuRegs[fd_idx] = f32::from_bits(bits);
    }
}

/// `SUBA.S` — `ACC = fs - ft`.
pub fn suba_s(code: u32) {
    // SAFETY: see `adda_s`.
    unsafe {
        let fs_idx = fs(code);
        let ft_idx = ft(code);
        let result = FpuFifo::fpuDouble(FpuFifo::fpuRegs[fs_idx].to_bits())
            - FpuFifo::fpuDouble(FpuFifo::fpuRegs[ft_idx].to_bits());
        fpuacc_write_lo(result.to_bits());
        let mut bits = fpuacc_read_lo();
        if check_overflow(&mut bits, FpuFifo::FPU_FLAG_O | FpuFifo::FPU_FLAG_SO) {
            fpuacc_write_lo(bits);
            return;
        }
        check_underflow(&mut bits, FpuFifo::FPU_FLAG_U | FpuFifo::FPU_FLAG_SU);
        fpuacc_write_lo(bits);
    }
}

// =====================================================================
// COP1 sign-manipulation opcodes
// =====================================================================

/// `ABS.S` — `fd = |fs|`. Clears O/U sticky flags.
pub fn abs_s(code: u32) {
    // SAFETY: fpuRegs/FPUcs are `static mut` updated through the
    // FpuFifo module; the COP1 opcode dispatch is single-threaded.
    unsafe {
        let fs_idx = fs(code);
        let fd_idx = fd(code);
        let raw = FpuFifo::fpuRegs[fs_idx].to_bits() & 0x7FFF_FFFF;
        FpuFifo::fpuRegs[fd_idx] = f32::from_bits(raw);
        clear_fpu_flags(FpuFifo::FPU_FLAG_O | FpuFifo::FPU_FLAG_U);
    }
}

/// `NEG.S` — `fd = -fs`. Clears O/U sticky flags.
pub fn neg_s(code: u32) {
    // SAFETY: see `abs_s`.
    unsafe {
        let fs_idx = fs(code);
        let fd_idx = fd(code);
        let raw = FpuFifo::fpuRegs[fs_idx].to_bits() ^ 0x8000_0000;
        FpuFifo::fpuRegs[fd_idx] = f32::from_bits(raw);
        clear_fpu_flags(FpuFifo::FPU_FLAG_O | FpuFifo::FPU_FLAG_U);
    }
}

/// `MOV.S` — `fd = fs` (bitwise, no flags touched).
pub fn mov_s(code: u32) {
    // SAFETY: see `abs_s`.
    unsafe {
        let fs_idx = fs(code);
        let fd_idx = fd(code);
        FpuFifo::fpuRegs[fd_idx] = FpuFifo::fpuRegs[fs_idx];
    }
}

/// `MAX.S` — `fd = max(fs, ft)` with the PS2 negative-min quirk.
pub fn max_s(code: u32) {
    // SAFETY: see `abs_s`.
    unsafe {
        let fs_idx = fs(code);
        let ft_idx = ft(code);
        let fd_idx = fd(code);
        FpuFifo::fpuRegs[fd_idx] = f32::from_bits(fp_max(
            FpuFifo::fpuRegs[fs_idx].to_bits(),
            FpuFifo::fpuRegs[ft_idx].to_bits(),
        ));
        clear_fpu_flags(FpuFifo::FPU_FLAG_O | FpuFifo::FPU_FLAG_U);
    }
}

/// `MIN.S` — `fd = min(fs, ft)` with the PS2 negative-max quirk.
pub fn min_s(code: u32) {
    // SAFETY: see `abs_s`.
    unsafe {
        let fs_idx = fs(code);
        let ft_idx = ft(code);
        let fd_idx = fd(code);
        FpuFifo::fpuRegs[fd_idx] = f32::from_bits(fp_min(
            FpuFifo::fpuRegs[fs_idx].to_bits(),
            FpuFifo::fpuRegs[ft_idx].to_bits(),
        ));
        clear_fpu_flags(FpuFifo::FPU_FLAG_O | FpuFifo::FPU_FLAG_U);
    }
}

/// `SQRT.S` — `fd = sqrt(ft)`. Sets the I/SI flags on negative input.
pub fn sqrt_s(code: u32) {
    // SAFETY: see `abs_s`.
    unsafe {
        let ft_idx = ft(code);
        let fd_idx = fd(code);
        clear_fpu_flags(FpuFifo::FPU_FLAG_I | FpuFifo::FPU_FLAG_D);
        let ft_bits = FpuFifo::fpuRegs[ft_idx].to_bits();
        if (ft_bits & 0x7F80_0000) == 0 {
            // +/-0 -> result is 0 (sign preserved).
            FpuFifo::fpuRegs[fd_idx] = f32::from_bits(ft_bits & 0x8000_0000);
        } else if ft_bits & 0x8000_0000 != 0 {
            // Negative: invalid, sqrt of |ft|.
            FpuFifo::FPUcs |= FpuFifo::FPU_FLAG_I | FpuFifo::FPU_FLAG_SI;
            FpuFifo::fpuRegs[fd_idx] = FpuFifo::fpuDouble(ft_bits).abs().sqrt();
        } else {
            FpuFifo::fpuRegs[fd_idx] = FpuFifo::fpuDouble(ft_bits).sqrt();
        }
    }
}

/// `RSQRT.S` — `fd = fs / sqrt(ft)`. The EE spec's peculiar format:
///
/// * If `ft` is +/-0, result is +/-fmax and D/SD flags are set.
/// * If `ft` is negative, I/SI flags are set and we compute
///   `fs / sqrt(|ft|)`.
/// * Otherwise, result is `fs / sqrt(ft)`.
pub fn rsqrt_s(code: u32) {
    // SAFETY: see `abs_s`.
    unsafe {
        let fs_idx = fs(code);
        let ft_idx = ft(code);
        let fd_idx = fd(code);
        clear_fpu_flags(FpuFifo::FPU_FLAG_D | FpuFifo::FPU_FLAG_I);
        let ft_bits = FpuFifo::fpuRegs[ft_idx].to_bits();
        if (ft_bits & 0x7F80_0000) == 0 {
            // ft is zero (denormals are zero).
            FpuFifo::FPUcs |= FpuFifo::FPU_FLAG_D | FpuFifo::FPU_FLAG_SD;
            FpuFifo::fpuRegs[fd_idx] = f32::from_bits((ft_bits & 0x8000_0000) | POS_FMAX);
            return;
        }
        let result = if ft_bits & 0x8000_0000 != 0 {
            // ft is negative.
            FpuFifo::FPUcs |= FpuFifo::FPU_FLAG_I | FpuFifo::FPU_FLAG_SI;
            let sqrt_abs = FpuFifo::fpuDouble(ft_bits).abs().sqrt();
            FpuFifo::fpuDouble(FpuFifo::fpuRegs[fs_idx].to_bits()) / sqrt_abs
        } else {
            FpuFifo::fpuDouble(FpuFifo::fpuRegs[fs_idx].to_bits())
                / FpuFifo::fpuDouble(ft_bits).sqrt()
        };
        FpuFifo::fpuRegs[fd_idx] = result;
        let mut bits = FpuFifo::fpuRegs[fd_idx].to_bits();
        if check_overflow(&mut bits, 0) {
            FpuFifo::fpuRegs[fd_idx] = f32::from_bits(bits);
            return;
        }
        check_underflow(&mut bits, 0);
        FpuFifo::fpuRegs[fd_idx] = f32::from_bits(bits);
    }
}

// =====================================================================
// COP1 convert opcodes
// =====================================================================

/// `CVT.S` — `fd = (float) fs`. Truncates the 32-bit integer to single.
pub fn cvt_s(code: u32) {
    // SAFETY: see `abs_s`.
    unsafe {
        let fs_idx = fs(code);
        let fd_idx = fd(code);
        let raw = FpuFifo::fpuRegs[fs_idx].to_bits();
        FpuFifo::fpuRegs[fd_idx] = (raw as i32) as f32;
    }
}

/// `CVT.W` — `fd = (s32) fs`. Clamps to +/-INT_MAX on overflow.
pub fn cvt_w(code: u32) {
    // SAFETY: see `abs_s`.
    unsafe {
        let fs_idx = fs(code);
        let fd_idx = fd(code);
        let fs_bits = FpuFifo::fpuRegs[fs_idx].to_bits();
        if (fs_bits & 0x7F80_0000) <= 0x4E80_0000 {
            // In-range: truncate to s32.
            let raw = FpuFifo::fpuRegs[fs_idx].to_bits() as i32 as u32;
            FpuFifo::fpuRegs[fd_idx] = f32::from_bits(raw);
        } else if (fs_bits & 0x8000_0000) == 0 {
            // Positive overflow: clamp to +INT_MAX.
            FpuFifo::fpuRegs[fd_idx] = f32::from_bits(0x7FFF_FFFF);
        } else {
            // Negative overflow: clamp to -INT_MAX.
            FpuFifo::fpuRegs[fd_idx] = f32::from_bits(0x8000_0000);
        }
    }
}

// =====================================================================
// COP1 compare opcodes
// =====================================================================

/// `C_EQ` — set FCR31.C iff `fs == ft`.
pub fn c_eq(code: u32) {
    c_cond_s(
        unsafe { FpuFifo::fpuRegs[fs(code)].to_bits() },
        unsafe { FpuFifo::fpuRegs[ft(code)].to_bits() },
        |a, b| a == b,
    );
}

/// `C_F` — always clear FCR31.C.
pub fn c_f(_code: u32) {
    // SAFETY: see `abs_s`.
    unsafe {
        FpuFifo::FPUcs &= !FpuFifo::FPU_FLAG_C;
    }
}

/// `C_LE` — set FCR31.C iff `fs <= ft`.
pub fn c_le(code: u32) {
    c_cond_s(
        unsafe { FpuFifo::fpuRegs[fs(code)].to_bits() },
        unsafe { FpuFifo::fpuRegs[ft(code)].to_bits() },
        |a, b| a <= b,
    );
}

/// `C_LT` — set FCR31.C iff `fs < ft`.
pub fn c_lt(code: u32) {
    c_cond_s(
        unsafe { FpuFifo::fpuRegs[fs(code)].to_bits() },
        unsafe { FpuFifo::fpuRegs[ft(code)].to_bits() },
        |a, b| a < b,
    );
}

// =====================================================================
// COP1 conditional branch opcodes
// =====================================================================
//
// The C++ source uses a `BC1(cond)` / `BC1L(cond)` macro. The Rust port
// takes the branch target as an explicit `target` argument (the C++
// version reads it from the interpreter's branch-decoder context).

/// `BC1F` — branch if FCR31.C is clear.
pub fn bc1f(target: u32) {
    // SAFETY: see `abs_s`.
    unsafe {
        if FpuFifo::FPUcs & FpuFifo::FPU_FLAG_C == 0 {
            CoreMain::intDoBranch(target);
        }
    }
}

/// `BC1T` — branch if FCR31.C is set.
pub fn bc1t(target: u32) {
    // SAFETY: see `abs_s`.
    unsafe {
        if FpuFifo::FPUcs & FpuFifo::FPU_FLAG_C != 0 {
            CoreMain::intDoBranch(target);
        }
    }
}

/// `BC1FL` — branch-likely if FCR31.C is clear; else annul (PC += 4).
pub fn bc1fl(target: u32) {
    // SAFETY: see `abs_s`.
    unsafe {
        if FpuFifo::FPUcs & FpuFifo::FPU_FLAG_C == 0 {
            CoreMain::intDoBranch(target);
        } else {
            CoreMain::cpuRegs.pc = CoreMain::cpuRegs.pc.wrapping_add(4);
        }
    }
}

/// `BC1TL` — branch-likely if FCR31.C is set; else annul (PC += 4).
pub fn bc1tl(target: u32) {
    // SAFETY: see `abs_s`.
    unsafe {
        if FpuFifo::FPUcs & FpuFifo::FPU_FLAG_C != 0 {
            CoreMain::intDoBranch(target);
        } else {
            CoreMain::cpuRegs.pc = CoreMain::cpuRegs.pc.wrapping_add(4);
        }
    }
}

// =====================================================================
// COP1 control register access
// =====================================================================

/// `CFC1` — read FCR##fs into GPR rt (sign-extended to 64 bits).
///
/// Only FCR31 is wired up in the C++ source; FCR0 returns a constant
/// `0x2E00` (matches the EE Core Users Manual) and other FCRs return 0.
pub fn cfc1(code: u32) {
    // SAFETY: cpuRegs.GPR.r is a `static mut` updated through the
    // CoreMain module; the COP1 opcode dispatch is single-threaded.
    unsafe {
        let rt_idx = rt(code);
        if rt_idx == 0 {
            return;
        }
        let fs_idx = fs(code);
        let value: u64 = if fs_idx == 31 {
            FpuFifo::FPUcs as i32 as u64
        } else if fs_idx == 0 {
            0x2E00u64
        } else {
            0
        };
        // GprReg is `pub [u64; 2]`, so `.0[0]` is the lower 64 bits
        // (equivalent to C++ `SD[0]`). Sign-extend the 32-bit value
        // into the low 64 bits; the upper 64 bits stay zero.
        let low32 = (value & 0xFFFF_FFFF) as i32 as u64;
        CoreMain::cpuRegs.GPR.r[rt_idx].0[0] = low32;
    }
}

/// `CTC1` — write GPR rt into FCR31. Other FCRs are no-ops.
pub fn ctc1(code: u32) {
    // SAFETY: see `cfc1`.
    unsafe {
        let fs_idx = fs(code);
        if fs_idx != 31 {
            return;
        }
        let rt_idx = rt(code);
        FpuFifo::FPUcs = CoreMain::cpuRegs.GPR.r[rt_idx].0[0] as u32;
    }
}

/// `MFC1` — move FPR##fs to GPR rt (sign-extended).
pub fn mfc1(code: u32) {
    // SAFETY: see `cfc1`.
    unsafe {
        let rt_idx = rt(code);
        if rt_idx == 0 {
            return;
        }
        let fs_idx = fs(code);
        let value = FpuFifo::fpuRegs[fs_idx].to_bits();
        CoreMain::cpuRegs.GPR.r[rt_idx].0[0] = value as i32 as u64;
    }
}

/// `MTC1` — move GPR rt to FPR##fs.
pub fn mtc1(code: u32) {
    // SAFETY: see `cfc1`.
    unsafe {
        let rt_idx = rt(code);
        let fs_idx = fs(code);
        FpuFifo::fpuRegs[fs_idx] = f32::from_bits(CoreMain::cpuRegs.GPR.r[rt_idx].0[0] as u32);
    }
}

// =====================================================================
// COP1 load / store helpers (LWC1 / SWC1)
// =====================================================================

/// `LWC1` — load a 32-bit word from memory into FPR##rt.
pub fn lwc1(code: u32) {
    // SAFETY: see `cfc1`. The C++ source logs a console error on
    // misaligned addresses but does not raise an exception; we mirror
    // that behaviour by simply returning without touching the register.
    unsafe {
        let rs_idx = rs(code);
        let rt_idx = rt(code);
        let addr = (CoreMain::cpuRegs.GPR.r[rs_idx].0[0] as u32)
            .wrapping_add(simm16(code));
        if addr & 0x3 != 0 {
            eprintln!("FPU (LWC1 Opcode): Invalid Unaligned Memory Address");
            return;
        }
        FpuFifo::fpuRegs[rt_idx] = f32::from_bits(CoreMain::Memory.memRead32(addr));
    }
}

/// `SWC1` — store FPR##rt to memory as a 32-bit word.
pub fn swc1(code: u32) {
    // SAFETY: see `lwc1`.
    unsafe {
        let rs_idx = rs(code);
        let rt_idx = rt(code);
        let addr = (CoreMain::cpuRegs.GPR.r[rs_idx].0[0] as u32)
            .wrapping_add(simm16(code));
        if addr & 0x3 != 0 {
            eprintln!("FPU (SWC1 Opcode): Invalid Unaligned Memory Address");
            return;
        }
        CoreMain::Memory.memWrite32(addr, FpuFifo::fpuRegs[rt_idx].to_bits());
    }
}
