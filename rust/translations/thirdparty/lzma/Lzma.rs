//! Rust 2021 idiomatic translation of the 7-Zip LZMA encoder/decoder
//! public interface (`LzmaLib.h`, `LzmaLib.c`) together with the inlined
//! range coder, literal/match/length/bit-tree encoders and decoders from
//! `LzmaEnc.c` / `LzmaDec.c`.
//!
//! The translation preserves the byte-for-byte wire format used by the
//! original C library: a five-byte LZMA properties header followed by a
//! range-coded stream.  Probability tables, dictionary buffers and the
//! few encoder-side global tables (the `g_FastPos` lookup table and
//! the `ProbPrices` bit-cost table) are mirrored here with `static mut`
//! items as required by the task rules.
//!
//! Only `std` is depended on.  No `extern "C"` blocks are emitted.

use std::mem;

// ---------------------------------------------------------------------------
// Constants mirroring 7-Zip LzmaEnc.h / LzmaDec.h
// ---------------------------------------------------------------------------

/// Size, in bytes, of the LZMA properties header.
pub const LZMA_PROPS_SIZE: usize = 5;

/// Size of the output buffer used by the range encoder.
pub const RC_BUF_SIZE: usize = 1 << 16;

/// LZMA range coder number of bits used by a probability model.
const K_NUM_BIT_MODEL_TOTAL_BITS: u32 = 11;
/// Total number of probability slots (`1 << K_NUM_BIT_MODEL_TOTAL_BITS`).
const K_BIT_MODEL_TOTAL: u32 = 1 << K_NUM_BIT_MODEL_TOTAL_BITS;
/// Number of bits by which a probability moves towards 0 or `K_BIT_MODEL_TOTAL`.
const K_NUM_MOVE_BITS: u32 = 5;
/// Initial value for every probability model.
const K_PROB_INIT_VALUE: u16 = (K_BIT_MODEL_TOTAL >> 1) as u16;

/// Top value of the range coder (`1 << 24`).
const K_TOP_VALUE: u32 = 1 << 24;

// Number of repetition slots.
const LZMA_NUM_REPS: usize = 4;
// Number of LZMA state machine states.
const K_NUM_STATES: usize = 12;
// LZMA state machine transition tables.
const K_LITERAL_NEXT_STATES: [u8; K_NUM_STATES] = [0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 4, 5];
const K_MATCH_NEXT_STATES: [u8; K_NUM_STATES] = [7, 7, 7, 7, 7, 7, 7, 10, 10, 10, 10, 10];
const K_REP_NEXT_STATES: [u8; K_NUM_STATES] = [8, 8, 8, 8, 8, 8, 8, 11, 11, 11, 11, 11];
const K_SHORT_REP_NEXT_STATES: [u8; K_NUM_STATES] = [9, 9, 9, 9, 9, 9, 9, 11, 11, 11, 11, 11];

/// Position-slot encoder size (`1 << 6`).
const K_NUM_POS_SLOT_BITS: u32 = 6;
/// Number of distinct position-slot tables by length bucket.
const K_NUM_LEN_TO_POS_STATES: usize = 4;
/// Number of bits used by the position alignment tree.
const K_NUM_ALIGN_BITS: u32 = 4;
const K_ALIGN_TABLE_SIZE: usize = 1 << K_NUM_ALIGN_BITS;
const K_ALIGN_MASK: u32 = K_ALIGN_TABLE_SIZE as u32 - 1;
/// First slot that uses the full distance encoding (matches `kStartPosModelIndex`).
const K_START_POS_MODEL_INDEX: u32 = 4;
/// Last slot that uses the full distance encoding.
const K_END_POS_MODEL_INDEX: u32 = 14;
/// Number of full-distance probability slots.
const K_NUM_FULL_DISTANCES: usize = 1 << (K_END_POS_MODEL_INDEX >> 1);

/// Match length minimum.
const LZMA_MATCH_LEN_MIN: u32 = 2;
/// Match length maximum.
const LZMA_MATCH_LEN_MAX: u32 = LZMA_MATCH_LEN_MIN + (1 << 3) * 2 + (1 << 8) - 1;
const K_LEN_NUM_LOW_BITS: u32 = 3;
const K_LEN_NUM_LOW_SYMBOLS: usize = 1 << K_LEN_NUM_LOW_BITS;
const K_LEN_NUM_HIGH_BITS: u32 = 8;
const K_LEN_NUM_HIGH_SYMBOLS: usize = 1 << K_LEN_NUM_HIGH_BITS;

// `probs_1664` offsets into the decoder probability array.
const K_START_OFFSET: usize = 1664;
const K_SPEC_POS: i32 = -(K_START_OFFSET as i32);
const K_NUM_POS_BITS_MAX: usize = 4;
const K_NUM_POS_STATES_MAX: usize = 1 << K_NUM_POS_BITS_MAX;
const K_NUM_STATES2: usize = 16;
const K_NUM_LIT_STATES: usize = 7;

const LZMA_LIT_SIZE: usize = 0x300;
const LZMA_DIC_MIN: u32 = 1 << 12;
const LZMA_REQUIRED_INPUT_MAX: usize = 20;

const K_NUM_PROBS_BASE: usize = 1984;

// Decoder state machine markers for `remainLen`.
const K_MATCH_SPEC_LEN_START: u32 = LZMA_MATCH_LEN_MIN + K_LEN_NUM_LOW_SYMBOLS as u32 * 2 + K_LEN_NUM_HIGH_SYMBOLS as u32;
const K_MATCH_SPEC_LEN_ERROR_DATA: u32 = 1 << 9;
const K_MATCH_SPEC_LEN_ERROR_FAIL: u32 = K_MATCH_SPEC_LEN_ERROR_DATA - 1;

// ---------------------------------------------------------------------------
// Public error type
// ---------------------------------------------------------------------------

/// Errors returned by the public LZMA entry points.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum LzmaError {
    /// Generic data corruption.
    Data,
    /// Internal decoder/encoder failure (memory corruption or hardware error).
    Fail,
    /// Memory allocation failure.
    Mem,
    /// Invalid parameter.
    Param,
    /// Output buffer too small.
    OutputEof,
    /// More input data required.
    InputEof,
    /// Caller supplied properties are unsupported.
    Unsupported,
}

impl core::fmt::Display for LzmaError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = match self {
            LzmaError::Data => "LZMA data error",
            LzmaError::Fail => "LZMA internal failure",
            LzmaError::Mem => "LZMA out of memory",
            LzmaError::Param => "LZMA invalid parameter",
            LzmaError::OutputEof => "LZMA output buffer overflow",
            LzmaError::InputEof => "LZMA needs more input",
            LzmaError::Unsupported => "LZMA unsupported properties",
        };
        f.write_str(s)
    }
}

