//! Rust 2021 translation of PCSX2's IOP module name catalogue, savestate
//! writer (originally `pcsx2/SaveState.cpp` + `SaveState.h`), and the
//! `StateWrapper` RAII helper (originally `pcsx2/StateWrapper.cpp` +
//! `StateWrapper.h`).
//!
//! The C++ side kept an in-memory `std::vector<u8>` for the savestate
//! buffer and used `Freeze*` template helpers to read/write typed blobs.
//! The Rust port keeps a `Vec<u8>` per [`SaveState`] and exposes
//! section-based read/write helpers on [`SaveState`] plus an RAII
//! [`StateWrapper`] guard returned by [`SaveState::begin_section`].
//! Sections are length-prefixed and self-patching: the placeholder
//! length is written up front and patched when the section closes
//! (either explicitly via [`SaveState::end_section`] /
//! [`StateWrapper::end_section`] or implicitly via [`Drop`]).
//!
//! Only `std` is used; the C++-specific hooks (`Console`, `FreezeData`,
//! `EmuConfig`, libzip, png, etc.) are not used so the module compiles
//! in isolation. The original C++ side also maintained `g_SaveVersion`
//! and a long list of known IOP module names for the debugger/symbol
//! importer; both are preserved here as public constants.

#![allow(dead_code)]

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
//  Constants preserved from the C++ side.
// ---------------------------------------------------------------------------

/// PCSX2 savestate version baked into the file header. The high 16 bits
/// classify the build type; the low 16 bits are the actual format
/// version. Mirrors `g_SaveVersion` from `SaveState.h`.
pub const SAVE_VERSION: u32 = (0x9A59u32 << 16) | 0x0000;

/// Size of the PCSX2 version string embedded in the file header.
pub const STATE_PCSX2_VERSION_SIZE: usize = 32;

