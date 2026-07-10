// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
// Rust translation of `common/MD5Digest.h` and `common/MD5Digest.cpp`.
//
// This module provides an incremental MD5 hasher modelled directly on the
// original C++ implementation, which itself is based on John Walker's
// public-domain reference. The algorithm is implemented from scratch in
// pure Rust (no external crate dependencies) and is therefore suitable for
// use wherever PCSX2 needs to be able to produce MD5 digests without
// pulling in a third-party crypto crate.
//
// Endianness note: the C++ implementation interprets the internal byte
// buffer as an array of `u32` directly via pointer aliasing. On every
// platform PCSX2 ships for that means little-endian word order, so we
// read 32-bit message words using `u32::from_le_bytes` to preserve the
// exact bit-for-bit behaviour of the original.

#![allow(non_snake_case)]

/// MD5 hasher state.
///
/// An instance starts at the standard MD5 initial state and accumulates
/// input bytes via [`MD5Digest::update`]. The final 16-byte digest is
/// produced by [`MD5Digest::finalize`], which consumes the hasher.
#[derive(Clone)]
pub struct MD5Digest {
    /// Accumulator state (A, B, C, D).
    buf: [u32; 4],
    /// Total number of bits fed into the hasher, stored as a 64-bit
    /// little-endian pair `[low, high]`. We only need to track the
    /// 64-bit length, but keeping the same `[u32; 2]` shape as the
    /// C++ original makes the carry/borrow handling easy to mirror.
    bits: [u32; 2],
    /// Holding area for the final block of input. The MD5 transform
    /// operates on 64-byte blocks, so partial blocks are buffered here
    /// until they fill up.
    in_buf: [u8; 64],
    /// Number of bytes currently buffered in `in_buf`.
    in_len: usize,
}

impl Default for MD5Digest {
    fn default() -> Self {
        Self::new()
    }
}

impl MD5Digest {
    /// Create a fresh MD5 hasher in its standard initial state.
    pub fn new() -> Self {
        let mut d = Self {
            buf: [0; 4],
            bits: [0; 2],
            in_buf: [0; 64],
            in_len: 0,
        };
        d.reset();
        d
    }

    /// Reset the hasher back to its standard initial state, discarding
    /// any data that had been fed in so far.
    pub fn reset(&mut self) {
        self.buf[0] = 0x67452301;
        self.buf[1] = 0xefcdab89;
        self.buf[2] = 0x98badcfe;
        self.buf[3] = 0x10325476;
        self.bits[0] = 0;
        self.bits[1] = 0;
        self.in_buf = [0; 64];
        self.in_len = 0;
    }

    /// Feed `data` into the running MD5 computation.
    pub fn update(&mut self, data: &[u8]) {
        // Update the bit count. The C++ version stores the bit length
        // as a pair of u32s (low, high); we mirror that representation
        // so the carry from low to high behaves identically.
        let cb = data.len() as u64;
        let add_bits_low = (cb << 3) as u32;
        let add_bits_high = (cb >> 29) as u32;

        let t = self.bits[0];
        let (new_low, carry) = t.overflowing_add(add_bits_low);
        self.bits[0] = new_low;
        if carry {
            self.bits[1] = self.bits[1].wrapping_add(1);
        }
        self.bits[1] = self.bits[1].wrapping_add(add_bits_high);

        // Bytes already buffered modulo 64.
        let mut t = ((t >> 3) & 0x3f) as usize;

        // Handle any leading partial block.
        if t != 0 {
            let need = 64 - t;
            if data.len() < need {
                self.in_buf[t..t + data.len()].copy_from_slice(data);
                self.in_len += data.len();
                return;
            }
            self.in_buf[t..64].copy_from_slice(&data[..need]);
            let block = block_to_words(&self.in_buf);
            md5_transform(&mut self.buf, &block);
            // The block has been consumed; the holding area is empty
            // again until the tail of the input is buffered below.
            t = need;
        } else {
            t = 0;
        }

        // Process as many full 64-byte blocks as we can straight from
        // the input slice.
        let mut consumed = t;
        while data.len() - consumed >= 64 {
            let mut block = [0u32; 16];
            for (i, word) in block.iter_mut().enumerate() {
                let off = consumed + i * 4;
                *word = u32::from_le_bytes([
                    data[off],
                    data[off + 1],
                    data[off + 2],
                    data[off + 3],
                ]);
            }
            md5_transform(&mut self.buf, &block);
            consumed += 64;
        }

        // Buffer the tail of the input.
        let tail = &data[consumed..];
        self.in_buf[..tail.len()].copy_from_slice(tail);
        self.in_len = tail.len();
    }

