// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust 2021 translation of `pcsx2/VMManager.{h,cpp}`.
//!
//! This module owns the PlayStation 2 virtual-machine lifecycle:
//!
//! * [`VmManager::initialize`]  - bring up CDVD, BIOS, GS, SPU2, USB,
//!   FW, DEV9, memcards, SIO0/SIO2, PAD, and run the optional
//!   save-state load.
//! * [`VmManager::shutdown`]    - symmetric teardown, including the
//!   optional resume-state save.
//! * [`VmManager::reset`]       - full cold reset; defers to the CPU
//!   thread when currently `Running`.
//! * [`VmManager::reset_cpu`]   - `cpuReset()` plus the matching
//!   re-bind of conditional memory handlers and the EE/VU caches.
//! * [`VmManager::boot`]        - sets state to `Running` after init
//!   (or after a save-state restore).
//! * [`VmManager::pause`] /
//!   [`VmManager::resume`]      - mirror `SetPaused`.
//! * [`VmManager::save_state`] /
//!   [`VmManager::load_state`]  - `SaveState` / `LoadState`.
//! * [`VmManager::get_cpu`]     - `GetCPU()`; returns the currently
//!   selected EE CPU implementation, if any.
//!
//! The original C++ module talks to a wide cross-section of
//! subsystems (CDVD, MTGS, SPU2, USB, FW, DEV9, PAD, SIO, achievements,
//! input recording, save-state archiving, ...).  Those subsystems are
//! modelled here as small opaque stubs so the lifecycle can be
//! reviewed in isolation.  Every call site that would touch a
//! subsystem carries a clearly-labelled `TODO(real subsystem)`
//! pointing at the C++ entry point that should be wired in once the
//! rest of the Rust port catches up.
//!
//! Only `std` is used.

#![allow(dead_code)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};

// ---------------------------------------------------------------------------
// Opaque placeholder types for the PCSX2 subsystems that VMManager talks to.
// ---------------------------------------------------------------------------

/// The full emulator configuration.  C++: `Pcsx2Config` from `Config.h`.
#[derive(Clone, Debug, Default)]
pub struct Pcsx2Config {
    pub inner: ConfigInner,
}

/// Placeholder payload for [`Pcsx2Config`].
#[derive(Clone, Debug, Default)]
pub struct ConfigInner {
    pub enable_fast_boot: bool,
    pub enable_fast_boot_fast_forward: bool,
    pub inhibit_screensaver: bool,
    pub cdvd_precache: bool,
    pub backup_savestate: bool,
    pub enable_discord_presence: bool,
    pub emulation_speed: EmulationSpeedConfig,
    pub gs: GsConfig,
    pub cpu: CpuConfig,
    pub gamefixes: GamefixesConfig,
}

#[derive(Clone, Debug, Default)]
pub struct EmulationSpeedConfig {
    pub nominal_scalar: f32,
    pub slomo_scalar: f32,
    pub turbo_scalar: f32,
    pub sync_to_host_refresh_rate: bool,
    pub use_vsync_for_timing: bool,
}

#[derive(Clone, Debug, Default)]
pub struct GsConfig {
    pub vsync_enable: bool,
    pub skip_duplicate_frames: bool,
    pub disable_mailbox_presentation: bool,
    pub manual_user_hacks: bool,
    pub aspect_ratio: AspectRatio,
    pub upscale_multiplier: f32,
}

#[derive(Clone, Debug, Default)]
pub struct CpuConfig {
    pub extra_memory: u32,
    pub fpu_fpcr: u32,
    pub recompiler: CpuRecompilerConfig,
}

#[derive(Clone, Debug, Default)]
pub struct CpuRecompilerConfig {
    pub enable_vu0: bool,
}

#[derive(Clone, Debug, Default)]
pub struct GamefixesConfig {
    pub instant_dma_hack: bool,
}

#[derive(Clone, Debug, Default, Copy)]
pub enum AspectRatio {
    #[default]
    RAuto4_3_3_2,
    R4_3,
    R16_9,
    R10_7,
    Stretch,
}

/// Source type of the CDVD drive.  C++: `CDVD_SourceType` from `CDVD.h`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CdvdSourceType {
    #[default]
    NoDisc,
    Iso,
    Disc,
    Odd,
}

/// Vsync mode the GS will use.  C++: `GSVSyncMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GsVSyncMode {
    Disabled,
    Fifo,
    Mailbox,
}

/// Speed-limiter mode.  C++: `LimiterModeType` from `Counters.h`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LimiterModeType {
    #[default]
    Nominal,
    Slomo,
    Turbo,
    Unlimited,
}

/// Result of a VM boot attempt.  C++: `VMBootResult`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VmBootResult {
    /// The boot succeeded.
    StartupSuccess,
    /// The boot failed and an error should be displayed in the UI.
    StartupFailure,
    /// The boot failed because the user needs to be prompted to disable
    /// hardcore mode.
    PromptDisableHardcoreMode,
}

/// Current state of the VM.  C++: `VMState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum VmState {
    Shutdown = 0,
    Initializing = 1,
    Running = 2,
    Paused = 3,
    Resetting = 4,
    Stopping = 5,
}

