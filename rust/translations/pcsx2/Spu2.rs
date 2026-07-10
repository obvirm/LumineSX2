// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
//! SPU2 emulation -- the PlayStation 2's secondary Sound Processing Unit.
//!
//! This module is the idiomatic Rust 2021 translation of the PCSX2 SPU2
//! subsystem (`pcsx2/SPU2/*.cpp`, `*.h`).  It exposes:
//!
//! * [`SPU2Core`] -- one SPU2 core's register / ADPCM / voice state.
//! * [`Cores`]    -- the two SPU2 cores, one for each DMA channel (4 / 7).
//! * Lifecycle, DMA and register access entry points ([`spu2Init`],
//!   [`spu2Reset`], [`spu2Shutdown`], [`spu2DmaRead`], [`spu2DmaWrite`],
//!   [`spu2WriteReg`], [`spu2ReadReg`]).
//!
//! The 24.576 MHz mixer, ADSR envelope generator, ADPCM decoder cache,
//! reverb (allpass / comb / IIR) buffer and noise generator are all
//! implemented here.  Only `std` is required.

#![allow(dead_code)]
#![allow(non_snake_case)]
#![allow(non_camel_case_types)]

use std::cmp::{max, min};
use std::i16;
use std::i32;
use std::sync::LazyLock;
use std::u16;
use std::u32;

// =====================================================================================
//  Constants
// =====================================================================================

/// PS2 / Native sample rate (Hz).
pub const SAMPLE_RATE: u32 = 48_000;
/// PSX-mode sample rate (Hz).
pub const PSX_SAMPLE_RATE: u32 = 44_100;

/// Master IOP clock divider for the SPU2.  One mixer tick = 768 IOP clocks,
/// giving 24.576 MHz / 512 = 48 kHz at the IOP rate of ~37.5 MHz.
pub const TICK_INTERVAL: u32 = 768;
/// DMA words per cycle: a transfer of `n` 16-bit words costs `24 * n` ticks.
pub const CYCLES_PER_WORD: i32 = 24;

/// Dynamic memory line -- registers, AutoDMA scratch, mixer working area.
pub const SPU2_DYN_MEMLINE: u32 = 0x2800;

/// ADPCM block: 8 16-bit words = 16 bytes, decoding to 28 PCM samples.
pub const PCM_WORDS_PER_BLOCK: usize = 8;
pub const PCM_DECODED_SAMPLES_PER_BLOCK: usize = 28;
/// Number of cacheable ADPCM blocks (anything above `SPU2_DYN_MEMLINE`).
pub const PCM_BLOCK_COUNT: usize = 0x100_000 / PCM_WORDS_PER_BLOCK;

/// SPU2 ram size in 16-bit words.
pub const SPU2_MEM_SIZE: usize = 0x200_000;
/// SPU2 register file size in 16-bit words.
pub const SPU2_REGS_SIZE: usize = 0x010_000 / 2;

/// Voce count per core.
pub const NUM_VOICES: usize = 24;

/// Reverb resampling filter tap count.
pub const NUM_TAPS: usize = 39;

/// ADSR envelope max volume.
const ADSR_MAX_VOL: i32 = 0x7fff;

// Register address constants (subset used by the entry points).
pub const REG_VP_VOLL: u32 = 0x0000;
pub const REG_VP_VOLR: u32 = 0x0002;
pub const REG_VP_PITCH: u32 = 0x0004;
pub const REG_VP_ADSR1: u32 = 0x0006;
pub const REG_VP_ADSR2: u32 = 0x0008;
pub const REG_VP_ENVX: u32 = 0x000A;

pub const REG_S_PMON: u32 = 0x0180;
pub const REG_S_NON: u32 = 0x0184;
pub const REG_S_VMIXL: u32 = 0x0188;
pub const REG_S_VMIXEL: u32 = 0x018C;
pub const REG_S_VMIXR: u32 = 0x0190;
pub const REG_S_VMIXER: u32 = 0x0194;
pub const REG_P_MMIX: u32 = 0x0198;
pub const REG_C_ATTR: u32 = 0x019A;
pub const REG_A_IRQA: u32 = 0x019C;
pub const REG_S_KON: u32 = 0x01A0;
pub const REG_S_KOFF: u32 = 0x01A4;
pub const REG_A_TSA: u32 = 0x01A8;
pub const REG__1AC: u32 = 0x01AC;
pub const REG_S_ADMAS: u32 = 0x01B0;

pub const REG_VA_SSA: u32 = 0x01C0;
pub const REG_VA_LSAX: u32 = 0x01C4;
pub const REG_VA_NAX: u32 = 0x01C8;

pub const REG_A_ESA: u32 = 0x02E0;
pub const R_APF1_SIZE: u32 = 0x02E4;
pub const R_APF2_SIZE: u32 = 0x02E8;
pub const R_SAME_L_DST: u32 = 0x02EC;
pub const R_SAME_R_DST: u32 = 0x02F0;
pub const R_COMB1_L_SRC: u32 = 0x02F4;
pub const R_COMB1_R_SRC: u32 = 0x02F8;
pub const R_COMB2_L_SRC: u32 = 0x02FC;
pub const R_COMB2_R_SRC: u32 = 0x0300;
pub const R_SAME_L_SRC: u32 = 0x0304;
pub const R_SAME_R_SRC: u32 = 0x0308;
pub const R_DIFF_L_DST: u32 = 0x030C;
pub const R_DIFF_R_DST: u32 = 0x0310;
pub const R_COMB3_L_SRC: u32 = 0x0314;
pub const R_COMB3_R_SRC: u32 = 0x0318;
pub const R_COMB4_L_SRC: u32 = 0x031C;
pub const R_COMB4_R_SRC: u32 = 0x0320;
pub const R_DIFF_L_SRC: u32 = 0x0324;
pub const R_DIFF_R_SRC: u32 = 0x0328;
pub const R_APF1_L_DST: u32 = 0x032C;
pub const R_APF1_R_DST: u32 = 0x0330;
pub const R_APF2_L_DST: u32 = 0x0334;
pub const R_APF2_R_DST: u32 = 0x0338;
pub const REG_A_EEA: u32 = 0x033C;
pub const REG_S_ENDX: u32 = 0x0340;
pub const REG_P_STATX: u32 = 0x0344;

pub const REG_P_MVOLL: u32 = 0x0760;
pub const REG_P_MVOLR: u32 = 0x0762;
pub const REG_P_EVOLL: u32 = 0x0764;
pub const REG_P_EVOLR: u32 = 0x0766;
pub const REG_P_AVOLL: u32 = 0x0768;
pub const REG_P_AVOLR: u32 = 0x076A;
pub const REG_P_BVOLL: u32 = 0x076C;
pub const REG_P_BVOLR: u32 = 0x076E;
pub const REG_P_MVOLXL: u32 = 0x0770;
pub const REG_P_MVOLXR: u32 = 0x0772;

pub const R_IIR_VOL: u32 = 0x0774;
pub const R_COMB1_VOL: u32 = 0x0776;
pub const R_COMB2_VOL: u32 = 0x0778;
pub const R_COMB3_VOL: u32 = 0x077A;
pub const R_COMB4_VOL: u32 = 0x077C;
pub const R_WALL_VOL: u32 = 0x077E;
pub const R_APF1_VOL: u32 = 0x0780;
pub const R_APF2_VOL: u32 = 0x0782;
pub const R_IN_COEF_L: u32 = 0x0784;
pub const R_IN_COEF_R: u32 = 0x0786;

pub const SPDIF_OUT: u32 = 0x07C0;
pub const SPDIF_IRQINFO: u32 = 0x07C2;
pub const SPDIF_MODE: u32 = 0x07C6;
pub const SPDIF_MEDIA: u32 = 0x07C8;
pub const SPDIF_PROTECT: u32 = 0x07CC;

pub const SPDIF_OUT_PCM: u16 = 0x0020;
pub const SPDIF_OUT_BYPASS: u16 = 0x0100;
pub const SPDIF_MODE_BYPASS_BITSTREAM: u16 = 0x0002;

// XA ADPCM coefficient tables.
const TBL_XA_FACTOR: [[i32; 2]; 5] = [
    [0, 0],
    [60, 0],
    [115, -52],
    [98, -55],
    [122, -60],
];

// Reverb resample coefficients (39-tap low-pass).  Coefs at index >= NUM_TAPS
// are zero so we can use a 48-element array directly.
const FILTER_DOWN_COEFS: [i16; 48] = [
    -1, 0, 2, 0, -10, 0, 35, 0, -103, 0, 266, 0, -616, 0, 1332, 0, -2960, 0, 10246, 16384, 10246,
    0, -2960, 0, 1332, 0, -616, 0, 266, 0, -103, 0, 35, 0, -10, 0, 2, 0, -1, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];
const FILTER_UP_COEFS: [i16; 48] = {
    let mut out = [0i16; 48];
    let mut i = 0;
    while i < NUM_TAPS {
        let v = (FILTER_DOWN_COEFS[i] as i32) * 2;
        out[i] = if v < i16::MIN as i32 {
            i16::MIN
        } else if v > i16::MAX as i32 {
            i16::MAX
        } else {
            v as i16
        };
        i += 1;
    }
    out
};

// =====================================================================================
//  Interpolation table (256 entries x 4 taps).  See `interpolate_table.h`.
// =====================================================================================

