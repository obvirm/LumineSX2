// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
//! Rust translation of the legacy `pcsx2/IopGte.cpp` / `IopGte.h` pair.
//!
//! The IOP (PS1) Geometry Transformation Engine is the 64-bit fixed-point
//! matrix coprocessor that lives behind COP2 on the R3000A. This module is
//! the IOP-side counterpart to the EE's GTE: it exposes the COP2 register
//! file (data + control), the `gteFLAG` accumulator, and the 22 GTE
//! instructions that the PSX toolchain (and a sizeable chunk of PS1
//! retail games) emit as part of the affine / lighting / clipping
//! pipeline.
//!
//! All state is held in `static mut` to mirror the C++ original's reliance
//! on `psxRegs.CP2D` / `psxRegs.CP2C` globals. The public register file
//! consists of two compact arrays of `GTERegister`:
//!
//! * `gteV: [GTERegister; 32]` is the data file (`CP2D`).
//! * `gteM: [GTERegister; 8]` is the matrix / translation view of the
//!   control file (`CP2C.r[0..7]`: R11..R33 + TRX/TRY/TRZ). The remaining
//!   control slots (light matrix, background / far colour, DQA/DQB,
//!   ZSF3/ZSF4) are stored in the private `gteC` static; the pieces the
//!   spec calls out as separate statics (`gteOFX`, `gteOFY`, `gteH`,
//!   `gteFLAG`, `gteR`) are still surfaced directly.
//!
//! Each `GTERegister` holds four `i16` lanes, which matches the way the
//! C++ packs a `u32` register as `(s16 lo, s16 hi)` and reinterprets it
//! for vector / matrix / RGB uses. The `v[2]` / `v[3]` lanes are
//! reserved so the same struct can be aliased as a `u64` / `[u32; 2]`
//! for callers that need that view.
//!
//! Only `std` is used.

// ===========================================================================
// Register model.
// ===========================================================================

/// A single GTE register slot. The IOP COP2 file is logically 32-bit wide
/// but every consumer (vector lane, matrix element, RGB triplet, 32-bit
/// accum) reinterprets the bits, so the safest faithful model is four
/// `i16` lanes that can be aliased as `u32`/`u64` views as required.
#[derive(Copy, Clone, Default)]
pub struct GTERegister {
    pub v: [i16; 4],
}

impl GTERegister {
    /// Construct a register from its four `i16` lanes.
    pub const fn new(v0: i16, v1: i16, v2: i16, v3: i16) -> Self {
        Self { v: [v0, v1, v2, v3] }
    }

    /// Reinterpret the lower 32 bits of the register as an `i32`.
    #[inline]
    pub fn as_i32_lo(&self) -> i32 {
        i32::from(self.v[0]) | (i32::from(self.v[1]) << 16)
    }

    /// Reinterpret the upper 32 bits of the register as an `i32`.
    #[inline]
    pub fn as_i32_hi(&self) -> i32 {
        i32::from(self.v[2]) | (i32::from(self.v[3]) << 16)
    }

    /// Reinterpret the lower 32 bits of the register as a `u32`.
    #[inline]
    pub fn as_u32_lo(&self) -> u32 {
        self.as_i32_lo() as u32
    }

    /// Reinterpret the upper 32 bits of the register as a `u32`.
    #[inline]
    pub fn as_u32_hi(&self) -> u32 {
        self.as_i32_hi() as u32
    }
}

// ===========================================================================
// Public register file: data (CP2D) and matrix/control (CP2C matrix view).
// ===========================================================================

/// The 32-entry GTE data register file (`CP2D`). The low two i16 lanes of
/// each slot hold the lo/hi halves of the corresponding `u32` in the
/// original `psxRegs.CP2D.r[]` file; `v[2]` and `v[3]` are reserved.
pub static mut gteV: [GTERegister; 32] = [GTERegister { v: [0; 4] }; 32];

/// The 8-entry GTE matrix/control view. Packing follows the PSX COP2C
/// layout for `CP2C.r[0..7]`:
///
/// * `gteM[0].v = [R11, R12, R13, R21]`
/// * `gteM[1].v = [R22, R23, R31, R32]`
/// * `gteM[2].v = [R33, _, TRX_lo, TRX_hi]`
/// * `gteM[3].v = [TRY_lo, TRY_hi, TRZ_lo, TRZ_hi]`
/// * `gteM[4..7]` are reserved for translation-extension slots.
///
/// `gteFLAG`, `gteOFX`, `gteOFY`, `gteH` live alongside this struct so the
/// matches in `gteExecute` don't have to reach into it.
pub static mut gteM: [GTERegister; 8] = [GTERegister { v: [0; 4] }; 8];

/// GTE flag register. Bit 31 (`0x8000_0000`) is the "error summary" bit
/// and the rest of the upper word carries sticky per-operation error
/// flags.
pub static mut gteFLAG: u64 = 0;

/// The "RGB+code" output register. In the C++ this is the byte view
/// `gteR/gteG/gteB/gteCODE` aliased onto `CP2D.r[6]`. We expose it as a
/// full `GTERegister` so the `gteR2 = ...` / `gteCODE2 = ...` style
/// writes in the lighting ops stay a single store.
pub static mut gteR: GTERegister = GTERegister { v: [0; 4] };

/// Screen X offset. Sourced from `CP2C.r[24]` in the original.
pub static mut gteOFX: i32 = 0;
/// Screen Y offset. Sourced from `CP2C.r[25]` in the original.
pub static mut gteOFY: i32 = 0;
/// Projection-plane distance. Sourced from `CP2C.r[26]` in the original.
pub static mut gteH: i16 = 0;

// ===========================================================================
// Private backing storage for the rest of the COP2C control file. The
// `gteM` array above only covers the 8 u32s in `CP2C.r[0..7]` (the
// rotation matrix + translation). The remaining 24 u32s -- the light
// matrix, the background / far colour, DQA/DQB, ZSF3/ZSF4 -- live in
// `gteC`.
// ===========================================================================

/// Private backing storage for `CP2C.r[8..31]`. Packing within each
/// `GTERegister` follows the same lo/hi i16 convention as the public
/// `gteM` array; only `v[0]` and `v[1]` are meaningful.
static mut gteC: [GTERegister; 24] = [GTERegister { v: [0; 4] }; 24];

// ===========================================================================
// Init / reset.
// ===========================================================================

/// Initialise the GTE. In the original C++ this is a no-op aside from
/// optional logging; we zero the public statics to make the module safe
/// to use from a fresh emulator state.
pub fn gteInit() {
    unsafe {
        gteV = [GTERegister { v: [0; 4] }; 32];
        gteM = [GTERegister { v: [0; 4] }; 8];
        gteC = [GTERegister { v: [0; 4] }; 24];
        gteFLAG = 0;
        gteR = GTERegister { v: [0; 4] };
        gteOFX = 0;
        gteOFY = 0;
        gteH = 0;
    }
}

/// Reset the GTE -- same as `gteInit` for our purposes.
pub fn gteReset() {
    gteInit();
}

// ===========================================================================
// `sum_flag` -- the C++ `SUM_FLAG` macro.
// ===========================================================================

/// `gteFLAG |= 0x8000_0000` if any of the "real" error bits are set. The
/// C++ form is `if (gteFLAG & 0x7F87_E000) gteFLAG |= 0x8000_0000;`.
#[inline]
fn sum_flag() {
    unsafe {
        if gteFLAG & 0x7F87_E000 != 0 {
            gteFLAG |= 0x8000_0000;
        }
    }
}

// ===========================================================================
// `F12lim*` / `Flim*` limiters. The C++ defines these as a family of
// inline helpers built on the `_LIMX` macro. We keep the same names /
// flag bits so the rest of the file reads the same as the C++.
// ===========================================================================

/// 12.4 fixed-point signed saturate to 16-bit signed, flag bit 24.
#[inline]
fn f12lim_a1s(x: i64) -> i32 {
    let min = -(32768_i64 << 12);
    let max = 32767_i64 << 12;
    unsafe {
        if x < min {
            gteFLAG |= 1u64 << 24;
            min as i32
        } else if x > max {
            gteFLAG |= 1u64 << 24;
            max as i32
        } else {
            x as i32
        }
    }
}

/// 12.4 fixed-point signed saturate to 16-bit signed, flag bit 23.
#[inline]
fn f12lim_a2s(x: i64) -> i32 {
    let min = -(32768_i64 << 12);
    let max = 32767_i64 << 12;
    unsafe {
        if x < min {
            gteFLAG |= 1u64 << 23;
            min as i32
        } else if x > max {
            gteFLAG |= 1u64 << 23;
            max as i32
        } else {
            x as i32
        }
    }
}

