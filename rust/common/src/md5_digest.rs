// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
// Pure-Rust port of `common/MD5Digest.{h,cpp}`.
// Reference: http://www.fourmilab.ch/md5/ (public domain).

//! MD5 message-digest implementation.
//!
//! The public, safe API mirrors the original C++ class:
//! - [`MD5Digest::new`] creates a fresh context.
//! - [`MD5Digest::update`] feeds bytes in.
//! - [`MD5Digest::finalize`] consumes the context and produces the 16-byte digest.
//! - [`MD5Digest::reset`] returns a context to its initial state for reuse.
//! - [`MD5Digest::hash`] is a one-shot convenience.
//!
//! The FFI surface (C++ save-state verification) is exposed at the bottom of
//! the file as `#[no_mangle] pub extern "C"` functions, exactly matching the
//! layout the C++ side expects.

// ============================================================================
// Internal helpers
// ============================================================================

/// Standard MD5 round function (optimized form: `z ^ (x & (y ^ z))`).
#[inline(always)]
const fn f1(x: u32, y: u32, z: u32) -> u32 {
    z ^ (x & (y ^ z))
}

/// F2 = F1 with rotated arguments.
#[inline(always)]
const fn f2(x: u32, y: u32, z: u32) -> u32 {
    f1(z, x, y)
}

/// F3 = XOR of all three.
#[inline(always)]
const fn f3(x: u32, y: u32, z: u32) -> u32 {
    x ^ y ^ z
}

/// F4 = `y ^ (x | ~z)`.
#[inline(always)]
const fn f4(x: u32, y: u32, z: u32) -> u32 {
    y ^ (x | !z)
}

/// One MD5 round step: `w += f(x,y,z) + data; w = rotl(w, s); w += x;`.
///
/// Equivalent to the C++ `MD5STEP` macro. Inlined directly in
/// [`md5_transform`] to keep the round logic as close to the original
/// source as possible (and to satisfy `const fn`).
#[inline(always)]
const fn md5_step_add(w: u32, fx_y_z: u32, data: u32) -> u32 {
    w.wrapping_add(fx_y_z.wrapping_add(data))
}

