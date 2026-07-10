// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
// Rust translation of `common/Image.h` and `common/Image.cpp`.
//
// Provides a lightweight `Image` type that mirrors the C++ `Image<PixelType>`
// template / `RGBA8Image` concrete class. Pixel data is stored in a single
// `Vec<u8>` whose layout depends on the `ImageFormat`. PNG/JPG/WebP/BMP
// loaders and savers from the C++ implementation are intentionally stubbed
// with `unimplemented!()`; callers must opt in to a real codec crate.

use std::fmt;
use std::io;
use std::path::Path;

/// Pixel data layouts supported by [`Image`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageFormat {
    /// 32 bits per pixel, channels packed R-G-B-A in memory.
    RGBA8,
    /// 32 bits per pixel, channels packed B-G-R-A in memory.
    BGRA8,
    /// 24 bits per pixel, channels packed R-G-B with no alpha.
    RGB8,
    /// 8 bits per pixel, single luminance channel.
    I8,
    /// 16 bits per pixel, luminance + alpha.
    IA8,
    /// 16 bits per pixel, single half-precision float channel.
    I16F,
    /// 32 bits per pixel, half-precision float luminance + alpha.
    IA16F,
    /// 64 bits per pixel, four half-precision float channels.
    RGBA16F,
}

impl ImageFormat {
    /// Number of bytes a single pixel occupies in memory for this format.
    #[inline]
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            ImageFormat::RGBA8 | ImageFormat::BGRA8 => 4,
            ImageFormat::RGB8 => 3,
            ImageFormat::I8 => 1,
            ImageFormat::IA8 | ImageFormat::I16F => 2,
            ImageFormat::IA16F => 4,
            ImageFormat::RGBA16F => 8,
        }
    }
}

impl fmt::Display for ImageFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            ImageFormat::RGBA8 => "RGBA8",
            ImageFormat::BGRA8 => "BGRA8",
            ImageFormat::RGB8 => "RGB8",
            ImageFormat::I8 => "I8",
            ImageFormat::IA8 => "IA8",
            ImageFormat::I16F => "I16F",
            ImageFormat::IA16F => "IA16F",
            ImageFormat::RGBA16F => "RGBA16F",
        };
        f.write_str(name)
    }
}

/// Errors produced by [`Image`] load/save helpers.
#[derive(Debug)]
pub enum ImageError {
    /// The image has zero width or height, or coordinates fall outside it.
    InvalidDimensions,
    /// The pixel data buffer is shorter than `width * height * bytes_per_pixel`.
    BufferTooSmall,
    /// File could not be opened or read.
    Io(io::Error),
    /// A requested file format is not recognised.
    UnsupportedFormat,
    /// Placeholder for a feature that has not been implemented yet.
    Unimplemented(&'static str),
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImageError::InvalidDimensions => f.write_str("invalid image dimensions"),
            ImageError::BufferTooSmall => f.write_str("image pixel buffer is too small"),
            ImageError::Io(err) => write!(f, "i/o error: {err}"),
            ImageError::UnsupportedFormat => f.write_str("unsupported image format"),
            ImageError::Unimplemented(what) => write!(f, "unimplemented: {what}"),
        }
    }
}

impl std::error::Error for ImageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ImageError::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<io::Error> for ImageError {
    fn from(err: io::Error) -> Self {
        ImageError::Io(err)
    }
}

/// A 2D pixel buffer backed by a single contiguous `Vec<u8>`.
///
/// The C++ original stores a typed `std::vector<PixelType>`; in Rust the
/// underlying storage is always a `Vec<u8>` whose interpretation is
/// dictated by [`Image::format`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub format: ImageFormat,
    pub data: Vec<u8>,
}

impl Image {
    /// Default save quality (matches `RGBA8Image::DEFAULT_SAVE_QUALITY`).
    pub const DEFAULT_SAVE_QUALITY: u8 = 85;

    /// Constructs a new image of the given dimensions, zero-filled.
    pub fn new(width: u32, height: u32, format: ImageFormat) -> Self {
        let bpp = format.bytes_per_pixel() as u64;
        let len = (width as u64 * height as u64 * bpp) as usize;
        Self {
            width,
            height,
            format,
            data: vec![0u8; len],
        }
    }

