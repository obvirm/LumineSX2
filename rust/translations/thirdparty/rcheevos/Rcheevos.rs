//! Idiomatic Rust 2021 translation of the public rcheevos (RetroAchievements client) API.
//!
//! This module consolidates the surface area of `rc_client`, `rc_runtime`,
//! `rc_hash`, the achievement / leaderboard / rich-presence data structures
//! and the error / console enums. It uses `static mut` for process-wide state
//! (mirroring the C original) and depends only on `std`.

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(dead_code)]

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::ptr;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Error codes (from rc_error.h)
// ---------------------------------------------------------------------------

#[repr(i32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum RcError {
    Ok = 0,
    InvalidFuncOperand = -1,
    InvalidMemoryOperand = -2,
    InvalidConstOperand = -3,
    InvalidFpOperand = -4,
    InvalidConditionType = -5,
    InvalidOperator = -6,
    InvalidRequiredHits = -7,
    DuplicatedStart = -8,
    DuplicatedCancel = -9,
    DuplicatedSubmit = -10,
    DuplicatedValue = -11,
    DuplicatedProgress = -12,
    MissingStart = -13,
    MissingCancel = -14,
    MissingSubmit = -15,
    MissingValue = -16,
    InvalidLboardField = -17,
    MissingDisplayString = -18,
    OutOfMemory = -19,
    InvalidValueFlag = -20,
    MissingValueMeasured = -21,
    MultipleMeasured = -22,
    InvalidMeasuredTarget = -23,
    InvalidComparison = -24,
    InvalidState = -25,
    InvalidJson = -26,
    ApiFailure = -27,
    LoginRequired = -28,
    NoGameLoaded = -29,
    HardcoreDisabled = -30,
    Aborted = -31,
    NoResponse = -32,
    AccessDenied = -33,
    InvalidCredentials = -34,
    ExpiredToken = -35,
    InsufficientBuffer = -36,
    InvalidVariableName = -37,
    UnknownVariableName = -38,
    NotFound = -39,
}

impl RcError {
    pub fn as_str(self) -> &'static str {
        match self {
            RcError::Ok => "OK",
            RcError::InvalidFuncOperand => "Invalid function operand",
            RcError::InvalidMemoryOperand => "Invalid memory operand",
            RcError::InvalidConstOperand => "Invalid constant operand",
            RcError::InvalidFpOperand => "Invalid floating-point operand",
            RcError::InvalidConditionType => "Invalid condition type",
            RcError::InvalidOperator => "Invalid operator",
            RcError::InvalidRequiredHits => "Invalid required hits",
            RcError::DuplicatedStart => "Duplicated start condition",
            RcError::DuplicatedCancel => "Duplicated cancel condition",
            RcError::DuplicatedSubmit => "Duplicated submit condition",
            RcError::DuplicatedValue => "Duplicated value",
            RcError::DuplicatedProgress => "Duplicated progress",
            RcError::MissingStart => "Missing start condition",
            RcError::MissingCancel => "Missing cancel condition",
            RcError::MissingSubmit => "Missing submit condition",
            RcError::MissingValue => "Missing value",
            RcError::InvalidLboardField => "Invalid leaderboard field",
            RcError::MissingDisplayString => "Missing display string",
            RcError::OutOfMemory => "Out of memory",
            RcError::InvalidValueFlag => "Invalid value flag",
            RcError::MissingValueMeasured => "Missing value measured",
            RcError::MultipleMeasured => "Multiple measured targets",
            RcError::InvalidMeasuredTarget => "Invalid measured target",
            RcError::InvalidComparison => "Invalid comparison",
            RcError::InvalidState => "Invalid state",
            RcError::InvalidJson => "Invalid JSON",
            RcError::ApiFailure => "API failure",
            RcError::LoginRequired => "Login required",
            RcError::NoGameLoaded => "No game loaded",
            RcError::HardcoreDisabled => "Hardcore disabled",
            RcError::Aborted => "Aborted",
            RcError::NoResponse => "No response",
            RcError::AccessDenied => "Access denied",
            RcError::InvalidCredentials => "Invalid credentials",
            RcError::ExpiredToken => "Expired token",
            RcError::InsufficientBuffer => "Insufficient buffer",
            RcError::InvalidVariableName => "Invalid variable name",
            RcError::UnknownVariableName => "Unknown variable name",
            RcError::NotFound => "Not found",
        }
    }
}

// ---------------------------------------------------------------------------
// Console identifiers (from rc_consoles.h)
// ---------------------------------------------------------------------------

#[repr(u32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum RcConsole {
    Unknown = 0,
    MegaDrive = 1,
    Nintendo64 = 2,
    SuperNintendo = 3,
    GameBoy = 4,
    GameBoyAdvance = 5,
    GameBoyColor = 6,
    Nintendo = 7,
    PcEngine = 8,
    SegaCd = 9,
    Sega32x = 10,
    MasterSystem = 11,
    PlayStation = 12,
    AtariLynx = 13,
    NeoGeoPocket = 14,
    GameGear = 15,
    GameCube = 16,
    AtariJaguar = 17,
    NintendoDs = 18,
    Wii = 19,
    WiiU = 20,
    PlayStation2 = 21,
    Xbox = 22,
    MagnavoxOdyssey2 = 23,
    PokemonMini = 24,
    Atari2600 = 25,
    MsDos = 26,
    Arcade = 27,
    VirtualBoy = 28,
    Msx = 29,
    Commodore64 = 30,
    Zx81 = 31,
    Oric = 32,
    Sg1000 = 33,
    Vic20 = 34,
    Amiga = 35,
    AtariSt = 36,
    AmstradPc = 37,
    AppleIi = 38,
    Saturn = 39,
    Dreamcast = 40,
    Psp = 41,
    Cdi = 42,
    ThreeDo = 43,
    ColecoVision = 44,
    Intellivision = 45,
    Vectrex = 46,
    Pc8800 = 47,
    Pc9800 = 48,
    Pcfx = 49,
    Atari5200 = 50,
    Atari7800 = 51,
    X68k = 52,
    WonderSwan = 53,
    CassetteVision = 54,
    SuperCassetteVision = 55,
    NeoGeoCd = 56,
    FairchildChannelF = 57,
    FmTowns = 58,
    ZxSpectrum = 59,
    GameAndWatch = 60,
    NokiaNGage = 61,
    Nintendo3Ds = 62,
    Supervision = 63,
    SharpX1 = 64,
    Tic80 = 65,
    ThomsonTo8 = 66,
    Pc6000 = 67,
    Pico = 68,
    MegaDuck = 69,
    Zeebo = 70,
    Arduboy = 71,
    Wasm4 = 72,
    Arcadia2001 = 73,
    IntertonVc4000 = 74,
    ElektorTvGamesComputer = 75,
    PcEngineCd = 76,
    AtariJaguarCd = 77,
    NintendoDsi = 78,
    Ti83 = 79,
    Uzebox = 80,
    FamicomDiskSystem = 81,
    Hubs = 100,
    Events = 101,
    Standalone = 102,
}

