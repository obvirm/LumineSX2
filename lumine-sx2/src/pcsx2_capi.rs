// FFI bindings to pcsx2_capi (C++ bridge to PCSX2 core)

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_float, c_int, c_uint, c_uchar};
use std::sync::Mutex;

// ─── VM State Enum ───
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PCSX2_VMState {
    Invalid = 0,
    Initializing = 1,
    Running = 2,
    Paused = 3,
    Stopping = 4,
}

// ─── Boot Params ───
#[repr(C)]
pub struct PCSX2_BootParams {
    pub filename: *const c_char,
    pub save_state: *const c_char,
    pub fast_boot: bool,
    pub fullscreen: bool,
    pub start_turbo: bool,
    pub start_unlimited: bool,
}

// ─── C FFI declarations ───
extern "C" {
    fn pcsx2_initialize(bios_dir: *const c_char) -> bool;
    fn pcsx2_set_bios_dir(path: *const c_char) -> bool;
    fn pcsx2_get_bios_dir() -> *const c_char;
    fn pcsx2_boot(params: *const PCSX2_BootParams) -> bool;
    fn pcsx2_boot_disc(disc_path: *const c_char, fast_boot: bool) -> bool;
    fn pcsx2_shutdown();
    fn pcsx2_execute();
    fn pcsx2_pump_messages();
    fn pcsx2_set_render_parent(hwnd: *mut std::ffi::c_void, x: i32, y: i32, w: i32, h: i32);
    fn pcsx2_resize_render(x: i32, y: i32, w: i32, h: i32);
    fn pcsx2_reset();
    fn pcsx2_set_state(state: c_int);
    fn pcsx2_set_paused(paused: bool);
    fn pcsx2_get_state() -> c_int;
    fn pcsx2_has_valid_vm() -> bool;
    fn pcsx2_get_disc_path() -> *const c_char;
    fn pcsx2_get_disc_serial() -> *const c_char;
    fn pcsx2_get_title() -> *const c_char;
    fn pcsx2_save_state(slot: c_int) -> bool;
    fn pcsx2_load_state(slot: c_int) -> bool;
    fn pcsx2_has_save_state(slot: c_int) -> bool;
    fn pcsx2_get_bool_setting(s: *const c_char, k: *const c_char, d: bool) -> bool;
    fn pcsx2_get_int_setting(s: *const c_char, k: *const c_char, d: c_int) -> c_int;
    fn pcsx2_get_float_setting(s: *const c_char, k: *const c_char, d: c_float) -> c_float;
    fn pcsx2_get_string_setting(s: *const c_char, k: *const c_char, d: *const c_char) -> *const c_char;
    fn pcsx2_set_bool_setting(s: *const c_char, k: *const c_char, v: bool);
    fn pcsx2_set_int_setting(s: *const c_char, k: *const c_char, v: c_int);
    fn pcsx2_set_float_setting(s: *const c_char, k: *const c_char, v: c_float);
    fn pcsx2_set_string_setting(s: *const c_char, k: *const c_char, v: *const c_char);
    fn pcsx2_commit_settings();
    fn pcsx2_apply_settings();
    fn pcsx2_reload_game_settings();
    fn pcsx2_change_disc(path: *const c_char);
    fn pcsx2_reload_input_bindings();
    fn pcsx2_osd_message(msg: *const c_char, dur: c_float);
    fn pcsx2_osd_clear();
    fn pcsx2_copy_to_clipboard(text: *const c_char) -> bool;
    fn pcsx2_get_from_clipboard() -> *const c_char;
    fn pcsx2_refresh_game_list(invalidate: bool);
    fn pcsx2_cancel_game_list_refresh();
    fn pcsx2_get_limiter_mode() -> c_int;
    fn pcsx2_set_limiter_mode(mode: c_int);
    fn pcsx2_free_string(s: *const c_char);
    fn pcsx2_register_callbacks(
        on_vm_starting: *const u8,
        on_vm_started: *const u8,
        on_vm_paused: *const u8,
        on_vm_resumed: *const u8,
        on_vm_destroyed: *const u8,
        on_game_changed: *const u8,
        on_ss_loading: *const u8,
        on_ss_loaded: *const u8,
        on_ss_saved: *const u8,
        on_error: *const u8,
        on_info: *const u8,
        on_frame: *const u8,
    );

    // Frame capture API
    fn pcsx2_frame_width() -> c_int;
    fn pcsx2_frame_height() -> c_int;
    fn pcsx2_frame_ready() -> bool;
    fn pcsx2_frame_data() -> *const u8;
    fn pcsx2_frame_size() -> c_int;
    fn pcsx2_frame_consumed();

    // Log streaming + version
    fn pcsx2_register_log_callback(on_log: *const u8);
    fn pcsx2_get_version_string() -> *const c_char;

    // Hotkeys
    fn pcsx2_get_hotkey_list() -> *const c_char;
    fn pcsx2_get_hotkey_binding(name: *const c_char) -> *const c_char;
    fn pcsx2_set_hotkey_binding(name: *const c_char, binding: *const c_char);
    fn pcsx2_clear_hotkey_binding(name: *const c_char);
    fn pcsx2_capture_hotkey_begin();
    fn pcsx2_capture_hotkey_poll(out: *mut c_char, size: c_int) -> bool;
    fn pcsx2_capture_hotkey_cancel();
}

