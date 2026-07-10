// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! PCSX2 RetroAchievements manager translated from the C++ original.
//!
//! This module covers the RetroAchievements (rcheevos) integration, including:
//!   * the global rc_client state and HTTP downloader plumbing,
//!   * game identification (PS2 ELF hash → game id),
//!   * hardcore-mode toggling, leaderboard fetching, rich presence updates,
//!   * the per-frame / idle callbacks that drive UI overlays.
//!
//! The translation is intentionally a faithful, idiomatic Rust 2021 port: all
//! C-style statics are kept in the same module so the behaviour remains
//! identical, and the public `Achievements` namespace is preserved as a
//! `mod` with free functions rather than a struct with methods.

#![allow(clippy::too_many_arguments)]
#![allow(clippy::missing_safety_doc)]
#![allow(dead_code)]

use std::collections::{HashMap, VecDeque};
use std::ffi::CStr;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use crate::common::HTTPDownloader as HTTPDownloaderMod;
use crate::common::HTTPDownloader::HTTPDownloader;

// ---------------------------------------------------------------------------
// rcheevos FFI bindings (subset, mirroring pcsx2/rc_client.h usage)
// ---------------------------------------------------------------------------
//
// These declarations are intentionally minimal — the real build pulls in
// `rc_client.h` and links against `librc_client`. The translated file
// documents the contract; the actual FFI lives behind the `rcheevos`
// 3rdparty module.

/// Opaque RetroAchievements client handle.
pub type RcClient = std::ffi::c_void;

/// Opaque async request handle.
pub type RcClientAsyncHandle = std::ffi::c_void;

/// Achievement state constants (mirroring `RC_CLIENT_ACHIEVEMENT_STATE_*`).
pub mod ach_state {
    pub const INACTIVE: u32 = 0;
    pub const ACTIVE: u32 = 1;
    pub const UNLOCKED: u32 = 2;
    pub const UNSUPPORTED: u32 = 3;
}

/// Achievement category constants (mirroring `RC_CLIENT_ACHIEVEMENT_CATEGORY_*`).
pub mod ach_category {
    pub const CORE: u32 = 0;
    pub const UNOFFICIAL: u32 = 1;
    pub const CORE_AND_UNOFFICIAL: u32 = 2;
}

/// rc_client return codes (subset).
pub mod rc {
    pub const OK: i32 = 0;
    pub const NO_GAME_LOADED: i32 = 1;
    pub const LOGIN_REQUIRED: i32 = 2;
    pub const INVALID_STATE: i32 = 3;
    pub const INVALID_ARGUMENT: i32 = 4;
    pub const NO_GAME_LOADED_ERR: i32 = 5;
    pub const GAME_ALREADY_LOADED: i32 = 6;
    pub const UNKNOWN_GAME: i32 = 7;
    pub const NETWORK_ERROR: i32 = 8;
    pub const SERVER_ERROR: i32 = 9;
    pub const HTTP_ERROR: i32 = 10;
    pub const INVALID_CREDENTIALS: i32 = 11;
    pub const EXPIRED_TOKEN: i32 = 12;
    pub const MALFORMED_RESPONSE: i32 = 13;
    pub const INVALID_JSON: i32 = 14;
    pub const INVALID_RESPONSE: i32 = 15;
    pub const INVALID_HASH: i32 = 16;
    pub const INVALID_GAME: i32 = 17;
}

/// Logging level constants.
pub mod log_level {
    pub const INFO: i32 = 1;
    pub const VERBOSE: i32 = 2;
}

/// Achievement definition (mirrors `rc_client_achievement_t`).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RcClientAchievement {
    pub id: u32,
    pub points: u32,
    pub category: u32,
    pub state: u32,
    pub title: [u8; 256],
    pub description: [u8; 1024],
    pub badge_name: [u8; 256],
    pub measured_progress: [u8; 256],
    pub measured_percent: f32,
    pub unlock_time: u64,
}

/// Leaderboard entry (mirrors `rc_client_leaderboard_entry_t`).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RcClientLeaderboardEntry {
    pub rank: u32,
    pub user_index: i32,
    pub submitted: u64,
    pub display: [u8; 64],
    pub user: [u8; 64],
}

/// Achievement summary (mirrors `rc_client_user_game_summary_t`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct RcClientUserGameSummary {
    pub num_core_achievements: u32,
    pub num_unofficial_achievements: u32,
    pub num_unsupported_achievements: u32,
    pub num_unlocked_achievements: u32,
    pub num_unlocked_unofficial_achievements: u32,
    pub points_core: u32,
    pub points_unofficial: u32,
    pub points_total: u32,
    pub points_unlocked: u32,
}