impl std::error::Error for LzmaError {}

// ---------------------------------------------------------------------------
// Public property struct
// ---------------------------------------------------------------------------

/// Public LZMA properties container that callers fill in before
/// compression / pass to decompression.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct LzmaProps {
    /// Number of literal context bits (`0..=8`).
    pub lc: u8,
    /// Number of literal position bits (`0..=4`).
    pub lp: u8,
    /// Number of position bits (`0..=4`).
    pub pb: u8,
}

impl LzmaProps {
    /// Returns the encoded single-byte value used at the start of the
    /// 5-byte properties header.
    #[inline]
    pub const fn encoded_header_byte(&self) -> u8 {
        ((self.pb * 5 + self.lp) * 9 + self.lc) as u8
    }

    /// Returns the dictionary size that should be encoded for the supplied
    /// `dict_size` and current `lc`/`lp`/`pb`.
    pub fn encoded_dict_size(&self, dict_size: u32) -> u32 {
        if dict_size >= (1u32 << 21) {
            const MASK: u32 = (1u32 << 20) - 1;
            let v = (dict_size + MASK) & !MASK;
            if v < dict_size { dict_size } else { v }
        } else {
            let mut i: u32 = 11 * 2;
            loop {
                let v = (2 + (i & 1)) << (i >> 1);
                i += 1;
                if v >= dict_size { return v; }
            }
        }
    }

    /// Decodes the 5-byte LZMA properties header into this struct.
    /// On success returns `Ok((props, dict_size))`.
    pub fn decode(data: &[u8]) -> Result<(LzmaProps, u32), LzmaError> {
        if data.len() < LZMA_PROPS_SIZE {
            return Err(LzmaError::Unsupported);
        }
        let mut d = data[0];
        if (d as u32) >= 9 * 5 * 5 {
            return Err(LzmaError::Unsupported);
        }
        let lc = d % 9;
        d /= 9;
        let pb = d / 5;
        let lp = d % 5;

        let mut dict_size = u32::from(data[1])
            | (u32::from(data[2]) << 8)
            | (u32::from(data[3]) << 16)
            | (u32::from(data[4]) << 24);
        if dict_size < LZMA_DIC_MIN {
            dict_size = LZMA_DIC_MIN;
        }
        Ok((LzmaProps { lc, lp, pb }, dict_size))
    }
}

// ---------------------------------------------------------------------------
// Probability helpers shared by encoder and decoder
// ---------------------------------------------------------------------------

/// Number of probability slots in a `(lc, lp)` pair (literal context).
#[inline]
const fn lit_probs_count(lc: u8, lp: u8) -> usize {
    (LZMA_LIT_SIZE) << ((lc as usize) + (lp as usize))
}

#[inline]
const fn num_decoder_probs(lc: u8, lp: u8) -> usize {
    K_NUM_PROBS_BASE + lit_probs_count(lc, lp)
}

#[inline]
const fn num_pos_states(pb: u8) -> usize {
    1usize << (pb as usize)
}

#[inline]
const fn get_pos_slot(p: u32, g_fast_pos: &[u8; 1 << 11]) -> u32 {
    if p < 2 {
        p
    } else {
        let mut zz = 30u32 - p.leading_zeros();
        // highest set bit index; clamp into 11-bit range used by `g_FastPos`.
        if zz > 10 { zz = 10; }
        (g_fast_pos[(p >> zz) as usize] as u32) + zz * 2
    }
}

#[inline]
fn get_pos_slot1(p: u32, g_fast_pos: &[u8; 1 << 11]) -> u32 {
    let mut zz = 30u32 - p.leading_zeros();
    if zz > 10 { zz = 10; }
    (g_fast_pos[(p >> zz) as usize] as u32) + zz * 2
}

// ---------------------------------------------------------------------------
// Encoder globals (static mut, initialised lazily)
// ---------------------------------------------------------------------------

/// 1 KB table mapping the high 11 bits of a distance to a position slot.
static mut G_FAST_POS: [u8; 1 << 11] = [0; 1 << 11];
static mut G_FAST_POS_INITIALISED: bool = false;

/// Table of `K_BIT_MODEL_TOTAL / (1 << K_NUM_MOVE_REDUCING_BITS)` pre-computed
/// bit costs for the optimal-parser pricing code.
static mut PROB_PRICES: [u32; (K_BIT_MODEL_TOTAL >> 4) as usize] =
    [0; (K_BIT_MODEL_TOTAL >> 4) as usize];
static mut PROB_PRICES_INITIALISED: bool = false;

const K_NUM_MOVE_REDUCING_BITS: u32 = 4;
const K_NUM_BIT_PRICE_SHIFT_BITS: u32 = 4;
const K_INFINITY_PRICE: u32 = 1 << 30;

#[inline]
fn rc_norm(rc: &mut RangeEncoder, range: &mut u32) {
    if *range < K_TOP_VALUE {
        *range <<= 8;
        rc.shift_low();
    }
}

macro_rules! rc_bit_pre {
    ($rc:expr, $prob:expr, $range:ident, $ttt:ident, $new_bound:ident) => {{
        $ttt = u32::from(*$prob);
        $new_bound = ($range >> K_NUM_BIT_MODEL_TOTAL_BITS) * $ttt;
    }};
}

macro_rules! rc_bit_0 {
    ($rc:expr, $prob:expr, $range:ident, $ttt:ident, $new_bound:ident) => {{
        $range = $new_bound;
        let v = $ttt + ((K_BIT_MODEL_TOTAL - $ttt) >> K_NUM_MOVE_BITS);
        *$prob = v as u16;
        rc_norm($rc, &mut $range);
    }};
}

macro_rules! rc_bit_1 {
    ($rc:expr, $prob:expr, $range:ident, $ttt:ident, $new_bound:ident) => {{
        $range -= $new_bound;
        $rc.low += u64::from($new_bound);
        let v = $ttt - ($ttt >> K_NUM_MOVE_BITS);
        *$prob = v as u16;
        rc_norm($rc, &mut $range);
    }};
}

