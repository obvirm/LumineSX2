//! CSO/CISO (Compressed ISO) disc image reader
//!
//! Implements the CSO/ZSO format used by PCSX2. Each frame is either stored raw or
//! compressed with zlib (CSO) or LZ4 (ZSO). See `CsoFileReader.cpp` in the C++ tree.

use crate::reader::{CDVDReader, CDVDError, Result, SECTOR_SIZE};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

// CSO header constants
const CSO_MAGIC_C: u8 = b'C';
const CSO_MAGIC_Z: u8 = b'Z'; // ZSO uses LZ4
const CSO_MAGIC_I: u8 = b'I';
const CSO_MAGIC_S: u8 = b'S';
const CSO_MAGIC_O: u8 = b'O';

const CSO_INDEX_ENTRY_COMPRESSED: u32 = 0x8000_0000;

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct CsoHeader {
    magic: [u8; 4],
    header_size: u32,
    total_bytes: u64,
    frame_size: u32,
    ver: u8,
    align: u8,
    reserved: [u8; 2],
}

impl CsoHeader {
    fn is_valid(&self) -> bool {
        let m0 = self.magic[0];
        (m0 == CSO_MAGIC_C || m0 == CSO_MAGIC_Z)
            && self.magic[1] == CSO_MAGIC_I
            && self.magic[2] == CSO_MAGIC_S
            && self.magic[3] == CSO_MAGIC_O
    }

    fn is_zso(&self) -> bool {
        self.magic[0] == CSO_MAGIC_Z
    }
}

/// CSO/CISO compressed ISO reader
pub struct CsoReader {
    file: File,
    /// Decompressed frame size in bytes (power of two, >= 2048)
    frame_size: u32,
    /// log2(frame_size)
    frame_shift: u32,
    /// alignment shift for index values
    index_shift: u32,
    /// total decompressed size in bytes
    total_size: u64,
    /// true if ZSO (LZ4), false if CSO (zlib)
    use_lz4: bool,
    /// Raw frame index (num_frames + 1 entries)
    index: Vec<u32>,
    /// Scratch buffer for reading compressed frames
    read_buffer: Vec<u8>,
    /// zlib inflate stream state
    z_stream: flate2::Decompress,
}

impl CsoReader {
    /// Open a CSO/CISO file
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let mut file = File::open(path.as_ref())?;

        let mut hdr_bytes = [0u8; std::mem::size_of::<CsoHeader>()];
        file.read_exact(&mut hdr_bytes)?;

        // SAFETY: CsoHeader is a packed repr(C) struct; we read raw bytes and reinterpret.
        let hdr: CsoHeader = unsafe { std::mem::transmute_copy(&hdr_bytes) };

        if !hdr.is_valid() {
            return Err(CDVDError::InvalidFormat("File is not a CSO or ZSO".to_string()));
        }
        if hdr.ver > 1 {
            return Err(CDVDError::InvalidFormat("Only CSOv1 files are supported".to_string()));
        }
        if hdr.frame_size == 0 || (hdr.frame_size & (hdr.frame_size - 1)) != 0 {
            return Err(CDVDError::InvalidFormat("CSO frame size must be a power of two".to_string()));
        }
        if hdr.frame_size < SECTOR_SIZE as u32 {
            return Err(CDVDError::InvalidFormat(
                "CSO frame size must be at least one sector".to_string(),
            ));
        }

        // Compute frame shift
        let mut frame_shift = 0u32;
        let mut fs = hdr.frame_size;
        while fs > 1 {
            fs >>= 1;
            frame_shift += 1;
        }

        let use_lz4 = hdr.is_zso();
        let total_size = hdr.total_bytes;

        // Number of frames (round up)
        let num_frames = ((total_size + hdr.frame_size as u64 - 1) / hdr.frame_size as u64) as u32 + 1;
        let index_size = num_frames as usize;

        let mut index = vec![0u32; index_size];
        file.read_exact(unsafe {
            std::slice::from_raw_parts_mut(index.as_mut_ptr() as *mut u8, index_size * 4)
        })?;

        // Read buffer large enough for frame + alignment padding
        let read_buffer_size = (hdr.frame_size + (1u32 << hdr.align)) as usize;
        let read_buffer = vec![0u8; read_buffer_size.max(SECTOR_SIZE)];

        // CSO frames are raw DEFLATE (no zlib header); use raw inflate.
        let z_stream = flate2::Decompress::new(false);

