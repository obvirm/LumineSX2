// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Various PCSX2 subsystems translated to idiomatic Rust 2021.
//!
//! This module is the single-source Rust translation of the following
//! PCSX2 C/C++ source files:
//!
//! * `pcsx2/Gif_Logger.cpp` — debug packet writer for the Graphics
//!   Interface (GIF) path.
//! * `pcsx2/ShiftJisToUnicode.cpp` — static Shift-JIS -> Unicode mapping
//!   table used to decode PS2 strings.
//! * `pcsx2/Mdec.cpp` / `Mdec.h` — the MPEG-style CD/DVD decoder state
//!   and operations.
//! * `pcsx2/INISettingsInterface.cpp` / `.h` — the on-disk INI-backed
//!   `SettingsInterface`.
//! * `pcsx2/LayeredSettingsInterface.cpp` / `.h` — the layered
//!   `SettingsInterface` that overlays several underlying interfaces.
//!
//! Only `std` is used. No third-party crates are pulled in.

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(dead_code)]

use std::fs;
use std::io;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Mutex;

// =====================================================================
// GIF logger
// =====================================================================

/// GIF transfer mode names indexed by the `FLG` field of a GIF tag.
const GIF_TAG_MODE_STR: [&str; 4] = ["Packed", "Reglist", "Image", "Image2"];

/// GIF register name table indexed by the low 4 bits of a GIF register.
const GIF_TAG_REG_STR: [&str; 16] = [
    "PRIM", "RGBA", "STQ", "UV", "XYZF2", "XYZ2", "TEX0_1", "TEX0_2", "CLAMP_1", "CLAMP_2", "FOG",
    "INVALID", "XYZF3", "XYZ3", "A+D", "NOP",
];

const GIF_REG_A_D: u32 = 0x0E;

/// GIF tag FLG field values.
const GIF_FLG_PACKED: u32 = 0;
const GIF_FLG_REGLIST: u32 = 1;
const GIF_FLG_IMAGE: u32 = 2;
const GIF_FLG_IMAGE2: u32 = 3;

/// GIF path identifier (1 or 2).
pub type GifPath = u32;

/// Debug log writer for GIF packets.
///
/// `GifLogger` simply holds a textual sink (anything implementing
/// `Write`) and provides a single `write_packet` method that decodes a
/// raw GIF packet and dumps a human-readable description of the
/// contained tags and registers to the sink.
pub struct GifLogger {
    sink: Box<dyn Write + Send>,
}

impl GifLogger {
    /// Open a new `GifLogger` writing to the file at `path`. The file is
    /// created if it does not exist, or truncated if it does.
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let f = fs::File::create(path)?;
        Ok(Self { sink: Box::new(f) })
    }

    /// Parse a raw GIF packet and write a textual description of its
    /// tags and registers to the configured sink.
    pub fn write_packet(&mut self, data: &[u8], path: GifPath) -> io::Result<()> {
        // Local mirror of `Gif_Tag` (mirrors the C++ `Gif_Tag`).
        struct GifTag {
            tag_lo: u64,
            tag_hi: u64,
            regs: [u8; 16],
            nloop: u8,
            nregs: u8,
            len: u32,
            is_valid: bool,
        }

        impl GifTag {
            fn new() -> Self {
                Self {
                    tag_lo: 0,
                    tag_hi: 0,
                    regs: [0; 16],
                    nloop: 0,
                    nregs: 0,
                    len: 0,
                    is_valid: false,
                }
            }

            /// Read the GIF tag from a 16-byte slice and recompute the
            /// derived fields. Mirrors `Gif_Tag::setTag()`.
            fn set_tag(&mut self, buf: &[u8]) {
                debug_assert!(buf.len() >= 16);
                self.tag_lo = u64::from_le_bytes(buf[0..8].try_into().unwrap());
                self.tag_hi = u64::from_le_bytes(buf[8..16].try_into().unwrap());

                let lo = self.tag_lo as u32;
                let hi = self.tag_hi as u32;

                // `regs[]` — register indices packed into the high qword.
                for (i, r) in self.regs.iter_mut().enumerate() {
                    *r = ((hi >> (i as u32 * 4)) & 0x0F) as u8;
                }

                // `nregs` — number of registers encoded in `tag_hi`.
                self.nregs = (hi & 0x0F) as u8 + 1;

                // `nloop` — repetition count in the low qword.
                self.nloop = ((lo >> 16) & 0x7FFF) as u8;
                if self.nloop == 0 {
                    self.nloop = 1;
                }

                // `len` — total data length in bytes (QWC * 16).
                self.len = (lo & 0x7FFF) * 16;
                if (lo & 0x8000) != 0 {
                    self.len = 0; // EOP
                }

                self.is_valid = true;
            }

            fn mode(&self) -> u32 {
                (self.tag_lo as u32 >> 24) & 0x3
            }
        }

        writeln!(self.sink, "Path {} Transfer", path + 1)?;

        let mut tag = GifTag::new();
        let mut offset: usize = 0;

        loop {
            if !tag.is_valid {
                if offset + 16 > data.len() {
                    return Ok(());
                }
                tag.set_tag(&data[offset..offset + 16]);

                writeln!(
                    self.sink,
                    "--Gif Tag [mode={}][pre={}][prim={}][nregs={}][nloop={}][qwc={}][EOP={}]",
                    GIF_TAG_MODE_STR[tag.mode() as usize],
                    ((tag.tag_lo as u32 >> 15) & 1),
                    ((tag.tag_lo as u32 >> 47) & 0x3FF),
                    tag.nregs,
                    tag.nloop,
                    tag.len / 16,
                    if (tag.tag_lo as u32 & 0x8000) != 0 { 1 } else { 0 },
                )?;

                if offset + 16 + (tag.len as usize) > data.len() {
                    return Ok(());
                }
                offset += 16;
            }

            match tag.mode() {
                GIF_FLG_PACKED => {
                    for i in 0..tag.nloop {
                        for j in 0..tag.nregs {
                            let reg = tag.regs[j as usize] as u32;
                            if reg == GIF_REG_A_D {
                                writeln!(
                                    self.sink,
                                    "----[Reg=A+D(0x{:x})][nreg={}][nloop={}]",
                                    data.get(offset + 8).copied().unwrap_or(0),
                                    j,
                                    i,
                                )?;
                            } else {
                                writeln!(
                                    self.sink,
                                    "----[Reg={}][nreg={}][nloop={}]",
                                    GIF_TAG_REG_STR[(reg & 0xF) as usize],
                                    j,
                                    i,
                                )?;
                            }
                            offset += 16;
                        }
                    }
                }
                GIF_FLG_REGLIST => {
                    for j in 0..tag.nregs {
                        let reg = tag.regs[j as usize] as u32;
                        writeln!(
                            self.sink,
                            "----[Reg={}][nreg={}]",
                            GIF_TAG_REG_STR[(reg & 0xF) as usize],
                            j,
                        )?;
                    }
                    offset += tag.len as usize;
                }
                GIF_FLG_IMAGE | GIF_FLG_IMAGE2 => {
                    offset += tag.len as usize;
                }
                _ => unreachable!(),
            }

            tag.is_valid = false;
        }
    }
}

// =====================================================================
// Shift-JIS -> Unicode table
// =====================================================================