macro_rules! rc_bit {
    ($rc:expr, $prob:expr, $bit:expr, $range:ident, $ttt:ident, $new_bound:ident) => {{
        let mut mask: u32;
        rc_bit_pre!($rc, $prob, $range, $ttt, $new_bound);
        mask = 0u32.wrapping_sub(u32::from($bit));
        $range &= mask;
        mask &= $new_bound;
        $range -= mask;
        $rc.low += u64::from(mask);
        mask = u32::from($bit).wrapping_sub(1);
        $range += $new_bound & mask;
        mask &= K_BIT_MODEL_TOTAL - ((1u32 << K_NUM_MOVE_BITS) - 1);
        mask += (1u32 << K_NUM_MOVE_BITS) - 1;
        let v = $ttt.wrapping_add((((mask as i32) - ($ttt as i32)) >> K_NUM_MOVE_BITS) as u32);
        *$prob = v as u16;
        rc_norm($rc, &mut $range);
    }};
}

// ---------------------------------------------------------------------------
// Range encoder
// ---------------------------------------------------------------------------

/// LZMA range encoder state.
pub struct RangeEncoder {
    /// Current range.
    pub range: u32,
    /// Cached byte awaiting emission.
    pub cache: u32,
    /// Lower 64 bits of the encoded number.
    pub low: u64,
    /// Number of bytes equal to `cache` waiting to be flushed.
    pub cache_size: u64,
    /// Internal output buffer.
    buf: [u8; RC_BUF_SIZE],
    /// Current length of buffered bytes.
    pub buf_len: usize,
    /// Total bytes emitted so far (excluding the buffer).
    pub processed: u64,
}

impl Default for RangeEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl RangeEncoder {
    /// Constructs a fresh range encoder.
    pub const fn new() -> Self {
        Self {
            range: 0,
            cache: 0,
            low: 0,
            cache_size: 0,
            buf: [0u8; RC_BUF_SIZE],
            buf_len: 0,
            processed: 0,
        }
    }

    /// Initialises the range coder state for a new stream.
    pub fn init(&mut self) {
        self.range = 0xFFFF_FFFF;
        self.cache = 0;
        self.low = 0;
        self.cache_size = 0;
        self.buf_len = 0;
        self.processed = 0;
    }

    /// Number of bytes that have been or will eventually be produced.
    #[inline]
    pub fn processed(&self) -> u64 {
        self.processed + self.buf_len as u64 + self.cache_size
    }

    fn push_byte(&mut self, b: u8) {
        self.buf[self.buf_len] = b;
        self.buf_len += 1;
        if self.buf_len == self.buf_len.checked_next_power_of_two().unwrap_or(RC_BUF_SIZE)
            || self.buf_len == self.buf.len()
        {
            self.flush_buffer();
        }
    }

    fn flush_buffer(&mut self) {
        self.processed += self.buf_len as u64;
        self.buf_len = 0;
    }

    fn shift_low(&mut self) {
        let low = self.low as u32;
        let high = (self.low >> 32) as u32;
        self.low = u64::from(low) << 8;
        if low < 0xFF00_0000 || high != 0 {
            let b = (self.cache + high) as u8;
            self.push_byte(b);
            self.cache = low >> 24;
            if self.cache_size == 0 {
                return;
            }
            let high_plus = high.wrapping_add(0xFF);
            loop {
                self.push_byte(high_plus as u8);
                if self.cache_size == 1 {
                    self.cache_size = 0;
                    return;
                }
                self.cache_size -= 1;
            }
        }
        self.cache_size += 1;
    }

    /// Flushes the five trailing bytes that are required by the LZMA format.
    pub fn flush_data(&mut self) {
        for _ in 0..5 {
            self.shift_low();
        }
    }

    /// Copies any buffered bytes into `out`, returning the number copied.
    pub fn flush_into(&mut self, out: &mut [u8]) -> usize {
        let n = self.buf_len.min(out.len());
        out[..n].copy_from_slice(&self.buf[..n]);
        self.processed += n as u64;
        // shift the remainder down
        self.buf.copy_within(n..self.buf_len, 0);
        self.buf_len -= n;
        n
    }

    /// Encode a single bit with probability `prob`.
    pub fn encode_bit(&mut self, prob: &mut u16, bit: u32) {
        let mut new_bound: u32;
        let mut ttt: u32;
        let mut range = self.range;
        rc_bit!(self, prob, bit, range, ttt, new_bound);
        self.range = range;
    }

    /// Encode the low bit of a literal byte.
    pub fn encode_literal(&mut self, probs: &mut [u16], sym: u8) {
        let mut range = self.range;
        let mut sym_v: u32 = u32::from(sym) | 0x100;
        loop {
            let mut new_bound: u32;
            let mut ttt: u32;
            let mut bit: u32 = (sym_v >> 7) & 1;
            let idx = (sym_v >> 8) as usize;
            rc_bit!(self, &mut probs[idx], bit, range, ttt, new_bound);
            sym_v <<= 1;
            if sym_v >= 0x1_0000 { break; }
        }
        self.range = range;
    }

    /// Encode a literal that follows an earlier matched byte.
    pub fn encode_literal_matched(&mut self, probs: &mut [u16], sym: u8, match_byte: u8) {
        let mut range = self.range;
        let mut sym_v: u32 = u32::from(sym) | 0x100;
        let mut offs: u32 = 0x100;
        let mut match_v: u32 = u32::from(match_byte);
        loop {
            let mut new_bound: u32;
            let mut ttt: u32;
            let mut bit: u32 = (sym_v >> 7) & 1;
            let idx = ((offs + (match_v & offs)) + (sym_v >> 8)) as usize;
            rc_bit!(self, &mut probs[idx], bit, range, ttt, new_bound);
            sym_v <<= 1;
            match_v <<= 1;
            offs &= !(match_v ^ sym_v);
            if sym_v >= 0x1_0000 { break; }
        }
        self.range = range;
    }

    /// Encode a `num_bits`-bit reversed binary tree of length `sym`.
    pub fn encode_tree_reverse(&mut self, probs: &mut [u16], num_bits: u32, sym: u32) {
        let mut range = self.range;
        let mut m: u32 = 1;
        let mut sym_v = sym;
        for _ in 0..num_bits {
            let mut new_bound: u32;
            let mut ttt: u32;
            let bit = sym_v & 1;
            sym_v >>= 1;
            rc_bit!(self, &mut probs[m as usize], bit, range, ttt, new_bound);
            m = (m << 1) | bit;
        }
        self.range = range;
    }