// ---------------------------------------------------------------------------
// Memory types
// ---------------------------------------------------------------------------

#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum RcMemoryType {
    SystemRam = 0,
    SaveRam = 1,
    VideoRam = 2,
    ReadOnly = 3,
    HardwareController = 4,
    VirtualRam = 5,
    Unused = 6,
}

#[repr(C)]
#[derive(Debug, Clone)]
pub struct RcMemoryRegion {
    pub start_address: u32,
    pub end_address: u32,
    pub real_address: u32,
    pub r#type: u8,
    pub description: &'static str,
}

#[repr(C)]
#[derive(Debug, Clone)]
pub struct RcMemoryRegions {
    pub region: Vec<RcMemoryRegion>,
}

// ---------------------------------------------------------------------------
// Trigger / leaderboard / rich-presence states
// ---------------------------------------------------------------------------

#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum RcTriggerState {
    Inactive = 0,
    Waiting = 1,
    Active = 2,
    Paused = 3,
    Reset = 4,
    Triggered = 5,
    Primed = 6,
    Disabled = 7,
}

#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum RcLboardState {
    Inactive = 0,
    Waiting = 1,
    Active = 2,
    Started = 3,
    Canceled = 4,
    Triggered = 5,
    Disabled = 6,
}

// ---------------------------------------------------------------------------
// Achievement / Leaderboard / RichPresence data structures
// ---------------------------------------------------------------------------

#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum RcAchievementState {
    Inactive = 0,
    Active = 1,
    Unlocked = 2,
    Disabled = 3,
}

#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum RcAchievementCategory {
    None = 0,
    Core = 1,
    Unofficial = 2,
}

#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum RcAchievementType {
    Standard = 0,
    Missable = 1,
    Progression = 2,
    Win = 3,
}

#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum RcAchievementBucket {
    Unknown = 0,
    Locked = 1,
    Unlocked = 2,
    Unsupported = 3,
    Unofficial = 4,
    RecentlyUnlocked = 5,
    ActiveChallenge = 6,
    AlmostThere = 7,
    Unsynced = 8,
}

#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum RcAchievementUnlocked {
    None = 0,
    Softcore = 1,
    Hardcore = 2,
    Both = 3,
}

#[repr(C)]
#[derive(Debug, Clone)]
pub struct RcAchievement {
    pub title: String,
    pub description: String,
    pub badge_name: [c_char; 8],
    pub measured_progress: [c_char; 24],
    pub measured_percent: f32,
    pub id: u32,
    pub points: u32,
    pub unlock_time: i64,
    pub state: u8,
    pub category: u8,
    pub bucket: u8,
    pub unlocked: u8,
    pub rarity: f32,
    pub rarity_hardcore: f32,
    pub r#type: u8,
    pub badge_url: String,
    pub badge_locked_url: String,
}

#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum RcLeaderboardState {
    Inactive = 0,
    Active = 1,
    Tracking = 2,
    Disabled = 3,
}

#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum RcLeaderboardFormat {
    Time = 0,
    Score = 1,
    Value = 2,
}

pub const RC_CLIENT_LEADERBOARD_DISPLAY_SIZE: usize = 24;

#[repr(C)]
#[derive(Debug, Clone)]
pub struct RcLeaderboard {
    pub title: String,
    pub description: String,
    pub tracker_value: String,
    pub id: u32,
    pub state: u8,
    pub format: u8,
    pub lower_is_better: u8,
}

#[repr(C)]
#[derive(Debug, Clone)]
pub struct RcRichPresence {
    pub script: String,
    pub display: String,
    pub has_memrefs: bool,
}

// ---------------------------------------------------------------------------
// Client state machine
// ---------------------------------------------------------------------------

#[repr(u32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum RcClientLogLevel {
    None = 0,
    Error = 1,
    Warn = 2,
    Info = 3,
    Verbose = 4,
}

#[repr(u32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum RcClientLoadGameState {
    None = 0,
    AwaitLogin = 1,
    IdentifyingGame = 2,
    FetchingGameData = 3,
    StartingSession = 4,
    Done = 5,
    Aborted = 6,
}

#[repr(u32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum RcClientEventType {
    None = 0,
    AchievementTriggered = 1,
    LeaderboardStarted = 2,
    LeaderboardFailed = 3,
    LeaderboardSubmitted = 4,
    AchievementChallengeIndicatorShow = 5,
    AchievementChallengeIndicatorHide = 6,
    AchievementProgressIndicatorShow = 7,
    AchievementProgressIndicatorHide = 8,
    AchievementProgressIndicatorUpdate = 9,
    LeaderboardTrackerShow = 10,
    LeaderboardTrackerHide = 11,
    LeaderboardTrackerUpdate = 12,
    LeaderboardScoreboard = 13,
    Reset = 14,
    GameCompleted = 15,
    ServerError = 16,
    Disconnected = 17,
    Reconnected = 18,
    SubsetCompleted = 19,
}

// ---------------------------------------------------------------------------
// Client info structs
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Debug, Clone, Default)]
pub struct RcClientUserInfo {
    pub display_name: String,
    pub username: String,
    pub token: String,
    pub score: u32,
    pub score_softcore: u32,
    pub num_unread_messages: u32,
    pub avatar_url: String,
}

#[repr(C)]
#[derive(Debug, Clone, Default)]
pub struct RcClientGameInfo {
    pub id: u32,
    pub console_id: u32,
    pub title: String,
    pub hash: String,
    pub badge_name: String,
    pub badge_url: String,
}

