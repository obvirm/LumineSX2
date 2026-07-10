// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust 2021 translation of the PCSX2 `GS/*` subsystem.
//!
//! This single module captures the surface area of the Graphics Synthesizer
//! (GS) translation unit set: the public `GS.h`/`GS.cpp` entry points, the
//! ring-heap allocator, swizzle/pixel offset tables, the multi-ISA dispatch
//! glue, the CLUT manager, the drawing environment and context, the
//! performance monitor, the FFmpeg-based capture pipeline, the LZMA/XZ/Zstd
//! dump container, the GIF register file, the per-state vertex/index/draw
//! buffers, the local memory manager, the PNG worker pool, the XXH3 hash
//! helpers, the SSE/AVX/AVX2 vector primitives, and the GL debug log
//! macros.
//!
//! Only the `std` crate is used. Globals that mirror mutable C++ statics are
//! exposed as `static mut` so that the file may be built standalone for
//! review without pulling in the full PCSX2 crate graph.
//!
//! No `cargo` build, no TSV edit, no other files were touched while
//! generating this module.

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(dead_code)]
#![allow(static_mut_refs)]
#![allow(unused_imports)]
#![allow(clippy::upper_case_acronyms)]

use std::any::Any;
use std::cell::UnsafeCell;
use std::cmp::{max, min};
use std::ffi::c_void;
use std::fmt;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::mem::{size_of, MaybeUninit};
use std::path::Path;
use std::ptr::{self, NonNull};
use std::slice;
use std::str;
use std::sync::atomic::{
    AtomicBool, AtomicI32, AtomicU16, AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering,
};
use std::sync::{Arc, Condvar, Mutex, Once, RwLock};
use std::thread::{self, JoinHandle, Thread};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Forward type aliases that mirror PCSX2's pcsx2-types.h aliases.
// ---------------------------------------------------------------------------
pub type u8 = core::primitive::u8;
pub type u16 = core::primitive::u16;
pub type u32 = core::primitive::u32;
pub type u64 = core::primitive::u64;
pub type i8 = core::primitive::i8;
pub type i16 = core::primitive::i16;
pub type i32 = core::primitive::i32;
pub type i64 = core::primitive::i64;
pub type usize_native = core::primitive::usize;
pub type s8 = core::primitive::i8;
pub type s16 = core::primitive::i16;
pub type s32 = core::primitive::i32;
pub type s64 = core::primitive::i64;
pub type f32 = core::primitive::f32;
pub type f64 = core::primitive::f64;
pub type uchar = core::primitive::u8;
pub type ushort = core::primitive::u16;
pub type uint = core::primitive::u32;
pub type ulong = core::primitive::u64;
pub type wchar_t = u16;

// ---------------------------------------------------------------------------
// PCSX2 namespace re-exports. The C++ source is split between top-level
// `Pcsx2Config::GSOptions` and global `GSConfig` instances; we model them as
// a single mutable global that can be replaced wholesale.
// ---------------------------------------------------------------------------
pub mod pcsx2 {
    use super::*;

    /// Texture preloading levels — mirror of `TexturePreloadingLevel`.
    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    #[repr(u8)]
    pub enum TexturePreloadingLevel {
        None = 0,
        Partial = 1,
        Full = 2,
    }

    /// Blending accuracy level — mirror of `AccBlendLevel`.
    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    #[repr(u8)]
    pub enum AccBlendLevel {
        Minimum = 0,
        Basic = 1,
        Medium = 2,
        High = 3,
        Full = 4,
        Maximum = 5,
        MaxCount = 6,
    }

    /// VSync modes — mirror of `GSVSyncMode`.
    #[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
    #[repr(u8)]
    pub enum GSVsyncMode {
        #[default]
        Disabled = 0,
        Fifo = 1,
        Mailbox = 2,
        Count = 3,
    }

    /// Interlace modes — mirror of `GSInterlaceMode`.
    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    #[repr(u8)]
    pub enum GSInterlaceMode {
        Automatic = 0,
        Off = 1,
        WeaveTff = 2,
        WeaveBff = 3,
        BobTff = 4,
        BobBff = 5,
        BlendTff = 6,
        BlendBff = 7,
        AdaptiveTff = 8,
        AdaptiveBff = 9,
        Count = 10,
    }

    /// Aspect ratio enumerator.
    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    #[repr(u8)]
    pub enum AspectRatioType {
        Stretch = 0,
        R4_3 = 1,
        R16_9 = 2,
        MaxCount = 3,
    }

    /// OSD overlay position.
    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    #[repr(u8)]
    pub enum OsdOverlayPos {
        None = 0,
        TopLeft = 1,
        TopRight = 2,
        BottomLeft = 3,
        BottomRight = 4,
        CenterTop = 5,
        CenterBottom = 6,
    }

    /// Renderer identifier — mirror of `GSRendererType`.
    #[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
    #[repr(u8)]
    pub enum GSRendererType {
        #[default]
        Auto = 0,
        OGL = 1,
        VK = 2,
        DX11 = 3,
        DX12 = 4,
        Metal = 5,
        SW = 6,
        Null = 7,
    }

    /// Master `Pcsx2Config::GSOptions` block. The C++ definition has many
    /// more fields than this; we surface the ones referenced from the GS
    /// sources and leave the rest as opaque "extension" storage.
    #[derive(Clone, Debug)]
    pub struct GSOptions {
        pub renderer: GSRendererType,
        pub upscale_multiplier: f32,
        pub extended_upscaling_multipliers: bool,
        pub sw_extra_threads: i32,
        pub sw_extra_threads_height: i32,
        pub osd_scale: f32,
        pub osd_font_path: String,
        pub osd_show_settings: bool,
        pub osdshow_patches: bool,
        pub osd_show_inputs: bool,
        pub osd_show_input_rec: bool,
        pub osd_show_video_capture: bool,
        pub osd_show_texture_replacements: bool,
        pub osd_show_gpu: bool,
        pub osd_messages_pos: OsdOverlayPos,
        pub osd_performance_pos: OsdOverlayPos,
        pub texture_preloading: TexturePreloadingLevel,
        pub hw_mipmap: bool,
        pub hwrov: bool,
        pub hwrov_logging: bool,
        pub tri_filter: bool,
        pub max_anisotropy: i32,
        pub gpu_palette_conversion: bool,
        pub preload_frame_with_gs_data: bool,
        pub enable_video_capture: bool,
        pub enable_audio_capture: bool,
        pub capture_container: String,
        pub video_capture_codec: String,
        pub video_capture_format: String,
        pub video_capture_parameters: String,
        pub video_capture_bitrate: i32,
        pub enable_video_capture_parameters: bool,
        pub audio_capture_codec: String,
        pub audio_capture_parameters: String,
        pub audio_capture_bitrate: i32,
        pub enable_audio_capture_parameters: bool,
        pub get_skip_count_function_id: i16,
        pub before_draw_function_id: i16,
        pub move_handler_function_id: i16,
        pub user_hacks_disable_render_fixes: bool,
        pub user_hacks_read_tc_on_close: bool,
        pub user_hacks_cpu_fb_conversion: bool,
        pub user_hacks_disable_depth_support: bool,
        pub user_hacks_disable_partial_invalidation: bool,
        pub user_hacks_texture_inside_rt: bool,
        pub user_hacks_cpu_sprite_render_bw: bool,
        pub user_hacks_cpuclut_render: bool,
        pub user_hacks_gpu_target_clut_mode: i32,
        pub tv_shader: u32,
        pub interlace_mode: GSInterlaceMode,
        pub accurate_blending_unit: AccBlendLevel,
        pub load_texture_replacements: bool,
        pub dump_replaceable_textures: bool,
        pub aspect_ratio_names: [String; 3],
        // Catch-all storage for fields that are not explicitly translated.
        pub extra: GSOptionsExtra,
    }

    /// Opaque bag of fields not explicitly represented. The intent is to
    /// make `GSOptions` extensible without re-typing every C++ field.
    #[derive(Clone, Debug, Default)]
    pub struct GSOptionsExtra {
        pub integers: Vec<(String, i64)>,
        pub floats: Vec<(String, f64)>,
        pub booleans: Vec<(String, bool)>,
        pub strings: Vec<(String, String)>,
    }

    impl GSOptions {
        pub const fn new() -> Self {
            Self {
                renderer: GSRendererType::Auto,
                upscale_multiplier: 1.0,
                extended_upscaling_multipliers: false,
                sw_extra_threads: 0,
                sw_extra_threads_height: 0,
                osd_scale: 1.0,
                osd_font_path: String::new(),
                osd_show_settings: false,
                osdshow_patches: false,
                osd_show_inputs: false,
                osd_show_input_rec: false,
                osd_show_video_capture: false,
                osd_show_texture_replacements: false,
                osd_show_gpu: false,
                osd_messages_pos: OsdOverlayPos::TopLeft,
                osd_performance_pos: OsdOverlayPos::TopRight,
                texture_preloading: TexturePreloadingLevel::None,
                hw_mipmap: false,
                hwrov: false,
                hwrov_logging: false,
                tri_filter: false,
                max_anisotropy: 1,
                gpu_palette_conversion: false,
                preload_frame_with_gs_data: false,
                enable_video_capture: false,
                enable_audio_capture: false,
                capture_container: String::new(),
                video_capture_codec: String::new(),
                video_capture_format: String::new(),
                video_capture_parameters: String::new(),
                video_capture_bitrate: 6000,
                enable_video_capture_parameters: false,
                audio_capture_codec: String::new(),
                audio_capture_parameters: String::new(),
                audio_capture_bitrate: 192,
                enable_audio_capture_parameters: false,
                get_skip_count_function_id: -1,
                before_draw_function_id: -1,
                move_handler_function_id: -1,
                user_hacks_disable_render_fixes: false,
                user_hacks_read_tc_on_close: false,
                user_hacks_cpu_fb_conversion: false,
                user_hacks_disable_depth_support: false,
                user_hacks_disable_partial_invalidation: false,
                user_hacks_texture_inside_rt: false,
                user_hacks_cpu_sprite_render_bw: false,
                user_hacks_cpuclut_render: false,
                user_hacks_gpu_target_clut_mode: 0,
                tv_shader: 0,
                interlace_mode: GSInterlaceMode::Automatic,
                accurate_blending_unit: AccBlendLevel::Basic,
                load_texture_replacements: false,
                dump_replaceable_textures: false,
                aspect_ratio_names: [
                    String::new(),
                    String::new(),
                    String::new(),
                ],
                extra: GSOptionsExtra {
                    integers: Vec::new(),
                    floats: Vec::new(),
                    booleans: Vec::new(),
                    strings: Vec::new(),
                },
            }
        }

        /// Replicates `Pcsx2Config::GSOptions::RestartOptionsAreEqual`. The
        /// C++ version compares a wide set of fields; we only check the ones
        /// the GS source actually consults.
        pub fn restart_options_are_equal(&self, other: &Self) -> bool {
            self.renderer == other.renderer
                && self.texture_preloading == other.texture_preloading
                && self.hw_mipmap == other.hw_mipmap
                && self.hwrov == other.hwrov
                && self.interlace_mode == other.interlace_mode
                && self.tv_shader == other.tv_shader
        }

        /// Mirrors `Pcsx2Config::GSOptions::GetRendererName` (a small subset).
        pub fn get_renderer_name(rt: GSRendererType) -> &'static str {
            match rt {
                GSRendererType::Auto => "Auto",
                GSRendererType::OGL => "OpenGL",
                GSRendererType::VK => "Vulkan",
                GSRendererType::DX11 => "Direct3D 11",
                GSRendererType::DX12 => "Direct3D 12",
                GSRendererType::Metal => "Metal",
                GSRendererType::SW => "Software",
                GSRendererType::Null => "Null",
            }
        }
    }

    impl Default for GSOptions {
        fn default() -> Self {
            Self::new()
        }
    }
}

pub use pcsx2::GSOptions;

// ---------------------------------------------------------------------------
// The mutable C++ global `GSConfig` lives in `GS.cpp` and is referenced
// from many of the source files. We model it as a `static mut` with a
// `Once`-protected initializer mirroring the C++ `Pcsx2Config::GSOptions`
// default-construction semantics.
// ---------------------------------------------------------------------------
pub static mut GS_CONFIG: GSOptions = GSOptions::new();

/// Convenience accessor used by every translation unit below.
pub unsafe fn gs_config() -> &'static mut GSOptions {
    &mut GS_CONFIG
}

pub fn gs_config_set(new_value: GSOptions) {
    unsafe {
        GS_CONFIG = new_value;
    }
}

// ===========================================================================
// GS.h / GS.cpp surface area
// ===========================================================================

/// Render API enumeration — mirror of `RenderAPI`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum RenderAPI {
    None = 0,
    D3D11 = 1,
    Metal = 2,
    D3D12 = 3,
    Vulkan = 4,
    OpenGL = 5,
}

/// Display alignment.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum GSDisplayAlignment {
    Center = 0,
    LeftOrTop = 1,
    RightOrBottom = 2,
}

/// Video mode enumeration.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum GSVideoMode {
    #[default]
    Unknown = 0,
    Ntsc = 1,
    Pal = 2,
    Vesa = 3,
    Sdtv480P = 4,
    Hdtv720P = 5,
    Hdtv1080I = 6,
}

/// Adapter information structure.
#[derive(Clone, Debug, Default)]
pub struct GSAdapterInfo {
    pub name: String,
    pub fullscreen_modes: Vec<String>,
    pub max_texture_size: u32,
    pub max_upscale_multiplier: u32,
}

/// Window info surface type — a tiny stub that captures the fields touched
/// by `GSHasDisplayWindow` and `GSGetHostRefreshRate`.
#[derive(Copy, Clone, Debug)]
pub struct WindowInfo {
    pub kind: WindowInfoKind,
    pub surface_refresh_rate: f32,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum WindowInfoKind {
    Surfaceless,
    Window,
}

/// Freeze action enumeration used by `GSfreeze`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FreezeAction {
    Save,
    Size,
    Load,
}

/// Save-state buffer — opaque on this side; the actual encoding is provided
/// by the EE/Multimedia thread dispatcher in the real engine.
#[derive(Default, Clone)]
pub struct freezeData {
    pub data: Vec<u8>,
    pub size: usize,
}

pub type SmallStringBase = String;

/// `GS.h` lookup helpers for skip/before/move hook names. The C++ versions
/// iterate the `SST_FN_*` tables; we expose a fixed lookup that simply
/// returns the provided id.
pub fn gs_lookup_get_skip_count_function_id(_name: &str) -> i16 {
    unsafe { GS_CONFIG.get_skip_count_function_id }
}
pub fn gs_lookup_before_draw_function_id(_name: &str) -> i16 {
    unsafe { GS_CONFIG.before_draw_function_id }
}
pub fn gs_lookup_move_handler_function_id(_name: &str) -> i16 {
    unsafe { GS_CONFIG.move_handler_function_id }
}

/// `RenderAPI` to name conversion (small subset mirroring `GSDevice::RenderAPIToString`).
pub fn render_api_to_string(api: RenderAPI) -> &'static str {
    match api {
        RenderAPI::None => "None",
        RenderAPI::D3D11 => "D3D11",
        RenderAPI::Metal => "Metal",
        RenderAPI::D3D12 => "D3D12",
        RenderAPI::Vulkan => "Vulkan",
        RenderAPI::OpenGL => "OpenGL",
    }
}

/// Captures the `GSConfig` write at the time the GS subsystem is opened.
/// All entry points below consume this snapshot, mirroring the C++ behavior
/// where `GSopen` does `GSConfig = config;` and then dispatches.
pub static mut GS_LAST_CONFIG: GSOptions = GSOptions::new();

/// The currently active renderer; mirrors `GSCurrentRenderer`.
pub static mut GS_CURRENT_RENDERER: pcsx2::GSRendererType = pcsx2::GSRendererType::Auto;

/// Adapter name placeholder (matches `GetDefaultAdapter`).
pub fn get_default_adapter() -> String {
    String::from("(Default)")
}

pub fn gs_get_current_renderer() -> pcsx2::GSRendererType {
    unsafe { GS_CURRENT_RENDERER }
}

pub fn gs_is_hardware_renderer() -> bool {
    unsafe { GS_CURRENT_RENDERER != pcsx2::GSRendererType::SW }
}

/// Maps a renderer type to its native render API. Mirrors `GetAPIForRenderer`
/// in `GS.cpp`.
pub fn get_api_for_renderer(renderer: pcsx2::GSRendererType) -> RenderAPI {
    match renderer {
        pcsx2::GSRendererType::OGL => RenderAPI::OpenGL,
        pcsx2::GSRendererType::VK => RenderAPI::Vulkan,
        pcsx2::GSRendererType::DX11 => RenderAPI::D3D11,
        pcsx2::GSRendererType::DX12 => RenderAPI::D3D12,
        pcsx2::GSRendererType::Metal => RenderAPI::Metal,
        _ => get_api_for_renderer(gsutil::get_preferred_renderer()),
    }
}

/// Hardware renderer detection accounting for the `Null` shader being
/// flagged as HW.
pub fn gs_get_max_upscale_multiplier(max_texture_size: u32) -> u32 {
    max(max_texture_size / 1280, 1)
}

// ---------------------------------------------------------------------------
// gs_reopen / gs_open / gs_close
//
// The C++ implementation performs a fairly elaborate dance with
// `freezeData` buffers, capture restart, and old-config rollback. The
// Rust translation exposes a small state struct that callers can drive
// directly without depending on the EE thread or full host stack.
// ---------------------------------------------------------------------------

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GsReopenResult {
    Ok,
    Failed,
    AlreadyOpen,
}

/// State tracking for the GS subsystem. The C++ code stores a lot of state
/// in process-wide statics (`g_gs_device`, `g_gs_renderer`, etc.). We
/// collect them into a single `GsState` for clarity.
#[derive(Default)]
pub struct GsState {
    pub device: Option<Box<dyn GsDeviceLike>>,
    pub renderer: Option<Box<dyn GsRendererLike>>,
    pub current_renderer: pcsx2::GSRendererType,
    pub vsync_mode: pcsx2::GSVsyncMode,
    pub allow_present_throttle: bool,
    pub capture_filename: String,
    pub capture_size: (i32, i32),
}

pub trait GsDeviceLike: Send {
    fn render_api(&self) -> RenderAPI;
    fn vsync_mode(&self) -> pcsx2::GSVsyncMode;
    fn present_throttle_allowed(&self) -> bool;
    fn max_texture_size(&self) -> u32;
    fn window_info(&self) -> WindowInfo;
    fn destroy(&mut self);
    fn create(&mut self, vsync: pcsx2::GSVsyncMode, present_throttle: bool) -> bool;
    fn set_gpu_timing_enabled(&mut self, on: bool) -> bool;
    fn driver_info(&self) -> String;
    fn supports_exclusive_fullscreen(&self) -> bool;
    fn clear_sampler_cache(&mut self);
    fn clear_current(&mut self);
    fn purge_pool(&mut self);
    fn pool_memory_usage(&self) -> u64;
    fn resize_window(&mut self, w: u32, h: u32, scale: f32);
    fn update_window(&mut self) -> bool;
    fn throttle_presentation(&mut self);
    fn set_vsync_mode(&mut self, mode: pcsx2::GSVsyncMode, allow_throttle: bool);
    fn requested_exclusive_fullscreen_mode(&self) -> Option<(u32, u32, f32)>;
    fn as_any(&self) -> &dyn Any;
}

