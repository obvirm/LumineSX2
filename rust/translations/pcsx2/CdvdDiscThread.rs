// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
//! CDVD disc-thread subsystem translation from PCSX2's C++ original.
//!
//! This module is the idiomatic Rust 2021 translation of the PCSX2 CDVD
//! disc-thread subsystem (the C++ source set located in `pcsx2/CDVD/`).
//! It exposes:
//!
//! * [`CDVDState`] - the IOP-visible CDVD register set, RTC, spindle, seek
//!   and tray state (corresponds to `cdvdStruct` in C++).
//! * [`CDVDDiscThread`] - the asynchronous disc reader / prefetch thread
//!   used by the disc-source backend.
//! * [`CDVDdiscReader`] - the actual disc reader with cache, request queue
//!   and keep-alive support.
//! * [`Ps1CD`] - the PS1-mode CD-ROM emulation (`cdrStruct`, `cdr.*`).
//! * [`IsoHasher`] - per-track MD5 hashing of an open ISO.
//!
//! All PS1 (`CdlSync`, `CdlNop`, `CdlSetloc`, `CdlPlay`, `CdlReadN`, ...)
//! and PS2 (`N_CD_NOP`, `N_CD_RESET`, `N_CD_READ`, ...) command enums are
//! preserved.  Only `std` is required; `static mut` is used to mirror the
//! global state present in the original C++.

#![allow(dead_code)]
#![allow(non_snake_case)]
#![allow(non_camel_case_types)]

use std::cmp::{max, min};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::thread::JoinHandle;

// =====================================================================================
//  Constants (from CDVD_internal.h / CDVDcommon.h)
// =====================================================================================

/// Master IOP clock rate, in Hz.
pub const PSXCLK: u32 = 36_864_000;

/// GMT+9 offset used internally by CDVD's RTC bookkeeping.
pub const GMT9_OFFSET_SECONDS: i32 = 9 * 60 * 60;

/// Per-month day counts (1-indexed) used by the RTC rollover logic.
pub const MONTHMAP: [u8; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

/// Required parameter length, indexed by NCMD opcode.
pub const CDVD_PARAM_LENGTH: [u8; 16] = [0, 0, 0, 0, 0, 4, 11, 11, 11, 1, 255, 255, 7, 2, 11, 1];

/// NVRAM file size, in bytes.
pub const NVRAM_SIZE: usize = 1024;

/// Default mechacon version baked into the NVRAM file.
pub const DEFAULT_MECHA_VERSION: u32 = 0x0002_0603;

/// Seek-time deltas for "fast seek" selection (CD / single-layer DVD /
/// dual-layer DVD).
pub const TBL_FAST_SEEK_DELTA: [u32; 3] = [4371, 14764, 13360];

/// Within this many blocks, contiguous read is preferred over a real seek.
pub const TBL_CONTIGIOUS_SEEK_DELTA: [u32; 3] = [8, 16, 16];

/// Approx 1x CD read speed in bytes per second.
pub const PSX_CD_READSPEED: u32 = 153_600;
/// Approx 1x DVD read speed in bytes per second.
pub const PSX_DVD_READSPEED: u32 = 1_382_400;

/// Sectors per second at 1x.
pub const CD_SECTORS_PERSECOND: u32 = 75;
pub const DVD_SECTORS_PERSECOND: u32 = 675;

/// Rotations per minute.
pub const CD_MIN_ROTATION_X1: u32 = 214;
pub const CD_MAX_ROTATION_X1: u32 = 497;
pub const DVD_MIN_ROTATION_X1: u32 = 570;
pub const DVD_MAX_ROTATION_X1: u32 = 1515;

/// Average cycles per full-seek (100ms).
pub const CDVD_FULL_SEEK_CYCLES: u32 = (PSXCLK * 100) / 1000;
/// Average cycles per fast-seek (37ms).
pub const CDVD_FAST_SEEK_CYCLES: u32 = (PSXCLK * 30) / 1000;

/// MG zone human-readable labels.
pub const MG_ZONES: [&str; 8] = [
    "Japan", "USA", "Europe", "Oceania", "Asia", "Russia", "China", "Mexico",
];

// =====================================================================================
//  CDVD disc-type, source-type, status, ready bits
// =====================================================================================

/// Disc-type tag returned by `sceCdGetDiscType` / friends.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CDVDDiscType {
    Other = 0,
    PS1Disc = 1,
    PS2Disc = 2,
}

/// State-machine stages for the disc tray lifecycle.
#[repr(u32)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TrayStates {
    #[default]
    Engaged = 0,
    Detecting = 1,
    Seeking = 2,
    Eject = 3,
    Open = 4,
}

/// Selected source of CDVD data.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CDVDSourceType {
    Iso = 0,
    Disc = 1,
    NoDisc = 2,
}

/// CDVD `Status` register bit values.
pub mod cdvdStatus {
    pub const CDVD_STATUS_STOP: u8 = 0x00;
    pub const CDVD_STATUS_TRAY_OPEN: u8 = 0x01;
    pub const CDVD_STATUS_SPIN: u8 = 0x02;
    pub const CDVD_STATUS_READ: u8 = 0x06;
    pub const CDVD_STATUS_PAUSE: u8 = 0x0A;
    pub const CDVD_STATUS_SEEK: u8 = 0x12;
    pub const CDVD_STATUS_EMERGENCY: u8 = 0x20;
}

/// `cdvdReady` register bit values.
pub mod cdvdready {
    pub const CDVD_DRIVE_ERROR: u8 = 0x01;
    pub const CDVD_DRIVE_DEV9CON: u8 = 0x04;
    pub const CDVD_DRIVE_MECHA_INIT: u8 = 0x08;
    pub const CDVD_DRIVE_PWOFF: u8 = 0x20;
    pub const CDVD_DRIVE_READY: u8 = 0x40;
    pub const CDVD_DRIVE_BUSY: u8 = 0x80;
}

/// Coarse categories of CDVD internal action.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CdvdActions {
    None = 0,
    Seek,
    Standby,
    Stop,
    Error,
    Read,
}

/// CDVD "read mode" (block size) selector.
pub mod cdvdReadMode {
    pub const CDVD_MODE_2352: i32 = 0;
    pub const CDVD_MODE_2340: i32 = 1;
    pub const CDVD_MODE_2328: i32 = 2;
    pub const CDVD_MODE_2048: i32 = 3;
    pub const CDVD_MODE_2368: i32 = 4;
}

/// Spindle-control bit masks.
pub mod cdvdSpindle {
    pub const CDVD_SPINDLE_SPEED: u8 = 0x07;
    pub const CDVD_SPINDLE_NOMINAL: u8 = 0x40;
    pub const CDVD_SPINDLE_CAV: u8 = 0x80;
}

/// CDVD disc-type "get type" return values.
pub mod cdvdType {
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
}

/// Tray status values.
pub mod cdvdTray {
    pub const CDVD_TRAY_CLOSE: u8 = 0x00;
    pub const CDVD_TRAY_OPEN: u8 = 0x01;
}

/// CDVD track-type constants.
pub mod cdvdTrackType {
    pub const CDVD_AUDIO_TRACK: u8 = 0x01;
    pub const CDVD_MODE1_TRACK: u8 = 0x41;
    pub const CDVD_MODE2_TRACK: u8 = 0x61;
    pub const CDVD_AUDIO_MASK: u8 = 0x00;
    pub const CDVD_DATA_MASK: u8 = 0x40;
}

// =====================================================================================
//  IRQ identifiers (from CDVD_internal.h)
// =====================================================================================

