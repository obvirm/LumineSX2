// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
//! PS2 Image Processing Unit (IPU) — full MPEG-2 macroblock/video decoder.
//!
//! This module is a Rust 2021 translation of the entire PCSX2 IPU subsystem
//! (`pcsx2/IPU/IPU.cpp`, `IPU.h`, `IPU_MultiISA.cpp`, `IPU_MultiISA.h`,
//! `IPUdither.cpp`, `IPUdma.cpp`, `IPUdma.h`, `IPU_Fifo.cpp`, `IPU_Fifo.h`,
//! `mpeg2_vlc.h`, `yuv2rgb.cpp`, `yuv2rgb.h`). The PS2 IPU is a hardware
//! MPEG-2 macroblock decoder that reads an elementary stream from an input
//! FIFO, performs VLC decoding, IDCT, and motion-compensated prediction,
//! and writes decoded macroblocks (RGB16 / RGB32 / INDX4) out through an
//! output FIFO. The translation preserves the C++ linkage and the
//! hardware-visible register layout while expressing the bitfield and
//! union types as idiomatic Rust structures. Only the standard library is
//! used and the global hardware state is exposed via `static mut`, mirroring
//! the C++ linkage the EE side mutates through the DMAC.
//!
//! Public entry points: [`ipuInit`], [`ipuReset`], [`ipuShutdown`],
//! [`ipuRead32`], [`ipuWrite32`]. The MPEG-2 state, the IDCT, the dither,
//! and the YUV to RGB colour-space conversion are all reachable from the
//! internal `decoder` and `g_BP` statics.

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(dead_code)]

use std::cmp;
use std::collections::VecDeque;
use std::ptr;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// PS2 IPU register page base (the EE maps the IPU at 0x1000_2000).
pub const IPU_BASE: u32 = 0x1000_2000;

/// Non-linear quantiser scaling (table B-1 in the MPEG-2 spec).
pub const NON_LINEAR_QUANTIZER_SCALE: [i32; 32] = [
    0, 1, 2, 3, 4, 5, 6, 7,
    8, 10, 12, 14, 16, 18, 20, 22,
    24, 28, 32, 36, 40, 44, 48, 52,
    56, 64, 72, 80, 88, 96, 104, 112,
];

// SCE-IPU opcodes (low 4 bits of an IPU_CMD write).
pub const SCE_IPU_BCLR: u32 = 0x0;
pub const SCE_IPU_IDEC: u32 = 0x1;
pub const SCE_IPU_BDEC: u32 = 0x2;
pub const SCE_IPU_VDEC: u32 = 0x3;
pub const SCE_IPU_FDEC: u32 = 0x4;
pub const SCE_IPU_SETIQ: u32 = 0x5;
pub const SCE_IPU_SETVQ: u32 = 0x6;
pub const SCE_IPU_CSC: u32 = 0x7;
pub const SCE_IPU_PACK: u32 = 0x8;
pub const SCE_IPU_SETTH: u32 = 0x9;

// MPEG-2 macroblock / picture enums (from mpeg2_vlc.h).
pub const MACROBLOCK_INTRA: i32 = 1;
pub const MACROBLOCK_PATTERN: i32 = 2;
pub const MACROBLOCK_MOTION_BACKWARD: i32 = 4;
pub const MACROBLOCK_MOTION_FORWARD: i32 = 8;
pub const MACROBLOCK_QUANT: i32 = 16;
pub const DCT_TYPE_INTERLACED: i32 = 32;

pub const MOTION_TYPE_BASE: i32 = 64;
pub const MC_FIELD: i32 = 64;
pub const MC_FRAME: i32 = 128;
pub const MC_16X8: i32 = 128;
pub const MC_DMV: i32 = 192;

pub const TOP_FIELD: i32 = 1;
pub const BOTTOM_FIELD: i32 = 2;
pub const FRAME_PICTURE: i32 = 3;

pub const I_TYPE: i32 = 1;
pub const P_TYPE: i32 = 2;
pub const B_TYPE: i32 = 3;
pub const D_TYPE: i32 = 4;

// YUV to RGB BT.601 coefficients (from yuv2rgb.cpp).
const IPU_Y_BIAS: i32 = 16;
const IPU_C_BIAS: i32 = 128;
const IPU_Y_COEFF: i32 = 0x95; //  1.1640625
const IPU_GCR_COEFF: i32 = -0x68; // -0.8125
const IPU_GCB_COEFF: i32 = -0x32; // -0.390625
const IPU_RCR_COEFF: i32 = 0xcc; //  1.59375
const IPU_BCB_COEFF: i32 = 0x102; // 2.015625

// INTC interrupt lines (used by the C++ code as plain integers).
pub const INTC_IPU: i32 = 8;
pub const DMAC_FROM_IPU: i32 = 13;
pub const DMAC_TO_IPU: i32 = 4;
pub const IPU_PROCESS: i32 = 24;

// IPU address space selectors (used as keys to gate the address decode).
const IPU_CMD_OFF: u32 = 0x00;
const IPU_CTRL_OFF: u32 = 0x04;
const IPU_BP_OFF: u32 = 0x08;
const IPU_TOP_OFF: u32 = 0x0c;

// IPU register file is 0x100 bytes; addresses above 0x100 mirror below 0x100.
const IPU_PAGE_MASK: u32 = 0xfff;
const IPU_REG_MASK: u32 = 0xff;

// Decoder stride (the IPU uses 16-byte / QWC strides throughout).
const DECODER_STRIDE: i32 = 16;

// BIAS factor for cycle conversions (matches the C++ IPU_INT_* macros).
const BIAS: i32 = 1;

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Big-endian byte swap (replaces the C++ `BigEndian` macro).
#[inline]
pub fn big_endian(v: u32) -> u32 {
    v.swap_bytes()
}

/// Big-endian byte swap for 64-bit values.
#[inline]
pub fn big_endian_64(v: u64) -> u64 {
    v.swap_bytes()
}

#[inline]
fn clamp_u8(v: i32) -> u8 {
    if v < 0 {
        0
    } else if v > 255 {
        255
    } else {
        v as u8
    }
}

#[inline]
fn clamp_signed(v: i32, lo: i32, hi: i32) -> i32 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

// ---------------------------------------------------------------------------
// MPEG-2 VLC tables (from mpeg2_vlc.h)
// ---------------------------------------------------------------------------

/// Macroblock VLC entry.
#[derive(Clone, Copy, Default)]
pub struct MBtab {
    pub modes: u8,
    pub len: u8,
}

/// Motion vector VLC entry.
#[derive(Clone, Copy, Default)]
pub struct MVtab {
    pub delta: u8,
    pub len: u8,
}

/// Dual-motion-vector VLC entry.
#[derive(Clone, Copy, Default)]
pub struct DMVtab {
    pub dmv: i8,
    pub len: u8,
}

/// Coded-block-pattern VLC entry.
#[derive(Clone, Copy, Default)]
pub struct CBPtab {
    pub cbp: u8,
    pub len: u8,
}

/// DC coefficient VLC entry.
#[derive(Clone, Copy, Default)]
pub struct DCtab {
    pub size: u8,
    pub len: u8,
}

/// DCT coefficient VLC entry.
#[derive(Clone, Copy, Default)]
pub struct DCTtab {
    pub run: u8,
    pub level: u8,
    pub len: u8,
}

/// Macroblock address VLC entry.
#[derive(Clone, Copy, Default)]
pub struct MBAtab {
    pub mba: u8,
    pub len: u8,
}

pub const MB_I: [MBtab; 2] = [
    MBtab { modes: MACROBLOCK_INTRA as u8 | MACROBLOCK_QUANT as u8, len: 2 },
    MBtab { modes: MACROBLOCK_INTRA as u8, len: 1 },
];

pub const MB_P: [MBtab; 36] = [
    MBtab { modes: 17, len: 6 }, MBtab { modes: 18, len: 5 }, MBtab { modes: 26, len: 5 },
    MBtab { modes: 1, len: 5 },
    MBtab { modes: 8, len: 3 }, MBtab { modes: 8, len: 3 }, MBtab { modes: 8, len: 3 },
    MBtab { modes: 8, len: 3 },
    MBtab { modes: 2, len: 2 }, MBtab { modes: 2, len: 2 }, MBtab { modes: 2, len: 2 },
    MBtab { modes: 2, len: 2 },
    MBtab { modes: 2, len: 2 }, MBtab { modes: 2, len: 2 }, MBtab { modes: 2, len: 2 },
    MBtab { modes: 2, len: 2 },
    MBtab { modes: 10, len: 1 }, MBtab { modes: 10, len: 1 }, MBtab { modes: 10, len: 1 },
    MBtab { modes: 10, len: 1 },
    MBtab { modes: 10, len: 1 }, MBtab { modes: 10, len: 1 }, MBtab { modes: 10, len: 1 },
    MBtab { modes: 10, len: 1 },
    MBtab { modes: 10, len: 1 }, MBtab { modes: 10, len: 1 }, MBtab { modes: 10, len: 1 },
    MBtab { modes: 10, len: 1 },
    MBtab { modes: 10, len: 1 }, MBtab { modes: 10, len: 1 }, MBtab { modes: 10, len: 1 },
    MBtab { modes: 10, len: 1 },
    MBtab { modes: 10, len: 1 }, MBtab { modes: 10, len: 1 }, MBtab { modes: 10, len: 1 },
    MBtab { modes: 10, len: 1 },
];