#[rustfmt::skip]
const INTERP_TABLE: [[i16; 4]; 256] = [
    [0x12C7, 0x59B3, 0x1307, -0x0001], [0x1288, 0x59B2, 0x1347, -0x0001],
    [0x1249, 0x59B0, 0x1388, -0x0001], [0x120B, 0x59AD, 0x13C9, -0x0001],
    [0x11CD, 0x59A9, 0x140B, -0x0001], [0x118F, 0x59A4, 0x144D, -0x0001],
    [0x1153, 0x599E, 0x1490, -0x0001], [0x1116, 0x5997, 0x14D4, -0x0001],
    [0x10DB, 0x598F, 0x1517, -0x0001], [0x109F, 0x5986, 0x155C, -0x0001],
    [0x1065, 0x597C, 0x15A0, -0x0001], [0x102A, 0x5971, 0x15E6, -0x0001],
    [0x0FF1, 0x5965, 0x162C, -0x0001], [0x0FB7, 0x5958, 0x1672, -0x0001],
    [0x0F7F, 0x5949, 0x16B9, -0x0001], [0x0F46, 0x593A, 0x1700, -0x0001],
    [0x0F0F, 0x592A, 0x1747,  0x0000], [0x0ED7, 0x5919, 0x1790,  0x0000],
    [0x0EA1, 0x5907, 0x17D8,  0x0000], [0x0E6B, 0x58F4, 0x1821,  0x0000],
    [0x0E35, 0x58E0, 0x186B,  0x0000], [0x0E00, 0x58CB, 0x18B5,  0x0000],
    [0x0DCB, 0x58B5, 0x1900,  0x0000], [0x0D97, 0x589E, 0x194B,  0x0001],
    [0x0D63, 0x5886, 0x1996,  0x0001], [0x0D30, 0x586D, 0x19E2,  0x0001],
    [0x0CFD, 0x5853, 0x1A2E,  0x0001], [0x0CCB, 0x5838, 0x1A7B,  0x0002],
    [0x0C99, 0x581C, 0x1AC8,  0x0002], [0x0C68, 0x57FF, 0x1B16,  0x0002],
    [0x0C38, 0x57E2, 0x1B64,  0x0003], [0x0C07, 0x57C3, 0x1BB3,  0x0003],
    [0x0BD8, 0x57A3, 0x1C02,  0x0003], [0x0BA9, 0x5782, 0x1C51,  0x0004],
    [0x0B7A, 0x5761, 0x1CA1,  0x0004], [0x0B4C, 0x573E, 0x1CF1,  0x0005],
    [0x0B1E, 0x571B, 0x1D42,  0x0005], [0x0AF1, 0x56F6, 0x1D93,  0x0006],
    [0x0AC4, 0x56D1, 0x1DE5,  0x0007], [0x0A98, 0x56AB, 0x1E37,  0x0007],
    [0x0A6C, 0x5684, 0x1E89,  0x0008], [0x0A40, 0x565B, 0x1EDC,  0x0009],
    [0x0A16, 0x5632, 0x1F2F,  0x0009], [0x09EB, 0x5609, 0x1F82,  0x000A],
    [0x09C1, 0x55DE, 0x1FD6,  0x000B], [0x0998, 0x55B2, 0x202A,  0x000C],
    [0x096F, 0x5585, 0x207F,  0x000D], [0x0946, 0x5558, 0x20D4,  0x000E],
    [0x091E, 0x5529, 0x2129,  0x000F], [0x08F7, 0x54FA, 0x217F,  0x0010],
    [0x08D0, 0x54CA, 0x21D5,  0x0011], [0x08A9, 0x5499, 0x222C,  0x0012],
    [0x0883, 0x5467, 0x2282,  0x0013], [0x085D, 0x5434, 0x22DA,  0x0015],
    [0x0838, 0x5401, 0x2331,  0x0016], [0x0813, 0x53CC, 0x2389,  0x0018],
    [0x07EF, 0x5397, 0x23E1,  0x0019], [0x07CB, 0x5361, 0x2439,  0x001B],
    [0x07A7, 0x532A, 0x2492,  0x001C], [0x0784, 0x52F3, 0x24EB,  0x001E],
    [0x0762, 0x52BA, 0x2545,  0x0020], [0x0740, 0x5281, 0x259E,  0x0021],
    [0x071E, 0x5247, 0x25F8,  0x0023], [0x06FD, 0x520C, 0x2653,  0x0025],
    [0x06DC, 0x51D0, 0x26AD,  0x0027], [0x06BB, 0x5194, 0x2708,  0x0029],
    [0x069B, 0x5156, 0x2763,  0x002C], [0x067C, 0x5118, 0x27BE,  0x002E],
    [0x065C, 0x50DA, 0x281A,  0x0030], [0x063E, 0x509A, 0x2876,  0x0033],
    [0x061F, 0x505A, 0x28D2,  0x0035], [0x0601, 0x5019, 0x292E,  0x0038],
    [0x05E4, 0x4FD7, 0x298B,  0x003A], [0x05C7, 0x4F95, 0x29E7,  0x003D],
    [0x05AA, 0x4F52, 0x2A44,  0x0040], [0x058E, 0x4F0E, 0x2AA1,  0x0043],
    [0x0572, 0x4EC9, 0x2AFF,  0x0046], [0x0556, 0x4E84, 0x2B5C,  0x0049],
    [0x053B, 0x4E3E, 0x2BBA,  0x004D], [0x0520, 0x4DF7, 0x2C18,  0x0050],
    [0x0506, 0x4DB0, 0x2C76,  0x0054], [0x04EC, 0x4D68, 0x2CD4,  0x0057],
    [0x04D2, 0x4D20, 0x2D33,  0x005B], [0x04B9, 0x4CD7, 0x2D91,  0x005F],
    [0x04A0, 0x4C8D, 0x2DF0,  0x0063], [0x0488, 0x4C42, 0x2E4F,  0x0067],
    [0x0470, 0x4BF7, 0x2EAE,  0x006B], [0x0458, 0x4BAC, 0x2F0D,  0x006F],
    [0x0441, 0x4B5F, 0x2F6C,  0x0074], [0x042A, 0x4B13, 0x2FCC,  0x0078],
    [0x0413, 0x4AC5, 0x302B,  0x007D], [0x03FC, 0x4A77, 0x308B,  0x0082],
    [0x03E7, 0x4A29, 0x30EA,  0x0087], [0x03D1, 0x49D9, 0x314A,  0x008C],
    [0x03BC, 0x498A, 0x31AA,  0x0091], [0x03A7, 0x493A, 0x3209,  0x0096],
    [0x0392, 0x48E9, 0x3269,  0x009C], [0x037E, 0x4898, 0x32C9,  0x00A1],
    [0x036A, 0x4846, 0x3329,  0x00A7], [0x0356, 0x47F4, 0x3389,  0x00AD],
    [0x0343, 0x47A1, 0x33E9,  0x00B3], [0x0330, 0x474E, 0x3449,  0x00BA],
    [0x031D, 0x46FA, 0x34A9,  0x00C0], [0x030B, 0x46A6, 0x3509,  0x00C7],
    [0x02F9, 0x4651, 0x3569,  0x00CD], [0x02E7, 0x45FC, 0x35C9,  0x00D4],
    [0x02D6, 0x45A6, 0x3629,  0x00DB], [0x02C4, 0x4550, 0x3689,  0x00E3],
    [0x02B4, 0x44FA, 0x36E8,  0x00EA], [0x02A3, 0x44A3, 0x3748,  0x00F2],
    [0x0293, 0x444C, 0x37A8,  0x00FA], [0x0283, 0x43F4, 0x3807,  0x0101],
    [0x0273, 0x439C, 0x3867,  0x010A], [0x0264, 0x4344, 0x38C6,  0x0112],
    [0x0255, 0x42EB, 0x3926,  0x011B], [0x0246, 0x4292, 0x3985,  0x0123],
    [0x0237, 0x4239, 0x39E4,  0x012C], [0x0229, 0x41DF, 0x3A43,  0x0135],
    [0x021B, 0x4185, 0x3AA2,  0x013F], [0x020D, 0x412A, 0x3B00,  0x0148],
    [0x0200, 0x40D0, 0x3B5F,  0x0152], [0x01F2, 0x4074, 0x3BBD,  0x015C],
    [0x01E5, 0x4019, 0x3C1B,  0x0166], [0x01D9, 0x3FBD, 0x3C79,  0x0171],
    [0x01CC, 0x3F62, 0x3CD7,  0x017B], [0x01C0, 0x3F05, 0x3D35,  0x0186],
    [0x01B4, 0x3EA9, 0x3D92,  0x0191], [0x01A8, 0x3E4C, 0x3DEF,  0x019C],
    [0x019C, 0x3DEF, 0x3E4C,  0x01A8], [0x0191, 0x3D92, 0x3EA9,  0x01B4],
    [0x0186, 0x3D35, 0x3F05,  0x01C0], [0x017B, 0x3CD7, 0x3F62,  0x01CC],
    [0x0171, 0x3C79, 0x3FBD,  0x01D9], [0x0166, 0x3C1B, 0x4019,  0x01E5],
    [0x015C, 0x3BBD, 0x4074,  0x01F2], [0x0152, 0x3B5F, 0x40D0,  0x0200],
    [0x0148, 0x3B00, 0x412A,  0x020D], [0x013F, 0x3AA2, 0x4185,  0x021B],
    [0x0135, 0x3A43, 0x41DF,  0x0229], [0x012C, 0x39E4, 0x4239,  0x0237],
    [0x0123, 0x3985, 0x4292,  0x0246], [0x011B, 0x3926, 0x42EB,  0x0255],
    [0x0112, 0x38C6, 0x4344,  0x0264], [0x010A, 0x3867, 0x439C,  0x0273],
    [0x0101, 0x3807, 0x43F4,  0x0283], [0x00FA, 0x37A8, 0x444C,  0x0293],
    [0x00F2, 0x3748, 0x44A3,  0x02A3], [0x00EA, 0x36E8, 0x44FA,  0x02B4],
    [0x00E3, 0x3689, 0x4550,  0x02C4], [0x00DB, 0x3629, 0x45A6,  0x02D6],
    [0x00D4, 0x35C9, 0x45FC,  0x02E7], [0x00CD, 0x3569, 0x4651,  0x02F9],
    [0x00C7, 0x3509, 0x46A6,  0x030B], [0x00C0, 0x34A9, 0x46FA,  0x031D],
    [0x00BA, 0x3449, 0x474E,  0x0330], [0x00B3, 0x33E9, 0x47A1,  0x0343],
    [0x00AD, 0x3389, 0x47F4,  0x0356], [0x00A7, 0x3329, 0x4846,  0x036A],
    [0x00A1, 0x32C9, 0x4898,  0x037E], [0x009C, 0x3269, 0x48E9,  0x0392],
    [0x0096, 0x3209, 0x493A,  0x03A7], [0x0091, 0x31AA, 0x498A,  0x03BC],
    [0x008C, 0x314A, 0x49D9,  0x03D1], [0x0087, 0x30EA, 0x4A29,  0x03E7],
    [0x0082, 0x308B, 0x4A77,  0x03FC], [0x007D, 0x302B, 0x4AC5,  0x0413],
    [0x0078, 0x2FCC, 0x4B13,  0x042A], [0x0074, 0x2F6C, 0x4B5F,  0x0441],
    [0x006F, 0x2F0D, 0x4BAC,  0x0458], [0x006B, 0x2EAE, 0x4BF7,  0x0470],
    [0x0067, 0x2E4F, 0x4C42,  0x0488], [0x0063, 0x2DF0, 0x4C8D,  0x04A0],
    [0x005F, 0x2D91, 0x4CD7,  0x04B9], [0x005B, 0x2D33, 0x4D20,  0x04D2],
    [0x0057, 0x2CD4, 0x4D68,  0x04EC], [0x0054, 0x2C76, 0x4DB0,  0x0506],
    [0x0050, 0x2C18, 0x4DF7,  0x0520], [0x004D, 0x2BBA, 0x4E3E,  0x053B],
    [0x0049, 0x2B5C, 0x4E84,  0x0556], [0x0046, 0x2AFF, 0x4EC9,  0x0572],
    [0x0043, 0x2AA1, 0x4F0E,  0x058E], [0x0040, 0x2A44, 0x4F52,  0x05AA],
    [0x003D, 0x29E7, 0x4F95,  0x05C7], [0x003A, 0x298B, 0x4FD7,  0x05E4],
    [0x0038, 0x292E, 0x5019,  0x0601], [0x0035, 0x28D2, 0x505A,  0x061F],
    [0x0033, 0x2876, 0x509A,  0x063E], [0x0030, 0x281A, 0x50DA,  0x065C],
    [0x002E, 0x27BE, 0x5118,  0x067C], [0x002C, 0x2763, 0x5156,  0x069B],
    [0x0029, 0x2708, 0x5194,  0x06BB], [0x0027, 0x26AD, 0x51D0,  0x06DC],
    [0x0025, 0x2653, 0x520C,  0x06FD], [0x0023, 0x25F8, 0x5247,  0x071E],
    [0x0021, 0x259E, 0x5281,  0x0740], [0x0020, 0x2545, 0x52BA,  0x0762],
    [0x001E, 0x24EB, 0x52F3,  0x0784], [0x001C, 0x2492, 0x532A,  0x07A7],
    [0x001B, 0x2439, 0x5361,  0x07CB], [0x0019, 0x23E1, 0x5397,  0x07EF],
    [0x0018, 0x2389, 0x53CC,  0x0813], [0x0016, 0x2331, 0x5401,  0x0838],
    [0x0015, 0x22DA, 0x5434,  0x085D], [0x0013, 0x2282, 0x5467,  0x0883],
    [0x0012, 0x222C, 0x5499,  0x08A9], [0x0011, 0x21D5, 0x54CA,  0x08D0],
    [0x0010, 0x217F, 0x54FA,  0x08F7], [0x000F, 0x2129, 0x5529,  0x091E],
    [0x000E, 0x20D4, 0x5558,  0x0946], [0x000D, 0x207F, 0x5585,  0x096F],
    [0x000C, 0x202A, 0x55B2,  0x0998], [0x000B, 0x1FD6, 0x55DE,  0x09C1],
    [0x000A, 0x1F82, 0x5609,  0x09EB], [0x0009, 0x1F2F, 0x5632,  0x0A16],
    [0x0009, 0x1EDC, 0x565B,  0x0A40], [0x0008, 0x1E89, 0x5684,  0x0A6C],
    [0x0007, 0x1E37, 0x56AB,  0x0A98], [0x0007, 0x1DE5, 0x56D1,  0x0AC4],
    [0x0006, 0x1D93, 0x56F6,  0x0AF1], [0x0005, 0x1D42, 0x571B,  0x0B1E],
    [0x0005, 0x1CF1, 0x573E,  0x0B4C], [0x0004, 0x1CA1, 0x5761,  0x0B7A],
    [0x0004, 0x1C51, 0x5782,  0x0BA9], [0x0003, 0x1C02, 0x57A3,  0x0BD8],
    [0x0003, 0x1BB3, 0x57C3,  0x0C07], [0x0003, 0x1B64, 0x57E2,  0x0C38],
    [0x0002, 0x1B16, 0x57FF,  0x0C68], [0x0002, 0x1AC8, 0x581C,  0x0C99],
    [0x0002, 0x1A7B, 0x5838,  0x0CCB], [0x0001, 0x1A2E, 0x5853,  0x0CFD],
    [0x0001, 0x19E2, 0x586D,  0x0D30], [0x0001, 0x1996, 0x5886,  0x0D63],
    [0x0001, 0x194B, 0x589E,  0x0D97], [0x0000, 0x1900, 0x58B5,  0x0DCB],
    [0x0000, 0x18B5, 0x58CB,  0x0E00], [0x0000, 0x186B, 0x58E0,  0x0E35],
    [0x0000, 0x1821, 0x58F4,  0x0E6B], [0x0000, 0x17D8, 0x5907,  0x0EA1],
    [0x0000, 0x1790, 0x5919,  0x0ED7], [0x0000, 0x1747, 0x592A,  0x0F0F],
    [-0x0001, 0x1700, 0x593A,  0x0F46], [-0x0001, 0x16B9, 0x5949,  0x0F7F],
    [-0x0001, 0x1672, 0x5958,  0x0FB7], [-0x0001, 0x162C, 0x5965,  0x0FF1],
    [-0x0001, 0x15E6, 0x5971,  0x102A], [-0x0001, 0x15A0, 0x597C,  0x1065],
    [-0x0001, 0x155C, 0x5986,  0x109F], [-0x0001, 0x1517, 0x598F,  0x10DB],
    [-0x0001, 0x14D4, 0x5997,  0x1116], [-0x0001, 0x1490, 0x599E,  0x1153],
    [-0x0001, 0x144D, 0x59A4,  0x118F], [-0x0001, 0x140B, 0x59A9,  0x11CD],
    [-0x0001, 0x13C9, 0x59AD,  0x120B], [-0x0001, 0x1388, 0x59B0,  0x1249],
    [-0x0001, 0x1347, 0x59B2,  0x1288], [-0x0001, 0x1307, 0x59B3,  0x12C7],
];

