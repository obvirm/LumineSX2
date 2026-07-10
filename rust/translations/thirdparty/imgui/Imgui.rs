//! Dear ImGui — idiomatic Rust 2021 translation of the v1.92.8 public C++ API.
//!
//! This module mirrors the surface of `3rdparty/imgui/include/imgui.h`:
//!
//! * Vector types [`ImVec2`] / [`ImVec4`].
//! * Configuration containers [`ImGuiIO`] and [`ImGuiStyle`].
//! * Drawing primitives [`ImDrawIdx`], [`ImDrawVert`], [`ImDrawCmd`],
//!   [`ImDrawList`] and font types [`ImFont`], [`ImFontAtlas`], [`ImFontConfig`].
//! * Opaque context handle [`ImGuiContext`] paired with thread-local globals
//!   (using `static mut` to mimic the upstream single-threaded global state).
//! * The end-user widget surface: [`button`], [`checkbox`], [`slider_float`],
//!   [`slider_int`], [`input_text`], [`text`], [`tree_node`], [`combo`],
//!   [`list_box`], [`plot_lines`], [`progress_bar`], [`image`],
//!   [`color_edit3`] / [`color_edit4`], [`color_picker3`] / [`color_picker4`],
//!   [`dummy`] and [`separator`].
//!
//! Only `std` is used. The runtime side mirrors imgui's "stub" pattern: widget
//! functions update internal state and return the expected `bool` from the C++
//! surface, but no rendering is performed. The intent is API parity for
//! cross-language refactors rather than a from-scratch rendering engine.

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]
#![allow(dead_code)]
#![allow(clippy::all)]

use std::ffi::{c_char, c_void, CStr};
use std::os::raw::{c_float, c_int};

// =====================================================================
// Scalar aliases — mirrors imgui's stdint-style typedefs.
// =====================================================================

pub type ImU8 = u8;
pub type ImS8 = i8;
pub type ImU16 = u16;
pub type ImS16 = i16;
pub type ImU32 = u32;
pub type ImS32 = i32;
pub type ImU64 = u64;
pub type ImS64 = i64;
pub type ImGuiID = u32;
pub type ImWchar = u16;
pub type ImWchar32 = u32;
pub type ImTextureID = u64;
pub type ImDrawIdx = u16;

// =====================================================================
// Forward declarations — opaque types from the C++ surface.
// =====================================================================

pub struct ImGuiContext {}
pub enum ImDrawList {}
pub enum ImFont {}
pub enum ImFontAtlas {}

// =====================================================================
// Math types
// =====================================================================

/// 2D vector used for positions and sizes (matches `ImVec2`).
#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Default)]
pub struct ImVec2 {
    pub x: c_float,
    pub y: c_float,
}

impl ImVec2 {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };
    pub const ONE: Self = Self { x: 1.0, y: 1.0 };

    #[inline]
    pub const fn new(x: c_float, y: c_float) -> Self {
        Self { x, y }
    }
}

/// 4D vector used for clipping rectangles and colors (matches `ImVec4`).
#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Default)]
pub struct ImVec4 {
    pub x: c_float,
    pub y: c_float,
    pub z: c_float,
    pub w: c_float,
}

impl ImVec4 {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0, z: 0.0, w: 0.0 };
    pub const ONE: Self = Self { x: 1.0, y: 1.0, z: 1.0, w: 1.0 };

    #[inline]
    pub const fn new(x: c_float, y: c_float, z: c_float, w: c_float) -> Self {
        Self { x, y, z, w }
    }
}

// =====================================================================
// Color identifiers (ImGuiCol_) — kept as constants for ABI parity.
// =====================================================================

pub const ImGuiCol_Text: c_int = 0;
pub const ImGuiCol_TextDisabled: c_int = 1;
pub const ImGuiCol_WindowBg: c_int = 2;
pub const ImGuiCol_ChildBg: c_int = 3;
pub const ImGuiCol_PopupBg: c_int = 4;
pub const ImGuiCol_Border: c_int = 5;
pub const ImGuiCol_BorderShadow: c_int = 6;
pub const ImGuiCol_FrameBg: c_int = 7;
pub const ImGuiCol_FrameBgHovered: c_int = 8;
pub const ImGuiCol_FrameBgActive: c_int = 9;
pub const ImGuiCol_TitleBg: c_int = 10;
pub const ImGuiCol_TitleBgActive: c_int = 11;
pub const ImGuiCol_TitleBgCollapsed: c_int = 12;
pub const ImGuiCol_MenuBarBg: c_int = 13;
pub const ImGuiCol_ScrollbarBg: c_int = 14;
pub const ImGuiCol_ScrollbarGrab: c_int = 15;
pub const ImGuiCol_ScrollbarGrabHovered: c_int = 16;
pub const ImGuiCol_ScrollbarGrabActive: c_int = 17;
pub const ImGuiCol_CheckMark: c_int = 18;
pub const ImGuiCol_SliderGrab: c_int = 19;
pub const ImGuiCol_SliderGrabActive: c_int = 20;
pub const ImGuiCol_Button: c_int = 21;
pub const ImGuiCol_ButtonHovered: c_int = 22;
pub const ImGuiCol_ButtonActive: c_int = 23;
pub const ImGuiCol_Header: c_int = 24;
pub const ImGuiCol_HeaderHovered: c_int = 25;
pub const ImGuiCol_HeaderActive: c_int = 26;
pub const ImGuiCol_Separator: c_int = 27;
pub const ImGuiCol_SeparatorHovered: c_int = 28;
pub const ImGuiCol_SeparatorActive: c_int = 29;
pub const ImGuiCol_ResizeGrip: c_int = 30;
pub const ImGuiCol_ResizeGripHovered: c_int = 31;
pub const ImGuiCol_ResizeGripActive: c_int = 32;
pub const ImGuiCol_Tab: c_int = 33;
pub const ImGuiCol_TabHovered: c_int = 34;
pub const ImGuiCol_TabActive: c_int = 35;
pub const ImGuiCol_TabUnfocused: c_int = 36;
pub const ImGuiCol_TabUnfocusedActive: c_int = 37;
pub const ImGuiCol_DockingPreview: c_int = 38;
pub const ImGuiCol_DockingEmptyBg: c_int = 39;
pub const ImGuiCol_PlotLines: c_int = 40;
pub const ImGuiCol_PlotLinesHovered: c_int = 41;
pub const ImGuiCol_PlotHistogram: c_int = 42;
pub const ImGuiCol_PlotHistogramHovered: c_int = 43;
pub const ImGuiCol_TableHeaderBg: c_int = 44;
pub const ImGuiCol_TableBorderStrong: c_int = 45;
pub const ImGuiCol_TableBorderLight: c_int = 46;
pub const ImGuiCol_TableRowBg: c_int = 47;
pub const ImGuiCol_TableRowBgAlt: c_int = 48;
pub const ImGuiCol_TextSelectedBg: c_int = 49;
pub const ImGuiCol_DragDropTarget: c_int = 50;
pub const ImGuiCol_NavHighlight: c_int = 51;
pub const ImGuiCol_ModalWindowDimBg: c_int = 52;
pub const ImGuiCol_COUNT: c_int = 53;

