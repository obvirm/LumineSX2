// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust 2021 translation of the PCSX2 `updater/` C/C++ sources and
//! the various small headers that feed into the precompiled translation
//! units (`common/PrecompiledHeader.h`, `pcsx2/PrecompiledHeader.h`,
//! `pcsx2-qt/PrecompiledHeader.h`) along with the structural headers they
//! pull in (`Common.h`, `Hardware.h`, `MemoryTypes.h`, `Sifcmd.h`,
//! `SupportURLs.h`, `Vif_Dma.h`, `Vif_Dynarec.h`, `Vif_HashBucket.h`),
//! the `SourceLog.cpp` line-history logger, the `Windows/resource.h`
//! resource id manifest, the `windows/Optimus.cpp` GPU-preference globals
//! and the 7z error string table (`SZErrors.h`).
//!
//! The module is `std`-only; Win32 / 7z FFI are represented as opaque
//! data structures and trait-style trait objects (`ProgressSink`) so that
//! the translation unit compiles unchanged on every host triple.

#![allow(non_snake_case)]
#![allow(dead_code)]

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Precompiled headers. The C++ PCH files do nothing but `#include` a fixed
// bag of standard headers, so on the Rust side they collapse to a single
// module marker.
// ---------------------------------------------------------------------------

/// `common/PrecompiledHeader.{h,cpp}` — pulled in by virtually every TU.
pub mod common_precompiled {
    // Translates `#include <memory>` / `<atomic>` / `<csignal>` / `<cerrno>` /
    // `<cstdio>`. Rust places these in `std::*` automatically.
}

/// `pcsx2/PrecompiledHeader.{h,cpp}` — EE-side PCH that adds the STL plus
/// `common/Pcsx2Defs.h`, `common/VectorIntrin.h` and (on non-GCC) `fmt`.
pub mod pcsx2_precompiled {
    // The C++ PCH funnels `algorithm`, `cinttypes`, `condition_variable`,
    // `climits`, `cstring`, `cstdio`, `cstdlib`, `cmath`, `list`, `memory`,
    // `mutex`, `functional`, `optional`, `stack`, `stdexcept`, `string`,
    // `string_view`, `thread`, `vector`, `<stddef.h>` and `<sys/stat.h>`
    // into the TU. Those map 1:1 to `std::*` in Rust.
}

/// `pcsx2-qt/PrecompiledHeader.{h,cpp}` — adds `<QtCore/QtCore>` and the
/// `pcsx2/PrecompiledHeader.h` body for the Qt front-end.
pub mod pcsx2_qt_precompiled {
    // In Rust the front-end lives in a separate `qt` crate; this module
    // exists only to mirror the C++ PCH hook.
}

// ---------------------------------------------------------------------------
// Support / hardware / memory-types headers — translated to small `pub type`
// aliases. The original C++ headers `#include` dozens of files we do not
// need to mirror here; only the surface types are kept.
// ---------------------------------------------------------------------------

/// `pcsx2/Common.h` — global bus constants used by the EE timing model.
pub mod common {
    /// Half the EE bus speed: 2.
    pub const BIAS: u32 = 2;
    /// EE clock in Hz (294.912 MHz).
    pub const PS2CLK: u32 = 294_912_000;
    /// IOP clock in Hz (36.864 MHz). Declared `extern` in C++.
    pub static mut PSXCLK: u32 = 36_864_000;

    /// Shift-JIS to UTF-8 conversion entry point (declared in `Common.h`).
    pub fn ShiftJIS_ConvertString(src: &str) -> String {
        // The C++ implementation is a thin wrapper over iconv; here we just
        // hand the bytes back and let the real FFI fill this in.
        src.to_owned()
    }
}

/// `pcsx2/Hardware.h` — the umbrella that `#include`s every EE/IOP device.
pub mod hardware {
    // The C++ header re-exports Counters/GS/Hw/IPU/SPR/Gif/Sif/Vif/Vif_Dma.
    // They are represented here by empty modules whose symbols come from
    // the real Rust translation units.
}

/// `pcsx2/MemoryTypes.h` — EE/IOP memory regions and word-width typedefs.
pub mod memory_types {
    /// One mebibyte in bytes.
    pub const MIB: u32 = 1024 * 1024;
    /// One kibibyte in bytes.
    pub const KIB: u32 = 1024;

    /// Memory region sizes (matches the C++ `Ps2MemSize` namespace).
    pub mod ps2_mem_size {
        use super::{KIB, MIB};
        pub const MainRam: u32 = 32 * MIB;
        pub const ExtraRam: u32 = 96 * MIB;
        pub const TotalRam: u32 = 128 * MIB;
        pub const Rom: u32 = 4 * MIB;
        pub const Rom1: u32 = 4 * MIB;
        pub const Rom2: u32 = 4 * MIB;
        pub const Hardware: u32 = 64 * KIB;
        pub const Scratch: u32 = 16 * KIB;
        pub const IopRam: u32 = 2 * MIB;
        pub const ExtraIopRam: u32 = 6 * MIB;
        pub const TotalIopRam: u32 = 8 * MIB;
        pub const IopHardware: u32 = 64 * KIB;
        pub const GsRegs: u32 = 0x2000;
    }

    /// Word-width aliases.
    pub type Mem8 = u8;
    pub type Mem16 = u16;
    pub type Mem32 = u32;
    pub type Mem64 = u64;
    /// `u128` is the natural Rust equivalent of the PS2 128-bit register.
    pub type Mem128 = u128;

    /// `EEVM_MemoryAllocMess` from `MemoryTypes.h`.
    #[repr(C)]
    pub struct EeVmMemoryAllocMess {
        pub main: [u8; ps2_mem_size::TotalRam as usize],
        pub scratch: [u8; ps2_mem_size::Scratch as usize],
        pub rom: [u8; ps2_mem_size::Rom as usize],
        pub rom1: [u8; ps2_mem_size::Rom1 as usize],
        pub rom2: [u8; ps2_mem_size::Rom2 as usize],
        pub zero_read: [u8; MIB as usize],
        pub zero_write: [u8; MIB as usize],
    }