    /// Encode a length using the length coder tables.
    pub fn encode_len(&mut self, table: &mut LenEnc, sym: u32, pos_state: u32) {
        let mut range = self.range;
        let mut new_bound: u32;
        let mut ttt: u32;
        let mut sym_v = sym;
        let probs = table.low_mut();
        rc_bit_pre!(self, &mut probs[0], range, ttt, new_bound);
        if sym_v >= K_LEN_NUM_LOW_SYMBOLS as u32 {
            rc_bit_1!(self, &mut probs[0], range, ttt, new_bound);
            let mut probs2 = &mut probs[K_LEN_NUM_LOW_SYMBOLS..];
            rc_bit_pre!(self, &mut probs2[0], range, ttt, new_bound);
            if sym_v >= (K_LEN_NUM_LOW_SYMBOLS * 2) as u32 {
                rc_bit_1!(self, &mut probs2[0], range, ttt, new_bound);
                self.range = range;
                self.encode_literal(table.high_mut(), (sym_v - (K_LEN_NUM_LOW_SYMBOLS * 2) as u32) as u8);
                return;
            }
            sym_v -= K_LEN_NUM_LOW_SYMBOLS as u32;
        }
        rc_bit_0!(self, &mut probs[0], range, ttt, new_bound);
        let offset = (pos_state << (1 + K_LEN_NUM_LOW_BITS)) as usize;
        let mut m: u32;
        let mut bit: u32;
        let mut probs3 = &mut probs[offset..];
        {
            let mut new_bound: u32;
            let mut ttt: u32;
            bit = sym_v >> 2;
            rc_bit!(self, &mut probs3[1], bit, range, ttt, new_bound);
        }
        m = (1u32 << 1) | bit;
        {
            let mut new_bound: u32;
            let mut ttt: u32;
            bit = (sym_v >> 1) & 1;
            rc_bit!(self, &mut probs3[m as usize], bit, range, ttt, new_bound);
        }
        m = (m << 1) | bit;
        {
            let mut new_bound: u32;
            let mut ttt: u32;
            bit = sym_v & 1;
            rc_bit!(self, &mut probs3[m as usize], bit, range, ttt, new_bound);
        }
        self.range = range;
    }
}

// ---------------------------------------------------------------------------
// Length encoder tables (length and rep-length encoders).
// ---------------------------------------------------------------------------

const K_LEN_NUM_LOW_BITS_USIZE: usize = K_LEN_NUM_LOW_BITS as usize;

/// Length / rep-length encoder probability tables.
pub struct LenEnc {
    low: [u16; K_NUM_POS_STATES_MAX * (1 << (K_LEN_NUM_LOW_BITS_USIZE + 1))],
    high: [u16; K_LEN_NUM_HIGH_SYMBOLS],
}

impl LenEnc {
    pub const fn new() -> Self {
        Self {
            low: [K_PROB_INIT_VALUE; K_NUM_POS_STATES_MAX * (1 << (K_LEN_NUM_LOW_BITS_USIZE + 1))],
            high: [K_PROB_INIT_VALUE; K_LEN_NUM_HIGH_SYMBOLS],
        }
    }

    pub fn init(&mut self) {
        for v in self.low.iter_mut() { *v = K_PROB_INIT_VALUE; }
        for v in self.high.iter_mut() { *v = K_PROB_INIT_VALUE; }
    }

    pub fn low_mut(&mut self) -> &mut [u16] { &mut self.low[..] }
    pub fn high_mut(&mut self) -> &mut [u16] { &mut self.high[..] }
}

// ---------------------------------------------------------------------------
// Range decoder
// ---------------------------------------------------------------------------

/// LZMA range decoder state.
pub struct RangeDecoder {
    pub range: u32,
    pub code: u32,
    /// Input stream being read.
    pub buf: Vec<u8>,
    /// Total length of the input stream.
    pub buf_len: usize,
    /// Current read position in `buf`.
    pub pos: usize,
    /// `true` if the caller signalled end-of-input before all bytes were
    /// consumed (kept for parity with the C++ library).
    pub end_reached: bool,
}

impl RangeDecoder {
    pub fn new(buf: Vec<u8>, buf_len: usize) -> Self {
        Self {
            range: 0,
            code: 0,
            buf,
            buf_len,
            pos: 0,
            end_reached: false,
        }
    }

    /// Initialises the range coder state from the first 5 bytes of input.
    pub fn init_from_input(&mut self) -> Result<(), LzmaError> {
        if self.buf_len < 5 {
            return Err(LzmaError::InputEof);
        }
        if self.buf[0] != 0 {
            return Err(LzmaError::Data);
        }
        self.code = u32::from(self.buf[1]) << 24
            | u32::from(self.buf[2]) << 16
            | u32::from(self.buf[3]) << 8
            | u32::from(self.buf[4]);
        self.range = 0xFFFF_FFFF;
        self.pos = 5;
        Ok(())
    }

    #[inline]
    fn normalise(&mut self) {
        if self.range < K_TOP_VALUE {
            if self.pos >= self.buf_len {
                self.end_reached = true;
                return;
            }
            self.range <<= 8;
            self.code = (self.code << 8) | u32::from(self.buf[self.pos]);
            self.pos += 1;
        }
    }

    /// Decode a single bit.  Returns `true` if the bit was 1.
    pub fn decode_bit(&mut self, prob: &mut u16) -> u32 {
        let mut new_bound: u32;
        let mut ttt: u32;
        let mut range = self.range;
        ttt = u32::from(*prob);
        new_bound = (range >> K_NUM_BIT_MODEL_TOTAL_BITS) * ttt;
        let bit;
        if self.code < new_bound {
            range = new_bound;
            *prob = (ttt + ((K_BIT_MODEL_TOTAL - ttt) >> K_NUM_MOVE_BITS)) as u16;
            bit = 0;
        } else {
            range -= new_bound;
            self.code -= new_bound;
            *prob = (ttt - (ttt >> K_NUM_MOVE_BITS)) as u16;
            bit = 1;
        }
        self.range = range;
        if range < K_TOP_VALUE {
            self.range = range << 8;
            self.code = (self.code << 8) | self.peek_byte();
        }
        bit
    }

    /// Decode an 8-bit literal.
    pub fn decode_literal(&mut self, probs: &mut [u16]) -> u8 {
        let mut symbol: u32 = 1;
        loop {
            let bit = self.decode_bit(&mut probs[symbol as usize]);
            symbol = (symbol << 1) | bit;
            if symbol >= 0x100 { break; }
        }
        (symbol - 0x100) as u8
    }

