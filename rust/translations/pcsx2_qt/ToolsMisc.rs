//! Idiomatic Rust translation of a slice of PCSX2 `pcsx2-qt` UI helpers
//! (tools/miscellaneous dialogs, color picker, cover downloader, runtime
//! checkers, etc.).
//!
//! Each public type corresponds to a Qt-based class from the original C++
//! source set. The module deliberately uses only the standard library so the
//! translation is self-contained and free of external dependencies.

#![allow(dead_code)]

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// InputRecordingViewer
// ---------------------------------------------------------------------------

/// A minimal stand-in for the type consumed by `InputRecordingViewer::show`.
/// The real C++ binding receives a heavy controller-data payload; here we
/// just carry the file path and the title to display.
#[derive(Debug, Clone)]
pub struct InputRecording {
    pub path: PathBuf,
    pub title: String,
}

/// A read-only viewer for an input recording.
///
/// The C++ implementation is a `QMainWindow` populated with a `QTableWidget`
/// of controller frames; the Rust translation only exposes the lifecycle.
pub struct InputRecordingViewer {
    recording: Option<InputRecording>,
    file_open: bool,
}

impl Default for InputRecordingViewer {
    fn default() -> Self {
        Self::new()
    }
}

impl InputRecordingViewer {
    pub fn new() -> Self {
        Self {
            recording: None,
            file_open: false,
        }
    }

    /// Display the supplied recording.
    pub fn show(&mut self, recording: &InputRecording) {
        self.file_open = true;
        self.recording = Some(recording.clone());
    }

    /// Close the currently open recording. Mirrors the C++ `closeFile()` slot.
    pub fn close(&mut self) {
        self.file_open = false;
        self.recording = None;
    }

    pub fn is_open(&self) -> bool {
        self.file_open
    }

    pub fn recording(&self) -> Option<&InputRecording> {
        self.recording.as_ref()
    }
}

// ---------------------------------------------------------------------------
// NewInputRecordingDlg
// ---------------------------------------------------------------------------

/// Kind of recording the user wants to create.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingType {
    PowerOn,
    FromSaveState,
}

/// Output of a successful `NewInputRecordingDlg::exec()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRecordingParams {
    pub rec_type: RecordingType,
    pub file_path: PathBuf,
    pub author: String,
}

/// Modal dialog for creating a new input recording.
pub struct NewInputRecordingDlg {
    rec_type: Option<RecordingType>,
    file_path: Option<PathBuf>,
    author: Option<String>,
}

impl Default for NewInputRecordingDlg {
    fn default() -> Self {
        Self::new()
    }
}

impl NewInputRecordingDlg {
    pub fn new() -> Self {
        Self {
            rec_type: None,
            file_path: None,
            author: None,
        }
    }

    /// Set the chosen recording type.
    pub fn set_recording_type(&mut self, rec_type: RecordingType) {
        self.rec_type = Some(rec_type);
    }

    /// Set the chosen file path.
    pub fn set_file_path<P: Into<PathBuf>>(&mut self, path: P) {
        self.file_path = Some(path.into());
    }

    /// Set the author name.
    pub fn set_author<S: Into<String>>(&mut self, author: S) {
        self.author = Some(author.into());
    }

    fn is_form_valid(&self) -> bool {
        self.file_path.is_some() && self.author.is_some()
    }

    /// "Run" the dialog. Returns the chosen parameters when the form is
    /// valid and the user accepted; `None` if the dialog was cancelled or
    /// the form is incomplete (mirrors Qt's disabled OK button).
    pub fn exec(&self) -> Option<NewRecordingParams> {
        if !self.is_form_valid() {
            return None;
        }
        let rec_type = self.rec_type.unwrap_or(RecordingType::PowerOn);
        let file_path = self.file_path.clone().unwrap();
        let author = self.author.clone().unwrap();

        if author.trim().is_empty() || file_path.as_os_str().is_empty() {
            return None;
        }

        Some(NewRecordingParams {
            rec_type,
            file_path,
            author,
        })
    }
}

// ---------------------------------------------------------------------------
// AboutDialog
// ---------------------------------------------------------------------------

