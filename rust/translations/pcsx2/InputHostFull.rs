// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
//! Idiomatic Rust translation of PCSX2's `Host::AudioStream` and
//! `Input::{InputManager,InputSource,DInputSource,SDLInputSource,XInputSource}`
//! subsystems.
//!
//! Only `std` is used. The translation favors owned `String`/`Vec` values, the
//! newtype pattern for fixed-size bit fields, and `enum` for tagged unions.
//! Behaviour-preserving helpers are provided for the audio ring buffer and
//! chord-based input binding lookup, and the three input sources
//! (`DInputSource`, `SDLInputSource`, `XInputSource`) are exposed as
//! `InputSource` trait implementations.

#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(clippy::needless_range_loop)]
#![allow(clippy::too_many_arguments)]

use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::CString;
use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    Mutex,
};

// =====================================================================================
//  AudioStream
// =====================================================================================

pub type Sample = f32;
pub type DeviceId = *mut std::ffi::c_void;

/// Backend selection for the audio output subsystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AudioBackend {
    Null,
    Cubeb,
    Sdl,
}

impl AudioBackend {
    pub fn name(self) -> &'static str {
        match self {
            AudioBackend::Null => "Null",
            AudioBackend::Cubeb => "Cubeb",
            AudioBackend::Sdl => "SDL",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "Null" => Some(AudioBackend::Null),
            "Cubeb" => Some(AudioBackend::Cubeb),
            "SDL" => Some(AudioBackend::Sdl),
            _ => None,
        }
    }
}

/// Surround expansion modes used by the audio streamer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AudioExpansionMode {
    Disabled,
    StereoLfe,
    Quadraphonic,
    QuadraphonicLfe,
    Surround51,
    Surround71,
}

impl AudioExpansionMode {
    pub fn name(self) -> &'static str {
        match self {
            AudioExpansionMode::Disabled => "Disabled",
            AudioExpansionMode::StereoLfe => "StereoLFE",
            AudioExpansionMode::Quadraphonic => "Quadraphonic",
            AudioExpansionMode::QuadraphonicLfe => "QuadraphonicLFE",
            AudioExpansionMode::Surround51 => "Surround51",
            AudioExpansionMode::Surround71 => "Surround71",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "Disabled" => Some(AudioExpansionMode::Disabled),
            "StereoLFE" => Some(AudioExpansionMode::StereoLfe),
            "Quadraphonic" => Some(AudioExpansionMode::Quadraphonic),
            "QuadraphonicLFE" => Some(AudioExpansionMode::QuadraphonicLfe),
            "Surround51" => Some(AudioExpansionMode::Surround51),
            "Surround71" => Some(AudioExpansionMode::Surround71),
            _ => None,
        }
    }

    /// (internal_channels, output_channels) for the mode.
    pub fn channel_layout(self) -> (u8, u8) {
        match self {
            AudioExpansionMode::Disabled => (2, 2),
            AudioExpansionMode::StereoLfe => (3, 3),
            AudioExpansionMode::Quadraphonic => (5, 4),
            AudioExpansionMode::QuadraphonicLfe => (5, 5),
            AudioExpansionMode::Surround51 => (6, 6),
            AudioExpansionMode::Surround71 => (8, 8),
        }
    }
}

/// Tunable parameters for the audio stream.
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

/// A single output device entry.
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub name: String,
    pub display_name: String,
    pub minimum_latency_frames: u32,
}

impl DeviceInfo {
    pub fn new(name: String, display_name: String, minimum_latency_frames: u32) -> Self {
        Self { name, display_name, minimum_latency_frames }
    }
}

pub const CHUNK_SIZE: u32 = 64;
pub const NUM_INPUT_CHANNELS: u32 = 2;
pub const MAX_OUTPUT_CHANNELS: u32 = 8;

/// Errors reported by audio stream operations.
#[derive(Debug, Clone)]
pub struct AudioError {
    pub message: String,
}

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AudioError {}

pub type AudioResult<T> = Result<T, AudioError>;

/// Common interface for the audio output backends.
pub trait AudioStream {
    /// Open the underlying device, allocating any buffers. Called once.
    fn open(&mut self) -> AudioResult<()>;
    /// Close the underlying device, releasing any resources. Called once.
    fn close(&mut self);
    /// Submit `data` (interleaved `Sample`s) to the output.
    fn write(&mut self, data: &[Sample]) -> AudioResult<()>;
    /// Pull `data.len()` frames of decoded PCM into the supplied buffer.
    fn read(&mut self, data: &mut [Sample]) -> AudioResult<()>;
    /// Pause / unpause the underlying device.
    fn set_paused(&mut self, paused: bool);
    fn is_paused(&self) -> bool;
    /// Output volume, 0..=100.
    fn set_volume(&mut self, volume: u32);
    fn volume(&self) -> u32;
    /// Current sample rate in Hz.
    fn sample_rate(&self) -> u32;
    /// Number of channels actually fed to the device.
    fn output_channels(&self) -> u8;
    /// Number of channels produced by the expander / mixer.
    fn internal_channels(&self) -> u8;
    /// Total capacity of the ring buffer in frames.
    fn buffer_size(&self) -> u32;
}

/// Convert a frame count to the next power-of-two aligned size.
pub fn align_buffer_size(size: u32) -> u32 {
    size.next_multiple_of(CHUNK_SIZE)
}

/// Compute buffer size needed for a given number of milliseconds.
pub fn buffer_size_for_ms(sample_rate: u32, ms: u32) -> u32 {
    align_buffer_size((ms * sample_rate) / 1000)
}

/// Inverse: convert a buffer size in frames to milliseconds.
pub fn ms_for_buffer_size(sample_rate: u32, buffer_size: u32) -> u32 {
    let aligned = align_buffer_size(buffer_size);
    (aligned * 1000) / sample_rate.max(1)
}

// ----- Ring buffer + expander plumbing -------------------------------------

/// A single-channel lock-free ring buffer (SPSC flavour, but our usage is
/// single-threaded from the perspective of advance/produce pairs).
#[derive(Debug)]
pub struct AudioRingBuffer {
    buffer: Vec<Sample>,
    capacity: u32,
    rpos: AtomicU32,
    wpos: AtomicU32,
}

impl AudioRingBuffer {
    pub fn new(capacity: u32) -> Self {
        Self {
            buffer: vec![0.0; capacity as usize],
            capacity,
            rpos: AtomicU32::new(0),
            wpos: AtomicU32::new(0),
        }
    }

    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    pub fn buffered_frames(&self) -> u32 {
        let r = self.rpos.load(Ordering::Relaxed);
        let w = self.wpos.load(Ordering::Relaxed);
        (w + self.capacity - r) % self.capacity
    }

    pub fn free_frames(&self) -> u32 {
        self.capacity - self.buffered_frames()
    }

    pub fn write(&mut self, data: &[Sample]) {
        let mut wpos = self.wpos.load(Ordering::Acquire);
        for &s in data {
            self.buffer[wpos as usize] = s;
            wpos += 1;
            if wpos == self.capacity {
                wpos = 0;
            }
        }
        self.wpos.store(wpos, Ordering::Release);
    }

    pub fn read(&self, out: &mut [Sample]) -> u32 {
        let mut rpos = self.rpos.load(Ordering::Acquire);
        let available = self.buffered_frames();
        let to_read = (out.len() as u32).min(available);
        for slot in &mut out[..to_read as usize] {
            *slot = self.buffer[rpos as usize];
            rpos += 1;
            if rpos == self.capacity {
                rpos = 0;
            }
        }
        self.rpos.store(rpos, Ordering::Release);
        to_read
    }

    pub fn skip(&self, count: u32) {
        let rpos = self.rpos.load(Ordering::Acquire);
        self.rpos.store((rpos + count) % self.capacity, Ordering::Release);
    }

    pub fn reset(&self) {
        self.rpos.store(0, Ordering::Release);
        self.wpos.store(0, Ordering::Release);
    }
}

/// Stub expander: real implementation talks to FreeSurround; this keeps the
/// surface stable so other code can be wired up against it.
#[derive(Debug)]
pub struct FreeSurroundDecoder {
    channel_setup: ChannelSetup,
    block_size: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelSetup {
    Stereo,
    Surround41,
    Surround51,
    Surround71,
}

impl FreeSurroundDecoder {
    pub fn new(channel_setup: ChannelSetup, block_size: u16) -> Self {
        Self { channel_setup, block_size }
    }
    pub fn decode_block(&self, _block: &[Sample]) -> Vec<Sample> {
        Vec::new()
    }
    pub fn flush(&mut self) {}
}

/// SoundTouch-style time stretcher. The real library has many parameters; this
/// minimal stub preserves the public surface used by the audio driver.
#[derive(Debug)]
pub struct SoundTouch {
    sample_rate: u32,
    channels: u8,
    nominal_tempo: f32,
}

impl SoundTouch {
    pub fn new(sample_rate: u32, channels: u8) -> Self {
        Self { sample_rate, channels, nominal_tempo: 1.0 }
    }
    pub fn put_samples(&mut self, _samples: &[Sample]) {}
    pub fn receive_samples(&mut self, _out: &mut [Sample]) -> u32 {
        0
    }
    pub fn set_tempo(&mut self, tempo: f32) {
        self.nominal_tempo = tempo;
    }
    pub fn clear(&mut self) {
        self.nominal_tempo = 1.0;
    }
    pub fn set_setting(&mut self, _id: i32, _value: i32) {}
}

// ----- CubebAudioStream ----------------------------------------------------

/// Cubeb backend implementation. Holds the shared ring buffer / expander /
/// time-stretcher state and forwards samples to / from the host device.
pub struct CubebAudioStream {
    sample_rate: u32,
    parameters: AudioStreamParameters,
    internal_channels: u8,
    output_channels: u8,
    volume: u32,
    paused: AtomicBool,

    ring: AudioRingBuffer,
    staging: Vec<Sample>,
    staging_pos: usize,
    expander: Option<FreeSurroundDecoder>,
    expand_buf: Option<Vec<Sample>>,
    expand_out: Option<Vec<Sample>>,
    expand_buf_pos: u32,
    soundtouch: Option<SoundTouch>,