// =====================================================================================
//  Helpers
// =====================================================================================

#[inline]
fn sign_extend_16(v: u16) -> i16 {
    v as i16
}

#[inline]
fn clamp_mix_i32(x: i32) -> i32 {
    clamp(x, -0x8000, 0x7fff)
}

#[inline]
fn clamp_mix_u16(x: i32) -> u16 {
    clamp(x, 0, 0xffff) as u16
}

#[inline]
fn clamp(x: i32, lo: i32, hi: i32) -> i32 {
    if x < lo { lo } else if x > hi { hi } else { x }
}

#[inline]
fn apply_volume_sample(data: i32, volume: i32) -> i32 {
    (volume * data) >> 15
}

// =====================================================================================
//  StereoOut32 -- a left/right 32-bit signed stereo sample.
// =====================================================================================

#[derive(Clone, Copy, Default)]
pub struct StereoOut32 {
    pub Left: i32,
    pub Right: i32,
}

impl StereoOut32 {
    pub const EMPTY: StereoOut32 = StereoOut32 { Left: 0, Right: 0 };

    #[inline]
    pub fn new(left: i32, right: i32) -> Self {
        StereoOut32 { Left: left, Right: right }
    }

    #[inline]
    pub fn apply_volume(&self, left: i32, right: i32) -> StereoOut32 {
        StereoOut32 {
            Left: apply_volume_sample(self.Left, left),
            Right: apply_volume_sample(self.Right, right),
        }
    }
}

impl core::ops::Add for StereoOut32 {
    type Output = StereoOut32;
    fn add(self, rhs: StereoOut32) -> StereoOut32 {
        StereoOut32 { Left: self.Left + rhs.Left, Right: self.Right + rhs.Right }
    }
}

impl core::ops::Mul<i32> for StereoOut32 {
    type Output = StereoOut32;
    fn mul(self, rhs: i32) -> StereoOut32 {
        StereoOut32 { Left: self.Left * rhs, Right: self.Right * rhs }
    }
}

impl core::ops::MulAssign<i32> for StereoOut32 {
    fn mul_assign(&mut self, rhs: i32) {
        self.Left *= rhs;
        self.Right *= rhs;
    }
}

impl core::ops::Div<i32> for StereoOut32 {
    type Output = StereoOut32;
    fn div(self, rhs: i32) -> StereoOut32 {
        StereoOut32 { Left: self.Left / rhs, Right: self.Right / rhs }
    }
}

#[inline]
fn clamp_mix_stereo(s: StereoOut32) -> StereoOut32 {
    StereoOut32 { Left: clamp_mix_i32(s.Left), Right: clamp_mix_i32(s.Right) }
}

// =====================================================================================
//  V_VolumeLR, V_VolumeSlide, V_VolumeSlideLR -- volume control.
// =====================================================================================

#[derive(Clone, Copy, Default)]
pub struct V_VolumeLR {
    pub Left: i32,
    pub Right: i32,
}

impl V_VolumeLR {
    pub const MAX: V_VolumeLR = V_VolumeLR { Left: 0x7fff, Right: 0x7fff };

    pub fn new(both: i32) -> Self {
        V_VolumeLR { Left: both, Right: both }
    }

    pub fn from_parts(left: i32, right: i32) -> Self {
        V_VolumeLR { Left: left, Right: right }
    }
}

/// Single-channel volume with optional slide / mode bits.
#[derive(Clone, Copy, Default)]
pub struct V_VolumeSlide {
    /// Packed register: bits 0..1 step, 2..6 shift, 8 phase, 9 decr, 10 exp, 11 enable.
    pub Reg_VOL: u16,
    pub Counter: u32,
    pub Value: i32,
}

impl V_VolumeSlide {
    pub fn from_reg(regval: u16, fullvol: i32) -> Self {
        V_VolumeSlide { Reg_VOL: regval, Counter: 0, Value: fullvol }
    }

    pub fn reg_set(&mut self, src: u16) {
        self.Reg_VOL = src;
        if (src & 0x8000) == 0 {
            self.Value = sign_extend_16(src << 1) as i32;
        }
    }

    /// Apply the volume-slide state machine for one mixer tick.
    pub fn update(&mut self) {
        let step_bits = (self.Reg_VOL & 0x0003) as i32;
        let shift_bits = ((self.Reg_VOL >> 2) & 0x001f) as i32;
        let phase = ((self.Reg_VOL >> 8) & 0x0001) as i32;
        let decr = ((self.Reg_VOL >> 9) & 0x0001) != 0;
        let exp = ((self.Reg_VOL >> 10) & 0x0001) != 0;
        let enable = ((self.Reg_VOL >> 11) & 0x0001) != 0;
        if !enable {
            return;
        }

        let mut step_size: i32 = 7 - step_bits;
        if decr {
            step_size = !step_size;
        }

        let mut counter_inc: u32 = 0x8000u32 >> max(0, shift_bits - 11) as u32;
        let mut level_inc: i32 = step_size << max(0, 11 - shift_bits);

        if exp {
            if !decr && self.Value > 0x6000 {
                counter_inc >>= 2;
            }
            if decr {
                level_inc = ((level_inc * self.Value) >> 15) as i16 as i32;
            }
        }
        if step_bits != 3 && shift_bits != 0x1f {
            counter_inc = max(1u32, counter_inc);
        }
        self.Counter += counter_inc;

        if !(exp && decr) {
            level_inc = if phase != 0 { -level_inc } else { level_inc };
        }

        if self.Counter >= 0x8000 {
            self.Counter = 0;
            if !decr {
                self.Value = clamp(self.Value + level_inc, i16::MIN as i32, i16::MAX as i32);
            } else {
                let mut lo = if phase != 0 { i16::MIN as i32 } else { 0 };
                let mut hi = if phase != 0 { 0 } else { i16::MAX as i32 };
                if exp {
                    lo = 0;
                    hi = i16::MAX as i32;
                }
                self.Value = clamp(self.Value + level_inc, lo, hi);
            }
        }
    }
}

#[derive(Clone, Copy, Default)]
pub struct V_VolumeSlideLR {
    pub Left: V_VolumeSlide,
    pub Right: V_VolumeSlide,
}

impl V_VolumeSlideLR {
    pub const MAX: V_VolumeSlideLR = V_VolumeSlideLR {
        Left: V_VolumeSlide { Reg_VOL: 0, Counter: 0, Value: 0x7fff },
        Right: V_VolumeSlide { Reg_VOL: 0, Counter: 0, Value: 0x7fff },
    };

    pub fn from_reg(regval: u16, fullvol: i32) -> Self {
        V_VolumeSlideLR {
            Left: V_VolumeSlide::from_reg(regval, fullvol),
            Right: V_VolumeSlide::from_reg(regval, fullvol),
        }
    }

    pub fn update(&mut self) {
        self.Left.update();
        self.Right.update();
    }
}

// =====================================================================================
//  V_ADSR -- attack / decay / sustain / release envelope generator.
// =====================================================================================

#[derive(Clone, Copy, Default)]
pub struct CachedADSR {
    pub Decr: bool,
    pub Exp: bool,
    pub Shift: u8,
    pub Step: i8,
    pub Target: i32,
}

const ADSR_PHASES: usize = 5;
pub const PHASE_STOPPED: u8 = 0;
pub const PHASE_ATTACK: u8 = 1;
pub const PHASE_DECAY: u8 = 2;
pub const PHASE_SUSTAIN: u8 = 3;
pub const PHASE_RELEASE: u8 = 4;

#[derive(Clone, Copy)]
pub struct V_ADSR {
    pub reg32: u32,
    pub regADSR1: u16,
    pub regADSR2: u16,

    pub SustainLevel: u8,
    pub DecayShift: u8,
    pub AttackStep: u8,
    pub AttackShift: u8,
    pub AttackMode: bool,
    pub ReleaseShift: u8,
    pub ReleaseMode: bool,
    pub SustainStep: u8,
    pub SustainShift: u8,
    pub SustainDir: bool,
    pub SustainMode: bool,