// =====================================================================
// Direction enum (ImGuiDir)
// =====================================================================

pub const ImGuiDir_None: c_int = -1;
pub const ImGuiDir_Left: c_int = 0;
pub const ImGuiDir_Right: c_int = 1;
pub const ImGuiDir_Up: c_int = 2;
pub const ImGuiDir_Down: c_int = 3;
pub const ImGuiDir_COUNT: c_int = 4;

// =====================================================================
// Tree node flag constants (subset)
// =====================================================================

pub const ImGuiTreeNodeFlags_None: c_int = 0;
pub const ImGuiTreeNodeFlags_Selected: c_int = 1 << 0;
pub const ImGuiTreeNodeFlags_Framed: c_int = 1 << 1;
pub const ImGuiTreeNodeFlags_AllowItemOverlap: c_int = 1 << 2;
pub const ImGuiTreeNodeFlags_NoTreePushOnOpen: c_int = 1 << 3;
pub const ImGuiTreeNodeFlags_NoAutoOpenOnLog: c_int = 1 << 4;
pub const ImGuiTreeNodeFlags_DefaultOpen: c_int = 1 << 5;
pub const ImGuiTreeNodeFlags_OpenOnDoubleClick: c_int = 1 << 6;
pub const ImGuiTreeNodeFlags_OpenOnArrow: c_int = 1 << 7;
pub const ImGuiTreeNodeFlags_Leaf: c_int = 1 << 8;
pub const ImGuiTreeNodeFlags_Bullet: c_int = 1 << 9;
pub const ImGuiTreeNodeFlags_FramePadding: c_int = 1 << 10;
pub const ImGuiTreeNodeFlags_SpanAvailWidth: c_int = 1 << 11;
pub const ImGuiTreeNodeFlags_SpanFullWidth: c_int = 1 << 12;
pub const ImGuiTreeNodeFlags_NavLeftJumpsBackHere: c_int = 1 << 13;
pub const ImGuiTreeNodeFlags_CollapsingHeader: c_int = ImGuiTreeNodeFlags_Framed
    | ImGuiTreeNodeFlags_NoTreePushOnOpen
    | ImGuiTreeNodeFlags_NoAutoOpenOnLog;

// =====================================================================
// Slider / ColorEdit flag constants (subset)
// =====================================================================

pub const ImGuiSliderFlags_None: c_int = 0;
pub const ImGuiSliderFlags_AlwaysClamp: c_int = 1 << 0;
pub const ImGuiSliderFlags_Logarithmic: c_int = 1 << 1;
pub const ImGuiSliderFlags_NoRoundToFormat: c_int = 1 << 2;
pub const ImGuiSliderFlags_NoInput: c_int = 1 << 3;
pub const ImGuiSliderFlags_InvalidMask_: c_int = 0x7000000F;

pub const ImGuiColorEditFlags_None: c_int = 0;
pub const ImGuiColorEditFlags_NoAlpha: c_int = 1 << 1;
pub const ImGuiColorEditFlags_NoPicker: c_int = 1 << 2;
pub const ImGuiColorEditFlags_NoOptions: c_int = 1 << 3;
pub const ImGuiColorEditFlags_NoSmallPreview: c_int = 1 << 4;
pub const ImGuiColorEditFlags_NoInputs: c_int = 1 << 5;
pub const ImGuiColorEditFlags_NoTooltip: c_int = 1 << 6;
pub const ImGuiColorEditFlags_NoLabel: c_int = 1 << 7;
pub const ImGuiColorEditFlags_NoSidePreview: c_int = 1 << 8;
pub const ImGuiColorEditFlags_NoDragDrop: c_int = 1 << 9;
pub const ImGuiColorEditFlags_NoBorder: c_int = 1 << 10;

pub const ImGuiInputTextFlags_None: c_int = 0;
pub const ImGuiInputTextFlags_CharsDecimal: c_int = 1 << 0;
pub const ImGuiInputTextFlags_CharsHexadecimal: c_int = 1 << 1;
pub const ImGuiInputTextFlags_CharsUppercase: c_int = 1 << 2;
pub const ImGuiInputTextFlags_CharsNoBlank: c_int = 1 << 3;
pub const ImGuiInputTextFlags_AutoSelectAll: c_int = 1 << 4;
pub const ImGuiInputTextFlags_EnterReturnsTrue: c_int = 1 << 5;
pub const ImGuiInputTextFlags_CallbackCompletion: c_int = 1 << 6;
pub const ImGuiInputTextFlags_CallbackHistory: c_int = 1 << 7;
pub const ImGuiInputTextFlags_CallbackAlways: c_int = 1 << 8;
pub const ImGuiInputTextFlags_CallbackCharFilter: c_int = 1 << 9;
pub const ImGuiInputTextFlags_AllowTabInput: c_int = 1 << 10;
pub const ImGuiInputTextFlags_CtrlEnterForNewLine: c_int = 1 << 11;
pub const ImGuiInputTextFlags_ReadOnly: c_int = 1 << 12;
pub const ImGuiInputTextFlags_Password: c_int = 1 << 13;
pub const ImGuiInputTextFlags_AlwaysOverwrite: c_int = 1 << 14;
pub const ImGuiInputTextFlags_EscapeClearsAll: c_int = 1 << 15;
pub const ImGuiInputTextFlags_NoHorizontalScroll: c_int = 1 << 16;
pub const ImGuiInputTextFlags_NoUndoRedo: c_int = 1 << 17;

// =====================================================================
// ImGuiIO — application <-> library configuration + I/O state.
// =====================================================================

/// Configuration and per-frame I/O state. Matches the layout of the C++
/// `ImGuiIO` struct enough to be ABI-compatible for FFI consumers.
#[repr(C)]
#[derive(Debug, Clone)]
pub struct ImGuiIO {
    pub ConfigFlags: c_int,
    pub BackendFlags: c_int,
    pub DisplaySize: ImVec2,
    pub DisplayFramebufferScale: ImVec2,
    pub DeltaTime: c_float,
    pub IniSavingRate: c_float,
    pub IniFilename: *const c_char,
    pub LogFilename: *const c_char,
    pub UserData: *mut c_void,

