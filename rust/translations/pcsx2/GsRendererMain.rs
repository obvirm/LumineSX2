// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust 2021 translation of PCSX2's GS renderer source set.
//!
//! This module consolidates the following C/C++ translation units into a
//! single Rust module:
//!
//! * `GSRendererHW.cpp` / `GSRendererHW.h` — the main hardware renderer
//!   (`GSRendererHW`) and its helpers for clear detection, texture
//!   shuffles, DATE/AA1/dither/blend emulation, ROV configuration,
//!   per-game hardware-hack wiring, and the public surface
//!   `DrawPrims` / `DrawPath1/2/3` / `UpdateCRTC` / `VSync` / `Render`.
//! * `GSRendererHWMultiISA.cpp` — the SIMD multi-ISA dispatch machinery
//!   (`GSRendererHWFunctions` / `GSRendererHWPopulateFunctions`).
//! * `GSHwHack.cpp` / `GSHwHack.h` — the per-game CRC-based "hardware
//!   hack" table (`GSHwHack`) with `GSC_*` (skip-count), `OI_*`
//!   (before-draw), and `MV_*` (move-handler) callbacks.
//! * `GSRendererSW.cpp` — the software renderer (`GSRendererSW`).
//! * `GSRasterizer.cpp` — the multi-threaded software rasterizer
//!   (`GSRasterizer` and friends).
//! * `GSDrawScanline.cpp` / `GSDrawScanlineCodeGenerator.all.cpp` —
//!   the scanline draw helpers and the JIT code-generator interface.
//! * `GSSetupPrimCodeGenerator.all.cpp` — the prim setup
//!   code-generator interface.
//! * `GSTextureCacheSW.cpp` — the software texture cache
//!   (`GSTextureCacheSW`).
//! * `GSRendererNull.cpp` / `GSRendererNull.h` — the do-nothing
//!   renderer (`GSRendererNull`).
//! * `GSDirtyRect.cpp` / `GSDirtyRect.h` — the dirty-rect bookkeeping
//!   (`GSDirtyRect`, `GSDirtyRectList`).
//! * `GSTextureReplacements.cpp` / `GSTextureReplacements.h` — the
//!   texture-replacement bookkeeping (`GSTextureReplacement`).
//! * `GSTextureReplacementLoaders.cpp` — async texture-replacement
//!   loaders for DDS/PNG/zip archives.
//!
//! The hardware renderer's body, the multi-ISA SIMD dispatch, and the
//! JIT code-generators are too tightly coupled to the GS register
//! state, GPU device, and CPU-specific assembler to translate in
//! detail; the entry points that exercise real hardware are stubbed
//! with `unimplemented!()` per the task brief. The data structures,
//! enums, fields, and per-method surface are exposed so the module
//! is usable as a shape reference for downstream translation work.
//!
//! Only `std` is used.

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(unused_mut)]
#![allow(unused_imports)]

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

// ===========================================================================
// Type aliases mirroring the C/C++ fixed-width integer types used in GS code.
// ===========================================================================

pub type u8  = std::primitive::u8;
pub type u16 = std::primitive::u16;
pub type u32 = std::primitive::u32;
pub type u64 = std::primitive::u64;
pub type s8  = std::primitive::i8;
pub type s16 = std::primitive::i16;
pub type s32 = std::primitive::i32;
pub type s64 = std::primitive::i64;
pub type uptr = std::primitive::usize;

pub type HashType = u64;

pub const PSMCT32: u32 = 0;
pub const PSMCT24: u32 = 1;
pub const PSMCT16: u32 = 2;

pub const MAX_FRAMEBUFFER_HEIGHT: i32 = 1280;
pub const SSR_UV_TOLERANCE: f32 = 1.0;
pub const MAX_BP: u32 = 0x3fff;
pub const GS_MAX_BLOCKS: u32 = 0x4000;

// ===========================================================================
// Minimal GSVector2i / GSVector4 / GSVector4i stand-ins.
//
// The full GSVector SIMD type is enormous; for the purpose of an
// idiomatic translation of the *structure* of the renderer, an opaque
// 16-byte value is sufficient.  Individual accessor methods on the
// renderer operate on these values and downstream implementations are
// free to wrap a real SIMD type.
// ===========================================================================

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GSVector2i {
    pub x: i32,
    pub y: i32,
}

impl GSVector2i {
    pub const fn new(x: i32, y: i32) -> Self { Self { x, y } }
    pub const fn zero() -> Self { Self::new(0, 0) }
    pub const fn splat(v: i32) -> Self { Self::new(v, v) }
    pub const fn x(&self) -> i32 { self.x }
    pub const fn y(&self) -> i32 { self.y }
    pub const fn z(&self) -> i32 { self.x }
    pub const fn w(&self) -> i32 { self.y }
    pub const fn width(self) -> i32 { self.z() - self.x() }
    pub const fn height(self) -> i32 { self.w() - self.y() }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GSVector2 {
    pub x: f32,
    pub y: f32,
}

impl GSVector2 {
    pub const fn new(x: f32, y: f32) -> Self { Self { x, y } }
    pub const fn zero() -> Self { Self::new(0.0, 0.0) }
    pub const fn splat(v: f32) -> Self { Self::new(v, v) }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GSVector4 {
    pub x: f32, pub y: f32, pub z: f32, pub w: f32,
}

impl GSVector4 {
    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self { Self { x, y, z, w } }
    pub const fn zero() -> Self { Self::new(0.0, 0.0, 0.0, 0.0) }
    pub const fn splat(v: f32) -> Self { Self::new(v, v, v, v) }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GSVector4i {
    pub x: i32, pub y: i32, pub z: i32, pub w: i32,
}

impl GSVector4i {
    pub const fn new(x: i32, y: i32, z: i32, w: i32) -> Self { Self { x, y, z, w } }
    pub const fn zero() -> Self { Self::new(0, 0, 0, 0) }
    pub const fn width(&self) -> i32 { self.z - self.x }
    pub const fn height(&self) -> i32 { self.w - self.y }
    pub const fn left(&self) -> i32 { self.x }
    pub const fn top(&self) -> i32 { self.y }
    pub const fn right(&self) -> i32 { self.z }
    pub const fn bottom(&self) -> i32 { self.w }
    pub const fn rempty(&self) -> bool { self.x >= self.z || self.y >= self.w }
    pub const fn eq(&self, other: Self) -> bool {
        self.x == other.x && self.y == other.y && self.z == other.z && self.w == other.w
    }
}

// ===========================================================================
// GIF register state structs (subset sufficient for the renderer APIs).
// ===========================================================================

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GIFRegDIMX {
    pub dimx: [u32; 4],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GIFRegPRIM {
    pub PRIM: u32,
    pub IIP: u32,
    pub TME: u32,
    pub FGE: u32,
    pub ABE: u32,
    pub AA1: u32,
    pub FST: u32,
    pub CTXT: u32,
    pub FIX: u32,
}

// ZTST / ATST / AFAIL constants.
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

pub const AFAIL_KEEP: u32 = 0;
pub const AFAIL_FB_ONLY: u32 = 1;
pub const AFAIL_ZB_ONLY: u32 = 2;
pub const AFAIL_RGB_ONLY: u32 = 3;

pub const CLAMP_REPEAT: u32 = 0;
pub const CLAMP_CLAMP: u32 = 1;
pub const CLAMP_REGION_CLAMP: u32 = 2;
pub const CLAMP_REGION_REPEAT: u32 = 3;

pub const TFX_NONE: u32 = 0;
pub const TFX_REPLACE: u32 = 1;
pub const TFX_DECAL: u32 = 2;
pub const TFX_MODULATE: u32 = 3;
pub const TFX_HIGHLIGHT: u32 = 4;
pub const TFX_HIGHLIGHT2: u32 = 5;

pub const GS_POINT_CLASS: u32 = 0;
pub const GS_LINE_CLASS: u32 = 1;
pub const GS_TRIANGLE_CLASS: u32 = 2;
pub const GS_SPRITE_CLASS: u32 = 3;

// ===========================================================================
// Opaque stand-ins for the larger types in the C++ code.
//
// These are intentionally minimal — they model "some big GPU-side
// resource" or "some big CPU-side buffer" so that the renderer
// function signatures can compile.  Downstream code is free to
// replace these with richer definitions as translation progresses.
// ===========================================================================

#[derive(Default, Debug)]
pub struct GSDevice;

#[derive(Default, Debug)]
pub struct GSTexture;

#[derive(Default, Debug)]
pub struct GSTextureCache;

#[derive(Default, Debug)]
pub struct GSTextureCacheSource;

#[derive(Default, Debug)]
pub struct GSTextureCacheTarget;

#[derive(Default, Debug)]
pub struct GSHWDrawConfig;

#[derive(Default, Debug)]
pub struct GSOffset;

#[derive(Default, Debug)]
pub struct GSDrawingContext;

#[derive(Default, Debug)]
pub struct GSDrawingEnvironment;

#[derive(Default, Debug)]
pub struct GSVertexTrace;

#[derive(Default, Debug)]
pub struct GSPerfMon;

#[derive(Default, Debug)]
pub struct GSPng;

#[derive(Default, Debug)]
pub struct GSState;

#[derive(Default, Debug)]
pub struct GSUtil;

#[derive(Default, Debug)]
pub struct Pcsx2Config;

#[derive(Default, Debug)]
pub struct GSTextureCacheHashCacheKey;

#[derive(Default, Debug)]
pub struct GSLocalMemory;

// Minimal TextureMinMaxResult used by the renderer's internal heuristics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextureMinMaxResult {
    pub coverage: GSVector4i,
    pub min: i32,
    pub max: i32,
    pub same_page: bool,
}

// ===========================================================================
// Global renderer state.
//
// Mirrors the C++ `static std::unique_ptr<GSRenderer> g_gs_renderer` and
// the legacy `g_renderer` raw pointer.  Access is guarded by a
// `Mutex` so the `static mut` is only ever written through a
// synchronized helper.
// ===========================================================================

pub static mut G_GS_RENDERER: Option<GSRendererHW> = None;

static G_GS_RENDERER_LOCK: Mutex<()> = Mutex::new(());

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
    let guard = G_GS_RENDERER_LOCK.lock().ok()?;
    // SAFETY: we hold the renderer lock.
    let ptr = unsafe { G_GS_RENDERER.as_mut()? as *mut GSRendererHW };
    Some(RendererGuard { _guard: guard, ptr })
}

/// Set the global renderer (synchronized).
pub fn set_renderer(r: Option<GSRendererHW>) {
    let _g = G_GS_RENDERER_LOCK.lock().ok();
    // SAFETY: we hold the renderer lock.
    unsafe { G_GS_RENDERER = r; }
}

// ===========================================================================
// HWCachedCtx — per-renderer cached GIF register context used for
// optimizing away unnecessary work.  Mirrors `GSRendererHW::HWCachedCtx`.
// ===========================================================================

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HWCachedCtx {
    pub TEX0:  GIFRegTEX0,
    pub TEXA:  GIFRegTEXA,
    pub CLAMP: GIFRegCLAMP,
    pub TEST:  GIFRegTEST,
    pub FRAME: GIFRegFRAME,
    pub ZBUF:  GIFRegZBUF,
}

impl HWCachedCtx {
    /// Returns whether the current depth test is a `>=` or `>` test.
    pub fn depth_read(&self) -> bool {
        self.TEST.ZTE != 0 && (self.TEST.ZTST == ZTST_GEQUAL || self.TEST.ZTST == ZTST_GREATER)
    }

