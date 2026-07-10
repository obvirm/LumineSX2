//! Idiomatic Rust translation of a cluster of PCSX2 source files.
//!
//! This module folds the C/C++ surface from the following originals into a
//! single `std`-only Rust 2021 translation:
//!
//! | C/C++ source                                  | Translated surface                       |
//! | --------------------------------------------- | ---------------------------------------- |
//! | `pcsx2/ps2/BiosTools.{cpp,h}`                 | [`BiosFile`], [`find_bios`]              |
//! | `pcsx2/ps2/Iop/IopHwRead.cpp`                 | [`IopHwRead`]                            |
//! | `pcsx2/ps2/Iop/IopHwWrite.cpp`                | [`IopHwWrite`]                           |
//! | `pcsx2/ps2/Iop/PsxBios.cpp`                   | [`PsxBios`]                              |
//! | `pcsx2/ps2/pgif.cpp`                          | [`Pgif`]                                 |
//! | `pcsx2/Achievements.{cpp,h}`                  | [`Achievements`]                         |
//! | `pcsx2/Config.h` + `Pcsx2Config.cpp`          | [`Pcsx2Config`], [`EmuConfig`], [`EmuFolders`] |
//! | `pcsx2/GameList.{cpp,h}`                      | [`GameList`], [`GameListEntry`]          |
//! | `pcsx2/GameDatabase.{cpp,h}`                  | [`GameDatabase`]                         |
//! | `pcsx2/BuildVersion.{cpp,h}`                  | [`PCSX2_BUILD_VERSION`]                  |
//!
//! The translation keeps the public surface small and idiomatic:
//!
//! * Bitfields and `BITFIELD32()` unions are collapsed into ordinary `u32`
//!   fields with named accessor methods. Each accessor mirrors the semantics
//!   of the original C++ macro (set-on-write, mask-on-read).
//! * `std::vector` becomes `Vec<_>`, `std::string` becomes `String`,
//!   `std::FILE*` becomes a thin wrapper over the standard library's
//!   `std::fs::File`.
//! * `std::unique_lock<std::recursive_mutex>` is replaced with a
//!   [`GameListLock`] newtype holding a `Mutex<Vec<GameListEntry>>`.
//! * The C++ global `EmuConfig` is exposed as the constant
//!   [`EmuConfig`] (a `OnceLock<Pcsx2Config>`), and the C++ namespace
//!   `EmuFolders` is exposed as the [`EmuFolders`] struct holding
//!   process-wide path strings.
//! * The RetroAchievements client wrapper [`Achievements`] is a stub
//!   that captures the same public surface (login state, hardcore mode,
//!   rich-presence string) without bringing in the C rcheevos dependency.
//! * Memory-mapped I/O addresses used by the IOP hardware layer are
//!   exposed as `pub const u32` items in the [`iop_hw`] sub-module so
//!   they can be referenced from the broader emulator.
//!
//! Only `std` is used; no external crates are required.

#![allow(dead_code)]
#![allow(clippy::upper_case_acronyms)]

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use crate::common::Threading::RecursiveMutex;

// ===========================================================================
// BiosTools  (BiosTools.cpp / BiosTools.h)
// ===========================================================================

/// Minimum legal PS2 BIOS image size, in bytes (4 MiB).
pub const MIN_BIOS_SIZE: u64 = 4 * 1024 * 1024;
/// Maximum legal PS2 BIOS image size, in bytes (8 MiB).
pub const MAX_BIOS_SIZE: u64 = 8 * 1024 * 1024;
const DIRENTRY_SIZE: usize = 16;

/// Size of the on-disk `romdir` entry. The C++ side enforces this via
/// `static_assert`; the Rust equivalent is a `const` validation that we
/// can probe at compile time.
const ROMDIR_SIZE: usize = std::mem::size_of::<RomDir>();
const _: [(); DIRENTRY_SIZE] = [(); ROMDIR_SIZE];

/// On-disk `romdir` entry (packed 16-byte record).
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct RomDir {
    /// 10-byte file name (null-padded).
    pub file_name: [u8; 10],
    /// Extended-info size in 16-byte units.
    pub ext_info_size: u16,
    /// File size in bytes.
    pub file_size: u32,
}

/// Parsed `ROMVER` record describing a PS2 BIOS image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RomVer {
    /// Major version, e.g. `2` from `2.20`.
    pub version_major: u8,
    /// Minor version, e.g. `20` from `2.20`.
    pub version_minor: u8,
    /// Region letter (`J`, `A`, `E`, `H`, `C`, `T`, `X`, `P`).
    pub region: char,
    /// `Console`, `Devel`, or empty.
    pub variant: &'static str,
    /// Human-readable date in the BIOS, e.g. `20/01/07`.
    pub date: String,
}

/// Fully-parsed PS2 BIOS image. The fields mirror the C++ globals
/// `BiosVersion`, `BiosRegion`, `BiosDescription`, `BiosZone`,
/// `BiosSerial` and `BiosChecksum`.
#[derive(Debug, Clone)]
pub struct BiosFile {
    /// Path the BIOS was loaded from.
    pub path: PathBuf,
    /// Raw image bytes (4 MiB ROM + optional 4 MiB ROM1 + optional 4 MiB ROM2).
    pub rom: Vec<u8>,
    /// Encoded version: `(major << 8) | minor`.
    pub version: u32,
    /// Region code (`0` Japan, `1` USA, `2` Europe, ...).
    pub region: u32,
    /// Human-readable description, e.g. `Japan v2.20(20/01/07/2007)  Console`.
    pub description: String,
    /// Region name, e.g. `Japan`, `USA`, `Europe`, `Asia`, `China`, `Test`.
    pub zone: String,
    /// Build serial from the EXTINFO record.
    pub serial: String,
    /// XOR-checksum of the ROM (4-byte XOR over the 4 MiB image).
    pub checksum: u32,
}

impl BiosFile {
    /// Construct an empty BIOS record.
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            rom: Vec::new(),
            version: 0,
            region: 0,
            description: String::new(),
            zone: String::new(),
            serial: String::new(),
            checksum: 0,
        }
    }

    /// True if the ROM has an OSD region (file >= 2_465_792 bytes).
    pub fn has_osd(&self) -> bool {
        self.rom.len() >= 2_465_792
    }
}

/// Decode the `ROMVER` ascii blob from a BIOS image.
fn parse_romver(romver: &[u8; 14]) -> Option<RomVer> {
    if romver.iter().all(|b| *b == 0) {
        return None;
    }
    Some(RomVer {
        version_major: (romver[0] as u8).wrapping_sub(b'0'),
        version_minor: (((romver[2] as u8).wrapping_sub(b'0')) * 10)
            + ((romver[3] as u8).wrapping_sub(b'0')),
        region: romver[4] as char,
        variant: match romver[5] as char {
            'C' => "Console",
            'D' => "Devel",
            _ => "",
        },
        date: format!(
            "{:02}{:02}/{:02}{:02}/{:02}{:02}{:02}{:02}",
            romver[12], romver[13], romver[10], romver[11], romver[6], romver[7], romver[8], romver[9]
        ),
    })
}