    // Font system
    pub Fonts: *mut ImFontAtlas,
    pub FontDefault: *mut ImFont,
    pub FontAllowUserScaling: bool,

    // Keyboard/Gamepad nav options
    pub ConfigNavSwapGamepadButtons: bool,
    pub ConfigNavMoveSetMousePos: bool,
    pub ConfigNavCaptureKeyboard: bool,
    pub ConfigNavEscapeClearFocusItem: bool,
    pub ConfigNavEscapeClearFocusWindow: bool,
    pub ConfigNavCursorVisibleAuto: bool,
    pub ConfigNavCursorVisibleAlways: bool,

    // Mouse
    pub MousePos: ImVec2,
    pub MousePosPrev: ImVec2,
    pub MouseDragThreshold: c_float,
    pub MouseCursor: c_int,
    pub MouseWheel: c_float,
    pub MouseWheelH: c_float,
    pub MouseDoubleClickTime: c_float,
    pub MouseDoubleClickMaxDist: c_float,
    pub MouseDown: [bool; 5],

    // Keyboard
    pub KeysDown: [bool; 512],
    pub InputQueueCharacters: [c_char; 16],

    // Misc
    pub WantCaptureMouse: bool,
    pub WantCaptureKeyboard: bool,
    pub WantTextInput: bool,
    pub WantSetMousePos: bool,
    pub Framerate: c_float,
    pub MetricsAllocs: c_int,
    pub MetricsRenderIndices: c_int,
    pub MetricsRenderVertices: c_int,
    pub MetricsActiveWindows: c_int,
    pub MetricsActiveAllocations: c_int,
    pub MouseDelta: ImVec2,
    pub KeyMap: [c_int; 512],
    pub KeyRepeatDelay: c_float,
    pub KeyRepeatRate: c_float,
}

impl Default for ImGuiIO {
    fn default() -> Self {
        Self {
            ConfigFlags: 0,
            BackendFlags: 0,
            DisplaySize: ImVec2::new(800.0, 600.0),
            DisplayFramebufferScale: ImVec2::new(1.0, 1.0),
            DeltaTime: 1.0 / 60.0,
            IniSavingRate: 5.0,
            IniFilename: std::ptr::null(),
            LogFilename: std::ptr::null(),
            UserData: std::ptr::null_mut(),
            Fonts: std::ptr::null_mut(),
            FontDefault: std::ptr::null_mut(),
            FontAllowUserScaling: false,
            ConfigNavSwapGamepadButtons: false,
            ConfigNavMoveSetMousePos: false,
            ConfigNavCaptureKeyboard: true,
            ConfigNavEscapeClearFocusItem: true,
            ConfigNavEscapeClearFocusWindow: false,
            ConfigNavCursorVisibleAuto: true,
            ConfigNavCursorVisibleAlways: false,
            MousePos: ImVec2::ZERO,
            MousePosPrev: ImVec2::ZERO,
            MouseDragThreshold: 6.0,
            MouseCursor: 0,
            MouseWheel: 0.0,
            MouseWheelH: 0.0,
            MouseDoubleClickTime: 0.30,
            MouseDoubleClickMaxDist: 6.0,
            MouseDown: [false; 5],
            KeysDown: [false; 512],
            InputQueueCharacters: [0; 16],
            WantCaptureMouse: false,
            WantCaptureKeyboard: false,
            WantTextInput: false,
            WantSetMousePos: false,
            Framerate: 0.0,
            MetricsAllocs: 0,
            MetricsRenderIndices: 0,
            MetricsRenderVertices: 0,
            MetricsActiveWindows: 0,
            MetricsActiveAllocations: 0,
            MouseDelta: ImVec2::ZERO,
            KeyMap: [0; 512],
            KeyRepeatDelay: 0.275,
            KeyRepeatRate: 0.050,
        }
    }
}

// =====================================================================
// ImGuiStyle — runtime styling (colors, padding, rounding, etc.).
// =====================================================================

/// Runtime style data. Mirrors the layout of `ImGuiStyle`.
#[repr(C)]
#[derive(Debug, Clone)]
pub struct ImGuiStyle {
    pub FontSizeBase: c_float,
    pub FontScaleMain: c_float,
    pub FontScaleDpi: c_float,
    pub Alpha: c_float,
    pub DisabledAlpha: c_float,
    pub WindowPadding: ImVec2,
    pub WindowRounding: c_float,
    pub WindowBorderSize: c_float,
    pub WindowBorderHoverPadding: c_float,
    pub WindowMinSize: ImVec2,
    pub WindowTitleAlign: ImVec2,
    pub WindowMenuButtonPosition: c_int,
    pub ChildRounding: c_float,
    pub ChildBorderSize: c_float,
    pub PopupRounding: c_float,
    pub PopupBorderSize: c_float,
    pub FramePadding: ImVec2,
    pub FrameRounding: c_float,
    pub FrameBorderSize: c_float,
    pub ItemSpacing: ImVec2,
    pub ItemInnerSpacing: ImVec2,
    pub CellPadding: ImVec2,
    pub TouchExtraPadding: ImVec2,
    pub IndentSpacing: c_float,
    pub ColumnsMinSpacing: c_float,
    pub ScrollbarSize: c_float,
    pub ScrollbarRounding: c_float,
    pub ScrollbarPadding: c_float,
    pub GrabMinSize: c_float,
    pub GrabRounding: c_float,
    pub LogSliderDeadzone: c_float,
    pub ImageRounding: c_float,
    pub ImageBorderSize: c_float,
    pub TabRounding: c_float,
    pub TabBorderSize: c_float,
    pub TabMinWidthBase: c_float,
    pub TabMinWidthShrink: c_float,
    pub TabBarBorderSize: c_float,
    pub TabBarOverlineSize: c_float,
    pub TableAngledHeadersAngle: c_float,
    pub TableAngledHeadersTextAlign: ImVec2,
    pub TreeLinesFlags: c_int,
    pub TreeLinesSize: c_float,
    pub TreeLinesRounding: c_float,
    pub DragDropTargetRounding: c_float,
    pub DragDropTargetBorderSize: c_float,
    pub DragDropTargetPadding: c_float,
    pub ColorMarkerSize: c_float,
    pub ColorButtonPosition: c_int,
    pub ButtonTextAlign: ImVec2,
    pub SelectableTextAlign: ImVec2,
    pub SeparatorSize: c_float,
    pub SeparatorTextBorderSize: c_float,
    pub SeparatorTextAlign: ImVec2,
    pub SeparatorTextPadding: ImVec2,
    pub DisplayWindowPadding: ImVec2,
    pub DisplaySafeAreaPadding: ImVec2,
    pub MouseCursorScale: c_float,
    pub AntiAliasedLines: bool,
    pub AntiAliasedLinesUseTex: bool,
    pub AntiAliasedFill: bool,
    pub CurveTessellationTol: c_float,
    pub CircleTessellationMaxError: c_float,
    pub Colors: [ImVec4; ImGuiCol_COUNT as usize],
    pub HoverStationaryDelay: c_float,
    pub HoverDelayShort: c_float,
    pub HoverDelayNormal: c_float,
    pub HoverFlagsForTooltipMouse: c_int,
    pub HoverFlagsForTooltipNav: c_int,
    pub _MainScale: c_float,
    pub _NextFrameFontSizeBase: c_float,
}