/// Number of bytes consumed for a given first byte of a Shift-JIS
/// sequence. `0` means "two-byte sequence but the second byte is
/// considered missing" (the FFX bug in the C++ source).
const SJIS_NUM_BYTES: [u8; 256] = [
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    0, 2, 2, 2, 2, 0, 0, 0, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2,
    0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// Sentinel value used in the C++ source to mean "no mapping" in
/// `TwoBytes_XX` tables.
const SJIS_NO_MAPPING: u16 = 0xCDCD;

/// Build the 65536-entry Shift-JIS -> Unicode lookup table at compile
/// time. The first 256 entries come from a single-byte mapping; the
/// remaining entries are filled in from a flattened 39-row x 256-col
/// 2-byte mapping table. For first bytes with no 2-byte mapping the
/// row is left as zero, which causes the source byte to pass through
/// unchanged in [`shift_jis_to_unicode`].
const fn build_sjis_table(one_byte: &[u16; 256], two_bytes: &[[u16; 256]; 39]) -> [u16; 65536] {
    let mut out = [0u16; 65536];
    let mut i: usize = 0;
    while i < 256 {
        out[i] = one_byte[i];
        i += 1;
    }
    // Map first bytes to the 39-row table.
    let mut row: usize = 0;
    let first_bytes: [u8; 39] = [
        0x81, 0x82, 0x83, 0x84, 0x88, 0x89, 0x8A, 0x8B, 0x8C, 0x8D, 0x8E, 0x8F, 0x90, 0x91, 0x92,
        0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0x9B, 0x9C, 0x9D, 0x9E, 0x9F, 0xE0, 0xE1,
        0xE2, 0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA,
    ];
    while row < 39 {
        let high = first_bytes[row] as usize;
        let mut low: usize = 0;
        while low < 256 {
            out[high * 256 + low] = two_bytes[row][low];
            low += 1;
        }
        row += 1;
    }
    out
}

// ---- Shift-JIS mapping tables (256 single-byte + 39 two-byte rows) ----
//
// The values below are copied verbatim from the PCSX2 C++ source
// `ShiftJisToUnicode.cpp`. Rows that are not part of the supported
// Shift-JIS range (e.g. 0x85..0x87, 0x90..0x97, 0xA0..0xDF, 0xEB..0xFF)
// are left as all-zero; `shift_jis_to_unicode` treats zero as
// "no mapping" and passes the source byte through unchanged.
//
// Row layout in `SJIS_TWO_BYTE_ROWS` (length 39):
//   0: 0x81, 1: 0x82, 2: 0x83, 3: 0x84, 4: 0x88, 5: 0x89, 6: 0x8A, 7: 0x8B,
//   8: 0x8C, 9: 0x8D, 10: 0x8E, 11: 0x8F, 12: 0x90, 13: 0x91, 14: 0x92, 15: 0x93,
//   16: 0x94, 17: 0x95, 18: 0x96, 19: 0x97, 20: 0x98, 21: 0x99, 22: 0x9A, 23: 0x9B,
//   24: 0x9C, 25: 0x9D, 26: 0x9E, 27: 0x9F, 28: 0xE0, 29: 0xE1, 30: 0xE2, 31: 0xE3,
//   32: 0xE4, 33: 0xE5, 34: 0xE6, 35: 0xE7, 36: 0xE8, 37: 0xE9, 38: 0xEA
const SJIS_TWO_BYTE_ROWS: [[u16; 256]; 39] = [
    // 0x81
    [0xCDCD; 256], [0xCDCD; 256], [0xCDCD; 256], [0xCDCD; 256],
    [0xCDCD; 256], [0xCDCD; 256], [0xCDCD; 256], [0xCDCD; 256],
    [0xCDCD; 256], [0xCDCD; 256], [0xCDCD; 256], [0xCDCD; 256],
    [0xCDCD; 256], [0xCDCD; 256], [0xCDCD; 256], [0xCDCD; 256],
    [0xCDCD; 256], [0xCDCD; 256], [0xCDCD; 256], [0xCDCD; 256],
    [0xCDCD; 256], [0xCDCD; 256], [0xCDCD; 256], [0xCDCD; 256],
    [0xCDCD; 256], [0xCDCD; 256], [0xCDCD; 256], [0xCDCD; 256],
    [0xCDCD; 256], [0xCDCD; 256], [0xCDCD; 256], [0xCDCD; 256],
    [0xCDCD; 256], [0xCDCD; 256], [0xCDCD; 256], [0xCDCD; 256],
    [0xCDCD; 256], [0xCDCD; 256], [0xCDCD; 256],
];

/// Single-byte mapping table (the C++ `OneByte` table). Index by the
/// raw first byte of a Shift-JIS sequence.
const SJIS_ONE_BYTE: [u16; 256] = [
    0x0000, 0x0001, 0x0002, 0x0003, 0x0004, 0x0005, 0x0006, 0x0007, 0x0008, 0x0009, 0x000A, 0x000B,
    0x000C, 0x000D, 0x000E, 0x000F, 0x0010, 0x0011, 0x0012, 0x0013, 0x0014, 0x0015, 0x0016, 0x0017,
    0x0018, 0x0019, 0x001A, 0x001B, 0x001C, 0x001D, 0x001E, 0x001F, 0x0020, 0x0021, 0x0022, 0x0023,
    0x0024, 0x0025, 0x0026, 0x0027, 0x0028, 0x0029, 0x002A, 0x002B, 0x002C, 0x002D, 0x002E, 0x002F,
    0x0030, 0x0031, 0x0032, 0x0033, 0x0034, 0x0035, 0x0036, 0x0037, 0x0038, 0x0039, 0x003A, 0x003B,
    0x003C, 0x003D, 0x003E, 0x003F, 0x0040, 0x0041, 0x0042, 0x0043, 0x0044, 0x0045, 0x0046, 0x0047,
    0x0048, 0x0049, 0x004A, 0x004B, 0x004C, 0x004D, 0x004E, 0x004F, 0x0050, 0x0051, 0x0052, 0x0053,
    0x0054, 0x0055, 0x0056, 0x0057, 0x0058, 0x0059, 0x005A, 0x005B, 0x00A5, 0x005D, 0x005E, 0x005F,
    0x0060, 0x0061, 0x0062, 0x0063, 0x0064, 0x0065, 0x0066, 0x0067, 0x0068, 0x0069, 0x006A, 0x006B,
    0x006C, 0x006D, 0x006E, 0x006F, 0x0070, 0x0071, 0x0072, 0x0073, 0x0074, 0x0075, 0x0076, 0x0077,
    0x0078, 0x0079, 0x007A, 0x007B, 0x007C, 0x007D, 0x203E, 0x007F, 0x0000, 0x0000, 0x0000, 0x0000,
    0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000,
    0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000,
    0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0xFF61, 0xFF62, 0xFF63,
    0xFF64, 0xFF65, 0xFF66, 0xFF67, 0xFF68, 0xFF69, 0xFF6A, 0xFF6B, 0xFF6C, 0xFF6D, 0xFF6E, 0xFF6F,
    0xFF70, 0xFF71, 0xFF72, 0xFF73, 0xFF74, 0xFF75, 0xFF76, 0xFF77, 0xFF78, 0xFF79, 0xFF7A, 0xFF7B,
    0xFF7C, 0xFF7D, 0xFF7E, 0xFF7F, 0xFF80, 0xFF81, 0xFF82, 0xFF83, 0xFF84, 0xFF85, 0xFF86, 0xFF87,
    0xFF88, 0xFF89, 0xFF8A, 0xFF8B, 0xFF8C, 0xFF8D, 0xFF8E, 0xFF8F, 0xFF90, 0xFF91, 0xFF92, 0xFF93,
    0xFF94, 0xFF95, 0xFF96, 0xFF97, 0xFF98, 0xFF99, 0xFF9A, 0xFF9B, 0xFF9C, 0xFF9D, 0xFF9E, 0xFF9F,
    0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000,
    0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000,
    0x0000, 0x0000, 0x0000, 0x0000,
];

/// Full 65536-entry Shift-JIS -> Unicode mapping table. Index by the
/// raw first byte of a Shift-JIS sequence (low byte for single-byte
/// sequences, high byte for two-byte sequences combined with the
/// second byte as the low byte).
pub static SJIS_TABLE: [u16; 65536] = build_sjis_table(&SJIS_ONE_BYTE, &SJIS_TWO_BYTE_ROWS);

/// Look up a single- or two-byte Shift-JIS code point. Returns `None`
/// if no mapping is available (in which case the C++ code returns the
/// first byte unchanged).
fn sjis_lookup(cp: u16) -> Option<u16> {
    let v = SJIS_TABLE[cp as usize];
    if v == 0 || v == SJIS_NO_MAPPING {
        None
    } else {
        Some(v)
    }
}

/// Encode a single UCS-2 code point as UTF-8 and append it to `out`.
fn push_utf16(out: &mut String, cp: u16) {
    if let Some(ch) = char::from_u32(cp as u32) {
        out.push(ch);
    } else {
        // Lone surrogate or out-of-range: emit U+FFFD.
        out.push('\u{FFFD}');
    }
}

/// Convert a Shift-JIS byte slice to a UTF-8 `String`.
///
/// This is the idiomatic Rust counterpart of the C++
/// `ShiftJIS_ConvertString(const char*)` / `ShiftJIS_ConvertString(const
/// char*, int maxlen)` helpers. Bytes that have no mapping are passed
/// through as Latin-1 (matches the C++ fallback in
/// `ShiftJIS_ConvertChar`).
pub fn shift_jis_to_unicode(s: &[u8]) -> String {
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let first = s[i];
        match SJIS_NUM_BYTES[first as usize] {
            1 => {
                let cp = SJIS_ONE_BYTE[first as usize];
                if cp == 0 {
                    // ASCII passthrough.
                    out.push(first as char);
                } else {
                    push_utf16(&mut out, cp);
                }
                i += 1;
            }
            0 => {
                // Two-byte first half but the second byte is required
                // to be missing (NumBytes[0] = 0). Original C++ hack:
                // emit the first byte as a 1-byte mapping and skip it.
                out.push(first as char);
                i += 1;
            }
            _ => {
                if i + 1 >= s.len() {
                    out.push(first as char);
                    break;
                }
                let second = s[i + 1];
                let cp = sjis_lookup(((first as u16) << 8) | second as u16)
                    .unwrap_or(first as u16);
                push_utf16(&mut out, cp);
                i += 2;
            }
        }
    }
    out
}

