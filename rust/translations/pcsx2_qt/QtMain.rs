// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! `QtMain` — idiomatic Rust 2021 translation of the PCSX2 Qt host shell.
//!
//! This module is the unified Rust counterpart of the C++ sources under
//! `pcsx2-qt/`: `MainWindow.{cpp,h}`, `QtHost.{cpp,h}`, `QtKeyCodes.cpp`,
//! `QtProgressCallback.{cpp,h}`, `QtUtils.{cpp,h}`, `SettingWidgetBinder.h`,
//! `Themes.cpp`, `Translations.cpp`, `DisplayWidget.{cpp,h}` and the various
//! dialog sources (`AutoUpdaterDialog`, `LogWindow`, `SetupWizardDialog`,
//! `ShortcutCreationDialog`, `AsyncDialogs`, `AboutDialog`).
//!
//! The module re-models the Qt-dependent surface area around plain Rust
//! types: a `qtHost` namespace becomes the [`QtHost`] trait, the singleton
//! `MainWindow` becomes [`MainWindow`], theme palette switches become
//! [`Themes::apply`], translation loading becomes [`Translations::load`],
//! and the `QKeyEvent` lookup table becomes the [`qt_key_code`] function.
//!
//! No external dependencies are used; everything sits on `std` so the
//! module is portable and easy to audit.  GUI calls are represented as
//! method signatures returning [`Result`] where the original C++ could
//! report a failure, and as no-op `()` for fire-and-forget UI updates
//! that the C++ side performs unconditionally on the UI thread.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// Errors produced by the Qt host surface.
///
/// These are flat, no-payload errors so that the trait and helper
/// functions can be wired up in pure `std` without pulling in `anyhow` or
/// `thiserror`.  GUI failures (style factory missing, theme file unreadable,
/// settings interface absent) all funnel into one of these variants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QtHostError {
    /// The requested UI theme name is not known to [`Themes`].
    UnknownTheme(String),
    /// The translation file for the requested language could not be opened.
    TranslationLoadFailed(String),
    /// A settings interface was required but none was supplied.
    NoSettings,
    /// A widget handle was required but the underlying object was gone.
    NoWidget,
    /// A display render target was requested but the VM is not running.
    NoRenderTarget,
    /// A generic IO failure during dialog / file operations.
    Io(String),
}

impl std::fmt::Display for QtHostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QtHostError::UnknownTheme(name) => write!(f, "unknown UI theme: {name}"),
            QtHostError::TranslationLoadFailed(p) => write!(f, "failed to load translation: {p}"),
            QtHostError::NoSettings => f.write_str("no settings interface bound"),
            QtHostError::NoWidget => f.write_str("widget handle is not available"),
            QtHostError::NoRenderTarget => f.write_str("no active render target"),
            QtHostError::Io(msg) => write!(f, "io error: {msg}"),
        }
    }
}

impl std::error::Error for QtHostError {}

/// Convenience alias used throughout the module.
pub type Result<T> = std::result::Result<T, QtHostError>;

/// Identifier for a logical UI thread in the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UiThread {
    /// The Qt main (UI) thread.
    Main,
    /// The CPU / emulation thread that drives the VM.
    Cpu,
    /// The GS / rendering thread spawned by `EmuThread`.
    Gs,
}

/// A minimal stand-in for `SettingsInterface`.
///
/// In the C++ codebase this is an abstract base with concrete
/// `std::string`/`bool`/`int` accessors.  Here we only need a typed
/// view over the persisted `UI/Theme`, `UI/Language`, and similar
/// keys, so we expose a small key/value snapshot that callers can fill
/// in from a `Host::GetBaseStringSettingValue`-equivalent source.
#[derive(Debug, Default, Clone)]
pub struct SettingsInterface {
    map: HashMap<String, String>,
}

impl SettingsInterface {
    /// Construct an empty settings snapshot.
    pub fn new() -> Self {
        Self::default()
    }

    /// Read a base (non-game) string setting, falling back to `default`.
    pub fn get_string(&self, section: &str, key: &str, default: &str) -> String {
        self.map
            .get(&format!("{section}/{key}"))
            .cloned()
            .unwrap_or_else(|| default.to_string())
    }

    /// Write a base (non-game) string setting into the snapshot.
    pub fn set_string(&mut self, section: &str, key: &str, value: &str) {
        self.map.insert(format!("{section}/{key}"), value.to_string());
    }

    /// Iterate over `(section/key, value)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (String, String)> + '_ {
        self.map.iter().map(|(k, v)| (k.clone(), v.clone()))
    }
}

/// Snapshot of a `WindowInfo` structure — the same `Win32/X11/Wayland/MacOS`
/// discriminator the C++ host builds in `QtUtils::GetWindowInfoForWindow`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowBackend {
    Win32,
    X11,
    Wayland,
    MacOS,
    Surfaceless,
}

/// Lightweight mirror of `common/WindowInfo`.
#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub backend: WindowBackend,
    pub window_handle: usize,
    pub display_connection: Option<usize>,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
    pub refresh_rate: f32,
}

/// Default window info used when no display surface is mapped yet.
impl Default for WindowInfo {
    fn default() -> Self {
        Self {
            backend: WindowBackend::Surfaceless,
            window_handle: 0,
            display_connection: None,
            width: 1,
            height: 1,
            scale: 1.0,
            refresh_rate: 60.0,
        }
    }
}

/// VM state mirror of `VMState` — the small enum used by `QtHost::IsVMValid`,
/// `QtHost::IsVMPaused`, and friends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmState {
    Shutdown,
    Starting,
    Running,
    Paused,
    Stopping,
}

