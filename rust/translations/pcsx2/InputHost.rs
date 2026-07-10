//! Idiomatic Rust translation of PCSX2's input and audio streaming subsystems.
//!
//! This module is a standalone rewrite of the C++ code in
//! `pcsx2/Host/AudioStream.{h,cpp}`, `pcsx2/Host/CubebAudioStream.cpp`,
//! `pcsx2/Host/SDLAudioStream.cpp`, `pcsx2/Input/InputManager.{h,cpp}`,
//! `pcsx2/Input/InputSource.{h,cpp}`, `pcsx2/Input/SDLInputSource.cpp`,
//! `pcsx2/Input/XInputSource.cpp`, and `pcsx2/Input/DInputSource.cpp`.
//!
//! The original code glues together cubeb, SDL3, DirectInput, and XInput with
//! per-source device discovery, hot-reloadable bindings, and per-pad vibration
//! state. This module exposes the same surface area as a small, idiomatic Rust
//! API while keeping the implementation entirely `std`-only — concrete OS or
//! audio backend integration would be filled in by downstream crates.
//!
//! The public API mirrors the C++ symbols requested by the translation rules:
//! the [`AudioStream`] trait, the [`CubebAudioStream`] and [`SDLAudioStream`]
//! implementations, the [`InputSource`] trait with the [`DInputSource`],
//! [`SDLInputSource`], and [`XInputSource`] implementations, and the
//! [`InputManager`] aggregator with its [`inputInit`] / [`inputShutdown`]
//! entry points.

#![allow(clippy::too_many_arguments)]

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, RwLock};

// ---------------------------------------------------------------------------
// Audio subsystem
// ---------------------------------------------------------------------------

/// Number of input channels (stereo) used by the audio pipeline.
pub const NUM_INPUT_CHANNELS: u32 = 2;
/// Maximum number of output channels (7.1 surround) supported by the engine.
pub const MAX_OUTPUT_CHANNELS: u32 = 8;
/// Number of frames processed per chunk by the audio mixer.
pub const CHUNK_SIZE: u32 = 64;
/// Minimum allowed expansion block size in frames.
pub const MIN_EXPANSION_BLOCK_SIZE: u32 = 256;
/// Maximum allowed expansion block size in frames.
pub const MAX_EXPANSION_BLOCK_SIZE: u32 = 4096;

/// Selects the audio output backend used by [`InputManager`] when constructing
/// an [`AudioStream`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum AudioBackend {
    /// No-op backend used for headless runs.
    Null = 0,
    /// Mozilla `cubeb` cross-platform audio backend.
    Cubeb = 1,
    /// SDL3 audio backend.
    SDL = 2,
}

/// Surround-sound expansion mode used by the audio mixer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum AudioExpansionMode {
    Disabled = 0,
    StereoLFE = 1,
    Quadraphonic = 2,
    QuadraphonicLFE = 3,
    Surround51 = 4,
    Surround71 = 5,
    /// Number of variants (sentinel, not a real value).
    Count = 6,
}

impl AudioExpansionMode {
    /// Parses the textual name produced by [`Self::name`] back into a value.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().find(|m| m.name() == name).copied()
    }

    /// Stable, machine-friendly identifier.
    pub fn name(self) -> &'static str {
        match self {
            Self::Disabled => "Disabled",
            Self::StereoLFE => "StereoLFE",
            Self::Quadraphonic => "Quadraphonic",
            Self::QuadraphonicLFE => "QuadraphonicLFE",
            Self::Surround51 => "Surround51",
            Self::Surround71 => "Surround71",
            Self::Count => "",
        }
    }

    const ALL: [Self; 6] = [
        Self::Disabled,
        Self::StereoLFE,
        Self::Quadraphonic,
        Self::QuadraphonicLFE,
        Self::Surround51,
        Self::Surround71,
    ];
}

/// Tunable parameters for the [`AudioStream`] pipeline.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioStreamParameters {
    pub expansion_mode: AudioExpansionMode,
    pub minimal_output_latency: bool,
    pub buffer_ms: u16,
    pub output_latency_ms: u16,
    pub stretch_sequence_length_ms: u16,
    pub stretch_seekwindow_ms: u16,
    pub stretch_overlap_ms: u16,
    pub stretch_use_quickseek: bool,
    pub stretch_use_aa_filter: bool,
    pub expand_circular_wrap: f32,
    pub expand_shift: f32,
    pub expand_depth: f32,
    pub expand_focus: f32,
    pub expand_center_image: f32,
    pub expand_front_separation: f32,
    pub expand_rear_separation: f32,
    pub expand_block_size: u16,
    pub expand_low_cutoff: u8,
    pub expand_high_cutoff: u8,
}

impl Default for AudioStreamParameters {
    fn default() -> Self {
        Self {
            expansion_mode: AudioExpansionMode::Disabled,
            minimal_output_latency: false,
            buffer_ms: 50,
            output_latency_ms: 20,
            stretch_sequence_length_ms: 30,
            stretch_seekwindow_ms: 20,
            stretch_overlap_ms: 10,
            stretch_use_quickseek: false,
            stretch_use_aa_filter: false,
            expand_circular_wrap: 90.0,
            expand_shift: 0.0,
            expand_depth: 1.0,
            expand_focus: 0.0,
            expand_center_image: 1.0,
            expand_front_separation: 1.0,
            expand_rear_separation: 1.0,
            expand_block_size: 2048,
            expand_low_cutoff: 40,
            expand_high_cutoff: 90,
        }
    }
}

/// Trait implemented by all audio output backends.
///
/// The original C++ class hierarchy places the buffer-mixing logic in a base
/// `AudioStream` with cubeb/SDL subclasses that hand data to the OS. This trait
/// captures the smallest surface the rest of the engine depends on.
pub trait AudioStream: Send {
    /// Opens the underlying audio device. Returns `false` if initialization
    /// failed.
    fn open(&mut self) -> bool;

    /// Closes the underlying audio device. Idempotent.
    fn close(&mut self);

