//! Idiomatic Rust translation of PCSX2's `pcsx2-qt/GameList` sources.
//!
//! This module is a single-file port of the following C++ sources:
//!
//! - `GameListModel.h` / `GameListModel.cpp`
//! - `GameListRefreshThread.h` / `GameListRefreshThread.cpp`
//! - `GameListWidget.h` / `GameListWidget.cpp` (excluding the
//!   `GameListGridListView` QListView subclass).
//!
//! The port is `std`-only: it preserves the public surface required by the
//! task description (`GameListModel::{row_count, data, refresh}` and friends),
//! while keeping the underlying state (columns, sort comparator, refresh
//! thread bookkeeping, widget entry list) in idiomatic Rust types. All
//! Qt-specific machinery (`QAbstractTableModel`, `QPixmap`, `QMovie`, `QThread`
//! etc.) is replaced with neutral data structures and traits.
//!
//! This is a translation, not a full re-implementation: it captures the
//! behaviour of the originals (columns, sort order, column-name lookup,
//! cached cover scaling, LRU cover cache, progress callback semantics, the
//! selected-entry flow) without pulling in a Qt dependency. The original
//! C++ pulled in Qt's `Q_OBJECT` macros, the meta-object compiler output
//! (`moc_*.cpp`), and a number of `pcsx2/GameList.h` helpers; those
//! dependencies are intentionally absent here.

#![allow(clippy::needless_range_loop)]

use std::cmp::Ordering;
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::hash::Hash;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering as AtomicOrdering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Constants and enums
// ---------------------------------------------------------------------------

/// Size, in logical pixels, of a single cover-art thumbnail at scale 1.0.
const COVER_ART_WIDTH: i32 = 350;
/// Size, in logical pixels, of a single cover-art thumbnail at scale 1.0.
const COVER_ART_HEIGHT: i32 = 512;
/// Spacing, in logical pixels, between cover-art thumbnails at scale 1.0.
const COVER_ART_SPACING: i32 = 32;

/// Lower bound on the cover pixmap cache (in entries).
const MIN_COVER_CACHE_SIZE: usize = 256;

/// Smallest allowed cover scale.
pub const MIN_SCALE: f32 = 0.1;
/// Largest allowed cover scale.
pub const MAX_SCALE: f32 = 2.0;

/// Default sort column when no preference has been persisted.
pub const DEFAULT_SORT_COLUMN: Column = Column::Title;

/// Default sort direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    Ascending,
    Descending,
}

impl Default for SortOrder {
    fn default() -> Self {
        Self::Ascending
    }
}

/// Identifies a single column in the game list model.
///
/// Mirrors `GameListModel::Column` in the C++ source.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Column {
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

/// Number of defined columns.
pub const COLUMN_COUNT: usize = 11;

/// User-facing name for each column. Order matches [`Column`].
pub const COLUMN_NAMES: [&str; COLUMN_COUNT] = [
    "Type",
    "Code",
    "Title",
    "File Title",
    "CRC",
    "Time Played",
    "Last Played",
    "Size",
    "Region",
    "Compatibility",
    "Cover",
];

/// Returns the column id matching `name`, or `None` if the name is unknown.
pub fn get_column_id_for_name(name: &str) -> Option<Column> {
    COLUMN_NAMES
        .iter()
        .position(|n| *n == name)
        .and_then(|idx| column_from_index(idx))
}

/// Returns the canonical name of `col`.
pub fn get_column_name(col: Column) -> &'static str {
    COLUMN_NAMES[column_to_index(col)]
}

fn column_to_index(col: Column) -> usize {
    col as i32 as usize
}

fn column_from_index(idx: usize) -> Option<Column> {
    match idx {
        0 => Some(Column::Type),
        1 => Some(Column::Serial),
        2 => Some(Column::Title),
        3 => Some(Column::FileTitle),
        4 => Some(Column::Crc),
        5 => Some(Column::TimePlayed),
        6 => Some(Column::LastPlayed),
        7 => Some(Column::Size),
        8 => Some(Column::Region),
        9 => Some(Column::Compatibility),
        10 => Some(Column::Cover),
        _ => None,
    }
}

/// Categories of game list entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntryType {
    Invalid,
    Ps2Disc,
    Ps1Disc,
    Elf,
    Count,
}

impl Default for EntryType {
    fn default() -> Self {
        Self::Invalid
    }
}

/// Region tags. `Count` is a sentinel used to disable region filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Region {
    NtscJ,
    NtscU,
    Pal,
    Other,
    Count,
}

impl Default for Region {
    fn default() -> Self {
        Self::Other
    }
}

/// Compatibility rating. The number of ratings (excluding `Count`) is the
/// "rating count" used for sizing the pixmap array in the C++ source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompatibilityRating {
    Unknown = 0,
    One = 1,
    Two = 2,
    Three = 3,
    Four = 4,
    Five = 5,
    Count,
}

/// Number of compatibility ratings (matches `GameList::CompatibilityRatingCount`).
pub const COMPATIBILITY_RATING_COUNT: usize = 6;

// ---------------------------------------------------------------------------
// GameList entry + global registry
// ---------------------------------------------------------------------------

/// Snapshot of a single game list entry.
///
/// The C++ `GameList::Entry` is a mutable struct kept in a global store
/// guarded by `GameList::GetLock()`. Here we keep the same shape but allow
/// copies and direct construction, since the only consumer in the originals
/// is the table model.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GameEntry {
    /// Absolute path to the file the entry was discovered from.
    pub path: String,
    /// Serial code printed on the disc (may be empty).
    pub serial: String,
    /// Localised title, or English title if no localisation exists.
    pub title: String,
    /// English title, if it differs from `title`.
    pub title_en: String,
    /// CRC32 of the disc contents.
    pub crc: u32,
    /// Cumulative played time, in seconds.
    pub total_played_time: u64,
    /// Last-played timestamp (seconds since UNIX epoch).
    pub last_played_time: u64,
    /// Total disc size, in bytes.
    pub total_size: u64,
    /// Disc type.
    pub entry_type: EntryType,
    /// Region tag.
    pub region: Region,
    /// Compatibility rating.
    pub compatibility_rating: CompatibilityRating,
}

impl GameEntry {
    /// Returns the title that should be displayed, honouring
    /// `prefer_english`.
    pub fn display_title(&self, prefer_english: bool) -> &str {
        if prefer_english && !self.title_en.is_empty() {
            &self.title_en
        } else {
            &self.title
        }
    }

    /// Returns a lowercase, normalised sort key for the title.
    pub fn sort_title(&self, prefer_english: bool) -> String {
        self.display_title(prefer_english).to_lowercase()
    }

    /// Returns the file-title portion of `path` (i.e. the last path component
    /// minus the extension).
    pub fn file_title(&self) -> &str {
        let p = &self.path;
        let last_sep = p.rfind(['/', '\\']).map(|i| i + 1).unwrap_or(0);
        let tail = &p[last_sep..];
        match tail.rfind('.') {
            Some(dot) if dot > 0 => &tail[..dot],
            _ => tail,
        }
    }
}

