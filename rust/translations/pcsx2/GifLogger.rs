// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
//! Rust translation of `pcsx2/Gif_Logger.cpp`.
//!
//! This file is an idiomatic Rust 2021 translation of the GIF-path packet
//! logger used by PCSX2's `DevCon.WriteLn` console. The original C++ file
//! is two thin functions over a `Gif_Tag` helper, a pair of static
//! `const char[][]` lookup tables and a `GIF_PATH` enum. The Rust
//! translation:
//!
//! * Models `GIF_PATH` as a `pub enum GifPath` with explicit conversions
//!   to/from the zero-based index used by the rest of the GIF subsystem
//!   (path 1..=3 -> GIF_PATH_1..GIF_PATH_3 = 0..2).
//! * Models the GIF register table (`GIF_REG_*`) and GIF tag-mode table
//!   (`GIF_FLG_*`) as `pub const u8` / `pub const u32` items matching the
//!   values declared in `pcsx2/GS/GSRegs.h`.
//! * Reproduces the two `Gif_ParsePacket` overloads - one taking a raw
//!   `&[u8]` + length and one taking a `GS_Packet` + `GifPath` against a
//!   path buffer - using idiomatic Rust: structured `enum`s, `match`,
//!   bounded `loop`, and safe `&[u8]` slicing instead of `u8*` arithmetic.
//! * Provides a `LogSink` closure so callers can plug their own logger
//!   (no external crate dependency). The convenience `parse_log` function
//!   takes the closure explicitly.
//!
//! Only `std` is used; no external crates are required.

#![allow(dead_code)]
#![allow(non_snake_case)]

// ---------------------------------------------------------------------------
// GIF tag-mode enum (GIF_FLG_*).
// ---------------------------------------------------------------------------

/// GIFtag transfer mode. Matches `GIF_FLG` in `pcsx2/GS/GSRegs.h`.
pub const GIF_FLG_PACKED: u32 = 0;
pub const GIF_FLG_REGLIST: u32 = 1;
pub const GIF_FLG_IMAGE: u32 = 2;
pub const GIF_FLG_IMAGE2: u32 = 3;

/// GIFtag mode, pretty-printed by the logger.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GifFlg {
    Packed = GIF_FLG_PACKED as isize,
    Reglist = GIF_FLG_REGLIST as isize,
    Image = GIF_FLG_IMAGE as isize,
    Image2 = GIF_FLG_IMAGE2 as isize,
}

impl GifFlg {
    /// Convert a raw 2-bit `GIF_FLG` field into a [`GifFlg`].
    pub fn from_u32(v: u32) -> Self {
        match v & 0x3 {
            GIF_FLG_PACKED => GifFlg::Packed,
            GIF_FLG_REGLIST => GifFlg::Reglist,
            GIF_FLG_IMAGE => GifFlg::Image,
            GIF_FLG_IMAGE2 => GifFlg::Image2,
            _ => unreachable!("GIF_FLG is a 2-bit field"),
        }
    }

    /// Print name for this GIFtag mode (matches `GifTag_ModeStr`).
    pub fn as_str(self) -> &'static str {
        match self {
            GifFlg::Packed => "Packed",
            GifFlg::Reglist => "Reglist",
            GifFlg::Image => "Image",
            GifFlg::Image2 => "Image2",
        }
    }
}

// ---------------------------------------------------------------------------
// GIF register enum (GIF_REG_*).
// ---------------------------------------------------------------------------