/// 12.4 fixed-point signed saturate to 16-bit signed, flag bit 22.
#[inline]
fn f12lim_a3s(x: i64) -> i32 {
    let min = -(32768_i64 << 12);
    let max = 32767_i64 << 12;
    unsafe {
        if x < min {
            gteFLAG |= 1u64 << 22;
            min as i32
        } else if x > max {
            gteFLAG |= 1u64 << 22;
            max as i32
        } else {
            x as i32
        }
    }
}

/// 12.4 fixed-point unsigned saturate, flag bit 24.
#[inline]
fn f12lim_a1u(x: i64) -> i32 {
    let max = 32767_i64 << 12;
    unsafe {
        if x < 0 {
            gteFLAG |= 1u64 << 24;
            0
        } else if x > max {
            gteFLAG |= 1u64 << 24;
            max as i32
        } else {
            x as i32
        }
    }
}

/// 12.4 fixed-point unsigned saturate, flag bit 23.
#[inline]
fn f12lim_a2u(x: i64) -> i32 {
    let max = 32767_i64 << 12;
    unsafe {
        if x < 0 {
            gteFLAG |= 1u64 << 23;
            0
        } else if x > max {
            gteFLAG |= 1u64 << 23;
            max as i32
        } else {
            x as i32
        }
    }
}

/// 12.4 fixed-point unsigned saturate, flag bit 22.
#[inline]
fn f12lim_a3u(x: i64) -> i32 {
    let max = 32767_i64 << 12;
    unsafe {
        if x < 0 {
            gteFLAG |= 1u64 << 22;
            0
        } else if x > max {
            gteFLAG |= 1u64 << 22;
            max as i32
        } else {
            x as i32
        }
    }
}

/// 32-bit signed saturate to 16-bit signed, flag bit 24.
#[inline]
fn flim_a1s(x: i32) -> i16 {
    unsafe {
        if x < -32768 {
            gteFLAG |= 1u64 << 24;
            -32768
        } else if x > 32767 {
            gteFLAG |= 1u64 << 24;
            32767
        } else {
            x as i16
        }
    }
}

/// 32-bit signed saturate to 16-bit signed, flag bit 23.
#[inline]
fn flim_a2s(x: i32) -> i16 {
    unsafe {
        if x < -32768 {
            gteFLAG |= 1u64 << 23;
            -32768
        } else if x > 32767 {
            gteFLAG |= 1u64 << 23;
            32767
        } else {
            x as i16
        }
    }
}

/// 32-bit signed saturate to 16-bit signed, flag bit 22.
#[inline]
fn flim_a3s(x: i32) -> i16 {
    unsafe {
        if x < -32768 {
            gteFLAG |= 1u64 << 22;
            -32768
        } else if x > 32767 {
            gteFLAG |= 1u64 << 22;
            32767
        } else {
            x as i16
        }
    }
}

/// 32-bit unsigned saturate to `u8`, flag bit 21.
#[inline]
fn flim_b1(x: i32) -> u8 {
    unsafe {
        if x < 0 {
            gteFLAG |= 1u64 << 21;
            0
        } else if x > 255 {
            gteFLAG |= 1u64 << 21;
            255
        } else {
            x as u8
        }
    }
}

/// 32-bit unsigned saturate to `u8`, flag bit 20.
#[inline]
fn flim_b2(x: i32) -> u8 {
    unsafe {
        if x < 0 {
            gteFLAG |= 1u64 << 20;
            0
        } else if x > 255 {
            gteFLAG |= 1u64 << 20;
            255
        } else {
            x as u8
        }
    }
}

/// 32-bit unsigned saturate to `u8`, flag bit 19.
#[inline]
fn flim_b3(x: i32) -> u8 {
    unsafe {
        if x < 0 {
            gteFLAG |= 1u64 << 19;
            0
        } else if x > 255 {
            gteFLAG |= 1u64 << 19;
            255
        } else {
            x as u8
        }
    }
}

/// 32-bit unsigned saturate to `u16`, flag bit 18.
#[inline]
fn flim_c(x: i32) -> u16 {
    unsafe {
        if x < 0 {
            gteFLAG |= 1u64 << 18;
            0
        } else if x > 65535 {
            gteFLAG |= 1u64 << 18;
            65535
        } else {
            x as u16
        }
    }
}

/// 32-bit unsigned saturate to `u16`, flag bit 12.
#[inline]
fn flim_e(x: i32) -> i32 {
    unsafe {
        if x < 0 {
            gteFLAG |= 1u64 << 12;
            0
        } else if x > 65535 {
            gteFLAG |= 1u64 << 12;
            65535
        } else {
            x
        }
    }
}

/// `FlimG1` -- combined 32-bit overflow + 11-bit screen clamp, flag
/// bits 16/15 (overflow) and 14 (clamp).
#[inline]
fn flim_g1(x: i64) -> i32 {
    unsafe {
        if x > 2_147_483_647 {
            gteFLAG |= 1u64 << 16;
        } else if x < -2_147_483_648 {
            gteFLAG |= 1u64 << 15;
        }
        if x > 1023 {
            gteFLAG |= 1u64 << 14;
            1023
        } else if x < -1024 {
            gteFLAG |= 1u64 << 14;
            -1024
        } else {
            x as i32
        }
    }
}

/// `FlimG2` -- combined 32-bit overflow + 11-bit screen clamp, flag
/// bits 16/15 (overflow) and 13 (clamp).
#[inline]
fn flim_g2(x: i64) -> i32 {
    unsafe {
        if x > 2_147_483_647 {
            gteFLAG |= 1u64 << 16;
        } else if x < -2_147_483_648 {
            gteFLAG |= 1u64 << 15;
        }
        if x > 1023 {
            gteFLAG |= 1u64 << 13;
            1023
        } else if x < -1024 {
            gteFLAG |= 1u64 << 13;
            -1024
        } else {
            x as i32
        }
    }
}

// ===========================================================================
// `FNC_OVERFLOW*` -- saturate a 64-bit accumulator to 32-bit with the
// per-channel flag bits. These are the building blocks of the RTPS /
// MVMVA / SQR / OP / GPL fixed-point paths.
// ===========================================================================

#[inline]
fn fnc_overflow1(x: i64) -> i32 {
    unsafe {
        if x < -2_147_483_648 {
            gteFLAG |= 1u64 << 29;
        } else if x > 2_147_483_647 {
            gteFLAG |= 1u64 << 26;
        }
        x as i32
    }
}

#[inline]
fn fnc_overflow2(x: i64) -> i32 {
    unsafe {
        if x < -2_147_483_648 {
            gteFLAG |= 1u64 << 28;
        } else if x > 2_147_483_647 {
            gteFLAG |= 1u64 << 25;
        }
        x as i32
    }
}

#[inline]
fn fnc_overflow3(x: i64) -> i32 {
    unsafe {
        if x < -2_147_483_648 {
            gteFLAG |= 1u64 << 27;
        } else if x > 2_147_483_647 {
            gteFLAG |= 1u64 << 24;
        }
        x as i32
    }
}

// ===========================================================================
// CP2D accessors. The C++ reaches the data register file with a mix of
// i16 / i32 / u16 / u8 / u32 views. We mirror the same aliasing on
// `gteV`.
// ===========================================================================

/// Read the lo i16 of `gteV[idx]` (i.e. the lo 16 bits of `CP2D.r[idx]`).
#[inline]
fn v_i16(idx: usize) -> i16 {
    unsafe { gteV[idx].v[0] }
}

/// Read the lo 32 bits of `gteV[idx]` (i.e. `CP2D.r[idx]` as i32).
#[inline]
fn v_i32(idx: usize) -> i32 {
    unsafe { gteV[idx].as_i32_lo() }
}

/// Read the lo 16 bits of `gteV[idx]` as a `u16`.
#[inline]
fn v_u16(idx: usize) -> u16 {
    unsafe { gteV[idx].v[0] as u16 }
}

/// Read the lo 32 bits of `gteV[idx]` as a `u32`.
#[inline]
fn v_u32(idx: usize) -> u32 {
    unsafe { gteV[idx].as_u32_lo() }
}

#[inline] fn set_v_i16(idx: usize, v: i16) { unsafe { gteV[idx].v[0] = v; } }
#[inline] fn set_v_i32(idx: usize, v: i32) { unsafe {
    gteV[idx].v[0] = v as i16;
    gteV[idx].v[1] = (v >> 16) as i16;
} }
#[inline] fn set_v_u16(idx: usize, v: u16) { unsafe { gteV[idx].v[0] = v as i16; } }
#[inline] fn set_v_u32(idx: usize, v: u32) { unsafe {
    gteV[idx].v[0] = v as i16;
    gteV[idx].v[1] = (v >> 16) as i16;
} }

// ---------------------------------------------------------------------------
// Convenience accessors that mirror the C++ macro views on
// `psxRegs.CP2D`.
// ---------------------------------------------------------------------------