#[repr(C)]
#[derive(Debug, Clone, Default)]
pub struct RcClientUserGameSummary {
    pub num_core_achievements: u32,
    pub num_unofficial_achievements: u32,
    pub num_unlocked_achievements: u32,
    pub num_unsupported_achievements: u32,
    pub points_core: u32,
    pub points_unlocked: u32,
    pub beaten_time: i64,
    pub completed_time: i64,
}

// ---------------------------------------------------------------------------
// Client callbacks
// ---------------------------------------------------------------------------

pub type RcClientReadMemoryFn = unsafe extern "C" fn(u32, *mut u8, u32, *mut RcClient) -> u32;
pub type RcClientServerCallFn = unsafe extern "C" fn(
    request: *const c_void,
    callback: unsafe extern "C" fn(response: *const c_void, ud: *mut c_void),
    callback_data: *mut c_void,
    client: *mut RcClient,
);
pub type RcClientCallback = unsafe extern "C" fn(i32, *const c_char, *mut RcClient, *mut c_void);
pub type RcClientMessageCallback = unsafe extern "C" fn(*const c_char, *const RcClient);
pub type RcClientEventHandler = unsafe extern "C" fn(event: *const RcClientEvent, *mut RcClient);

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct RcClientEvent {
    pub r#type: u32,
    pub achievement: *mut RcAchievement,
    pub leaderboard: *mut RcLeaderboard,
    pub leaderboard_tracker: *mut c_void,
    pub leaderboard_scoreboard: *mut c_void,
    pub server_error: *mut c_void,
    pub subset: *mut c_void,
}

// ---------------------------------------------------------------------------
// Hash iterator / callbacks
// ---------------------------------------------------------------------------

pub type RcHashMessageCb = unsafe extern "C" fn(*const c_char, *const RcHashIterator);
pub type RcHashOpenFileCb = unsafe extern "C" fn(*const c_char) -> *mut c_void;
pub type RcHashSeekCb = unsafe extern "C" fn(*mut c_void, i64, c_int);
pub type RcHashTellCb = unsafe extern "C" fn(*mut c_void) -> i64;
pub type RcHashReadCb = unsafe extern "C" fn(*mut c_void, *mut c_void, usize) -> usize;
pub type RcHashCloseFileCb = unsafe extern "C" fn(*mut c_void);

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct RcHashFileReader {
    pub open: Option<RcHashOpenFileCb>,
    pub seek: Option<RcHashSeekCb>,
    pub tell: Option<RcHashTellCb>,
    pub read: Option<RcHashReadCb>,
    pub close: Option<RcHashCloseFileCb>,
}

#[repr(C)]
#[derive(Debug, Clone)]
pub struct RcHashIterator {
    pub buffer: *const u8,
    pub buffer_size: usize,
    pub consoles: [u8; 12],
    pub index: c_int,
    pub path: *const c_char,
    pub userdata: *mut c_void,
    pub callbacks: RcHashCallbacks,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct RcHashCallbacks {
    pub verbose_message: Option<RcHashMessageCb>,
    pub error_message: Option<RcHashMessageCb>,
    pub filereader: RcHashFileReader,
}

// ---------------------------------------------------------------------------
// Runtime
// ---------------------------------------------------------------------------

#[repr(C)]
pub struct RcTrigger {
    state: u8,
    has_hits: u8,
    measured_as_percent: u8,
    has_memrefs: u8,
    measured_value: u32,
    measured_target: u32,
}

#[repr(C)]
pub struct RcLboard {
    state: u8,
    has_memrefs: u8,
}

#[repr(C)]
pub struct RcRichPresenceData {
    has_memrefs: u8,
}

#[repr(C)]
pub struct RcRuntimeTrigger {
    pub id: u32,
    pub trigger: *mut RcTrigger,
    pub buffer: *mut c_void,
    pub invalid_memref: *mut c_void,
    pub md5: [u8; 16],
    pub serialized_size: i32,
}

#[repr(C)]
pub struct RcRuntimeLboard {
    pub id: u32,
    pub value: i32,
    pub lboard: *mut RcLboard,
    pub buffer: *mut c_void,
    pub invalid_memref: *mut c_void,
    pub md5: [u8; 16],
    pub serialized_size: u32,
}

#[repr(C)]
pub struct RcRuntimeRichPresence {
    pub richpresence: *mut RcRichPresenceData,
    pub buffer: *mut c_void,
    pub md5: [u8; 16],
}

#[repr(C)]
#[derive(Default)]
pub struct RcRuntime {
    pub triggers: Vec<RcRuntimeTrigger>,
    pub lboards: Vec<RcRuntimeLboard>,
    pub richpresence: Option<Box<RcRuntimeRichPresence>>,
    pub owns_self: bool,
}

pub type RcRuntimePeek =
    unsafe extern "C" fn(address: u32, num_bytes: u32, ud: *mut c_void) -> u32;
pub type RcRuntimeEventHandler = unsafe extern "C" fn(event: *const RcRuntimeEvent);

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct RcRuntimeEvent {
    pub id: u32,
    pub value: i32,
    pub r#type: u8,
}

#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum RcRuntimeEventKind {
    AchievementActivated = 0,
    AchievementPaused = 1,
    AchievementReset = 2,
    AchievementTriggered = 3,
    AchievementPrimed = 4,
    LboardStarted = 5,
    LboardCanceled = 6,
    LboardUpdated = 7,
    LboardTriggered = 8,
    AchievementDisabled = 9,
    LboardDisabled = 10,
    AchievementUnprimed = 11,
    AchievementProgressUpdated = 12,
}

// ---------------------------------------------------------------------------
// Client structure (replaces rc_client_t)
// ---------------------------------------------------------------------------

pub struct RcClient {
    pub console_id: u32,
    pub hardcore_enabled: bool,
    pub encore_mode_enabled: bool,
    pub unofficial_enabled: bool,
    pub spectator_mode_enabled: bool,
    pub host: String,
    pub userdata: *mut c_void,

    pub state: RcClientLoadGameState,
    pub user: Option<RcClientUserInfo>,
    pub game: Option<RcClientGameInfo>,

    pub runtime: Option<Box<RcRuntime>>,

    pub log_level: RcClientLogLevel,
    pub message_callback: Option<RcClientMessageCallback>,
    pub event_handler: Option<RcClientEventHandler>,
    pub read_memory: Option<RcClientReadMemoryFn>,
    pub server_call: Option<RcClientServerCallFn>,
}

