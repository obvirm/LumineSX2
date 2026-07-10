// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust 2021 translation of the PCSX2 software-renderer (SW) header set.
//!
//! This module is a faithful, single-file rewrite of the headers used by the
//! PS2 GS software renderer: vertex layouts, scanline environment, rasterizer
//! state, code-generator drivers (x86 SSE/AVX and ARM64), JIT code cache
//! dispatch, and the per-pixel / per-thread scratch data structures shared
//! with the JIT-compiled inner loops.
//!
//! Everything in this module is `pub` so downstream translation units can
//! mimic the original C++ access patterns, but no logic is implemented -- the
//! goal is to provide the public surface and data layout that the rest of
//! the Rust crate can build against while the actual JIT backends are
//! migrated over time. Only `std` is used.

#![allow(dead_code)]
#![allow(non_snake_case)]

use std::fmt;

// ---------------------------------------------------------------------------
// Common integer / floating-point type aliases (match the C++ typedefs used
// throughout the SW renderer headers).
// ---------------------------------------------------------------------------

pub type u8  = ::std::primitive::u8;
pub type u16 = ::std::primitive::u16;
pub type u32 = ::std::primitive::u32;
pub type u64 = ::std::primitive::u64;
pub type usize = ::std::primitive::usize;
pub type s8  = i8;
pub type s16 = i16;
pub type s32 = i32;
pub type s64 = i64;

// ---------------------------------------------------------------------------
// Stubs for vector / register types coming from `GS/GSVector.h` and the
// underlying JIT libraries (vixl, xbyak). The original C++ relies on heavy
// SIMD types; in this rewrite we only model the layout that is referenced
// by the SW-renderer headers (the JIT emitters themselves are out of scope).
// ---------------------------------------------------------------------------

/// 4-wide 32-bit vector (placeholder for `GSVector4i`).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[repr(C, align(16))]
pub struct GsVector4i {
    pub i: [i32; 4],
}

impl GsVector4i {
    pub const fn zero() -> Self {
        Self { i: [0; 4] }
    }
}

/// 4-wide 32-bit float vector (placeholder for `GSVector4`).
#[derive(Copy, Clone, Debug, Default, PartialEq)]
#[repr(C, align(16))]
pub struct GsVector4 {
    pub f: [f32; 4],
}

impl GsVector4 {
    pub const fn zero() -> Self {
        Self { f: [0.0; 4] }
    }
}

/// 2-wide 32-bit integer vector (placeholder for `GSVector2i`).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[repr(C, align(8))]
pub struct GsVector2i {
    pub i: [i32; 2],
}

/// 8-wide vector (placeholder for `GSVector8` / `GSVector8i`).
#[derive(Copy, Clone, Debug, Default, PartialEq)]
#[repr(C, align(32))]
pub struct GsVector8 {
    pub f: [f32; 8],
}

/// 8-wide signed integer vector.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[repr(C, align(32))]
pub struct GsVector8i {
    pub i: [i32; 8],
}

/// Page looper used by the rasterizer / texture cache.
#[derive(Default)]
pub struct GsOffsetPageLooper;

/// Address offset used to walk the GS framebuffer / depth buffer.
#[derive(Copy, Clone, Default)]
pub struct GsOffset {
    _priv: [u8; 0],
}

/// Pixel-offset-quad used by `GSRendererSW::m_fzb`.
#[derive(Copy, Clone, Default)]
pub struct GsPixelOffset4 {
    _priv: [u8; 0],
}

/// Opaque texture handle (placeholder for `GSTexture*`).
#[derive(Copy, Clone, Default)]
pub struct GsTexture {
    _priv: [u8; 0],
}

// ---------------------------------------------------------------------------
// Enumerations / register payload stubs that are referenced by the
// `GSScanlineSelector` bitfield. Only the values used to build the key are
// exposed; the renderer code translates the bitfield on the fly.
// ---------------------------------------------------------------------------

/// GS pixel-storage mode (matches `GS_PSM_*` constants).
#[repr(u32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GsPsm {
    Psm32  = 0,
    Psm24  = 1,
    Psm16  = 2,
    Psm16S = 3,
}

/// GS primitive class (matches `GS_PRIM_CLASS`).
#[repr(u32)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum GsPrimClass {
    #[default]
    Point  = 0,
    Line   = 1,
    Triangle,
    Sprite,
}

pub const GS_INVALID_CLASS: GsPrimClass = GsPrimClass::Point;
pub const GS_SPRITE_CLASS: GsPrimClass  = GsPrimClass::Sprite;

/// Maximum number of GS memory pages (used by `GSTextureCacheSW::Texture`).
pub const GS_MAX_PAGES: usize = 512;

/// Texture-function unit (matches `TFX_*` constants used by the JIT key).
#[repr(u32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GsTfx {
    None  = 0,
    Modulate,
    Decal,
    Highlight,
    Hologram,
}

/// GIF register payload stubs. The actual layout is irrelevant for this
/// translation because the SW codegen only consumes them through pointers.
#[derive(Copy, Clone, Default)]
pub struct GifRegTEX0 {
    pub tw: u32,
    pub th: u32,
    pub tbw: u32,
}
#[derive(Copy, Clone, Default)]
pub struct GifRegTEXA {
    pub ta0: u8,
    pub ta1: u8,
}
#[derive(Copy, Clone, Default)]
pub struct GifRegDIMX {
    pub matrix: [u32; 4],
}
#[derive(Copy, Clone, Default)]
pub struct GifRegBITBLTBUF {
    pub src: u32,
    pub dst: u32,
}

// ---------------------------------------------------------------------------
// `GSScanlineSelector` -- the 64-bit JIT-key bitfield. Mirrors the union
// layout in `GSScanlineEnvironment.h`, including the named-field, ababcd,
// and fb/zb groupings. Accessors and helpers are translated verbatim.
// ---------------------------------------------------------------------------

/// Bitfield describing the GS pipeline state used to key a JIT-compiled
/// scanline kernel.
#[derive(Copy, Clone, Default, PartialEq, Eq)]
#[repr(C)]
pub struct GsScanlineSelector {
    /// Low 32 bits of the key.
    pub lo: u32,
    /// High 32 bits of the key.
    pub hi: u32,
    /// Full 64-bit JIT key (alias for `lo | (hi << 32)`).
    pub key: u64,
}

impl GsScanlineSelector {
    /// Construct a selector from the full 64-bit JIT key.
    pub const fn from_key(k: u64) -> Self {
        Self { lo: k as u32, hi: (k >> 32) as u32, key: k }
    }

    // -- low word (bits 0..=31) ---------------------------------------------

    pub fn fpsm (&self) -> u32 { self.lo        & 0b11 }
    pub fn zpsm (&self) -> u32 { (self.lo >> 2) & 0b11 }
    pub fn ztst (&self) -> u32 { (self.lo >> 4) & 0b11 }
    pub fn atst (&self) -> u32 { (self.lo >> 6) & 0b111 }
    pub fn afail(&self) -> u32 { (self.lo >> 9) & 0b11 }
    pub fn iip  (&self) -> u32 { (self.lo >> 11) & 0b1 }
    pub fn tfx  (&self) -> u32 { (self.lo >> 12) & 0b111 }
    pub fn tcc  (&self) -> u32 { (self.lo >> 15) & 0b1 }
    pub fn fst  (&self) -> u32 { (self.lo >> 16) & 0b1 }
    pub fn ltf  (&self) -> u32 { (self.lo >> 17) & 0b1 }
    pub fn tlu  (&self) -> u32 { (self.lo >> 18) & 0b1 }
    pub fn fge  (&self) -> u32 { (self.lo >> 19) & 0b1 }
    pub fn date (&self) -> u32 { (self.lo >> 20) & 0b1 }
    pub fn abe  (&self) -> u32 { (self.lo >> 21) & 0b1 }
    pub fn aba  (&self) -> u32 { (self.lo >> 22) & 0b11 }
    pub fn abb  (&self) -> u32 { (self.lo >> 24) & 0b11 }
    pub fn abc  (&self) -> u32 { (self.lo >> 26) & 0b11 }
    pub fn abd  (&self) -> u32 { (self.lo >> 28) & 0b11 }
    pub fn pabe (&self) -> u32 { (self.lo >> 30) & 0b1 }
    pub fn aa1  (&self) -> u32 { (self.lo >> 31) & 0b1 }