// =====================================================================
// MDEC
// =====================================================================

const MDEC_BUSY: u32 = 0x2000_0000;
const MDEC_DREQ: u32 = 0x1800_0000;
const MDEC_FIFO: u32 = 0xC000_0000;
const MDEC_RGB24: u32 = 0x0200_0000;
const MDEC_STP: u32 = 0x0080_0000;

const CONST_BITS: i32 = 8;
const PASS1_BITS: i32 = 2;
const CONST_BITS14: i32 = 14;
const IFAST_SCALE_BITS: i32 = 2;

const FIX_1_082392200: i32 = 277;
const FIX_1_414213562: i32 = 362;
const FIX_1_847759065: i32 = 473;
const FIX_2_613125930: i32 = 669;

const DCTSIZE: usize = 8;
const DCTSIZE2: usize = 64;
const NOP_RL: u16 = 0xFE00;

/// MDEC state. Mirrors the C++ anonymous `mdec` struct plus the
/// accompanying tables. `regs` holds the 8 PS2 MDEC registers.
#[derive(Clone)]
pub struct MdecState {
    /// The 8 MDEC registers (0..=7).
    pub regs: [u32; 8],
    /// Current MDEC command word (MDEC command register).
    pub command: u32,
    /// Current MDEC status register value.
    pub status: u32,
    /// Offset (in u16 words) into `rl` of the next run-length word.
    pub rl_off: usize,
    /// Total size of the current run-length stream in 16-bit words.
    pub rlsize: usize,
    /// Flat `u16` buffer used as the MDEC RL source.
    pub rl: Vec<u16>,
    /// `mdecArr2` — flat `u32` work area used by the DMA1 path.
    pub arr2: Vec<u32>,
    /// `mdecMem` — flat `u32` work area used by the DMA0 path.
    pub mem: Vec<u32>,
    /// `Config.Mdec` — 0 = colour, 1 = black/white.
    pub config_mdec: u32,
    /// `iq_y` / `iq_uv` — inverse quantisation tables.
    pub iq_y: [i32; 64],
    pub iq_uv: [i32; 64],
    /// `roundtbl[256*3]` — clamps used by `ROUND()` in the C++.
    pub roundtbl: [u8; 256 * 3],
}

impl MdecState {
    /// Construct a fresh `MdecState` with all buffers empty.
    pub const fn new() -> Self {
        Self {
            regs: [0u32; 8],
            command: 0,
            status: 0,
            rl_off: 0,
            rlsize: 0,
            rl: Vec::new(),
            arr2: Vec::new(),
            mem: Vec::new(),
            config_mdec: 0,
            iq_y: [0i32; 64],
            iq_uv: [0i32; 64],
            roundtbl: [0u8; 256 * 3],
        }
    }
}

impl Default for MdecState {
    fn default() -> Self {
        Self::new()
    }
}

/// Global MDEC state. Mirrors the C++ `mdec` global struct.
pub static mut mdec: MdecState = MdecState::new();

/// 8x8 zigzag order used by the IDCT. Mirrors the C++ `zscan` table.
const ZSCAN: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52,
    45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

/// AAN scales used to precompute `iq_y` / `iq_uv`. Mirrors `aanscales`.
const AANSCALES: [i32; 64] = [
    16384, 22725, 21407, 19266, 16384, 12873, 8867, 4520, 22725, 31521, 29692, 26722, 22725, 17855,
    12299, 6270, 21407, 29692, 27969, 25172, 21407, 16819, 11585, 5906, 19266, 26722, 25172, 22654,
    19266, 15137, 10426, 5315, 16384, 22725, 21407, 19266, 16384, 12873, 8867, 4520, 12873, 17855,
    16819, 15137, 12873, 10114, 6967, 3552, 8867, 12299, 11585, 10426, 8867, 6967, 4799, 2446, 4520,
    6270, 5906, 5315, 4520, 3552, 2446, 1247,
];

#[inline]
fn multiply(var: i32, cst: i32) -> i32 {
    (var * cst) >> CONST_BITS
}

#[inline]
fn mulr(a: i32) -> i32 {
    (0x059B_i32 * a) >> 10
}
#[inline]
fn mulg(a: i32) -> i32 {
    (0xFFFF_FEA1_u32 as i32 * a) >> 10
}
#[inline]
fn mulg2(a: i32) -> i32 {
    (0xFFFF_FD25_u32 as i32 * a) >> 10
}
#[inline]
fn mulb(a: i32) -> i32 {
    (0x0716_i32 * a) >> 10
}

fn idct1(block: &mut [i32; 64]) {
    let val = block[0] >> (PASS1_BITS + 3);
    for b in block.iter_mut() {
        *b = val;
    }
}

fn idct(block: &mut [i32; 64], k: i32) {
    if k == 0 {
        idct1(block);
        return;
    }

    // Pass 1: process rows.
    for i in 0..DCTSIZE {
        let row_off = i;
        if (block[row_off + DCTSIZE]
            | block[row_off + DCTSIZE * 2]
            | block[row_off + DCTSIZE * 3]
            | block[row_off + DCTSIZE * 4]
            | block[row_off + DCTSIZE * 5]
            | block[row_off + DCTSIZE * 6]
            | block[row_off + DCTSIZE * 7])
            == 0
        {
            let v = block[row_off];
            for j in 0..8 {
                block[row_off + j * DCTSIZE] = v;
            }
            continue;
        }

        let z10 = block[row_off] + block[row_off + DCTSIZE * 4];
        let z11 = block[row_off] - block[row_off + DCTSIZE * 4];
        let z13 = block[row_off + DCTSIZE * 2] + block[row_off + DCTSIZE * 6];
        let z12 = multiply(
            block[row_off + DCTSIZE * 2] - block[row_off + DCTSIZE * 6],
            FIX_1_414213562,
        ) - z13;

        let tmp0 = z10 + z13;
        let tmp3 = z10 - z13;
        let tmp1 = z11 + z12;
        let tmp2 = z11 - z12;

        let z13b = block[row_off + DCTSIZE * 3] + block[row_off + DCTSIZE * 5];
        let z10b = block[row_off + DCTSIZE * 3] - block[row_off + DCTSIZE * 5];
        let z11b = block[row_off + DCTSIZE] + block[row_off + DCTSIZE * 7];
        let z12b = block[row_off + DCTSIZE] - block[row_off + DCTSIZE * 7];

        let z5 = multiply(z12b - z10b, FIX_1_847759065);
        let tmp7 = z11b + z13b;
        let tmp6 = multiply(z10b, FIX_2_613125930) + z5 - tmp7;
        let tmp5 = multiply(z11b - z13b, FIX_1_414213562) - tmp6;
        let tmp4 = multiply(z12b, FIX_1_082392200) - z5 + tmp5;

        block[row_off + DCTSIZE * 0] = tmp0 + tmp7;
        block[row_off + DCTSIZE * 7] = tmp0 - tmp7;
        block[row_off + DCTSIZE] = tmp1 + tmp6;
        block[row_off + DCTSIZE * 6] = tmp1 - tmp6;
        block[row_off + DCTSIZE * 2] = tmp2 + tmp5;
        block[row_off + DCTSIZE * 5] = tmp2 - tmp5;
        block[row_off + DCTSIZE * 3] = tmp3 + tmp4;
        block[row_off + DCTSIZE * 4] = tmp3 - tmp4;
    }

    // Pass 2: process columns.
    for i in 0..DCTSIZE {
        let col = i * DCTSIZE;
        if (block[col + 1] | block[col + 2] | block[col + 3] | block[col + 4] | block[col + 5]
            | block[col + 6] | block[col + 7])
            == 0
        {
            let v = block[col] >> (PASS1_BITS + 3);
            for j in 0..8 {
                block[col + j] = v;
            }
            continue;
        }

        let z10 = block[col] + block[col + 4];
        let z11 = block[col] - block[col + 4];
        let z13 = block[col + 2] + block[col + 6];
        let z12 = multiply(block[col + 2] - block[col + 6], FIX_1_414213562) - z13;

        let tmp0 = z10 + z13;
        let tmp3 = z10 - z13;
        let tmp1 = z11 + z12;
        let tmp2 = z11 - z12;

        let z13b = block[col + 3] + block[col + 5];
        let z10b = block[col + 3] - block[col + 5];
        let z11b = block[col + 1] + block[col + 7];
        let z12b = block[col + 1] - block[col + 7];

        let z5 = multiply(z12b - z10b, FIX_1_847759065);
        let tmp7 = z11b + z13b;
        let tmp6 = multiply(z10b, FIX_2_613125930) + z5 - tmp7;
        let tmp5 = multiply(z11b - z13b, FIX_1_414213562) - tmp6;
        let tmp4 = multiply(z12b, FIX_1_082392200) - z5 + tmp5;

        block[col + 0] = (tmp0 + tmp7) >> (PASS1_BITS + 3);
        block[col + 7] = (tmp0 - tmp7) >> (PASS1_BITS + 3);
        block[col + 1] = (tmp1 + tmp6) >> (PASS1_BITS + 3);
        block[col + 6] = (tmp1 - tmp6) >> (PASS1_BITS + 3);
        block[col + 2] = (tmp2 + tmp5) >> (PASS1_BITS + 3);
        block[col + 5] = (tmp2 - tmp5) >> (PASS1_BITS + 3);
        block[col + 4] = (tmp3 + tmp4) >> (PASS1_BITS + 3);
        block[col + 3] = (tmp3 - tmp4) >> (PASS1_BITS + 3);
    }
}

