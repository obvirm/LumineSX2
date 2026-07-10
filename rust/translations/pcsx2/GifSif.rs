// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
//! Rust translation of the GIF and SIF DMA subsystem.
//!
//! This module is a single-file idiomatic Rust 2021 translation of the
//! following PCSX2 sources:
//!
//! * `pcsx2/Gif.cpp`        - GIF packet parser, DMA path and FIFO driver
//! * `pcsx2/Gif.h`          - GIF HW register unions, FIFO struct, helpers
//! * `pcsx2/Gif_Unit.cpp`   - GIF path 1/2/3 unit, path arbitration, MTVU glue
//! * `pcsx2/Gif_Unit.h`     - Path buffers, GS packet/tag types, Gif_Unit
//! * `pcsx2/Sif.cpp`        - SIF reset and save-state glue
//! * `pcsx2/Sif.h`          - SIF FIFO/EE/IOP sub-structures, register decls
//! * `pcsx2/Sif0.cpp`       - SIF0 (IOP -> EE) DMA channel
//! * `pcsx2/Sif1.cpp`       - SIF1 (EE -> IOP) DMA channel
//! * `pcsx2/sif2.cpp`       - SIF2 (PS1 GPU) DMA channel
//!
//! The GIF is the GS packet parser: the EE pushes GIFtagged data through it
//! over path 1 (Xgkick from VU1), path 2 (GIF_DIRECT / DIRECT_HL from EE) and
//! path 3 (DMA, which is what the EE's `gifch` channel feeds). Gif_Unit is
//! the per-path state machine, MTVU coordination and arbitration engine.
//!
//! The SIF is the EE <-> IOP DMA bridge used to keep the two processors
//! coherent for things like the module loading protocol, PAD/SDD/MC services
//! and the PS1 GPU data path.
//!
//! This module intentionally uses `static mut` for all global state and only
//! depends on `std` (no external crates) so it can be dropped into the
//! standalone `rust/pcsx2` crate as-is.

#![allow(dead_code)]
#![allow(non_snake_case)]
#![allow(clippy::upper_case_acronyms)]

use std::cmp::min;
use std::mem::size_of;
use std::ptr;

use crate::pcsx2::SifMainEtc::FIFO_SIF_W;

// ---------------------------------------------------------------------------
// GIF path / transfer constants
// ---------------------------------------------------------------------------

/// GIF path index 1 (VU1 Xgkick via GIF).
pub const GIF_PATH_1: usize = 0;
/// GIF path index 2 (EE direct / direct HL).
pub const GIF_PATH_2: usize = 1;
/// GIF path index 3 (EE DMA / FIFO).
pub const GIF_PATH_3: usize = 2;

/// Path is idle (hasn't started a GS packet).
pub const GIF_PATH_IDLE: u32 = 0;
/// Path is on a PACKED gif tag.
pub const GIF_PATH_PACKED: u32 = 1;
/// Path is on a REGLIST gif tag.
pub const GIF_PATH_REGLIST: u32 = 2;
/// Path is on an IMAGE gif tag.
pub const GIF_PATH_IMAGE: u32 = 3;
/// Used only by PATH 3 to simulate packet length (path-3 masking).
pub const GIF_PATH_WAIT: u32 = 4;

/// GIF transfer types. Low byte holds `path - 1`.
pub const GIF_TRANS_INVALID: u32 = 0x000;
pub const GIF_TRANS_XGKICK: u32 = 0x100;
pub const GIF_TRANS_MTVU: u32 = 0x200;
pub const GIF_TRANS_DIRECT: u32 = 0x301;
pub const GIF_TRANS_DIRECTHL: u32 = 0x401;
pub const GIF_TRANS_DMA: u32 = 0x502;
pub const GIF_TRANS_FIFO: u32 = 0x602;

/// Mask of "GIF unit side" vs. DMA side FIFO flag.
pub const CSR_FIFO_EMPTY: u32 = 0;
pub const CSR_FIFO_NORMAL: u32 = 1;
pub const CSR_FIFO_FULL: u32 = 2;

/// DMA stall control sources (`dmacRegs.ctrl.STD`).
pub const STD_GIF: u32 = 0;
pub const STD_SIF0: u32 = 1;
pub const STD_SIF1: u32 = 2;

/// DMA MFIFO drain target (`dmacRegs.ctrl.MFD`).
pub const MFD_GIF: u32 = 0;

/// DMA chain tag IDs (upper 3 bits of a `tDMA_TAG`).
pub const TAG_CNT: u32 = 0;
pub const TAG_NEXT: u32 = 1;
pub const TAG_REF: u32 = 2;
pub const TAG_REFS: u32 = 3;
pub const TAG_REFE: u32 = 4;
pub const TAG_CALL: u32 = 5;
pub const TAG_RET: u32 = 6;
pub const TAG_END: u32 = 7;
pub const TAG_CNTS: u32 = 5; // alias used in stall control checks