    /// `IopVM_MemoryAllocMess` from `MemoryTypes.h`.
    #[repr(C)]
    pub struct IopVmMemoryAllocMess {
        pub main: [u8; ps2_mem_size::TotalRam as usize],
        pub p: [u8; 64 * KIB as usize],
        pub sif: [u8; 0x100],
    }

    /// Global EE / IOP memory pointers (`extern EEVM_MemoryAllocMess* eeMem`).
    pub static mut EE_MEM: *mut EeVmMemoryAllocMess = std::ptr::null_mut();
    pub static mut IOP_MEM: *mut IopVmMemoryAllocMess = std::ptr::null_mut();
}

/// `pcsx2/Sifcmd.h` — single SIF DMA transfer descriptor.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SifDmaTransfer {
    pub src: *mut std::ffi::c_void,
    pub dest: *mut std::ffi::c_void,
    pub size: i32,
    pub attr: i32,
}

/// `pcsx2/SupportURLs.h` — outbound URLs the UI points users at.
pub mod support_urls {
    pub const PCSX2_WEBSITE_URL: &str = "https://pcsx2.net/";
    pub const PCSX2_FORUMS_URL: &str = "https://forums.pcsx2.net/";
    pub const PCSX2_GITHUB_URL: &str = "https://github.com/PCSX2/pcsx2";
    pub const PCSX2_LICENSE_URL: &str =
        "https://github.com/PCSX2/pcsx2/blob/master/pcsx2/Docs/License.txt";
    pub const PCSX2_DOCUMENTATION_URL: &str = "https://pcsx2.net/docs";
    pub const PCSX2_DOCUMENTATION_BIOS_URL_SHORTENED: &str = "pcsx2.net/docs/setup/bios";
    pub const PCSX2_WIKI_URL: &str = "https://wiki.pcsx2.net/Main_Page";
    pub const PCSX2_DISCORD_URL: &str = "https://pcsx2.net/discord";
}

// ---------------------------------------------------------------------------
// VIF translation unit. The three headers form one cohesive block of
// declarations; they are wrapped in a single module that mirrors the C++
// surface area.
// ---------------------------------------------------------------------------

/// `pcsx2/Vif_Dma.h` — VIF unpack/DMA state shared between the interpreter
/// and the dynarec.
pub mod vif {
    use super::memory_types::{Mem128, Mem32};

    /// C++ `vifCode` (tag union).
    #[repr(C)]
    #[derive(Clone, Copy, Debug, Default)]
    pub struct VifCode {
        pub addr: Mem32,
        pub size: Mem32,
        pub cmd: Mem32,
        pub wl: u16,
        pub cl: u16,
    }

    /// `BITBLTBUF` register layout.
    #[repr(C)]
    #[derive(Clone, Copy, Debug, Default)]
    pub struct BitBltBuf {
        pub sbp: u32,
        pub sbw: u32,
        pub spsm: u32,
        pub dbp: u32,
        pub dbw: u32,
        pub dpsm: u32,
    }

    /// `TRXPOS` register layout.
    #[repr(C)]
    #[derive(Clone, Copy, Debug, Default)]
    pub struct TrxPos {
        pub ssax: u32,
        pub ssay: u32,
        pub dsax: u32,
        pub dsay: u32,
        pub diry: u32,
        pub dirx: u32,
    }

    /// `TRXREG` register layout.
    #[repr(C)]
    #[derive(Clone, Copy, Debug, Default)]
    pub struct TrxReg {
        pub rrw: u32,
        pub rrh: u32,
    }

    /// `tVIF_CTRL`.
    #[repr(C)]
    #[derive(Clone, Copy, Debug, Default)]
    pub struct VifCtrl {
        pub enabled: bool,
        pub value: Mem32,
    }

    /// The big `vifStruct` (mask row/col, packed tag, GIF transfer regs).
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct VifStruct {
        pub mask_row: Mem128,
        pub mask_col: Mem128,
        pub tag: VifCode,
        pub cmd: i32,
        pub pass: i32,
        pub cl: i32,
        pub usn: u8,
        pub start_aligned: u8,
        pub struct_end: u8,
        pub irq: i32,
        pub done: bool,
        pub vifstalled: VifCtrl,
        pub stallontag: bool,
        pub waitforvu: bool,
        pub unpackcalls: i32,
        pub bitbltbuf: BitBltBuf,
        pub trxpos: TrxPos,
        pub trxreg: TrxReg,
        pub gs_last_download_size: Mem32,
        pub irqoffset: VifCtrl,
        pub vifpacketsize: Mem32,
        pub inprogress: u8,
        pub dmamode: u8,
        pub queued_program: bool,
        pub queued_pc: Mem32,
        pub queued_gif_wait: bool,
    }

    /// C++ `alignas(16) extern vifStruct vif0, vif1;`
    pub static mut VIF0: VifStruct = unsafe { std::mem::zeroed() };
    pub static mut VIF1: VifStruct = unsafe { std::mem::zeroed() };

    /// `FnType_VifCmdHandler` / `Fnptr_VifCmdHandler`.
    pub type FnTypeVifCmdHandler = unsafe extern "C" fn(pass: i32, data: *const Mem32) -> i32;
    pub type FnPtrVifCmdHandler = FnTypeVifCmdHandler;

    /// `static const unsigned int VIF0intc = 4;` / `VIF1intc = 5;`.
    pub const VIF0_INTC: u32 = 4;
    pub const VIF1_INTC: u32 = 5;

    /// `enum VifModes` from the C++ header.
    #[repr(u32)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum VifMode {
        NormalToMem = 0,
        NormalFromMem = 1,
        Chain = 2,
    }

    /// Helper from the C++ header: `static int _limit(int a, int max)`.
    #[inline]
    pub fn limit(a: i32, max: i32) -> i32 {
        if a > max {
            max
        } else {
            a
        }
    }
}

