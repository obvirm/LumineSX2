//! CHD (Compressed Hunks of Data) disc image reader

use crate::reader::{CDVDReader, CDVDError, Result, SECTOR_SIZE};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

/// CHD reader using the `chd` crate
pub struct ChdReader {
    chd: chd::Chd<BufReader<File>>,
    size: u64,
    hunk_size: u32,
}

impl ChdReader {
    /// Open a CHD file
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = File::open(path.as_ref())?;
        let reader = BufReader::new(file);
        let chd = chd::Chd::open(reader, None)
            .map_err(|e| CDVDError::Chd(format!("Failed to open CHD: {:?}", e)))?;
        
        let header = chd.header();
        let size = header.logical_bytes();
        let hunk_size = header.hunk_size();
        
        // Validate
        if size % SECTOR_SIZE as u64 != 0 {
            return Err(CDVDError::InvalidFormat(
                format!("CHD logical size {} is not a multiple of {}", size, SECTOR_SIZE)
            ));
        }
        
        Ok(Self { chd, size, hunk_size })
    }
}

impl CDVDReader for ChdReader {
    fn read_sectors(&mut self, _lsn: u32, _buffer: &mut [u8]) -> Result<usize> {
        // TODO: Fix chd crate Hunk API usage
        // The chd crate's Hunk type doesn't have obvious methods to get bytes
        // Need to investigate the correct API or use a different CHD library
        Err(CDVDError::Unsupported("CHD reading temporarily disabled - API mismatch".to_string()))
    }
    
    fn get_size(&self) -> u64 {
        self.size
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_chd_hunk_calculation() {
        // hunk_size = 16384, offset = 32768
        // hunk_index should be 2, hunk_offset should be 0
        let hunk_size = 16384u64;
        let offset = 32768u64;
        let hunk_index = offset / hunk_size;
        let hunk_offset = offset % hunk_size;
        
        assert_eq!(hunk_index, 2);
        assert_eq!(hunk_offset, 0);
    }
}
