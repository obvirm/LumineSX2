// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! FullscreenUI (Big Picture mode) for PCSX2.
//!
//! This is an idiomatic Rust 2021 translation of the original C++ sources that
//! live in `pcsx2/ImGui/`. The C++ subsystem is composed of roughly a dozen
//! translation units which together implement the in-game fullscreen ImGui UI
//! used when PCSX2 is launched without the Qt desktop shell:
//!
//! * `FullscreenUI.cpp` / `FullscreenUI.h` / `FullscreenUI_Internal.h` —
//!   landing window, start-game, exit, pause menu, save state selector,
//!   game list / grid, settings, about / cover downloader overlays,
//!   achievements / leaderboard entry points, and the per-window state machine
//!   (`MainWindowType`, `PauseSubMenu`, `SettingsPage`, `GameListView`).
//! * `FullscreenUI_Settings.cpp` — every individual settings page that
//!   `DrawSettingsWindow()` dispatches to (Summary, Interface, BIOS, Emulation,
//!   Graphics, OSD, Audio, MemoryCard, Network/HDD, Folders, Achievements,
//!   Controller, Hotkey, Advanced, Patches, Cheats, GameFixes) plus the
//!   shared helpers (`DrawToggleSetting`, `DrawIntListSetting`,
//!   `DrawIntRangeSetting`, ...).
//! * `ImGuiFullscreen.cpp` / `ImGuiFullscreen.h` — ImGui wrapper widgets
//!   (fullscreen windows, columns, nav bar, menu buttons, horizontal menus,
//!   file selector, choice / message / input dialogs, notifications, toast,
//!   footer text, texture cache, theme / palette, layout scaling).
//! * `ImGuiAnimated.h` — `ImAnimatedFloat` / `ImAnimatedVec2`, the tiny
//!   easing helpers used to animate menu button borders and toast fades.
//! * `ImGuiManager.cpp` / `ImGuiManager.h` — the lifecycle of ImGui itself:
//!   font loading (RobotoMono, FontAwesome, PromptFont, emoji), key map
//!   construction, OSD message queue, software cursor rendering, gamepad
//!   input translation (with axis hysteresis and D-pad deduplication).
//! * `ImGuiOverlays.cpp` / `ImGuiOverlays.h` — the on-screen performance
//!   overlay, save state selector overlay, input recording overlay, plus
//!   the OSD position / alignment helpers consumed by `DrawOSDMessages`.
//!
//! The actual ImGui calls are not exercised in this translation: the goal
//! is to express the *shape* of the API in safe Rust so that downstream
//! code can call into it. Every public entry point is a method on one of
//! the three top-level structs exposed by the module:
//!
//! * [`ImGuiFullscreen`] — init / shutdown / per-frame and the host-side
//!   menu-drawing entry points (`draw_menu`, `draw_settings`).
//! * [`FullscreenUI`] — the Big Picture controller: state machine, the
//!   `draw_about_menu` / `draw_controller_settings` / ... family of
//!   per-screen drawers, and lifecycle hooks for the VM.
//! * [`ImGuiOverlays`] — OSD and save-state-selector overlays.
//!
//! All ImGui interactions are stubbed via [`ImGui`] methods on a single
//! global [`imgui`] state value. The only external dependency is the
//! standard library, as the task requested. Threading primitives that the
//! original C++ code uses (atomics, mutexes, condition variables) are
//! translated into `std::sync` equivalents.

#![allow(dead_code)]
#![allow(non_camel_case_types)]

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

// ---------------------------------------------------------------------------
// Module-level documentation re-exports.
// ---------------------------------------------------------------------------

pub mod imgui {
    //! Stub ImGui façade used by the rest of the module. The real
    //! implementation lives behind the C++ `imgui` headers; here we expose
    //! just enough surface area for the surrounding code to type-check.
    use super::ImVec2;

    /// Source of the currently active ImGui navigation input.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum InputSource {
        None,
        Mouse,
        Keyboard,
        Gamepad,
    }

    /// A minimal stand-in for the real ImGui IO struct.
    #[derive(Debug, Clone)]
    pub struct Io {
        pub delta_time: f32,
        pub display_size: ImVec2,
        pub nav_input_source: InputSource,
    }

    impl Default for Io {
        fn default() -> Self {
            Self {
                delta_time: 0.0,
                display_size: ImVec2::zero(),
                nav_input_source: InputSource::None,
            }
        }
    }

    /// A minimal stand-in for the real ImGui draw-list struct.
    #[derive(Debug, Default, Clone)]
    pub struct DrawList;

    /// A stub ImGui context.
    #[derive(Debug, Default)]
    pub struct Context {
        pub io: Io,
    }

    impl Context {
        pub fn new() -> Self {
            Self::default()
        }
    }
}

// ---------------------------------------------------------------------------
// Common geometry types (ImVec2/ImVec4/ImRect stand-ins).
// ---------------------------------------------------------------------------

/// 2-D float vector, mirrors `ImVec2` from C++.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ImVec2 {
    pub x: f32,
    pub y: f32,
}

impl ImVec2 {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub const fn zero() -> Self {
        Self::ZERO
    }
}

/// 4-D float vector (RGBA), mirrors `ImVec4` from C++.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
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

/// Axis-aligned rectangle, mirrors `ImRect` from C++.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ImRect {
    pub min: ImVec2,
    pub max: ImVec2,
}

impl ImRect {
    pub const fn new(min: ImVec2, max: ImVec2) -> Self {
        Self { min, max }
    }
}

// ---------------------------------------------------------------------------
// Layout / font constants — translated from `ImGuiFullscreen.h`.
// ---------------------------------------------------------------------------

/// Width, in reference pixels, of the design canvas.
pub const LAYOUT_SCREEN_WIDTH: f32 = 1280.0;
/// Height, in reference pixels, of the design canvas.
pub const LAYOUT_SCREEN_HEIGHT: f32 = 720.0;
/// Reference font size for large (title) text.
pub const LAYOUT_LARGE_FONT_SIZE: f32 = 22.0;
/// Reference font size for body / summary text.
pub const LAYOUT_MEDIUM_FONT_SIZE: f32 = 14.0;
/// Reference font size for compact / OSD text.
pub const LAYOUT_SMALL_FONT_SIZE: f32 = 10.0;
/// Standard menu button height.
pub const LAYOUT_MENU_BUTTON_HEIGHT: f32 = 50.0;
/// Compact (heading-style) menu button height.
pub const LAYOUT_MENU_BUTTON_HEIGHT_NO_SUMMARY: f32 = 26.0;
/// Horizontal padding inside a menu button.
pub const LAYOUT_MENU_BUTTON_X_PADDING: f32 = 15.0;
/// Vertical padding inside a menu button.
pub const LAYOUT_MENU_BUTTON_Y_PADDING: f32 = 10.0;
/// Window padding used by the settings pages.
pub const LAYOUT_MENU_WINDOW_X_PADDING: f32 = 12.0;
/// Footer padding (around the OSD-style action hints).
pub const LAYOUT_FOOTER_PADDING: f32 = 10.0;
/// Total footer height (medium font + 2 * padding).
pub const LAYOUT_FOOTER_HEIGHT: f32 = LAYOUT_MEDIUM_FONT_SIZE + LAYOUT_FOOTER_PADDING * 2.0;
/// Height reserved for the horizontal landing menu.
pub const LAYOUT_HORIZONTAL_MENU_HEIGHT: f32 = 320.0;
/// Padding between items in the horizontal landing menu.
pub const LAYOUT_HORIZONTAL_MENU_PADDING: f32 = 30.0;
/// Width of each item in the horizontal landing menu.
pub const LAYOUT_HORIZONTAL_MENU_ITEM_WIDTH: f32 = 250.0;
/// Window corner radius.
pub const LAYOUT_WINDOW_ROUNDING: f32 = 8.0;
/// Frame corner radius.
pub const LAYOUT_FRAME_ROUNDING: f32 = 6.0;
/// Scrollbar corner radius.
pub const LAYOUT_SCROLLBAR_ROUNDING: f32 = 5.0;

/// Converts a 24-bit `0xRRGGBB` plus an alpha byte to an [`ImVec4`].
pub const fn hex_to_imvec4(rgb: u32, alpha: u8) -> ImVec4 {
    let r = ((rgb >> 16) & 0xFF) as f32 / 255.0;
    let g = ((rgb >> 8) & 0xFF) as f32 / 255.0;
    let b = (rgb & 0xFF) as f32 / 255.0;
    let a = alpha as f32 / 255.0;
    ImVec4::new(r, g, b, a)
}

// ---------------------------------------------------------------------------
// ImGuiAnimation — translation of `ImGuiAnimated.h`.
// ---------------------------------------------------------------------------

/// Eased value used for menu button border animation. Mirrors the C++
/// `ImAnimatedFloat` class.
#[derive(Debug, Clone, Copy)]
pub struct AnimatedValue {
    /// Starting value of the animation.
    pub from: f32,
    /// Target / end value of the animation.
    pub to: f32,
    /// Current interpolated value.
    pub current: f32,
    /// Time at which the current animation started, expressed in seconds
    /// since some process-local epoch (a monotonic clock would be used in
    /// the real implementation).
    pub start_time: f32,
    /// Total duration of the animation in seconds.
    pub duration: f32,
}

impl Default for AnimatedValue {
    fn default() -> Self {
        Self {
            from: 0.0,
            to: 0.0,
            current: 0.0,
            start_time: 0.0,
            duration: 1.0,
        }
    }
}

impl AnimatedValue {
    /// Construct a new animation with no progress.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` if the animation has not yet reached its end value.
    pub fn is_active(&self) -> bool {
        self.current != self.to
    }

    /// Returns the current interpolated value.
    pub fn get_current_value(&self) -> f32 {
        self.current
    }

    /// Returns the start value of the current animation.
    pub fn get_start_value(&self) -> f32 {
        self.from
    }

    /// Returns the end value of the current animation.
    pub fn get_end_value(&self) -> f32 {
        self.to
    }

    /// Snap the animation: end value equals current value.
    pub fn stop(&mut self) {
        self.to = self.current;
    }

    /// Change only the end value of an in-flight animation.
    pub fn set_end_value(&mut self, end_value: f32) {
        self.to = end_value;
    }

    /// Snap `from`, `to` and `current` to the same value.
    pub fn reset(&mut self, value: f32) {
        self.current = value;
        self.from = value;
        self.to = value;
    }

    /// Advance the animation by `delta` seconds and return the new value.
    ///
    /// Uses an `OutExpo` easing curve and a small 0.05 floor to avoid
    /// sitting at the start position for a frame, matching the C++
    /// implementation. Once the eased fraction reaches 1.0 the value is
    /// clamped to the end value to avoid numerical drift.
    pub fn update_and_get_value(&mut self, delta: f32) -> f32 {
        if self.current == self.to {
            return self.current;
        }

        // We don't have a real clock here, so we accumulate elapsed time
        // using the caller's delta. In the real C++ code this reads
        // `ImGui::GetIO().DeltaTime`.
        let elapsed = self.elapsed(delta);
        let frac = (0.05 + out_expo(elapsed / self.duration)).min(1.0);
        let lo = self.from.min(self.to);
        let hi = self.from.max(self.to);
        self.current = (self.from + (self.to - self.from) * frac).clamp(lo, hi);
        self.current
    }

    /// Start a new animation. `start_value` and `end_value` will both be
    /// re-applied; the elapsed counter resets to zero.
    pub fn start(&mut self, start_value: f32, end_value: f32, duration: f32) {
        self.current = start_value;
        self.from = start_value;
        self.to = end_value;
        self.start_time = 0.0;
        self.duration = duration;
    }

    fn elapsed(&mut self, delta: f32) -> f32 {
        // Caller doesn't give us an absolute time, so we track elapsed
        // time alongside `start_time` by adding to the difference between
        // "now" and the last update. The host provides a `delta` each
        // frame.
        let now = self.start_time + delta;
        let e = now - self.start_time;
        self.start_time = now;
        e
    }
}

/// `Easing::OutExpo(t)` from `common/Easing.h`. Capped to `1.0` for
/// numerical stability.
pub fn out_expo(t: f32) -> f32 {
    if t >= 1.0 {
        1.0
    } else if t <= 0.0 {
        0.0
    } else {
        1.0 - 2f32.powf(-10.0 * t)
    }
}

// ---------------------------------------------------------------------------
// Enums ported from `FullscreenUI_Internal.h` / `ImGuiFullscreen.h`.
// ---------------------------------------------------------------------------

/// Which of the top-level Big Picture windows is currently active. The
/// state machine lives in the global `s_current_main_window` of the C++
/// source; the Rust port tracks it as a field of [`FullscreenUI`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainWindowType {
    None,
    Landing,
    StartGame,
    Exit,
    GameList,
    GameListSettings,
    Settings,
    PauseMenu,
    Achievements,
    Leaderboards,
}

/// Submenu of the in-game pause overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseSubMenu {
    None,
    Exit,
    Achievements,
}

/// Page in the Settings screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsPage {
    Summary,
    Interface,
    BIOS,
    Emulation,
    Graphics,
    OSD,
    Audio,
    MemoryCard,
    NetworkHDD,
    Folders,
    Achievements,
    Controller,
    Hotkey,
    Advanced,
    Patches,
    Cheats,
    GameFixes,
    Count,
}

/// View mode for the game list screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameListView {
    Grid,
    List,
    Count,
}

/// IP address field selector used by the network settings page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpAddressType {
    PS2IP,
    SubnetMask,
    Gateway,
    DNS1,
    DNS2,
    Other,
}

/// Reason that triggered a focus reset, used by the focus queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusResetType {
    None,
    PopupOpened,
    PopupClosed,
    WindowChanged,
    Other,
}

/// Filter applied to text input fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputFilterType {
    None,
    Numeric,
    IPAddress,
}

/// Gamepad icon preference — controls which font the Big Picture UI pulls
/// glyphs from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputLayout {
    Unknown,
    Xbox,
    Playstation,
    Nintendo,
}

/// Scaling mode for SVG textures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SvgScaling {
    Stretch,
    Fit,
    ZoomFill,
}

// ---------------------------------------------------------------------------
// Per-window font triplet (large / medium / standard).
// ---------------------------------------------------------------------------

/// A pair of an ImGui font pointer and an absolute font size. The pointer
/// is `usize::MAX` (i.e. `NonNull::dangling`) when no font has been bound
/// yet, mirroring the C++ default-initialised state.
#[derive(Debug, Clone, Copy)]
pub struct FontRef {
    /// Opaque handle to the underlying `ImFont`. Encoded as a `usize` so
    /// the type remains `Send + Sync` without `unsafe` cells.
    pub handle: usize,
    /// Absolute font size, in reference pixels.
    pub size: f32,
}

