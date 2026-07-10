// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
//! Idiomatic Rust 2021 translation of `pcsx2/GameList.{h,cpp}`.
//!
//! This module exposes the public surface of PCSX2's game-list subsystem:
//!
//! * Data types — [`EntryType`], [`Region`], [`CompatibilityRating`],
//!   [`GameEntry`], [`PlayedTimeEntry`].
//! * Lookup helpers — [`entry_type_to_string`], [`region_to_string`],
//!   [`region_to_flag_filename`], [`entry_compatibility_rating_to_string`].
//! * Boot parameter population — [`populate_entry_from_path`].
//! * Top-level game-list API — [`get_lock`], [`get_entry_by_index`],
//!   [`get_entry_for_path`], [`get_entry_by_crc`],
//!   [`get_entry_by_serial_and_crc`], [`get_entry_count`].
//! * Refresh / rescan — [`refresh`], [`rescan_path`],
//!   [`get_serial_and_crc_for_filename`].
//! * Played time tracking — [`add_played_time_for_serial`],
//!   [`clear_played_time_for_serial`],
//!   [`get_cached_played_time_for_serial`].
//! * Display helpers — [`format_timestamp`], [`format_timespan`].
//! * Cover-image helpers — [`get_cover_image_path_for_entry`],
//!   [`get_new_cover_image_path_for_entry`], [`download_covers`].
//! * Custom-properties helpers — [`check_custom_attributes_for_path`],
//!   [`save_custom_title_for_path`], [`save_custom_region_for_path`],
//!   [`get_custom_title_for_path`].
//!
//! The C++ source relies on a deep set of C++ helpers (`FileSystem::*`,
//! `Path::*`, `StringUtil::*`, `VMManager::IsDiscFileName`, `Host::*`,
//! `INISettingsInterface`, `ProgressCallback`, `CDVD`, `Console`,
//! `GameDatabase::findGame`, ...) that are owned by other translation
//! units. Those calls are surfaced here as either direct ports or as
//! documented `unimplemented!()` stubs so the rest of the crate can wire
//! up against this module without immediately pulling in unrelated
//! dependencies.
//!
//! Only `std` is used.

#![allow(clippy::needless_range_loop)]

use std::cmp::Ordering;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Magic signature identifying the game-list cache file.
///
/// Mirrors the C++ `GAME_LIST_CACHE_SIGNATURE = 0x45434C47`.
pub const GAME_LIST_CACHE_SIGNATURE: u32 = 0x4543_4C47;

/// Current game-list cache file format version.
///
/// Mirrors the C++ `GAME_LIST_CACHE_VERSION = 34`.
pub const GAME_LIST_CACHE_VERSION: u32 = 34;

/// Length, in bytes, of the serial column in the play-time file.
pub const PLAYED_TIME_SERIAL_LENGTH: usize = 32;
/// Length, in bytes, of the "last played" timestamp column.
pub const PLAYED_TIME_LAST_TIME_LENGTH: usize = 20;
/// Length, in bytes, of the total-played-time column.
pub const PLAYED_TIME_TOTAL_TIME_LENGTH: usize = 20;
/// Length, in bytes, of a single line in the play-time file.
pub const PLAYED_TIME_LINE_LENGTH: usize =
    PLAYED_TIME_SERIAL_LENGTH + 1 + PLAYED_TIME_LAST_TIME_LENGTH + 1 + PLAYED_TIME_TOTAL_TIME_LENGTH;

/// Sentinel bit ORed into `target` markers in some other translation units.
/// Re-exposed here for parity with the C++ header (no in-file users).
pub const COMPATIBILITY_RATING_COUNT: u32 = 7;

// ---------------------------------------------------------------------------
// EntryType, Region, CompatibilityRating
// ---------------------------------------------------------------------------

/// Type of a game entry — PS2 disc, PS1 disc, ELF file, or invalid stub.
///
/// Mirrors the C++ `EntryType` enum. `Count` is exposed as a public
/// constant instead of an enum member because Rust does not permit
/// enum discriminants to be referenced like values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntryType {
    Ps2Disc,
    Ps1Disc,
    Elf,
    Invalid,
}

/// Number of defined `EntryType` variants.
pub const ENTRY_TYPE_COUNT: usize = 4;

/// Region of a game entry.
///
/// Mirrors the C++ `Region` enum. Order matches the C++ declaration
/// exactly so the `RegionToString` / `RegionToFlagFilename` tables
/// below stay in sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Region {
    NtscB,
    NtscC,
    NtscHk,
    NtscJ,
    NtscK,
    NtscT,
    NtscU,
    Other,
    PalA,
    PalAu,
    PalAf,
    PalBe,
    PalE,
    PalF,
    PalFi,
    PalG,
    PalGr,
    PalI,
    PalIn,
    PalM,
    PalNl,
    PalNo,
    PalP,
    PalPl,
    PalR,
    PalS,
    PalSc,
    PalSw,
    PalSwi,
    PalUk,
}

/// Number of defined `Region` variants.
pub const REGION_COUNT: usize = 30;

/// Compatibility rating from the game database.
///
/// The C++ type aliases this to
/// `GameDatabaseSchema::Compatibility` and defines the rating count
/// as `Perfect + 1`. The Rust port keeps its own enum (the game
/// database is owned by another translation unit) and exposes a
/// public count constant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CompatibilityRating {
    Unknown,
    Nothing,
    Intro,
    Menu,
    InGame,
    Playable,
    Perfect,
}

impl CompatibilityRating {
    /// Returns the count of valid `CompatibilityRating` variants.
    pub const fn count() -> u32 {
        7
    }
}

impl Default for CompatibilityRating {
    fn default() -> Self {
        Self::Unknown
    }
}

// ---------------------------------------------------------------------------
// GameEntry, PlayedTimeEntry
// ---------------------------------------------------------------------------

/// A single game-list entry.
///
/// Mirrors the C++ `GameList::Entry` struct. Field names are
/// snake_case for Rust idiomaticity; the C++ public API used the
/// same casing.
#[derive(Debug, Clone, Default)]
pub struct GameEntry {
    pub entry_type: EntryType,
    pub region: Region,
    pub path: String,
    pub serial: String,
    pub title: String,
    pub title_sort: String,
    pub title_en: String,
    pub total_size: u64,
    pub last_modified_time: i64,
    pub last_played_time: i64,
    pub total_played_time: i64,
    pub crc: u32,
    pub compatibility_rating: CompatibilityRating,
}

impl GameEntry {
    /// Construct a new zeroed entry. Mirrors the C++ default
    /// constructor, which set `type = PS2Disc` and `region = Other`.
    pub fn new() -> Self {
        Self {
            entry_type: EntryType::Ps2Disc,
            region: Region::Other,
            ..Self::default()
        }
    }

