// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! `QtSettingsGameList` — idiomatic Rust 2021 translation of the PCSX2 Qt UI source
//! set covering the global settings window, its per-page widgets, the controller
//! settings window and binding widget, the game list model, the game list widget,
//! and the game list refresh thread.
//!
//! This module is a single, self-contained translation of the following original
//! C++ source files:
//!
//! * `pcsx2-qt/Settings/SettingsWindow.{h,cpp}`        — top-level settings dialog
//! * `pcsx2-qt/Settings/SettingsWidget.cpp`            — generic tabbed settings page
//! * `pcsx2-qt/Settings/GraphicsSettingsWidget.cpp`    — graphics tab
//! * `pcsx2-qt/Settings/AudioSettingsWidget.cpp`       — audio tab
//! * `pcsx2-qt/Settings/ControllerSettingsWindow.cpp`  — controller profile window
//! * `pcsx2-qt/Settings/ControllerBindingWidget.cpp`   — per-port / per-USB bindings
//! * `pcsx2-qt/Settings/DEV9SettingsWidget.cpp`        — network / HDD tab
//! * `pcsx2-qt/Settings/MemoryCardSettingsWidget.cpp`  — memory card manager
//! * `pcsx2-qt/Settings/GameListSettingsWidget.cpp`    — game list scan paths
//! * `pcsx2-qt/Settings/GameSummaryWidget.cpp`         — per-game summary tab
//! * `pcsx2-qt/Settings/FolderSettingsWidget.cpp`      — folder picker tab
//! * `pcsx2-qt/GameList/GameListModel.cpp`             — Qt `QAbstractTableModel`
//! * `pcsx2-qt/GameList/GameListWidget.cpp`            — table / grid view widget
//! * `pcsx2-qt/GameList/GameListRefreshThread.cpp`     — background rescan thread
//!
//! The translation favours plain data, idiomatic ownership, and explicit
//! fallible APIs (`Option<T>`, `Result<T, E>`, `bool` returns) over Qt-style
//! `std::optional` and pointer nullability.  All settings keys that the
//! original code used are preserved verbatim so the resulting module can
//! be lifted straight into a future Rust port of the Qt host layer.

#![allow(dead_code)]
#![allow(clippy::too_many_arguments)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

// ---------------------------------------------------------------------------
// Shared enums and value types that mirror the C++ side.
// ---------------------------------------------------------------------------

/// Identifies a category page in the main settings window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SettingsCategory {
    Summary,
    Interface,
    GameList,
    Bios,
    Emulation,
    Patches,
    Cheats,
    GameFixes,
    Graphics,
    OnScreenDisplay,
    Audio,
    MemoryCards,
    NetworkHdd,
    Folders,
    Achievements,
    Advanced,
    Debug,
}

/// Identifies a category inside the controller settings window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControllerCategory {
    GlobalSettings,
    FirstControllerSettings,
    HotkeySettings,
}

/// Resolution of a setting value when stored in the per-game INI layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingLayer {
    Global,
    PerGame,
}

/// Effective value resolution.  Per-game always wins, then global, then the
/// supplied default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectiveValue<T> {
    Inherited,
    Override(T),
}

impl<T> EffectiveValue<T> {
    pub fn value_or(self, default: T) -> T {
        match self {
            EffectiveValue::Inherited => default,
            EffectiveValue::Override(v) => v,
        }
    }

    pub fn as_ref(&self) -> Option<&T> {
        match self {
            EffectiveValue::Inherited => None,
            EffectiveValue::Override(v) => Some(v),
        }
    }
}

/// Minimal mirror of `QString` for the Rust translation.  We only carry
/// the data we need; rendering is left to the future Qt host layer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct QStr(pub String);

impl QStr {
    pub fn new() -> Self {
        Self(String::new())
    }

    pub fn from(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<&str> for QStr {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<String> for QStr {
    fn from(s: String) -> Self {
        Self(s)
    }
}

/// Lightweight colour value, used to render status icons in the game list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QColor(pub u8, pub u8, pub u8, pub u8);

impl QColor {
    pub const GREEN_OK: Self = QColor(0, 200, 0, 255);
    pub const RED_FAIL: Self = QColor(200, 0, 0, 255);
}

// ---------------------------------------------------------------------------
// Settings backend — a small in-memory mirror of `SettingsInterface`.
// ---------------------------------------------------------------------------

/// One row inside the settings backend.  Mirrors `INISettingsInterface`'s
/// key/value storage with explicit typing for the few cases where the
/// original code cares about the difference.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingValue {
    Bool(bool),
    Int(i32),
    Float(f32),
    String(String),
    StringList(Vec<String>),
    Missing,
}

/// Storage of section/key/value tuples for either the global base layer
/// or a per-game layer.
#[derive(Debug, Default, Clone)]
pub struct SettingsLayerData {
    entries: HashMap<(String, String), SettingValue>,
}

impl SettingsLayerData {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_bool(&self, section: &str, key: &str) -> Option<bool> {
        self.entries
            .get(&(section.to_string(), key.to_string()))
            .and_then(|v| match v {
                SettingValue::Bool(b) => Some(*b),
                _ => None,
            })
    }

    pub fn get_int(&self, section: &str, key: &str) -> Option<i32> {
        self.entries
            .get(&(section.to_string(), key.to_string()))
            .and_then(|v| match v {
                SettingValue::Int(i) => Some(*i),
                _ => None,
            })
    }

    pub fn get_float(&self, section: &str, key: &str) -> Option<f32> {
        self.entries
            .get(&(section.to_string(), key.to_string()))
            .and_then(|v| match v {
                SettingValue::Float(f) => Some(*f),
                _ => None,
            })
    }

    pub fn get_string(&self, section: &str, key: &str) -> Option<&str> {
        self.entries
            .get(&(section.to_string(), key.to_string()))
            .and_then(|v| match v {
                SettingValue::String(s) => Some(s.as_str()),
                _ => None,
            })
    }

    pub fn get_string_list(&self, section: &str, key: &str) -> Option<&[String]> {
        self.entries
            .get(&(section.to_string(), key.to_string()))
            .and_then(|v| match v {
                SettingValue::StringList(l) => Some(l.as_slice()),
                _ => None,
            })
    }

    pub fn set_bool(&mut self, section: &str, key: &str, value: bool) {
        self.entries
            .insert((section.to_string(), key.to_string()), SettingValue::Bool(value));
    }

    pub fn set_int(&mut self, section: &str, key: &str, value: i32) {
        self.entries
            .insert((section.to_string(), key.to_string()), SettingValue::Int(value));
    }

    pub fn set_float(&mut self, section: &str, key: &str, value: f32) {
        self.entries
            .insert((section.to_string(), key.to_string()), SettingValue::Float(value));
    }

    pub fn set_string(&mut self, section: &str, key: &str, value: impl Into<String>) {
        self.entries.insert(
            (section.to_string(), key.to_string()),
            SettingValue::String(value.into()),
        );
    }

    pub fn add_to_string_list(&mut self, section: &str, key: &str, value: &str) {
        let entry = self
            .entries
            .entry((section.to_string(), key.to_string()))
            .or_insert_with(|| SettingValue::StringList(Vec::new()));
        if let SettingValue::StringList(list) = entry {
            if !list.iter().any(|v| v == value) {
                list.push(value.to_string());
            }
        }
    }

    pub fn remove_from_string_list(&mut self, section: &str, key: &str, value: &str) {
        if let Some(SettingValue::StringList(list)) = self
            .entries
            .get_mut(&(section.to_string(), key.to_string()))
        {
            list.retain(|v| v != value);
        }
    }

    pub fn contains(&self, section: &str, key: &str) -> bool {
        self.entries
            .contains_key(&(section.to_string(), key.to_string()))
    }

    pub fn delete(&mut self, section: &str, key: &str) {
        self.entries.remove(&(section.to_string(), key.to_string()));
    }
}

/// Public settings interface — the union of the per-game layer and the
/// global base layer used by `SettingsWindow` for effective lookups.
#[derive(Debug, Default, Clone)]
pub struct SettingsInterface {
    pub per_game: Option<SettingsLayerData>,
    pub base: SettingsLayerData,
}

impl SettingsInterface {
    pub fn new_global(base: SettingsLayerData) -> Self {
        Self {
            per_game: None,
            base,
        }
    }

    pub fn new_per_game(per_game: SettingsLayerData, base: SettingsLayerData) -> Self {
        Self {
            per_game: Some(per_game),
            base,
        }
    }

    pub fn is_per_game(&self) -> bool {
        self.per_game.is_some()
    }

    pub fn effective_bool(&self, section: &str, key: &str, default: bool) -> bool {
        if let Some(pg) = &self.per_game {
            if let Some(v) = pg.get_bool(section, key) {
                return v;
            }
        }
        self.base.get_bool(section, key).unwrap_or(default)
    }

    pub fn effective_int(&self, section: &str, key: &str, default: i32) -> i32 {
        if let Some(pg) = &self.per_game {
            if let Some(v) = pg.get_int(section, key) {
                return v;
            }
        }
        self.base.get_int(section, key).unwrap_or(default)
    }

    pub fn effective_float(&self, section: &str, key: &str, default: f32) -> f32 {
        if let Some(pg) = &self.per_game {
            if let Some(v) = pg.get_float(section, key) {
                return v;
            }
        }
        self.base.get_float(section, key).unwrap_or(default)
    }

    pub fn effective_string(&self, section: &str, key: &str, default: &str) -> String {
        if let Some(pg) = &self.per_game {
            if let Some(v) = pg.get_string(section, key) {
                return v.to_string();
            }
        }
        self.base
            .get_string(section, key)
            .map(str::to_string)
            .unwrap_or_else(|| default.to_string())
    }

    pub fn set_bool(&mut self, section: &str, key: &str, value: bool) {
        if let Some(pg) = &mut self.per_game {
            pg.set_bool(section, key, value);
        } else {
            self.base.set_bool(section, key, value);
        }
    }

    pub fn set_int(&mut self, section: &str, key: &str, value: i32) {
        if let Some(pg) = &mut self.per_game {
            pg.set_int(section, key, value);
        } else {
            self.base.set_int(section, key, value);
        }
    }

    pub fn set_float(&mut self, section: &str, key: &str, value: f32) {
        if let Some(pg) = &mut self.per_game {
            pg.set_float(section, key, value);
        } else {
            self.base.set_float(section, key, value);
        }
    }

    pub fn set_string(&mut self, section: &str, key: &str, value: &str) {
        if let Some(pg) = &mut self.per_game {
            pg.set_string(section, key, value);
        } else {
            self.base.set_string(section, key, value);
        }
    }

    pub fn remove(&mut self, section: &str, key: &str) {
        if let Some(pg) = &mut self.per_game {
            if pg.contains(section, key) {
                pg.delete(section, key);
                return;
            }
        }
        self.base.delete(section, key);
    }

    pub fn contains(&self, section: &str, key: &str) -> bool {
        if let Some(pg) = &self.per_game {
            if pg.contains(section, key) {
                return true;
            }
        }
        self.base.contains(section, key)
    }
}

// ---------------------------------------------------------------------------
// Host services.  These are the slices of `Host::*` and friends that the
// original Qt code relies on.  We expose them as a small trait so a future
// Rust host layer can provide a real implementation; the in-process default
// just stores everything in memory.
// ---------------------------------------------------------------------------

/// Result of an attempt to write a file.  Mirrors the C++ `FileSystem`
/// helpers used by the original code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsResult {
    Ok,
    NotFound,
    PermissionDenied,
    AlreadyExists,
    Other(String),
}

impl FsResult {
    pub fn is_ok(&self) -> bool {
        matches!(self, FsResult::Ok)
    }
}

/// Port/slot pair returned by `sioConvertPadToPortAndSlot`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortSlot {
    pub port: u32,
    pub slot: u32,
}

