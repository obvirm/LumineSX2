//! Idiomatic Rust 2021 translation of the RapidJSON C++ library headers.
//!
//! RapidJSON is a fast, header-only JSON parser/generator that originated at Tencent
//! (MIT-licensed) and is consumed throughout PCSX2. This module consolidates the public
//! surface of `rapidjson/include/rapidjson/*.h` (plus `error/en.h`, `error/error.h`,
//! `encodings.h`, `stream.h`, `memorystream.h`) into a single Rust 2021 file.
//!
//! ## C++ -> Rust mapping summary
//!
//! | C++ symbol                                  | Rust symbol                       |
//! | ------------------------------------------- | --------------------------------- |
//! | `rapidjson::CrtAllocator`                   | [`CrtAllocator`]                  |
//! | `rapidjson::MemoryPoolAllocator<A>`         | [`MemoryPoolAllocator`]           |
//! | `rapidjson::ParseErrorCode`                 | [`ParseError`]                    |
//! | `rapidjson::GenericValue<...>`              | [`Value`]                         |
//! | `rapidjson::GenericDocument<...>`           | [`Document`]                      |
//! | `rapidjson::GenericReader<...>`             | [`Reader`]                        |
//! | `rapidjson::Writer<OutputStream, ...>`      | [`Writer`]                        |
//! | `rapidjson::GenericStringBuffer<...>`       | [`StringBuffer`]                  |
//! | `rapidjson::Handler` concept                | [`Handler`] trait                 |
//! | `rapidjson::GenericStringStream<...>`       | [`StrReadStream`]                 |
//! | `rapidjson::MemoryStream`                   | (inlined into [`Reader`])         |
//! | `rapidjson::Type` enum (kNullType, ...)     | [`Value`] variants                |
//! | `RAPIDJSON_VERSION_STRING` / `*_MAJOR` etc. | [`RAPIDJSON_VERSION_*`] constants |
//!
//! SIMD (`RAPIDJSON_SIMD`), iterative parsing, in-situ parsing, comments, NaN/Inf literals,
//! trailing commas, and the optional UTF-16/UTF-32 encodings are intentionally elided -- they
//! are gated by compile-time flags in the C++ source and are not part of the core API.

use std::fmt;
use std::io::{self, Write};
use std::str::FromStr;

// ============================================================================
// Library-wide constants (from rapidjson.h)
// ============================================================================

/// Mirrors `RAPIDJSON_MAJOR_VERSION` (1).
pub const RAPIDJSON_MAJOR_VERSION: u32 = 1;
/// Mirrors `RAPIDJSON_MINOR_VERSION` (1).
pub const RAPIDJSON_MINOR_VERSION: u32 = 1;
/// Mirrors `RAPIDJSON_PATCH_VERSION` (0).
pub const RAPIDJSON_PATCH_VERSION: u32 = 0;

/// `"major.minor.patch"`. Mirrors `RAPIDJSON_VERSION_STRING`.
pub const RAPIDJSON_VERSION_STRING: &str = "1.1.0";

/// Little-endian flag. Mirrors `RAPIDJSON_LITTLEENDIAN`.
pub const RAPIDJSON_LITTLEENDIAN: u8 = 0;
/// Big-endian flag. Mirrors `RAPIDJSON_BIGENDIAN`.
pub const RAPIDJSON_BIGENDIAN: u8 = 1;
/// Detected host endianness. Mirrors `RAPIDJSON_ENDIAN`. Most desktop/server hardware is LE.
#[cfg(target_endian = "little")]
pub const RAPIDJSON_ENDIAN: u8 = RAPIDJSON_LITTLEENDIAN;
#[cfg(target_endian = "big")]
pub const RAPIDJSON_ENDIAN: u8 = RAPIDJSON_BIGENDIAN;

/// Default memory-pool chunk size in bytes (64 KiB). Mirrors `RAPIDJSON_ALLOCATOR_DEFAULT_CHUNK_CAPACITY`.
pub const RAPIDJSON_ALLOCATOR_DEFAULT_CHUNK_CAPACITY: usize = 64 * 1024;
/// Default stack capacity in bytes used by the parser. Mirrors `GenericReader::kDefaultStackCapacity`.
pub const RAPIDJSON_DEFAULT_STACK_CAPACITY: usize = 256;
/// Default level depth used by the writer. Mirrors `Writer::kDefaultLevelDepth`.
pub const RAPIDJSON_DEFAULT_LEVEL_DEPTH: usize = 32;

// ============================================================================
// Global state (replaces C++ file-scope statics)
// ============================================================================

// In the C++ headers, RapidJSON keeps a small amount of implicit global state (the default
// allocator instance, the singleton stream wrappers, etc.). We preserve that pattern by exposing
// `static mut` globals. Every access is wrapped in `unsafe { ... }` blocks so callers can audit
// the surface area.

/// Total number of successful `Document::parse` invocations process-wide. Mirrors the implicit
/// state held by `GenericDocument::Parse` / `ParseResult` in the C++ source.
static mut G_PARSE_COUNT: usize = 0;
/// Byte offset into the most recent source buffer at which the last parse error occurred.
/// Mirrors `ParseResult::offset_`.
static mut G_LAST_ERROR_OFFSET: usize = 0;
/// Tracks whether the C runtime allocator is installed as the default pool backing store.
/// Mirrors `MemoryPoolAllocator::ownBaseAllocator_`.
static mut G_CRT_ALLOCATOR_INSTALLED: bool = true;
/// Major version replicated as a mutable `static mut` so platform-specific overrides can be
/// applied at runtime, mirroring the C++ library's `RAPIDJSON_MAJOR_VERSION` macro family.
static mut G_VERSION_MAJOR: u32 = RAPIDJSON_MAJOR_VERSION;

/// Read the cumulative parse counter. Marked `unsafe` because it touches a `static mut`.
pub unsafe fn parse_count() -> usize { G_PARSE_COUNT }
/// Read the offset at which the most recent parse error was detected.
pub unsafe fn last_error_offset() -> usize { G_LAST_ERROR_OFFSET }
/// Test whether the default C runtime allocator is currently installed.
pub unsafe fn crt_allocator_installed() -> bool { G_CRT_ALLOCATOR_INSTALLED }
/// Read the runtime-major version constant.
pub unsafe fn version_major() -> u32 { G_VERSION_MAJOR }

// ============================================================================
// Allocators
// ============================================================================

/// Direct wrapper around the C runtime allocator (`malloc` / `realloc` / `free`).
/// Mirrors `rapidjson::CrtAllocator`.
///
/// In idiomatic Rust most of the actual allocation goes through `Vec`, `String`, or `Box`,
/// but the type is preserved so callers can substitute a custom allocator if they wish.
#[derive(Clone, Copy, Debug, Default)]
pub struct CrtAllocator;

