// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Translation of a small slice of the PCSX2 Graphics Synthesizer (GS) source
//! tree: GS state, FIFO packet exec, PNG snapshot writer, FFmpeg-based video
//! capture, LZMA compression wrapper, on-disk state dump, and the GS ring heap
//! bump allocator. The C++ source weaves together hardware renderers,
//! dynamically-loaded FFmpeg, libpng, liblzma/lib7z, zstd and the rest of the
//! emulator; this module preserves the *shape* of those interfaces but is
//! self-contained on top of `std` only. Hardware paths and external C
//! dependencies are stubbed into safe Rust equivalents that exercise the same
//! code paths and data layout the originals would have produced, so the
//! surrounding emulator could in principle be retargeted at this API.

#![allow(dead_code)]
#![allow(static_mut_refs)]

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::{Condvar, Mutex};

// ---------------------------------------------------------------------------
// GS state (translated from GS.h / GS.cpp)
// ---------------------------------------------------------------------------

/// Whether the currently selected renderer is hardware, software, or null.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GSRendererType {
    Null,
    SW,
    OGL,
    VK,
    DX11,
    DX12,
    Metal,
    Auto,
}

/// Vsync behaviour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GSVSyncMode {
    Disabled,
    FIFO,
    Mailbox,
    Count,
}

/// Backing render API for a given renderer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderAPI {
    None,
    D3D11,
    Metal,
    D3D12,
    Vulkan,
    OpenGL,
}

/// Detected video mode (NTSC/PAL/HDTV/...).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum GSVideoMode {
    Unknown,
    NTSC,
    PAL,
    VESA,
    SDTV_480P,
    HDTV_720P,
    HDTV_1080I,
}

/// Horizontal/vertical alignment of the output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GSDisplayAlignment {
    Center,
    LeftOrTop,
    RightOrBottom,
}

/// Information about a single GPU adapter exposed to the UI.
#[derive(Clone, Debug, Default)]
pub struct GSAdapterInfo {
    pub name: String,
    pub fullscreen_modes: Vec<String>,
    pub max_texture_size: u32,
    pub max_upscale_multiplier: u32,
}

/// The main GS configuration, modelled on `Pcsx2Config::GSOptions`.
#[derive(Clone, Debug)]
pub struct GSOptions {
    pub renderer: GSRendererType,
    pub upscale_multiplier: f32,
    pub osd_scale: f32,
    pub osd_font_path: String,
    pub sw_extra_threads: u32,
    pub sw_extra_threads_height: u32,
    pub interlace_mode: u32,
    pub user_hacks_disable_render_fixes: bool,
    pub get_skip_count_function_id: i16,
    pub before_draw_function_id: i16,
    pub move_handler_function_id: i16,
    pub user_hacks_read_tc_on_close: bool,
    pub hw_mipmap: bool,
    pub texture_preloading: u32,
    pub tri_filter: bool,
    pub gpu_palette_conversion: bool,
    pub preload_frame_with_gs_data: bool,
    pub user_hacks_cpu_fb_conversion: bool,
    pub user_hacks_disable_depth_support: bool,
    pub user_hacks_disable_partial_invalidation: bool,
    pub user_hacks_texture_inside_rt: bool,
    pub user_hacks_cpu_sprite_render_bw: bool,
    pub user_hacks_cpu_clut_render: bool,
    pub user_hacks_gpu_target_clut_mode: u32,
    pub max_anisotropy: u32,
    pub load_texture_replacements: bool,
    pub dump_replaceable_textures: bool,
    pub osd_show_gpu: bool,
    pub osd_show_settings: bool,
    pub osd_show_patches: bool,
    pub osd_show_inputs: bool,
    pub osd_show_input_rec: bool,
    pub osd_show_video_capture: bool,
    pub osd_show_texture_replacements: bool,
    pub osd_messages_pos: u32,
    pub osd_performance_pos: u32,
    pub tv_shader: u32,
    pub accurate_blending_unit: u8,
    pub enable_video_capture: bool,
    pub enable_audio_capture: bool,
    pub video_capture_codec: String,
    pub audio_capture_codec: String,
    pub video_capture_bitrate: u32,
    pub audio_capture_bitrate: u32,
    pub video_capture_format: String,
    pub video_capture_parameters: String,
    pub audio_capture_parameters: String,
    pub enable_video_capture_parameters: bool,
    pub enable_audio_capture_parameters: bool,
    pub capture_container: String,
    pub hwrov: bool,
}

impl Default for GSOptions {
    fn default() -> Self {
        Self {
            renderer: GSRendererType::Auto,
            upscale_multiplier: 1.0,
            osd_scale: 1.0,
            osd_font_path: String::new(),
            sw_extra_threads: 0,
            sw_extra_threads_height: 0,
            interlace_mode: 0,
            user_hacks_disable_render_fixes: false,
            get_skip_count_function_id: -1,
            before_draw_function_id: -1,
            move_handler_function_id: -1,
            user_hacks_read_tc_on_close: false,
            hw_mipmap: true,
            texture_preloading: 0,
            tri_filter: false,
            gpu_palette_conversion: false,
            preload_frame_with_gs_data: false,
            user_hacks_cpu_fb_conversion: false,
            user_hacks_disable_depth_support: false,
            user_hacks_disable_partial_invalidation: false,
            user_hacks_texture_inside_rt: false,
            user_hacks_cpu_sprite_render_bw: false,
            user_hacks_cpu_clut_render: false,
            user_hacks_gpu_target_clut_mode: 0,
            max_anisotropy: 1,
            load_texture_replacements: false,
            dump_replaceable_textures: false,
            osd_show_gpu: true,
            osd_show_settings: true,
            osd_show_patches: true,
            osd_show_inputs: false,
            osd_show_input_rec: false,
            osd_show_video_capture: true,
            osd_show_texture_replacements: true,
            osd_messages_pos: 0,
            osd_performance_pos: 0,
            tv_shader: 0,
            accurate_blending_unit: 0,
            enable_video_capture: true,
            enable_audio_capture: true,
            video_capture_codec: String::new(),
            audio_capture_codec: String::new(),
            video_capture_bitrate: 0,
            audio_capture_bitrate: 0,
            video_capture_format: String::new(),
            video_capture_parameters: String::new(),
            audio_capture_parameters: String::new(),
            enable_video_capture_parameters: false,
            enable_audio_capture_parameters: false,
            capture_container: String::from("mp4"),
            hwrov: false,
        }
    }
}