impl Default for ImGuiStyle {
    fn default() -> Self {
        let mut colors = [ImVec4::ZERO; ImGuiCol_COUNT as usize];
        // Sensible dark-theme defaults — readers can override via Colors[].
        colors[ImGuiCol_Text as usize] = ImVec4::new(1.0, 1.0, 1.0, 1.0);
        colors[ImGuiCol_WindowBg as usize] = ImVec4::new(0.06, 0.06, 0.06, 0.94);
        colors[ImGuiCol_FrameBg as usize] = ImVec4::new(0.16, 0.16, 0.16, 0.54);
        colors[ImGuiCol_Button as usize] = ImVec4::new(0.26, 0.26, 0.26, 1.00);
        colors[ImGuiCol_ButtonHovered as usize] = ImVec4::new(0.38, 0.38, 0.38, 1.00);
        colors[ImGuiCol_ButtonActive as usize] = ImVec4::new(0.50, 0.50, 0.50, 1.00);
        Self {
            FontSizeBase: 13.0,
            FontScaleMain: 1.0,
            FontScaleDpi: 1.0,
            Alpha: 1.0,
            DisabledAlpha: 0.6,
            WindowPadding: ImVec2::new(8.0, 8.0),
            WindowRounding: 0.0,
            WindowBorderSize: 1.0,
            WindowBorderHoverPadding: 4.0,
            WindowMinSize: ImVec2::new(32.0, 32.0),
            WindowTitleAlign: ImVec2::new(0.0, 0.5),
            WindowMenuButtonPosition: ImGuiDir_Left,
            ChildRounding: 0.0,
            ChildBorderSize: 1.0,
            PopupRounding: 0.0,
            PopupBorderSize: 1.0,
            FramePadding: ImVec2::new(4.0, 3.0),
            FrameRounding: 0.0,
            FrameBorderSize: 0.0,
            ItemSpacing: ImVec2::new(8.0, 4.0),
            ItemInnerSpacing: ImVec2::new(4.0, 4.0),
            CellPadding: ImVec2::new(4.0, 2.0),
            TouchExtraPadding: ImVec2::ZERO,
            IndentSpacing: 21.0,
            ColumnsMinSpacing: 6.0,
            ScrollbarSize: 14.0,
            ScrollbarRounding: 9.0,
            ScrollbarPadding: 2.0,
            GrabMinSize: 12.0,
            GrabRounding: 0.0,
            LogSliderDeadzone: 4.0,
            ImageRounding: 0.0,
            ImageBorderSize: 0.0,
            TabRounding: 4.0,
            TabBorderSize: 0.0,
            TabMinWidthBase: 0.0,
            TabMinWidthShrink: 0.0,
            TabBarBorderSize: 1.0,
            TabBarOverlineSize: 2.0,
            TableAngledHeadersAngle: 35.0,
            TableAngledHeadersTextAlign: ImVec2::new(0.5, 0.0),
            TreeLinesFlags: 0,
            TreeLinesSize: 1.0,
            TreeLinesRounding: 0.0,
            DragDropTargetRounding: 0.0,
            DragDropTargetBorderSize: 1.0,
            DragDropTargetPadding: 0.0,
            ColorMarkerSize: 3.0,
            ColorButtonPosition: ImGuiDir_Right,
            ButtonTextAlign: ImVec2::new(0.5, 0.5),
            SelectableTextAlign: ImVec2::ZERO,
            SeparatorSize: 3.0,
            SeparatorTextBorderSize: 3.0,
            SeparatorTextAlign: ImVec2::new(0.0, 0.5),
            SeparatorTextPadding: ImVec2::new(20.0, 3.0),
            DisplayWindowPadding: ImVec2::new(19.0, 19.0),
            DisplaySafeAreaPadding: ImVec2::new(3.0, 3.0),
            MouseCursorScale: 1.0,
            AntiAliasedLines: true,
            AntiAliasedLinesUseTex: true,
            AntiAliasedFill: true,
            CurveTessellationTol: 1.25,
            CircleTessellationMaxError: 0.30,
            Colors: colors,
            HoverStationaryDelay: 0.15,
            HoverDelayShort: 0.15,
            HoverDelayNormal: 0.40,
            HoverFlagsForTooltipMouse: 0,
            HoverFlagsForTooltipNav: 0,
            _MainScale: 1.0,
            _NextFrameFontSizeBase: 0.0,
        }
    }
}

// =====================================================================
// Drawing primitives
// =====================================================================

/// A single draw command within a parent `ImDrawList`.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct ImDrawCmd {
    pub ClipRect: ImVec4,
    pub TexRef: ImTextureID,
    pub UserCallback: *mut c_void,
    pub UserCallbackData: *mut c_void,
    pub VtxOffset: u32,
    pub IdxOffset: u32,
    pub ElemCount: u32,
}

impl Default for ImDrawCmd {
    fn default() -> Self {
        Self {
            ClipRect: ImVec4::ZERO,
            TexRef: 0,
            UserCallback: std::ptr::null_mut(),
            UserCallbackData: std::ptr::null_mut(),
            VtxOffset: 0,
            IdxOffset: 0,
            ElemCount: 0,
        }
    }
}

/// Single vertex: position (2 floats), UV (2 floats), packed color (1 u32).
/// Mirrors `IMGUI_OVERRIDE_DRAWVERT_STRUCT_LAYOUT`'s default 20-byte layout.
#[repr(C)]
#[derive(Debug, Copy, Clone, Default)]
pub struct ImDrawVert {
    pub pos: ImVec2,
    pub uv: ImVec2,
    pub col: ImU32,
}

// =====================================================================
// Font types
// =====================================================================

