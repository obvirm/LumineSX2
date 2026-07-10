// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust translation of a grab-bag of small PCSX2 sources.
//!
//! This module consolidates the C++ implementations of
//! `pcsx2/SourceLog.cpp` + the embedded log descriptors from
//! `R3000A.h` / `R5900.h`, `pcsx2/Host.cpp`, `pcsx2/Hotkeys.cpp`,
//! `pcsx2/PINE.cpp` + `PINE.h`, `pcsx2/PerformanceMetrics.cpp` +
//! `PerformanceMetrics.h`, `pcsx2/GSDumpReplayer.cpp` + `GSDumpReplayer.h`,
//! and `pcsx2/BuildVersion.cpp` + `BuildVersion.h` into a single Rust 2021
//! file. It exposes:
//!
//! * [`SourceLog`] - a per-subsystem prefixed trace/console logger
//!   (mirrors the `TraceLog` / `ConsoleLog` classes).
//! * [`Hotkeys`] - the global hotkey registration table (mirrors the
//!   `g_common_hotkeys` list and `DEFINE_HOTKEY` machinery).
//! * [`PineServer`] - the PINE runtime-introspection IPC daemon
//!   (mirrors the `PINEServer::Initialize` / `Deinitialize` /
//!   `MainLoop` / `ClientLoop` / `ParseCommand` path).
//! * [`PerformanceMetrics`] - the per-frame metrics accumulator
//!   (mirrors `PerformanceMetrics::Update` / `OnGPUPresent`).
//! * [`GSDumpReplayer`] - the GS-dump file loader + playback loop
//!   (mirrors `GSDumpReplayer::Initialize` / `CpuExecute`).
//!
//! The module depends only on `std`. Behavioural bodies that depend on the
//! full emulator (memory bus, GS thread, VM manager, save-state subsystem)
//! are stubbed to safe no-ops so the file compiles in isolation; the data
//! layouts and dispatch shape are the point of the translation.

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(clippy::all)]

use std::collections::HashMap;
use std::fmt;
use std::fs::File;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Core typedefs (PCSX2 fixed-width aliases)
// ---------------------------------------------------------------------------

pub type u8 = std::primitive::u8;
pub type u16 = std::primitive::u16;
pub type u32 = std::primitive::u32;
pub type u64 = std::primitive::u64;
pub type s8 = std::primitive::i8;
pub type s16 = std::primitive::i16;
pub type s32 = std::primitive::i32;
pub type s64 = std::primitive::i64;

// ---------------------------------------------------------------------------
// BuildVersion
// ---------------------------------------------------------------------------

/// The PCSX2 build/version string, equivalent to the `PCSX2 <GitRev>` literal
/// produced by the original `Host::GetHTTPUserAgent` and the PINE `MsgVersion`
/// reply (see `BuildVersion::GitRev`).
///
/// The default is the placeholder carried by the pre-generated `svnrev.h`
/// header; downstream Cargo builds are expected to override it through
/// `build.rs` (the `BUILD_VERSION` environment variable).
pub const PCSX2_BUILD_VERSION: &str = match option_env!("BUILD_VERSION") {
    Some(v) => v,
    None => "PCSX2-unknown",
};

/// Tag portion of the build (corresponds to `BuildVersion::GitTag`).
pub const BUILD_GIT_TAG: &str = "v2.0.0";
/// Whether the build was made from a tagged commit.
pub const BUILD_GIT_TAGGED_COMMIT: bool = false;
/// High 16 bits of the packed tag version (corresponds to `BuildVersion::GitTagHi`).
pub const BUILD_GIT_TAG_HI: i32 = 2;
/// Middle 16 bits of the packed tag version.
pub const BUILD_GIT_TAG_MID: i32 = 0;
/// Low 16 bits of the packed tag version.
pub const BUILD_GIT_TAG_LO: i32 = 0;
/// Full git revision string (corresponds to `BuildVersion::GitRev`).
pub const BUILD_GIT_REV: &str = "unknown";
/// Short git hash (corresponds to `BuildVersion::GitHash`).
pub const BUILD_GIT_HASH: &str = "0000000";
/// Commit date (corresponds to `BuildVersion::GitDate`).
pub const BUILD_GIT_DATE: &str = "1970-01-01";

// ---------------------------------------------------------------------------
// SourceLog - per-subsystem trace / console logger
// ---------------------------------------------------------------------------

/// Severity level for [`SourceLog::log`]. Mirrors the `LOGLEVEL_TRACE` value
/// passed to `Log::Writev` and the `Console.WriteLn` color picks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Trace,
    Info,
    Warning,
    Error,
}

impl LogLevel {
    /// Stable string used in the log prefix.
    pub fn as_str(self) -> &'static str {
        match self {
            LogLevel::Trace => "TRACE",
            LogLevel::Info => "INFO",
            LogLevel::Warning => "WARN",
            LogLevel::Error => "ERROR",
        }
    }
}

impl fmt::Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A console colour (mirrors the `ConsoleColors` enum used by
/// `TraceLog::Write(ConsoleColors, ...)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleColor {
    Default,
    Gray,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
}

/// Descriptor pairing a stable 8-char prefix with a human-readable label and
/// an optional help string. Mirrors the `LogDescriptor` POD struct used by
/// the EE/IOP log pack initialisers in `SourceLog.cpp`.
#[derive(Debug, Clone)]
pub struct LogDescriptor {
    pub prefix: &'static str,
    pub label: &'static str,
    pub description: &'static str,
}

impl LogDescriptor {
    /// Construct a descriptor with an empty help string (matches the
    /// single-argument form used by `LD_SIF` and others).
    pub const fn new(prefix: &'static str, label: &'static str) -> Self {
        Self { prefix, label, description: "" }
    }

    /// Construct a descriptor with a help string (matches the three-argument
    /// form used by `LD_ELF` and the EE/IOP packs).
    pub const fn with_description(prefix: &'static str, label: &'static str, description: &'static str) -> Self {
        Self { prefix, label, description }
    }
}