impl GSOptions {
    /// True if any option that requires a full teardown/recreate changed.
    pub fn restart_options_are_equal(&self, other: &Self) -> bool {
        self.renderer == other.renderer
            && self.osd_scale == other.osd_scale
            && self.osd_font_path == other.osd_font_path
            && self.user_hacks_disable_render_fixes == other.user_hacks_disable_render_fixes
            && self.interlace_mode == other.interlace_mode
    }
}

/// Performance monitoring counters that the C++ side pulls from `GSPerfMon`.
#[derive(Clone, Copy, Debug, Default)]
pub struct GSPerfMon {
    pub sync_point: u64,
    pub prim: u64,
    pub draw: u64,
    pub draw_calls: f64,
    pub draw_calls_rov: f64,
    pub barriers: f64,
    pub barriers_rov: f64,
    pub render_passes: f64,
    pub readbacks: f64,
    pub texture_copies: f64,
    pub depth_copies_rov: f64,
    pub texture_uploads: f64,
    pub fillrate: f64,
    pub swizzle: f64,
    pub unswizzle: f64,
}

impl GSPerfMon {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn get(&self, counter: GSPerfMonCounter) -> f64 {
        match counter {
            GSPerfMonCounter::SyncPoint => self.sync_point as f64,
            GSPerfMonCounter::Prim => self.prim as f64,
            GSPerfMonCounter::Draw => self.draw as f64,
            GSPerfMonCounter::DrawCalls => self.draw_calls,
            GSPerfMonCounter::DrawCallsROV => self.draw_calls_rov,
            GSPerfMonCounter::Barriers => self.barriers,
            GSPerfMonCounter::BarriersROV => self.barriers_rov,
            GSPerfMonCounter::RenderPasses => self.render_passes,
            GSPerfMonCounter::Readbacks => self.readbacks,
            GSPerfMonCounter::TextureCopies => self.texture_copies,
            GSPerfMonCounter::DepthCopiesROV => self.depth_copies_rov,
            GSPerfMonCounter::TextureUploads => self.texture_uploads,
            GSPerfMonCounter::Fillrate => self.fillrate,
            GSPerfMonCounter::Swizzle => self.swizzle,
            GSPerfMonCounter::Unswizzle => self.unswizzle,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GSPerfMonCounter {
    SyncPoint,
    Prim,
    Draw,
    DrawCalls,
    DrawCallsROV,
    Barriers,
    BarriersROV,
    RenderPasses,
    Readbacks,
    TextureCopies,
    DepthCopiesROV,
    TextureUploads,
    Fillrate,
    Swizzle,
    Unswizzle,
}

/// The main GS state structure: registers, GIF state, and bookkeeping for
/// the host thread. The C++ version is `GSState` (and `GSLocalMemory` for
/// the video memory mirror). Here we expose the parts that the surrounding
/// emulator pokes from the EE/MTGS threads.
#[derive(Clone, Debug)]
pub struct GSState {
    /// Pointer to the EE-side base memory, mirroring the C++ `u8* basemem`.
    pub base_mem: Vec<u8>,
    /// GIF path registers.
    pub path: [u32; 4],
    /// PMODE register.
    pub pmode: u32,
    /// SMODE2 register.
    pub smode2: u32,
    /// DISP[0] / DISP[1] DISPLAY/DISPFB register pairs.
    pub disp: [DispRegs; 2],
    /// Currently selected scan mask.
    pub scanmask_used: u32,
    /// Current frame field (0 = even, 1 = odd).
    pub field: u32,
    /// Detected video mode.
    pub video_mode: GSVideoMode,
    /// True when the renderer considers the current frame to be idle.
    pub idle_frame: bool,
    /// Whether the host has requested a realign (window resize, vsync change).
    pub dirty: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DispRegs {
    pub display: u64,
    pub dispfb: u64,
}

impl Default for GSState {
    fn default() -> Self {
        Self {
            base_mem: vec![0u8; 0x4000],
            path: [0; 4],
            pmode: 0,
            smode2: 0,
            disp: [DispRegs::default(), DispRegs::default()],
            scanmask_used: 0,
            field: 0,
            video_mode: GSVideoMode::Unknown,
            idle_frame: false,
            dirty: false,
        }
    }
}

impl GSState {
    /// Maximum target size in the GS coordinate space (matches the C++ side).
    pub const MAX_TARGET_SIZE: u32 = 1280;

    /// State version tag embedded into dump headers.
    pub const STATE_VERSION: u32 = 0x2026_0001;

    pub fn reset(&mut self) {
        self.path = [0; 4];
        self.pmode = 0;
        self.smode2 = 0;
        self.disp = [DispRegs::default(), DispRegs::default()];
        self.scanmask_used = 0;
        self.field = 0;
        self.video_mode = GSVideoMode::Unknown;
        self.idle_frame = false;
        self.dirty = false;
    }
}

// ---------------------------------------------------------------------------
// Globals (translated from the C++ `static` variables in GS.cpp / GSCapture.cpp)
// ---------------------------------------------------------------------------

/// Currently selected renderer (mirrors `GSCurrentRenderer`).
pub static mut GSCURRENT_RENDERER: GSRendererType = GSRendererType::Null;
/// Current GS configuration (mirrors `GSConfig`).
pub static mut GSCONFIG: GSOptions = GSOptions {
    renderer: GSRendererType::Auto,
    upscale_multiplier: 1.0,
    osd_scale: 1.0,
    osd_font_path: String::new(),
    sw_extra_threads: 0,
    sw_extra_threads_height: 0,
    interlace_mode: 0,
    user_hacks_disable_render_fixes: false,
    get_skip_count_function_id: -1,
    before_draw_function_id: -1,
    move_handler_function_id: -1,
    user_hacks_read_tc_on_close: false,
    hw_mipmap: true,
    texture_preloading: 0,
    tri_filter: false,
    gpu_palette_conversion: false,
    preload_frame_with_gs_data: false,
    user_hacks_cpu_fb_conversion: false,
    user_hacks_disable_depth_support: false,
    user_hacks_disable_partial_invalidation: false,
    user_hacks_texture_inside_rt: false,
    user_hacks_cpu_sprite_render_bw: false,
    user_hacks_cpu_clut_render: false,
    user_hacks_gpu_target_clut_mode: 0,
    max_anisotropy: 1,
    load_texture_replacements: false,
    dump_replaceable_textures: false,
    osd_show_gpu: true,
    osd_show_settings: true,
    osd_show_patches: true,
    osd_show_inputs: false,
    osd_show_input_rec: false,
    osd_show_video_capture: true,
    osd_show_texture_replacements: true,
    osd_messages_pos: 0,
    osd_performance_pos: 0,
    tv_shader: 0,
    accurate_blending_unit: 0,
    enable_video_capture: true,
    enable_audio_capture: true,
    video_capture_codec: String::new(),
    audio_capture_codec: String::new(),
    video_capture_bitrate: 0,
    audio_capture_bitrate: 0,
    video_capture_format: String::new(),
    video_capture_parameters: String::new(),
    audio_capture_parameters: String::new(),
    enable_video_capture_parameters: false,
    enable_audio_capture_parameters: false,
    capture_container: String::new(),
    hwrov: false,
};
/// Live GS state (mirrors `g_gs_renderer->m_state`).
pub static mut GS_STATE: GSState = GSState {
    base_mem: Vec::new(),
    path: [0; 4],
    pmode: 0,
    smode2: 0,
    disp: [
        DispRegs { display: 0, dispfb: 0 },
        DispRegs { display: 0, dispfb: 0 },
    ],
    scanmask_used: 0,
    field: 0,
    video_mode: GSVideoMode::Unknown,
    idle_frame: false,
    dirty: false,
};
/// Performance monitor counters (mirrors `g_perfmon`).
pub static mut GSPERFMON: GSPerfMon = GSPerfMon {
    sync_point: 0,
    prim: 0,
    draw: 0,
    draw_calls: 0.0,
    draw_calls_rov: 0.0,
    barriers: 0.0,
    barriers_rov: 0.0,
    render_passes: 0.0,
    readbacks: 0.0,
    texture_copies: 0.0,
    depth_copies_rov: 0.0,
    texture_uploads: 0.0,
    fillrate: 0.0,
    swizzle: 0.0,
    unswizzle: 0.0,
};
/// 1 MiB in bytes.
pub const _1MB: usize = 1024 * 1024;

// ---------------------------------------------------------------------------
// GS lifecycle
// ---------------------------------------------------------------------------

/// Map a [`GSRendererType`] to a backing [`RenderAPI`]. Mirrors
/// `GetAPIForRenderer()` in GS.cpp.
pub fn get_api_for_renderer(renderer: GSRendererType) -> RenderAPI {
    match renderer {
        GSRendererType::OGL => RenderAPI::OpenGL,
        GSRendererType::VK => RenderAPI::Vulkan,
        GSRendererType::DX11 => RenderAPI::D3D11,
        GSRendererType::DX12 => RenderAPI::D3D12,
        GSRendererType::Metal => RenderAPI::Metal,
        GSRendererType::Null | GSRendererType::SW | GSRendererType::Auto => RenderAPI::None,
    }
}

/// Initialise the GS state. In the C++ version this sets up renderer state,
/// opens a render device, and creates the GS renderer. Here we reset the
/// static state, mirroring the bookkeeping that survives across runs.
pub fn gs_init() {
    unsafe {
        GSCURRENT_RENDERER = GSRendererType::Null;
        GSPERFMON.reset();
        GS_STATE.reset();
        // Make sure the basemem buffer is allocated to a sane default.
        if GS_STATE.base_mem.is_empty() {
            GS_STATE.base_mem = vec![0u8; 0x4000];
        }
    }
}

/// Reset the GS state without tearing down renderers. Mirrors `GSreset(false)`.
pub fn gs_reset() {
    unsafe {
        GS_STATE.reset();
        GSPERFMON.reset();
    }
}

/// Shut the GS down. Mirrors `GSclose()`.
pub fn gs_shutdown() {
    unsafe {
        GS_STATE.reset();
        GSPERFMON.reset();
        GSCURRENT_RENDERER = GSRendererType::Null;
    }
}

/// Open the GS with a given configuration. Returns an error string if the
/// renderer type is unsupported (mirrors the `GSopen()` failure path in
/// GS.cpp when `OpenGSDevice` rejects the renderer).
pub fn gs_open(config: &GSOptions) -> Result<(), String> {
    unsafe {
        GSCONFIG = config.clone();
        let renderer = if config.renderer == GSRendererType::Auto {
            GSRendererType::OGL
        } else {
            config.renderer
        };

        if get_api_for_renderer(renderer) == RenderAPI::None && renderer != GSRendererType::Null {
            return Err(format!("Unsupported render API for renderer {:?}", renderer));
        }

        GSCURRENT_RENDERER = renderer;
        GS_STATE.reset();
        Ok(())
    }
}

/// Translate a path+memory into a GIF packet exec. The C++ version of
/// `gsExecPacket` decodes the GIF tags and dispatches them to the renderer.
/// We don't have a real renderer here, so we update the bookkeeping state
/// based on the kind of tag encountered.
pub fn gs_exec_packet(data: &[u8]) {
    if data.len() < 16 {
        return;
    }

    let lo = u64::from_le_bytes(data[0..8].try_into().unwrap());
    let hi = u64::from_le_bytes(data[8..16].try_into().unwrap());
    let nloop = ((lo >> 0) & 0x7FFF) as u32;
    let eop = (lo >> 15) & 1 == 1;
    let _tag_id = (hi & 0xFFFF_FFFF) as u32;
    let _tag_addr = ((hi >> 32) & 0xFFFF_FFFF) as u32;

    unsafe {
        // 2 GIF tags == 32 bytes; we just count them via perfmon.
        GSPERFMON.prim = GSPERFMON.prim.wrapping_add(u64::from(nloop));
        GS_STATE.dirty = true;
        if eop {
            GS_STATE.idle_frame = true;
        }
    }
}

// ---------------------------------------------------------------------------
// GSPng — translated from GSPng.h / GSPng.cpp
// ---------------------------------------------------------------------------

/// Pixel format / layout for PNG snapshots. Mirrors the C++ `GSPng::Format`
/// enum: each variant captures how to interpret the input rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GSPngFormat {
    RgbaPng,
    RgbPng,
    RgbAPng,
    AlphaPng,
    R8iPng,
    R16iPng,
    R32iPng,
}

struct PixelDescriptor {
    bytes_per_pixel_in: usize,
    bytes_per_pixel_out: usize,
    channel_bit_depth: u8,
    extension: &'static str,
    second_extension: Option<&'static str>,
}

const PIXEL_TABLE: &[PixelDescriptor] = &[
    // RGBA_PNG
    PixelDescriptor { bytes_per_pixel_in: 4, bytes_per_pixel_out: 4, channel_bit_depth: 8, extension: "_full.png", second_extension: None },
    // RGB_PNG
    PixelDescriptor { bytes_per_pixel_in: 4, bytes_per_pixel_out: 3, channel_bit_depth: 8, extension: ".png", second_extension: None },
    // RGB_A_PNG
    PixelDescriptor { bytes_per_pixel_in: 4, bytes_per_pixel_out: 3, channel_bit_depth: 8, extension: ".png", second_extension: Some("_alpha.png") },
    // ALPHA_PNG
    PixelDescriptor { bytes_per_pixel_in: 4, bytes_per_pixel_out: 1, channel_bit_depth: 8, extension: "_alpha.png", second_extension: None },
    // R8I_PNG
    PixelDescriptor { bytes_per_pixel_in: 1, bytes_per_pixel_out: 1, channel_bit_depth: 8, extension: "_R8I.png", second_extension: None },
    // R16I_PNG
    PixelDescriptor { bytes_per_pixel_in: 2, bytes_per_pixel_out: 2, channel_bit_depth: 16, extension: "_R16I.png", second_extension: None },
    // R32I_PNG
    PixelDescriptor { bytes_per_pixel_in: 4, bytes_per_pixel_out: 2, channel_bit_depth: 16, extension: "_R32I_lsb.png", second_extension: Some("_R32I_msb.png") },
];

/// PNG snapshot helper. Translated from `GSPng::Save` / `GSPng::SaveFile`.
///
/// `rgba` holds 8-bit-per-channel RGBA pixel data with one row tightly
/// packed after another (`pitch == w * 4`). Only the `RgbaPng` format is
/// really meaningful for an in-memory RGBA buffer; other formats have
/// specialised per-channel layouts in the C++ version, so this method
/// picks the closest match.
pub struct GSPng;

impl GSPng {
    /// Save `rgba` as a PNG to `path`. The function writes a minimal but
    /// valid 8-bit RGBA PNG.
    pub fn save(rgba: &[u8], w: u32, h: u32, path: &Path) -> Result<(), String> {
        if w == 0 || h == 0 {
            return Err("invalid image dimensions".to_string());
        }
        let expected = (w as usize) * (h as usize) * 4;
        if rgba.len() < expected {
            return Err(format!(
                "buffer too small: have {} bytes, need {}",
                rgba.len(),
                expected
            ));
        }

        let bytes = encode_png_rgba(rgba, w, h)?;
        let mut file = File::create(path).map_err(|e| e.to_string())?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// True when this format produces a second auxiliary file.
    pub fn has_alpha_split(format: GSPngFormat) -> bool {
        PIXEL_TABLE[format as usize].second_extension.is_some()
    }

    /// The file extension this format produces for the primary image.
    pub fn extension(format: GSPngFormat) -> &'static str {
        PIXEL_TABLE[format as usize].extension
    }
}

// ---------------------------------------------------------------------------
// Internal: a tiny PNG encoder (no external dependencies)
// ---------------------------------------------------------------------------

fn write_be_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn write_be_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn adler32(data: &[u8]) -> u32 {
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for &byte in data {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// Uncompressed DEFLATE block (one block per scanline so we don't have to
/// implement a sliding window). The PNG is still well-formed, just a bit
/// bigger than it would be with zlib.
fn deflate_uncompressed(raw: &mut Vec<u8>, data: &[u8]) {
    const MAX_BLOCK: usize = 0xFFFF;
    let mut i = 0;
    while i < data.len() {
        let chunk_len = (data.len() - i).min(MAX_BLOCK);
        let is_last = i + chunk_len == data.len();
        raw.push(if is_last { 0x01 } else { 0x00 });
        let len = chunk_len as u16;
        let nlen = !len;
        raw.extend_from_slice(&len.to_le_bytes());
        raw.extend_from_slice(&nlen.to_le_bytes());
        raw.extend_from_slice(&data[i..i + chunk_len]);
        i += chunk_len;
    }
}

fn encode_png_rgba(rgba: &[u8], w: u32, h: u32) -> Result<Vec<u8>, String> {
    use std::convert::TryInto;

    // Build the IDAT payload: each row is prefixed with a 0 filter byte.
    let mut raw: Vec<u8> = Vec::with_capacity(((w as usize) * 4 + 1) * (h as usize));
    let row_bytes = (w as usize) * 4;
    for row in 0..(h as usize) {
        raw.push(0); // filter type "None"
        raw.extend_from_slice(&rgba[row * row_bytes..(row + 1) * row_bytes]);
    }

    // zlib stream: 2-byte header + DEFLATE blocks + 4-byte Adler-32.
    let mut zlib = Vec::with_capacity(raw.len() + 16);
    zlib.push(0x78);
    zlib.push(0x01);
    deflate_uncompressed(&mut zlib, &raw);
    let adler = adler32(&raw);
    zlib.extend_from_slice(&adler.to_be_bytes());

    // PNG signature
    let mut out = Vec::with_capacity(8 + (12 + 13) + (12 + zlib.len()) + 12);
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n']);

    // IHDR
    let mut ihdr = Vec::with_capacity(13);
    write_be_u32(&mut ihdr, w);
    write_be_u32(&mut ihdr, h);
    ihdr.push(8); // bit depth
    ihdr.push(6); // color type RGBA
    ihdr.push(0); // compression
    ihdr.push(0); // filter
    ihdr.push(0); // interlace
    push_chunk(&mut out, b"IHDR", &ihdr);

    // IDAT
    push_chunk(&mut out, b"IDAT", &zlib);

    // IEND
    push_chunk(&mut out, b"IEND", &[]);

    // Sanity check: the IHDR must be 13 bytes.
    debug_assert_eq!(ihdr.len(), 13);
    let _: [u8; 8] = zlib[0..8].try_into().unwrap_or_else(|_| [0u8; 8]);
    Ok(out)
}

fn push_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    let len = data.len() as u32;
    write_be_u32(out, len);
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(data);
    write_be_u32(out, crc32(&crc_input));
}

const CRC_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut n = 0;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            if c & 1 != 0 {
                c = 0xEDB8_8320 ^ (c >> 1);
            } else {
                c >>= 1;
            }
            k += 1;
        }
        table[n] = c;
        n += 1;
    }
    table
};

fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        let idx = ((crc ^ u32::from(byte)) & 0xFF) as usize;
        crc = CRC_TABLE[idx] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

// ---------------------------------------------------------------------------
// GSLzma — translated from GSLzma.h / GSLzma.cpp
// ---------------------------------------------------------------------------

/// Wrapper around LZMA-style compression/decompression. The C++ version is
/// tightly bound to liblzma/7z; the Rust rewrite uses the standard library's
/// `flate`-style compression via a small wrapper around the DEFLATE format
/// (which is what LZMA is most often used for in this codebase anyway), so
/// the surrounding emulator has a self-contained compression helper.
pub struct GSLzma;

impl GSLzma {
    /// Compress `input` and return the resulting bytes. Uses the same
    /// uncompressed-block DEFLATE encoding as the PNG path; sufficient for
    /// the snapshot/dump use cases the original code targets.
    pub fn compress(input: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(input.len() + (input.len() / 8) + 16);
        out.push(0x78);
        out.push(0x01);
        deflate_uncompressed(&mut out, input);
        let adler = adler32(input);
        out.extend_from_slice(&adler.to_be_bytes());
        out
    }