    /// Resizes the image, zero-filling the pixel buffer.
    pub fn set_size(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        self.data = vec![0u8; Self::buffer_len(width, height, self.format)];
    }

    /// Invalidates the image, clearing the dimensions and pixel buffer.
    pub fn invalidate(&mut self) {
        self.width = 0;
        self.height = 0;
        self.data.clear();
    }

    /// Returns `true` if the image has non-zero dimensions.
    #[inline]
    pub fn is_valid(&self) -> bool {
        self.width > 0 && self.height > 0
    }

    /// Returns the number of bytes in a single row of pixels.
    #[inline]
    pub fn pitch(&self) -> usize {
        self.format.bytes_per_pixel() * self.width as usize
    }

    /// Returns a slice containing the entire pixel buffer.
    #[inline]
    pub fn pixels(&self) -> &[u8] {
        &self.data
    }

    /// Returns a mutable slice containing the entire pixel buffer.
    #[inline]
    pub fn pixels_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    /// Returns the byte slice for row `y`. Panics if `y` is out of bounds.
    #[inline]
    pub fn row_pixels(&self, y: u32) -> &[u8] {
        assert!(
            y < self.height,
            "row index {y} out of bounds (height {})",
            self.height
        );
        let pitch = self.pitch();
        let start = y as usize * pitch;
        &self.data[start..start + pitch]
    }

    /// Returns the mutable byte slice for row `y`.
    #[inline]
    pub fn row_pixels_mut(&mut self, y: u32) -> &mut [u8] {
        let pitch = self.pitch();
        let start = y as usize * pitch;
        &mut self.data[start..start + pitch]
    }

    /// Returns the raw 4-byte pixel at `(x, y)` interpreted as a little/big-
    /// endian `u32`, assuming the format is 4 bytes per pixel. Returns
    /// `None` for narrower formats.
    pub fn get_pixel(&self, x: u32, y: u32) -> Option<Pixel> {
        let offset = self.pixel_offset(x, y)?;
        match self.format {
            ImageFormat::RGBA8 | ImageFormat::BGRA8 | ImageFormat::IA16F => {
                let bytes: [u8; 4] = self.data[offset..offset + 4].try_into().ok()?;
                Some(Pixel::U32(u32::from_ne_bytes(bytes)))
            }
            ImageFormat::RGBA16F => {
                let bytes: [u8; 8] = self.data[offset..offset + 8].try_into().ok()?;
                Some(Pixel::U64(u64::from_ne_bytes(bytes)))
            }
            ImageFormat::RGB8 => {
                let bytes: [u8; 4] = [
                    self.data[offset],
                    self.data[offset + 1],
                    self.data[offset + 2],
                    0,
                ];
                Some(Pixel::U32(u32::from_ne_bytes(bytes)))
            }
            ImageFormat::I8 => Some(Pixel::U32(self.data[offset] as u32)),
            ImageFormat::IA8 | ImageFormat::I16F => {
                Some(Pixel::U32(u16::from_ne_bytes([self.data[offset], self.data[offset + 1]]) as u32))
            }
        }
    }

    /// Writes a 4-byte value at `(x, y)`. Returns `None` if the format is
    /// not 4 bytes per pixel or the coordinates are out of bounds.
    pub fn set_pixel(&mut self, x: u32, y: u32, value: Pixel) -> Option<()> {
        let offset = self.pixel_offset(x, y)?;
        match (self.format, value) {
            (ImageFormat::RGBA8 | ImageFormat::BGRA8 | ImageFormat::IA16F, Pixel::U32(v)) => {
                self.data[offset..offset + 4].copy_from_slice(&v.to_ne_bytes());
            }
            (ImageFormat::RGBA16F, Pixel::U64(v)) => {
                self.data[offset..offset + 8].copy_from_slice(&v.to_ne_bytes());
            }
            (ImageFormat::I8, Pixel::U32(v)) => {
                self.data[offset] = v as u8;
            }
            (ImageFormat::IA8 | ImageFormat::I16F, Pixel::U32(v)) => {
                self.data[offset..offset + 2].copy_from_slice(&(v as u16).to_ne_bytes());
            }
            _ => return None,
        }
        Some(())
    }

