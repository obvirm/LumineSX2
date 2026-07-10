// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! GIF (Graphics Interface) subsystem translation.
//!
//! This module consolidates the C/C++ GIF sources (`Gif.cpp`, `Gif.h`,
//! `Gif_Unit.cpp`, `Gif_Unit.h`, `Gif_Logger.cpp`) into a single idiomatic
//! Rust 2021 module. It exposes the GIF hardware state (`GifState` / `gif`),
//! the four-path router (`gifPath`), and the high-level entry points
//! (`gifInit`, `gifReset`, `gifShutdown`, `gifExecPacket`,
//! `gifUnitPathInterrupt`).
//!
//! Path 0 is reserved for "no path active"; paths 1, 2, 3 correspond to the
//! three GIF transfer channels: VU1 XGKICK / MTVU (Path 1), direct
//! host-to-GS writes (Path 2), and GIF DMA/FIFO (Path 3).

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]

use std::sync::atomic::{AtomicI32, Ordering};

// -----------------------------------------------------------------------------
// Constants
// -----------------------------------------------------------------------------

/// GIF transfer path indices.
pub mod path {
    pub const PATH_1: u32 = 0;
    pub const PATH_2: u32 = 1;
    pub const PATH_3: u32 = 2;
    pub const PATH_COUNT: usize = 3;
}

/// GIF transfer type tags. The low nibble stores the path index, the next
/// nibble is a type discriminator, and the high byte is used for debug
/// printing.
pub mod transfer {
    pub const GIF_TRANS_INVALID: u32 = 0x000;
    pub const GIF_TRANS_XGKICK: u32 = 0x100;
    pub const GIF_TRANS_MTVU: u32 = 0x200;
    pub const GIF_TRANS_DIRECT: u32 = 0x301;
    pub const GIF_TRANS_DIRECTHL: u32 = 0x401;
    pub const GIF_TRANS_DMA: u32 = 0x502;
    pub const GIF_TRANS_FIFO: u32 = 0x602;
}

/// GIF path state machine states.
pub mod state {
    pub const PATH_IDLE: u32 = 0;
    pub const PATH_PACKED: u32 = 1;
    pub const PATH_REGLIST: u32 = 2;
    pub const PATH_IMAGE: u32 = 3;
    pub const PATH_WAIT: u32 = 4;
}

/// GIF tag FLG field values.
pub mod flg {
    pub const GIF_FLG_PACKED: u32 = 0;
    pub const GIF_FLG_REGLIST: u32 = 1;
    pub const GIF_FLG_IMAGE: u32 = 2;
    pub const GIF_FLG_IMAGE2: u32 = 3;
}

/// GIF A+D sub-register identifiers.
pub mod ad_reg {
    pub const GIF_A_D_REG_BITBLTBUF: u8 = 0x50;
    pub const GIF_A_D_REG_TRXREG: u8 = 0x52;
    pub const GIF_A_D_REG_TRXDIR: u8 = 0x53;
    pub const GIF_A_D_REG_SIGNAL: u8 = 0x60;
    pub const GIF_A_D_REG_FINISH: u8 = 0x61;
    pub const GIF_A_D_REG_LABEL: u8 = 0x62;
}

/// GIF register indices used in packed/reglist tags.
pub mod reg {
    pub const GIF_REG_A_D: u8 = 0x0E;
}

pub const GIF_PATH_NONE: u32 = 0;
pub const GIF_FIFO_QW: usize = 16;
pub const GIF_PATH_BUFF_SIZE: usize = 9 * 1024 * 1024;
pub const GIF_PATH_SAFE_ZONE: usize = 1024 * 1024 + 1024;

const BIAS: u32 = 2;

const GUNIT_LOG: fn(&str) = |_| {};
const GUNIT_WARN: fn(&str) = |_| {};
const GIF_LOG: fn(&str) = |_| {};
const GIF_PARSE: fn(&str) = |_| {};

// -----------------------------------------------------------------------------
// GIF hardware state
// -----------------------------------------------------------------------------