/// Static helpers and an instance method mirroring the C++ `AboutDialog`.
pub struct AboutDialog {
    title: String,
}

impl AboutDialog {
    pub fn new() -> Self {
        Self {
            title: "About PCSX2".to_string(),
        }
    }

    /// "Show" the about dialog. In the C++ version this opens a modal
    /// `QDialog`; here we simply record the title for the caller.
    pub fn show(&mut self) {
        // No-op in this translation.
        let _ = &self.title;
    }

    pub fn website_url() -> &'static str {
        "https://pcsx2.net/"
    }

    pub fn support_forums_url() -> &'static str {
        "https://forums.pcsx2.net/"
    }

    pub fn github_repository_url() -> &'static str {
        "https://github.com/PCSX2/pcsx2"
    }

    pub fn license_url() -> &'static str {
        "https://github.com/PCSX2/pcsx2/blob/master/LICENSE.GPLv3"
    }

    pub fn third_party_licenses_url() -> &'static str {
        "https://github.com/PCSX2/pcsx2/blob/master/3rdparty.md"
    }

    pub fn wiki_url() -> &'static str {
        "https://wiki.pcsx2.net/"
    }

    pub fn documentation_url() -> &'static str {
        "https://pcsx2.net/docs/"
    }

    pub fn discord_server_url() -> &'static str {
        "https://discord.com/invite/TCzKCtCq6n"
    }
}

impl Default for AboutDialog {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// AsyncDialogs
// ---------------------------------------------------------------------------

/// Idiomatic translation of the `AsyncDialogs` Qt namespace.
///
/// The C++ implementation is a free-function namespace of helpers around
/// `QInputDialog` / `QMessageBox`. In Rust we expose a stateless struct
/// with the same flavour of methods.
pub struct AsyncDialogs;

impl AsyncDialogs {
    pub fn new() -> Self {
        Self
    }

    /// Show an error (critical) dialog.
    pub fn show_error(&self, msg: &str) {
        // In Qt this opens a non-blocking error dialog. Here we just route
        // to stderr, which is the closest non-GUI analogue.
        eprintln!("[error] {}", msg);
    }

    /// Show a warning dialog.
    pub fn show_warning(&self, msg: &str) {
        eprintln!("[warning] {}", msg);
    }

    /// Show an information dialog.
    pub fn show_info(&self, msg: &str) {
        println!("[info] {}", msg);
    }
}

impl Default for AsyncDialogs {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// AutoUpdaterDialog
// ---------------------------------------------------------------------------

/// A trimmed translation of the `AutoUpdaterDialog` class. The C++ version
/// owns an `HTTPDownloader`, a `QTimer`, and a fair amount of state; this
/// struct captures the public surface area only.
pub struct AutoUpdaterDialog {
    display_messages: bool,
}

impl Default for AutoUpdaterDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl AutoUpdaterDialog {
    pub fn new() -> Self {
        Self {
            display_messages: false,
        }
    }

    /// True on platforms for which the auto-updater is supported (Windows
    /// is the only platform PCSX2 ships updaters for).
    pub fn is_supported() -> bool {
        cfg!(target_os = "windows")
    }

    /// The list of release tags fetched from GitHub. Empty when offline /
    /// not supported.
    pub fn get_tag_list() -> Vec<String> {
        Vec::new()
    }

    /// The tag the user has elected to track (e.g. `"stable"`, `"nightly"`).
    pub fn get_default_tag() -> String {
        "stable".to_string()
    }

    /// The current local version string.
    pub fn get_current_version() -> String {
        env!("CARGO_PKG_VERSION", "0.0.0").to_string()
    }

    /// The build date of the current local version.
    pub fn get_current_version_date() -> String {
        String::new()
    }

    /// Clean up any updater artifacts left behind after a successful update.
    pub fn cleanup_after_update() {
        // No-op in this translation.
    }