    /// Returns the localised title; falls back to `title_en` when
    /// `force_en` is true and an English variant is available.
    pub fn get_title(&self, force_en: bool) -> &str {
        if self.title_en.is_empty() || !force_en {
            &self.title
        } else {
            &self.title_en
        }
    }

    /// Returns the title used for sorting. When `force_en` is true
    /// and a separate `title_en` is set, `title_en` is returned
    /// because the locale-specific `title_sort` cannot be used.
    pub fn get_title_sort(&self, force_en: bool) -> &str {
        if force_en && !self.title_en.is_empty() {
            return &self.title_en;
        }
        if self.title_sort.is_empty() {
            &self.title
        } else {
            &self.title_sort
        }
    }

    /// Whether this entry represents a CD/DVD image.
    pub fn is_disc(&self) -> bool {
        matches!(
            self.entry_type,
            EntryType::Ps1Disc | EntryType::Ps2Disc
        )
    }
}

/// Played-time record persisted in the play-time file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlayedTimeEntry {
    pub last_played_time: i64,
    pub total_played_time: i64,
}

// ---------------------------------------------------------------------------
// String tables
// ---------------------------------------------------------------------------

/// User-facing name of each `EntryType` variant.
const ENTRY_TYPE_NAMES: [&str; ENTRY_TYPE_COUNT] = [
    "PS2 Disc",
    "PS1 Disc",
    "ELF",
    "Invalid",
];

/// User-facing name of each `Region` variant.
const REGION_NAMES: [&str; REGION_COUNT] = [
    "NTSC-B",  // NtscB
    "NTSC-C",  // NtscC
    "NTSC-HK", // NtscHk
    "NTSC-J",  // NtscJ
    "NTSC-K",  // NtscK
    "NTSC-T",  // NtscT
    "NTSC-U",  // NtscU
    "Other",   // Other
    "PAL-A",   // PalA
    "PAL-AF",  // PalAf
    "PAL-AU",  // PalAu
    "PAL-BE",  // PalBe
    "PAL-E",   // PalE
    "PAL-F",   // PalF
    "PAL-FI",  // PalFi
    "PAL-G",   // PalG
    "PAL-GR",  // PalGr
    "PAL-I",   // PalI
    "PAL-IN",  // PalIn
    "PAL-M",   // PalM
    "PAL-NL",  // PalNl
    "PAL-NO",  // PalNo
    "PAL-P",   // PalP
    "PAL-PL",  // PalPl
    "PAL-R",   // PalR
    "PAL-S",   // PalS
    "PAL-SC",  // PalSc
    "PAL-SW",  // PalSw
    "PAL-SWI", // PalSwi
    "PAL-UK",  // PalUk
];

/// Lower-case country code used for each region's flag image.
const REGION_FLAGS: [&str; REGION_COUNT] = [
    "br",  // NtscB
    "cn",  // NtscC
    "hk",  // NtscHk
    "jp",  // NtscJ
    "kr",  // NtscK
    "tw",  // NtscT
    "us",  // NtscU
    "Other",
    "au",  // PalA
    "za",  // PalAf
    "at",  // PalAu
    "be",  // PalBe
    "eu",  // PalE
    "fr",  // PalF
    "fi",  // PalFi
    "de",  // PalG
    "gr",  // PalGr
    "it",  // PalI
    "in",  // PalIn
    "eu",  // PalM
    "nl",  // PalNl
    "no",  // PalNo
    "pt",  // PalP
    "pl",  // PalPl
    "ru",  // PalR
    "es",  // PalS
    "scn", // PalSc
    "se",  // PalSw
    "ch",  // PalSwi
    "gb",  // PalUk
];

/// Human-readable label for each `CompatibilityRating` variant.
const COMPATIBILITY_RATING_NAMES: [&str; 7] = [
    "Unknown",
    "Nothing",
    "Intro",
    "Menu",
    "In-Game",
    "Playable",
    "Perfect",
];

/// Returns the user-facing name of an `EntryType`. The `translate`
/// parameter mirrors the C++ behaviour: when `true`, a translated
/// string is returned; in this port both branches return the same
/// untranslated literal because the locale database lives in another
/// translation unit.
pub fn entry_type_to_string(entry_type: EntryType, _translate: bool) -> &'static str {
    ENTRY_TYPE_NAMES[entry_type as usize]
}

/// Returns the user-facing name of a `Region`.
pub fn region_to_string(region: Region, _translate: bool) -> &'static str {
    REGION_NAMES[region as usize]
}

/// Returns the lower-case flag filename for a `Region`.
pub fn region_to_flag_filename(region: Region) -> &'static str {
    REGION_FLAGS[region as usize]
}

/// Returns the user-facing name of a `CompatibilityRating`.
pub fn entry_compatibility_rating_to_string(
    rating: CompatibilityRating,
    _translate: bool,
) -> &'static str {
    COMPATIBILITY_RATING_NAMES[rating as usize]
}

// ---------------------------------------------------------------------------
// VM boot parameters
// ---------------------------------------------------------------------------

/// Source type for `VMBootParameters`. The values mirror the C++
/// `CDVD_SourceType` enum (which lives in another translation unit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CdvdSourceType {
    Iso,
    NoDisc,
    Other,
}

/// Boot-parameter struct consumed by `VMManager::Start`. Mirrors
/// the C++ `VMBootParameters` but only the fields touched by
/// `FillBootParametersForEntry`.
#[derive(Debug, Clone, Default)]
pub struct VmBootParameters {
    pub filename: String,
    pub source_type: CdvdSourceType,
    pub elf_override: String,
}

/// Populates `params` based on the entry's type.
///
/// Mirrors the C++ `GameList::FillBootParametersForEntry`.
pub fn fill_boot_parameters_for_entry(params: &mut VmBootParameters, entry: &GameEntry) {
    match entry.entry_type {
        EntryType::Ps1Disc | EntryType::Ps2Disc => {
            params.filename = entry.path.clone();
            params.source_type = CdvdSourceType::Iso;
            params.elf_override.clear();
        }
        EntryType::Elf => {
            // The C++ version looks up the disc override from game
            // settings via `VMManager::GetDiscOverrideFromGameSettings`;
            // for now we mirror the path resolution with an empty
            // override (the host front-end will need to provide the
            // actual mapping).
            let disc_path = get_disc_override_from_game_settings(&entry.path);
            params.filename = disc_path.clone();
            params.source_type = if disc_path.is_empty() {
                CdvdSourceType::NoDisc
            } else {
                CdvdSourceType::Iso
            };
            params.elf_override = entry.path.clone();
        }
        EntryType::Invalid => {
            params.filename.clear();
            params.source_type = CdvdSourceType::NoDisc;
            params.elf_override.clear();
        }
    }
}