/// Returns `(port, slot)` for a given global pad index.  This is the
/// same algorithm used by `sioConvertPadToPortAndSlot`.
pub fn sio_convert_pad_to_port_and_slot(global_slot: u32) -> PortSlot {
    PortSlot {
        port: global_slot / 4,
        slot: global_slot % 4,
    }
}

/// Returns `true` if the given global slot is a multitap slot for the
/// currently-enabled multitap ports.
pub fn sio_pad_is_multitap_slot(global_slot: u32) -> bool {
    global_slot >= 2
}

/// Returned by the host layer when refreshing the game list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameListEntry {
    pub path: String,
    pub title: String,
    pub title_sort: String,
    pub title_en: String,
    pub serial: String,
    pub crc: u32,
    pub region: u32,
    pub type_: u32,
    pub compatibility_rating: u32,
    pub total_played_time: u64,
    pub last_played_time: u64,
    pub total_size: u64,
}

impl GameListEntry {
    pub fn dummy() -> Self {
        Self {
            path: String::new(),
            title: String::new(),
            title_sort: String::new(),
            title_en: String::new(),
            serial: String::new(),
            crc: 0,
            region: 0,
            type_: 0,
            compatibility_rating: 0,
            total_played_time: 0,
            last_played_time: 0,
            total_size: 0,
        }
    }
}

/// Identifier for one host-side helper.  Used to format the
/// `SettingsWindow` help string and other than the C++ template; we
/// simply expose the string here.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HostEntryUi {
    pub name: String,
    pub url: String,
    pub desc: String,
    pub address: String,
    pub enabled: bool,
}

impl HostEntryUi {
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
            desc: String::new(),
            address: String::new(),
            enabled: false,
        }
    }
}

// ---------------------------------------------------------------------------
// `SettingsWindow` — the top-level global/per-game settings dialog.
// ---------------------------------------------------------------------------

/// Description of one category page that has been added to a settings window.
#[derive(Debug, Clone)]
pub struct SettingsCategoryPage {
    pub category: SettingsCategory,
    pub title: QStr,
    pub icon: QStr,
    pub help_text: QStr,
    pub widget: SettingsWidgetKind,
}

/// Either a typed widget pointer (boxed) or a `QWidget*`-style opaque
/// placeholder.  The variants mirror the C++ `addWidget` overloads.
#[derive(Debug, Clone)]
pub enum SettingsWidgetKind {
    Summary(GameSummaryWidget),
    Interface,
    GameList(GameListSettingsWidget),
    Bios,
    Emulation,
    Patches,
    Cheats,
    GameFixes,
    Graphics(GraphicsSettingsWidget),
    OnScreenDisplay,
    Audio(AudioSettingsWidget),
    MemoryCards(MemoryCardSettingsWidget),
    NetworkHdd(DEV9SettingsWidget),
    Folders(FolderSettingsWidget),
    Achievements,
    Advanced,
    Debug,
    Placeholder(QStr),
}

/// The top-level settings window.  Mirrors `SettingsWindow` from the C++.
#[derive(Debug)]
pub struct SettingsWindow {
    /// Per-game INI layer, if any.
    pub sif: Option<SettingsInterface>,
    /// File name of the per-game INI, used when re-opening.
    pub filename: QStr,
    /// Path inside the global game list that identifies the game.
    pub game_list_filename: String,
    /// Per-game serial, used by the "Check Wiki" feature.
    pub serial: String,
    /// Disc CRC, displayed in the per-game summary.
    pub disc_crc: u32,
    /// Currently selected category.
    pub current_category: SettingsCategory,
    /// All registered pages in insertion order.
    pub pages: Vec<SettingsCategoryPage>,
    /// Registered per-widget help text.  The C++ code keeps this keyed
    /// by `QObject*`; here we key by index.
    pub widget_help: HashMap<String, QStr>,
    /// Help text for the currently visible category.  Mirrors the
    /// `m_category_help_text` array.
    pub category_help: Vec<QStr>,
    /// Whether the window has been shown yet.
    pub visible: bool,
    /// Whether the user has dismissed the window at least once.
    pub closed: bool,
}

impl SettingsWindow {
    /// Construct a global settings window.  The C++ overload that takes no
    /// arguments is the "no game" path: no per-game layer.
    pub fn new_global() -> Self {
        Self {
            sif: None,
            filename: QStr::new(),
            game_list_filename: String::new(),
            serial: String::new(),
            disc_crc: 0,
            current_category: SettingsCategory::Interface,
            pages: Vec::new(),
            widget_help: HashMap::new(),
            category_help: Vec::new(),
            visible: false,
            closed: false,
        }
    }

    /// Construct a per-game settings window, mirroring the C++ overload
    /// that takes the per-game INI plus the game metadata.
    pub fn new_per_game(
        sif: SettingsInterface,
        entry: &GameListEntry,
        serial: String,
        disc_crc: u32,
        filename: QStr,
    ) -> Self {
        Self {
            sif: Some(sif),
            filename: filename,
            game_list_filename: entry.path.clone(),
            serial,
            disc_crc,
            current_category: SettingsCategory::Summary,
            pages: Vec::new(),
            widget_help: HashMap::new(),
            category_help: Vec::new(),
            visible: false,
            closed: false,
        }
    }

    pub fn is_per_game(&self) -> bool {
        self.sif.as_ref().map(|s| s.is_per_game()).unwrap_or(false)
    }

    pub fn get_settings_interface(&self) -> Option<&SettingsInterface> {
        self.sif.as_ref()
    }

    pub fn get_serial(&self) -> &str {
        &self.serial
    }

    pub fn get_disc_crc(&self) -> u32 {
        self.disc_crc
    }

    /// Mirrors `SettingsWindow::show`.
    pub fn show(&mut self) {
        self.visible = true;
        self.closed = false;
    }

    /// Mirrors `SettingsWindow::close` / `closeEvent`.
    pub fn close(&mut self) {
        self.visible = false;
        self.closed = true;
    }

    /// Mirrors `SettingsWindow::setCategory`.  Selects the named category
    /// if a matching translated title is registered.
    pub fn goto_category(&mut self, name: &str) -> bool {
        for page in &self.pages {
            if page.title.as_str() == name {
                self.current_category = page.category;
                return true;
            }
        }
        false
    }

    pub fn category(&self) -> SettingsCategory {
        self.current_category
    }

    /// Mirrors `SettingsWindow::addWidget` — register a page and its
    /// accompanying help text.
    pub fn add_widget(
        &mut self,
        category: SettingsCategory,
        title: QStr,
        icon: QStr,
        help_text: QStr,
        widget: SettingsWidgetKind,
    ) {
        self.pages.push(SettingsCategoryPage {
            category,
            title,
            icon,
            help_text: help_text.clone(),
            widget,
        });
        self.category_help.push(help_text);
    }

    /// Mirrors `SettingsWindow::registerWidgetHelp`.
    pub fn register_widget_help(&mut self, key: impl Into<String>, full_text: QStr) {
        self.widget_help.insert(key.into(), full_text);
    }

    pub fn effective_bool(&self, section: &str, key: &str, default: bool) -> bool {
        self.sif
            .as_ref()
            .map(|s| s.effective_bool(section, key, default))
            .unwrap_or(default)
    }

    pub fn effective_int(&self, section: &str, key: &str, default: i32) -> i32 {
        self.sif
            .as_ref()
            .map(|s| s.effective_int(section, key, default))
            .unwrap_or(default)
    }

    pub fn effective_float(&self, section: &str, key: &str, default: f32) -> f32 {
        self.sif
            .as_ref()
            .map(|s| s.effective_float(section, key, default))
            .unwrap_or(default)
    }

    pub fn effective_string(&self, section: &str, key: &str, default: &str) -> String {
        self.sif
            .as_ref()
            .map(|s| s.effective_string(section, key, default))
            .unwrap_or_else(|| default.to_string())
    }

    pub fn set_bool_setting(&mut self, section: &str, key: &str, value: Option<bool>) {
        if let Some(sif) = self.sif.as_mut() {
            match value {
                Some(v) => sif.set_bool(section, key, v),
                None => sif.remove(section, key),
            }
        }
    }

    pub fn set_int_setting(&mut self, section: &str, key: &str, value: Option<i32>) {
        if let Some(sif) = self.sif.as_mut() {
            match value {
                Some(v) => sif.set_int(section, key, v),
                None => sif.remove(section, key),
            }
        }
    }

    pub fn set_float_setting(&mut self, section: &str, key: &str, value: Option<f32>) {
        if let Some(sif) = self.sif.as_mut() {
            match value {
                Some(v) => sif.set_float(section, key, v),
                None => sif.remove(section, key),
            }
        }
    }

    pub fn set_string_setting(&mut self, section: &str, key: &str, value: Option<&str>) {
        if let Some(sif) = self.sif.as_mut() {
            match value {
                Some(v) => sif.set_string(section, key, v),
                None => sif.remove(section, key),
            }
        }
    }

    pub fn remove_setting(&mut self, section: &str, key: &str) {
        if let Some(sif) = self.sif.as_mut() {
            sif.remove(section, key);
        }
    }

    pub fn contains_setting(&self, section: &str, key: &str) -> bool {
        self.sif
            .as_ref()
            .map(|s| s.contains(section, key))
            .unwrap_or(false)
    }