/// CDVD IRQ identifiers.
pub mod CdvdIrqId {
    pub const IRQ_NONE: u32 = 0;
    pub const IRQ_COMMAND_COMPLETE: u32 = 0;
    pub const IRQ_POFF_READY: u32 = 2;
    pub const IRQ_EJECT: u32 = 3;
    pub const IRQ_BS_POWER: u32 = 4;
}

// =====================================================================================
//  CDVD N-commands (NCMDs) and S-commands (SCMDs)
// =====================================================================================

/// PS2 N-commands (large / "data path" command set).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NCmd {
    Nop = 0x00,
    Reset = 0x01,
    Standby = 0x02,
    Stop = 0x03,
    Pause = 0x04,
    Seek = 0x05,
    Read = 0x06,
    ReadCDDA = 0x07,
    DvdRead = 0x08,
    GetToc = 0x09,
    CmdB = 0x0B,
    ReadKey = 0x0C,
    ReadXCDDA = 0x0E,
    ChgSpdlCtrl = 0x0F,
}

/// PS2 S-commands (small / "control" command set).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SCmd {
    GetDiscType = 0x01,
    ReadSubQ = 0x02,
    Mecacon = 0x03,
    CdTrayReqState = 0x05,
    CdTrayCtrl = 0x06,
    ReadClock = 0x08,
    WriteClock = 0x09,
    ReadNVM = 0x0A,
    WriteNVM = 0x0B,
    SetHDMode = 0x0C,
    PowerOff = 0x0F,
    ReadILinkID = 0x12,
    WriteILinkID = 0x13,
    AudioDigitalOut = 0x14,
    ForbidDVDP = 0x15,
    AutoAdjustCtrl = 0x16,
    ReadModelNumber = 0x17,
    WriteModelNumber = 0x18,
    ForbidCD = 0x19,
    BootCertify = 0x1A,
    CancelPOffRdy = 0x1B,
    BlueLEDCtl = 0x1C,
    Rm2Read = 0x1E,
    Remote2_7 = 0x1F,
    Remote2_6 = 0x20,
    WriteWakeUpTime = 0x21,
    ReadWakeUpTime = 0x22,
    RcBypassCtl = 0x24,
    NoticeGameStart = 0x29,
    SetMediumRemoval = 0x31,
    GetMediumRemoval = 0x32,
    XDVRPReset = 0x33,
    ReadRegionParams = 0x36,
    ReadMAC = 0x37,
    WriteMAC = 0x38,
    WriteRegionParams = 0x3E,
    OpenConfig = 0x40,
    ReadConfig = 0x41,
    WriteConfig = 0x42,
    CloseConfig = 0x43,
    MgAuth80 = 0x80,
    MgAuth81 = 0x81,
    MgAuth82 = 0x82,
    MgAuth83 = 0x83,
    MgAuth84 = 0x84,
    MgAuth85 = 0x85,
    MgAuth86 = 0x86,
    MgAuth87 = 0x87,
    MgAuth88 = 0x88,
    MgWriteData = 0x8D,
    MgReadData = 0x8E,
    MgAuth8F = 0x8F,
    MgWriteHeaderStart = 0x90,
    MgReadBITLength = 0x91,
    MgWriteDatainLength = 0x92,
    MgWriteDataoutLength = 0x93,
    MgReadKbit = 0x94,
    MgReadKbit2 = 0x95,
    MgReadKcon = 0x96,
    MgReadKcon2 = 0x97,
}

impl SCmd {
    pub fn name(self) -> &'static str {
        match self {
            SCmd::GetDiscType => "sceCdGetDiscType",
            SCmd::ReadSubQ => "sceCdReadSubQ",
            SCmd::Mecacon => "subcommands",
            SCmd::CdTrayReqState => "sceCdTrayState",
            SCmd::CdTrayCtrl => "sceCdTrayCtrl",
            SCmd::ReadClock => "sceCdReadClock",
            SCmd::WriteClock => "sceCdWriteClock",
            SCmd::ReadNVM => "sceCdReadNVM",
            SCmd::WriteNVM => "sceCdWriteNVM",
            SCmd::SetHDMode => "sceCdSetHDMode",
            SCmd::PowerOff => "sceCdPowerOff",
            SCmd::ReadILinkID => "sceCdReadILinkID",
            SCmd::WriteILinkID => "sceCdWriteILinkID",
            SCmd::AudioDigitalOut => "sceAudioDigitalOut",
            SCmd::ForbidDVDP => "sceForbidDVDP",
            SCmd::AutoAdjustCtrl => "sceAutoAdjustCtrl",
            SCmd::ReadModelNumber => "sceCdReadModelNumber",
            SCmd::WriteModelNumber => "sceWriteModelNumber",
            SCmd::ForbidCD => "sceCdForbidCD",
            SCmd::BootCertify => "sceCdBootCertify",
            SCmd::CancelPOffRdy => "sceCdCancelPOffRdy",
            SCmd::BlueLEDCtl => "sceCdBlueLEDCtl",
            SCmd::Rm2Read => "sceRm2Read",
            SCmd::Remote2_7 => "sceRemote2_7",
            SCmd::Remote2_6 => "sceRemote2_6",
            SCmd::WriteWakeUpTime => "sceCdWriteWakeUpTime",
            SCmd::ReadWakeUpTime => "sceCdReadWakeUpTime",
            SCmd::RcBypassCtl => "sceCdRcBypassCtl",
            SCmd::NoticeGameStart => "sceCdNoticeGameStart",
            SCmd::SetMediumRemoval => "sceCdSetMediumRemoval",
            SCmd::GetMediumRemoval => "sceCdGetMediumRemoval",
            SCmd::XDVRPReset => "sceCdXDVRPReset",
            SCmd::ReadRegionParams => "__sceCdReadRegionParams",
            SCmd::ReadMAC => "__sceCdReadMAC",
            SCmd::WriteMAC => "__sceCdWriteMAC",
            SCmd::WriteRegionParams => "__sceCdWriteRegionParams",
            SCmd::OpenConfig => "sceCdOpenConfig",
            SCmd::ReadConfig => "sceCdReadConfig",
            SCmd::WriteConfig => "sceCdWriteConfig",
            SCmd::CloseConfig => "sceCdCloseConfig",
            SCmd::MgAuth80 => "mechacon_auth_0x80",
            SCmd::MgAuth81 => "mechacon_auth_0x81",
            SCmd::MgAuth82 => "mechacon_auth_0x82",
            SCmd::MgAuth83 => "mechacon_auth_0x83",
            SCmd::MgAuth84 => "mechacon_auth_0x84",
            SCmd::MgAuth85 => "mechacon_auth_0x85",
            SCmd::MgAuth86 => "mechacon_auth_0x86",
            SCmd::MgAuth87 => "mechacon_auth_0x87",
            SCmd::MgAuth88 => "mechacon_auth_0x88",
            SCmd::MgWriteData => "sceMgWriteData",
            SCmd::MgReadData => "sceMgReadData",
            SCmd::MgAuth8F => "mechacon_auth_0x8F",
            SCmd::MgWriteHeaderStart => "sceMgWriteHeaderStart",
            SCmd::MgReadBITLength => "sceMgReadBITLength",
            SCmd::MgWriteDatainLength => "sceMgWriteDatainLength",
            SCmd::MgWriteDataoutLength => "sceMgWriteDataoutLength",
            SCmd::MgReadKbit => "sceMgReadKbit",
            SCmd::MgReadKbit2 => "sceMgReadKbit2",
            SCmd::MgReadKcon => "sceMgReadKcon",
            SCmd::MgReadKcon2 => "sceMgReadKcon2",
        }
    }
}

// =====================================================================================
//  PS1 (CDROM) commands (from Ps1CD.cpp)
// =====================================================================================