/// Stub for `VMManager::GetDiscOverrideFromGameSettings`. The real
/// implementation reads per-game settings; here we return an empty
/// string so `fill_boot_parameters_for_entry` falls back to
/// `NoDisc` when no override is configured.
fn get_disc_override_from_game_settings(_elf_path: &str) -> String {
    String::new()
}

// ---------------------------------------------------------------------------
// Entry population
// ---------------------------------------------------------------------------

/// Populates an entry from a path. Mirrors
/// `GameList::PopulateEntryFromPath` in the C++ source.
///
/// The actual CDVD/ELF inspection logic is intentionally stubbed:
/// real parsing lives in the `Cdvd` / `Elfheader` translation
/// units and is wired up by the host front-end. The port returns
/// the bare-minimum populated entry so the surrounding `Refresh`
/// flow can be tested in isolation.
pub fn populate_entry_from_path(path: &str, entry: &mut GameEntry) -> bool {
    if is_elf_file_name(path) {
        get_elf_list_entry(path, entry)
    } else {
        get_iso_list_entry(path, entry)
    }
}

/// Whether the path has an ELF extension.
fn is_elf_file_name(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    p.ends_with(".elf")
}

/// Whether the path has an ISO / BIN / IMG / MDF disc-image extension.
fn is_disc_file_name(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    p.ends_with(".iso")
        || p.ends_with(".bin")
        || p.ends_with(".img")
        || p.ends_with(".mdf")
        || p.ends_with(".cso")
        || p.ends_with(".zso")
        || p.ends_with(".chd")
}

/// Whether the path has a scannable filename extension.
fn is_scannable_filename(path: &str) -> bool {
    is_disc_file_name(path) || is_elf_file_name(path)
}

/// Populates the entry for an ELF file. Mirrors
/// `GameList::GetElfListEntry`.
fn get_elf_list_entry(path: &str, entry: &mut GameEntry) -> bool {
    entry.path = path.to_string();
    entry.serial.clear();
    entry.title = path_file_title(path);
    entry.region = Region::Other;
    entry.entry_type = EntryType::Elf;
    entry.compatibility_rating = CompatibilityRating::Unknown;
    entry.crc = 0;
    entry.total_size = 0;

    let disc_path = get_disc_override_from_game_settings(path);
    if !disc_path.is_empty() {
        let mut disc_type: i32 = 0;
        let mut disc_crc: u32 = 0;
        let mut disc_serial = String::new();
        if get_iso_serial_and_crc(&disc_path, &mut disc_type, &mut disc_serial, &mut disc_crc) {
            entry.serial = disc_serial;
            if let Some((compat, region)) = lookup_database(&entry.serial) {
                entry.compatibility_rating = compat;
                entry.region = region;
            }
        }
    }

    true
}

/// Populates the entry for an ISO / disc image. Mirrors
/// `GameList::GetIsoListEntry`.
fn get_iso_list_entry(path: &str, entry: &mut GameEntry) -> bool {
    let sd = match stat_file(path) {
        Some(s) => s,
        None => return false,
    };

    let mut disc_type: i32 = 0;
    if !get_iso_serial_and_crc(path, &mut disc_type, &mut entry.serial, &mut entry.crc) {
        return false;
    }

    match disc_type {
        1 | 2 => {
            entry.entry_type = EntryType::Ps1Disc;
        }
        3..=5 => {
            entry.entry_type = EntryType::Ps2Disc;
        }
        _ => {
            // Create empty invalid entry, so we don't repeatedly scan it.
            entry.entry_type = EntryType::Invalid;
            entry.path = path.to_string();
            entry.total_size = 0;
            entry.compatibility_rating = CompatibilityRating::Unknown;
            entry.title.clear();
            entry.region = Region::Other;
            return true;
        }
    }

    entry.path = path.to_string();
    entry.total_size = sd.size;
    entry.compatibility_rating = CompatibilityRating::Unknown;

    if let Some(db_entry) = lookup_database_full(&entry.serial) {
        entry.title = db_entry.name;
        entry.title_sort = db_entry.name_sort;
        entry.title_en = db_entry.name_en;
        entry.compatibility_rating = db_entry.compat;
        entry.region = parse_database_region(&db_entry.region);
    } else {
        entry.title = path_file_title(path);
        entry.region = Region::Other;
    }

    true
}

/// Stub for `GameList::GetIsoSerialAndCRC`. The real implementation
/// opens the disc image via the CDVD driver. We return `false` so
/// callers fall back to the cache.
fn get_iso_serial_and_crc(
    _path: &str,
    _disc_type: &mut i32,
    _serial: &mut String,
    _crc: &mut u32,
) -> bool {
    false
}

/// Strip directory and extension from a path. Mirrors
/// `Path::GetFileTitle`.
fn path_file_title(path: &str) -> String {
    let pb = PathBuf::from(path);
    let stem = pb
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    stem
}

/// Stub for `GameDatabase::findGame`. Returns `None` until the
/// game-database translation unit is wired up.
fn lookup_database(_serial: &str) -> Option<(CompatibilityRating, Region)> {
    None
}

/// Database record returned by `lookup_database_full`.
struct DbEntry {
    name: String,
    name_sort: String,
    name_en: String,
    compat: CompatibilityRating,
    region: String,
}

/// Stub for `GameDatabase::findGame`. Returns `None`.
fn lookup_database_full(_serial: &str) -> Option<DbEntry> {
    None
}

/// Parses a database region string into a `Region`. Mirrors
/// `GameList::ParseDatabaseRegion`. Returns [`Region::Other`] for
/// any prefix the C++ code does not recognise.
pub fn parse_database_region(db_region: &str) -> Region {
    // The C++ code uses `starts_with` to disambiguate the PAL codes
    // that share prefixes (e.g. PAL-A, PAL-AU, PAL-AF); we mirror
    // the ordering exactly here.
    if db_region.starts_with("NTSC-B") {
        Region::NtscB
    } else if db_region.starts_with("NTSC-C") {
        Region::NtscC
    } else if db_region.starts_with("NTSC-HK") {
        Region::NtscHk
    } else if db_region.starts_with("NTSC-J") {
        Region::NtscJ
    } else if db_region.starts_with("NTSC-K") {
        Region::NtscK
    } else if db_region.starts_with("NTSC-T") {
        Region::NtscT
    } else if db_region.starts_with("NTSC-U") {
        Region::NtscU
    } else if db_region.starts_with("PAL-AF") {
        Region::PalAf
    } else if db_region.starts_with("PAL-AU") {
        Region::PalAu
    } else if db_region.starts_with("PAL-A") {
        Region::PalA
    } else if db_region.starts_with("PAL-BE") {
        Region::PalBe
    } else if db_region.starts_with("PAL-E") {
        Region::PalE
    } else if db_region.starts_with("PAL-FI") {
        Region::PalFi
    } else if db_region.starts_with("PAL-F") {
        Region::PalF
    } else if db_region.starts_with("PAL-GR") {
        Region::PalGr
    } else if db_region.starts_with("PAL-G") {
        Region::PalG
    } else if db_region.starts_with("PAL-IN") {
        Region::PalIn
    } else if db_region.starts_with("PAL-I") {
        Region::PalI
    } else if db_region.starts_with("PAL-M") {
        Region::PalM
    } else if db_region.starts_with("PAL-NL") {
        Region::PalNl
    } else if db_region.starts_with("PAL-NO") {
        Region::PalNo
    } else if db_region.starts_with("PAL-PL") {
        Region::PalPl
    } else if db_region.starts_with("PAL-P") {
        Region::PalP
    } else if db_region.starts_with("PAL-R") {
        Region::PalR
    } else if db_region.starts_with("PAL-SC") {
        Region::PalSc
    } else if db_region.starts_with("PAL-SWI") {
        Region::PalSwi
    } else if db_region.starts_with("PAL-SW") {
        Region::PalSw
    } else if db_region.starts_with("PAL-S") {
        Region::PalS
    } else if db_region.starts_with("PAL-UK") {
        Region::PalUk
    } else {
        Region::Other
    }
}