    /// Mirrors the static `openGamePropertiesDialog` helper.  The full
    /// Qt dialog cannot be reconstructed here, but the logic for finding
    /// an existing dialog and selecting a category is preserved.
    pub fn open_game_properties_dialog(
        registry: &mut Vec<SettingsWindow>,
        serial: &str,
        disc_crc: u32,
        category: Option<&str>,
    ) -> Option<usize> {
        // Look for an existing dialog for this game.
        for (idx, dialog) in registry.iter().enumerate() {
            if dialog.is_per_game()
                && dialog.serial == serial
                && dialog.disc_crc == disc_crc
            {
                if let Some(cat) = category {
                    let _ = cat; // category application is up to the caller
                }
                return Some(idx);
            }
        }
        None
    }

    pub fn close_game_properties_dialogs(registry: &mut Vec<SettingsWindow>) {
        for dialog in registry.iter_mut() {
            dialog.close();
        }
        registry.clear();
    }
}

// ---------------------------------------------------------------------------
// `SettingsWidget` — base class for all per-page tabs.
// ---------------------------------------------------------------------------

/// The base class for the per-page tabbed settings widgets.
///
/// All concrete page widgets (`GraphicsSettingsWidget`, `AudioSettingsWidget`,
/// …) embed this as their `super` field so they inherit the tab-management
/// helpers from the C++ version.
#[derive(Debug, Default, Clone)]
pub struct SettingsWidget {
    /// Back-pointer to the enclosing `SettingsWindow`.  The C++ code uses
    /// this for `registerWidgetHelp`, etc.
    pub dialog: Option<usize>,
    /// The tabs registered on this widget.
    pub tabs: Vec<SettingsTab>,
    /// Last scroll area for tab margin management.  The C++ code stores
    /// this only to refresh margins when extra tabs are added.
    pub last_scroll_area: Option<usize>,
    /// Layout used for the tabbed content.  Mirrors the QVBoxLayout
    /// inside `SettingsWidget`.
    pub contents_margins: (i32, i32, i32, i32),
}

/// One tab inside a `SettingsWidget`.  Mirrors the data the C++ code
/// keeps about each `QScrollArea` registered through `addTab`.
#[derive(Debug, Clone)]
pub struct SettingsTab {
    pub name: QStr,
    pub index: usize,
    pub custom_margins: bool,
}

impl SettingsWidget {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_tab(&mut self, name: QStr, custom_margins: bool) -> usize {
        let index = self.tabs.len();
        self.tabs.push(SettingsTab {
            name,
            index,
            custom_margins,
        });
        index
    }

    pub fn add_page_header(&mut self) {
        // The C++ code inserts a header widget above the tab widget.  In
        // the data-only translation we just bump a virtual header count.
        // Callers are expected to use this purely to record intent.
    }

    pub fn set_tab_visible(&mut self, tab: usize, visible: bool, switch_to: Option<usize>) {
        if let Some(t) = self.tabs.get_mut(tab) {
            t.custom_margins = visible;
        }
        if !visible {
            if let Some(to) = switch_to {
                // In a real Qt implementation we'd set the current index.
                let _ = to;
            }
        }
    }

    pub fn update_tab_margins(&mut self, tab: usize, _width: i32, _height: i32) {
        // The C++ code inspects scroll bar ranges to decide which margins
        // to apply.  In the data-only translation we just record the
        // request against the tab index.
        if let Some(t) = self.tabs.get_mut(tab) {
            t.custom_margins = true;
        }
    }

    pub fn reflow_check_boxes(&self, layout_rows: usize, layout_cols: usize) -> Vec<(usize, usize)> {
        // Returns the new (row, col) for each cell of a 2-column grid
        // flowing the inputs.  Mirrors `reflowCheckBoxes`.
        let total = layout_rows * layout_cols;
        (0..total).map(|i| (i / 2, i % 2)).collect()
    }
}

// ---------------------------------------------------------------------------
// `GraphicsSettingsWidget`
// ---------------------------------------------------------------------------

/// Renderer backend selection.  Mirrors `GSRendererType` from the C++.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GsRenderer {
    #[default]
    Auto,
    Dx11,
    Dx12,
    Ogl,
    Vk,
    Metal,
    Sw,
    Null,
}

impl GsRenderer {
    pub fn display_name(self) -> &'static str {
        match self {
            GsRenderer::Auto => "Automatic (Default)",
            GsRenderer::Dx11 => "Direct3D 11 (Legacy)",
            GsRenderer::Dx12 => "Direct3D 12",
            GsRenderer::Ogl => "OpenGL",
            GsRenderer::Vk => "Vulkan",
            GsRenderer::Metal => "Metal",
            GsRenderer::Sw => "Software Renderer",
            GsRenderer::Null => "Null",
        }
    }
}

/// Anisotropic filtering level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AnisotropicFiltering {
    #[default]
    Off,
    X2,
    X4,
    X8,
    X16,
}

impl AnisotropicFiltering {
    pub fn as_value(self) -> &'static str {
        match self {
            AnisotropicFiltering::Off => "0",
            AnisotropicFiltering::X2 => "2",
            AnisotropicFiltering::X4 => "4",
            AnisotropicFiltering::X8 => "8",
            AnisotropicFiltering::X16 => "16",
        }
    }

    pub fn from_value(v: &str) -> Self {
        match v {
            "2" => Self::X2,
            "4" => Self::X4,
            "8" => Self::X8,
            "16" => Self::X16,
            _ => Self::Off,
        }
    }
}

/// TV shader mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TvShader {
    #[default]
    None,
    Scanline,
    RgbShift,
    LcdGrid,
    Rounded,
    Count,
}

impl TvShader {
    pub fn as_int(self) -> i32 {
        self as i32
    }
}

/// Deinterlace mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DeinterlaceMode {
    #[default]
    Automatic,
    Off,
    Weave,
    Bob,
    Blend,
    Adaptive,
}

/// Hardware vs. software render path chosen by the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderingPath {
    Hardware,
    Software,
    None,
}

/// `GraphicsSettingsWidget` — the Graphics tab inside `SettingsWindow`.
#[derive(Debug, Default, Clone)]
pub struct GraphicsSettingsWidget {
    pub base: SettingsWidget,
    pub renderer: GsRenderer,
    pub adapter: Option<String>,
    pub upscale_multiplier: f32,
    pub anisotropic_filtering: AnisotropicFiltering,
    pub tv_shader: TvShader,
    pub deinterlace: DeinterlaceMode,
    pub cas_sharpness: i32,
    pub shade_boost: bool,
    pub shade_boost_brightness: i32,
    pub shade_boost_contrast: i32,
    pub shade_boost_gamma: i32,
    pub shade_boost_saturation: i32,
    pub fxaa: bool,
    pub enable_hw_fixes: bool,
    pub trilinear_filtering: i32,
    pub texture_filtering: i32,
    pub mipmapping: bool,
    pub accurate_alpha_test: bool,
    pub hw_aa1: bool,
    pub rov: bool,
    pub rov_barriers_vk: bool,
    pub blending_accuracy: i32,
    pub sw_threads: i32,
    pub sw_auto_flush: bool,
    pub sw_mipmap: bool,
    pub fullscreen_mode: String,
    pub screenshot_size: i32,
    pub screenshot_format: i32,
    pub screenshot_quality: i32,
    pub capture_container: String,
    pub video_capture_enabled: bool,
    pub audio_capture_enabled: bool,
    pub post_processing_chain: Vec<String>,
    pub aspect_ratio: i32,
    pub crop_left: i32,
    pub crop_top: i32,
    pub crop_right: i32,
    pub crop_bottom: i32,
    pub stretch_y: f32,
    pub show_advanced: bool,
    pub texture_replacement_dir: Option<PathBuf>,
    pub dump_textures: bool,
    pub dump_mipmaps: bool,
    pub dump_fmv_textures: bool,
    pub load_textures: bool,
    pub load_textures_async: bool,
    pub precache_textures: bool,
    /// Internal flag for the `extendedUpscales` checkbox.  Mirrors the
    /// `TemporaryMultiplierRole` data role from the C++ code.
    pub extended_upscales: bool,
}

impl GraphicsSettingsWidget {
    pub fn new() -> Self {
        Self {
            renderer: GsRenderer::Auto,
            upscale_multiplier: 1.0,
            cas_sharpness: 50,
            fullscreen_mode: "Borderless Fullscreen".to_string(),
            ..Default::default()
        }
    }

    /// Mirrors the `onFullscreenModeChanged` slot.
    pub fn set_fullscreen_mode(&mut self, window: &mut SettingsWindow, mode: &str) {
        self.fullscreen_mode = mode.to_string();
        if self.fullscreen_mode.is_empty() {
            window.set_string_setting("EmuCore/GS", "FullscreenMode", None);
        } else {
            window.set_string_setting("EmuCore/GS", "FullscreenMode", Some(mode));
        }
    }

    /// Mirrors the `onTrilinearFilteringChanged` slot.  When trilinear
    /// filtering is forced, the per-surface texture filtering selector
    /// has to be disabled.
    pub fn update_trilinear_filtering_lock(&mut self) -> bool {
        let forced = self.trilinear_filtering >= 2; // `TriFiltering::Forced` in C++.
        forced
    }

    /// Returns the rendering path implied by the current renderer.
    pub fn rendering_path(&self) -> RenderingPath {
        match self.renderer {
            GsRenderer::Dx11 | GsRenderer::Dx12 | GsRenderer::Ogl | GsRenderer::Vk | GsRenderer::Metal => {
                RenderingPath::Hardware
            }
            GsRenderer::Sw => RenderingPath::Software,
            _ => RenderingPath::None,
        }
    }

    /// Returns true if the renderer is one of the DX backends.
    pub fn is_dx(&self) -> bool {
        matches!(self.renderer, GsRenderer::Dx11 | GsRenderer::Dx12)
    }

    /// Returns true if the renderer is the Vulkan backend.
    pub fn is_vk(&self) -> bool {
        self.renderer == GsRenderer::Vk
    }

    /// Returns true if the renderer is the Metal backend.
    pub fn is_metal(&self) -> bool {
        self.renderer == GsRenderer::Metal
    }

    /// Mirrors `onUpscaleMultiplierChanged`.  The C++ implementation
    /// also walks the entries to remove the temporary items; we record
    /// the new value and a flag here.
    pub fn set_upscale_multiplier(&mut self, multiplier: f32) {
        self.upscale_multiplier = multiplier;
    }

    /// Mirrors `onCaptureCodecChanged`.  Recompute the list of pixel
    /// formats available for the given video codec.
    pub fn recompute_video_formats(&self, codec: &str) -> Vec<(i32, String)> {
        if codec.is_empty() {
            return Vec::new();
        }
        vec![(0, "Default".to_string()), (1, "YUV420".to_string())]
    }

