// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! ELF header parser for PS2 ELF binaries.
//!
//! Idiomatic Rust translation of the C++ `Elfheader` module. Provides the
//! `Elf32_Ehdr` and `Elf32_Phdr` on-disk structures along with a small helper
//! for initialising the parser, identifying ELF files by their magic bytes,
//! and reading their raw contents from disk.

use std::fs;
use std::io;
use std::io::Read;
use std::path::Path;

/// On-disk layout of an ELF32 file header.
///
/// Mirrors the C `ELF_HEADER` / `Elf32_Ehdr` struct. `#[repr(C)]` guarantees
/// a stable layout compatible with the binary format used by the original
/// PS2 toolchain.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Elf32_Ehdr {
    /// ELF file identifier: `0x7F`, `'E'`, `'L'`, `'F'`, ...
    pub e_ident: [u8; 16],
    /// ELF type: 0=NONE, 1=REL, 2=EXEC, 3=SHARED, 4=CORE.
    pub e_type: u16,
    /// Target processor: 8=MIPS R3000 for PS2.
    pub e_machine: u16,
    /// ELF format version (1 = current).
    pub e_version: u32,
    /// Virtual address of the entry point.
    pub e_entry: u32,
    /// File offset of the program header table.
    pub e_phoff: u32,
    /// File offset of the section header table.
    pub e_shoff: u32,
    /// Processor-specific flags (e.g. 0x20924001 for MIPS).
    pub e_flags: u32,
    /// Size of this header in bytes (52 for ELF32).
    pub e_ehsize: u16,
    /// Size of a single program header entry.
    pub e_phentsize: u16,
    /// Number of program header entries.
    pub e_phnum: u16,
    /// Size of a single section header entry.
    pub e_shentsize: u16,
    /// Number of section header entries.
    pub e_shnum: u16,
    /// Index of the section name string table in the section header table.
    pub e_shstrndx: u16,
}

/// On-disk layout of an ELF32 program header.
///
/// Mirrors the C `ELF_PHR` / `Elf32_Phdr` struct. `p_type` values:
/// 0=Inactive, 1=Load, 2=Dynamic, 3=Interpreter, 4=Note, 6=PHDR.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Elf32_Phdr {
    /// Segment type (see module-level notes).
    pub p_type: u32,
    /// Offset of the segment within the file.
    pub p_offset: u32,
    /// Virtual address of the segment in memory.
    pub p_vaddr: u32,
    /// Physical address of the segment (unused on PS2).
    pub p_paddr: u32,
    /// Number of bytes the segment occupies in the file image.
    pub p_filesz: u32,
    /// Number of bytes the segment occupies in the memory image.
    pub p_memsz: u32,
    /// Segment permission flags.
    pub p_flags: u32,
    /// Required alignment; 0 or 1 means no alignment.
    pub p_align: u32,
}

const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];
const HEADER_SIZE: usize = std::mem::size_of::<Elf32_Ehdr>();

/// Initialise the ELF parser state.
///
/// This is a placeholder for parity with the C++ `ElfObject` constructor.
/// In the Rust port the parser is stateless, so the call is a no-op kept
/// for source-compatibility with translated callers.
pub fn ps2ElfInit() {}

/// Returns `true` if `path` exists, is a regular file, and begins with the
/// ELF magic bytes (`0x7F` followed by `"ELF"`).
///
/// Any I/O error encountered while reading the first four bytes of the file
/// causes the function to return `false` rather than propagating the error,
/// matching the C++ `ElfObject::OpenFile` semantics where a malformed file
/// is reported as "not an ELF" rather than an error.
pub fn ps2ElfIsElf(path: &Path) -> bool {
    let mut file = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };

    let mut magic = [0u8; 4];
    if let Err(err) = file.read_exact(&mut magic) {
        if err.kind() == io::ErrorKind::UnexpectedEof {
            return false;
        }
        return false;
    }

    magic == ELF_MAGIC
}