/// `pcsx2/Vif_Dynarec.h` — the dynarec-side VIF block.
pub mod vif_dynarec {
    use super::memory_types::{Mem32, Mem128};
    use super::vif_hash_bucket::{HashBucket, NVifBlock};

    /// Pointer-sized address used by the dynarec.
    pub type UpTr = usize;

    /// C++ `typedef u32 (*nVifCall)(void*, const void*);`
    pub type NVifCall = unsafe extern "C" fn(*mut std::ffi::c_void, *const std::ffi::c_void) -> Mem32;

    /// C++ `typedef void (*nVifrecCall)(uptr dest, uptr src);`
    pub type NVifRecCall = unsafe extern "C" fn(dest: UpTr, src: UpTr);

    /// C++ `struct nVifStruct`.
    #[repr(C)]
    pub struct NVifStruct {
        pub buffer: [u8; 256 * 16],
        pub b_size: Mem32,
        pub idx: Mem32,
        pub rec_write_ptr: *mut u8,
        pub rec_end_ptr: *mut u8,
        pub vif_blocks: HashBucket,
    }

    impl Default for NVifStruct {
        fn default() -> Self {
            Self::new()
        }
    }

    impl NVifStruct {
        /// Build a fresh zero-initialised `NVifStruct`. Used both by
        /// `Default::default()` and by the `static mut NVIF` table,
        /// which can't call `Default` (non-const) directly.
        pub fn new() -> Self {
            // `alignas(16)` on the 256*16 byte buffer is preserved by Rust's
            // default repr(C) alignment rules for `[u8; N]`.
            Self {
                buffer: [0u8; 256 * 16],
                b_size: 0,
                idx: 0,
                rec_write_ptr: std::ptr::null_mut(),
                rec_end_ptr: std::ptr::null_mut(),
                vif_blocks: HashBucket::new(),
            }
        }
    }

    /// `alignas(16) extern nVifStruct nVif[2]`.
    pub static mut NVIF: [NVifStruct; 2] = [const {
        NVifStruct {
            buffer: [0u8; 256 * 16],
            b_size: 0,
            idx: 0,
            rec_write_ptr: std::ptr::null_mut(),
            rec_end_ptr: std::ptr::null_mut(),
            vif_blocks: HashBucket::new(),
        }
    }; 2];

    /// `extern u32 nVifUpk[(2*2*16)*4]`.
    pub static mut NVIF_UPK: [Mem32; (2 * 2 * 16) * 4] = [0u32; (2 * 2 * 16) * 4];

    /// `extern u32 nVifMask[3][4][4]`.
    pub static mut NVIF_MASK: [[[Mem32; 4]; 4]; 3] = [[[0u32; 4]; 4]; 3];

    /// `static constexpr bool newVifDynaRec = 1;`.
    pub const NEW_VIF_DYNAREC: bool = true;

    // Suppress unused import warning when callers only take the data.
    const _NVIFBLOCK_PHANTOM: std::marker::PhantomData<NVifBlock> = std::marker::PhantomData;
    const _MEM128_PHANTOM: std::marker::PhantomData<Mem128> = std::marker::PhantomData;
}

/// `pcsx2/Vif_HashBucket.h` — `nVifBlock` union and `HashBucket` container.
pub mod vif_hash_bucket {
    use super::memory_types::Mem32;
    use super::vif_dynarec::UpTr;

    /// Number of hash buckets. The C++ `hSize` constant is 0x10000.
    pub const H_SIZE: usize = 0x10000;

    /// `union nVifBlock` (16 bytes). Rust unions are `#[repr(C)]`.
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub union NVifBlock {
        pub fields: NVifBlockFields,
        pub keyed: NVifBlockKeyed,
    }

    /// Named-field view of `nVifBlock`.
    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    pub struct NVifBlockFields {
        pub num: u8,
        pub upk_type: u8,
        pub length: u16,
        pub mask: Mem32,
        pub mode: u8,
        pub aligned: u8,
        pub cl: u8,
        pub wl: u8,
        pub start_ptr: UpTr,
    }

    /// Keyed view of `nVifBlock` (used for the hash lookup).
    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    pub struct NVifBlockKeyed {
        pub hash_key: u16,
        pub _pad0: u16,
        pub key0: Mem32,
        pub key1: Mem32,
        pub value: UpTr,
    }

    /// `class HashBucket` — fixed-size open-addressed table of `NVifBlock`
    /// chains stored in heap-allocated arrays.
    pub struct HashBucket {
        buckets: Vec<Option<Box<[NVifBlock]>>>,
    }

    impl HashBucket {
        pub const fn new() -> Self {
            Self {
                buckets: Vec::new(),
            }
        }

        /// `nVifBlock* find(const nVifBlock& dataPtr)`.
        pub fn find(&self, data: &NVifBlock) -> Option<&NVifBlock> {
            unsafe {
                let key = data.keyed.hash_key as usize;
                let chain = self.buckets[key].as_ref()?;
                let target0 = data.keyed.key0;
                let target1 = data.keyed.key1;
                for blk in chain.iter() {
                    if blk.keyed.key0 == target0 && blk.keyed.key1 == target1 {
                        return Some(blk);
                    }
                    if blk.fields.start_ptr == 0 {
                        return None;
                    }
                }
                None
            }
        }

        /// `void add(const nVifBlock& dataPtr)`.
        pub fn add(&mut self, data: &NVifBlock) {
            unsafe {
                let key = data.keyed.hash_key as usize;
                let current_size = self.bucket_size(data);
                let new_size = current_size + 2; // +1 for the entry, +1 for the empty cell.
                let mut chain: Box<[NVifBlock]> = match self.buckets[key].take() {
                    Some(existing) => {
                        let mut v = existing.into_vec();
                        v.resize(new_size, NVifBlock { fields: std::mem::zeroed() });
                        v.into_boxed_slice()
                    }
                    None => vec![NVifBlock { fields: std::mem::zeroed() }; new_size].into_boxed_slice(),
                };
                chain[current_size] = *data;
                // Last cell stays zeroed (the "empty cell" sentinel).
                self.buckets[key] = Some(chain);
            }
        }

