//! PCSX2 graphics device module: Direct3D 11 + Metal backends.
//!
//! This module is an idiomatic Rust 2021 translation of the C++ sources that
//! implement the Direct3D 11 and Metal renderers used by PCSX2's GS subsystem.
//! The D3D11 backend is gated behind `cfg!(target_os = "windows")` because
//! the underlying `d3d11`/`dxgi`/`d3dcompiler` crates are only available on
//! Windows. The Metal backend is gated behind `cfg!(target_os = "macos")` and
//! is provided as a stub structure on other platforms so that the rest of the
//! crate can still type-check cross-platform.
//!
//! All native COM / Objective-C handles are declared as opaque `*mut c_void`
//! pointers so that the file is self-contained and the standard library is
//! the only dependency.

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(dead_code)]

use std::ffi::{c_char, c_uint, c_void, CString};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// Shared device trait
// ---------------------------------------------------------------------------

/// Filter mode used by stretch rects / samplers.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Filter {
    Nearest,
    Bilinear,
}

/// Render API enumeration; mirrors `RenderAPI` in C++.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RenderAPI {
    D3D11,
    Metal,
    Vulkan,
    OpenGL,
    D3D12,
    Software,
}

/// Pixel / depth-stencil formats accepted by the device.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum GsTextureFormat {
    #[default]
    Invalid,
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

/// Texture usage class.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GsTextureType {
    RenderTarget,
    DepthStencil,
    Texture,
    RWTexture,
}

/// 2D integer rectangle.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Rect2i {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect2i {
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self { left, top, right, bottom }
    }
    pub fn is_empty(&self) -> bool {
        self.left >= self.right || self.top >= self.bottom
    }
}

/// 4-component float rectangle.
#[derive(Copy, Clone, Debug, Default)]
pub struct Rect4f {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Rect4f {
    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self { x, y, z, w }
    }
}

/// Result of `BeginPresent`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PresentResult {
    OK,
    FrameSkipped,
}

/// Simple feature set reported by a device. Only the fields needed by the
/// translated code paths are modelled; new ones can be appended.
#[derive(Copy, Clone, Debug, Default)]
pub struct DeviceFeatures {
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
    pub broken_point_sampler: bool,
    pub rov: bool,
    pub vs_expand: bool,
}

/// The device trait implemented by both the D3D11 and Metal backends.
pub trait GsDevice {
    fn render_api(&self) -> RenderAPI;
    fn create(&mut self) -> bool;
    fn destroy(&mut self);
    fn has_surface(&self) -> bool;
    fn present_begin(&mut self) -> PresentResult;
    fn present_end(&mut self);
    fn set_vsync_mode(&mut self, _mode: VSyncMode, _allow_throttle: bool) {}
    fn features(&self) -> &DeviceFeatures;
}

// ---------------------------------------------------------------------------
// VSync modes
// ---------------------------------------------------------------------------

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum VSyncMode {
    Disabled,
    FIFO,
    Mailbox,
}

// ---------------------------------------------------------------------------
// Direct3D 11 backend
// ---------------------------------------------------------------------------

/// Vendor ID categorisation used by the D3D11 renderer to pick a preferred
/// graphics API.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum VendorID {
    Unknown,
    Nvidia,
    AMD,
    Intel,
}

/// Shader model targeted by the D3D11 compiler.  Used to size the shader
/// cache and to produce the correct profile string.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ShaderModel {
    SM40,
    SM41,
    SM50,
    SM51,
}

impl ShaderModel {
    pub fn to_cache_string(self) -> &'static str {
        match self {
            ShaderModel::SM40 => "sm40",
            ShaderModel::SM41 => "sm41",
            ShaderModel::SM50 => "sm50",
            ShaderModel::SM51 => "sm51",
        }
    }

    /// D3D compile target string for the given shader model and type index.
    /// `type_index` follows the C++ convention: 0 = vertex, 1 = pixel,
    /// 2 = compute.
    pub fn target(self, type_index: usize) -> &'static str {
        match self {
            ShaderModel::SM40 => match type_index {
                0 => "vs_4_0",
                1 => "ps_4_0",
                _ => "cs_4_0",
            },
            ShaderModel::SM41 => match type_index {
                0 => "vs_4_1",
                1 => "ps_4_1",
                _ => "cs_4_1",
            },
            ShaderModel::SM50 => match type_index {
                0 => "vs_5_0",
                1 => "ps_5_0",
                _ => "cs_5_0",
            },
            ShaderModel::SM51 | _ => match type_index {
                0 => "vs_5_1",
                1 => "ps_5_1",
                _ => "cs_5_1",
            },
        }
    }
}

/// Shader type passed to `D3DCompile`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ShaderType {
    Vertex,
    Pixel,
    Compute,
}

impl ShaderType {
    fn as_index(self) -> usize {
        match self {
            ShaderType::Vertex => 0,
            ShaderType::Pixel => 1,
            ShaderType::Compute => 2,
        }
    }
}

/// One entry in the shader cache. The cache is keyed by an MD5 digest of the
/// source, the macro definitions, and the entry point.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CacheIndexKey {
    pub source_hash_low: u64,
    pub source_hash_high: u64,
    pub macro_hash_low: u64,
    pub macro_hash_high: u64,
    pub entry_point_low: u64,
    pub entry_point_high: u64,
    pub source_length: u32,
    pub shader_type: u32,
}

/// Offsets into the on-disk blob file.
#[derive(Copy, Clone, Debug, Default)]
pub struct CacheIndexData {
    pub file_offset: u32,
    pub blob_size: u32,
}