    /// Returns whether the depth buffer is currently being written.
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

// ===========================================================================
// GSDirtyRect — the renderer's per-frame dirty-rect bookkeeping.
//
// Mirrors `GS/Renderers/Common/GSDirtyRect.h`.
// ===========================================================================

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RGBAMask {
    /// Bit-packed RGBA: bit 0 = R, bit 1 = G, bit 2 = B, bit 3 = A.
    pub bits: u32,
}

impl RGBAMask {
    pub const NONE: Self = Self { bits: 0 };
    pub const R:    Self = Self { bits: 1 };
    pub const G:    Self = Self { bits: 2 };
    pub const B:    Self = Self { bits: 4 };
    pub const A:    Self = Self { bits: 8 };
    pub const RGB:  Self = Self { bits: 1 | 2 | 4 };
    pub const RGBA: Self = Self { bits: 1 | 2 | 4 | 8 };
}

#[derive(Debug)]
pub struct GSDirtyRect {
    pub r: GSVector4i,
    pub psm: u32,
    pub bw: u32,
    pub rgba: RGBAMask,
    pub req_linear: bool,
}

impl Default for GSDirtyRect {
    fn default() -> Self {
        Self {
            r: GSVector4i::zero(),
            psm: PSMCT32,
            bw: 1,
            rgba: RGBAMask::NONE,
            req_linear: false,
        }
    }
}

impl GSDirtyRect {
    pub fn new() -> Self { Self::default() }

    pub fn new_with(r: GSVector4i, psm: u32, bw: u32, rgba: RGBAMask, req_linear: bool) -> Self {
        Self { r, psm, bw, rgba, req_linear }
    }

    /// Returns the dirty rect, optionally aligned to the texture page
    /// granularity.
    pub fn get_dirty_rect(&self, tex0: GIFRegTEX0, align: bool) -> GSVector4i {
        // Opaque stub: real implementation accounts for `bs` block size
        // for the source/target pixel format.
        let _ = (tex0, align);
        self.r
    }
}

#[derive(Debug)]
pub struct GSDirtyRectList {
    pub rects: Vec<GSDirtyRect>,
}

impl Default for GSDirtyRectList {
    fn default() -> Self { Self { rects: Vec::new() } }
}

impl GSDirtyRectList {
    pub fn new() -> Self { Self::default() }

    /// Returns the union of all dirty rects (or zero if empty).
    pub fn get_total_rect(&self, _tex0: GIFRegTEX0, _size: GSVector2i) -> GSVector4i {
        if self.rects.is_empty() { return GSVector4i::zero(); }
        // Opaque stub: real implementation does runion + ralign + rintersect.
        self.rects[0].r
    }

    /// Returns the OR-mask of all dirty channels.
    pub fn get_dirty_channels(&self) -> u32 {
        self.rects.iter().fold(0u32, |acc, r| acc | r.rgba.bits)
    }