// ─── Default callback implementations ───
extern "C" fn cb_starting() { eprintln!("[PCSX2] VM Starting"); }
extern "C" fn cb_started() { eprintln!("[PCSX2] VM Started"); }
extern "C" fn cb_paused() { eprintln!("[PCSX2] VM Paused"); }
extern "C" fn cb_resumed() { eprintln!("[PCSX2] VM Resumed"); }
extern "C" fn cb_destroyed() { eprintln!("[PCSX2] VM Destroyed"); }
extern "C" fn cb_game(p: *const c_char, s: *const c_char, t: *const c_char) {
    unsafe {
        let path = if p.is_null() { "" } else { CStr::from_ptr(p).to_str().unwrap_or("") };
        let serial = if s.is_null() { "" } else { CStr::from_ptr(s).to_str().unwrap_or("") };
        let title = if t.is_null() { "" } else { CStr::from_ptr(t).to_str().unwrap_or("") };
        eprintln!("[PCSX2] Game Changed: path={} serial={} title={}", path, serial, title);
    }
}
extern "C" fn cb_ss_load(_: *const c_char) {}
extern "C" fn cb_ss_loaded(_: *const c_char, _: bool) {}
extern "C" fn cb_ss_saved(_: *const c_char, _: bool, _: *const c_char) {}
extern "C" fn cb_error(t: *const c_char, m: *const c_char) {
    unsafe {
        let title = if t.is_null() { "" } else { CStr::from_ptr(t).to_str().unwrap_or("") };
        let msg = if m.is_null() { "" } else { CStr::from_ptr(m).to_str().unwrap_or("") };
        eprintln!("[PCSX2] Error: {} — {}", title, msg);
    }
}
extern "C" fn cb_info(_: *const c_char, _: *const c_char) {}
extern "C" fn cb_frame(_: *const u8, _: c_int, _: c_int, _: c_int) {}

// ─── Log ring buffer (shared between core log sink and Slint viewer) ───
#[derive(Clone)]
pub struct LogEntry {
    pub level: i32,
    pub color: i32,
    pub message: String,
}

static LOG_BUFFER: Mutex<Vec<LogEntry>> = Mutex::new(Vec::new());
const MAX_LOG_LINES: usize = 2000;

extern "C" fn cb_log(level: c_int, color: c_int, message: *const c_char) {
    let msg = unsafe {
        if message.is_null() {
            String::new()
        } else {
            CStr::from_ptr(message).to_string_lossy().into_owned()
        }
    };
    if let Ok(mut buf) = LOG_BUFFER.lock() {
        buf.push(LogEntry { level, color, message: msg });
        if buf.len() > MAX_LOG_LINES {
            let drop = buf.len() - MAX_LOG_LINES;
            buf.drain(0..drop);
        }
    }
}

pub fn get_log_lines() -> Vec<LogEntry> {
    LOG_BUFFER.lock().map(|b| b.clone()).unwrap_or_default()
}

/// Drains the in-memory log buffer (used by the Log viewer "Clear" button).
pub fn clear_log_buffer() {
    if let Ok(mut buf) = LOG_BUFFER.lock() {
        buf.clear();
    }
}