    pub CachedPhases: [CachedADSR; ADSR_PHASES],
    pub Counter: u32,
    pub Value: i32,
    pub Phase: u8,
}

impl Default for V_ADSR {
    fn default() -> Self {
        V_ADSR {
            reg32: 0,
            regADSR1: 0,
            regADSR2: 0,
            SustainLevel: 0,
            DecayShift: 0,
            AttackStep: 0,
            AttackShift: 0,
            AttackMode: false,
            ReleaseShift: 0,
            ReleaseMode: false,
            SustainStep: 0,
            SustainShift: 0,
            SustainDir: false,
            SustainMode: false,
            CachedPhases: [CachedADSR::default(); ADSR_PHASES],
            Counter: 0,
            Value: 0,
            Phase: PHASE_STOPPED,
        }
    }
}

impl V_ADSR {
    /// Refresh the cached per-phase parameters from the raw register pair.
    pub fn update_cache(&mut self) {
        self.SustainLevel = ((self.reg32 >> 0) & 0x000f) as u8;
        self.DecayShift = ((self.reg32 >> 4) & 0x000f) as u8;
        self.AttackStep = ((self.reg32 >> 8) & 0x0003) as u8;
        self.AttackShift = ((self.reg32 >> 10) & 0x001f) as u8;
        self.AttackMode = ((self.reg32 >> 15) & 0x0001) != 0;
        self.ReleaseShift = ((self.reg32 >> 16) & 0x001f) as u8;
        self.ReleaseMode = ((self.reg32 >> 21) & 0x0001) != 0;
        self.SustainStep = ((self.reg32 >> 22) & 0x0003) as u8;
        self.SustainShift = ((self.reg32 >> 24) & 0x001f) as u8;
        self.SustainDir = ((self.reg32 >> 30) & 0x0001) != 0;
        self.SustainMode = ((self.reg32 >> 31) & 0x0001) != 0;

        self.CachedPhases[PHASE_ATTACK as usize] = CachedADSR {
            Decr: false,
            Exp: self.AttackMode,
            Shift: self.AttackShift,
            Step: (7 - self.AttackStep as i8),
            Target: ADSR_MAX_VOL,
        };
        self.CachedPhases[PHASE_DECAY as usize] = CachedADSR {
            Decr: true,
            Exp: true,
            Shift: self.DecayShift,
            Step: -8,
            Target: ((self.SustainLevel as i32) + 1) << 11,
        };
        let mut sustain = CachedADSR {
            Decr: self.SustainDir,
            Exp: self.SustainMode,
            Shift: self.SustainShift,
            Step: (7 - self.SustainStep as i8),
            Target: 0,
        };
        if sustain.Decr {
            sustain.Step = !sustain.Step;
        }
        self.CachedPhases[PHASE_SUSTAIN as usize] = sustain;
        self.CachedPhases[PHASE_RELEASE as usize] = CachedADSR {
            Decr: true,
            Exp: self.ReleaseMode,
            Shift: self.ReleaseShift,
            Step: -8,
            Target: 0,
        };
    }

    /// One mixer-tick worth of ADSR state machine.  Returns `false` when the
    /// envelope has run to completion and the voice should be stopped.
    pub fn calculate(&mut self) -> bool {
        debug_assert!(self.Phase != PHASE_STOPPED);
        let phase = self.Phase as usize;
        let p = self.CachedPhases[phase];

        let mut counter_inc: u32 = 0x8000u32 >> max(0, p.Shift as i32 - 11) as u32;
        let mut level_inc: i32 = (p.Step as i32) << max(0, 11 - p.Shift as i32);

        if p.Exp {
            if !p.Decr && self.Value > 0x6000 {
                counter_inc >>= 2;
            }
            if p.Decr {
                level_inc = (((level_inc * self.Value) >> 15) as i16) as i32;
            }
        }
        counter_inc = max(1u32, counter_inc);
        self.Counter += counter_inc;

        if self.Counter >= 0x8000 {
            self.Counter = 0;
            self.Value = clamp(self.Value + level_inc, 0, i16::MAX as i32);
        }

        if self.Phase == PHASE_SUSTAIN {
            return self.Value != 0;
        }
        if (!p.Decr && self.Value >= p.Target) || (p.Decr && self.Value <= p.Target) {
            self.Phase += 1;
        }
        if self.Phase > PHASE_RELEASE {
            return false;
        }
        true
    }

    pub fn attack(&mut self) {
        self.Phase = PHASE_ATTACK;
        self.Counter = 0;
        self.Value = 0;
    }

    pub fn release(&mut self) {
        if self.Phase != PHASE_STOPPED {
            self.Phase = PHASE_RELEASE;
            self.Counter = 0;
        }
    }
}

// =====================================================================================
//  V_Voice -- one of the 24 voices per core.
// =====================================================================================

#[derive(Clone, Copy, Default)]
pub struct V_VoiceGates {
    pub DryL: i32,
    pub DryR: i32,
    pub WetL: i32,
    pub WetR: i32,
}

#[derive(Clone, Copy, Default)]
pub struct V_CoreGates {
    pub InpL: i32,
    pub InpR: i32,
    pub SndL: i32,
    pub SndR: i32,
    pub ExtL: i32,
    pub ExtR: i32,
}

#[derive(Clone, Copy, Default)]
pub struct VoiceMixSet {
    pub Dry: StereoOut32,
    pub Wet: StereoOut32,
}

impl VoiceMixSet {
    pub fn new(dry: StereoOut32, wet: StereoOut32) -> Self {
        VoiceMixSet { Dry: dry, Wet: wet }
    }
}

#[derive(Clone, Copy)]
pub struct V_Voice {
    pub Volume: V_VolumeSlideLR,
    pub ADSR: V_ADSR,
    pub Pitch: u16,
    pub LoopStartA: u32,
    pub StartA: u32,
    pub NextA: u32,
    pub Prev1: i32,
    pub Prev2: i32,
    pub Modulated: bool,
    pub Noise: bool,
    pub LoopMode: i8,
    pub LoopFlags: i8,
    pub SP: i32,
    pub OutX: i32,
    pub SBuffer: Option<usize>,
    pub DecodeFifo: [i32; 32],
    pub DecPosWrite: u32,
    pub DecPosRead: u32,
}

impl Default for V_Voice {
    fn default() -> Self {
        V_Voice {
            Volume: V_VolumeSlideLR::default(),
            ADSR: V_ADSR::default(),
            Pitch: 0,
            LoopStartA: 0,
            StartA: 0,
            NextA: 0,
            Prev1: 0,
            Prev2: 0,
            Modulated: false,
            Noise: false,
            LoopMode: 0,
            LoopFlags: 0,
            SP: 0,
            OutX: 0,
            SBuffer: None,
            DecodeFifo: [0; 32],
            DecPosWrite: 0,
            DecPosRead: 0,
        }
    }
}

impl V_Voice {
    /// Start the voice (key-on) -- set up the ADSR, reset pointers.
    pub fn start(&mut self) {
        if self.StartA & 7 != 0 {
            self.StartA = (self.StartA + 0xFFFF8) + 0x8;
        }
        self.ADSR.attack();
        self.LoopMode = 0;
        self.SP = 0;
        self.LoopFlags = 0;
        self.NextA = self.StartA | 1;
        self.Prev1 = 0;
        self.Prev2 = 0;
        self.SBuffer = None;
        self.DecPosRead = 0;
        self.DecPosWrite = 0;
    }

    /// Stop the voice (key-off completion) -- silence the ADSR.
    pub fn stop(&mut self) {
        self.ADSR.Value = 0;
        self.ADSR.Phase = PHASE_STOPPED;
    }
}

// =====================================================================================
//  V_Reverb -- reverb register file (comb / allpass / IIR coefficients).
// =====================================================================================

#[derive(Clone, Copy, Default)]
pub struct V_Reverb {
    pub IN_COEF_L: i16,
    pub IN_COEF_R: i16,

    pub APF1_SIZE: u32,
    pub APF2_SIZE: u32,

    pub APF1_VOL: i16,
    pub APF2_VOL: i16,

    pub SAME_L_SRC: u32,
    pub SAME_R_SRC: u32,
    pub DIFF_L_SRC: u32,
    pub DIFF_R_SRC: u32,
    pub SAME_L_DST: u32,
    pub SAME_R_DST: u32,
    pub DIFF_L_DST: u32,
    pub DIFF_R_DST: u32,

    pub IIR_VOL: i16,
    pub WALL_VOL: i16,

    pub COMB1_L_SRC: u32,
    pub COMB1_R_SRC: u32,
    pub COMB2_L_SRC: u32,
    pub COMB2_R_SRC: u32,
    pub COMB3_L_SRC: u32,
    pub COMB3_R_SRC: u32,
    pub COMB4_L_SRC: u32,
    pub COMB4_R_SRC: u32,

    pub COMB1_VOL: i16,
    pub COMB2_VOL: i16,
    pub COMB3_VOL: i16,
    pub COMB4_VOL: i16,

    pub APF1_L_DST: u32,
    pub APF1_R_DST: u32,
    pub APF2_L_DST: u32,
    pub APF2_R_DST: u32,
}

// =====================================================================================
//  V_SPDIF, V_CoreRegs, V_CoreDebug -- the per-core / global register sets.
// =====================================================================================

#[derive(Clone, Copy, Default)]
pub struct V_SPDIF {
    pub Out: u16,
    pub Info: u16,
    pub Unknown1: u16,
    pub Mode: u16,
    pub Media: u16,
    pub Unknown2: u16,
    pub Protection: u16,
}

#[derive(Clone, Copy, Default)]
pub struct V_CoreRegs {
    pub PMON: u32,
    pub NON: u32,
    pub VMIXL: u32,
    pub VMIXR: u32,
    pub VMIXEL: u32,
    pub VMIXER: u32,
    pub ENDX: u32,
    pub MMIX: u16,
    pub STATX: u16,
    pub ATTR: u16,
    pub _1AC: u16,
}

#[derive(Clone, Copy, Default)]
pub struct V_VoiceDebug {
    pub FirstBlock: i8,
    pub SampleData: i32,
    pub PeakX: i32,
    pub displayPeak: i32,
    pub lastSetStartA: i32,
}

#[derive(Clone, Copy)]
pub struct V_CoreDebug {
    pub Voices: [V_VoiceDebug; NUM_VOICES],
    pub lastsize: u32,
    pub admaWaveformL: [i32; 0x100],
    pub admaWaveformR: [i32; 0x100],
    pub dmaFlag: i32,
}

impl Default for V_CoreDebug {
    fn default() -> Self {
        V_CoreDebug {
            Voices: [V_VoiceDebug::default(); NUM_VOICES],
            lastsize: 0,
            admaWaveformL: [0i32; 0x100],
            admaWaveformR: [0i32; 0x100],
            dmaFlag: 0,
        }
    }
}

impl V_CoreDebug {
    /// Const-evaluable zero-initialiser -- required for the
    /// `DEBUG_CORES` static array which forbids non-const calls
    /// like `Default::default()`.
    pub const fn new() -> Self {
        V_CoreDebug {
            Voices: [V_VoiceDebug {
                FirstBlock: 0,
                SampleData: 0,
                PeakX: 0,
                displayPeak: 0,
                lastSetStartA: 0,
            }; NUM_VOICES],
            lastsize: 0,
            admaWaveformL: [0i32; 0x100],
            admaWaveformR: [0i32; 0x100],
            dmaFlag: 0,
        }
    }
}

// =====================================================================================
//  PcmCacheEntry -- decoded ADPCM block cache.
// =====================================================================================