/// Read the entire contents of `path` into memory.
///
/// Returns the raw bytes of the file, mirroring `ElfObject::OpenFile`'s
/// behaviour of loading the whole image. The function rejects paths that
/// cannot be opened or whose contents cannot be read in a single pass.
pub fn ps2ElfReadFile(path: &Path) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|err| format!("Failed to read ELF from '{}': {}", path.display(), err))
}

// =========================================================================
// ELF section / symbol / relocation headers
// =========================================================================
//
// Mirrors `ELF_SHR`, `Elf32_Sym` and `Elf32_Rel` declared in
// `pcsx2/Elfheader.h`. The PS2/EE toolchain emits these alongside the main
// ELF header so the loader can resolve sections, symbols and relocations
// during program loading.

/// On-disk layout of an ELF32 section header.
///
/// Mirrors the C `ELF_SHR` / `Elf32_Shdr` struct. `sh_type` values:
/// 0=Inactive, 1=PROGBITS, 2=SYMTAB, 3=STRTAB, 4=RELA, 8=NOBITS, 9=REL.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Elf32_Shdr {
    /// Index into the section name string table.
    pub sh_name: u32,
    /// Section type (see module-level notes).
    pub sh_type: u32,
    /// Section flags (1=write, 2=alloc, 4=exec).
    pub sh_flags: u32,
    /// Section start address.
    pub sh_addr: u32,
    /// Offset from start of file to the section.
    pub sh_offset: u32,
    /// Section size in bytes.
    pub sh_size: u32,
    /// Section header table index link.
    pub sh_link: u32,
    /// Extra info (depends on section type).
    pub sh_info: u32,
    /// Required alignment; 0 or 1 means no alignment.
    pub sh_addralign: u32,
    /// Fixed size entries, where applicable.
    pub sh_entsize: u32,
}

/// ELF32 symbol table entry.
///
/// Mirrors the C `Elf32_Sym` struct. `st_info` packs the symbol binding
/// into the upper nibble and the symbol type into the lower nibble.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Elf32_Sym {
    /// Index into the symbol string table.
    pub st_name: u32,
    /// Symbol value (address or absolute value).
    pub st_value: u32,
    /// Symbol size in bytes.
    pub st_size: u32,
    /// Symbol binding/type byte (binding << 4 | type).
    pub st_info: u8,
    /// Visibility (0=DEFAULT, 1=INTERNAL, 2=HIDDEN, 3=PROTECTED).
    pub st_other: u8,
    /// Section index the symbol is defined in.
    pub st_shndx: u16,
}

/// ELF32 relocation entry (without addend).
///
/// Mirrors the C `Elf32_Rel` struct. `r_info` packs the symbol table
/// index into the upper 24 bits and the relocation type into the low 8.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Elf32_Rel {
    /// Address at which the relocation should be applied.
    pub r_offset: u32,
    /// Symbol index + relocation type packed into a single word.
    pub r_info: u32,
}

/// Extracts the symbol-table index half of an `r_info` value.
///
/// Mirrors the C `ELF32_R_SYM(i)` macro (`(i) >> 8`).
#[inline]
pub fn elf32_r_sym(r_info: u32) -> u32 {
    r_info >> 8
}

/// Extracts the relocation type half of an `r_info` value.
///
/// Mirrors the C `ELF32_R_TYPE(i)` macro (`(i) & 0xff`).
#[inline]
pub fn elf32_r_type(r_info: u32) -> u32 {
    r_info & 0xff
}

/// Extracts the symbol type from an `st_info` byte.
///
/// Mirrors the C `ELF32_ST_TYPE(i)` macro (`(i) & 0xf`).
#[inline]
pub fn elf32_st_type(st_info: u8) -> u8 {
    st_info & 0xf
}

// =========================================================================
// PS-EXE header (private to the C++ file, used by HasValidPSXHeader)
// =========================================================================