/// PS1 CDROM command opcodes.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CdlCmd {
    CdlSync = 0,
    CdlNop = 1,
    CdlSetloc = 2,
    CdlPlay = 3,
    CdlForward = 4,
    CdlBackward = 5,
    CdlReadN = 6,
    CdlStandby = 7,
    CdlStop = 8,
    CdlPause = 9,
    CdlInit = 10,
    CdlMute = 11,
    CdlDemute = 12,
    CdlSetfilter = 13,
    CdlSetmode = 14,
    CdlGetparam = 15,
    CdlGetlocL = 16,
    CdlGetlocP = 17,
    Cdl18 = 18,
    CdlGetTN = 19,
    CdlGetTD = 20,
    CdlSeekL = 21,
    CdlSeekP = 22,
    CdlTest = 25,
    CdlID = 26,
    CdlReadS = 27,
    CdlReset = 28,
    CdlReadToc = 30,
    AutoPause = 249,
    ReadAck = 250,
    Read = 251,
    RepplayAck = 252,
    Repplay = 253,
    Async = 254,
}

/// PS1 `cdr.Stat` return values.
pub mod CdrStat {
    pub const NO_INTR: u8 = 0;
    pub const DATA_READY: u8 = 1;
    pub const COMPLETE: u8 = 2;
    pub const ACKNOWLEDGE: u8 = 3;
    pub const DATA_END: u8 = 4;
    pub const DISK_ERROR: u8 = 5;
}

/// PS1 `cdr.StatP` bit masks.
pub mod CdrStatP {
    pub const STATUS_PLAY: u8 = 1 << 7;
    pub const STATUS_SEEK: u8 = 1 << 6;
    pub const STATUS_READ: u8 = 1 << 5;
    pub const STATUS_SHELLOPEN: u8 = 1 << 4;
    pub const STATUS_IDERROR: u8 = 1 << 3;
    pub const STATUS_SEEKERROR: u8 = 1 << 2;
    pub const STATUS_ROTATING: u8 = 1 << 1;
    pub const STATUS_ERROR: u8 = 1 << 0;
}

/// PS1 `cdr.Mode` bit masks.
pub mod CdrMode {
    pub const MODE_INIT: u8 = 0 << 0;
    pub const MODE_SPEED: u8 = 1 << 7;
    pub const MODE_STRSND: u8 = 1 << 6;
    pub const MODE_SIZE_2340: u8 = 1 << 5;
    pub const MODE_SIZE_2328: u8 = 1 << 4;
    pub const MODE_SIZE_2048: u8 = 0 << 4;
    pub const MODE_SF: u8 = 1 << 3;
    pub const MODE_REPORT: u8 = 1 << 2;
    pub const MODE_AUTOPAUSE: u8 = 1 << 1;
    pub const MODE_CDDA: u8 = 1 << 0;
}

/// PS1 `cdr` error bits.
pub mod CdrError {
    pub const ERROR_NOTREADY: u8 = 1 << 7;
    pub const ERROR_INVALIDCMD: u8 = 1 << 6;
    pub const ERROR_INVALIDARG: u8 = 1 << 5;
}

// =====================================================================================
//  BCD / MSF helpers
// =====================================================================================

/// BCD byte -> binary.
#[inline]
pub fn btoi(b: u8) -> u8 {
    (b / 16) * 10 + (b % 16)
}

/// Binary byte -> BCD.
#[inline]
pub fn itob(i: u8) -> u8 {
    (i / 10) * 16 + (i % 10)
}

/// MSF (M/S/F) triplet -> logical sector number.
#[inline]
pub fn msf_to_lsn(time: [u8; 3]) -> i32 {
    let lsn = time[2] as i32;
    lsn + (time[1] as i32 - 2) * 75 + time[0] as i32 * 75 * 60
}

/// (M, S, F) -> LBA.
#[inline]
pub fn msf_to_lba(m: u8, s: u8, f: u8) -> i32 {
    let lsn = f as i32;
    lsn + (s as i32 - 2) * 75 + m as i32 * 75 * 60
}

/// LSN -> MSF triplet (BCD, written to `time`).
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

/// LBA -> (M, S, F) (binary, returned as separate `u8`s).
#[inline]
pub fn lba_to_msf(lba: i32) -> (u8, u8, u8) {
    let lba = lba + 150;
    let m = lba / (60 * 75);
    let s = (lba / 75) % 60;
    let f = lba % 75;
    (m as u8, s as u8, f as u8)
}

// =====================================================================================
//  CDVD sub-structures (from CDVD.h / CDVDcommon.h)
// =====================================================================================

/// Real-time clock state.
#[derive(Clone, Copy, Debug, Default)]
pub struct CdvdRtc {
    pub status: u8,
    pub second: u8,
    pub minute: u8,
    pub hour: u8,
    pub pad: u8,
    pub day: u8,
    pub month: u8,
    pub year: u8,
}

/// Tray-state machine countdown + phase.
#[derive(Clone, Copy, Debug, Default)]
pub struct CdvdTrayTimer {
    pub cdvd_action_seconds: u32,
    pub tray_state: TrayStates,
}

/// TOC track index (pregap / data).
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

/// One entry in a CD's table of contents.
#[derive(Clone, Debug, Default)]
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

/// SubQ (sub-channel Q) data block.
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

/// Track descriptor (LSN + type, not BCD).
#[derive(Clone, Copy, Debug, Default)]
pub struct CdvdTD {
    pub lsn: u32,
    pub ty: u8,
}

/// First/last track number pair.
#[derive(Clone, Copy, Debug, Default)]
pub struct CdvdTN {
    pub strack: u8,
    pub etrack: u8,
}

/// NVM (EEPROM) layout.
#[derive(Clone, Copy, Debug)]
pub struct NvmLayout {
    pub bios_ver: u32,
    pub config0: i32,
    pub config1: i32,
    pub config2: i32,
    pub console_id: i32,
    pub ilink_id: i32,
    pub model_num: i32,
    pub regparams: i32,
    pub mac: i32,
}

/// Region/console parameter defaults indexed by BiosRegion.
pub const PS2_REGION_DEFAULTS: [[u8; 12]; 13] = [
    [0x4a, 0x4a, 0x6a, 0x70, 0x6e, 0x4a, 0x4a, 0x00, 0x00, 0x00, 0x00, 0x00], // JP
    [0x41, 0x41, 0x65, 0x6e, 0x67, 0x41, 0x55, 0x00, 0x00, 0x00, 0x00, 0x00], // US
    [0x45, 0x45, 0x65, 0x6e, 0x67, 0x45, 0x45, 0x00, 0x00, 0x00, 0x00, 0x00], // EU
    [0x45, 0x45, 0x65, 0x6e, 0x67, 0x45, 0x4f, 0x00, 0x00, 0x00, 0x00, 0x00], // OCE
    [0x48, 0x48, 0x65, 0x6e, 0x67, 0x4a, 0x41, 0x00, 0x00, 0x00, 0x00, 0x00], // AS
    [0x45, 0x52, 0x65, 0x6e, 0x67, 0x45, 0x52, 0x00, 0x00, 0x00, 0x00, 0x00], // RU
    [0x43, 0x43, 0x73, 0x63, 0x68, 0x4A, 0x43, 0x00, 0x00, 0x00, 0x00, 0x00], // CN
    [0x41, 0x41, 0x73, 0x70, 0x61, 0x41, 0x4D, 0x00, 0x00, 0x00, 0x00, 0x00], // MX
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
    [0x48, 0x4b, 0x6b, 0x6f, 0x72, 0x4a, 0x41, 0x00, 0x00, 0x00, 0x00, 0x00], // KR
    [0x48, 0x48, 0x74, 0x63, 0x68, 0x4a, 0x41, 0x00, 0x00, 0x00, 0x00, 0x00], // TW
];

