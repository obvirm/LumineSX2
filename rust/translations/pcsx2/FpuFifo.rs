// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
//! Idiomatic Rust translation of `pcsx2/FPU.cpp` and `pcsx2/FiFo.cpp`.
//!
//! Combines two related parts of the EE's data plane into a single module:
//!
//! * **FPU state** — the COP1 register file, status/control registers, and
//!   accumulator/index registers. The C++ source spreads these across
//!   `FPU.cpp` and a number of header files; the Rust translation keeps
//!   them together with the same names (`FPUcs`, `FPUcc`, `fpuRegs`,
//!   `FPUregs`, ...) so existing callers can drop in unchanged.
//!
//! * **FIFOs** — a small generic [`Fifo<T>`] backed by `VecDeque<T>`. The
//!   C++ side has separate `ReadFIFO_VIF1`, `WriteFIFO_VIF0`,
//!   `WriteFIFO_VIF1`, and `WriteFIFO_GIF` entry points; in the Rust
//!   port each of those is expressed as a thin wrapper around a
//!   `Fifo<T>` of 128-bit quads (or whatever element type the consumer
//!   wants), with the host-side transfer logic left to the caller.
//!
//! All state lives in `static mut` mirrors of the C++ globals; the FPU
//! opcode implementations are exposed as plain functions so downstream
//! modules can call them without going through the interpreter
//! dispatch.

use std::collections::VecDeque;

// =====================================================================
// FPU control register (FCR31)
// =====================================================================

/// FPU control register (FCR31).
///
/// The architectural layout of FCR31 packs the rounding mode in bits 0-1,
/// the condition-code flags in bits 23/25/27/29/31 and 26/28/30, and a
/// number of sticky cause/flag bits elsewhere. This newtype wraps the raw
/// 32-bit value and provides dedicated `set_rm` / `get_rm` accessors for
/// the rounding-mode field; bit-level manipulation of the other fields
/// is the caller's responsibility.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct FPUControl {
    bits: u32,
}

impl FPUControl {
    /// Construct an `FPUControl` from its raw 32-bit representation.
    pub const fn from_bits(bits: u32) -> Self {
        Self { bits }
    }

    /// Read the raw 32-bit value.
    pub const fn bits(&self) -> u32 {
        self.bits
    }

    /// Set the rounding-mode field (bits 0-1).
    ///
    /// * `0` — round to nearest (RN)
    /// * `1` — round toward zero (RZ)
    /// * `2` — round toward positive infinity (RP)
    /// * `3` — round toward negative infinity (RM)
    pub fn set_rm(&mut self, rm: u8) {
        self.bits = (self.bits & !0x3) | ((rm as u32) & 0x3);
    }

    /// Read the rounding-mode field (bits 0-1).
    pub fn get_rm(&self) -> u8 {
        (self.bits & 0x3) as u8
    }
}

// =====================================================================
// FPU state
// =====================================================================

/// 128-bit (pair-of-`f64`) view of the FPU register file.
///
/// Each 128-bit slot packs two `f64` values — the lower 64 bits are the
/// "even" register and the upper 64 bits are the "odd" register. This
/// matches the EE's PR (paired-single) / SWC1 layout.
pub static mut FPUregs: [u128; 32] = [0u128; 32];

/// 32-bit `f32` view of the FPU register file. Mirrors `fpuRegs.fpr[]`
/// in the C++ source.
pub static mut fpuRegs: [f32; 32] = [0.0; 32];

/// FPU control/status register (FCR31), exposed as a raw `u32` for
/// callers that want direct bit access.
pub static mut FPUcs: u32 = 0;

/// FPU condition-code register file. 8 single-bit codes, packed one
/// per byte so the array is indexable by `cc` number.
pub static mut FPUcc: [u8; 8] = [0; 8];

/// FPU accumulator register (used by the MADD/MSUB family of opcodes).
pub static mut FPUACC: u128 = 0;

/// COP1 INDEX register.
pub static mut FPUINDEX: u32 = 0;

// =====================================================================
// FPU init / reset
// =====================================================================

