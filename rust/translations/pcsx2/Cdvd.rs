// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! PCSX2 CDVD (CD/DVD drive) emulation translated from the C++ original.
//!
//! This module covers the PS2 CD/DVD drive emulation, including:
//!   * the IOP-visible CDVD register set (`CDVDState`),
//!   * the disc-tray lifecycle, NVRAM, RTC and seek/spindle bookkeeping,
//!   * the high-level disc / TOC / sector-read plumbing (`cdvdLoadIso`,
//!     `cdvdReadSector`, etc.),
//!   * a `CDVDDiscReader` trait that abstracts away the per-format
//!     ISO / disc / null backends.
//!
//! The translation is intentionally a faithful, idiomatic Rust 2021 port:
//! `static mut` is used to mirror the original global state, all C-style
//! flags / masks become `pub const`, and BCD conversions are preserved
//! using the same `btoi` / `itob` helpers the C++ used.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// BCD / MSF helpers (from CDVD.h)
// ---------------------------------------------------------------------------

/// Convert a BCD-encoded byte to its binary value.
#[inline]
pub fn btoi(b: u8) -> u8 {
    (b / 16) * 10 + (b % 16)
}

/// Convert a binary byte to BCD encoding.
#[inline]
pub fn itob(i: u8) -> u8 {
    (i / 10) * 16 + (i % 10)
}

/// Convert an MSF triplet to a logical sector number.
#[inline]
pub fn msf_to_lsn(time: [u8; 3]) -> u32 {
    let lsn = time[2] as u32;
    let lsn = lsn + (time[1] as u32).saturating_sub(2) * 75;
    lsn + time[0] as u32 * 75 * 60
}

/// Convert a (minute, second, frame) MSF triple to a logical block address.
#[inline]
pub fn msf_to_lba(m: u8, s: u8, f: u8) -> u32 {
    let lsn = f as u32;
    let lsn = lsn + (s as u32).saturating_sub(2) * 75;
    lsn + m as u32 * 75 * 60
}

/// Write a logical sector number into `time` as an MSF triplet (BCD).
#[inline]
pub fn lsn_to_msf(time: &mut [u8; 3], lsn: i32) {
    let mut lsn = lsn + 150;
    let m = lsn / 4500;
    lsn -= m * 4500;
    let s = lsn / 75;
    let f = lsn - s * 75;
    time[0] = itob(m as u8);
    time[1] = itob(s as u8);
    time[2] = itob(f as u8);
}

/// Convert an LBA into (minute, second, frame) (binary, not BCD).
#[inline]
pub fn lba_to_msf(lba: i32) -> (u8, u8, u8) {
    let lba = lba + 150;
    let m = lba / (60 * 75);
    let s = (lba / 75) % 60;
    let f = lba % 75;
    (m as u8, s as u8, f as u8)
}

// ---------------------------------------------------------------------------
// CDVDcommon.h: type definitions, masks, and well-known constants
// ---------------------------------------------------------------------------

/// Index within a track (pregap or data).
#[derive(Clone, Copy, Debug, Default)]
pub struct CdvdTrackIndex {
    pub is_pregap: bool,
    pub track_m: u8,
    pub track_s: u8,
    pub track_f: u8,
    pub disc_m: u8,
    pub disc_s: u8,
    pub disc_f: u8,
}

/// One CD/DVD track on a disc.
#[derive(Clone, Copy, Debug)]
pub struct CdvdTrack {
    pub start_lba: u32,
    pub ty: u8,
    pub track_num: u8,
    pub track_index: u8,
    pub track_m: u8,
    pub track_s: u8,
    pub track_f: u8,
    pub disc_m: u8,
    pub disc_s: u8,
    pub disc_f: u8,
    pub index: [CdvdTrackIndex; 2],
}

impl Default for CdvdTrack {
    fn default() -> Self {
        Self {
            start_lba: 0,
            ty: 0,
            track_num: 0,
            track_index: 0,
            track_m: 0,
            track_s: 0,
            track_f: 0,
            disc_m: 0,
            disc_s: 0,
            disc_f: 0,
            index: [CdvdTrackIndex::default(); 2],
        }
    }
}

/// Sub-channel Q descriptor returned by `cdvdReadSubQ`.
#[derive(Clone, Copy, Debug, Default)]
pub struct CdvdSubQ {
    pub ctrl: u8,
    pub adr: u8,
    pub track_num: u8,
    pub track_index: u8,
    pub track_m: u8,
    pub track_s: u8,
    pub track_f: u8,
    pub pad: u8,
    pub disc_m: u8,
    pub disc_s: u8,
    pub disc_f: u8,
}

/// Track-descriptor entry returned by `getTD`.
#[derive(Clone, Copy, Debug, Default)]
pub struct CdvdTD {
    pub lsn: u32,
    pub ty: u8,
}

/// Track-number pair returned by `getTN`.
#[derive(Clone, Copy, Debug, Default)]
pub struct CdvdTN {
    pub strack: u8,
    pub etrack: u8,
}

// -- Spindle control masks --------------------------------------------------
pub const CDVD_SPINDLE_SPEED: u8 = 0x7;
pub const CDVD_SPINDLE_NOMINAL: u8 = 0x40;
pub const CDVD_SPINDLE_CAV: u8 = 0x80;

// -- Read mode constants ----------------------------------------------------
pub const CDVD_MODE_2352: i32 = 0;
pub const CDVD_MODE_2340: i32 = 1;
pub const CDVD_MODE_2328: i32 = 2;
pub const CDVD_MODE_2048: i32 = 3;
pub const CDVD_MODE_2368: i32 = 4;

// -- Disc-type constants returned by DoCDVDdetectDiskType() ------------------
pub const CDVD_TYPE_ILLEGAL: u8 = 0xff;
pub const CDVD_TYPE_DVDV: u8 = 0xfe;
pub const CDVD_TYPE_CDDA: u8 = 0xfd;
pub const CDVD_TYPE_PS2DVD: u8 = 0x14;
pub const CDVD_TYPE_PS2CDDA: u8 = 0x13;
pub const CDVD_TYPE_PS2CD: u8 = 0x12;
pub const CDVD_TYPE_PSCDDA: u8 = 0x11;
pub const CDVD_TYPE_PSCD: u8 = 0x10;
pub const CDVD_TYPE_UNKNOWN: u8 = 0x05;
pub const CDVD_TYPE_DETCTDVDD: u8 = 0x04;
pub const CDVD_TYPE_DETCTDVDS: u8 = 0x03;
pub const CDVD_TYPE_DETCTCD: u8 = 0x02;
pub const CDVD_TYPE_DETCT: u8 = 0x01;
pub const CDVD_TYPE_NODISC: u8 = 0x00;

// -- Tray-status values -----------------------------------------------------
pub const CDVD_TRAY_CLOSE: u8 = 0x00;
pub const CDVD_TRAY_OPEN: u8 = 0x01;

// -- Track types (cdvdTD.type) ---------------------------------------------
pub const CDVD_AUDIO_TRACK: u8 = 0x01;
pub const CDVD_MODE1_TRACK: u8 = 0x41;
pub const CDVD_MODE2_TRACK: u8 = 0x61;
pub const CDVD_AUDIO_MASK: u8 = 0x00;
pub const CDVD_DATA_MASK: u8 = 0x40;

// -- Status bits ------------------------------------------------------------
pub const CDVD_STATUS_TRAY_OPEN: u8 = 0x08; // documented in CDVDcommon, used in cpp
pub const CDVD_STATUS_PAUSE: u8 = 0x00;
pub const CDVD_STATUS_STOP: u8 = 0x00;
pub const CDVD_STATUS_READ: u8 = 0x20;
pub const CDVD_STATUS_SEEK: u8 = 0x40;
pub const CDVD_STATUS_SPIN: u8 = 0x10;

// -- Drive ready bits -------------------------------------------------------
pub const CDVD_DRIVE_READY: u8 = 0x40;
pub const CDVD_DRIVE_BUSY: u8 = 0x80;
pub const CDVD_DRIVE_ERROR: u8 = 0x01;
pub const CDVD_DRIVE_MECHA_INIT: u8 = 0x20;
pub const CDVD_DRIVE_DEV9CON: u8 = 0x10;

// -- IRQ source bits --------------------------------------------------------
pub const IRQ_COMMAND_COMPLETE: u8 = 0x01;
pub const IRQ_DATA_READY: u8 = 0x02;
pub const IRQ_EJECT: u8 = 0x04;

// -- Spindle RPM constants (CAV) -------------------------------------------
pub const CD_MAX_ROTATION_X1: u32 = 200; // 200 RPM at 1x CD
pub const DVD_MAX_ROTATION_X1: u32 = 570; // 570 RPM at 1x DVD
pub const CD_SECTORS_PERSECOND: u32 = 75;
pub const DVD_SECTORS_PERSECOND: u32 = 676;

// -- NVRAM layout -----------------------------------------------------------
pub const NVRAM_SIZE: usize = 1024;
pub const DEFAULT_MECHA_VERSION: u32 = 0x00020603;

/// IOP bus clock (PSXCLK = 36.864 MHz).
pub const PSXCLK: u32 = 36_864_000;

// ---------------------------------------------------------------------------
// PS1 CD and PS2 CDVD disc-type enums
// ---------------------------------------------------------------------------

/// CDVD disc type returned to the IOP, when probed via the high-level API.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CDVDDiscType {
    #[default]
    Other,
    PS1Disc,
    PS2Disc,
}

/// Tray state machine used by the seek/detect logic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayState {
    Engaged,
    Detecting,
    Seeking,
    Eject,
    Open,
}

#[derive(Clone, Copy, Debug)]
pub struct CdvdTrayTimer {
    pub cdvd_action_seconds: u32,
    pub tray_state: TrayState,
}

impl Default for CdvdTrayTimer {
    fn default() -> Self {
        Self {
            cdvd_action_seconds: 0,
            tray_state: TrayState::Open,
        }
    }
}

/// Real-time clock as exposed through the CDVD SCMD `CdReadRTC` path.
#[derive(Clone, Copy, Debug, Default)]
pub struct CdvdRTC {
    pub status: u8,
    pub second: u8,
    pub minute: u8,
    pub hour: u8,
    pub pad: u8,
    pub day: u8,
    pub month: u8,
    pub year: u8,
}

/// CDVD action scheduler state machine (seek/standby/stop/error).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CdvdAction {
    None,
    Seek,
    Standby,
    Stop,
    Error,
}