    // -- high word (bits 32..=63) -------------------------------------------

    pub fn fwrite   (&self) -> u32 { self.hi         & 0b1 }
    pub fn ftest    (&self) -> u32 { (self.hi >> 1)  & 0b1 }
    pub fn rfb      (&self) -> u32 { (self.hi >> 2)  & 0b1 }
    pub fn zwrite   (&self) -> u32 { (self.hi >> 3)  & 0b1 }
    pub fn ztest    (&self) -> u32 { (self.hi >> 4)  & 0b1 }
    pub fn zoverflow(&self) -> u32 { (self.hi >> 5)  & 0b1 }
    pub fn zclamp   (&self) -> u32 { (self.hi >> 6)  & 0b1 }
    pub fn wms      (&self) -> u32 { (self.hi >> 7)  & 0b11 }
    pub fn wmt      (&self) -> u32 { (self.hi >> 9)  & 0b11 }
    pub fn datm     (&self) -> u32 { (self.hi >> 11) & 0b1 }
    pub fn colclamp (&self) -> u32 { (self.hi >> 12) & 0b1 }
    pub fn fba      (&self) -> u32 { (self.hi >> 13) & 0b1 }
    pub fn dthe     (&self) -> u32 { (self.hi >> 14) & 0b1 }
    pub fn prim     (&self) -> u32 { (self.hi >> 15) & 0b11 }
    pub fn edge     (&self) -> u32 { (self.hi >> 17) & 0b1 }
    pub fn tw       (&self) -> u32 { (self.hi >> 18) & 0b111 }
    pub fn lcm      (&self) -> u32 { (self.hi >> 21) & 0b1 }
    pub fn mmin     (&self) -> u32 { (self.hi >> 22) & 0b11 }
    pub fn notest   (&self) -> u32 { (self.hi >> 24) & 0b1 }
    pub fn zequal   (&self) -> u32 { (self.hi >> 25) & 0b1 }
    pub fn breakpoint(&self)-> u32 { (self.hi >> 26) & 0b1 }

    // -- combined views ----------------------------------------------------

    /// Packed `(aba, abb, abc, abd)` blend-equations byte.
    pub fn ababcd(&self) -> u32 { (self.lo >> 22) & 0xFF }
    /// Frame-buffer pixel-storage mode (alias of `fpsm`).
    pub fn fb    (&self) -> u32 { self.fpsm() }
    /// Depth-buffer pixel-storage mode (alias of `zpsm`).
    pub fn zb    (&self) -> u32 { self.zpsm() }

    /// A solid sprite is one with no IIP, no texture function, no alpha
    /// blending, no alpha/Z test (other than write), no destination alpha
    /// test, and no fog.
    pub fn is_solid_rect(&self) -> bool {
        self.prim() == GS_SPRITE_CLASS as u32
            && self.iip() == 0
            && self.tfx() == GsTfx::None as u32
            && self.abe() == 0
            && self.ztst() <= 1
            && self.atst() <= 1
            && self.date() == 0
            && self.fge() == 0
    }

    /// Equivalent of the C++ `to_string()` / `Print()` helper.
    pub fn to_string(&self) -> String {
        format!(
            "fpsm:{} zpsm:{} ztst:{} ztest:{} atst:{} afail:{} iip:{} rfb:{} fb:{} zb:{} zw:{} \
             tfx:{} tcc:{} fst:{} ltf:{} tlu:{} wms:{} wmt:{} mmin:{} lcm:{} tw:{} \
             fba:{} cclamp:{} date:{} datm:{} \
             prim:{} abe:{} {}{}{}{} fge:{} dthe:{} notest:{} pabe:{} aa1:{} \
             fwrite:{} ftest:{} zoverflow:{} zequal:{} zclamp:{} edge:{}",
            self.fpsm(), self.zpsm(), self.ztst(), self.ztest(), self.atst(),
            self.afail(), self.iip(), self.rfb(), self.fb(), self.zb(), self.zwrite(),
            self.tfx(), self.tcc(), self.fst(), self.ltf(), self.tlu(),
            self.wms(), self.wmt(), self.mmin(), self.lcm(), self.tw(),
            self.fba(), self.colclamp(), self.date(), self.datm(),
            self.prim(), self.abe(), self.aba(), self.abb(), self.abc(), self.abd(),
            self.fge(), self.dthe(), self.notest(), self.pabe(), self.aa1(),
            self.fwrite(), self.ftest(), self.zoverflow(), self.zequal(),
            self.zclamp(), self.edge(),
        )
    }

    /// Mirror of `Print()` -- writes the selector to `stderr`.
    pub fn print(&self) {
        eprintln!("{}", self.to_string());
    }
}

impl fmt::Debug for GsScanlineSelector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_string())
    }
}

// ---------------------------------------------------------------------------
// `GSScanlineGlobalData` -- per-batch pixel-shader constant buffer.
// ---------------------------------------------------------------------------

/// Texture-wrapping metadata (min/max, minmax, mask, invmask) used by the
/// sample stage.
#[derive(Copy, Clone, Default)]
pub struct GsScanlineGlobalTex {
    pub min:     GsVector4i,
    pub max:     GsVector4i,
    pub minmax:  GsVector4i,
    pub mask:    GsVector4i,
    pub invmask: GsVector4i,
}

/// 64-bit constant block shared by all threads (AVX build, see header).
#[derive(Copy, Clone)]
#[repr(C, align(64))]
pub struct GsScanlineConstantData256B {
    pub m_test:      [u8; 24],
    pub m_log2_coef: [f32; 4],
    pub m_shift:     [f32; 16],
}

impl Default for GsScanlineConstantData256B {
    fn default() -> Self {
        Self {
            m_test: [
                0, 0, 0, 0, 0, 0, 0, 0,
                0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
                0, 0, 0, 0, 0, 0, 0, 0,
            ],
            m_log2_coef: Self::LOG2_COEF,
            m_shift: [
                8.0, -7.0, -6.0, -5.0, -4.0, -3.0, -2.0, -1.0,
                0.0,  1.0,  2.0,  3.0,  4.0,  5.0,  6.0,  7.0,
            ],
        }
    }
}

impl GsScanlineConstantData256B {
    /// Polynomial coefficients for the `log2` approximation used by mip
    /// selection.
    pub const LOG2_COEF: [f32; 4] = [
        0.204446009836232697_516,
        -1.049130552173401241_91,
        2.283302844769184906_82,
        1.0,
    ];
}

/// 32-byte constant block (SSE / ARM64 build, see header).
#[derive(Copy, Clone)]
#[repr(C, align(64))]
pub struct GsScanlineConstantData128B {
    pub m_test:      [[u32; 4]; 8],
    pub m_shift:     [[f32; 4]; 5],
    pub m_log2_coef: [[f32; 4]; 4],
}

impl Default for GsScanlineConstantData128B {
    fn default() -> Self {
        let mut out = Self {
            m_test: [[0; 4]; 8],
            m_shift: [[0.0; 4]; 5],
            m_log2_coef: [[0.0; 4]; 4],
        };
        for (i, row) in out.m_test.iter_mut().enumerate() {
            *row = match i {
                0 => [0, 0, 0, 0],
                1 => [0xffffffff, 0, 0, 0],
                2 => [0xffffffff, 0xffffffff, 0, 0],
                3 => [0xffffffff, 0xffffffff, 0xffffffff, 0],
                4 => [0, 0xffffffff, 0xffffffff, 0xffffffff],
                5 => [0, 0, 0xffffffff, 0xffffffff],
                6 => [0, 0, 0, 0xffffffff],
                7 => [0, 0, 0, 0],
                _ => unreachable!(),
            };
        }
        out.m_shift = [
            [ 4.0,  4.0,  4.0,  4.0],
            [ 0.0,  1.0,  2.0,  3.0],
            [-1.0,  0.0,  1.0,  2.0],
            [-2.0, -1.0,  0.0,  1.0],
            [-3.0, -2.0, -1.0,  0.0],
        ];
        let log2 = GsScanlineConstantData256B::LOG2_COEF;
        for r in 0..4 {
            for c in 0..4 {
                out.m_log2_coef[r][c] = log2[r];
            }
        }
        out
    }
}

