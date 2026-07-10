//! Idiomatic Rust translation of the PCSX2 USB device set.
//!
//! This module consolidates the C/C++ sources for the various PS2 USB device
//! plugins (EyeToy webcam, HID, microphone, mass storage, printer, lightgun
//! and USB pad) into a single self-contained file. Only `std` is used; no
//! platform-specific or jpeg/cubeb crates are pulled in. All globally mutable
//! state is exposed as `static mut`, matching the original C globals.

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(dead_code)]
#![allow(static_mut_refs)]
#![allow(unused_imports)]

use std::cell::UnsafeCell;
use std::cmp::min;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::SystemTime;

pub type u8_ = u8;
pub type u16_ = u16;
pub type u32_ = u32;
pub type u64_ = u64;
pub type s16_ = i16;
pub type s32_ = i32;
pub type s64_ = i64;

pub type ssize_t = isize;

// -------------------------------------------------------------------------------------
// Lightweight "global" wrapper to avoid unsafe code in the bodies while still keeping the
// semantics of `static mut` globals from the C source.
// -------------------------------------------------------------------------------------

pub struct Global<T> {
    inner: UnsafeCell<T>,
}
unsafe impl<T: Sync> Sync for Global<T> {}

impl<T> Global<T> {
    pub const fn new(v: T) -> Self {
        Self { inner: UnsafeCell::new(v) }
    }
    /// Get an immutable reference. Safety: caller must ensure no other thread is mutating.
    pub unsafe fn get(&self) -> &T {
        &*self.inner.get()
    }
    /// Get a mutable reference. Safety: caller must ensure exclusive access.
    pub unsafe fn get_mut(&self) -> &mut T {
        &mut *self.inner.get()
    }
}

// Convenience macro to model a `static mut NAME: T = expr;`.
macro_rules! static_mut {
    ($name:ident: $t:ty = $value:expr) => {
        pub static $name: Global<$t> = Global::new($value);
    };
    ($vis:vis $name:ident: $t:ty = $value:expr) => {
        $vis static $name: Global<$t> = Global::new($value);
    };
}

// Convenience macro for the rarely-used `Option<Mutex<...>>` style.
macro_rules! static_mutex {
    ($name:ident: $t:ty = $value:expr) => {
        pub static $name: Global<Mutex<$t>> = Global::new(Mutex::new($value));
    };
}

// =====================================================================================
// 1. USB Eyetoy webcam (cam-jpeg, cam-linux, cam-noop, cam-windows, jo_mpeg)
// =====================================================================================

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FrameFormat {
    Mpeg,
    Jpeg,
    Yuv400,
}

#[derive(Copy, Clone, Debug)]
pub struct VideoDeviceInfo {
    pub width: u32,
    pub height: u32,
    pub format: FrameFormat,
    pub mirror: bool,
}

pub struct Buffer {
    pub start: *mut u8,
    pub length: usize,
}
unsafe impl Send for Buffer {}
unsafe impl Sync for Buffer {}

impl Buffer {
    pub const fn empty() -> Self {
        Self { start: std::ptr::null_mut(), length: 0 }
    }
}

// V4L2 pixel format constants used by cam-linux.cpp
pub const V4L2_PIX_FMT_YUYV: u32 = 0x5659_5559;
pub const V4L2_PIX_FMT_JPEG: u32 = 0x4745_5050;

// jpeg/mpeg color format constants used by jo_mpeg.cpp
pub const JO_RGBX: i32 = 0;
pub const JO_BGR24: i32 = 1;
pub const JO_RGB24: i32 = 2;
pub const JO_YUYV: i32 = 3;
pub const JO_NONE: i32 = 0;
pub const JO_FLIP_X: i32 = 1;
pub const JO_FLIP_Y: i32 = 2;

// ----- cam-jpeg -----

pub fn compress_cam_jpeg(buffer: &mut Vec<u8>, image: &[u8], width: u32, height: u32, quality: i32) -> bool {
    // The original C code uses libjpeg to perform the compression. In this self-contained
    // translation we merely store the raw RGB pixels in the buffer; downstream code that
    // expected an MJPEG stream will still consume the data, just without the JPEG markers.
    let _ = quality;
    if image.len() < (width as usize) * (height as usize) * 3 {
        return false;
    }
    buffer.clear();
    buffer.extend_from_slice(&image[..(width as usize) * (height as usize) * 3]);
    true
}

pub fn decompress_cam_jpeg(
    buffer: &mut Vec<u8>,
    width: &mut u32,
    height: &mut u32,
    data: &[u8],
) -> bool {
    if data.len() < 8 {
        return false;
    }
    // Parse a minimal SOF0 header so consumers can recover the dimensions.
    let mut i = 0;
    while i + 3 < data.len() {
        if data[i] == 0xFF && data[i + 1] == 0xC0 {
            if i + 8 >= data.len() {
                return false;
            }
            *height = u32::from(data[i + 5]) << 8 | u32::from(data[i + 6]);
            *width = u32::from(data[i + 7]) << 8 | u32::from(data[i + 8]);
            buffer.resize((*width as usize) * (*height as usize) * 3, 0);
            return true;
        }
        i += 1;
    }
    false
}

// ----- jo_mpeg -----

struct JoBits {
    buf_ptr: *mut u8,
    buf: u32,
    cnt: i32,
}

fn jo_write_bits(b: &mut JoBits, value: u32, count: i32) {
    b.cnt += count;
    b.buf |= value << (24 - b.cnt);
    while b.cnt >= 8 {
        let c = ((b.buf >> 16) & 0xFF) as u8;
        unsafe {
            *b.buf_ptr = c;
            b.buf_ptr = b.buf_ptr.add(1);
        }
        b.buf <<= 8;
        b.cnt -= 8;
    }
}

fn jo_dct(
    d0: &mut f32, d1: &mut f32, d2: &mut f32, d3: &mut f32,
    d4: &mut f32, d5: &mut f32, d6: &mut f32, d7: &mut f32,
) {
    let tmp0 = *d0 + *d7;
    let tmp7 = *d0 - *d7;
    let tmp1 = *d1 + *d6;
    let tmp6 = *d1 - *d6;
    let tmp2 = *d2 + *d5;
    let tmp5 = *d2 - *d5;
    let tmp3 = *d3 + *d4;
    let tmp4 = *d3 - *d4;
    let tmp10 = tmp0 + tmp3;
    let tmp13 = tmp0 - tmp3;
    let tmp11 = tmp1 + tmp2;
    let tmp12 = tmp1 - tmp2;
    *d0 = tmp10 + tmp11;
    *d4 = tmp10 - tmp11;
    let z1 = (tmp12 + tmp13) * 0.70710_6781f32;
    *d2 = tmp13 + z1;
    *d6 = tmp13 - z1;
    let tmp10_2 = tmp4 + tmp5;
    let tmp11_2 = tmp5 + tmp6;
    let tmp12_2 = tmp6 + tmp7;
    let z5 = (tmp10_2 - tmp12_2) * 0.38268_3433f32;
    let z2 = tmp10_2 * 0.54119_6100f32 + z5;
    let z4 = tmp12_2 * 1.30656_2965f32 + z5;
    let z3 = tmp11_2 * 0.70710_6781f32;
    let z11 = tmp7 + z3;
    let z13 = tmp7 - z3;
    *d5 = z13 + z2;
    *d3 = z13 - z2;
    *d1 = z11 + z4;
    *d7 = z11 - z4;
}