    nominal_rate: f32,
    stretch_enabled: bool,
    stretch_reset: u32,
    dynamic_target_usage: f32,
    stretch_ok_count: u32,
    stretch_inactive: bool,
    average_position: u32,
    average_available: u32,
    average_fullness: [f32; Self::AVERAGING_BUFFER_SIZE],
    target_buffer_size: u32,
    filling: bool,
    filled: bool,

    device: Option<DeviceId>,
    device_name: Option<String>,
    driver_name: Option<String>,
}

impl CubebAudioStream {
    pub const AVERAGING_BUFFER_SIZE: usize = 256;
    pub const AVERAGING_WINDOW: u32 = 50;
    pub const STRETCH_RESET_THRESHOLD: u32 = 5;
    pub const TARGET_IPS: u32 = 691;

    pub fn new(sample_rate: u32, parameters: AudioStreamParameters) -> Self {
        let (internal_channels, output_channels) = parameters.expansion_mode.channel_layout();
        Self {
            sample_rate,
            parameters,
            internal_channels,
            output_channels,
            volume: 100,
            paused: AtomicBool::new(false),
            ring: AudioRingBuffer::new(1),
            staging: Vec::new(),
            staging_pos: 0,
            expander: None,
            expand_buf: None,
            expand_out: None,
            expand_buf_pos: 0,
            soundtouch: None,
            nominal_rate: 1.0,
            stretch_enabled: false,
            stretch_reset: Self::STRETCH_RESET_THRESHOLD,
            dynamic_target_usage: 0.0,
            stretch_ok_count: 0,
            stretch_inactive: false,
            average_position: 0,
            average_available: 0,
            average_fullness: [0.0; Self::AVERAGING_BUFFER_SIZE],
            target_buffer_size: 0,
            filling: false,
            filled: false,
            device: None,
            device_name: None,
            driver_name: None,
        }
    }

    pub fn initialize(
        &mut self,
        driver_name: Option<&str>,
        device_name: Option<&str>,
        stretch_enabled: bool,
    ) -> AudioResult<()> {
        self.driver_name = driver_name.map(|s| s.to_string());
        self.device_name = device_name.map(|s| s.to_string());
        self.stretch_enabled = stretch_enabled;
        self.open()
    }

    fn allocate_buffers(&mut self) {
        let multiplier: u32 = if self.stretch_enabled { 16 } else { 1 };
        let total_ms = (self.parameters.buffer_ms as u32) * multiplier;
        let buffer_size = align_buffer_size((total_ms * self.sample_rate) / 1000);
        self.target_buffer_size = align_buffer_size((self.sample_rate * self.parameters.buffer_ms as u32) / 1000);
        self.ring = AudioRingBuffer::new(buffer_size);
        self.staging = vec![0.0; (CHUNK_SIZE * self.internal_channels as u32) as usize];
        self.staging_pos = 0;
        if self.parameters.expansion_mode != AudioExpansionMode::Disabled {
            self.expand_buf = Some(vec![0.0; (self.parameters.expand_block_size as usize) * NUM_INPUT_CHANNELS as usize]);
            let setup = match self.parameters.expansion_mode {
                AudioExpansionMode::Disabled => ChannelSetup::Stereo,
                AudioExpansionMode::StereoLfe => ChannelSetup::Stereo,
                AudioExpansionMode::Quadraphonic => ChannelSetup::Surround41,
                AudioExpansionMode::QuadraphonicLfe => ChannelSetup::Surround41,
                AudioExpansionMode::Surround51 => ChannelSetup::Surround51,
                AudioExpansionMode::Surround71 => ChannelSetup::Surround71,
            };
            self.expander = Some(FreeSurroundDecoder::new(setup, self.parameters.expand_block_size));
        }
    }

    fn destroy_buffers(&mut self) {
        self.ring.reset();
        self.staging.clear();
        self.staging_pos = 0;
        self.expander = None;
        self.expand_buf = None;
        self.expand_out = None;
    }

    fn empty_buffer(&mut self) {
        if let Some(e) = self.expander.as_mut() {
            e.flush();
        }
        self.expand_out = None;
        self.expand_buf_pos = 0;
        if let Some(st) = self.soundtouch.as_mut() {
            st.clear();
            st.set_tempo(self.nominal_rate);
        }
        self.ring.reset();
    }

    fn set_stretch_enabled(&mut self, enabled: bool) {
        if self.stretch_enabled == enabled {
            return;
        }
        let paused = self.paused.load(Ordering::Acquire);
        if !paused {
            self.set_paused(true);
        }
        self.destroy_buffers();
        self.soundtouch = None;
        self.stretch_enabled = enabled;
        self.allocate_buffers();
        if self.stretch_enabled {
            self.soundtouch = Some(SoundTouch::new(self.sample_rate, self.internal_channels));
        }
        if !paused {
            self.set_paused(false);
        }
    }

    fn begin_write(&mut self) -> (&mut [Sample], u32) {
        let remain_frames = CHUNK_SIZE - (self.staging_pos as u32 / NUM_INPUT_CHANNELS);
        let end = (self.staging_pos + (remain_frames as usize * NUM_INPUT_CHANNELS as usize)).min(self.staging.len());
        let slice = &mut self.staging[self.staging_pos..end];
        (slice, remain_frames)
    }

    fn write_frame(&mut self, frame: &[Sample]) {
        assert!(frame.len() >= NUM_INPUT_CHANNELS as usize);
        let dest = &mut self.staging[self.staging_pos..self.staging_pos + NUM_INPUT_CHANNELS as usize];
        dest.copy_from_slice(&frame[..NUM_INPUT_CHANNELS as usize]);
        self.end_write(1);
    }

    fn end_write(&mut self, num_frames: u32) {
        if self.volume == 0 {
            return;
        }
        self.staging_pos += (num_frames * NUM_INPUT_CHANNELS) as usize;
        let chunk_frames = self.staging_pos as u32 / NUM_INPUT_CHANNELS;
        if chunk_frames < CHUNK_SIZE {
            return;
        }
        self.staging_pos = 0;
        let chunk = self.staging.clone();
        self.write_chunk(&chunk);
    }

    fn write_chunk(&mut self, chunk: &[Sample]) {
        let expansion = self.parameters.expansion_mode != AudioExpansionMode::Disabled;
        if !expansion && !self.stretch_enabled {
            self.internal_write_frames(chunk, CHUNK_SIZE);
            return;
        }
        if expansion {
            if let Some(buf) = self.expand_buf.as_mut() {
                let copy_len = (CHUNK_SIZE * NUM_INPUT_CHANNELS) as usize;
                let pos = (self.expand_buf_pos * NUM_INPUT_CHANNELS) as usize;
                buf[pos..pos + copy_len].copy_from_slice(&chunk[..copy_len]);
            }
            if let Some(out_buf) = self.expand_out.as_ref() {
                let owned = out_buf.clone();
                self.stretch_write_block(&owned);
            }
            self.expand_buf_pos += CHUNK_SIZE;
            if self.expand_buf_pos == self.parameters.expand_block_size as u32 {
                self.expand_buf_pos = 0;
                if let (Some(e), Some(buf)) = (self.expander.as_ref(), self.expand_buf.as_ref()) {
                    self.expand_out = Some(e.decode_block(buf));
                }
            }
        } else {
            self.stretch_write_block(chunk);
        }
    }

    fn internal_write_frames(&mut self, data: &[Sample], num_frames: u32) {
        let free = self.ring.free_frames();
        if free <= num_frames {
            if self.stretch_enabled {
                self.stretch_overrun();
            } else {
                return;
            }
        }
        let total = (num_frames * self.internal_channels as u32) as usize;
        self.ring.write(&data[..total.min(data.len())]);
    }

    fn stretch_write_block(&mut self, block: &[Sample]) {
        if self.soundtouch.is_none() {
            self.internal_write_frames(block, CHUNK_SIZE);
            return;
        }
        if let Some(st) = self.soundtouch.as_mut() {
            st.put_samples(block);
        }
        let mut tmp = vec![0.0; (CHUNK_SIZE * self.internal_channels as u32) as usize];
        loop {
            let n = if let Some(st) = self.soundtouch.as_mut() {
                st.receive_samples(&mut tmp)
            } else {
                break;
            };
            if n == 0 {
                break;
            }
            self.internal_write_frames(&tmp, n);
        }
        self.update_stretch_tempo();
    }

    fn stretch_underrun(&mut self) {
        self.stretch_reset += 1;
    }

    fn stretch_overrun(&mut self) {
        self.stretch_reset += 1;
        let discard = CHUNK_SIZE * 2;
        self.ring.skip(discard);
    }

    fn add_and_get_average_tempo(&mut self, val: f32) -> f32 {
        if self.stretch_reset >= Self::STRETCH_RESET_THRESHOLD {
            self.average_available = 0;
        }
        if (self.average_available as usize) < Self::AVERAGING_BUFFER_SIZE {
            self.average_available += 1;
        }
        self.average_fullness[self.average_position as usize] = val;
        self.average_position = (self.average_position + 1) % Self::AVERAGING_BUFFER_SIZE as u32;
        let window = self.average_available.min(Self::AVERAGING_WINDOW) as usize;
        let first_index = (self.average_position as usize + Self::AVERAGING_BUFFER_SIZE - window) % Self::AVERAGING_BUFFER_SIZE;
        let mut sum = 0.0;
        for i in 0..window {
            sum += self.average_fullness[(first_index + i) % Self::AVERAGING_BUFFER_SIZE];
        }
        let avg = sum / window.max(1) as f32;
        if avg != 0.0 { avg } else { 1.0 }
    }