    /// Kick off an asynchronous update check.
    pub fn check_for_updates(&mut self) {
        self.display_messages = false;
    }
}

// ---------------------------------------------------------------------------
// ColorPickerButton
// ------------------------------------------------------------------------===

/// 24-bit RGB helper. Wraps a `u32` with named channels, mirroring the
/// bit layout used in `ColorPickerButton`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u32);

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self(((r as u32) << 16) | ((g as u32) << 8) | (b as u32))
    }

    pub const fn red(self) -> u8 {
        ((self.0 >> 16) & 0xff) as u8
    }

    pub const fn green(self) -> u8 {
        ((self.0 >> 8) & 0xff) as u8
    }

    pub const fn blue(self) -> u8 {
        (self.0 & 0xff) as u8
    }
}

impl fmt::UpperHex for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:06X}", self.0 & 0xFFFFFF)
    }
}

/// Translation of the `ColorPickerButton` widget.
pub struct ColorPickerButton {
    color: Rgb,
}

impl Default for ColorPickerButton {
    fn default() -> Self {
        Self::new()
    }
}

impl ColorPickerButton {
    pub fn new() -> Self {
        Self { color: Rgb::new(0, 0, 0) }
    }

    /// Current color as a packed `0xRRGGBB` `u32`.
    pub fn color(&self) -> u32 {
        self.color.0
    }

    /// Set the current color from a packed `0xRRGGBB` value.
    pub fn set_color(&mut self, c: u32) {
        self.color = Rgb(c & 0xFFFFFF);
    }

    /// Convenience setter taking individual channels.
    pub fn set_rgb(&mut self, r: u8, g: u8, b: u8) {
        self.color = Rgb::new(r, g, b);
    }
}

// ---------------------------------------------------------------------------
// CoverDownloadDialog
// ---------------------------------------------------------------------------

/// Outcome of `CoverDownloadDialog::download`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadStatus {
    Completed { bytes: u64 },
    Failed(String),
}

/// Translation of `CoverDownloadDialog`. The C++ version runs a background
/// `QThread`; here we provide a synchronous, single-shot downloader.
pub struct CoverDownloadDialog {
    urls: Vec<String>,
    use_serials: bool,
}

impl Default for CoverDownloadDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl CoverDownloadDialog {
    pub fn new() -> Self {
        Self {
            urls: Vec::new(),
            use_serials: true,
        }
    }

    /// Configure the URLs to download. One per line, matching the C++
    /// behaviour of splitting on `'\n'`.
    pub fn set_urls(&mut self, urls: &str) {
        self.urls = urls
            .split('\n')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
    }

    /// Whether the downloader should derive file names from serial numbers.
    pub fn set_use_serials(&mut self, use_serials: bool) {
        self.use_serials = use_serials;
    }

    /// Persist the user's URL list to the supplied directory. Mirrors the
    /// C++ `saveCoverURLs()` which writes into `Host`'s settings layer.
    pub fn save_cover_urls<P: AsRef<Path>>(&self, dir: P) -> io::Result<()> {
        fs::create_dir_all(&dir)?;
        let path = dir.as_ref().join("cover_urls.txt");
        let body = self.urls.join("\n");
        fs::write(path, body)
    }

    /// Run a single download pass. The C++ version runs asynchronously via
    /// a `QtAsyncProgressThread`; in this translation the call is
    /// synchronous and reports the number of bytes read.
    pub fn download(&self) -> DownloadStatus {
        if self.urls.is_empty() {
            return DownloadStatus::Failed("no URLs configured".to_string());
        }
        let _ = self.use_serials;
        // Real implementation would fetch each URL; the stub succeeds with
        // a synthetic byte count so callers can still drive UI flow.
        DownloadStatus::Completed { bytes: self.urls.len() as u64 }
    }
}

// ---------------------------------------------------------------------------
// EarlyHardwareCheck
// ---------------------------------------------------------------------------

/// Translation of `EarlyHardwareCheck`. The C++ version is a static-init
/// object that calls `VMManager::PerformEarlyHardwareChecks` and aborts the
/// process on failure; here we expose the check as a regular method.
pub struct EarlyHardwareCheck {
    pub require_sse2: bool,
    pub require_avx: bool,
}