/// Names of the well-known IOP modules that PCSX2's symbol importer and
/// debugger can encounter. The list mixes `rom0:` (BIOS-resident),
/// `cdrom0:` (loaded from disc) and `host:` (loaded from the host
/// filesystem) variants; PCSX2's `IopModuleNames` table is the source
/// of truth on the C++ side.
pub const IOP_MODULES: &[&str] = &[
    // -----------------------------------------------------------------
    // rom0: BIOS-resident core modules
    // -----------------------------------------------------------------
    "rom0:LOADCORE",
    "rom0:INTRMAN",
    "rom0:SYSMEM",
    "rom0:THBASE",
    "rom0:THEVENT",
    "rom0:THSEMAP",
    "rom0:THFPL",
    "rom0:THVPL",
    "rom0:THMSGBX",
    "rom0:TIMRMAN",
    "rom0:EXCEPMAN",
    "rom0:IOMAN",
    "rom0:IOMANX",
    "rom0:STIO",
    "rom0:STDIO",
    "rom0:SYSMCLIB",
    "rom0:SYSCLIB",
    "rom0:HEAPLIB",
    "rom0:DECI2MAN",
    "rom0:DECI2API",
    "rom0:SIFMAN",
    "rom0:SIFCMD",
    "rom0:SIFRPC",
    "rom0:VBLANK",
    "rom0:REBOOT",
    "rom0:MODLOAD",
    "rom0:LIBSD",
    "rom0:PADMAN",
    "rom0:MCMAN",
    "rom0:MCSERV",
    "rom0:SIO2MAN",
    "rom0:RTC",
    "rom0:CDVDMAN",
    "rom0:CDVDFSV",
    "rom0:CDVDSTM",
    "rom0:DEV9",
    "rom0:ATAD",
    "rom0:SMSUTILS",
    "rom0:USBD",
    "rom0:USBMLOAD",
    "rom0:USBHDFSD",
    "rom0:USBHDDFSV",
    "rom0:OSDSND",
    "rom0:INET",
    "rom0:INETCTL",
    "rom0:NETCNF",
    "rom0:NETDEV",
    "rom0:EENETCTL",
    "rom0:ENT_DEVM",
    "rom0:MSIFRPC",
    "rom0:DECI2TYP",
    "rom0:MODMIDI",
    "rom0:MODHSYN",
    "rom0:MODSESQ",
    "rom0:MODSSYN",
    "rom0:MODSEIN",
    "rom0:MODMONO",
    "rom0:MODDELAY",
    "rom0:MODMSIN",
    "rom0:MODEM",
    "rom0:SPUCORDEC",
    "rom0:ILINK",
    "rom0:ILSOCKET",
    "rom0:SDHD",
    "rom0:SDRDRV",
    "rom0:SDSQ",
    "rom0:DEVMAN",
    "rom0:SIO",
    "rom0:IOPBOOT",
    "rom0:MAIN",
    "rom0:RTC2",
    "rom0:VRTC",
    "rom0:LCDSRV",
    "rom0:SCREEN",
    "rom0:GSVRFY",
    "rom0:DTLIMG",
    "rom0:DTLTBL",
    "rom0:DBCFONT",
    "rom0:DBGSYSM",
    "rom0:ISJBJS",
    "rom0:ISIOTERM",
    "rom0:ISIOSCH",
    "rom0:ISIOTAB",
    "rom0:ISDUMMY",
    "rom0:RMMAN",
    "rom0:RMDRV",
    "rom0:DMACMAN",
    "rom0:SYSCRTC",
    "rom0:SSBUSC",
    // -----------------------------------------------------------------
    // cdrom0: disc-loaded common IRX modules
    // -----------------------------------------------------------------
    "cdrom0:\\MODULES\\LIBSD.IRX;1",
    "cdrom0:\\MODULES\\MCMAN.IRX;1",
    "cdrom0:\\MODULES\\MCSERV.IRX;1",
    "cdrom0:\\MODULES\\PADMAN.IRX;1",
    "cdrom0:\\MODULES\\SIO2MAN.IRX;1",
    "cdrom0:\\MODULES\\CDVDMAN.IRX;1",
    "cdrom0:\\MODULES\\CDVDFSV.IRX;1",
    "cdrom0:\\MODULES\\CDVDSTM.IRX;1",
    "cdrom0:\\MODULES\\EENETCTL.IRX;1",
    "cdrom0:\\MODULES\\INET.IRX;1",
    "cdrom0:\\MODULES\\INETCTL.IRX;1",
    "cdrom0:\\MODULES\\NETCNF.IRX;1",
    "cdrom0:\\MODULES\\NETDEV.IRX;1",
    "cdrom0:\\MODULES\\USBD.IRX;1",
    "cdrom0:\\MODULES\\USBMLOAD.IRX;1",
    "cdrom0:\\MODULES\\USBHDFSD.IRX;1",
    "cdrom0:\\MODULES\\USBHDDFSV.IRX;1",
    "cdrom0:\\MODULES\\ATAD.IRX;1",
    "cdrom0:\\MODULES\\DEV9.IRX;1",
    "cdrom0:\\MODULES\\SDHD.IRX;1",
    "cdrom0:\\MODULES\\SDRDRV.IRX;1",
    "cdrom0:\\MODULES\\SDSQ.IRX;1",
    "cdrom0:\\MODULES\\ILINK.IRX;1",
    "cdrom0:\\MODULES\\ILSOCKET.IRX;1",
    "cdrom0:\\MODULES\\ENT_DEVM.IRX;1",
    "cdrom0:\\MODULES\\MODMIDI.IRX;1",
    "cdrom0:\\MODULES\\MODHSYN.IRX;1",
    "cdrom0:\\MODULES\\MODSESQ.IRX;1",
    "cdrom0:\\MODULES\\MODSSYN.IRX;1",
    "cdrom0:\\MODULES\\MODSEIN.IRX;1",
    "cdrom0:\\MODULES\\MODMONO.IRX;1",
    "cdrom0:\\MODULES\\MODDELAY.IRX;1",
    "cdrom0:\\MODULES\\MODMSIN.IRX;1",
    "cdrom0:\\MODULES\\SPUCORDEC.IRX;1",
    "cdrom0:\\MODULES\\SCREEN0P.IRX;1",
    "cdrom0:\\MODULES\\SCREEN1P.IRX;1",
    "cdrom0:\\MODULES\\OSDSND.IRX;1",
    "cdrom0:\\MODULES\\RMMAN.IRX;1",
    "cdrom0:\\MODULES\\RMDRV.IRX;1",
    "cdrom0:\\MODULES\\IBEACON.IRX;1",
    "cdrom0:\\MODULES\\DMACMAN.IRX;1",
    "cdrom0:\\MODULES\\SYSCRTC.IRX;1",
    "cdrom0:\\MODULES\\SSBUSC.IRX;1",
    "cdrom0:\\MODULES\\MSIFRPC.IRX;1",
    "cdrom0:\\MODULES\\MODEM.IRX;1",
    "cdrom0:\\MODULES\\DEV9LOG.IRX;1",
    "cdrom0:\\MODULES\\DEV9SMAP.IRX;1",
    "cdrom0:\\MODULES\\SMAP.IRX;1",
    "cdrom0:\\MODULES\\SMAP_DRV.IRX;1",
    "cdrom0:\\MODULES\\SMSUTILS.IRX;1",
    "cdrom0:\\MODULES\\DECI2TYP.IRX;1",
    "cdrom0:\\MODULES\\KBD.IRX;1",
    "cdrom0:\\MODULES\\MOUSE.IRX;1",
    "cdrom0:\\MODULES\\DS2O.IRX;1",
    "cdrom0:\\MODULES\\DS2D.IRX;1",
    "cdrom0:\\MODULES\\MPEG.IRX;1",
    "cdrom0:\\MODULES\\IIC.IRX;1",
    "cdrom0:\\MODULES\\SPIC.IRX;1",
    "cdrom0:\\MODULES\\PS2LINK.IRX;1",
    "cdrom0:\\MODULES\\PS2HDD.IRX;1",
    "cdrom0:\\MODULES\\PS2FS.IRX;1",
    "cdrom0:\\MODULES\\PS2NETFS.IRX;1",
    "cdrom0:\\MODULES\\HDD.IRX;1",
    "cdrom0:\\MODULES\\PFS.IRX;1",
    "cdrom0:\\MODULES\\CDFS.IRX;1",
    "cdrom0:\\MODULES\\CDVD.IRX;1",
    // -----------------------------------------------------------------
    // host: PCSX2 host-filesystem-mounted modules
    // -----------------------------------------------------------------
    "host:IOPFS",
    "host:IOPFS0",
    "host:IOPFS1",
    "host:IOPFS2",
    "host:IOPFS3",
    "host:IOPFS4",
    "host:IOPFS5",
    "host:IOPFS6",
    "host:IOPFS7",
    "host:IOPFS8",
    "host:IOPFS9",
    "host:IOPFSA",
    "host:IOPFSB",
    "host:IOPFSC",
    "host:IOPFSD",
    "host:IOPFSE",
    "host:IOPFSF",
    "host:IOPFSG",
    "host:IOPFSH",
    "host:IOPFSI",
    "host:IOPFSJ",
    "host:IOPFSK",
    "host:IOPFSL",
    "host:IOPFSM",
    "host:IOPFSN",
    "host:IOPFSO",
    "host:IOPFSP",
    "host:IOPFSQ",
    "host:IOPFSR",
    "host:IOPFSS",
    "host:IOPFST",
    "host:IOPFSU",
    "host:IOPFSV",
    "host:IOPFSW",
    "host:IOPFSX",
    "host:IOPFSY",
    "host:IOPFSZ",
    // -----------------------------------------------------------------
    // mc: memory-card-resident modules (used by some games)
    // -----------------------------------------------------------------
    "mc0:\\MODULES\\LIBSD.IRX",
    "mc0:\\MODULES\\MCMAN.IRX",
    "mc0:\\MODULES\\MCSERV.IRX",
    "mc0:\\MODULES\\PADMAN.IRX",
    "mc0:\\MODULES\\SIO2MAN.IRX",
    "mc0:\\MODULES\\CDVDMAN.IRX",
    "mc0:\\MODULES\\CDVDFSV.IRX",
    "mc0:\\MODULES\\EENETCTL.IRX",
    "mc0:\\MODULES\\INET.IRX",
    "mc0:\\MODULES\\INETCTL.IRX",
    "mc0:\\MODULES\\NETCNF.IRX",
    "mc0:\\MODULES\\NETDEV.IRX",
    "mc0:\\MODULES\\USBD.IRX",
    "mc0:\\MODULES\\USBMLOAD.IRX",
    "mc0:\\MODULES\\ATAD.IRX",
    "mc0:\\MODULES\\DEV9.IRX",
    "mc0:\\MODULES\\SDHD.IRX",
    "mc0:\\MODULES\\SDRDRV.IRX",
    "mc0:\\MODULES\\SDSQ.IRX",
    "mc0:\\MODULES\\ILINK.IRX",
    "mc0:\\MODULES\\ILSOCKET.IRX",
    "mc0:\\MODULES\\ENT_DEVM.IRX",
    "mc0:\\MODULES\\MODMIDI.IRX",
    "mc0:\\MODULES\\MODHSYN.IRX",
    "mc0:\\MODULES\\MODSESQ.IRX",
    "mc0:\\MODULES\\MODSSYN.IRX",
    "mc0:\\MODULES\\MODSEIN.IRX",
    "mc0:\\MODULES\\MODMONO.IRX",
    "mc0:\\MODULES\\MODDELAY.IRX",
    "mc0:\\MODULES\\MODMSIN.IRX",
    "mc0:\\MODULES\\SPUCORDEC.IRX",
    "mc0:\\MODULES\\OSDSND.IRX",
    "mc0:\\MODULES\\RMMAN.IRX",
    "mc0:\\MODULES\\RMDRV.IRX",
    "mc0:\\MODULES\\IBEACON.IRX",
    "mc0:\\MODULES\\DMACMAN.IRX",
    "mc0:\\MODULES\\SYSCRTC.IRX",
    "mc0:\\MODULES\\SSBUSC.IRX",
    "mc0:\\MODULES\\MSIFRPC.IRX",
    "mc0:\\MODULES\\MODEM.IRX",
    "mc0:\\MODULES\\SMAP.IRX",
    "mc0:\\MODULES\\SMAP_DRV.IRX",
    "mc0:\\MODULES\\DECI2TYP.IRX",
    "mc0:\\MODULES\\KBD.IRX",
    "mc0:\\MODULES\\MOUSE.IRX",
    "mc0:\\MODULES\\MPEG.IRX",
    "mc0:\\MODULES\\IIC.IRX",
    "mc0:\\MODULES\\SPIC.IRX",
    "mc0:\\MODULES\\PS2LINK.IRX",
    "mc0:\\MODULES\\PS2HDD.IRX",
    "mc0:\\MODULES\\PS2FS.IRX",
    "mc0:\\MODULES\\PS2NETFS.IRX",
    "mc0:\\MODULES\\HDD.IRX",
    "mc0:\\MODULES\\PFS.IRX",
    "mc0:\\MODULES\\CDFS.IRX",
    "mc0:\\MODULES\\CDVD.IRX",
    "mc0:\\MODULES\\SIO2.IRX",
    "mc0:\\MODULES\\RTC.IRX",
    "mc0:\\MODULES\\VBLANK.IRX",
    "mc0:\\MODULES\\STDIO.IRX",
    "mc0:\\MODULES\\IOMAN.IRX",
    "mc0:\\MODULES\\IOMANX.IRX",
    "mc0:\\MODULES\\THBASE.IRX",
    "mc0:\\MODULES\\THEVENT.IRX",
    "mc0:\\MODULES\\THSEMAP.IRX",
    "mc0:\\MODULES\\THFPL.IRX",
    "mc0:\\MODULES\\THVPL.IRX",
    "mc0:\\MODULES\\THMSGBX.IRX",
    "mc0:\\MODULES\\TIMRMAN.IRX",
    "mc0:\\MODULES\\EXCEPMAN.IRX",
    "mc0:\\MODULES\\HEAPLIB.IRX",
    "mc0:\\MODULES\\SYSMEM.IRX",
    "mc0:\\MODULES\\LOADCORE.IRX",
    "mc0:\\MODULES\\INTRMAN.IRX",
    "mc0:\\MODULES\\SIFMAN.IRX",
    "mc0:\\MODULES\\SIFCMD.IRX",
    "mc0:\\MODULES\\SIFRPC.IRX",
    "mc0:\\MODULES\\MODLOAD.IRX",
    "mc0:\\MODULES\\REBOOT.IRX",
    "mc0:\\MODULES\\SYSCLIB.IRX",
    "mc0:\\MODULES\\SYSMCLIB.IRX",
    "mc0:\\MODULES\\HEAPLIB.IRX",
    "mc0:\\MODULES\\DEV9.IRX",
    "mc0:\\MODULES\\ATAD.IRX",
    "mc0:\\MODULES\\SMSUTILS.IRX",
    "mc0:\\MODULES\\OSDSND.IRX",
    "mc0:\\MODULES\\RMMAN.IRX",
    "mc0:\\MODULES\\RMDRV.IRX",
    "mc0:\\MODULES\\IBEACON.IRX",
    "mc0:\\MODULES\\DMACMAN.IRX",
    "mc0:\\MODULES\\SYSCRTC.IRX",
    "mc0:\\MODULES\\SSBUSC.IRX",
    "mc0:\\MODULES\\MSIFRPC.IRX",
];