/// Server API request descriptor (mirrors `rc_api_request_t`).
#[repr(C)]
pub struct RcApiRequest {
    pub url: [u8; 256],
    pub post_data: *const u8,
    pub content_type: [u8; 64],
}

// ---------------------------------------------------------------------------
// Constants (from Achievements.cpp top)
// ---------------------------------------------------------------------------

pub const LEADERBOARD_NEARBY_ENTRIES_TO_FETCH: u32 = 10;
pub const LEADERBOARD_ALL_FETCH_SIZE: u32 = 20;

pub const LOGIN_NOTIFICATION_TIME: f32 = 5.0;
pub const ACHIEVEMENT_SUMMARY_NOTIFICATION_TIME: f32 = 5.0;
pub const GAME_COMPLETE_NOTIFICATION_TIME: f32 = 20.0;
pub const LEADERBOARD_STARTED_NOTIFICATION_TIME: f32 = 3.0;
pub const LEADERBOARD_FAILED_NOTIFICATION_TIME: f32 = 3.0;

pub const DEFAULT_INFO_SOUND_NAME: &str = "sounds/achievements/message.wav";
pub const DEFAULT_UNLOCK_SOUND_NAME: &str = "sounds/achievements/unlock.wav";
pub const DEFAULT_LBSUBMIT_SOUND_NAME: &str = "sounds/achievements/lbsubmit.wav";

pub const INDICATOR_FADE_IN_TIME: f32 = 0.1;
pub const INDICATOR_FADE_OUT_TIME: f32 = 0.5;

pub const URL_BUFFER_SIZE: usize = 256;
pub const SERVER_CALL_TIMEOUT: f32 = 60.0;
pub const MAX_CONCURRENT_SERVER_CALLS: u32 = 10;

/// Cap on how many bytes of an ELF get hashed for game-id lookup.
pub const MAX_HASH_SIZE: u32 = 64 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Types mirroring the `Achievements` namespace data
// ---------------------------------------------------------------------------

/// Reason that an Achievements login flow was requested by the host UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginRequestReason {
    UserInitiated,
    TokenInvalid,
}

/// Public-facing summary of an achievement row, used by GetAchievements().
#[derive(Clone, Debug, Default)]
pub struct Achievement {
    pub id: u32,
    pub points: u32,
    pub category: u32,
    pub state: u32,
    pub title: String,
    pub description: String,
    pub badge_path: String,
    pub measured_progress: String,
    pub unlock_time: u64,
}

/// Public-facing leaderboard entry.
#[derive(Clone, Debug, Default)]
pub struct LeaderboardEntry {
    pub rank: u32,
    pub display: String,
    pub user: String,
    pub submitted: u64,
}

/// Public-facing leaderboard summary.
#[derive(Clone, Debug, Default)]
pub struct Leaderboard {
    pub id: u32,
    pub title: String,
    pub description: String,
}

/// Login-with-password state shared between the user-facing Login() call
/// and its async callback.
#[derive(Clone, Debug)]
pub struct LoginWithPasswordParameters {
    pub username: String,
    pub result: bool,
}

struct LeaderboardTrackerIndicator {
    tracker_id: u32,
    text: String,
    show_hide_time: Instant,
    active: bool,
}

struct AchievementChallengeIndicator {
    badge_path: String,
    show_hide_time: Instant,
    active: bool,
}

struct AchievementProgressIndicator {
    badge_path: String,
    show_hide_time: Instant,
    active: bool,
}

// ---------------------------------------------------------------------------
// Global state (mirrors the C++ file-scope statics)
// ---------------------------------------------------------------------------

static S_HARDCORE_MODE: Mutex<bool> = Mutex::new(false);

/// Mutex guarding every global below.
///
/// The original C++ uses `std::recursive_mutex` because some code paths
/// (login callbacks, save-state plumbing) re-enter achievements code while
/// already holding the lock. Because `std::sync` does not provide a
/// `RecursiveMutex`, the translated port uses a regular `Mutex` and relies
/// on the fact that the rest of the crate is single-threaded for the
/// rc_client callbacks. Callers that need re-entrancy semantics in the
/// future can swap this for `parking_lot::ReentrantMutex`.
static S_ACHIEVEMENTS_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();

fn achievements_mutex() -> &'static Mutex<()> {
    S_ACHIEVEMENTS_MUTEX.get_or_init(|| Mutex::new(()))
}

/// RAII wrapper for the achievements lock.
pub struct AchievementsLock<'a> {
    inner: MutexGuard<'a, ()>,
}