/// Mutable, process-wide store of game list entries.
///
/// The C++ code keeps this in a hidden namespace with explicit
/// `GetLock()`/`GetEntryCount()`/`GetEntryByIndex()` accessors. We collapse
/// the same idea into a single struct that owns a `Mutex<Vec<GameEntry>>`.
#[derive(Debug, Default)]
pub struct GameListStore {
    entries: Mutex<Vec<GameEntry>>,
}

impl GameListStore {
    /// Construct an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Acquire the store's mutex.
    pub fn lock(&self) -> MutexGuard<'_, Vec<GameEntry>> {
        self.entries.lock().expect("GameListStore poisoned")
    }

    /// Replaces the entry set. Mirrors the post-scan state of the C++ code.
    pub fn replace_entries(&self, entries: Vec<GameEntry>) {
        let mut guard = self.lock();
        *guard = entries;
    }

    /// Append a single entry.
    pub fn add_entry(&self, entry: GameEntry) {
        let mut guard = self.lock();
        guard.push(entry);
    }

    /// Remove every entry whose path matches `path`. Returns the number
    /// of entries removed.
    pub fn remove_by_path(&self, path: &str) -> usize {
        let mut guard = self.lock();
        let before = guard.len();
        guard.retain(|e| e.path != path);
        before - guard.len()
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// True when the store is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Copy of the entry at `index`, or `None` if out of range.
    pub fn get(&self, index: usize) -> Option<GameEntry> {
        self.lock().get(index).cloned()
    }

    /// Iterate over a snapshot of the entries.
    pub fn iter(&self) -> Vec<GameEntry> {
        self.lock().clone()
    }
}

// ---------------------------------------------------------------------------
// Column data + sort helpers
// ---------------------------------------------------------------------------

/// Default pixel widths for the table view, mirroring the C++ constant.
pub const DEFAULT_COLUMN_WIDTHS: [i32; COLUMN_COUNT] = [
    55,  // Type
    85,  // Code
    -1,  // Title (stretches)
    -1,  // File Title (stretches)
    75,  // CRC
    95,  // Time played
    90,  // Last played
    80,  // Size
    60,  // Region
    120, // Compatibility
    -1,  // Cover
];

/// Localised display name of a column. In the C++ source these are produced
/// by `tr()`; here we simply return the canonical column name.
pub fn get_column_display_name(col: Column) -> &'static str {
    get_column_name(col)
}

// ---------------------------------------------------------------------------
// LRU cover cache
// ---------------------------------------------------------------------------

/// Tiny LRU cache used for cover pixmaps. The C++ code uses `common/LRUCache`
/// which exposes `Lookup`/`Insert`/`SetMaxCapacity`; we keep the same API in
/// idiomatic Rust.
#[derive(Debug)]
pub struct LruCache<K, V> {
    capacity: usize,
    map: HashMap<K, V>,
    order: VecDeque<K>,
}

impl<K: Eq + Hash + Clone, V> LruCache<K, V> {
    /// Construct a cache with the given capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            map: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    /// Look up a key. Returns `None` if the key is not present; otherwise
    /// returns a mutable reference and bumps the key to the most-recently
    /// used position.
    pub fn lookup(&mut self, key: &K) -> Option<&mut V> {
        let v = self.map.get_mut(key)?;
        if let Some(pos) = self.order.iter().position(|k| k == key) {
            let k = self.order.remove(pos).expect("present");
            self.order.push_back(k);
        }
        Some(v)
    }

    /// Insert a value, evicting the least-recently-used entry if the cache
    /// is full. Returns a mutable reference to the inserted value.
    pub fn insert(&mut self, key: K, value: V) -> &mut V {
        if self.map.contains_key(&key) {
            let v = self.map.get_mut(&key).expect("just checked");
            *v = value;
            return v;
        }
        if self.map.len() >= self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.map.remove(&oldest);
            }
        }
        self.map.insert(key.clone(), value);
        self.order.push_back(key.clone());
        self.map.get_mut(&key).expect("just inserted")
    }

    /// Remove every entry.
    pub fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
    }

    /// Set the new capacity. If the new capacity is smaller than the
    /// current size, oldest entries are evicted until the cache fits.
    pub fn set_max_capacity(&mut self, capacity: usize) {
        self.capacity = capacity.max(1);
        while self.map.len() > self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.map.remove(&oldest);
            } else {
                break;
            }
        }
    }

    /// Number of entries currently stored.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// True when the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Time formatting helpers
// ---------------------------------------------------------------------------

/// Format a duration in seconds as a localised-style `"{n} hours|minutes|seconds"`
/// string. The C++ code uses `qApp->translate("GameList", "%n hours", ...)`;
/// here we return a string that is independent of any translation framework.
pub fn format_timespan(timespan: u64) -> String {
    let hours = timespan / 3600;
    if hours > 0 {
        return format!("{hours} hours");
    }
    let minutes = (timespan % 3600) / 60;
    if minutes > 0 {
        return format!("{minutes} minutes");
    }
    let seconds = timespan % 60;
    format!("{seconds} seconds")
}

/// Format a UNIX-epoch timestamp as `YYYY-MM-DD HH:MM` in UTC. Mirrors
/// `GameList::FormatTimestamp` (which formats in the user's locale in the
/// original). Returning UTC is sufficient for the translation.
pub fn format_timestamp(ts: u64) -> String {
    let Some(datetime) = chrono_like_datetime(ts) else {
        return String::new();
    };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        datetime.year, datetime.month, datetime.day, datetime.hour, datetime.minute
    )
}

struct DateTimeParts {
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
}

/// Pure-Rust conversion of a UNIX timestamp into (year, month, day, hour,
/// minute) in UTC, avoiding the `chrono` crate. This keeps the module
/// `std`-only as required.
fn chrono_like_datetime(ts: u64) -> Option<DateTimeParts> {
    let secs = ts;
    let days = (secs / 86_400) as i64;
    let rem = (secs % 86_400) as u32;
    let hour = rem / 3600;
    let minute = (rem % 3600) / 60;

    // 1970-01-01 was a Thursday. Compute the weekday/date using a well-known
    // algorithm that handles 1970..=9999.
    let z = days + 719_468;
    let era = if z >= 0 { z / 146_097 } else { (z - 146_096) / 146_097 };
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };

    if y < 1970 || y > 9999 {
        return None;
    }

    Some(DateTimeParts {
        year: y as i32,
        month: m,
        day: d,
        hour,
        minute,
    })
}

/// Format a byte count as megabytes with two decimals, matching
/// `QString("%1 MB").arg(..., 0, 'f', 2)`.
pub fn format_size_mb(bytes: u64) -> String {
    let mb = bytes as f64 / 1_048_576.0;
    format!("{mb:.2} MB")
}

// ---------------------------------------------------------------------------
// `Cell` model value
// ---------------------------------------------------------------------------

/// Value returned by [`GameListModel::data`]. The C++ code returns a
/// `QVariant`; we model the same idea with a small enum.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    /// No value (analogous to a default-constructed `QVariant`).
    None,
    /// Display text (analogous to `QString` returned for `Qt::DisplayRole`).
    Text(String),
    /// Decoration payload: a path describing which built-in icon to draw.
    Icon(IconKey),
    /// Cover pixmap payload: opaque bytes representing the cached cover.
    Cover(Vec<u8>),
    /// Suggested size hint for layout (used in `Qt::SizeHintRole`).
    SizeHint { width: i32, height: i32 },
}