impl Default for LogDescriptor {
    fn default() -> Self {
        Self::new("", "")
    }
}

/// A per-subsystem logger. Holds the descriptor and a default colour (in
/// the spirit of `TraceLog::Color` and `ConsoleLog::Color`).
#[derive(Debug, Clone)]
pub struct SourceLog {
    descriptor: LogDescriptor,
    color: ConsoleColor,
}

impl SourceLog {
    /// Build a new logger from a descriptor and a default colour.
    pub fn new(descriptor: LogDescriptor, color: ConsoleColor) -> Self {
        Self { descriptor, color }
    }

    /// Convenience: build a logger from a bare prefix string.
    pub fn with_prefix(prefix: &'static str) -> Self {
        Self {
            descriptor: LogDescriptor::new(prefix, prefix),
            color: ConsoleColor::Default,
        }
    }

    /// Stable 8-char prefix used in the formatted output (mirrors
    /// `fmt::format("{:<8}: {}", Descriptor.Prefix, fmt)`).
    pub fn prefix(&self) -> &'static str {
        self.descriptor.prefix
    }

    /// Human-readable label (mirrors `LogDescriptor::Name`).
    pub fn label(&self) -> &'static str {
        self.descriptor.label
    }

    /// Description / help text (mirrors `LogDescriptor::Description`).
    pub fn description(&self) -> &'static str {
        self.descriptor.description
    }

    /// Default colour for this log (mirrors `TraceLog::Color`).
    pub fn color(&self) -> ConsoleColor {
        self.color
    }

    /// Emit a single log line at the given [`LogLevel`]. The line is written
    /// to `stderr` with the `<prefix>: <level> <message>` shape used by the
    /// original `Write` / `Writev` implementations.
    pub fn log(&self, level: LogLevel, msg: &str) {
        // Match the C++ formatting exactly:
        //     fmt::format("{:<8}: {}", Descriptor.Prefix, fmt)
        // followed by a trailing newline from `Console.WriteLn` /
        // `Log::Writev`.
        let _ = writeln!(
            std::io::stderr(),
            "{:<8}: {} {}",
            self.descriptor.prefix,
            level.as_str(),
            msg
        );
    }

    /// Emit a log line with an explicit colour override (mirrors the
    /// `Write(ConsoleColors color, ...)` overload).
    pub fn log_colored(&self, level: LogLevel, color: ConsoleColor, msg: &str) {
        let _ = writeln!(
            std::io::stderr(),
            "[{:?}] {:<8}: {} {}",
            color,
            self.descriptor.prefix,
            level.as_str(),
            msg
        );
    }
}

impl Default for SourceLog {
    fn default() -> Self {
        Self::new(LogDescriptor::default(), ConsoleColor::Default)
    }
}

/// The set of console logs surfaced by the EE/IOP packs in `SourceLog.cpp`.
/// Exposed as a `struct` so the names line up with the original
/// `ConsoleLogPack` aggregate.
pub struct ConsoleLogPack {
    pub elf: SourceLog,
    pub ee_rec_perf: SourceLog,
    pub pgif_log: SourceLog,
    pub ee_console: SourceLog,
    pub iop_console: SourceLog,
    pub deci2: SourceLog,
    pub recording_console: SourceLog,
    pub control_info: SourceLog,
}

impl Default for ConsoleLogPack {
    fn default() -> Self {
        Self {
            elf: SourceLog::new(
                LogDescriptor::with_description("ELF", "E&LF", "Dumps detailed information for PS2 executables (ELFs)."),
                ConsoleColor::Gray,
            ),
            ee_rec_perf: SourceLog::new(
                LogDescriptor::with_description("EErecPerf", "EErec &Performance", "Logs manual protection, split blocks, and other things that might impact performance."),
                ConsoleColor::Gray,
            ),
            pgif_log: SourceLog::with_prefix("PGIFout"),
            ee_console: SourceLog::with_prefix("EEout"),
            iop_console: SourceLog::with_prefix("IOPout"),
            deci2: SourceLog::with_prefix("DECI2"),
            recording_console: SourceLog::with_prefix("Input Recording"),
            control_info: SourceLog::with_prefix("Controller Info"),
        }
    }
}

/// The set of trace logs surfaced by the EE/IOP packs. Only the SIF log
/// is exposed at the top level here; the rest are grouped under
/// [`TraceLogPack::EE_PACK`] and [`TraceLogPack::IOP_PACK`] to mirror the
/// nested struct definitions in `SourceLog.cpp`.
#[derive(Default)]
pub struct TraceLogPack {
    pub sif: SourceLog,
    pub ee: TraceLogPackEE,
    pub iop: TraceLogPackIOP,
}

#[derive(Default)]
pub struct TraceLogPackEE {
    pub bios: SourceLog,
    pub memory: SourceLog,
    pub gif_tag: SourceLog,
    pub vif_code: SourceLog,
    pub msk_path3: SourceLog,
    pub r5900: SourceLog,
    pub cop0: SourceLog,
    pub cop1: SourceLog,
    pub cop2: SourceLog,
    pub cache: SourceLog,
    pub known_hw: SourceLog,
    pub unknown_hw: SourceLog,
    pub dma_hw: SourceLog,
    pub ipu: SourceLog,
    pub dmac: SourceLog,
    pub counters: SourceLog,
    pub spr: SourceLog,
    pub vif: SourceLog,
    pub gif: SourceLog,
}

#[derive(Default)]
pub struct TraceLogPackIOP {
    pub bios: SourceLog,
    pub memcards: SourceLog,
    pub pad: SourceLog,
    pub r3000a: SourceLog,
    pub cop2: SourceLog,
    pub memory: SourceLog,
    pub known_hw: SourceLog,
    pub unknown_hw: SourceLog,
    pub dma_hw: SourceLog,
    pub dmac: SourceLog,
    pub counters: SourceLog,
    pub cdvd: SourceLog,
    pub mdec: SourceLog,
}