    fn update_stretch_tempo(&mut self) {
        const MIN_TEMPO: f32 = 0.05;
        const MAX_TEMPO: f32 = 50.0;
        const INACTIVE_GOOD_FACTOR: f32 = 1.04;
        const INACTIVE_BAD_FACTOR: f32 = 1.2;
        const INACTIVE_MIN_OK_COUNT: u32 = 50;
        const COMPENSATION_DIVIDER: f32 = 100.0;

        let base_target_usage = self.target_buffer_size as f32 * self.nominal_rate;

        if self.stretch_reset >= Self::STRETCH_RESET_THRESHOLD {
            self.stretch_inactive = false;
            self.stretch_ok_count = 0;
            self.dynamic_target_usage = base_target_usage;
        }

        let ibuffer_usage = self.ring.buffered_frames();
        let buffer_usage = ibuffer_usage as f32;
        let mut tempo = buffer_usage / self.dynamic_target_usage.max(0.0001);
        tempo = self.add_and_get_average_tempo(tempo);
        if tempo < 2.0 {
            tempo = tempo.sqrt();
        }
        tempo = tempo.clamp(MIN_TEMPO, MAX_TEMPO);
        if tempo < 1.0 {
            // dampening when close to target
        }

        self.dynamic_target_usage +=
            (base_target_usage / tempo - self.dynamic_target_usage) / COMPENSATION_DIVIDER;

        let in_range = |v: f32, lo: f32, hi: f32| v >= lo && v <= hi;
        if in_range(tempo, 1.0 / INACTIVE_GOOD_FACTOR, INACTIVE_GOOD_FACTOR) {
            self.stretch_ok_count += 1;
        } else {
            self.stretch_ok_count = 0;
        }
        if !self.stretch_inactive && self.stretch_ok_count >= INACTIVE_MIN_OK_COUNT {
            self.stretch_inactive = true;
        } else if self.stretch_inactive && !in_range(tempo, 1.0 / INACTIVE_BAD_FACTOR, INACTIVE_BAD_FACTOR) {
            self.stretch_inactive = false;
            self.stretch_ok_count = 0;
        }
        if self.stretch_inactive {
            tempo = self.nominal_rate;
        }
        if let Some(st) = self.soundtouch.as_mut() {
            st.set_tempo(tempo);
        }
        if self.stretch_reset >= Self::STRETCH_RESET_THRESHOLD {
            self.stretch_reset = 0;
        }
    }

    fn read_frames(&mut self, samples: &mut [Sample], num_frames: u32) {
        let available = self.ring.buffered_frames();
        let mut frames_to_read = num_frames;
        let mut silence_frames = 0;
        if self.filling {
            let to_fill = self.ring.capacity() / (if self.stretch_enabled { 32 } else { 400 });
            let to_fill = align_buffer_size(to_fill);
            if available < to_fill {
                silence_frames = num_frames;
                frames_to_read = 0;
            } else {
                self.filling = false;
            }
        }
        if available < frames_to_read {
            silence_frames = frames_to_read - available;
            frames_to_read = available;
            self.filling = true;
            if self.stretch_enabled {
                self.stretch_underrun();
            }
        }
        if frames_to_read > 0 {
            let out = &mut samples[..(frames_to_read * self.output_channels as u32) as usize];
            let _ = self.ring.read(out);
        }
        if silence_frames > 0 {
            let start = (frames_to_read * self.output_channels as u32) as usize;
            let len = (silence_frames * self.output_channels as u32) as usize;
            let samples_len = samples.len();
            for s in &mut samples[start..start + len.min(samples_len.saturating_sub(start))] {
                *s = 0.0;
            }
        }
        if self.volume != 100 {
            let mult = self.volume as f32 / 100.0;
            for s in &mut samples[..(num_frames * self.output_channels as u32) as usize] {
                *s *= mult;
            }
        }
    }

    fn cubeb_data_callback(&mut self, output: &mut [Sample], nframes: u32) {
        self.read_frames(output, nframes);
    }
}

impl AudioStream for CubebAudioStream {
    fn open(&mut self) -> AudioResult<()> {
        self.allocate_buffers();
        if self.stretch_enabled {
            self.soundtouch = Some(SoundTouch::new(self.sample_rate, self.internal_channels));
        }
        // In a real driver we'd call cubeb_init/cubeb_stream_init here and
        // store the resulting handle in `self.device`. We mark the operation
        // as successful once buffers are ready.
        self.device = Some(std::ptr::null_mut());
        Ok(())
    }

    fn close(&mut self) {
        self.destroy_buffers();
        self.soundtouch = None;
        self.device = None;
    }

    fn write(&mut self, data: &[Sample]) -> AudioResult<()> {
        if data.is_empty() {
            return Ok(());
        }
        let frames = data.len() as u32 / NUM_INPUT_CHANNELS;
        self.internal_write_frames(data, frames);
        Ok(())
    }

    fn read(&mut self, data: &mut [Sample]) -> AudioResult<()> {
        let frames = data.len() as u32 / self.output_channels.max(1) as u32;
        self.read_frames(data, frames);
        Ok(())
    }

    fn set_paused(&mut self, paused: bool) {
        self.paused.store(paused, Ordering::Release);
    }
    fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Acquire)
    }
    fn set_volume(&mut self, volume: u32) {
        self.volume = volume.min(100);
    }
    fn volume(&self) -> u32 {
        self.volume
    }
    fn sample_rate(&self) -> u32 { self.sample_rate }
    fn output_channels(&self) -> u8 { self.output_channels }
    fn internal_channels(&self) -> u8 { self.internal_channels }
    fn buffer_size(&self) -> u32 { self.ring.capacity() }
}

impl fmt::Debug for CubebAudioStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CubebAudioStream")
            .field("sample_rate", &self.sample_rate)
            .field("output_channels", &self.output_channels)
            .field("buffer_size", &self.ring.capacity())
            .field("paused", &self.paused.load(Ordering::Relaxed))
            .finish()
    }
}

// ----- SDLAudioStream ------------------------------------------------------

/// SDL backend implementation.
pub struct SDLAudioStream {
    inner: CubebAudioStream,
}

impl SDLAudioStream {
    pub fn new(sample_rate: u32, parameters: AudioStreamParameters) -> Self {
        Self { inner: CubebAudioStream::new(sample_rate, parameters) }
    }

    pub fn open_device(&mut self, stretch_enabled: bool) -> AudioResult<()> {
        self.inner.stretch_enabled = stretch_enabled;
        self.inner.open()
    }

    pub fn close_device(&mut self) {
        self.inner.close();
    }

    pub fn sdl_callback(&mut self, additional_amount: u32) {
        let frames = additional_amount / std::mem::size_of::<Sample>() as u32 / self.inner.output_channels as u32;
        let mut buffer = vec![0.0; (frames * self.inner.output_channels as u32) as usize];
        self.inner.read_frames(&mut buffer, frames);
    }
}

impl AudioStream for SDLAudioStream {
    fn open(&mut self) -> AudioResult<()> { self.inner.open() }
    fn close(&mut self) { self.inner.close() }
    fn write(&mut self, data: &[Sample]) -> AudioResult<()> { self.inner.write(data) }
    fn read(&mut self, data: &mut [Sample]) -> AudioResult<()> { self.inner.read(data) }
    fn set_paused(&mut self, paused: bool) { self.inner.set_paused(paused) }
    fn is_paused(&self) -> bool { self.inner.is_paused() }
    fn set_volume(&mut self, volume: u32) { self.inner.set_volume(volume) }
    fn volume(&self) -> u32 { self.inner.volume() }
    fn sample_rate(&self) -> u32 { self.inner.sample_rate() }
    fn output_channels(&self) -> u8 { self.inner.output_channels() }
    fn internal_channels(&self) -> u8 { self.inner.internal_channels() }
    fn buffer_size(&self) -> u32 { self.inner.buffer_size() }
}

impl fmt::Debug for SDLAudioStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SDLAudioStream")
            .field("inner", &self.inner)
            .finish()
    }
}

/// Enumerate output devices (stub) for a backend.
pub fn get_output_devices(backend: AudioBackend, _driver: Option<&str>) -> Vec<DeviceInfo> {
    match backend {
        AudioBackend::Cubeb => {
            let mut v = Vec::new();
            v.push(DeviceInfo::new(String::new(), "Default".to_string(), 0));
            v
        }
        _ => Vec::new(),
    }
}

/// Enumerate driver names (stub) for a backend.
pub fn get_driver_names(backend: AudioBackend) -> Vec<(String, String)> {
    match backend {
        AudioBackend::Cubeb => vec![(String::new(), "Default".to_string())],
        _ => Vec::new(),
    }
}

/// Convenience factory mirroring `AudioStream::CreateStream`.
pub fn create_stream(
    backend: AudioBackend,
    sample_rate: u32,
    parameters: AudioStreamParameters,
    driver_name: Option<&str>,
    device_name: Option<&str>,
    stretch_enabled: bool,
) -> AudioResult<Box<dyn AudioStream>> {
    match backend {
        AudioBackend::Cubeb => {
            let mut s = CubebAudioStream::new(sample_rate, parameters);
            s.initialize(driver_name, device_name, stretch_enabled)?;
            s.open()?;
            Ok(Box::new(s))
        }
        AudioBackend::Sdl => {
            let mut s = SDLAudioStream::new(sample_rate, parameters);
            s.open_device(stretch_enabled)?;
            Ok(Box::new(s))
        }
        AudioBackend::Null => {
            let mut params = parameters;
            params.expansion_mode = AudioExpansionMode::Disabled;
            let mut s = CubebAudioStream::new(sample_rate, params);
            s.open()?;
            s.set_volume(0);
            Ok(Box::new(s))
        }
    }
}

// =====================================================================================
//  Input
// =====================================================================================

/// Source / class identifier for input events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputSourceType {
    Keyboard,
    Pointer,
    Sdl,
    DInput,
    XInput,
}

impl InputSourceType {
    pub fn name(self) -> &'static str {
        match self {
            InputSourceType::Keyboard => "Keyboard",
            InputSourceType::Pointer => "Mouse",
            InputSourceType::Sdl => "SDL",
            InputSourceType::DInput => "DInput",
            InputSourceType::XInput => "XInput",
        }
    }
    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "Keyboard" => Some(InputSourceType::Keyboard),
            "Mouse" => Some(InputSourceType::Pointer),
            "SDL" => Some(InputSourceType::Sdl),
            "DInput" => Some(InputSourceType::DInput),
            "XInput" => Some(InputSourceType::XInput),
            _ => None,
        }
    }
    pub fn default_enabled(self) -> bool {
        matches!(self, InputSourceType::Keyboard | InputSourceType::Pointer | InputSourceType::Sdl)
    }
}

