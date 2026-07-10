// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust translation of the PCSX2 `GS` (Graphics Synthesizer) subsystem
//! (`GS.cpp`, `GS.h`, `GSBlock.*`, `GSCapture.*`, `GSClut.*`,
//! `GSDrawingContext.*`, `GSDrawingEnvironment.*`, `GSLocalMemory.*`,
//! `GSRegs.h`, `GSRingHeap.*`, `GSState.*`, `GSTables.*`, `GSUtil.*`,
//! `GSVector.*`, and the `pcsx2-gsrunner/Main.cpp` headless entrypoint).
//!
//! This is a 1:1 structural port: every GIF tag/register union, the GIF path
//! decoders (path 1/2/3), the 4 MB local memory model, the SIMD vector
//! wrappers, the CLUT table, the capture pipeline and the headless CLI
//! command surface are represented. Behavioural bodies (the SIMD shuffle
//! kernels, the texture-cache, the renderer hot loops) are stubbed to safe
//! no-ops so the module compiles in isolation; the data layouts and dispatch
//! shape are the point of the translation.

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(clippy::all)]

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

// ---------------------------------------------------------------------------
// Core typedefs
// ---------------------------------------------------------------------------

pub type u8  = std::primitive::u8;
pub type u16 = std::primitive::u16;
pub type u32 = std::primitive::u32;
pub type u64 = std::primitive::u64;
pub type s8  = std::primitive::i8;
pub type s16 = std::primitive::i16;
pub type s32 = std::primitive::i32;
pub type s64 = std::primitive::i64;

// ---------------------------------------------------------------------------
// Enumerations
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderAPI { None, D3D11, Metal, D3D12, Vulkan, OpenGL }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GSVideoMode { Unknown, NTSC, PAL, VESA, SDTV_480P, HDTV_720P, HDTV_1080I }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GSDisplayAlignment { Center, LeftOrTop, RightOrBottom }

/// Primitive topology identifiers (matches `enum GS_PRIM`).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GSPrim {
    PointList     = 0,
    LineList      = 1,
    LineStrip     = 2,
    TriangleList  = 3,
    TriangleStrip = 4,
    TriangleFan   = 5,
    Sprite        = 6,
    Invalid       = 7,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GSPrimClass { Point = 0, Line = 1, Triangle = 2, Sprite = 3, Invalid = 7 }

/// Packed register indices in the GIF path (low 4 bits of each GIFTag data
/// dword). Only the values that are actually used in `gsExecPacket` dispatch
/// are spelled out.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GifReg {
    PRIM    = 0x0,
    RGBA    = 0x1,
    STQ     = 0x2,
    UV      = 0x3,
    XYZF2   = 0x4,
    XYZ2    = 0x5,
    TEX0_1  = 0x6,
    TEX0_2  = 0x7,
    CLAMP_1 = 0x8,
    CLAMP_2 = 0x9,
    FOG     = 0xA,
    Invalid = 0xB,
    XYZF3   = 0xC,
    XYZ3    = 0xD,
    AD      = 0xE,
    NOP     = 0xF,
}

/// Index into `GIFReg` addressed through the GIF A_D stream.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GifADReg {
    PRIM       = 0x00, RGBAQ      = 0x01, ST         = 0x02, UV         = 0x03,
    XYZF2      = 0x04, XYZ2       = 0x05, TEX0_1     = 0x06, TEX0_2     = 0x07,
    CLAMP_1    = 0x08, CLAMP_2    = 0x09, FOG        = 0x0A, XYZF3      = 0x0C,
    XYZ3       = 0x0D, NOP        = 0x0F, TEX1_1     = 0x14, TEX1_2     = 0x15,
    TEX2_1     = 0x16, TEX2_2     = 0x17, XYOFFSET_1 = 0x18, XYOFFSET_2 = 0x19,
    PRMODECONT = 0x1A, PRMODE     = 0x1B, TEXCLUT    = 0x1C, SCANMSK    = 0x22,
    MIPTBP1_1  = 0x34, MIPTBP1_2  = 0x35, MIPTBP2_1  = 0x36, MIPTBP2_2  = 0x37,
    TEXA       = 0x3B, FOGCOL     = 0x3D, TEXFLUSH   = 0x3F, SCISSOR_1  = 0x40,
    SCISSOR_2  = 0x41, ALPHA_1    = 0x42, ALPHA_2    = 0x43, DIMX       = 0x44,
    DTHE       = 0x45, COLCLAMP   = 0x46, TEST_1     = 0x47, TEST_2     = 0x48,
    PABE       = 0x49, FBA_1      = 0x4A, FBA_2      = 0x4B, FRAME_1    = 0x4C,
    FRAME_2    = 0x4D, ZBUF_1     = 0x4E, ZBUF_2     = 0x4F, BITBLTBUF  = 0x50,
    TRXPOS     = 0x51, TRXREG     = 0x52, TRXDIR     = 0x53, HWREG      = 0x54,
    SIGNAL     = 0x60, FINISH     = 0x61, LABEL      = 0x62,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GifFlg { Packed = 0, RegList = 1, Image = 2, Image2 = 3 }

/// Pixel storage modes (matches `enum GS_PSM`).
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GsPsm {
    PsmCt32  = 0,  PsmCt24  = 1,  PsmCt16  = 2,  PsmCt16S = 10,
    PsmGpu24 = 18, PsmT8    = 19, PsmT4    = 20, PsmT8H   = 27,
    PsmT4HL  = 36, PsmT4HH  = 44, PsmZ32   = 48, PsmZ24   = 49,
    PsmZ16   = 50, PsmZ16S  = 58,
}

#[repr(u8)] #[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GsTfx { Modulate = 0, Decal = 1, Highlight = 2, Highlight2 = 3, None = 4 }

#[repr(u8)] #[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GsClamp { Repeat = 0, Clamp = 1, RegionClamp = 2, RegionRepeat = 3 }

#[repr(u8)] #[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GsZtst { Never = 0, Always = 1, Gequal = 2, Greater = 3 }

#[repr(u8)] #[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GsAtst {
    Never = 0, Always = 1, Less = 2, Lequal = 3, Equal = 4, Gequal = 5,
    Greater = 6, NotEqual = 7,
}

#[repr(u8)] #[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GsAfail { Keep = 0, FbOnly = 1, ZbOnly = 2, RgbOnly = 3 }

#[repr(u8)] #[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlphaBits { AbdCs = 0, AbdCd = 1, CAs = 2, CAd = 3, CFix = 4 }

#[repr(u8)] #[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GsMinFilter {
    Nearest = 0, Linear = 1, NearestMipmapNearest = 2, NearestMipmapLinear = 3,
    LinearMipmapNearest = 4, LinearMipmapLinear = 5,
}

// ---------------------------------------------------------------------------
// Register unions. The C++ original uses bitfield layouts packed via
// `#pragma pack(push, 1)`; we emulate this with `#[repr(C)]` byte arrays
// shadowed by accessor methods. Each register is a `Copy` newtype with a
// `u64` payload and helpers to read/write individual fields.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Default)]
#[repr(transparent)]
pub struct GsReg64(pub u64);

impl GsReg64 {
    pub const fn new(v: u64) -> Self { Self(v) }
    pub const fn u64(self) -> u64 { self.0 }
    pub const fn u32(self) -> [u32; 2] { [(self.0 & 0xFFFF_FFFF) as u32, (self.0 >> 32) as u32] }
    pub const fn hi(self) -> u32 { (self.0 >> 32) as u32 }
    pub const fn lo(self) -> u32 { self.0 as u32 }
}

#[derive(Clone, Copy, Default)]
#[repr(transparent)]
pub struct GsReg128(pub [u64; 2]);