/// PSX "PS-X EXE" header (always 0x800 bytes total).
///
/// Mirrors the C `PSXEXEHeader` struct declared inside `Elfheader.cpp`.
/// Loaded PS1 executables begin with this fixed-size prefix; only the
/// `id`, `initial_pc`, `initial_gp`, `load_address`, `file_size` and the
/// stack-pointer fields are read by `ElfObject`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PsxExeHeader {
    /// Magic: must be the ASCII string `"PS-X EXE"`.
    pub id: [u8; 8],
    /// Reserved padding.
    pub pad1: [u8; 8],
    /// Initial program counter.
    pub initial_pc: u32,
    /// Initial global pointer.
    pub initial_gp: u32,
    /// Load address of the program in PSX RAM.
    pub load_address: u32,
    /// File size *excluding* the 0x800-byte header.
    pub file_size: u32,
    /// Unknown field at offset 0x020.
    pub unk0: u32,
    /// Unknown field at offset 0x024.
    pub unk1: u32,
    /// Start address for memory fill.
    pub memfill_start: u32,
    /// Size of memory fill.
    pub memfill_size: u32,
    /// Initial stack pointer base.
    pub initial_sp_base: u32,
    /// Initial stack pointer offset.
    pub initial_sp_offset: u32,
    /// Reserved words at 0x038..0x04B.
    pub reserved: [u32; 5],
    /// Marker bytes from 0x04C to 0x7FF.
    pub marker: [u8; 0x7B4],
}

impl Default for PsxExeHeader {
    fn default() -> Self {
        Self {
            id: [0; 8],
            pad1: [0; 8],
            initial_pc: 0,
            initial_gp: 0,
            load_address: 0,
            file_size: 0,
            unk0: 0,
            unk1: 0,
            memfill_start: 0,
            memfill_size: 0,
            initial_sp_base: 0,
            initial_sp_offset: 0,
            reserved: [0; 5],
            marker: [0; 0x7B4],
        }
    }
}

// =========================================================================
// IsoReader abstraction
// =========================================================================
//
// `ElfObject::OpenIsoFile` only needs two operations from an ISO reader:
// locating a file (so the loader can validate the size up-front) and
// reading its full contents into memory. We define a minimal trait here so
// that `pcsx2::Elfheader` stays self-contained and does not need to know
// about `pcsx2::Cdvd::IsoReader` directly. Concrete `IsoReader`
// implementations from `Cdvd.rs` can be adapted to this trait at the call
// site.

/// Subset of an ISO file descriptor that `ElfObject` cares about.
///
/// Mirrors the `length_le` field of `IsoReader::DirectoryEntry`. In the
/// C++ code it is a `s64` because `-1` is used as a sentinel for
/// "file does not exist".
#[derive(Clone, Copy, Debug, Default)]
pub struct IsoFileDescriptor {
    /// File length in bytes (signed; `-1` means "does not exist").
    pub length_le: i64,
}

/// Minimal interface that `ElfObject::OpenIsoFile` needs from an
/// ISO 9660 reader. Concrete `IsoReader` implementations can be wrapped
/// in a thin adapter at the call site.
pub trait IsoReaderLike {
    /// Look up `srcfile` inside the ISO. On success returns the descriptor
    /// and leaves `error` untouched; on failure populates `error` and
    /// returns `None`.
    fn locate_file(&self, srcfile: &str, error: &mut Option<String>) -> Option<IsoFileDescriptor>;

    /// Read the full contents of the previously-located file into a
    /// `Vec<u8>`. On failure populates `error` and returns `None`.
    fn read_file(&self, descriptor: IsoFileDescriptor, error: &mut Option<String>) -> Option<Vec<u8>>;
}

// =========================================================================
// ElfObject
// =========================================================================
//
// Mirrors the C++ `ElfObject` class from `pcsx2/Elfheader.{h,cpp}`.
//
// The struct owns the raw ELF bytes plus two offsets that point into the
/// buffer: one to the program-header table and one to the section-header
/// table. Storing offsets (rather than raw pointers into a `Vec`) keeps
/// the type safe — accessing a header is just a bounds-checked slice
/// lookup.

/// PS2 ELF / PSX PS-EXE parser. Mirrors `ElfObject` from
/// `pcsx2/Elfheader.h`.
pub struct ElfObject {
    /// Raw file bytes.
    data: Vec<u8>,
    /// Offset of the program header table inside `data`, or `None`.
    proghead_offset: Option<u32>,
    /// Offset of the section header table inside `data`, or `None`.
    secthead_offset: Option<u32>,
    /// Source filename (may be inside an ISO image).
    filename: String,
    /// `true` if this is a PS1 PS-EXE rather than a PS2 ELF.
    is_psx_elf: bool,
}