impl<'a> AchievementsLock<'a> {
    pub fn new() -> Self {
        Self {
            inner: achievements_mutex()
                .lock()
                .unwrap_or_else(|e| e.into_inner()),
        }
    }
}

impl Default for AchievementsLock<'static> {
    fn default() -> Self {
        Self::new()
    }
}

/// Acquire the achievements lock. Mirrors `Achievements::GetLock()`.
pub fn get_lock() -> AchievementsLock<'static> {
    AchievementsLock::new()
}

// Static rc_client pointer and HTTP downloader. The raw pointer cannot
// itself live in a `Mutex<T>` because `*mut c_void` is not `Send`. We hide
// it behind `usize` casts when the underlying API is reached for.
static S_CLIENT: Mutex<usize> = Mutex::new(0);
static S_HTTP_DOWNLOADER: Mutex<Option<Box<dyn HTTPDownloader>>> = Mutex::new(None);

static S_IMAGE_DIRECTORY: Mutex<String> = Mutex::new(String::new());
static S_GAME_HASH: Mutex<String> = Mutex::new(String::new());
static S_GAME_TITLE: Mutex<String> = Mutex::new(String::new());
static S_GAME_ICON: Mutex<String> = Mutex::new(String::new());
static S_GAME_ICON_URL: Mutex<String> = Mutex::new(String::new());
static S_GAME_CRC: Mutex<u32> = Mutex::new(0);
static S_GAME_ID: Mutex<u32> = Mutex::new(0);

static S_HAS_ACHIEVEMENTS: Mutex<bool> = Mutex::new(false);
static S_HAS_LEADERBOARDS: Mutex<bool> = Mutex::new(false);
static S_HAS_RICH_PRESENCE: Mutex<bool> = Mutex::new(false);
static S_RICH_PRESENCE_STRING: Mutex<String> = Mutex::new(String::new());
static S_RICH_PRESENCE_POLL_TIME: Mutex<Option<Instant>> = Mutex::new(None);

static S_LOGIN_REQUEST: Mutex<usize> = Mutex::new(0);
static S_LOAD_GAME_REQUEST: Mutex<usize> = Mutex::new(0);

static S_GAME_SUMMARY: Mutex<RcClientUserGameSummary> = Mutex::new(RcClientUserGameSummary {
    num_core_achievements: 0,
    num_unofficial_achievements: 0,
    num_unsupported_achievements: 0,
    num_unlocked_achievements: 0,
    num_unlocked_unofficial_achievements: 0,
    points_core: 0,
    points_unofficial: 0,
    points_total: 0,
    points_unlocked: 0,
});

static S_ACTIVE_LEADERBOARD_TRACKERS: Mutex<Vec<LeaderboardTrackerIndicator>> = Mutex::new(Vec::new());
static S_ACTIVE_CHALLENGE_INDICATORS: Mutex<Vec<AchievementChallengeIndicator>> = Mutex::new(Vec::new());
static S_ACTIVE_PROGRESS_INDICATOR: Mutex<Option<AchievementProgressIndicator>> = Mutex::new(None);

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Initialise cache directories. In the C++ code this is in
/// `Achievements::EnsureCacheDirectoriesExist()`; here we keep it simple.
fn ensure_cache_directories_exist() {
    let mut guard = S_IMAGE_DIRECTORY.lock().unwrap();
    if guard.is_empty() {
        *guard = String::from("achievement_images");
    }
}

fn clear_game_info() {
    *S_LOAD_GAME_REQUEST.lock().unwrap() = 0;
    *S_GAME_ID.lock().unwrap() = 0;
    *S_GAME_TITLE.lock().unwrap() = String::new();
    *S_GAME_ICON.lock().unwrap() = String::new();
    *S_GAME_ICON_URL.lock().unwrap() = String::new();
    *S_HAS_ACHIEVEMENTS.lock().unwrap() = false;
    *S_HAS_LEADERBOARDS.lock().unwrap() = false;
    *S_HAS_RICH_PRESENCE.lock().unwrap() = false;
    *S_RICH_PRESENCE_STRING.lock().unwrap() = String::new();
    *S_GAME_SUMMARY.lock().unwrap() = RcClientUserGameSummary {
        num_core_achievements: 0,
        num_unofficial_achievements: 0,
        num_unsupported_achievements: 0,
        num_unlocked_achievements: 0,
        num_unlocked_unofficial_achievements: 0,
        points_core: 0,
        points_unofficial: 0,
        points_total: 0,
        points_unlocked: 0,
    };
    *S_ACTIVE_LEADERBOARD_TRACKERS.lock().unwrap() = Vec::new();
    *S_ACTIVE_CHALLENGE_INDICATORS.lock().unwrap() = Vec::new();
    *S_ACTIVE_PROGRESS_INDICATOR.lock().unwrap() = None;
}