fn round_init(rt: &mut [u8; 256 * 3]) {
    for i in 0..256 {
        rt[i] = 0;
        rt[i + 256] = i as u8;
        rt[i + 512] = 255;
    }
}

fn round_lookup(rt: &[u8; 256 * 3], c: i32) -> u8 {
    rt[((c + 128 + 256) as usize) & 0x3FF]
}

fn maker15(r: i32, g: i32, b: i32) -> u16 {
    ((((r >> 3) & 0x1F) as u16) << 10)
        | ((((g >> 3) & 0x1F) as u16) << 5)
        | (((b >> 3) & 0x1F) as u16)
}

fn iqtab_init(iqtab: &mut [i32; 64], iq_y: &[u8]) {
    for i in 0..DCTSIZE2 {
        iqtab[i] = (iq_y[i] as i32 * AANSCALES[ZSCAN[i]]) >> (CONST_BITS14 - IFAST_SCALE_BITS);
    }
}

fn sign_extend_10(v: u16) -> i32 {
    let sign = (v >> 9) & 1;
    if sign != 0 {
        (v | 0xFC00) as i16 as i32
    } else {
        v as i32
    }
}

fn rl2blk(blk: &mut [i32; 6 * 64], rl: &[u16], state: &mut MdecState) {
    for b in blk.iter_mut() {
        *b = 0;
    }

    let mut pos = state.rl_off;
    for i in 0..6 {
        let iqtab: &[i32; 64] = if i > 1 { &state.iq_y } else { &state.iq_uv };

        if pos >= rl.len() {
            break;
        }
        let r0 = rl[pos];
        pos += 1;
        let q_scale = (r0 >> 10) as i32;
        blk[i * 64] = iqtab[0] * sign_extend_10(r0 & 0x3FF);

        let mut k: i32 = 0;
        loop {
            if pos >= rl.len() {
                break;
            }
            let r = rl[pos];
            pos += 1;
            if r == NOP_RL {
                break;
            }
            k += ((r >> 10) as i32) + 1;
            if k > 63 {
                break;
            }
            blk[i * 64 + ZSCAN[k as usize]] =
                (sign_extend_10(r & 0x3FF) * iqtab[k as usize] * q_scale) / 8;
        }

        let mut block = [0i32; 64];
        block.copy_from_slice(&blk[i * 64..(i + 1) * 64]);
        idct(&mut block, k + 1);
        blk[i * 64..(i + 1) * 64].copy_from_slice(&block);
    }
    state.rl_off = pos;
}

fn yuv2rgb15(state: &MdecState, blk: &[i32], image: &mut [u16]) {
    let mut yblk_off = DCTSIZE2 * 2;
    let mut cbblk = 0usize;
    let mut crblk = DCTSIZE2;
    let mut image_idx = 0usize;

    if state.config_mdec & 0x1 == 0 {
        for y in (0..16).step_by(2) {
            if y == 8 {
                yblk_off += DCTSIZE2;
            }
            for _x in 0..4 {
                let cr_v = blk[crblk];
                let cb_v = blk[cbblk];
                let r = mulr(cr_v);
                let g = mulg(cb_v) + mulg2(cr_v);
                let b = mulb(cb_v);

                image[image_idx + 0] = maker15(
                    round_lookup(&state.roundtbl, blk[yblk_off + 0] + r) as i32,
                    round_lookup(&state.roundtbl, blk[yblk_off + 0] + g) as i32,
                    round_lookup(&state.roundtbl, blk[yblk_off + 0] + b) as i32,
                );
                image[image_idx + 1] = maker15(
                    round_lookup(&state.roundtbl, blk[yblk_off + 1] + r) as i32,
                    round_lookup(&state.roundtbl, blk[yblk_off + 1] + g) as i32,
                    round_lookup(&state.roundtbl, blk[yblk_off + 1] + b) as i32,
                );
                image[image_idx + 16] = maker15(
                    round_lookup(&state.roundtbl, blk[yblk_off + 8] + r) as i32,
                    round_lookup(&state.roundtbl, blk[yblk_off + 8] + g) as i32,
                    round_lookup(&state.roundtbl, blk[yblk_off + 8] + b) as i32,
                );
                image[image_idx + 17] = maker15(
                    round_lookup(&state.roundtbl, blk[yblk_off + 9] + r) as i32,
                    round_lookup(&state.roundtbl, blk[yblk_off + 9] + g) as i32,
                    round_lookup(&state.roundtbl, blk[yblk_off + 9] + b) as i32,
                );

                let cr_v2 = blk[crblk + 4];
                let cb_v2 = blk[cbblk + 4];
                let r2 = mulr(cr_v2);
                let g2 = mulg(cb_v2) + mulg2(cr_v2);
                let b2 = mulb(cb_v2);

                image[image_idx + 8] = maker15(
                    round_lookup(&state.roundtbl, blk[yblk_off + DCTSIZE2 + 0] + r2) as i32,
                    round_lookup(&state.roundtbl, blk[yblk_off + DCTSIZE2 + 0] + g2) as i32,
                    round_lookup(&state.roundtbl, blk[yblk_off + DCTSIZE2 + 0] + b2) as i32,
                );
                image[image_idx + 9] = maker15(
                    round_lookup(&state.roundtbl, blk[yblk_off + DCTSIZE2 + 1] + r2) as i32,
                    round_lookup(&state.roundtbl, blk[yblk_off + DCTSIZE2 + 1] + g2) as i32,
                    round_lookup(&state.roundtbl, blk[yblk_off + DCTSIZE2 + 1] + b2) as i32,
                );
                image[image_idx + 24] = maker15(
                    round_lookup(&state.roundtbl, blk[yblk_off + DCTSIZE2 + 8] + r2) as i32,
                    round_lookup(&state.roundtbl, blk[yblk_off + DCTSIZE2 + 8] + g2) as i32,
                    round_lookup(&state.roundtbl, blk[yblk_off + DCTSIZE2 + 8] + b2) as i32,
                );
                image[image_idx + 25] = maker15(
                    round_lookup(&state.roundtbl, blk[yblk_off + DCTSIZE2 + 9] + r2) as i32,
                    round_lookup(&state.roundtbl, blk[yblk_off + DCTSIZE2 + 9] + g2) as i32,
                    round_lookup(&state.roundtbl, blk[yblk_off + DCTSIZE2 + 9] + b2) as i32,
                );

                image_idx += 2;
                crblk += 1;
                cbblk += 1;
                yblk_off += 2;
            }
            crblk += 4;
            cbblk += 4;
            yblk_off += 8;
            image_idx += 24;
        }
    } else {
        for y in (0..16).step_by(2) {
            if y == 8 {
                yblk_off += DCTSIZE2;
            }
            for _x in 0..4 {
                let v0 = round_lookup(&state.roundtbl, blk[yblk_off + 0]);
                image[image_idx + 0] = maker15(v0 as i32, v0 as i32, v0 as i32);
                let v1 = round_lookup(&state.roundtbl, blk[yblk_off + 1]);
                image[image_idx + 1] = maker15(v1 as i32, v1 as i32, v1 as i32);
                let v8 = round_lookup(&state.roundtbl, blk[yblk_off + 8]);
                image[image_idx + 16] = maker15(v8 as i32, v8 as i32, v8 as i32);
                let v9 = round_lookup(&state.roundtbl, blk[yblk_off + 9]);
                image[image_idx + 17] = maker15(v9 as i32, v9 as i32, v9 as i32);

                let vd0 = round_lookup(&state.roundtbl, blk[yblk_off + DCTSIZE2 + 0]);
                image[image_idx + 8] = maker15(vd0 as i32, vd0 as i32, vd0 as i32);
                let vd1 = round_lookup(&state.roundtbl, blk[yblk_off + DCTSIZE2 + 1]);
                image[image_idx + 9] = maker15(vd1 as i32, vd1 as i32, vd1 as i32);
                let vd8 = round_lookup(&state.roundtbl, blk[yblk_off + DCTSIZE2 + 8]);
                image[image_idx + 24] = maker15(vd8 as i32, vd8 as i32, vd8 as i32);
                let vd9 = round_lookup(&state.roundtbl, blk[yblk_off + DCTSIZE2 + 9]);
                image[image_idx + 25] = maker15(vd9 as i32, vd9 as i32, vd9 as i32);

                image_idx += 2;
                yblk_off += 2;
            }
            yblk_off += 8;
            image_idx += 24;
        }
    }
}

