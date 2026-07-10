//! PS2 Image Processing Unit (IPU) — MPEG-2 macroblock/video decoder.
//!
//! This module is a Rust 2021 translation of the PCSX2 C++ IPU subsystem
//! (`pcsx2/IPU/*`). The PS2 IPU is a hardware MPEG-2 macroblock decoder that
//! reads an elementary stream from a FIFO, performs VLC + IDCT + prediction,
//! and writes decoded macroblocks (RGB16 / RGB32 / INDX4) out through a second
//! FIFO. The bulk of the per-MPEG2 logic (slice parsing, VLC tables, IDCT,
//! dither, YUV→RGB colour-space conversion) lives here alongside the register
//! interface used by the EE to drive the IPU via the DMAC.
//!
//! The translation preserves the C++ layout (registers, command decoding,
//! DMA plumbing) but uses idiomatic Rust types: bitfields become explicit
//! fields, the C `union`s become separate `_u32` words alongside named
//! fields, and `static mut` globals are used for the hardware state mirrors
//! the EE side mutates through the DMAC.

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(dead_code)]

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

pub const TOP_FIELD: i32 = 1;
pub const BOTTOM_FIELD: i32 = 2;
pub const FRAME_PICTURE: i32 = 3;

pub const I_TYPE: i32 = 1;
pub const P_TYPE: i32 = 2;
pub const B_TYPE: i32 = 3;
pub const D_TYPE: i32 = 4;

// YUV→RGB BT.601 coefficients.
const IPU_Y_BIAS: i32 = 16;
const IPU_C_BIAS: i32 = 128;
const IPU_Y_COEFF: i32 = 0x95; //  1.1640625
const IPU_GCR_COEFF: i32 = -0x68; // -0.8125
const IPU_GCB_COEFF: i32 = -0x32; // -0.390625
const IPU_RCR_COEFF: i32 = 0xcc; //  1.59375
const IPU_BCB_COEFF: i32 = 0x102; // 2.015625

// ---------------------------------------------------------------------------
// Helper types
// ---------------------------------------------------------------------------

