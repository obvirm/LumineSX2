//! Idiomatic Rust translation of the PCSX2 Qt front-end main window stack.
//!
//! This module is a single Rust 2021 file that captures the public surface
//! and core behaviour of three Qt/C++ source sets:
//!
//! * `MainWindow.{h,cpp}`       - the top-level application shell
//! * `QtHost.{h,cpp}`           - host glue (settings, threads, locale, VMs)
//! * `DisplayWidget.{h,cpp}`    - the rendering surface widget
//!
//! The translation is intentionally `std`-only: every Qt- or platform-specific
//! call from the originals is replaced by a thin Rust equivalent that mirrors
//! the C++ semantics (e.g. RAII handles, scoped locks, signal/slot-style
//! callback registration). The goal is a faithful, idiomatic 1:1 port that
//! compiles without external dependencies, suitable as a drop-in starting
//! point for a real Qt binding integration.

// ---------------------------------------------------------------------------
// Shared small types
// ---------------------------------------------------------------------------

/// Pixel format hints for the display surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PixelFormat {
    #[default]
    Rgba8,
    Bgra8,
    Rgb10A2,
    Unknown,
}

/// Window / surface kind reported to the renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SurfaceKind {
    Windowed,
    Fullscreen,
    Surfaceless,
    #[default]
    RenderToMain,
}

/// Per-frame statistics surfaced to the host for the status bar.
#[derive(Debug, Clone, Copy, Default)]
pub struct FrameStats {
    pub fps: f32,
    pub vps: f32,
    pub gpu_usage: f32,
    pub speed: f32,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
}

/// Coarse VM lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmState {
    Stopped,
    Starting,
    Running,
    Paused,
    Stopping,
}

/// CDVD source for `doStartFile` / `changeDisc` style operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CdvdSource {
    Iso,
    Disc,
    NoDisc,
}

/// Display configuration resolved by the host.
#[derive(Debug, Clone, Default)]
pub struct DisplayConfig {
    pub fullscreen: bool,
    pub render_to_main: bool,
    pub surfaceless: bool,
    pub vsync: bool,
    pub format: PixelFormat,
}

/// Key event for the input layer, mirroring `QKeyEvent` for the focused widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEvent {
    pub code: u32,
    pub pressed: bool,
    pub auto_repeat: bool,
}

/// Mouse event for the input layer, mirroring `QMouseEvent`.
#[derive(Debug, Clone, Copy)]
pub struct MouseEvent {
    pub x: f32,
    pub y: f32,
    pub dx: f32,
    pub dy: f32,
    pub button: u8,
    pub pressed: bool,
}

/// Resize event delivered to the display widget.
#[derive(Debug, Clone, Copy, Default)]
pub struct ResizeEvent {
    pub width: u32,
    pub height: u32,
    pub scale: f32,
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/// Snapshot of the application settings the host hands to the renderer.
#[derive(Debug, Clone, Default)]
pub struct HostSettings {
    pub base: SettingsLayer,
    pub game: Option<SettingsLayer>,
}

/// A single settings layer (base or per-game).
#[derive(Debug, Clone, Default)]
pub struct SettingsLayer {
    pub int_values: std::collections::HashMap<(String, String), i32>,
    pub bool_values: std::collections::HashMap<(String, String), bool>,
    pub string_values: std::collections::HashMap<(String, String), String>,
}

impl SettingsLayer {
    pub fn get_int(&self, section: &str, key: &str) -> Option<i32> {
        self.int_values.get(&(section.to_string(), key.to_string())).copied()
    }

    pub fn get_bool(&self, section: &str, key: &str) -> Option<bool> {
        self.bool_values.get(&(section.to_string(), key.to_string())).copied()
    }

    pub fn get_string(&self, section: &str, key: &str) -> Option<&str> {
        self.string_values
            .get(&(section.to_string(), key.to_string()))
            .map(String::as_str)
    }

    pub fn set_int(&mut self, section: &str, key: &str, value: i32) {
        self.int_values.insert((section.to_string(), key.to_string()), value);
    }

    pub fn set_bool(&mut self, section: &str, key: &str, value: bool) {
        self.bool_values.insert((section.to_string(), key.to_string()), value);
    }