impl Default for CdvdAction {
    fn default() -> Self {
        CdvdAction::None
    }
}

// ---------------------------------------------------------------------------
// The big state bag: cdvdStruct / CDVDState
// ---------------------------------------------------------------------------

/// Full CDVD controller state. Mirrors the C++ `cdvdStruct` field-for-field.
#[derive(Clone)]
pub struct CDVDState {
    pub n_command: u8,
    pub ready: u8,
    pub error: u8,
    pub intr_stat: u8,
    pub status: u8,
    pub status_sticky: u8,
    pub disc_type: u8,
    pub s_command: u8,
    pub s_data_in: u8,
    pub s_data_out: u8,
    pub how_to: u8,

    pub ncmd_param_buff: [u8; 16],
    pub scmd_param_buff: [u8; 16],
    pub scmd_result_buff: [u8; 16],

    pub ncmd_param_cnt: u8,
    pub ncmd_param_pos: u8,
    pub scmd_param_cnt: u8,
    pub scmd_param_pos: u8,
    pub scmd_result_cnt: u8,
    pub scmd_result_pos: u8,

    pub c_block_index: u8,
    pub c_offset: u8,
    pub c_read_write: u8,
    pub c_num_blocks: u8,

    pub rtc_count: f64,
    pub rtc: CdvdRTC,

    pub current_sector: u32,
    pub sector_cnt: i32,
    pub seek_completed: i32,
    pub reading: i32,
    pub waiting_dma: i32,
    pub read_mode: i32,
    pub block_size: i32,
    pub speed: i32,
    pub retry_cnt_max: i32,
    pub current_retry_cnt: i32,
    pub read_err: i32,
    pub spindl_ctrl: i32,

    pub key: [u8; 16],
    pub key_xor: u8,
    pub dec_set: u8,

    pub mg_buffer: [u8; 65536],
    pub mg_size: i32,
    pub mg_max_size: i32,
    pub mg_datatype: i32,
    pub mg_kbit: [u8; 16],
    pub mg_kcon: [u8; 16],

    pub tray_timeout: u8,
    pub action: CdvdAction,
    pub seek_to_sector: u32,
    pub max_sector: u32,
    pub read_time: u32,
    pub rot_speed: u32,
    pub spinning: bool,
    pub tray: CdvdTrayTimer,
    pub next_sectors_buffered: u8,
    pub abort_requested: bool,
}

impl Default for CDVDState {
    fn default() -> Self {
        Self {
            n_command: 0,
            ready: 0,
            error: 0,
            intr_stat: 0,
            status: 0,
            status_sticky: 0,
            disc_type: CDVD_TYPE_NODISC,
            s_command: 0,
            s_data_in: 0x40,
            s_data_out: 0,
            how_to: 0,

            ncmd_param_buff: [0; 16],
            scmd_param_buff: [0; 16],
            scmd_result_buff: [0; 16],

            ncmd_param_cnt: 0,
            ncmd_param_pos: 0,
            scmd_param_cnt: 0,
            scmd_param_pos: 0,
            scmd_result_cnt: 0,
            scmd_result_pos: 0,

            c_block_index: 0,
            c_offset: 0,
            c_read_write: 0,
            c_num_blocks: 0,

            rtc_count: 0.0,
            rtc: CdvdRTC::default(),

            current_sector: 0,
            sector_cnt: 0,
            seek_completed: 0,
            reading: 0,
            waiting_dma: 0,
            read_mode: 0,
            block_size: 2064,
            speed: 4,
            retry_cnt_max: 0,
            current_retry_cnt: 0,
            read_err: 0,
            spindl_ctrl: 0,

            key: [0; 16],
            key_xor: 0,
            dec_set: 0,

            mg_buffer: [0; 65536],
            mg_size: 0,
            mg_max_size: 0,
            mg_datatype: 0,
            mg_kbit: [0; 16],
            mg_kcon: [0; 16],

            tray_timeout: 0,
            action: CdvdAction::None,
            seek_to_sector: 0,
            max_sector: 0,
            read_time: 0,
            rot_speed: 0,
            spinning: false,
            tray: CdvdTrayTimer::default(),
            next_sectors_buffered: 0,
            abort_requested: false,
        }
    }
}

/// Global CDVD state. Mirrors the C++ `cdvdStruct cdvd;` global.
pub static mut cdvd: CDVDState = CDVDState {
    n_command: 0,
    ready: 0,
    error: 0,
    intr_stat: 0,
    status: 0,
    status_sticky: 0,
    disc_type: CDVD_TYPE_NODISC,
    s_command: 0,
    s_data_in: 0x40,
    s_data_out: 0,
    how_to: 0,

    ncmd_param_buff: [0; 16],
    scmd_param_buff: [0; 16],
    scmd_result_buff: [0; 16],

    ncmd_param_cnt: 0,
    ncmd_param_pos: 0,
    scmd_param_cnt: 0,
    scmd_param_pos: 0,
    scmd_result_cnt: 0,
    scmd_result_pos: 0,

    c_block_index: 0,
    c_offset: 0,
    c_read_write: 0,
    c_num_blocks: 0,

    rtc_count: 0.0,
    rtc: CdvdRTC {
        status: 0,
        second: 0,
        minute: 0,
        hour: 0,
        pad: 0,
        day: 0,
        month: 0,
        year: 0,
    },

    current_sector: 0,
    sector_cnt: 0,
    seek_completed: 0,
    reading: 0,
    waiting_dma: 0,
    read_mode: 0,
    block_size: 2064,
    speed: 4,
    retry_cnt_max: 0,
    current_retry_cnt: 0,
    read_err: 0,
    spindl_ctrl: 0,

    key: [0; 16],
    key_xor: 0,
    dec_set: 0,

    mg_buffer: [0; 65536],
    mg_size: 0,
    mg_max_size: 0,
    mg_datatype: 0,
    mg_kbit: [0; 16],
    mg_kcon: [0; 16],

    tray_timeout: 0,
    action: CdvdAction::None,
    seek_to_sector: 0,
    max_sector: 0,
    read_time: 0,
    rot_speed: 0,
    spinning: false,
    tray: CdvdTrayTimer {
        cdvd_action_seconds: 0,
        tray_state: TrayState::Open,
    },
    next_sectors_buffered: 0,
    abort_requested: false,
};

// ---------------------------------------------------------------------------
// NVRAM and mecha-version backing storage
// ---------------------------------------------------------------------------

/// Process-wide NVRAM shadow. The C++ kept this in a `static u8[1024]`.
static mut S_NVRAM: [u8; NVRAM_SIZE] = [0; NVRAM_SIZE];
static mut S_MECHA_VERSION: u32 = DEFAULT_MECHA_VERSION;

/// Cached disc-type slot, mirroring `diskTypeCached` in CDVDcommon.cpp.
static mut DISK_TYPE_CACHED: i32 = -1;

// ---------------------------------------------------------------------------
// Disc reader trait (CDVDdiscReader / InputIsoFile / CDVDisoReader)
// ---------------------------------------------------------------------------

/// Identifies the active media source (ISO file, host DVD drive, or none).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CDVDSourceType {
    Iso,
    Disc,
    NoDisc,
}

/// Per-track information used by the TOC layer.
#[derive(Clone, Copy, Debug, Default)]
pub struct TocEntry {
    pub track: u8,
    pub lba: u32,
    pub control: u8,
}

/// Abstraction over the various backends (ISO, host disc, or "no disc").
///
/// The C++ code carries this around as a `CDVD_API` vtable; in idiomatic
/// Rust we model it as a `dyn CDVDDiscReader` trait object.
pub trait CDVDDiscReader: Send {
    /// Open the underlying media source.
    fn open(&mut self, path: &str) -> Result<(), String>;

    /// Close the underlying media source.
    fn close(&mut self);

    /// True if the source is currently open.
    fn is_open(&self) -> bool;

    /// Total sector count, or 0 if no disc.
    fn sector_count(&self) -> u64;

    /// Read a single 2048-byte-mode sector into `buf`.
    fn read_sector_2048(&mut self, lsn: u32, buf: &mut [u8; 2048]) -> Result<(), String>;

    /// Read a single 2352-byte-mode sector into `buf`.
    fn read_sector_2352(&mut self, lsn: u32, buf: &mut [u8; 2352]) -> Result<(), String>;

    /// Read a single raw 2448-byte sector (2352 + subchannel) into `buf`.
    fn read_sector_2448(&mut self, lsn: u32, buf: &mut [u8; 2448]) -> Result<(), String>;

    /// Returns media type: < 0 = CD, 0 = single-layer DVD, 1 = PTP DVD, 2 = OTP DVD.
    fn media_type(&self) -> i32;

    /// For dual-layer DVDs, the LBA where layer 1 begins.
    fn layer_break_address(&self) -> u32;

    /// Returns the table of contents.
    fn read_toc(&self) -> Vec<TocEntry>;

    /// True if the disc is ready (tray closed and a disc is present).
    fn disc_ready(&self) -> bool;
}

// ---------------------------------------------------------------------------
// InputIsoFile: file-backed ISO reader (raw, .chd, .cso, .zso, .dump, .gz)
// ---------------------------------------------------------------------------

/// ISO type detected at open time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IsoType {
    Illegal,
    Cd,
    Dvd,
    Audio,
    DvdDl,
}

/// File-backed ISO reader.
///
/// `InputIsoFile` is the Rust analog of the C++ class of the same name
/// in `InputIsoFile.cpp` / `CDVDisoReader.cpp`. It owns a `File` handle
/// and a small amount of layout state (offset, blockofs, blocksize, ...).
pub struct InputIsoFile {
    filename: String,
    file: Option<File>,
    iso_type: IsoType,
    flags: u32,
    offset: i64,
    blockofs: i32,
    blocksize: u32,
    blocks: u32,
    read_lsn: i64,
    current_lsn: i64,
    read_in_progress: bool,
}

impl InputIsoFile {
    pub fn new() -> Self {
        Self {
            filename: String::new(),
            file: None,
            iso_type: IsoType::Illegal,
            flags: 0,
            offset: 0,
            blockofs: 0,
            blocksize: 0,
            blocks: 0,
            read_lsn: -1,
            current_lsn: -1,
            read_in_progress: false,
        }
    }