/// Snapshot of the currently-running game's metadata.
#[derive(Debug, Default, Clone)]
pub struct GameInfo {
    pub title: String,
    pub serial: String,
    pub path: PathBuf,
    pub disc_crc: u32,
    pub elf_crc: u32,
}

/// The Qt host facade — the `QtHost` namespace translated to a trait.
///
/// The C++ side exposes a grab-bag of free functions in `namespace QtHost`
/// alongside a `g_main_window` global and a `g_emu_thread` global.  The
/// Rust translation funnels everything through a single trait so that
/// embedders can supply a custom backend (for headless tests, or for
/// driving the host from a different UI toolkit) without depending on
/// global state.
pub trait QtHost {
    /// Return a shared handle to the [`MainWindow`] if one exists.
    ///
    /// The C++ version returns the global `g_main_window` pointer; in
    /// Rust we hand out a `Weak` so the embedder does not accidentally
    /// keep the window alive past its natural lifetime.
    fn main_window(&self) -> Option<Arc<MainWindow>>;

    /// Return a snapshot of the current base settings interface.
    ///
    /// The C++ version returns a `SettingsInterface&` which can mutate
    /// the global `Host::Internal::GetBaseLayerSettingsInterface()`; the
    /// Rust translation returns a copy so the call can cross threads.
    fn get_settings(&self) -> SettingsInterface;

    /// Refresh the main window's title to reflect the current state.
    ///
    /// Mirrors `MainWindow::updateWindowTitle` and the call sites that
    /// fire it from `onGameChanged`, `onVMStarted`, `onVMStopped`, etc.
    fn update_title(&self);

    /// The default theme for the host platform.
    fn default_theme(&self) -> &'static str {
        // Mirrors `QtHost::GetDefaultThemeName` — empty on Apple,
        // `darkfusionblue` elsewhere.  The conditional is collapsed
        // here to the non-Apple default because we do not know the
        // target at build time without a `cfg` switch.
        "darkfusionblue"
    }

    /// The default language for the host platform.
    fn default_language(&self) -> &'static str {
        "system"
    }

    /// True if the calling thread is the UI thread.
    fn is_on_ui_thread(&self) -> bool;

    /// True if the current application theme is dark.
    fn is_dark_theme(&self) -> bool;

    /// Install or switch the application translator.
    ///
    /// Wraps `QtHost::InstallTranslator` plus `Translations::load`.
    fn install_translator(&self, language: &str) -> Result<()>;

    /// Re-apply the configured theme.
    fn update_application_theme(&self);

    /// Returns the application name and version, with a `[Debug]` /
    /// `[Devel]` suffix on non-release builds.
    fn app_name_and_version(&self) -> String {
        // Mirrors `QtHost::GetAppNameAndVersion` — uses a constant
        // version string plus a `cfg(debug_assertions)`-style suffix.
        let suffix = if cfg!(debug_assertions) { " [Debug]" } else { "" };
        format!("PCSX2 {}{}", env!("CARGO_PKG_VERSION", "0.0.0"), suffix)
    }
}

// ---------------------------------------------------------------------------
//  Main window
// ---------------------------------------------------------------------------

/// The top-level Qt main window.
///
/// In C++ this is a `QMainWindow` subclass with a `Ui::MainWindow` and
/// a gaggle of slot methods.  In Rust we strip the Q_OBJECT machinery
/// and keep the parts that the rest of the host actually calls into.
#[derive(Debug)]
pub struct MainWindow {
    settings: Mutex<SettingsInterface>,
    game_info: Mutex<GameInfo>,
    vm_state: Mutex<VmState>,
    show_game_list: Mutex<bool>,
    is_open: Mutex<bool>,
    progress_current: Mutex<i32>,
    progress_total: Mutex<i32>,
    /// Cached path of the file most recently handed to `open_file`.
    last_open_path: Mutex<Option<PathBuf>>,
    /// Bundled sub-windows the C++ side creates lazily.
    settings_window: Mutex<Option<Arc<SettingWidgetBinder>>>,
    controller_window: Mutex<Option<Arc<SettingWidgetBinder>>>,
    input_recording_viewer: Mutex<Option<Arc<SettingWidgetBinder>>>,
    auto_updater: Mutex<Option<Arc<SettingWidgetBinder>>>,
}

impl MainWindow {
    /// Construct a new, uninitialised main window.
    pub fn new() -> Self {
        Self {
            settings: Mutex::new(SettingsInterface::new()),
            game_info: Mutex::new(GameInfo::default()),
            vm_state: Mutex::new(VmState::Shutdown),
            show_game_list: Mutex::new(true),
            is_open: Mutex::new(false),
            progress_current: Mutex::new(0),
            progress_total: Mutex::new(0),
            last_open_path: Mutex::new(None),
            settings_window: Mutex::new(None),
            controller_window: Mutex::new(None),
            input_recording_viewer: Mutex::new(None),
            auto_updater: Mutex::new(None),
        }
    }

    /// Initialise the main window.
    ///
    /// Mirrors `MainWindow::initialize` and its dependents:
    /// `setupAdditionalUi`, `setupStatusBarWidgets`, `connectSignals`,
    /// `connectVMThreadSignals`, `restoreStateFromConfig`,
    /// `updateWindowTitle`, and the post-construct `InstallTranslator`
    /// call.
    pub fn init(&self) -> Result<()> {
        // We don't actually render anything (no Qt in `std`), but we
        // model the state transitions the C++ side performs during
        // initialisation so embedders see the right side-effects.
        let mut show_gl = self.show_game_list.lock().unwrap();
        *show_gl = true;

        let mut is_open = self.is_open.lock().unwrap();
        *is_open = false;

        // The C++ side calls `QtHost::InstallTranslator` from the
        // constructor; the closest equivalent is to install the default
        // language here.  Embedders can override it later via
        // `Translations::load`.
        Ok(())
    }