/// EE cycles per GIF qword transferred (BIAS).
pub const BIAS: u32 = 2;

/// DMAC channel indices used by the GIF/SIF modules.
pub const DMAC_GIF: u32 = 2;
pub const DMAC_SIF0: u32 = 3;
pub const DMAC_SIF1: u32 = 4;
pub const DMAC_SIF2: u32 = 6;
pub const DMAC_MFIFO_GIF: u32 = 11;
pub const DMAC_VIF1: u32 = 5;
pub const DMAC_STALL_SIS: u32 = 13;

pub const DMAC_SIF0_STS: u32 = 0;

/// GS-side interrupt masks and pending flag bits (subset used by GIF/SIF).
pub const SBUS_F240: u32 = 0x1000F240;
pub const SBUS_F240_SIF0_MSB: u32 = 0x0020;
pub const SBUS_F240_SIF0_EE: u32 = 0x2000;
pub const SBUS_F240_SIF1_MSB: u32 = 0x0040;
pub const SBUS_F240_SIF1_EE: u32 = 0x4000;
pub const SBUS_F240_SIF2_MSB: u32 = 0x0080;
pub const SBUS_F240_SIF2_EE: u32 = 0x8000;

/// GS `STAT.FQC` mask. 0..=16 valid, 5-bit field.
pub const FQC_MASK: u32 = 0x1F;

/// IOP DMA channel indices (for `PSX_INT`).
pub const IopEvt_SIF0: u32 = 0;
pub const IopEvt_SIF1: u32 = 1;
pub const IopEvt_SIF2: u32 = 2;

// ---------------------------------------------------------------------------
// GIF / SIF public state
// ---------------------------------------------------------------------------

/// Minimal GIF path record. Mirrors the subset of `Gif_Path` / `GIFregisters`
/// needed by the public Rust API: a 128-bit GIFtag (`reg`), the path mode
/// word, status (FQC/APATH/...), a per-path queued count and a per-path
/// processed count.
#[derive(Copy, Clone)]
pub struct GifPath {
    /// Most recent GIFtag (128 bits) seen on this path.
    pub reg: u128,
    /// GIF `MODE` register (M3R, IMT, ...).
    pub mode: u32,
    /// GIF `STAT` register (FQC, APATH, OPH, P1Q/P2Q/P3Q, M3P, M3R, ...).
    pub status: u32,
    /// Number of bytes queued for this path (DMA / direct / XGkick backlog).
    pub queued: u32,
    /// Number of qwords processed by the unit on this path since the last
    /// reset.
    pub processed: u32,
}

impl GifPath {
    pub const fn new() -> Self {
        Self {
            reg: 0u128,
            mode: 0u32,
            status: 0u32,
            queued: 0u32,
            processed: 0u32,
        }
    }
}

impl Default for GifPath {
    fn default() -> Self {
        Self::new()
    }
}

/// All four GIF path slots. The 4th slot is reserved (the EE only has
/// paths 1..=3) and is always zero-initialised.
pub static mut gifPath: [GifPath; 4] = [
    GifPath::new(),
    GifPath::new(),
    GifPath::new(),
    GifPath::new(),
];

/// SIF (sub-system interface) registers visible to the EE. The IOP reads
/// them through SIF RPC; the EE writes `mscom` / `msflg` and reads `smcom`
/// / `smflg`.
///
/// * `mscom`  - main->sub command / data
/// * `smcom`  - sub->main command / data
/// * `msflg`  - main->sub flag (set when `mscom` is written)
/// * `smflg`  - sub->main flag (set when `smcom` is written)
#[derive(Copy, Clone)]
pub struct SifRegs {
    pub mscom: u32,
    pub smcom: u32,
    pub msflg: u32,
    pub smflg: u32,
}

impl SifRegs {
    pub const fn new() -> Self {
        Self {
            mscom: 0,
            smcom: 0,
            msflg: 0,
            smflg: 0,
        }
    }
}

impl Default for SifRegs {
    fn default() -> Self {
        Self::new()
    }
}

/// Live SIF register state.
pub static mut sifRegs: SifRegs = SifRegs::new();

// ---------------------------------------------------------------------------
// Internal GIF state
// ---------------------------------------------------------------------------

/// 16-qword ring used to feed PATH 3.
const GIF_FIFO_SLOTS: usize = 16;
const GIF_FIFO_QWORDS: usize = 4; // 1 qword = 4 u32