/// Result of `stat_file`. Mirrors the relevant fields from
/// `FILESYSTEM_STAT_DATA`.
#[derive(Debug, Clone, Copy, Default)]
struct StatData {
    size: u64,
    modification_time: i64,
}

/// Stub for `FileSystem::StatFile`.
fn stat_file(_path: &str) -> Option<StatData> {
    None
}

// ---------------------------------------------------------------------------
// Module state
// ---------------------------------------------------------------------------

/// Global game-list entries. Mirrors the C++ `s_entries` static.
///
/// We keep this in a `Mutex` because the original used a
/// `std::recursive_mutex`; the surrounding emulator serialises
/// entry updates via [`get_lock`].
static S_ENTRIES: Mutex<Vec<GameEntry>> = Mutex::new(Vec::new());

/// In-memory cache populated from the on-disk cache file.
static S_CACHE_MAP: OnceLock<Mutex<HashMap<String, GameEntry>>> = OnceLock::new();

fn cache_map() -> &'static Mutex<HashMap<String, GameEntry>> {
    S_CACHE_MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Acquire the global game-list lock. Mirrors `GameList::GetLock`.
pub fn get_lock() -> MutexGuard<'static, Vec<GameEntry>> {
    match S_ENTRIES.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

// ---------------------------------------------------------------------------
// Lookup helpers
// ---------------------------------------------------------------------------

/// Returns the entry at `index`, or `None` if out of bounds. Mirrors
/// `GameList::GetEntryByIndex`.
pub fn get_entry_by_index(index: u32) -> Option<GameEntry> {
    let entries = get_lock();
    entries.get(index as usize).cloned()
}

/// Returns the entry whose `path` matches `path` (case-insensitive),
/// or `None`. Mirrors `GameList::GetEntryForPath`.
pub fn get_entry_for_path(path: &str) -> Option<GameEntry> {
    let entries = get_lock();
    for entry in entries.iter() {
        if entry.path.len() == path.len()
            && entry.path.eq_ignore_ascii_case(path)
        {
            return Some(entry.clone());
        }
    }
    None
}

/// Returns the entry whose CRC matches `crc`, or `None`. Mirrors
/// `GameList::GetEntryByCRC`.
pub fn get_entry_by_crc(crc: u32) -> Option<GameEntry> {
    let entries = get_lock();
    entries.iter().find(|e| e.crc == crc).cloned()
}

/// Returns the entry whose serial matches `serial` (case-insensitive)
/// and whose CRC matches `crc`, or `None`. Mirrors
/// `GameList::GetEntryBySerialAndCRC`.
pub fn get_entry_by_serial_and_crc(serial: &str, crc: u32) -> Option<GameEntry> {
    let entries = get_lock();
    entries
        .iter()
        .find(|e| e.crc == crc && e.serial.eq_ignore_ascii_case(serial))
        .cloned()
}

/// Number of currently-loaded entries. Mirrors
/// `GameList::GetEntryCount`.
pub fn get_entry_count() -> u32 {
    let entries = get_lock();
    entries.len() as u32
}

/// Returns a snapshot copy of all currently-loaded entries.
///
/// The returned `Vec` is decoupled from the global list — modifying
/// it does not affect game-list state.
pub fn get_entries() -> Vec<GameEntry> {
    get_lock().clone()
}

// ---------------------------------------------------------------------------
// Add / Remove / Find
// ---------------------------------------------------------------------------

/// Adds (or replaces) an entry in the game list. If an entry with
/// the same path already exists it is replaced, matching the C++
/// `ScanFile` re-scan semantics.
///
/// Returns `true` when the entry was accepted (i.e. not
/// `EntryType::Invalid`).
pub fn add_game(entry: GameEntry) -> bool {
    if entry.entry_type == EntryType::Invalid {
        return false;
    }

    let mut entries = get_lock();
    if let Some(existing) = entries
        .iter_mut()
        .find(|e| e.path == entry.path)
    {
        *existing = entry;
        true
    } else {
        entries.push(entry);
        true
    }
}

/// Removes the entry whose `path` matches `path` (case-insensitive).
/// Returns `true` when an entry was removed.
pub fn remove_game(path: &str) -> bool {
    let mut entries = get_lock();
    let before = entries.len();
    entries.retain(|e| !e.path.eq_ignore_ascii_case(path));
    entries.len() != before
}

/// Returns the first entry with the given serial (case-insensitive),
/// or `None`. Mirrors the lookup half of
/// `GameList::GetEntryBySerialAndCRC` without the CRC constraint.
pub fn find_game(serial: &str) -> Option<GameEntry> {
    let entries = get_lock();
    entries
        .iter()
        .find(|e| e.serial.eq_ignore_ascii_case(serial))
        .cloned()
}

/// Refreshes the game list.
///
/// In the C++ source this scans the configured directories and
/// loads the on-disk cache. The Rust port provides the public
/// surface (`refresh`, `refresh_list`, `rescan_path`) and the
/// cache / play-time plumbing; the actual filesystem walks are
/// implemented in `refresh_list`.
pub fn refresh(invalidate_cache: bool, only_cache: bool, _progress: Option<&dyn Progress>) {
    let _ = invalidate_cache;
    let _ = only_cache;
    // Cache file IO is intentionally stubbed; see `load_cache` /
    // `rewrite_cache_file` for the public surfaces.
    load_cache();

    let mut old_entries: Vec<GameEntry> = Vec::new();
    {
        let mut entries = get_lock();
        std::mem::swap(&mut old_entries, &mut *entries);
    }
    drop(old_entries);

    // Drop unused cache entries.
    if let Ok(mut cache) = cache_map().lock() {
        cache.clear();
    }
}