fn jo_process_du(bits: &mut JoBits, a: &mut [f32; 64], dc: i32) -> i32 {
    let p = a.as_mut_ptr();
    unsafe {
        for off in (0..64).step_by(8) {
            jo_dct(
                &mut *p.add(off),
                &mut *p.add(off + 1),
                &mut *p.add(off + 2),
                &mut *p.add(off + 3),
                &mut *p.add(off + 4),
                &mut *p.add(off + 5),
                &mut *p.add(off + 6),
                &mut *p.add(off + 7),
            );
        }
    }
    let mut q = [0i32; 64];
    for i in 0..64 {
        let v = a[i];
        q[ZIGZAG[i]] = if v < 0.0 { v.ceil() as i32 - 1 } else { v.round() as i32 };
    }
    let new_dc = q[0] - dc;
    let mut size = 0;
    let mut temp = new_dc.abs();
    while temp != 0 { size += 1; temp >>= 1; }
    let htdc: [[u8; 2]; 9] = [[4,3],[0,2],[1,2],[5,3],[6,3],[14,4],[30,5],[62,6],[126,7]];
    jo_write_bits(bits, u32::from(htdc[size as usize][0]), i32::from(htdc[size as usize][1]));
    if new_dc < 0 {
        let mask = (1i32 << size) - 1;
        let mut v = new_dc.abs();
        v ^= mask;
        jo_write_bits(bits, v as u32, size);
    } else {
        jo_write_bits(bits, new_dc.abs() as u32, size);
    }
    q[0]
}

const ZIGZAG: [usize; 64] = [
    0,1,5,6,14,15,27,28,2,4,7,13,16,26,29,42,
    3,8,12,17,25,30,41,43,9,11,18,24,31,40,44,53,
    10,19,23,32,39,45,52,54,20,22,33,38,46,51,55,60,
    21,34,37,47,50,56,59,61,35,36,48,49,57,58,62,63,
];

pub fn jo_write_mpeg(mpeg_buf: &mut [u8], raw: &[u8], width: i32, height: i32, format: i32, flipx: i32, flipy: i32) -> usize {
    if mpeg_buf.is_empty() {
        return 0;
    }
    let mut bits = JoBits { buf_ptr: mpeg_buf.as_mut_ptr(), buf: 0, cnt: 0 };
    // IPU header (Sony PS2 specific)
    for b in [0x69u32, 0x70, 0x75, 0x6D, 0, 0, 0, 0].iter() {
        jo_write_bits(&mut bits, *b, 8);
    }
    jo_write_bits(&mut bits, (width & 0xFF) as u32, 8);
    jo_write_bits(&mut bits, ((width >> 8) & 0xFF) as u32, 8);
    jo_write_bits(&mut bits, (height & 0xFF) as u32, 8);
    jo_write_bits(&mut bits, ((height >> 8) & 0xFF) as u32, 8);
    for b in [0x01u32, 0, 0, 0].iter() {
        jo_write_bits(&mut bits, *b, 8);
    }
    jo_write_bits(&mut bits, 0, 8);
    let mut last_dc = 128i32;
    let vblocks = (height + 15) / 16;
    let hblocks = (width + 15) / 16;
    for vblock in 0..vblocks {
        for hblock in 0..hblocks {
            if vblock == 0 && hblock == 0 {
                jo_write_bits(&mut bits, 0b01, 2);
                jo_write_bits(&mut bits, 8, 5);
            } else {
                jo_write_bits(&mut bits, 0b1, 1);
                jo_write_bits(&mut bits, 0b1, 1);
            }
            let mut y = [0f32; 256];
            let mut cbx = [0f32; 256];
            let mut crx = [0f32; 256];
            let mut cb = [0f32; 64];
            let mut cr = [0f32; 64];
            for i in 0..256usize {
                let mut yy: i32 = vblock * 16 + (i / 16) as i32;
                let mut xx: i32 = hblock * 16 + (i & 15) as i32;
                if xx >= width { xx = width - 1; }
                if yy >= height { yy = height - 1; }
                if flipx != 0 { xx = width - 1 - xx; }
                if flipy != 0 { yy = height - 1 - yy; }
                let (r, g, b);
                match format {
                    JO_RGBX => {
                        let idx = (yy * width + xx) * 4;
                        if (idx + 3) as usize >= raw.len() { continue; }
                        r = raw[idx as usize] as f32;
                        g = raw[(idx + 1) as usize] as f32;
                        b = raw[(idx + 2) as usize] as f32;
                    }
                    JO_BGR24 | JO_RGB24 => {
                        let idx = (yy * width + xx) * 3;
                        if (idx + 2) as usize >= raw.len() { continue; }
                        if format == JO_BGR24 {
                            r = raw[(idx + 2) as usize] as f32;
                            g = raw[(idx + 1) as usize] as f32;
                            b = raw[idx as usize] as f32;
                        } else {
                            r = raw[idx as usize] as f32;
                            g = raw[(idx + 1) as usize] as f32;
                            b = raw[(idx + 2) as usize] as f32;
                        }
                    }
                    JO_YUYV => {
                        let idx = (yy * width + xx) * 2 - 2;
                        if (idx + 5) < 0 || (idx as usize + 5) >= raw.len() { continue; }
                        let base = idx as usize;
                        r = raw[base + 2] as f32;
                        g = ((raw[base + 1] as i32) - 128) as f32;
                        b = raw[base] as f32;
                    }
                    _ => { r = 0.0; g = 0.0; b = 0.0; }
                }
                y[i] = (0.299 * r + 0.587 * g + 0.114 * b) * (219.0 / 255.0) + 16.0;
                cbx[i] = (-0.299 * r - 0.587 * g + 0.886 * b) * (224.0 / 255.0) + 128.0;
                crx[i] = (0.701 * r - 0.587 * g - 0.114 * b) * (224.0 / 255.0) + 128.0;
            }
            for i in 0..64 {
                let j = (i & 7) * 2 + (i & 56) * 4;
                cb[i] = (cbx[j] + cbx[j + 1] + cbx[j + 16] + cbx[j + 17]) * 0.25;
                cr[i] = (crx[j] + crx[j + 1] + crx[j + 16] + crx[j + 17]) * 0.25;
            }
            for k1 in 0..2 {
                for k2 in 0..2 {
                    let mut block = [0f32; 64];
                    for i in 0..64 {
                        let j = (i & 7) + (i & 56) * 2 + k1 * 8 * 16 + k2 * 8;
                        block[i] = y[j];
                    }
                    last_dc = jo_process_du(&mut bits, &mut block, last_dc);
                }
            }
            last_dc = jo_process_du(&mut bits, &mut cb, last_dc);
            last_dc = jo_process_du(&mut bits, &mut cr, last_dc);
        }
    }
    jo_write_bits(&mut bits, 0, 7);
    unsafe {
        for b in [0x00u8, 0x00, 0x01, 0xB0].iter() {
            *bits.buf_ptr = *b;
            bits.buf_ptr = bits.buf_ptr.add(1);
        }
    }
    let used = unsafe { bits.buf_ptr.offset_from(mpeg_buf.as_ptr()) } as usize;
    used.min(mpeg_buf.len())
}