/// Initialise the FPU state. Mirrors the C++ `fpuInit()`.
pub fn fpuInit() {
    fpuReset();
}

/// Reset the FPU to power-on defaults. Mirrors the C++ `fpuReset()`.
pub fn fpuReset() {
    // SAFETY: All FPU state is `static mut` and `fpuReset` is the
    // canonical initial writer; concurrent access is not possible in
    // the single-threaded interpreter path.
    unsafe {
        FPUregs = [0u128; 32];
        fpuRegs = [0.0; 32];
        FPUcs = 0;
        FPUcc = [0; 8];
        FPUACC = 0;
        FPUINDEX = 0;
    }
}

// =====================================================================
// FPU flag constants
// =====================================================================
//
// Architectural flag bits inside FCR31, mirroring the C++ `FPUflag*`
// macros. Kept in one place so the opcode implementations below can
// reference them by name.

/// FPU condition-code flag, bit 23 of FCR31.
pub const FPU_FLAG_C: u32 = 0x0080_0000;
/// FPU invalid-operation flag, bit 17 of FCR31.
pub const FPU_FLAG_I: u32 = 0x0002_0000;
/// FPU divide-by-zero flag, bit 16 of FCR31.
pub const FPU_FLAG_D: u32 = 0x0001_0000;
/// FPU overflow flag, bit 15 of FCR31.
pub const FPU_FLAG_O: u32 = 0x0000_8000;
/// FPU underflow flag, bit 14 of FCR31.
pub const FPU_FLAG_U: u32 = 0x0000_4000;
/// FPU sticky "invalid" flag, bit 6 of FCR31.
pub const FPU_FLAG_SI: u32 = 0x0000_0040;
/// FPU sticky "divide-by-zero" flag, bit 5 of FCR31.
pub const FPU_FLAG_SD: u32 = 0x0000_0020;
/// FPU sticky "overflow" flag, bit 4 of FCR31.
pub const FPU_FLAG_SO: u32 = 0x0000_0010;
/// FPU sticky "underflow" flag, bit 3 of FCR31.
pub const FPU_FLAG_SU: u32 = 0x0000_0008;

// =====================================================================
// FPU helpers / opcodes
// =====================================================================

/// Convert a raw 32-bit IEEE-754 word to `f32`, clamping infinities to
/// the largest representable finite value (the EE's FPU doesn't honour
/// infinities the way the host does). Mirrors `fpuDouble()` in the C++.
#[inline]
pub fn fpuDouble(bits: u32) -> f32 {
    let exp = bits & 0x7f80_0000;
    match exp {
        // Zero / denormal — keep just the sign bit.
        0 => f32::from_bits(bits & 0x8000_0000),
        // Positive / negative infinity — clamp to +/-fmax.
        0x7f80_0000 => f32::from_bits((bits & 0x8000_0000) | 0x7f7f_ffff),
        // Normal value — bit-cast straight to `f32`.
        _ => f32::from_bits(bits),
    }
}

/// `index & 31` — convenience mask used by every FPU opcode.
#[inline]
fn reg(index: usize) -> usize {
    index & 31
}

/// ABS.S — `fd = |fs|`.
pub fn abs_s(fd: usize, fs: usize) {
    // SAFETY: see `fpuReset`.
    unsafe {
        let raw = fpuRegs[reg(fs)].to_bits() & 0x7fff_ffff;
        fpuRegs[reg(fd)] = f32::from_bits(raw);
        FPUcs &= !(FPU_FLAG_O | FPU_FLAG_U);
    }
}

/// NEG.S — `fd = -fs`.
pub fn neg_s(fd: usize, fs: usize) {
    // SAFETY: see `fpuReset`.
    unsafe {
        let raw = fpuRegs[reg(fs)].to_bits() ^ 0x8000_0000;
        fpuRegs[reg(fd)] = f32::from_bits(raw);
        FPUcs &= !(FPU_FLAG_O | FPU_FLAG_U);
    }
}

/// ADD.S — `fd = fs + ft`.
pub fn add_s(fd: usize, fs: usize, ft: usize) {
    // SAFETY: see `fpuReset`.
    unsafe {
        fpuRegs[reg(fd)] = fpuRegs[reg(fs)] + fpuRegs[reg(ft)];
    }
}

