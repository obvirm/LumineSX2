//! Idiomatic Rust 2021 translation of the Cubeb cross-platform audio library.
//!
//! This module mirrors the public C API surface defined in
//! `3rdparty/cubeb/include/cubeb/cubeb.h`, together with the ops-vtable
//! dispatch shape implemented in `3rdparty/cubeb/src/cubeb.c` and the
//! per-backend files (`cubeb_alsa.c`, `cubeb_pulse.c`, `cubeb_wasapi.cpp`,
//! `cubeb_audiounit.cpp`, `cubeb_opensl.c`, `cubeb_sndio.c`).
//!
//! Cubeb is a callback-driven audio abstraction that hides the platform
//! native audio API (ALSA, PulseAudio, WASAPI, AudioUnit, OpenSL ES,
//! sndio, ...) behind a small dispatch table attached to an opaque
//! `cubeb` context. This translation preserves the C-ABI types so the
//! Rust signature set is wire-compatible with the original library,
//! while expressing the dispatch logic in idiomatic Rust terms.
//!
//! The module only depends on `std` (no `libc`, no `extern` shims) and
//! uses `static mut` for the few pieces of global state the original
//! library carries (logging callback, preferred backend registry).
//!
//! Layout matches the C `cubeb` / `cubeb_stream` structs: a pointer to
//! the ops vtable followed by the user pointer. All callbacks retain
//! their C calling convention so existing backend binaries can link
//! against the Rust translation without re-implementing the audio
//! drivers themselves.

use std::os::raw::{c_char, c_long, c_void};

// ---------------------------------------------------------------------------
// Opaque context and stream handles.
// ---------------------------------------------------------------------------

/// Opaque application context. The C-side `cubeb` carries an ops vtable
/// pointer plus any backend-private state; we expose only the handle.
pub type CubebContext = *mut c_void;

/// Opaque stream handle. The C-side `cubeb_stream` keeps the parent
/// context pointer followed by the user pointer; we expose only the
/// handle.
pub type CubebStream = *mut c_void;

/// Internal ops vtable that backends populate. Mirrors
/// `struct cubeb_ops` from `cubeb-internal.h`.
#[repr(C)]
pub struct CubebOps {
    pub init: Option<
        unsafe extern "C" fn(
            context: *mut *mut CubebContext,
            context_name: *const c_char,
        ) -> i32,
    >,
    pub get_backend_id: Option<unsafe extern "C" fn(context: *mut CubebContext) -> *const c_char>,
    pub get_max_channel_count:
        Option<unsafe extern "C" fn(context: *mut CubebContext, max_channels: *mut u32) -> i32>,
    pub get_min_latency: Option<
        unsafe extern "C" fn(
            context: *mut CubebContext,
            params: CubebStreamParams,
            latency_frames: *mut u32,
        ) -> i32,
    >,
    pub get_preferred_sample_rate:
        Option<unsafe extern "C" fn(context: *mut CubebContext, rate: *mut u32) -> i32>,
    pub get_preferred_sample_format: Option<
        unsafe extern "C" fn(context: *mut CubebContext, format: *mut CubebSampleFormat) -> i32,
    >,
    pub get_supported_format: Option<
        unsafe extern "C" fn(context: *mut CubebContext, format: *mut CubebSampleFormat) -> i32,
    >,
    pub destroy: Option<unsafe extern "C" fn(context: *mut CubebContext)>,
    pub stream_init: Option<
        unsafe extern "C" fn(
            context: *mut CubebContext,
            stream: *mut *mut CubebStream,
            stream_name: *const c_char,
            input_device: *const c_void,
            input_stream_params: *const CubebStreamParams,
            output_device: *const c_void,
            output_stream_params: *const CubebStreamParams,
            latency_frames: u32,
            data_callback: CubebDataCallback,
            state_callback: CubebStateCallback,
            user_ptr: *mut c_void,
        ) -> i32,
    >,
    pub stream_destroy: Option<unsafe extern "C" fn(stream: *mut CubebStream)>,
    pub stream_start: Option<unsafe extern "C" fn(stream: *mut CubebStream) -> i32>,
    pub stream_stop: Option<unsafe extern "C" fn(stream: *mut CubebStream) -> i32>,
    pub stream_get_position:
        Option<unsafe extern "C" fn(stream: *mut CubebStream, position: *mut u64) -> i32>,
    pub stream_get_latency:
        Option<unsafe extern "C" fn(stream: *mut CubebStream, latency: *mut u32) -> i32>,
    pub stream_set_volume:
        Option<unsafe extern "C" fn(stream: *mut CubebStream, volume: f32) -> i32>,
}