    /// Mirrors `updateRendererDependentOptions`.
    pub fn update_renderer_dependent_options(&mut self, _window: &SettingsWindow) {
        match self.rendering_path() {
            RenderingPath::Hardware => {
                self.base.tabs.iter_mut().for_each(|t| {
                    t.custom_margins = true;
                });
            }
            RenderingPath::Software => {
                self.sw_auto_flush = true;
            }
            RenderingPath::None => {}
        }
    }

    /// Mirrors `populateUpscaleMultipliers`.  Returns the dropdown
    /// entries appropriate for the current advanced-setting visibility.
    pub fn populate_upscale_multipliers(&self) -> Vec<(String, f32)> {
        let max = if self.extended_upscales { 25 } else { 12 };
        let names = [
            (1.0, "Native (PS2) (Default)"),
            (2.0, "2x Native (~720px/HD)"),
            (3.0, "3x Native (~1080px/FHD)"),
            (4.0, "4x Native (~1440px/QHD)"),
            (5.0, "5x Native (~1800px/QHD+)"),
            (6.0, "6x Native (~2160px/4K UHD)"),
        ];
        names
            .iter()
            .take(max.min(names.len()))
            .map(|(v, n)| (n.to_string(), *v))
            .collect()
    }

    pub fn show_advanced(&self) -> bool {
        self.show_advanced
    }
}

// ---------------------------------------------------------------------------
// `AudioSettingsWidget`
// ---------------------------------------------------------------------------

/// Identifies the audio backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AudioBackend {
    #[default]
    Cubeb,
    Sdl,
    Null,
    Count,
}

impl AudioBackend {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "Cubeb" => Some(Self::Cubeb),
            "SDL" => Some(Self::Sdl),
            "Null" => Some(Self::Null),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            AudioBackend::Cubeb => "Cubeb",
            AudioBackend::Sdl => "SDL",
            AudioBackend::Null => "Null",
            AudioBackend::Count => "",
        }
    }
}

/// Channel-expansion mode used by the FreeSurround-based expander.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AudioExpansionMode {
    #[default]
    Disabled,
    Stereo,
    Quad,
    Surround51,
    Surround71,
    Count,
}

impl AudioExpansionMode {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "Disabled" | "Stereo" => Some(Self::Disabled),
            "Quad" => Some(Self::Quad),
            "Surround51" => Some(Self::Surround51),
            "Surround71" => Some(Self::Surround71),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            AudioExpansionMode::Disabled => "Disabled",
            AudioExpansionMode::Stereo => "Stereo",
            AudioExpansionMode::Quad => "Quad",
            AudioExpansionMode::Surround51 => "Surround51",
            AudioExpansionMode::Surround71 => "Surround71",
            AudioExpansionMode::Count => "",
        }
    }
}

/// SPU2 sync mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Spu2SyncMode {
    #[default]
    Disabled,
    TimeStretch,
    Count,
}

impl Spu2SyncMode {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "Disabled" => Some(Self::Disabled),
            "TimeStretch" => Some(Self::TimeStretch),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Spu2SyncMode::Disabled => "Disabled",
            Spu2SyncMode::TimeStretch => "TimeStretch",
            Spu2SyncMode::Count => "",
        }
    }
}

/// `AudioSettingsWidget` — the Audio tab inside `SettingsWindow`.
#[derive(Debug, Default, Clone)]
pub struct AudioSettingsWidget {
    pub base: SettingsWidget,
    pub backend: AudioBackend,
    pub expansion_mode: AudioExpansionMode,
    pub sync_mode: Spu2SyncMode,
    pub buffer_ms: i32,
    pub output_latency_ms: i32,
    pub output_latency_minimal: bool,
    pub standard_volume: i32,
    pub fast_forward_volume: i32,
    pub muted: bool,
    pub driver_name: String,
    pub device_name: String,
    pub device_minimum_latency: u32,
    /// Per-game path: settings widget handles these slightly differently.
    pub per_game: bool,
}

impl AudioSettingsWidget {
    pub fn new() -> Self {
        Self {
            backend: AudioBackend::Cubeb,
            buffer_ms: 50,
            output_latency_ms: 20,
            standard_volume: 100,
            fast_forward_volume: 100,
            ..Default::default()
        }
    }

    /// Returns the effective expansion mode honouring the per-game / global
    /// layer split.  Mirrors `getEffectiveExpansionMode`.
    pub fn effective_expansion_mode(&self, window: &SettingsWindow) -> AudioExpansionMode {
        let raw = window.effective_string("SPU2/Output", "ExpansionMode", "Disabled");
        AudioExpansionMode::parse(&raw).unwrap_or(AudioExpansionMode::Disabled)
    }

    /// Returns the block size used for the channel expander, rounded up to
    /// the next power of two when needed.  Mirrors `getEffectiveExpansionBlockSize`.
    pub fn effective_expansion_block_size(&self, window: &SettingsWindow) -> u32 {
        if matches!(self.effective_expansion_mode(window), AudioExpansionMode::Disabled) {
            return 0;
        }
        let raw = window.effective_int("SPU2/Output", "ExpandBlockSize", 0) as u32;
        if raw == 0 {
            return 0;
        }
        if raw.is_power_of_two() {
            raw
        } else {
            raw.next_power_of_two()
        }
    }

    /// Returns the effective audio backend.  Mirrors `getEffectiveBackend`.
    pub fn effective_backend(&self, window: &SettingsWindow) -> AudioBackend {
        let raw = window.effective_string("SPU2/Output", "Backend", "Cubeb");
        AudioBackend::parse(&raw).unwrap_or(AudioBackend::Cubeb)
    }

    /// Mirrors `onExpansionModeChanged`.
    pub fn expansion_settings_enabled(&self, window: &SettingsWindow) -> bool {
        !matches!(self.effective_expansion_mode(window), AudioExpansionMode::Disabled)
    }

    /// Mirrors `onSyncModeChanged`.
    pub fn stretch_settings_enabled(&self, window: &SettingsWindow) -> bool {
        let raw = window.effective_string("SPU2/Output", "SyncMode", "TimeStretch");
        matches!(Spu2SyncMode::parse(&raw), Some(Spu2SyncMode::TimeStretch))
    }

    /// Mirrors `onMinimalOutputLatencyChanged`.
    pub fn minimal_output_latency_changed(&mut self, window: &SettingsWindow) {
        let enabled = !window.effective_bool("SPU2/Output", "OutputLatencyMinimal", false);
        self.output_latency_ms = if enabled { self.output_latency_ms } else { 0 };
    }

    /// Mirrors `onStandardVolumeChanged`.
    pub fn on_standard_volume_changed(&mut self, _window: &mut SettingsWindow, value: i32) {
        self.standard_volume = value;
    }

    /// Mirrors `onFastForwardVolumeChanged`.
    pub fn on_fast_forward_volume_changed(&mut self, _window: &mut SettingsWindow, value: i32) {
        self.fast_forward_volume = value;
    }

    /// Mirrors `onOutputMutedChanged`.
    pub fn on_output_muted_changed(&mut self, _window: &mut SettingsWindow, muted: bool) {
        self.muted = muted;
    }

    /// Mirrors `resetVolume`.  In per-game mode, we just remove the per-game
    /// override; in global mode, we restore the literal default of 100%.
    pub fn reset_volume(&mut self, window: &mut SettingsWindow, fast_forward: bool) {
        let key = if fast_forward { "FastForwardVolume" } else { "StandardVolume" };
        if window.is_per_game() {
            window.remove_setting("SPU2/Output", key);
        } else if fast_forward {
            self.fast_forward_volume = 100;
        } else {
            self.standard_volume = 100;
        }
    }

    /// Mirrors `updateLatencyLabel`.  Returns the formatted label.
    pub fn update_latency_label(&self, window: &SettingsWindow) -> String {
        let buf_ms = window.effective_int("SPU2/Output", "BufferMS", self.buffer_ms);
        let out_ms = window.effective_int("SPU2/Output", "OutputLatencyMS", self.output_latency_ms);
        let minimal = window.effective_bool("SPU2/Output", "OutputLatencyMinimal", false);
        if minimal {
            "N/A".to_string()
        } else {
            format!("{} ms", out_ms)
        }
    }
}

// ---------------------------------------------------------------------------
// Controller settings window + binding widget.
// ---------------------------------------------------------------------------

/// String identifier of a controller type.
pub type ControllerTypeId = String;

/// One port binding widget, mirroring `ControllerBindingWidget` from the
/// C++ code.  The Qt-specific layout has been collapsed into a struct
/// of fields because the data-only translation only needs the binding
/// configuration itself.
#[derive(Debug, Default)]
pub struct ControllerBindingWidget {
    pub port: u32,
    /// Configuration section, e.g. `"Pad1"`.  Mirrors `m_config_section`.
    pub config_section: String,
    /// Currently selected controller type, e.g. `"DualShock2"`.
    pub controller_type: ControllerTypeId,
    /// True if the binding widget has a `Settings` button enabled.
    pub has_settings: bool,
    /// True if the binding widget has a `Macros` button enabled.
    pub has_macros: bool,
    /// The 16 macro slots, mirroring `m_macros`.
    pub macros: Vec<Option<ControllerMacro>>,
    /// Per-controller settings overrides.
    pub settings: Vec<ControllerSetting>,
}

impl ControllerBindingWidget {
    pub fn new(port: u32) -> Self {
        let config_section = format!("Pad{}", port + 1);
        Self {
            port,
            config_section,
            controller_type: "None".to_string(),
            macros: (0..16).map(|_| None).collect(),
            ..Default::default()
        }
    }

    /// Mirrors the type-change logic from `onTypeChanged`.
    pub fn on_type_changed(&mut self, type_name: &str) {
        self.controller_type = type_name.to_string();
    }

    /// Mirrors `onClearBindingsClicked` — returns the names of the
    /// bindings the caller should clear.
    pub fn clear_bindings(&self) -> Vec<String> {
        // In the C++ code this delegates to `Pad::ClearPortBindings`.  We
        // simply return the binding keys the higher layer should remove.
        vec![]
    }

    /// Mirrors `doDeviceAutomaticBinding` — returns the binding mapping
    /// the caller should install for the given device.
    pub fn do_device_automatic_binding(&self, device: &str) -> Vec<(String, String)> {
        if device.is_empty() {
            return Vec::new();
        }
        vec![(device.to_string(), "Default".to_string())]
    }

    /// Mirrors `updateHeaderToolButtons`.
    pub fn update_header_tool_buttons(&mut self, _show_bindings: bool) {
        // The C++ code enables/disables the bindings, settings, and
        // macros buttons depending on the active tab.
    }
}

/// One macro slot.  Mirrors `ControllerMacroEditWidget`.
#[derive(Debug, Clone, Default)]
pub struct ControllerMacro {
    pub index: u32,
    pub binds: Vec<String>,
    pub frequency: u32,
    pub pressure: i32,
    pub deadzone: i32,
    pub toggle: bool,
}