/// SUB.S — `fd = fs - ft`.
pub fn sub_s(fd: usize, fs: usize, ft: usize) {
    // SAFETY: see `fpuReset`.
    unsafe {
        fpuRegs[reg(fd)] = fpuRegs[reg(fs)] - fpuRegs[reg(ft)];
    }
}

/// MUL.S — `fd = fs * ft`.
pub fn mul_s(fd: usize, fs: usize, ft: usize) {
    // SAFETY: see `fpuReset`.
    unsafe {
        fpuRegs[reg(fd)] = fpuRegs[reg(fs)] * fpuRegs[reg(ft)];
    }
}

/// DIV.S — `fd = fs / ft`.
pub fn div_s(fd: usize, fs: usize, ft: usize) {
    // SAFETY: see `fpuReset`.
    unsafe {
        fpuRegs[reg(fd)] = fpuRegs[reg(fs)] / fpuRegs[reg(ft)];
    }
}

/// SQRT.S — `fd = sqrt(ft)`.
pub fn sqrt_s(fd: usize, ft: usize) {
    // SAFETY: see `fpuReset`.
    unsafe {
        fpuRegs[reg(fd)] = fpuRegs[reg(ft)].sqrt();
    }
}

/// MOV.S — `fd = fs`.
pub fn mov_s(fd: usize, fs: usize) {
    // SAFETY: see `fpuReset`.
    unsafe {
        fpuRegs[reg(fd)] = fpuRegs[reg(fs)];
    }
}

// =====================================================================
// FPU helpers (overflow / underflow / divide-by-zero / max / min)
// =====================================================================
//
// Mirrors the C++ `checkOverflow`, `checkUnderflow`, `fp_max`, `fp_min`,
// and `checkDivideByZero` helpers. The helpers mutate `FPUcs` and may
// also clamp the destination register to a finite value when an
// exceptional condition fires; callers pass a mutable reference to the
// destination register's raw `u32` representation.

/// If `x_reg` is `+/-Infinity`, clamp it to `+/-Fmax` and set the cause
/// bits in `c_flags_to_set` on FCR31. Mirrors the C++ `checkOverflow`.
#[inline]
pub fn check_overflow(x_reg: &mut u32, c_flags_to_set: u32) -> bool {
    // SAFETY: `FPUcs` is a `static mut`; callers serialize through the
    // interpreter dispatch and don't race here.
    unsafe {
        if (*x_reg & !0x8000_0000) == 0x7f80_0000 {
            *x_reg = (*x_reg & 0x8000_0000) | 0x7f7f_ffff;
            FPUcs |= c_flags_to_set;
            true
        } else {
            if c_flags_to_set & FPU_FLAG_O != 0 {
                FPUcs &= !FPU_FLAG_O;
            }
            false
        }
    }
}

/// If `x_reg` is a denormal, clamp it to `+/-0` and set the cause bits
/// in `c_flags_to_set` on FCR31. Mirrors the C++ `checkUnderflow`.
#[inline]
pub fn check_underflow(x_reg: &mut u32, c_flags_to_set: u32) -> bool {
    // SAFETY: see `check_overflow`.
    unsafe {
        if (*x_reg & 0x7f80_0000) == 0 && (*x_reg & 0x007f_ffff) != 0 {
            *x_reg &= 0x8000_0000;
            FPUcs |= c_flags_to_set;
            true
        } else {
            if c_flags_to_set & FPU_FLAG_U != 0 {
                FPUcs &= !FPU_FLAG_U;
            }
            false
        }
    }
}

/// `fp_max` — bit-level max that treats negatives specially. Mirrors
/// the C++ `fp_max`.
#[inline]
pub fn fp_max(a: u32, b: u32) -> u32 {
    if (a as i32) < 0 && (b as i32) < 0 {
        a.min(b)
    } else {
        a.max(b)
    }
}

/// `fp_min` — bit-level min that treats negatives specially. Mirrors
/// the C++ `fp_min`.
#[inline]
pub fn fp_min(a: u32, b: u32) -> u32 {
    if (a as i32) < 0 && (b as i32) < 0 {
        a.max(b)
    } else {
        a.min(b)
    }
}