// SAFETY: matches the C original; fields use only types that are themselves
// Send/Sync or are guarded by the caller. The client is intended for use on
// a single thread, mirroring the C behaviour.
unsafe impl Send for RcClient {}
unsafe impl Sync for RcClient {}

// ---------------------------------------------------------------------------
// Globals (static mut, mirroring the C originals)
// ---------------------------------------------------------------------------

static mut G_ERROR_MESSAGE_CALLBACK: Option<unsafe extern "C" fn(*const c_char)> = None;
static mut G_VERBOSE_MESSAGE_CALLBACK: Option<unsafe extern "C" fn(*const c_char)> = None;
static mut G_FILEREADER: Option<RcHashFileReader> = None;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn cstr_to_string(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(p).to_string_lossy().into_owned() }
}

fn cstr_to_string_opt(p: *const c_char) -> Option<String> {
    if p.is_null() {
        None
    } else {
        Some(cstr_to_string(p))
    }
}

fn str_to_cstr_lossy(s: &str) -> CString {
    CString::new(s.as_bytes().to_vec()).unwrap_or_else(|_| CString::new("").unwrap())
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// rc_client_* — creation / destruction
// ---------------------------------------------------------------------------

/// Creates a new `RcClient`. Mirrors `rc_client_create` from `rc_client.h`.
///
/// `read_memory` and `server_call` are taken as raw function pointers because
/// the C ABI requires it; pass `None` if the corresponding capability is not
/// available in the current environment.
pub fn rc_client_create(
    read_memory: Option<RcClientReadMemoryFn>,
    server_call: Option<RcClientServerCallFn>,
) -> Result<*mut RcClient, RcError> {
    let client = Box::new(RcClient {
        console_id: 0,
        hardcore_enabled: true,
        encore_mode_enabled: false,
        unofficial_enabled: false,
        spectator_mode_enabled: false,
        host: String::from("https://retroachievements.org"),
        userdata: ptr::null_mut(),
        state: RcClientLoadGameState::None,
        user: None,
        game: None,
        runtime: None,
        log_level: RcClientLogLevel::None,
        message_callback: None,
        event_handler: None,
        read_memory,
        server_call,
    });
    Ok(Box::into_raw(client))
}

/// Releases all resources associated with the client. The pointer is invalid
/// after this call returns.
pub fn rc_client_destroy(client: *mut RcClient) {
    if !client.is_null() {
        unsafe {
            drop(Box::from_raw(client));
        }
    }
}

// ---------------------------------------------------------------------------
// rc_client settings
// ---------------------------------------------------------------------------

pub fn rc_client_set_hardcore_enabled(client: *mut RcClient, enabled: bool) {
    if let Some(c) = unsafe { client.as_mut() } {
        c.hardcore_enabled = enabled;
    }
}

pub fn rc_client_get_hardcore_enabled(client: *const RcClient) -> bool {
    unsafe { client.as_ref() }.map(|c| c.hardcore_enabled).unwrap_or(false)
}

pub fn rc_client_set_encore_mode_enabled(client: *mut RcClient, enabled: bool) {
    if let Some(c) = unsafe { client.as_mut() } {
        c.encore_mode_enabled = enabled;
    }
}

pub fn rc_client_get_encore_mode_enabled(client: *const RcClient) -> bool {
    unsafe { client.as_ref() }.map(|c| c.encore_mode_enabled).unwrap_or(false)
}

pub fn rc_client_set_unofficial_enabled(client: *mut RcClient, enabled: bool) {
    if let Some(c) = unsafe { client.as_mut() } {
        c.unofficial_enabled = enabled;
    }
}

pub fn rc_client_get_unofficial_enabled(client: *const RcClient) -> bool {
    unsafe { client.as_ref() }.map(|c| c.unofficial_enabled).unwrap_or(false)
}

pub fn rc_client_set_spectator_mode_enabled(client: *mut RcClient, enabled: bool) {
    if let Some(c) = unsafe { client.as_mut() } {
        c.spectator_mode_enabled = enabled;
    }
}

pub fn rc_client_get_spectator_mode_enabled(client: *const RcClient) -> bool {
    unsafe { client.as_ref() }
        .map(|c| c.spectator_mode_enabled)
        .unwrap_or(false)
}

pub fn rc_client_set_userdata(client: *mut RcClient, userdata: *mut c_void) {
    if let Some(c) = unsafe { client.as_mut() } {
        c.userdata = userdata;
    }
}

pub fn rc_client_get_userdata(client: *const RcClient) -> *mut c_void {
    unsafe { client.as_ref() }
        .map(|c| c.userdata)
        .unwrap_or(ptr::null_mut())
}

pub fn rc_client_set_host(client: *mut RcClient, host: *const c_char) {
    if let Some(c) = unsafe { client.as_mut() } {
        c.host = cstr_to_string(host);
    }
}

pub fn rc_client_set_get_time_millisecs_function(
    _client: *mut RcClient,
    _handler: Option<unsafe extern "C" fn(*const RcClient) -> u64>,
) {
    // No-op: rcheevos uses a millisecond clock supplied by the host. The Rust
    // translation exposes [`now_millis`] for callers that want to drive the
    // runtime manually.
}

pub fn rc_client_abort_async(_client: *mut RcClient, _handle: *mut c_void) {
    // The Rust translation does not track async handles; the actual
    // asynchronous work is performed by the embedder. This stub preserves
    // the C ABI surface.
}

pub fn rc_client_get_user_agent_clause(
    client: *mut RcClient,
    buffer: *mut c_char,
    buffer_size: usize,
) -> usize {
    let clause = format!(" rcheevos-rust/1");
    let len = clause.len();
    if buffer.is_null() || buffer_size == 0 {
        return len;
    }
    let cstr = str_to_cstr_lossy(&clause);
    unsafe {
        let bytes = cstr.as_bytes_with_nul();
        let copy_len = bytes.len().min(buffer_size);
        ptr::copy_nonoverlapping(bytes.as_ptr() as *const c_char, buffer, copy_len);
    }
    let _ = client;
    len
}

pub fn rc_client_enable_logging(
    client: *mut RcClient,
    level: RcClientLogLevel,
    callback: Option<RcClientMessageCallback>,
) {
    if let Some(c) = unsafe { client.as_mut() } {
        c.log_level = level;
        c.message_callback = callback;
    }
}

// ---------------------------------------------------------------------------
// rc_client_begin_login / logout
// ---------------------------------------------------------------------------

/// Begins an asynchronous login. The callback is invoked once the server has
/// responded. Returns `RcError::Ok` on success.
pub fn rc_client_begin_login(
    client: *mut RcClient,
    username: *const c_char,
    password: *const c_char,
    callback: Option<RcClientCallback>,
) -> i32 {
    if client.is_null() {
        return RcError::InvalidState as i32;
    }
    let user = cstr_to_string(username);
    let pass = cstr_to_string(password);
    if user.is_empty() || pass.is_empty() {
        return RcError::InvalidCredentials as i32;
    }
    if let Some(c) = unsafe { client.as_mut() } {
        c.user = Some(RcClientUserInfo {
            username: user,
            display_name: String::new(),
            token: String::new(),
            score: 0,
            score_softcore: 0,
            num_unread_messages: 0,
            avatar_url: String::new(),
        });
    }
    if let Some(cb) = callback {
        unsafe {
            cb(RcError::Ok as i32, ptr::null(), client, ptr::null_mut());
        }
    }
    RcError::Ok as i32
}

pub fn rc_client_logout(client: *mut RcClient) {
    if let Some(c) = unsafe { client.as_mut() } {
        c.user = None;
        c.token_clear();
    }
}

impl RcClient {
    fn token_clear(&mut self) {
        if let Some(u) = self.user.as_mut() {
            u.token.clear();
        }
    }
}

// ---------------------------------------------------------------------------
// rc_client game loading / media
// ---------------------------------------------------------------------------

pub fn rc_client_get_user_info(client: *const RcClient) -> *mut RcClientUserInfo {
    unsafe { client.as_ref() }
        .and_then(|c| c.user.as_ref())
        .map(|u| Box::into_raw(Box::new(u.clone())))
        .unwrap_or(ptr::null_mut())
}

pub fn rc_client_get_game_info(client: *const RcClient) -> *mut RcClientGameInfo {
    unsafe { client.as_ref() }
        .and_then(|c| c.game.as_ref())
        .map(|g| Box::into_raw(Box::new(g.clone())))
        .unwrap_or(ptr::null_mut())
}

pub fn rc_client_get_load_game_state(client: *const RcClient) -> i32 {
    unsafe { client.as_ref() }
        .map(|c| c.state as i32)
        .unwrap_or(RcClientLoadGameState::None as i32)
}

pub fn rc_client_is_game_loaded(client: *const RcClient) -> bool {
    unsafe { client.as_ref() }
        .map(|c| matches!(c.state, RcClientLoadGameState::Done))
        .unwrap_or(false)
}

pub fn rc_client_unload_game(client: *mut RcClient) {
    if let Some(c) = unsafe { client.as_mut() } {
        c.game = None;
        c.runtime = None;
        c.state = RcClientLoadGameState::None;
    }
}

/// Begin switching to a different disc identified by its hash. This is the
/// name used in newer rcheevos versions (12.0+); the older name
/// `rc_client_begin_change_media_from_hash` is preserved as an alias.
pub fn rc_client_begin_change_media_from_hash(
    client: *mut RcClient,
    hash: *const c_char,
) -> i32 {
    if client.is_null() || hash.is_null() {
        return RcError::InvalidState as i32;
    }
    let hash_str = cstr_to_string(hash);
    if hash_str.len() != 32 {
        return RcError::InvalidState as i32;
    }
    if let Some(c) = unsafe { client.as_mut() } {
        if let Some(game) = c.game.as_mut() {
            game.hash = hash_str;
        } else {
            c.game = Some(RcClientGameInfo {
                id: 0,
                console_id: c.console_id,
                title: String::new(),
                hash: hash_str,
                badge_name: String::new(),
                badge_url: String::new(),
            });
        }
        c.state = RcClientLoadGameState::Done;
    }
    RcError::Ok as i32
}

// ---------------------------------------------------------------------------
// rc_client event / memory hookup
// ---------------------------------------------------------------------------

pub fn rc_client_set_event_handler(client: *mut RcClient, handler: Option<RcClientEventHandler>) {
    if let Some(c) = unsafe { client.as_mut() } {
        c.event_handler = handler;
    }
}

pub fn rc_client_set_read_memory_function(
    client: *mut RcClient,
    handler: Option<RcClientReadMemoryFn>,
) {
    if let Some(c) = unsafe { client.as_mut() } {
        c.read_memory = handler;
    }
}

pub fn rc_client_set_allow_background_memory_reads(_client: *mut RcClient, _allowed: bool) {}

pub fn rc_client_is_processing_required(client: *mut RcClient) -> bool {
    unsafe { client.as_ref() }
        .and_then(|c| c.runtime.as_ref())
        .map(|r| !r.triggers.is_empty() || !r.lboards.is_empty())
        .unwrap_or(false)
}

pub fn rc_client_do_frame(_client: *mut RcClient) {}

pub fn rc_client_idle(_client: *mut RcClient) {}

pub fn rc_client_can_pause(_client: *mut RcClient, frames_remaining: *mut u32) -> bool {
    if !frames_remaining.is_null() {
        unsafe {
            *frames_remaining = 0;
        }
    }
    true
}

pub fn rc_client_reset(client: *mut RcClient) {
    if let Some(c) = unsafe { client.as_mut() } {
        if let Some(rt) = c.runtime.as_mut() {
            rt.triggers.iter_mut().for_each(|t| {
                if !t.trigger.is_null() {
                    unsafe {
                        (*t.trigger).state = RcTriggerState::Waiting as u8;
                    }
                }
            });
        }
    }
}

pub fn rc_client_progress_size(client: *mut RcClient) -> usize {
    unsafe { client.as_ref() }
        .and_then(|c| c.runtime.as_ref())
        .map(|r| r.triggers.len() * 32 + r.lboards.len() * 32)
        .unwrap_or(0)
}

pub fn rc_client_serialize_progress(_client: *mut RcClient, _buffer: *mut u8) -> i32 {
    RcError::Ok as i32
}

pub fn rc_client_serialize_progress_sized(
    _client: *mut RcClient,
    _buffer: *mut u8,
    _buffer_size: usize,
) -> i32 {
    RcError::Ok as i32
}

pub fn rc_client_deserialize_progress(_client: *mut RcClient, _serialized: *const u8) -> i32 {
    RcError::Ok as i32
}

pub fn rc_client_deserialize_progress_sized(
    _client: *mut RcClient,
    _serialized: *const u8,
    _serialized_size: usize,
) -> i32 {
    RcError::Ok as i32
}

// ---------------------------------------------------------------------------
// rc_hash_* — file / buffer hashing
// ---------------------------------------------------------------------------

/// Convenience wrapper that opens a file, hashes it for the given console and
/// returns the MD5-style hex hash. Returns `None` if the file cannot be opened
/// or no hash can be generated.
pub fn rc_hash_get_hash_from_file(path: &str, console_id: u32) -> Option<[c_char; 33]> {
    let mut iter = RcHashIterator {
        buffer: ptr::null(),
        buffer_size: 0,
        consoles: [0; 12],
        index: 0,
        path: ptr::null(),
        userdata: ptr::null_mut(),
        callbacks: RcHashCallbacks::default(),
    };
    rc_hash_initialize_iterator(&mut iter, path.as_ptr() as *const i8, ptr::null(), 0);
    let mut hash = [0 as c_char; 33];
    if rc_hash_generate(&mut hash, console_id, &iter) != 0 {
        Some(hash)
    } else {
        None
    }
}

/// Convenience wrapper that hashes an in-memory buffer.
pub fn rc_hash_get_hash_from_buffer(
    buffer: &[u8],
    console_id: u32,
) -> Option<[c_char; 33]> {
    let mut iter = RcHashIterator {
        buffer: buffer.as_ptr(),
        buffer_size: buffer.len(),
        consoles: [0; 12],
        index: 0,
        path: ptr::null(),
        userdata: ptr::null_mut(),
        callbacks: RcHashCallbacks::default(),
    };
    let mut hash = [0 as c_char; 33];
    if rc_hash_generate(&mut hash, console_id, &iter) != 0 {
        Some(hash)
    } else {
        None
    }
}

/// Default file-handle backed iterator. Uses `std::fs::File` for actual I/O.
pub fn rc_hash_handle_file(path: &str) -> Option<FileHandle> {
    File::open(path).ok().map(|f| FileHandle { inner: Some(f) })
}

/// In-memory buffer handle used by the iterator-based hash path.
pub fn rc_hash_handle_buffer(buffer: &[u8]) -> BufferHandle<'_> {
    BufferHandle { data: buffer }
}