pub const MB_B: [MBtab; 64] = [
    MBtab { modes: 0, len: 0 }, MBtab { modes: 17, len: 6 },
    MBtab { modes: 22, len: 6 }, MBtab { modes: 26, len: 6 },
    MBtab { modes: 28, len: 5 }, MBtab { modes: 28, len: 5 },
    MBtab { modes: 1, len: 5 }, MBtab { modes: 1, len: 5 },
    MBtab { modes: 8, len: 4 }, MBtab { modes: 8, len: 4 }, MBtab { modes: 8, len: 4 },
    MBtab { modes: 8, len: 4 },
    MBtab { modes: 10, len: 4 }, MBtab { modes: 10, len: 4 }, MBtab { modes: 10, len: 4 },
    MBtab { modes: 10, len: 4 },
    MBtab { modes: 4, len: 3 }, MBtab { modes: 4, len: 3 }, MBtab { modes: 4, len: 3 },
    MBtab { modes: 4, len: 3 },
    MBtab { modes: 4, len: 3 }, MBtab { modes: 4, len: 3 }, MBtab { modes: 4, len: 3 },
    MBtab { modes: 4, len: 3 },
    MBtab { modes: 6, len: 3 }, MBtab { modes: 6, len: 3 }, MBtab { modes: 6, len: 3 },
    MBtab { modes: 6, len: 3 },
    MBtab { modes: 6, len: 3 }, MBtab { modes: 6, len: 3 }, MBtab { modes: 6, len: 3 },
    MBtab { modes: 6, len: 3 },
    MBtab { modes: 12, len: 2 }, MBtab { modes: 12, len: 2 }, MBtab { modes: 12, len: 2 },
    MBtab { modes: 12, len: 2 },
    MBtab { modes: 12, len: 2 }, MBtab { modes: 12, len: 2 }, MBtab { modes: 12, len: 2 },
    MBtab { modes: 12, len: 2 },
    MBtab { modes: 12, len: 2 }, MBtab { modes: 12, len: 2 }, MBtab { modes: 12, len: 2 },
    MBtab { modes: 12, len: 2 },
    MBtab { modes: 12, len: 2 }, MBtab { modes: 12, len: 2 }, MBtab { modes: 12, len: 2 },
    MBtab { modes: 12, len: 2 },
    MBtab { modes: 14, len: 2 }, MBtab { modes: 14, len: 2 }, MBtab { modes: 14, len: 2 },
    MBtab { modes: 14, len: 2 },
    MBtab { modes: 14, len: 2 }, MBtab { modes: 14, len: 2 }, MBtab { modes: 14, len: 2 },
    MBtab { modes: 14, len: 2 },
    MBtab { modes: 14, len: 2 }, MBtab { modes: 14, len: 2 }, MBtab { modes: 14, len: 2 },
    MBtab { modes: 14, len: 2 },
    MBtab { modes: 14, len: 2 }, MBtab { modes: 14, len: 2 }, MBtab { modes: 14, len: 2 },
    MBtab { modes: 14, len: 2 },
];

pub const MV_4: [MVtab; 8] = [
    MVtab { delta: 3, len: 6 }, MVtab { delta: 2, len: 4 },
    MVtab { delta: 1, len: 3 }, MVtab { delta: 1, len: 3 },
    MVtab { delta: 0, len: 2 }, MVtab { delta: 0, len: 2 },
    MVtab { delta: 0, len: 2 }, MVtab { delta: 0, len: 2 },
];

pub const MV_10: [MVtab; 48] = [
    MVtab { delta: 0, len: 10 }, MVtab { delta: 0, len: 10 }, MVtab { delta: 0, len: 10 },
    MVtab { delta: 0, len: 10 },
    MVtab { delta: 0, len: 10 }, MVtab { delta: 0, len: 10 }, MVtab { delta: 0, len: 10 },
    MVtab { delta: 0, len: 10 },
    MVtab { delta: 0, len: 10 }, MVtab { delta: 0, len: 10 }, MVtab { delta: 0, len: 10 },
    MVtab { delta: 0, len: 10 },
    MVtab { delta: 15, len: 10 }, MVtab { delta: 14, len: 10 }, MVtab { delta: 13, len: 10 },
    MVtab { delta: 12, len: 10 },
    MVtab { delta: 11, len: 10 }, MVtab { delta: 10, len: 10 },
    MVtab { delta: 9, len: 9 }, MVtab { delta: 9, len: 9 },
    MVtab { delta: 8, len: 9 }, MVtab { delta: 8, len: 9 },
    MVtab { delta: 7, len: 9 }, MVtab { delta: 7, len: 9 },
    MVtab { delta: 6, len: 7 }, MVtab { delta: 6, len: 7 }, MVtab { delta: 6, len: 7 },
    MVtab { delta: 6, len: 7 },
    MVtab { delta: 6, len: 7 }, MVtab { delta: 6, len: 7 }, MVtab { delta: 6, len: 7 },
    MVtab { delta: 6, len: 7 },
    MVtab { delta: 5, len: 7 }, MVtab { delta: 5, len: 7 }, MVtab { delta: 5, len: 7 },
    MVtab { delta: 5, len: 7 },
    MVtab { delta: 5, len: 7 }, MVtab { delta: 5, len: 7 }, MVtab { delta: 5, len: 7 },
    MVtab { delta: 5, len: 7 },
    MVtab { delta: 4, len: 7 }, MVtab { delta: 4, len: 7 }, MVtab { delta: 4, len: 7 },
    MVtab { delta: 4, len: 7 },
    MVtab { delta: 4, len: 7 }, MVtab { delta: 4, len: 7 }, MVtab { delta: 4, len: 7 },
    MVtab { delta: 4, len: 7 },
];

pub const DMV_2: [DMVtab; 4] = [
    DMVtab { dmv: 0, len: 1 }, DMVtab { dmv: 0, len: 1 },
    DMVtab { dmv: 1, len: 2 }, DMVtab { dmv: -1, len: 2 },
];