/// Core MD5 transform: 64 steps over 16 32-bit words of input.
#[inline]
fn md5_transform(buf: &mut [u32; 4], input: &[u8; 64]) {
    // Decode 64 bytes into 16 little-endian u32 words. Equivalent to the
    // C++ `memcpy` from `u8*` to `u32*` on a little-endian host.
    let mut in_words = [0u32; 16];
    let mut i = 0;
    while i < 16 {
        let off = i * 4;
        in_words[i] = u32::from_le_bytes([input[off], input[off + 1], input[off + 2], input[off + 3]]);
        i += 1;
    }

    let mut a = buf[0];
    let mut b = buf[1];
    let mut c = buf[2];
    let mut d = buf[3];

    // Round 1
    a = md5_step_add(a, f1(b, c, d), in_words[0].wrapping_add(0xd76aa478)).rotate_left(7).wrapping_add(b);
    d = md5_step_add(d, f1(a, b, c), in_words[1].wrapping_add(0xe8c7b756)).rotate_left(12).wrapping_add(a);
    c = md5_step_add(c, f1(d, a, b), in_words[2].wrapping_add(0x242070db)).rotate_left(17).wrapping_add(d);
    b = md5_step_add(b, f1(c, d, a), in_words[3].wrapping_add(0xc1bdceee)).rotate_left(22).wrapping_add(c);
    a = md5_step_add(a, f1(b, c, d), in_words[4].wrapping_add(0xf57c0faf)).rotate_left(7).wrapping_add(b);
    d = md5_step_add(d, f1(a, b, c), in_words[5].wrapping_add(0x4787c62a)).rotate_left(12).wrapping_add(a);
    c = md5_step_add(c, f1(d, a, b), in_words[6].wrapping_add(0xa8304613)).rotate_left(17).wrapping_add(d);
    b = md5_step_add(b, f1(c, d, a), in_words[7].wrapping_add(0xfd469501)).rotate_left(22).wrapping_add(c);
    a = md5_step_add(a, f1(b, c, d), in_words[8].wrapping_add(0x698098d8)).rotate_left(7).wrapping_add(b);
    d = md5_step_add(d, f1(a, b, c), in_words[9].wrapping_add(0x8b44f7af)).rotate_left(12).wrapping_add(a);
    c = md5_step_add(c, f1(d, a, b), in_words[10].wrapping_add(0xffff5bb1)).rotate_left(17).wrapping_add(d);
    b = md5_step_add(b, f1(c, d, a), in_words[11].wrapping_add(0x895cd7be)).rotate_left(22).wrapping_add(c);
    a = md5_step_add(a, f1(b, c, d), in_words[12].wrapping_add(0x6b901122)).rotate_left(7).wrapping_add(b);
    d = md5_step_add(d, f1(a, b, c), in_words[13].wrapping_add(0xfd987193)).rotate_left(12).wrapping_add(a);
    c = md5_step_add(c, f1(d, a, b), in_words[14].wrapping_add(0xa679438e)).rotate_left(17).wrapping_add(d);
    b = md5_step_add(b, f1(c, d, a), in_words[15].wrapping_add(0x49b40821)).rotate_left(22).wrapping_add(c);

    // Round 2
    a = md5_step_add(a, f2(b, c, d), in_words[1].wrapping_add(0xf61e2562)).rotate_left(5).wrapping_add(b);
    d = md5_step_add(d, f2(a, b, c), in_words[6].wrapping_add(0xc040b340)).rotate_left(9).wrapping_add(a);
    c = md5_step_add(c, f2(d, a, b), in_words[11].wrapping_add(0x265e5a51)).rotate_left(14).wrapping_add(d);
    b = md5_step_add(b, f2(c, d, a), in_words[0].wrapping_add(0xe9b6c7aa)).rotate_left(20).wrapping_add(c);
    a = md5_step_add(a, f2(b, c, d), in_words[5].wrapping_add(0xd62f105d)).rotate_left(5).wrapping_add(b);
    d = md5_step_add(d, f2(a, b, c), in_words[10].wrapping_add(0x02441453)).rotate_left(9).wrapping_add(a);
    c = md5_step_add(c, f2(d, a, b), in_words[15].wrapping_add(0xd8a1e681)).rotate_left(14).wrapping_add(d);
    b = md5_step_add(b, f2(c, d, a), in_words[4].wrapping_add(0xe7d3fbc8)).rotate_left(20).wrapping_add(c);
    a = md5_step_add(a, f2(b, c, d), in_words[9].wrapping_add(0x21e1cde6)).rotate_left(5).wrapping_add(b);
    d = md5_step_add(d, f2(a, b, c), in_words[14].wrapping_add(0xc33707d6)).rotate_left(9).wrapping_add(a);
    c = md5_step_add(c, f2(d, a, b), in_words[3].wrapping_add(0xf4d50d87)).rotate_left(14).wrapping_add(d);
    b = md5_step_add(b, f2(c, d, a), in_words[8].wrapping_add(0x455a14ed)).rotate_left(20).wrapping_add(c);
    a = md5_step_add(a, f2(b, c, d), in_words[13].wrapping_add(0xa9e3e905)).rotate_left(5).wrapping_add(b);
    d = md5_step_add(d, f2(a, b, c), in_words[2].wrapping_add(0xfcefa3f8)).rotate_left(9).wrapping_add(a);
    c = md5_step_add(c, f2(d, a, b), in_words[7].wrapping_add(0x676f02d9)).rotate_left(14).wrapping_add(d);
    b = md5_step_add(b, f2(c, d, a), in_words[12].wrapping_add(0x8d2a4c8a)).rotate_left(20).wrapping_add(c);

    // Round 3
    a = md5_step_add(a, f3(b, c, d), in_words[5].wrapping_add(0xfffa3942)).rotate_left(4).wrapping_add(b);
    d = md5_step_add(d, f3(a, b, c), in_words[8].wrapping_add(0x8771f681)).rotate_left(11).wrapping_add(a);
    c = md5_step_add(c, f3(d, a, b), in_words[11].wrapping_add(0x6d9d6122)).rotate_left(16).wrapping_add(d);
    b = md5_step_add(b, f3(c, d, a), in_words[14].wrapping_add(0xfde5380c)).rotate_left(23).wrapping_add(c);
    a = md5_step_add(a, f3(b, c, d), in_words[1].wrapping_add(0xa4beea44)).rotate_left(4).wrapping_add(b);
    d = md5_step_add(d, f3(a, b, c), in_words[4].wrapping_add(0x4bdecfa9)).rotate_left(11).wrapping_add(a);
    c = md5_step_add(c, f3(d, a, b), in_words[7].wrapping_add(0xf6bb4b60)).rotate_left(16).wrapping_add(d);
    b = md5_step_add(b, f3(c, d, a), in_words[10].wrapping_add(0xbebfbc70)).rotate_left(23).wrapping_add(c);
    a = md5_step_add(a, f3(b, c, d), in_words[13].wrapping_add(0x289b7ec6)).rotate_left(4).wrapping_add(b);
    d = md5_step_add(d, f3(a, b, c), in_words[0].wrapping_add(0xeaa127fa)).rotate_left(11).wrapping_add(a);
    c = md5_step_add(c, f3(d, a, b), in_words[3].wrapping_add(0xd4ef3085)).rotate_left(16).wrapping_add(d);
    b = md5_step_add(b, f3(c, d, a), in_words[6].wrapping_add(0x04881d05)).rotate_left(23).wrapping_add(c);
    a = md5_step_add(a, f3(b, c, d), in_words[9].wrapping_add(0xd9d4d039)).rotate_left(4).wrapping_add(b);
    d = md5_step_add(d, f3(a, b, c), in_words[12].wrapping_add(0xe6db99e5)).rotate_left(11).wrapping_add(a);
    c = md5_step_add(c, f3(d, a, b), in_words[15].wrapping_add(0x1fa27cf8)).rotate_left(16).wrapping_add(d);
    b = md5_step_add(b, f3(c, d, a), in_words[2].wrapping_add(0xc4ac5665)).rotate_left(23).wrapping_add(c);

    // Round 4
    a = md5_step_add(a, f4(b, c, d), in_words[0].wrapping_add(0xf4292244)).rotate_left(6).wrapping_add(b);
    d = md5_step_add(d, f4(a, b, c), in_words[7].wrapping_add(0x432aff97)).rotate_left(10).wrapping_add(a);
    c = md5_step_add(c, f4(d, a, b), in_words[14].wrapping_add(0xab9423a7)).rotate_left(15).wrapping_add(d);
    b = md5_step_add(b, f4(c, d, a), in_words[5].wrapping_add(0xfc93a039)).rotate_left(21).wrapping_add(c);
    a = md5_step_add(a, f4(b, c, d), in_words[12].wrapping_add(0x655b59c3)).rotate_left(6).wrapping_add(b);
    d = md5_step_add(d, f4(a, b, c), in_words[3].wrapping_add(0x8f0ccc92)).rotate_left(10).wrapping_add(a);
    c = md5_step_add(c, f4(d, a, b), in_words[10].wrapping_add(0xffeff47d)).rotate_left(15).wrapping_add(d);
    b = md5_step_add(b, f4(c, d, a), in_words[1].wrapping_add(0x85845dd1)).rotate_left(21).wrapping_add(c);
    a = md5_step_add(a, f4(b, c, d), in_words[8].wrapping_add(0x6fa87e4f)).rotate_left(6).wrapping_add(b);
    d = md5_step_add(d, f4(a, b, c), in_words[15].wrapping_add(0xfe2ce6e0)).rotate_left(10).wrapping_add(a);
    c = md5_step_add(c, f4(d, a, b), in_words[6].wrapping_add(0xa3014314)).rotate_left(15).wrapping_add(d);
    b = md5_step_add(b, f4(c, d, a), in_words[13].wrapping_add(0x4e0811a1)).rotate_left(21).wrapping_add(c);
    a = md5_step_add(a, f4(b, c, d), in_words[4].wrapping_add(0xf7537e82)).rotate_left(6).wrapping_add(b);
    d = md5_step_add(d, f4(a, b, c), in_words[11].wrapping_add(0xbd3af235)).rotate_left(10).wrapping_add(a);
    c = md5_step_add(c, f4(d, a, b), in_words[2].wrapping_add(0x2ad7d2bb)).rotate_left(15).wrapping_add(d);
    b = md5_step_add(b, f4(c, d, a), in_words[9].wrapping_add(0xeb86d391)).rotate_left(21).wrapping_add(c);

    buf[0] = buf[0].wrapping_add(a);
    buf[1] = buf[1].wrapping_add(b);
    buf[2] = buf[2].wrapping_add(c);
    buf[3] = buf[3].wrapping_add(d);
}