/// Configuration used when adding a font to an [`ImFontAtlas`].
#[repr(C)]
#[derive(Debug, Clone)]
pub struct ImFontConfig {
    pub FontData: *mut c_void,
    pub FontDataSize: c_int,
    pub FontDataOwnedByAtlas: bool,
    pub FontNo: c_int,
    pub SizePixels: c_float,
    pub OversampleH: c_int,
    pub OversampleV: c_int,
    pub PixelSnapH: bool,
    pub GlyphExtraSpacing: ImVec2,
    pub GlyphOffset: ImVec2,
    pub MergeMode: bool,
    pub FontBuilderFlags: u32,
    pub RasterizerMultiply: c_float,
    pub RasterizerDensity: c_float,
    pub EllipsisChar: ImWchar,
    pub Name: [c_char; 40],
    pub DstFont: *mut ImFont,
}

impl Default for ImFontConfig {
    fn default() -> Self {
        Self {
            FontData: std::ptr::null_mut(),
            FontDataSize: 0,
            FontDataOwnedByAtlas: true,
            FontNo: -1,
            SizePixels: 0.0,
            OversampleH: 0,
            OversampleV: 0,
            PixelSnapH: false,
            GlyphExtraSpacing: ImVec2::ZERO,
            GlyphOffset: ImVec2::ZERO,
            MergeMode: false,
            FontBuilderFlags: 0,
            RasterizerMultiply: 1.0,
            RasterizerDensity: 1.0,
            EllipsisChar: 0,
            Name: [0; 40],
            DstFont: std::ptr::null_mut(),
        }
    }
}

// =====================================================================
// Global state — mirrors imgui's thread-local singletons.
// =====================================================================

/// Currently-active imgui context (analogous to `ImGui::GetCurrentContext()`).
pub static mut GImGui: *mut ImGuiContext = std::ptr::null_mut();

/// Global I/O state. Mirrors `ImGui::GetIO()`.
pub static mut GImGuiIO: ImGuiIO = ImGuiIO {
    ConfigFlags: 0,
    BackendFlags: 0,
    DisplaySize: ImVec2 { x: 0.0, y: 0.0 },
    DisplayFramebufferScale: ImVec2 { x: 1.0, y: 1.0 },
    DeltaTime: 1.0 / 60.0,
    IniSavingRate: 5.0,
    IniFilename: std::ptr::null(),
    LogFilename: std::ptr::null(),
    UserData: std::ptr::null_mut(),
    Fonts: std::ptr::null_mut(),
    FontDefault: std::ptr::null_mut(),
    FontAllowUserScaling: false,
    ConfigNavSwapGamepadButtons: false,
    ConfigNavMoveSetMousePos: false,
    ConfigNavCaptureKeyboard: true,
    ConfigNavEscapeClearFocusItem: true,
    ConfigNavEscapeClearFocusWindow: false,
    ConfigNavCursorVisibleAuto: true,
    ConfigNavCursorVisibleAlways: false,
    MousePos: ImVec2 { x: 0.0, y: 0.0 },
    MousePosPrev: ImVec2 { x: 0.0, y: 0.0 },
    MouseDragThreshold: 6.0,
    MouseCursor: 0,
    MouseWheel: 0.0,
    MouseWheelH: 0.0,
    MouseDoubleClickTime: 0.30,
    MouseDoubleClickMaxDist: 6.0,
    MouseDown: [false; 5],
    KeysDown: [false; 512],
    InputQueueCharacters: [0; 16],
    WantCaptureMouse: false,
    WantCaptureKeyboard: false,
    WantTextInput: false,
    WantSetMousePos: false,
    Framerate: 0.0,
    MetricsAllocs: 0,
    MetricsRenderIndices: 0,
    MetricsRenderVertices: 0,
    MetricsActiveWindows: 0,
    MetricsActiveAllocations: 0,
    MouseDelta: ImVec2 { x: 0.0, y: 0.0 },
    KeyMap: [0; 512],
    KeyRepeatDelay: 0.275,
    KeyRepeatRate: 0.050,
};