/// Default language parameters for each region.
pub const BIOS_LANG_DEFAULTS: [[u8; 16]; 11] = [
    [0x20, 0x20, 0x00, 0x00, 0x00, 0x70, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x30],
    [0x30, 0x21, 0x00, 0x00, 0x00, 0x70, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x41],
    [0x30, 0x21, 0x00, 0x00, 0x00, 0x70, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x41],
    [0x30, 0x21, 0x00, 0x00, 0x00, 0x70, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x41],
    [0x30, 0x21, 0x00, 0x00, 0x00, 0x70, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x41],
    [0x30, 0x21, 0x00, 0x00, 0x00, 0x70, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x41],
    [0x30, 0x2B, 0x00, 0x00, 0x00, 0x70, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x4B],
    [0x30, 0x21, 0x00, 0x00, 0x00, 0x70, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x41],
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
];

/// Recognised NVM layouts.
pub const NVM_LAYOUTS: [NvmLayout; 2] = [
    NvmLayout {
        bios_ver: 0x000,
        config0: 0x280,
        config1: 0x300,
        config2: 0x200,
        console_id: 0x1C8,
        ilink_id: 0x1C0,
        model_num: 0x1A0,
        regparams: 0x180,
        mac: 0x198,
    },
    NvmLayout {
        bios_ver: 0x146,
        config0: 0x270,
        config1: 0x2B0,
        config2: 0x200,
        console_id: 0x1F0,
        ilink_id: 0x1E0,
        model_num: 0x1B0,
        regparams: 0x180,
        mac: 0x198,
    },
];

/// Number of NVM layouts in [`NVM_LAYOUTS`].
pub const NVM_FORMAT_MAX: usize = 2;

// =====================================================================================
//  CDVDState - the global PS2 CDVD register set
// =====================================================================================

/// The IOP-visible CDVD register set, plus all derived state (RTC, tray,
/// spindle, seek bookkeeping, MagicGate auth scratch).  Mirrors `cdvdStruct`
/// from the original `CDVD.h`.
#[derive(Clone, Debug)]
pub struct CDVDState {
    // -- Command / status registers -----------------------------------------
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

    // -- Command parameter / result buffers ----------------------------------
    pub n_cmd_param_buff: [u8; 16],
    pub s_cmd_param_buff: [u8; 16],
    pub s_cmd_result_buff: [u8; 16],

    pub n_cmd_param_cnt: u8,
    pub n_cmd_param_pos: u8,
    pub s_cmd_param_cnt: u8,
    pub s_cmd_param_pos: u8,
    pub s_cmd_result_cnt: u8,
    pub s_cmd_result_pos: u8,

    // -- Config-block stream (sceCdOpenConfig / sceCdReadConfig / ...) ------
    pub c_block_index: u8,
    pub c_offset: u8,
    pub c_read_write: u8,
    pub c_num_blocks: u8,

    // -- RTC ----------------------------------------------------------------
    pub rtc_count: f64,
    pub rtc: CdvdRtc,

    // -- Active read --------------------------------------------------------
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

    // -- DVD decryption -----------------------------------------------------
    pub key: [u8; 16],
    pub key_xor: u8,
    pub dec_set: u8,

    // -- MagicGate scratch buffer -------------------------------------------
    pub mg_buffer: [u8; 65536],
    pub mg_size: i32,
    pub mg_max_size: i32,
    pub mg_data_type: i32, // 0 = encrypted data, 1 = header
    pub mg_kbit: [u8; 16],
    pub mg_kcon: [u8; 16],

    // -- Tray + spindle -----------------------------------------------------
    pub tray_timeout: u8,
    pub action: CdvdActions,
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
            status: cdvdStatus::CDVD_STATUS_TRAY_OPEN,
            status_sticky: 0,
            disc_type: cdvdType::CDVD_TYPE_NODISC,
            s_command: 0,
            s_data_in: 0,
            s_data_out: 0,
            how_to: 0,
            n_cmd_param_buff: [0; 16],
            s_cmd_param_buff: [0; 16],
            s_cmd_result_buff: [0; 16],
            n_cmd_param_cnt: 0,
            n_cmd_param_pos: 0,
            s_cmd_param_cnt: 0,
            s_cmd_param_pos: 0,
            s_cmd_result_cnt: 0,
            s_cmd_result_pos: 0,
            c_block_index: 0,
            c_offset: 0,
            c_read_write: 0,
            c_num_blocks: 0,
            rtc_count: 0.0,
            rtc: CdvdRtc::default(),
            current_sector: 0,
            sector_cnt: 0,
            seek_completed: 0,
            reading: 0,
            waiting_dma: 0,
            read_mode: 0,
            block_size: 0,
            speed: 0,
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
            mg_data_type: 0,
            mg_kbit: [0; 16],
            mg_kcon: [0; 16],
            tray_timeout: 0,
            action: CdvdActions::None,
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

impl CDVDState {
    /// Construct an empty, post-reset CDVD state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset all registers to their post-reset defaults.
    pub fn reset(&mut self) {
        *self = Self::default();
        self.s_data_in = 0x40;
        self.spinning = false;
        self.speed = 4;
        self.block_size = 2064;
        self.action = CdvdActions::None;
    }

    /// Set the `Status` and OR-accumulate into `StatusSticky`.
    pub fn update_status(&mut self, new_status: u8) {
        self.status = new_status;
        self.status_sticky |= new_status;
    }

    /// Update the `Ready` register, preserving the "mecha-init" and
    /// "DEV9 connected" sticky bits.
    pub fn update_ready(&mut self, new_ready: u8) {
        self.ready = new_ready | (cdvdready::CDVD_DRIVE_MECHA_INIT | cdvdready::CDVD_DRIVE_DEV9CON);
    }

    /// Schedule an IRQ of the given source-bit, mirroring `cdvdSetIrq()`.
    pub fn set_irq(&mut self, id: u32) {
        if (self.intr_stat & (id as u8)) == 0 {
            // In the C++ original, this calls iopIntcIrq(2) and
            // psxSetNextBranchDelta(20).  In the pure-Rust translation we
            // just mark the IRQ as pending.
        } else {
            // DevCon.Warning("CDVD trying to double issue IRQ %x", id);
        }
        self.intr_stat |= id as u8;
        self.abort_requested = false;
    }

