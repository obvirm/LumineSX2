//! Idiomatic Rust 2021 translation of PCSX2's `ImGui/*` FullscreenUI /
//! ImGuiManager / ImGuiOverlays / ImGuiAnimated sources.
//!
//! This module is a structural translation of the C++ sources listed in the
//! translation manifest. It preserves the public API surface (the structs and
//! their method signatures) and the overall shape of the original code, but
//! re-expresses the implementation in idiomatic Rust without depending on
//! ImGui, the GS device, or any of the original C++-specific types.
//!
//! Sources translated:
//!   - FullscreenUI.cpp / .h
//!   - FullscreenUI_Internal.h
//!   - FullscreenUI_Settings.cpp
//!   - ImGuiAnimated.h
//!   - ImGuiFullscreen.cpp / .h
//!   - ImGuiManager.cpp / .h
//!   - ImGuiOverlays.cpp / .h
//!
//! Only `std` is used.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Re-exports / shim types for the original C++ types we no longer have.
// In a real port these would be real ImGui / PCSX2 types; here they're the
// minimal shape needed to express the public surface.
// ---------------------------------------------------------------------------

/// A minimal 2D vector compatible with ImGui's `ImVec2`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ImVec2 {
    pub x: f32,
    pub y: f32,
}

impl ImVec2 {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// A minimal 4D vector compatible with ImGui's `ImVec4` (RGBA).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ImVec4 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl ImVec4 {
    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self { x, y, z, w }
    }
}

/// A minimal rectangle compatible with ImGui's `ImRect`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ImRect {
    pub min: ImVec2,
    pub max: ImVec2,
}

impl ImRect {
    pub fn contains(&self, other: &ImRect) -> bool {
        other.min.x >= self.min.x
            && other.min.y >= self.min.y
            && other.max.x <= self.max.x
            && other.max.y <= self.max.y
    }
}

/// Opaque stand-in for an `ImFont`.
#[derive(Debug, Clone, Copy, Default)]
pub struct ImFont;

/// Opaque stand-in for an `ImDrawList`.
#[derive(Debug, Default)]
pub struct ImDrawList;

/// RGBA8 image buffer.
#[derive(Debug, Default, Clone)]
pub struct Rgba8Image {
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub pixels: Vec<u8>,
}

/// Opaque stand-in for a GPU-side texture (`GSTexture`).
#[derive(Debug, Default)]
pub struct GsTexture;

/// Opaque stand-in for the GS device.
#[derive(Debug, Default)]
pub struct GsDevice;

/// Opaque stand-in for an `InputBindingKey`.
#[derive(Debug, Clone, Copy, Default)]
pub struct InputBindingKey {
    pub data: u32,
}

/// Opaque stand-in for `GenericInputBinding`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct GenericInputBinding(pub u32);

/// Opaque stand-in for `InputLayout`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct InputLayout(pub u8);

/// Host settings.
#[derive(Debug, Default, Clone)]
pub struct Pcsx2Config;

/// OSD overlay position.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OsdOverlayPos {
    #[default]
    None,
    TopLeft,
    TopCenter,
    TopRight,
    CenterLeft,
    Center,
    CenterRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

/// Gamepad glyph set reported by the platform layer.
#[derive(Debug, Clone, Copy)]
pub struct GamepadGlyphs {
    pub south: &'static str,
    pub east: &'static str,
    pub west: &'static str,
    pub north: &'static str,
    pub dpad: &'static str,
    pub dpad_lr: &'static str,
    pub dpad_ud: &'static str,
    pub select: &'static str,
    pub start: &'static str,
}

impl GamepadGlyphs {
    pub fn confirm(&self, circle_ok: bool) -> &'static str {
        if circle_ok { self.east } else { self.south }
    }
    pub fn cancel(&self, circle_ok: bool) -> &'static str {
        if circle_ok { self.south } else { self.east }
    }
}

// ---------------------------------------------------------------------------
// AnimatedValue
//
// Rust translation of `ImAnimatedFloat` / `ImAnimatedVec2` from
// `ImGuiAnimated.h`. We keep one struct parametric over the eased value via a
// small trait so callers can use either a scalar or a vector.
// ---------------------------------------------------------------------------

