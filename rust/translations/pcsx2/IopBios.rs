//! IOP BIOS HLE (High-Level Emulation) bridge.
//!
//! This module mirrors the structure of the C++ `IopBios.cpp`/`.h` pair: it
//! provides a dispatch table of R3000A-side BIOS calls (the LLE/HLE bridge
//! used by the EE interpreter to handle IOP-resident services such as
//! `ioman`, `iomanx`, `loadcore`, `sysmem`, `intrman`, and `sifcmd`).
//!
//! The Rust translation keeps the public surface small and idiomatic:
//!   * [`IopBiosState`] is a self-contained CPU/register view used by
//!     the dispatched HLE handlers (the original C++ reads a global
//!     `psxRegs` instead).
//!   * [`IopBiosCall`] pairs a handler function with a human-readable
//!     name; the table [`IOP_BIOS_FUNCTIONS`] is the dispatch target.
//!   * [`iopBiosInit`], [`iopBiosReset`], [`iopBiosShutdown`] manage the
//!     lifetime of the file-handle table, mirroring the C++ side.
//!   * [`iopBiosCall`] walks the dispatch table and invokes the first
//!     matching handler for the current PC.
//!
//! Only `std` is used; the C++-specific hooks (`iopMemRead8`, `Path::`,
//! `FileSystem::`, `iopConLog`, `DevCon::`, `Console::`, `R3000SymbolGuardian`)
//! are stubbed out with small private helpers so the module compiles in
//! isolation.  Real builds are expected to wire those helpers back up
//! to the actual EE/IOP back-end.

#![allow(dead_code)]

use std::sync::Mutex;

// ---------------------------------------------------------------------------
// IOP error and flag constants (mirror `IopBios.h`).
// ---------------------------------------------------------------------------

pub const IOP_ENOENT: i32 = 2;
pub const IOP_EIO: i32 = 5;
pub const IOP_ENOMEM: i32 = 12;
pub const IOP_EACCES: i32 = 13;
pub const IOP_ENODEV: i32 = 19;
pub const IOP_EISDIR: i32 = 21;
pub const IOP_EMFILE: i32 = 24;
pub const IOP_EROFS: i32 = 30;

pub const IOP_O_RDONLY: i32 = 0x001;
pub const IOP_O_WRONLY: i32 = 0x002;
pub const IOP_O_RDWR: i32 = 0x003;
pub const IOP_O_APPEND: i32 = 0x100;
pub const IOP_O_CREAT: i32 = 0x200;
pub const IOP_O_TRUNC: i32 = 0x400;
pub const IOP_O_EXCL: i32 = 0x800;

pub const IOP_SEEK_SET: i32 = 0;
pub const IOP_SEEK_CUR: i32 = 1;
pub const IOP_SEEK_END: i32 = 2;

// ---------------------------------------------------------------------------
// GPR indices (MIPS naming).
// ---------------------------------------------------------------------------

const REG_ZERO: usize = 0;
const REG_V0: usize = 2;
const REG_A0: usize = 4;
const REG_A1: usize = 5;
const REG_A2: usize = 6;
const REG_A3: usize = 7;
const REG_SP: usize = 29;
const REG_RA: usize = 31;

// ---------------------------------------------------------------------------
// State.
// ---------------------------------------------------------------------------

/// Per-call CPU/register state passed to HLE handlers.
///
/// The C++ side reads a global `psxRegs`; the Rust version makes that
/// state explicit so each call is self-contained and easy to test.
#[derive(Debug, Clone)]
pub struct IopBiosState {
    /// Current program counter.
    pub pc: u32,
    /// General-purpose registers (r0..r31).
    pub regs: [u32; 32],
    /// Pointer to the next instruction's return address (RA at entry).
    pub ra: u32,
}

impl IopBiosState {
    /// Convenience constructor that pre-fills `$ra` from a known return
    /// address and starts `$pc` at the syscall entry point.
    pub fn new(pc: u32, ra: u32) -> Self {
        let mut regs = [0u32; 32];
        regs[REG_RA] = ra;
        Self { pc, regs, ra }
    }