// ---------------------------------------------------------------------------
//  Section bookkeeping
// ---------------------------------------------------------------------------

/// One currently-open section on the [`SaveState`] write stack.
///
/// `length_pos` points at the little-endian `u32` placeholder that will
/// be patched to the section's content length when the section closes;
/// `start_pos` points at the first byte of actual content (just after
/// the placeholder).
#[derive(Debug, Clone)]
struct SectionInfo {
    length_pos: usize,
    start_pos: usize,
    name: String,
}

// ---------------------------------------------------------------------------
//  SaveState
// ---------------------------------------------------------------------------

/// In-memory savestate writer with self-patching section headers.
///
/// The buffer is a flat `Vec<u8>` that grows as sections and data are
/// written into it; once construction is finished the caller can either
/// call [`SaveState::save`] to flush to the original path or move the
/// [`SaveState::buffer`] somewhere else.
///
/// Sections are written as:
///
/// ```text
/// [u32 length_le][length bytes of content]
/// ```
///
/// where `length` is the number of content bytes (not including the
/// placeholder). The placeholder is patched when the section closes,
/// either explicitly or via `Drop` on the [`StateWrapper`] guard.
#[derive(Debug)]
pub struct SaveState {
    buffer: Vec<u8>,
    path: PathBuf,
    version: u32,
    /// Current read/write position inside `buffer`.
    idx: usize,
    /// Sticky error flag (mirrors `m_error` from `SaveStateBase`).
    error: bool,
    /// Stack of currently-open sections.
    section_stack: Vec<SectionInfo>,
}