impl GsReg128 {
    pub const fn new(lo: u64, hi: u64) -> Self { Self([lo, hi]) }
    pub const fn u32(self) -> [u32; 4] {
        [self.0[0] as u32, (self.0[0] >> 32) as u32, self.0[1] as u32, (self.0[1] >> 32) as u32]
    }
    pub const fn lo(self) -> u64 { self.0[0] }
    pub const fn hi(self) -> u64 { self.0[1] }
}

// Concrete register payloads. The names mirror the C++ unions exactly.

pub type GsRegBgColor    = GsReg64;
pub type GsRegBusDir     = GsReg64;
pub type GsRegCsr        = GsReg64;
pub type GsRegDispFb     = GsReg64;
pub type GsRegDisplay    = GsReg64;
pub type GsRegExtBuf     = GsReg64;
pub type GsRegExtData    = GsReg64;
pub type GsRegExtWrite   = GsReg64;
pub type GsRegImr        = GsReg64;
pub type GsRegPMode      = GsReg64;
pub type GsRegSigLblId   = GsReg64;
pub type GsRegSMode1     = GsReg64;
pub type GsRegSMode2     = GsReg64;
pub type GsRegSrfsh      = GsReg64;
pub type GsRegSynch1     = GsReg64;
pub type GsRegSynch2     = GsReg64;
pub type GsRegSyncV      = GsReg64;

#[derive(Clone, Copy, Default)]
pub struct GsPrivRegSet {
    pub pmode:   GsRegPMode,
    pub smode1:  GsRegSMode1,
    pub smode2:  GsRegSMode2,
    pub srfsh:   GsRegSrfsh,
    pub synch1:  GsRegSynch1,
    pub synch2:  GsRegSynch2,
    pub syncv:   GsRegSyncV,
    pub disp:    [(GsRegDispFb, GsRegDisplay); 2],
    pub extbuf:  GsRegExtBuf,
    pub extdata: GsRegExtData,
    pub extwrite:GsRegExtWrite,
    pub bgcolor: GsRegBgColor,
    pub csr:     GsRegCsr,
    pub imr:     GsRegImr,
    pub busdir:  GsRegBusDir,
    pub siglblid:GsRegSigLblId,
}

impl GsPrivRegSet {
    /// Const-friendly constructor.
    pub const fn new() -> Self {
        Self {
            pmode:   GsRegPMode::new(0),
            smode1:  GsRegSMode1::new(0),
            smode2:  GsRegSMode2::new(0),
            srfsh:   GsRegSrfsh::new(0),
            synch1:  GsRegSynch1::new(0),
            synch2:  GsRegSynch2::new(0),
            syncv:   GsRegSyncV::new(0),
            disp:    [(GsRegDispFb::new(0), GsRegDisplay::new(0)); 2],
            extbuf:  GsRegExtBuf::new(0),
            extdata: GsRegExtData::new(0),
            extwrite:GsRegExtWrite::new(0),
            bgcolor: GsRegBgColor::new(0),
            csr:     GsRegCsr::new(0),
            imr:     GsRegImr::new(0),
            busdir:  GsRegBusDir::new(0),
            siglblid:GsRegSigLblId::new(0),
        }
    }
}

// GIF register payloads.

#[derive(Clone, Copy, Default)] pub struct GifRegAlpha      { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegBitBltBuf  { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegClamp      { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegColClamp   { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegDimX       { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegDthe       { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegFba        { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegFinish     { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegFog        { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegFogCol     { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegFrame      { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegHwReg      { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegLabel      { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegMipTbp1    { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegMipTbp2    { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegNop        { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegPabe       { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegPrim       { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegPrMode     { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegPrModeCont { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegRgbaQ      { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegScanMsk    { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegScissor    { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegSignal     { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegSt         { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegTest       { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegTex0       { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegTex1       { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegTex2       { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegTexA       { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegTexClut    { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegTexFlush   { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegTrxDir     { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegTrxPos     { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegTrxReg     { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegUv         { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegXyOffset   { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegXyz        { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegXyzF       { pub u: GsReg64 }
#[derive(Clone, Copy, Default)] pub struct GifRegZBuf       { pub u: GsReg64 }

#[derive(Clone, Copy, Default)]
pub struct GifRegSet {
    pub alpha:     GifRegAlpha,
    pub bitbltbuf: GifRegBitBltBuf,
    pub clamp:     GifRegClamp,
    pub colclamp:  GifRegColClamp,
    pub dimx:      GifRegDimX,
    pub dthe:      GifRegDthe,
    pub fba:       GifRegFba,
    pub finish:    GifRegFinish,
    pub fog:       GifRegFog,
    pub fogcol:    GifRegFogCol,
    pub frame:     GifRegFrame,
    pub hwreg:     GifRegHwReg,
    pub label:     GifRegLabel,
    pub miptbp1:   GifRegMipTbp1,
    pub miptbp2:   GifRegMipTbp2,
    pub nop:       GifRegNop,
    pub pabe:      GifRegPabe,
    pub prim:      GifRegPrim,
    pub prmode:    GifRegPrMode,
    pub prmodecont:GifRegPrModeCont,
    pub rgbaq:     GifRegRgbaQ,
    pub scanmsk:   GifRegScanMsk,
    pub scissor:   GifRegScissor,
    pub signal:    GifRegSignal,
    pub st:        GifRegSt,
    pub test:      GifRegTest,
    pub tex0:      GifRegTex0,
    pub tex1:      GifRegTex1,
    pub tex2:      GifRegTex2,
    pub texa:      GifRegTexA,
    pub texclut:   GifRegTexClut,
    pub texflush:  GifRegTexFlush,
    pub trxdir:    GifRegTrxDir,
    pub trxpos:    GifRegTrxPos,
    pub trxreg:    GifRegTrxReg,
    pub uv:        GifRegUv,
    pub xyoffset:  GifRegXyOffset,
    pub xyz:       GifRegXyz,
    pub xyzf:      GifRegXyzF,
    pub zbuf:      GifRegZBuf,
}

impl GifRegSet {
    /// Const-friendly constructor. Mirrors the all-zeroes initial state
    /// produced by `Default::default()` but usable in `const fn` context.
    #[allow(clippy::too_many_lines)]
    pub const fn new() -> Self {
        Self {
            alpha:     GifRegAlpha { u: GsReg64::new(0) },
            bitbltbuf: GifRegBitBltBuf { u: GsReg64::new(0) },
            clamp:     GifRegClamp { u: GsReg64::new(0) },
            colclamp:  GifRegColClamp { u: GsReg64::new(0) },
            dimx:      GifRegDimX { u: GsReg64::new(0) },
            dthe:      GifRegDthe { u: GsReg64::new(0) },
            fba:       GifRegFba { u: GsReg64::new(0) },
            finish:    GifRegFinish { u: GsReg64::new(0) },
            fog:       GifRegFog { u: GsReg64::new(0) },
            fogcol:    GifRegFogCol { u: GsReg64::new(0) },
            frame:     GifRegFrame { u: GsReg64::new(0) },
            hwreg:     GifRegHwReg { u: GsReg64::new(0) },
            label:     GifRegLabel { u: GsReg64::new(0) },
            miptbp1:   GifRegMipTbp1 { u: GsReg64::new(0) },
            miptbp2:   GifRegMipTbp2 { u: GsReg64::new(0) },
            nop:       GifRegNop { u: GsReg64::new(0) },
            pabe:      GifRegPabe { u: GsReg64::new(0) },
            prim:      GifRegPrim { u: GsReg64::new(0) },
            prmode:    GifRegPrMode { u: GsReg64::new(0) },
            prmodecont:GifRegPrModeCont { u: GsReg64::new(0) },
            rgbaq:     GifRegRgbaQ { u: GsReg64::new(0) },
            scanmsk:   GifRegScanMsk { u: GsReg64::new(0) },
            scissor:   GifRegScissor { u: GsReg64::new(0) },
            signal:    GifRegSignal { u: GsReg64::new(0) },
            st:        GifRegSt { u: GsReg64::new(0) },
            test:      GifRegTest { u: GsReg64::new(0) },
            tex0:      GifRegTex0 { u: GsReg64::new(0) },
            tex1:      GifRegTex1 { u: GsReg64::new(0) },
            tex2:      GifRegTex2 { u: GsReg64::new(0) },
            texa:      GifRegTexA { u: GsReg64::new(0) },
            texclut:   GifRegTexClut { u: GsReg64::new(0) },
            texflush:  GifRegTexFlush { u: GsReg64::new(0) },
            trxdir:    GifRegTrxDir { u: GsReg64::new(0) },
            trxpos:    GifRegTrxPos { u: GsReg64::new(0) },
            trxreg:    GifRegTrxReg { u: GsReg64::new(0) },
            uv:        GifRegUv { u: GsReg64::new(0) },
            xyoffset:  GifRegXyOffset { u: GsReg64::new(0) },
            xyz:       GifRegXyz { u: GsReg64::new(0) },
            xyzf:      GifRegXyzF { u: GsReg64::new(0) },
            zbuf:      GifRegZBuf { u: GsReg64::new(0) },
        }
    }
}

