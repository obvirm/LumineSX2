//! FFmpeg C library — idiomatic Rust 2021 translation of the public C headers.
//!
//! This module mirrors the relevant surface of:
//!   * `libavcodec`   (avcodec.h, codec.h, codec_id.h, codec_par.h, packet.h)
//!   * `libavformat`  (avformat.h, avio.h)
//!   * `libavutil`    (avutil.h, buffer.h, dict.h, frame.h, imgutils.h,
//!                    opt.h, pixdesc.h, pixfmt.h, rational.h)
//!   * `libswscale`   (swscale.h)
//!
//! Only the headers are translated — function bodies remain linked from the
//! upstream `avcodec`/`avformat`/`avutil`/`swscale` shared libraries at runtime.
//! All structs keep their FFmpeg C ABI layout so that pointers obtained from
//! the FFI surface can be dereferenced through these definitions.
//!
//! No external crates are used; the only allowed dependency is `std`.

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]
#![allow(clippy::all)]

use std::os::raw::{c_char, c_double, c_int, c_uint, c_uchar, c_ulong, c_void};

// =====================================================================
// Common primitives
// =====================================================================

pub type int8_t = i8;
pub type int16_t = i16;
pub type int32_t = i32;
pub type int64_t = i64;
pub type uint8_t = u8;
pub type uint16_t = u16;
pub type uint32_t = u32;
pub type uint64_t = u64;
pub type size_t = usize;
pub type ptrdiff_t = isize;

pub const AV_NOPTS_VALUE: i64 = i64::MIN;
pub const AV_TIME_BASE: c_int = 1_000_000;
pub const AV_NUM_DATA_POINTERS: usize = 8;
pub const AVPALETTE_SIZE: c_int = 1024;
pub const AVPALETTE_COUNT: c_int = 256;
pub const AV_VIDEO_MAX_PLANES: c_int = 4;

/// The number of planes in a [`AVPicture`]. Kept for ABI parity.
pub const AVPICTURE_MAX_PLANES: usize = 4;

// =====================================================================
// libavutil — media type, picture type, rational
// =====================================================================

/// Media kind carried by a stream or codec.
#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum AVMediaType {
    AVMEDIA_TYPE_UNKNOWN = -1,
    AVMEDIA_TYPE_VIDEO,
    AVMEDIA_TYPE_AUDIO,
    AVMEDIA_TYPE_DATA,
    AVMEDIA_TYPE_SUBTITLE,
    AVMEDIA_TYPE_ATTACHMENT,
    AVMEDIA_TYPE_NB,
}

/// Picture type classification of a decoded video frame.
#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum AVPictureType {
    AV_PICTURE_TYPE_NONE = 0,
    AV_PICTURE_TYPE_I,
    AV_PICTURE_TYPE_P,
    AV_PICTURE_TYPE_B,
    AV_PICTURE_TYPE_S,
    AV_PICTURE_TYPE_SI,
    AV_PICTURE_TYPE_SP,
    AV_PICTURE_TYPE_BI,
}

/// Rational number (numerator / denominator) used for framerates, timebases,
/// aspect ratios and similar quantities.
#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub struct AVRational {
    pub num: c_int,
    pub den: c_int,
}

impl AVRational {
    #[inline]
    pub const fn new(num: c_int, den: c_int) -> Self {
        Self { num, den }
    }

    #[inline]
    pub const fn zero() -> Self {
        Self { num: 0, den: 0 }
    }

    #[inline]
    pub fn to_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }
}

// =====================================================================
// libavutil — pixel format descriptors and constants
// =====================================================================

/// Pixel format identifiers. Only the most common values used by PCSX2 are
/// named here; the full enum is represented as a 32-bit integer for ABI
/// compatibility with the upstream library.
pub type AVPixelFormat = c_int;

pub const AV_PIX_FMT_NONE: AVPixelFormat = -1;
pub const AV_PIX_FMT_YUV420P: AVPixelFormat = 0;
pub const AV_PIX_FMT_YUYV422: AVPixelFormat = 1;
pub const AV_PIX_FMT_RGB24: AVPixelFormat = 2;
pub const AV_PIX_FMT_BGR24: AVPixelFormat = 3;
pub const AV_PIX_FMT_YUV422P: AVPixelFormat = 4;
pub const AV_PIX_FMT_YUV444P: AVPixelFormat = 5;
pub const AV_PIX_FMT_YUV410P: AVPixelFormat = 6;
pub const AV_PIX_FMT_YUV411P: AVPixelFormat = 7;
pub const AV_PIX_FMT_GRAY8: AVPixelFormat = 8;
pub const AV_PIX_FMT_MONOWHITE: AVPixelFormat = 9;
pub const AV_PIX_FMT_MONOBLACK: AVPixelFormat = 10;
pub const AV_PIX_FMT_PAL8: AVPixelFormat = 11;
pub const AV_PIX_FMT_YUVJ420P: AVPixelFormat = 12;
pub const AV_PIX_FMT_YUVJ422P: AVPixelFormat = 13;
pub const AV_PIX_FMT_YUVJ444P: AVPixelFormat = 14;
pub const AV_PIX_FMT_UYVY422: AVPixelFormat = 15;
pub const AV_PIX_FMT_UYYVYY411: AVPixelFormat = 16;
pub const AV_PIX_FMT_BGR8: AVPixelFormat = 17;
pub const AV_PIX_FMT_BGR4: AVPixelFormat = 18;
pub const AV_PIX_FMT_BGR4_BYTE: AVPixelFormat = 19;
pub const AV_PIX_FMT_RGB8: AVPixelFormat = 20;
pub const AV_PIX_FMT_RGB4: AVPixelFormat = 21;
pub const AV_PIX_FMT_RGB4_BYTE: AVPixelFormat = 22;
pub const AV_PIX_FMT_NV12: AVPixelFormat = 23;
pub const AV_PIX_FMT_NV21: AVPixelFormat = 24;
pub const AV_PIX_FMT_ARGB: AVPixelFormat = 25;
pub const AV_PIX_FMT_RGBA: AVPixelFormat = 26;
pub const AV_PIX_FMT_ABGR: AVPixelFormat = 27;
pub const AV_PIX_FMT_BGRA: AVPixelFormat = 28;
pub const AV_PIX_FMT_GRAY16BE: AVPixelFormat = 29;
pub const AV_PIX_FMT_GRAY16LE: AVPixelFormat = 30;
pub const AV_PIX_FMT_YUV440P: AVPixelFormat = 31;
pub const AV_PIX_FMT_YUVJ440P: AVPixelFormat = 32;
pub const AV_PIX_FMT_YUVA420P: AVPixelFormat = 33;
pub const AV_PIX_FMT_RGB48BE: AVPixelFormat = 34;
pub const AV_PIX_FMT_RGB48LE: AVPixelFormat = 35;
pub const AV_PIX_FMT_RGB565BE: AVPixelFormat = 36;
pub const AV_PIX_FMT_RGB565LE: AVPixelFormat = 37;
pub const AV_PIX_FMT_RGB555BE: AVPixelFormat = 38;
pub const AV_PIX_FMT_RGB555LE: AVPixelFormat = 39;
pub const AV_PIX_FMT_BGR565BE: AVPixelFormat = 40;
pub const AV_PIX_FMT_BGR565LE: AVPixelFormat = 41;
pub const AV_PIX_FMT_BGR555BE: AVPixelFormat = 42;
pub const AV_PIX_FMT_BGR555LE: AVPixelFormat = 43;
pub const AV_PIX_FMT_VAAPI: AVPixelFormat = 44;
pub const AV_PIX_FMT_YUV420P16LE: AVPixelFormat = 45;
pub const AV_PIX_FMT_YUV420P16BE: AVPixelFormat = 46;
pub const AV_PIX_FMT_YUV422P16LE: AVPixelFormat = 47;
pub const AV_PIX_FMT_YUV422P16BE: AVPixelFormat = 48;
pub const AV_PIX_FMT_YUV444P16LE: AVPixelFormat = 49;
pub const AV_PIX_FMT_YUV444P16BE: AVPixelFormat = 50;
pub const AV_PIX_FMT_DXVA2_VLD: AVPixelFormat = 51;
pub const AV_PIX_FMT_RGB444LE: AVPixelFormat = 52;
pub const AV_PIX_FMT_RGB444BE: AVPixelFormat = 53;
pub const AV_PIX_FMT_BGR444LE: AVPixelFormat = 54;
pub const AV_PIX_FMT_BGR444BE: AVPixelFormat = 55;
pub const AV_PIX_FMT_YA8: AVPixelFormat = 56;
pub const AV_PIX_FMT_BGR48BE: AVPixelFormat = 57;
pub const AV_PIX_FMT_BGR48LE: AVPixelFormat = 58;
pub const AV_PIX_FMT_YUV420P9BE: AVPixelFormat = 59;
pub const AV_PIX_FMT_YUV420P9LE: AVPixelFormat = 60;
pub const AV_PIX_FMT_YUV420P10BE: AVPixelFormat = 61;
pub const AV_PIX_FMT_YUV420P10LE: AVPixelFormat = 62;
pub const AV_PIX_FMT_YUV422P10BE: AVPixelFormat = 63;
pub const AV_PIX_FMT_YUV422P10LE: AVPixelFormat = 64;
pub const AV_PIX_FMT_YUV444P9BE: AVPixelFormat = 65;
pub const AV_PIX_FMT_YUV444P9LE: AVPixelFormat = 66;
pub const AV_PIX_FMT_YUV444P10BE: AVPixelFormat = 67;
pub const AV_PIX_FMT_YUV444P10LE: AVPixelFormat = 68;
pub const AV_PIX_FMT_YUV422P9BE: AVPixelFormat = 69;
pub const AV_PIX_FMT_YUV422P9LE: AVPixelFormat = 70;
pub const AV_PIX_FMT_GBRP: AVPixelFormat = 71;
pub const AV_PIX_FMT_GBRP9BE: AVPixelFormat = 72;
pub const AV_PIX_FMT_GBRP9LE: AVPixelFormat = 73;
pub const AV_PIX_FMT_GBRP10BE: AVPixelFormat = 74;
pub const AV_PIX_FMT_GBRP10LE: AVPixelFormat = 75;
pub const AV_PIX_FMT_GBRP16BE: AVPixelFormat = 76;
pub const AV_PIX_FMT_GBRP16LE: AVPixelFormat = 77;
pub const AV_PIX_FMT_YUVA422P: AVPixelFormat = 78;
pub const AV_PIX_FMT_YUVA444P: AVPixelFormat = 79;
pub const AV_PIX_FMT_NB: AVPixelFormat = 304;