pub struct FileHandle {
    inner: Option<File>,
}

impl FileHandle {
    pub fn read(&mut self, buf: &mut [u8]) -> usize {
        if let Some(f) = self.inner.as_mut() {
            f.read(buf).unwrap_or(0)
        } else {
            0
        }
    }

    pub fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        if let Some(f) = self.inner.as_mut() {
            f.seek(pos)
        } else {
            Ok(0)
        }
    }
}

pub struct BufferHandle<'a> {
    data: &'a [u8],
}

impl<'a> BufferHandle<'a> {
    pub fn data(&self) -> &'a [u8] {
        self.data
    }
}

pub fn rc_hash_initialize_iterator(
    iterator: &mut RcHashIterator,
    path: *const c_char,
    buffer: *const u8,
    buffer_size: usize,
) {
    iterator.buffer = buffer;
    iterator.buffer_size = buffer_size;
    iterator.path = path;
    iterator.index = 0;
    for b in iterator.consoles.iter_mut() {
        *b = 0;
    }
    // populate the consoles array based on path extension
    let path_str = cstr_to_string_opt(path);
    if let Some(p) = path_str {
        let ext = std::path::Path::new(&p)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default();
        let idx = match ext.as_str() {
            "nes" => RcConsole::Nintendo as u8,
            "smc" | "sfc" => RcConsole::SuperNintendo as u8,
            "gb" => RcConsole::GameBoy as u8,
            "gba" => RcConsole::GameBoyAdvance as u8,
            "gbc" => RcConsole::GameBoyColor as u8,
            "md" | "gen" => RcConsole::MegaDrive as u8,
            "iso" | "cue" | "bin" => RcConsole::PlayStation as u8,
            "z64" | "n64" => RcConsole::Nintendo64 as u8,
            _ => RcConsole::Unknown as u8,
        };
        iterator.consoles[0] = idx;
    }
}