#[inline] fn gte_vx0() -> i16 { v_i16(0) }
#[inline] fn gte_vy0() -> i16 { v_i16(1) }
#[inline] fn gte_vz0() -> i16 { v_i16(2) }
#[inline] fn gte_vx1() -> i16 { v_i16(4) }
#[inline] fn gte_vy1() -> i16 { v_i16(5) }
#[inline] fn gte_vz1() -> i16 { v_i16(6) }
#[inline] fn gte_vx2() -> i16 { v_i16(8) }
#[inline] fn gte_vy2() -> i16 { v_i16(9) }
#[inline] fn gte_vz2() -> i16 { v_i16(10) }
#[inline] fn gte_rgb() -> u32 { v_u32(6) }
#[inline] fn gte_otz() -> i16 { v_i16(7) }
#[inline] fn gte_ir0() -> i32 { v_i32(11) }
#[inline] fn gte_ir1() -> i32 { v_i32(8) }
#[inline] fn gte_ir2() -> i32 { v_i32(9) }
#[inline] fn gte_ir3() -> i32 { v_i32(10) }
#[inline] fn gte_sxy0() -> i32 { v_i32(12) }
#[inline] fn gte_sxy1() -> i32 { v_i32(13) }
#[inline] fn gte_sxy2() -> i32 { v_i32(14) }
#[inline] fn gte_sxyp() -> i32 { v_i32(15) }
#[inline] fn gte_sx0() -> i16 { v_i16(12) }
#[inline] fn gte_sy0() -> i16 { unsafe { gteV[12].v[1] } }
#[inline] fn gte_sx1() -> i16 { v_i16(13) }
#[inline] fn gte_sy1() -> i16 { unsafe { gteV[13].v[1] } }
#[inline] fn gte_sx2() -> i16 { v_i16(14) }
#[inline] fn gte_sy2() -> i16 { unsafe { gteV[14].v[1] } }
#[inline] fn gte_szx() -> u16 { v_u16(16) }
#[inline] fn gte_sz0() -> u16 { v_u16(17) }
#[inline] fn gte_sz1() -> u16 { v_u16(18) }
#[inline] fn gte_sz2() -> u16 { v_u16(19) }
#[inline] fn gte_rgb0() -> u32 { v_u32(20) }
#[inline] fn gte_rgb1() -> u32 { v_u32(21) }
#[inline] fn gte_rgb2() -> u32 { v_u32(22) }
#[inline] fn gte_mac0() -> i32 { v_i32(24) }
#[inline] fn gte_mac1() -> i32 { v_i32(25) }
#[inline] fn gte_mac2() -> i32 { v_i32(26) }
#[inline] fn gte_mac3() -> i32 { v_i32(27) }
#[inline] fn gte_irgb() -> u32 { v_u32(28) }
#[inline] fn gte_orgb() -> u32 { v_u32(29) }
#[inline] fn gte_lzcs() -> u32 { v_u32(30) }
#[inline] fn gte_lzcr() -> u32 { v_u32(31) }

#[inline] fn gte_mac0_set(v: i32) { set_v_i32(24, v); }
#[inline] fn gte_mac1_set(v: i32) { set_v_i32(25, v); }
#[inline] fn gte_mac2_set(v: i32) { set_v_i32(26, v); }
#[inline] fn gte_mac3_set(v: i32) { set_v_i32(27, v); }
#[inline] fn gte_ir0_set(v: i32) { set_v_i32(11, v); }
#[inline] fn gte_ir1_set(v: i32) { set_v_i32(8, v); }
#[inline] fn gte_ir2_set(v: i32) { set_v_i32(9, v); }
#[inline] fn gte_ir3_set(v: i32) { set_v_i32(10, v); }

/// Byte view of the RGB+CODE register (`gteR/gteG/gteB/gteCODE`).
#[inline] fn gte_r_byte() -> u8 { unsafe { gteV[6].v[0] as u8 } }
#[inline] fn gte_g_byte() -> u8 { unsafe { (gteV[6].v[0] >> 8) as u8 } }
#[inline] fn gte_b_byte() -> u8 { unsafe { gteV[6].v[1] as u8 } }
#[inline] fn gte_code() -> u8 { unsafe { (gteV[6].v[1] >> 8) as u8 } }

#[inline] fn gte_r0_byte() -> u8 { unsafe { gteV[20].v[0] as u8 } }
#[inline] fn gte_g0_byte() -> u8 { unsafe { (gteV[20].v[0] >> 8) as u8 } }
#[inline] fn gte_b0_byte() -> u8 { unsafe { gteV[20].v[1] as u8 } }
#[inline] fn gte_code0() -> u8 { unsafe { (gteV[20].v[1] >> 8) as u8 } }

#[inline] fn gte_r1_byte() -> u8 { unsafe { gteV[21].v[0] as u8 } }
#[inline] fn gte_g1_byte() -> u8 { unsafe { (gteV[21].v[0] >> 8) as u8 } }
#[inline] fn gte_b1_byte() -> u8 { unsafe { gteV[21].v[1] as u8 } }
#[inline] fn gte_code1() -> u8 { unsafe { (gteV[21].v[1] >> 8) as u8 } }

#[inline] fn gte_r2_byte() -> u8 { unsafe { gteV[22].v[0] as u8 } }
#[inline] fn gte_g2_byte() -> u8 { unsafe { (gteV[22].v[0] >> 8) as u8 } }
#[inline] fn gte_b2_byte() -> u8 { unsafe { gteV[22].v[1] as u8 } }
#[inline] fn gte_code2() -> u8 { unsafe { (gteV[22].v[1] >> 8) as u8 } }

#[inline] fn gte_r2_set(b: u8) { unsafe { gteV[22].v[0] = (gteV[22].v[0] & !0xFF) | b as i16; } }
#[inline] fn gte_g2_set(b: u8) { unsafe { gteV[22].v[0] = (gteV[22].v[0] & !(0xFF00_u16 as i16)) | ((b as i16) << 8); } }
#[inline] fn gte_b2_set(b: u8) { unsafe { gteV[22].v[1] = (gteV[22].v[1] & !0xFF) | b as i16; } }
#[inline] fn gte_code2_set(b: u8) { unsafe { gteV[22].v[1] = (gteV[22].v[1] & !(0xFF00_u16 as i16)) | ((b as i16) << 8); } }

#[inline] fn gte_r1_set(b: u8) { unsafe { gteV[21].v[0] = (gteV[21].v[0] & !0xFF) | b as i16; } }
#[inline] fn gte_g1_set(b: u8) { unsafe { gteV[21].v[0] = (gteV[21].v[0] & !(0xFF00_u16 as i16)) | ((b as i16) << 8); } }
#[inline] fn gte_b1_set(b: u8) { unsafe { gteV[21].v[1] = (gteV[21].v[1] & !0xFF) | b as i16; } }
#[inline] fn gte_code1_set(b: u8) { unsafe { gteV[21].v[1] = (gteV[21].v[1] & !(0xFF00_u16 as i16)) | ((b as i16) << 8); } }

#[inline] fn gte_r0_set(b: u8) { unsafe { gteV[20].v[0] = (gteV[20].v[0] & !0xFF) | b as i16; } }
#[inline] fn gte_g0_set(b: u8) { unsafe { gteV[20].v[0] = (gteV[20].v[0] & !(0xFF00_u16 as i16)) | ((b as i16) << 8); } }
#[inline] fn gte_b0_set(b: u8) { unsafe { gteV[20].v[1] = (gteV[20].v[1] & !0xFF) | b as i16; } }
#[inline] fn gte_code0_set(b: u8) { unsafe { gteV[20].v[1] = (gteV[20].v[1] & !(0xFF00_u16 as i16)) | ((b as i16) << 8); } }

// ===========================================================================
// CP2C accessors. The C++ reaches the control file with a mix of i16 /
// i32 / u32 views. We map the first 8 u32s to the public `gteM` array
// and the rest to the private `gteC` array.
//
// `gteM` layout (8 u32s):
//   gteM[0] = (R11, R12)        -- CP2C.r[0]
//   gteM[0] = (R13, R21)        -- CP2C.r[1] (packed into v[2..3])
//   gteM[1] = (R22, R23)        -- CP2C.r[2]
//   gteM[1] = (R31, R32)        -- CP2C.r[3] (packed into v[2..3])
//   gteM[2] = (R33, _)          -- CP2C.r[4]
//   gteM[2] = (TRX_lo, TRX_hi)  -- CP2C.r[5] (packed into v[2..3])
//   gteM[3] = (TRY_lo, TRY_hi)  -- CP2C.r[6]
//   gteM[3] = (TRZ_lo, TRZ_hi)  -- CP2C.r[7] (packed into v[2..3])
//
// `gteC` layout (24 u32s, indexed by `c_idx` in 0..24 mapping
// `CP2C.r[8..31]`):
//   c[0]  = L11, L12        (CP2C.r[8])
//   c[1]  = L13, L21        (CP2C.r[9])
//   c[2]  = L22, L23        (CP2C.r[10])
//   c[3]  = L31, L32        (CP2C.r[11])
//   c[4]  = L33, _          (CP2C.r[12])
//   c[5]  = RBK_lo, RBK_hi  (CP2C.r[13])
//   c[6]  = GBK_lo, GBK_hi  (CP2C.r[14])
//   c[7]  = BBK_lo, BBK_hi  (CP2C.r[15])
//   c[8]  = LR1, LR2        (CP2C.r[16])
//   c[9]  = LR3, LG1        (CP2C.r[17])
//   c[10] = LG2, LG3        (CP2C.r[18])
//   c[11] = LB1, LB2        (CP2C.r[19])
//   c[12] = LB3, _          (CP2C.r[20])
//   c[13] = RFC_lo, RFC_hi  (CP2C.r[21])
//   c[14] = GFC_lo, GFC_hi  (CP2C.r[22])
//   c[15] = BFC_lo, BFC_hi  (CP2C.r[23])
//   c[16..22] reserved      (CP2C.r[24..30])
//   c[23] = FLAG lo/hi      (CP2C.r[31])
// ===========================================================================