/// Single component of an [`AVPixFmtDescriptor`].
#[repr(C)]
#[derive(Debug, Copy, Clone, Default)]
pub struct AVComponentDescriptor {
    pub plane: c_int,
    pub step: c_int,
    pub offset: c_int,
    pub shift: c_int,
    pub depth: c_int,
}

/// Describes the layout of a pixel format: chroma subsampling, component
/// placement, flags, etc. Returned by `av_pix_fmt_desc_get`.
#[repr(C)]
pub struct AVPixFmtDescriptor {
    pub name: *const c_char,
    pub nb_components: uint8_t,
    pub log2_chroma_w: uint8_t,
    pub log2_chroma_h: uint8_t,
    pub flags: uint64_t,
    pub comp: [AVComponentDescriptor; 4],
    pub alias: *const c_char,
}

pub const AV_PIX_FMT_FLAG_BE: u64 = 1 << 0;
pub const AV_PIX_FMT_FLAG_PAL: u64 = 1 << 1;
pub const AV_PIX_FMT_FLAG_BITSTREAM: u64 = 1 << 2;
pub const AV_PIX_FMT_FLAG_HWACCEL: u64 = 1 << 3;
pub const AV_PIX_FMT_FLAG_PLANAR: u64 = 1 << 4;
pub const AV_PIX_FMT_FLAG_RGB: u64 = 1 << 5;
pub const AV_PIX_FMT_FLAG_ALPHA: u64 = 1 << 7;
pub const AV_PIX_FMT_FLAG_BAYER: u64 = 1 << 8;
pub const AV_PIX_FMT_FLAG_FLOAT: u64 = 1 << 9;
pub const AV_PIX_FMT_FLAG_XYZ: u64 = 1 << 10;

// =====================================================================
// libavutil — colour description enums
// =====================================================================

#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum AVColorPrimaries {
    AVCOL_PRI_RESERVED0 = 0,
    AVCOL_PRI_BT709 = 1,
    AVCOL_PRI_UNSPECIFIED = 2,
    AVCOL_PRI_RESERVED = 3,
    AVCOL_PRI_BT470M = 4,
    AVCOL_PRI_BT470BG = 5,
    AVCOL_PRI_SMPTE170M = 6,
    AVCOL_PRI_SMPTE240M = 7,
    AVCOL_PRI_FILM = 8,
    AVCOL_PRI_BT2020 = 9,
    AVCOL_PRI_SMPTE428 = 10,
    AVCOL_PRI_SMPTE431 = 11,
    AVCOL_PRI_SMPTE432 = 12,
    AVCOL_PRI_EBU3213 = 22,
    AVCOL_PRI_NB,
}

#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum AVColorTransferCharacteristic {
    AVCOL_TRC_RESERVED0 = 0,
    AVCOL_TRC_BT709 = 1,
    AVCOL_TRC_UNSPECIFIED = 2,
    AVCOL_TRC_RESERVED = 3,
    AVCOL_TRC_GAMMA22 = 4,
    AVCOL_TRC_GAMMA28 = 5,
    AVCOL_TRC_SMPTE170M = 6,
    AVCOL_TRC_SMPTE240M = 7,
    AVCOL_TRC_LINEAR = 8,
    AVCOL_TRC_LOG = 9,
    AVCOL_TRC_LOG_SQRT = 10,
    AVCOL_TRC_IEC61966_2_4 = 11,
    AVCOL_TRC_BT1361_ECG = 12,
    AVCOL_TRC_IEC61966_2_1 = 13,
    AVCOL_TRC_BT2020_10 = 14,
    AVCOL_TRC_BT2020_12 = 15,
    AVCOL_TRC_SMPTE2084 = 16,
    AVCOL_TRC_SMPTE428 = 17,
    AVCOL_TRC_ARIB_STD_B67 = 18,
    AVCOL_TRC_NB,
}

#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum AVColorSpace {
    AVCOL_SPC_RGB = 0,
    AVCOL_SPC_BT709 = 1,
    AVCOL_SPC_UNSPECIFIED = 2,
    AVCOL_SPC_RESERVED = 3,
    AVCOL_SPC_FCC = 4,
    AVCOL_SPC_BT470BG = 5,
    AVCOL_SPC_SMPTE170M = 6,
    AVCOL_SPC_SMPTE240M = 7,
    AVCOL_SPC_YCGCO = 8,
    AVCOL_SPC_BT2020_NCL = 9,
    AVCOL_SPC_BT2020_CL = 10,
    AVCOL_SPC_SMPTE2085 = 11,
    AVCOL_SPC_CHROMA_DERIVED_NCL = 12,
    AVCOL_SPC_CHROMA_DERIVED_CL = 13,
    AVCOL_SPC_ICTCP = 14,
    AVCOL_SPC_IPT_C2 = 15,
    AVCOL_SPC_YCGCO_RE = 16,
    AVCOL_SPC_YCGCO_RO = 17,
    AVCOL_SPC_NB,
}

#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum AVColorRange {
    AVCOL_RANGE_UNSPECIFIED = 0,
    AVCOL_RANGE_MPEG = 1,
    AVCOL_RANGE_JPEG = 2,
    AVCOL_RANGE_NB,
}

#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum AVChromaLocation {
    AVCHROMA_LOC_UNSPECIFIED = 0,
    AVCHROMA_LOC_LEFT = 1,
    AVCHROMA_LOC_CENTER = 2,
    AVCHROMA_LOC_TOPLEFT = 3,
    AVCHROMA_LOC_TOP = 4,
    AVCHROMA_LOC_BOTTOMLEFT = 5,
    AVCHROMA_LOC_BOTTOM = 6,
    AVCHROMA_LOC_NB,
}

// =====================================================================
// libavutil — sample format
// =====================================================================

#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum AVSampleFormat {
    AV_SAMPLE_FMT_NONE = -1,
    AV_SAMPLE_FMT_U8,
    AV_SAMPLE_FMT_S16,
    AV_SAMPLE_FMT_S32,
    AV_SAMPLE_FMT_FLT,
    AV_SAMPLE_FMT_DBL,
    AV_SAMPLE_FMT_U8P,
    AV_SAMPLE_FMT_S16P,
    AV_SAMPLE_FMT_S32P,
    AV_SAMPLE_FMT_FLTP,
    AV_SAMPLE_FMT_DBLP,
    AV_SAMPLE_FMT_S64,
    AV_SAMPLE_FMT_S64P,
    AV_SAMPLE_FMT_NB,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct AVChannelLayout {
    pub order: c_int,
    pub nb_channels: c_int,
    pub opaqueness: u64,
    pub mask: u64,
    pub map: [u8; 64],
}

// =====================================================================
// libavutil — buffer, dictionary, frame side data
// =====================================================================

#[repr(C)]
pub struct AVBuffer {
    _private: [u8; 0],
}

/// A reference to a data buffer.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct AVBufferRef {
    pub buffer: *mut AVBuffer,
    pub data: *mut uint8_t,
    pub size: size_t,
}

