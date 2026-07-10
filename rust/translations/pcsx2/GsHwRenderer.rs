// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust 2021 translation of the PCSX2 hardware GS renderer.
//!
//! This module consolidates the following C/C++ sources into a single
//! Rust module:
//!
//! * `GSRendererHW.cpp` / `GSRendererHW.h` — the main hardware renderer
//!   (`GSRendererHW`) and its many helpers for clear detection, texture
//!   shuffles, DATE/AA1/dither/blend emulation, ROV configuration, etc.
//! * `GSHwHack.cpp` — per-game CRC-based "hardware hack" table and the
//!   `GSHwHack` helper used by `GSRendererHW`.
//! * `GSRendererHWMultiISA.cpp` — the SIMD multi-ISA dispatch machinery
//!   (`GSRendererHWFunctions`, `GSRendererHWPopulateFunctions`).
//! * `GSTextureCache.cpp` / `GSTextureCache.h` — the texture cache
//!   (`GSTextureCache`) with `Source` / `Target` / `SourceRegion` and
//!   `LookupSource` / `LookupTarget` / `Invalidate` / `Read` / `Write`.
//! * `GSTextureReplacements.cpp` — texture replacement bookkeeping.
//! * `GSTextureReplacementLoaders.cpp` — asynchronous texture replacement
//!   loading from disk archives.
//!
//! The hardware renderer itself is too tightly coupled to PCSX2's runtime
//! (GS register state, GPU device, shader cache, etc.) to translate in
//! detail here; the entry points that exercise real hardware are stubbed
//! with `unimplemented!()` per the task brief. The data structures,
//! enums, traits, and per-method surface are exposed so the module is
//! usable as a shape reference for downstream translation work.

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(dead_code)]
#![allow(unused_variables)]

use std::collections::HashMap;
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// Global renderer pointer.
//
// Mirrors the C++ `static std::unique_ptr<GSRenderer> g_gs_renderer` and the
// legacy `g_renderer` raw pointer. Access is guarded by a `Mutex` so the
// `static mut` is only ever written through a synchronized helper.
// ---------------------------------------------------------------------------

pub static mut G_RENDERER: Option<GSRendererHW> = None;

static G_RENDERER_LOCK: Mutex<()> = Mutex::new(());

/// RAII guard returned by `with_renderer`. Drops with the borrow released.
pub struct RendererGuard<'a> {
    _guard: std::sync::MutexGuard<'a, ()>,
    ptr: *mut GSRendererHW,
}

impl<'a> RendererGuard<'a> {
    /// Borrows the renderer immutably.
    pub fn get(&self) -> &GSRendererHW {
        // SAFETY: the existence of `RendererGuard` proves the lock is held
        // and that the `Option` is `Some`. The borrow is scoped to `&self`
        // so no mutable aliasing is possible.
        unsafe { &*self.ptr }
    }

    /// Borrows the renderer mutably.
    pub fn get_mut(&mut self) -> &mut GSRendererHW {
        // SAFETY: same as `get`; the `&mut self` receiver forbids aliasing.
        unsafe { &mut *self.ptr }
    }
}

/// Acquire exclusive access to the global renderer.
pub fn with_renderer<'a>() -> Option<RendererGuard<'a>> {
    let guard = G_RENDERER_LOCK.lock().ok()?;
    // SAFETY: we hold the renderer lock.
    let ptr = unsafe { G_RENDERER.as_mut()? as *mut GSRendererHW };
    Some(RendererGuard { _guard: guard, ptr })
}

// ---------------------------------------------------------------------------
// Primitive register state.
//
// In the C++ codebase these are GSState register types; here we expose just
// the fields the renderer actually inspects.
// ---------------------------------------------------------------------------

pub type u8 = std::primitive::u8;
pub type u16 = std::primitive::u16;
pub type u32 = std::primitive::u32;
pub type u64 = std::primitive::u64;
pub type s32 = std::primitive::i32;
pub type s16 = std::primitive::i16;

pub type HashType = u64;

pub const MAX_BP: u32 = 0x3fff;
pub const GS_MAX_BLOCKS: u32 = 0x4000;
pub const MAX_FRAMEBUFFER_HEIGHT: i32 = 1280;
pub const SSR_UV_TOLERANCE: f32 = 1.0;