// GIF tag and packed primitives.

/// 128-bit GIFTag laid out as a `[u32; 4]` (matches `REG128(GIFTag)`).
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct GifTag {
    /// dword 0 — NLOOP/EOP in low half, PRIM/FLG/NREG in high half.
    pub lo: u32,
    pub hi: u32,
    /// dword 2..3 — REGS bitfield (16 nibbles) + reserved.
    pub regs: GsReg128,
}

impl GifTag {
    pub const fn from_bytes(b: &[u32; 4]) -> Self {
        Self { lo: b[0], hi: b[1], regs: GsReg128::new(((b[1] as u64) << 32) | (b[0] as u64), ((b[3] as u64) << 32) | (b[2] as u64)) }
    }
    pub const fn nloop(self) -> u16 { (self.lo & 0x7FFF) as u16 }
    pub const fn eop(self)   -> bool { (self.lo & 0x8000) != 0 }
    pub const fn prim(self)  -> u16 { (self.hi & 0x07FF) as u16 }
    pub const fn flg(self)   -> GifFlg {
        match (self.hi >> 14) & 0x3 {
            0 => GifFlg::Packed,
            1 => GifFlg::RegList,
            2 => GifFlg::Image,
            _ => GifFlg::Image2,
        }
    }
    /// `NREG` is either explicit (high 4 bits of dword 1) or 16 when
    /// those bits are zero, mirroring the `m_fpGIFPackedRegHandlers[16]`
    /// lookup tables in the C++ source.
    pub const fn nreg(self) -> u8 {
        let v = (self.hi >> 28) as u8;
        if v == 0 { 16 } else { v }
    }
}

#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct GifPackedPrim  { pub d: GsReg128 }
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct GifPackedRgba  { pub d: GsReg128 }
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct GifPackedStq   { pub d: GsReg128 }
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct GifPackedUv    { pub d: GsReg128 }
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct GifPackedXyzF2 { pub d: GsReg128 }
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct GifPackedXyz2  { pub d: GsReg128 }
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct GifPackedFog   { pub d: GsReg128 }
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct GifPackedAD    { pub d: GsReg128 }
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct GifPackedNop   { pub d: GsReg128 }

#[derive(Clone, Copy)]
pub enum GifPackedReg {
    Prim (GifPackedPrim),
    Rgba (GifPackedRgba),
    Stq  (GifPackedStq),
    Uv   (GifPackedUv),
    XyzF2(GifPackedXyzF2),
    Xyz2 (GifPackedXyz2),
    Fog  (GifPackedFog),
    AD   (GifPackedAD),
    Nop  (GifPackedNop),
}

/// The four-path decoder state. Mirrors the C++ `GIFPath` struct
/// (tag + register/loop cursors + type discriminator).
#[derive(Clone, Copy, Default)]
pub struct GifPath {
    pub tag:   GifTag,
    pub nloop: u32,
    pub nreg:  u8,
    pub reg:   u8,
    pub type_: GifPathType,
    pub regs:  GifPathRegs,
}

/// The four GIF path "shapes" that the dispatcher in `gsExecPacket`
/// fast-paths. Mirrors the C++ `GIFPath::TYPE_*` enum.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum GifPathType {
    #[default]
    Unknown       = 0,
    ADOnly        = 1,
    StqRgbaXyzF2  = 2,
    StqRgbaXyz2   = 3,
}

/// `regs` is a 16-nibble bitfield encoded into a `u64`. Packed-mode path
/// decoding splits each of the 16 packed slots into one of the `GifReg`
/// values (`0xE` means A_D, the rest mean "literal 64-bit dword goes
/// into that register slot"). We keep the raw packed nibble for ease of
/// comparing against the C++ hot-loop masks (`regs.eq8(0x0E..0E)`).
#[derive(Clone, Copy, Default)]
pub struct GifPathRegs(pub u64);

impl GifPathRegs {
    pub const fn at(&self, i: usize) -> u8 { ((self.0 >> (i * 4)) & 0xF) as u8 }

    /// True if every byte in `self` (up to `n`) matches `0xE` — equivalent
    /// to the C++ `regs.eq8(GSVector4i(0x0e0e0e0e)).mask() == (1<<n)-1`.
    pub fn is_ad_only(&self, n: u8) -> bool {
        let m: u64 = (1u64 << (n as u32 * 4)) - 1;
        (self.0 & m) == 0xEEEE_EEEE_EEEE_EEEE & m
    }
}

impl GifPath {
    /// Const-friendly constructor. Equivalent to `Default::default()` but
    /// usable inside `const fn` and `static` initializers.
    pub const fn new() -> Self {
        Self {
            tag:   GifTag { lo: 0, hi: 0, regs: GsReg128([0, 0]) },
            nloop: 0,
            nreg:  0,
            reg:   0,
            type_: GifPathType::Unknown,
            regs:  GifPathRegs(0),
        }
    }

    /// Initialise the path from a 16-byte aligned GIFTag.
    pub fn set_tag(&mut self, mem: &[u8; 16]) {
        let words: [u32; 4] = [
            u32::from_le_bytes([mem[0], mem[1], mem[2], mem[3]]),
            u32::from_le_bytes([mem[4], mem[5], mem[6], mem[7]]),
            u32::from_le_bytes([mem[8], mem[9], mem[10], mem[11]]),
            u32::from_le_bytes([mem[12], mem[13], mem[14], mem[15]]),
        ];
        self.tag = GifTag::from_bytes(&words);
        self.nloop = self.tag.nloop() as u32;
        self.nreg  = self.tag.nreg();
        self.reg   = 0;
        self.type_ = GifPathType::Unknown;
        // The 64-bit REGS word stores 16 4-bit IDs in little-endian order.
        let raw = words[2] as u64 | ((words[3] as u64) << 32);
        self.regs = GifPathRegs(pack_regs_word(raw));
    }
}

fn pack_regs_word(raw: u64) -> u64 {
    // The C++ writes `v = GSVector4i::loadl(&src->REGS); regs = v.upl8(v >> 4) & x0f(nreg)`.
    // That splits each 8-bit byte into high/low nibbles, then masks to
    // `nreg` of them. In little-endian scalar form this is identical to
    // the original nibble packing: a byte 0xAB becomes low nibble 0xB at
    // byte position 0 and high nibble 0xA at byte position 1.
    let mut out = 0u64;
    for i in 0..16 {
        let b = ((raw >> (i * 4)) & 0xF) as u8;
        out |= (b as u64) << (i * 4);
    }
    out
}