/// A single dictionary entry.
#[repr(C)]
pub struct AVDictionaryEntry {
    pub key: *mut c_char,
    pub value: *mut c_char,
}

/// Opaque dictionary used for stream metadata and option passing.
#[repr(C)]
pub struct AVDictionary {
    _private: [u8; 0],
}

/// Side data attached to an [`AVFrame`].
#[repr(C)]
pub struct AVFrameSideData {
    pub kind: AVFrameSideDataType,
    pub data: *mut uint8_t,
    pub size: size_t,
    pub metadata: *mut AVDictionary,
    pub buf: *mut AVBufferRef,
}

/// Re-export as `type` because the field name in C is `type`.
pub type AVFrameSideDataType = c_int;

#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum AVDiscard {
    AVDISCARD_NONE = -16,
    AVDISCARD_DEFAULT = 0,
    AVDISCARD_NONREF = 8,
    AVDISCARD_BIDIR = 16,
    AVDISCARD_NONINTRA = 24,
    AVDISCARD_NONKEY = 32,
    AVDISCARD_ALL = 48,
}

// =====================================================================
// libavutil — AVPicture (legacy API)
// =====================================================================

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct AVPicture {
    pub data: [*mut uint8_t; AVPICTURE_MAX_PLANES],
    pub linesize: [c_int; AVPICTURE_MAX_PLANES],
}

// =====================================================================
// libavutil — AVOption
// =====================================================================

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct AVRational_default {
    pub q: AVRational,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub union AVOption_default_val {
    pub i64: int64_t,
    pub dbl: c_double,
    pub str: *const c_char,
    pub q: AVRational,
    pub arr: *const AVOptionArrayDef,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct AVOptionArrayDef {
    pub def: *const c_char,
    pub size_min: c_uint,
    pub size_max: c_uint,
    pub sep: c_char,
}

/// One entry in the option table of an [`AVClass`].
#[repr(C)]
pub struct AVOption {
    pub name: *const c_char,
    pub help: *const c_char,
    pub offset: c_int,
    pub kind: AVOptionType,
    pub default_val: AVOption_default_val,
    pub min: c_double,
    pub max: c_double,
    pub flags: c_int,
    pub unit: *const c_char,
}

#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum AVOptionType {
    AV_OPT_TYPE_FLAGS = 1,
    AV_OPT_TYPE_INT,
    AV_OPT_TYPE_INT64,
    AV_OPT_TYPE_DOUBLE,
    AV_OPT_TYPE_FLOAT,
    AV_OPT_TYPE_STRING,
    AV_OPT_TYPE_RATIONAL,
    AV_OPT_TYPE_BINARY,
    AV_OPT_TYPE_DICT,
    AV_OPT_TYPE_UINT64,
    AV_OPT_TYPE_CONST,
    AV_OPT_TYPE_IMAGE_SIZE,
    AV_OPT_TYPE_PIXEL_FMT,
    AV_OPT_TYPE_SAMPLE_FMT,
    AV_OPT_TYPE_VIDEO_RATE,
    AV_OPT_TYPE_DURATION,
    AV_OPT_TYPE_COLOR,
    AV_OPT_TYPE_BOOL,
    AV_OPT_TYPE_CHLAYOUT,
    AV_OPT_TYPE_UINT,
    AV_OPT_TYPE_FLAG_ARRAY = 1 << 16,
}

/// Class descriptor used by the AVOptions machinery.
#[repr(C)]
pub struct AVClass {
    pub class_name: *const c_char,
    pub item_name: Option<extern "C" fn(*mut c_void) -> *const c_char>,
    pub option: *const AVOption,
    pub version: c_int,
    pub log_level_offset_offset: c_int,
    pub parent_log_ctx_offset: c_int,
    pub child_next: Option<extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void>,
    pub child_class_iterate: Option<extern "C" fn(*mut *mut c_void) -> *const AVClass>,
    pub category: AVClassCategory,
    pub get_category: Option<extern "C" fn(*mut c_void) -> AVClassCategory>,
    pub query_ranges: Option<
        extern "C" fn(*mut *const AVOptionRanges, *mut c_void, *const c_char, c_int) -> c_int,
    >,
    pub iterate: Option<extern "C" fn(*mut c_void) -> *mut c_void>,
    pub child_class: *const AVClass,
    pub child_class_next: Option<extern "C" fn(*const AVClass) -> *const AVClass>,
}

#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum AVClassCategory {
    AV_CLASS_CATEGORY_NA = 0,
    AV_CLASS_CATEGORY_INPUT,
    AV_CLASS_CATEGORY_OUTPUT,
    AV_CLASS_CATEGORY_MUXER,
    AV_CLASS_CATEGORY_DEMUXER,
    AV_CLASS_CATEGORY_ENCODER,
    AV_CLASS_CATEGORY_DECODER,
    AV_CLASS_CATEGORY_FILTER,
    AV_CLASS_CATEGORY_BITSTREAM_FILTER,
    AV_CLASS_CATEGORY_SWSCALER,
    AV_CLASS_CATEGORY_SWRESAMPLER,
    AV_CLASS_CATEGORY_HWACCEL,
    AV_CLASS_CATEGORY_DEVICE,
    AV_CLASS_CATEGORY_DEVICE_INPUT,
    AV_CLASS_CATEGORY_DEVICE_OUTPUT,
    AV_CLASS_CATEGORY_NB,
}

#[repr(C)]
pub struct AVOptionRanges {
    pub range: *mut *mut AVOptionRange,
    pub nb_ranges: c_int,
    pub nb_components: c_int,
}

#[repr(C)]
pub struct AVOptionRange {
    pub str: *const c_char,
    pub value_min: c_double,
    pub value_max: c_double,
    pub component_min: c_double,
    pub component_max: c_double,
    pub flags: c_int,
}

// =====================================================================
// libavcodec — codec IDs (top values used by PCSX2)
// =====================================================================

pub type AVCodecID = c_int;

pub const AV_CODEC_ID_NONE: AVCodecID = 0;
pub const AV_CODEC_ID_MPEG1VIDEO: AVCodecID = 1;
pub const AV_CODEC_ID_MPEG2VIDEO: AVCodecID = 2;
pub const AV_CODEC_ID_H261: AVCodecID = 3;
pub const AV_CODEC_ID_H263: AVCodecID = 4;
pub const AV_CODEC_ID_RV10: AVCodecID = 5;
pub const AV_CODEC_ID_RV20: AVCodecID = 6;
pub const AV_CODEC_ID_MJPEG: AVCodecID = 7;
pub const AV_CODEC_ID_MPEG4: AVCodecID = 12;
pub const AV_CODEC_ID_RAWVIDEO: AVCodecID = 13;
pub const AV_CODEC_ID_H264: AVCodecID = 27;
pub const AV_CODEC_ID_VP3: AVCodecID = 30;
pub const AV_CODEC_ID_THEORA: AVCodecID = 31;
pub const AV_CODEC_ID_ASV1: AVCodecID = 32;
pub const AV_CODEC_ID_ASV2: AVCodecID = 33;
pub const AV_CODEC_ID_VP8: AVCodecID = 139;
pub const AV_CODEC_ID_VP9: AVCodecID = 167;
pub const AV_CODEC_ID_HEVC: AVCodecID = 173;
pub const AV_CODEC_ID_AV1: AVCodecID = 226;
pub const AV_CODEC_ID_FIRST_AUDIO: AVCodecID = 0x10000;
pub const AV_CODEC_ID_PCM_S16LE: AVCodecID = 0x10000;
pub const AV_CODEC_ID_MP2: AVCodecID = 0x15000;
pub const AV_CODEC_ID_MP3: AVCodecID = 0x15001;
pub const AV_CODEC_ID_AAC: AVCodecID = 0x15002;
pub const AV_CODEC_ID_AC3: AVCodecID = 0x15003;
pub const AV_CODEC_ID_FLAC: AVCodecID = 0x15008;
pub const AV_CODEC_ID_OPUS: AVCodecID = 0x15038;
pub const AV_CODEC_ID_VORBIS: AVCodecID = 0x15005;
pub const AV_CODEC_ID_PROBE: AVCodecID = 0x19000;

#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum AVCodecHWConfigMethod {
    AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX = 0x01,
    AV_CODEC_HW_CONFIG_METHOD_HW_FRAMES_CTX = 0x02,
    AV_CODEC_HW_CONFIG_METHOD_INTERNAL = 0x04,
    AV_CODEC_HW_CONFIG_METHOD_AD_HOC = 0x08,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct AVCodecHWConfig {
    pub pix_fmt: AVPixelFormat,
    pub methods: c_int,
    pub device_type: AVHWDeviceType,
}