// ============================================================================
// Public safe API
// ============================================================================

/// MD5 message-digest context.
///
/// Construct with [`MD5Digest::new`], feed bytes with [`MD5Digest::update`],
/// then call [`MD5Digest::finalize`] (consuming `self`) to obtain the 16-byte
/// digest. A context may be reused via [`MD5Digest::reset`].
///
/// State layout (mirrors the original C++ class):
/// - `state`:   4-word running hash (`buf` in the C++ source).
/// - `bits`:    2-word total bit count (`bits[0]` = low, `bits[1]` = high).
/// - `buffer`:  64-byte block staging buffer (`in` in the C++ source).
/// - `buffer_len`: number of valid bytes currently in `buffer`.
pub struct MD5Digest {
    state: [u32; 4],
    bits: [u32; 2],
    buffer: [u8; 64],
    buffer_len: usize,
}

// Clone is needed for the FFI `pcsx2_md5_final` which must not take
// ownership of the context — the caller still calls `pcsx2_md5_destroy`.
impl Clone for MD5Digest {
    fn clone(&self) -> Self {
        Self {
            state: self.state,
            bits: self.bits,
            buffer: self.buffer,
            buffer_len: self.buffer_len,
        }
    }
}

impl MD5Digest {
    /// Standard MD5 initialization vector.
    const INIT: [u32; 4] = [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476];

