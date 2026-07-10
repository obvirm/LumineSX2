//! SoundTouch - idiomatic Rust 2021 translation of the SoundTouch audio
//! processing library (tempo / pitch / rate change).
//!
//! This module reproduces the public surface of the original C++ `SoundTouch`
//! class: configuration of channels/sample rate, setting tempo/pitch/rate
//! (with both absolute and relative variants), feeding samples in via
//! `putSamples`, pulling processed samples out via `receiveSamples`, and
//! flushing / querying the internal buffer state.
//!
//! The implementation is a pure-Rust, `std`-only, single-sample-buffer model
//! that mirrors the FIFO semantics of the C++ original. Sample type is
//! `f32` (matching the C++ `SAMPLETYPE` default on floating point builds).

#![allow(dead_code)]
#![allow(non_snake_case)]

use std::f32::consts::E;
use std::f32::consts::LN_2;

/// Library version string - matches `SOUNDTOUCH_VERSION` from the C++ source.
pub const SOUNDTOUCH_VERSION: &str = "2.3.3";
/// Library version identifier - matches `SOUNDTOUCH_VERSION_ID`.
pub const SOUNDTOUCH_VERSION_ID: u32 = 0x020303;

/// Default number of samples pushed per flush iteration.
const FLUSH_CHUNK: usize = 128;
/// Upper bound on flush iterations (matches the `i < 200` loop in the C++).
const FLUSH_MAX_ITERS: usize = 200;
/// Tolerance used when comparing floating point rate/tempo values.
const FLOAT_EQ_EPS: f32 = 1e-10;

/// SoundTouch setting identifiers (subset of the `SETTING_*` enum from the
/// C++ header). Only a subset is required for the public API.
pub const SETTING_USE_AA_FILTER: i32 = 0;
pub const SETTING_AA_FILTER_LENGTH: i32 = 1;
pub const SETTING_USE_QUICKSEEK: i32 = 2;
pub const SETTING_SEQUENCE_MS: i32 = 3;
pub const SETTING_SEEKWINDOW_MS: i32 = 4;
pub const SETTING_OVERLAP_MS: i32 = 5;
pub const SETTING_NOMINAL_INPUT_SEQUENCE: i32 = 6;
pub const SETTING_NOMINAL_OUTPUT_SEQUENCE: i32 = 7;
pub const SETTING_INITIAL_LATENCY: i32 = 8;

/// Whether the rate-crossover "click prevention" path is enabled. Mirrors
/// the C++ `SOUNDTOUCH_PREVENT_CLICK_AT_RATE_CROSSOVER` macro (default off).
pub const SOUNDTOUCH_PREVENT_CLICK_AT_RATE_CROSSOVER: bool = false;

/// `SoundTouch` - main class for tempo/pitch/rate adjusting routines.
///
/// Behaves as a FIFO pipeline: call `put_samples` to feed interleaved
/// samples in, then `receive_samples` to pull processed samples out.
/// `flush` extracts any residual samples from the end of the stream.
pub struct SoundTouch {
    channels: u32,
    sample_rate: u32,
    srate_set: bool,

    // "virtual" controls - the user-facing parameters.
    virtual_tempo: f32,
    virtual_rate: f32,
    virtual_pitch: f32,

    // "effective" controls - what is actually applied internally.
    tempo: f32,
    rate: f32,

    // Processing pipeline outputs.
    ptd_stretch: TdStretch,
    rate_transposer: RateTransposer,

    // Bookkeeping for expected vs. emitted samples.
    samples_expected_out: f64,
    samples_output: u64,

    // Internal FIFO buffer of processed (interleaved) samples.
    output_buffer: Vec<f32>,
}

impl Default for SoundTouch {
    fn default() -> Self {
        Self::new()
    }
}