pub type AVHWDeviceType = c_int;
pub const AV_HWDEVICE_TYPE_NONE: AVHWDeviceType = 0;
pub const AV_HWDEVICE_TYPE_VDPAU: AVHWDeviceType = 1;
pub const AV_HWDEVICE_TYPE_CUDA: AVHWDeviceType = 2;
pub const AV_HWDEVICE_TYPE_VAAPI: AVHWDeviceType = 3;
pub const AV_HWDEVICE_TYPE_DXVA2: AVHWDeviceType = 4;
pub const AV_HWDEVICE_TYPE_QSV: AVHWDeviceType = 5;
pub const AV_HWDEVICE_TYPE_VIDEOTOOLBOX: AVHWDeviceType = 6;
pub const AV_HWDEVICE_TYPE_D3D11VA: AVHWDeviceType = 7;
pub const AV_HWDEVICE_TYPE_DRM: AVHWDeviceType = 8;
pub const AV_HWDEVICE_TYPE_OPENCL: AVHWDeviceType = 9;
pub const AV_HWDEVICE_TYPE_MEDIACODEC: AVHWDeviceType = 10;
pub const AV_HWDEVICE_TYPE_VULKAN: AVHWDeviceType = 11;
pub const AV_HWDEVICE_TYPE_D3D12VA: AVHWDeviceType = 12;

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct AVProfile {
    pub profile: c_int,
    pub name: *const c_char,
}

// =====================================================================
// libavcodec — codec, codec context, parameters, packet, frame
// =====================================================================

#[repr(C)]
pub struct AVCodec {
    pub name: *const c_char,
    pub long_name: *const c_char,
    pub kind: AVMediaType,
    pub id: AVCodecID,
    pub capabilities: c_int,
    pub max_lowres: uint8_t,
    pub supported_framerates: *const AVRational,
    pub pix_fmts: *const AVPixelFormat,
    pub supported_samplerates: *const c_int,
    pub sample_fmts: *const AVSampleFormat,
    pub priv_class: *const AVClass,
    pub profiles: *const AVProfile,
    pub wrapper_name: *const c_char,
    pub ch_layouts: *const AVChannelLayout,
}

#[repr(C)]
pub struct AVCodecContext {
    pub av_class: *const AVClass,
    pub log_level_offset: c_int,
    pub codec_type: AVMediaType,
    pub codec: *const AVCodec,
    pub codec_id: AVCodecID,
    pub codec_tag: c_uint,
    pub priv_data: *mut c_void,
    pub internal: *mut AVCodecInternal,
    pub opaque: *mut c_void,
    pub bit_rate: int64_t,
    pub flags: c_int,
    pub flags2: c_int,
    pub extradata: *mut uint8_t,
    pub extradata_size: c_int,
    pub time_base: AVRational,
    pub pkt_timebase: AVRational,
    pub framerate: AVRational,
    pub delay: c_int,
    pub width: c_int,
    pub height: c_int,
    pub coded_width: c_int,
    pub coded_height: c_int,
    pub sample_aspect_ratio: AVRational,
    pub pix_fmt: AVPixelFormat,
    pub sw_pix_fmt: AVPixelFormat,
    pub color_primaries: AVColorPrimaries,
    pub color_trc: AVColorTransferCharacteristic,
    pub colorspace: AVColorSpace,
    pub color_range: AVColorRange,
    pub chroma_sample_location: AVChromaLocation,
    pub sample_rate: c_int,
    pub channels: c_int,
    pub sample_fmt: AVSampleFormat,
    pub frame_size: c_int,
    pub ch_layout: AVChannelLayout,
    pub refcounted_frames: c_int,
    pub has_b_frames: c_int,
    pub block_align: c_int,
    pub bits_per_coded_sample: c_int,
    pub bits_per_raw_sample: c_int,
    pub slice_count: c_int,
    pub sample_aspect_ratio_2: AVRational,
    pub skip_top: c_int,
    pub skip_bottom: c_int,
    pub slices: c_int,
    pub field_order: AVFieldOrder,
    pub idct_algo: c_int,
    pub bits_per_coded_sample_2: c_int,
    pub thread_count: c_int,
    pub thread_type: c_int,
    pub active_thread_type: c_int,
    pub execute: *mut c_void,
    pub execute2: *mut c_void,
    pub nsse_weight: c_int,
    pub profile: c_int,
    pub level: c_int,
    pub lowres: c_int,
    pub idct_method: c_int,
    pub colorspace_2: c_int,
    pub color_range_2: c_int,
    pub chroma_sample_location_2: c_int,
    pub pkt_timebase_2: AVRational,
    pub sub_charenc: *mut c_char,
    pub sub_charenc_mode: c_int,
    pub skip_alpha: c_int,
    pub seek_preroll: c_int,
    pub debug: c_int,
    pub debug_mv: c_int,
    pub workaround_bugs: c_int,
    pub strict_std_compliance: c_int,
    pub error_concealment: c_int,
    pub debug_capabilities: c_int,
    pub err_recognition: c_int,
    pub reordered_opaque: int64_t,
    pub hwaccel: *const c_void,
    pub hwaccel_context: *mut c_void,
    pub hw_frames_ctx: *mut AVBufferRef,
    pub hw_device_ctx: *mut AVBufferRef,
    pub hwaccel_flags: c_int,
    pub extra_hw_frames: c_int,
    pub discard_damaged_percentage: c_int,
    pub export_side_data: c_int,
    pub get_encode_buffer: *mut c_void,
    pub encode: *mut c_void,
    pub encode2: *mut c_void,
    pub encoded_frame_count: int64_t,
    pub decoded_frame_count: int64_t,
    pub time_base_den: c_int,
    pub time_base_num: c_int,
    pub stats_in: *mut c_void,
    pub stats_out: *mut c_void,
    pub rc_override: *mut c_void,
    pub rc_eq: *mut c_char,
    pub rc_max_rate: int64_t,
    pub rc_min_rate: int64_t,
    pub rc_buffer_size: c_int,
    pub rc_override_count: c_int,
    pub rc_initial_cplx: c_double,
    pub rc_max_available_vbv_use: c_double,
    pub rc_min_vbv_overflow_use: c_double,
    pub rc_initial_buffer_occupancy: c_int,
    pub global_quality: c_int,
    pub flags3: c_int,
    pub bits_per_raw_sample_2: c_int,
    pub thread_safe_callbacks: c_int,
    pub qscale: c_int,
    pub qmin: c_int,
    pub qmax: c_int,
    pub me_method: c_int,
    pub me_cmp: c_int,
    pub me_cmp_func: c_int,
    pub me_range: c_int,
    pub me_threshold: c_int,
    pub pre_me: c_int,
    pub pre_me_cmp: c_int,
    pub pre_me_cmp_func: c_int,
    pub me_subpel_quality: c_int,
    pub bidir_refine: c_int,
    pub me_range_2: c_int,
    pub scenechange_threshold: c_int,
    pub lmin: c_int,
    pub lmax: c_int,
    pub noise_reduction: c_int,
    pub rc_initial_buffer_occupancy_2: c_int,
    pub frame_skip_threshold: c_int,
    pub frame_skip_factor: c_int,
    pub frame_skip_exp: c_int,
    pub frame_skip_cmp: c_int,
    pub frame_skip_cmp_func: c_int,
    pub b_frame_strategy: c_int,
    pub b_sensitivity: c_int,
    pub brd_scale: c_int,
    pub brd_frame: c_int,
    pub min_prediction_order: c_int,
    pub max_prediction_order: c_int,
    pub pns_reduction: c_int,
    pub prediction_method: c_int,
    pub sample_rate_2: c_int,
    pub predictor: c_int,
    pub ac_pred: c_int,
    pub min_partition_order: c_int,
    pub max_partition_order: c_int,
    pub dia_size: c_int,
    pub last_predictor_count: c_int,
    pub pre_dia_size: c_int,
    pub trellis: c_int,
    pub min_luma_hits: c_int,
    pub mb_lmin: c_int,
    pub mb_lmax: c_int,
    pub me_penalty_compensation: c_int,
    pub bidir_refine_2: c_int,
    pub scenechange_factor: c_int,
    pub lmin_2: c_int,
    pub lmax_2: c_int,
    pub brd_scale_2: c_int,
    pub brd_frame_2: c_int,
    pub qcompress: c_double,
    pub qblur: c_double,
    pub complexity_blur: c_double,
    pub b_quant_factor: c_double,
    pub b_quant_offset: c_double,
    pub i_quant_factor: c_double,
    pub i_quant_offset: c_double,
    pub b_quant_factor_2: c_double,
    pub b_quant_offset_2: c_double,
    pub i_quant_factor_2: c_double,
    pub i_quant_offset_2: c_double,
    pub luma_elim_threshold: c_int,
    pub chroma_elim_threshold: c_int,
    pub strict_std_compliance_2: c_int,
    pub noise_reduction_2: c_int,
    pub huffman: c_int,
    pub aura: c_int,
    pub level_2: c_int,
    pub profile_2: c_int,
    pub cavlc: c_int,
    pub b_sensitivity_2: c_int,
    pub compression_level: c_int,
    pub min_prediction_order_2: c_int,
    pub max_prediction_order_2: c_int,
    pub prediction_method_2: c_int,
    pub resample: c_int,
    pub sub_text: *mut c_char,
    pub sws_flags: c_int,
    pub alpha_blend: c_int,
    pub packet_loss: c_int,
    pub frame_bits: c_int,
    pub frame_packets: c_int,
    pub alpha: c_int,
    pub apply_crop: c_int,
    pub flags4: int64_t,
    pub chroma_intra_matrix: *mut uint16_t,
    pub dump_separator: *mut c_char,
    pub od_side_data: *mut c_void,
    pub video_stats: *mut c_void,
    pub is_sum_bytes: c_int,
    pub dst_extradata_size: c_int,
    pub dst_extradata: *mut uint8_t,
    pub audio_resample: c_int,
    pub audio_sample_rate: c_int,
    pub audio_ch_layout: AVChannelLayout,
    pub audio_channels: c_int,
    pub audio_sample_fmt: AVSampleFormat,
    pub pkt_timebase_3: AVRational,
    pub bits_per_raw_sample_3: c_int,
    pub pkt_timebase_4: AVRational,
    pub audio_timebase: AVRational,
    pub swr_context: *mut c_void,
    pub swr_i_context: *mut c_void,
    pub qpmax: c_int,
    pub audio_resample_2: c_int,
    pub side_data: *mut *mut AVPacketSideData,
    pub nb_side_data: c_int,
    pub codec_text_subtitle_rect: *const AVClass,
    pub subtitle_header_size: c_int,
    pub subtitle_header: *mut uint8_t,
    pub vaapi_context: *mut c_void,
    pub vaapi_context_2: *mut c_void,
    pub qsv_context: *mut c_void,
    pub vdpau_context: *mut c_void,
    pub vaapi_context_3: *mut c_void,
    pub vaapi_context_4: *mut c_void,
    pub vdpau_context_2: *mut c_void,
    pub cuda_context: *mut c_void,
    pub hwaccel_context_2: *mut c_void,
    pub hw_frames_ctx_2: *mut AVBufferRef,
    pub hw_device_ctx_2: *mut AVBufferRef,
    pub codec_text_subtitle_rect_2: *const AVClass,
    pub codec_descriptor: *const AVCodecDescriptor,
    pub log_flags: c_int,
    pub extradata_size_2: c_int,
    pub extradata_2: *mut uint8_t,
    pub seek_preroll_2: c_int,
    pub frame_packets_2: c_int,
    pub subtitle_header_size_2: c_int,
    pub subtitle_header_2: *mut uint8_t,
    pub dump_separator_2: *mut c_char,
    pub dump_separator_3: *mut c_char,
    pub opaque_ref: *mut AVBufferRef,
    pub priv_data_opaque: *mut c_void,
    pub skip_samples: c_int,
    pub audio_pad: c_int,
    pub max_samples: c_int,
    pub pkt_timebase_5: AVRational,
    pub frame_packets_3: c_int,
    pub frame_bits_2: c_int,
    pub priv_data_opaque_2: *mut c_void,
    pub pkt_timebase_6: AVRational,
    pub pkt_timebase_7: AVRational,
}