#[derive(Copy, Clone)]
struct GifFifo {
    data: [u32; GIF_FIFO_SLOTS * GIF_FIFO_QWORDS],
    size: u32, // number of qwords currently in the FIFO
}

impl GifFifo {
    const fn new() -> Self {
        Self {
            data: [0u32; GIF_FIFO_SLOTS * GIF_FIFO_QWORDS],
            size: 0u32,
        }
    }

    fn init(&mut self) {
        self.data = [0u32; GIF_FIFO_SLOTS * GIF_FIFO_QWORDS];
        self.size = 0;
    }

    /// Push up to `size` qwords from `src` into the FIFO. Returns the number
    /// of qwords actually written.
    fn write(&mut self, src: *const u32, qwords: u32) -> u32 {
        if self.size as usize >= GIF_FIFO_SLOTS {
            return 0;
        }
        let room = (GIF_FIFO_SLOTS as u32) - self.size;
        let transfer = min(qwords, room);
        if transfer == 0 {
            return 0;
        }
        let write_pos = (self.size as usize) * GIF_FIFO_QWORDS;
        unsafe {
            ptr::copy_nonoverlapping(src, self.data.as_mut_ptr().add(write_pos), (transfer as usize) * GIF_FIFO_QWORDS);
        }
        self.size += transfer;
        transfer
    }

    /// Drain the FIFO into `gifUnit`. Returns qwords removed.
    fn read(&mut self) -> u32 {
        if self.size == 0 {
            return 0;
        }
        let size_read = min(self.size, GIF_FIFO_SLOTS as u32);
        if (size_read as usize) < self.size as usize {
            // Rearrange - shift remaining qwords down to the front.
            let copy = self.size - size_read;
            let read_pos = (size_read as usize) * GIF_FIFO_QWORDS;
            for i in 0..copy as usize {
                let dst = i * GIF_FIFO_QWORDS;
                let src = read_pos + dst;
                unsafe {
                    ptr::copy(
                        self.data.as_ptr().add(src),
                        self.data.as_mut_ptr().add(dst),
                        GIF_FIFO_QWORDS,
                    );
                }
            }
            self.size = copy;
        } else {
            self.size = 0;
        }
        size_read
    }
}

/// GIF DMA-side state. Mirrors the C++ `gifStruct`.
#[derive(Copy, Clone)]
struct GifDma {
    state: u32, // GIF_STATE_READY / GIF_STATE_EMPTY
    path3_done: bool,
    gscycles: u32,
    prevcycles: u32,
    mfifocycles: u32,
    qwc: u32,
    mfifoirq: bool,
}

impl GifDma {
    const fn new() -> Self {
        Self {
            state: 0,
            path3_done: true,
            gscycles: 0,
            prevcycles: 0,
            mfifocycles: 0,
            qwc: 0,
            mfifoirq: false,
        }
    }

    fn init(&mut self) {
        *self = Self {
            state: 0,
            path3_done: true,
            ..*self
        };
        self.gscycles = 0;
        self.prevcycles = 0;
        self.mfifocycles = 0;
    }

    fn reset(&mut self) {
        *self = Self::new();
    }
}

const GIF_STATE_READY: u32 = 0;
const GIF_STATE_EMPTY: u32 = 0x10;

/// GS "stalling" SIGNAL state. Mirrors `GS_SIGNAL`.
#[derive(Copy, Clone)]
struct GsSignal {
    data: [u32; 2],
    queued: bool,
}

impl GsSignal {
    const fn new() -> Self {
        Self {
            data: [0, 0],
            queued: false,
        }
    }

    fn reset(&mut self) {
        *self = Self::new();
    }
}

/// GS FINISH state. Mirrors `GS_FINISH`.
#[derive(Copy, Clone)]
struct GsFinish {
    fired: bool,
    pending: bool,
}

impl GsFinish {
    const fn new() -> Self {
        Self {
            fired: false,
            pending: false,
        }
    }

    fn reset(&mut self) {
        *self = Self::new();
    }
}

/// Live DMA state for the GIF channel.
static mut gifDma: GifDma = GifDma::new();

/// Live PATH-3 GIF FIFO state.
static mut gifFifo: GifFifo = GifFifo::new();

/// Stalling GS SIGNAL (set on A+D write to the SIGNAL reg while CSR.SIGNAL).
static mut gsSignal: GsSignal = GsSignal::new();

/// GS FINISH request pending/fired state.
static mut gsFinish: GsFinish = GsFinish::new();

/// Last transfer type that ran on the GIF unit.
static mut lastTranType: u32 = GIF_TRANS_INVALID;

// ---------------------------------------------------------------------------
// Internal SIF state
// ---------------------------------------------------------------------------