/// Detect divide-by-zero on `y_divisor_reg`; clamp `x_reg` to
/// `+/-Fmax` and set the appropriate cause bits on FCR31. Mirrors the
/// C++ `checkDivideByZero`.
#[inline]
pub fn check_divide_by_zero(
    x_reg: &mut u32,
    y_divisor_reg: u32,
    z_dividend_reg: u32,
    c_flags_to_set_1: u32,
    c_flags_to_set_2: u32,
) -> bool {
    // SAFETY: see `check_overflow`.
    unsafe {
        if (y_divisor_reg & 0x7f80_0000) == 0 {
            FPUcs |= if (z_dividend_reg & 0x7f80_0000) == 0 {
                c_flags_to_set_2
            } else {
                c_flags_to_set_1
            };
            *x_reg = ((y_divisor_reg ^ z_dividend_reg) & 0x8000_0000) | 0x7f7f_ffff;
            true
        } else {
            false
        }
    }
}

// ---------------------------------------------------------------------
// Accumulator register accessors
// ---------------------------------------------------------------------
//
// `FPUACC` is exposed as a `u128` for layout compatibility with the
// paired-single register file, but the C++ `fpuRegs.ACC` slot is a
// single `f32`. The helpers below hide the cast so callers can treat
// it as an `f32`-sized slot.

/// Read the lower 32 bits of `FPUACC` (the actual stored value).
#[inline]
unsafe fn acc_get_bits() -> u32 {
    FPUACC as u32
}

/// Write the lower 32 bits of `FPUACC`.
#[inline]
unsafe fn acc_set_bits(bits: u32) {
    FPUACC = bits as u128;
}

// =====================================================================
// Additional FPU opcodes
// =====================================================================
//
// Mirrors the remainder of the COP1 opcode implementations in
// `pcsx2/FPU.cpp`: max/min, accumulator variants, multiply-add/sub,
// reciprocal sqrt, convert, and the compare family. The branch
// opcodes (BC1F/T/FL/TL) and load/store ops (LWC1/SWC1) live in the
// interpreter dispatch, not here.

// --- Max / Min --------------------------------------------------------

/// MAX.S — `fd = max(fs, ft)` using bit-level semantics.
pub fn max_s(fd: usize, fs: usize, ft: usize) {
    // SAFETY: see `fpuReset`.
    unsafe {
        let result = fp_max(fpuRegs[reg(fs)].to_bits(), fpuRegs[reg(ft)].to_bits());
        fpuRegs[reg(fd)] = f32::from_bits(result);
        FPUcs &= !(FPU_FLAG_O | FPU_FLAG_U);
    }
}

/// MIN.S — `fd = min(fs, ft)` using bit-level semantics.
pub fn min_s(fd: usize, fs: usize, ft: usize) {
    // SAFETY: see `fpuReset`.
    unsafe {
        let result = fp_min(fpuRegs[reg(fs)].to_bits(), fpuRegs[reg(ft)].to_bits());
        fpuRegs[reg(fd)] = f32::from_bits(result);
        FPUcs &= !(FPU_FLAG_O | FPU_FLAG_U);
    }
}

// --- Accumulator variants --------------------------------------------

/// ADDA.S — `ACC = fs + ft` (with overflow/underflow flags).
pub fn adda_s(fs: usize, ft: usize) {
    // SAFETY: see `fpuReset`.
    unsafe {
        let sum = fpuDouble(fpuRegs[reg(fs)].to_bits())
            + fpuDouble(fpuRegs[reg(ft)].to_bits());
        let mut bits = sum.to_bits();
        acc_set_bits(bits);
        if check_overflow(&mut bits, FPU_FLAG_O | FPU_FLAG_SO) {
            acc_set_bits(bits);
            return;
        }
        check_underflow(&mut bits, FPU_FLAG_U | FPU_FLAG_SU);
        acc_set_bits(bits);
    }
}