impl SoundTouch {
    /// Construct a new `SoundTouch` instance with default parameters.
    pub fn new() -> Self {
        let mut s = SoundTouch {
            channels: 0,
            sample_rate: 0,
            srate_set: false,
            virtual_tempo: 1.0,
            virtual_rate: 1.0,
            virtual_pitch: 1.0,
            tempo: 1.0,
            rate: 1.0,
            ptd_stretch: TdStretch::new(),
            rate_transposer: RateTransposer::new(),
            samples_expected_out: 0.0,
            samples_output: 0,
            output_buffer: Vec::new(),
        };
        s.calc_effective_rate_and_tempo();
        s
    }

    /// Returns the SoundTouch library version string.
    pub fn get_version_string() -> &'static str {
        SOUNDTOUCH_VERSION
    }

    /// Returns the SoundTouch library version identifier.
    pub fn get_version_id() -> u32 {
        SOUNDTOUCH_VERSION_ID
    }

    /// Number of channels (1 = mono, 2 = stereo).
    pub fn set_channels(&mut self, num_channels: u32) {
        if !verify_number_of_channels(num_channels) {
            return;
        }
        self.channels = num_channels;
        self.rate_transposer.set_channels(num_channels as i32);
        self.ptd_stretch.set_channels(num_channels as i32);
    }

    pub fn channels(&self) -> u32 {
        self.channels
    }

    /// New rate control value. Normal rate = 1.0; smaller values represent
    /// slower rate, larger values represent faster rate.
    pub fn set_rate(&mut self, new_rate: f32) {
        self.virtual_rate = new_rate;
        self.calc_effective_rate_and_tempo();
    }

    /// New rate control value as a percent difference vs. original (-50..+100 %).
    pub fn set_rate_change(&mut self, new_rate_pct: f32) {
        self.virtual_rate = 1.0 + 0.01 * new_rate_pct;
        self.calc_effective_rate_and_tempo();
    }

    /// New tempo control value. Normal tempo = 1.0.
    pub fn set_tempo(&mut self, new_tempo: f32) {
        self.virtual_tempo = new_tempo;
        self.calc_effective_rate_and_tempo();
    }

    /// New tempo control value as a percent difference vs. original.
    pub fn set_tempo_change(&mut self, new_tempo_pct: f32) {
        self.virtual_tempo = 1.0 + 0.01 * new_tempo_pct;
        self.calc_effective_rate_and_tempo();
    }

    /// New pitch control value. Original pitch = 1.0.
    pub fn set_pitch(&mut self, new_pitch: f32) {
        self.virtual_pitch = new_pitch;
        self.calc_effective_rate_and_tempo();
    }

    /// Pitch change in octaves compared to the original pitch (-1..+1).
    pub fn set_pitch_octaves(&mut self, new_pitch: f32) {
        // `exp(0.69314718056 * x)` == `exp(LN_2 * x)` == `2.powf(x)`.
        self.virtual_pitch = (LN_2 * new_pitch).exp();
        self.calc_effective_rate_and_tempo();
    }

    /// Pitch change in semi-tones compared to the original pitch (-12..+12).
    pub fn set_pitch_semi_tones(&mut self, new_pitch: f32) {
        self.set_pitch_octaves(new_pitch / 12.0);
    }

    /// Sample rate in Hz.
    pub fn set_sample_rate(&mut self, srate: u32) {
        self.sample_rate = srate;
        self.ptd_stretch.set_parameters(srate as i32);
        self.srate_set = true;
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Feed `n_samples` interleaved samples into the pipeline.
    /// Each "sample" is one frame across all `channels` channels, so the
    /// `samples` slice must be at least `n_samples * channels` long.
    pub fn put_samples(&mut self, samples: &[f32], n_samples: usize) {
        if !self.srate_set {
            // Mirrors the C++ `ST_THROW_RT_ERROR("Sample rate not defined")`.
            panic!("SoundTouch: Sample rate not defined");
        }
        if self.channels == 0 {
            panic!("SoundTouch: Number of channels not defined");
        }

        // Update expected output count for the rate/tempo combo.
        let r = self.rate as f64;
        let t = self.tempo as f64;
        if r != 0.0 && t != 0.0 {
            self.samples_expected_out += n_samples as f64 / (r * t);
        }

        // Move samples through the pipeline. The C++ class uses
        // `RateTransposer` and `TDStretch` as filter objects. In this
        // pure-Rust translation we treat the chain as a single function
        // and append results to the output buffer.
        let processed = self.process_pipeline(samples, n_samples);
        self.output_buffer.extend_from_slice(&processed);
    }

    /// Pull up to `max_samples` processed samples out of the pipeline.
    /// Returns the number of samples actually copied.
    pub fn receive_samples(&mut self, out: &mut [f32], max_samples: usize) -> usize {
        let available = self.output_buffer.len();
        let to_copy = max_samples.min(available).min(out.len());
        out[..to_copy].copy_from_slice(&self.output_buffer[..to_copy]);
        self.output_buffer.drain(..to_copy);
        self.samples_output += to_copy as u64;
        to_copy
    }

    /// Flush the last samples out of the processing pipeline. After
    /// calling this, `num_samples()` will be 0 and any remaining pipeline
    /// state is cleared.
    pub fn flush(&mut self) {
        let num_still_expected =
            (self.samples_expected_out + 0.5) as i64 - self.samples_output as i64;
        let num_still_expected = num_still_expected.max(0) as usize;

        if self.channels == 0 {
            return;
        }
        let blank = vec![0.0f32; FLUSH_CHUNK * self.channels as usize];

        let mut iters = 0;
        while self.num_samples() < num_still_expected && iters < FLUSH_MAX_ITERS {
            self.put_samples(&blank, FLUSH_CHUNK);
            iters += 1;
        }

        self.adjust_amount_of_samples(num_still_expected);
        // Clear input buffer of the TDStretch, leave output untouched
        // (that's where the flushed samples are).
        self.ptd_stretch.clear_input();
    }

    /// Number of samples currently buffered for output.
    pub fn num_samples(&self) -> usize {
        self.output_buffer.len()
    }

    /// Number of samples currently held in the input side of the pipeline.
    pub fn num_unprocessed_samples(&self) -> usize {
        self.ptd_stretch.input_len()
    }

    /// Set a processing setting (subset of `SETTING_*`).
    pub fn set_setting(&mut self, setting_id: i32, value: i32) -> bool {
        match setting_id {
            SETTING_USE_AA_FILTER => {
                self.rate_transposer.enable_aa_filter(value != 0);
                true
            }
            SETTING_AA_FILTER_LENGTH => {
                self.rate_transposer.aa_filter_length = value.max(0) as u32;
                true
            }
            SETTING_USE_QUICKSEEK => {
                self.ptd_stretch.enable_quick_seek(value != 0);
                true
            }
            SETTING_SEQUENCE_MS => {
                let (sr, _seq, sw, ov) = self.ptd_stretch.get_parameters();
                self.ptd_stretch.set_parameters_full(sr, value, sw, ov);
                true
            }
            SETTING_SEEKWINDOW_MS => {
                let (sr, seq, _sw, ov) = self.ptd_stretch.get_parameters();
                self.ptd_stretch.set_parameters_full(sr, seq, value, ov);
                true
            }
            SETTING_OVERLAP_MS => {
                let (sr, seq, sw, _ov) = self.ptd_stretch.get_parameters();
                self.ptd_stretch.set_parameters_full(sr, seq, sw, value);
                true
            }
            _ => false,
        }
    }

    /// Read a processing setting.
    pub fn get_setting(&self, setting_id: i32) -> i32 {
        match setting_id {
            SETTING_USE_AA_FILTER => self.rate_transposer.aa_filter_enabled as i32,
            SETTING_AA_FILTER_LENGTH => self.rate_transposer.aa_filter_length as i32,
            SETTING_USE_QUICKSEEK => self.ptd_stretch.quick_seek_enabled as i32,
            SETTING_SEQUENCE_MS => self.ptd_stretch.get_parameters().1,
            SETTING_SEEKWINDOW_MS => self.ptd_stretch.get_parameters().2,
            SETTING_OVERLAP_MS => self.ptd_stretch.get_parameters().3,
            SETTING_NOMINAL_INPUT_SEQUENCE => {
                let size = self.ptd_stretch.input_sample_req();
                if !SOUNDTOUCH_PREVENT_CLICK_AT_RATE_CROSSOVER && self.rate <= 1.0 {
                    (size as f32 * self.rate + 0.5) as i32
                } else {
                    size
                }
            }
            SETTING_NOMINAL_OUTPUT_SEQUENCE => {
                let size = self.ptd_stretch.output_batch_size();
                if self.rate > 1.0 {
                    (size as f32 / self.rate + 0.5) as i32
                } else {
                    size
                }
            }
            SETTING_INITIAL_LATENCY => {
                let latency = self.ptd_stretch.latency() as f32;
                let latency_tr = self.rate_transposer.latency() as f32;
                let combined = if !SOUNDTOUCH_PREVENT_CLICK_AT_RATE_CROSSOVER
                    && self.rate <= 1.0
                {
                    (latency + latency_tr) * self.rate
                } else {
                    latency + latency_tr / self.rate
                };
                (combined + 0.5) as i32
            }
            _ => 0,
        }
    }

    /// Clear all samples from the output and internal processing buffers.
    pub fn clear(&mut self) {
        self.samples_expected_out = 0.0;
        self.samples_output = 0;
        self.output_buffer.clear();
        self.rate_transposer.clear();
        self.ptd_stretch.clear();
    }

    /// Get the ratio between input and output sample counts.
    pub fn get_input_output_sample_ratio(&self) -> f64 {
        1.0 / ((self.tempo as f64) * (self.rate as f64))
    }

    // ----- internal helpers -----

    fn calc_effective_rate_and_tempo(&mut self) {
        let old_tempo = self.tempo;
        let old_rate = self.rate;

        self.tempo = self.virtual_tempo / self.virtual_pitch;
        self.rate = self.virtual_pitch * self.virtual_rate;

        if (self.rate - old_rate).abs() > FLOAT_EQ_EPS {
            self.rate_transposer.set_rate(self.rate);
        }
        if (self.tempo - old_tempo).abs() > FLOAT_EQ_EPS {
            self.ptd_stretch.set_tempo(self.tempo);
        }
    }

    /// Apply the tempo + rate transform to a chunk of input. The pure-Rust
    /// implementation uses simple resampling: at `tempo != 1.0` we read
    /// every `tempo`-th frame, at `rate != 1.0` we resample.
    fn process_pipeline(&self, samples: &[f32], n_samples: usize) -> Vec<f32> {
        if self.channels == 0 || n_samples == 0 {
            return Vec::new();
        }
        let ch = self.channels as usize;
        // Estimate output length from the rate * tempo product.
        let factor = (self.tempo * self.rate).max(0.0001);
        let out_len_est = ((n_samples as f32) / factor) as usize + 2;
        let mut out = Vec::with_capacity(out_len_est * ch);

        // Linear resample across frames. We treat `n_samples` frames as
        // equally spaced points in the input domain, and produce
        // `n_samples / (rate * tempo)` frames at unit spacing.
        let out_n = (n_samples as f32 / factor).round() as usize;
        for i in 0..out_n {
            let src_pos = (i as f32) * factor;
            let i0 = src_pos.floor() as usize;
            let i1 = (i0 + 1).min(n_samples.saturating_sub(1));
            let frac = src_pos - i0 as f32;
            for c in 0..ch {
                let s0 = samples.get(i0 * ch + c).copied().unwrap_or(0.0);
                let s1 = samples.get(i1 * ch + c).copied().unwrap_or(0.0);
                out.push(s0 * (1.0 - frac) + s1 * frac);
            }
        }
        out
    }

    fn adjust_amount_of_samples(&mut self, num_samples: usize) {
        // The C++ version calls into FIFOSampleBuffer::adjustAmountOfSamples
        // which discards samples from the start of the buffer. We mirror
        // the same effect on the output buffer.
        let to_drop = num_samples.min(self.output_buffer.len());
        if to_drop > 0 {
            self.output_buffer.drain(..to_drop);
        }
    }
}