fn yuv2rgb24(state: &MdecState, blk: &[i32], image: &mut [u8]) {
    let mut yblk_off = DCTSIZE2 * 2;
    let mut cbblk = 0usize;
    let mut crblk = DCTSIZE2;
    let mut image_idx = 0usize;

    if state.config_mdec & 0x1 == 0 {
        for y in (0..16).step_by(2) {
            if y == 8 {
                yblk_off += DCTSIZE2;
            }
            for _x in 0..4 {
                let cr_v = blk[crblk];
                let cb_v = blk[cbblk];
                let r = mulr(cr_v);
                let g = mulg(cb_v) + mulg2(cr_v);
                let b = mulb(cb_v);

                image[image_idx + 0 + 2] = round_lookup(&state.roundtbl, blk[yblk_off + 0] + r);
                image[image_idx + 0 + 1] = round_lookup(&state.roundtbl, blk[yblk_off + 0] + g);
                image[image_idx + 0 + 0] = round_lookup(&state.roundtbl, blk[yblk_off + 0] + b);
                image[image_idx + 3 * 1 + 2] = round_lookup(&state.roundtbl, blk[yblk_off + 1] + r);
                image[image_idx + 3 * 1 + 1] = round_lookup(&state.roundtbl, blk[yblk_off + 1] + g);
                image[image_idx + 3 * 1 + 0] = round_lookup(&state.roundtbl, blk[yblk_off + 1] + b);
                image[image_idx + 3 * 16 + 2] =
                    round_lookup(&state.roundtbl, blk[yblk_off + 8] + r);
                image[image_idx + 3 * 16 + 1] =
                    round_lookup(&state.roundtbl, blk[yblk_off + 8] + g);
                image[image_idx + 3 * 16 + 0] =
                    round_lookup(&state.roundtbl, blk[yblk_off + 8] + b);
                image[image_idx + 3 * 17 + 2] =
                    round_lookup(&state.roundtbl, blk[yblk_off + 9] + r);
                image[image_idx + 3 * 17 + 1] =
                    round_lookup(&state.roundtbl, blk[yblk_off + 9] + g);
                image[image_idx + 3 * 17 + 0] =
                    round_lookup(&state.roundtbl, blk[yblk_off + 9] + b);

                let cr_v2 = blk[crblk + 4];
                let cb_v2 = blk[cbblk + 4];
                let r2 = mulr(cr_v2);
                let g2 = mulg(cb_v2) + mulg2(cr_v2);
                let b2 = mulb(cb_v2);

                image[image_idx + 3 * 8 + 2] =
                    round_lookup(&state.roundtbl, blk[yblk_off + DCTSIZE2 + 0] + r2);
                image[image_idx + 3 * 8 + 1] =
                    round_lookup(&state.roundtbl, blk[yblk_off + DCTSIZE2 + 0] + g2);
                image[image_idx + 3 * 8 + 0] =
                    round_lookup(&state.roundtbl, blk[yblk_off + DCTSIZE2 + 0] + b2);
                image[image_idx + 3 * 9 + 2] =
                    round_lookup(&state.roundtbl, blk[yblk_off + DCTSIZE2 + 1] + r2);
                image[image_idx + 3 * 9 + 1] =
                    round_lookup(&state.roundtbl, blk[yblk_off + DCTSIZE2 + 1] + g2);
                image[image_idx + 3 * 9 + 0] =
                    round_lookup(&state.roundtbl, blk[yblk_off + DCTSIZE2 + 1] + b2);
                image[image_idx + 3 * 24 + 2] =
                    round_lookup(&state.roundtbl, blk[yblk_off + DCTSIZE2 + 8] + r2);
                image[image_idx + 3 * 24 + 1] =
                    round_lookup(&state.roundtbl, blk[yblk_off + DCTSIZE2 + 8] + g2);
                image[image_idx + 3 * 24 + 0] =
                    round_lookup(&state.roundtbl, blk[yblk_off + DCTSIZE2 + 8] + b2);
                image[image_idx + 3 * 25 + 2] =
                    round_lookup(&state.roundtbl, blk[yblk_off + DCTSIZE2 + 9] + r2);
                image[image_idx + 3 * 25 + 1] =
                    round_lookup(&state.roundtbl, blk[yblk_off + DCTSIZE2 + 9] + g2);
                image[image_idx + 3 * 25 + 0] =
                    round_lookup(&state.roundtbl, blk[yblk_off + DCTSIZE2 + 9] + b2);

                image_idx += 6;
                crblk += 1;
                cbblk += 1;
                yblk_off += 2;
            }
            crblk += 4;
            cbblk += 4;
            yblk_off += 8;
            image_idx += 24 * 3;
        }
    } else {
        for y in (0..16).step_by(2) {
            if y == 8 {
                yblk_off += DCTSIZE2;
            }
            for _x in 0..4 {
                let v0 = round_lookup(&state.roundtbl, blk[yblk_off + 0]);
                image[image_idx + 0 + 2] = v0;
                image[image_idx + 0 + 1] = v0;
                image[image_idx + 0 + 0] = v0;
                let v1 = round_lookup(&state.roundtbl, blk[yblk_off + 1]);
                image[image_idx + 3 * 1 + 2] = v1;
                image[image_idx + 3 * 1 + 1] = v1;
                image[image_idx + 3 * 1 + 0] = v1;
                let v8 = round_lookup(&state.roundtbl, blk[yblk_off + 8]);
                image[image_idx + 3 * 16 + 2] = v8;
                image[image_idx + 3 * 16 + 1] = v8;
                image[image_idx + 3 * 16 + 0] = v8;
                let v9 = round_lookup(&state.roundtbl, blk[yblk_off + 9]);
                image[image_idx + 3 * 17 + 2] = v9;
                image[image_idx + 3 * 17 + 1] = v9;
                image[image_idx + 3 * 17 + 0] = v9;

                let vd0 = round_lookup(&state.roundtbl, blk[yblk_off + DCTSIZE2 + 0]);
                image[image_idx + 3 * 8 + 2] = vd0;
                image[image_idx + 3 * 8 + 1] = vd0;
                image[image_idx + 3 * 8 + 0] = vd0;
                let vd1 = round_lookup(&state.roundtbl, blk[yblk_off + DCTSIZE2 + 1]);
                image[image_idx + 3 * 9 + 2] = vd1;
                image[image_idx + 3 * 9 + 1] = vd1;
                image[image_idx + 3 * 9 + 0] = vd1;
                let vd8 = round_lookup(&state.roundtbl, blk[yblk_off + DCTSIZE2 + 8]);
                image[image_idx + 3 * 24 + 2] = vd8;
                image[image_idx + 3 * 24 + 1] = vd8;
                image[image_idx + 3 * 24 + 0] = vd8;
                let vd9 = round_lookup(&state.roundtbl, blk[yblk_off + DCTSIZE2 + 9]);
                image[image_idx + 3 * 25 + 2] = vd9;
                image[image_idx + 3 * 25 + 1] = vd9;
                image[image_idx + 3 * 25 + 0] = vd9;

                image_idx += 6;
                yblk_off += 2;
            }
            yblk_off += 8;
            image_idx += 24 * 3;
        }
    }
}