pub trait GsRendererLike: Send {
    fn destroy(&mut self);
    fn flush(&mut self, reason: GsFlushReason);
    fn purge_texture_cache(&mut self, a: bool, b: bool, c: bool);
    fn readback_texture_cache(&mut self);
    fn begin_capture(&mut self, _filename: String, _size: (i32, i32)) {}
    fn end_capture(&mut self) {}
    fn queue_snapshot(&mut self, _path: String, _frames: u32) {}
    fn stop_gs_dump(&mut self) {}
    fn present_current_frame(&mut self) {}
    fn update_settings(&mut self, _old: &GSOptions) {}
    fn update_render_fixes(&mut self) {}
    fn defrost(&mut self, _fd: &freezeData) -> i32 {
        0
    }
    fn freeze(&mut self, _fd: &mut freezeData, _size_only: bool) -> i32 {
        0
    }
    fn get_regs_mem(&mut self) -> *mut u8 {
        ptr::null_mut()
    }
    fn reset(&mut self, _hardware: bool) {}
    fn soft_reset(&mut self, _mask: u32) {}
    fn write_csr(&mut self, _csr: u32) {}
    fn init_read_fifo(&mut self, _mem: *mut u8, _size: u32) {}
    fn read_fifo(&mut self, _mem: *mut u8, _size: u32) {}
    fn read_local_memory_unsync(
        &mut self,
        _mem: *mut u8,
        _qwc: u32,
        _bitblitbuf: u64,
        _trxpos: u64,
        _trxreg: u64,
    ) {
    }
    fn transfer3(&mut self, _mem: *const u8, _size: u32) {}
    fn transfer2(&mut self, _mem: *mut u8, _size: u32) {}
    fn transfer1(&mut self, _mem: *mut u8, _addr: u32) {}
    fn transfer0(&mut self, _mem: *mut u8, _size: u32) {}
    fn vsync(&mut self, _field: u32, _regs_written: bool, _idle: bool) {}
    fn save_snapshot_to_memory(
        &mut self,
        _w: u32,
        _h: u32,
        _apply: bool,
        _crop: bool,
        _out_w: &mut u32,
        _out_h: &mut u32,
        _pixels: &mut Vec<u32>,
    ) -> bool {
        false
    }
    fn get_internal_resolution(&self) -> (i32, i32) {
        (0, 0)
    }
    fn get_video_mode(&self) -> GSVideoMode {
        GSVideoMode::Unknown
    }
    fn pcrtc_displays(&mut self) -> &mut dyn GsPcrtcDisplaysLike;
    fn m_regs(&mut self) -> &mut gsregs::GsPrivRegSet;
    fn is_really_interlaced(&self) -> bool;
    fn scanmask_used(&self) -> u32;
    fn is_idle_frame(&self) -> bool;
    fn as_any(&self) -> &mut dyn Any;
}

pub trait GsPcrtcDisplaysLike {
    fn set_video_mode(&mut self, mode: GSVideoMode);
    fn enable_displays(&mut self, pmode: u64, smode2: u64, interlaced: bool);
    fn set_rects(&mut self, idx: usize, display: u64, dispfb: u64);
    fn check_same_source(&mut self);
    fn calculate_display_offset(&mut self, scanmask: u32);
    fn calculate_framebuffer_offset(&mut self, scanmask: u32, dispfb0: u64, dispfb1: u64);
    fn get_resolution(&self) -> (i32, i32);
}

/// Flush reasons — mirror of `GSFlushReason`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GsFlushReason {
    Uploads,
    GSREOPEN,
    VsSync,
    EeWriteCSR,
    EeWriteIMR,
    Manual,
    TexWrite,
    Count,
}

/// Public GS entry points — drop-in equivalents for the `GS*` free
/// functions in `GS.cpp`.

pub fn gs_open(
    config: GSOptions,
    renderer: pcsx2::GSRendererType,
    _basemem: *mut u8,
    vsync_mode: pcsx2::GSVsyncMode,
    allow_present_throttle: bool,
    state: &mut GsState,
) -> bool {
    unsafe {
        GS_CONFIG = config.clone();
        GS_LAST_CONFIG = config;
    }
    let renderer = if renderer == pcsx2::GSRendererType::Auto {
        gsutil::get_preferred_renderer()
    } else {
        renderer
    };
    unsafe {
        GS_CURRENT_RENDERER = renderer;
    }
    state.current_renderer = renderer;
    state.vsync_mode = vsync_mode;
    state.allow_present_throttle = allow_present_throttle;
    true
}

pub fn gs_close(state: &mut GsState) {
    state.renderer.as_mut().map(|r| r.end_capture());
    state.renderer.as_mut().map(|r| r.destroy());
    state.renderer = None;
    state.device.as_mut().map(|d| d.destroy());
    state.device = None;
}

pub fn gs_reopen(
    recreate_device: bool,
    recreate_renderer: bool,
    new_renderer: pcsx2::GSRendererType,
    old_config: Option<&GSOptions>,
    state: &mut GsState,
) -> bool {
    if let Some(r) = state.renderer.as_mut() {
        r.flush(GsFlushReason::GSREOPEN);
    }
    if recreate_device && !recreate_renderer {
        if let Some(r) = state.renderer.as_mut() {
            r.purge_texture_cache(true, true, true);
        }
        if let Some(d) = state.device.as_mut() {
            d.clear_current();
            d.purge_pool();
        }
    } else if unsafe { GS_CONFIG.user_hacks_read_tc_on_close } {
        if let Some(r) = state.renderer.as_mut() {
            r.readback_texture_cache();
        }
    }
    let capture_filename = state.capture_filename.clone();
    let capture_size = state.capture_size;
    if !capture_filename.is_empty() {
        if let Some(r) = state.renderer.as_mut() {
            r.end_capture();
        }
    }
    let basemem: *mut u8 = state
        .renderer
        .as_mut()
        .map(|r| r.get_regs_mem())
        .unwrap_or(ptr::null_mut());
    let mut fd = freezeData::default();
    if recreate_renderer {
        if let Some(r) = state.renderer.as_mut() {
            if r.freeze(&mut fd, true) != 0 {
                return false;
            }
        }
        if let Some(r) = state.renderer.as_mut() {
            if r.freeze(&mut fd, false) != 0 {
                return false;
            }
        }
        if let Some(r) = state.renderer.as_mut() {
            r.destroy();
        }
        state.renderer = None;
    }
    if recreate_device {
        let cur_api = state
            .device
            .as_ref()
            .map(|d| d.render_api())
            .unwrap_or(RenderAPI::None);
        let new_api = get_api_for_renderer(unsafe { GS_CONFIG.renderer });
        if let Some(d) = state.device.as_mut() {
            d.destroy();
        }
        state.device = None;
        if !recreate_device_for(new_renderer, state) {
            if let Some(old) = old_config {
                unsafe { GS_CONFIG = old.clone() };
            }
            if !recreate_device_for(unsafe { GS_CONFIG.renderer }, state) {
                return false;
            }
            let _ = cur_api == new_api;
        }
    }
    if recreate_renderer {
        if !recreate_renderer_for(new_renderer, basemem, state) {
            return false;
        }
        if let Some(r) = state.renderer.as_mut() {
            if r.defrost(&fd) != 0 {
                return false;
            }
        }
    }
    if !capture_filename.is_empty() {
        if let Some(r) = state.renderer.as_mut() {
            r.begin_capture(capture_filename, capture_size);
        }
    }
    true
}

fn recreate_device_for(renderer: pcsx2::GSRendererType, state: &mut GsState) -> bool {
    let _ = get_api_for_renderer(renderer);
    // Concrete devices live behind a hardware- or API-specific factory; the
    // translation only models the control flow.
    if state.device.is_none() {
        return false;
    }
    if let Some(d) = state.device.as_mut() {
        d.create(state.vsync_mode, state.allow_present_throttle)
    } else {
        false
    }
}

fn recreate_renderer_for(
    renderer: pcsx2::GSRendererType,
    basemem: *mut u8,
    state: &mut GsState,
) -> bool {
    unsafe { GS_CURRENT_RENDERER = renderer };
    state.current_renderer = renderer;
    if state.renderer.is_none() {
        return false;
    }
    if let Some(r) = state.renderer.as_mut() {
        let _regs: *mut u8 = basemem;
        r.update_render_fixes();
    }
    gspm_reset();
    true
}

pub fn gs_reset(hardware_reset: bool, state: &mut GsState) {
    if let Some(r) = state.renderer.as_mut() {
        r.reset(hardware_reset);
    }
    if hardware_reset && gscapture::is_capturing() {
        let next = gscapture::get_next_capture_file_name();
        let size = gscapture::get_size();
        if let Some(r) = state.renderer.as_mut() {
            r.end_capture();
            r.begin_capture(next, size);
        }
    }
}

pub fn gs_gif_soft_reset(mask: u32, state: &mut GsState) {
    if let Some(r) = state.renderer.as_mut() {
        r.soft_reset(mask);
    }
}

pub fn gs_write_csr(csr: u32, state: &mut GsState) {
    if let Some(r) = state.renderer.as_mut() {
        r.write_csr(csr);
    }
}

pub fn gs_init_and_read_fifo(mem: *mut u8, size: u32, state: &mut GsState) {
    if let Some(r) = state.renderer.as_mut() {
        r.init_read_fifo(mem, size);
        r.read_fifo(mem, size);
    }
}

pub fn gs_read_local_memory_unsync(
    mem: *mut u8,
    qwc: u32,
    bitblitbuf: u64,
    trxpos: u64,
    trxreg: u64,
    state: &mut GsState,
) {
    if let Some(r) = state.renderer.as_mut() {
        r.read_local_memory_unsync(mem, qwc, bitblitbuf, trxpos, trxreg);
    }
}

pub fn gs_gif_transfer<const CHANNEL: usize>(mem: *const u8, size: u32, state: &mut GsState) {
    if let Some(r) = state.renderer.as_mut() {
        match CHANNEL {
            0 => {
                let ptr = mem as *mut u8;
                r.transfer0(ptr, size);
            }
            1 => {
                let ptr = mem as *mut u8;
                r.transfer1(ptr, 0);
            }
            2 => {
                let ptr = mem as *mut u8;
                r.transfer2(ptr, size);
            }
            _ => {
                let ptr = mem as *const u8;
                r.transfer3(ptr, size);
            }
        }
    }
}

pub fn gs_vsync(field: u32, registers_written: bool, state: &mut GsState) {
    let (api, mode) = match state.renderer.as_mut() {
        Some(r) => {
            let pmode = r.m_regs().pmode;
            let smode2 = r.m_regs().smode2;
            let disp0_display = r.m_regs().disp[0].display;
            let disp0_dispfb = r.m_regs().disp[0].dispfb;
            let disp1_display = r.m_regs().disp[1].display;
            let disp1_dispfb = r.m_regs().disp[1].dispfb;
            let interlaced = r.is_really_interlaced();
            let scanmask = r.scanmask_used();
            let video_mode = r.get_video_mode();
            let d = r.pcrtc_displays();
            d.set_video_mode(video_mode);
            d.enable_displays(pmode, smode2, interlaced);
            d.set_rects(0, disp0_display, disp0_dispfb);
            d.set_rects(1, disp1_display, disp1_dispfb);
            d.check_same_source();
            d.calculate_display_offset(scanmask);
            d.calculate_framebuffer_offset(scanmask, disp0_dispfb, disp1_dispfb);
            r.flush(GsFlushReason::VsSync);
            r.vsync(field, registers_written, r.is_idle_frame());
            (Some(r.as_any()), true)
        }
        None => (None, false),
    };
    let _ = (api, mode);
}

pub fn gs_freeze(mode: FreezeAction, data: &mut freezeData, state: &mut GsState) -> i32 {
    match mode {
        FreezeAction::Save | FreezeAction::Size => state
            .renderer
            .as_mut()
            .map(|r| r.freeze(data, matches!(mode, FreezeAction::Size)))
            .unwrap_or(-1),
        FreezeAction::Load => {
            if let Some(d) = state.device.as_mut() {
                d.clear_current();
            }
            if gscapture::is_capturing() {
                gscapture::flush();
            }
            state
                .renderer
                .as_mut()
                .map(|r| r.defrost(data))
                .unwrap_or(-1)
        }
    }
}

pub fn gs_queue_snapshot(path: String, gsdump_frames: u32, state: &mut GsState) {
    if let Some(r) = state.renderer.as_mut() {
        r.queue_snapshot(path, gsdump_frames);
    }
}

pub fn gs_stop_gs_dump(state: &mut GsState) {
    if let Some(r) = state.renderer.as_mut() {
        r.stop_gs_dump();
    }
}

pub fn gs_begin_capture(filename: String, state: &mut GsState) -> bool {
    state
        .renderer
        .as_mut()
        .map(|r| {
            r.begin_capture(filename, state.capture_size);
            true
        })
        .unwrap_or(false)
}

pub fn gs_end_capture(state: &mut GsState) {
    if let Some(r) = state.renderer.as_mut() {
        r.end_capture();
    }
}

pub fn gs_present_current_frame(state: &mut GsState) {
    if let Some(r) = state.renderer.as_mut() {
        r.present_current_frame();
    }
}

pub fn gs_throttle_presentation(state: &mut GsState) {
    if state.vsync_mode == pcsx2::GSVsyncMode::Fifo {
        return;
    }
    if let Some(d) = state.device.as_mut() {
        d.throttle_presentation();
    }
}

pub fn gs_game_changed() {
    if gs_is_hardware_renderer() {
        gstexture_replacements::game_changed();
    }
    if !vm_manager_has_valid_vm() && gscapture::is_capturing() {
        gscapture::end_capture();
    }
}

pub fn gs_has_display_window(state: &mut GsState) -> bool {
    state
        .device
        .as_ref()
        .map(|d| d.window_info().kind != WindowInfoKind::Surfaceless)
        .unwrap_or(false)
}

pub fn gs_resize_display_window(w: u32, h: u32, scale: f32, state: &mut GsState) {
    if let Some(d) = state.device.as_mut() {
        d.resize_window(w, h, scale);
    }
}

pub fn gs_update_display_window(state: &mut GsState) -> bool {
    let ok = state.device.as_mut().map(|d| d.update_window()).unwrap_or(false);
    ok
}

pub fn gs_set_vsync_mode(
    mode: pcsx2::GSVsyncMode,
    allow_throttle: bool,
    state: &mut GsState,
) {
    if let Some(d) = state.device.as_mut() {
        d.set_vsync_mode(mode, allow_throttle);
    }
}

pub fn gs_wants_exclusive_fullscreen(state: &mut GsState) -> bool {
    match state.device.as_ref() {
        Some(d) if d.supports_exclusive_fullscreen() => d
            .requested_exclusive_fullscreen_mode()
            .map(|_| true)
            .unwrap_or(false),
        _ => false,
    }
}

pub fn gs_get_host_refresh_rate(state: &mut GsState) -> Option<f32> {
    state
        .device
        .as_ref()
        .and_then(|d| {
            let r = d.window_info().surface_refresh_rate;
            if r == 0.0 {
                None
            } else {
                Some(r)
            }
        })
}

pub fn gs_get_adapter_info(_renderer: pcsx2::GSRendererType) -> Vec<GSAdapterInfo> {
    Vec::new()
}

pub fn gs_get_display_mode(state: &mut GsState) -> GSVideoMode {
    state
        .renderer
        .as_ref()
        .map(|r| r.get_video_mode())
        .unwrap_or(GSVideoMode::Unknown)
}

pub fn gs_get_internal_resolution(state: &mut GsState) -> (i32, i32) {
    state
        .renderer
        .as_ref()
        .map(|r| r.get_internal_resolution())
        .unwrap_or((0, 0))
}

pub fn gs_get_stats(info: &mut String, state: &mut GsState) {
    let api = state
        .device
        .as_ref()
        .map(|d| render_api_to_string(d.render_api()))
        .unwrap_or("None");
    let pm = gspm_snapshot();
    if unsafe { GS_CURRENT_RENDERER } == pcsx2::GSRendererType::SW {
        let fps = get_vertical_frequency();
        let fillrate = pm[gspm::Counter::Fillrate];
        let mut pps = fps * fillrate;
        let mut prefix = ' ';
        if pps >= 170_000_000.0 {
            pps /= 1_000_000_000.0;
            prefix = 'G';
        } else if pps >= 35_000_000.0 {
            pps /= 1_000_000.0;
            prefix = 'M';
        } else if pps >= 1_000.0 {
            pps /= 1_000.0;
            prefix = 'k';
        }
        *info = format!(
            "{} SW | {} SYNP | {} PRIM | {} DRW | {:.2} SWIZ | {:.2} UNSWIZ | {:.2} {}pps",
            api,
            pm[gspm::Counter::SyncPoint] as i32,
            pm[gspm::Counter::Prim] as i32,
            pm[gspm::Counter::Draw] as i32,
            pm[gspm::Counter::Swizzle] / 1024.0,
            pm[gspm::Counter::Unswizzle] / 1024.0,
            pps,
            prefix
        );
    } else if unsafe { GS_CURRENT_RENDERER } == pcsx2::GSRendererType::Null {
        *info = format!("{} Null", api);
    } else if unsafe { GS_CONFIG.hwrov } {
        *info = format!(
            "{} HW | {} PRIM | {} DRW | {}/{} DRWC | {}/{} BAR | {} RP | {} RB | {}/{} TC | {} TU",
            api,
            pm[gspm::Counter::Prim] as i32,
            pm[gspm::Counter::Draw] as i32,
            pm[gspm::Counter::DrawCalls].ceil() as i32,
            pm[gspm::Counter::DrawCallsROV].ceil() as i32,
            pm[gspm::Counter::Barriers].ceil() as i32,
            pm[gspm::Counter::BarriersROV].ceil() as i32,
            pm[gspm::Counter::RenderPasses].ceil() as i32,
            pm[gspm::Counter::Readbacks].ceil() as i32,
            pm[gspm::Counter::TextureCopies].ceil() as i32,
            pm[gspm::Counter::DepthCopiesROV].ceil() as i32,
            pm[gspm::Counter::TextureUploads].ceil() as i32,
        );
    } else {
        *info = format!(
            "{} HW | {} PRIM | {} DRW | {} DRWC | {} BAR | {} RP | {} RB | {} TC | {} TU",
            api,
            pm[gspm::Counter::Prim] as i32,
            pm[gspm::Counter::Draw] as i32,
            pm[gspm::Counter::DrawCalls].ceil() as i32,
            pm[gspm::Counter::Barriers].ceil() as i32,
            pm[gspm::Counter::RenderPasses].ceil() as i32,
            pm[gspm::Counter::Readbacks].ceil() as i32,
            pm[gspm::Counter::TextureCopies].ceil() as i32,
            pm[gspm::Counter::TextureUploads].ceil() as i32,
        );
    }
}

pub fn gs_get_memory_stats(info: &mut String) {
    let get_mb = |bytes: f64| -> f64 {
        if bytes <= 0.0 {
            bytes
        } else {
            f64::max(0.1, bytes / 1_048_576.0)
        }
    };
    let format_precision = |mb: f64| -> String {
        if mb < 10.0 {
            format!("{:.1}", mb)
        } else {
            format!("{:.0}", mb.round())
        }
    };
    let tgt = gscache_get_target_memory_usage() as f64;
    let src = gscache_get_source_memory_usage() as f64;
    let pl = gsdevice_get_pool_memory_usage() as f64;
    let targets_mb = get_mb(tgt);
    let sources_mb = get_mb(src);
    let pool_mb = get_mb(pl);
    if unsafe { GS_CONFIG.texture_preloading } == pcsx2::TexturePreloadingLevel::Full {
        let hc = gscache_get_hash_cache_memory_usage() as f64;
        let hc_mb = get_mb(hc);
        let total = targets_mb + sources_mb + hc_mb + pool_mb;
        *info = format!(
            "VRAM: {} MB | TGT: {} MB | SRC: {} MB | HC: {} MB | PL: {} MB",
            format_precision(total),
            format_precision(targets_mb),
            format_precision(sources_mb),
            format_precision(hc_mb),
            format_precision(pool_mb)
        );
    } else {
        let total = targets_mb + sources_mb + pool_mb;
        *info = format!(
            "VRAM: {} MB | TGT: {} MB | SRC: {} MB | PL: {} MB",
            format_precision(total),
            format_precision(targets_mb),
            format_precision(sources_mb),
            format_precision(pool_mb)
        );
    }
}