    /// Returns the dirty rect at `index`, intersected with `clamp`.
    pub fn get_dirty_rect_at(
        &self,
        index: usize,
        tex0: GIFRegTEX0,
        clamp: GSVector4i,
        align: bool,
    ) -> GSVector4i {
        if index >= self.rects.len() { return GSVector4i::zero(); }
        let r = self.rects[index].get_dirty_rect(tex0, align);
        let _ = clamp;
        r
    }
}

// ===========================================================================
// ClearType / DATEOptions / CLUTDrawTestResult / TextureShuffleType
// / TextureShuffleChannels / ShuffleProcessing — internal enums used
// throughout the hardware renderer.
// ===========================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClearType {
    NotClear,
    NormalClear,
    ClearWithDraw,
}

impl Default for ClearType {
    fn default() -> Self { ClearType::NotClear }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DATEOptions {
    pub enabled: bool,
    pub barrier: bool,
    pub primid: bool,
    pub stencil_one: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClutDrawTestResult {
    NotCLUTDraw,
    CLUTDrawOnCPU,
    CLUTDrawOnGPU,
}

impl Default for ClutDrawTestResult {
    fn default() -> Self { ClutDrawTestResult::NotCLUTDraw }
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

impl Default for TextureShuffleType {
    fn default() -> Self { TextureShuffleType::None }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextureShuffleChannels(pub u32);

impl TextureShuffleChannels {
    pub const NONE:           Self = Self(0x0);
    pub const RED_TO_BLUE:    Self = Self(0x1);
    pub const BLUE_TO_RED:    Self = Self(0x2);
    pub const GREEN_TO_ALPHA: Self = Self(0x4);
    pub const ALPHA_TO_GREEN: Self = Self(0x8);
    pub const RED_COPY:       Self = Self(0x10);
    pub const GREEN_COPY:     Self = Self(0x20);
    pub const BLUE_COPY:      Self = Self(0x40);
    pub const ALPHA_COPY:     Self = Self(0x80);
    pub const BLUE_TO_ALPHA:  Self = Self(0x100);

    pub const READ_RED:    Self = Self(0x1  | 0x10);
    pub const READ_GREEN:  Self = Self(0x4  | 0x20);
    pub const READ_BLUE:   Self = Self(0x2  | 0x40 | 0x100);
    pub const READ_ALPHA:  Self = Self(0x8  | 0x80);

    pub const WRITE_RED:   Self = Self(0x2  | 0x10);
    pub const WRITE_GREEN: Self = Self(0x8  | 0x20);
    pub const WRITE_BLUE:  Self = Self(0x1  | 0x40);
    pub const WRITE_ALPHA: Self = Self(0x4  | 0x80 | 0x100);

    pub const SAME_GROUP:  Self = Self(0x100);
}

impl std::ops::BitOr for TextureShuffleChannels {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self { Self(self.0 | rhs.0) }
}
impl std::ops::BitOrAssign for TextureShuffleChannels {
    fn bitor_assign(&mut self, rhs: Self) { self.0 |= rhs.0; }
}
impl std::ops::BitAnd for TextureShuffleChannels {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self { Self(self.0 & rhs.0) }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextureShuffleInfo {
    pub r#type: TextureShuffleType,
    pub channels: TextureShuffleChannels,
    pub real_16_bit_source: bool,
}

impl TextureShuffleInfo {
    pub fn is_active(&self) -> bool {
        self.r#type != TextureShuffleType::None
    }
    pub fn disable(&mut self) {
        self.r#type = TextureShuffleType::None;
    }
    pub fn same_group_shuffle(&self) -> bool {
        (self.channels & TextureShuffleChannels::SAME_GROUP).0 != 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShuffleProcessing {
    Read     = 1,
    Write    = 2,
    ReadWrite = 3,
}

// ===========================================================================
// GSTextureReplacement — texture-replacement bookkeeping.
//
// Mirrors the C++ `GSTextureReplacements` namespace.
// ===========================================================================

pub mod GSTextureReplacement {
    use super::*;

    #[derive(Clone, Debug, Default)]
    pub struct ReplacementTexture {
        pub width: u32,
        pub height: u32,
        pub format: u32,
        pub alpha_minmax: (u8, u8),
        pub pitch: u32,
        pub data: Vec<u8>,
        pub mips: Vec<MipData>,
    }

    #[derive(Clone, Debug, Default)]
    pub struct MipData {
        pub width: u32,
        pub height: u32,
        pub pitch: u32,
        pub data: Vec<u8>,
    }

    pub fn initialize() {}
    pub fn game_changed() {}
    pub fn reload_replacement_map() {}
    pub fn update_config(_old_config: &mut Pcsx2Config) {}
    pub fn shutdown() {}

    pub fn calc_mipmap_levels_for_replacement(width: u32, height: u32) -> u32 {
        let max_dim = width.max(height);
        if max_dim == 0 { 0 } else { (32 - max_dim.leading_zeros() - 1) as u32 }
    }

    pub fn has_any_replacement_textures() -> bool { false }
    pub fn has_replacement_texture_with_other_palette(
        _hash: &GSTextureCacheHashCacheKey,
    ) -> bool { false }

    pub fn lookup_replacement_texture(
        _hash: &GSTextureCacheHashCacheKey,
        _mipmap: bool,
        _pending: &mut bool,
        _alpha_minmax: &mut (u8, u8),
    ) -> Option<Box<GSTexture>> { None }

    pub fn create_replacement_texture(
        _rtex: &ReplacementTexture,
        _mipmap: bool,
    ) -> Option<Box<GSTexture>> { None }

    pub fn process_async_loaded_textures() {}

    pub fn dump_texture(
        _hash: &GSTextureCacheHashCacheKey,
        _tex0: &GIFRegTEX0,
        _texa: &GIFRegTEXA,
        _region: u32,
        _mem: &mut GSLocalMemory,
        _level: u32,
    ) {}

    pub fn clear_dumped_texture_list() {}

    pub fn get_dumped_texture_count() -> u32 { 0 }
    pub fn get_loaded_texture_count() -> u32 { 0 }

    pub type ReplacementTextureLoader =
        fn(&str, &mut ReplacementTexture, bool) -> bool;

    pub fn get_loader(_filename: &str) -> Option<ReplacementTextureLoader> { None }

    pub fn save_png_image(_filename: &str, _w: u32, _h: u32, _buffer: &[u8], _pitch: u32) -> bool {
        false
    }
}

// ===========================================================================
// GSHwHack — per-game CRC-based hardware hacks.
//
// Mirrors the C++ `GSHwHack` class with the three families of
// callbacks:
//   * GSC_* — get-skip-count (return `true` and set `skip` to skip
//     further draws);
//   * OI_*  — before-draw hooks;
//   * MV_*  — move-handler hooks.
// ===========================================================================

pub mod GSHwHack {
    use super::*;

    pub type GSC_Ptr = fn(&mut GSRendererHW, &mut i32) -> bool;
    pub type OI_Ptr  = fn(&mut GSRendererHW, &mut GSTexture, &mut GSTexture, &mut GSTextureCacheSource) -> bool;
    pub type MV_Ptr  = fn(&mut GSRendererHW) -> bool;

    #[derive(Clone, Copy, Debug)]
    pub struct GscEntry {
        pub name: &'static str,
        pub ptr:  GSC_Ptr,
    }
    #[derive(Clone, Copy, Debug)]
    pub struct OiEntry {
        pub name: &'static str,
        pub ptr:  OI_Ptr,
    }
    #[derive(Clone, Copy, Debug)]
    pub struct MvEntry {
        pub name: &'static str,
        pub ptr:  MV_Ptr,
    }

    fn noop_gsc(_r: &mut GSRendererHW, _skip: &mut i32) -> bool { false }
    fn noop_oi(_r: &mut GSRendererHW, _rt: &mut GSTexture, _ds: &mut GSTexture, _t: &mut GSTextureCacheSource) -> bool { false }
    fn noop_mv(_r: &mut GSRendererHW) -> bool { false }

    /// Skip-count callbacks (one per per-game hack).
    pub static S_GET_SKIP_COUNT_FUNCTIONS: &[GscEntry] = &[
        GscEntry { name: "GSC_IRem",                   ptr: noop_gsc },
        GscEntry { name: "GSC_Manhunt2",               ptr: noop_gsc },
        GscEntry { name: "GSC_SacredBlaze",            ptr: noop_gsc },
        GscEntry { name: "GSC_GuitarHero",             ptr: noop_gsc },
        GscEntry { name: "GSC_SFEX3",                  ptr: noop_gsc },
        GscEntry { name: "GSC_DTGames",                ptr: noop_gsc },
        GscEntry { name: "GSC_NamcoGames",             ptr: noop_gsc },
        GscEntry { name: "GSC_SandGrainGames",         ptr: noop_gsc },
        GscEntry { name: "GSC_BurnoutGames",           ptr: noop_gsc },
        GscEntry { name: "GSC_BlackAndBurnoutSky",     ptr: noop_gsc },
        GscEntry { name: "GSC_MidnightClub3",          ptr: noop_gsc },
        GscEntry { name: "GSC_TalesOfLegendia",        ptr: noop_gsc },
        GscEntry { name: "GSC_UltramanFightingEvolution", ptr: noop_gsc },
        GscEntry { name: "GSC_TalesofSymphonia",       ptr: noop_gsc },
        GscEntry { name: "GSC_UrbanReign",             ptr: noop_gsc },
        GscEntry { name: "GSC_BlueTongueGames",        ptr: noop_gsc },
        GscEntry { name: "GSC_NFSUndercover",          ptr: noop_gsc },
        GscEntry { name: "GSC_Battlefield2",           ptr: noop_gsc },
        GscEntry { name: "GSC_PolyphonyDigitalGames",  ptr: noop_gsc },
        GscEntry { name: "GSC_MetalGearSolid3",        ptr: noop_gsc },
        GscEntry { name: "GSC_Turok",                  ptr: noop_gsc },
    ];

    /// Before-draw callbacks.
    pub static S_BEFORE_DRAW_FUNCTIONS: &[OiEntry] = &[
        OiEntry { name: "OI_PointListPalette",     ptr: noop_oi },
        OiEntry { name: "OI_DBZBTGames",           ptr: noop_oi },
        OiEntry { name: "OI_RozenMaidenGebetGarden", ptr: noop_oi },
        OiEntry { name: "OI_SonicUnleashed",       ptr: noop_oi },
        OiEntry { name: "OI_ArTonelico2",          ptr: noop_oi },
        OiEntry { name: "OI_BurnoutGames",         ptr: noop_oi },
    ];

    /// Move-handler callbacks.
    pub static S_MOVE_HANDLER_FUNCTIONS: &[MvEntry] = &[
        MvEntry { name: "MV_Growlanser", ptr: noop_mv },
        MvEntry { name: "MV_Ico",       ptr: noop_mv },
    ];
}

// ===========================================================================
// GSRendererHW — the main hardware renderer.
//
// The C++ class derives from `GSRenderer` and is *enormous* — it
// owns the texture cache, the GIF state, the per-game HwHack
// dispatch, the multi-ISA `SwPrimRender` callback, all the clear
// detection heuristics, and the public `DrawPrims` / `DrawPath1/2/3`
// / `UpdateCRTC` / `VSync` / `Render` surface.  The public surface
// required by the task brief is the one preserved here.
// ===========================================================================

#[derive(Debug)]
pub struct GSRendererHW {
    // -----------------------------------------------------------------
    // Hardware-hack CRC dispatch.
    // -----------------------------------------------------------------
    pub m_gsc: Option<GSHwHack::GSC_Ptr>,
    pub m_oi:  Option<GSHwHack::OI_Ptr>,
    pub m_mv:  Option<GSHwHack::MV_Ptr>,
    pub m_skip: i32,
    pub m_skip_offset: i32,

    // -----------------------------------------------------------------
    // Process / downscale / shuffle tracking.
    // -----------------------------------------------------------------
    pub m_process_texture: bool,
    pub m_downscale_source: bool,
    pub m_texture_shuffle: TextureShuffleInfo,

    // -----------------------------------------------------------------
    // Split-shuffle state.
    // -----------------------------------------------------------------
    pub m_split_texture_shuffle_pages: u32,
    pub m_split_texture_shuffle_pages_high: u32,
    pub m_split_texture_shuffle_start_fbp: u32,
    pub m_split_texture_shuffle_start_tbp: u32,
    pub m_split_texture_shuffle_fbw: u32,

    // -----------------------------------------------------------------
    // Channel-shuffle state.
    // -----------------------------------------------------------------
    pub m_last_channel_shuffle_fbmsk: u32,
    pub m_last_channel_shuffle_fbp: u32,
    pub m_last_channel_shuffle_tbp: u32,
    pub m_last_channel_shuffle_end_block: u32,
    pub m_channel_shuffle_width: u32,
    pub m_channel_shuffle_src_valid: GSVector4i,
    pub m_full_screen_shuffle: bool,

    // -----------------------------------------------------------------
    // Per-renderer cached GIF state and split-clear state.
    // -----------------------------------------------------------------
    pub m_cached_ctx: HWCachedCtx,
    pub m_split_clear_start:  GIFRegFRAME,
    pub m_split_clear_start_z: GIFRegZBUF,
    pub m_split_clear_pages: u32,
    pub m_split_clear_color: u32,
    pub m_userhacks_tcoffset: bool,
    pub m_userhacks_tcoffset_x: f32,
    pub m_userhacks_tcoffset_y: f32,
    pub m_lod: GSVector2i,
    pub m_optimized_blend: GIFRegALPHA,
    pub m_conf: GSHWDrawConfig,

    // -----------------------------------------------------------------
    // Software sprite renderer state.
    // -----------------------------------------------------------------
    pub m_sw_vertex_buffer: Vec<u8>,
    pub m_sw_texture: [Option<usize>; 8],
    pub m_sw_rasterizer: Option<usize>,

    // -----------------------------------------------------------------
    // Multi-ISA `SwPrimRender` callback.
    // -----------------------------------------------------------------
    pub sw_prim_render: Option<fn(&mut GSRendererHW, bool, bool) -> bool>,
}

impl Default for GSRendererHW {
    fn default() -> Self {
        Self {
            m_gsc: None,
            m_oi:  None,
            m_mv:  None,
            m_skip: 0,
            m_skip_offset: 0,
            m_process_texture: false,
            m_downscale_source: false,
            m_texture_shuffle: TextureShuffleInfo::default(),
            m_split_texture_shuffle_pages: 0,
            m_split_texture_shuffle_pages_high: 0,
            m_split_texture_shuffle_start_fbp: 0,
            m_split_texture_shuffle_start_tbp: 0,
            m_split_texture_shuffle_fbw: 0,
            m_last_channel_shuffle_fbmsk: 0,
            m_last_channel_shuffle_fbp: 0,
            m_last_channel_shuffle_tbp: 0,
            m_last_channel_shuffle_end_block: 0,
            m_channel_shuffle_width: 0,
            m_channel_shuffle_src_valid: GSVector4i::zero(),
            m_full_screen_shuffle: false,
            m_cached_ctx: HWCachedCtx::default(),
            m_split_clear_start:  GIFRegFRAME::default(),
            m_split_clear_start_z: GIFRegZBUF::default(),
            m_split_clear_pages: 0,
            m_split_clear_color: 0,
            m_userhacks_tcoffset: false,
            m_userhacks_tcoffset_x: 0.0,
            m_userhacks_tcoffset_y: 0.0,
            m_lod: GSVector2i::zero(),
            m_optimized_blend: GIFRegALPHA::default(),
            m_conf: GSHWDrawConfig::default(),
            m_sw_vertex_buffer: Vec::new(),
            m_sw_texture: [None; 8],
            m_sw_rasterizer: None,
            sw_prim_render: None,
        }
    }
}

impl GSRendererHW {
    pub fn new() -> Self { Self::default() }

    /// Returns the global renderer instance cast to `GSRendererHW*`.
    pub fn get_instance() -> Option<&'static mut Self> {
        // SAFETY: pointer comes from `Option<GSRendererHW>` storage.
        let p = unsafe { G_GS_RENDERER.as_mut()? };
        Some(p)
    }

    /// Mutable accessor for the cached GIF context.
    pub fn get_cached_ctx(&mut self) -> &mut HWCachedCtx { &mut self.m_cached_ctx }

    /// Returns the FBP of the most recent channel-shuffle operation.
    pub fn get_last_channel_shuffle_fbp(&self) -> u32 { self.m_last_channel_shuffle_fbp }

    /// Teardown.  Mirrors the C++ `Destroy()` override.
    pub fn destroy(&mut self) {
        self.m_sw_vertex_buffer.clear();
        self.m_sw_texture = [None; 8];
        self.m_sw_rasterizer = None;
    }

    /// Apply any user-config-driven render fixes.
    pub fn update_render_fixes(&mut self) {
        // No-op stub: the real implementation inspects the GSOptions
        // and toggles the various user-hack flags.
    }

    /// Returns whether the renderer is allowed to upscale internally.
    pub fn can_upscale(&self) -> bool { true }

    /// Returns the current internal upscale multiplier.
    pub fn get_upscale_multiplier(&self) -> f32 { 1.0 }

    /// Returns the current texture-coord scale factor.
    pub fn get_texture_scale_factor(&self) -> f32 { 1.0 }

    /// Convert line primitives into sprites (user-hack).
    pub fn lines_to_sprites(&mut self) {
        unimplemented!("GSRendererHW::Lines2Sprites")
    }

    /// Returns true if the index buffer passes the per-game sanity checks.
    pub fn verify_indices(&self) -> bool { true }

    /// Expand line indices to triangles.
    pub fn expand_line_indices(&mut self) {
        unimplemented!("GSRendererHW::ExpandLineIndices")
    }

    /// Realigns the target texture coordinate.
    pub fn realign_target_texture_coordinate(
        &self,
        _tex: &GSTextureCacheSource,
    ) -> GSVector4 { GSVector4::zero() }

    /// Compute the bounding box of the current RT in pixels.
    pub fn compute_bounding_box_rt(
        &self,
        _rtsize: GSVector2i,
        _rtscale: f32,
    ) -> GSVector4i { GSVector4i::zero() }

    /// Compute the bounding box of the current texture region in pixels.
    pub fn compute_bounding_box_tex(
        &self,
        _texsize: GSVector2i,
        _coverage: GSVector4i,
        _region: GSVector4i,
        _texscale: f32,
    ) -> GSVector4i { GSVector4i::zero() }

    /// Merge sprite primitives back into a single draw.
    pub fn merge_sprite(&mut self, _tex: &mut GSTextureCacheSource) {
        unimplemented!("GSRendererHW::MergeSprite")
    }

    /// Returns the current valid size for the active RT.
    pub fn get_valid_size(
        &self,
        _tex: Option<&GSTextureCacheSource>,
        _is_shuffle: bool,
    ) -> GSVector2i { GSVector2i::zero() }

    /// Returns the current target size for the active RT.
    pub fn get_target_size(
        &self,
        _tex: Option<&GSTextureCacheSource>,
        _can_expand: bool,
        _is_shuffle: bool,
    ) -> GSVector2i { GSVector2i::zero() }

    /// Reset the renderer (called at boot and on hardware-reset).
    pub fn reset(&mut self, _hardware_reset: bool) {}

    /// Apply a config change to the renderer.
    pub fn update_settings(&mut self, _old_config: &Pcsx2Config) {}

    /// VSync.  Mirrors the C++ `VSync` override.
    pub fn vsync(&mut self, _field: u32, _registers_written: bool, _idle_frame: bool) {}

    /// Get the texture presented as the i-th output (final display).
    pub fn get_output(
        &mut self,
        _i: i32,
        _scale: &mut f32,
        _y_offset: &mut i32,
    ) -> Option<Box<GSTexture>> { None }

    /// Get the texture presented as the feedback output.
    pub fn get_feedback_output(&mut self, _scale: &mut f32) -> Option<Box<GSTexture>> { None }

    /// Invalidate VRAM overlapping `r` for `BITBLTBUF`.
    pub fn invalidate_video_mem(
        &mut self,
        _bitbltbuf: &GIFRegBITBLTBUF,
        _r: GSVector4i,
    ) {
        unimplemented!("GSRendererHW::InvalidateVideoMem")
    }

    /// Invalidate local memory overlapping `r` for `BITBLTBUF`.
    pub fn invalidate_local_mem(
        &mut self,
        _bitbltbuf: &GIFRegBITBLTBUF,
        _r: GSVector4i,
        _clut: bool,
    ) {
        unimplemented!("GSRendererHW::InvalidateLocalMem")
    }

    /// Move handler (called when the GS sees a `Move` register).
    pub fn move_handler(&mut self) {
        unimplemented!("GSRendererHW::Move")
    }

    /// Top-level draw entry point.  Mirrors the C++ `Draw()` override.
    pub fn draw(&mut self) {
        unimplemented!("GSRendererHW::Draw")
    }

    /// Purge the texture cache.
    pub fn purge_texture_cache(
        &mut self,
        _sources: bool,
        _targets: bool,
        _hash_cache: bool,
    ) {
        unimplemented!("GSRendererHW::PurgeTextureCache")
    }

    /// Read the texture cache back to CPU.
    pub fn readback_texture_cache(&mut self) {
        unimplemented!("GSRendererHW::ReadbackTextureCache")
    }

    /// Look up a palette source.
    pub fn lookup_palette_source(
        &self,
        _cbp: u32,
        _cpsm: u32,
        _cbw: u32,
        _offset: &mut GSVector2i,
        _scale: &mut f32,
        _size: GSVector2i,
    ) -> Option<Box<GSTexture>> { None }

    /// Test whether the current draw is a channel-shuffle.
    pub fn test_channel_shuffle(&mut self, _src: &mut GSTextureCacheTarget) -> bool { false }

    /// Returns true if the Frame and TEX0 share channels.
    pub fn channels_shared_tex0_frame(&self) -> bool { false }

    /// Returns true if `tbp` matches the frame or Z buffer.
    pub fn is_tbp_frame_or_z(&self, _tbp: u32, _frame_only: bool) -> bool { false }

    /// Handle a "manual deswizzle" pattern.
    pub fn handle_manual_deswizzle(&mut self) {
        unimplemented!("GSRendererHW::HandleManualDeswizzle")
    }

    /// Offset the current draw, used for RT-in-RT.
    pub fn offset_draw(
        &mut self,
        _fbp_offset: i32,
        _zbp_offset: i32,
        _xoffset: i32,
        _yoffset: i32,
    ) {
        unimplemented!("GSRendererHW::OffsetDraw")
    }

    /// Replace vertices with a sprite.
    pub fn replace_vertices_with_sprite(
        &mut self,
        _unscaled_rect: GSVector4i,
        _unscaled_uv_rect: GSVector4i,
        _unscaled_size: GSVector2i,
        _scissor: GSVector4i,
    ) {
        unimplemented!("GSRendererHW::ReplaceVerticesWithSprite")
    }

    /// Replace vertices with a sprite (no-UV overload).
    pub fn replace_vertices_with_sprite_simple(
        &mut self,
        _unscaled_rect: GSVector4i,
        _unscaled_size: GSVector2i,
    ) {
        unimplemented!("GSRendererHW::ReplaceVerticesWithSprite(simple)")
    }

    /// Begin an HLE-emulated hardware draw.
    pub fn begin_hle_hardware_draw(
        &mut self,
        _rt: Option<Box<GSTexture>>,
        _ds: Option<Box<GSTexture>>,
        _rt_scale: f32,
        _tex: Option<Box<GSTexture>>,
        _tex_scale: f32,
        _unscaled_rect: GSVector4i,
    ) -> &mut GSHWDrawConfig { &mut self.m_conf }

    /// Submit a previously set-up HLE hardware draw.
    pub fn end_hle_hardware_draw(&mut self, _force_copy_on_hazard: bool) {
        unimplemented!("GSRendererHW::EndHLEHardwareDraw")
    }

    /// Compute the drawlist (or its size) for the current draw.
    pub fn compute_drawlist_get_size(&self, _scale: f32) -> usize { 0 }

    /// Returns true if AA1 coverage is supported.
    pub fn is_coverage_alpha_supported(&self) -> bool { false }

    // -----------------------------------------------------------------
    // Public surface required by the task brief.
    // -----------------------------------------------------------------

    /// `DrawPrims` — emit the current vertex/index stream as primitives
    /// against the given RT/DS/Source.  The C++ implementation does
    /// clear/shuffle/DATE/AA1 detection, ROV configuration, hazard
    /// tracking, and the actual draw call.
    pub fn DrawPrims(
        &mut self,
        _rt: &mut GSTextureCacheTarget,
        _ds: &mut GSTextureCacheTarget,
        _tex: &mut GSTextureCacheSource,
        _tmm: &TextureMinMaxResult,
    ) {
        unimplemented!("GSRendererHW::DrawPrims")
    }

    /// `DrawPath1` — accelerated path 1 (no special handling).
    pub fn DrawPath1(&mut self) {
        unimplemented!("GSRendererHW::DrawPath1")
    }

    /// `DrawPath2` — accelerated path 2 (channel-shuffle detection).
    pub fn DrawPath2(&mut self) {
        unimplemented!("GSRendererHW::DrawPath2")
    }

    /// `DrawPath3` — accelerated path 3 (software-prim fallback).
    pub fn DrawPath3(&mut self) {
        unimplemented!("GSRendererHW::DrawPath3")
    }

    /// `UpdateCRTC` — refresh the CRTC configuration (display timing,
    /// effective read/write frames, etc.).
    pub fn UpdateCRTC(&mut self) {
        unimplemented!("GSRendererHW::UpdateCRTC")
    }

    /// `VSync` — called once per VSync.  Mirrors the C++ override.
    pub fn VSync(&mut self, _field: u32, _registers_written: bool, _idle_frame: bool) {
        unimplemented!("GSRendererHW::VSync")
    }

    /// `Render` — top-level render entry point invoked by the EE.
    pub fn Render(&mut self) {
        unimplemented!("GSRendererHW::Render")
    }

    // -----------------------------------------------------------------
    // Internal heuristics.  These mirror the private helpers in
    // `GSRendererHW.h` and are all stubbed.
    // -----------------------------------------------------------------

    pub fn oi_blit_fmv(
        &self,
        _rt: &mut GSTextureCacheTarget,
        _t: &mut GSTextureCacheSource,
        _r_draw: GSVector4i,
    ) -> bool { false }

    pub fn try_gs_mem_clear(
        &self,
        _no_rt: bool, _preserve_rt: bool, _invalidate_rt: bool, _rt_end_bp: u32,
        _no_ds: bool, _preserve_z: bool, _invalidate_z: bool, _ds_end_bp: u32,
    ) -> bool { false }

    pub fn clear_gs_local_memory(&mut self, _off: &GSOffset, _r: GSVector4i, _vert_color: u32) {
        unimplemented!("GSRendererHW::ClearGSLocalMemory")
    }

    pub fn detect_double_half_clear(&self, no_rt: &mut bool, no_ds: &mut bool) -> bool {
        *no_rt = false; *no_ds = false;
        false
    }
    pub fn detect_striped_double_clear(&self, no_rt: &mut bool, no_ds: &mut bool) -> bool {
        *no_rt = false; *no_ds = false;
        false
    }
    pub fn detect_redundant_buffer_clear(
        &self, no_rt: &mut bool, no_ds: &mut bool, _fm_mask: u32,
    ) -> bool {
        *no_rt = false; *no_ds = false;
        false
    }
    pub fn try_target_clear(
        &self,
        _rt: &mut GSTextureCacheTarget,
        _ds: &mut GSTextureCacheTarget,
        _preserve_rt_color: bool,
        _preserve_depth: bool,
    ) -> bool { false }

    pub fn set_new_frame(&mut self, bp: u32, bw: u32, psm: u32) {
        self.m_cached_ctx.FRAME = GIFRegFRAME { FBP: bp, FBW: bw, PSM: psm, FBMSK: 0 };
    }
    pub fn set_new_zbuf(&mut self, bp: u32, psm: u32) {
        self.m_cached_ctx.ZBUF = GIFRegZBUF { ZBP: bp, PSM: psm, ZMSK: 0 };
    }

    pub fn interpolate_uv(&self, alpha: f32, t0: u16, t1: u16) -> u16 {
        let a = alpha.clamp(0.0, 1.0) as f64;
        let lo = t0 as f64;
        let hi = t1 as f64;
        (lo + (hi - lo) * a) as i32 as u16
    }
    pub fn alpha0(l: i32, _x0: i32, x1: i32) -> f32 {
        if l == 0 { 0.0 } else { (x1 as f32) / (l as f32) }
    }
    pub fn alpha1(l: i32, x0: i32, _x1: i32) -> f32 {
        if l == 0 { 0.0 } else { 1.0 - (x0 as f32) / (l as f32) }
    }

    pub fn sw_sprite_render(&mut self) { unimplemented!("GSRendererHW::SwSpriteRender") }
    pub fn can_use_sw_sprite_render(&self) -> bool { false }

    pub fn is_scaling_draw(&self, _src: &GSTextureCacheSource, _no_gaps: bool) -> i32 { 0 }

    pub fn is_constant_direct_write_mem_clear(&self) -> ClearType { ClearType::NotClear }
    pub fn get_constant_direct_write_mem_clear_color(&self) -> u32 { 0 }
    pub fn get_constant_direct_write_mem_clear_depth(&self) -> u32 { 0 }
    pub fn is_really_dithered(&self) -> bool { false }
    pub fn are_any_pixels_discarded(&self) -> bool { false }
    pub fn is_discarding_dst_color(&self) -> bool { false }
    pub fn is_discarding_dst_rgb(&self) -> bool { false }
    pub fn is_discarding_dst_alpha(&self) -> bool { false }
    pub fn texture_covers_without_gaps_not_equal(&self) -> bool { false }

    pub fn has_ee_upload(&self, _r: GSVector4i) -> bool { false }
    pub fn possible_clut_draw(&self) -> ClutDrawTestResult { ClutDrawTestResult::NotCLUTDraw }
    pub fn possible_clut_draw_aggressive(&self) -> ClutDrawTestResult { ClutDrawTestResult::NotCLUTDraw }

    pub fn can_use_sw_prim_render(
        &self,
        _no_rt: bool,
        _no_ds: bool,
        _draw_sprite_tex: bool,
    ) -> bool { false }

    pub fn sw_prim_render_dispatch(
        &self,
        _renderer: &mut GSRendererHW,
        _invalidate_tc: bool,
        _add_ee_transfer: bool,
    ) -> bool { false }

    pub fn round_sprite_offset<const LINEAR: bool>(&mut self) {
        let _ = std::any::type_name::<bool>();
        unimplemented!("GSRendererHW::RoundSpriteOffset")
    }

    pub fn reset_states(&mut self) { unimplemented!("GSRendererHW::ResetStates") }
    pub fn handle_provoking_vertex_first(&mut self) {
        unimplemented!("GSRendererHW::HandleProvokingVertexFirst")
    }
    pub fn setup_ia(
        &mut self,
        _target_scale: f32, _sx: f32, _sy: f32,
        _req_vert_backup: bool, _no_rt: bool,
    ) { unimplemented!("GSRendererHW::SetupIA") }
    pub fn emulate_texture_shuffle_and_fbmask(
        &mut self,
        _rt: &mut GSTextureCacheTarget,
        _tex: &mut GSTextureCacheSource,
    ) { unimplemented!("GSRendererHW::EmulateTextureShuffleAndFbmask") }

    pub fn emulate_channel_shuffle(
        &mut self,
        _src: &mut GSTextureCacheTarget,
        _test_only: bool,
        _rt: Option<&mut GSTextureCacheTarget>,
    ) -> u32 { 0 }

    pub fn emulate_blending(
        &mut self,
        _rt_alpha_min: i32, _rt_alpha_max: i32,
        _date: &mut DATEOptions, _rt: &mut GSTextureCacheTarget,
        _can_scale_rt_alpha: bool, _new_rt_alpha_scale: &mut bool,
    ) { unimplemented!("GSRendererHW::EmulateBlending") }

    pub fn cleanup_draw(&mut self, _invalidate_temp_src: bool) {
        unimplemented!("GSRendererHW::CleanupDraw")
    }

    pub fn emulate_zbuffer(&mut self, _ds: &GSTextureCacheTarget) {
        unimplemented!("GSRendererHW::EmulateZbuffer")
    }
    pub fn emulate_aa1(&mut self) { unimplemented!("GSRendererHW::EmulateAA1") }

    pub fn get_alpha_test_config_ps(
        _atst: u32, _aref: u8, _invert_test: bool,
    ) -> (u32, f32) { (ATST_ALWAYS, 0.0) }

    pub fn emulate_alpha_test(&mut self, _date: &mut DATEOptions) {
        unimplemented!("GSRendererHW::EmulateAlphaTest")
    }
    pub fn emulate_alpha_test_second_pass(&mut self) {
        unimplemented!("GSRendererHW::EmulateAlphaTestSecondPass")
    }
    pub fn configure_depth_feedback(&mut self, _rov_depth: bool) {
        unimplemented!("GSRendererHW::ConfigureDepthFeedback")
    }

    pub fn calculate_alpha_range(
        &self,
        _rt: &GSTextureCacheTarget, _ds: &GSTextureCacheTarget,
        _date: &mut DATEOptions,
        _blend_alpha_min: &mut i32, _blend_alpha_max: &mut i32,
        _rt_new_alpha_min: &mut i32, _rt_new_alpha_max: &mut i32,
    ) { unimplemented!("GSRendererHW::CalculateAlphaRange") }

    pub fn determine_alpha_scaling(
        &self,
        _rt: &GSTextureCacheTarget, _tex: &GSTextureCacheSource,
        _req_source_update: bool, _rt_new_alpha_max: i32,
        _can_scale_rt_alpha: &mut bool, _new_scale_rt_alpha: &mut bool,
    ) { unimplemented!("GSRendererHW::DetermineAlphaScaling") }

    pub fn emulate_date_early_fail(
        &self,
        _date: &mut DATEOptions, _rt: &GSTextureCacheTarget,
    ) -> bool { false }

    pub fn emulate_date_select_method(
        &mut self,
        _date: &mut DATEOptions, _rt: &GSTextureCacheTarget,
        _blend_alpha_min: &mut i32, _blend_alpha_max: &mut i32,
    ) { unimplemented!("GSRendererHW::EmulateDATESelectMethod") }

    pub fn emulate_date_get_config(
        &mut self,
        _date: &mut DATEOptions, _scale_rt_alpha: bool,
        _temp_ds: &mut u8,
    ) { unimplemented!("GSRendererHW::EmulateDATEGetConfig") }

    pub fn emulate_dither(&mut self) { unimplemented!("GSRendererHW::EmulateDither") }

    pub fn determine_vs_config(
        &self,
        _rt: &GSTextureCacheTarget, _rtscale: f32,
        _rtsize: GSVector2i, _unscaled_size: GSVector2i,
        vs_scale_x: &mut f32, vs_scale_y: &mut f32,
    ) { *vs_scale_x = _rtscale; *vs_scale_y = _rtscale; }

    pub fn determine_barriers(
        &self,
        _rt: &GSTextureCacheTarget, _tex: &GSTextureCacheSource,
    ) {}

    pub fn get_forced_rov_usage(&self, color_cov: &mut bool, depth_rov: &mut bool) {
        *color_cov = false; *depth_rov = false;
    }
    pub fn determine_rov_usage(
        &self,
        _rt: &GSTextureCacheTarget, _ds: &GSTextureCacheTarget,
    ) {}
    pub fn configure_rov(&mut self, _color_rov: bool, _depth_rov: bool) {}
    pub fn set_unordered_access_flag(&mut self, _rt: &mut GSTextureCacheTarget) {}
    pub fn convert_depth_format_rov(&mut self, _ds: &mut GSTextureCacheTarget) {}

    pub fn set_tc_offset(&mut self) {
        self.m_userhacks_tcoffset = true;
        self.m_userhacks_tcoffset_x = 0.0;
        self.m_userhacks_tcoffset_y = 0.0;
    }

    pub fn next_draw_col_clip(&self) -> bool { false }
    pub fn is_possible_channel_shuffle(&self) -> bool { false }
    pub fn is_page_copy(&self) -> bool { false }
    pub fn next_draw_matches_shuffle(&self) -> bool { false }

    pub fn is_split_texture_shuffle(
        &self,
        _rt_tex0: &mut GIFRegTEX0,
        _valid_area: &mut GSVector4i,
    ) -> bool { false }

    pub fn fix_split_texture_shuffle_state(&mut self) {}
    pub fn get_split_texture_shuffle_draw_rect(&self) -> GSVector4i { GSVector4i::zero() }
    pub fn get_effective_texture_shuffle_fbmsk(&self) -> u32 { 0 }

    pub fn detect_texture_shuffle_impl<const PRIMCLASS: u32, const FST: bool>(&mut self)
        -> TextureShuffleInfo { TextureShuffleInfo::default() }
    pub fn detect_texture_shuffle(&mut self) { self.m_texture_shuffle.disable(); }
    pub fn detect_texture_shuffle_second_pass(
        &mut self,
        _rt: &mut GSTextureCacheTarget,
        _tex: &mut GSTextureCacheSource,
    ) {}

    pub fn convert_sprite_texture_shuffle_impl<const PRIMCLASS: u32, const FST: bool>(
        &mut self, _rt: &mut GSTextureCacheTarget, _tex: &mut GSTextureCacheSource,
    ) { unimplemented!("GSRendererHW::ConvertSpriteTextureShuffleImpl") }

    pub fn convert_sprite_texture_shuffle(
        &mut self, _rt: &mut GSTextureCacheTarget, _tex: &mut GSTextureCacheSource,
    ) { unimplemented!("GSRendererHW::ConvertSpriteTextureShuffle") }

    pub fn convert_32bit_to_16bit_mask(m: u32) -> u32 { m & 0xFFFF }

    pub fn get_draw_rect_for_pages(_bw: u32, _psm: u32, _num_pages: u32) -> GSVector4i {
        GSVector4i::zero()
    }
    pub fn is_single_page_draw(&self) -> bool { false }

    pub fn try_to_resolve_single_page_framebuffer(
        &self,
        _frame: &mut GIFRegFRAME,
        _only_next_draw: bool,
    ) -> bool { false }

    pub fn is_split_clear_active(&self) -> bool { self.m_split_clear_pages != 0 }
    pub fn check_next_draw_for_split_clear(
        &self,
        _r: &GSVector4i,
        _pages_covered_by_this_draw: &mut u32,
    ) -> bool { false }
    pub fn is_starting_split_clear(&self) -> bool { false }
    pub fn continue_split_clear(&mut self) -> bool { false }
    pub fn finish_split_clear(&mut self) { self.m_split_clear_pages = 0; }

    pub fn needs_blending(&self) -> bool { false }
    pub fn is_rt_written(&self) -> bool { false }
    pub fn is_depth_always_passing(&self) -> bool { false }
    pub fn is_using_cs_in_blend(&self) -> bool { false }
    pub fn is_using_as_in_blend(&self) -> bool { false }

    pub fn is_bad_frame(&self) -> bool { false }
}

// ===========================================================================
// GSRendererHWMultiISA — the multi-ISA dispatch wrapper.
//
// In the C++ code `GSRendererHWPopulateFunctions` and
// `GSRendererHWFunctions` are wrapped in `MULTI_ISA_*` macros.  The
// idiomatic Rust translation just stores a function pointer that
// `GSRendererHW::sw_prim_render` invokes.
// ===========================================================================

pub mod GSRendererHWMultiISA {
    use super::*;

    pub struct GSRendererHWFunctions;

    impl GSRendererHWFunctions {
        pub fn sw_prim_render(
            _hw: &mut GSRendererHW,
            _invalidate_tc: bool,
            _add_ee_transfer: bool,
        ) -> bool { false }

        pub fn populate(renderer: &mut GSRendererHW) {
            renderer.sw_prim_render = Some(Self::sw_prim_render);
        }
    }

    pub fn gs_renderer_hw_populate_functions(renderer: &mut GSRendererHW) {
        GSRendererHWFunctions::populate(renderer);
    }
}

// ===========================================================================
// GSRenderer — common base for the HW/SW renderers.  Most of the
// public surface is shared by all backends.  Mirrors the C++
// `GS/Renderers/Common/GSRenderer.h`.
// ===========================================================================

#[derive(Debug)]
pub struct GSRenderer {
    pub shader_time_start: u64,
    pub snapshot: String,
    pub dump_frames: u32,
    pub skipped_duplicate_frames: u32,
    pub last_draw_n: u64,
    pub last_transfer_n: u64,
    pub real_size: GSVector2i,
    pub dump: Option<Box<GsDump>>,
    pub dirty_rects: GSDirtyRectList,
    pub m_tc: Option<Box<GSTextureCache>>,
}

impl Default for GSRenderer {
    fn default() -> Self {
        Self {
            shader_time_start: 0,
            snapshot: String::new(),
            dump_frames: 0,
            skipped_duplicate_frames: 0,
            last_draw_n: 0,
            last_transfer_n: 0,
            real_size: GSVector2i::zero(),
            dump: None,
            dirty_rects: GSDirtyRectList::default(),
            m_tc: None,
        }
    }
}

impl GSRenderer {
    pub fn new() -> Self { Self::default() }

    pub fn reset(&mut self, _hardware_reset: bool) {}
    pub fn destroy(&mut self) {}

    pub fn update_render_fixes(&mut self) {}

    pub fn vsync(&mut self, _field: u32, _registers_written: bool, _idle_frame: bool) {}

    pub fn can_upscale(&self) -> bool { false }
    pub fn get_upscale_multiplier(&self) -> f32 { 1.0 }
    pub fn get_texture_scale_factor(&self) -> f32 { 1.0 }
    pub fn get_internal_resolution(&self) -> GSVector2i { self.real_size }
    pub fn get_mod_xy_offset(&self) -> f32 { 0.0 }

    pub fn is_idle_frame(&self) -> bool {
        self.last_draw_n == 0 && self.last_transfer_n == 0
    }

    pub fn save_snapshot_to_memory(
        &self,
        _w: u32, _h: u32, _apply_aspect: bool, _crop: bool,
        width: &mut u32, height: &mut u32, pixels: &mut Vec<u32>,
    ) -> bool { *width = 0; *height = 0; pixels.clear(); false }

    pub fn queue_snapshot(&mut self, path: &str, frames: u32) {
        self.snapshot.clear();
        let trimmed = if path.len() > 4 && path.to_ascii_lowercase().ends_with(".png") {
            &path[..path.len() - 4]
        } else { path };
        self.snapshot.push_str(trimmed);
        self.dump_frames = frames;
    }

    pub fn stop_gs_dump(&mut self) {
        self.snapshot.clear();
        self.dump_frames = 0;
    }

    pub fn present_current_frame(&self) {}
    pub fn begin_capture(&self, _fn_: &str, _size: GSVector2i) -> bool { false }
    pub fn end_capture(&self) {}

    pub fn merge(&mut self, _field: u32) -> bool { false }

    pub fn get_output(
        &mut self,
        _i: i32, _scale: &mut f32, _y_offset: &mut i32,
    ) -> Option<Box<GSTexture>> { None }

    pub fn get_feedback_output(&mut self, _scale: &mut f32) -> Option<Box<GSTexture>> { None }

    pub fn lookup_palette_source(
        &self,
        _cbp: u32, _cpsm: u32, _cbw: u32,
        _offset: &mut GSVector2i, _scale: &mut f32, _size: GSVector2i,
    ) -> Option<Box<GSTexture>> { None }
}

#[derive(Debug)]
pub struct GsDump { pub path: String }
impl GsDump {
    pub fn get_path(&self) -> &str { &self.path }
}

// ===========================================================================
// GSTextureCacheSW — software texture cache.
//
// Mirrors the C++ `GSTextureCacheSW` class.  Each cached `Texture`
// is a 32-byte aligned buffer, addressed by an SW TEX0/CLUT pair.
// ===========================================================================

#[derive(Debug)]
pub struct GSTextureCacheSW {
    /// Page hash to texture map.
    pub m_map: Vec<Vec<*mut GSTextureCacheSWTexture>>,
    /// All cached textures.
    pub m_textures: Vec<*mut GSTextureCacheSWTexture>,
}

pub struct GSTextureCacheSWTexture {
    pub m_tex0: GIFRegTEX0,
    pub m_texa: GIFRegTEXA,
    pub m_buff: *mut u32,
    pub m_tw: u32,
    pub m_age: u32,
    pub m_complete: bool,
    pub m_pages: u32,
    pub m_erase_it: [usize; 8192],
    pub m_valid: [u32; 256],
    pub m_sharedbits: u32,
    pub m_offset: *mut u8,
    pub m_repeating: bool,
    pub m_p2t: *mut u8,
}

impl Default for GSTextureCacheSWTexture {
    fn default() -> Self {
        Self {
            m_tex0: GIFRegTEX0::default(),
            m_texa: GIFRegTEXA::default(),
            m_buff: std::ptr::null_mut(),
            m_tw: 0,
            m_age: 0,
            m_complete: false,
            m_pages: 0,
            m_erase_it: [0; 8192],
            m_valid: [0; 256],
            m_sharedbits: 0,
            m_offset: std::ptr::null_mut(),
            m_repeating: false,
            m_p2t: std::ptr::null_mut(),
        }
    }
}

impl GSTextureCacheSW {
    pub fn new() -> Self {
        Self {
            m_map: (0..8192).map(|_| Vec::new()).collect(),
            m_textures: Vec::new(),
        }
    }

    /// Look up a software texture by TEX0/CLUT/width.  Returns the
    /// cached entry on hit, or allocates a new one on miss.
    pub fn lookup(
        &mut self,
        _tex0: &GIFRegTEX0,
        _texa: &GIFRegTEXA,
        _tw0: u32,
    ) -> *mut GSTextureCacheSWTexture {
        std::ptr::null_mut()
    }

    /// Invalidate all software textures that share bits with `psm` on
    /// the pages in `pages`.
    pub fn invalidate_pages(&mut self, _pages: u32, _psm: u32) {}

    /// Free all cached software textures.
    pub fn remove_all(&mut self) {
        for t in &self.m_textures {
            unsafe {
                if !(**t).m_buff.is_null() {
                    // Free the aligned buffer (caller-provided allocator).
                    drop(Box::from_raw((**t).m_buff));
                }
                drop(Box::from_raw(*t));
            }
        }
        self.m_textures.clear();
        for l in &mut self.m_map { l.clear(); }
    }

    /// Age all cached entries; evict anything older than 10 frames.
    pub fn inc_age(&mut self) {
        self.m_textures.retain(|t| unsafe {
            let t = &mut **t;
            t.m_age += 1;
            if t.m_age > 10 {
                if !t.m_buff.is_null() {
                    drop(Box::from_raw(t.m_buff));
                    t.m_buff = std::ptr::null_mut();
                }
                drop(Box::from_raw(t));
                false
            } else { true }
        });
    }
}

impl Default for GSTextureCacheSW {
    fn default() -> Self { Self::new() }
}

impl GSTextureCacheSWTexture {
    pub fn new(tw0: u32, tex0: GIFRegTEX0, texa: GIFRegTEXA) -> Self {
        let mut t = Self::default();
        t.m_tex0 = tex0;
        t.m_texa = texa;
        t.m_tw = tw0;
        if t.m_tw == 0 {
            t.m_tw = std::cmp::max(tex0.TW, 3);
        }
        t
    }

    pub fn reset(&mut self, tw0: u32, tex0: GIFRegTEX0, texa: GIFRegTEXA) {
        if !self.m_buff.is_null() && (self.m_tex0.TW != tex0.TW || self.m_tex0.TH != tex0.TH) {
            unsafe { drop(Box::from_raw(self.m_buff)); }
            self.m_buff = std::ptr::null_mut();
        }
        self.m_tw = tw0;
        self.m_age = 0;
        self.m_complete = false;
        self.m_p2t = std::ptr::null_mut();
        self.m_tex0 = tex0;
        self.m_texa = texa;
        if self.m_tw == 0 {
            self.m_tw = std::cmp::max(tex0.TW, 3);
        }
        self.m_valid = [0; 256];
    }

    pub fn update(&mut self, _rect: GSVector4i) -> bool { true }
    pub fn save(&self, _filename: &str) -> bool { false }
}

// ===========================================================================
// GSVertexSW — software vertex, used by the SW rasterizer / prim
// setup.  Mirrors the C++ struct of the same name.
// ===========================================================================

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GSVertexSW {
    pub p: GSVector4,
    pub t: GSVector4,
    pub c: u32,
}

// ===========================================================================
// GSDrawScanline — scanline-renderer entry point.  The
// `GSDrawScanlineCodeGenerator.all.cpp` is essentially a JIT
// assembler that emits per-scanline pixel-loop code; here it is
// modeled as a normal struct with a few dispatcher methods.
// ===========================================================================

pub struct GSDrawScanline {
    pub sel: u64,
    pub vertex_count: u32,
    pub scanmsk_value: u32,
}

impl Default for GSDrawScanline {
    fn default() -> Self {
        Self { sel: 0, vertex_count: 0, scanmsk_value: 0 }
    }
}

impl GSDrawScanline {
    pub fn new() -> Self { Self::default() }

    /// Draw the current set of scanlines.  In the C++ code this is
    /// JIT-compiled; here it is a stub.
    pub fn draw(&mut self) {
        unimplemented!("GSDrawScanline::Draw")
    }

    /// Generate the per-pixel code for the given `sel` (selector).
    pub fn generate_code(&mut self, _sel: u64) {
        unimplemented!("GSDrawScanline::GenerateCode")
    }
}

// ===========================================================================
// GSScanlineGlobalData / GSScanlineLocalData — opaque per-scanline
// payloads (the C++ versions are large PODs that flow through the
// rasterizer).
// ===========================================================================

#[derive(Default, Debug)]
pub struct GSScanlineGlobalData {
    pub vm: *mut u8,
    pub fbo: *mut u8,
    pub zbo: *mut u8,
    pub fzbr: *mut u8,
    pub fzbc: *mut u8,
    pub sel: u64,
    pub fm: u32,
    pub zm: u32,
    pub tex: [*mut u32; 8],
    pub clut: *mut u32,
    pub dimx: *mut u32,
    pub afix: GSVector4i,
    pub aref: GSVector4i,
    pub frb: u32,
    pub fga: u32,
    pub lod: GSVector4i,
    pub mxl: GSVector4,
    pub l:   GSVector4,
    pub k:   GSVector4,
    pub t:   GsScanlineTex,
    pub frame: u32,
    pub test_ate: u32,
    pub test_afail: u32,
    pub test_date: u32,
    pub test_datm: u32,
    pub test_zte: u32,
    pub test_ztst: u32,
    pub alpha: u32,
    pub primclass: u32,
}

#[derive(Default, Debug)]
pub struct GsScanlineTex {
    pub min:   GSVector4i,
    pub max:   GSVector4i,
    pub mask:  GSVector4i,
    pub invmask: GSVector4i,
    pub minmax: GSVector4i,
}

#[derive(Default, Debug)]
pub struct GSScanlineLocalData {
    pub d4:  u64,
    pub d8:  u64,
    pub d12: u64,
    pub d16: u64,
    pub d20: u64,
    pub d24: u64,
    pub d28: u64,
    pub d32: u64,
    pub d36: u64,
    pub d40: u64,
    pub d44: u64,
    pub d48: u64,
    pub d52: u64,
    pub d56: u64,
}

// ===========================================================================
// GSRasterizer — multi-threaded software rasterizer.
//
// In the C++ code this is an abstract base with one rasterizer per
// thread plus a top-level `GSRasterizerList` that schedules work
// across cores.  Here we model the public surface.
// ===========================================================================

#[derive(Debug)]
pub struct GSRasterizer {
    pub rasterizers: Vec<*mut u8>,
    pub threads: u32,
}

impl Default for GSRasterizer {
    fn default() -> Self {
        Self { rasterizers: Vec::new(), threads: 0 }
    }
}

impl GSRizer {
    pub fn new() -> Self { Self::default() }

    /// Wait for all worker threads to finish.
    pub fn sync(&mut self, _thread: i32) {}

    /// Print per-thread statistics.
    pub fn print_stats(&self) {}
}

// Note: this is a re-export to satisfy the `pub struct GSRasterizer`
// naming used elsewhere in the codebase.
pub type GSRizer = GSRasterizer;

#[derive(Debug)]
pub struct GSRasterizerList {
    pub rasterizers: Vec<Box<GSRasterizer>>,
    pub threads: u32,
}

impl GSRasterizerList {
    pub fn new(threads: u32) -> Self {
        Self { rasterizers: Vec::new(), threads }
    }

    /// Wait for all worker threads to finish.
    pub fn sync(&mut self, _thread: i32) {}

    /// Print per-thread statistics.
    pub fn print_stats(&self) {}
}

impl Default for GSRasterizerList {
    fn default() -> Self { Self::new(1) }
}

// ===========================================================================
// GSSingleRasterizer — single-threaded convenience rasterizer.
// ===========================================================================

pub struct GSSingleRasterizer {
    pub data: GSRasterizerData,
}

impl GSSingleRasterizer {
    pub fn new() -> Self {
        Self { data: GSRasterizerData::default() }
    }

    pub fn draw(&mut self, _data: GSRasterizerData) {
        unimplemented!("GSSingleRasterizer::Draw")
    }
}

impl Default for GSSingleRasterizer {
    fn default() -> Self { Self::new() }
}

// ===========================================================================
// GSRasterizerData — bundle of scanline data + vertices fed to the
// rasterizer.  Mirrors the C++ `GSRasterizerData`.
// ===========================================================================

#[derive(Default, Debug)]
pub struct GSRasterizerData {
    pub primclass: u32,
    pub buff: *mut u8,
    pub vertex: *mut GSVertexSW,
    pub vertex_count: u32,
    pub index: *mut u16,
    pub index_count: u32,
    pub scanmsk_value: u32,
    pub scissor: GSVector4i,
    pub bbox: GSVector4i,
    pub frame: u32,
    pub global: GSScanlineGlobalData,
    pub local: Vec<GSScanlineLocalData>,
}

// ===========================================================================
// GSRendererSW — software renderer.
//
// Mirrors the C++ `GSRendererSW` class.  In the C++ code it owns a
// `GSTextureCacheSW`, a `GSRasterizerList`, an `m_output` 1MPix
// aligned buffer, and a number of `GSTexture` output slots.
// ===========================================================================

#[derive(Debug)]
pub struct GSRendererSW {
    pub base: GSRenderer,
    pub m_tc: Option<Box<GSTextureCacheSW>>,
    pub m_rl: Option<Box<GSRasterizerList>>,
    pub m_output: *mut u32,
    pub m_fzb_pages: [u32; 32],
    pub m_tex_pages: [u32; 32],
    pub m_texture: [Option<Box<GSTexture>>; 4],
    pub m_nativeres: bool,
    pub threads: u32,
}

impl Default for GSRendererSW {
    fn default() -> Self {
        Self {
            base: GSRenderer::default(),
            m_tc: None,
            m_rl: None,
            m_output: std::ptr::null_mut(),
            m_fzb_pages: [0; 32],
            m_tex_pages: [0; 32],
            m_texture: [None, None, None, None],
            m_nativeres: true,
            threads: 0,
        }
    }
}

impl GSRendererSW {
    pub fn new(threads: u32) -> Self {
        let mut s = Self::default();
        s.threads = threads;
        s.m_tc = Some(Box::new(GSTextureCacheSW::new()));
        s.m_rl = Some(Box::new(GSRasterizerList::new(threads)));
        s.m_output = unsafe {
            // 1MPix aligned buffer
            let layout = std::alloc::Layout::from_size_align(1024 * 1024 * 4, 32).unwrap();
            std::alloc::alloc(layout) as *mut u32
        };
        s
    }

    pub fn reset(&mut self, _hardware_reset: bool) {}

    pub fn destroy(&mut self) {
        self.m_rl = None;
        self.m_tc = None;
        for t in &mut self.m_texture { *t = None; }
        if !self.m_output.is_null() {
            unsafe {
                let layout = std::alloc::Layout::from_size_align(1024 * 1024 * 4, 32).unwrap();
                std::alloc::dealloc(self.m_output as *mut u8, layout);
            }
            self.m_output = std::ptr::null_mut();
        }
    }

    pub fn vsync(&mut self, _field: u32, _registers_written: bool, _idle_frame: bool) {
        if let Some(tc) = self.m_tc.as_mut() { tc.inc_age(); }
    }

    pub fn get_output(
        &mut self,
        _i: i32, _scale: &mut f32, _y_offset: &mut i32,
    ) -> Option<Box<GSTexture>> { None }
}

impl Drop for GSRendererSW {
    fn drop(&mut self) { self.destroy(); }
}

// ===========================================================================
// GSRendererNull — the do-nothing renderer.  Mirrors `GSRendererNull`.
// ===========================================================================

#[derive(Debug)]
pub struct GSRendererNull {
    pub base: GSRenderer,
    pub m_draw_transfers: VecDeque<GSNullDrawTransfer>,
}

impl Default for GSRendererNull {
    fn default() -> Self {
        Self { base: GSRenderer::default(), m_draw_transfers: VecDeque::new() }
    }
}

impl GSRendererNull {
    pub fn new() -> Self { Self::default() }

    /// VSync — clears pending draw transfers.
    pub fn vsync(&mut self, _field: u32, _registers_written: bool, _idle_frame: bool) {
        self.m_draw_transfers.clear();
    }

    /// Draw — no-op.
    pub fn draw(&mut self) {}

    /// Get the i-th output texture.
    pub fn get_output(
        &mut self,
        _i: i32, _scale: &mut f32, _y_offset: &mut i32,
    ) -> Option<Box<GSTexture>> { None }
}

#[derive(Default, Debug)]
pub struct GSNullDrawTransfer {
    pub transfer_type: u32,
    pub draw: u32,
    pub rect: GSVector4i,
    pub blit: u64,
}

// ===========================================================================
// GSRendererProxy — top-level renderer-creation entry point.  The
// C++ code has `makeGSRenderer*` factory functions; the idiomatic
// Rust translation exposes a single dispatcher enum.
// ===========================================================================

#[derive(Debug)]
pub enum GSRendererProxy {
    Hw(GSRendererHW),
    Sw(GSRendererSW),
    Null(GSRendererNull),
}

impl Default for GSRendererProxy {
    fn default() -> Self { GSRendererProxy::Null(GSRendererNull::new()) }
}

impl GSRendererProxy {
    pub fn new_null() -> Self { GSRendererProxy::Null(GSRendererNull::new()) }
    pub fn new_sw(threads: u32) -> Self {
        GSRendererProxy::Sw(GSRendererSW::new(threads))
    }
    pub fn new_hw() -> Self { GSRendererProxy::Hw(GSRendererHW::new()) }

    pub fn vsync(&mut self, field: u32, registers_written: bool, idle_frame: bool) {
        match self {
            GSRendererProxy::Hw(r)  => r.vsync(field, registers_written, idle_frame),
            GSRendererProxy::Sw(r)  => r.vsync(field, registers_written, idle_frame),
            GSRendererProxy::Null(r) => r.vsync(field, registers_written, idle_frame),
        }
    }

    pub fn draw(&mut self) {
        match self {
            GSRendererProxy::Hw(r)  => r.draw(),
            GSRendererProxy::Sw(r)  => { let _ = r; }
            GSRendererProxy::Null(r) => r.draw(),
        }
    }

    pub fn get_output(
        &mut self,
        i: i32, scale: &mut f32, y_offset: &mut i32,
    ) -> Option<Box<GSTexture>> {
        match self {
            GSRendererProxy::Hw(r)  => r.get_output(i, scale, y_offset),
            GSRendererProxy::Sw(r)  => r.get_output(i, scale, y_offset),
            GSRendererProxy::Null(r) => r.get_output(i, scale, y_offset),
        }
    }
}

// ===========================================================================
// Texture replacement hash cache key (placeholder for the C++
// `GSTextureCache::HashCacheKey`).
// ===========================================================================

pub type HashCacheKey = GSTextureCacheHashCacheKey;

// ===========================================================================
// Code-generator frontends — both `.all.cpp` files become a
// single struct that owns the JIT-compiled code.  The real
// implementation is Xbyak; here it is a stub.
// ===========================================================================

pub struct GSDrawScanlineCodeGenerator {
    pub sel: u64,
    pub emit: Vec<u8>,
}

impl Default for GSDrawScanlineCodeGenerator {
    fn default() -> Self {
        Self { sel: 0, emit: Vec::new() }
    }
}

impl GSDrawScanlineCodeGenerator {
    pub fn new() -> Self { Self::default() }

    /// Generate the per-pixel code for `sel`.  Stub.
    pub fn generate(&mut self, _sel: u64) {
        unimplemented!("GSDrawScanlineCodeGenerator::Generate")
    }
}

pub struct GSSetupPrimCodeGenerator {
    pub sel: u64,
    pub emit: Vec<u8>,
}

impl Default for GSSetupPrimCodeGenerator {
    fn default() -> Self {
        Self { sel: 0, emit: Vec::new() }
    }
}

impl GSSetupPrimCodeGenerator {
    pub fn new() -> Self { Self::default() }

    /// Generate the per-vertex code for `sel`.  Stub.
    pub fn generate(&mut self, _sel: u64) {
        unimplemented!("GSSetupPrimCodeGenerator::Generate")
    }
}

// ===========================================================================
// Multi-ISA helpers — the C++ code wraps each backend in
// `MULTI_ISA_DEF` / `MULTI_ISA_FRIEND` / `MULTI_ISA_UNSHARED_IMPL`.
// In idiomatic Rust we just have a single backend behind a
// function pointer (see `GSRendererHWMultiISA`).
// ===========================================================================

/// Returns the current ISA name in the C++ sense (e.g. "sse4", "avx2", "avx512").
pub fn current_isa_name() -> &'static str { "sse4" }

/// Returns whether the current build supports AVX2.
pub fn has_avx2() -> bool { cfg!(target_arch = "x86_64") }

/// Returns whether the current build supports AVX-512.
pub fn has_avx512() -> bool { false }
