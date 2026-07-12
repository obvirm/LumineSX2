//! CHD (Compressed Hunks of Data) disc image reader
//!
//! Uses the pure-Rust `chd` crate. CHD files store data in "hunks" of `hunk_size` bytes
//! (typically 16384 or 32768). The CDVD layer reads 2048-byte sectors, so we translate
//! sector offset -> hunk index + hunk offset, and cache the last decoded hunk.

use crate::reader::{CDVDReader, CDVDError, Result, SECTOR_SIZE};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

/// CHD reader using the `chd` crate
pub struct ChdReader {
    chd: chd::Chd<BufReader<File>>,
    /// Total decompressed logical size in bytes
    size: u64,
    /// Bytes per hunk (header.hunk_size())
    hunk_size: u64,
    /// Last decoded hunk index (for caching)
    cached_hunk: Option<u32>,
    /// Scratch buffer for the current hunk (hunk_size bytes)
    hunk_buf: Vec<u8>,
    /// Scratch buffer for compressed hunk data
    cmp_buf: Vec<u8>,
}

impl ChdReader {
    /// Open a CHD file
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = File::open(path.as_ref())?;
        let reader = BufReader::new(file);
        let chd = chd::Chd::open(reader, None)
            .map_err(|e| CDVDError::Chd(format!("Failed to open CHD: {:?}", e)))?;

        let header = chd.header().clone();
        let size = header.logical_bytes();
        let hunk_size = header.hunk_size() as u64;

        if size == 0 || hunk_size == 0 {
            return Err(CDVDError::InvalidFormat("CHD has zero logical size".to_string()));
        }
        if size % SECTOR_SIZE as u64 != 0 {
            return Err(CDVDError::InvalidFormat(format!(
                "CHD logical size {} not multiple of {}",
                size, SECTOR_SIZE
            )));
        }

        Ok(Self {
            chd,
            size,
            hunk_size,
            cached_hunk: None,
            hunk_buf: vec![0u8; hunk_size as usize],
            cmp_buf: Vec::new(),
        })
    }

    /// Decode and cache the given hunk if not already cached.
    fn ensure_hunk(&mut self, hunk_index: u32) -> Result<()> {
        if self.cached_hunk == Some(hunk_index) {
            return Ok(());
        }

        let mut hunk = self
            .chd
            .hunk(hunk_index)
            .map_err(|e| CDVDError::Chd(format!("hunk({}) failed: {:?}", hunk_index, e)))?;

        hunk.read_hunk_in(&mut self.cmp_buf, &mut self.hunk_buf)
            .map_err(|e| CDVDError::Chd(format!("read_hunk_in({}) failed: {:?}", hunk_index, e)))?;

        self.cached_hunk = Some(hunk_index);
        Ok(())
    }
}

impl CDVDReader for ChdReader {
    fn read_sectors(&mut self, lsn: u32, buffer: &mut [u8]) -> Result<usize> {
        let offset = lsn as u64 * SECTOR_SIZE as u64;
        if offset >= self.size {
            return Err(CDVDError::InvalidFormat(format!(
                "LSN {} out of bounds (size={})",
                lsn, self.size
            )));
        }

        let bytes_to_read = buffer.len().min((self.size - offset) as usize);
        let mut read = 0usize;
        let mut cur_offset = offset;

        while read < bytes_to_read {
            let hunk_index = (cur_offset / self.hunk_size) as u32;
            let hunk_offset = (cur_offset % self.hunk_size) as usize;

            self.ensure_hunk(hunk_index)?;

            let available = self.hunk_buf.len() - hunk_offset;
            let take = (bytes_to_read - read).min(available);
            buffer[read..read + take].copy_from_slice(&self.hunk_buf[hunk_offset..hunk_offset + take]);

            read += take;
            cur_offset += take as u64;
        }

        Ok(read)
    }

    fn get_size(&self) -> u64 {
        self.size
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hunk_translation() {
        let hunk_size = 16384u64;
        let sectors_per_hunk = hunk_size / SECTOR_SIZE as u64;
        assert_eq!(sectors_per_hunk, 8);

        // Offset 32768 -> hunk 2, offset 0
        let offset = 32768u64;
        assert_eq!(offset / hunk_size, 2);
        assert_eq!(offset % hunk_size, 0);

        // LSN 10 -> byte 20480 -> hunk 1 (16384..32767), offset 4096
        let lsn10 = 10u64 * SECTOR_SIZE as u64;
        assert_eq!(lsn10 / hunk_size, 1);
        assert_eq!(lsn10 % hunk_size, 4096);
    }

    #[test]
    fn test_read_matches_iso() {
        let chd_path = r"E:\project\ps2\game\Black (USA).chd";
        let iso_path = r"E:\project\ps2\game\Black (USA).iso";
        if !std::path::Path::new(chd_path).exists() || !std::path::Path::new(iso_path).exists() {
            eprintln!("skipping: CHD/ISO test fixture not present");
            return;
        }

        let mut reader = ChdReader::open(chd_path).expect("open chd");
        // Read PVD (sector 16) + a chunk of 64 sectors, compare to raw ISO.
        let start = 16u32;
        let count = 64u32;
        let mut chd_buf = vec![0u8; (count as usize) * SECTOR_SIZE];
        let n = reader.read_sectors(start, &mut chd_buf).expect("read chd");
        assert_eq!(n, chd_buf.len());

        let mut f = std::fs::File::open(iso_path).unwrap();
        use std::io::{Read, Seek, SeekFrom};
        f.seek(SeekFrom::Start(start as u64 * SECTOR_SIZE as u64)).unwrap();
        let mut iso_buf = vec![0u8; chd_buf.len()];
        f.read_exact(&mut iso_buf).unwrap();

        assert_eq!(chd_buf, iso_buf, "CHD sector data must match ISO");
        eprintln!("CHD vs ISO (sectors {}+{}) match: OK", start, count);
    }
}