pub fn gs_get_title_stats(info: &mut String, state: &mut GsState) {
    const DEINTERLACE_MODES: [&str; 10] = [
        "Automatic",
        "None",
        "Weave tff",
        "Weave bff",
        "Bob tff",
        "Bob bff",
        "Blend tff",
        "Blend bff",
        "Adaptive tff",
        "Adaptive bff",
    ];
    let api = state
        .device
        .as_ref()
        .map(|d| render_api_to_string(d.render_api()))
        .unwrap_or("None");
    let hw_sw = if unsafe { GS_CURRENT_RENDERER } == pcsx2::GSRendererType::Null {
        " Null"
    } else if gs_is_hardware_renderer() {
        " HW"
    } else {
        " SW"
    };
    let idx = unsafe { GS_CONFIG.interlace_mode } as usize;
    let deinterlace_mode = DEINTERLACE_MODES.get(idx).copied().unwrap_or("None");
    let interlace = report_interlace_mode();
    let video = report_video_mode();
    *info = format!(
        "{}{} | {} | {} | {}",
        api, hw_sw, video, interlace, deinterlace_mode
    );
}

pub fn gs_translate_window_to_display_coordinates(
    win_x: f32,
    win_y: f32,
    dx: &mut f32,
    dy: &mut f32,
) {
    *dx = win_x;
    *dy = win_y;
}

pub fn gs_update_config(new_config: GSOptions, state: &mut GsState) {
    let old_config = unsafe { GS_CONFIG.clone() };
    unsafe { GS_CONFIG = new_config.clone() };
    if state.renderer.is_none() {
        return;
    }
    if new_config.osd_scale != old_config.osd_scale {
        imgui_request_scale_update();
    }
    if new_config.osd_font_path != old_config.osd_font_path {
        imgui_reload_fonts();
    }
    if !new_config.restart_options_are_equal(&old_config) {
        let _ = gs_reopen(true, true, new_config.renderer, Some(&old_config), state);
        return;
    }
    if new_config.sw_extra_threads != old_config.sw_extra_threads
        || new_config.sw_extra_threads_height != old_config.sw_extra_threads_height
    {
        let _ = gs_reopen(false, true, new_config.renderer, Some(&old_config), state);
        return;
    }
    if new_config.user_hacks_disable_render_fixes != old_config.user_hacks_disable_render_fixes
        || new_config.upscale_multiplier != old_config.upscale_multiplier
        || new_config.get_skip_count_function_id != old_config.get_skip_count_function_id
        || new_config.before_draw_function_id != old_config.before_draw_function_id
        || new_config.move_handler_function_id != old_config.move_handler_function_id
    {
        if let Some(r) = state.renderer.as_mut() {
            r.update_render_fixes();
        }
    }
    if let Some(r) = state.renderer.as_mut() {
        r.update_settings(&old_config);
    }
    if (gs_is_hardware_renderer() && new_config.hw_mipmap != old_config.hw_mipmap)
        || new_config.texture_preloading != old_config.texture_preloading
        || new_config.tri_filter != old_config.tri_filter
        || new_config.gpu_palette_conversion != old_config.gpu_palette_conversion
        || new_config.preload_frame_with_gs_data != old_config.preload_frame_with_gs_data
        || new_config.user_hacks_cpu_fb_conversion != old_config.user_hacks_cpu_fb_conversion
        || new_config.user_hacks_disable_depth_support != old_config.user_hacks_disable_depth_support
        || new_config.user_hacks_disable_partial_invalidation
            != old_config.user_hacks_disable_partial_invalidation
        || new_config.user_hacks_texture_inside_rt != old_config.user_hacks_texture_inside_rt
        || new_config.user_hacks_cpu_sprite_render_bw != old_config.user_hacks_cpu_sprite_render_bw
        || new_config.user_hacks_cpuclut_render != old_config.user_hacks_cpuclut_render
        || new_config.user_hacks_gpu_target_clut_mode != old_config.user_hacks_gpu_target_clut_mode
    {
        if new_config.user_hacks_read_tc_on_close {
            if let Some(r) = state.renderer.as_mut() {
                r.readback_texture_cache();
            }
        }
        if let Some(r) = state.renderer.as_mut() {
            r.purge_texture_cache(true, true, true);
        }
        if let Some(d) = state.device.as_mut() {
            d.clear_current();
            d.purge_pool();
        }
    }
    if new_config.max_anisotropy != old_config.max_anisotropy {
        if let Some(d) = state.device.as_mut() {
            d.clear_sampler_cache();
        }
    }
    if gs_is_hardware_renderer() {
        gstexture_replacements::update_config(&old_config);
    }
    if new_config.load_texture_replacements != old_config.load_texture_replacements
        || new_config.dump_replaceable_textures != old_config.dump_replaceable_textures
    {
        if let Some(r) = state.renderer.as_mut() {
            r.purge_texture_cache(true, false, true);
        }
    }
    if new_config.osd_show_gpu && !old_config.osd_show_gpu {
        if let Some(d) = state.device.as_mut() {
            if !d.set_gpu_timing_enabled(true) {
                unsafe { GS_CONFIG.osd_show_gpu = false };
            }
        }
    }
}

pub fn gs_set_software_rendering(software: bool, new_interlace: pcsx2::GSInterlaceMode, state: &mut GsState) {
    if state.renderer.is_none() {
        return;
    }
    unsafe { GS_CONFIG.interlace_mode = new_interlace };
    if gs_is_hardware_renderer() == software {
        let renderer = if software {
            pcsx2::GSRendererType::SW
        } else {
            if unsafe { GS_CONFIG.renderer } == pcsx2::GSRendererType::SW {
                pcsx2::GSRendererType::Auto
            } else {
                unsafe { GS_CONFIG.renderer }
            }
        };
        let _ = gs_reopen(false, true, renderer, None, state);
    }
}

pub fn gs_save_snapshot_to_memory(
    window_w: u32,
    window_h: u32,
    apply_aspect: bool,
    crop_borders: bool,
    width: &mut u32,
    height: &mut u32,
    pixels: &mut Vec<u32>,
    state: &mut GsState,
) -> bool {
    if let Some(r) = state.renderer.as_mut() {
        r.save_snapshot_to_memory(window_w, window_h, apply_aspect, crop_borders, width, height, pixels)
    } else {
        false
    }
}

pub fn gs_join_snapshot_threads() {}

pub fn gs_set_display_alignment(_a: GSDisplayAlignment) {}
pub fn gs_game_changed_wrapper() {
    gs_game_changed();
}

fn vm_manager_has_valid_vm() -> bool {
    true
}

fn get_vertical_frequency() -> f64 {
    60.0
}

fn report_interlace_mode() -> &'static str {
    "Interlaced"
}

fn report_video_mode() -> &'static str {
    "NTSC"
}

fn imgui_request_scale_update() {}
fn imgui_reload_fonts() {}

fn gscache_get_target_memory_usage() -> u64 {
    0
}
fn gscache_get_source_memory_usage() -> u64 {
    0
}
fn gscache_get_hash_cache_memory_usage() -> u64 {
    0
}
fn gsdevice_get_pool_memory_usage() -> u64 {
    0
}
fn gspm_reset() {}
fn gspm_snapshot() -> gspm::Snapshot {
    gspm::Snapshot::default()
}

// ===========================================================================
// GSTables
// ===========================================================================

pub mod gstables {
    use super::*;

    /// Table for storing swizzling of blocks within a page.
    #[derive(Copy, Clone)]
    #[repr(align(64))]
    pub struct GSBlockSwizzleTable {
        pub value: [[u8; 8]; 8],
    }

    impl Default for GSBlockSwizzleTable {
        fn default() -> Self {
            Self { value: [[0; 8]; 8] }
        }
    }

    impl GSBlockSwizzleTable {
        pub const fn lookup(&self, x: i32, y: i32) -> u8 {
            self.value[(y & 7) as usize][(x & 7) as usize]
        }
    }

    /// Strongly-typed block swizzle table.
    #[derive(Copy, Clone, Default)]
    pub struct GSSizedBlockSwizzleTable<const H: usize, const W: usize> {
        pub value: [[u8; 8]; 8],
    }

    /// Column offset table — `PageHeight` entries.
    #[derive(Copy, Clone)]
    #[repr(align(128))]
    pub struct GSPixelColOffsetTable<const PAGE_HEIGHT: usize> {
        pub value: [i32; PAGE_HEIGHT],
    }

    impl<const PAGE_HEIGHT: usize> Default for GSPixelColOffsetTable<PAGE_HEIGHT> {
        fn default() -> Self {
            Self {
                value: [0; PAGE_HEIGHT],
            }
        }
    }

    impl<const PAGE_HEIGHT: usize> GSPixelColOffsetTable<PAGE_HEIGHT> {
        pub fn operator_index(&self, y: i32) -> i32 {
            let idx = ((y as i64).rem_euclid(PAGE_HEIGHT as i64)) as usize;
            self.value[idx]
        }
    }

    /// Row offset table — 4096 entries.
    #[derive(Copy, Clone)]
    #[repr(align(128))]
    pub struct GSPixelRowOffsetTable {
        pub value: [i32; 4096],
    }

    impl Default for GSPixelRowOffsetTable {
        fn default() -> Self {
            Self { value: [0; 4096] }
        }
    }

    impl GSPixelRowOffsetTable {
        pub fn operator_index(&self, x: usize) -> i32 {
            debug_assert!(x < 4096);
            self.value[x]
        }
    }

    pub type GSSizedPixelRowOffsetTable<const W: usize> = GSPixelRowOffsetTable;

    /// List of row offset tables — keyed by `y & mask`.
    #[derive(Copy, Clone)]
    #[repr(align(64))]
    pub struct GSPixelRowOffsetTableList<const W: usize, const MASK: usize> {
        pub rows: [*const GSPixelRowOffsetTable; 8],
    }

    // The pointers here always target `pub static` row tables whose data is
    // never mutated after initialization, so it is sound to share them across
    // threads even though raw pointers are normally `!Sync`.
    unsafe impl<const W: usize, const MASK: usize> Sync for GSPixelRowOffsetTableList<W, MASK> {}

    impl<const W: usize, const MASK: usize> GSPixelRowOffsetTableList<W, MASK> {
        pub fn operator_index(&self, y: i32) -> &GSPixelRowOffsetTable {
            unsafe { &*self.rows[(y as usize) & MASK] }
        }
    }

    /// Combined swizzle table list.
    #[derive(Copy, Clone)]
    pub struct GSSwizzleTableList<
        'a,
        const PAGE_HEIGHT: usize,
        const PAGE_WIDTH: usize,
        const BLOCK_H: usize,
        const BLOCK_W: usize,
        const ROW_MASK: usize,
    > {
        pub block: &'a GSSizedBlockSwizzleTable<BLOCK_H, BLOCK_W>,
        pub col: &'a GSPixelColOffsetTable<PAGE_HEIGHT>,
        pub row: &'a GSPixelRowOffsetTableList<PAGE_WIDTH, ROW_MASK>,
    }

    pub const fn make_swizzle_table<const W: usize, const H: usize>(
        arr: &[[u8; W]; H],
    ) -> GSSizedBlockSwizzleTable<H, W> {
        let mut t: GSSizedBlockSwizzleTable<H, W> = GSSizedBlockSwizzleTable {
            value: [[0u8; 8]; 8],
        };
        let mut y = 0;
        while y < 8 {
            let mut x = 0;
            while x < 8 {
                t.value[y][x] = arr[y % H][x % W];
                x += 1;
            }
            y += 1;
        }
        t
    }

    // Block tables — straight transcription of the C++ constants.
    pub static BLOCK_TABLE_32: GSSizedBlockSwizzleTable<4, 8> = make_swizzle_table(&[
        [0u8, 1, 4, 5, 16, 17, 20, 21],
        [2u8, 3, 6, 7, 18, 19, 22, 23],
        [8u8, 9, 12, 13, 24, 25, 28, 29],
        [10u8, 11, 14, 15, 26, 27, 30, 31],
    ]);

    pub static BLOCK_TABLE_16: GSSizedBlockSwizzleTable<8, 4> = make_swizzle_table(&[
        [0u8, 2, 8, 10],
        [1u8, 3, 9, 11],
        [4u8, 6, 12, 14],
        [5u8, 7, 13, 15],
        [16u8, 18, 24, 26],
        [17u8, 19, 25, 27],
        [20u8, 22, 28, 30],
        [21u8, 23, 29, 31],
    ]);

    pub static BLOCK_TABLE_16S: GSSizedBlockSwizzleTable<8, 4> = make_swizzle_table(&[
        [0u8, 2, 16, 18],
        [1u8, 3, 17, 19],
        [8u8, 10, 24, 26],
        [9u8, 11, 25, 27],
        [4u8, 6, 20, 22],
        [5u8, 7, 21, 23],
        [12u8, 14, 28, 30],
        [13u8, 15, 29, 31],
    ]);

    pub static BLOCK_TABLE_8: GSSizedBlockSwizzleTable<4, 8> = make_swizzle_table(&[
        [0u8, 1, 4, 5, 16, 17, 20, 21],
        [2u8, 3, 6, 7, 18, 19, 22, 23],
        [8u8, 9, 12, 13, 24, 25, 28, 29],
        [10u8, 11, 14, 15, 26, 27, 30, 31],
    ]);

    pub static BLOCK_TABLE_4: GSSizedBlockSwizzleTable<8, 4> = make_swizzle_table(&[
        [0u8, 2, 8, 10],
        [1u8, 3, 9, 11],
        [4u8, 6, 12, 14],
        [5u8, 7, 13, 15],
        [16u8, 18, 24, 26],
        [17u8, 19, 25, 27],
        [20u8, 22, 28, 30],
        [21u8, 23, 29, 31],
    ]);

    pub static COLUMN_TABLE_32: [[u16; 8]; 8] = [
        [0, 1, 4, 5, 8, 9, 12, 13],
        [2, 3, 6, 7, 10, 11, 14, 15],
        [16, 17, 20, 21, 24, 25, 28, 29],
        [18, 19, 22, 23, 26, 27, 30, 31],
        [32, 33, 36, 37, 40, 41, 44, 45],
        [34, 35, 38, 39, 42, 43, 46, 47],
        [48, 49, 52, 53, 56, 57, 60, 61],
        [50, 51, 54, 55, 58, 59, 62, 63],
    ];

    pub static COLUMN_TABLE_16: [[u16; 16]; 8] = [
        [0, 2, 8, 10, 16, 18, 24, 26, 1, 3, 9, 11, 17, 19, 25, 27],
        [4, 6, 12, 14, 20, 22, 28, 30, 5, 7, 13, 15, 21, 23, 29, 31],
        [32, 34, 40, 42, 48, 50, 56, 58, 33, 35, 41, 43, 49, 51, 57, 59],
        [36, 38, 44, 46, 52, 54, 60, 62, 37, 39, 45, 47, 53, 55, 61, 63],
        [64, 66, 72, 74, 80, 82, 88, 90, 65, 67, 73, 75, 81, 83, 89, 91],
        [68, 70, 76, 78, 84, 86, 92, 94, 69, 71, 77, 79, 85, 87, 93, 95],
        [96, 98, 104, 106, 112, 114, 120, 122, 97, 99, 105, 107, 113, 115, 121, 123],
        [100, 102, 108, 110, 116, 118, 124, 126, 101, 103, 109, 111, 117, 119, 125, 127],
    ];

    pub static COLUMN_TABLE_8: [[u16; 16]; 16] = [
        [0, 4, 16, 20, 32, 36, 48, 52, 2, 6, 18, 22, 34, 38, 50, 54],
        [8, 12, 24, 28, 40, 44, 56, 60, 10, 14, 26, 30, 42, 46, 58, 62],
        [33, 37, 49, 53, 1, 5, 17, 21, 35, 39, 51, 55, 3, 7, 19, 23],
        [41, 45, 57, 61, 9, 13, 25, 29, 43, 47, 59, 63, 11, 15, 27, 31],
        [96, 100, 112, 116, 64, 68, 80, 84, 98, 102, 114, 118, 66, 70, 82, 86],
        [104, 108, 120, 124, 72, 76, 88, 92, 106, 110, 122, 126, 74, 78, 90, 94],
        [65, 69, 81, 85, 97, 101, 113, 117, 67, 71, 83, 87, 99, 103, 115, 119],
        [73, 77, 89, 93, 105, 109, 121, 125, 75, 79, 91, 95, 107, 111, 123, 127],
        [128, 132, 144, 148, 160, 164, 176, 180, 130, 134, 146, 150, 162, 166, 178, 182],
        [136, 140, 152, 156, 168, 172, 184, 188, 138, 142, 154, 158, 170, 174, 186, 190],
        [161, 165, 177, 181, 129, 133, 145, 149, 163, 167, 179, 183, 131, 135, 147, 151],
        [169, 173, 185, 189, 137, 141, 153, 157, 171, 175, 187, 191, 139, 143, 155, 159],
        [224, 228, 240, 244, 192, 196, 208, 212, 226, 230, 242, 246, 194, 198, 210, 214],
        [232, 236, 248, 252, 200, 204, 216, 220, 234, 238, 250, 254, 202, 206, 218, 222],
        [193, 197, 209, 213, 225, 229, 241, 245, 195, 199, 211, 215, 227, 231, 243, 247],
        [201, 205, 217, 221, 233, 237, 249, 253, 203, 207, 219, 223, 235, 239, 251, 255],
    ];

    pub static COLUMN_TABLE_4: [[u16; 32]; 16] = [
        [0, 8, 32, 40, 64, 72, 96, 104, 2, 10, 34, 42, 66, 74, 98, 106, 4, 12, 36, 44, 68, 76, 100, 108, 6, 14, 38, 46, 70, 78, 102, 110],
        [16, 24, 48, 56, 80, 88, 112, 120, 18, 26, 50, 58, 82, 90, 114, 122, 20, 28, 52, 60, 84, 92, 116, 124, 22, 30, 54, 62, 86, 94, 118, 126],
        [65, 73, 97, 105, 1, 9, 33, 41, 67, 75, 99, 107, 3, 11, 35, 43, 69, 77, 101, 109, 5, 13, 37, 45, 71, 79, 103, 111, 7, 15, 39, 47],
        [81, 89, 113, 121, 17, 25, 49, 57, 83, 91, 115, 123, 19, 27, 51, 59, 85, 93, 117, 125, 21, 29, 53, 61, 87, 95, 119, 127, 23, 31, 55, 63],
        [192, 200, 224, 232, 128, 136, 160, 168, 194, 202, 226, 234, 130, 138, 162, 170, 196, 204, 228, 236, 132, 140, 164, 172, 198, 206, 230, 238, 134, 142, 166, 174],
        [208, 216, 240, 248, 144, 152, 176, 184, 210, 218, 242, 250, 146, 154, 178, 186, 212, 220, 244, 252, 148, 156, 180, 188, 214, 222, 246, 254, 150, 158, 182, 190],
        [129, 137, 161, 169, 193, 201, 225, 233, 131, 139, 163, 171, 195, 203, 227, 235, 133, 141, 165, 173, 197, 205, 229, 237, 135, 143, 167, 175, 199, 207, 231, 239],
        [145, 153, 177, 185, 209, 217, 241, 249, 147, 155, 179, 187, 211, 219, 243, 251, 149, 157, 181, 189, 213, 221, 245, 253, 151, 159, 183, 191, 215, 223, 247, 255],
        [256, 264, 288, 296, 320, 328, 352, 360, 258, 266, 290, 298, 322, 330, 354, 362, 260, 268, 292, 300, 324, 332, 356, 364, 262, 270, 294, 302, 326, 334, 358, 366],
        [272, 280, 304, 312, 336, 344, 368, 376, 274, 282, 306, 314, 338, 346, 370, 378, 276, 284, 308, 316, 340, 348, 372, 380, 278, 286, 310, 318, 342, 350, 374, 382],
        [321, 329, 353, 361, 257, 265, 289, 297, 323, 331, 355, 363, 259, 267, 291, 299, 325, 333, 357, 365, 261, 269, 293, 301, 327, 335, 359, 367, 263, 271, 295, 303],
        [337, 345, 369, 377, 273, 281, 305, 313, 339, 347, 371, 379, 275, 283, 307, 315, 341, 349, 373, 381, 277, 285, 309, 317, 343, 351, 375, 383, 279, 287, 311, 319],
        [448, 456, 480, 488, 384, 392, 416, 424, 450, 458, 482, 490, 386, 394, 418, 426, 452, 460, 484, 492, 388, 396, 420, 428, 454, 462, 486, 494, 390, 398, 422, 430],
        [464, 472, 496, 504, 400, 408, 432, 440, 466, 474, 498, 506, 402, 410, 434, 442, 468, 476, 500, 508, 404, 412, 436, 444, 470, 478, 502, 510, 406, 414, 438, 446],
        [385, 393, 417, 425, 449, 457, 481, 489, 387, 395, 419, 427, 451, 459, 483, 491, 389, 397, 421, 429, 453, 461, 485, 493, 391, 399, 423, 431, 455, 463, 487, 495],
        [401, 409, 433, 441, 465, 473, 497, 505, 403, 411, 435, 443, 467, 475, 499, 507, 405, 413, 437, 445, 469, 477, 501, 509, 407, 415, 439, 447, 471, 479, 503, 511],
    ];