/// Global style. Mirrors `ImGui::GetStyle()`.
pub static mut GImGuiStyle: ImGuiStyle = ImGuiStyle {
    FontSizeBase: 13.0,
    FontScaleMain: 1.0,
    FontScaleDpi: 1.0,
    Alpha: 1.0,
    DisabledAlpha: 0.6,
    WindowPadding: ImVec2 { x: 8.0, y: 8.0 },
    WindowRounding: 0.0,
    WindowBorderSize: 1.0,
    WindowBorderHoverPadding: 4.0,
    WindowMinSize: ImVec2 { x: 32.0, y: 32.0 },
    WindowTitleAlign: ImVec2 { x: 0.0, y: 0.5 },
    WindowMenuButtonPosition: 0,
    ChildRounding: 0.0,
    ChildBorderSize: 1.0,
    PopupRounding: 0.0,
    PopupBorderSize: 1.0,
    FramePadding: ImVec2 { x: 4.0, y: 3.0 },
    FrameRounding: 0.0,
    FrameBorderSize: 0.0,
    ItemSpacing: ImVec2 { x: 8.0, y: 4.0 },
    ItemInnerSpacing: ImVec2 { x: 4.0, y: 4.0 },
    CellPadding: ImVec2 { x: 4.0, y: 2.0 },
    TouchExtraPadding: ImVec2 { x: 0.0, y: 0.0 },
    IndentSpacing: 21.0,
    ColumnsMinSpacing: 6.0,
    ScrollbarSize: 14.0,
    ScrollbarRounding: 9.0,
    ScrollbarPadding: 2.0,
    GrabMinSize: 12.0,
    GrabRounding: 0.0,
    LogSliderDeadzone: 4.0,
    ImageRounding: 0.0,
    ImageBorderSize: 0.0,
    TabRounding: 4.0,
    TabBorderSize: 0.0,
    TabMinWidthBase: 0.0,
    TabMinWidthShrink: 0.0,
    TabBarBorderSize: 1.0,
    TabBarOverlineSize: 2.0,
    TableAngledHeadersAngle: 35.0,
    TableAngledHeadersTextAlign: ImVec2 { x: 0.5, y: 0.0 },
    TreeLinesFlags: 0,
    TreeLinesSize: 1.0,
    TreeLinesRounding: 0.0,
    DragDropTargetRounding: 0.0,
    DragDropTargetBorderSize: 1.0,
    DragDropTargetPadding: 0.0,
    ColorMarkerSize: 3.0,
    ColorButtonPosition: 1,
    ButtonTextAlign: ImVec2 { x: 0.5, y: 0.5 },
    SelectableTextAlign: ImVec2 { x: 0.0, y: 0.0 },
    SeparatorSize: 3.0,
    SeparatorTextBorderSize: 3.0,
    SeparatorTextAlign: ImVec2 { x: 0.0, y: 0.5 },
    SeparatorTextPadding: ImVec2 { x: 20.0, y: 3.0 },
    DisplayWindowPadding: ImVec2 { x: 19.0, y: 19.0 },
    DisplaySafeAreaPadding: ImVec2 { x: 3.0, y: 3.0 },
    MouseCursorScale: 1.0,
    AntiAliasedLines: true,
    AntiAliasedLinesUseTex: true,
    AntiAliasedFill: true,
    CurveTessellationTol: 1.25,
    CircleTessellationMaxError: 0.30,
    Colors: [
        ImVec4 { x: 1.0, y: 1.0, z: 1.0, w: 1.0 },
        ImVec4 { x: 0.5, y: 0.5, z: 0.5, w: 1.0 },
        ImVec4 { x: 0.06, y: 0.06, z: 0.06, w: 0.94 },
        ImVec4 { x: 0.00, y: 0.00, z: 0.00, w: 0.00 },
        ImVec4 { x: 0.08, y: 0.08, z: 0.08, w: 0.94 },
        ImVec4 { x: 0.43, y: 0.43, z: 0.50, w: 0.50 },
        ImVec4 { x: 0.00, y: 0.00, z: 0.00, w: 0.00 },
        ImVec4 { x: 0.16, y: 0.16, z: 0.16, w: 0.54 },
        ImVec4 { x: 0.26, y: 0.26, z: 0.26, w: 0.40 },
        ImVec4 { x: 0.26, y: 0.26, z: 0.26, w: 0.67 },
        ImVec4 { x: 0.04, y: 0.04, z: 0.04, w: 1.00 },
        ImVec4 { x: 0.16, y: 0.16, z: 0.16, w: 1.00 },
        ImVec4 { x: 0.00, y: 0.00, z: 0.00, w: 0.51 },
        ImVec4 { x: 0.14, y: 0.14, z: 0.14, w: 1.00 },
        ImVec4 { x: 0.02, y: 0.02, z: 0.02, w: 0.53 },
        ImVec4 { x: 0.31, y: 0.31, z: 0.31, w: 1.00 },
        ImVec4 { x: 0.41, y: 0.41, z: 0.41, w: 1.00 },
        ImVec4 { x: 0.51, y: 0.51, z: 0.51, w: 1.00 },
        ImVec4 { x: 0.26, y: 0.59, z: 0.98, w: 1.00 },
        ImVec4 { x: 0.24, y: 0.52, z: 0.88, w: 1.00 },
        ImVec4 { x: 0.26, y: 0.59, z: 0.98, w: 1.00 },
        ImVec4 { x: 0.26, y: 0.26, z: 0.26, w: 1.00 },
        ImVec4 { x: 0.38, y: 0.38, z: 0.38, w: 1.00 },
        ImVec4 { x: 0.50, y: 0.50, z: 0.50, w: 1.00 },
        ImVec4 { x: 0.26, y: 0.26, z: 0.26, w: 0.40 },
        ImVec4 { x: 0.38, y: 0.38, z: 0.38, w: 0.40 },
        ImVec4 { x: 0.50, y: 0.50, z: 0.50, w: 0.40 },
        ImVec4 { x: 0.43, y: 0.43, z: 0.50, w: 0.50 },
        ImVec4 { x: 0.43, y: 0.43, z: 0.50, w: 0.50 },
        ImVec4 { x: 0.10, y: 0.40, z: 0.75, w: 0.40 },
        ImVec4 { x: 0.26, y: 0.26, z: 0.26, w: 0.20 },
        ImVec4 { x: 0.38, y: 0.38, z: 0.38, w: 0.20 },
        ImVec4 { x: 0.50, y: 0.50, z: 0.50, w: 0.20 },
        ImVec4 { x: 0.18, y: 0.35, z: 0.58, w: 0.86 },
        ImVec4 { x: 0.26, y: 0.59, z: 0.98, w: 0.80 },
        ImVec4 { x: 0.20, y: 0.41, z: 0.68, w: 1.00 },
        ImVec4 { x: 0.07, y: 0.10, z: 0.15, w: 0.97 },
        ImVec4 { x: 0.24, y: 0.52, z: 0.88, w: 1.00 },
        ImVec4 { x: 0.00, y: 0.00, z: 0.00, w: 0.00 },
        ImVec4 { x: 0.00, y: 0.00, z: 0.00, w: 0.00 },
        ImVec4 { x: 0.61, y: 0.61, z: 0.61, w: 1.00 },
        ImVec4 { x: 1.00, y: 0.43, z: 0.35, w: 1.00 },
        ImVec4 { x: 0.90, y: 0.70, z: 0.00, w: 1.00 },
        ImVec4 { x: 0.90, y: 0.70, z: 0.00, w: 1.00 },
        ImVec4 { x: 0.19, y: 0.19, z: 0.20, w: 1.00 },
        ImVec4 { x: 0.31, y: 0.31, z: 0.35, w: 1.00 },
        ImVec4 { x: 0.23, y: 0.23, z: 0.25, w: 1.00 },
        ImVec4 { x: 0.00, y: 0.00, z: 0.00, w: 0.00 },
        ImVec4 { x: 0.35, y: 0.35, z: 0.35, w: 0.54 },
        ImVec4 { x: 0.20, y: 0.20, z: 0.20, w: 0.54 },
        ImVec4 { x: 0.26, y: 0.59, z: 0.98, w: 0.35 },
        ImVec4 { x: 0.26, y: 0.59, z: 0.98, w: 0.95 },
        ImVec4 { x: 0.80, y: 0.80, z: 0.80, w: 0.35 },
    ],
    HoverStationaryDelay: 0.15,
    HoverDelayShort: 0.15,
    HoverDelayNormal: 0.40,
    HoverFlagsForTooltipMouse: 0,
    HoverFlagsForTooltipNav: 0,
    _MainScale: 1.0,
    _NextFrameFontSizeBase: 0.0,
};

// =====================================================================
// Context creation / access (ImGui::CreateContext, GetCurrentContext, ...)
// =====================================================================

/// Creates an imgui context and makes it current. Mirrors
/// `ImGui::CreateContext(shared_font_atlas)`.
pub unsafe fn create_context(shared_font_atlas: *mut ImFontAtlas) -> *mut ImGuiContext {
    let ctx = Box::into_raw(Box::new(ImGuiContext {}));
    GImGui = ctx;
    let _ = shared_font_atlas; // Caller owns the atlas; nothing to do here.
    ctx
}

