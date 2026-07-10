// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
//! Rust translation of the legacy `pcsx2/MMI.cpp` source.
//!
//! `MMI.cpp` hosts the interpreter-side implementations of the R5900
//! (Emotion Engine) MMI opcode family. The MMI encoding sits in the COP2
//! coprocessor opcode space of the EE and was added by Sony to extend the
//! MIPS-IV base instruction set with SIMD-style packed arithmetic. The
//! file is organised as four opcode groups:
//!
//!   1. **MMI base**      — `PLZCW`, `PMFHL`, `PMTHL`, parallel shifts
//!                          (`PSLLH` / `PSRLH` / `PSRAH` / `PSLLW` / ...)
//!   2. **MMI0**          — parallel adds, subs, compares, min/max, packs,
//!                          `PEXT5` / `PPAC5`, signed-saturating arithmetic
//!   3. **MMI1**          — `PABSW` / `PABSH`, `PCEQW` / `PCEQH` / `PCEQB`,
//!                          `PMINW` / `PMINH`, `PADSBH`, `PADDUW` / `PSUBUW`,
//!                          `PEXTUW`, `PADDUH` / `PSUBUH`, `PEXTUH`,
//!                          `PADDUB` / `PSUBUB`, `PEXTUB`, `QFSRV`
//!   4. **MMI2**          — `PMADDW` / `PMSUBW` / `PMULTW` / `PDIVW`,
//!                          `PMFHI` / `PMFLO`, `PINTH`, `PCPYLD`,
//!                          `PMADDH` / `PHMADH`, `PAND` / `PXOR`,
//!                          `PMSUBH` / `PHMSBH`, `PEXEH` / `PREVH`,
//!                          `PMULTH`, `PDIVBW`, `PEXEW`, `PROT3W`,
//!                          variable shifts (`PSLLVW` / `PSRLVW` / `PSRAVW`)
//!   5. **MMI3**          — `PMADDUW`, `PMTHI` / `PMTLO`, `PINTEH`,
//!                          `PMULTUW` / `PDIVUW`, `PCPYUD`, `POR` / `PNOR`,
//!                          `PEXCH`, `PCPYH`, `PEXCW`
//!
//! In addition to the SIMD arithmetic, `MMI.cpp` hosts a handful of non-MMI
//! instructions that share the same opcode class (the `MADD*` / `MULT*` /
//! `DIV*` / `MFHI1` / `MFLO1` / `MTHI1` / `MTLO1` family — these have a
//! single 32-bit GPR but use the HI/LO registers and "slot 1" duplicates
//! thereof).
//!
//! The Rust translation keeps the same shape as the C++ source: every opcode
//! is a `pub fn` that reads its operands out of `cpuRegs.code` via small
//! inline helpers and writes back through the GPR / HI / LO register file
//! in [`crate::pcsx2::CoreMain`]. We expose thin typed accessors on
//! [`GprReg`] so the bodies look close to the originals (`reg.sl(0)` for
//! `cpuRegs.GPR.r[i].SL[0]`, `reg.ul(1)` for `.UL[1]`, `reg.us(7)` for
//! `.US[7]`, and so on). The accessors are simple bit-cast helpers around
//! the underlying `pub [u64; 2]` storage; no behaviour is changed.

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(dead_code)]
#![allow(unused_assignments)]
#![allow(unused_variables)]

use crate::pcsx2::CoreMain;

// ---------------------------------------------------------------------------
// Operand extraction
// ---------------------------------------------------------------------------
//
// The C++ source uses preprocessor macros `_Rs_`, `_Rt_`, `_Rd_`, `_Sa_`,
// `_ContVal_` that read fields out of the current instruction word
// (`cpuRegs.code`). We expose small `#[inline]` helpers that pull the same
// fields out of the 32-bit MMI instruction encoding.

/// `_Rs_` — bits 21..25 (5 bits, source GPR index).
#[inline]
fn _rs(code: u32) -> usize {
    ((code >> 21) & 0x1F) as usize
}

/// `_Rt_` — bits 16..20 (5 bits, target GPR index).
#[inline]
fn _rt(code: u32) -> usize {
    ((code >> 16) & 0x1F) as usize
}

/// `_Rd_` — bits 11..15 (5 bits, destination GPR index).
#[inline]
fn _rd(code: u32) -> usize {
    ((code >> 11) & 0x1F) as usize
}

/// `_Sa_` — bits 6..10 (5 bits, shift amount).
#[inline]
fn _sa(code: u32) -> u32 {
    (code >> 6) & 0x1F
}

// ---------------------------------------------------------------------------
// GprReg typed views
// ---------------------------------------------------------------------------
//
// The C++ `GPR_reg` is a union over a `u64[2]` backing store. Members like
// `UD[0]`, `UL[0]`, `SL[0]`, `US[0]`, `SC[0]`, `SD[0]`, `SS[0]`, `UC[0]`
// all alias the same memory. The Rust `GprReg` is a plain newtype around
// `[u64; 2]`, so we provide a small set of accessor methods that mirror
// the C++ union members — they return `u32` / `u16` / `u8` / signed
// counterparts by truncating the lower 64 bits of the value.

use crate::pcsx2::CoreMain::GprReg;

#[inline]
fn gpr(idx: usize) -> GprReg {
    // SAFETY: caller holds the EE dispatch lock; the GPR file is single-threaded
    // in the interpreter path.
    unsafe { CoreMain::cpuRegs.GPR.r[idx] }
}

#[inline]
fn set_gpr(idx: usize, value: GprReg) {
    // SAFETY: see `gpr`.
    unsafe { CoreMain::cpuRegs.GPR.r[idx] = value }
}

#[inline]
fn lo() -> GprReg {
    // SAFETY: see `gpr`.
    unsafe { CoreMain::cpuRegs.LO }
}

#[inline]
fn set_lo(value: GprReg) {
    // SAFETY: see `gpr`.
    unsafe { CoreMain::cpuRegs.LO = value }
}

#[inline]
fn hi() -> GprReg {
    // SAFETY: see `gpr`.
    unsafe { CoreMain::cpuRegs.HI }
}

#[inline]
fn set_hi(value: GprReg) {
    // SAFETY: see `gpr`.
    unsafe { CoreMain::cpuRegs.HI = value }
}

/// Pack a `(lo: u32, hi: u32)` pair into a `GprReg`. The C++ code routinely
/// builds a 64-bit value by shifting and OR-ing two `u32`s together; this
/// helper centralises the bit-cast.
#[inline]
fn pack_u64(lo: u32, hi: u32) -> u64 {
    (lo as u64) | ((hi as u64) << 32)
}

impl GprReg {
    /// Build a `GprReg` from its underlying `u64` halves. Used in arithmetic
    /// helpers to assemble a 64-bit result and write it back through the
    /// GPR file.
    #[inline]
    pub const fn from_u64(lo: u64, hi: u64) -> Self {
        GprReg([lo, hi])
    }

    /// Lower 64 bits, unsigned (`UD[0]` view).
    #[inline]
    pub fn ud0(self) -> u64 {
        self.0[0]
    }

    /// Upper 64 bits, unsigned (`UD[1]` view).
    #[inline]
    pub fn ud1(self) -> u64 {
        self.0[1]
    }

    /// Lower 64 bits, signed (`SD[0]` view).
    #[inline]
    pub fn sd0(self) -> i64 {
        self.0[0] as i64
    }

    /// Upper 64 bits, signed (`SD[1]` view).
    #[inline]
    pub fn sd1(self) -> i64 {
        self.0[1] as i64
    }

    /// Lower 32 bits, unsigned (`UL[0]` view).
    #[inline]
    pub fn ul0(self) -> u32 {
        self.0[0] as u32
    }

    /// Upper 32 bits of the lower 64 (`UL[1]` view).
    #[inline]
    pub fn ul1(self) -> u32 {
        (self.0[0] >> 32) as u32
    }

    /// Lower 32 bits of the lower 64, third slot (`UL[2]` view).
    #[inline]
    pub fn ul2(self) -> u32 {
        self.0[1] as u32
    }

    /// Upper 32 bits of the upper 64 (`UL[3]` view).
    #[inline]
    pub fn ul3(self) -> u32 {
        (self.0[1] >> 32) as u32
    }

    /// `UL[n]` view for `n` in 0..4. The C++ unions index `UL` four times
    /// across two 64-bit words; this method preserves that mapping.
    #[inline]
    pub fn ul(self, n: usize) -> u32 {
        match n & 3 {
            0 => self.ul0(),
            1 => self.ul1(),
            2 => self.ul2(),
            _ => self.ul3(),
        }
    }

    /// `SL[n]` — signed 32-bit slot, mirroring `UL[n]` indexing.
    #[inline]
    pub fn sl(self, n: usize) -> i32 {
        self.ul(n) as i32
    }

    /// `UD[n]` — unsigned 64-bit slot. `n == 0` reads the lower half.
    #[inline]
    pub fn ud(self, n: usize) -> u64 {
        self.0[n & 1]
    }

    /// `SD[n]` — signed 64-bit slot, mirroring `UD[n]` indexing.
    #[inline]
    pub fn sd(self, n: usize) -> i64 {
        self.ud(n) as i64
    }

    /// `US[n]` — unsigned 16-bit slot. Eight slots span the two 64-bit halves.
    #[inline]
    pub fn us(self, n: usize) -> u16 {
        ((self.0[(n >> 2) & 1] >> ((n & 3) * 16)) & 0xFFFF) as u16
    }

    /// `SS[n]` — signed 16-bit slot, mirroring `US[n]` indexing.
    #[inline]
    pub fn ss(self, n: usize) -> i16 {
        self.us(n) as i16
    }

    /// `UC[n]` — unsigned 8-bit slot. Sixteen slots span the two 64-bit halves.
    #[inline]
    pub fn uc(self, n: usize) -> u8 {
        ((self.0[(n >> 3) & 1] >> ((n & 7) * 8)) & 0xFF) as u8
    }

    /// `SC[n]` — signed 8-bit slot, mirroring `UC[n]` indexing.
    #[inline]
    pub fn sc(self, n: usize) -> i8 {
        self.uc(n) as i8
    }

    /// Construct a `GprReg` from `UL[n]` slot writes (used in `cpuRegs.GPR.r[i].UL[n] = ...`
    /// patterns).
    #[inline]
    pub fn set_ul(mut self, n: usize, value: u32) -> Self {
        match n & 3 {
            0 => self.0[0] = (self.0[0] & 0xFFFF_FFFF_0000_0000) | (value as u64),
            1 => self.0[0] = (self.0[0] & 0x0000_0000_FFFF_FFFF) | ((value as u64) << 32),
            2 => self.0[1] = (self.0[1] & 0xFFFF_FFFF_0000_0000) | (value as u64),
            _ => self.0[1] = (self.0[1] & 0x0000_0000_FFFF_FFFF) | ((value as u64) << 32),
        }
        self
    }

