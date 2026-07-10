//! CSO/CISO (Compressed ISO) disc image reader

use crate::reader::{CDVDReader, CDVDError, Result};
use std::path::Path;

/// CSO/CISO compressed ISO reader
pub struct CsoReader {
    // TODO: Implement CSO format
}

impl CsoReader {
    /// Open a CSO/CISO file
    pub fn open<P: AsRef<Path>>(_path: P) -> Result<Self> {
        Err(CDVDError::Unsupported("CSO format not yet implemented".to_string()))
    }
}

impl CDVDReader for CsoReader {
    fn read_sectors(&mut self, _lsn: u32, _buffer: &mut [u8]) -> Result<usize> {
        Err(CDVDError::Unsupported("CSO format not yet implemented".to_string()))
    }
    
    fn get_size(&self) -> u64 {
        0
    }
}