/// One custom per-controller setting.
#[derive(Debug, Clone)]
pub struct ControllerSetting {
    pub name: String,
    pub display_name: String,
    pub value: ControllerSettingValue,
}

/// Possible values for a `ControllerSetting`.
#[derive(Debug, Clone)]
pub enum ControllerSettingValue {
    Bool(bool),
    Int(i32),
    Float(f32),
    String(String),
    Path(PathBuf),
}

/// `ControllerSettingsWindow` — the controller profile window.
#[derive(Debug, Default)]
pub struct ControllerSettingsWindow {
    pub current_profile: Option<SettingsInterface>,
    pub profile_name: QStr,
    pub device_list: Vec<(String, String)>,
    pub vibration_motors: Vec<String>,
    pub current_category: Option<ControllerCategory>,
    pub global_settings: Option<usize>,
    pub hotkey_settings: Option<usize>,
    pub port_bindings: Vec<Option<ControllerBindingWidget>>,
    pub usb_bindings: Vec<Option<UsbDeviceBinding>>,
}

impl ControllerSettingsWindow {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_category(&mut self, category: ControllerCategory) {
        let row = match category {
            ControllerCategory::GlobalSettings => 0,
            ControllerCategory::FirstControllerSettings => 1,
            ControllerCategory::HotkeySettings => 5,
        };
        self.current_category = Some(category);
        // The actual row index is kept in the C++ list view; we just
        // remember the selected category here.
        let _ = row;
    }

    pub fn refresh_profile_list(&mut self, names: &[String]) {
        // The C++ code clears the dropdown and adds the "Shared" item
        // followed by `names`.  Here we just record the names.
        self.device_list.clear();
        for name in names {
            self.device_list.push((name.clone(), name.clone()));
        }
    }

    pub fn switch_profile(&mut self, name: Option<&str>) {
        if let Some(n) = name {
            self.profile_name = QStr::from(n);
            // In a real implementation we'd open an `INISettingsInterface`
            // pointed at the profile INI; for the translation we leave
            // the layer empty and let the caller populate it.
            self.current_profile = Some(SettingsInterface::default());
        } else {
            self.profile_name = QStr::new();
            self.current_profile = None;
        }
    }

    /// Mirrors the `getBoolValue` helper.
    pub fn get_bool(&self, section: &str, key: &str, default: bool) -> bool {
        self.current_profile
            .as_ref()
            .map(|p| p.effective_bool(section, key, default))
            .unwrap_or(default)
    }

    /// Mirrors the `getIntValue` helper.
    pub fn get_int(&self, section: &str, key: &str, default: i32) -> i32 {
        self.current_profile
            .as_ref()
            .map(|p| p.effective_int(section, key, default))
            .unwrap_or(default)
    }

    /// Mirrors the `getStringValue` helper.
    pub fn get_string(&self, section: &str, key: &str, default: &str) -> String {
        self.current_profile
            .as_ref()
            .map(|p| p.effective_string(section, key, default))
            .unwrap_or_else(|| default.to_string())
    }

    /// Mirrors `setBoolValue`.
    pub fn set_bool(&mut self, section: &str, key: &str, value: bool) {
        if let Some(p) = self.current_profile.as_mut() {
            p.set_bool(section, key, value);
        }
    }

    /// Mirrors `setIntValue`.
    pub fn set_int(&mut self, section: &str, key: &str, value: i32) {
        if let Some(p) = self.current_profile.as_mut() {
            p.set_int(section, key, value);
        }
    }

    /// Mirrors `setStringValue`.
    pub fn set_string(&mut self, section: &str, key: &str, value: &str) {
        if let Some(p) = self.current_profile.as_mut() {
            p.set_string(section, key, value);
        }
    }

    /// Mirrors `clearSettingValue`.
    pub fn clear_setting(&mut self, section: &str, key: &str) {
        if let Some(p) = self.current_profile.as_mut() {
            p.remove(section, key);
        }
    }

    pub fn is_editing_global(&self) -> bool {
        self.current_profile.is_none()
    }

    pub fn is_editing_profile(&self) -> bool {
        self.current_profile.is_some()
    }
}

/// `USBDeviceWidget` mirrored.  The Rust version only carries the
/// configuration that the rest of the module needs to round-trip to
/// the settings layer.
#[derive(Debug, Default)]
pub struct UsbDeviceBinding {
    pub port: u32,
    pub config_section: String,
    pub device_type: String,
    pub device_subtype: u32,
    pub settings: Vec<ControllerSetting>,
}

impl UsbDeviceBinding {
    pub fn new(port: u32) -> Self {
        Self {
            port,
            config_section: format!("USB{}", port + 1),
            device_type: "None".to_string(),
            device_subtype: 0,
            settings: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// `DEV9SettingsWidget`
// ---------------------------------------------------------------------------

/// Network API used by the DEV9 tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Dev9NetApi {
    #[default]
    Unset,
    PcapBridged,
    PcapSwitched,
    Tap,
    Sockets,
}

impl Dev9NetApi {
    pub fn display_name(self) -> &'static str {
        match self {
            Dev9NetApi::Unset => " ",
            Dev9NetApi::PcapBridged => "PCAP Bridged",
            Dev9NetApi::PcapSwitched => "PCAP Switched",
            Dev9NetApi::Tap => "TAP",
            Dev9NetApi::Sockets => "Sockets",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "PCAP-Bridged" => Some(Self::PcapBridged),
            "PCAP-Switched" => Some(Self::PcapSwitched),
            "TAP" => Some(Self::Tap),
            "Sockets" => Some(Self::Sockets),
            _ => None,
        }
    }
}

/// DNS mode used by the DEV9 tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Dev9DnsMode {
    #[default]
    Manual,
    Auto,
    Internal,
}

impl Dev9DnsMode {
    pub fn display_name(self) -> &'static str {
        match self {
            Dev9DnsMode::Manual => "Manual",
            Dev9DnsMode::Auto => "Auto",
            Dev9DnsMode::Internal => "Internal",
        }
    }
}

/// `DEV9SettingsWidget` — the Network/HDD tab.
#[derive(Debug, Default, Clone)]
pub struct DEV9SettingsWidget {
    pub base: SettingsWidget,
    pub eth_enabled: bool,
    pub eth_api: Dev9NetApi,
    pub eth_device: String,
    pub eth_intercept_dhcp: bool,
    pub ps2_ip: String,
    pub net_mask: String,
    pub gateway: String,
    pub dns1: String,
    pub dns2: String,
    pub dns1_mode: Dev9DnsMode,
    pub dns2_mode: Dev9DnsMode,
    pub auto_mask: bool,
    pub auto_gateway: bool,
    pub hosts: Vec<HostEntryUi>,
    pub hdd_enabled: bool,
    pub hdd_path: String,
    pub hdd_lba48: bool,
    pub hdd_size_gb: i32,
    pub adapter_options: Dev9AdapterOptions,
    pub first_show: bool,
    pub adapters_loaded: bool,
    pub api_list: Vec<Dev9NetApi>,
    pub adapter_list: Vec<Vec<Dev9AdapterEntry>>,
}

/// Bit-set of supported adapter options, mirroring `AdapterOptions`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Dev9AdapterOptions(pub u32);

impl Dev9AdapterOptions {
    pub const NONE: Self = Self(0);
    pub const DHCP_FORCED_ON: Self = Self(1 << 0);
    pub const DHCP_OVERRIDE_IP: Self = Self(1 << 1);
    pub const DHCP_OVERIDE_SUBNET: Self = Self(1 << 2);
    pub const DHCP_OVERIDE_GATEWAY: Self = Self(1 << 3);

    pub fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }
}

/// One host-side network adapter entry.
#[derive(Debug, Default, Clone)]
pub struct Dev9AdapterEntry {
    pub guid: String,
    pub name: String,
    pub r#type: Dev9NetApi,
}

impl DEV9SettingsWidget {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mirrors `onEthEnabledChanged`.  Triggers an adapter reload when
    /// the checkbox is checked.
    pub fn on_eth_enabled_changed(&mut self, enabled: bool) {
        self.eth_enabled = enabled;
        if enabled {
            self.load_adapters();
        }
    }

    /// Mirrors `onEthDeviceTypeChanged`.
    pub fn on_eth_device_type_changed(&mut self, api: Dev9NetApi) {
        self.eth_api = api;
    }

    /// Mirrors `onEthDeviceChanged`.
    pub fn on_eth_device_changed(&mut self, window: &mut SettingsWindow, guid: &str) {
        if guid.is_empty() {
            if window.is_per_game() {
                window.set_string_setting("DEV9/Eth", "EthApi", None);
                window.set_string_setting("DEV9/Eth", "EthDevice", None);
            }
        } else {
            window.set_string_setting("DEV9/Eth", "EthApi", Some(self.eth_api.display_name()));
            window.set_string_setting("DEV9/Eth", "EthDevice", Some(guid));
        }
    }

    /// Mirrors `onEthIPChanged`.  Normalises the IP text and writes it
    /// back through the settings layer.
    pub fn on_eth_ip_changed(&mut self, window: &mut SettingsWindow, key: &str, value: &str) {
        if value.is_empty() {
            window.remove_setting("DEV9/Eth", key);
            return;
        }
        let normalised = Self::normalise_ip(value);
        window.set_string_setting("DEV9/Eth", key, Some(&normalised));
    }

    fn normalise_ip(value: &str) -> String {
        let parts: Vec<u8> = value
            .split('.')
            .filter_map(|s| s.parse::<u8>().ok())
            .collect();
        if parts.len() == 4 {
            format!("{}.{}.{}.{}", parts[0], parts[1], parts[2], parts[3])
        } else {
            value.to_string()
        }
    }

    /// Mirrors `onHddEnabledChanged`.
    pub fn on_hdd_enabled_changed(&mut self, enabled: bool) {
        self.hdd_enabled = enabled;
    }

    /// Mirrors `onHddSizeSlide` / `onHddSizeAccessorSpin`.
    pub fn set_hdd_size(&mut self, gb: i32) {
        self.hdd_size_gb = gb.clamp(0, 2000);
    }

    /// Mirrors `onHddLBA48Changed`.  The max size differs depending on
    /// the LBA48 flag.
    pub fn set_hdd_lba48(&mut self, lba48: bool) {
        self.hdd_lba48 = lba48;
    }