// ─── Safe Rust API ───
pub struct Pcsx2Api;

/// Mirrors a PCSX2 hotkey (name/category/display + current binding string).
#[derive(Clone)]
pub struct Pcsx2Hotkey {
    pub name: String,
    pub category: String,
    pub display_name: String,
    pub binding: String,
}

impl Pcsx2Api {
    pub fn initialize(bios_dir: &str) -> bool {
        let c = CString::new(bios_dir).unwrap();
        unsafe { pcsx2_initialize(c.as_ptr()) }
    }

    pub fn set_bios_dir(path: &str) -> bool {
        let c = CString::new(path).unwrap();
        unsafe { pcsx2_set_bios_dir(c.as_ptr()) }
    }

    pub fn get_bios_dir() -> String {
        unsafe {
            let p = pcsx2_get_bios_dir();
            if p.is_null() { String::new() } else { CStr::from_ptr(p).to_str().unwrap_or("").to_string() }
        }
    }

    pub fn boot(filename: &str, fast_boot: bool) -> bool {
        let c = CString::new(filename).unwrap();
        let save = CString::new("").unwrap();
        let p = PCSX2_BootParams {
            filename: c.as_ptr(),
            save_state: save.as_ptr(),
            fast_boot,
            fullscreen: false,
            start_turbo: false,
            start_unlimited: false,
        };
        unsafe { pcsx2_boot(&p) }
    }

    pub fn boot_disc(disc_path: &str, fast_boot: bool) -> bool {
        let c = CString::new(disc_path).unwrap();
        unsafe { pcsx2_boot_disc(c.as_ptr(), fast_boot) }
    }

    pub fn shutdown() { unsafe { pcsx2_shutdown() } }
    pub fn execute() { unsafe { pcsx2_execute() } }
    pub fn pump_messages() { unsafe { pcsx2_pump_messages() } }
    pub fn set_render_parent(hwnd: *mut std::ffi::c_void, x: i32, y: i32, w: i32, h: i32) { unsafe { pcsx2_set_render_parent(hwnd, x, y, w, h) } }
    pub fn resize_render(x: i32, y: i32, w: i32, h: i32) { unsafe { pcsx2_resize_render(x, y, w, h) } }
    pub fn reset() { unsafe { pcsx2_reset() } }

    pub fn set_state(state: PCSX2_VMState) {
        unsafe { pcsx2_set_state(state as c_int) }
    }

    pub fn set_paused(paused: bool) { unsafe { pcsx2_set_paused(paused) } }

    pub fn get_state() -> PCSX2_VMState {
        match unsafe { pcsx2_get_state() } {
            1 => PCSX2_VMState::Initializing,
            2 => PCSX2_VMState::Running,
            3 => PCSX2_VMState::Paused,
            4 => PCSX2_VMState::Stopping,
            _ => PCSX2_VMState::Invalid,
        }
    }

    pub fn has_valid_vm() -> bool { unsafe { pcsx2_has_valid_vm() } }

    pub fn get_disc_path() -> String {
        unsafe {
            let p = pcsx2_get_disc_path();
            if p.is_null() { String::new() } else { CStr::from_ptr(p).to_str().unwrap_or("").to_string() }
        }
    }

    pub fn get_disc_serial() -> String {
        unsafe {
            let p = pcsx2_get_disc_serial();
            if p.is_null() { String::new() } else { CStr::from_ptr(p).to_str().unwrap_or("").to_string() }
        }
    }

    pub fn get_title() -> String {
        unsafe {
            let p = pcsx2_get_title();
            if p.is_null() { String::new() } else { CStr::from_ptr(p).to_str().unwrap_or("").to_string() }
        }
    }

    pub fn save_state(slot: i32) -> bool { unsafe { pcsx2_save_state(slot) } }
    pub fn load_state(slot: i32) -> bool { unsafe { pcsx2_load_state(slot) } }
    pub fn has_save_state(slot: i32) -> bool { unsafe { pcsx2_has_save_state(slot) } }

    pub fn get_bool_setting(section: &str, key: &str, default: bool) -> bool {
        let s = CString::new(section).unwrap();
        let k = CString::new(key).unwrap();
        unsafe { pcsx2_get_bool_setting(s.as_ptr(), k.as_ptr(), default) }
    }