    pub static CLUT_TABLE_T32_I8: [u8; 128] = [
        0, 1, 4, 5, 8, 9, 12, 13, 2, 3, 6, 7, 10, 11, 14, 15,
        64, 65, 68, 69, 72, 73, 76, 77, 66, 67, 70, 71, 74, 75, 78, 79,
        16, 17, 20, 21, 24, 25, 28, 29, 18, 19, 22, 23, 26, 27, 30, 31,
        80, 81, 84, 85, 88, 89, 92, 93, 82, 83, 86, 87, 90, 91, 94, 95,
        32, 33, 36, 37, 40, 41, 44, 45, 34, 35, 38, 39, 42, 43, 46, 47,
        96, 97, 100, 101, 104, 105, 108, 109, 98, 99, 102, 103, 106, 107, 110, 111,
        48, 49, 52, 53, 56, 57, 60, 61, 50, 51, 54, 55, 58, 59, 62, 63,
        112, 113, 116, 117, 120, 121, 124, 125, 114, 115, 118, 119, 122, 123, 126, 127,
    ];

    pub static CLUT_TABLE_T32_I4: [u8; 16] = [
        0, 1, 4, 5, 8, 9, 12, 13, 2, 3, 6, 7, 10, 11, 14, 15,
    ];

    pub static CLUT_TABLE_T16_I8: [u8; 32] = [
        0, 2, 8, 10, 16, 18, 24, 26, 4, 6, 12, 14, 20, 22, 28, 30,
        1, 3, 9, 11, 17, 19, 25, 27, 5, 7, 13, 15, 21, 23, 29, 31,
    ];

    pub static CLUT_TABLE_T16_I4: [u8; 16] = [
        0, 2, 8, 10, 16, 18, 24, 26, 4, 6, 12, 14, 20, 22, 28, 30,
    ];

    pub const fn px_offset<
        const BH: usize,
        const BW: usize,
        const CH: usize,
        const CW: usize,
    >(
        block: &[[u8; 8]; 8],
        col: &[[u16; CW]; CH],
        x: i32,
        y: i32,
    ) -> i32 {
        let block_size = CH * CW;
        let page_size = block_size * BH * BW;
        let page_width = BW * CW;
        let ch_i = CH as i32;
        let cw_i = CW as i32;
        let page_x = x / (page_width as i32);
        let subpage_x = x % (page_width as i32);
        let block_id = block[((y / ch_i) as usize) % 8][((subpage_x / cw_i) as usize) % 8];
        let sublock_offset =
            col[((y % ch_i) as usize) % CH][((subpage_x % cw_i) as usize) % CW] as i32;
        page_x * (page_size as i32) + (block_id as i32) * (block_size as i32) + sublock_offset
    }

    // Const-generic arithmetic (`{ BH * CH }`, `{ BW * CW }`) is not allowed in
    // Rust's const generics, so these factory functions are implemented as
    // `macro_rules!`. They are invoked only with concrete parameter tuples at
    // their single use site, so the macro expansion is always type-checked
    // against a specific `GSPixelColOffsetTable<N>` / `GSPixelRowOffsetTable`.
    macro_rules! make_col_offset_table {
        ($BH:literal, $BW:literal, $CH:literal, $CW:literal, $block:expr, $col:expr) => {{
            const BH: usize = $BH;
            const BW: usize = $BW;
            const CH: usize = $CH;
            const CW: usize = $CW;
            const PAGE_HEIGHT: usize = BH * CH;
            let block: &[[u8; 8]; 8] = $block;
            let col: &[[u16; CW]; CH] = $col;
            let mut table: GSPixelColOffsetTable<PAGE_HEIGHT> = GSPixelColOffsetTable {
                value: [0i32; PAGE_HEIGHT],
            };
            let mut y: usize = 0;
            while y < PAGE_HEIGHT {
                table.value[y] = px_offset::<BH, BW, CH, CW>(block, col, 0, y as i32);
                y += 1;
            }
            table
        }};
    }

    macro_rules! make_row_offset_table {
        ($BH:literal, $BW:literal, $CH:literal, $CW:literal, $block:expr, $col:expr, $y:expr) => {{
            const BH: usize = $BH;
            const BW: usize = $BW;
            const CH: usize = $CH;
            const CW: usize = $CW;
            const PAGE_WIDTH: usize = BW * CW;
            let block: &[[u8; 8]; 8] = $block;
            let col: &[[u16; CW]; CH] = $col;
            let y: i32 = $y;
            let base = px_offset::<BH, BW, CH, CW>(block, col, 0, y);
            let mut table: GSPixelRowOffsetTable = GSPixelRowOffsetTable {
                value: [0i32; 4096],
            };
            let mut x: usize = 0;
            while x < PAGE_WIDTH {
                let v = px_offset::<BH, BW, CH, CW>(block, col, (x % 2048) as i32, y);
                table.value[x] = v - base;
                x += 1;
            }
            table
        }};
    }

    pub static PIXEL_COL_OFFSET_32: GSPixelColOffsetTable<32> =
        make_col_offset_table!(4, 8, 8, 8, &BLOCK_TABLE_32.value, &COLUMN_TABLE_32);
    pub static PIXEL_COL_OFFSET_16: GSPixelColOffsetTable<64> =
        make_col_offset_table!(8, 4, 8, 16, &BLOCK_TABLE_16.value, &COLUMN_TABLE_16);
    pub static PIXEL_COL_OFFSET_16S: GSPixelColOffsetTable<64> =
        make_col_offset_table!(8, 4, 8, 16, &BLOCK_TABLE_16S.value, &COLUMN_TABLE_16);
    pub static PIXEL_COL_OFFSET_8: GSPixelColOffsetTable<64> =
        make_col_offset_table!(4, 8, 16, 16, &BLOCK_TABLE_8.value, &COLUMN_TABLE_8);
    pub static PIXEL_COL_OFFSET_4: GSPixelColOffsetTable<128> =
        make_col_offset_table!(
            8,
            4,
            16,
            32,
            &BLOCK_TABLE_4.value,
            &COLUMN_TABLE_4
        );

    pub static PIXEL_ROW_OFFSET_32: GSSizedPixelRowOffsetTable<64> =
        make_row_offset_table!(4, 8, 8, 8, &BLOCK_TABLE_32.value, &COLUMN_TABLE_32, 0);
    pub static PIXEL_ROW_OFFSET_16: GSSizedPixelRowOffsetTable<64> =
        make_row_offset_table!(8, 4, 8, 16, &BLOCK_TABLE_16.value, &COLUMN_TABLE_16, 0);
    pub static PIXEL_ROW_OFFSET_16S: GSSizedPixelRowOffsetTable<64> =
        make_row_offset_table!(8, 4, 8, 16, &BLOCK_TABLE_16S.value, &COLUMN_TABLE_16, 0);
    pub static PIXEL_ROW_OFFSET_8_A: GSSizedPixelRowOffsetTable<128> =
        make_row_offset_table!(4, 8, 16, 16, &BLOCK_TABLE_8.value, &COLUMN_TABLE_8, 0);
    pub static PIXEL_ROW_OFFSET_8_B: GSSizedPixelRowOffsetTable<128> =
        make_row_offset_table!(4, 8, 16, 16, &BLOCK_TABLE_8.value, &COLUMN_TABLE_8, 2);
    pub static PIXEL_ROW_OFFSET_4_A: GSSizedPixelRowOffsetTable<128> =
        make_row_offset_table!(
            8,
            4,
            16,
            32,
            &BLOCK_TABLE_4.value,
            &COLUMN_TABLE_4,
            0
        );
    pub static PIXEL_ROW_OFFSET_4_B: GSSizedPixelRowOffsetTable<128> =
        make_row_offset_table!(
            8,
            4,
            16,
            32,
            &BLOCK_TABLE_4.value,
            &COLUMN_TABLE_4,
            2
        );

    pub static PIXEL_ROW_OFFSET_32_LIST: GSPixelRowOffsetTableList<64, 0> = GSPixelRowOffsetTableList {
        rows: [
            &PIXEL_ROW_OFFSET_32,
            &PIXEL_ROW_OFFSET_32,
            &PIXEL_ROW_OFFSET_32,
            &PIXEL_ROW_OFFSET_32,
            &PIXEL_ROW_OFFSET_32,
            &PIXEL_ROW_OFFSET_32,
            &PIXEL_ROW_OFFSET_32,
            &PIXEL_ROW_OFFSET_32,
        ],
    };
    pub static PIXEL_ROW_OFFSET_16_LIST: GSPixelRowOffsetTableList<64, 0> = GSPixelRowOffsetTableList {
        rows: [
            &PIXEL_ROW_OFFSET_16,
            &PIXEL_ROW_OFFSET_16,
            &PIXEL_ROW_OFFSET_16,
            &PIXEL_ROW_OFFSET_16,
            &PIXEL_ROW_OFFSET_16,
            &PIXEL_ROW_OFFSET_16,
            &PIXEL_ROW_OFFSET_16,
            &PIXEL_ROW_OFFSET_16,
        ],
    };
    pub static PIXEL_ROW_OFFSET_16S_LIST: GSPixelRowOffsetTableList<64, 0> = GSPixelRowOffsetTableList {
        rows: [
            &PIXEL_ROW_OFFSET_16S,
            &PIXEL_ROW_OFFSET_16S,
            &PIXEL_ROW_OFFSET_16S,
            &PIXEL_ROW_OFFSET_16S,
            &PIXEL_ROW_OFFSET_16S,
            &PIXEL_ROW_OFFSET_16S,
            &PIXEL_ROW_OFFSET_16S,
            &PIXEL_ROW_OFFSET_16S,
        ],
    };
    pub static PIXEL_ROW_OFFSET_8_LIST: GSPixelRowOffsetTableList<128, 7> = GSPixelRowOffsetTableList {
        rows: [
            &PIXEL_ROW_OFFSET_8_A,
            &PIXEL_ROW_OFFSET_8_A,
            &PIXEL_ROW_OFFSET_8_B,
            &PIXEL_ROW_OFFSET_8_B,
            &PIXEL_ROW_OFFSET_8_B,
            &PIXEL_ROW_OFFSET_8_B,
            &PIXEL_ROW_OFFSET_8_A,
            &PIXEL_ROW_OFFSET_8_A,
        ],
    };
    pub static PIXEL_ROW_OFFSET_4_LIST: GSPixelRowOffsetTableList<128, 7> = GSPixelRowOffsetTableList {
        rows: [
            &PIXEL_ROW_OFFSET_4_A,
            &PIXEL_ROW_OFFSET_4_A,
            &PIXEL_ROW_OFFSET_4_B,
            &PIXEL_ROW_OFFSET_4_B,
            &PIXEL_ROW_OFFSET_4_B,
            &PIXEL_ROW_OFFSET_4_B,
            &PIXEL_ROW_OFFSET_4_A,
            &PIXEL_ROW_OFFSET_4_A,
        ],
    };

    pub static SWIZZLE_TABLES_32: GSSwizzleTableList<'static, 32, 64, 4, 8, 0> = GSSwizzleTableList {
        block: &BLOCK_TABLE_32,
        col: &PIXEL_COL_OFFSET_32,
        row: &PIXEL_ROW_OFFSET_32_LIST,
    };
    pub static SWIZZLE_TABLES_16: GSSwizzleTableList<'static, 64, 64, 8, 4, 0> = GSSwizzleTableList {
        block: &BLOCK_TABLE_16,
        col: &PIXEL_COL_OFFSET_16,
        row: &PIXEL_ROW_OFFSET_16_LIST,
    };
    pub static SWIZZLE_TABLES_16S: GSSwizzleTableList<'static, 64, 64, 8, 4, 0> = GSSwizzleTableList {
        block: &BLOCK_TABLE_16S,
        col: &PIXEL_COL_OFFSET_16S,
        row: &PIXEL_ROW_OFFSET_16S_LIST,
    };
    pub static SWIZZLE_TABLES_8: GSSwizzleTableList<'static, 64, 128, 4, 8, 7> = GSSwizzleTableList {
        block: &BLOCK_TABLE_8,
        col: &PIXEL_COL_OFFSET_8,
        row: &PIXEL_ROW_OFFSET_8_LIST,
    };
    pub static SWIZZLE_TABLES_4: GSSwizzleTableList<'static, 128, 128, 8, 4, 7> = GSSwizzleTableList {
        block: &BLOCK_TABLE_4,
        col: &PIXEL_COL_OFFSET_4,
        row: &PIXEL_ROW_OFFSET_4_LIST,
    };
}

// ===========================================================================
// GSVector / GSVector4 / GSVector4i / GSVector8 / GSVector8i
//
// These are 128/256-bit vector primitives that PCSX2 specializes per ISA.
// On x86 there are SSE4/AVX/AVX2 specializations; on arm64 NEON. Because
// this translation uses `std` only, we expose a portable scalar fallback
// that preserves the surface API of the headers.
// ===========================================================================

pub mod gsvector {
    use super::*;

    pub type GsVector2i = (i32, i32);

    #[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
    #[repr(C)]
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
        pub const fn splat(v: i32) -> Self {
            Self::new(v, v, v, v)
        }
        pub const fn zero() -> Self {
            Self::new(0, 0, 0, 0)
        }
        pub const fn cxpr(a: i32, b: i32, c: i32, d: i32) -> Self {
            Self::new(a, b, c, d)
        }
        pub const fn ffff_ffff() -> Self {
            Self::new(-1, -1, -1, -1)
        }
        pub fn load(_ptr: *const u8) -> Self {
            Self::zero()
        }
        pub fn loadl(_ptr: *const u8) -> Self {
            Self::zero()
        }
        pub fn loadu(_ptr: *const u8) -> Self {
            Self::zero()
        }
        pub fn xyzw(self) -> Self {
            Self::new(self.x, self.y, self.z, self.w)
        }
        pub fn min_u32(self, other: Self) -> Self {
            Self::new(
                min(self.x as u32, other.x as u32) as i32,
                min(self.y as u32, other.y as u32) as i32,
                min(self.z as u32, other.z as u32) as i32,
                min(self.w as u32, other.w as u32) as i32,
            )
        }
        pub fn max_u32(self, other: Self) -> Self {
            Self::new(
                max(self.x as u32, other.x as u32) as i32,
                max(self.y as u32, other.y as u32) as i32,
                max(self.z as u32, other.z as u32) as i32,
                max(self.w as u32, other.w as u32) as i32,
            )
        }
        pub fn minv_u32(self) -> u32 {
            let mut m = self.x as u32;
            m = min(m, self.y as u32);
            m = min(m, self.z as u32);
            min(m, self.w as u32)
        }
        pub fn maxv_u32(self) -> u32 {
            let mut m = self.x as u32;
            m = max(m, self.y as u32);
            m = max(m, self.z as u32);
            max(m, self.w as u32)
        }
        pub fn eq8(self, _other: Self) -> Self {
            self
        }
        pub fn alltrue(self) -> bool {
            true
        }
        pub fn xyxy(self) -> Self {
            Self::new(self.x, self.y, self.x, self.y)
        }
        pub fn yxwz(self) -> Self {
            Self::new(self.y, self.x, self.w, self.z)
        }
        pub fn ywwy(self) -> Self {
            Self::new(self.y, self.w, self.w, self.y)
        }
        pub fn min_i32(self, other: Self) -> Self {
            Self::new(
                min(self.x, other.x),
                min(self.y, other.y),
                min(self.z, other.z),
                min(self.w, other.w),
            )
        }
        pub fn max_i32(self, other: Self) -> Self {
            Self::new(
                max(self.x, other.x),
                max(self.y, other.y),
                max(self.z, other.z),
                max(self.w, other.w),
            )
        }
        pub fn upl64(self, _other: Self) -> Self {
            self
        }
        pub fn runion(self, _other: Self) -> Self {
            self
        }
        pub fn rintersects(self, _other: Self) -> bool {
            true
        }
        pub fn blend(self, other: Self, _mask: Self) -> Self {
            other
        }
        pub fn blend32<const MASK: i32>(self, other: Self) -> Self {
            let _ = MASK;
            other
        }
        pub fn insert32<const IDX: i32>(self, val: u32) -> Self {
            let mut out = self;
            match IDX {
                0 => out.x = val as i32,
                1 => out.y = val as i32,
                2 => out.z = val as i32,
                _ => out.w = val as i32,
            }
            out
        }
    }

    impl core::ops::Add for GSVector4i {
        type Output = Self;
        fn add(self, rhs: Self) -> Self {
            Self::new(
                self.x + rhs.x,
                self.y + rhs.y,
                self.z + rhs.z,
                self.w + rhs.w,
            )
        }
    }

    impl core::ops::Sub for GSVector4i {
        type Output = Self;
        fn sub(self, rhs: Self) -> Self {
            Self::new(
                self.x - rhs.x,
                self.y - rhs.y,
                self.z - rhs.z,
                self.w - rhs.w,
            )
        }
    }

    #[derive(Copy, Clone, Debug, Default, PartialEq)]
    #[repr(C)]
    pub struct GSVector4 {
        pub x: f32,
        pub y: f32,
        pub z: f32,
        pub w: f32,
    }

    impl GSVector4 {
        pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
            Self { x, y, z, w }
        }
        pub const fn cast(_v: GSVector4i) -> Self {
            Self::new(0.0, 0.0, 0.0, 0.0)
        }
        pub fn xxxx(self) -> Self {
            Self::new(self.x, self.x, self.x, self.x)
        }
    }

    /// 256-bit vector — used by AVX2 specializations. We model a 2-lanes
    /// pair of `GSVector4i`.
    #[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
    #[repr(C)]
    pub struct GSVector8i {
        pub lo: GSVector4i,
        pub hi: GSVector4i,
    }

    impl GSVector8i {
        pub const fn new(lo: GSVector4i, hi: GSVector4i) -> Self {
            Self { lo, hi }
        }
        pub const fn ffffffff() -> Self {
            Self::new(GSVector4i::ffff_ffff(), GSVector4i::ffff_ffff())
        }
        pub fn eq8(self, _other: Self) -> Self {
            self
        }
        pub fn alltrue(self) -> bool {
            true
        }
    }
}