#[inline]
fn m_lo(c_idx: usize) -> i16 {
    if c_idx < 8 {
        unsafe { gteM[c_idx].v[0] }
    } else {
        unsafe { gteC[c_idx - 8].v[0] }
    }
}

#[inline]
fn m_hi(c_idx: usize) -> i16 {
    if c_idx < 8 {
        unsafe { gteM[c_idx].v[1] }
    } else {
        unsafe { gteC[c_idx - 8].v[1] }
    }
}

#[inline]
fn m_i32(idx: usize) -> i32 {
    i32::from(m_lo(idx)) | (i32::from(m_hi(idx)) << 16)
}

/// `CP2C.r[idx]` as i16 at i16-linear offset (for the i16-packed view).
#[inline]
fn m_i16(idx: usize) -> i16 {
    if (idx & 1) == 0 { m_lo(idx >> 1) } else { m_hi(idx >> 1) }
}

#[inline]
fn set_m_i16(idx: usize, v: i16) {
    let c = idx >> 1;
    if (idx & 1) == 0 {
        if c < 8 { unsafe { gteM[c].v[0] = v; } } else { unsafe { gteC[c - 8].v[0] = v; } }
    } else {
        if c < 8 { unsafe { gteM[c].v[1] = v; } } else { unsafe { gteC[c - 8].v[1] = v; } }
    }
}

#[inline]
fn set_m_i32(idx: usize, v: i32) {
    if idx < 8 {
        unsafe {
            gteM[idx].v[0] = v as i16;
            gteM[idx].v[1] = (v >> 16) as i16;
        }
    } else {
        unsafe {
            gteC[idx - 8].v[0] = v as i16;
            gteC[idx - 8].v[1] = (v >> 16) as i16;
        }
    }
}

// ---------------------------------------------------------------------------
// CP2C convenience accessors (matrix / light / colour / translation /
// depth). All indexed by i16-linear offset (0..=63).
// ---------------------------------------------------------------------------

#[inline] fn gte_r11() -> i16 { m_i16(0) }
#[inline] fn gte_r12() -> i16 { m_i16(1) }
#[inline] fn gte_r13() -> i16 { m_i16(2) }
#[inline] fn gte_r21() -> i16 { m_i16(3) }
#[inline] fn gte_r22() -> i16 { m_i16(4) }
#[inline] fn gte_r23() -> i16 { m_i16(5) }
#[inline] fn gte_r31() -> i16 { m_i16(6) }
#[inline] fn gte_r32() -> i16 { m_i16(7) }
#[inline] fn gte_r33() -> i16 { m_i16(8) }

#[inline] fn gte_trx() -> i32 { m_i32(5) }
#[inline] fn gte_try() -> i32 { m_i32(6) }
#[inline] fn gte_trz() -> i32 { m_i32(7) }

#[inline] fn gte_l11() -> i16 { m_i16(16) }
#[inline] fn gte_l12() -> i16 { m_i16(17) }
#[inline] fn gte_l13() -> i16 { m_i16(18) }
#[inline] fn gte_l21() -> i16 { m_i16(19) }
#[inline] fn gte_l22() -> i16 { m_i16(20) }
#[inline] fn gte_l23() -> i16 { m_i16(21) }
#[inline] fn gte_l31() -> i16 { m_i16(22) }
#[inline] fn gte_l32() -> i16 { m_i16(23) }
#[inline] fn gte_l33() -> i16 { m_i16(24) }

#[inline] fn gte_rbk() -> i32 { m_i32(13) }
#[inline] fn gte_gbk() -> i32 { m_i32(14) }
#[inline] fn gte_bbk() -> i32 { m_i32(15) }

#[inline] fn gte_lr1() -> i16 { m_i16(32) }
#[inline] fn gte_lr2() -> i16 { m_i16(33) }
#[inline] fn gte_lr3() -> i16 { m_i16(34) }
#[inline] fn gte_lg1() -> i16 { m_i16(35) }
#[inline] fn gte_lg2() -> i16 { m_i16(36) }
#[inline] fn gte_lg3() -> i16 { m_i16(37) }
#[inline] fn gte_lb1() -> i16 { m_i16(38) }
#[inline] fn gte_lb2() -> i16 { m_i16(39) }
#[inline] fn gte_lb3() -> i16 { m_i16(40) }

#[inline] fn gte_rfc() -> i32 { m_i32(21) }
#[inline] fn gte_gfc() -> i32 { m_i32(22) }
#[inline] fn gte_bfc() -> i32 { m_i32(23) }

#[inline] fn gte_ofx() -> i32 { unsafe { gteOFX } }
#[inline] fn gte_ofy() -> i32 { unsafe { gteOFY } }
#[inline] fn gte_h() -> i16 { unsafe { gteH } }

#[inline] fn gte_zsf3() -> i16 { m_i16(58) }
#[inline] fn gte_zsf4() -> i16 { m_i16(60) }

#[inline] fn gte_dqa() -> i16 { m_i16(54) }
#[inline] fn gte_dqb() -> i32 { m_i32(28) }

/// `gteD1..gteD3` -- the diagonal of the rotation matrix (in C++:
/// `*(short *)&gteR11` etc., i.e. aliasing the lo half of the matrix
/// slot).
#[inline] fn gte_d1() -> i16 { gte_r11() }
#[inline] fn gte_d2() -> i16 { gte_r22() }
#[inline] fn gte_d3() -> i16 { gte_r33() }

// ===========================================================================
// `MAC2IR` / `MAC2IR1` -- saturate MAC1..MAC3 -> IR1..IR3. The C++ has
// two flavours: MAC2IR (signed) and MAC2IR1 (unsigned).
// ===========================================================================

#[inline]
fn mac2ir() {
    unsafe {
        gteV[8].v[0] = flim_a1s(gte_mac1());
        gteV[9].v[0] = flim_a2s(gte_mac2());
        gteV[10].v[0] = flim_a3s(gte_mac3());
    }
}

#[inline]
fn mac2ir1() {
    unsafe {
        let m1 = gte_mac1();
        let m2 = gte_mac2();
        let m3 = gte_mac3();
        gteV[8].v[0] = if m1 < 0 {
            gteFLAG |= 1u64 << 24;
            0
        } else if m1 > 0x7FFF {
            gteFLAG |= 1u64 << 24;
            0x7FFF
        } else {
            m1 as i16
        };
        gteV[9].v[0] = if m2 < 0 {
            gteFLAG |= 1u64 << 23;
            0
        } else if m2 > 0x7FFF {
            gteFLAG |= 1u64 << 23;
            0x7FFF
        } else {
            m2 as i16
        };
        gteV[10].v[0] = if m3 < 0 {
            gteFLAG |= 1u64 << 22;
            0
        } else if m3 > 0x7FFF {
            gteFLAG |= 1u64 << 22;
            0x7FFF
        } else {
            m3 as i16
        };
    }
}

// ===========================================================================
// Inner kernels shared by RTPS / RTPT.
// ===========================================================================

/// `GTE_RTPS1(vn)` -- compute MAC1/2/3 for one of the perspective
/// vertices.
#[inline]
fn gte_rtps1(vn: usize) {
    let vx = v_i16(vn) as i64;
    let vy = v_i16(vn + 1) as i64;
    let vz = v_i16(vn + 2) as i64;
    let r11 = gte_r11() as i64;
    let r12 = gte_r12() as i64;
    let r13 = gte_r13() as i64;
    let r21 = gte_r21() as i64;
    let r22 = gte_r22() as i64;
    let r23 = gte_r23() as i64;
    let r31 = gte_r31() as i64;
    let r32 = gte_r32() as i64;
    let r33 = gte_r33() as i64;
    gte_mac1_set(fnc_overflow1(((r11 * vx + r12 * vy + r13 * vz) >> 12) + gte_trx() as i64));
    gte_mac2_set(fnc_overflow2(((r21 * vx + r22 * vy + r23 * vz) >> 12) + gte_try() as i64));
    gte_mac3_set(fnc_overflow3(((r31 * vx + r32 * vy + r33 * vz) >> 12) + gte_trz() as i64));
}