pub const CBP_7: [CBPtab; 112] = [
    CBPtab { cbp: 0x22, len: 7 }, CBPtab { cbp: 0x12, len: 7 }, CBPtab { cbp: 0x0a, len: 7 },
    CBPtab { cbp: 0x06, len: 7 },
    CBPtab { cbp: 0x21, len: 7 }, CBPtab { cbp: 0x11, len: 7 }, CBPtab { cbp: 0x09, len: 7 },
    CBPtab { cbp: 0x05, len: 7 },
    CBPtab { cbp: 0x3f, len: 6 }, CBPtab { cbp: 0x3f, len: 6 }, CBPtab { cbp: 0x03, len: 6 },
    CBPtab { cbp: 0x03, len: 6 },
    CBPtab { cbp: 0x24, len: 6 }, CBPtab { cbp: 0x24, len: 6 }, CBPtab { cbp: 0x18, len: 6 },
    CBPtab { cbp: 0x18, len: 6 },
    CBPtab { cbp: 0x3e, len: 5 }, CBPtab { cbp: 0x3e, len: 5 }, CBPtab { cbp: 0x3e, len: 5 },
    CBPtab { cbp: 0x3e, len: 5 },
    CBPtab { cbp: 0x02, len: 5 }, CBPtab { cbp: 0x02, len: 5 }, CBPtab { cbp: 0x02, len: 5 },
    CBPtab { cbp: 0x02, len: 5 },
    CBPtab { cbp: 0x3d, len: 5 }, CBPtab { cbp: 0x3d, len: 5 }, CBPtab { cbp: 0x3d, len: 5 },
    CBPtab { cbp: 0x3d, len: 5 },
    CBPtab { cbp: 0x01, len: 5 }, CBPtab { cbp: 0x01, len: 5 }, CBPtab { cbp: 0x01, len: 5 },
    CBPtab { cbp: 0x01, len: 5 },
    CBPtab { cbp: 0x38, len: 5 }, CBPtab { cbp: 0x38, len: 5 }, CBPtab { cbp: 0x38, len: 5 },
    CBPtab { cbp: 0x38, len: 5 },
    CBPtab { cbp: 0x34, len: 5 }, CBPtab { cbp: 0x34, len: 5 }, CBPtab { cbp: 0x34, len: 5 },
    CBPtab { cbp: 0x34, len: 5 },
    CBPtab { cbp: 0x2c, len: 5 }, CBPtab { cbp: 0x2c, len: 5 }, CBPtab { cbp: 0x2c, len: 5 },
    CBPtab { cbp: 0x2c, len: 5 },
    CBPtab { cbp: 0x1c, len: 5 }, CBPtab { cbp: 0x1c, len: 5 }, CBPtab { cbp: 0x1c, len: 5 },
    CBPtab { cbp: 0x1c, len: 5 },
    CBPtab { cbp: 0x28, len: 5 }, CBPtab { cbp: 0x28, len: 5 }, CBPtab { cbp: 0x28, len: 5 },
    CBPtab { cbp: 0x28, len: 5 },
    CBPtab { cbp: 0x14, len: 5 }, CBPtab { cbp: 0x14, len: 5 }, CBPtab { cbp: 0x14, len: 5 },
    CBPtab { cbp: 0x14, len: 5 },
    CBPtab { cbp: 0x30, len: 5 }, CBPtab { cbp: 0x30, len: 5 }, CBPtab { cbp: 0x30, len: 5 },
    CBPtab { cbp: 0x30, len: 5 },
    CBPtab { cbp: 0x0c, len: 5 }, CBPtab { cbp: 0x0c, len: 5 }, CBPtab { cbp: 0x0c, len: 5 },
    CBPtab { cbp: 0x0c, len: 5 },
    CBPtab { cbp: 0x20, len: 4 }, CBPtab { cbp: 0x20, len: 4 }, CBPtab { cbp: 0x20, len: 4 },
    CBPtab { cbp: 0x20, len: 4 },
    CBPtab { cbp: 0x20, len: 4 }, CBPtab { cbp: 0x20, len: 4 }, CBPtab { cbp: 0x20, len: 4 },
    CBPtab { cbp: 0x20, len: 4 },
    CBPtab { cbp: 0x10, len: 4 }, CBPtab { cbp: 0x10, len: 4 }, CBPtab { cbp: 0x10, len: 4 },
    CBPtab { cbp: 0x10, len: 4 },
    CBPtab { cbp: 0x10, len: 4 }, CBPtab { cbp: 0x10, len: 4 }, CBPtab { cbp: 0x10, len: 4 },
    CBPtab { cbp: 0x10, len: 4 },
    CBPtab { cbp: 0x08, len: 4 }, CBPtab { cbp: 0x08, len: 4 }, CBPtab { cbp: 0x08, len: 4 },
    CBPtab { cbp: 0x08, len: 4 },
    CBPtab { cbp: 0x08, len: 4 }, CBPtab { cbp: 0x08, len: 4 }, CBPtab { cbp: 0x08, len: 4 },
    CBPtab { cbp: 0x08, len: 4 },
    CBPtab { cbp: 0x04, len: 4 }, CBPtab { cbp: 0x04, len: 4 }, CBPtab { cbp: 0x04, len: 4 },
    CBPtab { cbp: 0x04, len: 4 },
    CBPtab { cbp: 0x04, len: 4 }, CBPtab { cbp: 0x04, len: 4 }, CBPtab { cbp: 0x04, len: 4 },
    CBPtab { cbp: 0x04, len: 4 },
    CBPtab { cbp: 0x3c, len: 3 }, CBPtab { cbp: 0x3c, len: 3 }, CBPtab { cbp: 0x3c, len: 3 },
    CBPtab { cbp: 0x3c, len: 3 },
    CBPtab { cbp: 0x3c, len: 3 }, CBPtab { cbp: 0x3c, len: 3 }, CBPtab { cbp: 0x3c, len: 3 },
    CBPtab { cbp: 0x3c, len: 3 },
    CBPtab { cbp: 0x3c, len: 3 }, CBPtab { cbp: 0x3c, len: 3 }, CBPtab { cbp: 0x3c, len: 3 },
    CBPtab { cbp: 0x3c, len: 3 },
    CBPtab { cbp: 0x3c, len: 3 }, CBPtab { cbp: 0x3c, len: 3 }, CBPtab { cbp: 0x3c, len: 3 },
    CBPtab { cbp: 0x3c, len: 3 },
];

pub const CBP_9: [CBPtab; 64] = [
    CBPtab { cbp: 0, len: 0 }, CBPtab { cbp: 0x00, len: 9 }, CBPtab { cbp: 0x27, len: 9 },
    CBPtab { cbp: 0x1b, len: 9 },
    CBPtab { cbp: 0x3b, len: 9 }, CBPtab { cbp: 0x37, len: 9 }, CBPtab { cbp: 0x2f, len: 9 },
    CBPtab { cbp: 0x1f, len: 9 },
    CBPtab { cbp: 0x3a, len: 8 }, CBPtab { cbp: 0x3a, len: 8 }, CBPtab { cbp: 0x36, len: 8 },
    CBPtab { cbp: 0x36, len: 8 },
    CBPtab { cbp: 0x2e, len: 8 }, CBPtab { cbp: 0x2e, len: 8 }, CBPtab { cbp: 0x1e, len: 8 },
    CBPtab { cbp: 0x1e, len: 8 },
    CBPtab { cbp: 0x39, len: 8 }, CBPtab { cbp: 0x39, len: 8 }, CBPtab { cbp: 0x35, len: 8 },
    CBPtab { cbp: 0x35, len: 8 },
    CBPtab { cbp: 0x2d, len: 8 }, CBPtab { cbp: 0x2d, len: 8 }, CBPtab { cbp: 0x1d, len: 8 },
    CBPtab { cbp: 0x1d, len: 8 },
    CBPtab { cbp: 0x26, len: 8 }, CBPtab { cbp: 0x26, len: 8 }, CBPtab { cbp: 0x1a, len: 8 },
    CBPtab { cbp: 0x1a, len: 8 },
    CBPtab { cbp: 0x25, len: 8 }, CBPtab { cbp: 0x25, len: 8 }, CBPtab { cbp: 0x19, len: 8 },
    CBPtab { cbp: 0x19, len: 8 },
    CBPtab { cbp: 0x2b, len: 8 }, CBPtab { cbp: 0x2b, len: 8 }, CBPtab { cbp: 0x17, len: 8 },
    CBPtab { cbp: 0x17, len: 8 },
    CBPtab { cbp: 0x33, len: 8 }, CBPtab { cbp: 0x33, len: 8 }, CBPtab { cbp: 0x0f, len: 8 },
    CBPtab { cbp: 0x0f, len: 8 },
    CBPtab { cbp: 0x2a, len: 8 }, CBPtab { cbp: 0x2a, len: 8 }, CBPtab { cbp: 0x16, len: 8 },
    CBPtab { cbp: 0x16, len: 8 },
    CBPtab { cbp: 0x32, len: 8 }, CBPtab { cbp: 0x32, len: 8 }, CBPtab { cbp: 0x0e, len: 8 },
    CBPtab { cbp: 0x0e, len: 8 },
    CBPtab { cbp: 0x29, len: 8 }, CBPtab { cbp: 0x29, len: 8 }, CBPtab { cbp: 0x15, len: 8 },
    CBPtab { cbp: 0x15, len: 8 },
    CBPtab { cbp: 0x31, len: 8 }, CBPtab { cbp: 0x31, len: 8 }, CBPtab { cbp: 0x0d, len: 8 },
    CBPtab { cbp: 0x0d, len: 8 },
    CBPtab { cbp: 0x23, len: 8 }, CBPtab { cbp: 0x23, len: 8 }, CBPtab { cbp: 0x13, len: 8 },
    CBPtab { cbp: 0x13, len: 8 },
    CBPtab { cbp: 0x0b, len: 8 }, CBPtab { cbp: 0x0b, len: 8 }, CBPtab { cbp: 0x07, len: 8 },
    CBPtab { cbp: 0x07, len: 8 },
];

/// MBA lookup set: short (5-bit) codes and long (11-bit) codes.
pub struct MBAtabSet {
    pub mba5: [MBAtab; 30],
    pub mba11: [MBAtab; 104],
}