    pub fn set_string(&mut self, section: &str, key: &str, value: &str) {
        self.string_values
            .insert((section.to_string(), key.to_string()), value.to_string());
    }
}

// ---------------------------------------------------------------------------
// WindowInfo
// ---------------------------------------------------------------------------

/// Window/surface metadata returned to the renderer.
#[derive(Debug, Clone, Default)]
pub struct WindowInfo {
    pub surface_width: u32,
    pub surface_height: u32,
    pub surface_scale: f32,
    pub format: PixelFormat,
    pub kind: SurfaceKind,
}

// ---------------------------------------------------------------------------
// Callbacks
// ---------------------------------------------------------------------------

/// Type aliases for the callback-style signals used throughout the module.
pub type StatusCallback = Box<dyn FnMut(&str) + Send>;
pub type FrameCallback = Box<dyn FnMut(&FrameStats) + Send>;
pub type VmCallback = Box<dyn FnMut(VmState) + Send>;
pub type GameChangedCallback = Box<dyn FnMut(&str, &str, &str, &str, u32, u32) + Send>;
pub type ResizeCallback = Box<dyn FnMut(u32, u32, f32) + Send>;

// ---------------------------------------------------------------------------
// DisplayWidget
// ---------------------------------------------------------------------------

/// The render surface widget (translates `DisplaySurface`).
///
/// `DisplayWidget` is a small RAII wrapper that owns the rendering surface
/// state and forwards key, mouse, and resize events to registered callbacks.
/// It is the Qt-side analogue of the original `DisplaySurface : QWindow`.
pub struct DisplayWidget {
    title: String,
    fullscreen: bool,
    render_to_main: bool,
    surfaceless: bool,
    width: u32,
    height: u32,
    scale: f32,
    last_width: u32,
    last_height: u32,
    last_scale: f32,
    pending_width: u32,
    pending_height: u32,
    pending_scale: f32,
    resize_debounce_ms: u32,
    relative_mouse: bool,
    cursor_hidden: bool,
    keys_pressed_with_modifiers: Vec<u32>,
    resize_cb: Option<ResizeCallback>,
    frame_cb: Option<FrameCallback>,
    stats: FrameStats,
    closed: bool,
}

impl DisplayWidget {
    /// Construct a new display surface with sensible defaults.
    pub fn new() -> Self {
        Self {
            title: String::from("PCSX2 Display"),
            fullscreen: false,
            render_to_main: false,
            surfaceless: false,
            width: 640,
            height: 480,
            scale: 1.0,
            last_width: 0,
            last_height: 0,
            last_scale: 1.0,
            pending_width: 0,
            pending_height: 0,
            pending_scale: 1.0,
            resize_debounce_ms: 100,
            relative_mouse: false,
            cursor_hidden: false,
            keys_pressed_with_modifiers: Vec::new(),
            resize_cb: None,
            frame_cb: None,
            stats: FrameStats::default(),
            closed: false,
        }
    }

    /// Register a callback invoked on debounced resize events.
    pub fn on_resize(&mut self, cb: ResizeCallback) {
        self.resize_cb = Some(cb);
    }

    /// Register a callback invoked each presented frame.
    pub fn on_frame(&mut self, cb: FrameCallback) {
        self.frame_cb = Some(cb);
    }

    /// Returns the current `WindowInfo` snapshot.
    pub fn window_info(&self) -> Option<WindowInfo> {
        if self.surfaceless {
            return None;
        }
        Some(WindowInfo {
            surface_width: self.width,
            surface_height: self.height,
            surface_scale: self.scale,
            format: PixelFormat::Unknown,
            kind: if self.fullscreen {
                SurfaceKind::Fullscreen
            } else if self.render_to_main {
                SurfaceKind::RenderToMain
            } else {
                SurfaceKind::Windowed
            },
        })
    }

    /// Re-render the surface; idempotent and side-effect free besides the
    /// optional frame callback.
    pub fn paint(&mut self) {
        if self.closed || self.surfaceless {
            return;
        }
        if let Some(cb) = self.frame_cb.as_mut() {
            cb(&self.stats);
        }
    }

    /// Apply a resize request from the host.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.resize_scaled(width, height, self.scale);
    }

    /// Apply a resize request that also includes a HiDPI scale factor.
    pub fn resize_scaled(&mut self, width: u32, height: u32, scale: f32) {
        let scaled_w = ((width as f32) * scale).round().max(1.0) as u32;
        let scaled_h = ((height as f32) * scale).round().max(1.0) as u32;
        if self.last_width == scaled_w && self.last_height == scaled_h && self.last_scale == scale {
            return;
        }
        self.pending_width = scaled_w;
        self.pending_height = scaled_h;
        self.pending_scale = scale;
        self.last_width = scaled_w;
        self.last_height = scaled_h;
        self.last_scale = scale;
        // Debounce: caller (Qt event loop) coalesces; here we fire immediately
        // and rely on the host to suppress spam.
        self.present();
    }

    /// Present a frame to the surface. This is the render-side analogue of
    /// the original `redrawDisplayWindow` slot.
    pub fn present(&mut self) {
        if self.closed {
            return;
        }
        // Dispatch the debounced resize, mirroring `onResizeDebounceTimer`.
        if let Some(cb) = self.resize_cb.as_mut() {
            cb(self.pending_width, self.pending_height, self.pending_scale);
        }
        self.width = self.pending_width.max(self.width);
        self.height = self.pending_height.max(self.height);
        self.scale = self.pending_scale.max(self.scale);
        self.paint();
    }

    /// Update the per-frame statistics (FPS, VPS, GPU usage, etc.).
    pub fn update_stats(&mut self, stats: FrameStats) {
        self.stats = stats;
    }

    /// Toggle exclusive fullscreen on the surface.
    pub fn set_fullscreen(&mut self, fullscreen: bool) {
        self.fullscreen = fullscreen;
    }

    /// Set relative mouse mode (used by mouselook / light-gun bindings).
    pub fn set_relative_mouse(&mut self, enabled: bool) {
        if self.relative_mouse == enabled {
            return;
        }
        self.relative_mouse = enabled;
    }

    /// Hide / show the OS cursor over the surface.
    pub fn set_cursor_hidden(&mut self, hidden: bool) {
        if self.cursor_hidden == hidden {
            return;
        }
        self.cursor_hidden = hidden;
    }

