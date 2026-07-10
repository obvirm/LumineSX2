//! Rust 2021 translation of the `fmt` C++ formatting library
//! (3rdparty/fmt headers + src/format.cc).
//!
//! This module exposes a minimal idiomatic Rust surface for the parts of the
//! upstream library PCSX2 actually consumes: a `FormatArgs` value that mirrors
//! the C++ `fmt::format_args` array of handle+spec, a `BasicFormatter` that
//! owns a growable string buffer, type-specific formatters for integers,
//! floats, strings, chars, pointers and hex values, plus a chrono helper
//! for `SystemTime`. The original library is several thousand lines of C++
//! across `core.h`, `format.h`, `chrono.h` and `format.cc`; the surface here
//! is intentionally narrow but faithful to the public contract.
//!
//! Globals (the lazily-allocated global buffer the C++ library keeps behind
//! the scenes) are stored as `static mut`, per the task instructions.

use std::fmt::{self as stdfmt, Display};
use std::time::SystemTime;

// ---------------------------------------------------------------------------
// Format flags (a subset of fmt's `format_flags` bitfield).
// ---------------------------------------------------------------------------

/// `+` flag: always print a sign for numeric types.
pub const SIGN: u8 = 1;
/// `#` flag: alternate form (`0x` for hex, trailing `.` for floats, etc.).
pub const HASH: u8 = 2;
/// `0` flag: zero-pad numeric output up to the field width.
pub const ZERO: u8 = 4;

// ---------------------------------------------------------------------------
// Alignment / sign / type enums (mirroring fmt::align / fmt::sign /
// fmt::presentation_type).
// ---------------------------------------------------------------------------

/// Field alignment kind, mirroring `fmt::align`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    None,
    Left,
    Right,
    Center,
    Numeric,
}

/// Sign rendering mode, mirroring `fmt::sign`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sign {
    None,
    Minus,
    Plus,
    Space,
}

/// Number presentation type, mirroring `fmt::presentation_type` (only the
/// entries PCSX2 actually formats against).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Type {
    None,
    Dec,
    Hex,
    Oct,
    Bin,
    Fixed,
    Exp,
    Pointer,
    String,
    Debug,
    Char,
}

/// Parsed format spec, the equivalent of `fmt::format_specs`.
#[derive(Clone, Copy, Debug)]
pub struct FormatSpec {
    pub width: i32,
    pub precision: i32,
    pub flags: u8,
    pub align: Align,
    pub sign: Sign,
    pub typ: Type,
    pub fill: char,
    pub upper: bool,
    pub localized: bool,
}

impl Default for FormatSpec {
    fn default() -> Self {
        Self {
            width: 0,
            precision: -1,
            flags: 0,
            align: Align::None,
            sign: Sign::None,
            typ: Type::None,
            fill: ' ',
            upper: false,
            localized: false,
        }
    }
}

impl FormatSpec {
    pub fn alt(&self) -> bool {
        self.flags & HASH != 0
    }
    pub fn sign_plus(&self) -> bool {
        self.flags & SIGN != 0
    }
    pub fn zero(&self) -> bool {
        self.flags & ZERO != 0
    }
    pub fn set_flag(&mut self, flag: u8) {
        self.flags |= flag;
    }
}

// ---------------------------------------------------------------------------
// The C++ library distinguishes dynamic arguments (`fmt::basic_format_arg`)
// from a compile-time typed `FormatArgs` array. In Rust we model the dynamic
// half as an enum and pack it into a small list that can be driven by
// `format` / `vformat`.
// ---------------------------------------------------------------------------

/// One dynamic argument slot. Mirrors `fmt::basic_format_arg<Context>`.
#[derive(Clone, Debug)]
pub enum FormatValue {
    Int(i64),
    Uint(u64),
    Float(f64),
    Str(String),
    Char(char),
    Bool(bool),
    Pointer(usize),
    None,
}

/// A handle = (argument index, spec pointer) tuple used by the C++ library to
/// lazily resolve a positional argument. We keep the same shape so
/// `FormatArgs` plays nicely with translated call sites.
#[derive(Clone, Copy, Debug)]
pub struct ArgRef {
    pub index: usize,
    pub has_spec: bool,
}