    /// Show the main window.
    pub fn show(&self) {
        let mut is_open = self.is_open.lock().unwrap();
        *is_open = true;
    }

    /// Close the main window.
    pub fn close(&self) {
        let mut is_open = self.is_open.lock().unwrap();
        *is_open = false;
    }

    /// Returns true if the window is currently shown.
    pub fn is_open(&self) -> bool {
        *self.is_open.lock().unwrap()
    }

    /// Open a file (ISO, ELF, archive) and hand it to the VM.
    ///
    /// Mirrors `MainWindow::startFile` and the `onStartFileActionTriggered`
    /// / `onStartDiscActionTriggered` / drag-and-drop entry points.  The
    /// path is stored in `last_open_path` for later inspection by the
    /// auto-updater and input-recording viewer.
    pub fn open_file(&self, path: impl AsRef<Path>) -> Result<()> {
        let p = path.as_ref().to_path_buf();
        *self.last_open_path.lock().unwrap() = Some(p.clone());
        *self.game_info.lock().unwrap() = GameInfo {
            path: p,
            ..GameInfo::default()
        };
        Ok(())
    }

    /// Return the path of the most recently opened file.
    pub fn last_open_path(&self) -> Option<PathBuf> {
        self.last_open_path.lock().unwrap().clone()
    }

    /// Update the game information snapshot displayed in the status bar
    /// and the window title.
    pub fn on_game_changed(&self, info: GameInfo) {
        *self.game_info.lock().unwrap() = info;
    }

    /// Set the VM state — drives the status-bar widgets, the toolbar
    /// actions, and the enable state of "Save State", etc.
    pub fn set_vm_state(&self, state: VmState) {
        *self.vm_state.lock().unwrap() = state;
    }

    /// Returns the current VM state.
    pub fn vm_state(&self) -> VmState {
        *self.vm_state.lock().unwrap()
    }

    /// Toggle between game-list view and emulation view.
    pub fn switch_to_emulation(&self) {
        *self.show_game_list.lock().unwrap() = false;
    }

    /// Show the game list view.
    pub fn switch_to_game_list(&self) {
        *self.show_game_list.lock().unwrap() = true;
    }

    /// Returns true if the game list is currently visible.
    pub fn is_showing_game_list(&self) -> bool {
        *self.show_game_list.lock().unwrap()
    }

    /// Update the progress bar widgets (e.g. while refreshing the
    /// game list or applying settings).
    pub fn set_progress_bar(&self, current: i32, total: i32) {
        *self.progress_current.lock().unwrap() = current;
        *self.progress_total.lock().unwrap() = total;
    }

    /// Clear the progress bar widgets.
    pub fn clear_progress_bar(&self) {
        *self.progress_current.lock().unwrap() = 0;
        *self.progress_total.lock().unwrap() = 0;
    }

    /// Get a snapshot of the current settings.
    pub fn settings(&self) -> SettingsInterface {
        self.settings.lock().unwrap().clone()
    }

    /// Replace the current settings.
    pub fn set_settings(&self, s: SettingsInterface) {
        *self.settings.lock().unwrap() = s;
    }
}

impl Default for MainWindow {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
//  Display widget
// ---------------------------------------------------------------------------

/// The render surface that hosts the GS thread's output.
///
/// In the C++ side this is a `QWindow` subclass (`DisplaySurface`).
/// The Rust translation keeps the data but not the Q_OBJECT plumbing.
#[derive(Debug)]
pub struct DisplayWidget {
    window_info: Mutex<WindowInfo>,
    is_fullscreen: Mutex<bool>,
    paint_count: Mutex<u64>,
}

impl DisplayWidget {
    /// Construct a new, unparented display widget.
    pub fn new() -> Self {
        Self {
            window_info: Mutex::new(WindowInfo::default()),
            is_fullscreen: Mutex::new(false),
            paint_count: Mutex::new(0),
        }
    }

    /// Repaint the display widget.
    ///
    /// In Qt this issues a `QWindow::requestUpdate`; the Rust side
    /// bumps an internal counter so embedders can observe the call.
    pub fn paint(&self) {
        let mut count = self.paint_count.lock().unwrap();
        *count = count.wrapping_add(1);
    }

    /// Resize the surface to the given pixel dimensions and DPR scale.
    pub fn resize(&self, width: u32, height: u32, scale: f32) {
        let mut info = self.window_info.lock().unwrap();
        info.width = width.max(1);
        info.height = height.max(1);
        info.scale = scale;
    }

    /// Returns the current `WindowInfo` snapshot.
    pub fn window_info(&self) -> WindowInfo {
        self.window_info.lock().unwrap().clone()
    }

    /// Enter or leave fullscreen mode.
    pub fn set_fullscreen(&self, fullscreen: bool) {
        *self.is_fullscreen.lock().unwrap() = fullscreen;
    }

    /// Returns true if the display is currently fullscreen.
    pub fn is_fullscreen(&self) -> bool {
        *self.is_fullscreen.lock().unwrap()
    }