impl Default for ElfObject {
    fn default() -> Self {
        Self::new()
    }
}

impl ElfObject {
    /// Construct a new, empty `ElfObject`. Mirrors the C++ default
    /// constructor (`ElfObject::ElfObject() = default;`).
    pub const fn new() -> Self {
        Self {
            data: Vec::new(),
            proghead_offset: None,
            secthead_offset: None,
            filename: String::new(),
            is_psx_elf: false,
        }
    }

    // ------------------------------------------------------------------
    // Public accessors
    // ------------------------------------------------------------------

    /// Returns the full ELF file image. Mirrors `GetData()`.
    #[inline]
    pub fn get_data(&self) -> &[u8] {
        &self.data
    }

    /// Move the ELF file image out, leaving `self` in an empty state.
    /// Mirrors `ReleaseData()`.
    pub fn release_data(&mut self) -> Vec<u8> {
        // Reset auxiliary state so `self` is left in a valid empty state.
        self.proghead_offset = None;
        self.secthead_offset = None;
        self.filename.clear();
        self.is_psx_elf = false;
        std::mem::take(&mut self.data)
    }

    /// Borrow the ELF32 file header from the front of the image.
    /// Mirrors `GetHeader()`.
    ///
    /// Panics in debug builds if `data` is smaller than the ELF header.
    #[inline]
    pub fn get_header(&self) -> &Elf32_Ehdr {
        debug_assert!(self.data.len() >= HEADER_SIZE);
        // SAFETY: `HEADER_SIZE` equals `size_of::<Elf32_Ehdr>()` and
        // `data` is at least that long. ELF headers are POD (`#[repr(C)]`
        // with only primitive fields), so the pointer cast is sound.
        unsafe { &*(self.data.as_ptr() as *const Elf32_Ehdr) }
    }

    /// Returns the size of the ELF file image in bytes. Mirrors `GetSize()`.
    #[inline]
    pub fn get_size(&self) -> u32 {
        self.data.len() as u32
    }

    /// Returns the source filename (the path passed to `open_file` /
    /// `open_iso_file`).
    #[inline]
    pub fn filename(&self) -> &str {
        &self.filename
    }

    /// Returns `true` if this object was opened as a PS1 PS-EXE rather
    /// than a PS2 ELF.
    #[inline]
    pub fn is_psx_elf(&self) -> bool {
        self.is_psx_elf
    }

    // ------------------------------------------------------------------
    // Openers
    // ------------------------------------------------------------------

    /// Open an ELF / PS-EXE file from disk. Mirrors `ElfObject::OpenFile`.
    ///
    /// Returns `false` (and populates `error`) if the file cannot be
    /// opened, stat-ed, or read, or if its size is illegal. On success
    /// `self` is populated and the internal pointers are refreshed by
    /// `init_elf_headers`.
    pub fn open_file(
        &mut self,
        srcfile: &str,
        is_psx_elf: bool,
        error: &mut Option<String>,
    ) -> bool {
        let path = Path::new(srcfile);
        let file = match fs::File::open(path) {
            Ok(f) => f,
            Err(e) => {
                *error = Some(format!("Failed to read ELF from '{}': {}", srcfile, e));
                return false;
            }
        };

        let metadata = match file.metadata() {
            Ok(m) => m,
            Err(e) => {
                *error = Some(format!("Failed to read ELF from '{}': {}", srcfile, e));
                return false;
            }
        };

        let size = metadata.len() as i64;
        if !is_psx_elf && !self.check_elf_size(size, error) {
            return false;
        }

        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        if let Err(e) = std::io::Read::read_to_end(&mut std::io::Read::take(
            std::io::BufReader::new(file),
            metadata.len(),
        ), &mut bytes)
        {
            *error = Some(format!("Failed to read ELF from '{}': {}", srcfile, e));
            return false;
        }

        self.data = bytes;
        self.filename = srcfile.to_string();
        self.is_psx_elf = is_psx_elf;
        self.init_elf_headers();
        true
    }