    /// Reverses the order of rows in the pixel buffer. Useful when the
    /// source image (e.g. PNG) is stored top-down but the destination
    /// (e.g. a GPU texture) expects bottom-up.
    pub fn flip_vertically(&mut self) {
        if self.height < 2 {
            return;
        }
        let pitch = self.pitch();
        let mut a = 0usize;
        let mut b = (self.height as usize - 1) * pitch;
        while a < b {
            let (left, right) = self.data.split_at_mut(b);
            left[a..a + pitch].swap_with_slice(&mut right[..pitch]);
            a += pitch;
            b -= pitch;
        }
    }

    /// Replaces the pixel buffer with a copy of `pixels` and updates the
    /// dimensions. Returns `Err` if the slice is shorter than required.
    pub fn set_pixels_from_slice(
        &mut self,
        width: u32,
        height: u32,
        pixels: &[u8],
    ) -> Result<(), ImageError> {
        let needed = Self::buffer_len(width, height, self.format);
        if pixels.len() < needed {
            return Err(ImageError::BufferTooSmall);
        }
        self.width = width;
        self.height = height;
        self.data.clear();
        self.data.extend_from_slice(&pixels[..needed]);
        Ok(())
    }

    /// Consumes the image, returning the pixel buffer and resetting state.
    pub fn take_pixels(&mut self) -> Vec<u8> {
        self.width = 0;
        self.height = 0;
        std::mem::take(&mut self.data)
    }

    /// Loads an image from the file at `path`.
    ///
    /// Dispatches by extension to the appropriate codec. BMP is implemented
    /// in pure Rust (no external dependency); PNG/JPEG/WebP remain stubs
    /// that return [`ImageError::Unimplemented`] until a codec crate is
    /// wired into `Cargo.toml`.
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, ImageError> {
        let path = path.as_ref();
        let format = ImageFileFormat::from_path(path)
            .ok_or(ImageError::UnsupportedFormat)?;
        let bytes = std::fs::read(path)?;
        Self::load_from_buffer(format, &bytes)
    }

    /// Saves the image to the file at `path` using the given format.
    ///
    /// `quality` is interpreted by the encoder (e.g. JPEG quality). BMP
    /// encoding is implemented in pure Rust; PNG/JPEG/WebP remain stubs
    /// until a codec crate is wired into `Cargo.toml`.
    pub fn save_to_file<P: AsRef<Path>>(
        &self,
        path: P,
        format: ImageFileFormat,
        quality: u8,
    ) -> Result<(), ImageError> {
        let buf = self.save_to_buffer(format, quality)?;
        std::fs::write(path, buf)?;
        Ok(())
    }

    /// Decodes an image from `data` using the codec identified by `format`.
    pub fn load_from_buffer(format: ImageFileFormat, data: &[u8]) -> Result<Self, ImageError> {
        match format {
            ImageFileFormat::Bmp => bmp::decode(data),
            ImageFileFormat::Png => {
                Err(ImageError::Unimplemented("Image::load_from_buffer (PNG)"))
            }
            ImageFileFormat::Jpeg => {
                Err(ImageError::Unimplemented("Image::load_from_buffer (JPEG)"))
            }
            ImageFileFormat::WebP => {
                Err(ImageError::Unimplemented("Image::load_from_buffer (WebP)"))
            }
        }
    }

    /// Encodes the image into a `Vec<u8>` using the given `format`. `quality`
    /// is only honoured by lossy codecs (JPEG/WebP) once they are wired in.
    pub fn save_to_buffer(
        &self,
        format: ImageFileFormat,
        _quality: u8,
    ) -> Result<Vec<u8>, ImageError> {
        if !self.is_valid() {
            return Err(ImageError::InvalidDimensions);
        }
        match format {
            ImageFileFormat::Bmp => bmp::encode(self),
            ImageFileFormat::Png => {
                Err(ImageError::Unimplemented("Image::save_to_buffer (PNG)"))
            }
            ImageFileFormat::Jpeg => {
                Err(ImageError::Unimplemented("Image::save_to_buffer (JPEG)"))
            }
            ImageFileFormat::WebP => {
                Err(ImageError::Unimplemented("Image::save_to_buffer (WebP)"))
            }
        }
    }