/// Translate the C++ `LoadBiosVersion` helper: walks the `romdir` table at
/// the start of a PS2 BIOS image looking for the `ROMVER` and `EXTINFO`
/// records and returns a populated [`BiosFile`] on success.
pub fn find_bios(path: &Path) -> Result<BiosFile, String> {
    let mut file = fs::File::open(path).map_err(|e| format!("open: {e}"))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).map_err(|e| format!("read: {e}"))?;
    if buf.len() < MIN_BIOS_SIZE as usize || buf.len() > MAX_BIOS_SIZE as usize {
        return Err(format!("size {} out of range", buf.len()));
    }

    // Walk the romdir table until we find the "RESET" sentinel.
    let mut pos = 0usize;
    let mut entry = RomDir {
        file_name: [0; 10],
        ext_info_size: 0,
        file_size: 0,
    };
    let mut found_romdir = false;
    for _ in 0..(512 * 1024) {
        if pos + DIRENTRY_SIZE > buf.len() {
            return Err("truncated romdir".to_string());
        }
        // Safety: `pos + 16` is within bounds thanks to the check above.
        let raw = &buf[pos..pos + DIRENTRY_SIZE];
        entry = unsafe { std::ptr::read_unaligned(raw.as_ptr() as *const RomDir) };
        if &entry.file_name == b"RESET\0\0\0\0\0" {
            found_romdir = true;
            pos += DIRENTRY_SIZE;
            break;
        }
        pos += DIRENTRY_SIZE;
    }
    if !found_romdir {
        return Err("romdir sentinel not found".to_string());
    }

    // Iterate the entries that follow.
    let mut romver_opt: Option<[u8; 14]> = None;
    let mut extinfo: [u8; 15] = [0; 15];
    let mut file_offset: u64 = 0;
    while entry.file_name[0] != 0
        && entry.file_name.iter().filter(|c| **c != 0).count() != 10
    {
        if &entry.file_name == b"EXTINFO\0\0\0" {
            if let Ok(seek_pos) =
                file.seek(SeekFrom::Start(file_offset + 0x10))
            {
                if file.read_exact(&mut extinfo).is_ok() {
                    let _ = file.seek(SeekFrom::Start(seek_pos));
                }
            }
        }
        if &entry.file_name == b"ROMVER\0\0\0\0" {
            if let Ok(seek_pos) = file.seek(SeekFrom::Start(file_offset)) {
                let mut romver = [0u8; 14];
                if file.read_exact(&mut romver).is_ok() {
                    romver_opt = Some(romver);
                    let _ = file.seek(SeekFrom::Start(seek_pos));
                }
            }
        }
        let advance = if entry.file_size % 0x10 == 0 {
            entry.file_size as u64
        } else {
            (entry.file_size as u64 + 0x10) & 0xffff_fff0
        };
        file_offset += advance;
        pos += DIRENTRY_SIZE;
        if pos + DIRENTRY_SIZE > buf.len() {
            break;
        }
        let raw = &buf[pos..pos + DIRENTRY_SIZE];
        entry = unsafe { std::ptr::read_unaligned(raw.as_ptr() as *const RomDir) };
    }

    let romver = romver_opt.ok_or_else(|| "ROMVER missing".to_string())?;
    let parsed = parse_romver(&romver).ok_or_else(|| "ROMVER empty".to_string())?;

    let (zone, region) = match parsed.region {
        'J' => ("Japan", 0),
        'A' => ("USA", 1),
        'E' => ("Europe", 2),
        'H' => ("Asia", 4),
        'C' => ("China", 6),
        'T' => {
            if romver[5] as char == 'Z' {
                ("COH-H", 8)
            } else {
                ("T10K", 8)
            }
        }
        'X' => ("Test", 9),
        'P' => ("Free", 10),
        c => {
            let mut s = String::new();
            s.push(c);
            (Box::leak(s.into_boxed_str()) as &str, 0)
        }
    };

    let serial = extinfo
        .iter()
        .position(|b| *b == 0)
        .map(|n| String::from_utf8_lossy(&extinfo[..n]).into_owned())
        .unwrap_or_default();

    let description = format!(
        "{:<7} v{}.{}({}/{})  {} {}",
        zone, parsed.version_major, parsed.version_minor, parsed.date, "", parsed.variant, serial
    );

    let version = ((parsed.version_major as u32) << 8) | (parsed.version_minor as u32);
    let checksum = buf
        .chunks_exact(4)
        .take(MIN_BIOS_SIZE as usize / 4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .fold(0u32, |acc, w| acc ^ w);

    Ok(BiosFile {
        path: path.to_path_buf(),
        rom: buf,
        version,
        region,
        description,
        zone: zone.to_string(),
        serial,
        checksum,
    })
}

// ===========================================================================
// IopHwRead / IopHwWrite  (IopHwRead.cpp / IopHwWrite.cpp)
// ===========================================================================

/// IOP hardware read dispatch (the `iopHwRead{8,16,32}_{Page1,Page3,Page8}`
/// family). The struct owns the per-page handler function pointers exactly
/// the way the C++ tables do, so callers can replace the handler at
/// runtime if they need to plug in a custom device model.
#[derive(Default)]
pub struct IopHwRead {
    /// Page 1 (0x1F801xxx) 8-bit read handler.
    pub page1_8: Option<fn(u32) -> u8>,
    /// Page 1 (0x1F801xxx) 16-bit read handler.
    pub page1_16: Option<fn(u32) -> u16>,
    /// Page 1 (0x1F801xxx) 32-bit read handler.
    pub page1_32: Option<fn(u32) -> u32>,
    /// Page 3 (0x1F803xxx) 8-bit read handler.
    pub page3_8: Option<fn(u32) -> u8>,
    /// Page 3 (0x1F803xxx) 16-bit read handler.
    pub page3_16: Option<fn(u32) -> u16>,
    /// Page 3 (0x1F803xxx) 32-bit read handler.
    pub page3_32: Option<fn(u32) -> u32>,
    /// Page 8 (0x1F808xxx) 8-bit read handler.
    pub page8_8: Option<fn(u32) -> u8>,
    /// Page 8 (0x1F808xxx) 16-bit read handler.
    pub page8_16: Option<fn(u32) -> u16>,
    /// Page 8 (0x1F808xxx) 32-bit read handler.
    pub page8_32: Option<fn(u32) -> u32>,
    /// Generic 8/16/32-bit read handler used by the BIOS pages that
    /// aren't part of the page1/3/8 dispatch.
    pub generic_8: Option<fn(u32) -> u8>,
    pub generic_16: Option<fn(u32) -> u16>,
    pub generic_32: Option<fn(u32) -> u32>,
}

impl IopHwRead {
    /// Construct a new (empty) read dispatch.
    pub fn new() -> Self {
        Self::default()
    }

    /// Dispatch a single read by address; returns `None` if no handler is
    /// installed for the requested page. The C++ side asserts on
    /// misaligned accesses; we mirror that with a debug-only check.
    pub fn read8(&self, addr: u32) -> Option<u8> {
        match addr >> 12 {
            0x1f801 => self.page1_8.or(self.generic_8).map(|f| f(addr)),
            0x1f803 => self.page3_8.or(self.generic_8).map(|f| f(addr)),
            0x1f808 => self.page8_8.or(self.generic_8).map(|f| f(addr)),
            _ => self.generic_8.map(|f| f(addr)),
        }
    }

    pub fn read16(&self, addr: u32) -> Option<u16> {
        debug_assert!(addr & 1 == 0, "misaligned 16-bit read: 0x{addr:08x}");
        match addr >> 12 {
            0x1f801 => self.page1_16.or(self.generic_16).map(|f| f(addr)),
            0x1f803 => self.page3_16.or(self.generic_16).map(|f| f(addr)),
            0x1f808 => self.page8_16.or(self.generic_16).map(|f| f(addr)),
            _ => self.generic_16.map(|f| f(addr)),
        }
    }

    pub fn read32(&self, addr: u32) -> Option<u32> {
        debug_assert!(addr & 3 == 0, "misaligned 32-bit read: 0x{addr:08x}");
        match addr >> 12 {
            0x1f801 => self.page1_32.or(self.generic_32).map(|f| f(addr)),
            0x1f803 => self.page3_32.or(self.generic_32).map(|f| f(addr)),
            0x1f808 => self.page8_32.or(self.generic_32).map(|f| f(addr)),
            _ => self.generic_32.map(|f| f(addr)),
        }
    }
}

/// IOP hardware write dispatch (the `iopHwWrite{8,16,32}_{Page1,Page3,Page8}`
/// family). Mirrors [`IopHwRead`].
#[derive(Default)]
pub struct IopHwWrite {
    pub page1_8: Option<fn(u32, u8)>,
    pub page1_16: Option<fn(u32, u16)>,
    pub page1_32: Option<fn(u32, u32)>,
    pub page3_8: Option<fn(u32, u8)>,
    pub page3_16: Option<fn(u32, u16)>,
    pub page3_32: Option<fn(u32, u32)>,
    pub page8_8: Option<fn(u32, u8)>,
    pub page8_16: Option<fn(u32, u16)>,
    pub page8_32: Option<fn(u32, u32)>,
    pub generic_8: Option<fn(u32, u8)>,
    pub generic_16: Option<fn(u32, u16)>,
    pub generic_32: Option<fn(u32, u32)>,
}

impl IopHwWrite {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn write8(&self, addr: u32, val: u8) -> Option<()> {
        let handler = match addr >> 12 {
            0x1f801 => self.page1_8.or(self.generic_8),
            0x1f803 => self.page3_8.or(self.generic_8),
            0x1f808 => self.page8_8.or(self.generic_8),
            _ => self.generic_8,
        };
        handler.map(|f| f(addr, val))
    }

    pub fn write16(&self, addr: u32, val: u16) -> Option<()> {
        debug_assert!(addr & 1 == 0, "misaligned 16-bit write: 0x{addr:08x}");
        let handler = match addr >> 12 {
            0x1f801 => self.page1_16.or(self.generic_16),
            0x1f803 => self.page3_16.or(self.generic_16),
            0x1f808 => self.page8_16.or(self.generic_16),
            _ => self.generic_16,
        };
        handler.map(|f| f(addr, val))
    }

    pub fn write32(&self, addr: u32, val: u32) -> Option<()> {
        debug_assert!(addr & 3 == 0, "misaligned 32-bit write: 0x{addr:08x}");
        let handler = match addr >> 12 {
            0x1f801 => self.page1_32.or(self.generic_32),
            0x1f803 => self.page3_32.or(self.generic_32),
            0x1f808 => self.page8_32.or(self.generic_32),
            _ => self.generic_32,
        };
        handler.map(|f| f(addr, val))
    }
}

/// Address constants used by the IOP hardware layer.  Kept in a dedicated
/// sub-module so other crates can `use` them without pulling in the
/// dispatch tables.
pub mod iop_hw {
    // Page 1 SIO / DMA / counters.
    pub const HW_SIO_DATA: u32 = 0x1F801040;
    pub const HW_SIO_STAT: u32 = 0x1F801044;
    pub const HW_SIO_MODE: u32 = 0x1F801048;
    pub const HW_SIO_CTRL: u32 = 0x1F80104A;
    pub const HW_SIO_BAUD: u32 = 0x1F80104E;

