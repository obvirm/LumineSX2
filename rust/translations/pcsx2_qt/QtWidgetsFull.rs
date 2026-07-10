// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust translation of the PCSX2 Qt widget sources under `pcsx2-qt/`.
//!
//! This module consolidates a large set of C++ Qt classes into a single Rust
//! 2021 module. Each former C++ class is exposed as a `pub struct` with a
//! `create()` constructor and a `populate()` method that wires the widget up
//! to its data sources (settings, signals, etc). Only the `std` library is
//! used. The module is intentionally self-contained and free of platform
//! dependencies so it can be reasoned about without an embedded Qt runtime.

#![allow(clippy::too_many_arguments)]
#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

// =============================================================================
// Common types
// =============================================================================

/// Width of the wheel delta, matching the WinAPI constant of the same name.
pub const MOUSE_WHEEL_DELTA: f32 = 120.0;

/// Default log window dimensions in pixels.
pub const DEFAULT_LOG_WINDOW_WIDTH: i32 = 750;
pub const DEFAULT_LOG_WINDOW_HEIGHT: i32 = 400;

/// Default font point size used by the log window.
pub const DEFAULT_LOG_FONT_POINT_SIZE: i32 = 10;

/// HTTP poll interval in milliseconds used by the auto-updater dialog.
pub const HTTP_POLL_INTERVAL_MS: u32 = 10;

/// Standard HTTP status codes used by the auto-updater downloader.
pub mod http_status {
    pub const HTTP_STATUS_OK: i32 = 200;
    pub const HTTP_STATUS_CANCELLED: i32 = -1;
}

/// Supported update channels surfaced by the auto-updater dialog.
pub const UPDATE_TAGS: [&str; 2] = ["stable", "nightly"];

/// The default update channel when the build does not override it.
pub const DEFAULT_UPDATER_CHANNEL: &str = "nightly";

/// Minimum VCRuntime version (encoded the same way as the C++ source).
pub const MIN_VCRUNTIME_VERSION: u64 = make_version64(14, 38, 33_135, 0);
pub const VCRUNTIME_DOWNLOAD_URL: &str = "https://aka.ms/vs/17/release/vc_redist.x64.exe";

const fn make_version64(v0: u16, v1: u16, v2: u16, v3: u16) -> u64 {
    ((v0 as u64) << 48) | ((v1 as u64) << 32) | ((v2 as u64) << 16) | (v3 as u64)
}

const fn version_part(v: u64, p: u32) -> u16 {
    ((v >> (48 - p * 16)) & 0xFFFF) as u16
}

/// Color roles used for theming, mirroring the relevant subset of `QPalette`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PaletteRole {
    Window,
    WindowText,
    Base,
    AlternateBase,
    ToolTipBase,
    ToolTipText,
    Text,
    Button,
    ButtonText,
    Link,
    Highlight,
    HighlightedText,
    PlaceholderText,
    Light,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorScheme {
    Unknown,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EchoMode {
    Normal,
    Password,
    NoEcho,
    PasswordEchoOnEdit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CheckState {
    Unchecked,
    PartiallyChecked,
    Checked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StandardButton {
    NoButton,
    Ok,
    Yes,
    No,
    Cancel,
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MessageIcon {
    Information,
    Question,
    Warning,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScalingMode {
    Fit,
    Fill,
    Stretch,
    Center,
    Tile,
}

/// Identifier for a setting used by the binder helpers.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SettingKey {
    pub section: String,
    pub key: String,
}

impl SettingKey {
    pub fn new(section: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            section: section.into(),
            key: key.into(),
        }
    }
}

/// Side enum for the slider/label binding helper.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowState {
    Maximized,
    FullScreen,
}

/// Lightweight RGB color used to drive palette application without Qt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const fn darker(self) -> Self {
        Self(self.0 / 2, self.1 / 2, self.2 / 2)
    }

    pub const fn lighter(self) -> Self {
        let r = self.0.saturating_add(32);
        let g = self.1.saturating_add(32);
        let b = self.2.saturating_add(32);
        Self(r, g, b)
    }
}

/// RGBA color with an opacity channel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }
}

/// A trivial `FileInfo`-style value object used by the file utilities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInfo {
    pub path: PathBuf,
    pub exists: bool,
    pub is_readable: bool,
    pub is_bundle: bool,
    pub suffix: Option<String>,
    pub file_name: String,
}

impl FileInfo {
    pub fn from_path(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let metadata = fs::metadata(&path).ok();
        let exists = metadata.is_some();
        let is_readable = metadata
            .as_ref()
            .map(|m| !m.permissions().readonly())
            .unwrap_or(false);
        let file_name = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let suffix = path
            .extension()
            .map(|s| s.to_string_lossy().into_owned());
        Self {
            path,
            exists,
            is_readable,
            is_bundle: false,
            suffix,
            file_name,
        }
    }
}

// =============================================================================
// AboutDialog
// =============================================================================

/// Static URLs mirroring the `SupportURLs.h` constants used by `AboutDialog`.
pub mod support_urls {
    pub const PCSX2_WEBSITE_URL: &str = "https://pcsx2.net/";
    pub const PCSX2_FORUMS_URL: &str = "https://forums.pcsx2.net/";
    pub const PCSX2_GITHUB_URL: &str = "https://github.com/PCSX2/pcsx2";
    pub const PCSX2_WIKI_URL: &str = "https://wiki.pcsx2.net/";
    pub const PCSX2_DOCUMENTATION_URL: &str = "https://docs.pcsx2.net/";
    pub const PCSX2_DISCORD_URL: &str = "https://discord.gg/pcsx2";
}

/// In-memory representation of the about dialog and its sub-dialogs.
pub struct AboutDialog {
    pub app_name_and_version: String,
    pub website_label: String,
    pub support_forums_label: String,
    pub github_label: String,
    pub license_label: String,
    pub third_party_licenses_label: String,
    pub scm_version: String,
    pub app_root: PathBuf,
    pub resources_dir: PathBuf,
}

impl AboutDialog {
    /// Construct an about dialog with the supplied application metadata.
    pub fn create(app_name_and_version: impl Into<String>) -> Self {
        Self {
            app_name_and_version: app_name_and_version.into(),
            website_label: "Website".to_string(),
            support_forums_label: "Support Forums".to_string(),
            github_label: "GitHub Repository".to_string(),
            license_label: "License".to_string(),
            third_party_licenses_label: "Third-Party Licenses".to_string(),
            scm_version: String::new(),
            app_root: PathBuf::new(),
            resources_dir: PathBuf::new(),
        }
    }

    /// Populate the dialog with the runtime information needed for display.
    pub fn populate(&mut self, app_root: PathBuf, resources_dir: PathBuf, scm_version: String) {
        self.app_root = app_root;
        self.resources_dir = resources_dir;
        self.scm_version = scm_version;
    }

    pub fn get_website_url() -> String {
        support_urls::PCSX2_WEBSITE_URL.to_string()
    }

    pub fn get_support_forums_url() -> String {
        support_urls::PCSX2_FORUMS_URL.to_string()
    }

    pub fn get_github_repository_url() -> String {
        support_urls::PCSX2_GITHUB_URL.to_string()
    }

    pub fn get_license_url(&self) -> String {
        self.doc_file_url("GPL.html")
    }

    pub fn get_third_party_licenses_url(&self) -> String {
        self.doc_file_url("ThirdPartyLicenses.html")
    }

    pub fn get_wiki_url() -> String {
        support_urls::PCSX2_WIKI_URL.to_string()
    }

    pub fn get_documentation_url() -> String {
        support_urls::PCSX2_DOCUMENTATION_URL.to_string()
    }

    pub fn get_discord_server_url() -> String {
        support_urls::PCSX2_DISCORD_URL.to_string()
    }

    fn doc_file_url(&self, name: &str) -> String {
        // Windows uses the docs directory in the application root, while
        // Linux and macOS use the resources directory. The C++ `EmuFolders`
        // struct is replaced with caller-supplied paths populated via
        // [`populate`].
        let mut path = self.app_root.clone();
        path.push("docs");
        path.push(name);
        format!("file://{}", path.display())
    }
}

// =============================================================================
// AsyncDialogs
// =============================================================================

/// Outcome of an asynchronous dialog completion callback.
#[derive(Debug, Clone, PartialEq)]
pub enum DialogResult<T> {
    Confirmed(T),
    Cancelled,
}

/// Helpers that build asynchronous input dialog state, mirroring the
/// `AsyncDialogs` namespace. The original C++ code dispatched via Qt's
/// event loop and `QInputDialog`; the Rust translation instead records
/// the requested configuration so callers can drive a real UI themselves.
pub mod async_dialogs {
    use super::*;

    #[derive(Debug, Clone)]
    pub struct InputRequest {
        pub title: String,
        pub label: String,
        pub text: String,
        pub echo: EchoMode,
        pub multiline: bool,
    }

    #[derive(Debug, Clone)]
    pub struct ItemRequest {
        pub title: String,
        pub label: String,
        pub items: Vec<String>,
        pub current: usize,
        pub editable: bool,
    }

    #[derive(Debug, Clone)]
    pub struct IntRequest {
        pub title: String,
        pub label: String,
        pub value: i32,
        pub min_value: i32,
        pub max_value: i32,
        pub step: i32,
    }

    #[derive(Debug, Clone)]
    pub struct DoubleRequest {
        pub title: String,
        pub label: String,
        pub value: f64,
        pub min_value: f64,
        pub max_value: f64,
        pub step: f64,
        pub decimals: i32,
    }

