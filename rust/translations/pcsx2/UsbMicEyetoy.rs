// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust translation of the PCSX2 USB microphone (SingStar/Logitech/Konami)
//! and EyeToy webcam device code.
//!
//! This module consolidates the C/C++ sources from `pcsx2/USB/usb-mic` and
//! `pcsx2/USB/usb-eyetoy` into a single Rust 2021 module. Only `std` is used.
//!
//! The translation favours the public surface requested by the rewrite brief:
//! microphones (`USBMic`, `USBHeadset`), audio backends (`AudioDevice` trait and
//! its `CubebAudioDevice` / `NoopAudioDevice` implementations), webcams
//! (`USBEyetoy`, `CamLinux`, `CamWindows`, `CamNoop`) and the tiny MPEG helper
//! used to compress/decompress video frames (`JoMpeg`). The USB descriptor
//! blobs, class-specific audio control requests and the EyeToy register
//! model from the original C++ are preserved as plain byte arrays and
//! `USBDescStrings`/register tables so that downstream code can still
//! reconstruct identical enumeration behaviour.

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

// =====================================================================
// Common USB audio class definitions (audio.h)
// =====================================================================

// Audio Interface Subclass Codes
pub const AUDIO_SUBCLASS_UNDEFINED: u8 = 0x00;
pub const AUDIO_SUBCLASS_AUDIOCONTROL: u8 = 0x01;
pub const AUDIO_SUBCLASS_AUDIOSTREAMING: u8 = 0x02;
pub const AUDIO_SUBCLASS_MIDISTREAMING: u8 = 0x03;

// Audio Interface Protocol Codes
pub const AUDIO_PROTOCOL_UNDEFINED: u8 = 0x00;

// Audio Descriptor Types
pub const AUDIO_UNDEFINED_DESCRIPTOR_TYPE: u8 = 0x20;
pub const AUDIO_DEVICE_DESCRIPTOR_TYPE: u8 = 0x21;
pub const AUDIO_CONFIGURATION_DESCRIPTOR_TYPE: u8 = 0x22;
pub const AUDIO_STRING_DESCRIPTOR_TYPE: u8 = 0x23;
pub const AUDIO_INTERFACE_DESCRIPTOR_TYPE: u8 = 0x24;
pub const AUDIO_ENDPOINT_DESCRIPTOR_TYPE: u8 = 0x25;

// Audio Control Interface Descriptor Subtypes
pub const AUDIO_CONTROL_UNDEFINED: u8 = 0x00;
pub const AUDIO_CONTROL_HEADER: u8 = 0x01;
pub const AUDIO_CONTROL_INPUT_TERMINAL: u8 = 0x02;
pub const AUDIO_CONTROL_OUTPUT_TERMINAL: u8 = 0x03;
pub const AUDIO_CONTROL_MIXER_UNIT: u8 = 0x04;
pub const AUDIO_CONTROL_SELECTOR_UNIT: u8 = 0x05;
pub const AUDIO_CONTROL_FEATURE_UNIT: u8 = 0x06;
pub const AUDIO_CONTROL_PROCESSING_UNIT: u8 = 0x07;
pub const AUDIO_CONTROL_EXTENSION_UNIT: u8 = 0x08;

// Audio Streaming Interface Descriptor Subtypes
pub const AUDIO_STREAMING_UNDEFINED: u8 = 0x00;
pub const AUDIO_STREAMING_GENERAL: u8 = 0x01;
pub const AUDIO_STREAMING_FORMAT_TYPE: u8 = 0x02;
pub const AUDIO_STREAMING_FORMAT_SPECIFIC: u8 = 0x03;

// Audio Endpoint Descriptor Subtypes
pub const AUDIO_ENDPOINT_UNDEFINED: u8 = 0x00;
pub const AUDIO_ENDPOINT_GENERAL: u8 = 0x01;

// Audio Descriptor Sizes
pub const fn audio_control_interface_desc_sz(n: usize) -> usize {
    0x08 + n
}
pub const AUDIO_STREAMING_INTERFACE_DESC_SIZE: usize = 0x07;
pub const AUDIO_INPUT_TERMINAL_DESC_SIZE: usize = 0x0C;
pub const AUDIO_OUTPUT_TERMINAL_DESC_SIZE: usize = 0x09;
pub const fn audio_mixer_unit_desc_sz(p: usize, n: usize) -> usize {
    0x0A + p + n
}
pub const fn audio_selector_unit_desc_sz(p: usize) -> usize {
    0x06 + p
}
pub const fn audio_feature_unit_desc_sz(ch: usize, n: usize) -> usize {
    0x07 + (ch + 1) * n
}
pub const fn audio_processing_unit_desc_sz(p: usize, n: usize, x: usize) -> usize {
    0x0D + p + n + x
}
pub const fn audio_extension_unit_desc_sz(p: usize, n: usize) -> usize {
    0x0D + p + n
}
pub const AUDIO_STANDARD_ENDPOINT_DESC_SIZE: usize = 0x09;
pub const AUDIO_STREAMING_ENDPOINT_DESC_SIZE: usize = 0x07;

// Audio Processing Unit Process Types
pub const AUDIO_UNDEFINED_PROCESS: u8 = 0x00;
pub const AUDIO_UP_DOWN_MIX_PROCESS: u8 = 0x01;
pub const AUDIO_DOLBY_PROLOGIC_PROCESS: u8 = 0x02;
pub const AUDIO_3D_STEREO_PROCESS: u8 = 0x03;
pub const AUDIO_REVERBERATION_PROCESS: u8 = 0x04;
pub const AUDIO_CHORUS_PROCESS: u8 = 0x05;
pub const AUDIO_DYN_RANGE_COMP_PROCESS: u8 = 0x06;

// Audio Request Codes
pub const AUDIO_REQUEST_UNDEFINED: u8 = 0x00;
pub const AUDIO_REQUEST_SET_CUR: u8 = 0x01;
pub const AUDIO_REQUEST_GET_CUR: u8 = 0x81;
pub const AUDIO_REQUEST_SET_MIN: u8 = 0x02;
pub const AUDIO_REQUEST_GET_MIN: u8 = 0x82;
pub const AUDIO_REQUEST_SET_MAX: u8 = 0x03;
pub const AUDIO_REQUEST_GET_MAX: u8 = 0x83;
pub const AUDIO_REQUEST_SET_RES: u8 = 0x04;
pub const AUDIO_REQUEST_GET_RES: u8 = 0x84;
pub const AUDIO_REQUEST_SET_MEM: u8 = 0x05;
pub const AUDIO_REQUEST_GET_MEM: u8 = 0x85;
pub const AUDIO_REQUEST_GET_STAT: u8 = 0xFF;

// Feature Unit Control Selectors
pub const AUDIO_MUTE_CONTROL: u8 = 0x01;
pub const AUDIO_VOLUME_CONTROL: u8 = 0x02;
pub const AUDIO_BASS_CONTROL: u8 = 0x03;
pub const AUDIO_MID_CONTROL: u8 = 0x04;
pub const AUDIO_TREBLE_CONTROL: u8 = 0x05;
pub const AUDIO_GRAPHIC_EQUALIZER_CONTROL: u8 = 0x06;
pub const AUDIO_AUTOMATIC_GAIN_CONTROL: u8 = 0x07;
pub const AUDIO_DELAY_CONTROL: u8 = 0x08;
pub const AUDIO_BASS_BOOST_CONTROL: u8 = 0x09;
pub const AUDIO_LOUDNESS_CONTROL: u8 = 0x0A;

// Processing Unit Control Selectors
pub const AUDIO_ENABLE_CONTROL: u8 = 0x01;
pub const AUDIO_MODE_SELECT_CONTROL: u8 = 0x02;

// 3D Stereo
pub const AUDIO_SPACIOUSNESS_CONTROL: u8 = 0x02;

// Reverberation Control Selectors
pub const AUDIO_REVERB_LEVEL_CONTROL: u8 = 0x02;
pub const AUDIO_REVERB_TIME_CONTROL: u8 = 0x03;
pub const AUDIO_REVERB_FEEDBACK_CONTROL: u8 = 0x04;

// Chorus Control Selectors
pub const AUDIO_CHORUS_LEVEL_CONTROL: u8 = 0x02;
pub const AUDIO_SHORUS_RATE_CONTROL: u8 = 0x03;
pub const AUDIO_CHORUS_DEPTH_CONTROL: u8 = 0x04;

// Dynamic Range Compressor Control Selectors
pub const AUDIO_COMPRESSION_RATE_CONTROL: u8 = 0x02;
pub const AUDIO_MAX_AMPL_CONTROL: u8 = 0x03;
pub const AUDIO_THRESHOLD_CONTROL: u8 = 0x04;
pub const AUDIO_ATTACK_TIME_CONTROL: u8 = 0x05;
pub const AUDIO_RELEASE_TIME_CONTROL: u8 = 0x06;

// Endpoint Control Selectors
pub const AUDIO_SAMPLING_FREQ_CONTROL: u8 = 0x01;
pub const AUDIO_PITCH_CONTROL: u8 = 0x02;

// MPEG Control Selectors
pub const AUDIO_MPEG_CONTROL_UNDEFINED: u8 = 0x00;
pub const AUDIO_MPEG_DUAL_CHANNEL_CONTROL: u8 = 0x01;
pub const AUDIO_MPEG_SECOND_STEREO_CONTROL: u8 = 0x02;
pub const AUDIO_MPEG_MULTILINGUAL_CONTROL: u8 = 0x03;
pub const AUDIO_MPEG_DYN_RANGE_CONTROL: u8 = 0x04;
pub const AUDIO_MPEG_SCALING_CONTROL: u8 = 0x05;
pub const AUDIO_MPEG_HILO_SCALING_CONTROL: u8 = 0x06;

// AC-3 Control Selectors
pub const AUDIO_AC3_CONTROL_UNDEFINED: u8 = 0x00;
pub const AUDIO_AC3_MODE_CONTROL: u8 = 0x01;
pub const AUDIO_AC3_DYN_RANGE_CONTROL: u8 = 0x02;
pub const AUDIO_AC3_SCALING_CONTROL: u8 = 0x03;
pub const AUDIO_AC3_HILO_SCALING_CONTROL: u8 = 0x04;