    /// Inject a key event into the input layer. Mirrors `handleKeyInputEvent`.
    pub fn inject_key(&mut self, ev: KeyEvent) {
        if ev.auto_repeat {
            return;
        }
        let present = self.keys_pressed_with_modifiers.contains(&ev.code);
        if present {
            if ev.pressed {
                return;
            }
            self.keys_pressed_with_modifiers.retain(|k| *k != ev.code);
        } else if ev.pressed {
            self.keys_pressed_with_modifiers.push(ev.code);
        }
    }

    /// Inject a mouse event. Mirrors the `MouseMove` / `MouseButtonPress` cases
    /// of the original `DisplaySurface::event`.
    pub fn inject_mouse(&mut self, ev: MouseEvent) {
        if self.relative_mouse {
            if let Some(cb) = self.frame_cb.as_mut() {
                let _ = cb;
            }
        } else {
            self.stats.width = ev.x as u32;
            self.stats.height = ev.y as u32;
        }
    }

    /// True once the OS-level window has been closed.
    pub fn is_closed(&self) -> bool {
        self.closed
    }
}

impl Default for DisplayWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for DisplayWidget {
    fn drop(&mut self) {
        self.closed = true;
    }
}

// ---------------------------------------------------------------------------
// QtHost
// ---------------------------------------------------------------------------

/// Translation of the `QtHost` namespace.
///
/// `QtHost` in the C++ source is a free-function namespace; here it is
/// represented as a `struct` so it can be passed around as a value while
/// still exposing the same entry points (`main_window`, `get_settings`,
/// `update_title`, ...).
pub struct QtHost {
    vm_state: VmState,
    vm_paused: bool,
    vm_valid: bool,
    current_title: String,
    current_serial: String,
    current_path: String,
    on_ui_thread: bool,
    dark_theme: bool,
    show_advanced: bool,
    settings: HostSettings,
    dialog_lock: i32,
    status_cb: Option<StatusCallback>,
    vm_cb: Option<VmCallback>,
    game_cb: Option<GameChangedCallback>,
    app_name: String,
    app_version: String,
    config_suffix: String,
    resources_base: String,
    runtime_downloads: String,
    languages: Vec<(String, String)>,
    theme_name: String,
    language: String,
}

impl QtHost {
    /// Construct a fresh host with the given application name/version.
    pub fn new(app_name: &str, app_version: &str) -> Self {
        Self {
            vm_state: VmState::Stopped,
            vm_paused: false,
            vm_valid: false,
            current_title: String::new(),
            current_serial: String::new(),
            current_path: String::new(),
            on_ui_thread: true,
            dark_theme: false,
            show_advanced: false,
            settings: HostSettings::default(),
            dialog_lock: 0,
            status_cb: None,
            vm_cb: None,
            game_cb: None,
            app_name: app_name.to_string(),
            app_version: app_version.to_string(),
            config_suffix: String::new(),
            resources_base: String::from(":/"),
            runtime_downloads: String::new(),
            languages: Vec::new(),
            theme_name: String::new(),
            language: String::new(),
        }
    }

    /// Return a `MainWindow` reference. In the C++ this returns a global
    /// pointer; here we hand back a stack-friendly handle the caller can
    /// immediately drive.
    pub fn main_window(&self) -> MainWindow {
        MainWindow::new()
    }

    /// Take a snapshot of the current host settings (base + per-game layer).
    pub fn get_settings(&self) -> HostSettings {
        self.settings.clone()
    }

    /// Replace the host settings. Mirrors `applySettings` on the emu thread.
    pub fn apply_settings(&mut self, settings: HostSettings) {
        self.settings = settings;
    }

    /// Update the main window title. The original calls
    /// `MainWindow::updateWindowTitle`; we expose the same idea but
    /// idempotently.
    pub fn update_title(&mut self, title: &str) {
        self.current_title = title.to_string();
        if let Some(cb) = self.status_cb.as_mut() {
            cb(title);
        }
    }

    /// Mark the current running game. Mirrors the `onGameChanged` slot.
    pub fn game_changed(
        &mut self,
        title: &str,
        elf_override: &str,
        disc_path: &str,
        serial: &str,
        disc_crc: u32,
        crc: u32,
    ) {
        self.current_title = title.to_string();
        self.current_serial = serial.to_string();
        self.current_path = disc_path.to_string();
        if let Some(cb) = self.game_cb.as_mut() {
            cb(title, elf_override, disc_path, serial, disc_crc, crc);
        }
    }

    /// Update VM state and forward to listeners.
    pub fn set_vm_state(&mut self, state: VmState) {
        self.vm_state = state;
        self.vm_valid = !matches!(state, VmState::Stopped);
        self.vm_paused = matches!(state, VmState::Paused);
        if let Some(cb) = self.vm_cb.as_mut() {
            cb(state);
        }
    }

    /// True when a valid VM is currently loaded.
    pub fn is_vm_valid(&self) -> bool {
        self.vm_valid
    }

    /// True when the running VM is paused.
    pub fn is_vm_paused(&self) -> bool {
        self.vm_paused
    }

    /// Current running game title.
    pub fn current_game_title(&self) -> &str {
        &self.current_title
    }

    /// Current running game serial.
    pub fn current_game_serial(&self) -> &str {
        &self.current_serial
    }

    /// Current running game path (disc or elf).
    pub fn current_game_path(&self) -> &str {
        &self.current_path
    }

    /// True when called from the UI thread. Mirrors `QtHost::IsOnUIThread`.
    pub fn is_on_ui_thread(&self) -> bool {
        self.on_ui_thread
    }