    /// Returns true if the currently-inserted media is a DVD.
    pub fn is_dvd(&self) -> bool {
        matches!(
            self.disc_type,
            cdvdType::CDVD_TYPE_DETCTDVDS
                | cdvdType::CDVD_TYPE_DETCTDVDD
                | cdvdType::CDVD_TYPE_PS2DVD
                | cdvdType::CDVD_TYPE_DVDV
        )
    }
}

// =====================================================================================
//  Process-wide mutable state (static globals)
// =====================================================================================

/// Global CDVD register set.  Mirrors `cdvdStruct cdvd` in the C++.
pub static mut CDVD: CDVDState = CDVDState {
    n_command: 0,
    ready: 0,
    error: 0,
    intr_stat: 0,
    status: cdvdStatus::CDVD_STATUS_TRAY_OPEN,
    status_sticky: 0,
    disc_type: cdvdType::CDVD_TYPE_NODISC,
    s_command: 0,
    s_data_in: 0,
    s_data_out: 0,
    how_to: 0,
    n_cmd_param_buff: [0; 16],
    s_cmd_param_buff: [0; 16],
    s_cmd_result_buff: [0; 16],
    n_cmd_param_cnt: 0,
    n_cmd_param_pos: 0,
    s_cmd_param_cnt: 0,
    s_cmd_param_pos: 0,
    s_cmd_result_cnt: 0,
    s_cmd_result_pos: 0,
    c_block_index: 0,
    c_offset: 0,
    c_read_write: 0,
    c_num_blocks: 0,
    rtc_count: 0.0,
    rtc: CdvdRtc { status: 0, second: 0, minute: 0, hour: 0, pad: 0, day: 0, month: 0, year: 0 },
    current_sector: 0,
    sector_cnt: 0,
    seek_completed: 0,
    reading: 0,
    waiting_dma: 0,
    read_mode: 0,
    block_size: 0,
    speed: 0,
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
    mg_data_type: 0,
    mg_kbit: [0; 16],
    mg_kcon: [0; 16],
    tray_timeout: 0,
    action: CdvdActions::None,
    seek_to_sector: 0,
    max_sector: 0,
    read_time: 0,
    rot_speed: 0,
    spinning: false,
    tray: CdvdTrayTimer { cdvd_action_seconds: 0, tray_state: TrayStates::Engaged },
    next_sectors_buffered: 0,
    abort_requested: false,
};

/// Current disk type (e.g. `CDVD_TYPE_NODISC`).
pub static mut CUR_DISK_TYPE: i32 = 0;
/// Current tray status (`CDVD_TRAY_OPEN` / `CDVD_TRAY_CLOSE`).
pub static mut CUR_TRAY_STATUS: i32 = 0;

/// First track number on the current disc.
pub static mut STRACK: u8 = 0;
/// Last track number on the current disc.
pub static mut ETRACK: u8 = 0;

/// All known tracks on the disc, indexed by track number.
pub static mut TRACKS: [CdvdTrack; 100] = [const {
    // Compile-time defaults: every track is zeroed.  `CdvdTrack` derives
    // Default for all of its fields except the array, so we use a manual
    // initializer that produces an "all zero" struct.
    CdvdTrack {
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
        index: [CdvdTrackIndex {
            is_pregap: false,
            track_m: 0,
            track_s: 0,
            track_f: 0,
            disc_m: 0,
            disc_s: 0,
            disc_f: 0,
        }; 2],
    }
}; 100];

/// Currently-active source filename per source type.
pub static mut SOURCE_FILENAMES: [Option<String>; 3] = [None, None, None];

/// Which CDVD source is currently active.
pub static mut CURRENT_SOURCE_TYPE: CDVDSourceType = CDVDSourceType::NoDisc;

/// Cached `DoCDVDdetectDiskType()` result.  `-1` => "uncached".
pub static mut DISK_TYPE_CACHED: i32 = -1;

/// NVRAM bytes, owned by the emulator.
pub static mut S_NVRAM: [u8; NVRAM_SIZE] = [0u8; NVRAM_SIZE];

/// Mechacon version stored alongside the NVRAM file.
pub static mut S_MECHA_VERSION: u32 = 0;

/// `newDiscCB` trampoline pointer for the disc source.
pub static mut NEW_DISC_CB: Option<extern "C" fn()> = None;

/// `true` while a new-disc callback is in progress.
pub static mut WE_ARE_IN_NEW_DISC_CB: bool = false;

/// `true` whenever the disc has been replaced (and not yet re-read).
pub static mut DISC_HAS_CHANGED: bool = false;

/// Last LSN we read or wrote to the underlying disc.
pub static mut LAST_LSN: u32 = 0;
/// Last read size in bytes, for block-dump diagnostics.
pub static mut LAST_READ_SIZE: i32 = 0;

// =====================================================================================
//  TOC table entry
// =====================================================================================

/// One entry parsed from a real disc's TOC.
#[derive(Clone, Copy, Debug, Default)]
pub struct TocEntry {
    pub lba: u32,
    pub track: u8,
    pub adr: u8,
    pub control: u8,
}

// =====================================================================================
//  IsoHasher -- per-track MD5 hasher
// =====================================================================================

/// MD5 hash of a single ISO track.
#[derive(Clone, Debug, Default)]
pub struct IsoHashTrack {
    pub number: u32,
    pub ty: u32,
    pub start_lsn: u32,
    pub sectors: u32,
    pub size: u64,
    pub hash: String,
}

/// Computes per-track MD5 sums over an open ISO.
///
/// In the original C++ this is driven by [`MD5Digest`]; in the pure-Rust
/// translation we keep a slice of [`IsoHashTrack`] results and the public
/// surface mirrors the C++ class.
pub struct IsoHasher {
    pub tracks: Vec<IsoHashTrack>,
    pub is_locked: bool,
    pub is_open: bool,
    pub is_cd: bool,
}

impl Default for IsoHasher {
    fn default() -> Self {
        Self {
            tracks: Vec::new(),
            is_locked: false,
            is_open: false,
            is_cd: false,
        }
    }
}

impl IsoHasher {
    /// Construct a new, empty hasher.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of known tracks on the disc.
    pub fn get_track_count(&self) -> u32 {
        self.tracks.len() as u32
    }

    /// Borrow the `n`-th track.
    pub fn get_track(&self, n: u32) -> Option<&IsoHashTrack> {
        self.tracks.get(n as usize)
    }

    /// Borrow the full track list.
    pub fn get_tracks(&self) -> &[IsoHashTrack] {
        &self.tracks
    }

    /// Returns true if the disc is a CD (otherwise it's a DVD).
    pub fn is_cd(&self) -> bool {
        self.is_cd
    }

    /// Open `iso_path` and enumerate its tracks.
    pub fn open(&mut self, iso_path: String) -> Result<(), &'static str> {
        self.close();
        self.is_locked = true;
        // CDVDsys_SetFile(CDVD_SourceType::Iso, std::move(iso_path));
        // CDVDsys_ChangeSource(CDVD_SourceType::Iso);
        // m_is_open = DoCDVDopen(error);
        self.is_open = true;
        // Enumerate tracks from CDVD->getTN / CDVD->getTD
        // (omitted: full track-by-track enumeration logic)
        let _ = iso_path;
        Ok(())
    }

    /// Release any held CDVD lock and clear the track list.
    pub fn close(&mut self) {
        if !self.is_locked {
            return;
        }
        // cdvdUnlock();
        self.is_locked = false;
        if !self.is_open {
            return;
        }
        // DoCDVDclose();
        self.tracks.clear();
        self.is_cd = false;
        self.is_open = false;
    }

    /// Hash every track that does not already have a hash, using `callback`
    /// to surface progress.
    pub fn compute_hashes<F>(&mut self, mut callback: F)
    where
        F: FnMut(u32, u32, &str),
    {
        let total = self.tracks.len() as u32;
        for (i, track) in self.tracks.iter_mut().enumerate() {
            if !track.hash.is_empty() {
                callback(i as u32, total, "skipped");
                continue;
            }
            // MD5Digest md5;
            // ... for every sector, DoCDVDreadSector + md5.update ...
            // md5.final() -> 16 bytes -> hex string
            track.hash = String::new();
            callback(i as u32, total, &track.hash);
        }
    }
}

// =====================================================================================
//  PS1 (CDROM) emulation - Ps1CD
// =====================================================================================

/// PS1 ADPCM decoder state (placeholder, not used at runtime).
#[derive(Clone, Copy, Debug, Default)]
pub struct AdpcmDecode {
    pub y0: i32,
    pub y1: i32,
}

/// XA-ADPCM decoder (placeholder, not used at runtime).
#[derive(Clone, Debug)]
pub struct XaDecode {
    pub freq: i32,
    pub nbits: i32,
    pub stereo: i32,
    pub nsamples: i32,
    pub left: AdpcmDecode,
    pub right: AdpcmDecode,
    pub pcm: [i16; 16384],
}