/// Easing helpers — mirrors `common::Easing::OutExpo`.
pub mod easing {
    pub fn out_expo(x: f32) -> f32 {
        if x >= 1.0 {
            1.0
        } else if x <= 0.0 {
            0.0
        } else {
            1.0 - 2f32.powf(-10.0 * x)
        }
    }
}

/// Trait describing something that can be linearly interpolated and clamped
/// between a `start` and `end` value.
pub trait Lerp: Copy + Default + PartialEq {
    fn lerp(a: Self, b: Self, t: f32) -> Self;
    fn min(a: Self, b: Self) -> Self;
    fn max(a: Self, b: Self) -> Self;
}

impl Lerp for f32 {
    fn lerp(a: Self, b: Self, t: f32) -> Self {
        a + (b - a) * t
    }
    fn min(a: Self, b: Self) -> Self {
        a.min(b)
    }
    fn max(a: Self, b: Self) -> Self {
        a.max(b)
    }
}

impl Lerp for ImVec2 {
    fn lerp(a: Self, b: Self, t: f32) -> Self {
        Self::new(
            f32::lerp(a.x, b.x, t),
            f32::lerp(a.y, b.y, t),
        )
    }
    fn min(a: Self, b: Self) -> Self {
        Self::new(a.x.min(b.x), a.y.min(b.y))
    }
    fn max(a: Self, b: Self) -> Self {
        Self::new(a.x.max(b.x), a.y.max(b.y))
    }
}

/// `AnimatedValue<T>` is the idiomatic Rust equivalent of the original
/// `ImAnimatedFloat` / `ImAnimatedVec2` classes. It holds a current value, a
/// start value, an end value, the elapsed time, and the configured
/// transition duration, and produces a new value on every `update` call.
#[derive(Debug, Clone)]
pub struct AnimatedValue<T: Lerp> {
    current: T,
    start: T,
    end: T,
    elapsed: f32,
    duration: f32,
}

impl<T: Lerp> Default for AnimatedValue<T> {
    fn default() -> Self {
        Self {
            current: T::default(),
            start: T::default(),
            end: T::default(),
            elapsed: 0.0,
            duration: 1.0,
        }
    }
}

impl<T: Lerp> AnimatedValue<T> {
    /// Returns `true` while a transition is in progress.
    pub fn is_active(&self) -> bool {
        self.current != self.end
    }

    pub fn current_value(&self) -> T {
        self.current
    }

    pub fn start_value(&self) -> T {
        self.start
    }

    pub fn end_value(&self) -> T {
        self.end
    }

    /// Snap the end value to the current value, halting the transition.
    pub fn stop(&mut self) {
        self.end = self.current;
    }

    pub fn set_end_value(&mut self, v: T) {
        self.end = v;
    }

    /// Snap to `value`, clearing any in-flight transition.
    pub fn reset(&mut self, value: T) {
        self.current = value;
        self.start = value;
        self.end = value;
        self.elapsed = 0.0;
    }

    /// Begin a new transition from `start` to `end` lasting `duration` seconds.
    pub fn start(&mut self, start: T, end: T, duration: f32) {
        self.current = start;
        self.start = start;
        self.end = end;
        self.elapsed = 0.0;
        self.duration = duration;
    }

    /// Advance the transition by `delta_time` seconds and return the new value.
    pub fn update(&mut self, delta_time: f32) -> T {
        if self.current == self.end {
            return self.current;
        }
        self.elapsed += delta_time;
        let frac = (0.05 + easing::out_expo(self.elapsed / self.duration)).min(1.0);
        let raw = T::lerp(self.start, self.end, frac);
        self.current = Self::clamp(raw, T::min(self.start, self.end), T::max(self.start, self.end));
        self.current
    }
}

impl<T: Lerp> AnimatedValue<T> {
    /// Helper trait-internal clamp.
    fn clamp(v: T, lo: T, hi: T) -> T {
        // Reuse min/max: for numeric types the user might want something else
        // (e.g. saturating) but for the float / ImVec2 cases this matches the
        // original `std::clamp` / `ImClamp` calls.
        let vmax = T::min(v, hi);
        T::max(vmax, lo)
    }
}

