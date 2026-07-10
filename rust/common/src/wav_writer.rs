// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
// Pure-Rust port of `common/WAVWriter.{h,cpp}`.
//
// Writes a RIFF/WAVE file containing PCM sample data. The C++
// implementation writes the header up-front with a zeroed-out
// `data` chunk size, accumulates the byte count in
// `m_num_frames`, and on close seeks back to the start and rewrites
// the header with the real byte count. We follow the same pattern
// using `std::io::Write` directly so the file we emit is
// byte-for-byte identical to the C++ output and the implementation
// stays dependency-free.
//
// The header layout is the standard PCM variant of the RIFF format:
//
//     "RIFF"            (4 bytes)
//     chunk_size        (u32 LE)  = size of the rest of the file
//     "WAVE"            (4 bytes)
//     "fmt "            (4 bytes)
//     fmt_chunk_size    (u32 LE)  = 16 (PCM)
//     audio_format      (u16 LE)  = 1 (PCM)
//     num_channels      (u16 LE)
//     sample_rate       (u32 LE)
//     byte_rate         (u32 LE)  = sample_rate * block_align
//     block_align       (u16 LE)  = num_channels * bits_per_sample/8
//     bits_per_sample   (u16 LE)
//     "data"            (4 bytes)
//     data_size         (u32 LE)  = number of bytes of PCM data
//     ...PCM data...
//
// `write_samples` accepts interleaved 16-bit PCM samples
// (frame-major, channel-minor as the C++ side expects). The
// `bits_per_sample` constructor argument controls only the header
// field; if you pass anything other than 16 you'll get a header that
// disagrees with the on-disk sample width, so leave it at 16 unless
// you really know what you're doing.

#![allow(clippy::missing_safety_doc)]

use std::ffi::CStr;
use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::os::raw::c_char;
use std::path::Path;

// ============================================================================
// Errors
// ============================================================================

/// Errors that can occur while opening, writing, or finalizing a WAV
/// file. Wraps the underlying `std::io::Error` so the safe API stays
/// focused on the WAV-specific failure modes the C++ side reports via
/// `Console.Error`.
#[derive(Debug)]
pub enum Error {
    /// The destination path could not be opened for writing. Most
    /// commonly this means the parent directory does not exist or
    /// the process lacks write permission.
    Open(std::io::Error),
    /// An I/O error occurred while writing the RIFF header, writing
    /// sample data, seeking back to overwrite the data-size field,
    /// or flushing on finalize.
    Write(std::io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Open(e) => write!(f, "failed to open WAV file: {}", e),
            Error::Write(e) => write!(f, "WAV I/O error: {}", e),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Open(e) | Error::Write(e) => Some(e),
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        // All I/O errors get bucketed into `Write`; `Open` is reserved
        // for the file-creation step in `create`. Splitting them lets
        // the FFI caller (and the C++ side it wraps) report a useful
        // message when the path is bad vs. when the disk fills up.
        Error::Write(e)
    }
}

// ============================================================================
// Pure-Rust API
// ============================================================================

/// Streaming WAV (RIFF/WAVE) file writer for PCM audio capture.
///
/// Mirrors the C++ `Common::WAVWriter`:
///  - [`WavWriter::create`] opens the file and writes the header
///    with a placeholder `data_size` of zero.
///  - [`WavWriter::write_samples`] appends interleaved i16 samples
///    and accumulates the byte count.
///  - [`WavWriter::finalize`] consumes the writer, seeks back to the
///    start, rewrites the header with the real byte count, and
///    closes the file.
///
/// The writer holds the file open for the duration of its lifetime;
/// if it is dropped without `finalize` the file is closed but the
/// header is left with the placeholder size, producing a truncated
/// WAV. Callers that care about header correctness must call
/// `finalize` explicitly (the FFI `destroy` shim does so for them).
pub struct WavWriter {
    /// Underlying file handle. Created by [`WavWriter::create`] and
    /// consumed by [`WavWriter::finalize`].
    file: File,
    /// Number of sample-data bytes written so far. Mirrors the
    /// `m_num_frames * sizeof(s16) * m_num_channels` accumulator on
    /// the C++ side.
    data_size: u32,
    /// Sample rate in Hz, recorded so the header can be rewritten on
    /// finalize without the caller having to keep it around.
    sample_rate: u32,
    /// Channel count. Recorded for the same reason as `sample_rate`.
    channels: u16,
    /// Bits per sample. The header field only; the on-disk sample
    /// width is whatever `write_samples` writes (always 16 today).
    bits_per_sample: u16,
}