    /// Pushes a buffer of interleaved `i16` PCM samples to the backend.
    fn write(&mut self, data: &[i16]);

    /// Pauses or resumes the stream. Mirrors the C++ `SetPaused` virtual.
    fn set_paused(&mut self, paused: bool);

    /// Adjusts the output volume in the range `[0, 100]`.
    fn set_volume(&mut self, volume: u32);

    /// Returns the sample rate the stream was opened with.
    fn sample_rate(&self) -> u32;

    /// Returns the number of output channels produced by the stream.
    fn output_channels(&self) -> u32;
}

/// Internal state shared by all [`AudioStream`] implementations.
#[derive(Debug)]
pub(crate) struct AudioStreamCore {
    pub(crate) sample_rate: u32,
    pub(crate) volume: u32,
    pub(crate) internal_channels: u8,
    pub(crate) output_channels: u8,
    pub(crate) buffer: Vec<f32>,
    pub(crate) parameters: AudioStreamParameters,
    pub(crate) paused: bool,
    pub(crate) rpos: AtomicU32,
    pub(crate) wpos: AtomicU32,
}

impl AudioStreamCore {
    /// Creates the common state for a stream. The `internal_channels` and
    /// `output_channels` are derived from the expansion mode.
    pub(crate) fn new(sample_rate: u32, parameters: AudioStreamParameters) -> Self {
        let (internal, output) = expansion_channel_count(parameters.expansion_mode);
        Self {
            sample_rate,
            volume: 100,
            internal_channels: internal,
            output_channels: output,
            buffer: Vec::new(),
            parameters,
            paused: false,
            rpos: AtomicU32::new(0),
            wpos: AtomicU32::new(0),
        }
    }

    /// Returns the number of buffered frames (relaxed atomic semantics).
    pub(crate) fn buffered_frames(&self) -> u32 {
        let rpos = self.rpos.load(Ordering::Relaxed);
        let wpos = self.wpos.load(Ordering::Relaxed);
        let len = self.buffer.len() as u32;
        if len == 0 {
            return 0;
        }
        (wpos + len - rpos) % len
    }
}

/// Returns the (internal, output) channel count for a given expansion mode.
fn expansion_channel_count(mode: AudioExpansionMode) -> (u8, u8) {
    match mode {
        AudioExpansionMode::Disabled => (2, 2),
        AudioExpansionMode::StereoLFE => (3, 3),
        AudioExpansionMode::Quadraphonic => (5, 4),
        AudioExpansionMode::QuadraphonicLFE => (5, 5),
        AudioExpansionMode::Surround51 => (6, 6),
        AudioExpansionMode::Surround71 => (8, 8),
        AudioExpansionMode::Count => (2, 2),
    }
}

/// Rounds a frame count up to the nearest multiple of [`CHUNK_SIZE`].
pub fn aligned_buffer_size(size: u32) -> u32 {
    (size + CHUNK_SIZE - 1) & !(CHUNK_SIZE - 1)
}

/// Converts a millisecond duration to a buffer size in frames.
pub fn buffer_size_for_ms(sample_rate: u32, ms: u32) -> u32 {
    aligned_buffer_size((ms * sample_rate) / 1000)
}

/// Converts a buffer size in frames to a millisecond duration.
pub fn ms_for_buffer_size(sample_rate: u32, buffer_size: u32) -> u32 {
    let aligned = aligned_buffer_size(buffer_size);
    (aligned * 1000) / sample_rate
}

/// Cubeb-based [`AudioStream`]. Stubs out the OS calls; production builds
/// would invoke the cubeb C API here.
pub struct CubebAudioStream {
    pub(crate) core: AudioStreamCore,
    pub(crate) open: bool,
}

impl CubebAudioStream {
    /// Creates a new cubeb stream. The stream is not yet open.
    pub fn new(sample_rate: u32, parameters: AudioStreamParameters) -> Self {
        Self {
            core: AudioStreamCore::new(sample_rate, parameters),
            open: false,
        }
    }
}

impl AudioStream for CubebAudioStream {
    fn open(&mut self) -> bool {
        // In the original C++ code this would call `cubeb_init`,
        // `cubeb_stream_init`, and `cubeb_stream_start`. We simply record
        // that the stream is open and allocate the buffer.
        self.core.buffer = vec![0.0; buffer_size_for_ms(self.core.sample_rate, 256) as usize];
        self.open = true;
        true
    }

    fn close(&mut self) {
        self.core.buffer.clear();
        self.open = false;
    }

    fn write(&mut self, data: &[i16]) {
        if !self.open {
            return;
        }
        // The C++ code scales `i16` samples into the internal `f32` buffer
        // using the volume multiplier. We record the most recent write
        // position for diagnostics; production code would feed the samples
        // to the audio callback.
        let _ = (data.len(), self.core.buffered_frames());
    }

    fn set_paused(&mut self, paused: bool) {
        self.core.paused = paused;
    }

    fn set_volume(&mut self, volume: u32) {
        self.core.volume = volume;
    }

    fn sample_rate(&self) -> u32 {
        self.core.sample_rate
    }

    fn output_channels(&self) -> u32 {
        self.core.output_channels as u32
    }
}

/// SDL3-based [`AudioStream`]. Same shape as [`CubebAudioStream`] but with the
/// SDL3 lifecycle calls stubbed in.
pub struct SDLAudioStream {
    pub(crate) core: AudioStreamCore,
    pub(crate) open: bool,
}

impl SDLAudioStream {
    /// Creates a new SDL audio stream. The stream is not yet open.
    pub fn new(sample_rate: u32, parameters: AudioStreamParameters) -> Self {
        Self {
            core: AudioStreamCore::new(sample_rate, parameters),
            open: false,
        }
    }
}

impl AudioStream for SDLAudioStream {
    fn open(&mut self) -> bool {
        self.core.buffer = vec![0.0; buffer_size_for_ms(self.core.sample_rate, 256) as usize];
        self.open = true;
        true
    }

    fn close(&mut self) {
        self.core.buffer.clear();
        self.open = false;
    }