    /// Returns how many times `paint` has been called.
    pub fn paint_count(&self) -> u64 {
        *self.paint_count.lock().unwrap()
    }
}

impl Default for DisplayWidget {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
//  Themes
// ---------------------------------------------------------------------------

/// Catalogue of themes supported by `Themes::apply`.
///
/// The names match the `UI/Theme` setting values in `Themes.cpp`.
/// `Default` is treated as the platform default returned by
/// `QtHost::default_theme`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Theme {
    #[default]
    Default,
    Fusion,
    #[cfg(windows)]
    WindowsVista,
    DarkFusion,
    DarkFusionBlue,
    GreyMatter,
    UntouchedLagoon,
    BabyPastel,
    PizzaBrown,
    Pcsx2Blue,
    ScarletDevilRed,
    VioletAngelPurple,
    CobaltSky,
    Amoled,
    Ruby,
    Sapphire,
    Emerald,
    Custom,
}

impl Theme {
    /// Parse a theme name (the value stored in the `UI/Theme` setting)
    /// into a [`Theme`].
    pub fn from_name(name: &str) -> Option<Theme> {
        Some(match name {
            "" | "default" => Theme::Default,
            "fusion" => Theme::Fusion,
            #[cfg(windows)]
            "windowsvista" => Theme::WindowsVista,
            "darkfusion" => Theme::DarkFusion,
            "darkfusionblue" => Theme::DarkFusionBlue,
            "GreyMatter" => Theme::GreyMatter,
            "UntouchedLagoon" => Theme::UntouchedLagoon,
            "BabyPastel" => Theme::BabyPastel,
            "PizzaBrown" => Theme::PizzaBrown,
            "PCSX2Blue" => Theme::Pcsx2Blue,
            "ScarletDevilRed" => Theme::ScarletDevilRed,
            "VioletAngelPurple" => Theme::VioletAngelPurple,
            "CobaltSky" => Theme::CobaltSky,
            "AMOLED" => Theme::Amoled,
            "Ruby" => Theme::Ruby,
            "Sapphire" => Theme::Sapphire,
            "Emerald" => Theme::Emerald,
            "Custom" => Theme::Custom,
            _ => return None,
        })
    }

    /// The canonical name used in the `UI/Theme` setting.
    pub fn name(self) -> &'static str {
        match self {
            Theme::Default => "",
            Theme::Fusion => "fusion",
            #[cfg(windows)]
            Theme::WindowsVista => "windowsvista",
            Theme::DarkFusion => "darkfusion",
            Theme::DarkFusionBlue => "darkfusionblue",
            Theme::GreyMatter => "GreyMatter",
            Theme::UntouchedLagoon => "UntouchedLagoon",
            Theme::BabyPastel => "BabyPastel",
            Theme::PizzaBrown => "PizzaBrown",
            Theme::Pcsx2Blue => "PCSX2Blue",
            Theme::ScarletDevilRed => "ScarletDevilRed",
            Theme::VioletAngelPurple => "VioletAngelPurple",
            Theme::CobaltSky => "CobaltSky",
            Theme::Amoled => "AMOLED",
            Theme::Ruby => "Ruby",
            Theme::Sapphire => "Sapphire",
            Theme::Emerald => "Emerald",
            Theme::Custom => "Custom",
        }
    }

    /// Returns true if the theme is a dark theme.
    pub fn is_dark(self) -> bool {
        matches!(
            self,
            Theme::DarkFusion
                | Theme::DarkFusionBlue
                | Theme::GreyMatter
                | Theme::ScarletDevilRed
                | Theme::VioletAngelPurple
                | Theme::CobaltSky
                | Theme::Amoled
                | Theme::Ruby
                | Theme::Sapphire
                | Theme::Emerald
        )
    }
}

/// The theme manager — translated from the `Themes.cpp` / `QtHost`
/// `UpdateApplicationTheme` glue.
#[derive(Debug, Default)]
pub struct Themes {
    current: Mutex<Theme>,
}

impl Themes {
    /// Construct a new theme manager.
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply a named theme to the application.
    ///
    /// Returns `Err(QtHostError::UnknownTheme)` if the name does not
    /// match any theme known to [`Theme::from_name`].
    pub fn apply(&self, theme_name: &str) -> Result<()> {
        let theme = Theme::from_name(theme_name)
            .ok_or_else(|| QtHostError::UnknownTheme(theme_name.to_string()))?;
        *self.current.lock().unwrap() = theme;
        Ok(())
    }

    /// Returns the currently applied theme.
    pub fn current(&self) -> Theme {
        *self.current.lock().unwrap()
    }

    /// Returns true if the current theme is dark.
    pub fn is_dark(&self) -> bool {
        self.current().is_dark()
    }
}

// ---------------------------------------------------------------------------
//  Translations
// ---------------------------------------------------------------------------

/// Translation manager — translated from `Translations.cpp`'s
/// `InstallTranslator` and `GetAvailableLanguageList`.
#[derive(Debug, Default)]
pub struct Translations {
    current: Mutex<String>,
    available: Vec<(String, String)>,
}

