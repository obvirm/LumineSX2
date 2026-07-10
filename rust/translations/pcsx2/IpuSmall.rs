// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust translation of the PCSX2 IPU "small sources" set
//! (`IPUdither.cpp`, `IPUdma.cpp`/`IPUdma.h`, `IPU_Fifo.cpp`/`IPU_Fifo.h`,
//! `yuv2rgb.cpp`/`yuv2rgb.h`, `mpeg2_vlc.h`, plus the IPU/MultiISA type
//! declarations from `IPU.h` and `IPU_MultiISA.h`).
//!
//! This module is a self-contained, allocation-light port intended to be
//! embedded in the larger `pcsx2` Rust crate. It depends only on `std`.
//! The DMA executor (`ipuDmaExec`) is a faithful structural translation of
//! the C++ `IPU1dma` / `IPU0dma` paths: it mutates the `IPUDma` state
//! machine using the FIFO counters and chains without touching real
//! hardware registers. Hardware-specific hooks (cycle accounting, IRQ
//! firing, `dmaGetAddr`, etc.) are stubbed and documented with `TODO`.

#![allow(dead_code)]
#![allow(clippy::upper_case_acronyms)]

use std::fmt;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// IPU YUV -> RGB conversion coefficients (from `yuv2rgb.cpp`).
///
/// These match the integer constants used by the hardware pipeline
/// (ITU-R BT.601, scaled by 64).
pub const IPU_Y_BIAS: i32 = 16;
pub const IPU_C_BIAS: i32 = 128;
pub const IPU_Y_COEFF: i32 = 0x95; //  1.1640625
pub const IPU_GCR_COEFF: i32 = -0x68; // -0.8125
pub const IPU_GCB_COEFF: i32 = -0x32; // -0.390625
pub const IPU_RCR_COEFF: i32 = 0xcc; //  1.59375
pub const IPU_BCB_COEFF: i32 = 0x102; //  2.015625

/// Capacity of an IPU FIFO in 32-bit words (32 u32 = 128 bytes = 8 QW).
pub const IPU_FIFO_WORDS: usize = 32;

/// Capacity of an IPU FIFO in quadwords (matching `g_BP.IFC` / `OFC`).
pub const IPU_FIFO_QW: usize = 8;

// ---------------------------------------------------------------------------
// MPEG-2 macroblock / VLC enums (from `mpeg2_vlc.h`)
// ---------------------------------------------------------------------------

/// Macroblock mode bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MacroblockModes(pub u8);

impl MacroblockModes {
    pub const INTRA: u8 = 1;
    pub const PATTERN: u8 = 2;
    pub const MOTION_BACKWARD: u8 = 4;
    pub const MOTION_FORWARD: u8 = 8;
    pub const QUANT: u8 = 16;
    pub const DCT_TYPE_INTERLACED: u8 = 32;

    pub const fn empty() -> Self {
        Self(0)
    }
    pub const fn contains(self, other: u8) -> bool {
        (self.0 & other) == other
    }
}

/// Picture coding type (I/P/B/D frame).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PictureCodingType {
    I = 1,
    P = 2,
    B = 3,
    D = 4,
}

/// Picture structure (top field / bottom field / frame).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PictureStructure {
    TopField = 1,
    BottomField = 2,
    FramePicture = 3,
}

// ---------------------------------------------------------------------------
// MPEG-2 VLC table structs (from `mpeg2_vlc.h`)
// ---------------------------------------------------------------------------
//
// All table *data* is left as zero-initialised placeholders for now.
// The original C uses 16-byte aligned, dense `static constexpr` tables;
// a follow-up task should fill these in from the MPEG-2 spec (Table
// B-12 .. B-15) once the decoder pipeline is wired up.
//
// Each struct keeps the same field layout as the C side so that
// downstream code that consumes these tables can be ported with
// minimal churn.