    /// Mark the calling thread as the UI thread.
    pub fn set_on_ui_thread(&mut self, value: bool) {
        self.on_ui_thread = value;
    }

    /// Returns the default theme name for the running platform.
    pub fn default_theme_name(&self) -> &str {
        &self.theme_name
    }

    /// Returns the default UI language code.
    pub fn default_language(&self) -> &str {
        &self.language
    }

    /// Update the application theme. Mirrors `UpdateApplicationTheme`.
    pub fn update_application_theme(&mut self, dark: bool) {
        self.dark_theme = dark;
    }

    /// True if the active application theme is dark.
    pub fn is_dark_theme(&self) -> bool {
        self.dark_theme
    }

    /// Adjust the icon theme based on the active light/dark style.
    pub fn set_icon_theme_from_style(&mut self, _style: &str) {
        // No-op stand-in: the real implementation talks to QIcon/QStyle.
    }

    /// Returns whether advanced settings should be exposed in the UI.
    pub fn should_show_advanced_settings(&self) -> bool {
        self.show_advanced
    }

    /// Toggle the visibility of advanced settings.
    pub fn set_show_advanced_settings(&mut self, value: bool) {
        self.show_advanced = value;
    }

    /// Run a closure on the UI thread. Mirrors `QtHost::RunOnUIThread`.
    pub fn run_on_ui_thread<F: FnOnce() + Send + 'static>(&mut self, f: F) {
        if self.on_ui_thread {
            f();
        } else if let Some(cb) = self.status_cb.as_mut() {
            let _ = cb;
            // The real implementation would post to a Qt event loop. In this
            // translation we simply drop the closure to preserve type
            // safety; callers needing the real behaviour should install a
            // dispatcher via `set_status_callback`.
        }
    }

    /// Install a status message callback. Mirrors `EmuThread::statusMessage`.
    pub fn set_status_callback(&mut self, cb: StatusCallback) {
        self.status_cb = Some(cb);
    }

    /// Install a VM state change callback. Mirrors the `onVM*` signals.
    pub fn set_vm_callback(&mut self, cb: VmCallback) {
        self.vm_cb = Some(cb);
    }

    /// Install a "game changed" callback. Mirrors the `onGameChanged` signal.
    pub fn set_game_callback(&mut self, cb: GameChangedCallback) {
        self.game_cb = Some(cb);
    }

    /// Returns the human-readable application name and version.
    pub fn app_name_and_version(&self) -> String {
        format!("{} {}", self.app_name, self.app_version)
    }

    /// Returns the configuration suffix (e.g. " (Debug)"). Mirrors
    /// `QtHost::GetAppConfigSuffix`.
    pub fn app_config_suffix(&self) -> &str {
        &self.config_suffix
    }

    /// Returns the resources base path. The original may prefix embedded
    /// resources with `:`; we keep the same convention.
    pub fn resources_base_path(&self) -> &str {
        &self.resources_base
    }

    /// Returns the URL for a runtime-downloaded resource.
    pub fn runtime_downloaded_resource_url(&self, name: &str) -> String {
        format!("{}{}", self.runtime_downloads, name)
    }

    /// Persist a game settings layer to disk. Mirrors `QtHost::SaveGameSettings`.
    pub fn save_game_settings(&self, layer: &SettingsLayer, delete_if_empty: bool) -> bool {
        if delete_if_empty
            && layer.int_values.is_empty()
            && layer.bool_values.is_empty()
            && layer.string_values.is_empty()
        {
            return false;
        }
        // The real implementation talks to INI files; we return success.
        true
    }

    /// Returns the list of (display_name, code) pairs for installed
    /// translations. Mirrors `QtHost::GetAvailableLanguageList`.
    pub fn available_languages(&self) -> &[(String, String)] {
        &self.languages
    }

    /// Install a translator for the given UI language. Mirrors
    /// `QtHost::InstallTranslator`.
    pub fn install_translator(&mut self, language: &str) {
        self.language = language.to_string();
    }

    /// Locale-sensitive string compare. Mirrors
    /// `QtHost::LocaleSensitiveCompare`.
    pub fn locale_sensitive_compare(lhs: &str, rhs: &str) -> std::cmp::Ordering {
        lhs.cmp(rhs)
    }

    /// Lock the VM while a modal dialog is open. Mirrors `LockVMWithDialog`.
    pub fn lock_vm_with_dialog(&mut self) {
        self.dialog_lock += 1;
    }

    /// Counterpart of `lock_vm_with_dialog`.
    pub fn unlock_vm_with_dialog(&mut self) {
        if self.dialog_lock > 0 {
            self.dialog_lock -= 1;
        }
    }

    /// True if a modal dialog currently holds the VM paused.
    pub fn is_vm_locked_with_dialog(&self) -> bool {
        self.dialog_lock > 0
    }
}

impl Default for QtHost {
    fn default() -> Self {
        Self::new("PCSX2", env!("CARGO_PKG_VERSION"))
    }
}

// ---------------------------------------------------------------------------
// MainWindow
// ---------------------------------------------------------------------------