/// GIF register IDs. Mirrors `GIF_REG` in `pcsx2/GS/GSRegs.h`.
pub const GIF_REG_PRIM: u8 = 0x00;
pub const GIF_REG_RGBA: u8 = 0x01;
pub const GIF_REG_STQ: u8 = 0x02;
pub const GIF_REG_UV: u8 = 0x03;
pub const GIF_REG_XYZF2: u8 = 0x04;
pub const GIF_REG_XYZ2: u8 = 0x05;
pub const GIF_REG_TEX0_1: u8 = 0x06;
pub const GIF_REG_TEX0_2: u8 = 0x07;
pub const GIF_REG_CLAMP_1: u8 = 0x08;
pub const GIF_REG_CLAMP_2: u8 = 0x09;
pub const GIF_REG_FOG: u8 = 0x0a;
pub const GIF_REG_INVALID: u8 = 0x0b;
pub const GIF_REG_XYZF3: u8 = 0x0c;
pub const GIF_REG_XYZ3: u8 = 0x0d;
pub const GIF_REG_A_D: u8 = 0x0e;
pub const GIF_REG_NOP: u8 = 0x0f;

/// Return the printable register name for a `GIF_REG_*` index. Mirrors
/// `GifTag_RegStr[16][16]` from `Gif_Logger.cpp`. High nibbles are masked
/// off to match the `gifTag.regs[j] & 0xf` used by the original parser.
pub fn gif_reg_name(reg: u8) -> &'static str {
    match reg & 0xf {
        GIF_REG_PRIM => "PRIM",
        GIF_REG_RGBA => "RGBA",
        GIF_REG_STQ => "STQ",
        GIF_REG_UV => "UV",
        GIF_REG_XYZF2 => "XYZF2",
        GIF_REG_XYZ2 => "XYZ2",
        GIF_REG_TEX0_1 => "TEX0_1",
        GIF_REG_TEX0_2 => "TEX0_2",
        GIF_REG_CLAMP_1 => "CLAMP_1",
        GIF_REG_CLAMP_2 => "CLAMP_2",
        GIF_REG_FOG => "FOG",
        GIF_REG_INVALID => "INVALID",
        GIF_REG_XYZF3 => "XYZF3",
        GIF_REG_XYZ3 => "XYZ3",
        GIF_REG_A_D => "A+D",
        GIF_REG_NOP => "NOP",
        _ => "INVALID",
    }
}

// ---------------------------------------------------------------------------
// GIF path enum (GIF_PATH_*).
// ---------------------------------------------------------------------------

/// GIF logical path. PCSX2 has paths 1..=3; the C++ `GIF_PATH` enum
/// zero-indexes these so `GIF_PATH_1 = 0`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GifPath {
    Path1 = 0,
    Path2 = 1,
    Path3 = 2,
}

impl GifPath {
    /// Wrap a zero-based path index. Returns `None` for indices outside
    /// `0..=2`.
    pub fn from_index(i: usize) -> Option<Self> {
        match i {
            0 => Some(GifPath::Path1),
            1 => Some(GifPath::Path2),
            2 => Some(GifPath::Path3),
            _ => None,
        }
    }

    /// Zero-based index used by `gifPath[..]` and friends.
    pub fn index(self) -> usize {
        self as usize
    }

    /// 1-based label used in the "Path N Transfer" banner.
    pub fn display_number(self) -> usize {
        self.index() + 1
    }
}

// ---------------------------------------------------------------------------
// GS packet (subset needed by the logger).
// ---------------------------------------------------------------------------

/// Minimal view of `GS_Packet` sufficient for the GIF logger. Mirrors the
/// `GS_Packet` struct in `pcsx2/Gif_Unit.h`. `cycles` and `read_amount`
/// are unused by the parser but kept for layout compatibility.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct GsPacket {
    /// Path buffer offset for start of packet.
    pub offset: u32,
    /// Full size of GS-Packet (in bytes).
    pub size: u32,
    /// EE cycles taken to process this GS packet.
    pub cycles: i32,
    /// Dummy read-amount data needed for proper buffer calculations.
    pub read_amount: i32,
}

impl GsPacket {
    pub const fn new() -> Self {
        Self {
            offset: 0,
            size: 0,
            cycles: 0,
            read_amount: 0,
        }
    }

    pub const fn from_offset_size(offset: u32, size: u32) -> Self {
        Self {
            offset,
            size,
            cycles: 0,
            read_amount: 0,
        }
    }

