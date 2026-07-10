//! CDVDReader trait - base interface for all disc image readers

use thiserror::Error;

/// Errors that can occur during CDVD operations
#[derive(Error, Debug)]
pub enum CDVDError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("Invalid format: {0}")]
    InvalidFormat(String),
    
    #[error("Unsupported feature: {0}")]
    Unsupported(String),
    
    #[error("CHD error: {0}")]
    Chd(String),
    
    #[error("Decompression error: {0}")]
    Decompression(String),
}

pub type Result<T> = std::result::Result<T, CDVDError>;

/// CD/DVD sector size (Mode 1)
pub const SECTOR_SIZE: usize = 2048;

/// Base trait for all CDVD readers
pub trait CDVDReader: Send + Sync {
    /// Read sectors starting at logical sector number (LSN)
    /// 
    /// Returns the number of bytes read (should be `buffer.len()` on success)
    fn read_sectors(&mut self, lsn: u32, buffer: &mut [u8]) -> Result<usize>;
    
    /// Get the total size of the disc in bytes
    fn get_size(&self) -> u64;
    
    /// Get the sector count
    fn get_sector_count(&self) -> u32 {
        (self.get_size() / SECTOR_SIZE as u64) as u32
    }
    
    /// Close the reader (optional cleanup)
    fn close(&mut self) {}
}