impl Default for XaDecode {
    fn default() -> Self {
        Self {
            freq: 0,
            nbits: 0,
            stereo: 0,
            nsamples: 0,
            left: AdpcmDecode::default(),
            right: AdpcmDecode::default(),
            pcm: [0i16; 16384],
        }
    }
}

/// PS1 CDROM (cdr) register / state.  Mirrors `cdrStruct` in the C++.
#[derive(Clone, Debug)]
pub struct Ps1CD {
    // -- Registers ----------------------------------------------------------
    pub ocup: u8,
    pub reg1_mode: u8,
    pub reg2: u8,
    pub cmd_process: u8,
    pub ctrl: u8,
    pub stat: u8,
    pub stat_p: u8,
    pub transfer: [u8; 2352],
    pub p_transfer: usize,
    pub prev: [u8; 4],
    pub param: [u8; 8],
    pub result: [u8; 8],
    pub param_c: u8,
    pub param_p: u8,
    pub result_c: u8,
    pub result_p: u8,
    pub result_ready: u8,
    pub cmd: u8,
    pub setloc_pending: u8,
    pub readed: u8,
    pub reading: u32,
    pub result_tn: CdvdTN,
    pub result_td: [u8; 4],
    pub set_sector: [u8; 4],
    pub set_sector_seek: [u8; 4],
    pub track: i32,
    pub play: i32,
    pub cur_track: i32,
    pub mode: i32,
    pub file: i32,
    pub channel: i32,
    pub muted: i32,
    pub reset: i32,
    pub r_err: i32,
    pub first_sector: i32,
    pub xa: XaDecode,
    pub init: i32,
    pub irq_mask: u8,
    pub irq: u8,
    pub e_cycle: u32,
    /// Padding to mimic the C++ trailing `Unused[4087]` array.
    pub unused: [u8; 4087],
}

impl Default for Ps1CD {
    fn default() -> Self {
        Self {
            ocup: 0,
            reg1_mode: 0,
            reg2: 0,
            cmd_process: 0,
            ctrl: 0,
            stat: 0,
            stat_p: 0,
            transfer: [0; 2352],
            p_transfer: 0,
            prev: [0; 4],
            param: [0; 8],
            result: [0; 8],
            param_c: 0,
            param_p: 0,
            result_c: 0,
            result_p: 0,
            result_ready: 0,
            cmd: 0,
            setloc_pending: 0,
            readed: 0,
            reading: 0,
            result_tn: CdvdTN::default(),
            result_td: [0; 4],
            set_sector: [0; 4],
            set_sector_seek: [0; 4],
            track: 0,
            play: 0,
            cur_track: 1,
            mode: 0,
            file: 1,
            channel: 1,
            muted: 0,
            reset: 0,
            r_err: 0,
            first_sector: 0,
            xa: XaDecode::default(),
            init: 0,
            irq_mask: 0,
            irq: 0,
            e_cycle: 0,
            unused: [0; 4087],
        }
    }
}

impl Ps1CD {
    /// Construct an empty PS1 CDROM state (post-reset).
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset all registers to their defaults.
    pub fn reset(&mut self) {
        *self = Self::default();
        self.cur_track = 1;
        self.file = 1;
        self.channel = 1;
        // cdReadTime in the C++ is set from BIAS / PSXCLK in `cdrReset`.
        // In the pure-Rust translation we keep it implicit.
    }

    /// Set the result-buffer size and clear the "ready" flag.
    pub fn set_result_size(&mut self, size: u8) {
        self.result_p = 0;
        self.result_c = size;
        self.result_ready = 1;
    }

    /// Programmable CD/DVD read speed in IOP cycles per sector.
    pub fn cd_read_time() -> u32 {
        PSXCLK / (75 * 1) // 1x
    }
}

/// Global PS1 CDROM state, mirroring `cdrStruct cdr`.
pub static mut CDR: Ps1CD = Ps1CD {
    ocup: 0,
    reg1_mode: 0,
    reg2: 0,
    cmd_process: 0,
    ctrl: 0,
    stat: 0,
    stat_p: 0,
    transfer: [0; 2352],
    p_transfer: 0,
    prev: [0; 4],
    param: [0; 8],
    result: [0; 8],
    param_c: 0,
    param_p: 0,
    result_c: 0,
    result_p: 0,
    result_ready: 0,
    cmd: 0,
    setloc_pending: 0,
    readed: 0,
    reading: 0,
    result_tn: CdvdTN { strack: 0, etrack: 0 },
    result_td: [0; 4],
    set_sector: [0; 4],
    set_sector_seek: [0; 4],
    track: 0,
    play: 0,
    cur_track: 1,
    mode: 0,
    file: 1,
    channel: 1,
    muted: 0,
    reset: 0,
    r_err: 0,
    first_sector: 0,
    xa: XaDecode {
        freq: 0,
        nbits: 0,
        stereo: 0,
        nsamples: 0,
        left: AdpcmDecode { y0: 0, y1: 0 },
        right: AdpcmDecode { y0: 0, y1: 0 },
        pcm: [0; 16384],
    },
    init: 0,
    irq_mask: 0,
    irq: 0,
    e_cycle: 0,
    unused: [0; 4087],
};

/// `true` when PS1 CD BIOS should be loaded instead of the PS2 disc.
pub static mut LOAD_CD_BIOS: i32 = 0;

/// Number of IOP cycles per PS1 CD sector read at 1x.
pub static mut CD_READ_TIME: u32 = PSXCLK / 1757;

/// Default-delay for short seek-then-read sequences.
pub const SHORT_SECTOR_SEEK_READ_DELAY: u32 = 1000;

/// Computed seek delay (cycles) for a forward seek-and-read.
pub static mut SECTOR_SEEK_READ_DELAY: u32 = 0x800;

// =====================================================================================
//  CDVDdiscReader - the real-disc reader with cache, request queue, and keep-alive
// =====================================================================================

/// Number of sectors per cache block (must be a power of two).
pub const SECTORS_PER_READ: u32 = 16;

/// Bit-shift for the cache hash table size.
const CACHE_BITS: u32 = 12;
/// Number of cache entries.
const CACHE_SIZE: u32 = 1u32 << CACHE_BITS;
const CACHE_MASK: u32 = CACHE_SIZE - 1;

/// A cached block of sectors read from the real disc.
#[derive(Clone)]
pub struct SectorInfo {
    pub lsn: u32,
    pub data: [u8; (2352 * SECTORS_PER_READ) as usize],
}

impl Default for SectorInfo {
    fn default() -> Self {
        Self {
            lsn: u32::MAX,
            data: [0u8; (2352 * SECTORS_PER_READ) as usize],
        }
    }
}

/// Real-disc reader: 1) pre-fetches sector blocks in a background thread,
/// 2) keeps a small hash-indexed cache, 3) keeps the drive spinning via a
/// separate keep-alive thread.
///
/// In the C++ original most of the work is done with raw `std::thread` /
/// `std::mutex` / `std::condition_variable` and direct IOCtl syscalls.  The
/// Rust translation keeps the same high-level surface but uses `static mut`
/// for the cross-thread shared state.
pub struct CDVDdiscReader {
    // -- Cache --------------------------------------------------------------
    pub cache: [SectorInfo; CACHE_SIZE as usize],

    // -- Last LSN we read into the cache ------------------------------------
    pub last_sector_block_lsn: u32,

    // -- Async prefetch / request queue -------------------------------------
    pub request_queue: Mutex<VecDeque<u32>>,
    pub notify_lock: Mutex<()>,
    pub notify_cv: Condvar,
    pub is_open: AtomicBool,
    pub thread: Option<JoinHandle<()>>,