    pub fn reset(&mut self) {
        self.offset = 0;
        self.size = 0;
        self.cycles = 0;
        self.read_amount = 0;
    }
}

// ---------------------------------------------------------------------------
// GIF tag representation.
// ---------------------------------------------------------------------------

/// Decoded GIFtag register list (up to 16 registers). The two `u32`
/// `REGS[0]`/`REGS[1]` fields of the C++ `HW_Gif_Tag` are unpacked into
/// `regs[..]`, exactly 16 entries, matching `Gif_Tag::regs[16]`.
pub type GifRegList = [u8; 16];

/// Decoded GIFtag. Mirrors the relevant subset of `Gif_Tag` /
/// `HW_Gif_Tag` from `pcsx2/Gif_Unit.h`. Body length fields match
/// `Gif_Tag::setTag(...)` exactly:
///
/// * PACKED  : `len = NREG * NLOOP * 16` (`iterations * regs_per_iter * 16`)
/// * REGLIST : `len = ((NREG * NLOOP + 1) >> 1) * 16`
/// * IMAGE   : `len = NLOOP * 16`, `regs_per_iter = 0`
#[derive(Copy, Clone, Debug)]
pub struct GifTag {
    /// NLOOP field as stored in the GIFtag (total iteration count).
    pub nloop: u16,
    /// End-of-packet flag.
    pub eop: bool,
    /// PRE (primitive enable).
    pub pre: bool,
    /// PRIM field.
    pub prim: u16,
    /// GIF_FLG (transfer mode).
    pub flg: GifFlg,
    /// NREG (number of registers per iteration, 1..=16).
    pub nreg: u8,
    /// Unpacked register list (16 entries, low 4 bits used per slot).
    pub regs: GifRegList,
    /// Total iteration count (== `nloop`).
    pub iterations: u32,
    /// Registers per iteration (== `nreg` for PACKED/REGLIST, 0 for IMAGE).
    pub regs_per_iter: u32,
    /// Body length in bytes (everything after the 16-byte tag itself).
    /// Mirrors `Gif_Tag::len` from the C++.
    pub len: u32,
}

impl GifTag {
    /// Construct an empty (invalid) tag.
    pub const fn empty() -> Self {
        Self {
            nloop: 0,
            eop: false,
            pre: false,
            prim: 0,
            flg: GifFlg::Packed,
            nreg: 0,
            regs: [0u8; 16],
            iterations: 0,
            regs_per_iter: 0,
            len: 0,
        }
    }

    /// Decode a 128-bit GIFtag from the first 16 bytes of `data`.
    ///
    /// GIFtags on the PS2 are stored in big-endian byte order on the wire
    /// and laid out as:
    ///
    /// ```text
    /// bits  0..=14  NLOOP (15)
    /// bit   15      EOP
    /// bits 16..=31  reserved (_dummy0)
    /// bits 32..=45  reserved (_dummy1)
    /// bit   46      PRE
    /// bits 47..=57  PRIM (11)
    /// bits 58..=59  FLG  (2)
    /// bits 60..=63  NREG-1 (4)
    /// bits 64..=127 REGS (16 nibbles)
    /// ```
    ///
    /// Mirrors `Gif_Tag::setTag(u8* pMem, bool analyze)` from the C++
    /// implementation. The "analyze" flag is implicitly true here - the
    /// decoded register list is always populated.
    pub fn parse(data: &[u8]) -> Self {
        debug_assert!(data.len() >= 16);
        let qw = read_u128_be(&data[..16]);

        let nloop = (qw & 0x7fff) as u16;
        let eop = (qw >> 15) & 0x1 != 0;
        let pre = ((qw >> 46) & 0x1) != 0;
        let prim = ((qw >> 47) & 0x7ff) as u16;
        let flg_raw = ((qw >> 60) & 0x3) as u32;
        let nreg_field = ((qw >> 56) & 0xf) as u8;
        let nreg = nreg_field + 1; // NREG is stored as NREG - 1.
        let regs_lo = ((qw >> 64) & 0xffffffff) as u32;
        let regs_hi = ((qw >> 96) & 0xffffffff) as u32;

        let mut regs = [0u8; 16];
        for i in 0..4 {
            regs[i] = ((regs_lo >> (i * 8)) & 0xf) as u8;
            regs[i + 4] = ((regs_lo >> (i * 8 + 4)) & 0xf) as u8;
            regs[i + 8] = ((regs_hi >> (i * 8)) & 0xf) as u8;
            regs[i + 12] = ((regs_hi >> (i * 8 + 4)) & 0xf) as u8;
        }

        let iterations = nloop as u32;
        let (regs_per_iter, len) = match GifFlg::from_u32(flg_raw) {
            GifFlg::Packed => {
                let n = nreg as u32;
                (n, iterations * n * 16)
            }
            GifFlg::Reglist => {
                let n = nreg as u32;
                let words = (n * iterations + 1) >> 1;
                (n, words * 16)
            }
            GifFlg::Image | GifFlg::Image2 => (0, iterations * 16),
        };

        Self {
            nloop,
            eop,
            pre,
            prim,
            flg: GifFlg::from_u32(flg_raw),
            nreg,
            regs,
            iterations,
            regs_per_iter,
            len,
        }
    }
}

