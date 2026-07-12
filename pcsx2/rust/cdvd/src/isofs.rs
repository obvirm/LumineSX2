//! ISO9660 filesystem parser (pure Rust port of `pcsx2/CDVD/IsoReader.cpp`)
//!
//! Reads the Primary Volume Descriptor and walks directory entries to locate
//! files by path. Sector reads are delegated to a [`crate::reader::CDVDReader`]
//! (which may be the raw ISO reader, CHD, CSO, blockdump, or gzip reader).
//!
//! This is used by `cdvdGetDiscInfo` / `cdvdLoadElf` in `CDVD.cpp` for disc
//! detection (SYSTEM.CNF parse) and ELF loading.

use crate::reader::{CDVDReader, CDVDError, Result};

/// ISO9660 sector size (Mode 1 data)
pub const ISO_SECTOR_SIZE: usize = 2048;

/// A directory entry as stored on the ISO9660 disc.
#[derive(Clone, Copy, Debug)]
pub struct IsoDirEntry {
    /// LBA of the file/directory extent (sector number)
    pub location: u32,
    /// Data length in bytes
    pub length: u32,
    /// True if this entry is a directory
    pub is_directory: bool,
}

/// ISO9660 filesystem accessor.
pub struct IsoFS {
    reader: Box<dyn CDVDReader>,
    /// Root directory entry (from the PVD)
    root: IsoDirEntry,
}

impl IsoFS {
    /// Open an ISO9660 filesystem on top of the given disc reader.
    /// Reads and validates the Primary Volume Descriptor.
    pub fn open(mut reader: Box<dyn CDVDReader>) -> Result<Self> {
        let mut pvd = vec![0u8; ISO_SECTOR_SIZE];
        read_sector(&mut *reader, 16, &mut pvd)?;

        // Volume descriptor header is at offset 0: type (1 byte) + "CD001" (5 bytes) + version.
        if &pvd[1..6] != b"CD001" {
            return Err(CDVDError::InvalidFormat(
                "ISO9660 Primary Volume Descriptor signature 'CD001' not found".to_string(),
            ));
        }
        if pvd[0] != 1 {
            return Err(CDVDError::InvalidFormat(format!(
                "Expected primary volume descriptor (type 1), got type {}",
                pvd[0]
            )));
        }

        // Root directory entry record starts at offset 156 (within the 2048-byte PVD).
        // Record layout:
        //   offset 0: u8  entry_length
        //   offset 1: u8  extended_attr_length
        //   offset 2: u32 location_le
        //   offset 6: u32 location_be
        //   offset 10: u32 length_le
        //   offset 14: u32 length_be
        //   offset 18..25: recording datetime (7 bytes)
        //   offset 25: u8  flags
        //   ...
        //   offset 32: u8  filename_length
        const ROOT_OFF: usize = 156;
        let entry_length = pvd[ROOT_OFF] as usize;
        if entry_length < 34 {
            return Err(CDVDError::InvalidFormat(
                "ISO9660 root directory entry too small".to_string(),
            ));
        }
        let location = u32::from_le_bytes([
            pvd[ROOT_OFF + 2],
            pvd[ROOT_OFF + 3],
            pvd[ROOT_OFF + 4],
            pvd[ROOT_OFF + 5],
        ]);
        let length = u32::from_le_bytes([
            pvd[ROOT_OFF + 10],
            pvd[ROOT_OFF + 11],
            pvd[ROOT_OFF + 12],
            pvd[ROOT_OFF + 13],
        ]);
        let flags = pvd[ROOT_OFF + 25];
        let is_directory = (flags & 0x02) != 0;

        Ok(Self {
            reader,
            root: IsoDirEntry {
                location,
                length,
                is_directory,
            },
        })
    }