impl SaveState {
    // -----------------------------------------------------------------
    //  Construction
    // -----------------------------------------------------------------

    /// Open or create a savestate file at `path`. If the file already
    /// exists its bytes are read into the in-memory buffer (loading
    /// mode); otherwise an empty buffer is created (saving mode).
    pub fn open(path: impl AsRef<Path>, version: u32) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let buffer = if path.exists() {
            fs::read(&path)?
        } else {
            Vec::new()
        };
        Ok(Self {
            buffer,
            path,
            version,
            idx: 0,
            error: false,
            section_stack: Vec::new(),
        })
    }

    /// Open an in-memory savestate with the given version, no on-disk
    /// file backing. Useful for tests and for nested sub-streams.
    pub fn in_memory(version: u32) -> Self {
        Self {
            buffer: Vec::new(),
            path: PathBuf::new(),
            version,
            idx: 0,
            error: false,
            section_stack: Vec::new(),
        }
    }

    // -----------------------------------------------------------------
    //  Section push / pop
    // -----------------------------------------------------------------

    /// Begin a new length-prefixed section. Returns an RAII guard
    /// ([`StateWrapper`]) that closes the section when dropped.
    pub fn begin_section(&mut self, name: &str) -> io::Result<StateWrapper> {
        let length_pos = self.idx;
        // Reserve 4 bytes for the section length placeholder.
        self.write_bytes(&0u32.to_le_bytes())?;
        let start_pos = self.idx;
        self.section_stack.push(SectionInfo {
            length_pos,
            start_pos,
            name: name.to_string(),
        });
        Ok(StateWrapper {
            state: self,
            closed: false,
        })
    }

    /// Close the most recently opened section. Patches the placeholder
    /// length at the section's header to the number of content bytes
    /// actually written.
    pub fn end_section(&mut self) {
        if let Some(info) = self.section_stack.pop() {
            let end = self.idx;
            // Saturate to u32::MAX rather than panic on absurd sizes;
            // a single section bigger than 4 GiB is never going to fit
            // in the in-memory buffer anyway.
            let len = end.saturating_sub(info.start_pos).min(u32::MAX as usize) as u32;
            let bytes = len.to_le_bytes();
            self.buffer[info.length_pos..info.length_pos + 4].copy_from_slice(&bytes);
        }
    }

    // -----------------------------------------------------------------
    //  Primitive writes
    // -----------------------------------------------------------------

    /// Append a single byte to the buffer.
    pub fn write_u8(&mut self, value: u8) -> io::Result<()> {
        if self.error {
            return Err(io::Error::new(io::ErrorKind::Other, "SaveState in error state"));
        }
        self.buffer.push(value);
        self.idx += 1;
        Ok(())
    }

    /// Append a little-endian `u16`.
    pub fn write_u16(&mut self, value: u16) -> io::Result<()> {
        if self.error {
            return Err(io::Error::new(io::ErrorKind::Other, "SaveState in error state"));
        }
        self.buffer.extend_from_slice(&value.to_le_bytes());
        self.idx += 2;
        Ok(())
    }

    /// Append a little-endian `u32`.
    pub fn write_u32(&mut self, value: u32) -> io::Result<()> {
        if self.error {
            return Err(io::Error::new(io::ErrorKind::Other, "SaveState in error state"));
        }
        self.buffer.extend_from_slice(&value.to_le_bytes());
        self.idx += 4;
        Ok(())
    }

    /// Append a little-endian `u64`.
    pub fn write_u64(&mut self, value: u64) -> io::Result<()> {
        if self.error {
            return Err(io::Error::new(io::ErrorKind::Other, "SaveState in error state"));
        }
        self.buffer.extend_from_slice(&value.to_le_bytes());
        self.idx += 8;
        Ok(())
    }

    /// Append raw bytes verbatim.
    pub fn write_bytes(&mut self, data: &[u8]) -> io::Result<()> {
        if self.error {
            return Err(io::Error::new(io::ErrorKind::Other, "SaveState in error state"));
        }
        self.buffer.extend_from_slice(data);
        self.idx += data.len();
        Ok(())
    }

    /// Convenience wrapper mirroring the C++ `Freeze<bool>` /
    /// `Freeze<u32>` / `Freeze<&[u8]>` overloads.
    pub fn write(&mut self, data: &dyn SaveStateWritable) -> io::Result<()> {
        data.write_to(self)
    }

    /// Append a length-prefixed UTF-8 string (a "marker" in the
    /// `StateWrapper` sense). Mirrors `StateWrapper::Do(std::string*)`.
    pub fn write_marker(&mut self, marker: &str) -> io::Result<()> {
        self.write_u32(marker.len() as u32)?;
        self.write_bytes(marker.as_bytes())
    }

    // -----------------------------------------------------------------
    //  Primitive reads
    // -----------------------------------------------------------------

    /// Read a single byte at the current position.
    pub fn read_u8(&mut self) -> io::Result<u8> {
        if self.idx + 1 > self.buffer.len() {
            self.set_error();
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "SaveState: read past end of buffer",
            ));
        }
        let v = self.buffer[self.idx];
        self.idx += 1;
        Ok(v)
    }

    /// Read a little-endian `u16` at the current position.
    pub fn read_u16(&mut self) -> io::Result<u16> {
        if self.idx + 2 > self.buffer.len() {
            self.set_error();
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "SaveState: read past end of buffer",
            ));
        }
        let bytes: [u8; 2] = self.buffer[self.idx..self.idx + 2].try_into().unwrap();
        self.idx += 2;
        Ok(u16::from_le_bytes(bytes))
    }

    /// Read a little-endian `u32` at the current position.
    pub fn read_u32(&mut self) -> io::Result<u32> {
        if self.idx + 4 > self.buffer.len() {
            self.set_error();
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "SaveState: read past end of buffer",
            ));
        }
        let bytes: [u8; 4] = self.buffer[self.idx..self.idx + 4].try_into().unwrap();
        self.idx += 4;
        Ok(u32::from_le_bytes(bytes))
    }

    /// Read a little-endian `u64` at the current position.
    pub fn read_u64(&mut self) -> io::Result<u64> {
        if self.idx + 8 > self.buffer.len() {
            self.set_error();
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "SaveState: read past end of buffer",
            ));
        }
        let bytes: [u8; 8] = self.buffer[self.idx..self.idx + 8].try_into().unwrap();
        self.idx += 8;
        Ok(u64::from_le_bytes(bytes))
    }

    /// Read `len` raw bytes at the current position.
    pub fn read_bytes(&mut self, len: usize) -> io::Result<Vec<u8>> {
        if self.idx + len > self.buffer.len() {
            self.set_error();
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "SaveState: read past end of buffer",
            ));
        }
        let out = self.buffer[self.idx..self.idx + len].to_vec();
        self.idx += len;
        Ok(out)
    }

    /// Read a length-prefixed UTF-8 marker. Errors if the marker in the
    /// buffer doesn't match `expected`, mirroring the C++ `DoMarker`
    /// behaviour.
    pub fn read_marker(&mut self, expected: &str) -> io::Result<()> {
        let len = self.read_u32()? as usize;
        let bytes = self.read_bytes(len)?;
        let s = std::str::from_utf8(&bytes).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("invalid UTF-8 marker: {e}"))
        })?;
        if s != expected {
            self.set_error();
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("marker mismatch: expected {expected:?}, found {s:?}"),
            ));
        }
        Ok(())
    }

    // -----------------------------------------------------------------
    //  Bookkeeping
    // -----------------------------------------------------------------

    fn set_error(&mut self) {
        self.error = true;
    }

    /// Flip the sticky error flag on. Mirrors the C++ side setting
    /// `m_error = true` on any IO failure.
    pub fn mark_error(&mut self) {
        self.set_error();
    }

    /// Returns true if a previous read or write has failed.
    pub fn has_error(&self) -> bool {
        self.error
    }

    /// The opposite of [`Self::has_error`]. Mirrors `IsOkay()`.
    pub fn is_okay(&self) -> bool {
        !self.error
    }

    /// The savestate version the file was opened with.
    pub fn version(&self) -> u32 {
        self.version
    }

    /// The current read/write position. Mirrors `GetCurrentPos()`.
    pub fn position(&self) -> usize {
        self.idx
    }

    /// The path the savestate was opened with, if any.
    pub fn path(&self) -> Option<&Path> {
        if self.path.as_os_str().is_empty() {
            None
        } else {
            Some(&self.path)
        }
    }

    /// Borrow the underlying buffer. Mirrors `GetBuffer()`.
    pub fn buffer(&self) -> &[u8] {
        &self.buffer
    }

    /// Take the buffer out of the savestate, leaving an empty one
    /// behind.
    pub fn into_buffer(self) -> Vec<u8> {
        self.buffer
    }

    /// Open a [`StateWrapper`] (the RAII section guard) without
    /// pushing a new section. Useful for wrapping a sub-stream.
    pub fn wrap(&mut self) -> StateWrapper {
        StateWrapper {
            state: self,
            closed: true, // never auto-closes anything
        }
    }

    /// Flush the current buffer to the file the savestate was opened
    /// with. Returns an error if no path was associated.
    pub fn save(&self) -> io::Result<()> {
        match self.path() {
            Some(p) => fs::write(p, &self.buffer),
            None => Err(io::Error::new(
                io::ErrorKind::Other,
                "SaveState has no associated path; use into_buffer or save_to",
            )),
        }
    }

    /// Flush the current buffer to an arbitrary path.
    pub fn save_to(&self, path: impl AsRef<Path>) -> io::Result<()> {
        fs::write(path, &self.buffer)
    }

    /// Number of currently-open sections.
    pub fn open_section_count(&self) -> usize {
        self.section_stack.len()
    }

    /// Names of the currently-open sections, top of stack first.
    pub fn open_section_names(&self) -> Vec<&str> {
        self.section_stack
            .iter()
            .rev()
            .map(|s| s.name.as_str())
            .collect()
    }
}