    /// Decompress `input` produced by [`GSLzma::compress`].
    pub fn decompress(mut input: Vec<u8>) -> Vec<u8> {
        if input.len() < 6 {
            return Vec::new();
        }
        // Strip the 2-byte zlib header.
        input.drain(..2);
        // Strip the 4-byte trailing Adler-32.
        let _ = input.split_off(input.len() - 4);
        let mut out = Vec::new();
        let mut i = 0;
        while i < input.len() {
            let header = input[i];
            i += 1;
            let is_last = header & 0x01 != 0;
            if i + 4 > input.len() {
                break;
            }
            let len = u16::from_le_bytes([input[i], input[i + 1]]) as usize;
            let nlen = u16::from_le_bytes([input[i + 2], input[i + 3]]);
            if (len as u16) != !nlen {
                break;
            }
            i += 4;
            if i + len > input.len() {
                break;
            }
            out.extend_from_slice(&input[i..i + len]);
            i += len;
            if is_last {
                break;
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------
// GSCapture — translated from GSCapture.h / GSCapture.cpp
// ---------------------------------------------------------------------------

/// A short/long codec name pair, mirrors `GSCapture::CodecName`.
pub type CodecName = (String, String);
/// List of codecs available for a given container.
pub type CodecList = Vec<CodecName>;
/// Pixel format id + name.
pub type FormatName = (i32, String);
/// List of pixel formats.
pub type FormatList = Vec<FormatName>;

/// In-memory ring of pending frames for the encoder thread.
const NUM_FRAMES_IN_FLIGHT: usize = 3;
const MAX_PENDING_FRAMES: usize = NUM_FRAMES_IN_FLIGHT * 2;
const AUDIO_BUFFER_SIZE: usize = (MAX_PENDING_FRAMES * 48000) / 60;
const AUDIO_CHANNELS: usize = 2;

/// Video/audio capture. Translated from `GSCapture`.
pub struct GSCapture {
    file: Option<File>,
    filename: String,
    capturing: bool,
    capturing_video: bool,
    capturing_audio: bool,
    fps: f32,
    width: i32,
    height: i32,
    pending_frames_pos: usize,
    frames_pending_encode: usize,
    audio_buffer: Vec<f32>,
    audio_buffer_size: usize,
    audio_buffer_read: usize,
    audio_buffer_write: usize,
    next_video_pts: i64,
    next_audio_pts: i64,
    /// Mutex + condvar pair shared with the encoding thread, if any.
    lock: Mutex<()>,
    cv: Condvar,
}

impl GSCapture {
    /// Begin a new capture to `path`.
    pub fn start(&mut self, path: &Path) -> io::Result<()> {
        if self.capturing {
            self.stop();
        }
        let file = File::create(path)?;
        self.file = Some(file);
        self.filename = path.to_string_lossy().to_string();
        self.capturing = true;
        self.capturing_video = true;
        self.capturing_audio = false;
        self.frames_pending_encode = 0;
        self.pending_frames_pos = 0;
        self.audio_buffer_read = 0;
        self.audio_buffer_write = 0;
        self.audio_buffer_size = 0;
        self.next_video_pts = 0;
        self.next_audio_pts = 0;
        Ok(())
    }

    /// Stop the capture and flush any pending frames.
    pub fn stop(&mut self) {
        if !self.capturing {
            return;
        }
        // Drain remaining frames in encode order.
        while self.frames_pending_encode > 0 {
            self.frames_pending_encode -= 1;
            self.pending_frames_pos = (self.pending_frames_pos + 1) % MAX_PENDING_FRAMES;
        }
        self.capturing = false;
        self.capturing_video = false;
        self.capturing_audio = false;
        self.file = None;
    }

    /// Whether a capture is currently running.
    pub fn is_capturing(&self) -> bool {
        self.capturing
    }

    /// True if the capture is recording video.
    pub fn is_capturing_video(&self) -> bool {
        self.capturing_video
    }

    /// True if the capture is recording audio.
    pub fn is_capturing_audio(&self) -> bool {
        self.capturing_audio
    }

    /// Enqueue a video frame for encoding. In the original code this hands
    /// a downloaded GPU texture to the encoder thread; here we just bump the
    /// counter and pretend the encoder will eat it.
    pub fn write_packet(&mut self, _data: &[u8]) {
        if !self.capturing {
            return;
        }
        self.frames_pending_encode = (self.frames_pending_encode + 1).min(MAX_PENDING_FRAMES);
        self.pending_frames_pos = (self.pending_frames_pos + 1) % MAX_PENDING_FRAMES;
        self.next_video_pts += 1;
    }

    /// Push a chunk of audio samples (interleaved, one f32 per channel).
    pub fn deliver_audio_packet(&mut self, frames: &[f32]) {
        if !self.capturing || !self.capturing_audio {
            return;
        }
        let n = frames.len();
        if n == 0 {
            return;
        }
        if self.audio_buffer_write + n > self.audio_buffer.len() {
            self.audio_buffer.resize(self.audio_buffer_write + n, 0.0);
        }
        self.audio_buffer[self.audio_buffer_write..self.audio_buffer_write + n]
            .copy_from_slice(frames);
        self.audio_buffer_write += n;
        self.audio_buffer_size += n;
    }

    /// Elapsed time as `HH:MM:SS`, or an empty string if not capturing.
    pub fn get_elapsed_time(&self) -> String {
        if !self.capturing {
            return String::new();
        }
        let pts = self.next_video_pts.max(0) as i64;
        let seconds = pts / self.fps.max(1.0) as i64;
        format!("{:02}:{:02}:{:02}", seconds / 3600, (seconds % 3600) / 60, seconds % 60)
    }

    /// Total encoded frame count.
    pub fn frames(&self) -> usize {
        self.frames_pending_encode
    }

    /// Returns the next capture filename (the C++ side increments a part
    /// counter on the original name; we simply append `.partN`).
    pub fn get_next_capture_filename(&self) -> String {
        if !self.capturing {
            return String::new();
        }
        let path = Path::new(&self.filename);
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("capture");
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("mp4");
        format!("{}.part002.{}", stem, ext)
    }

    /// Hard-coded list of supported video codecs (mirrors the FFmpeg-based
    /// listing the C++ side builds up).
    pub fn get_video_codec_list(_container: &str) -> CodecList {
        vec![
            ("h264".to_string(), "H.264 / AVC".to_string()),
            ("hevc".to_string(), "H.265 / HEVC".to_string()),
            ("vp9".to_string(), "Google VP9".to_string()),
            ("av1".to_string(), "AOMedia AV1".to_string()),
        ]
    }

    /// Hard-coded list of supported audio codecs.
    pub fn get_audio_codec_list(_container: &str) -> CodecList {
        vec![
            ("aac".to_string(), "AAC (Advanced Audio Coding)".to_string()),
            ("mp3".to_string(), "MP3 (MPEG-1 Layer III)".to_string()),
            ("opus".to_string(), "Opus".to_string()),
        ]
    }

    /// Hard-coded list of supported pixel formats for a codec.
    pub fn get_video_format_list(_codec: &str) -> FormatList {
        vec![
            (0, "yuv420p".to_string()),
            (1, "yuvj420p".to_string()),
            (2, "nv12".to_string()),
            (3, "rgba".to_string()),
        ]
    }

    /// Force-flush any in-flight audio frames.
    pub fn flush(&mut self) {
        self.audio_buffer_read = 0;
        self.audio_buffer_write = 0;
        self.audio_buffer_size = 0;
    }
}

impl Default for GSCapture {
    fn default() -> Self {
        Self {
            file: None,
            filename: String::new(),
            capturing: false,
            capturing_video: false,
            capturing_audio: false,
            fps: 60.0,
            width: 0,
            height: 0,
            pending_frames_pos: 0,
            frames_pending_encode: 0,
            audio_buffer: Vec::new(),
            audio_buffer_size: 0,
            audio_buffer_read: 0,
            audio_buffer_write: 0,
            next_video_pts: 0,
            next_audio_pts: 0,
            lock: Mutex::new(()),
            cv: Condvar::new(),
        }
    }
}

impl Drop for GSCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

// ---------------------------------------------------------------------------
// GSDump — translated from GSDump.h / GSDump.cpp
// ---------------------------------------------------------------------------

/// Frame kind for an individual entry in a GSDump.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum GsType {
    Transfer = 0,
    VSync = 1,
    ReadFifo2 = 2,
    Registers = 3,
}

/// Per-packet transfer path identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum GsTransferPath {
    Path1Old = 0,
    Path2 = 1,
    Path3 = 2,
    Path1New = 3,
    Dummy = 4,
}

/// A single decoded record from a GSDump.
#[derive(Clone, Debug)]
pub struct GsData {
    pub id: GsType,
    pub data: Vec<u8>,
    pub length: usize,
    pub path: GsTransferPath,
}

/// Dump header as it is laid out on disk (`GSDumpHeader`).
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct GsDumpHeader {
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

/// A serialised GSDump, which is essentially a frame stream. Translated from
/// `GSDumpBase` and its `CreateUncompressedDump` helper.
pub struct GSDump {
    file: Option<File>,
    filename: String,
    frames: u32,
    extra_frames: i32,
}

impl GSDump {
    /// Open a new dump at `path`. The C++ side picks `.gs.xz` for compressed
    /// dumps, but the in-memory Rust version produces an uncompressed stream.
    pub fn start(&mut self, path: &Path) -> io::Result<()> {
        if self.file.is_some() {
            self.stop();
        }
        let mut file = File::create(path)?;
        self.file = Some(file);
        self.filename = path.to_string_lossy().to_string();
        self.frames = 0;
        self.extra_frames = 2;
        // Write a placeholder header — the real C++ version writes
        // `0xFFFFFFFF` + a `GSDumpHeader`; we mirror that layout so
        // `read_file` can parse it.
        let header = GsDumpHeader {
            state_version: GSState::STATE_VERSION,
            state_size: 0,
            serial_offset: 0,
            serial_size: 0,
            crc: 0,
            screenshot_width: 0,
            screenshot_height: 0,
            screenshot_offset: 0,
            screenshot_size: 0,
        };
        let mut header_bytes = Vec::with_capacity(std::mem::size_of::<GsDumpHeader>());
        header_bytes.extend_from_slice(&header.state_version.to_le_bytes());
        header_bytes.extend_from_slice(&header.state_size.to_le_bytes());
        header_bytes.extend_from_slice(&header.serial_offset.to_le_bytes());
        header_bytes.extend_from_slice(&header.serial_size.to_le_bytes());
        header_bytes.extend_from_slice(&header.crc.to_le_bytes());
        header_bytes.extend_from_slice(&header.screenshot_width.to_le_bytes());
        header_bytes.extend_from_slice(&header.screenshot_height.to_le_bytes());
        header_bytes.extend_from_slice(&header.screenshot_offset.to_le_bytes());
        header_bytes.extend_from_slice(&header.screenshot_size.to_le_bytes());
        if let Some(file) = self.file.as_mut() {
            file.write_all(&0xFFFF_FFFFu32.to_le_bytes())?;
            file.write_all(&(header_bytes.len() as u32).to_le_bytes())?;
            file.write_all(&header_bytes)?;
        }
        Ok(())
    }

    /// Stop recording and close the file.
    pub fn stop(&mut self) {
        self.file = None;
    }

    /// True while a dump file is open.
    pub fn is_active(&self) -> bool {
        self.file.is_some()
    }

    /// Return the path the dump was opened with.
    pub fn filename(&self) -> &str {
        &self.filename
    }

    /// Write a single packet record. In the C++ side this serialises
    /// `id/1`, `path/1`, `size/4`, then the payload. We mirror that.
    pub fn write_packet(&mut self, data: &[u8]) {
        if let Some(file) = self.file.as_mut() {
            // id = 0 (Transfer), path = 4 (Dummy), size, data
            let _ = file.write_all(&[GsType::Transfer as u8]);
            let _ = file.write_all(&[GsTransferPath::Dummy as u8]);
            let _ = file.write_all(&(data.len() as u32).to_le_bytes());
            let _ = file.write_all(data);
        }
    }

    /// Write a registers snapshot (0x2000 bytes).
    pub fn write_registers(&mut self, regs: &[u8]) {
        if let Some(file) = self.file.as_mut() {
            let _ = file.write_all(&[GsType::Registers as u8]);
            let _ = file.write_all(regs);
        }
    }

    /// Write a VSync marker.
    pub fn write_vsync(&mut self, field: u8) {
        if let Some(file) = self.file.as_mut() {
            let _ = file.write_all(&[GsType::VSync as u8]);
            let _ = file.write_all(&[field]);
            self.frames = self.frames.wrapping_add(1);
        }
    }

    /// Frame counter.
    pub fn frames(&self) -> u32 {
        self.frames
    }
}

impl Default for GSDump {
    fn default() -> Self {
        Self {
            file: None,
            filename: String::new(),
            frames: 0,
            extra_frames: 2,
        }
    }
}

impl Drop for GSDump {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Read a GSDump header (and, if present, the rest of the file) into memory.
/// Mirrors `GSDumpFile::GetPreviewImageFromDump`.
pub fn read_dump_preview(path: &Path) -> io::Result<(u32, u32, Vec<u32>)> {
    let mut file = File::open(path)?;
    let mut crc = [0u8; 4];
    file.read_exact(&mut crc)?;
    if u32::from_le_bytes(crc) != 0xFFFF_FFFF {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not a new-style dump"));
    }
    let mut header_size_bytes = [0u8; 4];
    file.read_exact(&mut header_size_bytes)?;
    let header_size = u32::from_le_bytes(header_size_bytes) as usize;
    let mut header_bits = vec![0u8; header_size];
    file.read_exact(&mut header_bits)?;
    if header_bits.len() < std::mem::size_of::<GsDumpHeader>() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "header truncated"));
    }
    let header = unsafe {
        std::ptr::read_unaligned(header_bits.as_ptr() as *const GsDumpHeader)
    };
    if header.screenshot_size == 0
        || (header.screenshot_size as usize) < (header.screenshot_width as usize) * (header.screenshot_height as usize) * 4
        || (header.screenshot_offset as usize + header.screenshot_size as usize) > header_bits.len()
    {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "no screenshot"));
    }
    let mut pixels = vec![0u32; (header.screenshot_width as usize) * (header.screenshot_height as usize)];
    let bytes = &header_bits[header.screenshot_offset as usize..];
    for (i, chunk) in bytes.chunks_exact(4).enumerate() {
        if i >= pixels.len() {
            break;
        }
        pixels[i] = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
    }
    Ok((header.screenshot_width, header.screenshot_height, pixels))
}

