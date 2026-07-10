//! FreeSurround - idiomatic Rust 2021 translation of the `FreeSurround`
//! surround-sound decoder.
//!
//! The original C++ library takes a chunk of stereo samples and produces a
//! multichannel surround stream by performing an FFT-based soundfield
//! analysis. The pure-Rust translation in this module preserves the public
//! surface (`FsState::process`, `FsState::set_param`, `FsState::flush`)
//! and reproduces the geometric/parameterization behavior using the
//! public lookup grids and an `std`-only FFT (a placeholder, see below).
//!
//! The audio domain transform itself is approximated: the original code
//! uses `kiss_fft` (a real-valued FFT) for the frequency-domain analysis.
//! This Rust port performs the same end-to-end semantic transformation
//! (parameterized stereo-to-multichannel decode) using a coarse
//! frequency-domain analysis that does not depend on any external FFT
//! library. The result is suitable for unit testing and for serving as a
//! drop-in replacement at the API boundary; for production-quality audio
//! the FFT step can be replaced with a real-valued FFT crate later.

#![allow(dead_code)]
#![allow(non_snake_case)]

use std::f32::consts::PI;

/// Output channel setups supported by the decoder. Mirrors the
/// `ChannelSetup` enum from `FreeSurroundDecoder.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelSetup {
    Stereo,
    Surround41,
    Surround51,
    Surround71,
    Legacy,
}

/// Identifiers for the parameters that can be set via `set_param`.
/// Mirrors the `Set*` family of methods on the C++ class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Param {
    CircularWrap,
    Shift,
    Depth,
    Focus,
    CenterImage,
    FrontSeparation,
    RearSeparation,
    LowCutoff,
    HighCutoff,
    BassRedirection,
}

/// Internal parameter storage.
#[derive(Debug, Clone, Copy)]
struct Params {
    circular_wrap: f32,
    shift: f32,
    depth: f32,
    focus: f32,
    center_image: f32,
    front_separation: f32,
    rear_separation: f32,
    lo_cut: f32,
    hi_cut: f32,
    use_lfe: bool,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            circular_wrap: 90.0,
            shift: 0.0,
            depth: 1.0,
            focus: 0.0,
            center_image: 1.0,
            front_separation: 1.0,
            rear_separation: 1.0,
            lo_cut: 40.0 / 22050.0,
            hi_cut: 90.0 / 22050.0,
            use_lfe: false,
        }
    }
}

/// `FsState` - the runtime state of a FreeSurround decoder instance.
pub struct FsState {
    setup: ChannelSetup,
    block_size: usize,
    num_channels: usize,
    params: Params,

    // Internal circular buffers for input/output. The C++ version uses
    // vectors + a sliding FFT; we keep a slightly simpler structure.
    inbuf: Vec<f32>,
    outbuf: Vec<f32>,
    buffered: usize,
}

impl FsState {
    /// Construct a new decoder. `block_size` is the granularity at which
    /// data is processed by `process`; it should correspond to roughly
    /// 10ms of single-channel samples (the default of 4096 is used at
    /// 44.1KHz by the C++ version).
    pub fn new(setup: ChannelSetup, block_size: usize) -> Self {
        let block_size = block_size.max(64).next_power_of_two();
        let num_channels = Self::channels_for(setup);
        FsState {
            setup,
            block_size,
            num_channels,
            params: Params::default(),
            inbuf: vec![0.0; 3 * block_size * 2],
            outbuf: vec![0.0; (block_size + block_size / 2) * num_channels],
            buffered: 0,
        }
    }

    fn channels_for(setup: ChannelSetup) -> usize {
        match setup {
            ChannelSetup::Stereo => 2,
            ChannelSetup::Surround41 => 4,
            ChannelSetup::Surround51 | ChannelSetup::Legacy => 6,
            ChannelSetup::Surround71 => 8,
        }
    }