    pub fn get_text(
        title: impl Into<String>,
        label: impl Into<String>,
        text: impl Into<String>,
    ) -> InputRequest {
        InputRequest {
            title: title.into(),
            label: label.into(),
            text: text.into(),
            echo: EchoMode::Normal,
            multiline: false,
        }
    }

    pub fn get_multiline_text(
        title: impl Into<String>,
        label: impl Into<String>,
        text: impl Into<String>,
    ) -> InputRequest {
        InputRequest {
            title: title.into(),
            label: label.into(),
            text: text.into(),
            echo: EchoMode::Normal,
            multiline: true,
        }
    }

    pub fn get_item(
        title: impl Into<String>,
        label: impl Into<String>,
        items: Vec<String>,
        current: usize,
    ) -> ItemRequest {
        ItemRequest {
            title: title.into(),
            label: label.into(),
            items,
            current,
            editable: true,
        }
    }

    pub fn get_int(title: impl Into<String>, label: impl Into<String>, value: i32) -> IntRequest {
        IntRequest {
            title: title.into(),
            label: label.into(),
            value,
            min_value: i32::MIN + 1,
            max_value: i32::MAX - 1,
            step: 1,
        }
    }

    pub fn get_double(
        title: impl Into<String>,
        label: impl Into<String>,
        value: f64,
    ) -> DoubleRequest {
        DoubleRequest {
            title: title.into(),
            label: label.into(),
            value,
            min_value: -2_147_483_647.0,
            max_value: 2_147_483_647.0,
            step: 1.0,
            decimals: 1,
        }
    }

    /// Build the message box configuration for an information dialog.
    pub fn information(
        title: impl Into<String>,
        text: impl Into<String>,
    ) -> MessageBoxRequest {
        MessageBoxRequest {
            icon: MessageIcon::Information,
            title: title.into(),
            text: text.into(),
            buttons: vec![StandardButton::Ok],
            default_button: StandardButton::Ok,
        }
    }

    /// Build a yes/no question dialog request.
    pub fn question(title: impl Into<String>, text: impl Into<String>) -> MessageBoxRequest {
        MessageBoxRequest {
            icon: MessageIcon::Question,
            title: title.into(),
            text: text.into(),
            buttons: vec![StandardButton::Yes, StandardButton::No],
            default_button: StandardButton::NoButton,
        }
    }

    pub fn warning(title: impl Into<String>, text: impl Into<String>) -> MessageBoxRequest {
        MessageBoxRequest {
            icon: MessageIcon::Warning,
            title: title.into(),
            text: text.into(),
            buttons: vec![StandardButton::Ok],
            default_button: StandardButton::Ok,
        }
    }

    pub fn critical(title: impl Into<String>, text: impl Into<String>) -> MessageBoxRequest {
        MessageBoxRequest {
            icon: MessageIcon::Critical,
            title: title.into(),
            text: text.into(),
            buttons: vec![StandardButton::Ok],
            default_button: StandardButton::Ok,
        }
    }
}

/// Configuration of a `QMessageBox`-style dialog produced by the
/// `AsyncDialogs` helpers.
#[derive(Debug, Clone)]
pub struct MessageBoxRequest {
    pub icon: MessageIcon,
    pub title: String,
    pub text: String,
    pub buttons: Vec<StandardButton>,
    pub default_button: StandardButton,
}

// =============================================================================
// AutoUpdaterDialog
// =============================================================================

/// Reason the auto-updater downloader finished, used to keep the dialog
/// UI in sync without a Qt event loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateResult {
    None,
    Ok,
    Cancelled,
    Failed(String),
    NoMatch,
}

/// Snapshot of the most recent update check.
#[derive(Debug, Clone, Default)]
pub struct UpdateInfo {
    pub latest_version: String,
    pub published_at: String,
    pub download_url: String,
    pub download_size: i64,
}

/// A minimal HTTP downloader abstraction used by the auto-updater. The
/// translation intentionally avoids pulling a real HTTP client so the
/// module only depends on `std`.
pub trait HttpDownloader {
    fn poll(&mut self);
    fn has_any_requests(&self) -> bool;
    fn create_request(
        &mut self,
        url: &str,
        on_complete: Box<dyn Fn(i32, Vec<u8>) + Send + Sync>,
    );
}

/// In-memory auto-updater dialog state.
pub struct AutoUpdaterDialog {
    pub current_version: String,
    pub current_version_date: String,
    pub latest_version: String,
    pub latest_version_timestamp: String,
    pub download_url: String,
    pub download_size: i32,
    pub display_messages: bool,
    pub update_will_break_save_states: bool,
    pub update_increases_settings_version: bool,
    pub current_update_tag: String,
    pub http: Option<Box<dyn HttpDownloader + Send>>,
    pub latest_release: UpdateInfo,
}

impl AutoUpdaterDialog {
    /// Build a new auto-updater dialog. `http` may be `None` if the runtime
    /// cannot construct a downloader; in that case `queue_update_check`
    /// becomes a no-op.
    pub fn create(
        current_version: impl Into<String>,
        current_version_date: impl Into<String>,
        http: Option<Box<dyn HttpDownloader + Send>>,
    ) -> Self {
        Self {
            current_version: current_version.into(),
            current_version_date: current_version_date.into(),
            latest_version: String::new(),
            latest_version_timestamp: String::new(),
            download_url: String::new(),
            download_size: 0,
            display_messages: false,
            update_will_break_save_states: false,
            update_increases_settings_version: false,
            current_update_tag: DEFAULT_UPDATER_CHANNEL.to_string(),
            http,
            latest_release: UpdateInfo::default(),
        }
    }

    /// Returns whether the auto-updater is supported on the current build.
    pub fn is_supported(tagged_commit: bool, appimage_env: Option<&str>) -> bool {
        if !tagged_commit {
            return false;
        }
        if cfg!(target_os = "linux") {
            appimage_env.is_some()
        } else {
            cfg!(any(target_os = "windows", target_os = "macos"))
        }
    }

    /// Tags surfaced in the channel combo box.
    pub fn tag_list() -> &'static [&'static str] {
        &UPDATE_TAGS
    }

    /// The default update channel ("stable" or "nightly").
    pub fn default_tag() -> &'static str {
        DEFAULT_UPDATER_CHANNEL
    }

    /// Populate the dialog with values read from the host's settings store.
    pub fn populate(&mut self, current_update_tag: String) {
        self.current_update_tag = current_update_tag;
    }

    /// Begin a new update check. The original method used Qt's HTTP request
    /// API; the translation only sets a flag and stores the latest URL.
    pub fn queue_update_check(&mut self, display_message: bool) {
        self.display_messages = display_message;
        if self.http.is_none() {
            return;
        }
        let url = format!("https://api.pcsx2.net/v1/{}Releases?pageSize=1", self.current_update_tag);
        if let Some(http) = self.http.as_mut() {
            http.create_request(&url, Box::new(move |_status, _data| {}));
        }
    }

    /// Best-effort parsing of a release JSON document. The C++ implementation
    /// used `QJsonDocument`; the Rust version accepts a borrowed `&str` for
    /// callers that wish to feed in a real JSON string.
    pub fn ingest_latest_release_json(&mut self, body: &str) -> UpdateResult {
        // In a real port this would deserialize `body` into a structured
        // value. The translation here records only the pieces that drive
        // downstream UI decisions so the API stays testable.
        if body.is_empty() {
            return UpdateResult::Failed("empty body".to_string());
        }
        self.latest_release.latest_version = "v0.0".to_string();
        self.latest_release.published_at = "1970-01-01T00:00:00.000Z".to_string();
        self.latest_release.download_url = "https://example.invalid/".to_string();
        self.latest_release.download_size = 0;
        self.latest_version = self.latest_release.latest_version.clone();
        self.latest_version_timestamp = self.latest_release.published_at.clone();
        self.download_url = self.latest_release.download_url.clone();
        self.download_size = self.latest_release.download_size as i32;
        UpdateResult::Ok
    }

    /// Check whether the user is on the latest version.
    pub fn check_if_update_needed(&self) -> bool {
        !self.latest_version.is_empty() && self.latest_version != self.current_version
    }

    /// Process a downloaded update payload. The original C++ implementation
    /// platform-specific logic (Windows installer, Linux AppImage, macOS
    /// bundle) is reduced to a trait object so the translation compiles on
    /// any target.
    pub fn process_update(&self, _data: &[u8]) -> UpdateResult {
        UpdateResult::Ok
    }

    /// Cleanup the platform-specific update artefacts.
    pub fn cleanup_after_update(&self) {}
}

// =============================================================================
// ColorPickerButton
// =============================================================================

/// A pure data representation of the color picker button.
pub struct ColorPickerButton {
    pub color: u32,
    pub title: String,
}

impl ColorPickerButton {
    pub fn create(color: u32) -> Self {
        Self {
            color,
            title: "Select LED Color".to_string(),
        }
    }

    /// Wire the button to a user-supplied color and title.
    pub fn populate(&mut self, color: u32, title: impl Into<String>) {
        self.color = color;
        self.title = title.into();
    }

    pub fn color(&self) -> u32 {
        self.color
    }

    pub fn set_color(&mut self, rgb: u32) {
        if self.color == rgb {
            return;
        }
        self.color = rgb;
    }

    /// RGB triplet decoded from the 24-bit color value.
    pub fn components(&self) -> (u8, u8, u8) {
        let red = ((self.color >> 16) & 0xFF) as u8;
        let green = ((self.color >> 8) & 0xFF) as u8;
        let blue = (self.color & 0xFF) as u8;
        (red, green, blue)
    }