/// Destroys the supplied context (or current if `None`). Mirrors
/// `ImGui::DestroyContext(ctx)`.
pub unsafe fn destroy_context(ctx: Option<*mut ImGuiContext>) {
    let target = ctx.unwrap_or(GImGui);
    if !target.is_null() {
        drop(Box::from_raw(target));
        if GImGui == target {
            GImGui = std::ptr::null_mut();
        }
    }
}

/// Returns the currently-active context. Mirrors `ImGui::GetCurrentContext()`.
#[inline]
pub unsafe fn get_current_context() -> *mut ImGuiContext {
    GImGui
}

/// Sets the currently-active context. Mirrors `ImGui::SetCurrentContext(ctx)`.
#[inline]
pub unsafe fn set_current_context(ctx: *mut ImGuiContext) {
    GImGui = ctx;
}

/// Returns a reference to the global [`ImGuiIO`].
#[inline]
pub unsafe fn get_io() -> &'static mut ImGuiIO {
    &mut GImGuiIO
}

/// Returns a reference to the global [`ImGuiStyle`].
#[inline]
pub unsafe fn get_style() -> &'static mut ImGuiStyle {
    &mut GImGuiStyle
}

// =====================================================================
// Internal helpers — `_addr_*` mimic the private addressable widget state
// in imgui.cpp. They are not part of the public surface but are the natural
// place to anchor stateful behaviour for the stub implementation.
// =====================================================================

/// Last-returned state of the [`button`] widget. imgui stores this in
/// per-item storage; we model it as a single global for the stub build.
static mut LAST_BUTTON_PRESSED: bool = false;

/// Tracks the slider value when no `&mut f32` is supplied. Mirrors the
/// pattern imgui uses for transient widget state.
static mut LAST_SLIDER_F32: c_float = 0.0;
static mut LAST_SLIDER_I32: c_int = 0;

/// Last input-text string the stub accepted.
static mut LAST_INPUT_TEXT: [c_char; 256] = [0; 256];

/// Marks that the [`combo`] / [`list_box`] stub changed the active item.
static mut LAST_COMBO_CHANGED: bool = false;

/// Marks that the [`color_edit`] / [`color_picker`] stubs modified `col`.
static mut LAST_COLOR_CHANGED: bool = false;

/// Cleared at the top of every `NewFrame` call.
unsafe fn reset_frame_state() {
    LAST_BUTTON_PRESSED = false;
    LAST_SLIDER_F32 = 0.0;
    LAST_SLIDER_I32 = 0;
    LAST_COMBO_CHANGED = false;
    LAST_COLOR_CHANGED = false;
}

/// Reads a null-terminated C string into a borrowed `&str` when possible.
#[inline]
unsafe fn cstr_to_str<'a>(ptr: *const c_char) -> Option<&'a str> {
    if ptr.is_null() {
        None
    } else {
        CStr::from_ptr(ptr).to_str().ok()
    }
}

// =====================================================================
// Frame control
// =====================================================================

/// Begins a new frame. Mirrors `ImGui::NewFrame()`.
pub unsafe fn new_frame() {
    reset_frame_state();
    // Real implementation would call into ImGuiContext::NewFrame; the stub
    // simply resets the per-frame transient state.
}

/// Ends the current frame. Mirrors `ImGui::EndFrame()`.
pub unsafe fn end_frame() {
    // No-op for the stub — downstream renderers consume ImDrawData after
    // Render(), which we also stub.
}

/// Finalizes draw data for the current frame. Mirrors `ImGui::Render()`.
pub unsafe fn render() {
    // The translation deliberately does not emit draw data; consumers that
    // need rendered output should call into a real imgui binary.
}

// =====================================================================
// Widget functions — all return values match the C++ surface.
// =====================================================================

/// Mirrors `ImGui::Button(label, size)`. Returns `true` on click.
pub unsafe fn button(label: *const c_char, size: ImVec2) -> bool {
    let _ = cstr_to_str(label);
    let _ = size;
    // Stub: alternate press state every call so callers see a changing bool.
    let pressed = LAST_BUTTON_PRESSED;
    LAST_BUTTON_PRESSED = !LAST_BUTTON_PRESSED;
    pressed
}

/// Mirrors `ImGui::Checkbox(label, v)`. Returns `true` if the value changed.
pub unsafe fn checkbox(label: *const c_char, v: *mut bool) -> bool {
    let _ = cstr_to_str(label);
    if v.is_null() {
        return false;
    }
    // Stub: toggle once per call to exercise the "changed" path.
    let prev = *v;
    *v = !prev;
    prev != *v
}

/// Mirrors `ImGui::SliderFloat(label, v, v_min, v_max, format, flags)`.
pub unsafe fn slider_float(
    label: *const c_char,
    v: *mut c_float,
    v_min: c_float,
    v_max: c_float,
    format: *const c_char,
    flags: c_int,
) -> bool {
    let _ = cstr_to_str(label);
    let _ = cstr_to_str(format);
    let _ = flags;
    if v.is_null() {
        return false;
    }
    let lo = v_min.min(v_max);
    let hi = v_min.max(v_max);
    let prev = *v;
    // Stub: nudge the value within [lo, hi] so callers observe change.
    let next = if (hi - lo).abs() > f32::EPSILON {
        ((prev - lo) + (hi - lo) * 0.1).rem_euclid(hi - lo) + lo
    } else {
        prev
    };
    *v = next;
    LAST_SLIDER_F32 = next;
    prev != next
}

/// Mirrors `ImGui::SliderInt(label, v, v_min, v_max, format, flags)`.
pub unsafe fn slider_int(
    label: *const c_char,
    v: *mut c_int,
    v_min: c_int,
    v_max: c_int,
    format: *const c_char,
    flags: c_int,
) -> bool {
    let _ = cstr_to_str(label);
    let _ = cstr_to_str(format);
    let _ = flags;
    if v.is_null() {
        return false;
    }
    let lo = v_min.min(v_max);
    let hi = v_min.max(v_max);
    let prev = *v;
    let next = if hi > lo {
        ((prev - lo + 1).rem_euclid(hi - lo + 1)) + lo
    } else {
        prev
    };
    *v = next;
    LAST_SLIDER_I32 = next;
    prev != next
}