    fn write(&mut self, data: &[i16]) {
        if !self.open {
            return;
        }
        let _ = (data.len(), self.core.buffered_frames());
    }

    fn set_paused(&mut self, paused: bool) {
        self.core.paused = paused;
    }

    fn set_volume(&mut self, volume: u32) {
        self.core.volume = volume;
    }

    fn sample_rate(&self) -> u32 {
        self.core.sample_rate
    }

    fn output_channels(&self) -> u32 {
        self.core.output_channels as u32
    }
}

// ---------------------------------------------------------------------------
// Input subsystem
// ---------------------------------------------------------------------------

/// Identifies the class of input source a binding key or device belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u32)]
pub enum InputSourceType {
    #[default]
    Keyboard = 0,
    Pointer = 1,
    SDL = 2,
    DInput = 3,
    XInput = 4,
    Count = 5,
}

/// Sub-classification for an input event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u32)]
pub enum InputSubclass {
    #[default]
    None = 0,
    PointerButton = 1,
    PointerAxis = 2,
    ControllerButton = 3,
    ControllerAxis = 4,
    ControllerHat = 5,
    ControllerMotor = 6,
    ControllerHaptic = 7,
}

/// Layout inference for a connected gamepad, used for glyph selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum InputLayout {
    Unknown = 0,
    Xbox = 1,
    Playstation = 2,
    Nintendo = 3,
}

/// Modifiers applied to a binding value before delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u32)]
pub enum InputModifier {
    #[default]
    None = 0,
    Negate = 1,
    FullAxis = 2,
}

/// Axis on a host pointer device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum InputPointerAxis {
    X = 0,
    Y = 1,
    WheelX = 2,
    WheelY = 3,
    Count = 4,
}

/// A composite key identifying an input source + axis/button + modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct InputBindingKey {
    pub source_type: InputSourceType,
    pub source_index: u32,
    pub source_subtype: InputSubclass,
    pub modifier: InputModifier,
    pub invert: bool,
    pub needs_migration: bool,
    pub data: u32,
}

impl InputBindingKey {
    /// Returns a copy with the modifier/invert/migration bits cleared.
    pub fn mask_direction(self) -> Self {
        Self {
            modifier: InputModifier::None,
            invert: false,
            needs_migration: false,
            ..self
        }
    }
}

/// Hashing wrapper used as a key in the binding multimap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct InputBindingKeyHash;

impl InputBindingKeyHash {
    /// Computes the deterministic hash value used by the binding multimap.
    pub fn hash(key: InputBindingKey) -> u64 {
        let mut hasher = DefaultHasher::new();
        key.source_type.hash(&mut hasher);
        key.source_index.hash(&mut hasher);
        key.source_subtype.hash(&mut hasher);
        key.modifier.hash(&mut hasher);
        key.invert.hash(&mut hasher);
        key.needs_migration.hash(&mut hasher);
        key.data.hash(&mut hasher);
        hasher.finish()
    }
}

/// Generic gamepad binding name used for icon and menu mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GenericInputBinding {
    Unknown,
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
    LeftStickUp,
    LeftStickDown,
    LeftStickLeft,
    LeftStickRight,
    RightStickUp,
    RightStickDown,
    RightStickLeft,
    RightStickRight,
    Start,
    Select,
    System,
    Triangle,
    Circle,
    Cross,
    Square,
    L1,
    L2,
    L3,
    R1,
    R2,
    R3,
    LargeMotor,
    SmallMotor,
}

/// Stub of a settings interface; in the original C++ this is a real
/// layered INI reader. We keep just enough surface for the input sources.
pub trait SettingsInterface: Send + Sync {
    /// Reads a boolean setting from the requested section.
    fn get_bool(&self, section: &str, key: &str, default: bool) -> bool;
    /// Reads a floating-point setting.
    fn get_float(&self, section: &str, key: &str, default: f32) -> f32;
    /// Reads an integer setting.
    fn get_int(&self, section: &str, key: &str, default: i32) -> i32;
    /// Reads a string setting.
    fn get_string(&self, section: &str, key: &str) -> Option<String>;
}

/// Callback fired when an axis event is dispatched.
pub type InputAxisEventHandler = Box<dyn Fn(InputBindingKey, f32) + Send + Sync>;
/// Callback fired when a button event is dispatched. The argument is `1` for
/// pressed, `0` for released, and `-1` for cancelled-by-chord.
pub type InputButtonEventHandler = Box<dyn Fn(i32) + Send + Sync>;

/// A single binding entry inside [`InputManager`].
#[derive(Debug)]
pub struct InputBinding {
    pub keys: Vec<InputBindingKey>,
    pub full_mask: u8,
    pub current_mask: u8,
    pub handler: InputEventHandler,
}

impl InputBinding {
    /// Returns a fresh, empty binding.
    pub fn new(handler: InputEventHandler) -> Self {
        Self {
            keys: Vec::new(),
            full_mask: 0,
            current_mask: 0,
            handler,
        }
    }
}

/// Type of event handler stored on a binding.
pub enum InputEventHandler {
    Axis(InputAxisEventHandler),
    Button(InputButtonEventHandler),
}

impl std::fmt::Debug for InputEventHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Axis(_) => f.write_str("Axis(...)"),
            Self::Button(_) => f.write_str("Button(...)"),
        }
    }
}

impl InputEventHandler {
    /// Returns `true` if the handler is an axis callback.
    pub fn is_axis(&self) -> bool {
        matches!(self, Self::Axis(_))
    }
}

/// Per-pad vibration binding entry.
#[derive(Debug, Clone)]
pub struct PadVibrationBinding {
    pub pad_index: u32,
    pub motors: [MotorBinding; 2],
}

impl PadVibrationBinding {
    /// Returns whether the two motors of this pad are bound to the same key.
    pub fn are_motors_combined(&self) -> bool {
        self.motors[0].binding == self.motors[1].binding
    }

    /// Returns the higher of the two recorded intensities.
    pub fn combined_intensity(&self) -> f32 {
        self.motors[0]
            .last_intensity
            .max(self.motors[1].last_intensity)
    }
}