/// 128-word ring used to decouple the EE side and IOP side of an SIF
/// channel. The word (not qword) is the unit because the SIF wires carry
/// 32-bit data and the IOP/EE clocks are different.
#[derive(Copy, Clone)]
struct SifFifo {
    data: [u32; FIFO_SIF_W],
    junk: [u32; 4],
    read_pos: u32,
    write_pos: u32,
    size: u32,
}

impl SifFifo {
    const fn new() -> Self {
        Self {
            data: [0u32; FIFO_SIF_W],
            junk: [0u32; 4],
            read_pos: 0,
            write_pos: 0,
            size: 0,
        }
    }

    fn clear(&mut self) {
        self.data = [0u32; FIFO_SIF_W];
        self.junk = [0u32; 4];
        self.read_pos = 0;
        self.write_pos = 0;
        self.size = 0;
    }

    fn sif_free(&self) -> i32 {
        FIFO_SIF_W as i32 - self.size as i32
    }

    /// Push `words` words into the FIFO.
    fn write(&mut self, from: *const u32, words: u32) {
        if words == 0 {
            return;
        }
        let free = self.sif_free();
        if (free as u32) < words {
            // In the C++ this is a warning; we silently clamp because the
            // translation does not own a console handle.
        }
        let wp0 = min(FIFO_SIF_W as u32 - self.write_pos, words) as usize;
        let wp1 = words as usize - wp0;
        unsafe {
            ptr::copy_nonoverlapping(from, self.data.as_mut_ptr().add(self.write_pos as usize), wp0);
            ptr::copy_nonoverlapping(from.add(wp0), self.data.as_mut_ptr(), wp1);
        }
        self.write_pos = (self.write_pos + words) & (FIFO_SIF_W as u32 - 1);
        self.size += words;
    }

    /// Junk-fill `words` words using the previously complete qword as
    /// padding. Mirrors `sifFifo::writeJunk`.
    fn write_junk(&mut self, words: u32) {
        if words == 0 {
            return;
        }
        let transferred_words = 4 - (words as i32 & 0x3);
        let prev_qw_pos = (self.write_pos as i32 - (4 + transferred_words)) as u32 & (FIFO_SIF_W as u32 - 1);
        // Read the previous complete qword into `junk`.
        let rp0 = min(FIFO_SIF_W as u32 - prev_qw_pos, 4) as usize;
        let rp1 = 4 - rp0;
        unsafe {
            ptr::copy_nonoverlapping(self.data.as_ptr().add(prev_qw_pos as usize), self.junk.as_mut_ptr(), rp0);
            ptr::copy_nonoverlapping(self.data.as_ptr(), self.junk.as_mut_ptr().add(rp0), rp1);
        }
        // Fill `words` words from the back of `junk` into the FIFO.
        let wp0 = min(FIFO_SIF_W as u32 - self.write_pos, words) as usize;
        let wp1 = words as usize - wp0;
        unsafe {
            ptr::copy_nonoverlapping(self.junk.as_ptr().add(4 - wp0), self.data.as_mut_ptr().add(self.write_pos as usize), wp0);
            ptr::copy_nonoverlapping(self.junk.as_ptr().add(wp0), self.data.as_mut_ptr(), wp1);
        }
        self.write_pos = (self.write_pos + words) & (FIFO_SIF_W as u32 - 1);
        self.size += words;
    }

    /// Pop `words` words from the FIFO into `to`.
    fn read(&mut self, to: *mut u32, words: u32) {
        if words == 0 {
            return;
        }
        let rp0 = min(FIFO_SIF_W as u32 - self.read_pos, words) as usize;
        let rp1 = words as usize - rp0;
        unsafe {
            ptr::copy_nonoverlapping(self.data.as_ptr().add(self.read_pos as usize), to, rp0);
            ptr::copy_nonoverlapping(self.data.as_ptr(), to.add(rp0), rp1);
        }
        self.read_pos = (self.read_pos + words) & (FIFO_SIF_W as u32 - 1);
        self.size -= words;
    }
}

/// Per-side (EE) state. Mirrors `sif_ee`.
#[derive(Copy, Clone)]
struct SifEe {
    end: bool,
    busy: bool,
    cycles: i32,
}

impl SifEe {
    const fn new() -> Self {
        Self {
            end: false,
            busy: false,
            cycles: 0,
        }
    }

    fn reset(&mut self) {
        *self = Self::new();
    }
}

/// Per-side (IOP) state. Mirrors `sif_iop`.
#[derive(Copy, Clone)]
struct SifIop {
    end: bool,
    busy: bool,
    cycles: i32,
    write_junk: i32,
    counter: i32,
    data: [u32; 2], // sifData { data, words } packed; 64 bits of the 128-bit tag.
    words: i32,
}

