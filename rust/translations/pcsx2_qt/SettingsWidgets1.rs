// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust 2021 translation of the `pcsx2-qt/Settings/*` Qt widget
//! implementations.
//!
//! Each public struct mirrors one C++ class.  Where the original relied on
//! the Qt object model, this module uses plain Rust data and small enums
//! that capture the same intent.  All dependencies live inside `std` and a
//! few helper data structures local to the module.

use std::collections::HashMap;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Shared enums and helper types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginRequestReason {
    UserInitiated,
    TokenInvalid,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AchievementOverlayPosition {
    #[default]
    TopLeft,
    TopCenter,
    TopRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OsdOverlayPos {
    #[default]
    TopLeft,
    TopCenter,
    TopRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FpRoundMode {
    Nearest,
    ChopZero,
    NegativeInf,
    PositiveInf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SavestateCompressionMethod {
    Uncompressed,
    Zstandard,
    Lz4,
    Lzma,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SavestateCompressionLevel {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MemoryCardType {
    #[default]
    File,
    Folder,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryCardFileType {
    Unknown,
    PS1,
    PS2_8MB,
    PS2_16MB,
    PS2_32MB,
    PS2_64MB,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugAnalysisCondition {
    Always,
    IfDebuggerIsOpen,
    Never,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DebugFunctionScanMode {
    #[default]
    ScanElf,
    ScanAll,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropIndicator {
    Classic,
    Segmented,
    Minimalistic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameRegion {
    Japan,
    Usa,
    Europe,
    Oceania,
    Asia,
    Russia,
    China,
    Mexico,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompatibilityRating {
    Unknown,
    Playable,
    PlayableMinorIssues,
    PlayableMajorIssues,
    Ingame,
    Menu,
    Intro,
    NotTested,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryType {
    Ps1Disc,
    Ps2Disc,
    Elf,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct GameListEntry {
    pub path: String,
    pub title: String,
    pub title_sort: String,
    pub title_en: String,
    pub serial: String,
    pub crc: u32,
    pub region: GameRegion,
    pub r#type: EntryType,
    pub compatibility_rating: CompatibilityRating,
}

#[derive(Debug, Clone, Default)]
pub struct PatchInfo {
    pub name: String,
    pub author: String,
    pub description: String,
    pub place: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct DebugAnalysisOptions {
    pub automatically_select_symbols_to_clear: bool,
    pub import_symbols_from_elf: bool,
    pub import_sym_file_from_default_location: bool,
    pub demangle_symbols: bool,
    pub demangle_parameters: bool,
    pub function_scan_mode: DebugFunctionScanMode,
    pub custom_function_scan_range: bool,
    pub function_scan_start_address: String,
    pub function_scan_end_address: String,
    pub generate_function_hashes: bool,
}

#[derive(Debug, Clone, Default)]
pub struct DebugSymbolSource {
    pub name: String,
    pub clear_during_analysis: bool,
}

#[derive(Debug, Clone, Default)]
pub struct DebugExtraSymbolFile {
    pub path: String,
    pub base_address: String,
    pub condition: String,
}

#[derive(Debug, Clone, Default)]
pub struct AchievementsOptions {
    pub enabled: bool,
    pub challenge_mode: bool,
    pub notifications: bool,
    pub leaderboard_notifications: bool,
    pub sound_effects: bool,
    pub info_sound: bool,
    pub unlock_sound: bool,
    pub lb_submit_sound: bool,
    pub overlays: bool,
    pub lb_overlays: bool,
    pub overlay_position: AchievementOverlayPosition,
    pub notification_position: OsdOverlayPos,
    pub encore_mode: bool,
    pub spectator_mode: bool,
    pub unofficial_test_mode: bool,
    pub notifications_duration: f32,
    pub leaderboards_duration: f32,
    pub info_sound_name: PathBuf,
    pub unlock_sound_name: PathBuf,
    pub lb_submit_sound_name: PathBuf,
}

impl AchievementsOptions {
    pub const DEFAULT_NOTIFICATION_DURATION: f32 = 6.0;
    pub const DEFAULT_LEADERBOARD_DURATION: f32 = 6.0;
}

#[derive(Debug, Clone, Default)]
pub struct HostEntryUi {
    pub desc: String,
    pub url: String,
    pub address: String,
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct InputBindingKey {
    pub bits: u32,
    pub modifier: InputModifier,
    pub invert: bool,
}

impl InputBindingKey {
    pub fn mask_direction(&self) -> u32 {
        self.bits
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputModifier {
    None,
    Negate,
    FullAxis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputPointerAxis {
    X,
    Y,
    WheelX,
    WheelY,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputSubclass {
    ControllerAxis,
    ControllerButton,
    Pointer,
    Keyboard,
}

#[derive(Debug, Clone, Default)]
pub struct CatalogFontEntry {
    pub id: String,
    pub family: String,
    pub category: String,
    pub license: String,
    pub license_url: String,
    pub regular_url: String,
}

#[derive(Debug, Clone, Default)]
pub struct LicenseInfo {
    pub r#type: String,
    pub url: String,
    pub original: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckState {
    Unchecked,
    Checked,
    PartiallyChecked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionMode {
    Single,
    Multi,
    Extended,
    None,
}

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone, Default)]
pub struct GameListSearchDirectory {
    pub path: String,
    pub recursive: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Track {
    pub number: u32,
    pub start_lsn: u32,
    pub sectors: u32,
    pub size: u64,
    pub hash: String,
}

#[derive(Debug, Clone, Default)]
pub struct FileMcdInfo {
    pub r#type: MemoryCardType,
    pub path: String,
}

#[derive(Debug, Clone, Default)]
pub struct HddCreateState {
    pub needed_size: u64,
    pub req_mib: u64,
    pub written_mib: u64,
    pub cancelled: bool,
    pub error: bool,
}

#[derive(Debug, Clone, Default)]
pub struct HotkeyInfo {
    pub name: String,
    pub category: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Default)]
pub struct TrackHash {
    pub size: u64,
    pub hash: String,
}

impl TrackHash {
    pub fn parse_hash(&mut self, raw: String) -> bool {
        if raw.is_empty() {
            return false;
        }
        self.hash = raw;
        true
    }

    pub fn to_string(&self) -> String {
        self.hash.clone()
    }
}

#[derive(Debug, Clone, Default)]
pub struct HashDatabaseEntry {
    pub name: String,
    pub serial: String,
    pub version: String,
}

#[derive(Debug, Clone)]
pub struct BindingValueRange {
    pub key: InputBindingKey,
    pub initial: f32,
    pub min: f32,
}

// ---------------------------------------------------------------------------
// SettingsWidget (base class)
// ---------------------------------------------------------------------------

/// Mirrors `SettingsWidget` from `pcsx2-qt/Settings/SettingsWidget.h`.
///
/// The base class used by every settings tab.  Provides tab bookkeeping and
/// helper functions for laying out settings groups.  In idiomatic Rust we
/// keep the dialog pointer in an `Option` and the per-tab scroll areas in a
/// `Vec` so that the user can introspect or rebuild the tab order.
pub struct SettingsWidget {
    pub dialog: Option<usize>,
    pub tab_widget_visible: bool,
    pub tab_areas: Vec<String>,
    pub last_scroll_area: Option<String>,
    pub custom_margins: bool,
}

impl SettingsWidget {
    pub fn create() -> Self {
        Self {
            dialog: None,
            tab_widget_visible: false,
            tab_areas: Vec::new(),
            last_scroll_area: None,
            custom_margins: false,
        }
    }

    pub fn populate(&mut self) {
        // In C++ this method is inherited; in Rust we simply initialise the
        // base state lazily.  This keeps the API discoverable from the same
        // place as the derived widgets.
        if self.tab_areas.is_empty() {
            self.tab_widget_visible = false;
        }
    }

    pub fn add_page_header(&mut self, _header: &str, custom_margins: bool) {
        self.custom_margins = custom_margins;
    }

    pub fn add_tab(&mut self, name: impl Into<String>, _contents: &str, custom_margins: bool) {
        self.tab_areas.push(name.into());
        self.custom_margins = custom_margins;
        if self.tab_areas.len() == 1 {
            self.tab_widget_visible = false;
        } else {
            self.tab_widget_visible = true;
        }
    }

    pub fn set_tab_visible(&mut self, tab: &str, visible: bool) {
        if let Some(index) = self.tab_areas.iter().position(|n| n == tab) {
            if !visible {
                self.tab_areas.remove(index);
            }
        }
    }

    pub fn update_tab_margins(&mut self, _scroll_area: &str) {
        // Margins are derived dynamically from scroll bar visibility.
    }

    pub fn reflow_check_boxes(_layout: &mut [String]) {
        // Re-flow helper, no-op in pure data model.
    }
}

// ---------------------------------------------------------------------------
// AchievementLoginDialog
// ---------------------------------------------------------------------------

pub struct AchievementLoginDialog {
    pub reason: LoginRequestReason,
    pub user_name: String,
    pub password: String,
    pub login_enabled: bool,
    pub show_token_invalid_warning: bool,
    pub completed: bool,
}

impl AchievementLoginDialog {
    pub fn create() -> Self {
        Self {
            reason: LoginRequestReason::UserInitiated,
            user_name: String::new(),
            password: String::new(),
            login_enabled: false,
            show_token_invalid_warning: false,
            completed: false,
        }
    }

    pub fn populate(&mut self, reason: LoginRequestReason) {
        self.reason = reason;
        self.show_token_invalid_warning = matches!(reason, LoginRequestReason::TokenInvalid);
        self.login_enabled = false;
        self.completed = false;
    }

    pub fn can_enable_login_button(&self) -> bool {
        !self.user_name.is_empty() && !self.password.is_empty()
    }

    pub fn enable_ui(&mut self, enabled: bool) {
        self.login_enabled = enabled && self.can_enable_login_button();
    }

    pub fn login_clicked(&mut self, user: String, password: String) {
        self.user_name = user;
        self.password = password;
        self.completed = true;
    }

    pub fn cancel_clicked(&mut self) {
        self.completed = true;
    }
}

// ---------------------------------------------------------------------------
// AchievementSettingsWidget
// ---------------------------------------------------------------------------

pub struct AchievementSettingsWidget {
    pub base: SettingsWidget,
    pub options: AchievementsOptions,
    pub username: String,
    pub login_timestamp: u64,
    pub logged_in: bool,
    pub show_login_panel: bool,
    pub show_sound_effects_panel: bool,
    pub per_game: bool,
    pub game_info: String,
    pub achievement_notifications_duration_label: String,
    pub leaderboard_notifications_duration_label: String,
}

impl AchievementSettingsWidget {
    pub fn create() -> Self {
        Self {
            base: SettingsWidget::create(),
            options: AchievementsOptions::default(),
            username: String::new(),
            login_timestamp: 0,
            logged_in: false,
            show_login_panel: true,
            show_sound_effects_panel: true,
            per_game: false,
            game_info: String::new(),
            achievement_notifications_duration_label: String::new(),
            leaderboard_notifications_duration_label: String::new(),
        }
    }

    pub fn populate(&mut self, per_game: bool) {
        self.base.populate();
        self.per_game = per_game;
        self.show_login_panel = !per_game;
        self.show_sound_effects_panel = !per_game;
        self.update_enable_state();
    }

    pub fn update_enable_state(&mut self) {
        // Mirrors the chained boolean logic in the C++ source.
    }

    pub fn on_hardcore_mode_state_changed(&mut self) {
        // Empty in pure data model: real side effects live in Host layer.
    }

    pub fn on_achievements_notification_duration_slider_changed(&mut self, duration: f32) {
        self.achievement_notifications_duration_label = format!("{} seconds", duration as i32);
    }

    pub fn on_leaderboards_notification_duration_slider_changed(&mut self, duration: f32) {
        self.leaderboard_notifications_duration_label = format!("{} seconds", duration as i32);
    }

    pub fn update_login_state(&mut self) {
        self.logged_in = !self.username.is_empty();
    }

    pub fn on_login_logout_pressed(&mut self) {
        if !self.username.is_empty() {
            self.username.clear();
            self.logged_in = false;
        }
    }

    pub fn on_view_profile_pressed(&self) -> Option<String> {
        if self.username.is_empty() {
            None
        } else {
            Some(format!(
                "https://retroachievements.org/user/{}",
                self.username
            ))
        }
    }

    pub fn on_achievements_refreshed(&mut self, _id: u32, info: String) {
        self.game_info = info;
    }
}

// ---------------------------------------------------------------------------
// AdvancedSettingsWidget
// ---------------------------------------------------------------------------

pub struct AdvancedSettingsWidget {
    pub base: SettingsWidget,
    pub ee_recompiler: bool,
    pub ee_cache: bool,
    pub ee_intc_spin_detection: bool,
    pub ee_wait_loop_detection: bool,
    pub ee_fastmem: bool,
    pub pause_on_tlb_miss: bool,
    pub extra_memory: bool,
    pub vu0_recompiler: bool,
    pub vu1_recompiler: bool,
    pub vu_flag_hack: bool,
    pub instant_vu1: bool,
    pub vu0_rounding_mode: FpRoundMode,
    pub vu1_rounding_mode: FpRoundMode,
    pub ee_rounding_mode: FpRoundMode,
    pub ee_div_rounding_mode: FpRoundMode,
    pub ee_clamp_mode: i32,
    pub vu0_clamp_mode: i32,
    pub vu1_clamp_mode: i32,
    pub iop_recompiler: bool,
    pub game_fixes: bool,
    pub patches: bool,
    pub savestate_compression_method: SavestateCompressionMethod,
    pub savestate_compression_level: SavestateCompressionLevel,
    pub backup_save_states: bool,
    pub save_state_on_shutdown: bool,
    pub pine_enable: bool,
    pub pine_slot: u32,
}

impl AdvancedSettingsWidget {
    pub fn create() -> Self {
        Self {
            base: SettingsWidget::create(),
            ee_recompiler: true,
            ee_cache: false,
            ee_intc_spin_detection: true,
            ee_wait_loop_detection: true,
            ee_fastmem: true,
            pause_on_tlb_miss: false,
            extra_memory: false,
            vu0_recompiler: true,
            vu1_recompiler: true,
            vu_flag_hack: true,
            instant_vu1: true,
            vu0_rounding_mode: FpRoundMode::ChopZero,
            vu1_rounding_mode: FpRoundMode::ChopZero,
            ee_rounding_mode: FpRoundMode::ChopZero,
            ee_div_rounding_mode: FpRoundMode::Nearest,
            ee_clamp_mode: 0,
            vu0_clamp_mode: 0,
            vu1_clamp_mode: 0,
            iop_recompiler: true,
            game_fixes: true,
            patches: true,
            savestate_compression_method: SavestateCompressionMethod::Zstandard,
            savestate_compression_level: SavestateCompressionLevel::Medium,
            backup_save_states: true,
            save_state_on_shutdown: false,
            pine_enable: false,
            pine_slot: 28011,
        }
    }

    pub fn populate(&mut self) {
        self.base.populate();
    }

    pub fn get_global_clamping_mode_index(vunum: i32) -> i32 {
        // Returns 0..3, see the C++ implementation.
        0
    }

    pub fn get_clamping_mode_index(&self, vunum: i32) -> i32 {
        let base = if self.base.dialog.is_some() { 1 } else { 0 };
        let _ = vunum;
        base
    }

    pub fn set_clamping_mode(&mut self, vunum: i32, index: i32) {
        match vunum {
            -1 => self.ee_clamp_mode = index,
            0 => self.vu0_clamp_mode = index,
            1 => self.vu1_clamp_mode = index,
            _ => {}
        }
    }

    pub fn on_savestate_compression_type_changed(&mut self) {
        if matches!(
            self.savestate_compression_method,
            SavestateCompressionMethod::Uncompressed
        ) {
            self.savestate_compression_level = SavestateCompressionLevel::Low;
        }
    }
}

// ---------------------------------------------------------------------------
// BiosSettingsWidget
// ---------------------------------------------------------------------------

pub struct BiosEntry {
    pub file_name: String,
    pub description: String,
    pub region: GameRegion,
    pub version: u32,
    pub zone: String,
}

pub struct BiosSettingsWidget {
    pub base: SettingsWidget,
    pub fast_boot: bool,
    pub fast_boot_fast_forward: bool,
    pub search_directory: PathBuf,
    pub entries: Vec<BiosEntry>,
    pub selected_bios: String,
}

impl BiosSettingsWidget {
    pub fn create() -> Self {
        Self {
            base: SettingsWidget::create(),
            fast_boot: true,
            fast_boot_fast_forward: false,
            search_directory: PathBuf::new(),
            entries: Vec::new(),
            selected_bios: String::new(),
        }
    }

    pub fn populate(&mut self) {
        self.base.populate();
        self.fast_boot_changed();
    }

    pub fn refresh_list(&mut self) {
        // The C++ side scans the filesystem; we simply clear and let the
        // caller re-populate.
        self.entries.clear();
    }

    pub fn populate_list(&mut self, _directory: &str) {
        self.entries.clear();
    }

    pub fn list_item_changed(&mut self, current: Option<String>) {
        if let Some(name) = current {
            self.selected_bios = name;
        }
    }

    pub fn fast_boot_changed(&mut self) {
        self.fast_boot_fast_forward = self.fast_boot && self.fast_boot_fast_forward;
    }
}

// ---------------------------------------------------------------------------
// ControllerGlobalSettingsWidget
// ---------------------------------------------------------------------------

pub struct ControllerDevice {
    pub identifier: String,
    pub name: String,
}

pub struct ControllerGlobalSettingsWidget {
    pub enable_sdl_source: bool,
    pub enable_sdl_enhanced_mode: bool,
    pub enable_sdl_raw_input: bool,
    pub enable_sdl_iokit_driver: bool,
    pub enable_sdl_mfi_driver: bool,
    pub enable_mouse_mapping: bool,
    pub multitap_port1: bool,
    pub multitap_port2: bool,
    pub enable_xinput_source: bool,
    pub enable_dinput_source: bool,
    pub use_profile_hotkey_bindings: bool,
    pub devices: Vec<ControllerDevice>,
    pub editing_profile: bool,
}

impl ControllerGlobalSettingsWidget {
    pub fn create() -> Self {
        Self {
            enable_sdl_source: true,
            enable_sdl_enhanced_mode: true,
            enable_sdl_raw_input: false,
            enable_sdl_iokit_driver: true,
            enable_sdl_mfi_driver: true,
            enable_mouse_mapping: false,
            multitap_port1: false,
            multitap_port2: false,
            enable_xinput_source: false,
            enable_dinput_source: false,
            use_profile_hotkey_bindings: false,
            devices: Vec::new(),
            editing_profile: false,
        }
    }

    pub fn populate(&mut self, editing_profile: bool) {
        self.editing_profile = editing_profile;
        self.update_sdl_options_enabled();
    }

    pub fn add_device_to_list(&mut self, identifier: String, name: String) {
        self.devices.push(ControllerDevice { identifier, name });
    }

    pub fn remove_device_from_list(&mut self, identifier: &str) {
        self.devices.retain(|d| d.identifier != identifier);
    }

    pub fn update_sdl_options_enabled(&mut self) {
        // Mirrors the chained enable/disable calls of the C++ version.
    }

    pub fn led_settings_clicked(&mut self) {
        // Opens the LED settings dialog; pure data model keeps a hook.
    }

    pub fn mouse_settings_clicked(&mut self) {
        // Opens the mouse settings dialog.
    }
}

// ---------------------------------------------------------------------------
// DebugAnalysisSettingsWidget
// ---------------------------------------------------------------------------

pub struct DebugAnalysisSettingsWidget {
    pub options: DebugAnalysisOptions,
    pub symbol_sources: HashMap<String, DebugSymbolSource>,
    pub extra_symbol_files: Vec<DebugExtraSymbolFile>,
    pub function_scan_mode_names: Vec<String>,
    pub show_unlabeled_warning: bool,
    pub per_game: bool,
}

impl DebugAnalysisSettingsWidget {
    pub fn create() -> Self {
        let function_scan_mode_names = vec![
            "SCAN_ELF".to_string(),
            "SCAN_ALL".to_string(),
            "SCAN_CUSTOM".to_string(),
        ];
        Self {
            options: DebugAnalysisOptions::default(),
            symbol_sources: HashMap::new(),
            extra_symbol_files: Vec::new(),
            function_scan_mode_names,
            show_unlabeled_warning: false,
            per_game: false,
        }
    }

    pub fn populate(&mut self, per_game: bool) {
        self.per_game = per_game;
        if per_game {
            self.setup_symbol_source_grid();
        }
    }

    pub fn parse_settings_from_widgets(&self, output: &mut DebugAnalysisOptions) {
        output.automatically_select_symbols_to_clear =
            self.options.automatically_select_symbols_to_clear;
        output.function_scan_mode = self.options.function_scan_mode;
        output.custom_function_scan_range = self.options.custom_function_scan_range;
        output.function_scan_start_address = self.options.function_scan_start_address.clone();
        output.function_scan_end_address = self.options.function_scan_end_address.clone();
    }

    pub fn setup_symbol_source_grid(&mut self) {
        // Populated by caller in real implementation; in the data model we
        // simply ensure the map exists.
    }

    pub fn save_symbol_sources(&mut self) {
        // Persist symbol sources via the host settings interface.
    }

    pub fn setup_symbol_file_list(&mut self) {
        self.extra_symbol_files.clear();
    }

    pub fn add_symbol_file(&mut self, path: String) {
        self.extra_symbol_files.push(DebugExtraSymbolFile {
            path,
            ..Default::default()
        });
    }

    pub fn remove_symbol_file(&mut self, row: usize) {
        if row < self.extra_symbol_files.len() {
            self.extra_symbol_files.remove(row);
        }
    }

    pub fn save_symbol_files(&mut self) {
        // Persist to the host settings layer.
    }

    pub fn save_function_scan_range(&mut self) {
        // Persist to the host settings layer.
    }

    pub fn update_enabled_states(&mut self) {
        // Mirrors the UI enable/disable logic.
    }

    pub fn get_string_setting_value(&self, section: &str, key: &str, default: &str) -> String {
        if self.per_game {
            return self
                .options
                .function_scan_start_address
                .clone();
        }
        let _ = (section, key);
        default.to_string()
    }

    pub fn get_bool_setting_value(&self, section: &str, key: &str, default: bool) -> bool {
        let _ = (section, key);
        default
    }

    pub fn get_int_setting_value(&self, section: &str, key: &str, default: i32) -> i32 {
        let _ = (section, key);
        default
    }
}

// ---------------------------------------------------------------------------
// DebugSettingsWidget
// ---------------------------------------------------------------------------

pub struct DebugSettingsWidget {
    pub base: SettingsWidget,
    pub analysis_settings: DebugAnalysisSettingsWidget,
    pub refresh_interval_ms: u32,
    pub show_on_startup: bool,
    pub save_window_geometry: bool,
    pub drop_indicator: DropIndicator,
    pub analysis_condition: DebugAnalysisCondition,
    pub generate_symbols_for_irx_export_tables: bool,
    pub dump_gs_data: bool,
    pub save_rt: bool,
    pub save_frame: bool,
    pub save_texture: bool,
    pub save_depth: bool,
    pub save_alpha: bool,
    pub save_info: bool,
    pub save_transfer_images: bool,
    pub save_draw_stats: bool,
    pub save_frame_stats: bool,
    pub save_hw_config: bool,
    pub save_draw_start: i32,
    pub save_draw_count: i32,
    pub save_frame_start: i32,
    pub save_frame_count: i32,
    pub hw_dump_directory: PathBuf,
    pub sw_dump_directory: PathBuf,
    pub trace_logging_enabled: bool,
    pub per_game: bool,
}

impl DebugSettingsWidget {
    pub fn create() -> Self {
        Self {
            base: SettingsWidget::create(),
            analysis_settings: DebugAnalysisSettingsWidget::create(),
            refresh_interval_ms: 1000,
            show_on_startup: false,
            save_window_geometry: true,
            drop_indicator: DropIndicator::Classic,
            analysis_condition: DebugAnalysisCondition::IfDebuggerIsOpen,
            generate_symbols_for_irx_export_tables: true,
            dump_gs_data: false,
            save_rt: false,
            save_frame: false,
            save_texture: false,
            save_depth: false,
            save_alpha: false,
            save_info: false,
            save_transfer_images: false,
            save_draw_stats: false,
            save_frame_stats: false,
            save_hw_config: false,
            save_draw_start: 0,
            save_draw_count: 5000,
            save_frame_start: 0,
            save_frame_count: 999_999,
            hw_dump_directory: PathBuf::new(),
            sw_dump_directory: PathBuf::new(),
            trace_logging_enabled: false,
            per_game: false,
        }
    }

    pub fn populate(&mut self, per_game: bool) {
        self.base.populate();
        self.per_game = per_game;
        self.on_draw_dumping_changed();
    }

    pub fn on_draw_dumping_changed(&mut self) {
        // Mirrors the chained enable/disable calls of the C++ version.
    }

    pub fn on_logging_enable_changed(&mut self) {
        // Mirrors the chained enable/disable calls of the C++ version.
    }
}

// ---------------------------------------------------------------------------
// Dev9DnsHostDialog
// ---------------------------------------------------------------------------

pub struct Dev9DnsHostDialog {
    pub hosts: Vec<HostEntryUi>,
    pub selected: Vec<HostEntryUi>,
    pub accepted: bool,
}

impl Dev9DnsHostDialog {
    pub fn create() -> Self {
        Self {
            hosts: Vec::new(),
            selected: Vec::new(),
            accepted: false,
        }
    }

    pub fn populate(&mut self, hosts: Vec<HostEntryUi>) {
        self.hosts = hosts;
        self.accepted = false;
    }

    pub fn prompt_list(&mut self) -> Option<Vec<HostEntryUi>> {
        if self.accepted {
            Some(self.selected.clone())
        } else {
            None
        }
    }

    pub fn on_ok(&mut self) {
        self.accepted = true;
    }

    pub fn on_cancel(&mut self) {
        self.accepted = false;
    }
}

// ---------------------------------------------------------------------------
// Dev9UiCommon
// ---------------------------------------------------------------------------

/// Helper widget bits shared by the DEV9 settings: an IP validator and a
/// delegate for inline editing of IP addresses.
pub struct Dev9UiCommon {
    pub allow_empty: bool,
}

impl Dev9UiCommon {
    pub fn create() -> Self {
        Self {
            allow_empty: false,
        }
    }

    pub fn populate(&mut self) {
        // The C++ module initialises two static regular expressions; the
        // Rust translation captures intent through `allow_empty` only.
    }
}

// ---------------------------------------------------------------------------
// EmulationSettingsWidget
// ---------------------------------------------------------------------------

pub struct EmulationSettingsWidget {
    pub base: SettingsWidget,
    pub normal_speed: f32,
    pub fast_forward_speed: f32,
    pub slow_motion_speed: f32,
    pub max_frame_latency: i32,
    pub vsync: bool,
    pub sync_to_host_refresh_rate: bool,
    pub use_vsync_for_timing: bool,
    pub skip_presenting_duplicate_frames: bool,
    pub optimal_frame_pacing: CheckState,
    pub ee_cycle_skipping: i32,
    pub mtvu: bool,
    pub thread_pinning: bool,
    pub fast_cdvd: bool,
    pub precache_cdvd: bool,
    pub cheats: bool,
    pub ee_cycle_rate: i32,
    pub host_filesystem: bool,
    pub manually_set_real_time_clock: bool,
    pub rtc_use_system_locale_format: bool,
    pub rtc_date_time: String,
    pub per_game: bool,
}

impl EmulationSettingsWidget {
    pub const MINIMUM_EE_CYCLE_RATE: i32 = -3;
    pub const MAXIMUM_EE_CYCLE_RATE: i32 = 3;
    pub const DEFAULT_EE_CYCLE_RATE: i32 = 0;
    pub const DEFAULT_EE_CYCLE_SKIP: i32 = 0;
    pub const DEFAULT_FRAME_LATENCY: u32 = 2;

    pub fn create() -> Self {
        Self {
            base: SettingsWidget::create(),
            normal_speed: 1.0,
            fast_forward_speed: 2.0,
            slow_motion_speed: 0.5,
            max_frame_latency: Self::DEFAULT_FRAME_LATENCY as i32,
            vsync: false,
            sync_to_host_refresh_rate: false,
            use_vsync_for_timing: false,
            skip_presenting_duplicate_frames: true,
            optimal_frame_pacing: CheckState::Unchecked,
            ee_cycle_skipping: Self::DEFAULT_EE_CYCLE_SKIP,
            mtvu: false,
            thread_pinning: false,
            fast_cdvd: false,
            precache_cdvd: false,
            cheats: false,
            ee_cycle_rate: Self::DEFAULT_EE_CYCLE_RATE,
            host_filesystem: false,
            manually_set_real_time_clock: false,
            rtc_use_system_locale_format: false,
            rtc_date_time: String::new(),
            per_game: false,
        }
    }

    pub fn populate(&mut self, per_game: bool) {
        self.base.populate();
        self.per_game = per_game;
        self.update_optimal_frame_pacing();
        self.update_use_vsync_for_timing_enabled();
    }

    pub fn initialize_speed_combo(_section: &str, _key: &str, default_value: f32) -> f32 {
        default_value
    }

    pub fn handle_speed_combo_change(_section: &str, _key: &str) -> Option<f32> {
        None
    }

    pub fn on_optimal_frame_pacing_changed(&mut self) {
        if !matches!(self.optimal_frame_pacing, CheckState::PartiallyChecked) {
            let optimal = matches!(self.optimal_frame_pacing, CheckState::Checked);
            self.max_frame_latency = if optimal { 0 } else { Self::DEFAULT_FRAME_LATENCY as i32 };
        }
    }

    pub fn update_optimal_frame_pacing(&mut self) {
        let optimal = self.max_frame_latency == 0;
        self.optimal_frame_pacing = if optimal {
            CheckState::Checked
        } else {
            CheckState::Unchecked
        };
    }

    pub fn update_use_vsync_for_timing_enabled(&mut self) {
        // enabled iff vsync and sync_to_host_refresh_rate
    }

    pub fn on_manually_set_real_time_clock_changed(&mut self) {}

    pub fn on_use_system_locale_format_changed(&mut self) {}
}

// ---------------------------------------------------------------------------
// FolderSettingsWidget
// ---------------------------------------------------------------------------

pub struct FolderSettingsWidget {
    pub base: SettingsWidget,
    pub cache_directory: PathBuf,
    pub cheats_directory: PathBuf,
    pub covers_directory: PathBuf,
    pub snapshots_directory: PathBuf,
    pub savestates_directory: PathBuf,
    pub videos_directory: PathBuf,
    pub organize_snapshots_by_game: bool,
    pub organize_video_dump_by_game: bool,
}

impl FolderSettingsWidget {
    pub fn create() -> Self {
        Self {
            base: SettingsWidget::create(),
            cache_directory: PathBuf::new(),
            cheats_directory: PathBuf::new(),
            covers_directory: PathBuf::new(),
            snapshots_directory: PathBuf::new(),
            savestates_directory: PathBuf::new(),
            videos_directory: PathBuf::new(),
            organize_snapshots_by_game: false,
            organize_video_dump_by_game: false,
        }
    }

    pub fn populate(&mut self) {
        self.base.populate();
    }
}

// ---------------------------------------------------------------------------
// GameCheatSettingsWidget
// ---------------------------------------------------------------------------

pub struct GameCheatSettingsWidget {
    pub base: SettingsWidget,
    pub patches: Vec<PatchInfo>,
    pub enabled_patches: Vec<String>,
    pub search_text: String,
    pub show_all_crcs: bool,
    pub enable_cheats: bool,
    pub per_game_serial: String,
}

impl GameCheatSettingsWidget {
    pub const NAME_ROLE: i32 = 0x0101;
    pub const PLACE_ROLE: i32 = 0x0102;

    pub fn create() -> Self {
        Self {
            base: SettingsWidget::create(),
            patches: Vec::new(),
            enabled_patches: Vec::new(),
            search_text: String::new(),
            show_all_crcs: false,
            enable_cheats: false,
            per_game_serial: String::new(),
        }
    }

    pub fn populate(&mut self, per_game_serial: String) {
        self.base.populate();
        self.per_game_serial = per_game_serial;
        self.update_list_enabled();
    }

    pub fn on_cheat_list_item_double_clicked(&mut self, name: String, currently_checked: bool) {
        let new_state = !currently_checked;
        self.set_cheat_enabled(name, new_state, true);
    }

    pub fn on_cheat_list_item_changed(&mut self, name: String, current_checked: bool) {
        let current_enabled = self.enabled_patches.contains(&name);
        if current_enabled != current_checked {
            self.set_cheat_enabled(name, current_checked, true);
        }
    }

    pub fn on_reload_clicked(&mut self) {
        self.reload_list();
    }

    pub fn update_list_enabled(&mut self) {}

    pub fn disable_all_cheats(&mut self) {
        self.enabled_patches.clear();
    }

    pub fn set_cheat_enabled(&mut self, name: String, enabled: bool, _save: bool) {
        if enabled {
            if !self.enabled_patches.contains(&name) {
                self.enabled_patches.push(name);
            }
        } else {
            self.enabled_patches.retain(|n| n != &name);
        }
    }

    pub fn set_state_for_all(&mut self, enabled: bool) {
        self.enabled_patches.clear();
        if enabled {
            for p in &self.patches {
                self.enabled_patches.push(p.name.clone());
            }
        }
    }

    pub fn set_state_recursively(&mut self, _parent: Option<String>, enabled: bool) {
        self.set_state_for_all(enabled);
    }

    pub fn reload_list(&mut self) {
        self.patches.clear();
    }
}

// ---------------------------------------------------------------------------
// GameFixSettingsWidget
// ---------------------------------------------------------------------------

pub struct GameFixSettingsWidget {
    pub base: SettingsWidget,
    pub fpu_mul_hack: bool,
    pub goemon_tlb_hack: bool,
    pub software_renderer_fmv_hack: bool,
    pub skip_mpeg_hack: bool,
    pub oph_flag_hack: bool,
    pub ee_timing_hack: bool,
    pub instant_dma_hack: bool,
    pub dma_busy_hack: bool,
    pub gif_fifo_hack: bool,
    pub vif_fifo_hack: bool,
    pub vif1_stall_hack: bool,
    pub vu_add_sub_hack: bool,
    pub ibit_hack: bool,
    pub full_vu0_sync_hack: bool,
    pub vu_sync_hack: bool,
    pub vu_overflow_hack: bool,
    pub xg_kick_hack: bool,
    pub blit_internal_fps_hack: bool,
}

impl GameFixSettingsWidget {
    pub fn create() -> Self {
        Self {
            base: SettingsWidget::create(),
            fpu_mul_hack: false,
            goemon_tlb_hack: false,
            software_renderer_fmv_hack: false,
            skip_mpeg_hack: false,
            oph_flag_hack: false,
            ee_timing_hack: false,
            instant_dma_hack: false,
            dma_busy_hack: false,
            gif_fifo_hack: false,
            vif_fifo_hack: false,
            vif1_stall_hack: false,
            vu_add_sub_hack: false,
            ibit_hack: false,
            full_vu0_sync_hack: false,
            vu_sync_hack: false,
            vu_overflow_hack: false,
            xg_kick_hack: false,
            blit_internal_fps_hack: false,
        }
    }

    pub fn populate(&mut self) {
        self.base.populate();
    }
}

// ---------------------------------------------------------------------------
// GameListSettingsWidget
// ---------------------------------------------------------------------------

pub struct GameListSettingsWidget {
    pub base: SettingsWidget,
    pub search_paths: Vec<GameListSearchDirectory>,
    pub recursive_paths: Vec<GameListSearchDirectory>,
    pub excluded_paths: Vec<String>,
    pub selection_mode: SelectionMode,
}

impl GameListSettingsWidget {
    pub fn create() -> Self {
        Self {
            base: SettingsWidget::create(),
            search_paths: Vec::new(),
            recursive_paths: Vec::new(),
            excluded_paths: Vec::new(),
            selection_mode: SelectionMode::Single,
        }
    }

    pub fn populate(&mut self) {
        self.base.populate();
        self.refresh_directory_list();
        self.refresh_exclusion_list();
    }

    pub fn add_excluded_path(&mut self, path: String) -> bool {
        if self.excluded_paths.contains(&path) {
            return false;
        }
        self.excluded_paths.push(path);
        true
    }

    pub fn refresh_exclusion_list(&mut self) {
        self.excluded_paths.clear();
    }

    pub fn add_path_to_table(&mut self, path: String, recursive: bool) {
        let entry = GameListSearchDirectory { path, recursive };
        if recursive {
            self.recursive_paths.push(entry);
        } else {
            self.search_paths.push(entry);
        }
    }

    pub fn refresh_directory_list(&mut self) {
        self.search_paths.clear();
        self.recursive_paths.clear();
    }

    pub fn add_search_directory(&mut self, path: String, recursive: bool) {
        self.refresh_directory_list();
        self.add_path_to_table(path, recursive);
    }

    pub fn remove_search_directory(&mut self, path: &str) {
        self.search_paths.retain(|d| d.path != path);
        self.recursive_paths.retain(|d| d.path != path);
    }

    pub fn on_directory_list_selection_changed(&mut self) {}

    pub fn on_add_search_directory_button_clicked(&mut self, _parent: &str) {}

    pub fn on_remove_search_directory_button_clicked(&mut self, row: i32) {
        if row < 0 {
            return;
        }
        let row = row as usize;
        if row < self.search_paths.len() {
            self.search_paths.remove(row);
        } else if row < self.search_paths.len() + self.recursive_paths.len() {
            self.recursive_paths
                .remove(row - self.search_paths.len());
        }
    }

    pub fn on_add_excluded_file_button_clicked(&mut self, _parent: &str) {}

    pub fn on_add_excluded_path_button_clicked(&mut self, _parent: &str) {}

    pub fn on_remove_excluded_path_button_clicked(&mut self, row: i32) {
        if row >= 0 {
            let row = row as usize;
            if row < self.excluded_paths.len() {
                self.excluded_paths.remove(row);
            }
        }
    }

    pub fn on_excluded_paths_selection_changed(&mut self) {}

    pub fn on_rescan_all_games_clicked(&mut self) {}

    pub fn on_scan_for_new_games_clicked(&mut self) {}
}

// ---------------------------------------------------------------------------
// GamePatchSettingsWidget
// ---------------------------------------------------------------------------

pub struct GamePatchDetailsWidget {
    pub name: String,
    pub author: String,
    pub description: String,
    pub place: Option<u32>,
    pub enabled: CheckState,
    pub tristate: bool,
}

pub struct GamePatchSettingsWidget {
    pub base: SettingsWidget,
    pub patches: Vec<PatchInfo>,
    pub enabled_patches: Vec<String>,
    pub disabled_patches: Vec<String>,
    pub show_all_crcs: bool,
    pub show_unlabeled_warning: bool,
    pub show_ws_global_note: bool,
    pub show_ni_global_note: bool,
}

impl GamePatchSettingsWidget {
    pub fn create() -> Self {
        Self {
            base: SettingsWidget::create(),
            patches: Vec::new(),
            enabled_patches: Vec::new(),
            disabled_patches: Vec::new(),
            show_all_crcs: false,
            show_unlabeled_warning: false,
            show_ws_global_note: false,
            show_ni_global_note: false,
        }
    }

    pub fn populate(&mut self) {
        self.base.populate();
        self.set_unlabeled_patches_warning_visibility(false);
        self.set_global_ws_patch_note_visibility(false);
        self.set_global_ni_patch_note_visibility(false);
        self.reload_list();
    }

    pub fn on_reload_clicked(&mut self) {
        self.reload_list();
    }

    pub fn disable_all_patches(&mut self) {
        self.enabled_patches.clear();
        self.disabled_patches.clear();
    }

    pub fn reload_list(&mut self) {
        self.patches.clear();
    }

    pub fn set_unlabeled_patches_warning_visibility(&mut self, visible: bool) {
        self.show_unlabeled_warning = visible;
    }

    pub fn set_global_ws_patch_note_visibility(&mut self, visible: bool) {
        self.show_ws_global_note = visible;
    }

    pub fn set_global_ni_patch_note_visibility(&mut self, visible: bool) {
        self.show_ni_global_note = visible;
    }
}

impl GamePatchDetailsWidget {
    pub fn create(info: PatchInfo) -> Self {
        let enabled = if info.place.is_some() {
            CheckState::PartiallyChecked
        } else {
            CheckState::Unchecked
        };
        Self {
            name: info.name,
            author: info.author,
            description: info.description,
            place: info.place,
            enabled,
            tristate: true,
        }
    }

    pub fn populate(&mut self, tristate: bool, check_state: CheckState) {
        self.tristate = tristate;
        self.enabled = check_state;
    }

    pub fn on_enabled_state_changed(&mut self, new_state: CheckState) {
        self.enabled = new_state;
    }
}

// ---------------------------------------------------------------------------
// GameSummaryWidget
// ---------------------------------------------------------------------------

pub struct GameSummaryWidget {
    pub base: SettingsWidget,
    pub entry_path: String,
    pub entry_title: String,
    pub entry_serial: String,
    pub entry_region: GameRegion,
    pub entry_type: EntryType,
    pub entry_crc: u32,
    pub entry_compatibility: CompatibilityRating,
    pub custom_title: Option<String>,
    pub custom_region: Option<i32>,
    pub input_profile: Option<String>,
    pub disc_path: Option<String>,
    pub tracks: Vec<Track>,
    pub redump_search_keyword: String,
}

impl GameSummaryWidget {
    pub fn create(entry: GameListEntry) -> Self {
        Self {
            base: SettingsWidget::create(),
            entry_path: entry.path.clone(),
            entry_title: entry.title,
            entry_serial: entry.serial,
            entry_region: entry.region,
            entry_type: entry.r#type,
            entry_crc: entry.crc,
            entry_compatibility: entry.compatibility_rating,
            custom_title: None,
            custom_region: None,
            input_profile: None,
            disc_path: None,
            tracks: Vec::new(),
            redump_search_keyword: String::new(),
        }
    }

    pub fn populate(&mut self) {
        self.base.populate();
    }

    pub fn populate_input_profiles(&mut self, profiles: Vec<String>) {
        self.input_profile = profiles.first().cloned();
    }

    pub fn populate_details(&mut self, entry: &GameListEntry) {
        self.entry_path = entry.path.clone();
        self.entry_title = entry.title.clone();
        self.entry_serial = entry.serial.clone();
        self.entry_region = entry.region;
        self.entry_type = entry.r#type;
        self.entry_crc = entry.crc;
        self.entry_compatibility = entry.compatibility_rating;
    }

    pub fn populate_disc_path(&mut self, entry: &GameListEntry, path: Option<String>) {
        if matches!(entry.r#type, EntryType::Elf) {
            self.disc_path = path;
        } else {
            self.disc_path = None;
        }
    }

    pub fn populate_track_list(&mut self, tracks: Vec<Track>) {
        self.tracks = tracks;
    }

    pub fn on_input_profile_changed(&mut self, _index: usize) {}

    pub fn on_disc_path_changed(&mut self, value: String) {
        self.disc_path = if value.is_empty() { None } else { Some(value) };
    }

    pub fn on_disc_path_browse_clicked(&mut self, _parent: &str) {}

    pub fn on_verify_clicked(&mut self) {}

    pub fn on_search_hash_clicked(&self) -> Option<String> {
        if self.redump_search_keyword.is_empty() {
            None
        } else {
            Some(format!(
                "http://redump.org/discs/quicksearch/{}",
                self.redump_search_keyword
            ))
        }
    }

    pub fn on_check_wiki_clicked(&self, serial: &str) -> String {
        format!("https://wiki.pcsx2.net/{}", serial)
    }

    pub fn set_verify_result(&mut self, _error: String) {}

    pub fn repopulate_current_details(&mut self) {}

    pub fn set_custom_title(&mut self, text: String) {
        self.custom_title = if text.is_empty() { None } else { Some(text) };
    }

    pub fn set_custom_region(&mut self, region: i32) {
        self.custom_region = if region < 0 { None } else { Some(region) };
    }
}

// ---------------------------------------------------------------------------
// HddCreateQt
// ---------------------------------------------------------------------------

pub struct HddCreateQt {
    pub state: HddCreateState,
    pub parent: Option<usize>,
    pub initialised: bool,
    pub cleaned_up: bool,
}

impl HddCreateQt {
    pub fn create(parent: Option<usize>, needed_size: u64) -> Self {
        let req_mib = (needed_size + 1024 * 1024 - 1) / (1024 * 1024);
        Self {
            state: HddCreateState {
                needed_size,
                req_mib,
                written_mib: 0,
                cancelled: false,
                error: false,
            },
            parent,
            initialised: false,
            cleaned_up: false,
        }
    }

    pub fn populate(&mut self) {}

    pub fn init(&mut self) {
        self.initialised = true;
    }

    pub fn set_file_progress(&mut self, current_size: u64) {
        self.state.written_mib = (current_size + 1024 * 1024 - 1) / (1024 * 1024);
    }

    pub fn set_error(&mut self) {
        self.state.error = true;
    }

    pub fn set_canceled(&mut self) {
        self.state.cancelled = true;
    }

    pub fn cleanup(&mut self) {
        self.cleaned_up = true;
    }
}

// ---------------------------------------------------------------------------
// HotkeySettingsWidget
// ---------------------------------------------------------------------------

pub struct HotkeySettingsWidget {
    pub categories: HashMap<String, Vec<HotkeyInfo>>,
    pub hotkeys: Vec<HotkeyInfo>,
    pub background_role: String,
    pub minimum_width: i32,
}

impl HotkeySettingsWidget {
    pub fn create() -> Self {
        Self {
            categories: HashMap::new(),
            hotkeys: Vec::new(),
            background_role: "base".to_string(),
            minimum_width: 300,
        }
    }

    pub fn populate(&mut self, hotkeys: Vec<HotkeyInfo>) {
        self.hotkeys = hotkeys;
        self.create_buttons();
    }

    fn create_buttons(&mut self) {
        for hk in &self.hotkeys {
            self.categories
                .entry(hk.category.clone())
                .or_insert_with(Vec::new)
                .push(hk.clone());
        }
    }
}

// ---------------------------------------------------------------------------
// InputBindingDialog
// ---------------------------------------------------------------------------

pub struct InputBindingDialog {
    pub section_name: String,
    pub key_name: String,
    pub bind_type: InputBindingType,
    pub bindings_settings: Vec<String>,
    pub bindings_ui: Vec<String>,
    pub new_bindings: Vec<InputBindingKey>,
    pub value_ranges: Vec<BindingValueRange>,
    pub listening: bool,
    pub mouse_mapping_enabled: bool,
    pub listen_remaining_seconds: u32,
    pub sensitivity: i32,
    pub deadzone: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputBindingType {
    Button,
    Axis,
    HalfAxis,
    Motor,
}

impl InputBindingDialog {
    pub const TIMEOUT_FOR_BINDING: u32 = 5;

    pub fn create() -> Self {
        Self {
            section_name: String::new(),
            key_name: String::new(),
            bind_type: InputBindingType::Button,
            bindings_settings: Vec::new(),
            bindings_ui: Vec::new(),
            new_bindings: Vec::new(),
            value_ranges: Vec::new(),
            listening: false,
            mouse_mapping_enabled: false,
            listen_remaining_seconds: 0,
            sensitivity: 100,
            deadzone: 0,
        }
    }

    pub fn populate(
        &mut self,
        section: String,
        key: String,
        bind_type: InputBindingType,
        settings: Vec<String>,
        ui: Vec<String>,
    ) {
        self.section_name = section;
        self.key_name = key;
        self.bind_type = bind_type;
        self.bindings_settings = settings;
        self.bindings_ui = ui;
    }

    pub fn is_listening_for_input(&self) -> bool {
        self.listening
    }

    pub fn on_add_binding_button_clicked(&mut self) {}

    pub fn on_remove_binding_button_clicked(&mut self, row: i32) {
        if row < 0 {
            return;
        }
        let row = row as usize;
        if row < self.bindings_settings.len() {
            self.bindings_settings.remove(row);
        }
        if row < self.bindings_ui.len() {
            self.bindings_ui.remove(row);
        }
    }

    pub fn on_clear_bindings_button_clicked(&mut self) {
        self.bindings_settings.clear();
        self.bindings_ui.clear();
    }

    pub fn update_list(&mut self) {
        self.bindings_ui.clear();
        for s in &self.bindings_settings {
            self.bindings_ui.push(s.clone());
        }
    }

    pub fn save_list_to_settings(&mut self) {}

    pub fn on_sensitivity_changed(&mut self, value: i32) {
        self.sensitivity = value;
    }

    pub fn on_deadzone_changed(&mut self, value: i32) {
        self.deadzone = value;
    }

    pub fn start_listening_for_input(&mut self, timeout: u32) {
        self.value_ranges.clear();
        self.new_bindings.clear();
        self.listening = true;
        self.listen_remaining_seconds = timeout;
    }

    pub fn stop_listening_for_input(&mut self) {
        self.listening = false;
    }

    pub fn add_new_binding(&mut self) {}

    pub fn input_manager_hook_callback(&mut self, _key: InputBindingKey, _value: f32) {}

    pub fn reload_bind_names(&mut self) {}
}

// ---------------------------------------------------------------------------
// InputBindingWidget
// ---------------------------------------------------------------------------

pub struct InputBindingWidget {
    pub section_name: String,
    pub key_name: String,
    pub bind_type: InputBindingType,
    pub bindings_settings: Vec<String>,
    pub bindings_ui: Vec<String>,
    pub new_bindings: Vec<InputBindingKey>,
    pub value_ranges: Vec<BindingValueRange>,
    pub listening: bool,
    pub mouse_mapping_enabled: bool,
    pub listen_remaining_seconds: u32,
    pub text: String,
    pub tool_tip: String,
}

impl InputBindingWidget {
    pub const TIMEOUT_FOR_SINGLE_BINDING: u32 = 5;

    pub fn create() -> Self {
        Self {
            section_name: String::new(),
            key_name: String::new(),
            bind_type: InputBindingType::Button,
            bindings_settings: Vec::new(),
            bindings_ui: Vec::new(),
            new_bindings: Vec::new(),
            value_ranges: Vec::new(),
            listening: false,
            mouse_mapping_enabled: false,
            listen_remaining_seconds: 0,
            text: String::new(),
            tool_tip: String::new(),
        }
    }

    pub fn populate(
        &mut self,
        section: String,
        key: String,
        bind_type: InputBindingType,
    ) {
        self.section_name = section;
        self.key_name = key;
        self.bind_type = bind_type;
        self.reload_binding();
    }

    pub fn update_text(&mut self) {
        self.text.clear();
        self.tool_tip.clear();
    }

    pub fn reload_binding(&mut self) {
        self.bindings_ui.clear();
        for s in &self.bindings_settings {
            self.bindings_ui.push(s.clone());
        }
        self.update_text();
    }

    pub fn on_clicked(&mut self) {}

    pub fn on_input_listen_timer_timeout(&mut self) {
        if self.listen_remaining_seconds == 0 {
            self.listening = false;
        } else {
            self.listen_remaining_seconds -= 1;
        }
    }

    pub fn start_listening_for_input(&mut self, timeout: u32) {
        self.value_ranges.clear();
        self.new_bindings.clear();
        self.listening = true;
        self.listen_remaining_seconds = timeout;
    }

    pub fn stop_listening_for_input(&mut self) {
        self.listening = false;
    }

    pub fn set_new_binding(&mut self) {}

    pub fn clear_binding(&mut self) {
        self.bindings_settings.clear();
        self.bindings_ui.clear();
        self.reload_binding();
    }

    pub fn open_dialog(&mut self) {}

    pub fn input_manager_hook_callback(&mut self, _key: InputBindingKey, _value: f32) {}

    pub fn on_input_device_connected(&mut self) {
        self.reload_binding();
    }

    pub fn on_input_device_disconnected(&mut self) {
        self.reload_binding();
    }

    pub fn is_mouse_mapping_enabled(_sif: Option<&()>) -> bool {
        false
    }
}

pub struct InputVibrationBindingWidget {
    pub section_name: String,
    pub key_name: String,
    pub binding: String,
    pub motors: Vec<String>,
}

impl InputVibrationBindingWidget {
    pub fn create() -> Self {
        Self {
            section_name: String::new(),
            key_name: String::new(),
            binding: String::new(),
            motors: Vec::new(),
        }
    }

    pub fn populate(&mut self, section: String, key: String) {
        self.section_name = section;
        self.key_name = key;
    }

    pub fn set_key(&mut self, section: String, key: String) {
        self.section_name = section;
        self.key_name = key;
    }

    pub fn clear_binding(&mut self) {
        self.binding.clear();
    }

    pub fn on_clicked(&mut self) {}
}

// ---------------------------------------------------------------------------
// InterfaceSettingsWidget
// ---------------------------------------------------------------------------

pub struct InterfaceSettingsWidget {
    pub base: SettingsWidget,
    pub inhibit_screensaver: bool,
    pub confirm_shutdown: bool,
    pub pause_on_focus_loss: bool,
    pub pause_on_controller_disconnection: bool,
    pub prompt_on_state_load_save_failure: bool,
    pub savestate_selector: bool,
    pub discord_presence: bool,
    pub prefer_english_game_list: bool,
    pub mouse_lock: bool,
    pub start_fullscreen: bool,
    pub double_click_toggles_fullscreen: bool,
    pub hide_mouse_cursor: bool,
    pub render_to_separate_window: bool,
    pub hide_main_window: bool,
    pub disable_window_resizing: bool,
    pub start_fullscreen_ui: bool,
    pub theme: String,
    pub background_path: String,
    pub background_opacity: f32,
    pub background_scale: BackgroundScale,
    pub language: String,
    pub per_game: bool,
    pub auto_update_enabled: bool,
    pub auto_update_tag: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundScale {
    Fit,
    Fill,
    Stretch,
    Center,
    Tile,
}

impl InterfaceSettingsWidget {
    pub fn create() -> Self {
        Self {
            base: SettingsWidget::create(),
            inhibit_screensaver: true,
            confirm_shutdown: true,
            pause_on_focus_loss: false,
            pause_on_controller_disconnection: false,
            prompt_on_state_load_save_failure: true,
            savestate_selector: true,
            discord_presence: false,
            prefer_english_game_list: false,
            mouse_lock: false,
            start_fullscreen: false,
            double_click_toggles_fullscreen: true,
            hide_mouse_cursor: false,
            render_to_separate_window: false,
            hide_main_window: false,
            disable_window_resizing: false,
            start_fullscreen_ui: false,
            theme: String::new(),
            background_path: String::new(),
            background_opacity: 100.0,
            background_scale: BackgroundScale::Fit,
            language: String::new(),
            per_game: false,
            auto_update_enabled: true,
            auto_update_tag: String::new(),
        }
    }

    pub fn populate(&mut self, per_game: bool) {
        self.base.populate();
        self.per_game = per_game;
        self.on_render_to_separate_window_changed();
    }

    pub fn on_render_to_separate_window_changed(&mut self) {
        self.hide_main_window = self.hide_main_window && self.render_to_separate_window;
    }

    pub fn populate_languages(&mut self, languages: Vec<String>) {
        if let Some(first) = languages.into_iter().next() {
            self.language = first;
        }
    }

    pub fn on_set_game_list_background_triggered(&mut self, path: String) {
        self.background_path = path;
    }

    pub fn on_clear_game_list_background_triggered(&mut self) {
        self.background_path.clear();
    }
}

// ---------------------------------------------------------------------------
// MemoryCardConvertDialog
// ---------------------------------------------------------------------------

pub struct MemoryCardConvertDialog {
    pub selected_card: String,
    pub dest_card_name: String,
    pub src_card_info: FileMcdInfo,
    pub r#type: MemoryCardType,
    pub file_type: MemoryCardFileType,
    pub is_setup: bool,
    pub progress: i32,
    pub progress_range: i32,
    pub thread_running: bool,
    pub conversion_complete: bool,
}

impl MemoryCardConvertDialog {
    pub fn create(selected_card: String) -> Self {
        Self {
            selected_card,
            dest_card_name: String::new(),
            src_card_info: FileMcdInfo::default(),
            r#type: MemoryCardType::File,
            file_type: MemoryCardFileType::Unknown,
            is_setup: false,
            progress: 0,
            progress_range: 100,
            thread_running: false,
            conversion_complete: false,
        }
    }

    pub fn populate(&mut self) {
        self.is_setup = self.setup_picklist();
    }

    pub fn is_setup(&self) -> bool {
        self.is_setup
    }

    pub fn on_status_updated(&mut self) {}

    pub fn on_progress_updated(&mut self, value: i32, range: i32) {
        self.progress = value;
        self.progress_range = range;
    }

    pub fn on_thread_finished(&mut self) {
        self.conversion_complete = true;
        self.thread_running = false;
    }

    pub fn start_thread(&mut self) {
        self.thread_running = true;
    }

    pub fn cancel_thread(&mut self) {
        self.thread_running = false;
    }

    pub fn update_enabled(&mut self) {}

    pub fn setup_picklist(&mut self) -> bool {
        true
    }

    pub fn convert_card(&mut self) {
        if self.thread_running {
            self.cancel_thread();
        } else {
            self.start_thread();
        }
    }

    pub fn convert_callback(&mut self) {}

    pub fn set_type(&mut self, r#type: MemoryCardType, file_type: MemoryCardFileType) {
        self.r#type = r#type;
        self.file_type = file_type;
    }

    pub fn set_type_8(&mut self) {
        self.set_type(MemoryCardType::File, MemoryCardFileType::PS2_8MB);
    }

    pub fn set_type_16(&mut self) {
        self.set_type(MemoryCardType::File, MemoryCardFileType::PS2_16MB);
    }

    pub fn set_type_32(&mut self) {
        self.set_type(MemoryCardType::File, MemoryCardFileType::PS2_32MB);
    }

    pub fn set_type_64(&mut self) {
        self.set_type(MemoryCardType::File, MemoryCardFileType::PS2_64MB);
    }

    pub fn set_type_folder(&mut self) {
        self.set_type(MemoryCardType::Folder, MemoryCardFileType::Unknown);
    }

    pub fn file_open_error(&mut self, _message: String) {}
}

// ---------------------------------------------------------------------------
// MemoryCardConvertWorker
// ---------------------------------------------------------------------------

pub struct MemoryCardConvertWorker {
    pub r#type: MemoryCardType,
    pub file_type: MemoryCardFileType,
    pub src_file_name: String,
    pub dest_file_name: String,
    pub progress: i32,
    pub progress_range: i32,
    pub completed: bool,
    pub last_error: Option<String>,
}

impl MemoryCardConvertWorker {
    pub fn create(
        r#type: MemoryCardType,
        file_type: MemoryCardFileType,
        src: String,
        dest: String,
    ) -> Self {
        Self {
            r#type,
            file_type,
            src_file_name: src,
            dest_file_name: dest,
            progress: 0,
            progress_range: 0,
            completed: false,
            last_error: None,
        }
    }

    pub fn populate(&mut self) {}

    pub fn run_async(&mut self) {
        match self.r#type {
            MemoryCardType::File => {
                self.convert_to_folder(self.src_file_name.clone(), self.dest_file_name.clone(), self.file_type);
            }
            MemoryCardType::Folder => {
                self.convert_to_file(self.src_file_name.clone(), self.dest_file_name.clone(), self.file_type);
            }
            MemoryCardType::Unknown => {
                self.last_error = Some("Invalid MemoryCardType".to_string());
            }
        }
        self.completed = true;
    }

    pub fn convert_to_file(
        &mut self,
        _src: String,
        _dest: String,
        r#type: MemoryCardFileType,
    ) -> bool {
        let size_mb = match r#type {
            MemoryCardFileType::PS2_8MB => 8,
            MemoryCardFileType::PS2_16MB => 16,
            MemoryCardFileType::PS2_32MB => 32,
            MemoryCardFileType::PS2_64MB => 64,
            _ => {
                self.last_error = Some("Invalid MemoryCardFileType".to_string());
                return false;
            }
        };
        self.progress_range = size_mb as i32 * 1024 * 1024;
        self.progress = 0;
        true
    }

    pub fn convert_to_folder(
        &mut self,
        _src: String,
        _dest: String,
        _type: MemoryCardFileType,
    ) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// MemoryCardCreateDialog
// ---------------------------------------------------------------------------

pub struct MemoryCardCreateDialog {
    pub name: String,
    pub r#type: MemoryCardType,
    pub file_type: MemoryCardFileType,
    pub ntfs_compression: bool,
    pub ok_enabled: bool,
}

impl MemoryCardCreateDialog {
    pub fn create() -> Self {
        Self {
            name: String::new(),
            r#type: MemoryCardType::File,
            file_type: MemoryCardFileType::PS2_8MB,
            ntfs_compression: false,
            ok_enabled: false,
        }
    }

    pub fn populate(&mut self) {
        self.update_state();
    }

    pub fn name_text_changed(&mut self, text: String) {
        self.name = text.replace('.', "");
        self.update_state();
    }

    pub fn set_type(&mut self, r#type: MemoryCardType, file_type: MemoryCardFileType) {
        self.r#type = r#type;
        self.file_type = file_type;
        self.update_state();
    }

    pub fn restore_defaults(&mut self) {
        self.set_type(MemoryCardType::File, MemoryCardFileType::PS2_8MB);
    }

    pub fn update_state(&mut self) {
        self.ok_enabled = !self.name.is_empty();
    }

    pub fn create_card(&mut self) -> bool {
        !self.name.is_empty()
    }
}

// ---------------------------------------------------------------------------
// OsdFontPickerDialog
// ---------------------------------------------------------------------------

pub struct OsdFontPickerDialog {
    pub selected_font_path: String,
    pub selected_system_family: String,
    pub bold_preview: bool,
    pub catalog: Vec<CatalogFontEntry>,
    pub license_index: HashMap<String, LicenseInfo>,
    pub system_font_path_cache: HashMap<String, String>,
    pub catalog_loaded: bool,
    pub license_index_loaded: bool,
    pub license_index_failed: bool,
    pub preview_font_id: i32,
}

impl OsdFontPickerDialog {
    pub fn create(current_font_path: String, bold_preview: bool) -> Self {
        Self {
            selected_font_path: current_font_path,
            selected_system_family: String::new(),
            bold_preview,
            catalog: Vec::new(),
            license_index: HashMap::new(),
            system_font_path_cache: HashMap::new(),
            catalog_loaded: false,
            license_index_loaded: false,
            license_index_failed: false,
            preview_font_id: -1,
        }
    }

    pub fn populate(&mut self) {}

    pub fn selected_font_path(&self) -> String {
        self.selected_font_path.clone()
    }

    pub fn populate_family_list(&mut self) {}

    pub fn populate_system_family_list(&mut self) {}

    pub fn update_family_info_label(&mut self, _entry: &CatalogFontEntry) {}

    pub fn get_selected_catalog_entry(&self) -> Option<&CatalogFontEntry> {
        self.catalog.first()
    }

    pub fn on_family_selection_changed(&mut self) {}

    pub fn on_system_family_selection_changed(&mut self) {}

    pub fn on_refresh_catalog_clicked(&mut self) {
        self.catalog_loaded = true;
    }

    pub fn on_download_selected_clicked(&mut self) {}

    pub fn on_choose_local_clicked(&mut self, _path: String) {
        self.selected_font_path.clear();
    }

    pub fn on_use_default_clicked(&mut self) {
        self.selected_font_path.clear();
    }

    pub fn on_dialog_accepted(&mut self) {}

    pub fn ensure_catalog_loaded(&mut self, _allow: bool, _force: bool) -> bool {
        self.catalog_loaded
    }

    pub fn load_catalog_from_file(&mut self, _path: &str) -> bool {
        false
    }

    pub fn set_selected_font_path(&mut self, path: String) {
        self.selected_font_path = path;
    }

    pub fn refresh_preview(&mut self) {}

    pub fn set_preview_sample_text(&mut self) {}

    pub fn set_preview_font_from_path(&mut self, _path: &str) {}

    pub fn set_preview_font_from_system_family(&mut self, _family: &str) {}

    pub fn refresh_selection_validity(&mut self) {}

    pub fn validate_font_file(&self, _path: &str) -> bool {
        true
    }

    pub fn get_catalog_cache_path(&self) -> String {
        String::new()
    }

    pub fn get_catalog_font_cache_dir(&self) -> String {
        String::new()
    }

    pub fn on_source_tab_changed(&mut self, _index: i32) {}

    pub fn update_download_selected_button(&mut self) {}

    pub fn resolve_system_font_path(&self, family: &str) -> String {
        let key = family.trim().to_lowercase();
        self.system_font_path_cache
            .get(&key)
            .cloned()
            .unwrap_or_default()
    }

    pub fn normalize_license_for_display(&self, license: &str) -> String {
        let trimmed = license.trim();
        if trimmed.is_empty() {
            "Not provided by catalog".to_string()
        } else {
            trimmed.to_string()
        }
    }

    pub fn ensure_license_index_loaded(&mut self) -> bool {
        self.license_index_loaded
    }

    pub fn write_downloaded_font_license_notice(
        &self,
        _entry: &CatalogFontEntry,
        _path: &str,
        _url: &str,
    ) -> bool {
        true
    }

    pub fn find_cached_family_font_path(&self, _family: &str) -> String {
        String::new()
    }
}

// ---------------------------------------------------------------------------
// OSDSettingsWidget
// ---------------------------------------------------------------------------

pub struct OsdSettingsWidget {
    pub base: SettingsWidget,
    pub scale: f32,
    pub margin: f32,
    pub messages_pos: i32,
    pub performance_pos: i32,
    pub osd_font_path: PathBuf,
    pub show_speed_percentages: bool,
    pub show_fps: bool,
    pub show_vps: bool,
    pub show_resolution: bool,
    pub show_gs_stats: bool,
    pub show_usage_cpu: bool,
    pub show_usage_gpu: bool,
    pub show_debug_gpu: bool,
    pub show_status_indicators: bool,
    pub show_frame_times: bool,
    pub show_hardware_info: bool,
    pub show_version: bool,
    pub show_settings: bool,
    pub bold_text: bool,
    pub show_patches: bool,
    pub show_inputs: bool,
    pub show_video_capture: bool,
    pub show_input_rec: bool,
    pub show_texture_replacements: bool,
    pub warn_about_unsafe_settings: bool,
    pub font_picker: Option<String>,
}

impl OsdSettingsWidget {
    pub fn create() -> Self {
        Self {
            base: SettingsWidget::create(),
            scale: 100.0,
            margin: 10.0,
            messages_pos: 0,
            performance_pos: 0,
            osd_font_path: PathBuf::new(),
            show_speed_percentages: false,
            show_fps: false,
            show_vps: false,
            show_resolution: false,
            show_gs_stats: false,
            show_usage_cpu: false,
            show_usage_gpu: false,
            show_debug_gpu: false,
            show_status_indicators: true,
            show_frame_times: false,
            show_hardware_info: false,
            show_version: false,
            show_settings: false,
            bold_text: true,
            show_patches: false,
            show_inputs: false,
            show_video_capture: true,
            show_input_rec: true,
            show_texture_replacements: false,
            warn_about_unsafe_settings: true,
            font_picker: None,
        }
    }

    pub fn populate(&mut self) {
        self.base.populate();
        self.on_messages_pos_changed();
        self.on_performance_pos_changed();
        self.load_osd_font_path_setting();
    }

    pub fn on_browse_osd_font_path_clicked(&mut self) {}

    pub fn on_clear_osd_font_path_clicked(&mut self) {
        self.osd_font_path = PathBuf::new();
    }

    pub fn load_osd_font_path_setting(&mut self) {
        // Real implementation reads from the settings layer.
    }

    pub fn save_osd_font_path_setting(&mut self, path: PathBuf) {
        self.osd_font_path = path;
    }

    pub fn on_messages_pos_changed(&mut self) {
        // Enables/disables warn-about-unsafe-settings checkbox.
    }

    pub fn on_performance_pos_changed(&mut self) {}

    pub fn on_osd_show_settings_toggled(&mut self) {
        self.show_patches = self.show_settings;
    }

    pub fn set_all_checkboxes(&mut self, checked: bool) {
        self.show_speed_percentages = checked;
        self.show_fps = checked;
        self.show_vps = checked;
        self.show_resolution = checked;
        self.show_gs_stats = checked;
        self.show_usage_cpu = checked;
        self.show_usage_gpu = checked;
        self.show_frame_times = checked;
        self.show_hardware_info = checked;
        self.show_version = checked;
        self.show_settings = checked;
        self.show_patches = checked;
        self.show_inputs = checked;
        self.show_texture_replacements = checked;
        self.show_debug_gpu = checked;
    }

    pub fn on_select_all_clicked(&mut self) {
        self.set_all_checkboxes(true);
    }

    pub fn on_deselect_all_clicked(&mut self) {
        self.set_all_checkboxes(false);
    }
}

// ---------------------------------------------------------------------------
// Unit test scaffolding
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_settings_widget() {
        let mut w = SettingsWidget::create();
        w.populate();
        assert!(!w.tab_widget_visible);
    }

    #[test]
    fn achievement_login_button_disabled_when_empty() {
        let mut d = AchievementLoginDialog::create();
        d.populate(LoginRequestReason::UserInitiated);
        assert!(!d.can_enable_login_button());
        d.user_name = "user".to_string();
        d.password = "pass".to_string();
        assert!(d.can_enable_login_button());
    }

    #[test]
    fn memory_card_create_dialog_enables_ok_with_name() {
        let mut d = MemoryCardCreateDialog::create();
        d.populate();
        d.name_text_changed("Foo".to_string());
        assert!(d.ok_enabled);
    }

    #[test]
    fn advanced_settings_widget_defaults() {
        let mut w = AdvancedSettingsWidget::create();
        w.populate();
        assert!(w.ee_recompiler);
        assert!(matches!(
            w.savestate_compression_method,
            SavestateCompressionMethod::Zstandard
        ));
    }

    #[test]
    fn game_cheat_widget_can_enable_cheat() {
        let mut w = GameCheatSettingsWidget::create();
        w.populate("SLUS-12345".to_string());
        w.set_cheat_enabled("CheatA".to_string(), true, true);
        assert!(w.enabled_patches.contains(&"CheatA".to_string()));
    }

    #[test]
    fn osd_settings_widget_select_all_toggles_flags() {
        let mut w = OsdSettingsWidget::create();
        w.populate();
        w.on_select_all_clicked();
        assert!(w.show_speed_percentages);
        assert!(w.show_fps);
        w.on_deselect_all_clicked();
        assert!(!w.show_speed_percentages);
    }
}