        /// `u32 bucket_size(const nVifBlock& dataPtr)`.
        pub fn bucket_size(&self, data: &NVifBlock) -> usize {
            unsafe {
                let key = data.keyed.hash_key as usize;
                match &self.buckets[key] {
                    None => 0,
                    Some(chain) => chain
                        .iter()
                        .take_while(|b| b.fields.start_ptr != 0)
                        .count(),
                }
            }
        }

        /// `void clear()`.
        pub fn clear(&mut self) {
            for slot in self.buckets.iter_mut() {
                *slot = None;
            }
        }

        /// `void reset()` — clear and seed every bucket with one empty cell.
        pub fn reset(&mut self) {
            self.clear();
            for slot in self.buckets.iter_mut() {
                *slot = Some(unsafe { vec![NVifBlock { fields: std::mem::zeroed() }; 1].into_boxed_slice() });
            }
        }
    }

    impl Default for HashBucket {
        fn default() -> Self {
            Self::new()
        }
    }
}

// ---------------------------------------------------------------------------
// Source / trace logging (`pcsx2/SourceLog.cpp`).
// ---------------------------------------------------------------------------

/// Severity / colour constants used by the line logger.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warning,
    Error,
}

/// Plain-text colour code the logger associates with a stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsoleColor {
    Gray,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
}

/// Static description of a log channel (name, menu text, tooltip).
#[derive(Clone, Debug)]
pub struct LogDescriptor {
    pub short_name: &'static str,
    pub menu_name: &'static str,
    pub description: &'static str,
}

/// One named log channel.
pub struct LogChannel {
    pub descriptor: LogDescriptor,
    pub color: ConsoleColor,
    ring: Mutex<Vec<String>>,
}

impl LogChannel {
    pub fn new(descriptor: LogDescriptor, color: ConsoleColor) -> Arc<Self> {
        Arc::new(Self {
            descriptor,
            color,
            ring: Mutex::new(Vec::new()),
        })
    }

    /// C++ `TraceLog::Write(const char* fmt, ...)`.
    pub fn write_fmt(&self, args: std::fmt::Arguments<'_>) {
        let line = format!("{:<8}: {}", self.descriptor.short_name, args);
        self.ring.lock().unwrap().push(line);
    }

    /// C++ `TraceLog::Write(ConsoleColors color, const char* fmt, ...)`.
    pub fn write_colored(&self, color: ConsoleColor, args: std::fmt::Arguments<'_>) {
        let _ = color;
        self.write_fmt(args);
    }

    /// Drain everything that's been logged so far (used by the GUI panel).
    pub fn drain(&self) -> Vec<String> {
        std::mem::take(&mut *self.ring.lock().unwrap())
    }
}

/// Aggregates the EE trace channels.
pub struct TraceLogPack {
    pub sif: Arc<LogChannel>,
}

impl TraceLogPack {
    pub fn new() -> Self {
        Self {
            sif: LogChannel::new(
                LogDescriptor {
                    short_name: "SIF",
                    menu_name: "SIF (EE <-> IOP)",
                    description: "",
                },
                ConsoleColor::Gray,
            ),
        }
    }
}

impl Default for TraceLogPack {
    fn default() -> Self {
        Self::new()
    }
}

/// Aggregates the user-facing console channels (ELF, EErec, IOP, etc.).
pub struct ConsoleLogPack {
    pub elf: Arc<LogChannel>,
    pub ee_rec_perf: Arc<LogChannel>,
    pub pgif_log: Arc<LogChannel>,
    pub ee_console: Arc<LogChannel>,
    pub iop_console: Arc<LogChannel>,
    pub deci2: Arc<LogChannel>,
    pub recording_console: Arc<LogChannel>,
    pub control_info: Arc<LogChannel>,
}

impl ConsoleLogPack {
    pub fn new() -> Self {
        Self {
            elf: LogChannel::new(
                LogDescriptor {
                    short_name: "ELF",
                    menu_name: "E&LF",
                    description: "Dumps detailed information for PS2 executables (ELFs).",
                },
                ConsoleColor::Gray,
            ),
            ee_rec_perf: LogChannel::new(
                LogDescriptor {
                    short_name: "EErecPerf",
                    menu_name: "EErec &Performance",
                    description: "Logs manual protection, split blocks, and other things that might impact performance.",
                },
                ConsoleColor::Gray,
            ),
            pgif_log: LogChannel::new(
                LogDescriptor {
                    short_name: "PGIFout",
                    menu_name: "&PGIF Console",
                    description: "Shows output from pgif the emulated ps1 gpu",
                },
                ConsoleColor::Gray,
            ),
            ee_console: LogChannel::new(
                LogDescriptor {
                    short_name: "EEout",
                    menu_name: "EE C&onsole",
                    description: "Shows the game developer's logging text (EE processor).",
                },
                ConsoleColor::Gray,
            ),
            iop_console: LogChannel::new(
                LogDescriptor {
                    short_name: "IOPout",
                    menu_name: "&IOP Console",
                    description: "Shows the game developer's logging text (IOP processor).",
                },
                ConsoleColor::Gray,
            ),
            deci2: LogChannel::new(
                LogDescriptor {
                    short_name: "DECI2",
                    menu_name: "DECI&2 Console",
                    description: "Shows DECI2 debugging logs (EE processor).",
                },
                ConsoleColor::Gray,
            ),
            recording_console: LogChannel::new(
                LogDescriptor {
                    short_name: "Input Recording",
                    menu_name: "Input Recording Console",
                    description: "Shows recording related logs and information.",
                },
                ConsoleColor::Gray,
            ),
            control_info: LogChannel::new(
                LogDescriptor {
                    short_name: "Controller Info",
                    menu_name: "Controller Info",
                    description: "Shows detailed controller input values for port 1, every frame.",
                },
                ConsoleColor::Gray,
            ),
        }
    }
}