pub const MBA: MBAtabSet = MBAtabSet {
    mba5: [
        MBAtab { mba: 6, len: 5 }, MBAtab { mba: 5, len: 5 },
        MBAtab { mba: 4, len: 4 }, MBAtab { mba: 4, len: 4 },
        MBAtab { mba: 3, len: 4 }, MBAtab { mba: 3, len: 4 },
        MBAtab { mba: 2, len: 3 }, MBAtab { mba: 2, len: 3 },
        MBAtab { mba: 2, len: 3 }, MBAtab { mba: 2, len: 3 },
        MBAtab { mba: 1, len: 3 }, MBAtab { mba: 1, len: 3 },
        MBAtab { mba: 1, len: 3 }, MBAtab { mba: 1, len: 3 },
        MBAtab { mba: 0, len: 1 }, MBAtab { mba: 0, len: 1 },
        MBAtab { mba: 0, len: 1 }, MBAtab { mba: 0, len: 1 },
        MBAtab { mba: 0, len: 1 }, MBAtab { mba: 0, len: 1 },
        MBAtab { mba: 0, len: 1 }, MBAtab { mba: 0, len: 1 },
        MBAtab { mba: 0, len: 1 }, MBAtab { mba: 0, len: 1 },
        MBAtab { mba: 0, len: 1 }, MBAtab { mba: 0, len: 1 },
        MBAtab { mba: 0, len: 1 }, MBAtab { mba: 0, len: 1 },
        MBAtab { mba: 0, len: 1 }, MBAtab { mba: 0, len: 1 },
    ],
    mba11: [
        MBAtab { mba: 32, len: 11 }, MBAtab { mba: 31, len: 11 },
        MBAtab { mba: 30, len: 11 }, MBAtab { mba: 29, len: 11 },
        MBAtab { mba: 28, len: 11 }, MBAtab { mba: 27, len: 11 },
        MBAtab { mba: 26, len: 11 }, MBAtab { mba: 25, len: 11 },
        MBAtab { mba: 24, len: 11 }, MBAtab { mba: 23, len: 11 },
        MBAtab { mba: 22, len: 11 }, MBAtab { mba: 21, len: 11 },
        MBAtab { mba: 20, len: 10 }, MBAtab { mba: 20, len: 10 },
        MBAtab { mba: 19, len: 10 }, MBAtab { mba: 19, len: 10 },
        MBAtab { mba: 18, len: 10 }, MBAtab { mba: 18, len: 10 },
        MBAtab { mba: 17, len: 10 }, MBAtab { mba: 17, len: 10 },
        MBAtab { mba: 16, len: 10 }, MBAtab { mba: 16, len: 10 },
        MBAtab { mba: 15, len: 10 }, MBAtab { mba: 15, len: 10 },
        MBAtab { mba: 14, len: 8 }, MBAtab { mba: 14, len: 8 },
        MBAtab { mba: 14, len: 8 }, MBAtab { mba: 14, len: 8 },
        MBAtab { mba: 14, len: 8 }, MBAtab { mba: 14, len: 8 },
        MBAtab { mba: 14, len: 8 }, MBAtab { mba: 14, len: 8 },
        MBAtab { mba: 13, len: 8 }, MBAtab { mba: 13, len: 8 },
        MBAtab { mba: 13, len: 8 }, MBAtab { mba: 13, len: 8 },
        MBAtab { mba: 13, len: 8 }, MBAtab { mba: 13, len: 8 },
        MBAtab { mba: 13, len: 8 }, MBAtab { mba: 13, len: 8 },
        MBAtab { mba: 12, len: 8 }, MBAtab { mba: 12, len: 8 },
        MBAtab { mba: 12, len: 8 }, MBAtab { mba: 12, len: 8 },
        MBAtab { mba: 12, len: 8 }, MBAtab { mba: 12, len: 8 },
        MBAtab { mba: 12, len: 8 }, MBAtab { mba: 12, len: 8 },
        MBAtab { mba: 11, len: 8 }, MBAtab { mba: 11, len: 8 },
        MBAtab { mba: 11, len: 8 }, MBAtab { mba: 11, len: 8 },
        MBAtab { mba: 11, len: 8 }, MBAtab { mba: 11, len: 8 },
        MBAtab { mba: 11, len: 8 }, MBAtab { mba: 11, len: 8 },
        MBAtab { mba: 10, len: 8 }, MBAtab { mba: 10, len: 8 },
        MBAtab { mba: 10, len: 8 }, MBAtab { mba: 10, len: 8 },
        MBAtab { mba: 10, len: 8 }, MBAtab { mba: 10, len: 8 },
        MBAtab { mba: 10, len: 8 }, MBAtab { mba: 10, len: 8 },
        MBAtab { mba: 9, len: 8 }, MBAtab { mba: 9, len: 8 },
        MBAtab { mba: 9, len: 8 }, MBAtab { mba: 9, len: 8 },
        MBAtab { mba: 9, len: 8 }, MBAtab { mba: 9, len: 8 },
        MBAtab { mba: 9, len: 8 }, MBAtab { mba: 9, len: 8 },
        MBAtab { mba: 8, len: 7 }, MBAtab { mba: 8, len: 7 },
        MBAtab { mba: 8, len: 7 }, MBAtab { mba: 8, len: 7 },
        MBAtab { mba: 8, len: 7 }, MBAtab { mba: 8, len: 7 },
        MBAtab { mba: 8, len: 7 }, MBAtab { mba: 8, len: 7 },
        MBAtab { mba: 8, len: 7 }, MBAtab { mba: 8, len: 7 },
        MBAtab { mba: 8, len: 7 }, MBAtab { mba: 8, len: 7 },
        MBAtab { mba: 8, len: 7 }, MBAtab { mba: 8, len: 7 },
        MBAtab { mba: 8, len: 7 }, MBAtab { mba: 8, len: 7 },
        MBAtab { mba: 7, len: 7 }, MBAtab { mba: 7, len: 7 },
        MBAtab { mba: 7, len: 7 }, MBAtab { mba: 7, len: 7 },
        MBAtab { mba: 7, len: 7 }, MBAtab { mba: 7, len: 7 },
        MBAtab { mba: 7, len: 7 }, MBAtab { mba: 7, len: 7 },
        MBAtab { mba: 7, len: 7 }, MBAtab { mba: 7, len: 7 },
        MBAtab { mba: 7, len: 7 }, MBAtab { mba: 7, len: 7 },
        MBAtab { mba: 7, len: 7 }, MBAtab { mba: 7, len: 7 },
        MBAtab { mba: 7, len: 7 }, MBAtab { mba: 7, len: 7 },
    ],
};

/// DC coefficient lookup tables.
pub struct DCtabSet {
    pub lum0: [DCtab; 32],
    pub lum1: [DCtab; 16],
    pub chrom0: [DCtab; 32],
    pub chrom1: [DCtab; 32],
}