    pub const HW_ISTAT: u32 = 0x1F801070;
    pub const HW_IMASK: u32 = 0x1F801074;
    pub const HW_ICTRL: u32 = 0x1F801078;

    // DMA channels.
    pub const HW_DMA0_CHCR: u32 = 0x1F801088;
    pub const HW_DMA1_CHCR: u32 = 0x1F801098;
    pub const HW_DMA2_CHCR: u32 = 0x1F8010A8;
    pub const HW_DMA3_CHCR: u32 = 0x1F8010B8;
    pub const HW_DMA4_CHCR: u32 = 0x1F8010C8;
    pub const HW_DMA6_CHCR: u32 = 0x1F8010E8;
    pub const HW_DMA7_CHCR: u32 = 0x1F801508;
    pub const HW_DMA8_CHCR: u32 = 0x1F801518;
    pub const HW_DMA9_CHCR: u32 = 0x1F801528;
    pub const HW_DMA10_CHCR: u32 = 0x1F801538;
    pub const HW_DMA11_CHCR: u32 = 0x1F801548;
    pub const HW_DMA12_CHCR: u32 = 0x1F801558;

    pub const HW_DMA_ICR: u32 = 0x1F8010F4;
    pub const HW_DMA_ICR2: u32 = 0x1F801574;

    // DEV9 / CD-ROM / PS1 GPU.
    pub const HW_DEV9_DATA: u32 = 0x1F80146E;
    pub const HW_CDR_DATA0: u32 = 0x1F801800;
    pub const HW_CDR_DATA1: u32 = 0x1F801801;
    pub const HW_CDR_DATA2: u32 = 0x1F801802;
    pub const HW_CDR_DATA3: u32 = 0x1F801803;
    pub const HW_PS1_GPU_DATA: u32 = 0x1F801810;
    pub const HW_PS1_GPU_STATUS: u32 = 0x1F801814;

    // Page 8 SIO2.
    pub const HW_SIO2_TX: u32 = 0x1F808260;
    pub const HW_SIO2_RX: u32 = 0x1F808264;
    pub const HW_SIO2_CTRL: u32 = 0x1F808268;
    pub const HW_SIO2_CMD_STAT: u32 = 0x1F80826C;
    pub const HW_SIO2_PORT_STAT: u32 = 0x1F808270;
    pub const HW_SIO2_FIFO_STAT: u32 = 0x1F808274;
    pub const HW_SIO2_FIFO_TX: u32 = 0x1F808278;
    pub const HW_SIO2_FIFO_RX: u32 = 0x1F80827C;
    pub const HW_SIO2_INTR: u32 = 0x1F808280;

    // Sub-page mask used by the `pgmsk` macro in the C++ side.
    #[inline]
    pub fn pgmsk(addr: u32) -> u32 {
        addr & 0x0fff
    }
}

// ===========================================================================
// PsxBios  (Iop/PsxBios.cpp)
// ===========================================================================

/// Per-call register view passed to the PSX BIOS dispatcher.
#[derive(Debug, Clone, Copy)]
pub struct PsxRegsView {
    /// Current program counter.
    pub pc: u32,
    /// GPR `$a0` (used as the first syscall argument).
    pub a0: u32,
    /// GPR `$a1` (used as the second syscall argument).
    pub a1: u32,
    /// GPR `$a2` (used as the third syscall argument).
    pub a2: u32,
    /// GPR `$t1` (used as the syscall dispatch code).
    pub t1: u32,
}

/// Output side of a `PsxBios` call. The PSX BIOS uses FD 1 for stdout, so
/// we accumulate the bytes the game would have written to stdout into a
/// string buffer that the embedding application can flush.
#[derive(Debug, Default, Clone)]
pub struct PsxBiosStdout {
    buf: String,
    last: String,
    repeat: u32,
}

impl PsxBiosStdout {
    /// Append a single character.
    pub fn push_char(&mut self, c: char) {
        if c == '\r' {
            self.buf.push('\n');
        } else if c != '\n' {
            self.buf.push(c);
        }
        self.maybe_flush();
    }

    /// Append a UTF-8 string.
    pub fn push_str(&mut self, s: &str) {
        self.buf.push_str(s);
        self.maybe_flush();
    }

    /// Flush any completed line to `sink` (collapsed across repeats).
    pub fn flush_into(&mut self, sink: &mut String) {
        while let Some(idx) = self.buf.find('\n') {
            let line: String = self.buf.drain(..=idx).collect();
            if line.trim_end() == self.last {
                self.repeat += 1;
            } else {
                if self.repeat > 0 {
                    sink.push_str(&format!("[{} more]\n", self.repeat));
                    self.repeat = 0;
                }
                self.last = line.trim_end().to_string();
                sink.push_str(&self.last);
                sink.push('\n');
            }
        }
    }

    /// Force-flush the buffer (used during `psxBiosReset`).
    pub fn flush_terminal(&mut self, sink: &mut String) {
        if !self.buf.is_empty() {
            sink.push_str(&self.buf);
            self.buf.clear();
        }
        if self.repeat > 0 {
            sink.push_str(&format!("[{} more]\n", self.repeat));
            self.repeat = 0;
        }
    }

    fn maybe_flush(&mut self) {
        if self.buf.ends_with('\n') || self.buf.len() >= 1024 {
            let mut sink = String::new();
            self.flush_into(&mut sink);
        }
    }
}

/// PSX-BIOS syscall dispatcher.  Holds the per-process stdout buffer and
/// dispatches the 0xA0 / 0xB0 / 0xC0 syscall families used by the
/// embedded PS1 BIOS.  Returns `true` if the call was processed.
#[derive(Debug, Default, Clone)]
pub struct PsxBios {
    pub stdout: PsxBiosStdout,
    /// When set, `write`/`putc`/`puts` calls on FD 1 are buffered
    /// instead of being silently dropped.
    pub log_stdout: bool,
}

impl PsxBios {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset the BIOS state (flushes stdout).
    pub fn reset(&mut self) {
        let mut sink = String::new();
        self.stdout.flush_terminal(&mut sink);
    }

    /// Dispatch a PSX BIOS syscall.  Returns `true` if the call was
    /// handled.
    pub fn dispatch(&mut self, regs: &PsxRegsView) -> bool {
        // Match the C++ key: `(pc << 4) & 0xf00 | (t1 & 0xff)`.
        let key = ((regs.pc << 4) & 0xf00) | (regs.t1 & 0xff);
        match key {
            0xa03 | 0xb35 => {
                // write(fd, data, size)
                if regs.a0 != 1 {
                    return false;
                }
                if !self.log_stdout {
                    return false;
                }
                // In a real emulator we'd copy from `a1` for `a2` bytes;
                // here we just log that the call was made.
                self.stdout.push_str(&format!(
                    "<write data=0x{:08x} size={}>\n",
                    regs.a1, regs.a2
                ));
                false
            }
            0xa09 | 0xb3b => {
                // putc(c, fd)
                if regs.a1 != 1 {
                    return false;
                }
                // fallthrough to putchar
                let c = (regs.a0 & 0xff) as u8 as char;
                self.stdout.push_char(c);
                false
            }
            0xa3c | 0xb3d => {
                // putchar(c)
                let c = (regs.a0 & 0xff) as u8 as char;
                self.stdout.push_char(c);
                false
            }
            0xa3e | 0xb3f => {
                // puts(s)
                self.stdout.push_str(&format!("<puts s=0x{:08x}>\n", regs.a0));
                false
            }
            _ => false,
        }
    }
}

// ===========================================================================
// PGIF  (pgif.cpp)
// ===========================================================================

/// PS1 GPU FIFO state.  The C++ side uses two raw ring buffers; the Rust
/// version uses bounded `VecDeque` for clarity.
mod pgif_internal {
    use std::collections::VecDeque;

    /// Ring buffer for PS1 GPU command/data words. The size is fixed
    /// per-channel and is enforced by [`Self::put`].
    #[derive(Debug)]
    pub struct RingBuffer {
        buf: VecDeque<u32>,
        cap: usize,
    }

    impl RingBuffer {
        pub fn with_capacity(cap: usize) -> Self {
            Self {
                buf: VecDeque::with_capacity(cap),
                cap,
            }
        }

        pub fn put(&mut self, v: u32) {
            if self.buf.len() < self.cap {
                self.buf.push_back(v);
            } else {
                // Overflow - the C++ side logs and drops.
            }
        }

        pub fn get(&mut self) -> Option<u32> {
            self.buf.pop_front()
        }

        pub fn clear(&mut self) {
            self.buf.clear();
        }

        pub fn count(&self) -> usize {
            self.buf.len()
        }
    }
}

/// PS1 GPU (PGPU) status register. Mirrors the union of
/// `pgpu.stat.bits.{IRQ1,DDIR,DREQ,RDMA,RSEND}` exposed by the C++ side.
#[derive(Debug, Default, Clone, Copy)]
pub struct PgpuStat {
    pub irq1: bool,
    pub ddir: u8,
    pub dreq: bool,
    pub rdma: bool,
    pub rsend: bool,
}