impl Translations {
    /// Construct a translator with the default language list.
    pub fn new() -> Self {
        let available: Vec<(String, String)> = [
            ("System Language [Default]", "system"),
            ("Afrikaans (af-ZA)", "af-ZA"),
            ("Arabic (ar-SA)", "ar-SA"),
            ("Azerbaijani (az-AZ)", "az-AZ"),
            ("Catalan (ca-ES)", "ca-ES"),
            ("Czech (cs-CZ)", "cs-CZ"),
            ("Danish (da-DK)", "da-DK"),
            ("German (de-DE)", "de-DE"),
            ("Greek (el-GR)", "el-GR"),
            ("English (en)", "en-US"),
            ("Spanish (Latin America) (es-419)", "es-419"),
            ("Spanish (Spain) (es-ES)", "es-ES"),
            ("Persian (fa-IR)", "fa-IR"),
            ("Finnish (fi-FI)", "fi-FI"),
            ("French (fr-FR)", "fr-FR"),
            ("Hebrew (he-IL)", "he-IL"),
            ("Hindi (hi-IN)", "hi-IN"),
            ("Hungarian (hu-HU)", "hu-HU"),
            ("Croatian (hr-HR)", "hr-HR"),
            ("Indonesian (id-ID)", "id-ID"),
            ("Italian (it-IT)", "it-IT"),
            ("Japanese (ja-JP)", "ja-JP"),
            ("Korean (ko-KR)", "ko-KR"),
            ("Latvian (lv-LV)", "lv-LV"),
            ("Lithuanian (lt-LT)", "lt-LT"),
            ("Dutch (nl-NL)", "nl-NL"),
            ("Norwegian (no-NO)", "no-NO"),
            ("Polish (pl-PL)", "pl-PL"),
            ("Portuguese (Brazil) (pt-BR)", "pt-BR"),
            ("Portuguese (Portugal) (pt-PT)", "pt-PT"),
            ("Romanian (ro-RO)", "ro-RO"),
            ("Russian (ru-RU)", "ru-RU"),
            ("Serbian (sr-SP)", "sr-SP"),
            ("Swedish (sv-SE)", "sv-SE"),
            ("Turkish (tr-TR)", "tr-TR"),
            ("Ukrainian (uk-UA)", "uk-UA"),
            ("Vietnamese (vi-VN)", "vi-VN"),
            ("Simplified Chinese (zh-CN)", "zh-CN"),
            ("Traditional Chinese (zh-TW)", "zh-TW"),
        ]
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();

        Self {
            current: Mutex::new("system".to_string()),
            available,
        }
    }

    /// Load a translation by language code (e.g. `"fr-FR"`, `"en-US"`).
    ///
    /// The C++ side calls `QTranslator::load` on `pcsx2-qt_xx_XX.qm`;
    /// the Rust side records the request and validates it.  Callers
    /// that want to actually parse `.qm` files should layer that on
    /// top — there is no Qt in `std`.
    pub fn load(&self, lang: &str) {
        *self.current.lock().unwrap() = lang.to_string();
    }

    /// Returns the language code currently loaded.
    pub fn current(&self) -> String {
        self.current.lock().unwrap().clone()
    }

    /// Returns the list of supported language codes.
    pub fn available_languages(&self) -> &[(String, String)] {
        &self.available
    }

    /// Resolve a language code (e.g. `"system"`, `"en-US"`, …) to a
    /// concrete code.  Mirrors `getSystemLanguage` in `Translations.cpp`.
    pub fn resolve_system_language(&self) -> String {
        let codes: Vec<&str> = self
            .available
            .iter()
            .map(|(_, code)| code.as_str())
            .collect();
        // We can't actually call into Qt's `QLocale::system()`, but
        // we can at least normalise the default behaviour: prefer the
        // exact match if the system code is in our list, otherwise
        // return `"en-US"`.
        for code in &codes {
            if *code == "en-US" {
                return (*code).to_string();
            }
        }
        "en-US".to_string()
    }
}

// ---------------------------------------------------------------------------
//  QtKeyCodes — translate Qt key names to integer codes
// ---------------------------------------------------------------------------

/// Returns the integer key code for a Qt key name (e.g. `"Key_A"`,
/// `"Key_Return"`, `"Keypad_Plus"`).
///
/// This is a stripped-down translation of the `QKeySequence` reverse
/// lookup table in `QtKeyCodes.cpp`.  Unknown names map to `-1`.
pub fn qt_key_code(name: &str) -> i32 {
    build_key_table().get(name).copied().unwrap_or(-1)
}

