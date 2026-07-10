//! `RecorderFull` — an idiomatic Rust 2021 translation of PCSX2's input
//! recording subsystem.
//!
//! This module bundles the C++ types that drive PCSX2's input recording
//! feature into a single, self-contained, `std`-only module:
//!
//! - [`PadData`] — a single per-frame controller payload.
//! - [`InputRecordingFile`] — on-disk file I/O for the binary recording
//!   format.
//! - [`InputRecordingControls`] — a small recording / replaying state
//!   machine.
//! - [`InputRecording`] — the top-level orchestrator that ties it together.
//! - [`InputRecordingLogger`] — informational logger for the OSD / console.
//!
//! The translation intentionally omits the C++ version's reach into the
//! rest of the emulator (frame counters, savestates, OSD, ...). Callers
//! drive the recorder explicitly via [`InputRecording`].
//!
//! # On-disk format
//!
//! The recording file is a small packed binary blob:
//!
//! ```text
//! | offset | size | contents
//! |--------|------|--------------------------------
//! |      0 |    1 | u8  file version (must be 1)
//! |      1 |   50 | emulator version (NUL-padded)
//! |     51 |  255 | author (NUL-padded)
//! |    306 |  255 | game name (NUL-padded)
//! |    561 |    4 | u32 total frame count
//! |    565 |    4 | u32 undo count
//! |    569 |    1 | u8  from-save-state flag
//! |    570 |  ... | per-frame controller blocks
//! ```
//!
//! Each controller block is 18 bytes: 2 button bytes, 6 analog/pressure
//! bytes, and 10 reserved bytes. Two controller blocks are written per
//! frame (one per port), for a total of 36 bytes per frame.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// File format constants
// ---------------------------------------------------------------------------

const FILE_VERSION: u8 = 1;
const EMU_VERSION_LEN: usize = 50;
const AUTHOR_LEN: usize = 255;
const GAME_NAME_LEN: usize = 255;
const CONTROLLER_PORTS: usize = 2;
const CONTROLLER_INPUT_BYTES: usize = 18;
const INPUT_BYTES_PER_FRAME: usize = CONTROLLER_PORTS * CONTROLLER_INPUT_BYTES;

const HEADER_VERSION_OFF: u64 = 0;
const HEADER_EMU_OFF: u64 = HEADER_VERSION_OFF + 1;
const HEADER_AUTHOR_OFF: u64 = HEADER_EMU_OFF + EMU_VERSION_LEN as u64;
const HEADER_GAME_OFF: u64 = HEADER_AUTHOR_OFF + AUTHOR_LEN as u64;
const HEADER_END: u64 = HEADER_GAME_OFF + GAME_NAME_LEN as u64;

const SEEK_TOTAL_FRAMES: u64 = HEADER_END;
const SEEK_UNDO_COUNT: u64 = SEEK_TOTAL_FRAMES + 4;
const SEEK_SAVESTATE: u64 = SEEK_UNDO_COUNT + 4;
const DATA_START: u64 = SEEK_SAVESTATE + 1;

// ===========================================================================
// Logger
// ===========================================================================

/// `std`-only stand-in for the C++ `InputRec` namespace.
///
/// The C++ version forwards messages to the host's OSD / console sinks. In
/// this translation we keep a thread-safe history of every line that has
/// been logged; callers can inspect the history with
/// [`messages`](Self::messages).
#[derive(Debug, Default)]
pub struct InputRecordingLogger {
    history: Mutex<Vec<String>>,
}

impl InputRecordingLogger {
    /// Create a new, empty logger.
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot of every line that has been logged so far.
    pub fn messages(&self) -> Vec<String> {
        self.history
            .lock()
            .expect("logger mutex poisoned")
            .clone()
    }

    /// OSD-style log line with a `duration` hint in seconds.
    pub fn log(&self, message: &str, duration: f32) {
        if !message.is_empty() {
            self.push(format!("[REC]: {} (duration={:.1}s)", message, duration));
        }
    }

    /// Console-only log line.
    pub fn console_log(&self, message: &str) {
        if !message.is_empty() {
            self.push(format!("[REC]: {}", message));
        }
    }

    /// Emit several lines at once.
    pub fn console_multi_log<I, S>(&self, lines: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for line in lines {
            self.push(format!("[REC]: {}", line.as_ref()));
        }
    }

    fn push(&self, line: String) {
        self.history
            .lock()
            .expect("logger mutex poisoned")
            .push(line);
    }
}

