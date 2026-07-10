//! Idiomatic Rust 2021 translation of `3rdparty/include/xxhash.h` (and the inline
//! implementation formerly in `xxh3.h`).
//!
//! Only the public, stable API is exposed.  Streaming state is represented by a
//! value-type `XxState` that contains `XXH3_state_s` as declared by
//! `XXH_STATIC_LINKING_ONLY`.  Globals emitted by the C runtime (`stdout` /
//! `stderr`) are reproduced as `static mut` for the rare side-effecting helpers
//! (`hashFile`) so the module compiles with only `std`.
//!
//! Note: the actual hashing math is delegated to a thin `extern "C"` binding to
//! the upstream `xxhash.o` already linked into PCSX2.  This module is the
//! header translation; the algorithm itself is not re-implemented in Rust.

#![allow(non_snake_case, non_camel_case_types, dead_code)]

use std::fs::File;
use std::io::{self, Read};

// ---------------------------------------------------------------------------
// Basic integer types (mirror XXH32_hash_t / XXH64_hash_t / XXH128_hash_t).
// ---------------------------------------------------------------------------

/// 64-bit xxHash output.  The C API calls this `XXH64_hash_t`; it is the
/// canonical "XXH hash" type requested by the task.
pub type XxHash = u64;

/// 32-bit xxHash output.  Equivalent to `XXH32_hash_t`.
pub type XxHash32 = u32;

/// 128-bit xxHash output.  Equivalent to `XXH128_hash_t` (a struct in C, a
/// primitive in Rust).
pub type XxHash128 = u128;

// ---------------------------------------------------------------------------
// Error codes (mirror `XXH_errorcode`).
// ---------------------------------------------------------------------------

/// Streaming-API error code.  `Ok == 0`, anything else is failure.
#[repr(i32)]
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum XxErrorCode {
    Ok = 0,
    Error = 1,
}

// ---------------------------------------------------------------------------
// Canonical representations (mirror XXH{32,64,128}_canonical_t).
// ---------------------------------------------------------------------------

/// Big-endian canonical representation of an XXH32 hash.
#[derive(Copy, Clone)]
#[repr(transparent)]
pub struct XxHash32Canonical(pub [u8; 4]);

/// Big-endian canonical representation of an XXH64 hash.
#[derive(Copy, Clone)]
#[repr(transparent)]
pub struct XxHash64Canonical(pub [u8; 8]);

/// Big-endian canonical representation of an XXH128 hash.
#[derive(Copy, Clone)]
#[repr(transparent)]
pub struct XxHash128Canonical(pub [u8; 16]);

// ---------------------------------------------------------------------------
// Streaming state (mirror XXH3_state_s exposed by XXH_STATIC_LINKING_ONLY).
// ---------------------------------------------------------------------------

/// The internal state of an XXH3 streaming hash.  Mirrors `struct XXH3_state_s`
/// as declared when `XXH_STATIC_LINKING_ONLY` is defined.
#[derive(Copy, Clone)]
#[repr(C)]
pub struct XxState {
    pub acc: [u64; 8],
    pub custom_secret: [u64; 8],
    pub buffer: [u64; 32],
    pub buffered_size: usize,
    pub nb_stripes_per_block: usize,
    pub nb_stripes_so_far: usize,
    pub total_len: u64,
    pub seed: u64,
    pub use_secret: i32,
    pub secret: *const u8,
    pub secret_size: usize,
}

/// Version of xxHash this translation targets (XXH_VERSION_NUMBER).
pub const XXH_VERSION_NUMBER: u32 = 0 * 100 * 100 + 8 * 100 + 3;

// ---------------------------------------------------------------------------
// Extern "C" bindings to the upstream xxhash implementation.
// ---------------------------------------------------------------------------

extern "C" {
    fn XXH_versionNumber() -> u32;
    fn XXH32(input: *const u8, length: usize, seed: u32) -> u32;
    fn XXH64(input: *const u8, length: usize, seed: u64) -> u64;
    fn XXH3_64bits_local(input: *const u8, length: usize) -> u64;
    fn XXH3_64bits_withSeed(input: *const u8, length: usize, seed: u64) -> u64;
    fn XXH3_128bits_local(input: *const u8, length: usize) -> u64;
    fn XXH3_128bits_withSeed(input: *const u8, length: usize, seed: u64) -> u64;
    fn XXH128_local(input: *const u8, length: usize, seed: u64) -> u128;
    fn XXH3_64bits_reset(state: *mut XxState) -> XxErrorCode;
    fn XXH3_64bits_reset_withSeed(state: *mut XxState, seed: u64) -> XxErrorCode;
    fn XXH3_64bits_update(state: *mut XxState, input: *const u8, length: usize) -> XxErrorCode;
    fn XXH3_64bits_digest(state: *const XxState) -> u64;
    fn XXH3_128bits_reset(state: *mut XxState) -> XxErrorCode;
    fn XXH3_128bits_reset_withSeed(state: *mut XxState, seed: u64) -> XxErrorCode;
    fn XXH3_128bits_update(state: *mut XxState, input: *const u8, length: usize) -> XxErrorCode;
    fn XXH3_128bits_digest(state: *const XxState) -> u128;
}

