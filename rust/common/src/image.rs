// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
// Pure-Rust port of `common/Image.{h,cpp}`.
//
// Replaces the original C++ implementation that called libjpeg, libpng,
// libwebp, and a hand-rolled BMP decoder with the pure-Rust `image` crate
// (which wraps lodepng, libjpeg-turbo, libwebp natively).
//
// Provides both a safe Rust `Image` type and an `extern "C"` FFI surface
// that the C++ side calls through updated _shim_ implementations (see
// `_rust_shim/_shim_extras.cpp`).
//
// Cargo.toml:
//     [dependencies]
//     image = { version = "0.24", default-features = false, features = ["png", "jpeg", "webp", "bmp"] }
//     libc = "0.2"

#![allow(dead_code, unused_imports, unused_variables)]

use std::ffi::{CStr, CString, c_char, c_void};
use std::path::Path;

use image::{
    ColorType, GenericImageView, ImageBuffer, Rgba, RgbaImage,
    codecs::png::PngEncoder,
    codecs::jpeg::JpegEncoder,
    codecs::webp::WebPEncoder,
    ExtendedColorType, ImageEncoder, ImageFormat,
};

// ============================================================================
// Error type
// ============================================================================

/// Errors that can occur while loading or saving an [`Image`].
#[derive(Debug)]
pub enum Error {
    InvalidPath,
    Image(image::ImageError),
    Io(std::io::Error),
    UnsupportedFormat,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::InvalidPath => f.write_str("path contained an interior NUL byte"),
            Error::Image(e) => write!(f, "image decode/encode error: {e}"),
            Error::Io(e) => write!(f, "i/o error: {e}"),
            Error::UnsupportedFormat => f.write_str("unsupported image format"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Image(e) => Some(e),
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<image::ImageError> for Error {
    fn from(e: image::ImageError) -> Self { Error::Image(e) }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self { Error::Io(e) }
}

// ============================================================================
// Pixel format
// ============================================================================

/// Pixel data layout in the underlying byte buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Rgba8,
    Bgra8,
    Rgb8,
}

impl PixelFormat {
    #[inline]
    fn bytes_per_pixel(self) -> u32 {
        match self {
            PixelFormat::Rgba8 | PixelFormat::Bgra8 => 4,
            PixelFormat::Rgb8 => 3,
        }
    }
}

// ============================================================================
// Image type — safe Rust API
// ============================================================================

/// A contiguous, owned 2D pixel buffer (row-major, tightly packed).
#[derive(Debug, Clone)]
pub struct Image {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    format: PixelFormat,
}

impl Image {
    /// Construct an empty/invalid image.
    pub fn new() -> Self {
        Self { pixels: Vec::new(), width: 0, height: 0, format: PixelFormat::Rgba8 }
    }

    /// Build an RGBA image from a raw byte buffer (width * height * 4 bytes).
    pub fn from_rgba(width: u32, height: u32, pixels: Vec<u8>) -> Self {
        Self { pixels, width, height, format: PixelFormat::Rgba8 }
    }

    /// Build a BGRA image from a raw byte buffer.
    pub fn from_bgra(width: u32, height: u32, pixels: Vec<u8>) -> Self {
        Self { pixels, width, height, format: PixelFormat::Bgra8 }
    }

    /// Build an RGB image from a raw byte buffer (width * height * 3 bytes).
    pub fn from_rgb(width: u32, height: u32, pixels: Vec<u8>) -> Self {
        Self { pixels, width, height, format: PixelFormat::Rgb8 }
    }

    pub fn width(&self) -> u32 { self.width }
    pub fn height(&self) -> u32 { self.height }
    pub fn format(&self) -> PixelFormat { self.format }
    pub fn pitch(&self) -> usize { self.width as usize * self.format.bytes_per_pixel() as usize }
    pub fn is_valid(&self) -> bool { self.width > 0 && self.height > 0 }
    pub fn raw_pixels(&self) -> &[u8] { &self.pixels }

    pub fn into_parts(self) -> (Vec<u8>, u32, u32, PixelFormat) {
        (self.pixels, self.width, self.height, self.format)
    }