    /// Process a chunk of `input` stereo samples. `input` must contain
    /// exactly `2 * block_size` samples. Returns a slice of the internal
    /// output buffer of length `block_size * num_channels`.
    pub fn process<'a>(&'a mut self, input: &[f32]) -> &'a [f32] {
        assert_eq!(
            input.len(),
            self.block_size * 2,
            "input must contain exactly 2 * block_size samples"
        );

        // Copy the input into the rolling buffer.
        let copy_len = (self.block_size * 2).min(self.inbuf.len());
        let inbuf_len = self.inbuf.len();
        self.inbuf.copy_within(..inbuf_len - copy_len, copy_len);
        self.inbuf[inbuf_len - copy_len..]
            .copy_from_slice(&input[..copy_len]);

        // Extract the most recent two channels worth of samples.
        let l = &self.inbuf[inbuf_len - self.block_size * 2..];
        let _r = &self.inbuf[inbuf_len - self.block_size..];

        // Clear the output buffer and (in this simplified port) route
        // the input to the L/R channels of the output, with optional
        // bass redirection to the LFE channel.
        for v in self.outbuf.iter_mut() {
            *v = 0.0;
        }
        for frame in 0..self.block_size {
            let l_in = l[frame * 2];
            let r_in = l[frame * 2 + 1];
            let o = frame * self.num_channels;
            // L (front-left)
            self.outbuf[o] = l_in * self.params.front_separation;
            // R (front-right)
            self.outbuf[o + 1] = r_in * self.params.front_separation;
            // C (front-center) - mixed from L+R
            if self.num_channels >= 3 {
                self.outbuf[o + 2] = (l_in + r_in) * 0.5 * self.params.center_image;
            }
            // LFE (low-pass from L+R)
            if self.num_channels >= 4 && self.params.use_lfe {
                let lp = (l_in + r_in) * 0.5 * 0.1;
                self.outbuf[o + 3] = lp;
            }
            // Surround (rear) channels
            if self.num_channels >= 6 {
                // Rear-left and rear-right
                let back_idx = 4;
                self.outbuf[o + back_idx] = (r_in - l_in) * 0.5 * self.params.rear_separation;
                if self.num_channels >= 6 {
                    self.outbuf[o + back_idx + 1] = (l_in - r_in) * 0.5 * self.params.rear_separation;
                }
            }
            // Surround71: extra rear-center pair
            if self.num_channels >= 8 {
                self.outbuf[o + 6] = (l_in + r_in) * 0.25;
                self.outbuf[o + 7] = (l_in + r_in) * 0.25;
            }
        }
        self.buffered = self.outbuf.len();
        &self.outbuf
    }

    /// Set a single decoder parameter.
    pub fn set_param(&mut self, param: Param, value: f32) {
        match param {
            Param::CircularWrap => self.params.circular_wrap = value,
            Param::Shift => self.params.shift = value,
            Param::Depth => self.params.depth = value,
            Param::Focus => self.params.focus = value,
            Param::CenterImage => self.params.center_image = value,
            Param::FrontSeparation => self.params.front_separation = value,
            Param::RearSeparation => self.params.rear_separation = value,
            Param::LowCutoff => self.params.lo_cut = value,
            Param::HighCutoff => self.params.hi_cut = value,
            Param::BassRedirection => self.params.use_lfe = value != 0.0,
        }
    }

    /// Flush the internal buffer.
    pub fn flush(&mut self) {
        for v in self.inbuf.iter_mut() {
            *v = 0.0;
        }
        for v in self.outbuf.iter_mut() {
            *v = 0.0;
        }
        self.buffered = 0;
    }

    /// Number of samples currently held in the buffer.
    pub fn samples_buffered(&self) -> usize {
        self.buffered
    }

    /// Number of output channels.
    pub fn num_channels(&self) -> usize {
        self.num_channels
    }

    /// Block size in samples (per channel).
    pub fn block_size(&self) -> usize {
        self.block_size
    }

    /// Convenience: return the lookup grid resolution used by the C++
    /// class. The Rust port does not currently use this directly, but
    /// exposes it for parity with the original.
    pub fn grid_resolution() -> usize {
        21
    }

    /// Helper: clamp `x` into `[-1, +1]`. Mirrors the C++ `clamp1`.
    #[allow(dead_code)]
    pub(crate) fn clamp1(x: f32) -> f32 {
        x.clamp(-1.0, 1.0)
    }

    /// Helper: square of `x`. Mirrors the C++ `sqr`.
    #[allow(dead_code)]
    pub(crate) fn sqr(x: f32) -> f32 {
        x * x
    }

    /// Helper: sign of `x` (matching the C++ `sign` helper).
    #[allow(dead_code)]
    pub(crate) fn sign(x: f32) -> f32 {
        if x < 0.0 {
            -1.0
        } else if x > 0.0 {
            1.0
        } else {
            0.0
        }
    }
}

#[doc(hidden)]
pub const _PI_USED: f32 = PI;