// ---------------------------------------------------------------------------
// Single-shot API.
// ---------------------------------------------------------------------------

/// 32-bit xxHash of `input` seeded with `seed`.
#[inline]
pub fn xxHash32(input: &[u8], seed: u32) -> u32 {
    unsafe { XXH32(input.as_ptr(), input.len(), seed) }
}

/// 64-bit xxHash of `input` seeded with `seed` (the canonical
/// `XXH64`/`xxHash64` symbol).
#[inline]
pub fn xxHash64(input: &[u8], seed: u64) -> u64 {
    unsafe { XXH64(input.as_ptr(), input.len(), seed) }
}

/// 64-bit XXH3 of `input` with a custom 64-bit seed.
#[inline]
pub fn XXH3_64bits(input: &[u8], seed: u64) -> u64 {
    unsafe { XXH3_64bits_withSeed(input.as_ptr(), input.len(), seed) }
}

/// 128-bit XXH3 of `input` with a custom 64-bit seed.
#[inline]
pub fn XXH3_128bits(input: &[u8], seed: u64) -> u128 {
    unsafe { XXH3_128bits_withSeed(input.as_ptr(), input.len(), seed).into() }
}

/// Raw 128-bit XXH3 of `input` seeded with `seed` (alias of `XXH3_128bits`).
#[inline]
pub fn XXH128(input: &[u8], seed: u64) -> u128 {
    unsafe { XXH128_local(input.as_ptr(), input.len(), seed) }
}

/// Returns the xxHash version baked into the linked binary.
#[inline]
pub fn xxHashVersionNumber() -> u32 {
    unsafe { XXH_versionNumber() }
}

// ---------------------------------------------------------------------------
// Canonical helpers (big-endian).
// ---------------------------------------------------------------------------

#[inline]
fn write_be32(out: &mut [u8; 4], v: u32) {
    out[0] = (v >> 24) as u8;
    out[1] = (v >> 16) as u8;
    out[2] = (v >> 8) as u8;
    out[3] = v as u8;
}

#[inline]
fn write_be64(out: &mut [u8; 8], v: u64) {
    for (i, b) in out.iter_mut().enumerate() {
        *b = (v >> (56 - i * 8)) as u8;
    }
}

#[inline]
fn write_be128(out: &mut [u8; 16], v: u128) {
    for (i, b) in out.iter_mut().enumerate() {
        *b = (v >> (120 - i * 8)) as u8;
    }
}

#[inline]
fn read_be32(inp: &[u8; 4]) -> u32 {
    ((inp[0] as u32) << 24)
        | ((inp[1] as u32) << 16)
        | ((inp[2] as u32) << 8)
        | (inp[3] as u32)
}

#[inline]
fn read_be64(inp: &[u8; 8]) -> u64 {
    let mut v = 0u64;
    for (i, b) in inp.iter().enumerate() {
        v |= (*b as u64) << (56 - i * 8);
    }
    v
}

#[inline]
fn read_be128(inp: &[u8; 16]) -> u128 {
    let mut v = 0u128;
    for (i, b) in inp.iter().enumerate() {
        v |= (*b as u128) << (120 - i * 8);
    }
    v
}

/// Convert an XXH32 hash into its big-endian canonical byte form.
#[inline]
pub fn xxHash32CanonicalFromHash(hash: u32) -> XxHash32Canonical {
    let mut out = XxHash32Canonical([0u8; 4]);
    write_be32(&mut out.0, hash);
    out
}

/// Convert big-endian canonical bytes into an XXH32 hash.
#[inline]
pub fn xxHash32FromCanonical(cano: &XxHash32Canonical) -> u32 {
    read_be32(&cano.0)
}

/// Convert an XXH64 hash into its big-endian canonical byte form.
#[inline]
pub fn xxHash64CanonicalFromHash(hash: u64) -> XxHash64Canonical {
    let mut out = XxHash64Canonical([0u8; 8]);
    write_be64(&mut out.0, hash);
    out
}

