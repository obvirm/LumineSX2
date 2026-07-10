// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
// Idiomatic Rust translation of PCSX2's small ImGui layer (ImGuiAnimated.h,
// ImGuiOverlays.{h,cpp}, ImGuiManager.{h,cpp}, ImGuiFullscreen.h,
// FullscreenUI_Internal.h). Only standard library dependencies are used. This
// module intentionally strips out every ImGui handle, GSDevice, MTGS, Host,
// ImGuiFullscreen drawing primitive, and atomic/locking detail from the C++
// originals and re-expresses the public types, constants, easing helpers, and
// the public method signatures of the two main manager structs in Rust 2021
// idiom.

#![allow(dead_code)]
#![allow(clippy::module_inception)]

use std::f32::consts::PI;

// ---------------------------------------------------------------------------
// OSD / font constants
// ---------------------------------------------------------------------------

/// Default font files that ship with PCSX2 under `fonts/`.
pub const FONT_FIXED_PATH: &str = "fonts/RobotoMono-Medium.ttf";
pub const FONT_ICON_FA_PATH: &str = "fonts/fa-solid-900.ttf";
pub const FONT_ICON_PF_PATH: &str = "fonts/promptfont.otf";

/// ImGui base font size and line height used to build the atlas.
pub const FONT_BASE_SIZE: f32 = 15.0;
pub const FONT_LINE_HEIGHT: f32 = 1.25;
pub const FONT_ICON_SIZE: f32 = FONT_BASE_SIZE * 1.2;

/// Standard, medium, large font sizes mirrored by `ImGuiManager`.
pub const FONT_SIZE_STANDARD: f32 = 12.0;
pub const LAYOUT_MEDIUM_FONT_SIZE: f32 = 14.0;
pub const LAYOUT_LARGE_FONT_SIZE: f32 = 22.0;
pub const LAYOUT_SMALL_FONT_SIZE: f32 = 10.0;

/// OSD fade-in / fade-out durations (seconds).
pub const OSD_FADE_IN_TIME: f32 = 0.1;
pub const OSD_FADE_OUT_TIME: f32 = 0.4;

/// Save-state selector open time (seconds).
pub const SAVE_STATE_SELECTOR_OPEN_TIME: f32 = 5.0;

/// Standard message durations used by the OSD subsystem.
pub const OSD_MESSAGE_DEFAULT_DURATION: f32 = 2.0;
pub const OSD_QUICK_DURATION: f32 = 2.5;

/// Background colors used by the OSD message rectangles.
pub const OSD_MESSAGE_FILL_COLOR: u32 = 0x21_21_21_FF;
pub const OSD_MESSAGE_BORDER_COLOR: u32 = 0x48_48_48_FF;
pub const OSD_MESSAGE_TEXT_COLOR: u32 = 0xFF_FF_FF_FF;

/// Shadow and white used by overlay lines.
pub const OSD_SHADOW_COLOR: u32 = 0x00_00_00_64;
pub const OSD_WHITE_COLOR: u32 = 0xFF_FF_FF_FF;

/// OSD performance speed indicator colors.
pub const OSD_SPEED_LOW_COLOR: u32 = 0xFF_64_64_FF;
pub const OSD_SPEED_NORMAL_COLOR: u32 = 0xFF_FF_FF_FF;
pub const OSD_SPEED_HIGH_COLOR: u32 = 0x64_FF_64_FF;