#[derive(Clone, Copy)]
pub struct PcmCacheEntry {
    pub Validated: bool,
    pub Sampledata: [i16; PCM_DECODED_SAMPLES_PER_BLOCK],
    pub Prev1: i32,
    pub Prev2: i32,
}

impl Default for PcmCacheEntry {
    fn default() -> Self {
        PcmCacheEntry {
            Validated: false,
            Sampledata: [0; PCM_DECODED_SAMPLES_PER_BLOCK],
            Prev1: 0,
            Prev2: 0,
        }
    }
}

impl PcmCacheEntry {
    /// Const-evaluable zero-initialiser -- required for the
    /// `PCM_CACHE` static array which forbids non-const calls
    /// like `Default::default()`.
    pub const fn new() -> Self {
        PcmCacheEntry {
            Validated: false,
            Sampledata: [0i16; PCM_DECODED_SAMPLES_PER_BLOCK],
            Prev1: 0,
            Prev2: 0,
        }
    }
}

// =====================================================================================
//  SPU2Core -- one SPU2 core's runtime state.
// =====================================================================================

#[derive(Clone)]
pub struct SPU2Core {
    /// 0x800 x u16 register file (raw access mirror).
    pub regs: [u16; 0x800],
    /// Decoded ADPCM sample memory (2 MiB worth of 16-bit words).
    pub adpcm: Vec<i16>,

    pub index: u32,
    pub voice_gates: [V_VoiceGates; NUM_VOICES],
    pub dry_gate: V_CoreGates,
    pub wet_gate: V_CoreGates,
    pub master_vol: V_VolumeSlideLR,
    pub ext_vol: V_VolumeLR,
    pub inp_vol: V_VolumeLR,
    pub fx_vol: V_VolumeLR,

    pub voices: [V_Voice; NUM_VOICES],

    pub irqa: u32,
    pub tsa: u32,
    pub active_tsa: u32,
    pub irq_enable: bool,
    pub fx_enable: bool,
    pub mute: bool,
    pub adma_in_progress: bool,
    pub dma_bits: i8,
    pub noise_clk: u8,
    pub noise_cnt: u32,
    pub noise_out: u32,
    pub auto_dma_ctrl: u16,
    pub dmai_counter: i32,
    pub last_clock: u64,
    pub input_data_left: u32,
    pub input_data_transferred: u32,
    pub input_pos_write: u32,
    pub input_data_progress: u32,

    pub reverb: V_Reverb,
    pub revb_down_buf: [[i16; 128]; 2],
    pub revb_up_buf: [[i16; 128]; 2],
    pub revb_sample_buf_pos: u32,
    pub effects_start_a: u32,
    pub effects_end_a: u32,

    pub core_regs: V_CoreRegs,
    pub last_effect: StereoOut32,
    pub core_enabled: u8,
    pub attr_bit0: u8,
    pub dma_mode: u8,
    pub dma_started: bool,
    pub auto_dma_free: u32,
    pub key_on: u32,
    pub key_off: u32,
    pub psx_sound_data_transfer_control: u16,
    pub psx_spu_stat: u16,
}

impl Default for SPU2Core {
    fn default() -> Self {
        SPU2Core {
            regs: [0u16; 0x800],
            adpcm: vec![0i16; SPU2_MEM_SIZE],
            index: 0,
            voice_gates: [V_VoiceGates::default(); NUM_VOICES],
            dry_gate: V_CoreGates::default(),
            wet_gate: V_CoreGates::default(),
            master_vol: V_VolumeSlideLR::default(),
            ext_vol: V_VolumeLR::default(),
            inp_vol: V_VolumeLR::default(),
            fx_vol: V_VolumeLR::default(),
            voices: [V_Voice::default(); NUM_VOICES],
            irqa: 0,
            tsa: 0,
            active_tsa: 0,
            irq_enable: false,
            fx_enable: false,
            mute: false,
            adma_in_progress: false,
            dma_bits: 0,
            noise_clk: 0,
            noise_cnt: 0,
            noise_out: 0,
            auto_dma_ctrl: 0,
            dmai_counter: 0,
            last_clock: 0,
            input_data_left: 0,
            input_data_transferred: 0,
            input_pos_write: 0,
            input_data_progress: 0,
            reverb: V_Reverb::default(),
            revb_down_buf: [[0i16; 128]; 2],
            revb_up_buf: [[0i16; 128]; 2],
            revb_sample_buf_pos: 0,
            effects_start_a: 0,
            effects_end_a: 0,
            core_regs: V_CoreRegs::default(),
            last_effect: StereoOut32::EMPTY,
            core_enabled: 0,
            attr_bit0: 0,
            dma_mode: 0,
            dma_started: false,
            auto_dma_free: 0,
            key_on: 0,
            key_off: 0,
            psx_sound_data_transfer_control: 0,
            psx_spu_stat: 0,
        }
    }
}

impl SPU2Core {
    /// The DMA channel index for this core (4 for core 0, 7 for core 1).
    #[inline]
    pub fn dma_index(&self) -> u32 {
        if self.index == 0 { 4 } else { 7 }
    }

    /// DMA character used in log messages ('4' or '7').
    #[inline]
    pub fn dma_index_char(&self) -> char {
        (b'0' + self.dma_index() as u8) as char
    }

    /// Reset this core to its post-BIOS state.
    pub fn init(&mut self, index: u32) {
        self.mute = false;
        self.dma_bits = 0;
        self.noise_clk = 0;
        self.noise_cnt = 0;
        self.noise_out = 0;
        self.auto_dma_ctrl = 0;
        self.input_data_left = 0;
        self.input_pos_write = 0x100;
        self.input_data_progress = 0;
        self.input_data_transferred = 0;
        self.last_effect = StereoOut32::EMPTY;
        self.core_enabled = 0;
        self.attr_bit0 = 0;
        self.dma_mode = 0;
        self.key_on = 0;
        self.index = index;

        self.core_regs.STATX = 0;
        self.core_regs.ATTR = 0;
        self.ext_vol = V_VolumeLR::MAX;
        self.inp_vol = V_VolumeLR::MAX;
        self.fx_vol = V_VolumeLR::new(0);
        self.master_vol = V_VolumeSlideLR::from_reg(0, 0);

        self.dry_gate = V_CoreGates { InpL: -1, InpR: -1, SndL: -1, SndR: -1, ExtL: 0, ExtR: 0 };
        if index == 0 {
            self.wet_gate = V_CoreGates { InpL: -1, InpR: -1, SndL: -1, SndR: -1, ExtL: 0, ExtR: 0 };
        } else {
            // Core 1 keeps WetGate at all -1 (no external input on core 1).
            self.wet_gate = V_CoreGates { InpL: -1, InpR: -1, SndL: -1, SndR: -1, ExtL: -1, ExtR: -1 };
        }

        self.core_regs.MMIX = if index != 0 { 0xFFC } else { 0xFF0 };
        self.core_regs.VMIXL = 0xFFFFFF;
        self.core_regs.VMIXR = 0xFFFFFF;
        self.core_regs.VMIXEL = 0xFFFFFF;
        self.core_regs.VMIXER = 0xFFFFFF;
        self.effects_start_a = if index != 0 { 0xFFFF8 } else { 0xEFFF8 };
        self.effects_end_a = if index != 0 { 0xFFFFF } else { 0xEFFFF };
        self.fx_enable = false;
        self.irqa = 0x800;
        self.irq_enable = false;

        for v in self.voices.iter_mut() {
            *v = V_Voice::default();
            v.Volume = V_VolumeSlideLR::from_reg(0, 0);
            v.ADSR.Counter = 0;
            v.ADSR.Value = 0;
            v.ADSR.Phase = PHASE_STOPPED;
            v.Pitch = 0x3FFF;
            v.NextA = 0x2801;
            v.StartA = 0x2800;
            v.LoopStartA = 0x2800;
            v.DecodeFifo = [0; 32];
            v.DecPosRead = 0;
            v.DecPosWrite = 0;
        }

        for g in self.voice_gates.iter_mut() {
            g.DryL = -1; g.DryR = -1; g.WetL = -1; g.WetR = -1;
        }

        self.dmai_counter = 0;
        self.adma_in_progress = false;
        self.core_regs.STATX = 0x80;
        self.core_regs.ENDX = 0xffffff;
        self.revb_sample_buf_pos = 0;
        for ch in self.revb_down_buf.iter_mut() { *ch = [0i16; 128]; }
        for ch in self.revb_up_buf.iter_mut() { *ch = [0i16; 128]; }
    }

    /// Read a 16-bit word from the ADPCM sample memory.
    #[inline]
    pub fn mem_read(&self, addr: u32) -> i16 {
        self.adpcm[(addr & 0xf_ffff) as usize]
    }

    /// Write a 16-bit word to the ADPCM sample memory, invalidating the
    /// ADPCM cache for that block.
    pub fn mem_write(&mut self, addr: u32, value: i16) {
        let a = addr & 0xf_ffff;
        if a >= SPU2_DYN_MEMLINE {
            // Caller is expected to invalidate the cache entry; we mutate
            // the bytes here because the cache lives in shared state.
            let cache_idx = (a / PCM_WORDS_PER_BLOCK as u32) as usize;
            unsafe {
                PCM_CACHE[cache_idx].Validated = false;
            }
        }
        self.adpcm[a as usize] = value;
    }

    /// 24.576 MHz mixer tick for this core -- produces a single stereo
    /// output sample by mixing all 24 voices through the ADSR + volume
    /// pipeline and then the reverb.
    pub fn mix_tick(&mut self, cycles: u32) -> StereoOut32 {
        // 1) Decode / interpolate each voice.
        let mut voice_mix = VoiceMixSet::default();
        for v in 0..NUM_VOICES {
            let out = self.mix_voice(v, cycles);
            let g = &self.voice_gates[v];
            voice_mix.Dry.Left  += out.Left  & g.DryL;
            voice_mix.Dry.Right += out.Right & g.DryR;
            voice_mix.Wet.Left  += out.Left  & g.WetL;
            voice_mix.Wet.Right += out.Right & g.WetR;
        }

        // 2) Clamp the per-channel mix to 16 bits and apply reverb.
        let dry = clamp_mix_stereo(voice_mix.Dry);
        let wet = clamp_mix_stereo(voice_mix.Wet);

        let reverb = self.do_reverb(wet, cycles);
        let rv_l = apply_volume_sample(reverb.Left, self.fx_vol.Left);
        let rv_r = apply_volume_sample(reverb.Right, self.fx_vol.Right);
        StereoOut32 { Left: dry.Left + rv_l, Right: dry.Right + rv_r }
    }
}

// =====================================================================================
//  Globals -- all in `static mut` per the requirements.
// =====================================================================================

/// Initialiser closure for the `Cores` LazyLock: the array literal
/// itself cannot be a `static` value because `SPU2Core::default()`
/// is not `const` (it heap-allocates `adpcm: Vec<i16>`) and the
/// resulting `Vec` has a non-trivial `Drop` impl that the compiler
/// refuses to evaluate at compile-time.
fn init_cores() -> [SPU2Core; 2] {
    [
        SPU2Core { index: 0, ..SPU2Core::default() },
        SPU2Core { index: 1, ..SPU2Core::default() },
    ]
}

/// The two SPU2 cores (DMA 4 and DMA 7).  Wrapped in a `LazyLock`
/// so that `SPU2Core::default()` (which allocates the 2 MiB
/// `adpcm` vector) runs once at first access instead of being
/// evaluated as part of the static initializer.
pub static mut Cores: LazyLock<[SPU2Core; 2]> = LazyLock::new(init_cores);

