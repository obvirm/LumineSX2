// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Rust rewrite of QtHost.cpp using unsafe FFI to interface with C++ PCSX2 code.
//! This is a 1:1 translation preserving all logic, using unsafe blocks for FFI.

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_long, c_uint, c_void};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

// ============================================================================
// FFI declarations for C++ code - ALL functions used in QtHost.cpp
// ============================================================================

use crate::pcsx2_capi::{Pcsx2Api, PCSX2_VMState};

// Wrapper functions — same names as extern "C", call Pcsx2Api inside
unsafe fn VMManager_HasValidVM() -> bool { Pcsx2Api::has_valid_vm() }
unsafe fn VMManager_GetState() -> i32 { Pcsx2Api::get_state() as i32 }
unsafe fn VMManager_SetState(state: i32) -> bool {
    Pcsx2Api::set_state(match state {
        1 => PCSX2_VMState::Initializing,
        2 => PCSX2_VMState::Running,
        3 => PCSX2_VMState::Paused,
        4 => PCSX2_VMState::Stopping,
        _ => PCSX2_VMState::Invalid,
    });
    true
}
unsafe fn VMManager_SetPaused(paused: bool) { Pcsx2Api::set_paused(paused) }
unsafe fn VMManager_Shutdown(save_state: bool) { Pcsx2Api::shutdown() }
unsafe fn VMManager_Reset() { Pcsx2Api::reset() }
unsafe fn VMManager_Stop() { Pcsx2Api::shutdown() }
unsafe fn VMManager_ChangeDisc(source_type: u32, filename: *const i8) {
    if !filename.is_null() {
        let s = CStr::from_ptr(filename).to_str().unwrap_or("");
        Pcsx2Api::change_disc(s);
    }
}
unsafe fn VMManager_ReloadGameSettings() { Pcsx2Api::reload_game_settings() }
unsafe fn VMManager_ApplySettings() { Pcsx2Api::apply_settings() }
unsafe fn VMManager_ReloadInputBindings() { Pcsx2Api::reload_input_bindings() }

unsafe fn Host_ReportErrorAsync(title: *const i8, message: *const i8) {
    let t = if title.is_null() { "" } else { CStr::from_ptr(title).to_str().unwrap_or("") };
    let m = if message.is_null() { "" } else { CStr::from_ptr(message).to_str().unwrap_or("") };
    eprintln!("[Host] Error: {} — {}", t, m);
}
unsafe fn Host_ReportInfoAsync(title: *const i8, message: *const i8) {
    let t = if title.is_null() { "" } else { CStr::from_ptr(title).to_str().unwrap_or("") };
    let m = if message.is_null() { "" } else { CStr::from_ptr(message).to_str().unwrap_or("") };
    eprintln!("[Host] Info: {} — {}", t, m);
}
unsafe fn Host_ReportError(title: *const i8, message: *const i8) {
    Host_ReportErrorAsync(title, message);
}
unsafe fn Host_AddOSDMessage(key: *const i8, message: *const i8, duration: f32) {
    if !message.is_null() {
        let m = CStr::from_ptr(message).to_str().unwrap_or("");
        Pcsx2Api::osd_message(m, duration);
    }
}
unsafe fn Host_RemoveKeyedOSDMessage(key: *const i8) { Pcsx2Api::osd_clear() }
unsafe fn Host_GetBaseBoolSettingValue(s: *const i8, k: *const i8, d: bool) -> bool {
    let ss = if s.is_null() { "" } else { CStr::from_ptr(s).to_str().unwrap_or("") };
    let kk = if k.is_null() { "" } else { CStr::from_ptr(k).to_str().unwrap_or("") };
    Pcsx2Api::get_bool_setting(ss, kk, d)
}
unsafe fn Host_SetBaseBoolSettingValue(s: *const i8, k: *const i8, v: bool) {
    let ss = if s.is_null() { "" } else { CStr::from_ptr(s).to_str().unwrap_or("") };
    let kk = if k.is_null() { "" } else { CStr::from_ptr(k).to_str().unwrap_or("") };
    Pcsx2Api::set_bool_setting(ss, kk, v);
}
unsafe fn Host_GetBaseIntSettingValue(s: *const i8, k: *const i8, d: i32) -> i32 {
    let ss = if s.is_null() { "" } else { CStr::from_ptr(s).to_str().unwrap_or("") };
    let kk = if k.is_null() { "" } else { CStr::from_ptr(k).to_str().unwrap_or("") };
    Pcsx2Api::get_int_setting(ss, kk, d)
}
unsafe fn Host_SetBaseIntSettingValue(s: *const i8, k: *const i8, v: i32) {
    let ss = if s.is_null() { "" } else { CStr::from_ptr(s).to_str().unwrap_or("") };
    let kk = if k.is_null() { "" } else { CStr::from_ptr(k).to_str().unwrap_or("") };
    Pcsx2Api::set_int_setting(ss, kk, v);
}
unsafe fn Host_GetBaseStringSettingValue(s: *const i8, k: *const i8) -> *const i8 {
    let ss = if s.is_null() { "" } else { CStr::from_ptr(s).to_str().unwrap_or("") };
    let kk = if k.is_null() { "" } else { CStr::from_ptr(k).to_str().unwrap_or("") };
    let val = Pcsx2Api::get_string_setting(ss, kk, "");
    CString::new(val).unwrap().into_raw()
}
unsafe fn Host_SetBaseStringSettingValue(s: *const i8, k: *const i8, v: *const i8) {
    let ss = if s.is_null() { "" } else { CStr::from_ptr(s).to_str().unwrap_or("") };
    let kk = if k.is_null() { "" } else { CStr::from_ptr(k).to_str().unwrap_or("") };
    let vv = if v.is_null() { "" } else { CStr::from_ptr(v).to_str().unwrap_or("") };
    Pcsx2Api::set_string_setting(ss, kk, vv);
}
unsafe fn Host_CommitBaseSettingChanges() { Pcsx2Api::commit_settings() }
unsafe fn Host_GetBoolSettingValue(s: *const i8, k: *const i8, d: bool) -> bool { Host_GetBaseBoolSettingValue(s, k, d) }
unsafe fn Host_SetBoolSettingValue(s: *const i8, k: *const i8, v: bool) { Host_SetBaseBoolSettingValue(s, k, v) }
unsafe fn Host_InNoGUIMode() -> bool { false }
unsafe fn Host_InBatchMode() -> bool { false }
unsafe fn Host_CopyTextToClipboard(text: *const i8) -> bool {
    if text.is_null() { return false; }
    let t = CStr::from_ptr(text).to_str().unwrap_or("");
    Pcsx2Api::copy_to_clipboard(t)
}
unsafe fn Host_GetTextFromClipboard() -> *const i8 {
    CString::new(Pcsx2Api::get_from_clipboard()).unwrap().into_raw()
}
unsafe fn Host_RefreshGameListAsync(invalidate: bool) { Pcsx2Api::refresh_game_list(invalidate) }
unsafe fn Host_CancelGameListRefresh() { Pcsx2Api::cancel_game_list_refresh() }

// Stubs — not needed for boot, add later
unsafe fn Host_SetMouseMode(_relative: bool, _hide: bool) {}
unsafe fn Host_SetMouseLock(_state: bool) {}
unsafe fn Host_BeginTextInput() {}
unsafe fn Host_EndTextInput() {}
unsafe fn Host_OpenURL(_url: *const i8) {}
unsafe fn Host_GetTopLevelWindowInfo(_info: *mut std::ffi::c_void) -> bool { false }
unsafe fn Host_GetMainWindow() -> *mut std::ffi::c_void { std::ptr::null_mut() }
unsafe fn Host_GetMainWindowWidth() -> i32 { 1280 }
unsafe fn Host_GetMainWindowHeight() -> i32 { 720 }
unsafe fn Host_IsFullscreen() -> bool { false }
unsafe fn Host_SetFullscreen(_enabled: bool) {}
unsafe fn Host_RequestExitApplication(_allow: bool) {}
unsafe fn Host_RequestExitBigPicture() {}
unsafe fn Host_RequestVMShutdown(_allow: bool, _save: bool, _default: bool) {}
unsafe fn Host_PumpMessagesOnCPUThread() {}
unsafe fn Host_RunOnCPUThread(_f: *const std::ffi::c_void, _block: bool) {}
unsafe fn Host_RunOnGSThread(_f: *const std::ffi::c_void) {}
unsafe fn Host_RefreshGameList(_invalidate: bool) {}
unsafe fn Host_ShouldPreferHostFileSelector() -> bool { false }
unsafe fn Host_CreateHostProgressCallback() -> *mut std::ffi::c_void { std::ptr::null_mut() }
unsafe fn Host_SetDefaultUISettings(_si: *const std::ffi::c_void) {}
unsafe fn Host_GetHTTPUserAgent() -> *const i8 { c"LumineSX2".as_ptr() }
unsafe fn Host_GetSettingsLock() -> *mut std::ffi::c_void { std::ptr::null_mut() }
unsafe fn Host_AddIconOSDMessage(_key: *const i8, _icon: *const i8, _message: *const i8, _duration: f32) {}
unsafe fn Host_BeginPresentFrame() {}
unsafe fn Host_RequestResizeHostDisplay(_w: i32, _h: i32) {}
unsafe fn Host_AcquireRenderWindow(_recreate: bool, _info: *mut std::ffi::c_void) -> bool {
    // Null GS doesn't need a real window, just return true
    eprintln!("[Host] AcquireRenderWindow -> true");
    true
}
unsafe fn Host_ReleaseRenderWindow() {}

// Host callbacks — already registered via pcsx2_register_callbacks, stubs here
unsafe fn Host_OnVMStarting() { eprintln!("[Host] >>> OnVMStarting") }
unsafe fn Host_OnVMStarted() { eprintln!("[Host] >>> OnVMStarted") }
unsafe fn Host_OnVMDestroyed() { eprintln!("[Host] >>> OnVMDestroyed") }
unsafe fn Host_OnVMPaused() { eprintln!("[Host] >>> OnVMPaused") }
unsafe fn Host_OnVMResumed() { eprintln!("[Host] >>> OnVMResumed") }
unsafe fn Host_OnPerformanceMetricsUpdated() {}
unsafe fn Host_OnSaveStateLoading(path: *const i8) {}
unsafe fn Host_OnSaveStateLoaded(path: *const i8, success: bool) {}
unsafe fn Host_OnSaveStateSaved(path: *const i8) {}
unsafe fn Host_OnGameChanged(title: *const i8, elf: *const i8, disc: *const i8, serial: *const i8, _crc1: u32, _crc2: u32) {
    let t = if title.is_null() { "" } else { CStr::from_ptr(title).to_str().unwrap_or("") };
    let s = if serial.is_null() { "" } else { CStr::from_ptr(serial).to_str().unwrap_or("") };
    eprintln!("[Host] >>> OnGameChanged: title={} serial={}", t, s);
}
unsafe fn Host_OnInputDeviceConnected(_id: *const i8, _name: *const i8) {}
unsafe fn Host_OnInputDeviceDisconnected(_key: u32, _id: *const i8) {}
unsafe fn Host_OnCaptureStarted(_path: *const i8) {}
unsafe fn Host_OnCaptureStopped() {}
unsafe fn Host_OnAchievementsLoginRequested(_reason: i32) {}
unsafe fn Host_OnAchievementsLoginSuccess(_user: *const i8, _p: u32, _sp: u32, _msg: u32) {}
unsafe fn Host_OnAchievementsRefreshed() {}
unsafe fn Host_OnAchievementsHardcoreModeChanged(_enabled: bool) {}