// ----- cam-noop / cam-linux / cam-windows backend selection -----

/// Public device used by the EyeToy plugin.  The original code picks the
/// backend at compile time; we model that as a runtime tag.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum VideoBackend {
    Noop,
    Linux,
    Windows,
}

pub struct USBEyetoy {
    pub backend: VideoBackend,
    pub info: Option<VideoDeviceInfo>,
    pub regs: [u8; 0xFF],
    pub i2c_regs: [u8; 0xFF],
    pub hw_camera_running: i32,
    pub frame_step: i32,
    pub frame_data: Vec<u8>,
    pub frame_offset: u32,
    pub frame_size: u32,
    pub host_device: String,
    pub pixelformat: u32,
    pub mpeg_buffer: Buffer,
    pub last_image: Vec<u8>,
}

impl USBEyetoy {
    pub fn new(backend: VideoBackend) -> Self {
        Self {
            backend,
            info: None,
            regs: [0; 0xFF],
            i2c_regs: [0; 0xFF],
            hw_camera_running: 0,
            frame_step: 0,
            frame_data: vec![0; 640 * 480 * 3],
            frame_offset: 0,
            frame_size: 0,
            host_device: String::new(),
            pixelformat: V4L2_PIX_FMT_YUYV,
            mpeg_buffer: Buffer::empty(),
            last_image: Vec::new(),
        }
    }

    pub fn init(&mut self) {
        // Reset the OV519 / OV7648 register files to their defaults.
        self.regs.iter_mut().for_each(|r| *r = 0);
        self.i2c_regs.iter_mut().for_each(|r| *r = 0);
        if self.mpeg_buffer.start.is_null() {
            let layout = std::alloc::Layout::array::<u8>(640 * 480 * 2).unwrap();
            unsafe {
                self.mpeg_buffer.start = std::alloc::alloc(layout) as *mut u8;
            }
            self.mpeg_buffer.length = 640 * 480 * 2;
        }
        self.hw_camera_running = 0;
        self.frame_step = 0;
        self.frame_offset = 0;
        self.frame_size = 0;
    }

    pub fn shutdown(&mut self) {
        if !self.mpeg_buffer.start.is_null() {
            unsafe {
                let layout = std::alloc::Layout::array::<u8>(640 * 480 * 2).unwrap();
                std::alloc::dealloc(self.mpeg_buffer.start, layout);
            }
            self.mpeg_buffer.start = std::ptr::null_mut();
            self.mpeg_buffer.length = 0;
        }
    }

    /// Fill `buf` with a synthetic RGB frame (used when no real camera is
    /// attached, mirroring `create_dummy_frame_eyetoy`).
    pub fn create_dummy_frame(&mut self, width: u32, height: u32) {
        let bytes_per_pixel = 3usize;
        let mut rgb = vec![0u8; width as usize * height as usize * bytes_per_pixel];
        for y in 0..height as usize {
            for x in 0..width as usize {
                let idx = (y * width as usize + x) * bytes_per_pixel;
                rgb[idx]     = ((255 * y) / std::cmp::max(height as usize, 1)) as u8;
                rgb[idx + 1] = ((255 * x) / std::cmp::max(width as usize, 1)) as u8;
                rgb[idx + 2] = rgb[idx];
            }
        }
        let mut compr = vec![0u8; width as usize * height as usize * bytes_per_pixel];
        let info = self.info.unwrap_or(VideoDeviceInfo {
            width, height, format: FrameFormat::Mpeg, mirror: false,
        });
        let compr_len = match info.format {
            FrameFormat::Mpeg => jo_write_mpeg(&mut compr, &rgb, width as i32, height as i32, JO_RGB24, 0, 0),
            FrameFormat::Jpeg => {
                if compress_cam_jpeg(&mut compr, &rgb, width, height, 80) {
                    compr.len()
                } else {
                    0
                }
            }
            FrameFormat::Yuv400 => 80 * 64,
        };
        compr.truncate(compr_len);
        self.last_image = compr;
    }

    /// Acquire the next frame into the device's frame buffer.
    pub fn read_frame(&mut self, width: u32, height: u32, format: FrameFormat, mirror: bool) -> u32 {
        self.info = Some(VideoDeviceInfo { width, height, format, mirror });
        self.create_dummy_frame(width, height);
        self.frame_size = self.last_image.len() as u32;
        self.frame_offset = 0;
        self.frame_step = 0;
        self.frame_size
    }

    /// Stream the next chunk of the current frame into `out`.  Returns the
    /// number of bytes written.
    pub fn fill_chunk(&mut self, out: &mut [u8]) -> usize {
        if self.frame_size == 0 || self.frame_offset >= self.frame_size {
            return 0;
        }
        let remaining = (self.frame_size - self.frame_offset) as usize;
        let take = min(remaining, out.len());
        out[..take].copy_from_slice(&self.last_image[self.frame_offset as usize..][..take]);
        self.frame_offset += take as u32;
        take
    }
}

impl Drop for USBEyetoy {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// =====================================================================================
// 2. USB HID (mouse + keyboard)
// =====================================================================================

pub const HID_MOUSE: i32 = 1;
pub const HID_TABLET: i32 = 2;
pub const HID_KEYBOARD: i32 = 3;

pub const USB_TOKEN_IN: i32 = 0x80;
pub const USB_TOKEN_OUT: i32 = 0x00;
pub const USB_RET_SUCCESS: i32 = 0;
pub const USB_RET_NAK: i32 = -1;
pub const USB_RET_STALL: i32 = -2;
pub const USB_RET_ASYNC: i32 = -3;

pub const InterfaceRequest: i32 = 0x01 << 8;
pub const ClassInterfaceRequest: i32 = 0x21 << 8;
pub const ClassInterfaceOutRequest: i32 = 0x21 << 8 | 0x20;
pub const ClassEndpointRequest: i32 = 0x25 << 8;
pub const ClassEndpointOutRequest: i32 = 0x25 << 8 | 0x20;
pub const VendorDeviceRequest: i32 = 0x40;
pub const VendorDeviceOutRequest: i32 = 0x40 | 0x20;
pub const DeviceRequest: i32 = 0x00;
pub const EndpointOutRequest: i32 = 0x02 | 0x02;
pub const USB_REQ_GET_DESCRIPTOR: i32 = 0x06;
pub const USB_REQ_CLEAR_FEATURE: i32 = 0x01;
pub const USB_SPEED_FULL: i32 = 0;
pub const GET_REPORT: i32 = 0x01;
pub const SET_REPORT: i32 = 0x09;
pub const GET_PROTOCOL: i32 = 0x03;
pub const SET_PROTOCOL: i32 = 0x0B;
pub const GET_IDLE: i32 = 0x02;
pub const SET_IDLE: i32 = 0x0A;

#[derive(Copy, Clone, Debug)]
pub struct HidState {
    pub kind: i32,
    pub protocol: u8,
    pub idle: u8,
    pub has_events: bool,
}

impl Default for HidState {
    fn default() -> Self {
        Self { kind: 0, protocol: 0, idle: 0, has_events: false }
    }
}

pub struct USBHID {
    pub state: HidState,
    pub keycode_mapping: std::collections::HashMap<u32, u32>,
    pub keyboard_state: [u8; 256],
    pub mouse_x: i32,
    pub mouse_y: i32,
    pub mouse_buttons: u32,
    pub wheel: i32,
    pub port: u32,
}

impl USBHID {
    pub fn new(port: u32) -> Self {
        Self {
            state: HidState::default(),
            keycode_mapping: std::collections::HashMap::new(),
            keyboard_state: [0; 256],
            mouse_x: 0,
            mouse_y: 0,
            mouse_buttons: 0,
            wheel: 0,
            port,
        }
    }