    /// Locate a file or directory by path (e.g. "SYSTEM.CNF" or "cdrom:\\SCES_123.45;1").
    /// Strips the `cdrom:` / `cdrom0:` prefix and version (`;1`) automatically.
    pub fn locate(&mut self, path: &str) -> Result<Option<IsoDirEntry>> {
        // Strip device prefix.
        let mut p = path;
        if let Some(rest) = p.strip_prefix("cdrom0:") {
            p = rest;
        } else if let Some(rest) = p.strip_prefix("cdrom:") {
            p = rest;
        }

        // C++ cdvdUncheckedLoadDiscElf: start_pos = (elfpath[5]=='0') ? 7 : 6, then drop leading slashes.
        // Our prefix handling above covers it; now drop leading slashes.
        p = p.trim_start_matches(['\\', '/']);

        // Split into components.
        let components: Vec<&str> = p.split(['\\', '/']).filter(|c| !c.is_empty()).collect();
        if components.is_empty() {
            return Ok(Some(self.root));
        }

        let mut current = self.root;
        for (i, comp) in components.iter().enumerate() {
            // Strip ISO9660 version suffix (`;1`) like the PS2 BIOS does.
            let comp = comp.split(';').next().unwrap_or(comp);
            let de = self.locate_in_dir(current, comp)?;
            match de {
                Some(de) if i == components.len() - 1 => return Ok(Some(de)),
                Some(de) if de.is_directory => current = de,
                Some(_) => return Ok(None), // path component is a file but we expected a dir
                None => return Ok(None),
            }
        }
        Ok(Some(current))
    }

    fn locate_in_dir(&mut self, dir: IsoDirEntry, name: &str) -> Result<Option<IsoDirEntry>> {
        let num_sectors = (dir.length as usize + ISO_SECTOR_SIZE - 1) / ISO_SECTOR_SIZE;
        let mut sector_buf = vec![0u8; ISO_SECTOR_SIZE];

        for i in 0..num_sectors as u32 {
            read_sector(&mut *self.reader, dir.location + i, &mut sector_buf)?;
            let mut offset = 0usize;
            while offset + 33 < ISO_SECTOR_SIZE {
                let entry_length = sector_buf[offset] as usize;
                if entry_length < 33 {
                    break; // end of directory records
                }
                let de_name = dir_entry_name(&sector_buf, offset);
                if !de_name.is_empty() && de_name != "." && de_name != ".." {
                    if de_name.eq_ignore_ascii_case(name) {
                        return Ok(Some(parse_dir_entry(&sector_buf, offset)));
                    }
                }
                offset += entry_length;
            }
        }
        Ok(None)
    }

    /// Read the full contents of a file at `path`.
    pub fn read_file(&mut self, path: &str) -> Result<Vec<u8>> {
        let de = match self.locate(path)? {
            Some(de) => de,
            None => return Err(CDVDError::InvalidFormat(format!("File not found: {path}"))),
        };
        if de.is_directory {
            return Err(CDVDError::InvalidFormat(format!("Path is a directory: {path}")));
        }
        if de.length == 0 {
            return Ok(Vec::new());
        }

        let num_sectors = (de.length as usize + ISO_SECTOR_SIZE - 1) / ISO_SECTOR_SIZE;
        let mut data = vec![0u8; num_sectors * ISO_SECTOR_SIZE];
        for i in 0..num_sectors as u32 {
            read_sector(&mut *self.reader, de.location + i, &mut data[i as usize * ISO_SECTOR_SIZE..])?;
        }
        data.truncate(de.length as usize);
        Ok(data)
    }

    /// List file paths in a directory (relative, with leading `/`).
    pub fn list_directory(&mut self, path: &str) -> Result<Vec<String>> {
        let dir = match self.locate(path)? {
            Some(de) => de,
            None => return Err(CDVDError::InvalidFormat(format!("Directory not found: {path}"))),
        };
        if !dir.is_directory {
            return Err(CDVDError::InvalidFormat(format!("Not a directory: {path}")));
        }

        let mut base = path.to_string();
        if !base.ends_with('/') {
            base.push('/');
        }

        let num_sectors = (dir.length as usize + ISO_SECTOR_SIZE - 1) / ISO_SECTOR_SIZE;
        let mut sector_buf = vec![0u8; ISO_SECTOR_SIZE];
        let mut out = Vec::new();
        for i in 0..num_sectors as u32 {
            read_sector(&mut *self.reader, dir.location + i, &mut sector_buf)?;
            let mut offset = 0usize;
            while offset + 33 < ISO_SECTOR_SIZE {
                let entry_length = sector_buf[offset] as usize;
                if entry_length < 33 {
                    break;
                }
                let de_name = dir_entry_name(&sector_buf, offset);
                if !de_name.is_empty() && de_name != "." && de_name != ".." {
                    out.push(format!("{base}{de_name}"));
                }
                offset += entry_length;
            }
        }
        Ok(out)
    }