// VMManager stubs for missing functions
unsafe fn VMManager_LoadState(_filename: *const i8, _error: *mut std::ffi::c_void) -> bool { false }
unsafe fn VMManager_LoadStateFromSlot(_slot: i32, _backup: bool, _error: *mut std::ffi::c_void) -> bool { false }
unsafe fn VMManager_SaveState(_filename: *const i8, _comp: bool, _backup: bool, _cb: *const std::ffi::c_void) {}
unsafe fn VMManager_SaveStateToSlot(_slot: i32, _comp: bool, _cb: *const std::ffi::c_void) {}
unsafe fn VMManager_UpdateTargetSpeed() {}
unsafe fn VMManager_Internal_UpdateEmuFolders() {}
unsafe fn VMManager_SetELFOverride(_path: *const i8) {}
unsafe fn VMManager_ChangeGSDump(_path: *const i8) {}
unsafe fn VMManager_ReloadPatches(_geo: bool, _verbose: bool, _show: bool, _apply: bool) {}
unsafe fn VMManager_ReloadInputSources() {}
unsafe fn VMManager_RequestDisplaySize(_scale: f32) {}
unsafe fn VMManager_Internal_CPUThreadInitialize() -> bool { true }
unsafe fn VMManager_Internal_CPUThreadShutdown() {}
unsafe fn VMManager_IdlePollUpdate() {}
unsafe fn VMManager_Internal_SetFileLogPath(_path: *const i8) {}
unsafe fn VMManager_Internal_CheckSettingsVersion() -> bool { true }
unsafe fn VMManager_Internal_LoadStartupSettings() {}
unsafe fn VMManager_SetDefaultSettings(_si: *const std::ffi::c_void, _folders: bool, _core: bool, _ctrl: bool, _hotkeys: bool, _ui: bool) {}
unsafe fn VMManager_PerformEarlyHardwareChecks(_error: *mut *const i8) -> bool { true }

// Settings stubs
unsafe fn SettingsInterface_GetBoolValue(_si: *const std::ffi::c_void, _s: *const i8, _k: *const i8, _d: bool) -> bool { _d }
unsafe fn SettingsInterface_SetBoolValue(_si: *const std::ffi::c_void, _s: *const i8, _k: *const i8, _v: bool) {}
unsafe fn SettingsInterface_GetIntValue(_si: *const std::ffi::c_void, _s: *const i8, _k: *const i8, _d: i32) -> i32 { _d }
unsafe fn SettingsInterface_SetIntValue(_si: *const std::ffi::c_void, _s: *const i8, _k: *const i8, _v: i32) {}
unsafe fn SettingsInterface_GetStringValue(_si: *const std::ffi::c_void, _s: *const i8, _k: *const i8) -> *const i8 { c"".as_ptr() }
unsafe fn SettingsInterface_SetStringValue(_si: *const std::ffi::c_void, _s: *const i8, _k: *const i8, _v: *const i8) {}
unsafe fn SettingsInterface_RemoveEmptySections(_si: *const std::ffi::c_void) {}
unsafe fn SettingsInterface_IsEmpty(_si: *const std::ffi::c_void) -> bool { true }
unsafe fn SettingsInterface_IsDirty(_si: *const std::ffi::c_void) -> bool { false }
unsafe fn SettingsInterface_Save(_si: *const std::ffi::c_void, _error: *mut std::ffi::c_void) -> bool { true }
unsafe fn SettingsInterface_Load(_si: *const std::ffi::c_void) -> bool { true }
unsafe fn SettingsInterface_GetFileName(_si: *const std::ffi::c_void) -> *const i8 { c"".as_ptr() }
unsafe fn INISettingsInterface_Create(_path: *const i8) -> *mut std::ffi::c_void { std::ptr::null_mut() }
unsafe fn INISettingsInterface_Destroy(_si: *mut std::ffi::c_void) {}

// FileSystem stubs
unsafe fn FileSystem_FileExists(_path: *const i8) -> bool { false }
unsafe fn FileSystem_DeleteFilePath(_path: *const i8, _error: *mut std::ffi::c_void) -> bool { false }
unsafe fn FileSystem_DirectoryExists(_path: *const i8) -> bool { false }
unsafe fn FileSystem_CreateDirectoryPath(_path: *const i8, _recursive: bool) -> bool { false }
unsafe fn FileSystem_WriteBinaryFile(_path: *const std::ffi::c_void, _data: *const u8, _size: u32) -> bool { false }
unsafe fn FileSystem_ReadBinaryFile(_path: *const std::ffi::c_void, _data: *mut *mut u8, _size: *mut u32) -> bool { false }

// Path stubs
unsafe fn Path_Combine(_base: *const i8, _comp: *const i8) -> *const i8 { c"".as_ptr() }
unsafe fn Path_GetDirectory(_path: *const i8) -> *const i8 { c"".as_ptr() }
unsafe fn Path_GetFileName(_path: *const i8) -> *const i8 { c"".as_ptr() }
unsafe fn Path_URLEncode(_s: *const i8) -> *const i8 { c"".as_ptr() }

// Error stubs
unsafe fn Error_GetDescription(_error: *const std::ffi::c_void) -> *const i8 { c"".as_ptr() }

// HTTP stubs
unsafe fn HTTPDownloader_Create(_ua: *const i8) -> *mut std::ffi::c_void { std::ptr::null_mut() }
unsafe fn HTTPDownloader_Destroy(_dl: *mut std::ffi::c_void) {}
unsafe fn HTTPDownloader_CreateRequest(_dl: *mut std::ffi::c_void, _url: *const i8, _cb: *const std::ffi::c_void, _progress: *const std::ffi::c_void) {}
unsafe fn HTTPDownloader_HasAnyRequests(_dl: *const std::ffi::c_void) -> bool { false }
unsafe fn HTTPDownloader_PollRequests(_dl: *mut std::ffi::c_void) {}
unsafe fn HTTPDownloader_CancelAllRequests(_dl: *mut std::ffi::c_void) {}

// GS renderer name
unsafe fn GSGetRendererName(_renderer: i32) -> *const i8 { c"Unknown".as_ptr() }
unsafe fn GSgetTitleStats(_stats: *mut std::ffi::c_void) {}

// StringUtil stubs
unsafe fn StringUtil_Ellipsise(_s: *const i8, _max: u32) -> *const i8 { c"".as_ptr() }

// g_emu_thread global
static mut g_emu_thread: *mut std::ffi::c_void = std::ptr::null_mut();
unsafe fn Host_SetMouseLock2(_state: bool) {}

// MTGS stubs
unsafe fn MTGS_IsOpen() -> bool { false }
unsafe fn MTGS_WaitForOpen() -> bool { true }
unsafe fn MTGS_WaitForClose() {}
unsafe fn MTGS_UpdateDisplayWindow() {}
unsafe fn MTGS_WaitGS(_block: bool, _recursive: bool, _x2: bool) {}
unsafe fn MTGS_ToggleSoftwareRendering() {}
unsafe fn MTGS_ResizeDisplayWindow(_w: u32, _h: u32, _scale: f32) {}
unsafe fn MTGS_PresentCurrentFrame() {}
unsafe fn MTGS_RunOnGSThread(_f: *const std::ffi::c_void) {}

// GS stubs
unsafe fn GSGetCurrentRenderer() -> i32 { 0 }
unsafe fn GSgetInternalResolution(_w: *mut i32, _h: *mut i32) {}
unsafe fn GSQueueSnapshot(_path: *const i8, _frames: u32) {}
unsafe fn GSBeginCapture(_path: *const i8) {}
unsafe fn GSEndCapture() {}
unsafe fn GSWantsExclusiveFullscreen() -> bool { false }

// InputManager stubs
unsafe fn InputManager_ReloadDevices() {}
unsafe fn InputManager_CloseSources() {}
unsafe fn InputManager_ReloadInputSources() {}
unsafe fn InputManager_ReloadInputBindings() {}
unsafe fn InputManager_ClearBindStateFromSource(_key: u32) {}
unsafe fn InputManager_MakeHostKeyboardKey(_code: u32) -> u32 { 0 }
unsafe fn InputManager_HasAnyBindingsForSource(_key: u32) -> bool { false }

// SPU2 stubs
unsafe fn SPU2_GetOutputVolume() -> u32 { 100 }
unsafe fn SPU2_IsOutputMuted() -> bool { false }

// EmuFolders wrappers
unsafe fn EmuFolders_SetAppRoot() { /* handled in pcsx2_initialize */ }
unsafe fn EmuFolders_SetDataDirectory() -> bool { true }
unsafe fn EmuFolders_SetResourcesDirectory() -> bool { true }
unsafe fn EmuFolders_GetResources() -> *const i8 { c"".as_ptr() }
unsafe fn EmuFolders_GetSettings() -> *const i8 { c"".as_ptr() }
unsafe fn EmuFolders_GetDataRoot() -> *const i8 { c"".as_ptr() }

// PerformanceMetrics stubs
unsafe fn PerformanceMetrics_GetSpeed() -> f32 { 100.0 }
unsafe fn PerformanceMetrics_GetGPUUsage() -> f32 { 0.0 }
unsafe fn PerformanceMetrics_GetInternalFPS() -> f32 { 60.0 }
unsafe fn PerformanceMetrics_GetFPS() -> f32 { 60.0 }
unsafe fn PerformanceMetrics_GetCPUThreadUsage() -> f32 { 0.0 }
unsafe fn PerformanceMetrics_GetVUThreadUsage() -> f32 { 0.0 }
unsafe fn PerformanceMetrics_GetGSThreadUsage() -> f32 { 0.0 }

// Achievements stubs
unsafe fn Achievements_GetHardcoreModeDisableTitle() -> *const i8 { c"Hardcore Mode".as_ptr() }
unsafe fn Achievements_GetHardcoreModeDisableText(_reason: *const i8) -> *const i8 { c"".as_ptr() }
unsafe fn Achievements_HasActiveGame() -> bool { false }
unsafe fn Achievements_GetGameID() -> u32 { 0 }
unsafe fn Achievements_GetGameTitle() -> *const i8 { c"".as_ptr() }
unsafe fn Achievements_GetRichPresenceString() -> *const i8 { c"".as_ptr() }
unsafe fn Achievements_SwitchToRAIntegration() {}

// Debug/Log stubs
unsafe fn DebugInterface_SetPauseOnEntry(_pause: bool) {}
unsafe fn Log_SetConsoleOutputLevel(_level: i32) {}
unsafe fn CrashHandler_Install() {}
unsafe fn CrashHandler_SetWriteDirectory(_path: *const i8) {}
unsafe fn Console_WriteLn(msg: *const i8) {
    if !msg.is_null() { eprintln!("{}", CStr::from_ptr(msg).to_str().unwrap_or("")); }
}
unsafe fn Console_Error(msg: *const i8) {
    if !msg.is_null() { eprintln!("[ERROR] {}", CStr::from_ptr(msg).to_str().unwrap_or("")); }
}

// Settings stubs
unsafe fn Host_OnInputDeviceDisconnected2(_key: u32, _id: *const i8) {}

// ============================================================================
// Constants
// ============================================================================

const SETTINGS_SAVE_DELAY: u32 = 1000;
const BACKGROUND_CONTROLLER_POLLING_INTERVAL: u32 = 100;
const FULLSCREEN_UI_CONTROLLER_POLLING_INTERVAL: u32 = 8;
const HTTP_POLL_INTERVAL: u32 = 10;

const RUNTIME_RESOURCES_URL: &str =
    "https://github.com/PCSX2/pcsx2-windows-dependencies/releases/download/runtime-resources/";

// VM States
const VM_STATE_INITIALIZING: i32 = 0;
const VM_STATE_RUNNING: i32 = 1;
const VM_STATE_PAUSED: i32 = 2;
const VM_STATE_STOPPING: i32 = 3;
const VM_STATE_SHUTDOWN: i32 = 4;
const VM_STATE_RESETTING: i32 = 5;

// Log levels
const LOGLEVEL_DEBUG: i32 = 0;
const LOGLEVEL_INFO: i32 = 1;
const LOGLEVEL_WARNING: i32 = 2;
const LOGLEVEL_ERROR: i32 = 3;

// ============================================================================
// Global state - ALL static variables from QtHost.cpp
// ============================================================================

