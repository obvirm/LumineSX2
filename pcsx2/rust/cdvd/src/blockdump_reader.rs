//! Blockdump (PCSX2 debug format) disc image reader
//!
//! Blockdump files (`.dump` / `.blockdump`) are sparse captures used for debugging.
//! Format from `BlockDumpFileReader.cpp`:
//!   - 4-byte signature "BDV2"
//!   - u32 dblocksize
//!   - u32 blocks
//!   - u32 blockofs
//!   - Then for each entry in the table: u32 lsn, followed by `dblocksize` bytes of data

use crate::reader::{CDVDReader, CDVDError, Result, SECTOR_SIZE};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const BLOCKDUMP_SIGNATURE: &[u8; 4] = b"BDV2";
const BLOCKDUMP_HEADER_SIZE: u64 = 16; // 4 sig + 3*u32 = 16

/// Blockdump reader
pub struct BlockdumpReader {
    file: File,
    /// Decompressed block size (often 2048 or larger)
    dblocksize: u32,
    /// Number of blocks in the original disc
    blocks: u32,
    /// Table mapping table index -> LSN
    dtable: Vec<u32>,
}

impl BlockdumpReader {
    /// Open a blockdump file
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let mut file = File::open(path.as_ref())?;

        let mut sig = [0u8; 4];
        file.read_exact(&mut sig)?;
        if &sig != BLOCKDUMP_SIGNATURE {
            return Err(CDVDError::InvalidFormat(
                "Block dump signature is invalid".to_string(),
            ));
        }

        let dblocksize = read_u32(&mut file)?;
        let blocks = read_u32(&mut file)?;
        let blockofs = read_u32(&mut file)?;
        let _ = blockofs; // unused in read, kept for completeness

        if dblocksize == 0 {
            return Err(CDVDError::InvalidFormat(
                "Block dump has zero block size".to_string(),
            ));
        }

        let flen = file.seek(SeekFrom::End(0))?;
        let datalen = flen - BLOCKDUMP_HEADER_SIZE;
        if datalen % (dblocksize as u64 + 4) != 0 {
            return Err(CDVDError::InvalidFormat(
                "Block dump data length mismatch".to_string(),
            ));
        }

        let dtablesize = (datalen / (dblocksize as u64 + 4)) as usize;
        let mut dtable = vec![0u32; dtablesize];

        file.seek(SeekFrom::Start(BLOCKDUMP_HEADER_SIZE))?;
        let mut buf = vec![0u8; (dblocksize as usize + 4) * 1024];
        let mut off = 0usize;
        let mut has = 0usize;
        let mut i = 0usize;

        loop {
            has = file.read(&mut buf)?;
            if has == 0 {
                break;
            }
            while i < dtablesize && off < has {
                let lsn = u32::from_le_bytes([
                    buf[off],
                    buf[off + 1],
                    buf[off + 2],
                    buf[off + 3],
                ]);
                dtable[i] = lsn;
                i += 1;
                off += 4 + dblocksize as usize;
            }
            if i >= dtablesize {
                break;
            }
            off = off.saturating_sub(has);
        }

        Ok(Self {
            file,
            dblocksize,
            blocks,
            dtable,
        })
    }

    fn read_block(&mut self, lsn: u32, dst: &mut [u8]) -> Result<usize> {
        // Find lsn in dtable
        if let Some(pos) = self.dtable.iter().position(|&x| x == lsn) {
            let data_pos = BLOCKDUMP_HEADER_SIZE + (pos as u64) * (self.dblocksize as u64 + 4) + 4;
            self.file.seek(SeekFrom::Start(data_pos))?;
            let n = self.file.read(&mut dst[..self.dblocksize as usize])?;
            Ok(n)
        } else {
            // Not in dump - return error (missing sector)
            Err(CDVDError::InvalidFormat(format!(
                "Blockdump: LSN {} not in dump",
                lsn
            )))
        }
    }
}

fn read_u32(f: &mut File) -> Result<u32> {
    let mut b = [0u8; 4];
    f.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

impl CDVDReader for BlockdumpReader {
    fn read_sectors(&mut self, lsn: u32, buffer: &mut [u8]) -> Result<usize> {
        let bytes_to_read = buffer.len().min(self.dblocksize as usize);
        let read = self.read_block(lsn, buffer)?;
        Ok(read.min(bytes_to_read))
    }

    fn get_size(&self) -> u64 {
        (self.blocks as u64) * (self.dblocksize as u64)
    }
}