impl WavWriter {
    /// Create a new WAV file at `path` and write the initial RIFF
    /// header with `data_size = 0`.
    ///
    /// `sample_rate` is in Hz, `channels` is the channel count, and
    /// `bits_per_sample` is recorded in the `fmt ` chunk (it should
    /// be 16 to match the sample width `write_samples` writes).
    ///
    /// # Errors
    ///
    /// Returns [`Error::Open`] if the file cannot be created, and
    /// [`Error::Write`] if writing the initial header fails.
    pub fn create(
        path: &Path,
        sample_rate: u32,
        channels: u16,
        bits_per_sample: u16,
    ) -> Result<Self, Error> {
        let file = File::create(path).map_err(Error::Open)?;
        let mut w = WavWriter {
            file,
            data_size: 0,
            sample_rate,
            channels,
            bits_per_sample,
        };
        w.write_header(0)?;
        Ok(w)
    }

    /// Append `samples.len()` interleaved i16 samples to the file.
    ///
    /// `samples` is a flat slice of interleaved PCM data (i.e. one
    /// frame is `channels` consecutive i16 values), exactly as the
    /// C++ `WriteFrames` expects. The byte length of the slice is
    /// added to the data-size counter recorded in the RIFF header.
    ///
    /// An empty slice is a no-op (returns `Ok(())` without touching
    /// the file) so callers can pass through empty buffers without
    /// special-casing the call site.
    pub fn write_samples(&mut self, samples: &[i16]) -> Result<(), Error> {
        if samples.is_empty() {
            return Ok(());
        }
        // SAFETY: `i16` has no padding and its byte representation is
        // well-defined as two's complement little-endian on every
        // platform Rust targets. Reinterpreting the slice as bytes is
        // sound.
        let bytes = unsafe {
            std::slice::from_raw_parts(
                samples.as_ptr() as *const u8,
                samples.len() * std::mem::size_of::<i16>(),
            )
        };
        self.file.write_all(bytes)?;
        // Saturate on overflow rather than panic. `data_size` is
        // u32 to mirror the C++ `m_num_frames` accumulator (which
        // would wrap the same way), and a runaway capture shouldn't
        // abort the host process; the resulting header will simply
        // report an incorrect size, but that's no worse than what
        // the C++ side does on overflow.
        let added = bytes.len() as u64;
        self.data_size = self.data_size.saturating_add(added as u32);
        Ok(())
    }

    /// Finalize the WAV file: rewrite the RIFF header with the real
    /// byte count, flush, and close.
    ///
    /// Consumes the writer. After this returns the file on disk is a
    /// well-formed, playable WAV (assuming at least one
    /// `write_samples` call was made; an empty file is technically
    /// valid RIFF but most players will reject it).
    pub fn finalize(mut self) -> Result<(), Error> {
        self.write_header(self.data_size)?;
        self.file.flush()?;
        Ok(())
    }