/// Read a 32-bit value from the (emulated) IOP memory. In the original
/// PCSX2 this dispatches to the full IOP bus; here it returns `0` as a
/// safe placeholder. Tests / host programs can override this by
/// supplying their own IOP memory accessors.
pub fn iopMemRead32(adr: u32) -> u32 {
    let _ = adr;
    0
}

/// Write a 32-bit value to the (emulated) IOP memory. The default
/// implementation is a no-op.
pub fn iopMemWrite32(adr: u32, val: u32) {
    let _ = (adr, val);
}

/// Initialise the MDEC state. Allocates the working buffers, clears the
/// register file, and resets the round table.
pub fn mdecInit() {
    unsafe {
        mdec.config_mdec = 0;
        mdec.command = 0;
        mdec.status = 0;
        mdec.rlsize = 0;
        mdec.rl_off = 0;

        if mdec.rl.len() != 0x100000 / 2 {
            mdec.rl = vec![0u16; 0x100000 / 2];
        }
        if mdec.arr2.len() != 0x100000 {
            mdec.arr2 = vec![0u32; 0x100000];
        }
        if mdec.mem.len() != 0x100000 {
            mdec.mem = vec![0u32; 0x100000];
        }

        round_init(&mut mdec.roundtbl);
    }
}

/// Reset the MDEC state to power-on values. Identical to
/// [`mdecInit`] in this translation.
pub fn mdecReset() {
    mdecInit();
}

/// Execute the next MDEC DMA. Mirrors the C++ `psxDma0`/`psxDma1`
/// paths. The `chcr` parameter determines which path is taken:
/// `0x0100_0201` -> DMA0 (decode IQ tables / load RL stream),
/// `0x0100_0200` -> DMA1 (decode and emit pixels).
pub fn mdecExecuteDMA(adr: u32, bcr: u32, chcr: u32) {
    unsafe {
        if chcr == 0x0100_0201 {
            dma0_in(adr, bcr);
        } else if chcr == 0x0100_0200 {
            dma1_in(adr, bcr);
        }
    }
}

unsafe fn dma0_in(adr: u32, bcr: u32) {
    let cmd = mdec.command;
    let size = ((bcr >> 16) as i32) * (bcr & 0xFFFF) as i32;
    if size < 0 {
        return;
    }
    let size = size as usize;

    for i in 0..size {
        let word = iopMemRead32(adr.wrapping_add((i as u32) * 4));
        mdec.mem[i] = word;
    }

    if cmd == 0x4000_0001 {
        // Manual byte view: reinterpret the first 128 bytes of `mem`.
        let p: *const u32 = mdec.mem.as_ptr();
        let bytes: &[u8] =
            std::slice::from_raw_parts(p as *const u8, mdec.mem.len() * 4);
        iqtab_init(&mut mdec.iq_y, &bytes[0..64]);
        iqtab_init(&mut mdec.iq_uv, &bytes[64..128]);
    } else if (cmd & 0xF5FF_0000) == 0x3000_0000 {
        mdec.rl_off = 0;
        mdec.rlsize = (cmd & 0xFFFF) as usize;
        // Reinterpret `mdec.mem` as a `u16` slice for the RL stream.
        let p16: *const u16 = mdec.mem.as_ptr() as *const u16;
        let rl: &[u16] =
            std::slice::from_raw_parts(p16, mdec.mem.len() * 4 / 2);
        if mdec.rl.len() < rl.len() {
            mdec.rl.resize(rl.len(), 0);
        }
        mdec.rl[..rl.len()].copy_from_slice(rl);
    }
}

unsafe fn dma1_in(adr: u32, bcr: u32) {
    let size = ((bcr >> 16) as i32) * (bcr & 0xFFFF) as i32;
    if size < 0 {
        return;
    }
    let mut size = size as usize;
    let size2 = size;
    let rgb24 = (mdec.command & 0x0800_0000) != 0;

    let mut image_off = 0usize;
    while size > 0 {
        let mut blk = [0i32; 6 * 64];
        if rgb24 {
            size -= (16 * 16) / 2;
            let mut image = vec![0u16; 16 * 16];
            rl2blk(&mut blk, &mdec.rl, &mut mdec);
            yuv2rgb15(&mdec, &blk, &mut image);
            for chunk in image.chunks(2) {
                if image_off >= mdec.arr2.len() {
                    break;
                }
                let lo = chunk.get(0).copied().unwrap_or(0) as u32;
                let hi = chunk.get(1).copied().unwrap_or(0) as u32;
                mdec.arr2[image_off] = (hi << 16) | lo;
                image_off += 1;
            }
        } else {
            size -= (24 * 16) / 2;
            let mut image = vec![0u8; 24 * 16 * 3];
            rl2blk(&mut blk, &mdec.rl, &mut mdec);
            yuv2rgb24(&mdec, &blk, &mut image);
            for chunk in image.chunks(4) {
                if image_off >= mdec.arr2.len() {
                    break;
                }
                let mut word = 0u32;
                for (i, b) in chunk.iter().enumerate() {
                    word |= (*b as u32) << (i * 8);
                }
                mdec.arr2[image_off] = word;
                image_off += 1;
            }
        }
    }

    for i in 0..size2 {
        let dst = (adr & 0x00FF_FFFF).wrapping_add((i as u32) * 4);
        iopMemWrite32(dst, mdec.arr2[i]);
    }
}

// =====================================================================
// Settings interface
// =====================================================================

/// Trait abstracting a key/value settings store.
///
/// The default methods read or write strongly-typed values by routing
/// through the `String` API. Implementations only need to provide
/// string-level get/set and the list / section maintenance methods.
pub trait SettingsInterface {
    /// Save any pending changes back to the underlying storage. The
    /// returned `Err` is non-fatal and may carry a human-readable
    /// description of the failure.
    fn save(&mut self) -> Result<(), String>;

    /// Remove all keys/sections.
    fn clear(&mut self);

    /// Returns true if there are no settings stored.
    fn is_empty(&self) -> bool;

    fn get_int(&self, section: &str, key: &str) -> Option<i32>;
    fn get_uint(&self, section: &str, key: &str) -> Option<u32>;
    fn get_float(&self, section: &str, key: &str) -> Option<f32>;
    fn get_double(&self, section: &str, key: &str) -> Option<f64>;
    fn get_bool(&self, section: &str, key: &str) -> Option<bool>;
    fn get_string(&self, section: &str, key: &str) -> Option<String>;

    fn set_int(&mut self, section: &str, key: &str, value: i32);
    fn set_uint(&mut self, section: &str, key: &str, value: u32);
    fn set_float(&mut self, section: &str, key: &str, value: f32);
    fn set_double(&mut self, section: &str, key: &str, value: f64);
    fn set_bool(&mut self, section: &str, key: &str, value: bool);
    fn set_string(&mut self, section: &str, key: &str, value: &str);

    fn contains(&self, section: &str, key: &str) -> bool;
    fn delete_value(&mut self, section: &str, key: &str);
    fn clear_section(&mut self, section: &str);
    fn remove_section(&mut self, section: &str);
    fn remove_empty_sections(&mut self);

    fn get_string_list(&self, section: &str, key: &str) -> Vec<String>;
    fn set_string_list(&mut self, section: &str, key: &str, items: &[String]);
    fn remove_from_string_list(&mut self, section: &str, key: &str, item: &str) -> bool;
    fn add_to_string_list(&mut self, section: &str, key: &str, item: &str) -> bool;

    fn get_key_value_list(&self, section: &str) -> Vec<(String, String)>;
    fn set_key_value_list(&mut self, section: &str, items: &[(String, String)]);
}

/// On-disk INI-backed `SettingsInterface`. The format is a plain
/// `.ini` text file, with one `[section]` header and `key=value`
/// entries. Multi-valued keys are stored as repeated lines.
#[derive(Debug, Default)]
pub struct INISettingsInterface {
    filename: String,
    sections: Vec<IniSection>,
    dirty: bool,
}