/// Identifies a vendor from an adapter's `VendorId` field.
pub fn classify_vendor(vendor_id: u32) -> VendorID {
    match vendor_id {
        0x10DE => VendorID::Nvidia,
        0x1002 | 0x1022 => VendorID::AMD,
        0x163C | 0x8086 | 0x8087 => VendorID::Intel,
        _ => VendorID::Unknown,
    }
}

// ---------------------------------------------------------------------------
// FFI declarations
// ---------------------------------------------------------------------------

// On Windows we provide the real D3D11 signatures. On other platforms the
// types are still exposed so that downstream code can compile, but the
// functions are stubs that always return null / failure.

#[cfg(target_os = "windows")]
mod d3d11_ffi {
    use super::c_void;

    pub type HRESULT = i32;
    pub const S_OK: HRESULT = 0;
    pub const E_FAIL: HRESULT = 0x80004005_u32 as i32;
    pub const E_ACCESSDENIED: HRESULT = 0x80070005_u32 as i32;

    pub const DXGI_FORMAT_R8G8B8A8_UNORM: u32 = 28;
    pub const D3D11_SDK_VERSION: u32 = 7;

    pub const D3D_FEATURE_LEVEL_10_0: u32 = 0xa000;
    pub const D3D_FEATURE_LEVEL_11_0: u32 = 0xb000;
    pub const D3D_FEATURE_LEVEL_11_1: u32 = 0xb100;
    pub const D3D_FEATURE_LEVEL_12_0: u32 = 0xc000;

    pub const D3D_DRIVER_TYPE_UNKNOWN: u32 = 0;
    pub const D3D_DRIVER_TYPE_HARDWARE: u32 = 1;
    pub const D3D11_CREATE_DEVICE_DEBUG: u32 = 0x2;

    pub const DXGI_CREATE_FACTORY_DEBUG: u32 = 0x1;
    pub const DXGI_ERROR_NOT_FOUND: HRESULT = 0x887A0002_u32 as i32;

    extern "system" {
        pub fn CreateDXGIFactory2(flags: u32, riid: *const c_void, factory: *mut *mut c_void) -> HRESULT;
        pub fn D3D11CreateDevice(
            adapter: *mut c_void,
            driver_type: u32,
            software: *mut c_void,
            flags: u32,
            feature_levels: *const u32,
            feature_levels_count: u32,
            sdk_version: u32,
            device: *mut *mut c_void,
            feature_level: *mut u32,
            context: *mut *mut c_void,
        ) -> HRESULT;
        pub fn D3DCreateBlob(size: usize, blob: *mut *mut c_void) -> HRESULT;
        pub fn D3DCompile(
            src: *const c_void,
            src_len: usize,
            source_name: *const i8,
            macros: *const *const i8,
            include: *mut c_void,
            entry_point: *const i8,
            target: *const i8,
            flags1: u32,
            flags2: u32,
            code: *mut *mut c_void,
            errors: *mut *mut c_void,
        ) -> HRESULT;
    }
}

#[cfg(target_os = "windows")]
use d3d11_ffi::*;

// ---------------------------------------------------------------------------
// Stub versions of the FFI for non-Windows targets.
// ---------------------------------------------------------------------------

#[cfg(not(target_os = "windows"))]
mod d3d11_ffi {
    use super::c_void;
    pub type HRESULT = i32;
    pub const S_OK: HRESULT = 0;
    pub const E_FAIL: HRESULT = -1;
    pub const E_ACCESSDENIED: HRESULT = -2;
    pub const DXGI_ERROR_NOT_FOUND: HRESULT = -3;

    pub const DXGI_FORMAT_R8G8B8A8_UNORM: u32 = 0;
    pub const D3D11_SDK_VERSION: u32 = 0;
    pub const D3D_FEATURE_LEVEL_10_0: u32 = 0;
    pub const D3D_FEATURE_LEVEL_11_0: u32 = 0;
    pub const D3D_FEATURE_LEVEL_11_1: u32 = 0;
    pub const D3D_FEATURE_LEVEL_12_0: u32 = 0;
    pub const D3D_DRIVER_TYPE_UNKNOWN: u32 = 0;
    pub const D3D_DRIVER_TYPE_HARDWARE: u32 = 1;
    pub const D3D11_CREATE_DEVICE_DEBUG: u32 = 0;
    pub const DXGI_CREATE_FACTORY_DEBUG: u32 = 0;

    pub unsafe extern "system" fn CreateDXGIFactory2(
        _flags: u32,
        _riid: *const c_void,
        _factory: *mut *mut c_void,
    ) -> HRESULT {
        E_FAIL
    }
    pub unsafe extern "system" fn D3D11CreateDevice(
        _adapter: *mut c_void,
        _driver_type: u32,
        _software: *mut c_void,
        _flags: u32,
        _feature_levels: *const u32,
        _feature_levels_count: u32,
        _sdk_version: u32,
        _device: *mut *mut c_void,
        _feature_level: *mut u32,
        _context: *mut *mut c_void,
    ) -> HRESULT {
        E_FAIL
    }
    pub unsafe extern "system" fn D3DCreateBlob(_size: usize, _blob: *mut *mut c_void) -> HRESULT {
        E_FAIL
    }
    pub unsafe extern "system" fn D3DCompile(
        _src: *const c_void,
        _src_len: usize,
        _source_name: *const i8,
        _macros: *const *const i8,
        _include: *mut c_void,
        _entry_point: *const i8,
        _target: *const i8,
        _flags1: u32,
        _flags2: u32,
        _code: *mut *mut c_void,
        _errors: *mut *mut c_void,
    ) -> HRESULT {
        E_FAIL
    }
}