    pub fn get_int_setting(section: &str, key: &str, default: i32) -> i32 {
        let s = CString::new(section).unwrap();
        let k = CString::new(key).unwrap();
        unsafe { pcsx2_get_int_setting(s.as_ptr(), k.as_ptr(), default) }
    }

    pub fn get_float_setting(section: &str, key: &str, default: f32) -> f32 {
        let s = CString::new(section).unwrap();
        let k = CString::new(key).unwrap();
        unsafe { pcsx2_get_float_setting(s.as_ptr(), k.as_ptr(), default) }
    }

    pub fn get_string_setting(section: &str, key: &str, default: &str) -> String {
        let s = CString::new(section).unwrap();
        let k = CString::new(key).unwrap();
        let d = CString::new(default).unwrap();
        unsafe {
            let p = pcsx2_get_string_setting(s.as_ptr(), k.as_ptr(), d.as_ptr());
            if p.is_null() { default.to_string() } else { CStr::from_ptr(p).to_str().unwrap_or(default).to_string() }
        }
    }

    pub fn set_bool_setting(section: &str, key: &str, value: bool) {
        let s = CString::new(section).unwrap();
        let k = CString::new(key).unwrap();
        unsafe { pcsx2_set_bool_setting(s.as_ptr(), k.as_ptr(), value) }
    }

    pub fn set_int_setting(section: &str, key: &str, value: i32) {
        let s = CString::new(section).unwrap();
        let k = CString::new(key).unwrap();
        unsafe { pcsx2_set_int_setting(s.as_ptr(), k.as_ptr(), value) }
    }

    pub fn set_float_setting(section: &str, key: &str, value: f32) {
        let s = CString::new(section).unwrap();
        let k = CString::new(key).unwrap();
        unsafe { pcsx2_set_float_setting(s.as_ptr(), k.as_ptr(), value) }
    }

    pub fn set_string_setting(section: &str, key: &str, value: &str) {
        let s = CString::new(section).unwrap();
        let k = CString::new(key).unwrap();
        let v = CString::new(value).unwrap();
        unsafe { pcsx2_set_string_setting(s.as_ptr(), k.as_ptr(), v.as_ptr()) }
    }

    pub fn commit_settings() { unsafe { pcsx2_commit_settings() } }
    pub fn apply_settings() { unsafe { pcsx2_apply_settings() } }
    pub fn reload_game_settings() { unsafe { pcsx2_reload_game_settings() } }

    pub fn change_disc(path: &str) {
        let c = CString::new(path).unwrap();
        unsafe { pcsx2_change_disc(c.as_ptr()) }
    }

    pub fn reload_input_bindings() { unsafe { pcsx2_reload_input_bindings() } }

    pub fn osd_message(msg: &str, duration: f32) {
        let c = CString::new(msg).unwrap();
        unsafe { pcsx2_osd_message(c.as_ptr(), duration) }
    }

    pub fn osd_clear() { unsafe { pcsx2_osd_clear() } }

    pub fn copy_to_clipboard(text: &str) -> bool {
        let c = CString::new(text).unwrap();
        unsafe { pcsx2_copy_to_clipboard(c.as_ptr()) }
    }

    pub fn get_from_clipboard() -> String {
        unsafe {
            let p = pcsx2_get_from_clipboard();
            if p.is_null() { String::new() } else { CStr::from_ptr(p).to_str().unwrap_or("").to_string() }
        }
    }

    pub fn refresh_game_list(invalidate: bool) { unsafe { pcsx2_refresh_game_list(invalidate) } }
    pub fn cancel_game_list_refresh() { unsafe { pcsx2_cancel_game_list_refresh() } }

    pub fn get_limiter_mode() -> i32 { unsafe { pcsx2_get_limiter_mode() } }
    pub fn set_limiter_mode(mode: i32) { unsafe { pcsx2_set_limiter_mode(mode) } }