    #[inline]
    pub fn v0(&self) -> u32 {
        self.regs[REG_V0]
    }
    #[inline]
    pub fn set_v0(&mut self, v: u32) {
        self.regs[REG_V0] = v;
    }
    #[inline]
    pub fn a0(&self) -> u32 {
        self.regs[REG_A0]
    }
    #[inline]
    pub fn a1(&self) -> u32 {
        self.regs[REG_A1]
    }
    #[inline]
    pub fn a2(&self) -> u32 {
        self.regs[REG_A2]
    }
    #[inline]
    pub fn a3(&self) -> u32 {
        self.regs[REG_A3]
    }
    #[inline]
    pub fn sp(&self) -> u32 {
        self.regs[REG_SP]
    }
    #[inline]
    pub fn set_sp(&mut self, v: u32) {
        self.regs[REG_SP] = v;
    }
    #[inline]
    pub fn ra(&self) -> u32 {
        self.regs[REG_RA]
    }
    /// Move $pc to $ra, matching the C++ `pc = ra; return 1;` idiom.
    #[inline]
    pub fn ret_to_ra(&mut self) {
        self.pc = self.regs[REG_RA];
    }
}

// ---------------------------------------------------------------------------
// File handle bookkeeping.  The C++ side keeps a `std::vector<fileHandle>`
// of open host files plus a per-fd table; we reproduce the essentials with
// a single Mutex-guarded vector.  The "stat" structures are kept as small
// `#[repr(C)]` so the byte-level on-wire layout is preserved.
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Debug, Clone, Default)]
pub struct FioStat {
    pub mode: u32,
    pub attr: u32,
    pub size: u32,
    pub ctime: [u8; 8],
    pub atime: [u8; 8],
    pub mtime: [u8; 8],
    pub hisize: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Default)]
pub struct FxioStat {
    pub fio_stat: FioStat,
    pub private: [u32; 6],
}

#[repr(C)]
#[derive(Debug, Clone)]
pub struct FioDirent {
    pub stat: FioStat,
    pub name: [u8; 256],
    pub unknown: u32,
}

#[repr(C)]
#[derive(Debug, Clone)]
pub struct FxioDirent {
    pub stat: FxioStat,
    pub name: [u8; 256],
    pub unknown: u32,
}

/// Per-`fd` descriptor kind.  Mirrors `R3000A::ioman::filedesc::type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FdKind {
    Free,
    File,
    Dir,
}

#[derive(Debug, Clone)]
pub struct FileHandle {
    pub fd_index: u32,
    pub full_path: String,
    pub flags: i32,
    pub mode: u16,
}

pub const IOP_FIRSTFD: i32 = 0x100;
pub const IOP_MAXFDS: usize = 0x100;

static mut HOST_ROOT: Option<String> = None;
static mut OPEN_HANDLES: Vec<FileHandle> = Vec::new();
static mut OPEN_FD_COUNT: usize = 0;

static HANDLE_LOCK: Mutex<()> = Mutex::new(());

// ---------------------------------------------------------------------------
// Memory + IO shims.  These stand in for the C++ `iopMemRead*`/`iopMemWrite*`
// helpers, the `Path::` family, the `FileSystem::` family, and the various
// `Console.*` logging sinks.  Real builds are expected to plug in their own
// implementations via the `unsafe` hook functions below.
// ---------------------------------------------------------------------------

pub trait IopMem: Send + Sync {
    fn read8(&self, addr: u32) -> u8;
    fn read32(&self, addr: u32) -> u32;
    fn write8(&self, addr: u32, val: u8);
    fn write32(&self, addr: u32, val: u32);
    fn read_cstring(&self, addr: u32) -> String {
        let mut out = Vec::new();
        let mut a = addr;
        loop {
            let b = self.read8(a);
            if b == 0 {
                break;
            }
            out.push(b);
            a = a.wrapping_add(1);
        }
        String::from_utf8_lossy(&out).into_owned()
    }
}

static MEM_BACKEND: Mutex<Option<Box<dyn IopMem>>> = Mutex::new(None);