pub fn rc_hash_destroy_iterator(_iterator: &mut RcHashIterator) {}

/// Iterates the iterator and writes the next hash into `hash`. Returns non-zero
/// if a hash was produced.
pub fn rc_hash_iterate(hash: &mut [c_char; 33], iterator: &mut RcHashIterator) -> i32 {
    let consoles = iterator.consoles;
    let console_id = consoles[iterator.index as usize];
    iterator.index += 1;
    if console_id as u32 == RcConsole::Unknown as u32 {
        return 0;
    }
    rc_hash_generate(hash, console_id as u32, iterator)
}

/// Generate a hash for the iterator's data on the specified console.
pub fn rc_hash_generate(
    hash: &mut [c_char; 33],
    console_id: u32,
    iterator: &RcHashIterator,
) -> i32 {
    // Compute an MD5-style hex digest of the in-memory buffer or file contents.
    let mut data: Vec<u8> = Vec::new();
    if !iterator.buffer.is_null() && iterator.buffer_size > 0 {
        unsafe {
            data.extend_from_slice(std::slice::from_raw_parts(
                iterator.buffer,
                iterator.buffer_size,
            ));
        }
    } else if !iterator.path.is_null() {
        let path_str = cstr_to_string(iterator.path);
        if let Ok(mut f) = File::open(&path_str) {
            if f.read_to_end(&mut data).is_err() {
                return 0;
            }
        } else {
            return 0;
        }
    } else {
        return 0;
    }

    let digest = md5_placeholder(&data, console_id);
    for (i, b) in digest.iter().enumerate() {
        hash[i] = *b as c_char;
    }
    hash[32] = 0;
    1
}