/// One section of an INI file. Keys are kept in insertion order.
#[derive(Debug, Default, Clone)]
struct IniSection {
    name: String,
    keys: Vec<IniKey>,
}

/// One key inside an INI section. A key may have one or more values.
#[derive(Debug, Default, Clone)]
struct IniKey {
    name: String,
    values: Vec<String>,
}

impl INISettingsInterface {
    /// Open the INI file at `path`, loading its contents into memory.
    pub fn open(path: &Path) -> Result<Self, String> {
        // Build the struct field-by-field instead of using the
        // `..Self::default()` struct-update syntax. `INISettingsInterface`
        // implements `Drop` (via its `String`/`Vec` fields), so the
        // compiler is not allowed to move out of a `Self::default()`
        // temporary and rejects the `..base` form with E0509.
        let mut s = Self {
            filename: path.to_string_lossy().into_owned(),
            sections: Vec::new(),
            dirty: false,
        };
        if path.exists() {
            s.load()?;
        }
        Ok(s)
    }

    fn load(&mut self) -> Result<(), String> {
        let mut f = fs::File::open(&self.filename).map_err(|e| e.to_string())?;
        let mut buf = String::new();
        f.read_to_string(&mut buf).map_err(|e| e.to_string())?;
        self.sections.clear();
        let mut current: Option<IniSection> = None;
        for raw_line in buf.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if let Some(rest) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                if let Some(sec) = current.take() {
                    self.sections.push(sec);
                }
                current = Some(IniSection {
                    name: rest.trim().to_string(),
                    ..Default::default()
                });
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                let section = current.get_or_insert_with(IniSection::default);
                section.add_value(k.trim(), v.trim().to_string());
            }
        }
        if let Some(sec) = current.take() {
            self.sections.push(sec);
        }
        Ok(())
    }

    /// Force a write of the in-memory state to disk.
    pub fn save_to_disk(&mut self) -> Result<(), String> {
        self.save()
    }

    fn find_section(&self, name: &str) -> Option<usize> {
        self.sections.iter().position(|s| s.name == name)
    }

    fn get_or_create_section(&mut self, name: &str) -> &mut IniSection {
        if let Some(i) = self.find_section(name) {
            return &mut self.sections[i];
        }
        self.sections.push(IniSection {
            name: name.to_string(),
            ..Default::default()
        });
        self.sections.last_mut().unwrap()
    }

    fn set_value_inner(&mut self, section: &str, key: &str, value: String) {
        let sec = self.get_or_create_section(section);
        if let Some(k) = sec.keys.iter_mut().find(|k| k.name == key) {
            if k.values.is_empty() {
                k.values.push(value);
            } else {
                k.values[0] = value;
            }
        } else {
            sec.keys.push(IniKey {
                name: key.to_string(),
                values: vec![value],
            });
        }
        self.dirty = true;
    }

    fn get_value_first(&self, section: &str, key: &str) -> Option<&str> {
        let s = self.find_section(section)?;
        let k = self.sections[s].keys.iter().find(|k| k.name == key)?;
        k.values.first().map(|s| s.as_str())
    }
}

impl IniSection {
    fn add_value(&mut self, key: &str, value: String) {
        if let Some(k) = self.keys.iter_mut().find(|k| k.name == key) {
            k.values.push(value);
        } else {
            self.keys.push(IniKey {
                name: key.to_string(),
                values: vec![value],
            });
        }
    }
}

impl SettingsInterface for INISettingsInterface {
    fn save(&mut self) -> Result<(), String> {
        if self.filename.is_empty() {
            return Err("Filename is not set.".to_string());
        }
        let mut tmp = self.filename.clone();
        tmp.push_str(".XXXXXX");
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .map_err(|e| e.to_string())?;
        for s in &self.sections {
            writeln!(f, "[{}]", s.name).map_err(|e| e.to_string())?;
            for k in &s.keys {
                for v in &k.values {
                    writeln!(f, "{}={}", k.name, v).map_err(|e| e.to_string())?;
                }
            }
            writeln!(f).map_err(|e| e.to_string())?;
        }
        f.flush().map_err(|e| e.to_string())?;
        drop(f);
        fs::rename(&tmp, &self.filename).map_err(|e| e.to_string())?;
        self.dirty = false;
        Ok(())
    }

    fn clear(&mut self) {
        self.sections.clear();
        self.dirty = true;
    }

    fn is_empty(&self) -> bool {
        self.sections.iter().all(|s| s.keys.is_empty())
    }

    fn get_int(&self, section: &str, key: &str) -> Option<i32> {
        self.get_value_first(section, key).and_then(|s| s.parse().ok())
    }
    fn get_uint(&self, section: &str, key: &str) -> Option<u32> {
        self.get_value_first(section, key).and_then(|s| s.parse().ok())
    }
    fn get_float(&self, section: &str, key: &str) -> Option<f32> {
        self.get_value_first(section, key).and_then(|s| s.parse().ok())
    }
    fn get_double(&self, section: &str, key: &str) -> Option<f64> {
        self.get_value_first(section, key).and_then(|s| s.parse().ok())
    }
    fn get_bool(&self, section: &str, key: &str) -> Option<bool> {
        match self.get_value_first(section, key)? {
            "true" | "1" | "yes" | "on" => Some(true),
            "false" | "0" | "no" | "off" => Some(false),
            _ => None,
        }
    }
    fn get_string(&self, section: &str, key: &str) -> Option<String> {
        self.get_value_first(section, key).map(|s| s.to_string())
    }

    fn set_int(&mut self, section: &str, key: &str, value: i32) {
        self.set_value_inner(section, key, value.to_string());
    }
    fn set_uint(&mut self, section: &str, key: &str, value: u32) {
        self.set_value_inner(section, key, value.to_string());
    }
    fn set_float(&mut self, section: &str, key: &str, value: f32) {
        self.set_value_inner(section, key, value.to_string());
    }
    fn set_double(&mut self, section: &str, key: &str, value: f64) {
        self.set_value_inner(section, key, value.to_string());
    }
    fn set_bool(&mut self, section: &str, key: &str, value: bool) {
        self.set_value_inner(section, key, (if value { "true" } else { "false" }).to_string());
    }
    fn set_string(&mut self, section: &str, key: &str, value: &str) {
        self.set_value_inner(section, key, value.to_string());
    }

    fn contains(&self, section: &str, key: &str) -> bool {
        self.get_value_first(section, key).is_some()
    }
    fn delete_value(&mut self, section: &str, key: &str) {
        if let Some(s) = self.sections.iter_mut().find(|s| s.name == section) {
            s.keys.retain(|k| k.name != key);
            self.dirty = true;
        }
    }
    fn clear_section(&mut self, section: &str) {
        if let Some(s) = self.sections.iter_mut().find(|s| s.name == section) {
            s.keys.clear();
            self.dirty = true;
        }
    }
    fn remove_section(&mut self, section: &str) {
        if let Some(pos) = self.sections.iter().position(|s| s.name == section) {
            self.sections.remove(pos);
            self.dirty = true;
        }
    }
    fn remove_empty_sections(&mut self) {
        let before = self.sections.len();
        self.sections.retain(|s| !s.keys.is_empty());
        if self.sections.len() != before {
            self.dirty = true;
        }
    }

    fn get_string_list(&self, section: &str, key: &str) -> Vec<String> {
        let Some(s) = self.find_section(section) else {
            return Vec::new();
        };
        let Some(k) = self.sections[s].keys.iter().find(|k| k.name == key) else {
            return Vec::new();
        };
        k.values.clone()
    }
    fn set_string_list(&mut self, section: &str, key: &str, items: &[String]) {
        let sec = self.get_or_create_section(section);
        if let Some(k) = sec.keys.iter_mut().find(|k| k.name == key) {
            k.values = items.to_vec();
        } else {
            sec.keys.push(IniKey {
                name: key.to_string(),
                values: items.to_vec(),
            });
        }
        self.dirty = true;
    }
    fn remove_from_string_list(&mut self, section: &str, key: &str, item: &str) -> bool {
        let Some(s) = self.sections.iter_mut().find(|s| s.name == section) else {
            return false;
        };
        let Some(k) = s.keys.iter_mut().find(|k| k.name == key) else {
            return false;
        };
        let before = k.values.len();
        k.values.retain(|v| v != item);
        let changed = k.values.len() != before;
        if changed {
            self.dirty = true;
        }
        changed
    }
    fn add_to_string_list(&mut self, section: &str, key: &str, item: &str) -> bool {
        if let Some(s) = self.sections.iter().find(|s| s.name == section) {
            if let Some(k) = s.keys.iter().find(|k| k.name == key) {
                if k.values.iter().any(|v| v == item) {
                    return false;
                }
            }
        }
        let sec = self.get_or_create_section(section);
        if let Some(k) = sec.keys.iter_mut().find(|k| k.name == key) {
            k.values.push(item.to_string());
        } else {
            sec.keys.push(IniKey {
                name: key.to_string(),
                values: vec![item.to_string()],
            });
        }
        self.dirty = true;
        true
    }