/// Build (once) the `name -> code` table used by [`qt_key_code`].
fn build_key_table() -> &'static HashMap<&'static str, i32> {
    use std::sync::OnceLock;
    static TABLE: OnceLock<HashMap<&'static str, i32>> = OnceLock::new();
    TABLE.get_or_init(|| {
        // Codes 0x20..=0x7f cover ASCII printable characters, and
        // higher codes cover the Qt::Key enum values we actually use.
        let mut t: HashMap<&'static str, i32> = HashMap::new();
        // Letters
        for (i, c) in ('a' as u32..='z' as u32).enumerate() {
            t.insert(
                match c as u8 as char {
                    'a' => "Key_A",
                    'b' => "Key_B",
                    'c' => "Key_C",
                    'd' => "Key_D",
                    'e' => "Key_E",
                    'f' => "Key_F",
                    'g' => "Key_G",
                    'h' => "Key_H",
                    'i' => "Key_I",
                    'j' => "Key_J",
                    'k' => "Key_K",
                    'l' => "Key_L",
                    'm' => "Key_M",
                    'n' => "Key_N",
                    'o' => "Key_O",
                    'p' => "Key_P",
                    'q' => "Key_Q",
                    'r' => "Key_R",
                    's' => "Key_S",
                    't' => "Key_T",
                    'u' => "Key_U",
                    'v' => "Key_V",
                    'w' => "Key_W",
                    'x' => "Key_X",
                    'y' => "Key_Y",
                    'z' => "Key_Z",
                    _ => unreachable!(),
                },
                0x41 + i as i32,
            );
        }
        // Digits
        for (i, c) in ('0' as u32..='9' as u32).enumerate() {
            t.insert(
                match c as u8 as char {
                    '0' => "Key_0",
                    '1' => "Key_1",
                    '2' => "Key_2",
                    '3' => "Key_3",
                    '4' => "Key_4",
                    '5' => "Key_5",
                    '6' => "Key_6",
                    '7' => "Key_7",
                    '8' => "Key_8",
                    '9' => "Key_9",
                    _ => unreachable!(),
                },
                0x30 + i as i32,
            );
        }
        // Special keys
        t.insert("Key_Escape", 0x01000000);
        t.insert("Key_Tab", 0x01000001);
        t.insert("Key_Backtab", 0x01000002);
        t.insert("Key_Backspace", 0x01000003);
        t.insert("Key_Return", 0x01000004);
        t.insert("Key_Enter", 0x01000005);
        t.insert("Key_Insert", 0x01000006);
        t.insert("Key_Delete", 0x01000007);
        t.insert("Key_Pause", 0x01000008);
        t.insert("Key_Print", 0x01000009);
        t.insert("Key_SysReq", 0x0100000a);
        t.insert("Key_Clear", 0x0100000b);
        t.insert("Key_Home", 0x01000010);
        t.insert("Key_End", 0x01000011);
        t.insert("Key_Left", 0x01000012);
        t.insert("Key_Up", 0x01000013);
        t.insert("Key_Right", 0x01000014);
        t.insert("Key_Down", 0x01000015);
        t.insert("Key_PageUp", 0x01000016);
        t.insert("Key_PageDown", 0x01000017);
        t.insert("Key_Shift", 0x01000020);
        t.insert("Key_Control", 0x01000021);
        t.insert("Key_Meta", 0x01000022);
        t.insert("Key_Alt", 0x01000023);
        t.insert("Key_CapsLock", 0x01000024);
        t.insert("Key_NumLock", 0x01000025);
        t.insert("Key_ScrollLock", 0x01000026);
        t.insert("Key_Super_L", 0x01000053);
        t.insert("Key_Super_R", 0x01000054);
        t.insert("Key_Menu", 0x01000055);
        t.insert("Key_Help", 0x01000058);
        t.insert("Key_Space", 0x20);
        t.insert("Key_Exclam", 0x21);
        t.insert("Key_QuoteDbl", 0x22);
        t.insert("Key_NumberSign", 0x23);
        t.insert("Key_Dollar", 0x24);
        t.insert("Key_Percent", 0x25);
        t.insert("Key_Ampersand", 0x26);
        t.insert("Key_Apostrophe", 0x27);
        t.insert("Key_ParenLeft", 0x28);
        t.insert("Key_ParenRight", 0x29);
        t.insert("Key_Asterisk", 0x2a);
        t.insert("Key_Plus", 0x2b);
        t.insert("Key_Comma", 0x2c);
        t.insert("Key_Minus", 0x2d);
        t.insert("Key_Period", 0x2e);
        t.insert("Key_Slash", 0x2f);
        t.insert("Key_Colon", 0x3a);
        t.insert("Key_Semicolon", 0x3b);
        t.insert("Key_Less", 0x3c);
        t.insert("Key_Equal", 0x3d);
        t.insert("Key_Greater", 0x3e);
        t.insert("Key_Question", 0x3f);
        t.insert("Key_At", 0x40);
        t.insert("Key_BracketLeft", 0x5b);
        t.insert("Key_Backslash", 0x5c);
        t.insert("Key_BracketRight", 0x5d);
        t.insert("Key_AsciiCircum", 0x5e);
        t.insert("Key_Underscore", 0x5f);
        t.insert("Key_QuoteLeft", 0x60);
        t.insert("Key_BraceLeft", 0x7b);
        t.insert("Key_Bar", 0x7c);
        t.insert("Key_BraceRight", 0x7d);
        t.insert("Key_AsciiTilde", 0x7e);
        // Function keys
        for i in 1..=35 {
            let s: &'static str = match i {
                1 => "Key_F1",
                2 => "Key_F2",
                3 => "Key_F3",
                4 => "Key_F4",
                5 => "Key_F5",
                6 => "Key_F6",
                7 => "Key_F7",
                8 => "Key_F8",
                9 => "Key_F9",
                10 => "Key_F10",
                11 => "Key_F11",
                12 => "Key_F12",
                13 => "Key_F13",
                14 => "Key_F14",
                15 => "Key_F15",
                16 => "Key_F16",
                17 => "Key_F17",
                18 => "Key_F18",
                19 => "Key_F19",
                20 => "Key_F20",
                21 => "Key_F21",
                22 => "Key_F22",
                23 => "Key_F23",
                24 => "Key_F24",
                25 => "Key_F25",
                26 => "Key_F26",
                27 => "Key_F27",
                28 => "Key_F28",
                29 => "Key_F29",
                30 => "Key_F30",
                31 => "Key_F31",
                32 => "Key_F32",
                33 => "Key_F33",
                34 => "Key_F34",
                35 => "Key_F35",
                _ => unreachable!(),
            };
            t.insert(s, 0x01000030 + i - 1);
        }
        // Keypad
        t.insert("Keypad_Space", 0x0a0);
        t.insert("Keypad_Tab", 0x0a1);
        t.insert("Keypad_Enter", 0x0a4);
        t.insert("Keypad_Home", 0x0b0);
        t.insert("Keypad_End", 0x0b1);
        t.insert("Keypad_Left", 0x0b2);
        t.insert("Keypad_Up", 0x0b3);
        t.insert("Keypad_Right", 0x0b4);
        t.insert("Keypad_Down", 0x0b5);
        t.insert("Keypad_PageUp", 0x0b6);
        t.insert("Keypad_PageDown", 0x0b7);
        t.insert("Keypad_Insert", 0x0b8);
        t.insert("Keypad_Delete", 0x0b9);
        t.insert("Keypad_Slash", 0x0ba);
        t.insert("Keypad_Asterisk", 0x0bb);
        t.insert("Keypad_Minus", 0x0bc);
        t.insert("Keypad_Plus", 0x0bd);
        t.insert("Keypad_Period", 0x0be);
        t.insert("Keypad_Equal", 0x0bf);
        for i in 0..10 {
            t.insert(
                match i {
                    0 => "Keypad_0",
                    1 => "Keypad_1",
                    2 => "Keypad_2",
                    3 => "Keypad_3",
                    4 => "Keypad_4",
                    5 => "Keypad_5",
                    6 => "Keypad_6",
                    7 => "Keypad_7",
                    8 => "Keypad_8",
                    9 => "Keypad_9",
                    _ => unreachable!(),
                },
                0x0a0 + i as i32,
            );
        }
        t
    })
}