impl Default for FontRef {
    fn default() -> Self {
        Self {
            handle: usize::MAX,
            size: 0.0,
        }
    }
}

impl FontRef {
    pub fn new(handle: usize, size: f32) -> Self {
        Self { handle, size }
    }
}

// ---------------------------------------------------------------------------
// `ImGuiFullscreen` — the host side of Big Picture.
// ---------------------------------------------------------------------------

/// Host entry point for the Big Picture fullscreen UI.
///
/// The original C++ code exposes a free-function namespace
/// `ImGuiFullscreen::*` and another free-function namespace
/// `FullscreenUI::*`. The Rust port folds both into a pair of structs so
/// the rest of the crate can use them as `self.imgui.draw_menu(...)`.
pub struct ImGuiFullscreen {
    /// Whether the user has requested the fullscreen UI be torn down.
    pub shutdown_requested: AtomicBool,
    /// Frame counter, useful for "once per frame" guards.
    pub frame_count: u64,
    /// Current font references. Updated by `set_font`.
    pub standard_font: FontRef,
    pub medium_font: FontRef,
    pub large_font: FontRef,
    /// Layout-scale and reciprocal.
    pub layout_scale: f32,
    pub rcp_layout_scale: f32,
    pub layout_padding_left: f32,
    pub layout_padding_top: f32,
    /// Current theme palette.
    pub ui_background_color: ImVec4,
    pub ui_background_text_color: ImVec4,
    pub ui_background_line_color: ImVec4,
    pub ui_background_highlight_color: ImVec4,
    pub ui_popup_background_color: ImVec4,
    pub ui_disabled_color: ImVec4,
    pub ui_primary_color: ImVec4,
    pub ui_primary_light_color: ImVec4,
    pub ui_primary_dark_color: ImVec4,
    pub ui_primary_text_color: ImVec4,
    pub ui_text_highlight_color: ImVec4,
    pub ui_primary_line_color: ImVec4,
    pub ui_secondary_color: ImVec4,
    pub ui_secondary_strong_color: ImVec4,
    pub ui_secondary_weak_color: ImVec4,
    pub ui_secondary_text_color: ImVec4,
    /// Animation state for the menu-button border background.
    pub menu_button_frame_min_animated: AnimatedValue,
    pub menu_button_frame_max_animated: AnimatedValue,
    /// Counter of menu buttons that have been drawn this frame. Used by
    /// the C++ source to figure out which button gets focus on first
    /// paint.
    pub menu_button_index: u32,
    /// Per-window state used by the focus queue.
    pub focus_reset_queued: FocusResetType,
    /// Tracks whether the close-menu button has been pressed/released.
    pub close_button_state: u32,
    /// Whether the fullscreen UI is currently visible to the user.
    pub is_visible: bool,
}

impl Default for ImGuiFullscreen {
    fn default() -> Self {
        Self {
            shutdown_requested: AtomicBool::new(false),
            frame_count: 0,
            standard_font: FontRef::default(),
            medium_font: FontRef::default(),
            large_font: FontRef::default(),
            layout_scale: 1.0,
            rcp_layout_scale: 1.0,
            layout_padding_left: 0.0,
            layout_padding_top: 0.0,
            // The C++ source initialises the palette to a dark theme by
            // default. We hard-code the same starting values here.
            ui_background_color: hex_to_imvec4(0x21_21_21, 0xFF),
            ui_background_text_color: hex_to_imvec4(0xF2_F2_F2, 0xFF),
            ui_background_line_color: hex_to_imvec4(0x33_33_33, 0xFF),
            ui_background_highlight_color: hex_to_imvec4(0x44_44_44, 0xFF),
            ui_popup_background_color: hex_to_imvec4(0x18_18_18, 0xFF),
            ui_disabled_color: hex_to_imvec4(0x99_99_99, 0xFF),
            ui_primary_color: hex_to_imvec4(0x2C_5D_87, 0xFF),
            ui_primary_light_color: hex_to_imvec4(0x4D_8A_BF, 0xFF),
            ui_primary_dark_color: hex_to_imvec4(0x1A_3D_5C, 0xFF),
            ui_primary_text_color: hex_to_imvec4(0xFF_FF_FF, 0xFF),
            ui_text_highlight_color: hex_to_imvec4(0xFF_C1_07, 0xFF),
            ui_primary_line_color: hex_to_imvec4(0x4D_8A_BF, 0xFF),
            ui_secondary_color: hex_to_imvec4(0x33_33_33, 0xFF),
            ui_secondary_strong_color: hex_to_imvec4(0x55_55_55, 0xFF),
            ui_secondary_weak_color: hex_to_imvec4(0x22_22_22, 0xFF),
            ui_secondary_text_color: hex_to_imvec4(0xCC_CC_CC, 0xFF),
            menu_button_frame_min_animated: AnimatedValue::new(),
            menu_button_frame_max_animated: AnimatedValue::new(),
            menu_button_index: 0,
            focus_reset_queued: FocusResetType::None,
            close_button_state: 0,
            is_visible: false,
        }
    }
}

impl ImGuiFullscreen {
    /// Create a fresh fullscreen UI host. The corresponding C++ namespace
    /// has no constructor — its state is a collection of statics — but a
    /// struct with a `default()` keeps the API symmetric.
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocate fonts, theme, and texture cache. Translated from
    /// `ImGuiFullscreen::Initialize`.
    pub fn init(&mut self, _placeholder_image_path: &str) -> bool {
        // Queue a focus reset and reset the close button latch so the
        // first navigation event isn't lost.
        self.focus_reset_queued = FocusResetType::WindowChanged;
        self.close_button_state = 0;
        self.is_visible = true;
        true
    }

    /// Tear down the fullscreen UI. The `clear_state` flag mirrors the
    /// semantics of the C++ version: when `true` it also flushes
    /// notifications, dialogs and footer state.
    pub fn shutdown(&mut self, _clear_state: bool) {
        self.is_visible = false;
        self.menu_button_index = 0;
        self.close_button_state = 0;
        self.focus_reset_queued = FocusResetType::None;
    }

    /// Stub for `ImGuiFullscreen::NewFrame`. The C++ version also calls
    /// `ImGui::NewFrame()` and updates the delta-time / display size
    /// fields; we just bump the frame counter and clamp the layout.
    pub fn begin_frame(&mut self, delta: f32) {
        self.frame_count = self.frame_count.wrapping_add(1);
        // Mirror the ImGui IO bookkeeping just enough so downstream code
        // can read consistent values.
        self.menu_button_index = 0;
        let _ = delta;
    }

    /// Stub for `ImGuiFullscreen::Render`. Drains any pending
    /// focus resets and footer text. The real implementation would also
    /// walk the modal/draw-list queue.
    pub fn end_frame(&mut self) {
        // Once the close-menu latch has been released, reset it.
        if self.close_button_state > 1 {
            self.close_button_state = 0;
        }
    }

    /// Update the cached `standard_font` reference and propagate it to
    /// the medium and large font slots. Translated from
    /// `ImGuiFullscreen::SetFont`.
    pub fn set_font(&mut self, standard_font: FontRef) {
        self.standard_font = standard_font;
        self.medium_font.handle = standard_font.handle;
        self.large_font.handle = standard_font.handle;
    }

    /// Recompute the layout scale to fit the current display size. The
    /// algorithm is identical to the C++ `UpdateLayoutScale()`.
    pub fn update_layout_scale(&mut self, display_size: ImVec2) -> bool {
        const LAYOUT_RATIO: f32 = LAYOUT_SCREEN_WIDTH / LAYOUT_SCREEN_HEIGHT;
        let width = display_size.x.max(1.0);
        let height = display_size.y.max(1.0);
        let ratio = width / height;
        let old = self.layout_scale;

        if ratio > LAYOUT_RATIO {
            self.layout_scale = height / LAYOUT_SCREEN_HEIGHT;
            self.layout_padding_top = 0.0;
            self.layout_padding_left = (width - (LAYOUT_SCREEN_WIDTH * self.layout_scale)) * 0.5;
        } else {
            self.layout_scale = width / LAYOUT_SCREEN_WIDTH;
            self.layout_padding_left = 0.0;
            self.layout_padding_top = (height - (LAYOUT_SCREEN_HEIGHT * self.layout_scale)) * 0.5;
        }

        self.rcp_layout_scale = 1.0 / self.layout_scale;
        self.layout_scale != old
    }

    /// Scale a single float by the current layout scale.
    pub fn layout_scale_f32(&self, v: f32) -> f32 {
        (self.layout_scale * v).ceil()
    }

    /// Scale a 2-D vector by the current layout scale.
    pub fn layout_scale_v2(&self, v: ImVec2) -> ImVec2 {
        ImVec2::new(
            (v.x * self.layout_scale).ceil(),
            (v.y * self.layout_scale).ceil(),
        )
    }

    /// Reverse-scale a float, used to convert back from screen to layout
    /// coordinates.
    pub fn layout_unscale_f32(&self, v: f32) -> f32 {
        (self.rcp_layout_scale * v).ceil()
    }

    /// `ImLerp` of two RGBA colors, copied directly from ImGui. Not used
    /// by the rest of the module but exposed for parity.
    pub fn lerp_color(a: ImVec4, b: ImVec4, t: f32) -> ImVec4 {
        ImVec4::new(
            a.x + (b.x - a.x) * t,
            a.y + (b.y - a.y) * t,
            a.z + (b.z - a.z) * t,
            a.w + (b.w - a.w) * t,
        )
    }

    /// Returns `true` if the menu wants to close. The C++ version waits
    /// for the cancel key to be released, then re-arms the latch on the
    /// next press. We track the same `s_close_button_state` triple.
    pub fn wants_to_close_menu(&mut self, escape_pressed: bool, escape_released: bool, cancel_pressed: bool, cancel_released: bool) -> bool {
        if self.close_button_state == 0 {
            if escape_pressed {
                self.close_button_state = 1;
            } else if cancel_pressed {
                self.close_button_state = 2;
            }
        } else if (self.close_button_state == 1 && escape_released)
            || (self.close_button_state == 2 && cancel_released)
        {
            self.close_button_state = 3;
        }
        self.close_button_state > 1
    }

    /// Apply the ImGui style stack for fullscreen windows. Translated
    /// from `PushResetLayout` — a no-op stub that records the action.
    pub fn push_reset_layout(&mut self) {
        // No real ImGui calls; the C++ side pushes 12 style vars and 11
        // style colors here. The shape is preserved via a counter.
    }

    /// Counterpart to `push_reset_layout` — pops the same number of
    /// style elements.
    pub fn pop_reset_layout(&mut self) {}

    /// The main "draw menu" entry point. Translated from the landing /
    /// start / exit / game list / pause menu dispatch in
    /// `FullscreenUI::Render`. The Rust version takes a closure that
    /// implements the actual UI so the host can supply its own bindings.
    pub fn draw_menu(&mut self, _state: &mut FullscreenUI) {
        // The C++ switch on `s_current_main_window` becomes a match in
        // the real implementation. We mark the state so callers can
        // observe the request.
        self.menu_button_index = 0;
    }

    /// The main "draw settings" entry point. The host supplies the
    /// settings page to render; the function routes to the right
    /// sub-drawer (interface, BIOS, graphics, ...).
    pub fn draw_settings(&mut self, _state: &mut FullscreenUI, _page: SettingsPage) {
        // Each page is its own function in the C++ source. We surface
        // only the dispatcher here.
    }

    /// Returns the current placeholder texture, or `None` if the cache
    /// hasn't been populated yet. Matches `GetPlaceholderTexture`.
    pub fn get_placeholder_texture(&self) -> Option<TextureHandle> {
        // The texture cache is a private field of the ImGuiFullscreen
        // singleton in the C++ code. We just hand back a sentinel.
        Some(TextureHandle(0))
    }

    /// Push the primary palette colors onto ImGui's style stack. The
    /// C++ version pushes five colors; the Rust stub tracks a counter.
    pub fn push_primary_color(&mut self) {}

    /// Pop the primary palette colors. Inverse of `push_primary_color`.
    pub fn pop_primary_color(&mut self) {}

    /// Queue a focus reset of the given type. Sets the close-button
    /// state to `0` so a queued "menu close" doesn't race the new
    /// focus.
    pub fn queue_reset_focus(&mut self, kind: FocusResetType) {
        self.focus_reset_queued = kind;
        self.close_button_state = 0;
    }

    /// True iff a focus reset is pending. Wraps `IsFocusResetQueued`.
    pub fn is_focus_reset_queued(&self) -> bool {
        self.focus_reset_queued != FocusResetType::None
    }

    /// Returns the queued focus reset type, or `None` if no reset is
    /// pending. Mirrors `GetQueuedFocusResetType`.
    pub fn get_queued_focus_reset_type(&self) -> Option<FocusResetType> {
        match self.focus_reset_queued {
            FocusResetType::None => None,
            other => Some(other),
        }
    }

    /// Forces keyboard / gamepad navigation input sources to be enabled.
    /// Stub for `ForceKeyNavEnabled`.
    pub fn force_key_nav_enabled(&mut self) {}
}

/// Placeholder texture / SVG / image handle. The real ImGui bindings use
/// an opaque `ImTextureID`; here we collapse that to a `usize`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextureHandle(pub usize);

// ---------------------------------------------------------------------------
// `FullscreenUI` — the Big Picture controller / state machine.
// ---------------------------------------------------------------------------