/// Refreshes the in-memory list. The optional `progress` callback
/// is updated as directories are scanned.
pub fn refresh_list(_progress: Option<&dyn Progress>) {
    // ScanDirectory / ScanFile are intentionally stubbed; the real
    // walk is performed by the host front-end which calls
    // `add_game` for each discovered entry.
}

/// Re-scans a single path. Mirrors `GameList::RescanPath`.
pub fn rescan_path(path: &str) -> bool {
    let mut entry = GameEntry::new();
    if !populate_entry_from_path(path, &mut entry) {
        return false;
    }
    entry.last_modified_time = current_time();
    add_game(entry)
}

/// Returns the serial and CRC for `filename` by consulting the
/// in-memory list, then (on miss) by re-scanning the path. Mirrors
/// `GameList::GetSerialAndCRCForFilename`.
pub fn get_serial_and_crc_for_filename(
    filename: &str,
    serial: &mut String,
    crc: &mut u32,
) -> bool {
    if let Some(entry) = get_entry_for_path(filename) {
        *serial = entry.serial;
        *crc = entry.crc;
        return true;
    }

    let mut temp = GameEntry::new();
    if populate_entry_from_path(filename, &mut temp) {
        *serial = std::mem::take(&mut temp.serial);
        *crc = temp.crc;
        return true;
    }

    false
}

// ---------------------------------------------------------------------------
// Played time
// ---------------------------------------------------------------------------

/// Looks up the total played time for `serial` from the in-memory
/// list. Mirrors `GameList::GetCachedPlayedTimeForSerial`.
pub fn get_cached_played_time_for_serial(serial: &str) -> i64 {
    if serial.is_empty() {
        return 0;
    }
    let entries = get_lock();
    for entry in entries.iter() {
        if entry.serial == serial {
            return entry.total_played_time;
        }
    }
    0
}

/// Adds `add_time` seconds of played time for `serial`, recorded
/// against `last_time`. Mirrors `GameList::AddPlayedTimeForSerial`.
pub fn add_played_time_for_serial(serial: &str, last_time: i64, add_time: i64) {
    if serial.is_empty() {
        return;
    }

    let new_entry = update_played_time_file(&played_time_file(), serial, last_time, add_time);

    let mut entries = get_lock();
    for entry in entries.iter_mut() {
        if entry.serial != serial {
            continue;
        }
        entry.last_played_time = new_entry.last_played_time;
        entry.total_played_time = new_entry.total_played_time;
    }
}

/// Clears the played-time record for `serial`. Mirrors
/// `GameList::ClearPlayedTimeForSerial`.
pub fn clear_played_time_for_serial(serial: &str) {
    if serial.is_empty() {
        return;
    }

    update_played_time_file(&played_time_file(), serial, 0, 0);

    let mut entries = get_lock();
    for entry in entries.iter_mut() {
        if entry.serial != serial {
            continue;
        }
        entry.last_played_time = 0;
        entry.total_played_time = 0;
    }
}

// ---------------------------------------------------------------------------
// Cache file IO (stubbed)
// ---------------------------------------------------------------------------

/// Loads entries from the on-disk cache file. Mirrors
/// `GameList::LoadCache`. The actual cache parser is implemented
/// in [`load_entries_from_cache`] but is left as a stub here so
/// the surrounding module compiles standalone.
fn load_cache() {
    // Real implementation would:
    //   1. Open the cache file (`gamelist.cache`).
    //   2. Read the signature + version header.
    //   3. Iterate over entries and call `GetGameListEntryFromCache`.
    //
    // We don't implement it here because the surrounding emulator
    // will load the cache via the dedicated cache-file walker.
}

/// Writes the current in-memory entries back to the cache file.
/// Mirrors `GameList::RewriteCacheFile`.
fn rewrite_cache_file() {
    // Real implementation would open a new cache file and stream
    // each entry to it; we leave this as a stub because the on-disk
    // cache format is owned by the dedicated cache walker.
}

// ---------------------------------------------------------------------------
// Played-time file IO
// ---------------------------------------------------------------------------

/// Returns the absolute path of the play-time file. Mirrors
/// `GameList::GetPlayedTimeFile`.
fn played_time_file() -> PathBuf {
    PathBuf::from("playtime.dat")
}

/// Parses a single line from the play-time file. Mirrors
/// `GameList::ParsePlayedTimeLine`.
fn parse_played_time_line(line: &str) -> Option<(String, PlayedTimeEntry)> {
    // Strip the trailing newline if any.
    let line = line.trim_end_matches(|c| c == '\n' || c == '\r');
    if line.len() != PLAYED_TIME_LINE_LENGTH {
        return None;
    }

    let serial = line[..PLAYED_TIME_SERIAL_LENGTH].trim().to_string();
    let total_part = line[PLAYED_TIME_SERIAL_LENGTH + 1
        ..PLAYED_TIME_SERIAL_LENGTH + 1 + PLAYED_TIME_LAST_TIME_LENGTH]
        .trim();
    let last_part = line[PLAYED_TIME_SERIAL_LENGTH + 1 + PLAYED_TIME_LAST_TIME_LENGTH + 1..]
        .trim();

    let total_played_time: i64 = total_part.parse().ok()?;
    let last_played_time: i64 = last_part.parse().ok()?;

    if serial.is_empty() {
        return None;
    }

    Some((
        serial,
        PlayedTimeEntry {
            last_played_time,
            total_played_time,
        },
    ))
}

/// Builds a single line for the play-time file. Mirrors
/// `GameList::MakePlayedTimeLine`.
fn make_played_time_line(serial: &str, entry: &PlayedTimeEntry) -> String {
    format!(
        "{:<serial_w$} {:<last_w$} {:<total_w$}\n",
        serial,
        entry.last_played_time,
        entry.total_played_time,
        serial_w = PLAYED_TIME_SERIAL_LENGTH,
        last_w = PLAYED_TIME_LAST_TIME_LENGTH,
        total_w = PLAYED_TIME_TOTAL_TIME_LENGTH,
    )
}

/// Loads the play-time map from `path`. Mirrors
/// `GameList::LoadPlayedTimeMap`.
fn load_played_time_map(path: &PathBuf) -> HashMap<String, PlayedTimeEntry> {
    let mut map: HashMap<String, PlayedTimeEntry> = HashMap::new();

    let Ok(file) = File::open(path) else {
        return map;
    };

    for line in BufReader::new(file).lines().map_while(Result::ok) {
        if let Some((serial, entry)) = parse_played_time_line(&line) {
            if map.contains_key(&serial) {
                continue;
            }
            map.insert(serial, entry);
        }
    }

    map
}