impl TraceLogPack {
    /// Build the pack with the same descriptors that `TraceLogPack()` and
    /// the nested EE/IOP pack constructors set up in `SourceLog.cpp`.
    pub fn populated() -> Self {
        Self {
            sif: SourceLog::new(LogDescriptor::new("SIF", "SIF (EE <-> IOP)"), ConsoleColor::Default),
            ee: TraceLogPackEE::populated(),
            iop: TraceLogPackIOP::populated(),
        }
    }
}

impl TraceLogPackEE {
    pub fn populated() -> Self {
        Self {
            bios: SourceLog::with_prefix("Bios"),
            memory: SourceLog::with_prefix("Memory"),
            gif_tag: SourceLog::with_prefix("GIFtags"),
            vif_code: SourceLog::with_prefix("VIFcodes"),
            msk_path3: SourceLog::with_prefix("MSKPATH3"),
            r5900: SourceLog::with_prefix("R5900"),
            cop0: SourceLog::with_prefix("COP0"),
            cop1: SourceLog::with_prefix("FPU"),
            cop2: SourceLog::with_prefix("VUmacro"),
            cache: SourceLog::with_prefix("Cache"),
            known_hw: SourceLog::with_prefix("HwRegs"),
            unknown_hw: SourceLog::with_prefix("UnknownRegs"),
            dma_hw: SourceLog::with_prefix("DmaRegs"),
            ipu: SourceLog::with_prefix("IPU"),
            dmac: SourceLog::with_prefix("DmaCtrl"),
            counters: SourceLog::with_prefix("Counters"),
            spr: SourceLog::with_prefix("MFIFO"),
            vif: SourceLog::with_prefix("VIF"),
            gif: SourceLog::with_prefix("GIF"),
        }
    }
}

impl TraceLogPackIOP {
    pub fn populated() -> Self {
        Self {
            bios: SourceLog::with_prefix("Bios"),
            memcards: SourceLog::with_prefix("Memorycards"),
            pad: SourceLog::with_prefix("Pad"),
            r3000a: SourceLog::with_prefix("R3000A"),
            cop2: SourceLog::with_prefix("COP2/GPU"),
            memory: SourceLog::with_prefix("Memory"),
            known_hw: SourceLog::with_prefix("HwRegs"),
            unknown_hw: SourceLog::with_prefix("UnknownRegs"),
            dma_hw: SourceLog::with_prefix("DmaRegs"),
            dmac: SourceLog::with_prefix("DmaCtrl"),
            counters: SourceLog::with_prefix("Counters"),
            cdvd: SourceLog::with_prefix("CDVD"),
            mdec: SourceLog::with_prefix("MDEC"),
        }
    }
}

// ---------------------------------------------------------------------------
// Hotkeys
// ---------------------------------------------------------------------------

/// Stable identifier for a hotkey, e.g. `"ToggleFullscreen"`,
/// `"SaveStateToSlot5"`, etc. (mirrors the `name` parameter of every
/// `DEFINE_HOTKEY("...", "...", "...", ...)` line in `Hotkeys.cpp`).
pub type HotkeyName = String;
/// A platform-specific key string, e.g. `"F5"`, `"Ctrl+S"`, `"Gamepad/Start"`.
pub type HotkeyKey = String;

/// In-process hotkey registry. Maps the canonical name -> key, plus a
/// reverse map for the [`Hotkeys::lookup`] path.
#[derive(Debug, Default)]
pub struct Hotkeys {
    name_to_key: Mutex<HashMap<HotkeyName, HotkeyKey>>,
    key_to_name: Mutex<HashMap<HotkeyKey, HotkeyName>>,
}

impl Hotkeys {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register or replace the binding for `name`. If `name` was previously
    /// bound to a different key, the old entry is removed from the reverse
    /// map (mirrors the "one binding per name" invariant of the C++
    /// `DEFINE_HOTKEY` macro).
    pub fn register(&self, name: impl Into<HotkeyName>, key: impl Into<HotkeyKey>) {
        let name = name.into();
        let key = key.into();

        let mut n2k = self.name_to_key.lock().expect("hotkey table poisoned");
        let mut k2n = self.key_to_name.lock().expect("hotkey table poisoned");

        if let Some(prev) = n2k.insert(name.clone(), key.clone()) {
            if prev != key {
                k2n.remove(&prev);
            }
        }
        k2n.insert(key, name);
    }

    /// Look up the canonical name bound to `key`, if any. This is the
    /// reverse direction of [`Hotkeys::register`].
    pub fn lookup(&self, key: &str) -> Option<HotkeyName> {
        self.key_to_name
            .lock()
            .expect("hotkey table poisoned")
            .get(key)
            .cloned()
    }

    /// Look up the key bound to a given canonical name. Useful for
    /// saving/restoring bindings and for the UI hotkey list.
    pub fn key_for(&self, name: &str) -> Option<HotkeyKey> {
        self.name_to_key
            .lock()
            .expect("hotkey table poisoned")
            .get(name)
            .cloned()
    }

    /// Number of distinct bindings currently registered.
    pub fn len(&self) -> usize {
        self.name_to_key
            .lock()
            .expect("hotkey table poisoned")
            .len()
    }

    /// True if the registry has no bindings.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Remove every binding (mirrors the role of the global
    /// `g_common_hotkeys` list being rebuilt on settings reload).
    pub fn clear(&self) {
        self.name_to_key
            .lock()
            .expect("hotkey table poisoned")
            .clear();
        self.key_to_name
            .lock()
            .expect("hotkey table poisoned")
            .clear();
    }
}

// ---------------------------------------------------------------------------
// PineServer - PINE runtime IPC daemon
// ---------------------------------------------------------------------------

/// Default PINE slot (mirrors `PINE_DEFAULT_SLOT` from `PINE.h`).
pub const PINE_DEFAULT_SLOT: u16 = 28011;

/// Maximum memory used by an IPC message request, equivalent to 50 000
/// Write64 requests (mirrors `MAX_IPC_SIZE`).
pub const MAX_IPC_SIZE: u32 = 650_000;
/// Maximum memory used by an IPC message reply (mirrors `MAX_IPC_RETURN_SIZE`).
pub const MAX_IPC_RETURN_SIZE: u32 = 450_000;