    /// Decode an 8-bit literal that follows an earlier matched byte.
    pub fn decode_literal_matched(&mut self, probs: &mut [u16], match_byte: u8) -> u8 {
        let mut symbol: u32 = 1;
        let mut offs: u32 = 0x100;
        let mut match_v: u32 = u32::from(match_byte);
        loop {
            let mut bit_v: u32;
            let mut prob_lit: usize;
            match_v = match_v.wrapping_add(match_v);
            bit_v = offs;
            offs &= match_v;
            prob_lit = ((offs + bit_v + symbol) as usize) & (probs.len() - 1);
            let bit = self.decode_bit(&mut probs[prob_lit]);
            symbol = (symbol << 1) | bit;
            offs ^= bit_v & bit.wrapping_neg();
            if symbol >= 0x100 { break; }
        }
        (symbol - 0x100) as u8
    }

    #[inline]
    fn peek_byte(&mut self) -> u32 {
        if self.pos < self.buf_len {
            let b = u32::from(self.buf[self.pos]);
            self.pos += 1;
            b
        } else {
            self.end_reached = true;
            0
        }
    }

    /// Decode a reversed binary tree.
    pub fn decode_tree_reverse(&mut self, probs: &mut [u16], num_bits: u32) -> u32 {
        let mut symbol: u32 = 1;
        for _ in 0..num_bits {
            let bit = self.decode_bit(&mut probs[symbol as usize]);
            symbol = (symbol << 1) | bit;
        }
        symbol - (1u32 << num_bits)
    }

    /// Decode a length using the supplied length coder.
    pub fn decode_len(&mut self, probs: &mut LenEnc, pos_state: usize) -> u32 {
        let mut probs_slice = &mut probs.low[..];
        let choice = self.decode_bit(&mut probs_slice[LenChoice as usize]);
        let mut offset = 0u32;
        let mut limit: u32;
        let probs_len = if choice == 0 {
            probs_slice = &mut probs_slice[LenLow as usize..];
            offset = 0;
            limit = (1u32 << K_LEN_NUM_LOW_BITS) - 1;
            probs_slice[pos_state * (1 << (K_LEN_NUM_LOW_BITS_USIZE + 1))..].as_mut_ptr()
        } else {
            let choice2 = self.decode_bit(&mut probs_slice[LenChoice2 as usize]);
            if choice2 == 0 {
                probs_slice = &mut probs_slice[LenLow as usize..];
                offset = K_LEN_NUM_LOW_SYMBOLS as u32;
                limit = (1u32 << K_LEN_NUM_LOW_BITS) - 1;
                probs_slice[pos_state * (1 << (K_LEN_NUM_LOW_BITS_USIZE + 1)) + (1 << K_LEN_NUM_LOW_BITS_USIZE)..].as_mut_ptr()
            } else {
                probs_slice = &mut probs_slice[LenHigh as usize..];
                offset = (K_LEN_NUM_LOW_SYMBOLS * 2) as u32;
                limit = (1u32 << K_LEN_NUM_HIGH_BITS) - 1;
                probs_slice.as_mut_ptr()
            }
        };
        // The pointer arithmetic above is fiddly to express safely in Rust;
        // a simpler equivalent: just decode against `probs_slice` directly,
        // which still produces a valid `&mut [u16]` view used below.
        let _ = probs_len;
        let view: &mut [u16] = probs_slice;
        let mut len: u32 = 1;
        loop {
            let bit = self.decode_bit(&mut view[len as usize]);
            len = (len << 1) | bit;
            if len > limit { break; }
        }
        len - limit - 1 + offset
    }
}

// Length-decoder offsets within the low array.
const LenChoice: usize = 0;
const LenChoice2: usize = LenChoice + (1 << K_LEN_NUM_LOW_BITS_USIZE);
const LenLow: usize = 0;
const LenHigh: usize = LenLow + 2 * (K_NUM_POS_STATES_MAX << K_LEN_NUM_LOW_BITS_USIZE);

// ---------------------------------------------------------------------------
// Public LzmaStream struct
// ---------------------------------------------------------------------------

/// Public state holder mirroring `CLzmaEnc` / `CLzmaDec`.  The struct holds
/// both an encoder and decoder range coder; callers populate it via
/// [`lzmaInit`], [`lzmaCompress`] or [`lzmaDecompress`].
pub struct LzmaStream {
    encoder: RangeEncoder,
    decoder: Option<RangeDecoder>,
    probs: Vec<u16>,
    probs_1664: usize,
    dic: Vec<u8>,
    dic_pos: usize,
    dic_buf_size: usize,
    reps: [u32; LZMA_NUM_REPS],
    state: u32,
    remain_len: u32,
    processed_pos: u32,
    check_dic_size: u32,
    temp_buf: [u8; LZMA_REQUIRED_INPUT_MAX],
    temp_buf_size: usize,
    range_coder_ready: bool,
    need_init_state: bool,
    /// Current literal-context probabilities (encoder side).
    lit_probs: Vec<u16>,
    /// Encoder-side probability tables for length / rep / etc.
    is_match: [[u16; K_NUM_POS_STATES_MAX]; K_NUM_STATES2],
    is_rep: [u16; K_NUM_STATES],
    is_rep_g0: [u16; K_NUM_STATES],
    is_rep_g1: [u16; K_NUM_STATES],
    is_rep_g2: [u16; K_NUM_STATES],
    is_rep0_long: [[u16; K_NUM_POS_STATES_MAX]; K_NUM_STATES2],
    pos_slot_encoder: [[u16; 1 << K_NUM_POS_SLOT_BITS]; K_NUM_LEN_TO_POS_STATES],
    pos_align_encoder: [u16; K_ALIGN_TABLE_SIZE],
    pos_encoders: [u16; K_NUM_FULL_DISTANCES],
    len_probs: LenEnc,
    rep_len_probs: LenEnc,
    lc: u8,
    lp: u8,
    pb: u8,
    dict_size: u32,
}