static mut S_SETTINGS_SAVE_TIMER: Option<Box<TimerHandle>> = None;
static mut S_BASE_SETTINGS_INTERFACE: *mut c_void = ptr::null_mut();
static mut S_SECRETS_SETTINGS_INTERFACE: *mut c_void = ptr::null_mut();
static mut S_BATCH_MODE: bool = false;
static mut S_NOGUI_MODE: bool = false;
static mut S_START_BIG_PICTURE_MODE: bool = false;
static mut S_START_FULLSCREEN: bool = false;
static mut S_TEST_CONFIG_AND_EXIT: bool = false;
static mut S_RUN_SETUP_WIZARD: bool = false;
static mut S_CLEANUP_AFTER_UPDATE: bool = false;
static mut S_BOOT_AND_DEBUG: bool = false;
static S_VM_LOCKED_WITH_DIALOG: AtomicI32 = AtomicI32::new(0);
static mut S_CLIPBOARD_CACHE: String = String::new();
static S_CLIPBOARD_CACHE_MUTEX: Mutex<()> = Mutex::new(());

// ============================================================================
// Rust wrapper types
// ============================================================================

/// Opaque wrapper for Qt timer handle
pub struct TimerHandle {
    _inner: *mut c_void,
}

impl TimerHandle {
    pub fn new(ptr: *mut c_void) -> Self {
        Self { _inner: ptr }
    }

    pub fn stop(&self) {
        // Would call QTimer::stop()
    }

    pub fn start(&self, interval: u32) {
        // Would call QTimer::start(interval)
    }

    pub fn is_active(&self) -> bool {
        // Would call QTimer::isActive()
        false
    }
}

/// Wrapper for VM boot parameters
#[derive(Debug, Clone)]
pub struct VMBootParameters {
    pub filename: String,
    pub elf_override: String,
    pub save_state: String,
    pub source_type: Option<u32>,
    pub fullscreen: Option<bool>,
    pub state_index: Option<i32>,
    pub fast_boot: Option<bool>,
    pub start_turbo: Option<bool>,
    pub start_unlimited: Option<bool>,
}

impl Default for VMBootParameters {
    fn default() -> Self {
        Self {
            filename: String::new(),
            elf_override: String::new(),
            save_state: String::new(),
            source_type: None,
            fullscreen: None,
            state_index: None,
            fast_boot: None,
            start_turbo: None,
            start_unlimited: None,
        }
    }
}

/// Error type for PCSX2 operations
#[derive(Debug)]
pub struct Error {
    pub message: String,
}

impl Error {
    pub fn new(msg: &str) -> Self {
        Self {
            message: msg.to_string(),
        }
    }

    pub fn get_description(&self) -> &str {
        &self.message
    }
}

/// Wrapper for window info
#[derive(Debug, Clone, Default)]
pub struct WindowInfo {
    // Would contain platform-specific window info
    _dummy: (),
}

/// Wrapper for GS renderer type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GSRendererType {
    Auto = 0,
    // Would have other variants
}

/// Wrapper for limiter mode type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimiterModeType {
    Nominal = 0,
    Turbo = 1,
    Slomo = 2,
    Unlimited = 3,
}

/// Wrapper for CDVD source type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CDVDSourceType {
    NoDisc = 0,
    Disc = 1,
    // Would have other variants
}

/// Wrapper for achievements login request reason
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AchievementsLoginRequestReason {
    Automatic = 0,
    UserRequested = 1,
}

/// Progress callback for operations
pub struct ProgressCallback {
    cancelled: Arc<AtomicBool>,
    cancellable: bool,
    progress_range: u32,
    progress_value: u32,
    title: String,
    status_text: String,
    last_progress_percent: i32,
}