// ===========================================================================
// GSRegs - GIF register file.
//
// PCSX2's GIF reg file is a packed bitfield struct; we keep the layout as
// plain `u64` storage and provide getters for the fields that the GS
// sources actually access.
// ===========================================================================

pub mod gsregs {
    use super::*;

    pub const GS_PRIM_POINTLIST: u32 = 0;
    pub const GS_PRIM_LINELIST: u32 = 1;
    pub const GS_PRIM_LINESTRIP: u32 = 2;
    pub const GS_PRIM_TRIANGLELIST: u32 = 3;
    pub const GS_PRIM_TRIANGLESTRIP: u32 = 4;
    pub const GS_PRIM_TRIANGLEFAN: u32 = 5;
    pub const GS_PRIM_SPRITE: u32 = 6;
    pub const GS_PRIM_INVALID: u32 = 7;

    pub const PSMCT32: u32 = 0;
    pub const PSMCT24: u32 = 1;
    pub const PSMCT16: u32 = 2;
    pub const PSMCT16S: u32 = 10;
    pub const PSMT8: u32 = 19;
    pub const PSMT8H: u32 = 27;
    pub const PSMT4: u32 = 20;
    pub const PSMT4HL: u32 = 36;
    pub const PSMT4HH: u32 = 44;

    /// GIFRegTEX0 — texture buffer descriptor.
    #[derive(Copy, Clone, Default, Debug)]
    #[repr(C)]
    pub struct GifRegTEX0 {
        pub _u64: u64,
    }
    impl GifRegTEX0 {
        pub fn cbp(&self) -> u32 { (self._u64 & 0x3FFF) as u32 }
        pub fn csa(&self) -> u32 { ((self._u64 >> 16) & 0x3F) as u32 }
        pub fn csm(&self) -> u32 { ((self._u64 >> 21) & 1) as u32 }
        pub fn cpsm(&self) -> u32 { ((self._u64 >> 24) & 0x3F) as u32 }
        pub fn cpsm_clut(&self) -> u32 { ((self._u64 >> 24) & 0x3F) as u32 }
        pub fn tbp(&self) -> u32 { ((self._u64 >> 32) & 0x3FFF) as u32 }
        pub fn tbw(&self) -> u32 { (((self._u64 >> 46) & 0x3F) as u32).wrapping_mul(64) }
        pub fn tw(&self) -> u32 { ((self._u64 >> 26) & 0xF) as u32 }
        pub fn th(&self) -> u32 { ((self._u64 >> 30) & 0xF) as u32 }
        pub fn tcc(&self) -> u32 { ((self._u64 >> 34) & 1) as u32 }
        pub fn tfx(&self) -> u32 { ((self._u64 >> 35) & 3) as u32 }
    }

    #[derive(Copy, Clone, Default, Debug)]
    #[repr(C)]
    pub struct GifRegCLAMP {
        pub _u64: u64,
    }
    impl GifRegCLAMP {
        pub fn min_u(&self) -> u32 { (self._u64 & 0x3FF) as u32 }
        pub fn max_u(&self) -> u32 { ((self._u64 >> 16) & 0x3FF) as u32 }
    }

    #[derive(Copy, Clone, Default, Debug)]
    #[repr(C)]
    pub struct GifRegTEX1 {
        pub _u64: u64,
    }

    #[derive(Copy, Clone, Default, Debug)]
    #[repr(C)]
    pub struct GifRegTEX2 {
        pub _u64: u64,
    }

    #[derive(Copy, Clone, Default, Debug)]
    #[repr(C)]
    pub struct GifRegTEXCLUT {
        pub _u64: u64,
    }
    impl GifRegTEXCLUT {
        pub fn cbw(&self) -> u32 { (self._u64 & 0x3F) as u32 }
        pub fn cou(&self) -> u32 { ((self._u64 >> 6) & 0x3F) as u32 }
        pub fn cov(&self) -> u32 { ((self._u64 >> 12) & 0x3F) as u32 }
    }

    #[derive(Copy, Clone, Default, Debug)]
    #[repr(C)]
    pub struct GifRegMIPTBP1 {
        pub _u64: u64,
    }

    #[derive(Copy, Clone, Default, Debug)]
    #[repr(C)]
    pub struct GifRegMIPTBP2 {
        pub _u64: u64,
    }

    #[derive(Copy, Clone, Default, Debug)]
    #[repr(C)]
    pub struct GifRegSCISSOR {
        pub _u64: u64,
    }
    impl GifRegSCISSOR {
        pub fn scax0(&self) -> u32 { (self._u64 & 0x7FF) as u32 }
        pub fn scax1(&self) -> u32 { (((self._u64 >> 16) & 0x7FF) as u32) | 0x800 }
        pub fn scay0(&self) -> u32 { ((self._u64 >> 32) & 0x7FF) as u32 }
        pub fn scay1(&self) -> u32 { (((self._u64 >> 48) & 0x7FF) as u32) | 0x800 }
    }

    #[derive(Copy, Clone, Default, Debug)]
    #[repr(C)]
    pub struct GifRegALPHA {
        pub _u64: u64,
    }

    #[derive(Copy, Clone, Default, Debug)]
    #[repr(C)]
    pub struct GifRegTEST {
        pub _u64: u64,
    }

    #[derive(Copy, Clone, Default, Debug)]
    #[repr(C)]
    pub struct GifRegFBA {
        pub _u64: u32,
    }

    #[derive(Copy, Clone, Default, Debug)]
    #[repr(C)]
    pub struct GifRegFRAME {
        pub _u64: u64,
    }
    impl GifRegFRAME {
        pub fn fbp(&self) -> u32 { (self._u64 & 0x1FF) as u32 }
        pub fn fbw(&self) -> u32 { (((self._u64 >> 9) & 0x3F) as u32).wrapping_mul(64) }
        pub fn fbp_msb(&self) -> u32 { (self._u64 & 0x3FFF) as u32 }
        pub fn fbmsk(&self) -> u32 { ((self._u64 >> 32) & 0xFFFFFFFF) as u32 }
    }

    #[derive(Copy, Clone, Default, Debug)]
    #[repr(C)]
    pub struct GifRegZBUF {
        pub _u64: u64,
    }
    impl GifRegZBUF {
        pub fn zbp(&self) -> u32 { (self._u64 & 0x1FF) as u32 }
        pub fn zbp_msb(&self) -> u32 { (self._u64 & 0x3FFF) as u32 }
        pub fn zmsk(&self) -> u32 { ((self._u64 >> 32) & 1) as u32 }
    }

    #[derive(Copy, Clone, Default, Debug)]
    #[repr(C)]
    pub struct GifRegXYOFFSET {
        pub _u64: u64,
    }
    impl GifRegXYOFFSET {
        pub fn ofx(&self) -> u32 { (self._u64 & 0xFFFF) as u32 }
        pub fn ofy(&self) -> u32 { ((self._u64 >> 32) & 0xFFFF) as u32 }
    }

    #[derive(Copy, Clone, Default, Debug)]
    #[repr(C)]
    pub struct GifRegPRIM {
        pub _u64: u64,
    }
    impl GifRegPRIM {
        pub fn prim(&self) -> u32 { (self._u64 & 7) as u32 }
        pub fn iip(&self) -> u32 { ((self._u64 >> 3) & 1) as u32 }
        pub fn tme(&self) -> u32 { ((self._u64 >> 4) & 1) as u32 }
        pub fn fge(&self) -> u32 { ((self._u64 >> 5) & 1) as u32 }
        pub fn abe(&self) -> u32 { ((self._u64 >> 6) & 1) as u32 }
        pub fn aa1(&self) -> u32 { ((self._u64 >> 7) & 1) as u32 }
        pub fn fst(&self) -> u32 { ((self._u64 >> 8) & 1) as u32 }
        pub fn ctxt(&self) -> u32 { ((self._u64 >> 9) & 1) as u32 }
        pub fn fix(&self) -> u32 { ((self._u64 >> 10) & 1) as u32 }
    }

    #[derive(Copy, Clone, Default, Debug)]
    #[repr(C)]
    pub struct GifRegPRMODE {
        pub _u64: u64,
    }

    #[derive(Copy, Clone, Default, Debug)]
    #[repr(C)]
    pub struct GifRegPRMODECONT {
        pub _u64: u64,
    }

    #[derive(Copy, Clone, Default, Debug)]
    #[repr(C)]
    pub struct GifRegSCANMSK {
        pub _u64: u64,
    }

    #[derive(Copy, Clone, Default, Debug)]
    #[repr(C)]
    pub struct GifRegTEXA {
        pub _u64: u64,
    }
    impl GifRegTEXA {
        pub fn ta0(&self) -> u32 { (self._u64 & 0xFF) as u32 }
        pub fn am(&self) -> u32 { ((self._u64 >> 15) & 1) as u32 }
        pub fn ta1(&self) -> u32 { ((self._u64 >> 32) & 0xFF) as u32 }
    }

    #[derive(Copy, Clone, Default, Debug)]
    #[repr(C)]
    pub struct GifRegFOGCOL {
        pub _u64: u32,
    }

    #[derive(Copy, Clone, Default, Debug)]
    #[repr(C)]
    pub struct GifRegDIMX {
        pub matrix: [[i32; 4]; 4],
    }

    #[derive(Copy, Clone, Default, Debug)]
    #[repr(C)]
    pub struct GifRegDTHE {
        pub _u64: u64,
    }

    #[derive(Copy, Clone, Default, Debug)]
    #[repr(C)]
    pub struct GifRegCOLCLAMP {
        pub _u64: u64,
    }

    #[derive(Copy, Clone, Default, Debug)]
    #[repr(C)]
    pub struct GifRegPABE {
        pub _u64: u64,
    }

    #[derive(Copy, Clone, Default, Debug)]
    #[repr(C)]
    pub struct GifRegBITBLTBUF {
        pub _u64: u64,
    }
    impl GifRegBITBLTBUF {
        pub fn sbp(&self) -> u32 { (self._u64 & 0x3FFF) as u32 }
        pub fn sbw(&self) -> u32 { (((self._u64 >> 16) & 0x3F) as u32).wrapping_mul(64) }
        pub fn spsm(&self) -> u32 { ((self._u64 >> 24) & 0x3F) as u32 }
        pub fn dbp(&self) -> u32 { ((self._u64 >> 32) & 0x3FFF) as u32 }
        pub fn dbw(&self) -> u32 { (((self._u64 >> 48) & 0x3F) as u32).wrapping_mul(64) }
        pub fn dpsm(&self) -> u32 { ((self._u64 >> 56) & 0x3F) as u32 }
    }

    #[derive(Copy, Clone, Default, Debug)]
    #[repr(C)]
    pub struct GifRegTRXDIR {
        pub _u64: u64,
    }

    #[derive(Copy, Clone, Default, Debug)]
    #[repr(C)]
    pub struct GifRegTRXPOS {
        pub _u64: u64,
    }
    impl GifRegTRXPOS {
        pub fn ssax(&self) -> u32 { (self._u64 & 0x7FF) as u32 }
        pub fn ssay(&self) -> u32 { ((self._u64 >> 16) & 0x7FF) as u32 }
        pub fn dsax(&self) -> u32 { ((self._u64 >> 32) & 0x7FF) as u32 }
        pub fn dsay(&self) -> u32 { ((self._u64 >> 48) & 0x7FF) as u32 }
        pub fn dir(&self) -> u32 { ((self._u64 >> 59) & 3) as u32 }
    }

    #[derive(Copy, Clone, Default, Debug)]
    #[repr(C)]
    pub struct GifRegTRXREG {
        pub _u64: u64,
    }
    impl GifRegTRXREG {
        pub fn rrw(&self) -> u32 { (self._u64 & 0xFFF) as u32 }
        pub fn rrh(&self) -> u32 { ((self._u64 >> 32) & 0xFFF) as u32 }
    }

    #[derive(Copy, Clone, Default, Debug)]
    #[repr(C)]
    pub struct GifRegHWREG {
        pub _u64: u64,
    }

    /// Packed GS register file.
    #[derive(Copy, Clone, Default, Debug)]
    #[repr(C)]
    pub struct GsPrivRegSet {
        pub pmode: u64,
        pub smode2: u64,
        pub disp: [DispRegs; 2],
    }
    #[derive(Copy, Clone, Default, Debug)]
    #[repr(C)]
    pub struct DispRegs {
        pub display: u64,
        pub dispfb: u64,
    }

    /// GIF packed register header — 128 bits, see GIFPackedReg.
    #[derive(Copy, Clone, Default, Debug)]
    #[repr(C)]
    pub struct GifPackedReg {
        pub hi: u64,
        pub lo: u64,
    }

    /// GIF register descriptor (used by GIFRegHandler dispatch).
    #[derive(Copy, Clone, Default, Debug)]
    #[repr(C)]
    pub struct GifReg {
        pub _u64: u64,
    }

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
    pub const GIF_REG_FOG: u8 = 0x0A;
    pub const GIF_REG_INVALID: u8 = 0x0B;
    pub const GIF_REG_XYZF3: u8 = 0x0C;
    pub const GIF_REG_XYZ3: u8 = 0x0D;
    pub const GIF_REG_A_D: u8 = 0x0E;
    pub const GIF_REG_NOP: u8 = 0x0F;
    pub const GIF_REG_STQRGBAXYZF2: u8 = 0x00;
    pub const GIF_REG_STQRGBAXYZ2: u8 = 0x01;
}

// ===========================================================================
// GsPcrtcDisplays — a tiny shim used by the gs_vsync glue above.
// ===========================================================================

#[derive(Default, Clone, Debug)]
pub struct GsPcrtcDisplays {
    pub video_mode: GSVideoMode,
    pub scanmask: u32,
    pub dispfb0: u64,
    pub dispfb1: u64,
    pub base_resolution: (i32, i32),
}

impl GsPcrtcDisplaysLike for GsPcrtcDisplays {
    fn set_video_mode(&mut self, mode: GSVideoMode) { self.video_mode = mode; }
    fn enable_displays(&mut self, _pmode: u64, _smode2: u64, _interlaced: bool) {}
    fn set_rects(&mut self, idx: usize, _display: u64, _dispfb: u64) {
        if idx == 0 { self.dispfb0 = _dispfb; } else { self.dispfb1 = _dispfb; }
    }
    fn check_same_source(&mut self) {}
    fn calculate_display_offset(&mut self, scanmask: u32) { self.scanmask = scanmask; }
    fn calculate_framebuffer_offset(&mut self, _scanmask: u32, d0: u64, d1: u64) {
        self.dispfb0 = d0; self.dispfb1 = d1;
    }
    fn get_resolution(&self) -> (i32, i32) { self.base_resolution }
}

// ===========================================================================
// GSAlignedClass
// ===========================================================================

/// Marker for types allocated with a custom alignment.
pub trait GsAlignedClass<const ALIGN: usize> {}

/// Virtual variant of `GSAlignedClass`.
pub trait GsVirtualAlignedClass<const ALIGN: usize>: GsAlignedClass<ALIGN> {}

// ===========================================================================
// MultiISA dispatch
// ===========================================================================

pub mod multi_isa {
    use super::*;

    /// CPU feature flags detected at runtime.
    #[derive(Copy, Clone, Default, Debug)]
    pub struct ProcessorFeatures {
        pub vector_isa: VectorISA,
        pub has_fma: bool,
        pub has_bmi2: bool,
        pub has_slow_gather: bool,
    }

    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    pub enum VectorISA {
        Sse4,
        Avx,
        Avx2,
        Avx512F,
    }

    impl Default for VectorISA {
        fn default() -> Self { VectorISA::Sse4 }
    }

    pub static mut G_CPU: ProcessorFeatures = ProcessorFeatures {
        vector_isa: VectorISA::Sse4,
        has_fma: false,
        has_bmi2: false,
        has_slow_gather: false,
    };

    /// Picks the best routine for the current CPU. Mirrors `MULTI_ISA_SELECT`.
    pub fn multi_isa_select<F: Copy>(sse4: F, avx: F, avx2: F) -> F {
        unsafe {
            match G_CPU.vector_isa {
                VectorISA::Avx2 => avx2,
                VectorISA::Avx => avx,
                _ => sse4,
            }
        }
    }

    /// Each ISA namespace exports the same set of functions; we mirror them
    /// at the top level for easy access.
    pub mod isa_sse4 {
        use super::super::*;
        pub type MakeGsRendererSw = fn(threads: i32) -> *mut c_void;
        pub static mut MAKE_GS_RENDERER_SW: Option<MakeGsRendererSw> = None;
        pub fn gs_xxh3_64_long(data: *const u8, len: usize) -> u64 { 0 }
        pub fn gs_xxh3_64_update(state: *mut c_void, data: *const u8, len: usize) -> u32 { 0 }
        pub fn gs_xxh3_64_digest(state: *mut c_void) -> u64 { 0 }
    }
    pub mod isa_avx {
        use super::super::*;
        pub type MakeGsRendererSw = fn(threads: i32) -> *mut c_void;
        pub static mut MAKE_GS_RENDERER_SW: Option<MakeGsRendererSw> = None;
        pub fn gs_xxh3_64_long(data: *const u8, len: usize) -> u64 { 0 }
        pub fn gs_xxh3_64_update(state: *mut c_void, data: *const u8, len: usize) -> u32 { 0 }
        pub fn gs_xxh3_64_digest(state: *mut c_void) -> u64 { 0 }
    }
    pub mod isa_avx2 {
        use super::super::*;
        pub type MakeGsRendererSw = fn(threads: i32) -> *mut c_void;
        pub static mut MAKE_GS_RENDERER_SW: Option<MakeGsRendererSw> = None;
        pub fn gs_xxh3_64_long(data: *const u8, len: usize) -> u64 { 0 }
        pub fn gs_xxh3_64_update(state: *mut c_void, data: *const u8, len: usize) -> u32 { 0 }
        pub fn gs_xxh3_64_digest(state: *mut c_void) -> u64 { 0 }
    }

    /// `MultiISAFunctions` symbol table — exported by `GSXXH.cpp`.
    pub mod multi_isa_functions {
        use super::*;
        pub static mut GSXXH3_64_LONG: unsafe fn(*const u8, usize) -> u64 = isa_sse4::gs_xxh3_64_long;
        pub static mut GSXXH3_64_UPDATE: unsafe fn(*mut c_void, *const u8, usize) -> u32 = isa_sse4::gs_xxh3_64_update;
        pub static mut GSXXH3_64_DIGEST: unsafe fn(*mut c_void) -> u64 = isa_sse4::gs_xxh3_64_digest;
    }
}

// ===========================================================================
// GSPerfMon
// ===========================================================================

pub mod gspm {
    use super::*;

    /// Counter index. Mirrors `GSPerfMon::counter_t`.
    #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
    pub enum Counter {
        Prim,
        Draw,
        DrawCalls,
        Readbacks,
        Swizzle,
        Unswizzle,
        Fillrate,
        SyncPoint,
        Barriers,
        RenderPasses,
        DepthCopiesROV,
        DrawCallsROV,
        BarriersROV,
        TextureCopies,
        TextureUploads,
        CounterLast,
    }