    /// Open an ELF / PS-EXE from an already-mounted ISO image.
    /// Mirrors `ElfObject::OpenIsoFile`.
    pub fn open_iso_file<R: IsoReaderLike + ?Sized>(
        &mut self,
        srcfile: &str,
        isor: &R,
        is_psx_elf: bool,
        error: &mut Option<String>,
    ) -> bool {
        let descriptor = match isor.locate_file(srcfile, error) {
            Some(d) => d,
            None => return false,
        };

        if !self.check_elf_size(descriptor.length_le, error) {
            return false;
        }

        let bytes = match isor.read_file(descriptor, error) {
            Some(b) => b,
            None => return false,
        };

        self.data = bytes;
        self.filename = srcfile.to_string();
        self.is_psx_elf = is_psx_elf;
        self.init_elf_headers();
        true
    }

    // ------------------------------------------------------------------
    // Header inspection
    // ------------------------------------------------------------------

    /// Returns `true` if the buffer starts with a valid PS-EXE header.
    /// Mirrors `HasValidPSXHeader`.
    pub fn has_valid_psx_header(&self) -> bool {
        if self.data.len() < std::mem::size_of::<PsxExeHeader>() {
            return false;
        }

        // SAFETY: `data` is at least `size_of::<PsxExeHeader>()` bytes,
        // and `PsxExeHeader` is `#[repr(C)]` POD.
        let header = unsafe { &*(self.data.as_ptr() as *const PsxExeHeader) };

        static EXPECTED_ID: [u8; 8] = *b"PS-X EXE";
        if header.id != EXPECTED_ID {
            return false;
        }

        // C++: only warns, never fails. We replicate the warning as a
        // log line through `eprintln!` to keep the API self-contained.
        let header_size = std::mem::size_of::<PsxExeHeader>() as u64;
        if u64::from(header.file_size) + header_size > self.data.len() as u64 {
            eprintln!(
                "(ELF) Incorrect file size in PS-EXE header: {} bytes should not be greater than {} bytes",
                header.file_size,
                (self.data.len() as u64) - header_size
            );
        }

        true
    }

    /// Returns the program-header table parsed from the file image.
    /// Mirrors `HasProgramHeaders` (but exposes the actual data instead
    /// of just a non-null check).
    pub fn program_headers(&self) -> &[Elf32_Phdr] {
        let Some(offset) = self.proghead_offset else {
            return &[];
        };
        let count = self.get_header().e_phnum as usize;
        let size = std::mem::size_of::<Elf32_Phdr>();
        let start = offset as usize;
        let end = start + count * size;
        if end > self.data.len() {
            return &[];
        }
        // SAFETY: bounds-checked above, and `Elf32_Phdr` is `#[repr(C)]`
        // POD so a slice cast is sound.
        unsafe {
            std::slice::from_raw_parts(
                self.data.as_ptr().add(start) as *const Elf32_Phdr,
                count,
            )
        }
    }

    /// Returns the section-header table parsed from the file image.
    /// Mirrors `HasSectionHeaders`.
    pub fn section_headers(&self) -> &[Elf32_Shdr] {
        let Some(offset) = self.secthead_offset else {
            return &[];
        };
        let count = self.get_header().e_shnum as usize;
        let size = std::mem::size_of::<Elf32_Shdr>();
        let start = offset as usize;
        let end = start + count * size;
        if end > self.data.len() {
            return &[];
        }
        unsafe {
            std::slice::from_raw_parts(
                self.data.as_ptr().add(start) as *const Elf32_Shdr,
                count,
            )
        }
    }

    /// Returns `true` if both the program- and section-header tables
    /// could be located in the file. Mirrors `HasHeaders`.
    pub fn has_headers(&self) -> bool {
        self.proghead_offset.is_some() && self.secthead_offset.is_some()
    }