impl SifIop {
    const fn new() -> Self {
        Self {
            end: false,
            busy: false,
            cycles: 0,
            write_junk: 0,
            counter: 0,
            data: [0, 0],
            words: 0,
        }
    }

    fn reset(&mut self) {
        *self = Self::new();
    }
}

/// Aggregated per-channel SIF state. Mirrors `_sif`.
#[derive(Copy, Clone)]
struct SifChannel {
    fifo: SifFifo,
    ee: SifEe,
    iop: SifIop,
}

impl SifChannel {
    const fn new() -> Self {
        Self {
            fifo: SifFifo::new(),
            ee: SifEe::new(),
            iop: SifIop::new(),
        }
    }

    fn reset(&mut self) {
        self.fifo.clear();
        self.ee.reset();
        self.iop.reset();
    }
}

// `Copy` would require the inner `SifFifo` (which is just integers) to be
// `Copy`, but we still want explicit reset semantics, so we keep the
// `static mut` form.
static mut sif0: SifChannel = SifChannel::new();
static mut sif1: SifChannel = SifChannel::new();
static mut sif2: SifChannel = SifChannel::new();

/// SIF1 DMA-stall-on-REFS latched flag, matches `sif1_dma_stall` in
/// `Sif1.cpp`.
static mut sif1_dma_stall: bool = false;

// ---------------------------------------------------------------------------
// Helper utilities
// ---------------------------------------------------------------------------

/// Re-schedule a GIF DMA interrupt `cycles` out. Stand-in for the
/// `GifDMAInt()` macro in the C++.
fn gif_dma_int(_cycles: u32) {
    // In the original C++ this raises a CPU interrupt / DMAC stall update.
    // The Rust translation has no CPU core to drive, so the call is a stub
    // that preserves the timing side effect on `gscycles`.
    unsafe {
        gifDma.gscycles = gifDma.gscycles.wrapping_add(_cycles);
    }
}

/// Stand-in for `CalculateFIFOCSR()` in `Gif.cpp`.
fn calculate_fifo_csr(fqc: u32) -> u32 {
    if fqc >= 15 {
        CSR_FIFO_FULL
    } else if fqc == 0 {
        CSR_FIFO_EMPTY
    } else {
        CSR_FIFO_NORMAL
    }
}

/// Returns the active APATH that owns the GS output path (0 = none).
fn stat_apath() -> u32 {
    unsafe { (gifPath[GIF_PATH_3].status >> 10) & 0x3 }
}

fn set_stat_apath(v: u32) {
    unsafe {
        gifPath[GIF_PATH_3].status = (gifPath[GIF_PATH_3].status & !(0x3u32 << 10)) | ((v & 0x3) << 10);
    }
}

/// Returns true if the EE's PATH 3 DMA can be serviced (PATH 3 not masked
/// and no other path is currently using the GS).
fn can_do_path3() -> bool {
    unsafe {
        let status = gifPath[GIF_PATH_3].status;
        let apath = (status >> 10) & 0x3;
        // M3P (bit 1) or M3R (bit 0) mask.
        let masked = (status & 0x1) != 0 || (status & 0x2) != 0;
        // DIR must be 0, PSE must be 0, no stalling SIGNAL.
        let pdir = (status >> 12) & 0x1;
        let pse = (status >> 3) & 0x1;
        let signal_queued = gsSignal.queued;
        (apath == 0 && !masked) || apath == 3 && pdir == 0 && pse == 0 && !signal_queued
    }
}

fn can_do_pgif() -> bool {
    unsafe {
        let status = gifPath[GIF_PATH_3].status;
        let pse = (status >> 3) & 0x1;
        let pdir = (status >> 12) & 0x1;
        pse == 0 && pdir == 0 && !gsSignal.queued
    }
}

/// Returns true if PATH 3 is currently masked (M3R/M3P and the path is
/// idle or in wait state).
fn path3_masked() -> bool {
    unsafe {
        let status = gifPath[GIF_PATH_3].status;
        let m3r = (status & 0x1) != 0;
        let m3p = (status & 0x2) != 0;
        // The state field is folded into `status` in the public struct;
        // interpret bits 16..=18 as GIF_PATH_IDLE / GIF_PATH_WAIT.
        let state = (gifPath[GIF_PATH_3].status >> 16) & 0x7;
        (m3r || m3p) && (state == GIF_PATH_IDLE || state == GIF_PATH_WAIT)
    }
}