#[repr(C)]
pub struct AVCodecInternal {
    _private: [u8; 0],
}

#[repr(C)]
pub struct AVCodecDescriptor {
    pub id: AVCodecID,
    pub kind: AVMediaType,
    pub name: *const c_char,
    pub long_name: *const c_char,
    pub props: c_int,
    pub mime_types: *const *const c_char,
    pub profiles: *const AVProfile,
}

/// Describes the properties of an encoded stream; allocated/freed via
/// `avcodec_parameters_alloc` / `avcodec_parameters_free`.
#[repr(C)]
pub struct AVCodecParameters {
    pub codec_type: AVMediaType,
    pub codec_id: AVCodecID,
    pub codec_tag: uint32_t,
    pub extradata: *mut uint8_t,
    pub extradata_size: c_int,
    pub coded_side_data: *mut AVPacketSideData,
    pub nb_coded_side_data: c_int,
    pub format: c_int,
    pub bit_rate: int64_t,
    pub bits_per_coded_sample: c_int,
    pub bits_per_raw_sample: c_int,
    pub profile: c_int,
    pub level: c_int,
    pub width: c_int,
    pub height: c_int,
    pub sample_aspect_ratio: AVRational,
    pub framerate: AVRational,
    pub field_order: AVFieldOrder,
    pub color_range: AVColorRange,
    pub color_primaries: AVColorPrimaries,
    pub color_trc: AVColorTransferCharacteristic,
    pub color_space: AVColorSpace,
    pub chroma_location: AVChromaLocation,
    pub video_delay: c_int,
    pub ch_layout: AVChannelLayout,
    pub sample_rate: c_int,
    pub block_align: c_int,
    pub frame_size: c_int,
    pub initial_padding: c_int,
    pub trailing_padding: c_int,
    pub seek_preroll: c_int,
}

#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum AVFieldOrder {
    AV_FIELD_UNKNOWN,
    AV_FIELD_PROGRESSIVE,
    AV_FIELD_TT,
    AV_FIELD_BB,
    AV_FIELD_TB,
    AV_FIELD_BT,
}

#[repr(C)]
pub struct AVPacketSideData {
    pub data: *mut uint8_t,
    pub size: size_t,
    pub kind: AVPacketSideDataType,
}

pub type AVPacketSideDataType = c_int;

pub const AV_PKT_DATA_PALETTE: AVPacketSideDataType = 0;
pub const AV_PKT_DATA_NEW_EXTRADATA: AVPacketSideDataType = 1;
pub const AV_PKT_DATA_PARAM_CHANGE: AVPacketSideDataType = 2;
pub const AV_PKT_DATA_H263_MB_INFO: AVPacketSideDataType = 3;
pub const AV_PKT_DATA_REPLAYGAIN: AVPacketSideDataType = 4;
pub const AV_PKT_DATA_DISPLAYMATRIX: AVPacketSideDataType = 5;
pub const AV_PKT_DATA_STEREO3D: AVPacketSideDataType = 6;
pub const AV_PKT_DATA_AUDIO_SERVICE_TYPE: AVPacketSideDataType = 7;
pub const AV_PKT_DATA_QUALITY_STATS: AVPacketSideDataType = 8;
pub const AV_PKT_DATA_FALLBACK_TRACK: AVPacketSideDataType = 9;
pub const AV_PKT_DATA_CPB_PROPERTIES: AVPacketSideDataType = 10;
pub const AV_PKT_DATA_SKIP_SAMPLES: AVPacketSideDataType = 11;
pub const AV_PKT_DATA_JP_DUALMONO: AVPacketSideDataType = 12;
pub const AV_PKT_DATA_STRINGS_METADATA: AVPacketSideDataType = 13;
pub const AV_PKT_DATA_SUBTITLE_POSITION: AVPacketSideDataType = 14;
pub const AV_PKT_DATA_MATROSKA_BLOCKADDITIONAL: AVPacketSideDataType = 15;
pub const AV_PKT_DATA_WEBVTT_IDENTIFIER: AVPacketSideDataType = 16;
pub const AV_PKT_DATA_WEBVTT_SETTINGS: AVPacketSideDataType = 17;
pub const AV_PKT_DATA_METADATA_UPDATE: AVPacketSideDataType = 18;
pub const AV_PKT_DATA_MPEGTS_STREAM_ID: AVPacketSideDataType = 19;
pub const AV_PKT_DATA_MASTERING_DISPLAY_METADATA: AVPacketSideDataType = 20;
pub const AV_PKT_DATA_SPHERICAL: AVPacketSideDataType = 21;
pub const AV_PKT_DATA_CONTENT_LIGHT_LEVEL: AVPacketSideDataType = 22;
pub const AV_PKT_DATA_A53_CC: AVPacketSideDataType = 23;
pub const AV_PKT_DATA_ENCRYPTION_INIT_INFO: AVPacketSideDataType = 24;
pub const AV_PKT_DATA_ENCRYPTION_INFO: AVPacketSideDataType = 25;
pub const AV_PKT_DATA_AFD: AVPacketSideDataType = 26;
pub const AV_PKT_DATA_PRFT: AVPacketSideDataType = 27;
pub const AV_PKT_DATA_ICC_PROFILE: AVPacketSideDataType = 28;
pub const AV_PKT_DATA_DOVI_CONF: AVPacketSideDataType = 29;
pub const AV_PKT_DATA_S12M_TIMECODE: AVPacketSideDataType = 30;
pub const AV_PKT_DATA_DYNAMIC_HDR10_PLUS: AVPacketSideDataType = 31;
pub const AV_PKT_DATA_AMBIENT_VIEWING_ENVIRONMENT: AVPacketSideDataType = 32;
pub const AV_PKT_DATA_FRAME_CROPPING: AVPacketSideDataType = 33;
pub const AV_PKT_DATA_LCEVC: AVPacketSideDataType = 34;
pub const AV_PKT_DATA_RTCP_SR: AVPacketSideDataType = 35;
pub const AV_PKT_DATA_NB: AVPacketSideDataType = 36;