    /// Returns the entry point of the ELF / PS-EXE. Mirrors `GetEntryPoint`.
    ///
    /// PS1 PS-EXEs return `0xFFFFFFFF` if their header is invalid.
    pub fn get_entry_point(&self) -> u32 {
        if self.is_psx_elf {
            if self.has_valid_psx_header() {
                // SAFETY: bounds-checked by `has_valid_psx_header`.
                let header = unsafe { &*(self.data.as_ptr() as *const PsxExeHeader) };
                header.initial_pc
            } else {
                0xFFFF_FFFF
            }
        } else {
            self.get_header().e_entry
        }
    }

    /// Returns `(start, size)` of the program segment that contains the
    /// ELF entry point, or `(0, 0)` if none. Mirrors `GetTextRange`.
    pub fn get_text_range(&self) -> (u32, u32) {
        if self.is_psx_elf || self.proghead_offset.is_none() {
            return (0, 0);
        }

        let header = *self.get_header();
        for phdr in self.program_headers() {
            let start = phdr.p_vaddr;
            let end = start.wrapping_add(phdr.p_memsz);
            if start <= header.e_entry && header.e_entry < end {
                return (start, phdr.p_memsz);
            }
        }

        (0, 0)
    }

    /// XOR-folds the ELF image into a single 32-bit checksum.
    /// Mirrors `GetCRC` (the C++ comment notes this is `//getCRC();`-ed
    /// out in `InitElfHeaders`, but the method itself is still public).
    pub fn get_crc(&self) -> u32 {
        let mut crc: u32 = 0;
        let mut chunks = self.data.chunks_exact(4);
        for word in &mut chunks {
            let value = u32::from_le_bytes([word[0], word[1], word[2], word[3]]);
            crc ^= value;
        }
        // Any trailing 1..3 bytes are XOR-ed in as the low bytes of a
        // zero-extended word, matching `reinterpret_cast<const u32*>`.
        let remainder = chunks.remainder();
        if !remainder.is_empty() {
            let mut buf = [0u8; 4];
            buf[..remainder.len()].copy_from_slice(remainder);
            crc ^= u32::from_le_bytes(buf);
        }
        crc
    }

    /// Iterate over program headers, logging each one. Mirrors the
    /// side-effecting body of `LoadProgramHeaders` (the C++ method has
    /// no return value, only log lines via `ELF_LOG`).
    pub fn load_program_headers(&self) {
        let header = self.get_header();
        eprintln!("Elf32 Program Header");
        for (i, phdr) in self.program_headers().iter().enumerate() {
            eprintln!("  [{i}] type:      0x{:x}", phdr.p_type);
            eprintln!("  [{i}] offset:    0x{:08x}", phdr.p_offset);
            eprintln!("  [{i}] vaddr:     0x{:08x}", phdr.p_vaddr);
            eprintln!("  [{i}] paddr:     0x{:08x}", phdr.p_paddr);
            eprintln!("  [{i}] file size: 0x{:08x}", phdr.p_filesz);
            eprintln!("  [{i}] mem size:  0x{:08x}", phdr.p_memsz);
            eprintln!("  [{i}] flags:     0x{:08x}", phdr.p_flags);
            eprintln!("  [{i}] palign:    0x{:08x}", phdr.p_align);
            let _ = header; // silence unused warning if there are 0 headers
        }
    }