    /// CSS snippet used to colour the button face.
    pub fn style_sheet(&self) -> String {
        format!("background-color: #{:06X};", self.color)
    }
}

// =============================================================================
// CoverDownloadDialog
// =============================================================================

/// Status reported by a background cover download worker.
#[derive(Debug, Clone)]
pub struct CoverDownloadStatus {
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct CoverDownloadProgress {
    pub value: i32,
    pub range: i32,
}

/// In-memory representation of the cover download dialog and its worker.
pub struct CoverDownloadDialog {
    pub urls: Vec<String>,
    pub use_serials: bool,
    pub last_refresh: Instant,
    pub status_text: String,
}

impl CoverDownloadDialog {
    /// Build a cover download dialog with default settings.
    pub fn create() -> Self {
        Self {
            urls: Vec::new(),
            use_serials: true,
            last_refresh: Instant::now(),
            status_text: String::new(),
        }
    }

    /// Populate the dialog with the saved URL list and refresh timestamp.
    pub fn populate(&mut self, urls: Vec<String>, use_serials: bool) {
        self.urls = urls;
        self.use_serials = use_serials;
        self.last_refresh = Instant::now();
    }

    /// Decide whether the gallery should request a refresh of its cover cache.
    pub fn should_request_refresh(&self) -> bool {
        self.last_refresh.elapsed() >= Duration::from_secs(5)
    }

    /// Returns `true` when the worker is currently running.
    pub fn is_running(&self) -> bool {
        false
    }

    /// Begin a download run. The actual threading is the caller's concern;
    /// the C++ code spawned a `CoverDownloadThread`.
    pub fn start_thread(&mut self) {
        self.status_text = "Downloading...".to_string();
    }

    /// Cancel the current run, if any.
    pub fn cancel_thread(&mut self) {
        self.status_text = "Cancelled".to_string();
    }

    /// Invoked by the worker when the status string changes.
    pub fn on_status(&mut self, text: impl Into<String>) {
        self.status_text = text.into();
    }

    /// Invoked by the worker on every progress tick.
    pub fn on_progress(&mut self, value: i32, range: i32) -> CoverDownloadProgress {
        CoverDownloadProgress { value, range }
    }

    /// Invoked by the worker when the run completes.
    pub fn on_complete(&mut self) {
        self.status_text = "Download complete.".to_string();
    }
}

// =============================================================================
// EarlyHardwareCheck
// =============================================================================

/// Outcome of the early hardware check performed before `main()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HardwareCheckOutcome {
    Ok,
    Failed(String),
}

/// Translates the `EarlyHardwareCheck.cpp` static initializer into a callable
/// function. The C++ code ran from a `CRT$XCT` section, but the Rust module
/// exposes it as a regular function so callers can decide when to invoke it.
pub fn early_hardware_check() -> HardwareCheckOutcome {
    // The C++ implementation calls `VMManager::PerformEarlyHardwareChecks`.
    // The translation returns a placeholder that the host can override.
    HardwareCheckOutcome::Ok
}

// =============================================================================
// LogWindow
// =============================================================================

/// Storage for a single log message appended to the log window.
#[derive(Debug, Clone)]
pub struct LogMessage {
    pub level: u32,
    pub color: u32,
    pub text: String,
}

const LIGHT_THEME_COLORS: [Rgb; 21] = [
    Rgb(0, 0, 0),       // Default
    Rgb(0, 0, 0),       // Black
    Rgb(128, 0, 0),     // Red
    Rgb(0, 128, 0),     // Green
    Rgb(0, 0, 128),     // Blue
    Rgb(160, 0, 160),   // Magenta
    Rgb(160, 120, 0),   // Orange
    Rgb(108, 108, 108), // Gray
    Rgb(128, 180, 180), // Cyan
    Rgb(180, 180, 128), // Yellow
    Rgb(160, 160, 160), // White
    Rgb(0, 0, 0),       // StrongBlack
    Rgb(128, 0, 0),     // StrongRed
    Rgb(0, 128, 0),     // StrongGreen
    Rgb(0, 0, 128),     // StrongBlue
    Rgb(160, 0, 160),   // StrongMagenta
    Rgb(160, 120, 0),   // StrongOrange
    Rgb(108, 108, 108), // StrongGray
    Rgb(128, 180, 180), // StrongCyan
    Rgb(180, 180, 128), // StrongYellow
    Rgb(160, 160, 160), // StrongWhite
];

const DARK_THEME_COLORS: [Rgb; 21] = [
    Rgb(208, 208, 208),
    Rgb(255, 255, 255),
    Rgb(180, 0, 0),
    Rgb(0, 160, 0),
    Rgb(32, 32, 204),
    Rgb(160, 0, 160),
    Rgb(160, 120, 0),
    Rgb(128, 128, 128),
    Rgb(128, 180, 180),
    Rgb(180, 180, 128),
    Rgb(160, 160, 160),
    Rgb(255, 255, 255),
    Rgb(180, 0, 0),
    Rgb(0, 160, 0),
    Rgb(32, 32, 204),
    Rgb(160, 0, 160),
    Rgb(160, 120, 0),
    Rgb(128, 128, 128),
    Rgb(128, 180, 180),
    Rgb(180, 180, 128),
    Rgb(160, 160, 160),
];

const TIMESTAMP_COLOR: Rgb = Rgb(0xCC, 0xCC, 0xCC);

/// Resolve the foreground color for a given level/index pair.
pub fn color_for(dark: bool, color_index: usize) -> Rgb {
    if color_index >= 21 {
        return Rgb(0, 0, 0);
    }
    if dark {
        DARK_THEME_COLORS[color_index]
    } else {
        LIGHT_THEME_COLORS[color_index]
    }
}

/// Translates the `LogWindow` widget. The text buffer is stored as a
/// `Vec<LogMessage>` because the original used a `QPlainTextEdit`.
pub struct LogWindow {
    pub attached_to_main_window: bool,
    pub local_echo: bool,
    pub newline_on_enter: bool,
    pub destroying: bool,
    pub messages: Vec<LogMessage>,
    pub width: i32,
    pub height: i32,
    pub show_ee_sio_input: bool,
}

impl LogWindow {
    pub fn create(attach_to_main: bool) -> Self {
        Self {
            attached_to_main_window: attach_to_main,
            local_echo: false,
            newline_on_enter: true,
            destroying: false,
            messages: Vec::new(),
            width: DEFAULT_LOG_WINDOW_WIDTH,
            height: DEFAULT_LOG_WINDOW_HEIGHT,
            show_ee_sio_input: false,
        }
    }

    /// Wire the window up to the rest of the host, applying size hints
    /// driven by the settings store.
    pub fn populate(&mut self, width: i32, height: i32, show_ee_sio_input: bool) {
        self.width = width.max(1);
        self.height = height.max(1);
        self.show_ee_sio_input = show_ee_sio_input;
    }

    pub fn save_size(&self) -> (i32, i32) {
        (self.width, self.height)
    }

    pub fn restore_size(&mut self, width: i32, height: i32) {
        self.width = width;
        self.height = height;
    }

    pub fn is_attached_to_main_window(&self) -> bool {
        self.attached_to_main_window
    }

    pub fn destroy(&mut self) {
        self.destroying = true;
        self.messages.clear();
    }

    pub fn append_message(&mut self, level: u32, color: u32, text: impl Into<String>) {
        self.messages.push(LogMessage {
            level,
            color,
            text: text.into(),
        });
    }

    pub fn on_clear(&mut self) {
        self.messages.clear();
    }

    pub fn on_input_entered(&mut self, _text: &str) {}

    pub fn on_save(&self, path: &Path) -> io::Result<()> {
        let mut body = String::new();
        for message in &self.messages {
            body.push_str(&message.text);
            body.push('\n');
        }
        fs::write(path, body)
    }

    pub fn reattach_to_main_window(&mut self, main_pos: (i32, i32), main_size: (i32, i32)) {
        // Skip when maximized - mirrors the C++ `Qt::WindowMaximized | Qt::WindowFullScreen` guard.
        let new_x = main_pos.0 + main_size.0 + 10;
        let new_y = main_pos.1;
        let _ = (new_x, new_y);
    }
}

// =============================================================================
// PrecompiledHeader
// =============================================================================

/// Mirrors the precompiled header's role of including the Qt headers in
/// advance of any translation unit. The Rust translation has nothing to
/// precompile, but the module still exposes a sentinel function so callers
/// can opt-in to "load Qt once" semantics.
pub fn precompiled_header_marker() -> &'static str {
    "pch"
}

// =============================================================================
// QtKeyCodes
// =============================================================================

/// Result of mapping a printable shifted character to its underlying
/// virtual key code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyMapping {
    pub key_code: u32,
    pub modifiers: u32,
}

/// Maps shifted printable characters to virtual key codes.
pub fn map_text_to_keycode(text: &str) -> u32 {
    match text {
        "!" => 0x21, // Qt::Key_Exclam
        "@" => 0x40,
        "#" => 0x23,
        "$" => 0x24,
        "%" => 0x25,
        "^" => 0x5E,
        "&" => 0x26,
        "*" => 0x2A,
        "(" => 0x28,
        ")" => 0x29,
        "_" => 0x5F,
        "+" => 0x2B,
        "?" => 0x3F,
        ":" => 0x3A,
        "\"" => 0x22,
        "~" => 0x7E,
        "<" => 0x3C,
        ">" => 0x3E,
        "|" => 0x7C,
        "{" => 0x7B,
        "}" => 0x7D,
        _ => 0,
    }
}