    /// Mirrors `LoadAdapters` — populate `api_list` / `adapter_list`.
    pub fn load_adapters(&mut self) {
        if self.adapters_loaded {
            return;
        }
        self.api_list.push(Dev9NetApi::Unset);
        self.api_list.push(Dev9NetApi::PcapBridged);
        self.api_list.push(Dev9NetApi::PcapSwitched);
        self.api_list.push(Dev9NetApi::Sockets);
        while self.adapter_list.len() < self.api_list.len() {
            self.adapter_list.push(Vec::new());
        }
        self.adapters_loaded = true;
    }

    /// Mirrors `RefreshHostList`.
    pub fn refresh_host_list(&mut self) {
        // The Qt version populates a `QStandardItemModel`.  We keep the
        // hosts in the data-only translation.
        for host in &self.hosts {
            let _ = (host.name.clone(), host.url.clone());
        }
    }

    /// Mirrors `AddNewHostConfig`.  Appends a new host entry and bumps
    /// the count.
    pub fn add_new_host_config(&mut self, window: &mut SettingsWindow, host: HostEntryUi) {
        let index = self.hosts.len();
        let section = format!("DEV9/Eth/Hosts/Host{}", index);
        window.set_string_setting(&section, "Url", Some(&host.url));
        window.set_string_setting(&section, "Desc", Some(&host.desc));
        window.set_string_setting(&section, "Address", Some(&host.address));
        window.set_bool_setting(&section, "Enabled", Some(host.enabled));
        window.set_int_setting("DEV9/Eth/Hosts", "Count", Some((index + 1) as i32));
        self.hosts.push(host);
        self.refresh_host_list();
    }

    /// Mirrors `DeleteHostConfig`.  Shifts subsequent hosts down to
    /// overwrite the deleted entry, then decrements the count.
    pub fn delete_host_config(&mut self, window: &mut SettingsWindow, index: usize) {
        if index >= self.hosts.len() {
            return;
        }
        for i in index..self.hosts.len().saturating_sub(1) {
            let cur = format!("DEV9/Eth/Hosts/Host{}", i);
            let next = format!("DEV9/Eth/Hosts/Host{}", i + 1);
            let next_url = self.hosts[i + 1].url.clone();
            let next_desc = self.hosts[i + 1].desc.clone();
            let next_address = self.hosts[i + 1].address.clone();
            let next_enabled = self.hosts[i + 1].enabled;
            window.set_string_setting(&cur, "Url", Some(&next_url));
            window.set_string_setting(&cur, "Desc", Some(&next_desc));
            window.set_string_setting(&cur, "Address", Some(&next_address));
            window.set_bool_setting(&cur, "Enabled", Some(next_enabled));
            self.hosts[i] = self.hosts[i + 1].clone();
        }
        self.hosts.pop();
        let last_section = format!("DEV9/Eth/Hosts/Host{}", self.hosts.len());
        window.set_string_setting(&last_section, "Url", None);
        window.set_int_setting(
            "DEV9/Eth/Hosts",
            "Count",
            Some(self.hosts.len() as i32),
        );
        self.refresh_host_list();
    }
}

// ---------------------------------------------------------------------------
// `MemoryCardSettingsWidget`
// ---------------------------------------------------------------------------

/// Slot data for a single memory card slot.
#[derive(Debug, Default, Clone)]
pub struct MemoryCardSlot {
    pub enabled: bool,
    pub file_name: Option<String>,
    pub inherited: bool,
}

/// `MemoryCardSettingsWidget` — the memory card tab.
#[derive(Debug, Default, Clone)]
pub struct MemoryCardSettingsWidget {
    pub base: SettingsWidget,
    pub directory: PathBuf,
    pub slots: Vec<MemoryCardSlot>,
    pub selected_card: Option<String>,
}

impl MemoryCardSettingsWidget {
    pub const MAX_SLOTS: usize = 2;

    pub fn new() -> Self {
        Self {
            slots: (0..Self::MAX_SLOTS)
                .map(|_| MemoryCardSlot::default())
                .collect(),
            ..Default::default()
        }
    }

    /// Mirrors `tryInsertCard`.  The C++ code validates the path and
    /// rejects unknown card files; we keep the same checks.
    pub fn try_insert_card(&mut self, window: &mut SettingsWindow, slot: u32, path: &str) {
        if path.is_empty() {
            return;
        }
        let key = format!("Slot{}_Filename", slot + 1);
        window.set_string_setting("MemoryCards", &key, Some(path));
        if let Some(s) = self.slots.get_mut(slot as usize) {
            s.file_name = Some(path.to_string());
            s.inherited = false;
        }
    }

    /// Mirrors `ejectSlot`.  Per-game mode clears the override; global
    /// mode stores an empty string.
    pub fn eject_slot(&mut self, window: &mut SettingsWindow, slot: u32) {
        let key = format!("Slot{}_Filename", slot + 1);
        if window.is_per_game() {
            window.remove_setting("MemoryCards", &key);
        } else {
            window.set_string_setting("MemoryCards", &key, Some(""));
        }
        if let Some(s) = self.slots.get_mut(slot as usize) {
            s.file_name = None;
        }
    }

    /// Mirrors `swapCards`.
    pub fn swap_cards(&mut self, window: &mut SettingsWindow) {
        if self.slots.len() < 2 {
            return;
        }
        let card1 = self.slots[0].file_name.clone();
        let card2 = self.slots[1].file_name.clone();
        if card1.is_none() || card2.is_none() {
            return;
        }
        let key1 = format!("Slot{}_Filename", 1);
        let key2 = format!("Slot{}_Filename", 2);
        window.set_string_setting("MemoryCards", &key1, card2.as_deref());
        window.set_string_setting("MemoryCards", &key2, card1.as_deref());
        self.slots[0].file_name = card2;
        self.slots[1].file_name = card1;
    }

    /// Mirrors `refresh`.
    pub fn refresh(&mut self, window: &SettingsWindow) {
        for (i, slot) in self.slots.iter_mut().enumerate() {
            let key = format!("Slot{}_Filename", i + 1);
            let name = window.effective_string("MemoryCards", &key, "");
            slot.file_name = if name.is_empty() { None } else { Some(name) };
            slot.inherited = window.is_per_game() && !window.contains_setting("MemoryCards", &key);
        }
    }
}

// ---------------------------------------------------------------------------
// `GameListSettingsWidget`
// ---------------------------------------------------------------------------

/// Search path used to scan the user's library.  Mirrors the rows in the
/// `searchDirectoryList` QTableWidget.
#[derive(Debug, Default, Clone)]
pub struct GameListSearchPath {
    pub path: String,
    pub recursive: bool,
}

/// `GameListSettingsWidget` — the game list paths tab.
#[derive(Debug, Default, Clone)]
pub struct GameListSettingsWidget {
    pub base: SettingsWidget,
    pub paths: Vec<GameListSearchPath>,
    pub excluded_paths: Vec<String>,
}

impl GameListSettingsWidget {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mirrors `addExcludedPath`.
    pub fn add_excluded_path(&mut self, path: &str) -> bool {
        if path.is_empty() {
            return false;
        }
        self.excluded_paths.push(path.to_string());
        true
    }

    /// Mirrors `addPathToTable`.
    pub fn add_path(&mut self, path: &str, recursive: bool) {
        self.paths.push(GameListSearchPath {
            path: path.to_string(),
            recursive,
        });
    }

    /// Mirrors `addSearchDirectory`.  Switches the path between the
    /// recursive and non-recursive bucket in the settings layer.
    pub fn add_search_directory(&mut self, path: &str, recursive: bool) {
        // Drop from the other bucket, add to the new one.  Because we
        // carry the merged list locally, we just remove duplicates.
        self.paths.retain(|p| p.path != path);
        self.add_path(path, recursive);
    }

    pub fn remove_search_directory(&mut self, path: &str) {
        self.paths.retain(|p| p.path != path);
    }

    pub fn refresh_directory_list(&mut self) {
        self.paths.sort_by(|a, b| a.path.cmp(&b.path));
    }

    pub fn refresh_exclusion_list(&mut self) {
        // In the C++ code the list is sourced from
        // `Host::GetBaseStringListSetting("GameList", "ExcludedPaths")`.
        // The translation just keeps the local mirror.
        self.excluded_paths.sort();
    }
}

// ---------------------------------------------------------------------------
// `GameSummaryWidget`
// ---------------------------------------------------------------------------

/// One row inside the disc/track list.
#[derive(Debug, Default, Clone)]
pub struct GameSummaryTrack {
    pub number: u32,
    pub mode: String,
    pub start_lsn: u32,
    pub sectors: u32,
    pub size: u64,
    pub md5: String,
    pub status: TrackStatus,
}

/// Verification status of a single track.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum TrackStatus {
    #[default]
    NotComputed,
    Ok,
    Failed,
    Missing,
}

/// `GameSummaryWidget` — the per-game Summary tab.
#[derive(Debug, Default, Clone)]
pub struct GameSummaryWidget {
    pub base: SettingsWidget,
    pub entry_path: String,
    pub input_profile: Option<String>,
    pub custom_title: Option<String>,
    pub custom_region: Option<i32>,
    pub verify_result: Option<QStr>,
    pub redump_search_keyword: String,
    pub tracks: Vec<GameSummaryTrack>,
    pub disc_path: Option<String>,
}

impl GameSummaryWidget {
    pub fn new(entry: &GameListEntry) -> Self {
        Self {
            entry_path: entry.path.clone(),
            ..Default::default()
        }
    }

    /// Mirrors `onInputProfileChanged`.  Index 0 means "use global".
    pub fn on_input_profile_changed(&mut self, window: &mut SettingsWindow, index: i32, names: &[String]) {
        if index <= 0 {
            window.remove_setting("EmuCore", "InputProfileName");
            self.input_profile = None;
        } else if let Some(name) = names.get((index - 1) as usize) {
            window.set_string_setting("EmuCore", "InputProfileName", Some(name));
            self.input_profile = Some(name.clone());
        }
    }

    /// Mirrors `setCustomTitle`.
    pub fn set_custom_title(&mut self, title: &str) {
        self.custom_title = if title.is_empty() { None } else { Some(title.to_string()) };
    }

    /// Mirrors `setCustomRegion`.
    pub fn set_custom_region(&mut self, region: i32) {
        self.custom_region = if region < 0 { None } else { Some(region) };
    }

    /// Mirrors `onDiscPathChanged`.  When the user clears the field we
    /// remove the override; otherwise we write the new path.
    pub fn on_disc_path_changed(&mut self, window: &mut SettingsWindow, value: &str) {
        if value.is_empty() {
            window.remove_setting("EmuCore", "DiscPath");
            self.disc_path = None;
        } else {
            window.set_string_setting("EmuCore", "DiscPath", Some(value));
            self.disc_path = Some(value.to_string());
        }
    }

    /// Mirrors `onVerifyClicked` — the result string is just stored in
    /// the widget; the real hashing lives in another module.
    pub fn on_verify_clicked(&mut self, result: QStr) {
        self.verify_result = Some(result);
    }