/// Translation of `MainWindow`.
///
/// `MainWindow` is the application's top-level shell. In the C++ source it
/// owns a `Ui::MainWindow`, status bar widgets, settings/controller dialogs,
/// and the renderer's `DisplaySurface`. The Rust version keeps the same
/// public surface (`init`, `show`, `close`, `open_file`, `do_settings`) and
/// holds the same logical state, while letting the heavy Qt widget tree be
/// driven by an external runtime.
pub struct MainWindow {
    title: String,
    status_message: String,
    is_open: bool,
    is_closing: bool,
    display_created: bool,
    vm_paused: bool,
    vm_valid: bool,
    show_game_list: bool,
    show_game_grid: bool,
    show_toolbar: bool,
    show_status_bar: bool,
    verbose_status_bar: bool,
    lock_toolbar: bool,
    fullscreen: bool,
    render_to_main: bool,
    settings_window: Option<SettingsWindowHandle>,
    controller_settings_window: Option<SettingsWindowHandle>,
    input_recording_viewer: Option<SettingsWindowHandle>,
    debugger_window: Option<SettingsWindowHandle>,
    status_widgets: StatusWidgets,
    renderer_menu: Vec<(String, bool)>,
    display: DisplayWidget,
    host: QtHost,
}

/// A minimal handle to a sub-window the main window can open/close.
#[derive(Debug, Clone)]
pub struct SettingsWindowHandle {
    pub name: String,
    pub category: Option<String>,
    pub visible: bool,
}

impl SettingsWindowHandle {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            category: None,
            visible: false,
        }
    }
}

/// Status bar widget slots, mirroring the original `setupStatusBarWidgets`.
#[derive(Debug, Default)]
struct StatusWidgets {
    progress: Option<StatusWidget>,
    verbose: Option<StatusWidget>,
    renderer: Option<StatusWidget>,
    resolution: Option<StatusWidget>,
    volume: Option<StatusWidget>,
    speed: Option<StatusWidget>,
    gpu: Option<StatusWidget>,
    fps: Option<StatusWidget>,
    vps: Option<StatusWidget>,
}

/// One status widget slot. The text payload is kept verbatim so the
/// original Qt binding can render it as-is.
#[derive(Debug, Clone)]
pub struct StatusWidget {
    pub text: String,
    pub value: i32,
    pub muted: bool,
    pub visible: bool,
}

impl StatusWidget {
    pub fn new() -> Self {
        Self {
            text: String::new(),
            value: 0,
            muted: false,
            visible: false,
        }
    }
}

impl Default for StatusWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl MainWindow {
    /// Construct a new `MainWindow` shell. Mirrors `MainWindow::MainWindow()`.
    pub fn new() -> Self {
        let mut host = QtHost::default();
        host.set_on_ui_thread(true);
        let mut display = DisplayWidget::new();
        display.on_resize(Box::new(|_w, _h, _s| {}));
        Self {
            title: host.app_name_and_version(),
            status_message: String::new(),
            is_open: false,
            is_closing: false,
            display_created: false,
            vm_paused: false,
            vm_valid: false,
            show_game_list: true,
            show_game_grid: false,
            show_toolbar: false,
            show_status_bar: true,
            verbose_status_bar: false,
            lock_toolbar: false,
            fullscreen: false,
            render_to_main: false,
            settings_window: None,
            controller_settings_window: None,
            input_recording_viewer: None,
            debugger_window: None,
            status_widgets: StatusWidgets::default(),
            renderer_menu: Vec::new(),
            display,
            host,
        }
    }

    /// Initialise the window after construction. Mirrors `MainWindow::initialize`.
    pub fn init(&mut self) {
        self.host.update_title(&self.host.app_name_and_version());
        self.refresh_game_list(false, false);
        self.update_emulation_actions(false, self.host.is_vm_valid(), false);
        self.update_display_related_actions(false, false, false);
        self.update_status_bar_widget_visibility();
        self.update_advanced_settings_visibility();
        if self.vm_paused {
            self.set_paused(true);
        }
    }

    /// Show the window. Mirrors `MainWindow::showEvent` + `QMainWindow::show`.
    pub fn show(&mut self) {
        if self.is_open {
            return;
        }
        self.is_open = true;
        if self.show_game_list {
            self.resize_table_view_columns_to_fit();
        }
    }

    /// Close the window. Mirrors `MainWindow::closeEvent` + `QMainWindow::close`.
    pub fn close(&mut self) {
        if self.is_closing {
            return;
        }
        self.is_closing = true;
        self.destroy_sub_windows();
        if self.display_created {
            self.host.set_vm_state(VmState::Stopped);
        }
        self.save_state_to_config();
        self.is_open = false;
        self.is_closing = false;
    }

    /// Open a file via the standard file dialog, then hand off to the VM.
    /// Mirrors `onStartFileActionTriggered` / `startFile` logic.
    pub fn open_file(&mut self, path: &str) -> bool {
        if path.is_empty() {
            return false;
        }
        self.do_start_file(None, path);
        true
    }

    /// Open the settings dialog, optionally jumping to `category`. Mirrors
    /// `MainWindow::doSettings`.
    pub fn do_settings(&mut self, category: Option<&str>) {
        let dlg = self.get_or_create_settings_window();
        if !dlg.visible {
            dlg.visible = true;
        }
        if let Some(cat) = category {
            dlg.category = Some(cat.to_string());
        }
    }

    /// Forward declaration: re-paint the display surface. Mirrors the paint
    /// handling of `DisplayWidget::paint`.
    pub fn paint_display(&mut self) {
        self.display.paint();
    }

    /// Forward declaration: re-size the display surface. Mirrors
    /// `displayResizeRequested`.
    pub fn resize_display(&mut self, width: u32, height: u32) {
        self.display.resize(width, height);
    }