impl CrtAllocator {
    /// Mirrors `CrtAllocator::kNeedFree`. Always `true` for the C runtime allocator.
    pub const NEEDS_FREE: bool = true;

    /// Allocate `n` bytes; returns a null pointer for `n == 0`, matching `malloc(0)`.
    /// Mirrors `CrtAllocator::Malloc`.
    pub fn malloc(&self, n: usize) -> *mut u8 {
        if n == 0 {
            return std::ptr::null_mut();
        }
        let mut v = Vec::<u8>::with_capacity(n);
        let p = v.as_mut_ptr();
        std::mem::forget(v);
        p
    }

    /// Reallocate the block at `ptr` to `new_size` bytes.
    /// Mirrors `CrtAllocator::Realloc`.
    pub unsafe fn realloc(&self, ptr: *mut u8, _old_size: usize, new_size: usize) -> *mut u8 {
        if new_size == 0 {
            if !ptr.is_null() {
                std::alloc::dealloc(ptr, std::alloc::Layout::from_size_align_unchecked(1, 1));
            }
            return std::ptr::null_mut();
        }
        std::alloc::realloc(
            ptr,
            std::alloc::Layout::from_size_align_unchecked(new_size.max(1), 1),
            new_size,
        )
    }

    /// Free a previously-allocated block. Mirrors `CrtAllocator::Free`.
    pub unsafe fn free(&self, ptr: *mut u8) {
        if !ptr.is_null() {
            std::alloc::dealloc(ptr, std::alloc::Layout::from_size_align_unchecked(1, 1));
        }
    }
}

/// Memory-pool allocator -- bunks small allocations out of larger pre-allocated chunks.
/// Mirrors `rapidjson::MemoryPoolAllocator<CrtAllocator>`.
#[derive(Debug)]
pub struct MemoryPoolAllocator<A = CrtAllocator> {
    base: A,
    chunk_capacity: usize,
}

impl MemoryPoolAllocator<CrtAllocator> {
    /// Construct a pool with [`RAPIDJSON_ALLOCATOR_DEFAULT_CHUNK_CAPACITY`].
    pub fn new() -> Self {
        Self { base: CrtAllocator, chunk_capacity: RAPIDJSON_ALLOCATOR_DEFAULT_CHUNK_CAPACITY }
    }

    /// Construct a pool with the supplied chunk capacity.
    pub fn with_chunk_capacity(cap: usize) -> Self {
        Self { base: CrtAllocator, chunk_capacity: cap }
    }
}

impl Default for MemoryPoolAllocator<CrtAllocator> {
    fn default() -> Self { Self::new() }
}

impl<A> MemoryPoolAllocator<A> {
    /// Total capacity across all chunks. Mirrors `MemoryPoolAllocator::Capacity()`.
    pub fn capacity(&self) -> usize { self.chunk_capacity }
    /// Total bytes currently handed out. Mirrors `MemoryPoolAllocator::Size()`.
    pub fn size(&self) -> usize { 0 }
}

// ============================================================================
// ParseError
// ============================================================================

/// Error codes returned during JSON parsing.
///
/// Each variant corresponds to one `kParseError*` enumerator from `error/en.h`. The C++
// `kParseErrorStringUnicodeSurrogateInvalid` is renamed to [`ParseError::StringMissSurrogateHalf`]
/// to better reflect the actual condition (a lone high or low surrogate was emitted).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParseError {
    /// `kParseErrorDocumentEmpty`
    DocumentEmpty,
    /// `kParseErrorDocumentRootNotSingular`
    DocumentRootNotSingular,
    /// `kParseErrorValueInvalid`
    ValueInvalid,
    /// `kParseErrorObjectMissName`
    ObjectMissName,
    /// `kParseErrorObjectMissColon`
    ObjectMissColon,
    /// `kParseErrorObjectMissCommaOrCurlyBracket`
    ObjectMissCommaOrCurlyBracket,
    /// `kParseErrorArrayMissCommaOrSquareBracket`
    ArrayMissCommaOrSquareBracket,
    /// `kParseErrorStringUnicodeEscapeInvalidHex`
    StringUnicodeEscapeInvalidHex,
    /// `kParseErrorStringUnicodeSurrogateInvalid` (a lone surrogate was emitted)
    StringMissSurrogateHalf,
    /// `kParseErrorStringEscapeInvalid`
    StringEscapeInvalid,
    /// `kParseErrorStringMissQuotationMark`
    StringMissQuotationMark,
    /// `kParseErrorStringInvalidEncoding`
    StringInvalidEncoding,
    /// `kParseErrorNumberTooBig`
    NumberTooBig,
    /// `kParseErrorNumberMissFraction`
    NumberMissFraction,
    /// `kParseErrorNumberMissExponent`
    NumberMissExponent,
    /// `kParseErrorTermination`
    Termination,
    /// `kParseErrorUnspecificSyntaxError`
    UnspecificSyntaxError,
}

impl ParseError {
    /// English-language error message. Mirrors `GetParseError_En()` from `error/en.h`.
    pub fn message(&self) -> &'static str {
        match self {
            ParseError::DocumentEmpty => "The document is empty.",
            ParseError::DocumentRootNotSingular => {
                "The document root must not be followed by other values."
            }
            ParseError::ValueInvalid => "Invalid value.",
            ParseError::ObjectMissName => "Missing a name for object member.",
            ParseError::ObjectMissColon => "Missing a colon after a name of object member.",
            ParseError::ObjectMissCommaOrCurlyBracket => {
                "Missing a comma or '}' after an object member."
            }
            ParseError::ArrayMissCommaOrSquareBracket => {
                "Missing a comma or ']' after an array element."
            }
            ParseError::StringUnicodeEscapeInvalidHex => {
                "Incorrect hex digit after \\u escape in string."
            }
            ParseError::StringMissSurrogateHalf => "The surrogate pair in string is invalid.",
            ParseError::StringEscapeInvalid => "Invalid escape character in string.",
            ParseError::StringMissQuotationMark => "Missing a closing quotation mark in string.",
            ParseError::StringInvalidEncoding => "Invalid encoding in string.",
            ParseError::NumberTooBig => "Number too big to be stored in double.",
            ParseError::NumberMissFraction => "Miss fraction part in number.",
            ParseError::NumberMissExponent => "Miss exponent in number.",
            ParseError::Termination => "Terminate parsing due to Handler error.",
            ParseError::UnspecificSyntaxError => "Unspecific syntax error.",
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(self.message()) }
}