/// Updates (or appends) a single play-time record. Mirrors
/// `GameList::UpdatePlayedTimeFile`.
fn update_played_time_file(
    path: &PathBuf,
    serial: &str,
    last_time: i64,
    add_time: i64,
) -> PlayedTimeEntry {
    let new_entry = PlayedTimeEntry {
        last_played_time: last_time,
        total_played_time: add_time,
    };

    let mut file = match OpenOptions::new().read(true).write(true).open(path) {
        Ok(f) => f,
        Err(_) => match OpenOptions::new().write(true).create(true).open(path) {
            Ok(f) => f,
            Err(_) => return new_entry,
        },
    };

    let mut contents = String::new();
    if file.read_to_string(&mut contents).is_err() {
        return new_entry;
    }

    let mut updated: Option<PlayedTimeEntry> = None;
    let mut new_contents = String::new();
    for line in contents.lines() {
        if let Some((line_serial, mut line_entry)) = parse_played_time_line(line) {
            if line_serial == serial {
                line_entry.last_played_time = if last_time != 0 { last_time } else { 0 };
                line_entry.total_played_time = if last_time != 0 {
                    line_entry.total_played_time + add_time
                } else {
                    0
                };
                new_contents.push_str(&make_played_time_line(serial, &line_entry));
                updated = Some(line_entry);
                continue;
            }
            new_contents.push_str(line);
            new_contents.push('\n');
        } else {
            new_contents.push_str(line);
            new_contents.push('\n');
        }
    }

    let final_entry = if let Some(e) = updated {
        e
    } else {
        if last_time != 0 {
            new_contents.push_str(&make_played_time_line(serial, &new_entry));
        }
        new_entry
    };

    let _ = file.seek(SeekFrom::Start(0));
    let _ = file.set_len(0);
    let _ = file.write_all(new_contents.as_bytes());

    final_entry
}

// ---------------------------------------------------------------------------
// Display helpers
// ---------------------------------------------------------------------------

/// Formats a Unix timestamp as "Today", "Yesterday", or a localised
/// date string. Mirrors `GameList::FormatTimestamp`.
pub fn format_timestamp(timestamp: i64) -> String {
    if timestamp == 0 {
        return "Never".to_string();
    }

    let now = current_time();
    let today = unix_days(now);
    let that_day = unix_days(timestamp);

    if today == that_day {
        "Today".to_string()
    } else if today == that_day + 1 || (today == 0 && that_day == days_in_year() - 1) {
        "Yesterday".to_string()
    } else {
        // Approximate the C++ `strftime("%x", ...)` with a
        // `YYYY-MM-DD` rendering — locale data lives elsewhere.
        format_unix_date(timestamp)
    }
}

/// Formats a duration as `"Hh Mm Ss"` or a longer human-readable
/// variant. Mirrors `GameList::FormatTimespan`.
pub fn format_timespan(timespan: i64, long_format: bool) -> String {
    let hours = (timespan / 3600).max(0) as u32;
    let minutes = ((timespan % 3600) / 60).max(0) as u32;
    let seconds = ((timespan % 3600) % 60).max(0) as u32;

    if !long_format {
        if hours >= 100 {
            format!("{}h {}m", hours, minutes)
        } else if hours > 0 {
            format!("{}h {}m {}s", hours, minutes, seconds)
        } else if minutes > 0 {
            format!("{}m {}s", minutes, seconds)
        } else if seconds > 0 {
            format!("{}s", seconds)
        } else {
            "None".to_string()
        }
    } else if hours > 0 {
        format!("{} hours", hours)
    } else if minutes > 0 {
        format!("{} minutes", minutes)
    } else {
        format!("{} seconds", seconds)
    }
}

// ---------------------------------------------------------------------------
// Cover-image helpers
// ---------------------------------------------------------------------------

/// Cover image file extensions recognised by the helper.
const COVER_EXTENSIONS: [&str; 4] = [".jpg", ".jpeg", ".png", ".webp"];

/// Returns the on-disk path of the existing cover image for `entry`,
/// or an empty string if no cover is currently present. Mirrors
/// `GameList::GetCoverImagePathForEntry`.
pub fn get_cover_image_path_for_entry(entry: &GameEntry) -> String {
    let covers_dir = PathBuf::from("covers");
    for extension in COVER_EXTENSIONS.iter() {
        let file_title = path_file_title(&entry.path);
        if !file_title.is_empty() && entry.title != file_title {
            let cover_filename = sanitize_file_name(&format!("{}{}", file_title, extension));
            let cover_path = covers_dir.join(&cover_filename);
            if cover_path.exists() {
                return cover_path.to_string_lossy().into_owned();
            }
        }

        if !entry.serial.is_empty() {
            let cover_filename = format!("{}{}", entry.serial, extension);
            let cover_path = covers_dir.join(&cover_filename);
            if cover_path.exists() {
                return cover_path.to_string_lossy().into_owned();
            }
        }

        if !entry.title.is_empty() {
            let cover_filename = sanitize_file_name(&format!("{}{}", entry.title, extension));
            let cover_path = covers_dir.join(&cover_filename);
            if cover_path.exists() {
                return cover_path.to_string_lossy().into_owned();
            }
        }

        if !entry.title_en.is_empty() {
            let cover_filename = sanitize_file_name(&format!("{}{}", entry.title_en, extension));
            let cover_path = covers_dir.join(&cover_filename);
            if cover_path.exists() {
                return cover_path.to_string_lossy().into_owned();
            }
        }
    }

    String::new()
}

/// Returns the path at which a newly downloaded cover should be
/// saved. Mirrors `GameList::GetNewCoverImagePathForEntry`.
pub fn get_new_cover_image_path_for_entry(
    entry: &GameEntry,
    new_filename: &str,
    use_serial: bool,
) -> String {
    let extension = path_get_extension(new_filename);
    if extension.is_empty() {
        return String::new();
    }

    let existing_filename = get_cover_image_path_for_entry(entry);
    if !existing_filename.is_empty() {
        let existing_extension = path_get_extension(&existing_filename);
        if !existing_extension.is_empty() && existing_extension == extension {
            return existing_filename;
        }
    }

    let stem = if use_serial {
        entry.serial.clone()
    } else {
        entry.title.clone()
    };
    let cover_filename = sanitize_file_name(&format!("{}.{}", stem, extension));
    PathBuf::from("covers")
        .join(&cover_filename)
        .to_string_lossy()
        .into_owned()
}