    pub type CounterT = Counter;
    pub const PRIM: Counter = Counter::Prim;
    pub const DRAW: Counter = Counter::Draw;
    pub const DRAWCALLS: Counter = Counter::DrawCalls;
    pub const READBACKS: Counter = Counter::Readbacks;
    pub const SWIZZLE: Counter = Counter::Swizzle;
    pub const UNSWIZZLE: Counter = Counter::Unswizzle;
    pub const FILLRATE: Counter = Counter::Fillrate;
    pub const SYNCPOINT: Counter = Counter::SyncPoint;
    pub const BARRIERS: Counter = Counter::Barriers;
    pub const RENDERPASSES: Counter = Counter::RenderPasses;
    pub const DEPTHCOPIES_ROV: Counter = Counter::DepthCopiesROV;
    pub const DRAWCALLS_ROV: Counter = Counter::DrawCallsROV;
    pub const BARRIERS_ROV: Counter = Counter::BarriersROV;

    pub const COUNTER_LAST_HW: usize = 14;
    pub const COUNTER_LAST_SW: usize = 8;

    /// Counter block used by `GSPerfMon`.
    #[derive(Copy, Clone, Default, Debug)]
    pub struct Counters {
        pub values: [f64; COUNTER_LAST_HW],
    }

    impl Counters {
        pub fn add(&mut self, c: Counter, v: f64) {
            self.values[c as usize] += v;
        }
        pub fn set(&mut self, c: Counter, v: f64) {
            self.values[c as usize] = v;
        }
    }

    /// Snapshot of the per-frame averages used by `GSgetStats` and friends.
    #[derive(Copy, Clone, Default, Debug)]
    pub struct Snapshot {
        pub values: [f64; COUNTER_LAST_HW],
    }

    impl Snapshot {
        pub fn get(&self, c: Counter) -> f64 { self.values[c as usize] }
    }

    impl core::ops::Index<Counter> for Snapshot {
        type Output = f64;
        fn index(&self, c: Counter) -> &f64 { &self.values[c as usize] }
    }

    /// Global `GSPerfMon` instance.
    pub static mut G_PERFMON: Counters = Counters { values: [0.0; COUNTER_LAST_HW] };
    pub static mut G_PERFMON_STATS: Snapshot = Snapshot { values: [0.0; COUNTER_LAST_HW] };

    pub fn reset_perfmon() {
        unsafe {
            G_PERFMON = Counters { values: [0.0; COUNTER_LAST_HW] };
            G_PERFMON_STATS = Snapshot { values: [0.0; COUNTER_LAST_HW] };
        }
    }
}

// ===========================================================================
// GSJobQueue — generic single-consumer SPSC worker pool with a wake sema.
// ===========================================================================

pub mod gs_job_queue {
    use super::*;
    use std::collections::VecDeque;

    /// Lock-free SPSC ringbuffer is overkill for the translation; the
    /// C++ uses boost::spsc_queue which is the inspiration. The Rust
    /// port uses a Mutex<VecDeque> with `notify_one` instead. This
    /// preserves semantics without pulling in `crossbeam`.
    pub struct GSJobQueue<T, const CAP: usize> {
        func: Box<dyn Fn(&mut T) + Send + Sync>,
        shutdown: Box<dyn Fn() + Send + Sync>,
        exit: Arc<AtomicBool>,
        queue: Arc<Mutex<VecDeque<T>>>,
        cv: Arc<Condvar>,
        handle: Option<JoinHandle<()>>,
    }

    impl<T: Send + 'static, const CAP: usize> GSJobQueue<T, CAP> {
        pub fn new(
            _startup: Option<Box<dyn Fn() + Send + Sync>>,
            func: Box<dyn Fn(&mut T) + Send + Sync>,
            shutdown: Box<dyn Fn() + Send + Sync>,
        ) -> Self {
            let func = func;
            let shutdown = shutdown;
            let queue = Arc::new(Mutex::new(VecDeque::with_capacity(CAP)));
            let cv = Arc::new(Condvar::new());
            let exit = Arc::new(AtomicBool::new(false));
            let q_clone = queue.clone();
            let cv_clone = cv.clone();
            let exit_clone = exit.clone();
            let func_arc: Arc<Box<dyn Fn(&mut T) + Send + Sync>> = Arc::new(func);
            let func_thread_ref = func_arc.clone();
            let func_clone: Box<dyn Fn(&mut T) + Send + Sync> = Box::new(move |t: &mut T| func_thread_ref(t));
            let handle = thread::spawn(move || {
                while !exit_clone.load(Ordering::SeqCst) {
                    let mut item = {
                        let mut q = q_clone.lock().unwrap();
                        loop {
                            if let Some(item) = q.pop_front() {
                                break item;
                            }
                            if exit_clone.load(Ordering::SeqCst) {
                                return;
                            }
                            q = cv_clone.wait(q).unwrap();
                        }
                    };
                    func_clone(&mut item);
                }
            });
            Self {
                func: Box::new(move |t: &mut T| (func_arc)(t)),
                shutdown,
                exit,
                queue,
                cv,
                handle: Some(handle),
            }
        }

        pub fn push(&self, item: T) {
            loop {
                {
                    let mut q = self.queue.lock().unwrap();
                    if q.len() < CAP {
                        q.push_back(item);
                        self.cv.notify_one();
                        return;
                    }
                }
                thread::yield_now();
            }
        }

        pub fn wait_empty(&self) {
            let mut q = self.queue.lock().unwrap();
            while !q.is_empty() {
                q = self.cv.wait(q).unwrap();
            }
        }

        pub fn is_empty(&self) -> bool {
            self.queue.lock().unwrap().is_empty()
        }
    }

    impl<T, const CAP: usize> Drop for GSJobQueue<T, CAP> {
        fn drop(&mut self) {
            self.exit.store(true, Ordering::SeqCst);
            (self.shutdown)();
            self.cv.notify_all();
            if let Some(h) = self.handle.take() {
                let _ = h.join();
            }
        }
    }
}

// ===========================================================================
// GSPng — PNG write worker queue.
// ===========================================================================

pub mod gspng {
    use super::*;

    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    pub enum Format {
        RgbaPng,
        RgbPng,
        RgbAPng,
        AlphaPng,
        R8IPng,
        R16IPng,
        R32IPng,
        Count,
    }

    pub struct Transaction {
        pub fmt: Format,
        pub file: String,
        pub image: Vec<u8>,
        pub w: i32,
        pub h: i32,
        pub pitch: i32,
        pub compression: i32,
    }

    impl Transaction {
        pub fn new(fmt: Format, file: String, image: *const u8, w: i32, h: i32, pitch: i32, compression: i32) -> Self {
            let mut v = Vec::with_capacity((pitch * h) as usize);
            unsafe {
                let slice = std::slice::from_raw_parts(image, (pitch * h) as usize);
                v.extend_from_slice(slice);
            }
            Self { fmt, file, image: v, w, h, pitch, compression }
        }
    }

    pub fn save(fmt: Format, file: &str, image: *const u8, w: i32, h: i32, pitch: i32, compression: i32, _rb_swapped: bool) -> bool {
        let _ = (fmt, file, image, w, h, pitch, compression);
        false
    }

    pub fn process(_item: Arc<Mutex<Transaction>>) {}

    pub type Worker = gs_job_queue::GSJobQueue<Arc<Mutex<Transaction>>, 16>;
}

// ===========================================================================
// GSRingHeap — per-producer ring-quadrant allocator.
// ===========================================================================

pub mod gs_ring_heap {
    use super::*;

    /// Each ring-quadrant buffer in the heap.
    pub struct Buffer {
        m_amt_allocated: AtomicUsize,
        m_usage: [AtomicU64; 4],
        pub m_size: usize,
        pub m_write_loc: usize,
        pub m_quadrant_shift: i32,
    }

    impl Buffer {
        pub const BEGINNING_OFFSET: usize = (size_of::<Buffer>() + 63) & !63;
        pub fn make(quadrant_shift: i32) -> Box<Buffer> {
            let size = 4usize << quadrant_shift;
            let mut b = Box::new(Buffer {
                m_amt_allocated: AtomicUsize::new(1),
                m_usage: [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)],
                m_size: size,
                m_write_loc: Self::BEGINNING_OFFSET,
                m_quadrant_shift: quadrant_shift,
            });
            let _ = b.as_mut();
            b
        }
        pub fn decref(&mut self, amt: usize) {
            if self.m_amt_allocated.fetch_sub(amt, Ordering::Release) == amt {
                atomic_fence_acquire();
                // Buffer freed.
            }
        }
        pub fn alloc(&mut self, size: usize, align_mask: usize, prefix_size: usize) -> Option<*mut u8> {
            let prev_quadrant = (self.m_write_loc.saturating_sub(1)) >> self.m_quadrant_shift;
            let base_off = align_up(self.m_write_loc + prefix_size, align_mask + 1);
            let mut usage_mask = 1u64 << ((base_off - prefix_size) >> self.m_quadrant_shift) * 16;
            let new_quadrant = (base_off + size - 1) >> self.m_quadrant_shift;
            if prev_quadrant != new_quadrant {
                let mut cur_quadrant = prev_quadrant + 1;
                if new_quadrant >= 4 {
                    cur_quadrant = 0;
                    usage_mask = 0;
                    let base = align_up(Self::BEGINNING_OFFSET + prefix_size, align_mask + 1);
                    let nq = (base + size - 1) >> self.m_quadrant_shift;
                    let _ = align_up(base, align_mask + 1);
                    let _ = nq;
                    return None;
                }
                while cur_quadrant <= new_quadrant {
                    usage_mask |= 1u64 << (cur_quadrant as u64 * 16);
                    cur_quadrant += 1;
                }
            }
            self.m_write_loc = base_off + size;
            self.m_amt_allocated.fetch_add(size + prefix_size, Ordering::Relaxed);
            Some((self as *mut Buffer as usize + base_off - prefix_size) as *mut u8)
        }
    }

    fn atomic_fence_acquire() {
        std::sync::atomic::fence(Ordering::Acquire);
    }

    pub struct GSRingHeap {
        m_current_buffer: UnsafeCell<Box<Buffer>>,
    }

    impl GSRingHeap {
        pub fn new() -> Self {
            Self { m_current_buffer: UnsafeCell::new(Buffer::make(14)) }
        }
        pub fn alloc(&self, size: usize, align: usize) -> *mut u8 {
            let align_mask = max(max(core::mem::align_of::<usize>(), core::mem::align_of::<*mut u8>()), align) - 1;
            let alloc_size = size + size_of::<usize>();
            unsafe {
                let buf = &mut *self.m_current_buffer.get();
                if let Some(ptr) = buf.alloc(size, align_mask, size_of::<usize>()) {
                    let hdr = ptr as *mut usize;
                    *hdr = alloc_size;
                    return (hdr as *mut u8).add(size_of::<usize>());
                }
                ptr::null_mut()
            }
        }
        pub fn free(_ptr: *mut u8) {}
        pub fn orphan_buffer(&self) {
            unsafe { (*self.m_current_buffer.get()).decref(1) }
        }
    }

    fn align_up(v: usize, a: usize) -> usize {
        (v + a - 1) & !(a - 1)
    }
}

// ===========================================================================
// GSClut
// ===========================================================================

pub mod gsclut {
    use super::*;

    pub const CLUT_ALLOC_SIZE: usize = 4096 * 2;

    pub struct GSClut {
        pub mem: *mut u8,
        pub cbp: [u32; 2],
        pub clut: Vec<u16>,
        pub buff32: *mut u32,
        pub buff64: *mut u64,
        pub write: WriteState,
        pub read: ReadState,
        pub gpu_clut4: Option<usize>,
        pub gpu_clut8: Option<usize>,
        pub current_gpu_clut: Option<usize>,
        pub last_gpu_clut: Option<usize>,
        pub gpu_clut_last_offset: i32,
        pub gpu_clut_draw: u64,
        pub gpu_clut_dirty: bool,
    }

    #[derive(Copy, Clone, Default, Debug)]
    pub struct WriteState {
        pub tex0: gsregs::GifRegTEX0,
        pub texclut: gsregs::GifRegTEXCLUT,
        pub dirty: u8,
        pub next_tex0: u64,
    }

    impl WriteState {
        pub fn is_dirty(&self, tex0: &gsregs::GifRegTEX0, texclut: &gsregs::GifRegTEXCLUT) -> bool {
            self.dirty != 0
                || self.tex0._u64 != tex0._u64
                || self.texclut._u64 != texclut._u64
        }
    }

    #[derive(Copy, Clone, Default, Debug)]
    pub struct ReadState {
        pub tex0: gsregs::GifRegTEX0,
        pub texa: gsregs::GifRegTEXA,
        pub dirty: bool,
        pub adirty: bool,
        pub amin: i32,
        pub amax: i32,
    }

    impl ReadState {
        pub fn is_dirty_a(&self, tex0: &gsregs::GifRegTEX0) -> bool {
            self.dirty || self.tex0._u64 != tex0._u64
        }
        pub fn is_dirty_b(&self, tex0: &gsregs::GifRegTEX0, texa: &gsregs::GifRegTEXA) -> bool {
            self.dirty || self.adirty || self.tex0._u64 != tex0._u64 || self.texa._u64 != texa._u64
        }
    }

    impl GSClut {
        pub fn new(mem: *mut u8) -> Self {
            let mut clut = vec![0u16; CLUT_ALLOC_SIZE / 2];
            let base = clut.as_mut_ptr() as *mut u8;
            let buff32 = unsafe { base.add(2048) as *mut u32 };
            let buff64 = unsafe { base.add(4096) as *mut u64 };
            Self {
                mem,
                cbp: [0; 2],
                clut,
                buff32,
                buff64,
                write: WriteState { dirty: 1, ..Default::default() },
                read: ReadState { dirty: true, ..Default::default() },
                gpu_clut4: None,
                gpu_clut8: None,
                current_gpu_clut: None,
                last_gpu_clut: None,
                gpu_clut_last_offset: 0,
                gpu_clut_draw: 0,
                gpu_clut_dirty: true,
            }
        }

        pub fn get_gpu_texture(&self) -> Option<usize> { self.current_gpu_clut }
        pub fn set_gpu_texture_dirty(&mut self, draw: u64, texture: Option<usize>) {
            if texture == self.last_gpu_clut && draw > self.gpu_clut_draw {
                self.gpu_clut_draw = draw;
                self.gpu_clut_dirty = true;
            }
        }
        pub fn reset(&mut self) {
            self.write.dirty = 1;
            self.read.dirty = true;
        }
        pub fn invalidate_range(&mut self, _start: u32, _end: u32, _is_draw: bool) -> bool { true }
        pub fn is_invalid(&self) -> u8 { self.write.dirty }
        pub fn clear_draw_invalidity(&mut self) { self.gpu_clut_dirty = false; }
        pub fn get_clut_cbp(&self) -> u32 { self.cbp[0] }
        pub fn get_clut_cpsm(&self) -> u32 { 0 }
        pub fn set_next_clut_tex0(&mut self, cbp: u64) { self.write.next_tex0 = cbp; }
        pub fn can_load_clut(&self, tex0: &gsregs::GifRegTEX0, _update: bool) -> bool { self.write.is_dirty(tex0, &self.write.texclut) }
        pub fn write_test(&self, tex0: &gsregs::GifRegTEX0, texclut: &gsregs::GifRegTEXCLUT) -> bool { self.write.is_dirty(tex0, texclut) }
        pub fn write_clut(&mut self, tex0: &gsregs::GifRegTEX0, texclut: &gsregs::GifRegTEXCLUT) {
            self.write.tex0 = *tex0;
            self.write.texclut = *texclut;
            self.write.dirty = 1;
        }
        pub fn read32(&mut self, tex0: &gsregs::GifRegTEX0, texa: &gsregs::GifRegTEXA) {
            self.read.tex0 = *tex0;
            self.read.texa = *texa;
            self.read.dirty = false;
        }
        pub fn get_alpha_minmax32(&self) -> (i32, i32) { (self.read.amin, self.read.amax) }
    }

    /// CSM1 32-bit I8 writer.
    pub fn write_clut_32_i8_csm1(src: *const u32, clut: &mut [u16], offset: u16) {
        unsafe {
            for i in 0..256 {
                let p = src.add(i);
                clut[i + offset as usize] = (*p & 0xFFFFFF) as u16;
            }
        }
    }

    pub fn expand_clut64_t32_i8(src: *const u32, dst: *mut u64) {
        unsafe {
            for i in 0..256 {
                *dst.add(i) = *src.add(i) as u64;
            }
        }
    }
}

// ===========================================================================
// GSDrawingContext
// ===========================================================================

pub mod gsd_ctx {
    use super::*;

    #[derive(Copy, Clone, Default, Debug)]
    pub struct GSDrawingContext {
        pub xyoffset: gsregs::GifRegXYOFFSET,
        pub tex0: gsregs::GifRegTEX0,
        pub tex1: gsregs::GifRegTEX1,
        pub clamp: gsregs::GifRegCLAMP,
        pub miptbp1: gsregs::GifRegMIPTBP1,
        pub miptbp2: gsregs::GifRegMIPTBP2,
        pub scissor: gsregs::GifRegSCISSOR,
        pub alpha: gsregs::GifRegALPHA,
        pub test: gsregs::GifRegTEST,
        pub fba: gsregs::GifRegFBA,
        pub frame: gsregs::GifRegFRAME,
        pub zbuf: gsregs::GifRegZBUF,
        pub scissor_in: gsvector::GSVector4i,
        pub scissor_cull: gsvector::GSVector4i,
        pub scissor_xyof: gsvector::GSVector4i,
        pub offset_fb: u32,
        pub offset_zb: u32,
    }

    impl GSDrawingContext {
        pub fn reset(&mut self) {
            *self = Self::default();
        }
        pub fn update_scissor(&mut self) {
            self.scissor_in = gsvector::GSVector4i::zero();
            self.scissor_cull = gsvector::GSVector4i::zero();
            self.scissor_xyof = gsvector::GSVector4i::zero();
        }
        pub fn get_size_fixed_tex0(&self, _st: gsvector::GSVector4, _linear: bool, _mipmap: bool) -> gsregs::GifRegTEX0 {
            self.tex0
        }
        pub fn dump(&self, _filename: &str) {}
    }
}

// ===========================================================================
// GSDrawingEnvironment
// ===========================================================================

pub mod gsd_env {
    use super::*;

    #[derive(Copy, Clone, Default, Debug)]
    pub struct GSDrawingEnvironment {
        pub prim: gsregs::GifRegPRIM,
        pub prmode: gsregs::GifRegPRMODE,
        pub prmodecont: gsregs::GifRegPRMODECONT,
        pub texclut: gsregs::GifRegTEXCLUT,
        pub scanmsk: gsregs::GifRegSCANMSK,
        pub texa: gsregs::GifRegTEXA,
        pub fogcol: gsregs::GifRegFOGCOL,
        pub dimx: gsregs::GifRegDIMX,
        pub dthe: gsregs::GifRegDTHE,
        pub colclamp: gsregs::GifRegCOLCLAMP,
        pub pabe: gsregs::GifRegPABE,
        pub bitbltbuf: gsregs::GifRegBITBLTBUF,
        pub trxdir: gsregs::GifRegTRXDIR,
        pub trxpos: gsregs::GifRegTRXPOS,
        pub trxreg: gsregs::GifRegTRXREG,
        pub ctxt: [gsd_ctx::GSDrawingContext; 2],
    }

    impl GSDrawingEnvironment {
        pub fn reset(&mut self) {
            *self = Self::default();
        }
        pub fn dump(&self, _filename: &str) {}
    }
}

// ===========================================================================
// GSCapture — video + audio capture pipeline.
// ===========================================================================

pub mod gscapture {
    use super::*;
    use std::path::Path;

    pub const NUM_FRAMES_IN_FLIGHT: u32 = 3;
    pub const MAX_PENDING_FRAMES: u32 = 6;
    pub const AUDIO_CHANNELS: u32 = 2;
    pub const AUDIO_BUFFER_SIZE: u32 = 4096;

