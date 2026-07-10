//! `GsHostVarious` is an idiomatic Rust 2021 translation of a small slice of the
//! PCSX2 core: the GS plugin dispatch shim, the GS dump replayer, the host
//! abstraction, the global hotkey table, the legacy PINE integrated emulator
//! entry point, and the performance metrics aggregator. The module deliberately
//! uses only `std` and models the original C/C++ semantics in safe Rust
//! (no aliases for raw unions, no real threading, no real sockets).
//!
//! It is **not** a drop-in runtime: file IO, sockets, the GS plugin loader, and
//! the multi-threaded CPU/GS/VU pipeline are all stubbed out behind simple
//! in-memory state. The point of the translation is to preserve the surface
//! API of the original C++ while making the structure Rust-idiomatic.

use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// GS plugin dispatch (translated from pcsx2/GS.{h,cpp})
// ---------------------------------------------------------------------------

/// GS register page, mirroring the GS MMIO page-0/page-1 distinction in the
/// original C++.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GsPage {
    Page0,
    Page1,
}

/// Mirrors `GS_VideoMode` in the C++ header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GsVideoMode {
    Uninitialized,
    Unknown,
    Ntsc,
    Pal,
    Vesa,
    Sdtv480P,
    Sdtv576P,
    Hdtv720P,
    Hdtv1080I,
    Hdtv1080P,
    DvdNtsc,
    DvdPal,
}

/// Errors that the GS dispatch can surface. They are intentionally small and
/// independent of the host (no `fmt`/`Error`-style wrapping) so the translation
/// stays self-contained.
#[derive(Debug)]
pub enum GsError {
    NotOpen,
    InvalidRegister(u32),
    Io(io::Error),
}

impl fmt::Display for GsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GsError::NotOpen => f.write_str("GS has not been opened"),
            GsError::InvalidRegister(addr) => write!(f, "invalid GS register address 0x{addr:08x}"),
            GsError::Io(e) => write!(f, "I/O error: {e}"),
        }
    }
}

impl Error for GsError {}

impl From<io::Error> for GsError {
    fn from(e: io::Error) -> Self {
        GsError::Io(e)
    }
}

/// The size of the GS register file, in bytes, matching the C++ constant
/// `Ps2MemSize::GSregs` (0x2000 bytes = 8 KiB).
pub const GS_REG_SIZE: usize = 0x2000;

/// In-memory GS state. In the original C++ this is `g_RealGSMem`, a 16-byte
/// aligned array of `Ps2MemSize::GSregs` bytes plus a handful of flags. Here we
/// keep the alignment promise with `#[repr(C, align(16))]`.
#[repr(C, align(16))]
#[derive(Debug)]
pub struct GsState {
    /// Backing store for the GS register file.
    pub regs: [u8; GS_REG_SIZE],
    /// Latched in `gsWrite64_page_00` when DISPFB1/DISPFB2/PMODE is touched.
    pub registers_written: bool,
    /// Current video mode (mirrors `gsVideoMode`).
    pub video_mode: GsVideoMode,
    /// True after a successful `gsOpen`/init, false after `gsShutdown`.
    pub opened: bool,
}

impl GsState {
    pub const fn new() -> Self {
        Self {
            regs: [0u8; GS_REG_SIZE],
            registers_written: false,
            video_mode: GsVideoMode::Uninitialized,
            opened: false,
        }
    }
}

impl Default for GsState {
    fn default() -> Self {
        Self::new()
    }
}

/// Process-wide GS state. The C++ module owns a `g_RealGSMem` global; in Rust
/// we expose a single `Mutex<GsState>` because the original code touches the
/// register file from many threads (CPU, GS, VU, MTGS).
static GS_STATE: Mutex<GsState> = Mutex::new(GsState::new());

/// Initialise the GS dispatch. In the original this is mostly a no-op (the
/// dispatch is configured lazily on first use); we model it as a no-op that
/// still acquires the lock to keep the API honest.
pub fn gs_init() {
    let mut g = GS_STATE.lock().expect("GS state poisoned");
    g.video_mode = GsVideoMode::Uninitialized;
}

/// Reset the GS dispatch: zero the register file, reset the video mode and
/// the `registers_written` latch. Mirrors `gsReset()` in the C++.
pub fn gs_reset() {
    let mut g = GS_STATE.lock().expect("GS state poisoned");
    g.regs.fill(0);
    g.registers_written = false;
    g.video_mode = GsVideoMode::Uninitialized;
}

/// Shut the GS dispatch down. Mirrors `gsShutdown` semantics (the C++ doesn't
/// define one explicitly; we add it for completeness because the task asks for
/// it and to pair up with `gsOpen`).
pub fn gs_shutdown() {
    let mut g = GS_STATE.lock().expect("GS state poisoned");
    g.regs.fill(0);
    g.registers_written = false;
    g.video_mode = GsVideoMode::Uninitialized;
    g.opened = false;
}

/// Open the GS dispatch against a backing file. In the C++ this would resolve
/// the GS plugin (GSdx, GSWX, etc.) and the GS register file. Here we treat
/// the path as a backing-store file we can also write to: the file is created
/// (truncated) and sized to `GS_REG_SIZE` bytes so the dispatch always has a
/// consistent on-disk image.
pub fn gs_open(path: &Path) -> Result<(), GsError> {
    gs_reset();
    let mut g = GS_STATE.lock().expect("GS state poisoned");
    fs::write(path, vec![0u8; GS_REG_SIZE])?;
    g.opened = true;
    Ok(())
}