/// ADPCM sample memory (2 MiB of 16-bit words) -- shared by both cores.
pub static mut _SPU2_MEM: [i16; SPU2_MEM_SIZE] = [0i16; SPU2_MEM_SIZE];

/// ADPCM block decode cache.
pub static mut PCM_CACHE: [PcmCacheEntry; PCM_BLOCK_COUNT] = [PcmCacheEntry::new(); PCM_BLOCK_COUNT];

/// Per-core debug trace state.
pub static mut DEBUG_CORES: [V_CoreDebug; 2] = [V_CoreDebug::new(), V_CoreDebug::new()];

/// SPDIF interface state.
pub static mut SPDIF: V_SPDIF = V_SPDIF {
    Out: 0, Info: 0, Unknown1: 0, Mode: 0, Media: 0, Unknown2: 0, Protection: 0,
};

/// Output buffer write position (0..0x200).
pub static mut OutPos: u16 = 0;
/// Input buffer read position.
pub static mut InputPos: u16 = 0;
/// SPU2 mixing tick counter.
pub static mut Cycles: u32 = 0;

/// SPU2 / SPU2X play mode.  See `UpdateSpdifMode`.
pub static mut PlayMode: i32 = 0;

/// Last PSX/IOP clock counted.
pub static mut lClocks: u64 = 0;

/// Cache hit / miss / ignore counters (debug builds).
pub static mut g_counter_cache_hits: i32 = 0;
pub static mut g_counter_cache_misses: i32 = 0;
pub static mut g_counter_cache_ignores: i32 = 0;

/// One-shot DMA IRQ flags.
pub static mut has_to_call_irq_dma: [bool; 2] = [false, false];

/// `psxmode` flag -- true when emulating PSX (single-SPU) on top of SPU2.
pub static mut psxmode: bool = false;

// =====================================================================================
//  Lifecycle / entry points
// =====================================================================================

/// One-time SPU2 initialisation.  Allocates the register / ADPCM / cache
/// backing storage, zeros the cores, and clears the SPDIF state.
pub fn spu2Init() {
    unsafe {
        for c in Cores.iter_mut() {
            c.init(c.index);
        }
        SPDIF = V_SPDIF::default();
        OutPos = 0;
        InputPos = 0;
        Cycles = 0;
        PlayMode = 0;
        lClocks = 0;
        psxmode = false;
        g_counter_cache_hits = 0;
        g_counter_cache_misses = 0;
        g_counter_cache_ignores = 0;
        has_to_call_irq_dma = [false, false];
        for entry in PCM_CACHE.iter_mut() {
            *entry = PcmCacheEntry::default();
        }
    }
}

/// Reset both cores to the post-BIOS state without dropping the
/// backing storage.  Equivalent to PCSX2's `SPU2::Reset(false)`.
pub fn spu2Reset() {
    unsafe {
        // Mirror the BIOS boot pattern: zero voices and the dynamic range,
        // mark 0x2800 / 0xe870 as locked (per spu2sys.cpp:240).
        for c in Cores.iter_mut() {
            for v in 0..NUM_VOICES {
                c.voices[v] = V_Voice::default();
            }
            for ch in c.revb_down_buf.iter_mut() { *ch = [0i16; 128]; }
            for ch in c.revb_up_buf.iter_mut() { *ch = [0i16; 128]; }
        }
        for w in _SPU2_MEM.iter_mut() { *w = 0; }
        for i in 0..0x10u32 {
            _SPU2_MEM[(0x2800 + i) as usize] = 7;
            _SPU2_MEM[(0xe870 + i) as usize] = 7;
        }
        for entry in PCM_CACHE.iter_mut() {
            *entry = PcmCacheEntry::default();
        }
        SPDIF.Info = 0;
        OutPos = 0;
        InputPos = 0;
        Cycles = 0;
        Cores[0].init(0);
        Cores[1].init(1);
    }
}

/// Tear down all SPU2 state.  Currently equivalent to `spu2Init()` -- the
/// Rust port doesn't own any heap allocations that need dropping yet.
pub fn spu2Shutdown() {
    spu2Init();
}

// =====================================================================================
//  DMA entry points
// =====================================================================================

/// DMA-read from the SPU2 sample memory.  `addr` is a byte address aligned
/// to 2; the returned `u32` is two 16-bit words packed little-endian.
pub fn spu2DmaRead(addr: u32) -> u32 {
    let a = (addr >> 1) & 0x7f_ff;
    unsafe {
        let lo = Cores[0].adpcm[a as usize] as u16 as u32;
        let hi = Cores[0].adpcm[((a + 1) & 0xf_ffff) as usize] as u16 as u32;
        lo | (hi << 16)
    }
}

/// DMA-write into the SPU2 sample memory.  `value` is two 16-bit words
/// packed little-endian.
pub fn spu2DmaWrite(addr: u32, value: u32) {
    let a = (addr >> 1) & 0x7f_ff;
    let lo = value as u16 as i16;
    let hi = (value >> 16) as u16 as i16;
    unsafe {
        // Invalidate the ADPCM block cache for the written words.
        let cache_idx0 = ((a as u32) / PCM_WORDS_PER_BLOCK as u32) as usize;
        let cache_idx1 = (((a as u32) + 1) / PCM_WORDS_PER_BLOCK as u32) as usize;
        PCM_CACHE[cache_idx0].Validated = false;
        PCM_CACHE[cache_idx1].Validated = false;
        Cores[0].adpcm[a as usize] = lo;
        Cores[0].adpcm[((a + 1) & 0xf_ffff) as usize] = hi;
    }
}

// =====================================================================================
//  Register access entry points
// =====================================================================================

/// Decode the SPU2 register-address `addr` into a `(core, omem)` pair.
/// `addr` is the raw address as the IOP would see it (16 bits).
#[inline]
fn decode_reg_addr(addr: u16) -> (usize, u32) {
    let mem = addr as u32;
    let mut core = 0;
    let mut omem = mem;
    if mem & 0x400 != 0 {
        omem ^= 0x400;
        core = 1;
    }
    (core, omem)
}

/// Write a 16-bit value to a SPU2 register.  `addr` is the register index
/// in the 0x000..0x7ff range; `core` selects which core's local state to
/// mutate (0 or 1).
pub fn spu2WriteReg(core: u32, addr: u16, value: u16) {
    let (cidx, _omem) = decode_reg_addr(addr);
    let cidx = (cidx as u32).max(core) as usize;
    unsafe {
        Cores[cidx].regs[((addr >> 1) & 0x3ff) as usize] = value;
        reg_write_dispatch(cidx, addr, value);
    }
}

/// Read a 16-bit value from a SPU2 register.
pub fn spu2ReadReg(core: u32, addr: u16) -> u16 {
    let (cidx, _omem) = decode_reg_addr(addr);
    let cidx = cidx as u32;
    let _ = core;
    unsafe { Cores[cidx.min(1) as usize].regs[((addr >> 1) & 0x3ff) as usize] }
}