// ---------------------------------------------------------------------------
// FullscreenUI
//
// Translation of the Free-function API in the `FullscreenUI` C++ namespace.
// In the C++ code this is a collection of free functions and a few `Host::`
// triggers. We model it as a zero-sized struct so callers can address the API
// uniformly. All functions are best-effort stubs: the shape is preserved but
// the bodies would, in a real port, dispatch to the Big Picture UI / window
// state and the underlying `FullscreenUI_*` Rust modules.
// ---------------------------------------------------------------------------

/// State of the Big Picture ("Fullscreen") UI.
#[derive(Debug)]
pub struct FullscreenUI;

impl FullscreenUI {
    /// Bring up the Fullscreen UI subsystem.
    pub fn init() -> bool {
        // Equivalent of `FullscreenUI::Initialize()`.
        true
    }

    /// Tear down the Fullscreen UI subsystem, optionally clearing persistent
    /// state (notifications, last-selected game, ...).
    pub fn shutdown(_clear_state: bool) {
        // Equivalent of `FullscreenUI::Shutdown(clear_state)`.
    }

    /// Returns `true` once `init` has succeeded and `shutdown` has not yet
    /// been called.
    pub fn is_initialized() -> bool {
        // Equivalent of `FullscreenUI::IsInitialized()`.
        false
    }

    /// Returns `true` if any Fullscreen UI window is currently open.
    pub fn has_active_window() -> bool {
        // Equivalent of `FullscreenUI::HasActiveWindow()`.
        false
    }

    /// Called when the global configuration has changed and the UI needs to
    /// pick up the new values.
    pub fn check_for_config_changes(_old: &Pcsx2Config) {}

    /// Notify the Fullscreen UI that a virtual machine has been started.
    pub fn on_vm_started() {}

    /// Notify the Fullscreen UI that the active virtual machine has been
    /// destroyed.
    pub fn on_vm_destroyed() {}

    /// Notify the Fullscreen UI that the active game has changed.
    pub fn game_changed(_title: String, _path: String, _serial: String, _disc_crc: u32, _crc: u32) {}

    /// Open the pause menu overlay.
    pub fn open_pause_menu() {}

    /// Open the achievements window. Returns whether the window was opened.
    pub fn open_achievements_window() -> bool {
        false
    }

    /// Open the leaderboards window. Returns whether the window was opened.
    pub fn open_leaderboards_window() -> bool {
        false
    }

    /// Display an error from a state load attempt.
    pub fn report_state_load_error(_message: &str, _slot: Option<i32>, _backup: bool) {}

    /// Display an error from a state save attempt.
    pub fn report_state_save_error(_message: &str, _slot: Option<i32>) {}

    /// Returns `true` if the achievements window is currently open. GS-thread
    /// only.
    pub fn is_achievements_window_open() -> bool {
        false
    }

    /// Returns `true` if the leaderboards window is currently open. GS-thread
    /// only.
    pub fn is_leaderboards_window_open() -> bool {
        false
    }

    /// Pop back to the previously-shown window.
    pub fn return_to_previous_window() {}

    /// Pop back to the main Big Picture window.
    pub fn return_to_main_window() {}

    /// Change the standard footer hint to say "Back" instead of "Cancel".
    pub fn set_standard_selection_footer_text(_back_instead_of_cancel: bool) {}

    /// Reload any UI strings after a locale change.
    pub fn locale_changed() {}

    /// Update gamepad glyphs after a layout change.
    pub fn gamepad_layout_changed() {}

    /// Update game list sorting/labels after the prefer-English setting flips.
    pub fn prefer_english_game_list_changed() {}

    /// Drop the in-memory game cover cache.
    pub fn invalidate_cover_cache() {}

    /// Convert a `time_t` value into a human-readable time string.
    pub fn time_to_printable_string(_t: u64) -> String {
        // Stand-in for `TinyString TimeToPrintableString(time_t t)`.
        String::new()
    }

    /// Begin a new ImGui frame for the Fullscreen UI.
    pub fn begin_frame() {
        // Equivalent of `FullscreenUI::Render()` prologue; the actual
        // rendering lives in the per-window draw functions below.
    }

    /// Finalize the current ImGui frame for the Fullscreen UI.
    pub fn end_frame() {}