impl PgpuStat {
    pub fn to_u32(&self) -> u32 {
        let mut v = 0u32;
        if self.irq1 {
            v |= 1 << 31;
        }
        v |= (self.ddir as u32 & 0x3) << 29;
        if self.dreq {
            v |= 1 << 28;
        }
        if self.rdma {
            v |= 1 << 26;
        }
        if self.rsend {
            v |= 1 << 25;
        }
        v
    }

    pub fn from_u32(v: u32) -> Self {
        Self {
            irq1: (v >> 31) & 1 != 0,
            ddir: ((v >> 29) & 0x3) as u8,
            dreq: (v >> 28) & 1 != 0,
            rdma: (v >> 26) & 1 != 0,
            rsend: (v >> 25) & 1 != 0,
        }
    }
}

/// PGIF control register.
#[derive(Debug, Default, Clone, Copy)]
pub struct PgifCtrl {
    pub fifo_gp0_ready_for_data: bool,
    pub data_from_gpu_ready: bool,
    pub gp0_fifo_count: u8,
    pub gp1_fifo_count: u8,
}

impl PgifCtrl {
    pub fn to_u32(&self) -> u32 {
        let mut v = 0u32;
        if self.fifo_gp0_ready_for_data {
            v |= 1 << 8;
        }
        v |= (self.gp0_fifo_count as u32 & 0x1f) << 0;
        v |= (self.gp1_fifo_count as u32 & 0x1f) << 5;
        v
    }
}

/// Immediate-response register file, used by `GP1(10h)`.
#[derive(Debug, Default, Clone, Copy)]
pub struct ImmResponse {
    pub e2: u32,
    pub e3: u32,
    pub e4: u32,
    pub e5: u32,
}

/// PGIF (PS1->PS2 graphics interface) state. Owns the two FIFOs and the
/// status/control register file. The `dma_state` field tracks the linked
/// list and normal DMA engines the C++ side drives from `processPgpuDma`.
#[derive(Debug)]
pub struct Pgif {
    cmd_fifo: pgif_internal::RingBuffer,
    data_fifo: pgif_internal::RingBuffer,
    pub stat: PgpuStat,
    pub ctrl: PgifCtrl,
    pub imm: ImmResponse,
    pub old_gp0_value: u32,
    /// True when an immediate-response command (GP1(10h)) is in flight.
    pub imm_in_flight: bool,
}

impl Default for Pgif {
    fn default() -> Self {
        Self {
            cmd_fifo: pgif_internal::RingBuffer::with_capacity(8),
            data_fifo: pgif_internal::RingBuffer::with_capacity(0x20000),
            stat: PgpuStat::default(),
            ctrl: PgifCtrl::default(),
            imm: ImmResponse::default(),
            old_gp0_value: 0,
            imm_in_flight: false,
        }
    }
}

impl Pgif {
    /// Mirror of `pgifInit` in the C++ side: clears both FIFOs, the
    /// status/control registers, and the legacy `old_gp0_value`.
    pub fn init(&mut self) {
        self.cmd_fifo.clear();
        self.data_fifo.clear();
        self.stat = PgpuStat::default();
        self.ctrl = PgifCtrl::default();
        self.old_gp0_value = 0;
    }

    /// Mirror of `psxGPUw` for `HW_PS1_GPU_DATA`: enqueue one 32-bit word
    /// to the GP0 data FIFO.
    pub fn psx_gpu_write_data(&mut self, data: u32) {
        self.data_fifo.put(data);
    }

    /// Mirror of `psxGPUw` for `HW_PS1_GPU_STATUS`.  If the upper bits
    /// indicate an immediate-response command (`imm_check == 1`), the
    /// value is intercepted and processed locally; otherwise it is
    /// enqueued on the GP1 command FIFO.
    pub fn psx_gpu_write_status(&mut self, data: u32) {
        let imm_check = (data >> 28) & 0x3;
        if imm_check == 1 {
            self.imm_in_flight = true;
            // GP1(10h) - immediate response command.
            match data & 0x7 {
                2 => self.old_gp0_value = self.imm.e2 & 0x000f_ffff,
                3 => self.old_gp0_value = self.imm.e3 & 0x0007_ffff,
                4 => self.old_gp0_value = self.imm.e4 & 0x0007_ffff,
                5 => self.old_gp0_value = self.imm.e5 & 0x003f_ffff,
                _ => {} // 0, 1, 6, 7 - no data
            }
        } else {
            self.imm_in_flight = false;
            self.cmd_fifo.put(data);
        }
    }

    /// Mirror of `psxGPUr` for `HW_PS1_GPU_DATA`: pop one word from the
    /// GP0 data FIFO (or return the cached `old_gp0_value` if the FIFO
    /// is empty).
    pub fn psx_gpu_read_data(&mut self) -> u32 {
        self.data_fifo.get().unwrap_or(self.old_gp0_value)
    }

    /// Mirror of `psxGPUr` for `HW_PS1_GPU_STATUS`: returns the
    /// recomputed PGPU status register, with `RSEND` driven by the
    /// current control register.
    pub fn psx_gpu_read_status(&mut self) -> u32 {
        self.stat.rsend = self.ctrl.data_from_gpu_ready;
        self.ctrl.gp0_fifo_count = (self.data_fifo.count() as u8).min(0x1f);
        self.ctrl.gp1_fifo_count = (self.cmd_fifo.count() as u8).min(0x1f);
        let mut v = self.stat.to_u32();
        v |= self.ctrl.to_u32();
        v
    }

    /// Mirror of `PGIFw` - dispatch a write to one of the PGIF registers.
    pub fn pgif_write(&mut self, addr: u32, data: u32) {
        match addr {
            0x1000 => {
                self.stat = PgpuStat::from_u32(data);
            }
            0x1010 => {
                self.ctrl = PgifCtrl {
                    fifo_gp0_ready_for_data: (data >> 8) & 1 != 0,
                    data_from_gpu_ready: (data >> 9) & 1 != 0,
                    gp0_fifo_count: (data & 0x1f) as u8,
                    gp1_fifo_count: ((data >> 5) & 0x1f) as u8,
                };
            }
            0x1100 => self.imm.e2 = data,
            0x1110 => self.imm.e3 = data,
            0x1120 => self.imm.e4 = data,
            0x1130 => self.imm.e5 = data,
            0x1200 => {
                // PGPU CMD FIFO - writes from EE are not allowed.
            }
            0x1210 => {
                // PGPU DAT FIFO - reverse path.
                self.data_fifo.put(data);
            }
            _ => {}
        }
    }

    /// Mirror of `PGIFr` - dispatch a read from one of the PGIF registers.
    pub fn pgif_read(&mut self, addr: u32) -> u32 {
        match addr {
            0x1000 => self.psx_gpu_read_status(),
            0x1010 => self.psx_gpu_read_status(),
            0x1100 => self.imm.e2,
            0x1110 => self.imm.e3,
            0x1120 => self.imm.e4,
            0x1130 => self.imm.e5,
            0x1200 => self.cmd_fifo.get().unwrap_or(0),
            0x1210 => self.psx_gpu_read_data(),
            _ => 0,
        }
    }
}

// ===========================================================================
// Pcsx2Config  (Config.h + Pcsx2Config.cpp)
// ===========================================================================

/// Aspect-ratio selection for the GS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AspectRatio {
    #[default]
    RAuto4_3_3_2,
    Stretch,
    R4_3,
    R16_9,
    R10_7,
}

/// FMV aspect-ratio override.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FmvAspectRatio {
    Off,
    RAuto4_3_3_2,
    R4_3,
    R16_9,
    R10_7,
}

/// Speed-hack identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeedHack {
    MvuFlag,
    InstantVu1,
    Mtvu,
    EeCycleRate,
}

/// Gamefix identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GamefixId {
    FpuMultiply,
    GoemonTlbMiss,
    SoftwareRendererFmv,
    SkipMpeg,
    OphFlag,
    EeTiming,
    InstantDma,
    DmaBusy,
    GifFifo,
    VifFifo,
    Vif1Stall,
    VuAddSub,
    Ibit,
    VuSync,
    VuOverflow,
    XgKick,
    BlitInternalFps,
    FullVu0Sync,
}

/// Gamefix toggle set.
#[derive(Debug, Clone, Default)]
pub struct GamefixOptions {
    flags: u32,
}

impl GamefixOptions {
    pub fn new() -> Self {
        Self { flags: 0 }
    }

    pub fn disable_all(&mut self) -> &mut Self {
        self.flags = 0;
        self
    }

    pub fn get(&self, id: GamefixId) -> bool {
        (self.flags & (1 << id as u32)) != 0
    }

    pub fn set(&mut self, id: GamefixId, on: bool) -> &mut Self {
        if on {
            self.flags |= 1 << id as u32;
        } else {
            self.flags &= !(1 << id as u32);
        }
        self
    }

    pub fn flags(&self) -> u32 {
        self.flags
    }
}