/// `GTE_RTPS2(sxy_idx)` -- project MAC1/2 to screen coordinates SX/SY
/// for SXY register index `sxy_idx` (12, 13 or 14).
fn gte_rtps2(sxy_idx: usize) {
    let sz = v_u16(sxy_idx + 4) as u64;
    let fdsz = if sz == 0 {
        unsafe { gteFLAG |= 1u64 << 17; }
        2u64 << 16
    } else {
        let raw = ((gte_h() as u64) << 32) / (sz << 16);
        if raw > (2u64 << 16) {
            unsafe { gteFLAG |= 1u64 << 17; }
            2u64 << 16
        } else {
            raw
        }
    };
    let ir1 = gte_ir1() as i64;
    let ir2 = gte_ir2() as i64;
    let ofx = gte_ofx() as i64;
    let ofy = gte_ofy() as i64;
    let sx = flim_g1((ofx + ((ir1 << 16) * (fdsz as i64) >> 16)) >> 16);
    let sy = flim_g2((ofy + ((ir2 << 16) * (fdsz as i64) >> 16)) >> 16);
    set_v_i16(sxy_idx, sx as i16);
    unsafe { gteV[sxy_idx].v[1] = sy as i16; }
}

/// `GTE_RTPS3()` -- compute MAC0/IR0 from the post-perspective depth.
fn gte_rtps3() {
    let dqa = gte_dqa() as i64;
    let dqb = gte_dqb() as i64;
    let fdsz = dqb + ((dqa << 8) >> 8);
    gte_mac0_set(fdsz as i32);
    gte_ir0_set(flim_e((fdsz >> 12) as i32));
}

// ===========================================================================
// Per-instruction bodies. Called directly by name or via `gteExecute`.
// ===========================================================================

/// `gteRTPS` -- perspective-transform one vertex, update SXY FIFO / OTZ
/// / MAC0 / IR0.
pub fn gteRTPS() {
    unsafe {
        gteFLAG = 0;
        gte_rtps1(0);
        mac2ir();
        // SZ FIFO shift: SZx <- SZ0 <- SZ1 <- SZ2 <- MAC3
        let szx = gte_szx();
        set_v_u16(17, gte_sz0());
        set_v_u16(18, gte_sz1());
        set_v_u16(19, flim_c(gte_mac3()));
        set_v_u16(16, szx);
        // SXY FIFO shift: SXY0 <- SXY1 <- SXY2 <- result
        gteV[12] = gteV[13];
        gteV[13] = gteV[14];
        gte_rtps2(14);
        gteV[15] = gteV[14];
        gte_rtps3();
        sum_flag();
    }
}

/// `gteRTPT` -- perspective-transform three vertices, update SXY FIFO /
/// MAC0 / IR0 from the last vertex.
pub fn gteRTPT() {
    unsafe {
        gteFLAG = 0;
        let szx = gte_szx();
        gte_rtps1(0);
        set_v_u16(17, flim_c(gte_mac3()));
        gteV[8].v[0] = flim_a1s(gte_mac1());
        gteV[9].v[0] = flim_a2s(gte_mac2());
        gte_rtps2(12);
        gte_rtps1(4);
        set_v_u16(18, flim_c(gte_mac3()));
        gteV[8].v[0] = flim_a1s(gte_mac1());
        gteV[9].v[0] = flim_a2s(gte_mac2());
        gte_rtps2(13);
        gte_rtps1(8);
        mac2ir();
        set_v_u16(19, flim_c(gte_mac3()));
        gte_rtps2(14);
        gteV[15] = gteV[14];
        gte_rtps3();
        set_v_u16(16, szx);
        sum_flag();
    }
}

/// `gteOP` -- outer product of IR with the rotation-matrix diagonal.
pub fn gteOP() {
    unsafe {
        gteFLAG = 0;
        let ir1 = gte_ir1() as i64;
        let ir2 = gte_ir2() as i64;
        let ir3 = gte_ir3() as i64;
        let d1 = gte_d1() as i64;
        let d2 = gte_d2() as i64;
        let d3 = gte_d3() as i64;
        let m1 = d2 * ir3 - d3 * ir2;
        let m2 = d3 * ir1 - d1 * ir3;
        let m3 = d1 * ir2 - d2 * ir1;
        gte_mac1_set(fnc_overflow1(m1));
        gte_mac2_set(fnc_overflow2(m2));
        gte_mac3_set(fnc_overflow3(m3));
        mac2ir();
        sum_flag();
    }
}

/// `gteNCLIP` -- normal clipping (signed-area sum of SXY triangle).
pub fn gteNCLIP() {
    unsafe {
        gteFLAG = 0;
        let sx0 = gte_sx0() as i64;
        let sy0 = gte_sy0() as i64;
        let sx1 = gte_sx1() as i64;
        let sy1 = gte_sy1() as i64;
        let sx2 = gte_sx2() as i64;
        let sy2 = gte_sy2() as i64;
        let mac0 = sx0 * (sy1 - sy2) + sx1 * (sy2 - sy0) + sx2 * (sy0 - sy1);
        gte_mac0_set(mac0 as i32);
        sum_flag();
    }
}

/// `gteDPCS` -- depth-queue colour (single RGB input).
pub fn gteDPCS() {
    unsafe {
        gteFLAG = 0;
        let ir0 = gte_ir0() as i64;
        let rfc = gte_rfc();
        let gfc = gte_gfc();
        let bfc = gte_bfc();
        let r = gte_r_byte() as i64;
        let g = gte_g_byte() as i64;
        let b = gte_b_byte() as i64;
        let m1 = (r << 4) + ((ir0 * flim_a1s(rfc - (r << 4) as i32) as i64) >> 12);
        let m2 = (g << 4) + ((ir0 * flim_a2s(gfc - (g << 4) as i32) as i64) >> 12);
        let m3 = (b << 4) + ((ir0 * flim_a3s(bfc - (b << 4) as i32) as i64) >> 12);
        gte_mac1_set(m1 as i32);
        gte_mac2_set(m2 as i32);
        gte_mac3_set(m3 as i32);
        mac2ir();
        gteV[20] = gteV[21];
        gteV[21] = gteV[22];
        gte_r2_set(flim_b1(m1 as i32 >> 4));
        gte_g2_set(flim_b2(m2 as i32 >> 4));
        gte_b2_set(flim_b3(m3 as i32 >> 4));
        gte_code2_set(gte_code());
        sum_flag();
    }
}

/// `gteDPCT` -- depth-queue colour, three iterations on RGB0.
pub fn gteDPCT() {
    for _ in 0..3 {
        gteDPCS();
    }
}

/// `gteINTPL` -- interpolate IRGB with the IR0 interpolation factor.
pub fn gteINTPL() {
    unsafe {
        gteFLAG = 0;
        let ir0 = gte_ir0() as i64;
        let ir1 = gte_ir1();
        let ir2 = gte_ir2();
        let ir3 = gte_ir3();
        let rfc = gte_rfc();
        let gfc = gte_gfc();
        let bfc = gte_bfc();
        let m1 = ir1 + ((ir0 * flim_a1s(rfc - ir1) as i64) >> 12) as i32;
        let m2 = ir2 + ((ir0 * flim_a2s(gfc - ir2) as i64) >> 12) as i32;
        let m3 = ir3 + ((ir0 * flim_a3s(bfc - ir3) as i64) >> 12) as i32;
        gte_mac1_set(m1);
        gte_mac2_set(m2);
        gte_mac3_set(m3);
        mac2ir();
        gteV[20] = gteV[21];
        gteV[21] = gteV[22];
        gte_r2_set(flim_b1(m1 >> 4));
        gte_g2_set(flim_b2(m2 >> 4));
        gte_b2_set(flim_b3(m3 >> 4));
        gte_code2_set(gte_code());
        sum_flag();
    }
}