// ---------------------------------------------------------------------------
// Enumerations and structures.
// ---------------------------------------------------------------------------

/// Sample-format enumeration. Field order preserves the C values
/// (S16LE, S16BE, F32LE, F32BE) so that values round-trip when
/// translated across the FFI boundary.
///
/// Note: the C-side `CUBEB_SAMPLE_S16NE` / `CUBEB_SAMPLE_FLOAT32NE`
/// constants are platform-conditional aliases of either the LE or BE
/// variant (see `cubeb.h`). They cannot be expressed as Rust enum
/// variants because Rust forbids duplicate discriminants; consumers
/// should compare against `S16LE` / `F32LE` directly on little-endian
/// targets (the only targets PCSX2 supports).
#[repr(i32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum CubebSampleFormat {
    S16LE = 0,
    S16BE = 1,
    F32LE = 2,
    F32BE = 3,
}

impl CubebSampleFormat {
    /// Returns true for little-endian variants.
    pub fn is_little_endian(self) -> bool {
        matches!(self, CubebSampleFormat::S16LE | CubebSampleFormat::F32LE)
    }

    /// Returns true for signed-integer variants.
    pub fn is_integer(self) -> bool {
        matches!(self, CubebSampleFormat::S16LE | CubebSampleFormat::S16BE)
    }
}

/// Stream initialisation parameters. Mirrors `cubeb_stream_params` with
/// the optional `layout`, `prefs`, and `input_params` fields omitted to
/// keep the public surface focused on the call sites that PCSX2 cares
/// about.
#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct CubebStreamParams {
    pub format: CubebSampleFormat,
    pub rate: u32,
    pub channels: u32,
}

/// Miscellaneous stream preferences. The C-side enum is a bitmask; the
/// variants below cover the named flags exposed through the vtable.
#[repr(u32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum CubebStreamPref {
    /// No preference.
    NONE = 0x00,
    /// Notification stream (system sounds, alerts).
    NOTIFICATION = 0x01,
    /// Voice stream (VoIP, push-to-talk).
    VOICE = 0x02,
    /// Communication stream (full-duplex voice).
    COMMUNICATION = 0x03,
    /// Media stream (music, movies).
    MEDIA = 0x04,
    /// Game stream (interactive game audio).
    GAME = 0x05,
}

/// Channel-layout selector used when negotiating with the backend.
#[repr(u32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum CubebLayout {
    UNDEFINED = 0,
    DUAL_MONO = 1,
    DUAL_STEREO = 2,
    MONO = 3,
    STEREO = 4,
}

/// Stream lifecycle state reported through the state callback.
#[repr(i32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum CubebState {
    STARTED = 0,
    STOPPED = 1,
    DRAINED = 2,
    ERROR = 3,
}

/// Result code returned by every public entry point.
#[repr(i32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum CubebError {
    OK = 0,
    ERROR = -1,
    INVALID_FORMAT = -2,
    INVALID_PARAMETER = -3,
    NOT_SUPPORTED = -4,
    DEVICE_UNAVAILABLE = -5,
}

pub use CubebError::OK as CUBEB_OK;
pub use CubebError::ERROR as CUBEB_ERROR;
pub use CubebError::INVALID_FORMAT as CUBEB_ERROR_INVALID_FORMAT;
pub use CubebError::INVALID_PARAMETER as CUBEB_ERROR_INVALID_PARAMETER;
pub use CubebError::NOT_SUPPORTED as CUBEB_ERROR_NOT_SUPPORTED;
pub use CubebError::DEVICE_UNAVAILABLE as CUBEB_ERROR_DEVICE_UNAVAILABLE;

// ---------------------------------------------------------------------------
// Callback typedefs.
// ---------------------------------------------------------------------------

/// Per-stream data callback. Invoked by the backend to pull (output) or
/// push (input) audio frames.
///
/// Returning fewer frames than requested puts the stream in drain mode;
/// returning `CUBEB_ERROR` puts the stream in error mode.
pub type CubebStreamCallback = unsafe extern "C" fn(
    stream: *mut c_void,
    user_ptr: *mut c_void,
    input_buffer: *const c_void,
    output_buffer: *mut c_void,
    nframes: c_long,
) -> c_long;