pub const AV_PKT_FLAG_KEY: c_int = 0x0001;
pub const AV_PKT_FLAG_CORRUPT: c_int = 0x0002;
pub const AV_PKT_FLAG_DISCARD: c_int = 0x0004;
pub const AV_PKT_FLAG_TRUSTED: c_int = 0x0008;
pub const AV_PKT_FLAG_DISPOSABLE: c_int = 0x0010;

/// Compressed data packet; allocated/freed via `av_packet_alloc`/`av_packet_free`.
#[repr(C)]
pub struct AVPacket {
    pub buf: *mut AVBufferRef,
    pub pts: int64_t,
    pub dts: int64_t,
    pub data: *mut uint8_t,
    pub size: c_int,
    pub stream_index: c_int,
    pub flags: c_int,
    pub side_data: *mut AVPacketSideData,
    pub side_data_elems: c_int,
    pub duration: int64_t,
    pub pos: int64_t,
    pub opaque: *mut c_void,
    pub opaque_ref: *mut AVBufferRef,
    pub time_base: AVRational,
}

/// Reference-counted decoded (raw) audio or video frame.
#[repr(C)]
pub struct AVFrame {
    pub data: [*mut uint8_t; AV_NUM_DATA_POINTERS],
    pub linesize: [c_int; AV_NUM_DATA_POINTERS],
    pub extended_data: *mut *mut uint8_t,
    pub width: c_int,
    pub height: c_int,
    pub nb_samples: c_int,
    pub format: c_int,
    pub pict_type: AVPictureType,
    pub sample_aspect_ratio: AVRational,
    pub pts: int64_t,
    pub pkt_dts: int64_t,
    pub time_base: AVRational,
    pub quality: c_int,
    pub opaque: *mut c_void,
    pub repeat_pict: c_int,
    pub sample_rate: c_int,
    pub buf: [*mut AVBufferRef; AV_NUM_DATA_POINTERS],
    pub extended_buf: *mut *mut AVBufferRef,
    pub nb_extended_buf: c_int,
    pub side_data: *mut *mut AVFrameSideData,
    pub nb_side_data: c_int,
    pub flags: c_int,
    pub color_range: AVColorRange,
    pub color_primaries: AVColorPrimaries,
    pub color_trc: AVColorTransferCharacteristic,
    pub colorspace: AVColorSpace,
    pub chroma_location: AVChromaLocation,
    pub best_effort_timestamp: int64_t,
    pub metadata: *mut AVDictionary,
    pub decode_error_flags: c_int,
    pub hw_frames_ctx: *mut AVBufferRef,
    pub opaque_ref: *mut AVBufferRef,
    pub crop_top: size_t,
    pub crop_bottom: size_t,
    pub crop_left: size_t,
    pub crop_right: size_t,
    pub private_ref: *mut c_void,
    pub ch_layout: AVChannelLayout,
    pub duration: int64_t,
}

pub const AV_FRAME_FLAG_CORRUPT: c_int = 1 << 0;
pub const AV_FRAME_FLAG_KEY: c_int = 1 << 1;
pub const AV_FRAME_FLAG_DISCARD: c_int = 1 << 2;
pub const AV_FRAME_FLAG_INTERLACED: c_int = 1 << 3;
pub const AV_FRAME_FLAG_TOP_FIELD_FIRST: c_int = 1 << 4;
pub const AV_FRAME_FLAG_LOSSLESS: c_int = 1 << 5;

// =====================================================================
// libavformat — streams and container
// =====================================================================

#[repr(C)]
pub struct AVInputFormat {
    pub name: *const c_char,
    pub long_name: *const c_char,
    pub flags: c_int,
    pub extensions: *const c_char,
    pub codec_tag: *const *const AVCodecTag,
    pub priv_class: *const AVClass,
    pub next: *mut AVInputFormat,
    pub raw_codec_id: AVCodecID,
    pub priv_data_size: c_int,
    pub read_probe: *mut c_void,
    pub read_header: *mut c_void,
    pub read_packet: *mut c_void,
    pub read_seek: *mut c_void,
    pub read_timestamp: *mut c_void,
    pub read_close: *mut c_void,
    pub read_pause: *mut c_void,
    pub read_seek2: *mut c_void,
    pub get_device_list: *mut c_void,
}

#[repr(C)]
pub struct AVOutputFormat {
    pub name: *const c_char,
    pub long_name: *const c_char,
    pub mime_type: *const c_char,
    pub extensions: *const c_char,
    pub audio_codec: AVCodecID,
    pub video_codec: AVCodecID,
    pub subtitle_codec: AVCodecID,
    pub flags: c_int,
    pub codec_tag: *const *const AVCodecTag,
    pub priv_class: *const AVClass,
    pub next: *mut AVOutputFormat,
    pub priv_data_size: c_int,
    pub write_header: *mut c_void,
    pub write_packet: *mut c_void,
    pub write_trailer: *mut c_void,
    pub interleave_packet: *mut c_void,
    pub query_codec: *mut c_void,
    pub get_output_timestamp: *mut c_void,
    pub control_message: *mut c_void,
    pub write_uncoded_frame: *mut c_void,
    pub get_device_list: *mut c_void,
    pub init: *mut c_void,
    pub deinit: *mut c_void,
    pub check_bitstream: *mut c_void,
}

#[repr(C)]
pub struct AVCodecTag {
    pub id: AVCodecID,
    pub tag: c_uint,
}

#[repr(C)]
pub struct AVStreamGroup {
    _private: [u8; 0],
}

#[repr(C)]
pub struct AVChapter {
    pub id: int64_t,
    pub time_base: AVRational,
    pub start: int64_t,
    pub end: int64_t,
    pub metadata: *mut AVDictionary,
}

#[repr(C)]
pub struct AVProgram {
    pub id: c_int,
    pub flags: c_int,
    pub discard: AVDiscard,
    pub stream_index: *mut c_uint,
    pub nb_stream_indexes: c_uint,
    pub metadata: *mut AVDictionary,
    pub program_num: c_int,
    pub pmt_pid: c_int,
    pub pcr_pid: c_int,
    pub pmt_version: c_int,
    pub start_time: int64_t,
    pub end_time: int64_t,
    pub pts_wrap_reference: int64_t,
    pub pts_wrap_behavior: c_int,
}

#[repr(C)]
pub struct AVStream {
    pub av_class: *const AVClass,
    pub index: c_int,
    pub id: c_int,
    pub codecpar: *mut AVCodecParameters,
    pub priv_data: *mut c_void,
    pub time_base: AVRational,
    pub start_time: int64_t,
    pub duration: int64_t,
    pub nb_frames: int64_t,
    pub disposition: c_int,
    pub discard: AVDiscard,
    pub sample_aspect_ratio: AVRational,
    pub metadata: *mut AVDictionary,
    pub avg_frame_rate: AVRational,
    pub attached_pic: AVPacket,
    pub event_flags: c_int,
    pub r_frame_rate: AVRational,
    pub pts_wrap_bits: c_int,
}

pub const AVSTREAM_EVENT_FLAG_METADATA_UPDATED: c_int = 0x0001;
pub const AVSTREAM_EVENT_FLAG_NEW_PACKETS: c_int = 1 << 1;

/// I/O context used by [`AVFormatContext`].
#[repr(C)]
pub struct AVIOContext {
    pub av_class: *const AVClass,
    pub buffer: *mut c_void,
    pub buffer_size: c_int,
    pub buf_ptr: *mut uint8_t,
    pub buf_end: *mut uint8_t,
    pub opaque: *mut c_void,
    pub read_packet: Option<extern "C" fn(*mut c_void, *mut uint8_t, c_int) -> c_int>,
    pub write_packet:
        Option<extern "C" fn(*mut c_void, *const uint8_t, c_int) -> c_int>,
    pub seek: Option<extern "C" fn(*mut c_void, int64_t, c_int) -> int64_t>,
    pub pos: int64_t,
    pub eof_reached: c_int,
    pub error: c_int,
    pub write_flag: c_int,
    pub max_packet_size: c_int,
    pub min_packet_size: c_int,
    pub checksum: c_ulong,
    pub checksum_ptr: *mut c_uchar,
    pub update_checksum: Option<extern "C" fn(c_ulong, *const uint8_t, c_uint) -> c_ulong>,
    pub read_pause: Option<extern "C" fn(*mut c_void, c_int) -> c_int>,
    pub read_seek: Option<extern "C" fn(*mut c_void, c_int, int64_t, c_int) -> int64_t>,
    pub seekable: c_int,
    pub direct: c_int,
    pub protocol_whitelist: *const c_char,
    pub protocol_blacklist: *const c_char,
    pub write_data_type: Option<
        extern "C" fn(*mut c_void, *const uint8_t, c_int, AVIODataMarkerType, int64_t) -> c_int,
    >,
    pub ignore_boundary_point: c_int,
    pub buf_ptr_max: *mut c_uchar,
    pub bytes_read: int64_t,
    pub bytes_written: int64_t,
}