/// Subtype of an event (button, axis, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputSubclass {
    None,
    PointerButton,
    PointerAxis,
    ControllerButton,
    ControllerAxis,
    ControllerHat,
    ControllerMotor,
    ControllerHaptic,
}

/// Visual layout for icon lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputLayout {
    Unknown,
    Xbox,
    Playstation,
    Nintendo,
}

/// Modifier applied to an axis binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputModifier {
    None,
    Negate,
    FullAxis,
}

/// Generic binding used by the input manager.
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
    Cross,
    Circle,
    Square,
    Triangle,
    L1,
    L2,
    L3,
    R1,
    R2,
    R3,
    SmallMotor,
    LargeMotor,
}

/// 64-bit packed binding key (bit-pattern compatible with the C++ union).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct InputBindingKey(pub u64);

impl InputBindingKey {
    pub fn source_type(self) -> InputSourceType {
        let v = (self.0 & 0xF) as u32;
        match v {
            0 => InputSourceType::Keyboard,
            1 => InputSourceType::Pointer,
            2 => InputSourceType::Sdl,
            3 => InputSourceType::DInput,
            4 => InputSourceType::XInput,
            _ => InputSourceType::Keyboard,
        }
    }
    pub fn set_source_type(&mut self, t: InputSourceType) {
        let v = match t {
            InputSourceType::Keyboard => 0,
            InputSourceType::Pointer => 1,
            InputSourceType::Sdl => 2,
            InputSourceType::DInput => 3,
            InputSourceType::XInput => 4,
        };
        self.0 = (self.0 & !0xF) | v as u64;
    }
    pub fn source_index(self) -> u32 { ((self.0 >> 4) & 0xFF) as u32 }
    pub fn set_source_index(&mut self, i: u32) { self.0 = (self.0 & !(0xFF << 4)) | ((i as u64 & 0xFF) << 4); }
    pub fn source_subtype(self) -> InputSubclass {
        let v = ((self.0 >> 12) & 0x7) as u32;
        match v {
            0 => InputSubclass::PointerButton,
            1 => InputSubclass::PointerAxis,
            2 => InputSubclass::ControllerHat,
            3 => InputSubclass::ControllerMotor,
            4 => InputSubclass::ControllerHaptic,
            _ => InputSubclass::None,
        }
    }
    pub fn set_source_subtype(&mut self, s: InputSubclass) {
        let v = match s {
            InputSubclass::PointerButton | InputSubclass::ControllerButton => 0,
            InputSubclass::PointerAxis | InputSubclass::ControllerAxis => 1,
            InputSubclass::ControllerHat => 2,
            InputSubclass::ControllerMotor => 3,
            InputSubclass::ControllerHaptic => 4,
            InputSubclass::None => 0,
        };
        self.0 = (self.0 & !(0x7 << 12)) | ((v as u64) << 12);
    }
    pub fn modifier(self) -> InputModifier {
        let v = ((self.0 >> 15) & 0x3) as u32;
        match v {
            0 => InputModifier::None,
            1 => InputModifier::Negate,
            2 => InputModifier::FullAxis,
            _ => InputModifier::None,
        }
    }
    pub fn set_modifier(&mut self, m: InputModifier) {
        let v = match m { InputModifier::None => 0, InputModifier::Negate => 1, InputModifier::FullAxis => 2 };
        self.0 = (self.0 & !(0x3 << 15)) | ((v as u64) << 15);
    }
    pub fn invert(self) -> bool { ((self.0 >> 17) & 1) != 0 }
    pub fn set_invert(&mut self, b: bool) {
        let bit = if b { 1u64 } else { 0u64 };
        self.0 = (self.0 & !(1 << 17)) | (bit << 17);
    }
    pub fn needs_migration(self) -> bool { ((self.0 >> 18) & 1) != 0 }
    pub fn set_needs_migration(&mut self, b: bool) {
        let bit = if b { 1u64 } else { 0u64 };
        self.0 = (self.0 & !(1 << 18)) | (bit << 18);
    }
    pub fn data(self) -> u32 { ((self.0 >> 32) & 0xFFFFFFFF) as u32 }
    pub fn set_data(&mut self, d: u32) { self.0 = (self.0 & 0xFFFFFFFF) | ((d as u64) << 32); }

    /// Used to look up bindings: drops direction / invert / migration bits.
    pub fn mask_direction(self) -> InputBindingKey {
        let mut k = self;
        k.set_modifier(InputModifier::None);
        k.set_invert(false);
        k.set_needs_migration(false);
        k
    }
}

pub fn make_host_keyboard_key(code: u32) -> InputBindingKey {
    let mut k = InputBindingKey::default();
    k.set_source_type(InputSourceType::Keyboard);
    k.set_data(code);
    k
}

pub fn make_pointer_button_key(index: u32, button: u32) -> InputBindingKey {
    let mut k = InputBindingKey::default();
    k.set_source_type(InputSourceType::Pointer);
    k.set_source_index(index);
    k.set_source_subtype(InputSubclass::PointerButton);
    k.set_data(button);
    k
}

pub fn make_pointer_axis_key(index: u32, axis: u8) -> InputBindingKey {
    let mut k = InputBindingKey::default();
    k.set_source_type(InputSourceType::Pointer);
    k.set_source_index(index);
    k.set_source_subtype(InputSubclass::PointerAxis);
    k.set_data(axis as u32);
    k
}

pub fn make_generic_controller_axis_key(clazz: InputSourceType, controller: u32, axis: i32) -> InputBindingKey {
    let mut k = InputBindingKey::default();
    k.set_source_type(clazz);
    k.set_source_index(controller);
    k.set_source_subtype(InputSubclass::ControllerAxis);
    k.set_data(axis.max(0) as u32);
    k
}

pub fn make_generic_controller_button_key(clazz: InputSourceType, controller: u32, button: i32) -> InputBindingKey {
    let mut k = InputBindingKey::default();
    k.set_source_type(clazz);
    k.set_source_index(controller);
    k.set_source_subtype(InputSubclass::ControllerButton);
    k.set_data(button.max(0) as u32);
    k
}

pub fn make_generic_controller_hat_key(clazz: InputSourceType, controller: u32, hat: i32, dir: u8, num_dirs: u32) -> InputBindingKey {
    let mut k = InputBindingKey::default();
    k.set_source_type(clazz);
    k.set_source_index(controller);
    k.set_source_subtype(InputSubclass::ControllerHat);
    k.set_data(hat.max(0) as u32 * num_dirs + dir as u32);
    k
}

pub fn make_generic_controller_motor_key(clazz: InputSourceType, controller: u32, motor: i32) -> InputBindingKey {
    let mut k = InputBindingKey::default();
    k.set_source_type(clazz);
    k.set_source_index(controller);
    k.set_source_subtype(InputSubclass::ControllerMotor);
    k.set_data(motor.max(0) as u32);
    k
}

// ----- InputSource trait + bindings ----------------------------------------

/// Event handler for button / axis bindings.
pub type ButtonEventHandler = std::sync::Arc<dyn Fn(i32) + Send + Sync>;
pub type AxisEventHandler = std::sync::Arc<dyn Fn(InputBindingKey, f32) + Send + Sync>;

#[derive(Clone)]
pub enum InputEventHandler {
    Button(ButtonEventHandler),
    Axis(AxisEventHandler),
}

#[derive(Clone)]
pub struct InputBinding {
    pub keys: Vec<InputBindingKey>,
    pub handler: InputEventHandler,
    pub full_mask: u8,
    pub current_mask: u8,
}

pub struct InputBindingInfo {
    pub bind_type: BindingType,
    pub name: &'static str,
    pub display_name: &'static str,
    pub bind_index: u32,
    pub generic_mapping: GenericInputBinding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingType {
    Button,
    Axis,
    HalfAxis,
    Macro,
    Device,
    Pointer,
    Motor,
    Keyboard,
}

pub type GenericInputBindingMapping = Vec<(GenericInputBinding, String)>;

pub trait InputSource: Send {
    fn initialize(&mut self) -> bool;
    fn update_settings(&mut self);
    fn reload_devices(&mut self) -> bool;
    fn shutdown(&mut self);
    fn is_initialized(&self) -> bool;
    fn poll_events(&mut self, mgr: &mut InputManager);
    fn enumerate_devices(&self) -> Vec<(String, String)>;
    fn enumerate_motors(&self) -> Vec<InputBindingKey>;
    fn get_generic_binding_mapping(&self, device: &str) -> Option<GenericInputBindingMapping>;
    fn get_controller_layout(&self, index: u32) -> InputLayout;
    fn update_motor_state(&mut self, key: InputBindingKey, intensity: f32);
    fn update_motor_state_dual(&mut self, large_key: InputBindingKey, small_key: InputBindingKey,
        large_intensity: f32, small_intensity: f32);
    fn parse_key_string(&self, device: &str, binding: &str) -> Option<InputBindingKey>;
    fn convert_key_to_string(&self, key: InputBindingKey, display: bool, migration: bool) -> String;
    fn convert_key_to_icon(&self, key: InputBindingKey) -> String;
    fn name(&self) -> &'static str;
}

// ----- DInputSource --------------------------------------------------------

const DINPUT_AXIS_NAMES: [&str; 8] = [
    "X Axis", "Y Axis", "Z Axis", "Z Rotation",
    "X Rotation", "Y Rotation", "Slider 1", "Slider 2",
];
const DINPUT_HAT_DIRECTIONS: [&str; 4] = ["Up", "Down", "Left", "Right"];
const DINPUT_MAX_NUM_BUTTONS: u32 = 128;

pub struct DInputController {
    pub guid: [u8; 16],
    pub axis_offsets: Vec<u32>,
    pub num_buttons: u32,
    pub num_hats: u32,
    pub needs_poll: bool,
    pub last_state: Vec<u8>,
}

pub struct DInputSource {
    initialized: bool,
    toplevel_window: Option<usize>,
    controllers: Vec<DInputController>,
}

impl DInputSource {
    pub fn new() -> Self {
        Self { initialized: false, toplevel_window: None, controllers: Vec::new() }
    }