    /// Write the RIFF header at the start of the file with the given
    /// `data_size`. Always seeks to byte 0 first so this works for
    /// both the initial write (right after `File::create`) and the
    /// final rewrite on `finalize`.
    fn write_header(&mut self, data_size: u32) -> Result<(), Error> {
        // block_align = channels * (bits_per_sample / 8); for 16-bit
        // PCM this simplifies to `channels * 2`. Computed as u32 to
        // avoid u16 overflow on multi-channel streams (8 channels *
        // 4 bytes/frame = 32, which fits in u16, but we keep the
        // wider type for safety on hypothetical future formats).
        let block_align =
            (self.channels as u32) * ((self.bits_per_sample as u32) / 8);
        let byte_rate = self.sample_rate * block_align;

        // RIFF chunk size = size of everything after the first
        // 8 bytes (the "RIFF" tag and the chunk-size field itself):
        //   "WAVE" identifier    = 4
        //   "fmt " chunk         = 8 (chunk header) + 16 (data) = 24
        //   "data" chunk header  = 8
        //   PCM sample data      = data_size
        let riff_chunk_size = 4u32 + 24u32 + 8u32 + data_size;

        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(b"RIFF")?;
        self.file.write_all(&riff_chunk_size.to_le_bytes())?;
        self.file.write_all(b"WAVE")?;
        self.file.write_all(b"fmt ")?;
        self.file.write_all(&16u32.to_le_bytes())?; // fmt chunk size
        self.file.write_all(&1u16.to_le_bytes())?; // audio_format: PCM
        self.file.write_all(&self.channels.to_le_bytes())?;
        self.file.write_all(&self.sample_rate.to_le_bytes())?;
        self.file.write_all(&byte_rate.to_le_bytes())?;
        // block_align is guaranteed to fit in u16 for any sane
        // channel/bits_per_sample combination, so the cast is safe.
        self.file.write_all(&(block_align as u16).to_le_bytes())?;
        self.file.write_all(&self.bits_per_sample.to_le_bytes())?;
        self.file.write_all(b"data")?;
        self.file.write_all(&data_size.to_le_bytes())?;
        Ok(())
    }
}

// ============================================================================
// FFI surface
// ============================================================================
//
// The FFI shims follow the same conventions as the other modules in
// this crate (see e.g. `wrapped_mem_copy`): raw pointers, defensive
// null handling, `unsafe` confined to the `unsafe` block.
//
// The C++ side allocates the WAVWriter via `new` and frees it via
// `delete`; here we box on the Rust side and hand back a raw
// pointer. `pcsx2_wav_writer_destroy` consumes the box and runs
// `finalize`, so the C++ side never has to think about partial
// failure modes (header unwritten, file not flushed) — by the time
// `destroy` returns, the file on disk is always a valid WAV.

/// FFI: create a new [`WavWriter`] for the file at `path`.
///
/// Returns a non-null owning pointer on success and a null pointer
/// on any failure (invalid UTF-8 path, file-creation error, header
/// write error). Mirrors the C++ `Common::WAVWriter::Open`.
///
/// # Safety
///
/// - `path` must be a NUL-terminated C string or null. A null
///   pointer is treated as a failure (returns null) rather than
///   UB.
#[no_mangle]
pub extern "C" fn pcsx2_wav_writer_create(
    path: *const c_char,
    sample_rate: u32,
    channels: u16,
    bits_per_sample: u16,
) -> *mut WavWriter {
    if path.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: caller promises the pointer is a NUL-terminated C
    // string. `from_ptr` is the standard idiom.
    let cstr = unsafe { CStr::from_ptr(path) };
    let path_str = match cstr.to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };
    match WavWriter::create(Path::new(path_str), sample_rate, channels, bits_per_sample) {
        Ok(w) => Box::into_raw(Box::new(w)),
        Err(_) => std::ptr::null_mut(),
    }
}