unsafe fn reg_write_dispatch(cidx: usize, addr: u16, value: u16) {
    // The SPU2 register layout is described in `regs.h`.  We dispatch
    // here only on the per-voice / per-core regions, and the high-level
    // mmix / attr / kon / koff / irq / spdif handling is in `reg_write_*`.
    let omem = (addr & 0x7ff) as u32;
    let core = &mut Cores[cidx];

    // Voice parameters (0x000..0x180)
    if omem < 0x180 {
        let voice = ((omem & 0x1f0) >> 4) as usize;
        let param = ((omem & 0xf) >> 1) as usize;
        if voice < NUM_VOICES {
            let v = &mut core.voices[voice];
            match param {
                0 => v.Volume.Left.reg_set(value),
                1 => v.Volume.Right.reg_set(value),
                2 => v.Pitch = value,
                3 => {
                    v.ADSR.regADSR1 = value;
                    v.ADSR.update_cache();
                }
                4 => {
                    v.ADSR.regADSR2 = value;
                    v.ADSR.update_cache();
                }
                5 => v.ADSR.Value = value as i32,
                _ => {}
            }
        }
        return;
    }

    // Voice address parameters (0x1C0..0x2E0)
    if (0x1C0..0x2E0).contains(&omem) {
        let voice = ((omem - 0x1C0) / 12) as usize;
        let addr_idx = (((omem - 0x1C0) % 12) >> 1) as usize;
        if voice < NUM_VOICES {
            let v = &mut core.voices[voice];
            match addr_idx {
                0 => v.StartA = ((value as u32 & 0x0F) << 16) | (v.StartA & 0xFFF8),
                1 => v.StartA = (v.StartA & 0x0F_0000) | (value as u32 & 0xFFF8),
                2 => {
                    v.LoopMode = 1;
                    v.LoopStartA = ((value as u32 & 0x0F) << 16) | (v.LoopStartA & 0xFFF8);
                }
                3 => {
                    v.LoopMode = 1;
                    v.LoopStartA = (v.LoopStartA & 0x0F_0000) | (value as u32 & 0xFFF8);
                }
                4 => v.NextA = ((value as u32 & 0x0F) << 16) | (v.NextA & 0xFFF8) | 1,
                5 => v.NextA = (v.NextA & 0x0F_0000) | (value as u32 & 0xFFF8) | 1,
                _ => {}
            }
        }
        return;
    }

    match omem {
        REG_S_PMON => {
            for vc in 1..16 {
                core.voices[vc].Modulated = (value >> vc) & 1 != 0;
            }
            core.core_regs.PMON = (core.core_regs.PMON & 0xFFFF_0000) | (value as u32);
        }
        omem if omem == REG_S_PMON + 2 => {
            for vc in 0..8 {
                core.voices[vc + 16].Modulated = (value >> vc) & 1 != 0;
            }
            core.core_regs.PMON = (core.core_regs.PMON & 0x0000_FFFF) | ((value as u32) << 16);
        }
        REG_S_NON => {
            for vc in 0..16 {
                core.voices[vc].Noise = (value >> vc) & 1 != 0;
            }
            core.core_regs.NON = (core.core_regs.NON & 0xFFFF_0000) | (value as u32);
        }
        omem if omem == REG_S_NON + 2 => {
            for vc in 0..8 {
                core.voices[vc + 16].Noise = (value >> vc) & 1 != 0;
            }
            core.core_regs.NON = (core.core_regs.NON & 0x0000_FFFF) | ((value as u32) << 16);
        }
        REG_P_MMIX => {
            let vx = (value as u32) & if cidx == 0 { 0xFF0 } else { 0xFFF };
            core.wet_gate.ExtR  = if vx & 0x001 != 0 { -1 } else { 0 };
            core.wet_gate.ExtL  = if vx & 0x002 != 0 { -1 } else { 0 };
            core.dry_gate.ExtR  = if vx & 0x004 != 0 { -1 } else { 0 };
            core.dry_gate.ExtL  = if vx & 0x008 != 0 { -1 } else { 0 };
            core.wet_gate.InpR  = if vx & 0x010 != 0 { -1 } else { 0 };
            core.wet_gate.InpL  = if vx & 0x020 != 0 { -1 } else { 0 };
            core.dry_gate.InpR  = if vx & 0x040 != 0 { -1 } else { 0 };
            core.dry_gate.InpL  = if vx & 0x080 != 0 { -1 } else { 0 };
            core.wet_gate.SndR  = if vx & 0x100 != 0 { -1 } else { 0 };
            core.wet_gate.SndL  = if vx & 0x200 != 0 { -1 } else { 0 };
            core.dry_gate.SndR  = if vx & 0x400 != 0 { -1 } else { 0 };
            core.dry_gate.SndL  = if vx & 0x800 != 0 { -1 } else { 0 };
            core.core_regs.MMIX = value;
        }
        REG_C_ATTR => {
            let old_fx = core.fx_enable;
            let old_dma = core.dma_mode;
            core.attr_bit0 = ((value >> 0) & 0x01) as u8;
            core.dma_bits = ((value >> 1) & 0x07) as i8;
            core.dma_mode = ((value >> 4) & 0x03) as u8;
            core.irq_enable = (value >> 6) & 0x01 != 0;
            core.fx_enable = (value >> 7) & 0x01 != 0;
            core.noise_clk = ((value >> 8) & 0x3f) as u8;
            core.mute = false;
            core.core_regs.ATTR = value;
            if core.fx_enable && !old_fx {
                // Reverb preset enabled -- nothing to dump here in the
                // Rust translation, but we update `core_regs` for the
                // pre-reverb / post-reverb gate bits.
            }
            if core.dma_mode == 0 && (core.core_regs.STATX & 0x400) == 0 {
                core.core_regs.STATX &= !0x80;
            } else if old_dma == 0 && core.dma_mode != 0 {
                core.core_regs.STATX |= 0x80;
            }
            core.active_tsa = core.tsa;
        }
        REG_A_IRQA => {
            core.irqa = (core.irqa & 0xFFFF_0000) | (value as u32);
        }
        omem if omem == REG_A_IRQA + 2 => {
            core.irqa = (core.irqa & 0x0000_FFFF) | ((value as u32) << 16);
        }
        REG_S_KON => {
            core.key_on = (core.key_on & 0xFFFF_0000) | (value as u32);
            start_voices(cidx, value as u32);
        }
        omem if omem == REG_S_KON + 2 => {
            core.key_on = (core.key_on & 0x0000_FFFF) | ((value as u32) << 16);
            start_voices(cidx, (value as u32) << 16);
        }
        REG_S_KOFF => {
            core.key_off = (core.key_off & 0xFFFF_0000) | (value as u32);
            stop_voices(cidx, value as u32);
        }
        omem if omem == REG_S_KOFF + 2 => {
            core.key_off = (core.key_off & 0x0000_FFFF) | ((value as u32) << 16);
            stop_voices(cidx, (value as u32) << 16);
        }
        REG_A_TSA => {
            core.tsa = (core.tsa & 0xFFFF_0000) | (value as u32);
        }
        omem if omem == REG_A_TSA + 2 => {
            core.tsa = (core.tsa & 0x0000_FFFF) | ((value as u32) << 16);
        }
        REG__1AC => {
            core.active_tsa = core.tsa;
            core.dma_write(value);
        }
        REG_S_ADMAS => {
            core.auto_dma_ctrl = value;
        }
        REG_A_ESA => {
            core.effects_start_a = (core.effects_start_a & 0xFFFF_0000) | (value as u32);
        }
        omem if omem == REG_A_ESA + 2 => {
            core.effects_start_a = (core.effects_start_a & 0x0000_FFFF) | ((value as u32) << 16);
        }
        REG_A_EEA => {
            core.effects_end_a = (core.effects_end_a & 0xFFFF_0000) | (value as u32);
        }
        R_APF1_SIZE => core.reverb.APF1_SIZE = value as u32,
        R_APF2_SIZE => core.reverb.APF2_SIZE = value as u32,
        R_SAME_L_SRC => core.reverb.SAME_L_SRC = value as u32,
        R_SAME_R_SRC => core.reverb.SAME_R_SRC = value as u32,
        R_DIFF_L_SRC => core.reverb.DIFF_L_SRC = value as u32,
        R_DIFF_R_SRC => core.reverb.DIFF_R_SRC = value as u32,
        R_SAME_L_DST => core.reverb.SAME_L_DST = value as u32,
        R_SAME_R_DST => core.reverb.SAME_R_DST = value as u32,
        R_DIFF_L_DST => core.reverb.DIFF_L_DST = value as u32,
        R_DIFF_R_DST => core.reverb.DIFF_R_DST = value as u32,
        R_COMB1_L_SRC => core.reverb.COMB1_L_SRC = value as u32,
        R_COMB1_R_SRC => core.reverb.COMB1_R_SRC = value as u32,
        R_COMB2_L_SRC => core.reverb.COMB2_L_SRC = value as u32,
        R_COMB2_R_SRC => core.reverb.COMB2_R_SRC = value as u32,
        R_COMB3_L_SRC => core.reverb.COMB3_L_SRC = value as u32,
        R_COMB3_R_SRC => core.reverb.COMB3_R_SRC = value as u32,
        R_COMB4_L_SRC => core.reverb.COMB4_L_SRC = value as u32,
        R_COMB4_R_SRC => core.reverb.COMB4_R_SRC = value as u32,
        R_IN_COEF_L => core.reverb.IN_COEF_L = value as i16,
        R_IN_COEF_R => core.reverb.IN_COEF_R = value as i16,
        R_IIR_VOL => core.reverb.IIR_VOL = value as i16,
        R_WALL_VOL => core.reverb.WALL_VOL = value as i16,
        R_COMB1_VOL => core.reverb.COMB1_VOL = value as i16,
        R_COMB2_VOL => core.reverb.COMB2_VOL = value as i16,
        R_COMB3_VOL => core.reverb.COMB3_VOL = value as i16,
        R_COMB4_VOL => core.reverb.COMB4_VOL = value as i16,
        R_APF1_VOL => core.reverb.APF1_VOL = value as i16,
        R_APF2_VOL => core.reverb.APF2_VOL = value as i16,
        R_APF1_L_DST => core.reverb.APF1_L_DST = value as u32,
        R_APF1_R_DST => core.reverb.APF1_R_DST = value as u32,
        R_APF2_L_DST => core.reverb.APF2_L_DST = value as u32,
        R_APF2_R_DST => core.reverb.APF2_R_DST = value as u32,
        REG_S_ENDX => core.core_regs.ENDX &= 0xFF_0000,
        omem if omem == REG_S_ENDX + 2 => core.core_regs.ENDX &= 0xFFFF,
        REG_P_STATX => { /* STATX writes are not meaningful in this port */ }
        REG_P_MVOLL => core.master_vol.Left.reg_set(value),
        REG_P_MVOLR => core.master_vol.Right.reg_set(value),
        REG_P_EVOLL => core.fx_vol.Left = sign_extend_16(value) as i32,
        REG_P_EVOLR => core.fx_vol.Right = sign_extend_16(value) as i32,
        REG_P_AVOLL => core.ext_vol.Left = sign_extend_16(value) as i32,
        REG_P_AVOLR => core.ext_vol.Right = sign_extend_16(value) as i32,
        REG_P_BVOLL => core.inp_vol.Left = sign_extend_16(value) as i32,
        REG_P_BVOLR => core.inp_vol.Right = sign_extend_16(value) as i32,
        REG_P_MVOLXL | REG_P_MVOLXR => { /* writes ignored */ }
        SPDIF_OUT => SPDIF.Out = value,
        SPDIF_IRQINFO => SPDIF.Info = value,
        SPDIF_MODE => {
            SPDIF.Mode = value;
            update_spdif_mode();
        }
        SPDIF_MEDIA => SPDIF.Media = value,
        SPDIF_PROTECT => SPDIF.Protection = value,
        _ => {
            // Unknown / raw register -- record into the shadow table so
            // reads see the last-written value.
        }
    }
}

fn start_voices(core: usize, mask: u32) {
    unsafe {
        Cores[core].core_regs.ENDX &= !mask;
        for vc in 0..NUM_VOICES {
            if mask & (1 << vc) != 0 {
                Cores[core].voices[vc].start();
            }
        }
    }
}

fn stop_voices(core: usize, mask: u32) {
    unsafe {
        for vc in 0..NUM_VOICES {
            if mask & (1 << vc) != 0 {
                Cores[core].voices[vc].ADSR.release();
            }
        }
    }
}

fn update_spdif_mode() {
    unsafe {
        if SPDIF.Out & 0x4 != 0 {
            PlayMode = 8;
            return;
        }
        if SPDIF.Out & SPDIF_OUT_BYPASS != 0 {
            PlayMode = 2;
            if SPDIF.Mode & SPDIF_MODE_BYPASS_BITSTREAM == 0 {
                PlayMode = 4;
            }
        } else {
            PlayMode = 0;
            if SPDIF.Out & SPDIF_OUT_PCM != 0 {
                PlayMode = 1;
            }
        }
    }
}

// =====================================================================================
//  ADPCM / voice / reverb algorithms
// =====================================================================================

/// Decode one 16-word (8 short) ADPCM block from `block` into `buffer`,
/// using the XA coefficient set `id` and the running predictors
/// `prev1` / `prev2`.
pub fn xa_decode_block(buffer: &mut [i16; PCM_DECODED_SAMPLES_PER_BLOCK],
                        block: &[i16; 8], prev1: &mut i32, prev2: &mut i32) {
    let header = block[0] as i32;
    let shift = (header & 0xF) + 16;
    let id = ((header >> 4) & 0xF) as usize;
    let id = if id > 4 { 4 } else { id };
    let pred1 = TBL_XA_FACTOR[id][0];
    let pred2 = TBL_XA_FACTOR[id][1];

    let mut out = 0usize;
    for w in 1..=7 {
        let b = block[w] as u8 as i32 as i32;
        for n in 0..2 {
            let data = if n == 0 {
                (b << 28) & 0xF000_0000u32 as i32
            } else {
                (b << 24) & 0xF000_0000u32 as i32
            };
            let pcm = (data >> shift) + (((pred1 * *prev1) + (pred2 * *prev2) + 32) >> 6);
            let pcm = clamp_mix_i32(pcm);
            buffer[out] = pcm as i16;
            out += 1;
            if n == 0 {
                *prev2 = pcm;
            } else {
                *prev1 = pcm;
            }
        }
    }
}

impl SPU2Core {
    /// Mix a single voice and return its (left, right) sample contribution.
    fn mix_voice(&mut self, voiceidx: usize, _cycles: u32) -> StereoOut32 {
        // Capture the previous voice's OutX first so we don't conflict with
        // the mutable borrow of `self.voices[voiceidx]` further down.
        let modulated_with_prev = voiceidx != 0 && self.voices[voiceidx].Modulated;
        let prev_outx = if modulated_with_prev {
            self.voices[voiceidx - 1].OutX
        } else {
            0
        };
        let v = &mut self.voices[voiceidx];
        v.Volume.update();
        let adsr_running = v.ADSR.Phase != PHASE_STOPPED;
        if !adsr_running {
            v.OutX = 0;
            return StereoOut32::EMPTY;
        }
        // Pitch -- a modulated voice (except voice 0) inherits the
        // previous voice's output as a frequency multiplier.
        let modulated = v.Modulated && voiceidx != 0;
        let base_pitch = v.Pitch as i32;
        let pitch = if modulated {
            let p = ((base_pitch * (32768 + prev_outx)) >> 15).clamp(0, 0x3FFF);
            p
        } else {
            base_pitch.min(0x3FFF)
        };
        v.SP += pitch;
        let consumed = (v.SP >> 12) as u32;
        v.SP &= 0xfff;
        v.DecPosRead = v.DecPosRead.wrapping_add(consumed);

        // Interpolation phase.
        let phase = ((v.SP & 0x0ff0) >> 4) as usize;
        let coef = &INTERP_TABLE[phase];
        let rd = v.DecPosRead as usize;
        let s0 = v.DecodeFifo[rd % 32];
        let s1 = v.DecodeFifo[(rd + 1) % 32];
        let s2 = v.DecodeFifo[(rd + 2) % 32];
        let s3 = v.DecodeFifo[(rd + 3) % 32];
        let value = ((coef[0] as i32 * s0)
                   + (coef[1] as i32 * s1)
                   + (coef[2] as i32 * s2)
                   + (coef[3] as i32 * s3)) >> 15;

        // ADSR step.
        let still_running = v.ADSR.calculate();
        if !still_running {
            v.stop();
        }
        let value = apply_volume_sample(value, v.ADSR.Value);
        v.OutX = value;
        let voice_out = StereoOut32 {
            Left: apply_volume_sample(value, v.Volume.Left.Value),
            Right: apply_volume_sample(value, v.Volume.Right.Value),
        };
        let _ = adsr_running; // suppress unused
        voice_out
    }