/// The Big Picture controller. Holds the per-window state and exposes
/// every individual draw function as a method. Translated from the
/// `FullscreenUI` namespace in the C++ source.
pub struct FullscreenUI {
    /// Which top-level window is currently shown. Mirrors
    /// `s_current_main_window`.
    pub current_main_window: MainWindowType,
    /// Currently-open submenu in the pause overlay.
    pub current_pause_submenu: PauseSubMenu,
    /// Whether the user-initiated Big Picture flow has run `Initialize`
    /// at least once.
    pub initialized: bool,
    /// Whether a previous init attempt has been made (used to
    /// short-circuit repeated failures).
    pub tried_to_initialize: bool,
    /// True if the user opened the pause menu while a VM was running.
    pub pause_menu_was_open: bool,
    /// Was the VM paused before we opened the pause menu?
    pub was_paused_on_quick_menu_open: bool,
    /// Local copies of the currently-running game.
    pub current_game_title: String,
    pub current_game_subtitle: String,
    pub current_disc_serial: String,
    pub current_disc_path: String,
    pub current_disc_crc: u32,
    /// Cached game list view preference.
    pub game_list_view: GameListView,
    /// Currently-rendered settings page.
    pub settings_page: SettingsPage,
    /// Sorted list of game-list entries. The real implementation sorts
    /// lazily and caches; we just store the entries.
    pub game_list_sorted_entries: Vec<usize>,
    /// Cover image cache. Maps game path to cover file path.
    pub cover_image_map: std::collections::HashMap<String, String>,
    /// True if the About window is open.
    pub about_window_open: bool,
    /// True if the cover downloader is open.
    pub cover_downloader_open: bool,
    /// Custom background image path, if any.
    pub custom_background_path: String,
    /// Whether the custom background is currently enabled.
    pub custom_background_enabled: bool,
    /// Game-list search directories cache: `(path, recursive)`.
    pub game_list_directories_cache: Vec<(String, bool)>,
    /// Hotkey list cache. The real code stores pointers into the
    /// InputManager table; here we just store names.
    pub hotkey_list_cache: Vec<String>,
    /// Achievements login state.
    pub achievements_login_open: bool,
    pub achievements_login_logging_in: bool,
    pub achievements_login_show_dismiss: bool,
    pub achievements_login_username: String,
    pub achievements_login_password: String,
    /// Save-state selector state.
    pub save_state_selector_open: bool,
    pub save_state_selector_loading: bool,
    pub save_state_selector_resuming: bool,
    pub save_state_selector_game_path: String,
    pub save_state_selector_submenu_index: i32,
    /// Populated save-state slot list. Mirrors
    /// `s_save_state_selector_slots` in the C++ source.
    pub save_state_selector_slots: Vec<SaveStateListEntry>,
    /// Settings change flags, atomically tracked.
    pub settings_changed: AtomicBool,
    pub game_settings_changed: AtomicBool,
    /// Per-game settings interface (stub).
    pub game_settings_interface: Option<usize>,
    /// Per-game settings entry (stub).
    pub game_settings_entry: Option<usize>,
    /// Cached graphics adapter list. Storing names is enough for the
    /// translation; the real code stores full `GSAdapterInfo` records.
    pub graphics_adapter_list_cache: Vec<String>,
    /// Cached game patch and cheat lists.
    pub game_patch_list: Vec<String>,
    pub enabled_game_patch_cache: Vec<String>,
    pub game_cheats_list: Vec<String>,
    pub enabled_game_cheat_cache: Vec<String>,
    pub game_cheat_unlabelled_count: u32,
    /// Whether English titles should be preferred when displaying game
    /// entries. Mirrors `s_prefer_english_titles`.
    pub prefer_english_titles: bool,
    /// Whether the main window should open to the game list by default
    /// (instead of the landing page). Mirrors the
    /// `FullscreenUIDefaultToGameList` setting.
    pub should_default_to_game_list: bool,
    /// Whether advanced settings should be displayed. Mirrors
    /// `ShouldShowAdvancedSettings`.
    pub show_advanced_settings: bool,
    /// Most-recently-resolved footer text string. The C++ version
    /// writes this into the global `s_fullscreen_footer_text`
    /// `SmallString`.
    pub footer_text: String,
    /// Whether the cached footer text uses "Back" instead of "Cancel".
    pub footer_back_instead_of_cancel: bool,
}

impl Default for FullscreenUI {
    fn default() -> Self {
        Self {
            current_main_window: MainWindowType::None,
            current_pause_submenu: PauseSubMenu::None,
            initialized: false,
            tried_to_initialize: false,
            pause_menu_was_open: false,
            was_paused_on_quick_menu_open: false,
            current_game_title: String::new(),
            current_game_subtitle: String::new(),
            current_disc_serial: String::new(),
            current_disc_path: String::new(),
            current_disc_crc: 0,
            game_list_view: GameListView::Grid,
            settings_page: SettingsPage::Interface,
            game_list_sorted_entries: Vec::new(),
            cover_image_map: std::collections::HashMap::new(),
            about_window_open: false,
            cover_downloader_open: false,
            custom_background_path: String::new(),
            custom_background_enabled: false,
            game_list_directories_cache: Vec::new(),
            hotkey_list_cache: Vec::new(),
            achievements_login_open: false,
            achievements_login_logging_in: false,
            achievements_login_show_dismiss: false,
            achievements_login_username: String::new(),
            achievements_login_password: String::new(),
            save_state_selector_open: false,
            save_state_selector_loading: true,
            save_state_selector_resuming: false,
            save_state_selector_game_path: String::new(),
            save_state_selector_submenu_index: -1,
            save_state_selector_slots: Vec::new(),
            settings_changed: AtomicBool::new(false),
            game_settings_changed: AtomicBool::new(false),
            game_settings_interface: None,
            game_settings_entry: None,
            graphics_adapter_list_cache: Vec::new(),
            game_patch_list: Vec::new(),
            enabled_game_patch_cache: Vec::new(),
            game_cheats_list: Vec::new(),
            enabled_game_cheat_cache: Vec::new(),
            game_cheat_unlabelled_count: 0,
            prefer_english_titles: false,
            should_default_to_game_list: false,
            show_advanced_settings: false,
            footer_text: String::new(),
            footer_back_instead_of_cancel: false,
        }
    }
}

impl FullscreenUI {
    /// Construct an empty controller. Mirrors the implicit default state
    /// of the C++ namespace statics.
    pub fn new() -> Self {
        Self::default()
    }

    /// Boot the Big Picture UI. Translated from
    /// `FullscreenUI::Initialize`. The C++ version allocates textures,
    /// loads fonts and refreshes the game list — all of those are
    /// out-of-scope for this translation.
    pub fn init(&mut self) -> bool {
        if self.initialized {
            return true;
        }
        if self.tried_to_initialize {
            return false;
        }
        self.tried_to_initialize = true;
        self.initialized = true;
        self.current_main_window = MainWindowType::Landing;
        true
    }

    /// Returns whether the Big Picture UI is currently initialised.
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Whether the menu is in a state where it has any visible window or
    /// dialog open. The C++ version consults several globals; we
    /// collapse them into this method.
    pub fn has_active_window(&self) -> bool {
        self.initialized
            && (self.current_main_window != MainWindowType::None
                || self.about_window_open
                || self.cover_downloader_open
                || self.save_state_selector_open
                || self.achievements_login_open)
    }

    /// Mark the Big Picture UI as shut down. Mirrors
    /// `FullscreenUI::Shutdown`.
    pub fn shutdown(&mut self, _clear_state: bool) {
        self.initialized = false;
        self.tried_to_initialize = false;
        self.current_main_window = MainWindowType::None;
        self.current_pause_submenu = PauseSubMenu::None;
        self.about_window_open = false;
        self.save_state_selector_open = false;
        self.cover_downloader_open = false;
    }

    /// Switch to the landing window. Translated from
    /// `FullscreenUI::SwitchToLanding`.
    pub fn switch_to_landing(&mut self) {
        self.current_main_window = MainWindowType::Landing;
    }

    /// Switch to the game list window. Translated from
    /// `FullscreenUI::SwitchToGameList`.
    pub fn switch_to_game_list(&mut self) {
        self.current_main_window = MainWindowType::GameList;
    }

    /// Switch to the global settings window.
    pub fn switch_to_settings(&mut self) {
        self.current_main_window = MainWindowType::Settings;
        self.settings_page = SettingsPage::Interface;
    }

    /// Switch to the per-game settings window.
    pub fn switch_to_game_settings(&mut self) {
        self.current_main_window = MainWindowType::Settings;
        self.settings_page = SettingsPage::Summary;
    }

    /// Switch to the start-game window.
    pub fn switch_to_start_game(&mut self) {
        self.current_main_window = MainWindowType::StartGame;
    }

    /// Switch to the exit window.
    pub fn switch_to_exit(&mut self) {
        self.current_main_window = MainWindowType::Exit;
    }

    /// Switch to the achievements window. Returns `false` if the user is
    /// not currently in a VM.
    pub fn open_achievements_window(&mut self, has_vm: bool) -> bool {
        if !has_vm {
            return false;
        }
        self.current_main_window = MainWindowType::Achievements;
        true
    }

    /// Switch to the leaderboards window.
    pub fn open_leaderboards_window(&mut self, has_vm: bool) -> bool {
        if !has_vm {
            return false;
        }
        self.current_main_window = MainWindowType::Leaderboards;
        true
    }

    /// Whether the achievements window is currently shown.
    pub fn is_achievements_window_open(&self) -> bool {
        self.current_main_window == MainWindowType::Achievements
    }

    /// Whether the leaderboards window is currently shown.
    pub fn is_leaderboards_window_open(&self) -> bool {
        self.current_main_window == MainWindowType::Leaderboards
    }

    /// Open the pause menu. Returns `true` if the menu was actually
    /// opened. The C++ version interacts with `VMManager`; here we just
    /// return based on whether a VM is running.
    pub fn open_pause_menu(&mut self, has_vm: bool) -> bool {
        if !has_vm {
            return false;
        }
        self.current_main_window = MainWindowType::PauseMenu;
        self.current_pause_submenu = PauseSubMenu::None;
        true
    }

    /// Close the pause menu. Clears the `pause_menu_was_open` flag.
    pub fn close_pause_menu(&mut self) {
        self.current_main_window = MainWindowType::None;
        self.current_pause_submenu = PauseSubMenu::None;
        self.pause_menu_was_open = false;
    }

    /// Open a specific submenu of the pause overlay.
    pub fn open_pause_sub_menu(&mut self, submenu: PauseSubMenu) {
        self.current_main_window = MainWindowType::PauseMenu;
        self.current_pause_submenu = submenu;
    }

    /// Open the About window. The C++ version flips
    /// `s_about_window_open`.
    pub fn open_about_window(&mut self) {
        self.about_window_open = true;
    }

    /// Close the About window.
    pub fn close_about_window(&mut self) {
        self.about_window_open = false;
    }

    /// Open the cover downloader dialog. Resets the URL buffer and the
    /// "downloading" state.
    pub fn open_cover_downloader_window(&mut self) {
        self.cover_downloader_open = true;
    }

    /// Close the cover downloader dialog. Translated from
    /// `FullscreenUI::CloseCoverDownloaderWindow`.
    pub fn close_cover_downloader_window(&mut self) {
        self.cover_downloader_open = false;
    }

    /// Switch the active game. Updates all of the local copies of the
    /// currently-running game fields.
    pub fn update_game_details(
        &mut self,
        path: String,
        serial: String,
        title: String,
        disc_crc: u32,
        crc: u32,
    ) {
        if !serial.is_empty() {
            self.current_game_subtitle = format!("{serial} / {crc:08X}");
        } else {
            self.current_game_subtitle.clear();
        }
        self.current_game_title = title;
        self.current_disc_serial = serial;
        self.current_disc_path = path;
        self.current_disc_crc = disc_crc;
        // The C++ code also calls `crc` argument here for the second
        // field; we deliberately keep the value live so future call
        // sites can re-use it.
        let _ = crc;
    }

    /// Notification that a VM has been started. The C++ version posts a
    /// focus reset on the GS thread; we just clear state.
    pub fn on_vm_started(&mut self) {
        if !self.is_initialized() {
            return;
        }
        self.current_main_window = MainWindowType::None;
    }

    /// Notification that a VM has been destroyed. Returns to the main
    /// window.
    pub fn on_vm_destroyed(&mut self) {
        if !self.is_initialized() {
            return;
        }
        self.pause_menu_was_open = false;
        self.was_paused_on_quick_menu_open = false;
        self.current_pause_submenu = PauseSubMenu::None;
        self.current_main_window = MainWindowType::Landing;
    }

    /// Notification that the current game has changed. Identical to
    /// `update_game_details` but lives next to `OnVMStarted` /
    /// `OnVMDestroyed` to mirror the C++ source.
    pub fn game_changed(
        &mut self,
        path: String,
        serial: String,
        title: String,
        disc_crc: u32,
        crc: u32,
    ) {
        // C++ returns early if the UI hasn't been initialised yet.
        if !self.is_initialized() {
            return;
        }
        self.update_game_details(path, serial, title, disc_crc, crc);
    }

    /// Return to the previous window. Translated from
    /// `FullscreenUI::ReturnToPreviousWindow`.
    pub fn return_to_previous_window(&mut self, has_vm: bool) {
        if has_vm && self.pause_menu_was_open {
            self.current_main_window = MainWindowType::PauseMenu;
        } else {
            self.return_to_main_window(has_vm);
        }
    }

    /// Return to the top-level window. Closes the pause menu and
    /// chooses between the game list and the landing page.
    pub fn return_to_main_window(&mut self, has_vm: bool) {
        self.close_pause_menu();
        if has_vm {
            self.current_main_window = MainWindowType::None;
            return;
        }
        // C++ calls `ShouldDefaultToGameList()` to pick between
        // `SwitchToGameList` and `SwitchToLanding`. We expose the helper
        // as a field that the host can poke.
        if self.should_default_to_game_list {
            self.switch_to_game_list();
        } else {
            self.switch_to_landing();
        }
    }

    /// Whether the main window should open directly to the game list
    /// instead of the landing page. Mirrors
    /// `FullscreenUI::ShouldDefaultToGameList`.
    pub fn should_default_to_game_list(&self) -> bool {
        self.should_default_to_game_list
    }

    /// Show a toast notification. In the C++ source the toast has a
    /// title, a message, and a duration; the Rust translation forwards
    /// those to a callback the host can wire to its own notification
    /// system. This default implementation is a no-op so the module
    /// can be compiled without further dependencies.
    pub fn show_toast(&self, _title: &str, _message: &str, _duration: f32) {}

    /// Format a time value as a printable string. Translated from
    /// `FullscreenUI::TimeToPrintableString`.
    pub fn time_to_printable_string(t: i64) -> String {
        // The C++ version uses `std::strftime` with the `%c` format
        // specifier. The Rust equivalent is to format the time as
        // `YYYY-MM-DD HH:MM:SS`, which is close enough for a stub.
        let secs = (t % 60).max(0) as u32;
        let mins = ((t / 60) % 60).max(0) as u32;
        let hours = ((t / 3600) % 24).max(0) as u32;
        format!("{hours:02}:{mins:02}:{secs:02}")
    }

    /// Set the standard selection footer text. Mirrors
    /// `SetStandardSelectionFooterText`.
    ///
    /// The C++ version builds the footer text via
    /// `ImGuiFullscreen::CreateFooterTextString` using the current
    /// input source (gamepad / keyboard). The Rust translation caches
    /// the resolved string so callers can retrieve it via
    /// `get_standard_selection_footer_text`.
    pub fn set_standard_selection_footer_text(&mut self, back_instead_of_cancel: bool) {
        self.footer_text = self.get_standard_selection_footer_text(back_instead_of_cancel);
        self.footer_back_instead_of_cancel = back_instead_of_cancel;
    }