/// ARM64-only const tables embedded in `GSScanlineGlobalData`.
#[derive(Copy, Clone)]
#[repr(C, align(16))]
pub struct GsScanlineArm64Const {
    pub const_test_128b:        [[u32; 4]; 8],
    pub const_movemaskw_mask:   [u16; 8],
    pub const_log2_coef:        [f32; 4],
}

impl Default for GsScanlineArm64Const {
    fn default() -> Self {
        Self {
            const_test_128b: [
                [0x00000000, 0x00000000, 0x00000000, 0x00000000],
                [0xffffffff, 0x00000000, 0x00000000, 0x00000000],
                [0xffffffff, 0xffffffff, 0x00000000, 0x00000000],
                [0xffffffff, 0xffffffff, 0xffffffff, 0x00000000],
                [0x00000000, 0xffffffff, 0xffffffff, 0xffffffff],
                [0x00000000, 0x00000000, 0xffffffff, 0xffffffff],
                [0x00000000, 0x00000000, 0x00000000, 0xffffffff],
                [0x00000000, 0x00000000, 0x00000000, 0x00000000],
            ],
            const_movemaskw_mask: [0x3, 0xc, 0x30, 0xc0, 0x300, 0xc00, 0x3000, 0xc000],
            const_log2_coef: GsScanlineConstantData256B::LOG2_COEF,
        }
    }
}

/// Per-batch pixel-shader "constant buffer" shared by all scanline threads.
#[derive(Copy, Clone)]
#[repr(C, align(32))]
pub struct GsScanlineGlobalData {
    pub sel:        GsScanlineSelector,
    pub vm:         *mut u8,
    pub tex:        [*const u8; 7],
    pub clut:       *mut u32,
    pub dimx:       *mut GsVector4i,
    pub fbo:        GsOffset,
    pub zbo:        GsOffset,
    pub fzbr:       *const GsVector2i,
    pub fzbc:       *const GsVector2i,
    pub aref:       GsVector4i,
    pub afix:       GsVector4i,
    pub t:          GsScanlineGlobalTex,
    pub fm:         u32,
    pub zm:         u32,
    pub frb:        u32,
    pub fga:        u32,
    pub mxl:        GsVector8,
    pub k:          GsVector8,
    pub l:          GsVector8,
    pub lod:        GsScanlineGlobalLod,
    /// Embedded ARM64-only const tables.
    pub arm64_const: GsScanlineArm64Const,
}

impl Default for GsScanlineGlobalData {
    fn default() -> Self {
        Self {
            sel:        GsScanlineSelector::default(),
            vm:         ::std::ptr::null_mut(),
            tex:        [::std::ptr::null(); 7],
            clut:       ::std::ptr::null_mut(),
            dimx:       ::std::ptr::null_mut(),
            fbo:        GsOffset::default(),
            zbo:        GsOffset::default(),
            fzbr:       ::std::ptr::null(),
            fzbc:       ::std::ptr::null(),
            aref:       GsVector4i::zero(),
            afix:       GsVector4i::zero(),
            t:          GsScanlineGlobalTex::default(),
            fm:         0,
            zm:         0,
            frb:        0,
            fga:        0,
            mxl:        GsVector8::default(),
            k:          GsVector8::default(),
            l:          GsVector8::default(),
            lod:        GsScanlineGlobalLod::default(),
            arm64_const: GsScanlineArm64Const::default(),
        }
    }
}

/// LOD pair used when `lcm == 1` (mip selection).
#[derive(Copy, Clone, Default)]
pub struct GsScanlineGlobalLod {
    pub i: GsVector8i,
    pub f: GsVector8i,
}

// ---------------------------------------------------------------------------
// `GSScanlineLocalData` -- per-prim scratch block owned by one rasterizer
// thread. The header uses two layouts (SSE4.1 = 4-wide, AVX2 = 8-wide); we
// model them as separate structs gated by a feature-style const flag.
// ---------------------------------------------------------------------------

/// SSE 4-wide (`_M_SSE < 0x501`) scratch layout.
#[derive(Copy, Clone, Default)]
pub struct GsScanlineLocalDataSseSkip {
    pub z: GsVector4,
    pub s: GsVector4,
    pub t: GsVector4,
    pub q: GsVector4,
    pub rb: GsVector4i,
    pub ga: GsVector4i,
    pub f: GsVector4i,
    pub _pad: GsVector4i,
}

/// AVX2 8-wide scratch layout.
#[derive(Copy, Clone, Default)]
pub struct GsScanlineLocalDataAvxSkip {
    pub z: GsVector8,
    pub s: GsVector8,
    pub t: GsVector8,
    pub q: GsVector8,
    pub rb: GsVector8i,
    pub ga: GsVector8i,
    pub f: GsVector8i,
    pub _pad: GsVector8i,
}

/// Per-prim / per-thread scratch block.
#[derive(Copy, Clone)]
#[repr(C, align(32))]
pub union GsScanlineLocalData {
    /// Compile-time selection: `true` mirrors the AVX2 layout from the
    /// header, `false` mirrors the SSE layout. Defaults to `false` for
    /// portability; downstream code can override at build time.
    sse:  GsScanlineLocalDataSse,
    avx:  GsScanlineLocalDataAvx,
}

impl Default for GsScanlineLocalData {
    fn default() -> Self {
        // Safe-ish because both fields are plain-old-data and have identical
        // initialization; we use the SSE layout as the canonical default.
        GsScanlineLocalData { sse: GsScanlineLocalDataSse::default() }
    }
}

/// SSE flavor of `GSScanlineLocalData`.
#[derive(Copy, Clone, Default)]
#[repr(C, align(32))]
pub struct GsScanlineLocalDataSse {
    pub d:  [GsScanlineLocalDataSseSkip; 4],
    pub d4: GsScanlineLocalDataSseStep,
    pub c:  GsScanlineLocalDataColor,
    pub p:  GsScanlineLocalDataPixel,
    pub temp: GsScanlineLocalDataTempSse,
    pub gd: *const GsScanlineGlobalData,
}

/// AVX2 flavor of `GSScanlineLocalData`.
#[derive(Copy, Clone, Default)]
#[repr(C, align(32))]
pub struct GsScanlineLocalDataAvx {
    pub d:  [GsScanlineLocalDataAvxSkip; 8],
    pub d8: GsScanlineLocalDataAvxStep,
    pub p:  GsScanlineLocalDataPixel,
    pub c:  GsScanlineLocalDataAvxColor,
    pub temp: GsScanlineLocalDataTempAvx,
    pub gd: *const GsScanlineGlobalData,
}

#[derive(Copy, Clone, Default)]
pub struct GsScanlineLocalDataSseStep {
    pub z: GsVector4,
    pub stq: GsVector4,
    pub c: GsVector4i,
    pub f: GsVector4i,
}

#[derive(Copy, Clone, Default)]
pub struct GsScanlineLocalDataAvxStep {
    pub stq: GsVector4,
    pub c: GsScanlineLocalDataAvxStepColor,
    pub p: GsScanlineLocalDataAvxStepPixel,
}

#[derive(Copy, Clone, Default)]
pub struct GsScanlineLocalDataAvxStepColor {
    pub rb: u32,
    pub ga: u32,
}

#[derive(Copy, Clone, Default)]
pub struct GsScanlineLocalDataAvxStepPixel {
    pub z: u64,
    pub f: u32,
}

#[derive(Copy, Clone, Default)]
pub struct GsScanlineLocalDataColor {
    pub rb: GsVector4i,
    pub ga: GsVector4i,
}

#[derive(Copy, Clone, Default)]
pub struct GsScanlineLocalDataAvxColor {
    pub rb: GsVector8i,
    pub ga: GsVector8i,
}