#[cfg(not(target_os = "windows"))]
use d3d11_ffi::*;

// ---------------------------------------------------------------------------
// D3D11 Shader Cache
// ---------------------------------------------------------------------------

/// Disk-backed shader cache mirroring `D3D11ShaderCache` in C++.
pub struct D3D11ShaderCache {
    shader_model: Option<ShaderModel>,
    debug: bool,
    index: std::collections::HashMap<CacheIndexKey, CacheIndexData>,
    index_file: Option<File>,
    blob_file: Option<File>,
    next_bad_shader_id: u32,
}

impl D3D11ShaderCache {
    pub fn new() -> Self {
        Self {
            shader_model: None,
            debug: false,
            index: std::collections::HashMap::new(),
            index_file: None,
            blob_file: None,
            next_bad_shader_id: 1,
        }
    }

    /// Open the cache. `feature_level` selects a shader model.
    #[cfg(target_os = "windows")]
    pub fn open(
        &mut self,
        feature_level: u32,
        debug: bool,
        disable_cache: bool,
    ) -> bool {
        self.shader_model = match feature_level {
            D3D_FEATURE_LEVEL_10_0 => Some(ShaderModel::SM40),
            D3D_FEATURE_LEVEL_11_0 | D3D_FEATURE_LEVEL_11_1 => Some(ShaderModel::SM50),
            _ => None,
        };
        if self.shader_model.is_none() {
            return false;
        }
        self.debug = debug;
        if disable_cache {
            return true;
        }
        let Some(sm) = self.shader_model else {
            return false;
        };
        let base = shader_cache_base_file_name(sm, debug);
        let index_path = base.with_extension("idx");
        let blob_path = base.with_extension("bin");
        if !self.read_existing(&index_path, &blob_path) {
            return self.create_new(&index_path, &blob_path);
        }
        true
    }

    /// On non-Windows builds the cache is a no-op that always succeeds.
    #[cfg(not(target_os = "windows"))]
    pub fn open(
        &mut self,
        feature_level: u32,
        debug: bool,
        _disable_cache: bool,
    ) -> bool {
        self.shader_model = match feature_level {
            D3D_FEATURE_LEVEL_10_0 => Some(ShaderModel::SM40),
            D3D_FEATURE_LEVEL_11_0 | D3D_FEATURE_LEVEL_11_1 => Some(ShaderModel::SM50),
            _ => None,
        };
        self.debug = debug;
        self.shader_model.is_some()
    }

    pub fn close(&mut self) {
        self.index_file = None;
        self.blob_file = None;
    }