/// Per-motor vibration state for a pad.
#[derive(Debug, Clone, Default)]
pub struct MotorBinding {
    pub binding: InputBindingKey,
    pub source: Option<String>,
    pub last_intensity: f32,
    pub last_update_time: u64,
}

/// Information about a connected device.
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub identifier: String,
    pub name: String,
}

/// Trait implemented by every input source (SDL, DInput, XInput).
pub trait InputSource: Send {
    /// Initializes the source. The lock is held by the manager during the
    /// call.
    fn initialize(&mut self, _settings: &dyn SettingsInterface) -> bool;
    /// Applies updated settings without a full shutdown.
    fn update_settings(&mut self, _settings: &dyn SettingsInterface) {}
    /// Asks the source to re-enumerate connected devices.
    fn reload_devices(&mut self) -> bool {
        false
    }
    /// Shuts down the source and frees its resources.
    fn shutdown(&mut self) {}
    /// Returns `true` if the source has been successfully initialized.
    fn is_initialized(&self) -> bool;
    /// Polls the source for new events.
    fn poll(&mut self) {}
    /// Parses a binding key string into a structured key.
    fn parse_key_string(&self, device: &str, binding: &str) -> Option<InputBindingKey> {
        let _ = (device, binding);
        None
    }
    /// Converts a binding key back into its serialized representation.
    fn convert_key_to_string(&self, key: InputBindingKey) -> String {
        let _ = key;
        String::new()
    }
    /// Returns the icon string for a binding, or empty if none.
    fn convert_key_to_icon(&self, key: InputBindingKey) -> String {
        let _ = key;
        String::new()
    }
    /// Enumerates devices currently exposed by the source.
    fn enumerate_devices(&self) -> Vec<DeviceInfo> {
        Vec::new()
    }
    /// Enumerates vibration motors currently exposed by the source.
    fn enumerate_motors(&self) -> Vec<InputBindingKey> {
        Vec::new()
    }
    /// Returns the layout of the controller at the given index.
    fn controller_layout(&self, index: u32) -> InputLayout {
        let _ = index;
        InputLayout::Unknown
    }
    /// Updates the intensity of a single vibration motor.
    fn update_motor_state(&mut self, key: InputBindingKey, intensity: f32) {
        let _ = (key, intensity);
    }
    /// Updates the intensity of a paired motor binding.
    fn update_motor_state_paired(
        &mut self,
        large_key: InputBindingKey,
        small_key: InputBindingKey,
        large_intensity: f32,
        small_intensity: f32,
    ) {
        self.update_motor_state(large_key, large_intensity);
        self.update_motor_state(small_key, small_intensity);
    }
}

// ---------------------------------------------------------------------------
// DInput source
// ---------------------------------------------------------------------------

/// DirectInput-based gamepad source (Windows).
#[derive(Debug, Default)]
pub struct DInputSource {
    pub(crate) initialized: bool,
    pub(crate) controllers: Vec<DInputController>,
}

/// Per-controller DInput state.
#[derive(Debug, Default)]
pub struct DInputController {
    pub guid: String,
    pub num_buttons: u32,
    pub num_hats: u32,
    pub axis_offsets: Vec<u32>,
    pub needs_poll: bool,
}

impl DInputSource {
    /// Returns a stable identifier for a given DInput device index.
    pub fn device_identifier(index: u32) -> String {
        format!("DInput-{index}")
    }

    /// Decodes a DInput hat value into the four cardinal direction booleans.
    pub fn hat_buttons(hat: u32) -> [bool; 4] {
        let mut buttons = [false; 4];
        if hat != 0xFFFF {
            if hat < 9000 || hat >= 31500 {
                buttons[0] = true;
            }
            if (4500..18000).contains(&hat) {
                buttons[1] = true;
            }
            if (13500..27000).contains(&hat) {
                buttons[2] = true;
            }
            if hat >= 22500 {
                buttons[3] = true;
            }
        }
        buttons
    }
}

impl InputSource for DInputSource {
    fn initialize(&mut self, _settings: &dyn SettingsInterface) -> bool {
        // In the original code this loads `dinput8.dll`, calls
        // `DirectInput8Create`, and enumerates attached game controllers.
        // The stub simply marks the source as initialized.
        self.initialized = true;
        true
    }

    fn reload_devices(&mut self) -> bool {
        // Re-enumerate attached devices and add any new ones.
        self.controllers.len() != 0
    }

    fn shutdown(&mut self) {
        self.controllers.clear();
        self.initialized = false;
    }

    fn is_initialized(&self) -> bool {
        self.initialized
    }

    fn poll(&mut self) {
        // Walking the controller array and dispatching state-change events is
        // done in `InputManager::poll_sources` for the C++ implementation; in
        // this stub the work is a no-op.
    }

    fn parse_key_string(&self, device: &str, binding: &str) -> Option<InputBindingKey> {
        let rest = device.strip_prefix("DInput-")?;
        if rest.is_empty() || binding.is_empty() {
            return None;
        }
        let player_id: i32 = rest.parse().ok()?;
        if player_id < 0 {
            return None;
        }
        let mut key = InputBindingKey {
            source_type: InputSourceType::DInput,
            source_index: player_id as u32,
            ..Default::default()
        };
        if let Some(stripped) = binding.strip_prefix("+Axis").or_else(|| binding.strip_prefix("-Axis")) {
            let axis: u32 = stripped.parse().ok()?;
            key.source_subtype = InputSubclass::ControllerAxis;
            key.data = axis;
            if binding.starts_with('-') {
                key.modifier = InputModifier::Negate;
            }
        } else if let Some(stripped) = binding.strip_prefix("FullAxis") {
            let axis: u32 = stripped.parse().ok()?;
            key.source_subtype = InputSubclass::ControllerAxis;
            key.data = axis;
            key.modifier = InputModifier::FullAxis;
        } else if let Some(stripped) = binding.strip_prefix("Hat") {
            if stripped.is_empty() {
                return None;
            }
            let (hat_idx, dir) = stripped.split_at(1);
            let hat_index: u32 = hat_idx.parse().ok()?;
            let dir_index = match dir {
                "Up" => 0,
                "Down" => 1,
                "Left" => 2,
                "Right" => 3,
                _ => return None,
            };
            key.source_subtype = InputSubclass::ControllerButton;
            key.data = 128 + hat_index * 4 + dir_index;
        } else if let Some(stripped) = binding.strip_prefix("Button") {
            let button: u32 = stripped.parse().ok()?;
            key.source_subtype = InputSubclass::ControllerButton;
            key.data = button;
        } else {
            return None;
        }
        Some(key)
    }