    /// Create a fresh context.
    #[inline]
    pub fn new() -> Self {
        Self {
            state: Self::INIT,
            bits: [0, 0],
            buffer: [0; 64],
            buffer_len: 0,
        }
    }

    /// Reset the context to its initial state, allowing reuse.
    #[inline]
    pub fn reset(&mut self) {
        self.state = Self::INIT;
        self.bits = [0, 0];
        self.buffer = [0; 64];
        self.buffer_len = 0;
    }

    /// Feed `data` into the running hash.
    ///
    /// May be called repeatedly; data does not need to be block-aligned.
    pub fn update(&mut self, mut data: &[u8]) {
        // Update total bit count. `bits[0]` is the low 32 bits of the bit
        // count; `bits[1]` is the high 32 bits, catching carry from below
        // and large inputs.
        let prev = self.bits[0];
        let add_bits = (data.len() as u32).wrapping_shl(3);
        self.bits[0] = prev.wrapping_add(add_bits);
        if self.bits[0] < prev {
            self.bits[1] = self.bits[1].wrapping_add(1);
        }
        self.bits[1] = self.bits[1].wrapping_add((data.len() as u32) >> 29);

        // Bytes already buffered from a previous (incomplete) block.
        let buffered = self.buffer_len;

        // Finish the current partial block, if any.
        if buffered > 0 {
            let need = 64 - buffered;
            if data.len() < need {
                self.buffer[buffered..buffered + data.len()].copy_from_slice(data);
                self.buffer_len = buffered + data.len();
                return;
            }
            self.buffer[buffered..64].copy_from_slice(&data[..need]);
            data = &data[need..];
            let block = self.buffer; // copy out — md5_transform takes &[u8; 64]
            md5_transform(&mut self.state, &block);
            self.buffer_len = 0;
        }

        // Process whole 64-byte blocks directly from the input slice,
        // avoiding the intermediate copy into `self.buffer`.
        while data.len() >= 64 {
            let block: [u8; 64] = data[..64].try_into().expect("64-byte slice");
            md5_transform(&mut self.state, &block);
            data = &data[64..];
        }

        // Stash the trailing partial block.
        if !data.is_empty() {
            self.buffer[..data.len()].copy_from_slice(data);
            self.buffer_len = data.len();
        }
    }