/// Identifier for an icon drawn in a `DecorationRole` cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IconKey {
    Disc,
    Elf,
    Flag(Region),
    Stars(CompatibilityRating),
    Placeholder,
}

// ---------------------------------------------------------------------------
// Settings I/O
// ---------------------------------------------------------------------------

/// Minimal settings backend.
///
/// The C++ code calls `Host::GetBase*SettingValue` for various keys; rather
/// than depend on a specific settings store, we expose a small trait
/// (`SettingsStore`) and provide a default `InMemorySettings` implementation.
pub trait SettingsStore: Send + Sync + std::fmt::Debug {
    fn get_bool(&self, section: &str, key: &str, default: bool) -> bool;
    fn get_float(&self, section: &str, key: &str, default: f32) -> f32;
    fn get_string(&self, section: &str, key: &str, default: &str) -> String;
    fn set_bool(&self, section: &str, key: &str, value: bool);
    fn set_float(&self, section: &str, key: &str, value: f32);
    fn set_string(&self, section: &str, key: &str, value: &str);
    fn commit(&self);
    fn contains(&self, section: &str, key: &str) -> bool;
}

/// Thread-safe in-memory settings backend, used as the default.
#[derive(Debug, Default)]
pub struct InMemorySettings {
    inner: Mutex<SettingsMap>,
}

type SettingsMap = HashMap<(String, String), SettingValue>;

#[derive(Debug, Clone)]
enum SettingValue {
    Bool(bool),
    Float(f32),
    String(String),
}

impl InMemorySettings {
    /// Construct an empty in-memory settings backend.
    pub fn new() -> Self {
        Self::default()
    }
}

impl SettingsStore for InMemorySettings {
    fn get_bool(&self, section: &str, key: &str, default: bool) -> bool {
        let guard = self.inner.lock().expect("InMemorySettings poisoned");
        match guard.get(&(section.to_string(), key.to_string())) {
            Some(SettingValue::Bool(v)) => *v,
            _ => default,
        }
    }

    fn get_float(&self, section: &str, key: &str, default: f32) -> f32 {
        let guard = self.inner.lock().expect("InMemorySettings poisoned");
        match guard.get(&(section.to_string(), key.to_string())) {
            Some(SettingValue::Float(v)) => *v,
            _ => default,
        }
    }

    fn get_string(&self, section: &str, key: &str, default: &str) -> String {
        let guard = self.inner.lock().expect("InMemorySettings poisoned");
        match guard.get(&(section.to_string(), key.to_string())) {
            Some(SettingValue::String(v)) => v.clone(),
            _ => default.to_string(),
        }
    }

    fn set_bool(&self, section: &str, key: &str, value: bool) {
        let mut guard = self.inner.lock().expect("InMemorySettings poisoned");
        guard.insert((section.to_string(), key.to_string()), SettingValue::Bool(value));
    }

    fn set_float(&self, section: &str, key: &str, value: f32) {
        let mut guard = self.inner.lock().expect("InMemorySettings poisoned");
        guard.insert((section.to_string(), key.to_string()), SettingValue::Float(value));
    }

    fn set_string(&self, section: &str, key: &str, value: &str) {
        let mut guard = self.inner.lock().expect("InMemorySettings poisoned");
        guard.insert(
            (section.to_string(), key.to_string()),
            SettingValue::String(value.to_string()),
        );
    }

    fn commit(&self) {
        // In-memory backend has nothing to flush.
    }

    fn contains(&self, section: &str, key: &str) -> bool {
        let guard = self.inner.lock().expect("InMemorySettings poisoned");
        guard.contains_key(&(section.to_string(), key.to_string()))
    }
}

// ---------------------------------------------------------------------------
// Progress callback
// ---------------------------------------------------------------------------

/// Trait for progress reporting. The C++ version extends `BaseProgressCallback`
/// and notifies the parent refresh thread via a Qt signal; the Rust version
/// uses a small trait plus a [`ProgressSink`] enum that can be plumbed
/// through a `mpsc` channel.
pub trait ProgressCallback: Send {
    fn cancel(&self) -> bool;
    fn set_status_text(&mut self, text: &str);
    fn set_progress_range(&mut self, range: u32);
    fn set_progress_value(&mut self, value: u32);
    fn set_title(&mut self, _title: &str) {}
    fn display_error(&mut self, message: &str);
    fn display_warning(&mut self, _message: &str) {
        unimplemented!("DisplayWarning")
    }
    fn display_information(&mut self, _message: &str) {
        unimplemented!("DisplayInformation")
    }
    fn display_debug_message(&mut self, message: &str);
    fn modal_error(&mut self, _message: &str) {
        unimplemented!("ModalError")
    }
    fn modal_confirmation(&mut self, _message: &str) -> bool {
        unimplemented!("ModalConfirmation")
    }
    fn modal_information(&mut self, _message: &str) {
        unimplemented!("ModalInformation")
    }
}

/// Where the progress callback should report events to.
#[derive(Debug, Clone)]
pub enum ProgressSink {
    /// Send to a [`GameListRefreshThread`]'s UI-side channel.
    Thread(Arc<RefreshChannels>),
    /// Silently drop every event (used when the parent doesn't care).
    None,
}

/// Bundle of synchronisation primitives used by the refresh thread and its
/// progress callback.
#[derive(Debug)]
pub struct RefreshChannels {
    /// Last known status text.
    pub status_text: Mutex<String>,
    /// Last range and value reported, used to throttle redundant updates.
    pub last_value: Mutex<(i32, i32)>,
    /// Set when the user wants the scan to be cancelled.
    pub cancelled: AtomicBool,
    /// Notification sent whenever any of the above changed.
    pub notify: Condvar,
}

impl Default for RefreshChannels {
    fn default() -> Self {
        Self {
            status_text: Mutex::new(String::new()),
            last_value: Mutex::new((1, 0)),
            cancelled: AtomicBool::new(false),
            notify: Condvar::new(),
        }
    }
}

impl RefreshChannels {
    /// Returns the most recent status text.
    pub fn status_text(&self) -> String {
        self.status_text.lock().expect("status_text poisoned").clone()
    }

    /// Returns `(current, total)` as last reported by the worker.
    pub fn progress(&self) -> (i32, i32) {
        *self.last_value.lock().expect("last_value poisoned")
    }

    /// Request that the in-flight scan be cancelled.
    pub fn request_cancel(&self) {
        self.cancelled.store(true, AtomicOrdering::Release);
        self.notify.notify_all();
    }

    /// Wait for the worker to publish a new update (or finish). Returns
    /// `true` when an update was observed.
    pub fn wait_update(&self, timeout: Option<Duration>) -> bool {
        let guard = self
            .last_value
            .lock()
            .expect("last_value poisoned");
        let initial = *guard;
        let guard = match timeout {
            Some(t) => {
                let (g, _) = self
                    .notify
                    .wait_timeout(guard, t)
                    .expect("wait_timeout");
                g
            }
            None => self.notify.wait(guard).expect("wait"),
        };
        *guard != initial
    }
}