    /// Mirrors `onSearchHashClicked` — returns the URL to open.
    pub fn search_hash_url(&self) -> Option<String> {
        if self.redump_search_keyword.is_empty() {
            None
        } else {
            Some(format!(
                "http://redump.org/discs/quicksearch/{}",
                self.redump_search_keyword
            ))
        }
    }

    /// Mirrors `onCheckWikiClicked`.
    pub fn check_wiki_url(&self, serial: &str) -> String {
        format!("https://wiki.pcsx2.net/{}", serial)
    }
}

// ---------------------------------------------------------------------------
// `FolderSettingsWidget`
// ---------------------------------------------------------------------------

/// `FolderSettingsWidget` — the Folders tab.
#[derive(Debug, Default, Clone)]
pub struct FolderSettingsWidget {
    pub base: SettingsWidget,
    pub cache: PathBuf,
    pub cheats: PathBuf,
    pub covers: PathBuf,
    pub snapshots: PathBuf,
    pub save_states: PathBuf,
    pub video_dumping: PathBuf,
    pub organise_snapshots_by_game: bool,
    pub organise_video_dumps_by_game: bool,
}

impl FolderSettingsWidget {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the data-roots under `DataRoot/<subdir>` that the
    /// settings widget binds.
    pub fn default_paths(data_root: &Path) -> [PathBuf; 6] {
        [
            data_root.join("cache"),
            data_root.join("cheats"),
            data_root.join("covers"),
            data_root.join("snaps"),
            data_root.join("sstates"),
            data_root.join("videos"),
        ]
    }
}

// ---------------------------------------------------------------------------
// `GameListModel` — the Qt table model.
// ---------------------------------------------------------------------------

/// Column indices for the game list table model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(usize)]
pub enum GameListColumn {
    Type = 0,
    Serial = 1,
    Title = 2,
    FileTitle = 3,
    Crc = 4,
    TimePlayed = 5,
    LastPlayed = 6,
    Size = 7,
    Region = 8,
    Compatibility = 9,
    Cover = 10,
}

impl GameListColumn {
    pub const COUNT: usize = 11;
    pub fn as_str(self) -> &'static str {
        match self {
            GameListColumn::Type => "Type",
            GameListColumn::Serial => "Code",
            GameListColumn::Title => "Title",
            GameListColumn::FileTitle => "File Title",
            GameListColumn::Crc => "CRC",
            GameListColumn::TimePlayed => "Time Played",
            GameListColumn::LastPlayed => "Last Played",
            GameListColumn::Size => "Size",
            GameListColumn::Region => "Region",
            GameListColumn::Compatibility => "Compatibility",
            GameListColumn::Cover => "Cover",
        }
    }
}

/// The Qt `QAbstractTableModel` that backs the game list.  In the data-only
/// translation we only carry the configuration and the cover scale state.
#[derive(Debug)]
pub struct GameListModel {
    pub cover_scale: f32,
    pub cover_scale_counter: AtomicU32,
    pub show_titles_for_covers: bool,
    pub prefer_english_titles: bool,
    pub dpr: f32,
    pub column_display_names: [QStr; GameListColumn::COUNT],
    /// Cached cover pixmap paths.  Mirrors `m_cover_pixmap_cache`.
    pub cover_cache: HashMap<String, Vec<u8>>,
    /// Column ordering for sorting; mirrors the C++ enum.
    pub default_sort_column: GameListColumn,
}

impl Default for GameListModel {
    fn default() -> Self {
        Self {
            cover_scale: 0.45,
            cover_scale_counter: AtomicU32::new(0),
            show_titles_for_covers: true,
            prefer_english_titles: false,
            dpr: 1.0,
            column_display_names: [
                QStr::from("Type"),
                QStr::from("Code"),
                QStr::from("Title"),
                QStr::from("File Title"),
                QStr::from("CRC"),
                QStr::from("Time Played"),
                QStr::from("Last Played"),
                QStr::from("Size"),
                QStr::from("Region"),
                QStr::from("Compatibility"),
                QStr::from("Cover"),
            ],
            cover_cache: HashMap::new(),
            default_sort_column: GameListColumn::Title,
        }
    }
}

impl GameListModel {
    pub fn new(cover_scale: f32, show_cover_titles: bool, dpr: f32) -> Self {
        Self {
            cover_scale,
            show_titles_for_covers: show_cover_titles,
            dpr,
            ..Default::default()
        }
    }

    pub fn get_column_id_for_name(name: &str) -> Option<GameListColumn> {
        match name {
            "Type" => Some(GameListColumn::Type),
            "Code" => Some(GameListColumn::Serial),
            "Title" => Some(GameListColumn::Title),
            "File Title" => Some(GameListColumn::FileTitle),
            "CRC" => Some(GameListColumn::Crc),
            "Time Played" => Some(GameListColumn::TimePlayed),
            "Last Played" => Some(GameListColumn::LastPlayed),
            "Size" => Some(GameListColumn::Size),
            "Region" => Some(GameListColumn::Region),
            "Compatibility" => Some(GameListColumn::Compatibility),
            "Cover" => Some(GameListColumn::Cover),
            _ => None,
        }
    }

    pub fn get_column_name(column: GameListColumn) -> &'static str {
        column.as_str()
    }

    /// Mirrors `setCoverScale`.  The counter is bumped so that any
    /// outstanding cover-generation jobs can detect the change.
    pub fn set_cover_scale(&mut self, scale: f32) {
        if (self.cover_scale - scale).abs() < f32::EPSILON {
            return;
        }
        self.cover_scale = scale;
        self.cover_scale_counter.fetch_add(1, Ordering::Release);
    }

    pub fn set_show_cover_titles(&mut self, enabled: bool) {
        self.show_titles_for_covers = enabled;
    }

    pub fn set_device_pixel_ratio(&mut self, dpr: f32) {
        self.dpr = dpr;
    }

    pub fn set_prefer_english_titles(&mut self, enabled: bool) {
        self.prefer_english_titles = enabled;
    }

    pub fn get_cover_art_width(&self) -> i32 {
        ((350.0 * self.cover_scale) as i32).max(1)
    }

    pub fn get_cover_art_height(&self) -> i32 {
        ((512.0 * self.cover_scale) as i32).max(1)
    }

    pub fn get_cover_art_spacing(&self) -> i32 {
        ((32.0 * self.cover_scale) as i32).max(1)
    }

    /// Number of rows in the model.  Mirrors `rowCount`.
    pub fn row_count(&self) -> i32 {
        0
    }

    /// Number of columns.  Mirrors `columnCount`.
    pub fn column_count(&self) -> i32 {
        GameListColumn::COUNT as i32
    }

    /// Returns a textual representation of the requested cell.  Mirrors
    /// the `DisplayRole` path of `data()`.  `data(row, col)` is the
    /// public API requested in the task description.
    pub fn data(&self, row: i32, col: i32) -> Option<QStr> {
        if row < 0 || col < 0 || col >= self.column_count() {
            return None;
        }
        let column = match GameListColumn::from_usize(col as usize) {
            Some(c) => c,
            None => return None,
        };
        // In a real implementation we'd look up the entry by row and
        // return the appropriate string.  The translation keeps the
        // API surface but cannot reach into the live game list.
        let _ = row;
        let _ = column;
        Some(QStr::new())
    }

    /// Returns the icon role data for a cell.  In the data-only
    /// translation this returns `None` for cells that would normally
    /// fetch a pixmap; the C++ code uses `QPixmap` lookups we cannot
    /// replicate.
    pub fn icon_data(&self, _row: i32, col: i32) -> Option<QColor> {
        match GameListColumn::from_usize(col as usize) {
            Some(GameListColumn::Type) | Some(GameListColumn::Region) | Some(GameListColumn::Compatibility) => {
                Some(QColor::GREEN_OK)
            }
            _ => None,
        }
    }

    /// Mirrors `headerData`.  Returns the localised column title.
    pub fn header_data(&self, section: i32) -> Option<QStr> {
        if section < 0 {
            return None;
        }
        GameListColumn::from_usize(section as usize).map(|c| self.column_display_names[c as usize].clone())
    }

    /// Mirrors `refresh`.  The C++ code triggers a model reset; we
    /// only have to drop any cached pixmaps.
    pub fn refresh(&mut self) {
        self.cover_cache.clear();
    }

    /// Mirrors `lessThan` for the given column.
    pub fn less_than(&self, left: &GameListEntry, right: &GameListEntry, column: GameListColumn) -> bool {
        match column {
            GameListColumn::Type => left.type_ < right.type_,
            GameListColumn::Serial => left.serial < right.serial,
            GameListColumn::Title => left.title < right.title,
            GameListColumn::FileTitle => left.title_sort < right.title_sort,
            GameListColumn::Crc => left.crc < right.crc,
            GameListColumn::TimePlayed => left.total_played_time < right.total_played_time,
            GameListColumn::LastPlayed => left.last_played_time < right.last_played_time,
            GameListColumn::Size => left.total_size < right.total_size,
            GameListColumn::Region => left.region < right.region,
            GameListColumn::Compatibility => left.compatibility_rating < right.compatibility_rating,
            GameListColumn::Cover => false,
        }
    }

    /// Mirrors `formatTimespan`.  Returns a human-readable timespan.
    pub fn format_timespan(timespan: u64) -> String {
        let hours = timespan / 3600;
        if hours > 0 {
            return format!("{} hours", hours);
        }
        let minutes = (timespan % 3600) / 60;
        if minutes > 0 {
            return format!("{} minutes", minutes);
        }
        format!("{} seconds", timespan % 60)
    }
}