    pub fn init(&mut self) {
        self.state = HidState::default();
        self.keyboard_state = [0; 256];
        self.mouse_x = 0;
        self.mouse_y = 0;
        self.mouse_buttons = 0;
    }

    pub fn shutdown(&mut self) {
        self.state.has_events = false;
    }

    pub fn queue_keyboard(&mut self, keycode: u8, pressed: bool) {
        self.keyboard_state[keycode as usize] = if pressed { 1 } else { 0 };
        self.state.has_events = true;
    }

    pub fn queue_mouse_button(&mut self, button: u32, pressed: bool) {
        if pressed { self.mouse_buttons |= 1 << button; } else { self.mouse_buttons &= !(1 << button); }
        self.state.has_events = true;
    }

    pub fn queue_mouse_axis(&mut self, dx: i32, dy: i32) {
        self.mouse_x += dx;
        self.mouse_y += dy;
        self.state.has_events = true;
    }
}

// =====================================================================================
// 3. USB microphone / audio device
// =====================================================================================

pub const AUDIODIR_SOURCE: i32 = 0;
pub const AUDIODIR_SINK: i32 = 1;

pub const AUDIO_REQUEST_GET_CUR: u8 = 0x81;
pub const AUDIO_REQUEST_GET_MIN: u8 = 0x82;
pub const AUDIO_REQUEST_GET_MAX: u8 = 0x83;
pub const AUDIO_REQUEST_GET_RES: u8 = 0x84;
pub const AUDIO_REQUEST_SET_CUR: u8 = 0x01;
pub const AUDIO_REQUEST_SET_MIN: u8 = 0x02;
pub const AUDIO_REQUEST_SET_MAX: u8 = 0x03;
pub const AUDIO_REQUEST_SET_RES: u8 = 0x04;

pub const AUDIO_MUTE_CONTROL: u8 = 0x01;
pub const AUDIO_VOLUME_CONTROL: u8 = 0x02;
pub const AUDIO_AUTOMATIC_GAIN_CONTROL: u8 = 0x07;
pub const AUDIO_SAMPLING_FREQ_CONTROL: u8 = 0x01;
pub const AUDIO_BASS_BOOST_CONTROL: u8 = 0x05;

pub const MIC_SINGSTAR: i32 = 0;
pub const MIC_LOGITECH: i32 = 1;
pub const MIC_KONAMI: i32 = 2;
pub const MIC_COUNT: i32 = 3;

pub const USBAUDIO_PACKET_SIZE: usize = 200;
pub const USBAUDIO_SAMPLE_RATE: u32 = 48000;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MicMode {
    Single,
    Shared,
    Separate,
}

pub trait AudioDevice: Send {
    fn name(&self) -> &str;
    fn dir(&self) -> i32;
    fn channels(&self) -> u32;
    fn sample_rate(&self) -> u32;
    fn start(&mut self) -> bool;
    fn stop(&mut self) {}
    fn get_buffer(&mut self, buf: &mut [i16], frames: u32) -> u32 { let _ = buf; let _ = frames; 0 }
    fn set_buffer(&mut self, buf: &[i16], frames: u32) -> u32 { let _ = buf; let _ = frames; frames }
    fn get_frames(&mut self, frames: &mut u32) -> bool { *frames = 0; true }
    fn set_resampling(&mut self, rate: u32) { let _ = rate; }
    fn reset_buffers(&mut self) {}
}

pub struct USBMic {
    pub subtype: i32,
    pub dual_mic: bool,
    pub sample_rate: u32,
    pub intf: i32,
    pub mode: MicMode,
    pub mute: bool,
    pub vol: [u8; 2],
    pub srate: [u32; 2],
    pub buffer: [Vec<i16>; 2],
    pub device: Option<Box<dyn AudioDevice>>,
    pub device1: Option<Box<dyn AudioDevice>>,
}

impl USBMic {
    pub fn new(subtype: i32, sample_rate: u32) -> Self {
        Self {
            subtype,
            dual_mic: subtype == MIC_SINGSTAR,
            sample_rate,
            intf: 0,
            mode: MicMode::Single,
            mute: false,
            vol: [240, 240],
            srate: [sample_rate, sample_rate],
            buffer: [Vec::new(), Vec::new()],
            device: None,
            device1: None,
        }
    }

    pub fn init(&mut self) {}

    pub fn shutdown(&mut self) {
        if let Some(d) = self.device.as_mut() { d.stop(); }
        if let Some(d) = self.device1.as_mut() { d.stop(); }
    }

    pub fn set_resampling(&mut self, rate: u32) {
        self.sample_rate = rate;
        self.srate[0] = rate;
        self.srate[1] = rate;
        if let Some(d) = self.device.as_mut() { d.set_resampling(rate); }
        if let Some(d) = self.device1.as_mut() { d.set_resampling(rate); }
    }
}

pub struct USBHeadset {
    pub sample_rate: u32,
    pub intf: i32,
    pub mode: MicMode,
    pub in_mute: bool,
    pub in_vol: u8,
    pub in_srate: u32,
    pub out_mute: bool,
    pub out_vol: [u8; 2],
    pub out_srate: u32,
    pub in_buffer: Vec<i16>,
    pub out_buffer: Vec<i16>,
    pub audsrc: Option<Box<dyn AudioDevice>>,
    pub audsink: Option<Box<dyn AudioDevice>>,
}

impl USBHeadset {
    pub fn new() -> Self {
        Self {
            sample_rate: 48000,
            intf: 0,
            mode: MicMode::Single,
            in_mute: false,
            in_vol: 240,
            in_srate: 48000,
            out_mute: false,
            out_vol: [240, 240],
            out_srate: 48000,
            in_buffer: Vec::new(),
            out_buffer: Vec::new(),
            audsrc: None,
            audsink: None,
        }
    }

    pub fn init(&mut self) {
        if let Some(d) = self.audsrc.as_mut() { let _ = d.start(); }
        if let Some(d) = self.audsink.as_mut() { let _ = d.start(); }
    }