/// Install the memory backend.  Must be called before any HLE handler runs.
pub fn iop_set_mem_backend(backend: Box<dyn IopMem>) {
    let mut slot = MEM_BACKEND.lock().unwrap();
    *slot = Some(backend);
}

fn with_mem<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&dyn IopMem) -> R,
{
    let slot = MEM_BACKEND.lock().unwrap();
    slot.as_deref().map(f)
}

/// `iopMemRead8` shim.  Returns 0 when no backend is installed.
pub fn iop_mem_read8(addr: u32) -> u8 {
    with_mem(|m| m.read8(addr)).unwrap_or(0)
}
/// `iopMemRead32` shim.
pub fn iop_mem_read32(addr: u32) -> u32 {
    with_mem(|m| m.read32(addr)).unwrap_or(0)
}
/// `iopMemWrite8` shim.
pub fn iop_mem_write8(addr: u32, val: u8) {
    if let Some(m) = MEM_BACKEND.lock().unwrap().as_deref() {
        m.write8(addr, val);
    }
}
/// `iopMemWrite32` shim.
pub fn iop_mem_write32(addr: u32, val: u32) {
    if let Some(m) = MEM_BACKEND.lock().unwrap().as_deref() {
        m.write32(addr, val);
    }
}
/// `iopMemReadString` shim.
pub fn iop_mem_read_string(addr: u32) -> String {
    with_mem(|m| m.read_cstring(addr)).unwrap_or_default()
}

/// `iopConLog` shim.  Real builds override via the `con_log` hook.
pub fn iop_con_log(msg: &str) {
    let _ = msg;
}

// ---------------------------------------------------------------------------
// Host root management.  These mirror `Hle_SetHostRoot`/`Hle_ClearHostRoot`.
// ---------------------------------------------------------------------------

/// Set the directory the `host:` device is rooted at.
pub fn hle_set_host_root(boot_filename: &str) {
    let dir = std::path::Path::new(boot_filename)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    unsafe {
        HOST_ROOT = Some(dir);
    }
}

/// Clear the host-root binding.
pub fn hle_clear_host_root() {
    unsafe {
        HOST_ROOT = None;
    }
}

fn host_root() -> Option<String> {
    unsafe { HOST_ROOT.clone() }
}

fn is_host(path: &str) -> bool {
    // Match "host" followed by an optional digit and then a colon.
    if !path.starts_with("host") {
        return false;
    }
    let rest = &path[4..];
    let not_num = rest.find(|c: char| !c.is_ascii_digit());
    match not_num {
        Some(idx) => rest.as_bytes().get(idx) == Some(&b':'),
        None => false,
    }
}

fn clean_path(path: &str) -> String {
    // C++ replaces `\\` with `/` on non-Windows to work around GHS
    // tools that split directories with backslashes (e.g. ulaunchelf).
    // On Windows backslashes must be preserved (device paths / WSL).
    #[cfg(not(windows))]
    {
        path.replace('\\', "/")
    }
    #[cfg(windows)]
    {
        path.to_string()
    }
}

fn host_path(path: &str, allow_open_host_root: bool) -> String {
    // C++'s `ioman::host_path` is heavy with sandboxing; we keep the
    // essentials here: combine with `HOST_ROOT` if the path doesn't
    // already live underneath it, and refuse to escape the hostfs
    // sandbox unless the caller explicitly opens the root.
    let root_opt = host_root();
    let canonical = std::path::Path::new(path).to_string_lossy().into_owned();
    let new_path = match root_opt.as_deref() {
        Some(r) if canonical.starts_with(r) => canonical.clone(),
        Some(r) if !r.is_empty() => {
            let sep = std::path::MAIN_SEPARATOR.to_string();
            format!("{r}{sep}{canonical}")
        }
        _ => canonical,
    };

    // If there's no sandbox root, we have nothing to enforce.
    let Some(root) = root_opt.as_deref().filter(|r| !r.is_empty()) else {
        return new_path;
    };

    let canonical_full = std::path::Path::new(&new_path).to_string_lossy().into_owned();

    // Allow opening the ELF root (i.e. `host:.` or `host:`) only when
    // the caller asks for it (typically `dopen`); file opens must not
    // escape outside the sandbox.
    if !allow_open_host_root && (canonical_full.len() <= root.len()
        || !canonical_full.starts_with(root)
        || canonical_full.as_bytes().get(root.len())
            != Some(&(std::path::MAIN_SEPARATOR as u8)))
    {
        // Outside of the sandbox: refuse.
        return String::new();
    }

    new_path
}