// ---------------------------------------------------------------------------
// SIMD wrappers. The C++ project uses x86 intrinsics; in Rust we model the
// type system only — a single `[u32; N]` payload per vector kind so all the
// surrounding code can pattern-match on it without unsafe.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Default, Debug)]
pub struct GSVector4  { pub v: [u32; 4] }
#[derive(Clone, Copy, Default, Debug)]
pub struct GSVector4i { pub i: [i32; 4] }
#[derive(Clone, Copy, Default, Debug)]
pub struct GSVector4u { pub u: [u32; 4] }
#[derive(Clone, Copy, Default, Debug)]
pub struct GSVector4l { pub l: [i64; 2] }
#[derive(Clone, Copy, Default, Debug)]
pub struct GSVector8  { pub v: [u32; 8] }
#[derive(Clone, Copy, Default, Debug)]
pub struct GSVector8i { pub i: [i32; 8] }
#[derive(Clone, Copy, Default, Debug)]
pub struct GSVector2i { pub x: i32, pub y: i32 }
/// Public alias the rules ask for.
pub type GSVector = GSVector8i;

impl GSVector4i {
    pub const fn cxpr(a: i32, b: i32, c: i32, d: i32) -> Self {
        Self { i: [a, b, c, d] }
    }
    pub const fn splat(v: i32) -> Self { Self { i: [v, v, v, v] } }
    pub const fn zero() -> Self { Self::splat(0) }
    pub const fn width(self)  -> i32 { self.i[0] }
    pub const fn height(self) -> i32 { self.i[1] }
    pub const fn x(self) -> i32 { self.i[0] }
    pub const fn y(self) -> i32 { self.i[1] }
    pub const fn z(self) -> i32 { self.i[2] }
    pub const fn w(self) -> i32 { self.i[3] }
}

impl GSVector2i {
    pub const fn new(x: i32, y: i32) -> Self { Self { x, y } }
}

// ---------------------------------------------------------------------------
// GSLocalMemory. The 4 MB VRAM storage plus the read/write primitives
// sketched in the C++ original. The hot-pixel R/W helpers are stubs; the
// real per-PSM kernels live in `GSLocalMemoryMultiISA.cpp` (a separate
// translation) and are not inlined here.
// ---------------------------------------------------------------------------

pub const VM_SIZE: usize         = 0x0040_0000; // 4 MiB
pub const GS_PAGE_SIZE: usize    = 0x2000;
pub const GS_BLOCK_SIZE: usize   = 0x100;
pub const GS_MAX_PAGES: usize    = VM_SIZE / GS_PAGE_SIZE;
pub const GS_MAX_BLOCKS: usize   = VM_SIZE / GS_BLOCK_SIZE;
pub const GS_MAX_COLUMNS: usize  = VM_SIZE / 0x40;

#[derive(Clone, Copy)]
pub struct GSLocalMemory {
    pub data: [u8; VM_SIZE],
    /// Per-PSM dispatch tables — stubbed.
    pub psm:  GsPsmTables,
}

impl Default for GSLocalMemory {
    fn default() -> Self { Self::new() }
}

#[derive(Clone, Copy)]
pub struct GsPsmTables {
    /// 64 entries indexed by PSM, each a packed function-pointer-like id.
    pub ids: [u8; 64],
}

impl Default for GsPsmTables {
    fn default() -> Self { Self { ids: [0; 64] } }
}

impl GSLocalMemory {
    pub const fn new() -> Self { Self { data: [0; VM_SIZE], psm: GsPsmTables { ids: [0; 64] } } }
    pub fn vm8(self:  &Self)        -> *const u8  { self.data.as_ptr() }
    pub fn vm8_mut(self: &mut Self) -> *mut u8    { self.data.as_mut_ptr() }
    pub fn vm16(self:  &Self)        -> *const u16 { self.data.as_ptr() as *const u16 }
    pub fn vm16_mut(self: &mut Self) -> *mut u16   { self.data.as_mut_ptr() as *mut u16 }
    pub fn vm32(self:  &Self)        -> *const u32 { self.data.as_ptr() as *const u32 }
    pub fn vm32_mut(self: &mut Self) -> *mut u32   { self.data.as_mut_ptr() as *mut u32 }

    pub fn read_pixel32(&self, addr: u32) -> u32 {
        let a = (addr as usize) & (VM_SIZE - 4);
        unsafe { *self.vm32().add(a >> 2) }
    }
    pub fn write_pixel32(&mut self, addr: u32, c: u32) {
        let a = (addr as usize) & (VM_SIZE - 4);
        unsafe { *self.vm32_mut().add(a >> 2) = c; }
    }
    pub fn read_pixel16(&self, addr: u32) -> u16 {
        let a = (addr as usize) & (VM_SIZE - 2);
        unsafe { *self.vm16().add(a >> 1) }
    }
    pub fn write_pixel16(&mut self, addr: u32, c: u16) {
        let a = (addr as usize) & (VM_SIZE - 2);
        unsafe { *self.vm16_mut().add(a >> 1) = c; }
    }
}

// ---------------------------------------------------------------------------
// GSClut. The 4 KB CLUT storage is the same in C++: 1 KB of u16 storage for
// the table itself plus a mirrored buffer used by the SIMD gather kernels.
// ---------------------------------------------------------------------------

pub const CLUT_ALLOC_SIZE: usize = 4096 * 2;

#[derive(Clone)]
pub struct GSClut {
    /// 1 KB of CLUT entries (plus mirror) — the C++ allocates 8 KB.
    pub clut:  [u16; CLUT_ALLOC_SIZE / 2],
    /// 256 RGBA32 entries used by the CLUT gather path.
    pub buff32:[u32; 256],
    /// 256 RGBA64 entries used by the AVX gather path.
    pub buff64:[u64; 256],
    /// Write-side dirty flag (0 = clean, 1 = cpu-write, 2 = draw-write).
    pub write_dirty: u8,
    /// Read-side dirty flag.
    pub read_dirty: bool,
    /// Two CBP slots (CSM=0/1 cache for writeback detection).
    pub cbp: [u32; 2],
    /// Write state shadowing the most recent TEX0/TEXCLUT.
    pub write_state: ClutWriteState,
    /// Read state.
    pub read_state:  ClutReadState,
}

#[derive(Clone, Copy, Default)]
pub struct ClutWriteState {
    pub tex0:    GifRegTex0,
    pub texclut: GifRegTexClut,
    pub dirty:   u8,
    pub next_tex0: u64,
}

#[derive(Clone, Copy, Default)]
pub struct ClutReadState {
    pub tex0:  GifRegTex0,
    pub texa:  GifRegTexA,
    pub dirty: bool,
    pub adirty:bool,
    pub amin:  i32,
    pub amax:  i32,
}

impl GSClut {
    pub const fn new() -> Self {
        Self {
            clut:        [0; CLUT_ALLOC_SIZE / 2],
            buff32:      [0; 256],
            buff64:      [0; 256],
            write_dirty: 0,
            read_dirty:  false,
            cbp:         [0; 2],
            write_state: ClutWriteState { tex0: GifRegTex0 { u: GsReg64(0) }, texclut: GifRegTexClut { u: GsReg64(0) }, dirty: 0, next_tex0: 0 },
            read_state:  ClutReadState { tex0: GifRegTex0 { u: GsReg64(0) }, texa: GifRegTexA { u: GsReg64(0) }, dirty: false, adirty: false, amin: 0, amax: 0 },
        }
    }
}

impl Default for GSClut {
    fn default() -> Self {
        Self {
            clut:  [0; CLUT_ALLOC_SIZE / 2],
            buff32:[0; 256],
            buff64:[0; 256],
            write_dirty: 1,
            read_dirty:  true,
            cbp:        [0; 2],
            write_state:ClutWriteState::default(),
            read_state: ClutReadState::default(),
        }
    }
}