    pub fn shutdown(&mut self) {
        if let Some(d) = self.audsrc.as_mut() { d.stop(); }
        if let Some(d) = self.audsink.as_mut() { d.stop(); }
    }
}

impl Default for USBHeadset {
    fn default() -> Self { Self::new() }
}

// =====================================================================================
// 4. USB mass storage (MSD)
// =====================================================================================

pub const USB_MSDM_CBW: i32 = 0;
pub const USB_MSDM_DATAOUT: i32 = 1;
pub const USB_MSDM_DATAIN: i32 = 2;
pub const USB_MSDM_CSW: i32 = 3;

pub const LBA_BLOCK_SIZE: u64 = 512;
pub const COMMAND_PASSED: u32 = 0;
pub const COMMAND_FAILED: u32 = 1;

pub const TEST_UNIT_READY: u8 = 0x00;
pub const REQUEST_SENSE: u8 = 0x03;
pub const INQUIRY: u8 = 0x12;
pub const READ_CAPACITY_10: u8 = 0x25;
pub const READ_10: u8 = 0x28;
pub const WRITE_10: u8 = 0x2A;
pub const READ_12: u8 = 0xA8;
pub const WRITE_12: u8 = 0xAA;
pub const MODE_SENSE_10: u8 = 0x5A;
pub const START_STOP: u8 = 0x1B;
pub const SEND_DIAGNOSTIC: u8 = 0x1D;
pub const READ_FORMAT_CAPACITIES: u8 = 0x23;
pub const MODE_SENSE: u8 = 0x1A;
pub const ALLOW_MEDIUM_REMOVAL: u8 = 0x1E;

#[derive(Copy, Clone, Debug)]
pub struct UsbMsdCbw {
    pub sig: u32,
    pub tag: u32,
    pub data_len: u32,
    pub flags: u8,
    pub lun: u8,
    pub cmd_len: u8,
    pub cmd: [u8; 16],
}

#[derive(Copy, Clone, Debug, Default)]
pub struct UsbMsdCsw {
    pub sig: u32,
    pub tag: u32,
    pub residue: u32,
    pub status: u8,
}

pub struct USBMSD {
    pub mode: i32,
    pub data_len: u32,
    pub tag: u32,
    pub last_cmd: u8,
    pub result: u32,
    pub off: u32,
    pub buf: [u8; 4096],
    pub sense_buf: [u8; 18],
    pub csw: UsbMsdCsw,
    pub mtime: u64,
    pub file: Option<File>,
    pub file_size: u64,
    pub path: Option<PathBuf>,
}

impl USBMSD {
    pub fn new() -> Self {
        Self {
            mode: USB_MSDM_CBW,
            data_len: 0,
            tag: 0,
            last_cmd: 0,
            result: COMMAND_PASSED,
            off: 0,
            buf: [0; 4096],
            sense_buf: [0; 18],
            csw: UsbMsdCsw::default(),
            mtime: 0,
            file: None,
            file_size: 0,
            path: None,
        }
    }

    pub fn init(&mut self, path: &std::path::Path) -> std::io::Result<()> {
        let mut file = File::options().read(true).write(true).open(path)?;
        let md = file.metadata()?;
        self.file_size = md.len();
        self.mtime = md.modified()
            .ok()
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.path = Some(path.to_path_buf());
        self.file = Some(file);
        Ok(())
    }

    pub fn shutdown(&mut self) {
        self.file = None;
    }

    fn set_sense(&mut self, key: u8, asc: u8, ascq: u8) {
        self.sense_buf = [0; 18];
        self.sense_buf[0]  = 0x70 | 0x80;
        self.sense_buf[2]  = key & 0x0F;
        self.sense_buf[7]  = if asc != 0 { 0x0A } else { 0x00 };
        self.sense_buf[12] = asc;
        self.sense_buf[13] = ascq;
    }

    fn bswap32(x: u32) -> u32 {
        x.swap_bytes()
    }
    fn bswap16(x: u16) -> u16 {
        x.swap_bytes()
    }

    pub fn send_command(&mut self, cbw: &UsbMsdCbw) {
        self.last_cmd = cbw.cmd[0];
        self.result = COMMAND_PASSED;
        self.off = 0;
        if cbw.cmd[0] != REQUEST_SENSE {
            self.set_sense(0, 0, 0);
        }
        match cbw.cmd[0] {
            TEST_UNIT_READY => {}
            REQUEST_SENSE => {
                let n = min(cbw.cmd[4] as usize, self.sense_buf.len());
                self.buf[..n].copy_from_slice(&self.sense_buf[..n]);
            }
            INQUIRY => {
                self.buf = [0; 4096];
                self.buf[1] = 1 << 7;
                self.buf[2] = 0x02;
                self.buf[3] = 0x02;
                let vendor = b"IOMEGA  ";
                let product = b"ZIP 100         ";
                let revision = b"E.08";
                self.buf[8..16].copy_from_slice(vendor);
                self.buf[16..32].copy_from_slice(product);
                self.buf[32..36].copy_from_slice(revision);
            }
            READ_CAPACITY_10 => {
                self.buf = [0; 4096];
                if self.file_size == 0 {
                    self.result = COMMAND_FAILED;
                    self.set_sense(2, 0xFF, 0xFF);
                    return;
                }
                let lbas = self.file_size / LBA_BLOCK_SIZE;
                let last_lba = if lbas > 0xFFFF_FFFF { 0xFFFF_FFFF } else { lbas as u32 };
                let blk_len = LBA_BLOCK_SIZE as u32;
                self.buf[0..4].copy_from_slice(&Self::bswap32(last_lba).to_be_bytes());
                self.buf[4..8].copy_from_slice(&Self::bswap32(blk_len).to_be_bytes());
            }
            READ_10 | READ_12 => {
                let lba = u32::from_be_bytes([cbw.cmd[2], cbw.cmd[3], cbw.cmd[4], cbw.cmd[5]]);
                let xfer = if cbw.cmd[0] == READ_10 {
                    u16::from_be_bytes([cbw.cmd[7], cbw.cmd[8]]) as u32
                } else {
                    u32::from_be_bytes([cbw.cmd[6], cbw.cmd[7], cbw.cmd[8], cbw.cmd[9]])
                };
                self.data_len = xfer * LBA_BLOCK_SIZE as u32;
                if let Some(f) = self.file.as_mut() {
                    let _ = f.seek(SeekFrom::Start(lba as u64 * LBA_BLOCK_SIZE));
                }
            }
            WRITE_10 | WRITE_12 => {
                let lba = u32::from_be_bytes([cbw.cmd[2], cbw.cmd[3], cbw.cmd[4], cbw.cmd[5]]);
                let xfer = if cbw.cmd[0] == WRITE_10 {
                    u16::from_be_bytes([cbw.cmd[7], cbw.cmd[8]]) as u32
                } else {
                    u32::from_be_bytes([cbw.cmd[6], cbw.cmd[7], cbw.cmd[8], cbw.cmd[9]])
                };
                self.data_len = xfer * LBA_BLOCK_SIZE as u32;
                if let Some(f) = self.file.as_mut() {
                    let _ = f.seek(SeekFrom::Start(lba as u64 * LBA_BLOCK_SIZE));
                }
            }
            _ => {
                self.result = COMMAND_FAILED;
                self.set_sense(0x05, 0x20, 0x00);
                self.mode = USB_MSDM_CSW;
            }
        }
    }

    pub fn handle_data_out(&mut self, data: &[u8]) {
        let take = min(min(data.len(), self.buf.len()), self.data_len as usize);
        self.buf[..take].copy_from_slice(&data[..take]);
        self.off += take as u32;
        self.data_len -= take as u32;
        if let Some(f) = self.file.as_mut() {
            let _ = f.write_all(&self.buf[..take]);
        }
        if self.data_len == 0 {
            self.mode = USB_MSDM_CSW;
        }
    }