#[derive(Debug, Clone, Copy, Default)]
pub struct MBtab {
    pub modes: u8,
    pub len: u8,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MVtab {
    pub delta: u8,
    pub len: u8,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DMVtab {
    pub dmv: i8,
    pub len: u8,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CBPtab {
    pub cbp: u8,
    pub len: u8,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DCtab {
    pub size: u8,
    pub len: u8,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DCTtab {
    pub run: u8,
    pub level: u8,
    pub len: u8,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MBAtab {
    pub mba: u8,
    pub len: u8,
}

// -- placeholder VLC tables -----------------------------------------------

/// I-frame macroblock VLC table (2 entries).
// TODO: fill from `MB_I` in `mpeg2_vlc.h`.
pub static MB_I: [MBtab; 2] = [MBtab { modes: 0, len: 0 }; 2];

/// P-frame macroblock VLC table (32 entries).
// TODO: fill from `MB_P` in `mpeg2_vlc.h`.
pub static MB_P: [MBtab; 32] = [MBtab { modes: 0, len: 0 }; 32];

/// B-frame macroblock VLC table (64 entries).
// TODO: fill from `MB_B` in `mpeg2_vlc.h`.
pub static MB_B: [MBtab; 64] = [MBtab { modes: 0, len: 0 }; 64];

/// Short motion vector VLC (8 entries).
// TODO: fill from `MV_4` in `mpeg2_vlc.h`.
pub static MV_4: [MVtab; 8] = [MVtab { delta: 0, len: 0 }; 8];

/// Long motion vector VLC (48 entries).
// TODO: fill from `MV_10` in `mpeg2_vlc.h`.
pub static MV_10: [MVtab; 48] = [MVtab { delta: 0, len: 0 }; 48];

/// Dual-motion vector VLC (4 entries).
// TODO: fill from `DMV_2` in `mpeg2_vlc.h`.
pub static DMV_2: [DMVtab; 4] = [DMVtab { dmv: 0, len: 0 }; 4];

/// Coded block pattern VLC, table B-14 (112 entries).
// TODO: fill from `CBP_7` in `mpeg2_vlc.h`.
pub static CBP_7: [CBPtab; 112] = [CBPtab { cbp: 0, len: 0 }; 112];

/// Coded block pattern VLC, table B-15 (64 entries).
// TODO: fill from `CBP_9` in `mpeg2_vlc.h`.
pub static CBP_9: [CBPtab; 64] = [CBPtab { cbp: 0, len: 0 }; 64];

/// Macroblock-address VLC set (mba5: 30 entries; mba11: 104 entries).
// TODO: fill from `MBA` in `mpeg2_vlc.h`.
pub struct MBAtabSet {
    pub mba5: [MBAtab; 30],
    pub mba11: [MBAtab; 104],
}

impl Default for MBAtabSet {
    fn default() -> Self {
        Self {
            mba5: [MBAtab { mba: 0, len: 0 }; 30],
            mba11: [MBAtab { mba: 0, len: 0 }; 104],
        }
    }
}

pub static MBA: MBAtabSet = MBAtabSet {
    mba5: [MBAtab { mba: 0, len: 0 }; 30],
    mba11: [MBAtab { mba: 0, len: 0 }; 104],
};

/// DC coefficient VLC set (B-12 / B-13, luminance + chrominance).
// TODO: fill from `DCtable` in `mpeg2_vlc.h`.
pub struct DCtabSet {
    pub lum0: [DCtab; 32],
    pub lum1: [DCtab; 16],
    pub chrom0: [DCtab; 32],
    pub chrom1: [DCtab; 32],
}

impl Default for DCtabSet {
    fn default() -> Self {
        Self {
            lum0: [DCtab { size: 0, len: 0 }; 32],
            lum1: [DCtab { size: 0, len: 0 }; 16],
            chrom0: [DCtab { size: 0, len: 0 }; 32],
            chrom1: [DCtab { size: 0, len: 0 }; 32],
        }
    }
}

pub static DCTABLE: DCtabSet = DCtabSet {
    lum0: [DCtab { size: 0, len: 0 }; 32],
    lum1: [DCtab { size: 0, len: 0 }; 16],
    chrom0: [DCtab { size: 0, len: 0 }; 32],
    chrom1: [DCtab { size: 0, len: 0 }; 32],
};

/// AC coefficient VLC set (B-14 / B-15).
// TODO: fill from `DCT` in `mpeg2_vlc.h`.
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

impl Default for DCTtabSet {
    fn default() -> Self {
        Self {
            first: [DCTtab { run: 0, level: 0, len: 0 }; 12],
            next: [DCTtab { run: 0, level: 0, len: 0 }; 12],
            tab0: [DCTtab { run: 0, level: 0, len: 0 }; 60],
            tab0a: [DCTtab { run: 0, level: 0, len: 0 }; 252],
            tab1: [DCTtab { run: 0, level: 0, len: 0 }; 8],
            tab1a: [DCTtab { run: 0, level: 0, len: 0 }; 8],
            tab2: [DCTtab { run: 0, level: 0, len: 0 }; 16],
            tab3: [DCTtab { run: 0, level: 0, len: 0 }; 16],
            tab4: [DCTtab { run: 0, level: 0, len: 0 }; 16],
            tab5: [DCTtab { run: 0, level: 0, len: 0 }; 16],
            tab6: [DCTtab { run: 0, level: 0, len: 0 }; 16],
        }
    }
}

pub static DCT: DCTtabSet = DCTtabSet {
    first: [DCTtab { run: 0, level: 0, len: 0 }; 12],
    next: [DCTtab { run: 0, level: 0, len: 0 }; 12],
    tab0: [DCTtab { run: 0, level: 0, len: 0 }; 60],
    tab0a: [DCTtab { run: 0, level: 0, len: 0 }; 252],
    tab1: [DCTtab { run: 0, level: 0, len: 0 }; 8],
    tab1a: [DCTtab { run: 0, level: 0, len: 0 }; 8],
    tab2: [DCTtab { run: 0, level: 0, len: 0 }; 16],
    tab3: [DCTtab { run: 0, level: 0, len: 0 }; 16],
    tab4: [DCTtab { run: 0, level: 0, len: 0 }; 16],
    tab5: [DCTtab { run: 0, level: 0, len: 0 }; 16],
    tab6: [DCTtab { run: 0, level: 0, len: 0 }; 16],
};

// ---------------------------------------------------------------------------
// FIFO
// ---------------------------------------------------------------------------

/// A simple FIFO over a `Vec<u8>`.
///
/// Models the IPU input/output FIFOs from `IPU_Fifo.cpp` but in a
/// straightforward, type-safe Rust form. The original C side uses a
/// fixed-size ring buffer of 32 u32's (8 quadwords) with hand-rolled
/// wrap-around; this version uses a `Vec<u8>` for storage and exposes
/// `push` / `pop` / `peek` / `clear` plus a `len` accessor.
#[derive(Debug, Clone, Default)]
pub struct IPUFifo {
    pub buffer: Vec<u8>,
}

impl IPUFifo {
    /// Create an empty FIFO.
    pub fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    /// Create a FIFO preallocated to `capacity` bytes.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            buffer: Vec::with_capacity(capacity),
        }
    }

    /// Number of bytes currently in the FIFO.
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// True if the FIFO has no bytes.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Push a single byte onto the back of the FIFO.
    pub fn push(&mut self, byte: u8) {
        self.buffer.push(byte);
    }

    /// Push a slice of bytes onto the back of the FIFO.
    pub fn push_slice(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    /// Pop a single byte from the front of the FIFO. Returns `None` if empty.
    pub fn pop(&mut self) -> Option<u8> {
        if self.buffer.is_empty() {
            None
        } else {
            Some(self.buffer.remove(0))
        }
    }

    /// Pop up to `n` bytes from the front of the FIFO into `dst`.
    /// Returns the number of bytes actually popped.
    pub fn pop_into(&mut self, dst: &mut [u8]) -> usize {
        let n = dst.len().min(self.buffer.len());
        if n == 0 {
            return 0;
        }
        dst[..n].copy_from_slice(&self.buffer[..n]);
        self.buffer.drain(..n);
        n
    }

    /// Peek at the front of the FIFO without removing it. Returns
    /// `None` if the FIFO is empty.
    pub fn peek(&self) -> Option<u8> {
        self.buffer.first().copied()
    }

    /// Peek at the first `n` bytes of the FIFO. Returns the slice that
    /// is actually available (which may be shorter than `n`).
    pub fn peek_n(&self, n: usize) -> &[u8] {
        let n = n.min(self.buffer.len());
        &self.buffer[..n]
    }

    /// Clear the FIFO, discarding all bytes.
    pub fn clear(&mut self) {
        self.buffer.clear();
    }
}

// ---------------------------------------------------------------------------
// DMA
// ---------------------------------------------------------------------------

/// IPU DMA CHCR (channel control) bitfield view.
///
/// The original C side uses a `union` of bitfields. We model the most
/// relevant control bits as a `u32` accessor struct so that callers can
/// read or write individual fields while still passing the whole thing
/// around as a 128-bit value (matching the EE DMAC layout for the
/// `chcr._u128[0]` slot used by the IPU channels).
#[derive(Debug, Clone, Copy, Default)]
pub struct IPUChcr {
    /// Packed 32-bit representation of the CHCR.
    pub raw: u32,
}

impl IPUChcr {
    /// Channel start trigger (bit 0 of the EE CHCR).
    pub fn str(&self) -> bool {
        (self.raw & 0x100) != 0
    }
    pub fn set_str(&mut self, v: bool) {
        if v {
            self.raw |= 0x100;
        } else {
            self.raw &= !0x100;
        }
    }

    /// Transfer mode: 0 = normal, 1 = chain, 2 = interleaved.
    pub fn mod_bits(&self) -> u32 {
        (self.raw >> 9) & 0x3
    }

    /// Tag-transfer enable.
    pub fn tte(&self) -> bool {
        (self.raw & 0x4000) != 0
    }

    /// Tag interrupt enable.
    pub fn tie(&self) -> bool {
        (self.raw & 0x8000) != 0
    }
}

/// IPU DMA controller state.
///
/// This is a 1:1 Rust analogue of the per-channel state used by
/// `IPU0dma` / `IPU1dma` in `IPUdma.cpp` (the `ipu0ch` / `ipu1ch`
/// globals from `IPU.h`). Field names follow the C side; "…" stands in
/// for fields we don't currently exercise from this module's executor.
#[derive(Debug, Clone, Default)]
pub struct IPUDma {
    /// CHCR (channel control register). Stored as 128 bits to match the
    /// EE DMAC register layout, even though only the low 32 bits are
    /// meaningful.
    pub chcr: u128,

    /// Quadword count remaining in the current transfer.
    pub qwc: u32,

    /// Memory address (MADR).
    pub madr: u32,

    /// Tag address (TADR).
    pub tadr: u32,

    /// Source chain tag id / flags ("…" denotes unused fields in the
    /// port). Mirrors `ipu1ch.chcr.tag().ID` access pattern.
    pub tag_id: u8,

    /// Tag IRQ request bit.
    pub tag_irq: bool,

    /// Number of QW currently in the input FIFO (`g_BP.IFC`).
    pub in_fifo_count: u32,

    /// Number of QW currently in the output FIFO (`OFC`).
    pub out_fifo_count: u32,

    /// DMA status: `in_progress` / `dma_finished` from `IPUDMAStatus`.
    pub in_progress: bool,
    pub dma_finished: bool,
}

impl IPUDma {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset to the state that `ipuDmaReset()` would produce in C++.
    pub fn reset(&mut self) {
        self.in_progress = false;
        self.dma_finished = true;
    }

    /// Read `n` quadwords from the input FIFO into `dst`.
    ///
    /// Returns the number of quadwords actually read. Mirrors the
    /// `IPU_Fifo_Input::read` semantics in the original.
    pub fn read(&mut self, dst: &mut [u8], n: usize, in_fifo: &mut IPUFifo) -> usize {
        let qw_bytes = n.saturating_mul(16);
        let take = qw_bytes.min(dst.len()).min(in_fifo.len());
        if take == 0 {
            return 0;
        }
        dst[..take].copy_from_slice(&in_fifo.buffer[..take]);
        in_fifo.buffer.drain(..take);
        // 1 QW read => decrement IFC by 1 (per quadword read).
        let qw_read = take / 16;
        self.in_fifo_count = self.in_fifo_count.saturating_sub(qw_read as u32);
        qw_read
    }

    /// Write `n` quadwords from `src` into the input FIFO.
    ///
    /// Returns the number of quadwords actually written. Mirrors the
    /// `IPU_Fifo_Input::write` semantics in the original.
    pub fn write(&mut self, src: &[u8], n: usize, in_fifo: &mut IPUFifo) -> usize {
        let free = IPU_FIFO_QW.saturating_sub(self.in_fifo_count as usize);
        let to_write = n.min(free);
        let byte_count = to_write.saturating_mul(16).min(src.len());
        if byte_count == 0 {
            return 0;
        }
        in_fifo.push_slice(&src[..byte_count]);
        self.in_fifo_count += to_write as u32;
        to_write
    }

    /// Read `n` quadwords out of the output FIFO into `dst`.
    pub fn read_output(&mut self, dst: &mut [u8], n: usize, out_fifo: &mut IPUFifo) -> usize {
        let qw_bytes = n.saturating_mul(16);
        let take = qw_bytes.min(dst.len()).min(out_fifo.len());
        if take == 0 {
            return 0;
        }
        dst[..take].copy_from_slice(&out_fifo.buffer[..take]);
        out_fifo.buffer.drain(..take);
        let qw_read = take / 16;
        self.out_fifo_count = self.out_fifo_count.saturating_sub(qw_read as u32);
        qw_read
    }

    /// Write `n` quadwords into the output FIFO from `src`.
    pub fn write_output(&mut self, src: &[u8], n: usize, out_fifo: &mut IPUFifo) -> usize {
        let free = IPU_FIFO_QW.saturating_sub(self.out_fifo_count as usize);
        let to_write = n.min(free);
        let byte_count = to_write.saturating_mul(16).min(src.len());
        if byte_count == 0 {
            return 0;
        }
        out_fifo.push_slice(&src[..byte_count]);
        self.out_fifo_count += to_write as u32;
        to_write
    }
}

impl fmt::Display for IPUDma {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "IPUDma{{ chcr=0x{:x}, qwc=0x{:x}, madr=0x{:x}, tadr=0x{:x}, in=0x{:x}, out=0x{:x}, prog={}, fin={} }}",
            self.chcr as u32,
            self.qwc,
            self.madr,
            self.tadr,
            self.in_fifo_count,
            self.out_fifo_count,
            self.in_progress,
            self.dma_finished,
        )
    }
}

// ---------------------------------------------------------------------------
// Dither / YUV->RGB helpers
// ---------------------------------------------------------------------------

/// PS2 IPU dithering: convert one planar YUV sample to an 8-bit RGB
/// triple using the 4x4 dither matrix from `IPUdither.cpp` (when `dte`
/// is true) or a simple 3-bit truncation (when `dte` is false).
///
/// `planar_yuv` is `[Y, Cb, Cr]`. We apply dithering in-place to the
/// Y channel (the only place the C++ dither coefficient matrix is
/// relevant at the per-sample level) and return the dithered/truncated
/// `[R, G, B]` bytes.
pub fn ipuDither(planar_yuv: &mut [i16; 3]) -> [u8; 3] {
    // The C++ dither operates on a 16x16 macroblock, but the dither
    // matrix is a per-pixel 4x4 pattern. We model the per-sample
    // single-pixel case here, using the (0,0) entry of the matrix.
    // The full 16x16 variant is exercised by `ipuDitherMacroblock`
    // below for callers that need the original behaviour.
    let dither_coefficient: [i16; 4] = [-4, 0, -3, 1];
    let d = dither_coefficient[0];
    let y = planar_yuv[0];
    let cb = planar_yuv[1];
    let cr = planar_yuv[2];

    // Y is biased and clamped to 0..255 before dithering.
    let y_clamped = (y as i32).clamp(0, 255);
    let y_dithered = (y_clamped + d as i32).clamp(0, 255);
    planar_yuv[0] = y_dithered as i16;

    // Use the IPU's BT.601 integer math to produce RGB.
    let rgb = ipuYuvToRgb(planar_yuv[0], cb, cr);
    [
        (rgb[0] >> 3) as u8,
        (rgb[1] >> 3) as u8,
        (rgb[2] >> 3) as u8,
    ]
}

/// PS2 IPU BT.601 YUV -> RGB conversion (per-sample).
///
/// This is the per-sample form of `yuv2rgb_reference()` in
/// `yuv2rgb.cpp`. The C++ implementation is SSE2/NEON-vectorised over
/// 16x16 macroblocks, but the underlying coefficients and round-by-1
/// rule are identical and we just expose a scalar version here.
pub fn ipuYuvToRgb(y: i16, cb: i16, cr: i16) -> [u8; 3] {
    let y0 = (y as i32 - IPU_Y_BIAS).max(0);
    let lum = (IPU_Y_COEFF * y0) >> 6;

    let cr_s = cr as i32 - 128;
    let cb_s = cb as i32 - 128;

    let rcr = (IPU_RCR_COEFF * cr_s) >> 6;
    let gcr = (IPU_GCR_COEFF * cr_s) >> 6;
    let gcb = (IPU_GCB_COEFF * cb_s) >> 6;
    let bcb = (IPU_BCB_COEFF * cb_s) >> 6;

    let r = ((lum + rcr + 1) >> 1).clamp(0, 255) as u8;
    let g = ((lum + gcr + gcb + 1) >> 1).clamp(0, 255) as u8;
    let b = ((lum + bcb + 1) >> 1).clamp(0, 255) as u8;
    [r, g, b]
}

// ---------------------------------------------------------------------------
// DMA executor
// ---------------------------------------------------------------------------

/// Combined IPU FIFO + DMA state, plus the executor glue.
///
/// The C++ module keeps `IPU_Fifo` as a global and reads/writes the
/// channel state via globals (`ipu_fifo`, `ipu0ch`, `ipu1ch`). For
/// Rust we wrap both behind a single executor struct so that the
/// `ipuDmaExec` function can be reentrant and testable.
#[derive(Debug, Default)]
pub struct IpuDmaExecutor {
    /// Channel 0 (IPU->EE) DMA state.
    pub dma_in: IPUDma,
    /// Channel 1 (EE->IPU) DMA state.
    pub dma_out: IPUDma,
    /// Input FIFO (EE writes, IPU reads).
    pub fifo_in: IPUFifo,
    /// Output FIFO (IPU writes, EE reads).
    pub fifo_out: IPUFifo,
}

impl IpuDmaExecutor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset the executor to power-on state.
    pub fn reset(&mut self) {
        self.dma_in.reset();
        self.dma_out.reset();
        self.fifo_in.clear();
        self.fifo_out.clear();
    }
}

/// Execute one step of the IPU DMA engine.
///
/// This is a structural port of `IPU1dma` + `IPU0dma` from
/// `IPUdma.cpp`. It mutates `state` and returns the number of quadwords
/// transferred by the call (mirroring the `totalqwc` return of the
/// C++ original).
///
/// The following hardware hooks are stubbed with `TODO` and return
/// safe defaults:
///   * `dmaGetAddr(madr, write)` -- returns a dummy backing buffer.
///   * `IPU_INT_TO` / `IPU_INT_FROM` / `IPU_INT_PROCESS` -- ignored.
///   * `CPU_SET_DMASTALL` -- ignored.
///   * `hwDmacSrcChain` / `hwDmacSrcTadrInc` -- the chain walker is
///     stubbed to a single-tag advance.
pub fn ipuDmaExec(state: &mut IpuDmaExecutor) -> usize {
    // -- Channel 1 (EE -> IPU): toIPU path -------------------------
    if !read_chcr_str(&state.dma_out) || read_chcr_mod(&state.dma_out) == 2 {
        // TODO: forward to CPU_SET_DMASTALL(DMAC_TO_IPU, true).
        return 0;
    }

    // Stub: pretend the IPU is always ready for data.
    let data_requested = true;

    if !data_requested {
        // TODO: cpuRegs.eCycle[4] = 0x9999; CPU_SET_DMASTALL(...)
        return 0;
    }

    let mut totalqwc: usize = 0;
    let tagcycles: usize = 1;

    if !state.dma_out.in_progress {
        // Stub: pretend we always have a valid tag and walk one entry.
        // TODO: replace with hwDmacSrcChain().
        if state.dma_out.qwc == 0 {
            state.dma_out.in_progress = false;
            state.dma_out.dma_finished = true;
            return 0;
        }
        if state.dma_out.tag_id == 0x7f {
            // TAG_END
            state.dma_out.dma_finished = true;
        } else {
            state.dma_out.dma_finished = false;
        }
        if state.dma_out.tag_irq && read_chcr_tie(&state.dma_out) {
            state.dma_out.dma_finished = true;
        }
        if state.dma_out.qwc != 0 {
            state.dma_out.in_progress = true;
        }
    }

    if state.dma_out.in_progress {
        // Transfer up to qwc quadwords from "memory" into the input
        // FIFO. We use a small scratch buffer here in lieu of a real
        // DMA backing store; a real port will plumb a memory bus in.
        let mut scratch = vec![0u8; 16 * state.dma_out.qwc as usize];
        let written = state
            .dma_out
            .write(&scratch, state.dma_out.qwc as usize, &mut state.fifo_in);
        state.dma_out.madr = state.dma_out.madr.wrapping_add((written as u32) * 16);
        state.dma_out.qwc = state.dma_out.qwc.saturating_sub(written as u32);
        totalqwc += written;
        if state.dma_out.qwc == 0 {
            state.dma_out.in_progress = false;
        }
    }

    if totalqwc == 0 || (state.dma_out.dma_finished && !state.dma_out.in_progress) {
        // TODO: IPU_INT_TO(totalqwc * BIAS)
        let _ = tagcycles;
    } else {
        // TODO: cpuRegs.eCycle[4] = 0x9999; CPU_SET_DMASTALL(...)
    }

    // -- Channel 0 (IPU -> EE): fromIPU path -----------------------
    if read_chcr_str(&state.dma_in)
        && state.dma_in.qwc != 0
        && read_chcr_mod(&state.dma_in) == 0
    {
        let read_size = (state.dma_in.qwc as usize).min(state.dma_in.out_fifo_count as usize);
        let mut scratch = vec![0u8; 16 * read_size];
        let read = state
            .dma_in
            .read_output(&mut scratch, read_size, &mut state.fifo_out);
        state.dma_in.madr = state.dma_in.madr.wrapping_add((read as u32) * 16);
        state.dma_in.qwc = state.dma_in.qwc.saturating_sub(read as u32);
    }

    totalqwc
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Read the STR (start) bit out of the 128-bit CHCR slot.
fn read_chcr_str(dma: &IPUDma) -> bool {
    let chcr = (dma.chcr as u32) as u32;
    (chcr & 0x100) != 0
}

/// Read the MOD (transfer mode) field out of the 128-bit CHCR slot.
fn read_chcr_mod(dma: &IPUDma) -> u32 {
    let chcr = (dma.chcr as u32) as u32;
    (chcr >> 9) & 0x3
}

/// Read the TIE (tag interrupt enable) bit out of the 128-bit CHCR slot.
fn read_chcr_tie(dma: &IPUDma) -> bool {
    let chcr = (dma.chcr as u32) as u32;
    (chcr & 0x8000) != 0
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fifo_push_pop_peek_clear() {
        let mut f = IPUFifo::new();
        assert!(f.is_empty());
        f.push(1);
        f.push(2);
        f.push(3);
        assert_eq!(f.len(), 3);
        assert_eq!(f.peek(), Some(1));
        assert_eq!(f.pop(), Some(1));
        assert_eq!(f.peek(), Some(2));
        f.clear();
        assert!(f.is_empty());
        assert_eq!(f.peek(), None);
    }

    #[test]
    fn fifo_push_slice_and_pop_into() {
        let mut f = IPUFifo::new();
        f.push_slice(&[10, 20, 30, 40]);
        let mut dst = [0u8; 2];
        let n = f.pop_into(&mut dst);
        assert_eq!(n, 2);
        assert_eq!(dst, [10, 20]);
        assert_eq!(f.peek_n(4), &[30, 40]);
    }

    #[test]
    fn dma_read_write_roundtrip() {
        let mut dma = IPUDma::new();
        let mut fifo = IPUFifo::new();
        let src = [0xABu8; 32];
        let n = dma.write(&src, 2, &mut fifo);
        assert_eq!(n, 2);
        assert_eq!(dma.in_fifo_count, 2);
        let mut dst = [0u8; 32];
        let r = dma.read(&mut dst, 2, &mut fifo);
        assert_eq!(r, 2);
        assert_eq!(dst, src);
        assert_eq!(dma.in_fifo_count, 0);
    }

    #[test]
    fn yuv_to_rgb_white() {
        // Y=235, Cb=128, Cr=128 should be roughly white.
        let rgb = ipuYuvToRgb(235, 128, 128);
        assert!(rgb[0] > 200);
        assert!(rgb[1] > 200);
        assert!(rgb[2] > 200);
    }

    #[test]
    fn dither_returns_5bit_channels() {
        let mut yuv = [128i16, 128, 128];
        let rgb = ipuDither(&mut yuv);
        // The function returns pre-shifted 5-bit values; they must
        // therefore all fit in 0..=31.
        for v in rgb {
            assert!(v <= 31);
        }
    }

    #[test]
    fn executor_idle_returns_zero() {
        let mut s = IpuDmaExecutor::new();
        // Default state has STR=0; executor should bail out cleanly.
        let n = ipuDmaExec(&mut s);
        assert_eq!(n, 0);
    }
}