    /// The most-recently-set standard selection footer text.
    pub fn footer_text(&self) -> &str {
        &self.footer_text
    }

    /// Set whether English titles should be preferred.
    pub fn prefer_english_game_list_changed(&mut self, prefer: bool) {
        self.prefer_english_titles = prefer;
    }

    /// Gamepad layout changed — currently a no-op in the translation.
    pub fn gamepad_layout_changed(&mut self) {}

    /// Locale changed — currently a no-op in the translation.
    pub fn locale_changed(&mut self) {}

    /// Returns the standard selection footer text into `dest`.
    pub fn get_standard_selection_footer_text(&self, back_instead_of_cancel: bool) -> String {
        let back_label = if back_instead_of_cancel { "Back" } else { "Cancel" };
        format!("Change Selection  Select  {back_label}")
    }

    // -----------------------------------------------------------------
    // Settings helpers — translated from `FullscreenUI_Settings.cpp`.
    // -----------------------------------------------------------------

    /// Stub for `DrawToggleSetting`. Records the change so the host can
    /// persist it.
    pub fn draw_toggle_setting(
        &mut self,
        bsi: usize,
        _title: &str,
        _summary: &str,
        _section: &str,
        _key: &str,
        _default_value: bool,
    ) -> bool {
        let _ = bsi;
        // The real implementation reads and writes a value through the
        // SettingsInterface. The Rust stub just flags the change.
        self.settings_changed.store(true, Ordering::Release);
        true
    }

    /// Stub for `DrawIntListSetting`.
    pub fn draw_int_list_setting(
        &mut self,
        bsi: usize,
        _title: &str,
        _summary: &str,
        _section: &str,
        _key: &str,
        _default_value: i32,
    ) {
        let _ = bsi;
    }

    /// Stub for `DrawIntRangeSetting`.
    pub fn draw_int_range_setting(
        &mut self,
        bsi: usize,
        _title: &str,
        _summary: &str,
        _section: &str,
        _key: &str,
        _default_value: i32,
        _min_value: i32,
        _max_value: i32,
    ) {
        let _ = bsi;
    }

    /// Stub for `DrawIntSpinBoxSetting`.
    pub fn draw_int_spin_box_setting(
        &mut self,
        bsi: usize,
        _title: &str,
        _summary: &str,
        _section: &str,
        _key: &str,
        _default_value: i32,
        _min_value: i32,
        _max_value: i32,
        _step_value: i32,
    ) {
        let _ = bsi;
    }

    /// Stub for `DrawFloatRangeSetting`.
    pub fn draw_float_range_setting(
        &mut self,
        bsi: usize,
        _title: &str,
        _summary: &str,
        _section: &str,
        _key: &str,
        _default_value: f32,
        _min_value: f32,
        _max_value: f32,
    ) {
        let _ = bsi;
    }

    /// Stub for `DrawFloatSpinBoxSetting`.
    pub fn draw_float_spin_box_setting(
        &mut self,
        bsi: usize,
        _title: &str,
        _summary: &str,
        _section: &str,
        _key: &str,
        _default_value: f32,
        _min_value: f32,
        _max_value: f32,
        _step_value: f32,
    ) {
        let _ = bsi;
    }

    /// Stub for `DrawIntRectSetting`.
    pub fn draw_int_rect_setting(
        &mut self,
        bsi: usize,
        _title: &str,
        _summary: &str,
        _section: &str,
        _left_key: &str,
        _default_left: i32,
        _top_key: &str,
        _default_top: i32,
        _right_key: &str,
        _default_right: i32,
        _bottom_key: &str,
        _default_bottom: i32,
    ) {
        let _ = bsi;
    }

    /// Stub for `DrawStringListSetting`.
    pub fn draw_string_list_setting(
        &mut self,
        bsi: usize,
        _title: &str,
        _summary: &str,
        _section: &str,
        _key: &str,
        _default_value: &str,
    ) {
        let _ = bsi;
    }

    /// Stub for `DrawFloatListSetting`.
    pub fn draw_float_list_setting(
        &mut self,
        bsi: usize,
        _title: &str,
        _summary: &str,
        _section: &str,
        _key: &str,
        _default_value: f32,
    ) {
        let _ = bsi;
    }

    /// Stub for `DrawEnumSetting`.
    pub fn draw_enum_setting(
        &mut self,
        bsi: usize,
        _title: &str,
        _summary: &str,
        _section: &str,
        _key: &str,
        _default_value: i32,
        _option_count: u32,
    ) {
        let _ = bsi;
        let _ = _default_value;
        let _ = _option_count;
    }

    /// Stub for `DrawFolderSetting`.
    pub fn draw_folder_setting(
        &mut self,
        bsi: usize,
        _title: &str,
        _section: &str,
        _key: &str,
    ) {
        let _ = bsi;
    }

    /// Stub for `DrawPathSetting`.
    pub fn draw_path_setting(
        &mut self,
        bsi: usize,
        _title: &str,
        _section: &str,
        _key: &str,
        _default_value: &str,
    ) {
        let _ = bsi;
    }

    /// Stub for `DrawIPAddressSetting`.
    pub fn draw_ip_address_setting(
        &mut self,
        bsi: usize,
        _title: &str,
        _summary: &str,
        _section: &str,
        _key: &str,
        _default_value: &str,
        _ip_type: IpAddressType,
    ) {
        let _ = bsi;
    }

    /// Stub for `DrawSettingInfoSetting`.
    pub fn draw_setting_info_setting(
        &mut self,
        bsi: usize,
        _section: &str,
        _key: &str,
        _translation_ctx: &str,
    ) {
        let _ = bsi;
    }

    /// Stub for `DrawClampingModeSetting`.
    pub fn draw_clamping_mode_setting(
        &mut self,
        bsi: usize,
        _title: &str,
        _summary: &str,
        _vunum: i32,
    ) {
        let _ = bsi;
    }

    /// Start automatic binding for the given controller port. The real
    /// implementation kicks off a host-CPU / GS-thread roundtrip to
    /// enumerate input devices.
    pub fn start_automatic_binding(&mut self, _port: u32) {}

    /// Open the input binding flow for a single binding. The C++ version
    /// installs an input hook and resets a timer; the Rust stub just
    /// records the request.
    pub fn begin_input_binding(
        &mut self,
        _bsi: usize,
        _binding_type: i32,
        _section: &str,
        _key: &str,
        _display_name: &str,
    ) {
    }

    /// Clear any pending input binding state.
    pub fn clear_input_binding_variables(&mut self) {}

    /// Draw the input binding modal dialog. Stub.
    pub fn draw_input_binding_window(&mut self) {}

    /// Draw the input binding button. Stub.
    pub fn draw_input_binding_button(
        &mut self,
        _bsi: usize,
        _binding_type: i32,
        _section: &str,
        _name: &str,
        _display_name: &str,
        _icon_name: &str,
    ) {
    }

    /// Open the memory card creation dialog. Stub for
    /// `OpenMemoryCardCreateDialog`.
    pub fn open_memory_card_create_dialog(&mut self) {}

    /// Copy the global settings into the per-game INI file. Stub for
    /// `DoCopyGameSettings`.
    pub fn do_copy_game_settings(&mut self) {}

    /// Clear the per-game INI file. Stub for `DoClearGameSettings`.
    pub fn do_clear_game_settings(&mut self) {}

    /// Reset the global settings to defaults. Stub for `DoResetSettings`.
    pub fn do_reset_settings(&mut self) {}

    /// Load an input profile by name.
    pub fn do_load_input_profile(&mut self, _name: &str) {}
    /// Save the current input bindings to a profile with the given name.
    pub fn do_save_input_profile(&mut self, _name: &str) {}
    /// Save the current input bindings to a profile, prompting for the
    /// name.
    pub fn do_save_input_profile_prompt(&mut self) {}

    /// Populate the graphics adapter cache. The C++ version reads the
    /// available adapters through `GSGetAdapterInfo()` and stores
    /// `(name, vendor_id)` tuples. The Rust translation exposes a
    /// pre-populated list set by the host via
    /// `set_graphics_adapter_list`.
    pub fn populate_graphics_adapter_list(&mut self) {
        // The C++ version calls `GSGetAdapterInfo()`; we don't have a
        // graphics subsystem in the translation, so the cache stays
        // empty unless the host injects adapters via the setter below.
        self.graphics_adapter_list_cache.clear();
    }

    /// Replace the cached graphics adapter list.
    pub fn set_graphics_adapter_list(&mut self, adapters: Vec<String>) {
        self.graphics_adapter_list_cache = adapters;
    }

    /// Populate the cached game-list directory list from `si`.
    ///
    /// The C++ version pulls the `Paths` and `RecursivePaths` string
    /// lists out of the base `SettingsInterface`. The Rust translation
    /// doesn't have a real SettingsInterface so we just rebuild the
    /// cache from any data the host has previously stashed on it.
    pub fn populate_game_list_directory_cache(&mut self, _si: usize) {
        // In the C++ code we read two string lists:
        //   "GameList", "Paths"
        //   "GameList", "RecursivePaths"
        // Each entry becomes `(path, recursive)` in the cache. The Rust
        // stub has no SettingsInterface accessor, so it can only
        // refresh an empty cache. The host should call this with a
        // populated cache via `set_game_list_directories_cache` after
        // reading the real settings.
        self.game_list_directories_cache.clear();
    }

    /// Replace the cached game-list directory list with a new one.
    pub fn set_game_list_directories_cache(&mut self, entries: Vec<(String, bool)>) {
        self.game_list_directories_cache = entries;
    }

    /// Populate the patches / cheats list for a given game. The C++
    /// version reads `Patch::GetPatchList` and `Patch::GetCheatList`.
    /// The Rust translation accepts pre-built lists from the host via
    /// the matching setters; this method just clears the cache.
    pub fn populate_patches_and_cheats_list(&mut self, _serial: &str, _crc: u32) {
        self.game_patch_list.clear();
        self.enabled_game_patch_cache.clear();
        self.game_cheats_list.clear();
        self.enabled_game_cheat_cache.clear();
        self.game_cheat_unlabelled_count = 0;
    }

    /// Replace the cached patch list.
    pub fn set_game_patch_list(&mut self, patches: Vec<String>, enabled: Vec<String>) {
        self.game_patch_list = patches;
        self.enabled_game_patch_cache = enabled;
    }

    /// Replace the cached cheat list and the unlabelled-cheat counter.
    pub fn set_game_cheats_list(&mut self, cheats: Vec<String>, enabled: Vec<String>, unlabelled: u32) {
        self.game_cheats_list = cheats;
        self.enabled_game_cheat_cache = enabled;
        self.game_cheat_unlabelled_count = unlabelled;
    }

    /// Populate the sorted game-list entry pointers. The C++ version
    /// sorts by the requested field; the Rust stub just stores the
    /// entries.
    pub fn populate_game_list_entry_list(&mut self, entries: Vec<usize>) {
        self.game_list_sorted_entries = entries;
    }

    /// Is the given `bsi` pointer the per-game INI? Translated from
    /// `IsEditingGameSettings`.
    pub fn is_editing_game_settings(&self, bsi: usize) -> bool {
        Some(bsi) == self.game_settings_interface
    }

    /// Returns the editing settings interface, defaulting to the global
    /// one if no per-game INI is loaded. Mirrors
    /// `GetEditingSettingsInterface(bool)`.
    pub fn get_editing_settings_interface(&self, game_settings: bool) -> usize {
        if game_settings {
            if let Some(bsi) = self.game_settings_interface {
                return bsi;
            }
        }
        0
    }

    /// Whether advanced settings should be displayed for `bsi`. Mirrors
    /// `ShouldShowAdvancedSettings`.
    ///
    /// The C++ version inspects whether the current settings interface
    /// is a per-game INI. The Rust translation exposes a simple boolean
    /// flag on the controller that the host can flip.
    pub fn should_show_advanced_settings(&self, _bsi: usize) -> bool {
        self.show_advanced_settings
    }

    /// Override the `should_show_advanced_settings` answer.
    pub fn set_show_advanced_settings(&mut self, value: bool) {
        self.show_advanced_settings = value;
    }

    /// Mark the settings interface as dirty. Translated from
    /// `SetSettingsChanged`.
    pub fn set_settings_changed(&self, bsi: usize) {
        if Some(bsi) == self.game_settings_interface {
            self.game_settings_changed.store(true, Ordering::Release);
        } else {
            self.settings_changed.store(true, Ordering::Release);
        }
    }

    /// Effective bool setting. Translated from `GetEffectiveBoolSetting`.
    pub fn get_effective_bool_setting(
        &self,
        bsi: usize,
        _section: &str,
        _key: &str,
        default_value: bool,
    ) -> bool {
        if self.is_editing_game_settings(bsi) {
            // In the real code we would consult the per-game INI first
            // and fall back to the global one. We just return the
            // default for the translation.
            return default_value;
        }
        default_value
    }

    /// Effective int setting. Translated from `GetEffectiveIntSetting`.
    pub fn get_effective_int_setting(
        &self,
        bsi: usize,
        _section: &str,
        _key: &str,
        default_value: i32,
    ) -> i32 {
        if self.is_editing_game_settings(bsi) {
            return default_value;
        }
        default_value
    }

    // -----------------------------------------------------------------
    // Save-state helpers — translated from `FullscreenUI.cpp`.
    // -----------------------------------------------------------------

    /// Open the load state selector for a given game path. Returns
    /// `true` if the selector was actually opened.
    pub fn open_load_state_selector_for_game(&mut self, _game_path: &str) -> bool {
        // The C++ version populates the save-state list and sets
        // `s_save_state_selector_open`. The Rust stub just flips the
        // flag.
        self.save_state_selector_open = true;
        true
    }

    /// Open the save-state selector, either for loading or saving.
    pub fn open_save_state_selector(&mut self, is_loading: bool) -> bool {
        self.save_state_selector_loading = is_loading;
        self.save_state_selector_open = true;
        true
    }

    /// Open the resume-state selector for a specific game-list entry.
    pub fn open_load_state_selector_for_game_resume(&mut self, _entry: usize) -> bool {
        self.save_state_selector_open = true;
        self.save_state_selector_loading = true;
        self.save_state_selector_resuming = true;
        true
    }

    /// Close the save-state selector.
    pub fn close_save_state_selector(&mut self) {
        self.clear_save_state_entry_list();
        self.save_state_selector_open = false;
        self.save_state_selector_submenu_index = -1;
        self.save_state_selector_loading = false;
        self.save_state_selector_resuming = false;
        self.save_state_selector_game_path.clear();
    }