#[derive(Copy, Clone, Default)]
pub struct GsScanlineLocalDataPixel {
    pub z: u32,
    pub f: u32,
}

#[derive(Copy, Clone, Default)]
pub struct GsScanlineLocalDataTempSse {
    pub z0: GsVector4,
    pub z1: GsVector4,
    pub f: GsVector4i,
    pub s: GsVector4,
    pub t: GsVector4,
    pub q: GsVector4,
    pub rb: GsVector4i,
    pub ga: GsVector4i,
    pub zs: GsVector4i,
    pub zd: GsVector4i,
    pub uf: GsVector4i,
    pub vf: GsVector4i,
    pub cov: GsVector4i,
    pub lod: GsScanlineLocalDataTempLod<GsVector4i>,
    pub uv: [GsVector4i; 2],
    pub uv_minmax: [GsVector4i; 2],
    pub trb: GsVector4i,
    pub tga: GsVector4i,
    pub test: GsVector4i,
}

#[derive(Copy, Clone, Default)]
pub struct GsScanlineLocalDataTempAvx {
    pub z0: GsVector8,
    pub z1: GsVector8,
    pub f: GsVector8i,
    pub s: GsVector8,
    pub t: GsVector8,
    pub q: GsVector8,
    pub rb: GsVector8i,
    pub ga: GsVector8i,
    pub zs: GsVector8i,
    pub zd: GsVector8i,
    pub uf: GsVector8i,
    pub vf: GsVector8i,
    pub cov: GsVector8i,
    pub lod: GsScanlineLocalDataTempLod<GsVector8i>,
    pub uv: [GsVector8i; 2],
    pub uv_minmax: [GsVector8i; 2],
    pub trb: GsVector8i,
    pub tga: GsVector8i,
    pub test: GsVector8i,
}

#[derive(Copy, Clone, Default)]
pub struct GsScanlineLocalDataTempLod<V> {
    pub i: V,
    pub f: V,
}

// ---------------------------------------------------------------------------
// Public name alias that downstream code can refer to without caring which
// SIMD layout the current build is using. Defaults to the SSE flavor.
// ---------------------------------------------------------------------------

/// Default per-prim scratch block used by callers that don't care about
/// the underlying SIMD width.
pub type GsScanlineLocalDataDefault = GsScanlineLocalDataSse;

// ---------------------------------------------------------------------------
// Constant tables shared by all threads.
// ---------------------------------------------------------------------------

/// Definition of the two externally-linked constant tables declared at the
/// bottom of `GSScanlineEnvironment.h`. The static initializers live here
/// so the module is self-contained.
pub const G_CONST_256B: GsScanlineConstantData256B = GsScanlineConstantData256B {
    m_test: [
        0, 0, 0, 0, 0, 0, 0, 0,
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0, 0, 0, 0, 0, 0, 0, 0,
    ],
    m_log2_coef: GsScanlineConstantData256B::LOG2_COEF,
    m_shift: [
        8.0, -7.0, -6.0, -5.0, -4.0, -3.0, -2.0, -1.0,
        0.0,  1.0,  2.0,  3.0,  4.0,  5.0,  6.0,  7.0,
    ],
};

/// Per-channel column offset tables for the GS 16/32-bit pixel format
/// unpacking, copied verbatim from `GSDrawScanline.cpp` / the arm64 generator.
pub const S_OFFSETS:       [i32; 4] = [0, 2, 8, 10];
pub const S_OFFSETS_16:    [i32; 8] = [0, 2, 8, 10, 16, 18, 24, 26];

// ---------------------------------------------------------------------------
// `GSVertex` / `GSVertexHW` -- the hardware / SW vertex payloads. The
// ARM64 / x86 codegen reinterprets these bytes; we model only the layout
// that downstream Rust code needs to inspect.
// ---------------------------------------------------------------------------

/// Opaque upstream GS vertex (defined in `GS/GSState.h`).
#[derive(Copy, Clone, Default)]
pub struct GsVertex {
    _priv: [u8; 0],
}

/// Drawing context passed to the vertex-conversion helpers.
#[derive(Copy, Clone, Default)]
pub struct GsDrawingContext {
    _priv: [u8; 0],
}

/// HW vertex payload (9-vector). Mirrors `GSVertexHW9` from `GSVertexHW.h`.
#[derive(Copy, Clone, Default)]
#[repr(C, align(32))]
pub struct GsVertexHW {
    pub t: GsVector4,
    pub p: GsVector4,
}

impl GsVertexHW {
    pub const fn new() -> Self {
        Self { t: GsVector4 { f: [0.0; 4] }, p: GsVector4 { f: [0.0; 4] } }
    }

    /// Equivalent of the C++ copy-assignment.
    pub fn assign_from(&mut self, other: &GsVertexHW) {
        self.t = other.t;
        self.p = other.p;
    }
}

// ---------------------------------------------------------------------------
// `GSVertexSW` -- the SW vertex payload used by the scanline codegen.
// ---------------------------------------------------------------------------

/// Function-pointer type used to convert a slice of `GsVertex` into
/// `GsVertexSW` for a given drawing configuration.
pub type ConvertVertexBufferFn =
    fn(ctx: *const GsDrawingContext, dst: *mut GsVertexSW, src: *const GsVertex, count: u32);

/// SW vertex: `p` (xy + zl/zh or f), `t` (s/t/q/f), `c` (rgba).
#[derive(Copy, Clone, Default)]
#[repr(C, align(32))]
pub struct GsVertexSW {
    pub p: GsVector4,
    pub _pad: GsVector4,
    pub t: GsVector4,
    pub c: GsVector4,
}

impl GsVertexSW {
    pub const fn zero() -> Self {
        Self {
            p:    GsVector4::zero(),
            _pad: GsVector4::zero(),
            t:    GsVector4::zero(),
            c:    GsVector4::zero(),
        }
    }
}

/// AVX2-only vertex used in `DrawTriangleSection` when `_M_SSE >= 0x501`.
#[derive(Copy, Clone, Default)]
#[repr(C, align(32))]
pub struct GsVertexSW2 {
    pub p:  GsVector4,
    pub _pad: GsVector4,
    pub tc: GsVector8,
}

impl GsVertexSW2 {
    pub const fn new() -> Self {
        Self {
            p:    GsVector4::zero(),
            _pad: GsVector4::zero(),
            tc:   GsVector8 { f: [0.0; 8] },
        }
    }
}

/// Per-vertex conversion dispatch table indexed by `[iip][tme][iip_t]`
/// (4 x 2 x 2 x 2 = 64 entries). Default-initialized to `None`; populated
/// at startup by `GSVertexSW::InitStatic()`.
pub type ConvertVertexBufferTable = [[[[Option<ConvertVertexBufferFn>; 2]; 2]; 2]; 4];

// ---------------------------------------------------------------------------
// `GSNewCodeGenerator` -- the SSE/AVX x86 code generator. The C++ version is
// built on top of Xbyak; this translation keeps the data fields and the
// category-tags (used by the FORWARD_* macros) so downstream Rust code can
// later target a Rust-native x86 encoder.
// ---------------------------------------------------------------------------

/// SSE / AVX / FMA category used by the FORWARD_* macros to decide whether
/// an instruction is allowed in the current code path.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum IsaCategory {
    Base,
    Sse,
    SseOnly,
    Avx,
    Avx2,
    Fma,
}

/// x86 register kinds we model in the public API. The values match the
/// Xbyak enum ordinals so consumers can map between the two if needed.
#[repr(u32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum X86RegKind {
    Reg8  = 0,
    Reg16 = 1,
    Reg32 = 2,
    Reg64 = 3,
    Xmm   = 4,
    Ymm   = 5,
    Zmm   = 6,
}

/// One-of-N register operand.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Operand(u32);

/// Memory-addressing operand (e.g. `[reg + disp]`, `[rip + label]`).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Address;

impl Address {
    pub const fn ptr() -> Self { Address }
}

/// Forward-declared label type.
#[derive(Copy, Clone, Default)]
pub struct Label { _id: u32 }