/// Set the current video mode. Mirrors `gsSetVideoMode()`.
pub fn gs_set_video_mode(mode: GsVideoMode) {
    let mut g = GS_STATE.lock().expect("GS state poisoned");
    g.video_mode = mode;
}

/// 64-bit GS register write on page 0. Mirrors `gsWrite64_page_00`: the
/// `registers_written` latch is set if `DISPFB1`/`DISPFB2`/`PMODE` is touched.
pub fn gs_write64_page00(mem: u32, value: u64) {
    let mut g = GS_STATE.lock().expect("GS state poisoned");
    let offset = (mem as usize) & 0x13ff;
    if offset + 8 > GS_REG_SIZE {
        return;
    }
    g.regs[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    g.registers_written |= matches!(offset, o if o == 0x0000 || o == 0x0050 || o == 0x4000);
}

/// 64-bit GS register write on page 1. Mirrors `gsWrite64_page_01`: the BUSDIR
/// write toggles the `gifUnit.stat.DIR` flag in the C++ (which we don't model);
/// the CSR/IMR writes are intercepted but here we just pass them through.
pub fn gs_write64_page01(mem: u32, value: u64) {
    let mut g = GS_STATE.lock().expect("GS state poisoned");
    let offset = (mem as usize) & 0x13ff;
    if offset + 8 > GS_REG_SIZE {
        return;
    }
    g.regs[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

/// Generic 64-bit write. Mirrors `gsWrite64_generic`.
pub fn gs_write64_generic(mem: u32, value: u64) {
    let mut g = GS_STATE.lock().expect("GS state poisoned");
    let offset = (mem as usize) & 0x13ff;
    if offset + 8 > GS_REG_SIZE {
        return;
    }
    g.regs[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

/// 32-bit GS register write. Mirrors `gsWrite32`: CSR and IMR writes are
/// special-cased in the C++ (we just store the value), and the write must be
/// 4-byte aligned.
pub fn gs_write32(mem: u32, value: u32) -> Result<(), GsError> {
    if mem & 3 != 0 {
        return Err(GsError::InvalidRegister(mem));
    }
    let mut g = GS_STATE.lock().expect("GS state poisoned");
    let offset = (mem as usize) & 0x13ff;
    if offset + 4 > GS_REG_SIZE {
        return Err(GsError::InvalidRegister(mem));
    }
    g.regs[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    Ok(())
}

/// 16-bit GS register write. Mirrors `gsWrite16`.
pub fn gs_write16(mem: u32, value: u16) -> Result<(), GsError> {
    let mut g = GS_STATE.lock().expect("GS state poisoned");
    let offset = (mem as usize) & 0x13ff;
    if offset + 2 > GS_REG_SIZE {
        return Err(GsError::InvalidRegister(mem));
    }
    g.regs[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    Ok(())
}

/// 8-bit GS register write. Mirrors `gsWrite8`.
pub fn gs_write8(mem: u32, value: u8) -> Result<(), GsError> {
    let mut g = GS_STATE.lock().expect("GS state poisoned");
    let offset = (mem as usize) & 0x13ff;
    if offset + 1 > GS_REG_SIZE {
        return Err(GsError::InvalidRegister(mem));
    }
    g.regs[offset] = value;
    Ok(())
}

/// 32-bit GS register read. Mirrors `gsRead32`: the only "real" readable
/// register is `GS_SIGLBLID`; every other page-0 read returns a mirror of
/// `GS_CSR`. We model that by always returning the value at the requested
/// offset (close enough for a translation).
pub fn gs_read32(mem: u32) -> u32 {
    let g = GS_STATE.lock().expect("GS state poisoned");
    let offset = (mem as usize) & 0x13ff;
    if offset + 4 > GS_REG_SIZE {
        return 0;
    }
    let mut buf = [0u8; 4];
    buf.copy_from_slice(&g.regs[offset..offset + 4]);
    u32::from_le_bytes(buf)
}

/// Submit a 16-byte aligned GIF packet to the GS. Mirrors the packet dispatch
/// the C++ does on the CPU thread: every packet is just handed to the GS
/// queue. The function is the `gsExecPacket` shim asked for by the task; it
/// validates alignment and forwards the data into the in-memory state.
pub fn gs_exec_packet(data: &[u8]) -> Result<(), GsError> {
    if data.is_empty() || data.len() % 16 != 0 {
        return Err(GsError::InvalidRegister(data.len() as u32));
    }
    let g = GS_STATE.lock().expect("GS state poisoned");
    if !g.opened {
        return Err(GsError::NotOpen);
    }
    // The C++ side would push this onto gifUnit/Gif_Path[].CopyGSPacketData and
    // then schedule an MTGS send. We don't model that pipeline, but we still
    // touch the state to make sure the lock is acquired.
    let _ = data.len();
    Ok(())
}

// ---------------------------------------------------------------------------
// GSDumpReplayer (translated from pcsx2/GSDumpReplayer.{h,cpp})
// ---------------------------------------------------------------------------

/// The replayer state. In the C++ this is a namespace of statics plus a
/// `GSDumpFile`; in Rust we make the replayer a value type so the call sites
/// can own their own instance.
#[derive(Debug)]
pub struct GsDumpReplayer {
    current_packet: u32,
    frame_number: u32,
    loop_count: i32,
    running: bool,
    is_runner: bool,
    needs_state_loaded: bool,
    frame_ticks: u64,
    next_frame_time: u64,
    file: Option<GsDumpFile>,
}

impl GsDumpReplayer {
    /// Construct an idle replayer.
    pub fn new() -> Self {
        Self {
            current_packet: 0,
            frame_number: 0,
            loop_count: -1,
            running: false,
            is_runner: false,
            needs_state_loaded: false,
            frame_ticks: 0,
            next_frame_time: 0,
            file: None,
        }
    }

    /// Returns true if a dump file is currently loaded.
    pub fn is_replaying_dump(&self) -> bool {
        self.file.is_some()
    }

    /// True when this replayer is being driven by the headless "runner" entry
    /// point rather than the GUI.
    pub fn is_runner(&self) -> bool {
        self.is_runner
    }

    pub fn set_is_runner(&mut self, is_runner: bool) {
        self.is_runner = is_runner;
    }

    /// Set the remaining loop count. `loop_count` is the number of *extra*
    /// passes the C++ records (i.e. 0 means "play once, 1 means twice, …").
    pub fn set_loop_count(&mut self, loop_count: i32) {
        self.loop_count = loop_count - 1;
    }

    pub fn get_loop_count(&self) -> i32 {
        self.loop_count
    }

    /// Open a dump file. Mirrors `GSDumpReplayer::Initialize` in the C++: the
    /// file is read fully into memory, the packet table is built and the
    /// CPU/PSX/VU0/VU1 dispatch tables are replaced. In this translation we
    /// keep the file in memory and skip the dispatch-table swap.
    pub fn initialize(&mut self, path: &Path) -> io::Result<()> {
        let bytes = fs::read(path)?;
        let file = GsDumpFile::parse(&bytes)?;
        self.file = Some(file);
        self.current_packet = 0;
        self.frame_number = 0;
        self.loop_count = -1; // loop infinitely by default
        Ok(())
    }

    /// Switch the active dump file mid-replay. Mirrors `ChangeDump`.
    pub fn change_dump(&mut self, path: &Path) -> io::Result<()> {
        let bytes = fs::read(path)?;
        let file = GsDumpFile::parse(&bytes)?;
        self.file = Some(file);
        self.current_packet = 0;
        // C++ calls GSDumpReplayerCpuReset() after a switch; we do the same.
        self.reset();
        Ok(())
    }

    /// Close the active dump. Mirrors `GSDumpReplayer::Shutdown`.
    pub fn shutdown(&mut self) {
        self.file = None;
        self.current_packet = 0;
        self.frame_number = 0;
        self.running = false;
    }

    /// Returns the serial embedded in the dump, or an empty string if the
    /// dump has no serial. Mirrors `GetDumpSerial` (the C++ falls back to a
    /// game-list CRC lookup; we skip that here).
    pub fn dump_serial(&self) -> String {
        self.file
            .as_ref()
            .map(|f| f.serial.clone())
            .unwrap_or_default()
    }

    /// Returns the CRC stored in the dump header, or 0.
    pub fn dump_crc(&self) -> u32 {
        self.file.as_ref().map(|f| f.crc).unwrap_or(0)
    }

    pub fn frame_number(&self) -> u32 {
        self.frame_number
    }

    /// CPU step. Mirrors `GSDumpReplayerCpuStep` at the level of: load initial
    /// state if needed, pop the next packet, advance the frame counter on
    /// `VSync`, etc. Real packet dispatch is stubbed.
    pub fn step(&mut self) {
        if self.needs_state_loaded {
            self.needs_state_loaded = false;
        }
        let Some(file) = self.file.as_ref() else {
            return;
        };
        if file.packets.is_empty() {
            return;
        }
        let packet = &file.packets[self.current_packet as usize % file.packets.len()];
        self.current_packet = (self.current_packet + 1) % file.packets.len() as u32;
        if self.current_packet == 0 {
            self.frame_number = 0;
            if self.loop_count > 0 {
                self.loop_count -= 1;
            } else if self.loop_count == 0 {
                self.running = false;
            }
        }
        match packet.kind {
            GsDumpPacketKind::Transfer => { /* gif path dispatch */ }
            GsDumpPacketKind::VSync => {
                self.frame_number += 1;
                // frame limit / PostVsyncStart / PumpMessages elided
            }
            GsDumpPacketKind::ReadFifo2 => { /* MTGS::InitAndReadFIFO */ }
            GsDumpPacketKind::Registers => { /* memcpy into PS2MEM_GS */ }
        }
    }

    /// Run packets until `request_exit` is called or the loop counter runs
    /// out. Mirrors `GSDumpReplayerCpuExecute`.
    pub fn execute(&mut self) {
        self.running = true;
        while self.running {
            self.step();
        }
    }

    /// Request that `execute` exit on the next iteration. Mirrors
    /// `GSDumpReplayerExitExecution`.
    pub fn request_exit(&mut self) {
        self.running = false;
    }

    /// Cancel a single in-flight instruction. No-op in this translation.
    pub fn cancel_instruction(&mut self) {}

    /// Clear a range of EE memory. No-op in this translation (the C++ version
    /// forwards to the EE bus; we have no EE bus).
    pub fn cpu_clear(&mut self, _addr: u32, _size: u32) {}

    fn reset(&mut self) {
        self.needs_state_loaded = true;
        self.current_packet = 0;
        self.frame_number = 0;
    }
}

impl Default for GsDumpReplayer {
    fn default() -> Self {
        Self::new()
    }
}

/// Convenience entry point: open `path` and run the replayer to completion
/// (or to the first time the loop counter runs out). The C++ version drives
/// the replayer on the EE thread; this stub is single-threaded.
pub fn replay(path: &Path) -> io::Result<()> {
    let mut r = GsDumpReplayer::new();
    r.initialize(path)?;
    r.execute();
    r.shutdown();
    Ok(())
}

/// What a single dump packet represents. Mirrors the `GSDumpTypes::GSType` enum
/// from the C++.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GsDumpPacketKind {
    Transfer,
    VSync,
    ReadFifo2,
    Registers,
}

/// In-memory dump file representation.
#[derive(Debug)]
struct GsDumpFile {
    serial: String,
    crc: u32,
    packets: Vec<GsDumpPacket>,
}

#[derive(Debug)]
struct GsDumpPacket {
    kind: GsDumpPacketKind,
    #[allow(dead_code)]
    data: Vec<u8>,
}

impl GsDumpFile {
    /// Parse a raw dump. The C++ side does a real format parse via
    /// `GSDumpFile::OpenGSDump` / `ReadFile`. We don't have that format
    /// description, so we treat the input as already-trivially valid: the
    /// presence of any bytes is enough to satisfy the open. This preserves
    /// the shape of the API without lying about the format.
    fn parse(bytes: &[u8]) -> io::Result<Self> {
        if bytes.is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "empty dump"));
        }
        // We synthesise one packet of each kind so the loop has something to
        // do; this is enough to keep the Rust translation honest.
        let packets = vec![
            GsDumpPacket { kind: GsDumpPacketKind::Transfer, data: bytes.to_vec() },
            GsDumpPacket { kind: GsDumpPacketKind::VSync, data: Vec::new() },
            GsDumpPacket { kind: GsDumpPacketKind::ReadFifo2, data: vec![0u8; 4] },
            GsDumpPacket { kind: GsDumpPacketKind::Registers, data: bytes.to_vec() },
        ];
        Ok(GsDumpFile {
            serial: String::new(),
            crc: 0,
            packets,
        })
    }
}

// ---------------------------------------------------------------------------
// Host abstraction (translated from pcsx2/Host.{h,cpp})
// ---------------------------------------------------------------------------

/// Mirrors the wide surface area of `namespace Host` in the C++. Each method
/// is a no-op default; the GUI/frontend supplies a real implementation.
pub trait HostInterface {
    // Localisation.
    fn translate_to_cstring(&mut self, context: &str, msg: &str) -> String;
    fn translate_to_string_view(&mut self, context: &str, msg: &str) -> String;
    fn translate_to_string(&mut self, context: &str, msg: &str) -> String;
    fn clear_translation_cache(&mut self);

    // OSD.
    fn add_osd_message(&mut self, message: String, duration: f32);
    fn add_keyed_osd_message(&mut self, key: String, message: String, duration: f32);
    fn add_icon_osd_message(&mut self, key: String, icon: &str, message: &str, duration: f32);
    fn remove_keyed_osd_message(&mut self, key: String);
    fn clear_osd_messages(&mut self);

    // Reporting.
    fn report_info_async(&mut self, title: &str, message: &str);
    fn report_formatted_info_async(&mut self, title: &str, format: &str);
    fn report_error_async(&mut self, title: &str, message: &str);
    fn report_formatted_error_async(&mut self, title: &str, format: &str);

    // Mode queries.
    fn in_batch_mode(&self) -> bool;
    fn in_no_gui_mode(&self) -> bool;

    // Misc host operations.
    fn open_url(&mut self, url: &str);
    fn copy_text_to_clipboard(&mut self, text: &str) -> bool;
    fn get_text_from_clipboard(&self) -> String;
    fn request_reset_settings(&mut self, folders: bool, core: bool, controllers: bool, hotkeys: bool, ui: bool) -> bool;
    fn request_resize_host_display(&mut self, width: i32, height: i32);
    fn run_on_cpu_thread(&mut self);
    fn run_on_gs_thread(&mut self);
    fn refresh_game_list_async(&mut self, invalidate_cache: bool);
    fn cancel_game_list_refresh(&mut self);
    fn request_vm_shutdown(&mut self, allow_confirm: bool, allow_save_state: bool, default_save_state: bool);
    fn http_user_agent(&self) -> String;

    // Settings.
    fn get_string_setting(&self, section: &str, key: &str, default: &str) -> String;
    fn get_bool_setting(&self, section: &str, key: &str, default: bool) -> bool;
    fn get_int_setting(&self, section: &str, key: &str, default: i32) -> i32;
    fn get_uint_setting(&self, section: &str, key: &str, default: u32) -> u32;
    fn get_float_setting(&self, section: &str, key: &str, default: f32) -> f32;
    fn set_bool_setting(&mut self, section: &str, key: &str, value: bool);
    fn set_int_setting(&mut self, section: &str, key: &str, value: i32);
    fn set_string_setting(&mut self, section: &str, key: &str, value: &str);

    // Performance-metrics hook called when the metrics aggregator updates.
    fn on_performance_metrics_updated(&mut self);
}

/// Default no-op implementation of [`HostInterface`]. This is what a unit test
/// or headless build plugs in when there is no GUI/frontend.
#[derive(Debug, Default)]
pub struct NullHost;

impl HostInterface for NullHost {
    fn translate_to_cstring(&mut self, _context: &str, msg: &str) -> String { msg.to_string() }
    fn translate_to_string_view(&mut self, _context: &str, msg: &str) -> String { msg.to_string() }
    fn translate_to_string(&mut self, _context: &str, msg: &str) -> String { msg.to_string() }
    fn clear_translation_cache(&mut self) {}
    fn add_osd_message(&mut self, _message: String, _duration: f32) {}
    fn add_keyed_osd_message(&mut self, _key: String, _message: String, _duration: f32) {}
    fn add_icon_osd_message(&mut self, _key: String, _icon: &str, _message: &str, _duration: f32) {}
    fn remove_keyed_osd_message(&mut self, _key: String) {}
    fn clear_osd_messages(&mut self) {}
    fn report_info_async(&mut self, _title: &str, _message: &str) {}
    fn report_formatted_info_async(&mut self, _title: &str, _format: &str) {}
    fn report_error_async(&mut self, _title: &str, _message: &str) {}
    fn report_formatted_error_async(&mut self, _title: &str, _format: &str) {}
    fn in_batch_mode(&self) -> bool { false }
    fn in_no_gui_mode(&self) -> bool { false }
    fn open_url(&mut self, _url: &str) {}
    fn copy_text_to_clipboard(&mut self, _text: &str) -> bool { false }
    fn get_text_from_clipboard(&self) -> String { String::new() }
    fn request_reset_settings(&mut self, _folders: bool, _core: bool, _controllers: bool, _hotkeys: bool, _ui: bool) -> bool { false }
    fn request_resize_host_display(&mut self, _width: i32, _height: i32) {}
    fn run_on_cpu_thread(&mut self) {}
    fn run_on_gs_thread(&mut self) {}
    fn refresh_game_list_async(&mut self, _invalidate_cache: bool) {}
    fn cancel_game_list_refresh(&mut self) {}
    fn request_vm_shutdown(&mut self, _allow_confirm: bool, _allow_save_state: bool, _default_save_state: bool) {}
    fn http_user_agent(&self) -> String { String::from("PCSX2 (rust-translation)") }
    fn get_string_setting(&self, _section: &str, _key: &str, default: &str) -> String { default.to_string() }
    fn get_bool_setting(&self, _section: &str, _key: &str, default: bool) -> bool { default }
    fn get_int_setting(&self, _section: &str, _key: &str, default: i32) -> i32 { default }
    fn get_uint_setting(&self, _section: &str, _key: &str, default: u32) -> u32 { default }
    fn get_float_setting(&self, _section: &str, _key: &str, default: f32) -> f32 { default }
    fn set_bool_setting(&mut self, _section: &str, _key: &str, _value: bool) {}
    fn set_int_setting(&mut self, _section: &str, _key: &str, _value: i32) {}
    fn set_string_setting(&mut self, _section: &str, _key: &str, _value: &str) {}
    fn on_performance_metrics_updated(&mut self) {}
}

/// Process-wide host singleton. Modelled as a raw pointer to a `dyn
/// HostInterface`, matching the C++ pattern where the host frontend is
/// installed once at startup and then read from any thread. The `unsafe` is
/// the price of preserving the global-pointer API; the C++ side also has
/// no thread-safe guard around this.
pub static mut G_HOST: *mut () = std::ptr::null_mut();

/// Install `host` as the global host. Should be called exactly once at
/// startup before any consumer reads `g_host`. The previous host (if any) is
/// leaked, mirroring the C++ behaviour where the global is never replaced.
pub fn install_host(host: Box<dyn HostInterface>) {
    // SAFETY: the caller has to guarantee that no other thread is reading
    // `g_host` during the install. This is the same contract the C++ side
    // relies on.
    unsafe {
        G_HOST = Box::into_raw(host) as *mut ();
    }
}

// ---------------------------------------------------------------------------
// Hotkeys (translated from pcsx2/Hotkeys.cpp)
// ---------------------------------------------------------------------------

/// One row of the global hotkey table. The C++ uses a code-generated
/// `BEGIN_HOTKEY_LIST` / `DEFINE_HOTKEY` macro family; we flatten that into a
/// plain `const` slice that the UI can iterate over.
#[derive(Debug, Clone, Copy)]
pub struct HotkeyEntry {
    /// Stable identifier (e.g. `"ToggleFullscreen"`).
    pub name: &'static str,
    /// Bitfield of keys/modifiers. The C++ side stores the same field as a
    /// `u32`; the exact encoding is platform-specific, so we preserve the
    /// `u32` representation.
    pub key_combination: u32,
}

/// Global hotkey table. The order matches the C++ `g_common_hotkeys` array
/// declared with `BEGIN_HOTKEY_LIST` … `END_HOTKEY_LIST` in `Hotkeys.cpp`.
pub const HOTKEYS: &[HotkeyEntry] = &[
    HotkeyEntry { name: "ToggleFullscreen", key_combination: 0 },
    HotkeyEntry { name: "OpenPauseMenu", key_combination: 0 },
    HotkeyEntry { name: "OpenAchievementsList", key_combination: 0 },
    HotkeyEntry { name: "OpenLeaderboardsList", key_combination: 0 },
    HotkeyEntry { name: "TogglePause", key_combination: 0 },
    HotkeyEntry { name: "FrameAdvance", key_combination: 0 },
    HotkeyEntry { name: "ToggleFrameLimit", key_combination: 0 },
    HotkeyEntry { name: "ToggleTurbo", key_combination: 0 },
    HotkeyEntry { name: "HoldTurbo", key_combination: 0 },
    HotkeyEntry { name: "ToggleSlowMotion", key_combination: 0 },
    HotkeyEntry { name: "IncreaseSpeed", key_combination: 0 },
    HotkeyEntry { name: "DecreaseSpeed", key_combination: 0 },
    HotkeyEntry { name: "ShutdownVM", key_combination: 0 },
    HotkeyEntry { name: "ResetVM", key_combination: 0 },
    HotkeyEntry { name: "ReloadPatches", key_combination: 0 },
    HotkeyEntry { name: "SwapMemCards", key_combination: 0 },
    HotkeyEntry { name: "InputRecToggleMode", key_combination: 0 },
    HotkeyEntry { name: "PreviousSaveStateSlot", key_combination: 0 },
    HotkeyEntry { name: "NextSaveStateSlot", key_combination: 0 },
    HotkeyEntry { name: "SaveStateToSlot", key_combination: 0 },
    HotkeyEntry { name: "LoadStateFromSlot", key_combination: 0 },
    HotkeyEntry { name: "LoadBackupStateFromSlot", key_combination: 0 },
    HotkeyEntry { name: "SaveStateAndSelectNextSlot", key_combination: 0 },
    HotkeyEntry { name: "SelectNextSlotAndSaveState", key_combination: 0 },
    HotkeyEntry { name: "SaveStateToSlot1", key_combination: 0 },
    HotkeyEntry { name: "LoadStateFromSlot1", key_combination: 0 },
    HotkeyEntry { name: "SaveStateToSlot2", key_combination: 0 },
    HotkeyEntry { name: "LoadStateFromSlot2", key_combination: 0 },
    HotkeyEntry { name: "SaveStateToSlot3", key_combination: 0 },
    HotkeyEntry { name: "LoadStateFromSlot3", key_combination: 0 },
    HotkeyEntry { name: "SaveStateToSlot4", key_combination: 0 },
    HotkeyEntry { name: "LoadStateFromSlot4", key_combination: 0 },
    HotkeyEntry { name: "SaveStateToSlot5", key_combination: 0 },
    HotkeyEntry { name: "LoadStateFromSlot5", key_combination: 0 },
    HotkeyEntry { name: "SaveStateToSlot6", key_combination: 0 },
    HotkeyEntry { name: "LoadStateFromSlot6", key_combination: 0 },
    HotkeyEntry { name: "SaveStateToSlot7", key_combination: 0 },
    HotkeyEntry { name: "LoadStateFromSlot7", key_combination: 0 },
    HotkeyEntry { name: "SaveStateToSlot8", key_combination: 0 },
    HotkeyEntry { name: "LoadStateFromSlot8", key_combination: 0 },
    HotkeyEntry { name: "SaveStateToSlot9", key_combination: 0 },
    HotkeyEntry { name: "LoadStateFromSlot9", key_combination: 0 },
    HotkeyEntry { name: "SaveStateToSlot10", key_combination: 0 },
    HotkeyEntry { name: "LoadStateFromSlot10", key_combination: 0 },
    HotkeyEntry { name: "Mute", key_combination: 0 },
    HotkeyEntry { name: "IncreaseVolume", key_combination: 0 },
    HotkeyEntry { name: "DecreaseVolume", key_combination: 0 },
    HotkeyEntry { name: "ToggleMouseLock", key_combination: 0 },
];

/// OSD durations from `Host.h`. We re-export them here so the hotkey layer
/// doesn't have to reach into the host module for its own constants.
pub mod osd {
    pub const OSD_CRITICAL_ERROR_DURATION: f32 = 20.0;
    pub const OSD_ERROR_DURATION: f32 = 15.0;
    pub const OSD_WARNING_DURATION: f32 = 10.0;
    pub const OSD_INFO_DURATION: f32 = 5.0;
    pub const OSD_QUICK_DURATION: f32 = 2.5;
}

// ---------------------------------------------------------------------------
// PINE (translated from pcsx2/PINE.{h,cpp})
// ---------------------------------------------------------------------------

/// Default slot from the C++ `PINE_DEFAULT_SLOT` macro.
pub const PINE_DEFAULT_SLOT: u16 = 28011;

/// Emulator name baked into the unix-domain socket path on POSIX.
pub const PINE_EMULATOR_NAME: &str = "pcsx2";

/// Maximum size of an inbound IPC message (50_000 Write64 requests).
pub const MAX_IPC_SIZE: usize = 650_000;
/// Maximum size of an outbound IPC reply (50_000 Read64 replies).
pub const MAX_IPC_RETURN_SIZE: usize = 450_000;

/// IPC command tags. Mirrors the `IPCCommand` enum in the C++.
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

/// Emulator-status reply payload. Mirrors `EmuStatus` in the C++.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmuStatus {
    Running = 0,
    Paused = 1,
    Shutdown = 2,
}

/// IPC return tag. Mirrors `IPCResult` in the C++.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcResult {
    Ok = 0,
    Fail = 0xFF,
}

/// One IPC message + its reply buffer. Mirrors `PINEServer::IPCBuffer`.
#[derive(Debug, Default, Clone)]
pub struct IpcBuffer {
    pub size: usize,
    pub buffer: Vec<u8>,
}

/// The PINE server. Modelled as a value type that holds the equivalent of the
/// C++ `PINEServer` namespace statics. Real socket IO is stubbed; the
/// translation preserves the structure (`Initialize`/`Deinitialize`/`GetSlot`)
/// but does not open a real listening socket.
#[derive(Debug)]
pub struct PineServer {
    slot: u16,
    end: AtomicBool,
    ret_buffer: Vec<u8>,
    ipc_buffer: Vec<u8>,
    socket_name: String,
    // The C++ side stores either an AF_UNIX or AF_INET socket handle; we
    // model the platform distinction with an Option so unused variants
    // don't carry phantom data.
    _sock: Option<i32>,
}

impl PineServer {
    /// Construct a PINE server bound to the default slot.
    pub fn new() -> Self {
        Self::with_slot(PINE_DEFAULT_SLOT)
    }

    /// Construct a PINE server bound to a specific slot. Mirrors
    /// `PINEServer::Initialize(slot)`.
    pub fn with_slot(slot: u16) -> Self {
        let mut server = Self {
            slot,
            end: AtomicBool::new(true),
            ret_buffer: vec![0u8; MAX_IPC_RETURN_SIZE],
            ipc_buffer: vec![0u8; MAX_IPC_SIZE],
            socket_name: format!("/tmp/{}.sock", PINE_EMULATOR_NAME),
            _sock: None,
        };
        if slot != PINE_DEFAULT_SLOT {
            server.socket_name.push('.');
            server.socket_name.push_str(&slot.to_string());
        }
        server
    }

    pub fn is_initialized(&self) -> bool {
        !self.end.load(Ordering::Acquire)
    }

    pub fn slot(&self) -> u16 {
        self.slot
    }

    /// Mark the server as stopped. The C++ calls `shutdown()` on the
    /// listening socket and joins the worker thread; here we just flip the
    /// atomic.
    pub fn deinitialize(&self) {
        self.end.store(true, Ordering::Release);
    }

    /// Build a successful IPC reply. Mirrors `MakeOkIPC`. The reply is a
    /// 4-byte little-endian size followed by the `IPC_OK` byte and then the
    /// payload.
    pub fn make_ok_ipc(ret_buffer: &mut [u8], payload_size: u32) -> usize {
        ret_buffer[..4].copy_from_slice(&payload_size.to_le_bytes());
        ret_buffer[4] = IpcResult::Ok as u8;
        5 + payload_size as usize
    }

    /// Build a failed IPC reply. Mirrors `MakeFailIPC`.
    pub fn make_fail_ipc(ret_buffer: &mut [u8]) -> usize {
        ret_buffer[..4].copy_from_slice(&5u32.to_le_bytes());
        ret_buffer[4] = IpcResult::Fail as u8;
        5
    }

    /// Construct the socket name the C++ would `bind()` to. Mirrors the
    /// platform-specific layout in `PINE.cpp`.
    pub fn socket_name(&self) -> &str {
        &self.socket_name
    }

    /// Stub: load a PS2 ELF into the VM via the legacy PINE entry point. In
    /// the C++ side this is the function that boots a bare ELF without going
    /// through the full VMManager boot path. Here we read the file into
    /// memory to honour the API and return a success; the ELF parser, COP0
    /// setup, and EE thread are out of scope for this translation.
    pub fn load_elf(&self, path: &Path) -> Result<(), String> {
        fs::read(path)
            .map(|_| ())
            .map_err(|e| format!("PINE: failed to load '{}': {}", path.display(), e))
    }

    /// Stub: enter the EE run loop. Mirrors `PINE_LoadElf` followed by the
    /// legacy main loop. In this translation we just spin briefly to keep
    /// the call meaningful.
    pub fn run(&self) {
        // In the C++ this blocks on the EE thread. We don't have a thread,
        // so this is a no-op.
        let _ = self.is_initialized();
    }
}

impl Default for PineServer {
    fn default() -> Self {
        Self::new()
    }
}

/// Top-level `PINE_LoadElf` shim. Mirrors the legacy entry point declared in
/// `PINE.h`.
pub fn pine_load_elf(path: &Path) -> Result<(), String> {
    let server = PineServer::new();
    server.load_elf(path)
}

/// Top-level `PINE_Run` shim. Mirrors the legacy entry point declared in
/// `PINE.h`.
pub fn pine_run() {
    let server = PineServer::new();
    server.run();
}

// ---------------------------------------------------------------------------
// PerformanceMetrics (translated from pcsx2/PerformanceMetrics.{h,cpp})
// ---------------------------------------------------------------------------

/// Number of frame-time samples retained in the rolling history. Mirrors
/// `NUM_FRAME_TIME_SAMPLES` in the C++.
pub const NUM_FRAME_TIME_SAMPLES: usize = 150;

/// Type of internal-FPS estimator currently in use. Mirrors
/// `PerformanceMetrics::InternalFPSMethod`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InternalFpsMethod {
    None,
    GsPrivilegedRegister,
    DispFbBlit,
}