// ===========================================================================
// PadData
// ===========================================================================

/// Compact representation of a single controller frame.
///
/// The C++ original carried a per-button press/pressure tuple, two analog
/// sticks, and a "compact" button bitmap. The Rust translation collapses
/// that into a smaller surface that the rest of the rewrite expects:
///
/// - `rumble`  — packed rumble intensities.
/// - `buttons` — 16-bit digital button bitmap (group-one in the high byte,
///               group-two in the low byte).
/// - `analog`  — six analog/pressure bytes in the order
///               `[rx, ry, lx, ly, r-press, l-press]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PadData {
    pub rumble: u8,
    pub buttons: u16,
    pub analog: [u8; 6],
}

impl PadData {
    /// Group-one button bits (high byte of [`buttons`](Self::buttons)).
    pub fn group_one(&self) -> u8 {
        (self.buttons >> 8) as u8
    }

    /// Group-two button bits (low byte of [`buttons`](Self::buttons)).
    pub fn group_two(&self) -> u8 {
        (self.buttons & 0xff) as u8
    }

    /// One-line debug log of this frame.
    pub fn log(&self) -> String {
        format!(
            "[PAD buttons=0x{:04x} analog={:?} rumble={}]",
            self.buttons, self.analog, self.rumble
        )
    }
}

// ===========================================================================
// InputRecordingFile
// ===========================================================================

/// On-disk file I/O for the binary recording format.
///
/// Mirrors the C++ `InputRecordingFile`: owns a `File` handle and the
/// metadata that the C++ original stored in `InputRecordingFileHeader`.
/// All file operations return [`io::Result`] so callers can propagate
/// errors up the stack.
pub struct InputRecordingFile {
    filename: String,
    file: Option<File>,
    emulator_version: [u8; EMU_VERSION_LEN],
    author: [u8; AUTHOR_LEN],
    game_name: [u8; GAME_NAME_LEN],
    total_frames: u32,
    undo_count: u32,
    savestate: bool,
}

impl Default for InputRecordingFile {
    fn default() -> Self {
        Self {
            filename: String::new(),
            file: None,
            emulator_version: [0; EMU_VERSION_LEN],
            author: [0; AUTHOR_LEN],
            game_name: [0; GAME_NAME_LEN],
            total_frames: 0,
            undo_count: 0,
            savestate: false,
        }
    }
}