impl Read for SaveState {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.idx >= self.buffer.len() {
            return Ok(0);
        }
        let n = (self.buffer.len() - self.idx).min(buf.len());
        buf[..n].copy_from_slice(&self.buffer[self.idx..self.idx + n]);
        self.idx += n;
        Ok(n)
    }
}

impl Write for SaveState {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.error {
            return Err(io::Error::new(io::ErrorKind::Other, "SaveState in error state"));
        }
        self.buffer.extend_from_slice(buf);
        self.idx += buf.len();
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
//  Trait bridging typed values to the section's write helpers
// ---------------------------------------------------------------------------

/// Types that can be `Freeze`d into a [`SaveState`].
///
/// Mirrors the C++ `Freeze<T>` overload set. The C++ side overrode
/// `Freeze` for atomic types, `std::deque`, `std::string`, etc.; the
/// Rust port keeps the surface minimal: implementors just write their
/// bytes.
pub trait SaveStateWritable {
    /// Append this value's bytes to `state` and advance its cursor.
    fn write_to(&self, state: &mut SaveState) -> io::Result<()>;
}

impl SaveStateWritable for u8 {
    fn write_to(&self, state: &mut SaveState) -> io::Result<()> {
        state.write_u8(*self)
    }
}

impl SaveStateWritable for u16 {
    fn write_to(&self, state: &mut SaveState) -> io::Result<()> {
        state.write_u16(*self)
    }
}

impl SaveStateWritable for u32 {
    fn write_to(&self, state: &mut SaveState) -> io::Result<()> {
        state.write_u32(*self)
    }
}

impl SaveStateWritable for u64 {
    fn write_to(&self, state: &mut SaveState) -> io::Result<()> {
        state.write_u64(*self)
    }
}

impl SaveStateWritable for [u8] {
    fn write_to(&self, state: &mut SaveState) -> io::Result<()> {
        state.write_bytes(self)
    }
}

impl SaveStateWritable for Vec<u8> {
    fn write_to(&self, state: &mut SaveState) -> io::Result<()> {
        state.write_bytes(self)
    }
}

impl SaveStateWritable for str {
    fn write_to(&self, state: &mut SaveState) -> io::Result<()> {
        state.write_marker(self)
    }
}

impl SaveStateWritable for String {
    fn write_to(&self, state: &mut SaveState) -> io::Result<()> {
        state.write_marker(self)
    }
}

// ---------------------------------------------------------------------------
//  StateWrapper (RAII section guard)
// ---------------------------------------------------------------------------

/// RAII guard returned by [`SaveState::begin_section`].
///
/// Owns a mutable borrow of the parent [`SaveState`]. While the guard
/// is alive the section is open and additional data can be written
/// through it; on drop the section is closed automatically unless
/// [`StateWrapper::end_section`] was called first.
///
/// Mirrors the C++ `StateWrapper` read/write helper class but
/// specialises it as a section guard so that section lengths are
/// patched on time, no matter how the user leaves the scope.
pub struct StateWrapper<'a> {
    state: &'a mut SaveState,
    /// Set to `true` when `end_section` is called explicitly, so the
    /// `Drop` impl doesn't double-close.
    closed: bool,
}