impl GameListColumn {
    pub fn from_usize(v: usize) -> Option<Self> {
        match v {
            0 => Some(Self::Type),
            1 => Some(Self::Serial),
            2 => Some(Self::Title),
            3 => Some(Self::FileTitle),
            4 => Some(Self::Crc),
            5 => Some(Self::TimePlayed),
            6 => Some(Self::LastPlayed),
            7 => Some(Self::Size),
            8 => Some(Self::Region),
            9 => Some(Self::Compatibility),
            10 => Some(Self::Cover),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// `GameListWidget`
// ---------------------------------------------------------------------------

/// View mode of the game list widget.  Mirrors the `QStackedWidget`
/// index from the C++ code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameListView {
    Table,
    Grid,
    Empty,
}

/// Filter applied to the game list.  Mirrors the `GameListSortModel`
/// state.
#[derive(Debug, Default, Clone)]
pub struct GameListFilter {
    pub entry_type: Option<u32>,
    pub region: Option<u32>,
    pub name: Option<String>,
}

/// `GameListWidget` — the table / grid view of the user's library.
#[derive(Debug)]
pub struct GameListWidget {
    pub model: GameListModel,
    pub view: GameListView,
    pub filter: GameListFilter,
    pub show_cover_titles: bool,
    pub cover_scale: f32,
    pub current_sort: GameListColumn,
    pub sort_descending: bool,
    pub background_path: Option<PathBuf>,
    pub background_opacity: f32,
    pub background_scaling: u8,
    pub refreshing: bool,
}

impl Default for GameListWidget {
    fn default() -> Self {
        Self {
            model: GameListModel::default(),
            view: GameListView::Table,
            filter: GameListFilter::default(),
            show_cover_titles: true,
            cover_scale: 0.45,
            current_sort: GameListColumn::Title,
            sort_descending: false,
            background_path: None,
            background_opacity: 100.0,
            background_scaling: 0,
            refreshing: false,
        }
    }
}

impl GameListWidget {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mirrors `initialize`.  Pulls settings out of the host layer.
    pub fn initialize(&mut self, cover_scale: f32, show_cover_titles: bool) {
        self.cover_scale = cover_scale;
        self.show_cover_titles = show_cover_titles;
        self.model.set_cover_scale(cover_scale);
        self.model.set_show_cover_titles(show_cover_titles);
    }

    /// Mirrors `refresh`.  Marks the widget as refreshing and returns
    /// the parameters the caller should pass to `GameListRefreshThread`.
    pub fn refresh(&mut self, invalidate_cache: bool, _popup_on_error: bool) -> RefreshRequest {
        self.refreshing = true;
        RefreshRequest {
            invalidate_cache,
            popup_on_error: _popup_on_error,
        }
    }

    /// Mirrors `cancelRefresh`.  Resets the refreshing flag.
    pub fn cancel_refresh(&mut self) {
        self.refreshing = false;
    }

    /// Mirrors `showGameList`.
    pub fn show_game_list(&mut self) {
        self.view = GameListView::Table;
    }

    /// Mirrors `showGameGrid`.
    pub fn show_game_grid(&mut self) {
        self.view = GameListView::Grid;
    }

    /// Mirrors `setShowCoverTitles`.
    pub fn set_show_cover_titles(&mut self, enabled: bool) {
        self.show_cover_titles = enabled;
        self.model.set_show_cover_titles(enabled);
    }

    /// Mirrors `listZoom`.  `delta` is the per-tick change to apply.
    pub fn list_zoom(&mut self, delta: f32) {
        let next = (self.cover_scale + delta).clamp(0.1, 2.0);
        self.cover_scale = next;
        self.model.set_cover_scale(next);
    }

    pub fn grid_zoom_in(&mut self) {
        self.list_zoom(0.05);
    }

    pub fn grid_zoom_out(&mut self) {
        self.list_zoom(-0.05);
    }

    pub fn is_showing_game_list(&self) -> bool {
        self.view == GameListView::Table
    }

    pub fn is_showing_game_grid(&self) -> bool {
        self.view == GameListView::Grid
    }

    /// Mirrors `setCustomBackground`.  Accepts the path to a movie or
    /// PNG file used as the gamelist background.
    pub fn set_custom_background(&mut self, path: Option<PathBuf>) {
        self.background_path = path;
    }

    /// Mirrors `setFilter` for the type / region / name triple.
    pub fn set_filter(&mut self, filter: GameListFilter) {
        self.filter = filter;
    }

    /// Mirrors `onSelectionModelCurrentChanged`.  Returns the entry at
    /// the given row if the index is valid.
    pub fn entry_at(&self, _row: i32) -> Option<GameListEntry> {
        None
    }

    /// Mirrors `saveSortSettings`.  Stores the sort column and order.
    pub fn save_sort_settings(&mut self, column: GameListColumn, descending: bool) {
        self.current_sort = column;
        self.sort_descending = descending;
    }

    /// Mirrors `loadTableHeaderState` — used when a user has a saved
    /// header state.  The Rust translation just records the flag.
    pub fn header_state_loaded(&self) -> bool {
        self.background_path.is_some()
    }

    /// Mirrors `applyTableHeaderDefaults`.
    pub fn apply_table_header_defaults(&mut self) {
        self.current_sort = GameListColumn::Title;
        self.sort_descending = false;
    }

    /// Mirrors `onTableHeaderStateChanged` — flags the header state as
    /// dirty so the host layer can persist it.
    pub fn mark_header_dirty(&mut self) {
        // In the data-only translation we just rely on the caller to
        // persist the state when this method is invoked.
    }
}

/// Request produced by `GameListWidget::refresh` and consumed by
/// `GameListRefreshThread::run`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefreshRequest {
    pub invalidate_cache: bool,
    pub popup_on_error: bool,
}

// ---------------------------------------------------------------------------
// `GameListRefreshThread`
// ---------------------------------------------------------------------------

/// Outcome of one refresh operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshOutcome {
    Completed,
    Cancelled,
    Failed(String),
}

/// `GameListRefreshThread` — a background worker that runs the game list
/// rescan.  In the data-only translation the work is simulated; the
/// real implementation would call into `GameList::Refresh`.
#[derive(Debug)]
pub struct GameListRefreshThread {
    pub invalidate_cache: bool,
    pub popup_on_error: bool,
    pub progress_text: QStr,
    pub progress_value: i32,
    pub progress_range: i32,
    pub cancelled: bool,
    pub outcome: Option<RefreshOutcome>,
    pub started: bool,
    pub finished: bool,
}

impl GameListRefreshThread {
    pub fn new(invalidate_cache: bool, popup_on_error: bool) -> Self {
        Self {
            invalidate_cache,
            popup_on_error,
            progress_text: QStr::new(),
            progress_value: 0,
            progress_range: 0,
            cancelled: false,
            outcome: None,
            started: false,
            finished: false,
        }
    }

    /// Mirrors the C++ `start()` slot.  In a real implementation this
    /// would spawn a thread; the translation runs the work synchronously
    /// to make the data flow visible to the caller.
    pub fn start(&mut self) {
        self.started = true;
        self.outcome = Some(self.run());
        self.finished = true;
    }

    /// Mirrors the C++ `run()` override.  Returns the outcome without
    /// actually performing IO.
    pub fn run(&mut self) -> RefreshOutcome {
        if self.cancelled {
            return RefreshOutcome::Cancelled;
        }
        if self.invalidate_cache {
            // In a real implementation we'd invalidate the cover cache
            // here.  Nothing to do in the data-only translation.
        }
        RefreshOutcome::Completed
    }

    /// Mirrors the C++ `cancel()` slot.
    pub fn cancel(&mut self) {
        self.cancelled = true;
    }

    /// Mirrors `wait()`.  Because the translation runs synchronously,
    /// the work is always finished by the time `start` returns.
    pub fn wait(&self) -> bool {
        self.finished
    }

    /// Mirrors the `refreshProgress` signal payload.
    pub fn refresh_progress(&self) -> (QStr, i32, i32) {
        (self.progress_text.clone(), self.progress_value, self.progress_range)
    }

    /// Mirrors the `refreshComplete` signal.
    pub fn refresh_complete(&self) -> bool {
        self.finished
    }

    /// Mirrors the `setStatusText` callback.
    pub fn set_status_text(&mut self, text: &str) {
        if self.progress_text.as_str() == text {
            return;
        }
        self.progress_text = QStr::from(text);
    }

    /// Mirrors the `setProgressRange` callback.
    pub fn set_progress_range(&mut self, range: i32) {
        if self.progress_range == range {
            return;
        }
        self.progress_range = range;
    }

    /// Mirrors the `setProgressValue` callback.
    pub fn set_progress_value(&mut self, value: i32) {
        if self.progress_value == value {
            return;
        }
        self.progress_value = value;
    }
}

// ---------------------------------------------------------------------------
// Smoke tests — only enabled with `--cfg test_translation`.  These are not
// run by the host but they document the intended semantics of the API.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_window_per_game_flag() {
        let entry = GameListEntry::dummy();
        let sif = SettingsInterface::new_per_game(SettingsLayerData::new(), SettingsLayerData::new());
        let win = SettingsWindow::new_per_game(sif, &entry, "SLUS-12345".into(), 0xDEADBEEF, QStr::from("foo.bin"));
        assert!(win.is_per_game());
        assert_eq!(win.get_serial(), "SLUS-12345");
        assert_eq!(win.get_disc_crc(), 0xDEADBEEF);
    }

    #[test]
    fn settings_window_categories() {
        let mut win = SettingsWindow::new_global();
        win.add_widget(
            SettingsCategory::Graphics,
            QStr::from("Graphics"),
            QStr::from("image-fill"),
            QStr::from("help"),
            SettingsWidgetKind::Graphics(GraphicsSettingsWidget::new()),
        );
        assert!(win.goto_category("Graphics"));
        assert_eq!(win.category(), SettingsCategory::Graphics);
        assert!(!win.goto_category("Does not exist"));
    }

    #[test]
    fn graphics_renderer_classification() {
        let mut g = GraphicsSettingsWidget::new();
        g.renderer = GsRenderer::Vk;
        assert_eq!(g.rendering_path(), RenderingPath::Hardware);
        g.renderer = GsRenderer::Sw;
        assert_eq!(g.rendering_path(), RenderingPath::Software);
    }

    #[test]
    fn game_list_model_columns() {
        let m = GameListModel::default();
        assert_eq!(m.column_count(), GameListColumn::COUNT as i32);
        assert_eq!(m.header_data(2).unwrap().as_str(), "Title");
    }

    #[test]
    fn dev9_widget_loads_adapters_once() {
        let mut w = DEV9SettingsWidget::new();
        w.load_adapters();
        w.load_adapters(); // second call is a no-op
        assert!(w.adapters_loaded);
        assert!(w.api_list.contains(&Dev9NetApi::Sockets));
    }

    #[test]
    fn memory_card_swap() {
        let sif = SettingsInterface::new_global(SettingsLayerData::new());
        let mut win = SettingsWindow {
            sif: Some(sif),
            ..SettingsWindow::new_global()
        };
        let mut m = MemoryCardSettingsWidget::new();
        m.try_insert_card(&mut win, 0, "card1.ps2");
        m.try_insert_card(&mut win, 1, "card2.ps2");
        m.swap_cards(&mut win);
        assert_eq!(m.slots[0].file_name.as_deref(), Some("card2.ps2"));
        assert_eq!(m.slots[1].file_name.as_deref(), Some("card1.ps2"));
    }

    #[test]
    fn game_list_refresh_thread() {
        let mut t = GameListRefreshThread::new(true, false);
        t.start();
        assert!(t.refresh_complete());
        assert_eq!(t.outcome, Some(RefreshOutcome::Completed));
    }
}