impl InputRecordingFile {
    /// Create a new, empty `InputRecordingFile`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new, empty recording file at `path`.
    pub fn open_new<P: AsRef<Path>>(
        &mut self,
        path: P,
        from_savestate: bool,
    ) -> io::Result<()> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path.as_ref())?;
        self.file = Some(file);
        self.filename = path.as_ref().to_string_lossy().into_owned();
        self.total_frames = 0;
        self.undo_count = 0;
        self.savestate = from_savestate;
        self.emulator_version = [0; EMU_VERSION_LEN];
        self.author = [0; AUTHOR_LEN];
        self.game_name = [0; GAME_NAME_LEN];
        Ok(())
    }

    /// Open an existing recording file at `path` and verify its header.
    pub fn open_existing<P: AsRef<Path>>(&mut self, path: P) -> io::Result<()> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path.as_ref())?;
        self.file = Some(file);
        self.filename = path.as_ref().to_string_lossy().into_owned();
        self.read_header()?;
        self.read_metadata()?;
        Ok(())
    }

    /// Close the file, flushing any buffered data.
    pub fn close(&mut self) -> io::Result<()> {
        if let Some(mut f) = self.file.take() {
            f.flush()?;
        }
        self.filename.clear();
        Ok(())
    }

    /// Whether the file was opened from a savestate.
    pub fn from_save_state(&self) -> bool {
        self.savestate
    }

    /// Total number of frames in the recording.
    pub fn total_frames(&self) -> u32 {
        self.total_frames
    }

    /// Number of re-records (undo actions) accumulated so far.
    pub fn undo_count(&self) -> u32 {
        self.undo_count
    }

    /// Filename of the currently-open file.
    pub fn filename(&self) -> &str {
        &self.filename
    }

    /// Set the emulator-version field in the header.
    pub fn set_emulator_version(&mut self, version: &str) {
        copy_cstr(&mut self.emulator_version, version);
    }

    /// Set the author field in the header.
    pub fn set_author(&mut self, author: &str) {
        copy_cstr(&mut self.author, author);
    }

    /// Set the game-name field in the header.
    pub fn set_game_name(&mut self, name: &str) {
        copy_cstr(&mut self.game_name, name);
    }

    /// Read the emulator-version field from the header.
    pub fn emulator_version(&self) -> &str {
        cstr_trim(&self.emulator_version)
    }

    /// Read the author field from the header.
    pub fn author(&self) -> &str {
        cstr_trim(&self.author)
    }

    /// Read the game-name field from the header.
    pub fn game_name(&self) -> &str {
        cstr_trim(&self.game_name)
    }

    /// Increment the undo (re-record) counter and persist it.
    pub fn increment_undo_count(&mut self) -> io::Result<()> {
        self.undo_count = self.undo_count.saturating_add(1);
        if let Some(f) = self.file.as_mut() {
            f.seek(SeekFrom::Start(SEEK_UNDO_COUNT))?;
            f.write_all(&self.undo_count.to_le_bytes())?;
            f.flush()?;
        }
        Ok(())
    }

    /// Update the total-frames counter and persist it.
    pub fn set_total_frames(&mut self, frames: u32) -> io::Result<()> {
        self.total_frames = frames;
        if let Some(f) = self.file.as_mut() {
            f.seek(SeekFrom::Start(SEEK_TOTAL_FRAMES))?;
            f.write_all(&frames.to_le_bytes())?;
            f.flush()?;
        }
        Ok(())
    }

    /// Persist the header, counters, and savestate flag.
    pub fn write_header(&mut self) -> io::Result<()> {
        let f = self.file.as_mut().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "no file open")
        })?;
        f.seek(SeekFrom::Start(HEADER_VERSION_OFF))?;
        f.write_all(&[FILE_VERSION])?;
        f.write_all(&self.emulator_version)?;
        f.write_all(&self.author)?;
        f.write_all(&self.game_name)?;
        f.seek(SeekFrom::Start(SEEK_TOTAL_FRAMES))?;
        f.write_all(&self.total_frames.to_le_bytes())?;
        f.seek(SeekFrom::Start(SEEK_UNDO_COUNT))?;
        f.write_all(&self.undo_count.to_le_bytes())?;
        f.seek(SeekFrom::Start(SEEK_SAVESTATE))?;
        f.write_all(&[self.savestate as u8])?;
        f.flush()?;
        Ok(())
    }

    /// Write a single controller frame to the file.
    pub fn write_pad_data(
        &mut self,
        frame: u32,
        port: u32,
        _slot: u32,
        data: &PadData,
    ) -> io::Result<()> {
        let f = self.file.as_mut().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "no file open")
        })?;
        let seek = DATA_START
            + (frame as u64) * (INPUT_BYTES_PER_FRAME as u64)
            + (port as u64) * (CONTROLLER_INPUT_BYTES as u64);
        f.seek(SeekFrom::Start(seek))?;
        let mut block = [0u8; CONTROLLER_INPUT_BYTES];
        block[0] = data.group_one();
        block[1] = data.group_two();
        block[2..8].copy_from_slice(&data.analog);
        f.write_all(&block)?;
        f.flush()?;
        Ok(())
    }

    /// Read a single controller frame from the file.
    pub fn read_pad_data(
        &mut self,
        frame: u32,
        port: u32,
        _slot: u32,
    ) -> Option<PadData> {
        let f = self.file.as_mut()?;
        let seek = DATA_START
            + (frame as u64) * (INPUT_BYTES_PER_FRAME as u64)
            + (port as u64) * (CONTROLLER_INPUT_BYTES as u64);
        f.seek(SeekFrom::Start(seek)).ok()?;
        let mut block = [0u8; CONTROLLER_INPUT_BYTES];
        f.read_exact(&mut block).ok()?;
        let mut analog = [0u8; 6];
        analog.copy_from_slice(&block[2..8]);
        Some(PadData {
            rumble: 0,
            buttons: u16::from_be_bytes([block[0], block[1]]),
            analog,
        })
    }

    /// Read a range of controller frames for a single port.
    pub fn bulk_read_pad_data(
        &mut self,
        frame_start: u32,
        frame_end: u32,
        port: u32,
    ) -> Vec<PadData> {
        if frame_end < frame_start || self.file.is_none() {
            return Vec::new();
        }
        (frame_start..frame_end)
            .filter_map(|f| self.read_pad_data(f, port, 0))
            .collect()
    }

    /// Log the file's metadata via the supplied logger.
    pub fn log_metadata(&self, logger: &InputRecordingLogger) {
        logger.console_multi_log([
            format!("File: {}", self.filename),
            format!("PCSX2 Version Used: {}", self.emulator_version()),
            format!("Recording File Version: {}", FILE_VERSION),
            format!("Associated Game Name or ISO Filename: {}", self.game_name()),
            format!("Author: {}", self.author()),
            format!("Total Frames: {}", self.total_frames),
            format!("Undo Count: {}", self.undo_count),
        ]);
    }

    // ---- private helpers ---------------------------------------------------

    fn read_header(&mut self) -> io::Result<()> {
        let f = self.file.as_mut().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "no file open")
        })?;
        f.seek(SeekFrom::Start(HEADER_VERSION_OFF))?;
        let mut version = [0u8; 1];
        f.read_exact(&mut version)?;
        if version[0] != FILE_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsupported recording file version: {}", version[0]),
            ));
        }
        f.seek(SeekFrom::Start(HEADER_EMU_OFF))?;
        f.read_exact(&mut self.emulator_version)?;
        f.seek(SeekFrom::Start(HEADER_AUTHOR_OFF))?;
        f.read_exact(&mut self.author)?;
        f.seek(SeekFrom::Start(HEADER_GAME_OFF))?;
        f.read_exact(&mut self.game_name)?;
        Ok(())
    }

    fn read_metadata(&mut self) -> io::Result<()> {
        let f = self.file.as_mut().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "no file open")
        })?;
        let mut buf = [0u8; 4];
        f.seek(SeekFrom::Start(SEEK_TOTAL_FRAMES))?;
        f.read_exact(&mut buf)?;
        self.total_frames = u32::from_le_bytes(buf);
        f.seek(SeekFrom::Start(SEEK_UNDO_COUNT))?;
        f.read_exact(&mut buf)?;
        self.undo_count = u32::from_le_bytes(buf);
        let mut b = [0u8; 1];
        f.seek(SeekFrom::Start(SEEK_SAVESTATE))?;
        f.read_exact(&mut b)?;
        self.savestate = b[0] != 0;
        Ok(())
    }
}