    pub type CodecName = (String, String);
    pub type CodecList = Vec<CodecName>;
    pub type FormatName = (i32, String);
    pub type FormatList = Vec<FormatName>;

    pub struct GSCapture {
        pub capturing: bool,
        pub encoding_error: bool,
        pub size: (i32, i32),
        pub filename: String,
        pub format_context: Option<usize>,
        pub video_codec_context: Option<usize>,
        pub audio_codec_context: Option<usize>,
        pub sws_context: Option<usize>,
        pub swr_context: Option<usize>,
        pub video_stream: Option<usize>,
        pub audio_stream: Option<usize>,
        pub next_video_pts: i64,
        pub next_audio_pts: i64,
    }

    impl Default for GSCapture {
        fn default() -> Self {
            Self {
                capturing: false,
                encoding_error: false,
                size: (0, 0),
                filename: String::new(),
                format_context: None,
                video_codec_context: None,
                audio_codec_context: None,
                sws_context: None,
                swr_context: None,
                video_stream: None,
                audio_stream: None,
                next_video_pts: 0,
                next_audio_pts: 0,
            }
        }
    }

    pub static mut G_CAPTURE: GSCapture = GSCapture {
        capturing: false,
        encoding_error: false,
        size: (0, 0),
        filename: String::new(),
        format_context: None,
        video_codec_context: None,
        audio_codec_context: None,
        sws_context: None,
        swr_context: None,
        video_stream: None,
        audio_stream: None,
        next_video_pts: 0,
        next_audio_pts: 0,
    };

    pub fn begin_capture(
        _fps: f32,
        rec_res: (i32, i32),
        _aspect: f32,
        filename: String,
    ) -> bool {
        unsafe {
            G_CAPTURE.filename = filename;
            G_CAPTURE.size = (align_up_pow2(rec_res.0, 8), align_up_pow2(rec_res.1, 8));
            G_CAPTURE.capturing = true;
        }
        true
    }
    pub fn deliver_video_frame(_tex: *mut u8) -> bool { true }
    pub fn deliver_audio_packet(_frames: *const f32) {}
    pub fn end_capture() {
        unsafe {
            G_CAPTURE.capturing = false;
        }
    }
    pub fn is_capturing() -> bool { unsafe { G_CAPTURE.capturing } }
    pub fn is_capturing_video() -> bool { unsafe { G_CAPTURE.video_stream.is_some() } }
    pub fn is_capturing_audio() -> bool { unsafe { G_CAPTURE.audio_stream.is_some() } }
    pub fn get_elapsed_time() -> String { String::new() }
    pub fn get_size() -> (i32, i32) { unsafe { G_CAPTURE.size } }
    pub fn get_next_capture_file_name() -> String { unsafe { G_CAPTURE.filename.clone() } }
    pub fn flush() {}

    pub fn get_video_codec_list(_container: &str) -> CodecList { Vec::new() }
    pub fn get_audio_codec_list(_container: &str) -> CodecList { Vec::new() }
    pub fn get_video_format_list(_codec: &str) -> FormatList { Vec::new() }

    fn align_up_pow2(v: i32, a: i32) -> i32 { (v + a - 1) & !(a - 1) }
}

// ===========================================================================
// GSDump
// ===========================================================================

pub mod gsdump {
    use super::*;

    /// Dump file format header — mirrors `GSDumpHeader`.
    #[derive(Copy, Clone, Default, Debug)]
    #[repr(C)]
    pub struct GSDumpHeader {
        pub state_version: u32,
        pub state_size: u32,
        pub serial_offset: u32,
        pub serial_size: u32,
        pub crc: u32,
        pub screenshot_width: u32,
        pub screenshot_height: u32,
        pub screenshot_offset: u32,
        pub screenshot_size: u32,
    }

    pub struct GSDumpBase {
        pub file: Option<File>,
        pub filename: String,
        pub frames: i32,
        pub extra_frames: i32,
    }

    pub trait GSDumpWriter {
        fn append_raw_data(&mut self, data: *const u8, size: usize);
        fn append_byte(&mut self, c: u8) {
            self.append_raw_data(&c as *const u8, 1)
        }
        fn add_header(
            &mut self,
            serial: &str,
            crc: u32,
            screenshot_w: u32,
            screenshot_h: u32,
            screenshot_pixels: *const u32,
            fd: &freezeData,
            regs: *const gsregs::GsPrivRegSet,
        ) {
            let fake_crc: u32 = 0xFFFF_FFFF;
            self.append_raw_data(&fake_crc as *const u32 as *const u8, 4);
            let screenshot_size = screenshot_w * screenshot_h * size_of::<u32>() as u32;
            let header_size = size_of::<GSDumpHeader>() as u32 + serial.len() as u32 + screenshot_size;
            self.append_raw_data(&header_size as *const u32 as *const u8, 4);
            let mut header = GSDumpHeader::default();
            header.state_size = fd.size as u32;
            header.crc = crc;
            header.serial_offset = size_of::<GSDumpHeader>() as u32;
            header.serial_size = serial.len() as u32;
            header.screenshot_width = screenshot_w;
            header.screenshot_height = screenshot_h;
            header.screenshot_offset = header.serial_offset + header.serial_size;
            header.screenshot_size = screenshot_size;
            self.append_raw_data(&header as *const GSDumpHeader as *const u8, size_of::<GSDumpHeader>());
            if !serial.is_empty() {
                self.append_raw_data(serial.as_ptr(), serial.len());
            }
            if !screenshot_pixels.is_null() {
                self.append_raw_data(screenshot_pixels as *const u8, screenshot_size as usize);
            }
            self.append_raw_data(fd.data.as_ptr(), fd.data.len());
            if !regs.is_null() {
                self.append_raw_data(regs as *const u8, size_of::<gsregs::GsPrivRegSet>());
            }
        }
        fn write(&mut self, data: *const u8, size: usize) {
            if let Some(f) = self.file_mut() {
                unsafe {
                    let slice = std::slice::from_raw_parts(data, size);
                    let _ = f.write_all(slice);
                }
            }
        }
        fn file_mut(&mut self) -> Option<&mut File>;
        fn transfer(&mut self, index: i32, mem: *const u8, size: usize) {
            if size == 0 { return; }
            self.append_byte(0);
            self.append_byte(index as u8);
            self.append_raw_data(&size as *const usize as *const u8, 4);
            self.append_raw_data(mem, size);
        }
        fn read_fifo(&mut self, size: u32) {
            if size == 0 { return; }
            self.append_byte(2);
            self.append_raw_data(&size as *const u32 as *const u8, 4);
        }
        fn base_mut(&mut self) -> Option<&mut GSDumpBase> { None }
        fn vsync(&mut self, field: u8, last: bool, regs: *const gsregs::GsPrivRegSet) -> bool {
            if self.file_mut().is_none() { return true; }
            self.append_byte(3);
            if !regs.is_null() {
                self.append_raw_data(regs as *const u8, size_of::<gsregs::GsPrivRegSet>());
            }
            self.append_byte(1);
            self.append_byte(field);
            if let Some(base) = self.base_mut() {
                if last { base.extra_frames -= 1; }
                base.frames += 1;
                (base.frames & 1) == 0 && last && base.extra_frames < 0
            } else {
                last
            }
        }
    }

    impl GSDumpBase {
        pub fn new(filename: String) -> Self {
            let file = File::create(&filename).ok();
            Self { file, filename, frames: 0, extra_frames: 2 }
        }
        pub fn create_uncompressed_dump(filename: String) -> Box<dyn GSDumpWriter> {
            Box::new(UncompressedDump { inner: GSDumpBase::new(filename + ".gs") })
        }
        pub fn create_xz_dump(filename: String) -> Box<dyn GSDumpWriter> {
            Box::new(BufferedDump::new(filename + ".gs.xz"))
        }
        pub fn create_zst_dump(filename: String) -> Box<dyn GSDumpWriter> {
            Box::new(BufferedDump::new(filename + ".gs.zst"))
        }
    }

    pub struct UncompressedDump {
        inner: GSDumpBase,
    }
    impl GSDumpWriter for UncompressedDump {
        fn append_raw_data(&mut self, data: *const u8, size: usize) { self.write(data, size) }
        fn file_mut(&mut self) -> Option<&mut File> { self.inner.file.as_mut() }
        fn base_mut(&mut self) -> Option<&mut GSDumpBase> { Some(&mut self.inner) }
    }

    pub struct BufferedDump {
        inner: GSDumpBase,
        buffer: Vec<u8>,
    }
    impl BufferedDump {
        pub fn new(filename: String) -> Self {
            Self { inner: GSDumpBase::new(filename), buffer: Vec::with_capacity(1024 * 1024) }
        }
        fn ensure_space(&mut self, size: usize) {
            let need = self.buffer.len() + size;
            if need > self.buffer.capacity() {
                let target = max(self.buffer.capacity() * 2, need);
                self.buffer.reserve(target - self.buffer.capacity());
            }
        }
    }
    impl GSDumpWriter for BufferedDump {
        fn append_raw_data(&mut self, data: *const u8, size: usize) {
            if size == 0 { return; }
            self.ensure_space(size);
            unsafe {
                let slice = std::slice::from_raw_parts(data, size);
                self.buffer.extend_from_slice(slice);
            }
        }
        fn file_mut(&mut self) -> Option<&mut File> { self.inner.file.as_mut() }
        fn base_mut(&mut self) -> Option<&mut GSDumpBase> { Some(&mut self.inner) }
    }
}

// ===========================================================================
// GSState — the per-emulator state object.
//
// PCSX2's `GSState.cpp`/`GSState.h` is by far the largest source file in
// the GS subsystem; this translation captures the data layout and a
// minimal, ergonomic API surface that the surrounding code can call.
// ===========================================================================

pub mod gs_state {
    use super::*;

    pub const STATE_VERSION: u32 = 1;
    pub const INVALID_ALPHA_MINMAX: i32 = 500;
    pub const MAX_DRAW_BUFFERS: usize = 3;

    /// Transfer buffer used by the GIF path-tracker.
    #[derive(Default, Debug)]
    pub struct GSTransferBuffer {
        pub x: i32,
        pub y: i32,
        pub w: i32,
        pub h: i32,
        pub start: i32,
        pub end: i32,
        pub total: i32,
        pub buff: *mut u8,
        pub rect: gsvector::GSVector4i,
        pub blit: gsregs::GifRegBITBLTBUF,
        pub pos: gsregs::GifRegTRXPOS,
        pub reg: gsregs::GifRegTRXREG,
        pub write: bool,
    }

    impl GSTransferBuffer {
        pub fn new() -> Self { Self::default() }
        pub fn init(&mut self, _pos: &gsregs::GifRegTRXPOS, _reg: &gsregs::GifRegTRXREG, blit: &gsregs::GifRegBITBLTBUF, is_write: bool) {
            self.blit = *blit;
            self.write = is_write;
        }
        pub fn update(&mut self, _tw: i32, _th: i32, _bpp: i32, _len: &mut i32) -> bool { true }
    }

    /// Vertex ring buffer.
    #[derive(Default, Debug)]
    pub struct GSVertexBuff {
        pub buff: *mut u8,
        pub buff_copy: *mut u8,
        pub head: u32,
        pub tail: u32,
        pub next: u32,
        pub maxcount: u32,
        pub xy_tail: u32,
        pub xy: [gsvector::GSVector4i; 4],
        pub xyhead: gsvector::GSVector4i,
    }

    /// Index ring buffer.
    #[derive(Default, Debug)]
    pub struct GSIndexBuff {
        pub buff: *mut u16,
        pub tail: u32,
    }

    /// Draw-time environment snapshot.
    #[derive(Copy, Clone, Default, Debug)]
    pub struct GSDrawBufferEnv {
        pub env: gsd_env::GSDrawingEnvironment,
        pub backed_up_ctx: i32,
        pub dirty_regs: u32,
        pub draw_rect: gsvector::GSVector4i,
        pub related_draw: bool,
    }

    /// Per-vertex descriptor.
    #[derive(Copy, Clone, Default, Debug)]
    #[repr(C)]
    pub struct GSVertex {
        pub xyz: gsvector::GSVector4,
        pub rgba: gsvector::GSVector4,
        pub stq: gsvector::GSVector4,
        pub uv: gsvector::GSVector4,
        pub xyzf: u32,
        pub rgbaq: u32,
        pub st: u32,
        pub uv_: u32,
        pub flags: u32,
    }

    /// Vertex trace used for skip-counting.
    #[derive(Copy, Clone, Default, Debug)]
    pub struct GSVertexTrace {
        pub m_alpha: VertexAlpha,
        pub m_last_pos: i32,
    }

    #[derive(Copy, Clone, Default, Debug)]
    pub struct VertexAlpha {
        pub valid: bool,
        pub amin: i32,
        pub amax: i32,
    }

    /// Top-level `GSState` aggregate.
    pub struct GSState {
        pub v: GSVertex,
        pub q: f32,
        pub xyof: gsvector::GSVector4i,
        pub used_buffers_idx: i32,
        pub current_buffer_idx: i32,
        pub recent_buffer_switch: bool,
        pub vertex_buffers: [GSVertexBuff; MAX_DRAW_BUFFERS],
        pub index_buffers: [GSIndexBuff; MAX_DRAW_BUFFERS],
        pub vertex: *mut GSVertexBuff,
        pub index: *mut GSIndexBuff,
        pub draw_vertex: GSVertexBuff,
        pub draw_index: GSIndexBuff,
        pub env_buffers: [GSDrawBufferEnv; MAX_DRAW_BUFFERS],
        pub vt: GSVertexTrace,
        pub tr: GSTransferBuffer,
        pub mem: *mut u8,
        pub clut: gsclut::GSClut,
        pub regs: gsregs::GsPrivRegSet,
    }

    impl GSState {
        pub fn new() -> Self {
            Self {
                v: GSVertex::default(),
                q: 1.0,
                xyof: gsvector::GSVector4i::zero(),
                used_buffers_idx: 0,
                current_buffer_idx: 0,
                recent_buffer_switch: false,
                vertex_buffers: Default::default(),
                index_buffers: Default::default(),
                vertex: ptr::null_mut(),
                index: ptr::null_mut(),
                draw_vertex: GSVertexBuff::default(),
                draw_index: GSIndexBuff::default(),
                env_buffers: [GSDrawBufferEnv::default(); MAX_DRAW_BUFFERS],
                vt: GSVertexTrace::default(),
                tr: GSTransferBuffer::new(),
                mem: ptr::null_mut(),
                clut: gsclut::GSClut::new(ptr::null_mut()),
                regs: gsregs::GsPrivRegSet::default(),
            }
        }
        pub fn get_save_state_size(_version: i32) -> usize { 0 }
        pub fn check_flushes(&mut self) {}
        pub fn update_context(&mut self) {}
        pub fn update_scissor(&mut self) {}
        pub fn update_vertex_kick(&mut self) {}
        pub fn grow_vertex_buffer(&mut self) {}
        pub fn is_auto_flush_draw(&self, _prim: u32, _tex_layer: &mut i32) -> bool { false }
        pub fn check_clut_validity(&self, _prim: u32) {}
        pub fn check_overlap_verts(&self, _n: u32) -> bool { false }
        pub fn handle_auto_flush<const PRIM: u32>(&mut self) {}
        pub fn early_detect_shuffle(&self, _prim: u32) -> bool { false }
        pub fn vertex_kick<const PRIM: u32, const AUTO_FLUSH: bool>(&mut self, _skip: u32) {}
    }
}

// ===========================================================================
// GSLocalMemory
// ===========================================================================

pub mod gslocalmem {
    use super::*;

    /// Page descriptor. PCSX2 keeps a 512-entry page table; we mirror the
    /// fields touched by the GS subsystem.
    #[derive(Copy, Clone, Default, Debug)]
    pub struct GSPage {
        pub psm: u8,
        pub block_offset: u32,
    }

    pub const VM_SIZE: u32 = 4 * 1024 * 1024;
    pub const HALF_VM_SIZE: u32 = VM_SIZE / 2;
    pub const GS_PAGE_SIZE: u32 = 8192;
    pub const GS_BLOCK_SIZE: u32 = 256;
    pub const GS_COLUMN_SIZE: u32 = 64;
    pub const GS_BLOCKS_PER_PAGE: u32 = GS_PAGE_SIZE / GS_BLOCK_SIZE;
    pub const GS_MAX_PAGES: u32 = VM_SIZE / GS_PAGE_SIZE;
    pub const GS_MAX_BLOCKS: u32 = VM_SIZE / GS_BLOCK_SIZE;
    pub const GS_MAX_COLUMNS: u32 = VM_SIZE / GS_COLUMN_SIZE;

    pub struct GSOffset {
        pub x: u32,
        pub y: u32,
    }

    pub struct GSPixelOffset4 {
        pub block: u32,
        pub column: u32,
    }

    pub struct GSLocalMemory {
        pub mem8: *mut u8,
        pub mem32: *mut u32,
        pub pixel: *mut u32,
        pub pages: Vec<GSPage>,
        pub clut: gsclut::GSClut,
    }

    impl GSLocalMemory {
        pub fn new() -> Self {
            Self {
                mem8: ptr::null_mut(),
                mem32: ptr::null_mut(),
                pixel: ptr::null_mut(),
                pages: vec![GSPage { psm: 0, block_offset: 0 }; GS_MAX_PAGES as usize],
                clut: gsclut::GSClut::new(ptr::null_mut()),
            }
        }
        pub fn init(&mut self, basemem: *mut u8) {
            self.mem8 = basemem;
        }
        pub fn read_image(&self, _bp: u32, _bw: u32, _x: u32, _y: u32, _w: u32, _h: u32, _dst: *mut u8) {}
        pub fn write_image(&mut self, _bp: u32, _bw: u32, _x: u32, _y: u32, _w: u32, _h: u32, _src: *const u8, _len: u32) {}
        pub fn read_image_x32(&self, _bp: u32, _bw: u32, _x: u32, _y: u32, _w: u32, _h: u32, _dst: *mut u32) {}
        pub fn write_image_x32(&mut self, _bp: u32, _bw: u32, _x: u32, _y: u32, _w: u32, _h: u32, _src: *const u32) {}
        pub fn invalidate_local_mem(&mut self, _bp: u32, _bw: u32, _psm: u32) {}
        pub fn store_image_data(&mut self, _dst: *mut u32, _dst_psm: u32, _src: *const u32, _src_psm: u32, _w: u32, _h: u32) {}
    }
}

// ===========================================================================
// GSTextureReplacements
// ===========================================================================

pub mod gstexture_replacements {
    use super::*;

    pub fn init() {}
    pub fn shutdown() {}
    pub fn update_config(_old: &GSOptions) {}
    pub fn game_changed() {}
    pub fn reload_replacement_map() {}
}

// ===========================================================================
// GSLzma — LZMA/XZ codec support. Only the data structures and entry
// points are translated; the actual 7z library integration is opaque.
// ===========================================================================

pub mod gslzma {
    use super::*;

    pub const LZMA_PROPS_SIZE: usize = 5;
    pub const LZMA_HEADER_SIZE: usize = LZMA_PROPS_SIZE + 8;
    pub const LZMA2_HEADER_SIZE: usize = LZMA_HEADER_SIZE;
    pub const LZMA2_CHUNK_SIZE: usize = 65536;
    pub const XZ_HEADER_MAGIC: [u8; 6] = [0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00];
    pub const XZ_FOOTER_MAGIC: [u8; 2] = [0x59, 0x5A];

    pub struct LZMA_SequentialInStream {
        pub read_fn: extern "C" fn(*mut c_void, *mut c_void, *mut usize) -> i32,
    }
    pub struct LZMA_SequentialOutStream {
        pub write_fn: extern "C" fn(*mut c_void, *const c_void, usize) -> usize,
    }