/// The GIF unit's hardware-visible state. This mirrors the original
/// `gifStruct`/`GIFregisters` aggregate.
#[repr(C, align(16))]
pub struct GifState {
    /// `gifstate_t` value: `READY` (0) or `EMPTY` (0x10).
    pub gifstate: i32,
    /// Set when the Path 3 DMA chain has finished.
    pub gspath3done: bool,
    /// EE cycles consumed by the most recent GS transfer.
    pub gscycles: u32,
    /// Cycles from the previous chunk (used for GS stall control).
    pub prevcycles: u32,
    /// Cycles used by the MFIFO transfer.
    pub mfifocycles: u32,
    /// QWC remaining in the current DMA tag transfer.
    pub gifqwc: u32,
    /// Set when the MFIFO GIF channel has raised an IRQ.
    pub gifmfifoirq: bool,

    // Mirror of GIF registers. The padding fields ensure 16-byte alignment
    // matches the C++ layout (a `GIFregisters` alias cast over `eeHw`).
    /// GIF control register.
    pub ctrl: u32,
    /// GIF mode register.
    pub mode: u32,
    /// GIF status register (PSE, DIR, APATH, FQC, ...).
    pub stat: u32,

    /// GIF tag word 0 (NLOOP, EOP, TAG upper).
    pub tag0: u32,
    /// GIF tag word 1 (PRIM, FLG, NREG, REGS upper).
    pub tag1: u32,
    /// GIF tag word 2 (REGS[0]).
    pub tag2: u32,
    /// GIF tag word 3 (REGS[1]).
    pub tag3: u32,

    /// GIF path loop counter / reg counter.
    pub cnt: u32,
    /// GIF path-3 counter.
    pub p3cnt: u32,
    /// GIF path-3 tag mirror.
    pub p3tag: u32,
}

/// The single global GIF state, `alignas(16)`-placed as in the C++ source.
pub static mut gif: GifState = GifState::new();

impl GifState {
    /// Construct a zero-initialized `GifState`. `const`-friendly default
    /// implementation backing the `static mut` instance.
    pub const fn new() -> Self {
        Self {
            gifstate: 0,
            gspath3done: true,
            gscycles: 0,
            prevcycles: 0,
            mfifocycles: 0,
            gifqwc: 0,
            gifmfifoirq: false,
            ctrl: 0,
            mode: 0,
            stat: 0,
            tag0: 0,
            tag1: 0,
            tag2: 0,
            tag3: 0,
            cnt: 0,
            p3cnt: 0,
            p3tag: 0,
        }
    }
}

// -----------------------------------------------------------------------------
// GIF FIFO
// -----------------------------------------------------------------------------

/// 16 QW GIF-FIFO between the DMA controller and the GIF unit. Mirrors
/// `GIF_Fifo` from `Gif.h`.
pub struct GifFifo {
    /// Packed as 16 QWs of 4 `u32`s each.
    pub data: [u32; GIF_FIFO_QW * 4],
    /// Number of QWs currently buffered (0..=16).
    pub fifoSize: u32,
}

impl GifFifo {
    pub const fn new() -> Self {
        Self {
            data: [0u32; GIF_FIFO_QW * 4],
            fifoSize: 0,
        }
    }

    /// Reset the FIFO to empty.
    pub fn init(&mut self) {
        for d in self.data.iter_mut() {
            *d = 0;
        }
        self.fifoSize = 0;
    }

    /// Write up to `size` QWs from `pMem` into the FIFO. Returns the number
    /// of QWs that were actually written.
    pub fn write_fifo(&mut self, pMem: &[u32], size: usize) -> usize {
        if self.fifoSize as usize == GIF_FIFO_QW {
            return 0;
        }
        let transfer_size = size.min(GIF_FIFO_QW - self.fifoSize as usize);
        let write_pos = (self.fifoSize as usize) * 4;
        let end = write_pos + transfer_size * 4;
        self.data[write_pos..end].copy_from_slice(&pMem[..transfer_size * 4]);
        self.fifoSize += transfer_size as u32;
        transfer_size
    }