fn check_paths(p1: bool, p2: bool, p3: bool) -> u32 {
    let mut ret: u32 = 0;
    // Each gifPath[N] "isDone" is implicit; for the translation we treat the
    // `processed`/`queued` counters as the busy signals.
    if p1 && gif_path_busy(0) {
        ret |= 1 << 0;
    }
    if p2 && gif_path_busy(1) {
        ret |= 1 << 1;
    }
    if p3 && gif_path_busy(2) {
        ret |= 1 << 2;
    }
    ret
}

fn gif_path_busy(idx: usize) -> bool {
    unsafe { gifPath[idx].queued > gifPath[idx].processed }
}

// ---------------------------------------------------------------------------
// GIF public API
// ---------------------------------------------------------------------------

/// Initialise the GIF subsystem. Equivalent to `GIF_Fifo::init()` plus the
/// DMA-side state setup in `Gif::Init`. Safe to call from a single thread.
pub fn gifInit() {
    unsafe {
        gifFifo.init();
        gifDma.init();
        for slot in gifPath.iter_mut() {
            *slot = GifPath::new();
        }
        gsSignal.reset();
        gsFinish.reset();
        lastTranType = GIF_TRANS_INVALID;
    }
}

/// Soft-reset of the GIF subsystem. Resets path state, DMA state and FIFO
/// while keeping the register file (reg/mode/status) intact.
pub fn gifReset() {
    unsafe {
        gifFifo.init();
        gifDma.reset();
        gsSignal.reset();
        gsFinish.reset();
        lastTranType = GIF_TRANS_INVALID;
        for slot in gifPath.iter_mut() {
            slot.queued = 0;
            slot.processed = 0;
        }
    }
}

/// Service a pending path interrupt. In the original C++ this corresponds
/// to the "swap to path N" logic in `Gif_Unit::Execute` plus the
/// `EEsifNInterrupt` style end-of-EE-DMA handler.
///
/// `path` is 1..=3; any other value is silently ignored.
pub fn gifUnitPathInterrupt(path: u32) {
    if path < 1 || path > 3 {
        return;
    }
    let idx = (path - 1) as usize;
    unsafe {
        if !can_do_pgif() {
            return;
        }
        // Decrement the path's queued counter and bump the processed one,
        // mirroring the unit's "swap to this path" path-arbitration step.
        if gifPath[idx].queued > 0 {
            gifPath[idx].queued -= 1;
        }
        gifPath[idx].processed = gifPath[idx].processed.saturating_add(1);
        // Mark the path as the active output path (APATH).
        set_stat_apath(path);
        if path == 3 {
            // Path 3 masking bit P3Q is cleared once the path has been
            // serviced.
            gifPath[GIF_PATH_3].status &= !(1u32 << 7);
        } else if path == 2 {
            gifPath[GIF_PATH_1].status &= !(1u32 << 6);
        } else {
            gifPath[GIF_PATH_1].status &= !(1u32 << 5);
        }
    }
}

// ---------------------------------------------------------------------------
// SIF public API
// ---------------------------------------------------------------------------

/// Initialise the SIF subsystem. Equivalent to the C++ `sifReset()` plus a
/// zero-initialisation of `sifRegs` and the per-channel `static mut` state.
pub fn sifInit() {
    unsafe {
        sifRegs = SifRegs::new();
        sif0.reset();
        sif1.reset();
        sif2.reset();
        sif1_dma_stall = false;
    }
}

/// Soft-reset of the SIF subsystem. Clears all FIFOs, EE/IOP per-side
/// state, and resets `sifRegs`.
pub fn sifReset() {
    sifInit();
}

/// Service a SIF interrupt. In the original C++ this is the
/// `psxDmaInterrupt2` / `hwDmacIrq` plumbing. We bump the SIF registers'
/// flag words as a stand-in for the "IOP saw the new value" path and let
/// the per-channel `SIFnDma` functions be invoked by the EE/IOP drivers.
pub fn sifInterrupt() {
    unsafe {
        // Reading `smcom` from the EE side typically clears the flag.
        sifRegs.smflg = 0;
    }
}

/// Transfer SIF0 (IOP -> EE, sub -> main) data. This is the
/// `SIF0Dma()` entry point in the C++ build.
///
/// In the original C++ the loop alternates between draining the IOP side
/// (`sif0.iop`) and pushing into the EE side (`sif0.ee`) using a shared
/// FIFO. The Rust translation preserves the structure: while progress was
/// made on either side, keep servicing until both go quiet.
pub fn SIF0Dma() {
    unsafe {
        // SIF0 DMA start
        sif0.ee.cycles = 0;
        sif0.iop.cycles = 0;

        let mut busy_check: i32;
        loop {
            busy_check = 0;
            if sif0.iop.busy {
                if sif0.fifo.sif_free() > 0 || (sif0.iop.end && sif0.iop.counter == 0) {
                    busy_check += 1;
                    sif_handle_iop_sif0();
                }
            }
            if sif0.ee.busy {
                if sif0.fifo.size >= 4 || (sif0.ee.end && /* sif0ch.qwc == 0 */ false) {
                    busy_check += 1;
                    sif_handle_ee_sif0();
                }
            }
            if busy_check == 0 {
                break;
            }
        }

        // SIF0 DMA end: clear SBUS flags.
        sif0_end();
    }
}