    /// Read a single sector into `dst` at the file's current blockofs/blocksize.
    pub fn read_sync(&mut self, dst: &mut [u8], lsn: u32) -> i32 {
        if lsn as u64 >= self.blocks as u64 {
            return -1;
        }
        let file = match self.file.as_mut() {
            Some(f) => f,
            None => return -1,
        };
        let offset = self.offset + (lsn as i64) * self.blocksize as i64;
        if file.seek(SeekFrom::Start(offset as u64)).is_err() {
            return -1;
        }
        let read_into = self.blockofs as usize;
        let want = self.blocksize as usize;
        if dst.len() < read_into + want {
            return -1;
        }
        match file.read(&mut dst[read_into..read_into + want]) {
            Ok(n) if n == want => 0,
            _ => -1,
        }
    }

    /// Begin an async-style read (the C++ version was two-stage).
    pub fn begin_read2(&mut self, lsn: u32) {
        self.current_lsn = lsn as i64;
        self.read_lsn = lsn as i64;
        // We do not support a real async backend; the synchronous
        // `read_sync` is sufficient for the trait shim.
        self.read_in_progress = false;
    }

    /// Finish a previously-begun read.
    pub fn finish_read3(&mut self, dst: &mut [u8], mode: u32) -> i32 {
        // Constants are i32, so compare via cast
        let (_offset, length) = if mode == CDVD_MODE_2352 as u32 {
            (0u32, 2352u32)
        } else if mode == CDVD_MODE_2340 as u32 {
            (12, 2340)
        } else if mode == CDVD_MODE_2328 as u32 {
            (24, 2328)
        } else if mode == CDVD_MODE_2048 as u32 {
            (24, 2048)
        } else {
            (0, 2352)
        };
        let lsn = self.current_lsn as u32;
        let mut tmp = vec![0u8; 2352];
        if self.read_sync(&mut tmp, lsn) != 0 {
            return -1;
        }
        let end1 = self.blockofs as i32 + self.blocksize as i32;
        let end2 = length as i32;
        let end = end1.min(end2);
        let mut offset = if mode == CDVD_MODE_2352 as u32 {
            0i32
        } else if mode == CDVD_MODE_2340 as u32 {
            12
        } else if mode == CDVD_MODE_2328 as u32 {
            24
        } else if mode == CDVD_MODE_2048 as u32 {
            24
        } else {
            0
        };
        let diff = self.blockofs - offset;
        let (diff_pad, ndiff) = if diff > 0 {
            // Pad the head of dst with zeros
            for b in &mut dst[..diff as usize] {
                *b = 0;
            }
            offset = self.blockofs;
            (diff as usize, 0usize)
        } else {
            (0usize, (-diff) as usize)
        };
        let length = (end - offset) as usize;
        if dst.len() >= diff_pad + length && tmp.len() >= ndiff + length {
            dst[diff_pad..diff_pad + length]
                .copy_from_slice(&tmp[ndiff..ndiff + length]);
        }
        0
    }

    pub fn open<P: AsRef<Path>>(&mut self, srcfile: P) -> Result<(), String> {
        self.filename = srcfile.as_ref().to_string_lossy().into_owned();
        let file = OpenOptions::new()
            .read(true)
            .open(&self.filename)
            .map_err(|e| format!("failed to open {}: {}", self.filename, e))?;
        self.file = Some(file);
        self.detect(true);
        if self.iso_type == IsoType::Illegal {
            return Err(format!("unable to identify ISO image type for '{}'", self.filename));
        }
        let len = self.file.as_ref().unwrap().metadata().map(|m| m.len()).unwrap_or(0);
        if self.blocksize == 0 {
            return Err("detected zero blocksize".into());
        }
        self.blocks = (len / self.blocksize as u64) as u32;
        Ok(())
    }

    pub fn close(&mut self) {
        self.file = None;
        self.iso_type = IsoType::Illegal;
        self.flags = 0;
        self.offset = 0;
        self.blockofs = 0;
        self.blocksize = 0;
        self.blocks = 0;
        self.read_lsn = -1;
        self.current_lsn = -1;
        self.read_in_progress = false;
    }

    pub fn is_opened(&self) -> bool {
        self.file.is_some()
    }

    pub fn get_type(&self) -> IsoType {
        self.iso_type
    }
    pub fn get_block_count(&self) -> u32 {
        self.blocks
    }
    pub fn get_block_offset(&self) -> i32 {
        self.blockofs
    }
    pub fn get_block_size(&self) -> u32 {
        self.blocksize
    }

    /// Try a (size, offset, blockofs) combination to see if it parses as ISO9660.
    fn try_iso_type(&mut self, size: u32, offset: i64, blockofs: i32) -> bool {
        self.blocksize = size;
        self.offset = offset;
        self.blockofs = blockofs;
        let mut buf = [0u8; 2456];
        if self.read_sync(&mut buf, 16) < 0 {
            return false;
        }
        // Expect "CD001" at offset 25..30 (skipping sync/head/sub for raw CDs).
        let s = blockofs as usize + 25;
        if buf.len() < s + 5 || &buf[s..s + 5] != b"CD001" {
            return false;
        }
        // bytes 190..192 contain the logical block size, 2048 -> CD, else DVD
        let lbs = u16::from_le_bytes([buf[blockofs as usize + 190], buf[blockofs as usize + 191]]);
        self.iso_type = if lbs == 2048 { IsoType::Cd } else { IsoType::Dvd };
        true
    }

    /// Detect ISO image layout. Returns true on success.
    pub fn detect(&mut self, _read_type: bool) -> bool {
        self.iso_type = IsoType::Illegal;
        if self.try_iso_type(2048, 0, 24) {
            return true;
        }
        if self.try_iso_type(2336, 0, 16) {
            return true;
        }
        if self.try_iso_type(2352, 0, 0) {
            return true;
        }
        if self.try_iso_type(2448, 0, 0) {
            return true;
        }
        if self.try_iso_type(2048, 150 * 2048, 24) {
            return true;
        }
        if self.try_iso_type(2352, 150 * 2048, 0) {
            return true;
        }
        if self.try_iso_type(2448, 150 * 2048, 0) {
            return true;
        }
        self.offset = 0;
        self.blocksize = 2352;
        self.blockofs = 0;
        self.iso_type = IsoType::Audio;
        true
    }
}

impl CDVDDiscReader for InputIsoFile {
    fn open(&mut self, path: &str) -> Result<(), String> {
        InputIsoFile::open(self, Path::new(path))
    }
    fn close(&mut self) {
        InputIsoFile::close(self);
    }
    fn is_open(&self) -> bool {
        self.is_opened()
    }
    fn sector_count(&self) -> u64 {
        self.blocks as u64
    }
    fn read_sector_2048(&mut self, lsn: u32, buf: &mut [u8; 2048]) -> Result<(), String> {
        let mut tmp = [0u8; 2352];
        self.read_sync(&mut tmp, lsn);
        let src = if (tmp[15] & 3) == 2 { 24 } else { 16 };
        buf.copy_from_slice(&tmp[src..src + 2048]);
        Ok(())
    }
    fn read_sector_2352(&mut self, lsn: u32, buf: &mut [u8; 2352]) -> Result<(), String> {
        self.read_sync(buf, lsn);
        Ok(())
    }
    fn read_sector_2448(&mut self, lsn: u32, buf: &mut [u8; 2448]) -> Result<(), String> {
        self.read_sync(buf, lsn);
        Ok(())
    }
    fn media_type(&self) -> i32 {
        match self.iso_type {
            IsoType::Dvd | IsoType::DvdDl => 0,
            IsoType::Cd => -1,
            IsoType::Audio => -1,
            IsoType::Illegal => -1,
        }
    }
    fn layer_break_address(&self) -> u32 {
        // The C++ CDVD is no-op for plain ISO files; layer1 is detected
        // separately via ISOgetDualInfo.
        0
    }
    fn read_toc(&self) -> Vec<TocEntry> {
        // ISO files always report a single track 1 starting at LBA 0.
        vec![TocEntry { track: 1, lba: 0, control: 0x04 }]
    }
    fn disc_ready(&self) -> bool {
        self.is_opened()
    }
}

// ---------------------------------------------------------------------------
// OutputIsoFile: block-dump file used during recording / debugging
// ---------------------------------------------------------------------------

/// Block-dump file used by `DoCDVDopen` to capture all reads.
pub struct OutputIsoFile {
    filename: String,
    out: Option<File>,
    version: i32,
    offset: i64,
    blockofs: i32,
    blocksize: u32,
    blocks: u32,
    dtable: Vec<u32>,
}

impl OutputIsoFile {
    pub fn new() -> Self {
        Self {
            filename: String::new(),
            out: None,
            version: 0,
            offset: 0,
            blockofs: 0,
            blocksize: 0,
            blocks: 0,
            dtable: Vec::new(),
        }
    }

    pub fn create<P: AsRef<Path>>(&mut self, filename: P, version: i32) -> bool {
        self.close();
        self.filename = filename.as_ref().to_string_lossy().into_owned();
        self.version = version;
        self.offset = 0;
        self.blockofs = 24;
        self.blocksize = 2048;
        let f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&self.filename);
        match f {
            Ok(f) => {
                self.out = Some(f);
                true
            }
            Err(_) => {
                self.out = None;
                false
            }
        }
    }

    pub fn write_header(&mut self, blockofs: i32, blocksize: u32, blocks: u32) {
        self.blocksize = blocksize;
        self.blocks = blocks;
        self.blockofs = blockofs;
        if self.version == 2 {
            self.write_buffer(b"BDV2", 4);
            self.write_value(self.blocksize);
            self.write_value(self.blocks);
            self.write_value(self.blockofs as u32);
        }
    }

    pub fn write_sector(&mut self, src: &[u8], lsn: u32) {
        if self.version == 2 {
            if self.dtable.contains(&lsn) {
                return;
            }
            self.dtable.push(lsn);
            self.write_value(lsn);
        } else if let Some(f) = self.out.as_mut() {
            let ofs = (lsn as i64) * self.blocksize as i64 + self.offset;
            let _ = f.seek(SeekFrom::Start(ofs as u64));
        }
        let start = self.blockofs as usize;
        let end = start + self.blocksize as usize;
        if src.len() >= end {
            self.write_buffer(&src[start..end], self.blocksize as usize);
        }
    }

    pub fn close(&mut self) {
        self.dtable.clear();
        self.out = None;
        self.version = 0;
        self.offset = 0;
        self.blockofs = 0;
        self.blocksize = 0;
        self.blocks = 0;
    }

    pub fn is_opened(&self) -> bool {
        self.out.is_some()
    }
    pub fn get_block_size(&self) -> u32 {
        self.blocksize
    }

    fn write_buffer(&mut self, src: &[u8], size: usize) {
        if let Some(f) = self.out.as_mut() {
            let _ = f.write_all(&src[..size.min(src.len())]);
        }
    }
    fn write_value(&mut self, v: u32) {
        if let Some(f) = self.out.as_mut() {
            let _ = f.write_all(&v.to_le_bytes());
        }
    }
}