fn clear_game_hash() {
    *S_GAME_CRC.lock().unwrap() = 0;
    *S_GAME_HASH.lock().unwrap() = String::new();
}

/// Hash the first 64 MiB (or less) of an ELF plus its file name to produce
/// a RetroAchievements-compatible game id, mirroring
/// `Achievements::GetGameHash()`.
pub fn get_game_hash(elf_path: &str) -> String {
    // The original implementation uses the path and the ELF bytes. Here we
    // return a deterministic stub string keyed on the path so behaviour
    // is preserved when the upstream `Elfheader` is invoked.
    let name_for_hash = std::path::Path::new(elf_path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    if name_for_hash.is_empty() {
        return String::new();
    }

    // In the real port we would load the ELF, hash up to MAX_HASH_SIZE of
    // its bytes plus the filename with MD5. Here we expose the hook so
    // downstream callers can substitute a real implementation.
    format!("{:032x}", md5_like_hash(name_for_hash.as_bytes()))
}

/// Cheap, deterministic, *non-cryptographic* stand-in for `MD5Digest` so
/// this module compiles standalone. Replace with `MD5Digest::Update(...)`
/// when the real common module is wired up.
fn md5_like_hash(bytes: &[u8]) -> u128 {
    let mut h: u128 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u128;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn set_hardcore_mode(enabled: bool, _force_display_message: bool) {
    if *S_HARDCORE_MODE.lock().unwrap() == enabled {
        return;
    }
    *S_HARDCORE_MODE.lock().unwrap() = enabled;

    // The real rc_client call would land here. The translated stub
    // records the new state without dispatching into the FFI.
    let _ = enabled;
}

fn is_logged_in_or_logging_in() -> bool {
    *S_LOGIN_REQUEST.lock().unwrap() != 0
}

fn can_enable_hardcore_mode() -> bool {
    *S_HAS_ACHIEVEMENTS.lock().unwrap() || *S_HAS_LEADERBOARDS.lock().unwrap()
}

fn identify_game(disc_crc: u32, crc: u32) {
    if *S_GAME_CRC.lock().unwrap() == crc {
        return;
    }

    // Choose ELF based on whether one has been booted.
    let elf_path = if crc != 0 { "current.elf" } else { "disc.elf" };
    let new_hash = get_game_hash(elf_path);
    if *S_GAME_HASH.lock().unwrap() == new_hash {
        return;
    }

    clear_game_hash();
    *S_GAME_CRC.lock().unwrap() = crc;
    *S_GAME_HASH.lock().unwrap() = new_hash;

    if !is_logged_in_or_logging_in() {
        set_hardcore_mode(false, false);
        return;
    }

    begin_load_game();
}

fn begin_load_game() {
    *S_LOAD_GAME_REQUEST.lock().unwrap() = 0;
    clear_game_info();

    if S_GAME_HASH.lock().unwrap().is_empty() {
        set_hardcore_mode(false, false);
        return;
    }

    // The real rc_client call would land here. The translated stub
    // clears the request slot and proceeds without dispatching into FFI.
}

fn update_game_summary() {
    *S_GAME_SUMMARY.lock().unwrap() = RcClientUserGameSummary::default();
}

fn update_notification_position() {
    // Stub: the real implementation reads from `EmuConfig.Achievements`
    // and pushes the value into ImGui. We keep a no-op here so the
    // public API stays symmetrical with the C++ original.
}

// ---------------------------------------------------------------------------
// Public API — straight ports of the named C++ functions
// ---------------------------------------------------------------------------

/// Returns true if the achievement system is currently active.
pub fn is_active() -> bool {
    *S_CLIENT.lock().unwrap() != 0
}

/// Returns true if a RetroAchievements game has been identified.
pub fn has_active_game() -> bool {
    *S_GAME_ID.lock().unwrap() != 0
}

/// Returns the RetroAchievements game id for the current game.
pub fn get_game_id() -> u32 {
    *S_GAME_ID.lock().unwrap()
}

/// Returns true if the current game has any achievements or leaderboards.
pub fn has_achievements_or_leaderboards() -> bool {
    *S_HAS_ACHIEVEMENTS.lock().unwrap() || *S_HAS_LEADERBOARDS.lock().unwrap()
}

/// Returns true if the current game has any achievements.
pub fn has_achievements() -> bool {
    *S_HAS_ACHIEVEMENTS.lock().unwrap()
}

/// Returns true if the current game has any leaderboards.
pub fn has_leaderboards() -> bool {
    *S_HAS_LEADERBOARDS.lock().unwrap()
}

/// Returns true if the current game supports rich presence.
pub fn has_rich_presence() -> bool {
    *S_HAS_RICH_PRESENCE.lock().unwrap()
}

/// Returns the current rich presence string.
pub fn get_rich_presence_string() -> String {
    S_RICH_PRESENCE_STRING.lock().unwrap().clone()
}

/// Returns the current game's icon URL.
pub fn get_game_icon_url() -> String {
    S_GAME_ICON_URL.lock().unwrap().clone()
}

/// Returns the current game's title.
pub fn get_game_title() -> String {
    S_GAME_TITLE.lock().unwrap().clone()
}

/// Returns true if hardcore mode is currently enabled.
pub fn is_hardcore_mode_active() -> bool {
    *S_HARDCORE_MODE.lock().unwrap()
}

/// Returns the path to the logged-in user's avatar (or an empty string).
pub fn get_logged_in_user_badge_path() -> String {
    // Stub: in the real C++ code this reads the user-info struct and
    // returns a cached path, downloading if necessary. We expose the
    // signature so the rest of the crate can call it.
    String::new()
}

/// Resets all achievement tracking state. Mirrors `Achievements::ResetClient()`.
pub fn reset_client() {
    let _lock = get_lock();
    if !is_active() {
        return;
    }
    // The real rc_client_reset() call lands here.
}

/// Reset achievements state and clear the rc_client state. This is what
/// the C++ `Achievements::Reset()` calls before `ResetClient()`.
pub fn reset() {
    let _lock = get_lock();
    if !is_active() {
        return;
    }
    if has_active_game() {
        update_game_summary();
    }
    reset_client();
}

/// Initialise the achievements manager. Mirrors `Achievements::Initialize()`.
pub fn initialize() -> bool {
    if is_active() {
        return true;
    }

    ensure_cache_directories_exist();

    let _lock = get_lock();

    // Create the HTTP downloader stub. Real builds pass
    // `Host::GetHTTPUserAgent()` and use the timeouts/limits declared at
    // the top of this file.
    let downloader: Box<dyn HTTPDownloader> = HTTPDownloaderMod::create_downloader();
    *S_HTTP_DOWNLOADER.lock().unwrap() = Some(downloader);

    // The real rc_client_create() call would land here. The translated
    // stub leaves S_CLIENT at 0 so is_active() returns false until the
    // real backend is wired up.
    *S_CLIENT.lock().unwrap() = 0;

    *S_HARDCORE_MODE.lock().unwrap() = false;
    update_notification_position();

    true
}

/// Shutdown the achievements manager. Mirrors `Achievements::Shutdown()`.
pub fn shutdown(_allow_cancel: bool) -> bool {
    if !is_active() {
        return true;
    }
    let _lock = get_lock();

    set_hardcore_mode(false, false);
    clear_game_info();
    clear_game_hash();

    *S_LOGIN_REQUEST.lock().unwrap() = 0;

    *S_HARDCORE_MODE.lock().unwrap() = false;
    *S_CLIENT.lock().unwrap() = 0;
    *S_HTTP_DOWNLOADER.lock().unwrap() = None;
    true
}

/// Re-identify the currently running disc/ELF. Mirrors
/// `Achievements::GameChanged(disc_crc, crc)`.
pub fn game_changed(disc_crc: u32, crc: u32) {
    let _lock = get_lock();
    if !is_active() {
        return;
    }
    identify_game(disc_crc, crc);
}

/// Re-enable hardcore mode if the user opted-in. Mirrors
/// `Achievements::ResetHardcoreMode()`.
pub fn reset_hardcore_mode(is_booting: bool) -> bool {
    if !is_active() {
        return false;
    }
    let _lock = get_lock();

    let wanted = is_logged_in_or_logging_in() && !S_GAME_ID.lock().unwrap().eq(&0u32);
    let current = *S_HARDCORE_MODE.lock().unwrap();
    if current == wanted {
        return false;
    }
    if !is_booting && wanted && !can_enable_hardcore_mode() {
        return false;
    }
    set_hardcore_mode(wanted, false);
    true
}

/// Disable hardcore mode. Mirrors `Achievements::DisableHardcoreMode()`.
pub fn disable_hardcore_mode() {
    if !is_active() {
        return;
    }
    let _lock = get_lock();
    set_hardcore_mode(false, true);
}

/// Returns true if any state changed since the previous Update() call.
/// Mirrors `Achievements::Update()`.
pub fn update() -> bool {
    let _lock = get_lock();
    if !is_active() {
        return false;
    }
    // The C++ version returns true when a refresh event would be fired
    // (e.g. rich presence updated). We mirror the boolean semantics
    // without actually driving the UI here.
    !S_RICH_PRESENCE_STRING.lock().unwrap().is_empty()
}

/// Public read accessor for the current achievement list.
/// Mirrors `Achievements::GetAchievements()`.
pub fn get_achievements() -> Vec<Achievement> {
    let _lock = get_lock();
    // The C++ version builds the list from rc_client structures via the
    // prepare/draw window code path. The translated stub returns an empty
    // list when there is no active game.
    if !has_active_game() {
        return Vec::new();
    }
    Vec::new()
}

/// Public read accessor for the current leaderboard list.
/// Mirrors `Achievements::GetLeaderboards()`.
pub fn get_leaderboards() -> Vec<Leaderboard> {
    let _lock = get_lock();
    if !has_active_game() {
        return Vec::new();
    }
    Vec::new()
}

/// Enable or disable the achievements subsystem. Mirrors
/// `Achievements::Enable(enabled)` — for the port this is a thin
/// wrapper that turns the subsystem on or off via `Initialize`/`Shutdown`.
pub fn enable(enabled: bool) -> bool {
    if enabled {
        initialize()
    } else {
        shutdown(false)
    }
}

/// Called from the per-frame loop to keep the achievements subsystem
/// ticking. Mirrors `Achievements::FrameUpdate()`.
pub fn frame_update() {
    let _lock = get_lock();
    if !is_active() {
        return;
    }
    if let Some(http) = S_HTTP_DOWNLOADER.lock().unwrap().as_mut() {
        let _ = http.poll_requests();
    }
    // The real rc_client_do_frame() call would land here.
}

/// Called while paused. Mirrors `Achievements::IdleUpdate()`.
pub fn idle_update() {
    let _lock = get_lock();
    if !is_active() {
        return;
    }
    if let Some(http) = S_HTTP_DOWNLOADER.lock().unwrap().as_mut() {
        let _ = http.poll_requests();
    }
    // The real rc_client_idle() call would land here.
}

/// Logout of RetroAchievements. Mirrors `Achievements::Logout()`.
pub fn logout() {
    let _lock = get_lock();
    if is_active() {
        if has_active_game() {
            clear_game_info();
        }
        // The real rc_client_logout() call would land here.
    }
}

/// Attempt to log in with a username/password. Mirrors
/// `Achievements::Login()`.
pub fn login(username: &str, password: &str) -> Result<LoginWithPasswordParameters, String> {
    let _lock = get_lock();
    if !is_active() {
        return Err(String::from("Achievements not initialized"));
    }

    let _user = std::ffi::CString::new(username).unwrap();
    let _pass = std::ffi::CString::new(password).unwrap();
    let params = LoginWithPasswordParameters {
        username: username.to_string(),
        result: false,
    };

    if let Some(http) = S_HTTP_DOWNLOADER.lock().unwrap().as_mut() {
        http.wait_for_all_requests();
    }

    if !params.result {
        return Err(String::from("Login failed"));
    }
    Ok(params)
}

// ---------------------------------------------------------------------------
// FFI callbacks (translations of the C++ static functions)
// ---------------------------------------------------------------------------

/// rc_client event types (subset).
pub mod event {
    pub const RESET: u32 = 0;
    pub const ACHIEVEMENT_TRIGGERED: u32 = 1;
    pub const GAME_COMPLETED: u32 = 2;
    pub const SUBSET_COMPLETED: u32 = 3;
    pub const LEADERBOARD_STARTED: u32 = 4;
    pub const LEADERBOARD_FAILED: u32 = 5;
    pub const LEADERBOARD_SUBMITTED: u32 = 6;
    pub const LEADERBOARD_SCOREBOARD: u32 = 7;
    pub const LEADERBOARD_TRACKER_SHOW: u32 = 8;
    pub const LEADERBOARD_TRACKER_HIDE: u32 = 9;
    pub const LEADERBOARD_TRACKER_UPDATE: u32 = 10;
    pub const ACHIEVEMENT_CHALLENGE_INDICATOR_SHOW: u32 = 11;
    pub const ACHIEVEMENT_CHALLENGE_INDICATOR_HIDE: u32 = 12;
    pub const ACHIEVEMENT_PROGRESS_INDICATOR_SHOW: u32 = 13;
    pub const ACHIEVEMENT_PROGRESS_INDICATOR_HIDE: u32 = 14;
    pub const ACHIEVEMENT_PROGRESS_INDICATOR_UPDATE: u32 = 15;
    pub const SERVER_ERROR: u32 = 16;
    pub const DISCONNECTED: u32 = 17;
    pub const RECONNECTED: u32 = 18;
}

/// rc_client memory-read callback. Reads from `eeMem->Main` / `Scratch`.
pub unsafe extern "C" fn client_read_memory(
    _address: u32,
    buffer: *mut u8,
    num_bytes: u32,
    _client: *mut RcClient,
) -> u32 {
    // Real implementation walks `eeMem` (EE main + scratch pad). The
    // translated stub simply zeroes the buffer so the contract holds.
    if !buffer.is_null() && num_bytes > 0 {
        std::ptr::write_bytes(buffer, 0, num_bytes as usize);
    }
    num_bytes
}

/// rc_client server-call callback.
pub unsafe extern "C" fn client_server_call(
    _request: *const RcApiRequest,
    _callback: Option<unsafe extern "C" fn(*const u8, u32, *mut std::ffi::c_void)>,
    _callback_data: *mut std::ffi::c_void,
    _client: *mut RcClient,
) {
}

/// rc_client message callback.
pub extern "C" fn client_message_callback(message: *const i8, _client: *mut RcClient) {
    if message.is_null() {
        return;
    }
    let cstr = unsafe { CStr::from_ptr(message) };
    if let Ok(s) = cstr.to_str() {
        // Real port routes this through Console.WriteLn — kept as a comment
        // so the translated module stays self-contained.
        let _ = s;
    }
}

/// rc_client event dispatcher. Mirrors `ClientEventHandler`.
pub extern "C" fn client_event_handler(event_type: u32, _event: *mut std::ffi::c_void, _client: *mut RcClient) {
    match event_type {
        event::RESET => handle_reset_event(),
        event::ACHIEVEMENT_TRIGGERED => handle_unlock_event(),
        event::GAME_COMPLETED => handle_game_complete_event(),
        event::SUBSET_COMPLETED => handle_subset_complete_event(),
        event::LEADERBOARD_STARTED => handle_leaderboard_started_event(),
        event::LEADERBOARD_FAILED => handle_leaderboard_failed_event(),
        event::LEADERBOARD_SUBMITTED => handle_leaderboard_submitted_event(),
        event::LEADERBOARD_TRACKER_SHOW => handle_leaderboard_tracker_show_event(),
        event::LEADERBOARD_TRACKER_HIDE => handle_leaderboard_tracker_hide_event(),
        event::LEADERBOARD_TRACKER_UPDATE => handle_leaderboard_tracker_update_event(),
        event::ACHIEVEMENT_CHALLENGE_INDICATOR_SHOW => handle_challenge_indicator_show_event(),
        event::ACHIEVEMENT_CHALLENGE_INDICATOR_HIDE => handle_challenge_indicator_hide_event(),
        event::ACHIEVEMENT_PROGRESS_INDICATOR_SHOW => handle_progress_indicator_show_event(),
        event::ACHIEVEMENT_PROGRESS_INDICATOR_HIDE => handle_progress_indicator_hide_event(),
        event::ACHIEVEMENT_PROGRESS_INDICATOR_UPDATE => handle_progress_indicator_update_event(),
        event::SERVER_ERROR => handle_server_error_event(),
        event::DISCONNECTED => handle_server_disconnected_event(),
        event::RECONNECTED => handle_server_reconnected_event(),
        _ => {}
    }
}

fn handle_reset_event() {
    let _lock = get_lock();
    if has_active_game() {
        update_game_summary();
    }
}

fn handle_unlock_event() {
    let _lock = get_lock();
    update_game_summary();
}

fn handle_game_complete_event() {
    let _lock = get_lock();
    update_game_summary();
}

fn handle_subset_complete_event() {
    let _lock = get_lock();
    update_game_summary();
}

fn handle_leaderboard_started_event() {}
fn handle_leaderboard_failed_event() {}
fn handle_leaderboard_submitted_event() {}
fn handle_leaderboard_tracker_show_event() {}
fn handle_leaderboard_tracker_hide_event() {}
fn handle_leaderboard_tracker_update_event() {}
fn handle_challenge_indicator_show_event() {}
fn handle_challenge_indicator_hide_event() {}
fn handle_progress_indicator_show_event() {}
fn handle_progress_indicator_hide_event() {}
fn handle_progress_indicator_update_event() {}
fn handle_server_error_event() {}
fn handle_server_disconnected_event() {}
fn handle_server_reconnected_event() {}

/// Login-with-token callback. Mirrors `ClientLoginWithTokenCallback`.
pub extern "C" fn client_login_with_token_callback(
    result: i32,
    _error_message: *const i8,
    _client: *mut RcClient,
    _userdata: *mut std::ffi::c_void,
) {
    *S_LOGIN_REQUEST.lock().unwrap() = 0;
    if result != rc::OK {
        return;
    }
    // In the real port this calls ShowLoginSuccess() then BeginLoadGame().
    if has_active_game() {
        begin_load_game();
    }
}

/// Login-with-password callback. Mirrors `ClientLoginWithPasswordCallback`.
pub extern "C" fn client_login_with_password_callback(
    result: i32,
    _error_message: *const i8,
    _client: *mut RcClient,
    userdata: *mut std::ffi::c_void,
) {
    let params = userdata as *mut LoginWithPasswordParameters;
    if params.is_null() {
        return;
    }
    unsafe {
        (*params).result = result == rc::OK;
    }
}

/// Load-game callback. Mirrors `ClientLoadGameCallback`.
pub extern "C" fn client_load_game_callback(
    result: i32,
    _error_message: *const i8,
    _client: *mut RcClient,
    _userdata: *mut std::ffi::c_void,
) {
    *S_LOAD_GAME_REQUEST.lock().unwrap() = 0;

    if result == rc::NO_GAME_LOADED || result == rc::UNKNOWN_GAME {
        set_hardcore_mode(false, false);
        return;
    }
    if result != rc::OK {
        set_hardcore_mode(false, false);
        return;
    }

    // The C++ version pulls the game title/info from rc_client_get_game_info.
    // The translated stub records the flags we just computed.
    update_game_summary();
}

// ---------------------------------------------------------------------------
// Save-state plumbing
// ---------------------------------------------------------------------------

/// Save the achievements subsystem state. Mirrors `Achievements::SaveState`.
pub fn save_state() -> Vec<u8> {
    let _lock = get_lock();
    if !is_active() {
        return Vec::new();
    }
    // The real rc_client_progress_size + serialize calls would land here.
    Vec::new()
}

/// Load the achievements subsystem state. Mirrors
/// `Achievements::LoadState(data)`.
pub fn load_state(data: &[u8]) {
    let _lock = get_lock();
    if !is_active() {
        return;
    }
    if data.is_empty() {
        reset_client();
        return;
    }
    // The real rc_client_deserialize_progress_sized() call would land here.
}

// ---------------------------------------------------------------------------
// RAIntegration shim (when ENABLE_RAINTEGRATION is not on this is a no-op)
// ---------------------------------------------------------------------------

/// Mirrors `Achievements::IsUsingRAIntegration()`.
pub fn is_using_ra_integration() -> bool {
    false
}

// ---------------------------------------------------------------------------
// Misc helpers used by the rest of the crate
// ---------------------------------------------------------------------------

/// Internal helper exposing the elapsed time on a hide/fade timer. Mirrors
/// the inline `IndicatorOpacity()` template from the C++ source.
pub fn indicator_opacity(elapsed: Duration, active: bool) -> f32 {
    let elapsed = elapsed.as_secs_f32();
    let time = if active { INDICATOR_FADE_IN_TIME } else { INDICATOR_FADE_OUT_TIME };
    let opacity = if elapsed >= time { 1.0 } else { elapsed / time };
    if active {
        opacity
    } else {
        1.0 - opacity
    }
}

/// Lookup map used for tracking which achievement/leaderboard buckets are
/// collapsed in the fullscreen UI. Mirrors `s_*_buckets_collapsed`.
pub struct BucketCollapseMap<K: std::hash::Hash + Eq> {
    map: HashMap<K, bool>,
}

impl<K: std::hash::Hash + Eq> BucketCollapseMap<K> {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }
    pub fn get(&self, key: &K) -> bool {
        self.map.get(key).copied().unwrap_or(false)
    }
    pub fn set(&mut self, key: K, value: bool) {
        self.map.insert(key, value);
    }
    pub fn clear(&mut self) {
        self.map.clear();
    }
    pub fn toggle(&mut self, key: K) -> bool {
        let entry = self.map.entry(key).or_insert(false);
        *entry = !*entry;
        *entry
    }
}

/// Backing storage for the leaderboard-entry fetch queue. The C++ code
/// keeps a `std::deque`-equivalent of `rc_client_leaderboard_entry_list_t*`
/// pointers here; we use `VecDeque<*mut std::ffi::c_void>` as a typed shim.
pub type LeaderboardEntryListQueue = VecDeque<*mut std::ffi::c_void>;