    // Frame capture API
    pub fn frame_width() -> i32 { unsafe { pcsx2_frame_width() as i32 } }
    pub fn frame_height() -> i32 { unsafe { pcsx2_frame_height() as i32 } }
    pub fn frame_ready() -> bool { unsafe { pcsx2_frame_ready() } }
    pub fn frame_data() -> *const u8 { unsafe { pcsx2_frame_data() } }
    pub fn frame_size() -> usize { unsafe { pcsx2_frame_size() as usize } }
    pub fn frame_consumed() { unsafe { pcsx2_frame_consumed() } }

    pub fn register_log_callback() {
        unsafe { pcsx2_register_log_callback(cb_log as *const u8); }
    }

    pub fn get_version_string() -> String {
        unsafe {
            let p = pcsx2_get_version_string();
            if p.is_null() {
                String::new()
            } else {
                CStr::from_ptr(p).to_string_lossy().into_owned()
            }
        }
    }

    // ─── Hotkeys ───
    /// Returns all hotkeys as a Vec. Each hotkey's `binding` is the current first
    /// binding string (or empty if unbound).
    pub fn get_hotkey_list() -> Vec<Pcsx2Hotkey> {
        let raw = unsafe {
            let p = pcsx2_get_hotkey_list();
            if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().into_owned() }
        };
        raw.lines()
            .map(|line| {
                let mut parts = line.splitn(3, '|');
                let name = parts.next().unwrap_or("").to_string();
                let category = parts.next().unwrap_or("").to_string();
                let display_name = parts.next().unwrap_or("").to_string();
                let binding = Self::get_hotkey_binding(&name);
                Pcsx2Hotkey { name, category, display_name, binding }
            })
            .collect()
    }

    pub fn get_hotkey_binding(name: &str) -> String {
        let c = CString::new(name).unwrap();
        unsafe {
            let p = pcsx2_get_hotkey_binding(c.as_ptr());
            if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().into_owned() }
        }
    }

    pub fn set_hotkey_binding(name: &str, binding: &str) {
        let cn = CString::new(name).unwrap();
        let cb = CString::new(binding).unwrap();
        unsafe { pcsx2_set_hotkey_binding(cn.as_ptr(), cb.as_ptr()); }
    }

    pub fn clear_hotkey_binding(name: &str) {
        let c = CString::new(name).unwrap();
        unsafe { pcsx2_clear_hotkey_binding(c.as_ptr()); }
    }

    /// Begin capturing the next key press. Poll with `poll_hotkey_capture()`.
    pub fn capture_hotkey_begin() {
        unsafe { pcsx2_capture_hotkey_begin(); }
    }

    pub fn poll_hotkey_capture() -> Option<String> {
        let mut buf = [0u8; 256];
        unsafe {
            if pcsx2_capture_hotkey_poll(buf.as_mut_ptr() as *mut c_char, buf.len() as c_int) {
                let s = CStr::from_ptr(buf.as_ptr() as *const c_char)
                    .to_string_lossy()
                    .into_owned();
                if s.is_empty() { None } else { Some(s) }
            } else {
                None
            }
        }
    }

    pub fn capture_hotkey_cancel() {
        unsafe { pcsx2_capture_hotkey_cancel(); }
    }

    /// Get frame as BGRA pixel slice. Returns (width, height, data).
    pub fn get_frame() -> Option<(i32, i32, Vec<u8>)> {
        if !Self::frame_ready() { return None; }
        let w = Self::frame_width();
        let h = Self::frame_height();
        if w <= 0 || h <= 0 { return None; }
        let size = Self::frame_size();
        if size == 0 { return None; }
        let data = unsafe { std::slice::from_raw_parts(Self::frame_data(), size) }.to_vec();
        Self::frame_consumed();
        Some((w, h, data))
    }

    pub fn register_default_callbacks() {
        unsafe {
            pcsx2_register_callbacks(
                cb_starting as *const u8,
                cb_started as *const u8,
                cb_paused as *const u8,
                cb_resumed as *const u8,
                cb_destroyed as *const u8,
                cb_game as *const u8,
                cb_ss_load as *const u8,
                cb_ss_loaded as *const u8,
                cb_ss_saved as *const u8,
                cb_error as *const u8,
                cb_info as *const u8,
                cb_frame as *const u8,
            );
            // Also register the core log sink so the Slint Log viewer can show it.
            pcsx2_register_log_callback(cb_log as *const u8);
        }
    }
}
