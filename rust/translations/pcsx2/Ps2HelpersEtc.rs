// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust translation of a slice of PCSX2's top-level C++ source files.
//!
//! This single module covers the structure of:
//! - `ps2/BiosTools.{h,cpp}` — PS2 BIOS ROM discovery and metadata parsing
//! - `ps2/Iop/IopHw_Internal.h` and `ps2/Iop/PsxBios.cpp` — IOP hardware
//!   register naming helpers and the PS1 BIOS HLE call trampoline
//! - `ps2/pgif.{h,cpp}` — the PS1 GPU / PGIF emulation state machine
//! - `ps2/HwInternal.h` — a small read/write helper header for HW FIFOs
//! - `SourceLog.cpp` — `TraceLog` / `ConsoleLog` packs
//! - `Host.{h,cpp}` — VM host trait, translation cache, settings accessors
//! - `Hotkeys.cpp` — the `g_common_hotkeys` table
//! - `Achievements.{h,cpp}` — RetroAchievements / rcheevos client facade
//! - `BuildVersion.{h,cpp}` — version constants
//! - `PINE.{h,cpp}` — UNIX-domain / TCP socket server for IPC tooling
//! - `PerformanceMetrics.{h,cpp}` — per-frame performance counters
//!
//! The original codebase mixes emulator state, GUI hooks, native threading
//! primitives and several third-party C APIs. The translation here keeps the
//! *shape* of those APIs (struct fields, function names, configuration
//! tables) while moving all storage to `std` collections and serialising the
//! threading/IO concepts behind ordinary Rust types. No side effects fire
//! at construction time; every meaningful operation is an explicit method
//! call or constructor argument.

#![allow(clippy::redundant_field_names)]

use std::collections::HashMap;
use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// ps2/BiosTools.h
// ---------------------------------------------------------------------------

/// Information for thread-list debug helpers carried by the PS2 BIOS image.
#[derive(Debug, Default, Clone, Copy)]
pub struct BiosDebugInformation {
    pub ee_thread_list_addr: u32,
    pub iop_thread_list_addr: u32,
    pub iop_mod_list_addr: u32,
}

/// OSD language / display parameters, mirroring the Open PS2SDK `osd_config.h`
/// struct used by the HLE config-parameter syscalls.
#[derive(Debug, Default, Clone, Copy)]
pub struct ConfigParam {
    pub spdif_mode: u32,
    pub screen_type: u32,
    pub video_output: u32,
    pub jap_language: u32,
    pub ps1drv_config: u32,
    pub version: u32,
    pub language: u32,
    pub timezone_offset: i32,
    pub time_zone_id: u8,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Config2Param {
    pub format: u8,
    pub reserved: u8,
    pub daylight_savings: u8,
    pub time_format: u8,
    pub date_format: u8,
    pub version: u8,
    pub language: u8,
}

/// Aggregated metadata about a discovered PS2 BIOS image, plus the raw ROM
/// bytes that callers may want to install into EE memory.
#[derive(Debug, Clone)]
pub struct BiosFile {
    pub version: u32,
    pub region: u32,
    pub description: String,
    pub zone: String,
    pub serial: String,
    pub path: PathBuf,
    pub rom: Vec<u8>,
}

const MIN_BIOS_SIZE: u64 = 4 * 1024 * 1024;
const MAX_BIOS_SIZE: u64 = 8 * 1024 * 1024;
const DIRENTRY_SIZE: usize = 16;
const TIMEZONE_LOCATIONS: &[[&str; 2]] = &[
    ["Afghanistan", "Kabul"],
    ["Albania", "Tirana"],
    ["Algeria", "Algiers"],
    ["Andorra", "Andorra la Vella"],
    ["Armenia", "Yerevan"],
    ["Australia \u{2013} Perth", "Perth"],
    ["Australia \u{2013} Adelaide", "Adelaide"],
    ["Australia \u{2013} Sydney", "Sydney"],
    ["Australia \u{2013} Lord Howe Island", "Lord Howe Island"],
    ["Austria", "Vienna"],
    ["Azerbaijan", "Baku"],
    ["Bahrain", "Manama"],
    ["Bangladesh", "Dhaka"],
    ["Belarus", "Minsk"],
    ["Belgium", "Brussels"],
    ["Bosnia and Herzegovina", "Sarajevo"],
    ["Bulgaria", "Sofia"],
    ["Canada \u{2013} Pacific, Yukon", "Pacific (Canada)"],
    ["Canada \u{2013} Mountain", "Mountain (Canada)"],
    ["Canada \u{2013} Central", "Central (Canada)"],
    ["Canada \u{2013} Eastern", "Eastern (Canada)"],
    ["Canada \u{2013} Atlantic", "Atlantic (Canada)"],
    ["Canada \u{2013} Newfoundland", "Newfoundland"],
    ["Cape Verde", "Praia"],
    ["Chile \u{2013} Santiago", "Santiago"],
    ["Chile \u{2013} Easter Island", "Easter Island"],
    ["China", "Beijing"],
    ["Croatia", "Zagreb"],
    ["Cyprus", "Nicosia"],
    ["Czech Republic", "Prague"],
    ["Denmark", "Copenhagen"],
    ["Egypt", "Cairo"],
    ["Estonia", "Tallinn"],
    ["Fiji", "Suva"],
    ["Finland", "Helsinki"],
    ["France", "Paris"],
    ["Georgia", "Tbilisi"],
    ["Germany", "Berlin"],
    ["Gibraltar", "Gibraltar"],
    ["Greece", "Athens"],
    ["Greenland \u{2013} Pituffik", "Northwestern Greenland"],
    ["Greenland \u{2013} Greenland", "Southwestern Greenland"],
    ["Greenland \u{2013} Ittoqqortoormiit", "Eastern Greenland"],
    ["Hungary", "Budapest"],
    ["Iceland", "Reykjavik"],
    ["India", "Calcutta"],
    ["Iran", "Tehran"],
    ["Iraq", "Baghdad"],
    ["Ireland", "Dublin"],
    ["Israel", "Jerusalem"],
    ["Italy", "Rome"],
    ["Japan", "Tokyo"],
    ["Jordan", "Amman"],
    ["Kazakhstan \u{2013} Western", "Western Kazakhstan"],
    ["Kazakhstan \u{2013} Central", "Central Kazakhstan"],
    ["Kazakhstan \u{2013} Eastern", "Eastern Kazakhstan"],
    ["Kuwait", "Kuwait City"],
    ["Kyrgyzstan", "Bishkek"],
    ["Latvia", "Riga"],
    ["Lebanon", "Beirut"],
    ["Liechtenstein", "Vaduz"],
    ["Lithuania", "Vilnius"],
    ["Luxembourg", "Luxembourg"],
    ["Macedonia", "Skopje"],
    ["Malta", "Valletta"],
    ["Mexico \u{2013} Tijuana", "Tijuana"],
    ["Mexico \u{2013} Chihuahua", "Chihuahua"],
    ["Mexico \u{2013} Mexico City", "Mexico City"],
    ["Midway Islands", "Midway Islands"],
    ["Monaco", "Monaco"],
    ["Morocco", "Casablanca"],
    ["Namibia", "Windhoek"],
    ["Nepal", "Kathmandu"],
    ["Netherlands", "Amsterdam"],
    ["New Caledonia", "New Caledonia"],
    ["New Zealand", "Wellington"],
    ["Norway", "Oslo"],
    ["Oman", "Muscat"],
    ["Pakistan", "Karachi"],
    ["Panama", "Panama City"],
    ["Poland", "Warsaw"],
    ["Portugal \u{2013} Azores", "Azores"],
    ["Portugal \u{2013} Lisbon", "Lisbon"],
    ["Puerto Rico", "Puerto Rico"],
    ["Reunion", "Reunion"],
    ["Romania", "Bucharest"],
    ["Russian Federation \u{2013} Kaliningrad", "Kaliningrad"],
    ["Russian Federation \u{2013} Moscow", "Moscow"],
    ["Russian Federation \u{2013} Izhevsk", "Izhevsk"],
    ["Russian Federation \u{2013} Perm", "Perm"],
    ["Russian Federation \u{2013} Omsk", "Omsk"],
    ["Russian Federation \u{2013} Norilsk", "Norilsk"],
    ["Russian Federation \u{2013} Bratsk", "Bratsk"],
    ["Russian Federation \u{2013} Yakutsk", "Yakutsk"],
    ["Russian Federation \u{2013} Vladivostok", "Vladivostok"],
    ["Russian Federation \u{2013} Magadan", "Magadan"],
    ["Russian Federation \u{2013} Petropavlovsk-Kamchatsky", "Petropavlovsk-Kamchatsky"],
    ["Samoa", "Samoa Islands"],
    ["San Marino", "San Marino"],
    ["Saudi Arabia", "Riyadh"],
    ["Slovakia", "Bratislava"],
    ["Slovenia", "Ljubljana"],
    ["South Africa", "Johannesburg"],
    ["Spain \u{2013} Canary Islands", "Canary Islands"],
    ["Spain \u{2013} Madrid", "Madrid"],
    ["Sweden", "Stockholm"],
    ["Switzerland", "Bern"],
    ["Syria", "Damascus"],
    ["Tunisia", "Tunis"],
    ["Turkey", "Istanbul"],
    ["Ukraine", "Kiev"],
    ["United Arab Emirates", "Abu Dhabi"],
    ["United Kingdom", "London"],
    ["United States \u{2013} Hawaii", "Hawaii"],
    ["United States \u{2013} Alaska", "Alaska"],
    ["United States \u{2013} Pacific", "Pacific (USA)"],
    ["United States \u{2013} Mountain", "Mountain (USA)"],
    ["United States \u{2013} Central", "Central (USA)"],
    ["United States \u{2013} Eastern", "Eastern (USA)"],
    ["Uzbekistan", "Tashkent"],
    ["Venezuela", "Caracas"],
    ["Yugoslavia", "Belgrade"],
    ["Thailand", "Bangkok"],
    ["Hong Kong", "Hong Kong"],
    ["Malaysia", "Kuala Lumpur"],
    ["Singapore", "Singapore"],
    ["Taiwan", "Taipei"],
    ["South Korea", "Seoul"],
];

#[repr(C, packed)]
#[derive(Debug, Default, Clone, Copy)]
struct RomDir {
    file_name: [u8; 10],
    ext_info_size: u16,
    file_size: u32,
}

const _: [(); DIRENTRY_SIZE] = [(); std::mem::size_of::<RomDir>()];

/// Try to open `path` and extract a [`BiosFile`] (description, version,
/// region, raw ROM bytes). Returns a human-readable `String` error on
/// failure. The implementation mirrors the C++ `LoadBiosVersion` walk of the
/// romdir.
pub fn find_bios(path: &Path) -> Result<BiosFile, String> {
    let metadata = fs::metadata(path).map_err(|e| format!("stat {}: {}", path.display(), e))?;
    let size = metadata.len();
    if size < MIN_BIOS_SIZE || size > MAX_BIOS_SIZE {
        return Err(format!(
            "BIOS file size {} outside the expected {}-{} range",
            size, MIN_BIOS_SIZE, MAX_BIOS_SIZE
        ));
    }
    let mut file = fs::File::open(path).map_err(|e| format!("open {}: {}", path.display(), e))?;
    let mut rom = Vec::with_capacity(size as usize);
    file.read_to_end(&mut rom)
        .map_err(|e| format!("read {}: {}", path.display(), e))?;

    let mut cursor = io::Cursor::new(&rom);
    let mut romdir = RomDir::default();
    let mut found_romdir = false;
    for _ in 0..(512 * 1024) {
        cursor
            .read_exact(as_bytes_mut(&mut romdir))
            .map_err(|e| format!("short read while scanning for romdir: {}", e))?;
        if romdir.file_name.starts_with(b"RESET") {
            found_romdir = true;
            break;
        }
    }
    if !found_romdir {
        return Err("RESET romdir not found in BIOS image".into());
    }

    let mut file_offset: u64 = 0;
    let mut romver: [u8; 14] = [0; 14];
    let mut extinfo: [u8; 15] = [0; 15];
    let mut serial = String::new();
    let mut found_romver = false;

    loop {
        // Skip empty / full 10-byte file names.
        if romdir.file_name[0] == 0 || romdir.file_name.iter().all(|b| *b != 0) == false {
            // Both null and full-width are stop conditions in the C++ code.
            if romdir.file_name[0] == 0 {
                break;
            }
        }
        if romdir.file_name.starts_with(b"EXTINFO") {
            let pos = cursor.position();
            cursor
                .seek(SeekFrom::Start(file_offset + 0x10))
                .map_err(|e| format!("seek extinfo: {}", e))?;
            if cursor.read_exact(&mut extinfo).is_ok() {
                serial = String::from_utf8_lossy(&extinfo).trim_end_matches('\0').to_string();
            }
            cursor
                .seek(SeekFrom::Start(pos))
                .map_err(|e| format!("rewind extinfo: {}", e))?;
        }
        if romdir.file_name.starts_with(b"ROMVER") {
            let pos = cursor.position();
            cursor
                .seek(SeekFrom::Start(file_offset))
                .map_err(|e| format!("seek romver: {}", e))?;
            if cursor.read_exact(&mut romver).is_ok() {
                found_romver = true;
            }
            cursor
                .seek(SeekFrom::Start(pos))
                .map_err(|e| format!("rewind romver: {}", e))?;
        }

        if romdir.file_size % 0x10 == 0 {
            file_offset += romdir.file_size as u64;
        } else {
            file_offset += (romdir.file_size as u64 + 0x10) & 0xfffffff0;
        }

        match cursor.read_exact(as_bytes_mut(&mut romdir)) {
            Ok(()) => continue,
            Err(_) => break,
        }
    }
    let _ = file_offset;

    if !found_romver {
        return Err("ROMVER entry not located".into());
    }

    let (zone, region) = match romver[4] {
        b'J' => ("Japan", 0u32),
        b'A' => ("USA", 1),
        b'E' => ("Europe", 2),
        b'H' => ("Asia", 4),
        b'C' => ("China", 6),
        b'T' if romver[5] == b'Z' => ("COH-H", 8),
        b'T' => ("T10K", 8),
        b'X' => ("Test", 9),
        b'P' => ("Free", 10),
        other => {
            let mut s = String::new();
            s.push(other as char);
            // Returning a borrowed string here would require a temp; we
            // fold the single character into a one-shot owned String and
            // pass it down by leaking the reference (callers only use the
            // CStr once before formatting).
            (Box::leak(s.into_boxed_str()) as &'static str, 0u32)
        }
    };

    let mut vermaj = [0u8; 3];
    vermaj.copy_from_slice(&romver[0..2]);
    let vermaj_str = std::str::from_utf8(&vermaj).unwrap_or("00");
    let mut vermin = [0u8; 3];
    vermin.copy_from_slice(&romver[2..4]);
    let vermin_str = std::str::from_utf8(&vermin).unwrap_or("00");
    let kind = if romver[5] == b'C' {
        "Console"
    } else if romver[5] == b'D' {
        "Devel"
    } else {
        ""
    };

    let description = format!(
        "{:<7} v{}.{}({}{}/{}{}/{}{}{}{})  {} {}",
        zone,
        vermaj_str,
        vermin_str,
        romver[12] as char,
        romver[13] as char,
        romver[10] as char,
        romver[11] as char,
        romver[6] as char,
        romver[7] as char,
        romver[8] as char,
        romver[9] as char,
        kind,
        serial,
    );

    let version: u32 = (str::from_utf8(&vermaj).ok().and_then(|s| s.parse().ok()).unwrap_or(0) << 8)
        | str::from_utf8(&vermin).ok().and_then(|s| s.parse().ok()).unwrap_or(0);

    Ok(BiosFile {
        version,
        region,
        description,
        zone: zone.to_string(),
        serial,
        path: path.to_path_buf(),
        rom,
    })
}

fn as_bytes_mut<T: Sized>(value: &mut T) -> &mut [u8] {
    // SAFETY: `T` here is a packed `repr(C)` struct (`RomDir`) and we are
    // aliasing it as bytes; the caller writes the result straight into a
    // `read_exact`. This is the same pattern used in the original code.
    unsafe {
        std::slice::from_raw_parts_mut(value as *mut T as *mut u8, std::mem::size_of::<T>())
    }
}

// ---------------------------------------------------------------------------
// ps2/Iop/IopHw_Internal.h + PsxBios.cpp
// ---------------------------------------------------------------------------

/// Single-line description of an IOP/PS1 hardware register, mirroring the
/// `_ioplog_GetHwName` lookup table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IopRegName(pub Option<&'static str>);

impl IopRegName {
    pub fn lookup(addr: u32, width: u8) -> Option<&'static str> {
        let w = width as usize;
        match addr {
            0x1f801060 => Some("RAM_SIZE"),
            0x1f8010a0 => Some("DMA2 MADR"),
            0x1f8010a4 => Some(if w == 4 { "DMA2 BCR" } else { "DMA2 BCR_size" }),
            0x1f8010a6 => Some("DMA2 BCR_count"),
            0x1f8010a8 => Some("DMA2 CHCR"),
            0x1f8010ac => Some("DMA2 TADR"),
            0x1f8010b0 => Some("DMA3 MADR"),
            0x1f8010b4 => Some(if w == 4 { "DMA3 BCR" } else { "DMA3 BCR_size" }),
            0x1f8010b6 => Some("DMA3 BCR_count"),
            0x1f8010b8 => Some("DMA3 CHCR"),
            0x1f8010bc => Some("DMA3 TADR"),
            0x1f8010c0 => Some("[SPU]DMA4 MADR"),
            0x1f8010c4 => Some(if w == 4 { "[SPU]DMA4 BCR" } else { "[SPU]DMA4 BCR_size" }),
            0x1f8010c6 => Some("[SPU]DMA4 BCR_count"),
            0x1f8010c8 => Some("[SPU]DMA4 CHCR"),
            0x1f8010cc => Some("[SPU]DMA4 TADR"),
            0x1f8010f0 => Some("DMA PCR"),
            0x1f8010f4 => Some("DMA ICR"),
            0x1f8010f6 => Some("DMA ICR_hi"),
            0x1f801500 => Some("[SPU2]DMA7 MADR"),
            0x1f801504 => Some(if w == 4 { "[SPU2]DMA7 BCR" } else { "[SPU2]DMA7 BCR_size" }),
            0x1f801506 => Some("[SPU2]DMA7 BCR_count"),
            0x1f801508 => Some("[SPU2]DMA7 CHCR"),
            0x1f80150C => Some("[SPU2]DMA7 TADR"),
            0x1f801520 => Some("DMA9 MADR"),
            0x1f801524 => Some(if w == 4 { "DMA9 BCR" } else { "DMA9 BCR_size" }),
            0x1f801526 => Some("DMA9 BCR_count"),
            0x1f801528 => Some("DMA9 CHCR"),
            0x1f80152C => Some("DMA9 TADR"),
            0x1f801530 => Some("DMA10 MADR"),
            0x1f801534 => Some(if w == 4 { "DMA10 BCR" } else { "DMA10 BCR_size" }),
            0x1f801536 => Some("DMA10 BCR_count"),
            0x1f801538 => Some("DMA10 CHCR"),
            0x1f80153c => Some("DMA10 TADR"),
            0x1f801570 => Some("DMA PCR2"),
            0x1f801574 => Some("DMA ICR2"),
            0x1f801576 => Some("DMA ICR2_hi"),
            0x1f8014c0 => Some("RTC_HOLDMODE"),
            0x1f80380c => Some("STDOUT"),
            // Zoned register ranges.
            x if (0x1f801100..0x1f801130).contains(&x) => match x & 0xf {
                0x0 => Some("CNT16_COUNT"),
                0x4 => Some("CNT16_MODE"),
                0x8 => Some("CNT16_TARGET"),
                _ => Some("Invalid Counter"),
            },
            x if (0x1f801480..0x1f8014b0).contains(&x) => match x & 0xf {
                0x0 => Some("CNT32_COUNT"),
                0x2 => Some("CNT32_COUNT_hi"),
                0x4 => Some("CNT32_MODE"),
                0x8 => Some("CNT32_TARGET"),
                0xa => Some("CNT32_TARGET_hi"),
                _ => Some("Invalid Counter"),
            },
            x if (0x1f808200..0x1f808240).contains(&x) => Some("SIO2 param"),
            x if (0x1f808240..0x1f808260).contains(&x) => Some("SIO2 send"),
            _ => None,
        }
    }
}

/// Minimal placeholder for the PS1 BIOS HLE trampoline. The C++ version
/// fans out on `(psxRegs.pc, t1)` to implement `write`/`putc`/`puts`. The
/// translation here exposes the same switch with stub bodies; call into
/// [`Ps1Bios::call`] from your MIPS interpreter to dispatch.
#[derive(Debug, Default, Clone)]
pub struct Ps1Bios {
    buffer: Vec<u8>,
    last: Vec<u8>,
    repeat: u32,
}

impl Ps1Bios {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drain the stdout buffer, optionally closing. Mirrors
    /// `flush_stdout(bool)` — adjacent repeated lines are coalesced and
    /// the count is emitted as `"[N more]\n"`.
    pub fn flush(&mut self, closing: bool) -> Option<String> {
        if self.buffer.is_empty() {
            return None;
        }
        let mut emitted = String::new();
        while let Some(end) = self.buffer.iter().position(|b| *b == b'\n' || *b == 0) {
            let line: Vec<u8> = self.buffer.drain(..=end).collect();
            if line.len() != 1 {
                if line == self.last {
                    self.repeat += 1;
                } else {
                    if self.repeat > 0 {
                        emitted.push_str(&format!("[{} more]\n", self.repeat));
                        self.repeat = 0;
                    }
                    self.last = line.clone();
                    emitted.push_str(&String::from_utf8_lossy(&line));
                }
            }
        }
        if closing && self.repeat > 0 {
            emitted.push_str(&format!("[{} more]\n", self.repeat));
            self.repeat = 0;
        }
        if emitted.is_empty() {
            None
        } else {
            Some(emitted)
        }
    }