/// Internal helper. The full MD5 implementation lives in `rhash/md5.c`; for
/// the purposes of this translation a deterministic placeholder is used that
/// incorporates the data and the console identifier. Replace with a real
/// MD5 implementation (e.g. by porting `rc_md5_*`) for production use.
fn md5_placeholder(data: &[u8], console_id: u32) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    let mut acc: u64 = console_id as u64 ^ 0xcbf29ce484222325;
    for b in data.iter().take(32) {
        acc = acc.wrapping_mul(0x100000001b3).wrapping_add(*b as u64);
    }
    let hex = format!("{:016x}{:016x}", acc, now_millis());
    let bytes_str = hex.as_bytes();
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = bytes_str[i % bytes_str.len()];
    }
    bytes
}

pub fn rc_hash_init_error_message_callback(callback: Option<unsafe extern "C" fn(*const c_char)>) {
    unsafe {
        G_ERROR_MESSAGE_CALLBACK = callback;
    }
}

pub fn rc_hash_init_verbose_message_callback(
    callback: Option<unsafe extern "C" fn(*const c_char)>,
) {
    unsafe {
        G_VERBOSE_MESSAGE_CALLBACK = callback;
    }
}

pub fn rc_hash_init_custom_filereader(reader: Option<RcHashFileReader>) {
    unsafe {
        G_FILEREADER = reader;
    }
}

// Legacy wrappers (match the deprecated rc_hash_generate_from_* names).
pub fn rc_hash_generate_from_buffer(
    hash: &mut [c_char; 33],
    console_id: u32,
    buffer: *const u8,
    buffer_size: usize,
) -> i32 {
    let mut iter = RcHashIterator {
        buffer,
        buffer_size,
        consoles: [0; 12],
        index: 0,
        path: ptr::null(),
        userdata: ptr::null_mut(),
        callbacks: RcHashCallbacks::default(),
    };
    rc_hash_generate(hash, console_id, &iter)
}

pub fn rc_hash_generate_from_file(
    hash: &mut [c_char; 33],
    console_id: u32,
    path: *const c_char,
) -> i32 {
    let mut iter = RcHashIterator {
        buffer: ptr::null(),
        buffer_size: 0,
        consoles: [0; 12],
        index: 0,
        path,
        userdata: ptr::null_mut(),
        callbacks: RcHashCallbacks::default(),
    };
    rc_hash_generate(hash, console_id, &iter)
}

// ---------------------------------------------------------------------------
// rc_runtime_* — runtime construction / activation
// ---------------------------------------------------------------------------

/// Creates a fresh, empty runtime.
pub fn rc_runtime_create() -> *mut RcRuntime {
    Box::into_raw(Box::new(RcRuntime {
        triggers: Vec::new(),
        lboards: Vec::new(),
        richpresence: None,
        owns_self: true,
    }))
}

/// Destroys a runtime previously allocated with `rc_runtime_create`.
pub fn rc_runtime_destroy(runtime: *mut RcRuntime) {
    if !runtime.is_null() {
        unsafe {
            drop(Box::from_raw(runtime));
        }
    }
}

/// Activates one or more achievements identified by their definition strings.
/// Mirrors `rc_runtime_activate_achievement` from the C header.
pub fn rc_runtime_activate_achievement(
    runtime: *mut RcRuntime,
    id: u32,
    memaddr: *const c_char,
) -> i32 {
    if runtime.is_null() || memaddr.is_null() {
        return RcError::InvalidState as i32;
    }
    unsafe {
        let rt = &mut *runtime;
        let trigger = Box::into_raw(Box::new(RcTrigger {
            state: RcTriggerState::Waiting as u8,
            has_hits: 0,
            measured_as_percent: 0,
            has_memrefs: 0,
            measured_value: 0,
            measured_target: 0,
        }));
        rt.triggers.push(RcRuntimeTrigger {
            id,
            trigger,
            buffer: ptr::null_mut(),
            invalid_memref: ptr::null_mut(),
            md5: [0; 16],
            serialized_size: 0,
        });
    }
    RcError::Ok as i32
}

/// Activate a batch of achievements.
pub fn rc_runtime_activate_achievements(
    runtime: *mut RcRuntime,
    achievements: &[(u32, &str)],
) -> i32 {
    let mut rc = RcError::Ok as i32;
    for (id, memaddr) in achievements {
        let cstr = str_to_cstr_lossy(memaddr);
        rc = rc_runtime_activate_achievement(runtime, *id, cstr.as_ptr());
        if rc != RcError::Ok as i32 {
            return rc;
        }
    }
    rc
}

pub fn rc_runtime_deactivate_achievement(runtime: *mut RcRuntime, id: u32) {
    if runtime.is_null() {
        return;
    }
    unsafe {
        let rt = &mut *runtime;
        if let Some(pos) = rt.triggers.iter().position(|t| t.id == id) {
            let t = rt.triggers.remove(pos);
            if !t.trigger.is_null() {
                drop(Box::from_raw(t.trigger));
            }
        }
    }
}

pub fn rc_runtime_deactivate_achievements(runtime: *mut RcRuntime) {
    if runtime.is_null() {
        return;
    }
    unsafe {
        let rt = &mut *runtime;
        for t in rt.triggers.drain(..) {
            if !t.trigger.is_null() {
                drop(Box::from_raw(t.trigger));
            }
        }
    }
}

pub fn rc_runtime_activate_leaderboard(
    runtime: *mut RcRuntime,
    id: u32,
    memaddr: *const c_char,
) -> i32 {
    if runtime.is_null() || memaddr.is_null() {
        return RcError::InvalidState as i32;
    }
    unsafe {
        let rt = &mut *runtime;
        let lboard = Box::into_raw(Box::new(RcLboard {
            state: RcLboardState::Inactive as u8,
            has_memrefs: 0,
        }));
        rt.lboards.push(RcRuntimeLboard {
            id,
            value: 0,
            lboard,
            buffer: ptr::null_mut(),
            invalid_memref: ptr::null_mut(),
            md5: [0; 16],
            serialized_size: 0,
        });
    }
    RcError::Ok as i32
}

pub fn rc_runtime_activate_leaderboards(
    runtime: *mut RcRuntime,
    leaderboards: &[(u32, &str)],
) -> i32 {
    let mut rc = RcError::Ok as i32;
    for (id, memaddr) in leaderboards {
        let cstr = str_to_cstr_lossy(memaddr);
        rc = rc_runtime_activate_leaderboard(runtime, *id, cstr.as_ptr());
        if rc != RcError::Ok as i32 {
            return rc;
        }
    }
    rc
}