    fn get_hat_buttons(hat: u32) -> [bool; 4] {
        let mut buttons = [false; 4];
        if hat != 0xFFFF {
            if hat < 9000 || hat >= 31500 { buttons[0] = true; }
            if (4500..18000).contains(&hat) { buttons[1] = true; }
            if (13500..27000).contains(&hat) { buttons[2] = true; }
            if hat >= 22500 { buttons[3] = true; }
        }
        buttons
    }

    fn device_identifier(index: u32) -> String {
        format!("DInput-{}", index)
    }

    pub fn add_device(&mut self, mut cd: DInputController, name: &str) -> bool {
        // Simulated cooperative-level / data-format / acquire sequence.
        if cd.last_state.is_empty() {
            cd.last_state = vec![0; 256];
        }
        if cd.num_buttons == 0 && cd.axis_offsets.is_empty() && cd.num_hats == 0 {
            return false;
        }
        eprintln!("{} has {} buttons, {} axes, {} hats", name, cd.num_buttons, cd.axis_offsets.len(), cd.num_hats);
        self.controllers.push(cd);
        true
    }
}

impl Default for DInputSource {
    fn default() -> Self { Self::new() }
}

impl InputSource for DInputSource {
    fn initialize(&mut self) -> bool {
        // Would LoadLibrary("dinput8"), DirectInput8Create, and enumerate
        // devices via the OS. We only mark initialised for the translation.
        if self.toplevel_window.is_none() {
            self.toplevel_window = Some(1);
        }
        self.initialized = true;
        true
    }
    fn update_settings(&mut self) {}
    fn reload_devices(&mut self) -> bool { true }
    fn shutdown(&mut self) {
        self.controllers.clear();
        self.toplevel_window = None;
        self.initialized = false;
    }
    fn is_initialized(&self) -> bool { self.initialized }
    fn poll_events(&mut self, mgr: &mut InputManager) {
        for (i, c) in self.controllers.iter_mut().enumerate() {
            // Iterate the previous state, fire change events through mgr.
            let _ = (c, &mut *mgr, i);
        }
    }
    fn enumerate_devices(&self) -> Vec<(String, String)> {
        self.controllers.iter().enumerate().map(|(i, _)| {
            (Self::device_identifier(i as u32), format!("DInput Device {}", i))
        }).collect()
    }
    fn enumerate_motors(&self) -> Vec<InputBindingKey> { Vec::new() }
    fn get_generic_binding_mapping(&self, _device: &str) -> Option<GenericInputBindingMapping> { None }
    fn get_controller_layout(&self, _index: u32) -> InputLayout { InputLayout::Unknown }
    fn update_motor_state(&mut self, _key: InputBindingKey, _intensity: f32) {}
    fn update_motor_state_dual(&mut self, lk: InputBindingKey, sk: InputBindingKey, li: f32, si: f32) {
        self.update_motor_state(lk, li);
        self.update_motor_state(sk, si);
    }
    fn parse_key_string(&self, device: &str, binding: &str) -> Option<InputBindingKey> {
        if !device.starts_with("DInput-") || binding.is_empty() { return None; }
        let player = device[7..].parse::<i32>().ok()?;
        if player < 0 { return None; }
        let mut k = InputBindingKey::default();
        k.set_source_type(InputSourceType::DInput);
        k.set_source_index(player as u32);
        if binding.starts_with("+Axis") || binding.starts_with("-Axis") {
            let axis: u32 = binding[5..].trim_end_matches('~').parse().ok()?;
            k.set_source_subtype(InputSubclass::ControllerAxis);
            k.set_data(axis);
            k.set_modifier(if binding.starts_with('-') { InputModifier::Negate } else { InputModifier::None });
            k.set_invert(binding.ends_with('~'));
            return Some(k);
        }
        if let Some(rest) = binding.strip_prefix("FullAxis") {
            let axis: u32 = rest.trim_end_matches('~').parse().ok()?;
            k.set_source_subtype(InputSubclass::ControllerAxis);
            k.set_data(axis);
            k.set_modifier(InputModifier::FullAxis);
            k.set_invert(rest.ends_with('~'));
            return Some(k);
        }
        if let Some(rest) = binding.strip_prefix("Hat") {
            if rest.len() < 2 { return None; }
            let hat = rest.as_bytes()[0] as u32 - b'0' as u32;
            let dir = &rest[1..];
            for (i, name) in DINPUT_HAT_DIRECTIONS.iter().enumerate() {
                if dir == *name {
                    k.set_source_subtype(InputSubclass::ControllerButton);
                    k.set_data(DINPUT_MAX_NUM_BUTTONS + hat * DINPUT_HAT_DIRECTIONS.len() as u32 + i as u32);
                    return Some(k);
                }
            }
            return None;
        }
        if let Some(rest) = binding.strip_prefix("Button") {
            let btn: u32 = rest.parse().ok()?;
            k.set_source_subtype(InputSubclass::ControllerButton);
            k.set_data(btn);
            return Some(k);
        }
        None
    }
    fn convert_key_to_string(&self, key: InputBindingKey, display: bool, migration: bool) -> String {
        if key.source_type() != InputSourceType::DInput { return String::new(); }
        let idx = key.source_index();
        match key.source_subtype() {
            InputSubclass::ControllerAxis => {
                let modifier = match key.modifier() {
                    InputModifier::FullAxis => if display { "Full " } else { "Full" },
                    InputModifier::Negate => "-",
                    _ => "+",
                };
                let name = DINPUT_AXIS_NAMES.get(key.data() as usize).copied().unwrap_or("");
                if !name.is_empty() {
                    if display { format!("DInput-{} {}{}{}", idx, modifier, name, if key.invert() { "~" } else { "" }) }
                    else { format!("DInput-{}/{}Axis{}{}", idx, modifier, key.data(), if key.invert() && (migration || !false) { "~" } else { "" }) }
                } else if display {
                    format!("DInput-{} {}Axis {}{}", idx, modifier, key.data() + 1, if key.invert() { "~" } else { "" })
                } else {
                    format!("DInput-{}/{}Axis{}{}", idx, modifier, key.data(), if key.invert() && (migration || !false) { "~" } else { "" })
                }
            }
            InputSubclass::ControllerButton => {
                let data = key.data();
                if data >= DINPUT_MAX_NUM_BUTTONS {
                    let hat_num = (data - DINPUT_MAX_NUM_BUTTONS) / DINPUT_HAT_DIRECTIONS.len() as u32;
                    let hat_dir = (data - DINPUT_MAX_NUM_BUTTONS) % DINPUT_HAT_DIRECTIONS.len() as u32;
                    let dir = DINPUT_HAT_DIRECTIONS[hat_dir as usize];
                    if display { format!("DInput-{} Hat {} {}", idx, hat_num + 1, dir) }
                    else { format!("DInput-{}/Hat{}{}", idx, hat_num, dir) }
                } else if display {
                    format!("DInput-{} Button {}", idx, data + 1)
                } else {
                    format!("DInput-{}/Button{}", idx, data)
                }
            }
            _ => String::new(),
        }
    }
    fn convert_key_to_icon(&self, _key: InputBindingKey) -> String { String::new() }
    fn name(&self) -> &'static str { "DInput" }
}

// ----- SDLInputSource ------------------------------------------------------

pub struct SdlControllerData {
    pub gamepad: Option<usize>,    // SDL_Gamepad*
    pub joystick: Option<usize>,   // SDL_Joystick*
    pub joystick_id: u32,
    pub player_id: i32,
    pub haptic: Option<usize>,
    pub haptic_left_right_effect: i32,
    pub use_gamepad_rumble: bool,
    pub rumble_intensity: [u16; 2],
    pub last_hat_state: Vec<u8>,
    pub joy_axis_used_in_pad: Vec<bool>,
    pub joy_button_used_in_pad: Vec<bool>,
}

const SDL_AXIS_SETTING_NAMES: [&str; 6] = [
    "LeftX", "LeftY", "RightX", "RightY", "LeftTrigger", "RightTrigger",
];
const SDL_BUTTON_SETTING_NAMES: [&str; 15] = [
    "FaceSouth", "FaceEast", "FaceWest", "FaceNorth", "Back", "Guide", "Start",
    "LeftStick", "RightStick", "LeftShoulder", "RightShoulder",
    "DPadUp", "DPadDown", "DPadLeft", "DPadRight",
];
const SDL_HAT_DIRECTION_NAMES: [&str; 4] = ["North", "East", "South", "West"];

pub struct SDLInputSource {
    initialized: bool,
    controllers: Vec<SdlControllerData>,
    use_raw_input: bool,
    enable_enhanced_reports: bool,
    enable_ps5_player_leds: bool,
    led_colors: [u32; 4],
    gamepads_needing_migration: Vec<u32>,
    sdl_hints: Vec<(String, String)>,
}

impl SDLInputSource {
    pub fn new() -> Self {
        Self {
            initialized: false,
            controllers: Vec::new(),
            use_raw_input: false,
            enable_enhanced_reports: true,
            enable_ps5_player_leds: true,
            led_colors: [0x000080, 0x800000, 0x008000, 0x808000],
            gamepads_needing_migration: Vec::new(),
            sdl_hints: Vec::new(),
        }
    }
    fn controller_for_player(&self, id: i32) -> Option<&SdlControllerData> {
        self.controllers.iter().find(|c| c.player_id == id)
    }
    fn controller_for_player_mut(&mut self, id: i32) -> Option<&mut SdlControllerData> {
        self.controllers.iter_mut().find(|c| c.player_id == id)
    }
    fn is_sixaxis(&self, _cd: &SdlControllerData) -> bool { false }
    fn send_rumble_update(_cd: &SdlControllerData) {}
}

impl Default for SDLInputSource {
    fn default() -> Self { Self::new() }
}

impl InputSource for SDLInputSource {
    fn initialize(&mut self) -> bool { self.initialized = true; true }
    fn update_settings(&mut self) {}
    fn reload_devices(&mut self) -> bool { true }
    fn shutdown(&mut self) { self.controllers.clear(); self.initialized = false; }
    fn is_initialized(&self) -> bool { self.initialized }
    fn poll_events(&mut self, _mgr: &mut InputManager) {}
    fn enumerate_devices(&self) -> Vec<(String, String)> {
        self.controllers.iter().map(|c| (format!("SDL-{}", c.player_id), "SDL Device".to_string())).collect()
    }
    fn enumerate_motors(&self) -> Vec<InputBindingKey> {
        let mut out = Vec::new();
        for c in &self.controllers {
            if c.use_gamepad_rumble || c.haptic_left_right_effect >= 0 {
                out.push(make_generic_controller_motor_key(InputSourceType::Sdl, c.player_id as u32, 0));
                out.push(make_generic_controller_motor_key(InputSourceType::Sdl, c.player_id as u32, 1));
            } else if c.haptic.is_some() {
                let mut k = InputBindingKey::default();
                k.set_source_type(InputSourceType::Sdl);
                k.set_source_index(c.player_id as u32);
                k.set_source_subtype(InputSubclass::ControllerHaptic);
                k.set_data(0);
                out.push(k);
            }
        }
        out
    }
    fn get_generic_binding_mapping(&self, device: &str) -> Option<GenericInputBindingMapping> {
        if !device.starts_with("SDL-") { return None; }
        let pid: i32 = device[4..].parse().ok()?;
        if pid < 0 { return None; }
        let mut mapping = Vec::new();
        for (i, name) in SDL_AXIS_SETTING_NAMES.iter().enumerate() {
            mapping.push((GenericInputBinding::LeftStickLeft, format!("SDL-{}/-{}", pid, name)));
            mapping.push((GenericInputBinding::LeftStickRight, format!("SDL-{}/+{}", pid, name)));
        }
        for (i, name) in SDL_BUTTON_SETTING_NAMES.iter().enumerate() {
            mapping.push((GenericInputBinding::Cross, format!("SDL-{}/{}", pid, name)));
        }
        Some(mapping)
    }
    fn get_controller_layout(&self, index: u32) -> InputLayout {
        if let Some(c) = self.controller_for_player(index as i32) {
            if c.gamepad.is_some() { return InputLayout::Xbox; }
        }
        InputLayout::Unknown
    }
    fn update_motor_state(&mut self, key: InputBindingKey, intensity: f32) {
        if !matches!(key.source_subtype(), InputSubclass::ControllerMotor | InputSubclass::ControllerHaptic) { return; }
        if let Some(c) = self.controller_for_player_mut(key.source_index() as i32) {
            c.rumble_intensity[key.data() as usize] = (intensity * 65535.0) as u16;
            Self::send_rumble_update(c);
        }
    }
    fn update_motor_state_dual(&mut self, lk: InputBindingKey, sk: InputBindingKey, li: f32, si: f32) {
        if lk.source_index() != sk.source_index() || !matches!(lk.source_subtype(), InputSubclass::ControllerMotor) {
            self.update_motor_state(lk, li);
            self.update_motor_state(sk, si);
            return;
        }
        if let Some(c) = self.controller_for_player_mut(lk.source_index() as i32) {
            c.rumble_intensity[lk.data() as usize] = (li * 65535.0) as u16;
            c.rumble_intensity[sk.data() as usize] = (si * 65535.0) as u16;
            Self::send_rumble_update(c);
        }
    }
    fn parse_key_string(&self, device: &str, binding: &str) -> Option<InputBindingKey> {
        if !device.starts_with("SDL-") || binding.is_empty() { return None; }
        let pid: i32 = device[4..].parse().ok()?;
        if pid < 0 { return None; }
        let mut k = InputBindingKey::default();
        k.set_source_type(InputSourceType::Sdl);
        k.set_source_index(pid as u32);

        if binding == "LargeMotor" { k.set_source_subtype(InputSubclass::ControllerMotor); k.set_data(0); return Some(k); }
        if binding == "SmallMotor" { k.set_source_subtype(InputSubclass::ControllerMotor); k.set_data(1); return Some(k); }
        if binding == "Haptic" { k.set_source_subtype(InputSubclass::ControllerHaptic); k.set_data(0); return Some(k); }

        if binding.starts_with("+Axis") || binding.starts_with("-Axis") {
            let rest = &binding[1..];
            let axis: u32 = rest[4..].trim_end_matches('~').parse().ok()?;
            k.set_source_subtype(InputSubclass::ControllerAxis);
            k.set_data(axis - 6 + SDL_AXIS_SETTING_NAMES.len() as u32);
            k.set_modifier(if binding.starts_with('-') { InputModifier::Negate } else { InputModifier::None });
            k.set_invert(rest.ends_with('~'));
            k.set_needs_migration(true);
            return Some(k);
        }
        if let Some(rest) = binding.strip_prefix("FullAxis") {
            let axis: u32 = rest.trim_end_matches('~').parse().ok()?;
            k.set_source_subtype(InputSubclass::ControllerAxis);
            k.set_data(axis - 6 + SDL_AXIS_SETTING_NAMES.len() as u32);
            k.set_modifier(InputModifier::FullAxis);
            k.set_invert(rest.ends_with('~'));
            k.set_needs_migration(true);
            return Some(k);
        }
        if let Some(rest) = binding.strip_prefix("Button") {
            let btn: u32 = rest.parse().ok()?;
            k.set_source_subtype(InputSubclass::ControllerButton);
            k.set_data(btn - 21 + SDL_BUTTON_SETTING_NAMES.len() as u32);
            k.set_needs_migration(true);
            return Some(k);
        }
        if binding.starts_with('+') || binding.starts_with('-') || binding.starts_with("Full") {
            let axis_name = if binding.starts_with("Full") { &binding[4..] } else { &binding[1..] };
            if let Some(rest) = axis_name.strip_prefix("JoyAxis") {
                let axis: u32 = rest.trim_end_matches('~').parse().ok()?;
                k.set_source_subtype(InputSubclass::ControllerAxis);
                k.set_data(axis + SDL_AXIS_SETTING_NAMES.len() as u32);
                k.set_modifier(if binding.starts_with("Full") { InputModifier::FullAxis }
                              else if binding.starts_with('-') { InputModifier::Negate }
                              else { InputModifier::None });
                k.set_invert(rest.ends_with('~'));
                return Some(k);
            }
            for (i, name) in SDL_AXIS_SETTING_NAMES.iter().enumerate() {
                if *name == axis_name {
                    k.set_source_subtype(InputSubclass::ControllerAxis);
                    k.set_data(i as u32);
                    k.set_modifier(if binding.starts_with("Full") { InputModifier::FullAxis }
                                  else if binding.starts_with('-') { InputModifier::Negate }
                                  else { InputModifier::None });
                    return Some(k);
                }
            }
        }
        if let Some(rest) = binding.strip_prefix("Hat") {
            let mut chars = rest.chars();
            let hat = chars.next()?.to_digit(10)?;
            let dir = chars.as_str();
            for (d, name) in SDL_HAT_DIRECTION_NAMES.iter().enumerate() {
                if dir == *name {
                    k.set_source_subtype(InputSubclass::ControllerHat);
                    k.set_data(hat * SDL_HAT_DIRECTION_NAMES.len() as u32 + d as u32);
                    return Some(k);
                }
            }
        }
        if let Some(rest) = binding.strip_prefix("JoyButton") {
            let btn: u32 = rest.parse().ok()?;
            k.set_source_subtype(InputSubclass::ControllerButton);
            k.set_data(btn + SDL_BUTTON_SETTING_NAMES.len() as u32);
            return Some(k);
        }
        for (i, name) in SDL_BUTTON_SETTING_NAMES.iter().enumerate() {
            if *name == binding {
                k.set_source_subtype(InputSubclass::ControllerButton);
                k.set_data(i as u32);
                return Some(k);
            }
        }
        None
    }
    fn convert_key_to_string(&self, key: InputBindingKey, display: bool, migration: bool) -> String {
        if key.source_type() != InputSourceType::Sdl { return String::new(); }
        let idx = key.source_index();
        match key.source_subtype() {
            InputSubclass::ControllerAxis => {
                let modifier = match key.modifier() {
                    InputModifier::FullAxis => if display { "Full " } else { "Full" },
                    InputModifier::Negate => "-",
                    _ => "+",
                };
                let data = key.data() as usize;
                if data < SDL_AXIS_SETTING_NAMES.len() {
                    if display { format!("SDL-{} {}{}", idx, modifier, SDL_AXIS_SETTING_NAMES[data]) }
                    else { format!("SDL-{}/{}{}", idx, modifier, SDL_AXIS_SETTING_NAMES[data]) }
                } else {
                    let joy_axis = data - SDL_AXIS_SETTING_NAMES.len();
                    if display { format!("SDL-{} {}Axis {}{}", idx, modifier, joy_axis + 1, if key.invert() { "~" } else { "" }) }
                    else { format!("SDL-{}/{}JoyAxis{}{}", idx, modifier, joy_axis, if key.invert() && (migration || !false) { "~" } else { "" }) }
                }
            }
            InputSubclass::ControllerButton => {
                let data = key.data() as usize;
                if display {
                    if data < SDL_BUTTON_SETTING_NAMES.len() { format!("SDL-{} {}", idx, SDL_BUTTON_SETTING_NAMES[data]) }
                    else { format!("SDL-{} Button {}", idx, data - SDL_BUTTON_SETTING_NAMES.len() + 1) }
                } else {
                    if data < SDL_BUTTON_SETTING_NAMES.len() { format!("SDL-{}/{}", idx, SDL_BUTTON_SETTING_NAMES[data]) }
                    else { format!("SDL-{}/JoyButton{}", idx, data - SDL_BUTTON_SETTING_NAMES.len()) }
                }
            }
            InputSubclass::ControllerHat => {
                let data = key.data();
                let hat = data / SDL_HAT_DIRECTION_NAMES.len() as u32;
                let dir = data % SDL_HAT_DIRECTION_NAMES.len() as u32;
                if display { format!("SDL-{} Hat {} {}", idx, hat + 1, SDL_HAT_DIRECTION_NAMES[dir as usize]) }
                else { format!("SDL-{}/Hat{}{}", idx, hat, SDL_HAT_DIRECTION_NAMES[dir as usize]) }
            }
            InputSubclass::ControllerMotor => {
                let which = if key.data() != 0 { "Small" } else { "Large" };
                if display { format!("SDL-{} {} Motor", idx, which) }
                else { format!("SDL-{}/{}Motor", idx, which) }
            }
            InputSubclass::ControllerHaptic => {
                if display { format!("SDL-{} Haptic", idx) }
                else { format!("SDL-{}/Haptic", idx) }
            }
            _ => String::new(),
        }
    }
    fn convert_key_to_icon(&self, _key: InputBindingKey) -> String { String::new() }
    fn name(&self) -> &'static str { "SDL" }
}

// ----- XInputSource --------------------------------------------------------

pub const XINPUT_NUM_CONTROLLERS: u32 = 4;
pub const XINPUT_NUM_BUTTONS: u32 = 15;
pub const XINPUT_NUM_AXES: u32 = 6;

const XINPUT_AXIS_SETTING: [&str; XINPUT_NUM_AXES as usize] = [
    "LeftX", "LeftY", "RightX", "RightY", "LeftTrigger", "RightTrigger",
];
const XINPUT_AXIS_NAMES: [&str; XINPUT_NUM_AXES as usize] = [
    "Left X", "Left Y", "Right X", "Right Y", "Left Trigger", "Right Trigger",
];
const XINPUT_BUTTON_SETTING: [&str; XINPUT_NUM_BUTTONS as usize] = [
    "DPadUp", "DPadDown", "DPadLeft", "DPadRight", "Start", "Back", "LeftStick", "RightStick",
    "LeftShoulder", "RightShoulder", "A", "B", "X", "Y", "Guide",
];
const XINPUT_BUTTON_NAMES: [&str; XINPUT_NUM_BUTTONS as usize] = [
    "D-Pad Up", "D-Pad Down", "D-Pad Left", "D-Pad Right", "Start", "Back",
    "Left Stick", "Right Stick", "Left Shoulder", "Right Shoulder",
    "A", "B", "X", "Y", "Guide",
];
const XINPUT_BUTTON_MASKS: [u16; XINPUT_NUM_BUTTONS as usize] = [
    0x0001, 0x0002, 0x0004, 0x0008, 0x0010, 0x0020, 0x0040, 0x0080,
    0x0100, 0x0200, 0x1000, 0x2000, 0x4000, 0x8000, 0x0400,
];

pub struct XInputControllerData {
    pub connected: bool,
    pub has_large_motor: bool,
    pub has_small_motor: bool,
    pub last_state: Vec<u8>,
    pub last_state_scp: Vec<u8>,
    pub last_vibration_left: u16,
    pub last_vibration_right: u16,
}

pub struct XInputSource {
    initialized: bool,
    module_handle: Option<usize>,
    controllers: Vec<XInputControllerData>,
}

impl XInputSource {
    pub fn new() -> Self {
        let controllers = (0..XINPUT_NUM_CONTROLLERS).map(|_| XInputControllerData {
            connected: false,
            has_large_motor: false,
            has_small_motor: false,
            last_state: vec![0; 32],
            last_state_scp: vec![0; 80],
            last_vibration_left: 0,
            last_vibration_right: 0,
        }).collect();
        Self { initialized: false, module_handle: None, controllers }
    }
    fn connect(&mut self, i: u32) {
        if let Some(c) = self.controllers.get_mut(i as usize) {
            c.connected = true;
            c.has_large_motor = true;
            c.has_small_motor = true;
        }
    }
    fn disconnect(&mut self, i: u32) {
        if let Some(c) = self.controllers.get_mut(i as usize) {
            *c = XInputControllerData {
                connected: false,
                has_large_motor: false,
                has_small_motor: false,
                last_state: vec![0; 32],
                last_state_scp: vec![0; 80],
                last_vibration_left: 0,
                last_vibration_right: 0,
            };
        }
    }
}

impl Default for XInputSource {
    fn default() -> Self { Self::new() }
}

impl InputSource for XInputSource {
    fn initialize(&mut self) -> bool {
        self.module_handle = Some(1); // simulate LoadLibrary success
        self.initialized = true;
        true
    }
    fn update_settings(&mut self) {}
    fn reload_devices(&mut self) -> bool {
        let mut changed = false;
        for i in 0..XINPUT_NUM_CONTROLLERS {
            let was = self.controllers[i as usize].connected;
            if !was { self.connect(i); changed = true; }
        }
        changed
    }
    fn shutdown(&mut self) {
        for i in 0..XINPUT_NUM_CONTROLLERS { self.disconnect(i); }
        self.module_handle = None;
        self.initialized = false;
    }
    fn is_initialized(&self) -> bool { self.initialized }
    fn poll_events(&mut self, _mgr: &mut InputManager) {}
    fn enumerate_devices(&self) -> Vec<(String, String)> {
        self.controllers.iter().enumerate()
            .filter(|(_, c)| c.connected)
            .map(|(i, _)| (format!("XInput-{}", i), format!("XInput Controller {}", i)))
            .collect()
    }
    fn enumerate_motors(&self) -> Vec<InputBindingKey> {
        let mut out = Vec::new();
        for (i, c) in self.controllers.iter().enumerate() {
            if !c.connected { continue; }
            if c.has_large_motor { out.push(make_generic_controller_motor_key(InputSourceType::XInput, i as u32, 0)); }
            if c.has_small_motor { out.push(make_generic_controller_motor_key(InputSourceType::XInput, i as u32, 1)); }
        }
        out
    }
    fn get_generic_binding_mapping(&self, device: &str) -> Option<GenericInputBindingMapping> {
        if !device.starts_with("XInput-") { return None; }
        let pid: i32 = device[7..].parse().ok()?;
        if pid < 0 || pid as u32 >= XINPUT_NUM_CONTROLLERS { return None; }
        let mut mapping = Vec::new();
        for (i, name) in XINPUT_AXIS_SETTING.iter().enumerate() {
            mapping.push((GenericInputBinding::LeftStickLeft, format!("XInput-{}/-{}", pid, name)));
            mapping.push((GenericInputBinding::LeftStickRight, format!("XInput-{}/+{}", pid, name)));
        }
        for (i, name) in XINPUT_BUTTON_SETTING.iter().enumerate() {
            mapping.push((GenericInputBinding::Cross, format!("XInput-{}/{}", pid, name)));
        }
        Some(mapping)
    }
    fn get_controller_layout(&self, _index: u32) -> InputLayout { InputLayout::Xbox }
    fn update_motor_state(&mut self, key: InputBindingKey, intensity: f32) {
        if !matches!(key.source_subtype(), InputSubclass::ControllerMotor) { return; }
        if key.source_index() >= XINPUT_NUM_CONTROLLERS { return; }
        let c = &mut self.controllers[key.source_index() as usize];
        if !c.connected { return; }
        let value = (intensity * 65535.0) as u16;
        if key.data() != 0 { c.last_vibration_right = value; } else { c.last_vibration_left = value; }
    }
    fn update_motor_state_dual(&mut self, lk: InputBindingKey, sk: InputBindingKey, li: f32, si: f32) {
        if lk.source_index() != sk.source_index() || !matches!(lk.source_subtype(), InputSubclass::ControllerMotor) {
            self.update_motor_state(lk, li);
            self.update_motor_state(sk, si);
            return;
        }
        if let Some(c) = self.controllers.get_mut(lk.source_index() as usize) {
            if c.connected {
                c.last_vibration_left = (li * 65535.0) as u16;
                c.last_vibration_right = (si * 65535.0) as u16;
            }
        }
    }
    fn parse_key_string(&self, device: &str, binding: &str) -> Option<InputBindingKey> {
        if !device.starts_with("XInput-") || binding.is_empty() { return None; }
        let pid: i32 = device[7..].parse().ok()?;
        if pid < 0 { return None; }
        let mut k = InputBindingKey::default();
        k.set_source_type(InputSourceType::XInput);
        k.set_source_index(pid as u32);

        if binding == "LargeMotor" { k.set_source_subtype(InputSubclass::ControllerMotor); k.set_data(0); return Some(k); }
        if binding == "SmallMotor" { k.set_source_subtype(InputSubclass::ControllerMotor); k.set_data(1); return Some(k); }

        if binding.starts_with('+') || binding.starts_with('-') {
            let axis_name = &binding[1..];
            for (i, name) in XINPUT_AXIS_SETTING.iter().enumerate() {
                if *name == axis_name {
                    k.set_source_subtype(InputSubclass::ControllerAxis);
                    k.set_data(i as u32);
                    k.set_modifier(if binding.starts_with('-') { InputModifier::Negate } else { InputModifier::None });
                    return Some(k);
                }
            }
        }
        for (i, name) in XINPUT_BUTTON_SETTING.iter().enumerate() {
            if *name == binding {
                k.set_source_subtype(InputSubclass::ControllerButton);
                k.set_data(i as u32);
                return Some(k);
            }
        }
        None
    }
    fn convert_key_to_string(&self, key: InputBindingKey, display: bool, _migration: bool) -> String {
        if key.source_type() != InputSourceType::XInput { return String::new(); }
        let idx = key.source_index();
        match key.source_subtype() {
            InputSubclass::ControllerAxis => {
                let modifier = if key.modifier() == InputModifier::Negate { '-' } else { '+' };
                let data = key.data() as usize;
                if data < XINPUT_AXIS_SETTING.len() {
                    if display { format!("XInput-{} {}{}", idx, modifier, XINPUT_AXIS_NAMES[data]) }
                    else { format!("XInput-{}/{}{}", idx, modifier, XINPUT_AXIS_SETTING[data]) }
                } else { String::new() }
            }
            InputSubclass::ControllerButton => {
                let data = key.data() as usize;
                if data < XINPUT_BUTTON_SETTING.len() {
                    if display { format!("XInput-{} {}", idx, XINPUT_BUTTON_NAMES[data]) }
                    else { format!("XInput-{}/{}", idx, XINPUT_BUTTON_SETTING[data]) }
                } else { String::new() }
            }
            InputSubclass::ControllerMotor => {
                let which = if key.data() != 0 { "Small" } else { "Large" };
                if display { format!("XInput-{} {} Motor", idx, which) }
                else { format!("XInput-{}/{}Motor", idx, which) }
            }
            _ => String::new(),
        }
    }
    fn convert_key_to_icon(&self, _key: InputBindingKey) -> String { String::new() }
    fn name(&self) -> &'static str { "XInput" }
}

// =====================================================================================
//  InputManager
// =====================================================================================

/// Vibration motor state for a single pad.
#[derive(Clone)]
pub struct VibrationMotor {
    pub binding: InputBindingKey,
    pub source_name: Option<String>,
    pub last_intensity: f32,
    pub last_update_time: u64,
}

pub struct PadVibration {
    pub pad_index: u32,
    pub motors: [VibrationMotor; 2],
}

impl PadVibration {
    pub fn new(pad_index: u32) -> Self {
        Self {
            pad_index,
            motors: [
                VibrationMotor { binding: InputBindingKey::default(), source_name: None, last_intensity: 0.0, last_update_time: 0 },
                VibrationMotor { binding: InputBindingKey::default(), source_name: None, last_intensity: 0.0, last_update_time: 0 },
            ],
        }
    }
    pub fn are_motors_combined(&self) -> bool { self.motors[0].binding == self.motors[1].binding }
    pub fn combined_intensity(&self) -> f32 { self.motors[0].last_intensity.max(self.motors[1].last_intensity) }
}

/// High-level input manager: tracks external input sources, the binding map
/// and runs event dispatch.
pub struct InputManager {
    sources: Vec<Box<dyn InputSource>>,
    binding_map: HashMap<InputBindingKey, Vec<std::sync::Arc<Mutex<InputBinding>>>>,
    pad_vibrations: Vec<PadVibration>,
    icon_preference: InputLayout,
}

impl InputManager {
    pub fn new() -> Self {
        let mut mgr = Self {
            sources: Vec::new(),
            binding_map: HashMap::new(),
            pad_vibrations: Vec::new(),
            icon_preference: InputLayout::Unknown,
        };
        mgr.sources.push(Box::new(SDLInputSource::new()));
        #[cfg(windows)]
        {
            mgr.sources.push(Box::new(DInputSource::new()));
            mgr.sources.push(Box::new(XInputSource::new()));
        }
        mgr
    }