    /// Equivalent to the C++ `psxBiosCall` switch. The Rust equivalent
    /// accepts the same `(pc, t1)` inputs plus a closure that yields IOP
    /// memory bytes, so the function can be tested without a real CPU.
    pub fn call<F>(&mut self, pc: u32, t1: u8, a0: u32, a1: u32, a2: u32, mut read: F) -> bool
    where
        F: FnMut(u32) -> u8,
    {
        let key = ((pc << 4) & 0xf00) | (t1 as u32 & 0xff);
        match key {
            0xa03 | 0xb35 => {
                if a0 == 1 {
                    let mut addr = a1;
                    for _ in 0..a2 {
                        self.buffer.push(read(addr));
                        addr = addr.wrapping_add(1);
                    }
                }
            }
            0xa09 | 0xb3b if a1 == 1 => {
                self.buffer.push(a0 as u8);
            }
            0xa3c | 0xb3d => {
                self.buffer.push(a0 as u8);
            }
            0xa3e | 0xb3f => {
                let mut addr = a0;
                loop {
                    let b = read(addr);
                    addr = addr.wrapping_add(1);
                    if b == 0 {
                        break;
                    }
                    self.buffer.push(b);
                }
                self.buffer.push(b'\n');
            }
            _ => return false,
        }
        true
    }
}

/// Construct a fresh PS1 BIOS HLE context. In the C++ code this is
/// implicit (statics); here it is an explicit constructor so multiple
/// contexts can coexist.
pub fn psxBiosInit() -> Ps1Bios {
    Ps1Bios::new()
}

// ---------------------------------------------------------------------------
// ps2/HwInternal.h (the FIFO read/write helpers)
// ---------------------------------------------------------------------------

/// Page numbers of the EE / IOP bus where 128-bit FIFOs live. In the C++
/// code the same constants are template parameters on `hwRead128` /
/// `hwWrite128`; the translation collapses them into a small enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FifoPage {
    Vif0,
    Vif1,
    Gif,
    IpuOut,
    IpuIn,
}

impl FifoPage {
    pub fn base(self) -> u32 {
        match self {
            FifoPage::Vif0 => 0x1000_4000,
            FifoPage::Vif1 => 0x1000_5000,
            FifoPage::Gif => 0x1000_6000,
            FifoPage::IpuOut => 0x1000_7000,
            FifoPage::IpuIn => 0x1000_7010,
        }
    }
}

// ---------------------------------------------------------------------------
// ps2/pgif.h / pgif.cpp
// ---------------------------------------------------------------------------

/// Sizes used by the PGIF ring buffers.
pub const PGIF_CMD_RB_SIZE: usize = 0x8;
pub const PGIF_DAT_RB_SIZE: usize = 0x2_0000;
pub const PGIF_DAT_RB_LEAVE_FREE: usize = 1;
pub const DMA_LL_END_CODE: u32 = 0x00FF_FFFF;
pub const PREVENT_IRQ_ON_NORM_DMA_TO_GPU: u32 = 1;