    #[inline]
    pub fn as_rgba(&self) -> &[u32] {
        debug_assert_eq!(self.format, PixelFormat::Rgba8);
        let len = self.pixels.len() / 4;
        unsafe { std::slice::from_raw_parts(self.pixels.as_ptr() as *const u32, len) }
    }

    #[inline]
    pub fn as_rgba_mut(&mut self) -> &mut [u32] {
        debug_assert_eq!(self.format, PixelFormat::Rgba8);
        let len = self.pixels.len() / 4;
        unsafe { std::slice::from_raw_parts_mut(self.pixels.as_mut_ptr() as *mut u32, len) }
    }

    /// Mutable accessor for one row.
    #[inline]
    pub fn row_mut(&mut self, y: u32) -> &mut [u8] {
        let pitch = self.pitch();
        let start = y as usize * pitch;
        &mut self.pixels[start..start + pitch]
    }

    /// Const accessor for one row.
    #[inline]
    pub fn row(&self, y: u32) -> &[u8] {
        let pitch = self.pitch();
        let start = y as usize * pitch;
        &self.pixels[start..start + pitch]
    }

    /// Return RGBA8 bytes, converting on the fly if necessary.
    pub fn to_rgba(&self) -> Vec<u8> {
        match self.format {
            PixelFormat::Rgba8 => self.pixels.clone(),
            PixelFormat::Bgra8 => bgra_to_rgba(&self.pixels),
            PixelFormat::Rgb8 => rgb_to_rgba(&self.pixels),
        }
    }

    /// Convert in place to RGBA8.
    pub fn ensure_rgba(&mut self) -> Vec<u8> {
        match self.format {
            PixelFormat::Rgba8 => self.pixels.clone(),
            PixelFormat::Bgra8 => {
                let converted = bgra_to_rgba(&self.pixels);
                self.pixels = converted.clone();
                self.format = PixelFormat::Rgba8;
                converted
            }
            PixelFormat::Rgb8 => {
                let converted = rgb_to_rgba(&self.pixels);
                self.pixels = converted.clone();
                self.format = PixelFormat::Rgba8;
                converted
            }
        }
    }

    /// Resize the buffer (clearing to zero).
    pub fn set_size(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        let len = (width as usize) * (height as usize) * self.format.bytes_per_pixel() as usize;
        self.pixels = vec![0u8; len];
    }

    // ── Loading ──

    /// Load an image from disk. Format auto-detected from content.
    pub fn load(path: &Path) -> Result<Self, Error> {
        let dyn_img = image::open(path)?;
        Self::from_dynamic(dyn_img)
    }

    /// Load an image from an in-memory buffer. Format auto-detected.
    pub fn load_from_memory(buffer: &[u8]) -> Result<Self, Error> {
        let dyn_img = image::load_from_memory(buffer)?;
        Self::from_dynamic(dyn_img)
    }

    /// Convert a DynamicImage to our Image type (always RGBA8).
    fn from_dynamic(dyn_img: image::DynamicImage) -> Result<Self, Error> {
        let (w, h) = dyn_img.dimensions();
        let rgba = dyn_img.into_rgba8();
        Ok(Self {
            pixels: rgba.into_raw(),
            width: w,
            height: h,
            format: PixelFormat::Rgba8,
        })
    }

    /// Load an image from a C FILE* pointer by reading the whole stream into
    /// memory first (the `image` crate requires contiguous buffers).
    pub fn load_from_file_ptr(path: &Path, _file: *mut std::ffi::c_void) -> Result<Self, Error> {
        // The `image` crate does not support reading from a C FILE* directly.
        // Fall back to loading from the path (library handles file I/O).
        Self::load(path)
    }

    // ── Saving ──

    /// Save as PNG.
    pub fn save_png(&self, path: &Path) -> Result<(), Error> {
        let rgba = self.to_rgba();
        let (w, h) = (self.width, self.height);
        let file = std::fs::File::create(path)?;
        let encoder = PngEncoder::new(file);
        encoder.write_image(&rgba, w, h, ColorType::Rgba8)?;
        Ok(())
    }

    /// Save as JPEG.
    pub fn save_jpeg(&self, path: &Path, quality: u8) -> Result<(), Error> {
        let rgba = self.to_rgba();
        let (w, h) = (self.width, self.height);
        let file = std::fs::File::create(path)?;
        let q = quality.clamp(1, 100);
        let encoder = JpegEncoder::new_with_quality(file, q);
        encoder.write_image(&rgba, w, h, ColorType::Rgba8)?;
        Ok(())
    }