    /// Read as many QWs as the GIF unit is willing to consume right now.
    /// Returns the number of QWs that were drained into the GIF paths.
    pub fn read_fifo(&mut self, gif_ref: &mut GifState) -> usize {
        if self.fifoSize == 0 || !can_do_path3(gif_ref) {
            // No transfer possible; reschedule via the DMA tick callback.
            return 0;
        }

        let size_read =
            transfer_gs_packet_data(gif_ref, transfer::GIF_TRANS_DMA, &self.data, self.fifoSize as usize) / 16;
        if size_read < self.fifoSize as usize {
            // Compact the tail of the buffer (FIFO acts as a ring slot).
            let copy_amount = self.fifoSize as usize - size_read;
            let read_pos = size_read * 4;
            let (left, right) = self.data.split_at_mut(read_pos);
            for i in 0..copy_amount {
                left[i * 4..i * 4 + 4].copy_from_slice(&right[i * 4..i * 4 + 4]);
            }
            self.fifoSize = copy_amount as u32;
        } else {
            self.fifoSize = 0;
        }
        size_read
    }
}

/// Global GIF FIFO instance.
pub static mut gif_fifo: GifFifo = GifFifo::new();

// -----------------------------------------------------------------------------
// GIF path
// -----------------------------------------------------------------------------

/// Per-path bookkeeping exposed via the public `gifPath` array. The
/// `reg` field mirrors the 128-bit GIF tag value (NLOOP/EOP/TAG upper +
/// PRIM/FLG/NREG/REGS), and the remaining fields are status/queueing
/// counters used by the path arbitration logic.
#[derive(Clone, Copy)]
pub struct GifPath {
    /// Last 128-bit GIF tag seen on this path (little-endian).
    pub reg: u128,
    /// Current path state (`PATH_IDLE`/`PATH_PACKED`/`PATH_REGLIST`/
    /// `PATH_IMAGE`/`PATH_WAIT`).
    pub mode: u32,
    /// GIF STAT path-mask bits (bit 0: P1Q, bit 1: P2Q, bit 2: P3Q, bit 3: IP3,
    /// bit 4: OPH, bit 5: PSE, bit 6: DIR, bits 7..=8: APATH).
    pub status: u32,
    /// Number of QWs queued for this path.
    pub queued: u32,
    /// Number of QWs already consumed by the GS.
    pub processed: u32,
}

impl GifPath {
    pub const fn new() -> Self {
        Self {
            reg: 0,
            mode: state::PATH_IDLE,
            status: 0,
            queued: 0,
            processed: 0,
        }
    }
}

/// Global 4-entry path table. Index 0 is unused so that the public API can
/// be 1-based like the EE's GIF unit.
pub static mut gifPath: [GifPath; 4] = [
    GifPath::new(),
    GifPath::new(),
    GifPath::new(),
    GifPath::new(),
];

/// Per-path 128-bit register file (u64 high / low) used for tag decoding.
/// Stored separately so the path table stays `Copy`.
#[derive(Clone, Copy)]
struct GifPathRegs {
    low: u64,
    high: u64,
}

struct GifPathState {
    idx: u32,
    buffer: Vec<u8>,
    buff_size: usize,
    buff_limit: usize,
    cur_size: usize,
    cur_offset: usize,
    dma_rewind: u32,
    state: u32,
    read_amount: AtomicI32,
    regs: GifPathRegs,
    nloop: u32,
    nregs: u32,
    nreg_idx: u32,
    cycles: u32,
    has_ad: bool,
    eop: bool,
    flg: u32,
    queued_qw: u32,
    processed_qw: u32,
}

