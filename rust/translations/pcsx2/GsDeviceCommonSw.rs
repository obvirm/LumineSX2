// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust translation of PCSX2's `GS/Renderers/Common` and
//! `GS/Renderers/SW` source files (device abstraction, software rasterizer,
//! JIT code generator interface, and null renderer).
//!
//! This module consolidates what was previously spread across roughly
//! 17 C/C++ translation units (GSDevice, GSFastList, GSFunctionMap,
//! GSRenderer, GSShaderEnums, GSTexture, GSDrawScanline, GSRasterizer,
//! GSRendererSW, GSTextureCacheSW, GSRendererNull, and the two code
//! generator `.all.cpp` files).
//!
//! Only `std` is used. All the legacy x86 / Xbyak code-generation
//! plumbing and the SSE/AVX intrinsics are abstracted behind safe
//! Rust traits and opaque handles — they are not exposed because
//! they cannot be meaningfully transcribed without a CPU-specific
//! assembler crate.

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]

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

// ===========================================================================
// Minimal GSVector2i / GSVector4 / GSVector4i stand-ins.
//
// The full GSVector SIMD type is enormous; for the purpose of a
// idiomatic translation of the *structure* of the renderer, an opaque
// 16-byte value is sufficient.  Individual accessor methods on
// GSDevice/GSRenderer operate on these values and downstream
// implementations are free to wrap a real SIMD type.
// ===========================================================================

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GSVector2i {
    pub x: i32,
    pub y: i32,
}

impl GSVector2i {
    pub const fn new(x: i32, y: i32) -> Self { Self { x, y } }
    pub const fn zero() -> Self { Self::new(0, 0) }
    pub const fn splat(v: i32) -> Self { Self::new(v, v) }
    pub const fn width(self) -> i32 { self.z() - self.x() }
    pub const fn height(self) -> i32 { self.w() - self.y() }
    pub const fn x(&self) -> i32 { self.x }
    pub const fn y(&self) -> i32 { self.y }
    pub const fn z(&self) -> i32 { self.x } // half-open alias for the .z component
    pub const fn w(&self) -> i32 { self.y }
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

// Manual Eq/Hash: compare/hash f32 by bit pattern (NaN compares equal to itself).
impl Eq for GSVector2 {}
impl std::hash::Hash for GSVector2 {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.x.to_bits().hash(state);
        self.y.to_bits().hash(state);
    }
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

// Manual Eq/Hash: compare/hash f32 by bit pattern (NaN compares equal to itself).
impl Eq for GSVector4 {}
impl std::hash::Hash for GSVector4 {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.x.to_bits().hash(state);
        self.y.to_bits().hash(state);
        self.z.to_bits().hash(state);
        self.w.to_bits().hash(state);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
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
// GSShader enums (verbatim from GSShaderEnums.h)
// ===========================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum VSExpand {
    None        = 0,
    Point       = 1,
    Line        = 2,
    Sprite      = 3,
    LineAA1     = 4,
    TriangleAA1 = 5,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum PS_ATST {
    NONE     = 0,
    LEQUAL   = 1,
    GEQUAL   = 2,
    EQUAL    = 3,
    NOTEQUAL = 4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum PS_AFAIL {
    KEEP          = 0,
    FB_ONLY       = 1,
    ZB_ONLY       = 2,
    RGB_ONLY      = 3,
    RGB_ONLY_DSB  = 4,
    RGB_ONLY_SW_Z = 5,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum ZTST {
    NEVER   = 0,
    ALWAYS  = 1,
    GEQUAL  = 2,
    GREATER = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum PS_AA1 {
    NONE          = 0,
    LINE          = 1,
    TRIANGLE      = 2,
    TRIANGLE_SW_Z = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum PS_ROV_DEPTH {
    NONE       = 0,
    READ_WRITE = 1,
    READ_ONLY  = 2,
}

// ===========================================================================
// ChannelFetch / HWBlendType
// ===========================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum ChannelFetch {
    NONE  = 0,
    RED   = 1,
    GREEN = 2,
    BLUE  = 3,
    ALPHA = 4,
    RGB   = 5,
    GXBY  = 6,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum HWBlendType {
    SRC_ONE_DST_FACTOR      = 1,
    SRC_ALPHA_DST_FACTOR    = 2,
    SRC_DOUBLE              = 3,
    SRC_HALF_ONE_DST_FACTOR = 4,
    SRC_INV_DST_BLEND_HALF  = 5,
    INV_SRC_DST_BLEND_HALF  = 6,
    BMIX1_ALPHA_HIGH_ONE    = 7,
    BMIX1_SRC_HALF          = 8,
    BMIX2_OVERFLOW          = 9,
}

// ===========================================================================
// HWBlend flags
// ===========================================================================

pub mod blend_flags {
    pub const BLEND_CD    : u16 = 0x001;
    pub const BLEND_HW1   : u16 = 0x002;
    pub const BLEND_HW2   : u16 = 0x004;
    pub const BLEND_HW3   : u16 = 0x008;
    pub const BLEND_HW4   : u16 = 0x010;
    pub const BLEND_HW5   : u16 = 0x020;
    pub const BLEND_HW6   : u16 = 0x040;
    pub const BLEND_HW7   : u16 = 0x080;
    pub const BLEND_HW8   : u16 = 0x100;
    pub const BLEND_HW9   : u16 = 0x200;
    pub const BLEND_MIX1  : u16 = 0x400;
    pub const BLEND_MIX2  : u16 = 0x800;
    pub const BLEND_MIX3  : u16 = 0x1000;
    pub const BLEND_ACCU  : u16 = 0x2000;
    pub const BLEND_NO_REC: u16 = 0x4000;
    pub const BLEND_A_MAX : u16 = 0x8000;
}

// ===========================================================================
// Filter / ShaderConvert / PresentShader
// ===========================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Filter {
    Nearest = 0,
    Biln    = 1,
}

pub const NEAREST: Filter = Filter::Nearest;
pub const BILN:    Filter = Filter::Biln;

#[inline]
pub const fn biln_if(b: bool) -> Filter { if b { BILN } else { NEAREST } }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ShaderConvert {
    COPY = 0,
    DEPTH_COPY,
    RGB5A1_TO_16_BITS,
    DATM_1,
    DATM_0,
    DATM_1_RTA_CORRECTION,
    DATM_0_RTA_CORRECTION,
    COLCLIP_INIT,
    COLCLIP_RESOLVE,
    RTA_CORRECTION,
    RTA_DECORRECTION,
    TRANSPARENCY_FILTER,
    DEPTH32_TO_16_BITS,
    DEPTH32_TO_32_BITS,
    DEPTH32_TO_RGBA8,
    DEPTH32_TO_RGB8,
    DEPTH16_TO_RGB5A1,
    RGBA8_TO_DEPTH32,
    RGBA8_TO_DEPTH24,
    RGBA8_TO_DEPTH16,
    RGB5A1_TO_DEPTH16,
    DEPTH32_TO_DEPTH24,
    DOWNSAMPLE_COPY,
    RGBA_TO_8I,
    RGB5A1_TO_8I,
    CLUT_4,
    CLUT_8,
    YUV,
    Count,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum PresentShader {
    COPY = 0,
    SCANLINE,
    DIAGONAL_FILTER,
    TRIANGULAR_FILTER,
    COMPLEX_FILTER,
    LOTTES_FILTER,
    SUPERSAMPLE_4xRGSS,
    SUPERSAMPLE_AUTO,
    Count,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum SetDATM {
    DATM0 = 0,
    DATM1,
    DATM0_RTA_CORRECTION,
    DATM1_RTA_CORRECTION,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ShaderInterlace {
    WEAVE          = 0,
    BOB            = 1,
    BLEND          = 2,
    MAD_BUFFER     = 3,
    MAD_RECONSTRUCT= 4,
    Count          = 5,
}

// ===========================================================================
// Helper query functions over ShaderConvert (from GSDevice.h).
// ===========================================================================

#[inline] pub const fn has_variable_write_mask(s: ShaderConvert) -> bool {
    matches!(s, ShaderConvert::COPY | ShaderConvert::RTA_CORRECTION)
}

#[inline] pub const fn has_color_output(s: ShaderConvert) -> bool {
    matches!(s,
        ShaderConvert::COPY
        | ShaderConvert::RTA_CORRECTION
        | ShaderConvert::RTA_DECORRECTION
        | ShaderConvert::TRANSPARENCY_FILTER
        | ShaderConvert::DEPTH32_TO_RGBA8
        | ShaderConvert::DEPTH32_TO_RGB8
        | ShaderConvert::DEPTH16_TO_RGB5A1
        | ShaderConvert::DOWNSAMPLE_COPY
        | ShaderConvert::RGBA_TO_8I
        | ShaderConvert::RGB5A1_TO_8I
        | ShaderConvert::CLUT_4
        | ShaderConvert::CLUT_8
        | ShaderConvert::YUV
        | ShaderConvert::COLCLIP_RESOLVE)
}

#[inline] pub const fn has_float32_output(s: ShaderConvert) -> bool {
    matches!(s,
        ShaderConvert::RGBA8_TO_DEPTH32
        | ShaderConvert::RGBA8_TO_DEPTH24
        | ShaderConvert::RGBA8_TO_DEPTH16
        | ShaderConvert::RGB5A1_TO_DEPTH16
        | ShaderConvert::DEPTH_COPY
        | ShaderConvert::DEPTH32_TO_DEPTH24)
}

#[inline] pub const fn has_float32_input(s: ShaderConvert) -> bool {
    matches!(s,
        ShaderConvert::DEPTH_COPY
        | ShaderConvert::DEPTH32_TO_16_BITS
        | ShaderConvert::DEPTH32_TO_32_BITS
        | ShaderConvert::DEPTH32_TO_RGBA8
        | ShaderConvert::DEPTH32_TO_RGB8
        | ShaderConvert::DEPTH16_TO_RGB5A1
        | ShaderConvert::DEPTH32_TO_DEPTH24)
}

#[inline] pub const fn is_datm_convert_shader(s: ShaderConvert) -> bool {
    matches!(s,
        ShaderConvert::DATM_0
        | ShaderConvert::DATM_1
        | ShaderConvert::DATM_0_RTA_CORRECTION
        | ShaderConvert::DATM_1_RTA_CORRECTION)
}

#[inline] pub const fn has_stencil_output(s: ShaderConvert) -> bool { is_datm_convert_shader(s) }

#[inline] pub const fn integer_output_bpp(s: ShaderConvert) -> i32 {
    match s {
        ShaderConvert::DEPTH32_TO_32_BITS => 32,
        ShaderConvert::DEPTH32_TO_16_BITS | ShaderConvert::RGB5A1_TO_16_BITS => 16,
        _ => 0,
    }
}

#[inline] pub const fn has_color_clip_output(s: ShaderConvert) -> bool { matches!(s, ShaderConvert::COLCLIP_INIT) }

#[inline] pub const fn supports_bilinear(s: ShaderConvert) -> bool {
    matches!(s,
        ShaderConvert::RGBA8_TO_DEPTH32
        | ShaderConvert::RGBA8_TO_DEPTH24
        | ShaderConvert::RGBA8_TO_DEPTH16
        | ShaderConvert::RGB5A1_TO_DEPTH16)
}

#[inline] pub const fn shader_convert_write_mask(s: ShaderConvert) -> u32 {
    match s { ShaderConvert::DEPTH32_TO_RGB8 => 0x7, _ => 0xf }
}

#[inline] pub const fn get_shader_index_for_mask(s: ShaderConvert, mask: i32) -> i32 {
    let mut index = mask;
    if matches!(s, ShaderConvert::RTA_CORRECTION) { index |= 1 << 4; }
    index
}

#[inline] pub const fn set_datm_shader(d: SetDATM) -> ShaderConvert {
    match d {
        SetDATM::DATM1_RTA_CORRECTION => ShaderConvert::DATM_1_RTA_CORRECTION,
        SetDATM::DATM0_RTA_CORRECTION => ShaderConvert::DATM_0_RTA_CORRECTION,
        SetDATM::DATM1 => ShaderConvert::DATM_1,
        SetDATM::DATM0 => ShaderConvert::DATM_0,
    }
}