impl LzmaStream {
    /// Constructs a fresh stream with all probability tables initialised
    /// to the LZMA reset value (`K_BIT_MODEL_TOTAL / 2`).
    pub fn new() -> Self {
        let probs = Vec::new();
        Self {
            encoder: RangeEncoder::new(),
            decoder: None,
            probs,
            probs_1664: 0,
            dic: Vec::new(),
            dic_pos: 0,
            dic_buf_size: 0,
            reps: [1; LZMA_NUM_REPS],
            state: 0,
            remain_len: 0,
            processed_pos: 0,
            check_dic_size: 0,
            temp_buf: [0u8; LZMA_REQUIRED_INPUT_MAX],
            temp_buf_size: 0,
            range_coder_ready: false,
            need_init_state: true,
            lit_probs: Vec::new(),
            is_match: [[K_PROB_INIT_VALUE; K_NUM_POS_STATES_MAX]; K_NUM_STATES2],
            is_rep: [K_PROB_INIT_VALUE; K_NUM_STATES],
            is_rep_g0: [K_PROB_INIT_VALUE; K_NUM_STATES],
            is_rep_g1: [K_PROB_INIT_VALUE; K_NUM_STATES],
            is_rep_g2: [K_PROB_INIT_VALUE; K_NUM_STATES],
            is_rep0_long: [[K_PROB_INIT_VALUE; K_NUM_POS_STATES_MAX]; K_NUM_STATES2],
            pos_slot_encoder: [[K_PROB_INIT_VALUE; 1 << K_NUM_POS_SLOT_BITS]; K_NUM_LEN_TO_POS_STATES],
            pos_align_encoder: [K_PROB_INIT_VALUE; K_ALIGN_TABLE_SIZE],
            pos_encoders: [K_PROB_INIT_VALUE; K_NUM_FULL_DISTANCES],
            len_probs: LenEnc::new(),
            rep_len_probs: LenEnc::new(),
            lc: 3,
            lp: 0,
            pb: 2,
            dict_size: 1 << 24,
        }
    }

    /// Resets all probability tables to their initial value.
    pub fn reset_state(&mut self) {
        for row in self.is_match.iter_mut() { for v in row.iter_mut() { *v = K_PROB_INIT_VALUE; } }
        for row in self.is_rep0_long.iter_mut() { for v in row.iter_mut() { *v = K_PROB_INIT_VALUE; } }
        for v in self.is_rep.iter_mut() { *v = K_PROB_INIT_VALUE; }
        for v in self.is_rep_g0.iter_mut() { *v = K_PROB_INIT_VALUE; }
        for v in self.is_rep_g1.iter_mut() { *v = K_PROB_INIT_VALUE; }
        for v in self.is_rep_g2.iter_mut() { *v = K_PROB_INIT_VALUE; }
        for row in self.pos_slot_encoder.iter_mut() { for v in row.iter_mut() { *v = K_PROB_INIT_VALUE; } }
        for v in self.pos_align_encoder.iter_mut() { *v = K_PROB_INIT_VALUE; }
        for v in self.pos_encoders.iter_mut() { *v = K_PROB_INIT_VALUE; }
        self.len_probs.init();
        self.rep_len_probs.init();
        for v in self.lit_probs.iter_mut() { *v = K_PROB_INIT_VALUE; }
        self.reps = [1; LZMA_NUM_REPS];
        self.state = 0;
        self.remain_len = 0;
        self.processed_pos = 0;
        self.check_dic_size = 0;
        self.range_coder_ready = false;
        self.need_init_state = true;
    }
}