    /// Construct a `GprReg` from `US[n]` slot writes.
    #[inline]
    pub fn set_us(mut self, n: usize, value: u16) -> Self {
        let shift = (n & 3) * 16;
        let mask = 0xFFFFu64 << shift;
        let half = &mut self.0[(n >> 2) & 1];
        *half = (*half & !mask) | (((value as u64) & 0xFFFF) << shift);
        self
    }

    /// Construct a `GprReg` from `UC[n]` slot writes.
    #[inline]
    pub fn set_uc(mut self, n: usize, value: u8) -> Self {
        let shift = (n & 7) * 8;
        let mask = 0xFFu64 << shift;
        let half = &mut self.0[(n >> 3) & 1];
        *half = (*half & !mask) | (((value as u64) & 0xFF) << shift);
        self
    }

    /// Construct a `GprReg` from `UD[n]` slot writes.
    #[inline]
    pub fn set_ud(mut self, n: usize, value: u64) -> Self {
        self.0[n & 1] = value;
        self
    }
}

// ---------------------------------------------------------------------------
// `Common::CountLeadingSignBits` — mirror of the helper from `common/BitUtils.h`.
// Used by `PLZCW`. Returns the number of leading sign bits (32 for zero).
// ---------------------------------------------------------------------------

/// Number of leading sign bits of a signed 32-bit value, including the sign
/// bit itself. Mirrors `Common::CountLeadingSignBits` from the C++ source.
#[inline]
fn count_leading_sign_bits(n: i32) -> u32 {
    let magnitude = if n < 0 { !n } else { n } as u32;
    if magnitude == 0 {
        32
    } else {
        magnitude.leading_zeros()
    }
}

// ===========================================================================
// Non-MMI instructions that share the MMI opcode class
// ===========================================================================
//
// These are the MADD / MULT / DIV family of operations that have a single
// 32-bit GPR but use the HI / LO registers and "slot 1" duplicates thereof.
// In the original C++ they live alongside the MMI opcodes because they
// share the same encoding space.

// ---------------------------------------------------------------------------
// MADD / MADDU
// ---------------------------------------------------------------------------

/// `MADD` — `HI:LO = HI:LO + (Rs * Rt)`. Slot 0.
pub fn MADD() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rs = _rs(code);
    let rt = _rt(code);
    let rd = _rd(code);
    let hi_val = hi();
    let lo_val = lo();
    let acc = ((hi_val.ud0() as i64) << 32) | (lo_val.ud0() & 0xFFFF_FFFF) as i64;
    let rs_v = unsafe { CoreMain::cpuRegs.GPR.r[rs].sd0() };
    let rt_v = unsafe { CoreMain::cpuRegs.GPR.r[rt].sd0() };
    let temp = acc + rs_v.wrapping_mul(rt_v);
    let new_lo = (temp & 0xFFFF_FFFF) as i32;
    let new_hi = (temp >> 32) as i32;
    set_lo(GprReg::from_u64(new_lo as u64, 0));
    set_hi(GprReg::from_u64(new_hi as u64, 0));
    if rd != 0 {
        set_gpr(rd, GprReg::from_u64(new_lo as u64, 0));
    }
}

/// `MADDU` — unsigned multiply-add into slot 0.
pub fn MADDU() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rs = _rs(code);
    let rt = _rt(code);
    let rd = _rd(code);
    let hi_val = hi();
    let lo_val = lo();
    let acc = ((hi_val.ud0()) << 32) | (lo_val.ud0() & 0xFFFF_FFFF);
    let rs_v = unsafe { CoreMain::cpuRegs.GPR.r[rs].ud0() };
    let rt_v = unsafe { CoreMain::cpuRegs.GPR.r[rt].ud0() };
    let temp = acc.wrapping_add(rs_v.wrapping_mul(rt_v));
    let new_lo = (temp & 0xFFFF_FFFF) as i32;
    let new_hi = (temp >> 32) as i32;
    set_lo(GprReg::from_u64(new_lo as u64, 0));
    set_hi(GprReg::from_u64(new_hi as u64, 0));
    if rd != 0 {
        set_gpr(rd, GprReg::from_u64(new_lo as u64, 0));
    }
}

/// `MADD1` — `HI:LO = HI:LO + (Rs * Rt)` in slot 1.
pub fn MADD1() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rs = _rs(code);
    let rt = _rt(code);
    let rd = _rd(code);
    let hi_val = hi();
    let lo_val = lo();
    let acc = ((hi_val.ud1() as i64) << 32) | (lo_val.ud1() & 0xFFFF_FFFF) as i64;
    let rs_v = unsafe { CoreMain::cpuRegs.GPR.r[rs].sd0() };
    let rt_v = unsafe { CoreMain::cpuRegs.GPR.r[rt].sd0() };
    let temp = acc + rs_v.wrapping_mul(rt_v);
    let new_lo = (temp & 0xFFFF_FFFF) as i32;
    let new_hi = (temp >> 32) as i32;
    set_lo(GprReg::from_u64(lo_val.ud0(), new_lo as u64));
    set_hi(GprReg::from_u64(hi_val.ud0(), new_hi as u64));
    if rd != 0 {
        set_gpr(rd, GprReg::from_u64(new_lo as u64, 0));
    }
}

/// `MADDU1` — unsigned multiply-add into slot 1.
pub fn MADDU1() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rs = _rs(code);
    let rt = _rt(code);
    let rd = _rd(code);
    let hi_val = hi();
    let lo_val = lo();
    let acc = ((hi_val.ud1()) << 32) | (lo_val.ud1() & 0xFFFF_FFFF);
    let rs_v = unsafe { CoreMain::cpuRegs.GPR.r[rs].ud0() };
    let rt_v = unsafe { CoreMain::cpuRegs.GPR.r[rt].ud0() };
    let temp = acc.wrapping_add(rs_v.wrapping_mul(rt_v));
    let new_lo = (temp & 0xFFFF_FFFF) as i32;
    let new_hi = (temp >> 32) as i32;
    set_lo(GprReg::from_u64(lo_val.ud0(), new_lo as u64));
    set_hi(GprReg::from_u64(hi_val.ud0(), new_hi as u64));
    if rd != 0 {
        set_gpr(rd, GprReg::from_u64(new_lo as u64, 0));
    }
}

// ---------------------------------------------------------------------------
// MFHI1 / MFLO1 / MTHI1 / MTLO1 — slot-1 HI/LO access
// ---------------------------------------------------------------------------

/// `MFHI1` — copy `HI.UD[1]` to GPR `_Rd_`.
pub fn MFHI1() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    set_gpr(rd, GprReg::from_u64(hi().ud1(), 0));
}

/// `MFLO1` — copy `LO.UD[1]` to GPR `_Rd_`.
pub fn MFLO1() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    set_gpr(rd, GprReg::from_u64(lo().ud1(), 0));
}

/// `MTHI1` — write `_Rs_.UD[0]` into `HI.UD[1]`.
pub fn MTHI1() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rs = _rs(code);
    set_hi(GprReg::from_u64(hi().ud0(), gpr(rs).ud0()));
}

/// `MTLO1` — write `_Rs_.UD[0]` into `LO.UD[1]`.
pub fn MTLO1() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rs = _rs(code);
    set_lo(GprReg::from_u64(lo().ud0(), gpr(rs).ud0()));
}

// ---------------------------------------------------------------------------
// MULT1 / MULTU1 / DIV1 / DIVU1 — slot-1 HI/LO arithmetic
// ---------------------------------------------------------------------------

/// `MULT1` — signed 32x32->64 multiply, result in slot 1.
pub fn MULT1() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rs = _rs(code);
    let rt = _rt(code);
    let rd = _rd(code);
    let rs_v = unsafe { CoreMain::cpuRegs.GPR.r[rs].sl(0) };
    let rt_v = unsafe { CoreMain::cpuRegs.GPR.r[rt].sl(0) };
    let temp = (rs_v as i64) * (rt_v as i64);
    let new_lo = (temp & 0xFFFF_FFFF) as i32;
    let new_hi = (temp >> 32) as i32;
    let prev_lo = lo();
    let prev_hi = hi();
    set_lo(GprReg::from_u64(prev_lo.ud0(), new_lo as u64));
    set_hi(GprReg::from_u64(prev_hi.ud0(), new_hi as u64));
    if rd != 0 {
        set_gpr(rd, GprReg::from_u64(new_lo as u64, 0));
    }
}

/// `MULTU1` — unsigned 32x32->64 multiply, result in slot 1 (sign-extended
/// in the LO/HI writes per the EE spec).
pub fn MULTU1() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rs = _rs(code);
    let rt = _rt(code);
    let rd = _rd(code);
    let rs_v = unsafe { CoreMain::cpuRegs.GPR.r[rs].ul(0) };
    let rt_v = unsafe { CoreMain::cpuRegs.GPR.r[rt].ul(0) };
    let temp = (rs_v as u64) * (rt_v as u64);
    let new_lo = (temp & 0xFFFF_FFFF) as i32;
    let new_hi = (temp >> 32) as i32;
    let prev_lo = lo();
    let prev_hi = hi();
    set_lo(GprReg::from_u64(prev_lo.ud0(), new_lo as u64));
    set_hi(GprReg::from_u64(prev_hi.ud0(), new_hi as u64));
    if rd != 0 {
        set_gpr(rd, GprReg::from_u64(new_lo as u64, 0));
    }
}

/// `DIV1` — signed 32-bit divide, result in slot 1.
pub fn DIV1() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rs = _rs(code);
    let rt = _rt(code);
    let rs_v = unsafe { CoreMain::cpuRegs.GPR.r[rs].ul(0) };
    let rt_v = unsafe { CoreMain::cpuRegs.GPR.r[rt].ul(0) };
    let (new_lo, new_hi) = if rs_v == 0x8000_0000 && rt_v == 0xFFFF_FFFF {
        (0x8000_0000i32, 0i32)
    } else if (rt_v as i32) != 0 {
        let r = rs_v as i32;
        let d = rt_v as i32;
        (r / d, r % d)
    } else {
        let r = rs_v as i32;
        let lo = if r < 0 { 1i32 } else { -1i32 };
        (lo, r)
    };
    let prev_lo = lo();
    let prev_hi = hi();
    set_lo(GprReg::from_u64(prev_lo.ud0(), new_lo as u64));
    set_hi(GprReg::from_u64(prev_hi.ud0(), new_hi as u64));
}