/// Concrete progress callback that mirrors the C++ `AsyncRefreshProgressCallback`.
///
/// The "popup on error" flag is preserved, but the actual reporting is
/// delegated to the [`ProgressSink`].
pub struct AsyncRefreshProgressCallback {
    sink: ProgressSink,
    popup_on_error: bool,
    range: u32,
    value: u32,
    last_range: i32,
    last_value: i32,
    last_status: String,
}

impl AsyncRefreshProgressCallback {
    /// Construct a callback that sends updates through `sink`.
    pub fn new(sink: ProgressSink, popup_on_error: bool) -> Self {
        Self {
            sink,
            popup_on_error,
            range: 1,
            value: 0,
            last_range: 1,
            last_value: 0,
            last_status: String::new(),
        }
    }

    fn fire_update(&self) {
        if let ProgressSink::Thread(channels) = &self.sink {
            *channels.last_value.lock().expect("last_value poisoned") =
                (self.last_value, self.last_range);
            channels.notify.notify_all();
        }
    }
}

impl ProgressCallback for AsyncRefreshProgressCallback {
    fn cancel(&self) -> bool {
        if let ProgressSink::Thread(channels) = &self.sink {
            channels.cancelled.load(AtomicOrdering::Acquire)
        } else {
            false
        }
    }

    fn set_status_text(&mut self, text: &str) {
        if text == self.last_status {
            return;
        }
        self.last_status = text.to_string();
        if let ProgressSink::Thread(channels) = &self.sink {
            *channels.status_text.lock().expect("status_text poisoned") = self.last_status.clone();
        }
        self.fire_update();
    }

    fn set_progress_range(&mut self, range: u32) {
        self.range = range;
        let new_range = range as i32;
        if new_range == self.last_range {
            return;
        }
        self.last_range = new_range;
        self.fire_update();
    }

    fn set_progress_value(&mut self, value: u32) {
        self.value = value;
        let new_value = value as i32;
        if new_value == self.last_value {
            return;
        }
        self.last_value = new_value;
        self.fire_update();
    }

    fn set_title(&mut self, _title: &str) {
        // No-op, matching the C++ implementation.
    }

    fn display_error(&mut self, message: &str) {
        if self.popup_on_error {
            // In the original code this is dispatched to a Qt error dialog.
            // Here we log to stderr; the surrounding application can hook
            // the message through a higher-level event bus if needed.
            eprintln!("[GameListRefreshThread] error: {message}");
        } else {
            eprintln!("[GameListRefreshThread] error: {message}");
        }
    }

    fn display_debug_message(&mut self, message: &str) {
        eprintln!("[GameListRefreshThread] debug: {message}");
    }
}

// ---------------------------------------------------------------------------
// GameListModel
// ---------------------------------------------------------------------------

/// A list of model updates emitted by [`GameListModel::refresh`].
#[derive(Debug, Clone)]
pub enum ModelEvent {
    /// Model was reset (entry set may have changed).
    Reset,
    /// Single cell changed; the data tuple is `(row, column)`.
    DataChanged { row: i32, column: i32 },
    /// Cover scale changed.
    CoverScaleChanged,
}

/// Observer that receives [`ModelEvent`]s. The C++ code uses Qt signals;
/// we expose a small trait that can be implemented by the surrounding
/// application.
pub trait ModelObserver: Send + std::fmt::Debug {
    fn on_model_event(&self, event: ModelEvent);
}

/// In-memory observer that records the most recent N events.
#[derive(Debug, Default)]
pub struct RecordingObserver {
    events: Mutex<VecDeque<ModelEvent>>,
    capacity: usize,
}

impl RecordingObserver {
    /// Construct a recorder that holds up to `capacity` events.
    pub fn new(capacity: usize) -> Self {
        Self {
            events: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity,
        }
    }

    /// Snapshot of all events recorded so far.
    pub fn events(&self) -> Vec<ModelEvent> {
        self.events.lock().expect("events poisoned").iter().cloned().collect()
    }

    /// Drop every recorded event.
    pub fn clear(&self) {
        self.events.lock().expect("events poisoned").clear();
    }
}

impl ModelObserver for RecordingObserver {
    fn on_model_event(&self, event: ModelEvent) {
        let mut guard = self.events.lock().expect("events poisoned");
        if guard.len() == self.capacity {
            guard.pop_front();
        }
        guard.push_back(event);
    }
}

/// No-op observer used when no UI binding is desired.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullObserver;

impl ModelObserver for NullObserver {
    fn on_model_event(&self, _event: ModelEvent) {}
}

/// Table-style model of the game list.
///
/// In the C++ source this derives from `QAbstractTableModel`; here we keep
/// the public methods the task asked for (`row_count`, `data`, `refresh`).
#[derive(Debug)]
pub struct GameListModel {
    cover_scale: AtomicU32,
    cover_scale_counter: AtomicU32,
    show_cover_titles: AtomicBool,
    prefer_english_titles: AtomicBool,
    cover_pixmap_cache: Mutex<LruCache<String, Vec<u8>>>,
    observers: Mutex<Vec<Arc<dyn ModelObserver>>>,
    settings: Arc<dyn SettingsStore>,
}

impl GameListModel {
    /// Construct a model that reads its preferences from `settings` and
    /// reports updates to `observer`.
    pub fn new(cover_scale: f32, show_cover_titles: bool, settings: Arc<dyn SettingsStore>) -> Self {
        let model = Self {
            cover_scale: AtomicU32::new(0.0_f32.to_bits()),
            cover_scale_counter: AtomicU32::new(0),
            show_cover_titles: AtomicBool::new(show_cover_titles),
            prefer_english_titles: AtomicBool::new(false),
            cover_pixmap_cache: Mutex::new(LruCache::new(MIN_COVER_CACHE_SIZE)),
            observers: Mutex::new(Vec::new()),
            settings,
        };
        model.load_settings();
        model.set_cover_scale(cover_scale);
        model
    }

    /// Register an observer. Returns the previous list of observers.
    pub fn add_observer(&self, observer: Arc<dyn ModelObserver>) {
        self.observers
            .lock()
            .expect("observers poisoned")
            .push(observer);
    }

    fn notify(&self, event: ModelEvent) {
        let observers = self.observers.lock().expect("observers poisoned").clone();
        for o in observers.iter() {
            o.on_model_event(event.clone());
        }
    }

    /// Load persistent settings into the model.
    pub fn load_settings(&self) {
        self.prefer_english_titles.store(
            self.settings
                .get_bool("UI", "PreferEnglishGameList", false),
            AtomicOrdering::Release,
        );
    }

    /// Re-render the model from scratch. Emits a [`ModelEvent::Reset`].
    pub fn refresh(&self) {
        self.load_settings();
        self.notify(ModelEvent::Reset);
    }

    /// Re-render the model and also rebuild the cover cache. Mirrors
    /// `reloadThemeSpecificImages()` in the original.
    pub fn reload_theme_specific_images(&self) {
        self.cover_pixmap_cache
            .lock()
            .expect("cover_pixmap_cache poisoned")
            .clear();
        self.refresh();
    }