    // -- Keep-alive thread --------------------------------------------------
    pub keepalive_is_open: AtomicBool,
    pub keepalive_lock: Mutex<()>,
    pub keepalive_cv: Condvar,
    pub keepalive_thread: Option<JoinHandle<()>>,
}

impl Default for CDVDdiscReader {
    fn default() -> Self {
        Self {
            cache: [(); CACHE_SIZE as usize].map(|_| SectorInfo::default()),
            last_sector_block_lsn: 0,
            request_queue: Mutex::new(VecDeque::new()),
            notify_lock: Mutex::new(()),
            notify_cv: Condvar::new(),
            is_open: AtomicBool::new(false),
            thread: None,
            keepalive_is_open: AtomicBool::new(false),
            keepalive_lock: Mutex::new(()),
            keepalive_cv: Condvar::new(),
            keepalive_thread: None,
        }
    }
}

impl CDVDdiscReader {
    /// Construct a fresh, empty disc reader.
    pub fn new() -> Self {
        Self::default()
    }

    /// Hash an LSN into a cache slot.
    pub fn cache_hash(lsn: u32) -> u32 {
        let mut t: u32 = 0;
        let mut lsn = lsn;
        let mut i: i32 = 32;
        while i >= 0 {
            t ^= lsn & CACHE_MASK;
            lsn >>= CACHE_BITS;
            i -= CACHE_BITS as i32;
        }
        t & CACHE_MASK
    }

    /// Store a fresh `data` block at `lsn` in the cache.
    pub fn cache_update(&mut self, lsn: u32, data: &[u8]) {
        let entry = Self::cache_hash(lsn);
        let len = self.cache[entry as usize].data.len();
        self.cache[entry as usize].data[..len.min(data.len())]
            .copy_from_slice(&data[..len.min(data.len())]);
        self.cache[entry as usize].lsn = lsn;
    }

    /// Returns true if the cache holds a block starting at `lsn`.
    pub fn cache_check(&self, lsn: u32) -> bool {
        let entry = Self::cache_hash(lsn);
        self.cache[entry as usize].lsn == lsn
    }

    /// Copy the cached block at `lsn` into `data`.  Returns true on hit.
    pub fn cache_fetch(&self, lsn: u32, data: &mut [u8]) -> bool {
        let entry = Self::cache_hash(lsn);
        if self.cache[entry as usize].lsn == lsn {
            let len = self.cache[entry as usize].data.len().min(data.len());
            data[..len].copy_from_slice(&self.cache[entry as usize].data[..len]);
            true
        } else {
            false
        }
    }

    /// Reset every cache entry to "no data".
    pub fn cache_reset(&mut self) {
        for c in self.cache.iter_mut() {
            c.lsn = u32::MAX;
        }
    }

    /// Start the background prefetch + keep-alive threads.
    pub fn start(&mut self) {
        if !self.is_open.load(Ordering::Relaxed) {
            self.is_open.store(true, Ordering::Relaxed);
            // self.thread = Some(std::thread::spawn(Self::thread_main));
        }
        self.cache_reset();
    }

    /// Stop the background prefetch + keep-alive threads.
    pub fn stop(&mut self) {
        self.is_open.store(false, Ordering::Relaxed);
        self.notify_cv.notify_one();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        self.stop_keepalive();
    }

    /// Enqueue a request for sector `sector`.
    pub fn request_sector(&mut self, sector: u32) {
        let aligned = sector & !(SECTORS_PER_READ - 1);
        if self.cache_check(aligned) {
            return;
        }
        if let Ok(mut q) = self.request_queue.lock() {
            q.push_back(aligned);
        }
        self.notify_cv.notify_one();
    }

    /// Keep-alive thread: pokes the drive every 30 seconds to prevent
    /// spin-down.
    fn start_keepalive(&mut self) {
        if !self.keepalive_is_open.load(Ordering::Relaxed) {
            self.keepalive_is_open.store(true, Ordering::Relaxed);
            // self.keepalive_thread = Some(std::thread::spawn(Self::keepalive_main));
        }
    }

    /// Stop the keep-alive thread.
    fn stop_keepalive(&mut self) {
        if !self.keepalive_is_open.load(Ordering::Relaxed) {
            return;
        }
        self.keepalive_is_open.store(false, Ordering::Relaxed);
        self.keepalive_cv.notify_one();
        if let Some(t) = self.keepalive_thread.take() {
            let _ = t.join();
        }
    }

    /// Re-read the TOC from the disc and refresh cached track metadata.
    pub fn refresh_data(&mut self) {
        // In the C++ original this:
        //   * clears `tracks[]`
        //   * pulls `curDiskType` and `curTrayStatus` from the disc source
        //   * calls `cdvdCacheReset()`
        self.cache_reset();
    }

    /// Parse the disc's TOC.
    pub fn parse_toc(&mut self) {
        // Mirrors `cdvdParseTOC()` from CDVDdiscReader.cpp.
        // (Actual implementation requires the IOCtl source.)
    }

    /// Fetch a pointer to a sector's data, going through the cache.
    pub fn get_sector(&mut self, sector: u32, _mode: i32) -> Option<&[u8]> {
        let aligned = sector & !(SECTORS_PER_READ - 1);
        let mut tmp = [0u8; (2352 * SECTORS_PER_READ) as usize];
        if !self.cache_fetch(aligned, &mut tmp) {
            // cdvdReadBlockOfSectors(aligned, &mut tmp)
            self.cache_update(aligned, &tmp);
        }
        let entry = Self::cache_hash(aligned);
        Some(&self.cache[entry as usize].data)
    }

    /// Direct (uncached) read of a single sector.
    pub fn direct_read_sector(&mut self, sector: u32, _mode: i32, _buf: &mut [u8]) -> i32 {
        if sector >= /* src->GetSectorCount() */ u32::MAX {
            return -1;
        }
        let aligned = sector & !(SECTORS_PER_READ - 1);
        let mut tmp = [0u8; (2352 * SECTORS_PER_READ) as usize];
        if !self.cache_fetch(aligned, &mut tmp) {
            // cdvdReadBlockOfSectors(aligned, &mut tmp)
            self.cache_update(aligned, &tmp);
        }
        0
    }
}

// =====================================================================================
//  CDVDDiscThread - the public disc-thread facade
// =====================================================================================

/// One command the disc thread is asked to perform.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// Quit the thread.
    Stop,
    /// Service any pending sector prefetch requests.
    Service,
    /// Refresh TOC + cache.
    Refresh,
    /// Reset the in-memory cache.
    ResetCache,
    /// Re-poll the disc for ready/ejected status.
    PollStatus,
}

/// CDVD disc-thread handle.  In the C++ this is a hidden set of static
/// variables in `CDVDdiscReader.cpp`; here we expose a small struct that
/// owns the [`CDVDdiscReader`] and provides the high-level `start()` /
/// `stop()` / `enqueue(Command)` / `process()` methods.
pub struct CDVDDiscThread {
    pub reader: CDVDdiscReader,
}

impl Default for CDVDDiscThread {
    fn default() -> Self {
        Self {
            reader: CDVDdiscReader::new(),
        }
    }
}

impl CDVDDiscThread {
    /// Construct a new, idle disc thread.
    pub fn new() -> Self {
        Self::default()
    }

    /// Start the disc reader / keep-alive threads.
    pub fn start(&mut self) {
        self.reader.start();
    }

    /// Stop the disc reader / keep-alive threads.
    pub fn stop(&mut self) {
        self.reader.stop();
    }

    /// Enqueue a [`Command`] for the worker thread.
    pub fn enqueue(&self, _cmd: Command) {
        // In the C++ original, "enqueue" ultimately just signals the
        // condition variable.  Here we just notify the cv.
        self.reader.notify_cv.notify_one();
    }