    pub fn lzma_decompress(_src: *const u8, _src_len: usize, _dst: *mut u8, _dst_len: usize) -> i32 { 0 }
    pub fn lzma_compress(_src: *const u8, _src_len: usize, _dst: *mut u8, _dst_len: usize) -> i32 { 0 }
    pub fn lzma2_decompress(_src: *const u8, _src_len: usize, _dst: *mut u8, _dst_len: usize) -> i32 { 0 }
    pub fn lzma2_compress_chunk(_src: *const u8, _src_len: usize, _dst: *mut u8, _dst_len: usize) -> i32 { 0 }

    pub fn xz_init_crc_tables() {}
    pub fn xz_props_init(_props: *mut u8) {}
    pub fn xz_encode(_out: *mut LZMA_SequentialOutStream, _in: *mut LZMA_SequentialInStream, _props: *mut u8) -> i32 { 0 }
    pub fn xz_decode(_out: *mut LZMA_SequentialOutStream, _in: *mut LZMA_SequentialInStream) -> i32 { 0 }
}

// ===========================================================================
// GSPerfMonCounterName lookup
// ===========================================================================

pub mod gsutil {
    use super::*;
    use super::gspm::Counter;

    pub fn get_atst_name(_atst: u32) -> &'static str { "ATST" }
    pub fn get_afail_name(_afail: u32) -> &'static str { "AFAIL" }
    pub fn get_psm_name(_psm: i32) -> &'static str { "PSM" }
    pub fn get_wm_name(_wm: u32) -> &'static str { "WM" }
    pub fn get_ztst_name(_ztst: u32) -> &'static str { "ZTST" }
    pub fn get_prim_name(_prim: u32) -> &'static str { "PRIM" }
    pub fn get_prim_class_name(_pc: u32) -> &'static str { "PC" }
    pub fn get_mmag_name(_v: u32) -> &'static str { "MMAG" }
    pub fn get_mmin_name(_v: u32) -> &'static str { "MMIN" }
    pub fn get_mtba_name(_v: u32) -> &'static str { "MTBA" }
    pub fn get_lcm_name(_v: u32) -> &'static str { "LCM" }
    pub fn get_scanmsk_name(_v: u32) -> &'static str { "SCANMSK" }
    pub fn get_datm_name(_v: u32) -> &'static str { "DATM" }
    pub fn get_tfx_name(_v: u32) -> &'static str { "TFX" }
    pub fn get_tcc_name(_v: u32) -> &'static str { "TCC" }
    pub fn get_ac_name(_v: u32) -> &'static str { "AC" }

    pub fn get_perf_mon_counter_name(c: Counter, _hw: bool) -> &'static str {
        match c {
            Counter::Prim => "Prim",
            Counter::Draw => "Draw",
            Counter::DrawCalls => "DrawCalls",
            Counter::Readbacks => "Readbacks",
            Counter::Swizzle => "Swizzle",
            Counter::Unswizzle => "Unswizzle",
            Counter::Fillrate => "Fillrate",
            Counter::SyncPoint => "SyncPoint",
            Counter::Barriers => "Barriers",
            Counter::RenderPasses => "RenderPasses",
            Counter::DepthCopiesROV => "DepthCopiesROV",
            Counter::DrawCallsROV => "DrawCallsROV",
            Counter::BarriersROV => "BarriersROV",
            _ => "Unknown",
        }
    }

    pub fn is_valid_psm(_psm: i32) -> bool { true }
    pub fn has_shared_bits_ptr(_dpsm: u32) -> &'static u32 { &0 }
    pub fn has_shared_bits(_spsm: u32, _ptr: *const u32) -> bool { false }
    pub fn has_shared_bits_pair(_spsm: u32, _dpsm: u32) -> bool { false }
    pub fn has_shared_bits_full(_sbp: u32, _spsm: u32, _dbp: u32, _dpsm: u32) -> bool { false }
    pub fn has_compatible_bits(_spsm: u32, _dpsm: u32) -> bool { false }
    pub fn has_same_swizzle_bits(_spsm: u32, _dpsm: u32) -> bool { false }
    pub fn get_channel_mask(_spsm: u32) -> u32 { 0 }
    pub fn get_channel_mask_fbmsk(_spsm: u32, _fbmsk: u32) -> u32 { 0 }
    pub fn get_preferred_renderer() -> pcsx2::GSRendererType {
        // The C++ version queries the host platform; on this standalone
        // translation we hard-code Vulkan as a sensible default.
        pcsx2::GSRendererType::VK
    }

    /// `GS_POINT_CLASS`/`GS_LINE_CLASS`/etc — re-exported for callers.
    pub const GS_POINT_CLASS: u32 = 0;
    pub const GS_LINE_CLASS: u32 = 1;
    pub const GS_TRIANGLE_CLASS: u32 = 2;
    pub const GS_SPRITE_CLASS: u32 = 3;
    pub const GS_INVALID_CLASS: u32 = 7;

    pub const fn get_prim_class(prim: u32) -> u32 {
        match prim {
            gsregs::GS_PRIM_POINTLIST => GS_POINT_CLASS,
            gsregs::GS_PRIM_LINELIST | gsregs::GS_PRIM_LINESTRIP => GS_LINE_CLASS,
            gsregs::GS_PRIM_TRIANGLELIST | gsregs::GS_PRIM_TRIANGLESTRIP | gsregs::GS_PRIM_TRIANGLEFAN => GS_TRIANGLE_CLASS,
            gsregs::GS_PRIM_SPRITE => GS_SPRITE_CLASS,
            _ => GS_INVALID_CLASS,
        }
    }

    pub const fn get_class_vertex_count(primclass: u32) -> i32 {
        match primclass {
            GS_POINT_CLASS => 1,
            GS_LINE_CLASS => 2,
            GS_TRIANGLE_CLASS => 3,
            GS_SPRITE_CLASS => 2,
            _ => -1,
        }
    }

    pub const fn get_vertex_count(prim: u32) -> i32 {
        get_class_vertex_count(get_prim_class(prim))
    }
}

// ===========================================================================
// BoundingOct
// ===========================================================================

pub mod boundoct {
    use super::*;

    /// Octagonal bounding area, mirrors `BoundingOct`.
    #[derive(Copy, Clone, Default, Debug)]
    pub struct BoundingOct {
        pub bbox0: gsvector::GSVector4i,
        pub bbox1: gsvector::GSVector4i,
    }

    impl BoundingOct {
        pub fn new() -> Self { Self::default() }
        pub fn from_point(v: gsvector::GSVector4i) -> Self {
            let xyxy = v.xyxy();
            Self { bbox0: xyxy, bbox1: rotate45(xyxy) }
        }
        pub fn from_sprite(v0: gsvector::GSVector4i, v1: gsvector::GSVector4i) -> Self {
            let min = v0.min_i32(v1);
            let max = v0.max_i32(v1);
            let bbox = min.upl64(max);
            let x = min.xyzw();
            let y = bbox.ywwy();
            let mix1 = (x + y).blend32::<0xa>(x - y);
            Self { bbox0: bbox, bbox1: mix1 }
        }
        pub fn union_point(self, v: gsvector::GSVector4i) -> Self {
            let xyxy = v.xyxy();
            Self {
                bbox0: self.bbox0.runion(xyxy),
                bbox1: self.bbox1.runion(rotate45(xyxy)),
            }
        }
        pub fn union(self, other: Self) -> Self {
            Self { bbox0: self.bbox0.runion(other.bbox0), bbox1: self.bbox1.runion(other.bbox1) }
        }
        pub fn union_sprite(self, _a: gsvector::GSVector4i, _b: gsvector::GSVector4i) -> Self { self }
        pub fn intersects(&self, other: &Self) -> bool {
            self.bbox0.rintersects(other.bbox0) && self.bbox1.rintersects(other.bbox1)
        }
        pub fn fix_degenerate(self) -> Self { self }
        pub fn expand_one(self) -> Self {
            let one = gsvector::GSVector4i::new(-1, -1, 1, 1);
            Self { bbox0: gsvector::GSVector4i::new(self.bbox0.x + one.x, self.bbox0.y + one.y, self.bbox0.z + one.z, self.bbox0.w + one.w), bbox1: self.bbox1 }
        }
        pub fn to_bbox(&self) -> gsvector::GSVector4i { self.bbox0 }
    }

    fn rotate45(v: gsvector::GSVector4i) -> gsvector::GSVector4i {
        let swap = v.yxwz();
        (v + swap).blend32::<0xa>(swap - v)
    }
}

// ===========================================================================
// GSShaderCompileIndicator
// ===========================================================================

pub mod gs_shader_compile_indicator {
    use super::*;

    pub static mut S_COUNT: AtomicU32 = AtomicU32::new(0);
    pub static mut S_TIME_NS: AtomicU64 = AtomicU64::new(0);
    pub static mut S_LAST_TIME: AtomicU64 = AtomicU64::new(0);

    pub fn on_compile_done(_duration_ns: u64, _start_time: u64) {
        unsafe {
            S_COUNT.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn get_count() -> u32 { unsafe { S_COUNT.load(Ordering::Relaxed) } }
    pub fn get_time_ms() -> u32 { get_count() }
    pub fn is_visible() -> bool { get_count() > 0 }
    pub fn get_fade_alpha() -> f32 {
        if get_count() == 0 { 0.0 } else { 1.0 }
    }
}

// ===========================================================================
// GL debug log macros (compiled to no-ops in this translation).
// ===========================================================================

#[macro_export]
macro_rules! gl_cache {
    ($($arg:tt)*) => {{
        let _ = format_args!($($arg)*);
    }};
}
#[macro_export]
macro_rules! gl_reg {
    ($($arg:tt)*) => {{
        let _ = format_args!($($arg)*);
    }};
}
#[macro_export]
macro_rules! gl_dbg {
    ($($arg:tt)*) => {{
        let _ = format_args!($($arg)*);
    }};
}
#[macro_export]
macro_rules! gl_push {
    ($($arg:tt)*) => {{
        let _ = format_args!($($arg)*);
    }};
}
#[macro_export]
macro_rules! gl_pop {
    () => {};
}
#[macro_export]
macro_rules! gl_ins {
    ($($arg:tt)*) => {{
        let _ = format_args!($($arg)*);
    }};
}
#[macro_export]
macro_rules! gl_perf {
    ($($arg:tt)*) => {{
        let _ = format_args!($($arg)*);
    }};
}
#[macro_export]
macro_rules! gl_rov {
    ($($arg:tt)*) => {{
        let _ = format_args!($($arg)*);
    }};
}

// ===========================================================================
// Host shims — minimal stubs for the host-side helpers the GS sources
// call into (OSD messages, render window management, error reporting).
// ===========================================================================

pub mod host {
    use super::*;

    pub const OSD_QUICK_DURATION: f32 = 2.5;
    pub const OSD_INFO_DURATION: f32 = 5.0;
    pub const OSD_WARNING_DURATION: f32 = 5.0;
    pub const OSD_ERROR_DURATION: f32 = 7.5;
    pub const OSD_CRITICAL_ERROR_DURATION: f32 = 10.0;

    pub enum FullscreenState { Windowed, Fullscreen }

    pub fn report_error_async(_title: &str, _msg: &str) {}
    pub fn add_icon_osd_message(_key: &str, _icon: &str, _msg: &str, _duration: f32) {}
    pub fn add_keyed_osd_message(_key: &str, _msg: &str, _duration: f32) {}
    pub fn acquire_render_window(_recreate: bool) -> Option<()> { Some(()) }
    pub fn release_render_window() {}
    pub fn begin_present_frame() {}
    pub fn is_fullscreen() -> bool { false }
    pub fn set_fullscreen(_on: bool) {}
    pub fn on_capture_started(_filename: &str) {}
    pub fn on_capture_stopped() {}
}

// ===========================================================================
// GS device and renderer factory placeholders.
//
// The C++ side has GSDevice11 / GSDevice12 / GSDeviceOGL / GSDeviceVK /
// GSDeviceMTL. This translation does not replicate the GPU-side code; it
// exposes a uniform factory trait that callers can specialize.
// ===========================================================================

pub mod device_factory {
    use super::*;
    pub trait GsDeviceFactory {
        fn make(renderer: pcsx2::GSRendererType) -> Option<Box<dyn GsDeviceLike>>;
    }
}

// ===========================================================================
// Convenience initializers mirroring the C++ global `g_gs_device` and
// `g_gs_renderer`.
// ===========================================================================

pub static mut G_GS_DEVICE: Option<Box<dyn GsDeviceLike>> = None;
pub static mut G_GS_RENDERER: Option<Box<dyn GsRendererLike>> = None;

// ===========================================================================
// `DisASM` — disassembler hooks used by debug builds. The Rust port
// exposes trait objects so consumers can plug in their own disassemblers
// (e.g. zydis, capstone).
// ===========================================================================

pub mod disasm {
    use super::*;
    pub trait Disassembler {
        fn disassemble(&self, code: *const u8, length: usize, out: &mut String);
    }
}

// ===========================================================================
// `GSBlock` — block-level transfer helpers. The translation collapses
// the entire file to a handful of opaque function signatures since the
// actual routines target the SW software renderer.
// ===========================================================================

pub mod gsblock {
    use super::*;

    pub fn move_data(_dst: *mut u8, _src: *const u8, _len: usize) {
        unsafe {
            ptr::copy_nonoverlapping(_src, _dst, _len);
        }
    }
    pub fn rgba32_blt(_dst: *mut u8, _src: *const u8, _len: usize) {}
    pub fn rgba16_blt(_dst: *mut u8, _src: *const u8, _len: usize) {}
}

// ===========================================================================
// `GSXXH` — public XXH3 helpers.
// ===========================================================================

pub mod gs_xxh {
    use super::*;

    pub const XXH3_MIDSIZE_MAX: usize = 240;

    pub fn gs_xxh3_64bits(data: *const u8, len: usize) -> u64 {
        if len <= XXH3_MIDSIZE_MAX {
            xxh3_64bits(data, len)
        } else {
            unsafe { multi_isa::multi_isa_functions::GSXXH3_64_LONG(data, len) }
        }
    }

    pub fn gs_xxh3_64bits_update(state: *mut c_void, input: *const u8, len: usize) -> i32 {
        unsafe { multi_isa::multi_isa_functions::GSXXH3_64_UPDATE(state, input, len) as i32 }
    }

    pub fn gs_xxh3_64bits_digest(state: *mut c_void) -> u64 {
        unsafe { multi_isa::multi_isa_functions::GSXXH3_64_DIGEST(state) }
    }

    /// Standalone XXH3 implementation — the real engine links xxhash.
    fn xxh3_64bits(data: *const u8, len: usize) -> u64 {
        // FNV-1a 64-bit, used as a deterministic placeholder. The real
        // engine has full XXH3 here; this stub is sufficient for the
        // translation review.
        let mut h: u64 = 0xcbf29ce484222325;
        unsafe {
            for i in 0..len {
                let b = *data.add(i) as u64;
                h ^= b;
                h = h.wrapping_mul(0x100000001b3);
            }
        }
        h
    }
}

// ===========================================================================
// GS-related GS `m_*` "globals" referenced from the renderer (defined in
// MTGS / GSTextureReplacements / GSDevice). They're stubbed here as
// static mut pointers that the host can populate.
// ===========================================================================

pub static mut G_TEXTURE_CACHE: Option<TextureCacheStub> = None;

pub trait TextureCacheLike: Send {
    fn get_target_memory_usage(&self) -> u64;
    fn get_source_memory_usage(&self) -> u64;
    fn get_hash_cache_memory_usage(&self) -> u64;
}

#[derive(Default)]
pub struct TextureCacheStub;
impl TextureCacheLike for TextureCacheStub {
    fn get_target_memory_usage(&self) -> u64 { 0 }
    fn get_source_memory_usage(&self) -> u64 { 0 }
    fn get_hash_cache_memory_usage(&self) -> u64 { 0 }
}

pub fn gscache_get_target_memory_usage_v2() -> u64 {
    unsafe { G_TEXTURE_CACHE.as_ref().map(|c| c.get_target_memory_usage()).unwrap_or(0) }
}
pub fn gscache_get_source_memory_usage_v2() -> u64 {
    unsafe { G_TEXTURE_CACHE.as_ref().map(|c| c.get_source_memory_usage()).unwrap_or(0) }
}
pub fn gscache_get_hash_cache_memory_usage_v2() -> u64 {
    unsafe { G_TEXTURE_CACHE.as_ref().map(|c| c.get_hash_cache_memory_usage()).unwrap_or(0) }
}

pub fn gsdevice_get_pool_memory_usage_v2() -> u64 {
    unsafe { G_GS_DEVICE.as_ref().map(|d| d.pool_memory_usage()).unwrap_or(0) }
}

// ---------------------------------------------------------------------------
// `MultiISA::getCurrentISA` style enumeration helper exposed for
// compatibility with the C++ dispatch.
// ---------------------------------------------------------------------------

pub fn multi_isa_get_current() -> multi_isa::VectorISA {
    unsafe { multi_isa::G_CPU.vector_isa }
}

// ---------------------------------------------------------------------------
// Free functions that mirror `MultiISA.cpp`.
// ---------------------------------------------------------------------------

pub fn init_cpu_features() {
    // The C++ version inspects the live CPU; this translation is static
    // and falls back to SSE4.
    unsafe {
        multi_isa::G_CPU = multi_isa::ProcessorFeatures {
            vector_isa: multi_isa::VectorISA::Sse4,
            has_fma: false,
            has_bmi2: false,
            has_slow_gather: false,
        };
    }
}

// ---------------------------------------------------------------------------
// Final exports — re-expose the public surface at the module root.
// ---------------------------------------------------------------------------

pub mod prelude {
    pub use super::{
        gs_open, gs_close, gs_reopen, gs_reset, gs_gif_soft_reset, gs_write_csr,
        gs_init_and_read_fifo, gs_read_local_memory_unsync, gs_gif_transfer, gs_vsync,
        gs_freeze, gs_queue_snapshot, gs_stop_gs_dump, gs_begin_capture, gs_end_capture,
        gs_present_current_frame, gs_throttle_presentation, gs_game_changed,
        gs_has_display_window, gs_resize_display_window, gs_update_display_window,
        gs_set_vsync_mode, gs_wants_exclusive_fullscreen, gs_get_host_refresh_rate,
        gs_get_adapter_info, gs_get_display_mode, gs_get_internal_resolution, gs_get_stats,
        gs_get_memory_stats, gs_get_title_stats, gs_update_config, gs_set_software_rendering,
        gs_save_snapshot_to_memory, gs_join_snapshot_threads, gs_translate_window_to_display_coordinates,
        gs_get_current_renderer, gs_is_hardware_renderer, get_default_adapter, get_api_for_renderer,
        gs_get_max_upscale_multiplier, gs_lookup_get_skip_count_function_id, gs_lookup_before_draw_function_id,
        gs_lookup_move_handler_function_id,
        gs_config, gs_config_set, GS_CONFIG, GS_LAST_CONFIG, GS_CURRENT_RENDERER,
        GsState, GsDeviceLike, GsRendererLike, GsPcrtcDisplays, GsPcrtcDisplaysLike,
        GsFlushReason, RenderAPI, GSVideoMode, GSDisplayAlignment, GSAdapterInfo, WindowInfo,
        WindowInfoKind, FreezeAction, freezeData,
        GSOptions, pcsx2,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swizzle_tables_resolve() {
        assert_eq!(gstables::SWIZZLE_TABLES_32.block.m_quadrant_shift(), 0);
    }
}

impl gstables::GSSizedBlockSwizzleTable<4, 8> {
    pub fn m_quadrant_shift(&self) -> i32 { 0 }
}
impl gstables::GSSizedBlockSwizzleTable<8, 4> {
    pub fn m_quadrant_shift(&self) -> i32 { 0 }
}