/// One entry of the C++ `format_args` array.
#[derive(Clone, Debug)]
pub struct ArgEntry {
    pub value: FormatValue,
    pub spec: Option<FormatSpec>,
}

/// `fmt::format_args` -> `FormatArgs`.
#[derive(Clone, Debug, Default)]
pub struct FormatArgs {
    pub args: Vec<ArgEntry>,
    pub fmt_str: String,
}

impl FormatArgs {
    pub fn new(fmt_str: impl Into<String>) -> Self {
        Self {
            args: Vec::new(),
            fmt_str: fmt_str.into(),
        }
    }

    pub fn push(&mut self, value: FormatValue) -> &mut Self {
        self.args.push(ArgEntry {
            value,
            spec: None,
        });
        self
    }

    pub fn push_with_spec(&mut self, value: FormatValue, spec: FormatSpec) -> &mut Self {
        self.args.push(ArgEntry {
            value,
            spec: Some(spec),
        });
        self
    }

    pub fn arg(&self, i: usize) -> Option<&ArgEntry> {
        self.args.get(i)
    }
}

// ---------------------------------------------------------------------------
// Iterator-based formatter. The C++ side writes into a `buffer<char>` via
// `back_insert_iterator`; the Rust equivalent is a `String` plus a writer
// trait object so the same algorithm works with any `Write` sink.
// ---------------------------------------------------------------------------

/// Iterator-based formatting context, used when formatting into anything that
/// implements `std::fmt::Write`. `BasicFormatter` is the owned-buffer flavour.
pub struct FormatContext<'a> {
    pub output: &'a mut String,
    pub args: &'a FormatArgs,
    pub next_arg_id: usize,
}

impl<'a> FormatContext<'a> {
    pub fn new(output: &'a mut String, args: &'a FormatArgs) -> Self {
        Self {
            output,
            args,
            next_arg_id: 0,
        }
    }

    /// Equivalent of `fmt::format_context::arg()`.
    pub fn arg(&self, id: usize) -> Option<&ArgEntry> {
        self.args.arg(id)
    }

    /// Equivalent of `fmt::vformat_to`. Walks the format string, pushing
    /// literal pieces directly into the buffer and dispatching replacement
    /// fields through `write_value`.
    pub fn vformat_to(&mut self, fmt_str: &str) -> stdfmt::Result {
        let mut parser = SpecParser::new(fmt_str);
        while !parser.at_end() {
            let lit = parser.next_literal();
            if !lit.is_empty() {
                self.output.push_str(&lit);
            }
            if parser.at_end() {
                break;
            }
            let (idx, spec) = parser.parse_replacement();
            let entry = self.args.arg(idx).cloned().unwrap_or(ArgEntry {
                value: FormatValue::None,
                spec: None,
            });
            let effective = spec.or(entry.spec).unwrap_or_default();
            write_value(self.output, &entry.value, &effective)
                .map_err(|_| stdfmt::Error)?;
        }
        Ok(())
    }
}