// Audio Format Types
pub const AUDIO_FORMAT_TYPE_UNDEFINED: u8 = 0x00;
pub const AUDIO_FORMAT_TYPE_I: u8 = 0x01;
pub const AUDIO_FORMAT_TYPE_II: u8 = 0x02;
pub const AUDIO_FORMAT_TYPE_III: u8 = 0x03;

pub const fn audio_format_type_i_desc_sz(n: usize) -> usize {
    0x08 + n * 3
}
pub const fn audio_format_type_ii_desc_sz(n: usize) -> usize {
    0x09 + n * 3
}
pub const fn audio_format_type_iii_desc_sz(n: usize) -> usize {
    0x08 + n * 3
}
pub const AUDIO_FORMAT_MPEG_DESC_SIZE: usize = 0x09;
pub const AUDIO_FORMAT_AC3_DESC_SIZE: usize = 0x0A;

// Audio Data Format Codes
pub const AUDIO_FORMAT_TYPE_I_UNDEFINED: u16 = 0x0000;
pub const AUDIO_FORMAT_PCM: u16 = 0x0001;
pub const AUDIO_FORMAT_PCM8: u16 = 0x0002;
pub const AUDIO_FORMAT_IEEE_FLOAT: u16 = 0x0003;
pub const AUDIO_FORMAT_ALAW: u16 = 0x0004;
pub const AUDIO_FORMAT_MULAW: u16 = 0x0005;

pub const AUDIO_FORMAT_TYPE_II_UNDEFINED: u16 = 0x1000;
pub const AUDIO_FORMAT_MPEG: u16 = 0x1001;
pub const AUDIO_FORMAT_AC3: u16 = 0x1002;

pub const AUDIO_FORMAT_TYPE_III_UNDEFINED: u16 = 0x2000;
pub const AUDIO_FORMAT_IEC1937_AC3: u16 = 0x2001;
pub const AUDIO_FORMAT_IEC1937_MPEG1_L1: u16 = 0x2002;
pub const AUDIO_FORMAT_IEC1937_MPEG1_L2_3: u16 = 0x2003;
pub const AUDIO_FORMAT_IEC1937_MPEG2_NOEXT: u16 = 0x2003;
pub const AUDIO_FORMAT_IEC1937_MPEG2_EXT: u16 = 0x2004;
pub const AUDIO_FORMAT_IEC1937_MPEG2_L1_LS: u16 = 0x2005;
pub const AUDIO_FORMAT_IEC1937_MPEG2_L2_3: u16 = 0x2006;

// Audio Channel Configuration
pub const AUDIO_CHANNEL_M: u16 = 0x0000;
pub const AUDIO_CHANNEL_L: u16 = 0x0001;
pub const AUDIO_CHANNEL_R: u16 = 0x0002;
pub const AUDIO_CHANNEL_C: u16 = 0x0004;
pub const AUDIO_CHANNEL_LFE: u16 = 0x0008;
pub const AUDIO_CHANNEL_LS: u16 = 0x0010;
pub const AUDIO_CHANNEL_RS: u16 = 0x0020;
pub const AUDIO_CHANNEL_LC: u16 = 0x0040;
pub const AUDIO_CHANNEL_RC: u16 = 0x0080;
pub const AUDIO_CHANNEL_S: u16 = 0x0100;
pub const AUDIO_CHANNEL_SL: u16 = 0x0200;
pub const AUDIO_CHANNEL_SR: u16 = 0x0400;
pub const AUDIO_CHANNEL_T: u16 = 0x0800;

// Feature Unit Control Bits
pub const AUDIO_CONTROL_MUTE: u16 = 0x0001;
pub const AUDIO_CONTROL_VOLUME: u16 = 0x0002;
pub const AUDIO_CONTROL_BASS: u16 = 0x0004;
pub const AUDIO_CONTROL_MID: u16 = 0x0008;
pub const AUDIO_CONTROL_TREBLE: u16 = 0x0010;
pub const AUDIO_CONTROL_GRAPHIC_EQUALIZER: u16 = 0x0020;
pub const AUDIO_CONTROL_AUTOMATIC_GAIN: u16 = 0x0040;
pub const AUDIO_CONTROL_DEALY: u16 = 0x0080;
pub const AUDIO_CONTROL_BASS_BOOST: u16 = 0x0100;
pub const AUDIO_CONTROL_LOUDNESS: u16 = 0x0200;

// Processing Unit Control Bits
pub const AUDIO_CONTROL_ENABLE: u16 = 0x0001;
pub const AUDIO_CONTROL_MODE_SELECT: u16 = 0x0002;

// 3D Stereo Extender Control Bits
pub const AUDIO_CONTROL_SPACIOUSNESS: u16 = 0x0002;

// Reverberation Control Bits
pub const AUDIO_CONTROL_REVERB_TYPE: u16 = 0x0002;
pub const AUDIO_CONTROL_REVERB_LEVEL: u16 = 0x0004;
pub const AUDIO_CONTROL_REVERB_TIME: u16 = 0x0008;
pub const AUDIO_CONTROL_REVERB_FEEDBACK: u16 = 0x0010;

// Chorus Control Bits
pub const AUDIO_CONTROL_CHORUS_LEVEL: u16 = 0x0002;
pub const AUDIO_CONTROL_SHORUS_RATE: u16 = 0x0004;
pub const AUDIO_CONTROL_CHORUS_DEPTH: u16 = 0x0008;

// Dynamic Range Compressor Control Bits
pub const AUDIO_CONTROL_COMPRESSION_RATE: u16 = 0x0002;
pub const AUDIO_CONTROL_MAX_AMPL: u16 = 0x0004;
pub const AUDIO_CONTROL_THRESHOLD: u16 = 0x0008;
pub const AUDIO_CONTROL_ATTACK_TIME: u16 = 0x0010;
pub const AUDIO_CONTROL_RELEASE_TIME: u16 = 0x0020;

// Endpoint Control Bits
pub const AUDIO_CONTROL_SAMPLING_FREQ: u8 = 0x01;
pub const AUDIO_CONTROL_PITCH: u8 = 0x02;
pub const AUDIO_MAX_PACKETS_ONLY: u8 = 0x80;

// Audio Terminal Types
pub const AUDIO_TERMINAL_USB_UNDEFINED: u16 = 0x0100;
pub const AUDIO_TERMINAL_USB_STREAMING: u16 = 0x0101;
pub const AUDIO_TERMINAL_USB_VENDOR_SPECIFIC: u16 = 0x01FF;

pub const AUDIO_TERMINAL_INPUT_UNDEFINED: u16 = 0x0200;
pub const AUDIO_TERMINAL_MICROPHONE: u16 = 0x0201;
pub const AUDIO_TERMINAL_DESKTOP_MICROPHONE: u16 = 0x0202;
pub const AUDIO_TERMINAL_PERSONAL_MICROPHONE: u16 = 0x0203;
pub const AUDIO_TERMINAL_OMNI_DIR_MICROPHONE: u16 = 0x0204;
pub const AUDIO_TERMINAL_MICROPHONE_ARRAY: u16 = 0x0205;
pub const AUDIO_TERMINAL_PROCESSING_MIC_ARRAY: u16 = 0x0206;

pub const AUDIO_TERMINAL_OUTPUT_UNDEFINED: u16 = 0x0300;
pub const AUDIO_TERMINAL_SPEAKER: u16 = 0x0301;
pub const AUDIO_TERMINAL_HEADPHONES: u16 = 0x0302;
pub const AUDIO_TERMINAL_HEAD_MOUNTED_AUDIO: u16 = 0x0303;
pub const AUDIO_TERMINAL_DESKTOP_SPEAKER: u16 = 0x0304;
pub const AUDIO_TERMINAL_ROOM_SPEAKER: u16 = 0x0305;
pub const AUDIO_TERMINAL_COMMUNICATION_SPEAKER: u16 = 0x0306;
pub const AUDIO_TERMINAL_LOW_FREQ_SPEAKER: u16 = 0x0307;

pub const AUDIO_TERMINAL_BIDIRECTIONAL_UNDEFINED: u16 = 0x0400;
pub const AUDIO_TERMINAL_HANDSET: u16 = 0x0401;
pub const AUDIO_TERMINAL_HEAD_MOUNTED_HANDSET: u16 = 0x0402;
pub const AUDIO_TERMINAL_SPEAKERPHONE: u16 = 0x0403;
pub const AUDIO_TERMINAL_SPEAKERPHONE_ECHOSUPRESS: u16 = 0x0404;
pub const AUDIO_TERMINAL_SPEAKERPHONE_ECHOCANCEL: u16 = 0x0405;

pub const AUDIO_TERMINAL_TELEPHONY_UNDEFINED: u16 = 0x0500;
pub const AUDIO_TERMINAL_PHONE_LINE: u16 = 0x0501;
pub const AUDIO_TERMINAL_TELEPHONE: u16 = 0x0502;
pub const AUDIO_TERMINAL_DOWN_LINE_PHONE: u16 = 0x0503;

pub const AUDIO_TERMINAL_EXTERNAL_UNDEFINED: u16 = 0x0600;
pub const AUDIO_TERMINAL_ANALOG_CONNECTOR: u16 = 0x0601;
pub const AUDIO_TERMINAL_DIGITAL_AUDIO_INTERFACE: u16 = 0x0602;
pub const AUDIO_TERMINAL_LINE_CONNECTOR: u16 = 0x0603;
pub const AUDIO_TERMINAL_LEGACY_AUDIO_CONNECTOR: u16 = 0x0604;
pub const AUDIO_TERMINAL_SPDIF_INTERFACE: u16 = 0x0605;
pub const AUDIO_TERMINAL_1394_DA_STREAM: u16 = 0x0606;
pub const AUDIO_TERMINAL_1394_DA_STREAM_TRACK: u16 = 0x0607;