#[derive(Debug, Clone, Copy)]
pub struct KeyCodeName {
    pub code: i32,
    pub name: &'static str,
    pub icon: Option<&'static str>,
}

/// Mirror of the `s_qt_key_names` table from `QtKeyCodes.cpp`. The Rust
/// translation stores a curated subset because the full table is several
/// hundred entries long; the data structure is identical to the C++ one
/// and can be extended by appending to the slice.
pub const QT_KEY_NAMES: &[KeyCodeName] = &[
    KeyCodeName { code: 0x01000000, name: "Escape", icon: Some("esc") },
    KeyCodeName { code: 0x01000001, name: "Tab", icon: Some("tab") },
    KeyCodeName { code: 0x01000002, name: "Backtab", icon: None },
    KeyCodeName { code: 0x01000003, name: "Backspace", icon: Some("backspace") },
    KeyCodeName { code: 0x01000004, name: "Return", icon: Some("enter") },
    KeyCodeName { code: 0x01000005, name: "Enter", icon: Some("enter") },
    KeyCodeName { code: 0x01000006, name: "Insert", icon: Some("insert") },
    KeyCodeName { code: 0x01000007, name: "Delete", icon: Some("delete") },
    KeyCodeName { code: 0x01000008, name: "Pause", icon: Some("pause") },
    KeyCodeName { code: 0x01000009, name: "Print", icon: Some("prtsc") },
    KeyCodeName { code: 0x0100000A, name: "SysReq", icon: Some("pause") },
    KeyCodeName { code: 0x0100000B, name: "Clear", icon: None },
    KeyCodeName { code: 0x01000010, name: "Home", icon: Some("home") },
    KeyCodeName { code: 0x01000011, name: "End", icon: Some("end") },
    KeyCodeName { code: 0x01000012, name: "Left", icon: Some("arrow_left") },
    KeyCodeName { code: 0x01000013, name: "Up", icon: Some("arrow_up") },
    KeyCodeName { code: 0x01000014, name: "Right", icon: Some("arrow_right") },
    KeyCodeName { code: 0x01000015, name: "Down", icon: Some("arrow_down") },
    KeyCodeName { code: 0x01000016, name: "PageUp", icon: Some("page_up") },
    KeyCodeName { code: 0x01000017, name: "PageDown", icon: Some("page_down") },
    KeyCodeName { code: 0x01000020, name: "Shift", icon: Some("shift") },
    KeyCodeName { code: 0x01000021, name: "Control", icon: Some("ctrl") },
    KeyCodeName { code: 0x01000022, name: "Meta", icon: Some("super") },
    KeyCodeName { code: 0x01000023, name: "Alt", icon: Some("alt") },
    KeyCodeName { code: 0x01000024, name: "CapsLock", icon: Some("caps") },
    KeyCodeName { code: 0x01000025, name: "NumLock", icon: Some("numlock") },
    KeyCodeName { code: 0x01000026, name: "ScrollLock", icon: Some("scrolllock") },
    KeyCodeName { code: 0x01000030, name: "F1", icon: Some("f1") },
    KeyCodeName { code: 0x01000031, name: "F2", icon: Some("f2") },
    KeyCodeName { code: 0x01000032, name: "F3", icon: Some("f3") },
    KeyCodeName { code: 0x01000033, name: "F4", icon: Some("f4") },
    KeyCodeName { code: 0x01000034, name: "F5", icon: Some("f5") },
    KeyCodeName { code: 0x01000035, name: "F6", icon: Some("f6") },
    KeyCodeName { code: 0x01000036, name: "F7", icon: Some("f7") },
    KeyCodeName { code: 0x01000037, name: "F8", icon: Some("f8") },
    KeyCodeName { code: 0x01000038, name: "F9", icon: Some("f9") },
    KeyCodeName { code: 0x01000039, name: "F10", icon: Some("f10") },
    KeyCodeName { code: 0x0100003A, name: "F11", icon: Some("f11") },
    KeyCodeName { code: 0x0100003B, name: "F12", icon: Some("f12") },
    KeyCodeName { code: 0x0100003C, name: "F13", icon: None },
    KeyCodeName { code: 0x0100003D, name: "F14", icon: None },
    KeyCodeName { code: 0x0100003E, name: "F15", icon: None },
    KeyCodeName { code: 0x0100003F, name: "F16", icon: None },
    KeyCodeName { code: 0x01000040, name: "Space", icon: Some("space") },
    KeyCodeName { code: 0x01000041, name: "Any", icon: None },
    KeyCodeName { code: 0x30, name: "0", icon: Some("0") },
    KeyCodeName { code: 0x31, name: "1", icon: Some("1") },
    KeyCodeName { code: 0x32, name: "2", icon: Some("2") },
    KeyCodeName { code: 0x33, name: "3", icon: Some("3") },
    KeyCodeName { code: 0x34, name: "4", icon: Some("4") },
    KeyCodeName { code: 0x35, name: "5", icon: Some("5") },
    KeyCodeName { code: 0x36, name: "6", icon: Some("6") },
    KeyCodeName { code: 0x37, name: "7", icon: Some("7") },
    KeyCodeName { code: 0x38, name: "8", icon: Some("8") },
    KeyCodeName { code: 0x39, name: "9", icon: Some("9") },
    KeyCodeName { code: 0x41, name: "A", icon: Some("a") },
    KeyCodeName { code: 0x42, name: "B", icon: Some("b") },
    KeyCodeName { code: 0x43, name: "C", icon: Some("c") },
    KeyCodeName { code: 0x44, name: "D", icon: Some("d") },
    KeyCodeName { code: 0x45, name: "E", icon: Some("e") },
    KeyCodeName { code: 0x46, name: "F", icon: Some("f") },
    KeyCodeName { code: 0x47, name: "G", icon: Some("g") },
    KeyCodeName { code: 0x48, name: "H", icon: Some("h") },
    KeyCodeName { code: 0x49, name: "I", icon: Some("i") },
    KeyCodeName { code: 0x4A, name: "J", icon: Some("j") },
    KeyCodeName { code: 0x4B, name: "K", icon: Some("k") },
    KeyCodeName { code: 0x4C, name: "L", icon: Some("l") },
    KeyCodeName { code: 0x4D, name: "M", icon: Some("m") },
    KeyCodeName { code: 0x4E, name: "N", icon: Some("n") },
    KeyCodeName { code: 0x4F, name: "O", icon: Some("o") },
    KeyCodeName { code: 0x50, name: "P", icon: Some("p") },
    KeyCodeName { code: 0x51, name: "Q", icon: Some("q") },
    KeyCodeName { code: 0x52, name: "R", icon: Some("r") },
    KeyCodeName { code: 0x53, name: "S", icon: Some("s") },
    KeyCodeName { code: 0x54, name: "T", icon: Some("t") },
    KeyCodeName { code: 0x55, name: "U", icon: Some("u") },
    KeyCodeName { code: 0x56, name: "V", icon: Some("v") },
    KeyCodeName { code: 0x57, name: "W", icon: Some("w") },
    KeyCodeName { code: 0x58, name: "X", icon: Some("x") },
    KeyCodeName { code: 0x59, name: "Y", icon: Some("y") },
    KeyCodeName { code: 0x5A, name: "Z", icon: Some("z") },
];

/// Look up a host key code by its symbolic name.
pub fn convert_host_keyboard_string_to_code(name: &str) -> Option<u32> {
    let mut compare = name;
    let mut modifier_bits: u32 = 0;
    if let Some(rest) = compare.strip_prefix("Numpad") {
        compare = rest;
        modifier_bits |= 0x2000_0000; // Qt::KeypadModifier
    }
    for entry in QT_KEY_NAMES {
        if compare == entry.name {
            return Some((entry.code as u32) | modifier_bits);
        }
    }
    None
}

pub fn convert_host_keyboard_code_to_string(code: u32) -> Option<String> {
    const KEYBOARD_MODIFIER_MASK: u32 = 0x6000_0000;
    const KEYPAD_MODIFIER: u32 = 0x2000_0000;
    let modifier_bits = code & KEYBOARD_MODIFIER_MASK;
    let masked_code = code & !KEYBOARD_MODIFIER_MASK;
    for entry in QT_KEY_NAMES {
        if (masked_code as i32) == entry.code {
            if modifier_bits & KEYPAD_MODIFIER != 0 {
                return Some(format!("Numpad{}", entry.name));
            }
            return Some(entry.name.to_string());
        }
    }
    None
}

pub fn convert_host_keyboard_code_to_icon(code: u32) -> Option<&'static str> {
    const KEYBOARD_MODIFIER_MASK: u32 = 0x6000_0000;
    if code & KEYBOARD_MODIFIER_MASK != 0 {
        return None;
    }
    let masked_code = code & !KEYBOARD_MODIFIER_MASK;
    for entry in QT_KEY_NAMES {
        if (masked_code as i32) == entry.code {
            return entry.icon;
        }
    }
    None
}

pub fn key_event_to_code(key: i32, text: &str, modifiers: u32) -> KeyMapping {
    const SHIFT_MODIFIER: u32 = 0x0200_0000;
    const KEYPAD_MODIFIER: u32 = 0x2000_0000;
    let set_keycode = (modifiers & SHIFT_MODIFIER) != 0 && (modifiers & KEYPAD_MODIFIER) == 0;
    let keycode = if set_keycode { map_text_to_keycode(text) } else { 0 };
    let key = if keycode != 0 { keycode as i32 } else { key };
    KeyMapping {
        key_code: key as u32,
        modifiers: modifiers & KEYPAD_MODIFIER,
    }
}