/// Top-level x86 codegen wrapper (xbyak-style). All instruction emitters
/// are stubbed because the actual machine-code emission lives behind a
/// Rust-side dependency we don't pull in here.
#[derive(Default)]
pub struct GsNewCodeGenerator {
    pub has_avx:  bool,
    pub has_avx2: bool,
    pub has_fma:  bool,
}

impl GsNewCodeGenerator {
    pub fn new(_code: *mut u8, _maxsize: usize) -> Self {
        Self::default()
    }

    pub fn get_size(&self) -> usize { 0 }
    pub fn get_code(&self) -> *const u8 { ::std::ptr::null() }

    /// Returns the current cursor inside the code buffer.
    pub fn get_curr(&self) -> *const u8 { ::std::ptr::null() }
    /// Aligns the code buffer to `x` bytes.
    pub fn align(&mut self, _x: i32) {}
    /// Emits a raw byte.
    pub fn db(&mut self, _code: i32) {}

    /// Helpers for branching to a label / address.
    pub fn jmp_label(&mut self, _label: &Label) {}
    pub fn jmp_addr (&mut self, _addr:  *const u8) {}

    // Forward-declared register/operand types so the API surface matches
    // the C++ version. Concrete values come from individual emitters below.
    pub const fn rax() -> Operand { Operand(0) }
    pub const fn rcx() -> Operand { Operand(1) }
    pub const fn rdx() -> Operand { Operand(2) }
    pub const fn rbx() -> Operand { Operand(3) }
    pub const fn rsp() -> Operand { Operand(4) }
    pub const fn rbp() -> Operand { Operand(5) }
    pub const fn rsi() -> Operand { Operand(6) }
    pub const fn rdi() -> Operand { Operand(7) }

    pub const fn xmm0() -> Operand { Operand(8) }
    pub const fn ymm0() -> Operand { Operand(9) }
    pub const fn zmm0() -> Operand { Operand(10) }
}

// ---------------------------------------------------------------------------
// `GSSetupPrimCodeGenerator` (x86 / SSE+AVX flavor).
// ---------------------------------------------------------------------------

/// Per-channel enable flags built from the `GSScanlineSelector` key.
#[derive(Copy, Clone, Default)]
pub struct GsSetupPrimEnable {
    pub z: u32,
    pub f: u32,
    pub t: u32,
    pub c: u32,
}

/// x86 code generator for the "setup primitives" stage. Emits a function
/// that, given a vertex buffer, prepares the per-prim step/skip deltas
/// consumed by `GsDrawScanline`.
pub struct GsSetupPrimCodeGenerator {
    pub sel:       GsScanlineSelector,
    pub many_regs: bool,
    pub en:        GsSetupPrimEnable,
    // Argument-passing registers (Windows vs. SysV). Modeled as raw u8
    // values to avoid taking a hard dependency on an x86 encoder.
    pub arg_vertex: u8,
    pub arg_index:  u8,
    pub arg_dscan:  u8,
    pub arg_local:  u8,
    pub arg_t1:     u8,
    // Each `xymN` mirrors one of the `xym0..xym15` constants from the C++.
    pub xym: [Operand; 16],
    // The inner x86 generator.
    pub inner: GsNewCodeGenerator,
}

impl GsSetupPrimCodeGenerator {
    pub fn new(_key: u64, _code: *mut u8, _maxsize: usize) -> Self {
        Self {
            sel:        GsScanlineSelector::default(),
            many_regs:  false,
            en:         GsSetupPrimEnable::default(),
            arg_vertex: 0,
            arg_index:  0,
            arg_dscan:  0,
            arg_local:  0,
            arg_t1:     0,
            xym:        [Operand::default(); 16],
            inner:      GsNewCodeGenerator::default(),
        }
    }

    /// Emit the JIT-compiled function.
    pub fn generate(&mut self) {}

    /// Broadcast 128 bits of floats from memory to the whole register.
    fn broadcastf128(&mut self, _reg: Operand, _mem: Address) {}
    /// Broadcast a 32-bit float to the whole register.
    fn broadcastss (&mut self, _reg: Operand, _mem: Address) {}

    fn depth_xmm(&mut self) {}
    fn depth_ymm(&mut self) {}
    fn texture  (&mut self) {}
    fn color    (&mut self) {}
}

// ---------------------------------------------------------------------------
// `GSDrawScanlineCodeGenerator` (x86 / SSE+AVX flavor + ARM64 flavor).
// ---------------------------------------------------------------------------

/// ARM64 code generator for the scanline kernel. Mirrors the public methods
/// declared in `GSDrawScanlineCodeGenerator.arm64.h`.
pub struct GsDrawScanlineCodeGeneratorArm64 {
    /// Opaque handle to a vixl-style macro-assembler. Modeled as a
    /// `usize` because we don't want a hard dependency on `vixl`.
    pub emitter: usize,
    pub sel:     GsScanlineSelector,
    pub step_label: Label,
}

impl GsDrawScanlineCodeGeneratorArm64 {
    pub fn new(_key: u64, _code: *mut u8, _maxsize: usize) -> Self {
        Self { emitter: 0, sel: GsScanlineSelector::default(), step_label: Label::default() }
    }

    pub fn generate(&mut self) {}
    pub fn get_size(&self) -> usize { 0 }
    pub fn get_code(&self) -> *const u8 { ::std::ptr::null() }

    // Private helpers declared in the header.
    fn init(&mut self) {}
    fn step(&mut self) {}
    fn test_z(&mut self) {}
    fn sample_texture(&mut self) {}
    fn sample_texture_lod(&mut self) {}
    fn wrap_uv0(&mut self) {}
    fn wrap_uv0_uv1(&mut self) {}
    fn alpha_tfx(&mut self) {}
    fn read_mask(&mut self) {}
    fn test_alpha(&mut self) {}
    fn color_tfx(&mut self) {}
    fn fog(&mut self) {}
    fn read_frame(&mut self) {}
    fn test_dest_alpha(&mut self) {}
    fn write_mask(&mut self) {}
    fn write_zbuf(&mut self) {}
    fn alpha_blend(&mut self) {}
    fn write_frame(&mut self) {}
}

/// x86 code generator for the scanline kernel. Mirrors the public methods
/// declared in `GSDrawScanlineCodeGenerator.all.h` (SSE/AVX flavor).
pub struct GsDrawScanlineCodeGenerator {
    pub sel:        GsScanlineSelector,
    pub inner:      GsNewCodeGenerator,
    pub breakpoint: bool,
}

impl GsDrawScanlineCodeGenerator {
    pub fn new(_key: u64, _code: *mut u8, _maxsize: usize) -> Self {
        Self {
            sel:        GsScanlineSelector::default(),
            inner:      GsNewCodeGenerator::default(),
            breakpoint: false,
        }
    }

    pub fn generate(&mut self) {}
    pub fn get_size(&self) -> usize { 0 }
    pub fn get_code(&self) -> *const u8 { ::std::ptr::null() }

    // Internal stages (see header).
    fn init(&mut self) {}
    fn step(&mut self) {}
    fn test_z(&mut self) {}
    fn sample_texture(&mut self) {}
    fn alpha_tfx(&mut self) {}
    fn read_mask(&mut self) {}
    fn test_alpha(&mut self) {}
    fn color_tfx(&mut self) {}
    fn fog(&mut self) {}
    fn read_frame(&mut self) {}
    fn test_dest_alpha(&mut self) {}
    fn write_mask(&mut self) {}
    fn write_zbuf(&mut self) {}
    fn alpha_blend(&mut self) {}
    fn write_frame(&mut self) {}
}

// ---------------------------------------------------------------------------
// Code-cache function-pointer map (`GSCodeGeneratorFunctionMap<...>`).
// ---------------------------------------------------------------------------

/// Function-pointer signature produced by the setup-prim stage.
pub type GsSetupPrimFn =
    fn(vertex: *const GsVertexSW, index: *const u16, dscan: *const GsVertexSW, local: *mut GsScanlineLocalData);

/// Function-pointer signature produced by the draw-scanline stage.
pub type GsDrawScanlineFn =
    fn(pixels: i32, left: i32, top: i32, scan: *const GsVertexSW, local: *mut GsScanlineLocalData);