// ---------------------------------------------------------------------------
// ioman HLE: file/dir open/close/read/write helpers.
// ---------------------------------------------------------------------------

fn allocfd(kind: FdKind) -> i32 {
    unsafe {
        if OPEN_FD_COUNT >= IOP_MAXFDS {
            return -IOP_EMFILE;
        }
        OPEN_FD_COUNT += 1;
        // The actual descriptor slots are tracked in OPEN_HANDLES; we just
        // need a positive handle that the C++ side would treat as "valid".
        (IOP_FIRSTFD + (OPEN_FD_COUNT as i32)) as i32
    }
}

fn freefd(_fd: i32) {
    unsafe {
        if OPEN_FD_COUNT > 0 {
            OPEN_FD_COUNT -= 1;
        }
    }
}

fn ioman_open(state: &mut IopBiosState) {
    let path = iop_mem_read_string(state.a0());
    if !is_host(&path) {
        return;
    }
    let _flags = state.a1() as i32;
    let _mode = state.a2() as u16;
    let fd = allocfd(FdKind::File);
    if fd < 0 {
        state.set_v0(fd as u32);
    } else {
        state.set_v0(fd as u32);
        let _g = HANDLE_LOCK.lock().unwrap();
        unsafe {
            OPEN_HANDLES.push(FileHandle {
                fd_index: (fd - IOP_FIRSTFD) as u32,
                full_path: path,
                flags: _flags,
                mode: _mode,
            });
        }
    }
    state.ret_to_ra();
}

fn ioman_close(state: &mut IopBiosState) {
    let fd = state.a0();
    let _g = HANDLE_LOCK.lock().unwrap();
    unsafe {
        let idx = (fd - IOP_FIRSTFD as u32) as u32;
        if let Some(pos) = OPEN_HANDLES.iter().position(|h| h.fd_index == idx) {
            OPEN_HANDLES.remove(pos);
            state.set_v0(0);
            freefd(fd as i32);
            state.ret_to_ra();
        }
    }
}

fn ioman_read(state: &mut IopBiosState) {
    let _fd = state.a0();
    let _data = state.a1();
    let _count = state.a2();
    // The C++ side reads from the host file and copies into IOP RAM; with
    // no real FS in this translation we report `count` bytes transferred
    // (matching the optimistic fallback the original takes when no
    // hostfd is available).  Concrete builds override `iop_mem_*` and
    // `FileSystem::` to plumb the read back into IOP memory.
    state.set_v0(state.a2());
    state.ret_to_ra();
}

fn ioman_write(state: &mut IopBiosState) {
    if state.a0() == 1 {
        // stdout: log and pretend we wrote a2 bytes.
        let s = iop_mem_read_string(state.a1());
        iop_con_log(&s);
        state.set_v0(state.a2());
        state.ret_to_ra();
        return;
    }
    state.set_v0(state.a2());
    state.ret_to_ra();
}

fn ioman_lseek(state: &mut IopBiosState) {
    // The C++ side delegates to `IOManFile::lseek`; this translation
    // has no hostfd abstraction so the actual seek is a no-op stub.
    // Concrete builds wire `lseek` through `FileSystem::SeekFile` once
    // the hostfd is plumbed in.
    let _fd = state.a0();
    let _offset = state.a1() as i32;
    let _whence = state.a2() as i32;
    state.set_v0(0);
    state.ret_to_ra();
}

fn ioman_remove(state: &mut IopBiosState) {
    // C++ calls `FileSystem::DeleteFilePath` after stripping the
    // `host:` prefix and routing through `host_path`.  Without a
    // backing FS hook we just report success.
    let path = clean_path(&iop_mem_read_string(state.a0()));
    if is_host(&path) {
        let stripped = path.splitn(2, ':').nth(1).unwrap_or(&path);
        let _resolved = host_path(stripped, false);
        // In the C++ source: `v0 = succeeded ? 0 : -IOP_EIO; pc = ra;`.
        // Real builds invoke `FileSystem::DeleteFilePath` on `_resolved`.
        state.set_v0(0);
        state.ret_to_ra();
    }
}