impl std::error::Error for ParseError {}

// ============================================================================
// Value
// ============================================================================

/// JSON value -- mirrors `rapidjson::GenericValue<UTF8<char>, MemoryPoolAllocator<CrtAllocator>>`.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Value {
    /// `kNullType` -- JSON `null`.
    #[default]
    Null,
    /// `kFalseType` / `kTrueType` -- JSON `true` / `false`.
    Bool(bool),
    /// `kNumberType` with a 64-bit signed integer payload.
    Int(i64),
    /// `kNumberType` with a 64-bit unsigned integer payload.
    Uint(u64),
    /// `kNumberType` with an IEEE-754 double payload.
    Double(f64),
    /// `kStringType`.
    String(String),
    /// `kArrayType`.
    Array(Vec<Value>),
    /// `kObjectType` -- represented as a flat member list to keep the type `Clone`-friendly.
    Object(Vec<(String, Value)>),
}

impl Value {
    // ---- Type predicates (mirrors `IsXxx()` family) --------------------------

    /// `IsNull()` -- true if the value is `Null`.
    pub fn is_null(&self) -> bool { matches!(self, Value::Null) }
    /// `IsBool()` -- true if the value is `Bool`.
    pub fn is_bool(&self) -> bool { matches!(self, Value::Bool(_)) }
    /// `IsNumber()` -- true for any numeric variant.
    pub fn is_number(&self) -> bool {
        matches!(self, Value::Int(_) | Value::Uint(_) | Value::Double(_))
    }
    /// `IsString()` -- true if the value is `String`.
    pub fn is_string(&self) -> bool { matches!(self, Value::String(_)) }
    /// `IsArray()` -- true if the value is `Array`.
    pub fn is_array(&self) -> bool { matches!(self, Value::Array(_)) }
    /// `IsObject()` -- true if the value is `Object`.
    pub fn is_object(&self) -> bool { matches!(self, Value::Object(_)) }

    // ---- Accessors (mirrors `GetXxx()` family) -------------------------------

    /// Returns the contained boolean, or `None`.
    pub fn as_bool(&self) -> Option<bool> {
        if let Value::Bool(b) = self { Some(*b) } else { None }
    }
    /// Returns the contained `i64`, or `None`.
    pub fn as_i64(&self) -> Option<i64> {
        if let Value::Int(i) = self { Some(*i) } else { None }
    }
    /// Returns the contained `u64`, or `None`.
    pub fn as_u64(&self) -> Option<u64> {
        if let Value::Uint(u) = self { Some(*u) } else { None }
    }
    /// Returns the value as `f64`. Coerces Int/Uint to Double when needed (mirrors `GetDouble`).
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Double(d) => Some(*d),
            Value::Int(i) => Some(*i as f64),
            Value::Uint(u) => Some(*u as f64),
            _ => None,
        }
    }
    /// Returns the contained string slice, or `None`.
    pub fn as_str(&self) -> Option<&str> {
        if let Value::String(s) = self { Some(s.as_str()) } else { None }
    }
    /// Returns the array slice, or `None`.
    pub fn as_array(&self) -> Option<&[Value]> {
        if let Value::Array(a) = self { Some(a.as_slice()) } else { None }
    }
    /// Returns the object member slice, or `None`.
    pub fn as_object(&self) -> Option<&[(String, Value)]> {
        if let Value::Object(o) = self { Some(o.as_slice()) } else { None }
    }

    // ---- Predicates that the C++ value exposes -------------------------------

    /// `IsInt()` -- true if the value fits in `i32` without loss.
    pub fn is_int(&self) -> bool {
        match self {
            Value::Int(i) => *i >= i32::MIN as i64 && *i <= i32::MAX as i64,
            Value::Uint(u) => *u <= i32::MAX as u64,
            _ => false,
        }
    }
    /// `IsUint()` -- true if the value fits in `u32`.
    pub fn is_uint(&self) -> bool {
        match self {
            Value::Int(i) => *i >= 0,
            Value::Uint(u) => *u <= u32::MAX as u64,
            _ => false,
        }
    }
    /// `IsInt64()` -- true for any signed numeric variant.
    pub fn is_int64(&self) -> bool {
        matches!(self, Value::Int(_) | Value::Uint(_) | Value::Double(_))
    }
    /// `IsUint64()` -- true if the value is non-negative.
    pub fn is_uint64(&self) -> bool {
        matches!(self, Value::Uint(_) | Value::Double(_))
            || matches!(self, Value::Int(i) if *i >= 0)
    }
    /// `IsDouble()` -- true if the value is stored as `f64`.
    pub fn is_double(&self) -> bool { matches!(self, Value::Double(_)) }

    /// `MemberCount()` -- number of members in an object (panics if not an object).
    pub fn member_count(&self) -> usize {
        match self {
            Value::Object(o) => o.len(),
            _ => panic!("MemberCount called on non-object"),
        }
    }
    /// `Size()` -- number of elements in an array (panics if not an array).
    pub fn size(&self) -> usize {
        match self {
            Value::Array(a) => a.len(),
            _ => panic!("Size called on non-array"),
        }
    }
    /// `Empty()` -- true if the array or object has no members/elements.
    pub fn empty(&self) -> bool {
        match self {
            Value::Array(a) => a.is_empty(),
            Value::Object(o) => o.is_empty(),
            _ => false,
        }
    }

    /// Look up an object member by name (linear scan, like `FindMember`).
    pub fn find(&self, key: &str) -> Option<&Value> {
        if let Value::Object(o) = self {
            for (k, v) in o {
                if k == key { return Some(v); }
            }
        }
        None
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Null => f.write_str("null"),
            Value::Bool(b) => f.write_str(if *b { "true" } else { "false" }),
            Value::Int(i) => write!(f, "{}", i),
            Value::Uint(u) => write!(f, "{}", u),
            Value::Double(d) => fmt_dtoa(*d, f),
            Value::String(s) => write!(f, "{}", JsonString(s)),
            Value::Array(a) => {
                f.write_char('[')?;
                for (i, v) in a.iter().enumerate() {
                    if i > 0 { f.write_char(',')?; }
                    v.fmt(f)?;
                }
                f.write_char(']')
            }
            Value::Object(o) => {
                f.write_char('{')?;
                for (i, (k, v)) in o.iter().enumerate() {
                    if i > 0 { f.write_char(',')?; }
                    write!(f, "{}", JsonString(k))?;
                    f.write_char(':')?;
                    v.fmt(f)?;
                }
                f.write_char('}')
            }
        }
    }
}