impl<'a> StateWrapper<'a> {
    /// Borrow the underlying [`SaveState`]. The borrow is shared so
    /// the section remains open.
    pub fn state(&self) -> &SaveState {
        self.state
    }

    /// Borrow the underlying [`SaveState`] mutably so the caller can
    /// push a nested section or otherwise drive the state machine.
    /// The outer section is *not* closed by this call.
    pub fn state_mut(&mut self) -> &mut SaveState {
        self.state
    }

    // ---------------------------------------------------------------
    //  Writes (forwarded to the underlying SaveState)
    // ---------------------------------------------------------------

    pub fn write_u8(&mut self, v: u8) -> io::Result<()> {
        self.state.write_u8(v)
    }

    pub fn write_u16(&mut self, v: u16) -> io::Result<()> {
        self.state.write_u16(v)
    }

    pub fn write_u32(&mut self, v: u32) -> io::Result<()> {
        self.state.write_u32(v)
    }

    pub fn write_u64(&mut self, v: u64) -> io::Result<()> {
        self.state.write_u64(v)
    }

    pub fn write_bytes(&mut self, data: &[u8]) -> io::Result<()> {
        self.state.write_bytes(data)
    }

    pub fn write(&mut self, data: &dyn SaveStateWritable) -> io::Result<()> {
        self.state.write(data)
    }