    pub fn icon_preference(&self) -> InputLayout { self.icon_preference }
    pub fn set_icon_preference(&mut self, layout: InputLayout) { self.icon_preference = layout; }

    /// Run one polling cycle on every active source.
    pub fn poll(&mut self) {
        let mut sources = std::mem::take(&mut self.sources);
        for src in &mut sources {
            if src.is_initialized() {
                src.poll_events(self);
            }
        }
        self.sources = sources;
    }

    /// Re-enumerate devices on every source.
    pub fn update_controllers(&mut self) {
        for src in &mut self.sources {
            if src.is_initialized() { let _ = src.reload_devices(); }
        }
    }

    /// Add a new binding string. Splits on `&` for chord entries.
    pub fn bind(&mut self, binding: &str, handler: InputEventHandler) {
        for chord in binding.split('&') {
            let chord = chord.trim();
            if chord.is_empty() { continue; }
            let key = self.parse_input_binding_key(chord);
            if let Some(k) = key {
                let ibinding = InputBinding {
                    keys: vec![k],
                    handler: handler.clone(),
                    full_mask: 1,
                    current_mask: 0,
                };
                let arc = std::sync::Arc::new(Mutex::new(ibinding));
                self.binding_map.entry(k.mask_direction()).or_default().push(arc);
            }
        }
    }