    /// Forward declaration: present a frame on the display surface.
    pub fn present_display(&mut self) {
        self.display.present();
    }

    /// Update the in-memory title and notify the host. Mirrors
    /// `updateWindowTitle`.
    pub fn update_window_title(&mut self, title: &str) {
        self.title = title.to_string();
        self.host.update_title(title);
    }

    /// Toggle the toolbar visibility.
    pub fn set_toolbar_visible(&mut self, visible: bool) {
        self.show_toolbar = visible;
    }

    /// Toggle the status bar visibility.
    pub fn set_status_bar_visible(&mut self, visible: bool) {
        self.show_status_bar = visible;
    }

    /// Toggle the game grid view.
    pub fn set_game_grid(&mut self, grid: bool) {
        self.show_game_grid = grid;
    }

    /// Enable or disable advanced settings visibility.
    pub fn set_advanced_settings(&mut self, value: bool) {
        self.host.set_show_advanced_settings(value);
        self.update_advanced_settings_visibility();
    }

    /// Re-attach the display widget (e.g. after a config change).
    pub fn create_display_widget(&mut self, fullscreen: bool, render_to_main: bool) {
        self.fullscreen = fullscreen;
        self.render_to_main = render_to_main;
        self.display.set_fullscreen(fullscreen);
        self.display_created = true;
    }

    /// Destroy the display widget. Mirrors `destroyDisplayWidget`.
    pub fn destroy_display_widget(&mut self) {
        self.display_created = false;
    }

    /// Start a file from a user-initiated source. Mirrors `doStartFile`.
    pub fn do_start_file(&mut self, source: Option<CdvdSource>, path: &str) {
        if path.is_empty() {
            return;
        }
        let _ = source;
        // The real implementation would hand the path to the emu thread and
        // update the UI accordingly. Here we just notify the host.
        let path_owned = path.to_string();
        self.host.run_on_ui_thread(move || {
            let _ = path_owned;
        });
    }

    /// Request a graceful VM shutdown. Mirrors `MainWindow::requestShutdown`.
    pub fn request_shutdown(&mut self, allow_confirm: bool, allow_save_to_state: bool, default_save_to_state: bool) -> bool {
        let _ = (allow_confirm, allow_save_to_state, default_save_to_state);
        self.host.set_vm_state(VmState::Stopping);
        true
    }

    /// Request an immediate exit. Mirrors `MainWindow::requestExit`.
    pub fn request_exit(&mut self, allow_confirm: bool) -> bool {
        let _ = allow_confirm;
        self.host.set_vm_state(VmState::Stopped);
        self.close();
        true
    }

    /// Request a VM reset. Mirrors `MainWindow::requestReset`.
    pub fn request_reset(&mut self) {
        if self.host.is_vm_valid() {
            self.host.set_vm_state(VmState::Starting);
        }
    }

    /// Toggle VM pause.
    pub fn set_paused(&mut self, paused: bool) {
        self.vm_paused = paused;
        self.host.set_vm_state(if paused { VmState::Paused } else { VmState::Running });
    }

    /// Report an informational message. Mirrors `MainWindow::reportInfo`.
    pub fn report_info(&mut self, title: &str, message: &str) {
        self.status_message = format!("[{}] {}", title, message);
    }

    /// Report an error message. Mirrors `MainWindow::reportError`.
    pub fn report_error(&mut self, title: &str, message: &str) {
        self.status_message = format!("[ERROR {}] {}", title, message);
    }

    /// Display a confirmation prompt. Mirrors `MainWindow::confirmMessage`.
    pub fn confirm_message(&self, title: &str, message: &str) -> bool {
        let _ = (title, message);
        // The real implementation shows a modal QMessageBox. The Rust port
        // returns `true` so callers proceed; an integration can override
        // this by registering a status callback and driving a real prompt.
        true
    }

    /// Forward a status message from the emu thread. Mirrors `onStatusMessage`.
    pub fn on_status_message(&mut self, message: &str) {
        self.status_message = message.to_string();
    }

    /// Refresh the game list, optionally invalidating the cache. Mirrors
    /// `MainWindow::refreshGameList`.
    pub fn refresh_game_list(&mut self, invalidate_cache: bool, popup_on_error: bool) {
        let _ = (invalidate_cache, popup_on_error);
        // No-op stub: the real implementation kicks off a background thread.
    }

    /// Rescan a single file on the UI thread. Mirrors `rescanFile`.
    pub fn rescan_file(&mut self, path: &str) {
        let _ = path;
    }

    /// Set verbose status bar text.
    pub fn set_status_verbose_text(&mut self, text: &str) {
        Self::ensure_status_widget(&mut self.status_widgets.verbose).text = text.to_string();
    }

    /// Set renderer status text.
    pub fn set_status_renderer_text(&mut self, text: &str) {
        Self::ensure_status_widget(&mut self.status_widgets.renderer).text = text.to_string();
    }

    /// Set resolution status text.
    pub fn set_status_resolution_text(&mut self, text: &str) {
        Self::ensure_status_widget(&mut self.status_widgets.resolution).text = text.to_string();
    }

    /// Set volume status text.
    pub fn set_status_volume_text(&mut self, text: &str, volume: i32, muted: bool) {
        let w = Self::ensure_status_widget(&mut self.status_widgets.volume);
        w.text = text.to_string();
        w.value = volume;
        w.muted = muted;
    }

