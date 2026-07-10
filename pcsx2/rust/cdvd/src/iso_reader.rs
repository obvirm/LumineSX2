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
            return Err(CDVDError::InvalidFormat(
                format!("LSN {} out of bounds (size={})", lsn, self.size)
            ));
        }
        
        self.file.seek(SeekFrom::Start(offset))?;
        
        let bytes_to_read = buffer.len().min((self.size - offset) as usize);
        self.file.read_exact(&mut buffer[..bytes_to_read])?;
        
        Ok(bytes_to_read)
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