pub const DCTABLE: DCtabSet = DCtabSet {
    lum0: [
        DCtab { size: 1, len: 2 }, DCtab { size: 1, len: 2 }, DCtab { size: 1, len: 2 },
        DCtab { size: 1, len: 2 },
        DCtab { size: 1, len: 2 }, DCtab { size: 1, len: 2 }, DCtab { size: 1, len: 2 },
        DCtab { size: 1, len: 2 },
        DCtab { size: 2, len: 2 }, DCtab { size: 2, len: 2 }, DCtab { size: 2, len: 2 },
        DCtab { size: 2, len: 2 },
        DCtab { size: 2, len: 2 }, DCtab { size: 2, len: 2 }, DCtab { size: 2, len: 2 },
        DCtab { size: 2, len: 2 },
        DCtab { size: 0, len: 3 }, DCtab { size: 0, len: 3 }, DCtab { size: 0, len: 3 },
        DCtab { size: 0, len: 3 },
        DCtab { size: 3, len: 3 }, DCtab { size: 3, len: 3 }, DCtab { size: 3, len: 3 },
        DCtab { size: 3, len: 3 },
        DCtab { size: 4, len: 3 }, DCtab { size: 4, len: 3 }, DCtab { size: 4, len: 3 },
        DCtab { size: 4, len: 3 },
        DCtab { size: 5, len: 4 }, DCtab { size: 5, len: 4 },
        DCtab { size: 6, len: 5 }, DCtab { size: 0, len: 0 },
    ],
    lum1: [
        DCtab { size: 7, len: 6 }, DCtab { size: 7, len: 6 }, DCtab { size: 7, len: 6 },
        DCtab { size: 7, len: 6 },
        DCtab { size: 7, len: 6 }, DCtab { size: 7, len: 6 }, DCtab { size: 7, len: 6 },
        DCtab { size: 7, len: 6 },
        DCtab { size: 8, len: 7 }, DCtab { size: 8, len: 7 }, DCtab { size: 8, len: 7 },
        DCtab { size: 8, len: 7 },
        DCtab { size: 9, len: 8 }, DCtab { size: 9, len: 8 },
        DCtab { size: 10, len: 9 }, DCtab { size: 11, len: 9 },
    ],
    chrom0: [
        DCtab { size: 0, len: 2 }, DCtab { size: 0, len: 2 }, DCtab { size: 0, len: 2 },
        DCtab { size: 0, len: 2 },
        DCtab { size: 0, len: 2 }, DCtab { size: 0, len: 2 }, DCtab { size: 0, len: 2 },
        DCtab { size: 0, len: 2 },
        DCtab { size: 1, len: 2 }, DCtab { size: 1, len: 2 }, DCtab { size: 1, len: 2 },
        DCtab { size: 1, len: 2 },
        DCtab { size: 1, len: 2 }, DCtab { size: 1, len: 2 }, DCtab { size: 1, len: 2 },
        DCtab { size: 1, len: 2 },
        DCtab { size: 2, len: 2 }, DCtab { size: 2, len: 2 }, DCtab { size: 2, len: 2 },
        DCtab { size: 2, len: 2 },
        DCtab { size: 2, len: 2 }, DCtab { size: 2, len: 2 }, DCtab { size: 2, len: 2 },
        DCtab { size: 2, len: 2 },
        DCtab { size: 3, len: 3 }, DCtab { size: 3, len: 3 }, DCtab { size: 3, len: 3 },
        DCtab { size: 3, len: 3 },
        DCtab { size: 4, len: 4 }, DCtab { size: 4, len: 4 },
        DCtab { size: 5, len: 5 }, DCtab { size: 0, len: 0 },
    ],
    chrom1: [
        DCtab { size: 6, len: 6 }, DCtab { size: 6, len: 6 }, DCtab { size: 6, len: 6 },
        DCtab { size: 6, len: 6 },
        DCtab { size: 6, len: 6 }, DCtab { size: 6, len: 6 }, DCtab { size: 6, len: 6 },
        DCtab { size: 6, len: 6 },
        DCtab { size: 6, len: 6 }, DCtab { size: 6, len: 6 }, DCtab { size: 6, len: 6 },
        DCtab { size: 6, len: 6 },
        DCtab { size: 6, len: 6 }, DCtab { size: 6, len: 6 }, DCtab { size: 6, len: 6 },
        DCtab { size: 6, len: 6 },
        DCtab { size: 7, len: 7 }, DCtab { size: 7, len: 7 }, DCtab { size: 7, len: 7 },
        DCtab { size: 7, len: 7 },
        DCtab { size: 7, len: 7 }, DCtab { size: 7, len: 7 }, DCtab { size: 7, len: 7 },
        DCtab { size: 7, len: 7 },
        DCtab { size: 8, len: 8 }, DCtab { size: 8, len: 8 }, DCtab { size: 8, len: 8 },
        DCtab { size: 8, len: 8 },
        DCtab { size: 9, len: 9 }, DCtab { size: 9, len: 9 },
        DCtab { size: 10, len: 10 }, DCtab { size: 11, len: 10 },
    ],
};

/// DCT coefficient tables.
pub struct DCTtabSet {
    pub first: [DCTtab; 12],
    pub next: [DCTtab; 12],
    pub tab0: [DCTtab; 60],
    pub tab0a: [DCTtab; 252],
    pub tab1: [DCTtab; 8],
    pub tab1a: [DCTtab; 8],
    pub tab2: [DCTtab; 16],
    pub tab3: [DCTtab; 16],
    pub tab4: [DCTtab; 16],
    pub tab5: [DCTtab; 16],
    pub tab6: [DCTtab; 16],
}

const fn dct_const(run: u8, level: u8, len: u8) -> DCTtab {
    DCTtab { run, level, len }
}

pub const DCT: DCTtabSet = DCTtabSet {
    first: [
        dct_const(0, 2, 4), dct_const(2, 1, 4), dct_const(1, 1, 3), dct_const(1, 1, 3),
        dct_const(0, 1, 1), dct_const(0, 1, 1), dct_const(0, 1, 1), dct_const(0, 1, 1),
        dct_const(0, 1, 1), dct_const(0, 1, 1), dct_const(0, 1, 1), dct_const(0, 1, 1),
    ],
    next: [
        dct_const(0, 2, 4), dct_const(2, 1, 4), dct_const(1, 1, 3), dct_const(1, 1, 3),
        dct_const(64, 0, 2), dct_const(64, 0, 2), dct_const(64, 0, 2), dct_const(64, 0, 2),
        dct_const(0, 1, 2), dct_const(0, 1, 2), dct_const(0, 1, 2), dct_const(0, 1, 2),
    ],
    tab0: [
        dct_const(65, 0, 6), dct_const(65, 0, 6), dct_const(65, 0, 6), dct_const(65, 0, 6),
        dct_const(2, 2, 7), dct_const(2, 2, 7), dct_const(9, 1, 7), dct_const(9, 1, 7),
        dct_const(0, 4, 7), dct_const(0, 4, 7), dct_const(8, 1, 7), dct_const(8, 1, 7),
        dct_const(7, 1, 6), dct_const(7, 1, 6), dct_const(7, 1, 6), dct_const(7, 1, 6),
        dct_const(6, 1, 6), dct_const(6, 1, 6), dct_const(6, 1, 6), dct_const(6, 1, 6),
        dct_const(1, 2, 6), dct_const(1, 2, 6), dct_const(1, 2, 6), dct_const(1, 2, 6),
        dct_const(5, 1, 6), dct_const(5, 1, 6), dct_const(5, 1, 6), dct_const(5, 1, 6),
        dct_const(13, 1, 8), dct_const(0, 6, 8), dct_const(12, 1, 8), dct_const(11, 1, 8),
        dct_const(3, 2, 8), dct_const(1, 3, 8), dct_const(0, 5, 8), dct_const(10, 1, 8),
        dct_const(0, 3, 5), dct_const(0, 3, 5), dct_const(0, 3, 5), dct_const(0, 3, 5),
        dct_const(0, 3, 5), dct_const(0, 3, 5), dct_const(0, 3, 5), dct_const(0, 3, 5),
        dct_const(4, 1, 5), dct_const(4, 1, 5), dct_const(4, 1, 5), dct_const(4, 1, 5),
        dct_const(4, 1, 5), dct_const(4, 1, 5), dct_const(4, 1, 5), dct_const(4, 1, 5),
        dct_const(3, 1, 5), dct_const(3, 1, 5), dct_const(3, 1, 5), dct_const(3, 1, 5),
        dct_const(3, 1, 5), dct_const(3, 1, 5), dct_const(3, 1, 5), dct_const(3, 1, 5),
    ],
    tab0a: build_tab0a(),
    tab1: [
        dct_const(16, 1, 10), dct_const(5, 2, 10), dct_const(0, 7, 10), dct_const(2, 3, 10),
        dct_const(1, 4, 10), dct_const(15, 1, 10), dct_const(14, 1, 10), dct_const(4, 2, 10),
    ],
    tab1a: [
        dct_const(5, 2, 9), dct_const(5, 2, 9), dct_const(14, 1, 9), dct_const(14, 1, 9),
        dct_const(2, 4, 10), dct_const(16, 1, 10), dct_const(15, 1, 9), dct_const(15, 1, 9),
    ],
    tab2: [
        dct_const(0, 11, 12), dct_const(8, 2, 12), dct_const(4, 3, 12), dct_const(0, 10, 12),
        dct_const(2, 4, 12), dct_const(7, 2, 12), dct_const(21, 1, 12), dct_const(20, 1, 12),
        dct_const(0, 9, 12), dct_const(19, 1, 12), dct_const(18, 1, 12), dct_const(1, 5, 12),
        dct_const(3, 3, 12), dct_const(0, 8, 12), dct_const(6, 2, 12), dct_const(17, 1, 12),
    ],
    tab3: [
        dct_const(10, 2, 13), dct_const(9, 2, 13), dct_const(5, 3, 13), dct_const(3, 4, 13),
        dct_const(2, 5, 13), dct_const(1, 7, 13), dct_const(1, 6, 13), dct_const(0, 15, 13),
        dct_const(0, 14, 13), dct_const(0, 13, 13), dct_const(0, 12, 13), dct_const(26, 1, 13),
        dct_const(25, 1, 13), dct_const(24, 1, 13), dct_const(23, 1, 13), dct_const(22, 1, 13),
    ],
    tab4: [
        dct_const(0, 31, 14), dct_const(0, 30, 14), dct_const(0, 29, 14), dct_const(0, 28, 14),
        dct_const(0, 27, 14), dct_const(0, 26, 14), dct_const(0, 25, 14), dct_const(0, 24, 14),
        dct_const(0, 23, 14), dct_const(0, 22, 14), dct_const(0, 21, 14), dct_const(0, 20, 14),
        dct_const(0, 19, 14), dct_const(0, 18, 14), dct_const(0, 17, 14), dct_const(0, 16, 14),
    ],
    tab5: [
        dct_const(0, 40, 15), dct_const(0, 39, 15), dct_const(0, 38, 15), dct_const(0, 37, 15),
        dct_const(0, 36, 15), dct_const(0, 35, 15), dct_const(0, 34, 15), dct_const(0, 33, 15),
        dct_const(0, 32, 15), dct_const(1, 14, 15), dct_const(1, 13, 15), dct_const(1, 12, 15),
        dct_const(1, 11, 15), dct_const(1, 10, 15), dct_const(1, 9, 15), dct_const(1, 8, 15),
    ],
    tab6: [
        dct_const(1, 18, 16), dct_const(1, 17, 16), dct_const(1, 16, 16), dct_const(1, 15, 16),
        dct_const(6, 3, 16), dct_const(16, 2, 16), dct_const(15, 2, 16), dct_const(14, 2, 16),
        dct_const(13, 2, 16), dct_const(12, 2, 16), dct_const(11, 2, 16), dct_const(31, 1, 16),
        dct_const(30, 1, 16), dct_const(29, 1, 16), dct_const(28, 1, 16), dct_const(27, 1, 16),
    ],
};