impl Drop for InputRecordingFile {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

// ===========================================================================
// InputRecordingControls
// ===========================================================================

/// Mode the recorder is currently in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordingMode {
    /// Input is being recorded to a file.
    Recording,
    /// Input is being replayed from a file.
    Replaying,
}

impl Default for RecordingMode {
    fn default() -> Self {
        RecordingMode::Replaying
    }
}

/// Small state machine that tracks whether the recorder is currently
/// recording input or replaying it. Mirrors the C++
/// `InputRecordingControls` class and its deferred-action queue.
#[derive(Debug, Default)]
pub struct InputRecordingControls {
    state: RecordingMode,
    pending: VecDeque<RecordingMode>,
}

impl InputRecordingControls {
    /// Create a new `InputRecordingControls` in the default (Replaying)
    /// state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Switch the recorder into record mode.
    pub fn set_recording(&mut self) {
        self.state = RecordingMode::Recording;
    }

    /// Switch the recorder into replay (playback) mode.
    pub fn set_playing(&mut self) {
        self.state = RecordingMode::Replaying;
    }

    /// Toggle between record and replay.
    pub fn toggle(&mut self) {
        match self.state {
            RecordingMode::Recording => self.set_playing(),
            RecordingMode::Replaying => self.set_recording(),
        }
    }

    /// Queue a state change for the end of the current frame. The C++
    /// original does this when the VM is running; here it is a manual
    /// operation the caller invokes when ready.
    pub fn queue(&mut self, mode: RecordingMode) {
        self.pending.push_back(mode);
    }

    /// Apply any queued state changes.
    pub fn process_queue(&mut self) {
        while let Some(m) = self.pending.pop_front() {
            self.state = m;
        }
    }

    /// Current mode.
    pub fn mode(&self) -> RecordingMode {
        self.state
    }

    /// True if currently recording.
    pub fn is_recording(&self) -> bool {
        self.state == RecordingMode::Recording
    }

    /// True if currently replaying.
    pub fn is_replaying(&self) -> bool {
        self.state == RecordingMode::Replaying
    }
}

// ===========================================================================
// InputRecording
// ===========================================================================