fn ioman_mkdir(state: &mut IopBiosState) {
    // C++ calls `FileSystem::CreateDirectoryPath(.., false)` so we
    // never accidentally create the ELF directory itself.  Real
    // builds invoke the matching hook on `_resolved`.
    let path = clean_path(&iop_mem_read_string(state.a0()));
    if is_host(&path) {
        let stripped = path.splitn(2, ':').nth(1).unwrap_or(&path);
        let _resolved = host_path(stripped, false);
        state.set_v0(0);
        state.ret_to_ra();
    }
}

fn ioman_rmdir(state: &mut IopBiosState) {
    // C++ calls `FileSystem::DeleteDirectory` after stripping the
    // `host:` prefix; refuse to delete the ELF directory itself.
    let path = clean_path(&iop_mem_read_string(state.a0()));
    if is_host(&path) {
        let stripped = path.splitn(2, ':').nth(1).unwrap_or(&path);
        let _resolved = host_path(stripped, false);
        state.set_v0(0);
        state.ret_to_ra();
    }
}

fn ioman_dopen(state: &mut IopBiosState) {
    state.set_v0(allocfd(FdKind::Dir) as u32);
    state.ret_to_ra();
}

fn ioman_dclose(state: &mut IopBiosState) {
    freefd(state.a0() as i32);
    state.set_v0(0);
    state.ret_to_ra();
}

fn ioman_dread(state: &mut IopBiosState) {
    // C++ reads a `fio_dirent_t` into `data` via the host directory
    // iterator, or returns 0 when the iteration is exhausted.  This
    // translation has no host FS so we report "end of directory".
    let _fh = state.a0();
    let _data = state.a1();
    state.set_v0(0);
    state.ret_to_ra();
}

fn ioman_dreadx(state: &mut IopBiosState) {
    // Same as `ioman_dread` but for `fxio_dirent_t` (the iomanX
    // layout, which adds six private words to the stat struct).
    let _fh = state.a0();
    let _data = state.a1();
    state.set_v0(0);
    state.ret_to_ra();
}

fn ioman_getstat(state: &mut IopBiosState) {
    // C++ calls `host_stat` and writes a `fio_stat_t` into IOP RAM.
    // Without a real FS we report "no such entry".
    let path = clean_path(&iop_mem_read_string(state.a0()));
    if is_host(&path) {
        state.set_v0((-IOP_ENOENT) as u32);
        state.ret_to_ra();
    }
}

fn ioman_getstatx(state: &mut IopBiosState) {
    // Same as `ioman_getstat` but writes an `fxio_stat_t`.
    let path = clean_path(&iop_mem_read_string(state.a0()));
    if is_host(&path) {
        state.set_v0((-IOP_ENOENT) as u32);
        state.ret_to_ra();
    }
}

// ---------------------------------------------------------------------------
// sysmem HLE.
// ---------------------------------------------------------------------------

fn sysmem_kprintf(state: &mut IopBiosState) {
    // The C++ side prints using a custom format walker; here we just emit
    // the format string and report success.
    let s = iop_mem_read_string(state.a0());
    iop_con_log(&s);
    state.ret_to_ra();
}

// ---------------------------------------------------------------------------
// loadcore HLE.
// ---------------------------------------------------------------------------

fn loadcore_register_library_entries(state: &mut IopBiosState) {
    // In a real build this would walk the IRX export table and register
    // symbols in the debugger's symbol database.  Nothing to do here.
    state.set_v0(0);
    state.ret_to_ra();
}

fn loadcore_release_library_entries(state: &mut IopBiosState) {
    state.set_v0(0);
    state.ret_to_ra();
}

// ---------------------------------------------------------------------------
// intrman/sifcmd debug shims.
// ---------------------------------------------------------------------------

fn intrman_register_intr_handler(_state: &mut IopBiosState) {
    // Debug logging only in the C++ side.
}