/// `DIVU1` — unsigned 32-bit divide, result in slot 1.
pub fn DIVU1() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rs = _rs(code);
    let rt = _rt(code);
    let rs_v = unsafe { CoreMain::cpuRegs.GPR.r[rs].ul(0) };
    let rt_v = unsafe { CoreMain::cpuRegs.GPR.r[rt].ul(0) };
    let (new_lo, new_hi) = if rt_v != 0 {
        (
            (rs_v / rt_v) as i32,
            (rs_v % rt_v) as i32,
        )
    } else {
        (-1i32, rs_v as i32)
    };
    let prev_lo = lo();
    let prev_hi = hi();
    set_lo(GprReg::from_u64(prev_lo.ud0(), new_lo as u64));
    set_hi(GprReg::from_u64(prev_hi.ud0(), new_hi as u64));
}

// ===========================================================================
// MMI opcodes — base group (PLZCW / PMFHL / PMTHL / parallel shifts)
// ===========================================================================

/// `PLZCW` — pack leading-zero/leading-sign count words into `_Rd_`.
pub fn PLZCW() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    let rs = _rs(code);
    if rd == 0 {
        return;
    }
    let src = unsafe { CoreMain::cpuRegs.GPR.r[rs] };
    let mut out = GprReg::from_u64(0, 0);
    out = out.set_ul(
        0,
        count_leading_sign_bits(src.sl(0)).saturating_sub(1),
    );
    out = out.set_ul(
        1,
        count_leading_sign_bits(src.sl(1)).saturating_sub(1),
    );
    set_gpr(rd, out);
}

/// Clamp helper for `PMFHL` SH case.
#[inline]
fn pmfhl_clamp(dst: &mut u16, src: i32) {
    if src > 0x7FFF {
        *dst = 0x7FFF;
    } else if src < -0x8000 {
        *dst = 0x8000;
    } else {
        *dst = src as u16;
    }
}

/// `PMFHL` — pack from HI/LO. The `_Sa_` field selects which packing mode.
pub fn PMFHL() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let sa = _sa(code);
    let hi_v = hi();
    let lo_v = lo();
    let mut out = GprReg::from_u64(0, 0);
    match sa {
        0x00 => {
            // LW: pack 4x 32-bit values from LO.UL[0..1] and HI.UL[0..1].
            out = out.set_ul(0, lo_v.ul0());
            out = out.set_ul(1, hi_v.ul0());
            out = out.set_ul(2, lo_v.ul2());
            out = out.set_ul(3, hi_v.ul2());
        }
        0x01 => {
            // UW: pack from LO.UL[1] / HI.UL[1] / LO.UL[3] / HI.UL[3].
            out = out.set_ul(0, lo_v.ul1());
            out = out.set_ul(1, hi_v.ul1());
            out = out.set_ul(2, lo_v.ul3());
            out = out.set_ul(3, hi_v.ul3());
        }
        0x02 => {
            // SLW: signed 64-bit pack, with 31-bit saturation.
            let lo_0 = ((hi_v.ud0() << 32) | lo_v.ud0()) as i64;
            out = out.set_ud(
                0,
                if lo_0 >= 0x7FFF_FFFF {
                    0x7FFF_FFFFu64
                } else if lo_0 <= -0x8000_0000i64 {
                    0xFFFF_FFFF_8000_0000u64
                } else {
                    lo_v.ud0() & 0xFFFF_FFFF
                },
            );
            let lo_2 = ((hi_v.ud1() << 32) | lo_v.ud1()) as i64;
            out = out.set_ud(
                1,
                if lo_2 >= 0x7FFF_FFFF {
                    0x7FFF_FFFFu64
                } else if lo_2 <= -0x8000_0000i64 {
                    0xFFFF_FFFF_8000_0000u64
                } else {
                    lo_v.ud1() & 0xFFFF_FFFF
                },
            );
        }
        0x03 => {
            // LH: pack 8x 16-bit values.
            out = out.set_us(0, lo_v.us(0));
            out = out.set_us(1, lo_v.us(2));
            out = out.set_us(2, hi_v.us(0));
            out = out.set_us(3, hi_v.us(2));
            out = out.set_us(4, lo_v.us(4));
            out = out.set_us(5, lo_v.us(6));
            out = out.set_us(6, hi_v.us(4));
            out = out.set_us(7, hi_v.us(6));
        }
        0x04 => {
            // SH: signed-saturating 16-bit pack.
            let mut s0: u16 = 0;
            let mut s1: u16 = 0;
            let mut s2: u16 = 0;
            let mut s3: u16 = 0;
            let mut s4: u16 = 0;
            let mut s5: u16 = 0;
            let mut s6: u16 = 0;
            let mut s7: u16 = 0;
            pmfhl_clamp(&mut s0, lo_v.ul0() as i32);
            pmfhl_clamp(&mut s1, lo_v.ul1() as i32);
            pmfhl_clamp(&mut s2, hi_v.ul0() as i32);
            pmfhl_clamp(&mut s3, hi_v.ul1() as i32);
            pmfhl_clamp(&mut s4, lo_v.ul2() as i32);
            pmfhl_clamp(&mut s5, lo_v.ul3() as i32);
            pmfhl_clamp(&mut s6, hi_v.ul2() as i32);
            pmfhl_clamp(&mut s7, hi_v.ul3() as i32);
            out = out.set_us(0, s0);
            out = out.set_us(1, s1);
            out = out.set_us(2, s2);
            out = out.set_us(3, s3);
            out = out.set_us(4, s4);
            out = out.set_us(5, s5);
            out = out.set_us(6, s6);
            out = out.set_us(7, s7);
        }
        _ => {}
    }
    set_gpr(rd, out);
}

/// `PMTHL` — load LO/HI from `_Rs_`. Only valid for `_Sa_ == 0`.
pub fn PMTHL() {
    let code = unsafe { CoreMain::cpuRegs.code };
    if _sa(code) != 0 {
        return;
    }
    let rs = _rs(code);
    let src = gpr(rs);
    let hi_v = hi();
    let lo_v = lo();
    set_lo(GprReg::from_u64(src.ud0(), lo_v.ud1()));
    set_hi(GprReg::from_u64(src.ud1(), hi_v.ud1()));
}

// ---------------------------------------------------------------------------
// Parallel shifts — PSLLH / PSRLH / PSRAH (16-bit lanes), PSLLW / PSRLW /
// PSRAW (32-bit lanes). The original C++ uses inline static helpers
// `_PSLLH(n)` / `_PSRLH(n)` / `_PSRAW(n)`; we expose them as `#[inline]`
// free functions and dispatch from the outer opcode.
// ---------------------------------------------------------------------------

#[inline]
fn psllh_slot(n: usize, sa: u32) -> u32 {
    let rt = unsafe { CoreMain::cpuRegs.GPR.r[_rt(CoreMain::cpuRegs.code)] };
    (rt.us(n) as u32) << (sa & 0xF)
}

#[inline]
fn psrlh_slot(n: usize, sa: u32) -> u32 {
    let rt = unsafe { CoreMain::cpuRegs.GPR.r[_rt(CoreMain::cpuRegs.code)] };
    ((rt.us(n) as u32) >> (sa & 0xF)) & 0xFFFF
}

#[inline]
fn psrah_slot(n: usize, sa: u32) -> u32 {
    let rt = unsafe { CoreMain::cpuRegs.GPR.r[_rt(CoreMain::cpuRegs.code)] };
    ((rt.ss(n) as i32) >> (sa & 0xF)) as u16 as u32
}

#[inline]
fn psllw_slot(n: usize, sa: u32) -> u32 {
    let rt = unsafe { CoreMain::cpuRegs.GPR.r[_rt(CoreMain::cpuRegs.code)] };
    rt.ul(n) << sa
}

#[inline]
fn psrlw_slot(n: usize, sa: u32) -> u32 {
    let rt = unsafe { CoreMain::cpuRegs.GPR.r[_rt(CoreMain::cpuRegs.code)] };
    rt.ul(n) >> sa
}

#[inline]
fn psraw_slot(n: usize, sa: u32) -> u32 {
    let rt = unsafe { CoreMain::cpuRegs.GPR.r[_rt(CoreMain::cpuRegs.code)] };
    (rt.sl(n) >> sa) as u32
}

/// `PSLLH` — parallel shift-left of 8x 16-bit lanes by `_Sa_ & 0xf`.
pub fn PSLLH() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let sa = _sa(code);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..8 {
        out = out.set_us(n, psllh_slot(n, sa) as u16);
    }
    set_gpr(rd, out);
}

/// `PSRLH` — parallel logical shift-right of 8x 16-bit lanes by `_Sa_ & 0xf`.
pub fn PSRLH() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let sa = _sa(code);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..8 {
        out = out.set_us(n, psrlh_slot(n, sa) as u16);
    }
    set_gpr(rd, out);
}

/// `PSRAH` — parallel arithmetic shift-right of 8x 16-bit lanes by `_Sa_ & 0xf`.
pub fn PSRAH() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let sa = _sa(code);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..8 {
        out = out.set_us(n, psrah_slot(n, sa) as u16);
    }
    set_gpr(rd, out);
}

/// `PSLLW` — parallel shift-left of 4x 32-bit lanes by `_Sa_`.
pub fn PSLLW() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let sa = _sa(code);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..4 {
        out = out.set_ul(n, psllw_slot(n, sa));
    }
    set_gpr(rd, out);
}

/// `PSRLW` — parallel logical shift-right of 4x 32-bit lanes by `_Sa_`.
pub fn PSRLW() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let sa = _sa(code);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..4 {
        out = out.set_ul(n, psrlw_slot(n, sa));
    }
    set_gpr(rd, out);
}

/// `PSRAW` — parallel arithmetic shift-right of 4x 32-bit lanes by `_Sa_`.
pub fn PSRAW() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let sa = _sa(code);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..4 {
        out = out.set_ul(n, psraw_slot(n, sa));
    }
    set_gpr(rd, out);
}

// ===========================================================================
// MMI0 opcodes — parallel add / sub / cmp / minmax / packs / saturating arith
// ===========================================================================

// ---------------------------------------------------------------------------
// PADDW / PSUBW (32-bit lanes)
// ---------------------------------------------------------------------------

/// `PADDW` — parallel add of 4x 32-bit lanes.
pub fn PADDW() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..4 {
        out = out.set_ul(n, src.ul(n).wrapping_add(tgt.ul(n)));
    }
    set_gpr(rd, out);
}

/// `PSUBW` — parallel sub of 4x 32-bit lanes.
pub fn PSUBW() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..4 {
        out = out.set_ul(n, src.ul(n).wrapping_sub(tgt.ul(n)));
    }
    set_gpr(rd, out);
}

/// `PCGTW` — parallel compare-greater-than (signed) of 4x 32-bit lanes.
pub fn PCGTW() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..4 {
        out = out.set_ul(n, if src.sl(n) > tgt.sl(n) { 0xFFFF_FFFF } else { 0 });
    }
    set_gpr(rd, out);
}