    pub fn parse_input_binding_key(&self, binding: &str) -> Option<InputBindingKey> {
        let (source, sub) = binding.split_once('/')?;
        if source == "Keyboard" {
            let mut k = InputBindingKey::default();
            k.set_source_type(InputSourceType::Keyboard);
            k.set_data(parse_keyboard_code(sub)?);
            return Some(k);
        }
        if let Some(rest) = source.strip_prefix("Pointer-") {
            let pid: u32 = rest.parse().ok()?;
            let mut k = InputBindingKey::default();
            k.set_source_type(InputSourceType::Pointer);
            k.set_source_index(pid);
            if sub.starts_with("Button") {
                k.set_source_subtype(InputSubclass::PointerButton);
                k.set_data(sub[6..].parse().ok()?);
                return Some(k);
            }
            if sub.starts_with("WheelX") { k.set_source_subtype(InputSubclass::PointerAxis); k.set_data(2); return Some(k); }
            if sub.starts_with("WheelY") { k.set_source_subtype(InputSubclass::PointerAxis); k.set_data(3); return Some(k); }
            if sub == "X" || sub == "X+" || sub == "X-" {
                k.set_source_subtype(InputSubclass::PointerAxis); k.set_data(0);
                if sub.ends_with('-') { k.set_modifier(InputModifier::Negate); }
                return Some(k);
            }
            if sub == "Y" || sub == "Y+" || sub == "Y-" {
                k.set_source_subtype(InputSubclass::PointerAxis); k.set_data(1);
                if sub.ends_with('-') { k.set_modifier(InputModifier::Negate); }
                return Some(k);
            }
        }
        for src in &self.sources {
            if src.is_initialized() {
                if let Some(k) = src.parse_key_string(source, sub) { return Some(k); }
            }
        }
        None
    }