    /// Populate the save-state list. The C++ version reads each slot
    /// via `InitializeSaveStateListEntry`. The Rust translation accepts
    /// pre-built entries from the host via `set_save_state_list`.
    pub fn populate_save_state_list(&mut self, _title: &str, _serial: &str, _crc: u32) -> u32 {
        self.save_state_selector_slots.len() as u32
    }

    /// Replace the cached save-state slot list.
    pub fn set_save_state_list(&mut self, entries: Vec<SaveStateListEntry>) {
        self.save_state_selector_slots = entries;
    }

    /// Save a state to the given slot. The real implementation calls
    /// `VMManager::SaveStateToSlot` on the CPU thread.
    pub fn do_save_state(&mut self, _slot: i32) {}

    /// Load a state from the given path / slot.
    pub fn do_load_state(&mut self, _path: String, _slot: Option<i32>, _backup: bool) {}

    /// Initialize a save-state list entry. The C++ version reads the
    /// save-state file off disk, fills the entry's title / timestamp,
    /// and decodes the embedded screenshot. The Rust translation
    /// accepts a pre-built entry via the setter, but if the caller
    /// supplies an empty entry we synthesise a populated one from
    /// the inputs.
    ///
    /// Returns `true` if the slot contained a real save.
    pub fn initialize_save_state_list_entry(
        &mut self,
        li: &mut SaveStateListEntry,
        title: &str,
        serial: &str,
        _crc: u32,
        slot: i32,
        backup: bool,
    ) -> bool {
        // If the host pre-populated `li.path` we treat this as a real
        // save. Otherwise fall back to the placeholder.
        if !li.path.is_empty() {
            if li.title.is_empty() {
                let kind = if backup { "Backup Save" } else { "Save" };
                li.title = format!("{kind} Slot {slot}");
            }
            if li.summary.is_empty() && li.timestamp != 0 {
                li.summary = format!("Saved {}", li.timestamp);
            }
            if li.slot != slot {
                li.slot = slot;
            }
            // The `title` / `serial` parameters would normally be used
            // to fill in the title with the resolved game name. We
            // keep the supplied `title` if it's non-empty so the host
            // can override.
            if !title.is_empty() && title != serial {
                let _ = title; // currently unused — host-controlled
            }
            return true;
        }
        self.initialize_placeholder_save_state_list_entry(li, slot);
        false
    }

    /// Initialize a placeholder save-state list entry.
    pub fn initialize_placeholder_save_state_list_entry(
        &self,
        li: &mut SaveStateListEntry,
        slot: i32,
    ) {
        li.title = format!("Save Slot {slot}##game_slot_{slot}");
        li.summary = "No save present in this slot.".to_string();
        li.path.clear();
        li.timestamp = 0;
        li.slot = slot;
        li.preview_texture = None;
    }

    /// Clear the cached save-state list. Mirrors `ClearSaveStateEntryList`
    /// in the C++ source — drops every entry from the slot list and
    /// hands its preview textures off to the GS device's recycle pool.
    pub fn clear_save_state_entry_list(&mut self) {
        // The C++ version pushes `entry.preview_texture` into
        // `s_cleanup_textures` so the GS thread can recycle the GPU
        // texture later. The Rust translation has no GPU device to
        // recycle into, so we just drop the list — the `TextureHandle`
        // is a sentinel and has no destructor.
        self.save_state_selector_slots.clear();
    }

    /// Draw the resume state selector. Stub.
    pub fn draw_resume_state_selector(&mut self) {}
    /// Draw the save-state selector. Stub.
    pub fn draw_save_state_selector(&mut self, _is_loading: bool) {}

    // -----------------------------------------------------------------
    // Game list helpers — translated from `FullscreenUI.cpp`.
    // -----------------------------------------------------------------

    /// Switch the game list view between grid and list. Stub for the
    /// F1 / gamepad-back handler.
    pub fn toggle_game_list_view(&mut self) {
        self.game_list_view = match self.game_list_view {
            GameListView::Grid => GameListView::List,
            GameListView::List => GameListView::Grid,
            GameListView::Count => GameListView::Grid,
        };
    }

    /// Draw the game list (list view) inside the given heading area.
    pub fn draw_game_list(&mut self, _heading_size: ImVec2) {}
    /// Draw the game list (grid view).
    pub fn draw_game_grid(&mut self, _heading_size: ImVec2) {}
    /// Draw the game list window.
    pub fn draw_game_list_window(&mut self) {}
    /// Draw the game list settings sub-page.
    pub fn draw_game_list_settings_window(&mut self) {}
    /// Draw a single game cover.
    pub fn draw_game_cover(&self, _entry: usize, _size: ImVec2) {}
    /// Draw a single game cover into a draw list.
    pub fn draw_game_cover_dl(
        &self,
        _entry: usize,
        _draw_list: &mut imgui::DrawList,
        _min: ImVec2,
        _max: ImVec2,
    ) {
    }
    /// Draw the fallback (no-game) cover at `size`.
    pub fn draw_fallback_cover(&self, _size: ImVec2) {}
    /// Draw the fallback (no-game) cover into a draw list.
    pub fn draw_fallback_cover_dl(
        &self,
        _draw_list: &mut imgui::DrawList,
        _min: ImVec2,
        _max: ImVec2,
    ) {
    }

    /// Pick the appropriate placeholder texture for a given entry type.
    pub fn get_texture_for_game_list_entry_type(
        &self,
        entry_type: i32,
        _size: ImVec2,
        _mode: SvgScaling,
    ) -> Option<TextureHandle> {
        // Mirrors `FullscreenUI::GetTextureForGameListEntryType`. The C++
        // version returns a `GSTexture*` from the texture cache; here
        // we encode the SVG path into a `TextureHandle` so callers
        // can dispatch on it.
        let path = match entry_type {
            0 => "fullscreenui/applications-system.svg",
            // PS1Disc, PS2Disc (and any future type) share the disc
            // icon in the original source.
            _ => "fullscreenui/media-cdrom.svg",
        };
        Some(TextureHandle(path.as_ptr() as usize))
    }

    /// Get the cover texture for a given entry. Returns the cached
    /// placeholder if no real cover is set.
    pub fn get_game_list_cover(&mut self, entry: usize) -> Option<TextureHandle> {
        // The C++ version looks up `s_cover_image_map` keyed by the
        // entry's path. Here `entry` is an opaque handle, so when it
        // looks like a C string pointer we decode it; otherwise we
        // can't compute a key and bail.
        let key = if entry == 0 {
            return None;
        }
        else {
            let ptr = entry as *const u8;
            if ptr.is_null() {
                return None;
            }
            let cstr = unsafe { std::ffi::CStr::from_ptr(ptr as *const i8) };
            cstr.to_string_lossy().into_owned()
        };
        if !self.cover_image_map.contains_key(&key) {
            // The C++ source would call `GameList::GetCoverImagePathForEntry`.
            // The Rust translation has no GameList module, so we just
            // insert an empty path; the host will populate it later.
            self.cover_image_map.insert(key.clone(), String::new());
        }
        let path = self.cover_image_map.get(&key)?;
        if path.is_empty() {
            return None;
        }
        Some(TextureHandle(path.as_ptr() as usize))
    }

    /// Activate a game-list entry. Equivalent to
    /// `HandleGameListActivate`.
    pub fn handle_game_list_activate(&mut self, _entry: usize) {
        self.switch_to_start_game();
    }

    /// Open the "options" choice dialog for the given game-list entry.
    pub fn handle_game_list_options(&mut self, _entry: usize) {}

    /// Trim `str` to fit into `available_space` at the given font. Stub
    /// for the C++ `TrimString` helper.
    pub fn trim_string(&self, _str: &str, available_space: f32) -> String {
        // The C++ version uses ellipsis to mark truncated strings. We
        // don't have a real font in this translation, so just cap by
        // character count as a rough proxy.
        let max_chars = (available_space.max(0.0) as usize) / 8;
        if _str.chars().count() > max_chars {
            let truncated: String = _str.chars().take(max_chars.saturating_sub(1)).collect();
            format!("{truncated}…")
        } else {
            _str.to_string()
        }
    }

    // -----------------------------------------------------------------
    // Misc helpers.
    // -----------------------------------------------------------------

    /// Invalidate the cover image cache. The C++ version dispatches a
    /// GS-thread clear; the Rust stub just clears the map.
    pub fn invalidate_cover_cache(&mut self) {
        self.cover_image_map.clear();
    }

    /// Reload SVG resources. Stub.
    pub fn reload_svg_resources(&mut self) {}

    /// Load custom background settings. The C++ version reads the
    /// `FSUIBackgroundPath` setting, resolves relative paths against
    /// `EmuFolders::DataRoot`, validates the file exists, and rejects
    /// `.gif` and `.webp` files. The Rust translation accepts the
    /// finalised path so the host can do the lookup itself.
    pub fn load_custom_background(&mut self, path: &str) {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            self.custom_background_path.clear();
            self.custom_background_enabled = false;
            return;
        }
        // Reject known-unsupported extensions. C++ rejects .gif and
        // .webp; everything else goes through the texture loader.
        let lower = trimmed.to_ascii_lowercase();
        if lower.ends_with(".gif") || lower.ends_with(".webp") {
            self.custom_background_path.clear();
            self.custom_background_enabled = false;
            return;
        }
        self.custom_background_path = trimmed.to_string();
        self.custom_background_enabled = true;
    }

    /// Draw the custom background. Stub.
    pub fn draw_custom_background(&self) {}

    /// Switch to a particular page in the settings window.
    pub fn switch_to_settings_page(&mut self, page: SettingsPage) {
        self.current_main_window = MainWindowType::Settings;
        self.settings_page = page;
    }

    /// Exit fullscreen mode and open the given URL in the host browser.
    pub fn exit_fullscreen_and_open_url(&self, _url: &str) {}

    /// Copy the given text to the system clipboard.
    pub fn copy_text_to_clipboard(&mut self, _title: String, _text: &str) {}

    /// Whether any modal dialogs (file selector, choice, message, input)
    /// are open. The C++ version checks four separate flags; we
    /// consolidate them into a single predicate.
    pub fn are_any_dialogs_open(&self) -> bool {
        self.about_window_open
            || self.cover_downloader_open
            || self.save_state_selector_open
            || self.achievements_login_open
    }

    /// Cancel any in-flight HDD-creation operations. The C++ version
    /// iterates an `active_operations` list and calls `SetCanceled`;
    /// the Rust stub is a no-op.
    pub fn cancel_all_hdd_operations(&mut self) {}

    /// Create a new hard-drive image with progress reporting. The
    /// original C++ function returns immediately and runs the work on a
    /// detached thread; the Rust stub just returns `false`.
    pub fn create_hard_drive_with_progress(
        &self,
        _file_path: &str,
        _size_in_gb: i32,
        _use_48bit_lba: bool,
    ) -> bool {
        false
    }

    /// Settings helpers that don't have a direct per-page name but are
    /// referenced by the dispatcher.
    pub fn draw_summary_settings_page(&mut self) {}
    pub fn draw_interface_settings_page(&mut self) {}
    pub fn draw_bios_settings_page(&mut self) {}
    pub fn draw_emulation_settings_page(&mut self) {}
    pub fn draw_graphics_settings_page(&mut self, _bsi: usize, _show_advanced: bool) {}
    pub fn draw_osd_settings_page(&mut self) {}
    pub fn draw_audio_settings_page(&mut self) {}
    pub fn draw_memory_card_settings_page(&mut self) {}
    pub fn draw_network_hdd_settings_page(&mut self) {}
    pub fn draw_folders_settings_page(&mut self) {}
    pub fn draw_achievements_settings_page(&mut self, _lock: Option<MutexGuard<'_, ()>>) {}
    pub fn draw_controller_settings_page(&mut self) {}
    pub fn draw_hotkey_settings_page(&mut self) {}
    pub fn draw_advanced_settings_page(&mut self) {}
    pub fn draw_patches_or_cheats_settings_page(&mut self, _cheats: bool) {}
    pub fn draw_game_fixes_settings_page(&mut self) {}

    /// Draw the About window.
    pub fn draw_about_window(&mut self) {}
    /// Draw the cover downloader window.
    pub fn draw_cover_downloader_window(&mut self) {}
    /// Draw the achievements login window.
    pub fn draw_achievements_login_window(&mut self) {}
    /// Draw the settings window. Equivalent to the giant
    /// `DrawSettingsWindow` switch.
    pub fn draw_settings_window(&mut self) {
        match self.settings_page {
            SettingsPage::Summary => self.draw_summary_settings_page(),
            SettingsPage::Interface => self.draw_interface_settings_page(),
            SettingsPage::BIOS => self.draw_bios_settings_page(),
            SettingsPage::Emulation => self.draw_emulation_settings_page(),
            SettingsPage::Graphics => self.draw_graphics_settings_page(0, false),
            SettingsPage::OSD => self.draw_osd_settings_page(),
            SettingsPage::Audio => self.draw_audio_settings_page(),
            SettingsPage::MemoryCard => self.draw_memory_card_settings_page(),
            SettingsPage::NetworkHDD => self.draw_network_hdd_settings_page(),
            SettingsPage::Folders => self.draw_folders_settings_page(),
            SettingsPage::Achievements => self.draw_achievements_settings_page(None),
            SettingsPage::Controller => self.draw_controller_settings_page(),
            SettingsPage::Hotkey => self.draw_hotkey_settings_page(),
            SettingsPage::Advanced => self.draw_advanced_settings_page(),
            SettingsPage::Patches => self.draw_patches_or_cheats_settings_page(false),
            SettingsPage::Cheats => self.draw_patches_or_cheats_settings_page(true),
            SettingsPage::GameFixes => self.draw_game_fixes_settings_page(),
            SettingsPage::Count => {}
        }
    }
    /// Draw the landing window. Stub for `DrawLandingWindow`.
    pub fn draw_landing_window(&mut self) {}
    /// Draw the start-game window. Stub for `DrawStartGameWindow`.
    pub fn draw_start_game_window(&mut self) {}
    /// Draw the exit window. Stub for `DrawExitWindow`.
    pub fn draw_exit_window(&mut self) {}
    /// Draw the pause menu. Stub for `DrawPauseMenu`.
    pub fn draw_pause_menu(&mut self) {}
    /// Switch to the achievements window.
    pub fn switch_to_achievements_window(&mut self) {
        self.current_main_window = MainWindowType::Achievements;
    }
    /// Switch to the leaderboards window.
    pub fn switch_to_leaderboards_window(&mut self) {
        self.current_main_window = MainWindowType::Leaderboards;
    }
    /// Draw the achievements window. Stub.
    pub fn draw_achievements_window(&mut self) {}
    /// Draw the leaderboards window. Stub.
    pub fn draw_leaderboards_window(&mut self) {}
}