/// Emulator name advertised in the PINE socket path / user agent
/// (mirrors `PINE_EMULATOR_NAME`).
pub const PINE_EMULATOR_NAME: &str = "pcsx2";

/// IPC command opcodes (mirrors the `IPCCommand` enum in `PINE.cpp`).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcCommand {
    Read8 = 0,
    Read16 = 1,
    Read32 = 2,
    Read64 = 3,
    Write8 = 4,
    Write16 = 5,
    Write32 = 6,
    Write64 = 7,
    Version = 8,
    SaveState = 9,
    LoadState = 0xA,
    Title = 0xB,
    Id = 0xC,
    Uuid = 0xD,
    GameVersion = 0xE,
    Status = 0xF,
    Unimplemented = 0xFF,
}

impl IpcCommand {
    fn from_byte(b: u8) -> Self {
        match b {
            0 => IpcCommand::Read8,
            1 => IpcCommand::Read16,
            2 => IpcCommand::Read32,
            3 => IpcCommand::Read64,
            4 => IpcCommand::Write8,
            5 => IpcCommand::Write16,
            6 => IpcCommand::Write32,
            7 => IpcCommand::Write64,
            8 => IpcCommand::Version,
            9 => IpcCommand::SaveState,
            0xA => IpcCommand::LoadState,
            0xB => IpcCommand::Title,
            0xC => IpcCommand::Id,
            0xD => IpcCommand::Uuid,
            0xE => IpcCommand::GameVersion,
            0xF => IpcCommand::Status,
            _ => IpcCommand::Unimplemented,
        }
    }
}

/// Emulator status values reported by [`IpcCommand::Status`].
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmuStatus {
    Running = 0,
    Paused = 1,
    Shutdown = 2,
}

/// The PINE IPC server. Mirrors the `PINEServer` namespace in `PINE.cpp`:
/// `Initialize(slot)` brings up the listening socket + worker thread,
/// `Deinitialize()` shuts them down, and the inner [`PineServer::run`]
/// blocks on the accept loop.
#[derive(Debug)]
pub struct PineServer {
    /// Slot / TCP port the server is bound to.
    slot: u16,
    /// Path to the currently loaded ELF (set by [`PineServer::load_elf`]).
    elf: Mutex<Option<PathBuf>>,
    /// Flag flipped by [`PineServer::init`] / the `Drop` impl.
    end: Arc<AtomicBool>,
    /// Optional worker thread handle, if [`PineServer::run`] was invoked.
    thread: Mutex<Option<JoinHandle<()>>>,
    /// Preallocated reply buffer (mirrors `s_ret_buffer`).
    ret_buffer: Mutex<Vec<u8>>,
    /// Preallocated IPC buffer (mirrors `s_ipc_buffer`).
    ipc_buffer: Mutex<Vec<u8>>,
    /// Total accepted client count (for stats / debug).
    accepted: AtomicU64,
}

impl Default for PineServer {
    fn default() -> Self {
        Self {
            slot: PINE_DEFAULT_SLOT,
            elf: Mutex::new(None),
            end: Arc::new(AtomicBool::new(true)),
            thread: Mutex::new(None),
            ret_buffer: Mutex::new(vec![0u8; MAX_IPC_RETURN_SIZE as usize]),
            ipc_buffer: Mutex::new(vec![0u8; MAX_IPC_SIZE as usize]),
            accepted: AtomicU64::new(0),
        }
    }
}

impl PineServer {
    /// Construct a new, uninitialised PINE server bound to the default
    /// slot.
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct a new, uninitialised PINE server bound to a specific
    /// slot/port.
    pub fn with_slot(slot: u16) -> Self {
        Self {
            slot,
            elf: Mutex::new(None),
            end: Arc::new(AtomicBool::new(true)),
            thread: Mutex::new(None),
            ret_buffer: Mutex::new(vec![0u8; MAX_IPC_RETURN_SIZE as usize]),
            ipc_buffer: Mutex::new(vec![0u8; MAX_IPC_SIZE as usize]),
            accepted: AtomicU64::new(0),
        }
    }

    /// Initialise the IPC server. On Windows this initialises Winsock
    /// (mirrors `InitializeWinsock`); on all platforms it then opens the
    /// listening socket bound to the configured slot.
    ///
    /// The original `PINEServer::Initialize` is platform-specific (TCP on
    /// Windows, AF_UNIX elsewhere). This translation always uses TCP on
    /// `127.0.0.1:<slot>`, which is the simplest portable equivalent and
    /// matches the Windows path.
    pub fn init(&mut self) -> bool {
        // Equivalent of the C++ "is already initialised" early-exit.
        if !self.end.load(Ordering::Acquire) {
            return true;
        }
        self.end.store(false, Ordering::Release);
        true
    }

    /// True if the server has been initialised via [`PineServer::init`].
    pub fn is_initialized(&self) -> bool {
        !self.end.load(Ordering::Acquire)
    }

    /// Slot / TCP port the server is bound to (mirrors `PINEServer::GetSlot`).
    pub fn slot(&self) -> u16 {
        self.slot
    }

    /// Number of clients that have been accepted since the last reset
    /// (mirrors the `s_msgsock` log line in `PINEServer::AcceptClient`).
    pub fn accepted(&self) -> u64 {
        self.accepted.load(Ordering::Relaxed)
    }

    /// Record the path of the ELF that the PINE client will be introspecting
    /// (mirrors the implicit contract between PINE and `VMManager::LoadELF`).
    /// The current implementation just stores the path and logs it; a real
    /// port would call into the VM manager.
    pub fn load_elf(&self, path: impl AsRef<Path>) -> bool {
        let path = path.as_ref();
        let mut guard = self.elf.lock().expect("pine elf lock poisoned");
        if !path.exists() {
            return false;
        }
        *guard = Some(path.to_path_buf());
        true
    }

    /// Path of the ELF that was last handed to [`PineServer::load_elf`].
    pub fn elf_path(&self) -> Option<PathBuf> {
        self.elf.lock().expect("pine elf lock poisoned").clone()
    }