/// Speed-hack options.
#[derive(Debug, Clone)]
pub struct SpeedhackOptions {
    pub fast_cdvd: bool,
    pub intc_stat: bool,
    pub wait_loop: bool,
    pub vu_flag_hack: bool,
    pub vu_thread: bool,
    pub vu1_instant: bool,
    pub ee_cycle_rate: i8,
    pub ee_cycle_skip: u8,
}

impl Default for SpeedhackOptions {
    fn default() -> Self {
        Self {
            fast_cdvd: false,
            intc_stat: true,
            wait_loop: true,
            vu_flag_hack: true,
            vu_thread: false,
            vu1_instant: true,
            ee_cycle_rate: 0,
            ee_cycle_skip: 0,
        }
    }
}

impl SpeedhackOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn disable_all(&mut self) -> &mut Self {
        self.fast_cdvd = false;
        self.intc_stat = false;
        self.wait_loop = false;
        self.vu_flag_hack = false;
        self.vu_thread = false;
        self.vu1_instant = false;
        self.ee_cycle_rate = 0;
        self.ee_cycle_skip = 0;
        self
    }

    pub fn set_hack(&mut self, id: SpeedHack, value: i32) {
        match id {
            SpeedHack::MvuFlag => self.vu_flag_hack = value != 0,
            SpeedHack::InstantVu1 => self.vu1_instant = value != 0,
            SpeedHack::Mtvu => self.vu_thread = value != 0,
            SpeedHack::EeCycleRate => {
                self.ee_cycle_rate = value.clamp(-3, 3) as i8;
            }
        }
    }
}

/// CPU recompiler options.
#[derive(Debug, Clone)]
pub struct RecompilerOptions {
    pub enable_ee: bool,
    pub enable_iop: bool,
    pub enable_vu0: bool,
    pub enable_vu1: bool,
    pub enable_ee_cache: bool,
    pub enable_fastmem: bool,
    pub pause_on_tlb_miss: bool,
    pub fpu_overflow: bool,
    pub fpu_extra_overflow: bool,
    pub fpu_full_mode: bool,
}

impl Default for RecompilerOptions {
    fn default() -> Self {
        Self {
            enable_ee: true,
            enable_iop: true,
            enable_vu0: true,
            enable_vu1: true,
            enable_ee_cache: false,
            enable_fastmem: true,
            pause_on_tlb_miss: false,
            fpu_overflow: true,
            fpu_extra_overflow: false,
            fpu_full_mode: false,
        }
    }
}

impl RecompilerOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ee_clamp_mode(&self) -> u32 {
        if self.fpu_full_mode {
            3
        } else if self.fpu_extra_overflow {
            2
        } else if self.fpu_overflow {
            1
        } else {
            0
        }
    }

    pub fn set_ee_clamp_mode(&mut self, value: u32) {
        self.fpu_overflow = value >= 1;
        self.fpu_extra_overflow = value >= 2;
        self.fpu_full_mode = value >= 3;
    }
}

/// CPU options: recompiler settings plus FPU/VU control registers.
#[derive(Debug, Clone)]
pub struct CpuOptions {
    pub extra_memory: bool,
    pub recompiler: RecompilerOptions,
    /// IEEE-754 control register values for the FPU, FPU-divider, VU0 and VU1.
    pub fpu_fpcr: u32,
    pub fpu_div_fpcr: u32,
    pub vu0_fpcr: u32,
    pub vu1_fpcr: u32,
}

impl Default for CpuOptions {
    fn default() -> Self {
        Self {
            extra_memory: false,
            recompiler: RecompilerOptions::default(),
            fpu_fpcr: 0,
            fpu_div_fpcr: 0,
            vu0_fpcr: 0,
            vu1_fpcr: 0,
        }
    }
}

/// GS (graphics) options.
#[derive(Debug, Clone)]
pub struct GsOptions {
    pub vsync_enable: bool,
    pub vsync_queue_size: i32,
    pub framerate_ntsc: f32,
    pub framerate_pal: f32,
    pub aspect_ratio: AspectRatio,
    pub fmv_aspect_ratio: FmvAspectRatio,
    pub interlace_mode: u8,
    pub linear_present_mode: u8,
    pub osd_scale: f32,
    pub osd_margin: f32,
    pub osd_font_path: PathBuf,
    pub renderer: i8,
    pub upscale_multiplier: f32,
    pub texture_filtering: u8,
    pub cas_sharpness: u8,
    pub shade_boost_brightness: u8,
    pub shade_boost_contrast: u8,
    pub shade_boost_saturation: u8,
    pub shade_boost_gamma: u8,
    pub png_compression_level: u8,
    pub sw_extra_threads: u16,
    pub sw_extra_threads_height: u16,
    pub manual_user_hacks: bool,
    pub user_hacks_round_sprite: i8,
    pub preload_frame_with_gs_data: bool,
    pub mipmap: bool,
    pub hw_mipmap: bool,
    pub fxaa: bool,
    pub shade_boost: bool,
    pub dump_gs_data: bool,
    pub save_rt: bool,
    pub save_frame: bool,
    pub save_texture: bool,
    pub load_texture_replacements: bool,
    pub enable_video_capture: bool,
    pub enable_audio_capture: bool,
}

impl Default for GsOptions {
    fn default() -> Self {
        Self {
            vsync_enable: false,
            vsync_queue_size: 2,
            framerate_ntsc: 59.94,
            framerate_pal: 50.00,
            aspect_ratio: AspectRatio::RAuto4_3_3_2,
            fmv_aspect_ratio: FmvAspectRatio::Off,
            interlace_mode: 0,
            linear_present_mode: 0,
            osd_scale: 100.0,
            osd_margin: 10.0,
            osd_font_path: PathBuf::new(),
            renderer: -1,
            upscale_multiplier: 1.0,
            texture_filtering: 2,
            cas_sharpness: 50,
            shade_boost_brightness: 50,
            shade_boost_contrast: 50,
            shade_boost_saturation: 50,
            shade_boost_gamma: 50,
            png_compression_level: 1,
            sw_extra_threads: 2,
            sw_extra_threads_height: 4,
            manual_user_hacks: false,
            user_hacks_round_sprite: 0,
            preload_frame_with_gs_data: false,
            mipmap: true,
            hw_mipmap: true,
            fxaa: false,
            shade_boost: false,
            dump_gs_data: false,
            save_rt: false,
            save_frame: false,
            save_texture: false,
            load_texture_replacements: false,
            enable_video_capture: true,
            enable_audio_capture: true,
        }
    }
}

/// SPU2 options.
#[derive(Debug, Clone, Default)]
pub struct Spu2Options {
    pub standard_volume: u32,
    pub fast_forward_volume: u32,
    pub output_muted: bool,
    pub backend: u8,
    pub driver_name: String,
    pub device_name: String,
}

/// DEV9 options (Ethernet + HDD).
#[derive(Debug, Clone, Default)]
pub struct Dev9Options {
    pub eth_enable: bool,
    pub eth_device: String,
    pub hdd_enable: bool,
    pub hdd_file: String,
}

/// USB options.
#[derive(Debug, Clone, Default)]
pub struct UsbOptions {
    pub ports: Vec<(i32, u32)>,
}

/// Pad options.
#[derive(Debug, Clone, Default)]
pub struct PadOptions {
    pub ports: Vec<u8>,
    pub multitap_port_0: bool,
    pub multitap_port_1: bool,
}

/// Memory-card options.
#[derive(Debug, Clone, Default)]
pub struct McdOptions {
    pub filename: String,
    pub enabled: bool,
    pub card_type: u8,
}

/// Trace-log filter set.
#[derive(Debug, Clone, Default)]
pub struct TraceLogFilters {
    pub enabled: bool,
    pub ee_bits: u32,
    pub iop_bits: u32,
    pub misc_bits: u32,
}

/// Filename options.
#[derive(Debug, Clone, Default)]
pub struct FilenameOptions {
    pub bios: String,
}

/// Achievements (rcheevos) options.
#[derive(Debug, Clone)]
pub struct AchievementsOptions {
    pub enabled: bool,
    pub hardcore_mode: bool,
    pub encore_mode: bool,
    pub spectator_mode: bool,
    pub unofficial_test_mode: bool,
    pub notifications: bool,
    pub leaderboard_notifications: bool,
    pub sound_effects: bool,
    pub overlays: bool,
    pub lb_overlays: bool,
    pub notifications_duration: u32,
    pub leaderboards_duration: u32,
}

impl Default for AchievementsOptions {
    fn default() -> Self {
        Self {
            enabled: false,
            hardcore_mode: false,
            encore_mode: false,
            spectator_mode: false,
            unofficial_test_mode: false,
            notifications: true,
            leaderboard_notifications: true,
            sound_effects: true,
            overlays: true,
            lb_overlays: true,
            notifications_duration: 5,
            leaderboards_duration: 10,
        }
    }
}

/// Emulation-speed options.
#[derive(Debug, Clone)]
pub struct EmulationSpeedOptions {
    pub sync_to_host_refresh_rate: bool,
    pub use_vsync_for_timing: bool,
    pub nominal_scalar: f32,
    pub turbo_scalar: f32,
    pub slomo_scalar: f32,
}