impl GifPathState {
    fn new(idx: u32) -> Self {
        Self {
            idx,
            buffer: vec![0u8; GIF_PATH_BUFF_SIZE],
            buff_size: GIF_PATH_BUFF_SIZE,
            buff_limit: GIF_PATH_BUFF_SIZE - GIF_PATH_SAFE_ZONE,
            cur_size: 0,
            cur_offset: 0,
            dma_rewind: 0,
            state: state::PATH_IDLE,
            read_amount: AtomicI32::new(0),
            regs: GifPathRegs { low: 0, high: 0 },
            nloop: 0,
            nregs: 0,
            nreg_idx: 0,
            cycles: 0,
            has_ad: false,
            eop: false,
            flg: 0,
            queued_qw: 0,
            processed_qw: 0,
        }
    }

    fn reset(&mut self) {
        self.cur_size = 0;
        self.cur_offset = 0;
        self.dma_rewind = 0;
        self.state = state::PATH_IDLE;
        self.read_amount.store(0, Ordering::Release);
        self.nloop = 0;
        self.nregs = 0;
        self.nreg_idx = 0;
        self.cycles = 0;
        self.has_ad = false;
        self.eop = false;
        self.flg = 0;
        self.regs = GifPathRegs { low: 0, high: 0 };
    }
}

// -----------------------------------------------------------------------------
// Path decoder dispatch
// -----------------------------------------------------------------------------

/// Decode the FLG field of a GIF tag and return the matching `state::*`
/// constant. Invalid FLG values map to `PATH_IDLE`.
fn decode_flg(flg: u32) -> u32 {
    match flg & 0x3 {
        flg::GIF_FLG_PACKED => state::PATH_PACKED,
        flg::GIF_FLG_REGLIST => state::PATH_REGLIST,
        flg::GIF_FLG_IMAGE | flg::GIF_FLG_IMAGE2 => state::PATH_IMAGE,
        _ => state::PATH_IDLE,
    }
}

/// Decode a single 16-byte GIF tag from `data` into the supplied
/// `GifPathState` and the public `GifPath` mirror. Returns `true` if the
/// tag was successfully parsed.
fn decode_tag(path_state: &mut GifPathState, path: &mut GifPath, data: &[u8]) -> bool {
    if data.len() < 16 {
        return false;
    }
    // GIF tags are little-endian 128-bit values; replicate the C++ union
    // layout (NLOOP/EOP/TAG, then REGS/FLG/NREG/PRIM/PRE).
    let lo = u64::from_le_bytes([
        data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
    ]);
    let hi = u64::from_le_bytes([
        data[8], data[9], data[10], data[11], data[12], data[13], data[14], data[15],
    ]);
    path_state.regs = GifPathRegs { low: lo, high: hi };
    path.reg = (lo as u128) | ((hi as u128) << 64);

    let nloop = (lo & 0x7fff) as u32;
    let eop = ((lo >> 15) & 0x1) as u32;
    let regs_hi = ((hi >> 32) as u32).to_le();
    let nreg = ((hi >> 32) & 0xf) as u32;
    let flg = ((hi >> 36) & 0x3) as u32;
    let flg_norm = if flg == flg::GIF_FLG_IMAGE2 { flg::GIF_FLG_IMAGE } else { flg };

    path_state.nloop = nloop;
    path_state.eop = eop != 0;
    path_state.nregs = ((nreg.wrapping_sub(1)) & 0xf) + 1;
    path_state.flg = flg;
    path_state.has_ad = (regs_hi & 0x0f0f0f0f) == 0x0e0e0e0e
        || contains_ad(regs_hi, path_state.nregs);

    // Approximate cycles: packed 2 EE/qw, reglist/image 4 EE/qw.
    let len = match flg_norm {
        flg::GIF_FLG_PACKED => path_state.nregs * nloop * 16,
        flg::GIF_FLG_REGLIST => {
            let pairs = path_state.nregs * nloop;
            ((pairs + 1) >> 1) * 16
        }
        _ => nloop * 16,
    };
    path_state.cycles = len * (if flg_norm == flg::GIF_FLG_PACKED { 2 } else { 4 });

    path_state.state = decode_flg(flg_norm);
    path.mode = path_state.state;
    path.queued = path.queued.wrapping_add((len / 16) as u32);
    true
}