const fn build_tab0a() -> [DCTtab; 252] {
    let mut t = [DCTtab { run: 0, level: 0, len: 0 }; 252];
    t[0] = DCTtab { run: 65, level: 0, len: 6 };
    t[1] = DCTtab { run: 65, level: 0, len: 6 };
    t[2] = DCTtab { run: 65, level: 0, len: 6 };
    t[3] = DCTtab { run: 65, level: 0, len: 6 };
    t[4] = DCTtab { run: 7, level: 1, len: 7 };
    t[5] = DCTtab { run: 7, level: 1, len: 7 };
    t[6] = DCTtab { run: 8, level: 1, len: 7 };
    t[7] = DCTtab { run: 8, level: 1, len: 7 };
    t[8] = DCTtab { run: 6, level: 1, len: 7 };
    t[9] = DCTtab { run: 6, level: 1, len: 7 };
    t[10] = DCTtab { run: 2, level: 2, len: 7 };
    t[11] = DCTtab { run: 2, level: 2, len: 7 };
    t[12] = DCTtab { run: 0, level: 7, len: 6 };
    t[13] = DCTtab { run: 0, level: 7, len: 6 };
    t[14] = DCTtab { run: 0, level: 7, len: 6 };
    t[15] = DCTtab { run: 0, level: 7, len: 6 };
    t[16] = DCTtab { run: 0, level: 6, len: 6 };
    t[17] = DCTtab { run: 0, level: 6, len: 6 };
    t[18] = DCTtab { run: 0, level: 6, len: 6 };
    t[19] = DCTtab { run: 0, level: 6, len: 6 };
    t[20] = DCTtab { run: 4, level: 1, len: 6 };
    t[21] = DCTtab { run: 4, level: 1, len: 6 };
    t[22] = DCTtab { run: 4, level: 1, len: 6 };
    t[23] = DCTtab { run: 4, level: 1, len: 6 };
    t[24] = DCTtab { run: 5, level: 1, len: 6 };
    t[25] = DCTtab { run: 5, level: 1, len: 6 };
    t[26] = DCTtab { run: 5, level: 1, len: 6 };
    t[27] = DCTtab { run: 5, level: 1, len: 6 };
    t[28] = DCTtab { run: 1, level: 5, len: 8 };
    t[29] = DCTtab { run: 11, level: 1, len: 8 };
    t[30] = DCTtab { run: 0, level: 11, len: 8 };
    t[31] = DCTtab { run: 0, level: 10, len: 8 };
    t[32] = DCTtab { run: 13, level: 1, len: 8 };
    t[33] = DCTtab { run: 12, level: 1, len: 8 };
    t[34] = DCTtab { run: 3, level: 2, len: 8 };
    t[35] = DCTtab { run: 1, level: 4, len: 8 };
    t[36] = DCTtab { run: 2, level: 1, len: 5 };
    t[37] = DCTtab { run: 2, level: 1, len: 5 };
    t[38] = DCTtab { run: 2, level: 1, len: 5 };
    t[39] = DCTtab { run: 2, level: 1, len: 5 };
    t[40] = DCTtab { run: 2, level: 1, len: 5 };
    t[41] = DCTtab { run: 2, level: 1, len: 5 };
    t[42] = DCTtab { run: 2, level: 1, len: 5 };
    t[43] = DCTtab { run: 2, level: 1, len: 5 };
    t[44] = DCTtab { run: 1, level: 2, len: 5 };
    t[45] = DCTtab { run: 1, level: 2, len: 5 };
    t[46] = DCTtab { run: 1, level: 2, len: 5 };
    t[47] = DCTtab { run: 1, level: 2, len: 5 };
    t[48] = DCTtab { run: 1, level: 2, len: 5 };
    t[49] = DCTtab { run: 1, level: 2, len: 5 };
    t[50] = DCTtab { run: 1, level: 2, len: 5 };
    t[51] = DCTtab { run: 1, level: 2, len: 5 };
    t[52] = DCTtab { run: 3, level: 1, len: 5 };
    t[53] = DCTtab { run: 3, level: 1, len: 5 };
    t[54] = DCTtab { run: 3, level: 1, len: 5 };
    t[55] = DCTtab { run: 3, level: 1, len: 5 };
    t[56] = DCTtab { run: 3, level: 1, len: 5 };
    t[57] = DCTtab { run: 3, level: 1, len: 5 };
    t[58] = DCTtab { run: 3, level: 1, len: 5 };
    t[59] = DCTtab { run: 3, level: 1, len: 5 };
    let mut i = 60;
    while i < 92 {
        t[i] = DCTtab { run: 1, level: 1, len: 3 };
        i += 1;
    }
    let mut i = 92;
    while i < 108 {
        t[i] = DCTtab { run: 64, level: 0, len: 4 };
        i += 1;
    }
    let mut i = 108;
    while i < 124 {
        t[i] = DCTtab { run: 0, level: 3, len: 4 };
        i += 1;
    }
    let mut i = 124;
    while i < 188 {
        t[i] = DCTtab { run: 0, level: 1, len: 2 };
        i += 1;
    }
    let mut i = 188;
    while i < 220 {
        t[i] = DCTtab { run: 0, level: 2, len: 3 };
        i += 1;
    }
    let mut i = 220;
    while i < 228 {
        t[i] = DCTtab { run: 0, level: 4, len: 5 };
        i += 1;
    }
    let mut i = 228;
    while i < 236 {
        t[i] = DCTtab { run: 0, level: 5, len: 5 };
        i += 1;
    }
    t[236] = DCTtab { run: 9, level: 1, len: 7 };
    t[237] = DCTtab { run: 9, level: 1, len: 7 };
    t[238] = DCTtab { run: 1, level: 3, len: 7 };
    t[239] = DCTtab { run: 1, level: 3, len: 7 };
    t[240] = DCTtab { run: 10, level: 1, len: 7 };
    t[241] = DCTtab { run: 10, level: 1, len: 7 };
    t[242] = DCTtab { run: 0, level: 8, len: 7 };
    t[243] = DCTtab { run: 0, level: 8, len: 7 };
    t[244] = DCTtab { run: 0, level: 9, len: 7 };
    t[245] = DCTtab { run: 0, level: 9, len: 7 };
    t[246] = DCTtab { run: 0, level: 12, len: 8 };
    t[247] = DCTtab { run: 0, level: 13, len: 8 };
    t[248] = DCTtab { run: 2, level: 3, len: 8 };
    t[249] = DCTtab { run: 4, level: 2, len: 8 };
    t[250] = DCTtab { run: 0, level: 14, len: 8 };
    t[251] = DCTtab { run: 0, level: 15, len: 8 };
    t
}

/// Zigzag scan patterns.
#[derive(Clone, Copy)]
pub struct Mpeg2ScanPack {
    pub norm: [u8; 64],
    pub alt: [u8; 64],
}

pub const MPEG2_SCAN: Mpeg2ScanPack = Mpeg2ScanPack {
    norm: [
        0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5,
        12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21, 28,
        35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51,
        58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
    ],
    alt: [
        0, 8, 16, 24, 1, 9, 2, 10, 17, 25, 32, 40, 48, 56, 57, 49,
        41, 33, 26, 18, 3, 11, 4, 12, 19, 27, 34, 42, 50, 58, 35, 43,
        51, 59, 20, 28, 5, 13, 6, 14, 21, 29, 36, 44, 52, 60, 37, 45,
        53, 61, 22, 30, 7, 15, 23, 31, 38, 46, 54, 62, 39, 47, 55, 63,
    ],
};

/// IDCT clipping lookup table.
pub const G_IDCT_CLIP_LUT: [u8; 1024] = make_clip_lut();