/// `PMAXW` — parallel max of 4x 32-bit (signed) lanes.
pub fn PMAXW() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..4 {
        out = out.set_ul(n, if src.sl(n) > tgt.sl(n) { src.ul(n) } else { tgt.ul(n) });
    }
    set_gpr(rd, out);
}

// ---------------------------------------------------------------------------
// PADDH / PSUBH / PCGTH / PMAXH (16-bit lanes)
// ---------------------------------------------------------------------------

/// `PADDH` — parallel add of 8x 16-bit lanes.
pub fn PADDH() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..8 {
        out = out.set_us(
            n,
            (src.us(n) as u16).wrapping_add(tgt.us(n)),
        );
    }
    set_gpr(rd, out);
}

/// `PSUBH` — parallel sub of 8x 16-bit lanes.
pub fn PSUBH() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..8 {
        out = out.set_us(
            n,
            (src.us(n) as u16).wrapping_sub(tgt.us(n)),
        );
    }
    set_gpr(rd, out);
}

/// `PCGTH` — parallel compare-greater-than (signed) of 8x 16-bit lanes.
pub fn PCGTH() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..8 {
        out = out.set_us(n, if src.ss(n) > tgt.ss(n) { 0xFFFF } else { 0 });
    }
    set_gpr(rd, out);
}

/// `PMAXH` — parallel max of 8x 16-bit (signed) lanes.
pub fn PMAXH() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..8 {
        out = out.set_us(n, if src.ss(n) > tgt.ss(n) { src.us(n) } else { tgt.us(n) });
    }
    set_gpr(rd, out);
}

// ---------------------------------------------------------------------------
// PADDB / PSUBB / PCGTB (8-bit lanes)
// ---------------------------------------------------------------------------

/// `PADDB` — parallel add of 16x 8-bit lanes.
pub fn PADDB() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..16 {
        out = out.set_uc(
            n,
            (src.uc(n) as u8).wrapping_add(tgt.uc(n)),
        );
    }
    set_gpr(rd, out);
}

/// `PSUBB` — parallel sub of 16x 8-bit lanes.
pub fn PSUBB() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..16 {
        out = out.set_uc(
            n,
            (src.uc(n) as u8).wrapping_sub(tgt.uc(n)),
        );
    }
    set_gpr(rd, out);
}

/// `PCGTB` — parallel compare-greater-than (signed) of 16x 8-bit lanes.
pub fn PCGTB() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..16 {
        out = out.set_uc(n, if src.sc(n) > tgt.sc(n) { 0xFF } else { 0 });
    }
    set_gpr(rd, out);
}

// ---------------------------------------------------------------------------
// PADDSW / PSUBSW (signed-saturating 32-bit lanes)
// ---------------------------------------------------------------------------

/// `PADDSW` — parallel signed-saturating add of 4x 32-bit lanes.
pub fn PADDSW() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..4 {
        let s = (src.sl(n) as i64) + (tgt.sl(n) as i64);
        let v = if s > 0x7FFF_FFFF {
            0x7FFF_FFFFu32
        } else if s < -0x8000_0000i64 {
            0x8000_0000u32
        } else {
            s as i32 as u32
        };
        out = out.set_ul(n, v);
    }
    set_gpr(rd, out);
}

/// `PSUBSW` — parallel signed-saturating sub of 4x 32-bit lanes.
pub fn PSUBSW() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..4 {
        let s = (src.sl(n) as i64) - (tgt.sl(n) as i64);
        let v = if s >= 0x7FFF_FFFF {
            0x7FFF_FFFFu32
        } else if s < -0x8000_0000i64 {
            0x8000_0000u32
        } else {
            s as i32 as u32
        };
        out = out.set_ul(n, v);
    }
    set_gpr(rd, out);
}

// ---------------------------------------------------------------------------
// PEXTLW / PPACW — 32-bit interleave and pack
// ---------------------------------------------------------------------------

/// `PEXTLW` — pack from upper halves: `{Rt.UL[1], Rs.UL[1], Rt.UL[3], Rs.UL[3]}`.
pub fn PEXTLW() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    out = out.set_ul(0, tgt.ul(1));
    out = out.set_ul(1, src.ul(1));
    out = out.set_ul(2, tgt.ul(3));
    out = out.set_ul(3, src.ul(3));
    set_gpr(rd, out);
}

/// `PPACW` — pack: `{Rt.UL[0], Rt.UL[2], Rs.UL[0], Rs.UL[2]}`.
pub fn PPACW() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    out = out.set_ul(0, tgt.ul(0));
    out = out.set_ul(1, tgt.ul(2));
    out = out.set_ul(2, src.ul(0));
    out = out.set_ul(3, src.ul(2));
    set_gpr(rd, out);
}

// ---------------------------------------------------------------------------
// PADDSH / PSUBSH (signed-saturating 16-bit lanes)
// ---------------------------------------------------------------------------

/// `PADDSH` — parallel signed-saturating add of 8x 16-bit lanes.
pub fn PADDSH() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..8 {
        let s = (src.ss(n) as i32) + (tgt.ss(n) as i32);
        let v = if s > 0x7FFF {
            0x7FFFu16
        } else if s < -0x8000 {
            0x8000u16
        } else {
            s as i16 as u16
        };
        out = out.set_us(n, v);
    }
    set_gpr(rd, out);
}

/// `PSUBSH` — parallel signed-saturating sub of 8x 16-bit lanes.
pub fn PSUBSH() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..8 {
        let s = (src.ss(n) as i32) - (tgt.ss(n) as i32);
        let v = if s >= 0x7FFF {
            0x7FFFu16
        } else if s < -0x8000 {
            0x8000u16
        } else {
            s as i16 as u16
        };
        out = out.set_us(n, v);
    }
    set_gpr(rd, out);
}

// ---------------------------------------------------------------------------
// PEXTLH / PPACH — 16-bit interleave and pack
// ---------------------------------------------------------------------------

/// `PEXTLH` — interleave upper 16-bit halves: `{Rt.US[2], Rs.US[2], ...}`.
pub fn PEXTLH() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    out = out.set_us(0, tgt.us(2));
    out = out.set_us(1, src.us(2));
    out = out.set_us(2, tgt.us(3));
    out = out.set_us(3, src.us(3));
    out = out.set_us(4, tgt.us(6));
    out = out.set_us(5, src.us(6));
    out = out.set_us(6, tgt.us(7));
    out = out.set_us(7, src.us(7));
    set_gpr(rd, out);
}

/// `PPACH` — pack upper 16-bit halves.
pub fn PPACH() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    out = out.set_us(0, tgt.us(2));
    out = out.set_us(1, tgt.us(4));
    out = out.set_us(2, tgt.us(6));
    out = out.set_us(3, tgt.us(0) /* placeholder - see note */);
    out = out.set_us(4, src.us(2));
    out = out.set_us(5, src.us(4));
    out = out.set_us(6, src.us(6));
    out = out.set_us(7, src.us(0) /* placeholder - see note */);
    let _ = out;
    // PPACH from the C++ source uses indices `2,4,6,0` (i.e. odd/even lanes
    // interleaved). Reapply using the exact mapping the C++ uses.
    let mut out2 = GprReg::from_u64(0, 0);
    out2 = out2.set_us(0, tgt.us(2));
    out2 = out2.set_us(1, tgt.us(4));
    out2 = out2.set_us(2, tgt.us(6));
    out2 = out2.set_us(3, tgt.us(0));
    out2 = out2.set_us(4, src.us(2));
    out2 = out2.set_us(5, src.us(4));
    out2 = out2.set_us(6, src.us(6));
    out2 = out2.set_us(7, src.us(0));
    set_gpr(rd, out2);
}

// ---------------------------------------------------------------------------
// PADDSB / PSUBSB (signed-saturating 8-bit lanes)
// ---------------------------------------------------------------------------

/// `PADDSB` — parallel signed-saturating add of 16x 8-bit lanes.
pub fn PADDSB() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..16 {
        let s = (src.sc(n) as i16) + (tgt.sc(n) as i16);
        let v = if s > 0x7F {
            0x7Fu8
        } else if s < -0x80 {
            0x80u8
        } else {
            s as i8 as u8
        };
        out = out.set_uc(n, v);
    }
    set_gpr(rd, out);
}

/// `PSUBSB` — parallel signed-saturating sub of 16x 8-bit lanes.
pub fn PSUBSB() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..16 {
        let s = (src.sc(n) as i16) - (tgt.sc(n) as i16);
        let v = if s >= 0x7F {
            0x7Fu8
        } else if s < -0x80 {
            0x80u8
        } else {
            s as i8 as u8
        };
        out = out.set_uc(n, v);
    }
    set_gpr(rd, out);
}

// ---------------------------------------------------------------------------
// PEXTLB / PPACB — 8-bit interleave and pack
// ---------------------------------------------------------------------------

/// `PEXTLB` — interleave upper bytes: pairs of even/odd lanes.
pub fn PEXTLB() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let pairs_rt = [4usize, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
    let pairs_rs = [4usize, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
    let mut out = GprReg::from_u64(0, 0);
    out = out.set_uc(0, tgt.uc(4));
    out = out.set_uc(1, src.uc(4));
    for n in 0..12 {
        out = out.set_uc(2 + n, if n & 1 == 0 {
            tgt.uc(pairs_rt[n])
        } else {
            src.uc(pairs_rs[n])
        });
    }
    set_gpr(rd, out);
}

/// `PPACB` — pack upper bytes: even lanes from Rt then Rs.
pub fn PPACB() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    out = out.set_uc(0, tgt.uc(4));
    out = out.set_uc(1, tgt.uc(6));
    out = out.set_uc(2, tgt.uc(8));
    out = out.set_uc(3, tgt.uc(10));
    out = out.set_uc(4, tgt.uc(12));
    out = out.set_uc(5, tgt.uc(14));
    out = out.set_uc(6, tgt.uc(0));
    out = out.set_uc(7, tgt.uc(2));
    out = out.set_uc(8, src.uc(4));
    out = out.set_uc(9, src.uc(6));
    out = out.set_uc(10, src.uc(8));
    out = out.set_uc(11, src.uc(10));
    out = out.set_uc(12, src.uc(12));
    out = out.set_uc(13, src.uc(14));
    out = out.set_uc(14, src.uc(0));
    out = out.set_uc(15, src.uc(2));
    set_gpr(rd, out);
}

// ---------------------------------------------------------------------------
// PEXT5 / PPAC5 — 5-bit field extraction and packing (per 32-bit lane)
// ---------------------------------------------------------------------------

/// `PEXT5` — extract 5-bit fields from 4x 32-bit lanes (used by the EE
/// vector unit's 5-bit colour pipeline).
pub fn PEXT5() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rt = _rt(code);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..4 {
        let v = tgt.ul(n);
        let bits = ((v & 0x0000_001F) << 3)
            | ((v & 0x0000_03E0) << 6)
            | ((v & 0x0000_7C00) << 9)
            | ((v & 0x0000_8000) << 16);
        out = out.set_ul(n, bits);
    }
    set_gpr(rd, out);
}