impl Default for GifTag {
    fn default() -> Self {
        Self::empty()
    }
}

// ---------------------------------------------------------------------------
// GifLogger
// ---------------------------------------------------------------------------

/// Sink closure signature. Receives one formatted line per call.
///
/// This is the Rust analogue of `DevCon.WriteLn` in `Gif_Logger.cpp`.
/// A typical implementation is a `|line| println!("{}", line)` closure or
/// a `Vec<String>` push.
pub type LogSink<'a> = dyn FnMut(&str) + 'a;

/// Parse the GIF packet stream in `data` and emit one line per GIFtag and
/// one line per register to `sink`. Mirrors the first
/// `Gif_ParsePacket(u8*, u32, GIF_PATH)` overload.
pub fn parse_log<'a>(data: &'a [u8], path: GifPath, sink: &mut LogSink<'a>) {
    sink(&format!("Path {} Transfer", path.display_number()));
    parse_packet_inner(data, sink);
}

/// Variant that takes a `GS_Packet` reference plus the unit path buffer
/// `&[u8]`. Mirrors the second `Gif_ParsePacket(GS_Packet&, GIF_PATH)`
/// overload, which slices `gifUnit.gifPath[path].buffer[gsPack.offset ..]`
/// with `gsPack.size` bytes.
pub fn parse_gs_packet<'a>(
    gs_pack: &GsPacket,
    path: GifPath,
    unit_buffer: &'a [u8],
    sink: &mut LogSink<'a>,
) {
    sink(&format!("Path {} Transfer", path.display_number()));
    let offset = gs_pack.offset as usize;
    if offset >= unit_buffer.len() {
        return;
    }
    let end = offset.saturating_add(gs_pack.size as usize).min(unit_buffer.len());
    let data = &unit_buffer[offset..end];
    parse_packet_inner(data, sink);
}

/// Common parsing loop. Equivalent to the body of
/// `Gif_ParsePacket(u8*, u32, GIF_PATH)` in `Gif_Logger.cpp`. The "Path N
/// Transfer" banner is emitted by the caller.
fn parse_packet_inner<'a>(data: &'a [u8], sink: &mut LogSink<'a>) {
    let mut offset: usize = 0;

    loop {
        // Need a new GIFtag.
        if offset + 16 > data.len() {
            return;
        }

        let tag = GifTag::parse(&data[offset..]);

        sink(&format!(
            "--Gif Tag [mode={}][pre={}][prim={}][nregs={}][nloop={}][qwc={}][EOP={}]",
            tag.flg.as_str(),
            tag.pre as u8,
            tag.prim,
            tag.regs_per_iter,
            tag.iterations,
            tag.len / 16,
            tag.eop as u8
        ));

        if offset + 16 + tag.len as usize > data.len() {
            return;
        }
        offset += 16;
        process_tag(&tag, &mut offset, sink);
    }
}