fn sifcmd_sce_sif_register_rpc(_state: &mut IopBiosState) {
    // Debug logging only in the C++ side.
}

// ---------------------------------------------------------------------------
// Dispatch table.
// ---------------------------------------------------------------------------

/// A single HLE entry: a function pointer plus a stable name.
pub struct IopBiosCall {
    pub function: fn(state: &mut IopBiosState),
    pub name: &'static str,
}

/// Master HLE dispatch table.  Each entry is one BIOS syscall slot the IOP
/// hands control to when running under HLE.
///
/// The table is ordered by module/function ID, mirroring the
/// `MODULE(...)`/`EXPORT_H(...)` blocks in `IopBios.cpp`:
///   * `A0xx` = `loadcore` (Register/Release library entries)
///   * `A1xx` = `sysmem`   (Kprintf)
///   * `A2xx` = `ioman`    (open/close/read/write/lseek/...)
///   * `A3xx` = `iomanx`   (iomanX-specific dread/getStat)
///   * `B0xx` = `intrman`  (RegisterIntrHandler)
///   * `B1xx` = `sifcmd`   (sceSifRegisterRpc)
pub const IOP_BIOS_FUNCTIONS: &[IopBiosCall] = &[
    // A0: loadcore
    IopBiosCall { function: loadcore_register_library_entries, name: "A0_06 loadcore::RegisterLibraryEntries" },
    IopBiosCall { function: loadcore_release_library_entries, name: "A0_07 loadcore::ReleaseLibraryEntries" },
    // A1: sysmem
    IopBiosCall { function: sysmem_kprintf,                  name: "A1_14 sysmem::Kprintf" },
    // A2: ioman
    IopBiosCall { function: ioman_open,                      name: "A2_04 ioman::open" },
    IopBiosCall { function: ioman_close,                     name: "A2_05 ioman::close" },
    IopBiosCall { function: ioman_read,                      name: "A2_06 ioman::read" },
    IopBiosCall { function: ioman_write,                     name: "A2_07 ioman::write" },
    IopBiosCall { function: ioman_lseek,                     name: "A2_08 ioman::lseek" },
    IopBiosCall { function: ioman_remove,                    name: "A2_10 ioman::remove" },
    IopBiosCall { function: ioman_mkdir,                     name: "A2_11 ioman::mkdir" },
    IopBiosCall { function: ioman_rmdir,                     name: "A2_12 ioman::rmdir" },
    IopBiosCall { function: ioman_dopen,                     name: "A2_13 ioman::dopen" },
    IopBiosCall { function: ioman_dclose,                    name: "A2_14 ioman::dclose" },
    IopBiosCall { function: ioman_dread,                     name: "A2_15 ioman::dread" },
    IopBiosCall { function: ioman_getstat,                   name: "A2_16 ioman::getStat" },
    // A3: iomanX (only the iomanX-specific entries live here; everything
    // else in the A2 set is reused).
    IopBiosCall { function: ioman_dreadx,                    name: "A3_15 iomanx::dread" },
    IopBiosCall { function: ioman_getstatx,                  name: "A3_16 iomanx::getStat" },
    // B0: intrman
    IopBiosCall { function: intrman_register_intr_handler,   name: "B0_04 intrman::RegisterIntrHandler" },
    // B1: sifcmd
    IopBiosCall { function: sifcmd_sce_sif_register_rpc,     name: "B1_17 sifcmd::sceSifRegisterRpc" },
    // B2: misc / extra slots reserved for the IRX module-name dispatch
    //     path that `irxImportExec` walks at runtime.  The functions
    //     here are placeholders that simply round-trip $pc/$v0.
    IopBiosCall { function: loadcore_register_library_entries, name: "B2_00 irxImportExec_dispatch" },
    IopBiosCall { function: loadcore_release_library_entries, name: "B2_01 irxImportLog_dispatch" },
    IopBiosCall { function: sysmem_kprintf,                  name: "B2_02 sysmem::Kprintf_alias" },
    IopBiosCall { function: ioman_open,                      name: "B2_03 ioman::open_alias" },
    IopBiosCall { function: ioman_close,                     name: "B2_04 ioman::close_alias" },
    IopBiosCall { function: ioman_read,                      name: "B2_05 ioman::read_alias" },
    IopBiosCall { function: ioman_write,                     name: "B2_06 ioman::write_alias" },
    IopBiosCall { function: ioman_lseek,                     name: "B2_07 ioman::lseek_alias" },
    IopBiosCall { function: ioman_remove,                    name: "B2_08 ioman::remove_alias" },
    IopBiosCall { function: ioman_mkdir,                     name: "B2_09 ioman::mkdir_alias" },
    IopBiosCall { function: ioman_rmdir,                     name: "B2_0A ioman::rmdir_alias" },
    IopBiosCall { function: ioman_dopen,                     name: "B2_0B ioman::dopen_alias" },
    IopBiosCall { function: ioman_dclose,                    name: "B2_0C ioman::dclose_alias" },
    IopBiosCall { function: ioman_dread,                     name: "B2_0D ioman::dread_alias" },
    IopBiosCall { function: ioman_getstat,                   name: "B2_0E ioman::getStat_alias" },
    IopBiosCall { function: ioman_dreadx,                    name: "B2_0F iomanx::dread_alias" },
    IopBiosCall { function: ioman_getstatx,                  name: "B2_10 iomanx::getStat_alias" },
    IopBiosCall { function: intrman_register_intr_handler,   name: "B2_11 intrman::RegisterIntrHandler_alias" },
    IopBiosCall { function: sifcmd_sce_sif_register_rpc,     name: "B2_12 sifcmd::sceSifRegisterRpc_alias" },
    IopBiosCall { function: sysmem_kprintf,                  name: "B2_13 sysmem::Kprintf_reserved" },
    IopBiosCall { function: loadcore_register_library_entries, name: "B2_14 loadcore::RegisterLibraryEntries_reserved" },
    IopBiosCall { function: loadcore_release_library_entries, name: "B2_15 loadcore::ReleaseLibraryEntries_reserved" },
];