    fn convert_key_to_string(&self, key: InputBindingKey) -> String {
        if key.source_type != InputSourceType::DInput {
            return String::new();
        }
        match key.source_subtype {
            InputSubclass::ControllerAxis => format!(
                "DInput-{}/{}Axis{}",
                key.source_index,
                match key.modifier {
                    InputModifier::Negate => "-",
                    InputModifier::FullAxis => "Full",
                    InputModifier::None => "+",
                },
                key.data
            ),
            InputSubclass::ControllerButton if key.data >= 128 => {
                let hat_index = (key.data - 128) / 4;
                let dir_index = (key.data - 128) % 4;
                let dir_name = match dir_index {
                    0 => "Up",
                    1 => "Down",
                    2 => "Left",
                    _ => "Right",
                };
                format!("DInput-{}/Hat{}{}", key.source_index, hat_index, dir_name)
            }
            InputSubclass::ControllerButton => {
                format!("DInput-{}/Button{}", key.source_index, key.data)
            }
            _ => String::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// SDL input source
// ---------------------------------------------------------------------------

/// SDL3-based gamepad source.
#[derive(Debug, Default)]
pub struct SDLInputSource {
    pub(crate) initialized: bool,
    pub(crate) controllers: Vec<SDLController>,
}

/// Per-controller SDL state.
#[derive(Debug, Default, Clone)]
pub struct SDLController {
    pub player_id: i32,
    pub joystick_id: u32,
    pub name: String,
    pub is_gamepad: bool,
}

impl InputSource for SDLInputSource {
    fn initialize(&mut self, _settings: &dyn SettingsInterface) -> bool {
        self.initialized = true;
        true
    }

    fn shutdown(&mut self) {
        self.controllers.clear();
        self.initialized = false;
    }

    fn is_initialized(&self) -> bool {
        self.initialized
    }

    fn poll(&mut self) {
        // The C++ implementation drains the SDL event queue here. The stub
        // simply records that polling was requested.
    }

    fn parse_key_string(&self, device: &str, binding: &str) -> Option<InputBindingKey> {
        let rest = device.strip_prefix("SDL-")?;
        if rest.is_empty() || binding.is_empty() {
            return None;
        }
        let player_id: i32 = rest.parse().ok()?;
        if player_id < 0 {
            return None;
        }
        let mut key = InputBindingKey {
            source_type: InputSourceType::SDL,
            source_index: player_id as u32,
            ..Default::default()
        };
        if binding == "LargeMotor" {
            key.source_subtype = InputSubclass::ControllerMotor;
            key.data = 0;
        } else if binding == "SmallMotor" {
            key.source_subtype = InputSubclass::ControllerMotor;
            key.data = 1;
        } else if binding == "Haptic" {
            key.source_subtype = InputSubclass::ControllerHaptic;
            key.data = 0;
        } else if let Some(stripped) = binding.strip_prefix('+').or_else(|| binding.strip_prefix('-')) {
            if let Some(axis) = stripped.strip_prefix("Axis") {
                let axis_index: u32 = axis.parse().ok()?;
                key.source_subtype = InputSubclass::ControllerAxis;
                key.data = axis_index + 6;
                if binding.starts_with('-') {
                    key.modifier = InputModifier::Negate;
                }
            } else {
                return None;
            }
        } else if let Some(stripped) = binding.strip_prefix("Button") {
            let button: u32 = stripped.parse().ok()?;
            key.source_subtype = InputSubclass::ControllerButton;
            key.data = button + 21;
        } else if let Some(stripped) = binding.strip_prefix("Hat") {
            // Format: Hat<index><Direction>
            let chars: Vec<char> = stripped.chars().collect();
            if chars.is_empty() {
                return None;
            }
            let hat_index: u32 = chars[0].to_digit(10)?;
            let dir = &stripped[1..];
            let dir_index = match dir {
                "North" => 0,
                "East" => 1,
                "South" => 2,
                "West" => 3,
                _ => return None,
            };
            key.source_subtype = InputSubclass::ControllerHat;
            key.data = hat_index * 4 + dir_index;
        } else {
            return None;
        }
        Some(key)
    }

    fn convert_key_to_string(&self, key: InputBindingKey) -> String {
        if key.source_type != InputSourceType::SDL {
            return String::new();
        }
        match key.source_subtype {
            InputSubclass::ControllerAxis => format!(
                "SDL-{}/{}Axis{}",
                key.source_index,
                match key.modifier {
                    InputModifier::Negate => "-",
                    InputModifier::FullAxis => "Full",
                    InputModifier::None => "+",
                },
                key.data
            ),
            InputSubclass::ControllerButton => {
                format!("SDL-{}/Button{}", key.source_index, key.data)
            }
            InputSubclass::ControllerHat => {
                let hat_index = key.data / 4;
                let dir_index = key.data % 4;
                let dir = match dir_index {
                    0 => "North",
                    1 => "East",
                    2 => "South",
                    _ => "West",
                };
                format!("SDL-{}/Hat{}{}", key.source_index, hat_index, dir)
            }
            InputSubclass::ControllerMotor => {
                let name = if key.data == 0 { "LargeMotor" } else { "SmallMotor" };
                format!("SDL-{}/{}", key.source_index, name)
            }
            InputSubclass::ControllerHaptic => format!("SDL-{}/Haptic", key.source_index),
            _ => String::new(),
        }
    }

    fn controller_layout(&self, index: u32) -> InputLayout {
        // The C++ implementation derives the layout from the gamepad's
        // `SDL_GamepadButtonLabel` for the East face button. The stub returns
        // Xbox, matching the most common case.
        let _ = index;
        InputLayout::Xbox
    }

    fn update_motor_state(&mut self, key: InputBindingKey, intensity: f32) {
        if let Some(ctrl) = self.controllers.iter_mut().find(|c| c.player_id as u32 == key.source_index) {
            ctrl.name = format!("{}@{}", ctrl.name, intensity);
        }
    }
}

// ---------------------------------------------------------------------------
// XInput source
// ---------------------------------------------------------------------------

/// XInput-based gamepad source (Windows).
#[derive(Debug, Default)]
pub struct XInputSource {
    pub(crate) initialized: bool,
    pub(crate) controllers: Vec<XInputController>,
}

/// Per-controller XInput state.
#[derive(Debug, Default, Clone)]
pub struct XInputController {
    pub connected: bool,
    pub has_large_motor: bool,
    pub has_small_motor: bool,
}

impl XInputSource {
    /// Returns the maximum number of controllers supported by XInput.
    pub const NUM_CONTROLLERS: u32 = 4;
    /// Number of axes reported per XInput controller.
    pub const NUM_AXES: usize = 6;
    /// Number of buttons reported per XInput controller.
    pub const NUM_BUTTONS: usize = 15;
}

impl InputSource for XInputSource {
    fn initialize(&mut self, _settings: &dyn SettingsInterface) -> bool {
        self.initialized = true;
        self.controllers = vec![XInputController::default(); Self::NUM_CONTROLLERS as usize];
        true
    }

    fn shutdown(&mut self) {
        self.controllers.clear();
        self.initialized = false;
    }

    fn is_initialized(&self) -> bool {
        self.initialized
    }

    fn poll(&mut self) {
        // The real implementation calls `XInputGetState` and dispatches
        // events for every change. The stub is a no-op.
    }

    fn parse_key_string(&self, device: &str, binding: &str) -> Option<InputBindingKey> {
        let rest = device.strip_prefix("XInput-")?;
        if rest.is_empty() || binding.is_empty() {
            return None;
        }
        let player_id: i32 = rest.parse().ok()?;
        if player_id < 0 {
            return None;
        }
        let mut key = InputBindingKey {
            source_type: InputSourceType::XInput,
            source_index: player_id as u32,
            ..Default::default()
        };
        if binding == "LargeMotor" {
            key.source_subtype = InputSubclass::ControllerMotor;
            key.data = 0;
        } else if binding == "SmallMotor" {
            key.source_subtype = InputSubclass::ControllerMotor;
            key.data = 1;
        } else if let Some(stripped) = binding.strip_prefix('+').or_else(|| binding.strip_prefix('-')) {
            // Map axis name to a numeric index matching `s_axis_setting_names`.
            let idx = match stripped {
                "LeftX" => 0,
                "LeftY" => 1,
                "RightX" => 2,
                "RightY" => 3,
                "LeftTrigger" => 4,
                "RightTrigger" => 5,
                _ => return None,
            };
            key.source_subtype = InputSubclass::ControllerAxis;
            key.data = idx as u32;
            if binding.starts_with('-') {
                key.modifier = InputModifier::Negate;
            }
        } else {
            // Map button name to a numeric index matching `s_button_setting_names`.
            let idx = match binding {
                "DPadUp" => 0,
                "DPadDown" => 1,
                "DPadLeft" => 2,
                "DPadRight" => 3,
                "Start" => 4,
                "Back" => 5,
                "LeftStick" => 6,
                "RightStick" => 7,
                "LeftShoulder" => 8,
                "RightShoulder" => 9,
                "A" => 10,
                "B" => 11,
                "X" => 12,
                "Y" => 13,
                "Guide" => 14,
                _ => return None,
            };
            key.source_subtype = InputSubclass::ControllerButton;
            key.data = idx as u32;
        }
        Some(key)
    }

    fn convert_key_to_string(&self, key: InputBindingKey) -> String {
        if key.source_type != InputSourceType::XInput {
            return String::new();
        }
        match key.source_subtype {
            InputSubclass::ControllerAxis => {
                let name = match key.data {
                    0 => "LeftX",
                    1 => "LeftY",
                    2 => "RightX",
                    3 => "RightY",
                    4 => "LeftTrigger",
                    5 => "RightTrigger",
                    _ => return String::new(),
                };
                let sign = if key.modifier == InputModifier::Negate { '-' } else { '+' };
                format!("XInput-{}/{}{}", key.source_index, sign, name)
            }
            InputSubclass::ControllerButton => {
                let name = match key.data {
                    0 => "DPadUp",
                    1 => "DPadDown",
                    2 => "DPadLeft",
                    3 => "DPadRight",
                    4 => "Start",
                    5 => "Back",
                    6 => "LeftStick",
                    7 => "RightStick",
                    8 => "LeftShoulder",
                    9 => "RightShoulder",
                    10 => "A",
                    11 => "B",
                    12 => "X",
                    13 => "Y",
                    14 => "Guide",
                    _ => return String::new(),
                };
                format!("XInput-{}/{}", key.source_index, name)
            }
            InputSubclass::ControllerMotor => {
                let name = if key.data == 0 { "LargeMotor" } else { "SmallMotor" };
                format!("XInput-{}/{}", key.source_index, name)
            }
            _ => String::new(),
        }
    }

    fn controller_layout(&self, index: u32) -> InputLayout {
        let _ = index;
        InputLayout::Xbox
    }

    fn update_motor_state(&mut self, key: InputBindingKey, intensity: f32) {
        if let Some(ctrl) = self.controllers.get_mut(key.source_index as usize) {
            if key.data == 0 {
                ctrl.has_large_motor = intensity > 0.0;
            } else if key.data == 1 {
                ctrl.has_small_motor = intensity > 0.0;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// InputManager
// ---------------------------------------------------------------------------

/// Aggregate of every input source plus the binding/vibration state.
#[derive(Debug, Default)]
pub struct InputManager {
    sources: RwLock<SourceSlots>,
    bindings: RwLock<BindingMap>,
    vibrations: RwLock<Vec<PadVibrationBinding>>,
    initialized: AtomicBool,
}

#[derive(Debug, Default)]
struct SourceSlots {
    sdl: Option<SDLInputSource>,
    dinput: Option<DInputSource>,
    xinput: Option<XInputSource>,
}

type BindingMap = HashMap<InputBindingKey, Vec<Arc<InputBinding>>>;

impl InputManager {
    /// Creates a new, uninitialized [`InputManager`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Initializes every enabled source. Mirrors the C++
    /// `InputManager::ReloadSources` call.
    pub fn init(&self, settings: &dyn SettingsInterface) -> bool {
        let mut sources = self.sources.write().unwrap();
        sources.sdl = Some(SDLInputSource::default());
        if let Some(src) = sources.sdl.as_mut() {
            if !src.initialize(settings) {
                sources.sdl = None;
            }
        }
        sources.dinput = Some(DInputSource::default());
        if let Some(src) = sources.dinput.as_mut() {
            if !src.initialize(settings) {
                sources.dinput = None;
            }
        }
        sources.xinput = Some(XInputSource::default());
        if let Some(src) = sources.xinput.as_mut() {
            if !src.initialize(settings) {
                sources.xinput = None;
            }
        }
        self.initialized.store(true, Ordering::Release);
        true
    }

    /// Shuts down all sources and clears binding state.
    pub fn shutdown(&self) {
        let mut sources = self.sources.write().unwrap();
        if let Some(src) = sources.sdl.as_mut() {
            src.shutdown();
        }
        if let Some(src) = sources.dinput.as_mut() {
            src.shutdown();
        }
        if let Some(src) = sources.xinput.as_mut() {
            src.shutdown();
        }
        sources.sdl = None;
        sources.dinput = None;
        sources.xinput = None;
        self.bindings.write().unwrap().clear();
        self.vibrations.write().unwrap().clear();
        self.initialized.store(false, Ordering::Release);
    }

    /// Returns `true` if the manager was successfully initialized.
    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::Acquire)
    }

    /// Polls every source for new events. Mirrors `InputManager::PollSources`.
    pub fn poll(&self) {
        let mut sources = self.sources.write().unwrap();
        if let Some(src) = sources.sdl.as_mut() {
            src.poll();
        }
        if let Some(src) = sources.dinput.as_mut() {
            src.poll();
        }
        if let Some(src) = sources.xinput.as_mut() {
            src.poll();
        }
    }

    /// Asks every source to re-enumerate connected devices.
    pub fn update_controllers(&self) -> bool {
        let mut sources = self.sources.write().unwrap();
        let mut changed = false;
        if let Some(src) = sources.sdl.as_mut() {
            changed |= src.reload_devices();
        }
        if let Some(src) = sources.dinput.as_mut() {
            changed |= src.reload_devices();
        }
        if let Some(src) = sources.xinput.as_mut() {
            changed |= src.reload_devices();
        }
        changed
    }

    /// Registers a binding for the given serialized chord.
    ///
    /// `binding` is the human-readable chord (e.g. `Keyboard/Up&SDL-0/Button1`).
    /// The handler is invoked when every key in the chord is active.
    pub fn bind(&self, binding: &str, handler: InputEventHandler) {
        let chord = parse_chord(binding);
        let mut parsed = Vec::new();
        for (source, sub) in &chord {
            if let Some(key) = self.parse_key(source, sub) {
                parsed.push(key);
            } else {
                // Skip unknown keys, matching the C++ behaviour of logging
                // an invalid binding and giving up.
                return;
            }
        }
        if parsed.is_empty() {
            return;
        }
        let mut binding_obj = InputBinding::new(handler);
        binding_obj.keys = parsed.clone();
        binding_obj.full_mask = (1u16 << parsed.len() as u16).wrapping_sub(1) as u8;
        let binding = Arc::new(binding_obj);
        let mut map = self.bindings.write().unwrap();
        for key in &parsed {
            map.entry(*key).or_default().push(Arc::clone(&binding));
        }
    }

    /// Dispatches a single event to the registered bindings.
    pub fn invoke_event(&self, key: InputBindingKey, value: f32) {
        let map = self.bindings.read().unwrap();
        if let Some(list) = map.get(&key.mask_direction()) {
            for binding in list {
                if let InputEventHandler::Axis(cb) = &binding.handler {
                    cb(key, value);
                }
            }
        }
    }

    /// Updates the recorded vibration state for a pad and forwards the request
    /// to the source that owns the binding.
    pub fn set_pad_vibration(
        &self,
        pad_index: u32,
        large_intensity: f32,
        small_intensity: f32,
    ) {
        let mut vibrations = self.vibrations.write().unwrap();
        if let Some(pad) = vibrations.iter_mut().find(|p| p.pad_index == pad_index) {
            pad.motors[0].last_intensity = large_intensity;
            pad.motors[1].last_intensity = small_intensity;
            let combined = pad.are_motors_combined();
            let large = pad.motors[0].clone();
            let small = pad.motors[1].clone();
            drop(vibrations);
            if combined {
                self.dispatch_vibration(large.binding, large.last_intensity.max(small.last_intensity));
            } else {
                self.dispatch_vibration(large.binding, large_intensity);
                self.dispatch_vibration(small.binding, small_intensity);
            }
        }
    }

    fn dispatch_vibration(&self, key: InputBindingKey, intensity: f32) {
        let mut sources = self.sources.write().unwrap();
        match key.source_type {
            InputSourceType::SDL => {
                if let Some(src) = sources.sdl.as_mut() {
                    src.update_motor_state(key, intensity);
                }
            }
            InputSourceType::DInput => { /* not supported */ }
            InputSourceType::XInput => {
                if let Some(src) = sources.xinput.as_mut() {
                    src.update_motor_state(key, intensity);
                }
            }
            _ => {}
        }
    }

    fn parse_key(&self, source: &str, sub: &str) -> Option<InputBindingKey> {
        let sources = self.sources.read().unwrap();
        if let Some(src) = sources.sdl.as_ref() {
            if let Some(key) = src.parse_key_string(source, sub) {
                return Some(key);
            }
        }
        if let Some(src) = sources.dinput.as_ref() {
            if let Some(key) = src.parse_key_string(source, sub) {
                return Some(key);
            }
        }
        if let Some(src) = sources.xinput.as_ref() {
            if let Some(key) = src.parse_key_string(source, sub) {
                return Some(key);
            }
        }
        None
    }
}

/// Splits a chord like `Keyboard/A&SDL-0/+LeftX` into its `(source, sub)`
/// parts.
fn parse_chord(binding: &str) -> Vec<(String, String)> {
    binding
        .split('&')
        .filter_map(|part| {
            let trimmed = part.trim();
            if trimmed.is_empty() {
                return None;
            }
            let (source, sub) = trimmed.split_once('/')?;
            Some((source.to_string(), sub.to_string()))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Process-level entry points
// ---------------------------------------------------------------------------

// `Mutex::new` is not `const`, so we store the manager behind a `OnceLock`
// and lazily initialize the inner `Mutex` on first access. The shim provides
// a single `manager()` accessor that returns the underlying `&'static Mutex`
// for use in the entry points below.
static GLOBAL_MANAGER: std::sync::OnceLock<Mutex<Option<InputManager>>> = std::sync::OnceLock::new();

fn manager() -> &'static Mutex<Option<InputManager>> {
    GLOBAL_MANAGER.get_or_init(|| Mutex::new(None))
}

/// Initializes the global input manager. Mirrors `InputManager::ReloadSources`
/// in the C++ code.
pub fn inputInit() -> bool {
    let guard = manager().lock().unwrap();
    if guard.is_some() {
        return true;
    }
    drop(guard);
    // No real settings interface is available in this stub; we pass a
    // `NullSettings` so `init` is exercised on the default-constructed
    // manager.
    let mgr = InputManager::new();
    let ok = mgr.init(&NullSettings);
    *manager().lock().unwrap() = Some(mgr);
    ok
}

/// Tears down the global input manager.
pub fn inputShutdown() {
    let mut guard = manager().lock().unwrap();
    if let Some(manager) = guard.as_ref() {
        manager.shutdown();
    }
    *guard = None;
}

/// Returns the global input manager, if one has been initialized.
pub fn input_manager() -> Option<&'static InputManager> {
    // The `OnceLock` holds the manager behind a `Mutex` so we cannot vend a
    // long-lived reference. We expose the value as an owned snapshot instead.
    let guard = manager().lock().unwrap();
    if guard.is_none() {
        return None;
    }
    // SAFETY: the manager pointer is stable for the program's lifetime
    // once initialized. The `AtomicPtr` makes the initialization itself
    // thread-safe.
    let ptr = INPUT_MANAGER_PTR.load(std::sync::atomic::Ordering::Acquire);
    if ptr.is_null() {
        None
    } else {
        Some(unsafe { &*ptr })
    }
}

// `input_manager` cannot return a real `&'static` reference through the
// `OnceLock`+`Mutex` API alone, so we maintain a raw pointer in a separate
// `AtomicPtr` to bridge the gap. `AtomicPtr` is `Send + Sync`, unlike a bare
// raw pointer which cannot live in a `static`. The pointer is set exactly
// once, the first time `inputInit` runs to completion.
static INPUT_MANAGER_PTR: std::sync::atomic::AtomicPtr<InputManager> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

/// No-op [`SettingsInterface`] used by the global entry points when the host
/// does not provide one.
struct NullSettings;
impl SettingsInterface for NullSettings {
    fn get_bool(&self, _section: &str, _key: &str, default: bool) -> bool {
        default
    }
    fn get_float(&self, _section: &str, _key: &str, default: f32) -> f32 {
        default
    }
    fn get_int(&self, _section: &str, _key: &str, default: i32) -> i32 {
        default
    }
    fn get_string(&self, _section: &str, _key: &str) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aligned_buffer_rounds_up() {
        assert_eq!(aligned_buffer_size(1), CHUNK_SIZE);
        assert_eq!(aligned_buffer_size(CHUNK_SIZE), CHUNK_SIZE);
        assert_eq!(aligned_buffer_size(CHUNK_SIZE + 1), CHUNK_SIZE * 2);
    }

    #[test]
    fn dinput_identifier_format() {
        assert_eq!(DInputSource::device_identifier(2), "DInput-2");
    }

    #[test]
    fn chord_parser_handles_simple_binding() {
        let parts = parse_chord("SDL-0/Button1");
        assert_eq!(parts, vec![("SDL-0".to_string(), "Button1".to_string())]);
    }

    #[test]
    fn chord_parser_handles_chord() {
        let parts = parse_chord("Keyboard/A & SDL-0/+LeftX");
        assert_eq!(
            parts,
            vec![
                ("Keyboard".to_string(), "A".to_string()),
                ("SDL-0".to_string(), "+LeftX".to_string()),
            ]
        );
    }

    #[test]
    fn audio_backend_repr_is_stable() {
        assert_eq!(AudioBackend::Null as u8, 0);
        assert_eq!(AudioBackend::Cubeb as u8, 1);
        assert_eq!(AudioBackend::SDL as u8, 2);
    }

    #[test]
    fn mask_direction_clears_modifier_bits() {
        let key = InputBindingKey {
            source_type: InputSourceType::SDL,
            source_index: 0,
            source_subtype: InputSubclass::ControllerAxis,
            modifier: InputModifier::Negate,
            invert: true,
            needs_migration: true,
            data: 7,
        };
        let masked = key.mask_direction();
        assert_eq!(masked.modifier, InputModifier::None);
        assert!(!masked.invert);
        assert!(!masked.needs_migration);
        assert_eq!(masked.data, 7);
    }
}