#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum AVIODataMarkerType {
    AVIO_DATA_MARKER_HEADER,
    AVIO_DATA_MARKER_SYNC_POINT,
    AVIO_DATA_MARKER_BOUNDARY_POINT,
    AVIO_DATA_MARKER_UNKNOWN,
    AVIO_DATA_MARKER_TRAILER,
    AVIO_DATA_MARKER_FLUSH_POINT,
}

pub const AVIO_SEEKABLE_NORMAL: c_int = 1 << 0;
pub const AVIO_SEEKABLE_TIME: c_int = 1 << 1;

#[repr(C)]
pub struct AVIOInterruptCB {
    pub callback: Option<extern "C" fn(*mut c_void) -> c_int>,
    pub opaque: *mut c_void,
}

/// Top-level container context used by all muxers/demuxers.
#[repr(C)]
pub struct AVFormatContext {
    pub av_class: *const AVClass,
    pub iformat: *const AVInputFormat,
    pub oformat: *const AVOutputFormat,
    pub priv_data: *mut c_void,
    pub pb: *mut AVIOContext,
    pub ctx_flags: c_int,
    pub nb_streams: c_uint,
    pub streams: *mut *mut AVStream,
    pub nb_stream_groups: c_uint,
    pub stream_groups: *mut *mut AVStreamGroup,
    pub nb_chapters: c_uint,
    pub chapters: *mut *mut AVChapter,
    pub url: *mut c_char,
    pub start_time: int64_t,
    pub duration: int64_t,
    pub bit_rate: int64_t,
    pub packet_size: c_uint,
    pub max_delay: c_int,
    pub flags: c_int,
    pub probesize: int64_t,
    pub max_analyze_duration: int64_t,
    pub keylen: c_uint,
    pub key: *const uint8_t,
    pub nb_programs: c_uint,
    pub programs: *mut *mut AVProgram,
    pub video_codec_id: AVCodecID,
    pub audio_codec_id: AVCodecID,
    pub subtitle_codec_id: AVCodecID,
    pub data_codec_id: AVCodecID,
    pub metadata: *mut AVDictionary,
    pub start_time_realtime: int64_t,
    pub fps_probe_size: c_int,
    pub error_recognition: c_int,
    pub interrupt_callback: AVIOInterruptCB,
    pub debug: c_int,
    pub max_streams: c_uint,
    pub skip_estimate_duration_from_pts: c_int,
    pub probe_score: c_int,
    pub format_probesize: c_int,
    pub format_opts: *mut AVDictionary,
    pub duration_estimation_method: c_int,
    pub audio_preload: c_int,
    pub max_chunk_size: c_int,
    pub max_chunk_time: c_int,
    pub strict_std_compliance: c_int,
    pub event_flags: c_int,
    pub max_interleave_delta: c_int,
    pub strict_timestamping: c_int,
    pub flags_2: AVFormatFlags,
}

pub type AVFormatFlags = c_int;
pub const AVFMT_NOFILE: c_int = 0x0001;
pub const AVFMT_NEEDNUMBER: c_int = 0x0002;
pub const AVFMT_SHOW_IDS: c_int = 0x0008;
pub const AVFMT_GLOBALHEADER: c_int = 0x0040;
pub const AVFMT_NOTIMESTAMPS: c_int = 0x0080;
pub const AVFMT_GENERIC_INDEX: c_int = 0x0100;
pub const AVFMT_TS_DISCONT: c_int = 0x0200;
pub const AVFMT_VARIABLE_FPS: c_int = 0x0400;
pub const AVFMT_NODIMENSIONS: c_int = 0x0800;
pub const AVFMT_NOSTREAMS: c_int = 0x1000;
pub const AVFMT_NOBINSEARCH: c_int = 0x2000;
pub const AVFMT_NOGENSEARCH: c_int = 0x4000;
pub const AVFMT_NO_BYTE_SEEK: c_int = 0x8000;
pub const AVFMT_SEEK_TO_PTS: c_int = 0x4000000;
pub const AVFMT_TS_NONSTRICT: c_int = 0x20000;
pub const AVFMT_TS_NEGATIVE: c_int = 0x40000;

// =====================================================================
// libswscale — SwsContext
// =====================================================================

#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SwsDither {
    SWS_DITHER_NONE = 0,
    SWS_DITHER_AUTO,
    SWS_DITHER_BAYER,
    SWS_DITHER_ED,
    SWS_DITHER_A_DITHER,
    SWS_DITHER_X_DITHER,
    SWS_DITHER_NB,
}

#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SwsAlphaBlend {
    SWS_ALPHA_BLEND_NONE = 0,
    SWS_ALPHA_BLEND_UNIFORM,
    SWS_ALPHA_BLEND_CHECKERBOARD,
    SWS_ALPHA_BLEND_NB,
}

pub type SwsFlags = c_int;
pub const SWS_FAST_BILINEAR: c_int = 1 << 0;
pub const SWS_BILINEAR: c_int = 1 << 1;
pub const SWS_BICUBIC: c_int = 1 << 2;
pub const SWS_X: c_int = 1 << 3;
pub const SWS_POINT: c_int = 1 << 4;
pub const SWS_AREA: c_int = 1 << 5;
pub const SWS_BICUBLIN: c_int = 1 << 6;
pub const SWS_GAUSS: c_int = 1 << 7;
pub const SWS_SINC: c_int = 1 << 8;
pub const SWS_LANCZOS: c_int = 1 << 9;
pub const SWS_SPLINE: c_int = 1 << 10;

pub type SwsFilter = c_void;
pub type SwsVector = c_void;

/// Colour-conversion / scaling context.
#[repr(C)]
pub struct SwsContext {
    pub av_class: *const AVClass,
    pub opaque: *mut c_void,
    pub flags: c_uint,
    pub scaler_params: [c_double; 2],
    pub threads: c_int,
    pub dither: SwsDither,
    pub alpha_blend: SwsAlphaBlend,
    pub gamma_flag: c_int,
    pub src_w: c_int,
    pub src_h: c_int,
    pub dst_w: c_int,
    pub dst_h: c_int,
    pub src_format: c_int,
    pub dst_format: c_int,
    pub src_range: c_int,
    pub dst_range: c_int,
    pub src_v_chr_pos: c_int,
    pub src_h_chr_pos: c_int,
    pub dst_v_chr_pos: c_int,
    pub dst_h_chr_pos: c_int,
    pub intent: c_int,
}

// =====================================================================
// libavfilter — types referenced by the surface
// =====================================================================

/// Forward declaration for the filter graph.
#[repr(C)]
pub struct AVFilterGraph {
    _private: [u8; 0],
}

#[repr(C)]
pub struct AVFilterContext {
    _private: [u8; 0],
}

#[repr(C)]
pub struct AVFilter {
    pub name: *const c_char,
    pub description: *const c_char,
    pub inputs: *const AVFilterPad,
    pub outputs: *const AVFilterPad,
    pub priv_class: *const AVClass,
    pub flags: c_int,
    pub init: *mut c_void,
    pub uninit: *mut c_void,
    pub query_formats: *mut c_void,
    pub process_command: *mut c_void,
    pub activate: *mut c_void,
    pub next: *mut AVFilter,
}

#[repr(C)]
pub struct AVFilterPad {
    pub name: *const c_char,
    pub kind: AVMediaType,
    pub type_flags: c_int,
    pub min_perms: c_int,
    pub rej_perms: c_int,
    pub start_frame: *mut c_void,
    pub get_video_buffer: *mut c_void,
    pub get_audio_buffer: *mut c_void,
    pub end_frame: *mut c_void,
    pub draw_slice: *mut c_void,
    pub filter_frame: *mut c_void,
    pub request_frame: *mut c_void,
    pub config_props: *mut c_void,
}

#[repr(C)]
pub struct AVFilterLink {
    pub src: *mut AVFilterContext,
    pub dst: *mut AVFilterContext,
    pub srcpad: *const AVFilterPad,
    pub dstpad: *const AVFilterPad,
    pub kind: AVMediaType,
    pub w: c_int,
    pub h: c_int,
    pub sample_aspect_ratio: AVRational,
    pub format: c_int,
    pub time_base: AVRational,
    pub channel_layout: AVChannelLayout,
    pub sample_rate: c_int,
    pub incfg: *mut c_void,
    pub outcfg: *mut c_void,
    pub init_state: c_int,
    pub graph: *mut AVFilterGraph,
    pub current_pts: int64_t,
    pub age_index: c_int,
    pub frame_rate: AVRational,
    pub frame_count_in: int64_t,
    pub frame_count_out: int64_t,
    pub sample_count_in: int64_t,
    pub sample_count_out: int64_t,
    pub fifo: *mut c_void,
    pub status_in: c_int,
    pub status_out: c_int,
}