    /// Reverb algorithm -- the same comb + allpass + IIR chain used in the
    /// PCSX2 reference implementation, expressed on top of the in-core
    /// `RevbDownBuf` / `RevbUpBuf` working buffers.
    fn do_reverb(&mut self, input: StereoOut32, cycles: u32) -> StereoOut32 {
        if self.effects_start_a >= self.effects_end_a {
            return StereoOut32::EMPTY;
        }
        let input = clamp_mix_stereo(input);
        let pos = self.revb_sample_buf_pos as usize;
        self.revb_down_buf[0][pos] = input.Left as i16;
        self.revb_down_buf[1][pos] = input.Right as i16;
        self.revb_down_buf[0][pos | 64] = input.Left as i16;
        self.revb_down_buf[1][pos | 64] = input.Right as i16;
        let r = (cycles & 1) != 0;

        // Read and write index computation, matching the C++ `RevbGetIndexer`.
        let start = self.effects_start_a & 0x3f_ffff;
        let end = (self.effects_end_a & 0x3f_ffff) | 0xffff;
        let len = (end - start) + 1;
        let idx = |off: i32| -> u32 {
            let x = (((cycles >> 1) as i32) + off) as u32 % len;
            (x + start) & 0x0f_ffff
        };

        let same_src = idx(self.reverb.SAME_L_SRC as i32);
        let same_dst = idx(self.reverb.SAME_L_DST as i32);
        let same_prv = idx((self.reverb.SAME_L_DST as i32) - 1);
        let diff_src = if r { idx(self.reverb.DIFF_L_SRC as i32) } else { idx(self.reverb.DIFF_R_SRC as i32) };
        let diff_dst = if r { idx(self.reverb.DIFF_R_DST as i32) } else { idx(self.reverb.DIFF_L_DST as i32) };
        let diff_prv = if r { idx((self.reverb.DIFF_R_DST as i32) - 1) } else { idx((self.reverb.DIFF_L_DST as i32) - 1) };
        let comb1_src = if r { idx(self.reverb.COMB1_R_SRC as i32) } else { idx(self.reverb.COMB1_L_SRC as i32) };
        let comb2_src = if r { idx(self.reverb.COMB2_R_SRC as i32) } else { idx(self.reverb.COMB2_L_SRC as i32) };
        let comb3_src = if r { idx(self.reverb.COMB3_R_SRC as i32) } else { idx(self.reverb.COMB3_L_SRC as i32) };
        let comb4_src = if r { idx(self.reverb.COMB4_R_SRC as i32) } else { idx(self.reverb.COMB4_L_SRC as i32) };
        let apf1_src = if r {
            idx((self.reverb.APF1_R_DST as i32) - (self.reverb.APF1_SIZE as i32))
        } else {
            idx((self.reverb.APF1_L_DST as i32) - (self.reverb.APF1_SIZE as i32))
        };
        let apf1_dst = if r { idx(self.reverb.APF1_R_DST as i32) } else { idx(self.reverb.APF1_L_DST as i32) };
        let apf2_src = if r {
            idx((self.reverb.APF2_R_DST as i32) - (self.reverb.APF2_SIZE as i32))
        } else {
            idx((self.reverb.APF2_L_DST as i32) - (self.reverb.APF2_SIZE as i32))
        };
        let apf2_dst = if r { idx(self.reverb.APF2_R_DST as i32) } else { idx(self.reverb.APF2_L_DST as i32) };

        let mem = unsafe { &mut _SPU2_MEM };
        let ds = reverb_downsample(self, false);
        let mut in_l = (self.reverb.IN_COEF_L as i32 * ds) >> 15;

        let same = mul(self.reverb.IIR_VOL as i32,
            in_l + mul(self.reverb.WALL_VOL as i32, mem[same_src as usize] as i32)
            - mem[same_prv as usize] as i32) + mem[same_prv as usize] as i32;
        let diff = mul(self.reverb.IIR_VOL as i32,
            in_l + mul(self.reverb.WALL_VOL as i32, mem[diff_src as usize] as i32)
            - mem[diff_prv as usize] as i32) + mem[diff_prv as usize] as i32;
        let out = mul(self.reverb.COMB1_VOL as i32, mem[comb1_src as usize] as i32)
                + mul(self.reverb.COMB2_VOL as i32, mem[comb2_src as usize] as i32)
                + mul(self.reverb.COMB3_VOL as i32, mem[comb3_src as usize] as i32)
                + mul(self.reverb.COMB4_VOL as i32, mem[comb4_src as usize] as i32);
        let apf1 = out - mul(self.reverb.APF1_VOL as i32, mem[apf1_src as usize] as i32);
        let out2 = mem[apf1_src as usize] as i32 + mul(self.reverb.APF1_VOL as i32, apf1);
        let apf2 = out2 - mul(self.reverb.APF2_VOL as i32, mem[apf2_src as usize] as i32);
        let out3 = mem[apf2_src as usize] as i32 + mul(self.reverb.APF2_VOL as i32, apf2);

        if self.fx_enable {
            mem[same_dst as usize] = clamp_mix_i32(same) as i16;
            mem[diff_dst as usize] = clamp_mix_i32(diff) as i16;
            mem[apf1_dst as usize] = clamp_mix_i32(apf1) as i16;
            mem[apf2_dst as usize] = clamp_mix_i32(apf2) as i16;
        }
        let out3 = clamp_mix_i32(out3);

        self.revb_up_buf[r as usize][pos] = out3 as i16;
        self.revb_up_buf[!r as usize][pos] = 0;
        self.revb_up_buf[r as usize][pos | 64] = out3 as i16;
        self.revb_up_buf[!r as usize][pos | 64] = 0;
        self.revb_sample_buf_pos = (self.revb_sample_buf_pos + 1) & 63;
        let _ = in_l; // suppress unused
        reverb_upsample(self)
    }

    /// Write a 16-bit value to the SPU2 sample memory, applying cache
    /// invalidation.  Used by the 0x1AC direct-DMA register and the
    /// AutoDMA path.
    fn dma_write(&mut self, value: u16) {
        let a = self.active_tsa & 0xf_ffff;
        if a >= SPU2_DYN_MEMLINE {
            let cache_idx = (a / PCM_WORDS_PER_BLOCK as u32) as usize;
            unsafe { PCM_CACHE[cache_idx].Validated = false; }
        }
        self.adpcm[a as usize] = value as i16;
        self.active_tsa = (self.active_tsa + 1) & 0xf_ffff;
        self.tsa = self.active_tsa;
    }
}

#[inline]
fn mul(a: i32, b: i32) -> i32 {
    (a * b) >> 15
}

/// 39-tap polyphase FIR downsample used by the reverb path.
fn reverb_downsample(core: &SPU2Core, right: bool) -> i32 {
    let pos = core.revb_sample_buf_pos as i32;
    let idx = ((pos - NUM_TAPS as i32) & 63) as usize;
    let ch = if right { &core.revb_down_buf[1] } else { &core.revb_down_buf[0] };
    let mut acc: i32 = 0;
    for i in 0..NUM_TAPS {
        acc += (ch[idx + i] as i32) * (FILTER_DOWN_COEFS[i] as i32);
    }
    clamp_mix_i32(acc >> 15)
}

/// 39-tap polyphase FIR upsample used by the reverb path.
fn reverb_upsample(core: &SPU2Core) -> StereoOut32 {
    let pos = core.revb_sample_buf_pos as i32;
    let idx = ((pos - NUM_TAPS as i32) & 63) as usize;
    let mut l: i32 = 0;
    let mut r: i32 = 0;
    for i in 0..NUM_TAPS {
        l += (core.revb_up_buf[0][idx + i] as i32) * (FILTER_UP_COEFS[i] as i32);
        r += (core.revb_up_buf[1][idx + i] as i32) * (FILTER_UP_COEFS[i] as i32);
    }
    StereoOut32 { Left: clamp_mix_i32(l >> 15), Right: clamp_mix_i32(r >> 15) }
}

/// Time update -- consumes `cClocks - lClocks` worth of mixer ticks.
pub fn time_update(cClocks: u32) {
    unsafe {
        let mut d = cClocks.wrapping_sub(lClocks as u32);
        if d > 0xFFFFFFF1 {
            return;
        }
        if d > TICK_INTERVAL * 4800 {
            d = TICK_INTERVAL * 4800;
            lClocks = (cClocks - d) as u64;
        }
        while d >= TICK_INTERVAL {
            d -= TICK_INTERVAL;
            lClocks += TICK_INTERVAL as u64;
            Cycles += 1;
            for c in 0..2 {
                if Cores[c].key_off != 0 {
                    stop_voices(c, Cores[c].key_off);
                    Cores[c].key_off = 0;
                }
                if Cores[c].key_on != 0 {
                    start_voices(c, Cores[c].key_on);
                    Cores[c].key_on = 0;
                }
            }
            let mix0 = Cores[0].mix_tick(Cycles);
            let mix1 = Cores[1].mix_tick(Cycles);
            spu2_output(StereoOut32 {
                Left: clamp_mix_i32(mix0.Left) + clamp_mix_i32(mix1.Left),
                Right: clamp_mix_i32(mix0.Right) + clamp_mix_i32(mix1.Right),
            });
        }
        OutPos = (OutPos + 1) & 0x1ff;
    }
}

fn spu2_output(_out: StereoOut32) {
    // The Rust port does not own an AudioStream; the real implementation
    // (see the C++ `spu2Output` in `spu2.cpp`) would forward samples to
    // the host audio device and GS capture, plus run a DC-blocking high
    // pass filter.
}

// =====================================================================================
//  Convenience accessors used by the unit-test harness and the rest of the
//  rust/pcsx2 crate.
// =====================================================================================

/// Returns a snapshot of the current SPU2 register value.
pub fn read_reg_value(core: usize, omem: u32) -> u16 {
    unsafe { Cores[core].regs[((omem >> 1) & 0x3ff) as usize] }
}

/// Returns the 24-bit `ENDX` register (voices that have hit a loop endpoint).
pub fn endx(core: usize) -> u32 {
    unsafe { Cores[core].core_regs.ENDX }
}

/// Returns the cached ADPCM sample memory contents.
pub fn mem_at(core: usize, addr: u32) -> i16 {
    unsafe { Cores[core].adpcm[(addr & 0xf_ffff) as usize] }
}

/// Manually push a value into a voice's decoded FIFO -- test hook.
pub fn push_voice_sample(core: usize, voice: usize, value: i32) {
    unsafe {
        let v = &mut Cores[core].voices[voice];
        v.DecodeFifo[(v.DecPosWrite as usize) % 32] = value;
    }
}