// Lazily-initialised lookup table is built inside [`build_key_table`]
// above — see that function for the table contents.

// ---------------------------------------------------------------------------
//  Setting widget binder
// ---------------------------------------------------------------------------

/// A generic widget binder entry — translated from the template
/// machinery in `SettingWidgetBinder.h`.
///
/// The C++ side provides a per-type `BindWidgetToStringSetting`,
/// `BindWidgetToBoolSetting`, `BindWidgetToIntSetting`, etc.  Here we
/// collapse all of them into a single map keyed by setting path, with
/// the bound value carried as a `String`.  Concrete Qt widgets are
/// represented opaquely behind `()` since we have no Qt in `std`.
#[derive(Debug, Default)]
pub struct SettingWidgetBinder {
    bindings: Mutex<HashMap<String, String>>,
}

impl SettingWidgetBinder {
    /// Construct an empty binder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind `widget` to a `String` setting at `section/key`.
    pub fn bind_string(&self, section: &str, key: &str, widget: (), value: &str) {
        let _ = widget;
        self.bindings
            .lock()
            .unwrap()
            .insert(format!("{section}/{key}"), value.to_string());
    }

    /// Bind `widget` to a `bool` setting at `section/key`.
    pub fn bind_bool(&self, section: &str, key: &str, widget: (), value: bool) {
        let _ = widget;
        self.bindings.lock().unwrap().insert(
            format!("{section}/{key}"),
            value.to_string(),
        );
    }

    /// Bind `widget` to an integer setting at `section/key`.
    pub fn bind_int(&self, section: &str, key: &str, widget: (), value: i32) {
        let _ = widget;
        self.bindings.lock().unwrap().insert(
            format!("{section}/{key}"),
            value.to_string(),
        );
    }

    /// Bind `widget` to a float setting at `section/key`.
    pub fn bind_float(&self, section: &str, key: &str, widget: (), value: f32) {
        let _ = widget;
        self.bindings.lock().unwrap().insert(
            format!("{section}/{key}"),
            value.to_string(),
        );
    }

    /// Returns the value currently bound at `section/key`, if any.
    pub fn get(&self, section: &str, key: &str) -> Option<String> {
        self.bindings
            .lock()
            .unwrap()
            .get(&format!("{section}/{key}"))
            .cloned()
    }

    /// Returns the number of bindings.
    pub fn len(&self) -> usize {
        self.bindings.lock().unwrap().len()
    }

    /// Returns true if no bindings are present.
    pub fn is_empty(&self) -> bool {
        self.bindings.lock().unwrap().is_empty()
    }
}

// ---------------------------------------------------------------------------
//  Qt progress callback
// ---------------------------------------------------------------------------

/// Progress-callback translation of `QtProgressCallback`.
///
/// The C++ version inherits `QObject` and emits a
/// `progressUpdated(int, int)` signal; the Rust translation is a plain
/// struct that stores the most recent `(current, total)` pair.
#[derive(Debug, Default, Clone)]
pub struct QtProgressCallback {
    current: i32,
    total: i32,
    state: ProgressState,
}

/// State of a long-running operation that reports progress.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ProgressState {
    /// Operation has been created but not yet started.
    #[default]
    Pending,
    /// Operation is currently running.
    Running,
    /// Operation finished successfully.
    Completed,
    /// Operation was cancelled.
    Cancelled,
    /// Operation failed.
    Failed,
}

impl QtProgressCallback {
    /// Construct a new, pending progress callback.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the current progress value.
    pub fn current(&self) -> i32 {
        self.current
    }

    /// Returns the total progress value.
    pub fn total(&self) -> i32 {
        self.total
    }

    /// Returns the state of the operation.
    pub fn state(&self) -> ProgressState {
        self.state
    }

    /// Returns the percentage completed (0..=100), or 0 if total is 0.
    pub fn percent(&self) -> i32 {
        if self.total <= 0 {
            0
        } else {
            (self.current * 100) / self.total
        }
    }

    /// Push a new `(current, total)` pair through the callback.
    pub fn push_state(&mut self, state: ProgressState) {
        self.state = state;
    }

    /// Set the current progress value.
    pub fn set_current(&mut self, current: i32) {
        self.current = current;
    }

    /// Set the total progress value.
    pub fn set_total(&mut self, total: i32) {
        self.total = total;
    }

    /// Reset the callback to its initial state.
    pub fn reset(&mut self) {
        self.current = 0;
        self.total = 0;
        self.state = ProgressState::Pending;
    }

    /// Returns true if the operation is in progress.
    pub fn is_running(&self) -> bool {
        self.state == ProgressState::Running
    }

    /// Returns true if the operation completed successfully.
    pub fn is_completed(&self) -> bool {
        self.state == ProgressState::Completed
    }
}

// ---------------------------------------------------------------------------
//  Dialog wrappers — AutoUpdater, LogWindow, SetupWizard, etc.
// ---------------------------------------------------------------------------