/// SUBA.S — `ACC = fs - ft` (with overflow/underflow flags).
pub fn suba_s(fs: usize, ft: usize) {
    // SAFETY: see `fpuReset`.
    unsafe {
        let diff = fpuDouble(fpuRegs[reg(fs)].to_bits())
            - fpuDouble(fpuRegs[reg(ft)].to_bits());
        let mut bits = diff.to_bits();
        acc_set_bits(bits);
        if check_overflow(&mut bits, FPU_FLAG_O | FPU_FLAG_SO) {
            acc_set_bits(bits);
            return;
        }
        check_underflow(&mut bits, FPU_FLAG_U | FPU_FLAG_SU);
        acc_set_bits(bits);
    }
}

/// MULA.S — `ACC = fs * ft` (with overflow/underflow flags).
pub fn mula_s(fs: usize, ft: usize) {
    // SAFETY: see `fpuReset`.
    unsafe {
        let prod = fpuDouble(fpuRegs[reg(fs)].to_bits())
            * fpuDouble(fpuRegs[reg(ft)].to_bits());
        let mut bits = prod.to_bits();
        acc_set_bits(bits);
        if check_overflow(&mut bits, FPU_FLAG_O | FPU_FLAG_SO) {
            acc_set_bits(bits);
            return;
        }
        check_underflow(&mut bits, FPU_FLAG_U | FPU_FLAG_SU);
        acc_set_bits(bits);
    }
}

/// MADDA.S — `ACC += fs * ft` (with overflow/underflow flags).
pub fn madda_s(fs: usize, ft: usize) {
    // SAFETY: see `fpuReset`.
    unsafe {
        let prod = fpuDouble(fpuRegs[reg(fs)].to_bits())
            * fpuDouble(fpuRegs[reg(ft)].to_bits());
        let acc_val = fpuDouble(acc_get_bits());
        let mut bits = (acc_val + prod).to_bits();
        acc_set_bits(bits);
        if check_overflow(&mut bits, FPU_FLAG_O | FPU_FLAG_SO) {
            acc_set_bits(bits);
            return;
        }
        check_underflow(&mut bits, FPU_FLAG_U | FPU_FLAG_SU);
        acc_set_bits(bits);
    }
}

/// MSUBA.S — `ACC -= fs * ft` (with overflow/underflow flags).
pub fn msuba_s(fs: usize, ft: usize) {
    // SAFETY: see `fpuReset`.
    unsafe {
        let prod = fpuDouble(fpuRegs[reg(fs)].to_bits())
            * fpuDouble(fpuRegs[reg(ft)].to_bits());
        let acc_val = fpuDouble(acc_get_bits());
        let mut bits = (acc_val - prod).to_bits();
        acc_set_bits(bits);
        if check_overflow(&mut bits, FPU_FLAG_O | FPU_FLAG_SO) {
            acc_set_bits(bits);
            return;
        }
        check_underflow(&mut bits, FPU_FLAG_U | FPU_FLAG_SU);
        acc_set_bits(bits);
    }
}

// --- Multiply-add / multiply-sub -------------------------------------

/// MADD.S — `fd = ACC + (fs * ft)`.
pub fn madd_s(fd: usize, fs: usize, ft: usize) {
    // SAFETY: see `fpuReset`.
    unsafe {
        let prod = fpuDouble(fpuRegs[reg(fs)].to_bits())
            * fpuDouble(fpuRegs[reg(ft)].to_bits());
        let acc_val = fpuDouble(acc_get_bits());
        let mut bits = (acc_val + prod).to_bits();
        fpuRegs[reg(fd)] = f32::from_bits(bits);
        if check_overflow(&mut bits, FPU_FLAG_O | FPU_FLAG_SO) {
            fpuRegs[reg(fd)] = f32::from_bits(bits);
            return;
        }
        check_underflow(&mut bits, FPU_FLAG_U | FPU_FLAG_SU);
        fpuRegs[reg(fd)] = f32::from_bits(bits);
    }
}