    #[inline]
    fn buffer_len(width: u32, height: u32, format: ImageFormat) -> usize {
        (width as usize) * (height as usize) * format.bytes_per_pixel()
    }

    #[inline]
    fn pixel_offset(&self, x: u32, y: u32) -> Option<usize> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let bpp = self.format.bytes_per_pixel();
        Some((y as usize * self.width as usize + x as usize) * bpp)
    }
}

/// Minimal pure-Rust BMP (BITMAPINFOHEADER) decoder/encoder.
///
/// Supports 8-bit paletted, 16-bit (RGB565 / bitfields), 24-bit and
/// 32-bit (BGRA / bitfields) uncompressed BMPs — the same set the
/// C++ implementation handles via Qt's qbmphandler-derived code.
mod bmp {
    use super::{Image, ImageError, ImageFormat};

    /// `BM` little-endian.
    const BMP_MAGIC: u16 = 0x4D42;

    /// Decodes a BMP buffer into an [`Image`] with [`ImageFormat::RGBA8`]
    /// pixel data. The on-disk BMP layout is preserved as-is in the raw
    /// byte buffer (BGRA for 32-bit, BGR for 24-bit, palette-indexed for
    /// <=8-bit); we then copy into a normalized RGBA8 buffer so callers
    /// have a uniform layout.
    pub fn decode(data: &[u8]) -> Result<Image, ImageError> {
        const FILE_HEADER_SIZE: usize = 14;
        const INFO_HEADER_SIZE: usize = 40;

        if data.len() < FILE_HEADER_SIZE + INFO_HEADER_SIZE {
            return Err(ImageError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "BMP file too small",
            )));
        }

        // Parse file header (little-endian u16 / u32).
        let file_type = read_u16(data, 0);
        if file_type != BMP_MAGIC {
            return Err(ImageError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Invalid BMP signature",
            )));
        }
        let _file_size = read_u32(data, 2);
        let _reserved1 = read_u16(data, 6);
        let _reserved2 = read_u16(data, 8);
        let pixel_offset = read_u32(data, 10) as usize;

        // Parse BITMAPINFOHEADER (40 bytes).
        let info_size = read_u32(data, FILE_HEADER_SIZE);
        let width_raw = read_i32(data, FILE_HEADER_SIZE + 4);
        let height_raw = read_i32(data, FILE_HEADER_SIZE + 8);
        let planes = read_u16(data, FILE_HEADER_SIZE + 12);
        let bit_count = read_u16(data, FILE_HEADER_SIZE + 14);
        let compression = read_u32(data, FILE_HEADER_SIZE + 16);
        let _size_image = read_u32(data, FILE_HEADER_SIZE + 20);
        let _x_pels = read_i32(data, FILE_HEADER_SIZE + 24);
        let _y_pels = read_i32(data, FILE_HEADER_SIZE + 28);
        let clr_used = read_u32(data, FILE_HEADER_SIZE + 32);
        let _clr_important = read_u32(data, FILE_HEADER_SIZE + 36);

        if planes != 1 {
            return Err(ImageError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "BMP planes must be 1",
            )));
        }

        let width = width_raw.unsigned_abs();
        let height = height_raw.unsigned_abs();
        if width == 0 || height == 0 {
            return Err(ImageError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Invalid BMP dimensions",
            )));
        }
        if width >= 65536 || height >= 65536 {
            return Err(ImageError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "BMP dimensions too large",
            )));
        }

        let flip_vertical = height_raw > 0;

        // Resolve bitfields masks. BITMAPINFOHEADER defaults for 16-bit are
        // RGB565; for 32-bit they are X8R8G8B8 (compression == 0).
        let (mut red_mask, mut green_mask, mut blue_mask, mut alpha_mask): (u32, u32, u32, u32) =
            match (compression, bit_count) {
                (0, 16) => (0x7C00, 0x03E0, 0x001F, 0),
                (0, 32) => (0x00FF0000, 0x0000FF00, 0x000000FF, 0),
                _ => (0, 0, 0, 0),
            };

        if compression == 3 || compression == 4 {
            // Masks live after the info header.
            let base = FILE_HEADER_SIZE + info_size as usize;
            if data.len() >= base + 12 {
                red_mask = read_u32(data, base);
                green_mask = read_u32(data, base + 4);
                blue_mask = read_u32(data, base + 8);
            }
            if compression == 4 && data.len() >= base + 16 {
                alpha_mask = read_u32(data, base + 12);
            }
        }

        // Build palette for sub-8-bit formats.
        let mut palette: Vec<u32> = Vec::new();
        if bit_count <= 8 {
            let num_colors = if clr_used > 0 {
                clr_used as usize
            } else {
                1usize << bit_count
            };
            if num_colors > 256 {
                return Err(ImageError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Invalid palette size",
                )));
            }
            let palette_base = FILE_HEADER_SIZE + info_size as usize;
            palette.reserve(num_colors);
            for i in 0..num_colors {
                let off = palette_base + i * 4;
                if off + 4 > data.len() {
                    return Err(ImageError::Io(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "BMP palette truncated",
                    )));
                }
                let b = data[off];
                let g = data[off + 1];
                let r = data[off + 2];
                palette.push(r as u32 | (g as u32) << 8 | (b as u32) << 16 | 0xFF000000);
            }
            if bit_count == 1 {
                palette.clear();
                palette.push(0xFFFFFFFF);
                palette.push(0xFF000000);
            }
        }

        if pixel_offset >= data.len() {
            return Err(ImageError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "BMP pixel data offset out of range",
            )));
        }
        let src = &data[pixel_offset..];

        // Compute row stride. 4-byte aligned per BMP spec.
        let bytes_per_pixel = (bit_count as u32 / 8).max(1);
        let row_size = ((width * bit_count as u32 + 31) / 32) * 4;

        let mut img = Image::new(width, height, ImageFormat::RGBA8);
        let dst = img.pixels_mut();

        for y in 0..height {
            let dst_y = if flip_vertical { height - 1 - y } else { y };
            let row_src_start = y as usize * row_size as usize;
            if row_src_start + row_size as usize > src.len() {
                return Err(ImageError::Io(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "BMP pixel data truncated",
                )));
            }
            let row_src = &src[row_src_start..row_src_start + row_size as usize];

            for x in 0..width {
                let dst_off = (dst_y as usize * width as usize + x as usize) * 4;
                let pixel = match bit_count {
                    1 => {
                        let byte = row_src[x as usize / 8];
                        let bit = 7 - (x % 8);
                        let idx = ((byte >> bit) & 1) as usize;
                        palette.get(idx).copied().unwrap_or(0xFF000000)
                    }
                    4 => {
                        let byte = row_src[x as usize / 2];
                        let nibble = if x % 2 == 0 { byte >> 4 } else { byte & 0x0F };
                        palette.get(nibble as usize).copied().unwrap_or(0xFF000000)
                    }
                    8 => palette
                        .get(row_src[x as usize] as usize)
                        .copied()
                        .unwrap_or(0xFF000000),
                    16 => decode_16bit(row_src, x, compression, red_mask, green_mask, blue_mask),
                    24 => {
                        let off = x as usize * 3;
                        let b = row_src[off];
                        let g = row_src[off + 1];
                        let r = row_src[off + 2];
                        r as u32 | (g as u32) << 8 | (b as u32) << 16 | 0xFF000000
                    }
                    32 => {
                        let off = x as usize * 4;
                        decode_32bit(
                            row_src,
                            off,
                            compression,
                            red_mask,
                            green_mask,
                            blue_mask,
                            alpha_mask,
                        )
                    }
                    _ => {
                        return Err(ImageError::Io(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!("Unsupported BMP bit depth: {bit_count}").as_str(),
                        )));
                    }
                };
                dst[dst_off..dst_off + 4].copy_from_slice(&pixel.to_ne_bytes());
            }
        }

        Ok(img)
    }

    /// Encodes an RGBA8 [`Image`] as a 24-bit BGR BMP (mirrors the
    /// `BMPBufferSaver` in the C++ implementation, which also writes
    /// 24-bit uncompressed BMPs).
    pub fn encode(image: &Image) -> Result<Vec<u8>, ImageError> {
        if image.width == 0 || image.height == 0 {
            return Err(ImageError::InvalidDimensions);
        }
        let width = image.width as usize;
        let height = image.height as usize;

        // Row size is padded to 4-byte boundaries.
        let row_size = ((width * 3 + 3) / 4) * 4;
        let image_size = row_size * height;
        let file_header_size = 14usize;
        let info_header_size = 40usize;
        let pixel_offset = file_header_size + info_header_size;
        let file_size = pixel_offset + image_size;

        let mut buf = vec![0u8; file_size];

        // BITMAPFILEHEADER.
        buf[0..2].copy_from_slice(&BMP_MAGIC.to_le_bytes());
        buf[2..6].copy_from_slice(&(file_size as u32).to_le_bytes());
        // reserved1, reserved2 already zero.
        buf[10..14].copy_from_slice(&(pixel_offset as u32).to_le_bytes());

        // BITMAPINFOHEADER.
        let base = file_header_size;
        buf[base..base + 4].copy_from_slice(&(info_header_size as u32).to_le_bytes());
        buf[base + 4..base + 8].copy_from_slice(&(width as i32).to_le_bytes());
        buf[base + 8..base + 12].copy_from_slice(&(height as i32).to_le_bytes());
        buf[base + 12..base + 14].copy_from_slice(&1u16.to_le_bytes()); // planes
        buf[base + 14..base + 16].copy_from_slice(&24u16.to_le_bytes()); // bit_count
        buf[base + 16..base + 20].copy_from_slice(&0u32.to_le_bytes()); // compression
        buf[base + 20..base + 24].copy_from_slice(&(image_size as u32).to_le_bytes());
        // remaining fields default to zero.

        // Pixels: BMP rows are stored bottom-up, channels BGR.
        let pixel_base = pixel_offset;
        let src_pixels = image.pixels();
        let src_bpp = image.format.bytes_per_pixel();
        for y in 0..height {
            let src_y = height - 1 - y;
            let src_row = &src_pixels[src_y * width * src_bpp..(src_y + 1) * width * src_bpp];
            let dst_row = &mut buf[pixel_base + y * row_size..pixel_base + (y + 1) * row_size];
            for x in 0..width {
                let rgba = read_pixel(src_row, x, src_bpp);
                dst_row[x * 3] = (rgba >> 16) as u8; // B
                dst_row[x * 3 + 1] = (rgba >> 8) as u8; // G
                dst_row[x * 3 + 2] = rgba as u8; // R
            }
        }

        Ok(buf)
    }

    fn read_u16(data: &[u8], offset: usize) -> u16 {
        u16::from_le_bytes([data[offset], data[offset + 1]])
    }

    fn read_u32(data: &[u8], offset: usize) -> u32 {
        u32::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ])
    }

    fn read_i32(data: &[u8], offset: usize) -> i32 {
        i32::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ])
    }

    /// Reads a single RGBA8-equivalent pixel from a source row whose stride
    /// is determined by `src_bpp`. Returns 0xFF000000 (opaque black) if the
    /// pixel cannot be represented.
    fn read_pixel(row: &[u8], x: usize, src_bpp: usize) -> u32 {
        let off = x * src_bpp;
        if off + src_bpp > row.len() {
            return 0xFF000000;
        }
        match src_bpp {
            1 => row[off] as u32 | 0xFF000000,
            2 => {
                let v = u16::from_ne_bytes([row[off], row[off + 1]]);
                if v == 0 {
                    0xFF000000
                } else {
                    // Best-effort grayscale promotion (alpha becomes 255).
                    let l = ((v & 0xFF) as u32 * 0x0101) >> 1;
                    l | (l << 8) | (l << 16) | 0xFF000000
                }
            }
            3 => {
                let r = row[off];
                let g = row[off + 1];
                let b = row[off + 2];
                r as u32 | (g as u32) << 8 | (b as u32) << 16 | 0xFF000000
            }
            4 => u32::from_ne_bytes([row[off], row[off + 1], row[off + 2], row[off + 3]]),
            _ => 0xFF000000,
        }
    }

    fn decode_16bit(
        row: &[u8],
        x: u32,
        compression: u32,
        red_mask: u32,
        green_mask: u32,
        blue_mask: u32,
    ) -> u32 {
        let off = x as usize * 2;
        if off + 2 > row.len() {
            return 0xFF000000;
        }
        let value = row[off] as u32 | ((row[off + 1] as u32) << 8);
        if compression == 3 {
            let r = scale_5_to_8((value & red_mask) >> trailing_zeros(red_mask));
            let g = scale_5_to_8((value & green_mask) >> trailing_zeros(green_mask));
            let b = scale_5_to_8((value & blue_mask) >> trailing_zeros(blue_mask));
            r as u32 | (g as u32) << 8 | (b as u32) << 16 | 0xFF000000
        } else {
            let r = scale_5_to_8((value >> 10) & 0x1F);
            let g = scale_5_to_8((value >> 5) & 0x1F);
            let b = scale_5_to_8(value & 0x1F);
            r as u32 | (g as u32) << 8 | (b as u32) << 16 | 0xFF000000
        }
    }

    fn decode_32bit(
        row: &[u8],
        off: usize,
        compression: u32,
        red_mask: u32,
        green_mask: u32,
        blue_mask: u32,
        alpha_mask: u32,
    ) -> u32 {
        if off + 4 > row.len() {
            return 0xFF000000;
        }
        let value = row[off] as u32
            | ((row[off + 1] as u32) << 8)
            | ((row[off + 2] as u32) << 16)
            | ((row[off + 3] as u32) << 24);

        if compression == 3 || compression == 4 {
            let r = scale_mask((value & red_mask) >> trailing_zeros(red_mask), red_mask);
            let g = scale_mask((value & green_mask) >> trailing_zeros(green_mask), green_mask);
            let b = scale_mask((value & blue_mask) >> trailing_zeros(blue_mask), blue_mask);
            let a = if compression == 4 && alpha_mask != 0 {
                scale_mask(
                    (value & alpha_mask) >> trailing_zeros(alpha_mask),
                    alpha_mask,
                )
            } else {
                0xFF
            };
            r as u32 | (g as u32) << 8 | (b as u32) << 16 | (a as u32) << 24
        } else {
            // Uncompressed 32-bit BMP is stored BGRA with an opaque alpha if
            // the alpha channel is unset (matches the C++ logic).
            let b = row[off];
            let g = row[off + 1];
            let r = row[off + 2];
            let a = if alpha_mask == 0 { 0xFF } else { row[off + 3] };
            r as u32 | (g as u32) << 8 | (b as u32) << 16 | (a as u32) << 24
        }
    }

    /// Scales a 5-bit channel value to 8-bit (replicates the high bit).
    fn scale_5_to_8(v: u32) -> u8 {
        ((v << 3) | (v >> 2)) as u8
    }

    /// Scales a channel value extracted from an arbitrary bitfield mask to
    /// 8 bits, using bit-replication when the source field is narrower.
    fn scale_mask(value: u32, mask: u32) -> u8 {
        let bits = mask.count_ones();
        if bits == 0 || bits >= 8 {
            return (value & 0xFF) as u8;
        }
        let value = value & ((1u32 << bits) - 1);
        let mut result = value << (8 - bits);
        let mut filled = 8 - bits;
        while filled < 8 {
            result |= result >> filled;
            filled <<= 1;
        }
        result as u8
    }

    fn trailing_zeros(mask: u32) -> u32 {
        if mask == 0 {
            0
        } else {
            mask.trailing_zeros()
        }
    }
}

/// Output file formats understood by [`Image::save_to_file`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageFileFormat {
    Png,
    Jpeg,
    WebP,
    Bmp,
}

impl ImageFileFormat {
    /// Returns the format corresponding to the file extension on `path`, or
    /// `None` if it is not recognised.
    pub fn from_path<P: AsRef<Path>>(path: P) -> Option<Self> {
        let ext = path.as_ref().extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "png" => Some(ImageFileFormat::Png),
            "jpg" | "jpeg" => Some(ImageFileFormat::Jpeg),
            "webp" => Some(ImageFileFormat::WebP),
            "bmp" => Some(ImageFileFormat::Bmp),
            _ => None,
        }
    }
}

/// A single-pixel value type-tagged by the channel width it carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pixel {
    /// 32-bit payload, used for RGBA8/BGRA8/IA16F/IA8/I16F/RGB8 reads.
    U32(u32),
    /// 64-bit payload, used for RGBA16F reads.
    U64(u64),
}

impl Default for Image {
    fn default() -> Self {
        Self {
            width: 0,
            height: 0,
            format: ImageFormat::RGBA8,
            data: Vec::new(),
        }
    }
}
