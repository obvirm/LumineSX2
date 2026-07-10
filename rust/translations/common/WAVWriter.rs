// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust translation of PCSX2's `common/WAVWriter.h`.
//!
//! [`WAVWriter`] is a small RAII helper that opens a file, writes a
//! RIFF/WAVE header sized for the requested format, accepts interleaved
//! `i16` PCM samples, and patches the size fields in the header on drop.
//!
//! The header is written eagerly in [`WAVWriter::new`] so that the file on
//! disk is always a well-formed (if empty) RIFF/WAVE container even if the
//! writer is dropped before any samples are written. The `data` subchunk
//! size and the outer RIFF chunk size are patched on drop to reflect the
//! number of samples that were actually written.
//!
//! Only `std` is used: a `std::fs::File` holds the file handle, and the
//! `std::io::{Seek, SeekFrom, Write}` traits drive both the initial header
//! emission and the trailing size patches in `Drop`.

use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

/// PCM audio writer that produces a RIFF/WAVE (`.wav`) file.
///
/// On creation, the writer opens the file at `path` and writes a
/// RIFF/WAVE header sized for the given format. [`write_samples`](Self::write_samples)
/// appends interleaved PCM samples to the `data` chunk. When the writer
/// is dropped, the size fields in the RIFF and `data` subchunk headers
/// are patched to reflect the number of samples actually written.
///
/// The PCM bit depth recorded in the header is `bits_per_sample`, but
/// [`write_samples`](Self::write_samples) always consumes `i16` samples.
/// Callers wishing to write a different bit depth must convert their
/// samples to `i16` first; the `bits_per_sample` field is then a label
/// that the caller is responsible for matching to the data layout.
pub struct WAVWriter {
    file: File,
    sample_rate: u32,
    num_channels: u16,
    bits_per_sample: u16,
    num_frames: u64,
}

impl WAVWriter {
    /// Offset of the RIFF chunk size field in the 44-byte header.
    const RIFF_SIZE_OFFSET: u64 = 4;
    /// Offset of the `data` subchunk size field in the 44-byte header.
    const DATA_SIZE_OFFSET: u64 = 40;
    /// Total size in bytes of the WAV header written up front.
    const HEADER_SIZE: u64 = 44;
    /// PCM audio format code recorded in the `fmt ` subchunk.
    const PCM_FORMAT: u16 = 1;
    /// Size in bytes of the `fmt ` subchunk for PCM data.
    const FMT_SUBCHUNK_SIZE: u32 = 16;

    /// Open `path` for writing and write a RIFF/WAVE header for a stream of
    /// interleaved `i16` PCM samples at `sample_rate` Hz with `num_channels`
    /// channels and `bits_per_sample` bits per sample.
    pub fn new(
        path: &Path,
        sample_rate: u32,
        num_channels: u16,
        bits_per_sample: u16,
    ) -> Result<Self, std::io::Error> {
        let mut file = File::create(path)?;

        // Promote to u64 for the multiplication so the intermediate product
        // can't overflow even for unusual format combinations; the WAV
        // header only stores 32-bit fields, so the final value is truncated.
        let byte_rate = (u64::from(sample_rate)
            * u64::from(num_channels)
            * u64::from(bits_per_sample)
            / 8) as u32;
        let block_align = (u32::from(num_channels) * u32::from(bits_per_sample) / 8) as u16;

        file.write_all(b"RIFF")?;
        file.write_all(&0u32.to_le_bytes())?; // RIFF chunk size placeholder, patched on drop.
        file.write_all(b"WAVE")?;
        file.write_all(b"fmt ")?;
        file.write_all(&Self::FMT_SUBCHUNK_SIZE.to_le_bytes())?;
        file.write_all(&Self::PCM_FORMAT.to_le_bytes())?;
        file.write_all(&num_channels.to_le_bytes())?;
        file.write_all(&sample_rate.to_le_bytes())?;
        file.write_all(&byte_rate.to_le_bytes())?;
        file.write_all(&block_align.to_le_bytes())?;
        file.write_all(&bits_per_sample.to_le_bytes())?;
        file.write_all(b"data")?;
        file.write_all(&0u32.to_le_bytes())?; // data subchunk size placeholder, patched on drop.

        debug_assert_eq!(file.stream_position()?, Self::HEADER_SIZE);

        Ok(Self {
            file,
            sample_rate,
            num_channels,
            bits_per_sample,
            num_frames: 0,
        })
    }

    /// Append `pcm` interleaved PCM samples to the `data` chunk.
    ///
    /// `pcm.len()` should be a multiple of [`num_channels`](Self::num_channels).
    /// The number of frames written is added to the internal frame counter;
    /// the `data` subchunk size and the RIFF chunk size are patched on drop.
    pub fn write_samples(&mut self, pcm: &[i16]) -> Result<(), std::io::Error> {
        // Write each sample as little-endian i16 so that the on-disk layout
        // is correct regardless of the host platform's native endianness.
        for &sample in pcm {
            self.file.write_all(&sample.to_le_bytes())?;
        }
        self.num_frames += pcm.len() as u64 / u64::from(self.num_channels);
        Ok(())
    }

    /// Returns the sample rate the writer was opened with, in Hz.
    #[must_use]
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Returns the channel count the writer was opened with.
    #[must_use]
    pub fn num_channels(&self) -> u16 {
        self.num_channels
    }

    /// Returns the bits-per-sample value recorded in the header.
    #[must_use]
    pub fn bits_per_sample(&self) -> u16 {
        self.bits_per_sample
    }

    /// Returns the number of sample frames (one sample per channel) written
    /// so far.
    #[must_use]
    pub fn num_frames(&self) -> u64 {
        self.num_frames
    }
}

impl Drop for WAVWriter {
    fn drop(&mut self) {
        // Total bytes of PCM data that have been written to the file.
        let data_size = (self.num_frames
            * u64::from(self.num_channels)
            * u64::from(self.bits_per_sample)
            / 8) as u32;
        // RIFF chunk size = total file size - 8. With a 44-byte header and a
        // data chunk of `data_size` bytes, that is 36 + data_size.
        let riff_size = 36u32.saturating_add(data_size);

        // Any I/O error here is unrecoverable (we are in `Drop` and the
        // caller has no way to observe it) so it is intentionally swallowed.
        // The header will still describe a well-formed RIFF/WAVE container
        // even if the size fields end up stale.
        let _ = self.file.seek(SeekFrom::Start(Self::DATA_SIZE_OFFSET));
        let _ = self.file.write_all(&data_size.to_le_bytes());
        let _ = self.file.seek(SeekFrom::Start(Self::RIFF_SIZE_OFFSET));
        let _ = self.file.write_all(&riff_size.to_le_bytes());
    }
}