/// MSUB.S — `fd = ACC - (fs * ft)`.
pub fn msub_s(fd: usize, fs: usize, ft: usize) {
    // SAFETY: see `fpuReset`.
    unsafe {
        let prod = fpuDouble(fpuRegs[reg(fs)].to_bits())
            * fpuDouble(fpuRegs[reg(ft)].to_bits());
        let acc_val = fpuDouble(acc_get_bits());
        let mut bits = (acc_val - prod).to_bits();
        fpuRegs[reg(fd)] = f32::from_bits(bits);
        if check_overflow(&mut bits, FPU_FLAG_O | FPU_FLAG_SO) {
            fpuRegs[reg(fd)] = f32::from_bits(bits);
            return;
        }
        check_underflow(&mut bits, FPU_FLAG_U | FPU_FLAG_SU);
        fpuRegs[reg(fd)] = f32::from_bits(bits);
    }
}

// --- Reciprocal sqrt --------------------------------------------------

/// RSQRT.S — `fd = fs / sqrt(ft)` with all the special cases.
pub fn rsqrt_s(fd: usize, fs: usize, ft: usize) {
    // SAFETY: see `fpuReset`.
    unsafe {
        FPUcs &= !(FPU_FLAG_D | FPU_FLAG_I);
        let ft_bits = fpuRegs[reg(ft)].to_bits();

        // Ft is zero (denormals count as zero): divide-by-zero.
        if (ft_bits & 0x7f80_0000) == 0 {
            FPUcs |= FPU_FLAG_D | FPU_FLAG_SD;
            fpuRegs[reg(fd)] = f32::from_bits((ft_bits & 0x8000_0000) | 0x7f7f_ffff);
            return;
        }

        let mut bits = if ft_bits & 0x8000_0000 != 0 {
            // Ft is negative: invalid, use abs().
            FPUcs |= FPU_FLAG_I | FPU_FLAG_SI;
            let ft_val = fpuDouble(ft_bits).abs().sqrt();
            (fpuDouble(fpuRegs[reg(fs)].to_bits()) / ft_val).to_bits()
        } else {
            // Ft is positive and not zero.
            (fpuDouble(fpuRegs[reg(fs)].to_bits()) / fpuDouble(ft_bits).sqrt()).to_bits()
        };

        fpuRegs[reg(fd)] = f32::from_bits(bits);
        if check_overflow(&mut bits, 0) {
            fpuRegs[reg(fd)] = f32::from_bits(bits);
            return;
        }
        check_underflow(&mut bits, 0);
        fpuRegs[reg(fd)] = f32::from_bits(bits);
    }
}

// --- Convert ----------------------------------------------------------

/// CVT.S.W — convert integer to float: `fd = (f32)(s32)fs`.
pub fn cvt_s(fd: usize, fs: usize) {
    // SAFETY: see `fpuReset`.
    unsafe {
        let bits = fpuRegs[reg(fs)].to_bits();
        let signed = bits as i32;
        fpuRegs[reg(fd)] = signed as f32;
    }
}

/// CVT.W.S — convert float to integer, saturating on overflow.
pub fn cvt_w(fd: usize, fs: usize) {
    // SAFETY: see `fpuReset`.
    unsafe {
        let fs_bits = fpuRegs[reg(fs)].to_bits();
        if (fs_bits & 0x7f80_0000) <= 0x4e80_0000 {
            // In-range; convert via the f32-as-i32 path.
            let result = fpuRegs[reg(fs)] as i32;
            fpuRegs[reg(fd)] = f32::from_bits(result as u32);
        } else if (fs_bits & 0x8000_0000) == 0 {
            // Positive overflow: saturate to 0x7fffffff.
            fpuRegs[reg(fd)] = f32::from_bits(0x7fff_ffff);
        } else {
            // Negative overflow: saturate to 0x80000000.
            fpuRegs[reg(fd)] = f32::from_bits(0x8000_0000);
        }
    }
}

// --- Compare ----------------------------------------------------------

#[inline]
unsafe fn set_cond(cond: bool) {
    if cond {
        FPUcs |= FPU_FLAG_C;
    } else {
        FPUcs &= !FPU_FLAG_C;
    }
}