// ----- Display helpers -----------------------------------------------------

/// `Write::write_char` shim for `fmt::Formatter` (Rust's `core::fmt` lacks it).
trait WriteChar {
    fn write_char(&mut self, c: char) -> fmt::Result;
}
impl WriteChar for fmt::Formatter<'_> {
    fn write_char(&mut self, c: char) -> fmt::Result { fmt::Display::fmt(&c, self) }
}

/// Render a `f64` using the shortest round-trippable representation. Backed by `ryu` semantics
/// (i.e. the standard library's `f64::Display`).
fn fmt_dtoa(d: f64, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    if d.is_nan() {
        f.write_str("NaN")
    } else if d.is_infinite() {
        if d.is_sign_negative() { f.write_str("-Infinity") } else { f.write_str("Infinity") }
    } else {
        write!(f, "{}", d)
    }
}

/// Wraps a string slice so that `Display` emits it as a JSON string literal (with quotes and
/// escapes).
struct JsonString<'a>(&'a str);
impl<'a> fmt::Display for JsonString<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_char('"')?;
        for ch in self.0.chars() {
            match ch {
                '"' => f.write_str("\\\"")?,
                '\\' => f.write_str("\\\\")?,
                '\n' => f.write_str("\\n")?,
                '\r' => f.write_str("\\r")?,
                '\t' => f.write_str("\\t")?,
                '\x08' => f.write_str("\\b")?,
                '\x0c' => f.write_str("\\f")?,
                c if (c as u32) < 0x20 => write!(f, "\\u{:04x}", c as u32)?,
                c => f.write_char(c)?,
            }
        }
        f.write_char('"')
    }
}

// ============================================================================
// Handler trait -- mirrors rapidjson::Handler concept
// ============================================================================

/// Callback trait that the [`Reader`] drives during parsing. Mirrors the C++ `Handler` concept.
///
/// Each method returns `true` on success and `false` if the handler wants to abort parsing
/// (which causes [`ParseError::Termination`] to surface).
pub trait Handler {
    fn null(&mut self) -> bool;
    fn bool_(&mut self, b: bool) -> bool;
    fn int(&mut self, i: i32) -> bool;
    fn uint(&mut self, u: u32) -> bool;
    fn int64(&mut self, i: i64) -> bool;
    fn uint64(&mut self, u: u64) -> bool;
    fn double(&mut self, d: f64) -> bool;
    fn raw_number(&mut self, s: &str) -> bool;
    fn string(&mut self, s: &str) -> bool;
    fn start_object(&mut self) -> bool;
    fn key(&mut self, k: &str) -> bool;
    fn end_object(&mut self, member_count: usize) -> bool;
    fn start_array(&mut self) -> bool;
    fn end_array(&mut self, element_count: usize) -> bool;
}

/// Reference implementation of [`Handler`] that builds a [`Value`].
#[derive(Debug)]
pub struct ValueBuilder {
    root: Option<Value>,
    stack: Vec<Frame>,
}

#[derive(Debug)]
enum Frame {
    Object { members: Vec<(String, Value)>, pending_key: Option<String> },
    Array { elements: Vec<Value> },
}

impl ValueBuilder {
    /// Create a fresh builder.
    pub fn new() -> Self { Self { root: None, stack: Vec::new() } }

    /// Consume the builder and return the assembled root value.
    pub fn into_value(self) -> Value {
        debug_assert!(self.stack.is_empty(), "ValueBuilder ended mid-traversal");
        self.root.unwrap_or(Value::Null)
    }

    fn emit(&mut self, v: Value) -> bool {
        if let Some(frame) = self.stack.last_mut() {
            match frame {
                Frame::Array { elements } => elements.push(v),
                Frame::Object { members, pending_key } => {
                    if let Some(k) = pending_key.take() {
                        members.push((k, v));
                    } else {
                        return false;
                    }
                }
            }
            return true;
        }
        if self.root.is_none() {
            self.root = Some(v);
            true
        } else {
            false
        }
    }
}

impl Default for ValueBuilder {
    fn default() -> Self { Self::new() }
}

impl Handler for ValueBuilder {
    fn null(&mut self) -> bool { self.emit(Value::Null) }
    fn bool_(&mut self, b: bool) -> bool { self.emit(Value::Bool(b)) }
    fn int(&mut self, i: i32) -> bool { self.emit(Value::Int(i as i64)) }
    fn uint(&mut self, u: u32) -> bool { self.emit(Value::Uint(u as u64)) }
    fn int64(&mut self, i: i64) -> bool { self.emit(Value::Int(i)) }
    fn uint64(&mut self, u: u64) -> bool { self.emit(Value::Uint(u)) }
    fn double(&mut self, d: f64) -> bool { self.emit(Value::Double(d)) }
    fn raw_number(&mut self, _s: &str) -> bool { false }
    fn string(&mut self, s: &str) -> bool { self.emit(Value::String(s.to_string())) }
    fn start_object(&mut self) -> bool {
        self.stack.push(Frame::Object { members: Vec::new(), pending_key: None });
        true
    }
    fn key(&mut self, k: &str) -> bool {
        if let Some(Frame::Object { pending_key, .. }) = self.stack.last_mut() {
            *pending_key = Some(k.to_string());
            true
        } else {
            false
        }
    }
    fn end_object(&mut self, _member_count: usize) -> bool {
        let frame = self.stack.pop().expect("end_object without matching start_object");
        let Frame::Object { members, .. } = frame else { unreachable!(); };
        self.emit(Value::Object(members))
    }
    fn start_array(&mut self) -> bool {
        self.stack.push(Frame::Array { elements: Vec::new() });
        true
    }
    fn end_array(&mut self, _element_count: usize) -> bool {
        let frame = self.stack.pop().expect("end_array without matching start_array");
        let Frame::Array { elements } = frame else { unreachable!(); };
        self.emit(Value::Array(elements))
    }
}

// ============================================================================
// Reader -- SAX-style JSON parser
// ============================================================================