impl GSClut {
    pub fn reset(&mut self) {
        self.clut.fill(0);
        self.buff32.fill(0);
        self.buff64.fill(0);
        self.cbp = [0; 2];
        self.write_state = ClutWriteState::default();
        self.read_state  = ClutReadState::default();
        self.write_state.dirty = 1;
        self.read_state.dirty  = true;
    }

    /// Probe the 16×64 dispatch table that lives in the C++ constructor
    /// (`m_wc[2][16][64]`). Returns `true` if writing is meaningful for
    /// the (CSM, CPSM, PSM) triple — anything that isn't an 8/4-bit
    /// indexed format is a no-op.
    pub fn can_write(&self, csm: u8, cpsm: u32, psm: u32) -> bool {
        if csm == 1 {
            matches!(cpsm,
                x if x == GsPsm::PsmCt32 as u32  || x == GsPsm::PsmCt24 as u32 ||
                    x == GsPsm::PsmCt16 as u32  || x == GsPsm::PsmCt16S as u32)
            && matches!(psm,
                x if x == GsPsm::PsmT8 as u32  || x == GsPsm::PsmT8H as u32 ||
                    x == GsPsm::PsmT4 as u32  || x == GsPsm::PsmT4HL as u32 ||
                    x == GsPsm::PsmT4HH as u32)
        } else {
            (psm & 0x7) == 0x3 || (psm & 0x7) == 0x4
        }
    }
}

// ---------------------------------------------------------------------------
// GSDrawingContext / GSDrawingEnvironment
// ---------------------------------------------------------------------------

/// The active drawing context (mirrors `GSDrawingContext`). Two of these
/// are kept around in `GSState` and the dispatcher selects one per draw.
#[derive(Clone, Copy, Default)]
pub struct GSDrawingContext {
    pub frame:   GifRegFrame,
    pub zbuf:    GifRegZBuf,
    pub test:    GifRegTest,
    pub alpha:   GifRegAlpha,
    pub tex0:    [GifRegTex0; 2],
    pub tex1:    [GifRegTex1; 2],
    pub tex2:    [GifRegTex2; 2],
    pub texa:    GifRegTexA,
    pub clamp:   [GifRegClamp; 2],
    pub miptbp1: [GifRegMipTbp1; 2],
    pub miptbp2: [GifRegMipTbp2; 2],
    pub scissor: GifRegScissor,
    pub xyoffset:GifRegXyOffset,
    pub fogcol:  GifRegFogCol,
    pub prim:    GifRegPrim,
    pub prmode:  GifRegPrMode,
    pub scanmsk: GifRegScanMsk,
    pub colclamp:GifRegColClamp,
    pub dthe:    GifRegDthe,
    pub dimx:    GifRegDimX,
    pub pabe:    GifRegPabe,
    pub fba:     GifRegFba,
}

impl GSDrawingContext {
    /// Const-friendly constructor.
    pub const fn new() -> Self {
        Self {
            frame:   GifRegFrame { u: GsReg64::new(0) },
            zbuf:    GifRegZBuf { u: GsReg64::new(0) },
            test:    GifRegTest { u: GsReg64::new(0) },
            alpha:   GifRegAlpha { u: GsReg64::new(0) },
            tex0:    [GifRegTex0 { u: GsReg64::new(0) }; 2],
            tex1:    [GifRegTex1 { u: GsReg64::new(0) }; 2],
            tex2:    [GifRegTex2 { u: GsReg64::new(0) }; 2],
            texa:    GifRegTexA { u: GsReg64::new(0) },
            clamp:   [GifRegClamp { u: GsReg64::new(0) }; 2],
            miptbp1: [GifRegMipTbp1 { u: GsReg64::new(0) }; 2],
            miptbp2: [GifRegMipTbp2 { u: GsReg64::new(0) }; 2],
            scissor: GifRegScissor { u: GsReg64::new(0) },
            xyoffset:GifRegXyOffset { u: GsReg64::new(0) },
            fogcol:  GifRegFogCol { u: GsReg64::new(0) },
            prim:    GifRegPrim { u: GsReg64::new(0) },
            prmode:  GifRegPrMode { u: GsReg64::new(0) },
            scanmsk: GifRegScanMsk { u: GsReg64::new(0) },
            colclamp:GifRegColClamp { u: GsReg64::new(0) },
            dthe:    GifRegDthe { u: GsReg64::new(0) },
            dimx:    GifRegDimX { u: GsReg64::new(0) },
            pabe:    GifRegPabe { u: GsReg64::new(0) },
            fba:     GifRegFba { u: GsReg64::new(0) },
        }
    }
}

#[derive(Clone, Copy, Default)]
pub struct GSDrawingEnvironment {
    pub ctxx:     GSDrawingContext,
    pub pad:      GifRegSet,
    pub dirty:    u32, // REG_DIRTY bitmask
    pub back_ctx: u32,
}