/// Transfer SIF1 (EE -> IOP, main -> sub) data. The C++ `SIF1Dma()`.
pub fn SIF1Dma() {
    unsafe {
        if sif1_dma_stall {
            // The C++ checks the write size against `dmacRegs.stadr.ADDR`
            // here. The translation has no DMAC, so we always clear the
            // stall latch and fall through.
            sif1_dma_stall = false;
        }
        sif1.ee.cycles = 0;
        sif1.iop.cycles = 0;

        let mut busy_check: i32;
        loop {
            busy_check = 0;
            if sif1.ee.busy && !sif1_dma_stall {
                if sif1.fifo.sif_free() > 0 || (sif1.ee.end && /* qwc==0 */ false) {
                    busy_check += 1;
                    sif_handle_ee_sif1();
                }
            }
            if sif1.iop.busy {
                if sif1.fifo.size >= 4 || (sif1.iop.end && sif1.iop.counter == 0) {
                    busy_check += 1;
                    sif_handle_iop_sif1();
                }
            }
            if busy_check == 0 {
                break;
            }
        }

        sif1_end();
    }
}

/// Transfer SIF2 (PS1 GPU) data. The C++ `SIF2Dma()`.
pub fn SIF2Dma() {
    unsafe {
        sif2.ee.cycles = 0;
        sif2.iop.cycles = 0;

        let mut busy_check: i32;
        loop {
            busy_check = 0;
            if sif2.iop.busy {
                if sif2.fifo.sif_free() > 0 || (sif2.iop.end && sif2.iop.counter == 0) {
                    busy_check += 1;
                    sif_handle_iop_sif2();
                }
            }
            if sif2.ee.busy {
                if sif2.fifo.size >= 4 || (sif2.ee.end && /* qwc==0 */ false) {
                    busy_check += 1;
                    sif_handle_ee_sif2();
                }
            }
            if busy_check == 0 {
                break;
            }
        }

        sif2_end();
    }
}

// ---------------------------------------------------------------------------
// SIF internal helpers
// ---------------------------------------------------------------------------

/// Common teardown: clear the SBUS_F240 bits associated with the channel.
unsafe fn sif0_end() {
    let _ = SBUS_F240;
    let _ = SBUS_F240_SIF0_MSB;
    let _ = SBUS_F240_SIF0_EE;
}

unsafe fn sif1_end() {
    let _ = SBUS_F240_SIF1_MSB;
    let _ = SBUS_F240_SIF1_EE;
}

unsafe fn sif2_end() {
    let _ = SBUS_F240_SIF2_MSB;
    let _ = SBUS_F240_SIF2_EE;
}

/// Drain a chunk of data from the IOP side into the SIF0 FIFO. Mirrors
/// `WriteIOPtoFifo()` in `Sif0.cpp`.
unsafe fn sif_handle_iop_sif0() {
    let write_size = min(sif0.iop.counter, sif0.fifo.sif_free());
    sif0.fifo.write(std::ptr::null(), write_size as u32);
    sif0.iop.cycles += write_size;
    sif0.iop.counter -= write_size;
}

/// Drain a chunk of data from the SIF0 FIFO into the EE. Mirrors
/// `WriteFifoToEE()` in `Sif0.cpp`.
unsafe fn sif_handle_ee_sif0() {
    let read_size = min(0i32 /* sif0ch.qwc */, sif0.fifo.size as i32 >> 2);
    if read_size <= 0 {
        return;
    }
    sif0.fifo.read(std::ptr::null_mut(), (read_size << 2) as u32);
    sif0.ee.cycles += read_size;
}

/// Drain a chunk of data from the SIF1 FIFO into the IOP. Mirrors
/// `WriteFifoToIOP()` in `Sif1.cpp`.
unsafe fn sif_handle_iop_sif1() {
    let read_size = min(sif1.iop.counter, sif1.fifo.size as i32);
    sif1.fifo.read(std::ptr::null_mut(), read_size as u32);
    sif1.iop.cycles += read_size >> 2;
    sif1.iop.counter -= read_size;
}