    /// Iterate over section headers, logging each one.
    /// Mirrors `LoadSectionHeaders`.
    pub fn load_section_headers(&self) {
        let header = *self.get_header();
        let sectheads = self.section_headers();
        if sectheads.is_empty() || header.e_shoff as usize > self.data.len() {
            return;
        }

        // Pick the section-name string-table index. The C++ code treats
        // `e_shstrndx == 0xffff` as "use section 0" (matches the
        // SHN_XINDEX convention from the ELF spec).
        let strndx = if header.e_shstrndx == 0xffff {
            0
        } else {
            header.e_shstrndx as usize
        };

        let names_offset = sectheads
            .get(strndx)
            .map(|s| s.sh_offset as usize)
            .unwrap_or(0);
        let names_end = self.data.len();
        let names = self
            .data
            .get(names_offset..names_end)
            .unwrap_or(&[]);

        for (i, shdr) in sectheads.iter().enumerate() {
            // Section name is a NUL-terminated C string starting at
            // `names[shdr.sh_name]`.
            let name_start = shdr.sh_name as usize;
            let name_bytes = names.get(name_start..).unwrap_or(&[]);
            let name_len = name_bytes
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(name_bytes.len());
            let name = std::str::from_utf8(&name_bytes[..name_len]).unwrap_or("<bad name>");
            eprintln!("ELF32 Section Header [{i:#x}] {name}");
            eprintln!("  [{i}] type:      0x{:08x}", shdr.sh_type);
            eprintln!("  [{i}] flags:     0x{:08x}", shdr.sh_flags);
            eprintln!("  [{i}] addr:      0x{:08x}", shdr.sh_addr);
            eprintln!("  [{i}] offset:    0x{:08x}", shdr.sh_offset);
            eprintln!("  [{i}] size:      0x{:08x}", shdr.sh_size);
            eprintln!("  [{i}] link:      0x{:08x}", shdr.sh_link);
            eprintln!("  [{i}] info:      0x{:08x}", shdr.sh_info);
            eprintln!("  [{i}] addralign: 0x{:08x}", shdr.sh_addralign);
            eprintln!("  [{i}] entsize:   0x{:08x}", shdr.sh_entsize);
        }
    }

    /// Load + log both header tables. Mirrors `LoadHeaders`.
    pub fn load_headers(&self) {
        if self.is_psx_elf {
            return;
        }
        self.load_program_headers();
        self.load_section_headers();
    }

    // ------------------------------------------------------------------
    // Private helpers
    // ------------------------------------------------------------------

    /// Validate an ELF file size. Mirrors `CheckElfSize`.
    ///
    /// The C++ signature is `bool CheckElfSize(s64 size, Error* error)`:
    /// the function returns `true` for "size is OK" and populates `*error`
    /// on any failure path. We follow the same convention.
    fn check_elf_size(&self, size: i64, error: &mut Option<String>) -> bool {
        if size > 0x0FFF_FFFF {
            *error = Some("Illegal ELF file size over 2GB!".to_string());
            false
        } else if size == -1 {
            *error = Some("ELF file does not exist!".to_string());
            false
        } else if size <= HEADER_SIZE as i64 {
            *error = Some("Unexpected end of ELF file.".to_string());
            false
        } else {
            true
        }
    }

    /// Populate the program/section-header offsets from the ELF file
    /// header. Mirrors `InitElfHeaders`.
    fn init_elf_headers(&mut self) {
        // PSX PS-EXEs have neither program nor section headers.
        self.proghead_offset = None;
        self.secthead_offset = None;

        if self.is_psx_elf {
            return;
        }

        if self.data.len() < HEADER_SIZE {
            eprintln!(
                "(ELF) Initializing Elf: {} bytes is smaller than the ELF header",
                self.data.len()
            );
            return;
        }

        eprintln!("Initializing Elf: {} bytes", self.data.len());

        let header = *self.get_header();

        if header.e_phnum > 0 {
            let phdr_end = (header.e_phoff as usize)
                .saturating_add(std::mem::size_of::<Elf32_Phdr>());
            if phdr_end <= self.data.len() {
                self.proghead_offset = Some(header.e_phoff);
            } else {
                eprintln!(
                    "(ELF) Program header offset {:x} is larger than file size {}",
                    header.e_phoff,
                    self.data.len()
                );
            }
        }

        if header.e_shnum > 0 {
            let shdr_end = (header.e_shoff as usize)
                .saturating_add(std::mem::size_of::<Elf32_Shdr>());
            if shdr_end <= self.data.len() {
                self.secthead_offset = Some(header.e_shoff);
            } else {
                eprintln!(
                    "(ELF) Section header offset {:x} is larger than file size {}",
                    header.e_shoff,
                    self.data.len()
                );
            }
        }

        if header.e_shnum > 0 && header.e_shentsize != std::mem::size_of::<Elf32_Shdr>() as u16 {
            eprintln!("(ELF) Size of section headers is not standard");
        }

        if header.e_phnum > 0 && header.e_phentsize != std::mem::size_of::<Elf32_Phdr>() as u16 {
            eprintln!("(ELF) Size of program headers is not standard");
        }

        //getCRC();
        eprintln!("type:      {:?}", elf_type_name(header.e_type));
        eprintln!("machine:   {:?}", elf_machine_name(header.e_machine));
        eprintln!("version:   {}", header.e_version);
        eprintln!("entry:     {:08x}", header.e_entry);
        eprintln!("flags:     {:08x}", header.e_flags);
        eprintln!("eh size:   {:08x}", header.e_ehsize);
        eprintln!("ph off:    {:08x}", header.e_phoff);
        eprintln!("ph entsiz: {:08x}", header.e_phentsize);
        eprintln!("ph num:    {:08x}", header.e_phnum);
        eprintln!("sh off:    {:08x}", header.e_shoff);
        eprintln!("sh entsiz: {:08x}", header.e_shentsize);
        eprintln!("sh num:    {:08x}", header.e_shnum);
        eprintln!("sh strndx: {:08x}", header.e_shstrndx);
    }
}

