//! Blockdump disc image reader (PCSX2 debug format)

use crate::reader::{CDVDReader, CDVDError, Result};
use std::path::Path;

/// Blockdump reader (PCSX2 internal debug format)
pub struct BlockdumpReader {
    // TODO: Implement blockdump format
}

impl BlockdumpReader {
    /// Open a blockdump file
    pub fn open<P: AsRef<Path>>(_path: P) -> Result<Self> {
        Err(CDVDError::Unsupported("Blockdump format not yet implemented".to_string()))
    }
}

impl CDVDReader for BlockdumpReader {
    fn read_sectors(&mut self, _lsn: u32, _buffer: &mut [u8]) -> Result<usize> {
        Err(CDVDError::Unsupported("Blockdump format not yet implemented".to_string()))
    }
    
    fn get_size(&self) -> u64 {
        0
    }
}