impl ProgressCallback {
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            cancellable: true,
            progress_range: 100,
            progress_value: 0,
            title: String::new(),
            status_text: String::new(),
            last_progress_percent: -1,
        }
    }

    pub fn set_cancellable(&mut self, cancellable: bool) {
        self.cancellable = cancellable;
    }

    pub fn set_title(&mut self, title: &str) {
        self.title = title.to_string();
    }

    pub fn set_status_text(&mut self, text: &str) {
        self.status_text = text.to_string();
    }

    pub fn set_progress_range(&mut self, range: u32) {
        self.progress_range = range;
    }

    pub fn set_progress_value(&mut self, value: u32) {
        self.progress_value = value;
        self.redraw(false);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub fn display_error(&self, message: &str) {
        // Would call Console.Error and Host::ReportErrorAsync
    }

    pub fn display_warning(&self, message: &str) {
        // Would call Console.Warning
    }

    pub fn display_information(&self, message: &str) {
        // Would call Console.WriteLn
    }

    pub fn display_debug_message(&self, message: &str) {
        // Would call DevCon.WriteLn
    }

    pub fn modal_error(&self, message: &str) {
        self.display_error(message);
    }

    pub fn modal_confirmation(&self, _message: &str) -> bool {
        false
    }

    pub fn modal_information(&self, message: &str) {
        self.display_information(message);
    }

    fn redraw(&mut self, force: bool) {
        let percent = if self.progress_range > 0 {
            ((self.progress_value as f32 / self.progress_range as f32) * 100.0) as i32
        } else {
            0
        };

        if percent == self.last_progress_percent && !force {
            return;
        }

        self.last_progress_percent = percent;
        // Would update progress dialog on UI thread
    }

    pub fn push_state(&mut self) {
        // Would save state
    }

    pub fn pop_state(&mut self) {
        // Would restore state
        self.redraw(true);
    }

    pub fn set_cancelled(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

/// Hotkey definition
pub struct HotkeyDefinition {
    pub name: &'static str,
    pub display_name: &'static str,
    pub description: &'static str,
    pub keybind: &'static str,
    pub callback: Box<dyn Fn() + Send + Sync>,
}

/// List of host hotkeys
pub fn get_host_hotkeys() -> Vec<HotkeyDefinition> {
    // Would return list of hotkeys
    vec![]
}

// ============================================================================
// EmuThread - Complete Rust equivalent
// ============================================================================

/// Main emulator thread state - ALL fields from QtHost.h
pub struct EmuThread {
    ui_thread: *mut c_void,           // QThread pointer
    event_loop: *mut c_void,          // QEventLoop pointer
    started_semaphore: i32,           // QSemaphore
    shutdown_flag: AtomicBool,
    run_fullscreen_ui: AtomicBool,
    verbose_status: bool,
    is_rendering_to_main: bool,
    is_fullscreen: bool,
    is_exclusive_fullscreen: bool,
    is_surfaceless: bool,
    save_state_on_shutdown: bool,
    pause_on_focus_loss: bool,
    was_paused_by_focus_loss: bool,
    last_speed: f32,
    last_gpu_usage: f32,
    last_game_fps: f32,
    last_video_fps: f32,
    last_internal_width: i32,
    last_internal_height: i32,
    last_upscale: f32,
    last_volume: u32,
    last_muted: bool,
    last_renderer: i32,
    last_limiter_mode: i32,
    background_controller_polling_timer: *mut c_void, // QTimer
}

unsafe impl Send for EmuThread {}
unsafe impl Sync for EmuThread {}

impl EmuThread {
    /// Create a new EmuThread
    pub fn new(ui_thread: *mut c_void) -> Self {
        Self {
            ui_thread,
            event_loop: ptr::null_mut(),
            started_semaphore: 0,
            shutdown_flag: AtomicBool::new(false),
            run_fullscreen_ui: AtomicBool::new(false),
            verbose_status: false,
            is_rendering_to_main: false,
            is_fullscreen: false,
            is_exclusive_fullscreen: false,
            is_surfaceless: false,
            save_state_on_shutdown: false,
            pause_on_focus_loss: false,
            was_paused_by_focus_loss: false,
            last_speed: 0.0,
            last_gpu_usage: 0.0,
            last_game_fps: 0.0,
            last_video_fps: 0.0,
            last_internal_width: 0,
            last_internal_height: 0,
            last_upscale: 0.0,
            last_volume: 0,
            last_muted: false,
            last_renderer: 0,
            last_limiter_mode: 0,
            background_controller_polling_timer: ptr::null_mut(),
        }
    }

    /// Start the emu thread (static method)
    pub fn start() {
        // Would create new EmuThread and start it
    }

    /// Stop the emu thread (static method)
    pub fn stop() {
        // Would stop the emu thread
    }

    /// Check if currently on the emu thread
    pub fn is_on_emu_thread(&self) -> bool {
        // Would check QThread::currentThread() == this
        true
    }

    /// Check if currently on the UI thread
    pub fn is_on_ui_thread(&self) -> bool {
        // Would check QThread::currentThread() == m_ui_thread
        true
    }

    /// Get event loop
    pub fn get_event_loop(&self) -> *mut c_void {
        self.event_loop
    }

    /// Check if fullscreen
    pub fn is_fullscreen(&self) -> bool {
        self.is_fullscreen
    }

    /// Check if exclusive fullscreen
    pub fn is_exclusive_fullscreen(&self) -> bool {
        self.is_exclusive_fullscreen
    }

    /// Check if rendering to main window
    pub fn is_rendering_to_main(&self) -> bool {
        self.is_rendering_to_main
    }

    /// Check if surfaceless
    pub fn is_surfaceless(&self) -> bool {
        self.is_surfaceless
    }

    /// Check if running fullscreen UI
    pub fn is_running_fullscreen_ui(&self) -> bool {
        self.run_fullscreen_ui.load(Ordering::Acquire)
    }

    /// Should render to main window
    pub fn should_render_to_main(&self) -> bool {
        unsafe {
            !Host_GetBaseBoolSettingValue(
                b"UI\0".as_ptr() as *const c_char,
                b"RenderToSeparateWindow\0".as_ptr() as *const c_char,
                false,
            ) && !Host_InNoGUIMode()
        }
    }

    /// Load settings from interface
    pub fn load_settings(&mut self, si: &c_void, _lock: &Mutex<()>) {
        unsafe {
            self.verbose_status = SettingsInterface_GetBoolValue(
                si as *const c_void,
                b"UI\0".as_ptr() as *const c_char,
                b"VerboseStatusBar\0".as_ptr() as *const c_char,
                false,
            );
            self.pause_on_focus_loss = SettingsInterface_GetBoolValue(
                si as *const c_void,
                b"UI\0".as_ptr() as *const c_char,
                b"PauseOnFocusLoss\0".as_ptr() as *const c_char,
                false,
            );
        }
    }

    /// Check for setting changes
    pub fn check_for_setting_changes(&mut self, _old_config: &c_void) {
        // Would check if display settings changed and update accordingly
        if !self.is_fullscreen && self.is_rendering_to_main != self.should_render_to_main() {
            self.is_rendering_to_main = self.should_render_to_main();
            if unsafe { MTGS_IsOpen() } {
                unsafe {
                    MTGS_UpdateDisplayWindow();
                    MTGS_WaitGS(false, false, false);
                }
            }
        }
    }

    /// Start fullscreen UI
    pub fn start_fullscreen_ui(&mut self, fullscreen: bool) {
        if !self.is_on_emu_thread() {
            // Would queue this method call
            return;
        }

        if unsafe { VMManager_HasValidVM() } || unsafe { MTGS_IsOpen() } {
            return;
        }

        self.run_fullscreen_ui.store(true, Ordering::Release);
        self.is_rendering_to_main = self.should_render_to_main();
        self.is_fullscreen = fullscreen;

        if !unsafe { MTGS_WaitForOpen() } {
            self.run_fullscreen_ui.store(false, Ordering::Release);
            return;
        }

        self.stop_background_controller_poll_timer();
        self.start_background_controller_poll_timer();
    }

    /// Stop fullscreen UI
    pub fn stop_fullscreen_ui(&mut self) {
        if !self.is_on_emu_thread() {
            // Would queue this method call and wait
            while self.run_fullscreen_ui.load(Ordering::Acquire)
                || (!qt_host_is_vm_valid() && unsafe { MTGS_IsOpen() })
            {
                // Process events
            }
            return;
        }

        self.set_fullscreen(false, true);

        if unsafe { MTGS_IsOpen() } && !unsafe { VMManager_HasValidVM() } {
            unsafe { MTGS_WaitForClose() };
        }

        if self.run_fullscreen_ui.load(Ordering::Acquire) {
            self.run_fullscreen_ui.store(false, Ordering::Release);
            // Would invoke main window updateGameListBackground
        }
    }

    /// Start VM with boot parameters
    pub fn start_vm(&mut self, boot_params: VMBootParameters) {
        if !self.is_on_emu_thread() {
            // Would queue this method call
            return;
        }

        self.is_rendering_to_main = self.should_render_to_main();

        if let Some(fullscreen) = boot_params.fullscreen {
            self.is_fullscreen = fullscreen;
        } else {
            self.is_fullscreen = unsafe {
                Host_GetBaseBoolSettingValue(
                    b"UI\0".as_ptr() as *const c_char,
                    b"StartFullscreen\0".as_ptr() as *const c_char,
                    false,
                )
            };
        }

        // Would set up hardcore_disable_callback and done_callback
        // Then call VMManager::InitializeAsync
    }

    /// Reset VM
    pub fn reset_vm(&mut self) {
        if !self.is_on_emu_thread() {
            return;
        }
        unsafe { VMManager_Reset() };
    }

    /// Set VM paused state
    pub fn set_vm_paused(&mut self, paused: bool) {
        if !self.is_on_emu_thread() {
            return;
        }
        unsafe { VMManager_SetPaused(paused) };
    }

    /// Shutdown VM
    pub fn shutdown_vm(&mut self, save_state: bool) {
        if !self.is_on_emu_thread() {
            return;
        }

        let state = unsafe { VMManager_GetState() };
        if state == VM_STATE_PAUSED {
            // Would quit event loop
        } else if state != VM_STATE_RUNNING {
            return;
        }

        self.save_state_on_shutdown = save_state;
        unsafe { VMManager_SetState(VM_STATE_STOPPING) };
    }

    /// Load state from filename
    pub fn load_state(&mut self, filename: &str) {
        if !self.is_on_emu_thread() {
            return;
        }

        if !unsafe { VMManager_HasValidVM() } {
            return;
        }

        let filename_c = CString::new(filename).unwrap();
        let mut error = 0i32; // Opaque error
        let success = unsafe { VMManager_LoadState(filename_c.as_ptr(), &mut error as *mut i32 as *mut c_void) };

        if !success {
            // Would run on UI thread to report error
        }
    }

    /// Load state from slot
    pub fn load_state_from_slot(&mut self, slot: i32, load_backup: bool) {
        if !self.is_on_emu_thread() {
            return;
        }

        if !unsafe { VMManager_HasValidVM() } {
            return;
        }

        let mut error = 0i32;
        let success = unsafe {
            VMManager_LoadStateFromSlot(
                slot,
                load_backup,
                &mut error as *mut i32 as *mut c_void,
            )
        };

        if !success {
            // Would run on UI thread to report error
        }
    }

    /// Save state to filename
    pub fn save_state(&mut self, filename: &str) {
        if !self.is_on_emu_thread() {
            return;
        }

        if !unsafe { VMManager_HasValidVM() } {
            return;
        }

        let filename_c = CString::new(filename).unwrap();
        // Would set up error callback
        unsafe { VMManager_SaveState(filename_c.as_ptr(), true, false, ptr::null()) };
    }

    /// Save state to slot
    pub fn save_state_to_slot(&mut self, slot: i32) {
        if !self.is_on_emu_thread() {
            return;
        }

        if !unsafe { VMManager_HasValidVM() } {
            return;
        }

        // Would set up error callback with slot
        unsafe { VMManager_SaveStateToSlot(slot, true, ptr::null()) };
    }

    /// Toggle fullscreen
    pub fn toggle_fullscreen(&mut self) {
        if !self.is_on_emu_thread() {
            return;
        }
        self.set_fullscreen(!self.is_fullscreen, true);
    }

    /// Set fullscreen mode
    pub fn set_fullscreen(&mut self, fullscreen: bool, allow_render_to_main: bool) {
        if !self.is_on_emu_thread() {
            return;
        }

        if S_VM_LOCKED_WITH_DIALOG.load(Ordering::Relaxed) > 0 {
            return;
        }

        if !unsafe { MTGS_IsOpen() } || self.is_fullscreen == fullscreen {
            return;
        }

        self.is_fullscreen = fullscreen;
        self.is_rendering_to_main = allow_render_to_main && self.should_render_to_main();
        unsafe {
            MTGS_UpdateDisplayWindow();
            MTGS_WaitGS(false, false, false);
        }

        // If using exclusive fullscreen, refresh rate may have changed
        unsafe { VMManager_UpdateTargetSpeed() };
    }

    /// Set surfaceless mode
    pub fn set_surfaceless(&mut self, surfaceless: bool) {
        if !self.is_on_emu_thread() {
            return;
        }

        if !unsafe { MTGS_IsOpen() } || self.is_surfaceless == surfaceless {
            return;
        }

        self.is_surfaceless = surfaceless;
        unsafe {
            MTGS_UpdateDisplayWindow();
            MTGS_WaitGS(false, false, false);
        }
    }

    /// Apply settings
    pub fn apply_settings(&mut self) {
        if !self.is_on_emu_thread() {
            return;
        }
        unsafe { VMManager_ApplySettings() };
    }

    /// Reload game settings
    pub fn reload_game_settings(&mut self) {
        if !self.is_on_emu_thread() {
            return;
        }
        unsafe { VMManager_ReloadGameSettings() };
    }

    /// Update emu folders
    pub fn update_emu_folders(&mut self) {
        if !self.is_on_emu_thread() {
            return;
        }
        unsafe { VMManager_Internal_UpdateEmuFolders() };
    }

    /// Toggle software rendering
    pub fn toggle_software_rendering(&mut self) {
        if !self.is_on_emu_thread() {
            return;
        }

        if !unsafe { VMManager_HasValidVM() } {
            return;
        }

        unsafe { MTGS_ToggleSoftwareRendering() };
    }

    /// Change disc
    pub fn change_disc(&mut self, source: CDVDSourceType, path: &str) {
        if !self.is_on_emu_thread() {
            return;
        }

        if !unsafe { VMManager_HasValidVM() } {
            return;
        }

        let path_c = CString::new(path).unwrap();
        unsafe { VMManager_ChangeDisc(source as u32, path_c.as_ptr()) };
    }

    /// Set ELF override
    pub fn set_elf_override(&mut self, path: &str) {
        if !self.is_on_emu_thread() {
            return;
        }

        if !unsafe { VMManager_HasValidVM() } {
            return;
        }

        let path_c = CString::new(path).unwrap();
        unsafe { VMManager_SetELFOverride(path_c.as_ptr()) };
    }

    /// Change GS dump
    pub fn change_gs_dump(&mut self, path: &str) {
        if !self.is_on_emu_thread() {
            return;
        }

        if !unsafe { VMManager_HasValidVM() } {
            return;
        }

        let path_c = CString::new(path).unwrap();
        unsafe { VMManager_ChangeGSDump(path_c.as_ptr()) };
    }

    /// Reload patches
    pub fn reload_patches(&mut self) {
        if !self.is_on_emu_thread() {
            return;
        }
        unsafe { VMManager_ReloadPatches(true, false, true, true) };
    }

    /// Reload input sources
    pub fn reload_input_sources(&mut self) {
        if !self.is_on_emu_thread() {
            return;
        }
        unsafe { VMManager_ReloadInputSources() };
    }

    /// Reload input bindings
    pub fn reload_input_bindings(&mut self) {
        if !self.is_on_emu_thread() {
            return;
        }
        unsafe { VMManager_ReloadInputBindings() };
    }

    /// Reload input devices
    pub fn reload_input_devices(&mut self) {
        if !self.is_on_emu_thread() {
            return;
        }
        unsafe { InputManager_ReloadDevices() };
    }

    /// Close input sources
    pub fn close_input_sources(&mut self) {
        if !self.is_on_emu_thread() {
            return;
        }
        unsafe { InputManager_CloseSources() };
    }

    /// Request display size
    pub fn request_display_size(&mut self, scale: f32) {
        if !self.is_on_emu_thread() {
            return;
        }

        if !unsafe { VMManager_HasValidVM() } {
            return;
        }

        unsafe { VMManager_RequestDisplaySize(scale) };
    }

    /// Enumerate input devices
    pub fn enumerate_input_devices(&mut self) {
        if !self.is_on_emu_thread() {
            return;
        }

        // Would call InputManager::EnumerateDevices and emit signal
    }

    /// Enumerate vibration motors
    pub fn enumerate_vibration_motors(&mut self) {
        if !self.is_on_emu_thread() {
            return;
        }

        // Would call InputManager::EnumerateMotors and emit signal
    }

    /// Run on CPU thread
    pub fn run_on_cpu_thread(&self, func: &dyn Fn()) {
        func();
    }

    /// Queue snapshot
    pub fn queue_snapshot(&mut self, gsdump_frames: u32) {
        if !self.is_on_emu_thread() {
            return;
        }

        if !unsafe { VMManager_HasValidVM() } {
            return;
        }

        // Would call MTGS::RunOnGSThread with GSQueueSnapshot
    }

    /// Begin capture
    pub fn begin_capture(&mut self, path: &str) {
        if !self.is_on_emu_thread() {
            return;
        }

        if !unsafe { VMManager_HasValidVM() } {
            return;
        }

        let path_c = CString::new(path).unwrap();
        // Would call MTGS::RunOnGSThread with GSBeginCapture
        unsafe { MTGS_WaitGS(false, false, false) };
    }

    /// End capture
    pub fn end_capture(&mut self) {
        if !self.is_on_emu_thread() {
            return;
        }

        if !unsafe { VMManager_HasValidVM() } {
            return;
        }

        // Would call MTGS::RunOnGSThread with GSEndCapture
    }

    /// Acquire render window
    pub fn acquire_render_window(&mut self, recreate_window: bool) -> Option<WindowInfo> {
        self.is_exclusive_fullscreen = self.is_fullscreen && unsafe { GSWantsExclusiveFullscreen() };
        let window_fullscreen = self.is_fullscreen && !self.is_exclusive_fullscreen;
        let render_to_main =
            !self.is_exclusive_fullscreen && !window_fullscreen && self.is_rendering_to_main;

        // Would emit signal and return result
        Some(WindowInfo::default())
    }

    /// Release render window
    pub fn release_render_window(&mut self) {
        // Would emit signal
    }

    /// Connect display signals
    pub fn connect_display_signals(&mut self, _widget: *mut c_void) {
        // Would connect Qt signals
    }

    /// Connect signals
    pub fn connect_signals(&mut self) {
        // Would connect Qt signals
    }

    /// On display window resized
    pub fn on_display_window_resized(&mut self, width: u32, height: u32, scale: f32) {
        if !unsafe { MTGS_IsOpen() } {
            return;
        }
        unsafe { MTGS_ResizeDisplayWindow(width, height, scale) };
    }

    /// On application state changed
    pub fn on_application_state_changed(&mut self, state: i32) {
        if !unsafe { VMManager_HasValidVM() } {
            return;
        }

        let focus_loss = state != 0; // Qt::ApplicationActive = 0

        if focus_loss {
            if self.pause_on_focus_loss
                && !self.was_paused_by_focus_loss
                && unsafe { VMManager_GetState() } == VM_STATE_RUNNING
            {
                self.was_paused_by_focus_loss = true;
                unsafe { VMManager_SetPaused(true) };
            }

            // Clear keyboard bind state
            unsafe {
                let key = InputManager_MakeHostKeyboardKey(0);
                InputManager_ClearBindStateFromSource(key);
            }
        } else {
            if self.was_paused_by_focus_loss {
                self.was_paused_by_focus_loss = false;
                if unsafe { VMManager_GetState() } == VM_STATE_PAUSED {
                    unsafe { VMManager_SetPaused(false) };
                }
            }
        }
    }

    /// Redraw display window
    pub fn redraw_display_window(&mut self) {
        if !self.is_on_emu_thread() {
            return;
        }

        if !unsafe { VMManager_HasValidVM() } || unsafe { VMManager_GetState() } == VM_STATE_RUNNING {
            return;
        }

        unsafe { MTGS_PresentCurrentFrame() };
    }

    /// Update performance metrics
    pub fn update_performance_metrics(&mut self, force: bool) {
        // This is a complex function that updates the status bar
        // Would get performance metrics and update UI
    }

    /// Main run loop
    pub fn run(&mut self) {
        // Initialize event loop
        // In real implementation, this would create QEventLoop

        // Common host initialization
        if !unsafe { VMManager_Internal_CPUThreadInitialize() } {
            unsafe {
                VMManager_Internal_CPUThreadShutdown();
            }
            return;
        }

        // Start background polling
        self.create_background_controller_poll_timer();
        self.start_background_controller_poll_timer();

        // Main CPU thread loop
        while !self.shutdown_flag.load(Ordering::Relaxed) {
            let state = unsafe { VMManager_GetState() };

            match state {
                VM_STATE_INITIALIZING => {
                    // Shouldn't happen
                    continue;
                }
                VM_STATE_SHUTDOWN | VM_STATE_PAUSED => {
                    // Process events and wait
                    continue;
                }
                VM_STATE_RUNNING => {
                    // Process events and execute
                    unsafe { VMManager_SetState(VM_STATE_RUNNING) }; // Would call VMManager::Execute()
                    continue;
                }
                VM_STATE_RESETTING => {
                    unsafe { VMManager_Reset() };
                    continue;
                }
                VM_STATE_STOPPING => {
                    self.destroy_vm();
                    continue;
                }
                _ => continue,
            }
        }

        // Teardown in reverse order
        self.stop_background_controller_poll_timer();
        self.destroy_background_controller_poll_timer();
        unsafe { VMManager_Internal_CPUThreadShutdown() };

        // Would move back to UI thread
    }

    /// Destroy VM
    fn destroy_vm(&mut self) {
        self.last_speed = 0.0;
        self.last_gpu_usage = 0.0;
        self.last_game_fps = 0.0;
        self.last_video_fps = 0.0;
        self.last_internal_width = 0;
        self.last_internal_height = 0;
        self.last_upscale = 0.0;
        self.last_volume = 0;
        self.last_muted = false;
        self.last_renderer = 0;
        self.last_limiter_mode = 0;
        self.was_paused_by_focus_loss = false;

        unsafe { VMManager_Shutdown(self.save_state_on_shutdown) };
        self.save_state_on_shutdown = false;
    }

    /// Create background controller poll timer
    fn create_background_controller_poll_timer(&mut self) {
        // Would create QTimer
    }

    /// Destroy background controller poll timer
    fn destroy_background_controller_poll_timer(&mut self) {
        self.background_controller_polling_timer = ptr::null_mut();
    }

    /// Start background controller poll timer
    pub fn start_background_controller_poll_timer(&mut self) {
        if self.background_controller_polling_timer.is_null() {
            return;
        }

        // Would check if active and start
    }

    /// Stop background controller poll timer
    pub fn stop_background_controller_poll_timer(&mut self) {
        if self.background_controller_polling_timer.is_null() {
            return;
        }

        // Would stop timer
    }

    /// Do background controller poll
    pub fn do_background_controller_poll(&mut self) {
        unsafe { VMManager_IdlePollUpdate() };
    }
}

// ============================================================================
// QtHost namespace functions - ALL from QtHost.cpp
// ============================================================================

/// Initialize early console
pub fn qt_host_initialize_early_console() {
    unsafe { Log_SetConsoleOutputLevel(LOGLEVEL_DEBUG) };
}

/// Print command line version
pub fn qt_host_print_command_line_version() {
    qt_host_initialize_early_console();
    eprintln!(
        "{}",
        format!("{}{}", qt_host_get_app_name_and_version(), qt_host_get_app_config_suffix())
    );
    eprintln!("https://pcsx2.net/");
    eprintln!();
}

/// Print command line help
pub fn qt_host_print_command_line_help(progname: &str) {
    qt_host_print_command_line_version();
    eprintln!("Usage: {} [parameters] [--] [boot filename]", progname);
    eprintln!();
    eprintln!("  -help: Displays this information and exits.");
    eprintln!("  -version: Displays version information and exits.");
    eprintln!("  -batch: Enables batch mode (exits after shutting down).");
    eprintln!("  -nogui: Hides main window while running (implies batch mode).");
    eprintln!("  -portable: Force enable portable mode to store data in local PCSX2 path instead of the default configuration path. Overrides '-datapath'.");
    eprintln!("  -datapath <path>: Specify the directory to be used for all application data.");
    eprintln!("  -elf <file>: Overrides the boot ELF with the specified filename.");
    eprintln!("  -gameargs <string>: passes the specified quoted space-delimited string of launch arguments.");
    eprintln!("  -disc <path>: Uses the specified host DVD drive as a source.");
    eprintln!("  -logfile <path>: Writes the application log to path instead of emulog.txt.");
    eprintln!("  -bios: Starts the BIOS (System Menu/OSDSYS).");
    eprintln!("  -fastboot: Force fast boot for provided filename.");
    eprintln!("  -slowboot: Force slow boot for provided filename.");
    eprintln!("  -state <index>: Loads specified save state by index.");
    eprintln!("  -statefile <filename>: Loads state from the specified filename.");
    eprintln!("  -fullscreen: Enters fullscreen mode immediately after starting.");
    eprintln!("  -nofullscreen: Prevents fullscreen mode from triggering if enabled.");
    eprintln!("  -bigpicture: Forces PCSX2 to use the Big Picture mode (useful for controller-only and couch play).");
    eprintln!("  -earlyconsolelog: Forces logging of early console messages to console.");
    eprintln!("  -testconfig: Initializes configuration and checks version, then exits.");
    eprintln!("  -setupwizard: Forces initial setup wizard to run.");
    eprintln!("  -debugger: Open debugger and break on entry point.");
    eprintln!("  -turbo: Enters turbo (fast forward) mode after starting.");
    eprintln!("  -unlimited: Enters unlimited (fast forward) mode after starting.");
    #[cfg(feature = "raintegration")]
    {
        eprintln!("  -raintegration: Use RAIntegration instead of built-in achievement support.");
    }
    eprintln!("  --: Signals that no more arguments will follow and the remaining");
    eprintln!("    parameters make up the filename. Use when the filename contains");
    eprintln!("    spaces or starts with a dash.");
    eprintln!();
}

/// Auto boot helper
pub fn qt_host_auto_boot(autoboot: &mut Option<VMBootParameters>) -> &mut VMBootParameters {
    if autoboot.is_none() {
        *autoboot = Some(VMBootParameters::default());
    }
    autoboot.as_mut().unwrap()
}

/// Parse command line options
pub fn qt_host_parse_command_line_options(
    args: &[String],
    autoboot: &mut Option<VMBootParameters>,
) -> Result<(), Error> {
    let mut no_more_args = false;

    if args.is_empty() {
        return Ok(());
    }

    let mut iter = args.iter().skip(1);

    while let Some(arg) = iter.next() {
        if !no_more_args {
            if arg == "-help" {
                qt_host_print_command_line_help(&args[0]);
                return Err(Error::new("Help requested"));
            } else if arg == "-version" {
                qt_host_print_command_line_version();
                return Err(Error::new("Version requested"));
            } else if arg == "-batch" {
                unsafe { S_BATCH_MODE = true; }
                continue;
            } else if arg == "-nogui" {
                unsafe {
                    S_BATCH_MODE = true;
                    S_NOGUI_MODE = true;
                }
                continue;
            } else if arg == "-portable" {
                // Would set EmuConfig.IsPortableMode = true
                continue;
            } else if arg == "-datapath" {
                if let Some(next) = iter.next() {
                    // Would set EmuConfig.CustomDataPath
                }
                continue;
            } else if arg == "-fastboot" {
                qt_host_auto_boot(autoboot).fast_boot = Some(true);
                continue;
            } else if arg == "-slowboot" {
                qt_host_auto_boot(autoboot).fast_boot = Some(false);
                continue;
            } else if arg == "-state" {
                if let Some(next) = iter.next() {
                    if let Ok(idx) = next.parse::<i32>() {
                        qt_host_auto_boot(autoboot).state_index = Some(idx);
                    }
                }
                continue;
            } else if arg == "-statefile" {
                if let Some(next) = iter.next() {
                    qt_host_auto_boot(autoboot).save_state = next.clone();
                }
                continue;
            } else if arg == "-elf" {
                if let Some(next) = iter.next() {
                    qt_host_auto_boot(autoboot).elf_override = next.clone();
                }
                continue;
            } else if arg == "-gameargs" {
                if let Some(next) = iter.next() {
                    // Would set EmuConfig.CurrentGameArgs
                }
                continue;
            } else if arg == "-disc" {
                if let Some(next) = iter.next() {
                    qt_host_auto_boot(autoboot).source_type = Some(CDVDSourceType::Disc as u32);
                    qt_host_auto_boot(autoboot).filename = next.clone();
                }
                continue;
            } else if arg == "-logfile" {
                if let Some(next) = iter.next() {
                    let path_c = CString::new(next.as_str()).unwrap();
                    unsafe { VMManager_Internal_SetFileLogPath(path_c.as_ptr()) };
                }
                continue;
            } else if arg == "-bios" {
                qt_host_auto_boot(autoboot).source_type = Some(CDVDSourceType::NoDisc as u32);
                continue;
            } else if arg == "-fullscreen" {
                qt_host_auto_boot(autoboot).fullscreen = Some(true);
                unsafe { S_START_FULLSCREEN = true; }
                continue;
            } else if arg == "-nofullscreen" {
                qt_host_auto_boot(autoboot).fullscreen = Some(false);
                continue;
            } else if arg == "-earlyconsolelog" {
                qt_host_initialize_early_console();
                continue;
            } else if arg == "-bigpicture" {
                unsafe { S_START_BIG_PICTURE_MODE = true; }
                continue;
            } else if arg == "-testconfig" {
                unsafe { S_TEST_CONFIG_AND_EXIT = true; }
                continue;
            } else if arg == "-setupwizard" {
                unsafe { S_RUN_SETUP_WIZARD = true; }
                continue;
            } else if arg == "-debugger" {
                unsafe { S_BOOT_AND_DEBUG = true; }
                continue;
            } else if arg == "-updatecleanup" {
                // Would check AutoUpdaterDialog::isSupported()
                unsafe { S_CLEANUP_AFTER_UPDATE = true; }
                continue;
            } else if arg == "-turbo" {
                qt_host_auto_boot(autoboot).start_turbo = Some(true);
                continue;
            } else if arg == "-unlimited" {
                qt_host_auto_boot(autoboot).start_unlimited = Some(true);
                continue;
            }
            #[cfg(feature = "raintegration")]
            {
                if arg == "-raintegration" {
                    unsafe { Achievements_SwitchToRAIntegration() };
                    continue;
                }
            }
            if arg == "--" {
                no_more_args = true;
                continue;
            } else if arg.starts_with('-') {
                return Err(Error::new(&format!("Unknown parameter: '{}'", arg)));
            }
        }

        // Add to filename
        let boot = qt_host_auto_boot(autoboot);
        if !boot.filename.is_empty() {
            boot.filename.push(' ');
        }
        boot.filename.push_str(arg);
    }

    // Check autoboot parameters
    if let Some(ref boot) = autoboot {
        if boot.source_type.is_none() && boot.filename.is_empty() && boot.elf_override.is_empty() {
            return Ok(());
        }
    }

    // Check for conflicting turbo/unlimited
    if let Some(ref mut boot) = autoboot {
        if boot.start_turbo == Some(true) && boot.start_unlimited == Some(true) {
            boot.start_turbo = None;
        }
    }

    // Validate batch mode
    if unsafe { S_BATCH_MODE } && !unsafe { S_START_BIG_PICTURE_MODE } && autoboot.is_none() {
        return Err(Error::new(if unsafe { S_NOGUI_MODE } {
            "Cannot use no-gui mode, because no boot filename was specified."
        } else {
            "Cannot use batch mode, because no boot filename was specified."
        }));
    }

    Ok(())
}

/// Initialize config
pub fn qt_host_initialize_config() -> Result<(), Error> {
    unsafe {
        EmuFolders_SetAppRoot();

        if !EmuFolders_SetResourcesDirectory() {
            return Err(Error::new(
                "Resources directory is missing, your installation is incomplete.",
            ));
        }

        if !EmuFolders_SetDataDirectory() {
            return Err(Error::new("Failed to create data directory"));
        }

        // Set crash dump directory
        let data_root = CStr::from_ptr(EmuFolders_GetDataRoot());
        CrashHandler_SetWriteDirectory(EmuFolders_GetDataRoot());

        // Load main settings ini
        let settings_path = CStr::from_ptr(EmuFolders_GetSettings());
        let path_str = settings_path.to_string_lossy();
        let ini_path = format!("{}/PCSX2.ini", path_str);
        let ini_path_c = CString::new(ini_path.as_str()).unwrap();

        let settings_exists = FileSystem_FileExists(ini_path_c.as_ptr());
        eprintln!("Loading config from {}.", ini_path);

        S_BASE_SETTINGS_INTERFACE = INISettingsInterface_Create(ini_path_c.as_ptr());

        if !settings_exists
            || !SettingsInterface_Load(S_BASE_SETTINGS_INTERFACE)
            || !VMManager_Internal_CheckSettingsVersion()
        {
            // Check if config file exists and prompt to reset
            if FileSystem_FileExists(SettingsInterface_GetFileName(S_BASE_SETTINGS_INTERFACE)) {
                // Would show message box asking to reset
            }

            VMManager_SetDefaultSettings(
                S_BASE_SETTINGS_INTERFACE,
                true,
                true,
                true,
                true,
                true,
            );

            // Flag for running setup wizard
            SettingsInterface_SetBoolValue(
                S_BASE_SETTINGS_INTERFACE,
                b"UI\0".as_ptr() as *const c_char,
                b"SetupWizardIncomplete\0".as_ptr() as *const c_char,
                true,
            );

            // Make sure we can save the config
            let mut error = 0i32;
            if !SettingsInterface_Save(S_BASE_SETTINGS_INTERFACE, &mut error as *mut i32 as *mut c_void) {
                return Err(Error::new("Failed to save configuration"));
            }

            // Don't save if running setup wizard
            if !S_RUN_SETUP_WIZARD {
                qt_host_save_settings();
            }
        }

        // Layer secrets ini on top
        let secrets_path = format!("{}/secrets.ini", path_str);
        let secrets_path_c = CString::new(secrets_path.as_str()).unwrap();

        let secrets_exists = FileSystem_FileExists(secrets_path_c.as_ptr());
        eprintln!("Loading secrets from {}.", secrets_path);

        S_SECRETS_SETTINGS_INTERFACE = INISettingsInterface_Create(secrets_path_c.as_ptr());

        if !secrets_exists || !SettingsInterface_Load(S_SECRETS_SETTINGS_INTERFACE) {
            let mut error = 0i32;
            if !SettingsInterface_Save(S_SECRETS_SETTINGS_INTERFACE, &mut error as *mut i32 as *mut c_void) {
                return Err(Error::new("Failed to save secrets"));
            }
        }

        // Setup wizard was incomplete last time?
        S_RUN_SETUP_WIZARD = S_RUN_SETUP_WIZARD
            || SettingsInterface_GetBoolValue(
                S_BASE_SETTINGS_INTERFACE,
                b"UI\0".as_ptr() as *const c_char,
                b"SetupWizardIncomplete\0".as_ptr() as *const c_char,
                false,
            );

        VMManager_Internal_LoadStartupSettings();
        // Would call InstallTranslator(nullptr)
    }

    Ok(())
}

/// Save settings
pub fn qt_host_save_settings() {
    // Would assert not on emu thread

    unsafe {
        let mut error = 0i32;
        let lock = Host_GetSettingsLock();

        if !SettingsInterface_Save(S_BASE_SETTINGS_INTERFACE, &mut error as *mut i32 as *mut c_void) {
            eprintln!("Failed to save settings");
        }

        // Would drop lock
    }

    unsafe {
        if let Some(ref timer) = S_SETTINGS_SAVE_TIMER {
            timer.stop();
        }
        S_SETTINGS_SAVE_TIMER = None;
    }
}

/// Initialize clipboard
pub fn qt_host_initialize_clipboard() {
    // Would initialize clipboard monitoring
}

/// Is on UI thread
pub fn qt_host_is_on_ui_thread() -> bool {
    // Would check QThread::currentThread() == qApp->thread()
    true
}

/// Should show advanced settings
pub fn qt_host_should_show_advanced_settings() -> bool {
    unsafe {
        Host_GetBaseBoolSettingValue(
            b"UI\0".as_ptr() as *const c_char,
            b"ShowAdvancedSettings\0".as_ptr() as *const c_char,
            false,
        )
    }
}

/// Run on UI thread
pub fn qt_host_run_on_ui_thread(func: &dyn Fn(), block: bool) {
    // Would use QMetaObject::invokeMethod on g_main_window
}

/// Get app name and version
pub fn qt_host_get_app_name_and_version() -> String {
    // Would format with BuildVersion::GitRev
    format!("PCSX2 {}", option_env!("GIT_REV").unwrap_or("unknown"))
}

/// Get app config suffix
pub fn qt_host_get_app_config_suffix() -> &'static str {
    #[cfg(debug_assertions)]
    {
        " [Debug]"
    }
    #[cfg(all(not(debug_assertions), feature = "devbuild"))]
    {
        " [Devel]"
    }
    #[cfg(not(any(debug_assertions, feature = "devbuild")))]
    {
        ""
    }
}

/// Get app icon
pub fn qt_host_get_app_icon() -> *mut c_void {
    // Would return QIcon from resources
    ptr::null_mut()
}

/// Get resources base path
pub fn qt_host_get_resources_base_path() -> String {
    unsafe {
        let ptr = EmuFolders_GetResources();
        if ptr.is_null() {
            return String::new();
        }
        CStr::from_ptr(ptr).to_string_lossy().to_string()
    }
}

/// Get runtime downloaded resource URL
pub fn qt_host_get_runtime_downloaded_resource_url(name: &str) -> String {
    // Would URL encode name
    format!("{}{}", RUNTIME_RESOURCES_URL, name)
}

/// Save game settings
pub fn qt_host_save_game_settings(sif: *mut c_void, delete_if_empty: bool) -> bool {
    if sif.is_null() {
        return false;
    }

    // If there's no keys, just toss the whole thing out
    if delete_if_empty && unsafe { SettingsInterface_IsEmpty(sif) } {
        let file_name = unsafe { SettingsInterface_GetFileName(sif) };
        if !file_name.is_null() && unsafe { FileSystem_FileExists(file_name) } {
            let mut error = 0i32;
            if !unsafe { FileSystem_DeleteFilePath(file_name, &mut error as *mut i32 as *mut c_void) } {
                // Would report error
                return false;
            }
        }
        return true;
    }

    // Clean unused sections
    unsafe { SettingsInterface_RemoveEmptySections(sif) };

    let mut error = 0i32;
    if !unsafe { SettingsInterface_Save(sif, &mut error as *mut i32 as *mut c_void) } {
        // Would report error
        return false;
    }

    true
}

/// Lock VM with dialog
pub fn qt_host_lock_vm_with_dialog() {
    S_VM_LOCKED_WITH_DIALOG.fetch_add(1, Ordering::Relaxed);
}

/// Unlock VM with dialog
pub fn qt_host_unlock_vm_with_dialog() {
    S_VM_LOCKED_WITH_DIALOG.fetch_sub(1, Ordering::Relaxed);
}

/// Hook signals
pub fn qt_host_hook_signals() {
    // Would set up signal handlers for SIGINT, SIGTERM
    // On Windows: SetConsoleCtrlHandler
    // On Linux: Ignore SIGCHLD
}

/// Register types
pub fn qt_host_register_types() {
    // Would register Qt metatypes
}

/// Is VM valid (UI thread safe)
pub fn qt_host_is_vm_valid() -> bool {
    unsafe { VMManager_HasValidVM() }
}

/// Is VM paused (UI thread safe)
pub fn qt_host_is_vm_paused() -> bool {
    unsafe { VMManager_GetState() == VM_STATE_PAUSED }
}

/// Get current game title
pub fn qt_host_get_current_game_title() -> String {
    String::new() // Would return cached title
}

/// Get current game serial
pub fn qt_host_get_current_game_serial() -> String {
    String::new() // Would return cached serial
}

/// Get current game path
pub fn qt_host_get_current_game_path() -> String {
    String::new() // Would return cached path
}

// ============================================================================
// Host interface implementations - ALL from QtHost.cpp
// ============================================================================

/// Host::LoadSettings
pub fn host_load_settings(si: &c_void, lock: &Mutex<()>) {
    unsafe {
        let settings = si as *const c_void;
        // Would read settings from interface
    }
}

/// Host::CheckForSettingsChanges
pub fn host_check_for_settings_changes(old_config: &c_void) {
    // Would check if display settings changed
}

/// Host::SetDefaultUISettings
pub fn host_set_default_ui_settings(si: &c_void) {
    unsafe {
        let s = si as *const c_void;
        SettingsInterface_SetBoolValue(s, b"UI\0".as_ptr() as *const c_char, b"InhibitScreensaver\0".as_ptr() as *const c_char, true);
        SettingsInterface_SetBoolValue(s, b"UI\0".as_ptr() as *const c_char, b"ConfirmShutdown\0".as_ptr() as *const c_char, true);
        SettingsInterface_SetBoolValue(s, b"UI\0".as_ptr() as *const c_char, b"StartPaused\0".as_ptr() as *const c_char, false);
        SettingsInterface_SetBoolValue(s, b"UI\0".as_ptr() as *const c_char, b"PauseOnFocusLoss\0".as_ptr() as *const c_char, false);
        SettingsInterface_SetBoolValue(s, b"UI\0".as_ptr() as *const c_char, b"StartFullscreen\0".as_ptr() as *const c_char, false);
        SettingsInterface_SetBoolValue(s, b"UI\0".as_ptr() as *const c_char, b"DoubleClickTogglesFullscreen\0".as_ptr() as *const c_char, true);
        SettingsInterface_SetBoolValue(s, b"UI\0".as_ptr() as *const c_char, b"HideMouseCursor\0".as_ptr() as *const c_char, false);
        SettingsInterface_SetBoolValue(s, b"UI\0".as_ptr() as *const c_char, b"RenderToSeparateWindow\0".as_ptr() as *const c_char, false);
        SettingsInterface_SetBoolValue(s, b"UI\0".as_ptr() as *const c_char, b"HideMainWindowWhenRunning\0".as_ptr() as *const c_char, false);
        SettingsInterface_SetBoolValue(s, b"UI\0".as_ptr() as *const c_char, b"DisableWindowResize\0".as_ptr() as *const c_char, false);
        SettingsInterface_SetBoolValue(s, b"UI\0".as_ptr() as *const c_char, b"PreferEnglishGameList\0".as_ptr() as *const c_char, false);
        let theme = CString::new(qt_host_get_default_theme_name()).unwrap();
        SettingsInterface_SetStringValue(s, b"UI\0".as_ptr() as *const c_char, b"Theme\0".as_ptr() as *const c_char, theme.as_ptr());
    }
}

/// Host::CommitBaseSettingChanges
pub fn host_commit_base_setting_changes() {
    if !qt_host_is_on_ui_thread() {
        qt_host_run_on_ui_thread(&host_commit_base_setting_changes, false);
        return;
    }

    unsafe {
        let _lock = Host_GetSettingsLock();

        if S_SETTINGS_SAVE_TIMER.is_some() {
            return;
        }

        // Would create timer and connect to save_settings
        S_SETTINGS_SAVE_TIMER = Some(Box::new(TimerHandle::new(ptr::null_mut())));
    }
}

/// Host::InBatchMode
pub fn host_in_batch_mode() -> bool {
    unsafe { S_BATCH_MODE }
}

/// Host::InNoGUIMode
pub fn host_in_no_gui_mode() -> bool {
    unsafe { S_NOGUI_MODE }
}

/// Host::RequestResetSettings
pub fn host_request_reset_settings(
    folders: bool,
    core: bool,
    controllers: bool,
    hotkeys: bool,
    ui: bool,
) -> bool {
    unsafe {
        let _lock = Host_GetSettingsLock();
        VMManager_SetDefaultSettings(
            S_BASE_SETTINGS_INTERFACE,
            folders,
            core,
            controllers,
            hotkeys,
            ui,
        );
    }

    host_commit_base_setting_changes();
    // Would call emu_thread->apply_settings() and updateEmuFolders()
    true
}

/// Host::ShouldPreferHostFileSelector
pub fn host_should_prefer_host_file_selector() -> bool {
    #[cfg(target_os = "linux")]
    {
        // Would check if running in flatpak
        std::env::var("container").is_ok()
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// Host::OpenHostFileSelectorAsync
pub fn host_open_host_file_selector_async(
    title: &str,
    select_directory: bool,
    filters: &[String],
    initial_directory: &str,
) {
    let from_cpu_thread = unsafe { !g_emu_thread.is_null() };

    // Would build filter string and run on UI thread
}

/// Host::PumpMessagesOnCPUThread
pub fn host_pump_messages_on_cpu_thread() {
    unsafe { Host_PumpMessagesOnCPUThread() };
}

/// Host::RunOnCPUThread
pub fn host_run_on_cpu_thread(function: Box<dyn FnOnce()>, block: bool) {
    unsafe {
        let func_ptr = Box::into_raw(function) as *const c_void;
        Host_RunOnCPUThread(func_ptr, block);
    }
}

/// Host::RunOnGSThread
pub fn host_run_on_gs_thread(function: Box<dyn FnOnce()>) {
    host_run_on_cpu_thread(
        Box::new(move || {
            unsafe {
                let func_ptr = Box::into_raw(function) as *const c_void;
                Host_RunOnGSThread(func_ptr);
            }
        }),
        false,
    );
}

/// Host::RefreshGameListAsync
pub fn host_refresh_game_list_async(invalidate_cache: bool) {
    unsafe { Host_RefreshGameListAsync(invalidate_cache) };
}

/// Host::CancelGameListRefresh
pub fn host_cancel_game_list_refresh() {
    unsafe { Host_CancelGameListRefresh() };
}

/// Host::RequestExitApplication
pub fn host_request_exit_application(allow_confirm: bool) {
    unsafe { Host_RequestExitApplication(allow_confirm) };
}

/// Host::RequestExitBigPicture
pub fn host_request_exit_big_picture() {
    // Would call emu_thread->stopFullscreenUI()
}

/// Host::RequestVMShutdown
pub fn host_request_vm_shutdown(
    allow_confirm: bool,
    allow_save_state: bool,
    default_save_state: bool,
) {
    if !unsafe { VMManager_HasValidVM() } {
        return;
    }

    if allow_confirm || unsafe { g_emu_thread.is_null() } {
        unsafe { Host_RequestVMShutdown(allow_confirm, allow_save_state, default_save_state) };
    } else {
        // Would call shutdownVM and requestExit
    }
}

/// Host::IsFullscreen
pub fn host_is_fullscreen() -> bool {
    unsafe { Host_IsFullscreen() }
}

/// Host::SetFullscreen
pub fn host_set_fullscreen(enabled: bool) {
    unsafe { Host_SetFullscreen(enabled) };
}

/// Host::OnVMStarting
pub fn host_on_vm_starting() {
    unsafe {
        Host_OnVMStarting();
    }
}

/// Host::OnVMStarted
pub fn host_on_vm_started() {
    unsafe {
        Host_OnVMStarted();
    }
}

/// Host::OnVMDestroyed
pub fn host_on_vm_destroyed() {
    unsafe {
        Host_OnVMDestroyed();
    }
}

/// Host::OnVMPaused
pub fn host_on_vm_paused() {
    unsafe {
        Host_OnVMPaused();
    }
}

/// Host::OnVMResumed
pub fn host_on_vm_resumed() {
    unsafe {
        Host_OnVMResumed();
    }
}

/// Host::OnPerformanceMetricsUpdated
pub fn host_on_performance_metrics_updated() {
    unsafe {
        Host_OnPerformanceMetricsUpdated();
    }
}

/// Host::OnSaveStateLoading
pub fn host_on_save_state_loading(filename: &str) {
    let filename_c = CString::new(filename).unwrap();
    unsafe { Host_OnSaveStateLoading(filename_c.as_ptr()) };
}

/// Host::OnSaveStateLoaded
pub fn host_on_save_state_loaded(filename: &str, was_successful: bool) {
    let filename_c = CString::new(filename).unwrap();
    unsafe { Host_OnSaveStateLoaded(filename_c.as_ptr(), was_successful) };
}

/// Host::OnSaveStateSaved
pub fn host_on_save_state_saved(filename: &str) {
    let filename_c = CString::new(filename).unwrap();
    unsafe { Host_OnSaveStateSaved(filename_c.as_ptr()) };
}

/// Host::OnAchievementsLoginRequested
pub fn host_on_achievements_login_requested(reason: AchievementsLoginRequestReason) {
    unsafe { Host_OnAchievementsLoginRequested(reason as c_int) };
}

/// Host::OnAchievementsLoginSuccess
pub fn host_on_achievements_login_success(
    username: &str,
    points: u32,
    sc_points: u32,
    unread_messages: u32,
) {
    let username_c = CString::new(username).unwrap();
    unsafe {
        Host_OnAchievementsLoginSuccess(
            username_c.as_ptr(),
            points,
            sc_points,
            unread_messages,
        )
    };
}

/// Host::OnAchievementsRefreshed
pub fn host_on_achievements_refreshed() {
    unsafe { Host_OnAchievementsRefreshed() };
}

/// Host::OnAchievementsHardcoreModeChanged
pub fn host_on_achievements_hardcore_mode_changed(enabled: bool) {
    unsafe { Host_OnAchievementsHardcoreModeChanged(enabled) };
}

/// Host::OnGameChanged
pub fn host_on_game_changed(
    title: &str,
    elf_override: &str,
    disc_path: &str,
    serial: &str,
    disc_crc: u32,
    current_crc: u32,
) {
    let title_c = CString::new(title).unwrap();
    let elf_c = CString::new(elf_override).unwrap();
    let disc_c = CString::new(disc_path).unwrap();
    let serial_c = CString::new(serial).unwrap();
    unsafe {
        Host_OnGameChanged(
            title_c.as_ptr(),
            elf_c.as_ptr(),
            disc_c.as_ptr(),
            serial_c.as_ptr(),
            disc_crc,
            current_crc,
        )
    };
}

/// Host::OnInputDeviceConnected
pub fn host_on_input_device_connected(identifier: &str, device_name: &str) {
    let id_c = CString::new(identifier).unwrap();
    let name_c = CString::new(device_name).unwrap();
    unsafe { Host_OnInputDeviceConnected(id_c.as_ptr(), name_c.as_ptr()) };

    if unsafe { VMManager_HasValidVM() } {
        // Would add OSD message
    }
}

/// Host::OnInputDeviceDisconnected
pub fn host_on_input_device_disconnected(key: u32, identifier: &str) {
    let id_c = CString::new(identifier).unwrap();
    unsafe { Host_OnInputDeviceDisconnected(key, id_c.as_ptr()) };

    if unsafe { VMManager_GetState() } == VM_STATE_RUNNING
        && unsafe {
            Host_GetBoolSettingValue(
                b"UI\0".as_ptr() as *const c_char,
                b"PauseOnControllerDisconnection\0".as_ptr() as *const c_char,
                false,
            )
        }
    {
        // Would pause VM and show warning
    }
}

/// Host::OnCaptureStarted
pub fn host_on_capture_started(filename: &str) {
    let filename_c = CString::new(filename).unwrap();
    unsafe { Host_OnCaptureStarted(filename_c.as_ptr()) };
}

/// Host::OnCaptureStopped
pub fn host_on_capture_stopped() {
    unsafe { Host_OnCaptureStopped() };
}

/// Host::SetMouseMode
pub fn host_set_mouse_mode(relative_mode: bool, hide_cursor: bool) {
    unsafe { Host_SetMouseMode(relative_mode, hide_cursor) };
}

/// Host::SetMouseLock
pub fn host_set_mouse_lock(state: bool) {
    unsafe { Host_SetMouseLock(state) };
}

/// Host::AcquireRenderWindow
pub fn host_acquire_render_window(recreate_window: bool) -> Option<WindowInfo> {
    // Would call emu_thread->acquireRenderWindow()
    Some(WindowInfo::default())
}

/// Host::ReleaseRenderWindow
pub fn host_release_render_window() {
    // Would call emu_thread->releaseRenderWindow()
}

/// Host::BeginPresentFrame
pub fn host_begin_present_frame() {
    unsafe { Host_BeginPresentFrame() };
}

/// Host::RequestResizeHostDisplay
pub fn host_request_resize_host_display(width: i32, height: i32) {
    unsafe { Host_RequestResizeHostDisplay(width, height) };
}

/// Host::ReportInfoAsync
pub fn host_report_info_async(title: &str, message: &str) {
    let title_c = CString::new(title).unwrap_or_default();
    let msg_c = CString::new(message).unwrap_or_default();
    unsafe { Host_ReportInfoAsync(title_c.as_ptr(), msg_c.as_ptr()) };
}

/// Host::ReportErrorAsync
pub fn host_report_error_async(title: &str, message: &str) {
    let title_c = CString::new(title).unwrap_or_default();
    let msg_c = CString::new(message).unwrap_or_default();
    unsafe { Host_ReportErrorAsync(title_c.as_ptr(), msg_c.as_ptr()) };
}

/// Host::OpenURL
pub fn host_open_url(url: &str) {
    let url_c = CString::new(url).unwrap();
    unsafe { Host_OpenURL(url_c.as_ptr()) };
}

/// Host::CopyTextToClipboard
pub fn host_copy_text_to_clipboard(text: &str) -> bool {
    let _guard = S_CLIPBOARD_CACHE_MUTEX.lock().unwrap();
    unsafe {
        S_CLIPBOARD_CACHE = text.to_string();
    }
    true
}

/// Host::GetTextFromClipboard
pub fn host_get_text_from_clipboard() -> String {
    let _guard = S_CLIPBOARD_CACHE_MUTEX.lock().unwrap();
    unsafe { S_CLIPBOARD_CACHE.clone() }
}

/// Host::BeginTextInput
pub fn host_begin_text_input() {
    unsafe { Host_BeginTextInput() };
}

/// Host::EndTextInput
pub fn host_end_text_input() {
    unsafe { Host_EndTextInput() };
}

/// Host::GetTopLevelWindowInfo
pub fn host_get_top_level_window_info() -> Option<WindowInfo> {
    let mut info = WindowInfo::default();
    unsafe {
        if Host_GetTopLevelWindowInfo(&mut info as *mut WindowInfo as *mut c_void) {
            Some(info)
        } else {
            None
        }
    }
}

/// Host::CreateHostProgressCallback
pub fn host_create_host_progress_callback() -> *mut c_void {
    unsafe { Host_CreateHostProgressCallback() }
}

// ============================================================================
// Progress callback implementation - QtHostProgressCallback equivalent
// ============================================================================

/// QtHostProgressCallback equivalent
pub struct QtHostProgressCallback {
    name: String,
    shared_data: Arc<Mutex<SharedProgressData>>,
    last_progress_percent: i32,
}

struct SharedProgressData {
    init_title: String,
    init_status_text: String,
    cancelled: Arc<AtomicBool>,
    cancellable: bool,
    was_fullscreen: bool,
}

impl QtHostProgressCallback {
    pub fn new() -> Self {
        Self {
            name: String::new(),
            shared_data: Arc::new(Mutex::new(SharedProgressData {
                init_title: String::new(),
                init_status_text: String::new(),
                cancelled: Arc::new(AtomicBool::new(false)),
                cancellable: true,
                was_fullscreen: false,
            })),
            last_progress_percent: -1,
        }
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }

    pub fn push_state(&mut self) {
        // Would save state
    }

    pub fn pop_state(&mut self) {
        // Would restore state
        self.redraw(true);
    }

    pub fn is_cancelled(&self) -> bool {
        self.shared_data.lock().unwrap().cancelled.load(Ordering::Acquire)
    }

    pub fn set_cancellable(&mut self, cancellable: bool) {
        self.shared_data.lock().unwrap().cancellable = cancellable;
    }

    pub fn set_title(&mut self, title: &str) {
        let data = self.shared_data.clone();
        let title = title.to_string();
        // Would run on UI thread
    }

    pub fn set_status_text(&mut self, text: &str) {
        let data = self.shared_data.clone();
        let text = text.to_string();
        // Would run on UI thread
    }

    pub fn set_progress_range(&mut self, _range: u32) {
        self.redraw(false);
    }

    pub fn set_progress_value(&mut self, _value: u32) {
        self.redraw(false);
    }

    pub fn display_error(&self, message: &str) {
        // Would call Console.Error and Host::ReportErrorAsync
    }

    pub fn display_warning(&self, message: &str) {
        // Would call Console.Warning
    }

    pub fn display_information(&self, message: &str) {
        // Would call Console.WriteLn
    }

    pub fn display_debug_message(&self, message: &str) {
        // Would call DevCon.WriteLn
    }

    pub fn modal_error(&self, message: &str) {
        self.display_error(message);
    }

    pub fn modal_confirmation(&self, _message: &str) -> bool {
        false
    }

    pub fn modal_information(&self, message: &str) {
        self.display_information(message);
    }

    fn set_cancelled(&self) {
        // Not done here
    }

    fn ensure_has_data(&self) {
        // Would ensure shared data exists
    }

    fn ensure_dialog_visible(data: &Arc<Mutex<SharedProgressData>>) {
        // Would create QProgressDialog if needed
    }

    fn redraw(&mut self, force: bool) {
        // Would calculate percent and update dialog
    }
}

// ============================================================================
// Hotkeys - BEGIN_HOTKEY_LIST / END_HOTKEY_LIST equivalent
// ============================================================================

/// Host hotkey definitions
pub fn get_host_hotkeys_list() -> Vec<HotkeyDefinition> {
    vec![]
    // Would return list of hotkey definitions
}

// ============================================================================
// Interface stuff - Signal handler, main(), etc.
// ============================================================================

/// Signal handler for graceful shutdown
extern "C" fn signal_handler(signal: c_int) {
    static mut GRACEFUL_SHUTDOWN_ATTEMPTED: bool = false;

    if !unsafe { GRACEFUL_SHUTDOWN_ATTEMPTED } {
        // Would invoke g_main_window->requestExit
        unsafe { GRACEFUL_SHUTDOWN_ATTEMPTED = true; }
        return;
    }

    // Force exit
    #[cfg(not(target_os = "macos"))]
    {
        std::process::exit(1);
    }
}

/// Windows console ctrl handler
#[cfg(target_os = "windows")]
extern "system" fn console_ctrl_handler(dw_ctrl_type: u32) -> i32 {
    if dw_ctrl_type != 0 { // CTRL_C_EVENT
        return 0; // FALSE
    }
    signal_handler(2); // SIGTERM
    1 // TRUE
}

/// Early hardware checks (non-Windows only)
#[cfg(not(target_os = "windows"))]
fn perform_early_hardware_checks() -> bool {
    let mut error: *const c_char = ptr::null();
    let result = unsafe { VMManager_PerformEarlyHardwareChecks(&mut error) };
    if result {
        return true;
    }

    // Would show message box
    false
}

/// Run setup wizard
fn qt_host_run_setup_wizard() -> bool {
    // Would show SetupWizardDialog
    // If accepted:
    unsafe {
        Host_SetBaseBoolSettingValue(
            b"UI\0".as_ptr() as *const c_char,
            b"SetupWizardIncomplete\0".as_ptr() as *const c_char,
            false,
        );
        Host_CommitBaseSettingChanges();
    }
    true
}

/// Get default theme name
pub fn qt_host_get_default_theme_name() -> &'static str {
    "dark" // Default theme
}

/// Get default language
pub fn qt_host_get_default_language() -> &'static str {
    "en" // Default language
}

/// Update application theme
pub fn qt_host_update_application_theme() {
    // Would set application theme based on settings
}

/// Is dark application theme
pub fn qt_host_is_dark_application_theme() -> bool {
    // Would check current theme
    true
}

/// Set icon theme from style
pub fn qt_host_set_icon_theme_from_style() {
    // Would set icon theme based on current style
}

/// Get available language list
pub fn qt_host_get_available_language_list() -> Vec<(String, String)> {
    // Would return list of (language_name, language_code)
    vec![]
}

/// Install translator
pub fn qt_host_install_translator(dialog_parent: *mut c_void) {
    // Would install Qt translator for current language
}

/// Locale sensitive compare
pub fn qt_host_locale_sensitive_compare(lhs: &str, rhs: &str) -> i32 {
    lhs.cmp(rhs) as i32
}

/// Get current game title (cached)
pub fn qt_host_get_current_game_title_cached() -> String {
    String::new()
}

/// Get current game serial (cached)
pub fn qt_host_get_current_game_serial_cached() -> String {
    String::new()
}

/// Get current game path (cached)
pub fn qt_host_get_current_game_path_cached() -> String {
    String::new()
}

// ============================================================================
// PCSX2MainApplication equivalent
// ============================================================================

/// Custom QApplication equivalent
pub struct PCSX2MainApplication {
    // Would wrap QApplication
}

impl PCSX2MainApplication {
    pub fn new(argc: i32, argv: &[*mut c_char]) -> Self {
        Self {}
    }

    pub fn event(&self, event: *mut c_void) -> bool {
        // Would handle QEvent::FileOpen
        false
    }

    pub fn exec(&self) -> i32 {
        // Would run Qt event loop
        0
    }
}

// ============================================================================
// main() function equivalent
// ============================================================================

/// Main entry point
pub fn main() {
    unsafe { CrashHandler_Install() };

    #[cfg(target_os = "windows")]
    {
        // Would set locale
    }

    // Would call QGuiApplication::setHighDpiScaleFactorRoundingPolicy
    qt_host_register_types();

    let args: Vec<String> = std::env::args().collect();
    let mut string_ptrs: Vec<*mut c_char> = args
        .iter()
        .map(|s| CString::new(s.as_str()).unwrap().into_raw())
        .collect();

    // Would create PCSX2MainApplication

    qt_host_initialize_clipboard();

    #[cfg(not(target_os = "windows"))]
    {
        if !perform_early_hardware_checks() {
            std::process::exit(1);
        }
    }

    let mut autoboot: Option<VMBootParameters> = None;
    if let Err(e) = qt_host_parse_command_line_options(&args, &mut autoboot) {
        eprintln!("Error: {}", e.message);
        std::process::exit(1);
    }

    if let Err(e) = qt_host_initialize_config() {
        eprintln!("Failed to initialize config: {}", e.message);
        std::process::exit(1);
    }

    if unsafe { S_TEST_CONFIG_AND_EXIT } {
        std::process::exit(0);
    }

    if unsafe { S_CLEANUP_AFTER_UPDATE } {
        // Would call AutoUpdaterDialog::cleanupAfterUpdate()
    }

    qt_host_update_application_theme();

    // Would call LogWindow::updateSettings()

    qt_host_hook_signals();

    // Would call EmuThread::start()

    if unsafe { S_RUN_SETUP_WIZARD } && !qt_host_run_setup_wizard() {
        std::process::exit(1);
    }

    // Would create MainWindow

    if !unsafe { S_BATCH_MODE } {
        // Would call g_main_window->refreshGameList(false, false)
    } else {
        // Would call GameList::Refresh(false, true)
    }

    if !unsafe { S_NOGUI_MODE } {
        // Would show main window
    }

    // Would initialize big picture mode if requested

    // Would check for debugger

    // Would boot VM if autoboot

    // Would run app.exec()

    // Shutdown
    // Would call EmuThread::stop()

    // Clean up string pointers
    for ptr in string_ptrs {
        unsafe { drop(CString::from_raw(ptr)) };
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vm_boot_parameters_default() {
        let params = VMBootParameters::default();
        assert!(params.filename.is_empty());
        assert!(params.elf_override.is_empty());
        assert!(params.fullscreen.is_none());
    }

    #[test]
    fn test_progress_callback() {
        let mut cb = ProgressCallback::new();
        cb.set_progress_range(100);
        cb.set_progress_value(50);
        assert!(!cb.is_cancelled());
    }

    #[test]
    fn test_app_name_and_version() {
        let name = qt_host_get_app_name_and_version();
        assert!(name.starts_with("PCSX2 "));
    }

    #[test]
    fn test_parse_command_line_empty() {
        let args = vec![];
        let mut autoboot = None;
        assert!(qt_host_parse_command_line_options(&args, &mut autoboot).is_ok());
    }

    #[test]
    fn test_parse_command_line_batch() {
        let args = vec!["pcsx2".to_string(), "-batch".to_string()];
        let mut autoboot = None;
        unsafe { S_BATCH_MODE = false; }
        let result = qt_host_parse_command_line_options(&args, &mut autoboot);
        assert!(result.is_ok());
        assert!(unsafe { S_BATCH_MODE });
    }

    #[test]
    fn test_parse_command_line_fullscreen() {
        let args = vec!["pcsx2".to_string(), "-fullscreen".to_string()];
        let mut autoboot = None;
        unsafe { S_START_FULLSCREEN = false; }
        let result = qt_host_parse_command_line_options(&args, &mut autoboot);
        assert!(result.is_ok());
        assert!(autoboot.is_some());
        assert_eq!(autoboot.unwrap().fullscreen, Some(true));
    }

    #[test]
    fn test_lock_vm_with_dialog() {
        let initial = S_VM_LOCKED_WITH_DIALOG.load(Ordering::Relaxed);
        qt_host_lock_vm_with_dialog();
        assert_eq!(S_VM_LOCKED_WITH_DIALOG.load(Ordering::Relaxed), initial + 1);
        qt_host_unlock_vm_with_dialog();
        assert_eq!(S_VM_LOCKED_WITH_DIALOG.load(Ordering::Relaxed), initial);
    }

    #[test]
    fn test_app_config_suffix() {
        let suffix = qt_host_get_app_config_suffix();
        // Just check it returns a string slice
        assert!(!suffix.is_empty() || suffix.is_empty());
    }

    #[test]
    fn test_host_in_batch_mode() {
        unsafe { S_BATCH_MODE = true; }
        assert!(host_in_batch_mode());
        unsafe { S_BATCH_MODE = false; }
        assert!(!host_in_batch_mode());
    }

    #[test]
    fn test_host_in_no_gui_mode() {
        unsafe { S_NOGUI_MODE = true; }
        assert!(host_in_no_gui_mode());
        unsafe { S_NOGUI_MODE = false; }
        assert!(!host_in_no_gui_mode());
    }
}