    /// Consume the hasher and return the 16-byte MD5 digest.
    pub fn finalize(mut self) -> [u8; 16] {
        // Number of bytes in the current partial block, mod 64.
        let mut count = (self.bits[0] >> 3) & 0x3f;

        // Append the mandatory 0x80 padding byte. There's always at
        // least one byte free in the holding area.
        self.in_buf[count as usize] = 0x80;
        count += 1;

        // Pad out to 56 mod 64.
        if count > 56 {
            // Not enough room in the current block for the 64-bit
            // length. Pad the rest of this block, transform it, then
            // start a fresh block padded with zeroes.
            for b in &mut self.in_buf[count as usize..64] {
                *b = 0;
            }
            let block = block_to_words(&self.in_buf);
            md5_transform(&mut self.buf, &block);
            for b in &mut self.in_buf[..56] {
                *b = 0;
            }
        } else {
            for b in &mut self.in_buf[count as usize..56] {
                *b = 0;
            }
        }

        // Append the 64-bit length in bits, little-endian, in the last
        // two 32-bit words of the block.
        self.in_buf[56..60].copy_from_slice(&self.bits[0].to_le_bytes());
        self.in_buf[60..64].copy_from_slice(&self.bits[1].to_le_bytes());

        let block = block_to_words(&self.in_buf);
        md5_transform(&mut self.buf, &block);

        // Serialize the accumulator in little-endian byte order.
        let mut out = [0u8; 16];
        for (i, word) in self.buf.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        out
    }

    /// Convenience helper: feed `data` into a fresh hasher and return
    /// the digest as a 32-character lowercase hex string.
    pub fn hex_digest(data: &[u8]) -> String {
        let mut h = Self::new();
        h.update(data);
        let digest = h.finalize();
        let mut s = String::with_capacity(32);
        for byte in digest {
            // `{:02x}` zero-pads single-digit nibbles.
            s.push_str(&format!("{:02x}", byte));
        }
        s
    }
}

/// Interpret a 64-byte block as 16 little-endian 32-bit words, matching
/// the way the C++ code aliases `u32*` over its `u8 in[64]` array.
#[inline]
fn block_to_words(block: &[u8; 64]) -> [u32; 16] {
    let mut words = [0u32; 16];
    for (i, word) in words.iter_mut().enumerate() {
        let off = i * 4;
        *word = u32::from_le_bytes([block[off], block[off + 1], block[off + 2], block[off + 3]]);
    }
    words
}

/// The four auxiliary functions used by the MD5 transform.
#[inline]
fn f1(x: u32, y: u32, z: u32) -> u32 {
    // z ^ (x & (y ^ z))
    z ^ (x & (y ^ z))
}
#[inline]
fn f2(x: u32, y: u32, z: u32) -> u32 {
    f1(z, x, y)
}
#[inline]
fn f3(x: u32, y: u32, z: u32) -> u32 {
    x ^ y ^ z
}
#[inline]
fn f4(x: u32, y: u32, z: u32) -> u32 {
    // y ^ (x | !z)
    y ^ (x | !z)
}

/// A single MD5 round step, matching the `MD5STEP` macro from the C++.
#[inline]
fn md5_step(func: impl Fn(u32, u32, u32) -> u32, w: &mut u32, x: u32, y: u32, z: u32, data: u32, s: u32) {
    *w = w.wrapping_add(func(x, y, z)).wrapping_add(data);
    *w = w.rotate_left(s);
    *w = w.wrapping_add(x);
}