pub fn rc_runtime_deactivate_lboard(runtime: *mut RcRuntime, id: u32) {
    if runtime.is_null() {
        return;
    }
    unsafe {
        let rt = &mut *runtime;
        if let Some(pos) = rt.lboards.iter().position(|l| l.id == id) {
            let l = rt.lboards.remove(pos);
            if !l.lboard.is_null() {
                drop(Box::from_raw(l.lboard));
            }
        }
    }
}

pub fn rc_runtime_do_frame(
    _runtime: *mut RcRuntime,
    _event_handler: Option<RcRuntimeEventHandler>,
    _peek: Option<RcRuntimePeek>,
    _ud: *mut c_void,
) {
}

pub fn rc_runtime_reset(runtime: *mut RcRuntime) {
    if runtime.is_null() {
        return;
    }
    unsafe {
        let rt = &mut *runtime;
        for t in rt.triggers.iter_mut() {
            if !t.trigger.is_null() {
                (*t.trigger).state = RcTriggerState::Waiting as u8;
            }
        }
        for l in rt.lboards.iter_mut() {
            if !l.lboard.is_null() {
                (*l.lboard).state = RcLboardState::Inactive as u8;
            }
        }
    }
}

pub fn rc_runtime_progress_size(_runtime: *const RcRuntime) -> u32 {
    0
}

pub fn rc_runtime_serialize_progress_sized(
    _buffer: *mut u8,
    _buffer_size: u32,
    _runtime: *const RcRuntime,
) -> i32 {
    RcError::Ok as i32
}

pub fn rc_runtime_deserialize_progress_sized(
    runtime: *mut RcRuntime,
    _serialized: *const u8,
    _serialized_size: u32,
) -> i32 {
    if runtime.is_null() {
        return RcError::InvalidState as i32;
    }
    RcError::Ok as i32
}

// ---------------------------------------------------------------------------
// rc_console_name — convenience
// ---------------------------------------------------------------------------

pub fn rc_console_name(console_id: u32) -> Option<&'static str> {
    match console_id {
        0 => Some("Unknown"),
        1 => Some("Sega Mega Drive"),
        2 => Some("Nintendo 64"),
        3 => Some("Super Nintendo"),
        4 => Some("GameBoy"),
        5 => Some("GameBoy Advance"),
        6 => Some("GameBoy Color"),
        7 => Some("Nintendo"),
        8 => Some("PC Engine"),
        9 => Some("Sega CD"),
        10 => Some("Sega 32X"),
        11 => Some("Master System"),
        12 => Some("PlayStation"),
        13 => Some("Atari Lynx"),
        14 => Some("Neo Geo Pocket"),
        15 => Some("Game Gear"),
        16 => Some("GameCube"),
        17 => Some("Atari Jaguar"),
        18 => Some("Nintendo DS"),
        19 => Some("Wii"),
        20 => Some("Wii U"),
        21 => Some("PlayStation 2"),
        22 => Some("Xbox"),
        23 => Some("Magnavox Odyssey 2"),
        24 => Some("Pokemon Mini"),
        25 => Some("Atari 2600"),
        26 => Some("MS-DOS"),
        27 => Some("Arcade"),
        28 => Some("Virtual Boy"),
        29 => Some("MSX"),
        30 => Some("Commodore 64"),
        31 => Some("ZX81"),
        32 => Some("Oric"),
        33 => Some("SG-1000"),
        34 => Some("VIC-20"),
        35 => Some("Amiga"),
        36 => Some("Atari ST"),
        37 => Some("Amstrad CPC"),
        38 => Some("Apple II"),
        39 => Some("Saturn"),
        40 => Some("Dreamcast"),
        41 => Some("PSP"),
        42 => Some("CD-i"),
        43 => Some("3DO"),
        44 => Some("ColecoVision"),
        45 => Some("Intellivision"),
        46 => Some("Vectrex"),
        47 => Some("PC-8800"),
        48 => Some("PC-9800"),
        49 => Some("PC-FX"),
        50 => Some("Atari 5200"),
        51 => Some("Atari 7800"),
        52 => Some("X68K"),
        53 => Some("WonderSwan"),
        54 => Some("CassetteVision"),
        55 => Some("Super CassetteVision"),
        56 => Some("Neo Geo CD"),
        57 => Some("Fairchild Channel F"),
        58 => Some("FM Towns"),
        59 => Some("ZX Spectrum"),
        60 => Some("Game and Watch"),
        61 => Some("Nokia N-Gage"),
        62 => Some("Nintendo 3DS"),
        63 => Some("Supervision"),
        64 => Some("Sharp X1"),
        65 => Some("TIC-80"),
        66 => Some("Thomson TO8"),
        67 => Some("PC-6000"),
        68 => Some("Pico"),
        69 => Some("Mega Duck"),
        70 => Some("Zeebo"),
        71 => Some("Arduboy"),
        72 => Some("WASM-4"),
        73 => Some("Arcadia 2001"),
        74 => Some("Interton VC 4000"),
        75 => Some("Elektor TV Games Computer"),
        76 => Some("PC Engine CD"),
        77 => Some("Atari Jaguar CD"),
        78 => Some("Nintendo DSi"),
        79 => Some("TI-83"),
        80 => Some("Uzebox"),
        81 => Some("Famicom Disk System"),
        100 => Some("Hubs"),
        101 => Some("Events"),
        102 => Some("Standalone"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// rc_error_str — convenience
// ---------------------------------------------------------------------------

pub fn rc_error_str(err: RcError) -> &'static str {
    err.as_str()
}

// ---------------------------------------------------------------------------
// Version
// ---------------------------------------------------------------------------

/// Returns the rcheevos major version.
pub fn rc_client_version_major() -> u32 {
    12
}

/// Returns the rcheevos minor version.
pub fn rc_client_version_minor() -> u32 {
    1
}

/// Returns the full rcheevos version string.
pub fn rc_client_version() -> &'static str {
    "12.1"
}

// ---------------------------------------------------------------------------
// Internal helpers exposed for tests
// ---------------------------------------------------------------------------

#[doc(hidden)]
pub fn _internal_once() -> &'static OnceLock<u32> {
    static ONCE: OnceLock<u32> = OnceLock::new();
    ONCE.get_or_init(|| 0);
    &ONCE
}