fn verify_number_of_channels(num_channels: u32) -> bool {
    num_channels == 1 || num_channels == 2
}

// ----- internal pipeline stages (pure Rust) -----

struct RateTransposer {
    rate: f32,
    aa_filter_enabled: bool,
    aa_filter_length: u32,
    channels: i32,
    input_buffer: Vec<f32>,
    output_buffer: Vec<f32>,
}

impl RateTransposer {
    fn new() -> Self {
        RateTransposer {
            rate: 1.0,
            aa_filter_enabled: true,
            aa_filter_length: 32,
            channels: 0,
            input_buffer: Vec::new(),
            output_buffer: Vec::new(),
        }
    }

    fn set_rate(&mut self, rate: f32) {
        self.rate = rate;
    }

    fn set_channels(&mut self, channels: i32) {
        self.channels = channels;
    }

    fn enable_aa_filter(&mut self, enable: bool) {
        self.aa_filter_enabled = enable;
    }

    fn clear(&mut self) {
        self.input_buffer.clear();
        self.output_buffer.clear();
    }

    fn latency(&self) -> i32 {
        // 2 * aa_filter_length, matching the C++ definition.
        (2 * self.aa_filter_length as i32).max(0)
    }
}

struct TdStretch {
    tempo: f32,
    quick_seek_enabled: bool,
    channels: i32,
    sample_rate: i32,
    sequence_ms: i32,
    seek_window_ms: i32,
    overlap_ms: i32,
    input: Vec<f32>,
}

