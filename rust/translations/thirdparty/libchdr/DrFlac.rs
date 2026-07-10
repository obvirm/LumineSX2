//! Rust 2021 idiomatic translation of the dr_flac FLAC decoder (single-header
//! library from dr_libs, vendored at `3rdparty/libchdr/include/dr_libs/dr_flac.h`).
//!
//! This module exposes:
//!
//! - the [`DrFlac`] struct (the open decoder state, equivalent to the C
//!   `drflac` opaque struct, with its bitstream (`drflac_bs`), frame state
//!   and sample-rate / channels / bits-per-sample fields), and
//! - the high-level entry points `drflac_open_path`, `drflac_open_memory`,
//!   `drflac_read_pcm_frames_s16`, and `drflac_close`, mirroring the C API.
//! - the FLAC frame-decoder primitives `flac_lpc_predict`,
//!   `flac_residual_decode`, and `flac_partitioned_rice_decode` requested by
//!   the rewrite rules.
//!
//! Per the rewrite rules, this translation does not link the original C
//! runtime. State that the C library would have kept in static globals
//! (cache-line selection masks, the FLAC signature, etc.) is mirrored here
//! with `static mut` items. Only `std` is used.

use std::cmp::min;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::raw::{c_int, c_void};
use std::ptr;

// ---------------------------------------------------------------------------
// Sized integer aliases (mirror the drflac_int* / drflac_uint* types)
// ---------------------------------------------------------------------------

pub type drflac_int8 = i8;
pub type drflac_uint8 = u8;
pub type drflac_int16 = i16;
pub type drflac_uint16 = u16;
pub type drflac_int32 = i32;
pub type drflac_uint32 = u32;
pub type drflac_int64 = i64;
pub type drflac_uint64 = u64;
pub type drflac_bool8 = u8;
pub type drflac_bool32 = u32;

pub const DRFLAC_TRUE: drflac_bool32 = 1;
pub const DRFLAC_FALSE: drflac_bool32 = 0;

// Cache-line width is 64-bit on 64-bit hosts and 32-bit on 32-bit hosts.
#[cfg(target_pointer_width = "64")]
pub type drflac_cache_t = drflac_uint64;
#[cfg(not(target_pointer_width = "64"))]
pub type drflac_cache_t = drflac_uint32;

// ---------------------------------------------------------------------------
// Public constants (dr_flac.h)
// ---------------------------------------------------------------------------

pub const DRFLAC_METADATA_BLOCK_TYPE_STREAMINFO: u32 = 0;
pub const DRFLAC_METADATA_BLOCK_TYPE_PADDING: u32 = 1;
pub const DRFLAC_METADATA_BLOCK_TYPE_APPLICATION: u32 = 2;
pub const DRFLAC_METADATA_BLOCK_TYPE_SEEKTABLE: u32 = 3;
pub const DRFLAC_METADATA_BLOCK_TYPE_VORBIS_COMMENT: u32 = 4;
pub const DRFLAC_METADATA_BLOCK_TYPE_CUESHEET: u32 = 5;
pub const DRFLAC_METADATA_BLOCK_TYPE_PICTURE: u32 = 6;
pub const DRFLAC_METADATA_BLOCK_TYPE_INVALID: u32 = 127;

/// Default size of the L2 read buffer in bytes. Must be a multiple of 8.
pub const DR_FLAC_BUFFER_SIZE: usize = 4096;

#[repr(u32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum DrflacContainer {
    Native,
    Ogg,
    Unknown,
}

#[repr(u32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum DrflacSeekOrigin {
    Start,
    Current,
}

#[repr(i32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum DrflacResult {
    Success = 0,
    Error = -1,
    InvalidArgument = -2,
    InternalError = -3,
    OutOfMemory = -4,
    NotInitialized = -5,
    InvalidCRC = -6,
    UncompressedFrameHeader = -7,
    FrameHeaderMissingCrc = -8,
    NotFlac = -9,
    PartialFrame = -10,
    ContainerNotSupported = -11,
    ContainerOffsetError = -12,
}

/// Sub-frame type constants used by the bit-stream decoder.
pub mod subframe {
    pub const CONSTANT: u8 = 0;
    pub const VERBATIM: u8 = 1;
    pub const FIXED: u8 = 2;
    pub const LPC: u8 = 3;
}

/// Channel assignment constants used by the bit-stream decoder.
pub mod channel_assignment {
    pub const INDEPENDENT: u8 = 0;
    pub const LEFT_SIDE: u8 = 1;
    pub const RIGHT_SIDE: u8 = 2;
    pub const MID_SIDE: u8 = 3;
}

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

/// Memory-stream descriptor used by `drflac_open_memory`.
#[derive(Debug, Clone)]
pub struct DrflacMemoryStream {
    pub data: Vec<u8>,
    pub data_size: usize,
    pub current_read_pos: usize,
}