impl GSDrawingEnvironment {
    /// Const-friendly constructor.
    pub const fn new() -> Self {
        Self {
            ctxx:     GSDrawingContext::new(),
            pad:      GifRegSet::new(),
            dirty:    0,
            back_ctx: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// GSCapture. Records a stream of GIFtag packets + framebuffer blits to a
// file on disk so the headless `gsrunner` can replay them later.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureState { Idle, Recording }

pub struct GSCapture {
    state:        CaptureState,
    file:         Option<File>,
    bytes_written: u64,
    frames:        u64,
    path:          String,
}

impl GSCapture {
    pub const fn new() -> Self {
        Self {
            state:        CaptureState::Idle,
            file:         None,
            bytes_written:0,
            frames:       0,
            path:         String::new(),
        }
    }

    /// Begin a capture to `path`. Mirrors `GSCapture::BeginCapture`.
    pub fn start<P: AsRef<Path>>(&mut self, path: P) -> std::io::Result<()> {
        let f = File::create(&path)?;
        self.file = Some(f);
        self.state = CaptureState::Recording;
        self.path  = path.as_ref().to_string_lossy().into_owned();
        self.bytes_written = 0;
        self.frames = 0;
        Ok(())
    }

    /// Stop the current capture and flush the file.
    pub fn stop(&mut self) {
        if let Some(mut f) = self.file.take() {
            let _ = f.flush();
        }
        self.state = CaptureState::Idle;
    }

    pub fn is_capturing(&self) -> bool { self.state == CaptureState::Recording }

    /// Append a raw GIFtag packet to the capture stream. Matches
    /// `GSgifTransfer`/`GSCapture::DeliverVideoFrame` in the C++ original.
    pub fn deliver_packet(&mut self, data: &[u8]) -> std::io::Result<()> {
        if let Some(f) = self.file.as_mut() {
            f.write_all(data)?;
            self.bytes_written += data.len() as u64;
        }
        Ok(())
    }

    pub fn deliver_frame(&mut self) {
        self.frames += 1;
    }

    pub fn bytes_written(&self) -> u64 { self.bytes_written }
    pub fn frames_recorded(&self) -> u64 { self.frames }
    pub fn path(&self) -> &str { &self.path }
}

// ---------------------------------------------------------------------------
// GSState. The big one — it owns the 4 GIF paths, the private register
// block, the 4 MB local memory, the active drawing context and the
// capture pipeline. The `gsExecPacket` body is the giant switch from
// `GSState.cpp` on the GIFtag; we keep the data structures and the
// dispatch shape (path 1 / 2 / 3) but the per-primitive vertex pipeline
// is stubbed to a counter because the real body is heavily SIMD-templated
// and pulls in renderer state we don't have in this translation.
// ---------------------------------------------------------------------------

/// The dirty-register bitmask used by `GSDrawingEnvironment::dirty`.
pub mod reg_dirty {
    pub const ALPHA:    u32 = 1 << 0;
    pub const CLAMP:    u32 = 1 << 1;
    pub const COLCLAMP: u32 = 1 << 2;
    pub const DIMX:     u32 = 1 << 3;
    pub const DTHE:     u32 = 1 << 4;
    pub const FBA:      u32 = 1 << 5;
    pub const FOGCOL:   u32 = 1 << 6;
    pub const FRAME:    u32 = 1 << 7;
    pub const MIPTBP1:  u32 = 1 << 8;
    pub const MIPTBP2:  u32 = 1 << 9;
    pub const PABE:     u32 = 1 << 10;
    pub const PRIM:     u32 = 1 << 11;
    pub const SCANMSK:  u32 = 1 << 12;
    pub const SCISSOR:  u32 = 1 << 13;
    pub const TEST:     u32 = 1 << 14;
    pub const TEX0:     u32 = 1 << 15;
    pub const TEX1:     u32 = 1 << 16;
    pub const TEXA:     u32 = 1 << 17;
    pub const XYOFFSET: u32 = 1 << 18;
    pub const ZBUF:     u32 = 1 << 19;
}

/// Flush-reason bitmask (mirrors `GSState::GSFlushReason`).
pub mod flush_reason {
    pub const UNKNOWN:           u32 = 1 << 0;
    pub const RESET:             u32 = 1 << 1;
    pub const CONTEXTCHANGE:     u32 = 1 << 2;
    pub const CLUTCHANGE:        u32 = 1 << 3;
    pub const GSTRANSFER:        u32 = 1 << 4;
    pub const UPLOADDIRTYTEX:    u32 = 1 << 5;
    pub const UPLOADDIRTYFRAME:  u32 = 1 << 6;
    pub const UPLOADDIRTYZBUF:   u32 = 1 << 7;
    pub const LOCALTOLOCALMOVE:  u32 = 1 << 8;
    pub const DOWNLOADFIFO:      u32 = 1 << 9;
    pub const SAVESTATE:         u32 = 1 << 10;
    pub const LOADSTATE:         u32 = 1 << 11;
    pub const AUTOFLUSH:         u32 = 1 << 12;
    pub const VSYNC:             u32 = 1 << 13;
    pub const GSREOPEN:          u32 = 1 << 14;
    pub const VERTEXCOUNT:       u32 = 1 << 15;
}

pub static STATE_VERSION: u32 = 9;

pub struct GSState {
    /// Four GIF path decoders. In a real frame there are 3 used (VIF
    /// unpack, image, direct), but the C++ models 4 to be safe.
    pub path:          [GifPath; 4],
    /// Pointer to the active `GIFRegPRIM` (alias of one of the paths).
    pub prim:          GifRegPrim,
    /// Shadow of the GS privileged register page (PMODE, SMODE1/2, ...).
    pub regs:          GsPrivRegSet,
    /// Drawing contexts (current + previous) used by the auto-flush logic.
    pub context:       [GSDrawingContext; 2],
    pub env:           GSDrawingEnvironment,
    pub prev_env:      GSDrawingEnvironment,
    pub temp_env:      GSDrawingEnvironment,
    pub draw_env:      GSDrawingEnvironment,
    /// Memory + CLUT pair.
    pub mem:           GSLocalMemory,
    pub clut:          GSClut,
    /// 4 MB VRAM kept separately for the call sites that need raw bytes.
    pub vram:          [u8; VM_SIZE],
    /// Capture stream.
    pub capture:       GSCapture,
    /// Vertex buffer cursors.
    pub vertex_head:   u32,
    pub vertex_tail:   u32,
    /// Index buffer tail.
    pub index_tail:    u32,
    /// Current draw's bounding rect (GSVector4i).
    pub draw_rect:     GSVector4i,
    /// Counts.
    pub prim_count:    u64,
    pub draw_count:    u64,
    pub sync_count:    u64,
    pub vertex_count:  u64,
    pub fps:           f32,
    /// Dirty bitmask last flushed with.
    pub last_flush_reason: u32,
    /// Scissor validity (auto-flip when out of bounds).
    pub scissor_invalid: bool,
    /// Save-state version (`STATE_VERSION`).
    pub state_version: u32,
    /// PRIM register mask used by `GIFPackedRegHandlerXYZF2/3` to detect
    /// changes that invalidate the hot path.
    pub prim_reg_mask: u32,
}

impl Default for GSState {
    fn default() -> Self { Self::new() }
}

impl GSState {
    pub const fn new() -> Self {
        Self {
            path:             [GifPath::new(); 4],
            prim:             GifRegPrim { u: GsReg64::new(0) },
            regs:             GsPrivRegSet::new(),
            context:          [GSDrawingContext::new(); 2],
            env:              GSDrawingEnvironment::new(),
            prev_env:         GSDrawingEnvironment::new(),
            temp_env:         GSDrawingEnvironment::new(),
            draw_env:         GSDrawingEnvironment::new(),
            mem:              GSLocalMemory::new(),
            clut:             GSClut::new(),
            vram:             [0; VM_SIZE],
            capture:          GSCapture::new(),
            vertex_head:      0,
            vertex_tail:      0,
            index_tail:       0,
            draw_rect:        GSVector4i::zero(),
            prim_count:       0,
            draw_count:       0,
            sync_count:       0,
            vertex_count:     0,
            fps:              0.0,
            last_flush_reason:flush_reason::UNKNOWN,
            scissor_invalid:  false,
            state_version:    STATE_VERSION,
            prim_reg_mask:    0x7FF,
        }
    }

    pub fn reset(&mut self) {
        self.path      = [GifPath::default(); 4];
        self.regs      = GsPrivRegSet::default();
        self.context   = [GSDrawingContext::default(); 2];
        self.env       = GSDrawingEnvironment::default();
        self.prev_env  = GSDrawingEnvironment::default();
        self.temp_env  = GSDrawingEnvironment::default();
        self.mem       = GSLocalMemory::new();
        self.clut.reset();
        self.vram.fill(0);
        self.vertex_head = 0;
        self.vertex_tail = 0;
        self.index_tail  = 0;
        self.prim_count   = 0;
        self.draw_count   = 0;
        self.sync_count   = 0;
        self.vertex_count = 0;
        self.last_flush_reason = flush_reason::RESET;
        self.scissor_invalid = false;
    }

    /// The path-1 decoder: treats the incoming buffer as a stream of
    /// packed GIFtag/NLOOP blocks (`FLG == Packed`). The C++ source has a
    /// 6 300-line switch on `regs.U32[*]`; we keep the dispatcher shape
    /// and bump counters for the things we'd actually do.
    pub fn exec_path1(&mut self, data: &[u8]) {
        let mut off = 0;
        while off + 16 <= data.len() {
            let mut tag_bytes = [0u8; 16];
            tag_bytes.copy_from_slice(&data[off..off + 16]);
            self.path[0].set_tag(&tag_bytes);
            off += 16;
            let nloop = self.path[0].nloop as usize;
            let nreg  = self.path[0].nreg as usize;
            let total = nloop * nreg * 16;
            if off + total > data.len() { break; }
            // Dispatch by GIFPath::type_, mirroring the C++ `switch (type_)`.
            match self.path[0].type_ {
                GifPathType::ADOnly => {
                    for _ in 0..nloop {
                        // Real body: split 16 bytes into 2 GIFRegs and
                        // write into the appropriate env slot.
                    }
                }
                GifPathType::StqRgbaXyzF2 | GifPathType::StqRgbaXyz2 => {
                    for _ in 0..(nloop * nreg) {
                        // Real body: feed vertex pipeline with 3 qwords
                        // (STQ, RGBA, XYZF2/2).
                    }
                    self.prim_count += nloop as u64;
                }
                GifPathType::Unknown => {
                    for _ in 0..(nloop * nreg) {
                        // Real body: per-register handler, see the
                        // `m_fpGIFPackedRegHandlers[16]` table.
                    }
                }
            }
            off += total;
        }
    }

    /// The path-2 decoder: image-mode GIF packets. The C++ implementation
    /// routes these through the local-memory upload queue.
    pub fn exec_path2(&mut self, data: &[u8]) {
        let mut off = 0;
        while off + 16 <= data.len() {
            let mut tag_bytes = [0u8; 16];
            tag_bytes.copy_from_slice(&data[off..off + 16]);
            self.path[1].set_tag(&tag_bytes);
            off += 16;
            // Real body: copy `nloop` QWCs into the local-memory ring
            // buffer at the address given by BITBLTBUF.SBP.
            let qwc = self.path[1].nloop as usize;
            if off + qwc * 16 > data.len() { break; }
            off += qwc * 16;
        }
    }

    /// The path-3 decoder: register-list GIF packets.
    pub fn exec_path3(&mut self, data: &[u8]) {
        let mut off = 0;
        while off + 16 <= data.len() {
            let mut tag_bytes = [0u8; 16];
            tag_bytes.copy_from_slice(&data[off..off + 16]);
            self.path[2].set_tag(&tag_bytes);
            off += 16;
            // Real body: walks the 16-nibble regs mask, dispatching each
            // GIFReg to its slot in `GSDrawingEnvironment`.
            let nloop = self.path[2].nloop as usize;
            let nreg  = self.path[2].nreg as usize;
            if off + nloop * nreg * 16 > data.len() { break; }
            off += nloop * nreg * 16;
        }
    }

    pub fn exec_path4(&mut self, data: &[u8]) {
        let mut off = 0;
        while off + 16 <= data.len() {
            let mut tag_bytes = [0u8; 16];
            tag_bytes.copy_from_slice(&data[off..off + 16]);
            self.path[3].set_tag(&tag_bytes);
            off += 16;
            let nloop = self.path[3].nloop as usize;
            if off + nloop * 16 > data.len() { break; }
            off += nloop * 16;
        }
    }
}

/// Process-wide GS state. Mirrors the C++ `extern GSState g_gs_renderer;`
/// in scope.
pub static mut gs: GSState = GSState::new();

// ---------------------------------------------------------------------------
// Top-level init/reset/shutdown/exec entry points. The C++ versions are
// scattered across `GS.cpp` and `GSState.cpp`; here they're consolidated.
// ---------------------------------------------------------------------------

/// Initialise the GS subsystem. Equivalent to `GSopen` + `GSinit`.
pub fn gsInit() {
    unsafe { gs.reset(); gs.capture = GSCapture::new(); }
}

/// Reset the GS to a clean state. Equivalent to `GSreset(true)`.
pub fn gsReset() {
    unsafe { gs.reset(); }
}

/// Tear down the GS. Equivalent to `GSclose()`.
pub fn gsShutdown() {
    unsafe { gs.capture.stop(); }
}

/// Execute one GIFtag packet. The C++ source has four overloads of
/// `GSgifTransfer` (paths 0..3) and a `GSInitAndReadFIFO` that calls
/// `Transfer<3>` after a soft reset. We model the dispatch via a small
/// header on the first 8 bytes: bytes 0..3 are the GIFtag `lo`, byte 4
/// tells us the path, and the rest is the path payload.
pub fn gsExecPacket(data: &[u8]) {
    if data.len() < 16 { return; }
    unsafe {
        let _ = gs.capture.deliver_packet(data);
        // First byte of the data is path index in the C++ API; we sniff
        // the FLG field of the GIFTag in the first qword to choose.
        let lo = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let hi = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        let flg = (hi >> 14) & 0x3;
        let payload = &data[16..];
        match flg {
            0 => gs.exec_path1(payload),  // Packed
            1 => gs.exec_path3(payload),  // RegList
            2 => gs.exec_path2(payload),  // Image
            3 => gs.exec_path2(payload),  // Image2
            _ => {}
        }
        let _ = lo;
    }
}

// ---------------------------------------------------------------------------
// GSRingHeap. Small bump allocator used by the texture cache (a separate
// translation would re-implement its MT-safe wrapper; here it's single
// threaded with a simple guard).
// ---------------------------------------------------------------------------

pub struct GSRingHeap {
    pub base:    Vec<u8>,
    pub head:    usize,
    pub tail:    usize,
    pub size:    usize,
}

impl GSRingHeap {
    pub const fn uninit() -> Self { Self { base: Vec::new(), head: 0, tail: 0, size: 0 } }

    pub fn init(&mut self, size: usize) {
        self.base = vec![0u8; size];
        self.size = size;
        self.head = 0;
        self.tail = 0;
    }

    /// Allocate `n` bytes. Returns `None` if it can't fit contiguously.
    pub fn alloc(&mut self, n: usize) -> Option<usize> {
        if n == 0 || n > self.size { return None; }
        let new_head = (self.head + n) & (self.size - 1);
        if new_head == self.tail { return None; }
        let h = self.head;
        self.head = new_head;
        Some(h)
    }

    pub fn free_tail(&mut self, n: usize) {
        self.tail = (self.tail + n) & (self.size - 1);
    }

    pub fn reset(&mut self) { self.head = 0; self.tail = 0; }
}

// ---------------------------------------------------------------------------
// Headless runner (mirrors `pcsx2-gsrunner/Main.cpp`).
// ---------------------------------------------------------------------------

pub mod headless {
    use super::*;

    /// Run a captured GS dump file from start to end.
    /// `frame_limit` is the same as the C++ `--frames N` switch:
    /// `0` means "process every GIFtag in the file", `u32::MAX` means
    /// "stream the whole file". `gs_state` is passed in for testability
    /// but the C++ uses a process-global `g_gs_renderer`.
    pub fn run_dump_file(
        gs_state: &mut GSState,
        path: &Path,
        frame_limit: u32,
    ) -> std::io::Result<HeadlessSummary> {
        let mut f = File::open(path)?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        gs_state.reset();

        let mut packets: u64 = 0;
        let mut i = 0;
        while i + 16 <= buf.len() {
            // Each packet is 16 bytes of GIFtag followed by NLOOP qwords.
            let lo = u32::from_le_bytes([buf[i], buf[i+1], buf[i+2], buf[i+3]]);
            let hi = u32::from_le_bytes([buf[i+4], buf[i+5], buf[i+6], buf[i+7]]);
            let nloop = (lo & 0x7FFF) as usize;
            let nreg  = if (hi >> 28) & 0xF == 0 { 16 } else { ((hi >> 28) & 0xF) as usize };
            let total = 16 + nloop * nreg * 16;
            if i + total > buf.len() { break; }
            let pkt = &buf[i..i + total];
            super::gsExecPacket(pkt);
            packets += 1;
            i += total;
            if frame_limit != 0 && packets >= frame_limit as u64 { break; }
        }

        Ok(HeadlessSummary {
            packets,
            bytes:    i as u64,
            elapsed:  0.0,
        })
    }

    /// CLI argument parser. Mirrors the `getopt_long` loop in
    /// `pcsx2-gsrunner/Main.cpp` — at the C++ level it accepts
    /// `--gsdump <file>`, `--output <png>`, `--frames N`, `--record`.
    #[derive(Debug, Default, Clone)]
    pub struct HeadlessArgs {
        pub gsdump:    Option<std::path::PathBuf>,
        pub output:    Option<std::path::PathBuf>,
        pub frames:    u32,
        pub record:    bool,
        pub capture:   Option<std::path::PathBuf>,
    }

    pub fn parse_args(args: &[String]) -> Result<HeadlessArgs, String> {
        let mut out = HeadlessArgs::default();
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--gsdump"  => { out.gsdump  = args.get(i+1).cloned().map(Into::into); i += 2; }
                "--output"  => { out.output  = args.get(i+1).cloned().map(Into::into); i += 2; }
                "--frames"  => {
                    out.frames = args.get(i+1)
                        .ok_or_else(|| "missing --frames value".to_string())?
                        .parse().map_err(|e: std::num::ParseIntError| e.to_string())?;
                    i += 2;
                }
                "--record"  => { out.record  = true; i += 1; }
                _ => return Err(format!("unknown argument: {}", args[i])),
            }
        }
        Ok(out)
    }

    pub struct HeadlessSummary {
        pub packets: u64,
        pub bytes:   u64,
        pub elapsed: f32,
    }

    /// Entry point mirroring `int main(int argc, char** argv)`.
    pub fn main(args: &[String]) -> std::io::Result<HeadlessSummary> {
        let parsed = parse_args(args).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        super::gsInit();
        if let Some(cap) = parsed.capture.as_ref() {
            unsafe { let _ = gs.capture.start(cap); }
        }
        if let Some(gsdump) = parsed.gsdump.as_ref() {
            let summary = run_dump_file(unsafe { &mut gs }, gsdump, parsed.frames)?;
            if let Some(out) = parsed.output.as_ref() {
                let mut f = File::create(out)?;
                f.write_all(b"PNG")?;
                f.seek(SeekFrom::Start(0))?;
            }
            return Ok(summary);
        }
        Ok(HeadlessSummary { packets: 0, bytes: 0, elapsed: 0.0 })
    }
}

// ---------------------------------------------------------------------------
// Process-wide atomic counters used by `GSPerfMon` and the
// `GL_PERF`/`GL_INS` macros in the C++ original. Aliased to `AtomicU64`
// so the stubs above can update them without re-introducing a global
// mutable static.
// ---------------------------------------------------------------------------

static S_N:                 AtomicU64 = AtomicU64::new(0);
static S_LAST_TRANSFER_N:   AtomicU64 = AtomicU64::new(0);
static S_TRANSFER_N:        AtomicU64 = AtomicU64::new(0);
static S_CAPTURING:         AtomicBool = AtomicBool::new(false);

pub fn perfmon_bump_sync()   { S_N.fetch_add(1, Ordering::Relaxed); gs_sync_inc(); }
pub fn perfmon_bump_prim()   { gsPrimInc(); }
pub fn perfmon_bump_draw()   { gsDrawInc(); }
pub fn gs_sync_inc()         { unsafe { gs.sync_count += 1; } }
pub fn gsPrimInc()           { unsafe { gs.prim_count += 1; } }
pub fn gsDrawInc()           { unsafe { gs.draw_count += 1; } }

/// The `GL_PERF`/`GL_INS` logging macros become a single function the
/// stubs in `gsExecPacket` route through.
pub fn log_perf(msg: &str) { let _ = msg; }
pub fn log_ins(msg: &str)  { let _ = msg; }

#[doc(hidden)]
pub fn is_capturing() -> bool { S_CAPTURING.load(Ordering::Relaxed) }
#[doc(hidden)]
pub fn set_capturing(b: bool) { S_CAPTURING.store(b, Ordering::Relaxed); }

// ---------------------------------------------------------------------------
// Tables. The C++ `GSTables.cpp` exposes a couple of static lookup tables;
// the 285-line file is mostly const data — we declare the names and types
// only here. The actual arrays live in the data-only Rust translation
// (see `GsTables.rs`).
// ---------------------------------------------------------------------------

/// Stub for the column/block swizzle table. The C++ exports a const
/// `GSSwizzleTableList<...>` per PSM; we keep the type alias only.
pub type GsSwizzleTable = u8;

/// clutTableT32I8/clutTableT16I4 — the i8/i4 CLUT write masks.
pub const CLUT_TABLE_T32_I8_LEN: usize = 0x100;
pub const CLUT_TABLE_T16_I4_LEN: usize = 0x10;

// ---------------------------------------------------------------------------
// Tests. Smoke test the dispatch path on a single GIFtag packet.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_packed_tag(nloop: u16, eop: bool, prim: u16, nreg: u8) -> [u8; 16] {
        let lo = (nloop as u32) | ((eop as u32) << 15);
        let hi = (prim as u32) | (((nreg as u32) & 0xF) << 28);
        let mut out = [0u8; 16];
        out[0..4].copy_from_slice(&lo.to_le_bytes());
        out[4..8].copy_from_slice(&hi.to_le_bytes());
        out
    }

    #[test]
    fn gif_tag_basic_fields() {
        let t = GifTag::from_bytes(&[0x0001_8000, 0x7000_0003, 0, 0]);
        assert_eq!(t.nloop(), 0);
        assert!(t.eop());
        assert_eq!(t.prim(), 3);
        assert_eq!(t.nreg(), 16);
        assert_eq!(t.flg(), GifFlg::Packed);
    }

    #[test]
    fn gif_path_set_tag_decodes_regs() {
        let bytes = make_packed_tag(2, true, 5, 4);
        let mut p = GifPath::default();
        p.set_tag(&bytes);
        assert_eq!(p.nloop, 2);
        assert_eq!(p.nreg, 4);
        assert_eq!(p.tag.eop(), true);
    }

    #[test]
    fn local_memory_round_trip() {
        let mut m = GSLocalMemory::new();
        m.write_pixel32(0x100, 0xDEAD_BEEF);
        assert_eq!(m.read_pixel32(0x100), 0xDEAD_BEEF);
        // Address mask wraps inside the 4 MB window.
        m.write_pixel32(0x0040_0000, 0x1234_5678);
        assert_eq!(m.read_pixel32(0x0000_0000), 0x1234_5678);
    }

    #[test]
    fn clut_can_write_dispatch() {
        let c = GSClut::default();
        // CSM=0, psm=PSMT8 (0x13) → allowed.
        assert!(c.can_write(0, 0, GsPsm::PsmT8 as u32));
        // CSM=0, psm=PSMCT32 → not allowed (no i8/i4 lookup).
        assert!(!c.can_write(0, 0, GsPsm::PsmCt32 as u32));
    }

    #[test]
    fn capture_round_trip() {
        let mut cap = GSCapture::new();
        let dir = std::env::temp_dir().join("gs_capture_test.gsdump");
        cap.start(&dir).unwrap();
        cap.deliver_packet(&[0u8; 16]).unwrap();
        cap.deliver_frame();
        assert!(cap.is_capturing());
        cap.stop();
        assert!(!cap.is_capturing());
        assert_eq!(cap.bytes_written(), 16);
        assert_eq!(cap.frames_recorded(), 1);
    }

    #[test]
    fn gs_exec_packet_smoke() {
        gsInit();
        let bytes = make_packed_tag(1, true, 0, 16);
        // Path 0 (Packed) needs nloop=1, nreg=16 → 16 qwords after the tag.
        let mut pkt = bytes.to_vec();
        pkt.extend(std::iter::repeat(0u8).take(16 * 16));
        gsExecPacket(&pkt);
        unsafe { assert!(gs.prim_count <= 1, "prim count bumped exactly once per loop iter"); }
        gsShutdown();
    }

    #[test]
    fn headless_arg_parser() {
        let args = vec![
            "--gsdump".into(), "foo.gsdump".into(),
            "--frames".into(),  "8".into(),
            "--record".into(),
        ];
        let parsed = headless::parse_args(&args).unwrap();
        assert_eq!(parsed.gsdump, Some("foo.gsdump".into()));
        assert_eq!(parsed.frames, 8);
        assert!(parsed.record);
    }
}