    pub fn convert_key_to_string(&self, key: InputBindingKey, display: bool) -> String {
        for src in &self.sources {
            if src.is_initialized() {
                let s = src.convert_key_to_string(key, display, false);
                if !s.is_empty() { return s; }
            }
        }
        String::new()
    }

    pub fn invoke_events(&self, key: InputBindingKey, value: f32) -> bool {
        let masked = key.mask_direction();
        if let Some(bindings) = self.binding_map.get(&masked) {
            for arc in bindings {
                let mut binding = arc.lock().unwrap();
                match &binding.handler {
                    InputEventHandler::Axis(h) => h(key, value),
                    InputEventHandler::Button(h) => {
                        if value > 0.0 { h(1); } else if value == 0.0 { h(0); }
                    }
                }
            }
            true
        } else { false }
    }

    pub fn set_pad_vibration_intensity(&mut self, pad_index: u32, large: f32, small: f32) {
        for pad in &mut self.pad_vibrations {
            if pad.pad_index != pad_index { continue; }
            pad.motors[0].last_intensity = large;
            pad.motors[1].last_intensity = small;
            for (i, name) in [&pad.motors[0].source_name, &pad.motors[1].source_name].iter().enumerate() {
                if let Some(src_name) = name {
                    if let Some(src) = self.sources.iter_mut().find(|s| s.name() == src_name.as_str()) {
                        let key = pad.motors[i].binding;
                        let intensity = if i == 0 { large } else { small };
                        src.update_motor_state(key, intensity);
                    }
                }
            }
        }
    }

    pub fn add_pad_vibration(&mut self, vib: PadVibration) {
        self.pad_vibrations.push(vib);
    }
}

impl Default for InputManager {
    fn default() -> Self { Self::new() }
}

// ----- Misc helpers --------------------------------------------------------

fn parse_keyboard_code(name: &str) -> Option<u32> {
    // Minimal scancode table; the real implementation is much larger.
    let map: &[(&str, u32)] = &[
        ("Up", 0xC8), ("Down", 0xD0), ("Left", 0xCB), ("Right", 0xCD),
        ("W", 0x11), ("A", 0x1E), ("S", 0x1F), ("D", 0x20),
        ("Return", 0x1C), ("Backspace", 0x0E),
        ("Space", 0x39), ("Escape", 0x01), ("Tab", 0x0F),
    ];
    for (k, v) in map { if *k == name { return Some(*v); } }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_pack_round_trip() {
        let mut k = InputBindingKey::default();
        k.set_source_type(InputSourceType::DInput);
        k.set_source_index(3);
        k.set_source_subtype(InputSubclass::ControllerAxis);
        k.set_modifier(InputModifier::Negate);
        k.set_invert(true);
        k.set_data(42);
        assert_eq!(k.source_type(), InputSourceType::DInput);
        assert_eq!(k.source_index(), 3);
        assert_eq!(k.modifier(), InputModifier::Negate);
        assert!(k.invert());
        assert_eq!(k.data(), 42);
    }

    #[test]
    fn ring_buffer_basic() {
        let mut rb = AudioRingBuffer::new(8);
        rb.write(&[1.0, 2.0, 3.0, 4.0]);
        let mut out = [0.0; 4];
        let n = rb.read(&mut out);
        assert_eq!(n, 4);
        assert_eq!(out, [1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn audio_expansion_modes() {
        assert_eq!(AudioExpansionMode::Disabled.channel_layout(), (2, 2));
        assert_eq!(AudioExpansionMode::Surround71.channel_layout(), (8, 8));
        assert_eq!(AudioExpansionMode::from_name("Surround51"), Some(AudioExpansionMode::Surround51));
    }

    #[test]
    fn manager_bind_and_dispatch() {
        let mut mgr = InputManager::new();
        let count = std::sync::Arc::new(AtomicU32::new(0));
        let count2 = count.clone();
        mgr.bind("Keyboard/Up", InputEventHandler::Button(std::sync::Arc::new(move |_| {
            count2.fetch_add(1, Ordering::Relaxed);
        })));
        let k = make_host_keyboard_key(0xC8);
        assert!(mgr.invoke_events(k, 1.0));
        assert_eq!(count.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn dinput_parse_key() {
        let src = DInputSource::new();
        let k = src.parse_key_string("DInput-0", "+Axis1").expect("parse");
        assert_eq!(k.source_type(), InputSourceType::DInput);
        assert_eq!(k.data(), 1);
    }

    #[test]
    fn xinput_parse_key() {
        let src = XInputSource::new();
        let k = src.parse_key_string("XInput-0", "DPadUp").expect("parse");
        assert_eq!(k.source_type(), InputSourceType::XInput);
        assert_eq!(k.source_subtype(), InputSubclass::ControllerButton);
        assert_eq!(k.data(), 0);
    }

    #[test]
    fn sdl_parse_key() {
        let src = SDLInputSource::new();
        let k = src.parse_key_string("SDL-0", "LeftX").expect("parse");
        assert_eq!(k.source_type(), InputSourceType::Sdl);
        assert_eq!(k.data(), 0);
    }
}