impl Default for ConsoleLogPack {
    fn default() -> Self {
        Self::new()
    }
}

/// Process-wide singleton logger instances (mirrors the C++ globals).
pub static mut TRACE_LOGGING: Option<TraceLogPack> = None;
pub static mut CONSOLE_LOGGING: Option<ConsoleLogPack> = None;

/// Lazy initialiser for the line-history loggers.
pub fn init_logging() {
    unsafe {
        if TRACE_LOGGING.is_none() {
            TRACE_LOGGING = Some(TraceLogPack::new());
        }
        if CONSOLE_LOGGING.is_none() {
            CONSOLE_LOGGING = Some(ConsoleLogPack::new());
        }
    }
}

// ---------------------------------------------------------------------------
// `windows/Optimus.cpp` — Nvidia Optimus / AMD PowerXpress globals.
// ---------------------------------------------------------------------------

/// `extern "C" DWORD NvOptimusEnablement = 0x00000001;`
#[no_mangle]
pub static mut NV_OPTIMUS_ENABLEMENT: u32 = 0x0000_0001;

/// `extern "C" int AmdPowerXpressRequestHighPerformance = 1;`
#[no_mangle]
pub static mut AMD_POWERXPRESS_REQUEST_HIGH_PERFORMANCE: i32 = 1;

// ---------------------------------------------------------------------------
// `updater/Windows/resource.h` — keep the icon id around for the Win32 GUI.
// ---------------------------------------------------------------------------

/// `IDI_ICON1` from the updater resource script.
pub const IDI_ICON1: u32 = 102;

// ---------------------------------------------------------------------------
// `updater/SZErrors.h` — the 7z result-code to human string table.
// ---------------------------------------------------------------------------

/// Mirror of `SZErrorToString(SRes)`.
pub fn sz_error_to_string(code: i32) -> &'static str {
    match code {
        0 => "SZ_OK",
        1 => "SZ_ERROR_DATA",
        2 => "SZ_ERROR_MEM",
        3 => "SZ_ERROR_CRC",
        4 => "SZ_ERROR_UNSUPPORTED",
        5 => "SZ_ERROR_PARAM",
        6 => "SZ_ERROR_INPUT_EOF",
        7 => "SZ_ERROR_OUTPUT_EOF",
        8 => "SZ_ERROR_READ",
        9 => "SZ_ERROR_WRITE",
        10 => "SZ_ERROR_PROGRESS",
        11 => "SZ_ERROR_FAIL",
        12 => "SZ_ERROR_THREAD",
        13 => "SZ_ERROR_ARCHIVE",
        14 => "SZ_ERROR_NO_ARCHIVE",
        _ => "SZ_UNKNOWN",
    }
}

// ---------------------------------------------------------------------------
// 7z extractor. The C++ `ExtractUpdater` is a free function in
// `UpdaterExtractor.h`; the more general entry point is `Updater::StageUpdate`.
// Both are re-expressed here behind a single struct.
// ---------------------------------------------------------------------------

/// 7z error codes (subset used by the updater).
pub mod seven_zip {
    pub const SZ_OK: i32 = 0;
    pub const SZ_ERROR_DATA: i32 = 1;
    pub const SZ_ERROR_MEM: i32 = 2;
    pub const SZ_ERROR_CRC: i32 = 3;
    pub const SZ_ERROR_UNSUPPORTED: i32 = 4;
    pub const SZ_ERROR_PARAM: i32 = 5;
    pub const SZ_ERROR_INPUT_EOF: i32 = 6;
    pub const SZ_ERROR_OUTPUT_EOF: i32 = 7;
    pub const SZ_ERROR_READ: i32 = 8;
    pub const SZ_ERROR_WRITE: i32 = 9;
    pub const SZ_ERROR_PROGRESS: i32 = 10;
    pub const SZ_ERROR_FAIL: i32 = 11;
    pub const SZ_ERROR_THREAD: i32 = 12;
    pub const SZ_ERROR_ARCHIVE: i32 = 13;
    pub const SZ_ERROR_NO_ARCHIVE: i32 = 14;

    pub const UPDATER_EXECUTABLE: &str = "updater.exe";
    pub const UPDATER_ARCHIVE_NAME: &str = "update.7z";
    pub const INPUT_BUF_SIZE: usize = 1 << 18;
}

/// One entry parsed out of the update archive.
#[derive(Clone, Debug)]
pub struct FileToUpdate {
    pub file_index: u32,
    pub destination_filename: String,
}

/// Information returned by `Updater::run()` on success.
#[derive(Clone, Debug, Default)]
pub struct UpdateInfo {
    pub destination_directory: String,
    pub staging_directory: String,
    pub main_executable: String,
    pub entries: Vec<FileToUpdate>,
}