/// VM-level error type.  C++: `Error` from `common/Error.h`.
#[derive(Clone, Debug, Default)]
pub struct Error {
    pub message: String,
}

impl Error {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn set(&mut self, message: impl Into<String>) {
        self.message = message.into();
    }

    pub fn is_empty(&self) -> bool {
        self.message.is_empty()
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Parameters that control how a VM is booted.  C++: `VMBootParameters`.
#[derive(Clone, Debug, Default)]
pub struct VmBootParameters {
    pub filename: String,
    pub elf_override: String,
    pub save_state: String,
    pub state_index: Option<i32>,
    pub source_type: Option<CdvdSourceType>,

    pub fast_boot: Option<bool>,
    pub fullscreen: Option<bool>,
    pub start_turbo: Option<bool>,
    pub start_unlimited: Option<bool>,
    pub disable_achievements_hardcore_mode: bool,
}

/// Opaque handle to the currently selected EE CPU.
///
/// The C++ code wires up multiple CPU implementations
/// (interpreter, EE recompiler, GS-dump replayer) and exposes a
/// `Cpu` global pointing at the active one.  In this translation
/// the implementations are abstract trait objects behind a thin
/// facade, and [`VmManager::get_cpu`] hands them out to callers
/// (save-state, debugger, ...) when the VM is alive.
pub trait Cpu: std::fmt::Debug {
    /// Reset the CPU to a cold-boot state.
    fn reset(&mut self);
    /// Execute the CPU until [`Cpu::exit`] is called.
    fn execute(&mut self);
    /// Request that the running [`Cpu::execute`] loop return at the
    /// next safe point.  Mirrors `Cpu->ExitExecution()`.
    fn exit(&mut self);
    /// Returns true when the CPU is between two instructions and
    /// therefore safe to mutate state on.  Mirrors the read of
    /// `VMManager::Internal::IsExecutionInterrupted()`.
    fn is_interrupted(&self) -> bool;
}

/// Default CPU implementation used by the translation when no
/// concrete EE interpreter or recompiler has been wired in.
#[derive(Debug, Default)]
pub struct StubCpu {
    interrupted: bool,
}

impl Cpu for StubCpu {
    fn reset(&mut self) {
        self.interrupted = true;
    }
    fn execute(&mut self) {
        // The stub never actually executes instructions; it just
        // parks until something calls `exit`.
    }
    fn exit(&mut self) {
        self.interrupted = true;
    }
    fn is_interrupted(&self) -> bool {
        self.interrupted
    }
}

// ---------------------------------------------------------------------------
// In-process state shared by every VM.  C++: the various file-scope
// `s_*` globals in `VMManager.cpp`.
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct GlobalState {
    state: AtomicU8,
    limiter_mode: LimiterModeType,
    limiter_ticks_per_frame: i64,
    limiter_frame_start: u64,
    target_speed: f32,
    target_speed_can_sync_to_host: bool,
    target_speed_synced_to_host: bool,
    use_vsync_for_timing: bool,
    frame_advance_count: u32,
    fast_boot_requested: bool,
    gs_open_on_initialize: bool,
    disc_serial: String,
    disc_elf: String,
    disc_version: String,
    title: String,
    title_en_search: String,
    title_en_replace: String,
    disc_crc: u32,
    current_crc: u32,
    elf_entry_point: u32,
    elf_path: String,
    elf_executed: bool,
    elf_override: String,
    input_profile_name: String,
    session_resume_timestamp: u64,
    session_accumulated_playtime: u64,
}

impl Default for GlobalState {
    fn default() -> Self {
        Self {
            state: AtomicU8::new(VmState::Shutdown as u8),
            limiter_mode: LimiterModeType::default(),
            limiter_ticks_per_frame: 0,
            limiter_frame_start: 0,
            target_speed: 0.0,
            target_speed_can_sync_to_host: false,
            target_speed_synced_to_host: false,
            use_vsync_for_timing: false,
            frame_advance_count: 0,
            fast_boot_requested: false,
            gs_open_on_initialize: false,
            disc_serial: String::new(),
            disc_elf: String::new(),
            disc_version: String::new(),
            title: String::new(),
            title_en_search: String::new(),
            title_en_replace: String::new(),
            disc_crc: 0,
            current_crc: 0,
            elf_entry_point: 0,
            elf_path: String::new(),
            elf_executed: false,
            elf_override: String::new(),
            input_profile_name: String::new(),
            session_resume_timestamp: 0,
            session_accumulated_playtime: 0,
        }
    }
}

impl GlobalState {
    fn new() -> Self {
        Self::default()
    }

    fn state(&self) -> VmState {
        match self.state.load(Ordering::Acquire) {
            0 => VmState::Shutdown,
            1 => VmState::Initializing,
            2 => VmState::Running,
            3 => VmState::Paused,
            4 => VmState::Resetting,
            5 => VmState::Stopping,
            _ => VmState::Shutdown,
        }
    }

    fn set_state(&self, new_state: VmState) {
        self.state.store(new_state as u8, Ordering::Release);
    }
}

// ---------------------------------------------------------------------------
// Public VmManager type
// ---------------------------------------------------------------------------

/// The top-level VM lifecycle.  C++: the `VMManager` namespace combined
/// with the file-scope globals in `VMManager.cpp`.
#[derive(Debug, Default)]
pub struct VmManager {
    config: Pcsx2Config,
    g: GlobalState,
    /// Set to `true` once a VM has been successfully booted and not yet
    /// torn down.  Mirrors the C++ `s_state == Running | Paused | Resetting`
    /// check that `HasValidVM()` performs.
    has_valid_vm: bool,
    /// Currently selected EE CPU.  Mirrors the C++ `Cpu` global that
    /// `UpdateCPUImplementations()` repoints at runtime.
    cpu: Option<Box<dyn Cpu>>,
}

impl VmManager {
    /// Construct a fresh, shut-down VM.  Use [`VmManager::initialize`] to
    /// bring it up.
    pub fn new() -> Self {
        Self::default()
    }

    // ---- public API requested by the task ---------------------------------

    /// Resets all subsystems to a cold boot.  Mirrors `VMManager::Reset`.
    pub fn reset(&mut self) {
        if !self.has_valid_vm {
            return;
        }

        // If we are currently running, defer the actual reset until the
        // CPU thread observes the `Resetting` state -- otherwise we would
        // tear down state from under a live EE thread.  The C++ version
        // does the same dance.
        if self.g.state() == VmState::Running {
            self.g.set_state(VmState::Resetting);
            return;
        }

        // C++ ClearELFInfo / HandleELFChange.
        let elf_was_changed = self.g.current_crc != 0;
        self.g.current_crc = 0;
        self.g.elf_executed = false;
        self.g.elf_entry_point = 0xFFFF_FFFF;
        self.g.elf_path = String::new();
        if elf_was_changed {
            self.handle_elf_change();
        }

        self.reset_cpu();
        self.hardware_reset();
        self.reset_frame_limiter();

        // If we were paused we won't ever have been flipped to `Resetting`,
        // so do not bump back to `Running` in that case.
        if self.g.state() == VmState::Resetting {
            self.g.set_state(VmState::Running);
        }
    }

    /// Reset only the CPU state (no per-game patches reload, no
    /// ELF-handle bookkeeping).  Mirrors the inner block of
    /// `VMManager::Reset` that performs `mmap_ResetBlockTracking` /
    /// `memSetExtraMemMode` / `ClearCPUExecutionCaches` /
    /// `memBindConditionalHandlers` / `SysMemory::Reset` /
    /// `cpuReset`.
    pub fn reset_cpu(&mut self) {
        if !self.has_valid_vm {
            return;
        }
        // TODO: mmap_ResetBlockTracking
        // TODO: memSetExtraMemMode(EmuConfig.Cpu.ExtraMemory)
        // TODO: Internal::ClearCPUExecutionCaches
        // TODO: memBindConditionalHandlers
        // TODO: SysMemory::Reset
        if let Some(cpu) = self.cpu.as_mut() {
            cpu.reset();
        }
    }

    /// Initializes all system components and returns `true` on success.
    /// Mirrors `VMManager::Initialize` in the C++ original.  Returns
    /// `true` on success / `false` on failure to satisfy the `-> bool`
    /// signature requested by the task; richer failure information
    /// is surfaced via the optional [`Error`] parameter.
    pub fn initialize(
        &mut self,
        boot_params: &VmBootParameters,
        error: Option<&mut Error>,
    ) -> bool {
        if self.g.state() != VmState::Shutdown {
            if let Some(err) = error {
                err.set("The virtual machine is already running.");
            }
            return false;
        }

        self.g.set_state(VmState::Initializing);
        self.g.elf_override = boot_params.elf_override.clone();
        self.g.disc_serial = String::new();
        self.g.disc_elf = String::new();
        self.g.disc_version = String::new();
        self.g.title = String::new();
        self.g.disc_crc = 0;
        self.g.current_crc = 0;
        self.g.elf_executed = false;
        self.g.fast_boot_requested = false;
        self.g.gs_open_on_initialize = false;
        self.g.frame_advance_count = 0;
        self.g.limiter_mode = if boot_params.start_unlimited.unwrap_or(false) {
            LimiterModeType::Unlimited
        } else if boot_params.start_turbo.unwrap_or(false) {
            LimiterModeType::Turbo
        } else {
            LimiterModeType::Nominal
        };

        // Begin the ordered bring-up.  Each step is annotated with the
        // matching C++ entry point.
        if let Err(e) = self.init_cdvd_subsystem() {
            if let Some(err) = error {
                err.set(format!("CDVD lock: {e}"));
            }
            self.fail_initialize();
            return false;
        }
        if let Err(e) = self.init_bios() {
            if let Some(err) = error {
                err.set(format!("BIOS: {e}"));
            }
            self.fail_initialize();
            return false;
        }
        if let Err(e) = self.open_cdvd() {
            if let Some(err) = error {
                err.set(format!("CDVD open: {e}"));
            }
            self.fail_initialize();
            return false;
        }
        if let Err(e) = self.update_disc_details(true) {
            if let Some(err) = error {
                err.set(format!("Disc details: {e}"));
            }
            self.fail_initialize();
            return false;
        }
        if let Err(e) = self.open_memory_cards() {
            if let Some(err) = error {
                err.set(format!("Memcards: {e}"));
            }
            self.fail_initialize();
            return false;
        }
        if let Err(e) = self.init_pad() {
            if let Some(err) = error {
                err.set(format!("PAD: {e}"));
            }
            self.fail_initialize();
            return false;
        }
        if let Err(e) = self.init_spu2() {
            if let Some(err) = error {
                err.set(format!("SPU2: {e}"));
            }
            self.fail_initialize();
            return false;
        }
        if let Err(e) = self.init_gs() {
            if let Some(err) = error {
                err.set(format!("GS: {e}"));
            }
            self.fail_initialize();
            return false;
        }
        if let Err(e) = self.init_sio() {
            if let Some(err) = error {
                err.set(format!("SIO: {e}"));
            }
            self.fail_initialize();
            return false;
        }
        if let Err(e) = self.init_dev9() {
            if let Some(err) = error {
                err.set(format!("DEV9: {e}"));
            }
            self.fail_initialize();
            return false;
        }
        if let Err(e) = self.init_usb() {
            if let Some(err) = error {
                err.set(format!("USB: {e}"));
            }
            self.fail_initialize();
            return false;
        }
        if let Err(e) = self.init_fw() {
            if let Some(err) = error {
                err.set(format!("FW: {e}"));
            }
            self.fail_initialize();
            return false;
        }
        self.reset_cpu();
        self.hardware_reset();

        // Install the CPU implementation that was chosen at bring-up.
        // The C++ code stores `Cpu`, `psxCpu`, `CpuVU0`, `CpuVU1` as
        // raw pointers; in the Rust port we hold the EE CPU behind a
        // trait object so callers can dispatch through the [`Cpu`]
        // trait without caring about the underlying engine.
        self.cpu = Some(Box::new(StubCpu::default()));

        // The VM is now alive; flip to the post-init resting state.
        self.g.set_state(VmState::Paused);
        self.has_valid_vm = true;

        // Honour an optional save-state load that was requested via
        // boot parameters.  Mirrors the C++ block at the very end of
        // `Initialize`.
        if !boot_params.save_state.is_empty() {
            if let Err(e) = self.load_state_inner(Path::new(&boot_params.save_state)) {
                if let Some(err) = error {
                    err.set(e);
                }
                self.shutdown(false);
                return false;
            }
        }

        true
    }

    /// Destroys all system components.  Mirrors `VMManager::Shutdown`.
    pub fn shutdown(&mut self, save_resume_state: bool) {
        if self.g.state() == VmState::Shutdown {
            return;
        }

        self.g.set_state(VmState::Stopping);

        if save_resume_state && self.has_valid_vm {
            // TODO: DoSaveState(resume_file_name, -1, /*zip_on_thread=*/true,
            //                   /*backup_old_state=*/false, ...);
            let _ = self.synthesize_resume_filename();
        }

        // Subsystem shutdown order is intentional -- see C++ `Shutdown`.
        self.shutdown_fw();
        self.shutdown_usb();
        self.shutdown_dev9();
        self.shutdown_pad();
        self.shutdown_sio();
        self.shutdown_spu2();
        self.shutdown_gs();
        self.shutdown_memory_cards();
        self.shutdown_cdvd();

        // Detach the EE CPU before dropping the VM.
        self.cpu = None;

        self.g.set_state(VmState::Shutdown);
        self.has_valid_vm = false;
        self.g.elf_override = String::new();
        self.g.disc_serial = String::new();
        self.g.disc_elf = String::new();
        self.g.disc_version = String::new();
        self.g.title = String::new();
        self.g.disc_crc = 0;
        self.g.current_crc = 0;
        self.g.elf_executed = false;
        self.g.fast_boot_requested = false;
    }

    /// Boots the VM.  In the C++ original this is folded into
    /// `Initialize`; here it is split out to satisfy the task's
    /// API.  Calling `boot` assumes the VM has just been
    /// [`initialize`](Self::initialize)d.
    pub fn boot(&mut self) -> bool {
        if !self.has_valid_vm {
            return false;
        }

        // If a save-state load was deferred (e.g. it failed during
        // initialize and was logged there), do nothing extra here --
        // the user-visible failure path goes through `load_state`.

        self.g.set_state(VmState::Running);
        true
    }

    /// Pauses the VM.  Mirrors `SetPaused(true)`.
    pub fn pause(&mut self) {
        if !self.has_valid_vm {
            return;
        }
        self.g.set_state(VmState::Paused);
    }

    /// Resumes the VM.  Mirrors `SetPaused(false)`.
    pub fn resume(&mut self) {
        if !self.has_valid_vm {
            return;
        }
        self.g.set_state(VmState::Running);
        self.reset_frame_limiter();
    }

    /// Saves the current VM state to `path`.  Mirrors `SaveState`.
    /// Returns `true` on success.
    pub fn save_state(&mut self, path: &str) -> bool {
        if !self.has_valid_vm {
            return false;
        }
        if path.is_empty() {
            return false;
        }
        let p = Path::new(path);
        if p.as_os_str().is_empty() {
            return false;
        }
        // TODO: SaveState_DownloadState + SaveState_ZipToDisk
        // For now we just record the requested path so the
        // resume-state machine has somewhere to point at.
        self.g.elf_path = p.to_string_lossy().into_owned();
        true
    }

    /// Loads a VM state from `path`.  Mirrors `LoadState`.
    /// Returns `true` on success.
    pub fn load_state(&mut self, path: &str) -> bool {
        if !self.has_valid_vm {
            return false;
        }
        if path.is_empty() {
            return false;
        }
        match self.load_state_inner(Path::new(path)) {
            Ok(()) => true,
            Err(_) => false,
        }
    }

    /// Internal helper: do the file check + record-keeping without
    /// the `bool` return.  Returns `Err(String)` so callers can
    /// propagate the failure message up to `Initialize` and out to
    /// the host UI.
    fn load_state_inner(&mut self, p: &Path) -> Result<(), String> {
        if !p.exists() {
            return Err(format!(
                "Save state file does not exist: {}",
                p.display()
            ));
        }
        // TODO: SaveState_UnzipFromDisk
        self.g.elf_path = p.to_string_lossy().into_owned();
        Ok(())
    }

    /// Returns the active EE CPU, if the VM is currently alive.
    /// Mirrors the C++ `Cpu` global returned by `GetCPU()`.
    pub fn get_cpu(&self) -> Option<&dyn Cpu> {
        if !self.has_valid_vm {
            return None;
        }
        self.cpu.as_deref().map(|c| c as &dyn Cpu)
    }

    /// Mutable counterpart of [`VmManager::get_cpu`].  Mirrors the
    /// `Cpu` global as used by the save-state code and the debugger.
    pub fn get_cpu_mut(&mut self) -> Option<&mut dyn Cpu> {
        if !self.has_valid_vm {
            return None;
        }
        match self.cpu.as_deref_mut() {
            Some(c) => Some(c as &mut dyn Cpu),
            None => None,
        }
    }

    // ---- additional public accessors mirroring the C++ free functions -----

    /// Returns the current state of the VM.  C++: `VMManager::GetState`.
    pub fn state(&self) -> VmState {
        self.g.state()
    }

    /// Returns true if there is an active virtual machine.  C++:
    /// `VMManager::HasValidVM`.
    pub fn has_valid_vm(&self) -> bool {
        self.has_valid_vm
    }

    /// Returns the path of the disc currently running.
    pub fn disc_path(&self) -> String {
        self.g.disc_elf.clone()
    }

    /// Returns the serial of the disc currently running.
    pub fn disc_serial(&self) -> String {
        self.g.disc_serial.clone()
    }

    /// Returns the main ELF of the disc currently running.
    pub fn disc_elf(&self) -> String {
        self.g.disc_elf.clone()
    }

    /// Returns the disc version.
    pub fn disc_version(&self) -> String {
        self.g.disc_version.clone()
    }

    /// Returns the disc CRC.
    pub fn disc_crc(&self) -> u32 {
        self.g.disc_crc
    }

    /// Returns the CRC of the executable currently running.
    pub fn current_crc(&self) -> u32 {
        self.g.current_crc
    }

    /// Returns the path to the ELF currently running.
    pub fn current_elf(&self) -> &str {
        &self.g.elf_path
    }

    /// Returns the title of the running disc / executable.
    pub fn title(&self) -> String {
        self.g.title.clone()
    }

    /// Returns the configured target speed.
    pub fn target_speed(&self) -> f32 {
        self.g.target_speed
    }

    /// Returns the active limiter mode.
    pub fn limiter_mode(&self) -> LimiterModeType {
        self.g.limiter_mode
    }

    /// Sets the limiter mode and recomputes the target speed.
    pub fn set_limiter_mode(&mut self, mode: LimiterModeType) {
        if self.g.limiter_mode == mode {
            return;
        }
        self.g.limiter_mode = mode;
        self.update_target_speed();
    }

    /// Updates the target speed to match the current config and limiter
    /// mode.  Mirrors `UpdateTargetSpeed`.
    pub fn update_target_speed(&mut self) {
        self.g.target_speed = self.target_speed_for_limiter_mode(self.g.limiter_mode);
    }

    /// Runs the VM for the specified number of video frames and then
    /// automatically pauses.  Mirrors `FrameAdvance`.
    pub fn frame_advance(&mut self, num_frames: u32) {
        if !self.has_valid_vm {
            return;
        }
        self.g.frame_advance_count = num_frames;
        self.g.set_state(VmState::Running);
    }

    /// Polls input, updates subsystems which tick while paused or inactive.
    /// Mirrors `IdlePollUpdate`.
    pub fn idle_poll_update(&mut self) {
        // TODO: Achievements::IdleUpdate, PollDiscordPresence, InputManager::PollSources
    }

    /// Returns true if the specified path looks like an ELF.  C++:
    /// `IsElfFileName`.
    pub fn is_elf_file_name(path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("elf"))
            .unwrap_or(false)
    }

    /// Returns true if the specified path looks like a save state.  C++:
    /// `IsSaveStateFileName`.
    pub fn is_save_state_file_name(path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("p2s"))
            .unwrap_or(false)
    }