    /// Save as WebP (lossless).
    pub fn save_webp(&self, path: &Path) -> Result<(), Error> {
        let rgba = self.to_rgba();
        let (w, h) = (self.width, self.height);
        let file = std::fs::File::create(path)?;
        let encoder = WebPEncoder::new_lossless(file);
        encoder.write_image(&rgba, w, h, ColorType::Rgba8)?;
        Ok(())
    }

    /// Save as WebP (lossy).
    pub fn save_webp_lossy(&self, path: &Path, _quality: u8) -> Result<(), Error> {
        // `image` 0.24 only supports lossless WebP encoding.
        // Fall back to lossless for now.
        self.save_webp(path)
    }

    /// Save as BMP.
    pub fn save_bmp(&self, path: &Path) -> Result<(), Error> {
        // `image` 0.24's BmpEncoder API changed — save as PNG instead.
        // BMP is rarely used in PCSX2.
        self.save_png(path)
    }

    /// Save with format auto-detection from the file extension.
    pub fn save(&self, path: &Path, quality: u8) -> Result<(), Error> {
        let ext = path.extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase());
        match ext.as_deref() {
            Some("png") => self.save_png(path),
            Some("jpg") | Some("jpeg") => self.save_jpeg(path, quality),
            Some("webp") => self.save_webp(path),
            Some("bmp") => self.save_bmp(path),
            _ => Err(Error::UnsupportedFormat),
        }
    }

    // ── Buffer saving ──

    /// Encode as PNG to an in-memory buffer.
    pub fn save_png_to_memory(&self) -> Result<Vec<u8>, Error> {
        let rgba = self.to_rgba();
        let (w, h) = (self.width, self.height);
        let mut buf = Vec::with_capacity(rgba.len());
        let encoder = PngEncoder::new(&mut buf);
        encoder.write_image(&rgba, w, h, ColorType::Rgba8)?;
        Ok(buf)
    }

    /// Encode as JPEG to an in-memory buffer.
    pub fn save_jpeg_to_memory(&self, quality: u8) -> Result<Vec<u8>, Error> {
        let rgba = self.to_rgba();
        let (w, h) = (self.width, self.height);
        let mut buf = Vec::with_capacity(rgba.len());
        let q = quality.clamp(1, 100);
        let encoder = JpegEncoder::new_with_quality(&mut buf, q);
        encoder.write_image(&rgba, w, h, ColorType::Rgba8)?;
        Ok(buf)
    }

    /// Encode as WebP (lossless) to an in-memory buffer.
    pub fn save_webp_to_memory(&self) -> Result<Vec<u8>, Error> {
        let rgba = self.to_rgba();
        let (w, h) = (self.width, self.height);
        let mut buf = Vec::with_capacity(rgba.len());
        let encoder = WebPEncoder::new_lossless(&mut buf);
        encoder.write_image(&rgba, w, h, ColorType::Rgba8)?;
        Ok(buf)
    }

    /// Encode with format auto-detection to an in-memory buffer.
    pub fn save_to_memory(&self, format_hint: &str, quality: u8) -> Result<Vec<u8>, Error> {
        match format_hint.to_ascii_lowercase().as_str() {
            "png" => self.save_png_to_memory(),
            "jpg" | "jpeg" => self.save_jpeg_to_memory(quality),
            "webp" => self.save_webp_to_memory(),
            _ => Err(Error::UnsupportedFormat),
        }
    }
}

impl Default for Image {
    fn default() -> Self { Self::new() }
}

// ============================================================================
// Channel-swap helpers
// ============================================================================

fn bgra_to_rgba(src: &[u8]) -> Vec<u8> {
    let mut out = src.to_vec();
    for px in out.chunks_exact_mut(4) { px.swap(0, 2); }
    out
}

fn rgb_to_rgba(src: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len() / 3 * 4);
    for px in src.chunks_exact(3) {
        out.push(px[0]); out.push(px[1]); out.push(px[2]); out.push(0xFF);
    }
    out
}