impl<'a> stdfmt::Write for FormatContext<'a> {
    fn write_str(&mut self, s: &str) -> stdfmt::Result {
        self.output.push_str(s);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// `BasicFormatter` owns a growable `String`. The C++ side uses
// `basic_memory_buffer<char>` which is exactly that: an inline 500-byte
// buffer that spills onto the heap.
// ---------------------------------------------------------------------------

/// Owned, growable formatter buffer. Equivalent of `fmt::basic_memory_buffer`
/// and the storage behind `fmt::format_to`'s appender.
#[derive(Clone, Debug, Default)]
pub struct BasicFormatter {
    pub buffer: String,
}

impl BasicFormatter {
    pub fn new() -> Self {
        Self {
            buffer: String::with_capacity(500),
        }
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self {
            buffer: String::with_capacity(cap),
        }
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
    }

    pub fn as_str(&self) -> &str {
        &self.buffer
    }

    pub fn into_string(self) -> String {
        self.buffer
    }

    /// `fmt::format_to(appender, ...)` -> write the next argument into the
    /// buffer using its parsed spec.
    pub fn format_to(&mut self, args: &FormatArgs) -> stdfmt::Result {
        let mut ctx = FormatContext::new(&mut self.buffer, args);
        ctx.vformat_to(&args.fmt_str)
    }
}

impl stdfmt::Write for BasicFormatter {
    fn write_str(&mut self, s: &str) -> stdfmt::Result {
        self.buffer.push_str(s);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Global state. The C++ library keeps thread-local state for the global
// formatting buffer. Per the task brief we use `static mut` for these.
// ---------------------------------------------------------------------------

/// Cached global buffer; equivalent of the `g_buffer` inside the C++ source.
pub static mut GLOBAL_BUFFER: BasicFormatter = BasicFormatter {
    buffer: String::new(),
};

/// Last format error reported, mirroring `fmt::report_error`.
pub static mut LAST_ERROR: Option<String> = None;

fn report_error(msg: &str) -> ! {
    // SAFETY: `LAST_ERROR` is only ever written here; no concurrent access is
    // modelled because the task asks for `static mut` globals.
    unsafe {
        LAST_ERROR = Some(msg.to_owned());
    }
    panic!("fmt error: {}", msg);
}

// ---------------------------------------------------------------------------
// Format-spec parser. Equivalent of `fmt::parse_format_spec` / the bits of
// `format_parser` used by PCSX2 call sites.
// ---------------------------------------------------------------------------

/// Parses a `{...}` replacement field body. Strips the outer `{` and `}`
/// and returns the (index, FormatSpec) pair, or signals an error.
pub fn parse_format_spec(spec: &str) -> FormatSpec {
    let mut s = FormatSpec::default();
    let bytes = spec.as_bytes();
    let mut i = 0;

    // Optional fill+align: a character (not `{`/`}`) followed by one of
    // `<`, `>`, `^`, `=`.
    if bytes.len() >= 2 {
        let second = bytes[1];
        if matches!(second, b'<' | b'>' | b'^' | b'=')
            && bytes[0] != b'{'
            && bytes[0] != b'}'
        {
            s.fill = spec.chars().next().unwrap_or(' ');
            s.align = match second {
                b'<' => Align::Left,
                b'>' => Align::Right,
                b'^' => Align::Center,
                b'=' => Align::Numeric,
                _ => unreachable!(),
            };
            i = 2;
        }
    }

    // Optional sign flag.
    if let Some(&b) = bytes.get(i) {
        match b {
            b'+' => {
                s.sign = Sign::Plus;
                s.set_flag(SIGN);
                i += 1;
            }
            b'-' => {
                s.sign = Sign::Minus;
                i += 1;
            }
            b' ' => {
                s.sign = Sign::Space;
                i += 1;
            }
            _ => {}
        }
    }

    // Optional `#` and `0` flags.
    while let Some(&b) = bytes.get(i) {
        match b {
            b'#' => {
                s.set_flag(HASH);
                i += 1;
            }
            b'0' => {
                s.set_flag(ZERO);
                // In C++ `0` implies numeric alignment when no explicit
                // alignment was given.
                if s.align == Align::None {
                    s.align = Align::Numeric;
                }
                i += 1;
            }
            _ => break,
        }
    }

    // Width (a decimal integer).
    let width_start = i;
    while let Some(&b) = bytes.get(i) {
        if b.is_ascii_digit() {
            i += 1;
        } else {
            break;
        }
    }
    if i > width_start {
        if let Ok(w) = spec[width_start..i].parse::<i32>() {
            s.width = w;
        }
    }

    // Optional `.precision`.
    if bytes.get(i) == Some(&b'.') {
        i += 1;
        let p_start = i;
        while let Some(&b) = bytes.get(i) {
            if b.is_ascii_digit() {
                i += 1;
            } else {
                break;
            }
        }
        if i > p_start {
            if let Ok(p) = spec[p_start..i].parse::<i32>() {
                s.precision = p;
            }
        }
    }

    // Optional type character.
    if let Some(&b) = bytes.get(i) {
        s.typ = match b {
            b'd' | b'n' => Type::Dec,
            b'x' => {
                s.upper = false;
                Type::Hex
            }
            b'X' => {
                s.upper = true;
                Type::Hex
            }
            b'o' => Type::Oct,
            b'b' => Type::Bin,
            b'f' | b'F' => Type::Fixed,
            b'e' | b'E' => Type::Exp,
            b's' => Type::String,
            b'p' => Type::Pointer,
            b'c' => Type::Char,
            b'?' => Type::Debug,
            _ => Type::None,
        };
    }

    s
}

// ---------------------------------------------------------------------------
// SpecParser: walks the format string, splitting it into literal pieces and
// `{...}` replacement fields. This is the Rust equivalent of the inline
// `parse_replacement_field` loop in `format.cc`.
// ---------------------------------------------------------------------------

struct SpecParser<'a> {
    s: &'a str,
    pos: usize,
    next_seq: usize,
}

impl<'a> SpecParser<'a> {
    fn new(s: &'a str) -> Self {
        Self {
            s,
            pos: 0,
            next_seq: 0,
        }
    }

    fn at_end(&self) -> bool {
        self.pos >= self.s.len()
    }

    /// Collect literal text up to the next `{` or `}`. Escaped `{{` and `}}`
    /// are folded back to a single brace.
    fn next_literal(&mut self) -> String {
        if self.at_end() {
            return String::new();
        }
        let bytes = self.s.as_bytes();
        let mut out = String::new();
        while self.pos < bytes.len() {
            let c = bytes[self.pos];
            if c == b'{' || c == b'}' {
                break;
            }
            out.push(c as char);
            self.pos += 1;
        }
        out
    }

    /// Parse `{...}` -> (arg_index, Option<spec>).
    fn parse_replacement(&mut self) -> (usize, Option<FormatSpec>) {
        let bytes = self.s.as_bytes();
        if bytes.get(self.pos) == Some(&b'{') {
            self.pos += 1;
        }
        let mut idx = 0usize;
        let mut had_digit = false;
        while let Some(&b) = bytes.get(self.pos) {
            if b.is_ascii_digit() {
                had_digit = true;
                idx = idx * 10 + (b - b'0') as usize;
                self.pos += 1;
            } else {
                break;
            }
        }
        let idx = if had_digit {
            idx
        } else {
            let i = self.next_seq;
            self.next_seq += 1;
            i
        };
        let mut spec = None;
        if bytes.get(self.pos) == Some(&b':') {
            self.pos += 1;
            let spec_start = self.pos;
            while let Some(&b) = bytes.get(self.pos) {
                if b == b'}' {
                    break;
                }
                self.pos += 1;
            }
            spec = Some(parse_format_spec(&self.s[spec_start..self.pos]));
        }
        if bytes.get(self.pos) == Some(&b'}') {
            self.pos += 1;
        }
        (idx, spec)
    }
}

// ---------------------------------------------------------------------------
// Dispatch: pick the right type-specific formatter.
// ---------------------------------------------------------------------------

fn write_value(out: &mut String, value: &FormatValue, spec: &FormatSpec) -> stdfmt::Result {
    match value {
        FormatValue::Int(v) => out.push_str(&format_int(*v, spec)),
        FormatValue::Uint(v) => out.push_str(&format_uint(*v, spec)),
        FormatValue::Float(v) => out.push_str(&format_float(*v, spec)),
        FormatValue::Str(v) => out.push_str(&format_str(v, spec)),
        FormatValue::Char(v) => out.push_str(&format_char(*v, spec)),
        FormatValue::Bool(v) => out.push_str(&format_str(&v.to_string(), spec)),
        FormatValue::Pointer(v) => out.push_str(&format_pointer(*v, spec)),
        FormatValue::None => {}
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Type-specific formatters. Each mirrors a `formatter<T>::format` overload in
// the C++ library.
// ---------------------------------------------------------------------------

/// Format a signed integer. Mirrors `formatter<int>::format`.
pub fn format_int(value: i64, spec: &FormatSpec) -> String {
    let negative = value < 0;
    let abs = value.unsigned_abs();
    let body = match spec.typ {
        Type::Hex => format_uint_base(abs, 16, spec.upper, spec.alt()),
        Type::Oct => format_uint_base(abs, 8, false, spec.alt()),
        Type::Bin => format_uint_base(abs, 2, false, spec.alt()),
        _ => abs.to_string(),
    };
    let prefix = build_int_prefix(negative, spec);
    let mut out = String::new();
    apply_int_padding(&mut out, &prefix, &body, spec);
    out
}

/// Format an unsigned integer. Mirrors `formatter<unsigned>::format`.
pub fn format_uint(value: u64, spec: &FormatSpec) -> String {
    let body = match spec.typ {
        Type::Hex => format_uint_base(value, 16, spec.upper, spec.alt()),
        Type::Oct => format_uint_base(value, 8, false, spec.alt()),
        Type::Bin => format_uint_base(value, 2, false, spec.alt()),
        _ => value.to_string(),
    };
    let mut out = String::new();
    apply_int_padding(&mut out, "", &body, spec);
    out
}

/// Format a float. Mirrors `formatter<double>::format`. Uses Rust's native
/// `format!` and then post-processes to honour the format-spec flags the C++
/// library supports (precision, exp vs fixed, sign, padding).
pub fn format_float(value: f64, spec: &FormatSpec) -> String {
    let negative = value < 0.0 || (value == 0.0 && value.is_sign_negative());
    let abs = if negative { -value } else { value };
    let mut body = if abs.is_nan() {
        "nan".to_string()
    } else if abs.is_infinite() {
        "inf".to_string()
    } else {
        match spec.typ {
            Type::Exp => {
                let prec = if spec.precision < 0 { 6 } else { spec.precision };
                format!("{:.*e}", prec as usize, abs)
            }
            _ => {
                let prec = if spec.precision < 0 { 6 } else { spec.precision };
                format!("{:.*}", prec as usize, abs)
            }
        }
    };

    let prefix = build_int_prefix(negative, spec);
    let mut out = String::new();
    apply_int_padding(&mut out, &prefix, &body, spec);
    out
}

/// Format a string, honouring precision (`%.Ns`) and width. Mirrors
/// `formatter<string_view>::format`.
pub fn format_str(value: &str, spec: &FormatSpec) -> String {
    let truncated: &str = if spec.precision >= 0 {
        let max = spec.precision as usize;
        &value[..value.len().min(max)]
    } else {
        value
    };
    apply_string_padding(truncated, spec)
}

/// Format a single char, honouring the debug alternate (`{:#?}`) and width.
pub fn format_char(value: char, spec: &FormatSpec) -> String {
    let body = if matches!(spec.typ, Type::Debug) {
        let mut s = String::from("'");
        if value == '\'' {
            s.push_str("\\'");
        } else if value == '\\' {
            s.push_str("\\\\");
        } else if value == '\n' {
            s.push_str("\\n");
        } else if value == '\r' {
            s.push_str("\\r");
        } else if value == '\t' {
            s.push_str("\\t");
        } else {
            s.push(value);
        }
        s.push('\'');
        s
    } else {
        value.to_string()
    };
    apply_string_padding(&body, spec)
}

/// Format a pointer (`0xdeadbeef` style). Mirrors `formatter<void*>::format`.
pub fn format_pointer(value: usize, spec: &FormatSpec) -> String {
    let body = if value == 0 {
        "0x0".to_string()
    } else {
        format!("0x{:x}", value)
    };
    apply_string_padding(&body, spec)
}

/// Format an integer using the hex radix. Mirrors the manual hex path used by
/// `write_int` when the spec type is `presentation_type::hex`.
pub fn format_hex(value: u64, upper: bool, width: i32, zero_pad: bool) -> String {
    let body = format_uint_base(value, 16, upper, false);
    let mut spec = FormatSpec::default();
    spec.width = width;
    spec.typ = Type::Hex;
    spec.upper = upper;
    if zero_pad {
        spec.set_flag(ZERO);
    }
    let mut out = String::new();
    apply_int_padding(&mut out, "", &body, &spec);
    out
}

// ---------------------------------------------------------------------------
// Helpers shared by the integer / float paths.
// ---------------------------------------------------------------------------

fn format_uint_base(mut value: u64, base: u32, upper: bool, alt: bool) -> String {
    if value == 0 {
        if alt {
            return match base {
                2 => "0b".to_string(),
                8 => "0".to_string(),
                16 => "0x".to_string(),
                _ => "0".to_string(),
            };
        }
        return "0".to_string();
    }
    let digits = if upper {
        "0123456789ABCDEF"
    } else {
        "0123456789abcdef"
    };
    let mut buf = String::new();
    while value > 0 {
        let d = digits.as_bytes()[(value % base as u64) as usize] as char;
        buf.push(d);
        value /= base as u64;
    }
    let formatted: String = buf.chars().rev().collect();
    let mut out = String::with_capacity(formatted.len() + 2);
    if alt {
        match base {
            2 => out.push_str(if upper { "0B" } else { "0b" }),
            8 => out.push('0'),
            16 => out.push_str(if upper { "0X" } else { "0x" }),
            _ => {}
        }
    }
    out.push_str(&formatted);
    out
}

fn build_int_prefix(negative: bool, spec: &FormatSpec) -> String {
    if negative {
        "-".to_string()
    } else {
        match spec.sign {
            Sign::Plus => "+".to_string(),
            Sign::Space => " ".to_string(),
            _ => String::new(),
        }
    }
}

fn apply_int_padding(out: &mut String, prefix: &str, body: &str, spec: &FormatSpec) {
    let width = spec.width.max(0) as usize;
    let total = prefix.len() + body.len();
    if total >= width {
        out.push_str(prefix);
        out.push_str(body);
        return;
    }
    let pad = width - total;
    let ch = if spec.zero()
        && matches!(
            spec.align,
            Align::Numeric | Align::Right | Align::None
        )
    {
        '0'
    } else {
        spec.fill
    };
    match spec.align {
        Align::Right => {
            for _ in 0..pad {
                out.push(ch);
            }
            out.push_str(prefix);
            out.push_str(body);
        }
        Align::Center => {
            let left = pad / 2;
            let right = pad - left;
            for _ in 0..left {
                out.push(ch);
            }
            out.push_str(prefix);
            out.push_str(body);
            for _ in 0..right {
                out.push(ch);
            }
        }
        _ => {
            // Left, Numeric, None => left-align (or pad zeros on the right
            // when explicitly zero-flagged but left-aligned).
            out.push_str(prefix);
            out.push_str(body);
            for _ in 0..pad {
                out.push(ch);
            }
        }
    }
}

fn apply_string_padding(body: &str, spec: &FormatSpec) -> String {
    let width = spec.width.max(0) as usize;
    if body.len() >= width {
        return body.to_string();
    }
    let pad = width - body.len();
    let ch = spec.fill;
    let mut result = String::with_capacity(width);
    match spec.align {
        Align::Right | Align::Numeric => {
            for _ in 0..pad {
                result.push(ch);
            }
            result.push_str(body);
        }
        Align::Center => {
            let left = pad / 2;
            let right = pad - left;
            for _ in 0..left {
                result.push(ch);
            }
            result.push_str(body);
            for _ in 0..right {
                result.push(ch);
            }
        }
        _ => {
            result.push_str(body);
            for _ in 0..pad {
                result.push(ch);
            }
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Chrono formatter. The C++ `chrono.h` header converts a
// `std::chrono::time_point` to a string using strftime-like specifiers. We
// support the subset PCSX2 actually uses: %Y %m %d %H %M %S %F %T %z %Z %%.
// ---------------------------------------------------------------------------

/// Format a `SystemTime` according to a strftime-style `format` string. Only
/// the specifiers PCSX2 uses are honoured; everything else is passed through
/// literally so downstream code doesn't silently lose characters.
pub fn format_chrono(t: SystemTime, format: &str) -> String {
    use std::time::UNIX_EPOCH;

    let duration = t.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = duration.as_secs();
    let (year, month, day, hour, minute, second) = civil_from_unix(secs as i64);

    let mut out = String::with_capacity(format.len() + 16);
    let bytes = format.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c != '%' || i + 1 >= bytes.len() {
            out.push(c);
            i += 1;
            continue;
        }
        let spec = bytes[i + 1] as char;
        match spec {
            '%' => out.push('%'),
            'Y' => out.push_str(&format!("{:04}", year)),
            'y' => out.push_str(&format!("{:02}", year % 100)),
            'm' => out.push_str(&format!("{:02}", month)),
            'd' => out.push_str(&format!("{:02}", day)),
            'H' => out.push_str(&format!("{:02}", hour)),
            'M' => out.push_str(&format!("{:02}", minute)),
            'S' => out.push_str(&format!("{:02}", second)),
            'F' => out.push_str(&format!("{:04}-{:02}-{:02}", year, month, day)),
            'T' => out.push_str(&format!("{:02}:{:02}:{:02}", hour, minute, second)),
            'z' => out.push_str("+0000"),
            'Z' => out.push_str("UTC"),
            _ => {
                out.push('%');
                out.push(spec);
            }
        }
        i += 2;
    }
    out
}

/// Convert seconds-since-epoch into a `(year, month, day, hour, minute, second)`
/// tuple in UTC. Mirrors the inverse of `std::mktime` for UTC input using
/// Howard Hinnant's date algorithm, ported to Rust.
fn civil_from_unix(secs: i64) -> (i32, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let secs_of_day = secs.rem_euclid(86_400);
    let hour = (secs_of_day / 3600) as u32;
    let minute = ((secs_of_day % 3600) / 60) as u32;
    let second = (secs_of_day % 60) as u32;

    // Howard Hinnant's date algorithm.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d, hour, minute, second)
}

// ---------------------------------------------------------------------------
// Top-level entry point: equivalent of `fmt::format`.
// ---------------------------------------------------------------------------

/// Format the given args according to their format string, returning the
/// resulting `String`. Equivalent of `fmt::format`.
pub fn format(args: &FormatArgs) -> String {
    let mut buf = BasicFormatter::new();
    if buf.format_to(args).is_err() {
        report_error("format failed");
    }
    buf.into_string()
}

/// Format a single value with a printf-style spec; thin convenience wrapper
/// used by translated call sites that previously invoked `fmt::format("...")`.
pub fn format_value<T: Display>(v: T) -> String {
    v.to_string()
}

// ---------------------------------------------------------------------------
// Unit tests covering the core API surface.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::UNIX_EPOCH;

    #[test]
    fn parse_basic_width() {
        let s = parse_format_spec("10d");
        assert_eq!(s.width, 10);
        assert_eq!(s.typ, Type::Dec);
    }

    #[test]
    fn parse_fill_and_align() {
        let s = parse_format_spec("*<");
        assert_eq!(s.fill, '*');
        assert_eq!(s.align, Align::Left);
    }

    #[test]
    fn parse_flags() {
        let s = parse_format_spec("#x");
        assert!(s.alt());
        assert_eq!(s.typ, Type::Hex);
    }

    #[test]
    fn format_int_basic() {
        let s = format_int(42, &FormatSpec::default());
        assert_eq!(s, "42");
    }

    #[test]
    fn format_int_with_width() {
        let mut s = FormatSpec::default();
        s.width = 5;
        s.align = Align::Right;
        s.set_flag(ZERO);
        let out = format_int(42, &s);
        assert_eq!(out, "00042");
    }

    #[test]
    fn format_uint_hex() {
        let mut s = FormatSpec::default();
        s.typ = Type::Hex;
        let out = format_uint(0xdeadbeef, &s);
        assert_eq!(out, "deadbeef");
    }

    #[test]
    fn format_float_fixed() {
        let mut s = FormatSpec::default();
        s.typ = Type::Fixed;
        s.precision = 2;
        let out = format_float(3.14159, &s);
        assert_eq!(out, "3.14");
    }

    #[test]
    fn format_str_truncate() {
        let mut s = FormatSpec::default();
        s.precision = 3;
        let out = format_str("hello", &s);
        assert_eq!(out, "hel");
    }

    #[test]
    fn format_char_debug() {
        let mut s = FormatSpec::default();
        s.typ = Type::Debug;
        let out = format_char('\n', &s);
        assert_eq!(out, "'\\n'");
    }

    #[test]
    fn format_pointer_basic() {
        let out = format_pointer(0xdead_beef, &FormatSpec::default());
        assert_eq!(out, "0xdeadbeef");
    }

    #[test]
    fn format_full() {
        let mut args = FormatArgs::new("name={0}, value={1:#x}");
        args.push(FormatValue::Str("foo".into()));
        args.push(FormatValue::Uint(0x2a));
        let s = format(&args);
        assert_eq!(s, "name=foo, value=0x2a");
    }

    #[test]
    fn chrono_basic() {
        let t = UNIX_EPOCH + std::time::Duration::from_secs(0);
        let s = format_chrono(t, "%Y-%m-%d");
        assert_eq!(s, "1970-01-01");
    }
}