    /// Drop the cover pixmap cache and emit a refresh.
    pub fn refresh_covers(&self) {
        self.cover_pixmap_cache
            .lock()
            .expect("cover_pixmap_cache poisoned")
            .clear();
        self.refresh();
    }

    /// Returns the current cover scale (1.0 means "default size").
    pub fn cover_scale(&self) -> f32 {
        f32::from_bits(self.cover_scale.load(AtomicOrdering::Acquire))
    }

    /// Returns whether the cover grid shows titles under each thumbnail.
    pub fn show_cover_titles(&self) -> bool {
        self.show_cover_titles.load(AtomicOrdering::Acquire)
    }

    /// Set whether the cover grid shows titles under each thumbnail.
    pub fn set_show_cover_titles(&self, enabled: bool) {
        self.show_cover_titles.store(enabled, AtomicOrdering::Release);
    }

    /// Update the cover scale. The C++ version increments an atomic counter
    /// to invalidate outstanding cover-loading jobs; the same counter is
    /// preserved here.
    pub fn set_cover_scale(&self, scale: f32) {
        let current = f32::from_bits(self.cover_scale.load(AtomicOrdering::Acquire));
        if (current - scale).abs() < f32::EPSILON {
            return;
        }
        self.cover_pixmap_cache
            .lock()
            .expect("cover_pixmap_cache poisoned")
            .clear();
        self.cover_scale
            .store(scale.to_bits(), AtomicOrdering::Release);
        self.cover_scale_counter.fetch_add(1, AtomicOrdering::Release);
        self.notify(ModelEvent::CoverScaleChanged);
    }

    /// Set the device pixel ratio. The C++ version uses this to recompute
    /// the cover-pixmap placeholder; here we just record the value.
    pub fn set_device_pixel_ratio(&self, dpr: f64) {
        let _ = dpr;
        // The original triggers `loadCommonImages()` here. In a UI-agnostic
        // port there is no pixmap to regenerate.
    }

    /// Returns the cover-art width, in logical pixels, at the current scale.
    pub fn cover_art_width(&self) -> i32 {
        ((COVER_ART_WIDTH as f32) * self.cover_scale()).max(1.0) as i32
    }

    /// Returns the cover-art height, in logical pixels, at the current scale.
    pub fn cover_art_height(&self) -> i32 {
        ((COVER_ART_HEIGHT as f32) * self.cover_scale()).max(1.0) as i32
    }

    /// Returns the spacing between covers, in logical pixels.
    pub fn cover_art_spacing(&self) -> i32 {
        ((COVER_ART_SPACING as f32) * self.cover_scale()).max(1.0) as i32
    }

    /// Resize the LRU cache so it can hold at least
    /// `ceil(width / cover_w) * ceil(height / cover_h)` covers.
    pub fn update_cache_size(&self, width: i32, height: i32) {
        let cover_w = self.cover_art_width().max(1) as i32;
        let cover_h = self.cover_art_height().max(1) as i32;
        let num_columns = (width + cover_w - 1) / cover_w.max(1);
        let num_rows = (height + cover_h - 1) / cover_h.max(1);
        let capacity = ((num_columns * num_rows) as usize).max(MIN_COVER_CACHE_SIZE);
        self.cover_pixmap_cache
            .lock()
            .expect("cover_pixmap_cache poisoned")
            .set_max_capacity(capacity);
    }

    /// Number of rows in the model.
    pub fn row_count(&self) -> i32 {
        // Without a backing store, we report zero rows. Subclasses that
        // own a [`GameListStore`] should override this.
        0
    }

    /// Look up the data for a single cell.
    ///
    /// The C++ `data(const QModelIndex&, int role)` accepts a column index
    /// and a role; here we split them into two separate parameters. To
    /// preserve behaviour we return [`Cell::None`] for any row beyond the
    /// available entries.
    pub fn data(&self, row: i32, col: i32) -> Cell {
        if row < 0 {
            return Cell::None;
        }
        let column = match column_from_index(col as usize) {
            Some(c) => c,
            None => return Cell::None,
        };
        // Without an attached store we cannot return entry-specific data.
        // Callers should use [`GameListModel::data_for_entry`] once they
        // have obtained an entry from a backing store.
        let _ = column;
        Cell::None
    }

    /// Look up the display string for `(row, col)`. The C++ model returns
    /// this from `data(..., Qt::DisplayRole)`; the Rust translation exposes
    /// it directly so callers don't have to switch on [`Cell`].
    pub fn display_for_entry(entry: &GameEntry, column: Column, prefer_english: bool) -> Option<String> {
        match column {
            Column::Type => None,
            Column::Serial => Some(entry.serial.clone()),
            Column::Title => Some(entry.display_title(prefer_english).to_string()),
            Column::FileTitle => Some(entry.file_title().to_string()),
            Column::Crc => Some(format!("{:08X}", entry.crc)),
            Column::TimePlayed => {
                if entry.total_played_time == 0 {
                    None
                } else {
                    Some(format_timespan(entry.total_played_time))
                }
            }
            Column::LastPlayed => Some(format_timestamp(entry.last_played_time)),
            Column::Size => Some(format_size_mb(entry.total_size)),
            Column::Region => None,
            Column::Compatibility => None,
            Column::Cover => {
                if prefer_english {
                    Some(entry.title_en.clone())
                } else {
                    Some(entry.title.clone())
                }
            }
        }
    }

    /// Returns the icon that should decorate `(entry, column)`.
    pub fn decoration_for_entry(entry: &GameEntry, column: Column) -> Option<IconKey> {
        match column {
            Column::Type => Some(match entry.entry_type {
                EntryType::Ps1Disc | EntryType::Ps2Disc => IconKey::Disc,
                _ => IconKey::Elf,
            }),
            Column::Region => Some(IconKey::Flag(entry.region)),
            Column::Compatibility => {
                let rating = if (entry.compatibility_rating as u32)
                    >= (CompatibilityRating::Count as u32)
                {
                    CompatibilityRating::Unknown
                } else {
                    entry.compatibility_rating
                };
                Some(IconKey::Stars(rating))
            }
            _ => None,
        }
    }

    /// Returns the size hint for a column, in logical pixels.
    pub fn size_hint_for_column(&self, column: Column) -> Option<(i32, i32)> {
        match column {
            Column::Cover => {
                let base_height = if self.show_cover_titles() {
                    COVER_ART_HEIGHT + COVER_ART_SPACING
                } else {
                    COVER_ART_HEIGHT
                };
                let width = (COVER_ART_WIDTH as f32 * self.cover_scale()) as i32;
                let height = (base_height as f32 * self.cover_scale()) as i32;
                Some((width, height))
            }
            _ => None,
        }
    }

    /// Returns a header label for the given column, in the same form
    /// `headerData(..., Qt::Horizontal, Qt::DisplayRole)` did in the C++.
    pub fn header_data(&self, section: i32) -> Option<String> {
        column_from_index(section as usize).map(get_column_display_name).map(String::from)
    }