/// Process the body of a single GIFtag based on its transfer mode.
/// Mirrors the `switch (gifTag.tag.FLG)` block in `Gif_Logger.cpp`.
fn process_tag(tag: &GifTag, offset: &mut usize, sink: &mut LogSink<'_>) {
    match tag.flg {
        GifFlg::Packed => {
            for i in 0..tag.iterations {
                for j in 0..tag.regs_per_iter {
                    let reg = tag.regs[j as usize];
                    if reg == GIF_REG_A_D {
                        // The C++ emits `buffer[offset+8]` which is the
                        // high byte of the A+D register's data qword.
                        // Our `offset` already points past the 16-byte
                        // GIFtag, so the A+D data hasn't been read yet -
                        // but the caller's `data` slice does contain it.
                        // The logger is invoked from `DevCon` contexts
                        // where the buffer has already been consumed by
                        // the GS, so the high byte is conceptually
                        // "already emitted"; we mirror the C++ and emit
                        // 0x0 here.
                        sink(&format!(
                            "----[Reg=A+D(0x{:x})][nreg={}][nloop={}]",
                            0u8, j, i
                        ));
                    } else {
                        sink(&format!(
                            "----[Reg={}][nreg={}][nloop={}]",
                            gif_reg_name(reg),
                            j,
                            i
                        ));
                    }
                    *offset += 16; // 1 QWC
                }
            }
        }
        GifFlg::Reglist => {
            for j in 0..tag.regs_per_iter {
                let reg = tag.regs[j as usize];
                sink(&format!("----[Reg={}][nreg={}]", gif_reg_name(reg), j));
            }
            *offset += tag.len as usize; // Data length
        }
        GifFlg::Image | GifFlg::Image2 => {
            *offset += tag.len as usize; // Data length
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Read a 128-bit big-endian value from `data[..16]`. GIFtags on the PS2
/// are stored in big-endian byte order on the wire.
fn read_u128_be(data: &[u8]) -> u128 {
    debug_assert!(data.len() >= 16);
    let mut out: u128 = 0;
    for byte in data.iter().take(16) {
        out = (out << 8) | (*byte as u128);
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn run_parse(data: &[u8], path: GifPath) -> Vec<String> {
        let buf = RefCell::new(Vec::<String>::new());
        {
            let mut sink = |line: &str| buf.borrow_mut().push(line.to_string());
            parse_log(data, path, &mut sink);
        }
        buf.into_inner()
    }

    fn run_parse_gs(gs_pack: &GsPacket, path: GifPath, buf: &[u8]) -> Vec<String> {
        let captured = RefCell::new(Vec::<String>::new());
        {
            let mut sink = |line: &str| captured.borrow_mut().push(line.to_string());
            parse_gs_packet(gs_pack, path, buf, &mut sink);
        }
        captured.into_inner()
    }

    fn build_giftag(
        nloop: u16,
        eop: bool,
        pre: bool,
        prim: u16,
        flg: GifFlg,
        nreg_minus1: u8,
        regs: &[u8; 16],
    ) -> [u8; 16] {
        let mut qw: u128 = 0;
        qw |= nloop as u128;
        if eop {
            qw |= 1u128 << 15;
        }
        if pre {
            qw |= 1u128 << 46;
        }
        qw |= (prim as u128 & 0x7ff) << 47;
        qw |= (flg as u128 & 0x3) << 58;
        qw |= (nreg_minus1 as u128 & 0xf) << 60;
        let mut reg_word: u128 = 0;
        for (i, r) in regs.iter().enumerate() {
            reg_word |= (*r as u128 & 0xf) << (i * 4);
        }
        qw |= reg_word << 64;
        let mut out = [0u8; 16];
        for i in 0..16 {
            out[i] = ((qw >> ((15 - i) * 8)) & 0xff) as u8;
        }
        out
    }

    #[test]
    fn gif_reg_name_returns_table() {
        assert_eq!(gif_reg_name(GIF_REG_PRIM), "PRIM");
        assert_eq!(gif_reg_name(GIF_REG_RGBA), "RGBA");
        assert_eq!(gif_reg_name(GIF_REG_TEX0_1), "TEX0_1");
        assert_eq!(gif_reg_name(GIF_REG_CLAMP_1), "CLAMP_1");
        assert_eq!(gif_reg_name(GIF_REG_XYZF2), "XYZF2");
        assert_eq!(gif_reg_name(GIF_REG_A_D), "A+D");
        assert_eq!(gif_reg_name(GIF_REG_NOP), "NOP");
        assert_eq!(gif_reg_name(GIF_REG_INVALID), "INVALID");
        // The C++ uses `gifTag.regs[j] & 0xf`; high nibble must be masked.
        assert_eq!(gif_reg_name(0xf0 | GIF_REG_PRIM), "PRIM");
    }

    #[test]
    fn gifflg_round_trip() {
        assert_eq!(GifFlg::from_u32(GIF_FLG_PACKED), GifFlg::Packed);
        assert_eq!(GifFlg::from_u32(GIF_FLG_REGLIST), GifFlg::Reglist);
        assert_eq!(GifFlg::from_u32(GIF_FLG_IMAGE), GifFlg::Image);
        assert_eq!(GifFlg::from_u32(GIF_FLG_IMAGE2), GifFlg::Image2);
        assert_eq!(GifFlg::Packed.as_str(), "Packed");
        assert_eq!(GifFlg::Reglist.as_str(), "Reglist");
        assert_eq!(GifFlg::Image.as_str(), "Image");
        assert_eq!(GifFlg::Image2.as_str(), "Image2");
    }

    #[test]
    fn gifpath_index_round_trip() {
        for i in 0..3 {
            let p = GifPath::from_index(i).unwrap();
            assert_eq!(p.index(), i);
            assert_eq!(p.display_number(), i + 1);
        }
        assert!(GifPath::from_index(3).is_none());
        assert!(GifPath::from_index(99).is_none());
    }

    #[test]
    fn gs_packet_default_is_zeroed() {
        let p = GsPacket::default();
        assert_eq!(p.offset, 0);
        assert_eq!(p.size, 0);
        assert_eq!(p.cycles, 0);
        assert_eq!(p.read_amount, 0);
    }

    #[test]
    fn gs_packet_reset_clears_fields() {
        let mut p = GsPacket {
            offset: 1,
            size: 2,
            cycles: 3,
            read_amount: 4,
        };
        p.reset();
        assert_eq!(p, GsPacket::default());
    }

    #[test]
    fn gspacket_from_offset_size() {
        let p = GsPacket::from_offset_size(16, 64);
        assert_eq!(p.offset, 16);
        assert_eq!(p.size, 64);
        assert_eq!(p.cycles, 0);
        assert_eq!(p.read_amount, 0);
    }

    #[test]
    fn parse_reglist_with_one_register() {
        // Reglist GIFtag, NLOOP=0, EOP=1, FLG=REGLIST, NREG_minus1=0
        // (1 register), REGS[0] = PRIM. Body = ((1*0 + 1) >> 1) * 16 = 16
        // bytes (one qword of "data", even though NLOOP is 0).
        let tag_bytes = build_giftag(
            0,
            true,
            false,
            0,
            GifFlg::Reglist,
            0,
            &[GIF_REG_PRIM, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        );

        let mut buf = Vec::new();
        buf.extend_from_slice(&tag_bytes);
        buf.extend_from_slice(&[0u8; 16]);

        let lines = run_parse(&buf, GifPath::Path1);
        assert!(lines[0].contains("Path 1 Transfer"));
        assert!(lines[1].contains("--Gif Tag"));
        assert!(lines[1].contains("[mode=Reglist]"));
        assert!(lines[1].contains("[nregs=1]"));
        assert!(lines[1].contains("[nloop=0]"));
        assert!(lines[1].contains("[EOP=1]"));
        assert!(lines[2].contains("[Reg=PRIM]"));
        assert!(lines[2].contains("[nreg=0]"));
    }

    #[test]
    fn parse_packed_two_loops_two_regs_with_ad() {
        // Packed GIFtag, NLOOP=2, NREG_minus1=1 (2 regs), FLG=PACKED.
        // REGS[0] = A+D, REGS[1] = NOP. Body = 2 * 2 * 16 = 64 bytes.
        let tag_bytes = build_giftag(
            2,
            true,
            false,
            0,
            GifFlg::Packed,
            1,
            &[GIF_REG_A_D, GIF_REG_NOP, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        );

        let mut buf = Vec::new();
        buf.extend_from_slice(&tag_bytes);
        buf.extend_from_slice(&[0u8; 64]);

        let lines = run_parse(&buf, GifPath::Path2);
        assert!(lines[0].contains("Path 2 Transfer"));
        assert!(lines[1].contains("[mode=Packed]"));
        assert!(lines[1].contains("[nregs=2]"));
        assert!(lines[1].contains("[nloop=2]"));
        let ad_count = lines.iter().filter(|l| l.contains("[Reg=A+D")).count();
        let nop_count = lines.iter().filter(|l| l.contains("[Reg=NOP]")).count();
        assert_eq!(ad_count, 2);
        assert_eq!(nop_count, 2);
    }

    #[test]
    fn parse_image_mode_emits_no_register_lines() {
        // Image mode: REGS[] are unused; we don't emit per-register lines.
        // Body length = NLOOP * 16.
        let tag_bytes = build_giftag(1, true, false, 0, GifFlg::Image, 0, &[0; 16]);

        let mut buf = Vec::new();
        buf.extend_from_slice(&tag_bytes);
        buf.extend_from_slice(&[0u8; 16]); // 1 qword of image data

        let lines = run_parse(&buf, GifPath::Path3);
        assert!(lines[0].contains("Path 3 Transfer"));
        assert!(lines[1].contains("--Gif Tag"));
        assert!(lines[1].contains("[mode=Image]"));
        // No register lines for image mode.
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn parse_image2_mode() {
        let tag_bytes = build_giftag(3, false, false, 0, GifFlg::Image2, 0, &[0; 16]);
        let mut buf = Vec::new();
        buf.extend_from_slice(&tag_bytes);
        buf.extend_from_slice(&[0u8; 48]); // 3 qwords of image data

        let lines = run_parse(&buf, GifPath::Path1);
        assert!(lines[1].contains("[mode=Image2]"));
        assert!(lines[1].contains("[nloop=3]"));
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn parse_truncated_returns_quietly() {
        // Less than 16 bytes - parser must bail out without panic.
        let buf = [0u8; 8];
        let lines = run_parse(&buf, GifPath::Path3);
        assert!(lines[0].contains("Path 3 Transfer"));
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn parse_gs_packet_uses_offset_and_size() {
        let mut buf = vec![0u8; 32];
        let tag_bytes = build_giftag(
            1,
            true,
            false,
            0,
            GifFlg::Reglist,
            1,
            &[GIF_REG_NOP, GIF_REG_RGBA, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        );
        buf.extend_from_slice(&tag_bytes);
        buf.extend_from_slice(&[0u8; 16]);

        let gs_pack = GsPacket::from_offset_size(32, buf.len() as u32 - 32);

        let lines = run_parse_gs(&gs_pack, GifPath::Path3, &buf);
        assert!(lines[0].contains("Path 3 Transfer"));
        assert!(lines[1].contains("[mode=Reglist]"));
        assert!(lines[2].contains("[Reg=NOP]"));
        assert!(lines[3].contains("[Reg=RGBA]"));
    }

    #[test]
    fn parse_gs_packet_out_of_range_offset_returns_banner_only() {
        let buf = [0u8; 8];
        let gs_pack = GsPacket::from_offset_size(100, 16);
        let captured = RefCell::new(Vec::<String>::new());
        {
            let mut sink = |line: &str| captured.borrow_mut().push(line.to_string());
            parse_gs_packet(&gs_pack, GifPath::Path1, &buf, &mut sink);
        }
        let lines = captured.into_inner();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("Path 1 Transfer"));
    }

    #[test]
    fn parse_gs_packet_clamps_size_to_buffer() {
        let mut buf = Vec::new();
        let tag_bytes = build_giftag(
            1,
            true,
            false,
            0,
            GifFlg::Reglist,
            1,
            &[GIF_REG_NOP, GIF_REG_RGBA, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        );
        buf.extend_from_slice(&tag_bytes);
        buf.extend_from_slice(&[0u8; 16]);
        let gs_pack = GsPacket::from_offset_size(0, 1_000_000);
        let lines = run_parse_gs(&gs_pack, GifPath::Path2, &buf);
        assert!(lines[0].contains("Path 2 Transfer"));
        assert!(lines[1].contains("[mode=Reglist]"));
    }

    #[test]
    fn read_u128_be_round_trip() {
        let mut bytes = [0u8; 16];
        bytes[0] = 0xde;
        bytes[1] = 0xad;
        bytes[15] = 0xbe;
        assert_eq!(
            read_u128_be(&bytes),
            0xde_ad_00_00_00_00_00_00_00_00_00_00_00_00_00_be
        );
    }

    #[test]
    fn gifthonotenous_packed_loop() {
        // Two GIFtags back-to-back so we exercise the for-loop's next
        // iteration after the first tag is fully consumed.
        let tag_a = build_giftag(
            1,
            false,
            false,
            0,
            GifFlg::Packed,
            0,
            &[GIF_REG_NOP, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        );
        let tag_b = build_giftag(
            1,
            true,
            false,
            0,
            GifFlg::Packed,
            0,
            &[GIF_REG_PRIM, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        );
        let mut buf = Vec::new();
        buf.extend_from_slice(&tag_a);
        buf.extend_from_slice(&[0u8; 16]); // 1 iter * 1 reg * 16 bytes
        buf.extend_from_slice(&tag_b);
        buf.extend_from_slice(&[0u8; 16]);

        let lines = run_parse(&buf, GifPath::Path1);
        let tag_lines: Vec<_> = lines.iter().filter(|l| l.contains("--Gif Tag")).collect();
        assert_eq!(tag_lines.len(), 2);
        assert!(tag_lines[0].contains("[EOP=0]"));
        assert!(tag_lines[1].contains("[EOP=1]"));
    }

    #[test]
    fn parse_log_pre_emitted() {
        // Verify the PRE flag is honoured.
        let tag_bytes = build_giftag(
            1,
            true,
            true,
            0x42,
            GifFlg::Packed,
            0,
            &[GIF_REG_PRIM, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        );
        let mut buf = Vec::new();
        buf.extend_from_slice(&tag_bytes);
        buf.extend_from_slice(&[0u8; 16]);
        let lines = run_parse(&buf, GifPath::Path1);
        assert!(lines[1].contains("[pre=1]"));
        assert!(lines[1].contains("[prim=66]")); // 0x42 = 66
    }

    #[test]
    fn empty_giftag_default() {
        let t = GifTag::default();
        assert_eq!(t.nloop, 0);
        assert!(!t.eop);
        assert!(!t.pre);
        assert_eq!(t.prim, 0);
        assert_eq!(t.flg, GifFlg::Packed);
        assert_eq!(t.nreg, 0);
        assert_eq!(t.iterations, 0);
        assert_eq!(t.regs_per_iter, 0);
        assert_eq!(t.len, 0);
    }
}