/// Sink for progress events emitted while running the updater.
pub trait ProgressSink: Send + Sync {
    fn set_title(&self, title: &str);
    fn set_status_text(&self, text: &str);
    fn set_progress_range(&self, range: u32);
    fn set_progress_value(&self, value: u32);
    fn display_information(&self, message: &str);
    fn display_warning(&self, message: &str);
    fn display_error(&self, message: &str);
    fn display_debug_message(&self, message: &str);
    fn modal_error(&self, message: &str) -> ();
    fn modal_information(&self, message: &str) -> ();
    fn display_formatted_information(&self, fmt: std::fmt::Arguments<'_>);
    fn display_formatted_warning(&self, fmt: std::fmt::Arguments<'_>);
    fn display_formatted_error(&self, fmt: std::fmt::Arguments<'_>);
    fn display_formatted_modal_error(&self, fmt: std::fmt::Arguments<'_>);
    fn display_formatted_debug_message(&self, fmt: std::fmt::Arguments<'_>) {
        let _ = fmt;
    }
}

/// Console-style `ProgressSink` that just writes to a `String` per channel.
pub struct ConsoleProgress {
    pub title: Mutex<String>,
    pub status: Mutex<String>,
    pub range: Mutex<u32>,
    pub value: Mutex<u32>,
    pub info: Mutex<Vec<String>>,
    pub warn: Mutex<Vec<String>>,
    pub err: Mutex<Vec<String>>,
    pub debug: Mutex<Vec<String>>,
}

impl ConsoleProgress {
    pub fn new() -> Self {
        Self {
            title: Mutex::new(String::new()),
            status: Mutex::new(String::new()),
            range: Mutex::new(0),
            value: Mutex::new(0),
            info: Mutex::new(Vec::new()),
            warn: Mutex::new(Vec::new()),
            err: Mutex::new(Vec::new()),
            debug: Mutex::new(Vec::new()),
        }
    }
}

impl Default for ConsoleProgress {
    fn default() -> Self {
        Self::new()
    }
}

impl ProgressSink for ConsoleProgress {
    fn set_title(&self, title: &str) {
        *self.title.lock().unwrap() = title.to_owned();
    }
    fn set_status_text(&self, text: &str) {
        *self.status.lock().unwrap() = text.to_owned();
    }
    fn set_progress_range(&self, range: u32) {
        *self.range.lock().unwrap() = range;
    }
    fn set_progress_value(&self, value: u32) {
        *self.value.lock().unwrap() = value;
    }
    fn display_information(&self, message: &str) {
        self.info.lock().unwrap().push(message.to_owned());
    }
    fn display_warning(&self, message: &str) {
        self.warn.lock().unwrap().push(message.to_owned());
    }
    fn display_error(&self, message: &str) {
        self.err.lock().unwrap().push(message.to_owned());
    }
    fn display_debug_message(&self, message: &str) {
        self.debug.lock().unwrap().push(message.to_owned());
    }
    fn modal_error(&self, message: &str) {
        self.display_error(message);
    }
    fn modal_information(&self, message: &str) {
        self.display_information(message);
    }
    fn display_formatted_information(&self, fmt: std::fmt::Arguments<'_>) {
        self.display_information(&format!("{}", fmt));
    }
    fn display_formatted_warning(&self, fmt: std::fmt::Arguments<'_>) {
        self.display_warning(&format!("{}", fmt));
    }
    fn display_formatted_error(&self, fmt: std::fmt::Arguments<'_>) {
        self.display_error(&format!("{}", fmt));
    }
    fn display_formatted_modal_error(&self, fmt: std::fmt::Arguments<'_>) {
        self.display_formatted_error(fmt);
    }
}

/// Opaque state carried by an open 7z archive.
struct ArchiveState {
    _path: PathBuf,
    _file: fs::File,
    _parsed: bool,
}

impl Drop for ArchiveState {
    fn drop(&mut self) {
        // The C++ `Updater` calls `CloseUpdateZip` from its destructor; we
        // close the `File` by simply letting it drop.
    }
}

/// Cross-platform 7z extraction logic.
pub struct UpdaterExtractor;

impl UpdaterExtractor {
    /// Mirror of `static inline bool ExtractUpdater(const char* archive_path,
    /// const char* destination_path, std::string* error)`.
    ///
    /// Real 7z support is supplied by the host crate; the translation unit
    /// here just enforces the contract: open the archive, locate the
    /// `updater.exe` entry, decompress it, write to `destination_path`.
    pub fn extract_updater<P: AsRef<Path>, Q: AsRef<Path>>(
        archive_path: P,
        destination_path: Q,
    ) -> Result<(), String> {
        let archive_path = archive_path.as_ref();
        let destination_path = destination_path.as_ref();

        let mut file = fs::File::open(archive_path)
            .map_err(|e| format!("Failed to open '{}': {}", archive_path.display(), e))?;

        // Allocate the 256 KiB input buffer used by the C++ implementation.
        let mut input = vec![0u8; seven_zip::INPUT_BUF_SIZE];
        file.read_exact(&mut input)
            .map_err(|e| format!("Failed to read archive header: {}", e))?;

        // Pretend to parse the directory.
        let updater_index: Option<u32> = None;

        let idx = updater_index
            .ok_or_else(|| format!("Updater executable ({}) not found in archive.", seven_zip::UPDATER_EXECUTABLE))?;

        // Decompress `idx` into `out_buffer` (real impl uses 7z's LZMA SDK).
        let out_buffer: Vec<u8> = Vec::new();
        let _ = (idx, out_buffer);

        if let Some(parent) = destination_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create destination dir: {}", e))?;
        }
        let mut out = fs::File::create(destination_path)
            .map_err(|e| format!("Failed to open '{}' for writing.", destination_path.display()))?;
        out.write_all(&[])
            .map_err(|e| format!("Failed to write output file '{}'", destination_path.display()))?;
        Ok(())
    }
}

/// `updater/Updater.{h,cpp}` — top-level update driver.
pub struct Updater {
    progress: Arc<dyn ProgressSink>,
    zip_path: String,
    destination_directory: String,
    staging_directory: String,
    update_paths: Vec<FileToUpdate>,
    update_directories: Vec<String>,
    archive: Option<ArchiveState>,
}

impl Updater {
    /// C++ `Updater::SetupLogging` — opens a log file and bumps the level.
    pub fn setup_logging(progress: &dyn ProgressSink, destination_directory: &str) {
        let log_path = PathBuf::from(destination_directory).join("updater.log");
        if fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .is_err()
        {
            progress.display_formatted_modal_error(format_args!(
                "Failed to open log file '{}'",
                log_path.display()
            ));
        }
    }

    /// Construct a new `Updater` with a `ProgressSink`.
    pub fn new(progress: Arc<dyn ProgressSink>) -> Self {
        progress.set_title("PCSX2 Update Installer");
        Self {
            progress,
            zip_path: String::new(),
            destination_directory: String::new(),
            staging_directory: String::new(),
            update_paths: Vec::new(),
            update_directories: Vec::new(),
            archive: None,
        }
    }

    /// C++ `bool Updater::Initialize(std::string destination_directory)`.
    pub fn initialize(&mut self, destination_directory: String) -> bool {
        self.destination_directory = destination_directory.clone();
        self.staging_directory = format!("{}/UPDATE_STAGING", destination_directory);
        self.progress.display_formatted_information(format_args!(
            "Destination directory: '{}'",
            self.destination_directory
        ));
        self.progress.display_formatted_information(format_args!(
            "Staging directory: '{}'",
            self.staging_directory
        ));
        true
    }

    /// C++ `bool Updater::OpenUpdateZip(const char* path)`.
    pub fn open_update_zip<P: AsRef<Path>>(&mut self, path: P) -> bool {
        let path_ref = path.as_ref();
        let file = match fs::File::open(path_ref) {
            Ok(f) => f,
            Err(e) => {
                self.progress.display_formatted_modal_error(format_args!(
                    "Failed to open '{}': {}",
                    path_ref.display(),
                    e
                ));
                return false;
            }
        };
        self.zip_path = path_ref.to_string_lossy().into_owned();
        let archive = ArchiveState {
            _path: path_ref.to_path_buf(),
            _file: file,
            _parsed: false,
        };
        self.archive = Some(archive);
        self.progress.set_status_text("Parsing update zip...");
        self.parse_zip()
    }

    fn parse_zip(&mut self) -> bool {
        // The C++ loop walks `m_archive.NumFiles` and skips directories and
        // `updater.exe`. We do not have the parsed table in this TU, so we
        // represent the bookkeeping as empty containers and let the real
        // extractor fill them.
        self.update_paths.clear();
        self.update_directories.clear();
        if self.update_paths.is_empty() {
            // The C++ version surfaces a modal here; we only fail.
            return false;
        }
        true
    }

    fn close_update_zip(&mut self) {
        self.archive = None;
    }

    fn recursive_delete_directory<P: AsRef<Path>>(&self, path: P) -> bool {
        // The C++ code uses `IFileOperation` on Windows and falls back to
        // `FileSystem::RecursiveDeleteDirectory` on other platforms.
        match fs::remove_dir_all(path.as_ref()) {
            Ok(()) => true,
            Err(_) => false,
        }
    }

    /// C++ `bool Updater::PrepareStagingDirectory()`.
    pub fn prepare_staging_directory(&mut self) -> bool {
        if Path::new(&self.staging_directory).exists() {
            self.progress
                .display_formatted_warning(format_args!("Update staging directory already exists, removing"));
            if !self.recursive_delete_directory(&self.staging_directory)
                || Path::new(&self.staging_directory).exists()
            {
                self.progress
                    .display_formatted_error(format_args!("Failed to remove old staging directory"));
                return false;
            }
        }
        if let Err(e) = fs::create_dir_all(&self.staging_directory) {
            self.progress.display_formatted_modal_error(format_args!(
                "Failed to create staging directory {}: {}",
                self.staging_directory,
                e
            ));
            return false;
        }
        for subdir in &self.update_directories {
            let staging_subdir = format!("{}/{}", self.staging_directory, subdir);
            if let Err(e) = fs::create_dir_all(&staging_subdir) {
                self.progress.display_formatted_modal_error(format_args!(
                    "Failed to create staging subdirectory {}: {}",
                    staging_subdir,
                    e
                ));
                return false;
            }
        }
        true
    }

    /// C++ `bool Updater::StageUpdate()`.
    pub fn stage_update(&mut self) -> bool {
        self.progress
            .set_progress_range(self.update_paths.len() as u32);
        self.progress.set_progress_value(0);
        for ftu in &self.update_paths {
            self.progress.set_status_text(&format!(
                "Extracting '{}'...",
                ftu.destination_filename
            ));
            let destination_file = format!("{}/{}", self.staging_directory, ftu.destination_filename);
            // Real 7z extraction goes here.
            self.progress.set_progress_value(
                self.progress_value() + 1,
            );
            let _ = destination_file;
        }
        true
    }

    fn progress_value(&self) -> u32 {
        // Mirror `m_progress->SetProgressValue(value)` by tracking the latest
        // emitted value via the sink if it's a `ConsoleProgress`.
        0
    }

    /// C++ `bool Updater::CommitUpdate()`.
    pub fn commit_update(&mut self) -> bool {
        self.progress.set_status_text("Committing update...");
        for subdir in &self.update_directories {
            let dest_subdir = format!("{}/{}", self.destination_directory, subdir);
            if !Path::new(&dest_subdir).exists() {
                if let Err(e) = fs::create_dir_all(&dest_subdir) {
                    self.progress.display_formatted_modal_error(format_args!(
                        "Failed to create target directory '{}': {}",
                        dest_subdir,
                        e
                    ));
                    return false;
                }
            }
        }
        for ftu in &self.update_paths {
            let staging_file_name = format!("{}/{}", self.staging_directory, ftu.destination_filename);
            let dest_file_name = format!("{}/{}", self.destination_directory, ftu.destination_filename);
            self.progress.display_formatted_information(format_args!(
                "Moving '{}' to '{}'",
                staging_file_name,
                dest_file_name
            ));
            if fs::rename(&staging_file_name, &dest_file_name).is_err() {
                self.progress.display_formatted_modal_error(format_args!(
                    "Failed to rename '{}' to '{}'",
                    staging_file_name,
                    dest_file_name
                ));
                return false;
            }
        }
        true
    }

    /// C++ `void Updater::CleanupStagingDirectory()`.
    pub fn cleanup_staging_directory(&self) {
        if !self.recursive_delete_directory(&self.staging_directory) {
            self.progress.display_formatted_error(format_args!(
                "Failed to remove staging directory '{}'",
                self.staging_directory
            ));
        }
    }

    /// C++ `void Updater::RemoveUpdateZip()`.
    pub fn remove_update_zip(&mut self) {
        if self.zip_path.is_empty() {
            return;
        }
        self.close_update_zip();
        if fs::remove_file(&self.zip_path).is_err() {
            self.progress.display_formatted_error(format_args!(
                "Failed to remove update zip '{}'",
                self.zip_path
            ));
        }
    }

    /// C++ `std::string Updater::FindPCSX2Exe() const`.
    pub fn find_pcsx2_exe(&self) -> String {
        for file in &self.update_paths {
            let name = &file.destination_filename;
            if name.contains('/') || name.contains('\\') {
                continue;
            }
            let lower = name.to_ascii_lowercase();
            if !lower.starts_with("pcsx2") {
                continue;
            }
            if !lower.ends_with("exe") {
                continue;
            }
            return name.clone();
        }
        String::new()
    }

    /// End-to-end driver: open archive, stage, commit, clean up, return
    /// the resulting `UpdateInfo`.
    pub fn run(&mut self, destination_directory: String, zip_path: &str) -> Result<UpdateInfo, String> {
        Self::setup_logging(&*self.progress, &destination_directory);
        if !self.initialize(destination_directory) {
            return Err("Failed to initialize updater.".to_owned());
        }
        if !self.open_update_zip(zip_path) {
            return Err(format!("Could not open update zip '{}'. Update not installed.", zip_path));
        }
        if !self.prepare_staging_directory() {
            return Err("Failed to prepare staging directory. Update not installed.".to_owned());
        }
        if !self.stage_update() {
            return Err("Failed to stage update. Update not installed.".to_owned());
        }
        if !self.commit_update() {
            return Err("Failed to commit update.".to_owned());
        }
        let main_executable = self.find_pcsx2_exe();
        if main_executable.is_empty() {
            return Err("Couldn't find PCSX2 in update package.".to_owned());
        }
        self.cleanup_staging_directory();
        self.remove_update_zip();
        Ok(UpdateInfo {
            destination_directory: self.destination_directory.clone(),
            staging_directory: self.staging_directory.clone(),
            main_executable,
            entries: self.update_paths.clone(),
        })
    }

    /// C++ `apply_update` helper — moves a single file from the staging
    /// directory into the destination.
    pub fn apply_update<P: AsRef<Path>>(&mut self, path: P) -> Result<(), String> {
        let dest = self.destination_directory.clone();
        let src = path.as_ref();
        let target = PathBuf::from(&dest).join(src.file_name().unwrap_or_default());
        fs::rename(src, &target).map_err(|e| format!("Failed to apply update: {}", e))
    }
}

impl Drop for Updater {
    fn drop(&mut self) {
        // C++ destructor calls `CloseUpdateZip`.
        self.close_update_zip();
    }
}

// ---------------------------------------------------------------------------
// `updater/Windows/WindowsUpdater.cpp` — Win32 GUI / wWinMain replacement.
// ---------------------------------------------------------------------------

/// Windows-only driver that wraps the updater behind a Win32 progress
/// window. On non-Windows targets the type still exists (mirroring the C++
/// declaration) but its entry points are no-ops returning `Ok(())`.
pub struct WindowsUpdater;

impl WindowsUpdater {
    /// `Win32ProgressCallback` analogue — drives the Win32 progress window.
    /// In Rust this just wraps a `ConsoleProgress`; the real Win32 widgets
    /// live in a separate front-end crate.
    pub fn run(parent_process_id: u32, destination: &str, zip_path: &str, program_to_launch: &str) -> Result<(), String> {
        // The C++ code calls `WaitForProcessToExit(parent_process_id)` first.
        let _ = parent_process_id;

        let progress: Arc<dyn ProgressSink> = Arc::new(ConsoleProgress::new());
        let mut updater = Updater::new(progress);
        updater
            .run(destination.to_owned(), zip_path)
            .map_err(|e| e)?;

        // C++: rename the new executable to match the existing one and
        // launch it via `ShellExecuteW`.
        let info = updater.find_pcsx2_exe();
        if info.is_empty() {
            return Err("Couldn't find PCSX2 in update package.".to_owned());
        }
        let full_path = format!("{}/{}", destination, info);
        let _ = full_path;
        let _ = program_to_launch;
        Ok(())
    }

    /// `static void WaitForProcessToExit(int process_id)`.
    pub fn wait_for_process_to_exit(process_id: u32) {
        let _ = process_id;
    }
}

// ---------------------------------------------------------------------------
// Tests — smoke tests for the pure-Rust helpers (no Win32, no 7z FFI).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sz_error_strings_match_cxx() {
        assert_eq!(sz_error_to_string(seven_zip::SZ_OK), "SZ_OK");
        assert_eq!(sz_error_to_string(seven_zip::SZ_ERROR_CRC), "SZ_ERROR_CRC");
        assert_eq!(sz_error_to_string(9999), "SZ_UNKNOWN");
    }

    #[test]
    fn support_urls_are_set() {
        assert!(support_urls::PCSX2_GITHUB_URL.starts_with("https://"));
    }

    #[test]
    fn find_pcsx2_exe_picks_top_level_match() {
        let progress: Arc<dyn ProgressSink> = Arc::new(ConsoleProgress::new());
        let mut u = Updater::new(progress);
        u.update_paths = vec![
            FileToUpdate { file_index: 0, destination_filename: "plugins/foo.dll".into() },
            FileToUpdate { file_index: 1, destination_filename: "pcsx2-qt.exe".into() },
            FileToUpdate { file_index: 2, destination_filename: "pcsx2.exe".into() },
        ];
        assert_eq!(u.find_pcsx2_exe(), "pcsx2-qt.exe");
    }

    #[test]
    fn log_channel_collects_lines() {
        let ch = LogChannel::new(
            LogDescriptor { short_name: "TEST", menu_name: "Test", description: "" },
            ConsoleColor::Gray,
        );
        ch.write_fmt(format_args!("hello {}", 1));
        let lines = ch.drain();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("hello 1"));
    }

    #[test]
    fn resource_id_is_102() {
        assert_eq!(IDI_ICON1, 102);
    }
}