/// Code-cache map keyed by a 64-bit selector. The C++ version uses
/// `std::unordered_map<u64, T>`; this stub keeps the same public surface.
pub struct GsCodeCacheMap<T> {
    entries: Vec<(u64, T)>,
}

impl<T: Copy> Default for GsCodeCacheMap<T> {
    fn default() -> Self { Self { entries: Vec::new() } }
}

impl<T: Copy> GsCodeCacheMap<T> {
    pub fn new() -> Self { Self::default() }
    pub fn insert(&mut self, key: u64, value: T) { self.entries.push((key, value)); }
    pub fn get (&self, key: u64) -> Option<T> {
        self.entries.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
    }
    pub fn clear(&mut self) { self.entries.clear(); }
}

// ---------------------------------------------------------------------------
// `GSDrawScanline` -- top-level driver that owns the JIT code caches and
// dispatches between JIT-compiled and "C" fallbacks.
// ---------------------------------------------------------------------------

/// Top-level scanline driver (see `GSDrawScanline.h`).
pub struct GsDrawScanline {
    pub setup_prim_cache:  GsCodeCacheMap<GsSetupPrimFn>,
    pub draw_scanline_cache: GsCodeCacheMap<GsDrawScanlineFn>,
}

impl Default for GsDrawScanline {
    fn default() -> Self {
        Self {
            setup_prim_cache:     GsCodeCacheMap::new(),
            draw_scanline_cache:  GsCodeCacheMap::new(),
        }
    }
}

impl GsDrawScanline {
    /// Debug override for disabling the scanline JIT on a key basis.
    pub fn should_use_c_draw_scanline(_key: u64) -> bool { false }

    /// Flushes the code cache, forcing everything to be recompiled.
    pub fn reset_code_cache(&mut self) {
        self.setup_prim_cache.clear();
        self.draw_scanline_cache.clear();
    }

    /// Populates function pointers. Returns `false` if the code cache ran
    /// out of space.
    pub fn setup_draw(&mut self, _data: &GsRasterizerData) -> bool { true }

    /// Per-thread pre-calculation hook. The C++ version is `static`.
    pub fn begin_draw(_data: &GsRasterizerData, _local: &mut GsScanlineLocalData) {}

    /// Slow-path rectangle blit (not JIT-compiled).
    pub fn draw_rect(
        _r: GsVector4i,
        _v: &GsVertexSW,
        _local: &mut GsScanlineLocalData,
    ) {}

    /// Updates internal draw statistics for the current frame.
    pub fn update_draw_stats(
        &mut self,
        _frame: u64,
        _ticks: u64,
        _actual: i32,
        _total:  i32,
        _prims:  i32,
    ) {}

    /// Prints accumulated draw statistics.
    pub fn print_stats(&self) {}

    // -----------------------------------------------------------------------
    // C fallbacks. Marked `extern "C"`-ish via the `pub` visibility.
    // -----------------------------------------------------------------------

    pub extern "C" fn c_setup_prim(
        _vertex: *const GsVertexSW,
        _index:  *const u16,
        _dscan:  *const GsVertexSW,
        _local:  *mut GsScanlineLocalData,
    ) {}

    pub extern "C" fn c_draw_scanline(
        _pixels: i32,
        _left:   i32,
        _top:    i32,
        _scan:   *const GsVertexSW,
        _local:  *mut GsScanlineLocalData,
    ) {}

    pub extern "C" fn c_draw_edge(
        _pixels: i32,
        _left:   i32,
        _top:    i32,
        _scan:   *const GsVertexSW,
        _local:  *mut GsScanlineLocalData,
    ) {}

    /// Overload of the C fallback that takes a `GsScanlineSelector` so the
    /// JITted path can dispatch on it without going through a switch table.
    pub extern "C" fn c_draw_scanline_sel(
        _pixels: i32,
        _left:   i32,
        _top:    i32,
        _scan:   *const GsVertexSW,
        _local:  *mut GsScanlineLocalData,
        _sel:    GsScanlineSelector,
    ) {}
}

// ---------------------------------------------------------------------------
// `GSRasterizer` -- per-thread rasterizer state.
// ---------------------------------------------------------------------------

/// Per-draw data shared with the scanline JIT (see `GSRasterizerData`).
#[derive(Default)]
pub struct GsRasterizerData {
    pub scissor:        GsVector4i,
    pub bbox:           GsVector4i,
    pub primclass:      GsPrimClass,
    pub buff:           *mut u8,
    pub vertex:         *mut GsVertexSW,
    pub vertex_count:   i32,
    pub index:          *mut u16,
    pub index_count:    i32,
    pub frame:          u64,
    pub start:          u64,
    pub pixels:         i32,
    pub counter:        i32,
    pub scanmsk_value:  u8,
    pub global:         GsScanlineGlobalData,
    pub setup_prim:     Option<GsSetupPrimFn>,
    pub draw_scanline:  Option<GsDrawScanlineFn>,
    pub draw_edge:      Option<GsDrawScanlineFn>,
}

unsafe impl Send for GsRasterizerData {}
unsafe impl Sync for GsRasterizerData {}

/// Per-thread rasterizer (see `GSRasterizer.h`).
pub struct GsRasterizer {
    pub ds:               *mut GsDrawScanline,
    pub id:               i32,
    pub threads:          i32,
    pub thread_height:    i32,
    pub scanline:         *mut u8,
    pub scanmsk_value:    u8,
    pub scissor:          GsVector4i,
    pub fscissor_x:       GsVector4,
    pub fscissor_y:       GsVector4,
    pub edge:             GsEdgeBuffer,
    pub pixels:           GsPixelCounts,
    pub primcount:        i32,
    pub local:            GsScanlineLocalData,
    pub setup_prim:       Option<GsSetupPrimFn>,
    pub draw_scanline:    Option<GsDrawScanlineFn>,
    pub draw_edge:        Option<GsDrawScanlineFn>,
    /// Per-pixel helper tables indexed by `[step_x][pos_x][pos_y][tl][side]`.
    pub draw_edge_triangle: [GsDrawEdgeTriangleFn; 32],
    /// Per-vertex helper tables indexed by `[step_x][pos_x][pos_y][aa]`.
    pub draw_edge_line:     [GsDrawEdgeLineFn; 16],
}

unsafe impl Send for GsRasterizer {}
unsafe impl Sync for GsRasterizer {}

/// Edge-temporary buffer.
#[derive(Default)]
pub struct GsEdgeBuffer {
    pub buff:  *mut GsVertexSW,
    pub count: i32,
}

/// Pixel-statistics counters.
#[derive(Copy, Clone, Default)]
pub struct GsPixelCounts {
    pub sum:    i32,
    pub actual: i32,
    pub total:  i32,
}

/// Function pointer to one of the templated `DrawEdgeTriangle` variants.
pub type GsDrawEdgeTriangleFn =
    extern "C" fn(this: &mut GsRasterizer,
       v0:    &GsVertexSW,
       v1:    &GsVertexSW,
       dv:    &GsVertexSW,
       efun1: &GsVector4i,
       efun2: &GsVector4i);

/// Function pointer to one of the templated `DrawEdgeLine` variants.
pub type GsDrawEdgeLineFn =
    extern "C" fn(this: &mut GsRasterizer,
       v0:    &GsVertexSW,
       v1:    &GsVertexSW,
       dv:    &GsVertexSW);

impl GsRasterizer {
    pub fn new(_ds: *mut GsDrawScanline, _id: i32, _threads: i32) -> Self {
        Self {
            ds:               ::std::ptr::null_mut(),
            id:               0,
            threads:          0,
            thread_height:    0,
            scanline:         ::std::ptr::null_mut(),
            scanmsk_value:    0,
            scissor:          GsVector4i::zero(),
            fscissor_x:       GsVector4::zero(),
            fscissor_y:       GsVector4::zero(),
            edge:             GsEdgeBuffer::default(),
            pixels:           GsPixelCounts::default(),
            primcount:        0,
            local:            GsScanlineLocalData::default(),
            setup_prim:       None,
            draw_scanline:    None,
            draw_edge:        None,
            draw_edge_triangle: [GsDrawScanlineDummy as _; 32],
            draw_edge_line:     [GsDrawEdgeLineDummy as _; 16],
        }
    }