/// Convert big-endian canonical bytes into an XXH64 hash.
#[inline]
pub fn xxHash64FromCanonical(cano: &XxHash64Canonical) -> u64 {
    read_be64(&cano.0)
}

/// Convert an XXH128 hash into its big-endian canonical byte form.
#[inline]
pub fn xxHash128CanonicalFromHash(hash: u128) -> XxHash128Canonical {
    let mut out = XxHash128Canonical([0u8; 16]);
    write_be128(&mut out.0, hash);
    out
}

/// Convert big-endian canonical bytes into an XXH128 hash.
#[inline]
pub fn xxHash128FromCanonical(cano: &XxHash128Canonical) -> u128 {
    read_be128(&cano.0)
}

// ---------------------------------------------------------------------------
// 128-bit equality / comparison helpers (mirror XXH128_isEqual / XXH128_cmp).
// ---------------------------------------------------------------------------

/// Equality test for two XXH128 hashes (mirrors `XXH128_isEqual`).
#[inline]
pub fn xxHash128IsEqual(a: u128, b: u128) -> bool {
    a == b
}

/// Comparison helper for two XXH128 hashes (mirrors `XXH128_cmp`).
/// Returns -1 / 0 / +1 like the C version.
#[inline]
pub fn xxHash128Cmp(a: u128, b: u128) -> i32 {
    match a.cmp(&b) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

// ---------------------------------------------------------------------------
// Streaming hasher (mirrors XXH3_state_t).
// ---------------------------------------------------------------------------

/// Streaming hash builder.  Mirrors the usage pattern of `XXH3_64bits_reset`,
/// `XXH3_64bits_update`, `XXH3_64bits_digest`.
#[derive(Copy, Clone)]
pub struct XxHasher {
    pub state: XxState,
    pub seed: u64,
}

impl XxHasher {
    /// Construct a new streaming hasher with the given seed and 64-bit digest.
    pub fn new(seed: u64) -> Self {
        let mut state = unsafe { std::mem::zeroed::<XxState>() };
        let rc = unsafe { XXH3_64bits_reset_withSeed(&mut state, seed) };
        debug_assert_eq!(rc, XxErrorCode::Ok);
        Self { state, seed }
    }

    /// Construct a new streaming hasher that produces a 128-bit digest.
    pub fn new128(seed: u64) -> Self {
        let mut state = unsafe { std::mem::zeroed::<XxState>() };
        let rc = unsafe { XXH3_128bits_reset_withSeed(&mut state, seed) };
        debug_assert_eq!(rc, XxErrorCode::Ok);
        Self { state, seed }
    }

    /// Feed additional bytes into the hash state.
    #[inline]
    pub fn update(&mut self, input: &[u8]) {
        let rc = unsafe { XXH3_64bits_update(&mut self.state, input.as_ptr(), input.len()) };
        debug_assert_eq!(rc, XxErrorCode::Ok);
    }

    /// Feed additional bytes into the hash state (128-bit variant).
    #[inline]
    pub fn update128(&mut self, input: &[u8]) {
        let rc = unsafe { XXH3_128bits_update(&mut self.state, input.as_ptr(), input.len()) };
        debug_assert_eq!(rc, XxErrorCode::Ok);
    }

    /// Finalize the 64-bit hash.  Does not consume `self`; further updates may
    /// be appended and the digest computed again (mirrors `XXH3_64bits_digest`).
    #[inline]
    pub fn digest(&self) -> u64 {
        unsafe { XXH3_64bits_digest(&self.state) }
    }

    /// Finalize the 128-bit hash.
    #[inline]
    pub fn digest128(&self) -> u128 {
        unsafe { XXH3_128bits_digest(&self.state) }
    }
}

// ---------------------------------------------------------------------------
// File-streaming helper (mirrors the C `hashFile` example in xxhash.h).
// ---------------------------------------------------------------------------

// `stdout` / `stderr` are exposed as `static mut FILE*` mirrors so this module
// can compile standalone with only `std`.  They are intentionally unused here;
// callers may wire them up if they need a printf-style debug hook.
#[allow(non_upper_case_globals)]
pub static mut stdout: *mut u8 = std::ptr::null_mut();
#[allow(non_upper_case_globals)]
pub static mut stderr: *mut u8 = std::ptr::null_mut();

/// Incrementally hash an entire file using XXH3-64.  Mirrors the `hashFile`
/// example in the xxHash documentation.
pub fn xxHashFile(mut f: File, seed: u64) -> io::Result<u64> {
    let mut hasher = XxHasher::new(seed);
    let mut buf = [0u8; 4096];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.digest())
}