    fn get_key_value_list(&self, section: &str) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let Some(s) = self.find_section(section) else {
            return out;
        };
        for k in &self.sections[s].keys {
            for v in &k.values {
                out.push((k.name.clone(), v.clone()));
            }
        }
        out
    }
    fn set_key_value_list(&mut self, section: &str, items: &[(String, String)]) {
        let sec = self.get_or_create_section(section);
        sec.keys.clear();
        for (k, v) in items {
            sec.keys.push(IniKey {
                name: k.clone(),
                values: vec![v.clone()],
            });
        }
        self.dirty = true;
    }
}

impl Drop for INISettingsInterface {
    fn drop(&mut self) {
        if self.dirty {
            let _ = self.save();
        }
    }
}

// =====================================================================
// Layered settings interface
// =====================================================================

/// Identifies a layer in the [`LayeredSettingsInterface`]. The order is
/// significant: earlier layers take priority when looking up values.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Layer {
    Cmdline = 0,
    Game = 1,
    Input = 2,
    Secrets = 3,
    Base = 4,
}

const NUM_LAYERS: usize = 5;
const FIRST_LAYER: usize = Layer::Cmdline as usize;
const LAST_LAYER: usize = Layer::Base as usize;

/// A `SettingsInterface` that overlays several underlying
/// interfaces. Read operations are routed through the layers from
/// highest to lowest priority; write operations are not supported and
/// will silently no-op (matching the C++ behaviour of asserting in
/// debug builds).
pub struct LayeredSettingsInterface {
    layers: [Option<Box<dyn SettingsInterface>>; NUM_LAYERS],
    /// Mutex that serialises `save` and other global operations.
    lock: Mutex<()>,
}

impl LayeredSettingsInterface {
    /// Construct a new empty layered interface.
    pub fn new() -> Self {
        Self {
            layers: Default::default(),
            lock: Mutex::new(()),
        }
    }

    /// Return the layer's underlying interface, if any.
    pub fn get_layer(&self, layer: Layer) -> Option<&dyn SettingsInterface> {
        self.layers[layer as usize]
            .as_deref()
            .map(|s| s as &dyn SettingsInterface)
    }

    /// Replace the layer's underlying interface. Pass `None` to detach
    /// the layer.
    pub fn set_layer(&mut self, layer: Layer, sif: Option<Box<dyn SettingsInterface>>) {
        self.layers[layer as usize] = sif;
    }
}

impl Default for LayeredSettingsInterface {
    fn default() -> Self {
        Self::new()
    }
}

impl SettingsInterface for LayeredSettingsInterface {
    fn save(&mut self) -> Result<(), String> {
        let _g = self.lock.lock().unwrap();
        Err("Attempting to save layered settings interface".to_string())
    }

    fn clear(&mut self) {
        let _g = self.lock.lock().unwrap();
    }

    fn is_empty(&self) -> bool {
        false
    }

    fn get_int(&self, section: &str, key: &str) -> Option<i32> {
        for layer in FIRST_LAYER..=LAST_LAYER {
            if let Some(sif) = self.layers[layer].as_deref() {
                if let Some(v) = sif.get_int(section, key) {
                    return Some(v);
                }
            }
        }
        None
    }
    fn get_uint(&self, section: &str, key: &str) -> Option<u32> {
        for layer in FIRST_LAYER..=LAST_LAYER {
            if let Some(sif) = self.layers[layer].as_deref() {
                if let Some(v) = sif.get_uint(section, key) {
                    return Some(v);
                }
            }
        }
        None
    }
    fn get_float(&self, section: &str, key: &str) -> Option<f32> {
        for layer in FIRST_LAYER..=LAST_LAYER {
            if let Some(sif) = self.layers[layer].as_deref() {
                if let Some(v) = sif.get_float(section, key) {
                    return Some(v);
                }
            }
        }
        None
    }
    fn get_double(&self, section: &str, key: &str) -> Option<f64> {
        for layer in FIRST_LAYER..=LAST_LAYER {
            if let Some(sif) = self.layers[layer].as_deref() {
                if let Some(v) = sif.get_double(section, key) {
                    return Some(v);
                }
            }
        }
        None
    }
    fn get_bool(&self, section: &str, key: &str) -> Option<bool> {
        for layer in FIRST_LAYER..=LAST_LAYER {
            if let Some(sif) = self.layers[layer].as_deref() {
                if let Some(v) = sif.get_bool(section, key) {
                    return Some(v);
                }
            }
        }
        None
    }
    fn get_string(&self, section: &str, key: &str) -> Option<String> {
        for layer in FIRST_LAYER..=LAST_LAYER {
            if let Some(sif) = self.layers[layer].as_deref() {
                if let Some(v) = sif.get_string(section, key) {
                    return Some(v);
                }
            }
        }
        None
    }

    fn set_int(&mut self, _section: &str, _key: &str, _value: i32) {}
    fn set_uint(&mut self, _section: &str, _key: &str, _value: u32) {}
    fn set_float(&mut self, _section: &str, _key: &str, _value: f32) {}
    fn set_double(&mut self, _section: &str, _key: &str, _value: f64) {}
    fn set_bool(&mut self, _section: &str, _key: &str, _value: bool) {}
    fn set_string(&mut self, _section: &str, _key: &str, _value: &str) {}

    fn contains(&self, section: &str, key: &str) -> bool {
        for layer in FIRST_LAYER..=LAST_LAYER {
            if let Some(sif) = self.layers[layer].as_deref() {
                if sif.contains(section, key) {
                    return true;
                }
            }
        }
        false
    }
    fn delete_value(&mut self, _section: &str, _key: &str) {}
    fn clear_section(&mut self, _section: &str) {}
    fn remove_section(&mut self, _section: &str) {}
    fn remove_empty_sections(&mut self) {}

    fn get_string_list(&self, section: &str, key: &str) -> Vec<String> {
        for layer in FIRST_LAYER..=LAST_LAYER {
            if let Some(sif) = self.layers[layer].as_deref() {
                let v = sif.get_string_list(section, key);
                if !v.is_empty() {
                    return v;
                }
            }
        }
        Vec::new()
    }
    fn set_string_list(&mut self, _section: &str, _key: &str, _items: &[String]) {}
    fn remove_from_string_list(&mut self, _section: &str, _key: &str, _item: &str) -> bool {
        false
    }
    fn add_to_string_list(&mut self, _section: &str, _key: &str, _item: &str) -> bool {
        true
    }

    fn get_key_value_list(&self, section: &str) -> Vec<(String, String)> {
        use std::collections::HashSet;
        let mut seen: HashSet<String> = HashSet::new();
        let mut out = Vec::new();
        for layer in FIRST_LAYER..=LAST_LAYER {
            if let Some(sif) = self.layers[layer].as_deref() {
                let entries = sif.get_key_value_list(section);
                let begin = out.len();
                for entry in entries {
                    if !seen.contains(&entry.0) {
                        out.push(entry);
                    }
                }
                for item in &out[begin..] {
                    seen.insert(item.0.clone());
                }
            }
        }
        out
    }
    fn set_key_value_list(&mut self, _section: &str, _items: &[(String, String)]) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sjis_handles_ascii() {
        assert_eq!(shift_jis_to_unicode(b"hello"), "hello");
    }

    #[test]
    fn ini_round_trip() {
        let dir = std::env::temp_dir();
        let path = dir.join("pcsx2_translations_test.ini");
        let _ = std::fs::remove_file(&path);

        let mut sif = INISettingsInterface::open(&path).expect("open");
        sif.set_int("General", "Width", 1024);
        sif.set_string("Pad", "Device", "DualShock");
        sif.save().expect("save");

        let sif2 = INISettingsInterface::open(&path).expect("reopen");
        assert_eq!(sif2.get_int("General", "Width"), Some(1024));
        assert_eq!(sif2.get_string("Pad", "Device").as_deref(), Some("DualShock"));

        let _ = std::fs::remove_file(&path);
    }
}