/// Apply one 64-byte block to the running accumulator.
///
/// This is a direct line-by-line port of the original `MD5Transform`
/// function. The four rounds use the standard MD5 round constants.
fn md5_transform(buf: &mut [u32; 4], input: &[u32; 16]) {
    let mut a = buf[0];
    let mut b = buf[1];
    let mut c = buf[2];
    let mut d = buf[3];

    // Round 1.
    md5_step(f1, &mut a, b, c, d, input[0].wrapping_add(0xd76aa478), 7);
    md5_step(f1, &mut d, a, b, c, input[1].wrapping_add(0xe8c7b756), 12);
    md5_step(f1, &mut c, d, a, b, input[2].wrapping_add(0x242070db), 17);
    md5_step(f1, &mut b, c, d, a, input[3].wrapping_add(0xc1bdceee), 22);
    md5_step(f1, &mut a, b, c, d, input[4].wrapping_add(0xf57c0faf), 7);
    md5_step(f1, &mut d, a, b, c, input[5].wrapping_add(0x4787c62a), 12);
    md5_step(f1, &mut c, d, a, b, input[6].wrapping_add(0xa8304613), 17);
    md5_step(f1, &mut b, c, d, a, input[7].wrapping_add(0xfd469501), 22);
    md5_step(f1, &mut a, b, c, d, input[8].wrapping_add(0x698098d8), 7);
    md5_step(f1, &mut d, a, b, c, input[9].wrapping_add(0x8b44f7af), 12);
    md5_step(f1, &mut c, d, a, b, input[10].wrapping_add(0xffff5bb1), 17);
    md5_step(f1, &mut b, c, d, a, input[11].wrapping_add(0x895cd7be), 22);
    md5_step(f1, &mut a, b, c, d, input[12].wrapping_add(0x6b901122), 7);
    md5_step(f1, &mut d, a, b, c, input[13].wrapping_add(0xfd987193), 12);
    md5_step(f1, &mut c, d, a, b, input[14].wrapping_add(0xa679438e), 17);
    md5_step(f1, &mut b, c, d, a, input[15].wrapping_add(0x49b40821), 22);

    // Round 2.
    md5_step(f2, &mut a, b, c, d, input[1].wrapping_add(0xf61e2562), 5);
    md5_step(f2, &mut d, a, b, c, input[6].wrapping_add(0xc040b340), 9);
    md5_step(f2, &mut c, d, a, b, input[11].wrapping_add(0x265e5a51), 14);
    md5_step(f2, &mut b, c, d, a, input[0].wrapping_add(0xe9b6c7aa), 20);
    md5_step(f2, &mut a, b, c, d, input[5].wrapping_add(0xd62f105d), 5);
    md5_step(f2, &mut d, a, b, c, input[10].wrapping_add(0x02441453), 9);
    md5_step(f2, &mut c, d, a, b, input[15].wrapping_add(0xd8a1e681), 14);
    md5_step(f2, &mut b, c, d, a, input[4].wrapping_add(0xe7d3fbc8), 20);
    md5_step(f2, &mut a, b, c, d, input[9].wrapping_add(0x21e1cde6), 5);
    md5_step(f2, &mut d, a, b, c, input[14].wrapping_add(0xc33707d6), 9);
    md5_step(f2, &mut c, d, a, b, input[3].wrapping_add(0xf4d50d87), 14);
    md5_step(f2, &mut b, c, d, a, input[8].wrapping_add(0x455a14ed), 20);
    md5_step(f2, &mut a, b, c, d, input[13].wrapping_add(0xa9e3e905), 5);
    md5_step(f2, &mut d, a, b, c, input[2].wrapping_add(0xfcefa3f8), 9);
    md5_step(f2, &mut c, d, a, b, input[7].wrapping_add(0x676f02d9), 14);
    md5_step(f2, &mut b, c, d, a, input[12].wrapping_add(0x8d2a4c8a), 20);

    // Round 3.
    md5_step(f3, &mut a, b, c, d, input[5].wrapping_add(0xfffa3942), 4);
    md5_step(f3, &mut d, a, b, c, input[8].wrapping_add(0x8771f681), 11);
    md5_step(f3, &mut c, d, a, b, input[11].wrapping_add(0x6d9d6122), 16);
    md5_step(f3, &mut b, c, d, a, input[14].wrapping_add(0xfde5380c), 23);
    md5_step(f3, &mut a, b, c, d, input[1].wrapping_add(0xa4beea44), 4);
    md5_step(f3, &mut d, a, b, c, input[4].wrapping_add(0x4bdecfa9), 11);
    md5_step(f3, &mut c, d, a, b, input[7].wrapping_add(0xf6bb4b60), 16);
    md5_step(f3, &mut b, c, d, a, input[10].wrapping_add(0xbebfbc70), 23);
    md5_step(f3, &mut a, b, c, d, input[13].wrapping_add(0x289b7ec6), 4);
    md5_step(f3, &mut d, a, b, c, input[0].wrapping_add(0xeaa127fa), 11);
    md5_step(f3, &mut c, d, a, b, input[3].wrapping_add(0xd4ef3085), 16);
    md5_step(f3, &mut b, c, d, a, input[6].wrapping_add(0x04881d05), 23);
    md5_step(f3, &mut a, b, c, d, input[9].wrapping_add(0xd9d4d039), 4);
    md5_step(f3, &mut d, a, b, c, input[12].wrapping_add(0xe6db99e5), 11);
    md5_step(f3, &mut c, d, a, b, input[15].wrapping_add(0x1fa27cf8), 16);
    md5_step(f3, &mut b, c, d, a, input[2].wrapping_add(0xc4ac5665), 23);

    // Round 4.
    md5_step(f4, &mut a, b, c, d, input[0].wrapping_add(0xf4292244), 6);
    md5_step(f4, &mut d, a, b, c, input[7].wrapping_add(0x432aff97), 10);
    md5_step(f4, &mut c, d, a, b, input[14].wrapping_add(0xab9423a7), 15);
    md5_step(f4, &mut b, c, d, a, input[5].wrapping_add(0xfc93a039), 21);
    md5_step(f4, &mut a, b, c, d, input[12].wrapping_add(0x655b59c3), 6);
    md5_step(f4, &mut d, a, b, c, input[3].wrapping_add(0x8f0ccc92), 10);
    md5_step(f4, &mut c, d, a, b, input[10].wrapping_add(0xffeff47d), 15);
    md5_step(f4, &mut b, c, d, a, input[1].wrapping_add(0x85845dd1), 21);
    md5_step(f4, &mut a, b, c, d, input[8].wrapping_add(0x6fa87e4f), 6);
    md5_step(f4, &mut d, a, b, c, input[15].wrapping_add(0xfe2ce6e0), 10);
    md5_step(f4, &mut c, d, a, b, input[6].wrapping_add(0xa3014314), 15);
    md5_step(f4, &mut b, c, d, a, input[13].wrapping_add(0x4e0811a1), 21);
    md5_step(f4, &mut a, b, c, d, input[4].wrapping_add(0xf7537e82), 6);
    md5_step(f4, &mut d, a, b, c, input[11].wrapping_add(0xbd3af235), 10);
    md5_step(f4, &mut c, d, a, b, input[2].wrapping_add(0x2ad7d2bb), 15);
    md5_step(f4, &mut b, c, d, a, input[9].wrapping_add(0xeb86d391), 21);

    buf[0] = buf[0].wrapping_add(a);
    buf[1] = buf[1].wrapping_add(b);
    buf[2] = buf[2].wrapping_add(c);
    buf[3] = buf[3].wrapping_add(d);
}