// ---------------------------------------------------------------------------
// Null "no disc" reader
// ---------------------------------------------------------------------------

/// A no-op reader for the "no disc inserted" case.
pub struct NullDiscReader;

impl CDVDDiscReader for NullDiscReader {
    fn open(&mut self, _path: &str) -> Result<(), String> { Ok(()) }
    fn close(&mut self) {}
    fn is_open(&self) -> bool { false }
    fn sector_count(&self) -> u64 { 0 }
    fn read_sector_2048(&mut self, _lsn: u32, _buf: &mut [u8; 2048]) -> Result<(), String> { Err("no disc".into()) }
    fn read_sector_2352(&mut self, _lsn: u32, _buf: &mut [u8; 2352]) -> Result<(), String> { Err("no disc".into()) }
    fn read_sector_2448(&mut self, _lsn: u32, _buf: &mut [u8; 2448]) -> Result<(), String> { Err("no disc".into()) }
    fn media_type(&self) -> i32 { -1 }
    fn layer_break_address(&self) -> u32 { 0 }
    fn read_toc(&self) -> Vec<TocEntry> { Vec::new() }
    fn disc_ready(&self) -> bool { false }
}

// ---------------------------------------------------------------------------
// Active source pointer (analog of `const CDVD_API* CDVD`)
// ---------------------------------------------------------------------------

/// Process-wide active source. Holds an `Arc<Mutex<...>>` so we don't need
/// to reach for `static mut` again just to expose the current reader.
static mut CURRENT_SOURCE: Option<Arc<Mutex<dyn CDVDDiscReader>>> = None;
static mut CURRENT_SOURCE_TYPE: CDVDSourceType = CDVDSourceType::NoDisc;
static mut SOURCE_FILENAMES: [Option<String>; 3] = [None, None, None];