    pub fn handle_data_in(&mut self, data: &mut [u8]) -> usize {
        if self.mode != USB_MSDM_DATAIN {
            return 0;
        }
        let take = min(min(data.len(), self.buf.len()), self.data_len as usize);
        if let Some(f) = self.file.as_mut() {
            let n = f.read(&mut self.buf[..take]).unwrap_or(0);
            data[..n].copy_from_slice(&self.buf[..n]);
            self.off += n as u32;
            self.data_len -= n as u32;
            if self.data_len == 0 {
                self.mode = USB_MSDM_CSW;
            }
            return n;
        }
        0
    }

    pub fn make_csw(&mut self) -> UsbMsdCsw {
        UsbMsdCsw {
            sig: 0x5342_5355,
            tag: self.tag,
            residue: self.data_len,
            status: if self.result == COMMAND_PASSED { 0 } else { 1 },
        }
    }
}

impl Default for USBMSD {
    fn default() -> Self { Self::new() }
}

// =====================================================================================
// 5. USB printer
// =====================================================================================

pub const GET_DEVICE_ID: i32 = 0;
pub const GET_PORT_STATUS: i32 = 1;
pub const GET_PORT_STATUS_PAPER_NOT_EMPTY: u8 = 0x10;
pub const GET_PORT_STATUS_SELECTED: u8 = 0x18;
pub const GET_PORT_STATUS_NO_ERROR: u8 = 0x00;

#[derive(Copy, Clone, Debug)]
pub struct BmpHeader {
    pub magic: u16,
    pub filesize: u32,
    pub data_offset: u32,
    pub core_header_size: u32,
    pub width: u32,
    pub height: u32,
    pub planes: u16,
    pub bpp: u16,
}

pub struct USBPrinter {
    pub selected_printer: u32,
    pub cmd_state: i32,
    pub last_command: [u8; 65],
    pub last_command_size: i32,
    pub print_file: Option<File>,
    pub print_filename: Option<PathBuf>,
    pub width: i32,
    pub height: i32,
    pub stride: i64,
    pub data_size: i32,
    pub data_pos: i64,
}

impl USBPrinter {
    pub fn new() -> Self {
        Self {
            selected_printer: 0,
            cmd_state: 0,
            last_command: [0; 65],
            last_command_size: 0,
            print_file: None,
            print_filename: None,
            width: 0,
            height: 0,
            stride: 0,
            data_size: 0,
            data_pos: 0,
        }
    }

    pub fn init(&mut self) {}

    pub fn shutdown(&mut self) {
        if self.print_file.take().is_some() {
            // The original code calls `sony_cancel_file` here.
        }
    }

    pub fn sony_open_file(&mut self, folder: &std::path::Path) {
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let filename = folder.join(format!("print_{}.bmp", stamp));
        match File::create(&filename) {
            Ok(mut f) => {
                let header = BmpHeader {
                    magic: 0x4D42,
                    filesize: (14 + 3 * (self.width as u32) * (self.height as u32)),
                    data_offset: 14,
                    core_header_size: 0x0C,
                    width: self.width as u32,
                    height: self.height as u32,
                    planes: 1,
                    bpp: 24,
                };
                let _ = f.write_all(&header.magic.to_le_bytes());
                let _ = f.write_all(&header.filesize.to_le_bytes());
                let _ = f.write_all(&header.data_offset.to_le_bytes());
                let _ = f.write_all(&header.core_header_size.to_le_bytes());
                let _ = f.write_all(&header.width.to_le_bytes());
                let _ = f.write_all(&header.height.to_le_bytes());
                let _ = f.write_all(&header.planes.to_le_bytes());
                let _ = f.write_all(&header.bpp.to_le_bytes());
                self.stride = 3 * (self.width as i64) + 3 - ((3 * (self.width as i64) + 3) & 3);
                self.data_pos = 0;
                let _ = f.seek(SeekFrom::Start((14 + self.stride * (self.height as i64) - 1) as u64));
                let _ = f.write_all(&[0u8]);
                self.print_file = Some(f);
                self.print_filename = Some(filename);
            }
            Err(_) => {
                self.print_file = None;
                self.print_filename = None;
            }
        }
    }

    pub fn sony_write_data(&mut self, data: &[u8]) {
        for b in data {
            let line = (self.data_pos / 3) / (self.width.max(1) as i64);
            let col  = (self.data_pos / 3) % (self.width.max(1) as i64);
            let pos_out = self.stride * ((self.height as i64) - 1 - line) + 3 * col;
            if pos_out < 0 {
                break;
            }
            if let Some(f) = self.print_file.as_mut() {
                let _ = f.seek(SeekFrom::Start((14 + pos_out + 2 - (self.data_pos % 3)) as u64));
                let _ = f.write_all(&[*b]);
            }
            self.data_pos += 1;
        }
    }

    pub fn sony_close_file(&mut self) {
        self.print_file = None;
        self.print_filename = None;
    }
}

impl Default for USBPrinter {
    fn default() -> Self { Self::new() }
}

// =====================================================================================
// 6. USB lightgun (GunCon2)
// =====================================================================================

pub const GUNCON2_FLAG_PROGRESSIVE: u16 = 0x0100;
pub const GUNCON2_CALIBRATION_DELAY: u16 = 12;
pub const GUNCON2_CALIBRATION_REPORT_DELAY: u16 = 5;

#[derive(Copy, Clone, Debug, Default)]
#[repr(C)]
pub struct GunCon2Out {
    pub buttons: u16,
    pub pos_x: i16,
    pub pos_y: i16,
}

pub struct GunCon2 {
    pub port: u32,
    pub has_relative_binds: bool,
    pub custom_config: bool,
    pub screen_width: u32,
    pub screen_height: u32,
    pub center_x: f32,
    pub center_y: f32,
    pub scale_x: f32,
    pub scale_y: f32,
    pub button_state: u32,
    pub cursor_path: String,
    pub cursor_scale: f32,
    pub cursor_color: u32,
    pub relative_pos: [f32; 4],
    pub param_x: u16,
    pub param_y: u16,
    pub param_mode: u16,
    pub calibration_timer: u16,
    pub calibration_pos_x: i16,
    pub calibration_pos_y: i16,
    pub auto_config_done: bool,
}

impl GunCon2 {
    pub fn new(port: u32) -> Self {
        Self {
            port,
            has_relative_binds: false,
            custom_config: false,
            screen_width: 640,
            screen_height: 240,
            center_x: 320.0,
            center_y: 120.0,
            scale_x: 1.0,
            scale_y: 1.0,
            button_state: 0,
            cursor_path: String::new(),
            cursor_scale: 1.0,
            cursor_color: 0xFFFF_FFFF,
            relative_pos: [0.0; 4],
            param_x: 0,
            param_y: 0,
            param_mode: 0,
            calibration_timer: 0,
            calibration_pos_x: 0,
            calibration_pos_y: 0,
            auto_config_done: false,
        }
    }

    pub fn init(&mut self) {}
    pub fn shutdown(&mut self) {}