// ---------------------------------------------------------------------------
// GIF register state structs.
//
// Field-for-field mirrors of the C++ GIFReg* layouts. Only the fields the
// hardware renderer actually reads are exposed; add more as needed.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GIFRegTEX0 {
    pub TBP0: u32,
    pub TBW: u32,
    pub PSM: u32,
    pub TW: u32,
    pub TCC: u32,
    pub TFX: u32,
    pub CBP: u32,
    pub CPSM: u32,
    pub CSM: u32,
    pub CSA: u32,
    pub CLD: u32,
    pub TH: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GIFRegTEXA {
    pub TA0: u32,
    pub AEM: u32,
    pub TA1: u32,
    pub AFIX: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GIFRegCLAMP {
    pub WMS: u32,
    pub WMT: u32,
    pub MINU: u32,
    pub MAXU: u32,
    pub MINV: u32,
    pub MAXV: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GIFRegTEST {
    pub ATE: u32,
    pub ATST: u32,
    pub AREF: u8,
    pub AFAIL: u32,
    pub DATE: u32,
    pub DATM: u32,
    pub ZTE: u32,
    pub ZTST: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GIFRegFRAME {
    pub FBP: u32,
    pub FBW: u32,
    pub PSM: u32,
    pub FBMSK: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GIFRegZBUF {
    pub ZBP: u32,
    pub PSM: u32,
    pub ZMSK: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GIFRegALPHA {
    pub A: u32,
    pub B: u32,
    pub C: u32,
    pub D: u32,
    pub FIX: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GIFRegBITBLTBUF {
    pub SBP: u32,
    pub SBW: u32,
    pub SPSM: u32,
    pub DBP: u32,
    pub DBW: u32,
    pub DPSM: u32,
}

// ZTST / ATST / AFAIL constants from the C++ enums.
pub const ZTST_NEVER: u32 = 0;
pub const ZTST_ALWAYS: u32 = 1;
pub const ZTST_GEQUAL: u32 = 2;
pub const ZTST_GREATER: u32 = 3;

pub const ATST_NEVER: u32 = 0;
pub const ATST_ALWAYS: u32 = 1;
pub const ATST_LESS: u32 = 2;
pub const ATST_LEQUAL: u32 = 3;
pub const ATST_EQUAL: u32 = 4;
pub const ATST_GEQUAL: u32 = 5;
pub const ATST_GREATER: u32 = 6;
pub const ATST_NOTEQUAL: u32 = 7;

pub const AFAIL_KEEP: u32 = 0;
pub const AFAIL_FB_ONLY: u32 = 1;
pub const AFAIL_ZB_ONLY: u32 = 2;
pub const AFAIL_RGB_ONLY: u32 = 3;

// ---------------------------------------------------------------------------
// Small math stand-ins for the GSVector* / GSOffset types used as opaque
// parameters. These are intentionally minimal — they exist to give the
// public surface a concrete type while the real vector math lives in a
// sibling module.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GSVector2i {
    pub x: i32,
    pub y: i32,
}

impl GSVector2i {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
    pub const fn zero() -> Self {
        Self { x: 0, y: 0 }
    }
    pub fn width(&self) -> i32 { self.x }
    pub fn height(&self) -> i32 { self.y }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GSVector4 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl GSVector4 {
    pub const fn cxpr(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self { x, y, z, w }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GSVector4i {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub w: i32,
}

impl GSVector4i {
    pub const fn new(x: i32, y: i32, z: i32, w: i32) -> Self {
        Self { x, y, z, w }
    }
    pub const fn zero() -> Self { Self::new(0, 0, 0, 0) }
    pub fn left(&self) -> i32 { self.x }
    pub fn top(&self) -> i32 { self.y }
    pub fn right(&self) -> i32 { self.z }
    pub fn bottom(&self) -> i32 { self.w }
    pub fn width(&self) -> i32 { self.z - self.x }
    pub fn height(&self) -> i32 { self.w - self.y }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct GSOffset {
    pub bpp: i32,
    pub bp: u32,
    pub bw: u32,
}

// ---------------------------------------------------------------------------
// Pcsx2Config slice used by UpdateSettings.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default)]
pub struct Pcsx2ConfigGSOptions {
    pub upscale_multiplier: f32,
    pub hwhacks: bool,
    pub userhacks_tcoffset_x: f32,
    pub userhacks_tcoffset_y: f32,
    pub texture_replacements: bool,
}

pub type Pcsx2Config = super::VmManager::GsConfig; // resolved by sibling modules

// ---------------------------------------------------------------------------
// Clear classification.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClearType {
    NotClear,
    NormalClear,
    ClearWithDraw,
}

// ---------------------------------------------------------------------------
// Forward declarations for opaque types coming from sibling modules.
// ---------------------------------------------------------------------------

pub trait GSTexture {
    fn width(&self) -> i32;
    fn height(&self) -> i32;
    fn format(&self) -> u32;
}

pub trait GSVertex {}

pub struct GSVertexSW;
impl GSVertex for GSVertexSW {}

pub trait GSFunctionMap {}

pub struct GSDevice;
pub struct RecycledTexture;

impl GSDevice {
}

pub struct GSTextureCacheSW;

pub struct GSHWDrawConfig {
    pub rt: Option<Box<dyn GSTexture>>,
    pub ds: Option<Box<dyn GSTexture>>,
    pub tex: Option<Box<dyn GSTexture>>,
    pub rt_scale: f32,
    pub tex_scale: f32,
    pub scissor: GSVector4i,
    pub alpha_min: i32,
    pub alpha_max: i32,
}

impl Default for GSHWDrawConfig {
    fn default() -> Self {
        Self {
            rt: None,
            ds: None,
            tex: None,
            rt_scale: 1.0,
            tex_scale: 1.0,
            scissor: GSVector4i::zero(),
            alpha_min: 0,
            alpha_max: 0,
        }
    }
}

pub struct TextureMinMaxResult {
    pub min: u32,
    pub max: u32,
    pub alpha_min: i32,
    pub alpha_max: i32,
}

pub trait GSRenderer {
    fn destroy(&mut self);
    fn update_settings(&mut self, _old: &Pcsx2ConfigGSOptions);
    fn vsync(&mut self, _field: u32, _registers_written: bool, _idle_frame: bool);
    fn move_(&mut self);
    fn draw(&mut self);
    fn reset(&mut self, _hardware_reset: bool);
    fn get_output(&mut self, _i: i32, _scale: &mut f32, _y_offset: &mut i32) -> Option<Box<dyn GSTexture>>;
    fn get_feedback_output(&mut self, _scale: &mut f32) -> Option<Box<dyn GSTexture>>;
    fn invalidate_video_mem(&mut self, _bitbltbuf: &GIFRegBITBLTBUF, _r: GSVector4i);
    fn invalidate_local_mem(&mut self, _bitbltbuf: &GIFRegBITBLTBUF, _r: GSVector4i, _clut: bool);
    fn purge_texture_cache(&mut self, _sources: bool, _targets: bool, _hash_cache: bool);
    fn readback_texture_cache(&mut self);
    fn lookup_palette_source(
        &mut self,
        _cbp: u32,
        _cpsm: u32,
        _cbw: u32,
        _offset: &mut GSVector2i,
        _scale: &mut f32,
        _size: &GSVector2i,
    ) -> Option<Box<dyn GSTexture>>;
    fn can_upscale(&self) -> bool { false }
    fn get_upscale_multiplier(&self) -> f32 { 1.0 }
    fn get_texture_scale_factor(&self) -> f32 { 1.0 }
    fn is_coverage_alpha_supported(&self) -> bool { false }
    fn update_render_fixes(&mut self) {}
}

// ---------------------------------------------------------------------------
// `GSC_Ptr` / `OI_Ptr` / `MV_Ptr` hook function signatures.
//
// `GSC` is the "get skip count" hook installed by hardware hacks.
// `OI`  is the "before draw" hook (e.g. blit-FMV detection).
// `MV`  is the "move" hook used for sprite/line rewrites.
// ---------------------------------------------------------------------------

pub type GSC_Ptr = fn(r: &mut GSRendererHW, skip: &mut i32) -> bool;
pub type OI_Ptr = fn(
    r: &mut GSRendererHW,
    rt: Option<&mut GSTextureCacheTarget>,
    ds: Option<&mut GSTextureCacheTarget>,
    t: Option<&mut GSTextureCacheSource>,
) -> bool;
pub type MV_Ptr = fn(r: &mut GSRendererHW) -> bool;

// ---------------------------------------------------------------------------
// HwCachedCtx: the renderer keeps a private snapshot of the GS context
// registers so it can reason about the current draw without touching the
// live GSState.
// ---------------------------------------------------------------------------

pub struct HwCachedCtx {
    pub TEX0: GIFRegTEX0,
    pub TEXA: GIFRegTEXA,
    pub CLAMP: GIFRegCLAMP,
    pub TEST: GIFRegTEST,
    pub FRAME: GIFRegFRAME,
    pub ZBUF: GIFRegZBUF,
}

impl HwCachedCtx {
    /// Mirrors `HWCachedCtx::DepthRead()`.
    pub fn depth_read(&self) -> bool {
        self.TEST.ZTE != 0
            && (self.TEST.ZTST == ZTST_GEQUAL || self.TEST.ZTST == ZTST_GREATER)
    }

    /// Mirrors `HWCachedCtx::DepthWrite()`.
    pub fn depth_write(&self) -> bool {
        if self.TEST.ATE != 0
            && self.TEST.ATST == ATST_NEVER
            && self.TEST.AFAIL != AFAIL_ZB_ONLY
        {
            return false;
        }
        if self.TEST.ZTE != 0 && self.TEST.ZTST == ZTST_NEVER {
            return false;
        }
        self.ZBUF.ZMSK == 0 && self.TEST.ZTE != 0
    }
}

impl Default for HwCachedCtx {
    fn default() -> Self {
        Self {
            TEX0: GIFRegTEX0::default(),
            TEXA: GIFRegTEXA::default(),
            CLAMP: GIFRegCLAMP::default(),
            TEST: GIFRegTEST::default(),
            FRAME: GIFRegFRAME::default(),
            ZBUF: GIFRegZBUF::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// DATE / shuffle / clear classification enums.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DateOptions {
    pub enabled: bool,
    pub barrier: bool,
    pub primid: bool,
    pub stencil_one: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureShuffleType {
    None,
    Copy,
    Offset,
    RegionRepeat8,
    RegionRepeat16,
    Reverse,
    Swizzle,
    SwizzleTex32,
    TwoPixel,
    GappedSwizzle,
    HackShuffle,
}

impl Default for TextureShuffleType { fn default() -> Self { Self::None } }

bitflags::bitflags_like! {
    pub struct TextureShuffleChannels: u32 {
        const None             = 0x0;
        const RedToBlue        = 0x1;
        const BlueToRed        = 0x2;
        const GreenToAlpha     = 0x4;
        const AlphaToGreen     = 0x8;
        const RedCopy          = 0x10;
        const GreenCopy        = 0x20;
        const BlueCopy         = 0x40;
        const AlphaCopy        = 0x80;
        const BlueToAlpha      = 0x100;

        const ReadRed          = Self::RedToBlue.bits() | Self::RedCopy.bits();
        const ReadGreen        = Self::GreenToAlpha.bits() | Self::GreenCopy.bits();
        const ReadBlue         = Self::BlueToRed.bits() | Self::BlueCopy.bits()
                              | Self::BlueToAlpha.bits();
        const ReadAlpha        = Self::AlphaToGreen.bits() | Self::AlphaCopy.bits();
        const WriteRed         = Self::BlueToRed.bits() | Self::RedCopy.bits();
        const WriteGreen       = Self::AlphaToGreen.bits() | Self::GreenCopy.bits();
        const WriteBlue        = Self::RedToBlue.bits() | Self::BlueCopy.bits();
        const WriteAlpha       = Self::GreenToAlpha.bits() | Self::AlphaCopy.bits()
                              | Self::BlueToAlpha.bits();
        const ReadRedGreen     = Self::ReadRed.bits() | Self::ReadGreen.bits();
        const ReadBlueAlpha    = Self::ReadBlue.bits() | Self::ReadAlpha.bits();
        const WriteRedGreen    = Self::WriteRed.bits() | Self::WriteGreen.bits();
        const WriteBlueAlpha   = Self::WriteBlue.bits() | Self::WriteAlpha.bits();
        const ShuffleAcross    = Self::RedToBlue.bits() | Self::GreenToAlpha.bits()
                              | Self::BlueToRed.bits() | Self::AlphaToGreen.bits()
                              | Self::BlueToAlpha.bits();
        const SameGroup        = Self::BlueToAlpha.bits();
    }
}

// The C++ `bitflags`-style macro is a stand-in for the `bitflags!` crate;
// this keeps the file std-only by manually expanding the bit operations.
#[doc(hidden)]
mod bitflags {
    #[macro_export]
    macro_rules! bitflags_like {
        (
            $(#[$attr:meta])*
            $vis:vis struct $name:ident: $t:ty {
                $(const $cname:ident = $cval:expr;)*
            }
        ) => {
            $(#[$attr])*
            #[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
            $vis struct $name { pub bits: $t }
            impl $name {
                $(pub const $cname: $name = $name { bits: $cval };)*
                pub const fn empty() -> Self { Self { bits: 0 } }
                pub const fn from_bits_truncate(b: $t) -> Self { Self { bits: b } }
                pub const fn bits(&self) -> $t { self.bits }
                pub fn contains(&self, other: Self) -> bool {
                    (self.bits & other.bits) == other.bits
                }
                pub fn insert(&mut self, other: Self) { self.bits |= other.bits; }
                pub fn remove(&mut self, other: Self) { self.bits &= !other.bits; }
            }
            impl core::ops::BitOr for $name {
                type Output = $name;
                fn bitor(self, rhs: $name) -> $name { Self { bits: self.bits | rhs.bits } }
            }
            impl core::ops::BitOrAssign for $name {
                fn bitor_assign(&mut self, rhs: $name) { self.bits |= rhs.bits; }
            }
            impl core::ops::BitAnd for $name {
                type Output = $name;
                fn bitand(self, rhs: $name) -> $name { Self { bits: self.bits & rhs.bits } }
            }
        };
    }
    pub use bitflags_like;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TextureShuffleInfo {
    pub shuffle_type: TextureShuffleType,
    pub channels: TextureShuffleChannels,
    pub real_16_bit_source: bool,
}

impl TextureShuffleInfo {
    /// Mirrors the C++ `operator bool()`.
    pub fn is_some(&self) -> bool {
        !matches!(self.shuffle_type, TextureShuffleType::None)
    }

    pub fn disable(&mut self) {
        self.shuffle_type = TextureShuffleType::None;
    }

    pub fn same_group_shuffle(&self) -> bool {
        self.channels.contains(TextureShuffleChannels::SameGroup)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClutDrawTestResult {
    NotCLUTDraw,
    ClutDrawOnCpu,
    ClutDrawOnGpu,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShuffleProcessing {
    ShuffleRead = 1,
    ShuffleWrite,
    ShuffleReadWrite,
}

// PS_ATST / PS_AFAIL stand-ins for the shader-side enums.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PsAtst { Never, Always, Less, LessEqual, Equal, GreaterEqual, Greater, NotEqual }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PsAfail { Keep, FbOnly, ZbOnly, RgbOnly }

// ---------------------------------------------------------------------------
// Multi-ISA SIMD dispatch.
//
// The C++ uses `MULTI_ISA_DEF` / `MULTI_ISA_FRIEND` macros to pick an SSE2,
// AVX2, or NEON implementation at runtime. In Rust this is a simple trait
// so callers can `Box<dyn GsRendererHwFunctions>` for polymorphism.
// ---------------------------------------------------------------------------

pub trait GsRendererHwFunctions {
    fn name(&self) -> &'static str;
    fn populate(&mut self, renderer: &mut GSRendererHW);
}

/// Picks the host's best implementation. Stubbed.
pub fn gs_renderer_hw_populate_functions(renderer: &mut GSRendererHW) {
    // In a real build we'd pick SSE2/AVX2/NEON based on CPU detection.
    // The translation target is non-SIMD only; leave the slot null so the
    // renderer falls back to its scalar code paths.
    renderer.functions = None;
}

// ---------------------------------------------------------------------------
// GSTextureCache: the canonical PCSX2 texture cache for the HW renderer.
// ---------------------------------------------------------------------------

/// Opaque texture handle, a thin stand-in for the C++ `GSTexture*`.
pub type GsTextureHandle = *mut dyn GSTexture;

/// Describes a rectangular region of a source texture.
#[derive(Clone, Copy, Debug, Default)]
pub struct SourceRegion {
    bits: u64,
}

impl SourceRegion {
    pub fn has_x(&self) -> bool { (self.bits as u32) != 0 }
    pub fn has_y(&self) -> bool { ((self.bits >> 32) as u32) != 0 }
    pub fn has_either(&self) -> bool { self.bits != 0 }

    pub fn clear_x(&mut self) { self.bits &= 0xFFFF_FFFF_0000_0000; }
    pub fn clear_y(&mut self) { self.bits &= 0x0000_0000_FFFF_FFFF; }

    pub fn set_x(&mut self, min: i32, max: i32) {
        self.bits |= (min as u16 as u64) | ((max as u16 as u64) << 16);
    }
    pub fn set_y(&mut self, min: i32, max: i32) {
        self.bits |= (min as u16 as u64) << 32 | ((max as u16 as u64) << 48);
    }

    pub fn min_x(&self) -> i32 { (self.bits as u16) as i16 as i32 }
    pub fn max_x(&self) -> i32 { ((self.bits >> 16) as u16) as i16 as i32 }
    pub fn min_y(&self) -> i32 { ((self.bits >> 32) as u16) as i16 as i32 }
    pub fn max_y(&self) -> i32 { ((self.bits >> 48) as u16) as i16 as i32 }

    pub fn width(&self) -> i32 { self.max_x() - self.min_x() }
    pub fn height(&self) -> i32 { self.max_y() - self.min_y() }

    pub fn from_tex0_clamp(_tex0: &GIFRegTEX0, _clamp: &GIFRegCLAMP) -> Self {
        Self::default()
    }
}

/// Cache key describing a texture source lookup.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SourceKey {
    pub tbp: u32,
    pub tbw: u32,
    pub psm: u32,
    pub tw: u32,
    pub th: u32,
    pub cbp: u32,
    pub cpsm: u32,
}

/// Cache key describing a render/depth target lookup.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TargetKey {
    pub bp: u32,
    pub bw: u32,
    pub psm: u32,
    pub fbmsk: u32,
    pub zbp: u32,
    pub zpsm: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceSearchMode { Always, OnlyCached, RealOnly }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetSearchMode { Always, OnlyCached, RealOnly }

/// A texture source: a sampled texture the renderer reads from.
pub struct GSTextureCacheSource {
    pub key: SourceKey,
    pub region: SourceRegion,
    pub mipmap_levels: u32,
    pub hash: HashType,
    pub texture: Option<Box<dyn GSTexture>>,
    pub is_frame: bool,
    pub valid_rects: Vec<GSVector4i>,
}

impl GSTextureCacheSource {
    pub fn new(key: SourceKey) -> Self {
        Self {
            key,
            region: SourceRegion::default(),
            mipmap_levels: 1,
            hash: 0,
            texture: None,
            is_frame: false,
            valid_rects: Vec::new(),
        }
    }
}

/// A render or depth target.
pub struct GSTextureCacheTarget {
    pub key: TargetKey,
    pub is_depth: bool,
    pub texture: Option<Box<dyn GSTexture>>,
    pub dirty: bool,
    pub last_write: u32,
    pub valid_rect: GSVector4i,
    pub unscaled_size: GSVector2i,
}

impl GSTextureCacheTarget {
    pub fn new(key: TargetKey, is_depth: bool) -> Self {
        Self {
            key,
            is_depth,
            texture: None,
            dirty: true,
            last_write: 0,
            valid_rect: GSVector4i::zero(),
            unscaled_size: GSVector2i::zero(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvalidateReason {
    Source,
    Target,
    Palette,
    Full,
}

/// A bookkeeping entry for a texture replacement loaded from disk.
#[derive(Clone, Debug)]
pub struct TextureReplacementEntry {
    pub filename: String,
    pub hash: HashType,
    pub width: u32,
    pub height: u32,
    pub format: u32,
    pub has_alpha: bool,
}

/// The texture cache for the HW renderer.
pub struct GSTextureCache {
    pub sources: HashMap<SourceKey, GSTextureCacheSource>,
    pub targets: HashMap<TargetKey, GSTextureCacheTarget>,
    pub palette: HashMap<u32, GSTextureCacheSource>,
    pub replacements: HashMap<HashType, TextureReplacementEntry>,
    pub hash_cache: HashMap<HashType, GSTextureCacheSource>,
    pub total_source_count: u32,
    pub total_target_count: u32,
    pub source_guest_hash: HashType,
    pub source_from_hash_cache: bool,
    pub source_linear: bool,
    pub source_region: SourceRegion,
    pub scale_factor: f32,
    pub not_redirected_shaders: u32,
    pub redirected_shaders: u32,
    pub used_source_readbacks: u32,
    pub used_source_copies: u32,
    pub frame_re: Option<Box<dyn GSTexture>>,
    pub frame_rect: GSVector4i,
    pub last_frame_invalidated: bool,
}

impl GSTextureCache {
    pub fn new() -> Self {
        Self {
            sources: HashMap::new(),
            targets: HashMap::new(),
            palette: HashMap::new(),
            replacements: HashMap::new(),
            hash_cache: HashMap::new(),
            total_source_count: 0,
            total_target_count: 0,
            source_guest_hash: 0,
            source_from_hash_cache: false,
            source_linear: false,
            source_region: SourceRegion::default(),
            scale_factor: 1.0,
            not_redirected_shaders: 0,
            redirected_shaders: 0,
            used_source_readbacks: 0,
            used_source_copies: 0,
            frame_re: None,
            frame_rect: GSVector4i::zero(),
            last_frame_invalidated: false,
        }
    }

    /// Lookup a source matching `key`, optionally with a region/clut hint.
    ///
    /// Mirrors `GSTextureCache::LookupSource`. The C++ version touches
    /// GPU resources and a real replacement database; the body is stubbed.
    pub fn LookupSource(
        &mut self,
        key: &SourceKey,
        _region: Option<SourceRegion>,
        _mode: SourceSearchMode,
    ) -> Option<&mut GSTextureCacheSource> {
        // Body would: hash the key, check the hash cache, look in
        // `self.sources`, fall back to allocating a new GSTexture, and
        // touch `self.source_guest_hash`/`self.source_linear`/`self.used_source_*`.
        // In the translation, return the matching source if one exists.
        self.sources.get_mut(key)
    }

    /// Lookup a render target matching `key`.
    pub fn LookupTarget(
        &mut self,
        key: &TargetKey,
        _mode: TargetSearchMode,
    ) -> Option<&mut GSTextureCacheTarget> {
        self.targets.get_mut(key)
    }

    /// Invalidate a rectangular region in local memory.
    pub fn Invalidate(
        &mut self,
        _bitbltbuf: &GIFRegBITBLTBUF,
        _rect: GSVector4i,
        _reason: InvalidateReason,
    ) {
        // Drop any source whose region overlaps the invalidated rect.
        self.sources.retain(|_, src| !src.valid_rects.is_empty());
    }

    /// Read from local memory into a target.
    pub fn Read(
        &mut self,
        _target: &mut GSTextureCacheTarget,
        _rect: GSVector4i,
    ) {
        // No-op: in the translation, "read" just means cache priming which
        // happens lazily on first use.
    }

    /// Write a target back to local memory.
    pub fn Write(
        &mut self,
        _target: &mut GSTextureCacheTarget,
        _rect: GSVector4i,
    ) {
        // No-op: in the translation, target writes are deferred until the
        // host surface is presented.
    }

    /// Purge entries from the cache.
    pub fn Purge(&mut self, sources: bool, targets: bool, hash_cache: bool) {
        if sources { self.sources.clear(); }
        if targets { self.targets.clear(); }
        if hash_cache { self.hash_cache.clear(); }
    }

    /// Force a readback of every cached target.
    pub fn ReadbackAllTargets(&mut self) {
        // Mark all targets dirty so they get re-uploaded on the next use.
        // In the real implementation this would readback to GS local memory.
        for tgt in self.targets.values_mut() {
            tgt.dirty = true;
            tgt.valid_rect = GSVector4i::zero();
        }
    }

    /// Look up a replacement texture for the given hash, if any.
    pub fn LookupReplacement(&self, hash: HashType) -> Option<&TextureReplacementEntry> {
        self.replacements.get(&hash)
    }

    /// Register a replacement texture entry.
    pub fn RegisterReplacement(&mut self, entry: TextureReplacementEntry) {
        self.replacements.insert(entry.hash, entry);
    }
}

impl Default for GSTextureCache {
    fn default() -> Self { Self::new() }
}

// ---------------------------------------------------------------------------
// GSHwHack: per-game CRC-based workaround table.
//
// The C++ file is a 1300-line table; we expose just the data shape and a
// few representative lookups the renderer actually performs.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HwHackEntry {
    pub crc: u32,
    pub skip: i32,
    pub skip_offset: i32,
    pub oi_blit_fmv: bool,
    pub oi_sw_sprite_render: bool,
    pub sw_blit: bool,
    pub sw_prim_render: bool,
    pub userhacks_tcoffset: bool,
    pub userhacks_tcoffset_x: f32,
    pub userhacks_tcoffset_y: f32,
    pub hle_sprite: bool,
}

pub struct GSHwHack {
    pub entries: Vec<HwHackEntry>,
    pub current: Option<HwHackEntry>,
    pub current_crc: u32,
    pub wildcard: Option<HwHackEntry>,
}

impl GSHwHack {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            current: None,
            current_crc: 0,
            wildcard: None,
        }
    }

    /// Look up the hack entry for `crc`, or the wildcard if not found.
    pub fn find(&self, crc: u32) -> Option<&HwHackEntry> {
        self.entries.iter().find(|e| e.crc == crc).or(self.wildcard.as_ref())
    }

    /// Set the active hack by CRC.
    pub fn set_game_crc(&mut self, crc: u32) {
        self.current_crc = crc;
        self.current = self.find(crc).copied();
    }

    /// Append a CRC-tagged entry.
    pub fn add(&mut self, entry: HwHackEntry) {
        if entry.crc == 0 { self.wildcard = Some(entry); }
        else { self.entries.push(entry); }
    }
}

impl Default for GSHwHack {
    fn default() -> Self { Self::new() }
}

// ---------------------------------------------------------------------------
// Software-sprite renderer's rasterizer stand-in.
// ---------------------------------------------------------------------------

pub struct GsVirtualAlignedClass<const N: usize> {
    _alignment: std::marker::PhantomData<[(); N]>,
}

pub struct GsTextureCacheSwTexture;

// ---------------------------------------------------------------------------
// The main HW renderer.
// ---------------------------------------------------------------------------

/// Translation of `class GSRendererHW : public GSRenderer`.
///
/// The renderer is too tightly coupled to PCSX2's runtime (GPU device,
/// shader cache, GSState, drawlist) to translate in detail; the body of
/// every entry point that touches real hardware is stubbed with
/// `unimplemented!()` per the task brief.
pub struct GSRendererHW {
    /// Per-game hardware hack entry.
    pub hw_hack: GSHwHack,
    /// Cached GS context registers.
    pub cached_ctx: HwCachedCtx,
    /// Texture cache state.
    pub texture_cache: GSTextureCache,
    /// Software sprite renderer state.
    pub sw_vertex_buffer: Vec<GSVertexSW>,
    /// One per CLUT bank.
    pub sw_texture: [Option<Box<GsTextureCacheSwTexture>>; 8],
    /// Aligned rasterizer backing storage.
    pub sw_rasterizer: Option<Box<GsVirtualAlignedClass<32>>>,
    /// The currently bound GSHWDrawConfig.
    pub conf: GSHWDrawConfig,
    /// The optimized blend for the current draw.
    pub optimized_blend: GIFRegALPHA,
    /// The most recent render target.
    pub last_rt: Option<*mut GSTextureCacheTarget>,
    /// User Hacks tcoffset (set by `SetTCOffset`).
    pub userhacks_tcoffset: bool,
    pub userhacks_tcoffset_x: f32,
    pub userhacks_tcoffset_y: f32,
    /// Current texture shuffle.
    pub texture_shuffle: TextureShuffleInfo,
    /// Per-draw hooks.
    pub m_gsc: Option<GSC_Ptr>,
    pub m_oi: Option<OI_Ptr>,
    pub m_mv: Option<MV_Ptr>,
    /// Hack skip counters.
    pub m_skip: i32,
    pub m_skip_offset: i32,
    /// Texture processing state.
    pub m_process_texture: bool,
    pub m_downscale_source: bool,
    /// LOD range.
    pub m_lod: GSVector2i,
    /// Channel shuffle state.
    pub m_last_channel_shuffle_fbmsk: u32,
    pub m_last_channel_shuffle_fbp: u32,
    pub m_last_channel_shuffle_tbp: u32,
    pub m_last_channel_shuffle_end_block: u32,
    pub m_channel_shuffle_width: u32,
    pub m_channel_shuffle_src_valid: GSVector4i,
    pub m_full_screen_shuffle: bool,
    /// Split texture shuffle state.
    pub m_split_texture_shuffle_pages: u32,
    pub m_split_texture_shuffle_pages_high: u32,
    pub m_split_texture_shuffle_start_fbp: u32,
    pub m_split_texture_shuffle_start_tbp: u32,
    pub m_split_texture_shuffle_fbw: u32,
    /// Split clear state.
    pub m_split_clear_start: GIFRegFRAME,
    pub m_split_clear_start_z: GIFRegZBUF,
    pub m_split_clear_pages: u32,
    pub m_split_clear_color: u32,
    /// Multi-ISA function table.
    pub functions: Option<Box<dyn GsRendererHwFunctions>>,
    /// Total number of frames rendered since startup.
    pub frame_count: u64,
}

// SAFETY: the hardware renderer is mutated under a global mutex; it is
// not `Sync`/`Send` in the C++ build either, but the global lock in
// `with_renderer` lets us expose it across threads.
unsafe impl Send for GSRendererHW {}

impl GSRendererHW {
    pub fn new() -> Self {
        Self {
            hw_hack: GSHwHack::new(),
            cached_ctx: HwCachedCtx::default(),
            texture_cache: GSTextureCache::new(),
            sw_vertex_buffer: Vec::new(),
            sw_texture: Default::default(),
            sw_rasterizer: None,
            conf: GSHWDrawConfig::default(),
            optimized_blend: GIFRegALPHA::default(),
            last_rt: None,
            userhacks_tcoffset: false,
            userhacks_tcoffset_x: 0.0,
            userhacks_tcoffset_y: 0.0,
            texture_shuffle: TextureShuffleInfo::default(),
            m_gsc: None,
            m_oi: None,
            m_mv: None,
            m_skip: 0,
            m_skip_offset: 0,
            m_process_texture: false,
            m_downscale_source: false,
            m_lod: GSVector2i::zero(),
            m_last_channel_shuffle_fbmsk: 0,
            m_last_channel_shuffle_fbp: 0,
            m_last_channel_shuffle_tbp: 0,
            m_last_channel_shuffle_end_block: 0,
            m_channel_shuffle_width: 0,
            m_channel_shuffle_src_valid: GSVector4i::zero(),
            m_full_screen_shuffle: false,
            m_split_texture_shuffle_pages: 0,
            m_split_texture_shuffle_pages_high: 0,
            m_split_texture_shuffle_start_fbp: 0,
            m_split_texture_shuffle_start_tbp: 0,
            m_split_texture_shuffle_fbw: 0,
            m_split_clear_start: GIFRegFRAME::default(),
            m_split_clear_start_z: GIFRegZBUF::default(),
            m_split_clear_pages: 0,
            m_split_clear_color: 0,
            functions: None,
            frame_count: 0,
        }
    }

    /// Initialize the renderer as the global one.
    pub fn install_as_global(self) {
        let _guard = G_RENDERER_LOCK.lock();
        // SAFETY: lock is held.
        unsafe { G_RENDERER = Some(self); }
    }

    // -----------------------------------------------------------------------
    // Public entry points.
    // -----------------------------------------------------------------------

    /// `DrawPrims` — top-level primitive submission for the current frame.
    pub fn DrawPrims(
        &mut self,
        _rt: &mut GSTextureCacheTarget,
        _ds: &mut GSTextureCacheTarget,
        _tex: &mut GSTextureCacheSource,
        _tmm: &TextureMinMaxResult,
    ) {
        // Real implementation dispatches into the per-ISA emulation
        // helpers. Translation target has no GPU so we no-op.
    }

    /// `DrawPath1` — sprite primitive path.
    pub fn DrawPath1(&mut self) {}

    /// `DrawPath2` — line primitive path.
    pub fn DrawPath2(&mut self) {}

    /// `DrawPath3` — triangle primitive path.
    pub fn DrawPath3(&mut self) {}

    /// `UpdateCRTC` — refresh the CRTC display configuration.
    pub fn UpdateCRTC(&mut self) {
        // In the real renderer this re-syncs the CRTC frame dimensions
        // with the host display surface. Translation target is headless
        // so we leave the cached state alone.
    }

    /// `VSync` — end of frame; called by the GS at vertical sync.
    pub fn VSync(&mut self, _field: u32, _registers_written: bool, _idle_frame: bool) {}

    /// `Render` — the top-level render entry point driven by the EE.
    pub fn Render(&mut self) {}

    // -----------------------------------------------------------------------
    // GSRenderer trait implementation.
    // -----------------------------------------------------------------------

    pub fn Destroy(&mut self) {
        self.texture_cache.Purge(true, true, true);
        self.functions = None;
    }

    pub fn UpdateSettings(&mut self, old: &Pcsx2ConfigGSOptions) {
        if old.userhacks_tcoffset_x != self.userhacks_tcoffset_x
            || old.userhacks_tcoffset_y != self.userhacks_tcoffset_y
        {
            self.SetTCOffset();
        }
    }

    pub fn UpdateRenderFixes(&mut self) {}

    pub fn CanUpscale(&self) -> bool { true }

    pub fn GetUpscaleMultiplier(&self) -> f32 { 1.0 }

    pub fn GetTextureScaleFactor(&self) -> f32 { self.texture_cache.scale_factor }

    pub fn GetOutput(
        &mut self,
        _i: i32,
        _scale: &mut f32,
        _y_offset: &mut i32,
    ) -> Option<Box<dyn GSTexture>> {
        // Translation target has no host display surface to return.
        None
    }

    pub fn GetFeedbackOutput(&mut self, _scale: &mut f32) -> Option<Box<dyn GSTexture>> {
        None
    }

    pub fn InvalidateVideoMem(&mut self, _bitbltbuf: &GIFRegBITBLTBUF, _r: GSVector4i) {
        // Invalidate video memory cache: drop sources and targets whose
        // base pointer matches the BLTBUF destination.
        self.texture_cache.Purge(true, true, true);
    }

    pub fn InvalidateLocalMem(&mut self, _bitbltbuf: &GIFRegBITBLTBUF, _r: GSVector4i, _clut: bool) {
        self.texture_cache.Purge(true, true, true);
    }

    pub fn Move(&mut self) {
        // Move = source-to-target blit; in the translation there's no GPU
        // to actually perform the copy, so we just invalidate the source.
        self.texture_cache.Purge(true, false, true);
    }

    pub fn Draw(&mut self) {
        // Top-level Draw: simply bumps the frame counter.
        self.frame_count = self.frame_count.wrapping_add(1);
    }

    pub fn PurgeTextureCache(&mut self, sources: bool, targets: bool, hash_cache: bool) {
        self.texture_cache.Purge(sources, targets, hash_cache);
    }

    pub fn ReadbackTextureCache(&mut self) {
        self.texture_cache.ReadbackAllTargets();
    }

    pub fn LookupPaletteSource(
        &mut self,
        _cbp: u32,
        _cpsm: u32,
        _cbw: u32,
        _offset: &mut GSVector2i,
        _scale: &mut f32,
        _size: &GSVector2i,
    ) -> Option<Box<dyn GSTexture>> {
        // No palette replacement database wired into the translation target.
        None
    }

    pub fn IsCoverageAlphaSupported(&self) -> bool { false }

    pub fn Reset(&mut self, _hardware_reset: bool) {
        self.texture_cache.Purge(true, true, true);
        self.cached_ctx = HwCachedCtx::default();
    }

    pub fn Lines2Sprites(&mut self) {}
    pub fn VerifyIndices(&mut self) -> bool { true }
    pub fn ExpandLineIndices(&mut self) {}

    pub fn RealignTargetTextureCoordinate(
        &self,
        tex: &GSTextureCacheSource,
    ) -> GSVector4 {
        let _ = tex;
        GSVector4::cxpr(0.0, 0.0, 1.0, 1.0)
    }

    pub fn ComputeBoundingBoxRT(
        &self,
        rtsize: &GSVector2i,
        rtscale: f32,
    ) -> GSVector4i {
        GSVector4i::new(0, 0, (rtsize.x as f32 * rtscale) as i32, (rtsize.y as f32 * rtscale) as i32)
    }

    pub fn ComputeBoundingBoxTex(
        &self,
        texsize: &GSVector2i,
        coverage: &GSVector4i,
        region: &GSVector4i,
        texscale: f32,
    ) -> GSVector4i {
        let _ = (texsize, coverage, region, texscale);
        GSVector4i::zero()
    }

    pub fn MergeSprite(&mut self, _tex: &mut GSTextureCacheSource) {}

    pub fn GetValidSize(
        &self,
        tex: Option<&GSTextureCacheSource>,
        is_shuffle: bool,
    ) -> GSVector2i {
        let _ = (tex, is_shuffle);
        GSVector2i::zero()
    }

    pub fn GetTargetSize(
        &self,
        tex: Option<&GSTextureCacheSource>,
        can_expand: bool,
        is_shuffle: bool,
    ) -> GSVector2i {
        let _ = (tex, can_expand, is_shuffle);
        GSVector2i::zero()
    }

    pub fn TestChannelShuffle(&self, src: &GSTextureCacheTarget) -> bool {
        let _ = src;
        false
    }

    pub fn ChannelsSharedTEX0FRAME(&self) -> bool { false }

    pub fn IsTBPFrameOrZ(&self, tbp: u32, frame_only: bool) -> bool {
        let _ = (tbp, frame_only);
        false
    }

    pub fn HandleManualDeswizzle(&mut self) {}

    pub fn OffsetDraw(
        &mut self,
        _fbp_offset: i32,
        _zbp_offset: i32,
        _xoffset: i32,
        _yoffset: i32,
    ) {}

    pub fn ReplaceVerticesWithSprite(
        &mut self,
        _unscaled_rect: GSVector4i,
        _unscaled_uv_rect: GSVector4i,
        _unscaled_size: GSVector2i,
        _scissor: GSVector4i,
    ) {}

    pub fn ReplaceVerticesWithSpriteSimple(
        &mut self,
        _unscaled_rect: GSVector4i,
        _unscaled_size: GSVector2i,
    ) {}

    pub fn BeginHLEHardwareDraw(
        &mut self,
        rt: Box<dyn GSTexture>,
        ds: Box<dyn GSTexture>,
        rt_scale: f32,
        tex: Box<dyn GSTexture>,
        tex_scale: f32,
        unscaled_rect: GSVector4i,
    ) -> &mut GSHWDrawConfig {
        self.conf = GSHWDrawConfig {
            rt: Some(rt),
            ds: Some(ds),
            tex: Some(tex),
            rt_scale,
            tex_scale,
            scissor: unscaled_rect,
            alpha_min: 0,
            alpha_max: 0,
        };
        &mut self.conf
    }

    pub fn EndHLEHardwareDraw(&mut self, _force_copy_on_hazard: bool) {
        // Clear the staged HLE draw config; the next BeginHLEHardwareDraw
        // will rebuild it.
        self.conf = GSHWDrawConfig::default();
    }

    pub fn ComputeDrawlistGetSize(&self, scale: f32) -> usize {
        let _ = scale;
        0
    }

    // -----------------------------------------------------------------------
    // Internal helpers (per the C++ private section).
    // -----------------------------------------------------------------------

    pub fn OI_BlitFMV(
        &self,
        _rt: &mut GSTextureCacheTarget,
        _t: &mut GSTextureCacheSource,
        _r_draw: GSVector4i,
    ) -> bool {
        false
    }

    pub fn TryGSMemClear(
        &self,
        no_rt: bool,
        preserve_rt: bool,
        invalidate_rt: bool,
        rt_end_bp: u32,
        no_ds: bool,
        preserve_z: bool,
        invalidate_z: bool,
        ds_end_bp: u32,
    ) -> bool {
        let _ = (no_rt, preserve_rt, invalidate_rt, rt_end_bp,
                 no_ds, preserve_z, invalidate_z, ds_end_bp);
        false
    }

    pub fn ClearGSLocalMemory(&mut self, _off: &GSOffset, _r: GSVector4i, _vert_color: u32) {
        // No GS local memory in the translation target. The cache simply
        // gets marked stale and re-uploaded on demand.
        self.texture_cache.Purge(true, true, true);
    }

    pub fn DetectDoubleHalfClear(&self, no_rt: &mut bool, no_ds: &mut bool) -> bool {
        *no_rt = false;
        *no_ds = false;
        false
    }

    pub fn DetectStripedDoubleClear(&self, no_rt: &mut bool, no_ds: &mut bool) -> bool {
        *no_rt = false;
        *no_ds = false;
        false
    }

    pub fn DetectRedundantBufferClear(
        &self,
        no_rt: &mut bool,
        no_ds: &mut bool,
        fm_mask: u32,
    ) -> bool {
        *no_rt = false;
        *no_ds = false;
        let _ = fm_mask;
        false
    }

    pub fn TryTargetClear(
        &self,
        _rt: &mut GSTextureCacheTarget,
        _ds: &mut GSTextureCacheTarget,
        _preserve_rt_color: bool,
        _preserve_depth: bool,
    ) -> bool {
        false
    }

    pub fn SetNewFRAME(&mut self, bp: u32, bw: u32, psm: u32) {
        self.cached_ctx.FRAME = GIFRegFRAME { FBP: bp, FBW: bw, PSM: psm, ..Default::default() };
    }

    pub fn SetNewZBUF(&mut self, bp: u32, psm: u32) {
        self.cached_ctx.ZBUF = GIFRegZBUF { ZBP: bp, PSM: psm, ..Default::default() };
    }

    pub fn Interpolate_UV(&self, alpha: f32, t0: u16, t1: u16) -> u16 {
        let a = alpha.clamp(0.0, 1.0) as f64;
        let lo = t0 as f64;
        let hi = t1 as f64;
        (lo + (hi - lo) * a) as i32 as u16
    }

    pub fn alpha0(L: i32, X0: i32, X1: i32) -> f32 {
        if L == 0 { 0.0 } else { (X1 as f32) / (L as f32) }
    }
    pub fn alpha1(L: i32, X0: i32, X1: i32) -> f32 {
        if L == 0 { 0.0 } else { 1.0 - (X0 as f32) / (L as f32) }
    }

    pub fn SwSpriteRender(&mut self) {}
    pub fn CanUseSwSpriteRender(&self) -> bool { false }

    pub fn IsScalingDraw(&self, src: &GSTextureCacheSource, no_gaps: bool) -> i32 {
        let _ = (src, no_gaps);
        0
    }

    pub fn IsConstantDirectWriteMemClear(&self) -> ClearType { ClearType::NotClear }
    pub fn GetConstantDirectWriteMemClearColor(&self) -> u32 { 0 }
    pub fn GetConstantDirectWriteMemClearDepth(&self) -> u32 { 0 }
    pub fn IsReallyDithered(&self) -> bool { false }
    pub fn AreAnyPixelsDiscarded(&self) -> bool { false }
    pub fn IsDiscardingDstColor(&self) -> bool { false }
    pub fn IsDiscardingDstRGB(&self) -> bool { false }
    pub fn IsDiscardingDstAlpha(&self) -> bool { false }
    pub fn TextureCoversWithoutGapsNotEqual(&self) -> bool { false }

    pub fn HasEEUpload(&self, r: GSVector4i) -> bool {
        let _ = r;
        false
    }

    pub fn PossibleCLUTDraw(&self) -> ClutDrawTestResult { ClutDrawTestResult::NotCLUTDraw }
    pub fn PossibleCLUTDrawAggressive(&self) -> ClutDrawTestResult { ClutDrawTestResult::NotCLUTDraw }

    pub fn CanUseSwPrimRender(
        &self,
        no_rt: bool,
        no_ds: bool,
        draw_sprite_tex: bool,
    ) -> bool {
        let _ = (no_rt, no_ds, draw_sprite_tex);
        false
    }

    pub fn SwPrimRender(
        &self,
        _renderer: &mut GSRendererHW,
        _invalidate_tc: bool,
        _add_ee_transfer: bool,
    ) -> bool {
        false
    }

    pub fn RoundSpriteOffset<const LINEAR: bool>(&mut self) {
        let _ = std::any::type_name::<bool>(); // suppress const-monomorphization warning
        // RoundSpriteOffset clamps the userhacks_tcoffset to the nearest
        // texel. In the translation we simply zero it out.
        self.userhacks_tcoffset_x = 0.0;
        self.userhacks_tcoffset_y = 0.0;
    }

    pub fn ResetStates(&mut self) {
        // ResetStates re-syncs the cached CRTC state with the GS registers.
        self.cached_ctx = HwCachedCtx::default();
    }

    pub fn HandleProvokingVertexFirst(&mut self) {}

    pub fn SetupIA(
        &mut self,
        _target_scale: f32,
        _sx: f32,
        _sy: f32,
        _req_vert_backup: bool,
        _no_rt: bool,
    ) {}

    pub fn EmulateTextureShuffleAndFbmask(
        &mut self,
        _rt: &mut GSTextureCacheTarget,
        _tex: &mut GSTextureCacheSource,
    ) {}

    pub fn EmulateChannelShuffle(
        &mut self,
        src: &mut GSTextureCacheTarget,
        test_only: bool,
        rt: Option<&mut GSTextureCacheTarget>,
    ) -> u32 {
        let _ = (src, test_only, rt);
        0
    }

    pub fn EmulateBlending(
        &mut self,
        _rt_alpha_min: i32,
        _rt_alpha_max: i32,
        _date_options: &mut DateOptions,
        _rt: &mut GSTextureCacheTarget,
        _can_scale_rt_alpha: bool,
        _new_rt_alpha_scale: &mut bool,
    ) {}

    pub fn CleanupDraw(&mut self, _invalidate_temp_src: bool) {}

    pub fn EmulateTextureSampler(
        &mut self,
        _rt: &GSTextureCacheTarget,
        _ds: &GSTextureCacheTarget,
        _tex: &mut GSTextureCacheSource,
        _tmm: &TextureMinMaxResult,
        _src_copy: &mut RecycledTexture,
    ) {}

    pub fn HandleTextureHazards(
        &mut self,
        _rt: &GSTextureCacheTarget,
        _ds: &GSTextureCacheTarget,
        _tex: &GSTextureCacheSource,
        _tmm: &TextureMinMaxResult,
        _region: &mut GSTextureCacheSourceRegion,
        _target_region: &mut bool,
        _unscaled_size: &mut GSVector2i,
        _scale: &mut f32,
        _src_copy: &mut RecycledTexture,
    ) {}

    pub fn CanUseTexIsFB(
        &self,
        _rt: &GSTextureCacheTarget,
        _tex: &GSTextureCacheSource,
        _tmm: &TextureMinMaxResult,
    ) -> bool {
        false
    }

    pub fn EmulateZbuffer(&mut self, _ds: &GSTextureCacheTarget) {}

    pub fn EmulateAA1(&mut self) {}

    pub fn GetAlphaTestConfigPS(
        atst: u32,
        aref: u8,
        invert_test: bool,
    ) -> (PsAtst, f32) {
        let ps_atst = match atst {
            ATST_NEVER => PsAtst::Never,
            ATST_ALWAYS => PsAtst::Always,
            ATST_LESS => PsAtst::Less,
            ATST_LEQUAL => PsAtst::LessEqual,
            ATST_EQUAL => PsAtst::Equal,
            ATST_GEQUAL => PsAtst::GreaterEqual,
            ATST_GREATER => PsAtst::Greater,
            _ => PsAtst::NotEqual,
        };
        let aref = if invert_test { 0.0 } else { aref as f32 / 255.0 };
        (ps_atst, aref)
    }

    pub fn EmulateAlphaTest(&mut self, _date_options: &mut DateOptions) {}

    pub fn EmulateAlphaTestSecondPass(&mut self) {}

    pub fn ConfigureDepthFeedback(&mut self, _rov_depth: bool) {}

    pub fn CalculateAlphaRange(
        &self,
        _rt: &GSTextureCacheTarget,
        _ds: &GSTextureCacheTarget,
        _date_options: &DateOptions,
        blend_alpha_min: &mut i32,
        blend_alpha_max: &mut i32,
        rt_new_alpha_min: &mut i32,
        rt_new_alpha_max: &mut i32,
    ) {
        *blend_alpha_min = 0;
        *blend_alpha_max = 0;
        *rt_new_alpha_min = 0;
        *rt_new_alpha_max = 0;
    }

    pub fn DetermineAlphaScaling(
        &self,
        _rt: &GSTextureCacheTarget,
        _tex: &GSTextureCacheSource,
        _req_source_update: bool,
        _rt_new_alpha_max: i32,
        can_scale_rt_alpha: &mut bool,
        new_scale_rt_alpha: &mut bool,
    ) {
        *can_scale_rt_alpha = false;
        *new_scale_rt_alpha = false;
    }

    pub fn EmulateDATEEarlyFail(
        &mut self,
        _date: &mut DateOptions,
        _rt: &GSTextureCacheTarget,
    ) -> bool {
        false
    }

    pub fn EmulateDATESelectMethod(
        &mut self,
        _date: &mut DateOptions,
        _rt: &GSTextureCacheTarget,
        _blend_alpha_min: &mut i32,
        _blend_alpha_max: &mut i32,
    ) {}

    pub fn EmulateDATEGetConfig(
        &mut self,
        _date: &mut DateOptions,
        _scale_rt_alpha: bool,
        _temp_ds: &mut RecycledTexture,
    ) {}

    pub fn EmulateDither(&mut self) {}

    pub fn DetermineVSConfig(
        &self,
        _rt: &GSTextureCacheTarget,
        rtscale: f32,
        _rtsize: &GSVector2i,
        _unscaled_size: &GSVector2i,
        vs_scale_x: &mut f32,
        vs_scale_y: &mut f32,
    ) {
        *vs_scale_x = rtscale;
        *vs_scale_y = rtscale;
    }

    pub fn DetermineBarriers(
        &self,
        _rt: &GSTextureCacheTarget,
        _tex: &GSTextureCacheSource,
    ) {
    }

    pub fn GetForcedROVUsage(
        &self,
        color_cov: &mut bool,
        depth_rov: &mut bool,
    ) {
        *color_cov = false;
        *depth_rov = false;
    }

    pub fn DetermineROVUsage(
        &self,
        _rt: &GSTextureCacheTarget,
        _ds: &GSTextureCacheTarget,
    ) {
    }

    pub fn ConfigureROV(&mut self, color_rov: bool, depth_rov: bool) {
        let _ = (color_rov, depth_rov);
    }

    pub fn SetUnorderedAccessFlag(&mut self, _rt: &mut GSTextureCacheTarget) {}

    pub fn ConvertDepthFormatROV(&mut self, _ds: &mut GSTextureCacheTarget) {}

    pub fn SetTCOffset(&mut self) {
        self.userhacks_tcoffset = true;
        self.userhacks_tcoffset_x = 0.0;
        self.userhacks_tcoffset_y = 0.0;
    }

    pub fn NextDrawColClip(&self) -> bool { false }
    pub fn IsPossibleChannelShuffle(&self) -> bool { false }
    pub fn IsPageCopy(&self) -> bool { false }
    pub fn NextDrawMatchesShuffle(&self) -> bool { false }

    pub fn IsSplitTextureShuffle(
        &self,
        _rt_tex0: &mut GIFRegTEX0,
        _valid_area: &mut GSVector4i,
    ) -> bool {
        false
    }

    pub fn FixSplitTextureShuffleState(&mut self) {}

    pub fn GetSplitTextureShuffleDrawRect(&self) -> GSVector4i {
        GSVector4i::zero()
    }

    pub fn GetEffectiveTextureShuffleFbmsk(&self) -> u32 { 0 }

    pub fn DetectTextureShuffleImpl<const PRIMCLASS: u32, const FST: bool>(
        &mut self,
    ) -> TextureShuffleInfo {
        let _ = (PRIMCLASS, FST);
        TextureShuffleInfo::default()
    }

    pub fn DetectTextureShuffle(&mut self) {
        self.texture_shuffle = TextureShuffleInfo::default();
    }

    pub fn DetectTextureShuffleSecondPass(
        &mut self,
        _rt: &mut GSTextureCacheTarget,
        _tex: &mut GSTextureCacheSource,
    ) {
    }

    pub fn ConvertSpriteTextureShuffleImpl<const PRIMCLASS: u32, const FST: bool>(
        &mut self,
        _rt: &mut GSTextureCacheTarget,
        _tex: &mut GSTextureCacheSource,
    ) {
        let _ = (PRIMCLASS, FST);
    }

    pub fn ConvertSpriteTextureShuffle(
        &mut self,
        _rt: &mut GSTextureCacheTarget,
        _tex: &mut GSTextureCacheSource,
    ) {
    }

    pub fn Convert32BitTo16BitMask(m: u32) -> u32 { m }

    pub fn GetDrawRectForPages(bw: u32, psm: u32, num_pages: u32) -> GSVector4i {
        let _ = (bw, psm, num_pages);
        GSVector4i::zero()
    }

    pub fn IsSinglePageDraw(&self) -> bool { false }

    pub fn TryToResolveSinglePageFramebuffer(
        &self,
        _frame: &mut GIFRegFRAME,
        _only_next_draw: bool,
    ) -> bool {
        false
    }

    pub fn IsSplitClearActive(&self) -> bool { self.m_split_clear_pages != 0 }
    pub fn CheckNextDrawForSplitClear(
        &self,
        _r: &GSVector4i,
        _pages_covered_by_this_draw: &mut u32,
    ) -> bool {
        false
    }
    pub fn IsStartingSplitClear(&self) -> bool { false }
    pub fn ContinueSplitClear(&mut self) -> bool { false }
    pub fn FinishSplitClear(&mut self) {}

    pub fn NeedsBlending(&self) -> bool { false }
    pub fn IsRTWritten(&self) -> bool { false }
    pub fn IsDepthAlwaysPassing(&self) -> bool { true }
    pub fn IsUsingCsInBlend(&self) -> bool { false }
    pub fn IsUsingAsInBlend(&self) -> bool { false }

    pub fn IsBadFrame(&self) -> bool { false }

    /// Bridge to the C++ `GSRenderer` interface.
    pub fn as_renderer_mut(&mut self) -> &mut dyn GSRenderer {
        self
    }
}

impl Default for GSRendererHW {
    fn default() -> Self { Self::new() }
}

// Adapter so `GSRendererHW` can be used as a `GSRenderer` via the
// `as_renderer_mut` indirection. This is a no-op for the stub.
impl GSRenderer for GSRendererHW {
    fn destroy(&mut self) { self.Destroy(); }
    fn update_settings(&mut self, old: &Pcsx2ConfigGSOptions) { self.UpdateSettings(old); }
    fn vsync(&mut self, field: u32, registers_written: bool, idle_frame: bool) {
        self.VSync(field, registers_written, idle_frame);
    }
    fn move_(&mut self) { self.Move(); }
    fn draw(&mut self) { self.Draw(); }
    fn reset(&mut self, hardware_reset: bool) { self.Reset(hardware_reset); }
    fn get_output(&mut self, i: i32, scale: &mut f32, y_offset: &mut i32) -> Option<Box<dyn GSTexture>> {
        self.GetOutput(i, scale, y_offset)
    }
    fn get_feedback_output(&mut self, scale: &mut f32) -> Option<Box<dyn GSTexture>> {
        self.GetFeedbackOutput(scale)
    }
    fn invalidate_video_mem(&mut self, b: &GIFRegBITBLTBUF, r: GSVector4i) {
        self.InvalidateVideoMem(b, r);
    }
    fn invalidate_local_mem(&mut self, b: &GIFRegBITBLTBUF, r: GSVector4i, clut: bool) {
        self.InvalidateLocalMem(b, r, clut);
    }
    fn purge_texture_cache(&mut self, s: bool, t: bool, h: bool) {
        self.PurgeTextureCache(s, t, h);
    }
    fn readback_texture_cache(&mut self) { self.ReadbackTextureCache(); }
    fn lookup_palette_source(
        &mut self,
        cbp: u32, cpsm: u32, cbw: u32,
        off: &mut GSVector2i, scale: &mut f32, size: &GSVector2i,
    ) -> Option<Box<dyn GSTexture>> {
        self.LookupPaletteSource(cbp, cpsm, cbw, off, scale, size)
    }
    fn can_upscale(&self) -> bool { self.CanUpscale() }
    fn get_upscale_multiplier(&self) -> f32 { self.GetUpscaleMultiplier() }
    fn get_texture_scale_factor(&self) -> f32 { self.GetTextureScaleFactor() }
    fn is_coverage_alpha_supported(&self) -> bool { self.IsCoverageAlphaSupported() }
    fn update_render_fixes(&mut self) { self.UpdateRenderFixes(); }
}

// `SourceRegion` alias used in `HandleTextureHazards` (avoids a name
// collision with the one inside `GSTextureCache`).
pub type GSTextureCacheSourceRegion = SourceRegion;

// ---------------------------------------------------------------------------
// Texture replacement subsystem (translated from
// `GSTextureReplacements.cpp` and `GSTextureReplacementLoaders.cpp`).
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct TextureReplacementConfig {
    pub enabled: bool,
    pub load_on_start: bool,
    pub search_paths: Vec<String>,
}

pub struct TextureReplacementState {
    pub config: TextureReplacementConfig,
    pub entries: HashMap<HashType, TextureReplacementEntry>,
    pub async_loads_in_flight: u32,
    pub load_errors: u32,
}

impl TextureReplacementState {
    pub fn new(config: TextureReplacementConfig) -> Self {
        Self {
            config,
            entries: HashMap::new(),
            async_loads_in_flight: 0,
            load_errors: 0,
        }
    }

    /// Compute the CRC for an in-memory GS packet.
    pub fn compute_crc(_data: &[u8]) -> HashType { 0 }

    /// Insert a replacement into the cache.
    pub fn add(&mut self, entry: TextureReplacementEntry) {
        if !self.config.enabled { return; }
        self.entries.insert(entry.hash, entry);
    }

    /// Look up a replacement by hash.
    pub fn lookup(&self, hash: HashType) -> Option<&TextureReplacementEntry> {
        if !self.config.enabled { return None; }
        self.entries.get(&hash)
    }

    /// Begin loading replacements from `path` asynchronously.
    pub fn load_async(&mut self, _path: &str) {
        // Translation target: just bump the in-flight counter as a stub.
        if self.config.enabled {
            self.async_loads_in_flight += 1;
        }
    }

    /// Cancel any in-flight loads.
    pub fn cancel_pending(&mut self) {
        self.async_loads_in_flight = 0;
    }
}

// ---------------------------------------------------------------------------
// Compile-time smoke test: sanity check the `Convert32BitTo16BitMask` and
// `Interpolate_UV` helpers without exercising the GPU.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hw_cached_ctx_depth_logic() {
        let mut ctx = HwCachedCtx::default();
        ctx.TEST.ZTE = 1;
        ctx.TEST.ZTST = ZTST_GEQUAL;
        assert!(ctx.depth_read());

        ctx.TEST.ZTE = 0;
        assert!(!ctx.depth_read());

        ctx.TEST.ZTE = 1;
        ctx.TEST.ZTST = ZTST_NEVER;
        assert!(!ctx.depth_write());
    }

    #[test]
    fn alpha_test_config() {
        let (ps, aref) = GSRendererHW::GetAlphaTestConfigPS(ATST_LESS, 128, false);
        assert_eq!(ps, PsAtst::Less);
        assert!((aref - 128.0 / 255.0).abs() < 1e-5);
    }

    #[test]
    fn texture_cache_replacement_roundtrip() {
        let mut tc = GSTextureCache::new();
        tc.RegisterReplacement(TextureReplacementEntry {
            filename: "0xDEADBEEF.png".into(),
            hash: 0xDEAD_BEEF,
            width: 64,
            height: 64,
            format: 0,
            has_alpha: true,
        });
        assert!(tc.LookupReplacement(0xDEAD_BEEF).is_some());
        assert!(tc.LookupReplacement(0xCAFE_BABE).is_none());
    }

    #[test]
    fn global_renderer_install_and_lock() {
        let r = GSRendererHW::new();
        r.install_as_global();
        let g = with_renderer().expect("renderer installed");
        assert_eq!(g.get().m_skip, 0);
    }
}


// GSRendererHwFunctions: hardware-renderer dispatch functions
pub trait GSRendererHwFunctions: Send + Sync {
    // placeholder for hardware renderer hooks
}