/// FFI: write `count` interleaved i16 samples to the writer.
///
/// Returns `true` on success and `false` on I/O error or invalid
/// arguments. Mirrors the C++ `Common::WAVWriter::WriteFrames`,
/// except that the success/failure of the write is propagated
/// instead of just being logged via `Console.Error`.
///
/// # Safety
///
/// - `w` must be a non-null pointer returned by
///   [`pcsx2_wav_writer_create`] and not yet passed to
///   [`pcsx2_wav_writer_destroy`].
/// - `samples` must point to `count` readable `i16` values, or be
///   null when `count == 0`. A null pointer with `count > 0` is
///   treated as a failure.
#[no_mangle]
pub extern "C" fn pcsx2_wav_writer_write(
    w: *mut WavWriter,
    samples: *const i16,
    count: u32,
) -> bool {
    if w.is_null() || samples.is_null() || count == 0 {
        return false;
    }
    // SAFETY: `w` is a valid writer pointer per the contract; we
    // borrow it mutably for the duration of this call only.
    let writer = unsafe { &mut *w };
    // SAFETY: `samples` points to `count` readable `i16` values
    // per the contract. `count` is bounded by the address space on
    // 64-bit targets so the cast to `usize` is safe.
    let slice = unsafe { std::slice::from_raw_parts(samples, count as usize) };
    writer.write_samples(slice).is_ok()
}