/// `PPAC5` — inverse of `PEXT5`.
pub fn PPAC5() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rt = _rt(code);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..4 {
        let v = tgt.ul(n);
        let bits = ((v >> 3) & 0x0000_001F)
            | ((v >> 6) & 0x0000_03E0)
            | ((v >> 9) & 0x0000_7C00)
            | ((v >> 16) & 0x0000_8000);
        out = out.set_ul(n, bits);
    }
    set_gpr(rd, out);
}

// ===========================================================================
// MMI1 opcodes
// ===========================================================================

/// `PABSW` — parallel absolute value of 4x 32-bit lanes (with 0x80000000
/// saturation).
pub fn PABSW() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rt = _rt(code);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..4 {
        let v = if tgt.ul(n) == 0x8000_0000 {
            0x7FFF_FFFFu32
        } else if tgt.sl(n) < 0 {
            (tgt.sl(n).wrapping_neg()) as u32
        } else {
            tgt.ul(n)
        };
        out = out.set_ul(n, v);
    }
    set_gpr(rd, out);
}

/// `PCEQW` — parallel compare-equal of 4x 32-bit lanes.
pub fn PCEQW() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..4 {
        out = out.set_ul(n, if src.ul(n) == tgt.ul(n) { 0xFFFF_FFFF } else { 0 });
    }
    set_gpr(rd, out);
}

/// `PMINW` — parallel min of 4x 32-bit (signed) lanes.
pub fn PMINW() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..4 {
        out = out.set_ul(n, if src.sl(n) < tgt.sl(n) { src.ul(n) } else { tgt.ul(n) });
    }
    set_gpr(rd, out);
}

/// `PADSBH` — sub 16-bit lanes 0..3, add lanes 4..7 (the EE's
/// "subtract-and-add by halves" op).
pub fn PADSBH() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..4 {
        out = out.set_us(
            n,
            (src.us(n) as u16).wrapping_sub(tgt.us(n)),
        );
    }
    for n in 4..8 {
        out = out.set_us(
            n,
            (src.us(n) as u16).wrapping_add(tgt.us(n)),
        );
    }
    set_gpr(rd, out);
}

/// `PABSH` — parallel absolute value of 8x 16-bit lanes (with 0x8000
/// saturation).
pub fn PABSH() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rt = _rt(code);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..8 {
        let v = if tgt.us(n) == 0x8000 {
            0x7FFFu16
        } else if tgt.ss(n) < 0 {
            (tgt.ss(n).wrapping_neg()) as u16
        } else {
            tgt.us(n)
        };
        out = out.set_us(n, v);
    }
    set_gpr(rd, out);
}

/// `PCEQH` — parallel compare-equal of 8x 16-bit lanes.
pub fn PCEQH() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..8 {
        out = out.set_us(n, if src.us(n) == tgt.us(n) { 0xFFFF } else { 0 });
    }
    set_gpr(rd, out);
}

/// `PMINH` — parallel min of 8x 16-bit (signed) lanes.
pub fn PMINH() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..8 {
        out = out.set_us(n, if src.ss(n) < tgt.ss(n) { src.us(n) } else { tgt.us(n) });
    }
    set_gpr(rd, out);
}

/// `PCEQB` — parallel compare-equal of 16x 8-bit lanes.
pub fn PCEQB() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..16 {
        out = out.set_uc(n, if src.uc(n) == tgt.uc(n) { 0xFF } else { 0 });
    }
    set_gpr(rd, out);
}

/// `PADDUW` — parallel unsigned-saturating add of 4x 32-bit lanes.
pub fn PADDUW() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..4 {
        let s = (src.ul(n) as u64) + (tgt.ul(n) as u64);
        out = out.set_ul(n, if s > 0xFFFF_FFFF { 0xFFFF_FFFF } else { s as u32 });
    }
    set_gpr(rd, out);
}

/// `PSUBUW` — parallel unsigned-saturating sub of 4x 32-bit lanes.
pub fn PSUBUW() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..4 {
        let s = (src.ul(n) as i64) - (tgt.ul(n) as i64);
        out = out.set_ul(n, if s <= 0 { 0 } else { s as u32 });
    }
    set_gpr(rd, out);
}

/// `PEXTUW` — pack from upper 32-bit halves.
pub fn PEXTUW() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    out = out.set_ul(0, tgt.ul(2));
    out = out.set_ul(1, src.ul(2));
    out = out.set_ul(2, tgt.ul(3));
    out = out.set_ul(3, src.ul(3));
    set_gpr(rd, out);
}

/// `PADDUH` — parallel unsigned-saturating add of 8x 16-bit lanes.
pub fn PADDUH() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..8 {
        let s = (src.us(n) as u32) + (tgt.us(n) as u32);
        out = out.set_us(n, if s > 0xFFFF { 0xFFFF } else { s as u16 });
    }
    set_gpr(rd, out);
}

/// `PSUBUH` — parallel unsigned-saturating sub of 8x 16-bit lanes.
pub fn PSUBUH() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..8 {
        let s = (src.us(n) as i32) - (tgt.us(n) as i32);
        out = out.set_us(n, if s <= 0 { 0 } else { s as u16 });
    }
    set_gpr(rd, out);
}

/// `PEXTUH` — pack from upper 16-bit halves.
pub fn PEXTUH() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    out = out.set_us(0, tgt.us(4));
    out = out.set_us(1, src.us(4));
    out = out.set_us(2, tgt.us(5));
    out = out.set_us(3, src.us(5));
    out = out.set_us(4, tgt.us(6));
    out = out.set_us(5, src.us(6));
    out = out.set_us(6, tgt.us(7));
    out = out.set_us(7, src.us(7));
    set_gpr(rd, out);
}

/// `PADDUB` — parallel unsigned-saturating add of 16x 8-bit lanes.
pub fn PADDUB() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..16 {
        let s = (src.uc(n) as u16) + (tgt.uc(n) as u16);
        out = out.set_uc(n, if s > 0xFF { 0xFF } else { s as u8 });
    }
    set_gpr(rd, out);
}

/// `PSUBUB` — parallel unsigned-saturating sub of 16x 8-bit lanes.
pub fn PSUBUB() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    for n in 0..16 {
        let s = (src.uc(n) as i16) - (tgt.uc(n) as i16);
        out = out.set_uc(n, if s <= 0 { 0 } else { s as u8 });
    }
    set_gpr(rd, out);
}

/// `PEXTUB` — pack from upper 8-bit halves.
pub fn PEXTUB() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    let rt_idx = [8usize, 9, 10, 11, 12, 13, 14, 15];
    let rs_idx = [8usize, 9, 10, 11, 12, 13, 14, 15];
    for n in 0..8 {
        out = out.set_uc(2 * n, tgt.uc(rt_idx[n]));
        out = out.set_uc(2 * n + 1, src.uc(rs_idx[n]));
    }
    set_gpr(rd, out);
}

/// `QFSRV` — quad-word fixed-shift right variable: `Rd = Rt >>> (Rs << 3)`.
pub fn QFSRV() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rt = _rt(code);
    let rs = _rs(code);
    let sa = unsafe { CoreMain::cpuRegs.sa };
    let sa_amt = sa.wrapping_mul(8);
    if sa_amt == 0 {
        // Direct copy of Rt into Rd.
        let tgt = gpr(rt);
        set_gpr(rd, tgt);
        return;
    }
    let src_t = gpr(rt);
    let src_s = gpr(rs);
    let rt_lo = src_t.ud0();
    let rt_hi = src_t.ud1();
    let rs_lo = src_s.ud0();
    let rs_hi = src_s.ud1();
    let (new_lo, new_hi) = if sa_amt < 64 {
        (
            (rt_lo >> sa_amt) | (rt_hi << (64 - sa_amt)),
            (rt_hi >> sa_amt) | (rs_lo << (64 - sa_amt)),
        )
    } else if sa_amt == 64 {
        // 64-bit shift is equivalent to a 0-bit shift on a 6-bit shift mask.
        (
            rt_hi,
            rs_lo,
        )
    } else {
        let sub = sa_amt - 64;
        (
            (rt_hi >> sub) | (rs_lo << (64 - sub)),
            (rs_lo >> sub) | (rs_hi << (64 - sub)),
        )
    };
    set_gpr(rd, GprReg::from_u64(new_lo, new_hi));
}

// ===========================================================================
// MMI2 opcodes — 32-bit SIMD arithmetic + multiplies
// ===========================================================================

/// Internal helper for `PMADDW` / `PMSUBW`. Multiplies-and-adds the
/// `ss`-th lane of `Rs`/`Rt`, accumulates against the `ss`-th slot of HI,
/// and writes back the LO/HI result and (optionally) the GPR destination.
#[inline]
fn pmaddw_slot(dd: usize, ss: usize, neg: bool) {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let a = src.sl(ss) as i64;
    let b = tgt.sl(ss) as i64;
    let temp = a.wrapping_mul(b);
    // PS2 multiplication error emulation: when (a & 0x7FFFFFFF) is 0 or 0x7FFFFFFF
    // and a != b, add 0x70000000 (only on slot 0).
    let mut temp2 = temp + ((hi().sl(ss * 2) as i64) << 32);
    if ss == 0 {
        let av = a as i32;
        let bv = b as i32;
        if ((av & 0x7FFF_FFFF) == 0 || (av & 0x7FFF_FFFF) == 0x7FFF_FFFF) && av != bv {
            temp2 = temp2.wrapping_add(0x7000_0000);
        }
    }
    if neg {
        temp2 = (((hi().sl(ss * 2) as i64) << 32) - temp) + ((hi().sl(ss * 2) as i64) << 32) - temp2;
        let _ = temp2;
        // Recompute correctly: `temp2 = (HI << 32) - temp`, then divided-by-2^32-1.
        temp2 = ((hi().sl(ss * 2) as i64) << 32) - temp;
    }
    temp2 /= 4_294_967_295; // matches `(s32)(temp2 / 4294967295)` from C++.
    let new_lo = ((temp & 0xFFFF_FFFF) as i32)
        .wrapping_add(if neg {
            -(tgt.sl(ss)) /* placeholder */
        } else {
            0
        });
    let _ = new_lo;
    let new_lo = if neg {
        lo().sl(ss * 2).wrapping_sub((temp & 0xFFFF_FFFF) as i32)
    } else {
        lo().sl(ss * 2).wrapping_add((temp & 0xFFFF_FFFF) as i32)
    };
    let new_hi = temp2 as i32;
    let prev_lo = lo();
    let prev_hi = hi();
    if neg {
        set_lo(GprReg::from_u64(
            if dd * 2 == 0 {
                new_lo as u64
            } else {
                prev_lo.ud0()
            },
            if dd * 2 == 2 {
                new_lo as u64
            } else {
                prev_lo.ud1()
            },
        ));
        set_hi(GprReg::from_u64(
            if dd * 2 == 0 {
                new_hi as u64
            } else {
                prev_hi.ud0()
            },
            if dd * 2 == 2 {
                new_hi as u64
            } else {
                prev_hi.ud1()
            },
        ));
    } else {
        set_lo(GprReg::from_u64(
            if dd * 2 == 0 {
                new_lo as u64
            } else {
                prev_lo.ud0()
            },
            if dd * 2 == 2 {
                new_lo as u64
            } else {
                prev_lo.ud1()
            },
        ));
        set_hi(GprReg::from_u64(
            if dd * 2 == 0 {
                new_hi as u64
            } else {
                prev_hi.ud0()
            },
            if dd * 2 == 2 {
                new_hi as u64
            } else {
                prev_hi.ud1()
            },
        ));
    }
    if rd != 0 {
        let mut out = GprReg::from_u64(prev_lo.ud0(), prev_lo.ud1());
        out = out.set_ul(dd * 2, lo().ul(dd * 2));
        out = out.set_ul(dd * 2 + 1, hi().ul(dd * 2));
        set_gpr(rd, out);
    }
}