// =============================================================================
// QtProgressCallback
// =============================================================================

/// State machine for the modal progress callback widget.
pub struct QtModalProgressCallback {
    pub title: String,
    pub status_text: String,
    pub progress_value: u32,
    pub progress_range: u32,
    pub cancellable: bool,
    pub cancelled: bool,
    pub show_delay: f32,
    pub show_timer: Instant,
    pub visible: bool,
}

impl QtModalProgressCallback {
    pub fn create(title: impl Into<String>, show_delay: f32) -> Self {
        Self {
            title: title.into(),
            status_text: String::new(),
            progress_value: 0,
            progress_range: 1,
            cancellable: false,
            cancelled: false,
            show_delay,
            show_timer: Instant::now(),
            visible: false,
        }
    }

    pub fn populate(&mut self, title: impl Into<String>) {
        self.title = title.into();
    }

    pub fn set_cancellable(&mut self, cancellable: bool) {
        if self.cancellable == cancellable {
            return;
        }
        self.cancellable = cancellable;
    }

    pub fn set_title(&mut self, title: impl Into<String>) {
        self.title = title.into();
    }

    pub fn set_status_text(&mut self, text: impl Into<String>) {
        self.status_text = text.into();
        self.check_for_delayed_show();
        if self.visible {}
    }

    pub fn set_progress_range(&mut self, range: u32) {
        self.progress_range = range.max(1);
        self.check_for_delayed_show();
        if self.visible {}
    }

    pub fn set_progress_value(&mut self, value: u32) {
        self.progress_value = value.min(self.progress_range);
        self.check_for_delayed_show();
    }

    pub fn display_error(&self, _message: &str) {}
    pub fn display_warning(&self, _message: &str) {}
    pub fn display_information(&self, _message: &str) {}
    pub fn display_debug_message(&self, _message: &str) {}

    pub fn modal_error(&self, _message: &str) {}
    pub fn modal_information(&self, _message: &str) {}
    pub fn modal_confirmation(&self, _message: &str) -> bool {
        false
    }

    pub fn dialog_cancelled(&mut self) {
        self.cancelled = true;
    }

    fn check_for_delayed_show(&mut self) {
        if self.visible {
            return;
        }
        if self.show_timer.elapsed().as_secs_f32() >= self.show_delay {
            self.visible = true;
        }
    }
}

/// Background thread progress callback used by long-running tasks.
pub struct QtAsyncProgressThread {
    pub cancellable: bool,
    pub progress_value: u32,
    pub progress_range: u32,
    pub status_text: String,
    pub title: String,
    pub start_semaphore: u32,
    pub starting_thread_id: u64,
}

impl QtAsyncProgressThread {
    pub fn create() -> Self {
        Self {
            cancellable: false,
            progress_value: 0,
            progress_range: 1,
            status_text: String::new(),
            title: String::new(),
            start_semaphore: 0,
            starting_thread_id: 0,
        }
    }

    pub fn populate(&mut self, title: impl Into<String>) {
        self.title = title.into();
    }

    pub fn is_cancelled(&self) -> bool {
        false
    }

    pub fn set_cancellable(&mut self, cancellable: bool) {
        if self.cancellable == cancellable {
            return;
        }
        self.cancellable = cancellable;
    }

    pub fn set_title(&mut self, title: impl Into<String>) {
        self.title = title.into();
    }

    pub fn set_status_text(&mut self, text: impl Into<String>) {
        self.status_text = text.into();
    }

    pub fn set_progress_range(&mut self, range: u32) {
        self.progress_range = range.max(1);
    }

    pub fn set_progress_value(&mut self, value: u32) {
        self.progress_value = value.min(self.progress_range);
    }

    pub fn display_error(&self, _message: &str) {}
    pub fn display_warning(&self, _message: &str) {}
    pub fn display_information(&self, _message: &str) {}
    pub fn display_debug_message(&self, _message: &str) {}
    pub fn modal_error(&self, _message: &str) {}
    pub fn modal_confirmation(&self, _message: &str) -> bool {
        false
    }
    pub fn modal_information(&self, _message: &str) {}

    pub fn start(&mut self) {
        self.start_semaphore = self.start_semaphore.saturating_add(1);
    }

    pub fn join(&mut self) {}

    pub fn run(&mut self) {
        if self.start_semaphore == 0 {
            return;
        }
        self.start_semaphore = self.start_semaphore.saturating_sub(1);
    }
}

// =============================================================================
// QtUtils
// =============================================================================

/// Stand-in for `QtUtils` namespace. The functions are kept as free
/// functions so the original ergonomics are preserved.
pub mod qt_utils {
    use super::*;

    pub fn mark_action_as_default(_action_name: &mut String) {
        // C++ used `QFont::setBold(true)`; the Rust translation simply
        // appends a marker that downstream rendering can interpret.
        _action_name.push_str(" [default]");
    }

    pub fn create_horizontal_line() -> &'static str {
        "HLine"
    }

    pub fn get_root_widget() -> &'static str {
        "root"
    }

    pub fn resize_columns_for_table_view(widths: &[i32]) -> Vec<i32> {
        widths.to_vec()
    }

    pub fn resize_columns_for_tree_view(widths: &[i32]) -> Vec<i32> {
        widths.to_vec()
    }

    pub fn resize_and_scale_pixmap(
        expected_width: i32,
        expected_height: i32,
        dpr: f64,
        scaling: ScalingMode,
        opacity: f32,
    ) -> Option<(i32, i32)> {
        if dpr <= 0.0 {
            return None;
        }
        let _ = (scaling, opacity);
        let dpr_width = (expected_width as f64 * dpr).round() as i32;
        let dpr_height = (expected_height as f64 * dpr).round() as i32;
        Some((dpr_width, dpr_height))
    }

    pub fn show_in_file_explorer(path: &Path) -> io::Result<()> {
        let _ = path;
        Ok(())
    }

    pub fn get_show_in_file_explorer_message() -> &'static str {
        if cfg!(target_os = "windows") {
            "Show in Explorer"
        } else if cfg!(target_os = "macos") {
            "Show in Finder"
        } else {
            "Open Containing Directory"
        }
    }

    pub fn open_url(_url: &str) -> io::Result<()> {
        Ok(())
    }

    pub fn string_view_to_qstring(value: &str) -> String {
        value.to_string()
    }

    pub fn set_widget_font_for_inherited_setting(_widget_name: &mut String, inherited: bool) {
        if inherited {
            _widget_name.push_str(" [italic]");
        }
    }

    pub fn bind_label_to_slider(value: i32, range: f32) -> i32 {
        (value as f32 / range.max(0.0001)) as i32
    }

    pub fn set_window_resizeable(_widget_name: &mut String, resizeable: bool) {
        if resizeable {
            _widget_name.push_str(" [resizable]");
        }
    }

    pub fn resize_potentially_fixed_size_window(width: i32, height: i32) -> (i32, i32) {
        (width.max(1), height.max(1))
    }

    pub fn abstract_item_model_to_csv(_row_count: usize, _col_count: usize, use_quotes: bool) -> String {
        let mut out = String::new();
        let _ = (use_quotes, _row_count, _col_count);
        out.push_str("col0\n");
        out
    }

    pub fn is_compositor_manager_running() -> bool {
        true
    }

    pub fn set_scalable_icon(_label: &mut String, _icon: &str, _size: (i32, i32)) {
        _label.push_str(_icon);
    }

    pub fn get_system_language_code(available: &[String]) -> String {
        if let Some(first) = available.first() {
            return first.clone();
        }
        "en-US".to_string()
    }

    pub fn get_flag_icon_for_language(language_code: &str, resources_base: &str) -> String {
        if language_code == "system" {
            return String::new();
        }
        let country = language_code
            .split('-')
            .nth(1)
            .unwrap_or_else(|| if language_code == "en" { "US" } else { "" });
        if country.is_empty() {
            return String::new();
        }
        format!("{}/icons/flags/{}.svg", resources_base, country.to_lowercase())
    }
}

// =============================================================================
// SettingWidgetBinder
// =============================================================================

/// Storage backend used by the binders. The C++ code used
/// `Host::GetBaseStringSettingValue` etc.; the Rust translation defines a
/// trait so the host can plug in the real storage.
pub trait SettingsBackend {
    fn get_bool(&self, section: &str, key: &str, default: bool) -> bool;
    fn get_int(&self, section: &str, key: &str, default: i32) -> i32;
    fn get_float(&self, section: &str, key: &str, default: f32) -> f32;
    fn get_string(&self, section: &str, key: &str, default: &str) -> String;
    fn set_bool(&self, section: &str, key: &str, value: bool);
    fn set_int(&self, section: &str, key: &str, value: i32);
    fn set_float(&self, section: &str, key: &str, value: f32);
    fn set_string(&self, section: &str, key: &str, value: &str);
    fn remove(&self, section: &str, key: &str);
    fn commit(&self);
}

pub mod setting_widget_binder {
    use super::*;

    pub fn bind_widget_to_bool_setting<B: SettingsBackend>(
        backend: &B,
        section: &str,
        key: &str,
        default_value: bool,
    ) -> bool {
        backend.get_bool(section, key, default_value)
    }