#[cfg(test)]
mod tests {
    use super::MD5Digest;

    fn hex(bytes: &[u8; 16]) -> String {
        let mut s = String::with_capacity(32);
        for b in bytes {
            s.push_str(&format!("{:02x}", b));
        }
        s
    }

    #[test]
    fn empty_string() {
        // Known MD5("") = d41d8cd9 8f00b204 e9800998 ecf8427e
        let d = MD5Digest::new().finalize();
        assert_eq!(hex(&d), "d41d8cd98f00b204e9800998ecf8427e");
    }

    #[test]
    fn abc() {
        // Known MD5("abc") = 90015098 3cd24fb0 d6963f7d 28e17f72
        let mut h = MD5Digest::new();
        h.update(b"abc");
        assert_eq!(hex(&h.finalize()), "900150983cd24fb0d6963f7d28e17f72");
    }

    #[test]
    fn longer_message() {
        // RFC 1321 test vector: MD5("abcdefghijklmnopqrstuvwxyz")
        // = c3fcd3d7 6192e400 7dfb496c ca67e13b
        let mut h = MD5Digest::new();
        h.update(b"abcdefghijklmnopqrstuvwxyz");
        assert_eq!(hex(&h.finalize()), "c3fcd3d76192e4007dfb496cca67e13b");
    }

    #[test]
    fn multi_block_boundary() {
        // Input that crosses the 64-byte internal block boundary.
        // Known MD5("a" * 54 + b"1234567890") =
        //   014842d480b391462a0712d0a43aa760
        let mut h = MD5Digest::new();
        h.update(&vec![b'a'; 54]);
        h.update(b"1234567890");
        assert_eq!(hex(&h.finalize()), "014842d480b391462a0712d0a43aa760");
    }

    #[test]
    fn hex_digest_helper() {
        assert_eq!(
            MD5Digest::hex_digest(b"abc"),
            "900150983cd24fb0d6963f7d28e17f72"
        );
    }
}