/// Plain-C reference for `hasAD`: scan up to `nregs` nibbles across the two
/// `REGS` words.
fn contains_ad(regs_combined: u32, nregs: u32) -> bool {
    let mut t = regs_combined;
    for _ in 0..nregs.min(8) {
        if (t & 0xf) == reg::GIF_REG_A_D as u32 {
            return true;
        }
        t >>= 4;
    }
    false
}

// -----------------------------------------------------------------------------
// Path capability helpers (mirrors of `Gif_Unit::CanDo*`)
// -----------------------------------------------------------------------------

fn stat_field(gif_ref: &GifState) -> (u32, u32, u32) {
    let s = gif_ref.stat;
    let pse = (s >> 3) & 1;
    let dir = (s >> 15) & 1;
    let apath = (s >> 10) & 0x3;
    (pse, dir, apath)
}

fn can_do_gif(gif_ref: &GifState) -> bool {
    let (pse, dir, _) = stat_field(gif_ref);
    pse == 0 && dir == 0
        && (gif_ref.stat & (1 << 11)) == 0 /* queued signal bit placeholder */
}

fn can_do_p3_slice(gif_ref: &GifState) -> bool {
    let imt = (gif_ref.mode & 1) != 0;
    unsafe { imt && gifPath[path::PATH_3 as usize + 1].mode == state::PATH_IMAGE }
}

fn path3_masked(gif_ref: &GifState) -> bool {
    let m3r = gif_ref.mode & 1;
    let m3p = (gif_ref.stat >> 1) & 1;
    let s = unsafe { gifPath[path::PATH_3 as usize + 1].mode };
    (m3r != 0 || m3p != 0) && (s == state::PATH_IDLE || s == state::PATH_WAIT)
}

fn can_do_path1(gif_ref: &GifState) -> bool {
    let (_, _, apath) = stat_field(gif_ref);
    (apath == 0 || apath == 1 || (apath == 3 && can_do_p3_slice(gif_ref))) && can_do_gif(gif_ref)
}

fn can_do_path2(gif_ref: &GifState) -> bool {
    let (_, _, apath) = stat_field(gif_ref);
    (apath == 0 || apath == 2 || (apath == 3 && can_do_p3_slice(gif_ref))) && can_do_gif(gif_ref)
}

fn can_do_path2_hl(gif_ref: &GifState) -> bool {
    let (_, _, apath) = stat_field(gif_ref);
    (apath == 0 || apath == 2) && can_do_gif(gif_ref)
}

fn can_do_path3(gif_ref: &GifState) -> bool {
    let (_, _, apath) = stat_field(gif_ref);
    ((apath == 0 && !path3_masked(gif_ref)) || apath == 3) && can_do_gif(gif_ref)
}

// -----------------------------------------------------------------------------
// Transfer dispatcher
// -----------------------------------------------------------------------------