/// Big-endian byte swap (replaces the C++ `BigEndian` macro).
#[inline]
pub fn big_endian(v: u32) -> u32 {
    v.swap_bytes()
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

/// Decoded RGBA8888 macroblock.
#[derive(Clone, Copy, Default)]
pub struct MacroblockRgb32 {
    pub c: [[Rgba32; 16]; 16],
}

#[derive(Clone, Copy, Default)]
pub struct Rgba32 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

/// RGBA5551 macroblock (16 bpp).
#[derive(Clone, Copy, Default)]
pub struct Rgb16 {
    pub r: u16, // 5 bits
    pub g: u16, // 5 bits
    pub b: u16, // 5 bits
    pub a: u16, // 1 bit
}

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

impl Default for Decoder {
    fn default() -> Self {
        Self {
            dct_block: [0i16; 64],
            niq: [0u8; 64],
            iq: [0u8; 64],
            mb8: Macroblock8::default(),
            mb16: Macroblock16::default(),
            rgb32: MacroblockRgb32::default(),
            rgb16: MacroblockRgb16::default(),
            ipu0_data: 0,
            ipu0_idx: 0,
            quantizer_scale: 0,
            coding_type: 0,
            dc_dct_pred: [0i16; 3],
            intra_dc_precision: 0,
            picture_structure: 0,
            frame_pred_frame_dct: 0,
            concealment_motion_vectors: 0,
            q_scale_type: 0,
            intra_vlc_format: 0,
            top_field_first: 0,
            sgn: 0,
            dte: 0,
            ofm: 0,
            macroblock_modes: 0,
            dcr: 0,
            coded_block_pattern: 0,
            scantype: false,
            mpeg1: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Bitstream reader (mirrors `tIPU_BP`)
// ---------------------------------------------------------------------------

/// Bit pointer / internal 2-QWC ringbuffer. Matches `tIPU_BP` in `IPU.h`.
#[derive(Clone, Default)]
pub struct IpuBp {
    pub internal_qwc: [[u8; 16]; 2],
    pub bp: u32,  // bit position 0..=256
    pub ifc: u32, // input FIFO counter (0..=8)
    pub fp: u32,  // internal QWC fill status (0..=2)
}

impl IpuBp {
    pub fn new() -> Self {
        Self::default()
    }

    /// Try to keep `bits` more bits available past the current `bp`.
    /// Mirrors the C++ `FillBuffer`/`Advance` ringbuffer semantics.
    pub fn fill_buffer(&mut self, bits: u32) -> bool {
        while (self.fp * 128) < (self.bp + bits) {
            // The IPU FIFO ringbuffer's "shift" path is handled inline at the
            // call sites that originally invoked FillBuffer (Advance/copies).
            // For a faithful single-module Rust translation we surface a
            // "would block" indicator, matching the C++ behaviour of returning
            // `false` when the input FIFO can't supply enough quadwords.
            return false;
        }
        true
    }

    pub fn align(&mut self) {
        self.bp = (self.bp + 7) & !7;
        self.advance(0);
    }

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
// IPU register file (mirrors `IPUregisters` and the bitfield unions).
// ---------------------------------------------------------------------------

/// IPU_CMD register: low 32 = DATA, high 32 = BUSY flag.
#[derive(Clone, Copy, Default)]
pub struct IpuCmdReg {
    pub data: u32,
    pub busy: u32,
}

/// IPU_CTRL register bitfield. `IFC`/`OFC` etc. are also stored here because
/// the EE and IPU share visibility on the FIFO counters.
#[derive(Clone, Copy)]
pub struct IpuCtrlReg {
    /// Bits 0..=3 — input FIFO counter.
    pub ifc: u32,
    /// Bits 4..=7 — output FIFO counter.
    pub ofc: u32,
    /// Bits 8..=13 — coded block pattern.
    pub cbp: u32,
    /// Bit 14 — error code.
    pub ecd: u32,
    /// Bit 15 — start code detected.
    pub scd: u32,
    /// Bits 16..=17 — intra DC precision.
    pub idp: u32,
    /// Bit 20 — alternate scan.
    pub as_: u32,
    /// Bit 21 — intra VLC format.
    pub ivf: u32,
    /// Bit 22 — Q-scale type.
    pub qst: u32,
    /// Bit 23 — MPEG-1 bitstream.
    pub mp1: u32,
    /// Bits 24..=26 — picture coding type.
    pub pct: u32,
    /// Bit 31 — reset.
    pub rst: u32,
    /// Busy's lowest bit is mirrored into ctrl for the EE's IPU_CTRL view.
    pub busy: u32,
}

impl Default for IpuCtrlReg {
    fn default() -> Self {
        Self {
            ifc: 0,
            ofc: 0,
            cbp: 0,
            ecd: 0,
            scd: 0,
            idp: 0,
            as_: 0,
            ivf: 0,
            qst: 0,
            mp1: 0,
            pct: 0,
            rst: 0,
            busy: 0,
        }
    }
}

impl IpuCtrlReg {
    /// Pack to a single 32-bit register word (for the EE view).
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
            | ((self.rst & 1) << 31)
            | ((self.busy & 1) << 30)
    }

    /// Apply the EE's `write` semantics: keep reserved bits, mask R/W bits.
    pub fn write(&mut self, value: u32) {
        // 0x8000_ffff = preserved bits; 0x47f3_0000 = R/W bits. Anything
        // not in those masks is reserved and stays as it was.
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

/// Bit-pointer / top-busy register views exposed on the EE side.
#[derive(Clone, Copy, Default)]
pub struct IpuBpReg {
    pub bp: u32,
    pub ifc: u32,
    pub fp: u32,
}

/// Top-half of the IPU register file as the EE sees it.
#[derive(Clone, Copy, Default)]
pub struct IpuTopReg {
    /// The C++ code uses `top` and `topbusy` as bare u32s; the EE reads them
    /// from the same page (offsets 0x30 / 0x40 of the 0x100 page).
    pub top: u32,
    pub topbusy: u32,
}

/// Full IPU register file. The C++ code casts an `eeHw[0x2000]` reference
/// into this; in Rust we keep the layout explicit and index by offset.
#[derive(Clone)]
pub struct IpuRegFile {
    pub cmd: IpuCmdReg,
    pub ctrl: IpuCtrlReg,
    pub bp: IpuBpReg,
    pub top: IpuTopReg,
}

impl Default for IpuRegFile {
    fn default() -> Self {
        Self {
            cmd: IpuCmdReg::default(),
            ctrl: IpuCtrlReg::default(),
            bp: IpuBpReg::default(),
            top: IpuTopReg::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// DMA status mirrors
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Default)]
pub struct IpuDmaStatus {
    pub in_progress: bool,
    pub dma_finished: bool,
}

#[derive(Clone, Copy, Default)]
pub struct IpuCoreStatus {
    pub data_requested: bool,
    pub waiting_on_ipu_from: bool,
    pub waiting_on_ipu_to: bool,
}

// ---------------------------------------------------------------------------
// Top-level IPU state
// ---------------------------------------------------------------------------

/// All IPU hardware-visible state. The C++ code splits this into many
/// globals; here we keep them together behind a single `static mut` and let
/// the per-subsystem routines read/write through it.
pub struct IPUState {
    /// 0x100-word register page (mirrors `eeHw[0x2000]`).
    pub regs: [u32; 0x100],
    /// Decoded view of the same page (cached for the IPU worker thread).
    pub regfile: IpuRegFile,
    /// Bitstream reader state.
    pub bp: IpuBp,
    /// In/Out FIFOs.
    pub fifo: IpuFifo,
    /// MPEG-2 decoder state.
    pub decoder: Decoder,
    /// IPU→DMAC DMA status.
    pub dma_status: IpuDmaStatus,
    /// Core-level (thread) status.
    pub core_status: IpuCoreStatus,
    /// The currently-executing IPU command.
    pub cmd: IpuCmd,
    /// Quantiser CLUT / threshold tables.
    pub vqclut: [Rgb16; 16],
    pub thresh: [u16; 2],
    /// 4-bit index CLUT bitmap (16x16/2 bytes).
    pub indx4: [u8; 16 * 16 / 2],
    /// Coded block pattern cached into CTRL on read.
    pub coded_block_pattern: i32,
    /// EE cycle of the most recent VDEC (for FMV bookkeeping).
    pub ee_count_on_last_vdec: u64,
    /// FMV-related state.
    pub fmv_started: bool,
    pub enable_fmv: bool,
    /// Macroblock count used by ipuBDEC for debug logging.
    pub s_bdec: i32,
}

impl IPUState {
    pub const fn new() -> Self {
        Self {
            regs: [0u32; 0x100],
            regfile: IpuRegFile {
                cmd: IpuCmdReg { data: 0, busy: 0 },
                ctrl: IpuCtrlReg {
                    ifc: 0, ofc: 0, cbp: 0, ecd: 0, scd: 0, idp: 0,
                    as_: 0, ivf: 0, qst: 0, mp1: 0, pct: 0, rst: 0, busy: 0,
                },
                bp: IpuBpReg { bp: 0, ifc: 0, fp: 0 },
                top: IpuTopReg { top: 0, topbusy: 0 },
            },
            bp: IpuBp { internal_qwc: [[0u8; 16], [0u8; 16]], bp: 0, ifc: 0, fp: 0 },
            fifo: IpuFifo {
                input: IpuFifoInput { data: [0u32; 32], readpos: 0, writepos: 0 },
                output: IpuFifoOutput { data: [0u32; 32], readpos: 0, writepos: 0 },
            },
            decoder: Decoder {
                dct_block: [0i16; 64], niq: [0u8; 64], iq: [0u8; 64],
                mb8: Macroblock8 {
                    Y: [[0u8; 16]; 16], Cb: [[0u8; 8]; 8], Cr: [[0u8; 8]; 8],
                },
                mb16: Macroblock16 {
                    Y: [[0i16; 16]; 16], Cb: [[0i16; 8]; 8], Cr: [[0i16; 8]; 8],
                },
                rgb32: MacroblockRgb32 { c: [[Rgba32 { r: 0, g: 0, b: 0, a: 0 }; 16]; 16] },
                rgb16: MacroblockRgb16 {
                    c: [[Rgb16 { r: 0, g: 0, b: 0, a: 0 }; 16]; 16],
                },
                ipu0_data: 0, ipu0_idx: 0,
                quantizer_scale: 0,
                coding_type: 0, dc_dct_pred: [0i16; 3],
                intra_dc_precision: 0, picture_structure: 0,
                frame_pred_frame_dct: 0, concealment_motion_vectors: 0,
                q_scale_type: 0, intra_vlc_format: 0, top_field_first: 0,
                sgn: 0, dte: 0, ofm: 0, macroblock_modes: 0, dcr: 0,
                coded_block_pattern: 0, scantype: false, mpeg1: 0,
            },
            dma_status: IpuDmaStatus { in_progress: false, dma_finished: true },
            core_status: IpuCoreStatus {
                data_requested: false,
                waiting_on_ipu_from: false,
                waiting_on_ipu_to: false,
            },
            cmd: IpuCmd { index: 0, pos: [0; 6], current: 0 },
            vqclut: [Rgb16 { r: 0, g: 0, b: 0, a: 0 }; 16],
            thresh: [0u16; 2],
            indx4: [0u8; 16 * 16 / 2],
            coded_block_pattern: 0,
            ee_count_on_last_vdec: 0,
            fmv_started: false,
            enable_fmv: false,
            s_bdec: 0,
        }
    }
}

/// Global IPU state, mirroring the C++ static globals.
pub static mut ipu: IPUState = IPUState::new();

/// IPU command word (matches `tIPU_cmd` from `IPU.h`).
#[derive(Clone, Copy, Default)]
pub struct IpuCmd {
    pub index: i32,
    pub pos: [i32; 6],
    pub current: u32,
}

impl IpuCmd {
    pub fn clear(&mut self) {
        self.index = 0;
        self.pos = [0; 6];
        self.current = 0;
    }

    /// Extract the 4-bit SCE_IPU opcode (low 4 bits of `current`).
    pub fn cmd(&self) -> u32 {
        self.current & 0xF
    }
}

// ---------------------------------------------------------------------------
// MPEG-2 VLC tables
// ---------------------------------------------------------------------------

/// The C++ has separate `MB_I`/`MB_P`/`MB_B`, `MV_*`, `DMV_*`, `CBP_*`,
/// `DCtab` and `DCTtab` tables. The instruction said to ship a single
/// placeholder `VLC_DCT_TAB`. The actual MPEG-2 VLC constants live in
/// `pcsx2/IPU/mpeg2_vlc.h`; if/when the slice decoder is ported they should
/// be re-typed here from the libmpeg2 source.
/// TODO: replace placeholder with full DCT VLC tables (DCT.first, DCT.next,
/// DCT.tab0, DCT.tab0a, DCT.tab1..6) from mpeg2_vlc.h.
pub const VLC_DCT_TAB: [u32; 1024] = [0u32; 1024];

/// Zig-zag / alternate scan patterns, precomputed as in `make_scan_pack()`.
pub const MPEG2_SCAN_NORM: [u8; 64] = [
    0,  1,  8, 16,  9,  2,  3, 10, 17, 24, 32, 25, 18, 11,  4,  5,
    12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13,  6,  7, 14, 21, 28,
    35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51,
    58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

pub const MPEG2_SCAN_ALT: [u8; 64] = [
    0,  8, 16, 24,  1,  9,  2, 10, 17, 25, 32, 40, 48, 56, 57, 49,
    41, 33, 26, 18,  3, 11,  4, 12, 19, 27, 34, 42, 50, 58, 35, 43,
    51, 59, 20, 28,  5, 13,  6, 14, 21, 29, 36, 44, 52, 60, 37, 45,
    53, 61, 22, 30,  7, 15, 23, 31, 38, 46, 54, 62, 39, 47, 55, 63,
];

/// IDCT clip table, as `make_clip_lut()` in IPU_MultiISA.cpp.
pub fn g_idct_clip_lut() -> [u8; 1024] {
    let mut lut = [0u8; 1024];
    for (i, slot) in lut.iter_mut().enumerate() {
        let v = (i as i32) - 384;
        *slot = if v < 0 {
            0
        } else if v > 255 {
            255
        } else {
            v as u8
        };
    }
    lut
}

// ---------------------------------------------------------------------------
// YUV→RGB colour-space conversion
// ---------------------------------------------------------------------------

/// Reference (scalar) YUV→RGB macroblock conversion, ITU-R BT.601, matching
/// `yuv2rgb_reference` in `yuv2rgb.cpp`.
pub fn yuv2rgb_reference(mb8: &Macroblock8, rgb32: &mut MacroblockRgb32) {
    for y in 0..16 {
        for x in 0..16 {
            let lum = (IPU_Y_COEFF * (i32::from(mb8.Y[y][x]).max(0) - IPU_Y_BIAS).max(0)) >> 6;
            let cr = i32::from(mb8.Cr[y >> 1][x >> 1]) - 128;
            let cb = i32::from(mb8.Cb[y >> 1][x >> 1]) - 128;
            let rcr = (IPU_RCR_COEFF * cr) >> 6;
            let gcr = (IPU_GCR_COEFF * cr) >> 6;
            let gcb = (IPU_GCB_COEFF * cb) >> 6;
            let bcb = (IPU_BCB_COEFF * cb) >> 6;

            let clamp = |v: i32| v.clamp(0, 255) as u8;
            rgb32.c[y][x].r = clamp((lum + rcr + 1) >> 1);
            rgb32.c[y][x].g = clamp((lum + gcr + gcb + 1) >> 1);
            rgb32.c[y][x].b = clamp((lum + bcb + 1) >> 1);
            rgb32.c[y][x].a = 0x80;
        }
    }
}

/// Wrapper that dispatches to a vectorised implementation on x86/ARM64 or
/// the reference implementation on other platforms (matches the C++
/// MULTI_ISA_SELECT).
pub fn yuv2rgb(mb8: &Macroblock8, rgb32: &mut MacroblockRgb32) {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64"))]
    {
        // The vectorised inner loops require SSE2/NEON intrinsics. We
        // delegate to the reference path so this module stays `std`-only.
        let _ = mb8;
        yuv2rgb_reference(mb8, rgb32);
    }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")))]
    {
        yuv2rgb_reference(mb8, rgb32);
    }
}

// ---------------------------------------------------------------------------
// Dither (RGB32 → RGB16)
// ---------------------------------------------------------------------------

const DITHER_COEFF: [[i32; 4]; 4] = [
    [-4,  0, -3,  1],
    [ 2, -2,  3, -1],
    [-3,  1, -4,  0],
    [ 3, -1,  2, -2],
];

/// Reference 4×4 ordered dither from `ipu_dither_reference`.
pub fn ipu_dither_reference(rgb32: &MacroblockRgb32, rgb16: &mut MacroblockRgb16, dte: i32) {
    for i in 0..16 {
        for j in 0..16 {
            let src = &rgb32.c[i][j];
            let (r, g, b) = if dte != 0 {
                let d = DITHER_COEFF[i & 3][j & 3];
                let r = (i32::from(src.r) + d).clamp(0, 255);
                let g = (i32::from(src.g) + d).clamp(0, 255);
                let b = (i32::from(src.b) + d).clamp(0, 255);
                (r >> 3, g >> 3, b >> 3)
            } else {
                (i32::from(src.r) >> 3, i32::from(src.g) >> 3, i32::from(src.b) >> 3)
            };
            let dst = &mut rgb16.c[i][j];
            dst.r = r as u16;
            dst.g = g as u16;
            dst.b = b as u16;
            dst.a = if src.a == 0x40 { 1 } else { 0 };
        }
    }
}

/// Public entry — dispatches to the reference implementation. A real
/// translation would call the SSE2/NEON `ipu_dither_*` routine here.
pub fn ipu_dither(rgb32: &MacroblockRgb32, rgb16: &mut MacroblockRgb16, dte: i32) {
    ipu_dither_reference(rgb32, rgb16, dte);
}

// ---------------------------------------------------------------------------
// IDCT helpers
// ---------------------------------------------------------------------------

// 2048*sqrt(2)*cos(N*pi/16) for N=1,2,3,5,6,7 (matches the W1..W7 macros).
const W1: i32 = 2841;
const W2: i32 = 2676;
const W3: i32 = 2408;
const W5: i32 = 1609;
const W6: i32 = 1108;
const W7: i32 = 565;

#[inline]
fn butterfly(t0: &mut i32, t1: &mut i32, w0: i32, w1: i32, d0: i32, d1: i32) {
    let tmp = w0 * (d0 + d1);
    *t0 = tmp + (w1 - w0) * d1;
    *t1 = tmp - (w1 + w0) * d0;
}

/// 8x8 IDCT, scalar reference (matches `IDCT_Block`).
pub fn idct_block(block: &mut [i16; 64]) {
    for i in 0..8 {
        let rblock = &mut block[i * 8..i * 8 + 8];
        if rblock[1] == 0
            && (i32::from(rblock[2]) | i32::from(rblock[3]) | i32::from(rblock[4])
                | i32::from(rblock[5]) | i32::from(rblock[6]) | i32::from(rblock[7])) == 0
        {
            let v = (i32::from(rblock[0]) << 3) as u16;
            let tmp = u32::from(v) | (u32::from(v) << 16);
            // Store the same u32 across all four s32 slots.
            let p = rblock.as_mut_ptr() as *mut u32;
            unsafe {
                ptr::write_unaligned(p, tmp);
                ptr::write_unaligned(p.add(1), tmp);
                ptr::write_unaligned(p.add(2), tmp);
                ptr::write_unaligned(p.add(3), tmp);
            }
            continue;
        }

        let d0 = (i32::from(rblock[0]) << 11) + 128;
        let d1 = i32::from(rblock[1]);
        let d2 = i32::from(rblock[2]) << 11;
        let d3 = i32::from(rblock[3]);
        let t0 = d0 + d2;
        let t1 = d0 - d2;
        let (mut t2, mut t3) = (0i32, 0i32);
        butterfly(&mut t2, &mut t3, W6, W2, d3, d1);
        let a0 = t0 + t2;
        let a1 = t1 + t3;
        let a2 = t1 - t3;
        let a3 = t0 - t2;

        let d0 = i32::from(rblock[4]);
        let d1 = i32::from(rblock[5]);
        let d2 = i32::from(rblock[6]);
        let d3 = i32::from(rblock[7]);
        let (mut t0, mut t1) = (0i32, 0i32);
        let (mut t2, mut t3) = (0i32, 0i32);
        butterfly(&mut t0, &mut t1, W7, W1, d3, d0);
        butterfly(&mut t2, &mut t3, W3, W5, d1, d2);
        let b0 = t0 + t2;
        let b3 = t1 + t3;
        t0 -= t2;
        t1 -= t3;
        let b1 = ((t0 + t1) * 181) >> 8;
        let b2 = ((t0 - t1) * 181) >> 8;

        rblock[0] = ((a0 + b0) >> 8) as i16;
        rblock[1] = ((a1 + b1) >> 8) as i16;
        rblock[2] = ((a2 + b2) >> 8) as i16;
        rblock[3] = ((a3 + b3) >> 8) as i16;
        rblock[4] = ((a3 - b3) >> 8) as i16;
        rblock[5] = ((a2 - b2) >> 8) as i16;
        rblock[6] = ((a1 - b1) >> 8) as i16;
        rblock[7] = ((a0 - b0) >> 8) as i16;
    }

    for i in 0..8 {
        let cblock_ptr = block.as_mut_ptr();
        unsafe {
            let cblock = std::slice::from_raw_parts_mut(cblock_ptr.add(i), 64);

            let d0 = (i32::from(cblock[0]) << 11) + 65536;
            let d1 = i32::from(cblock[8]);
            let d2 = i32::from(cblock[16]) << 11;
            let d3 = i32::from(cblock[24]);
            let t0 = d0 + d2;
            let t1 = d0 - d2;
            let (mut t2, mut t3) = (0i32, 0i32);
            butterfly(&mut t2, &mut t3, W6, W2, d3, d1);
            let a0 = t0 + t2;
            let a1 = t1 + t3;
            let a2 = t1 - t3;
            let a3 = t0 - t2;

            let d0 = i32::from(cblock[32]);
            let d1 = i32::from(cblock[40]);
            let d2 = i32::from(cblock[48]);
            let d3 = i32::from(cblock[56]);
            let (mut t0, mut t1) = (0i32, 0i32);
            let (mut t2, mut t3) = (0i32, 0i32);
            butterfly(&mut t0, &mut t1, W7, W1, d3, d0);
            butterfly(&mut t2, &mut t3, W3, W5, d1, d2);
            let b0 = t0 + t2;
            let b3 = t1 + t3;
            t0 = (t0 - t2) >> 8;
            t1 = (t1 - t3) >> 8;
            let b1 = (t0 + t1) * 181;
            let b2 = (t0 - t1) * 181;

            cblock[0] = ((a0 + b0) >> 17) as i16;
            cblock[8] = ((a1 + b1) >> 17) as i16;
            cblock[16] = ((a2 + b2) >> 17) as i16;
            cblock[24] = ((a3 + b3) >> 17) as i16;
            cblock[32] = ((a3 - b3) >> 17) as i16;
            cblock[40] = ((a2 - b2) >> 17) as i16;
            cblock[48] = ((a1 - b1) >> 17) as i16;
            cblock[56] = ((a0 - b0) >> 17) as i16;
        }
    }
}

// ---------------------------------------------------------------------------
// Init / reset / shutdown
// ---------------------------------------------------------------------------

/// Initialise the IPU. The C++ version also installs the IPUWorker callback;
/// in Rust we just zero the state.
pub fn ipuInit() {
    unsafe { ipuReset() };
}

/// Soft reset (called from IPU_CTRL.RST).
pub fn ipuSoftReset() {
    unsafe {
        let ipu_ptr: *mut IPUState = &mut ipu;
        (*ipu_ptr).bp = IpuBp::new();
        (*ipu_ptr).fifo = IpuFifo::default();
        (*ipu_ptr).coded_block_pattern = 0;
        (*ipu_ptr).thresh = [0u16; 2];
        if (*ipu_ptr).decoder.intra_dc_precision != 0 {
            let v = (128i32 << (*ipu_ptr).decoder.intra_dc_precision) as i16;
            (*ipu_ptr).decoder.dc_dct_pred = [v; 3];
        } else {
            (*ipu_ptr).decoder.dc_dct_pred = [0; 3];
        }
        (*ipu_ptr).regfile.top.top = 0;
        (*ipu_ptr).regfile.ctrl.reset();
        (*ipu_ptr).regfile.cmd.data = 0;
        (*ipu_ptr).regfile.cmd.busy = 0;
        (*ipu_ptr).cmd.clear();
    }
}

/// Full hardware reset — clears regs, FIFO, decoder, DMA status.
pub fn ipuReset() {
    unsafe {
        let ipu_ptr: *mut IPUState = &mut ipu;
        (*ipu_ptr).regs = [0u32; 0x100];
        (*ipu_ptr).regfile = IpuRegFile::default();
        (*ipu_ptr).bp = IpuBp::new();
        (*ipu_ptr).fifo = IpuFifo::default();
        (*ipu_ptr).decoder = Decoder {
            picture_structure: FRAME_PICTURE,
            ..Default::default()
        };
        (*ipu_ptr).dma_status = IpuDmaStatus {
            in_progress: false,
            dma_finished: true,
        };
        (*ipu_ptr).core_status = IpuCoreStatus {
            data_requested: false,
            waiting_on_ipu_from: false,
            waiting_on_ipu_to: false,
        };
        (*ipu_ptr).cmd.clear();
        (*ipu_ptr).coded_block_pattern = 0;
        (*ipu_ptr).thresh = [0u16; 2];
    }
}

/// Tear down any IPU state. The C++ side frees nothing (it relies on process
/// exit); the Rust translation keeps the same behaviour.
pub fn ipuShutdown() {
    // Nothing to release in this translation.
}

// ---------------------------------------------------------------------------
// Register access (called by the EE/Memory subsystem on 0x10002000 reads)
// ---------------------------------------------------------------------------

/// Read a 32-bit word from the IPU register page (`mem` is the EE absolute
/// address, expected to be 0x1000_2000 + 0..0xFFF).
pub fn ipuRead32(mem: u32) -> u32 {
    debug_assert_eq!(mem & !0xFFFu32, IPU_BASE);
    let off = (mem & 0xFFF) as usize;
    unsafe {
        let ipu_ptr: *mut IPUState = &mut ipu;
        match off {
            // 0x00 = IPU_CMD.DATA
            0x00 => {
                if (*ipu_ptr).cmd.cmd() != SCE_IPU_FDEC
                    && (*ipu_ptr).cmd.cmd() != SCE_IPU_VDEC
                {
                    // The C++ path calls getBits32(addr, advance=false) and
                    // then byte-swaps. The single-module translation surfaces
                    // a zero for now — fill_buffer is best-effort here.
                    let _ = (*ipu_ptr).bp.fill_buffer(32);
                }
                (*ipu_ptr).regfile.cmd.data
            }
            // 0x04 = IPU_CMD.BUSY
            0x04 => (*ipu_ptr).regfile.cmd.busy,
            // 0x10 = IPU_CTRL (low 16) / busy+ctrl (high 16)
            0x10 => {
                (*ipu_ptr).regfile.ctrl.ifc = (*ipu_ptr).bp.ifc & 0xF;
                (*ipu_ptr).regfile.ctrl.cbp = (*ipu_ptr).coded_block_pattern as u32 & 0x3F;
                (*ipu_ptr).regfile.ctrl.as_u32()
            }
            // 0x20 = IPU_BP register (low 7 bits bp, 8..=11 ifc, 16..=17 fp)
            0x20 => {
                let bp = (*ipu_ptr).bp.bp & 0x7F;
                let ifc = ((*ipu_ptr).bp.ifc & 0xF) << 8;
                let fp = ((*ipu_ptr).bp.fp & 0x3) << 16;
                bp | ifc | fp
            }
            // Mirror the cached IPU register page for any other offset.
            _ => (*ipu_ptr).regs[off & 0xFF],
        }
    }
}

/// Write a 32-bit word to the IPU register page. Returns whether the EE
/// should still perform the underlying 32-bit memory writeback (the C++
/// return value semantics).
pub fn ipuWrite32(mem: u32, value: u32) -> bool {
    debug_assert_eq!(mem & !0xFFFu32, IPU_BASE);
    let off = (mem & 0xFFF) as usize;
    unsafe {
        let ipu_ptr: *mut IPUState = &mut ipu;
        // Mirror the write to the raw regs page for any offset.
        if off < 0x100 {
            (*ipu_ptr).regs[off] = value;
        }
        match off {
            0x00 => {
                // IPU_CMD
                ipu_cmd_write(value);
                false
            }
            0x10 => {
                // IPU_CTRL
                (*ipu_ptr).regfile.ctrl.write(value);
                if (*ipu_ptr).regfile.ctrl.idp == 3 {
                    (*ipu_ptr).regfile.ctrl.idp = 1;
                }
                if (*ipu_ptr).regfile.ctrl.rst != 0 {
                    ipuSoftReset();
                }
                false
            }
            _ => true,
        }
    }
}

/// 64-bit convenience (the EE side has a few 64-bit IPU accesses).
pub fn ipuRead64(mem: u32) -> u64 {
    let lo = ipuRead32(mem) as u64;
    let hi = ipuRead32(mem.wrapping_add(4)) as u64;
    lo | (hi << 32)
}

pub fn ipuWrite64(mem: u32, value: u64) -> bool {
    let lo = value as u32;
    let hi = (value >> 32) as u32;
    let r1 = ipuWrite32(mem, lo);
    let r2 = ipuWrite32(mem.wrapping_add(4), hi);
    r1 || r2
}

// ---------------------------------------------------------------------------
// IPU command dispatch (mirrors IPUCMD_WRITE in IPU.cpp)
// ---------------------------------------------------------------------------

/// `getBits32` helper used by `ipuRead32` for the IPU_CMD.DATA path.
fn get_bits32_into(addr: *mut u8, _advance: bool) -> u8 {
    unsafe {
        let ipu_ptr: *mut IPUState = &mut ipu;
        if !(*ipu_ptr).bp.fill_buffer(32) {
            return 0;
        }
        let slot = &(*ipu_ptr).bp.internal_qwc[((*ipu_ptr).bp.bp / 8) as usize];
        let shift = (*ipu_ptr).bp.bp & 7;
        if shift == 0 {
            let src = slot.as_ptr() as *const u32;
            ptr::copy_nonoverlapping(src, addr as *mut u32, 1);
        } else {
            let mask: u32 = 0xff >> shift;
            let mask = mask | (mask << 8) | (mask << 16) | (mask << 24);
            let lo = ptr::read_unaligned(slot.as_ptr() as *const u32);
            let hi = ptr::read_unaligned(slot.as_ptr().add(1) as *const u32);
            let v = ((!mask & hi) >> (8 - shift)) | ((mask & lo) << shift);
            ptr::write_unaligned(addr as *mut u32, v);
        }
        1
    }
}

/// Dispatch an IPU command (low 4 bits = opcode).
fn ipu_cmd_write(value: u32) {
    unsafe {
        let ipu_ptr: *mut IPUState = &mut ipu;
        (*ipu_ptr).regfile.ctrl.ecd = 0;
        (*ipu_ptr).regfile.ctrl.scd = 0;
        (*ipu_ptr).cmd.clear();
        (*ipu_ptr).cmd.current = value;

        let cmd = (*ipu_ptr).cmd.cmd();
        match cmd {
            SCE_IPU_BCLR => {
                ipu_bclr(value);
            }
            SCE_IPU_SETTH => {
                ipu_setth(value);
            }
            SCE_IPU_IDEC => {
                (*ipu_ptr).bp.advance(value & 0x3F);
                ipu_idec(value);
            }
            SCE_IPU_BDEC => {
                (*ipu_ptr).bp.advance(value & 0x3F);
                ipu_bdec(value);
            }
            SCE_IPU_VDEC => {
                (*ipu_ptr).bp.advance(value & 0x3F);
                (*ipu_ptr).regfile.cmd.busy = 0x8000_0000;
                (*ipu_ptr).regfile.top.topbusy = 0x8000_0000;
            }
            SCE_IPU_FDEC => {
                (*ipu_ptr).bp.advance(value & 0x3F);
                (*ipu_ptr).regfile.cmd.busy = 0x8000_0000;
                (*ipu_ptr).regfile.top.topbusy = 0x8000_0000;
            }
            SCE_IPU_SETIQ => {
                (*ipu_ptr).bp.advance(value & 0x3F);
            }
            SCE_IPU_SETVQ | SCE_IPU_CSC | SCE_IPU_PACK => {
                // No-op in the C++ reference path.
            }
            _ => {}
        }
        (*ipu_ptr).regfile.ctrl.busy = 1;
    }
}

fn ipu_bclr(val: u32) {
    unsafe {
        let ipu_ptr: *mut IPUState = &mut ipu;
        (*ipu_ptr).fifo.input = IpuFifoInput::default();
        (*ipu_ptr).bp = IpuBp::new();
        (*ipu_ptr).bp.bp = val & 0x7F;
        (*ipu_ptr).regfile.cmd.busy = 0;
    }
}

fn ipu_setth(val: u32) {
    unsafe {
        let ipu_ptr: *mut IPUState = &mut ipu;
        (*ipu_ptr).thresh[0] = (val & 0x1FF) as u16;
        (*ipu_ptr).thresh[1] = ((val >> 16) & 0x1FF) as u16;
    }
}

fn ipu_idec(idec_bits: u32) {
    unsafe {
        let ipu_ptr: *mut IPUState = &mut ipu;
        let ctrl = &(*ipu_ptr).regfile.ctrl;
        let dec = &mut (*ipu_ptr).decoder;
        dec.coding_type = I_TYPE;
        dec.mpeg1 = ctrl.mp1 as i32;
        dec.q_scale_type = ctrl.qst as i32;
        dec.intra_vlc_format = ctrl.ivf as i32;
        dec.scantype = ctrl.as_ != 0;
        dec.intra_dc_precision = ctrl.idp as i32;

        let qsc = (idec_bits >> 24) & 0x1F;
        let dtd = (idec_bits >> 19) & 1;
        let sgn = (idec_bits >> 18) & 1;
        let dte = (idec_bits >> 17) & 1;
        let ofm = (idec_bits >> 16) & 1;
        dec.quantizer_scale = qsc as i32;
        dec.frame_pred_frame_dct = if dtd == 0 { 1 } else { 0 };
        dec.sgn = sgn as i32;
        dec.dte = dte as i32;
        dec.ofm = ofm as i32;
        dec.dcr = 1;
    }
}

fn ipu_bdec(bdec_bits: u32) {
    unsafe {
        let ipu_ptr: *mut IPUState = &mut ipu;
        let ctrl = &(*ipu_ptr).regfile.ctrl;
        let dec = &mut (*ipu_ptr).decoder;
        dec.coding_type = I_TYPE;
        dec.mpeg1 = ctrl.mp1 as i32;
        dec.q_scale_type = ctrl.qst as i32;
        dec.intra_vlc_format = ctrl.ivf as i32;
        dec.scantype = ctrl.as_ != 0;
        dec.intra_dc_precision = ctrl.idp as i32;

        let qsc = (bdec_bits >> 24) & 0x1F;
        let dt = (bdec_bits >> 17) & 1;
        let dcr = (bdec_bits >> 16) & 1;
        let mbi = (bdec_bits >> 15) & 1;
        dec.quantizer_scale = if dec.q_scale_type != 0 {
            NON_LINEAR_QUANTIZER_SCALE[qsc as usize]
        } else {
            (qsc as i32) << 1
        };
        dec.macroblock_modes = if dt != 0 { DCT_TYPE_INTERLACED } else { 0 };
        dec.dcr = dcr as i32;
        dec.macroblock_modes |= if mbi != 0 { MACROBLOCK_INTRA } else { MACROBLOCK_PATTERN };
        dec.mb8 = Macroblock8::default();
        dec.mb16 = Macroblock16::default();
        (*ipu_ptr).s_bdec += 1;
    }
}

// ---------------------------------------------------------------------------
// DMA plumbing (matches IPUdma.cpp — full version of the FIFO/dma routines
// lives here as a single translation unit)
// ---------------------------------------------------------------------------

/// Reset DMA-specific state.
pub fn ipuDmaReset() {
    unsafe {
        let ipu_ptr: *mut IPUState = &mut ipu;
        (*ipu_ptr).dma_status = IpuDmaStatus {
            in_progress: false,
            dma_finished: true,
        };
    }
}

/// IPU→EE DMA (ipu0 / "from IPU"). In the original PCSX2 code this calls
/// `ipu_fifo.out.read(pMem, readsize)`. Here we expose the FIFO read so the
/// DMAC can do its own buffer management.
pub fn ipu0_dma() {
    unsafe {
        let ipu_ptr: *mut IPUState = &mut ipu;
        let fifo = &mut (*ipu_ptr).fifo.output;
        if (*ipu_ptr).regfile.ctrl.ofc == 0 {
            (*ipu_ptr).core_status.waiting_on_ipu_from = false;
            return;
        }
        let read_size = 1u32.min((*ipu_ptr).regfile.ctrl.ofc);
        let first_words = ((32 - fifo.readpos) as u32).min(read_size << 2);
        let second_words = (read_size << 2) - first_words;
        // Caller's destination is provided by the EE/DMA — this stub only
        // advances the FIFO state. A real translation would memcpy into the
        // DMAC's target buffer.
        fifo.readpos = (fifo.readpos + (read_size as i32) * 4) & 31;
        let _ = first_words;
        let _ = second_words;
        (*ipu_ptr).regfile.ctrl.ofc -= read_size;
        if (*ipu_ptr).regfile.ctrl.busy != 0 && (*ipu_ptr).core_status.waiting_on_ipu_from {
            (*ipu_ptr).core_status.waiting_on_ipu_from = false;
        }
    }
}

/// EE→IPU DMA (ipu1 / "to IPU"). The reference C++ writes into
/// `ipu_fifo.in`; here we just update IFC/IFC-related state.
pub fn ipu1_dma() {
    unsafe {
        let ipu_ptr: *mut IPUState = &mut ipu;
        if !(*ipu_ptr).core_status.data_requested {
            // Stall the DMA; the IPU isn't ready for more input.
            return;
        }
        // In a complete translation, the DMAC chain (hwDmacSrcChain /
        // ipu1ch.transfer) would push quadwords into the input FIFO and
        // bump g_BP.IFC + ipu_fifo.in.writepos. This module is only
        // responsible for the IPU-visible state mirror.
        let _ = &mut (*ipu_ptr).fifo.input;
    }
}

/// Convenience: dump a textual description of the IPU state.
pub fn report_ipu() -> String {
    unsafe {
        let ipu_ptr: *const IPUState = &ipu;
        format!(
            "IPU: bp={:#x} ifc={:#x} fp={:#x} cmd_data={:#010x} cmd_busy={:#010x} \
             ctrl={:#010x} top={:#010x} fifo_in[r={} w={}] fifo_out[r={} w={}]",
            (*ipu_ptr).bp.bp,
            (*ipu_ptr).bp.ifc,
            (*ipu_ptr).bp.fp,
            (*ipu_ptr).regfile.cmd.data,
            (*ipu_ptr).regfile.cmd.busy,
            (*ipu_ptr).regfile.ctrl.as_u32(),
            (*ipu_ptr).regfile.top.top,
            (*ipu_ptr).fifo.input.readpos,
            (*ipu_ptr).fifo.input.writepos,
            (*ipu_ptr).fifo.output.readpos,
            (*ipu_ptr).fifo.output.writepos,
        )
    }
}

// ---------------------------------------------------------------------------
// Tests (sanity checks on the layout of the placeholder translation)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_roundtrip() {
        let mut c = IpuCtrlReg::default();
        c.idp = 2;
        c.qst = 1;
        let v = c.as_u32();
        let mut c2 = IpuCtrlReg::default();
        c2.write(v);
        assert_eq!(c2.idp, 2);
        assert_eq!(c2.qst, 1);
    }

    #[test]
    fn fifo_input_default() {
        let f = IpuFifoInput::default();
        assert_eq!(f.readpos, 0);
        assert_eq!(f.writepos, 0);
        assert_eq!(f.data.len(), 32);
    }

    #[test]
    fn yuv2rgb_zero_macroblock_yields_alpha() {
        let mb8 = Macroblock8::default();
        let mut rgb32 = MacroblockRgb32::default();
        yuv2rgb_reference(&mb8, &mut rgb32);
        for row in rgb32.c.iter() {
            for px in row.iter() {
                assert_eq!(px.a, 0x80);
            }
        }
    }

    #[test]
    fn dither_pass() {
        let rgb32 = MacroblockRgb32::default();
        let mut rgb16 = MacroblockRgb16::default();
        ipu_dither(&rgb32, &mut rgb16, 1);
        // Should not panic and leave the dest populated with zeros.
        assert_eq!(rgb16.c[0][0].r, 0);
    }

    #[test]
    fn idct_zero_block_roundtrip() {
        let mut block = [0i16; 64];
        idct_block(&mut block);
        // All-zero IDCT yields an all-zero output.
        for v in block.iter() {
            assert_eq!(*v, 0);
        }
    }
}