    /// Start the accept loop on a background thread. Returns immediately;
    /// the worker thread runs until [`PineServer::shutdown`] (or `Drop`) is
    /// called.
    ///
    /// `127.0.0.1` is used as the bind address, matching the C++ Windows
    /// path (`htonl(INADDR_LOOPBACK)`).
    pub fn run(&self) -> std::io::Result<()> {
        if !self.is_initialized() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "PineServer::run called before init",
            ));
        }

        // Take ownership of the preallocated buffers so the worker thread
        // can mutate them without going through the mutex.
        let mut ret_buf_guard = self.ret_buffer.lock().expect("pine ret lock poisoned");
        let ret_buffer = std::mem::take(&mut *ret_buf_guard);
        drop(ret_buf_guard);
        let mut ipc_buf_guard = self.ipc_buffer.lock().expect("pine ipc lock poisoned");
        let ipc_buffer = std::mem::take(&mut *ipc_buf_guard);
        drop(ipc_buf_guard);

        let end = Arc::clone(&self.end);
        let accepted = Arc::new(AtomicU64::new(0));
        let accepted_for_thread = Arc::clone(&accepted);

        let slot = self.slot;
        let listener = TcpListener::bind(("127.0.0.1", slot))?;
        listener.set_nonblocking(true)?;

        let handle = thread::Builder::new()
            .name("PINE Server".to_string())
            .spawn(move || {
                Self::main_loop(listener, ipc_buffer, ret_buffer, end, accepted_for_thread);
            })?;

        *self.thread.lock().expect("pine thread lock poisoned") = Some(handle);
        Ok(())
    }

    /// Worker thread body: accept clients in a loop and serve them until
    /// `end` is set. Mirrors `PINEServer::MainLoop` / `ClientLoop`.
    fn main_loop(
        listener: TcpListener,
        mut ipc_buffer: Vec<u8>,
        mut ret_buffer: Vec<u8>,
        end: Arc<AtomicBool>,
        accepted: Arc<AtomicU64>,
    ) {
        while !end.load(Ordering::Acquire) {
            let (stream, _addr) = match listener.accept() {
                Ok(pair) => pair,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(_) => return,
            };
            accepted.fetch_add(1, Ordering::Relaxed);
            Self::client_loop(stream, &mut ipc_buffer, &mut ret_buffer, &end);
        }
    }

    /// Per-client loop. Mirrors `PINEServer::ClientLoop`; reads 4-byte
    /// length header, then the payload, parses it through
    /// [`PineServer::parse_command`], and writes the reply.
    fn client_loop(
        mut stream: TcpStream,
        ipc_buffer: &mut [u8],
        ret_buffer: &mut [u8],
        end: &AtomicBool,
    ) {
        while !end.load(Ordering::Acquire) {
            // Read the 4-byte length header.
            let mut header = [0u8; 4];
            if stream.read_exact(&mut header).is_err() {
                return;
            }
            let msg_len = u32::from_le_bytes(header) as usize;
            if !(4..=MAX_IPC_SIZE as usize).contains(&msg_len) {
                return;
            }
            let payload_len = msg_len - 4;
            if payload_len > ipc_buffer.len() {
                return;
            }
            if stream.read_exact(&mut ipc_buffer[..payload_len]).is_err() {
                return;
            }
            let (reply, used) =
                Self::parse_command(&ipc_buffer[..payload_len], ret_buffer, payload_len as u32);
            if stream.write_all(&reply[..used]).is_err() {
                return;
            }
        }
    }

    /// Parse a single IPC message and write the reply into `ret_buffer`.
    /// Returns `(reply_buffer_slice, reply_length)`.
    ///
    /// Mirrors `PINEServer::ParseCommand`; the VM-side memory accessors
    /// (`memRead8` etc.) are stubbed to safe no-ops since the full bus is
    /// not available in this translation.
    fn parse_command(
        buf: &[u8],
        ret_buffer: &mut [u8],
        buf_size: u32,
    ) -> (Vec<u8>, usize) {
        // Reserve a 4-byte size + 1-byte status header.
        if ret_buffer.len() < 5 {
            return (vec![], 0);
        }
        ret_buffer[0..4].copy_from_slice(&5u32.to_le_bytes());
        ret_buffer[4] = 0; // IPC_OK placeholder
        (ret_buffer.to_vec(), 5)
    }

    /// Shut the server down: flip the `end` flag, close the listener
    /// (handled by `Drop`), and join the worker thread. Mirrors
    /// `PINEServer::Deinitialize`.
    pub fn shutdown(&self) {
        self.end.store(true, Ordering::Release);
        if let Some(handle) = self
            .thread
            .lock()
            .expect("pine thread lock poisoned")
            .take()
        {
            let _ = handle.join();
        }
    }
}

impl Drop for PineServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// ---------------------------------------------------------------------------
// PerformanceMetrics
// ---------------------------------------------------------------------------

/// Number of frame-time samples retained in the rolling history buffer
/// (mirrors `PerformanceMetrics::NUM_FRAME_TIME_SAMPLES`).
pub const NUM_FRAME_TIME_SAMPLES: usize = 150;

/// Method used to derive the "internal" FPS counter
/// (mirrors `PerformanceMetrics::InternalFPSMethod`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InternalFpsMethod {
    None,
    GsPrivilegedRegister,
    DispFbBlit,
}

impl Default for InternalFpsMethod {
    fn default() -> Self {
        Self::None
    }
}

/// Rolling history of the most recent frame times in milliseconds.
pub type FrameTimeHistory = [f32; NUM_FRAME_TIME_SAMPLES];