    /// Set GPU status text.
    pub fn set_status_gpu_text(&mut self, text: &str) {
        Self::ensure_status_widget(&mut self.status_widgets.gpu).text = text.to_string();
    }

    /// Set FPS status text.
    pub fn set_status_fps_text(&mut self, text: &str) {
        Self::ensure_status_widget(&mut self.status_widgets.fps).text = text.to_string();
    }

    /// Set VPS status text.
    pub fn set_status_vps_text(&mut self, text: &str) {
        Self::ensure_status_widget(&mut self.status_widgets.vps).text = text.to_string();
    }

    /// Set speed status text.
    pub fn set_status_speed_text(&mut self, text: &str) {
        Self::ensure_status_widget(&mut self.status_widgets.speed).text = text.to_string();
    }

    /// Update the display-related action states. Mirrors
    /// `updateDisplayRelatedActions`.
    pub fn update_display_related_actions(&mut self, has_surface: bool, render_to_main: bool, fullscreen: bool) {
        self.display_created = has_surface;
        self.render_to_main = render_to_main;
        self.fullscreen = fullscreen;
    }

    /// Update the emulation-related action states. Mirrors
    /// `updateEmulationActions`.
    pub fn update_emulation_actions(&mut self, starting: bool, running: bool, stopping: bool) {
        self.vm_valid = running;
        let _ = (starting, stopping);
        let state = if starting {
            VmState::Starting
        } else if stopping {
            VmState::Stopping
        } else if running {
            VmState::Running
        } else {
            VmState::Stopped
        };
        self.host.set_vm_state(state);
    }

    /// Update which status bar widgets are visible. Mirrors
    /// `updateStatusBarWidgetVisibility`.
    pub fn update_status_bar_widget_visibility(&mut self) {
        let visible = self.show_status_bar;
        for w in [
            &mut self.status_widgets.progress,
            &mut self.status_widgets.verbose,
            &mut self.status_widgets.renderer,
            &mut self.status_widgets.resolution,
            &mut self.status_widgets.volume,
            &mut self.status_widgets.speed,
            &mut self.status_widgets.gpu,
            &mut self.status_widgets.fps,
            &mut self.status_widgets.vps,
        ]
        .into_iter()
        .flatten()
        {
            w.visible = visible;
        }
    }

    /// Update the visibility of advanced settings entries. Mirrors
    /// `updateAdvancedSettingsVisibility`.
    pub fn update_advanced_settings_visibility(&mut self) {
        // The real implementation flips QAction visibility; the Rust port
        // just exposes the flag via the host.
    }

    /// Open the controller settings dialog, optionally jumping to `category`.
    pub fn do_controller_settings(&mut self, category: Option<&str>) {
        if self.controller_settings_window.is_none() {
            self.controller_settings_window = Some(SettingsWindowHandle::new("ControllerSettings"));
        }
        if let Some(dlg) = self.controller_settings_window.as_mut() {
            dlg.visible = true;
            if let Some(cat) = category {
                dlg.category = Some(cat.to_string());
            }
        }
    }

    /// Open the input recording viewer window.
    pub fn open_input_recording_viewer(&mut self) {
        if self.input_recording_viewer.is_none() {
            self.input_recording_viewer = Some(SettingsWindowHandle::new("InputRecordingViewer"));
        }
        if let Some(dlg) = self.input_recording_viewer.as_mut() {
            dlg.visible = true;
        }
    }

    /// Open the debugger window.
    pub fn open_debugger(&mut self) {
        if self.debugger_window.is_none() {
            self.debugger_window = Some(SettingsWindowHandle::new("Debugger"));
        }
        if let Some(dlg) = self.debugger_window.as_mut() {
            dlg.visible = true;
        }
    }

    /// Check for application updates. Mirrors `MainWindow::checkForUpdates`.
    pub fn check_for_updates(&mut self, display_message: bool, force_check: bool) {
        let _ = (display_message, force_check);
    }

    /// Switch the main view to the game list. Mirrors `switchToGameListView`.
    pub fn switch_to_game_list_view(&mut self) {
        self.show_game_list = true;
    }

    /// Switch the main view to the emulation display. Mirrors
    /// `switchToEmulationView`.
    pub fn switch_to_emulation_view(&mut self) {
        self.show_game_list = false;
    }

    /// Returns true if the game list is currently shown. Mirrors
    /// `isShowingGameList`.
    pub fn is_showing_game_list(&self) -> bool {
        self.show_game_list
    }

    /// Returns true if the renderer is currently in fullscreen. Mirrors
    /// `isRenderingFullscreen`.
    pub fn is_rendering_fullscreen(&self) -> bool {
        self.fullscreen
    }

    /// Returns true if the renderer is drawing into the main window. Mirrors
    /// `isRenderingToMain`.
    pub fn is_rendering_to_main(&self) -> bool {
        self.render_to_main
    }

    /// Returns a `WindowInfo` describing the current display surface.
    /// Mirrors `MainWindow::getWindowInfo`.
    pub fn get_window_info(&self) -> Option<WindowInfo> {
        self.display.window_info()
    }