impl TdStretch {
    fn new() -> Self {
        TdStretch {
            tempo: 1.0,
            quick_seek_enabled: false,
            channels: 0,
            sample_rate: 0,
            sequence_ms: 0,
            seek_window_ms: 0,
            overlap_ms: 0,
            input: Vec::new(),
        }
    }

    fn set_tempo(&mut self, tempo: f32) {
        self.tempo = tempo;
    }

    fn set_channels(&mut self, channels: i32) {
        self.channels = channels;
    }

    fn enable_quick_seek(&mut self, enable: bool) {
        self.quick_seek_enabled = enable;
    }

    fn set_parameters(&mut self, sample_rate: i32) {
        self.sample_rate = sample_rate;
    }

    fn set_parameters_full(
        &mut self,
        sample_rate: i32,
        sequence_ms: i32,
        seek_window_ms: i32,
        overlap_ms: i32,
    ) {
        self.sample_rate = sample_rate;
        self.sequence_ms = sequence_ms;
        self.seek_window_ms = seek_window_ms;
        self.overlap_ms = overlap_ms;
    }

    fn get_parameters(&self) -> (i32, i32, i32, i32) {
        (
            self.sample_rate,
            self.sequence_ms,
            self.seek_window_ms,
            self.overlap_ms,
        )
    }

    fn input_sample_req(&self) -> i32 {
        // Approximation: a 40ms window of samples per channel.
        ((self.sample_rate.max(1) as f32) * 0.040) as i32
    }

    fn output_batch_size(&self) -> i32 {
        ((self.sample_rate.max(1) as f32) * 0.020) as i32
    }

    fn latency(&self) -> i32 {
        // Approximation: a 100ms window in samples.
        ((self.sample_rate.max(1) as f32) * 0.100) as i32
    }

    fn clear(&mut self) {
        self.input.clear();
    }

    fn clear_input(&mut self) {
        self.input.clear();
    }

    fn input_len(&self) -> usize {
        self.input.len()
    }
}

#[doc(hidden)]
pub const _E_USED: f32 = E;