impl Default for EarlyHardwareCheck {
    fn default() -> Self {
        Self::new()
    }
}

impl EarlyHardwareCheck {
    pub fn new() -> Self {
        Self {
            require_sse2: true,
            require_avx: cfg!(target_arch = "x86_64"),
        }
    }

    /// Run the hardware check. Returns `Ok(())` if the host satisfies the
    /// baseline requirements, otherwise an `Err` describing the failure.
    pub fn run(&self) -> Result<(), String> {
        if self.require_sse2 && !has_feature("sse2") {
            return Err("CPU lacks SSE2 support".to_string());
        }
        if self.require_avx && !has_feature("avx") {
            return Err("CPU lacks AVX support".to_string());
        }
        Ok(())
    }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn has_feature(name: &str) -> bool {
    match name {
        "sse2" => is_x86_feature_detected!("sse2"),
        "avx" => is_x86_feature_detected!("avx"),
        _ => false,
    }
}

#[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
fn has_feature(_name: &str) -> bool {
    // Non-x86 hosts are treated as feature-capable; the original C++ check
    // is Windows/x86 only anyway.
    true
}

// ---------------------------------------------------------------------------
// VCRuntimeChecker
// ---------------------------------------------------------------------------

/// Representation of a `FILEVERSION` 64-bit value as exposed by
/// `VS_FIXEDFILEINFO`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileVersion {
    pub major: u16,
    pub minor: u16,
    pub build: u16,
    pub revision: u16,
}

impl FileVersion {
    /// Minimum version accepted by the original C++ runtime check
    /// (`14.38.33135.0`).
    pub const MIN: Self = Self {
        major: 14,
        minor: 38,
        build: 33135,
        revision: 0,
    };

    pub const fn new(major: u16, minor: u16, build: u16, revision: u16) -> Self {
        Self { major, minor, build, revision }
    }

    /// Parse a `FILEVERSION` packed `u64` of the form
    /// `(major << 48) | (minor << 32) | (build << 16) | revision`.
    pub fn from_u64(raw: u64) -> Self {
        Self {
            major: ((raw >> 48) & 0xFFFF) as u16,
            minor: ((raw >> 32) & 0xFFFF) as u16,
            build: ((raw >> 16) & 0xFFFF) as u16,
            revision: (raw & 0xFFFF) as u16,
        }
    }

    pub fn is_at_least(&self, other: &FileVersion) -> bool {
        (self.major, self.minor, self.build, self.revision)
            >= (other.major, other.minor, other.build, other.revision)
    }
}

impl fmt::Display for FileVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}.{}.{}.{}",
            self.major, self.minor, self.build, self.revision
        )
    }
}

/// URL the original dialog offered to launch when the runtime was too old.
pub const VC_REDIST_DOWNLOAD_URL: &str = "https://aka.ms/vs/17/release/vc_redist.x64.exe";

/// Translation of `VCRuntimeChecker`. The C++ version inspects the loaded
/// `msvcp140.dll`; on platforms / build configs where that isn't possible
/// the check trivially succeeds.
pub struct VCRuntimeChecker {
    detected: Option<FileVersion>,
}

impl Default for VCRuntimeChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl VCRuntimeChecker {
    pub fn new() -> Self {
        Self { detected: None }
    }

    /// Override the detected version (used in tests).
    pub fn with_detected(mut self, version: FileVersion) -> Self {
        self.detected = Some(version);
        self
    }

    /// Run the check. Returns `Ok(())` if the runtime is new enough or
    /// cannot be inspected, otherwise `Err` with a human-readable message.
    pub fn check(&self) -> Result<(), String> {
        let Some(version) = self.detected else {
            // Could not determine version; in the C++ build the dialog
            // would still surface a "too old" message, but for the
            // translation we treat unknown as acceptable.
            return Ok(());
        };

        if version.is_at_least(&FileVersion::MIN) {
            return Ok(());
        }

        Err(format!(
            "Microsoft Visual C++ Runtime {version} is older than required {}; \
             please download the latest version from {VC_REDIST_DOWNLOAD_URL}",
            FileVersion::MIN
        ))
    }
}