const fn make_clip_lut() -> [u8; 1024] {
    let mut lut = [0u8; 1024];
    let mut i: i32 = -384;
    while i < 640 {
        let v = if i < 0 { 0 } else if i > 255 { 255 } else { i };
        lut[(i + 384) as usize] = v as u8;
        i += 1;
    }
    lut
}

// ---------------------------------------------------------------------------
// Macroblock / decoder state
// ---------------------------------------------------------------------------

/// Decoded 8-bit macroblock: luma 16x16, chroma Cb/Cr 8x8.
#[derive(Clone, Copy, Default)]
pub struct Macroblock8 {
    pub Y: [[u8; 16]; 16],
    pub Cb: [[u8; 8]; 8],
    pub Cr: [[u8; 8]; 8],
}

/// 16-bit (pre-IDCT) macroblock.
#[derive(Clone, Copy, Default)]
pub struct Macroblock16 {
    pub Y: [[i16; 16]; 16],
    pub Cb: [[i16; 8]; 8],
    pub Cr: [[i16; 8]; 8],
}

/// RGBA8888 pixel.
#[derive(Clone, Copy, Default)]
pub struct Rgba32 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

/// Decoded RGBA8888 macroblock.
#[derive(Clone, Copy, Default)]
pub struct MacroblockRgb32 {
    pub c: [[Rgba32; 16]; 16],
}

/// RGBA5551 pixel.
#[derive(Clone, Copy, Default)]
pub struct Rgb16 {
    pub r: u16, // 5 bits
    pub g: u16, // 5 bits
    pub b: u16, // 5 bits
    pub a: u16, // 1 bit
}

impl Rgb16 {
    /// Pack the four 16-bit fields into the hardware RGBA5551 layout.
    pub fn pack(&self) -> u16 {
        (self.r & 0x1f) | ((self.g & 0x1f) << 5) | ((self.b & 0x1f) << 10) | ((self.a & 1) << 15)
    }
}

/// Decoded RGBA5551 macroblock.
#[derive(Clone, Copy, Default)]
pub struct MacroblockRgb16 {
    pub c: [[Rgb16; 16]; 16],
}

/// Full decoder state carried between macroblocks.
#[derive(Clone)]
pub struct Decoder {
    pub dct_block: [i16; 64],
    pub niq: [u8; 64],
    pub iq: [u8; 64],

    pub mb8: Macroblock8,
    pub mb16: Macroblock16,
    pub rgb32: MacroblockRgb32,
    pub rgb16: MacroblockRgb16,

    pub ipu0_data: u32,
    pub ipu0_idx: u32,

    pub quantizer_scale: i32,

    pub coding_type: i32,
    pub dc_dct_pred: [i16; 3],
    pub intra_dc_precision: i32,
    pub picture_structure: i32,
    pub frame_pred_frame_dct: i32,
    pub concealment_motion_vectors: i32,
    pub q_scale_type: i32,
    pub intra_vlc_format: i32,
    pub top_field_first: i32,
    pub sgn: i32,
    pub dte: i32,
    pub ofm: i32,
    pub macroblock_modes: i32,
    pub dcr: i32,
    pub coded_block_pattern: i32,

    pub scantype: bool,
    pub mpeg1: i32,
}

impl Decoder {
    /// Point the IPU0 output pointer at the start of `obj`. The offset is
    /// required to be 16-byte aligned because the IPU works in quadwords.
    pub fn set_output_to<T>(&mut self, _obj_offset_words: u32, _obj_size_words: u32) {
        // Helper for the C++ template `SetOutputTo`. Use `set_output_to_offset`
        // for the actual offset arithmetic.
    }

    /// Set the IPU0 output pointer given the byte offset / 16 of an object
    /// inside the macroblock.
    pub fn set_output_to_offset(&mut self, mb_offset_bytes: u32, obj_size_bytes: u32) {
        self.ipu0_idx = mb_offset_bytes / 16;
        self.ipu0_data = obj_size_bytes / 16;
    }

    /// Pointer to the next IPU0 data quadword, expressed as a slice of u8.
    pub fn get_ipu_data_ptr(&self) -> *const u8 {
        let base = &self.mb8 as *const Macroblock8 as *const u8;
        unsafe { base.add((self.ipu0_idx * 16) as usize) }
    }

    /// Advance the output pointer after a successful FIFO write.
    pub fn advance_ipu_data_by(&mut self, amt: u32) {
        debug_assert!(self.ipu0_data >= amt);
        self.ipu0_idx += amt;
        self.ipu0_data -= amt;
    }
}

// ---------------------------------------------------------------------------
// Bitstream pointer (mirrors `tIPU_BP`)
// ---------------------------------------------------------------------------

/// Bit pointer / internal 2-QWC ringbuffer. Matches `tIPU_BP` in `IPU.h`.
#[derive(Clone)]
pub struct IpuBp {
    pub internal_qwc: [u128; 2],
    pub bp: u32,  // bit position 0..=256
    pub ifc: u32, // input FIFO counter (0..=8)
    pub fp: u32,  // internal QWC fill status (0..=2)
}

impl Default for IpuBp {
    fn default() -> Self {
        Self {
            internal_qwc: [0u128, 0u128],
            bp: 0,
            ifc: 0,
            fp: 0,
        }
    }
}

impl IpuBp {
    /// Try to keep `bits` more bits available past the current `bp`. When the
    /// input FIFO cannot supply enough quadwords the function returns `false`
    /// (matching the C++ behaviour).
    pub fn fill_buffer(&mut self, bits: u32) -> bool {
        while (self.fp * 128) < (self.bp + bits) {
            // The IPU FIFO ringbuffer's "shift" path is handled by the FIFO
            // state machine (see `IPU_Fifo_Input::read`). The standalone
            // module here returns false when the buffer is starved, which
            // matches the C++ semantics where Advance/FillBuffer return false
            // if the FIFO has nothing more to give.
            return false;
        }
        true
    }

    /// Align to the next byte boundary (rounded up to the nearest byte).
    pub fn align(&mut self) {
        self.bp = (self.bp + 7) & !7;
        self.advance(0);
    }