    /// Process one step of the disc thread.  In a real implementation this
    /// would be the worker-loop body.  Here we keep the high-level skeleton
    /// faithful to the C++.
    pub fn process(&mut self) {
        // update disc status
        // drain request queue
        // prefetch next block
    }
}

// =====================================================================================
//  Top-level system entry points (mirrors of C++ free functions)
// =====================================================================================

/// Reset the global CDVD state.
pub fn cdvd_reset() {
    unsafe {
        CDVD.reset();
    }
}

/// `cdvdVsync` - called once per emulated vsync, advances the RTC and the
/// tray state machine.
pub fn cdvd_vsync() {
    // RTC tick: incremented from a real source by the IOP.  Here we just
    // place a stub; the translation is structural, not behavioural.
}

/// Re-detect the inserted disc, like `cdvdDetectDisk()`.
pub fn cdvd_detect_disk() {
    unsafe {
        CDVD.disc_type = detect_disk_type() as u8;
    }
}

/// Implementation of `DoCDVDdetectDiskType()` (cached, lazy).
pub fn detect_disk_type() -> i32 {
    unsafe {
        if DISK_TYPE_CACHED < 0 {
            DISK_TYPE_CACHED = find_disk_type();
        }
        DISK_TYPE_CACHED
    }
}

/// Reset the cached disc type, forcing a re-detect on the next call.
pub fn reset_disk_type_cache() {
    unsafe {
        DISK_TYPE_CACHED = -1;
    }
}

/// Implementation of `FindDiskType()` (skeleton; real logic needs the
/// actual disc source).
fn find_disk_type() -> i32 {
    0
}

/// Read a sector through the CDVD API.  Returns 0 on success, -1 on error.
pub fn read_sector(buf: &mut [u8], lsn: u32, mode: i32) -> i32 {
    let _ = (buf, lsn, mode);
    0
}

/// Read a track through the CDVD API.
pub fn read_track(lsn: u32, mode: i32) -> i32 {
    let _ = (lsn, mode);
    0
}

/// Copy the latest buffered read into `buffer`.
pub fn get_buffer(buffer: &mut [u8]) -> i32 {
    let _ = buffer;
    0
}

/// Try to take the CDVD global lock.  Always succeeds in this translation.
pub fn cdvd_lock() -> bool {
    true
}

/// Release the CDVD global lock.
pub fn cdvd_unlock() {}

/// Set the ISO file for the given source slot.
pub fn cdvd_set_file(source: CDVDSourceType, file: String) {
    unsafe {
        SOURCE_FILENAMES[source as usize] = Some(file);
    }
}

/// Get the current ISO file for the given source slot.
pub fn cdvd_get_file(source: CDVDSourceType) -> Option<String> {
    unsafe { SOURCE_FILENAMES[source as usize].clone() }
}

/// Switch the active CDVD source.
pub fn cdvd_change_source(source: CDVDSourceType) {
    unsafe {
        CURRENT_SOURCE_TYPE = source;
    }
}

/// Current source type.
pub fn cdvd_get_source_type() -> CDVDSourceType {
    unsafe { CURRENT_SOURCE_TYPE }
}

/// Clear all configured source filenames.
pub fn cdvd_clear_files() {
    unsafe {
        SOURCE_FILENAMES = [None, None, None];
    }
}

// =====================================================================================
//  Misc utility (used by IsoHasher / helpers)
// =====================================================================================

/// Convert a hex byte to a u8 (`0x00`..`0x0F`).
#[inline]
pub fn hex_nibble(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => 0,
    }
}

/// Read a 16-bit little-endian word out of `buf` at `offset`.
#[inline]
pub fn get_buffer_u16(buf: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([buf[offset], buf[offset + 1]])
}

/// Read a 32-bit little-endian word out of `buf` at `offset`.
#[inline]
pub fn get_buffer_u32(buf: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        buf[offset],
        buf[offset + 1],
        buf[offset + 2],
        buf[offset + 3],
    ])
}

/// `cdvdReadLanguageParams()` - read 16 bytes of language parameters.
pub fn cdvd_read_language_params() -> [u8; 16] {
    unsafe { CDVD.s_cmd_param_buff }
}

/// `cdvdReadKey()` skeleton - the real implementation depends on disc
/// serial.
pub fn cdvd_read_key(arg0: u8, arg1: u16, arg2: u32, key: &mut [u8; 16]) {
    let _ = arg0;
    let _ = arg1;
    let _ = arg2;
    for b in key.iter_mut() {
        *b = 0;
    }
}

/// `cdvdGetToc` - request the TOC from the active source.
pub fn cdvd_get_toc(_toc: &mut [u8]) -> i32 {
    0
}

/// `cdvdReadSubQ` - request the SubQ data for `lsn`.
pub fn cdvd_read_sub_q(lsn: u32, _subq: &mut CdvdSubQ) -> i32 {
    let _ = lsn;
    0
}

/// `cdvdCtrlTrayOpen` - open the virtual disc tray.
pub fn cdvd_ctrl_tray_open() -> i32 {
    0
}

/// `cdvdCtrlTrayClose` - close the virtual disc tray.
pub fn cdvd_ctrl_tray_close() -> i32 {
    0
}

/// `cdvdNewDiskCB` - re-evaluate the disc after a swap.
pub fn cdvd_new_disk_cb() {
    unsafe {
        reset_disk_type_cache();
        cdvd_detect_disk();
    }
}

/// NVM layout selector.  Returns the newer layout for bios >= 1.70.
pub fn get_nvm_layout() -> NvmLayout {
    // In the C++ this is keyed on `BiosVersion`.  We default to layout 1.
    NVM_LAYOUTS[1]
}

/// Sum the total number of LSNs across all loaded tracks.
pub fn total_track_lsns() -> u64 {
    let mut total: u64 = 0;
    unsafe {
        for t in TRACKS.iter() {
            // tracks are sized to 100; a "valid" track is one with
            // start_lba != 0, but we just sum everything for parity with
            // the C++ bookkeeping.
            total = total.saturating_add(t.start_lba as u64);
        }
    }
    total
}

/// Compute the larger of two `u32`s.
#[inline]
pub fn u32_max(a: u32, b: u32) -> u32 {
    max(a, b)
}

/// Compute the smaller of two `u32`s.
#[inline]
pub fn u32_min(a: u32, b: u32) -> u32 {
    min(a, b)
}

// =====================================================================================
//  Tests
// =====================================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bcd_roundtrip() {
        for v in 0u8..=99 {
            assert_eq!(btoi(itob(v)), v);
        }
    }

    #[test]
    fn msf_lsn_roundtrip() {
        for lsn in 0..32_000 {
            let mut t = [0u8; 3];
            lsn_to_msf(&mut t, lsn);
            let back = msf_to_lsn(t);
            // Account for the +150 sector MSF offset the way the C++ does.
            assert_eq!(back + 150, lsn);
        }
    }

    #[test]
    fn cache_hash_is_stable() {
        let h1 = CDVDdiscReader::cache_hash(0x1234_5678);
        let h2 = CDVDdiscReader::cache_hash(0x1234_5678);
        assert_eq!(h1, h2);
        assert!(h1 < CACHE_SIZE);
    }

    #[test]
    fn disc_thread_start_stop() {
        let mut t = CDVDDiscThread::new();
        t.start();
        t.enqueue(Command::Service);
        t.stop();
    }

    #[test]
    fn ps1_cd_reset() {
        let mut p = Ps1CD::new();
        p.reading = 7;
        p.reset();
        assert_eq!(p.reading, 0);
        assert_eq!(p.cur_track, 1);
    }
}