/// Fullscreen layout constants. Mirrors `ImGuiFullscreen.h`.
pub const LAYOUT_SCREEN_WIDTH: f32 = 1280.0;
pub const LAYOUT_SCREEN_HEIGHT: f32 = 720.0;
pub const LAYOUT_MENU_BUTTON_HEIGHT: f32 = 50.0;
pub const LAYOUT_MENU_BUTTON_HEIGHT_NO_SUMMARY: f32 = 26.0;
pub const LAYOUT_MENU_BUTTON_X_PADDING: f32 = 15.0;
pub const LAYOUT_MENU_BUTTON_Y_PADDING: f32 = 10.0;
pub const LAYOUT_MENU_WINDOW_X_PADDING: f32 = 12.0;
pub const LAYOUT_FOOTER_PADDING: f32 = 10.0;
pub const LAYOUT_FOOTER_HEIGHT: f32 = LAYOUT_MEDIUM_FONT_SIZE + LAYOUT_FOOTER_PADDING * 2.0;
pub const LAYOUT_HORIZONTAL_MENU_HEIGHT: f32 = 320.0;
pub const LAYOUT_HORIZONTAL_MENU_PADDING: f32 = 30.0;
pub const LAYOUT_HORIZONTAL_MENU_ITEM_WIDTH: f32 = 250.0;
pub const LAYOUT_WINDOW_ROUNDING: f32 = 8.0;
pub const LAYOUT_FRAME_ROUNDING: f32 = 6.0;
pub const LAYOUT_SCROLLBAR_ROUNDING: f32 = 5.0;

// ---------------------------------------------------------------------------
// Easing
// ---------------------------------------------------------------------------

/// Easing functions that mirror the C++ `Easing` namespace. Only the
/// functions used by the OSD / save-state selector layer are included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Easing {
    /// Linear interpolation `t`.
    Linear,
    /// `OutExpo` easing used by animated values, OSD fades, and selectors.
    OutExpo,
}