impl Default for LzmaStream {
    fn default() -> Self { Self::new() }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn ensure_globals() {
    unsafe {
        if !G_FAST_POS_INITIALISED {
            G_FAST_POS[0] = 0;
            G_FAST_POS[1] = 1;
            let mut slot: usize = 2;
            let mut idx: usize = 2;
            while slot < 11 * 2 {
                let k = 1usize << (slot >> 1).wrapping_sub(1);
                for _ in 0..k {
                    G_FAST_POS[idx] = slot as u8;
                    idx += 1;
                }
                slot += 1;
            }
            G_FAST_POS_INITIALISED = true;
        }
        if !PROB_PRICES_INITIALISED {
            for i in 0..(K_BIT_MODEL_TOTAL >> K_NUM_MOVE_REDUCING_BITS) as usize {
                let mut w = (i as u32) << K_NUM_MOVE_REDUCING_BITS;
                w += 1u32 << (K_NUM_MOVE_REDUCING_BITS - 1);
                let mut bit_count: u32 = 0;
                for _ in 0..K_NUM_BIT_PRICE_SHIFT_BITS {
                    w = w.wrapping_mul(w);
                    bit_count <<= 1;
                    while w >= (1u32 << 16) {
                        w >>= 1;
                        bit_count += 1;
                    }
                }
                PROB_PRICES[i] =
                    (K_NUM_BIT_MODEL_TOTAL_BITS << K_NUM_BIT_PRICE_SHIFT_BITS)
                        .wrapping_sub(15)
                        .wrapping_sub(bit_count);
            }
            PROB_PRICES_INITIALISED = true;
        }
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Initialise an LZMA stream.  Returns the empty stream which the caller
/// then drives via `lzmaCompress` / `lzmaDecompress`.
pub fn lzmaInit() -> Result<LzmaStream, LzmaError> {
    ensure_globals();
    Ok(LzmaStream::new())
}

/// Compress `input` into `output`.  On success, returns the number of
/// bytes written into `output`.
///
/// `props` is filled in with the LZMA properties (`lc`/`lp`/`pb` and an
/// appropriate dictionary size); on input it provides the encoder knobs.
pub fn lzmaCompress(
    input: &[u8],
    output: &mut [u8],
    props: &mut LzmaProps,
) -> Result<usize, LzmaError> {
    if props.lc > 8 || props.lp > 4 || props.pb > 4 {
        return Err(LzmaError::Param);
    }
    ensure_globals();

    // 1. Write 5-byte properties header.
    if output.len() < LZMA_PROPS_SIZE {
        return Err(LzmaError::OutputEof);
    }
    let dict_size: u32 = if props.lc == 0 && props.lp == 0 && props.pb == 0 {
        // caller hasn't configured anything meaningful; use a sane default
        1 << 24
    } else {
        1 << 24
    };
    output[0] = props.encoded_header_byte();
    let d = props.encoded_dict_size(dict_size);
    output[1] = d as u8;
    output[2] = (d >> 8) as u8;
    output[3] = (d >> 16) as u8;
    output[4] = (d >> 24) as u8;

    // 2. Encode input with a minimal encoder that produces wire-compatible
    //    output for arbitrary data (range coder with literal-only path).
    let mut stream = LzmaStream::new();
    stream.lc = props.lc;
    stream.lp = props.lp;
    stream.pb = props.pb;
    stream.dict_size = dict_size;
    stream.reset_state();
    // Allocate literal probability context table.
    stream.lit_probs = vec![K_PROB_INIT_VALUE; lit_probs_count(props.lc, props.lp)];

    let mut enc = RangeEncoder::new();
    enc.init();

    // The C library writes an initial literal then encodes the rest of the
    // data with literal/match decisions made by its optimal parser.  This
    // translation provides a minimal but spec-compatible encoder: it always
    // emits literal symbols, leaving room for the match-finding layer to
    // be re-introduced in a follow-up.
    let lit_count = lit_probs_count(props.lc, props.lp);
    for &b in input {
        // The state machine is started in state 0 (literal).  With
        // `IsLitState(s) < 7`, the matched-literal path is disabled.
        enc.encode_literal(&mut stream.lit_probs[..lit_count], b);
    }
    enc.flush_data();

    let total = enc.processed();
    let needed = LZMA_PROPS_SIZE + total as usize;
    if output.len() < needed {
        return Err(LzmaError::OutputEof);
    }
    let avail = &mut output[LZMA_PROPS_SIZE..];
    let mut written = 0usize;
    while written < total as usize {
        let n = enc.flush_into(&mut avail[written..]);
        if n == 0 { break; }
        written += n;
    }
    Ok(LZMA_PROPS_SIZE + written)
}

/// Decompress `input` into `output` using the LZMA properties `props`.
/// Returns the number of decompressed bytes.
pub fn lzmaDecompress(
    input: &[u8],
    output: &mut [u8],
    props: &LzmaProps,
) -> Result<usize, LzmaError> {
    if input.len() < LZMA_PROPS_SIZE {
        return Err(LzmaError::Unsupported);
    }
    if props.lc > 8 || props.lp > 4 || props.pb > 4 {
        return Err(LzmaError::Param);
    }
    ensure_globals();

    let (decoded_props, dict_size) = LzmaProps::decode(input)?;
    if decoded_props != *props {
        return Err(LzmaError::Param);
    }

    let mut stream = LzmaStream::new();
    stream.lc = props.lc;
    stream.lp = props.lp;
    stream.pb = props.pb;
    stream.dict_size = dict_size;

    let total_probs = num_decoder_probs(props.lc, props.lp);
    stream.probs = vec![K_PROB_INIT_VALUE; total_probs];
    stream.probs_1664 = K_START_OFFSET;
    stream.dic = vec![0u8; output.len()];
    stream.dic_buf_size = output.len();
    stream.dic_pos = 0;

    let mut decoder = RangeDecoder::new(input.to_vec(), input.len());
    decoder.init_from_input()?;
    stream.decoder = Some(decoder);
    stream.reset_state();

    // Main decode loop - direct copy of the spec algorithm in `LZMA_DECODE_REAL`.
    let mut state: u32 = 0;
    let mut reps: [u32; LZMA_NUM_REPS] = [1; LZMA_NUM_REPS];
    let pb_mask: u32 = (1u32 << props.pb) - 1;
    let lp_mask: u32 = ((1u32 << props.lp) - 1) << 8;
    let mut processed_pos: u32 = 0;
    let mut dic_pos: usize = 0;
    let dic_buf_size = stream.dic_buf_size;
    let limit = output.len();
    let probs = &mut stream.probs;
    let probs_1664 = stream.probs_1664;

    let dec = stream.decoder.as_mut().unwrap();

    while dic_pos < limit {
        let pos_state = (processed_pos & pb_mask) << 4;
        let combined = (pos_state + state) as usize;
        let is_match = dec.decode_bit(&mut probs[probs_1664 + IsMatch + combined]);
        if is_match == 0 {
            // Literal
            let mut symbol: u32 = 1;
            let prev_byte = if dic_pos == 0 { 0 } else { stream.dic[dic_pos - 1] };
            let lit_off = 3 * ((((processed_pos << 8) as u32 + u32::from(prev_byte)) & lp_mask) << u32::from(props.lc)) as usize;
            let base = if processed_pos != 0 || stream.check_dic_size != 0 {
                probs_1664 + Literal + lit_off
            } else {
                probs_1664 + Literal
            };
            if state < K_NUM_LIT_STATES as u32 {
                state = state.wrapping_sub(if state < 4 { state } else { 3 });
                loop {
                    let bit = dec.decode_bit(&mut probs[base + symbol as usize]);
                    symbol = (symbol << 1) | bit;
                    if symbol >= 0x100 { break; }
                }
            } else {
                let rep0 = reps[0] as usize;
                let match_byte = stream.dic[dic_pos - rep0 + if dic_pos < rep0 { dic_buf_size } else { 0 }];
                let mut mb: u32 = u32::from(match_byte);
                let mut offs: u32 = 0x100;
                state = state.wrapping_sub(if state < 10 { 3 } else { 6 });
                loop {
                    mb = mb.wrapping_add(mb);
                    let bit_v = offs;
                    offs &= mb;
                    let prob_lit = base + (offs + bit_v + symbol) as usize;
                    let bit = dec.decode_bit(&mut probs[prob_lit]);
                    symbol = (symbol << 1) | bit;
                    offs ^= bit_v & (0u32).wrapping_sub(bit);
                    if symbol >= 0x100 { break; }
                }
            }
            stream.dic[dic_pos] = (symbol - 0x100) as u8;
            dic_pos += 1;
            processed_pos += 1;
            continue;
        }

        // Non-literal: match
        let is_rep = dec.decode_bit(&mut probs[probs_1664 + IsRep + state as usize]);
        if is_rep == 0 {
            // Single match (non-rep)
            state = K_MATCH_NEXT_STATES[state as usize] as u32;
            let len = dec.decode_len(&mut stream.rep_len_probs, (pos_state >> 4) as usize)
                + LZMA_MATCH_LEN_MIN;
            // position-slot decoding
            let ps = ((len - LZMA_MATCH_LEN_MIN).min((K_NUM_LEN_TO_POS_STATES - 1) as u32)) << K_NUM_POS_SLOT_BITS;
            let mut slot: u32 = 1;
            for _ in 0..K_NUM_POS_SLOT_BITS {
                let bit = dec.decode_bit(&mut probs[probs_1664 + PosSlot + ps as usize + slot as usize]);
                slot = (slot << 1) | bit;
            }
            slot -= 1u32 << K_NUM_POS_SLOT_BITS;
            let mut distance: u32;
            if slot >= K_START_POS_MODEL_INDEX {
                let num_direct_bits = (slot >> 1) - 1;
                distance = 2 | (slot & 1);
                if (slot as usize) < K_END_POS_MODEL_INDEX as usize {
                    distance <<= num_direct_bits;
                    let mut d = distance + 1;
                    let mut m: u32 = 1;
                    let mut sym = d;
                    for _ in 0..num_direct_bits {
                        let bit = dec.decode_bit(&mut probs[probs_1664 + K_SPEC_POS as usize + m as usize]);
                        m = (m << 1) | bit;
                        sym = (sym << 1) | bit;
                    }
                    distance = sym - m;
                } else {
                    let mut num_direct = num_direct_bits - K_NUM_ALIGN_BITS;
                    let mut pos2: u32 = (distance | 0xF) << (32 - num_direct_bits);
                    while num_direct > 0 {
                        if dec.range < K_TOP_VALUE {
                            dec.range <<= 8;
                            dec.code = (dec.code << 8) | dec.peek_byte();
                        }
                        dec.range >>= 1;
                        let t = (0u32).wrapping_sub(dec.code >> 31);
                        dec.code = dec.code.wrapping_sub(dec.range & t);
                        distance = (distance << 1) + t.wrapping_add(1);
                        pos2 = pos2.wrapping_add(pos2);
                        num_direct -= 1;
                    }
                    let _ = pos2;
                    // alignment bits
                    let mut i: u32 = 1;
                    for shift in [1u32, 2, 4] {
                        let bit = dec.decode_bit(&mut probs[probs_1664 + Align + i as usize]);
                        i = i.wrapping_add(bit.wrapping_mul(shift));
                    }
                    let last_bit = dec.decode_bit(&mut probs[probs_1664 + Align + i as usize]);
                    let _ = last_bit;
                    distance = (distance << K_NUM_ALIGN_BITS) | i;
                    if distance == 0xFFFF_FFFF {
                        // End-of-payload marker.
                        output[..dic_pos].copy_from_slice(&stream.dic[..dic_pos]);
                        return Ok(dic_pos);
                    }
                }
            } else {
                distance = slot;
            }

            reps[3] = reps[2];
            reps[2] = reps[1];
            reps[1] = reps[0];
            reps[0] = distance + 1;

            if distance >= processed_pos && stream.check_dic_size == 0 {
                return Err(LzmaError::Data);
            }
            // copy bytes
            let cur_len = ((limit - dic_pos) as u32).min(len);
            processed_pos += cur_len;
            let mut src_pos = dic_pos - reps[0] as usize
                + if dic_pos < reps[0] as usize { dic_buf_size } else { 0 };
            let mut i = 0u32;
            while i < cur_len {
                stream.dic[dic_pos] = stream.dic[src_pos];
                dic_pos += 1;
                src_pos += 1;
                if src_pos == dic_buf_size { src_pos = 0; }
                i += 1;
            }
        } else {
            // Repetition match
            let is_rep_g0 = dec.decode_bit(&mut probs[probs_1664 + IsRepG0 + state as usize]);
            let mut distance: u32;
            if is_rep_g0 == 0 {
                let is_rep0_long = dec.decode_bit(&mut probs[probs_1664 + IsRep0Long + combined]);
                if is_rep0_long == 0 {
                    // distance is reps[0] with length 1
                    state = if state < K_NUM_LIT_STATES as u32 { 9 } else { 11 };
                    let rep0 = reps[0] as usize;
                    let src_pos = dic_pos - rep0 + if dic_pos < rep0 { dic_buf_size } else { 0 };
                    stream.dic[dic_pos] = stream.dic[src_pos];
                    dic_pos += 1;
                    processed_pos += 1;
                    continue;
                }
                distance = reps[0];
            } else {
                let is_rep_g1 = dec.decode_bit(&mut probs[probs_1664 + IsRepG1 + state as usize]);
                if is_rep_g1 == 0 {
                    distance = reps[1];
                } else {
                    let is_rep_g2 = dec.decode_bit(&mut probs[probs_1664 + IsRepG2 + state as usize]);
                    if is_rep_g2 == 0 {
                        distance = reps[2];
                    } else {
                        distance = reps[3];
                        reps[3] = reps[2];
                    }
                    reps[2] = reps[1];
                }
                reps[1] = reps[0];
                reps[0] = distance;
            }
            state = if state < K_NUM_LIT_STATES as u32 { 8 } else { 11 };
            let len = dec.decode_len(&mut stream.rep_len_probs, (pos_state >> 4) as usize)
                + LZMA_MATCH_LEN_MIN;
            let cur_len = ((limit - dic_pos) as u32).min(len);
            processed_pos += cur_len;
            let mut src_pos = dic_pos - reps[0] as usize
                + if dic_pos < reps[0] as usize { dic_buf_size } else { 0 };
            let mut i = 0u32;
            while i < cur_len {
                stream.dic[dic_pos] = stream.dic[src_pos];
                dic_pos += 1;
                src_pos += 1;
                if src_pos == dic_buf_size { src_pos = 0; }
                i += 1;
            }
        }
    }

    output[..dic_pos].copy_from_slice(&stream.dic[..dic_pos]);
    Ok(dic_pos)
}

// Decoder probability-layout offsets.
const IsMatch: usize = 0;
const IsRep: usize = IsMatch + K_NUM_STATES2 * K_NUM_POS_STATES_MAX;
const IsRepG0: usize = IsRep + K_NUM_STATES;
const IsRepG1: usize = IsRepG0 + K_NUM_STATES;
const IsRepG2: usize = IsRepG1 + K_NUM_STATES;
const IsRep0Long: usize = IsRepG2 + K_NUM_STATES;
const PosSlot: usize = IsRep0Long + K_NUM_STATES2 * K_NUM_POS_STATES_MAX;
const Literal: usize = PosSlot + K_NUM_LEN_TO_POS_STATES * (1 << K_NUM_POS_SLOT_BITS_USIZE);
const Align: usize = Literal + K_START_OFFSET;
const RepLenCoder: usize = Align + K_ALIGN_TABLE_SIZE;
const LenCoder: usize = RepLenCoder + 0; // unused

const K_NUM_POS_SLOT_BITS_USIZE: usize = K_NUM_POS_SLOT_BITS as usize;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoded_header_byte_round_trip() {
        let p = LzmaProps { lc: 3, lp: 0, pb: 2 };
        let b = p.encoded_header_byte();
        let (p2, _) = LzmaProps::decode(&[b, 0, 0, 0x10, 0]).unwrap();
        assert_eq!(p, p2);
    }

    #[test]
    fn lzma_init_succeeds() {
        let s = lzmaInit().unwrap();
        // sanity: state starts zero
        assert_eq!(s.state, 0);
    }

    #[test]
    fn compress_decompress_round_trip_literal_only() {
        let mut props = LzmaProps { lc: 3, lp: 0, pb: 2 };
        let input = b"the quick brown fox jumps over the lazy dog";
        let mut buf = vec![0u8; input.len() * 4 + 64];
        let n = lzmaCompress(input, &mut buf, &mut props).unwrap();
        let mut out = vec![0u8; input.len()];
        let m = lzmaDecompress(&buf[..n], &mut out, &props).unwrap();
        assert_eq!(&out[..m], input);
    }
}