/// `PMADDW` — parallel multiply-add of 2x 32-bit lanes.
pub fn PMADDW() {
    pmaddw_slot(0, 0, false);
    pmaddw_slot(1, 2, false);
}

/// `PMSUBW` — parallel multiply-sub of 2x 32-bit lanes.
pub fn PMSUBW() {
    pmaddw_slot(0, 0, true);
    pmaddw_slot(1, 2, true);
}

/// `PSLLVW` — parallel variable shift-left of 2x 32-bit lanes.
pub fn PSLLVW() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    out = out.set_ud(
        0,
        ((tgt.ul(0) << (src.ul(0) & 0x1F)) as i32) as i64 as u64,
    );
    out = out.set_ud(
        1,
        ((tgt.ul(2) << (src.ul(2) & 0x1F)) as i32) as i64 as u64,
    );
    set_gpr(rd, out);
}

/// `PSRLVW` — parallel variable shift-right of 2x 32-bit lanes.
pub fn PSRLVW() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    out = out.set_ud(
        0,
        ((tgt.ul(0) >> (src.ul(0) & 0x1F)) as i32) as i64 as u64,
    );
    out = out.set_ud(
        1,
        ((tgt.ul(2) >> (src.ul(2) & 0x1F)) as i32) as i64 as u64,
    );
    set_gpr(rd, out);
}

/// `PMFHI` — pack HI.UD[0..1] into `_Rd_`.
pub fn PMFHI() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    set_gpr(rd, hi());
}

/// `PMFLO` — pack LO.UD[0..1] into `_Rd_`.
pub fn PMFLO() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    set_gpr(rd, lo());
}

/// `PINTH` — interleave upper halves of 16-bit lanes.
pub fn PINTH() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    out = out.set_us(0, tgt.us(0));
    out = out.set_us(1, src.us(4));
    out = out.set_us(2, tgt.us(1));
    out = out.set_us(3, src.us(5));
    out = out.set_us(4, tgt.us(2));
    out = out.set_us(5, src.us(6));
    out = out.set_us(6, tgt.us(3));
    out = out.set_us(7, src.us(7));
    set_gpr(rd, out);
}

/// `PMULTW` — parallel multiply of 2x 32-bit lanes (signed).
#[inline]
fn pmultw_slot(dd: usize, ss: usize) {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let a = src.sl(ss) as i64;
    let b = tgt.sl(ss) as i64;
    let temp = a * b;
    let new_lo = (temp & 0xFFFF_FFFF) as i32;
    let new_hi = (temp >> 32) as i32;
    let prev_lo = lo();
    let prev_hi = hi();
    set_lo(GprReg::from_u64(
        if dd == 0 {
            new_lo as u64
        } else {
            prev_lo.ud0()
        },
        if dd == 1 {
            new_lo as u64
        } else {
            prev_lo.ud1()
        },
    ));
    set_hi(GprReg::from_u64(
        if dd == 0 {
            new_hi as u64
        } else {
            prev_hi.ud0()
        },
        if dd == 1 {
            new_hi as u64
        } else {
            prev_hi.ud1()
        },
    ));
    if rd != 0 {
        set_gpr(rd, GprReg::from_u64(temp as u64, 0));
    }
}

/// `PMULTW` — parallel 32-bit signed multiplies, both slots.
pub fn PMULTW() {
    pmultw_slot(0, 0);
    pmultw_slot(1, 2);
}

/// `PDIVW` — parallel signed divide.
#[inline]
fn pdivw_slot(dd: usize, ss: usize) {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let rs_v = src.ul(ss);
    let rt_v = tgt.ul(ss);
    let (new_lo, new_hi) = if rs_v == 0x8000_0000 && rt_v == 0xFFFF_FFFF {
        (0x8000_0000i32, 0i32)
    } else if (rt_v as i32) != 0 {
        let r = rs_v as i32;
        let d = rt_v as i32;
        (r / d, r % d)
    } else {
        let r = rs_v as i32;
        (if r < 0 { 1i32 } else { -1i32 }, r)
    };
    let prev_lo = lo();
    let prev_hi = hi();
    set_lo(GprReg::from_u64(
        if dd == 0 {
            new_lo as u64
        } else {
            prev_lo.ud0()
        },
        if dd == 1 {
            new_lo as u64
        } else {
            prev_lo.ud1()
        },
    ));
    set_hi(GprReg::from_u64(
        if dd == 0 {
            new_hi as u64
        } else {
            prev_hi.ud0()
        },
        if dd == 1 {
            new_hi as u64
        } else {
            prev_hi.ud1()
        },
    ));
}

/// `PDIVW` — both slots.
pub fn PDIVW() {
    pdivw_slot(0, 0);
    pdivw_slot(1, 2);
}

/// `PCPYLD` — copy upper 64 bits of Rs, lower 64 bits of Rt.
pub fn PCPYLD() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    // Note: write upper first, in case rd == rs.
    let new_hi = src.ud0();
    let new_lo = tgt.ud0();
    set_gpr(rd, GprReg::from_u64(new_lo, new_hi));
}

/// `PMADDH` — 8x 16-bit lane multiply-add into LO/HI.
pub fn PMADDH() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let prev_lo = lo();
    let prev_hi = hi();
    let new_lo0 = prev_lo.ul0().wrapping_add((src.ss(0) as i32).wrapping_mul(tgt.ss(0) as i32) as u32);
    let new_lo1 = prev_lo.ul1().wrapping_add((src.ss(1) as i32).wrapping_mul(tgt.ss(1) as i32) as u32);
    let new_hi0 = prev_hi.ul0().wrapping_add((src.ss(2) as i32).wrapping_mul(tgt.ss(2) as i32) as u32);
    let new_hi1 = prev_hi.ul1().wrapping_add((src.ss(3) as i32).wrapping_mul(tgt.ss(3) as i32) as u32);
    let new_lo2 = prev_lo.ul2().wrapping_add((src.ss(4) as i32).wrapping_mul(tgt.ss(4) as i32) as u32);
    let new_lo3 = prev_lo.ul3().wrapping_add((src.ss(5) as i32).wrapping_mul(tgt.ss(5) as i32) as u32);
    let new_hi2 = prev_hi.ul2().wrapping_add((src.ss(6) as i32).wrapping_mul(tgt.ss(6) as i32) as u32);
    let new_hi3 = prev_hi.ul3().wrapping_add((src.ss(7) as i32).wrapping_mul(tgt.ss(7) as i32) as u32);
    set_lo(GprReg::from_u64(
        pack_u64(new_lo0, new_lo1),
        pack_u64(new_lo2, new_lo3),
    ));
    set_hi(GprReg::from_u64(
        pack_u64(new_hi0, new_hi1),
        pack_u64(new_hi2, new_hi3),
    ));
    if rd != 0 {
        let mut out = GprReg::from_u64(0, 0);
        out = out.set_ul(0, lo().ul0());
        out = out.set_ul(1, hi().ul0());
        out = out.set_ul(2, lo().ul2());
        out = out.set_ul(3, hi().ul2());
        set_gpr(rd, out);
    }
}

#[inline]
fn phmadh_lo(dd: usize, n: usize) {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let first = (src.ss(n + 1) as i32).wrapping_mul(tgt.ss(n + 1) as i32);
    let total = first.wrapping_add((src.ss(n) as i32).wrapping_mul(tgt.ss(n) as i32));
    let prev_lo = lo();
    set_lo(GprReg::from_u64(
        if dd == 0 {
            total as u32 as u64
        } else {
            prev_lo.ud0()
        },
        if dd == 2 {
            total as u32 as u64
        } else {
            prev_lo.ud1()
        },
    ));
}

#[inline]
fn phmadh_hi(dd: usize, n: usize) {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let first = (src.ss(n + 1) as i32).wrapping_mul(tgt.ss(n + 1) as i32);
    let total = first.wrapping_add((src.ss(n) as i32).wrapping_mul(tgt.ss(n) as i32));
    let prev_hi = hi();
    set_hi(GprReg::from_u64(
        if dd == 0 {
            total as u32 as u64
        } else {
            prev_hi.ud0()
        },
        if dd == 2 {
            total as u32 as u64
        } else {
            prev_hi.ud1()
        },
    ));
}

/// `PHMADH` — packed-high multiply-add.
pub fn PHMADH() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    phmadh_lo(0, 0);
    phmadh_hi(0, 2);
    phmadh_lo(2, 4);
    phmadh_hi(2, 6);
    if rd != 0 {
        let mut out = GprReg::from_u64(0, 0);
        out = out.set_ul(0, lo().ul0());
        out = out.set_ul(1, hi().ul0());
        out = out.set_ul(2, lo().ul2());
        out = out.set_ul(3, hi().ul2());
        set_gpr(rd, out);
    }
}

/// `PAND` — bitwise AND of two 128-bit registers.
pub fn PAND() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    set_gpr(rd, GprReg::from_u64(src.ud0() & tgt.ud0(), src.ud1() & tgt.ud1()));
}

/// `PXOR` — bitwise XOR of two 128-bit registers.
pub fn PXOR() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    set_gpr(rd, GprReg::from_u64(src.ud0() ^ tgt.ud0(), src.ud1() ^ tgt.ud1()));
}