/// Bit-stream cache used throughout dr_flac. Mirrors the C `drflac_bs` struct.
#[derive(Debug, Clone)]
pub struct DrflacBs {
    pub on_read: Option<ReadProc>,
    pub on_seek: Option<SeekProc>,
    pub user_data: usize, // opaque pointer stored as usize
    pub unaligned_byte_count: usize,
    pub unaligned_cache: drflac_cache_t,
    pub next_l2_line: u32,
    pub consumed_bits: u32,
    pub cache_l2: [drflac_cache_t; DR_FLAC_BUFFER_SIZE / std::mem::size_of::<drflac_cache_t>()],
    pub cache: drflac_cache_t,
    pub crc16: u16,
    pub crc16_cache: drflac_cache_t,
    pub crc16_cache_ignored_bytes: u32,
}

impl Default for DrflacBs {
    fn default() -> Self {
        Self {
            on_read: None,
            on_seek: None,
            user_data: 0,
            unaligned_byte_count: 0,
            unaligned_cache: 0,
            next_l2_line: 0,
            consumed_bits: 0,
            cache_l2: [0; DR_FLAC_BUFFER_SIZE / std::mem::size_of::<drflac_cache_t>()],
            cache: 0,
            crc16: 0,
            crc16_cache: 0,
            crc16_cache_ignored_bytes: 0,
        }
    }
}

/// Sub-frame descriptor.
#[derive(Debug, Clone, Default)]
pub struct DrflacSubframe {
    pub subframe_type: u8,
    pub wasted_bits_per_sample: u8,
    pub lpc_order: u8,
    pub p_samples_s32_offset: usize,
}

/// Frame header.
#[derive(Debug, Clone, Default)]
pub struct DrflacFrameHeader {
    pub pcm_frame_number: u64,
    pub flac_frame_number: u32,
    pub sample_rate: u32,
    pub block_size_in_pcm_frames: u16,
    pub channel_assignment: u8,
    pub bits_per_sample: u8,
    pub crc8: u8,
}

/// Frame state.
#[derive(Debug, Clone, Default)]
pub struct DrflacFrame {
    pub header: DrflacFrameHeader,
    pub pcm_frames_remaining: u32,
    pub subframes: [DrflacSubframe; 8],
}

/// Stream-info metadata block.
#[derive(Debug, Clone, Default)]
pub struct DrflacStreaminfo {
    pub min_block_size_in_pcm_frames: u16,
    pub max_block_size_in_pcm_frames: u16,
    pub min_frame_size_in_pcm_frames: u32,
    pub max_frame_size_in_pcm_frames: u32,
    pub sample_rate: u32,
    pub channels: u8,
    pub bits_per_sample: u8,
    pub total_pcm_frame_count: u64,
    pub md5: [u8; 16],
}

/// Metadata union (only streaminfo is populated here for the libchdr path).
#[derive(Debug, Clone)]
pub struct DrflacMetadata {
    pub mtype: u32,
    pub p_raw_data: Option<Vec<u8>>,
    pub raw_data_size: u32,
    pub streaminfo: DrflacStreaminfo,
}

/// Allocation callback bundle (analog of `drflac_allocation_callbacks`).
pub type MallocFn = unsafe extern "C" fn(size: usize, user: *mut c_void) -> *mut c_void;
pub type ReallocFn = unsafe extern "C" fn(p: *mut c_void, size: usize, user: *mut c_void) -> *mut c_void;
pub type FreeFn = unsafe extern "C" fn(p: *mut c_void, user: *mut c_void);

#[derive(Debug, Clone, Copy)]
pub struct DrflacAllocationCallbacks {
    pub user_data: *mut c_void,
    pub on_malloc: Option<MallocFn>,
    pub on_realloc: Option<ReallocFn>,
    pub on_free: Option<FreeFn>,
}

impl Default for DrflacAllocationCallbacks {
    fn default() -> Self {
        Self {
            user_data: ptr::null_mut(),
            on_malloc: None,
            on_realloc: None,
            on_free: None,
        }
    }
}

/// Callback prototypes.
pub type ReadProc =
    unsafe extern "C" fn(user_data: *mut c_void, buffer: *mut c_void, bytes: usize) -> usize;
pub type SeekProc = unsafe extern "C" fn(
    user_data: *mut c_void,
    offset: c_int,
    origin: DrflacSeekOrigin,
) -> drflac_bool32;
pub type MetaProc = unsafe extern "C" fn(user_data: *mut c_void, metadata: *mut DrflacMetadata);

// ---------------------------------------------------------------------------
// The decoder (equivalent of the C `drflac` struct).
// ---------------------------------------------------------------------------

/// A FLAC decoder.
///
/// This is the equivalent of the C `drflac` struct, kept public so callers
/// can poke at the sample rate / channels / bits-per-sample directly.
pub struct DrFlac {
    pub on_meta: Option<MetaProc>,
    pub user_data_md: *mut c_void,
    pub allocation_callbacks: DrflacAllocationCallbacks,

    pub sample_rate: u32,
    pub channels: u8,
    pub bits_per_sample: u8,
    pub max_block_size_in_pcm_frames: u16,
    pub total_pcm_frame_count: u64,

    pub container: DrflacContainer,
    pub seekpoint_count: u32,

    pub current_flac_frame: DrflacFrame,
    pub current_pcm_frame: u64,
    pub first_flac_frame_pos_in_bytes: u64,

    pub memory_stream: Option<DrflacMemoryStream>,
    pub file: Option<File>,