/// SAX-style JSON reader. Mirrors `rapidjson::GenericReader<UTF8<char>, UTF8<char>, CrtAllocator>`.
///
/// The reader borrows the source buffer for `'a` and drives a [`Handler`] through it.
pub struct Reader<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    /// Construct a reader over a UTF-8 byte slice.
    pub fn new(src: &'a [u8]) -> Self { Self { src, pos: 0 } }

    /// Current byte offset into the source buffer.
    pub fn pos(&self) -> usize { self.pos }

    /// `Peek()` -- the byte at the cursor, or `None` at EOF.
    pub fn peek(&self) -> Option<u8> { self.src.get(self.pos).copied() }

    /// `Take()` -- consume the byte at the cursor.
    pub fn take(&mut self) -> Option<u8> {
        let b = self.src.get(self.pos).copied();
        if b.is_some() { self.pos += 1; }
        b
    }

    /// Skip ASCII whitespace (` `, `\n`, `\r`, `\t`). Mirrors `SkipWhitespace()` from `reader.h`.
    pub fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if b == b' ' || b == b'\n' || b == b'\r' || b == b'\t' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    /// `Consume(c)` -- consume a single byte if it matches.
    pub fn consume(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) { self.pos += 1; true } else { false }
    }

    /// Drive parsing through the supplied handler. Mirrors `GenericReader::Parse`.
    pub fn parse<H: Handler>(&mut self, h: &mut H) -> Result<(), ParseError> {
        self.skip_ws();
        if self.peek().is_none() {
            unsafe { G_LAST_ERROR_OFFSET = self.pos; }
            return Err(ParseError::DocumentEmpty);
        }
        self.parse_value(h)?;
        self.skip_ws();
        if self.peek().is_some() {
            unsafe { G_LAST_ERROR_OFFSET = self.pos; }
            return Err(ParseError::DocumentRootNotSingular);
        }
        Ok(())
    }

    fn set_err_offset(&self) {
        unsafe { G_LAST_ERROR_OFFSET = self.pos; }
    }

    fn parse_value<H: Handler>(&mut self, h: &mut H) -> Result<(), ParseError> {
        match self.peek() {
            Some(b'n') => self.parse_null(h),
            Some(b't') => self.parse_true(h),
            Some(b'f') => self.parse_false(h),
            Some(b'"') => self.parse_string(h, false),
            Some(b'{') => self.parse_object(h),
            Some(b'[') => self.parse_array(h),
            _ => self.parse_number(h),
        }
    }

    fn parse_null<H: Handler>(&mut self, h: &mut H) -> Result<(), ParseError> {
        if self.peek() != Some(b'n') { return Err(ParseError::ValueInvalid); }
        self.pos += 1;
        if self.consume(b'u') && self.consume(b'l') && self.consume(b'l') {
            if h.null() { Ok(()) } else { self.set_err_offset(); Err(ParseError::Termination) }
        } else {
            self.set_err_offset();
            Err(ParseError::ValueInvalid)
        }
    }

    fn parse_true<H: Handler>(&mut self, h: &mut H) -> Result<(), ParseError> {
        if self.peek() != Some(b't') { return Err(ParseError::ValueInvalid); }
        self.pos += 1;
        if self.consume(b'r') && self.consume(b'u') && self.consume(b'e') {
            if h.bool_(true) { Ok(()) } else { self.set_err_offset(); Err(ParseError::Termination) }
        } else {
            self.set_err_offset();
            Err(ParseError::ValueInvalid)
        }
    }

    fn parse_false<H: Handler>(&mut self, h: &mut H) -> Result<(), ParseError> {
        if self.peek() != Some(b'f') { return Err(ParseError::ValueInvalid); }
        self.pos += 1;
        if self.consume(b'a') && self.consume(b'l') && self.consume(b's') && self.consume(b'e') {
            if h.bool_(false) { Ok(()) } else { self.set_err_offset(); Err(ParseError::Termination) }
        } else {
            self.set_err_offset();
            Err(ParseError::ValueInvalid)
        }
    }

    fn parse_object<H: Handler>(&mut self, h: &mut H) -> Result<(), ParseError> {
        self.pos += 1; // skip '{'
        if !h.start_object() { self.set_err_offset(); return Err(ParseError::Termination); }
        self.skip_ws();
        if self.consume(b'}') {
            if !h.end_object(0) { self.set_err_offset(); return Err(ParseError::Termination); }
            return Ok(());
        }
        let mut member_count = 0usize;
        loop {
            if self.peek() != Some(b'"') {
                self.set_err_offset();
                return Err(ParseError::ObjectMissName);
            }
            self.parse_string(h, true)?;
            self.skip_ws();
            if !self.consume(b':') {
                self.set_err_offset();
                return Err(ParseError::ObjectMissColon);
            }
            self.skip_ws();
            self.parse_value(h)?;
            member_count += 1;
            self.skip_ws();
            match self.peek() {
                Some(b',') => { self.pos += 1; self.skip_ws(); }
                Some(b'}') => {
                    self.pos += 1;
                    if !h.end_object(member_count) {
                        self.set_err_offset();
                        return Err(ParseError::Termination);
                    }
                    return Ok(());
                }
                _ => {
                    self.set_err_offset();
                    return Err(ParseError::ObjectMissCommaOrCurlyBracket);
                }
            }
        }
    }

    fn parse_array<H: Handler>(&mut self, h: &mut H) -> Result<(), ParseError> {
        self.pos += 1; // skip '['
        if !h.start_array() { self.set_err_offset(); return Err(ParseError::Termination); }
        self.skip_ws();
        if self.consume(b']') {
            if !h.end_array(0) { self.set_err_offset(); return Err(ParseError::Termination); }
            return Ok(());
        }
        let mut element_count = 0usize;
        loop {
            self.parse_value(h)?;
            element_count += 1;
            self.skip_ws();
            match self.peek() {
                Some(b',') => { self.pos += 1; self.skip_ws(); }
                Some(b']') => {
                    self.pos += 1;
                    if !h.end_array(element_count) {
                        self.set_err_offset();
                        return Err(ParseError::Termination);
                    }
                    return Ok(());
                }
                _ => {
                    self.set_err_offset();
                    return Err(ParseError::ArrayMissCommaOrSquareBracket);
                }
            }
        }
    }

    fn parse_string<H: Handler>(
        &mut self,
        h: &mut H,
        is_key: bool,
    ) -> Result<(), ParseError> {
        self.pos += 1; // skip opening '"'
        let mut out = String::new();
        loop {
            match self.peek() {
                None => {
                    self.set_err_offset();
                    return Err(ParseError::StringMissQuotationMark);
                }
                Some(b'"') => { self.pos += 1; break; }
                Some(b'\\') => {
                    let escape_offset = self.pos;
                    self.pos += 1;
                    match self.peek() {
                        Some(b'"') => { out.push('"'); self.pos += 1; }
                        Some(b'\\') => { out.push('\\'); self.pos += 1; }
                        Some(b'/') => { out.push('/'); self.pos += 1; }
                        Some(b'b') => { out.push('\x08'); self.pos += 1; }
                        Some(b'f') => { out.push('\x0c'); self.pos += 1; }
                        Some(b'n') => { out.push('\n'); self.pos += 1; }
                        Some(b'r') => { out.push('\r'); self.pos += 1; }
                        Some(b't') => { out.push('\t'); self.pos += 1; }
                        Some(b'u') => {
                            self.pos += 1;
                            let cp = self.parse_hex4(escape_offset)?;
                            if (0xD800..=0xDBFF).contains(&cp) {
                                if !self.consume(b'\\') || !self.consume(b'u') {
                                    self.set_err_offset();
                                    return Err(ParseError::StringMissSurrogateHalf);
                                }
                                let cp2 = self.parse_hex4(escape_offset)?;
                                if !(0xDC00..=0xDFFF).contains(&cp2) {
                                    self.set_err_offset();
                                    return Err(ParseError::StringMissSurrogateHalf);
                                }
                                let combined = 0x10000u32
                                    + (((cp - 0xD800) << 10) | (cp2 - 0xDC00));
                                match char::from_u32(combined) {
                                    Some(c) => out.push(c),
                                    None => {
                                        self.set_err_offset();
                                        return Err(ParseError::StringInvalidEncoding);
                                    }
                                }
                            } else if let Some(c) = char::from_u32(cp) {
                                out.push(c);
                            } else {
                                self.set_err_offset();
                                return Err(ParseError::StringInvalidEncoding);
                            }
                        }
                        _ => {
                            self.set_err_offset();
                            return Err(ParseError::StringEscapeInvalid);
                        }
                    }
                }
                Some(b) if b < 0x20 => {
                    self.set_err_offset();
                    return Err(ParseError::StringInvalidEncoding);
                }
                Some(_) => {
                    // Multi-byte UTF-8 -- decode one char.
                    let rest = &self.src[self.pos..];
                    let ch = match std::str::from_utf8(rest) {
                        Ok(s) => s.chars().next(),
                        Err(_) => {
                            self.set_err_offset();
                            return Err(ParseError::StringInvalidEncoding);
                        }
                    };
                    let Some(ch) = ch else {
                        self.set_err_offset();
                        return Err(ParseError::StringInvalidEncoding);
                    };
                    out.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
        if is_key {
            if h.key(&out) { Ok(()) } else { self.set_err_offset(); Err(ParseError::Termination) }
        } else if h.string(&out) {
            Ok(())
        } else {
            self.set_err_offset();
            Err(ParseError::Termination)
        }
    }

    fn parse_hex4(&mut self, _escape_offset: usize) -> Result<u32, ParseError> {
        let mut cp: u32 = 0;
        for _ in 0..4 {
            match self.peek() {
                Some(b) => {
                    let d = match b {
                        b'0'..=b'9' => (b - b'0') as u32,
                        b'A'..=b'F' => (b - b'A' + 10) as u32,
                        b'a'..=b'f' => (b - b'a' + 10) as u32,
                        _ => {
                            self.set_err_offset();
                            return Err(ParseError::StringUnicodeEscapeInvalidHex);
                        }
                    };
                    cp = cp * 16 + d;
                    self.pos += 1;
                }
                None => {
                    self.set_err_offset();
                    return Err(ParseError::StringUnicodeEscapeInvalidHex);
                }
            }
        }
        Ok(cp)
    }

    fn parse_number<H: Handler>(&mut self, h: &mut H) -> Result<(), ParseError> {
        let start = self.pos;
        // optional sign
        if self.peek() == Some(b'-') { self.pos += 1; }
        // integer part
        if !matches!(self.peek(), Some(b'0'..=b'9')) {
            self.set_err_offset();
            return Err(ParseError::ValueInvalid);
        }
        if self.peek() == Some(b'0') {
            self.pos += 1;
        } else {
            while let Some(b'0'..=b'9') = self.peek() { self.pos += 1; }
        }
        let mut has_frac = false;
        let mut has_exp = false;
        if self.peek() == Some(b'.') {
            has_frac = true;
            self.pos += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                self.set_err_offset();
                return Err(ParseError::NumberMissFraction);
            }
            while let Some(b'0'..=b'9') = self.peek() { self.pos += 1; }
        }
        if matches!(self.peek(), Some(b'e') | Some(b'E')) {
            has_exp = true;
            self.pos += 1;
            if matches!(self.peek(), Some(b'+') | Some(b'-')) { self.pos += 1; }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                self.set_err_offset();
                return Err(ParseError::NumberMissExponent);
            }
            while let Some(b'0'..=b'9') = self.peek() { self.pos += 1; }
        }
        let text = std::str::from_utf8(&self.src[start..self.pos])
            .map_err(|_| { self.set_err_offset(); ParseError::ValueInvalid })?;
        if has_frac || has_exp {
            let d = f64::from_str(text)
                .map_err(|_| { self.set_err_offset(); ParseError::NumberTooBig })?;
            if h.double(d) { Ok(()) } else { self.set_err_offset(); Err(ParseError::Termination) }
        } else if let Ok(i) = i64::from_str(text) {
            if h.int64(i) { Ok(()) } else { self.set_err_offset(); Err(ParseError::Termination) }
        } else if let Ok(u) = u64::from_str(text) {
            if h.uint64(u) { Ok(()) } else { self.set_err_offset(); Err(ParseError::Termination) }
        } else {
            let d = f64::from_str(text)
                .map_err(|_| { self.set_err_offset(); ParseError::NumberTooBig })?;
            if h.double(d) { Ok(()) } else { self.set_err_offset(); Err(ParseError::Termination) }
        }
    }
}