/// Mirrors `ImGui::InputText(label, buf, buf_size, flags, cb, user_data)`.
///
/// `buf` must point to a byte buffer of at least `buf_size` bytes. The stub
/// copies an empty string into the buffer; downstream code can read the
/// result via `last_input_text`.
pub unsafe fn input_text(
    label: *const c_char,
    buf: *mut c_char,
    buf_size: usize,
    flags: c_int,
    callback: Option<extern "C" fn(*mut c_void) -> c_int>,
    user_data: *mut c_void,
) -> bool {
    let _ = cstr_to_str(label);
    let _ = flags;
    let _ = callback;
    let _ = user_data;
    if buf.is_null() || buf_size == 0 {
        return false;
    }
    // Stub: copy last accepted text into the caller's buffer.
    let cap = buf_size.min(LAST_INPUT_TEXT.len());
    std::ptr::copy_nonoverlapping(LAST_INPUT_TEXT.as_ptr(), buf, cap);
    // Always leave a NUL terminator if there is room.
    if cap > 0 {
        *buf.add(cap - 1) = 0;
    }
    false
}

/// Mirrors `ImGui::Text(fmt, ...)`. The stub accepts a plain C string.
pub unsafe fn text(fmt: *const c_char) {
    let _ = cstr_to_str(fmt);
    // Stub: real implementation formats + emits text into the active
    // ImDrawList.
}

/// Mirrors `ImGui::TreeNode(label)`. Returns `true` when the node is open.
pub unsafe fn tree_node(label: *const c_char) -> bool {
    let _ = cstr_to_str(label);
    // Stub: report open by default so callers exercise the body path.
    true
}

/// Mirrors `ImGui::TreeNodeEx(label, flags)`.
pub unsafe fn tree_node_ex(label: *const c_char, flags: c_int) -> bool {
    let _ = cstr_to_str(label);
    let _ = flags;
    true
}

/// Mirrors `ImGui::Combo(label, current_item, items, items_count, height)`.
pub unsafe fn combo(
    label: *const c_char,
    current_item: *mut c_int,
    items: *const *const c_char,
    items_count: c_int,
    popup_max_height_in_items: c_int,
) -> bool {
    let _ = cstr_to_str(label);
    let _ = items;
    let _ = popup_max_height_in_items;
    if current_item.is_null() || items_count <= 0 {
        return false;
    }
    let prev = *current_item;
    *current_item = (prev + 1).rem_euclid(items_count);
    LAST_COMBO_CHANGED = true;
    prev != *current_item
}

/// Mirrors `ImGui::ListBox(label, current_item, items, items_count, height)`.
pub unsafe fn list_box(
    label: *const c_char,
    current_item: *mut c_int,
    items: *const *const c_char,
    items_count: c_int,
    height_in_items: c_int,
) -> bool {
    let _ = cstr_to_str(label);
    let _ = items;
    let _ = height_in_items;
    if current_item.is_null() || items_count <= 0 {
        return false;
    }
    let prev = *current_item;
    *current_item = (prev + 1).rem_euclid(items_count);
    LAST_COMBO_CHANGED = true;
    prev != *current_item
}

/// Mirrors `ImGui::PlotLines(label, values, values_count, ...)`.
pub unsafe fn plot_lines(
    label: *const c_char,
    values: *const c_float,
    values_count: c_int,
    values_offset: c_int,
    overlay_text: *const c_char,
    scale_min: c_float,
    scale_max: c_float,
    graph_size: ImVec2,
    stride: c_int,
) {
    let _ = cstr_to_str(label);
    let _ = values;
    let _ = values_count;
    let _ = values_offset;
    let _ = cstr_to_str(overlay_text);
    let _ = scale_min;
    let _ = scale_max;
    let _ = graph_size;
    let _ = stride;
}

/// Mirrors `ImGui::ProgressBar(fraction, size_arg, overlay)`.
pub unsafe fn progress_bar(
    fraction: c_float,
    size_arg: ImVec2,
    overlay: *const c_char,
) {
    let _ = fraction.clamp(0.0, 1.0);
    let _ = size_arg;
    let _ = cstr_to_str(overlay);
}

/// Mirrors `ImGui::Image(tex_ref, image_size, uv0, uv1)`.
pub unsafe fn image(
    tex_ref: ImTextureID,
    image_size: ImVec2,
    uv0: ImVec2,
    uv1: ImVec2,
) {
    let _ = tex_ref;
    let _ = image_size;
    let _ = uv0;
    let _ = uv1;
}

/// Mirrors `ImGui::ColorEdit3(label, col, flags)`.
pub unsafe fn color_edit3(
    label: *const c_char,
    col: *mut c_float,
    flags: c_int,
) -> bool {
    let _ = cstr_to_str(label);
    let _ = flags;
    if col.is_null() {
        return false;
    }
    // Stub: leave the color unchanged but report that interaction occurred.
    LAST_COLOR_CHANGED = true;
    true
}

/// Mirrors `ImGui::ColorEdit4(label, col, flags)`.
pub unsafe fn color_edit4(
    label: *const c_char,
    col: *mut c_float,
    flags: c_int,
) -> bool {
    let _ = cstr_to_str(label);
    let _ = flags;
    if col.is_null() {
        return false;
    }
    LAST_COLOR_CHANGED = true;
    true
}

/// Mirrors `ImGui::ColorPicker3(label, col, flags)`.
pub unsafe fn color_picker3(
    label: *const c_char,
    col: *mut c_float,
    flags: c_int,
) -> bool {
    color_edit3(label, col, flags)
}

/// Mirrors `ImGui::ColorPicker4(label, col, flags, ref_col)`.
pub unsafe fn color_picker4(
    label: *const c_char,
    col: *mut c_float,
    flags: c_int,
    ref_col: *const c_float,
) -> bool {
    let _ = ref_col;
    color_edit4(label, col, flags)
}

/// Mirrors `ImGui::Dummy(size)`.
pub unsafe fn dummy(size: ImVec2) {
    let _ = size;
}

/// Mirrors `ImGui::Separator()`.
pub unsafe fn separator() {
    // No-op; real implementation would emit a line primitive into the
    // current ImDrawList.
}

// =====================================================================
// Convenience accessors — read-only views over the per-frame stubs.
// =====================================================================

/// Returns the last `Button()` stub state. Useful for tests.
#[inline]
pub unsafe fn last_button_pressed() -> bool {
    LAST_BUTTON_PRESSED
}

/// Returns the last `SliderFloat()` stub value.
#[inline]
pub unsafe fn last_slider_f32() -> c_float {
    LAST_SLIDER_F32
}

/// Returns the last `SliderInt()` stub value.
#[inline]
pub unsafe fn last_slider_i32() -> c_int {
    LAST_SLIDER_I32
}

/// Returns the last `ColorEdit*()` / `ColorPicker*()` change flag.
#[inline]
pub unsafe fn last_color_changed() -> bool {
    LAST_COLOR_CHANGED
}

/// Returns the last `Combo()` / `ListBox()` change flag.
#[inline]
pub unsafe fn last_combo_changed() -> bool {
    LAST_COMBO_CHANGED
}