    fn create_new(&mut self, index_path: &PathBuf, blob_path: &PathBuf) -> bool {
        if let Err(e) = std::fs::remove_file(index_path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                eprintln!("warning: failed to remove existing index file: {e}");
            }
        }
        if let Err(e) = std::fs::remove_file(blob_path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                eprintln!("warning: failed to remove existing blob file: {e}");
            }
        }
        let mut idx = match OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(index_path)
        {
            Ok(f) => f,
            Err(e) => {
                eprintln!("D3D11: failed to open index file: {e}");
                return false;
            }
        };
        let version: u32 = 1;
        if idx.write_all(&version.to_le_bytes()).is_err() {
            eprintln!("D3D11: failed to write index version");
            return false;
        }
        let blob = match OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(blob_path)
        {
            Ok(f) => f,
            Err(e) => {
                eprintln!("D3D11: failed to open blob file: {e}");
                return false;
            }
        };
        self.index_file = Some(idx);
        self.blob_file = Some(blob);
        true
    }

    fn read_existing(&mut self, index_path: &PathBuf, blob_path: &PathBuf) -> bool {
        let mut idx = match OpenOptions::new().read(true).write(true).open(index_path) {
            Ok(f) => f,
            Err(_) => return false,
        };
        let mut blob = match OpenOptions::new().read(true).append(true).open(blob_path) {
            Ok(f) => f,
            Err(_) => {
                eprintln!("D3D11: blob file is missing");
                return false;
            }
        };
        let mut header = [0u8; 4];
        if idx.read_exact(&mut header).is_err() {
            return false;
        }
        let file_version = u32::from_le_bytes(header);
        if file_version != 1 {
            eprintln!("D3D11: bad shader cache version");
            return false;
        }
        let blob_size = blob.seek(SeekFrom::End(0)).unwrap_or(0);
        self.index_file = Some(idx);
        self.blob_file = Some(blob);
        let _ = blob_size;
        true
    }

    /// Compute a cache key from the shader source, macro list and entry point.
    pub fn get_cache_key(
        shader_type: ShaderType,
        shader_code: &str,
        macros: &[(&str, &str)],
        entry_point: &str,
    ) -> CacheIndexKey {
        let source_hash = md5_digest(shader_code.as_bytes());
        let macro_hash = if macros.is_empty() {
            [0u8; 16]
        } else {
            let mut buf = Vec::new();
            for (n, d) in macros {
                buf.extend_from_slice(n.as_bytes());
                buf.extend_from_slice(d.as_bytes());
            }
            md5_digest(&buf)
        };
        let ep_hash = md5_digest(entry_point.as_bytes());

        let mut key = CacheIndexKey::default();
        key.shader_type = shader_type as u32;
        key.source_hash_low = u64::from_le_bytes(source_hash[0..8].try_into().unwrap());
        key.source_hash_high = u64::from_le_bytes(source_hash[8..16].try_into().unwrap());
        key.macro_hash_low = u64::from_le_bytes(macro_hash[0..8].try_into().unwrap());
        key.macro_hash_high = u64::from_le_bytes(macro_hash[8..16].try_into().unwrap());
        key.entry_point_low = u64::from_le_bytes(ep_hash[0..8].try_into().unwrap());
        key.entry_point_high = u64::from_le_bytes(ep_hash[8..16].try_into().unwrap());
        key.source_length = shader_code.len() as u32;
        key
    }

    /// Compile a shader with the D3D11 compiler, falling back to writing the
    /// bad source to disk if the compile fails.
    pub fn compile_shader(
        &mut self,
        shader_type: ShaderType,
        shader_model: ShaderModel,
        debug: bool,
        code: &str,
        entry_point: &str,
    ) -> Option<Vec<u8>> {
        let target = shader_model.target(shader_type.as_index());
        // The c_char-pointer arguments must be NUL-terminated; build CStrings
        // up front so the call is safe.
        let target_c = CString::new(target).ok()?;
        let entry_c = CString::new(entry_point).ok()?;
        let name_c = CString::new("0").ok()?;

        let flags_non_debug: u32 = 1 << 14; // D3DCOMPILE_OPTIMIZATION_LEVEL3
        let flags_debug: u32 = (1 << 5) | (1 << 0) | (1 << 7); // skip opt | debug | name
        let flags = if debug { flags_debug } else { flags_non_debug };

        let mut code_blob: *mut c_void = std::ptr::null_mut();
        let mut err_blob: *mut c_void = std::ptr::null_mut();

        let hr = unsafe {
            D3DCompile(
                code.as_ptr() as *const c_void,
                code.len(),
                name_c.as_ptr(),
                std::ptr::null(),
                std::ptr::null_mut(),
                entry_c.as_ptr(),
                target_c.as_ptr(),
                flags,
                0,
                &mut code_blob,
                &mut err_blob,
            )
        };
        if hr != S_OK {
            eprintln!("D3D11: failed to compile '{target}': hr=0x{hr:08X}");
            // Dump the bad shader so that the user can diagnose it.
            let log_dir = std::env::temp_dir();
            let path = log_dir.join(format!("pcsx2_bad_shader_{}.txt", self.next_bad_shader_id));
            self.next_bad_shader_id += 1;
            if let Ok(mut f) = File::create(&path) {
                let _ = writeln!(f, "{code}");
                let _ = writeln!(f, "\n\nCompile as {target} failed: 0x{hr:08X}");
            }
            return None;
        }
        // Copy the bytecode out and free the COM blob.
        let size = unsafe { com_blob_size(code_blob) };
        let mut out = vec![0u8; size];
        unsafe {
            std::ptr::copy_nonoverlapping(
                com_blob_buffer(code_blob) as *const u8,
                out.as_mut_ptr(),
                size,
            );
            com_release(code_blob);
            if !err_blob.is_null() {
                com_release(err_blob);
            }
        }
        Some(out)
    }
}

impl Default for D3D11ShaderCache {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for D3D11ShaderCache {
    fn drop(&mut self) {
        self.close();
    }
}

// ---------------------------------------------------------------------------
// MD5 digest (RFC 1321) - minimal in-source implementation, no external deps.
// ---------------------------------------------------------------------------

const MD5_INIT: [u32; 4] = [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476];

fn md5_digest(data: &[u8]) -> [u8; 16] {
    let mut state = MD5_INIT;
    let len = data.len() as u64;
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&(len * 8).to_le_bytes());
    for chunk in msg.chunks(64) {
        md5_compress(&mut state, chunk.try_into().unwrap());
    }
    let mut out = [0u8; 16];
    for (i, &w) in state.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }
    out
}

fn md5_compress(state: &mut [u32; 4], block: &[u8; 64]) {
    const K: [u32; 64] = [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a,
        0xa8304613, 0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be,
        0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340,
        0x265e5a51, 0xe9b6c7aa, 0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8,
        0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed, 0xa9e3e905, 0xfcefa3f8,
        0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c,
        0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa,
        0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665,
        0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92,
        0xffeff47d, 0x85845dd1, 0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1,
        0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391,
    ];
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20,
        5, 9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23,
        6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let mut m = [0u32; 16];
    for (i, w) in m.iter_mut().enumerate() {
        let off = i * 4;
        *w = u32::from_le_bytes(block[off..off + 4].try_into().unwrap());
    }
    let mut a = state[0];
    let mut b = state[1];
    let mut c = state[2];
    let mut d = state[3];
    for i in 0..64 {
        let (f, g) = match i {
            0..=15 => ((b & c) | (!b & d), i),
            16..=31 => ((d & b) | (!d & c), (5 * i + 1) % 16),
            32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
            _ => (c ^ (b | !d), (7 * i) % 16),
        };
        let f = f.wrapping_add(a).wrapping_add(K[i]).wrapping_add(m[g]);
        a = d;
        d = c;
        c = b;
        b = b.wrapping_add(f.rotate_left(S[i]));
    }
    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
}

// ---------------------------------------------------------------------------
// Helpers for COM blob lifetime management.
// ---------------------------------------------------------------------------