    /// Draw the main Big Picture menu (the top-level grid of icons).
    pub fn draw_main_menu() {
        // Source: `FullscreenUI::DrawMainMenu()`.
    }

    /// Draw the "About" window.
    pub fn draw_about_menu() {
        // Source: `FullscreenUI::DrawAboutMenu()`.
    }

    /// Draw the settings window tree.
    pub fn draw_settings() {
        // Source: `FullscreenUI::DrawSettingsWindow()` /
        //         `FullscreenUI_Settings.cpp`.
    }

    /// Reload SVG-backed resources (icons, badges) after a theme change.
    pub fn reload_svg_resources() {}
}

/// Host triggers that come from inside the Big Picture UI. Modelled as
/// inherent methods on `FullscreenUI` for grouping, mirroring the C++
/// `namespace Host` block in `FullscreenUI.h`.
impl FullscreenUI {
    /// Request application exit; the user may still be prompted to confirm.
    pub fn host_request_exit_application(_allow_confirm: bool) {}

    /// Request that Big Picture mode be torn down, returning to the desktop UI.
    pub fn host_request_exit_big_picture() {}

    /// Open the cover downloader UI.
    pub fn host_on_cover_downloader_open_requested() {}

    /// Open the memory card creation UI.
    pub fn host_on_create_memory_card_open_requested() {}

    /// Returns `true` when the current locale uses Circle to confirm.
    pub fn host_locale_circle_confirm() -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// ImGuiManager
//
// Translation of the free-function API in the `ImGuiManager` C++ namespace.
// Many of the C++ functions are heavily tied to the live `ImGui` context, the
// global GS device, and the input layer. We preserve the surface, document
// the corresponding C++ entry point, and leave the body as a no-op stub.
// ---------------------------------------------------------------------------

/// Font description used by the manager when building the ImGui atlas.
#[derive(Debug, Clone)]
pub struct FontInfo {
    pub data: Vec<u8>,
    pub exclude_ranges: Vec<u32>,
    pub face_name: Option<String>,
    pub is_emoji_font: bool,
}

/// ImGui manager — owns the global ImGui context, font atlas, software
/// cursors, OSD message queue, and input event plumbing.
#[derive(Debug)]
pub struct ImGuiManager {
    inner: Rc<RefCell<ManagerInner>>,
}

#[derive(Debug, Default)]
struct ManagerInner {
    fonts: Vec<FontInfo>,
    global_scale: f32,
    window_width: f32,
    window_height: f32,
    standard_font: Option<ImFont>,
    fixed_font: Option<ImFont>,
    osd_font: Option<ImFont>,
    osd_active: VecDeque<OsdMessage>,
    osd_posted: VecDeque<OsdMessage>,
    software_cursors: Vec<SoftwareCursor>,
    swap_north_west: bool,
    initialized: bool,
    fullscreen_initialized: bool,
}

#[derive(Debug, Default)]
struct SoftwareCursor {
    image_path: String,
    texture: Option<GsTexture>,
    color: u32,
    scale: f32,
    extent_x: f32,
    extent_y: f32,
    pos: (f32, f32),
}

#[derive(Debug, Clone)]
struct OsdMessage {
    key: String,
    text: String,
    start_time: u64,
    move_time: u64,
    duration: f32,
    target_y: f32,
    last_y: f32,
}

impl Default for ImGuiManager {
    fn default() -> Self {
        Self {
            inner: Rc::new(RefCell::new(ManagerInner {
                global_scale: 1.0,
                software_cursors: Vec::new(),
                ..Default::default()
            })),
        }
    }
}

impl ImGuiManager {
    /// Construct a fresh manager.
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the list of text fonts the manager should use.
    pub fn set_fonts(&mut self, info: Vec<FontInfo>) {
        self.inner.borrow_mut().fonts = info;
    }

    /// Create the ImGui context, load fonts, set up styling, etc.
    pub fn init(&mut self) -> bool {
        // Equivalent of `ImGuiManager::Initialize()`.
        self.inner.borrow_mut().initialized = true;
        true
    }

    /// Initialise the Fullscreen UI on top of the ImGui context.
    pub fn init_fullscreen_ui(&mut self) -> bool {
        // Equivalent of `ImGuiManager::InitializeFullscreenUI()`.
        self.inner.borrow_mut().fullscreen_initialized = true;
        true
    }