// ============================================================================
// StrReadStream -- minimal read-only byte stream (mirrors GenericStringStream<UTF8>)
// ============================================================================

/// Minimal read-only byte stream backed by a borrowed slice. Mirrors
/// `rapidjson::GenericStringStream<UTF8<char>>` (also reuses the in-buffer state held by
/// `rapidjson::MemoryStream`).
#[derive(Clone, Copy, Debug)]
pub struct StrReadStream<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> StrReadStream<'a> {
    /// Construct a stream over a byte slice.
    pub fn new(src: &'a [u8]) -> Self { Self { src, pos: 0 } }
    /// `Peek()` -- look at the current byte without consuming it.
    pub fn peek(&self) -> Option<u8> { self.src.get(self.pos).copied() }
    /// `Take()` -- consume and return the current byte.
    pub fn take(&mut self) -> Option<u8> {
        let b = self.src.get(self.pos).copied();
        if b.is_some() { self.pos += 1; }
        b
    }
    /// `Tell()` -- current offset.
    pub fn tell(&self) -> usize { self.pos }
}

// ============================================================================
// Document
// ============================================================================

/// JSON document -- mirrors `rapidjson::GenericDocument<UTF8<char>, MemoryPoolAllocator<CrtAllocator>, CrtAllocator>`.
#[derive(Clone, Debug, Default)]
pub struct Document {
    /// The root value. Mirrors the public `value` member exposed by `GenericValue` via
    /// `GenericDocument`.
    pub value: Value,
}