/// Origin of the recording: a cold boot or a savestate resume.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordingType {
    /// The recording starts from power-on (cold boot).
    PowerOn,
    /// The recording resumes from a savestate.
    FromSavestate,
}

impl Default for RecordingType {
    fn default() -> Self {
        RecordingType::PowerOn
    }
}

/// Top-level orchestrator that ties the controls and file I/O together.
///
/// Mirrors the C++ `InputRecording` class. Because the C++ version reaches
/// into a number of emulator-internal subsystems (frame counters, OSD,
/// savestates, ...) that are out of scope for this `std`-only module, the
/// methods that need them have been replaced by methods that take or
/// return primitives explicitly.
pub struct InputRecording {
    /// Recording / replay state machine.
    pub controls: InputRecordingControls,
    /// On-disk recording file.
    pub file: InputRecordingFile,
    rec_type: RecordingType,
    initial_load_complete: bool,
    is_active: bool,
    watching_for_rerecords: bool,
    frame_counter: u32,
    frame_counter_stateless: u32,
    starting_frame: u32,
}

impl Default for InputRecording {
    fn default() -> Self {
        Self {
            controls: InputRecordingControls::new(),
            file: InputRecordingFile::new(),
            rec_type: RecordingType::PowerOn,
            initial_load_complete: false,
            is_active: false,
            watching_for_rerecords: false,
            frame_counter: 0,
            frame_counter_stateless: 0,
            starting_frame: 0,
        }
    }
}

impl InputRecording {
    /// Create a new, inactive recorder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Start a brand-new recording at `path`.
    ///
    /// When `from_savestate` is true the recording is flagged as resuming
    /// from a savestate and `watching_for_rerecords` is enabled so the
    /// first rewind produces a re-record count.
    pub fn start<P: AsRef<Path>>(
        &mut self,
        path: P,
        from_savestate: bool,
        author: &str,
    ) -> io::Result<()> {
        self.file.open_new(path, from_savestate)?;
        self.controls.set_recording();
        if from_savestate {
            self.rec_type = RecordingType::FromSavestate;
            self.is_active = true;
            self.initial_load_complete = true;
            self.watching_for_rerecords = true;
        } else {
            self.starting_frame = 0;
            self.rec_type = RecordingType::PowerOn;
            self.initial_load_complete = false;
            self.is_active = true;
        }
        self.file.set_emulator_version("PCSX2-rust");
        self.file.set_author(author);
        self.file.set_game_name("");
        self.file.write_header()?;
        self.initialize_state();
        Ok(())
    }

    /// Stop the active recording and close the file.
    pub fn stop(&mut self) {
        if !self.is_active {
            return;
        }
        if self.file.close().is_ok() {
            self.is_active = false;
        }
    }

    /// Persist the current state.
    ///
    /// Flushes the header and counters to the open file. If `path` differs
    /// from the currently-open file, the on-disk contents are also copied
    /// to the new path. The recording is left active.
    pub fn save<P: AsRef<Path>>(&mut self, path: P) -> io::Result<()> {
        if !self.is_active {
            return Ok(());
        }
        self.file.write_header()?;
        let new_path = path.as_ref();
        if self.file.filename() != new_path.to_string_lossy().as_ref() {
            std::fs::copy(self.file.filename(), new_path)?;
        }
        Ok(())
    }

    /// Load an existing recording from `path` for replay.
    pub fn load<P: AsRef<Path>>(&mut self, path: P) -> io::Result<()> {
        self.file.open_existing(path)?;
        if self.file.from_save_state() {
            self.rec_type = RecordingType::FromSavestate;
            self.initial_load_complete = false;
            self.is_active = true;
        } else {
            self.starting_frame = 0;
            self.rec_type = RecordingType::PowerOn;
            self.initial_load_complete = false;
            self.is_active = true;
        }
        self.controls.set_playing();
        self.initialize_state();
        Ok(())
    }

    // ---- queries -----------------------------------------------------------

    /// Origin type of the recording.
    pub fn rec_type(&self) -> RecordingType {
        self.rec_type
    }

    /// True while a recording is in progress (recording or replaying).
    pub fn is_active(&self) -> bool {
        self.is_active
    }

    /// True if the recording resumes from a savestate.
    pub fn is_type_savestate(&self) -> bool {
        self.rec_type == RecordingType::FromSavestate
    }

    /// Current frame counter, relative to the recording.
    pub fn frame_counter(&self) -> u32 {
        self.frame_counter
    }