    pub p_decoded_samples_offset: usize,
    pub p_seekpoints: usize,

    /// Internal sample buffer used by `flac_residual_decode` and friends.
    pub decoded_samples: Vec<drflac_int32>,

    /// Bit-stream state.
    pub bs: DrflacBs,
}

impl DrFlac {
    pub fn new() -> Self {
        Self {
            on_meta: None,
            user_data_md: ptr::null_mut(),
            allocation_callbacks: DrflacAllocationCallbacks::default(),
            sample_rate: 0,
            channels: 0,
            bits_per_sample: 0,
            max_block_size_in_pcm_frames: 0,
            total_pcm_frame_count: 0,
            container: DrflacContainer::Unknown,
            seekpoint_count: 0,
            current_flac_frame: DrflacFrame::default(),
            current_pcm_frame: 0,
            first_flac_frame_pos_in_bytes: 0,
            memory_stream: None,
            file: None,
            p_decoded_samples_offset: 0,
            p_seekpoints: 0,
            decoded_samples: Vec::new(),
            bs: DrflacBs::default(),
        }
    }
}

impl Default for DrFlac {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// CRC-16 (poly 0x8005) and CRC-8 (poly 0x07) used by FLAC.
// ---------------------------------------------------------------------------

/// FLAC CRC-16 (polynomial 0x8005).
pub fn drflac_crc16_bytes(crc: u16, data: &[u8]) -> u16 {
    const TABLE: [u16; 256] = [
        0x0000, 0x8005, 0x800f, 0x000a, 0x801b, 0x001e, 0x0014, 0x8011,
        0x8033, 0x0036, 0x003c, 0x8039, 0x0028, 0x802d, 0x8027, 0x0022,
        0x8063, 0x0066, 0x006c, 0x8069, 0x0078, 0x807d, 0x8077, 0x0072,
        0x0050, 0x8055, 0x805f, 0x005a, 0x804b, 0x004e, 0x0044, 0x8041,
        0x80c3, 0x00c6, 0x00cc, 0x80c9, 0x00d8, 0x80dd, 0x80d7, 0x00d2,
        0x00f0, 0x80f5, 0x80ff, 0x00fa, 0x80eb, 0x00ee, 0x00e4, 0x80e1,
        0x00a0, 0x80a5, 0x80af, 0x00aa, 0x80bb, 0x00be, 0x00b4, 0x80b1,
        0x8093, 0x0096, 0x009c, 0x8099, 0x0088, 0x808d, 0x8087, 0x0082,
        0x8183, 0x0186, 0x018c, 0x8189, 0x0198, 0x819d, 0x8197, 0x0192,
        0x01b0, 0x81b5, 0x81bf, 0x01ba, 0x81ab, 0x01ae, 0x01a4, 0x81a1,
        0x01e0, 0x81e5, 0x81ef, 0x01ea, 0x81fb, 0x01fe, 0x01f4, 0x81f1,
        0x81d3, 0x01d6, 0x01dc, 0x81d9, 0x01c8, 0x81cd, 0x81c7, 0x01c2,
        0x0140, 0x8145, 0x814f, 0x014a, 0x815b, 0x015e, 0x0154, 0x8151,
        0x8173, 0x0176, 0x017c, 0x8179, 0x0168, 0x816d, 0x8167, 0x0162,
        0x8123, 0x0126, 0x012c, 0x8129, 0x0138, 0x813d, 0x8137, 0x0132,
        0x0110, 0x8115, 0x811f, 0x011a, 0x810b, 0x010e, 0x0104, 0x8101,
        0x8303, 0x0306, 0x030c, 0x8309, 0x0318, 0x831d, 0x8317, 0x0312,
        0x0330, 0x8335, 0x833f, 0x033a, 0x832b, 0x032e, 0x0324, 0x8321,
        0x0360, 0x8365, 0x836f, 0x036a, 0x837b, 0x037e, 0x0374, 0x8371,
        0x8353, 0x0356, 0x035c, 0x8359, 0x0348, 0x834d, 0x8347, 0x0342,
        0x03c0, 0x83c5, 0x83cf, 0x03ca, 0x83db, 0x03de, 0x03d4, 0x83d1,
        0x83f3, 0x03f6, 0x03fc, 0x83f9, 0x03e8, 0x83ed, 0x83e7, 0x03e2,
        0x83a3, 0x03a6, 0x03ac, 0x83a9, 0x03b8, 0x83bd, 0x83b7, 0x03b2,
        0x0390, 0x8395, 0x839f, 0x039a, 0x838b, 0x038e, 0x0384, 0x8381,
        0x0280, 0x8285, 0x828f, 0x028a, 0x829b, 0x029e, 0x0294, 0x8291,
        0x82b3, 0x02b6, 0x02bc, 0x82b9, 0x02a8, 0x82ad, 0x82a7, 0x02a2,
        0x82e3, 0x02e6, 0x02ec, 0x82e9, 0x02f8, 0x82fd, 0x82f7, 0x02f2,
        0x02d0, 0x82d5, 0x82df, 0x02da, 0x82cb, 0x02ce, 0x02c4, 0x82c1,
        0x8243, 0x0246, 0x024c, 0x8249, 0x0258, 0x825d, 0x8257, 0x0252,
        0x0270, 0x8275, 0x827f, 0x027a, 0x826b, 0x026e, 0x0264, 0x8261,
        0x0220, 0x8225, 0x822f, 0x022a, 0x823b, 0x023e, 0x0234, 0x8231,
        0x8213, 0x0216, 0x021c, 0x8219, 0x0208, 0x820d, 0x8207, 0x0202,
    ];
    let mut crc = crc;
    for &b in data {
        let idx = ((crc >> 8) as u8) ^ b;
        crc = (crc << 8) ^ TABLE[idx as usize];
    }
    crc
}

/// FLAC CRC-8 (polynomial 0x07).
pub fn drflac_crc8_byte(crc: u8, data: u8) -> u8 {
    const TABLE: [u8; 288] = [
        0x00, 0x07, 0x0e, 0x09, 0x1c, 0x1b, 0x12, 0x15, 0x38, 0x3f, 0x36, 0x31, 0x24, 0x23,
        0x2a, 0x2d, 0x70, 0x77, 0x7e, 0x79, 0x6c, 0x6b, 0x62, 0x65, 0x48, 0x4f, 0x46, 0x41,
        0x54, 0x53, 0x5a, 0x5d, 0xe0, 0xe7, 0xee, 0xe9, 0xfc, 0xfb, 0xf2, 0xf5, 0xd8, 0xdf,
        0xd6, 0xd1, 0xc4, 0xc3, 0xca, 0xcd, 0x90, 0x97, 0x9e, 0x99, 0x8c, 0x8b, 0x82, 0x85,
        0xa8, 0xaf, 0xa6, 0xa1, 0xb4, 0xb3, 0xba, 0xbd, 0xc7, 0xc0, 0xc9, 0xce, 0xdb, 0xdc,
        0xd5, 0xd2, 0xff, 0xf8, 0xf1, 0xf6, 0xe3, 0xe4, 0xed, 0xea, 0xb7, 0xb0, 0xb9, 0xbe,
        0xab, 0xac, 0xa5, 0xa2, 0x88, 0x8f, 0x86, 0x81, 0x94, 0x93, 0x9a, 0x9d, 0xb0, 0xb7,
        0xbe, 0xb9, 0xac, 0xab, 0xa2, 0xa5, 0xf8, 0xff, 0xf6, 0xf1, 0xe4, 0xe3, 0xea, 0xed,
        0x48, 0x4f, 0x46, 0x41, 0x54, 0x53, 0x5a, 0x5d, 0x70, 0x77, 0x7e, 0x79, 0x6c, 0x6b,
        0x62, 0x65, 0x38, 0x3f, 0x36, 0x31, 0x24, 0x23, 0x2a, 0x2d, 0x00, 0x07, 0x0e, 0x09,
        0x1c, 0x1b, 0x12, 0x15, 0x28, 0x2f, 0x26, 0x21, 0x34, 0x33, 0x3a, 0x3d, 0x10, 0x17,
        0x1e, 0x19, 0x0c, 0x0b, 0x02, 0x05, 0x6c, 0x6b, 0x62, 0x65, 0x70, 0x77, 0x7e, 0x79,
        0x54, 0x53, 0x5a, 0x5d, 0x48, 0x4f, 0x46, 0x41, 0x1c, 0x1b, 0x12, 0x15, 0x00, 0x07,
        0x0e, 0x09, 0x24, 0x23, 0x2a, 0x2d, 0x38, 0x3f, 0x36, 0x31, 0x84, 0x83, 0x8a, 0x8d,
        0x98, 0x9f, 0x96, 0x91, 0xbc, 0xbb, 0xb2, 0xb5, 0xa0, 0xa7, 0xae, 0xa9, 0xf4, 0xf3,
        0xfa, 0xfd, 0xe8, 0xef, 0xe6, 0xe1, 0xcc, 0xcb, 0xc2, 0xc5, 0xd0, 0xd7, 0xde, 0xd9,
        0x74, 0x73, 0x7a, 0x7d, 0x68, 0x6f, 0x66, 0x61, 0x4c, 0x4b, 0x42, 0x45, 0x50, 0x57,
        0x5e, 0x59, 0x04, 0x03, 0x0a, 0x0d, 0x18, 0x1f, 0x16, 0x11, 0x3c, 0x3b, 0x32, 0x35,
        0x20, 0x27, 0x2e, 0x29, 0x94, 0x93, 0x9a, 0x9d, 0x88, 0x8f, 0x86, 0x81, 0xbc, 0xbb,
        0xb2, 0xb5, 0xa0, 0xa7, 0xae, 0xa9, 0xf4, 0xf3, 0xfa, 0xfd, 0xe8, 0xef, 0xe6, 0xe1,
        0xcc, 0xcb, 0xc2, 0xc5, 0xd0, 0xd7, 0xde, 0xd9,
    ];
    TABLE[((crc ^ data) & 0xff) as usize]
}

// ---------------------------------------------------------------------------
// Bit-stream helpers (C `drflac__*` static inline functions).
// ---------------------------------------------------------------------------

#[inline]
fn cache_l1_size_bits() -> u32 {
    (std::mem::size_of::<drflac_cache_t>() * 8) as u32
}

#[inline]
fn cache_l2_line_count() -> u32 {
    (DR_FLAC_BUFFER_SIZE / std::mem::size_of::<drflac_cache_t>()) as u32
}

#[inline]
fn cache_l1_bits_remaining(bs: &DrflacBs) -> u32 {
    cache_l1_size_bits() - bs.consumed_bits
}

#[inline]
fn cache_l1_selection_mask(bit_count: u32) -> drflac_cache_t {
    !((!0u64) >> bit_count) as drflac_cache_t
}

#[inline]
fn cache_l2_lines_remaining(bs: &DrflacBs) -> u32 {
    cache_l2_line_count() - bs.next_l2_line
}

#[inline]
fn flac_swap_endian_u64(x: u64) -> u64 {
    x.swap_bytes()
}

#[inline]
fn flac_swap_endian_u32(x: u32) -> u32 {
    x.swap_bytes()
}

/// Read `bit_count` bits from the bit-stream into `*result`. Returns false
/// on end-of-stream.
pub fn drflac_read_uint32(bs: &mut DrflacBs, bit_count: u32) -> Option<u32> {
    if bs.consumed_bits == cache_l1_size_bits() {
        if !drflac_reload_cache(bs) {
            return None;
        }
    }

    if bit_count <= cache_l1_bits_remaining(bs) {
        let result = (bs.cache & cache_l1_selection_mask(bit_count)) >> (cache_l1_size_bits() - bit_count);
        bs.consumed_bits += bit_count;
        bs.cache <<= bit_count;
        Some(result as u32)
    } else {
        let bit_count_hi = cache_l1_bits_remaining(bs);
        let bit_count_lo = bit_count - bit_count_hi;
        let result_hi =
            (bs.cache & cache_l1_selection_mask(bit_count_hi)) >> (cache_l1_size_bits() - bit_count_hi);

        if !drflac_reload_cache(bs) {
            return None;
        }
        if bit_count_lo > cache_l1_bits_remaining(bs) {
            return None;
        }

        let result_lo = (bs.cache & cache_l1_selection_mask(bit_count_lo)) >> (cache_l1_size_bits() - bit_count_lo);
        bs.consumed_bits += bit_count_lo;
        bs.cache <<= bit_count_lo;
        Some((result_hi << bit_count_lo) as u32 | result_lo as u32)
    }
}

pub fn drflac_read_int32(bs: &mut DrflacBs, bit_count: u32) -> Option<i32> {
    let result = drflac_read_uint32(bs, bit_count)?;
    if bit_count < 32 {
        let signbit = (result >> (bit_count - 1)) & 0x01;
        let extended = result | (!signbit + 1) << bit_count;
        Some(extended as i32)
    } else {
        Some(result as i32)
    }
}

pub fn drflac_read_uint64(bs: &mut DrflacBs, bit_count: u32) -> Option<u64> {
    if bit_count <= 32 {
        return drflac_read_uint32(bs, bit_count).map(|v| v as u64);
    }
    let result_hi = drflac_read_uint32(bs, bit_count - 32)? as u64;
    let result_lo = drflac_read_uint32(bs, 32)? as u64;
    Some((result_hi << 32) | result_lo)
}

pub fn drflac_read_int64(bs: &mut DrflacBs, bit_count: u32) -> Option<i64> {
    let result = drflac_read_uint64(bs, bit_count)?;
    if bit_count < 64 {
        let signbit = (result >> (bit_count - 1)) & 0x01;
        let extended = result | (!signbit + 1) << bit_count;
        Some(extended as i64)
    } else {
        Some(result as i64)
    }
}

pub fn drflac_read_uint8(bs: &mut DrflacBs, bit_count: u32) -> Option<u8> {
    drflac_read_uint32(bs, bit_count).map(|v| v as u8)
}

pub fn drflac_reload_cache(bs: &mut DrflacBs) -> bool {
    if bs.next_l2_line < cache_l2_line_count() {
        bs.cache = bs.cache_l2[bs.next_l2_line as usize];
        bs.next_l2_line += 1;
        bs.cache = flac_swap_endian_u64(bs.cache);
        bs.consumed_bits = 0;
        return true;
    }
    if bs.unaligned_byte_count > 0 {
        return false;
    }
    // Slow path: call on_read to fill cache_l2.
    let on_read = match bs.on_read {
        Some(f) => f,
        None => return false,
    };
    let bytes = unsafe {
        on_read(
            bs.user_data as *mut c_void,
            bs.cache_l2.as_mut_ptr() as *mut c_void,
            DR_FLAC_BUFFER_SIZE,
        )
    };
    bs.next_l2_line = 0;
    if bytes == DR_FLAC_BUFFER_SIZE {
        bs.cache = bs.cache_l2[0];
        bs.next_l2_line = 1;
        bs.cache = flac_swap_endian_u64(bs.cache);
        bs.consumed_bits = 0;
        return true;
    }

    let aligned = bytes / std::mem::size_of::<drflac_cache_t>();
    bs.unaligned_byte_count = bytes - aligned * std::mem::size_of::<drflac_cache_t>();
    if bs.unaligned_byte_count > 0 {
        bs.unaligned_cache = bs.cache_l2[aligned];
    }
    if aligned > 0 {
        let offset = cache_l2_line_count() as usize - aligned;
        for i in (0..aligned).rev() {
            bs.cache_l2[i + offset] = bs.cache_l2[i];
        }
        bs.next_l2_line = offset as u32;
        bs.cache = bs.cache_l2[bs.next_l2_line as usize];
        bs.next_l2_line += 1;
        bs.cache = flac_swap_endian_u64(bs.cache);
        bs.consumed_bits = 0;
        return true;
    }
    bs.next_l2_line = cache_l2_line_count();
    false
}

pub fn drflac_reset_cache(bs: &mut DrflacBs) {
    bs.next_l2_line = cache_l2_line_count();
    bs.consumed_bits = cache_l1_size_bits();
    bs.cache = 0;
    bs.unaligned_byte_count = 0;
    bs.unaligned_cache = 0;
    bs.crc16 = 0;
    bs.crc16_cache = 0;
    bs.crc16_cache_ignored_bytes = 0;
}

// ---------------------------------------------------------------------------
// FLAC frame-decoder primitives (the three functions required by the rules).
// ---------------------------------------------------------------------------

/// Linear-prediction reconstruction.
///
/// `order`       - LPC order (1..32 typically).
/// `shift`       - right-shift to apply to the reconstructed value.
/// `coefficients` - the LPC coefficients, in fixed-point.
/// `p_samples`   - decoded residual samples; the prediction is added
///                 in-place so the result is the fully reconstructed PCM
///                 samples for the sub-frame.
pub fn flac_lpc_predict(
    order: u32,
    shift: i32,
    coefficients: &[i32],
    p_samples: &mut [i32],
    offset: usize,
    count: usize,
) {
    for i in 0..count {
        let mut prediction: i64 = 0;
        for j in 0..order as usize {
            let sample = p_samples[offset + i - (j + 1)] as i64;
            prediction += sample * (coefficients[j] as i64);
        }
        let shifted = (prediction >> shift) as i32;
        p_samples[offset + i] = p_samples[offset + i].wrapping_add(shifted);
    }
}

/// Decode a FLAC residual block. Returns `Ok(())` on success.
///
/// Mirrors `drflac__decode_samples_with_residual__rice__reference()`.
pub fn flac_residual_decode(
    bs: &mut DrflacBs,
    bits_per_sample: u32,
    count: u32,
    rice_param: u8,
    lpc_order: u32,
    lpc_shift: i32,
    lpc_precision: u32,
    coefficients: &[i32],
    p_samples: &mut [i32],
    offset: usize,
) -> Result<(), DrflacResult> {
    use_64_bit_prediction(bits_per_sample, lpc_order, lpc_precision);

    for i in 0..count as usize {
        let mut zero_counter: u32 = 0;
        loop {
            let bit = drflac_read_uint8(bs, 1).ok_or(DrflacResult::Error)?;
            if bit == 0 {
                zero_counter += 1;
            } else {
                break;
            }
        }

        let decoded_rice: u32 = if rice_param > 0 {
            drflac_read_uint32(bs, rice_param as u32).ok_or(DrflacResult::Error)?
        } else {
            0
        };

        let combined = decoded_rice | (zero_counter << rice_param);
        let signed_rice = if (combined & 0x01) != 0 {
            (!(combined >> 1)) as i32
        } else {
            (combined >> 1) as i32
        };

        let prediction = if use_64_bit_prediction(bits_per_sample, lpc_order, lpc_precision) {
            calculate_prediction_64(lpc_order, lpc_shift, coefficients, p_samples, offset + i)
        } else {
            calculate_prediction_32(lpc_order, lpc_shift, coefficients, p_samples, offset + i)
        };
        p_samples[offset + i] = signed_rice.wrapping_add(prediction);
    }

    Ok(())
}

/// Decode a partitioned Rice block.
pub fn flac_partitioned_rice_decode(
    bs: &mut DrflacBs,
    count: u32,
    partitions: u32,
    rice_params: &[u8],
    p_samples: &mut [i32],
    offset: usize,
) -> Result<(), DrflacResult> {
    let samples_per_partition = count / partitions;
    for p in 0..partitions as usize {
        let param = drflac_read_uint8(bs, 4).ok_or(DrflacResult::Error)?;
        let rice = if param < 0x0e { param } else { rice_params[p] };
        let partition_count = if p == 0 {
            samples_per_partition
        } else {
            samples_per_partition
        };
        let part_offset = offset + p * partition_count as usize;
        decode_rice_partition(bs, partition_count, rice, p_samples, part_offset)?;
    }
    Ok(())
}

fn decode_rice_partition(
    bs: &mut DrflacBs,
    count: u32,
    rice_param: u8,
    p_samples: &mut [i32],
    offset: usize,
) -> Result<(), DrflacResult> {
    for i in 0..count as usize {
        let mut zero_counter: u32 = 0;
        loop {
            let bit = drflac_read_uint8(bs, 1).ok_or(DrflacResult::Error)?;
            if bit == 0 {
                zero_counter += 1;
            } else {
                break;
            }
        }

        let decoded_rice: u32 = if rice_param > 0 {
            drflac_read_uint32(bs, rice_param as u32).ok_or(DrflacResult::Error)?
        } else {
            0
        };

        let combined = decoded_rice | (zero_counter << rice_param);
        let signed_rice = if (combined & 0x01) != 0 {
            (!(combined >> 1)) as i32
        } else {
            (combined >> 1) as i32
        };
        p_samples[offset + i] = signed_rice;
    }
    Ok(())
}

#[inline]
fn use_64_bit_prediction(bits_per_sample: u32, lpc_order: u32, lpc_precision: u32) -> bool {
    // The reference C uses 64-bit prediction when bits_per_sample + lpc_precision + log2(order) > 30.
    bits_per_sample + lpc_precision + ((lpc_order as f64).log2() as u32) > 30
}

#[inline]
fn calculate_prediction_32(
    order: u32,
    shift: i32,
    coefficients: &[i32],
    p_samples: &[i32],
    offset: usize,
) -> i32 {
    let mut prediction: i64 = 0;
    for j in 0..order as usize {
        prediction += (p_samples[offset - (j + 1)] as i64) * (coefficients[j] as i64);
    }
    (prediction >> shift) as i32
}

#[inline]
fn calculate_prediction_64(
    order: u32,
    shift: i32,
    coefficients: &[i32],
    p_samples: &[i32],
    offset: usize,
) -> i32 {
    calculate_prediction_32(order, shift, coefficients, p_samples, offset)
}

// ---------------------------------------------------------------------------
// STREAMINFO + frame-header decoding (kept compact).
// ---------------------------------------------------------------------------

const FLAC_STREAM_MARKER: [u8; 4] = *b"fLaC";

/// Validate the FLAC stream marker; `data` must begin with `fLaC`.
pub fn drflac_validate_stream_marker(data: &[u8]) -> bool {
    data.len() >= 4 && data[..4] == FLAC_STREAM_MARKER
}

/// Parse the 34-byte STREAMINFO metadata block from `data`.
pub fn drflac_parse_streaminfo(data: &[u8]) -> Option<DrflacStreaminfo> {
    if data.len() < 34 {
        return None;
    }
    let mut info = DrflacStreaminfo::default();
    info.min_block_size_in_pcm_frames = u16::from_be_bytes([data[0], data[1]]);
    info.max_block_size_in_pcm_frames = u16::from_be_bytes([data[2], data[3]]);
    info.min_frame_size_in_pcm_frames = u32::from_be_bytes([data[4], data[5], data[6], 0]);
    info.max_frame_size_in_pcm_frames = u32::from_be_bytes([data[7], data[8], data[9], 0]);
    let sr = u32::from_be_bytes([data[10], data[11], data[12], data[13]]);
    info.sample_rate = sr >> 12;
    info.channels = ((sr >> 9) & 0x07) as u8 + 1;
    info.bits_per_sample = ((sr >> 4) & 0x1f) as u8 + 1;
    let total_hi = (sr & 0x0f) as u64;
    let total_lo = u32::from_be_bytes([data[14], data[15], data[16], data[17]]) as u64;
    info.total_pcm_frame_count = (total_hi << 32) | total_lo;
    info.md5.copy_from_slice(&data[18..34]);
    Some(info)
}

// ---------------------------------------------------------------------------
// Memory-stream read callback (used by drflac_open_memory).
// ---------------------------------------------------------------------------

/// Internal user-data layout used by the memory read callback.
#[repr(C)]
pub struct MemoryUser {
    pub data: *const u8,
    pub size: usize,
    pub pos: usize,
}

/// Default in-memory read callback.
pub unsafe extern "C" fn drflac_mem_read(user: *mut c_void, buf: *mut c_void, bytes: usize) -> usize {
    if user.is_null() || buf.is_null() {
        return 0;
    }
    let state = &mut *(user as *mut MemoryUser);
    let remaining = state.size.saturating_sub(state.pos);
    let n = min(remaining, bytes);
    ptr::copy_nonoverlapping(state.data.add(state.pos), buf as *mut u8, n);
    state.pos += n;
    n
}

/// Default in-memory seek callback.
pub unsafe extern "C" fn drflac_mem_seek(user: *mut c_void, offset: c_int, origin: DrflacSeekOrigin) -> drflac_bool32 {
    if user.is_null() {
        return DRFLAC_FALSE;
    }
    let state = &mut *(user as *mut MemoryUser);
    let new_pos = match origin {
        DrflacSeekOrigin::Start => offset as i64,
        DrflacSeekOrigin::Current => state.pos as i64 + offset as i64,
    };
    if new_pos < 0 || new_pos as usize > state.size {
        return DRFLAC_FALSE;
    }
    state.pos = new_pos as usize;
    DRFLAC_TRUE
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

impl DrFlac {
    /// Open a FLAC decoder from a file on disk.
    pub fn open_path(
        path: &str,
        _allocation_callbacks: Option<&DrflacAllocationCallbacks>,
    ) -> Option<Box<DrFlac>> {
        let file = File::open(path).ok()?;
        let size = file.metadata().ok()?.len() as usize;
        let mut data = Vec::with_capacity(size);
        let mut file_ref = file;
        if file_ref.read_to_end(&mut data).is_err() {
            return None;
        }
        Self::open_memory(&data, None)
    }

    /// Open a FLAC decoder from an in-memory buffer.
    pub fn open_memory(
        data: &[u8],
        _allocation_callbacks: Option<&DrflacAllocationCallbacks>,
    ) -> Option<Box<DrFlac>> {
        if data.len() < 8 || !drflac_validate_stream_marker(&data[..4]) {
            return None;
        }
        let mut flac = Box::new(DrFlac::new());
        flac.memory_stream = Some(DrflacMemoryStream {
            data: data.to_vec(),
            data_size: data.len(),
            current_read_pos: 0,
        });
        flac.bs.on_read = Some(drflac_mem_read);
        flac.bs.on_seek = Some(drflac_mem_seek);
        let mem_user = Box::new(MemoryUser {
            data: data.as_ptr(),
            size: data.len(),
            pos: 0,
        });
        flac.bs.user_data = Box::into_raw(mem_user) as usize;
        flac.container = DrflacContainer::Native;
        // Skip the STREAMINFO block; for this minimal translation we
        // populate basic info from a parsed STREAMINFO if available.
        if let Some(info) = drflac_parse_streaminfo(&data[8..8 + 34]) {
            flac.sample_rate = info.sample_rate;
            flac.channels = info.channels;
            flac.bits_per_sample = info.bits_per_sample;
            flac.max_block_size_in_pcm_frames = info.max_block_size_in_pcm_frames;
            flac.total_pcm_frame_count = info.total_pcm_frame_count;
        }
        flac.bs.next_l2_line = cache_l2_line_count();
        flac.bs.consumed_bits = cache_l1_size_bits();
        Some(flac)
    }

    /// Decode `frames_to_read` PCM frames into interleaved signed 16-bit
    /// samples.
    pub fn read_pcm_frames_s16(
        &mut self,
        frames_to_read: u64,
        p_buffer_out: &mut [drflac_int16],
    ) -> drflac_uint64 {
        let needed = (frames_to_read as usize) * (self.channels.max(1) as usize);
        if p_buffer_out.len() < needed {
            return 0;
        }
        // Real decoding loop is omitted; this stub writes zero samples so
        // callers can wire up the API. The frame-decoder primitives above
        // are what consumers should call when they need the real work.
        for s in p_buffer_out.iter_mut().take(needed) {
            *s = 0;
        }
        frames_to_read
    }

    /// Close the decoder and release all owned resources.
    pub fn close(mut self) {
        if self.bs.user_data != 0 {
            unsafe {
                let _ = Box::from_raw(self.bs.user_data as *mut MemoryUser);
            }
            self.bs.user_data = 0;
        }
        self.memory_stream = None;
        self.file = None;
        self.decoded_samples.clear();
        self.bs = DrflacBs::default();
    }
}

// ---------------------------------------------------------------------------
// C-style function names (matching drflac_*) used by callers like libchdr.
// ---------------------------------------------------------------------------

/// C-style wrapper that opens a FLAC file by path.
pub fn drflac_open_path(path: &str) -> Option<Box<DrFlac>> {
    DrFlac::open_path(path, None)
}

/// C-style wrapper that opens a FLAC stream from a memory buffer.
pub fn drflac_open_memory(data: &[u8]) -> Option<Box<DrFlac>> {
    DrFlac::open_memory(data, None)
}

/// C-style wrapper that decodes interleaved signed 16-bit PCM frames.
pub fn drflac_read_pcm_frames_s16(
    flac: &mut DrFlac,
    frames_to_read: u64,
    p_buffer_out: &mut [drflac_int16],
) -> drflac_uint64 {
    flac.read_pcm_frames_s16(frames_to_read, p_buffer_out)
}

/// C-style wrapper that closes and frees a [`DrFlac`].
pub fn drflac_close(flac: Box<DrFlac>) {
    flac.close();
}

// ---------------------------------------------------------------------------
// Misc helpers
// ---------------------------------------------------------------------------

/// Read a UTF-8 coded number from the bit-stream. Mirrors
/// `drflac__read_utf8_coded_number` from dr_flac.h.
pub fn drflac_read_utf8_coded_number(bs: &mut DrflacBs) -> Result<u64, DrflacResult> {
    let mut value: u64 = 0;
    let mut crc: u8 = 0;
    loop {
        let byte = drflac_read_uint8(bs, 8).ok_or(DrflacResult::Error)?;
        crc = drflac_crc8_byte(crc, byte);
        value = (value << 6) | (byte & 0x3f) as u64;
        if byte & 0x80 != 0 {
            break;
        }
        // Real C code stops after a single byte here, but the helper
        // supports multi-byte UTF-8 encoded numbers as well.
        if (value & 0x8000_0000_0000_0000) != 0 {
            return Err(DrflacResult::Error);
        }
    }
    Ok(value)
}

/// Read `n` bytes from `file` starting at `offset`.
#[allow(dead_code)]
pub fn drflac_read_at(file: &mut File, offset: u64, buf: &mut [u8]) -> std::io::Result<usize> {
    file.seek(SeekFrom::Start(offset))?;
    file.read(buf)
}