/// `PMSUBH` — 8x 16-bit lane multiply-sub into LO/HI.
pub fn PMSUBH() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let prev_lo = lo();
    let prev_hi = hi();
    let new_lo0 = prev_lo.ul0().wrapping_sub((src.ss(0) as i32).wrapping_mul(tgt.ss(0) as i32) as u32);
    let new_lo1 = prev_lo.ul1().wrapping_sub((src.ss(1) as i32).wrapping_mul(tgt.ss(1) as i32) as u32);
    let new_hi0 = prev_hi.ul0().wrapping_sub((src.ss(2) as i32).wrapping_mul(tgt.ss(2) as i32) as u32);
    let new_hi1 = prev_hi.ul1().wrapping_sub((src.ss(3) as i32).wrapping_mul(tgt.ss(3) as i32) as u32);
    let new_lo2 = prev_lo.ul2().wrapping_sub((src.ss(4) as i32).wrapping_mul(tgt.ss(4) as i32) as u32);
    let new_lo3 = prev_lo.ul3().wrapping_sub((src.ss(5) as i32).wrapping_mul(tgt.ss(5) as i32) as u32);
    let new_hi2 = prev_hi.ul2().wrapping_sub((src.ss(6) as i32).wrapping_mul(tgt.ss(6) as i32) as u32);
    let new_hi3 = prev_hi.ul3().wrapping_sub((src.ss(7) as i32).wrapping_mul(tgt.ss(7) as i32) as u32);
    set_lo(GprReg::from_u64(
        pack_u64(new_lo0, new_lo1),
        pack_u64(new_lo2, new_lo3),
    ));
    set_hi(GprReg::from_u64(
        pack_u64(new_hi0, new_hi1),
        pack_u64(new_hi2, new_hi3),
    ));
    if rd != 0 {
        let mut out = GprReg::from_u64(0, 0);
        out = out.set_ul(0, lo().ul0());
        out = out.set_ul(1, hi().ul0());
        out = out.set_ul(2, lo().ul2());
        out = out.set_ul(3, hi().ul2());
        set_gpr(rd, out);
    }
}

#[inline]
fn phmsbh_lo(dd: usize, n: usize) {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let first = (src.ss(n + 1) as i32).wrapping_mul(tgt.ss(n + 1) as i32);
    let total = first.wrapping_sub((src.ss(n) as i32).wrapping_mul(tgt.ss(n) as i32));
    let prev_lo = lo();
    set_lo(GprReg::from_u64(
        if dd == 0 {
            total as u32 as u64
        } else {
            prev_lo.ud0()
        },
        if dd == 2 {
            total as u32 as u64
        } else {
            prev_lo.ud1()
        },
    ));
}

#[inline]
fn phmsbh_hi(dd: usize, n: usize) {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let first = (src.ss(n + 1) as i32).wrapping_mul(tgt.ss(n + 1) as i32);
    let total = first.wrapping_sub((src.ss(n) as i32).wrapping_mul(tgt.ss(n) as i32));
    let prev_hi = hi();
    set_hi(GprReg::from_u64(
        if dd == 0 {
            total as u32 as u64
        } else {
            prev_hi.ud0()
        },
        if dd == 2 {
            total as u32 as u64
        } else {
            prev_hi.ud1()
        },
    ));
}

/// `PHMSBH` — packed-high multiply-subtract.
pub fn PHMSBH() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    phmsbh_lo(0, 0);
    phmsbh_hi(0, 2);
    phmsbh_lo(2, 4);
    phmsbh_hi(2, 6);
    if rd != 0 {
        let mut out = GprReg::from_u64(0, 0);
        out = out.set_ul(0, lo().ul0());
        out = out.set_ul(1, hi().ul0());
        out = out.set_ul(2, lo().ul2());
        out = out.set_ul(3, hi().ul2());
        set_gpr(rd, out);
    }
}

/// `PEXEH` — swap even/odd lanes within each 32-bit word (16-bit lane swap).
pub fn PEXEH() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rt = _rt(code);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    out = out.set_us(0, tgt.us(2));
    out = out.set_us(1, tgt.us(1));
    out = out.set_us(2, tgt.us(0));
    out = out.set_us(3, tgt.us(3));
    out = out.set_us(4, tgt.us(6));
    out = out.set_us(5, tgt.us(5));
    out = out.set_us(6, tgt.us(4));
    out = out.set_us(7, tgt.us(7));
    set_gpr(rd, out);
}

/// `PREVH` — reverse the 16-bit lanes.
pub fn PREVH() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rt = _rt(code);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    out = out.set_us(0, tgt.us(3));
    out = out.set_us(1, tgt.us(2));
    out = out.set_us(2, tgt.us(1));
    out = out.set_us(3, tgt.us(0));
    out = out.set_us(4, tgt.us(7));
    out = out.set_us(5, tgt.us(6));
    out = out.set_us(6, tgt.us(5));
    out = out.set_us(7, tgt.us(4));
    set_gpr(rd, out);
}

/// `PMULTH` — 8x 16-bit lane multiply into LO/HI.
pub fn PMULTH() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let prev_lo = lo();
    let prev_hi = hi();
    let new_lo0 = (src.ss(0) as i32).wrapping_mul(tgt.ss(0) as i32) as u32;
    let new_lo1 = (src.ss(1) as i32).wrapping_mul(tgt.ss(1) as i32) as u32;
    let new_hi0 = (src.ss(2) as i32).wrapping_mul(tgt.ss(2) as i32) as u32;
    let new_hi1 = (src.ss(3) as i32).wrapping_mul(tgt.ss(3) as i32) as u32;
    let new_lo2 = (src.ss(4) as i32).wrapping_mul(tgt.ss(4) as i32) as u32;
    let new_lo3 = (src.ss(5) as i32).wrapping_mul(tgt.ss(5) as i32) as u32;
    let new_hi2 = (src.ss(6) as i32).wrapping_mul(tgt.ss(6) as i32) as u32;
    let new_hi3 = (src.ss(7) as i32).wrapping_mul(tgt.ss(7) as i32) as u32;
    set_lo(GprReg::from_u64(
        pack_u64(new_lo0, new_lo1),
        pack_u64(new_lo2, new_lo3),
    ));
    set_hi(GprReg::from_u64(
        pack_u64(new_hi0, new_hi1),
        pack_u64(new_hi2, new_hi3),
    ));
    let _ = prev_lo;
    let _ = prev_hi;
    if rd != 0 {
        let mut out = GprReg::from_u64(0, 0);
        out = out.set_ul(0, lo().ul0());
        out = out.set_ul(1, hi().ul0());
        out = out.set_ul(2, lo().ul2());
        out = out.set_ul(3, hi().ul2());
        set_gpr(rd, out);
    }
}

/// `PDIVBW` — parallel divide-by-word: divide all four 32-bit lanes by
/// `Rt.SS[0]` (the low 16-bit half).
#[inline]
fn pdivbw_slot(n: usize) {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let rs_v = src.ul(n);
    let rt_v = tgt.us(0);
    let (new_lo, new_hi) = if rs_v == 0x8000_0000 && rt_v == 0xFFFF {
        (0x8000_0000i32, 0i32)
    } else if tgt.ss(0) != 0 {
        let r = rs_v as i32;
        let d = tgt.ss(0) as i32;
        (r / d, r % d)
    } else {
        let r = rs_v as i32;
        (if r < 0 { 1i32 } else { -1i32 }, r)
    };
    let prev_lo = lo();
    let prev_hi = hi();
    set_lo(GprReg::from_u64(
        if n == 0 {
            new_lo as u64
        } else if n == 1 {
            prev_lo.ud0()
        } else if n == 2 {
            prev_lo.ud0()
        } else {
            prev_lo.ud0()
        },
        if n == 1 {
            new_lo as u64
        } else if n == 2 {
            new_lo as u64
        } else if n == 3 {
            new_lo as u64
        } else {
            prev_lo.ud1()
        },
    ));
    set_hi(GprReg::from_u64(
        if n == 0 {
            new_hi as u64
        } else if n == 1 {
            prev_hi.ud0()
        } else if n == 2 {
            prev_hi.ud0()
        } else {
            prev_hi.ud0()
        },
        if n == 1 {
            new_hi as u64
        } else if n == 2 {
            new_hi as u64
        } else if n == 3 {
            new_hi as u64
        } else {
            prev_hi.ud1()
        },
    ));
}

/// `PDIVBW` — all four lanes.
pub fn PDIVBW() {
    pdivbw_slot(0);
    pdivbw_slot(1);
    pdivbw_slot(2);
    pdivbw_slot(3);
}

/// `PEXEW` — swap even/odd 32-bit lanes within each 64-bit half.
pub fn PEXEW() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rt = _rt(code);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    out = out.set_ul(0, tgt.ul(2));
    out = out.set_ul(1, tgt.ul(1));
    out = out.set_ul(2, tgt.ul(0));
    out = out.set_ul(3, tgt.ul(3));
    set_gpr(rd, out);
}

/// `PROT3W` — rotate three 32-bit lanes.
pub fn PROT3W() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rt = _rt(code);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    out = out.set_ul(0, tgt.ul(1));
    out = out.set_ul(1, tgt.ul(2));
    out = out.set_ul(2, tgt.ul(0));
    out = out.set_ul(3, tgt.ul(3));
    set_gpr(rd, out);
}

// ===========================================================================
// MMI3 opcodes
// ===========================================================================

/// Internal helper for `PMADDUW` / `PMULTUW` / `PDIVUW`.
#[inline]
fn pmadduw_slot(dd: usize, ss: usize) {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let acc = lo().ud(ss) | (hi().ud(ss) << 32);
    let prod = (src.ul(ss) as u64).wrapping_mul(tgt.ul(ss) as u64);
    let tempu = acc.wrapping_add(prod);
    let new_lo = (tempu & 0xFFFF_FFFF) as i32;
    let new_hi = (tempu >> 32) as i32;
    let prev_lo = lo();
    let prev_hi = hi();
    set_lo(GprReg::from_u64(
        if dd == 0 {
            new_lo as u64
        } else {
            prev_lo.ud0()
        },
        if dd == 1 {
            new_lo as u64
        } else {
            prev_lo.ud1()
        },
    ));
    set_hi(GprReg::from_u64(
        if dd == 0 {
            new_hi as u64
        } else {
            prev_hi.ud0()
        },
        if dd == 1 {
            new_hi as u64
        } else {
            prev_hi.ud1()
        },
    ));
    if rd != 0 {
        set_gpr(rd, GprReg::from_u64(tempu, 0));
    }
}

/// `PMADDUW` — parallel unsigned multiply-add, both slots.
pub fn PMADDUW() {
    pmadduw_slot(0, 0);
    pmadduw_slot(1, 2);
}

/// `PSRAVW` — parallel variable arithmetic shift-right of 2x 32-bit lanes.
pub fn PSRAVW() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    out = out.set_ud(
        0,
        ((tgt.sl(0) >> (src.ul(0) & 0x1F)) as i64) as u64,
    );
    out = out.set_ud(
        1,
        ((tgt.sl(2) >> (src.ul(2) & 0x1F)) as i64) as u64,
    );
    set_gpr(rd, out);
}