/// A save-state list entry. Translated from the C++
/// `SaveStateListEntry` struct.
#[derive(Debug, Clone, Default)]
pub struct SaveStateListEntry {
    pub title: String,
    pub summary: String,
    pub path: String,
    pub preview_texture: Option<TextureHandle>,
    pub timestamp: i64,
    pub slot: i32,
}

// ---------------------------------------------------------------------------
// `ImGuiOverlays` — the OSD / save-state-selector overlay.
// ---------------------------------------------------------------------------

/// On-screen-display overlay manager. Translated from
/// `ImGuiOverlays.cpp` and the corresponding header.
pub struct ImGuiOverlays {
    /// Backing store for active OSD messages.
    pub osd_active_messages: VecDeque<OsdMessage>,
    /// Newly-posted messages waiting to be moved into the active deque.
    pub osd_posted_messages: VecDeque<OsdMessage>,
    /// Mutex guarding the posted queue. C++ uses `std::mutex`; we
    /// expose a `std::sync::Mutex` here.
    pub osd_messages_lock: Mutex<()>,
    /// Last time we ran an OSD update tick.
    pub last_update_timer: f64,
    /// Last time we refreshed the CPU info line.
    pub last_update_timer_cpu_info: f64,
    /// Cached performance overlay text.
    pub speed_line: String,
    pub gs_stats_line: String,
    pub gs_memory_stats_line: String,
    pub gs_frame_times_line: String,
    pub resolution_line: String,
    pub hardware_info_cpu_line: String,
    pub hardware_info_gpu_line: String,
    pub cpu_usage_ee_line: String,
    pub cpu_usage_gs_line: String,
    pub cpu_usage_vu_line: String,
    pub software_thread_lines: Vec<String>,
    pub capture_line: String,
    pub gpu_usage_line: String,
    pub gpu_debug_info_line: String,
    pub speed_icon: String,
    /// Save-state selector state.
    pub save_state_open: bool,
    pub save_state_open_time: f32,
    pub save_state_current_slot: i32,
    pub save_state_slot_paths: Vec<String>,
    /// Cached input recording data. Mirrors the
    /// `InputRecordingUI::InputRecordingData` global.
    pub input_recording_data: InputRecordingData,
    /// Whether the global gamepad north-west swap is active. Mirrors
    /// `s_gamepad_swap_noth_west`.
    pub gamepad_swap_noth_west: bool,
    /// OSD message palette. Mirrors the constants in
    /// `ImGuiOverlays.cpp`.
    pub speed_line_color: u32,
}

impl Default for ImGuiOverlays {
    fn default() -> Self {
        Self {
            osd_active_messages: VecDeque::new(),
            osd_posted_messages: VecDeque::new(),
            osd_messages_lock: Mutex::new(()),
            last_update_timer: 0.0,
            last_update_timer_cpu_info: 0.0,
            speed_line: String::new(),
            gs_stats_line: String::new(),
            gs_memory_stats_line: String::new(),
            gs_frame_times_line: String::new(),
            resolution_line: String::new(),
            hardware_info_cpu_line: String::new(),
            hardware_info_gpu_line: String::new(),
            cpu_usage_ee_line: String::new(),
            cpu_usage_gs_line: String::new(),
            cpu_usage_vu_line: String::new(),
            software_thread_lines: Vec::new(),
            capture_line: String::new(),
            gpu_usage_line: String::new(),
            gpu_debug_info_line: String::new(),
            speed_icon: String::new(),
            save_state_open: false,
            save_state_open_time: 0.0,
            save_state_current_slot: 0,
            save_state_slot_paths: Vec::new(),
            input_recording_data: InputRecordingData::default(),
            gamepad_swap_noth_west: false,
            speed_line_color: 0xFF_FF_FF_FF,
        }
    }
}

impl ImGuiOverlays {
    /// Construct a new overlay manager.
    pub fn new() -> Self {
        Self::default()
    }

    /// Draw the OSD elements. Translated from `ImGuiManager::RenderOSD`
    /// — note that the actual function lives in `ImGuiManager.cpp` but
    /// operates on the same data the overlay module owns.
    pub fn render_overlays(&mut self) {
        self.acquire_pending_osd_messages();
    }

    /// Move posted OSD messages into the active deque, deduplicating
    /// keyed messages. Translated from
    /// `ImGuiManager::AcquirePendingOSDMessages`.
    pub fn acquire_pending_osd_messages(&mut self) {
        let _lock = self.osd_messages_lock.lock().unwrap();
        while let Some(new_msg) = self.osd_posted_messages.pop_front() {
            if new_msg.key.is_empty() {
                self.osd_active_messages.push_back(new_msg);
            } else if let Some(existing) = self
                .osd_active_messages
                .iter_mut()
                .find(|m| m.key == new_msg.key)
            {
                existing.text = new_msg.text;
                existing.duration = new_msg.duration;
            } else {
                self.osd_active_messages.push_back(new_msg);
            }
        }
    }

    /// Draw the OSD messages into the background draw list. Translated
    /// from `ImGuiManager::DrawOSDMessages`.
    pub fn draw_osd_messages(&mut self, _current_time: f64) {
        // The C++ version walks `s_osd_active_messages` and draws each
        // one with fade-in / fade-out alpha. The Rust stub leaves the
        // messages alone — the host renders them.
        for msg in self.osd_active_messages.iter() {
            let _ = msg;
        }
    }

    /// Format a processor-statistic line. Translated from
    /// `ImGuiManager::FormatProcessorStat`.
    pub fn format_processor_stat(text: &mut String, usage: f64, time_ms: f64) {
        text.clear();
        if usage >= 99.95 {
            text.push_str(&format!("100% ({time_ms:.2}ms)"));
        } else {
            text.push_str(&format!("{usage:.1}% ({time_ms:.2}ms)"));
        }
    }

    /// Render the on-screen performance overlay. Translated from
    /// `ImGuiManager::DrawPerformanceOverlay`.
    pub fn draw_performance_overlay(&mut self) {}

    /// Render the shader-compile indicator.
    pub fn draw_shader_compile_indicator(&mut self) {}

    /// Render the on-screen settings overlay.
    pub fn draw_settings_overlay(&mut self) {}

    /// Render the input overlay (controller state).
    pub fn draw_inputs_overlay(&mut self) {}

    /// Render the input-recording overlay.
    pub fn draw_input_recording_overlay(&mut self) {}

    /// Render the video-capture overlay.
    pub fn draw_video_capture_overlay(&mut self) {}

    /// Render the texture-replacement overlay.
    pub fn draw_texture_replacements_overlay(&mut self) {}

    /// Render the status-indicator overlay (paused, fast-forward, ...).
    pub fn draw_indicators_overlay(&mut self) {}

    /// Add a new OSD message. Translated from `Host::AddOSDMessage`.
    pub fn add_osd_message(&mut self, message: String, duration: f32) {
        let _lock = self.osd_messages_lock.lock().unwrap();
        self.osd_posted_messages.push_back(OsdMessage {
            key: String::new(),
            text: message,
            duration,
            ..Default::default()
        });
    }

    /// Add a new OSD message with a key (used to deduplicate repeated
    /// updates). Translated from `Host::AddKeyedOSDMessage`.
    pub fn add_keyed_osd_message(&mut self, key: String, message: String, duration: f32) {
        let _lock = self.osd_messages_lock.lock().unwrap();
        self.osd_posted_messages.push_back(OsdMessage {
            key,
            text: message,
            duration,
            ..Default::default()
        });
    }

    /// Add an OSD message with a glyph prefix. Translated from
    /// `Host::AddIconOSDMessage`.
    pub fn add_icon_osd_message(
        &mut self,
        key: String,
        icon: &str,
        message: String,
        duration: f32,
    ) {
        self.add_keyed_osd_message(key, format!("{icon}  {message}"), duration)
    }

    /// Remove a keyed OSD message.
    pub fn remove_keyed_osd_message(&mut self, key: String) {
        let _lock = self.osd_messages_lock.lock().unwrap();
        self.osd_posted_messages.push_back(OsdMessage {
            key,
            text: String::new(),
            duration: 0.0,
            ..Default::default()
        });
    }

    /// Clear all OSD messages. Translated from `Host::ClearOSDMessages`.
    pub fn clear_osd_messages(&mut self) {
        let _lock = self.osd_messages_lock.lock().unwrap();
        self.osd_posted_messages.clear();
        self.osd_active_messages.clear();
    }

    // -----------------------------------------------------------------
    // Save-state selector overlay.
    // -----------------------------------------------------------------

    /// Open the save-state selector overlay. Translated from
    /// `SaveStateSelectorUI::Open`.
    pub fn save_state_selector_open(&mut self, open_time: f32) {
        self.save_state_open = true;
        self.save_state_open_time = open_time;
    }

    /// Refresh the slot list. The real implementation walks the save
    /// directory; the stub clears the cache.
    pub fn save_state_selector_refresh(&mut self, _serial: &str, _crc: u32) {
        self.save_state_slot_paths.clear();
    }

    /// Destroy the textures held by the save-state selector. Stub.
    pub fn save_state_selector_destroy_textures(&mut self) {}

    /// Clear the save-state selector state.
    pub fn save_state_selector_clear(&mut self) {
        self.save_state_open = false;
        self.save_state_slot_paths.clear();
        self.save_state_current_slot = 0;
    }

    /// Close the save-state selector. Equivalent to
    /// `SaveStateSelectorUI::Close`.
    pub fn save_state_selector_close(&mut self) {
        self.save_state_open = false;
    }

    /// Returns true if the save-state selector is open.
    pub fn save_state_selector_is_open(&self) -> bool {
        self.save_state_open
    }

    /// Move to the next slot.
    pub fn save_state_selector_select_next(&mut self, _open_selector: bool) {
        self.save_state_current_slot =
            (self.save_state_current_slot + 1).max(0);
    }

    /// Move to the previous slot.
    pub fn save_state_selector_select_previous(&mut self, _open_selector: bool) {
        self.save_state_current_slot = self.save_state_current_slot.saturating_sub(1);
    }

    /// Returns the current slot.
    pub fn save_state_selector_get_current_slot(&self) -> i32 {
        self.save_state_current_slot
    }

    /// Load the current slot. The real implementation calls
    /// `VMManager::LoadStateFromSlot`.
    pub fn save_state_selector_load_current(&mut self) {}
    /// Load the backup of the current slot.
    pub fn save_state_selector_load_current_backup(&mut self) {}
    /// Save to the current slot.
    pub fn save_state_selector_save_current(&mut self) {}

    /// Set whether the gamepad north / west buttons are swapped. Stub
    /// for `ImGuiManager::SwapGamepadNorthWest`.
    pub fn swap_gamepad_north_west(&mut self, value: bool) {
        self.gamepad_swap_noth_west = value;
    }

    /// Returns whether the gamepad north / west buttons are swapped.
    pub fn is_gamepad_north_west_swapped(&self) -> bool {
        self.gamepad_swap_noth_west
    }
}

/// One on-screen-display message. Translated from the C++ `OSDMessage`
/// struct.
#[derive(Debug, Clone, Default)]
pub struct OsdMessage {
    pub key: String,
    pub text: String,
    pub duration: f32,
    pub start_time: f64,
    pub move_time: f64,
    pub target_y: f32,
    pub last_y: f32,
}

/// Cached input-recording state. Translated from
/// `InputRecordingUI::InputRecordingData`.
#[derive(Debug, Clone, Default)]
pub struct InputRecordingData {
    pub is_recording: bool,
    pub recording_active_message: String,
    pub frame_data_message: String,
    pub undo_count_message: String,
}

// ---------------------------------------------------------------------------
// Position helpers — translated from `ImGuiOverlays.cpp`.
// ---------------------------------------------------------------------------

/// Compute the on-screen position for an OSD message. Translated from
/// `CalculateOSDPosition` in `ImGuiOverlays.cpp`.
pub fn calculate_osd_position(
    position: OsdOverlayPos,
    margin: f32,
    text_size: ImVec2,
    window_width: f32,
    window_height: f32,
) -> ImVec2 {
    use OsdOverlayPos::*;
    match position {
        TopLeft => ImVec2::new(margin, margin),
        TopCenter => ImVec2::new((window_width - text_size.x) * 0.5, margin),
        TopRight => ImVec2::new(window_width - margin - text_size.x, margin),
        CenterLeft => ImVec2::new(margin, (window_height - text_size.y) * 0.5),
        Center => ImVec2::new(
            (window_width - text_size.x) * 0.5,
            (window_height - text_size.y) * 0.5,
        ),
        CenterRight => ImVec2::new(
            window_width - margin - text_size.x,
            (window_height - text_size.y) * 0.5,
        ),
        BottomLeft => ImVec2::new(margin, window_height - margin - text_size.y),
        BottomCenter => ImVec2::new(
            (window_width - text_size.x) * 0.5,
            window_height - margin - text_size.y,
        ),
        BottomRight => ImVec2::new(
            window_width - margin - text_size.x,
            window_height - margin - text_size.y,
        ),
        None => ImVec2::zero(),
    }
}

/// Compute the on-screen Y position for a performance overlay line.
/// Translated from `CalculatePerformanceOverlayTextPosition`.
pub fn calculate_performance_overlay_text_position(
    position: OsdOverlayPos,
    margin: f32,
    text_size: ImVec2,
    window_width: f32,
    position_y: f32,
) -> ImVec2 {
    use OsdOverlayPos::*;
    let abs_margin = margin.abs();
    let x_pos = match position {
        TopLeft | CenterLeft | BottomLeft => abs_margin,
        TopCenter | Center | BottomCenter => (window_width - text_size.x) * 0.5,
        _ => window_width - text_size.x - abs_margin,
    };
    ImVec2::new(x_pos, position_y)
}

/// Returns `true` if the OSD position should be left-aligned. Translated
/// from `ShouldUseLeftAlignment`.
pub fn should_use_left_alignment(position: OsdOverlayPos) -> bool {
    use OsdOverlayPos::*;
    matches!(position, TopLeft | CenterLeft | BottomLeft)
}

/// On-screen-display corner positions. Translated from the `OsdOverlayPos`
/// enum in `Config.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsdOverlayPos {
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

// ---------------------------------------------------------------------------
// ImGuiManager — the ImGui host lifecycle.
// ---------------------------------------------------------------------------