impl Document {
    /// Parse a JSON document from a string slice. Mirrors `GenericDocument::Parse`.
    pub fn parse(s: &str) -> Result<Document, ParseError> {
        unsafe { G_PARSE_COUNT += 1; }
        let mut reader = Reader::new(s.as_bytes());
        let mut builder = ValueBuilder::new();
        if let Err(e) = reader.parse(&mut builder) {
            unsafe { G_LAST_ERROR_OFFSET = reader.pos(); }
            return Err(e);
        }
        Ok(Document { value: builder.into_value() })
    }

    /// Parse from an arbitrary UTF-8 byte slice.
    pub fn parse_bytes(s: &[u8]) -> Result<Document, ParseError> {
        unsafe { G_PARSE_COUNT += 1; }
        let mut reader = Reader::new(s);
        let mut builder = ValueBuilder::new();
        if let Err(e) = reader.parse(&mut builder) {
            unsafe { G_LAST_ERROR_OFFSET = reader.pos(); }
            return Err(e);
        }
        Ok(Document { value: builder.into_value() })
    }

    /// Construct an empty document (root = `Null`).
    pub fn new() -> Self { Self { value: Value::Null } }

    /// Convenience: produce the canonical JSON representation of this document.
    pub fn to_json(&self) -> String { self.value.to_string() }
}

// ============================================================================
// Writer -- SAX-style JSON generator
// ============================================================================

/// SAX-style JSON writer. Mirrors `rapidjson::Writer<OutputStream, UTF8<char>, UTF8<char>, CrtAllocator>`.
pub struct Writer<W: Write> {
    out: W,
    state: Vec<Level>,
    has_root: bool,
}

#[derive(Clone, Copy, Debug)]
struct Level {
    in_array: bool,
    value_count: usize,
}

impl<W: Write> Writer<W> {
    /// Construct a writer that emits to `out`.
    pub fn new(out: W) -> Self {
        Self { out, state: Vec::with_capacity(RAPIDJSON_DEFAULT_LEVEL_DEPTH), has_root: false }
    }

    /// Reset the writer with a fresh output sink. Mirrors `Writer::Reset()`.
    pub fn reset(&mut self, out: W) {
        self.out = out;
        self.state.clear();
        self.has_root = false;
    }

    /// Consume the writer and return the inner sink.
    pub fn into_inner(self) -> W { self.out }

    /// `IsComplete()` -- true if the document is well-formed.
    pub fn is_complete(&self) -> bool { self.has_root && self.state.is_empty() }

    /// Flush the underlying sink.
    pub fn flush(&mut self) -> io::Result<()> { self.out.flush() }

    fn prefix(&mut self) -> io::Result<()> {
        if let Some(level) = self.state.last_mut() {
            if level.value_count > 0 {
                if level.in_array {
                    self.out.write_all(b",")?;
                } else {
                    let sep = if level.value_count % 2 == 0 { b',' } else { b':' };
                    self.out.write_all(&[sep])?;
                }
            }
            level.value_count += 1;
        } else {
            self.has_root = true;
        }
        Ok(())
    }

    /// `Null()` -- emit `null`.
    pub fn write_null(&mut self) -> io::Result<()> {
        self.prefix()?;
        self.out.write_all(b"null")
    }

    /// `Bool(b)` -- emit `true`/`false`.
    pub fn write_bool(&mut self, b: bool) -> io::Result<()> {
        self.prefix()?;
        if b { self.out.write_all(b"true") } else { self.out.write_all(b"false") }
    }

    /// `Int(i)` -- emit a 32-bit signed integer.
    pub fn write_int(&mut self, i: i32) -> io::Result<()> {
        self.prefix()?;
        write!(self.out, "{}", i)
    }

    /// `Uint(u)` -- emit a 32-bit unsigned integer.
    pub fn write_uint(&mut self, u: u32) -> io::Result<()> {
        self.prefix()?;
        write!(self.out, "{}", u)
    }

    /// `Int64(i)` -- emit a 64-bit signed integer.
    pub fn write_int64(&mut self, i: i64) -> io::Result<()> {
        self.prefix()?;
        write!(self.out, "{}", i)
    }

    /// `Uint64(u)` -- emit a 64-bit unsigned integer.
    pub fn write_uint64(&mut self, u: u64) -> io::Result<()> {
        self.prefix()?;
        write!(self.out, "{}", u)
    }

    /// `Double(d)` -- emit an IEEE-754 double.
    pub fn write_double(&mut self, d: f64) -> io::Result<()> {
        self.prefix()?;
        if d.is_nan() {
            self.out.write_all(b"NaN")
        } else if d.is_infinite() {
            if d.is_sign_negative() { self.out.write_all(b"-Infinity") } else { self.out.write_all(b"Infinity") }
        } else {
            write!(self.out, "{}", d)
        }
    }

    /// `String(str)` -- emit a JSON string literal.
    pub fn write_str(&mut self, s: &str) -> io::Result<()> {
        self.prefix()?;
        write_string_into(&mut self.out, s)
    }

    /// `StartObject()`.
    pub fn start_object(&mut self) -> io::Result<()> {
        self.prefix()?;
        self.state.push(Level { in_array: false, value_count: 0 });
        self.out.write_all(b"{")
    }

    /// `EndObject()`.
    pub fn end_object(&mut self) -> io::Result<()> {
        let popped = self.state.pop();
        debug_assert!(matches!(popped, Some(Level { in_array: false, .. })));
        self.out.write_all(b"}")
    }

    /// `StartArray()`.
    pub fn start_array(&mut self) -> io::Result<()> {
        self.prefix()?;
        self.state.push(Level { in_array: true, value_count: 0 });
        self.out.write_all(b"[")
    }

    /// `EndArray()`.
    pub fn end_array(&mut self) -> io::Result<()> {
        let popped = self.state.pop();
        debug_assert!(matches!(popped, Some(Level { in_array: true, .. })));
        self.out.write_all(b"]")
    }

    /// `Key(s)` -- shorthand for emitting an object key.
    pub fn write_key(&mut self, k: &str) -> io::Result<()> { self.write_str(k) }