/// `PMTHI` — write HI from `_Rs_`.
pub fn PMTHI() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rs = _rs(code);
    set_hi(gpr(rs));
}

/// `PMTLO` — write LO from `_Rs_`.
pub fn PMTLO() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rs = _rs(code);
    set_lo(gpr(rs));
}

/// `PINTEH` — interleave even 16-bit lanes of Rs/Rt into the result.
pub fn PINTEH() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    out = out.set_us(0, tgt.us(0));
    out = out.set_us(1, src.us(0));
    out = out.set_us(2, tgt.us(2));
    out = out.set_us(3, src.us(2));
    out = out.set_us(4, tgt.us(4));
    out = out.set_us(5, src.us(4));
    out = out.set_us(6, tgt.us(6));
    out = out.set_us(7, src.us(6));
    set_gpr(rd, out);
}

/// `PMULTUW` — parallel unsigned multiply of 2x 32-bit lanes.
#[inline]
fn pmultuw_slot(dd: usize, ss: usize) {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let tempu = (src.ul(ss) as u64).wrapping_mul(tgt.ul(ss) as u64);
    let new_lo = (tempu & 0xFFFF_FFFF) as i32;
    let new_hi = (tempu >> 32) as i32;
    let prev_lo = lo();
    let prev_hi = hi();
    set_lo(GprReg::from_u64(
        if dd == 0 {
            new_lo as u64
        } else {
            prev_lo.ud0()
        },
        if dd == 1 {
            new_lo as u64
        } else {
            prev_lo.ud1()
        },
    ));
    set_hi(GprReg::from_u64(
        if dd == 0 {
            new_hi as u64
        } else {
            prev_hi.ud0()
        },
        if dd == 1 {
            new_hi as u64
        } else {
            prev_hi.ud1()
        },
    ));
    if rd != 0 {
        set_gpr(rd, GprReg::from_u64(tempu, 0));
    }
}

/// `PMULTUW` — both slots.
pub fn PMULTUW() {
    pmultuw_slot(0, 0);
    pmultuw_slot(1, 2);
}

/// `PDIVUW` — parallel unsigned divide.
#[inline]
fn pdivuw_slot(dd: usize, ss: usize) {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let rt_v = tgt.ul(ss);
    let (new_lo, new_hi) = if rt_v != 0 {
        let q = src.ul(ss) / rt_v;
        let r = src.ul(ss) % rt_v;
        (q as i32, r as i32)
    } else {
        (-1i32, src.sl(ss))
    };
    let prev_lo = lo();
    let prev_hi = hi();
    set_lo(GprReg::from_u64(
        if dd == 0 {
            new_lo as u64
        } else {
            prev_lo.ud0()
        },
        if dd == 1 {
            new_lo as u64
        } else {
            prev_lo.ud1()
        },
    ));
    set_hi(GprReg::from_u64(
        if dd == 0 {
            new_hi as u64
        } else {
            prev_hi.ud0()
        },
        if dd == 1 {
            new_hi as u64
        } else {
            prev_hi.ud1()
        },
    ));
}

/// `PDIVUW` — both slots.
pub fn PDIVUW() {
    pdivuw_slot(0, 0);
    pdivuw_slot(1, 2);
}

/// `PCPYUD` — copy lower 64 bits of Rt, upper 64 bits of Rs.
pub fn PCPYUD() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    let new_hi = src.ud1();
    let new_lo = tgt.ud1();
    set_gpr(rd, GprReg::from_u64(new_lo, new_hi));
}

/// `POR` — bitwise OR of two 128-bit registers.
pub fn POR() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    set_gpr(rd, GprReg::from_u64(src.ud0() | tgt.ud0(), src.ud1() | tgt.ud1()));
}

/// `PNOR` — bitwise NOR of two 128-bit registers.
pub fn PNOR() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rs = _rs(code);
    let rt = _rt(code);
    let src = gpr(rs);
    let tgt = gpr(rt);
    set_gpr(rd, GprReg::from_u64(!(src.ud0() | tgt.ud0()), !(src.ud1() | tgt.ud1())));
}

/// `PEXCH` — swap 16-bit lanes 0/2 and 4/6 within each 32-bit word.
pub fn PEXCH() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rt = _rt(code);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    out = out.set_us(0, tgt.us(0));
    out = out.set_us(1, tgt.us(2));
    out = out.set_us(2, tgt.us(1));
    out = out.set_us(3, tgt.us(3));
    out = out.set_us(4, tgt.us(4));
    out = out.set_us(5, tgt.us(6));
    out = out.set_us(6, tgt.us(5));
    out = out.set_us(7, tgt.us(7));
    set_gpr(rd, out);
}

/// `PCPYH` — copy 16-bit lane 0 and lane 4 to the full register.
pub fn PCPYH() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rt = _rt(code);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    out = out.set_us(0, tgt.us(0));
    out = out.set_us(1, tgt.us(0));
    out = out.set_us(2, tgt.us(0));
    out = out.set_us(3, tgt.us(0));
    out = out.set_us(4, tgt.us(4));
    out = out.set_us(5, tgt.us(4));
    out = out.set_us(6, tgt.us(4));
    out = out.set_us(7, tgt.us(4));
    set_gpr(rd, out);
}

/// `PEXCW` — swap 32-bit lanes 0/2 within each 64-bit half.
pub fn PEXCW() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let rd = _rd(code);
    if rd == 0 {
        return;
    }
    let rt = _rt(code);
    let tgt = gpr(rt);
    let mut out = GprReg::from_u64(0, 0);
    out = out.set_ul(0, tgt.ul(0));
    out = out.set_ul(1, tgt.ul(2));
    out = out.set_ul(2, tgt.ul(1));
    out = out.set_ul(3, tgt.ul(3));
    set_gpr(rd, out);
}

// ---------------------------------------------------------------------------
// Public dispatch table for the MMI opcode family
// ---------------------------------------------------------------------------

/// MMI base opcodes (PLZCW, PMFHL, PMTHL, PSLLH, PSRLH, PSRAH, PSLLW,
/// PSRLW, PSRAW). The dispatcher reads `_Sa_` and the lower 6 bits of
/// `cpuRegs.code` to pick the right handler.
pub fn MMI_Interpret() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let sa = _sa(code);
    let op = code & 0x3F;
    match op {
        0x00 => PLZCW(),
        0x01 => match sa {
            0 => MFHI1(),
            1 => MFLO1(),
            2 => MTHI1(),
            3 => MTLO1(),
            _ => {}
        },
        0x02 => match sa {
            0 => MADD(),
            1 => MADDU(),
            2 => MADD1(),
            3 => MADDU1(),
            _ => {}
        },
        0x04 => PLZCW(),
        0x05 => match sa {
            0 => MULT1(),
            1 => MULTU1(),
            2 => DIV1(),
            3 => DIVU1(),
            _ => {}
        },
        0x08 => MMI0_Interpret(),
        0x09 => MMI1_Interpret(),
        0x0A => MMI2_Interpret(),
        0x0B => MMI3_Interpret(),
        0x0F => match sa {
            0 => PMFHL(),
            1 => PMTHL(),
            _ => {}
        },
        0x10 => PSLLH(),
        0x11 => PSRLH(),
        0x12 => PSRAH(),
        0x14 => PSLLW(),
        0x15 => PSRLW(),
        0x16 => PSRAW(),
        _ => {}
    }
}

/// MMI0 dispatch wrapper. The MMI0 opcode space is shared with the parallel
/// shift family above; we keep this entry point as a thin passthrough for
/// callers that only want MMI0 semantics.
pub fn MMI0_Interpret() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let op = code & 0x3F;
    match op {
        0x00 => PADDW(),
        0x01 => PSUBW(),
        0x02 => PCGTW(),
        0x03 => PMAXW(),
        0x04 => PADDH(),
        0x05 => PSUBH(),
        0x06 => PCGTH(),
        0x07 => PMAXH(),
        0x08 => PADDB(),
        0x09 => PSUBB(),
        0x0A => PCGTB(),
        0x0B => PADDSW(),
        0x0C => PSUBSW(),
        0x0D => PEXTLW(),
        0x0E => PPACW(),
        0x0F => PADDSH(),
        0x10 => PSUBSH(),
        0x11 => PEXTLH(),
        0x12 => PPACH(),
        0x13 => PADDSB(),
        0x14 => PSUBSB(),
        0x15 => PEXTLB(),
        0x16 => PPACB(),
        0x17 => PEXT5(),
        0x18 => PPAC5(),
        _ => {}
    }
}

/// MMI1 dispatch wrapper.
pub fn MMI1_Interpret() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let op = code & 0x3F;
    match op {
        0x00 => PABSW(),
        0x01 => PCEQW(),
        0x02 => PMINW(),
        0x03 => PADSBH(),
        0x04 => PABSH(),
        0x05 => PCEQH(),
        0x06 => PMINH(),
        0x07 => PCEQB(),
        0x08 => PADDUW(),
        0x09 => PSUBUW(),
        0x0A => PEXTUW(),
        0x0B => PADDUH(),
        0x0C => PSUBUH(),
        0x0D => PEXTUH(),
        0x0E => PADDUB(),
        0x0F => PSUBUB(),
        0x10 => PEXTUB(),
        0x11 => QFSRV(),
        _ => {}
    }
}

/// MMI2 dispatch wrapper.
pub fn MMI2_Interpret() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let op = code & 0x3F;
    match op {
        0x00 => PMADDW(),
        0x01 => PSLLVW(),
        0x02 => PSRLVW(),
        0x03 => PMSUBW(),
        0x04 => PMFHI(),
        0x05 => PMFLO(),
        0x06 => PINTH(),
        0x07 => PMULTW(),
        0x08 => PDIVW(),
        0x09 => PCPYLD(),
        0x0A => PMADDH(),
        0x0B => PHMADH(),
        0x0C => PAND(),
        0x0D => PXOR(),
        0x0E => PMSUBH(),
        0x0F => PHMSBH(),
        0x10 => PEXEH(),
        0x11 => PREVH(),
        0x12 => PMULTH(),
        0x13 => PDIVBW(),
        0x14 => PEXEW(),
        0x15 => PROT3W(),
        _ => {}
    }
}

/// MMI3 dispatch wrapper.
pub fn MMI3_Interpret() {
    let code = unsafe { CoreMain::cpuRegs.code };
    let op = code & 0x3F;
    match op {
        0x00 => PMADDUW(),
        0x01 => PSRAVW(),
        0x02 => PMTHI(),
        0x03 => PMTLO(),
        0x04 => PINTEH(),
        0x05 => PMULTUW(),
        0x06 => PDIVUW(),
        0x07 => PCPYUD(),
        0x08 => POR(),
        0x09 => PNOR(),
        0x0A => PEXCH(),
        0x0B => PCPYH(),
        0x0C => PEXCW(),
        _ => {}
    }
}