    /// Length-prefixed string write. Mirrors `StateWrapper::Do(std::string*)`.
    pub fn write_marker(&mut self, marker: &str) -> io::Result<()> {
        self.state.write_marker(marker)
    }

    /// Begin a nested section inside this section. The returned
    /// [`StateWrapper`]'s `Drop` will close the inner section; the
    /// outer section stays open.
    pub fn begin_section(&mut self, name: &str) -> io::Result<StateWrapper> {
        self.state.begin_section(name)
    }

    /// Close the section explicitly. Consumes the guard so the `Drop`
    /// impl doesn't run afterwards. Mirrors the C++ pattern of
    /// always pairing a section push with a section pop.
    pub fn end_section(mut self) {
        self.state.end_section();
        self.closed = true;
    }

    // ---------------------------------------------------------------
    //  Bookkeeping forwarded to the underlying SaveState
    // ---------------------------------------------------------------

    pub fn has_error(&self) -> bool {
        self.state.has_error()
    }

    pub fn is_okay(&self) -> bool {
        self.state.is_okay()
    }

    pub fn position(&self) -> usize {
        self.state.position()
    }

    pub fn version(&self) -> u32 {
        self.state.version()
    }

    /// Borrow the buffer of the underlying [`SaveState`].
    pub fn buffer(&self) -> &[u8] {
        self.state.buffer()
    }