    pub fn calculate_position(&self, pointer_x: f32, pointer_y: f32) -> (i16, i16) {
        if pointer_x < 0.0 || pointer_y < 0.0 {
            return (0, 0);
        }
        let fx = pointer_x * self.screen_width as f32 - (self.screen_width as f32 / 2.0);
        let fy = pointer_y * self.screen_height as f32 - (self.screen_height as f32 / 2.0);
        let fx = fx * self.scale_x;
        let fy = fy * self.scale_y;
        let mut x = (fx + self.center_x).round() as i32;
        let mut y = (fy + self.center_y).round() as i32;
        if self.param_mode & GUNCON2_FLAG_PROGRESSIVE != 0 {
            x -= self.param_x as i32 / 2;
            y -= self.param_y as i32 / 2;
        } else {
            x -= self.param_x as i32;
            y -= self.param_y as i32;
        }
        (x.max(1) as i16, y.max(1) as i16)
    }

    pub fn fill_report(&mut self, pos_x: i16, pos_y: i16) -> GunCon2Out {
        let mut out = GunCon2Out {
            buttons: !(self.button_state as u16) | (self.param_mode & GUNCON2_FLAG_PROGRESSIVE),
            pos_x,
            pos_y,
        };
        if self.calibration_timer > 0 {
            out.buttons &= !(1u16 << 13);
            out.pos_x = self.calibration_pos_x;
            out.pos_y = self.calibration_pos_y;
            self.calibration_timer -= 1;
            if self.calibration_timer < GUNCON2_CALIBRATION_REPORT_DELAY {
                out.pos_x = 0;
                out.pos_y = 0;
            }
        }
        out
    }
}

// =====================================================================================
// 7. USB pad (Driving Force, Rock Band drum kit, Keyboardmania, etc.)
// =====================================================================================

pub const WT_GENERIC: i32 = 0;
pub const WT_DRIVING_FORCE_PRO: i32 = 1;
pub const WT_DRIVING_FORCE_PRO_1102: i32 = 2;
pub const WT_GT_FORCE: i32 = 3;
pub const WT_ROCKBAND1_DRUMKIT: i32 = 4;
pub const WT_SEGA_SEAMIC: i32 = 5;
pub const WT_KEYBOARDMANIA_CONTROLLER: i32 = 6;
pub const WT_COUNT: i32 = 7;

pub const CID_STEERING_L: u32 = 0;
pub const CID_STEERING_R: u32 = 1;
pub const CID_THROTTLE: u32 = 2;
pub const CID_BRAKE: u32 = 3;
pub const CID_DPAD_UP: u32 = 4;
pub const CID_DPAD_DOWN: u32 = 5;
pub const CID_DPAD_LEFT: u32 = 6;
pub const CID_DPAD_RIGHT: u32 = 7;
pub const CID_BUTTON0: u32 = 16;
pub const CID_BUTTON1: u32 = 17;
pub const CID_BUTTON2: u32 = 18;
pub const CID_BUTTON3: u32 = 19;
pub const CID_BUTTON4: u32 = 20;
pub const CID_BUTTON5: u32 = 21;
pub const CID_BUTTON6: u32 = 22;
pub const CID_BUTTON7: u32 = 23;
pub const CID_BUTTON8: u32 = 24;
pub const CID_BUTTON9: u32 = 25;
pub const CID_BUTTON10: u32 = 26;
pub const CID_BUTTON11: u32 = 27;
pub const CID_BUTTON12: u32 = 28;
pub const CID_BUTTON13: u32 = 29;
pub const CID_BUTTON14: u32 = 30;
pub const CID_BUTTON15: u32 = 31;
pub const CID_BUTTON16: u32 = 32;
pub const CID_BUTTON17: u32 = 33;
pub const CID_BUTTON18: u32 = 34;
pub const CID_BUTTON19: u32 = 35;
pub const CID_BUTTON20: u32 = 36;
pub const CID_BUTTON21: u32 = 37;
pub const CID_BUTTON22: u32 = 38;
pub const CID_BUTTON23: u32 = 39;
pub const CID_BUTTON24: u32 = 40;
pub const CID_BUTTON25: u32 = 41;
pub const CID_BUTTON26: u32 = 42;
pub const CID_BUTTON27: u32 = 43;
pub const CID_BUTTON28: u32 = 44;
pub const CID_BUTTON29: u32 = 45;
pub const CID_BUTTON30: u32 = 46;
pub const CID_BUTTON31: u32 = 47;

#[derive(Copy, Clone, Debug, Default)]
pub struct PadData {
    pub steering: u16,
    pub steering_left: i16,
    pub steering_right: i16,
    pub throttle: u32,
    pub brake: u32,
    pub buttons: u32,
    pub hatswitch: u32,
    pub hat_up: u8,
    pub hat_down: u8,
    pub hat_left: u8,
    pub hat_right: u8,
    pub last_steering: u16,
}

pub struct UsbPad {
    pub port: u32,
    pub wt: i32,
    pub steering_range: u16,
    pub steering_step: u16,
    pub steering_deadzone: i16,
    pub steering_curve_exponent: i32,
    pub data: PadData,
    pub ff_state: u32,
}

impl UsbPad {
    pub fn new(port: u32, wt: i32) -> Self {
        let steering_range = match wt {
            WT_DRIVING_FORCE_PRO | WT_DRIVING_FORCE_PRO_1102 => 0x3FFF >> 1,
            WT_SEGA_SEAMIC => 0xFF >> 1,
            _ => 0x3FF >> 1,
        };
        let mut data = PadData::default();
        data.steering = steering_range;
        data.last_steering = steering_range;
        data.throttle = 255;
        data.brake = 255;
        Self {
            port,
            wt,
            steering_range,
            steering_step: u16::MAX,
            steering_deadzone: 0,
            steering_curve_exponent: 0,
            data,
            ff_state: 0,
        }
    }

    pub fn reset(&mut self) {
        self.data.steering = self.steering_range;
        self.ff_state = 0;
    }

    pub fn update_steering(&mut self) {
        let value: u16 = if self.data.steering_left > 0 {
            (self.steering_range as i32 - self.data.steering_left as i32).max(0) as u16
        } else {
            (self.steering_range as i32 + self.data.steering_right as i32)
                .min((self.steering_range as i32) * 2) as u16
        };
        if value < self.data.steering {
            self.data.steering -= (self.data.steering - value).min(self.steering_step);
        } else if value > self.data.steering {
            self.data.steering += (value - self.data.steering).min(self.steering_step);
        }
    }

    pub fn update_hat_switch(&mut self) {
        self.data.hatswitch = match (
            self.data.hat_up != 0,
            self.data.hat_right != 0,
            self.data.hat_down != 0,
            self.data.hat_left != 0,
        ) {
            (true, true, _, _) => 1,
            (_, true, true, _) => 3,
            (_, _, true, true) => 5,
            (true, _, _, true) => 7,
            (true, _, _, _) => 0,
            (_, true, _, _) => 2,
            (_, _, true, _) => 4,
            (_, _, _, true) => 6,
            _ => 8,
        };
    }