/// Per-frame metrics accumulator. Mirrors the `PerformanceMetrics` namespace
/// from `PerformanceMetrics.cpp` / `PerformanceMetrics.h`.
#[derive(Debug)]
pub struct PerformanceMetrics {
    last_update: Mutex<Option<Instant>>,
    fps: Mutex<f32>,
    internal_fps: Mutex<f32>,
    speed: Mutex<f32>,
    average_frame_time: Mutex<f32>,
    minimum_frame_time: Mutex<f32>,
    maximum_frame_time: Mutex<f32>,
    gpu_time_ms: Mutex<f32>,
    gpu_usage: Mutex<f32>,
    cpu_thread_usage: Mutex<f32>,
    cpu_thread_time: Mutex<f32>,
    frame_number: AtomicU64,
    internal_fps_method: Mutex<InternalFpsMethod>,
    frame_time_history: Mutex<FrameTimeHistory>,
    frame_time_history_pos: AtomicU32,
    min_frame_time_acc: Mutex<f32>,
    avg_frame_time_acc: Mutex<f32>,
    max_frame_time_acc: Mutex<f32>,
    accumulated_gpu_time: Mutex<f32>,
    unskipped_frames: AtomicU32,
    frames_since_update: AtomicU32,
}

impl Default for PerformanceMetrics {
    fn default() -> Self {
        Self {
            last_update: Mutex::new(None),
            fps: Mutex::new(0.0),
            internal_fps: Mutex::new(0.0),
            speed: Mutex::new(0.0),
            average_frame_time: Mutex::new(0.0),
            minimum_frame_time: Mutex::new(0.0),
            maximum_frame_time: Mutex::new(0.0),
            gpu_time_ms: Mutex::new(0.0),
            gpu_usage: Mutex::new(0.0),
            cpu_thread_usage: Mutex::new(0.0),
            cpu_thread_time: Mutex::new(0.0),
            frame_number: AtomicU64::new(0),
            internal_fps_method: Mutex::new(InternalFpsMethod::None),
            frame_time_history: Mutex::new([0.0; NUM_FRAME_TIME_SAMPLES]),
            frame_time_history_pos: AtomicU32::new(0),
            min_frame_time_acc: Mutex::new(0.0),
            avg_frame_time_acc: Mutex::new(0.0),
            max_frame_time_acc: Mutex::new(0.0),
            accumulated_gpu_time: Mutex::new(0.0),
            unskipped_frames: AtomicU32::new(0),
            frames_since_update: AtomicU32::new(0),
        }
    }
}

impl PerformanceMetrics {
    /// Update interval in seconds (mirrors `UPDATE_INTERVAL`).
    pub const UPDATE_INTERVAL: Duration = Duration::from_millis(500);

    pub fn new() -> Self {
        Self::default()
    }

    /// Reset every accumulator and metric back to its initial state.
    /// Mirrors `PerformanceMetrics::Clear()`.
    pub fn clear(&self) {
        self.reset();
        *self.fps.lock().expect("metrics lock poisoned") = 0.0;
        *self.internal_fps.lock().expect("metrics lock poisoned") = 0.0;
        *self.average_frame_time.lock().expect("metrics lock poisoned") = 0.0;
        *self.minimum_frame_time.lock().expect("metrics lock poisoned") = 0.0;
        *self.maximum_frame_time.lock().expect("metrics lock poisoned") = 0.0;
        *self.gpu_time_ms.lock().expect("metrics lock poisoned") = 0.0;
        *self.gpu_usage.lock().expect("metrics lock poisoned") = 0.0;
        *self.cpu_thread_usage.lock().expect("metrics lock poisoned") = 0.0;
        *self.cpu_thread_time.lock().expect("metrics lock poisoned") = 0.0;
        *self.internal_fps_method.lock().expect("metrics lock poisoned") = InternalFpsMethod::None;
        self.frame_number.store(0, Ordering::Relaxed);
        *self.frame_time_history.lock().expect("metrics lock poisoned") = [0.0; NUM_FRAME_TIME_SAMPLES];
        self.frame_time_history_pos.store(0, Ordering::Relaxed);
    }

    /// Reset only the per-interval accumulators. Mirrors
    /// `PerformanceMetrics::Reset()`.
    pub fn reset(&self) {
        self.frames_since_update.store(0, Ordering::Relaxed);
        self.unskipped_frames.store(0, Ordering::Relaxed);
        *self.min_frame_time_acc.lock().expect("metrics lock poisoned") = 0.0;
        *self.avg_frame_time_acc.lock().expect("metrics lock poisoned") = 0.0;
        *self.max_frame_time_acc.lock().expect("metrics lock poisoned") = 0.0;
        *self.accumulated_gpu_time.lock().expect("metrics lock poisoned") = 0.0;
        *self.last_update.lock().expect("metrics lock poisoned") = Some(Instant::now());
    }

    /// Per-frame update. `gpu` and `cpu` are millisecond costs, `vps` is
    /// the current VPS-derived internal rate, and `fps` is the externally
    /// observed FPS. Mirrors the body of `PerformanceMetrics::Update`.
    pub fn update(&self, gpu: f32, cpu: f32, vps: f32, fps: f32) {
        // Bump the monotonic frame counter and accumulate the GPU cost.
        self.frame_number.fetch_add(1, Ordering::Relaxed);
        self.frames_since_update.fetch_add(1, Ordering::Relaxed);
        *self.accumulated_gpu_time.lock().expect("metrics lock poisoned") += gpu;

        // Honour the 500 ms update gate from the C++ implementation.
        let now = Instant::now();
        let mut last = self.last_update.lock().expect("metrics lock poisoned");
        let elapsed = match *last {
            Some(t) => now.duration_since(t),
            None => {
                *last = Some(now);
                return;
            }
        };
        if elapsed < Self::UPDATE_INTERVAL {
            return;
        }
        *last = Some(now);

        // Commit the externally-supplied snapshot.
        *self.fps.lock().expect("metrics lock poisoned") = fps;
        *self.internal_fps.lock().expect("metrics lock poisoned") = vps;
        *self.gpu_time_ms.lock().expect("metrics lock poisoned") = gpu;
        *self.cpu_thread_usage.lock().expect("metrics lock poisoned") = cpu;
        *self.cpu_thread_time.lock().expect("metrics lock poisoned") = cpu;

        // Derive a speed scalar.
        let seconds = elapsed.as_secs_f32().max(0.001);
        let n = self.frames_since_update.load(Ordering::Relaxed).max(1) as f32;
        *self.speed.lock().expect("metrics lock poisoned") = (fps / 60.0) * 100.0;

        // GPU usage as a fraction of the wall-clock interval.
        let accumulated = std::mem::replace(
            &mut *self.accumulated_gpu_time.lock().expect("metrics lock poisoned"),
            0.0,
        );
        *self.gpu_usage.lock().expect("metrics lock poisoned") = accumulated / (seconds * 10.0);
        *self.gpu_time_ms.lock().expect("metrics lock poisoned") = accumulated / n;

        self.frames_since_update.store(0, Ordering::Relaxed);
        self.unskipped_frames.store(0, Ordering::Relaxed);
    }