        Ok(Self {
            file,
            frame_size: hdr.frame_size,
            frame_shift,
            index_shift: hdr.align as u32,
            total_size,
            use_lz4,
            index,
            read_buffer,
            z_stream,
        })
    }

    /// Read a single frame (chunk) into `dst` (frame_size bytes max).
    fn read_frame(&mut self, frame: u32, dst: &mut [u8]) -> Result<usize> {
        let idx0 = self.index[frame as usize] & !CSO_INDEX_ENTRY_COMPRESSED;
        let idx1 = self.index[frame as usize + 1] & !CSO_INDEX_ENTRY_COMPRESSED;

        // Bit 0x80000000 in the index entry means the frame is UNCOMPRESSED;
        // clear bit means compressed (matches PCSX2 CsoFileReader / maxcso spec).
        let compressed = (self.index[frame as usize] & CSO_INDEX_ENTRY_COMPRESSED) == 0;
        let frame_raw_pos = (idx0 as u64) << self.index_shift;
        let frame_raw_size = ((idx1 - idx0) as u64) << self.index_shift;

        self.file.seek(SeekFrom::Start(frame_raw_pos))?;
        let read_raw = self.file.read(&mut self.read_buffer)?;
        if read_raw == 0 {
            return Err(CDVDError::InvalidFormat(format!(
                "CSO: failed to read frame {} data",
                frame
            )));
        }

        let src = &self.read_buffer[..read_raw.min(frame_raw_size as usize)];

        if !compressed {
            let n = src.len().min(dst.len());
            dst[..n].copy_from_slice(&src[..n]);
            Ok(n)
        } else if self.use_lz4 {
            lz4_flex::decompress_into(src, dst)
                .map_err(|e| CDVDError::Decompression(format!("LZ4: {:?}", e)))
        } else {
            // Each CSO frame is an independent raw-deflate stream.
            let mut z = flate2::Decompress::new(false);
            z.decompress(src, dst, flate2::FlushDecompress::Finish)
                .map(|_status| dst.len())
                .map_err(|e| CDVDError::Decompression(format!("zlib: {:?}", e)))
        }
    }
}

impl CDVDReader for CsoReader {
    fn read_sectors(&mut self, lsn: u32, buffer: &mut [u8]) -> Result<usize> {
        let offset = lsn as u64 * SECTOR_SIZE as u64;
        if offset >= self.total_size {
            return Err(CDVDError::InvalidFormat(format!(
                "LSN {} out of bounds (size={})",
                lsn, self.total_size
            )));
        }

        let bytes_to_read = buffer.len().min((self.total_size - offset) as usize);
        let mut read = 0usize;
        let mut cur_offset = offset;

        // Per-frame scratch (frame_size)
        let mut frame_buf = vec![0u8; self.frame_size as usize];

        while read < bytes_to_read {
            let frame = (cur_offset >> self.frame_shift) as u32;
            let frame_offset = (cur_offset & (self.frame_size as u64 - 1)) as usize;

            let decoded = self.read_frame(frame, &mut frame_buf)?;

            let available = decoded.saturating_sub(frame_offset);
            let take = (bytes_to_read - read).min(available);
            if take == 0 {
                break;
            }
            buffer[read..read + take]
                .copy_from_slice(&frame_buf[frame_offset..frame_offset + take]);

            read += take;
            cur_offset += take as u64;
        }

        // Reset zlib stream for next call if used
        if !self.use_lz4 {
            self.z_stream = flate2::Decompress::new(true);
        }

        Ok(read)
    }

    fn get_size(&self) -> u64 {
        self.total_size
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CDVDReader;

    #[test]
    fn test_read_matches_iso() {
        let cso_path = r"E:\project\ps2\game\Black (USA).cso";
        let iso_path = r"E:\project\ps2\game\Black (USA).iso";
        if !std::path::Path::new(cso_path).exists() || !std::path::Path::new(iso_path).exists() {
            eprintln!("skipping: CSO/ISO test fixture not present");
            return;
        }

        let mut reader = CsoReader::open(cso_path).expect("open cso");
        let start = 16u32;
        let count = 64u32;
        let mut cso_buf = vec![0u8; (count as usize) * SECTOR_SIZE];
        let n = reader.read_sectors(start, &mut cso_buf).expect("read cso");
        assert_eq!(n, cso_buf.len());

        let mut f = std::fs::File::open(iso_path).unwrap();
        use std::io::{Read, Seek, SeekFrom};
        f.seek(SeekFrom::Start(start as u64 * SECTOR_SIZE as u64)).unwrap();
        let mut iso_buf = vec![0u8; cso_buf.len()];
        f.read_exact(&mut iso_buf).unwrap();

        assert_eq!(cso_buf, iso_buf, "CSO sector data must match ISO");
        eprintln!("CSO vs ISO (sectors {}+{}) match: OK", start, count);
    }
}