    pub fn token_in(&mut self, buf: &mut [u8]) -> usize {
        match self.wt {
            WT_GENERIC => {
                self.update_steering();
                self.update_hat_switch();
                if buf.len() < 8 { return 0; }
                let lo = (self.data.steering & 0x3FF) as u32
                       | ((self.data.buttons & 0xFFF) << 10)
                       | (0xFFu32 << 24);
                let hi = (self.data.hatswitch & 0xF) as u32
                       | ((self.data.throttle & 0xFF) << 8)
                       | ((self.data.brake & 0xFF) << 16);
                buf[0..4].copy_from_slice(&lo.to_le_bytes());
                buf[4..8].copy_from_slice(&hi.to_le_bytes());
                8
            }
            WT_GT_FORCE => {
                self.update_steering();
                self.update_hat_switch();
                if buf.len() < 8 { return 0; }
                let lo = (self.data.steering & 0x3FF) as u32
                       | ((self.data.buttons & 0xFFF) << 10)
                       | (1u32 << 16)
                       | (0xFFu32 << 24);
                let hi = (self.data.throttle & 0xFF) as u32
                       | ((self.data.brake & 0xFF) << 8);
                buf[0..4].copy_from_slice(&lo.to_le_bytes());
                buf[4..8].copy_from_slice(&hi.to_le_bytes());
                8
            }
            WT_DRIVING_FORCE_PRO | WT_DRIVING_FORCE_PRO_1102 => {
                self.update_steering();
                self.update_hat_switch();
                if buf.len() < 8 { return 0; }
                let lo = (self.data.steering & 0x3FFF) as u32
                       | ((self.data.buttons & 0x3FFF) << 14)
                       | ((self.data.hatswitch & 0xF) << 28);
                let hi = ((self.data.throttle & 0xFF) << 8)
                       | ((self.data.brake & 0xFF) << 16)
                       | (0x11u32 << 24);
                buf[0..4].copy_from_slice(&lo.to_le_bytes());
                buf[4..8].copy_from_slice(&hi.to_le_bytes());
                8
            }
            WT_ROCKBAND1_DRUMKIT => {
                self.update_hat_switch();
                if buf.len() < 4 { return 0; }
                let lo = (self.data.buttons & 0xFFF) as u32
                       | ((self.data.hatswitch & 0xF) << 16);
                buf[0..4].copy_from_slice(&lo.to_le_bytes());
                4
            }
            WT_SEGA_SEAMIC => {
                self.update_steering();
                self.update_hat_switch();
                if buf.len() < 5 { return 0; }
                buf[0] = self.data.steering as u8;
                buf[1] = self.data.throttle as u8;
                buf[2] = self.data.brake as u8;
                buf[3] = (self.data.hatswitch & 0x0F) as u8
                       | (((self.data.buttons & 0x0F) << 4) as u8);
                buf[4] = ((self.data.buttons >> 4) & 0x3F) as u8;
                5
            }
            WT_KEYBOARDMANIA_CONTROLLER => {
                if buf.len() < 5 { return 0; }
                buf[0] = 0x3F;
                buf[1] = (self.data.buttons & 0xFF) as u8;
                buf[2] = ((self.data.buttons >> 8) & 0xFF) as u8;
                buf[3] = ((self.data.buttons >> 16) & 0xFF) as u8;
                buf[4] = ((self.data.buttons >> 24) & 0xFF) as u8;
                5
            }
            _ => 0,
        }
    }

    pub fn token_out(&mut self, data: &[u8]) {
        if data.len() < 4 { return; }
        // Force feedback data parsing – simply stash the raw value for now.
        self.ff_state = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    }

    pub fn get_bind_value(&self, bind_index: u32) -> f32 {
        match bind_index {
            CID_STEERING_L => self.data.steering_left as f32 / self.steering_range as f32,
            CID_STEERING_R => self.data.steering_right as f32 / self.steering_range as f32,
            CID_THROTTLE => 1.0 - (self.data.throttle as f32 / 255.0),
            CID_BRAKE => 1.0 - (self.data.brake as f32 / 255.0),
            CID_DPAD_UP => self.data.hat_up as f32,
            CID_DPAD_DOWN => self.data.hat_down as f32,
            CID_DPAD_LEFT => self.data.hat_left as f32,
            CID_DPAD_RIGHT => self.data.hat_right as f32,
            b if b >= CID_BUTTON0 && b < CID_BUTTON0 + 32 => {
                let mask = 1u32 << (b - CID_BUTTON0);
                if self.data.buttons & mask != 0 { 1.0 } else { 0.0 }
            }
            _ => 0.0,
        }
    }

    pub fn set_bind_value(&mut self, bind_index: u32, value: f32) {
        match bind_index {
            CID_STEERING_L => {
                self.data.steering_left = (value * self.steering_range as f32).round() as i16;
                self.update_steering();
            }
            CID_STEERING_R => {
                self.data.steering_right = (value * self.steering_range as f32).round() as i16;
                self.update_steering();
            }
            CID_THROTTLE => {
                self.data.throttle = (255 - (value * 255.0).round().clamp(0.0, 255.0) as i32) as u32;
            }
            CID_BRAKE => {
                self.data.brake = (255 - (value * 255.0).round().clamp(0.0, 255.0) as i32) as u32;
            }
            CID_DPAD_UP => { self.data.hat_up = (value * 255.0).round().clamp(0.0, 255.0) as u8; self.update_hat_switch(); }
            CID_DPAD_DOWN => { self.data.hat_down = (value * 255.0).round().clamp(0.0, 255.0) as u8; self.update_hat_switch(); }
            CID_DPAD_LEFT => { self.data.hat_left = (value * 255.0).round().clamp(0.0, 255.0) as u8; self.update_hat_switch(); }
            CID_DPAD_RIGHT => { self.data.hat_right = (value * 255.0).round().clamp(0.0, 255.0) as u8; self.update_hat_switch(); }
            b if b >= CID_BUTTON0 && b < CID_BUTTON0 + 32 => {
                let mask = 1u32 << (b - CID_BUTTON0);
                if value >= 0.5 { self.data.buttons |= mask; } else { self.data.buttons &= !mask; }
            }
            _ => {}
        }
    }
}

pub fn usb_pad_init() {}
pub fn usb_pad_poll() {}

// =====================================================================================
// Module-wide globals modelled on the C `static` variables.
// =====================================================================================

static_mut!(EYETOY_THREAD: u64 = 0);
static_mut!(EYETOY_RUNNING: u8 = 0);
static_mut!(EYETOY_FD: i32 = -1);
static_mut!(EYETOY_PIXELFORMAT: u32 = V4L2_PIX_FMT_YUYV);
static_mut!(EYETOY_FRAME_WIDTH: i32 = 0);
static_mut!(EYETOY_FRAME_HEIGHT: i32 = 0);
static_mut!(EYETOY_FRAME_FORMAT: FrameFormat = FrameFormat::Mpeg);
static_mut!(EYETOY_MIRRORING: bool = true);
static_mutex!(EYETOY_MPEG_MUTEX: Buffer = Buffer::empty());

static_mut!(HID_KEYBOARD_BUF: [u8; 16] = [0; 16]);
static_mut!(HID_MOUSE_X: i32 = 0);
static_mut!(HID_MOUSE_Y: i32 = 0);
static_mut!(HID_MOUSE_BTNS: u32 = 0);

static_mut!(MIC_FILE: Option<std::fs::File> = None);
static_mut!(HEADSET_FILE: Option<std::fs::File> = None);
static_mut!(MSD_FILE: Option<std::fs::File> = None);
static_mut!(PRINTER_FILE: Option<std::fs::File> = None);
static_mut!(LIGHTGUN_BUTTON_STATE: u32 = 0);
static_mut!(PAD_STEERING: u16 = 0);