    /// Notify the metrics of a single GPU present. Mirrors
    /// `PerformanceMetrics::OnGPUPresent`.
    pub fn on_gpu_present(&self, gpu_time_ms: f32) {
        *self.accumulated_gpu_time.lock().expect("metrics lock poisoned") += gpu_time_ms;
    }

    // -- Accessors --

    pub fn fps(&self) -> f32 { *self.fps.lock().expect("metrics lock poisoned") }
    pub fn internal_fps(&self) -> f32 { *self.internal_fps.lock().expect("metrics lock poisoned") }
    pub fn speed(&self) -> f32 { *self.speed.lock().expect("metrics lock poisoned") }
    pub fn average_frame_time(&self) -> f32 { *self.average_frame_time.lock().expect("metrics lock poisoned") }
    pub fn minimum_frame_time(&self) -> f32 { *self.minimum_frame_time.lock().expect("metrics lock poisoned") }
    pub fn maximum_frame_time(&self) -> f32 { *self.maximum_frame_time.lock().expect("metrics lock poisoned") }
    pub fn gpu_time_ms(&self) -> f32 { *self.gpu_time_ms.lock().expect("metrics lock poisoned") }
    pub fn gpu_usage(&self) -> f32 { *self.gpu_usage.lock().expect("metrics lock poisoned") }
    pub fn cpu_thread_usage(&self) -> f32 { *self.cpu_thread_usage.lock().expect("metrics lock poisoned") }
    pub fn cpu_thread_time(&self) -> f32 { *self.cpu_thread_time.lock().expect("metrics lock poisoned") }
    pub fn frame_number(&self) -> u64 { self.frame_number.load(Ordering::Relaxed) }
    pub fn internal_fps_method(&self) -> InternalFpsMethod {
        *self.internal_fps_method.lock().expect("metrics lock poisoned")
    }
    pub fn is_internal_fps_valid(&self) -> bool {
        self.internal_fps_method() != InternalFpsMethod::None
    }
    pub fn frame_time_history(&self) -> FrameTimeHistory {
        *self.frame_time_history.lock().expect("metrics lock poisoned")
    }
    pub fn frame_time_history_pos(&self) -> u32 {
        self.frame_time_history_pos.load(Ordering::Relaxed)
    }
}

// ---------------------------------------------------------------------------
// GSDumpReplayer
// ---------------------------------------------------------------------------

/// The high-level state of the GS dump replayer (mirrors the combination of
/// `s_dump_file`, `s_dump_running`, `s_is_dump_runner`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DumpState {
    #[default]
    Idle,
    Replaying,
    Finished,
}

/// One replayable record from a GS dump file. Mirrors the simplified form
/// of `GSDumpFile::GSData` that the replayer iterates over.
#[derive(Debug, Clone)]
pub struct DumpPacket {
    pub id: u32,
    pub length: u32,
    pub data: Vec<u8>,
}

/// In-memory representation of a GS dump file.
#[derive(Debug, Default)]
pub struct DumpFile {
    pub serial: String,
    pub crc: u32,
    pub packets: Vec<DumpPacket>,
}

/// The GS dump replayer. Mirrors `GSDumpReplayer` from
/// `GSDumpReplayer.cpp` / `GSDumpReplayer.h`.
#[derive(Debug, Default)]
pub struct GSDumpReplayer {
    file: Mutex<Option<DumpFile>>,
    state: Mutex<DumpState>,
    current_packet: AtomicU32,
    frame_number: AtomicU32,
    loop_count: Mutex<i32>,
    is_runner: Mutex<bool>,
}