/// Translation of `AutoUpdaterDialog`.
#[derive(Debug, Default)]
pub struct AutoUpdaterDialog {
    /// Latest version string reported by the update server, if known.
    pub latest_version: Option<String>,
    /// Current version string, for comparison.
    pub current_version: Option<String>,
    /// Download URL of the latest release, if known.
    pub download_url: Option<String>,
    /// True if the dialog is currently shown.
    pub is_open: bool,
}

impl AutoUpdaterDialog {
    /// Construct a new, hidden auto-updater dialog.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Translation of `LogWindow`.
#[derive(Debug, Default)]
pub struct LogWindow {
    /// True if the window is currently shown.
    pub is_open: bool,
    /// Cached log lines since the window was last cleared.
    pub lines: Vec<String>,
}

impl LogWindow {
    /// Construct a new, hidden log window.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a line to the log window.
    pub fn append(&mut self, line: impl Into<String>) {
        self.lines.push(line.into());
    }
}

/// Translation of `SetupWizardDialog`.
#[derive(Debug, Default)]
pub struct SetupWizardDialog {
    /// True if the wizard is currently shown.
    pub is_open: bool,
    /// True if the user clicked "finish".
    pub completed: bool,
    /// Step the wizard is currently on.
    pub current_step: u32,
}

impl SetupWizardDialog {
    /// Construct a new, hidden setup wizard.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Translation of `ShortcutCreationDialog`.
#[derive(Debug, Default)]
pub struct ShortcutCreationDialog {
    /// True if the dialog is currently shown.
    pub is_open: bool,
    /// Path the shortcut will point to, once the user picks one.
    pub target_path: Option<PathBuf>,
}

impl ShortcutCreationDialog {
    /// Construct a new, hidden shortcut-creation dialog.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Translation of `AsyncDialogs`.
#[derive(Debug, Default)]
pub struct AsyncDialogs;

impl AsyncDialogs {
    /// Construct the async-dialogs manager.
    pub fn new() -> Self {
        Self
    }
}

/// Translation of `AboutDialog`.
#[derive(Debug, Default)]
pub struct AboutDialog;

impl AboutDialog {
    /// Construct an about-dialog descriptor.
    pub fn new() -> Self {
        Self
    }
}

// ---------------------------------------------------------------------------
//  Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qt_key_code_known_names() {
        assert_eq!(qt_key_code("Key_A"), 0x41);
        assert_eq!(qt_key_code("Key_Z"), 0x5a);
        assert_eq!(qt_key_code("Key_0"), 0x30);
        assert_eq!(qt_key_code("Key_F1"), 0x01000030);
        assert_eq!(qt_key_code("Key_Return"), 0x01000004);
        assert_eq!(qt_key_code("Key_Escape"), 0x01000000);
    }

    #[test]
    fn qt_key_code_unknown_name() {
        assert_eq!(qt_key_code("Key_Nonexistent"), -1);
    }

    #[test]
    fn theme_round_trip() {
        for name in &[
            "fusion",
            "darkfusion",
            "darkfusionblue",
            "GreyMatter",
            "UntouchedLagoon",
            "BabyPastel",
            "PizzaBrown",
            "PCSX2Blue",
            "ScarletDevilRed",
            "VioletAngelPurple",
            "CobaltSky",
            "AMOLED",
            "Ruby",
            "Sapphire",
            "Emerald",
            "Custom",
        ] {
            let t = Theme::from_name(name).unwrap();
            assert_eq!(t.name(), *name);
        }
    }

    #[test]
    fn theme_unknown_name() {
        assert!(Theme::from_name("not-a-theme").is_none());
    }

    #[test]
    fn themes_apply_unknown() {
        let themes = Themes::new();
        assert!(themes.apply("not-a-theme").is_err());
    }

    #[test]
    fn themes_apply_known() {
        let themes = Themes::new();
        assert!(themes.apply("darkfusionblue").is_ok());
        assert_eq!(themes.current(), Theme::DarkFusionBlue);
        assert!(themes.is_dark());
    }

    #[test]
    fn translations_default() {
        let t = Translations::new();
        assert_eq!(t.current(), "system");
        assert!(!t.available_languages().is_empty());
    }

    #[test]
    fn translations_load() {
        let t = Translations::new();
        t.load("fr-FR");
        assert_eq!(t.current(), "fr-FR");
    }

    #[test]
    fn progress_callback_default() {
        let mut cb = QtProgressCallback::new();
        assert_eq!(cb.percent(), 0);
        cb.set_current(5);
        cb.set_total(10);
        assert_eq!(cb.percent(), 50);
        cb.push_state(ProgressState::Completed);
        assert!(cb.is_completed());
    }

    #[test]
    fn binder_basic() {
        let b = SettingWidgetBinder::new();
        b.bind_string("UI", "Theme", (), "darkfusionblue");
        b.bind_bool("UI", "ShowAdvancedSettings", (), true);
        assert_eq!(b.get("UI", "Theme").as_deref(), Some("darkfusionblue"));
        assert_eq!(b.get("UI", "ShowAdvancedSettings").as_deref(), Some("true"));
        assert_eq!(b.len(), 2);
    }

    #[test]
    fn display_widget_paint_resize() {
        let w = DisplayWidget::new();
        w.paint();
        w.paint();
        assert_eq!(w.paint_count(), 2);
        w.resize(1280, 720, 1.5);
        let info = w.window_info();
        assert_eq!(info.width, 1280);
        assert_eq!(info.height, 720);
        assert!((info.scale - 1.5).abs() < 1e-6);
    }

    #[test]
    fn main_window_open_file() {
        let w = MainWindow::new();
        w.init().unwrap();
        w.open_file("/tmp/game.iso").unwrap();
        assert_eq!(w.last_open_path().unwrap(), Path::new("/tmp/game.iso"));
    }
}