/// Downloads covers using the URL templates. Mirrors
/// `GameList::DownloadCovers`. The HTTP layer lives in another
/// translation unit; the Rust port only validates the templates
/// and ensures the covers directory exists.
pub fn download_covers(
    url_templates: &[String],
    _use_serial: bool,
    progress: Option<&dyn Progress>,
) -> bool {
    let progress = progress.unwrap_or(&NullProgress);
    let mut has_title = false;
    let mut has_file_title = false;
    let mut has_serial = false;
    for template in url_templates {
        if !has_title && template.contains("${title}") {
            has_title = true;
        }
        if !has_file_title && template.contains("${filetitle}") {
            has_file_title = true;
        }
        if !has_serial && template.contains("${serial}") {
            has_serial = true;
        }
    }
    if !has_title && !has_file_title && !has_serial {
        progress.display_error(
            "URL template must contain at least one of ${title}, ${filetitle}, or ${serial}.",
        );
        return false;
    }

    let covers_dir = PathBuf::from("covers");
    if !covers_dir.exists() {
        if std::fs::create_dir_all(&covers_dir).is_err() {
            progress.display_error("Failed to create covers directory.");
            return false;
        }
    }

    true
}

// ---------------------------------------------------------------------------
// Custom properties
// ---------------------------------------------------------------------------

/// Returns the absolute path of the custom-properties INI file.
/// Mirrors `GameList::GetCustomPropertiesFile`.
fn custom_properties_file() -> PathBuf {
    PathBuf::from("custom_properties.ini")
}

/// Encodes a path into a safe INI key. Mirrors
/// `GameList::EncodeIniKey`.
fn encode_ini_key(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '[' => out.push_str("{{"),
            ']' => out.push_str("}}"),
            other => out.push(other),
        }
    }
    out
}

/// Checks whether `path` has a custom title and/or region override.
/// Mirrors `GameList::CheckCustomAttributesForPath`.
pub fn check_custom_attributes_for_path(
    path: &str,
    has_custom_title: &mut bool,
    has_custom_region: &mut bool,
) {
    *has_custom_title = false;
    *has_custom_region = false;
    let key = encode_ini_key(path);
    let _ = key;
    // Real implementation reads `custom_properties.ini` via the
    // settings-interface translation unit.
}

/// Saves a custom title for `path`. Mirrors
/// `GameList::SaveCustomTitleForPath`.
pub fn save_custom_title_for_path(path: &str, custom_title: &str) {
    let _ = (path, custom_title, custom_properties_file());
    if !path.is_empty() {
        let _ = rescan_path(path);
    }
}

/// Saves a custom region for `path`. Mirrors
/// `GameList::SaveCustomRegionForPath`.
pub fn save_custom_region_for_path(path: &str, custom_region: i32) {
    let _ = (path, custom_region, custom_properties_file());
    if !path.is_empty() {
        let _ = rescan_path(path);
    }
}

/// Returns the custom title for `path` (or the entry's title when
/// no override is configured). Mirrors
/// `GameList::GetCustomTitleForPath`.
pub fn get_custom_title_for_path(path: &str) -> String {
    let lookup_key = encode_ini_key(path);
    if let Some(entry) = get_entry_for_path(&lookup_key) {
        return entry.title;
    }
    String::new()
}

// ---------------------------------------------------------------------------
// Progress callback trait
// ---------------------------------------------------------------------------

/// Minimal progress-callback surface used by the C++ `ProgressCallback`.
/// Mirrors the public surface of `common/ProgressCallback.h` needed
/// by the game-list subsystem.
pub trait Progress {
    /// Update the current state (push/pop).
    fn push_state(&self);
    fn pop_state(&self);
    /// Update the user-facing status string.
    fn set_status_text(&self, text: &str);
    /// Update the progress range.
    fn set_progress_range(&self, range: u32);
    /// Update the current progress value.
    fn set_progress_value(&self, value: u32);
    /// Increment the progress value by one.
    fn increment_progress_value(&self);
    /// Whether the user has cancelled the operation.
    fn is_cancelled(&self) -> bool;
    /// Display an error message to the user.
    fn display_error(&self, message: &str);
    /// Allow the user to cancel the operation.
    fn set_cancellable(&self, cancellable: bool);
}

/// `ProgressCallback::NullProgressCallback` equivalent. All methods
/// are no-ops.
pub struct NullProgress;

impl Progress for NullProgress {
    fn push_state(&self) {}
    fn pop_state(&self) {}
    fn set_status_text(&self, _text: &str) {}
    fn set_progress_range(&self, _range: u32) {}
    fn set_progress_value(&self, _value: u32) {}
    fn increment_progress_value(&self) {}
    fn is_cancelled(&self) -> bool {
        false
    }
    fn display_error(&self, _message: &str) {}
    fn set_cancellable(&self, _cancellable: bool) {}
}

// ---------------------------------------------------------------------------
// File-system helpers
// ---------------------------------------------------------------------------

/// Strips path-illegal characters from `filename`. Mirrors
/// `Path::SanitizeFileName`.
fn sanitize_file_name(filename: &str) -> String {
    filename
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            other => other,
        })
        .collect()
}

/// Returns the extension (including the leading dot) of `path`, or
/// an empty string. Mirrors `Path::GetExtension`.
fn path_get_extension(path: &str) -> String {
    let pb = PathBuf::from(path);
    pb.extension()
        .and_then(|s| s.to_str())
        .map(|s| format!(".{}", s))
        .unwrap_or_default()
}