    /// Returns true if the specified path looks like a disc image.  C++:
    /// `IsDiscFileName`.
    pub fn is_disc_file_name(path: &Path) -> bool {
        match path.extension().and_then(|e| e.to_str()) {
            Some(ext) => matches!(
                ext.to_ascii_lowercase().as_str(),
                "iso" | "bin" | "img" | "mdf" | "gz" | "cso" | "zso" | "chd"
            ),
            None => false,
        }
    }

    /// Returns true if the specified path looks like a GS dump.  C++:
    /// `IsGSDumpFileName`.
    pub fn is_gs_dump_file_name(path: &Path) -> bool {
        // The C++ version matches `.gs`, `.gs.xz` and `.gs.zst`.  We do
        // the same by inspecting the file name.
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => return false,
        };
        let lower = name.to_ascii_lowercase();
        lower.ends_with(".gs") || lower.ends_with(".gs.xz") || lower.ends_with(".gs.zst")
    }

    /// Returns true if the specified path looks like a block dump.  C++:
    /// `IsBlockDumpFileName`.
    pub fn is_block_dump_file_name(path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("dump"))
            .unwrap_or(false)
    }

    /// Returns true if the specified path looks loadable.  C++:
    /// `IsLoadableFileName`.
    pub fn is_loadable_file_name(path: &Path) -> bool {
        Self::is_disc_file_name(path)
            || Self::is_elf_file_name(path)
            || Self::is_gs_dump_file_name(path)
            || Self::is_block_dump_file_name(path)
    }

    /// Returns the save-state filename for a given serial / CRC / slot.
    /// C++: `GetSaveStateFileName(const char*, u32, s32, bool)`.
    pub fn save_state_file_name(
        game_serial: &str,
        game_crc: u32,
        slot: i32,
        backup: bool,
    ) -> Option<PathBuf> {
        if game_serial.is_empty() {
            return None;
        }
        let name = if slot < 0 {
            format!("{game_serial} ({game_crc:08X}).resume.p2s")
        } else if backup {
            format!("{game_serial} ({game_crc:08X}).{slot:02}.p2s.backup")
        } else {
            format!("{game_serial} ({game_crc:08X}).{slot:02}.p2s")
        };
        // The C++ version places save states under `EmuFolders::Savestates`;
        // we return just the filename since the host owns the path.
        Some(PathBuf::from(name))
    }

    /// Returns true if a save state exists in the specified slot.  C++:
    /// `HasSaveStateInSlot`.
    pub fn has_save_state_in_slot(
        &self,
        game_serial: &str,
        game_crc: u32,
        slot: i32,
    ) -> bool {
        match Self::save_state_file_name(game_serial, game_crc, slot, false) {
            Some(name) => name.exists(),
            None => false,
        }
    }

    /// Synthesize the resume-state filename used by `Shutdown(true)`.
    /// Mirrors `GetCurrentSaveStateFileName(-1)`.
    fn synthesize_resume_filename(&self) -> Option<PathBuf> {
        Self::save_state_file_name(&self.g.disc_serial, self.g.disc_crc, -1, false)
    }

    /// Roll the VM back to the Shutdown state after a failed init.
    /// Mirrors the C++ `ScopedGuard close_state` in `Initialize`.
    fn fail_initialize(&mut self) {
        self.cpu = None;
        self.g.set_state(VmState::Shutdown);
        self.has_valid_vm = false;
    }

    // ---- private subsystem bring-up / tear-down ---------------------------

    /// Reserve the CDVD subsystem.  C++: `cdvdLock` + `ScopedGuard
    /// unlock_cdvd = &cdvdUnlock`.
    fn init_cdvd_subsystem(&mut self) -> Result<(), String> {
        // TODO: cdvdLock
        Ok(())
    }

    /// Load the BIOS.  C++: `LoadBIOS`.
    fn init_bios(&mut self) -> Result<(), String> {
        // TODO: LoadBIOS
        Ok(())
    }

    /// Open the CDVD device.  C++: `DoCDVDopen`.
    fn open_cdvd(&mut self) -> Result<(), String> {
        // TODO: DoCDVDopen
        Ok(())
    }

    /// Refresh the disc-detail globals.  C++: `UpdateDiscDetails(true)`.
    fn update_disc_details(&mut self, _booting: bool) -> Result<(), String> {
        // TODO: cdvdGetDiscInfo / GameList::GetCustomTitleForPath /
        //       GameDatabase::findGame / ApplySettings / ReportGameChangeToHost
        Ok(())
    }

    /// Open memory cards.  C++: `FileMcd_Reopen`.
    fn open_memory_cards(&mut self) -> Result<(), String> {
        // TODO: FileMcd_Reopen
        Ok(())
    }

    /// Initialize the controller subsystem.  C++: `Pad::Initialize`.
    fn init_pad(&mut self) -> Result<(), String> {
        // TODO: Pad::Initialize
        Ok(())
    }

    /// Initialize the SPU2 subsystem.  C++: `SPU2::Open`.
    fn init_spu2(&mut self) -> Result<(), String> {
        // TODO: SPU2::Open
        Ok(())
    }

    /// Initialize the GS subsystem.  C++: `MTGS::WaitForOpen`.
    fn init_gs(&mut self) -> Result<(), String> {
        // TODO: MTGS::WaitForOpen
        self.g.gs_open_on_initialize = false;
        Ok(())
    }

    /// Initialize the SIO2 / SIO0 subsystems.  C++: `g_Sio2.Initialize`
    /// + `g_Sio0.Initialize`.
    fn init_sio(&mut self) -> Result<(), String> {
        // TODO: g_Sio2.Initialize / g_Sio0.Initialize
        Ok(())
    }

    /// Initialize DEV9.  C++: `DEV9init` + `DEV9open`.
    fn init_dev9(&mut self) -> Result<(), String> {
        // TODO: DEV9init / DEV9open
        Ok(())
    }

    /// Initialize USB.  C++: `USBopen`.
    fn init_usb(&mut self) -> Result<(), String> {
        // TODO: USBopen
        Ok(())
    }

    /// Initialize FW.  C++: `FWopen`.
    fn init_fw(&mut self) -> Result<(), String> {
        // TODO: FWopen
        Ok(())
    }

    /// Perform the full hardware reset.  C++: `mmap_ResetBlockTracking` +
    /// `memSetExtraMemMode` + `ClearCPUExecutionCaches` + `cpuReset` +
    /// `hwReset`.
    fn hardware_reset(&mut self) {
        // TODO: mmap_ResetBlockTracking, memSetExtraMemMode, ClearCPUExecutionCaches,
        //       memBindConditionalHandlers, SysMemory::Reset, cpuReset, hwReset
    }

    /// Reset the frame limiter.  C++: `ResetFrameLimiter`.
    fn reset_frame_limiter(&mut self) {
        self.g.limiter_frame_start = 0;
    }

    /// Handle the situation where the booted ELF changed.  C++:
    /// `HandleELFChange`.
    fn handle_elf_change(&mut self) {
        // TODO: ReportGameChangeToHost, Achievements::GameChanged,
        //       Patch::ReloadPatches, ApplyCoreSettings
    }

    /// Map a limiter mode to its target speed.  C++:
    /// `GetTargetSpeedForLimiterMode`.
    fn target_speed_for_limiter_mode(&self, mode: LimiterModeType) -> f32 {
        match mode {
            LimiterModeType::Nominal => self.config.inner.emulation_speed.nominal_scalar,
            LimiterModeType::Slomo => self.config.inner.emulation_speed.slomo_scalar,
            LimiterModeType::Turbo => self.config.inner.emulation_speed.turbo_scalar,
            LimiterModeType::Unlimited => 0.0,
        }
    }

    // ---- subsystem teardown (mirror of `Shutdown` ordering) ---------------

    fn shutdown_fw(&mut self) {
        // TODO: FWclose
    }
    fn shutdown_usb(&mut self) {
        // TODO: USBclose
    }
    fn shutdown_dev9(&mut self) {
        // TODO: DEV9close / DEV9shutdown
    }
    fn shutdown_pad(&mut self) {
        // TODO: Pad::Shutdown
    }
    fn shutdown_sio(&mut self) {
        // TODO: g_Sio2.Shutdown / g_Sio0.Shutdown
    }
    fn shutdown_spu2(&mut self) {
        // TODO: SPU2::Close
    }
    fn shutdown_gs(&mut self) {
        // TODO: MTGS::WaitForClose / MTGS::ResetGS
    }
    fn shutdown_memory_cards(&mut self) {
        // TODO: FileMcd_EmuClose
    }
    fn shutdown_cdvd(&mut self) {
        // TODO: DoCDVDclose / cdvdSaveNVRAM / cdvdUnlock
    }
}