    /// Get the root directory entry.
    pub fn root_entry(&self) -> IsoDirEntry {
        self.root
    }
}

/// Read one 2048-byte sector into `dst` (which must be at least ISO_SECTOR_SIZE).
fn read_sector(reader: &mut dyn CDVDReader, lsn: u32, dst: &mut [u8]) -> Result<()> {
    // Some directory entries reference an LBA past the end of the image
    // (seen on real discs, e.g. SYSTEM.CNF at LBA 1898537 of a 1581056-sector
    // image). The C++ IsoReader logs a warning and the game still boots; we
    // mirror that by zero-filling a short read instead of erroring out.
    let n = reader.read_sectors(lsn, &mut dst[..ISO_SECTOR_SIZE])?;
    if n != ISO_SECTOR_SIZE {
        dst.iter_mut().take(ISO_SECTOR_SIZE).for_each(|b| *b = 0);
    }
    Ok(())
}

/// Parse a directory entry record starting at `offset` in `sector`.
fn parse_dir_entry(sector: &[u8], offset: usize) -> IsoDirEntry {
    let location = u32::from_le_bytes([
        sector[offset + 2],
        sector[offset + 3],
        sector[offset + 4],
        sector[offset + 5],
    ]);
    let length = u32::from_le_bytes([
        sector[offset + 10],
        sector[offset + 11],
        sector[offset + 12],
        sector[offset + 13],
    ]);
    let flags = sector[offset + 25];
    IsoDirEntry {
        location,
        length,
        is_directory: (flags & 0x02) != 0,
    }
}

/// Extract the filename from a directory entry record at `offset`.
/// ISO9660 names are stored as: [len byte at offset+32][len bytes of name at offset+33].
/// We trim the version suffix (`;1`) like the C++ `GetDirectoryEntryFileName`.
fn dir_entry_name(sector: &[u8], offset: usize) -> &str {
    let name_len = sector[offset + 32] as usize;
    if name_len == 0 {
        return "";
    }
    let name_bytes = &sector[offset + 33..offset + 33 + name_len];
    // Trim trailing `;version`
    let mut end = name_bytes.len();
    for (i, &b) in name_bytes.iter().enumerate() {
        if b == b';' {
            end = i;
            break;
        }
    }
    std::str::from_utf8(&name_bytes[..end])
        .unwrap_or("")
        .trim_end()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_black() -> IsoFS {
        // Use the real ISO for integration testing.
        let path = r"E:\project\ps2\game\Black (USA).iso";
        if !std::path::Path::new(path).exists() {
            panic!("test fixture missing: {path}");
        }
        let reader: Box<dyn CDVDReader> = Box::new(
            crate::iso_reader::IsoReader::open(path).expect("open iso"),
        );
        IsoFS::open(reader).expect("open isofs")
    }

    #[test]
    fn test_locate_system_cnf() {
        let mut fs = open_black();
        // SYSTEM.CNF is listed in the root directory; the parser must locate it
        // even though on this particular disc its extent LBA (1898537) is past
        // the end of the file (a quirk also present in C++ PCSX2, which logs
        // "Failed to read sector LSN #1898537" and still boots the game).
        let de = fs.locate("SYSTEM.CNF").expect("locate");
        assert!(de.is_some(), "SYSTEM.CNF should be found in root dir");
        let de = de.unwrap();
        assert!(!de.is_directory, "SYSTEM.CNF is a file");
        assert_eq!(de.location, 1898537, "SYSTEM.CNF LBA from directory entry");
        eprintln!("SYSTEM.CNF located at LBA {}", de.location);
    }

    #[test]
    fn test_list_root() {
        let mut fs = open_black();
        let files = fs.list_directory("/").expect("list root");
        assert!(files.iter().any(|f| f.eq_ignore_ascii_case("/SYSTEM.CNF")), "root should contain SYSTEM.CNF");
        assert!(files.iter().any(|f| f.eq_ignore_ascii_case("/IOP")), "root should contain IOP dir");
        eprintln!("root entries: {}", files.len());
    }
}