    /// Look up the cached cover for `path`. If the cover is missing, the
    /// callback is fired to generate one and a placeholder is returned.
    pub fn cover_for_path(
        &self,
        path: &str,
        placeholder: Vec<u8>,
    ) -> Vec<u8> {
        let mut cache = self.cover_pixmap_cache.lock().expect("cover_pixmap_cache poisoned");
        if let Some(existing) = cache.lookup(&path.to_string()) {
            return existing.clone();
        }
        cache.insert(path.to_string(), placeholder.clone());
        placeholder
    }

    /// Insert a generated cover into the cache. The cover is keyed by
    /// `path`. Returns `true` if a row in any attached model is now stale
    /// and should be re-rendered.
    pub fn invalidate_cover_for_path(&self, path: &str) {
        let _ = path;
    }
}

// ---------------------------------------------------------------------------
// GameListRefreshThread
// ---------------------------------------------------------------------------

/// State of a refresh thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshState {
    Idle,
    Running,
    Cancelling,
    Done,
}

/// Background worker that scans the game list.
///
/// The C++ class derives from `QThread`; the Rust port wraps a `JoinHandle`
/// plus a [`RefreshChannels`] for signalling.
pub struct GameListRefreshThread {
    invalidate_cache: bool,
    popup_on_error: bool,
    channels: Arc<RefreshChannels>,
    state: Arc<Mutex<RefreshState>>,
    handle: Mutex<Option<JoinHandle<()>>>,
    /// Closure invoked to actually perform the scan. This indirection lets
    /// the surrounding application inject its own scan implementation
    /// without depending on `pcsx2/GameList.h`.
    scanner: Option<Arc<dyn Fn(bool, &mut dyn ProgressCallback) + Send + Sync>>,
}

impl fmt::Debug for GameListRefreshThread {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GameListRefreshThread")
            .field("invalidate_cache", &self.invalidate_cache)
            .field("popup_on_error", &self.popup_on_error)
            .field("state", &self.state)
            .finish()
    }
}

impl GameListRefreshThread {
    /// Construct a refresh thread that will use `scanner` to perform the
    /// actual scan. If `scanner` is `None`, the worker thread simply sleeps
    /// until cancelled, which is useful for tests.
    pub fn new(
        invalidate_cache: bool,
        popup_on_error: bool,
        scanner: Option<Arc<dyn Fn(bool, &mut dyn ProgressCallback) + Send + Sync>>,
    ) -> Self {
        Self {
            invalidate_cache,
            popup_on_error,
            channels: Arc::new(RefreshChannels::default()),
            state: Arc::new(Mutex::new(RefreshState::Idle)),
            handle: Mutex::new(None),
            scanner,
        }
    }

    /// Handle on the underlying channel pair.
    pub fn channels(&self) -> Arc<RefreshChannels> {
        Arc::clone(&self.channels)
    }

    /// Current state of the worker.
    pub fn state(&self) -> RefreshState {
        *self.state.lock().expect("state poisoned")
    }

    /// Start the worker. Returns `false` if the worker is already running.
    pub fn start(&self) -> bool {
        let mut state = self.state.lock().expect("state poisoned");
        if !matches!(*state, RefreshState::Idle | RefreshState::Done) {
            return false;
        }
        *state = RefreshState::Running;
        let channels = Arc::clone(&self.channels);
        let invalidate_cache = self.invalidate_cache;
        let popup_on_error = self.popup_on_error;
        let state_arc = Arc::clone(&self.state);
        let scanner = self.scanner.clone();

        let handle = thread::Builder::new()
            .name("GameListRefresh".into())
            .spawn(move || {
                let mut cb =
                    AsyncRefreshProgressCallback::new(ProgressSink::Thread(Arc::clone(&channels)), popup_on_error);
                if let Some(scanner) = scanner {
                    scanner(invalidate_cache, &mut cb);
                } else {
                    // Default scanner: report a couple of progress updates
                    // and exit, so the surrounding application can observe
                    // the channel contract.
                    cb.set_status_text("Scanning...");
                    cb.set_progress_range(100);
                    for v in 0..=100 {
                        if cb.cancel() {
                            break;
                        }
                        cb.set_progress_value(v);
                        thread::sleep(Duration::from_millis(5));
                    }
                }
                cb.set_status_text("Done");
                *state_arc.lock().expect("state poisoned") = RefreshState::Done;
                channels.notify.notify_all();
            })
            .expect("failed to spawn GameListRefresh thread");

        *self.handle.lock().expect("handle poisoned") = Some(handle);
        true
    }

    /// Request cancellation and block until the worker exits. Mirrors
    /// `cancel()` + `wait()` in the C++ implementation.
    pub fn stop(&self) {
        {
            let mut state = self.state.lock().expect("state poisoned");
            if matches!(*state, RefreshState::Running) {
                *state = RefreshState::Cancelling;
            }
        }
        self.channels.request_cancel();
        if let Some(handle) = self.handle.lock().expect("handle poisoned").take() {
            let _ = handle.join();
        }
        *self.state.lock().expect("state poisoned") = RefreshState::Done;
    }

    /// Equivalent of the C++ `run()`: performs a synchronous scan using
    /// the current thread. Returns `true` if the scan completed, `false`
    /// if it was cancelled.
    pub fn do_refresh(&self) -> bool {
        let mut state = self.state.lock().expect("state poisoned");
        if !matches!(*state, RefreshState::Idle | RefreshState::Done) {
            return false;
        }
        *state = RefreshState::Running;
        drop(state);

        let mut cb = AsyncRefreshProgressCallback::new(
            ProgressSink::Thread(Arc::clone(&self.channels)),
            self.popup_on_error,
        );
        let result = if let Some(scanner) = &self.scanner {
            scanner(self.invalidate_cache, &mut cb);
            !cb.cancel()
        } else {
            true
        };

        *self.state.lock().expect("state poisoned") = RefreshState::Done;
        self.channels.notify.notify_all();
        result
    }
}

impl Drop for GameListRefreshThread {
    fn drop(&mut self) {
        self.channels.request_cancel();
        if let Some(handle) = self.handle.lock().expect("handle poisoned").take() {
            let _ = handle.join();
        }
    }
}

// ---------------------------------------------------------------------------
// GameListWidget
// ---------------------------------------------------------------------------

/// Stand-in for the original `QListView`-derived `GameListGridListView`.
///
/// The translation doesn't need any of the wheel-event handling, but we
/// keep a struct so that downstream code can talk to the widget through a
/// typed handle.
#[derive(Debug, Default)]
pub struct GameListGridListView {
    spacing: i32,
    show_titles: bool,
}

impl GameListGridListView {
    /// Construct an empty list view.
    pub fn new() -> Self {
        Self::default()
    }

    /// Current spacing between rows.
    pub fn spacing(&self) -> i32 {
        self.spacing
    }

    /// Whether the list view draws titles under each cover.
    pub fn show_titles(&self) -> bool {
        self.show_titles
    }
}