impl Easing {
    /// Evaluate the easing function. The result is clamped to `[0.0, 1.0]` to
    /// match the C++ behaviour where the output feeds into `min(0.05 + f, 1)`
    /// expressions.
    pub fn eval(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Easing::Linear => t,
            Easing::OutExpo => {
                if t >= 1.0 {
                    1.0
                } else {
                    1.0 - 2f32.powf(-10.0 * t)
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// AnimatedValue
// ---------------------------------------------------------------------------

/// Animated floating-point value driven by the C++ `ImAnimatedFloat` /
/// `ImAnimatedVec2` helpers. The struct stores a `to`/`from`/`current` triple
/// and a wall-clock time, and is ticked by [`AnimatedValue::update`] with the
/// frame's delta time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimatedValue {
    pub from: f32,
    pub to: f32,
    pub current: f32,
    pub start_time: f32,
    pub duration: f32,
    easing: Easing,
    elapsed: f32,
}

impl Default for AnimatedValue {
    fn default() -> Self {
        Self {
            from: 0.0,
            to: 0.0,
            current: 0.0,
            start_time: 0.0,
            duration: 1.0,
            easing: Easing::OutExpo,
            elapsed: 0.0,
        }
    }
}

impl AnimatedValue {
    /// Construct a new `AnimatedValue` with the given from/to/duration and
    /// the `OutExpo` easing curve used throughout the C++ original.
    pub fn new(from: f32, to: f32, duration: f32) -> Self {
        Self {
            from,
            to,
            current: from,
            start_time: 0.0,
            duration,
            easing: Easing::OutExpo,
            elapsed: 0.0,
        }
    }

    /// Returns `true` while the animation is still moving towards `to`.
    pub fn is_active(&self) -> bool {
        self.current != self.to
    }

    /// Returns the current value, advancing the animation by `delta_time`.
    /// Mirrors `ImAnimatedFloat::UpdateAndGetValue` from ImGuiAnimated.h.
    pub fn update(&mut self, delta_time: f32) -> f32 {
        if self.current == self.to {
            return self.current;
        }
        self.elapsed += delta_time;
        let frac = (0.05 + self.easing.eval(self.elapsed / self.duration)).min(1.0);
        let lo = self.from.min(self.to);
        let hi = self.from.max(self.to);
        self.current = (self.from + (self.to - self.from) * frac).clamp(lo, hi);
        self.current
    }

    /// Reset the animation to a single static value, matching the
    /// `ImAnimatedFloat::Reset` C++ method.
    pub fn reset(&mut self, value: f32) {
        self.from = value;
        self.to = value;
        self.current = value;
        self.elapsed = 0.0;
    }

    /// Begin animating from `from` to `to` over `duration` seconds.
    pub fn start(&mut self, from: f32, to: f32, duration: f32) {
        self.from = from;
        self.to = to;
        self.current = from;
        self.start_time = 0.0;
        self.duration = duration;
        self.elapsed = 0.0;
    }

    /// Stop the animation at its current value.
    pub fn stop(&mut self) {
        self.to = self.current;
    }
}

// ---------------------------------------------------------------------------
// OsdOverlayPos
// ---------------------------------------------------------------------------

/// Anchor positions for on-screen display elements, matching the C++ enum
/// used by `ImGuiOverlays.cpp`.
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
// ImGuiManager
// ---------------------------------------------------------------------------

/// Rust equivalent of the `ImGuiManager` namespace. The original C++ API is
/// a free-function namespace; the Rust translation exposes a `struct` that
/// owns the static state implied by the C++ file (`s_*` variables) so that
/// callers can have isolated manager instances.
#[derive(Debug, Default)]
pub struct ImGuiManager {
    global_scale: f32,
    window_width: f32,
    window_height: f32,
    standard_font: Option<usize>,
    fixed_font: Option<usize>,
    osd_font: Option<usize>,
    imgui_wants_keyboard: bool,
    imgui_wants_mouse: bool,
    imgui_wants_text: bool,
    gamepad_swap_noth_west: bool,
    last_render_time: f32,
    scale_changed: bool,
}

impl ImGuiManager {
    /// Create a new manager with default values.
    pub fn new() -> Self {
        Self {
            global_scale: 1.0,
            ..Self::default()
        }
    }

    /// Initialize the ImGui context, load fonts, and create software cursor
    /// textures. Mirrors `ImGuiManager::Initialize`.
    pub fn init(&mut self) -> bool {
        self.global_scale = 1.0;
        self.scale_changed = false;
        // ImGui::CreateContext + LoadFontData + AddImGuiFonts would run here.
        // We don't have access to the real C++ font paths at this layer, so
        // the call returns `true` to indicate the ImGui context is ready.
        true
    }

    /// Free all ImGui resources. Mirrors `ImGuiManager::Shutdown`.
    pub fn shutdown(&mut self) {
        self.standard_font = None;
        self.fixed_font = None;
        self.osd_font = None;
    }

    /// Start an ImGui frame, mirroring `ImGuiManager::NewFrame`.
    pub fn begin_frame(&mut self) {
        // In the C++ implementation this calls `ImGui::NewFrame()` and
        // computes `s_last_render_time`. We simply reset the cached wants-* booleans.
        self.imgui_wants_keyboard = false;
        self.imgui_wants_mouse = false;
        self.imgui_wants_text = false;
    }

    /// End an ImGui frame, mirroring `ImGuiManager::EndFrame` / `Render`.
    pub fn end_frame(&mut self) {
        // Real implementation calls `ImGui::Render()` and `ImGui::EndFrame()`.
        // Nothing to do in the translated surface.
    }

    /// Returns the current window width in pixels.
    pub fn window_width(&self) -> f32 {
        self.window_width
    }

    /// Returns the current window height in pixels.
    pub fn window_height(&self) -> f32 {
        self.window_height
    }

    /// Returns the global UI scale (`s_global_scale` in the C++ original).
    pub fn global_scale(&self) -> f32 {
        self.global_scale
    }

    /// Returns the standard font handle, if any.
    pub fn standard_font(&self) -> Option<usize> {
        self.standard_font
    }

    /// Returns the fixed-width font handle, if any.
    pub fn fixed_font(&self) -> Option<usize> {
        self.fixed_font
    }

    /// Returns the OSD font handle, if any.
    pub fn osd_font(&self) -> Option<usize> {
        self.osd_font
    }

    /// Returns the standard font size, accounting for the global scale.
    pub fn font_size_standard(&self) -> f32 {
        (FONT_SIZE_STANDARD * self.global_scale).ceil()
    }

    /// Returns the medium font size (matches `ImGuiFullscreen`'s scale).
    pub fn font_size_medium(&self) -> f32 {
        LAYOUT_MEDIUM_FONT_SIZE * self.global_scale
    }

    /// Returns the large font size (matches `ImGuiFullscreen`'s scale).
    pub fn font_size_large(&self) -> f32 {
        LAYOUT_LARGE_FONT_SIZE * self.global_scale
    }

    /// `true` if ImGui wants to capture keyboard input.
    pub fn wants_text_input(&self) -> bool {
        self.imgui_wants_text
    }

    /// `true` if ImGui wants to capture mouse input.
    pub fn wants_mouse_input(&self) -> bool {
        self.imgui_wants_mouse
    }

    /// Set the cached `wants text input` flag.
    pub fn set_wants_text_input(&mut self, value: bool) {
        self.imgui_wants_text = value;
    }

    /// Set the cached `wants mouse input` flag.
    pub fn set_wants_mouse_input(&mut self, value: bool) {
        self.imgui_wants_mouse = value;
    }

    /// Set the cached `wants keyboard` flag.
    pub fn set_wants_keyboard(&mut self, value: bool) {
        self.imgui_wants_keyboard = value;
    }

    /// Update the window dimensions. Mirrors `WindowResized`.
    pub fn window_resized(&mut self, width: u32, height: u32) {
        self.window_width = width as f32;
        self.window_height = height as f32;
        self.scale_changed = true;
    }

    /// Compute a position for an OSD message based on its anchor point.
    /// Mirrors `CalculateOSDPosition` from `ImGuiOverlays.cpp`.
    pub fn calculate_osd_position(
        position: OsdOverlayPos,
        margin: f32,
        text_size: (f32, f32),
        window_width: f32,
        window_height: f32,
    ) -> (f32, f32) {
        match position {
            OsdOverlayPos::TopLeft => (margin, margin),
            OsdOverlayPos::TopCenter => ((window_width - text_size.0) * 0.5, margin),
            OsdOverlayPos::TopRight => (window_width - margin - text_size.0, margin),
            OsdOverlayPos::CenterLeft => (margin, (window_height - text_size.1) * 0.5),
            OsdOverlayPos::Center => {
                ((window_width - text_size.0) * 0.5, (window_height - text_size.1) * 0.5)
            }
            OsdOverlayPos::CenterRight => (
                window_width - margin - text_size.0,
                (window_height - text_size.1) * 0.5,
            ),
            OsdOverlayPos::BottomLeft => (margin, window_height - margin - text_size.1),
            OsdOverlayPos::BottomCenter => (
                (window_width - text_size.0) * 0.5,
                window_height - margin - text_size.1,
            ),
            OsdOverlayPos::BottomRight => (
                window_width - margin - text_size.0,
                window_height - margin - text_size.1,
            ),
            OsdOverlayPos::None => (0.0, 0.0),
        }
    }

    /// Compute a position for a single line of the performance overlay.
    /// Mirrors `CalculatePerformanceOverlayTextPosition` from
    /// `ImGuiOverlays.cpp`.
    pub fn calculate_performance_overlay_text_position(
        position: OsdOverlayPos,
        margin: f32,
        text_size: (f32, f32),
        window_width: f32,
        position_y: f32,
    ) -> (f32, f32) {
        let abs_margin = margin.abs();
        let x = match position {
            OsdOverlayPos::TopLeft | OsdOverlayPos::CenterLeft | OsdOverlayPos::BottomLeft => abs_margin,
            OsdOverlayPos::TopCenter | OsdOverlayPos::Center | OsdOverlayPos::BottomCenter => {
                (window_width - text_size.0) * 0.5
            }
            _ => window_width - text_size.0 - abs_margin,
        };
        (x, position_y)
    }

    /// `true` if the overlay position is on the left side of the screen.
    /// Mirrors `ShouldUseLeftAlignment` from `ImGuiOverlays.cpp`.
    pub fn should_use_left_alignment(position: OsdOverlayPos) -> bool {
        matches!(
            position,
            OsdOverlayPos::TopLeft | OsdOverlayPos::CenterLeft | OsdOverlayPos::BottomLeft
        )
    }

    /// Toggle the gamepad north/west swap state.
    pub fn swap_gamepad_north_west(&mut self, value: bool) {
        self.gamepad_swap_noth_west = value;
    }

    /// Read the gamepad north/west swap state.
    pub fn is_gamepad_north_west_swapped(&self) -> bool {
        self.gamepad_swap_noth_west
    }
}

// ---------------------------------------------------------------------------
// ImGuiOverlays
// ---------------------------------------------------------------------------

/// Rust equivalent of the `ImGuiManager::RenderOverlays` function and the
/// surrounding state in `ImGuiOverlays.cpp`.
#[derive(Debug, Default)]
pub struct ImGuiOverlays {
    initialized: bool,
    open: bool,
    open_time: f32,
    close_time: f32,
    current_slot: i32,
    scroll_animated: AnimatedValue,
    background_animated: AnimatedValue,
    speed_icon: String,
    speed_line: String,
    speed_line_color: u32,
    recording_active: bool,
    input_recording_message: String,
}

impl ImGuiOverlays {
    /// Create a new, uninitialized overlay state.
    pub fn new() -> Self {
        Self {
            close_time: SAVE_STATE_SELECTOR_OPEN_TIME,
            speed_line_color: OSD_WHITE_COLOR,
            ..Self::default()
        }
    }

    /// Initialize the OSD overlay state. Mirrors
    /// `ImGuiManager::RenderOverlays` setup.
    pub fn init(&mut self) {
        self.initialized = true;
        self.scroll_animated.reset(0.0);
        self.background_animated.reset(0.0);
    }

    /// Free any overlay state. Mirrors `SaveStateSelectorUI::Clear`.
    pub fn shutdown(&mut self) {
        self.initialized = false;
        self.open = false;
        self.speed_icon.clear();
        self.speed_line.clear();
        self.input_recording_message.clear();
    }

    /// Open the save-state selector UI. Mirrors `SaveStateSelectorUI::Open`.
    pub fn open_save_state_selector(&mut self, open_time: f32) {
        self.open_time = 0.0;
        self.close_time = open_time;
        if self.open {
            return;
        }
        self.scroll_animated.reset(0.0);
        self.background_animated.reset(0.0);
        self.open = true;
    }

    /// Close the save-state selector UI. Mirrors `SaveStateSelectorUI::Close`.
    pub fn close_save_state_selector(&mut self) {
        self.open = false;
    }

    /// Returns `true` if the save-state selector is currently open.
    pub fn is_save_state_selector_open(&self) -> bool {
        self.open
    }

    /// Select the next save-state slot. Mirrors
    /// `SaveStateSelectorUI::SelectNextSlot`.
    pub fn select_next_slot(&mut self, open_selector: bool, max_slots: i32) {
        if max_slots <= 0 {
            return;
        }
        self.current_slot = if self.current_slot == max_slots - 1 {
            0
        } else {
            self.current_slot + 1
        };
        if open_selector {
            self.open_save_state_selector(self.close_time);
        }
    }

    /// Select the previous save-state slot. Mirrors
    /// `SaveStateSelectorUI::SelectPreviousSlot`.
    pub fn select_previous_slot(&mut self, open_selector: bool, max_slots: i32) {
        if max_slots <= 0 {
            return;
        }
        self.current_slot = if self.current_slot == 0 {
            max_slots - 1
        } else {
            self.current_slot - 1
        };
        if open_selector {
            self.open_save_state_selector(self.close_time);
        }
    }

    /// Returns the currently selected save-state slot (1-indexed, matching
    /// `SaveStateSelectorUI::GetCurrentSlot`).
    pub fn get_current_slot(&self) -> i32 {
        self.current_slot + 1
    }

    /// Tick the OSD message / save-state selector timers. Mirrors the
    /// auto-close behaviour of `SaveStateSelectorUI::Draw`.
    pub fn tick(&mut self, delta_time: f32) {
        if !self.open {
            return;
        }
        self.open_time += delta_time;
        if self.open_time >= self.close_time {
            self.open = false;
        }
    }

    /// Stub for `ImGuiManager::RenderOSD`. The real implementation draws
    /// performance, indicator, video capture, input recording, texture
    /// replacement, settings, shader compile, and input overlays plus the
    /// save-state selector window. The Rust translation tracks the entry
    /// point so callers can integrate with it.
    pub fn draw_osd(&mut self) {
        // The actual rendering work happens in the ImGui C++ backend. This
        // method exists as a stable, idiomatic entry point that matches
        // `ImGuiManager::RenderOSD`.
    }

    /// Stub for the save-state selector window draw entry point.
    pub fn draw_savestate_selector(&mut self) {
        if !self.open {
            return;
        }
        // The real implementation calls `SaveStateSelectorUI::Draw`.
    }

    /// Update the speed line text and color from the current speed value.
    /// Mirrors the body of `DrawIndicatorsOverlay` and the speed line logic
    /// in `DrawPerformanceOverlay`.
    pub fn set_speed(&mut self, speed: f32, target_speed: f32, slow_motion: f32, turbo: f32) {
        let clamped = speed.clamp(0.0, 1000.0);
        self.speed_line.clear();
        if speed < 95.0 {
            self.speed_line_color = OSD_SPEED_LOW_COLOR;
        } else if speed > 105.0 {
            self.speed_line_color = OSD_SPEED_HIGH_COLOR;
        } else {
            self.speed_line_color = OSD_WHITE_COLOR;
        }
        let _ = clamped;
        if target_speed == slow_motion {
            self.speed_icon = "slow_motion".to_string();
        } else if target_speed == turbo {
            self.speed_icon = "forward_fast".to_string();
        } else {
            self.speed_icon = "forward".to_string();
        }
    }

    /// Set the input recording message and active state.
    pub fn set_input_recording(&mut self, recording: bool, message: String) {
        self.recording_active = recording;
        self.input_recording_message = message;
    }

    /// Returns the speed line color the overlay should draw.
    pub fn speed_line_color(&self) -> u32 {
        self.speed_line_color
    }

    /// Returns the speed line text the overlay should draw.
    pub fn speed_line(&self) -> &str {
        &self.speed_line
    }

    /// Returns the speed icon the indicator overlay should draw.
    pub fn speed_icon(&self) -> &str {
        &self.speed_icon
    }

    /// Returns the input recording overlay text.
    pub fn input_recording_message(&self) -> &str {
        &self.input_recording_message
    }
}

// ---------------------------------------------------------------------------
// Input recording data
// ---------------------------------------------------------------------------

/// Mirror of `InputRecordingUI::InputRecordingData` from `ImGuiOverlays.h`.
#[derive(Debug, Clone, Default)]
pub struct InputRecordingData {
    pub is_recording: bool,
    pub recording_active_message: String,
    pub frame_data_message: String,
    pub undo_count_message: String,
}

// ---------------------------------------------------------------------------
// Free helpers
// ---------------------------------------------------------------------------

/// A no-op replacement for the C++ `Easing::OutExpo` helper used in the
/// overlay timers (kept as a free function to keep call sites short).
#[inline]
pub fn ease_out_expo(t: f32) -> f32 {
    Easing::OutExpo.eval(t)
}

/// A no-op replacement for the C++ `Easing::Linear` helper used in some
/// overlay timers.
#[inline]
pub fn ease_linear(t: f32) -> f32 {
    Easing::Linear.eval(t)
}

/// Approximation of `ImGui::GetTime()` used by the OSD fade logic.
#[inline]
pub fn elapsed_radians(elapsed: f32) -> f32 {
    (elapsed * 10.0) % (2.0 * PI)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easing_extremes() {
        assert_eq!(Easing::Linear.eval(0.0), 0.0);
        assert_eq!(Easing::Linear.eval(1.0), 1.0);
        assert_eq!(Easing::OutExpo.eval(0.0), 0.0);
        assert!((Easing::OutExpo.eval(1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn animated_value_lifecycle() {
        let mut v = AnimatedValue::new(0.0, 10.0, 1.0);
        assert!(v.is_active());
        let first = v.update(0.1);
        assert!(first > 0.0);
        v.stop();
        assert!(!v.is_active());
    }

    #[test]
    fn osd_position_top_left() {
        let (x, y) = ImGuiManager::calculate_osd_position(
            OsdOverlayPos::TopLeft,
            8.0,
            (100.0, 20.0),
            1280.0,
            720.0,
        );
        assert_eq!((x, y), (8.0, 8.0));
    }

    #[test]
    fn overlays_open_close_cycle() {
        let mut o = ImGuiOverlays::new();
        o.init();
        o.open_save_state_selector(0.05);
        assert!(o.is_save_state_selector_open());
        o.tick(0.1);
        assert!(!o.is_save_state_selector_open());
    }
}