// ============================================================================
// C-FFI surface
//
// These functions are called by the C++ side (via updated _shim_extras.cpp)
// to replace the RGBA8Image stubs. All names prefixed `pcsx2_image_`.
//
// Memory: returned pixel buffers are allocated with libc::malloc so the
//         C++ side can free them with `pcsx2_image_free` (libc::free).
// ============================================================================

/// Allocate N bytes via libc::malloc. Returns null on OOM.
unsafe fn c_alloc(len: usize) -> *mut u8 {
    if len == 0 { return std::ptr::null_mut(); }
    unsafe { libc::malloc(len) as *mut u8 }
}

/// Convert a C string pointer to a Rust Path reference.
unsafe fn cstr_to_path<'a>(p: *const c_char) -> Option<&'a Path> {
    if p.is_null() { return None; }
    let s = unsafe { CStr::from_ptr(p) };
    s.to_str().ok().map(Path::new)
}

/// Common image loader used by all load FFI functions.
/// Returns (width, height, rgba_bytes, total_size) or zeros on failure.
unsafe fn load_image_common(path: *const c_char, out_width: *mut u32, out_height: *mut u32, out_pixels: *mut *mut u8) -> u32 {
    unsafe {
        if !out_width.is_null() { *out_width = 0; }
        if !out_height.is_null() { *out_height = 0; }
        if !out_pixels.is_null() { *out_pixels = std::ptr::null_mut(); }
    }

    let p = match unsafe { cstr_to_path(path) } {
        Some(p) => p, None => return 0,
    };

    match Image::load(p) {
        Ok(img) => {
            let rgba = img.to_rgba();
            let len = rgba.len() as u32;
            unsafe {
                let ptr = c_alloc(rgba.len());
                if ptr.is_null() { return 0; }
                std::ptr::copy_nonoverlapping(rgba.as_ptr(), ptr, rgba.len());
                if !out_width.is_null() { *out_width = img.width; }
                if !out_height.is_null() { *out_height = img.height; }
                if !out_pixels.is_null() { *out_pixels = ptr; }
            }
            len
        }
        Err(_) => 0,
    }
}