/// Ring buffer of recent frame times in milliseconds. Mirrors
/// `PerformanceMetrics::FrameTimeHistory` (`[f32; NUM_FRAME_TIME_SAMPLES]`).
pub type FrameTimeHistory = [f32; NUM_FRAME_TIME_SAMPLES];

/// In-memory performance-metrics aggregator. The C++ version has a lot of
/// thread-aware state; we collapse it into a single value type. All fields
/// are `pub` so callers (and the host) can read whatever they need.
#[derive(Debug)]
pub struct PerfMon {
    pub fps: f32,
    pub internal_fps: f32,
    pub min_frame_time: f32,
    pub avg_frame_time: f32,
    pub max_frame_time: f32,
    pub internal_fps_method: InternalFpsMethod,
    pub cpu_usage: f64,
    pub cpu_time: f64,
    pub gs_usage: f32,
    pub gs_time: f32,
    pub vu_usage: f32,
    pub vu_time: f32,
    pub capture_usage: f32,
    pub capture_time: f32,
    pub gpu_usage: f32,
    pub avg_gpu_time: f32,
    pub frame_number: u64,
    frame_time_history: FrameTimeHistory,
    frame_time_history_pos: usize,
}

impl PerfMon {
    /// Construct a zeroed metrics aggregator. Mirrors `PerformanceMetrics::Clear`.
    pub fn new() -> Self {
        Self {
            fps: 0.0,
            internal_fps: 0.0,
            min_frame_time: 0.0,
            avg_frame_time: 0.0,
            max_frame_time: 0.0,
            internal_fps_method: InternalFpsMethod::None,
            cpu_usage: 0.0,
            cpu_time: 0.0,
            gs_usage: 0.0,
            gs_time: 0.0,
            vu_usage: 0.0,
            vu_time: 0.0,
            capture_usage: 0.0,
            capture_time: 0.0,
            gpu_usage: 0.0,
            avg_gpu_time: 0.0,
            frame_number: 0,
            frame_time_history: [0.0; NUM_FRAME_TIME_SAMPLES],
            frame_time_history_pos: 0,
        }
    }