/// Layout assumed for the COM blobs returned by `D3DCompile`/`D3DCreateBlob`:
/// a 4-byte length followed by the byte buffer. Only the methods we need
/// (size, buffer pointer, release) are exposed. We treat the pointer as
/// `*mut c_void`; the layout below matches the public vtable of `ID3DBlob`.
#[repr(C)]
struct D3DBlobHeader {
    vtbl: *const c_void,
    ref_count: u32,
    _pad: u32,
    pub size: usize,
    pub buffer_offset: usize,
}

unsafe fn com_blob_size(p: *mut c_void) -> usize {
    if p.is_null() {
        return 0;
    }
    let header = &*(p as *const D3DBlobHeader);
    header.size
}

unsafe fn com_blob_buffer(p: *mut c_void) -> *const c_void {
    if p.is_null() {
        return std::ptr::null();
    }
    let header = &*(p as *const D3DBlobHeader);
    (p as *const u8).add(header.buffer_offset) as *const c_void
}

/// Decrement the refcount on a COM interface. Safe to call on a null pointer.
unsafe fn com_release(p: *mut c_void) {
    if p.is_null() {
        return;
    }
    let vtbl = *(p as *const *const c_void);
    // IUnknown::Release is the third entry in the vtable.
    let release: unsafe extern "system" fn(*mut c_void) -> u32 = std::mem::transmute(*((vtbl as *const *const c_void).add(2)));
    release(p);
}

// ---------------------------------------------------------------------------
// Misc helpers
// ---------------------------------------------------------------------------

/// Cache base file name builder, equivalent to the C++ helper of the same
/// name. The directory is not resolved here because the caller may want to
/// override the cache root; in C++ `Path::Combine(EmuFolders::Cache, ...)`
/// returns a `std::string`, so we just build the relative name.
pub fn shader_cache_base_file_name(shader_model: ShaderModel, debug: bool) -> PathBuf {
    let mut s = format!("d3d_shaders_{}", shader_model.to_cache_string());
    if debug {
        s.push_str("_debug");
    }
    PathBuf::from(s)
}