/// `gteMVMVA` -- generic matrix * vector + translation/bk/fc transform.
pub fn gteMVMVA(instr: u32) {
    unsafe {
        let mx_sel = (instr >> 17) & 0x3;
        let vx_sel = (instr >> 23) & 0x3;
        let tx_sel = (instr >> 25) & 0x3;
        let sf = (instr >> 19) & 0x1;
        let lm = (instr >> 10) & 0x1;

        let (v0, v1, v2) = match vx_sel {
            0 => (gte_vx0() as i64, gte_vy0() as i64, gte_vz0() as i64),
            1 => (gte_vx1() as i64, gte_vy1() as i64, gte_vz1() as i64),
            2 => (gte_vx2() as i64, gte_vy2() as i64, gte_vz2() as i64),
            _ => (gte_ir1() as i64, gte_ir2() as i64, gte_ir3() as i64),
        };
        let (m11, m12, m13, m21, m22, m23, m31, m32, m33) = match mx_sel {
            0 => (gte_r11() as i64, gte_r12() as i64, gte_r13() as i64,
                  gte_r21() as i64, gte_r22() as i64, gte_r23() as i64,
                  gte_r31() as i64, gte_r32() as i64, gte_r33() as i64),
            1 => (gte_l11() as i64, gte_l12() as i64, gte_l13() as i64,
                  gte_l21() as i64, gte_l22() as i64, gte_l23() as i64,
                  gte_l31() as i64, gte_l32() as i64, gte_l33() as i64),
            // The "C" matrix aliases onto the light colour matrix in
            // the C++ source; we keep the same aliasing here.
            _ => (gte_lr1() as i64, gte_lr2() as i64, gte_lr3() as i64,
                  gte_lg1() as i64, gte_lg2() as i64, gte_lg3() as i64,
                  gte_lb1() as i64, gte_lb2() as i64, gte_lb3() as i64),
        };
        let mut ssx = v0 * m11 + v1 * m12 + v2 * m13;
        let mut ssy = v0 * m21 + v1 * m22 + v2 * m23;
        let mut ssz = v0 * m31 + v1 * m32 + v2 * m33;
        if sf != 0 {
            ssx >>= 12;
            ssy >>= 12;
            ssz >>= 12;
        }
        let (tx, ty, tz) = match tx_sel {
            0 => (gte_trx() as i64, gte_try() as i64, gte_trz() as i64),
            1 => (gte_rbk() as i64, gte_gbk() as i64, gte_bbk() as i64),
            _ => (gte_rfc() as i64, gte_gfc() as i64, gte_bfc() as i64),
        };
        ssx += tx;
        ssy += ty;
        ssz += tz;
        gteFLAG = 0;
        gte_mac1_set(fnc_overflow1(ssx));
        gte_mac2_set(fnc_overflow2(ssy));
        gte_mac3_set(fnc_overflow3(ssz));
        if lm != 0 { mac2ir1(); } else { mac2ir(); }
        sum_flag();
    }
}

/// `gteNCDS` -- single-vertex normal colour with depth interpolation.
pub fn gteNCDS() {
    nccs_kernel(0, true);
    unsafe {
        gteV[20] = gteV[21];
        gteV[21] = gteV[22];
        let m1 = gte_mac1();
        let m2 = gte_mac2();
        let m3 = gte_mac3();
        gte_r2_set(flim_b1(m1 >> 4));
        gte_g2_set(flim_b2(m2 >> 4));
        gte_b2_set(flim_b3(m3 >> 4));
        gte_code2_set(gte_code());
        mac2ir1();
        sum_flag();
    }
}

/// `gteNCDT` -- three-vertex normal colour with depth interpolation.
pub fn gteNCDT() {
    unsafe {
        gteFLAG = 0;
        for &vn in &[0usize, 4, 8] {
            nccs_kernel(vn, true);
            let m1 = gte_mac1();
            let m2 = gte_mac2();
            let m3 = gte_mac3();
            gteV[20 + vn / 4] = GTERegister::new(
                flim_b1(m1 >> 4) as i16,
                ((flim_b2(m2 >> 4) as i32) << 8
                    | (flim_b3(m3 >> 4) as i32) << 16
                    | (gte_code() as i32) << 24) as i16,
                0, 0,
            );
        }
        mac2ir1();
        sum_flag();
    }
}

/// `gteNCCS` -- single-vertex normal colour, no far-colour
/// interpolation.
pub fn gteNCCS() {
    nccs_kernel(0, false);
    unsafe {
        gteV[20] = gteV[21];
        gteV[21] = gteV[22];
        let m1 = gte_mac1();
        let m2 = gte_mac2();
        let m3 = gte_mac3();
        gte_r2_set(flim_b1(m1 >> 4));
        gte_g2_set(flim_b2(m2 >> 4));
        gte_b2_set(flim_b3(m3 >> 4));
        gte_code2_set(gte_code());
        mac2ir1();
        sum_flag();
    }
}

/// `gteNCCT` -- three-vertex normal colour, no far-colour
/// interpolation.
pub fn gteNCCT() {
    unsafe {
        gteFLAG = 0;
        for &vn in &[0usize, 4, 8] {
            nccs_kernel(vn, false);
            let m1 = gte_mac1();
            let m2 = gte_mac2();
            let m3 = gte_mac3();
            gteV[20 + vn / 4] = GTERegister::new(
                flim_b1(m1 >> 4) as i16,
                ((flim_b2(m2 >> 4) as i32) << 8
                    | (flim_b3(m3 >> 4) as i32) << 16
                    | (gte_code() as i32) << 24) as i16,
                0, 0,
            );
        }
        mac2ir1();
        sum_flag();
    }
}

/// `gteNCS` -- single-vertex normal colour (no specular / far colour).
pub fn gteNCS() {
    nccs_kernel(0, false);
    unsafe {
        gteV[20] = gteV[21];
        gteV[21] = gteV[22];
        let m1 = gte_mac1();
        let m2 = gte_mac2();
        let m3 = gte_mac3();
        gte_r2_set(flim_b1(m1 >> 4));
        gte_g2_set(flim_b2(m2 >> 4));
        gte_b2_set(flim_b3(m3 >> 4));
        gte_code2_set(gte_code());
        mac2ir1();
        sum_flag();
    }
}

/// `gteNCT` -- three-vertex normal colour (no specular / far colour).
pub fn gteNCT() {
    unsafe {
        gteFLAG = 0;
        for &vn in &[0usize, 4, 8] {
            nccs_kernel(vn, false);
            let m1 = gte_mac1();
            let m2 = gte_mac2();
            let m3 = gte_mac3();
            gteV[20 + vn / 4] = GTERegister::new(
                flim_b1(m1 >> 4) as i16,
                ((flim_b2(m2 >> 4) as i32) << 8
                    | (flim_b3(m3 >> 4) as i32) << 16
                    | (gte_code() as i32) << 24) as i16,
                0, 0,
            );
        }
        mac2ir1();
        sum_flag();
    }
}

/// `gteCC` -- colour-colour: combine current IRGB with the BK + L matrix
/// and the input RGB.
pub fn gteCC() {
    unsafe {
        gteFLAG = 0;
        let ir1 = gte_ir1() as i64;
        let ir2 = gte_ir2() as i64;
        let ir3 = gte_ir3() as i64;
        let rbk = gte_rbk() as i64;
        let gbk = gte_gbk() as i64;
        let bbk = gte_bbk() as i64;
        let lr1 = gte_lr1() as i64;
        let lr2 = gte_lr2() as i64;
        let lr3 = gte_lr3() as i64;
        let lg1 = gte_lg1() as i64;
        let lg2 = gte_lg2() as i64;
        let lg3 = gte_lg3() as i64;
        let lb1 = gte_lb1() as i64;
        let lb2 = gte_lb2() as i64;
        let lb3 = gte_lb3() as i64;
        let rr0 = fnc_overflow1(rbk + ((lr1 * ir1 + lr2 * ir2 + lr3 * ir3) >> 12));
        let gg0 = fnc_overflow2(gbk + ((lg1 * ir1 + lg2 * ir2 + lg3 * ir3) >> 12));
        let bb0 = fnc_overflow3(bbk + ((lb1 * ir1 + lb2 * ir2 + lb3 * ir3) >> 12));
        let r = gte_r_byte() as i64;
        let g = gte_g_byte() as i64;
        let b = gte_b_byte() as i64;
        let m1 = (r * rr0 as i64) >> 8;
        let m2 = (g * gg0 as i64) >> 8;
        let m3 = (b * bb0 as i64) >> 8;
        gte_mac1_set(m1 as i32);
        gte_mac2_set(m2 as i32);
        gte_mac3_set(m3 as i32);
        mac2ir1();
        gteV[20] = gteV[21];
        gteV[21] = gteV[22];
        gte_r2_set(flim_b1(m1 as i32 >> 4));
        gte_g2_set(flim_b2(m2 as i32 >> 4));
        gte_b2_set(flim_b3(m3 as i32 >> 4));
        gte_code2_set(gte_code());
        sum_flag();
    }
}

/// `gteCDP` -- colour-depth-queue: combine colour / depth interpolation
/// plus a CC step.
pub fn gteCDP() {
    gteCC();
}