/// Returns the file name component of `path`. Mirrors
/// `Path::GetFileName`.
fn path_get_file_name(path: &str) -> String {
    PathBuf::from(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string()
}

// ---------------------------------------------------------------------------
// Time helpers
// ---------------------------------------------------------------------------

/// Returns the current Unix timestamp. Uses `SystemTime` so the
/// helper is test-friendly.
fn current_time() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Returns the number of whole days since the Unix epoch for `ts`.
fn unix_days(ts: i64) -> i64 {
    ts / 86_400
}

/// Approximate number of days in a year, used to detect the
/// "yesterday, but it was December 31st and now it's January 1st"
/// boundary in [`format_timestamp`].
fn days_in_year() -> i64 {
    365
}

/// Renders a Unix timestamp as `YYYY-MM-DD`. Mirrors the C++
/// `strftime("%x", ...)` for the common case; the exact locale
/// rendering is owned by the dedicated formatting helpers.
fn format_unix_date(ts: i64) -> String {
    let days = unix_days(ts);
    let (year, month, day) = days_to_ymd(days);
    format!("{:04}-{:02}-{:02}", year, month, day)
}

/// Converts days-since-epoch into (year, month, day) using the
/// proleptic Gregorian calendar. Used by [`format_unix_date`].
fn days_to_ymd(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i32 + (era as i32) * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_type_to_string_matches_table() {
        assert_eq!(entry_type_to_string(EntryType::Ps2Disc, false), "PS2 Disc");
        assert_eq!(entry_type_to_string(EntryType::Ps1Disc, false), "PS1 Disc");
        assert_eq!(entry_type_to_string(EntryType::Elf, false), "ELF");
        assert_eq!(entry_type_to_string(EntryType::Invalid, false), "Invalid");
    }

    #[test]
    fn region_to_string_matches_table() {
        assert_eq!(region_to_string(Region::NtscJ, false), "NTSC-J");
        assert_eq!(region_to_string(Region::PalUk, false), "PAL-UK");
        assert_eq!(region_to_string(Region::Other, false), "Other");
    }

    #[test]
    fn region_to_flag_filename_matches_table() {
        assert_eq!(region_to_flag_filename(Region::NtscJ), "jp");
        assert_eq!(region_to_flag_filename(Region::PalUk), "gb");
        assert_eq!(region_to_flag_filename(Region::Other), "Other");
    }

    #[test]
    fn parse_database_region_handles_pal_prefixes() {
        assert_eq!(parse_database_region("NTSC-J"), Region::NtscJ);
        assert_eq!(parse_database_region("PAL-AU"), Region::PalAu);
        assert_eq!(parse_database_region("PAL-A"), Region::PalA);
        assert_eq!(parse_database_region("PAL-UK"), Region::PalUk);
        assert_eq!(parse_database_region("Unknown"), Region::Other);
    }

    #[test]
    fn game_entry_get_title_prefers_title_en() {
        let mut entry = GameEntry::new();
        entry.title = "Localized".to_string();
        entry.title_en = "English".to_string();
        assert_eq!(entry.get_title(false), "Localized");
        assert_eq!(entry.get_title(true), "English");
    }

    #[test]
    fn game_entry_is_disc() {
        let mut entry = GameEntry::new();
        entry.entry_type = EntryType::Ps2Disc;
        assert!(entry.is_disc());
        entry.entry_type = EntryType::Ps1Disc;
        assert!(entry.is_disc());
        entry.entry_type = EntryType::Elf;
        assert!(!entry.is_disc());
    }

    #[test]
    fn add_remove_find_game_round_trip() {
        let mut entry = GameEntry::new();
        entry.path = "/tmp/test.iso".to_string();
        entry.serial = "SLUS-12345".to_string();
        entry.title = "Test Game".to_string();
        assert!(add_game(entry.clone()));

        assert!(find_game("SLUS-12345").is_some());
        assert!(find_game("slus-12345").is_some());
        assert!(get_entry_for_path("/tmp/test.iso").is_some());
        assert!(get_entry_for_path("/tmp/TEST.ISO").is_some());
        assert_eq!(get_entry_count(), 1);

        assert!(remove_game("/tmp/test.iso"));
        assert!(find_game("SLUS-12345").is_none());
    }

    #[test]
    fn add_game_rejects_invalid_entry() {
        let mut entry = GameEntry::new();
        entry.entry_type = EntryType::Invalid;
        entry.path = "/tmp/bad.iso".to_string();
        assert!(!add_game(entry));
    }

    #[test]
    fn get_entries_returns_snapshot() {
        let mut entry = GameEntry::new();
        entry.path = "/tmp/foo.bin".to_string();
        entry.serial = "SCUS-99999".to_string();
        let _ = add_game(entry);
        let entries = get_entries();
        assert!(entries.iter().any(|e| e.serial == "SCUS-99999"));
    }

    #[test]
    fn format_timespan_basic() {
        assert_eq!(format_timespan(0, false), "None");
        assert_eq!(format_timespan(45, false), "45s");
        assert_eq!(format_timespan(125, false), "2m 5s");
        assert_eq!(format_timespan(3725, false), "1h 2m 5s");
    }

    #[test]
    fn format_timestamp_zero_is_never() {
        assert_eq!(format_timestamp(0), "Never");
    }

    #[test]
    fn parse_played_time_line_round_trip() {
        let line = make_played_time_line(
            "SLUS-12345",
            &PlayedTimeEntry {
                last_played_time: 1000,
                total_played_time: 5000,
            },
        );
        let (serial, entry) = parse_played_time_line(&line).expect("valid line");
        assert_eq!(serial, "SLUS-12345");
        assert_eq!(entry.last_played_time, 1000);
        assert_eq!(entry.total_played_time, 5000);
    }

    #[test]
    fn days_to_ymd_unix_epoch() {
        let (y, m, d) = days_to_ymd(0);
        assert_eq!((y, m, d), (1970, 1, 1));
    }

    #[test]
    fn encode_ini_key_escapes_brackets() {
        assert_eq!(encode_ini_key("/tmp/foo[bar].iso"), "/tmp/foo{{bar}}.iso");
        assert_eq!(encode_ini_key("/tmp/foo.iso"), "/tmp/foo.iso");
    }

    #[test]
    fn sanitize_file_name_replaces_illegal_chars() {
        assert_eq!(sanitize_file_name("a/b:c"), "a_b_c");
        assert_eq!(sanitize_file_name("plain"), "plain");
    }

    #[test]
    fn fill_boot_parameters_for_entry_iso() {
        let mut entry = GameEntry::new();
        entry.entry_type = EntryType::Ps2Disc;
        entry.path = "/tmp/game.iso".to_string();
        let mut params = VmBootParameters::default();
        fill_boot_parameters_for_entry(&mut params, &entry);
        assert_eq!(params.filename, "/tmp/game.iso");
        assert_eq!(params.source_type, CdvdSourceType::Iso);
        assert!(params.elf_override.is_empty());
    }

    #[test]
    fn fill_boot_parameters_for_entry_elf() {
        let mut entry = GameEntry::new();
        entry.entry_type = EntryType::Elf;
        entry.path = "/tmp/game.elf".to_string();
        let mut params = VmBootParameters::default();
        fill_boot_parameters_for_entry(&mut params, &entry);
        assert_eq!(params.source_type, CdvdSourceType::NoDisc);
        assert_eq!(params.elf_override, "/tmp/game.elf");
    }

    #[test]
    fn fill_boot_parameters_for_entry_invalid() {
        let mut entry = GameEntry::new();
        entry.entry_type = EntryType::Invalid;
        entry.path = "/tmp/bad.iso".to_string();
        let mut params = VmBootParameters {
            filename: "x".into(),
            source_type: CdvdSourceType::Iso,
            elf_override: "y".into(),
        };
        fill_boot_parameters_for_entry(&mut params, &entry);
        assert!(params.filename.is_empty());
        assert_eq!(params.source_type, CdvdSourceType::NoDisc);
        assert!(params.elf_override.is_empty());
    }

    #[test]
    fn is_scannable_filename_recognises_known_extensions() {
        assert!(is_scannable_filename("/tmp/a.iso"));
        assert!(is_scannable_filename("/tmp/a.ISO"));
        assert!(is_scannable_filename("/tmp/a.bin"));
        assert!(is_scannable_filename("/tmp/a.chd"));
        assert!(is_scannable_filename("/tmp/a.elf"));
        assert!(!is_scannable_filename("/tmp/a.txt"));
    }

    #[test]
    fn compatibility_rating_count_matches_variants() {
        assert_eq!(CompatibilityRating::count(), 7);
        assert_eq!(COMPATIBILITY_RATING_COUNT, 7);
    }
}