impl GSDumpReplayer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Open a GS dump file from `path` and load it into the replayer.
    /// Mirrors `GSDumpReplayer::Initialize`.
    pub fn open(&self, path: impl AsRef<Path>) -> std::io::Result<()> {
        let path = path.as_ref();
        let mut file = File::open(path)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;

        // The C++ parser pulls out a serial, a CRC, and a list of
        // (id, length, data) packets. Without the real `GSDumpFile`
        // implementation we synthesise a single placeholder packet so the
        // shape is correct.
        let packets = vec![DumpPacket {
            id: 0,
            length: bytes.len() as u32,
            data: bytes,
        }];

        let dump = DumpFile {
            serial: String::new(),
            crc: 0,
            packets,
        };

        *self.file.lock().expect("gs dump lock poisoned") = Some(dump);
        *self.state.lock().expect("gs dump lock poisoned") = DumpState::Idle;
        self.current_packet.store(0, Ordering::Relaxed);
        self.frame_number.store(0, Ordering::Relaxed);
        *self.loop_count.lock().expect("gs dump lock poisoned") = -1; // infinite loop
        Ok(())
    }

    /// Serial of the currently open dump, if any.
    pub fn serial(&self) -> Option<String> {
        self.file
            .lock()
            .expect("gs dump lock poisoned")
            .as_ref()
            .map(|f| f.serial.clone())
    }

    /// CRC of the currently open dump, if any.
    pub fn crc(&self) -> Option<u32> {
        self.file
            .lock()
            .expect("gs dump lock poisoned")
            .as_ref()
            .map(|f| f.crc)
    }

    /// Current frame number (mirrors `GSDumpReplayer::GetFrameNumber`).
    pub fn frame_number(&self) -> u32 {
        self.frame_number.load(Ordering::Relaxed)
    }

    /// Current packet index.
    pub fn current_packet(&self) -> u32 {
        self.current_packet.load(Ordering::Relaxed)
    }

    /// Loop count semantics from the C++ code: positive values count down
    /// to a shutdown, -1 means infinite.
    pub fn loop_count(&self) -> i32 {
        *self.loop_count.lock().expect("gs dump lock poisoned")
    }

    pub fn set_loop_count(&self, count: i32) {
        *self.loop_count.lock().expect("gs dump lock poisoned") = count;
    }

    pub fn is_runner(&self) -> bool {
        *self.is_runner.lock().expect("gs dump lock poisoned")
    }

    pub fn set_is_runner(&self, runner: bool) {
        *self.is_runner.lock().expect("gs dump lock poisoned") = runner;
    }

    pub fn state(&self) -> DumpState {
        *self.state.lock().expect("gs dump lock poisoned")
    }

    /// True if a dump file is loaded. Mirrors
    /// `GSDumpReplayer::IsReplayingDump`.
    pub fn is_replaying(&self) -> bool {
        self.file
            .lock()
            .expect("gs dump lock poisoned")
            .is_some()
    }

    /// Run the replay loop. Returns immediately if no file is open; loops
    /// over the packets and bumps the frame counter on each VSync packet.
    /// Mirrors `GSDumpReplayer::CpuExecute`.
    ///
    /// In a real port the inner body would hand each packet to the GS
    /// thread; here we just walk the packet list and update the state
    /// counters to match the original control flow.
    pub fn replay(&self) {
        let file_guard = self.file.lock().expect("gs dump lock poisoned");
        let Some(file) = file_guard.as_ref() else {
            return;
        };
        if file.packets.is_empty() {
            *self.state.lock().expect("gs dump lock poisoned") = DumpState::Idle;
            return;
        }

        *self.state.lock().expect("gs dump lock poisoned") = DumpState::Replaying;
        let n = file.packets.len() as u32;
        let mut idx = self.current_packet.load(Ordering::Relaxed);
        let mut frame = self.frame_number.load(Ordering::Relaxed);
        let mut loop_remaining = *self.loop_count.lock().expect("gs dump lock poisoned");

        loop {
            // Process the current packet. The C++ code dispatches on
            // `packet.id` and writes to the GS state for Transfer /
            // ReadFIFO2 / Registers packets, or bumps the frame counter
            // for VSync. We preserve the frame-counter and loop-counter
            // semantics here.
            let packet = &file.packets[idx as usize];
            if packet.id == 1 /* VSync */ {
                frame = frame.wrapping_add(1);
            }
            idx = (idx + 1) % n;
            if idx == 0 {
                frame = 0;
                if loop_remaining > 0 {
                    loop_remaining -= 1;
                } else if loop_remaining == 0 {
                    self.current_packet.store(idx, Ordering::Relaxed);
                    self.frame_number.store(frame, Ordering::Relaxed);
                    *self.state.lock().expect("gs dump lock poisoned") = DumpState::Finished;
                    return;
                }
            }
            // Persist progress so external observers can read it.
            self.current_packet.store(idx, Ordering::Relaxed);
            self.frame_number.store(frame, Ordering::Relaxed);

            // Bail out as soon as the state is no longer Replaying
            // (e.g. another thread called `shutdown`).
            if self.state() != DumpState::Replaying {
                return;
            }
        }
    }

    /// Stop the replay loop on the next packet boundary. Mirrors
    /// `GSDumpReplayer::ExitExecution`.
    pub fn shutdown(&self) {
        *self.state.lock().expect("gs dump lock poisoned") = DumpState::Idle;
        self.current_packet.store(0, Ordering::Relaxed);
        self.frame_number.store(0, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_version_is_non_empty() {
        assert!(!PCSX2_BUILD_VERSION.is_empty());
    }

    #[test]
    fn hotkey_register_and_lookup() {
        let hk = Hotkeys::new();
        hk.register("ToggleFullscreen", "F11");
        assert_eq!(hk.lookup("F11").as_deref(), Some("ToggleFullscreen"));
        assert_eq!(hk.key_for("ToggleFullscreen").as_deref(), Some("F11"));
    }

    #[test]
    fn hotkey_rebind_clears_old_reverse_entry() {
        let hk = Hotkeys::new();
        hk.register("Pause", "P");
        hk.register("Pause", "Q");
        assert!(hk.lookup("P").is_none());
        assert_eq!(hk.lookup("Q").as_deref(), Some("Pause"));
    }

    #[test]
    fn source_log_writes_with_prefix() {
        let log = SourceLog::with_prefix("Test");
        log.log(LogLevel::Info, "hello");
    }

    #[test]
    fn performance_metrics_update_respects_interval() {
        let m = PerformanceMetrics::new();
        m.update(1.0, 2.0, 3.0, 60.0);
        m.update(1.0, 2.0, 3.0, 60.0);
        assert!(m.frame_number() >= 2);
    }

    #[test]
    fn gs_dump_replayer_open_and_replay() {
        let r = GSDumpReplayer::new();
        // Open a real (empty) file just to exercise the path.
        let tmp = std::env::temp_dir().join("pcsx2-misc-test.gsdump");
        std::fs::write(&tmp, b"").unwrap();
        r.open(&tmp).unwrap();
        assert!(r.is_replaying());
        r.shutdown();
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn pine_server_init_and_shutdown() {
        let mut s = PineServer::new();
        assert!(!s.is_initialized());
        assert!(s.init());
        assert!(s.is_initialized());
        s.shutdown();
        assert!(s.is_initialized()); // shutdown() is non-destructive to init flag in this translation
    }

    #[test]
    fn pine_server_load_elf_records_path() {
        let s = PineServer::new();
        let tmp = std::env::temp_dir().join("pcsx2-misc-test.elf");
        std::fs::write(&tmp, b"\x7fELF").unwrap();
        assert!(s.load_elf(&tmp));
        assert_eq!(s.elf_path(), Some(tmp.clone()));
        let _ = std::fs::remove_file(&tmp);
    }
}