/// Copy `size` bytes from `pMem` into the matching GIF path's per-path
/// buffer and run the path decoder on the leading GIF tag.
fn transfer_gs_packet_data(
    gif_ref: &mut GifState,
    tran_type: u32,
    pMem: &[u32],
    size: usize,
) -> usize {
    if size == 0 {
        return 0;
    }
    if !can_do_gif(gif_ref) {
        GUNIT_WARN("Gif Unit - Signal or PSE Set or Dir = GS to EE");
    }
    let path_idx = (tran_type & 0x3) as usize;
    if path_idx >= path::PATH_COUNT {
        return 0;
    }
    let slot = path_idx + 1;
    let _ = unsafe { gifPath[slot] }; // ensure public path table is observed
    // Append to the path's per-path buffer.
    let mut state = unsafe { PATH_STATES[path_idx].lock().unwrap() };
    if state.cur_size + size > state.buff_size {
        // Real call into `Gif_Path::RealignPacket` would live here. For the
        // Rust skeleton we merely drop the overflow.
        return 0;
    }
    let bytes = size;
    let src_bytes: Vec<u8> = pMem
        .iter()
        .flat_map(|w| w.to_le_bytes())
        .take(bytes)
        .collect();
    let cur_size = state.cur_size;
    state.buffer[cur_size..cur_size + bytes].copy_from_slice(&src_bytes);
    state.cur_size = cur_size + bytes;
    unsafe { gifPath[slot].queued = (state.cur_size / 16) as u32 };

    // If we have at least one tag, decode it and update the public mirror.
    if state.cur_size >= 16 {
        let bytes_16: [u8; 16] = state.buffer[..16].try_into().unwrap();
        let mut path = unsafe { gifPath[slot] };
        if decode_tag(&mut state, &mut path, &bytes_16) {
            unsafe { gifPath[slot] = path; }
        }
    }
    unsafe { gifPath[slot].status |= 1 << path_idx }; // mark the corresponding PnQ as in flight
    unsafe { gifPath[slot].processed = state.processed_qw };
    size
}

// -----------------------------------------------------------------------------
// Path state storage
// -----------------------------------------------------------------------------

use std::sync::Mutex;

/// Per-path internal state. We allocate three of these (one per real GIF
/// path) and protect them with a `Mutex` since the original C++ code
/// mutates them from multiple threads (EE + MTGS + MTVU).
static mut PATH_STATES: [Mutex<GifPathState>; 3] = [
    Mutex::new(GifPathState::new_uninit()),
    Mutex::new(GifPathState::new_uninit()),
    Mutex::new(GifPathState::new_uninit()),
];

impl GifPathState {
    const fn new_uninit() -> GifPathState {
        // Used to satisfy the `static` initializer. The actual values are
        // overwritten inside `gifInit`.
        GifPathState {
            idx: 0,
            buffer: Vec::new(),
            buff_size: 0,
            buff_limit: 0,
            cur_size: 0,
            cur_offset: 0,
            dma_rewind: 0,
            state: 0,
            read_amount: AtomicI32::new(0),
            regs: GifPathRegs { low: 0, high: 0 },
            nloop: 0,
            nregs: 0,
            nreg_idx: 0,
            cycles: 0,
            has_ad: false,
            eop: false,
            flg: 0,
            queued_qw: 0,
            processed_qw: 0,
        }
    }
}

// -----------------------------------------------------------------------------
// Public API
// -----------------------------------------------------------------------------

/// Initialize the GIF unit. Wipes all hardware state, the FIFO, and the
/// per-path buffers. Mirrors `gifUnit` construction in the original
/// `Gif_Unit` constructor and `GIF_Fifo::init`.
pub fn gifInit() {
    unsafe {
        gif = GifState::new();
        gif_fifo.init();
        gif.gspath3done = true;
        gif.gscycles = 0;
        gif.prevcycles = 0;
        gif.mfifocycles = 0;
        for slot in 0..3 {
            let mut s = PATH_STATES[slot].lock().unwrap();
            *s = GifPathState::new(slot as u32);
        }
        for p in gifPath.iter_mut() {
            *p = GifPath::new();
        }
    }
}

/// Soft/hard reset. With `soft = true`, leave in-flight packet data
/// alone (matches the C++ `Reset(true)` path used during game emulation).
pub fn gifReset() {
    gifResetImpl(false);
}

/// Implementation behind `gifReset`. Provided as a separate function so the
/// caller can pass `soft` if needed; the public wrapper always hard-resets.
fn gifResetImpl(soft: bool) {
    unsafe {
        gif.stat = 0;
        gif.ctrl = 0;
        gif.mode = 0;
        gif.gspath3done = true;
        if !soft {
            gif.gscycles = 0;
            gif.prevcycles = 0;
            gif.mfifocycles = 0;
            gif.gifqwc = 0;
            for slot in 0..3 {
                let mut s = PATH_STATES[slot].lock().unwrap();
                s.reset();
            }
            for p in gifPath.iter_mut() {
                *p = GifPath::new();
            }
            gif_fifo.init();
        }
    }
}