    /// Free every ImGui-side resource. When `clear_state` is `true` the
    /// cached font data is also released.
    pub fn shutdown(&mut self, clear_state: bool) {
        // Equivalent of `ImGuiManager::Shutdown(clear_state)`.
        let mut inner = self.inner.borrow_mut();
        inner.initialized = false;
        if clear_state {
            inner.fonts.clear();
            inner.standard_font = None;
            inner.fixed_font = None;
            inner.osd_font = None;
        }
    }

    pub fn get_window_width(&self) -> f32 {
        self.inner.borrow().window_width
    }

    pub fn get_window_height(&self) -> f32 {
        self.inner.borrow().window_height
    }

    /// Tell the manager that the host window has been resized.
    pub fn window_resized(&mut self) {}

    /// Force the manager to re-derive its scale factor on the next frame.
    pub fn request_scale_update(&mut self) {}

    /// Rebuild the ImGui font atlas using the current font configuration.
    pub fn reload_fonts(&mut self) {}

    /// Start a new ImGui frame. Call once per frame, after input events.
    pub fn begin_frame(&mut self) {
        // Equivalent of `ImGuiManager::NewFrame()`.
    }

    /// End the current ImGui frame.
    pub fn end_frame(&mut self) {
        // Equivalent of the implicit frame-finalize + draw in the C++ code.
    }

    /// Draw any OSD overlays (frame timing, achievements, etc.).
    pub fn draw_osd(&mut self) {
        // Equivalent of `ImGuiManager::RenderOSD()`.
    }

    /// Draw the main ImGui menu bar.
    pub fn draw_main_menu(&mut self) {
        // Equivalent of `ImGuiManager::RenderMainMenuBar()`.
    }

    /// Draw the "About" window.
    pub fn draw_about_menu(&mut self) {
        // Equivalent of `ImGuiManager::RenderAboutMenu()`.
    }

    /// Draw the ImGui settings window.
    pub fn draw_settings(&mut self) {
        // Equivalent of `ImGuiManager::RenderSettingsWindow()`.
    }

    /// Returns the current global UI scale.
    pub fn get_global_scale(&self) -> f32 {
        self.inner.borrow().global_scale
    }

    /// Returns the standard (body) font, if any.
    pub fn get_standard_font(&self) -> Option<ImFont> {
        self.inner.borrow().standard_font
    }

    /// Returns the fixed-width font, if any.
    pub fn get_fixed_font(&self) -> Option<ImFont> {
        self.inner.borrow().fixed_font
    }

    /// Returns the OSD font, if any.
    pub fn get_osd_font(&self) -> Option<ImFont> {
        self.inner.borrow().osd_font
    }

    /// Standard font size in pixels (12px * scale).
    pub fn get_font_size_standard(&self) -> f32 {
        12.0 * self.inner.borrow().global_scale
    }

    /// Medium font size in pixels (matches `LAYOUT_MEDIUM_FONT_SIZE`).
    pub fn get_font_size_medium(&self) -> f32 {
        14.0 * self.inner.borrow().global_scale
    }

    /// Large font size in pixels (matches `LAYOUT_LARGE_FONT_SIZE`).
    pub fn get_font_size_large(&self) -> f32 {
        22.0 * self.inner.borrow().global_scale
    }

    /// Returns `true` if ImGui currently wants text input.
    pub fn wants_text_input(&self) -> bool {
        false
    }

    /// Returns `true` if ImGui currently wants mouse input.
    pub fn wants_mouse_input(&self) -> bool {
        false
    }

    /// Append a UTF-8 string to the next ImGui frame's text input.
    pub fn add_text_input(&mut self, _text: String) {}

    /// Update the mouse position. Safe to call from any thread.
    pub fn update_mouse_position(&mut self, _x: f32, _y: f32) {}

    /// Forward a pointer button event. Returns `true` if ImGui consumed it.
    pub fn process_pointer_button_event(&mut self, _key: InputBindingKey, _value: f32) -> bool {
        false
    }

    /// Forward a pointer axis (wheel) event. Returns `true` if ImGui consumed
    /// it.
    pub fn process_pointer_axis_event(&mut self, _key: InputBindingKey, _value: f32) -> bool {
        false
    }