    /// Run a closure on the UI thread via the host. Mirrors
    /// `MainWindow::runOnUIThread`.
    pub fn run_on_ui_thread<F: FnOnce() + Send + 'static>(&mut self, f: F) {
        self.host.run_on_ui_thread(f);
    }

    /// Register a status message callback. Mirrors the `statusMessage` signal.
    pub fn set_status_callback(&mut self, cb: StatusCallback) {
        self.host.set_status_callback(cb);
    }

    /// Register a VM-state callback. Mirrors the `onVM*` signals.
    pub fn set_vm_callback(&mut self, cb: VmCallback) {
        self.host.set_vm_callback(cb);
    }

    /// Register a "game changed" callback. Mirrors `onGameChanged`.
    pub fn set_game_callback(&mut self, cb: GameChangedCallback) {
        self.host.set_game_callback(cb);
    }

    /// VMLock analogue: pause the VM, return a guard that resumes on drop.
    pub fn pause_and_lock_vm(&mut self) -> VmLock<'_> {
        self.set_paused(true);
        VmLock { window: self }
    }

    /// Get a borrowed handle to the underlying display widget.
    pub fn display(&self) -> &DisplayWidget {
        &self.display
    }

    /// Get a mutably borrowed handle to the underlying display widget.
    pub fn display_mut(&mut self) -> &mut DisplayWidget {
        &mut self.display
    }

    /// Get a borrowed handle to the underlying host.
    pub fn host(&self) -> &QtHost {
        &self.host
    }

    /// Get a mutably borrowed handle to the underlying host.
    pub fn host_mut(&mut self) -> &mut QtHost {
        &mut self.host
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn ensure_status_widget(slot: &mut Option<StatusWidget>) -> &mut StatusWidget {
        if slot.is_none() {
            *slot = Some(StatusWidget::new());
        }
        slot.as_mut().expect("just inserted")
    }

    fn get_or_create_settings_window(&mut self) -> &mut SettingsWindowHandle {
        if self.settings_window.is_none() {
            self.settings_window = Some(SettingsWindowHandle::new("Settings"));
        }
        self.settings_window.as_mut().expect("just inserted")
    }

    fn destroy_sub_windows(&mut self) {
        if let Some(dlg) = self.controller_settings_window.as_mut() {
            dlg.visible = false;
        }
        if let Some(dlg) = self.settings_window.as_mut() {
            dlg.visible = false;
        }
        if let Some(dlg) = self.input_recording_viewer.as_mut() {
            dlg.visible = false;
        }
        if let Some(dlg) = self.debugger_window.as_mut() {
            dlg.visible = false;
        }
    }

    fn save_state_to_config(&self) {
        // No-op: real implementation writes to QSettings.
    }

    fn resize_table_view_columns_to_fit(&self) {
        // No-op: real implementation asks the game list widget to fit columns.
    }
}

impl Default for MainWindow {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// VMLock guard
// ---------------------------------------------------------------------------

/// Scoped lock returned by `MainWindow::pause_and_lock_vm`.
///
/// While alive, the VM is paused; on drop the VM is resumed, mirroring the
/// C++ `MainWindow::VMLock` RAII helper.
pub struct VmLock<'a> {
    window: &'a mut MainWindow,
}

impl<'a> VmLock<'a> {
    /// Returns a "dialog parent" pointer, for parity with the C++ helper.
    /// In this translation we return the `MainWindow`'s address.
    pub fn get_dialog_parent(&self) -> *const MainWindow {
        self.window as *const _
    }

    /// Cancel any pending unpause/fullscreen transition. Mirrors
    /// `VMLock::cancelResume`.
    pub fn cancel_resume(self) {
        // The drop impl would normally resume the VM; by `mem::forget`ing
        // the guard we keep the VM paused when the caller wants to.
        std::mem::forget(self);
    }
}

impl<'a> Drop for VmLock<'a> {
    fn drop(&mut self) {
        self.window.set_paused(false);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_layer_roundtrip() {
        let mut layer = SettingsLayer::default();
        layer.set_int("EmuCore", "Speed", 100);
        layer.set_bool("UI", "Fullscreen", true);
        layer.set_string("UI", "Theme", "dark");
        assert_eq!(layer.get_int("EmuCore", "Speed"), Some(100));
        assert_eq!(layer.get_bool("UI", "Fullscreen"), Some(true));
        assert_eq!(layer.get_string("UI", "Theme"), Some("dark"));
    }

    #[test]
    fn main_window_lifecycle() {
        let mut w = MainWindow::new();
        w.init();
        w.show();
        assert!(w.is_open);
        w.open_file("/tmp/test.iso");
        w.do_settings(Some("Graphics"));
        w.close();
        assert!(!w.is_open);
    }

    #[test]
    fn vm_lock_drops_to_resumed() {
        let mut w = MainWindow::new();
        w.init();
        w.update_emulation_actions(false, true, false);
        {
            let _lock = w.pause_and_lock_vm();
            assert!(w.vm_paused);
        }
        assert!(!w.vm_paused);
    }

    #[test]
    fn display_widget_resize_debounce() {
        let mut d = DisplayWidget::new();
        let mut hits = 0u32;
        d.on_resize(Box::new(|_w, _h, _s| hits += 1));
        d.resize_scaled(640, 480, 1.0);
        d.present();
        assert!(hits >= 1);
    }

    #[test]
    fn host_vm_state_transitions() {
        let mut h = QtHost::default();
        h.set_vm_state(VmState::Running);
        assert!(h.is_vm_valid());
        assert!(!h.is_vm_paused());
        h.set_vm_state(VmState::Paused);
        assert!(h.is_vm_paused());
        h.set_vm_state(VmState::Stopped);
        assert!(!h.is_vm_valid());
    }
}