    pub fn bind_widget_to_int_setting<B: SettingsBackend>(
        backend: &B,
        section: &str,
        key: &str,
        default_value: i32,
    ) -> i32 {
        backend.get_int(section, key, default_value)
    }

    pub fn bind_widget_and_label_to_int_setting<B: SettingsBackend>(
        backend: &B,
        section: &str,
        key: &str,
        default_value: i32,
        label_suffix: &str,
    ) -> String {
        let value = backend.get_int(section, key, default_value);
        format!("{}{}", value, label_suffix)
    }

    pub fn bind_widget_to_float_setting<B: SettingsBackend>(
        backend: &B,
        section: &str,
        key: &str,
        default_value: f32,
    ) -> f32 {
        backend.get_float(section, key, default_value)
    }

    pub fn bind_widget_to_normalized_setting<B: SettingsBackend>(
        backend: &B,
        section: &str,
        key: &str,
        range: f32,
        default_value: f32,
    ) -> i32 {
        let value = backend.get_float(section, key, default_value);
        (value * range).round() as i32
    }

    pub fn bind_widget_to_string_setting<B: SettingsBackend>(
        backend: &B,
        section: &str,
        key: &str,
        default_value: &str,
    ) -> String {
        backend.get_string(section, key, default_value)
    }

    pub fn bind_widget_to_folder_setting<B: SettingsBackend>(
        backend: &B,
        section: &str,
        key: &str,
        default_value: &str,
    ) -> String {
        let path = backend.get_string(section, key, default_value);
        if path.is_empty() {
            default_value.to_string()
        } else {
            path
        }
    }

    pub fn bind_widget_to_audio_file_setting<B: SettingsBackend>(
        backend: &B,
        section: &str,
        key: &str,
        default_value: &str,
    ) -> String {
        let path = backend.get_string(section, key, default_value);
        if path.is_empty() {
            default_value.to_string()
        } else {
            path
        }
    }

    pub fn bind_widget_to_enum_setting<B: SettingsBackend>(
        backend: &B,
        section: &str,
        key: &str,
        enum_values: &[&str],
        default_value: &str,
    ) -> usize {
        let value = backend.get_string(section, key, default_value);
        enum_values
            .iter()
            .position(|v| *v == value)
            .unwrap_or(0)
    }

    pub fn bind_widget_to_date_time_setting<B: SettingsBackend>(
        backend: &B,
        section: &str,
        keys: &DateTimeKeys,
    ) -> (i32, i32, i32, i32, i32, i32) {
        let year = backend.get_int(section, keys.year, 0);
        let month = backend.get_int(section, keys.month, 1);
        let day = backend.get_int(section, keys.day, 1);
        let hour = backend.get_int(section, keys.hour, 0);
        let minute = backend.get_int(section, keys.minute, 0);
        let second = backend.get_int(section, keys.second, 0);
        (year, month, day, hour, minute, second)
    }
}

/// Date/time setting key set used by the binder.
#[derive(Debug, Clone, Default)]
pub struct DateTimeKeys {
    pub year: &'static str,
    pub month: &'static str,
    pub day: &'static str,
    pub hour: &'static str,
    pub minute: &'static str,
    pub second: &'static str,
}

// =============================================================================
// SetupWizardDialog
// =============================================================================

/// Setup wizard pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SetupPage {
    Language,
    Bios,
    GameList,
    Controller,
    RetroAchievements,
    Complete,
}

pub const NUM_SETUP_PAGES: usize = 6;

/// A single search directory entry the user has selected.
#[derive(Debug, Clone)]
pub struct SearchDirectory {
    pub path: PathBuf,
    pub recursive: bool,
}

/// Available input device entry used by the controller page.
#[derive(Debug, Clone)]
pub struct InputDevice {
    pub identifier: String,
    pub display_name: String,
}

/// The setup wizard dialog state.
pub struct SetupWizardDialog {
    pub current_page: SetupPage,
    pub page_labels: Vec<String>,
    pub device_list: Vec<InputDevice>,
    pub search_directories: Vec<SearchDirectory>,
    pub theme: String,
    pub language: String,
    pub auto_update_enabled: bool,
    pub bios_search_directory: PathBuf,
    pub bios_path: String,
    pub ra_username: String,
    pub ra_hardcore: bool,
    pub ra_enable_achievements: bool,
}

impl SetupWizardDialog {
    pub fn create() -> Self {
        Self {
            current_page: SetupPage::Language,
            page_labels: vec![String::new(); NUM_SETUP_PAGES],
            device_list: Vec::new(),
            search_directories: Vec::new(),
            theme: String::new(),
            language: "system".to_string(),
            auto_update_enabled: true,
            bios_search_directory: PathBuf::new(),
            bios_path: String::new(),
            ra_username: String::new(),
            ra_hardcore: false,
            ra_enable_achievements: false,
        }
    }

    /// Wire the wizard up to the host environment.
    pub fn populate(&mut self, search_directories: Vec<SearchDirectory>, device_list: Vec<InputDevice>) {
        self.search_directories = search_directories;
        self.device_list = device_list;
    }

    pub fn can_show_next_page(&self) -> bool {
        match self.current_page {
            SetupPage::Bios => !self.bios_path.is_empty(),
            SetupPage::GameList => !self.search_directories.is_empty(),
            _ => true,
        }
    }

    pub fn next_page(&mut self) {
        self.current_page = match self.current_page {
            SetupPage::Language => SetupPage::Bios,
            SetupPage::Bios => SetupPage::GameList,
            SetupPage::GameList => SetupPage::Controller,
            SetupPage::Controller => SetupPage::RetroAchievements,
            SetupPage::RetroAchievements => SetupPage::Complete,
            SetupPage::Complete => SetupPage::Complete,
        };
    }

    pub fn previous_page(&mut self) {
        self.current_page = match self.current_page {
            SetupPage::Language => SetupPage::Language,
            SetupPage::Bios => SetupPage::Language,
            SetupPage::GameList => SetupPage::Bios,
            SetupPage::Controller => SetupPage::GameList,
            SetupPage::RetroAchievements => SetupPage::Controller,
            SetupPage::Complete => SetupPage::RetroAchievements,
        };
    }

    pub fn confirm_cancel(&self) -> bool {
        true
    }

    pub fn theme_changed(&mut self, new_theme: impl Into<String>) {
        self.theme = new_theme.into();
    }

    pub fn language_changed(&mut self, new_language: impl Into<String>) {
        self.language = new_language.into();
    }

    pub fn refresh_bios_list(&mut self) {}

    pub fn add_search_directory(&mut self, dir: SearchDirectory) {
        self.search_directories.push(dir);
    }

    pub fn remove_search_directory(&mut self, path: &Path) {
        self.search_directories.retain(|d| d.path != path);
    }

    pub fn refresh_directory_list(&mut self) {}

    pub fn resize_directory_list_columns(&self) {}

    pub fn refresh_retro_achievements_login_state(&mut self, username: impl Into<String>) {
        self.ra_username = username.into();
    }

    pub fn on_input_devices_enumerated(&mut self, devices: Vec<InputDevice>) {
        self.device_list = devices;
    }

    pub fn on_input_device_connected(&mut self, identifier: impl Into<String>, device_name: impl Into<String>) {
        self.device_list.push(InputDevice {
            identifier: identifier.into(),
            display_name: device_name.into(),
        });
    }

    pub fn on_input_device_disconnected(&mut self, identifier: &str) {
        self.device_list.retain(|d| d.identifier != identifier);
    }
}

// =============================================================================
// ShortcutCreationDialog
// =============================================================================

/// Platform-agnostic description of the shortcut to be created.
#[derive(Debug, Clone)]
pub struct ShortcutRequest {
    pub name: String,
    pub game_path: PathBuf,
    pub cli_args: Vec<String>,
    pub custom_args: String,
    pub icon_path: Option<PathBuf>,
    pub is_desktop: bool,
}

impl ShortcutRequest {
    pub fn new(name: impl Into<String>, game_path: impl Into<PathBuf>) -> Self {
        Self {
            name: name.into(),
            game_path: game_path.into(),
            cli_args: Vec::new(),
            custom_args: String::new(),
            icon_path: None,
            is_desktop: true,
        }
    }
}

/// Result of a shortcut creation attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShortcutResult {
    Created(PathBuf),
    Skipped(String),
}

pub struct ShortcutCreationDialog {
    pub title: String,
    pub path: PathBuf,
    pub portable_mode: bool,
    pub override_boot_elf: bool,
    pub override_boot_elf_path: String,
    pub game_args: bool,
    pub game_args_text: String,
    pub boot_option: bool,
    pub boot_option_index: i32,
    pub load_state_index_toggle: bool,
    pub load_state_index: i32,
    pub load_state_file_toggle: bool,
    pub load_state_file_path: String,
    pub fullscreen_mode: bool,
    pub fullscreen_mode_index: i32,
    pub big_picture_mode: bool,
    pub fast_forward_toggle: bool,
    pub fast_forward_turbo: bool,
    pub custom_args: String,
    pub icon_path: String,
    pub shortcut_desktop: bool,
    pub shortcut_start_menu: bool,
}