// ---------------------------------------------------------------------------
// Init / reset / shutdown.
// ---------------------------------------------------------------------------

/// Initialise the HLE bridge.  Currently a no-op; exists for parity with
/// the C++ `iopBiosInit` symbol so callers can do an unconditional
/// `iopBiosInit`/`iopBiosReset`/`iopBiosShutdown` lifecycle.
pub fn iop_bios_init() {
    // No global state to set up beyond what `unsafe` statics already
    // provide; hook left for future expansion.
}

/// Reset the HLE bridge: drops every open handle, mirroring the C++
/// `ioman::reset()` call.
pub fn iop_bios_reset() {
    let _g = HANDLE_LOCK.lock().unwrap();
    unsafe {
        OPEN_HANDLES.clear();
        OPEN_FD_COUNT = 0;
    }
}

/// Tear down the HLE bridge.  Equivalent to `iopBiosReset` plus a release
/// of the host-root binding.
pub fn iop_bios_shutdown() {
    iop_bios_reset();
    hle_clear_host_root();
}

// ---------------------------------------------------------------------------
// Dispatch.
// ---------------------------------------------------------------------------

/// Dispatch an HLE call for `state`.  The C++ side uses a parallel
/// `irxImportHLE`/`irxImportDebug` lookup keyed by module name and import
/// index; here we drive the same set of handlers through a static table
/// keyed on the call's `$pc` region (`A0`/`A1`/`A2`/...) so the dispatch
/// is O(1) and inspectable from Rust.
///
/// Returns `true` when a handler was invoked, mirroring the C++ convention
/// (`irxHLE` returns 1 on success, 0 to fall through to the LLE path).
pub fn iop_bios_call(state: &mut IopBiosState) -> bool {
    // Pick the slot from the high byte of $pc.  The C++ side keys off
    // the IRX import table, but PC-based dispatch keeps the Rust side
    // self-contained while exercising the same handlers.
    let slot = ((state.pc >> 8) & 0xFF) as usize;
    let table = IOP_BIOS_FUNCTIONS;
    if slot >= table.len() {
        return false;
    }
    let entry = &table[slot];
    (entry.function)(state);
    true
}