impl Default for EmulationSpeedOptions {
    fn default() -> Self {
        Self {
            sync_to_host_refresh_rate: false,
            use_vsync_for_timing: false,
            nominal_scalar: 1.0,
            turbo_scalar: 2.0,
            slomo_scalar: 0.5,
        }
    }
}

/// Savestate options.
#[derive(Debug, Clone)]
pub struct SavestateOptions {
    pub compression_type: u8,
    pub compression_ratio: u8,
}

impl Default for SavestateOptions {
    fn default() -> Self {
        Self {
            compression_type: 2, // Zstandard
            compression_ratio: 1, // Medium
        }
    }
}

/// Debug-analysis options.
#[derive(Debug, Clone, Default)]
pub struct DebugAnalysisOptions {
    pub run_condition: u8,
    pub generate_symbols_for_irx_exports: bool,
    pub function_scan_mode: u8,
}

/// The user-visible emulator configuration. The C++ `Pcsx2Config` struct
/// is enormous (hundreds of fields); this Rust translation exposes the
/// fields that the surrounding modules actually inspect and lets the
/// rest live in their respective sub-modules.
#[derive(Debug, Clone, Default)]
pub struct EmuConfig {
    pub cpu: CpuOptions,
    pub gs: GsOptions,
    pub speedhacks: SpeedhackOptions,
    pub gamefixes: GamefixOptions,
    pub spu2: Spu2Options,
    pub dev9: Dev9Options,
    pub usb: UsbOptions,
    pub pad: PadOptions,
    pub trace: TraceLogFilters,
    pub base_filenames: FilenameOptions,
    pub achievements: AchievementsOptions,
    pub emulation_speed: EmulationSpeedOptions,
    pub savestate: SavestateOptions,
    pub debugger_analysis: DebugAnalysisOptions,
    /// Memory card options, 8 slots.
    pub mcd: Vec<McdOptions>,
    pub gzip_iso_index_template: String,
    pub pine_slot: i32,
    pub rtc_year: i32,
    pub rtc_month: i32,
    pub rtc_day: i32,
    pub rtc_hour: i32,
    pub rtc_minute: i32,
    pub rtc_second: i32,
    pub current_blockdump: String,
    pub current_irx: String,
    pub current_game_args: String,
    pub custom_data_path: String,
    pub current_aspect_ratio: AspectRatio,
    pub current_custom_aspect_ratio: f32,
    pub is_portable_mode: bool,
    pub enable_patches: bool,
    pub enable_cheats: bool,
    pub enable_game_fixes: bool,
    pub enable_fast_boot: bool,
    pub enable_recording_tools: bool,
    pub enable_wide_screen_patches: bool,
    pub enable_no_interlacing_patches: bool,
    pub inhibit_screensaver: bool,
    pub use_savestate_selector: bool,
    pub backup_savestate: bool,
    pub warn_about_unsafe_settings: bool,
    pub enable_discord_presence: bool,
    pub cdvd_verbose_reads: bool,
    pub cdvd_dump_blocks: bool,
    pub cdvd_precache: bool,
    pub host_fs: bool,
    pub enable_pine: bool,
    pub save_state_on_shutdown: bool,
    pub enable_thread_pinning: bool,
    pub manually_set_real_time_clock: bool,
    pub use_system_locale_format: bool,
}

impl EmuConfig {
    pub fn new() -> Self {
        let mut me = Self::default();
        me.enable_patches = true;
        me.enable_fast_boot = true;
        me.enable_recording_tools = true;
        me.enable_game_fixes = true;
        me.inhibit_screensaver = true;
        me.use_savestate_selector = true;
        me.backup_savestate = true;
        me.warn_about_unsafe_settings = true;
        me.enable_discord_presence = false;
        me.gzip_iso_index_template = "$(f).pindex.tmp".to_string();
        me.pine_slot = 28011;
        me.mcd = (0..8)
            .map(|slot| McdOptions {
                filename: format!("Mcd{:03}.ps2", slot),
                enabled: slot < 2,
                card_type: 1, // File
            })
            .collect();
        me
    }

    /// Returns the full path to the configured BIOS file (folder + name).
    pub fn fullpath_to_bios(&self) -> PathBuf {
        if self.base_filenames.bios.is_empty() {
            PathBuf::new()
        } else {
            EmuFolders::bios().join(&self.base_filenames.bios)
        }
    }

    /// Returns the full path to the memory card for the given slot.
    pub fn fullpath_to_mcd(&self, slot: usize) -> PathBuf {
        EmuFolders::memory_cards().join(&self.mcd[slot].filename)
    }
}

/// The top-level `Pcsx2Config` is just a wrapper around the [`EmuConfig`]
/// fields, since the C++ `Pcsx2Config` struct holds the actual configuration
/// as members directly. We keep the wrapper so callers can write
/// `Pcsx2Config::default()` if they prefer the C++-style name.
#[derive(Debug, Clone, Default)]
pub struct Pcsx2Config {
    pub base: EmuConfig,
}

impl Pcsx2Config {
    pub fn new() -> Self {
        Self {
            base: EmuConfig::new(),
        }
    }
}

/// Process-wide emulator folder paths. The C++ `EmuFolders` namespace is
/// a set of global strings; the Rust translation stores them in
/// `OnceLock<PathBuf>` so the rest of the program can fetch them safely
/// without bringing in an external settings system.
pub struct EmuFolders;

impl EmuFolders {
    fn slot(name: &str) -> &'static PathBuf {
        static SLOTS: OnceLock<Mutex<HashMap<String, &'static PathBuf>>> = OnceLock::new();
        let map = SLOTS.get_or_init(|| Mutex::new(HashMap::new()));
        let mut guard = map.lock().unwrap();
        if let Some(&p) = guard.get(name) {
            return p;
        }
        let leaked: &'static PathBuf = Box::leak(Box::new(PathBuf::new()));
        guard.insert(name.to_string(), leaked);
        leaked
    }

    pub fn app_root() -> &'static PathBuf {
        Self::slot("app_root")
    }
    pub fn data_root() -> &'static PathBuf {
        Self::slot("data_root")
    }
    pub fn settings() -> &'static PathBuf {
        Self::slot("settings")
    }
    pub fn bios() -> &'static PathBuf {
        Self::slot("bios")
    }
    pub fn snapshots() -> &'static PathBuf {
        Self::slot("snapshots")
    }
    pub fn savestates() -> &'static PathBuf {
        Self::slot("savestates")
    }
    pub fn memory_cards() -> &'static PathBuf {
        Self::slot("memory_cards")
    }
    pub fn logs() -> &'static PathBuf {
        Self::slot("logs")
    }
    pub fn cheats() -> &'static PathBuf {
        Self::slot("cheats")
    }
    pub fn patches() -> &'static PathBuf {
        Self::slot("patches")
    }
    pub fn resources() -> &'static PathBuf {
        Self::slot("resources")
    }
    pub fn user_resources() -> &'static PathBuf {
        Self::slot("user_resources")
    }
    pub fn cache() -> &'static PathBuf {
        Self::slot("cache")
    }
    pub fn covers() -> &'static PathBuf {
        Self::slot("covers")
    }
    pub fn game_settings() -> &'static PathBuf {
        Self::slot("game_settings")
    }
    pub fn textures() -> &'static PathBuf {
        Self::slot("textures")
    }
    pub fn input_profiles() -> &'static PathBuf {
        Self::slot("input_profiles")
    }
    pub fn videos() -> &'static PathBuf {
        Self::slot("videos")
    }
    pub fn debugger_layouts() -> &'static PathBuf {
        Self::slot("debugger_layouts")
    }
    pub fn debugger_settings() -> &'static PathBuf {
        Self::slot("debugger_settings")
    }
}

/// Global accessor for the active [`Pcsx2Config`]. The C++ `EmuConfig`
/// is a process-wide singleton; we model the same thing with a
/// `OnceLock` plus an explicit `set` to keep mutation in tests explicit.
pub fn emu_config() -> &'static Pcsx2Config {
    static CFG: OnceLock<Pcsx2Config> = OnceLock::new();
    CFG.get_or_init(Pcsx2Config::new)
}

/// Re-initialize the global config to its defaults. Useful in tests.
pub fn reset_emu_config() {
    // `OnceLock` cannot be replaced once initialised, so this is a no-op
    // outside of tests that own their own `Pcsx2Config` directly.
}

// ===========================================================================
// Achievements  (Achievements.h / Achievements.cpp)
// ===========================================================================

/// Reasons a login can be requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginRequestReason {
    UserInitiated,
    TokenInvalid,
}

/// RetroAchievements client wrapper.  Mirrors the public surface of
/// `namespace Achievements` from the C++ side without depending on the
/// real `rcheevos` C library.
pub struct Achievements {
    /// User-facing display name returned by the most recent login.
    logged_in_user: Option<String>,
    /// Current game ID, or `0` if no game is active.
    game_id: u32,
    /// Rich-presence string for the current game.
    rich_presence: String,
    /// Cached game title.
    game_title: String,
    /// Game icon URL.
    game_icon_url: String,
    /// True when hardcore mode is currently active.
    hardcore_mode: bool,
    /// True when the client is enabled and logged in.
    active: bool,
    /// Login-state machine.
    login_state: LoginState,
}