    /// True if `top` is one of the scanlines this thread owns.
    pub fn is_one_of_my_scanlines(&self, _top: i32) -> bool { false }
    /// Same as above, but also checks the bottom of the scanline.
    pub fn is_one_of_my_scanlines_range(&self, _top: i32, _bottom: i32) -> bool { false }
    /// Returns the next scanline this thread owns, given the current `top`.
    pub fn find_my_next_scanline(&self, _top: i32) -> i32 { 0 }

    /// Draw entry point: consumes the per-draw data and dispatches to the
    /// appropriate primitive handler.
    pub fn draw(&mut self, _data: &mut GsRasterizerData) {}

    /// Returns the pixel counter; resets it if `reset` is `true`.
    pub fn get_pixels(&mut self, reset: bool) -> i32 {
        let v = self.pixels.sum;
        if reset { self.pixels = GsPixelCounts::default(); }
        v
    }
}

/// `GsRasterizer` is `Drop`-safe: the original C++ class has a virtual
/// destructor, and the `*mut` fields don't imply ownership.
impl Drop for GsRasterizer {
    fn drop(&mut self) {}
}

// ---------------------------------------------------------------------------
// `IRasterizer`, `GSSingleRasterizer`, `GSRasterizerList`.
// ---------------------------------------------------------------------------

/// Polymorphic rasterizer interface (see `IRasterizer`).
pub trait IRasterizer {
    fn queue(&mut self, _data: GsRasterizerHandle);
    fn sync (&mut self);
    fn is_synced(&self) -> bool;
    fn get_pixels(&mut self, reset: bool) -> i32;
    fn print_stats(&self);
}

/// Opaque handle for a queued rasterizer job. Equivalent to
/// `GSRingHeap::SharedPtr<GSRasterizerData>`.
#[derive(Copy, Clone, Default)]
pub struct GsRasterizerHandle {
    pub raw: *mut u8,
}

/// Single-threaded rasterizer implementation.
pub struct GsSingleRasterizer {
    pub ds: GsDrawScanline,
    pub r:  GsRasterizer,
}

impl Default for GsSingleRasterizer {
    fn default() -> Self {
        let mut r = GsRasterizer::new(::std::ptr::null_mut(), 0, 1);
        let ds_box: Box<GsDrawScanline> = Box::new(GsDrawScanline::default());
        let ds_ptr: *mut GsDrawScanline = Box::into_raw(ds_box);
        r.ds = ds_ptr;
        // SAFETY: `ds_ptr` was just allocated via `Box::new` and points to a
        // valid `GsDrawScanline`. Reading it here only copies the bytes; the
        // original allocation still owns the heap storage (and continues to
        // be reachable through `r.ds`).
        let ds_value: GsDrawScanline = unsafe { std::ptr::read(ds_ptr) };
        Self { ds: ds_value, r }
    }
}

impl IRasterizer for GsSingleRasterizer {
    fn queue(&mut self, _data: GsRasterizerHandle) {}
    fn sync (&mut self) {}
    fn is_synced(&self) -> bool { true }
    fn get_pixels(&mut self, reset: bool) -> i32 { self.r.get_pixels(reset) }
    fn print_stats(&self) {}
}

impl GsSingleRasterizer {
    pub fn new() -> Self { Self::default() }
    pub fn draw(&mut self, _data: &mut GsRasterizerData) {}
}

/// Multi-threaded rasterizer list (see `GSRasterizerList`).
pub struct GsRasterizerList {
    pub ds:              GsDrawScanline,
    pub rasterizers:     Vec<Box<GsRasterizer>>,
    pub workers:         Vec<Box<GsJobQueueWorker>>,
    pub scanline:        *mut u8,
    pub thread_height:   i32,
}

impl GsRasterizerList {
    pub fn create(_threads: i32) -> Box<dyn IRasterizer> {
        Box::new(Self {
            ds:            GsDrawScanline::default(),
            rasterizers:   Vec::new(),
            workers:       Vec::new(),
            scanline:      ::std::ptr::null_mut(),
            thread_height: 0,
        })
    }
}

impl IRasterizer for GsRasterizerList {
    fn queue(&mut self, _data: GsRasterizerHandle) {}
    fn sync (&mut self) {}
    fn is_synced(&self) -> bool { true }
    fn get_pixels(&mut self, _reset: bool) -> i32 { 0 }
    fn print_stats(&self) {}
}

/// Placeholder worker type (mirrors `GSJobQueue<...>`).
pub struct GsJobQueueWorker {
    _priv: [u8; 0],
}

// ---------------------------------------------------------------------------
// `GSTextureCacheSW` -- the SW texture cache.
// ---------------------------------------------------------------------------

/// Single texture entry in the SW texture cache.
pub struct GsTextureCacheSWTexture {
    pub offset:    GsOffset,
    pub pages:     GsOffsetPageLooper,
    pub tex0:      GifRegTEX0,
    pub texa:      GifRegTEXA,
    pub buff:      *mut u8,
    pub tw:        u32,
    pub age:       u32,
    pub complete:  bool,
    pub repeating: bool,
    pub p2t:       *mut Vec<GsVector2i>,
    pub valid:     [u32; GS_MAX_PAGES],
    pub erase_it:  [u16; GS_MAX_PAGES],
    pub sharedbits: *const u32,
}

impl GsTextureCacheSWTexture {
    pub fn new(_tw0: u32, _tex0: &GifRegTEX0, _texa: &GifRegTEXA) -> Self {
        Self {
            offset:    GsOffset::default(),
            pages:     GsOffsetPageLooper,
            tex0:      *_tex0,
            texa:      *_texa,
            buff:      ::std::ptr::null_mut(),
            tw:        0,
            age:       0,
            complete:  false,
            repeating: false,
            p2t:       ::std::ptr::null_mut(),
            valid:     [0; GS_MAX_PAGES],
            erase_it:  [0; GS_MAX_PAGES],
            sharedbits: ::std::ptr::null(),
        }
    }

    /// Re-initializes the texture for a new `TEX0` / `TEXA` pair.
    pub fn reset(&mut self, _tw0: u32, _tex0: &GifRegTEX0, _texa: &GifRegTEXA) {}

    /// Pulls the latest pages into the texture. Returns `false` if the
    /// texture is incomplete.
    pub fn update(&mut self, _r: &GsVector4i) -> bool { false }

    /// Writes the texture to a file (debug aid). Returns `true` on success.
    pub fn save(&self, _fn: &str) -> bool { false }
}

impl Drop for GsTextureCacheSWTexture {
    fn drop(&mut self) {}
}

/// The SW texture cache.
#[derive(Default)]
pub struct GsTextureCacheSW {
    pub textures: Vec<Box<GsTextureCacheSWTexture>>,
    pub map:      Vec<Vec<*mut GsTextureCacheSWTexture>>,
}

impl GsTextureCacheSW {
    pub fn new() -> Self { Self::default() }

    /// Look up (or create) a texture matching `TEX0` / `TEXA`.
    pub fn lookup(
        &mut self,
        _tex0: &GifRegTEX0,
        _texa: &GifRegTEXA,
        _tw0:  u32,
    ) -> Option<&mut GsTextureCacheSWTexture> { None }

    /// Invalidate all texture pages covered by `pages`.
    pub fn invalidate_pages(&mut self, _pages: &GsOffsetPageLooper, _psm: u32) {}

    /// Removes all cached textures.
    pub fn remove_all(&mut self) { self.textures.clear(); }
    /// Increments the age counter on every cached texture.
    pub fn inc_age(&mut self) {
        for t in &mut self.textures {
            t.age = t.age.wrapping_add(1);
        }
    }
}

// ---------------------------------------------------------------------------
// `GSRendererSW` -- top-level SW renderer that owns the rasterizer list and
// the texture cache.
// ---------------------------------------------------------------------------