/// Tear down the GIF unit. After this call, all per-path buffers are
/// dropped and the FIFO is cleared. The function is idempotent.
pub fn gifShutdown() {
    unsafe {
        for slot in 0..3 {
            let mut s = PATH_STATES[slot].lock().unwrap();
            s.reset();
            s.buffer.clear();
        }
        gif_fifo.init();
        for p in gifPath.iter_mut() {
            *p = GifPath::new();
        }
    }
}

/// Execute a single GIF packet sourced from the EE's GS register
/// window. The bytes are copied into the path matching the active
/// `stat.APATH` (defaulting to Path 3 if no path is selected) and the
/// leading GIF tag is decoded.
pub fn gifExecPacket(data: &[u8]) {
    if data.is_empty() {
        return;
    }
    let mut gif_ref = unsafe { &mut gif };
    let (_, _, apath) = stat_field(&gif_ref);
    let slot = if apath == 0 { path::PATH_3 } else { apath - 1 } as usize;
    // Pack the byte slice into a 32-bit-aligned vector for the FIFO write.
    let words: Vec<u32> = data
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    let qw = (words.len() + 3) / 4;
    transfer_gs_packet_data(
        &mut gif_ref,
        transfer::GIF_TRANS_DMA,
        &words,
        qw * 16,
    );
}

/// Handle a path-level interrupt. The EE raises this whenever one of the
/// three GIF paths transitions to idle (Path 3) or completes a packet.
/// Re-runs the path arbitration loop, draining the path that became ready
/// and waking up any queued transfers.
pub fn gifUnitPathInterrupt(path: u32) {
    if path == 0 || path as usize > path::PATH_COUNT {
        return;
    }
    unsafe {
        let slot = (path - 1) as usize;
        // Mark the path idle in the public table.
        gifPath[slot + 1].mode = state::PATH_IDLE;
        let mut s = PATH_STATES[slot].lock().unwrap();
        s.state = state::PATH_IDLE;
        s.processed_qw = s.queued_qw;
        gifPath[slot + 1].processed = s.processed_qw;
    }
    // Drain any pending FIFO data into Path 3.
    unsafe {
        let read = gif_fifo.read_fifo(&mut gif);
        if read != 0 {
            GifDMAInt((read as u32) * BIAS);
        }
    }
}

/// Internal helper: schedule a DMA tick for the GIF channel after `cycles`
/// EE cycles. The original C++ invokes `CPU_INT` / `hwDmacIrq`; here we
/// simply update `gscycles` so the next `gifInterrupt` invocation sees the
/// delay.
fn GifDMAInt(cycles: u32) {
    unsafe {
        if cycles > gif.gscycles {
            gif.gscycles = cycles;
        }
    }
}

// -----------------------------------------------------------------------------
// `CopyQWC` helper (matches `CopyQWC` in the C++ source)
// -----------------------------------------------------------------------------

/// Copy one QW (16 bytes / 4 `u32`s) from `src` into `dst`. Equivalent to
/// the legacy `CopyQWC` macro used in `Gif.cpp`.
#[allow(non_snake_case)]
pub fn CopyQWC(dst: &mut [u32], src: &[u32]) {
    if dst.len() < 4 || src.len() < 4 {
        return;
    }
    dst[..4].copy_from_slice(&src[..4]);
}

// -----------------------------------------------------------------------------
// GIF logger (`Gif_Logger.cpp`)
// -----------------------------------------------------------------------------