    /// Advance the bit pointer by `bits`, draining the FIFO as needed.
    pub fn advance(&mut self, bits: u32) {
        let _ = self.fill_buffer(bits);
        self.bp += bits;
        if self.bp >= 128 {
            self.bp -= 128;
            if self.fp == 2 {
                self.internal_qwc[0] = self.internal_qwc[1];
                self.fp = 1;
            } else if self.fp == 1 {
                self.fp = 0;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// FIFO (input + output)
// ---------------------------------------------------------------------------

/// 8-QWC input FIFO feeding the bitstream reader.
#[derive(Clone)]
pub struct IpuFifoInput {
    pub data: [u32; 32],
    pub readpos: i32,
    pub writepos: i32,
}

impl Default for IpuFifoInput {
    fn default() -> Self {
        Self {
            data: [0; 32],
            readpos: 0,
            writepos: 0,
        }
    }
}

/// Output FIFO holding decoded macroblocks.
#[derive(Clone)]
pub struct IpuFifoOutput {
    pub data: [u32; 32],
    pub readpos: i32,
    pub writepos: i32,
}

impl Default for IpuFifoOutput {
    fn default() -> Self {
        Self {
            data: [0; 32],
            readpos: 0,
            writepos: 0,
        }
    }
}

/// In + out pair (matches `IPU_Fifo` in `IPU_Fifo.h`).
#[derive(Clone, Default)]
pub struct IpuFifo {
    pub input: IpuFifoInput,
    pub output: IpuFifoOutput,
}

impl IpuFifo {
    pub fn init(&mut self) {
        self.input.readpos = 0;
        self.input.writepos = 0;
        self.output.readpos = 0;
        self.output.writepos = 0;
        self.input.data = [0; 32];
        self.output.data = [0; 32];
    }

    pub fn clear(&mut self) {
        self.input.clear();
        self.output.clear();
    }
}

impl IpuFifoInput {
    pub fn clear(&mut self) {
        self.data = [0; 32];
        self.readpos = 0;
        self.writepos = 0;
    }

    /// Write up to `size` quadwords from `p_mem` into the FIFO. Returns the
    /// number of quadwords actually written.
    pub fn write(&mut self, p_mem: *const u32, size: i32, current_ifc: u32) -> i32 {
        unsafe {
            let transfer_size = cmp::min(size, 8 - current_ifc as i32);
            if transfer_size == 0 {
                return 0;
            }
            let first_words = cmp::min(32 - self.writepos, transfer_size << 2);
            let second_words = (transfer_size << 2) - first_words;

            ptr::copy_nonoverlapping(
                p_mem,
                self.data.as_mut_ptr().add(self.writepos as usize),
                first_words as usize,
            );
            if second_words > 0 {
                ptr::copy_nonoverlapping(
                    p_mem.add(first_words as usize),
                    self.data.as_mut_ptr(),
                    second_words as usize,
                );
            }
            self.writepos = (self.writepos + (transfer_size << 2)) & 31;
            transfer_size
        }
    }

    /// Read a single quadword out of the FIFO. Returns 0 if no data is
    /// available, otherwise 1 and stores the data at `value` (16 bytes).
    pub fn read(&mut self, value: *mut u128) -> i32 {
        unsafe {
            ptr::copy_nonoverlapping(
                self.data.as_ptr().add(self.readpos as usize) as *const u128,
                value,
                1,
            );
            self.readpos = (self.readpos + 4) & 31;
            1
        }
    }
}

impl IpuFifoOutput {
    pub fn clear(&mut self) {
        self.data = [0; 32];
        self.readpos = 0;
        self.writepos = 0;
    }

    /// Write `size` quadwords from `value` into the FIFO. Returns the number
    /// of quadwords actually written.
    pub fn write(&mut self, value: *const u32, size: u32, current_ofc: u32) -> u32 {
        debug_assert!(size > 0);
        let transfer_size = cmp::min(size as i32, 8 - current_ofc as i32);
        if transfer_size <= 0 {
            return 0;
        }
        unsafe {
            let first_words = cmp::min(32 - self.writepos, transfer_size << 2);
            let second_words = (transfer_size << 2) - first_words;
            ptr::copy_nonoverlapping(
                value,
                self.data.as_mut_ptr().add(self.writepos as usize),
                first_words as usize,
            );
            if second_words > 0 {
                ptr::copy_nonoverlapping(
                    value.add(first_words as usize),
                    self.data.as_mut_ptr(),
                    second_words as usize,
                );
            }
            self.writepos = (self.writepos + (transfer_size << 2)) & 31;
        }
        transfer_size as u32
    }

    /// Read `size` quadwords from the FIFO into `value`.
    pub fn read(&mut self, value: *mut u32, size: u32) {
        let first_words = cmp::min(32 - self.readpos, (size << 2) as i32);
        let second_words = (size << 2) as i32 - first_words;
        unsafe {
            ptr::copy_nonoverlapping(
                self.data.as_ptr().add(self.readpos as usize),
                value,
                first_words as usize,
            );
            if second_words > 0 {
                ptr::copy_nonoverlapping(
                    self.data.as_ptr(),
                    value.add(first_words as usize),
                    second_words as usize,
                );
            }
        }
        self.readpos = (self.readpos + ((size << 2) as i32)) & 31;
    }
}

// ---------------------------------------------------------------------------
// DMA channel and status types (from IPUdma.h / IPU_Fifo.cpp)
// ---------------------------------------------------------------------------

/// DMA channel registers (CHCR, QWC, TADR, MADR).
#[derive(Clone, Copy, Default)]
pub struct IpuDmaChannel {
    /// Channel control register.
    pub chcr_str: bool,
    pub chcr_tte: bool,
    pub chcr_tie: bool,
    pub chcr_mod: u8,
    /// Quadword counter.
    pub qwc: u32,
    /// Tag address.
    pub tadr: u32,
    /// Memory address.
    pub madr: u32,
    pub tag: u32,
}

/// IPU->EE DMA status.
#[derive(Clone, Copy, Default)]
pub struct IpuDmaStatus {
    pub in_progress: bool,
    pub dma_finished: bool,
}

/// IPU core status shared between the worker and DMA paths.
#[derive(Clone, Copy, Default)]
pub struct IpuStatus {
    pub data_requested: bool,
    pub waiting_on_ipu_from: bool,
    pub waiting_on_ipu_to: bool,
}

// ---------------------------------------------------------------------------
// IPU register file (mirrors `IPUregisters` and the bitfield unions)
// ---------------------------------------------------------------------------

/// IPU_CMD register: low 32 = DATA, high 32 = BUSY flag.
#[derive(Clone, Copy, Default)]
pub struct IpuCmdReg {
    pub data: u32,
    pub busy: u32,
}

impl IpuCmdReg {
    pub fn as_u64(&self) -> u64 {
        (self.data as u64) | ((self.busy as u64) << 32)
    }
}

/// IPU_CTRL register bitfield.
#[derive(Clone, Copy, Default)]
pub struct IpuCtrlReg {
    pub ifc: u32,  // bits 0..3
    pub ofc: u32,  // bits 4..7
    pub cbp: u32,  // bits 8..13
    pub ecd: u32,  // bit 14
    pub scd: u32,  // bit 15
    pub idp: u32,  // bits 16..17
    pub as_: u32,  // bit 20
    pub ivf: u32,  // bit 21
    pub qst: u32,  // bit 22
    pub mp1: u32,  // bit 23
    pub pct: u32,  // bits 24..26
    pub rst: u32,  // bit 31
    pub busy: u32, // bit 30
}

impl IpuCtrlReg {
    /// Pack to a single 32-bit register word.
    pub fn as_u32(&self) -> u32 {
        (self.ifc & 0xF)
            | ((self.ofc & 0xF) << 4)
            | ((self.cbp & 0x3F) << 8)
            | ((self.ecd & 1) << 14)
            | ((self.scd & 1) << 15)
            | ((self.idp & 0x3) << 16)
            | ((self.as_ & 1) << 20)
            | ((self.ivf & 1) << 21)
            | ((self.qst & 1) << 22)
            | ((self.mp1 & 1) << 23)
            | ((self.pct & 0x7) << 24)
            | ((self.busy & 1) << 30)
            | ((self.rst & 1) << 31)
    }

    /// Apply the EE's `write` semantics: keep reserved bits, mask R/W bits.
    pub fn write(&mut self, value: u32) {
        let current = self.as_u32();
        let new_val = (value & 0x47f3_0000) | (current & 0x8000_ffff);
        self.unpack(new_val);
    }

    /// Reset clears most bits, keeping only the low FIFO counter field.
    pub fn reset(&mut self) {
        self.ifc = 0;
        self.ofc = 0;
        self.cbp = 0;
        self.ecd = 0;
        self.scd = 0;
        self.idp = 0;
        self.as_ = 0;
        self.ivf = 0;
        self.qst = 0;
        self.mp1 = 0;
        self.pct = 0;
        self.rst = 0;
        self.busy = 0;
    }

    fn unpack(&mut self, v: u32) {
        self.ifc = v & 0xF;
        self.ofc = (v >> 4) & 0xF;
        self.cbp = (v >> 8) & 0x3F;
        self.ecd = (v >> 14) & 1;
        self.scd = (v >> 15) & 1;
        self.idp = (v >> 16) & 0x3;
        self.as_ = (v >> 20) & 1;
        self.ivf = (v >> 21) & 1;
        self.qst = (v >> 22) & 1;
        self.mp1 = (v >> 23) & 1;
        self.pct = (v >> 24) & 0x7;
        self.rst = (v >> 31) & 1;
        self.busy = (v >> 30) & 1;
    }
}

/// Top register / topbusy.
#[derive(Clone, Copy, Default)]
pub struct IpuTopReg {
    pub top: u32,
    pub top_busy: u32,
}

/// Bit-pointer register view exposed on the EE side.
#[derive(Clone, Copy, Default)]
pub struct IpuBpReg {
    pub bp: u32,
    pub ifc: u32,
    pub fp: u32,
}

impl IpuBpReg {
    pub fn as_u32(&self) -> u32 {
        (self.bp & 0x7f) | ((self.ifc & 0xf) << 8) | ((self.fp & 0x3) << 16)
    }
}

/// The hardware-visible IPU register file.
#[derive(Clone, Copy, Default)]
pub struct IpuRegisters {
    pub cmd: IpuCmdReg,
    pub ctrl: IpuCtrlReg,
    pub bp: IpuBpReg,
    pub top: u32,
    pub top_busy: u32,
}

impl IpuRegisters {
    pub fn set_top_busy(&mut self) {
        self.top_busy = 0x8000_0000;
    }

    pub fn set_data_busy(&mut self) {
        self.cmd.busy = 0x8000_0000;
        self.top_busy = 0x8000_0000;
    }
}

/// Per-instruction IPU command state (matches `tIPU_cmd`).
#[derive(Clone, Copy, Default)]
pub struct IpuCmd {
    pub index: i32,
    pub pos: [i32; 6],
    pub current: u32,
}

impl IpuCmd {
    pub fn clear(&mut self) {
        *self = Self {
            current: 0xffff_ffff,
            ..Default::default()
        };
    }

    /// Extract the 4-bit SCE-IPU opcode from `current`.
    pub fn cmd(&self) -> u32 {
        self.current & 0xf
    }
}

/// 16-bit CLUT entry (matches `rgb16_t`).
#[derive(Clone, Copy, Default)]
pub struct Rgb16Clut {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}