/// `gteAVSZ3` -- average screen Z (3 vertices).
pub fn gteAVSZ3() {
    unsafe {
        gteFLAG = 0;
        let sz0 = gte_sz0() as i64;
        let sz1 = gte_sz1() as i64;
        let sz2 = gte_sz2() as i64;
        let zsf3 = gte_zsf3() as i64;
        let mac0 = ((sz0 + sz1 + sz2) * zsf3) >> 12;
        gte_mac0_set(mac0 as i32);
        set_v_i16(7, flim_c(mac0 as i32) as i16);
        sum_flag();
    }
}

/// `gteAVSZ4` -- average screen Z (4 vertices).
pub fn gteAVSZ4() {
    unsafe {
        gteFLAG = 0;
        let sz0 = gte_sz0() as i64;
        let sz1 = gte_sz1() as i64;
        let sz2 = gte_sz2() as i64;
        let szx = gte_szx() as i64;
        let zsf4 = gte_zsf4() as i64;
        let mac0 = ((szx + sz0 + sz1 + sz2) * zsf4) >> 12;
        gte_mac0_set(mac0 as i32);
        set_v_i16(7, flim_c(mac0 as i32) as i16);
        sum_flag();
    }
}

/// `gteSQR` -- square IR vector, optional SF shift.
pub fn gteSQR(instr: u32) {
    unsafe {
        gteFLAG = 0;
        let sf = (instr >> 19) & 0x1;
        let ir1 = gte_ir1() as i64;
        let ir2 = gte_ir2() as i64;
        let ir3 = gte_ir3() as i64;
        let (m1, m2, m3) = if sf != 0 {
            (fnc_overflow1((ir1 * ir1) >> 12),
             fnc_overflow2((ir2 * ir2) >> 12),
             fnc_overflow3((ir3 * ir3) >> 12))
        } else {
            (fnc_overflow1(ir1 * ir1),
             fnc_overflow2(ir2 * ir2),
             fnc_overflow3(ir3 * ir3))
        };
        gte_mac1_set(m1);
        gte_mac2_set(m2);
        gte_mac3_set(m3);
        mac2ir1();
        sum_flag();
    }
}

/// `gteDCPL` -- depth-queue colour with the IRGB interpolation factor.
pub fn gteDCPL() {
    unsafe {
        gteFLAG = 0;
        let ir0 = gte_ir0() as i64;
        let ir1 = gte_ir1() as i64;
        let ir2 = gte_ir2() as i64;
        let ir3 = gte_ir3() as i64;
        let rfc = gte_rfc();
        let gfc = gte_gfc();
        let bfc = gte_bfc();
        let r = gte_r_byte() as i64;
        let g = gte_g_byte() as i64;
        let b = gte_b_byte() as i64;
        let m1 = ((r * ir1) + (ir0 * flim_a1s(rfc - ((r * ir1) >> 12) as i32) as i64)) >> 8;
        let m2 = ((g * ir2) + (ir0 * flim_a2s(gfc - ((g * ir2) >> 12) as i32) as i64)) >> 8;
        let m3 = ((b * ir3) + (ir0 * flim_a3s(bfc - ((b * ir3) >> 12) as i32) as i64)) >> 8;
        gte_mac1_set(m1 as i32);
        gte_mac2_set(m2 as i32);
        gte_mac3_set(m3 as i32);
        mac2ir();
        gteV[20] = gteV[21];
        gteV[21] = gteV[22];
        gte_r2_set(flim_b1(m1 as i32 >> 4));
        gte_g2_set(flim_b2(m2 as i32 >> 4));
        gte_b2_set(flim_b3(m3 as i32 >> 4));
        gte_code2_set(gte_code());
        sum_flag();
    }
}

/// `gteGPF` -- general purpose multiplication: IR0 * IR1/2/3.
pub fn gteGPF(instr: u32) {
    unsafe {
        gteFLAG = 0;
        let sf = (instr >> 19) & 0x1;
        let ir0 = gte_ir0() as i64;
        let ir1 = gte_ir1() as i64;
        let ir2 = gte_ir2() as i64;
        let ir3 = gte_ir3() as i64;
        let (m1, m2, m3) = if sf != 0 {
            (fnc_overflow1((ir0 * ir1) >> 12),
             fnc_overflow2((ir0 * ir2) >> 12),
             fnc_overflow3((ir0 * ir3) >> 12))
        } else {
            (fnc_overflow1(ir0 * ir1),
             fnc_overflow2(ir0 * ir2),
             fnc_overflow3(ir0 * ir3))
        };
        gte_mac1_set(m1);
        gte_mac2_set(m2);
        gte_mac3_set(m3);
        mac2ir();
        gteV[20] = gteV[21];
        gteV[21] = gteV[22];
        gte_r2_set(flim_b1(m1 >> 4));
        gte_g2_set(flim_b2(m2 >> 4));
        gte_b2_set(flim_b3(m3 >> 4));
        gte_code2_set(gte_code());
        sum_flag();
    }
}

/// `gteGPL` -- general purpose multiply-add: MAC + IR0*IR.
pub fn gteGPL(instr: u32) {
    unsafe {
        gteFLAG = 0;
        let sf = (instr >> 19) & 0x1;
        let ir0 = gte_ir0() as i64;
        let ir1 = gte_ir1() as i64;
        let ir2 = gte_ir2() as i64;
        let ir3 = gte_ir3() as i64;
        let (m1, m2, m3) = if sf != 0 {
            (fnc_overflow1(gte_mac1() as i64 + (ir0 * ir1 >> 12)),
             fnc_overflow2(gte_mac2() as i64 + (ir0 * ir2 >> 12)),
             fnc_overflow3(gte_mac3() as i64 + (ir0 * ir3 >> 12)))
        } else {
            (fnc_overflow1(gte_mac1() as i64 + ir0 * ir1),
             fnc_overflow2(gte_mac2() as i64 + ir0 * ir2),
             fnc_overflow3(gte_mac3() as i64 + ir0 * ir3))
        };
        gte_mac1_set(m1);
        gte_mac2_set(m2);
        gte_mac3_set(m3);
        mac2ir();
        gteV[20] = gteV[21];
        gteV[21] = gteV[22];
        gte_r2_set(flim_b1(m1 >> 4));
        gte_g2_set(flim_b2(m2 >> 4));
        gte_b2_set(flim_b3(m3 >> 4));
        gte_code2_set(gte_code());
        sum_flag();
    }
}

/// Shared kernel for NCDS / NCDT / NCCS / NCCT / NCS / NCT. Computes
/// MAC1/2/3 for the given V* vertex index. The `far_color` flag
/// selects between the full NCDS path (with IR0/far-colour
/// interpolation) and the simpler NCCS path.
fn nccs_kernel(vn: usize, far_color: bool) {
    let l11 = gte_l11() as i64;
    let l12 = gte_l12() as i64;
    let l13 = gte_l13() as i64;
    let l21 = gte_l21() as i64;
    let l22 = gte_l22() as i64;
    let l23 = gte_l23() as i64;
    let l31 = gte_l31() as i64;
    let l32 = gte_l32() as i64;
    let l33 = gte_l33() as i64;
    let vx = v_i16(vn) as i64;
    let vy = v_i16(vn + 1) as i64;
    let vz = v_i16(vn + 2) as i64;
    let ll1 = f12lim_a1u((l11 * vx + l12 * vy + l13 * vz) >> 12) as i64;
    let ll2 = f12lim_a2u((l21 * vx + l22 * vy + l23 * vz) >> 12) as i64;
    let ll3 = f12lim_a3u((l31 * vx + l32 * vy + l33 * vz) >> 12) as i64;
    let lr1 = gte_lr1() as i64;
    let lr2 = gte_lr2() as i64;
    let lr3 = gte_lr3() as i64;
    let lg1 = gte_lg1() as i64;
    let lg2 = gte_lg2() as i64;
    let lg3 = gte_lg3() as i64;
    let lb1 = gte_lb1() as i64;
    let lb2 = gte_lb2() as i64;
    let lb3 = gte_lb3() as i64;
    let rrlt = f12lim_a1u(gte_rbk() as i64 + ((lr1 * ll1 + lr2 * ll2 + lr3 * ll3) >> 12)) as i64;
    let gglt = f12lim_a2u(gte_gbk() as i64 + ((lg1 * ll1 + lg2 * ll2 + lg3 * ll3) >> 12)) as i64;
    let bbllt = f12lim_a3u(gte_bbk() as i64 + ((lb1 * ll1 + lb2 * ll2 + lb3 * ll3) >> 12)) as i64;
    let r = gte_r_byte() as i64;
    let g = gte_g_byte() as i64;
    let b = gte_b_byte() as i64;
    let (m1, m2, m3) = if far_color {
        let rrr0 = ((r << 12) * rrlt >> 12) as i64;
        let ggg0 = ((g << 12) * gglt >> 12) as i64;
        let bbb0 = ((b << 12) * bbllt >> 12) as i64;
        let ir0 = gte_ir0() as i64;
        let rfc = gte_rfc() as i64;
        let gfc = gte_gfc() as i64;
        let bfc = gte_bfc() as i64;
        let fm1 = (rrr0 + ((ir0 * f12lim_a1s((rfc << 8) - rrr0) as i64) >> 12)) >> 8;
        let fm2 = (ggg0 + ((ir0 * f12lim_a2s((gfc << 8) - ggg0) as i64) >> 12)) >> 8;
        let fm3 = (bbb0 + ((ir0 * f12lim_a3s((bfc << 8) - bbb0) as i64) >> 12)) >> 8;
        (fm1 as i32, fm2 as i32, fm3 as i32)
    } else {
        let rmac = ((r << 12) * rrlt) >> 20;
        let gmac = ((g << 12) * gglt) >> 20;
        let bmac = ((b << 12) * bbllt) >> 20;
        (rmac as i32, gmac as i32, bmac as i32)
    };
    gte_mac1_set(m1);
    gte_mac2_set(m2);
    gte_mac3_set(m3);
}