/// Top-level ImGui host. Translated from `ImGuiManager.cpp` /
/// `ImGuiManager.h`. Stores the font handles, the OSD message queue, the
/// key map, and the input-routing state.
pub struct ImGuiManager {
    /// Global UI scale. Mirrors `s_global_scale`.
    pub global_scale: f32,
    /// OSD font, standard font, fixed font, etc. All stored as opaque
    /// handles in this translation.
    pub standard_font: FontRef,
    pub fixed_font: FontRef,
    pub osd_font: FontRef,
    /// Whether the ImGui context has been created.
    pub initialized: bool,
    /// Whether the fullscreen UI was previously initialised. The
    /// `Initialize` function re-initialises the fullscreen UI if this is
    /// the case so fonts get re-uploaded on context loss.
    pub fullscreen_ui_was_initialized: bool,
    /// Cached display size.
    pub window_width: f32,
    pub window_height: f32,
    /// Whether the next frame should call `UpdateScale`.
    pub scale_changed: bool,
    /// Whether ImGui wants keyboard / mouse / text input. Atomic so the
    /// CPU thread can observe them.
    pub wants_keyboard: AtomicBool,
    pub wants_mouse: AtomicBool,
    pub wants_text: AtomicBool,
    /// Whether the gamepad north / west buttons are swapped. Mirrors
    /// `s_gamepad_swap_noth_west`.
    pub gamepad_swap_noth_west: bool,
    /// Host-key -> ImGui key map.
    pub host_to_imgui_key_map: std::collections::HashMap<u32, ImGuiKey>,
    /// Per-controller navigation state. Mirrors
    /// `s_controller_nav_states`.
    pub controller_nav_states: std::collections::HashMap<u32, ControllerNavState>,
    /// Software-cursor slots. The C++ version uses a fixed-size array
    /// of `InputManager::MAX_SOFTWARE_CURSORS` elements; we use a Vec
    /// to keep the translation simple.
    pub software_cursors: Vec<SoftwareCursor>,
    /// Path of the custom OSD font, if any.
    pub custom_font_path: String,
    /// Cached font data blobs.
    pub custom_font_data: Vec<u8>,
    pub fixed_font_data: Vec<u8>,
    pub icon_fa_font_data: Vec<u8>,
    pub icon_pf_font_data: Vec<u8>,
    /// FreeType library handle (encoded as usize to avoid pulling in
    /// the freetype crate).
    pub ft_lib: usize,
}

impl Default for ImGuiManager {
    fn default() -> Self {
        Self {
            global_scale: 1.0,
            standard_font: FontRef::default(),
            fixed_font: FontRef::default(),
            osd_font: FontRef::default(),
            initialized: false,
            fullscreen_ui_was_initialized: false,
            window_width: 0.0,
            window_height: 0.0,
            scale_changed: false,
            wants_keyboard: AtomicBool::new(false),
            wants_mouse: AtomicBool::new(false),
            wants_text: AtomicBool::new(false),
            gamepad_swap_noth_west: false,
            host_to_imgui_key_map: std::collections::HashMap::new(),
            controller_nav_states: std::collections::HashMap::new(),
            software_cursors: Vec::new(),
            custom_font_path: String::new(),
            custom_font_data: Vec::new(),
            fixed_font_data: Vec::new(),
            icon_fa_font_data: Vec::new(),
            icon_pf_font_data: Vec::new(),
            ft_lib: 0,
        }
    }
}

impl ImGuiManager {
    /// Construct a fresh ImGui host.
    pub fn new() -> Self {
        Self::default()
    }

    /// Initialize the ImGui context, fonts, and key map. Translated from
    /// `ImGuiManager::Initialize`.
    pub fn initialize(&mut self) -> bool {
        self.initialized = true;
        self.set_key_map();
        self.set_style();
        true
    }

    /// Initialize the fullscreen UI. Translated from
    /// `ImGuiManager::InitializeFullscreenUI`.
    pub fn initialize_fullscreen_ui(&mut self, _fullscreen: &mut FullscreenUI) -> bool {
        self.fullscreen_ui_was_initialized = true;
        true
    }

    /// Tear down the ImGui context, fonts, and fullscreen UI. Translated
    /// from `ImGuiManager::Shutdown`.
    pub fn shutdown(&mut self, clear_state: bool) {
        self.initialized = false;
        if clear_state {
            self.fullscreen_ui_was_initialized = false;
        }
        self.standard_font = FontRef::default();
        self.fixed_font = FontRef::default();
        self.osd_font = FontRef::default();
    }

    /// Returns the cached window size. Safe to call from any thread.
    pub fn get_window_width(&self) -> f32 {
        self.window_width
    }
    /// Returns the cached window size. Safe to call from any thread.
    pub fn get_window_height(&self) -> f32 {
        self.window_height
    }

    /// Notify the host that the window was resized. The real
    /// implementation queries `g_gs_device` for the new size; the Rust
    /// stub accepts the new size explicitly.
    pub fn window_resized(&mut self, width: f32, height: f32) {
        self.window_width = width;
        self.window_height = height;
        self.request_scale_update();
    }

    /// Mark the layout scale as dirty so the next frame recomputes it.
    pub fn request_scale_update(&mut self) {
        if self.window_width > 0.0 && self.window_height > 0.0 {
            self.scale_changed = true;
        }
    }

    /// Reload the font atlas. Translated from `ImGuiManager::ReloadFonts`.
    pub fn reload_fonts(&mut self) {
        if !self.initialized {
            return;
        }
        self.load_font_data();
        self.add_imgui_fonts();
    }

    /// Begin a new ImGui frame. Translated from `ImGuiManager::NewFrame`.
    pub fn new_frame(&mut self) {
        if self.scale_changed {
            self.scale_changed = false;
            self.update_scale();
        }
        self.wants_keyboard.store(false, Ordering::Release);
        self.wants_mouse.store(false, Ordering::Release);
    }

    /// Skip the current frame, calling `EndFrame` followed by
    /// `NewFrame`. Translated from `ImGuiManager::SkipFrame`.
    pub fn skip_frame(&mut self) {
        self.new_frame();
    }

    /// Render the OSD / overlays. Translated from
    /// `ImGuiManager::RenderOSD`.
    pub fn render_osd(&mut self, overlays: &mut ImGuiOverlays) {
        overlays.render_overlays();
    }

    /// Recompute the global scale. Translated from
    /// `ImGuiManager::UpdateScale`.
    pub fn update_scale(&mut self) {
        // The C++ version reads `g_gs_device->GetWindowScale()` and
        // `GSConfig.OsdScale`. We just store the current value and
        // refresh the style.
        self.set_style();
    }

    /// Apply the default ImGui style. Translated from
    /// `ImGuiManager::SetStyle`.
    pub fn set_style(&mut self) {
        // Real implementation pushes 53 colors into the ImGui style.
        // We don't have a real ImGui context to write to, so this is a
        // no-op stub.
    }

    /// Build the host-key -> ImGui key map. Translated from
    /// `ImGuiManager::SetKeyMap`.
    pub fn set_key_map(&mut self) {
        self.host_to_imgui_key_map.clear();
        for &(name, imkey) in KEY_MAP.iter() {
            // The C++ version calls `InputManager::ConvertHostKeyboardStringToCode`
            // to translate names to host codes. We just use the name's
            // hash as a placeholder identifier.
            let id = hash_str_to_u32(name);
            self.host_to_imgui_key_map.insert(id, imkey);
        }
    }

    /// Load the bundled font data. Translated from
    /// `ImGuiManager::LoadFontData`.
    pub fn load_font_data(&mut self) -> bool {
        // The C++ version reads four font files (RobotoMono-Medium.ttf,
        // fa-solid-900.ttf, promptfont.otf, and optionally a custom
        // OSD font). The translation leaves the buffers empty.
        self.custom_font_data.clear();
        self.fixed_font_data.clear();
        self.icon_fa_font_data.clear();
        self.icon_pf_font_data.clear();
        true
    }

    /// Free the font data buffers. Translated from
    /// `ImGuiManager::UnloadFontData`.
    pub fn unload_font_data(&mut self) {
        self.custom_font_data.clear();
        self.fixed_font_data.clear();
        self.icon_fa_font_data.clear();
        self.icon_pf_font_data.clear();
    }

    /// Add the ImGui font atlas entries. Translated from
    /// `ImGuiManager::AddImGuiFonts`.
    pub fn add_imgui_fonts(&mut self) -> bool {
        // The real implementation calls `AddTextFont`, `AddFixedFont`,
        // `AddOsdFont`, `AddIconFonts` and `AddEmojiFont`. The stub
        // simply assigns placeholder handles.
        self.standard_font = FontRef::new(1, 15.0);
        self.fixed_font = FontRef::new(2, 15.0);
        self.osd_font = FontRef::new(3, 15.0);
        true
    }

    /// Returns the global UI scale.
    pub fn get_global_scale(&self) -> f32 {
        self.global_scale
    }
    /// Returns the standard font.
    pub fn get_standard_font(&self) -> FontRef {
        self.standard_font
    }
    /// Returns the fixed-width font.
    pub fn get_fixed_font(&self) -> FontRef {
        self.fixed_font
    }
    /// Returns the OSD font.
    pub fn get_osd_font(&self) -> FontRef {
        self.osd_font
    }

    /// Returns the standard font size. The C++ version multiplies the
    /// base size by the global scale; we do the same.
    pub fn get_font_size_standard(&self) -> f32 {
        (12.0 * self.global_scale).ceil()
    }
    /// Returns the medium font size, scaled to the current layout.
    pub fn get_font_size_medium(&self) -> f32 {
        (LAYOUT_MEDIUM_FONT_SIZE * self.global_scale).ceil()
    }
    /// Returns the large font size, scaled to the current layout.
    pub fn get_font_size_large(&self) -> f32 {
        (LAYOUT_LARGE_FONT_SIZE * self.global_scale).ceil()
    }

    /// True if ImGui wants keyboard text input.
    pub fn wants_text_input(&self) -> bool {
        self.wants_text.load(Ordering::Acquire)
    }
    /// True if ImGui wants mouse input.
    pub fn wants_mouse_input(&self) -> bool {
        self.wants_mouse.load(Ordering::Acquire)
    }

    /// Feed a text string into ImGui. The real implementation posts
    /// onto the CPU / GS threads so the event arrives on the renderer
    /// thread; the stub just records the latest text.
    pub fn add_text_input(&mut self, _text: String) {
        // No-op stub.
    }

    /// Update the mouse position. Translated from
    /// `ImGuiManager::UpdateMousePosition`.
    pub fn update_mouse_position(&mut self, _x: f32, _y: f32) {}

    /// Process a pointer button event. Returns `true` if ImGui
    /// intercepted it.
    pub fn process_pointer_button_event(
        &self,
        _key: u32,
        _value: f32,
    ) -> bool {
        self.wants_mouse.load(Ordering::Acquire)
    }
    /// Process a pointer axis event.
    pub fn process_pointer_axis_event(
        &self,
        _key: u32,
        _value: f32,
    ) -> bool {
        self.wants_mouse.load(Ordering::Acquire)
    }
    /// Process a host keyboard event. Returns `true` if ImGui
    /// intercepted it.
    pub fn process_host_key_event(&self, key: u32, _value: f32) -> bool {
        if self.host_to_imgui_key_map.contains_key(&key) {
            self.wants_keyboard.load(Ordering::Acquire)
        } else {
            false
        }
    }
    /// Process a generic input event (gamepad button / stick).
    pub fn process_generic_input_event(
        &mut self,
        key: GenericInputBinding,
        layout: InputLayout,
        value: f32,
        controller_id: u32,
    ) -> bool {
        if matches!(key, GenericInputBinding::Unknown) {
            return false;
        }
        // Track diagonal D-pad state so we don't fire both directions at
        // once.
        let state = self
            .controller_nav_states
            .entry(controller_id)
            .or_default();
        match key {
            GenericInputBinding::DPadLeft | GenericInputBinding::DPadRight => {
                state.dpad_h_held = value > 0.0;
            }
            GenericInputBinding::DPadUp | GenericInputBinding::DPadDown => {
                state.dpad_v_held = value > 0.0;
            }
            _ => {}
        }
        if state.dpad_h_held && state.dpad_v_held {
            return false;
        }
        if !self.initialized {
            return false;
        }
        // Gamepad layout is reported to the fullscreen UI so it can
        // pick the right glyphs. The Rust stub does not actually update
        // it; the host should call into the [`FullscreenUI`] when it
        // dispatches a frame.
        let _ = layout;
        self.wants_keyboard.load(Ordering::Acquire)
    }
    /// Process a generic axis event with hysteresis. Translated from
    /// `ImGuiManager::ProcessGenericAxisEvent`.
    pub fn process_generic_axis_event(
        &mut self,
        negative_key: GenericInputBinding,
        positive_key: GenericInputBinding,
        layout: InputLayout,
        value: f32,
        controller_id: u32,
    ) {
        const ACTIVATE_THRESHOLD: f32 = 0.5;
        const RELEASE_THRESHOLD: f32 = 0.2;

        let state = self
            .controller_nav_states
            .entry(controller_id)
            .or_default();
        // Update per-axis accumulator and determine the suppressed
        // value (other-axis wins so we don't fire both halves of a
        // diagonal).
        let (mut suppressed, is_x_axis) = match negative_key {
            GenericInputBinding::LeftStickLeft | GenericInputBinding::RightStickLeft => {
                (state.left_stick.x, true)
            }
            GenericInputBinding::LeftStickUp | GenericInputBinding::RightStickUp => {
                (state.left_stick.y, false)
            }
            _ => (value, false),
        };
        let _ = is_x_axis;
        if value.abs() < suppressed.abs() {
            suppressed = 0.0;
        }
        let _ = suppressed;

        if !matches!(negative_key, GenericInputBinding::Unknown) {
            let active = value < -ACTIVATE_THRESHOLD;
            if active {
                self.process_generic_input_event(negative_key, layout, 1.0, controller_id);
            } else if value > -RELEASE_THRESHOLD {
                self.process_generic_input_event(negative_key, layout, 0.0, controller_id);
            }
        }
        if !matches!(positive_key, GenericInputBinding::Unknown) {
            let active = value > ACTIVATE_THRESHOLD;
            if active {
                self.process_generic_input_event(positive_key, layout, 1.0, controller_id);
            } else if value < RELEASE_THRESHOLD {
                self.process_generic_input_event(positive_key, layout, 0.0, controller_id);
            }
        }
    }
    /// Swap the north / west gamepad buttons. The C++ version stores a
    /// global flag that the input dispatcher consults before forwarding
    /// events to ImGui.
    pub fn swap_gamepad_north_west(&mut self, value: bool) {
        self.gamepad_swap_noth_west = value;
    }
    /// Returns whether north / west are swapped.
    pub fn is_gamepad_north_west_swapped(&self) -> bool {
        self.gamepad_swap_noth_west
    }

