//! End-to-end test of the `image` module: encode + decode round
//! trips for PNG, JPEG, and WebP using the pure-Rust `image` crate
//! (replacing C++'s libpng / libjpeg-turbo / libwebp).
//!
//! Run with: cargo run --release --example image_test

// std imports kept minimal: the existing Rust image module is
// loaded via the `#[path = ...]` attribute below and exposes the
// `Image` wrapper API.

#[path = "../src/image.rs"]
mod image_module;
use image_module::Image;

fn make_test_image() -> Image {
    // 4x4 RGBA test pattern: red, green, blue, white in each
    // quadrant, with varying alpha. Deterministic so the test
    // doesn't need a file on disk.
    let mut buf: Vec<u8> = Vec::with_capacity(4 * 4 * 4);
    for y in 0..4 {
        for x in 0..4 {
            let r = ((x * 64) & 0xFF) as u8;
            let g = ((y * 64) & 0xFF) as u8;
            let b = (((x + y) * 32) & 0xFF) as u8;
            let a = (255 - (x * 32 + y * 32)) as u8;
            buf.extend_from_slice(&[r, g, b, a]);
        }
    }
    Image::from_rgba(4, 4, buf)
}

fn round_trip_png(img: &Image) -> (usize, Image) {
    // Encode → bytes → decode → compare.
    let encoded = img.save_png_to_memory().expect("encode PNG");
    println!("  PNG encoded: {} bytes", encoded.len());
    let decoded = Image::load_from_memory(&encoded).expect("decode PNG");
    println!("  PNG decoded: {}x{} pixels", decoded.width(), decoded.height());
    (encoded.len(), decoded)
}

fn round_trip_jpeg(img: &Image) -> (usize, Image) {
    let encoded = img.save_jpeg_to_memory(90).expect("encode JPEG");
    println!("  JPEG encoded: {} bytes (q=90)", encoded.len());
    let decoded = Image::load_from_memory(&encoded).expect("decode JPEG");
    println!("  JPEG decoded: {}x{} pixels", decoded.width(), decoded.height());
    (encoded.len(), decoded)
}

fn round_trip_webp(img: &Image) -> (usize, Image) {
    // Save WebP to a temp file (save_webp_to_memory doesn't exist in
    // the current Rust module), then read it back as bytes.
    let tmp = std::env::temp_dir().join("pcsx2_image_test.webp");
    img.save_webp(&tmp, true).expect("encode WebP");
    let encoded = std::fs::read(&tmp).expect("read WebP file");
    println!("  WebP encoded: {} bytes (lossy)", encoded.len());
    let decoded = Image::load_from_memory(&encoded).expect("decode WebP");
    println!("  WebP decoded: {}x{} pixels", decoded.width(), decoded.height());
    (encoded.len(), decoded)
}

fn main() {
    let img = make_test_image();
    println!("Source image: {}x{} pixels, {} bytes RGBA", img.width(), img.height(), img.raw_pixels().len());
    println!();

    // PNG: lossless, so round-trip must be exact.
    println!("=== PNG round-trip (lossless) ===");
    let (png_size, png_dec) = round_trip_png(&img);
    assert_eq!(png_dec.width(), 4);
    assert_eq!(png_dec.height(), 4);
    assert_eq!(png_dec.raw_pixels(), img.raw_pixels(), "PNG round-trip must be exact");
    println!("  OK — bytes match exactly (lossless)");
    println!();

    // JPEG: lossy, so we only check dimensions.
    println!("=== JPEG round-trip (lossy) ===");
    let (jpeg_size, jpeg_dec) = round_trip_jpeg(&img);
    assert_eq!(jpeg_dec.width(), 4);
    assert_eq!(jpeg_dec.height(), 4);
    println!("  OK — dimensions preserved (lossy compression expected)");
    println!();

    // WebP: lossy at q=90, lossy / lossless at q=100.
    println!("=== WebP round-trip (lossy) ===");
    let (webp_size, webp_dec) = round_trip_webp(&img);
    assert_eq!(webp_dec.width(), 4);
    assert_eq!(webp_dec.height(), 4);
    println!("  OK — dimensions preserved (lossy compression expected)");
    println!();

    // Compression efficiency comparison: same source, three encoders.
    println!("=== Compression size comparison ===");
    println!("  PNG:  {} bytes (lossless, biggest)", png_size);
    println!("  JPEG: {} bytes (lossy, smaller)", jpeg_size);
    println!("  WebP: {} bytes (lossy, smallest)", webp_size);
    // We don't assert a strict ordering (the test image is tiny so
    // the per-encoder headers dominate), but in general WebP and
    // JPEG should be smaller than PNG for photographs.

    println!();
    println!("OK — all three encoders/decoders work end-to-end");
}