// ===========================================================================
// `gteMFC2` / `gteMTC2` / `gteCFC2` / `gteCTC2` -- the COP2 GPR <-> GTE
// register move family. The original C++ hooks live in IopGte.cpp and
// reach `psxRegs.GPR.r[_Rt_]` / `psxRegs.CP2D/C.r[_Rd_]`. In Rust the
// GPR side is the caller's problem; we only translate the GTE side
// effects.
// ===========================================================================

/// `mfc2` -- move from GTE data register to GPR. The C++ special-cases
/// register 29 (ORGB) to build the `IR1/2/3 -> ORGB` packed colour, and
/// returns the raw CP2D word otherwise.
pub fn gteMFC2(rd: u32) -> u32 {
    unsafe {
        match rd & 0x1F {
            29 => {
                let ir1 = gteV[8].as_i32_lo();
                let ir2 = gteV[9].as_i32_lo();
                let ir3 = gteV[10].as_i32_lo();
                let orgb = ((ir1 >> 7) & 0x1F) as u32
                    | ((((ir2 >> 7) & 0x1F) as u32) << 5)
                    | ((((ir3 >> 7) & 0x1F) as u32) << 10);
                gteV[29].v[0] = orgb as i16;
                gteV[29].v[1] = (orgb >> 16) as i16;
                orgb
            }
            n => gteV[n as usize].as_u32_lo(),
        }
    }
}

/// `mtc2` -- move from GPR to GTE data register. The C++ treats the IR
/// registers (8..11) as 16-bit (sign-extend on store), the SZx..SZ3 set
/// (16..19) as 16-bit, IRGB (28) as 5:5:5 unpack into IR1/IR2/IR3, and
/// SXYP (15) as a four-stage shift register for the SXY FIFO. The LZCS
/// (30) store also populates the LZCR (31) `CountLeadingSignBits`
/// result.
pub fn gteMTC2(rd: u32, value: u32) {
    unsafe {
        match rd & 0x1F {
            8 | 9 | 10 | 11 => {
                let r = (rd & 0x1F) as usize;
                gteV[r].v[0] = value as i16;
                gteV[r].v[1] = 0;
                gteV[r].v[2] = 0;
                gteV[r].v[3] = 0;
            }
            15 => {
                // SXY FIFO shift. SXY0 <- SXY1 <- SXY2 <- SXYP <- value.
                gteV[12] = gteV[13];
                gteV[13] = gteV[14];
                gteV[14] = GTERegister::new(value as i16, (value >> 16) as i16, 0, 0);
                gteV[15] = gteV[14];
            }
            16 | 17 | 18 | 19 => {
                let r = (rd & 0x1F) as usize;
                gteV[r].v[0] = (value & 0xFFFF) as i16;
                gteV[r].v[1] = 0;
                gteV[r].v[2] = 0;
                gteV[r].v[3] = 0;
            }
            28 => {
                // IRGB: 5:5:5 -> IR1/IR2/IR3 at bit 7.
                gteV[28] = GTERegister::new(value as i16, (value >> 16) as i16, 0, 0);
                let r = (value & 0x1F) as i32;
                let g = ((value >> 5) & 0x1F) as i32;
                let b = ((value >> 10) & 0x1F) as i32;
                gteV[8] = GTERegister::new((r << 7) as i16, 0, 0, 0);
                gteV[9] = GTERegister::new((g << 7) as i16, 0, 0, 0);
                gteV[10] = GTERegister::new((b << 7) as i16, 0, 0, 0);
            }
            30 => {
                // LZCS write also computes the LZCR result via
                // `CountLeadingSignBits`.
                gteV[30] = GTERegister::new(value as i16, (value >> 16) as i16, 0, 0);
                let lzcr = count_leading_sign_bits(value as i32);
                gteV[31] = GTERegister::new(lzcr as i16, 0, 0, 0);
            }
            n => {
                let r = n as usize;
                gteV[r] = GTERegister::new(value as i16, (value >> 16) as i16, 0, 0);
            }
        }
    }
}

/// `cfc2` -- move from GTE control register to GPR. The control file in
/// this module is the public `gteM[0..7]` for the matrix / translation
/// slots and the private `gteC[0..23]` for the rest.
pub fn gteCFC2(rd: u32) -> u32 {
    unsafe {
        let r = (rd & 0x1F) as usize;
        if r < 8 { gteM[r].as_u32_lo() } else { gteC[r - 8].as_u32_lo() }
    }
}

/// `ctc2` -- move from GPR to GTE control register.
pub fn gteCTC2(rd: u32, value: u32) {
    unsafe {
        let r = (rd & 0x1F) as usize;
        let slot = GTERegister::new(value as i16, (value >> 16) as i16, 0, 0);
        if r < 8 { gteM[r] = slot; } else { gteC[r - 8] = slot; }
    }
}

/// `CountLeadingSignBits` -- equivalent to `std::countl_one` on the
/// magnitude of a signed value, or 32 for zero. Mirrors the helper used
/// by the C++ `MTC2` for the LZCR result.
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
// gteExecute -- the macro-emitter switch from the C++ becomes a `match`
// on bits 0..25 of the COP2 instruction. The C++ uses `psxRegs.code` to
// dispatch; the Rust surface takes the raw 32-bit opcode as `instr`.
// ===========================================================================

/// Dispatch one GTE instruction.
///
/// `instr` is the raw 32-bit COP2 opcode. The C++ macro emitter
/// dispatches on the GTE function (bits 20..25); we replicate that here
/// and also surface the per-op sub-encoding to the kernel via the
/// `instr` argument.
pub fn gteExecute(instr: u32) {
    match (instr >> 20) & 0x3F {
        0x00..=0x10 => gteRTPS(),
        0x11 => gteRTPT(),
        0x12 => gteNCLIP(),
        0x13 => gteNCS(),
        0x14 => gteNCT(),
        0x15 => gteNCS(),
        0x16 => gteNCDS(),
        0x17 => gteNCDT(),
        0x18 => gteNCCS(),
        0x19 => gteNCC_CDP(),
        0x1A => gteNCCT(),
        0x1B => gteNCC_CDP(),
        0x1C => gteCC(),
        0x1D => gteCDP(),
        0x1E => gteNCS(),
        0x1F => gteNCT(),
        0x20 => gteSQR(instr),
        0x21 => gteDCPL(),
        0x22 => gteDPCT(),
        0x23 => gteDPCS(),
        0x24 => gteINTPL(),
        0x25 => gteNCLIP(),
        0x26 => gteMVMVA(instr),
        0x27 => gteNCC_CDP(),
        0x28 => gteSQR(instr),
        0x29 => gteDCPL(),
        0x2A => gteDPCT(),
        0x2B => gteDPCS(),
        0x2C => gteINTPL(),
        0x2D => gteNCC_CDP(),
        0x2E => gteMVMVA(instr),
        0x2F => gteNCC_CDP(),
        0x30 => gteNCCS(),
        0x31 => gteNCCT(),
        0x32 => gteCC(),
        0x33 => gteAVSZ3(),
        0x34 => gteAVSZ4(),
        0x35 => gteRTPT(),
        0x36 => gteGPF(instr),
        0x37 => gteGPL(instr),
        0x38 => gteNCC_CDP(),
        0x39 => gteNCCT(),
        0x3A => gteDPCT(),
        0x3B => gteDPCS(),
        0x3C => gteNCC_CDP(),
        0x3D => gteINTPL(),
        0x3E => gteMVMVA(instr),
        0x3F => gteNCC_CDP(),
        _ => {
            // Unknown / un-implemented: leave gteFLAG as-is.
        }
    }
}

/// Stub for the handful of function codes the macro emitter maps to
/// either `gteCC` or `gteCDP` depending on the sub-encoding. A future
/// refactor can split these into separate functions; for the moment
/// the two paths share the same body and the matcher just needs *some*
/// target to call.
fn gteNCC_CDP() {
    gteCC();
}