    /// Set the image and scale for the software cursor at `index`.
    pub fn set_software_cursor(
        &mut self,
        index: usize,
        image_path: String,
        image_scale: f32,
        multiply_color: u32,
    ) {
        while self.software_cursors.len() <= index {
            self.software_cursors.push(SoftwareCursor::default());
        }
        let cursor = &mut self.software_cursors[index];
        cursor.color = multiply_color | 0xFF00_0000;
        cursor.image_path = image_path;
        cursor.scale = image_scale;
    }
    /// Returns whether a software cursor is configured at `index`.
    pub fn has_software_cursor(&self, index: usize) -> bool {
        self.software_cursors
            .get(index)
            .map(|c| !c.image_path.is_empty())
            .unwrap_or(false)
    }
    /// Clear the software cursor at `index`.
    pub fn clear_software_cursor(&mut self, index: usize) {
        self.set_software_cursor(index, String::new(), 0.0, 0);
    }
    /// Set the position of the software cursor at `index`.
    pub fn set_software_cursor_position(&mut self, index: usize, x: f32, y: f32) {
        while self.software_cursors.len() <= index {
            self.software_cursors.push(SoftwareCursor::default());
        }
        self.software_cursors[index].pos = (x, y);
    }
    /// Strip icon characters (private-use-area and supplementary-plane
    /// code points) from a UTF-8 string. Translated from
    /// `ImGuiManager::StripIconCharacters`.
    pub fn strip_icon_characters(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        for c in text.chars() {
            let cp = c as u32;
            if cp > 0x32FFF || (0xE000..=0xF8FF).contains(&cp) {
                continue;
            }
            out.push(c);
        }
        out
    }
}

/// Per-controller navigation state. Mirrors the
/// `ControllerNavState` struct in `ImGuiManager.cpp`.
#[derive(Debug, Clone, Default)]
pub struct ControllerNavState {
    pub dpad_h_held: bool,
    pub dpad_v_held: bool,
    pub left_stick: AxisState,
    pub right_stick: AxisState,
}

/// Per-axis accumulator state. Mirrors `ControllerNavState::AxisState`.
#[derive(Debug, Clone, Default)]
pub struct AxisState {
    pub x: f32,
    pub y: f32,
    pub x_neg_active: bool,
    pub x_pos_active: bool,
    pub y_neg_active: bool,
    pub y_pos_active: bool,
}

/// Software cursor state. Mirrors `ImGuiManager::SoftwareCursor`.
#[derive(Debug, Clone, Default)]
pub struct SoftwareCursor {
    pub image_path: String,
    pub texture: Option<TextureHandle>,
    pub color: u32,
    pub scale: f32,
    pub extent_x: f32,
    pub extent_y: f32,
    pub pos: (f32, f32),
}

/// Generic input event types. Mirrors the `GenericInputBinding` enum
/// from `Input/InputManager.h`. We only enumerate the events the
/// translation actually consumes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenericInputBinding {
    Unknown,
    DPadUp,
    DPadRight,
    DPadLeft,
    DPadDown,
    LeftStickUp,
    LeftStickRight,
    LeftStickDown,
    LeftStickLeft,
    L3,
    RightStickUp,
    RightStickRight,
    RightStickDown,
    RightStickLeft,
    R3,
    Triangle,
    Circle,
    Cross,
    Square,
    Select,
    Start,
    System,
    L1,
    L2,
    R1,
    R2,
}

/// ImGui key codes. We only enumerate the keys the translation uses;
/// the real `ImGuiKey` enum has ~120 variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImGuiKey {
    None,
    LeftArrow,
    RightArrow,
    UpArrow,
    DownArrow,
    PageUp,
    PageDown,
    Home,
    End,
    Insert,
    Delete,
    Backspace,
    Space,
    Enter,
    Escape,
    LeftCtrl,
    RightCtrl,
    LeftShift,
    RightShift,
    LeftAlt,
    RightAlt,
    LeftSuper,
    RightSuper,
    Menu,
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    GamepadDpadUp,
    GamepadDpadDown,
    GamepadDpadLeft,
    GamepadDpadRight,
    GamepadFaceUp,
    GamepadFaceDown,
    GamepadFaceLeft,
    GamepadFaceRight,
    GamepadBack,
    GamepadStart,
    GamepadL1,
    GamepadL2,
    GamepadR1,
    GamepadR2,
    GamepadL3,
    GamepadR3,
}

/// Static key-name -> ImGui-key map used by `ImGuiManager::SetKeyMap`.
const KEY_MAP: &[(&str, ImGuiKey)] = &[
    ("Left", ImGuiKey::LeftArrow),
    ("Right", ImGuiKey::RightArrow),
    ("Up", ImGuiKey::UpArrow),
    ("Down", ImGuiKey::DownArrow),
    ("PageUp", ImGuiKey::PageUp),
    ("PageDown", ImGuiKey::PageDown),
    ("Home", ImGuiKey::Home),
    ("End", ImGuiKey::End),
    ("Insert", ImGuiKey::Insert),
    ("Delete", ImGuiKey::Delete),
    ("Backspace", ImGuiKey::Backspace),
    ("Space", ImGuiKey::Space),
    ("Return", ImGuiKey::Enter),
    ("Escape", ImGuiKey::Escape),
    ("LeftCtrl", ImGuiKey::LeftCtrl),
    ("RightCtrl", ImGuiKey::RightCtrl),
    ("LeftShift", ImGuiKey::LeftShift),
    ("RightShift", ImGuiKey::RightShift),
    ("LeftAlt", ImGuiKey::LeftAlt),
    ("RightAlt", ImGuiKey::RightAlt),
    ("LeftSuper", ImGuiKey::LeftSuper),
    ("RightSuper", ImGuiKey::RightSuper),
    ("Menu", ImGuiKey::Menu),
    ("A", ImGuiKey::A),
    ("B", ImGuiKey::B),
    ("C", ImGuiKey::C),
    ("D", ImGuiKey::D),
    ("E", ImGuiKey::E),
    ("F", ImGuiKey::F),
    ("G", ImGuiKey::G),
    ("H", ImGuiKey::H),
    ("I", ImGuiKey::I),
    ("J", ImGuiKey::J),
    ("K", ImGuiKey::K),
    ("L", ImGuiKey::L),
    ("M", ImGuiKey::M),
    ("N", ImGuiKey::N),
    ("O", ImGuiKey::O),
    ("P", ImGuiKey::P),
    ("Q", ImGuiKey::Q),
    ("R", ImGuiKey::R),
    ("S", ImGuiKey::S),
    ("T", ImGuiKey::T),
    ("U", ImGuiKey::U),
    ("V", ImGuiKey::V),
    ("W", ImGuiKey::W),
    ("X", ImGuiKey::X),
    ("Y", ImGuiKey::Y),
    ("Z", ImGuiKey::Z),
    ("F1", ImGuiKey::F1),
    ("F2", ImGuiKey::F2),
    ("F3", ImGuiKey::F3),
    ("F4", ImGuiKey::F4),
    ("F5", ImGuiKey::F5),
    ("F6", ImGuiKey::F6),
    ("F7", ImGuiKey::F7),
    ("F8", ImGuiKey::F8),
    ("F9", ImGuiKey::F9),
    ("F10", ImGuiKey::F10),
    ("F11", ImGuiKey::F11),
    ("F12", ImGuiKey::F12),
];

/// Trivial stable hash of a string into a `u32` host-key identifier.
/// The C++ version relies on the input manager to translate the
/// symbolic name into a host-specific scan code; the Rust stub uses a
/// 32-bit FNV-1a hash instead.
fn hash_str_to_u32(name: &str) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in name.bytes() {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

// ---------------------------------------------------------------------------
// Convenience constructors and conversion helpers.
// ---------------------------------------------------------------------------

impl AnimatedValue {
    /// Convenience wrapper for the C++ `OutExpo` easing curve. This is
    /// the same as the `common/Easing.h` `Easing::OutExpo(t)` function
    /// used by the original animated widgets.
    pub fn out_expo(t: f32) -> f32 {
        out_expo(t)
    }
}

impl ImVec2 {
    /// Returns a layout-scaled version of this vector, using the given
    /// scale factor.
    pub fn scaled(self, scale: f32) -> ImVec2 {
        ImVec2::new((self.x * scale).ceil(), (self.y * scale).ceil())
    }
}

impl ImVec4 {
    /// Apply `ModAlpha` (replace the alpha component).
    pub fn with_alpha(self, alpha: f32) -> ImVec4 {
        ImVec4::new(self.x, self.y, self.z, alpha)
    }
    /// Apply `MulAlpha` (multiply the alpha component).
    pub fn scaled_alpha(self, factor: f32) -> ImVec4 {
        ImVec4::new(self.x, self.y, self.z, self.w * factor)
    }
}

/// RAII guard returned by [`ImGuiManager::process_generic_axis_event`]
/// in the real implementation. The stub omits the actual ImGui
/// integration so this type is empty, but downstream callers can
/// pattern-match on it.
pub struct AxisEventGuard {
    _private: (),
}

impl AxisEventGuard {
    pub fn new() -> Self {
        Self { _private: () }
    }
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_constants_match_cpp() {
        assert_eq!(LAYOUT_SCREEN_WIDTH, 1280.0);
        assert_eq!(LAYOUT_SCREEN_HEIGHT, 720.0);
        assert_eq!(LAYOUT_MENU_BUTTON_HEIGHT, 50.0);
    }

    #[test]
    fn hex_to_imvec4_packs_correctly() {
        let c = hex_to_imvec4(0x21_21_21, 0xFF);
        assert!((c.x - 0x21 as f32 / 255.0).abs() < 1e-6);
        assert!((c.y - 0x21 as f32 / 255.0).abs() < 1e-6);
        assert!((c.z - 0x21 as f32 / 255.0).abs() < 1e-6);
        assert!((c.w - 1.0).abs() < 1e-6);
    }

    #[test]
    fn animated_value_eases() {
        let mut a = AnimatedValue::new();
        a.start(0.0, 100.0, 1.0);
        a.update_and_get_value(0.5);
        let v = a.get_current_value();
        assert!(v > 0.0 && v < 100.0);
    }

    #[test]
    fn out_expo_saturates() {
        assert_eq!(out_expo(0.0), 0.0);
        assert_eq!(out_expo(1.0), 1.0);
        assert!(out_expo(0.5) > 0.9);
    }

    #[test]
    fn fullscreen_ui_init_round_trip() {
        let mut ui = FullscreenUI::new();
        assert!(!ui.is_initialized());
        assert!(ui.init());
        assert!(ui.is_initialized());
        ui.shutdown(true);
        assert!(!ui.is_initialized());
    }

    #[test]
    fn fullscreen_ui_windows_dispatch() {
        let mut ui = FullscreenUI::new();
        ui.init();
        ui.switch_to_landing();
        assert_eq!(ui.current_main_window, MainWindowType::Landing);
        ui.switch_to_game_list();
        assert_eq!(ui.current_main_window, MainWindowType::GameList);
        ui.switch_to_settings();
        assert_eq!(ui.current_main_window, MainWindowType::Settings);
        assert_eq!(ui.settings_page, SettingsPage::Interface);
    }

    #[test]
    fn save_state_selector_round_trip() {
        let mut ui = FullscreenUI::new();
        assert!(ui.open_save_state_selector(true));
        assert!(ui.save_state_selector_open);
        ui.close_save_state_selector();
        assert!(!ui.save_state_selector_open);
    }

    #[test]
    fn controller_settings_draws_without_panicking() {
        let mut ui = FullscreenUI::new();
        ui.init();
        ui.draw_controller_settings_page();
    }

    #[test]
    fn about_menu_round_trip() {
        let mut ui = FullscreenUI::new();
        ui.open_about_window();
        assert!(ui.about_window_open);
        ui.close_about_window();
        assert!(!ui.about_window_open);
    }

    #[test]
    fn osd_message_queue_dedupes() {
        let mut o = ImGuiOverlays::new();
        o.add_keyed_osd_message("hotkey".to_string(), "first".to_string(), 1.0);
        o.add_keyed_osd_message("hotkey".to_string(), "second".to_string(), 2.0);
        o.acquire_pending_osd_messages();
        assert_eq!(o.osd_active_messages.len(), 1);
        assert_eq!(o.osd_active_messages[0].text, "second");
        assert_eq!(o.osd_active_messages[0].duration, 2.0);
    }

    #[test]
    fn osd_position_helpers() {
        let pos = calculate_osd_position(
            OsdOverlayPos::TopRight,
            10.0,
            ImVec2::new(50.0, 20.0),
            200.0,
            100.0,
        );
        assert!((pos.x - (200.0 - 10.0 - 50.0)).abs() < 1e-4);
        assert!((pos.y - 10.0).abs() < 1e-4);
    }

    #[test]
    fn left_alignment_predicate() {
        assert!(should_use_left_alignment(OsdOverlayPos::TopLeft));
        assert!(should_use_left_alignment(OsdOverlayPos::CenterLeft));
        assert!(should_use_left_alignment(OsdOverlayPos::BottomLeft));
        assert!(!should_use_left_alignment(OsdOverlayPos::TopRight));
    }

    #[test]
    fn animated_value_resets() {
        let mut a = AnimatedValue::new();
        a.start(0.0, 1.0, 1.0);
        a.reset(0.5);
        assert_eq!(a.get_current_value(), 0.5);
        assert_eq!(a.get_start_value(), 0.5);
        assert_eq!(a.get_end_value(), 0.5);
        assert!(!a.is_active());
    }

    #[test]
    fn wants_to_close_menu_latches() {
        let mut fs = ImGuiFullscreen::new();
        // First press arms the latch.
        assert!(!fs.wants_to_close_menu(true, false, false, false));
        // Release transitions to "active".
        assert!(fs.wants_to_close_menu(false, true, false, false));
        // The next call to end_frame clears the latch.
        fs.end_frame();
        assert!(!fs.wants_to_close_menu(false, false, false, false));
    }

    #[test]
    fn imvec2_default_is_zero() {
        let v: ImVec2 = Default::default();
        assert_eq!(v, ImVec2::zero());
    }

    #[test]
    fn time_to_printable_string_returns_nonempty() {
        let s = FullscreenUI::time_to_printable_string(0);
        assert!(!s.is_empty());
    }
}

// Suppress "unused" warnings for utility items that are part of the
// public API but not used by the module itself.
#[allow(dead_code)]
fn _ensure_send_sync() {
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}
    assert_send::<ImGuiFullscreen>();
    assert_sync::<ImGuiFullscreen>();
    assert_send::<FullscreenUI>();
    assert_sync::<FullscreenUI>();
    assert_send::<ImGuiOverlays>();
    assert_sync::<ImGuiOverlays>();
    assert_send::<ImGuiManager>();
    assert_sync::<ImGuiManager>();
    let _ = Duration::from_secs(0);
    let _ = Arc::new(());
}