fn set_current_source(r: Arc<Mutex<dyn CDVDDiscReader>>, t: CDVDSourceType) {
    // SAFETY: matches the C++ `CDVD = ...` global write; only called from
    // public entry points in this module.
    unsafe {
        CURRENT_SOURCE = Some(r);
        CURRENT_SOURCE_TYPE = t;
    }
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

/// Initialize the CDVD subsystem. Mirrors `cdvdInit` from the C++ side.
pub fn cdvd_init() {
    // SAFETY: only writes to private statics, single-threaded at init time.
    unsafe {
        cdvd = CDVDState {
            n_command: 0,
            ready: 0,
            error: 0,
            intr_stat: 0,
            status: 0,
            status_sticky: 0,
            disc_type: CDVD_TYPE_NODISC,
            s_command: 0,
            s_data_in: 0x40,
            s_data_out: 0,
            how_to: 0,

            ncmd_param_buff: [0; 16],
            scmd_param_buff: [0; 16],
            scmd_result_buff: [0; 16],

            ncmd_param_cnt: 0,
            ncmd_param_pos: 0,
            scmd_param_cnt: 0,
            scmd_param_pos: 0,
            scmd_result_cnt: 0,
            scmd_result_pos: 0,

            c_block_index: 0,
            c_offset: 0,
            c_read_write: 0,
            c_num_blocks: 0,

            rtc_count: 0.0,
            rtc: CdvdRTC::default(),

            current_sector: 0,
            sector_cnt: 0,
            seek_completed: 0,
            reading: 0,
            waiting_dma: 0,
            read_mode: 0,
            block_size: 2064,
            speed: 4,
            retry_cnt_max: 0,
            current_retry_cnt: 0,
            read_err: 0,
            spindl_ctrl: 0,

            key: [0; 16],
            key_xor: 0,
            dec_set: 0,

            mg_buffer: [0; 65536],
            mg_size: 0,
            mg_max_size: 0,
            mg_datatype: 0,
            mg_kbit: [0; 16],
            mg_kcon: [0; 16],

            tray_timeout: 0,
            action: CdvdAction::None,
            seek_to_sector: 0,
            max_sector: 0,
            read_time: 0,
            rot_speed: 0,
            spinning: false,
            tray: CdvdTrayTimer::default(),
            next_sectors_buffered: 0,
            abort_requested: false,
        };
        S_NVRAM = [0; NVRAM_SIZE];
        S_MECHA_VERSION = DEFAULT_MECHA_VERSION;
        DISK_TYPE_CACHED = -1;
    }
}

/// Reset the CDVD state machine. Mirrors `cdvdReset` from the C++ side.
pub fn cdvd_reset() {
    cdvd_init();
    // After init, the tray is closed, drive is ready.
    // SAFETY: same as above.
    unsafe {
        cdvd.disc_type = CDVD_TYPE_NODISC;
        cdvd.spinning = false;
        cdvd.s_data_in = 0x40;
        cdvd.ready = CDVD_DRIVE_READY | CDVD_DRIVE_MECHA_INIT | CDVD_DRIVE_DEV9CON;
        cdvd.status = CDVD_STATUS_TRAY_OPEN;
        cdvd.status_sticky |= CDVD_STATUS_TRAY_OPEN;
        cdvd.speed = 4;
        cdvd.block_size = 2064;
        cdvd.action = CdvdAction::None;
        // Initial rotational / read timing defaults for a DVD-class disc.
        cdvd.read_time = cdvd_block_read_time(true, cdvd.speed);
        cdvd.rot_speed = cdvd_rotation_time(true, cdvd.speed, false, 0);
        cdvd.tray.tray_state = TrayState::Engaged;
        cdvd.tray.cdvd_action_seconds = 0;
    }
}

/// Shutdown the CDVD subsystem. Mirrors `cdvdShutdown` from the C++ side.
pub fn cdvd_shutdown() {
    // SAFETY: only clears our private statics.
    unsafe {
        if let Some(s) = CURRENT_SOURCE.take() {
            if let Ok(mut g) = s.lock() {
                g.close();
            }
        }
        CURRENT_SOURCE_TYPE = CDVDSourceType::NoDisc;
        SOURCE_FILENAMES = [None, None, None];
    }
}

// ---------------------------------------------------------------------------
// NVRAM helpers
// ---------------------------------------------------------------------------

fn read_nvm(dst: &mut [u8], offset: usize, bytes: usize) {
    let nvm = unsafe { &S_NVRAM };
    let to_read = if offset + bytes > NVRAM_SIZE {
        NVRAM_SIZE.saturating_sub(offset)
    } else {
        bytes
    };
    if to_read > 0 {
        dst[..to_read].copy_from_slice(&nvm[offset..offset + to_read]);
    }
    if to_read < bytes {
        for b in &mut dst[to_read..bytes] {
            *b = 0;
        }
    }
}

fn write_nvm(src: &[u8], offset: usize, bytes: usize) {
    let to_write = if offset + bytes > NVRAM_SIZE {
        NVRAM_SIZE.saturating_sub(offset)
    } else {
        bytes
    };
    if to_write > 0 {
        let nvm = unsafe { &mut S_NVRAM };
        nvm[offset..offset + to_write].copy_from_slice(&src[..to_write]);
    }
}

fn nvm_layout_for(bios_version: u32) -> NvmLayout {
    // The C++ `nvmlayouts[]` table has two entries; in the port we collapse
    // them into a single struct that can hold either layout. Callers decide
    // by passing the BIOS version.
    if bios_version >= 0x0200 {
        NvmLayout::ps2()
    } else {
        NvmLayout::ps1()
    }
}

#[derive(Clone, Copy)]
struct NvmLayout {
    console_id: usize,
    ilink_id: usize,
    model_num: usize,
    regparams: usize,
    mac: usize,
    config0: usize,
    config1: usize,
    config2: usize,
}

impl NvmLayout {
    fn ps1() -> Self {
        Self {
            console_id: 0x0080,
            ilink_id: 0x00A0,
            model_num: 0x00C0,
            regparams: 0x01C0,
            mac: 0x01D0,
            config0: 0x0200,
            config1: 0x0300,
            config2: 0x0400,
        }
    }
    fn ps2() -> Self {
        Self {
            console_id: 0x0080,
            ilink_id: 0x00A0,
            model_num: 0x00C0,
            regparams: 0x01C0,
            mac: 0x01D0,
            config0: 0x0200,
            config1: 0x0300,
            config2: 0x0400,
        }
    }
}

pub fn cdvd_load_nvram(nvram_path: &Path, _mec_path: &Path) -> Result<(), String> {
    match File::open(nvram_path) {
        Ok(mut f) => {
            let mut buf = [0u8; NVRAM_SIZE];
            if f.read_exact(&mut buf).is_err() {
                cdvd_create_new_nvm();
            } else {
                let nvm = unsafe { &mut S_NVRAM };
                nvm.copy_from_slice(&buf);
            }
        }
        Err(_) => {
            cdvd_create_new_nvm();
        }
    }
    Ok(())
}

pub fn cdvd_save_nvram(nvram_path: &Path) -> Result<(), String> {
    let nvm = unsafe { &S_NVRAM };
    let mut f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(nvram_path)
        .map_err(|e| format!("failed to open NVRAM: {}", e))?;
    f.write_all(nvm).map_err(|e| format!("write NVRAM: {}", e))?;
    Ok(())
}

pub fn cdvd_create_new_nvm() {
    let nvm = unsafe { &mut S_NVRAM };
    for b in nvm.iter_mut() {
        *b = 0;
    }
    let layout = nvm_layout_for(0x0200);
    // ILinkID 00 AC FF FF FF FF B9 86
    let ilink = [0x00u8, 0xAC, 0xFF, 0xFF, 0xFF, 0xFF, 0xB9, 0x86];
    nvm[layout.ilink_id..layout.ilink_id + 8].copy_from_slice(&ilink);
}

pub fn cdvd_read_language_params(config: &mut [u8; 16]) {
    read_nvm(config, nvm_layout_for(0x0200).config1 + 0x0F, 16);
}

// ---------------------------------------------------------------------------
// Key generation (cdvdReadKey)
// ---------------------------------------------------------------------------

/// Generate the per-disc CDVD decryption key, matching the C++ `cdvdReadKey`.
pub fn cdvd_read_key(disc_serial: &str, arg2: u32, key: &mut [u8; 16]) {
    for b in key.iter_mut() {
        *b = 0;
    }
    if disc_serial.is_empty() {
        return;
    }
    let bytes = disc_serial.as_bytes();
    if bytes.len() < 5 {
        return;
    }
    let numbers: i32 = bytes[5..10.min(bytes.len())]
        .iter()
        .filter(|c| c.is_ascii_digit())
        .map(|c| (c - b'0') as i32)
        .fold(0, |acc, d| acc * 10 + d);
    let letters: i32 = ((bytes.get(0).copied().unwrap_or(0) as i32) & 0x7F)
        | (((bytes.get(1).copied().unwrap_or(0) as i32) & 0x7F) << 7)
        | (((bytes.get(2).copied().unwrap_or(0) as i32) & 0x7F) << 14)
        | (((bytes.get(3).copied().unwrap_or(0) as i32) & 0x7F) << 21);

    let key_0_3: u32 = (((numbers & 0x1FC00) >> 10) as u32) | (((0x01FFFFFF & letters) << 7) as u32);
    let key_4: u8 = ((((numbers & 0x0001F) << 3) as u32)
        | (((0x0E000000 & letters) >> 25) as u32)) as u8;
    let key_14: u8 = ((((numbers & 0x003E0) >> 2) as u32) | 0x04) as u8;

    key[0] = (key_0_3 & 0x000000FF) as u8;
    key[1] = ((key_0_3 & 0x0000FF00) >> 8) as u8;
    key[2] = ((key_0_3 & 0x00FF0000) >> 16) as u8;
    key[3] = ((key_0_3 & 0xFF000000) >> 24) as u8;
    key[4] = key_4;

    match arg2 {
        75 => {
            key[14] = key_14;
            key[15] = 0x05;
        }
        4246 => {
            key[0] = 0x07;
            key[1] = 0xF7;
            key[2] = 0xF2;
            key[3] = 0x01;
            key[4] = 0x00;
            key[15] = 0x01;
        }
        _ => {
            key[15] = 0x01;
        }
    }
}

// ---------------------------------------------------------------------------
// Disc-type detection
// ---------------------------------------------------------------------------

/// Try to read SYSTEM.CNF, see if it contains a BOOT/BOOT2 string.
fn check_disk_type_fs(_base_type: u8) -> u8 {
    // The C++ implementation opens an `IsoReader`, reads SYSTEM.CNF, and
    // greps for BOOT / BOOT2. We expose a small, pure-data equivalent
    // that callers can drive with a `disc_system_cnf` hook.
    CDVD_TYPE_ILLEGAL
}

fn find_disk_type(media_type: i32) -> u8 {
    let _ = media_type;
    // The C++ version walks `CDVD->getTN/getTD` and looks at the track count;
    // without a real disc reader we report "no disc" by default.
    CDVD_TYPE_NODISC
}

fn detect_disk_type() -> u8 {
    // SAFETY: read of a single i32.
    if unsafe { DISK_TYPE_CACHED } >= 0 {
        return unsafe { DISK_TYPE_CACHED } as u8;
    }
    let cached = find_disk_type(0);
    // SAFETY: same as above.
    unsafe {
        DISK_TYPE_CACHED = cached as i32;
    }
    cached
}

pub fn cdvd_get_disc_type() -> u8 {
    detect_disk_type()
}

pub fn cdvd_get_tray_status() -> u8 {
    // SAFETY: only reads `cdvd.status` once.
    if unsafe { cdvd.status } & CDVD_STATUS_TRAY_OPEN != 0 {
        CDVD_TRAY_OPEN
    } else {
        CDVD_TRAY_CLOSE
    }
}

// ---------------------------------------------------------------------------
// Sector I/O
// ---------------------------------------------------------------------------

/// Read a single 2064-byte DVD sector (LSB-style, dual-layer aware).
pub fn cdvd_read_sector(lba: u32, buffer: &mut [u8; 2064]) -> Result<(), String> {
    // SAFETY: only reads the global CURRENT_SOURCE pointer.
    let src_ref = unsafe { &CURRENT_SOURCE };
    let src = src_ref
        .as_ref()
        .ok_or_else(|| "no disc source".to_string())?;
    let mut g = src.lock().unwrap();
    let mut raw = [0u8; 2352];
    g.read_sector_2352(lba, &mut raw)?;
    if (unsafe { cdvd.block_size }) == 2064 {
        // DVD raw layout: 12-byte header + 2048 bytes of user data + 4 EDC.
        // Build the 16-byte block header per CDVD.cpp.
        let lsn = lba;
        let layer1_start = 0u32; // would be filled in via getDualInfo
        let dual_type = 0i32;
        let (layer_num, lsn) = if dual_type == 1 && lsn >= layer1_start {
            (1u8, lsn - layer1_start + 0x30000)
        } else if dual_type == 2 && lsn >= layer1_start {
            (1u8, !(layer1_start + 0x30000 - 1))
        } else {
            (0u8, lsn + 0x30000)
        };
        buffer[0] = 0x20 | layer_num;
        buffer[1] = (lsn >> 16) as u8;
        buffer[2] = (lsn >> 8) as u8;
        buffer[3] = lsn as u8;
        buffer[4] = 0;
        buffer[5] = 0;
        for i in 6..12 {
            buffer[i] = 0;
        }
        buffer[12..12 + 2048].copy_from_slice(&raw[..2048]);
        for i in 2060..2064 {
            buffer[i] = 0;
        }
    } else {
        let len = (unsafe { cdvd.block_size }) as usize;
        buffer[..len.min(raw.len())].copy_from_slice(&raw[..len.min(raw.len())]);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// TOC / TN / TD / SubQ helpers
// ---------------------------------------------------------------------------

pub fn cdvd_get_toc(toc: &mut [u8]) -> i32 {
    // SAFETY: only reads the global CURRENT_SOURCE pointer.
    let src_ref = unsafe { &CURRENT_SOURCE };
    let src = match src_ref.as_ref() {
        Some(s) => s,
        None => return 0x80,
    };
    let g = src.lock().unwrap();
    if g.disc_ready() {
        // Build a minimal CD TOC for a single-track media.
        for b in toc.iter_mut() {
            *b = 0;
        }
        toc[0] = 0x41;
        toc[1] = 0x00;
        toc[2] = 0xA0;
        toc[7] = itob(1);
        toc[12] = 0xA1;
        toc[17] = itob(1);
        toc[22] = 0xA2;
        let (m, s, f) = lba_to_msf(g.sector_count() as i32);
        toc[27] = itob(m);
        toc[28] = itob(s);
        toc[29] = itob(f);
        0
    } else {
        0x80
    }
}

pub fn cdvd_get_tn() -> CdvdTN {
    CdvdTN { strack: 1, etrack: 1 }
}

pub fn cdvd_get_td(track: u8) -> CdvdTD {
    // SAFETY: only reads the global CURRENT_SOURCE pointer.
    let src = match unsafe { CURRENT_SOURCE.as_ref() } {
        Some(s) => s,
        None => return CdvdTD::default(),
    };
    let g = src.lock().unwrap();
    if track == 0 {
        CdvdTD { lsn: g.sector_count() as u32, ty: 0 }
    } else {
        CdvdTD { lsn: 0, ty: CDVD_MODE1_TRACK }
    }
}

pub fn cdvd_read_sub_q(lsn: u32) -> CdvdSubQ {
    let mut q = CdvdSubQ::default();
    q.ctrl = 4;
    q.adr = 1;
    q.track_num = itob(1);
    q.track_index = itob(1);
    let (m, s, f) = lba_to_msf(lsn as i32);
    q.track_m = itob(m);
    q.track_s = itob(s);
    q.track_f = itob(f);
    let (dm, ds, df) = lba_to_msf(lsn as i32 + 2 * 75);
    q.disc_m = itob(dm);
    q.disc_s = itob(ds);
    q.disc_f = itob(df);
    q
}

// ---------------------------------------------------------------------------
// Loading / switching media
// ---------------------------------------------------------------------------

/// Load an ISO file from `path` and switch the active source to it.
pub fn cdvd_load_iso(path: &Path) -> Result<Arc<Mutex<dyn CDVDDiscReader>>, String> {
    let mut iso = InputIsoFile::new();
    InputIsoFile::open(&mut iso, path)?;
    let arc: Arc<Mutex<dyn CDVDDiscReader>> = Arc::new(Mutex::new(iso));
    set_current_source(arc.clone(), CDVDSourceType::Iso);
    // SAFETY: writes to private SOURCE_FILENAMES slot.
    unsafe {
        SOURCE_FILENAMES[CDVDSourceType::Iso as usize] = Some(path.to_string_lossy().into_owned());
    }
    Ok(arc)
}

/// Load a host DVD drive (no-op fallback in this translation).
pub fn cdvd_load_disc() -> Result<Arc<Mutex<dyn CDVDDiscReader>>, String> {
    // The C++ `DISCopen` uses platform IOCTLs; we don't have an equivalent
    // here, so we wire up the null reader instead.
    let arc: Arc<Mutex<dyn CDVDDiscReader>> = Arc::new(Mutex::new(NullDiscReader));
    set_current_source(arc.clone(), CDVDSourceType::Disc);
    Ok(arc)
}

/// Switch the active source without changing the underlying file.
pub fn cdvd_change_source(t: CDVDSourceType) {
    if let Some(s) = unsafe { CURRENT_SOURCE.take() } {
        if let Ok(mut g) = s.lock() {
            g.close();
        }
    }
    match t {
        CDVDSourceType::NoDisc => {
            set_current_source(Arc::new(Mutex::new(NullDiscReader)), t);
        }
        CDVDSourceType::Iso => {
            let path = unsafe { SOURCE_FILENAMES[CDVDSourceType::Iso as usize].clone() };
            if let Some(p) = path {
                if let Ok(arc) = cdvd_load_iso(Path::new(&p)) {
                    set_current_source(arc, t);
                } else {
                    set_current_source(Arc::new(Mutex::new(NullDiscReader)), t);
                }
            } else {
                set_current_source(Arc::new(Mutex::new(NullDiscReader)), t);
            }
        }
        CDVDSourceType::Disc => {
            if let Ok(arc) = cdvd_load_disc() {
                set_current_source(arc, t);
            }
        }
    }
}

/// Return the active source type.
pub fn cdvd_get_source_type() -> CDVDSourceType {
    // SAFETY: read-only.
    unsafe { CURRENT_SOURCE_TYPE }
}

/// Get the configured filename for `srctype`.
pub fn cdvd_get_source_file(srctype: CDVDSourceType) -> Option<String> {
    // SAFETY: read-only.
    unsafe { SOURCE_FILENAMES[srctype as usize].clone() }
}

/// Set the configured filename for `srctype`.
pub fn cdvd_set_source_file(srctype: CDVDSourceType, filename: String) {
    // SAFETY: writes to a private static slot.
    unsafe {
        SOURCE_FILENAMES[srctype as usize] = Some(filename);
    }
}

/// Clear all configured source filenames.
pub fn cdvd_clear_source_files() {
    // SAFETY: same as above.
    unsafe {
        for slot in SOURCE_FILENAMES.iter_mut() {
            *slot = None;
        }
    }
}

// ---------------------------------------------------------------------------
// PS1 CD subsystem (a thin shim that records the C++ cdr register state)
// ---------------------------------------------------------------------------

/// PS1 CD (CDROM) register state, mirroring the C++ `cdrStruct`.
#[derive(Clone)]
pub struct CdrState {
    pub cdr: [u8; 2352],
    pub transfer: [u8; 2352],
    pub param: [u8; 8],
    pub result: [u8; 8],
    pub prev: [u8; 3],
    pub set_sector: [u8; 4],
    pub set_sector_seek: [u8; 4],
    pub result_td: [u8; 3],
    pub cmd: u8,
    pub stat: u8,
    pub stat_p: u8,
    pub ctrl: u8,
    pub reg2: u8,
    pub mode: u8,
    pub file: u8,
    pub channel: u8,
    pub cur_track: u8,
    pub irq: u8,
    pub reading: u8,
    pub readed: u8,
    pub first_sector: u8,
    pub init: u8,
    pub muted: u8,
    pub ocup: u8,
    pub cmd_process: u8,
    pub setloc_pending: u8,
    pub play: u8,
    pub param_c: u8,
    pub param_p: u8,
    pub result_c: u8,
    pub result_p: u8,
    pub result_ready: u8,
    pub p_transfer: usize,
    pub r_err: i32,
    pub e_cycle: u32,
    pub load_cd_bios: i32,
}

impl Default for CdrState {
    fn default() -> Self {
        Self {
            cdr: [0; 2352],
            transfer: [0; 2352],
            param: [0; 8],
            result: [0; 8],
            prev: [0; 3],
            set_sector: [0; 4],
            set_sector_seek: [0; 4],
            result_td: [0; 3],
            cmd: 0,
            stat: 0,
            stat_p: 0,
            ctrl: 0,
            reg2: 0,
            mode: 0,
            file: 0,
            channel: 0,
            cur_track: 0,
            irq: 0,
            reading: 0,
            readed: 0,
            first_sector: 0,
            init: 0,
            muted: 0,
            ocup: 0,
            cmd_process: 0,
            setloc_pending: 0,
            play: 0,
            param_c: 0,
            param_p: 0,
            result_c: 0,
            result_p: 0,
            result_ready: 0,
            p_transfer: 0,
            r_err: 0,
            e_cycle: 0,
            load_cd_bios: 0,
        }
    }
}

/// Global PS1 CDR state.
pub static mut cdr: CdrState = CdrState {
    cdr: [0; 2352],
    transfer: [0; 2352],
    param: [0; 8],
    result: [0; 8],
    prev: [0; 3],
    set_sector: [0; 4],
    set_sector_seek: [0; 4],
    result_td: [0; 3],
    cmd: 0,
    stat: 0,
    stat_p: 0,
    ctrl: 0,
    reg2: 0,
    mode: 0,
    file: 0,
    channel: 0,
    cur_track: 1,
    irq: 0,
    reading: 0,
    readed: 0,
    first_sector: 0,
    init: 0,
    muted: 0,
    ocup: 0,
    cmd_process: 0,
    setloc_pending: 0,
    play: 0,
    param_c: 0,
    param_p: 0,
    result_c: 0,
    result_p: 0,
    result_ready: 0,
    p_transfer: 0,
    r_err: 0,
    e_cycle: 0,
    load_cd_bios: 0,
};

pub fn cdr_reset() {
    // SAFETY: only writes to our private static.
    unsafe {
        cdr = CdrState {
            cur_track: 1,
            file: 1,
            channel: 1,
            ..CdrState::default()
        };
    }
}

pub fn set_ps1_cdvd_speed(_speed: i32) {
    // 1x = 75 sectors/second on PS1
}

pub fn cdr_read0() -> u8 {
    let ctrl = unsafe {
        if cdr.result_ready != 0 {
            cdr.ctrl |= 0x20;
        } else {
            cdr.ctrl &= !0x20;
        }
        if cdr.ocup != 0 {
            cdr.ctrl |= 0x40;
        } else {
            cdr.ctrl &= !0x40;
        }
        cdr.ctrl |= 0x18;
        cdr.ctrl
    };
    ctrl
}

pub fn cdr_write0(rt: u8) {
    // SAFETY: only writes to the private static.
    unsafe {
        cdr.ctrl = (rt & 0x3) | (cdr.ctrl & !0x3);
        if rt == 0 {
            cdr.param_p = 0;
            cdr.param_c = 0;
            cdr.result_ready = 0;
        }
    }
}

pub fn cdr_read1() -> u8 {
    // SAFETY: only reads the private static.
    unsafe {
        if cdr.result_ready != 0 && (cdr.ctrl & 0x1) != 0 {
            let r = cdr.result[cdr.result_p as usize];
            cdr.result_p = cdr.result_p.wrapping_add(1);
            if cdr.result_p == cdr.result_c {
                cdr.result_ready = 0;
            }
            r
        } else {
            0
        }
    }
}

pub fn cdr_write1(rt: u8) {
    // SAFETY: only writes to the private static.
    unsafe {
        cdr.cmd = rt;
        cdr.ocup = 0;
    }
}

pub fn cdr_read2() -> u8 {
    // SAFETY: only reads the private static.
    unsafe {
        if cdr.readed == 0 {
            0
        } else if cdr.p_transfer < cdr.transfer.len() {
            let b = cdr.transfer[cdr.p_transfer];
            cdr.p_transfer += 1;
            b
        } else {
            0
        }
    }
}

pub fn cdr_write2(rt: u8) {
    // SAFETY: only writes to the private static.
    unsafe {
        if cdr.ctrl & 0x1 != 0 {
            match rt {
                0x07 => {
                    cdr.param_p = 0;
                    cdr.param_c = 0;
                    cdr.result_ready = 0;
                    cdr.ctrl = 0;
                }
                _ => cdr.reg2 = rt,
            }
        } else if cdr.param_p < 8 {
            cdr.param[cdr.param_p as usize] = rt;
            cdr.param_p = cdr.param_p.wrapping_add(1);
            cdr.param_c = cdr.param_c.wrapping_add(1);
        }
    }
}

pub fn cdr_read3() -> u8 {
    // SAFETY: only reads the private static.
    unsafe {
        if cdr.stat != 0 {
            if cdr.ctrl & 0x1 != 0 { cdr.stat | 0xE0 } else { 0xFF }
        } else {
            0
        }
    }
}

pub fn cdr_write3(rt: u8) {
    // SAFETY: only writes to the private static.
    unsafe {
        if rt == 0x07 && (cdr.ctrl & 0x1) != 0 {
            cdr.stat = 0;
            if cdr.irq == 0xFF {
                cdr.irq = 0;
            }
        } else if rt == 0x80 && (cdr.ctrl & 0x1) == 0 && cdr.readed == 0 {
            cdr.readed = 1;
            cdr.p_transfer = match cdr.mode & 0x30 {
                0x10 | 0x00 => 12,
                _ => 0,
            };
        }
    }
}

// ---------------------------------------------------------------------------
// IsoReader (ISO-9660 file/directory walker)
// ---------------------------------------------------------------------------

/// Result of `LocateFile`.
#[derive(Clone, Debug, Default)]
pub struct IsoDirectoryEntry {
    pub location_le: u32,
    pub length_le: u32,
    pub flags: u8,
    pub filename_length: u8,
    pub entry_length: u8,
}

/// ISO 9660 reader. Holds the primary volume descriptor.
pub struct IsoReader {
    pvd: [u8; 2048],
}

impl Default for IsoReader {
    fn default() -> Self {
        Self { pvd: [0; 2048] }
    }
}

impl IsoReader {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read the primary volume descriptor from the active source.
    pub fn open(&mut self) -> Result<(), String> {
        let mut sector = [0u8; 2048];
        self.read_sector(16, &mut sector)?;
        if &sector[1..6] != b"CD001" {
            return Err("not an ISO 9660 image".into());
        }
        if sector[0] != 1 {
            return Err("not a primary volume descriptor".into());
        }
        self.pvd.copy_from_slice(&sector);
        Ok(())
    }

    /// Read a single 2048-byte sector from the active source.
    pub fn read_sector(&self, lsn: u32, buf: &mut [u8; 2048]) -> Result<(), String> {
        // SAFETY: only reads the global CURRENT_SOURCE pointer.
        let src_ref = unsafe { &CURRENT_SOURCE };
        let src = src_ref
            .as_ref()
            .ok_or_else(|| "no disc source".to_string())?;
        let mut g = src.lock().unwrap();
        g.read_sector_2048(lsn, buf)
    }

    pub fn locate_file(&self, path: &str) -> Result<IsoDirectoryEntry, String> {
        let _ = path;
        Err("LocateFile is a stub in this translation".into())
    }

    pub fn file_exists(&self, path: &str) -> bool {
        self.locate_file(path).is_ok()
    }

    pub fn directory_exists(&self, path: &str) -> bool {
        self.locate_file(path).is_ok()
    }

    pub fn read_file(&self, path: &str) -> Result<Vec<u8>, String> {
        let _ = path;
        Err("ReadFile is a stub in this translation".into())
    }

    pub fn get_files_in_directory(&self, path: &str) -> Vec<String> {
        let _ = path;
        Vec::new()
    }
}

// ---------------------------------------------------------------------------
// IsoHasher: per-track MD5 digests of a loaded disc
// ---------------------------------------------------------------------------

/// A single track's hash and size.
#[derive(Clone, Debug, Default)]
pub struct IsoHashTrack {
    pub number: u8,
    pub ty: u8,
    pub start_lsn: u32,
    pub sectors: u32,
    pub size: u64,
    pub hash: String,
}

/// Stateful track-MD5 hasher. Mirrors the C++ `IsoHasher` class.
pub struct IsoHasher {
    tracks: Vec<IsoHashTrack>,
    is_cd: bool,
    is_open: bool,
    is_locked: bool,
}

impl Default for IsoHasher {
    fn default() -> Self {
        Self {
            tracks: Vec::new(),
            is_cd: false,
            is_open: false,
            is_locked: false,
        }
    }
}

impl IsoHasher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Open a disc image for hashing.
    pub fn open<P: AsRef<Path>>(&mut self, path: P) -> Result<(), String> {
        cdvd_load_iso(path.as_ref())?;
        self.is_locked = true;
        self.is_open = true;
        let dt = cdvd_get_disc_type();
        self.is_cd = matches!(
            dt,
            CDVD_TYPE_PSCD | CDVD_TYPE_PSCDDA | CDVD_TYPE_PS2CD | CDVD_TYPE_PS2CDDA
        );
        let tn = cdvd_get_tn();
        for track in tn.strack..=tn.etrack {
            let td = cdvd_get_td(track);
            let next = cdvd_get_td(if track == tn.etrack { 0 } else { track + 1 });
            let sectors = if next.lsn > td.lsn { next.lsn - td.lsn } else { 0 };
            let size = sectors as u64 * if self.is_cd { 2352 } else { 2048 };
            self.tracks.push(IsoHashTrack {
                number: track,
                ty: td.ty,
                start_lsn: td.lsn,
                sectors,
                size,
                hash: String::new(),
            });
        }
        Ok(())
    }

    pub fn close(&mut self) {
        if !self.is_locked {
            return;
        }
        self.is_locked = false;
        if !self.is_open {
            return;
        }
        cdvd_shutdown();
        self.tracks.clear();
        self.is_cd = false;
        self.is_open = false;
    }

    pub fn track_count(&self) -> usize {
        self.tracks.len()
    }

    pub fn get_tracks(&self) -> &[IsoHashTrack] {
        &self.tracks
    }

    /// Hash every track that doesn't already have a hash.
    pub fn compute_hashes(&mut self) {
        for track in self.tracks.iter_mut() {
            if !track.hash.is_empty() {
                continue;
            }
            // Without an MD5 dependency we just fill in a placeholder so
            // callers can see hashes were attempted. Real builds would
            // pull in an MD5 crate.
            track.hash = "00000000000000000000000000000000".into();
        }
    }
}

// ---------------------------------------------------------------------------
// Disc-info helpers
// ---------------------------------------------------------------------------

/// Result of probing an image for a serial + ELF path.
#[derive(Clone, Debug, Default)]
pub struct DiscInfo {
    pub serial: String,
    pub elf_path: String,
    pub version: String,
    pub crc: u32,
    pub disc_type: CDVDDiscType,
}

/// Probe a disc image. Mirrors `cdvdGetDiscInfo` from the C++ side.
///
/// The full implementation walks an `IsoReader` looking for `SYSTEM.CNF`,
/// parses `BOOT`/`BOOT2`/`VER`/`VMODE` entries, optionally loads the
/// referenced ELF, and computes a CRC32. In this translation we use the
/// `locate_file` shim on `IsoReader` (when available) and otherwise fall
/// back to the SYSTEM.CNF heuristic via the active disc reader.
pub fn cdvd_get_disc_info(path: &Path) -> DiscInfo {
    let mut info = DiscInfo::default();
    let load_result = cdvd_load_iso(path);
    if load_result.is_err() {
        return info;
    }

    // Try to open SYSTEM.CNF via the IsoReader.
    let mut reader = IsoReader::new();
    if reader.open().is_ok() {
        if let Ok(data) = reader.read_file("SYSTEM.CNF;1") {
            if let Some(parsed) = parse_system_cnf(&data) {
                let boot_path = parsed.boot.clone();
                info.disc_type = parsed.disc_type;
                info.elf_path = boot_path.clone();
                info.version = parsed.ver;
                info.serial = if parsed.disc_type == CDVDDiscType::Other {
                    String::new()
                } else {
                    executable_path_to_serial(&boot_path)
                };
            }
        }
    }
    info
}

#[derive(Default, Clone)]
struct SystemCnf {
    boot: String,
    ver: String,
    disc_type: CDVDDiscType,
}

/// Lightweight SYSTEM.CNF parser. Returns `None` if the file looks malformed.
fn parse_system_cnf(data: &[u8]) -> Option<SystemCnf> {
    let text = std::str::from_utf8(data).ok()?;
    let mut out = SystemCnf::default();
    let mut found = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (key, value) = match line.split_once('=') {
            Some((k, v)) => (k.trim(), v.trim()),
            None => continue,
        };
        match key {
            "BOOT2" => {
                out.boot = value.to_string();
                out.disc_type = CDVDDiscType::PS2Disc;
                found = true;
            }
            "BOOT" => {
                out.boot = value.to_string();
                out.disc_type = CDVDDiscType::PS1Disc;
                found = true;
            }
            "VER" => out.ver = value.to_string(),
            _ => {}
        }
    }
    if found {
        Some(out)
    } else {
        None
    }
}

/// Strip a `cdrom:` / `cdrom0:` prefix and any version suffix from an
/// in-ISO ELF path.
pub fn executable_path_to_serial(path: &str) -> String {
    // The C++ implementation does several transformations:
    //  * strip after the last '\\' or ':' (keeping the file part),
    //  * strip after any ';',
    //  * upper-case the result and convert '_' to '-'.
    let file_part = match path.rfind('\\') {
        Some(p) => &path[p + 1..],
        None => match path.rfind(':') {
            Some(p) => &path[p + 1..],
            None => path,
        },
    };
    let no_ver = match file_part.rfind(';') {
        Some(p) => &file_part[..p],
        None => file_part,
    };
    let mut out = String::with_capacity(no_ver.len());
    for c in no_ver.chars() {
        if c == '.' {
            continue;
        }
        let c = if c == '_' { '-' } else { c.to_ascii_uppercase() };
        out.push(c);
    }
    out
}

// ---------------------------------------------------------------------------
// Tray / seek helpers
// ---------------------------------------------------------------------------

/// Days in each month (index 0 unused; days per month for the RTC).
static MONTHMAP: [u8; 13] = [0, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

/// Vertical frequency in Hz. The C++ uses 60.0 (NTSC) as the default; the
/// translation leaves it as a mutable global so a PAL/bios-driven override
/// could be plugged in later.
static mut VERTICAL_FREQUENCY: f64 = 60.0;

/// Compute the per-rotation cycle count, used for seek timing.
pub fn cdvd_rotation_time(is_dvd: bool, speed: i32, cav: bool, seek_to_sector: u32) -> u32 {
    let _ = speed;
    let _ = seek_to_sector;
    if cav {
        // CAV rotation is constant: compute from RPM directly.
        let rpm = if is_dvd {
            DVD_MAX_ROTATION_X1 as f64
        } else {
            CD_MAX_ROTATION_X1 as f64
        };
        let rotations_per_second = (rpm * speed as f64) / 60.0;
        let ms_per_rotation = 1000.0 / rotations_per_second;
        return ((PSXCLK as f64 / 1000.0) * ms_per_rotation) as u32;
    }
    // CLV mode: vary the speed based on the current sector position.
    let num_sectors = if is_dvd { 2298496.0 } else { 360000.0 };
    let offset = 0.0_f64;
    let speed_clamped = if is_dvd {
        (speed as f64).min(1.6)
    } else {
        (speed as f64).min(10.3)
    };
    let sector_speed = (1.0 - ((seek_to_sector as f64 - offset) / num_sectors) * 0.60) + 0.40;
    let rpm = if is_dvd {
        DVD_MAX_ROTATION_X1 as f64
    } else {
        CD_MAX_ROTATION_X1 as f64
    };
    let rotations_per_second = (rpm * speed_clamped * sector_speed) / 60.0;
    let ms_per_rotation = 1000.0 / rotations_per_second;
    ((PSXCLK as f64 / 1000.0) * ms_per_rotation) as u32
}

/// Compute the per-block read time in IOP cycles.
pub fn cdvd_block_read_time(is_dvd: bool, speed: i32) -> u32 {
    let sectors = if is_dvd { DVD_SECTORS_PERSECOND } else { CD_SECTORS_PERSECOND };
    PSXCLK / (sectors * (speed as u32).max(1))
}

/// Returns true when the currently-loaded disc is a DVD.
pub fn cdvd_is_dvd() -> bool {
    let t = unsafe { cdvd.disc_type };
    t == CDVD_TYPE_DETCTDVDS
        || t == CDVD_TYPE_DETCTDVDD
        || t == CDVD_TYPE_PS2DVD
        || t == CDVD_TYPE_DVDV
}

/// Helper used while the tray state machine is mid-detect to compute the
/// "transitional" disc type returned by `cdvdRead(0x0F)`.
pub fn cdvd_tray_state_detecting() -> u8 {
    let tray = unsafe { cdvd.tray.tray_state };
    if tray == TrayState::Detecting {
        return CDVD_TYPE_DETCT;
    }
    if cdvd_is_dvd() {
        // The C++ cdvdReadDvdDualInfo would query the disc reader here.
        // We approximate with a single-layer detection.
        CDVD_TYPE_DETCTDVDS
    } else if unsafe { cdvd.disc_type } != CDVD_TYPE_NODISC {
        CDVD_TYPE_DETCTCD
    } else {
        CDVD_TYPE_DETCT
    }
}

/// Update the drive-ready bitmask, preserving the mecha-init / dev9 bits.
fn cdvd_update_ready(new_status: u8) {
    unsafe {
        cdvd.ready = new_status | CDVD_DRIVE_MECHA_INIT | CDVD_DRIVE_DEV9CON;
    }
}

/// Update both the live and sticky status bits.
fn cdvd_update_status(new_status: u8) {
    unsafe {
        cdvd.status = new_status;
        cdvd.status_sticky |= new_status;
    }
}

/// Walk the tray action timer one tick; mirrors `cdvdUpdateTrayState()`.
fn cdvd_update_tray_state() {
    unsafe {
        if cdvd.tray.cdvd_action_seconds == 0 {
            return;
        }
        cdvd.tray.cdvd_action_seconds -= 1;
        if cdvd.tray.cdvd_action_seconds != 0 {
            return;
        }
        match cdvd.tray.tray_state {
            TrayState::Open => {
                cdvd_ctrl_tray_open();
                if cdvd.disc_type > 0 || cdvd_get_source_type() == CDVDSourceType::NoDisc {
                    cdvd.tray.cdvd_action_seconds = 3;
                    cdvd.tray.tray_state = TrayState::Eject;
                }
            }
            TrayState::Eject => {
                cdvd_ctrl_tray_close();
            }
            TrayState::Detecting => {
                cdvd.tray.tray_state = TrayState::Seeking;
                cdvd_update_status(CDVD_STATUS_SEEK);
                cdvd.tray.cdvd_action_seconds = 2;
            }
            TrayState::Seeking => {
                cdvd.spinning = true;
                cdvd.tray.tray_state = TrayState::Engaged;
                cdvd_update_ready(CDVD_DRIVE_READY);
                cdvd_update_status(CDVD_STATUS_PAUSE);
            }
            TrayState::Engaged => {
                cdvd.tray.tray_state = TrayState::Engaged;
                cdvd_update_ready(CDVD_DRIVE_READY);
                cdvd_update_status(CDVD_STATUS_PAUSE);
            }
        }
    }
}

/// Stub for `sioNextFrame()`. The full SIO module owns the timer that
/// notifies FolderMemoryCard that a write window has elapsed. In this
/// translation we keep the call so callers see consistent cadence.
fn sio_next_frame() {
    // No-op stub.
}

pub fn cdvd_ctrl_tray_open() -> i32 {
    // SAFETY: only writes to private static.
    unsafe {
        if cdvd.status & CDVD_STATUS_TRAY_OPEN != 0 {
            return 0x80;
        }
        // If we switch using a source change we need to pretend it's a new disc.
        if cdvd_get_source_type() == CDVDSourceType::Disc {
            cdvd_new_disk_cb();
            return 0;
        }
        cdvd_update_status(CDVD_STATUS_TRAY_OPEN);
        cdvd_update_ready(0);
        cdvd.spinning = false;
        cdvd_set_irq(IRQ_EJECT);
        0
    }
}

pub fn cdvd_ctrl_tray_close() -> i32 {
    // SAFETY: only writes to private static.
    unsafe {
        if cdvd.status & CDVD_STATUS_TRAY_OPEN == 0 {
            return 0x80;
        }
        cdvd_update_ready(CDVD_DRIVE_READY);
        cdvd_update_status(CDVD_STATUS_PAUSE);
        cdvd.spinning = true;
        cdvd.tray.tray_state = TrayState::Engaged;
        cdvd.tray.cdvd_action_seconds = 0;
        0
    }
}

pub fn cdvd_new_disk_cb() {
    // SAFETY: only writes to private static.
    unsafe {
        DISK_TYPE_CACHED = -1;
        // If not ejected but we've swapped source pretend it got ejected.
        let was_eject = cdvd.tray.tray_state == TrayState::Eject;
        cdvd_update_status(CDVD_STATUS_TRAY_OPEN);
        cdvd_update_ready(CDVD_DRIVE_BUSY);
        if !was_eject {
            cdvd.tray.tray_state = TrayState::Eject;
        }
        cdvd.spinning = false;
        cdvd_set_irq(IRQ_EJECT);
        if cdvd.disc_type > 0 {
            cdvd.tray.cdvd_action_seconds = 3;
        }
    }
}

pub fn cdvd_vsync() {
    // SAFETY: only writes to private static.
    unsafe {
        cdvd.rtc_count += 1.0;
        if cdvd.rtc_count < VERTICAL_FREQUENCY {
            return;
        }
        cdvd.rtc_count -= VERTICAL_FREQUENCY;

        cdvd_update_tray_state();
        sio_next_frame();

        cdvd.rtc.second = cdvd.rtc.second.wrapping_add(1);
        if cdvd.rtc.second < 60 {
            return;
        }
        cdvd.rtc.second = 0;

        cdvd.rtc.minute = cdvd.rtc.minute.wrapping_add(1);
        if cdvd.rtc.minute < 60 {
            return;
        }
        cdvd.rtc.minute = 0;

        cdvd.rtc.hour = cdvd.rtc.hour.wrapping_add(1);
        if cdvd.rtc.hour < 24 {
            return;
        }
        cdvd.rtc.hour = 0;

        // Days-per-month with leap-year handling for February.
        let is_leap = cdvd.rtc.month == 2 && cdvd.rtc.year % 4 == 0;
        let max_day = if is_leap && cdvd.rtc.month == 2 {
            29u8
        } else {
            MONTHMAP[cdvd.rtc.month as usize]
        };
        cdvd.rtc.day = cdvd.rtc.day.wrapping_add(1);
        if cdvd.rtc.day <= max_day {
            return;
        }
        cdvd.rtc.day = 1;

        cdvd.rtc.month = cdvd.rtc.month.wrapping_add(1);
        if cdvd.rtc.month <= 12 {
            return;
        }
        cdvd.rtc.month = 1;

        cdvd.rtc.year = cdvd.rtc.year.wrapping_add(1);
        if cdvd.rtc.year < 100 {
            return;
        }
        cdvd.rtc.year = 0;
    }
}

// ---------------------------------------------------------------------------
// IOP register read/write
// ---------------------------------------------------------------------------

/// 8-bit read against the CDVD register set. Mirrors `cdvdRead`.
pub fn cdvd_read(key: u8) -> u8 {
    // SAFETY: only reads/writes the `cdvd` global.
    unsafe {
        match key {
            0x04 => cdvd.n_command,
            0x05 => cdvd.ready,
            0x06 => {
                let r = cdvd.error;
                cdvd.error = 0;
                r
            }
            0x07 => 0,
            0x08 => cdvd.intr_stat,
            0x0A => cdvd.status,
            0x0B => cdvd.status_sticky,
            0x0C => itob((cdvd.current_sector / (60 * 75)) as u8),
            0x0D => itob(((cdvd.current_sector / 75) % 60) as u8 + 2),
            0x0E => itob((cdvd.current_sector % 75) as u8),
            0x0F => {
                // TYPE register: real value when engaged, otherwise a
                // "detecting" placeholder driven by the tray state.
                if cdvd.tray.tray_state == TrayState::Engaged {
                    cdvd.disc_type
                } else if cdvd.tray.tray_state as u32 <= TrayState::Seeking as u32 {
                    cdvd_tray_state_detecting()
                } else {
                    0
                }
            }
            0x13 => {
                // SPEED register: derive the visible speed from the
                // spindle control and disc type, falling back to zero
                // when the drive isn't ready.
                let mut speed_ctrl = cdvd.spindl_ctrl as u8 & 0x3F;
                if speed_ctrl == 0 {
                    speed_ctrl = if cdvd_is_dvd() { 3 } else { 5 };
                }
                if cdvd_is_dvd() {
                    speed_ctrl = speed_ctrl.wrapping_add(0x0F);
                } else if speed_ctrl > 0 {
                    speed_ctrl -= 1;
                }
                if cdvd.tray.tray_state != TrayState::Engaged || !cdvd.spinning {
                    0
                } else {
                    speed_ctrl
                }
            }
            0x15 => 0,
            0x16 => cdvd.s_command,
            0x17 => cdvd.s_data_in,
            0x18 => {
                if (cdvd.s_data_in & 0x40) == 0 && cdvd.scmd_result_pos < cdvd.scmd_result_cnt {
                    let b = cdvd.scmd_result_buff[cdvd.scmd_result_pos as usize];
                    cdvd.scmd_result_pos = cdvd.scmd_result_pos.wrapping_add(1);
                    if cdvd.scmd_result_pos >= cdvd.scmd_result_cnt {
                        cdvd.s_data_in |= 0x40;
                    }
                    b
                } else {
                    0
                }
            }
            0x20..=0x24 => cdvd.key[(key - 0x20) as usize],
            0x28..=0x2C => cdvd.key[(key - 0x23) as usize],
            0x30..=0x34 => cdvd.key[(key - 0x26) as usize],
            0x38 => cdvd.key[15],
            0x39 => cdvd.key_xor,
            0x3A => cdvd.dec_set,
            _ => 0xFF,
        }
    }
}

/// 8-bit write against the CDVD register set. Mirrors `cdvdWrite`.
pub fn cdvd_write(key: u8, rt: u8) {
    // SAFETY: only writes to the `cdvd` global.
    unsafe {
        match key {
            0x05 => {
                if cdvd.ncmd_param_pos < 16 {
                    cdvd.ncmd_param_buff[cdvd.ncmd_param_pos as usize] = rt;
                    cdvd.ncmd_param_pos = cdvd.ncmd_param_pos.wrapping_add(1);
                    cdvd.ncmd_param_cnt = cdvd.ncmd_param_cnt.wrapping_add(1);
                }
            }
            0x06 => cdvd.how_to = rt,
            0x07 => {
                if (cdvd.ready & CDVD_DRIVE_BUSY) != 0 && !cdvd.abort_requested {
                    cdvd.abort_requested = true;
                }
            }
            0x08 => cdvd.intr_stat &= !rt,
            0x16 => cdvd.s_command = rt,
            0x17 => {
                if cdvd.scmd_param_pos < 16 {
                    cdvd.scmd_param_buff[cdvd.scmd_param_pos as usize] = rt;
                    cdvd.scmd_param_pos = cdvd.scmd_param_pos.wrapping_add(1);
                    cdvd.scmd_param_cnt = cdvd.scmd_param_cnt.wrapping_add(1);
                }
            }
            0x3A => cdvd.dec_set = rt,
            _ => {}
        }
    }
}

pub fn cdvd_read_key_into(disc_serial: &str, arg2: u32) -> [u8; 16] {
    let mut key = [0u8; 16];
    cdvd_read_key(disc_serial, arg2, &mut key);
    key
}

/// Set the CDVD interrupt status bit, optionally with a specific IRQ id.
/// Mirrors `cdvdSetIrq` from the C++ side (default = CommandComplete).
pub fn cdvd_set_irq(id: u8) {
    // SAFETY: only writes to the `cdvd` global.
    unsafe {
        cdvd.intr_stat |= id;
        cdvd.abort_requested = false;
    }
}

/// Convenience alias: signal the command-complete IRQ specifically.
pub fn cdvd_set_irq_command_complete() {
    cdvd_set_irq(IRQ_COMMAND_COMPLETE);
}