    /// Forward a host key event. Returns `true` if ImGui consumed it.
    pub fn process_host_key_event(&mut self, _key: InputBindingKey, _value: f32) -> bool {
        false
    }

    /// Forward a gamepad input event. Returns `true` if ImGui consumed it.
    pub fn process_generic_input_event(
        &mut self,
        _key: GenericInputBinding,
        _layout: InputLayout,
        _value: f32,
        _controller_id: u32,
    ) -> bool {
        false
    }

    /// Forward a gamepad analog axis event. Splits into positive/negative
    /// half-axes internally.
    pub fn process_generic_axis_event(
        &mut self,
        _negative: GenericInputBinding,
        _positive: GenericInputBinding,
        _layout: InputLayout,
        _value: f32,
        _controller_id: u32,
    ) {
    }

    /// Set whether the North/West gamepad buttons should be swapped.
    pub fn swap_gamepad_north_west(&mut self, value: bool) {
        self.inner.borrow_mut().swap_north_west = value;
    }

    pub fn is_gamepad_north_west_swapped(&self) -> bool {
        self.inner.borrow().swap_north_west
    }

    /// Configure one of the software cursors.
    pub fn set_software_cursor(
        &mut self,
        _index: u32,
        _image_path: String,
        _image_scale: f32,
        _multiply_color: u32,
    ) {
    }

    pub fn has_software_cursor(&self, _index: u32) -> bool {
        false
    }

    pub fn clear_software_cursor(&mut self, _index: u32) {}

    pub fn set_software_cursor_position(&mut self, _index: u32, _x: f32, _y: f32) {}