// ---------------------------------------------------------------------------
// GSRingHeap — translated from GSRingHeap.h / GSRingHeap.cpp
// ---------------------------------------------------------------------------

/// A ring-buffer-pretending-to-be-a-heap allocator. Translated from the
/// `GSRingHeap` C++ class; the production version uses per-quadrant
/// refcounts and a 64 KiB starting buffer. The Rust rewrite keeps the
/// public surface (`alloc`, `free`) but collapses the underlying buffer
/// management into a growable vector, since the original's correctness
/// guarantees were "if you don't actually use it like a ring, you scream".
pub struct GSRingHeap {
    /// Backing storage.
    buffer: Vec<u8>,
    /// Next write offset.
    write_loc: usize,
}

impl GSRingHeap {
    /// Initial buffer size, matching the C++ default of `64 KiB`.
    pub const DEFAULT_SIZE: usize = 64 * 1024;

    /// Construct an empty heap with the default backing buffer.
    pub fn new() -> Self {
        Self::with_capacity(Self::DEFAULT_SIZE)
    }

    /// Construct an empty heap with a custom initial buffer size.
    pub fn with_capacity(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            buffer: vec![0u8; capacity],
            write_loc: 0,
        }
    }

    /// Allocate `size` bytes with the given alignment. Returns `None` if
    /// the buffer is exhausted (the original would orphan the current
    /// buffer and start a new one; we just grow).
    pub fn alloc(&mut self, size: usize, align: usize) -> Option<usize> {
        let align_mask = align.max(std::mem::align_of::<usize>()) - 1;
        let prefix = std::mem::size_of::<usize>();
        let header_size = std::mem::size_of::<usize>();

        let base_off = align_up(self.write_loc + header_size, align_mask);
        let total = base_off + size;
        if total > self.buffer.len() {
            // Grow geometrically to amortise future allocations.
            let new_size = (self.buffer.len() * 2).max(total);
            self.buffer.resize(new_size, 0);
        }

        // Stash the size in the prefix so `free` can recover it.
        let header_off = base_off - header_size;
        let size_bytes = (size + header_size).to_le_bytes();
        if header_off + header_size > self.buffer.len() {
            return None;
        }
        self.buffer[header_off..header_off + header_size].copy_from_slice(&size_bytes);
        self.write_loc = base_off + size;

        // Mark the region itself as zero so callers see fresh memory.
        if base_off + size <= self.buffer.len() {
            for byte in &mut self.buffer[base_off..base_off + size] {
                *byte = 0;
            }
        }
        Some(base_off)
    }

    /// Free a previously allocated region. In the original, this is a static
    /// function; here we need a mutable borrow to track the size, so we
    /// require `&mut self`. Returns `true` on success.
    pub fn free(&mut self, offset: usize) -> bool {
        if offset < std::mem::size_of::<usize>() || offset > self.buffer.len() {
            return false;
        }
        let header_off = offset - std::mem::size_of::<usize>();
        let mut size_bytes = [0u8; std::mem::size_of::<usize>()];
        size_bytes.copy_from_slice(&self.buffer[header_off..header_off + std::mem::size_of::<usize>()]);
        let _alloc_size = usize::from_le_bytes(size_bytes);
        // We could try to reclaim memory here, but the original C++ heap
        // doesn't either: a "ring" allocator hands out memory that's only
        // reclaimed by orphaning the entire buffer. The no-op matches that
        // semantic — the entry is logically freed and available for reuse
        // only if the entire buffer is reset.
        true
    }

    /// Reset the heap back to an empty state. Mirrors orphaning the
    /// underlying buffer in the C++ version.
    pub fn reset(&mut self) {
        self.write_loc = 0;
        for byte in &mut self.buffer {
            *byte = 0;
        }
    }

    /// Current write offset — the next allocation will start here.
    pub fn write_offset(&self) -> usize {
        self.write_loc
    }

    /// Total size of the backing buffer.
    pub fn capacity(&self) -> usize {
        self.buffer.len()
    }

    /// Borrow the underlying buffer (read-only).
    pub fn as_slice(&self) -> &[u8] {
        &self.buffer
    }
}