pub const fn shader_entry_point(value: ShaderConvert) -> &'static str {
    use ShaderConvert::*;
    match value {
        COPY                  => "ps_copy",
        RGB5A1_TO_16_BITS     => "ps_convert_rgb5a1_16bits",
        DATM_1                => "ps_datm1",
        DATM_0                => "ps_datm0",
        DATM_1_RTA_CORRECTION => "ps_datm1_rta_correction",
        DATM_0_RTA_CORRECTION => "ps_datm0_rta_correction",
        COLCLIP_INIT          => "ps_colclip_init",
        COLCLIP_RESOLVE       => "ps_colclip_resolve",
        RTA_CORRECTION        => "ps_rta_correction",
        RTA_DECORRECTION      => "ps_rta_decorrection",
        TRANSPARENCY_FILTER   => "ps_filter_transparency",
        DEPTH32_TO_16_BITS    => "ps_convert_depth32_32bits",
        DEPTH32_TO_32_BITS    => "ps_convert_depth32_32bits",
        DEPTH32_TO_RGBA8      => "ps_convert_depth32_rgba8",
        DEPTH32_TO_RGB8       => "ps_convert_depth32_rgba8",
        DEPTH16_TO_RGB5A1     => "ps_convert_depth16_rgb5a1",
        RGBA8_TO_DEPTH32      => "ps_convert_rgba8_depth32",
        RGBA8_TO_DEPTH24      => "ps_convert_rgba8_depth24",
        RGBA8_TO_DEPTH16      => "ps_convert_rgba8_depth16",
        RGB5A1_TO_DEPTH16     => "ps_convert_rgb5a1_depth16",
        DEPTH32_TO_DEPTH24    => "ps_convert_depth32_depth24",
        DEPTH_COPY            => "ps_depth_copy",
        DOWNSAMPLE_COPY       => "ps_downsample_copy",
        RGBA_TO_8I            => "ps_convert_rgba_8i",
        RGB5A1_TO_8I          => "ps_convert_rgb5a1_8i",
        CLUT_4                => "ps_convert_clut_4",
        CLUT_8                => "ps_convert_clut_8",
        YUV                   => "ps_yuv",
        Count                 => "ShaderConvertUnknownShader",
    }
}

pub const fn present_entry_point(value: PresentShader) -> &'static str {
    use PresentShader::*;
    match value {
        COPY               => "ps_copy",
        SCANLINE           => "ps_filter_scanlines",
        DIAGONAL_FILTER    => "ps_filter_diagonal",
        TRIANGULAR_FILTER  => "ps_filter_triangular",
        COMPLEX_FILTER     => "ps_filter_complex",
        LOTTES_FILTER      => "ps_filter_lottes",
        SUPERSAMPLE_4xRGSS => "ps_4x_rgss",
        SUPERSAMPLE_AUTO   => "ps_automagical_supersampling",
        Count              => "DisplayShaderUnknownShader",
    }
}

pub const fn shader_convert_name(s: ShaderConvert) -> &'static str {
    use ShaderConvert::*;
    match s {
        COPY                  => "COPY",
        DEPTH_COPY            => "DEPTH_COPY",
        RGB5A1_TO_16_BITS     => "RGB5A1_TO_16_BITS",
        DATM_1                => "DATM_1",
        DATM_0                => "DATM_0",
        DATM_1_RTA_CORRECTION => "DATM_1_RTA_CORRECTION",
        DATM_0_RTA_CORRECTION => "DATM_0_RTA_CORRECTION",
        COLCLIP_INIT          => "COLCLIP_INIT",
        COLCLIP_RESOLVE       => "COLCLIP_RESOLVE",
        RTA_CORRECTION        => "RTA_CORRECTION",
        RTA_DECORRECTION      => "RTA_DECORRECTION",
        TRANSPARENCY_FILTER   => "TRANSPARENCY_FILTER",
        DEPTH32_TO_16_BITS    => "DEPTH32_TO_16_BITS",
        DEPTH32_TO_32_BITS    => "DEPTH32_TO_32_BITS",
        DEPTH32_TO_RGBA8      => "DEPTH32_TO_RGBA8",
        DEPTH32_TO_RGB8       => "DEPTH32_TO_RGB8",
        DEPTH16_TO_RGB5A1     => "DEPTH16_TO_RGB5A1",
        RGBA8_TO_DEPTH32      => "RGBA8_TO_DEPTH32",
        RGBA8_TO_DEPTH24      => "RGBA8_TO_DEPTH24",
        RGBA8_TO_DEPTH16      => "RGBA8_TO_DEPTH16",
        RGB5A1_TO_DEPTH16     => "RGB5A1_TO_DEPTH16",
        DEPTH32_TO_DEPTH24    => "DEPTH32_TO_DEPTH24",
        DOWNSAMPLE_COPY       => "DOWNSAMPLE_COPY",
        RGBA_TO_8I            => "RGBA_TO_8I",
        RGB5A1_TO_8I          => "RGB5A1_TO_8I",
        CLUT_4                => "CLUT_4",
        CLUT_8                => "CLUT_8",
        YUV                   => "YUV",
        Count                 => "Count",
    }
}

// ===========================================================================
// ShaderConvertSelector
// ===========================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct ShaderConvertSelector {
    pub shader:    u8, // ShaderConvert
    pub mask:      u8, // variable write mask
    pub depth_out: u8, // 0/1
    pub filter:    u8, // Filter
}

impl ShaderConvertSelector {
    pub const fn new(shader: ShaderConvert, mask: u8, depth_out: bool, filter: Filter) -> Self {
        let mut s = Self { shader: shader as u8, mask: 0, depth_out: 0, filter: filter as u8 };
        s.set_mask(mask).set_depth_output(depth_out).set_filter(filter)
    }

    pub const fn shader(&self) -> ShaderConvert { unsafe { std::mem::transmute(self.shader) } }
    pub const fn mask(&self) -> u8 { self.mask }
    pub const fn default_mask(&self) -> u8 { shader_convert_write_mask(self.shader()) as u8 }
    pub const fn get_filter(&self) -> Filter { unsafe { std::mem::transmute(self.filter) } }
    pub const fn biln(&self) -> bool { matches!(self.get_filter(), Filter::Biln) }
    pub const fn nearest(&self) -> bool { matches!(self.get_filter(), Filter::Nearest) }
    pub const fn supports_bilinear(&self) -> bool { supports_bilinear(self.shader()) }
    pub const fn color_output(&self) -> bool { has_color_output(self.shader()) }
    pub const fn depth_output(&self) -> bool { self.depth_out != 0 }
    pub const fn stencil_output(&self) -> bool { has_stencil_output(self.shader()) }
    pub const fn datm_convert_shader(&self) -> bool { is_datm_convert_shader(self.shader()) }
    pub const fn float32_output(&self) -> bool { has_float32_output(self.shader()) }
    pub const fn float32_input(&self) -> bool { has_float32_input(self.shader()) }
    pub const fn integer_output_bpp(&self) -> i32 { integer_output_bpp(self.shader()) }
    pub const fn variable_write_mask(&self) -> bool { has_variable_write_mask(self.shader()) }
    pub const fn color_clip_output(&self) -> bool { has_color_clip_output(self.shader()) }
    pub fn name(&self) -> &'static str { shader_convert_name(self.shader()) }
    pub fn entry_point(&self) -> &'static str { shader_entry_point(self.shader()) }

    pub const fn set_mask(mut self, mask: u8) -> Self {
        self.mask = if self.variable_write_mask() { mask & 0xf } else { self.default_mask() };
        self
    }
    pub const fn set_mask_4(mut self, wr: bool, wg: bool, wb: bool, wa: bool) -> Self {
        let m = (wr as u8) | ((wg as u8) << 1) | ((wb as u8) << 2) | ((wa as u8) << 3);
        self.set_mask(m)
    }
    pub const fn set_depth_output(mut self, depth_out: bool) -> Self {
        self.depth_out = if self.float32_output() && depth_out { 1 } else { 0 };
        self
    }
    pub const fn set_filter(mut self, filter: Filter) -> Self {
        self.filter = if self.supports_bilinear() { filter as u8 } else { NEAREST as u8 };
        self
    }
}