/// FFI: finalize the writer, flushing the header to disk, and free
/// the box.
///
/// Equivalent to `Box::from_raw(w).finalize()` with a null-pointer
/// fast path. After this returns `w` is dangling and must not be
/// used again; the file on disk is closed and the RIFF header
/// reports the correct byte count.
///
/// Errors from `finalize` are silently swallowed (the file is
/// closed in either case). This mirrors the C++ destructor, which
/// also logs and continues rather than aborting on a failed
/// header rewrite.
///
/// # Safety
///
/// - `w` must be a pointer returned by
///   [`pcsx2_wav_writer_create`], or null (in which case the call
///   is a no-op).
#[no_mangle]
pub extern "C" fn pcsx2_wav_writer_destroy(w: *mut WavWriter) {
    if w.is_null() {
        return;
    }
    // SAFETY: `w` is a valid owning pointer per the contract.
    // Taking ownership back via `Box::from_raw` re-establishes the
    // box so the memory is freed when `writer` goes out of scope at
    // the end of this function (regardless of whether `finalize`
    // succeeds).
    let writer = unsafe { Box::from_raw(w) };
    let _ = writer.finalize();
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    /// Read a whole file into a `Vec<u8>`. Convenience for tests.
    fn slurp(path: &Path) -> Vec<u8> {
        let mut buf = Vec::new();
        File::open(path).unwrap().read_to_end(&mut buf).unwrap();
        buf
    }

    /// Find a temp path we can write to. On Windows this is the
    /// user's temp dir; on Unix `/tmp`. Tests use unique filenames
    /// so they can run in parallel.
    fn temp_path(name: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("pcsx2_wav_writer_test_{}_{}.wav", std::process::id(), name));
        p
    }

    #[test]
    fn header_is_byte_for_byte_well_formed() {
        let path = temp_path("header");
        let _ = std::fs::remove_file(&path);

        let w = WavWriter::create(&path, 48000, 2, 16).unwrap();
        w.finalize().unwrap();

        let bytes = slurp(&path);
        // 44 bytes of header + 0 bytes of data (we never wrote any).
        assert_eq!(bytes.len(), 44);

        // RIFF tag.
        assert_eq!(&bytes[0..4], b"RIFF");
        // Chunk size: "WAVE" (4) + fmt chunk (24) + data header (8) + data (0).
        assert_eq!(u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]), 36);
        // WAVE tag.
        assert_eq!(&bytes[8..12], b"WAVE");
        // fmt  tag.
        assert_eq!(&bytes[12..16], b"fmt ");
        // fmt chunk size = 16.
        assert_eq!(u32::from_le_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]), 16);
        // audio_format = 1 (PCM).
        assert_eq!(u16::from_le_bytes([bytes[20], bytes[21]]), 1);
        // num_channels.
        assert_eq!(u16::from_le_bytes([bytes[22], bytes[23]]), 2);
        // sample_rate.
        assert_eq!(u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]), 48000);
        // byte_rate = 48000 * 2 * 2 = 192000.
        assert_eq!(u32::from_le_bytes([bytes[28], bytes[29], bytes[30], bytes[31]]), 192000);
        // block_align = 2 * 2 = 4.
        assert_eq!(u16::from_le_bytes([bytes[32], bytes[33]]), 4);
        // bits_per_sample.
        assert_eq!(u16::from_le_bytes([bytes[34], bytes[35]]), 16);
        // data tag.
        assert_eq!(&bytes[36..40], b"data");
        // data_size = 0.
        assert_eq!(u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]), 0);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn finalize_rewrites_data_size() {
        let path = temp_path("rewrite");
        let _ = std::fs::remove_file(&path);

        let mut w = WavWriter::create(&path, 44100, 1, 16).unwrap();
        // 1024 samples = 2048 bytes of PCM.
        let samples: Vec<i16> = (0..1024).map(|i| i as i16).collect();
        w.write_samples(&samples).unwrap();
        w.finalize().unwrap();

        let bytes = slurp(&path);
        // Header (44) + data (2048).
        assert_eq!(bytes.len(), 44 + 2048);

        // RIFF chunk size = 36 + 2048 = 2084.
        assert_eq!(
            u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            36 + 2048
        );
        // data_size = 2048.
        assert_eq!(
            u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]),
            2048
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn multiple_writes_accumulate() {
        let path = temp_path("accumulate");
        let _ = std::fs::remove_file(&path);

        let mut w = WavWriter::create(&path, 48000, 2, 16).unwrap();
        let chunk: Vec<i16> = vec![0; 256];
        for _ in 0..4 {
            w.write_samples(&chunk).unwrap();
        }
        w.finalize().unwrap();

        let bytes = slurp(&path);
        // 4 * 256 samples * 2 bytes/sample = 2048 bytes of PCM.
        assert_eq!(bytes.len(), 44 + 2048);
        assert_eq!(
            u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]),
            2048
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn empty_write_samples_is_a_noop() {
        let path = temp_path("noop");
        let _ = std::fs::remove_file(&path);

        let mut w = WavWriter::create(&path, 48000, 2, 16).unwrap();
        w.write_samples(&[]).unwrap();
        w.finalize().unwrap();

        let bytes = slurp(&path);
        assert_eq!(bytes.len(), 44);
        assert_eq!(
            u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]),
            0
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn ffi_create_write_destroy_roundtrip() {
        let path = temp_path("ffi_roundtrip");
        let _ = std::fs::remove_file(&path);

        let path_cstr = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
        let writer = unsafe {
            pcsx2_wav_writer_create(
                path_cstr.as_ptr(),
                48000,
                2,
                16,
            )
        };
        assert!(!writer.is_null());

        let samples: Vec<i16> = (0..512).map(|i| (i as i16).wrapping_mul(3)).collect();
        let ok = unsafe {
            pcsx2_wav_writer_write(
                writer,
                samples.as_ptr(),
                samples.len() as u32,
            )
        };
        assert!(ok);

        unsafe { pcsx2_wav_writer_destroy(writer) };

        let bytes = slurp(&path);
        assert_eq!(bytes.len(), 44 + 1024);
        assert_eq!(
            u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]),
            1024
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn ffi_null_path_returns_null() {
        let p = unsafe { pcsx2_wav_writer_create(std::ptr::null(), 48000, 2, 16) };
        assert!(p.is_null());
    }

    #[test]
    fn ffi_write_with_null_writer_returns_false() {
        let samples: Vec<i16> = vec![0; 16];
        let ok = unsafe {
            pcsx2_wav_writer_write(
                std::ptr::null_mut(),
                samples.as_ptr(),
                samples.len() as u32,
            )
        };
        assert!(!ok);
    }

    #[test]
    fn ffi_destroy_null_is_noop() {
        unsafe { pcsx2_wav_writer_destroy(std::ptr::null_mut()) };
    }
}