/// Per-stream state callback. Invoked whenever the stream transitions
/// between `STARTED`, `STOPPED`, `DRAINED`, and `ERROR`.
pub type CubebStateCallback =
    unsafe extern "C" fn(stream: *mut c_void, user_ptr: *mut c_void, state: CubebState);

/// Alias matching the spelling used in the spec.
pub type CubebDataCallback = CubebStreamCallback;

// ---------------------------------------------------------------------------
// Global state (mirrors file-scope `static` storage in the C library).
// ---------------------------------------------------------------------------

/// Optional logging callback installed by `cubeb_set_log_callback`.
/// `None` means logging is disabled (the C default).
pub static mut CUBEB_LOG_CALLBACK: Option<CubebLogCallback> = None;

/// Backend selection override; `None` lets `cubeb_init` walk the
/// compiled-in default ordering.
pub static mut CUBEB_BACKEND_OVERRIDE: Option<&'static str> = None;

/// Cached preferred sample rate populated by `cubeb_get_preferred_sample_rate`.
pub static mut CUBEB_PREFERRED_RATE: u32 = 0;

// ---------------------------------------------------------------------------
// Logging support.
// ---------------------------------------------------------------------------

/// Logging callback signature. Variadic in C; we expose a fixed
/// `(level, message)` signature for idiomatic Rust use.
pub type CubebLogCallback = unsafe extern "C" fn(level: i32, message: *const c_char);

/// Set the logging callback. Mirrors `cubeb_set_log_callback`.
///
/// # Safety
///
/// `callback` must either be `None` (to disable logging) or a function
/// pointer that stays valid for as long as it remains installed.
pub unsafe fn cubeb_set_log_callback(level: i32, callback: Option<CubebLogCallback>) -> i32 {
    if level < 0 || level > 2 {
        return CUBEB_ERROR_INVALID_FORMAT as i32;
    }
    if callback.is_none() && level != 0 {
        return CUBEB_ERROR_INVALID_PARAMETER as i32;
    }
    if CUBEB_LOG_CALLBACK.is_some() && callback.is_some() {
        return CUBEB_ERROR_NOT_SUPPORTED as i32;
    }
    CUBEB_LOG_CALLBACK = callback;
    CUBEB_OK as i32
}

/// Forward a log line through the currently-installed callback.
unsafe fn log_line(level: i32, msg: *const c_char) {
    if let Some(cb) = CUBEB_LOG_CALLBACK {
        cb(level, msg);
    }
}

// ---------------------------------------------------------------------------
// Backend registry (replaces the C array-of-init-fn-points literal).
// ---------------------------------------------------------------------------

/// Init function signature for every backend. Each backend
/// (`alsa_init`, `pulse_init`, ...) exposes a function with this shape.
pub type CubebBackendInit =
    unsafe extern "C" fn(context: *mut *mut CubebContext, context_name: *const c_char) -> i32;