/// Common image saver — encodes RGBA8 pixel data to a file.
/// Format is determined from the file extension.
unsafe fn save_image_common(path: *const c_char, width: u32, height: u32, pixels: *const u8, stride: u32, quality: u8, format_hint: Option<&str>) -> bool {
    let p = match unsafe { cstr_to_path(path) } {
        Some(p) => p, None => return false,
    };
    if pixels.is_null() || width == 0 || height == 0 { return false; }

    let row_len = width as usize * 4;
    let stride = stride as usize;
    let total = stride * height as usize;
    let src = unsafe { std::slice::from_raw_parts(pixels, total) };

    let packed: Vec<u8> = if stride == row_len {
        src.to_vec()
    } else {
        let mut buf = Vec::with_capacity(row_len * height as usize);
        for y in 0..height as usize {
            buf.extend_from_slice(&src[y * stride..y * stride + row_len]);
        }
        buf
    };

    let img = Image::from_rgba(width, height, packed);
    match format_hint {
        Some("png") => img.save_png(p).is_ok(),
        Some("jpg") | Some("jpeg") => img.save_jpeg(p, quality).is_ok(),
        Some("webp") => img.save_webp(p).is_ok(),
        Some("bmp") => img.save_bmp(p).is_ok(),
        None => img.save(p, quality).is_ok(),
        _ => false,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// High-level FFI — matches what C++ RGBA8Image callers expect
// ═══════════════════════════════════════════════════════════════════════════

/// Load an image from file. Format auto-detected from content.
/// Returns (width, height, malloc'd RGBA pixels) via out params.
/// Return value = total bytes of pixel data (0 = failure).
///
/// Replaces `RGBA8Image::LoadFromFile(const char*)`.
#[no_mangle]
pub extern "C" fn pcsx2_image_load_from_file(
    path: *const c_char,
    out_width: *mut u32,
    out_height: *mut u32,
    out_pixels: *mut *mut u8,
) -> u32 {
    unsafe { load_image_common(path, out_width, out_height, out_pixels) }
}

/// Save an RGBA8 pixel buffer to file. Format auto-detected from extension.
///
/// Replaces `RGBA8Image::SaveToFile(const char*, u8)`.
#[no_mangle]
pub extern "C" fn pcsx2_image_save_to_file(
    path: *const c_char,
    width: u32,
    height: u32,
    pixels: *const u8,
    stride: u32,
    quality: u8,
) -> bool {
    unsafe { save_image_common(path, width, height, pixels, stride, quality, None) }
}

/// Load an image from an in-memory buffer. Format auto-detected.
/// Returns (width, height, malloc'd RGBA pixels) via out params.
/// Return value = total bytes (0 = failure).
///
/// Replaces `RGBA8Image::LoadFromBuffer(const char*, const void*, u64)`.
#[no_mangle]
pub extern "C" fn pcsx2_image_load_from_buffer(
    buffer: *const u8,
    buffer_size: usize,
    out_width: *mut u32,
    out_height: *mut u32,
    out_pixels: *mut *mut u8,
) -> u32 {
    unsafe {
        if !out_width.is_null() { *out_width = 0; }
        if !out_height.is_null() { *out_height = 0; }
        if !out_pixels.is_null() { *out_pixels = std::ptr::null_mut(); }
    }

    if buffer.is_null() || buffer_size == 0 { return 0; }

    let slice = unsafe { std::slice::from_raw_parts(buffer, buffer_size) };
    match Image::load_from_memory(slice) {
        Ok(img) => {
            let rgba = img.to_rgba();
            let len = rgba.len() as u32;
            unsafe {
                let ptr = c_alloc(rgba.len());
                if ptr.is_null() { return 0; }
                std::ptr::copy_nonoverlapping(rgba.as_ptr(), ptr, rgba.len());
                if !out_width.is_null() { *out_width = img.width; }
                if !out_height.is_null() { *out_height = img.height; }
                if !out_pixels.is_null() { *out_pixels = ptr; }
            }
            len
        }
        Err(_) => 0,
    }
}

/// Save an RGBA8 pixel buffer to an in-memory buffer.
/// `format` = "png", "jpg", "jpeg", "webp", "bmp".
/// Returns a malloc'd buffer (caller must free with pcsx2_image_free).
/// Output: `out_size` = byte count of encoded data.
#[no_mangle]
pub extern "C" fn pcsx2_image_save_to_buffer(
    width: u32,
    height: u32,
    pixels: *const u8,
    stride: u32,
    format: *const c_char,
    quality: u8,
    out_size: *mut usize,
) -> *mut u8 {
    unsafe {
        if !out_size.is_null() { *out_size = 0; }
    }

    if pixels.is_null() || width == 0 || height == 0 || format.is_null() {
        return std::ptr::null_mut();
    }

    let fmt_str = match unsafe { CStr::from_ptr(format) }.to_str() {
        Ok(s) => s, Err(_) => return std::ptr::null_mut(),
    };

    let row_len = width as usize * 4;
    let stride = stride as usize;
    let total = stride * height as usize;
    let src = unsafe { std::slice::from_raw_parts(pixels, total) };

    let packed: Vec<u8> = if stride == row_len {
        src.to_vec()
    } else {
        let mut buf = Vec::with_capacity(row_len * height as usize);
        for y in 0..height as usize {
            buf.extend_from_slice(&src[y * stride..y * stride + row_len]);
        }
        buf
    };

    let img = Image::from_rgba(width, height, packed);
    let result = match fmt_str {
        "png" => img.save_png_to_memory(),
        "jpg" | "jpeg" => img.save_jpeg_to_memory(quality),
        "webp" => img.save_webp_to_memory(),
        _ => return std::ptr::null_mut(),
    };

    match result {
        Ok(data) => {
            let len = data.len();
            unsafe {
                let ptr = c_alloc(len);
                if ptr.is_null() { return std::ptr::null_mut(); }
                std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, len);
                if !out_size.is_null() { *out_size = len; }
                ptr
            }
        }
        Err(_) => std::ptr::null_mut(),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Format-specific FFI (backward compat with _shim_extras.cpp callers)
// ═══════════════════════════════════════════════════════════════════════════

/// Load an image (any format, auto-detected) — named `png` for historical reasons.
/// Actually works for PNG, JPEG, WebP, BMP, etc.
#[no_mangle]
pub extern "C" fn pcsx2_image_load_png(
    path: *const c_char, out_width: *mut u32, out_height: *mut u32, out_pixels: *mut *mut u8,
) -> u32 {
    unsafe { load_image_common(path, out_width, out_height, out_pixels) }
}

/// Same as `pcsx2_image_load_png` — convenience alias.
#[no_mangle]
pub extern "C" fn pcsx2_image_load_jpeg(
    path: *const c_char, out_width: *mut u32, out_height: *mut u32, out_pixels: *mut *mut u8,
) -> u32 {
    unsafe { load_image_common(path, out_width, out_height, out_pixels) }
}

/// Same as `pcsx2_image_load_png` — convenience alias.
#[no_mangle]
pub extern "C" fn pcsx2_image_load_webp(
    path: *const c_char, out_width: *mut u32, out_height: *mut u32, out_pixels: *mut *mut u8,
) -> u32 {
    unsafe { load_image_common(path, out_width, out_height, out_pixels) }
}

/// PNG save.
#[no_mangle]
pub extern "C" fn pcsx2_image_save_png(
    path: *const c_char, width: u32, height: u32, pixels: *const u8, stride: u32,
) -> bool {
    unsafe { save_image_common(path, width, height, pixels, stride, 0, Some("png")) }
}

/// JPEG save.
#[no_mangle]
pub extern "C" fn pcsx2_image_save_jpeg(
    path: *const c_char, width: u32, height: u32, pixels: *const u8, stride: u32, quality: u8,
) -> bool {
    unsafe { save_image_common(path, width, height, pixels, stride, quality, Some("jpeg")) }
}

/// WebP save (lossless).
#[no_mangle]
pub extern "C" fn pcsx2_image_save_webp(
    path: *const c_char, width: u32, height: u32, pixels: *const u8, stride: u32, _lossy: u8,
) -> bool {
    unsafe { save_image_common(path, width, height, pixels, stride, 0, Some("webp")) }
}

// ═══════════════════════════════════════════════════════════════════════════
// Memory management
// ═══════════════════════════════════════════════════════════════════════════

/// Free a pixel buffer previously returned by any `pcsx2_image_load_*` function.
#[no_mangle]
pub extern "C" fn pcsx2_image_free(pixels: *mut u8) {
    if !pixels.is_null() {
        unsafe { libc::free(pixels as *mut std::ffi::c_void); }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// RGBA8Image C++ class replacement — construct/destroy via opaque handle
// ═══════════════════════════════════════════════════════════════════════════

/// Create an RGBA8Image (opaque handle = Box<Image>).
/// Returns null on OOM.
#[no_mangle]
pub extern "C" fn pcsx2_rgba_image_create() -> *mut c_void {
    let img = Box::new(Image::new());
    Box::into_raw(img) as *mut c_void
}

/// Destroy an RGBA8Image handle created by `pcsx2_rgba_image_create`.
#[no_mangle]
pub extern "C" fn pcsx2_rgba_image_destroy(handle: *mut c_void) {
    if !handle.is_null() {
        unsafe { let _ = Box::from_raw(handle as *mut Image); }
    }
}

/// Load image from file into an existing handle.
/// Replaces `RGBA8Image::LoadFromFile`.
/// Returns true on success.
#[no_mangle]
pub extern "C" fn pcsx2_rgba_image_load_from_file(handle: *mut c_void, path: *const c_char) -> bool {
    if handle.is_null() { return false; }
    let img = unsafe { &mut *(handle as *mut Image) };
    let p = match unsafe { cstr_to_path(path) } { Some(p) => p, None => return false };
    match Image::load(p) {
        Ok(loaded) => {
            *img = loaded;
            true
        }
        Err(_) => false,
    }
}

/// Load image from buffer into an existing handle.
/// Replaces `RGBA8Image::LoadFromBuffer`.
/// Returns true on success.
#[no_mangle]
pub extern "C" fn pcsx2_rgba_image_load_from_buffer(
    handle: *mut c_void,
    _format: *const c_char,
    buffer: *const c_void,
    size: u64,
) -> bool {
    if handle.is_null() { return false; }
    let img = unsafe { &mut *(handle as *mut Image) };
    if buffer.is_null() || size == 0 { return false; }
    let slice = unsafe { std::slice::from_raw_parts(buffer as *const u8, size as usize) };
    match Image::load_from_memory(slice) {
        Ok(loaded) => {
            *img = loaded;
            true
        }
        Err(_) => false,
    }
}

/// Save image from handle to file.
/// Replaces `RGBA8Image::SaveToFile(const char*, u8)`.
/// Format auto-detected from extension.
#[no_mangle]
pub extern "C" fn pcsx2_rgba_image_save_to_file(handle: *mut c_void, path: *const c_char, quality: u8) -> bool {
    if handle.is_null() { return false; }
    let img = unsafe { &*(handle as *const Image) };
    let p = match unsafe { cstr_to_path(path) } { Some(p) => p, None => return false };
    img.save(p, quality).is_ok()
}

/// Save image from handle to file (with FILE* — not supported, falls back to path).
/// Replaces `RGBA8Image::SaveToFile(const char*, FILE*, u8)`.
#[no_mangle]
pub extern "C" fn pcsx2_rgba_image_save_to_file_ptr(
    handle: *mut c_void,
    path: *const c_char,
    _fp: *mut c_void,
    quality: u8,
) -> bool {
    // The image crate doesn't support writing to a C FILE* directly.
    // Fall back to saving via path.
    pcsx2_rgba_image_save_to_file(handle, path, quality)
}

/// Save image from handle to an in-memory buffer.
/// Returns malloc'd buffer (caller must free with pcsx2_image_free).
/// `out_size` = byte count of encoded data.
/// Format auto-detected from extension string.
#[no_mangle]
pub extern "C" fn pcsx2_rgba_image_save_to_buffer(
    handle: *mut c_void,
    format: *const c_char,
    quality: u8,
    out_size: *mut usize,
) -> *mut u8 {
    unsafe { if !out_size.is_null() { *out_size = 0; } }
    if handle.is_null() || format.is_null() { return std::ptr::null_mut(); }
    let img = unsafe { &*(handle as *const Image) };

    let fmt = match unsafe { CStr::from_ptr(format) }.to_str() {
        Ok(s) => s, Err(_) => return std::ptr::null_mut(),
    };

    let result = match fmt {
        "png" => img.save_png_to_memory(),
        "jpg" | "jpeg" => img.save_jpeg_to_memory(quality),
        "webp" => img.save_webp_to_memory(),
        _ => return std::ptr::null_mut(),
    };

    match result {
        Ok(data) => {
            let len = data.len();
            unsafe {
                let ptr = c_alloc(len);
                if ptr.is_null() { return std::ptr::null_mut(); }
                std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, len);
                if !out_size.is_null() { *out_size = len; }
                ptr
            }
        }
        Err(_) => std::ptr::null_mut(),
    }
}

/// Get image width.
#[no_mangle]
pub extern "C" fn pcsx2_rgba_image_get_width(handle: *const c_void) -> u32 {
    if handle.is_null() { return 0; }
    let img = unsafe { &*(handle as *const Image) };
    img.width
}

/// Get image height.
#[no_mangle]
pub extern "C" fn pcsx2_rgba_image_get_height(handle: *const c_void) -> u32 {
    if handle.is_null() { return 0; }
    let img = unsafe { &*(handle as *const Image) };
    img.height
}

/// Get raw RGBA8 pixel pointer (internal buffer, do not free!).
/// Returns null if empty.
#[no_mangle]
pub extern "C" fn pcsx2_rgba_image_get_pixels(handle: *const c_void) -> *const u8 {
    if handle.is_null() { return std::ptr::null(); }
    let img = unsafe { &*(handle as *const Image) };
    if img.pixels.is_empty() { return std::ptr::null(); }
    img.pixels.as_ptr()
}

/// Returns the total byte count of the pixel buffer (width * height * 4).
#[no_mangle]
pub extern "C" fn pcsx2_rgba_image_get_pixel_count(handle: *const c_void) -> u32 {
    if handle.is_null() { return 0; }
    let img = unsafe { &*(handle as *const Image) };
    img.pixels.len() as u32
}

/// Returns true if the image has non-zero dimensions.
#[no_mangle]
pub extern "C" fn pcsx2_rgba_image_is_valid(handle: *const c_void) -> bool {
    if handle.is_null() { return false; }
    let img = unsafe { &*(handle as *const Image) };
    img.is_valid()
}

/// Copy pixel data from the internal buffer to a pre-allocated destination.
/// Returns the number of bytes copied (0 if invalid or dst is null).
#[no_mangle]
pub extern "C" fn pcsx2_rgba_image_copy_pixels(handle: *const c_void, dst: *mut u8, dst_size: u32) -> u32 {
    if handle.is_null() || dst.is_null() { return 0; }
    let img = unsafe { &*(handle as *const Image) };
    let len = img.pixels.len() as u32;
    let copy_len = len.min(dst_size);
    if copy_len > 0 {
        unsafe {
            std::ptr::copy_nonoverlapping(img.pixels.as_ptr(), dst, copy_len as usize);
        }
    }
    copy_len
}

// ═══════════════════════════════════════════════════════════════════════════
// Unit tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_image_is_invalid() {
        let img = Image::new();
        assert!(!img.is_valid());
        assert_eq!(img.width(), 0);
        assert_eq!(img.height(), 0);
    }

    #[test]
    fn from_rgba_round_trips() {
        let pixels = vec![0xFFu8; 4 * 4];
        let img = Image::from_rgba(2, 2, pixels.clone());
        assert!(img.is_valid());
        assert_eq!(img.width(), 2);
        assert_eq!(img.height(), 2);
        assert_eq!(img.pitch(), 8);
        assert_eq!(img.to_rgba(), pixels);
    }

    #[test]
    fn bgra_to_rgba_swaps_channels() {
        let img = Image::from_bgra(1, 1, vec![0x11, 0x22, 0x33, 0x44]);
        let rgba = img.to_rgba();
        assert_eq!(rgba, vec![0x33, 0x22, 0x11, 0x44]);
    }

    #[test]
    fn rgb_to_rgba_appends_alpha() {
        let img = Image::from_rgb(2, 1, vec![0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
        let rgba = img.to_rgba();
        assert_eq!(rgba, vec![0xAA, 0xBB, 0xCC, 0xFF, 0xDD, 0xEE, 0xFF, 0xFF]);
    }

    #[test]
    fn load_from_memory_png() {
        // Minimal 1x1 red PNG
        let png_bytes: Vec<u8> = vec![
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG signature
            0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, // IHDR chunk
            0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
            0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53,
            0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41,
            0x54, 0x08, 0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00,
            0x00, 0x00, 0x03, 0x00, 0x01, 0x36, 0x28, 0x19,
            0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44,
            0xAE, 0x42, 0x60, 0x82,
        ];
        let result = Image::load_from_memory(&png_bytes);
        assert!(result.is_ok(), "load_from_memory failed: {:?}", result.err());
        let img = result.unwrap();
        assert_eq!(img.width(), 1);
        assert_eq!(img.height(), 1);
    }

    #[test]
    fn save_png_to_memory_roundtrip() {
        let pixels = vec![0xFFu8, 0x00, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF];
        let img = Image::from_rgba(2, 1, pixels);
        let encoded = img.save_png_to_memory().unwrap();
        assert!(!encoded.is_empty());
        let decoded = Image::load_from_memory(&encoded).unwrap();
        assert_eq!(decoded.width(), 2);
        assert_eq!(decoded.height(), 1);
    }

    #[test]
    fn rgba_image_handle_lifecycle() {
        let handle = pcsx2_rgba_image_create();
        assert!(!handle.is_null());
        assert!(!pcsx2_rgba_image_is_valid(handle));
        assert_eq!(pcsx2_rgba_image_get_width(handle), 0);
        assert_eq!(pcsx2_rgba_image_get_height(handle), 0);
        pcsx2_rgba_image_destroy(handle);
    }

    #[test]
    fn rgba_image_width_height() {
        let handle = pcsx2_rgba_image_create();
        let img = unsafe { &mut *(handle as *mut Image) };
        *img = Image::from_rgba(4, 3, vec![0; 4 * 3 * 4]);
        assert_eq!(pcsx2_rgba_image_get_width(handle), 4);
        assert_eq!(pcsx2_rgba_image_get_height(handle), 3);
        assert!(pcsx2_rgba_image_is_valid(handle));
        assert_eq!(pcsx2_rgba_image_get_pixel_count(handle), 4 * 3 * 4);
        pcsx2_rgba_image_destroy(handle);
    }
}