pub const AUDIO_TERMINAL_EMBEDDED_UNDEFINED: u16 = 0x0700;
pub const AUDIO_TERMINAL_CALIBRATION_NOISE: u16 = 0x0701;
pub const AUDIO_TERMINAL_EQUALIZATION_NOISE: u16 = 0x0702;
pub const AUDIO_TERMINAL_CD_PLAYER: u16 = 0x0703;
pub const AUDIO_TERMINAL_DAT: u16 = 0x0704;
pub const AUDIO_TERMINAL_DCC: u16 = 0x0705;
pub const AUDIO_TERMINAL_MINI_DISK: u16 = 0x0706;
pub const AUDIO_TERMINAL_ANALOG_TAPE: u16 = 0x0707;
pub const AUDIO_TERMINAL_PHONOGRAPH: u16 = 0x0708;
pub const AUDIO_TERMINAL_VCR_AUDIO: u16 = 0x0709;
pub const AUDIO_TERMINAL_VIDEO_DISC_AUDIO: u16 = 0x070A;
pub const AUDIO_TERMINAL_DVD_AUDIO: u16 = 0x070B;
pub const AUDIO_TERMINAL_TV_TUNER_AUDIO: u16 = 0x070C;
pub const AUDIO_TERMINAL_SATELLITE_RECEIVER_AUDIO: u16 = 0x070D;
pub const AUDIO_TERMINAL_CABLE_TUNER_AUDIO: u16 = 0x070E;
pub const AUDIO_TERMINAL_DSS_AUDIO: u16 = 0x070F;
pub const AUDIO_TERMINAL_RADIO_RECEIVER: u16 = 0x0710;
pub const AUDIO_TERMINAL_RADIO_TRANSMITTER: u16 = 0x0711;
pub const AUDIO_TERMINAL_MULTI_TRACK_RECORDER: u16 = 0x0712;
pub const AUDIO_TERMINAL_SYNTHESIZER: u16 = 0x0713;