    /// Consume the context and return the 16-byte MD5 digest.
    pub fn finalize(mut self) -> [u8; 16] {
        // Bytes currently held in the partial block.
        let count = self.buffer_len;

        // Append the mandatory 0x80 padding byte.
        self.buffer[count] = 0x80;
        let mut pad_to = count + 1;

        // We need 8 bytes at the end for the 64-bit bit length. If there
        // isn't room, fill the rest of this block with zeros and flush.
        if pad_to > 56 {
            for b in &mut self.buffer[pad_to..] {
                *b = 0;
            }
            let block = self.buffer;
            md5_transform(&mut self.state, &block);
            pad_to = 0;
        }

        // Pad with zeros up to byte 56.
        for b in &mut self.buffer[pad_to..56] {
            *b = 0;
        }

        // Append the 64-bit bit length, little-endian, as two u32 words.
        self.buffer[56..60].copy_from_slice(&self.bits[0].to_le_bytes());
        self.buffer[60..64].copy_from_slice(&self.bits[1].to_le_bytes());

        let block = self.buffer;
        md5_transform(&mut self.state, &block);

        // Emit the digest in little-endian order.
        let mut out = [0u8; 16];
        out[0..4].copy_from_slice(&self.state[0].to_le_bytes());
        out[4..8].copy_from_slice(&self.state[1].to_le_bytes());
        out[8..12].copy_from_slice(&self.state[2].to_le_bytes());
        out[12..16].copy_from_slice(&self.state[3].to_le_bytes());
        out
    }

    /// One-shot MD5: hash `data` and return the 16-byte digest.
    pub fn hash(data: &[u8]) -> [u8; 16] {
        let mut ctx = Self::new();
        ctx.update(data);
        ctx.finalize()
    }
}

impl Default for MD5Digest {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// FFI surface (C++ save-state verification)
// ============================================================================

/// Opaque C-compatible context type.
///
/// The C++ side never inspects the layout; it treats the returned
/// `*mut MD5Digest` as an opaque handle. We declare this zero-sized type
/// to mirror the planned C++ declaration (`struct Pcsx2Md5Context;`)
/// that cbindgen will emit.
#[repr(C)]
#[allow(dead_code)]
pub struct Pcsx2Md5Context {
    _private: [u8; 0],
}

#[no_mangle]
pub extern "C" fn pcsx2_md5_new() -> *mut MD5Digest {
    Box::into_raw(Box::new(MD5Digest::new()))
}

#[no_mangle]
pub extern "C" fn pcsx2_md5_update(ctx: *mut MD5Digest, data: *const u8, len: u32) {
    // Safety: caller must pass a valid pointer from pcsx2_md5_new and a
    // data pointer with at least `len` readable bytes. Null `data` with
    // `len == 0` is allowed (no-op).
    if ctx.is_null() {
        return;
    }
    let digest = unsafe { &mut *ctx };
    if len == 0 {
        return;
    }
    if data.is_null() {
        return;
    }
    let slice = unsafe { std::slice::from_raw_parts(data, len as usize) };
    digest.update(slice);
}

#[no_mangle]
pub extern "C" fn pcsx2_md5_final(ctx: *mut MD5Digest, out: *mut u8) {
    // Safety: caller must pass a valid pointer from pcsx2_md5_new and a
    // 16-byte writable output buffer. The context is NOT consumed — caller
    // must still invoke `pcsx2_md5_destroy` to free it.
    if ctx.is_null() || out.is_null() {
        return;
    }
    // Clone the state to call the consuming `finalize` without freeing
    // the underlying allocation.
    let mut digest_copy = unsafe { (*ctx).clone() };
    let result = digest_copy.finalize();
    unsafe {
        std::ptr::copy_nonoverlapping(result.as_ptr(), out, 16);
    }
}

#[no_mangle]
pub extern "C" fn pcsx2_md5_destroy(ctx: *mut MD5Digest) {
    if !ctx.is_null() {
        // Safety: pointer was produced by `pcsx2_md5_new` (Box::into_raw)
        // and has not yet been freed.
        unsafe {
            let _ = Box::from_raw(ctx);
        }
    }
}

#[no_mangle]
pub extern "C" fn pcsx2_md5_hash(data: *const u8, len: u32, out: *mut u8) {
    // Safety: caller must pass a `len`-byte readable input and a 16-byte
    // writable output. Null inputs with `len == 0` yield the MD5 of the
    // empty string, matching the standard.
    if out.is_null() {
        return;
    }
    let result = if data.is_null() || len == 0 {
        MD5Digest::hash(&[])
    } else {
        let slice = unsafe { std::slice::from_raw_parts(data, len as usize) };
        MD5Digest::hash(slice)
    };
    unsafe {
        std::ptr::copy_nonoverlapping(result.as_ptr(), out, 16);
    }
}