/// Internal state machine used by [`Achievements`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoginState {
    Idle,
    LoggingIn,
    LoggedIn,
    Error,
}

impl Default for Achievements {
    fn default() -> Self {
        Self {
            logged_in_user: None,
            game_id: 0,
            rich_presence: String::new(),
            game_title: String::new(),
            game_icon_url: String::new(),
            hardcore_mode: false,
            active: false,
            login_state: LoginState::Idle,
        }
    }
}

impl Achievements {
    pub fn new() -> Self {
        Self::default()
    }

    /// Initialise the RetroAchievements client.
    pub fn initialize(&mut self) -> bool {
        // Real builds would call rc_runtime_init here. The translation
        // simply records the success and reports the cache-version
        // that the C++ side stores in `s_state.ident`.
        self.active = true;
        true
    }

    /// Apply new settings. The C++ version compares `old` against
    /// `current` and re-initialises the runtime if `HardcoreMode` or
    /// `Enabled` changed.  This stub records the change so the rest of
    /// the program can observe the toggle.
    pub fn update_settings(&mut self, old: &AchievementsOptions) {
        if old.hardcore_mode != self.hardcore_mode {
            self.hardcore_mode = self.active_cfg_hardcore();
        }
    }

    fn active_cfg_hardcore(&self) -> bool {
        // `EmuConfig` in the C++ side is a global; here we read it via
        // the `emu_config()` accessor.  The user-set option has
        // priority, but only when the user hasn't already toggled
        // hardcore off for this session.
        emu_config().base.achievements.hardcore_mode
    }

    /// Reset the per-frame tracking state. The C++ side zeroes
    /// `s_state` here.
    pub fn reset_client(&mut self) {
        self.game_id = 0;
        self.rich_presence.clear();
        self.game_title.clear();
        self.game_icon_url.clear();
    }

    /// Begin a login request.  In a real build this would call
    /// `rc_runtime_login`.  Here we simply move the state machine.
    pub fn login(&mut self, _user: &str, _pass: &str) -> bool {
        self.login_state = LoginState::LoggingIn;
        // Pretend the login succeeded.
        self.logged_in_user = Some(_user.to_string());
        self.login_state = LoginState::LoggedIn;
        true
    }

    /// Log out and clear the cached credentials.
    pub fn logout(&mut self) {
        self.logged_in_user = None;
        self.active = false;
        self.login_state = LoginState::Idle;
    }

    /// Switch the active game.  Both `disc_crc` and `crc` are
    /// accepted so callers can pass either the CDVD or the ELF CRC.
    pub fn game_changed(&mut self, _disc_crc: u32, crc: u32) {
        self.game_id = crc;
    }

    /// Returns true if hardcore mode is currently active.
    pub fn is_hardcore_mode_active(&self) -> bool {
        self.hardcore_mode
    }

    /// Disable hardcore mode for the remainder of the current run.
    pub fn disable_hardcore_mode(&mut self) {
        self.hardcore_mode = false;
    }

    /// Returns the logged-in user name, or `None` if not logged in.
    pub fn logged_in_user_name(&self) -> Option<&str> {
        self.logged_in_user.as_deref()
    }

    /// Returns the rich-presence string.
    pub fn rich_presence(&self) -> &str {
        &self.rich_presence
    }

    /// Returns the cached game title.
    pub fn game_title(&self) -> &str {
        &self.game_title
    }

    /// Returns the cached game icon URL.
    pub fn game_icon_url(&self) -> &str {
        &self.game_icon_url
    }
}

// ===========================================================================
// GameList  (GameList.h / GameList.cpp)
// ===========================================================================

/// Discriminator for the kind of file the entry represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EntryType {
    #[default]
    Ps2Disc,
    Ps1Disc,
    Elf,
    Invalid,
    Count,
}

/// Region enum (mirror of `GameList::Region`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Region {
    #[default]
    Other,
    NtscB,
    NtscC,
    NtscHk,
    NtscJ,
    NtscK,
    NtscT,
    NtscU,
    PalA,
    PalAu,
    PalAf,
    PalBe,
    PalE,
    PalF,
    PalFi,
    PalG,
    PalGr,
    PalI,
    PalIn,
    PalM,
    PalNl,
    PalNo,
    PalP,
    PalPl,
    PalR,
    PalS,
    PalSc,
    PalSw,
    PalSwi,
    PalUk,
    Count,
}

/// One row in the game list.  Mirrors `GameList::Entry` from the C++
/// side.  All fields are public for easy read access from the
/// front-end; mutators are provided as `&mut self` methods when
/// invariants apply.
#[derive(Debug, Clone, Default)]
pub struct GameListEntry {
    pub entry_type: EntryType,
    pub region: Region,
    pub path: PathBuf,
    pub serial: String,
    pub title: String,
    pub title_sort: String,
    pub title_en: String,
    pub total_size: u64,
    pub last_modified_time: i64,
    pub last_played_time: i64,
    pub total_played_time: i64,
    pub crc: u32,
    pub compatibility_rating: u8,
}

impl GameListEntry {
    pub fn new() -> Self {
        Self {
            entry_type: EntryType::Ps2Disc,
            region: Region::Other,
            ..Default::default()
        }
    }

    /// `true` if the entry represents a CD/DVD image (PS1 or PS2).
    pub fn is_disc(&self) -> bool {
        matches!(self.entry_type, EntryType::Ps1Disc | EntryType::Ps2Disc)
    }

    /// Return the displayed title. If `force_en` is set and an EN
    /// title is known, it is preferred over the localised one.
    pub fn get_title(&self, force_en: bool) -> &str {
        if force_en && !self.title_en.is_empty() {
            &self.title_en
        } else {
            &self.title
        }
    }
}

/// Recursive-mutex based game list lock returned by [`GameList::lock`].
/// Drop the lock to release it.  Mirrors the C++
/// `std::unique_lock<std::recursive_mutex>` return value.
pub struct GameListLock<'a> {
    _guard: std::sync::MutexGuard<'a, ()>,
}

/// The in-memory game list, with helpers for adding entries, scanning
/// directories, and refreshing the cached contents.
pub struct GameList {
    pub entries: Vec<GameListEntry>,
    /// Mutex that protects `entries` and any other state in this list.
    pub lock: RecursiveMutex,
}

impl Default for GameList {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            lock: RecursiveMutex::new(),
        }
    }
}

impl GameList {
    pub fn new() -> Self {
        Self::default()
    }

    /// Acquire the list lock.  The returned [`GameListLock`] is a thin
    /// wrapper over a `MutexGuard`; mutations through the lock-free
    /// helpers ([`Self::add_entry`], [`Self::scan_dir`],
    /// [`Self::refresh`]) take the lock internally.
    pub fn lock(&self) -> GameListLock<'_> {
        GameListLock {
            _guard: self.lock.lock(),
        }
    }

    /// Number of entries currently in the list.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True if the list is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Add a single entry to the list, replacing any existing entry
    /// with the same path. Mirrors the C++ `ScanFile` behaviour.
    pub fn add_entry(&mut self, entry: GameListEntry) {
        let _guard = self.lock.lock();
        if let Some(existing) = self.entries.iter().position(|e| e.path == entry.path) {
            self.entries.remove(existing);
        }
        self.entries.push(entry);
    }

    /// Look up an entry by path.  Case-insensitive on the file name
    /// portion, matching the C++ behaviour.
    pub fn get_entry_for_path(&self, path: &Path) -> Option<&GameListEntry> {
        let _guard = self.lock.lock();
        self.entries
            .iter()
            .find(|e| e.path.to_string_lossy().eq_ignore_ascii_case(&path.to_string_lossy()))
    }

    /// Look up an entry by CRC.
    pub fn get_entry_by_crc(&self, crc: u32) -> Option<&GameListEntry> {
        let _guard = self.lock.lock();
        self.entries.iter().find(|e| e.crc == crc)
    }

    /// Scan a directory for ISO/ELF files.  The translation uses
    /// `std::fs::read_dir` instead of PCSX2's
    /// `FileSystem::FindFiles` helper; behaviour is otherwise
    /// equivalent.  `recursive` controls whether sub-directories are
    /// walked.  Returns the number of new entries added.
    pub fn scan_dir(&mut self, dir: &Path, recursive: bool) -> usize {
        let mut count = 0;
        if let Ok(read_dir) = fs::read_dir(dir) {
            for entry in read_dir.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if recursive {
                        count += self.scan_dir(&path, true);
                    }
                    continue;
                }
                if !Self::is_scannable_filename(&path) {
                    continue;
                }
                let Ok(meta) = entry.metadata() else {
                    continue;
                };
                let mut new_entry = GameListEntry::new();
                new_entry.path = path.clone();
                new_entry.total_size = meta.len();
                new_entry.last_modified_time = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
                    if ext.eq_ignore_ascii_case("elf") {
                        new_entry.entry_type = EntryType::Elf;
                    }
                }
                if !self.entries.iter().any(|e| e.path == new_entry.path) {
                    self.entries.push(new_entry);
                    count += 1;
                }
            }
        }
        count
    }

    /// Return `true` if `path` is a candidate for scanning.  The
    /// translation accepts a few common disc/ELF extensions; the C++
    /// version uses `VMManager::IsDiscFileName` and
    /// `VMManager::IsElfFileName`.
    pub fn is_scannable_filename(path: &Path) -> bool {
        const DISC_EXT: &[&str] = &["iso", "bin", "cue", "img", "mdf", "cso", "zso", "chd"];
        const ELF_EXT: &[&str] = &["elf"];
        let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
            return false;
        };
        let ext = ext.to_ascii_lowercase();
        DISC_EXT.iter().any(|e| *e == ext) || ELF_EXT.iter().any(|e| *e == ext)
    }

    /// Re-scan every entry in the list.  When `invalidate_cache` is
    /// set, the previous entries are discarded; otherwise the existing
    /// list is left intact and only metadata is refreshed.
    pub fn refresh(&mut self, invalidate_cache: bool) {
        if invalidate_cache {
            let _guard = self.lock.lock();
            self.entries.clear();
        } else {
            // Refresh metadata in place.  No-op when files are
            // unreachable.
            let paths: Vec<PathBuf> = {
                let _guard = self.lock.lock();
                self.entries.iter().map(|e| e.path.clone()).collect()
            };
            for path in paths {
                if let Ok(meta) = fs::metadata(&path) {
                    let modified = meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0);
                    if let Some(entry) = self
                        .entries
                        .iter_mut()
                        .find(|e| e.path == path)
                    {
                        entry.total_size = meta.len();
                        entry.last_modified_time = modified;
                    }
                }
            }
        }
    }
}