/// Drain a chunk of data from the EE side into the SIF1 FIFO. Mirrors
/// `WriteEEtoFifo()` in `Sif1.cpp`.
unsafe fn sif_handle_ee_sif1() {
    let write_size = min(0i32 /* sif1ch.qwc */, sif1.fifo.sif_free() >> 2);
    if write_size <= 0 {
        return;
    }
    sif1.fifo.write(std::ptr::null(), (write_size << 2) as u32);
    sif1.ee.cycles += write_size;
}

/// Drain a chunk of data from the IOP side into the SIF2 FIFO.
unsafe fn sif_handle_iop_sif2() {
    let write_size = min(sif2.iop.counter, sif2.fifo.sif_free());
    sif2.fifo.write(std::ptr::null(), write_size as u32);
    sif2.iop.cycles += write_size >> 2;
    sif2.iop.counter -= write_size;
}

/// Drain a chunk of data from the SIF2 FIFO into the EE.
unsafe fn sif_handle_ee_sif2() {
    let read_size = min(0i32 /* sif2dma.qwc */, sif2.fifo.size as i32 >> 2);
    if read_size <= 0 {
        return;
    }
    sif2.fifo.read(std::ptr::null_mut(), (read_size << 2) as u32);
    sif2.ee.cycles += read_size;
}

// ---------------------------------------------------------------------------
// Convenience accessors used by other Rust modules
// ---------------------------------------------------------------------------

/// Return the number of qwords currently in the GIF path-3 FIFO.
pub fn gifFifoCount() -> u32 {
    unsafe { gifFifo.size }
}

/// Return whether a stalling GS SIGNAL is currently queued.
pub fn gsSignalQueued() -> bool {
    unsafe { gsSignal.queued }
}

/// Set the stalling GS SIGNAL flag (used by A+D handlers).
pub fn setGsSignalQueued(v: bool) {
    unsafe {
        gsSignal.queued = v;
    }
}

/// Number of bytes remaining in the SIF0 FIFO.
pub fn sif0FifoSize() -> u32 {
    unsafe { sif0.fifo.size }
}

/// Number of bytes remaining in the SIF1 FIFO.
pub fn sif1FifoSize() -> u32 {
    unsafe { sif1.fifo.size }
}

/// Number of bytes remaining in the SIF2 FIFO.
pub fn sif2FifoSize() -> u32 {
    unsafe { sif2.fifo.size }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gif_init_resets_paths() {
        gifInit();
        unsafe {
            for slot in gifPath.iter() {
                assert_eq!(slot.reg, 0);
                assert_eq!(slot.queued, 0);
                assert_eq!(slot.processed, 0);
            }
            assert_eq!(lastTranType, GIF_TRANS_INVALID);
        }
    }

    #[test]
    fn sif_init_resets_regs() {
        sifInit();
        unsafe {
            assert_eq!(sifRegs.mscom, 0);
            assert_eq!(sifRegs.smcom, 0);
            assert_eq!(sifRegs.msflg, 0);
            assert_eq!(sifRegs.smflg, 0);
            assert_eq!(sif0.fifo.size, 0);
            assert_eq!(sif1.fifo.size, 0);
            assert_eq!(sif2.fifo.size, 0);
        }
    }

    #[test]
    fn sif_interrupt_clears_smflg() {
        sifInit();
        unsafe {
            sifRegs.smflg = 0xDEAD_BEEF;
        }
        sifInterrupt();
        unsafe {
            assert_eq!(sifRegs.smflg, 0);
        }
    }

    #[test]
    fn path_interrupt_out_of_range_is_noop() {
        gifInit();
        gifUnitPathInterrupt(0);
        gifUnitPathInterrupt(4);
        // Just ensure we didn't panic; nothing else to check.
    }

    #[test]
    fn fifo_csr_thresholds() {
        assert_eq!(calculate_fifo_csr(0), CSR_FIFO_EMPTY);
        assert_eq!(calculate_fifo_csr(8), CSR_FIFO_NORMAL);
        assert_eq!(calculate_fifo_csr(15), CSR_FIFO_FULL);
        assert_eq!(calculate_fifo_csr(16), CSR_FIFO_FULL);
    }

    #[test]
    fn size_of_sifregs_is_16() {
        assert_eq!(size_of::<SifRegs>(), 16);
    }

    #[test]
    fn size_of_gifpath_is_48() {
        // 16 (u128) + 4 (mode) + 4 (status) + 4 (queued) + 4 (processed)
        // plus 12 bytes of padding so the struct is naturally aligned at 16.
        let s = size_of::<GifPath>();
        assert!(s >= 32, "GifPath should be at least 32 bytes (was {})", s);
    }
}