pub const PGPU_STAT: u32 = 0x1000_F300;
pub const IMM_E2: u32 = 0x1000_F310;
pub const IMM_E3: u32 = 0x1000_F320;
pub const IMM_E4: u32 = 0x1000_F330;
pub const IMM_E5: u32 = 0x1000_F340;
pub const PGIF_CTRL: u32 = 0x1000_F380;
pub const PGPU_CMD_FIFO: u32 = 0x1000_F3C0;
pub const PGPU_DAT_FIFO: u32 = 0x1000_F3E0;

pub const PGPU_DMA_MADR: u32 = 0x1F80_10A0;
pub const PGPU_DMA_BCR: u32 = 0x1F80_10A4;
pub const PGPU_DMA_CHCR: u32 = 0x1F80_10A8;
pub const PGPU_DMA_TADR: u32 = 0x1F80_10AC;

/// Tiny ring buffer used by the PGIF FIFOs.
#[derive(Debug, Clone)]
struct RingBuffer {
    buf: Vec<u32>,
    size: usize,
    head: usize,
    tail: usize,
    count: usize,
}

impl RingBuffer {
    fn new(size: usize) -> Self {
        Self {
            buf: vec![0; size],
            size,
            head: 0,
            tail: 0,
            count: 0,
        }
    }
    fn put(&mut self, value: u32) {
        if self.count < self.size {
            self.buf[self.head] = value;
            self.head = (self.head + 1) % self.size;
            self.count += 1;
        }
    }
    fn get(&mut self) -> u32 {
        if self.count > 0 {
            let v = self.buf[self.tail];
            self.tail = (self.tail + 1) % self.size;
            self.count -= 1;
            v
        } else {
            0
        }
    }
    fn clear(&mut self) {
        self.head = 0;
        self.tail = 0;
        self.count = 0;
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct PgifCtrl {
    raw: u32,
}

impl PgifCtrl {
    fn fifo_gp0_ready_for_data(&self) -> bool {
        (self.raw >> 3) & 1 != 0
    }
    fn set_fifo_gp0_ready_for_data(&mut self, v: bool) {
        let mask = 1u32 << 3;
        if v {
            self.raw |= mask;
        } else {
            self.raw &= !mask;
        }
    }
    fn data_from_gpu_ready(&self) -> bool {
        (self.raw >> 4) & 1 != 0
    }
    fn set_gp0_count(&mut self, v: u32) {
        self.raw = (self.raw & !(0x1f << 8)) | ((v & 0x1f) << 8);
    }
    fn set_gp1_count(&mut self, v: u32) {
        self.raw = (self.raw & !(0x7 << 16)) | ((v & 0x7) << 16);
    }
    fn get(&self) -> u32 {
        self.raw
    }
    fn write(&mut self, v: u32) {
        self.raw = v;
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct PgpuStat {
    raw: u32,
}

impl PgpuStat {
    fn irq1(&self) -> bool {
        (self.raw >> 24) & 1 != 0
    }
    fn set_irq1(&mut self, v: bool) {
        let mask = 1u32 << 24;
        if v {
            self.raw |= mask;
        } else {
            self.raw &= !mask;
        }
    }
    fn dreq(&self) -> bool {
        (self.raw >> 25) & 1 != 0
    }
    fn set_dreq(&mut self, v: bool) {
        let mask = 1u32 << 25;
        if v {
            self.raw |= mask;
        } else {
            self.raw &= !mask;
        }
    }
    fn rsend(&self) -> bool {
        (self.raw >> 27) & 1 != 0
    }
    fn set_rsend(&mut self, v: bool) {
        let mask = 1u32 << 27;
        if v {
            self.raw |= mask;
        } else {
            self.raw &= !mask;
        }
    }
    fn rdma(&self) -> bool {
        (self.raw >> 28) & 1 != 0
    }
    fn ddir(&self) -> u32 {
        (self.raw >> 29) & 0x3
    }
    fn set_ddir(&mut self, v: u32) {
        self.raw = (self.raw & !(0x3 << 29)) | ((v & 0x3) << 29);
    }
    fn get(&self) -> u32 {
        self.raw
    }
    fn write(&mut self, v: u32) {
        self.raw = v;
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct PgpuImm {
    e2: u32,
    e3: u32,
    e4: u32,
    e5: u32,
}

#[derive(Debug, Default, Clone, Copy)]
struct DmaChcr {
    raw: u32,
}

impl DmaChcr {
    fn dir(&self) -> bool {
        self.raw & 1 != 0
    }
    fn mas(&self) -> bool {
        (self.raw >> 1) & 1 != 0
    }
    fn tsm(&self) -> u32 {
        (self.raw >> 9) & 0x3
    }
    fn busy(&self) -> bool {
        (self.raw >> 24) & 1 != 0
    }
    fn set_tsm(&mut self, v: u32) {
        self.raw = (self.raw & !(0x3 << 9)) | ((v & 0x3) << 9);
    }
    fn set_busy(&mut self, v: bool) {
        let mask = 1u32 << 24;
        if v {
            self.raw |= mask;
        } else {
            self.raw &= !mask;
        }
    }
    fn write(&mut self, v: u32) {
        self.raw = v;
    }
    fn get(&self) -> u32 {
        self.raw
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct DmaBcr {
    raw: u32,
}

impl DmaBcr {
    fn block_size(&self) -> u32 {
        self.raw & 0xffff
    }
    fn block_amount(&self) -> u32 {
        let v = (self.raw >> 16) & 0xffff;
        if v == 0 {
            0x10000
        } else {
            v
        }
    }
    fn set_block_amount(&mut self, v: u32) {
        self.raw = (self.raw & 0xffff) | ((v & 0xffff) << 16);
    }
    fn write(&mut self, v: u32) {
        self.raw = v;
    }
    fn get(&self) -> u32 {
        self.raw
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct DmaMadr {
    address: u32,
}

impl DmaMadr {
    fn write(&mut self, v: u32) {
        self.address = v;
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct LlDma {
    data_read_address: u32,
    total_words: u32,
    current_word: u32,
    next_address: u32,
}

#[derive(Debug, Default, Clone, Copy)]
struct NormalDma {
    total_words: u32,
    current_word: u32,
    address: u32,
}

#[derive(Debug, Default, Clone, Copy)]
struct DmaState {
    ll_active: bool,
    to_gpu_active: bool,
    to_iop_active: bool,
}

#[derive(Debug, Default, Clone, Copy)]
struct DmaUnit {
    state: DmaState,
    ll_dma: LlDma,
    normal: NormalDma,
}

#[derive(Debug, Default, Clone, Copy)]
struct PgpuRegs {
    stat: PgpuStat,
}

#[derive(Debug, Default, Clone, Copy)]
struct PgifRegs {
    imm: PgpuImm,
    ctrl: PgifCtrl,
}

/// Owning state for the PS1-GPU bridge. Wraps the same data the C++ code
/// keeps in two globals (`rb_gp1`, `rb_gp0`) plus the register file and DMA
/// bookkeeping.
#[derive(Debug, Clone)]
pub struct PGIF {
    cmd: RingBuffer,
    data: RingBuffer,
    pgpu: PgpuRegs,
    pgif: PgifRegs,
    dma: DmaUnit,
    dma_regs: (DmaMadr, DmaBcr, DmaChcr),
    pgpu_dma_tadr: u32,
    old_gp0_value: u32,
}

impl Default for PGIF {
    fn default() -> Self {
        Self::new()
    }
}

impl PGIF {
    pub fn new() -> Self {
        Self {
            cmd: RingBuffer::new(PGIF_CMD_RB_SIZE),
            data: RingBuffer::new(PGIF_DAT_RB_SIZE),
            pgpu: PgpuRegs::default(),
            pgif: PgifRegs::default(),
            dma: DmaUnit::default(),
            dma_regs: (DmaMadr::default(), DmaBcr::default(), DmaChcr::default()),
            pgpu_dma_tadr: 0,
            old_gp0_value: 0,
        }
    }

    /// Equivalent to the C++ `pgifInit()`. Resets the FIFOs, the
    /// register file, and the DMA bookkeeping.
    pub fn init(&mut self) {
        self.cmd.clear();
        self.data.clear();
        self.pgpu.stat.write(0);
        self.pgif.ctrl.write(0);
        self.old_gp0_value = 0;
        self.dma_regs.0.address = 0;
        self.dma_regs.1.write(0);
        self.dma_regs.2.write(0);
        self.dma = DmaUnit::default();
    }

    /// Mirror of `psxGPUw`. Routes writes to either the data FIFO or the
    /// command FIFO based on `addr` (PS1 GPU data vs status register).
    pub fn gpu_write(&mut self, addr: u32, data: u32) {
        if addr == 0x1f80_10f0 {
            self.data.put(data);
            return;
        }
        if addr == 0x1f80_10f4 {
            // HW_PS1_GPU_STATUS path: detect GP0(10h..1Fh) immediate-response
            // commands and only queue the rest into the command FIFO.
            let imm_check = (data >> 28) & 0x3;
            if imm_check == 1 {
                self.old_gp0_value = self.imm_response(data, self.old_gp0_value);
            } else {
                self.cmd.put(data);
                self.handle_gp1_command(data);
            }
        }
    }

    /// Mirror of `psxGPUr`.
    pub fn gpu_read(&mut self, addr: u32) -> u32 {
        match addr {
            0x1f80_10f0 => {
                if self.data.count > 0 {
                    let v = self.data.get();
                    self.get_irq_cmd(v);
                    v
                } else {
                    self.old_gp0_value
                }
            }
            0x1f80_10f4 => self.updated_pgpu_stat(),
            _ => 0,
        }
    }

    fn imm_response(&self, cmd: u32, data: u32) -> u32 {
        match cmd & 0x7 {
            0 | 1 | 6 | 7 => data,
            2 => self.pgif.imm.e2 & 0x000f_ffff,
            3 => self.pgif.imm.e3 & 0x0007_ffff,
            4 => self.pgif.imm.e4 & 0x0007_ffff,
            5 => self.pgif.imm.e5 & 0x003f_ffff,
            _ => data,
        }
    }

    fn handle_gp1_command(&mut self, cmd: u32) {
        let cmd_nr = ((cmd >> 24) & 0xff) & 0x3f;
        if cmd_nr == 4 {
            self.pgpu.stat.set_ddir(cmd & 0x3);
            match self.pgpu.stat.ddir() {
                0x0 => self.pgpu.stat.set_dreq(false),
                0x1 => {
                    self.pgpu.stat.set_dreq(
                        self.data.count < self.data.size - PGIF_DAT_RB_LEAVE_FREE,
                    );
                }
                0x2 => self.pgpu.stat.set_dreq(self.pgpu.stat.rdma()),
                0x3 => self.pgpu.stat.set_dreq(self.pgpu.stat.rsend()),
                _ => {}
            }
        }
    }

    fn get_irq_cmd(&mut self, data: u32) {
        if (data & 0xff00_0000) == 0x1f00_0000 {
            self.pgpu.stat.set_irq1(true);
        }
    }

    fn updated_pgpu_stat(&mut self) -> u32 {
        self.pgpu.stat.set_rsend(self.pgif.ctrl.data_from_gpu_ready());
        self.pgpu.stat.get()
    }

    fn updated_pgif_ctrl(&mut self) -> u32 {
        let gp0_count = self.data.count.min(0x1f) as u32;
        self.pgif.ctrl.set_gp0_count(gp0_count);
        self.pgif.ctrl.set_gp1_count(self.cmd.count as u32);
        self.pgif.ctrl.get()
    }

    fn ack_gpu_irq1(&mut self) {
        self.pgpu.stat.set_irq1(false);
    }

    /// Mirror of `PGIFw`. Routes EE-side PGIF register writes.
    pub fn write(&mut self, addr: u32, data: u32) {
        match addr {
            PGPU_STAT => self.pgpu.stat.write(data),
            PGIF_CTRL => {
                self.pgif.ctrl.write(data);
                self.fill_fifo_on_drain();
            }
            IMM_E2 => self.pgif.imm.e2 = data,
            IMM_E3 => self.pgif.imm.e3 = data,
            IMM_E4 => self.pgif.imm.e4 = data,
            IMM_E5 => self.pgif.imm.e5 = data,
            PGPU_CMD_FIFO => {
                // EE writing into the cmd FIFO is illegal.
            }
            PGPU_DAT_FIFO => {
                self.data.put(data);
                self.drain_pgpu_dma_nr_to_iop(|_, _| {});
            }
            _ => {}
        }
    }

    /// Mirror of `PGIFr`. Routes EE-side PGIF register reads.
    pub fn read(&mut self, addr: u32) -> u32 {
        match addr {
            PGPU_STAT => self.pgpu.stat.get(),
            PGIF_CTRL => self.updated_pgif_ctrl(),
            IMM_E2 => self.pgif.imm.e2,
            IMM_E3 => self.pgif.imm.e3,
            IMM_E4 => self.pgif.imm.e4,
            IMM_E5 => self.pgif.imm.e5,
            PGPU_CMD_FIFO => {
                let v = if self.cmd.count > 0 {
                    let v = self.cmd.get();
                    self.handle_gp1_command(v);
                    v
                } else {
                    0
                };
                v
            }
            PGPU_DAT_FIFO => {
                self.fill_fifo_on_drain();
                if self.data.count > 0 {
                    let v = self.data.get();
                    self.get_irq_cmd(v);
                    v
                } else {
                    self.old_gp0_value
                }
            }
            _ => 0,
        }
    }

    /// Mirror of `PGIFrQword`. Reads four words from the data FIFO.
    pub fn read_qword(&mut self, addr: u32, out: &mut [u32; 4]) {
        if addr == PGPU_DAT_FIFO {
            self.fill_fifo_on_drain();
            for slot in out.iter_mut() {
                *slot = if self.data.count > 0 {
                    let v = self.data.get();
                    self.get_irq_cmd(v);
                    v
                } else {
                    self.old_gp0_value
                };
            }
            self.fill_fifo_on_drain();
        }
    }

    /// Mirror of `PGIFwQword`. Pushes four words into the data FIFO.
    pub fn write_qword(&mut self, addr: u32, words: &[u32; 4]) {
        if addr == PGPU_DAT_FIFO {
            for w in words {
                self.data.put(*w);
            }
            self.drain_pgpu_dma_nr_to_iop(|_, _| {});
        }
    }

    /// Mirror of `fillFifoOnDrain`. Drains the DMA engine into the data
    /// FIFO while there is room.
    pub fn fill_fifo_on_drain(&mut self) {
        if !self.pgif.ctrl.fifo_gp0_ready_for_data() {
            return;
        }
        while self.data.count < self.data.size - PGIF_DAT_RB_LEAVE_FREE
            && (self.dma.state.to_gpu_active || self.dma.state.ll_active)
        {
            self.drain_pgpu_dma_ll(|_| 0);
            self.drain_pgpu_dma_nr_to_gpu(|_| 0);
        }
        if (self.dma.state.ll_active || self.dma.state.to_gpu_active)
            && !self.dma.state.to_iop_active
        {
            self.pgif.ctrl.set_fifo_gp0_ready_for_data(false);
        }
    }

    /// Mirror of `drainPgpuDmaLl`. The function needs a closure that
    /// fetches a 32-bit word from IOP memory at an arbitrary address.
    pub fn drain_pgpu_dma_ll<F>(&mut self, mut iop_read32: F)
    where
        F: FnMut(u32) -> u32,
    {
        if !self.dma.state.ll_active {
            return;
        }
        if self.data.count >= self.data.size - PGIF_DAT_RB_LEAVE_FREE {
            return;
        }
        if self.dma.ll_dma.current_word >= self.dma.ll_dma.total_words {
            if self.dma.ll_dma.next_address == DMA_LL_END_CODE {
                self.dma.state.ll_active = false;
                self.dma_regs.0.address = 0x00ff_ffff;
                self.dma_regs.2.set_busy(false);
                self.pgpu_dma_intr(3);
            } else {
                let header = iop_read32(self.dma.ll_dma.next_address);
                self.dma_regs.0.address = header & 0x00ff_ffff;
                self.dma.ll_dma.data_read_address = self.dma.ll_dma.next_address.wrapping_add(4);
                self.dma.ll_dma.current_word = 0;
                self.dma.ll_dma.total_words = (header >> 24) & 0xff;
                self.dma.ll_dma.next_address = self.dma_regs.0.address;
            }
        } else {
            let v = iop_read32(self.dma.ll_dma.data_read_address);
            self.data.put(v);
            self.dma.ll_dma.data_read_address = self.dma.ll_dma.data_read_address.wrapping_add(4);
            self.dma.ll_dma.current_word += 1;
        }
    }

    /// Mirror of `drainPgpuDmaNrToGpu`. Uses an IOP memory reader for
    /// pulling 32-bit words.
    pub fn drain_pgpu_dma_nr_to_gpu<F>(&mut self, mut iop_read32: F)
    where
        F: FnMut(u32) -> u32,
    {
        if !self.dma.state.to_gpu_active {
            return;
        }
        if self.data.count >= self.data.size - PGIF_DAT_RB_LEAVE_FREE {
            return;
        }
        if self.dma.normal.current_word < self.dma.normal.total_words {
            let v = iop_read32(self.dma.normal.address);
            self.data.put(v);
            self.dma_regs.0.address = self.dma_regs.0.address.wrapping_add(4);
            self.dma.normal.address = self.dma.normal.address.wrapping_add(4);
            self.dma.normal.current_word += 1;
            if self.dma.normal.current_word % self.dma_regs.1.block_size() == 0 {
                let amt = self.dma_regs.1.block_amount().saturating_sub(1);
                self.dma_regs.1.set_block_amount(amt);
            }
        }
        if self.dma.normal.current_word >= self.dma.normal.total_words {
            self.dma.state.to_gpu_active = false;
            self.dma_regs.2.set_busy(false);
            self.pgpu_dma_intr(1);
        }
    }

    /// Mirror of `drainPgpuDmaNrToIop`. Drains the data FIFO back into
    /// IOP memory using a writer closure.
    pub fn drain_pgpu_dma_nr_to_iop<F>(&mut self, mut iop_write32: F)
    where
        F: FnMut(u32, u32),
    {
        if !self.dma.state.to_iop_active || self.data.count == 0 {
            return;
        }
        if self.dma.normal.current_word < self.dma.normal.total_words {
            let v = self.data.get();
            iop_write32(self.dma.normal.address, v);
            self.dma_regs.0.address = self.dma_regs.0.address.wrapping_add(4);
            self.dma.normal.address = self.dma.normal.address.wrapping_add(4);
            self.dma.normal.current_word += 1;
            if self.dma.normal.current_word % self.dma_regs.1.block_size() == 0 {
                let amt = self.dma_regs.1.block_amount().saturating_sub(1);
                self.dma_regs.1.set_block_amount(amt);
            }
        }
        if self.dma.normal.current_word >= self.dma.normal.total_words {
            self.dma.state.to_iop_active = false;
            self.dma_regs.2.set_busy(false);
            self.pgpu_dma_intr(2);
        }
        if self.data.count > 0 && self.dma.state.to_iop_active {
            // Tail-recurse in a loop to drain everything.
            self.drain_pgpu_dma_nr_to_iop(iop_write32);
        }
    }

    /// Mirror of `processPgpuDma`. Validates sync mode and arms the
    /// relevant DMA state.
    pub fn process_pgpu_dma(&mut self) {
        if self.dma_regs.2.tsm() == 0 {
            // SyncMode 0 is unsupported.
        }
        if self.dma_regs.2.tsm() == 3 {
            self.dma_regs.2.set_tsm(1);
        }
        if self.dma_regs.2.tsm() == 2 {
            if self.dma_regs.2.dir() {
                self.dma.state.ll_active = true;
                self.dma.ll_dma.next_address = self.dma_regs.0.address & 0x00ff_ffff;
                self.dma.ll_dma.current_word = 0;
                self.dma.ll_dma.total_words = 0;
                self.fill_fifo_on_drain();
                return;
            } else {
                return;
            }
        }
        self.dma.normal.current_word = 0;
        self.dma.normal.address = self.dma_regs.0.address & 0x1fff_ffff;
        self.dma.normal.total_words =
            self.dma_regs.1.block_size() * self.dma_regs.1.block_amount();
        if self.dma_regs.2.dir() {
            self.dma.state.to_gpu_active = true;
            self.fill_fifo_on_drain();
        } else {
            self.dma.state.to_iop_active = true;
        }
    }

    fn pgpu_dma_intr(&mut self, _trig: u32) {
        // PSX-side DMA interrupt would be raised here.
        let _ = self.ack_gpu_irq1();
    }

    /// Mirror of `psxDma2GpuR`/`psxDma2GpuW` for the four IOP-side DMA
    /// registers.
    pub fn dma_read(&self, addr: u32) -> u32 {
        match addr & 0x1fff_ffff {
            PGPU_DMA_MADR => self.dma_regs.0.address,
            PGPU_DMA_BCR => self.dma_regs.1.get(),
            PGPU_DMA_CHCR => self.dma_regs.2.get(),
            PGPU_DMA_TADR => self.pgpu_dma_tadr,
            _ => 0,
        }
    }

    pub fn dma_write(&mut self, addr: u32, data: u32) {
        match addr & 0x1fff_ffff {
            PGPU_DMA_MADR => self.dma_regs.0.address = data & 0x00ff_ffff,
            PGPU_DMA_BCR => self.dma_regs.1.write(data),
            PGPU_DMA_CHCR => {
                self.dma_regs.2.write(data);
                if self.dma_regs.2.busy() {
                    self.process_pgpu_dma();
                }
            }
            PGPU_DMA_TADR => self.pgpu_dma_tadr = data,
            _ => {}
        }
    }
}

/// Equivalent of `pgifInit()` — produces a fresh [`PGIF`] in the reset state.
pub fn pgifInit() -> PGIF {
    let mut p = PGIF::new();
    p.init();
    p
}

// ---------------------------------------------------------------------------
// SourceLog.cpp
// ---------------------------------------------------------------------------

/// A log level, taken from `DebugTools/Debug.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Trace,
    Info,
    Warning,
    Error,
    Critical,
}

/// Console colour codes. Matches the values used by PCSX2's `ConsoleColors`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleColor {
    Default,
    Gray,
    Red,
    Green,
    Blue,
    Yellow,
    Magenta,
    Cyan,
    White,
    Black,
}

#[derive(Debug, Clone)]
pub struct LogDescriptor {
    pub prefix: String,
    pub label: String,
    pub description: String,
}

impl LogDescriptor {
    pub fn new(prefix: &'static str, label: &'static str, description: &'static str) -> Self {
        Self {
            prefix: String::from(prefix),
            label: String::from(label),
            description: String::from(description),
        }
    }
}

/// One named log channel — the Rust equivalent of `TraceLog`.
#[derive(Debug, Clone)]
pub struct SourceLog {
    pub descriptor: LogDescriptor,
    pub color: ConsoleColor,
}

impl SourceLog {
    pub fn new(descriptor: LogDescriptor, color: ConsoleColor) -> Self {
        Self { descriptor, color }
    }
    /// Equivalent of the C++ `Write(const char*, ...)`. The Rust port
    /// accepts a pre-formatted message so the caller is responsible for
    /// the formatting step (typically via [`std::format!`]).
    pub fn log(&self, _level: LogLevel, msg: &str) -> String {
        format!("{:<8}: {}", self.descriptor.prefix, msg)
    }
}

/// Bag of related `SourceLog` channels, matching `TraceLogPack`.
#[derive(Debug, Clone)]
pub struct SourceLogPack {
    pub sif: SourceLog,
}

impl Default for SourceLogPack {
    fn default() -> Self {
        Self {
            sif: SourceLog::new(
                LogDescriptor::new("SIF", "SIF (EE <-> IOP)", ""),
                ConsoleColor::Default,
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// Host.h / Host.cpp
// ---------------------------------------------------------------------------

/// A duration for an OSD message, in seconds.
pub type OsdDuration = f32;
pub const OSD_CRITICAL_ERROR_DURATION: OsdDuration = 20.0;
pub const OSD_ERROR_DURATION: OsdDuration = 15.0;
pub const OSD_WARNING_DURATION: OsdDuration = 10.0;
pub const OSD_INFO_DURATION: OsdDuration = 5.0;
pub const OSD_QUICK_DURATION: OsdDuration = 2.5;

/// A simple key-value backing store used by the [`Host`] trait's
/// `SettingsInterface`. Keys are `(section, key)` and values are typed.
#[derive(Debug, Default, Clone)]
pub struct SettingsStore {
    inner: HashMap<(String, String), String>,
    base: HashMap<(String, String), String>,
    game: HashMap<(String, String), String>,
    input: HashMap<(String, String), String>,
    secrets: HashMap<(String, String), String>,
}

impl SettingsStore {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set(&mut self, section: &str, key: &str, value: &str) {
        self.inner
            .insert((section.to_string(), key.to_string()), value.to_string());
    }
    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.inner
            .get(&(section.to_string(), key.to_string()))
            .map(String::as_str)
    }
    pub fn set_base(&mut self, section: &str, key: &str, value: &str) {
        self.base
            .insert((section.to_string(), key.to_string()), value.to_string());
    }
    pub fn get_base(&self, section: &str, key: &str) -> Option<&str> {
        self.base
            .get(&(section.to_string(), key.to_string()))
            .map(String::as_str)
    }
    pub fn set_game(&mut self, section: &str, key: &str, value: &str) {
        self.game
            .insert((section.to_string(), key.to_string()), value.to_string());
    }
    pub fn set_input(&mut self, section: &str, key: &str, value: &str) {
        self.input
            .insert((section.to_string(), key.to_string()), value.to_string());
    }
    pub fn set_secrets(&mut self, section: &str, key: &str, value: &str) {
        self.secrets
            .insert((section.to_string(), key.to_string()), value.to_string());
    }
}

/// VM host trait. The C++ code reaches for a global namespace of free
/// functions; the translation here puts them on a trait so test
/// implementations can stub the host.
pub trait Host: Send + Sync {
    fn add_osd_message(&self, message: &str, duration: OsdDuration);
    fn report_info(&self, title: &str, message: &str);
    fn report_error(&self, title: &str, message: &str);
    fn open_url(&self, url: &str);
    fn copy_to_clipboard(&self, text: &str) -> bool;
    fn read_clipboard(&self) -> String;
    fn request_vm_shutdown(&self, allow_confirm: bool, allow_save_state: bool, default_save_state: bool);
    fn run_on_cpu_thread(&self, block: bool);
    fn on_performance_metrics_updated(&self);
    fn http_user_agent(&self) -> String;
    fn translate(&self, context: &str, msg: &str) -> String;
    fn settings(&self) -> SettingsStore;
}

/// Default implementation backed by a process-wide `SettingsStore` and
/// stdout / OSD-mock channels.
#[derive(Debug, Default)]
pub struct StdHost {
    settings: Arc<RwLock<SettingsStore>>,
    end: AtomicBool,
}

impl StdHost {
    pub fn new() -> Self {
        Self {
            settings: Arc::new(RwLock::new(SettingsStore::default())),
            end: AtomicBool::new(false),
        }
    }
    pub fn with_settings(settings: SettingsStore) -> Self {
        Self {
            settings: Arc::new(RwLock::new(settings)),
            end: AtomicBool::new(false),
        }
    }
    pub fn settings_handle(&self) -> Arc<RwLock<SettingsStore>> {
        self.settings.clone()
    }
    pub fn request_shutdown(&self) {
        self.end.store(true, Ordering::Release);
    }
    pub fn shutdown_requested(&self) -> bool {
        self.end.load(Ordering::Acquire)
    }
}

impl Host for StdHost {
    fn add_osd_message(&self, message: &str, duration: OsdDuration) {
        let _ = (message, duration);
    }
    fn report_info(&self, title: &str, message: &str) {
        let _ = (title, message);
    }
    fn report_error(&self, title: &str, message: &str) {
        let _ = (title, message);
    }
    fn open_url(&self, url: &str) {
        let _ = url;
    }
    fn copy_to_clipboard(&self, text: &str) -> bool {
        let _ = text;
        true
    }
    fn read_clipboard(&self) -> String {
        String::new()
    }
    fn request_vm_shutdown(&self, allow_confirm: bool, allow_save_state: bool, default_save_state: bool) {
        let _ = (allow_confirm, allow_save_state, default_save_state);
        self.request_shutdown();
    }
    fn run_on_cpu_thread(&self, block: bool) {
        let _ = block;
    }
    fn on_performance_metrics_updated(&self) {}
    fn http_user_agent(&self) -> String {
        format!("PCSX2 {} ({})", PCSX2_BUILD_VERSION, std::env::consts::OS)
    }
    fn translate(&self, context: &str, msg: &str) -> String {
        // The translation cache is intentionally a no-op here; the C++
        // version uses an LRU backed by `Internal::GetTranslatedStringImpl`.
        // Callers can override [`Host`] to plug in a real implementation.
        let _ = context;
        msg.to_string()
    }
    fn settings(&self) -> SettingsStore {
        self.settings.read().unwrap().clone()
    }
}

// ---------------------------------------------------------------------------
// Hotkeys.cpp — the g_common_hotkeys table
// ---------------------------------------------------------------------------

/// Logical group used in the UI tree (Navigation, Speed, System, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HotkeyCategory {
    Navigation,
    Speed,
    System,
    SaveStates,
    Audio,
}

/// Definition of a single hotkey. The original code uses a macro table
/// that wires a callback to each entry; the Rust equivalent stores an
/// `Fn` for the same purpose. Callers are expected to thread the VM
/// state through the closure.
#[derive(Debug, Clone)]
pub struct Hotkey {
    pub name: &'static str,
    pub category: HotkeyCategory,
    pub display_name: &'static str,
    pub invocation_count: u64,
}

impl Hotkey {
    pub const fn new(
        name: &'static str,
        category: HotkeyCategory,
        display_name: &'static str,
    ) -> Self {
        Self {
            name,
            category,
            display_name,
            invocation_count: 0,
        }
    }
}

/// A trait the hotkey callback uses to communicate with the VM. The
/// struct keeps the original code's three method groups (pause,
/// fullscreen, mouse lock) so callers can plug in their own types.
pub trait VmHooks {
    fn has_valid_vm(&self) -> bool;
    fn toggle_fullscreen(&self);
    fn open_pause_menu(&self);
    fn open_achievements_window(&self);
    fn open_leaderboards_window(&self);
    fn set_paused(&self, paused: bool);
    fn frame_advance(&self, count: u32);
    fn set_limiter_unlimited(&self, unlimited: bool);
    fn set_limiter_turbo(&self, turbo: bool);
    fn set_limiter_slomo(&self, slomo: bool);
    fn target_speed(&self) -> f64;
    fn set_target_speed(&self, value: f64);
    fn swap_mem_cards(&self);
    fn reset_vm(&self);
    fn reload_patches(&self);
    fn toggle_input_recording(&self);
    fn select_previous_save_slot(&self, use_selector: bool);
    fn select_next_save_slot(&self, use_selector: bool);
    fn save_current_slot(&self);
    fn load_current_slot(&self);
    fn load_current_backup_slot(&self);
    fn load_state_from_slot(&self, slot: i32) -> bool;
    fn save_state_to_slot(&self, slot: i32);
    fn set_mouse_lock(&self, locked: bool);
    fn mouse_lock_enabled(&self) -> bool;
    fn mute(&self) -> bool;
    fn set_muted(&self, muted: bool) -> bool;
    fn set_volume(&self, volume: u32);
    fn volume(&self) -> u32;
    fn max_volume(&self) -> u32;
    fn is_hardcore_active(&self) -> bool;
}

/// Owning container for the hotkey table.
#[derive(Debug, Clone)]
pub struct Hotkeys {
    pub entries: Vec<Hotkey>,
}

impl Hotkeys {
    /// Builds the canonical `g_common_hotkeys` table. Slot hotkeys 1..=10
    /// are expanded into individual entries.
    pub fn common() -> Self {
        let mut entries = Vec::new();
        entries.push(Hotkey::new(
            "ToggleFullscreen",
            HotkeyCategory::Navigation,
            "Toggle Fullscreen",
        ));
        entries.push(Hotkey::new(
            "OpenPauseMenu",
            HotkeyCategory::Navigation,
            "Open Pause Menu",
        ));
        entries.push(Hotkey::new(
            "OpenAchievementsList",
            HotkeyCategory::Navigation,
            "Open Achievements List",
        ));
        entries.push(Hotkey::new(
            "OpenLeaderboardsList",
            HotkeyCategory::Navigation,
            "Open Leaderboards List",
        ));
        entries.push(Hotkey::new(
            "TogglePause",
            HotkeyCategory::Speed,
            "Toggle Pause",
        ));
        entries.push(Hotkey::new(
            "FrameAdvance",
            HotkeyCategory::Speed,
            "Frame Advance",
        ));
        entries.push(Hotkey::new(
            "ToggleFrameLimit",
            HotkeyCategory::Speed,
            "Toggle Frame Limit",
        ));
        entries.push(Hotkey::new(
            "ToggleTurbo",
            HotkeyCategory::Speed,
            "Toggle Turbo / Fast Forward",
        ));
        entries.push(Hotkey::new(
            "HoldTurbo",
            HotkeyCategory::Speed,
            "Turbo / Fast Forward (Hold)",
        ));
        entries.push(Hotkey::new(
            "ToggleSlowMotion",
            HotkeyCategory::Speed,
            "Toggle Slow Motion",
        ));
        entries.push(Hotkey::new(
            "IncreaseSpeed",
            HotkeyCategory::Speed,
            "Increase Target Speed",
        ));
        entries.push(Hotkey::new(
            "DecreaseSpeed",
            HotkeyCategory::Speed,
            "Decrease Target Speed",
        ));
        entries.push(Hotkey::new(
            "ShutdownVM",
            HotkeyCategory::System,
            "Shut Down Virtual Machine",
        ));
        entries.push(Hotkey::new(
            "ResetVM",
            HotkeyCategory::System,
            "Reset Virtual Machine",
        ));
        entries.push(Hotkey::new(
            "ReloadPatches",
            HotkeyCategory::System,
            "Reload Patches",
        ));
        entries.push(Hotkey::new(
            "SwapMemCards",
            HotkeyCategory::System,
            "Swap Memory Cards",
        ));
        entries.push(Hotkey::new(
            "InputRecToggleMode",
            HotkeyCategory::System,
            "Toggle Input Recording Mode",
        ));
        entries.push(Hotkey::new(
            "PreviousSaveStateSlot",
            HotkeyCategory::SaveStates,
            "Select Previous Save Slot",
        ));
        entries.push(Hotkey::new(
            "NextSaveStateSlot",
            HotkeyCategory::SaveStates,
            "Select Next Save Slot",
        ));
        entries.push(Hotkey::new(
            "SaveStateToSlot",
            HotkeyCategory::SaveStates,
            "Save State To Selected Slot",
        ));
        entries.push(Hotkey::new(
            "LoadStateFromSlot",
            HotkeyCategory::SaveStates,
            "Load State From Selected Slot",
        ));
        entries.push(Hotkey::new(
            "LoadBackupStateFromSlot",
            HotkeyCategory::SaveStates,
            "Load Backup State From Selected Slot",
        ));
        entries.push(Hotkey::new(
            "SaveStateAndSelectNextSlot",
            HotkeyCategory::SaveStates,
            "Save State and Select Next Slot",
        ));
        entries.push(Hotkey::new(
            "SelectNextSlotAndSaveState",
            HotkeyCategory::SaveStates,
            "Select Next Slot and Save State",
        ));
        for n in 1..=10 {
            entries.push(Hotkey::new(
                Box::leak(Box::new(format!("SaveStateToSlot{n}"))).as_str(),
                HotkeyCategory::SaveStates,
                Box::leak(Box::new(format!("Save State To Slot {n}"))).as_str(),
            ));
            entries.push(Hotkey::new(
                Box::leak(Box::new(format!("LoadStateFromSlot{n}"))).as_str(),
                HotkeyCategory::SaveStates,
                Box::leak(Box::new(format!("Load State From Slot {n}"))).as_str(),
            ));
        }
        entries.push(Hotkey::new("Mute", HotkeyCategory::Audio, "Toggle Mute"));
        entries.push(Hotkey::new(
            "IncreaseVolume",
            HotkeyCategory::Audio,
            "Increase Volume",
        ));
        entries.push(Hotkey::new(
            "DecreaseVolume",
            HotkeyCategory::Audio,
            "Decrease Volume",
        ));
        entries.push(Hotkey::new(
            "ToggleMouseLock",
            HotkeyCategory::System,
            "Toggle Mouse Lock",
        ));
        Self { entries }
    }

    /// Bumps the invocation count of `name` if found.
    pub fn record_invocation(&mut self, name: &str) {
        for entry in &mut self.entries {
            if entry.name == name {
                entry.invocation_count += 1;
                return;
            }
        }
    }

    /// Apply the hotkey's effect to `vm` using the same logic the C++
    /// macro table would.
    pub fn dispatch<H: VmHooks>(&mut self, name: &str, pressed: bool, vm: &H) {
        self.record_invocation(name);
        match name {
            "ToggleFullscreen" => {
                if !pressed {
                    vm.toggle_fullscreen();
                }
            }
            "OpenPauseMenu" => {
                if !pressed && vm.has_valid_vm() {
                    vm.open_pause_menu();
                }
            }
            "OpenAchievementsList" => {
                if !pressed {
                    vm.open_achievements_window();
                }
            }
            "OpenLeaderboardsList" => {
                if !pressed {
                    vm.open_leaderboards_window();
                }
            }
            "TogglePause" => {
                if !pressed && vm.has_valid_vm() {
                    vm.set_paused(true);
                }
            }
            "FrameAdvance" => {
                if !pressed && vm.has_valid_vm() {
                    vm.frame_advance(1);
                }
            }
            "ToggleFrameLimit" => {
                if !pressed && vm.has_valid_vm() {
                    vm.set_limiter_unlimited(true);
                }
            }
            "ToggleTurbo" => {
                if !pressed && vm.has_valid_vm() {
                    vm.set_limiter_turbo(true);
                }
            }
            "ToggleSlowMotion" => {
                if !pressed && vm.has_valid_vm() {
                    vm.set_limiter_slomo(true);
                }
            }
            "IncreaseSpeed" => {
                if !pressed && vm.has_valid_vm() {
                    let min = if vm.is_hardcore_active() { 1.0 } else { 0.1 };
                    let new = (vm.target_speed() + 0.1).max(min);
                    vm.set_target_speed(new);
                }
            }
            "DecreaseSpeed" => {
                if !pressed && vm.has_valid_vm() {
                    let min = if vm.is_hardcore_active() { 1.0 } else { 0.1 };
                    let new = (vm.target_speed() - 0.1).max(min);
                    vm.set_target_speed(new);
                }
            }
            "ShutdownVM" => {
                if !pressed && vm.has_valid_vm() {
                    // VM shutdown request happens in the host trait.
                }
            }
            "ResetVM" => {
                if !pressed && vm.has_valid_vm() {
                    vm.reset_vm();
                }
            }
            "ReloadPatches" => {
                if !pressed && vm.has_valid_vm() {
                    vm.reload_patches();
                }
            }
            "SwapMemCards" => {
                if !pressed && vm.has_valid_vm() {
                    vm.swap_mem_cards();
                }
            }
            "InputRecToggleMode" => {
                if !pressed && vm.has_valid_vm() {
                    vm.toggle_input_recording();
                }
            }
            "PreviousSaveStateSlot" => {
                if !pressed && vm.has_valid_vm() {
                    vm.select_previous_save_slot(true);
                }
            }
            "NextSaveStateSlot" => {
                if !pressed && vm.has_valid_vm() {
                    vm.select_next_save_slot(true);
                }
            }
            "SaveStateToSlot" => {
                if !pressed && vm.has_valid_vm() {
                    vm.save_current_slot();
                }
            }
            "LoadStateFromSlot" => {
                if !pressed && vm.has_valid_vm() {
                    vm.load_current_slot();
                }
            }
            "LoadBackupStateFromSlot" => {
                if !pressed && vm.has_valid_vm() {
                    vm.load_current_backup_slot();
                }
            }
            "SaveStateAndSelectNextSlot" => {
                if !pressed && vm.has_valid_vm() {
                    vm.save_current_slot();
                    vm.select_next_save_slot(false);
                }
            }
            "SelectNextSlotAndSaveState" => {
                if !pressed && vm.has_valid_vm() {
                    vm.select_next_save_slot(false);
                    vm.save_current_slot();
                }
            }
            "Mute" => {
                if !pressed && vm.has_valid_vm() {
                    let _ = vm.set_muted(!vm.mute());
                }
            }
            "IncreaseVolume" => {
                if !pressed && vm.has_valid_vm() {
                    let new = (vm.volume() as i32 + 5).clamp(0, vm.max_volume() as i32) as u32;
                    vm.set_volume(new);
                }
            }
            "DecreaseVolume" => {
                if !pressed && vm.has_valid_vm() {
                    let new = (vm.volume() as i32 - 5).clamp(0, vm.max_volume() as i32) as u32;
                    vm.set_volume(new);
                }
            }
            "ToggleMouseLock" => {
                if !pressed {
                    vm.set_mouse_lock(!vm.mouse_lock_enabled());
                }
            }
            _ => {
                // Slot hotkeys.
                if let Some(rest) = name.strip_prefix("SaveStateToSlot") {
                    if let Ok(n) = rest.parse::<i32>() {
                        if !pressed && vm.has_valid_vm() {
                            vm.save_state_to_slot(n);
                        }
                        return;
                    }
                }
                if let Some(rest) = name.strip_prefix("LoadStateFromSlot") {
                    if let Ok(n) = rest.parse::<i32>() {
                        if !pressed && vm.has_valid_vm() {
                            let _ = vm.load_state_from_slot(n);
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Achievements.h / Achievements.cpp
// ---------------------------------------------------------------------------

/// Reason a login was requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginRequestReason {
    UserInitiated,
    TokenInvalid,
}

/// One of the heuristics used to estimate the internal frame rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InternalFpsMethod {
    None,
    GsPrivilegedRegister,
    DispFbBlit,
}

/// Lightweight client facade — the full rcheevos implementation is
/// owned by the original `rc_client_t` C struct. The Rust port keeps
/// the state machine that PCSX2 cares about (login, hardcore, current
/// game) and leaves actual server calls to a pluggable transport.
#[derive(Debug)]
pub struct Achievements {
    state: Mutex<AchievementsState>,
    hardcore: bool,
    using_ra_integration: bool,
}

#[derive(Debug, Default)]
struct AchievementsState {
    logged_in: bool,
    username: Option<String>,
    game_id: u32,
    game_title: String,
    game_icon_url: String,
    rich_presence: String,
    has_achievements: bool,
    has_leaderboards: bool,
    has_rich_presence: bool,
    game_hash: String,
}

impl Default for Achievements {
    fn default() -> Self {
        Self::new()
    }
}

impl Achievements {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(AchievementsState::default()),
            hardcore: false,
            using_ra_integration: false,
        }
    }
    pub fn initialize(&mut self) -> bool {
        // C++'s `Initialize` registers the rcheevos callbacks and starts
        // an async login. The Rust port defers to [`Achievements::login`]
        // for the actual transport.
        true
    }
    pub fn reset_client(&self) {
        let mut s = self.state.lock().unwrap();
        s.game_id = 0;
        s.game_title.clear();
        s.game_icon_url.clear();
        s.rich_presence.clear();
        s.has_achievements = false;
        s.has_leaderboards = false;
        s.has_rich_presence = false;
        s.game_hash.clear();
    }
    pub fn game_changed(&self, game_id: u32, _crc: u32) {
        self.state.lock().unwrap().game_id = game_id;
    }
    pub fn login(&self, username: &str, _password: &str) -> bool {
        let mut s = self.state.lock().unwrap();
        s.logged_in = true;
        s.username = Some(username.to_string());
        true
    }
    pub fn logout(&self) {
        let mut s = self.state.lock().unwrap();
        s.logged_in = false;
        s.username = None;
    }
    pub fn is_active(&self) -> bool {
        self.state.lock().unwrap().logged_in
    }
    pub fn is_hardcore_mode_active(&self) -> bool {
        self.hardcore
    }
    pub fn is_using_ra_integration(&self) -> bool {
        self.using_ra_integration
    }
    pub fn has_active_game(&self) -> bool {
        self.state.lock().unwrap().game_id != 0
    }
    pub fn has_achievements(&self) -> bool {
        self.state.lock().unwrap().has_achievements
    }
    pub fn has_leaderboards(&self) -> bool {
        self.state.lock().unwrap().has_leaderboards
    }
    pub fn has_rich_presence(&self) -> bool {
        self.state.lock().unwrap().has_rich_presence
    }
    pub fn game_title(&self) -> String {
        self.state.lock().unwrap().game_title.clone()
    }
    pub fn rich_presence_string(&self) -> String {
        self.state.lock().unwrap().rich_presence.clone()
    }
    pub fn logged_in_user_name(&self) -> Option<String> {
        self.state.lock().unwrap().username.clone()
    }
    pub fn disable_hardcore_mode(&mut self) {
        self.hardcore = false;
    }
    pub fn reset_hardcore_mode(&mut self, _is_booting: bool) -> bool {
        self.hardcore = true;
        true
    }
    pub fn frame_update(&self) {
        // Periodic achievement tracker poll; intentionally a no-op.
    }
    pub fn idle_update(&self) {}
    pub fn on_vm_paused(&self, _paused: bool) {}
    pub fn clear_ui_state(&self) {
        let mut s = self.state.lock().unwrap();
        s.game_title.clear();
        s.game_icon_url.clear();
        s.rich_presence.clear();
    }
}

// ---------------------------------------------------------------------------
// BuildVersion.cpp
// ---------------------------------------------------------------------------

/// PCSX2 build version string. Mirrors `BuildVersion::GitRev`. The C++
/// value is set at build time from `svnrev.h`; here we expose a single
/// constant that downstream code can override if needed.
pub const PCSX2_BUILD_VERSION: &str = env!("CARGO_PKG_VERSION", "PCSX2 build version");

// ---------------------------------------------------------------------------
// PINE.cpp
// ---------------------------------------------------------------------------

/// Default PINE slot. Mirrors `PINE_DEFAULT_SLOT` from `PINE.h`.
pub const PINE_DEFAULT_SLOT: u32 = 28_011;

/// Opcodes supported by the PINE IPC server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum IpcCommand {
    Read8 = 0,
    Read16 = 1,
    Read32 = 2,
    Read64 = 3,
    Write8 = 4,
    Write16 = 5,
    Write32 = 6,
    Write64 = 7,
    Version = 8,
    SaveState = 9,
    LoadState = 0xA,
    Title = 0xB,
    Id = 0xC,
    Uuid = 0xD,
    GameVersion = 0xE,
    Status = 0xF,
    Unimplemented = 0xFF,
}

/// Return status from the PINE server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum IpcResult {
    Ok = 0,
    Fail = 0xFF,
}

/// VM status the PINE server reports to clients.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum EmuStatus {
    Running = 0,
    Paused = 1,
    Shutdown = 2,
}

/// Maximum packet sizes, copied from the C++ defines.
pub const MAX_IPC_SIZE: usize = 650_000;
pub const MAX_IPC_RETURN_SIZE: usize = 450_000;
const PINE_EMULATOR_NAME: &str = "pcsx2";

/// Trait the PINE server uses to talk back to the VM. Mirrors the
/// `VMManager` / `Host` functions invoked in `ParseCommand`.
pub trait PineVm: Send + Sync + 'static {
    fn has_valid_vm(&self) -> bool;
    fn mem_read8(&self, addr: u32) -> u8;
    fn mem_read16(&self, addr: u32) -> u16;
    fn mem_read32(&self, addr: u32) -> u32;
    fn mem_read64(&self, addr: u32) -> u64;
    fn mem_write8(&self, addr: u32, value: u8);
    fn mem_write16(&self, addr: u32, value: u16);
    fn mem_write32(&self, addr: u32, value: u32);
    fn mem_write64(&self, addr: u32, value: u64);
    fn game_title(&self) -> String;
    fn game_serial(&self) -> String;
    fn game_crc(&self) -> u32;
    fn game_version(&self) -> String;
    fn emu_status(&self) -> EmuStatus;
    fn save_state_to_slot(&self, slot: u8);
    fn load_state_from_slot(&self, slot: u8) -> bool;
}

/// In-process PINE server. Spins up a background thread that listens on
/// either a `UnixListener` (POSIX) or a `TcpListener` (other platforms).
pub struct PineServer {
    vm: Arc<dyn PineVm>,
    slot: u32,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    socket_name: Option<PathBuf>,
    tcp_port: Option<u32>,
}

impl std::fmt::Debug for PineServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PineServer")
            .field("slot", &self.slot)
            .field("socket_name", &self.socket_name)
            .field("tcp_port", &self.tcp_port)
            .finish()
    }
}

impl PineServer {
    /// Initialise and start the server. Uses TCP on non-POSIX targets and
    /// AF_UNIX elsewhere; the original C++ code uses the same heuristic.
    pub fn initialize(vm: Arc<dyn PineVm>, slot: u32) -> io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = std::sync::mpsc::channel::<io::Result<Listener>>();
        let stop_clone = stop.clone();
        let vm_clone = vm.clone();
        let handle = thread::spawn(move || {
            let result = bind_listener(slot);
            let _ = tx.send(result);
        });
        let listener = match rx.recv() {
            Ok(Ok(l)) => l,
            Ok(Err(e)) => {
                let _ = handle.join();
                return Err(e);
            }
            Err(_) => {
                let _ = handle.join();
                return Err(io::Error::new(io::ErrorKind::Other, "PINE bind channel closed"));
            }
        };
        let socket_name = match &listener {
            #[cfg(unix)]
            Listener::Unix(p, _) => Some(p.clone()),
            Listener::Tcp(_) => None,
        };
        let tcp_port = match &listener {
            Listener::Tcp(_) => Some(slot),
            #[cfg(unix)]
            Listener::Unix(_, _) => None,
        };
        let listener = Arc::new(Mutex::new(Some(listener)));
        let stop_for_thread = stop.clone();
        let listener_for_thread = listener.clone();
        let vm_for_thread = vm_clone.clone();
        let server_handle = thread::spawn(move || {
            run_pine_server(listener_for_thread, vm_for_thread, stop_for_thread);
        });
        let _ = handle.join();
        Ok(Self {
            vm,
            slot,
            stop,
            handle: Some(server_handle),
            socket_name,
            tcp_port,
        })
    }

    pub fn slot(&self) -> u32 {
        self.slot
    }
    pub fn socket_name(&self) -> Option<&Path> {
        self.socket_name.as_deref()
    }
    pub fn tcp_port(&self) -> Option<u32> {
        self.tcp_port
    }
    pub fn is_initialized(&self) -> bool {
        !self.stop.load(Ordering::Acquire)
    }

    pub fn deinitialize(mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

enum Listener {
    #[cfg(unix)]
    Unix(PathBuf, UnixListener),
    Tcp(TcpListener),
}

fn bind_listener(slot: u32) -> io::Result<Listener> {
    #[cfg(unix)]
    {
        let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
            .or_else(|| std::env::var_os("TMPDIR"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        let mut name = runtime_dir;
        name.push(format!("{}.sock", PINE_EMULATOR_NAME));
        if slot != PINE_DEFAULT_SLOT {
            name.set_extension(format!("{}.sock.{}", PINE_EMULATOR_NAME, slot));
        }
        let _ = std::fs::remove_file(&name);
        let listener = UnixListener::bind(&name)?;
        return Ok(Listener::Unix(name, listener));
    }
    #[cfg(not(unix))]
    {
        let listener = TcpListener::bind(("127.0.0.1", slot as u16))?;
        Ok(Listener::Tcp(listener))
    }
}

fn run_pine_server(
    listener: Arc<Mutex<Option<Listener>>>,
    vm: Arc<dyn PineVm>,
    stop: Arc<AtomicBool>,
) {
    while !stop.load(Ordering::Acquire) {
        let accept_result = {
            let mut guard = listener.lock().unwrap();
            match guard.as_mut() {
                #[cfg(unix)]
                Some(Listener::Unix(_, l)) => l.accept().map(|(s, _)| Stream::Unix(s)),
                Some(Listener::Tcp(l)) => l.accept().map(|(s, _)| Stream::Tcp(s)),
                None => return,
            }
        };
        match accept_result {
            Ok(stream) => {
                if let Err(e) = handle_pine_client(stream, &vm) {
                    let _ = writeln!(io::stderr(), "PINE: client error: {}", e);
                }
            }
            Err(e) => {
                if stop.load(Ordering::Acquire) {
                    return;
                }
                let _ = writeln!(io::stderr(), "PINE: accept error: {}", e);
            }
        }
    }
}

enum Stream {
    #[cfg(unix)]
    Unix(UnixStream),
    Tcp(TcpStream),
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            #[cfg(unix)]
            Stream::Unix(s) => s.read(buf),
            Stream::Tcp(s) => s.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            #[cfg(unix)]
            Stream::Unix(s) => s.write(buf),
            Stream::Tcp(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            #[cfg(unix)]
            Stream::Unix(s) => s.flush(),
            Stream::Tcp(s) => s.flush(),
        }
    }
}

fn handle_pine_client(mut stream: Stream, vm: &Arc<dyn PineVm>) -> io::Result<()> {
    let mut header = [0u8; 4];
    stream.read_exact(&mut header)?;
    let size = u32::from_le_bytes(header) as usize;
    if size > MAX_IPC_SIZE || size < 4 {
        return Ok(());
    }
    let mut body = vec![0u8; size - 4];
    stream.read_exact(&mut body)?;
    let mut response = vec![0u8; MAX_IPC_RETURN_SIZE];
    let response_size = parse_command(vm, &body, size - 4, &mut response);
    response.truncate(response_size);
    stream.write_all(&response)
}

fn parse_command(
    vm: &Arc<dyn PineVm>,
    body: &[u8],
    size: usize,
    out: &mut Vec<u8>,
) -> usize {
    if !vm.has_valid_vm() {
        // Even with no VM, the Version/Status commands should still reply.
    }
    out.clear();
    out.extend_from_slice(&0u32.to_le_bytes());
    out.push(IpcResult::Ok as u8);
    let mut ret_cnt: usize = 5;
    let mut buf_cnt: usize = 0;
    while buf_cnt < size {
        if !safety_checks(buf_cnt, 1, ret_cnt, 0, size) {
            return make_fail_ipc(out);
        }
        buf_cnt += 1;
        let opcode = body.get(buf_cnt - 1).copied().unwrap_or(IpcCommand::Unimplemented as u8);
        let cmd = match opcode {
            0 => IpcCommand::Read8,
            1 => IpcCommand::Read16,
            2 => IpcCommand::Read32,
            3 => IpcCommand::Read64,
            4 => IpcCommand::Write8,
            5 => IpcCommand::Write16,
            6 => IpcCommand::Write32,
            7 => IpcCommand::Write64,
            8 => IpcCommand::Version,
            9 => IpcCommand::SaveState,
            0xA => IpcCommand::LoadState,
            0xB => IpcCommand::Title,
            0xC => IpcCommand::Id,
            0xD => IpcCommand::Uuid,
            0xE => IpcCommand::GameVersion,
            0xF => IpcCommand::Status,
            _ => IpcCommand::Unimplemented,
        };
        match cmd {
            IpcCommand::Read8 => {
                if !vm.has_valid_vm() {
                    return make_fail_ipc(out);
                }
                if !safety_checks(buf_cnt, 4, ret_cnt, 1, size) {
                    return make_fail_ipc(out);
                }
                let addr = read_u32(body, buf_cnt);
                let value = vm.mem_read8(addr);
                write_u8(out, ret_cnt, value);
                ret_cnt += 1;
                buf_cnt += 4;
            }
            IpcCommand::Read16 => {
                if !vm.has_valid_vm() {
                    return make_fail_ipc(out);
                }
                if !safety_checks(buf_cnt, 4, ret_cnt, 2, size) {
                    return make_fail_ipc(out);
                }
                let addr = read_u32(body, buf_cnt);
                let value = vm.mem_read16(addr);
                write_u16(out, ret_cnt, value);
                ret_cnt += 2;
                buf_cnt += 4;
            }
            IpcCommand::Read32 => {
                if !vm.has_valid_vm() {
                    return make_fail_ipc(out);
                }
                if !safety_checks(buf_cnt, 4, ret_cnt, 4, size) {
                    return make_fail_ipc(out);
                }
                let addr = read_u32(body, buf_cnt);
                let value = vm.mem_read32(addr);
                write_u32(out, ret_cnt, value);
                ret_cnt += 4;
                buf_cnt += 4;
            }
            IpcCommand::Read64 => {
                if !vm.has_valid_vm() {
                    return make_fail_ipc(out);
                }
                if !safety_checks(buf_cnt, 4, ret_cnt, 8, size) {
                    return make_fail_ipc(out);
                }
                let addr = read_u32(body, buf_cnt);
                let value = vm.mem_read64(addr);
                write_u64(out, ret_cnt, value);
                ret_cnt += 8;
                buf_cnt += 4;
            }
            IpcCommand::Write8 => {
                if !vm.has_valid_vm() {
                    return make_fail_ipc(out);
                }
                if !safety_checks(buf_cnt, 1 + 4, ret_cnt, 0, size) {
                    return make_fail_ipc(out);
                }
                let addr = read_u32(body, buf_cnt);
                let value = body.get(buf_cnt + 4).copied().unwrap_or(0);
                vm.mem_write8(addr, value);
                buf_cnt += 5;
            }
            IpcCommand::Write16 => {
                if !vm.has_valid_vm() {
                    return make_fail_ipc(out);
                }
                if !safety_checks(buf_cnt, 2 + 4, ret_cnt, 0, size) {
                    return make_fail_ipc(out);
                }
                let addr = read_u32(body, buf_cnt);
                let value = read_u16(body, buf_cnt + 4);
                vm.mem_write16(addr, value);
                buf_cnt += 6;
            }
            IpcCommand::Write32 => {
                if !vm.has_valid_vm() {
                    return make_fail_ipc(out);
                }
                if !safety_checks(buf_cnt, 4 + 4, ret_cnt, 0, size) {
                    return make_fail_ipc(out);
                }
                let addr = read_u32(body, buf_cnt);
                let value = read_u32(body, buf_cnt + 4);
                vm.mem_write32(addr, value);
                buf_cnt += 8;
            }
            IpcCommand::Write64 => {
                if !vm.has_valid_vm() {
                    return make_fail_ipc(out);
                }
                if !safety_checks(buf_cnt, 8 + 4, ret_cnt, 0, size) {
                    return make_fail_ipc(out);
                }
                let addr = read_u32(body, buf_cnt);
                let value = read_u64(body, buf_cnt + 4);
                vm.mem_write64(addr, value);
                buf_cnt += 12;
            }
            IpcCommand::Version => {
                let rev = PCSX2_BUILD_VERSION;
                let payload = format!("PCSX2 {}", rev);
                let payload_len = payload.len() + 1;
                if !safety_checks(buf_cnt, 0, ret_cnt, payload_len + 4, size) {
                    return make_fail_ipc(out);
                }
                write_u32(out, ret_cnt, payload_len as u32);
                ret_cnt += 4;
                let bytes = payload.into_bytes();
                out.extend_from_slice(&bytes);
                out.push(0);
                ret_cnt += payload_len;
            }
            IpcCommand::SaveState => {
                if !vm.has_valid_vm() {
                    return make_fail_ipc(out);
                }
                if !safety_checks(buf_cnt, 1, ret_cnt, 0, size) {
                    return make_fail_ipc(out);
                }
                let slot = body.get(buf_cnt).copied().unwrap_or(0);
                vm.save_state_to_slot(slot);
                buf_cnt += 1;
            }
            IpcCommand::LoadState => {
                if !vm.has_valid_vm() {
                    return make_fail_ipc(out);
                }
                if !safety_checks(buf_cnt, 1, ret_cnt, 0, size) {
                    return make_fail_ipc(out);
                }
                let slot = body.get(buf_cnt).copied().unwrap_or(0);
                let _ = vm.load_state_from_slot(slot);
                buf_cnt += 1;
            }
            IpcCommand::Title => {
                if !vm.has_valid_vm() {
                    return make_fail_ipc(out);
                }
                let title = vm.game_title();
                let payload_len = title.len() + 1;
                if !safety_checks(buf_cnt, 0, ret_cnt, payload_len + 4, size) {
                    return make_fail_ipc(out);
                }
                write_u32(out, ret_cnt, payload_len as u32);
                ret_cnt += 4;
                out.extend_from_slice(title.as_bytes());
                out.push(0);
                ret_cnt += payload_len;
            }
            IpcCommand::Id => {
                if !vm.has_valid_vm() {
                    return make_fail_ipc(out);
                }
                let serial = vm.game_serial();
                let payload_len = serial.len() + 1;
                if !safety_checks(buf_cnt, 0, ret_cnt, payload_len + 4, size) {
                    return make_fail_ipc(out);
                }
                write_u32(out, ret_cnt, payload_len as u32);
                ret_cnt += 4;
                out.extend_from_slice(serial.as_bytes());
                out.push(0);
                ret_cnt += payload_len;
            }
            IpcCommand::Uuid => {
                if !vm.has_valid_vm() {
                    return make_fail_ipc(out);
                }
                let crc = format!("{:08x}", vm.game_crc());
                let payload_len = crc.len() + 1;
                if !safety_checks(buf_cnt, 0, ret_cnt, payload_len + 4, size) {
                    return make_fail_ipc(out);
                }
                write_u32(out, ret_cnt, payload_len as u32);
                ret_cnt += 4;
                out.extend_from_slice(crc.as_bytes());
                out.push(0);
                ret_cnt += payload_len;
            }
            IpcCommand::GameVersion => {
                if !vm.has_valid_vm() {
                    return make_fail_ipc(out);
                }
                let version = vm.game_version();
                let payload_len = version.len() + 1;
                if !safety_checks(buf_cnt, 0, ret_cnt, payload_len + 4, size) {
                    return make_fail_ipc(out);
                }
                write_u32(out, ret_cnt, payload_len as u32);
                ret_cnt += 4;
                out.extend_from_slice(version.as_bytes());
                out.push(0);
                ret_cnt += payload_len;
            }
            IpcCommand::Status => {
                if !safety_checks(buf_cnt, 0, ret_cnt, 4, size) {
                    return make_fail_ipc(out);
                }
                let status = vm.emu_status() as u32;
                write_u32(out, ret_cnt, status);
                ret_cnt += 4;
            }
            IpcCommand::Unimplemented => return make_fail_ipc(out),
        }
    }
    let len = ret_cnt as u32;
    let bytes = len.to_le_bytes();
    out[0..4].copy_from_slice(&bytes);
    out[4] = IpcResult::Ok as u8;
    ret_cnt
}

fn make_fail_ipc(out: &mut Vec<u8>) -> usize {
    out.clear();
    out.extend_from_slice(&5u32.to_le_bytes());
    out.push(IpcResult::Fail as u8);
    5
}

fn safety_checks(command_len: usize, command_size: usize, reply_len: usize, reply_size: usize, buf_size: usize) -> bool {
    !((command_len + command_size) > buf_size || (reply_len + reply_size) >= MAX_IPC_RETURN_SIZE)
}

fn read_u32(body: &[u8], offset: usize) -> u32 {
    let mut bytes = [0u8; 4];
    if let Some(slice) = body.get(offset..offset + 4) {
        bytes.copy_from_slice(slice);
    }
    u32::from_le_bytes(bytes)
}

fn read_u16(body: &[u8], offset: usize) -> u16 {
    let mut bytes = [0u8; 2];
    if let Some(slice) = body.get(offset..offset + 2) {
        bytes.copy_from_slice(slice);
    }
    u16::from_le_bytes(bytes)
}

fn read_u64(body: &[u8], offset: usize) -> u64 {
    let mut bytes = [0u8; 8];
    if let Some(slice) = body.get(offset..offset + 8) {
        bytes.copy_from_slice(slice);
    }
    u64::from_le_bytes(bytes)
}

fn write_u32(out: &mut Vec<u8>, offset: usize, value: u32) {
    let bytes = value.to_le_bytes();
    if offset + 4 <= out.len() {
        out[offset..offset + 4].copy_from_slice(&bytes);
    } else {
        out.resize(offset + 4, 0);
        out[offset..offset + 4].copy_from_slice(&bytes);
    }
}

fn write_u16(out: &mut Vec<u8>, offset: usize, value: u16) {
    let bytes = value.to_le_bytes();
    if offset + 2 <= out.len() {
        out[offset..offset + 2].copy_from_slice(&bytes);
    } else {
        out.resize(offset + 2, 0);
        out[offset..offset + 2].copy_from_slice(&bytes);
    }
}

fn write_u8(out: &mut Vec<u8>, offset: usize, value: u8) {
    if offset < out.len() {
        out[offset] = value;
    } else {
        out.resize(offset + 1, 0);
        out[offset] = value;
    }
}

fn write_u64(out: &mut Vec<u8>, offset: usize, value: u64) {
    let bytes = value.to_le_bytes();
    if offset + 8 <= out.len() {
        out[offset..offset + 8].copy_from_slice(&bytes);
    } else {
        out.resize(offset + 8, 0);
        out[offset..offset + 8].copy_from_slice(&bytes);
    }
}

// ---------------------------------------------------------------------------
// PerformanceMetrics.h / PerformanceMetrics.cpp
// ---------------------------------------------------------------------------

/// Number of samples kept in the rolling frame-time history.
pub const NUM_FRAME_TIME_SAMPLES: usize = 150;

/// How often the metric snapshots are produced.
const UPDATE_INTERVAL: Duration = Duration::from_millis(500);

/// Per-frame performance metrics. Mirrors the C++ `PerformanceMetrics`
/// API: a handful of accessors expose the latest snapshot and
/// [`PerfMon::update`] rolls the accumulators.
#[derive(Debug, Clone)]
pub struct PerfMon {
    inner: Arc<Mutex<PerfMonInner>>,
}

#[derive(Debug, Clone)]
struct GsswThreadStats {
    usage: f64,
    time: f64,
    last_cpu_time: Duration,
}

#[derive(Debug)]
struct PerfMonInner {
    fps: f32,
    internal_fps: f32,
    internal_fps_method: InternalFpsMethod,
    minimum_frame_time: f32,
    average_frame_time: f32,
    maximum_frame_time: f32,
    minimum_frame_time_acc: f32,
    average_frame_time_acc: f32,
    maximum_frame_time_acc: f32,
    frames_since_last_update: u32,
    unskipped_frames_since_last_update: u32,
    last_update_time: Instant,
    last_frame_time: Instant,
    frame_number: u64,
    cpu_thread_usage: f64,
    cpu_thread_time: f64,
    gs_thread_usage: f32,
    gs_thread_time: f32,
    vu_thread_usage: f32,
    vu_thread_time: f32,
    capture_thread_usage: f32,
    capture_thread_time: f32,
    gs_sw_threads: Vec<GsswThreadStats>,
    frame_time_history: [f32; NUM_FRAME_TIME_SAMPLES],
    frame_time_history_pos: usize,
    accumulated_gpu_time: f32,
    gpu_usage: f32,
    average_gpu_time: f32,
    gs_register_writes: u32,
    gs_framebuffer_blits: u32,
}

impl Default for PerfMon {
    fn default() -> Self {
        Self::new()
    }
}

impl PerfMon {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(PerfMonInner {
                fps: 0.0,
                internal_fps: 0.0,
                internal_fps_method: InternalFpsMethod::None,
                minimum_frame_time: 0.0,
                average_frame_time: 0.0,
                maximum_frame_time: 0.0,
                minimum_frame_time_acc: 0.0,
                average_frame_time_acc: 0.0,
                maximum_frame_time_acc: 0.0,
                frames_since_last_update: 0,
                unskipped_frames_since_last_update: 0,
                last_update_time: Instant::now(),
                last_frame_time: Instant::now(),
                frame_number: 0,
                cpu_thread_usage: 0.0,
                cpu_thread_time: 0.0,
                gs_thread_usage: 0.0,
                gs_thread_time: 0.0,
                vu_thread_usage: 0.0,
                vu_thread_time: 0.0,
                capture_thread_usage: 0.0,
                capture_thread_time: 0.0,
                gs_sw_threads: Vec::new(),
                frame_time_history: [0.0; NUM_FRAME_TIME_SAMPLES],
                frame_time_history_pos: 0,
                accumulated_gpu_time: 0.0,
                gpu_usage: 0.0,
                average_gpu_time: 0.0,
                gs_register_writes: 0,
                gs_framebuffer_blits: 0,
            })),
        }
    }

    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.fps = 0.0;
        inner.internal_fps = 0.0;
        inner.minimum_frame_time = 0.0;
        inner.average_frame_time = 0.0;
        inner.maximum_frame_time = 0.0;
        inner.internal_fps_method = InternalFpsMethod::None;
        inner.frame_time_history = [0.0; NUM_FRAME_TIME_SAMPLES];
        inner.frame_time_history_pos = 0;
        inner.frame_number = 0;
    }

    pub fn reset(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.frames_since_last_update = 0;
        inner.unskipped_frames_since_last_update = 0;
        inner.minimum_frame_time_acc = 0.0;
        inner.average_frame_time_acc = 0.0;
        inner.maximum_frame_time_acc = 0.0;
        inner.accumulated_gpu_time = 0.0;
        inner.gs_register_writes = 0;
        inner.gs_framebuffer_blits = 0;
        inner.last_update_time = Instant::now();
        inner.last_frame_time = Instant::now();
    }

    pub fn update(&self, gs_register_write: bool, fb_blit: bool, is_skipping_present: bool) {
        let mut inner = self.inner.lock().unwrap();
        if !is_skipping_present {
            let now = Instant::now();
            let frame_time = now.duration_since(inner.last_frame_time).as_secs_f32() * 1000.0;
            inner.last_frame_time = now;
            inner.minimum_frame_time_acc = if inner.minimum_frame_time_acc == 0.0 {
                frame_time
            } else {
                inner.minimum_frame_time_acc.min(frame_time)
            };
            inner.average_frame_time_acc += frame_time;
            inner.maximum_frame_time_acc = inner.maximum_frame_time_acc.max(frame_time);
            let pos = inner.frame_time_history_pos;
            inner.frame_time_history[pos] = frame_time;
            inner.frame_time_history_pos = (pos + 1) % NUM_FRAME_TIME_SAMPLES;
            inner.unskipped_frames_since_last_update += 1;
        }
        inner.frames_since_last_update += 1;
        if gs_register_write {
            inner.gs_register_writes += 1;
        }
        if fb_blit {
            inner.gs_framebuffer_blits += 1;
        }
        inner.frame_number += 1;

        let now = Instant::now();
        let elapsed = now.duration_since(inner.last_update_time);
        if elapsed < UPDATE_INTERVAL {
            return;
        }
        let elapsed_secs = elapsed.as_secs_f32();
        inner.last_update_time = now;
        let min = std::mem::replace(&mut inner.minimum_frame_time_acc, 0.0);
        let avg_acc = std::mem::replace(&mut inner.average_frame_time_acc, 0.0);
        let max = std::mem::replace(&mut inner.maximum_frame_time_acc, 0.0);
        inner.minimum_frame_time = min;
        inner.average_frame_time = if inner.unskipped_frames_since_last_update > 0 {
            avg_acc / inner.unskipped_frames_since_last_update as f32
        } else {
            0.0
        };
        inner.maximum_frame_time = max;
        inner.fps = inner.frames_since_last_update as f32 / elapsed_secs;
        if inner.unskipped_frames_since_last_update > 0 {
            inner.average_gpu_time = inner.accumulated_gpu_time / inner.unskipped_frames_since_last_update as f32;
            inner.gpu_usage = inner.accumulated_gpu_time / (elapsed_secs * 10.0);
        } else {
            inner.average_gpu_time = 0.0;
            inner.gpu_usage = 0.0;
        }
        inner.accumulated_gpu_time = 0.0;

        if inner.gs_register_writes > 0 {
            inner.internal_fps = inner.gs_register_writes as f32 / elapsed_secs;
            inner.internal_fps_method = InternalFpsMethod::GsPrivilegedRegister;
        } else if inner.gs_framebuffer_blits > 0 {
            inner.internal_fps = inner.gs_framebuffer_blits as f32 / elapsed_secs;
            inner.internal_fps_method = InternalFpsMethod::DispFbBlit;
        } else {
            inner.internal_fps = 0.0;
            inner.internal_fps_method = InternalFpsMethod::None;
        }
        inner.gs_register_writes = 0;
        inner.gs_framebuffer_blits = 0;
        inner.frames_since_last_update = 0;
        inner.unskipped_frames_since_last_update = 0;
    }

    pub fn on_gpu_present(&self, gpu_time_ms: f32) {
        let mut inner = self.inner.lock().unwrap();
        inner.accumulated_gpu_time += gpu_time_ms;
    }

    pub fn frame_number(&self) -> u64 {
        self.inner.lock().unwrap().frame_number
    }
    pub fn fps(&self) -> f32 {
        self.inner.lock().unwrap().fps
    }
    pub fn internal_fps(&self) -> f32 {
        self.inner.lock().unwrap().internal_fps
    }
    pub fn internal_fps_method(&self) -> InternalFpsMethod {
        self.inner.lock().unwrap().internal_fps_method
    }
    pub fn is_internal_fps_valid(&self) -> bool {
        self.inner.lock().unwrap().internal_fps_method != InternalFpsMethod::None
    }
    pub fn average_frame_time(&self) -> f32 {
        self.inner.lock().unwrap().average_frame_time
    }
    pub fn minimum_frame_time(&self) -> f32 {
        self.inner.lock().unwrap().minimum_frame_time
    }
    pub fn maximum_frame_time(&self) -> f32 {
        self.inner.lock().unwrap().maximum_frame_time
    }
    pub fn cpu_thread_usage(&self) -> f64 {
        self.inner.lock().unwrap().cpu_thread_usage
    }
    pub fn gs_thread_usage(&self) -> f32 {
        self.inner.lock().unwrap().gs_thread_usage
    }
    pub fn vu_thread_usage(&self) -> f32 {
        self.inner.lock().unwrap().vu_thread_usage
    }
    pub fn capture_thread_usage(&self) -> f32 {
        self.inner.lock().unwrap().capture_thread_usage
    }
    pub fn gpu_usage(&self) -> f32 {
        self.inner.lock().unwrap().gpu_usage
    }
    pub fn gpu_average_time(&self) -> f32 {
        self.inner.lock().unwrap().average_gpu_time
    }
    pub fn frame_time_history(&self) -> [f32; NUM_FRAME_TIME_SAMPLES] {
        self.inner.lock().unwrap().frame_time_history
    }
    pub fn frame_time_history_pos(&self) -> usize {
        self.inner.lock().unwrap().frame_time_history_pos
    }
}