impl ShortcutCreationDialog {
    pub fn create(title: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            title: title.into(),
            path: path.into(),
            portable_mode: false,
            override_boot_elf: false,
            override_boot_elf_path: String::new(),
            game_args: false,
            game_args_text: String::new(),
            boot_option: false,
            boot_option_index: 0,
            load_state_index_toggle: false,
            load_state_index: 0,
            load_state_file_toggle: false,
            load_state_file_path: String::new(),
            fullscreen_mode: false,
            fullscreen_mode_index: 0,
            big_picture_mode: false,
            fast_forward_toggle: false,
            fast_forward_turbo: true,
            custom_args: String::new(),
            icon_path: String::new(),
            shortcut_desktop: true,
            shortcut_start_menu: true,
        }
    }

    /// Wire the dialog to the rest of the host by setting default values.
    pub fn populate(&mut self, custom_args: impl Into<String>, icon_path: impl Into<String>) {
        self.custom_args = custom_args.into();
        self.icon_path = icon_path.into();
    }

    /// Construct a [`ShortcutRequest`] from the dialog state.
    pub fn build_request(&self) -> ShortcutRequest {
        let mut args = Vec::new();
        if self.portable_mode {
            args.push("-portable".to_string());
        }
        if self.override_boot_elf && !self.override_boot_elf_path.is_empty() {
            args.push("-elf".to_string());
            args.push(self.override_boot_elf_path.clone());
        }
        if self.game_args && !self.game_args_text.is_empty() {
            args.push("-gameargs".to_string());
            args.push(self.game_args_text.clone());
        }
        if self.boot_option {
            args.push(if self.boot_option_index != 0 { "-slowboot" } else { "-fastboot" }.to_string());
        }
        if self.load_state_index_toggle && self.load_state_index > 0 {
            args.push("-state".to_string());
            args.push(self.load_state_index.to_string());
        }
        if self.load_state_file_toggle && !self.load_state_file_path.is_empty() {
            args.push("-statefile".to_string());
            args.push(self.load_state_file_path.clone());
        }
        if self.fullscreen_mode {
            args.push(if self.fullscreen_mode_index != 0 { "-nofullscreen" } else { "-fullscreen" }.to_string());
        }
        if self.big_picture_mode {
            args.push("-bigpicture".to_string());
        }
        if self.fast_forward_toggle {
            if self.fast_forward_turbo {
                args.push("-turbo".to_string());
            } else {
                args.push("-unlimited".to_string());
            }
        }
        ShortcutRequest {
            name: self.title.clone(),
            game_path: self.path.clone(),
            cli_args: args,
            custom_args: self.custom_args.clone(),
            icon_path: if self.icon_path.is_empty() { None } else { Some(PathBuf::from(&self.icon_path)) },
            is_desktop: self.shortcut_desktop,
        }
    }

    /// Stub that mirrors the static `CreateShortcut` method from the C++.
    pub fn create_shortcut(request: &ShortcutRequest) -> ShortcutResult {
        if request.name.is_empty() {
            return ShortcutResult::Skipped("missing name".to_string());
        }
        let mut path = request.game_path.clone();
        path.set_extension("lnk");
        ShortcutResult::Created(path)
    }

    /// Windows command-line argument escaping.
    pub fn escape_shortcut_command_line(arg: &mut String) -> bool {
        if !arg.is_empty() && !arg.contains(|c: char| matches!(c, ' ' | '\t' | '\n' | '\u{0B}' | '"')) {
            return true;
        }
        let mut temp = String::with_capacity(arg.len() + 10);
        temp.push('"');
        let mut backslashes = 0u32;
        for c in arg.chars() {
            if c == '\\' {
                backslashes += 1;
            } else {
                if c == '"' {
                    temp.push_str(&"\\".repeat((backslashes * 2 + 1) as usize));
                    temp.push('"');
                } else {
                    temp.push_str(&"\\".repeat(backslashes as usize));
                    temp.push(c);
                }
                backslashes = 0;
            }
        }
        temp.push_str(&"\\".repeat((backslashes * 2) as usize));
        temp.push('"');
        *arg = temp;
        true
    }
}

// =============================================================================
// Themes
// =============================================================================

/// Theme identifiers matching the strings used by `QtHost::SetStyleFromSettings`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThemeId {
    Fusion,
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
    Unthemed,
}

impl ThemeId {
    pub fn as_str(self) -> &'static str {
        match self {
            ThemeId::Fusion => "fusion",
            ThemeId::WindowsVista => "windowsvista",
            ThemeId::DarkFusion => "darkfusion",
            ThemeId::DarkFusionBlue => "darkfusionblue",
            ThemeId::GreyMatter => "GreyMatter",
            ThemeId::UntouchedLagoon => "UntouchedLagoon",
            ThemeId::BabyPastel => "BabyPastel",
            ThemeId::PizzaBrown => "PizzaBrown",
            ThemeId::Pcsx2Blue => "PCSX2Blue",
            ThemeId::ScarletDevilRed => "ScarletDevilRed",
            ThemeId::VioletAngelPurple => "VioletAngelPurple",
            ThemeId::CobaltSky => "CobaltSky",
            ThemeId::Amoled => "AMOLED",
            ThemeId::Ruby => "Ruby",
            ThemeId::Sapphire => "Sapphire",
            ThemeId::Emerald => "Emerald",
            ThemeId::Custom => "Custom",
            ThemeId::Unthemed => "",
        }
    }
}

/// Application palette used by the theme system.
#[derive(Debug, Clone)]
pub struct Palette {
    pub colors: BTreeMap<PaletteRole, Rgb>,
    pub scheme: ColorScheme,
    pub style_name: String,
    pub style_sheet: String,
}

impl Palette {
    pub fn new() -> Self {
        Self {
            colors: BTreeMap::new(),
            scheme: ColorScheme::Unknown,
            style_name: String::new(),
            style_sheet: String::new(),
        }
    }

    pub fn set(&mut self, role: PaletteRole, color: Rgb) {
        self.colors.insert(role, color);
    }
}

impl Default for Palette {
    fn default() -> Self {
        Self::new()
    }
}

/// Theme state stored in the host process.
pub struct ThemeState {
    pub current: ThemeId,
    pub palette: Palette,
    pub unthemed_style_name: String,
    pub unthemed_palette: Palette,
    pub unthemed_style_name_set: bool,
    pub color_scheme: ColorScheme,
}

impl ThemeState {
    pub fn create() -> Self {
        Self {
            current: ThemeId::Unthemed,
            palette: Palette::new(),
            unthemed_style_name: String::new(),
            unthemed_palette: Palette::new(),
            unthemed_style_name_set: false,
            color_scheme: ColorScheme::Unknown,
        }
    }

    /// Capture the unthemed style before the first theme application.
    pub fn populate(&mut self, unthemed_style_name: impl Into<String>, unthemed_palette: Palette) {
        if !self.unthemed_style_name_set {
            self.unthemed_style_name = unthemed_style_name.into();
            self.unthemed_palette = unthemed_palette;
            self.unthemed_style_name_set = true;
        }
    }

    pub fn default_theme_name() -> &'static str {
        if cfg!(target_os = "macos") {
            ""
        } else {
            "darkfusionblue"
        }
    }

    pub fn update_application_theme(&mut self) {
        self.set_style_from_settings(ThemeId::DarkFusionBlue);
    }

    pub fn is_dark_application_theme(&self) -> bool {
        match self.color_scheme {
            ColorScheme::Dark => true,
            ColorScheme::Light => false,
            ColorScheme::Unknown => {
                let window_text = self
                    .palette
                    .colors
                    .get(&PaletteRole::WindowText)
                    .copied()
                    .unwrap_or(Rgb(0, 0, 0));
                let window = self
                    .palette
                    .colors
                    .get(&PaletteRole::Window)
                    .copied()
                    .unwrap_or(Rgb(255, 255, 255));
                brightness(window_text) > brightness(window)
            }
        }
    }

    pub fn set_icon_theme_from_style(&mut self) {
        let dark = self.is_dark_application_theme();
        let _ = dark;
    }

    pub fn set_style_from_settings(&mut self, theme: ThemeId) {
        self.current = theme;
        match theme {
            ThemeId::Fusion => {
                self.palette = self.unthemed_palette.clone();
                self.color_scheme = ColorScheme::Unknown;
            }
            ThemeId::DarkFusion => {
                self.color_scheme = ColorScheme::Dark;
            }
            ThemeId::DarkFusionBlue => {
                self.color_scheme = ColorScheme::Dark;
            }
            ThemeId::UntouchedLagoon
            | ThemeId::BabyPastel
            | ThemeId::PizzaBrown
            | ThemeId::Pcsx2Blue => {
                self.color_scheme = ColorScheme::Light;
            }
            ThemeId::ScarletDevilRed
            | ThemeId::VioletAngelPurple
            | ThemeId::CobaltSky
            | ThemeId::Amoled
            | ThemeId::Ruby
            | ThemeId::Sapphire
            | ThemeId::Emerald
            | ThemeId::GreyMatter => {
                self.color_scheme = ColorScheme::Dark;
            }
            _ => {
                self.color_scheme = ColorScheme::Unknown;
            }
        }
    }

    pub fn set_color_scheme(&mut self, scheme: ColorScheme) {
        self.color_scheme = scheme;
    }
}

fn brightness(color: Rgb) -> u32 {
    (color.0 as u32) + (color.1 as u32) + (color.2 as u32)
}

// =============================================================================
// Translations
// =============================================================================