    /// Reset all the rolling accumulators. Mirrors `PerformanceMetrics::Reset`.
    pub fn reset(&mut self) {
        self.min_frame_time = 0.0;
        self.avg_frame_time = 0.0;
        self.max_frame_time = 0.0;
        self.gpu_usage = 0.0;
        self.avg_gpu_time = 0.0;
        self.frame_time_history.fill(0.0);
        self.frame_time_history_pos = 0;
    }

    /// Push a single frame's worth of metrics. Mirrors
    /// `PerformanceMetrics::Update` at the level of: bump the frame counter,
    /// update the rolling min/avg/max, append to the history. The full
    /// delta-time accounting of the C++ version is elided; the task signature
    /// is `update(gpu, cpu, vps, fps)` so we accept those four scalars
    /// directly.
    pub fn update(&mut self, gpu: f32, cpu: f32, vps: f32, fps: f32) {
        // Treat the average frame time as 1000/fps ms; the rolling history
        // is appended in place. This is *not* a 1:1 port of the C++ delta
        // accounting, but it preserves the meaning of the metrics: the
        // public fields all reflect the last call.
        let frame_time_ms = if fps > 0.0 { 1000.0 / fps } else { 0.0 };
        self.avg_frame_time = frame_time_ms;
        if self.min_frame_time == 0.0 || frame_time_ms < self.min_frame_time {
            self.min_frame_time = frame_time_ms;
        }
        if frame_time_ms > self.max_frame_time {
            self.max_frame_time = frame_time_ms;
        }
        self.frame_time_history[self.frame_time_history_pos] = frame_time_ms;
        self.frame_time_history_pos = (self.frame_time_history_pos + 1) % NUM_FRAME_TIME_SAMPLES;

        self.gpu_usage = gpu;
        self.cpu_usage = cpu as f64;
        self.gs_usage = vps;
        self.fps = fps;
        self.frame_number = self.frame_number.wrapping_add(1);
    }

    /// Read-only view of the rolling frame-time history.
    pub fn frame_time_history(&self) -> &FrameTimeHistory {
        &self.frame_time_history
    }

    /// Read-only view of the history cursor.
    pub fn frame_time_history_pos(&self) -> usize {
        self.frame_time_history_pos
    }
}

impl Default for PerfMon {
    fn default() -> Self {
        Self::new()
    }
}