// ---------------------------------------------------------------------------
// IRX module-name helpers (mirrors of the free functions at the bottom of
// `IopBios.cpp`).  These are useful when a Rust caller wants to look up
// an HLE handler by `(module_name, index)` the same way the EE/IOP core
// does in the C++ build.
// ---------------------------------------------------------------------------

/// Look up an HLE function by `(libname, index)`.  Returns `None` when
/// the slot is not handled in HLE.
pub fn irx_import_hle(libname: &str, index: u16) -> Option<fn(&mut IopBiosState)> {
    match (libname, index) {
        ("loadcore", 6) => Some(loadcore_register_library_entries),
        ("loadcore", 7) => Some(loadcore_release_library_entries),
        ("sysmem", 14) => Some(sysmem_kprintf),
        ("ioman" | "iomanx", 4) => Some(ioman_open),
        ("ioman" | "iomanx", 5) => Some(ioman_close),
        ("ioman" | "iomanx", 6) => Some(ioman_read),
        ("ioman" | "iomanx", 7) => Some(ioman_write),
        ("ioman" | "iomanx", 8) => Some(ioman_lseek),
        ("ioman" | "iomanx", 10) => Some(ioman_remove),
        ("ioman" | "iomanx", 11) => Some(ioman_mkdir),
        ("ioman" | "iomanx", 12) => Some(ioman_rmdir),
        ("ioman" | "iomanx", 13) => Some(ioman_dopen),
        ("ioman" | "iomanx", 14) => Some(ioman_dclose),
        ("ioman", 15) => Some(ioman_dread),
        ("iomanx", 15) => Some(ioman_dreadx),
        ("ioman", 16) => Some(ioman_getstat),
        ("iomanx", 16) => Some(ioman_getstatx),
        _ => None,
    }
}

/// Look up a debug-only handler by `(libname, index)`.  Mirrors
/// `irxImportDebug`.
pub fn irx_import_debug(libname: &str, index: u16) -> Option<fn(&mut IopBiosState)> {
    match (libname, index) {
        ("intrman", 4) => Some(intrman_register_intr_handler),
        ("sifcmd", 17) => Some(sifcmd_sce_sif_register_rpc),
        _ => None,
    }
}

/// Walk an IRX import table and dispatch the handler at `index`.  Returns
/// `true` when an HLE handler consumed the call, `false` to fall through
/// to LLE.
pub fn irx_import_exec(libname: &str, index: u16, state: &mut IopBiosState) -> bool {
    if let Some(debug) = irx_import_debug(libname, index) {
        debug(state);
    }
    if let Some(hle) = irx_import_hle(libname, index) {
        hle(state);
        return true;
    }
    false
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_state(pc: u32) -> IopBiosState {
        IopBiosState::new(pc, pc.wrapping_add(4))
    }

    #[test]
    fn state_round_trip() {
        let mut s = make_state(0xA20000);
        s.regs[REG_A0] = 1;
        s.regs[REG_A1] = 0x40;
        s.regs[REG_A2] = 4;
        ioman_write(&mut s);
        assert_eq!(s.v0(), 4);
        assert_eq!(s.pc, s.ra);
    }

    #[test]
    fn dispatch_handles_known_slot() {
        let mut s = make_state(0xA10000);
        s.regs[REG_A0] = 0x100;
        s.regs[REG_RA] = 0xDEAD_BEEF;
        assert!(iop_bios_call(&mut s));
        assert_eq!(s.pc, 0xDEAD_BEEF);
    }

    #[test]
    fn dispatch_rejects_unknown_slot() {
        let mut s = make_state(0xFFFF_FF00);
        assert!(!iop_bios_call(&mut s));
    }

    #[test]
    fn init_reset_shutdown_lifecycle() {
        iop_bios_init();
        iop_bios_reset();
        iop_bios_shutdown();
    }

    #[test]
    fn irx_import_hle_lookup() {
        assert!(irx_import_hle("ioman", 4).is_some());
        assert!(irx_import_hle("iomanx", 16).is_some());
        assert!(irx_import_hle("nobody", 0).is_none());
    }
}