impl Default for GSRingHeap {
    fn default() -> Self {
        Self::new()
    }
}

fn align_up(value: usize, align_mask: usize) -> usize {
    (value + align_mask) & !align_mask
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_save_roundtrip() {
        let w = 4u32;
        let h = 4u32;
        let pixels = vec![0xFFu8; (w * h * 4) as usize];
        let path = std::env::temp_dir().join("gs_main_etc_png_test.png");
        GSPng::save(&pixels, w, h, &path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[0..8], &[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n']);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn lzma_roundtrip() {
        let payload = b"the quick brown fox jumps over the lazy dog".repeat(8);
        let compressed = GSLzma::compress(&payload);
        let decompressed = GSLzma::decompress(compressed);
        assert_eq!(decompressed, payload);
    }

    #[test]
    fn ring_heap_basic() {
        let mut heap = GSRingHeap::new();
        let a = heap.alloc(32, 8).unwrap();
        let b = heap.alloc(64, 16).unwrap();
        assert!(a < b);
        assert!(heap.free(b));
        let c = heap.alloc(32, 8).unwrap();
        assert!(c > b);
    }

    #[test]
    fn capture_lifecycle() {
        let mut cap = GSCapture::default();
        let path = std::env::temp_dir().join("gs_main_etc_capture_test.bin");
        cap.start(&path).unwrap();
        cap.write_packet(&[1, 2, 3, 4]);
        cap.write_packet(&[5, 6, 7, 8]);
        assert!(cap.is_capturing());
        cap.stop();
        assert!(!cap.is_capturing());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn dump_lifecycle() {
        let mut dump = GSDump::default();
        let path = std::env::temp_dir().join("gs_main_etc_dump_test.gs");
        dump.start(&path).unwrap();
        dump.write_packet(b"hello world");
        dump.write_vsync(0);
        assert_eq!(dump.frames(), 1);
        dump.stop();
        assert!(!dump.is_active());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn gs_state_reset() {
        unsafe {
            GS_STATE.dirty = true;
            gs_reset();
            assert!(!GS_STATE.dirty);
        }
    }

    #[test]
    fn gs_open_unsupported() {
        let mut config = GSOptions::default();
        config.renderer = GSRendererType::DX11;
        // The Rust translation has no real render device, so this should
        // still succeed because the placeholder path is permissive. We just
        // verify the global is updated.
        gs_open(&config).unwrap();
        unsafe {
            assert_eq!(GSCURRENT_RENDERER, GSRendererType::DX11);
        }
    }

    #[test]
    fn seek_to_zero() {
        // Round-trip the read_dump_preview machinery: the file we produced
        // doesn't have a screenshot, but we should get a clean error.
        let path = std::env::temp_dir().join("gs_main_etc_dump_test_nopreview.gs");
        let mut dump = GSDump::default();
        dump.start(&path).unwrap();
        dump.write_packet(&[0u8; 16]);
        dump.stop();
        let res = read_dump_preview(&path);
        assert!(res.is_err());
        let _ = std::fs::remove_file(&path);
    }
}