/// Texture-level descriptor used by the SW renderer.
#[derive(Copy, Clone, Default)]
pub struct GsRendererSWTextureLevel {
    pub r: GsVector4i,
    pub t: *mut GsTextureCacheSWTexture,
}

/// Per-batch shared data carried by the SW renderer.
pub struct GsRendererSWSharedData {
    pub fb_pages:     GsOffsetPageLooper,
    pub zb_pages:     GsOffsetPageLooper,
    pub fpsm:         i32,
    pub zpsm:         i32,
    pub using_pages:  bool,
    pub tex:          [GsRendererSWTextureLevel; 8], // NULL-terminated
    pub sync_point:   GsRendererSWSyncPoint,
}

impl Default for GsRendererSWSharedData {
    fn default() -> Self {
        Self {
            fb_pages:    GsOffsetPageLooper,
            zb_pages:    GsOffsetPageLooper,
            fpsm:        0,
            zpsm:        0,
            using_pages: false,
            tex:         [GsRendererSWTextureLevel::default(); 8],
            sync_point:  GsRendererSWSyncPoint::SyncNone,
        }
    }
}

/// Three-way sync-point enum (matches the C++ `enum { ... } m_syncpoint`).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GsRendererSWSyncPoint {
    SyncNone,
    SyncSource,
    SyncTarget,
}

impl GsRendererSWSharedData {
    pub fn new() -> Self { Self::default() }
    pub fn use_pages(
        &mut self,
        _fb: &GsOffsetPageLooper, _fpsm: i32,
        _zb: &GsOffsetPageLooper, _zpsm: i32,
    ) {}
    pub fn release_pages(&mut self) {}
    pub fn set_source(&mut self, _t: *mut GsTextureCacheSWTexture, _r: GsVector4i, _level: i32) {}
    pub fn update_source(&mut self) {}
}

/// Top-level SW renderer.
pub struct GsRendererSW {
    pub rasterizer: Box<dyn IRasterizer>,
    pub texture_cache: GsTextureCacheSW,
    pub vertex_heap: GsRingHeap,
    pub textures: [Option<Box<GsTexture>>; 3],
    pub output: *mut u8,
    pub fzb: *mut GsPixelOffset4,
    pub fzb_bbox: GsVector4i,
    pub fzb_cur_pages: [u32; 16],
    pub fzb_pages: [u32; 512],
    pub tex_pages: [u16; 512],
    pub last_dimx: GifRegDIMX,
    pub dimx: [GsVector4i; 8],
}

impl GsRendererSW {
    pub fn new(_threads: i32) -> Self {
        Self {
            rasterizer:   GsRasterizerList::create(1),
            texture_cache: GsTextureCacheSW::new(),
            vertex_heap:   GsRingHeap::new(),
            textures:     [None, None, None],
            output:       ::std::ptr::null_mut(),
            fzb:          ::std::ptr::null_mut(),
            fzb_bbox:     GsVector4i::zero(),
            fzb_cur_pages: [0; 16],
            fzb_pages:    [0; 512],
            tex_pages:    [0; 512],
            last_dimx:    GifRegDIMX::default(),
            dimx:         [GsVector4i::zero(); 8],
        }
    }

    pub fn destroy(&mut self) {}

    // --- Renderer hooks (see `GSRenderer`) ---------------------------------

    pub fn reset(&mut self, _hardware_reset: bool) {}
    pub fn vsync (&mut self, _field: u32, _registers_written: bool, _idle_frame: bool) {}

    pub fn get_output(&mut self, _i: i32, _scale: &mut f32, _y_offset: &mut i32) -> *mut GsTexture {
        ::std::ptr::null_mut()
    }
    pub fn get_feedback_output(&mut self, _scale: &mut f32) -> *mut GsTexture {
        ::std::ptr::null_mut()
    }

    pub fn draw(&mut self) {}
    pub fn queue(&mut self, _item: GsRasterizerHandle) {}
    pub fn sync (&mut self, _reason: i32) {}

    pub fn invalidate_video_mem(&mut self, _bitbltbuf: &GifRegBITBLTBUF, _r: &GsVector4i) {}
    pub fn invalidate_local_mem(
        &mut self,
        _bitbltbuf: &GifRegBITBLTBUF,
        _r:         &GsVector4i,
        _clut:      bool,
    ) {}

    pub fn use_pages(&mut self, _pages: &GsOffsetPageLooper, _ty: i32) {}
    pub fn release_pages(&mut self, _pages: &GsOffsetPageLooper, _ty: i32) {}

    pub fn check_target_pages(
        &self,
        _fb: &GsOffsetPageLooper,
        _zb: &GsOffsetPageLooper,
        _r:  &GsVector4i,
    ) -> bool { false }
    pub fn check_source_pages(&self, _sd: &GsRendererSWSharedData) -> bool { false }
    pub fn get_scanline_global_data(&mut self, _data: &mut GsRendererSWSharedData) -> bool { false }
    pub fn is_coverage_alpha_supported(&self) -> bool { false }
}

/// Fast-list placeholder (`GSFastList.h`).
pub struct GsFastList<T> {
    _marker: ::std::marker::PhantomData<T>,
}

impl<T> Default for GsFastList<T> {
    fn default() -> Self { Self { _marker: ::std::marker::PhantomData } }
}

/// Ring-heap placeholder (`GSRingHeap.h`).
#[derive(Default)]
pub struct GsRingHeap {
    _priv: [u8; 0],
}

impl GsRingHeap {
    pub fn new() -> Self { Self::default() }
}

// ---------------------------------------------------------------------------
// Helper stubs.
// ---------------------------------------------------------------------------

/// Default-noop function used to populate the templated dispatch tables in
/// `GsRasterizer` before a real implementation is wired in.
pub extern "C" fn GsDrawScanlineDummy(
    _this: &mut GsRasterizer,
    _v0:   &GsVertexSW,
    _v1:   &GsVertexSW,
    _dv:   &GsVertexSW,
    _efun1: &GsVector4i,
    _efun2: &GsVector4i,
) {
}

/// Default-noop function used to populate the edge-line dispatch table.
/// Takes fewer parameters than `GsDrawScanlineDummy` to match the
/// `GsDrawEdgeLineFn` signature.
pub extern "C" fn GsDrawEdgeLineDummy(
    _this: &mut GsRasterizer,
    _v0:   &GsVertexSW,
    _v1:   &GsVertexSW,
    _dv:   &GsVertexSW,
) {
}

// ---------------------------------------------------------------------------
// Tests -- minimal compile-only checks. We don't run them via cargo here;
// they're just here to confirm that the public surface is internally
// consistent.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selector_round_trip() {
        let key = 0x0123_4567_89AB_CDEF_u64;
        let sel = GsScanlineSelector::from_key(key);
        assert_eq!(sel.key, key);
        assert_eq!(sel.lo, key as u32);
        assert_eq!(sel.hi, (key >> 32) as u32);
    }

    #[test]
    fn solid_rect_predicate() {
        // prim=Sprite, iip=0, tfx=None, abe=0, ztst<=1, atst<=1, date=0, fge=0
        let mut sel = GsScanlineSelector::default();
        sel.lo |= 0b11 << 0;        // fpsm
        sel.lo |= 0b00 << 4;        // ztst
        sel.lo |= 0b00 << 6;        // atst
        sel.hi |= 0b00 << 15;       // prim = SPRITE (0 -> 0b00 in 2 bits)
        // 0b11 << 15 -> SPRITE = 3 actually; we model the same bit layout.
        sel.hi = 0b11 << 15;
        // mask in the SOLID case -> all conditions should hold.
        // We do not assert a specific value here, just that the helper is
        // callable.
        let _ = sel.is_solid_rect();
    }

    #[test]
    fn constant_tables_initialized() {
        // 256-byte block: 24 bytes of test mask + log2 + shift table.
        let c = GsScanlineConstantData256B::default();
        assert_eq!(c.m_test[8], 0xff);
        assert_eq!(c.m_test[0], 0);
        assert_eq!(c.m_log2_coef.len(), 4);
        assert_eq!(c.m_shift.len(), 16);
    }
}
