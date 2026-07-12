//! ISO9660 disc image reader

use crate::reader::{CDVDReader, CDVDError, Result, SECTOR_SIZE};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Simple ISO9660 raw image reader
pub struct IsoReader {
    file: File,
    size: u64,
}

impl IsoReader {
    /// Open an ISO file
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = File::open(path.as_ref())?;
        let size = file.metadata()?.len();
        
        // Validate: size must be multiple of 2048
        if size % SECTOR_SIZE as u64 != 0 {
            return Err(CDVDError::InvalidFormat(
                format!("ISO size {} is not a multiple of {}", size, SECTOR_SIZE)
            ));
        }
        
        Ok(Self { file, size })
    }
}

impl CDVDReader for IsoReader {
    fn read_sectors(&mut self, lsn: u32, buffer: &mut [u8]) -> Result<usize> {
        let offset = lsn as u64 * SECTOR_SIZE as u64;
        if offset >= self.size {
            // C++ FlatFileReader membiarkan baca di luar batas (return 0 bytes).
            // Kita ikut behavior itu: kembalikan 0 bukan error, biar caller fallback.
            return Ok(0);
        }

        self.file.seek(SeekFrom::Start(offset))?;

        // Baca sebanyak yang tersedia (clamp ke sisa size), jangan error kalau
        // buffer melebihi ujung file.
        let available = (self.size - offset) as usize;
        let to_read = buffer.len().min(available);
        let bytes_read = self.file.read(&mut buffer[..to_read])?;
        Ok(bytes_read)
    }
    
    fn get_size(&self) -> u64 {
        self.size
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_iso_sector_size() {
        assert_eq!(SECTOR_SIZE, 2048);
    }
}