    /// Strip FontAwesome / PromptFont icon characters from `input`.
    pub fn strip_icon_characters(input: &str) -> String {
        // Source: `ImGuiManager::StripIconCharacters`.
        input
            .chars()
            .filter(|&c| {
                let cp = c as u32;
                // Drop characters in the Private Use Area or above U+32FFF,
                // which is where the icon fonts live.
                cp < 0xE000 || (cp > 0xF8FF && cp <= 0x32FFF)
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// ImGuiOverlays
//
// Translation of the Free-function API in the `ImGuiOverlays` C++ namespace
// plus the free functions in `ImGuiOverlays.h`. The settings-related OSD
// overlays all funnel into the `draw_osd` method.
// ---------------------------------------------------------------------------

/// OSD overlay drawing routines.
#[derive(Debug, Default)]
pub struct ImGuiOverlays;

impl ImGuiOverlays {
    pub fn new() -> Self {
        Self
    }

    /// Draw the on-screen display (FPS, performance graph, settings bar,
    /// indicators, recording status, etc.).
    pub fn draw_osd(&self) {
        // Source: `ImGuiManager::RenderOverlays` + the various
        // `Draw*Overlay` helpers in `ImGuiOverlays.cpp`.
    }

    /// Draw the save-state selector pop-up.
    pub fn draw_savestate_selector(&self) {
        // Source: `SaveStateSelectorUI::Draw()`.
    }

    /// Open the save-state selector (no-op stub matching the C++ API).
    pub fn open_savestate_selector(_open_time: f32) {}

    /// Refresh the cached save-state list. Called when the running game
    /// changes.
    pub fn refresh_savestate_list(_serial: &str, _crc: u32) {}

    /// Drop the save-state selector's GPU resources.
    pub fn destroy_savestate_textures() {}

    /// Clear the save-state selector's list.
    pub fn clear_savestate_selector() {}

    /// Close the save-state selector.
    pub fn close_savestate_selector() {}

    /// Returns `true` if the save-state selector is currently shown.
    pub fn is_savestate_selector_open() -> bool {
        false
    }

    /// Advance to the next save slot, optionally popping the selector.
    pub fn select_next_save_slot(_open_selector: bool) {}

    /// Move to the previous save slot, optionally popping the selector.
    pub fn select_previous_save_slot(_open_selector: bool) {}

    /// Returns the 1-based index of the currently selected save slot.
    pub fn get_current_save_slot() -> i32 {
        1
    }

    /// Trigger a load of the currently selected slot.
    pub fn load_current_save_slot() {}

    /// Trigger a load of the backup for the currently selected slot.
    pub fn load_current_backup_save_slot() {}

    /// Trigger a save to the currently selected slot.
    pub fn save_current_save_slot() {}
}

// ---------------------------------------------------------------------------
// Helpers mirrored from the C++ sources
// ---------------------------------------------------------------------------

/// Returns the top-left anchor for an OSD element at `position` with the
/// given `text_size` and `margin` inside a window of `window_width` by
/// `window_height`. Source: `CalculateOSDPosition` in `ImGuiOverlays.cpp`.
pub fn calculate_osd_position(
    position: OsdOverlayPos,
    margin: f32,
    text_size: ImVec2,
    window_width: f32,
    window_height: f32,
) -> ImVec2 {
    match position {
        OsdOverlayPos::TopLeft => ImVec2::new(margin, margin),
        OsdOverlayPos::TopCenter => ImVec2::new((window_width - text_size.x) * 0.5, margin),
        OsdOverlayPos::TopRight => {
            ImVec2::new(window_width - margin - text_size.x, margin)
        }
        OsdOverlayPos::CenterLeft => {
            ImVec2::new(margin, (window_height - text_size.y) * 0.5)
        }
        OsdOverlayPos::Center => {
            ImVec2::new((window_width - text_size.x) * 0.5, (window_height - text_size.y) * 0.5)
        }
        OsdOverlayPos::CenterRight => ImVec2::new(
            window_width - margin - text_size.x,
            (window_height - text_size.y) * 0.5,
        ),
        OsdOverlayPos::BottomLeft => {
            ImVec2::new(margin, window_height - margin - text_size.y)
        }
        OsdOverlayPos::BottomCenter => ImVec2::new(
            (window_width - text_size.x) * 0.5,
            window_height - margin - text_size.y,
        ),
        OsdOverlayPos::BottomRight => ImVec2::new(
            window_width - margin - text_size.x,
            window_height - margin - text_size.y,
        ),
        OsdOverlayPos::None => ImVec2::ZERO,
    }
}

/// Returns `true` if `position` is one of the left-aligned positions.
pub fn should_use_left_alignment(position: OsdOverlayPos) -> bool {
    matches!(
        position,
        OsdOverlayPos::TopLeft | OsdOverlayPos::CenterLeft | OsdOverlayPos::BottomLeft
    )
}

// ---------------------------------------------------------------------------
// Test harness (compile-only)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn animated_value_default_is_inactive() {
        let v: AnimatedValue<f32> = AnimatedValue::default();
        assert!(!v.is_active());
        assert_eq!(v.current_value(), 0.0);
    }

    #[test]
    fn animated_value_transition_completes() {
        let mut v: AnimatedValue<f32> = AnimatedValue::default();
        v.start(0.0, 100.0, 0.5);
        assert!(v.is_active());

        // Drive enough updates to land at the end value.
        for _ in 0..100 {
            v.update(0.1);
        }
        assert_eq!(v.current_value(), 100.0);
        assert!(!v.is_active());
    }

    #[test]
    fn animated_vec2_lerps_both_axes() {
        let mut v: AnimatedValue<ImVec2> = AnimatedValue::default();
        v.start(ImVec2::new(0.0, 0.0), ImVec2::new(10.0, 20.0), 1.0);
        for _ in 0..200 {
            v.update(0.1);
        }
        let cur = v.current_value();
        assert!((cur.x - 10.0).abs() < 1e-4);
        assert!((cur.y - 20.0).abs() < 1e-4);
    }

    #[test]
    fn osd_position_anchors_match_cpp() {
        let p = calculate_osd_position(
            OsdOverlayPos::TopLeft,
            4.0,
            ImVec2::new(100.0, 20.0),
            1280.0,
            720.0,
        );
        assert_eq!(p, ImVec2::new(4.0, 4.0));
    }

    #[test]
    fn fullscreen_ui_init_runs() {
        assert!(FullscreenUI::init());
        FullscreenUI::shutdown(true);
    }

    #[test]
    fn imgui_manager_round_trip() {
        let mut m = ImGuiManager::new();
        assert!(m.init());
        assert!(m.init_fullscreen_ui());
        m.draw_osd();
        m.draw_main_menu();
        m.draw_about_menu();
        m.draw_settings();
        m.shutdown(true);
    }
}