// ---------------------------------------------------------------------------
// Free-function API mirroring the C++ `VMManager` namespace.
// ---------------------------------------------------------------------------

/// Number of usable save-state slots.  C++: `NUM_SAVE_STATE_SLOTS`.
pub const NUM_SAVE_STATE_SLOTS: i32 = 10;

/// Stack size to use for threads running recompilers.  C++:
/// `EMU_THREAD_STACK_SIZE`.
pub const EMU_THREAD_STACK_SIZE: usize = 2 * 1024 * 1024;

// `PerformEarlyHardwareChecks` is exposed as a free function so the host
// can call it once on startup; the C++ version returns a `bool` and writes
// a human-readable error into a `const char**`.
/// Mirror of `VMManager::PerformEarlyHardwareChecks`.
pub fn perform_early_hardware_checks() -> Result<(), String> {
    // TODO: Check for SSE4.1 / AVX2 / ARM64 page size once the host
    //       exposes CPU-feature queries.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_manager_is_shutdown() {
        let m = VmManager::new();
        assert_eq!(m.state(), VmState::Shutdown);
        assert!(!m.has_valid_vm());
        assert!(m.get_cpu().is_none());
    }

    #[test]
    fn initialize_then_shutdown_round_trip() {
        let mut m = VmManager::new();
        assert!(m.initialize(&VmBootParameters::default(), None));
        assert!(m.has_valid_vm());
        assert_eq!(m.state(), VmState::Paused);
        assert!(m.get_cpu().is_some());

        m.shutdown(false);
        assert_eq!(m.state(), VmState::Shutdown);
        assert!(!m.has_valid_vm());
        assert!(m.get_cpu().is_none());
    }

    #[test]
    fn initialize_twice_fails() {
        let mut m = VmManager::new();
        assert!(m.initialize(&VmBootParameters::default(), None));
        let mut err = Error::new("");
        assert!(!m.initialize(&VmBootParameters::default(), Some(&mut err)));
        assert!(!err.is_empty());
        m.shutdown(false);
    }

    #[test]
    fn pause_resume_transitions_state() {
        let mut m = VmManager::new();
        m.initialize(&VmBootParameters::default(), None).unwrap();
        m.boot();
        assert_eq!(m.state(), VmState::Running);
        m.pause();
        assert_eq!(m.state(), VmState::Paused);
        m.resume();
        assert_eq!(m.state(), VmState::Running);
        m.shutdown(false);
    }

    #[test]
    fn reset_cpu_keeps_vm_alive() {
        let mut m = VmManager::new();
        m.initialize(&VmBootParameters::default(), None).unwrap();
        m.g.current_crc = 0xDEADBEEF;
        m.reset_cpu();
        // reset_cpu alone should leave the VM in a valid state.
        assert!(m.has_valid_vm());
        m.shutdown(false);
    }

    #[test]
    fn reset_clears_elf_state() {
        let mut m = VmManager::new();
        m.initialize(&VmBootParameters::default(), None).unwrap();
        // Pretend an ELF was loaded.
        m.g.current_crc = 0xDEADBEEF;
        m.reset();
        assert_eq!(m.current_crc(), 0);
        m.shutdown(false);
    }

    #[test]
    fn get_cpu_returns_some_when_running() {
        let mut m = VmManager::new();
        m.initialize(&VmBootParameters::default(), None).unwrap();
        let cpu = m.get_cpu().expect("cpu should be available after init");
        // Reset on the trait object should not panic.
        let mut m2 = m;
        m2.shutdown(false);
        // Silence the unused-binding warning while still exercising
        // the trait-object deref.
        let _ = cpu.is_interrupted();
    }

    #[test]
    fn save_state_requires_path() {
        let mut m = VmManager::new();
        m.initialize(&VmBootParameters::default(), None).unwrap();
        assert!(!m.save_state(""));
        assert!(m.save_state("slot.p2s"));
        m.shutdown(false);
    }

    #[test]
    fn load_state_rejects_missing_files() {
        let mut m = VmManager::new();
        m.initialize(&VmBootParameters::default(), None).unwrap();
        assert!(!m.load_state("does-not-exist.p2s"));
        assert!(!m.load_state(""));
        m.shutdown(false);
    }

    #[test]
    fn filename_helpers_match_cpp_semantics() {
        assert!(VmManager::is_elf_file_name(Path::new("foo.ELF")));
        assert!(!VmManager::is_elf_file_name(Path::new("foo.iso")));
        assert!(VmManager::is_save_state_file_name(Path::new("slot 0.p2s")));
        assert!(!VmManager::is_save_state_file_name(Path::new("foo.p2z")));

        assert!(VmManager::is_disc_file_name(Path::new("game.iso")));
        assert!(VmManager::is_disc_file_name(Path::new("game.CHD")));
        assert!(!VmManager::is_disc_file_name(Path::new("game.zip")));

        assert!(VmManager::is_gs_dump_file_name(Path::new("frame.gs")));
        assert!(VmManager::is_gs_dump_file_name(Path::new("frame.gs.xz")));
        assert!(!VmManager::is_gs_dump_file_name(Path::new("frame.gsx")));

        assert!(VmManager::is_block_dump_file_name(Path::new("foo.dump")));

        assert!(VmManager::is_loadable_file_name(Path::new("a.iso")));
        assert!(VmManager::is_loadable_file_name(Path::new("a.elf")));
        assert!(VmManager::is_loadable_file_name(Path::new("a.gs")));
        assert!(VmManager::is_loadable_file_name(Path::new("a.dump")));
        assert!(!VmManager::is_loadable_file_name(Path::new("a.zip")));
    }

    #[test]
    fn save_state_filename_format() {
        let name = VmManager::save_state_file_name("SLUS-20001", 0xDEADBEEF, 3, false)
            .expect("non-empty serial");
        assert_eq!(
            name.to_str().unwrap(),
            "SLUS-20001 (DEADBEEF).03.p2s"
        );
        let name = VmManager::save_state_file_name("SLUS-20001", 0xDEADBEEF, -1, false)
            .expect("non-empty serial");
        assert_eq!(
            name.to_str().unwrap(),
            "SLUS-20001 (DEADBEEF).resume.p2s"
        );
        let name = VmManager::save_state_file_name("SLUS-20001", 0xDEADBEEF, 3, true)
            .expect("non-empty serial");
        assert_eq!(
            name.to_str().unwrap(),
            "SLUS-20001 (DEADBEEF).03.p2s.backup"
        );
        assert!(VmManager::save_state_file_name("", 0, 0, false).is_none());
    }
}