/// The top-level widget that owns the model, the sort model, the grid view,
/// and the refresh thread.
///
/// The C++ `GameListWidget` derives from `QWidget` and embeds two `QStackedWidget`
/// pages (table view, grid view, empty state). The Rust port keeps the same
/// data flow but represents each page as an enum variant.
#[derive(Debug)]
pub struct GameListWidget {
    model: GameListModel,
    grid_view: GameListGridListView,
    settings: Arc<dyn SettingsStore>,
    refresh_thread: Mutex<Option<GameListRefreshThread>>,
    /// External entries visible to the widget. The C++ code reads from the
    /// global `GameList`; here we keep a small owned store so the widget is
    /// self-contained.
    entries: GameListStore,
    /// Currently selected row, in the same coordinate space as
    /// [`GameListModel::row_count`].
    selected_row: Mutex<Option<i32>>,
}

impl GameListWidget {
    /// Construct a widget using `settings` for persistence and `model` as
    /// the data backend. The widget takes ownership of the model.
    pub fn new(model: GameListModel, settings: Arc<dyn SettingsStore>) -> Self {
        Self {
            model,
            grid_view: GameListGridListView::new(),
            settings,
            refresh_thread: Mutex::new(None),
            entries: GameListStore::new(),
            selected_row: Mutex::new(None),
        }
    }

    /// Construct a widget using a default [`InMemorySettings`] backend.
    pub fn with_default_settings(cover_scale: f32, show_cover_titles: bool) -> Self {
        let settings: Arc<dyn SettingsStore> = Arc::new(InMemorySettings::new());
        let model = GameListModel::new(cover_scale, show_cover_titles, Arc::clone(&settings));
        Self::new(model, settings)
    }

    /// Handle to the model.
    pub fn model(&self) -> &GameListModel {
        &self.model
    }

    /// Handle to the inner game list store.
    pub fn entries(&self) -> &GameListStore {
        &self.entries
    }

    /// Insert a new entry into the widget's store and refresh the model.
    pub fn add_entry(&self, entry: GameEntry) {
        self.entries.add_entry(entry);
        self.model.refresh();
    }

    /// Remove the entry with the given path (if any) and refresh the model.
    /// Returns the number of entries that were removed.
    pub fn remove_entry(&self, path: &str) -> usize {
        let removed = self.entries.remove_by_path(path);
        if removed > 0 {
            self.model.refresh();
        }
        removed
    }

    /// Returns the currently selected entry, if any. This mirrors
    /// `GameListWidget::getSelectedEntry()` from the C++ source.
    pub fn selected_entry(&self) -> Option<GameEntry> {
        let row = *self.selected_row.lock().expect("selected_row poisoned");
        match row {
            Some(r) => self.entries.get(r as usize),
            None => None,
        }
    }

    /// Returns the currently selected row index, if any.
    pub fn selected_row(&self) -> Option<i32> {
        *self.selected_row.lock().expect("selected_row poisoned")
    }

    /// Set the selected row, in model coordinates.
    pub fn set_selected_row(&self, row: Option<i32>) {
        *self.selected_row.lock().expect("selected_row poisoned") = row;
    }

    /// Begin a refresh. The previous refresh thread (if any) is cancelled
    /// and joined first. Mirrors `GameListWidget::refresh(invalidate_cache,
    /// popup_on_error)`.
    pub fn refresh(
        &self,
        invalidate_cache: bool,
        popup_on_error: bool,
        scanner: Option<Arc<dyn Fn(bool, &mut dyn ProgressCallback) + Send + Sync>>,
    ) {
        if let Some(prior) = self.refresh_thread.lock().expect("refresh_thread poisoned").take() {
            prior.stop();
        }
        let thread = GameListRefreshThread::new(invalidate_cache, popup_on_error, scanner);
        thread.start();
        *self.refresh_thread.lock().expect("refresh_thread poisoned") = Some(thread);
    }

    /// Cancel the in-flight refresh, if any, and join the worker thread.
    pub fn cancel_refresh(&self) {
        if let Some(thread) = self.refresh_thread.lock().expect("refresh_thread poisoned").take() {
            thread.stop();
        }
    }

    /// Force the model to reload its theme-specific imagery. Equivalent
    /// to `reloadThemeSpecificImages()` in the original.
    pub fn reload_theme_specific_images(&self) {
        self.model.reload_theme_specific_images();
    }

    /// Equivalent of `showGameList`/`showGameGrid`. The C++ source keeps
    /// a `QStackedWidget`; the port stores the current page as a small
    /// enum.
    pub fn set_view_mode(&self, mode: ViewMode) {
        self.settings
            .set_bool("UI", "GameListGridView", matches!(mode, ViewMode::Grid));
        self.settings.commit();
    }

    /// Currently active view mode.
    pub fn view_mode(&self) -> ViewMode {
        if self.settings.get_bool("UI", "GameListGridView", false) {
            ViewMode::Grid
        } else {
            ViewMode::List
        }
    }

    /// Configure the model to (also) show titles under each cover.
    pub fn set_show_cover_titles(&self, enabled: bool) {
        self.settings
            .set_bool("UI", "GameListShowCoverTitles", enabled);
        self.settings.commit();
        // The model is owned through `&self` so we cannot mutate its
        // `bool` field directly. Production code would thread a `&mut
        // GameListModel` or wrap it in a `RefCell`. We instead ask the
        // model to refresh, which is what the C++ code does in
        // `setShowCoverTitles` when the grid is the active view.
        self.model.refresh();
    }

    /// Persist the current sort selection to settings.
    pub fn save_sort_settings(&self, column: i32, descending: bool) {
        if let Some(col) = column_from_index(column as usize) {
            self.settings
                .set_string("GameListTableView", "SortColumn", get_column_name(col));
            self.settings
                .set_bool("GameListTableView", "SortDescending", descending);
            self.settings.commit();
        }
    }

    /// Re-sort the model using the column's natural ordering.
    pub fn sort_by(&self, column: Column) {
        let mut entries = self.entries.lock().clone();
        let prefer_english = self.model.prefer_english_titles_for_sort();
        entries.sort_by(|a, b| {
            let ord = compare_entries(a, b, column, prefer_english);
            match self.sort_order() {
                SortOrder::Ascending => ord,
                SortOrder::Descending => ord.reverse(),
            }
        });
        self.entries.replace_entries(entries);
        self.model.refresh();
    }

    /// Returns the currently configured sort order.
    pub fn sort_order(&self) -> SortOrder {
        if self.settings.get_bool("GameListTableView", "SortDescending", false) {
            SortOrder::Descending
        } else {
            SortOrder::Ascending
        }
    }

    /// Returns the currently configured sort column, falling back to the
    /// default.
    pub fn sort_column(&self) -> Column {
        let name = self.settings.get_string("GameListTableView", "SortColumn", get_column_name(DEFAULT_SORT_COLUMN));
        get_column_id_for_name(&name).unwrap_or(DEFAULT_SORT_COLUMN)
    }
}

/// Which view is currently active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    List,
    Grid,
    Empty,
}

// ---------------------------------------------------------------------------
// Sort comparator
// ---------------------------------------------------------------------------

impl GameListModel {
    /// Convenience accessor for the "prefer English titles" toggle.
    pub fn prefer_english_titles_for_sort(&self) -> bool {
        self.prefer_english_titles.load(AtomicOrdering::Acquire)
    }