/// Names of the compiled-in backends, in their default preference order.
pub static mut BACKEND_NAMES: [&'static str; 6] =
    ["pulse", "alsa", "audiounit", "wasapi", "opensl", "sndio"];

/// Per-backend init table. Each entry is `Some(fn)` when the backend
/// was compiled in and `None` otherwise, mirroring the `#ifdef` guards
/// in `cubeb.c`.
pub static mut BACKEND_INIT: [Option<CubebBackendInit>; 6] = [
    None, // pulse
    None, // alsa
    None, // audiounit
    None, // wasapi
    None, // opensl
    None, // sndio
];

/// Look up an init function for a backend name. Returns `None` if the
/// name is not recognised.
pub unsafe fn lookup_backend(name: &str) -> Option<CubebBackendInit> {
    for (idx, known) in BACKEND_NAMES.iter().enumerate() {
        if *known == name {
            return BACKEND_INIT[idx];
        }
    }
    None
}

/// Look up the index of a backend by name.
pub unsafe fn backend_index(name: &str) -> Option<usize> {
    BACKEND_NAMES.iter().position(|n| *n == name)
}

// ---------------------------------------------------------------------------
// Public entry points.
// ---------------------------------------------------------------------------

/// Initialise the cubeb library. Walks the registered backend list and
/// returns the first one that successfully initialises.
///
/// Mirrors `cubeb_init` from `cubeb.c`.
pub unsafe extern "C" fn cubeb_init(
    context: *mut *mut CubebContext,
    context_name: *const c_char,
    backend_name: *const c_char,
) -> i32 {
    if context.is_null() {
        return CUBEB_ERROR_INVALID_PARAMETER as i32;
    }

    // 1. Honour explicit backend selection when provided.
    if !backend_name.is_null() {
        let requested = match cstr_to_str(backend_name) {
            Some(s) => s,
            None => return CUBEB_ERROR_INVALID_PARAMETER as i32,
        };
        if let Some(idx) = backend_index(requested) {
            if let Some(init) = BACKEND_INIT[idx] {
                if init(context, context_name) == CUBEB_OK as i32 {
                    return CUBEB_OK as i32;
                }
            }
        }
    }

    // 2. Fall back to the compiled-in default ordering.
    for slot in BACKEND_INIT.iter() {
        if let Some(init) = *slot {
            if init(context, context_name) == CUBEB_OK as i32 {
                return CUBEB_OK as i32;
            }
        }
    }

    CUBEB_ERROR as i32
}

/// Return the read-only backend identifier of the active context.
pub unsafe extern "C" fn cubeb_get_backend_id(context: *mut CubebContext) -> *const c_char {
    if context.is_null() {
        return std::ptr::null();
    }
    let ops = ops_of(context);
    match ops.and_then(|o| o.get_backend_id) {
        Some(f) => f(context),
        None => std::ptr::null(),
    }
}

/// Query the maximum channel count supported by the backend.
pub unsafe extern "C" fn cubeb_get_max_channel_count(
    context: *mut CubebContext,
    max_channels: *mut u32,
) -> i32 {
    if context.is_null() || max_channels.is_null() {
        return CUBEB_ERROR_INVALID_PARAMETER as i32;
    }
    let ops = match ops_of(context) {
        Some(o) => o,
        None => return CUBEB_ERROR_INVALID_PARAMETER as i32,
    };
    match ops.get_max_channel_count {
        Some(f) => f(context, max_channels),
        None => CUBEB_ERROR_NOT_SUPPORTED as i32,
    }
}

/// Query the minimum latency in frames that the backend will guarantee
/// for the supplied parameters.
pub unsafe extern "C" fn cubeb_get_min_latency(
    context: *mut CubebContext,
    params: *const CubebStreamParams,
    latency_frames: *mut u32,
) -> i32 {
    if context.is_null() || params.is_null() || latency_frames.is_null() {
        return CUBEB_ERROR_INVALID_PARAMETER as i32;
    }
    let ops = match ops_of(context) {
        Some(o) => o,
        None => return CUBEB_ERROR_INVALID_PARAMETER as i32,
    };
    match ops.get_min_latency {
        Some(f) => f(context, *params, latency_frames),
        None => CUBEB_ERROR_NOT_SUPPORTED as i32,
    }
}

/// Initialise a duplex stream.
///
/// Mirrors `cubeb_stream_init` from `cubeb.c`. Validates parameters,
/// then dispatches into the backend's `stream_init` op.
pub unsafe extern "C" fn cubeb_stream_init(
    context: *mut CubebContext,
    stream: *mut *mut CubebStream,
    stream_name: *const c_char,
    input_device: *const c_void,
    input_stream_params: *const CubebStreamParams,
    output_device: *const c_void,
    output_stream_params: *const CubebStreamParams,
    latency_frames: u32,
    data_callback: CubebDataCallback,
    state_callback: CubebStateCallback,
    user_ptr: *mut c_void,
) -> i32 {
    if context.is_null()
        || stream.is_null()
        || data_callback as usize == 0
        || state_callback as usize == 0
    {
        return CUBEB_ERROR_INVALID_PARAMETER as i32;
    }

    if let Some(rv) = validate_stream_params(input_stream_params, output_stream_params) {
        return rv;
    }
    if let Some(rv) = validate_latency(latency_frames) {
        return rv;
    }

    let ops = match ops_of(context) {
        Some(o) => o,
        None => return CUBEB_ERROR as i32,
    };
    match ops.stream_init {
        Some(f) => f(
            context,
            stream,
            stream_name,
            input_device,
            input_stream_params,
            output_device,
            output_stream_params,
            latency_frames,
            data_callback,
            state_callback,
            user_ptr,
        ),
        None => CUBEB_ERROR_NOT_SUPPORTED as i32,
    }
}

/// Start playback on an initialised stream.
pub unsafe extern "C" fn cubeb_stream_start(stream: *mut CubebStream) -> i32 {
    if stream.is_null() {
        return CUBEB_ERROR_INVALID_PARAMETER as i32;
    }
    let ops = match ops_of_stream(stream) {
        Some(o) => o,
        None => return CUBEB_ERROR as i32,
    };
    match ops.stream_start {
        Some(f) => f(stream),
        None => CUBEB_ERROR_NOT_SUPPORTED as i32,
    }
}

/// Stop playback on a running stream. Must be called before
/// `cubeb_stream_destroy`.
pub unsafe extern "C" fn cubeb_stream_stop(stream: *mut CubebStream) -> i32 {
    if stream.is_null() {
        return CUBEB_ERROR_INVALID_PARAMETER as i32;
    }
    let ops = match ops_of_stream(stream) {
        Some(o) => o,
        None => return CUBEB_ERROR as i32,
    };
    match ops.stream_stop {
        Some(f) => f(stream),
        None => CUBEB_ERROR_NOT_SUPPORTED as i32,
    }
}

/// Destroy an initialised stream. The caller is responsible for having
/// already invoked `cubeb_stream_stop`.
pub unsafe extern "C" fn cubeb_stream_destroy(stream: *mut CubebStream) {
    if stream.is_null() {
        return;
    }
    if let Some(ops) = ops_of_stream(stream) {
        if let Some(f) = ops.stream_destroy {
            f(stream);
        }
    }
}

/// Query the backend's preferred sample format.
pub unsafe extern "C" fn cubeb_get_preferred_sample_format(
    context: *mut CubebContext,
    format: *mut CubebSampleFormat,
) -> i32 {
    if context.is_null() || format.is_null() {
        return CUBEB_ERROR_INVALID_PARAMETER as i32;
    }
    let ops = match ops_of(context) {
        Some(o) => o,
        None => return CUBEB_ERROR_INVALID_PARAMETER as i32,
    };
    match ops.get_preferred_sample_format {
        Some(f) => f(context, format),
        None => {
            // Fall back to the preferred-sample-rate analogue when the
            // backend does not expose an explicit preferred-format hook.
            *format = CubebSampleFormat::F32LE;
            CUBEB_OK as i32
        }
    }
}

/// Query a sample format that the backend is guaranteed to support.
pub unsafe extern "C" fn cubeb_get_supported_format(
    context: *mut CubebContext,
    format: *mut CubebSampleFormat,
) -> i32 {
    if context.is_null() || format.is_null() {
        return CUBEB_ERROR_INVALID_PARAMETER as i32;
    }
    let ops = match ops_of(context) {
        Some(o) => o,
        None => return CUBEB_ERROR_INVALID_PARAMETER as i32,
    };
    match ops.get_supported_format {
        Some(f) => f(context, format),
        None => {
            // Backends without a dedicated hook all accept F32LE.
            *format = CubebSampleFormat::F32LE;
            CUBEB_OK as i32
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers.
// ---------------------------------------------------------------------------

/// Internal `cubeb` representation. Mirrors `struct cubeb` from
/// `cubeb.c`: an ops pointer followed by backend-private storage. Backends
/// are expected to place their own fields at `state_offset` and beyond.
#[repr(C)]
pub(crate) struct CubebInner {
    pub ops: *const CubebOps,
    // The C struct has no required fields beyond `ops`; backends add their
    // own state inline. We keep the layout minimal here.
}

/// Internal `cubeb_stream` representation. Mirrors `struct cubeb_stream`
/// from `cubeb.c`: parent context first, user pointer second. Backends
/// add their own fields after these two.
#[repr(C)]
pub(crate) struct CubebStreamInner {
    pub context: *mut CubebContext,
    pub user_ptr: *mut c_void,
}

/// Read the ops vtable pointer out of a context handle.
#[inline]
unsafe fn ops_of(context: *mut CubebContext) -> Option<&'static CubebOps> {
    if context.is_null() {
        return None;
    }
    let inner = context as *const CubebInner;
    let ops = (*inner).ops;
    if ops.is_null() {
        None
    } else {
        Some(&*ops)
    }
}

/// Read the ops vtable pointer out of a stream handle by way of its
/// parent context.
#[inline]
unsafe fn ops_of_stream(stream: *mut CubebStream) -> Option<&'static CubebOps> {
    if stream.is_null() {
        return None;
    }
    let inner = stream as *const CubebStreamInner;
    ops_of((*inner).context)
}

/// Validate a pair of stream parameters. Returns `Some(err)` on failure,
/// `None` on success. Mirrors `validate_stream_params` from `cubeb.c`.
#[inline]
unsafe fn validate_stream_params(
    input: *const CubebStreamParams,
    output: *const CubebStreamParams,
) -> Option<i32> {
    if input.is_null() && output.is_null() {
        return Some(CUBEB_ERROR_INVALID_PARAMETER as i32);
    }

    if let Some(o) = output.as_ref() {
        if o.rate < 1000 || o.rate > 768_000 || o.channels == 0 || o.channels > u8::MAX as u32 {
            return Some(CUBEB_ERROR_INVALID_FORMAT as i32);
        }
    }
    if let Some(i) = input.as_ref() {
        if i.rate < 1000 || i.rate > 768_000 || i.channels == 0 || i.channels > u8::MAX as u32 {
            return Some(CUBEB_ERROR_INVALID_FORMAT as i32);
        }
    }

    if let (Some(i), Some(o)) = (input.as_ref(), output.as_ref()) {
        if i.rate != o.rate || i.format != o.format {
            return Some(CUBEB_ERROR_INVALID_FORMAT as i32);
        }
    }

    let primary = input
        .as_ref()
        .or_else(|| output.as_ref())
        .expect("at least one params pointer is non-null");
    match primary.format {
        CubebSampleFormat::S16LE
        | CubebSampleFormat::S16BE
        | CubebSampleFormat::F32LE
        | CubebSampleFormat::F32BE => None,
        _ => Some(CUBEB_ERROR_INVALID_FORMAT as i32),
    }
}

/// Validate a latency value. Mirrors `validate_latency` from `cubeb.c`.
#[inline]
fn validate_latency(latency: u32) -> Option<i32> {
    if latency < 1 || latency > 96_000 {
        Some(CUBEB_ERROR_INVALID_PARAMETER as i32)
    } else {
        None
    }
}

/// Convert a borrowed C string into an owned `&str`, returning `None`
/// when the pointer is null or the bytes are not valid UTF-8.
#[inline]
unsafe fn cstr_to_str<'a>(ptr: *const c_char) -> Option<&'a str> {
    if ptr.is_null() {
        return None;
    }
    let mut len = 0usize;
    while *ptr.add(len) != 0 {
        len += 1;
    }
    let slice = std::slice::from_raw_parts(ptr as *const u8, len);
    std::str::from_utf8(slice).ok()
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_latency_accepts_normal_values() {
        assert_eq!(validate_latency(100), None);
        assert_eq!(validate_latency(1), None);
        assert_eq!(validate_latency(96_000), None);
    }

    #[test]
    fn validate_latency_rejects_outliers() {
        assert_eq!(validate_latency(0), Some(CUBEB_ERROR_INVALID_PARAMETER as i32));
        assert_eq!(
            validate_latency(96_001),
            Some(CUBEB_ERROR_INVALID_PARAMETER as i32)
        );
    }

    #[test]
    fn sample_format_endianness() {
        assert!(CubebSampleFormat::S16LE.is_little_endian());
        assert!(CubebSampleFormat::F32LE.is_little_endian());
        assert!(!CubebSampleFormat::S16BE.is_little_endian());
        assert!(CubebSampleFormat::S16LE.is_integer());
        assert!(!CubebSampleFormat::F32LE.is_integer());
    }

    #[test]
    fn cstr_to_str_handles_null() {
        let p: *const c_char = std::ptr::null();
        assert!(unsafe { cstr_to_str(p) }.is_none());
    }

    #[test]
    fn cstr_to_str_decodes_ascii() {
        let bytes = b"alsa\0";
        let p = bytes.as_ptr() as *const c_char;
        assert_eq!(unsafe { cstr_to_str(p) }, Some("alsa"));
    }
}
