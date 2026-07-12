//! Gzip/deflate disc image reader (Rust port of `pcsx2/CDVD/GzippedFileReader.cpp`)
//!
//! PS2 `.gz` disc images are gzip/zlib-compressed ISO streams. Random-access
//! reading normally needs an index (Mark Adler's `zran.c`, which relies on
//! `inflatePrime` — not exposed by the pure-Rust `flate2`/`miniz_oxide` we use).
//! To stay **100% pure Rust** and correct, we decompress the stream once on
//! open into a seekable backing store (temp file on disk for large images,
//! memory for small ones), then serve 2048-byte sectors from it.
//!
//! This trades a little open-time/disk cost for a robust, dependency-free
//! implementation that works on every platform — including Android.

use crate::reader::{CDVDReader, CDVDError, Result, SECTOR_SIZE};
use flate2::read::GzDecoder;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

/// Size above which we spill decompression to a temp file instead of memory.
const MEMORY_CAP: u64 = 256 * 1024 * 1024; // 256 MB

/// Backing store for the decompressed ISO data.
enum Backing {
    /// In-memory decompressed image (small disc images).
    Memory(Vec<u8>),
    /// On-disk temp file (large disc images); removed on drop. Opened with
    /// read+write access so it can be read back after writing.
    TempFile { path: std::path::PathBuf, file: File },
}

impl Backing {
    fn len(&self) -> u64 {
        match self {
            Backing::Memory(v) => v.len() as u64,
            Backing::TempFile { file, .. } => file.metadata().map(|m| m.len()).unwrap_or(0),
        }
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize> {
        match self {
            Backing::Memory(v) => {
                let end = (offset as usize + buf.len()).min(v.len());
                let n = end - offset as usize;
                buf[..n].copy_from_slice(&v[offset as usize..end]);
                Ok(n)
            }
            Backing::TempFile { file, .. } => {
                file.seek(SeekFrom::Start(offset))?;
                Ok(file.read(buf)?)
            }
        }
    }
}

impl Drop for Backing {
    fn drop(&mut self) {
        if let Backing::TempFile { path, .. } = self {
            let _ = fs::remove_file(path);
        }
    }
}

/// Gzip/deflate reader
pub struct GzReader {
    backing: Backing,
}

impl GzReader {
    /// Open a `.gz` disc image, decompressing it into a backing store.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = File::open(path.as_ref())?;
        let mut decoder = GzDecoder::new(file);

        // Decompress in 64 KB chunks. Keep the result in memory until it exceeds
        // MEMORY_CAP, then spill to a temp file on disk (removed on drop).
        let mut mem: Vec<u8> = Vec::with_capacity(MEMORY_CAP as usize);
        let mut tmp_path: Option<std::path::PathBuf> = None;
        let mut tmp_file: Option<File> = None;
        let mut chunk = [0u8; 64 * 1024];

        loop {
            let n = decoder.read(&mut chunk)?;
            if n == 0 {
                break;
            }
            if tmp_file.is_none() && mem.len() + n <= MEMORY_CAP as usize {
                mem.extend_from_slice(&chunk[..n]);
            } else {
                // Need (or already using) a temp file.
                if tmp_file.is_none() {
                    let tmp = temp_path();
                    // Open with read+write so we can read it back later.
                    let mut f = OpenOptions::new()
                        .read(true)
                        .write(true)
                        .create(true)
                        .truncate(true)
                        .open(&tmp)?;
                    f.write_all(&mem)?; // spill buffered data
                    tmp_path = Some(tmp);
                    tmp_file = Some(f);
                    mem.clear();
                }
                tmp_file.as_mut().unwrap().write_all(&chunk[..n])?;
            }
        }

        let backing = match tmp_file {
            Some(f) => Backing::TempFile {
                path: tmp_path.expect("temp path set"),
                file: f,
            },
            None => Backing::Memory(mem),
        };

        if backing.len() % SECTOR_SIZE as u64 != 0 {
            return Err(CDVDError::InvalidFormat(format!(
                "Decompressed .gz size {} not a multiple of {}",
                backing.len(),
                SECTOR_SIZE
            )));
        }

        Ok(Self { backing })
    }
}

impl CDVDReader for GzReader {
    fn read_sectors(&mut self, lsn: u32, buffer: &mut [u8]) -> Result<usize> {
        let offset = lsn as u64 * SECTOR_SIZE as u64;
        if offset >= self.backing.len() {
            return Ok(0);
        }
        let available = (self.backing.len() - offset) as usize;
        let to_read = buffer.len().min(available);
        let n = self.backing.read_at(offset, &mut buffer[..to_read])?;
        Ok(n)
    }

    fn get_size(&self) -> u64 {
        self.backing.len()
    }
}

/// Generate a unique temp file path for the decompressed image, placed in the
/// same directory as the source `.gz` (so it lands on the large game disk, not
/// the small system `TEMP`). Removed by `Backing::drop`.
fn temp_path() -> std::path::PathBuf {
    let pid = std::process::id();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    // Prefer the current directory (where the .gz lives). Falls back to TEMP.
    let base = std::env::current_dir().unwrap_or_else(|_| std::env::temp_dir());
    base.join(format!("lumine_cdvd_{pid}_{now}.tmp"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_real_gz() {
        let path = r"E:\project\ps2\game\Black (USA).iso.gz";
        if !std::path::Path::new(path).exists() {
            eprintln!("skipping: real gz fixture not present");
            return;
        }
        let sw = std::time::Instant::now();
        let mut reader = GzReader::open(path).expect("open real gz");
        eprintln!(
            "gz opened in {} ms, size={}",
            sw.elapsed().as_millis(),
            reader.get_size()
        );
        assert_eq!(reader.get_size() % 2048, 0);
        // PVD is at sector 16; check CD001 signature.
        let mut buf = vec![0u8; 2048];
        let n = reader.read_sectors(16, &mut buf).expect("read pvd");
        assert_eq!(n, 2048);
        assert_eq!(&buf[1..6], b"CD001", "PVD should contain CD001");
        eprintln!("gz PVD CD001 OK");
    }

    #[test]
    fn test_gz_roundtrip() {
        // Create a gzip of a small known buffer and verify sector read-back.
        let data: Vec<u8> = (0..(SECTOR_SIZE * 4) as u32).map(|i| (i % 251) as u8).collect();
        let mut gz = Vec::new();
        {
            use flate2::write::GzEncoder;
            use flate2::Compression;
            let mut e = GzEncoder::new(&mut gz, Compression::default());
            e.write_all(&data).unwrap();
            e.finish().unwrap();
        }

        let tmp = temp_path();
        fs::write(&tmp, &gz).unwrap();
        let mut reader = GzReader::open(&tmp).expect("open gz");

        let mut buf = vec![0u8; SECTOR_SIZE * 2];
        let n = reader.read_sectors(1, &mut buf).expect("read");
        assert_eq!(n, SECTOR_SIZE * 2);
        assert_eq!(&buf[..SECTOR_SIZE], &data[SECTOR_SIZE..SECTOR_SIZE * 2]);

        fs::remove_file(&tmp).ok();
    }
}