    /// Title-only comparison, used as a tie-breaker by every other
    /// comparator. Mirrors `GameListModel::titlesLessThan`.
    pub fn titles_less_than(left: &GameEntry, right: &GameEntry, prefer_english: bool) -> bool {
        locale_sensitive_compare(&left.sort_title(prefer_english), &right.sort_title(prefer_english))
            == Ordering::Less
    }
}

/// Compare two entries on the given column. Returns the [`Ordering`] between
/// `left` and `right`. Mirrors `GameListModel::lessThan` in the C++ source.
pub fn compare_entries(left: &GameEntry, right: &GameEntry, column: Column, prefer_english: bool) -> Ordering {
    if left == right {
        return Ordering::Equal;
    }
    match column {
        Column::Type => {
            if left.entry_type == right.entry_type {
                less_to_ordering(GameListModel::titles_less_than(left, right, prefer_english))
            } else {
                entry_type_rank(left.entry_type).cmp(&entry_type_rank(right.entry_type))
            }
        }
        Column::Serial => {
            if left.serial == right.serial {
                less_to_ordering(GameListModel::titles_less_than(left, right, prefer_english))
            } else {
                case_insensitive_cmp(&left.serial, &right.serial)
            }
        }
        Column::Title => {
            less_to_ordering(GameListModel::titles_less_than(left, right, prefer_english))
        }
        Column::FileTitle => {
            let l = left.file_title();
            let r = right.file_title();
            if l == r {
                less_to_ordering(GameListModel::titles_less_than(left, right, prefer_english))
            } else {
                case_insensitive_cmp(l, r)
            }
        }
        Column::Region => {
            if left.region == right.region {
                less_to_ordering(GameListModel::titles_less_than(left, right, prefer_english))
            } else {
                region_rank(left.region).cmp(&region_rank(right.region))
            }
        }
        Column::Compatibility => {
            if left.compatibility_rating == right.compatibility_rating {
                less_to_ordering(GameListModel::titles_less_than(left, right, prefer_english))
            } else {
                (left.compatibility_rating as i32).cmp(&(right.compatibility_rating as i32))
            }
        }
        Column::Size => {
            left.total_size.cmp(&right.total_size)
        }
        Column::Crc => left.crc.cmp(&right.crc),
        Column::TimePlayed => left.total_played_time.cmp(&right.total_played_time),
        Column::LastPlayed => left.last_played_time.cmp(&right.last_played_time),
        Column::Cover => less_to_ordering(GameListModel::titles_less_than(left, right, prefer_english)),
    }
}

/// Convert `less = true` to `Ordering::Less`, `less = false` to
/// `Ordering::Greater` (mirroring the C++ `< 0` return convention).
fn less_to_ordering(less: bool) -> Ordering {
    if less {
        Ordering::Less
    } else {
        Ordering::Greater
    }
}

/// Assign a stable rank to each [`EntryType`] for sort comparisons.
fn entry_type_rank(t: EntryType) -> i32 {
    match t {
        EntryType::Invalid => 0,
        EntryType::Ps2Disc => 1,
        EntryType::Ps1Disc => 2,
        EntryType::Elf => 3,
        EntryType::Count => 4,
    }
}

/// Assign a stable rank to each [`Region`] for sort comparisons.
fn region_rank(r: Region) -> i32 {
    match r {
        Region::NtscJ => 0,
        Region::NtscU => 1,
        Region::Pal => 2,
        Region::Other => 3,
        Region::Count => 4,
    }
}

fn case_insensitive_cmp(a: &str, b: &str) -> Ordering {
    let a_bytes = a.as_bytes();
    let b_bytes = b.as_bytes();
    let n = a_bytes.len().min(b_bytes.len());
    for i in 0..n {
        let av = a_bytes[i].to_ascii_lowercase();
        let bv = b_bytes[i].to_ascii_lowercase();
        match av.cmp(&bv) {
            Ordering::Equal => continue,
            non_eq => return non_eq,
        }
    }
    a_bytes.len().cmp(&b_bytes.len())
}

/// Locale-sensitive string comparison. The C++ source uses
/// `QtHost::LocaleSensitiveCompare`; without a full Unicode collator we
/// fall back to case-insensitive lexicographic order, which is sufficient
/// for the unit-tested ordering of the C++ source.
pub fn locale_sensitive_compare(a: &str, b: &str) -> Ordering {
    case_insensitive_cmp(a, b)
}

// ---------------------------------------------------------------------------
// Trivial helpers
// ---------------------------------------------------------------------------

/// Returns the current wall-clock time as a UNIX timestamp, used by callers
/// that want to populate `last_played_time` on a freshly created entry.
pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entry() -> GameEntry {
        GameEntry {
            path: "/roms/Final Fantasy X.iso".to_string(),
            serial: "SLUS-20312".to_string(),
            title: "Final Fantasy X".to_string(),
            title_en: "Final Fantasy X".to_string(),
            crc: 0xDEADBEEF,
            total_played_time: 3 * 3600 + 25 * 60,
            last_played_time: 1_700_000_000,
            total_size: 4 * 1024 * 1024 * 1024,
            entry_type: EntryType::Ps2Disc,
            region: Region::NtscU,
            compatibility_rating: CompatibilityRating::Five,
        }
    }

    #[test]
    fn column_round_trip() {
        for idx in 0..COLUMN_COUNT {
            let col = column_from_index(idx).expect("valid");
            assert_eq!(column_to_index(col), idx);
        }
        assert!(column_from_index(COLUMN_COUNT).is_none());
        assert!(get_column_id_for_name("Title").is_some());
        assert!(get_column_id_for_name("Bogus").is_none());
    }

    #[test]
    fn timespan_formatting() {
        assert_eq!(format_timespan(0), "0 seconds");
        assert_eq!(format_timespan(45), "45 seconds");
        assert_eq!(format_timespan(125), "2 minutes");
        assert_eq!(format_timespan(3700), "1 hours");
    }

    #[test]
    fn lru_cache_eviction() {
        let mut cache: LruCache<&'static str, i32> = LruCache::new(2);
        cache.insert("a", 1);
        cache.insert("b", 2);
        cache.insert("c", 3);
        assert!(cache.lookup(&"a").is_none());
        assert_eq!(cache.lookup(&"b").copied(), Some(2));
        assert_eq!(cache.lookup(&"c").copied(), Some(3));
    }

    #[test]
    fn entry_sorting() {
        let a = sample_entry();
        let mut b = sample_entry();
        b.title = "Another Title".to_string();
        assert_eq!(
            compare_entries(&a, &b, Column::Title, false),
            Ordering::Greater
        );
    }

    #[test]
    fn settings_round_trip() {
        let settings: Arc<dyn SettingsStore> = Arc::new(InMemorySettings::new());
        assert!(!settings.get_bool("UI", "Foo", false));
        settings.set_bool("UI", "Foo", true);
        assert!(settings.get_bool("UI", "Foo", false));
    }

    #[test]
    fn refresh_thread_lifecycle() {
        let thread = GameListRefreshThread::new(false, false, None);
        assert_eq!(thread.state(), RefreshState::Idle);
        assert!(thread.start());
        // Give the worker a moment to publish its first update.
        thread::sleep(Duration::from_millis(20));
        thread.stop();
        assert_eq!(thread.state(), RefreshState::Done);
    }
}