// =========================================================================
// Small string helpers (used by InitElfHeaders logging)
// =========================================================================

/// Human-readable name for the `e_type` field of the ELF header, or
/// `None` if the value is not one of the documented PS2/EE variants.
fn elf_type_name(e_type: u16) -> Option<&'static str> {
    match e_type {
        0x0 => Some("no file type"),
        0x1 => Some("relocatable"),
        0x2 => Some("executable"),
        _ => None,
    }
}

/// Human-readable name for the `e_machine` field of the ELF header.
fn elf_machine_name(e_machine: u16) -> Option<&'static str> {
    match e_machine {
        1 => Some("AT&T WE 32100"),
        2 => Some("SPARC"),
        3 => Some("Intel 80386"),
        4 => Some("Motorola 68000"),
        5 => Some("Motorola 88000"),
        7 => Some("Intel 80860"),
        8 => Some("mips_rs3000"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ehdr_size_is_52_bytes() {
        assert_eq!(std::mem::size_of::<Elf32_Ehdr>(), 52);
    }

    #[test]
    fn phdr_size_is_32_bytes() {
        assert_eq!(std::mem::size_of::<Elf32_Phdr>(), 32);
    }

    #[test]
    fn shdr_size_is_40_bytes() {
        assert_eq!(std::mem::size_of::<Elf32_Shdr>(), 40);
    }

    #[test]
    fn sym_size_is_16_bytes() {
        assert_eq!(std::mem::size_of::<Elf32_Sym>(), 16);
    }

    #[test]
    fn rel_size_is_8_bytes() {
        assert_eq!(std::mem::size_of::<Elf32_Rel>(), 8);
    }

    #[test]
    fn psx_exe_header_is_0x800_bytes() {
        assert_eq!(std::mem::size_of::<PsxExeHeader>(), 0x800);
    }

    #[test]
    fn elf32_r_sym_and_type_extract() {
        let info: u32 = (0x12u32 << 8) | 0x34;
        assert_eq!(elf32_r_sym(info), 0x12);
        assert_eq!(elf32_r_type(info), 0x34);
    }

    #[test]
    fn elf32_st_type_extracts_low_nibble() {
        assert_eq!(elf32_st_type(0xAB), 0xB);
    }

    #[test]
    fn ps2_elf_is_elf_recognises_magic() {
        let dir = std::env::temp_dir();
        let path = dir.join("pcsx2_elfheader_test.elf");
        std::fs::write(&path, [0x7F, b'E', b'L', b'F', 1, 1, 1, 0]).unwrap();
        assert!(ps2ElfIsElf(&path));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn ps2_elf_is_elf_rejects_other_magic() {
        let dir = std::env::temp_dir();
        let path = dir.join("pcsx2_elfheader_test_notelf.bin");
        std::fs::write(&path, [0, 1, 2, 3, 4, 5, 6, 7]).unwrap();
        assert!(!ps2ElfIsElf(&path));
        let _ = std::fs::remove_file(&path);
    }
}