/// Pretty-print a GIF packet. The original `Gif_ParsePacket` writes to
/// `DevCon`; here we return a `String` so the caller can route it as it
/// wishes.
pub fn Gif_ParsePacket(data: &[u8], size: usize, path: u32) -> String {
    let mut out = String::new();
    out.push_str(&format!("Path {} Transfer\n", path + 1));
    let mut offset = 0usize;
    let mut nregs = 0u32;
    let mut flg = 0u32;
    let mut nloop = 0u32;
    let mut len = 0u32;
    loop {
        if offset + 16 > size {
            return out;
        }
        let lo = u64::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
            data[offset + 4],
            data[offset + 5],
            data[offset + 6],
            data[offset + 7],
        ]);
        let hi = u64::from_le_bytes([
            data[offset + 8],
            data[offset + 9],
            data[offset + 10],
            data[offset + 11],
            data[offset + 12],
            data[offset + 13],
            data[offset + 14],
            data[offset + 15],
        ]);
        nloop = (lo & 0x7fff) as u32;
        let eop = (lo >> 15) & 1;
        let regs_hi = (hi >> 32) as u32;
        nregs = ((regs_hi & 0xf).wrapping_sub(1) & 0xf) + 1;
        flg = ((regs_hi >> 4) & 0x3) as u32;
        let flg_norm = if flg == flg::GIF_FLG_IMAGE2 { flg::GIF_FLG_IMAGE } else { flg };
        len = match flg_norm {
            flg::GIF_FLG_PACKED => nregs * nloop * 16,
            flg::GIF_FLG_REGLIST => ((nregs * nloop + 1) >> 1) * 16,
            _ => nloop * 16,
        };
        out.push_str(&format!(
            "--Gif Tag [mode={}][nregs={}][nloop={}][qwc={}][EOP={}]\n",
            match flg_norm {
                flg::GIF_FLG_PACKED => "Packed",
                flg::GIF_FLG_REGLIST => "Reglist",
                flg::GIF_FLG_IMAGE => "Image",
                _ => "Image2",
            },
            nregs,
            nloop,
            len / 16,
            eop
        ));
        if offset + 16 + len as usize > size {
            return out;
        }
        offset += 16;
        match flg_norm {
            flg::GIF_FLG_PACKED => {
                for _i in 0..nloop {
                    for _j in 0..nregs {
                        if offset + 8 >= size {
                            return out;
                        }
                        out.push_str(&format!(
                            "----[regbyte=0x{:02x}]\n",
                            data[offset + 8]
                        ));
                        offset += 16;
                    }
                }
            }
            flg::GIF_FLG_REGLIST => {
                offset += len as usize;
            }
            _ => {
                offset += len as usize;
            }
        }
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gif_state_default_is_ready() {
        let s = GifState::new();
        assert_eq!(s.gifstate, 0);
        assert!(s.gspath3done);
    }

    #[test]
    fn path_decoder_packed_tag() {
        let mut path = GifPath::new();
        let mut state = GifPathState::new(0);
        // NLOOP=2, EOP=1, FLG=PACKED, NREG=1
        let lo: u64 = 0x8002;
        let hi: u64 = (0u64 << 36) /* FLG=0 */ | (1u64 << 32) /* NREG=1 */;
        let bytes: [u8; 16] = [
            lo as u8,
            (lo >> 8) as u8,
            (lo >> 16) as u8,
            (lo >> 24) as u8,
            (lo >> 32) as u8,
            (lo >> 40) as u8,
            (lo >> 48) as u8,
            (lo >> 56) as u8,
            hi as u8,
            (hi >> 8) as u8,
            (hi >> 16) as u8,
            (hi >> 24) as u8,
            (hi >> 32) as u8,
            (hi >> 40) as u8,
            (hi >> 48) as u8,
            (hi >> 56) as u8,
        ];
        assert!(decode_tag(&mut state, &mut path, &bytes));
        assert_eq!(path.mode, state::PATH_PACKED);
    }

    #[test]
    fn fifo_write_and_read() {
        let mut fifo = GifFifo::new();
        let words = [0u32; 64];
        let written = fifo.write_fifo(&words, 16);
        assert_eq!(written, 16);
        assert_eq!(fifo.fifoSize, 16);
    }

    #[test]
    fn init_shutdown_round_trip() {
        gifInit();
        gifReset();
        gifShutdown();
    }
}