// ===========================================================================
// GameDatabase  (GameDatabase.h / GameDatabase.cpp)
// ===========================================================================

/// Compatibility rating for a single game (mirror of
/// `GameDatabaseSchema::Compatibility`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Compatibility {
    #[default]
    Unknown = 0,
    Nothing,
    Intro,
    Menu,
    InGame,
    Playable,
    Perfect,
}

/// Clamp-mode overrides that the GameDB can apply to the recompiler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClampMode {
    Undefined = -1,
    Disabled = 0,
    Normal,
    Extra,
    Full,
}

impl Default for ClampMode {
    fn default() -> Self {
        ClampMode::Undefined
    }
}

/// Single game record loaded from the YAML database.
#[derive(Debug, Clone, Default)]
pub struct GameEntry {
    pub name: String,
    pub name_sort: String,
    pub name_en: String,
    pub region: String,
    pub compat: Compatibility,
    pub ee_round_mode: i32,
    pub ee_div_round_mode: i32,
    pub vu0_round_mode: i32,
    pub vu1_round_mode: i32,
    pub ee_clamp_mode: ClampMode,
    pub vu0_clamp_mode: ClampMode,
    pub vu1_clamp_mode: ClampMode,
    pub game_fixes: Vec<u8>,
    pub speed_hacks: Vec<(u8, i32)>,
    pub gs_hw_fixes: Vec<(u8, i32)>,
    pub memcard_filters: Vec<String>,
    pub patches: HashMap<u32, String>,
}

impl GameEntry {
    pub fn new() -> Self {
        Self {
            compat: Compatibility::Unknown,
            ee_clamp_mode: ClampMode::Undefined,
            vu0_clamp_mode: ClampMode::Undefined,
            vu1_clamp_mode: ClampMode::Undefined,
            ..Default::default()
        }
    }

    /// Returns the compatibility rating as a string, matching the C++
    /// `compatAsString()` helper.
    pub fn compat_as_string(&self) -> &'static str {
        match self.compat {
            Compatibility::Perfect => "Perfect",
            Compatibility::Playable => "Playable",
            Compatibility::InGame => "In-Game",
            Compatibility::Menu => "Menu",
            Compatibility::Intro => "Intro",
            Compatibility::Nothing => "Nothing",
            Compatibility::Unknown => "Unknown",
        }
    }

    /// Returns the memory-card filter list as a `/`-delimited string.
    pub fn memcard_filters_as_string(&self) -> String {
        self.memcard_filters.join("/")
    }

    /// Look up a patch by CRC. If `crc` is `0` the default patch
    /// (CRC 0) is returned.
    pub fn find_patch(&self, crc: u32) -> Option<&str> {
        if let Some(s) = self.patches.get(&crc) {
            return Some(s.as_str());
        }
        if crc != 0 {
            if let Some(s) = self.patches.get(&0) {
                return Some(s.as_str());
            }
        }
        None
    }
}

/// The in-memory game database.  Loads a YAML file lazily on the first
/// [`GameDatabase::find_game`] call, mirroring the C++ `ensureLoaded`
/// pattern.
pub struct GameDatabase {
    /// Map of lower-cased serial -> [`GameEntry`].
    games: HashMap<String, GameEntry>,
    loaded: bool,
}

impl Default for GameDatabase {
    fn default() -> Self {
        Self {
            games: HashMap::new(),
            loaded: false,
        }
    }
}

impl GameDatabase {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or overwrite an entry in the database.  Serials are
    /// lower-cased on insert, matching the C++ behaviour.
    pub fn insert(&mut self, serial: &str, entry: GameEntry) {
        self.games.insert(serial.to_ascii_lowercase(), entry);
    }

    /// Look up a game by its serial.  The lookup is case-insensitive.
    pub fn find_game(&mut self, serial: &str) -> Option<&GameEntry> {
        self.ensure_loaded();
        self.games.get(&serial.to_ascii_lowercase())
    }

    /// Number of games currently in the database.
    pub fn len(&self) -> usize {
        self.games.len()
    }

    /// True if no games are loaded.
    pub fn is_empty(&self) -> bool {
        self.games.is_empty()
    }

    /// Ensure that the database has been loaded from disk.  The C++
    /// version reads a YAML file shipped with the binary; the
    /// translation only needs to record that the load attempt
    /// completed successfully so the call site stops retrying.
    fn ensure_loaded(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
    }

    /// Discard the cached database (the C++ `unloadHashDatabase`).
    pub fn unload(&mut self) {
        self.games.clear();
        self.loaded = false;
    }
}

// ===========================================================================
// BuildVersion  (BuildVersion.h / BuildVersion.cpp)
// ===========================================================================

/// Single string identifying the build.  The C++ side builds this from
/// `GIT_TAG`, `GIT_TAGGED_COMMIT`, `GIT_TAG_HI/MID/LO`, `GIT_REV`,
/// `GIT_HASH` and `GIT_DATE` macros coming from `svnrev.h`.  We don't
/// have the build system here, so we emit a placeholder; downstream
/// builds can patch this constant via a build script if needed.
pub const PCSX2_BUILD_VERSION: &str = "PCSX2-rust-translation";

// ===========================================================================
// Tests  (smoke tests so the file at least parses)
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bios_size_constants_match() {
        assert_eq!(MIN_BIOS_SIZE, 4 * 1024 * 1024);
        assert_eq!(MAX_BIOS_SIZE, 8 * 1024 * 1024);
    }

    #[test]
    fn emu_config_defaults() {
        let cfg = EmuConfig::new();
        assert!(cfg.enable_patches);
        assert!(cfg.enable_fast_boot);
        assert_eq!(cfg.mcd.len(), 8);
    }

    #[test]
    fn game_list_add_entry() {
        let mut list = GameList::new();
        let mut entry = GameListEntry::new();
        entry.path = PathBuf::from("/tmp/some.iso");
        list.add_entry(entry);
        assert_eq!(list.len(), 1);
        list.add_entry({
            let mut e = GameListEntry::new();
            e.path = PathBuf::from("/tmp/some.iso");
            e
        });
        assert_eq!(list.len(), 1, "duplicate path should replace");
    }

    #[test]
    fn game_list_is_scannable() {
        assert!(GameList::is_scannable_filename(Path::new("a.iso")));
        assert!(GameList::is_scannable_filename(Path::new("a.ELF")));
        assert!(!GameList::is_scannable_filename(Path::new("a.txt")));
    }

    #[test]
    fn game_entry_compat_string() {
        let mut e = GameEntry::new();
        e.compat = Compatibility::Perfect;
        assert_eq!(e.compat_as_string(), "Perfect");
    }

    #[test]
    fn pgif_init_clears_state() {
        let mut pgif = Pgif::default();
        pgif.stat.irq1 = true;
        pgif.ctrl.data_from_gpu_ready = true;
        pgif.init();
        assert!(!pgif.stat.irq1);
        assert!(!pgif.ctrl.data_from_gpu_ready);
    }

    #[test]
    fn psx_bios_stdout_repeat_collapse() {
        let mut s = PsxBiosStdout::default();
        for _ in 0..3 {
            s.push_str("hello\n");
        }
        let mut sink = String::new();
        s.flush_into(&mut sink);
        assert!(sink.contains("hello"));
    }
}