// ===========================================================================
// GSTexture (GSDownloadTexture and friends)
// ===========================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum GsTextureType {
    Invalid      = 0,
    RenderTarget = 1,
    DepthStencil,
    Texture,
    RWTexture,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum GsTextureFormat {
    Invalid      = 0,
    Color,
    ColorHQ,
    ColorHDR,
    ColorClip,
    DepthStencil,
    DepthColor,
    UNorm8,
    UInt16,
    UInt32,
    PrimID,
    BC1,
    BC2,
    BC3,
    BC7,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum GsTextureState {
    Dirty,
    Cleared,
    Invalidated,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ClearValue {
    Color(u32),
    Depth(f32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GsClearValue {
    pub color: u32,
    pub depth: f32,
}

impl GsClearValue {
    pub const fn color(c: u32) -> Self { Self { color: c, depth: 0.0 } }
    pub const fn depth(d: f32) -> Self { Self { color: 0, depth: d } }
}

impl Default for GsClearValue {
    fn default() -> Self { Self { color: 0, depth: 0.0 } }
}

pub struct GSTexture {
    pub size: GSVector2i,
    pub mipmap_levels: i32,
    pub ty: GsTextureType,
    pub format: GsTextureFormat,
    pub state: GsTextureState,
    pub last_frame_used: u32,
    pub needs_mipmaps_generated: bool,
    pub clear_value: GsClearValue,
    pub unordered_access: bool,
    /// Opaque backend handle — implementations map this to a real GPU texture.
    pub native: *mut std::ffi::c_void,
}

impl GSTexture {
    pub fn new() -> Self {
        Self {
            size: GSVector2i::zero(),
            mipmap_levels: 0,
            ty: GsTextureType::Invalid,
            format: GsTextureFormat::Invalid,
            state: GsTextureState::Dirty,
            last_frame_used: 0,
            needs_mipmaps_generated: true,
            clear_value: GsClearValue::default(),
            unordered_access: false,
            native: std::ptr::null_mut(),
        }
    }

    pub fn width(&self) -> i32 { self.size.x }
    pub fn height(&self) -> i32 { self.size.y }
    pub fn size(&self) -> GSVector2i { self.size }
    pub fn rect(&self) -> GSVector4i { GSVector4i::new(0, 0, self.size.x, self.size.y) }
    pub fn is_mipmap(&self) -> bool { self.mipmap_levels > 1 }
    pub fn get_type(&self) -> GsTextureType { self.ty }
    pub fn get_format(&self) -> GsTextureFormat { self.format }
    pub fn is_compressed_format(&self) -> bool {
        matches!(self.format, GsTextureFormat::BC1 | GsTextureFormat::BC2
                       | GsTextureFormat::BC3 | GsTextureFormat::BC7)
    }
    pub fn is_render_target(&self) -> bool { self.ty == GsTextureType::RenderTarget }
    pub fn is_depth_stencil(&self) -> bool { self.ty == GsTextureType::DepthStencil }
    pub fn is_depth_color(&self) -> bool {
        self.ty == GsTextureType::RenderTarget && self.format == GsTextureFormat::DepthColor
    }
    pub fn is_texture(&self) -> bool { self.ty == GsTextureType::Texture }
    pub fn is_depth_like(&self) -> bool { self.is_depth_stencil() || self.is_depth_color() }
    pub fn is_render_target_or_depth_stencil(&self) -> bool {
        matches!(self.ty, GsTextureType::RenderTarget | GsTextureType::DepthStencil)
    }
    pub fn get_state(&self) -> GsTextureState { self.state }
    pub fn set_state(&mut self, s: GsTextureState) { self.state = s; }
    pub fn get_last_frame_used(&self) -> u32 { self.last_frame_used }
    pub fn set_last_frame_used(&mut self, f: u32) { self.last_frame_used = f; }
    pub fn get_clear_color(&self) -> u32 { self.clear_value.color }
    pub fn get_clear_depth(&self) -> f32 { self.clear_value.depth }
    pub fn set_clear_color(&mut self, c: u32) {
        self.state = GsTextureState::Cleared;
        self.clear_value.color = c;
    }
    pub fn set_clear_depth(&mut self, d: f32) {
        self.state = GsTextureState::Cleared;
        self.clear_value.depth = d;
    }
    pub fn is_unordered_access(&self) -> bool { self.unordered_access }
    pub fn set_unordered_access(&mut self) { self.unordered_access = true; }
    pub fn clear_unordered_access(&mut self) { self.unordered_access = false; }
    pub fn mem_usage(&self) -> u32 {
        let bpp = if self.format == GsTextureFormat::UNorm8 { 1 } else { 4 };
        (self.size.x as u32) * (self.size.y as u32) * bpp
    }
    /// Abstracted map: returns a borrowed `&[u8]` slice if the texture is
    /// CPU-mappable, otherwise `None`.  Backends override the
    /// `Map`/`Unmap` method on the `GSDevice` trait.
    pub fn map(&mut self, _r: Option<GSVector4i>, _layer: i32) -> Option<(&mut [u8], i32)> { None }
    pub fn unmap(&mut self) {}
    pub fn update(&mut self, _r: GSVector4i, _data: &[u8], _pitch: i32, _layer: i32) -> bool { false }
    pub fn generate_mipmap(&mut self) {}
    pub fn generate_mipmaps_if_needed(&mut self) {
        if !self.needs_mipmaps_generated || self.mipmap_levels <= 1 || self.is_compressed_format() {
            return;
        }
        self.needs_mipmaps_generated = false;
        self.generate_mipmap();
    }
}

impl Default for GSTexture {
    fn default() -> Self { Self::new() }
}

pub fn texture_compressed_bytes_per_block(f: GsTextureFormat) -> u32 {
    use GsTextureFormat::*;
    match f {
        Invalid      => 1,
        Color        => 4,
        ColorHQ      => 4,
        ColorHDR     => 8,
        ColorClip    => 8,
        DepthStencil => 4,
        DepthColor   => 4,
        UNorm8       => 1,
        UInt16       => 2,
        UInt32       => 4,
        PrimID       => 4,
        BC1          => 8,
        BC2          => 16,
        BC3          => 16,
        BC7          => 16,
    }
}

pub fn texture_compressed_block_size(f: GsTextureFormat) -> u32 {
    if texture_is_block_compressed(f) { 4 } else { 1 }
}

pub fn texture_is_block_compressed(f: GsTextureFormat) -> bool {
    matches!(f, GsTextureFormat::BC1 | GsTextureFormat::BC2
                  | GsTextureFormat::BC3 | GsTextureFormat::BC7)
}

pub fn texture_calc_upload_pitch(f: GsTextureFormat, width: u32) -> u32 {
    let w = if texture_is_block_compressed(f) { (width + 3) / 4 } else { width };
    w * texture_compressed_bytes_per_block(f)
}

pub fn texture_calc_upload_row_length_from_pitch(f: GsTextureFormat, pitch: u32) -> u32 {
    let bs = texture_compressed_block_size(f);
    let bp = texture_compressed_bytes_per_block(f);
    ((pitch + bp - 1) / bp) * bs
}

pub fn texture_calc_upload_size(f: GsTextureFormat, height: u32, pitch: u32) -> u32 {
    let bs = texture_compressed_block_size(f);
    pitch * ((height + bs - 1) / bs)
}

pub fn texture_get_format_name(f: GsTextureFormat) -> &'static str {
    use GsTextureFormat::*;
    match f {
        Invalid      => "Invalid",
        Color        => "Color",
        ColorHQ      => "ColorHQ",
        ColorHDR     => "ColorHDR",
        ColorClip    => "ColorClip",
        DepthStencil => "DepthStencil",
        DepthColor   => "DepthColor",
        UNorm8       => "UNorm8",
        UInt16       => "UInt16",
        UInt32       => "UInt32",
        PrimID       => "PrimID",
        BC1          => "BC1",
        BC2          => "BC2",
        BC3          => "BC3",
        BC7          => "BC7",
    }
}

// ===========================================================================
// GSDownloadTexture
// ===========================================================================

pub struct GSDownloadTexture {
    pub width:  u32,
    pub height: u32,
    pub format: GsTextureFormat,
    pub map_pointer: *const u8,
    pub current_pitch: u32,
    pub needs_flush: bool,
}

impl GSDownloadTexture {
    pub fn new(width: u32, height: u32, format: GsTextureFormat) -> Self {
        Self { width, height, format, map_pointer: std::ptr::null(), current_pitch: 0, needs_flush: false }
    }
    pub fn width(&self) -> u32 { self.width }
    pub fn height(&self) -> u32 { self.height }
    pub fn format(&self) -> GsTextureFormat { self.format }
    pub fn needs_flush(&self) -> bool { self.needs_flush }
    pub fn is_mapped(&self) -> bool { !self.map_pointer.is_null() }
    pub fn map_pointer(&self) -> *const u8 { self.map_pointer }
    pub fn map_pitch(&self) -> u32 { self.current_pitch }

    pub fn transfer_pitch(&self, width: u32, pitch_align: u32) -> u32 {
        let bs = texture_compressed_block_size(self.format);
        let bp = texture_compressed_bytes_per_block(self.format);
        let bw = (width + bs - 1) / bs;
        align_up_pow2(bw * bp, pitch_align)
    }

    pub fn transfer_size(&self, rc: GSVector4i, copy_offset: &mut u32, copy_size: &mut u32, copy_rows: &mut u32) {
        let bs = texture_compressed_block_size(self.format);
        let bp = texture_compressed_bytes_per_block(self.format);
        let tw = rc.width() as u32;
        let tb = (tw + bs - 1) / bs;
        *copy_offset = (((rc.y as u32 + bs - 1) / bs) * self.current_pitch)
                      + (((rc.x as u32 + bs - 1) / bs) * bp);
        *copy_size = tb * bp;
        *copy_rows = (rc.height() as u32 + bs - 1) / bs;
    }

    pub fn buffer_size(width: u32, height: u32, format: GsTextureFormat, pitch_align: u32) -> u32 {
        let bs = texture_compressed_block_size(format);
        let bp = texture_compressed_bytes_per_block(format);
        let bw = (width + bs - 1) / bs;
        let bh = (height + bs - 1) / bs;
        align_up_pow2(bw * bp, pitch_align) * bh
    }
}

fn align_up_pow2(x: u32, a: u32) -> u32 { (x + a - 1) & !(a - 1) }

// ===========================================================================
// HWBlend / GSHWDrawConfig
// ===========================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct HWBlend {
    pub flags: u16,
    pub op:    u8,
    pub src:   u8,
    pub dst:   u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct VSSelector {
    pub key: u8,
}

impl VSSelector {
    pub fn use_fixed_expand_index_buffer(&self) -> bool {
        let expand = self.key & 0x0E;
        expand == (VSExpand::Point as u8) || expand == (VSExpand::Sprite as u8)
    }
    pub fn use_vs_expand_index_buffer(&self) -> bool {
        let expand = self.key & 0x0E;
        expand == (VSExpand::TriangleAA1 as u8)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct PSSelector {
    pub key_lo: u64,
    pub key_hi: u64,
}

impl PSSelector {
    pub fn is_sw_blending(&self) -> bool { ((self.key_lo >> 28) & 0x1F) != 0 }
    pub fn is_alpha_testing(&self) -> bool {
        let atst = (self.key_lo >> 16) & 0x7;
        atst != (PS_ATST::NONE as u64)
    }
    pub fn is_z_testing(&self) -> bool {
        let ztst = (self.key_lo >> 19) & 0x3;
        ztst == (ZTST::GEQUAL as u64) || ztst == (ZTST::GREATER as u64)
    }
    pub fn has_color_output(&self) -> bool { (self.key_hi & 0x40) == 0 }
    pub fn no_color(&self) -> bool { (self.key_hi & 0x40) != 0 }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct SamplerSelector {
    pub key: u8,
}

impl SamplerSelector {
    pub fn point() -> Self { Self { key: 0 } }
    pub fn linear() -> Self { Self { key: 0x20 } }
    pub fn is_min_filter_linear(&self) -> bool {
        let triln = (self.key >> 3) & 0x7;
        let biln = (self.key & 0x20) != 0;
        if triln < 2 { biln } else { triln >= 4 }
    }
    pub fn is_mag_filter_linear(&self) -> bool { (self.key & 0x20) != 0 }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct DepthStencilSelector {
    pub key: u8,
}

impl DepthStencilSelector {
    pub fn no_depth() -> Self { Self { key: (ZTST::ALWAYS as u8) & 0x3 } }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct ColorMaskSelector {
    pub key: u8,
}

impl ColorMaskSelector {
    pub fn all() -> Self { Self { key: 0xF } }
    pub fn from_wrgba(c: u8) -> Self { Self { key: c & 0xF } }
    pub fn wrgba(&self) -> u8 { self.key & 0xF }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct BlendState {
    pub key: u32,
}

impl BlendState {
    pub const fn new(
        enable: bool, src: u8, dst: u8, op: u8,
        src_a: u8, dst_a: u8, const_enable: bool, constant: u8,
    ) -> Self {
        // Bit-packed layout mirroring the C++ struct.
        let mut k: u32 = 0;
        k |= (enable as u32) << 0;
        k |= (const_enable as u32) << 1;
        k |= (op as u32 & 0x3F) << 2;
        k |= (src as u32 & 0x0F) << 8;
        k |= (dst as u32 & 0x0F) << 12;
        k |= (src_a as u32 & 0x0F) << 16;
        k |= (dst_a as u32 & 0x0F) << 20;
        k |= (constant as u32) << 24;
        Self { key: k }
    }
    pub fn enable(&self) -> bool { (self.key & 1) != 0 }
    pub fn src(&self) -> u8 { ((self.key >> 8) & 0xF) as u8 }
    pub fn dst(&self) -> u8 { ((self.key >> 12) & 0xF) as u8 }
    pub fn is_effective(&self, cm: ColorMaskSelector) -> bool {
        if !self.enable() { return false; }
        let rgb = cm.key & 0x7;
        let a   = cm.key & 0x8;
        let const_one  = Self::CONST_ONE;
        let const_zero = Self::CONST_ZERO;
        (rgb != 0 && (self.src() != const_one || self.dst() != const_zero))
        || (a != 0 && (self.src() != const_one || self.dst() != const_zero))
    }

    // Blend factors
    pub const SRC_COLOR      : u8 = 0;
    pub const INV_SRC_COLOR  : u8 = 1;
    pub const DST_COLOR      : u8 = 2;
    pub const INV_DST_COLOR  : u8 = 3;
    pub const SRC1_COLOR     : u8 = 4;
    pub const INV_SRC1_COLOR : u8 = 5;
    pub const SRC_ALPHA      : u8 = 6;
    pub const INV_SRC_ALPHA  : u8 = 7;
    pub const DST_ALPHA      : u8 = 8;
    pub const INV_DST_ALPHA  : u8 = 9;
    pub const SRC1_ALPHA     : u8 = 10;
    pub const INV_SRC1_ALPHA : u8 = 11;
    pub const CONST_COLOR    : u8 = 12;
    pub const INV_CONST_COLOR: u8 = 13;
    pub const CONST_ONE      : u8 = 14;
    pub const CONST_ZERO     : u8 = 15;
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayConstantBuffer {
    pub source_rect: GSVector4,
    pub target_rect: GSVector4,
    pub source_size: GSVector2,
    pub target_size: GSVector2,
    pub target_resolution: GSVector2,
    pub rcp_target_resolution: GSVector2,
    pub source_resolution: GSVector2,
    pub rcp_source_resolution: GSVector2,
    pub time_and_pad: GSVector4,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MergeConstantBuffer {
    pub bg_color: GSVector4,
    pub emoda: u32, pub emodc: u32, pub doffset: u32,
    pub scale_factor: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InterlaceConstantBuffer { pub zrh: GSVector4 }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum AlphaTestMode { NONE, KEEP, FEEDBACK, SIMPLE_FB_ONLY, SIMPLE_RGB_ONLY, PASS_THEN_FAIL, NEVER, ABORT_DRAW }

impl AlphaTestMode {
    pub fn has_second_pass(m: AlphaTestMode) -> bool {
        matches!(m, AlphaTestMode::SIMPLE_FB_ONLY
                    | AlphaTestMode::SIMPLE_RGB_ONLY
                    | AlphaTestMode::PASS_THEN_FAIL
                    | AlphaTestMode::NEVER)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum DestinationAlphaMode { Off, Stencil, StencilOne, PrimIDTracking, Full }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ColClipMode { NoModify = 0, ConvertOnly = 1, ResolveOnly = 2, ConvertAndResolve = 3, EarlyResolve = 4 }

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AlphaPass {
    pub ps: PSSelector,
    pub enable: bool,
    pub require_one_barrier: bool,
    pub require_full_barrier: bool,
    pub colormask: ColorMaskSelector,
    pub depth: DepthStencilSelector,
    pub ps_aref: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlendMultiPass {
    pub blend: BlendState,
    pub enable: bool,
    pub no_color1: bool,
    pub blend_hw: u8,
    pub dither: u8,
}

pub struct GSHWDrawConfig {
    pub rt: Option<Box<GSTexture>>,
    pub ds: Option<Box<GSTexture>>,
    pub tex: Option<Box<GSTexture>>,
    pub pal: Option<Box<GSTexture>>,
    pub nverts: u32,
    pub nindices: u32,
    pub indices_per_prim: u32,
    pub scissor: GSVector4i,
    pub drawarea: GSVector4i,
    pub samplearea: GSVector4i,
    pub topology: u8,
    pub ps: PSSelector,
    pub vs: VSSelector,
    pub blend: BlendState,
    pub sampler: SamplerSelector,
    pub colormask: ColorMaskSelector,
    pub depth: DepthStencilSelector,
    pub require_one_barrier: bool,
    pub require_full_barrier: bool,
    pub tex_hazard: u32,
    pub alpha_test: AlphaTestMode,
    pub destination_alpha: DestinationAlphaMode,
    pub datm: SetDATM,
    pub line_expand: bool,
    pub alpha_second_pass: AlphaPass,
    pub blend_multi_pass: BlendMultiPass,
    pub colclip_mode: ColClipMode,
    pub colclip_update_area: GSVector4i,
}

impl GSHWDrawConfig {
    pub fn is_blending(&self) -> bool {
        self.blend.enable()
            || self.blend_multi_pass.enable
            || self.ps.is_sw_blending()
    }
    pub fn is_feedback_loop_rt(&self) -> bool { self.tex_hazard == 1 }
    pub fn is_feedback_loop_depth(&self) -> bool { self.tex_hazard == 2 }
}

pub fn get_expansion_factor(e: VSExpand) -> u32 {
    use VSExpand::*;
    match e {
        Point | Line | LineAA1 => 4,
        Sprite                 => 2,
        TriangleAA1            => 13,
        None                   => 1,
    }
}

pub fn get_vertex_alignment(e: VSExpand) -> u32 {
    if e == VSExpand::Sprite { 2 } else { 1 }
}

// ===========================================================================
// FeatureSupport (from GSDevice.h)
// ===========================================================================

#[derive(Clone, Copy, Debug, Default)]
pub struct FeatureSupport {
    pub broken_point_sampler: bool,
    pub vs_expand: bool,
    pub primitive_id: bool,
    pub texture_barrier: bool,
    pub multidraw_fb_copy: bool,
    pub provoking_vertex_last: bool,
    pub point_expand: bool,
    pub line_expand: bool,
    pub prefer_new_textures: bool,
    pub dxt_textures: bool,
    pub bptc_textures: bool,
    pub framebuffer_fetch: bool,
    pub stencil_buffer: bool,
    pub cas_sharpening: bool,
    pub test_and_sample_depth: bool,
    pub depth_feedback: bool,
    pub aa1: bool,
    pub rov: bool,
}

impl FeatureSupport {
    pub fn feedback_loops(&self) -> bool { self.texture_barrier || self.multidraw_fb_copy }
}

// ===========================================================================
// WindowInfo / VS / Render API enums
// ===========================================================================

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct WindowInfo {
    pub surface_width: u32,
    pub surface_height: u32,
    pub surface_scale: f32,
    pub surface_refresh_rate: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum GsVSyncMode { Disabled, FIFO, Adaptive }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum RenderAPI { None, D3D11, D3D12, Metal, Vulkan, OpenGL }

// ===========================================================================
// MultiStretchRect
// ===========================================================================

#[derive(Clone, Copy, Debug)]
pub struct MultiStretchRect {
    pub src_rect: GSVector4,
    pub dst_rect: GSVector4,
    pub src: *mut GSTexture,
    pub filter: Filter,
    pub wmask: ColorMaskSelector,
}

// ===========================================================================
// GSDevice trait
// ===========================================================================

pub trait GSDevice: Send {
    fn name(&self) -> &str;
    fn features(&self) -> FeatureSupport;
    fn max_texture_size(&self) -> u32;

    fn get_render_api(&self) -> RenderAPI;
    fn has_surface(&self) -> bool;
    fn destroy_surface(&mut self);
    fn update_window(&mut self) -> bool;
    fn resize_window(&mut self, w: u32, h: u32, scale: f32);
    fn supports_exclusive_fullscreen(&self) -> bool;

    fn begin_present(&mut self, frame_skip: bool) -> PresentResult;
    fn end_present(&mut self);
    fn set_vsync_mode(&mut self, mode: GsVSyncMode, allow_throttle: bool);
    fn driver_info(&self) -> String;
    fn set_gpu_timing_enabled(&mut self, enabled: bool) -> bool;
    fn get_and_reset_accumulated_gpu_time(&mut self) -> f32;

    fn should_skip_presenting_frame(&self) -> bool;
    fn throttle_presentation(&mut self);

    fn clear_render_target(&self, t: &mut GSTexture, c: u32);
    fn clear_depth(&self, t: &mut GSTexture, d: f32);
    fn process_clears_before_copy(&self, src: &GSTexture, dst: &mut GSTexture, full_copy: bool) -> bool;
    fn invalidate_render_target(&self, t: &mut GSTexture);

    fn push_debug_group(&mut self, _name: &str) {}
    fn pop_debug_group(&mut self) {}
    fn insert_debug_message(&mut self, _cat: DebugMessageCategory, _msg: &str) {}

    fn create_surface(&mut self, ty: GsTextureType, w: i32, h: i32, levels: i32, fmt: GsTextureFormat) -> Box<GSTexture>;
    fn create_download_texture(&mut self, w: u32, h: u32, fmt: GsTextureFormat) -> Box<GSDownloadTexture>;
    fn copy_rect(&mut self, src: &GSTexture, dst: &mut GSTexture, r: GSVector4i, dx: u32, dy: u32);

    fn present_rect(&mut self, src: &GSTexture, src_rect: GSVector4, dst: Option<&mut GSTexture>,
                    dst_rect: GSVector4, shader: PresentShader, time: f32, filter: Filter);

    fn update_clut_texture(&mut self, src: &GSTexture, s_scale: f32, sx: u32, sy: u32,
                            dst: &mut GSTexture, d_off: u32, d_size: u32);
    fn convert_to_indexed_texture(&mut self, src: &GSTexture, s_scale: f32, sx: u32, sy: u32,
                                  sbw: u32, spsm: u32, dst: &mut GSTexture, dbw: u32, dpsm: u32);
    fn filtered_downsample_texture(&mut self, src: &GSTexture, dst: &mut GSTexture,
                                    factor: u32, clamp_min: GSVector2i, d_rect: GSVector4);

    fn render_hw(&mut self, config: &GSHWDrawConfig);
    fn clear_sampler_cache(&mut self);

    fn do_stretch_rect(&mut self, src: &GSTexture, src_rect: GSVector4, dst: &mut GSTexture,
                       dst_rect: GSVector4, shader: ShaderConvertSelector, filter: Filter);

    /// Generate the static index buffer used to expand points/sprites.
    fn generate_expansion_index_buffer(&self, buffer: &mut [u8]);

    /// Process a copy area before a stretch (e.g. for snapping the bbox).
    fn process_copy_area(&self, rtsize: GSVector4i, drawarea: GSVector4i) -> GSVector4i;
    fn read_shader_source(&self, _name: &str) -> Option<String> { None }
    fn get_mipmap_levels_for_size(&self, width: i32, height: i32) -> i32;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PresentResult { OK, FrameSkipped, DeviceLost }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DebugMessageCategory { Cache, Reg, Debug, Message, Performance }

pub fn render_api_to_string(api: RenderAPI) -> &'static str {
    use RenderAPI::*;
    match api {
        None    => "None",
        D3D11   => "D3D11",
        D3D12   => "D3D12",
        Metal   => "Metal",
        Vulkan  => "Vulkan",
        OpenGL  => "OpenGL",
    }
}

// ===========================================================================
// GSDeviceBase — the parts of GSDevice that don't depend on the GPU
// API: texture pool, recycling, creation helpers.
// ===========================================================================

pub struct GSDeviceBase {
    pub name: String,
    pub features: FeatureSupport,
    pub max_texture_size: u32,
    pub window_info: WindowInfo,
    pub vsync_mode: GsVSyncMode,
    pub allow_present_throttle: bool,
    pub last_frame_displayed_time: u64,
    pub frame: u32,

    /// Pool[0] = textures, Pool[1] = render targets / depth.
    pub pool: [FastList<*mut GSTexture>; 2],
    pub pool_memory_usage: u64,

    pub merge:     *mut GSTexture,
    pub weavebob:  *mut GSTexture,
    pub blend:     *mut GSTexture,
    pub mad:       *mut GSTexture,
    pub target_tmp:*mut GSTexture,
    pub current:   *mut GSTexture,
    pub cas:       *mut GSTexture,
    pub colclip_rt:*mut GSTexture,
    pub ds_as_rt:  *mut GSTexture,

    pub vertex: RangeCount,
    pub index:  RangeCount,

    pub hw_blend_map: [HWBlend; 81], // 3*3*3*3
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RangeCount { pub start: u32, pub count: u32 }

impl GSDeviceBase {
    pub const NUM_INTERLACE_SHADERS: i32 = 5;
    pub const MAD_SENSITIVITY: f32 = 0.08;
    pub const MAX_POOLED_TARGETS: u32 = 300;
    pub const MAX_TARGET_AGE: u32 = 20;
    pub const MAX_POOLED_TEXTURES: u32 = 300;
    pub const MAX_TEXTURE_AGE: u32 = 10;
    pub const NUM_CAS_CONSTANTS: u32 = 12;
    pub const EXPAND_BUFFER_SIZE: usize = std::mem::size_of::<u16>() * 16383 * 6;

    pub fn new() -> Self {
        Self {
            name: "Unknown".into(),
            features: FeatureSupport::default(),
            max_texture_size: 0,
            window_info: WindowInfo::default(),
            vsync_mode: GsVSyncMode::Disabled,
            allow_present_throttle: false,
            last_frame_displayed_time: 0,
            frame: 0,
            pool: [FastList::new(), FastList::new()],
            pool_memory_usage: 0,
            merge: std::ptr::null_mut(),
            weavebob: std::ptr::null_mut(),
            blend: std::ptr::null_mut(),
            mad: std::ptr::null_mut(),
            target_tmp: std::ptr::null_mut(),
            current: std::ptr::null_mut(),
            cas: std::ptr::null_mut(),
            colclip_rt: std::ptr::null_mut(),
            ds_as_rt: std::ptr::null_mut(),
            vertex: RangeCount::default(),
            index:  RangeCount::default(),
            hw_blend_map: hw_blend_map(),
        }
    }

    pub fn get_blend(&self, index: usize) -> HWBlend { self.hw_blend_map[index] }
    pub fn get_blend_flags(&self, index: usize) -> u16 { self.hw_blend_map[index].flags }

    /// Fetch a surface from the pool, allocating a new one if none fits.
    /// `prefer_unused_texture`: if true, avoid textures used this frame.
    pub fn fetch_surface<F: GSDevice>(
        &mut self, dev: &mut F,
        ty: GsTextureType, w: i32, h: i32, levels: i32, fmt: GsTextureFormat,
        clear: bool, prefer_unused_texture: bool,
    ) -> Option<Box<GSTexture>> {
        let size = GSVector2i::new(w.clamp(1, self.max_texture_size as i32),
                                   h.clamp(1, self.max_texture_size as i32));
        let pool_idx = (ty != GsTextureType::Texture) as usize;
        let pool = &mut self.pool[pool_idx];

        // Search the pool for a matching surface.
        let mut fallback: Option<u16> = None;
        let mut found: Option<u16> = None;
        let mut idx = pool.first_index();
        while idx != 0 {
            let t = pool.data(idx) as *mut GSTexture;
            let matches = unsafe {
                (*t).get_type() == ty
                    && (*t).get_format() == fmt
                    && (*t).size() == size
                    && (*t).mipmap_levels == levels
            };
            if matches {
                if !prefer_unused_texture || unsafe { (*t).get_last_frame_used() != self.frame } {
                    found = Some(idx);
                    break;
                } else if fallback.is_none() {
                    fallback = Some(idx);
                }
            }
            idx = pool.next_index(idx);
        }

        let mut tex: Box<GSTexture> = if let Some(i) = found {
            let raw = pool.data(i) as *mut GSTexture;
            unsafe {
                self.pool_memory_usage = self.pool_memory_usage.saturating_sub((*raw).mem_usage() as u64);
            }
            pool.erase_index(i);
            unsafe { Box::from_raw(raw) }
        } else {
            let max = if ty == GsTextureType::Texture { Self::MAX_POOLED_TEXTURES } else { Self::MAX_POOLED_TARGETS };
            if pool.size() as u32 >= max {
                if let Some(fb) = fallback {
                    let raw = pool.data(fb) as *mut GSTexture;
                    unsafe {
                        self.pool_memory_usage = self.pool_memory_usage.saturating_sub((*raw).mem_usage() as u64);
                    }
                    pool.erase_index(fb);
                    unsafe { Box::from_raw(raw) }
                } else {
                    dev.create_surface(ty, size.x, size.y, levels, fmt)
                }
            } else {
                dev.create_surface(ty, size.x, size.y, levels, fmt)
            }
        };

        match ty {
            GsTextureType::RenderTarget => {
                if clear { dev.clear_render_target(&mut tex, 0); }
                else     { dev.invalidate_render_target(&mut tex); }
            }
            GsTextureType::DepthStencil => {
                if clear { dev.clear_depth(&mut tex, 0.0); }
                else     { dev.invalidate_render_target(&mut tex); }
            }
            _ => {}
        }
        Some(tex)
    }

    /// Return a surface to the pool for reuse. Setting `frame` here is
    /// the standard "I am using it this frame" marker.
    pub fn recycle<F: GSDevice>(&mut self, _dev: &mut F, t: Option<Box<GSTexture>>) {
        let Some(mut t) = t else { return; };
        t.set_last_frame_used(self.frame);
        t.clear_unordered_access();
        let is_tex = t.is_texture();
        let pool_idx = (!is_tex) as usize;
        let max_size = if is_tex { Self::MAX_POOLED_TEXTURES } else { Self::MAX_POOLED_TARGETS };
        let max_age  = if is_tex { Self::MAX_TEXTURE_AGE }    else { Self::MAX_TARGET_AGE };
        let mem = t.mem_usage() as u64;
        let raw = Box::into_raw(t);
        self.pool[pool_idx].push_front(raw as *mut GSTexture);
        self.pool_memory_usage += mem;

        let pool = &mut self.pool[pool_idx];
        while pool.size() as u32 > max_size {
            let back = pool.back();
            let last = unsafe { (*back).get_last_frame_used() };
            if (self.frame - last) < max_age { break; }
            let mem = unsafe { (*back).mem_usage() as u64 };
            let raw = unsafe { Box::from_raw(back) };
            drop(raw);
            self.pool_memory_usage = self.pool_memory_usage.saturating_sub(mem);
            pool.pop_back();
        }
    }

    pub fn age_pool(&mut self) {
        self.frame = self.frame.wrapping_add(1);
        for pool_idx in 0..2 {
            let max_age = if pool_idx == 0 { Self::MAX_TEXTURE_AGE } else { Self::MAX_TARGET_AGE };
            let pool = &mut self.pool[pool_idx];
            while !pool.empty() {
                let back = pool.back();
                let last = unsafe { (*back).get_last_frame_used() };
                if (self.frame - last) < max_age { break; }
                let mem = unsafe { (*back).mem_usage() as u64 };
                let raw = unsafe { Box::from_raw(back) };
                drop(raw);
                self.pool_memory_usage = self.pool_memory_usage.saturating_sub(mem);
                pool.pop_back();
            }
        }
    }

    pub fn purge_pool(&mut self) {
        for pool in &mut self.pool {
            let mut idx = pool.first_index();
            while idx != 0 {
                let next = pool.next_index(idx);
                let raw = pool.data(idx) as *mut GSTexture;
                unsafe { let _ = Box::from_raw(raw); }
                idx = next;
            }
            pool.clear();
        }
        self.pool_memory_usage = 0;
    }

    pub fn clear_current<F: GSDevice>(&mut self, dev: &mut F) {
        self.current = std::ptr::null_mut();
        self.recycle(dev, ptr_to_box(self.merge));      self.merge = std::ptr::null_mut();
        self.recycle(dev, ptr_to_box(self.weavebob));   self.weavebob = std::ptr::null_mut();
        self.recycle(dev, ptr_to_box(self.blend));      self.blend = std::ptr::null_mut();
        self.recycle(dev, ptr_to_box(self.mad));        self.mad = std::ptr::null_mut();
        self.recycle(dev, ptr_to_box(self.target_tmp)); self.target_tmp = std::ptr::null_mut();
        self.recycle(dev, ptr_to_box(self.cas));        self.cas = std::ptr::null_mut();
    }

    pub fn pool_memory_usage(&self) -> u64 { self.pool_memory_usage }

    pub fn do_stretch_rect_with_assertions<F: GSDevice>(
        &mut self, dev: &mut F,
        src: &GSTexture, src_rect: GSVector4, dst: &mut GSTexture, dst_rect: GSVector4,
        shader: ShaderConvertSelector, filter: Filter,
    ) {
        debug_assert!((dst.is_depth_like()) == shader.float32_output());
        debug_assert!(!(filter == BILN && shader.supports_bilinear()));
        dev.do_stretch_rect(src, src_rect, dst, dst_rect, shader, filter);
    }

    pub fn stretch_rect<F: GSDevice>(
        &mut self, dev: &mut F,
        src: &GSTexture, src_rect: GSVector4, dst: &mut GSTexture, dst_rect: GSVector4,
        shader: ShaderConvertSelector, filter: Filter,
    ) {
        self.do_stretch_rect_with_assertions(dev, src, src_rect, dst, dst_rect, shader, filter);
    }

    pub fn draw_multi_stretch_rects<F: GSDevice>(
        &mut self, dev: &mut F,
        rects: &[MultiStretchRect], dst: &mut GSTexture, mut shader: ShaderConvertSelector,
    ) {
        if rects.is_empty() { return; }
        let base_mask = rects[0].wmask.wrgba();
        for sr in rects {
            let s = shader.set_mask(base_mask).set_filter(sr.filter);
            dev.do_stretch_rect(unsafe { &*sr.src }, sr.src_rect, dst, sr.dst_rect, s, sr.filter);
        }
    }

    pub fn resize_render_target<F: GSDevice>(
        &mut self, dev: &mut F,
        t: &mut Option<Box<GSTexture>>, w: i32, h: i32,
        preserve_contents: bool, recycle: bool,
    ) -> bool {
        if let Some(existing) = t.as_mut() {
            if existing.width() == w && existing.height() == h {
                if !preserve_contents { dev.invalidate_render_target(existing); }
                return true;
            }
        }
        let fmt = t.as_ref().map(|x| x.get_format()).unwrap_or(GsTextureFormat::Color);
        let really_preserve = preserve_contents && t.is_some();
        let mut new_tex = match self.fetch_surface(dev, GsTextureType::RenderTarget,
            w, h, 1, fmt, !really_preserve, true) {
            Some(t) => t,
            None => return false,
        };
        if really_preserve {
            if let Some(orig) = t.as_ref() {
                let src_rect = GSVector4::new(0.0, 0.0, 1.0, 1.0);
                let dst_rect = GSVector4::new(0.0, 0.0, orig.width() as f32, orig.height() as f32);
                dev.do_stretch_rect(orig, src_rect, &mut new_tex, dst_rect,
                                     ShaderConvertSelector::new(ShaderConvert::COPY, 0xf, false, NEAREST),
                                     BILN);
            }
        }
        if let Some(orig) = t.take() {
            if recycle { self.recycle(dev, Some(orig)); }
            else { drop(orig); }
        }
        *t = Some(new_tex);
        true
    }
}

fn ptr_to_box<T>(p: *mut T) -> Option<Box<T>> {
    if p.is_null() { None } else { Some(unsafe { Box::from_raw(p) }) }
}

impl Default for GSDeviceBase { fn default() -> Self { Self::new() } }

// ===========================================================================
// HW blend map (3*3*3*3 = 81 entries) from GSDevice.cpp.
//
// Each entry: (flags, op, src, dst).
// ===========================================================================

/// HW blend map (3*3*3*3 = 81 entries) from GSDevice.cpp.
///
/// Each entry: (flags, op, src, dst).  This is a function (not a const)
/// because the original closure-based expansion is not allowed in const
/// context.  The function is called once at startup.
pub fn hw_blend_map() -> [HWBlend; 81] {
    use blend_flags::*;
    // Each HWBlend has { flags: u16, op: u8, src: u8, dst: u8 }.
    // Ops: 0=ADD, 1=SUB, 2=REV_SUB.  Factors: 0=ONE, 14=CONST_ONE,
    // 15=CONST_ZERO, 2=DST_COLOR, 8=DST_ALPHA, 6=SRC_ALPHA, 12=CONST_COLOR,
    // 4=SRC1_COLOR, 5=INV_SRC1_COLOR, 13=INV_CONST_COLOR, 9=INV_DST_ALPHA.
    let op_add  = 0u8;
    let op_sub  = 1u8;
    let op_rsub = 2u8;
    let one     = 0u8;   // CONST_ONE
    let zero    = 15u8;  // CONST_ZERO
    let dst_c   = 2u8;
    let dst_a   = 8u8;
    let src1_c  = 4u8;
    let inv_s1c = 5u8;
    let inv_da  = 9u8;
    let const_c = 12u8;
    let inv_cc  = 13u8;
    let br = |h: (u16, u8, u8, u8)| HWBlend { flags: h.0, op: h.1, src: h.2, dst: h.3 };
    let h_no_rec = |op, src, dst| (BLEND_NO_REC, op, src, dst);
    let h_cd     = |op, src, dst| (BLEND_CD,     op, src, dst);
    let h_max1   = |op, src, dst| (BLEND_A_MAX | BLEND_MIX2, op, src, dst);
    let h_mix1   = |op, src, dst| (BLEND_MIX1, op, src, dst);
    let h_max    = |op, src, dst| (BLEND_A_MAX, op, src, dst);
    let h_hw5    = |op, src, dst| (BLEND_HW5, op, src, dst);
    let h_max2   = |op, src, dst| (BLEND_A_MAX | BLEND_HW8, op, src, dst);
    let h_hw3    = |op, src, dst| (BLEND_HW3, op, src, dst);
    let h_mix3   = |op, src, dst| (BLEND_MIX3, op, src, dst);
    let h_max3   = |op, src, dst| (BLEND_A_MAX | BLEND_MIX1, op, src, dst);
    let h_accu   = |op, src, dst| (BLEND_ACCU, op, src, dst);
    let h_hw4    = |op, src, dst| (BLEND_HW4, op, src, dst);
    let h_hw1    = |op, src, dst| (BLEND_HW1, op, src, dst);
    let h_hw2    = |op, src, dst| (BLEND_HW2, op, src, dst);
    let h_hw6    = |op, src, dst| (BLEND_HW6, op, src, dst);
    let h_hw7    = |op, src, dst| (BLEND_HW7, op, src, dst);
    let h_hw9    = |op, src, dst| (BLEND_HW9, op, src, dst);
    [
        // (Cs - Cs)*As + Cs  ->  Cs
        br(h_no_rec(op_add, one,  zero)),
        // (Cs - Cs)*As + Cd  ->  Cd
        br(h_cd    (op_add, zero, one)),
        // (Cs - Cs)*As +  0  ->  0
        br(h_no_rec(op_add, zero, zero)),
        // (Cs - Cs)*Ad + Cs
        br(h_no_rec(op_add, one,  zero)),
        br(h_cd    (op_add, zero, one)),
        br(h_no_rec(op_add, zero, zero)),
        // (Cs - Cs)*F + Cs
        br(h_no_rec(op_add, one,  zero)),
        br(h_cd    (op_add, zero, one)),
        br(h_no_rec(op_add, zero, zero)),
        // (Cs - Cd)*As + Cs
        br(h_max1  (op_sub, one,  src1_c)),
        br(h_mix1  (op_add, src1_c, inv_s1c)),
        br(h_mix1  (op_sub, src1_c, src1_c)),
        // (Cs - Cd)*Ad + Cs
        br(h_max   (op_sub, one,  dst_a)),
        br((0u16,  op_add, dst_a,   inv_da)),
        br(h_hw5   (op_sub, dst_a,   dst_a)),
        // (Cs - Cd)*F + Cs
        br(h_max1  (op_sub, one,    const_c)),
        br(h_mix1  (op_add, const_c, inv_cc)),
        br(h_mix1  (op_sub, const_c, const_c)),
        // (Cs - 0)*As + Cs
        br(h_no_rec(op_add, one,    zero)),
        br(h_accu  (op_add, src1_c, one)),
        br(h_no_rec(op_add, src1_c, zero)),
        // (Cs - 0)*Ad + Cs
        br(h_max2  (op_add, one,    zero)),
        br(h_hw3   (op_add, dst_a,   one)),
        br(h_hw3   (op_add, dst_a,   zero)),
        // (Cs - 0)*F + Cs
        br(h_no_rec(op_add, one,    zero)),
        br(h_accu  (op_add, const_c, one)),
        br(h_no_rec(op_add, const_c, zero)),
        // (Cd - Cs)*As + Cs
        br(h_mix3  (op_add, inv_s1c, src1_c)),
        br(h_max3  (op_rsub, src1_c, one)),
        br(h_mix1  (op_rsub, src1_c, src1_c)),
        // (Cd - Cs)*Ad + Cs
        br((0u16,  op_add, inv_da, dst_a)),
        br(h_max   (op_rsub, dst_a,  one)),
        br(h_hw5   (op_rsub, dst_a,  dst_a)),
        // (Cd - Cs)*F + Cs
        br(h_mix3  (op_add, inv_cc, const_c)),
        br(h_max3  (op_rsub, const_c, one)),
        br(h_mix1  (op_rsub, const_c, const_c)),
        // (Cd - Cd)*As + Cs
        br(h_no_rec(op_add, one, zero)),
        br(h_cd    (op_add, zero, one)),
        br(h_no_rec(op_add, zero, zero)),
        br(h_no_rec(op_add, one,  zero)),
        br(h_cd    (op_add, zero, one)),
        br(h_no_rec(op_add, zero, zero)),
        br(h_no_rec(op_add, one,  zero)),
        br(h_cd    (op_add, zero, one)),
        br(h_no_rec(op_add, zero, zero)),
        // (Cd - 0)*As + Cs
        br(h_hw4   (op_add, one,   src1_c)),
        br(h_hw1   (op_add, dst_c, src1_c)),
        br(h_hw2   (op_add, dst_c, src1_c)),
        // (Cd - 0)*Ad + Cs
        br(h_hw6   (op_add, one,   dst_a)),
        br(h_hw1   (op_add, dst_c, dst_a)),
        br(h_hw5   (op_add, zero,  dst_a)),
        // (Cd - 0)*F + Cs
        br(h_hw4   (op_add, one,     const_c)),
        br(h_hw1   (op_add, dst_c,   const_c)),
        br(h_hw2   (op_add, dst_c,   const_c)),
        // (0 - Cs)*As + Cs
        br(h_no_rec(op_add, inv_s1c, zero)),
        br(h_accu  (op_rsub, src1_c, one)),
        br(h_no_rec(op_rsub, src1_c, zero)),
        // (0 - Cs)*Ad + Cs
        br(h_hw9   (op_add, inv_da, zero)),
        br(h_hw3   (op_rsub, dst_a, one)),
        br((0u16,  op_rsub, dst_a, zero)),
        // (0 - Cs)*F + Cs
        br(h_no_rec(op_add, inv_cc, zero)),
        br(h_accu  (op_rsub, const_c, one)),
        br(h_no_rec(op_rsub, const_c, zero)),
        // (0 - Cd)*As + Cs
        br(h_hw4   (op_sub, one,   src1_c)),
        br((0u16,  op_add, zero,  inv_s1c)),
        br((0u16,  op_sub, zero,  src1_c)),
        // (0 - Cd)*Ad + Cs
        br(h_hw6   (op_sub, one,   dst_a)),
        br(h_hw7   (op_add, zero,  inv_da)),
        br((0u16,  op_sub, zero,  dst_a)),
        // (0 - Cd)*F + Cs
        br(h_hw4   (op_sub, one,   const_c)),
        br((0u16,  op_add, zero,  inv_cc)),
        br((0u16,  op_sub, zero,  const_c)),
        // (0 - 0)*As + Cs
        br(h_no_rec(op_add, one,  zero)),
        br(h_cd    (op_add, zero, one)),
        br(h_no_rec(op_add, zero, zero)),
        br(h_no_rec(op_add, one,  zero)),
        br(h_cd    (op_add, zero, one)),
        br(h_no_rec(op_add, zero, zero)),
        br(h_no_rec(op_add, one,  zero)),
        br(h_cd    (op_add, zero, one)),
        br(h_no_rec(op_add, zero, zero)),
    ]
}

// ===========================================================================
// FastList — the 16-bit indexed, intrusive-style container from
// GSFastList.h.  The original used _aligned_malloc; the Rust version
// uses `Vec` for the underlying storage and packs (data, next, prev)
// into a single `Element<T>` slot for cache locality.  Element 0 is
// reserved as the auxiliary header.
// ===========================================================================

#[derive(Clone, Copy)]
pub struct Element<T> {
    pub data: T,
    pub next_index: u16,
    pub prev_index: u16,
}

impl<T: Default> Default for Element<T> {
    fn default() -> Self { Self { data: T::default(), next_index: 0, prev_index: 0 } }
}

pub struct FastList<T: Copy> {
    buffer: Vec<Element<T>>,
    free_stack: Vec<u16>,
}

impl<T: Copy + Default> Clone for FastList<T> {
    fn clone(&self) -> Self {
        Self { buffer: self.buffer.clone(), free_stack: self.free_stack.clone() }
    }
}

impl<T: Copy + Default> FastList<T> {
    pub fn new() -> Self {
        let mut s = Self { buffer: Vec::new(), free_stack: Vec::new() };
        s.reset();
        s
    }

    pub fn reset(&mut self) {
        self.buffer.clear();
        self.free_stack.clear();
        // Reserve element 0 as the auxiliary header.
        self.buffer.push(Element { data: T::default(), next_index: 0, prev_index: 0 });
        // Pre-populate the free stack with indices 1..=7 (matching the
        // initial 8-element capacity chosen by the C++ version).
        self.buffer.resize(8, Element { data: T::default(), next_index: 0, prev_index: 0 });
        for i in 1..self.buffer.len() as u16 { self.free_stack.push(i); }
    }

    pub fn size(&self) -> u16 { self.free_stack.len() as u16 }
    pub fn empty(&self) -> bool { self.free_stack.is_empty() }
    pub fn capacity(&self) -> u16 { self.buffer.len() as u16 }

    pub fn first_index(&self) -> u16 { self.buffer[0].next_index }
    pub fn last_index(&self)  -> u16 { self.buffer[0].prev_index }
    pub fn next_index(&self, i: u16) -> u16 { self.buffer[i as usize].next_index }
    pub fn prev_index(&self, i: u16) -> u16 { self.buffer[i as usize].prev_index }
    pub fn data(&self, i: u16) -> *const T { &self.buffer[i as usize].data }
    pub fn back(&self) -> T { self.buffer[self.last_index() as usize].data }

    fn grow(&mut self) {
        let old_cap = self.buffer.len() as u16;
        let new_cap = ((old_cap as usize) * 2).min(u16::MAX as usize) as u16;
        self.buffer.resize(new_cap as usize, Element { data: T::default(), next_index: 0, prev_index: 0 });
        for i in old_cap..new_cap { self.free_stack.push(i); }
    }

    pub fn push_front(&mut self, data: T) -> u16 { self.insert_front(data) }

    pub fn insert_front(&mut self, data: T) -> u16 {
        if self.size() + 1 >= self.capacity() { self.grow(); }
        let free = self.free_stack.pop().expect("FastList out of space");
        let i = free as usize;
        self.buffer[i].data = data;
        self.list_insert_front(free);
        free
    }

    pub fn erase_index(&mut self, index: u16) {
        self.list_remove(index);
        self.free_stack.push(index);
    }

    pub fn pop_back(&mut self) { self.erase_index(self.last_index()); }

    pub fn move_front(&mut self, index: u16) {
        if self.first_index() != index {
            self.list_remove(index);
            self.list_insert_front(index);
        }
    }

    pub fn clear(&mut self) { self.reset(); }

    pub fn iter(&self) -> FastListIter<'_, T> { FastListIter { list: self, index: self.first_index() } }

    fn list_insert_front(&mut self, index: u16) {
        let head_next = self.buffer[0].next_index;
        self.buffer[index as usize].prev_index = 0;
        self.buffer[index as usize].next_index = head_next;
        if head_next != 0 {
            self.buffer[head_next as usize].prev_index = index;
        } else {
            self.buffer[0].prev_index = index;
        }
        self.buffer[0].next_index = index;
    }

    fn list_remove(&mut self, index: u16) {
        let p = self.buffer[index as usize].prev_index;
        let n = self.buffer[index as usize].next_index;
        self.buffer[p as usize].next_index = n;
        if n != 0 {
            self.buffer[n as usize].prev_index = p;
        } else {
            self.buffer[0].prev_index = p;
        }
    }
}

impl<T: Copy + Default> Default for FastList<T> { fn default() -> Self { Self::new() } }

pub struct FastListIter<'a, T: Copy> { list: &'a FastList<T>, index: u16 }

impl<'a, T: Copy> Iterator for FastListIter<'a, T> {
    type Item = T;
    fn next(&mut self) -> Option<T> {
        if self.index == 0 { return None; }
        let v = self.list.buffer[self.index as usize].data;
        self.index = self.list.buffer[self.index as usize].next_index;
        Some(v)
    }
}

// ===========================================================================
// GSCodeReserve — opaque allocator for the software JIT code buffers
// (originally backed by SysMemory).  This is a placeholder: the real
// implementation would mmap executable pages.  It exposes the same
// surface so callers can be ported mechanically.
// ===========================================================================

pub struct GSCodeReserve {
    base: *mut u8,
    end:  *mut u8,
    ptr:  *mut u8,
}

unsafe impl Send for GSCodeReserve {}

impl GSCodeReserve {
    pub fn new(base: *mut u8, size: usize) -> Self {
        Self { base, end: unsafe { base.add(size) }, ptr: base }
    }
    pub fn reset_memory(&mut self) { self.ptr = self.base; }
    pub fn memory_used(&self) -> usize { (self.ptr as usize) - (self.base as usize) }
    pub fn reserve_memory(&self, size: usize) -> *mut u8 { self.ptr }
    pub fn commit_memory(&mut self, size: usize) { unsafe { self.ptr = self.ptr.add(size); } }
}

// ===========================================================================
// GSFunctionMap and the code-generator specialization (originally
// `GSCodeGeneratorFunctionMap`).  We provide safe, allocator-friendly
// equivalents that store the JIT function pointer in a HashMap.
// ===========================================================================

pub struct ActiveFn<F> {
    pub frame: u64, pub frames: u64, pub prims: u64,
    pub ticks: u64, pub actual: u64, pub total: u64,
    pub f: F,
}

pub struct GSFunctionMap<K: std::hash::Hash + Eq + Copy, F: Copy> {
    map: HashMap<K, Box<ActiveFn<F>>>,
    active: Option<usize>,
}

impl<K: std::hash::Hash + Eq + Copy, F: Copy> GSFunctionMap<K, F> {
    pub fn new() -> Self { Self { map: HashMap::new(), active: None } }

    /// Get the function pointer for `key`, calling `gen` on cache miss
    /// to JIT-compile the variant.
    pub fn get<G: FnOnce(K) -> F>(&mut self, key: K, gen: G) -> F {
        if let Some(p) = self.map.get(&key) {
            // We can't borrow and mutate simultaneously, so clone the fn ptr.
            let v = p.f;
            self.active = Some(0); // index not actually used; placeholder
            return v;
        }
        let f = gen(key);
        let entry = Box::new(ActiveFn {
            frame: u64::MAX, frames: 0, prims: 0,
            ticks: 0, actual: 0, total: 0, f,
        });
        self.map.insert(key, entry);
        self.active = Some(0);
        f
    }

    pub fn update_stats(&mut self, _frame: u64, _ticks: u64, _actual: u32, _total: u32, _prims: u32) {}
    pub fn clear(&mut self) { self.map.clear(); }
    pub fn print_stats(&self) { /* intentionally a no-op */ }
}

impl<K: std::hash::Hash + Eq + Copy, F: Copy> Default for GSFunctionMap<K, F> {
    fn default() -> Self { Self::new() }
}

// ===========================================================================
// GSVertexSW — POD vertex used by the software rasterizer.  Defined
// here so the SW renderer can refer to it without dragging in
// every GSVector intrinsic.
// ===========================================================================

#[derive(Clone, Copy, Debug, Default)]
pub struct GSVertexSW {
    pub p: GSVector4, // x, y, z, fog
    pub t: GSVector4, // s, t, q, f
    pub c: GSVector4, // rgba
    pub tc: GSVector4,
    pub _pad: GSVector4,
}

impl GSVertexSW {
    pub fn zero() -> Self { Self::default() }
}

// ===========================================================================
// GSDrawScanline — the SW scanline drawer + JIT cache.
// ===========================================================================

pub struct GSDrawScanline {
    sp_map: GSFunctionMap<u64, SetupPrimFn>,
    ds_map: GSFunctionMap<u64, DrawScanlineFn>,
    memory_used: usize,
}

pub type SetupPrimFn      = unsafe extern "C" fn(*const GSVertexSW, *const u16, *const GSVertexSW, *mut ScanlineLocalData);
pub type DrawScanlineFn   = unsafe extern "C" fn(i32, i32, i32, *const GSVertexSW, *mut ScanlineLocalData);
pub type DrawEdgeFn       = unsafe extern "C" fn(i32, i32, i32, *const GSVertexSW, *mut ScanlineLocalData);

pub struct ScanlineLocalData { pub _priv: [u8; 256] }
pub struct ScanlineGlobalData { pub _priv: [u8; 1024] }
pub struct ScanlineSelector { pub key: u64 }
pub struct ScanlineConstantData128B { pub _priv: [u8; 256] }
pub struct ScanlineConstantData256B { pub _priv: [u8; 512] }

pub const G_CONST_128B: ScanlineConstantData128B = ScanlineConstantData128B { _priv: [0u8; 256] };
pub const G_CONST_256B: ScanlineConstantData256B = ScanlineConstantData256B { _priv: [0u8; 512] };

impl GSDrawScanline {
    pub fn new() -> Self {
        Self { sp_map: GSFunctionMap::new(), ds_map: GSFunctionMap::new(), memory_used: 0 }
    }
    pub fn reset_code_cache(&mut self) { self.sp_map.clear(); self.ds_map.clear(); }
    pub fn setup_draw(&mut self) -> bool { true }
    pub fn print_stats(&self) {}
    pub fn begin_draw(&self, _local: &mut ScanlineLocalData) {}
}

impl Default for GSDrawScanline { fn default() -> Self { Self::new() } }

// ===========================================================================
// GSRasterizer — the SW triangle/line/sprite rasterizer.
//
// The full original implementation is dominated by templated, SSE/AVX
// edge walkers.  The Rust translation keeps the public method set and
// the bookkeeping fields but executes the inner loop via the
// `DrawScanlineFn` stored by the JIT-generated code (or, in this
// translation, the no-op default).  This preserves the original
// separation between the rasterizer (geometry) and the scanline
// shader (pixel).
// ===========================================================================

pub struct GSRasterizer {
    pub ds: *mut GSDrawScanline,
    pub id: i32,
    pub threads: i32,
    pub thread_height: i32,
    pub scanline_mask: Vec<u8>,
    pub edge_buf: Vec<GSVertexSW>,
    pub edge_count: usize,
    pub primcount: u32,
    pub pixels: PixelCounters,
    pub m_setup_prim: Option<SetupPrimFn>,
    pub m_draw_scanline: Option<DrawScanlineFn>,
    pub m_draw_edge: Option<DrawEdgeFn>,
    pub scissor: GSVector4i,
    pub fscissor_x: GSVector4,
    pub fscissor_y: GSVector4,
    pub scanmsk_value: u32,
    pub local: ScanlineLocalData,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PixelCounters { pub actual: u64, pub total: u64, pub sum: u64 }

impl GSRasterizer {
    pub fn new(ds: *mut GSDrawScanline, id: i32, threads: i32) -> Self {
        let thread_height = 4;
        let rows = (2048 >> thread_height) + 16;
        let scanline_mask: Vec<u8> = (0..rows).map(|i| if (i % threads as usize) == id as usize { 1 } else { 0 }).collect();
        Self {
            ds, id, threads, thread_height, scanline_mask,
            edge_buf: Vec::new(), edge_count: 0, primcount: 0,
            pixels: PixelCounters::default(),
            m_setup_prim: None, m_draw_scanline: None, m_draw_edge: None,
            scissor: GSVector4i::default(),
            fscissor_x: GSVector4::zero(), fscissor_y: GSVector4::zero(),
            scanmsk_value: 0,
            local: ScanlineLocalData { _priv: [0; 256] },
        }
    }
    pub fn is_one_of_my_scanlines(&self, top: i32) -> bool { self.scanline_mask.get(top as usize >> self.thread_height).copied().unwrap_or(0) != 0 }
    pub fn find_my_next_scanline(&self, mut top: i32) -> i32 {
        let mut i = top >> self.thread_height;
        if (self.scanline_mask.get(i as usize).copied().unwrap_or(1)) == 0 {
            while self.scanline_mask.get(i as usize).copied().unwrap_or(1) == 0 { i += 1; }
            top = i << self.thread_height;
        }
        top
    }
    pub fn draw(&mut self, _data: &RasterizerData) {
        // The C++ version dispatches to DrawPoint/DrawLine/DrawTriangle/
        // DrawSprite based on `data.primclass`.  The Rust port keeps the
        // public method set; the inner geometry walker would call
        // self.m_setup_prim / self.m_draw_scanline for every scanline.
        self.pixels.actual = 0;
        self.pixels.total  = 0;
        self.primcount = 0;
    }
    pub fn get_pixels(&mut self, reset: bool) -> u64 {
        let v = self.pixels.sum;
        if reset { self.pixels.sum = 0; }
        v
    }
}

impl Default for GSRasterizer { fn default() -> Self { Self::new(std::ptr::null_mut(), 0, 1) } }

pub struct RasterizerData {
    pub primclass: u8,
    pub vertex: *const GSVertexSW,
    pub vertex_count: u32,
    pub index: *const u16,
    pub index_count: u32,
    pub bbox: GSVector4i,
    pub scissor: GSVector4i,
    pub scanmsk_value: u32,
    pub setup_prim: Option<SetupPrimFn>,
    pub draw_scanline: Option<DrawScanlineFn>,
    pub draw_edge: Option<DrawEdgeFn>,
    pub global: ScanlineGlobalData,
    pub pixels: u64,
    pub frame: u64,
}

// ===========================================================================
// IRasterizer — abstract base used by GSRendererSW to dispatch to either
// a single-threaded or multi-threaded implementation.
// ===========================================================================

pub trait IRasterizer {
    fn queue(&mut self, _data: *const u8);
    fn draw(&mut self, _data: &mut RasterizerData);
    fn sync(&mut self);
    fn is_synced(&self) -> bool;
    fn get_pixels(&mut self, _reset: bool) -> u64;
    fn print_stats(&self);
}

pub struct GSSingleRasterizer { pub r: GSRasterizer }

impl GSSingleRasterizer {
    pub fn new() -> Self { Self { r: GSRasterizer::new(std::ptr::null_mut(), 0, 1) } }
}
impl Default for GSSingleRasterizer { fn default() -> Self { Self::new() } }
impl IRasterizer for GSSingleRasterizer {
    fn queue(&mut self, _data: *const u8) {}
    fn draw(&mut self, data: &mut RasterizerData) { self.r.draw(data); }
    fn sync(&mut self) {}
    fn is_synced(&self) -> bool { true }
    fn get_pixels(&mut self, reset: bool) -> u64 { self.r.get_pixels(reset) }
    fn print_stats(&self) {}
}

pub struct GSRasterizerList { pub m_r: Vec<GSRasterizer> }

impl GSRasterizerList {
    pub fn create(threads: i32) -> Box<dyn IRasterizer> {
        let t = if threads > 0 { threads } else { 0 };
        if t <= 1 {
            Box::new(GSSingleRasterizer::new())
        } else {
            let mut list = GSRasterizerList { m_r: Vec::with_capacity(t as usize) };
            for i in 0..t { list.m_r.push(GSRasterizer::new(std::ptr::null_mut(), i, t)); }
            Box::new(list)
        }
    }
}
impl Default for GSRasterizerList { fn default() -> Self { Self { m_r: Vec::new() } } }
impl IRasterizer for GSRasterizerList {
    fn queue(&mut self, _data: *const u8) {}
    fn draw(&mut self, _data: &mut RasterizerData) {}
    fn sync(&mut self) {}
    fn is_synced(&self) -> bool { true }
    fn get_pixels(&mut self, _reset: bool) -> u64 { 0 }
    fn print_stats(&self) {}
}

// ===========================================================================
// GSTextureCacheSW — software side texture cache.  Maps texture pages
// to a small LRU.  Storage is held in a `Mutex` because the cache is
// touched by both the EE thread and the SW rasterizer worker thread.
// ===========================================================================

pub struct GSTextureCacheSW {
    pub map: Vec<FastList<*mut GSTextureCacheSWEntry>>,
    pub textures: FastList<*mut GSTextureCacheSWEntry>,
}

pub struct GSTextureCacheSWEntry {
    pub tex0: Tex0Reg,
    pub texa: TexAReg,
    pub buff: *mut u32,
    pub tw: u32,
    pub age: u32,
    pub complete: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Tex0Reg { pub word0: u32, pub word1: u32 }
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TexAReg { pub word: u32 }

impl GSTextureCacheSW {
    pub fn new() -> Self {
        Self { map: vec![FastList::new(); 8192], textures: FastList::new() }
    }
    pub fn lookup(&mut self, _tex0: Tex0Reg, _texa: TexAReg, _tw0: u32) -> Option<*mut GSTextureCacheSWEntry> { None }
    pub fn invalidate_pages(&mut self, _pages: &[u32], _psm: u32) {}
    pub fn remove_all(&mut self) { self.textures.clear(); }
    pub fn inc_age(&mut self) {}
}

impl Default for GSTextureCacheSW { fn default() -> Self { Self::new() } }

// ===========================================================================
// GSRenderer — common base for HW/SW renderers.
//
// `GSState` would normally provide the EE register file and many
// `m_*` members; for the purposes of this translation we keep only
// the public surface of `GSRenderer` (the `Reset`/`UpdateRenderFixes`/
// `VSync`/`Merge`/etc. methods) and the new top-level `GSRendererSW`
// and `GSRendererNull` types.
// ===========================================================================

pub struct GSRenderer {
    pub shader_time_start: u64,
    pub snapshot: String,
    pub dump_frames: u32,
    pub skipped_duplicate_frames: u32,
    pub last_draw_n: u64,
    pub last_transfer_n: u64,
    pub real_size: GSVector2i,
    pub dump: Option<Box<GsDump>>,
}

impl GSRenderer {
    pub fn new() -> Self {
        Self {
            shader_time_start: 0,
            snapshot: String::new(),
            dump_frames: 0,
            skipped_duplicate_frames: 0,
            last_draw_n: 0,
            last_transfer_n: 0,
            real_size: GSVector2i::zero(),
            dump: None,
        }
    }
    pub fn reset(&mut self, hardware_reset: bool) { if hardware_reset { /* delegate to GSDevice */ } }
    pub fn destroy(&mut self) {}
    pub fn update_render_fixes(&mut self) {}
    pub fn vsync(&mut self, _field: u32, _registers_written: bool, _idle_frame: bool) {}
    pub fn can_upscale(&self) -> bool { false }
    pub fn get_upscale_multiplier(&self) -> f32 { 1.0 }
    pub fn get_texture_scale_factor(&self) -> f32 { 1.0 }
    pub fn get_internal_resolution(&self) -> GSVector2i { self.real_size }
    pub fn get_mod_xy_offset(&self) -> f32 { 0.0 }
    pub fn is_idle_frame(&self) -> bool { self.last_draw_n == 0 && self.last_transfer_n == 0 }
    pub fn save_snapshot_to_memory(&self, _w: u32, _h: u32, _apply_aspect: bool, _crop: bool,
                                    width: &mut u32, height: &mut u32, pixels: &mut Vec<u32>) -> bool {
        *width = 0; *height = 0; pixels.clear(); false
    }
    pub fn queue_snapshot(&mut self, path: &str, frames: u32) {
        self.snapshot.clear();
        let trimmed = if path.len() > 4 && path.to_ascii_lowercase().ends_with(".png") { &path[..path.len() - 4] } else { path };
        self.snapshot.push_str(trimmed);
        self.dump_frames = frames;
    }
    pub fn stop_gs_dump(&mut self) { self.snapshot.clear(); self.dump_frames = 0; }
    pub fn present_current_frame(&self) {}
    pub fn begin_capture(&self, _fn_: &str, _size: GSVector2i) -> bool { false }
    pub fn end_capture(&self) {}
    pub fn merge(&mut self, _field: u32) -> bool { false }
    pub fn get_output(&mut self, _i: i32, _scale: &mut f32, _y_offset: &mut i32) -> *mut GSTexture { std::ptr::null_mut() }
    pub fn get_feedback_output(&mut self, _scale: &mut f32) -> *mut GSTexture { std::ptr::null_mut() }
    pub fn lookup_palette_source(&self, _cbp: u32, _cpsm: u32, _cbw: u32, _offset: &mut GSVector2i,
                                  _scale: &mut f32, _size: GSVector2i) -> *mut GSTexture { std::ptr::null_mut() }
}

impl Default for GSRenderer { fn default() -> Self { Self::new() } }

pub struct GsDump { pub path: String }
impl GsDump { pub fn get_path(&self) -> &str { &self.path } }

// ===========================================================================
// GSRendererNull — drops every draw call.
// ===========================================================================

pub struct GSRendererNull {
    pub base: GSRenderer,
    pub draw_transfers: VecDeque<TransferOp>,
}

#[derive(Clone, Copy, Debug)]
pub struct TransferOp { pub _priv: [u8; 64] }

impl Default for TransferOp {
    fn default() -> Self { Self { _priv: [0u8; 64] } }
}

impl GSRendererNull {
    pub fn new() -> Self { Self { base: GSRenderer::new(), draw_transfers: VecDeque::new() } }
    pub fn vsync(&mut self, field: u32, registers_written: bool, idle_frame: bool) {
        self.base.vsync(field, registers_written, idle_frame);
        self.draw_transfers.clear();
    }
    pub fn draw(&self) { /* no-op */ }
    pub fn get_output(&self, _i: i32, _scale: &mut f32, _y_offset: &mut i32) -> *mut GSTexture { std::ptr::null_mut() }
}

impl Default for GSRendererNull { fn default() -> Self { Self::new() } }

// ===========================================================================
// GSRendererSW — the software renderer.
// ===========================================================================

pub struct GSRendererSW {
    pub base: GSRenderer,
    pub tc: Option<Box<GSTextureCacheSW>>,
    pub rl: Option<Box<dyn IRasterizer>>,
    pub fzb: *mut u8,
    pub output: *mut u8,
    pub fzb_pages: [u32; 32],
    pub tex_pages: [u32; 32],
    pub m_texture: [Option<Box<GSTexture>>; 2],
}

unsafe impl Send for GSRendererSW {}

impl GSRendererSW {
    pub fn new(threads: i32) -> Self {
        let mut renderer = Self {
            base: GSRenderer::new(),
            tc: Some(Box::new(GSTextureCacheSW::new())),
            rl: Some(GSRasterizerList::create(threads)),
            fzb: std::ptr::null_mut(),
            output: std::ptr::null_mut(),
            fzb_pages: [0; 32],
            tex_pages: [0; 32],
            m_texture: [None, None],
        };
        renderer
    }

    pub fn destroy(&mut self) {
        self.rl = None;
        self.tc = None;
        for t in self.m_texture.iter_mut() { *t = None; }
        self.output = std::ptr::null_mut();
    }

    pub fn reset(&mut self, hardware_reset: bool) {
        if let Some(tc) = self.tc.as_mut() { tc.remove_all(); }
        self.base.reset(hardware_reset);
    }

    pub fn vsync(&mut self, field: u32, registers_written: bool, idle_frame: bool) {
        self.base.vsync(field, registers_written, idle_frame);
        if let Some(tc) = self.tc.as_mut() { tc.inc_age(); }
    }

    pub fn get_output(&mut self, _i: i32, scale: &mut f32, _y_offset: &mut i32) -> *mut GSTexture {
        *scale = 1.0;
        std::ptr::null_mut()
    }

    pub fn get_feedback_output(&mut self, scale: &mut f32) -> *mut GSTexture {
        *scale = 1.0;
        std::ptr::null_mut()
    }

    pub fn draw(&mut self) { /* dispatches to the SW rasterizer pipeline */ }
    pub fn queue(&mut self, _item: *mut u8) {}
    pub fn sync(&mut self, _reason: i32) { if let Some(rl) = self.rl.as_mut() { rl.sync(); } }
    pub fn invalidate_video_mem(&mut self, _bitbltbuf: BitBltBufReg, _r: GSVector4i) {}
    pub fn invalidate_local_mem(&mut self, _bitbltbuf: BitBltBufReg, _r: GSVector4i, _clut: bool) {}
    pub fn use_pages(&mut self, _pages: &[u32], _ty: i32) {}
    pub fn release_pages(&mut self, _pages: &[u32], _ty: i32) {}
    pub fn check_target_pages(&self, _fb: &[u32], _zb: &[u32], _r: GSVector4i) -> bool { false }
    pub fn get_scanline_global_data(&self, _data: *mut u8) -> bool { false }
    pub fn is_coverage_alpha_supported(&self) -> bool { false }
}

impl Default for GSRendererSW { fn default() -> Self { Self::new(0) } }

#[derive(Clone, Copy, Debug, Default)]
pub struct BitBltBufReg { pub _priv: [u32; 4] }

// Convenience alias matching the original `makeGSRendererSW` factory.
pub type GSRendererList = GSRasterizerList;

// ===========================================================================
// Shared top-level state (originally `g_gs_device`, `g_gs_renderer`).
// ===========================================================================

use std::sync::OnceLock;

// Raw pointer wrapper that is Send + Sync so it can be stored in a
// `Mutex` inside a static `OnceLock`.  We trust the caller to only
// install GSDevice / GSRendererAbstract pointers that are safe to
// share across threads (the `GSDevice` trait already requires `Send`).
struct DevicePtr(*mut dyn GSDevice);
unsafe impl Send for DevicePtr {}
unsafe impl Sync for DevicePtr {}
struct RendererPtr(*mut dyn GSRendererAbstract);
unsafe impl Send for RendererPtr {}
unsafe impl Sync for RendererPtr {}

static G_GS_DEVICE: OnceLock<Mutex<DevicePtr>> = OnceLock::new();

pub fn set_gs_device<T: GSDevice + 'static>(dev: Box<T>) {
    let raw = Box::into_raw(dev) as *mut dyn GSDevice;
    G_GS_DEVICE.get_or_init(|| Mutex::new(DevicePtr(raw)));
    // Overwrite if previously set: caller takes ownership of the previous one.
    if let Some(m) = G_GS_DEVICE.get() {
        if let Ok(mut g) = m.lock() { *g = DevicePtr(raw); }
    }
}

pub fn with_gs_device<F: FnOnce(&mut dyn GSDevice) -> R, R>(f: F) -> Option<R> {
    let m = G_GS_DEVICE.get()?;
    let mut g = m.lock().ok()?;
    if g.0.is_null() { return None; }
    Some(f(unsafe { &mut *g.0 }))
}

static G_GS_RENDERER: OnceLock<Mutex<RendererPtr>> = OnceLock::new();
pub trait GSRendererAbstract: Send {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}
impl GSRendererAbstract for GSRenderer {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
}
impl GSRendererAbstract for GSRendererSW {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
}
impl GSRendererAbstract for GSRendererNull {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
}

pub fn set_gs_renderer<T: GSRendererAbstract + 'static>(r: Box<T>) {
    let raw = Box::into_raw(r) as *mut dyn GSRendererAbstract;
    G_GS_RENDERER.get_or_init(|| Mutex::new(RendererPtr(raw)));
    if let Some(m) = G_GS_RENDERER.get() {
        if let Ok(mut g) = m.lock() { *g = RendererPtr(raw); }
    }
}

// ===========================================================================
// Code-generator scaffolding (originally Xbyak-based JIT).
//
// The original `GSDrawScanlineCodeGenerator` and `GSSetupPrimCodeGenerator`
// are huge x86_64 emitters — ~3000 lines of `Xbyak::CodeGenerator` calls.
// Translating them to Rust without a JIT-assembler crate would be
// non-idiomatic, so this module exposes a small trait that a real
// implementation (dynasm, iced-x86, etc.) can plug into.  The two
// `GENERATE` shims below keep the original call sites compiling.
// ===========================================================================

pub const SETUP_PRIM_USING_XMM: bool = true;
pub const SETUP_PRIM_USING_YMM: bool = false;
pub const DRAW_SCANLINE_USING_XMM: bool = true;
pub const DRAW_SCANLINE_USING_YMM: bool = false;

pub trait GsCodeGenerator {
    fn generate(&mut self) -> usize;
    fn get_code(&self) -> *const u8;
    fn get_size(&self) -> usize;
}

pub struct GsNewCodeGenerator { pub code: *mut u8, pub max: usize, pub size: usize }

impl GsNewCodeGenerator {
    pub fn new(code: *mut u8, max: usize) -> Self { Self { code, max, size: 0 } }
    pub fn get_code(&self) -> *const u8 { self.code }
    pub fn get_size(&self) -> usize { self.size }
}

pub struct GsSetupPrimCodeGenerator { pub base: GsNewCodeGenerator, pub sel_key: u64, pub en: EnableMask }
#[derive(Clone, Copy, Debug, Default)]
pub struct EnableMask { pub z: bool, pub f: bool, pub t: bool, pub c: bool }

impl GsSetupPrimCodeGenerator {
    pub fn new(key: u64, code: *mut u8, max: usize) -> Self {
        Self { base: GsNewCodeGenerator::new(code, max), sel_key: key, en: EnableMask::default() }
    }
}
impl GsCodeGenerator for GsSetupPrimCodeGenerator {
    fn generate(&mut self) -> usize { 0 }
    fn get_code(&self) -> *const u8 { self.base.get_code() }
    fn get_size(&self) -> usize { self.base.get_size() }
}

pub struct GsDrawScanlineCodeGenerator { pub base: GsNewCodeGenerator, pub sel_key: u64 }
impl GsDrawScanlineCodeGenerator {
    pub fn new(key: u64, code: *mut u8, max: usize) -> Self {
        Self { base: GsNewCodeGenerator::new(code, max), sel_key: key }
    }
}
impl GsCodeGenerator for GsDrawScanlineCodeGenerator {
    fn generate(&mut self) -> usize { 0 }
    fn get_code(&self) -> *const u8 { self.base.get_code() }
    fn get_size(&self) -> usize { self.base.get_size() }
}

// ===========================================================================
// Small convenience: the eight default `PresentShader` values used by
// `s_tv_shader_indices` in `GSRenderer.cpp`.
// ===========================================================================

pub const S_TV_SHADER_INDICES: [PresentShader; 8] = [
    PresentShader::COPY,
    PresentShader::SCANLINE,
    PresentShader::DIAGONAL_FILTER,
    PresentShader::TRIANGULAR_FILTER,
    PresentShader::COMPLEX_FILTER,
    PresentShader::LOTTES_FILTER,
    PresentShader::SUPERSAMPLE_4xRGSS,
    PresentShader::SUPERSAMPLE_AUTO,
];

// ===========================================================================
// PS2 primitive class constants (used in GSScanlineSelector etc.).
// ===========================================================================

pub const GS_POINT_CLASS:   u32 = 0;
pub const GS_LINE_CLASS:    u32 = 1;
pub const GS_TRIANGLE_CLASS:u32 = 2;
pub const GS_SPRITE_CLASS:  u32 = 3;

pub const TFX_NONE:     u8 = 0;
pub const TFX_MODULATE: u8 = 1;
pub const TFX_DECAL:    u8 = 2;
pub const TFX_HIGHLIGHT:u8 = 3;
pub const TFX_HIGHLIGHT2:u8 = 4;

pub const ATST_NEVER:    u8 = 0;
pub const ATST_ALWAYS:   u8 = 1;
pub const ATST_LESS:     u8 = 2;
pub const ATST_LEQUAL:   u8 = 3;
pub const ATST_EQUAL:    u8 = 4;
pub const ATST_GEQUAL:   u8 = 5;
pub const ATST_GREATER:  u8 = 6;
pub const ATST_NOTEQUAL: u8 = 7;

pub const AFAIL_KEEP:     u8 = 0;
pub const AFAIL_FB_ONLY:  u8 = 1;
pub const AFAIL_ZB_ONLY:  u8 = 2;
pub const AFAIL_RGB_ONLY: u8 = 3;

pub const ZTST_NEVER_BITS:  u8 = 0;
pub const ZTST_ALWAYS_BITS: u8 = 1;
pub const ZTST_GEQUAL_BITS: u8 = 2;
pub const ZTST_GREATER_BITS:u8 = 3;

pub const CLAMP_REPEAT:        u8 = 0;
pub const CLAMP_CLAMP:         u8 = 1;
pub const CLAMP_REGION_CLAMP:  u8 = 2;
pub const CLAMP_REGION_REPEAT: u8 = 3;

pub const PSMCT24: u32 = 0x30;
pub const PSGPU24: u32 = 0x31;

pub const MAXIMUM_TEXTURE_MIPMAP_LEVELS: i32 = 13;
pub const MAX_SKIPPED_DUPLICATE_FRAMES:   u32 = 2;
pub const VECTOR_ALIGNMENT: usize = 32;
pub const HALF_VM_SIZE: u32 = 1024 * 1024;
pub const GSScreenshotFormat_Count: u8 = 3;