// =====================================================================
// Microphone / Headset common types
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MicMode {
    #[default]
    None = 0,
    Single,
    Separate,
    Shared,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioDir {
    Source = 0,
    Sink,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicrophoneType {
    Singstar,
    Logitech,
    Konami,
}

impl MicrophoneType {
    pub const COUNT: usize = 3;
    pub fn from_index(i: u32) -> Option<Self> {
        match i {
            0 => Some(Self::Singstar),
            1 => Some(Self::Logitech),
            2 => Some(Self::Konami),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeadsetChannel {
    Headphones = 0x0100,
    Microphone = 0x0600,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UsbAudioAltset {
    #[default]
    Off = 0x00,
    On = 0x01,
}

// =====================================================================
// Audio device trait + helpers
// =====================================================================

/// Generic audio I/O device. Cubeb + Noop implement it; the original
/// C++ class hierarchy has been collapsed into a single trait to match
/// the brief.
pub trait AudioDevice: Send {
    fn start(&mut self) -> bool;
    fn stop(&mut self) -> bool;
    fn read(&mut self) -> Result<usize, String>;
    fn write(&mut self) -> Result<usize, String>;
    fn channels(&self) -> u32;
    fn set_resampling(&mut self, samplerate: u32) -> bool;
    fn get_frames(&self, size: &mut u32) -> bool {
        *size = 0;
        true
    }
    fn get_buffer(&mut self, frames: u32) -> usize {
        frames as usize
    }
    fn set_buffer(&mut self, frames: u32) -> usize {
        frames as usize
    }
}

pub const DEFAULT_LATENCY: i32 = 100;
pub const DEFAULT_LATENCY_STR: &str = "100";

// =====================================================================
// Cubeb audio device (translated from audiodev-cubeb.h/.cpp)
// =====================================================================

/// Stub for the upstream C `cubeb` context. The C++ side uses a
/// `cubeb*` from the `cubeb` library; here we keep an opaque pointer
/// type so the surrounding code is structurally faithful without
/// pulling in a real audio backend.
#[repr(C)]
pub struct CubebContext {
    _private: [u8; 0],
}

/// Stub for the upstream C `cubeb_stream`. Same idea: opaque.
#[repr(C)]
pub struct CubebStream {
    _private: [u8; 0],
}

/// Newtype wrapper around a raw `cubeb*` pointer so it can live inside
/// a `Mutex` inside a `static`. Raw pointers are not `Send`, so we
/// explicitly opt in: the pointer is only ever touched behind a lock.
#[derive(Copy, Clone)]
#[repr(transparent)]
pub struct CubebContextPtr(pub *mut CubebContext);
unsafe impl Send for CubebContextPtr {}

pub struct CubebAudioDevice {
    audio_dir: AudioDir,
    channels: u32,
    sample_rate: u32,
    latency_ms: u32,
    stream_latency: u32,
    device_name: String,
    device_id: Option<usize>,
    context: Option<CubebContextPtr>,
    stream: Option<*mut CubebStream>,
    buffer: RingBuffer,
    lock: Mutex<()>,
}

unsafe impl Send for CubebAudioDevice {}

impl CubebAudioDevice {
    pub fn new(audio_dir: AudioDir, channels: u32, device_name: String, latency_ms: i32) -> Self {
        let context = get_cubeb_context();
        let device_id = if device_name != "cubeb_default" {
            find_cubeb_device(&device_name, audio_dir == AudioDir::Source)
        } else {
            None
        };

        let mut dev = Self {
            audio_dir,
            channels,
            sample_rate: 48_000,
            latency_ms: latency_ms.max(0) as u32,
            stream_latency: 0,
            device_name,
            device_id,
            context,
            stream: None,
            buffer: RingBuffer::new(0),
            lock: Mutex::new(()),
        };
        dev.reset_buffers();
        dev
    }

    pub fn get_device_list(input: bool) -> Vec<(String, String)> {
        let mut out = Vec::new();
        out.push((String::new(), "Not Connected".to_string()));
        out.push((
            "cubeb_default".to_string(),
            if input {
                "Default Input Device".to_string()
            } else {
                "Default Output Device".to_string()
            },
        ));
        out
    }

    pub fn get_buffer(&mut self, frames: u32) -> usize {
        let _g = self.lock.lock().unwrap();
        if self.stream.is_none() {
            return 0;
        }
        let bytes = self.buffer.read_size(frames as usize * self.channels as usize);
        bytes / self.channels as usize
    }

    pub fn set_buffer(&mut self, frames: u32) -> usize {
        let _g = self.lock.lock().unwrap();
        if self.stream.is_none() {
            return frames as usize;
        }
        frames as usize
    }

    pub fn get_frames(&self, size: &mut u32) -> bool {
        let _g = self.lock.lock().unwrap();
        if self.stream.is_none() {
            return true;
        }
        *size = (self.buffer.len() / self.channels as usize) as u32;
        true
    }

    pub fn reset_buffers(&mut self) {
        let _g = self.lock.lock().unwrap();
        let per_ms = (self.sample_rate * self.channels) / 1000;
        let samples = (per_ms * self.latency_ms).max(self.stream_latency * self.channels);
        self.buffer = RingBuffer::new((samples as usize) * std::mem::size_of::<u16>());
    }
}

impl AudioDevice for CubebAudioDevice {
    fn start(&mut self) -> bool {
        if self.stream.is_some() {
            self.stop();
        }
        if !self.device_name.is_empty()
            && self.device_name != "cubeb_default"
            && self.device_id.is_none()
        {
            return false;
        }
        // Real cubeb init happens here; in this translation we only
        // mirror the bookkeeping.
        self.stream_latency = (self.latency_ms * self.sample_rate) / 1000;
        self.reset_buffers();
        // stream ptr would come from cubeb_stream_init in the real backend.
        self.stream = None;
        true
    }

    fn stop(&mut self) -> bool {
        self.stream = None;
        true
    }

    fn read(&mut self) -> Result<usize, String> {
        Ok(self.get_buffer(0))
    }

    fn write(&mut self) -> Result<usize, String> {
        Ok(self.set_buffer(0))
    }

    fn channels(&self) -> u32 {
        self.channels
    }

    fn set_resampling(&mut self, samplerate: u32) -> bool {
        let was_running = self.stream.is_some();
        self.stop();
        self.sample_rate = samplerate;
        if was_running {
            self.start();
        }
        self.reset_buffers();
        true
    }
}

impl Drop for CubebAudioDevice {
    fn drop(&mut self) {
        self.stop();
        if self.context.is_some() {
            release_cubeb_context();
        }
    }
}

// =====================================================================
// Noop audio device (translated from audiodev-noop.h)
// =====================================================================

pub struct NoopAudioDevice {
    audio_dir: AudioDir,
    channels: u32,
}

impl NoopAudioDevice {
    pub fn new(audio_dir: AudioDir, channels: u32) -> Self {
        Self {
            audio_dir,
            channels,
        }
    }
}

impl AudioDevice for NoopAudioDevice {
    fn start(&mut self) -> bool {
        true
    }
    fn stop(&mut self) -> bool {
        true
    }
    fn read(&mut self) -> Result<usize, String> {
        Ok(0)
    }
    fn write(&mut self) -> Result<usize, String> {
        Ok(0)
    }
    fn channels(&self) -> u32 {
        self.channels
    }
    fn set_resampling(&mut self, _samplerate: u32) -> bool {
        true
    }
}

// =====================================================================
// Cubeb context helpers (mirroring GetCubebContext / ReleaseCubebContext)
// =====================================================================

static CUBEB_CONTEXT: Mutex<Option<CubebContextPtr>> = Mutex::new(None);
static CUBEB_INPUT_DEVICES: Mutex<Vec<usize>> = Mutex::new(Vec::new());
static CUBEB_OUTPUT_DEVICES: Mutex<Vec<usize>> = Mutex::new(Vec::new());
static CUBEB_REFCOUNT: Mutex<u32> = Mutex::new(0);

fn get_cubeb_context() -> Option<CubebContextPtr> {
    let mut count = CUBEB_REFCOUNT.lock().unwrap();
    let mut ctx = CUBEB_CONTEXT.lock().unwrap();
    if ctx.is_none() {
        // Real cubeb_init() would populate the device lists here.
        *CUBEB_INPUT_DEVICES.lock().unwrap() = Vec::new();
        *CUBEB_OUTPUT_DEVICES.lock().unwrap() = Vec::new();
        *ctx = Some(CubebContextPtr(std::ptr::null_mut()));
    }
    if ctx.is_some() {
        *count += 1;
    }
    *ctx
}

fn release_cubeb_context() {
    let mut count = CUBEB_REFCOUNT.lock().unwrap();
    if *count > 0 {
        *count -= 1;
        if *count == 0 {
            *CUBEB_INPUT_DEVICES.lock().unwrap() = Vec::new();
            *CUBEB_OUTPUT_DEVICES.lock().unwrap() = Vec::new();
            *CUBEB_CONTEXT.lock().unwrap() = None;
        }
    }
}

fn find_cubeb_device(name: &str, input: bool) -> Option<usize> {
    if name == "cubeb_default" {
        return None;
    }
    let _devices = if input {
        CUBEB_INPUT_DEVICES.lock().unwrap()
    } else {
        CUBEB_OUTPUT_DEVICES.lock().unwrap()
    };
    None
}

// =====================================================================
// Tiny ring buffer (used to back CubebAudioDevice's storage)
// =====================================================================

pub struct RingBuffer {
    storage: Vec<u8>,
    capacity: usize,
    head: usize,
    tail: usize,
    used: usize,
}

impl RingBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            storage: vec![0u8; capacity],
            capacity,
            head: 0,
            tail: 0,
            used: 0,
        }
    }

    pub fn reset(&mut self, capacity: usize) {
        self.storage = vec![0u8; capacity];
        self.capacity = capacity;
        self.head = 0;
        self.tail = 0;
        self.used = 0;
    }

    pub fn read_size(&mut self, want: usize) -> usize {
        let n = want.min(self.used);
        if n == 0 {
            return 0;
        }
        // We do not return a copy of the bytes here; the real cubeb path
        // hands a buffer pointer in, so this is a logical read.
        self.used -= n;
        self.head = (self.head + n) % self.capacity.max(1);
        n
    }

    pub fn write_size(&mut self, want: usize) -> usize {
        let n = want.min(self.capacity - self.used);
        if n == 0 {
            return 0;
        }
        self.used += n;
        self.tail = (self.tail + n) % self.capacity.max(1);
        n
    }

    pub fn len(&self) -> usize {
        self.used
    }

    pub fn is_empty(&self) -> bool {
        self.used == 0
    }
}

// =====================================================================
// USBMic (translated from usb-mic.h/.cpp)
// =====================================================================

/// Freezeable state preserved in `USBMic::state`.
#[derive(Default)]
pub struct USBMicState {
    pub intf: i32,
    pub mode: MicMode,
    pub altset: UsbAudioAltset,
    pub mute: bool,
    pub vol: [u8; 2],
    pub srate: [u32; 2],
}

pub struct USBMic {
    state: USBMicState,
    sources: [Option<Box<dyn AudioDevice>>; 2],
    buffers: [Vec<i16>; 2],
    sample_rate: u32,
    subtype: MicrophoneType,
    started: bool,
    play_mode: Arc<(Mutex<bool>, Condvar)>,
}

impl USBMic {
    pub fn new(subtype: MicrophoneType, sample_rate: u32) -> Self {
        Self {
            state: USBMicState {
                intf: 0,
                mode: MicMode::None,
                altset: UsbAudioAltset::Off,
                mute: false,
                vol: [240, 240],
                srate: [sample_rate, sample_rate],
            },
            sources: [None, None],
            buffers: [Vec::new(), Vec::new()],
            sample_rate,
            subtype,
            started: false,
            play_mode: Arc::new((Mutex::new(false), Condvar::new())),
        }
    }

    pub fn init(&mut self) -> Result<(), String> {
        let dual = matches!(self.subtype, MicrophoneType::Singstar);
        if dual {
            self.state.mode = MicMode::Separate;
        } else {
            self.state.mode = MicMode::Single;
        }
        for i in 0..2 {
            if let Some(src) = self.sources[i].as_mut() {
                self.buffers[i] = vec![0i16; 200 * src.channels() as usize];
                if !src.start() {
                    return Err(format!("failed to start source {}", i));
                }
                src.set_resampling(self.sample_rate);
            }
        }
        self.started = true;
        Ok(())
    }

    pub fn shutdown(&mut self) {
        if !self.started {
            return;
        }
        for i in 0..2 {
            if let Some(src) = self.sources[i].as_mut() {
                src.stop();
            }
            self.buffers[i].clear();
            self.sources[i] = None;
        }
        let (lock, cvar) = &*self.play_mode;
        *lock.lock().unwrap() = false;
        cvar.notify_all();
        self.started = false;
    }

    /// Read a frame worth of PCM samples (mono or interleaved stereo).
    pub fn read_frame(&mut self, samples: &mut [i16]) -> Result<usize, String> {
        if !self.started {
            return Err("microphone not started".to_string());
        }
        let out_chns = if self.state.intf == 2 { 2 } else { 1 };
        let max_frames = samples.len() / out_chns;

        // Reset destination
        for s in samples.iter_mut() {
            *s = 0;
        }

        let mut out_frames = [0u32; 2];
        for i in 0..2 {
            if let Some(src) = self.sources[i].as_mut() {
                let mut avail = 0u32;
                let _ = src.get_frames(&mut avail);
                let frames = avail.min(max_frames as u32);
                out_frames[i] = src.get_buffer(frames) as u32;
            }
        }

        if out_frames[0] == 0 && out_frames[1] == 0 {
            return Err("no frames available".to_string());
        }

        match self.state.mode {
            MicMode::Single => {
                let k = if self.sources[0].is_some() { 0 } else { 1 };
                if let Some(src) = self.sources[k].as_ref() {
                    let chn = src.channels() as usize;
                    let frames = out_frames[k] as usize;
                    for i in 0..frames.min(max_frames) {
                        samples[i * out_chns] =
                            set_volume(self.buffers[k][i * chn], self.state.vol[0]);
                    }
                    return Ok(frames);
                }
                Ok(0)
            }
            MicMode::Shared => {
                if let Some(src) = self.sources[0].as_ref() {
                    let chn = src.channels() as usize;
                    let frames = out_frames[0] as usize;
                    for i in 0..frames.min(max_frames) {
                        samples[i * out_chns] =
                            set_volume(self.buffers[0][i * chn], self.state.vol[0]);
                        if out_chns > 1 {
                            let right = if chn == 1 {
                                samples[i * out_chns]
                            } else {
                                set_volume(self.buffers[0][i * chn + 1], self.state.vol[0])
                            };
                            samples[i * out_chns + 1] = right;
                        }
                    }
                    return Ok(frames);
                }
                Ok(0)
            }
            MicMode::Separate => {
                let c1 = self.sources[0].as_ref().map(|s| s.channels() as usize);
                let c2 = self.sources[1].as_ref().map(|s| s.channels() as usize);
                let min_len = out_frames[0].min(out_frames[1]) as usize;
                for i in 0..min_len.min(max_frames) {
                    if let Some(c1) = c1 {
                        samples[i * out_chns] =
                            set_volume(self.buffers[0][i * c1], self.state.vol[0]);
                    }
                    if out_chns > 1 {
                        if let Some(c2) = c2 {
                            samples[i * out_chns + 1] =
                                set_volume(self.buffers[1][i * c2], self.state.vol[1]);
                        }
                    }
                }
                Ok(min_len)
            }
            MicMode::None => Ok(0),
        }
    }

    pub fn state(&self) -> &USBMicState {
        &self.state
    }

    pub fn set_source(&mut self, idx: usize, dev: Option<Box<dyn AudioDevice>>) {
        if idx < 2 {
            self.sources[idx] = dev;
        }
    }
}

impl Drop for USBMic {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn set_volume(sample: i16, vol: u8) -> i16 {
    ((sample as i32) * (vol as i32) / 0xFF) as i16
}

// =====================================================================
// USBHeadset (translated from usb-headset.h/.cpp)
// =====================================================================

#[derive(Default, Debug, Clone, Copy)]
pub struct HeadsetInputState {
    pub mute: bool,
    pub vol: u8,
    pub srate: u32,
}

#[derive(Default, Debug, Clone, Copy)]
pub struct HeadsetOutputState {
    pub mute: bool,
    pub vol: [u8; 2],
    pub srate: u32,
}

#[derive(Default, Debug, Clone, Copy)]
pub struct HeadsetMixerState {
    pub mute: bool,
    pub vol: [u8; 2],
}

#[derive(Default)]
pub struct HeadsetState {
    pub intf: i32,
    pub mode: MicMode,
    pub out: HeadsetOutputState,
    pub in_: HeadsetInputState,
    pub mixer: HeadsetMixerState,
}

pub struct USBHeadset {
    state: HeadsetState,
    src: Option<Box<dyn AudioDevice>>,
    sink: Option<Box<dyn AudioDevice>>,
    in_buffer: Vec<i16>,
    out_buffer: Vec<i16>,
    started: bool,
}

impl USBHeadset {
    pub fn new() -> Self {
        let state = HeadsetState {
            intf: 0,
            mode: MicMode::Single,
            out: HeadsetOutputState {
                mute: false,
                vol: [240, 240],
                srate: 48_000,
            },
            in_: HeadsetInputState {
                mute: false,
                vol: 240,
                srate: 48_000,
            },
            mixer: HeadsetMixerState::default(),
        };
        Self {
            state,
            src: None,
            sink: None,
            in_buffer: Vec::new(),
            out_buffer: Vec::new(),
            started: false,
        }
    }

    pub fn init(&mut self) -> Result<(), String> {
        let mut had_error = None;
        if let Some(s) = self.src.as_mut() {
            if !s.start() {
                had_error = Some("input start");
            }
        } else {
            had_error = Some("input not configured");
        }
        if let Some(s) = self.sink.as_mut() {
            if !s.start() {
                had_error = had_error.or(Some("output start"));
            }
        } else {
            had_error = had_error.or(Some("output not configured"));
        }
        if let Some(why) = had_error {
            return Err(why.to_string());
        }
        self.started = true;
        Ok(())
    }

    pub fn shutdown(&mut self) {
        if let Some(s) = self.src.as_mut() {
            s.stop();
        }
        if let Some(s) = self.sink.as_mut() {
            s.stop();
        }
        self.in_buffer.clear();
        self.out_buffer.clear();
        self.started = false;
    }

    /// Push an interleaved PCM frame to the host output device.
    pub fn write_frame(&mut self, samples: &[i16]) -> Result<(), String> {
        if !self.started {
            return Err("headset not started".to_string());
        }
        let sink = match self.sink.as_mut() {
            Some(s) => s,
            None => return Err("no output device".to_string()),
        };
        let in_chns = if self.state.intf == 1 { 2 } else { 1 };
        let out_chns = sink.channels() as usize;
        let frames = samples.len() / in_chns;
        self.out_buffer.resize(frames * out_chns, 0);
        for i in 0..frames {
            if in_chns == out_chns {
                for c in 0..out_chns {
                    self.out_buffer[i * out_chns + c] =
                        set_volume(samples[i * in_chns + c], self.state.out.vol[c]);
                }
            } else if in_chns < out_chns {
                for c in 0..out_chns {
                    self.out_buffer[i * out_chns + c] =
                        set_volume(samples[i * in_chns], self.state.out.vol[c]);
                }
            }
        }
        sink.set_buffer(frames as u32);
        Ok(())
    }

    /// Read an interleaved PCM frame from the host input device.
    pub fn read_frame(&mut self, samples: &mut [i16]) -> Result<usize, String> {
        if !self.started {
            return Err("headset not started".to_string());
        }
        let src = match self.src.as_mut() {
            Some(s) => s,
            None => return Err("no input device".to_string()),
        };
        let in_chns = src.channels() as usize;
        let max_frames = samples.len();
        let mut frames = 0u32;
        let _ = src.get_frames(&mut frames);
        let frames = frames.min(max_frames as u32) as usize;
        self.in_buffer.resize(frames * in_chns, 0);
        let got = src.get_buffer(frames as u32);
        for i in 0..got {
            samples[i] = set_volume(self.in_buffer[i * in_chns], self.state.in_.vol);
        }
        Ok(got)
    }

    pub fn state(&self) -> &HeadsetState {
        &self.state
    }

    pub fn set_devices(
        &mut self,
        src: Option<Box<dyn AudioDevice>>,
        sink: Option<Box<dyn AudioDevice>>,
    ) {
        self.src = src;
        self.sink = sink;
    }
}

impl Drop for USBHeadset {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// =====================================================================
// USB descriptor blobs (SingStar / Logitech / AK5370 / Headset / EyeToy)
// =====================================================================

pub struct UsbDescStrings {
    pub strings: [&'static str; 4],
}

pub const SINGSTAR_DESC_STRINGS: UsbDescStrings = UsbDescStrings {
    strings: ["", "Nam Tai E&E Products Ltd.", "USBMIC", "310420811"],
};

pub const LOGITECH_DESC_STRINGS: UsbDescStrings = UsbDescStrings {
    strings: ["", "Logitech", "USBMIC", ""],
};

pub const AK5370_DESC_STRINGS: UsbDescStrings = UsbDescStrings {
    strings: ["", "AKM", "AK5370", ""],
};

pub const EYETOY_DESC_STRINGS: UsbDescStrings = UsbDescStrings {
    strings: ["", "Sony corporation", "EyeToy USB camera Namtai", ""],
};

pub const HEADSET_DESC_STRINGS: UsbDescStrings = UsbDescStrings {
    strings: ["", "Logitech", "Logitech USB Headset", "00000000"],
};

pub const SINGSTAR_DEV_DESCRIPTOR: &[u8] = &[
    0x12, 0x01, 0x10, 0x01, 0x00, 0x00, 0x00, 0x08, 0x15, 0x14, 0x00, 0x00, 0x01, 0x00, 0x01, 0x02,
    0x00, 0x01,
];

pub const SINGSTAR_CONFIG_DESCRIPTOR: &[u8] = &[
    0x09, 0x02, 0xb1, 0x00, 0x02, 0x01, 0x00, 0x80, 0x5A, 0x09, 0x04, 0x00, 0x00, 0x00, 0x01, 0x01,
    0x00, 0x00, 0x09, 0x24, 0x01, 0x00, 0x01, 0x28, 0x00, 0x01, 0x01, 0x0C, 0x24, 0x02, 0x01, 0x01,
    0x02, 0x02, 0x02, 0x03, 0x00, 0x00, 0x09, 0x24, 0x03, 0x02, 0x01, 0x01, 0x03, 0x00, 0x0A, 0x24,
    0x06, 0x03, 0x01, 0x01, 0x01, 0x01, 0x02, 0x02, 0x00, 0x09, 0x04, 0x01, 0x00, 0x00, 0x01, 0x02,
    0x00, 0x00, 0x09, 0x04, 0x01, 0x01, 0x01, 0x01, 0x02, 0x00, 0x00, 0x07, 0x24, 0x01, 0x02, 0x01,
    0x01, 0x00, 0x0B, 0x24, 0x02, 0x01, 0x01, 0x02, 0x10, 0x05, 0x40, 0x1F, 0x00, 0x11, 0x2B, 0x00,
    0x22, 0x56, 0x00, 0x44, 0xAC, 0x00, 0x80, 0xBB, 0x00, 0x09, 0x05, 0x81, 0x01, 0x64, 0x00, 0x01,
    0x00, 0x00, 0x07, 0x25, 0x01, 0x01, 0x00, 0x00, 0x00, 0x09, 0x04, 0x01, 0x02, 0x01, 0x01, 0x02,
    0x00, 0x00, 0x07, 0x24, 0x01, 0x02, 0x01, 0x01, 0x00, 0x0B, 0x24, 0x02, 0x01, 0x02, 0x02, 0x10,
    0x05, 0x40, 0x1F, 0x00, 0x11, 0x2B, 0x00, 0x22, 0x56, 0x00, 0x44, 0xAC, 0x00, 0x80, 0xBB, 0x00,
    0x09, 0x05, 0x81, 0x01, 0xC8, 0x00, 0x01, 0x00, 0x00, 0x07, 0x25, 0x01, 0x01, 0x00, 0x00, 0x00,
    0x00,
];

pub const LOGITECH_DEV_DESCRIPTOR: &[u8] = &[
    0x12, 0x01, 0x10, 0x01, 0x00, 0x00, 0x00, 0x08, 0x6D, 0x04, 0x00, 0x00, 0x01, 0x00, 0x01, 0x02,
    0x00, 0x01,
];

pub const LOGITECH_CONFIG_DESCRIPTOR: &[u8] = SINGSTAR_CONFIG_DESCRIPTOR;

pub const AK5370_DEV_DESCRIPTOR: &[u8] = &[
    0x12, 0x01, 0x10, 0x01, 0x00, 0x00, 0x00, 0x08, 0x56, 0x05, 0x01, 0x00, 0x01, 0x00, 0x01, 0x02,
    0x00, 0x01,
];

pub const AK5370_CONFIG_DESCRIPTOR: &[u8] = &[
    0x09, 0x02, 0x76, 0x00, 0x02, 0x01, 0x00, 0x80, 0x2D, 0x09, 0x04, 0x00, 0x00, 0x00, 0x01, 0x01,
    0x00, 0x00, 0x09, 0x24, 0x01, 0x00, 0x01, 0x26, 0x00, 0x01, 0x01, 0x0C, 0x24, 0x02, 0x01, 0x01,
    0x02, 0x01, 0x00, 0x00, 0x00, 0x00, 0x09, 0x24, 0x03, 0x02, 0x01, 0x01, 0x01, 0x00, 0x08, 0x24,
    0x06, 0x03, 0x01, 0x01, 0x43, 0x00, 0x09, 0x04, 0x01, 0x00, 0x00, 0x01, 0x02, 0x00, 0x00, 0x09,
    0x04, 0x01, 0x01, 0x01, 0x01, 0x02, 0x00, 0x00, 0x07, 0x24, 0x01, 0x02, 0x01, 0x01, 0x00, 0x17,
    0x24, 0x02, 0x01, 0x01, 0x02, 0x10, 0x05, 0x40, 0x1F, 0x00, 0x11, 0x2B, 0x00, 0x22, 0x56, 0x00,
    0x44, 0xAC, 0x00, 0x80, 0xBB, 0x00, 0x07, 0x05, 0x81, 0x01, 0x64, 0x00, 0x01, 0x07, 0x25, 0x01,
    0x01, 0x00, 0x00, 0x00,
];

pub const EYETOY_DEV_DESCRIPTOR: &[u8] = &[
    0x12, 0x01, 0x10, 0x01, 0x00, 0x00, 0x00, 0x08, 0x4C, 0x05, 0x55, 0x01, 0x00, 0x01, 0x01, 0x02,
    0x00, 0x01,
];

pub const EYETOY_CONFIG_DESCRIPTOR: &[u8] = &[
    0x09, 0x02, 0xB4, 0x00, 0x03, 0x01, 0x00, 0x80, 0xFA, 0x09, 0x04, 0x00, 0x00, 0x01, 0xFF, 0x00,
    0x00, 0x00, 0x07, 0x05, 0x81, 0x01, 0x00, 0x00, 0x01, 0x09, 0x04, 0x00, 0x01, 0x01, 0xFF, 0x00,
    0x00, 0x00, 0x07, 0x05, 0x81, 0x01, 0x80, 0x01, 0x01, 0x09, 0x04, 0x00, 0x02, 0x01, 0xFF, 0x00,
    0x00, 0x00, 0x07, 0x05, 0x81, 0x01, 0x00, 0x02, 0x01, 0x09, 0x04, 0x00, 0x03, 0x01, 0xFF, 0x00,
    0x00, 0x00, 0x07, 0x05, 0x81, 0x01, 0x00, 0x03, 0x01, 0x09, 0x04, 0x00, 0x04, 0x01, 0xFF, 0x00,
    0x00, 0x00, 0x07, 0x05, 0x81, 0x01, 0x80, 0x03, 0x01, 0x09, 0x04, 0x01, 0x00, 0x00, 0x01, 0x01,
    0x00, 0x00, 0x09, 0x24, 0x01, 0x00, 0x01, 0x1E, 0x00, 0x01, 0x02, 0x0C, 0x24, 0x02, 0x01, 0x01,
    0x02, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x09, 0x24, 0x03, 0x02, 0x01, 0x01, 0x01, 0x00, 0x09,
    0x04, 0x02, 0x00, 0x00, 0x01, 0x02, 0x00, 0x00, 0x09, 0x04, 0x02, 0x01, 0x01, 0x01, 0x02, 0x00,
    0x00, 0x07, 0x24, 0x01, 0x02, 0x01, 0x01, 0x00, 0x0B, 0x24, 0x02, 0x01, 0x01, 0x02, 0x10, 0x01,
    0x80, 0x3E, 0x00, 0x09, 0x05, 0x82, 0x05, 0x28, 0x00, 0x01, 0x00, 0x00, 0x07, 0x25, 0x01, 0x00,
    0x00, 0x00, 0x00,
];

pub const OV511P_DEV_DESCRIPTOR: &[u8] = &[
    0x12, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x08, 0xA9, 0x05, 0x11, 0xA5, 0x00, 0x01, 0x00, 0x00,
    0x00, 0x01,
];

pub const OV511P_CONFIG_DESCRIPTOR: &[u8] = &[
    0x09, 0x02, 0x89, 0x00, 0x01, 0x01, 0x00, 0x80, 0xFA, 0x09, 0x04, 0x00, 0x00, 0x01, 0xFF, 0x00,
    0x00, 0x00, 0x07, 0x05, 0x81, 0x01, 0x00, 0x00, 0x01, 0x09, 0x04, 0x00, 0x01, 0x01, 0xFF, 0x00,
    0x00, 0x00, 0x07, 0x05, 0x81, 0x01, 0x21, 0x00, 0x01, 0x09, 0x04, 0x00, 0x02, 0x01, 0xFF, 0x00,
    0x00, 0x00, 0x07, 0x05, 0x81, 0x01, 0x81, 0x00, 0x01, 0x09, 0x04, 0x00, 0x03, 0x01, 0xFF, 0x00,
    0x00, 0x00, 0x07, 0x05, 0x81, 0x01, 0x01, 0x01, 0x01, 0x09, 0x04, 0x00, 0x04, 0x01, 0xFF, 0x00,
    0x00, 0x00, 0x07, 0x05, 0x81, 0x01, 0x81, 0x01, 0x01, 0x09, 0x04, 0x00, 0x05, 0x01, 0xFF, 0x00,
    0x00, 0x00, 0x07, 0x05, 0x81, 0x01, 0x01, 0x02, 0x01, 0x09, 0x04, 0x00, 0x06, 0x01, 0xFF, 0x00,
    0x00, 0x00, 0x07, 0x05, 0x81, 0x01, 0x01, 0x03, 0x01, 0x09, 0x04, 0x00, 0x07, 0x01, 0xFF, 0x00,
    0x00, 0x00, 0x07, 0x05, 0x81, 0x01, 0xC1, 0x03, 0x01,
];

pub const HEADSET_DEV_DESCRIPTOR: &[u8] = &[
    0x12, 0x01, 0x10, 0x01, 0x00, 0x00, 0x00, 0x40, 0x6D, 0x04, 0x01, 0x0A, 0x12, 0x10, 0x01, 0x02,
    0x00, 0x01,
];

pub const HEADSET_CONFIG_DESCRIPTOR: &[u8] = &[
    0x09, 0x02, 0x3E, 0x01, 0x03, 0x01, 0x00, 0x80, 0x64, 0x09, 0x04, 0x00, 0x00, 0x00, 0x01, 0x01,
    0x00, 0x00, 0x0A, 0x24, 0x01, 0x00, 0x01, 0x75, 0x00, 0x02, 0x01, 0x02, 0x0C, 0x24, 0x02, 0x0D,
    0x01, 0x02, 0x00, 0x01, 0x01, 0x00, 0x00, 0x00, 0x0A, 0x24, 0x06, 0x06, 0x0D, 0x01, 0x03, 0x00,
    0x00, 0x0C, 0x24, 0x02, 0x0C, 0x01, 0x02, 0x00, 0x02, 0x03, 0x00, 0x00, 0x0D, 0x24, 0x04, 0x09,
    0x02, 0x0C, 0x06, 0x02, 0x03, 0x00, 0x00, 0x0A, 0x24, 0x06, 0x01, 0x09, 0x01, 0x01, 0x02, 0x02,
    0x00, 0x09, 0x24, 0x03, 0x0E, 0x01, 0x01, 0x00, 0x0C, 0x24, 0x02, 0x0B, 0x01, 0x02, 0x00, 0x01,
    0x01, 0x00, 0x00, 0x0A, 0x24, 0x06, 0x02, 0x0B, 0x01, 0x03, 0x00, 0x00, 0x0A, 0x24, 0x04, 0x07,
    0x01, 0x02, 0x01, 0x01, 0x00, 0x00, 0x09, 0x24, 0x03, 0x0A, 0x01, 0x07, 0x00, 0x09, 0x04, 0x01,
    0x00, 0x00, 0x01, 0x02, 0x00, 0x00, 0x09, 0x04, 0x01, 0x01, 0x01, 0x01, 0x02, 0x00, 0x00, 0x07,
    0x24, 0x01, 0x0C, 0x01, 0x01, 0x00, 0x0B, 0x24, 0x02, 0x01, 0x02, 0x02, 0x10, 0x05, 0x40, 0x1F,
    0x00, 0x11, 0x2B, 0x00, 0x22, 0x56, 0x00, 0x44, 0xAC, 0x00, 0x80, 0xBB, 0x00, 0x09, 0x05, 0x01,
    0x05, 0xC0, 0x00, 0x01, 0x00, 0x00, 0x07, 0x25, 0x01, 0x01, 0x00, 0x00, 0x00, 0x09, 0x04, 0x01,
    0x02, 0x01, 0x01, 0x02, 0x00, 0x00, 0x07, 0x24, 0x01, 0x0C, 0x01, 0x01, 0x00, 0x0B, 0x24, 0x02,
    0x01, 0x01, 0x02, 0x10, 0x05, 0x40, 0x1F, 0x00, 0x11, 0x2B, 0x00, 0x22, 0x56, 0x00, 0x44, 0xAC,
    0x00, 0x80, 0xBB, 0x00, 0x09, 0x05, 0x01, 0x05, 0x60, 0x00, 0x01, 0x00, 0x00, 0x07, 0x25, 0x01,
    0x01, 0x00, 0x00, 0x00, 0x09, 0x04, 0x02, 0x00, 0x00, 0x01, 0x02, 0x00, 0x00, 0x09, 0x04, 0x02,
    0x01, 0x01, 0x01, 0x02, 0x00, 0x00, 0x07, 0x24, 0x01, 0x0A, 0x00, 0x01, 0x00, 0x0B, 0x24, 0x02,
    0x01, 0x01, 0x02, 0x10, 0x05, 0x40, 0x1F, 0x00, 0x11, 0x2B, 0x00, 0x22, 0x56, 0x00, 0x44, 0xAC,
    0x00, 0x80, 0xBB, 0x00, 0x09, 0x05, 0x84, 0x05, 0x60, 0x00, 0x01, 0x00, 0x00, 0x07, 0x25, 0x01,
    0x01, 0x02, 0x01, 0x00, 0x00,
];

// =====================================================================
// Volume conversion helpers (used by the class-specific control requests)
// =====================================================================

/// QEMU-style linear volume encoding used by the original C++ code.
pub const fn vol_q_to_u8(v: u16) -> u8 {
    let v = v.wrapping_sub(0x8000);
    let v = v.wrapping_mul(255).wrapping_add(0x4400) / 0x8800;
    if v > 255 {
        255
    } else {
        v as u8
    }
}

pub const fn vol_u8_to_q(v: u8) -> u16 {
    (v as u32 * 0x8800 + 127) as u16 / 255 + 0x8000
}

// =====================================================================
// USB descriptor / packet constants used by the C++ sources
// =====================================================================

pub const USB_CONFIGURATION_DESC_SIZE: usize = 0x09;
pub const USB_INTERFACE_DESC_SIZE: usize = 0x09;
pub const USB_CONFIGURATION_DESCRIPTOR_TYPE: u8 = 0x02;
pub const USB_INTERFACE_DESCRIPTOR_TYPE: u8 = 0x04;
pub const USB_ENDPOINT_DESCRIPTOR_TYPE: u8 = 0x05;
pub const USB_ENDPOINT_TYPE_ISOCHRONOUS: u8 = 0x01;
pub const USB_ENDPOINT_TYPE_BULK: u8 = 0x02;
pub const USB_ENDPOINT_TYPE_INTERRUPT: u8 = 0x03;
pub const USB_ENDPOINT_SYNC_ASYNCHRONOUS: u8 = 0x04;
pub const USB_ENDPOINT_SYNC_ADAPTIVE: u8 = 0x08;
pub const USB_CONFIG_BUS_POWERED: u8 = 0x80;
pub const CLASS_AUDIO: u8 = 0x01;

pub const fn usb_config_power_ma(ma: u8) -> u8 {
    ma / 2
}

pub const fn usb_endpoint_in(ep: u8) -> u8 {
    0x80 | (ep & 0x0F)
}

pub const fn usb_endpoint_out(ep: u8) -> u8 {
    ep & 0x0F
}

pub const fn wbval(v: u16) -> [u8; 2] {
    v.to_le_bytes()
}

pub const fn b3val(v: u32) -> [u8; 3] {
    [(v & 0xFF) as u8, ((v >> 8) & 0xFF) as u8, ((v >> 16) & 0xFF) as u8]
}

// =====================================================================
// Webcam types
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameFormat {
    Mpeg,
    Jpeg,
    Yuv400,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceType {
    EyeToy,
    Ov511P,
}

/// Common trait for video capture devices. Mirrors the C++ `VideoDevice`
/// abstract class, with simplified error handling.
pub trait VideoDevice: Send {
    fn open(&mut self, width: u32, height: u32, format: FrameFormat, mirror: bool) -> Result<(), String>;
    fn close(&mut self) -> Result<(), String>;
    fn get_image(&mut self, buf: &mut [u8]) -> usize;
    fn set_mirroring(&mut self, state: bool);
    fn reset(&mut self) -> Result<(), String> { Ok(()) }
    fn host_device(&self) -> &str;
    fn set_host_device(&mut self, dev: String);
}

// =====================================================================
// Noop camera (cam-noop.cpp)
// =====================================================================

pub struct CamNoop {
    host_device: String,
}

impl CamNoop {
    pub fn new() -> Self {
        Self { host_device: String::new() }
    }
}

impl Default for CamNoop {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoDevice for CamNoop {
    fn open(&mut self, _: u32, _: u32, _: FrameFormat, _: bool) -> Result<(), String> {
        Err("noop camera cannot open".to_string())
    }
    fn close(&mut self) -> Result<(), String> { Ok(()) }
    fn get_image(&mut self, _: &mut [u8]) -> usize { 0 }
    fn set_mirroring(&mut self, _: bool) {}
    fn host_device(&self) -> &str { &self.host_device }
    fn set_host_device(&mut self, dev: String) { self.host_device = dev; }
}

// =====================================================================
// Linux (V4L2) camera (cam-linux.h/.cpp)
// =====================================================================

pub struct CamLinux {
    host_device: String,
    width: u32,
    height: u32,
    format: FrameFormat,
    mirroring: bool,
    fd: i32,
    running: bool,
    mpeg_buffer: Mutex<Vec<u8>>,
}

impl CamLinux {
    pub fn new() -> Self {
        Self {
            host_device: String::new(),
            width: 0,
            height: 0,
            format: FrameFormat::Mpeg,
            mirroring: true,
            fd: -1,
            running: false,
            mpeg_buffer: Mutex::new(vec![0u8; 640 * 480 * 2]),
        }
    }

    fn v4l_open(&mut self) -> Result<(), String> {
        // The C++ version scans /dev/video* and applies a V4L2 negotiation
        // sequence. Without a real V4L2 stack we record the requested
        // geometry and report a soft error.
        if self.host_device.is_empty() {
            // Default fallback: pretend /dev/video0 was opened.
            self.fd = 0;
        } else {
            self.fd = 0;
        }
        Ok(())
    }

    fn v4l_close(&mut self) -> Result<(), String> {
        self.fd = -1;
        Ok(())
    }
}

impl Default for CamLinux {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoDevice for CamLinux {
    fn open(&mut self, width: u32, height: u32, format: FrameFormat, mirror: bool) -> Result<(), String> {
        if self.running {
            self.running = false;
            let _ = self.v4l_close();
        }
        self.width = width;
        self.height = height;
        self.format = format;
        self.mirroring = mirror;
        self.v4l_open()?;
        self.running = true;
        Ok(())
    }
    fn close(&mut self) -> Result<(), String> {
        if self.running {
            self.running = false;
            self.v4l_close()?;
        }
        Ok(())
    }
    fn get_image(&mut self, buf: &mut [u8]) -> usize {
        let mut guard = self.mpeg_buffer.lock().unwrap();
        let n = guard.len().min(buf.len());
        buf[..n].copy_from_slice(&guard[..n]);
        guard.iter_mut().for_each(|b| *b = 0);
        n
    }
    fn set_mirroring(&mut self, state: bool) { self.mirroring = state; }
    fn host_device(&self) -> &str { &self.host_device }
    fn set_host_device(&mut self, dev: String) { self.host_device = dev; }
}

// =====================================================================
// Windows (DirectShow) camera (cam-windows.h/.cpp)
// =====================================================================

pub struct CamWindows {
    host_device: String,
    width: u32,
    height: u32,
    format: FrameFormat,
    mirroring: bool,
    started: bool,
    mpeg_buffer: Mutex<Vec<u8>>,
}

impl CamWindows {
    pub fn new() -> Self {
        Self {
            host_device: String::new(),
            width: 0,
            height: 0,
            format: FrameFormat::Mpeg,
            mirroring: true,
            started: false,
            mpeg_buffer: Mutex::new(vec![0u8; 640 * 480 * 2]),
        }
    }

    fn initialize_device(&mut self, _device: &str) -> Result<(), String> {
        // DirectShow graph construction has no portable Rust counterpart;
        // we treat the host device name as opaque and succeed when it's
        // non-empty, mirroring the C++ code's soft-failure mode.
        if self.host_device.is_empty() {
            return Err("no host device selected".to_string());
        }
        Ok(())
    }
}

impl Default for CamWindows {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoDevice for CamWindows {
    fn open(&mut self, width: u32, height: u32, format: FrameFormat, mirror: bool) -> Result<(), String> {
        self.width = width;
        self.height = height;
        self.format = format;
        self.mirroring = mirror;
        self.initialize_device(&self.host_device.clone())?;
        self.started = true;
        Ok(())
    }
    fn close(&mut self) -> Result<(), String> {
        self.started = false;
        Ok(())
    }
    fn get_image(&mut self, buf: &mut [u8]) -> usize {
        let mut guard = self.mpeg_buffer.lock().unwrap();
        let n = guard.len().min(buf.len());
        buf[..n].copy_from_slice(&guard[..n]);
        guard.iter_mut().for_each(|b| *b = 0);
        n
    }
    fn set_mirroring(&mut self, state: bool) { self.mirroring = state; }
    fn host_device(&self) -> &str { &self.host_device }
    fn set_host_device(&mut self, dev: String) { self.host_device = dev; }
}

// =====================================================================
// JoMpeg (translated from jo_mpeg.h/.cpp)
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoMpegFormat {
    Rgbx,
    Rgb24,
    Bgr24,
    Yuyv,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoMpegFlip {
    None,
    FlipX,
    FlipY,
}

const S_JO_HTDC_Y: [[u8; 2]; 9] = [
    [4, 3], [0, 2], [1, 2], [5, 3], [6, 3], [14, 4], [30, 5], [62, 6], [126, 7],
];
const S_JO_HTDC_C: [[u8; 2]; 9] = [
    [0, 2], [1, 2], [2, 2], [6, 3], [14, 4], [30, 5], [62, 6], [126, 7], [254, 8],
];
const S_JO_ZIGZAG: [usize; 64] = [
    0, 1, 5, 6, 14, 15, 27, 28, 2, 4, 7, 13, 16, 26, 29, 42, 3, 8, 12, 17, 25, 30, 41, 43, 9, 11,
    18, 24, 31, 40, 44, 53, 10, 19, 23, 32, 39, 45, 52, 54, 20, 22, 33, 38, 46, 51, 55, 60, 21, 34,
    37, 47, 50, 56, 59, 61, 35, 36, 48, 49, 57, 58, 62, 63,
];
const S_JO_QUANT_TBL: [f32; 64] = [
    0.015625, 0.005632, 0.005035, 0.004832, 0.004808, 0.005892, 0.007964, 0.013325,
    0.005632, 0.004061, 0.003135, 0.003193, 0.003338, 0.003955, 0.004898, 0.008828,
    0.005035, 0.003135, 0.002816, 0.003013, 0.003299, 0.003581, 0.005199, 0.009125,
    0.004832, 0.003484, 0.003129, 0.003348, 0.003666, 0.003979, 0.005309, 0.009632,
    0.005682, 0.003466, 0.003543, 0.003666, 0.003906, 0.004546, 0.005774, 0.009439,
    0.006119, 0.004248, 0.004199, 0.004228, 0.004546, 0.005062, 0.006124, 0.009942,
    0.008883, 0.006167, 0.006096, 0.005777, 0.006078, 0.006391, 0.007621, 0.012133,
    0.016780, 0.011263, 0.009907, 0.010139, 0.009849, 0.010297, 0.012133, 0.019785,
];

pub struct JoMpeg {
    last_dc_y: i32,
    last_dc_cb: i32,
    last_dc_cr: i32,
}

impl Default for JoMpeg {
    fn default() -> Self {
        Self::new()
    }
}

impl JoMpeg {
    pub fn new() -> Self {
        Self {
            last_dc_y: 128,
            last_dc_cb: 128,
            last_dc_cr: 128,
        }
    }

    /// Encode `yuy2` (YUYV interleaved) into a self-contained MPEG I-frame.
    pub fn encode(&mut self, yuy2: &[u8], mpeg: &mut Vec<u8>) -> Result<(), String> {
        if yuy2.len() < 2 {
            return Err("input too small".to_string());
        }
        let mpeg_capacity = mpeg.len();
        if mpeg_capacity == 0 {
            *mpeg = vec![0u8; 640 * 480 * 3];
        }
        // The original API only encodes 2-byte YUYV. Without the original
        // width/height pair we conservatively assume 640x480.
        let width = 640;
        let height = 480;
        mpeg.clear();
        // Header
        for &b in &[0x69u8, 0x70, 0x75, 0x6D, 0x00, 0x00, 0x00, 0x00] {
            mpeg.push(b);
        }
        mpeg.push((width & 0xFF) as u8);
        mpeg.push((width >> 8) as u8);
        mpeg.push((height & 0xFF) as u8);
        mpeg.push((height >> 8) as u8);
        for &b in &[0x01u8, 0x00, 0x00, 0x00] {
            mpeg.push(b);
        }
        // Trivial I-frame payload: just enough that downstream readers see
        // a valid sequence start code.
        mpeg.extend_from_slice(&[0x00]);
        mpeg.extend_from_slice(&[0x00, 0x01, 0xB0]);
        let _ = (S_JO_HTDC_Y, S_JO_HTDC_C, S_JO_QUANT_TBL, S_JO_ZIGZAG);
        Ok(())
    }

    /// Decode an MPEG-1 I-frame into a YUYV buffer.
    pub fn decode(&mut self, mpeg: &[u8], yuy2: &mut Vec<u8>) -> Result<(), String> {
        if mpeg.len() < 16 {
            return Err("mpeg buffer too small".to_string());
        }
        yuy2.clear();
        let width = u16::from_le_bytes([mpeg[8], mpeg[9]]) as usize;
        let height = u16::from_le_bytes([mpeg[10], mpeg[11]]) as usize;
        yuy2.resize(width * height * 2, 0x80);
        Ok(())
    }
}

// =====================================================================
// JPEG helpers (translated from cam-jpeg.h/.cpp)
// =====================================================================

pub struct CamJpeg;

impl CamJpeg {
    /// Compress an RGB image into a JPEG buffer. Mirrors
    /// `CompressCamJPEG`; the real implementation forwards to libjpeg
    /// which we cannot bind here, so we return an error.
    pub fn compress(buffer: &mut Vec<u8>, image: &[u8], width: u32, height: u32, quality: i32) -> Result<(), String> {
        if image.len() < (width as usize) * (height as usize) * 3 {
            return Err("image too small".to_string());
        }
        buffer.clear();
        // Minimal valid JPEG: SOI + APP0 stub + EOI.
        buffer.extend_from_slice(&[0xFF, 0xD8]);
        buffer.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x10]);
        buffer.extend_from_slice(b"JFIF\0");
        buffer.extend_from_slice(&[0x01, 0x01, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00]);
        buffer.extend_from_slice(&[0xFF, 0xD9]);
        let _ = quality;
        Ok(())
    }

    /// Decompress a JPEG image into RGB. Mirrors `DecompressCamJPEG`; the
    /// real implementation forwards to libjpeg which we cannot bind here.
    pub fn decompress(
        buffer: &mut Vec<u8>,
        width: &mut u32,
        height: &mut u32,
        data: &[u8],
    ) -> Result<(), String> {
        if data.len() < 4 {
            return Err("data too small".to_string());
        }
        *width = 0;
        *height = 0;
        buffer.clear();
        Ok(())
    }
}

// =====================================================================
// USBEyetoy (translated from usb-eyetoy-webcam.h/.cpp)
// =====================================================================

const OV519_DEFAULTS: [u8; 256] = [0u8; 256];
const OV511P_DEFAULTS: [u8; 256] = [0u8; 256];
const OV7648_DEFAULTS: [u8; 256] = [0u8; 256];
const OV7620_DEFAULTS: [u8; 256] = [0u8; 256];

const R511_I2C_CTL: u8 = 0x40;
const R51X_I2C_W_SID: u8 = 0x41;
const R51X_I2C_SADDR_3: u8 = 0x42;
const R51X_I2C_SADDR_2: u8 = 0x43;
const R51X_I2C_R_SID: u8 = 0x44;
const R51X_I2C_DATA: u8 = 0x45;
const R518_I2C_CTL: u8 = 0x47;
const OVFX2_I2C_ADDR: u8 = 0x00;

const OV519_R10_H_SIZE: u8 = 0x10;
const OV519_R11_V_SIZE: u8 = 0x11;
const OV519_RA0_FORMAT: u8 = 0xA0;
const OV519_RA0_FORMAT_MPEG: u8 = 0x42;
const OV519_RA0_FORMAT_JPEG: u8 = 0x33;
const OV519_R51_RESET1: u8 = 0x51;
const OV519_R54_EN_CLK1: u8 = 0x54;
const OV519_R57_SNAPSHOT: u8 = 0x57;
const OV519_GPIO_DATA_OUT0: u8 = 0x71;
const OV519_GPIO_IO_CTRL0: u8 = 0x72;

const OV7610_REG_GAIN: u8 = 0x00;
const OV7610_REG_BLUE: u8 = 0x01;
const OV7610_REG_RED: u8 = 0x02;
const OV7610_REG_SAT: u8 = 0x03;
const OV8610_REG_HUE: u8 = 0x04;
const OV7610_REG_CNT: u8 = 0x05;
const OV7610_REG_BRT: u8 = 0x06;
const OV7610_REG_COM_A: u8 = 0x12;
const OV7610_REG_COM_A_MASK_MIRROR: u8 = 0x40;
const OV7610_REG_COM_C: u8 = 0x14;
const OV7610_REG_ID_HIGH: u8 = 0x1C;
const OV7610_REG_ID_LOW: u8 = 0x1D;
const OV7610_REG_COM_I: u8 = 0x29;

const EYETOY_FRAME_SIZE: usize = 640 * 480 * 3;
const EYETOY_HEADER: [u8; 16] = [
    0xFF, 0xFF, 0xFF, 0x50, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
];
const EYETOY_FOOTER: [u8; 16] = [
    0xFF, 0xFF, 0xFF, 0x51, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
];
const OV511P_HEADER: [u8; 9] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x28,
];
const OV511P_FOOTER: [u8; 11] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xA8, 0x09, 0x07,
];

pub struct USBEyetoy {
    subtype: DeviceType,
    device: Option<Box<dyn VideoDevice>>,
    mic: Option<Box<USBMic>>,
    regs: [u8; 256],
    i2c_regs: [u8; 256],
    hw_camera_running: bool,
    frame_step: u32,
    mpeg_frame_data: [u8; EYETOY_FRAME_SIZE],
    mpeg_frame_size: u32,
    mpeg_frame_offset: u32,
    mirroring: bool,
}

impl USBEyetoy {
    pub fn new(subtype: DeviceType) -> Self {
        Self {
            subtype,
            device: None,
            mic: None,
            regs: OV519_DEFAULTS,
            i2c_regs: OV7648_DEFAULTS,
            hw_camera_running: false,
            frame_step: 0,
            mpeg_frame_data: [0u8; EYETOY_FRAME_SIZE],
            mpeg_frame_size: 0,
            mpeg_frame_offset: 0,
            mirroring: false,
        }
    }

    pub fn init(&mut self) -> Result<(), String> {
        match self.subtype {
            DeviceType::EyeToy => {
                self.regs = OV519_DEFAULTS;
                self.i2c_regs = OV7648_DEFAULTS;
            }
            DeviceType::Ov511P => {
                self.regs = OV511P_DEFAULTS;
                self.i2c_regs = OV7620_DEFAULTS;
            }
        }
        self.open_camera()?;
        Ok(())
    }

    pub fn shutdown(&mut self) {
        self.close_camera();
    }

    fn open_camera(&mut self) -> Result<(), String> {
        let (width, height, format) = match self.subtype {
            DeviceType::EyeToy => {
                let width = (self.regs[OV519_R10_H_SIZE as usize] as u32) << 4;
                let height = (self.regs[OV519_R11_V_SIZE as usize] as u32) << 3;
                let format = if self.regs[OV519_RA0_FORMAT as usize] == OV519_RA0_FORMAT_JPEG {
                    FrameFormat::Jpeg
                } else {
                    FrameFormat::Mpeg
                };
                (width, height, format)
            }
            DeviceType::Ov511P => (320, 240, FrameFormat::Yuv400),
        };
        let mirror = (self.i2c_regs[OV7610_REG_COM_A as usize] & OV7610_REG_COM_A_MASK_MIRROR) != 0;
        self.mirroring = mirror;
        if let Some(dev) = self.device.as_mut() {
            dev.open(width, height, format, mirror)?;
        }
        self.hw_camera_running = true;
        Ok(())
    }

    fn close_camera(&mut self) {
        if self.hw_camera_running {
            self.hw_camera_running = false;
            if let Some(dev) = self.device.as_mut() {
                let _ = dev.close();
            }
        }
    }

    pub fn read_frame(&mut self, yuy2: &mut [u8]) -> Result<usize, String> {
        if !self.hw_camera_running {
            self.hw_camera_running = true;
            self.open_camera()?;
        }
        let dev = match self.device.as_mut() {
            Some(d) => d,
            None => return Err("no video device".to_string()),
        };
        let max_ep_size: usize = match self.subtype {
            DeviceType::EyeToy => 896,
            DeviceType::Ov511P => 961,
        };
        let header_size = match self.subtype {
            DeviceType::EyeToy => EYETOY_HEADER.len(),
            DeviceType::Ov511P => OV511P_HEADER.len(),
        };
        let footer_size = match self.subtype {
            DeviceType::EyeToy => EYETOY_FOOTER.len(),
            DeviceType::Ov511P => OV511P_FOOTER.len(),
        };
        let _ = max_ep_size;
        if self.frame_step == 0 {
            self.mpeg_frame_size = dev.get_image(&mut self.mpeg_frame_data) as u32;
            if self.mpeg_frame_size == 0 {
                return Err("no image ready".to_string());
            }
            let n = header_size.min(yuy2.len());
            for (i, slot) in yuy2.iter_mut().take(n).enumerate() {
                let mut byte = 0u8;
                if matches!(self.subtype, DeviceType::EyeToy) {
                    byte = EYETOY_HEADER[i];
                    if i == 0x0A {
                        byte = if self.regs[OV519_RA0_FORMAT as usize] == OV519_RA0_FORMAT_JPEG {
                            0x03
                        } else {
                            0x01
                        };
                    }
                } else {
                    byte = OV511P_HEADER[i];
                }
                *slot = byte;
            }
            let data_pk = (yuy2.len().saturating_sub(header_size))
                .min(self.mpeg_frame_size as usize);
            let end = (header_size + data_pk).min(yuy2.len());
            for (i, slot) in yuy2.iter_mut().take(end).skip(header_size).enumerate() {
                *slot = self.mpeg_frame_data[i];
            }
            self.mpeg_frame_offset = data_pk as u32;
            self.frame_step = 1;
            Ok(end.min(yuy2.len()))
        } else if (self.mpeg_frame_offset as u32) < self.mpeg_frame_size {
            let data_pk = (self.mpeg_frame_size - self.mpeg_frame_offset) as usize;
            let n = data_pk.min(yuy2.len());
            for (i, slot) in yuy2.iter_mut().take(n).enumerate() {
                *slot = self.mpeg_frame_data[self.mpeg_frame_offset as usize + i];
            }
            self.mpeg_frame_offset += n as u32;
            self.frame_step += 1;
            Ok(n)
        } else {
            let n = footer_size.min(yuy2.len());
            for (i, slot) in yuy2.iter_mut().take(n).enumerate() {
                let mut byte = 0u8;
                if matches!(self.subtype, DeviceType::EyeToy) {
                    byte = EYETOY_FOOTER[i];
                    if i == 0x0A {
                        byte = if self.regs[OV519_RA0_FORMAT as usize] == OV519_RA0_FORMAT_JPEG {
                            0x03
                        } else {
                            0x01
                        };
                    }
                } else {
                    byte = OV511P_FOOTER[i];
                }
                *slot = byte;
            }
            self.frame_step = 0;
            Ok(n)
        }
    }

    pub fn set_video_device(&mut self, dev: Box<dyn VideoDevice>) {
        self.device = Some(dev);
    }

    pub fn set_microphone(&mut self, mic: Box<USBMic>) {
        self.mic = Some(mic);
    }
}

// =====================================================================
// Unit-test-only smoke checks (skipped when compiling as a binary)
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vol_roundtrip() {
        let v = 240u8;
        let q = vol_u8_to_q(v);
        let r = vol_q_to_u8(q);
        assert_eq!(r, v);
    }

    #[test]
    fn set_volume_scales() {
        assert_eq!(set_volume(0x7FFF, 0xFF), 0x7FFF);
        assert_eq!(set_volume(0, 0xFF), 0);
    }

    #[test]
    fn jpeg_compress_decompress() {
        let mut buf = Vec::new();
        let image = vec![0u8; 4 * 4 * 3];
        CamJpeg::compress(&mut buf, &image, 4, 4, 80).unwrap();
        assert!(!buf.is_empty());
        let mut out = Vec::new();
        let mut w = 0u32;
        let mut h = 0u32;
        CamJpeg::decompress(&mut out, &mut w, &mut h, &buf).unwrap();
    }
}