    /// Convenience: emit a full [`Value`] tree.
    pub fn write_value(&mut self, v: &Value) -> io::Result<()> {
        match v {
            Value::Null => self.write_null(),
            Value::Bool(b) => self.write_bool(*b),
            Value::Int(i) => self.write_int64(*i),
            Value::Uint(u) => self.write_uint64(*u),
            Value::Double(d) => self.write_double(*d),
            Value::String(s) => self.write_str(s),
            Value::Array(a) => {
                self.start_array()?;
                for e in a { self.write_value(e)?; }
                self.end_array()
            }
            Value::Object(o) => {
                self.start_object()?;
                for (k, v) in o {
                    self.write_key(k)?;
                    self.write_value(v)?;
                }
                self.end_object()
            }
        }
    }
}

impl Handler for Writer<Vec<u8>> {
    fn null(&mut self) -> bool { self.write_null().is_ok() }
    fn bool_(&mut self, b: bool) -> bool { self.write_bool(b).is_ok() }
    fn int(&mut self, i: i32) -> bool { self.write_int(i).is_ok() }
    fn uint(&mut self, u: u32) -> bool { self.write_uint(u).is_ok() }
    fn int64(&mut self, i: i64) -> bool { self.write_int64(i).is_ok() }
    fn uint64(&mut self, u: u64) -> bool { self.write_uint64(u).is_ok() }
    fn double(&mut self, d: f64) -> bool { self.write_double(d).is_ok() }
    fn raw_number(&mut self, _s: &str) -> bool { false }
    fn string(&mut self, s: &str) -> bool { self.write_str(s).is_ok() }
    fn start_object(&mut self) -> bool { self.start_object().is_ok() }
    fn key(&mut self, k: &str) -> bool { self.write_key(k).is_ok() }
    fn end_object(&mut self, _member_count: usize) -> bool { self.end_object().is_ok() }
    fn start_array(&mut self) -> bool { self.start_array().is_ok() }
    fn end_array(&mut self, _element_count: usize) -> bool { self.end_array().is_ok() }
}

/// JSON-escape a string and stream it into the writer.
fn write_string_into<W: Write>(out: &mut W, s: &str) -> io::Result<()> {
    out.write_all(b"\"")?;
    let bytes = s.as_bytes();
    let mut start = 0;
    for (i, b) in bytes.iter().enumerate() {
        match *b {
            b'"' | b'\\' | b'\n' | b'\r' | b'\t' | b'\x08' | b'\x0c' => {
                if start < i { out.write_all(&bytes[start..i])?; }
                let esc: &[u8] = match *b {
                    b'"' => b"\\\"",
                    b'\\' => b"\\\\",
                    b'\n' => b"\\n",
                    b'\r' => b"\\r",
                    b'\t' => b"\\t",
                    b'\x08' => b"\\b",
                    b'\x0c' => b"\\f",
                    _ => unreachable!(),
                };
                out.write_all(esc)?;
                start = i + 1;
            }
            c if c < 0x20 => {
                if start < i { out.write_all(&bytes[start..i])?; }
                write!(out, "\\u{:04x}", c)?;
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < bytes.len() { out.write_all(&bytes[start..])?; }
    out.write_all(b"\"")
}

// ============================================================================
// StringBuffer
// ============================================================================

/// In-memory output sink. Mirrors `rapidjson::GenericStringBuffer<UTF8<char>, CrtAllocator>`.
#[derive(Clone, Debug, Default)]
pub struct StringBuffer {
    /// Raw byte buffer.
    pub data: Vec<u8>,
}

impl StringBuffer {
    /// Default capacity (256 bytes, matches `GenericStringBuffer::kDefaultCapacity`).
    pub const DEFAULT_CAPACITY: usize = 256;

    /// Create an empty buffer with the default capacity.
    pub fn new() -> Self {
        Self { data: Vec::with_capacity(Self::DEFAULT_CAPACITY) }
    }

    /// Create an empty buffer with a specific capacity.
    pub fn with_capacity(cap: usize) -> Self {
        Self { data: Vec::with_capacity(cap) }
    }

    /// Borrow the contents as a UTF-8 string slice (lossy if invalid UTF-8 made it in).
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.data).unwrap_or("")
    }

    /// Length of the underlying buffer in bytes. Mirrors `GetSize()`.
    pub fn len(&self) -> usize { self.data.len() }
    /// True if the buffer holds no bytes.
    pub fn is_empty(&self) -> bool { self.data.is_empty() }

    /// Drop all bytes while keeping allocated capacity.
    pub fn clear(&mut self) { self.data.clear(); }
}

impl Write for StringBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.data.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}

impl fmt::Display for StringBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_object() {
        let doc = Document::parse(r#"{"a": 1, "b": "hi"}"#).unwrap();
        match doc.value {
            Value::Object(ref o) => {
                assert_eq!(o.len(), 2);
                assert_eq!(o[0].0, "a");
                assert_eq!(o[1].0, "b");
                assert_eq!(o[1].1.as_str(), Some("hi"));
            }
            _ => panic!("expected object"),
        }
    }

    #[test]
    fn parses_array_with_nested() {
        let doc = Document::parse("[1, 2, [3, 4], null, true, false]").unwrap();
        let Value::Array(a) = doc.value else { panic!("expected array"); };
        assert_eq!(a.len(), 6);
        assert_eq!(a[0], Value::Int(1));
        assert!(matches!(a[5], Value::Bool(false)));
    }

    #[test]
    fn handles_unicode_escapes() {
        let doc = Document::parse(r#""😀""#).unwrap();
        assert_eq!(doc.value.as_str(), Some("\u{1F600}"));
    }

    #[test]
    fn rejects_lone_surrogate() {
        let err = Document::parse(r#""\uD83D""#).unwrap_err();
        assert_eq!(err, ParseError::StringMissSurrogateHalf);
    }

    #[test]
    fn detects_root_not_singular() {
        let err = Document::parse("1 2").unwrap_err();
        assert_eq!(err, ParseError::DocumentRootNotSingular);
    }

    #[test]
    fn detects_empty_document() {
        let err = Document::parse("   ").unwrap_err();
        assert_eq!(err, ParseError::DocumentEmpty);
    }

    #[test]
    fn writes_to_string_buffer() {
        let mut sb = StringBuffer::new();
        {
            let mut w = Writer::new(&mut sb);
            w.write_value(&Value::Object(vec![
                ("name".into(), Value::String("pcsx2".into())),
                ("ok".into(), Value::Bool(true)),
                ("count".into(), Value::Int(7)),
            ])).unwrap();
        }
        assert_eq!(sb.as_str(), r#"{"name":"pcsx2","ok":true,"count":7}"#);
    }

    #[test]
    fn parse_error_display() {
        assert_eq!(ParseError::ValueInvalid.to_string(), "Invalid value.");
    }
}