// =====================================================================
// libavutil — error codes
// =====================================================================

pub const AVERROR_BSF_NOT_FOUND: c_int = -0xF8_63D9;
pub const AVERROR_BUG: c_int = -0xF8_8F3E;
pub const AVERROR_BUFFER_TOO_SMALL: c_int = -0xF7_39D7;
pub const AVERROR_DECODER_NOT_FOUND: c_int = -0xF8_3FB9;
pub const AVERROR_DEMUXER_NOT_FOUND: c_int = -0xF8_63CA;
pub const AVERROR_ENCODER_NOT_FOUND: c_int = -0xF8_3FB8;
pub const AVERROR_EOF: c_int = -0x54_1241;
pub const AVERROR_EXIT: c_int = -0x40_32B6;
pub const AVERROR_EXTERNAL: c_int = -0xBB_9BBA;
pub const AVERROR_FILTER_NOT_FOUND: c_int = -0xF8_63D8;
pub const AVERROR_INVALID_DATA: c_int = -0xFE_9BCB;
pub const AVERROR_MUXER_NOT_FOUND: c_int = -0xF8_63C9;
pub const AVERROR_OPTION_NOT_FOUND: c_int = -0xF8_5FB7;
pub const AVERROR_PATCHWELCOME: c_int = -0xFE_9BC0;
pub const AVERROR_PROTOCOL_NOT_FOUND: c_int = -0xF8_3FAF;
pub const AVERROR_STREAM_NOT_FOUND: c_int = -0xF8_3FB7;
pub const AVERROR_UNKNOWN: c_int = -0xF7_39B9;
pub const AVERROR_EAGAIN: c_int = -11;

#[inline]
pub fn AVERROR(e: c_int) -> c_int {
    -e
}

#[inline]
pub fn AVUNERROR(e: c_int) -> c_int {
    -e
}

// =====================================================================
// FFI — function declarations
// =====================================================================
//
// Every extern block below mirrors the C signatures as they appear in the
// upstream FFmpeg headers. Marked `unsafe extern "C"` because they are FFI
// and the caller must uphold the documented preconditions.

extern "C" {
    // ---- libavcodec ----
    pub fn avcodec_register_all();
    pub fn avcodec_find_decoder(id: AVCodecID) -> *const AVCodec;
    pub fn avcodec_alloc_context3(codec: *const AVCodec) -> *mut AVCodecContext;
    pub fn avcodec_open2(
        avctx: *mut AVCodecContext,
        codec: *const AVCodec,
        options: *mut *mut AVDictionary,
    ) -> c_int;
    pub fn avcodec_send_packet(avctx: *mut AVCodecContext, avpkt: *const AVPacket) -> c_int;
    pub fn avcodec_receive_frame(avctx: *mut AVCodecContext, frame: *mut AVFrame) -> c_int;
    pub fn avcodec_free_context(avctx: *mut *mut AVCodecContext);

    // ---- libavutil — frame / packet alloc ----
    pub fn av_frame_alloc() -> *mut AVFrame;
    pub fn av_frame_free(frame: *mut *mut AVFrame);
    pub fn av_packet_alloc() -> *mut AVPacket;
    pub fn av_packet_free(pkt: *mut *mut AVPacket);

    // ---- libavformat ----
    pub fn avformat_open_input(
        ps: *mut *mut AVFormatContext,
        url: *const c_char,
        fmt: *const AVInputFormat,
        options: *mut *mut AVDictionary,
    ) -> c_int;
    pub fn avformat_find_stream_info(ic: *mut AVFormatContext, options: *mut *mut AVDictionary)
        -> c_int;
    pub fn av_read_frame(s: *mut AVFormatContext, pkt: *mut AVPacket) -> c_int;
    pub fn avformat_close_input(s: *mut *mut AVFormatContext);
    pub fn avformat_free_context(s: *mut AVFormatContext);

    // ---- libavutil — image utilities ----
    pub fn av_image_alloc(
        pointers: *mut *mut uint8_t,
        linesizes: *mut c_int,
        w: c_int,
        h: c_int,
        pix_fmt: AVPixelFormat,
        align: c_int,
    ) -> c_int;
    pub fn av_image_fill_arrays(
        dst_data: *mut *mut uint8_t,
        dst_linesize: *mut c_int,
        src: *const uint8_t,
        pix_fmt: AVPixelFormat,
        width: c_int,
        height: c_int,
        align: c_int,
    ) -> c_int;

    // ---- libswscale ----
    pub fn sws_getContext(
        srcW: c_int,
        srcH: c_int,
        srcFormat: AVPixelFormat,
        dstW: c_int,
        dstH: c_int,
        dstFormat: AVPixelFormat,
        flags: c_int,
        srcFilter: *mut SwsFilter,
        dstFilter: *mut SwsFilter,
        param: *const c_double,
    ) -> *mut SwsContext;
    pub fn sws_scale(
        c: *mut SwsContext,
        srcSlice: *const *const uint8_t,
        srcStride: *const c_int,
        srcSliceY: c_int,
        srcSliceH: c_int,
        dst: *const *mut uint8_t,
        dstStride: *const c_int,
    ) -> c_int;
    pub fn sws_free_context(ctx: *mut *mut SwsContext);

    // ---- libavutil — pixel format descriptor ----
    pub fn av_pix_fmt_desc_get(pix_fmt: AVPixelFormat) -> *const AVPixFmtDescriptor;

    // ---- libavutil — dictionary (commonly used by callers) ----
    pub fn av_dict_set(
        pm: *mut *mut AVDictionary,
        key: *const c_char,
        value: *const c_char,
        flags: c_int,
    ) -> c_int;
    pub fn av_dict_free(m: *mut *mut AVDictionary);
    pub fn av_dict_count(m: *const AVDictionary) -> c_int;
}

// =====================================================================
// Safe convenience wrappers
// =====================================================================
//
// These thin wrappers translate the raw `*mut T` returns into `Option<NonNull<T>>`
// to make safe Rust code less error-prone. They are entirely optional — callers
// that need the raw pointers for FFI interop may use the extern declarations
// above directly.

#[inline]
pub unsafe fn sws_get_context_safe(
    src_w: c_int,
    src_h: c_int,
    src_fmt: AVPixelFormat,
    dst_w: c_int,
    dst_h: c_int,
    dst_fmt: AVPixelFormat,
    flags: c_int,
) -> *mut SwsContext {
    sws_getContext(
        src_w,
        src_h,
        src_fmt,
        dst_w,
        dst_h,
        dst_fmt,
        flags,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        std::ptr::null(),
    )
}

#[inline]
pub fn codec_kind_video() -> AVMediaType {
    AVMediaType::AVMEDIA_TYPE_VIDEO
}

#[inline]
pub fn codec_kind_audio() -> AVMediaType {
    AVMediaType::AVMEDIA_TYPE_AUDIO
}

// =====================================================================
// Global state used by PCSX2's bridge layer.
//
// PCSX2 caches a small set of FFmpeg decoder descriptors once at startup so
// that hot paths do not need to call `avcodec_find_decoder` for every
// stream. The pointers here are populated by the C bridge and read by the
// Rust callers; both sides treat them as immutable for the lifetime of the
// process.
// =====================================================================

/// Cached handle to the H.264 decoder, populated by the bridge.
pub static mut GLOBAL_H264_CODEC: *const AVCodec = std::ptr::null();
/// Cached handle to the HEVC decoder.
pub static mut GLOBAL_HEVC_CODEC: *const AVCodec = std::ptr::null();
/// Cached handle to the MPEG-2 video decoder.
pub static mut GLOBAL_MPEG2_CODEC: *const AVCodec = std::ptr::null();
/// Cached handle to the AAC audio decoder.
pub static mut GLOBAL_AAC_CODEC: *const AVCodec = std::ptr::null();
/// Cached handle to the MP3 audio decoder.
pub static mut GLOBAL_MP3_CODEC: *const AVCodec = std::ptr::null();
/// Cached handle to the AV1 decoder (used by the PS2 HW-path replacements).
pub static mut GLOBAL_AV1_CODEC: *const AVCodec = std::ptr::null();

/// Tracks whether [`avcodec_register_all`] has been invoked. PCSX2 calls it
/// exactly once during process startup so that the decoder descriptors are
/// available for the cached lookups above.
pub static mut GLOBAL_CODECS_REGISTERED: bool = false;