    /// Convenience: write the current savestate buffer to a [`Write`]
    /// (e.g. `&mut File`).
    pub fn write_to_file(&self, mut w: impl Write) -> io::Result<()> {
        w.write_all(self.state.buffer())
    }

    /// Convenience: load bytes from a [`Read`] into a fresh
    /// in-memory [`SaveState`]. Mostly useful in tests. Call
    /// [`SaveState::wrap`] on the returned state if you need a
    /// [`StateWrapper`].
    pub fn from_reader(mut r: impl Read, version: u32) -> io::Result<SaveState> {
        let mut buf = Vec::new();
        r.read_to_end(&mut buf)?;
        let state = SaveState {
            buffer: buf,
            path: PathBuf::new(),
            version,
            idx: 0,
            error: false,
            section_stack: Vec::new(),
        };
        Ok(state)
    }
}

impl<'a> Drop for StateWrapper<'a> {
    fn drop(&mut self) {
        if !self.closed {
            // The user forgot to call `end_section`; do it for them
            // so the section length is patched.
            self.state.end_section();
        }
    }
}

// ---------------------------------------------------------------------------
//  Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn iop_modules_list_is_long() {
        assert!(
            IOP_MODULES.len() >= 200,
            "expected >= 200 IOP module entries, got {}",
            IOP_MODULES.len()
        );
    }

    #[test]
    fn section_round_trip() {
        let mut s = SaveState::in_memory(SAVE_VERSION);
        {
            let mut sec = s.begin_section("hello").unwrap();
            sec.write_u32(0xDEAD_BEEF).unwrap();
            sec.write_marker("inner").unwrap();
        }
        // The section placeholder should now hold the actual length.
        let len = u32::from_le_bytes(s.buffer()[0..4].try_into().unwrap());
        assert_eq!(len, 4 /*u32*/ + 4 /*length prefix*/ + 5 /*"inner"*/);
    }

    #[test]
    fn section_auto_closes_on_drop() {
        let mut s = SaveState::in_memory(SAVE_VERSION);
        {
            let mut sec = s.begin_section("auto").unwrap();
            sec.write_u8(0x42).unwrap();
            // intentionally don't call end_section
        }
        assert_eq!(s.open_section_count(), 0);
        // The placeholder is now patched to 1 (the one byte we wrote).
        let len = u32::from_le_bytes(s.buffer()[0..4].try_into().unwrap());
        assert_eq!(len, 1);
    }

    #[test]
    fn nested_sections() {
        let mut s = SaveState::in_memory(SAVE_VERSION);
        {
            let mut outer = s.begin_section("outer").unwrap();
            outer.write_u8(1).unwrap();
            {
                let mut inner = outer.begin_section("inner").unwrap();
                inner.write_u8(2).unwrap();
                inner.write_u8(3).unwrap();
            }
            outer.write_u8(4).unwrap();
        }
        assert_eq!(s.open_section_count(), 0);
        assert!(s.is_okay());
    }

    #[test]
    fn save_and_reload() {
        let mut s = SaveState::in_memory(SAVE_VERSION);
        {
            let mut sec = s.begin_section("data").unwrap();
            sec.write_u32(42).unwrap();
        }
        let bytes = s.into_buffer();
        let mut cursor = Cursor::new(bytes);
        let mut loaded = SaveState::open("/dev/null", SAVE_VERSION).unwrap_or_else(|_| {
            // In-memory fallback if the filesystem refused /dev/null
            SaveState::in_memory(SAVE_VERSION)
        });
        loaded.buffer = cursor.get_ref().clone();
        loaded.idx = 0;
        loaded.read_u32().unwrap(); // section length
        loaded.read_marker("data").unwrap();
        assert_eq!(loaded.read_u32().unwrap(), 42);
    }
}