    /// Frame counter that does not get reset by re-records.
    pub fn frame_counter_stateless(&self) -> u32 {
        self.frame_counter_stateless
    }

    /// Internal frame number at which the recording started.
    pub fn starting_frame(&self) -> u32 {
        self.starting_frame
    }

    // ---- frame operations --------------------------------------------------

    /// Advance the per-frame counter, persisting the new total to the file
    /// if recording.
    pub fn inc_frame_counter(&mut self) -> io::Result<()> {
        if !self.is_active {
            return Ok(());
        }
        if self.frame_counter == u32::MAX {
            self.stop();
            return Ok(());
        }
        self.frame_counter += 1;
        if self.controls.is_replaying()
            && self.frame_counter == self.file.total_frames()
        {
            self.watching_for_rerecords = false;
        }
        if self.controls.is_recording() {
            self.frame_counter_stateless += 1;
            self.file.set_total_frames(self.frame_counter)?;
            if self.watching_for_rerecords {
                self.file.increment_undo_count()?;
                self.watching_for_rerecords = false;
            }
        }
        Ok(())
    }

    /// Set the starting frame for a savestate-based recording. Has no
    /// effect on power-on recordings.
    pub fn set_starting_frame(&mut self, frame: u32) {
        if self.rec_type == RecordingType::PowerOn {
            return;
        }
        self.starting_frame = frame;
    }

    /// Adjust the frame counter for a re-record. Returns immediately if
    /// the supplied frame lies outside the recording.
    pub fn adjust_frame_counter_on_rerecord(
        &mut self,
        new_frame: u32,
    ) -> io::Result<()> {
        if new_frame > self.starting_frame + self.file.total_frames() {
            self.frame_counter = self.file.total_frames();
            if self.controls.is_replaying() {
                self.controls.set_recording();
            }
            return Ok(());
        }
        if new_frame < self.starting_frame {
            self.frame_counter = 0;
            if self.controls.is_recording() {
                self.controls.set_playing();
            }
            return Ok(());
        }
        if new_frame == 0 && self.controls.is_recording() {
            self.controls.set_playing();
        }
        self.frame_counter = new_frame.saturating_sub(self.starting_frame);
        if self.frame_counter_stateless > 0 {
            self.frame_counter_stateless -= 1;
        }
        self.file.set_total_frames(self.frame_counter)
    }

    /// Called when the emulator loads a savestate. `current_frame` is the
    /// emulator's internal frame number at the time of the load.
    pub fn handle_loading_savestate(&mut self, current_frame: u32) {
        if self.is_type_savestate() && !self.initial_load_complete {
            self.set_starting_frame(current_frame);
            self.initial_load_complete = true;
        } else {
            let _ = self.adjust_frame_counter_on_rerecord(current_frame);
            self.watching_for_rerecords = true;
        }
    }

    /// Called when the emulator resets.
    pub fn handle_reset(&mut self) {
        if self.initial_load_complete {
            let _ = self.adjust_frame_counter_on_rerecord(0);
        }
        self.initial_load_complete = true;
    }

    /// Save a single frame's controller data while recording.
    pub fn save_controller_data(
        &mut self,
        port: u32,
        slot: u32,
        data: &PadData,
    ) -> io::Result<()> {
        self.file
            .write_pad_data(self.frame_counter, port, slot, data)
    }

    /// Update the controller's input from the recorded file while
    /// replaying. Returns `None` if the file is closed or the seek/read
    /// fails.
    pub fn update_controller_data(
        &mut self,
        port: u32,
        slot: u32,
    ) -> Option<PadData> {
        self.file
            .read_pad_data(self.frame_counter, port, slot)
    }

    fn initialize_state(&mut self) {
        self.frame_counter = 0;
        self.watching_for_rerecords = false;
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Copy `src` into the fixed-size C-string `dst`, NUL-padding the tail.
fn copy_cstr(dst: &mut [u8], src: &str) {
    for b in dst.iter_mut() {
        *b = 0;
    }
    let bytes = src.as_bytes();
    let n = bytes.len().min(dst.len().saturating_sub(1));
    dst[..n].copy_from_slice(&bytes[..n]);
}

/// Borrow `buf` as a `&str` up to the first NUL byte.
fn cstr_trim(buf: &[u8]) -> &str {
    let n = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    std::str::from_utf8(&buf[..n]).unwrap_or("")
}