/// C_EQ.S — set `FPU_FLAG_C` iff `fs == ft`, else clear it.
pub fn c_eq_s(fs: usize, ft: usize) {
    // SAFETY: see `fpuReset`.
    unsafe {
        let cond = fpuDouble(fpuRegs[reg(fs)].to_bits())
            == fpuDouble(fpuRegs[reg(ft)].to_bits());
        set_cond(cond);
    }
}

/// C_F.S — always clear `FPU_FLAG_C`.
pub fn c_f_s() {
    // SAFETY: see `fpuReset`.
    unsafe {
        FPUcs &= !FPU_FLAG_C;
    }
}

/// C_LE.S — set `FPU_FLAG_C` iff `fs <= ft`, else clear it.
pub fn c_le_s(fs: usize, ft: usize) {
    // SAFETY: see `fpuReset`.
    unsafe {
        let cond = fpuDouble(fpuRegs[reg(fs)].to_bits())
            <= fpuDouble(fpuRegs[reg(ft)].to_bits());
        set_cond(cond);
    }
}

/// C_LT.S — set `FPU_FLAG_C` iff `fs < ft`, else clear it.
pub fn c_lt_s(fs: usize, ft: usize) {
    // SAFETY: see `fpuReset`.
    unsafe {
        let cond = fpuDouble(fpuRegs[reg(fs)].to_bits())
            < fpuDouble(fpuRegs[reg(ft)].to_bits());
        set_cond(cond);
    }
}

// =====================================================================
// FIFO
// =====================================================================

/// Generic FIFO backed by `VecDeque<T>`.
///
/// The C++ side uses `mem128_t` quads for the in/out FIFOs that feed
/// VIF0/VIF1, the GIF unit, and the GS; the port exposes a type-agnostic
/// `Fifo<T>` and leaves the 128-bit element type choice to the caller
/// (typically `u128` for the VIF/GIF FIFOs, `u32` for the smaller PS2
/// device FIFOs).
#[derive(Debug, Clone)]
pub struct Fifo<T> {
    inner: VecDeque<T>,
}

impl<T> Fifo<T> {
    /// Construct an empty FIFO.
    pub const fn new() -> Self {
        Self {
            inner: VecDeque::new(),
        }
    }

    /// Construct a FIFO pre-allocated to hold at least `cap` elements.
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            inner: VecDeque::with_capacity(cap),
        }
    }

    /// Push `value` onto the back of the FIFO.
    pub fn push(&mut self, value: T) {
        self.inner.push_back(value);
    }

    /// Pop the front of the FIFO. Returns `None` if the FIFO is empty.
    pub fn pop(&mut self) -> Option<T> {
        self.inner.pop_front()
    }

    /// Clear the FIFO, dropping all elements.
    pub fn clear(&mut self) {
        self.inner.clear();
    }

    /// Number of elements currently in the FIFO.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// `true` if the FIFO holds no elements.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

impl<T> Default for Fifo<T> {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------
// FIFO page wrappers
// ---------------------------------------------------------------------
//
// The C++ `FiFo.cpp` defines four page-mapped FIFO entry points:
//   * 0x4000-0x5000 : VIF0  (write)
//   * 0x5000-0x6000 : VIF1  (read/write)
//   * 0x6000-0x7000 : GS    (read, via VIF1)
//   * 0x7000-0x8000 : IPU   (read/write)
//
// The Rust translation captures the data-plane shape of those FIFOs
// (one `Fifo<u128>` per PS2 device) and leaves the host-side transfer
// logic to the consumer; the original `ReadFIFO_VIF1`, `WriteFIFO_VIF0`,
// `WriteFIFO_VIF1`, and `WriteFIFO_GIF` bodies are reduced to a single
// `Fifo` instance shared with the rest of the emulator.

/// VIF0 write FIFO.
pub static mut VIF0_FIFO: Fifo<u128> = Fifo::new();
/// VIF1 read/write FIFO.
pub static mut VIF1_FIFO: Fifo<u128> = Fifo::new();
/// GIF write FIFO.
pub static mut GIF_FIFO: Fifo<u128> = Fifo::new();
/// IPU read/write FIFO.
pub static mut IPU_FIFO: Fifo<u128> = Fifo::new();