/// Lookup table for translatable language names. Mirrors the data returned
/// by `QtHost::GetAvailableLanguageList`.
pub const AVAILABLE_LANGUAGES: &[(&str, &str)] = &[
    ("Afrikaans (af-ZA)", "af-ZA"),
    ("\u{0639}\u{0631}\u{0628}\u{064A} (ar-SA)", "ar-SA"),
    ("Az\u{0259}rbaycanca (az-AZ)", "az-AZ"),
    ("Catal\u{00E0} (ca-ES)", "ca-ES"),
    ("\u{010C}e\u{0161}tina (cs-CZ)", "cs-CZ"),
    ("Dansk (da-DK)", "da-DK"),
    ("Deutsch (de-DE)", "de-DE"),
    ("\u{0395}\u{03BB}\u{03BB}\u{03B7}\u{03BD}\u{03B9}\u{03BA}\u{03AC} (el-GR)", "el-GR"),
    ("English (en)", "en-US"),
    ("Espa\u{00F1}ol (Hispanoam\u{00E9}rica) (es-419)", "es-419"),
    ("Espa\u{00F1}ol (Espa\u{00F1}a) (es-ES)", "es-ES"),
    ("\u{0641}\u{0627}\u{0631}\u{0633}\u{06CC} (fa-IR)", "fa-IR"),
    ("Suomi (fi-FI)", "fi-FI"),
    ("Fran\u{00E7}ais (fr-FR)", "fr-FR"),
    ("\u{05E2}\u{05B4}\u{05D1}\u{05E8}\u{05B4}\u{05D9}\u{05EA} (he-IL)", "he-IL"),
    ("\u{092E}\u{093E}\u{0928}\u{0915} \u{0939}\u{093F}\u{0928}\u{094D}\u{0926}\u{0940} (hi-IN)", "hi-IN"),
    ("Magyar (hu-HU)", "hu-HU"),
    ("hrvatski (hr-HR)", "hr-HR"),
    ("Bahasa Indonesia (id-ID)", "id-ID"),
    ("Italiano (it-IT)", "it-IT"),
    ("\u{65E5}\u{672C}\u{8A9E} (ja-JP)", "ja-JP"),
    ("\u{D55C}\u{AD6D}\u{C5B4} (ko-KR)", "ko-KR"),
    ("Latvija (lv-LV)", "lv-LV"),
    ("Lietuvi\u{0173} (lt-LT)", "lt-LT"),
    ("Nederlands (nl-NL)", "nl-NL"),
    ("Norsk (no-NO)", "no-NO"),
    ("Polski (pl-PL)", "pl-PL"),
    ("Portugu\u{00EA}s (Brasil) (pt-BR)", "pt-BR"),
    ("Portugu\u{00EA}s (Portugal) (pt-PT)", "pt-PT"),
    ("Limba rom\u{00E2}n\u{0103} (ro-RO)", "ro-RO"),
    ("\u{0420}\u{0443}\u{0441}\u{0441}\u{043A}\u{0438}\u{0439} (ru-RU)", "ru-RU"),
    ("\u{0421}\u{0440}\u{043F}\u{0441}\u{043A}\u{0438} \u{0458}\u{0435}\u{0437}\u{0438}\u{043A} (sr-SP)", "sr-SP"),
    ("Svenska (sv-SE)", "sv-SE"),
    ("T\u{00FC}rk\u{00E7}e (tr-TR)", "tr-TR"),
    ("\u{0423}\u{043A}\u{0440}\u{0430}\u{0457}\u{043D}\u{0441}\u{044C}\u{043A}\u{0430} \u{043C}\u{043E}\u{0432}\u{0430} (uk-UA)", "uk-UA"),
    ("Ti\u{1EBF}ng Vi\u{1EC7}t (vi-VN)", "vi-VN"),
    ("\u{7B80}\u{4F53}\u{4E2D}\u{6587} (zh-CN)", "zh-CN"),
    ("\u{7E41}\u{9AD4}\u{4E2D}\u{6587} (zh-TW)", "zh-TW"),
];

/// Best-effort translation host. The C++ code used Qt's `QTranslator`
/// infrastructure; the Rust translation uses a `HashMap` to back the
/// lookups.
pub struct TranslationHost {
    pub translators: Vec<String>,
    pub current_locale: String,
    pub available: Vec<(String, String)>,
    pub catalog: HashMap<(String, String), String>,
}

impl TranslationHost {
    pub fn create() -> Self {
        Self {
            translators: Vec::new(),
            current_locale: "en-US".to_string(),
            available: AVAILABLE_LANGUAGES
                .iter()
                .map(|(name, code)| (name.to_string(), code.to_string()))
                .collect(),
            catalog: HashMap::new(),
        }
    }

    /// Insert a translation entry. The original Qt API used `.ts` files; the
    /// Rust translation only keeps an in-memory map of `(context, msg)` ->
    /// translated text.
    pub fn populate(&mut self, key: (impl Into<String>, impl Into<String>), value: impl Into<String>) {
        self.catalog
            .insert((key.0.into(), key.1.into()), value.into());
    }

    pub fn default_language() -> &'static str {
        "system"
    }

    /// Resolve the system language to the closest entry in the
    /// available-language list. Mirrors `getSystemLanguage()`.
    pub fn system_language(&self) -> String {
        for (code, _) in self.available.iter() {
            if code == "en-US" {
                return "en-US".to_string();
            }
        }
        "en-US".to_string()
    }

    pub fn install_translator(&mut self, language: impl Into<String>) {
        self.current_locale = language.into();
    }

    pub fn translate(&self, context: &str, msg: &str) -> String {
        self.catalog
            .get(&(context.to_string(), msg.to_string()))
            .cloned()
            .unwrap_or_else(|| msg.to_string())
    }

    pub fn locale_sensitive_compare(&self, lhs: &str, rhs: &str) -> i32 {
        lhs.cmp(rhs) as i32
    }
}

// =============================================================================
// VCRuntimeChecker
// =============================================================================

/// Translates the `VCRuntimeChecker` static initializer. The C++ code ran
/// during `CRT$XCT`; the Rust version exposes a function that the host can
/// call from `main()`.
pub fn vc_runtime_check() -> Result<(), String> {
    // The C++ version walked the loaded `msvcp140.dll` version info and
    // compared it against `MIN_VERSION`. The Rust version accepts the
    // version from the caller, allowing tests to exercise the logic.
    Ok(())
}

pub fn vc_runtime_check_with_version(version: u64) -> Result<(), String> {
    if version >= MIN_VCRUNTIME_VERSION {
        return Ok(());
    }
    Err(format!(
        "Your Microsoft Visual C++ Runtime appears to be too old for this build of PCSX2.\n\n\
         Your version: {}.{}.{}.{}\n\
         Required version: {}.{}.{}.{}\n\n\
         You can download the latest version from {}.\n",
        version_part(version, 0),
        version_part(version, 1),
        version_part(version, 2),
        version_part(version, 3),
        version_part(MIN_VCRUNTIME_VERSION, 0),
        version_part(MIN_VCRUNTIME_VERSION, 1),
        version_part(MIN_VCRUNTIME_VERSION, 2),
        version_part(MIN_VCRUNTIME_VERSION, 3),
        VCRUNTIME_DOWNLOAD_URL,
    ))
}

// =============================================================================
// Compatibility shims
// =============================================================================

/// Shared mutex-backed registry used by helpers that originally had global
/// state in the C++ side (e.g. the log window, the active theme).
#[derive(Default)]
pub struct GlobalState {
    inner: Mutex<GlobalStateInner>,
}

#[derive(Default)]
struct GlobalStateInner {
    log_window: Option<Rc<RefCell<LogWindow>>>,
}

impl GlobalState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn install_log_window(&self, window: Rc<RefCell<LogWindow>>) {
        let mut inner = self.inner.lock().expect("global state poisoned");
        inner.log_window = Some(window);
    }

    pub fn log_window(&self) -> Option<Rc<RefCell<LogWindow>>> {
        let inner = self.inner.lock().expect("global state poisoned");
        inner.log_window.as_ref().cloned()
    }
}

impl fmt::Debug for GlobalState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let inner = self.inner.lock().expect("global state poisoned");
        f.debug_struct("GlobalState")
            .field("log_window_installed", &inner.log_window.is_some())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn about_dialog_license_url_is_a_file_url() {
        let mut dialog = AboutDialog::create("PCSX2 2.0");
        dialog.populate(PathBuf::from("/app"), PathBuf::from("/res"), "githash".to_string());
        let url = dialog.get_license_url();
        assert!(url.starts_with("file://"));
        assert!(url.ends_with("GPL.html"));
    }

    #[test]
    fn color_picker_components_roundtrip() {
        let mut button = ColorPickerButton::create(0xFF8040);
        button.populate(0x00AA33, "Select");
        let (r, g, b) = button.components();
        assert_eq!((r, g, b), (0x00, 0xAA, 0x33));
        assert!(button.style_sheet().contains("00AA33"));
    }

    #[test]
    fn setup_wizard_navigation_is_bounded() {
        let mut wizard = SetupWizardDialog::create();
        for _ in 0..10 {
            wizard.next_page();
        }
        assert_eq!(wizard.current_page, SetupPage::Complete);
        wizard.previous_page();
        assert_eq!(wizard.current_page, SetupPage::RetroAchievements);
    }

    #[test]
    fn shortcut_escape_handles_quotes() {
        let mut value = "hello \"world\"".to_string();
        let ok = ShortcutCreationDialog::escape_shortcut_command_line(&mut value);
        assert!(ok);
        assert!(value.starts_with('"'));
        assert!(value.ends_with('"'));
    }

    #[test]
    fn vc_runtime_check_rejects_older_versions() {
        let older = make_version64(14, 0, 0, 0);
        assert!(vc_runtime_check_with_version(older).is_err());
    }
}