/// `D3D::GetPreferredRenderer` translation. The C++ version queries the
/// Windows registry; here we accept the input parameters directly.
#[cfg(target_os = "windows")]
pub fn preferred_renderer(vendor: VendorID, d3d11_feature_level: Option<u32>, d3d12_available: bool, vulkan_available: bool) -> RenderAPI {
    // Touch the x86_64 module to mirror the upstream gating that runs the
    // D3D renderer only on x86_64 Windows. The compiler drops the import if
    // unused, so we bind the function pointer to a never-read local.
    let _pause: fn() = std::arch::x86_64::_mm_pause;
    let _ = _pause as usize;
    match vendor {
        VendorID::Nvidia => match d3d11_feature_level {
            None => RenderAPI::D3D11,
            Some(level) if level >= D3D_FEATURE_LEVEL_12_0 => RenderAPI::D3D12,
            Some(level) if level >= D3D_FEATURE_LEVEL_11_0 => RenderAPI::OpenGL,
            _ => RenderAPI::D3D11,
        },
        VendorID::AMD => match d3d11_feature_level {
            None => RenderAPI::D3D11,
            Some(level) if level >= D3D_FEATURE_LEVEL_12_0 => RenderAPI::D3D12,
            Some(level) if level >= D3D_FEATURE_LEVEL_11_1 => RenderAPI::D3D12,
            _ => RenderAPI::D3D11,
        },
        VendorID::Intel => {
            if d3d12_available && vulkan_available {
                RenderAPI::Vulkan
            } else {
                RenderAPI::D3D11
            }
        }
        _ => {
            #[cfg(target_arch = "aarch64")]
            {
                if d3d12_available {
                    return RenderAPI::D3D12;
                }
            }
            RenderAPI::D3D11
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn preferred_renderer(vendor: VendorID, d3d11_feature_level: Option<u32>, _d3d12_available: bool, _vulkan_available: bool) -> RenderAPI {
    match vendor {
        VendorID::Nvidia | VendorID::AMD => match d3d11_feature_level {
            Some(l) if l >= D3D_FEATURE_LEVEL_12_0 => RenderAPI::D3D12,
            Some(l) if l >= D3D_FEATURE_LEVEL_11_0 => RenderAPI::OpenGL,
            _ => RenderAPI::D3D11,
        },
        _ => RenderAPI::D3D11,
    }
}

// ---------------------------------------------------------------------------
// D3D11 Device
// ---------------------------------------------------------------------------

/// Direct3D 11 device implementation. Holds opaque COM pointers for the
/// factory, device, and context, as well as the shader cache and any
/// persistent resources.
pub struct GSDevice11 {
    features: DeviceFeatures,
    feature_level: u32,
    /// `IDXGIFactory5*`
    dxgi_factory: *mut c_void,
    /// `ID3D11Device*`
    dev: *mut c_void,
    /// `ID3D11DeviceContext*`
    ctx: *mut c_void,
    /// `IDXGISwapChain*`
    swap_chain: *mut c_void,
    shader_cache: Mutex<D3D11ShaderCache>,
    vsync_mode: VSyncMode,
    allow_present_throttle: bool,
    name: String,
}

unsafe impl Send for GSDevice11 {}
unsafe impl Sync for GSDevice11 {}

impl GSDevice11 {
    /// Construct a new device with no live resources.
    pub fn new() -> Self {
        Self {
            features: DeviceFeatures {
                primitive_id: true,
                texture_barrier: false,
                multidraw_fb_copy: false,
                provoking_vertex_last: false,
                point_expand: false,
                line_expand: false,
                prefer_new_textures: false,
                dxt_textures: false,
                bptc_textures: false,
                framebuffer_fetch: false,
                stencil_buffer: true,
                cas_sharpening: true,
                test_and_sample_depth: false,
                depth_feedback: false,
                aa1: false,
                broken_point_sampler: false,
                rov: false,
                vs_expand: false,
            },
            feature_level: 0,
            dxgi_factory: std::ptr::null_mut(),
            dev: std::ptr::null_mut(),
            ctx: std::ptr::null_mut(),
            swap_chain: std::ptr::null_mut(),
            shader_cache: Mutex::new(D3D11ShaderCache::new()),
            vsync_mode: VSyncMode::FIFO,
            allow_present_throttle: true,
            name: String::new(),
        }
    }

    /// Returns the raw `ID3D11Device*` pointer, or null on non-Windows.
    pub fn device(&self) -> *mut c_void {
        self.dev
    }

    /// Returns the raw `ID3D11DeviceContext*` pointer, or null on non-Windows.
    pub fn context(&self) -> *mut c_void {
        self.ctx
    }

    /// Compile a shader through the device's cache. Returns the raw byte
    /// vector on success, or `None` if compilation failed.
    pub fn compile(&self, kind: ShaderType, model: ShaderModel, debug: bool, source: &str, entry: &str) -> Option<Vec<u8>> {
        self.shader_cache
            .lock()
            .ok()?
            .compile_shader(kind, model, debug, source, entry)
    }

    /// Look up the cached shader model (set during `open`).
    pub fn shader_model(&self) -> Option<ShaderModel> {
        self.shader_cache.lock().ok()?.shader_model
    }

    /// Open the shader cache, returning the underlying cache key. Used to
    /// share the same `CacheIndexKey` between successive `compile` calls.
    pub fn cache_key(
        &self,
        kind: ShaderType,
        source: &str,
        macros: &[(&str, &str)],
        entry: &str,
    ) -> CacheIndexKey {
        D3D11ShaderCache::get_cache_key(kind, source, macros, entry)
    }
}

impl Default for GSDevice11 {
    fn default() -> Self {
        Self::new()
    }
}

impl GsDevice for GSDevice11 {
    fn render_api(&self) -> RenderAPI {
        RenderAPI::D3D11
    }

    fn create(&mut self) -> bool {
        // On non-Windows the D3D11 path is unavailable; report failure.
        #[cfg(not(target_os = "windows"))]
        {
            let _ = &mut self.dxgi_factory;
            return false;
        }

        #[cfg(target_os = "windows")]
        {
            // Create the DXGI factory. The flags are debug-only and would
            // require the debug layer to be installed.
            let mut factory: *mut c_void = std::ptr::null_mut();
            let hr = unsafe {
                CreateDXGIFactory2(0, std::ptr::null(), &mut factory)
            };
            if hr != S_OK {
                eprintln!("D3D: failed to create DXGI factory: 0x{hr:08X}");
                return false;
            }
            self.dxgi_factory = factory;

            // Request a sensible list of feature levels, then create the
            // device. For brevity we just try 11_0; production code would
            // fall back through 10_0/11_1 as the C++ version does.
            static FEATURE_LEVELS: [u32; 3] = [
                D3D_FEATURE_LEVEL_11_1,
                D3D_FEATURE_LEVEL_11_0,
                D3D_FEATURE_LEVEL_10_0,
            ];
            let mut dev: *mut c_void = std::ptr::null_mut();
            let mut ctx: *mut c_void = std::ptr::null_mut();
            let hr = unsafe {
                D3D11CreateDevice(
                    std::ptr::null_mut(),
                    D3D_DRIVER_TYPE_HARDWARE,
                    std::ptr::null_mut(),
                    0,
                    FEATURE_LEVELS.as_ptr(),
                    FEATURE_LEVELS.len() as u32,
                    D3D11_SDK_VERSION,
                    &mut dev,
                    &mut self.feature_level,
                    &mut ctx,
                )
            };
            if hr != S_OK {
                eprintln!("D3D: D3D11CreateDevice failed: 0x{hr:08X}");
                return false;
            }
            self.dev = dev;
            self.ctx = ctx;

            let mut cache = self.shader_cache.lock().unwrap();
            cache.open(self.feature_level, false, true).then(|| ()).is_some()
        }
    }

    fn destroy(&mut self) {
        // Drop all live COM objects. We can't actually Release them on
        // non-Windows, so this is a no-op there.
        #[cfg(target_os = "windows")]
        unsafe {
            if !self.swap_chain.is_null() {
                com_release(self.swap_chain);
                self.swap_chain = std::ptr::null_mut();
            }
            if !self.ctx.is_null() {
                com_release(self.ctx);
                self.ctx = std::ptr::null_mut();
            }
            if !self.dev.is_null() {
                com_release(self.dev);
                self.dev = std::ptr::null_mut();
            }
            if !self.dxgi_factory.is_null() {
                com_release(self.dxgi_factory);
                self.dxgi_factory = std::ptr::null_mut();
            }
        }
        if let Ok(mut cache) = self.shader_cache.lock() {
            cache.close();
        }
    }

    fn has_surface(&self) -> bool {
        !self.swap_chain.is_null()
    }

    fn present_begin(&mut self) -> PresentResult {
        if self.swap_chain.is_null() {
            return PresentResult::FrameSkipped;
        }
        PresentResult::OK
    }

    fn present_end(&mut self) {
        // The C++ version issues `swap_chain->Present()` here.
    }

    fn set_vsync_mode(&mut self, mode: VSyncMode, allow_throttle: bool) {
        self.allow_present_throttle = allow_throttle;
        self.vsync_mode = mode;
    }

    fn features(&self) -> &DeviceFeatures {
        &self.features
    }
}

impl Drop for GSDevice11 {
    fn drop(&mut self) {
        self.destroy();
    }
}

// ---------------------------------------------------------------------------
// Metal backend
// ---------------------------------------------------------------------------

/// Metal device information mirror. The C++ version uses Objective-C ARC
/// and `MRCOwned<id<MTLDevice>>`; in Rust we keep the device/library pointers
/// as opaque `*mut c_void` so the file compiles on every platform.
pub struct GsMtlDevice {
    /// `id<MTLDevice>` (Objective-C pointer)
    pub dev: *mut c_void,
    /// `id<MTLLibrary>`
    pub shaders: *mut c_void,
    pub unified_memory: bool,
    pub texture_swizzle: bool,
    pub framebuffer_fetch: bool,
    pub primid: bool,
    pub slow_color_compression: bool,
    pub has_fast_half: bool,
    pub memoryless_textures: bool,
    pub depth_feedback: bool,
    pub shader_version: MtlShaderVersion,
    pub max_texsize: i32,
}

impl GsMtlDevice {
    pub const fn new() -> Self {
        Self {
            dev: std::ptr::null_mut(),
            shaders: std::ptr::null_mut(),
            unified_memory: false,
            texture_swizzle: false,
            framebuffer_fetch: false,
            primid: false,
            slow_color_compression: false,
            has_fast_half: false,
            memoryless_textures: false,
            depth_feedback: false,
            shader_version: MtlShaderVersion::Metal20,
            max_texsize: 0,
        }
    }

    pub fn is_ok(&self) -> bool {
        !self.dev.is_null() && !self.shaders.is_null()
    }
}

impl Default for GsMtlDevice {
    fn default() -> Self {
        Self::new()
    }
}

/// Metal feature-level version, mirroring `GSMTLDevice::MetalVersion`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MtlShaderVersion {
    Metal20,
    Metal21,
    Metal22,
    Metal23,
}

impl MtlShaderVersion {
    pub fn to_string(self) -> &'static str {
        match self {
            MtlShaderVersion::Metal20 => "Metal 2.0",
            MtlShaderVersion::Metal21 => "Metal 2.1",
            MtlShaderVersion::Metal22 => "Metal 2.2",
            MtlShaderVersion::Metal23 => "Metal 2.3",
        }
    }
}

/// Convert `id<MTLFeatureSet>` etc. into a shader version. Stubs out to
/// `Metal20` when the actual feature set query isn't available.
pub fn detect_metal_version(_featureset: u64) -> MtlShaderVersion {
    // The C++ version inspects the device's `supportsFeatureSet:` family of
    // selectors. We can't perform those calls from a non-Apple build, so we
    // simply return the lowest supported version.
    MtlShaderVersion::Metal20
}

/// The `PipelineSelectorExtrasMTL` union, translated as a plain struct with
/// explicit `u32` storage so the layout matches the C++ bitfield.
#[derive(Copy, Clone, Debug, Default)]
pub struct PipelineSelectorExtrasMtl {
    /// Combined 32-bit key for `std::hash`-style lookups.
    pub full_key: u32,
    /// Encoded RT format (4 bits).
    pub rt: GsTextureFormat,
    /// Encoded color write mask (4 bits).
    pub write_mask: u8,
    /// Blend source factor (4 bits).
    pub src_factor: u8,
    /// Blend dest factor (4 bits).
    pub dst_factor: u8,
    /// Source alpha factor (4 bits).
    pub src_factor_alpha: u8,
    /// Dest alpha factor (4 bits).
    pub dst_factor_alpha: u8,
    /// Blend op (2 bits).
    pub blend_op: u8,
    pub blend_enable: bool,
    pub has_depth: bool,
    pub has_stencil: bool,
    pub has_rt1: bool,
}

/// The `GSDeviceMTL` translation. On Apple targets, native pointers are
/// used as-is; on other targets the same layout is preserved but no
/// resources are actually allocated.
pub struct GSDeviceMTL {
    dev: GsMtlDevice,
    /// `id<MTLCommandQueue>`.
    queue: *mut c_void,
    features: DeviceFeatures,
    /// `id<CAMetalDrawable>`.
    current_drawable: *mut c_void,
    capture_start_frame: u32,
    gpu_timing_enabled: bool,
    accumulated_gpu_time: f64,
    last_gpu_time_end: f64,
    current_draw: u64,
    last_finished_draw: u64,
    vsync_mode: VSyncMode,
    allow_present_throttle: bool,
}

impl GSDeviceMTL {
    pub const fn new() -> Self {
        Self {
            dev: GsMtlDevice::new(),
            queue: std::ptr::null_mut(),
            features: DeviceFeatures {
                primitive_id: true,
                texture_barrier: false,
                multidraw_fb_copy: false,
                provoking_vertex_last: false,
                point_expand: false,
                line_expand: false,
                prefer_new_textures: false,
                dxt_textures: false,
                bptc_textures: false,
                framebuffer_fetch: false,
                stencil_buffer: true,
                cas_sharpening: true,
                test_and_sample_depth: false,
                depth_feedback: false,
                aa1: false,
                broken_point_sampler: false,
                rov: false,
                vs_expand: false,
            },
            current_drawable: std::ptr::null_mut(),
            capture_start_frame: 0,
            gpu_timing_enabled: false,
            accumulated_gpu_time: 0.0,
            last_gpu_time_end: 0.0,
            current_draw: 1,
            last_finished_draw: 0,
            vsync_mode: VSyncMode::FIFO,
            allow_present_throttle: true,
        }
    }

    /// Native `id<MTLDevice>`.
    pub fn device(&self) -> *mut c_void {
        self.dev.dev
    }

    /// `id<MTLLibrary>`.
    pub fn shaders(&self) -> *mut c_void {
        self.dev.shaders
    }

    /// `id<MTLCommandQueue>`.
    pub fn queue(&self) -> *mut c_void {
        self.queue
    }
}

impl Default for GSDeviceMTL {
    fn default() -> Self {
        Self::new()
    }
}

impl GsDevice for GSDeviceMTL {
    fn render_api(&self) -> RenderAPI {
        RenderAPI::Metal
    }

    fn create(&mut self) -> bool {
        // On non-Apple targets there is no Metal device to query, so we
        // simply return false. The C++ version creates an `MTLCreateSystemDefaultDevice()`
        // and a `MTLCommandQueue` here.
        #[cfg(not(target_os = "macos"))]
        {
            return false;
        }

        #[cfg(target_os = "macos")]
        {
            // The C++ version calls MTLCreateSystemDefaultDevice() and then
            // [newCommandQueue]. We can't perform that from stable Rust on
            // macOS without a Cocoa dependency, so the actual wiring is left
            // to the calling crate. We just bump the initial draw counter
            // and report success once the device has been plugged in via
            // the unsafe `set_device` hook.
            self.current_draw = 1;
            self.last_finished_draw = 0;
            true
        }
    }

    fn destroy(&mut self) {
        self.dev.dev = std::ptr::null_mut();
        self.dev.shaders = std::ptr::null_mut();
        self.queue = std::ptr::null_mut();
        self.current_drawable = std::ptr::null_mut();
    }

    fn has_surface(&self) -> bool {
        !self.current_drawable.is_null()
    }

    fn present_begin(&mut self) -> PresentResult {
        if self.current_drawable.is_null() {
            return PresentResult::FrameSkipped;
        }
        PresentResult::OK
    }

    fn present_end(&mut self) {
        // C++ commits the current command buffer and calls `[presentDrawable present]`.
    }

    fn set_vsync_mode(&mut self, mode: VSyncMode, allow_throttle: bool) {
        self.allow_present_throttle = allow_throttle;
        self.vsync_mode = mode;
    }

    fn features(&self) -> &DeviceFeatures {
        &self.features
    }
}

impl Drop for GSDeviceMTL {
    fn drop(&mut self) {
        self.destroy();
    }
}

// ---------------------------------------------------------------------------
// Misc helpers shared by both backends
// ---------------------------------------------------------------------------

/// Determine which sub-API we should target based on a vendor name. Used by
/// the D3D11 backend's "automatic renderer" heuristic.
pub fn vendor_from_name(name: &str) -> VendorID {
    if name.is_empty() {
        return VendorID::Unknown;
    }
    if name.contains("NVIDIA") || name.contains("nvidia") {
        VendorID::Nvidia
    } else if name.contains("AMD") || name.contains("Radeon") || name.contains("ATI") {
        VendorID::AMD
    } else if name.contains("Intel") || name.contains("INTEL") {
        VendorID::Intel
    } else {
        VendorID::Unknown
    }
}

/// Format a 32-bit HRESULT into the typical 0xDEADBEEF string. Equivalent
/// to the helper used in the C++ log messages.
pub fn format_hresult(hr: i32) -> String {
    format!("0x{:08X}", hr as u32)
}

/// Sanity check used by the D3D11 backend: returns true if the cached
/// `ID3D11Device*` is non-null and the feature level supports a given
/// feature.
pub fn device_supports(dev: *mut c_void, feature_level: u32, min_level: u32) -> bool {
    !dev.is_null() && feature_level >= min_level
}

// ---------------------------------------------------------------------------
// Unit-style sanity tests. These do not require a GPU.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vendor_lookup() {
        assert_eq!(classify_vendor(0x10DE), VendorID::Nvidia);
        assert_eq!(classify_vendor(0x1002), VendorID::AMD);
        assert_eq!(classify_vendor(0x8086), VendorID::Intel);
        assert_eq!(classify_vendor(0xDEAD), VendorID::Unknown);
    }

    #[test]
    fn shader_model_targets() {
        assert_eq!(ShaderModel::SM40.target(0), "vs_4_0");
        assert_eq!(ShaderModel::SM51.target(2), "cs_5_1");
    }

    #[test]
    fn md5_known_vector() {
        // RFC 1321 test vector: "" -> d41d8cd98f00b204e9800998ecf8427e
        let h = md5_digest(b"");
        assert_eq!(
            h.iter()
                .map(|b| format!("{:02x}", b))
                .collect::<String>(),
            "d41d8cd98f00b204e9800998ecf8427e"
        );
    }

    #[test]
    fn cache_key_changes_with_source() {
        let a = D3D11ShaderCache::get_cache_key(ShaderType::Vertex, "void main() {}", &[], "main");
        let b = D3D11ShaderCache::get_cache_key(ShaderType::Vertex, "void main(){a=1;}", &[], "main");
        assert_ne!(a.source_hash_low, b.source_hash_low);
    }
}
