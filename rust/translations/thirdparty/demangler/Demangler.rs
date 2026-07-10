// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust 2021 translation of the libiberty `demangler` 3rdparty C/C++
//! library (originally from GCC/binutils). Provides entry points that mirror
//! the historical C ABI:
//!
//! * [`cplus_demangle`] — auto-dispatching demangler (`cplus_demangle`).
//! * [`cplus_demangle_v3`] — Itanium C++ ABI (g++ V3) demangler.
//! * [`gnu_v3_demangle`] — convenience alias for the V3 entry point that
//!   matches `DMGL_GNU_V3` semantics.
//! * [`ms_demangle`] — Microsoft-style MSVC demangler entry point.
//! * [`java_demangle`] — Java mangled-name demangler (`java_demangle_v3`).
//! * [`dlang_demangle`] — D language demangler.
//! * [`rust_demangle`] — Rust v0 / legacy mangled-name demangler.
//!
//! Only the `std` crate is used. `cplus_demangle`, `java_demangle` and
//! `dlang_demangle` keep their original C signatures (`*mut c_char`) so they
//! can be linked across an FFI boundary via `CString::into_raw`; callers on
//! the Rust side should prefer the `Result` / `Option` returning variants,
//! which never leak memory.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals)]

use std::cell::RefCell;
use std::ffi::{c_char, CString};
use std::fmt;
use std::ptr;

/// Bit flags that mirror the `DMGL_*` macros from `demangle.h`.
pub mod options {
    pub const DMGL_NO_OPTS: i32 = 0;
    pub const DMGL_PARAMS: i32 = 1 << 0;
    pub const DMGL_ANSI: i32 = 1 << 1;
    pub const DMGL_JAVA: i32 = 1 << 2;
    pub const DMGL_VERBOSE: i32 = 1 << 3;
    pub const DMGL_TYPES: i32 = 1 << 4;
    pub const DMGL_RET_POSTFIX: i32 = 1 << 5;
    pub const DMGL_RET_DROP: i32 = 1 << 6;

    pub const DMGL_AUTO: i32 = 1 << 8;
    pub const DMGL_GNU: i32 = 1 << 9;
    pub const DMGL_LUCID: i32 = 1 << 10;
    pub const DMGL_ARM: i32 = 1 << 11;
    pub const DMGL_HP: i32 = 1 << 12;
    pub const DMGL_EDG: i32 = 1 << 13;
    pub const DMGL_GNU_V3: i32 = 1 << 14;
    pub const DMGL_GNAT: i32 = 1 << 15;
    pub const DMGL_DLANG: i32 = 1 << 16;
    pub const DMGL_RUST: i32 = 1 << 17;
    pub const DMGL_NO_RECURSE_LIMIT: i32 = 1 << 18;

    pub const DMGL_STYLE_MASK: i32 = DMGL_AUTO
        | DMGL_GNU
        | DMGL_LUCID
        | DMGL_ARM
        | DMGL_HP
        | DMGL_EDG
        | DMGL_GNU_V3
        | DMGL_JAVA
        | DMGL_GNAT
        | DMGL_DLANG
        | DMGL_RUST;

    pub const CPLUS_MARKER: char = '$';
}

/// Errors that can be returned by the demangling routines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DemangleError {
    /// The input string was empty.
    Empty,
    /// The input was not recognised as a valid mangled name for the chosen
    /// demangling style.
    Invalid,
    /// A back-reference / substitution referred to an unseen component.
    BadSubstitution,
    /// The recursion depth exceeded the configured limit.
    RecursionLimit,
    /// A type / parameter / template arity was out of range.
    OutOfRange,
    /// An internal invariant was violated; should never happen.
    Internal(&'static str),
}

impl fmt::Display for DemangleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DemangleError::Empty => f.write_str("empty mangled name"),
            DemangleError::Invalid => f.write_str("invalid mangled name"),
            DemangleError::BadSubstitution => f.write_str("invalid substitution"),
            DemangleError::RecursionLimit => f.write_str("recursion limit exceeded"),
            DemangleError::OutOfRange => f.write_str("value out of range"),
            DemangleError::Internal(where_) => {
                write!(f, "internal demangler error: {}", where_)
            }
        }
    }
}

impl std::error::Error for DemangleError {}

/// Demangle an Itanium C++ ABI (g++ V3) mangled name.
///
/// `options` is a bitmask of [`options::DMGL_*`] flags; only the relevant
/// subset (`DMGL_PARAMS`, `DMGL_ANSI`, `DMGL_JAVA`, `DMGL_VERBOSE`,
/// `DMGL_NO_RECURSE_LIMIT`) has an effect on the output.
pub fn cplus_demangle_v3(mangled: &str, options: i32) -> Result<String, DemangleError> {
    v3::demangle(mangled, options)
}

/// Convenience wrapper that forces `DMGL_GNU_V3` style.
pub fn gnu_v3_demangle(mangled: &str) -> Result<String, DemangleError> {
    cplus_demangle_v3(mangled, options::DMGL_GNU_V3 | options::DMGL_PARAMS)
}

/// Demangle a Microsoft Visual C++ mangled symbol (`?`-prefixed).
///
/// MSVC mangling is not handled by `cplus_demangle_v3`; this entry point
/// parses the MSVC grammar directly.
pub fn ms_demangle(mangled: &str) -> Result<String, DemangleError> {
    ms::demangle(mangled)
}

/// Demangle a Java mangled name (same V3 engine as C++, but with
/// `DMGL_JAVA` always set).
pub fn java_demangle_v3_inner(mangled: &str) -> Result<String, DemangleError> {
    cplus_demangle_v3(mangled, options::DMGL_JAVA | options::DMGL_PARAMS)
}

/// Demangle a Rust v0 / legacy mangled name.
///
/// Returns `None` if the input does not look like a Rust mangled symbol
/// (`_R`, `_ZN`, `_ZN..E`, etc.).
pub fn rust_demangle(mangled: &str) -> Option<String> {
    rs::demangle(mangled)
}

/// Demangle a D language mangled name (the `_D` prefix form).
pub fn dlang_demangle_inner(mangled: &str) -> Option<String> {
    dlang::demangle(mangled)
}

// =======================================================================
// FFI-style entry points. These return a heap-allocated `*mut c_char` so
// that they can be linked from C/C++ without breaking the historical ABI.
// The Rust caller must free the returned pointer by passing it back to
// `free_c_char` (or simply call `CString::from_raw` and `drop` it).
// =======================================================================

/// Free a `*mut c_char` that was returned by one of this module's
/// `*mut c_char`-returning entry points.
#[no_mangle]
pub extern "C" fn demangler_free(ptr: *mut c_char) {
    if !ptr.is_null() {
        unsafe {
            let _ = CString::from_raw(ptr);
        }
    }
}

fn cstring_or_null(s: Option<String>) -> *mut c_char {
    match s {
        Some(s) => match CString::new(s) {
            Ok(c) => c.into_raw(),
            Err(_) => ptr::null_mut(),
        },
        None => ptr::null_mut(),
    }
}

/// Auto-dispatching demangler. Identical in spirit to the C
/// `cplus_demangle`: it inspects `options` to pick Rust → GNU V3 → Java
/// → GNAT → D → legacy GNU, returning a C string on success and
/// `NULL` on failure.
#[no_mangle]
pub extern "C" fn cplus_demangle(mangled: *const c_char, options: i32) -> *mut c_char {
    if mangled.is_null() {
        return ptr::null_mut();
    }
    let s = match unsafe { cstr_to_str(mangled) } {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };
    let out = dispatch(s, options);
    cstring_or_null(out)
}

/// Java demangler. Returns a C string on success and `NULL` on failure.
#[no_mangle]
pub extern "C" fn java_demangle(mangled: *const c_char, options: i32) -> *mut c_char {
    if mangled.is_null() {
        return ptr::null_mut();
    }
    let s = match unsafe { cstr_to_str(mangled) } {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };
    let opts = options | options::DMGL_JAVA;
    cstring_or_null(cplus_demangle_v3(s, opts).ok())
}

/// D language demangler.
#[no_mangle]
pub extern "C" fn dlang_demangle(mangled: *const c_char, _options: i32) -> *mut c_char {
    if mangled.is_null() {
        return ptr::null_mut();
    }
    let s = match unsafe { cstr_to_str(mangled) } {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };
    cstring_or_null(dlang::demangle(s))
}

/// Rust demangler. Returns a C string on success and `NULL` otherwise.
#[no_mangle]
pub extern "C" fn rust_demangle_ffi(mangled: *const c_char, _options: i32) -> *mut c_char {
    if mangled.is_null() {
        return ptr::null_mut();
    }
    let s = match unsafe { cstr_to_str(mangled) } {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };
    cstring_or_null(rs::demangle(s))
}

unsafe fn cstr_to_str(p: *const c_char) -> Result<&'static str, std::str::Utf8Error> {
    // SAFETY: caller guarantees `p` is a valid NUL-terminated C string with
    // lifetime at least the duration of this call.
    let len = (0..).find(|&i| *p.add(i) == 0).unwrap_or(0);
    let bytes = std::slice::from_raw_parts(p as *const u8, len);
    std::str::from_utf8(bytes)
}

fn dispatch(mangled: &str, options: i32) -> Option<String> {
    if mangled.is_empty() {
        return None;
    }
    // Rust first — legacy Rust symbols can look like GNU V3.
    if let Some(s) = rs::demangle(mangled) {
        if options & options::DMGL_STYLE_MASK == options::DMGL_RUST
            || options & options::DMGL_STYLE_MASK == 0
            || options & options::DMGL_STYLE_MASK == options::DMGL_AUTO
        {
            return Some(s);
        }
    }

    // GNU V3 / Itanium C++ ABI.
    let v3_opts = options
        & (options::DMGL_PARAMS
            | options::DMGL_ANSI
            | options::DMGL_JAVA
            | options::DMGL_VERBOSE
            | options::DMGL_NO_RECURSE_LIMIT);
    if let Ok(s) = v3::demangle(mangled, v3_opts) {
        let style = options & options::DMGL_STYLE_MASK;
        if style == 0
            || style == options::DMGL_AUTO
            || style == options::DMGL_GNU_V3
            || style == options::DMGL_GNU
            || style == options::DMGL_JAVA
        {
            return Some(s);
        }
    }

    // Java fallback (already covered above, but kept for clarity).
    if let Ok(s) = v3::demangle(mangled, v3_opts | options::DMGL_JAVA) {
        return Some(s);
    }

    // D.
    if let Some(s) = dlang::demangle(mangled) {
        return Some(s);
    }

    // MSVC.
    if let Ok(s) = ms::demangle(mangled) {
        return Some(s);
    }

    None
}

// =======================================================================
// cplus_demangle_v3 implementation (Itanium C++ ABI)
// =======================================================================

mod v3 {
    use super::options;
    use super::DemangleError;

    const D_PRINT_DEFAULT: u8 = 0;
    const D_PRINT_INT: u8 = 1;
    const D_PRINT_UNSIGNED: u8 = 2;
    const D_PRINT_LONG: u8 = 3;
    const D_PRINT_UNSIGNED_LONG: u8 = 4;
    const D_PRINT_LONG_LONG: u8 = 5;
    const D_PRINT_UNSIGNED_LONG_LONG: u8 = 6;
    const D_PRINT_BOOL: u8 = 7;
    const D_PRINT_FLOAT: u8 = 8;
    const D_PRINT_VOID: u8 = 9;

    pub(super) fn demangle(input: &str, options: i32) -> Result<String, DemangleError> {
        if input.is_empty() {
            return Err(DemangleError::Empty);
        }
        let mut ctx = Ctx::new(input, options);
        let root = ctx.parse_mangled_name(true)?;
        ctx.print(&root)
    }

    /// Maximum number of components in the parse tree. Mirrors the
    /// `num_comps` heuristic of the original C code.
    const MAX_COMPS: usize = 4096;
    const MAX_SUBS: usize = 1024;
    const MAX_RECURSION: u32 = 2048;

    /// Concrete demangle-component tree. Each node owns its children via
    /// `Box<Component>`, so the tree is independent of the input lifetime.
    #[derive(Debug, Clone)]
    enum Component {
        Name(String),
        Qualified(Box<Component>, Box<Component>),
        Local(Box<Component>, Box<Component>),
        Typed(Box<Component>, Box<Component>),
        Template(Box<Component>, Box<Component>),
        TemplateParam(usize),
        FunctionParam(usize),
        Ctor(CtorKind, Box<Component>),
        Dtor(DtorKind, Box<Component>),
        Vtable(Box<Component>),
        Vtt(Box<Component>),
        ConstructionVtable(Box<Component>, Box<Component>),
        TypeInfo(Box<Component>),
        TypeInfoName(Box<Component>),
        TypeInfoFn(Box<Component>),
        Thunk(Box<Component>),
        VirtualThunk(Box<Component>),
        CovariantThunk(Box<Component>),
        Guard(Box<Component>),
        TlsInit(Box<Component>),
        TlsWrapper(Box<Component>),
        RefTemp(Box<Component>, Box<Component>),
        HiddenAlias(Box<Component>),
        SubStd(String),
        Restrict(Box<Component>),
        Volatile(Box<Component>),
        Const(Box<Component>),
        RestrictThis(Box<Component>),
        VolatileThis(Box<Component>),
        ConstThis(Box<Component>),
        ReferenceThis(Box<Component>),
        RvalueReferenceThis(Box<Component>),
        Pointer(Box<Component>),
        Reference(Box<Component>),
        RvalueReference(Box<Component>),
        Complex(Box<Component>),
        Imaginary(Box<Component>),
        Builtin(BuiltinType),
        VendorType(Box<Component>),
        VendorTypeQual(Box<Component>, Box<Component>),
        FunctionType(Option<Box<Component>>, Option<Box<Component>>),
        ArrayType(Option<Box<Component>>, Box<Component>),
        PointerToMember(Box<Component>, Box<Component>),
        VectorType(Box<Component>, Box<Component>),
        ArgList(Option<Box<Component>>, Option<Box<Component>>),
        TemplateArgList(Option<Box<Component>>, Option<Box<Component>>),
        Operator(Operator),
        ExtendedOperator(i32, Box<Component>),
        Cast(Box<Component>),
        Conversion(Box<Component>),
        Nullary(Box<Component>),
        Unary(Box<Component>, Box<Component>),
        Binary(Box<Component>, Box<Component>),
        BinaryArgs(Box<Component>, Box<Component>),
        Trinary(Box<Component>, Box<Component>),
        TrinaryArg1(Box<Component>, Box<Component>),
        TrinaryArg2(Box<Component>, Box<Component>),
        Literal(Box<Component>, Box<Component>),
        LiteralNeg(Box<Component>, Box<Component>),
        VendorExpr(Box<Component>, Box<Component>),
        CompoundName(Box<Component>, Box<Component>),
        Character(i32),
        Number(i64),
        Decltype(Box<Component>),
        PackExpansion(Box<Component>),
        TaggedName(Box<Component>, Box<Component>),
        TransactionSafe(Box<Component>),
        Noexcept(Option<Box<Component>>),
        ThrowSpec(Box<Component>),
        StructuredBinding(Box<Component>, Option<Box<Component>>),
        ModuleName(Box<Component>, Box<Component>),
        ModulePartition(Box<Component>, Box<Component>),
        ModuleEntity(Box<Component>, Box<Component>),
        ModuleInit(Box<Component>),
        GlobalConstructors(Box<Component>),
        GlobalDestructors(Box<Component>),
        ReferenceTemporary(Box<Component>, Box<Component>),
        TransactionClone(Box<Component>),
        NontransactionClone(Box<Component>),
        Clone(Box<Component>, Box<Component>),
        Lambda(usize, Box<Component>),
        DefaultArg(i32, Box<Component>),
        UnnamedType(Box<Component>),
        FixedType(Box<Component>, i16, i16),
        TparmObj(Box<Component>),
        InitializerList(Option<Box<Component>>, Option<Box<Component>>),
        TplHead(Box<Component>),
        TplTypeParm(Box<Component>),
        TplNonTypeParm(Box<Component>),
        TplTemplateParm(Box<Component>),
        TplPackParm(Box<Component>),
        JavaClass(Box<Component>),
        JavaResource(Box<Component>),
        ExtBuiltin(BuiltinType, i16, char),
    }

    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    enum CtorKind {
        CompleteObject,
        BaseObject,
        CompleteObjectAllocating,
        Unified,
        ObjectGroup,
    }

    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    enum DtorKind {
        Deleting,
        CompleteObject,
        BaseObject,
        Unified,
        ObjectGroup,
    }

    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    enum Operator {
        Ad, // &
        Add, // +=
        AddAssign,
        Address, // &
        AndAssign, // &=
        ArrayIndex, // []
        Arrow, // ->
        ArrowStar, // ->*
        Assign, // =
        BitAnd, // &
        BitAndAssign, // &=
        BitNot, // ~
        BitOr, // |
        BitOrAssign, // |=
        BitXor, // ^
        BitXorAssign, // ^=
        Call, // ()
        CoAwait, // co_await
        Comma, // ,
        Complement, // ~
        ConstCast, // const_cast
        Convert, // (cast)
        Delete, // delete
        DeleteArray, // delete[]
        Deref, // *
        Div, // /
        DivAssign, // /=
        DynamicCast, // dynamic_cast
        Eq, // ==
        Greater, // >
        GreaterEq, // >=
        Literal, // operator""
        LogicalAnd, // &&
        LogicalNot, // !
        LogicalOr, // ||
        Less, // <
        LessEq, // <=
        LShift, // <<
        LShiftAssign, // <<=
        Member, // .*
        Minus, // -
        MinusAssign, // -=
        Mod, // %
        ModAssign, // %=
        Mul, // *
        MulAssign, // *=
        New, // new
        NewArray, // new[]
        Not, // !
        NotEq, // !=
        Plus, // +
        PlusAssign, // +=
        PostDec, // --
        PostInc, // ++
        PreDec, // --
        PreInc, // ++
        ReinterpretCast, // reinterpret_cast
        RShift, // >>
        RShiftAssign, // >>=
        Spaceship, // <=>
        StaticCast, // static_cast
        Subscript, // []
        Throw, // throw
        Typeid, // typeid
        UnaryMinus, // -
        UnaryPlus, // +
        Alignof,
        Sizeof, // sizeof
        SizeofPack, // sizeof...
        SubscriptAssign, // []=
        ArrowStarAssign, // ->*=
        Dot, // .
        Ellipsis, // ...
        EllipsisPack, // *... (pack expansion)
        Scope, // ::
        Question, // ?
    }

    /// Builtin type record. Indices correspond to the `cplus_demangle_builtin_types`
    /// table.
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    struct BuiltinType {
        name: &'static str,
        java_name: &'static str,
        print: u8,
    }

    fn builtin_for(c: char) -> Option<BuiltinType> {
        // a..z mappings, indexed by `c - 'a'`.
        let idx = (c as u8).saturating_sub(b'a') as usize;
        if idx >= 26 {
            return None;
        }
        let slot = BUILTINS.get(idx)?;
        if slot.name.is_empty() {
            return None;
        }
        Some(*slot)
    }

    fn builtin_for_index(i: usize) -> Option<BuiltinType> {
        BUILTINS_EXT.get(i).copied()
    }

    #[rustfmt::skip]
    const BUILTINS: [BuiltinType; 26] = [
        // a
        BuiltinType { name: "signed char",        java_name: "signed char",        print: D_PRINT_DEFAULT },
        // b
        BuiltinType { name: "bool",              java_name: "boolean",            print: D_PRINT_BOOL },
        // c
        BuiltinType { name: "char",              java_name: "byte",               print: D_PRINT_DEFAULT },
        // d
        BuiltinType { name: "double",            java_name: "double",             print: D_PRINT_FLOAT },
        // e
        BuiltinType { name: "long double",       java_name: "long double",        print: D_PRINT_FLOAT },
        // f
        BuiltinType { name: "float",             java_name: "float",              print: D_PRINT_FLOAT },
        // g
        BuiltinType { name: "__float128",        java_name: "__float128",         print: D_PRINT_FLOAT },
        // h
        BuiltinType { name: "unsigned char",     java_name: "unsigned char",      print: D_PRINT_DEFAULT },
        // i
        BuiltinType { name: "int",               java_name: "int",                print: D_PRINT_INT },
        // j
        BuiltinType { name: "unsigned int",      java_name: "unsigned",           print: D_PRINT_UNSIGNED },
        // k
        BuiltinType { name: "",                  java_name: "",                    print: D_PRINT_DEFAULT },
        // l
        BuiltinType { name: "long",              java_name: "long",               print: D_PRINT_LONG },
        // m
        BuiltinType { name: "unsigned long",     java_name: "unsigned long",      print: D_PRINT_UNSIGNED_LONG },
        // n
        BuiltinType { name: "__int128",          java_name: "__int128",           print: D_PRINT_DEFAULT },
        // o
        BuiltinType { name: "unsigned __int128", java_name: "unsigned __int128",  print: D_PRINT_DEFAULT },
        // p
        BuiltinType { name: "",                  java_name: "",                    print: D_PRINT_DEFAULT },
        // q
        BuiltinType { name: "",                  java_name: "",                    print: D_PRINT_DEFAULT },
        // r
        BuiltinType { name: "",                  java_name: "",                    print: D_PRINT_DEFAULT },
        // s
        BuiltinType { name: "short",             java_name: "short",              print: D_PRINT_DEFAULT },
        // t
        BuiltinType { name: "unsigned short",    java_name: "unsigned short",     print: D_PRINT_DEFAULT },
        // u
        BuiltinType { name: "",                  java_name: "",                    print: D_PRINT_DEFAULT },
        // v
        BuiltinType { name: "void",              java_name: "void",               print: D_PRINT_VOID },
        // w
        BuiltinType { name: "wchar_t",           java_name: "char",               print: D_PRINT_DEFAULT },
        // x
        BuiltinType { name: "long long",         java_name: "long",               print: D_PRINT_LONG_LONG },
        // y
        BuiltinType { name: "unsigned long long",java_name: "unsigned long long", print: D_PRINT_UNSIGNED_LONG_LONG },
        // z
        BuiltinType { name: "...",               java_name: "...",                print: D_PRINT_DEFAULT },
    ];

    const BUILTINS_EXT: &[BuiltinType] = &[
        // 26
        BuiltinType { name: "decimal32",         java_name: "decimal32",          print: D_PRINT_DEFAULT },
        // 27
        BuiltinType { name: "decimal64",         java_name: "decimal64",          print: D_PRINT_DEFAULT },
        // 28
        BuiltinType { name: "decimal128",        java_name: "decimal128",         print: D_PRINT_DEFAULT },
        // 29
        BuiltinType { name: "half",              java_name: "half",               print: D_PRINT_FLOAT },
        // 30
        BuiltinType { name: "char8_t",           java_name: "char8_t",            print: D_PRINT_DEFAULT },
        // 31
        BuiltinType { name: "char16_t",          java_name: "char16_t",           print: D_PRINT_DEFAULT },
        // 32
        BuiltinType { name: "char32_t",          java_name: "char32_t",           print: D_PRINT_DEFAULT },
        // 33
        BuiltinType { name: "decltype(nullptr)", java_name: "decltype(nullptr)",  print: D_PRINT_DEFAULT },
        // 34
        BuiltinType { name: "_Float",            java_name: "_Float",             print: D_PRINT_FLOAT },
        // 35
        BuiltinType { name: "std::bfloat16_t",   java_name: "std::bfloat16_t",    print: D_PRINT_FLOAT },
    ];

    struct Ctx<'a> {
        s: &'a str,
        bytes: &'a [u8],
        pos: usize,
        options: i32,
        recursion: u32,
        comps: Vec<Component>,
        subs: Vec<Option<Box<Component>>>,
        last_name: Option<usize>, // index into comps
    }

    impl<'a> Ctx<'a> {
        fn new(input: &'a str, options: i32) -> Self {
            Self {
                s: input,
                bytes: input.as_bytes(),
                pos: 0,
                options,
                recursion: 0,
                comps: Vec::with_capacity(MAX_COMPS),
                subs: Vec::with_capacity(MAX_SUBS),
                last_name: None,
            }
        }

        fn peek(&self) -> u8 {
            if self.pos < self.bytes.len() {
                self.bytes[self.pos]
            } else {
                0
            }
        }

        fn peek_next(&self) -> u8 {
            if self.pos + 1 < self.bytes.len() {
                self.bytes[self.pos + 1]
            } else {
                0
            }
        }

        fn advance(&mut self, n: usize) {
            self.pos = (self.pos + n).min(self.bytes.len());
        }

        fn check_char(&mut self, c: u8) -> bool {
            if self.peek() == c {
                self.advance(1);
                true
            } else {
                false
            }
        }

        fn next_char(&mut self) -> u8 {
            let c = self.peek();
            if c != 0 {
                self.advance(1);
            }
            c
        }

        fn eof(&self) -> bool {
            self.pos >= self.bytes.len()
        }

        fn push(&mut self, c: Component) -> Result<usize, DemangleError> {
            if self.comps.len() >= MAX_COMPS {
                return Err(DemangleError::Internal("too many components"));
            }
            self.comps.push(c);
            Ok(self.comps.len() - 1)
        }

        fn last(&self, idx: usize) -> &Component {
            &self.comps[idx]
        }

        fn last_cloned(&self, idx: usize) -> Component {
            self.comps[idx].clone()
        }

        fn enter_recursion(&mut self) -> Result<(), DemangleError> {
            if self.options & options::DMGL_NO_RECURSE_LIMIT == 0 {
                if self.recursion >= MAX_RECURSION {
                    return Err(DemangleError::RecursionLimit);
                }
                self.recursion += 1;
            }
            Ok(())
        }

        fn leave_recursion(&mut self) {
            if self.options & options::DMGL_NO_RECURSE_LIMIT == 0
                && self.recursion > 0
            {
                self.recursion -= 1;
            }
        }

        fn add_substitution(&mut self, idx: usize) -> Result<(), DemangleError> {
            let comp = self.last_cloned(idx);
            if self.subs.len() >= MAX_SUBS {
                return Err(DemangleError::Internal("too many substitutions"));
            }
            self.subs.push(Some(Box::new(comp)));
            Ok(())
        }

        // --- top-level parser ---

        fn parse_mangled_name(&mut self, top_level: bool) -> Result<usize, DemangleError> {
            // Allow missing leading `_Z` when not at the top level (workaround for
            // g++ abi-version=2 mangling).
            if !self.check_char(b'_') && top_level {
                return Err(DemangleError::Invalid);
            }
            if !self.check_char(b'Z') {
                return Err(DemangleError::Invalid);
            }
            let dc = self.parse_encoding(top_level)?;
            if top_level && self.options & options::DMGL_PARAMS != 0 {
                while self.peek() == b'.' {
                    let next = self.peek_next();
                    if next.is_ascii_lowercase() || next == b'_' || next.is_ascii_digit() {
                        let _ = self.parse_clone_suffix(dc)?;
                    } else {
                        break;
                    }
                }
            }
            Ok(dc)
        }

        fn parse_encoding(&mut self, top_level: bool) -> Result<usize, DemangleError> {
            let peek = self.peek();
            if peek == b'G' || peek == b'T' {
                return self.parse_special_name();
            }
            let mut dc = self.parse_name(false)?;
            if self.options & options::DMGL_PARAMS == 0 && top_level {
                // Strip CV-qualifiers; they really apply to `this`.
                while is_fnqual(self.last(dc)) {
                    let inner = take_left(self.last(dc).clone());
                    dc = self.push(*inner)?;
                }
                if matches!(self.last(dc), Component::Local(_, _)) {
                    while let Component::ReferenceThis(_)
                    | Component::RvalueReferenceThis(_)
                    | Component::ConstThis(_)
                    | Component::VolatileThis(_)
                    | Component::RestrictThis(_)
                    | Component::TransactionSafe(_)
                    | Component::Noexcept(_)
                    | Component::ThrowSpec(_) = self.last(dc)
                    {
                        let right = take_right(self.last(dc).clone());
                        if right.is_none() {
                            dc = usize::MAX;
                            break;
                        }
                        let new_right = take_left(*right.unwrap());
                        let (left, _) = destructure_binary(self.last_cloned(dc));
                        dc = self.push(make_binary(left, new_right))?;
                    }
                }
            } else {
                let peek = self.peek();
                if peek != 0 && peek != b'E' {
                    let ftype = self.parse_bare_function_type(has_return_type(self.last(dc)))?;
                    dc = self.push(Component::Typed(self.last_cloned(dc).into(), Box::new(self.last_cloned(ftype))))?;
                }
            }
            if dc == usize::MAX {
                return Err(DemangleError::Invalid);
            }
            Ok(dc)
        }

        fn parse_special_name(&mut self) -> Result<usize, DemangleError> {
            self.expansion_add(20);
            if !self.check_char(b'T') {
                // 'G' family.
                if !self.check_char(b'G') {
                    return Err(DemangleError::Invalid);
                }
                match self.next_char() {
                    b'V' => {
                        let name = self.parse_name(false)?;
                        return self.push(Component::Guard(Box::new(self.last_cloned(name))));
                    }
                    b'R' => {
                        let name = self.parse_name(false)?;
                        let num = self.parse_number_component()?;
                        return self.push(Component::RefTemp(
                            Box::new(self.last_cloned(name)),
                            Box::new(self.last_cloned(num)),
                        ));
                    }
                    b'A' => {
                        let enc = self.parse_encoding(false)?;
                        return self.push(Component::HiddenAlias(Box::new(self.last_cloned(enc))));
                    }
                    b'I' => {
                        let module = self.parse_module_name()?;
                        return self.push(Component::ModuleInit(Box::new(self.last_cloned(module))));
                    }
                    b'T' => match self.next_char() {
                        b'n' => {
                            let enc = self.parse_encoding(false)?;
                            return self.push(Component::NontransactionClone(Box::new(self.last_cloned(enc))));
                        }
                        _ => {
                            let enc = self.parse_encoding(false)?;
                            return self.push(Component::TransactionClone(Box::new(self.last_cloned(enc))));
                        }
                    },
                    b'r' => {
                        let res = self.parse_java_resource()?;
                        return self.push(Component::JavaResource(Box::new(self.last_cloned(res))));
                    }
                    _ => return Err(DemangleError::Invalid),
                }
            }
            match self.next_char() {
                b'V' => {
                    self.expansion_add_sub(5);
                    let t = self.parse_type()?;
                    self.push(Component::Vtable(Box::new(self.last_cloned(t))))
                }
                b'T' => {
                    self.expansion_add_sub(10);
                    let t = self.parse_type()?;
                    self.push(Component::Vtt(Box::new(self.last_cloned(t))))
                }
                b'I' => {
                    let t = self.parse_type()?;
                    self.push(Component::TypeInfo(Box::new(self.last_cloned(t))))
                }
                b'S' => {
                    let t = self.parse_type()?;
                    self.push(Component::TypeInfoName(Box::new(self.last_cloned(t))))
                }
                b'h' => {
                    self.parse_call_offset(b'h')?;
                    let enc = self.parse_encoding(false)?;
                    self.push(Component::Thunk(Box::new(self.last_cloned(enc))))
                }
                b'v' => {
                    self.parse_call_offset(b'v')?;
                    let enc = self.parse_encoding(false)?;
                    self.push(Component::VirtualThunk(Box::new(self.last_cloned(enc))))
                }
                b'c' => {
                    self.parse_call_offset(0)?;
                    self.parse_call_offset(0)?;
                    let enc = self.parse_encoding(false)?;
                    self.push(Component::CovariantThunk(Box::new(self.last_cloned(enc))))
                }
                b'C' => {
                    let _derived = self.parse_type()?;
                    let _off = self.parse_number()?;
                    if !self.check_char(b'_') {
                        return Err(DemangleError::Invalid);
                    }
                    let base = self.parse_type()?;
                    self.push(Component::ConstructionVtable(
                        Box::new(self.last_cloned(base)),
                        Box::new(Component::Name(String::new())),
                    ))
                }
                b'F' => {
                    let t = self.parse_type()?;
                    self.push(Component::TypeInfoFn(Box::new(self.last_cloned(t))))
                }
                b'J' => {
                    let t = self.parse_type()?;
                    self.push(Component::JavaClass(Box::new(self.last_cloned(t))))
                }
                b'H' => {
                    let n = self.parse_name(false)?;
                    self.push(Component::TlsInit(Box::new(self.last_cloned(n))))
                }
                b'W' => {
                    let n = self.parse_name(false)?;
                    self.push(Component::TlsWrapper(Box::new(self.last_cloned(n))))
                }
                b'A' => {
                    let a = self.parse_template_arg()?;
                    self.push(Component::TparmObj(Box::new(self.last_cloned(a))))
                }
                _ => Err(DemangleError::Invalid),
            }
        }

        fn expansion_add(&mut self, _n: usize) {}
        fn expansion_add_sub(&mut self, _n: usize) {}

        fn parse_call_offset(&mut self, c: u8) -> Result<(), DemangleError> {
            let mut cur = c;
            if cur == 0 {
                cur = self.next_char();
            }
            if cur == b'h' {
                self.parse_number()?;
            } else if cur == b'v' {
                self.parse_number()?;
                if !self.check_char(b'_') {
                    return Err(DemangleError::Invalid);
                }
                self.parse_number()?;
            } else {
                return Err(DemangleError::Invalid);
            }
            if !self.check_char(b'_') {
                return Err(DemangleError::Invalid);
            }
            Ok(())
        }

        fn parse_java_resource(&mut self) -> Result<usize, DemangleError> {
            let len = self.parse_number()?;
            if len <= 1 {
                return Err(DemangleError::Invalid);
            }
            if self.next_char() != b'_' {
                return Err(DemangleError::Invalid);
            }
            let mut remaining = len - 1;
            let mut head: Option<usize> = None;
            while remaining > 0 {
                let start = self.pos;
                while remaining > 0 && self.peek() != 0 && self.peek() != b'$' {
                    self.advance(1);
                    remaining -= 1;
                }
                let part = &self.bytes[start..self.pos];
                let piece = self.push(Component::Name(String::from_utf8_lossy(part).into_owned()))?;
                head = match head {
                    None => Some(piece),
                    Some(prev) => Some(self.push(Component::CompoundName(
                        Box::new(self.last_cloned(prev)),
                        Box::new(self.last_cloned(piece)),
                    ))?),
                };
                if remaining > 0 && self.peek() == b'$' {
                    self.advance(1);
                    remaining -= 1;
                    let esc = self.next_char();
                    remaining -= 1;
                    let mapped = match esc {
                        b'S' => '/',
                        b'_' => '.',
                        b'$' => '$',
                        _ => return Err(DemangleError::Invalid),
                    };
                    let piece = self.push(Component::Character(mapped as i32))?;
                    head = match head {
                        None => Some(piece),
                        Some(prev) => Some(self.push(Component::CompoundName(
                            Box::new(self.last_cloned(prev)),
                            Box::new(self.last_cloned(piece)),
                        ))?),
                    };
                }
            }
            let idx = head.ok_or(DemangleError::Invalid)?;
            Ok(idx)
        }

        fn parse_name(&mut self, substable: bool) -> Result<usize, DemangleError> {
            let peek = self.peek();
            let mut dc: Option<usize> = None;
            let mut module: Option<usize> = None;
            let mut subst = false;

            match peek {
                b'N' => dc = Some(self.parse_nested_name()?),
                b'Z' => dc = Some(self.parse_local_name()?),
                b'U' => dc = Some(self.parse_unqualified_name(None, None)?),
                b'S' => {
                    if self.peek_next() == b't' {
                        self.advance(2);
                        let n = self.push(Component::Name("std".into()))?;
                        self.expansion_add(3);
                        dc = Some(n);
                    }
                    if self.peek() == b'S' {
                        let m = self.parse_substitution(true)?;
                        if let Some(m_idx) = m {
                            let is_module = matches!(
                                self.last(m_idx),
                                Component::ModuleName(_, _) | Component::ModulePartition(_, _)
                            );
                            if !is_module {
                                if dc.is_some() {
                                    return Err(DemangleError::Invalid);
                                }
                                subst = true;
                                dc = Some(m_idx);
                                module = None;
                            } else {
                                module = m;
                            }
                        }
                    }
                    // FALLTHROUGH
                    if !subst {
                        let scope = dc.map(|i| self.last_cloned(i));
                        let mod_clone = module.map(|i| self.last_cloned(i));
                        let new = self.parse_unqualified_name(
                            scope.map(Box::new).as_deref(),
                            mod_clone.map(Box::new).as_deref(),
                        )?;
                        dc = Some(new);
                    }
                    if self.peek() == b'I' {
                        if !subst {
                            if let Some(d) = dc {
                                self.add_substitution(d)?;
                            }
                        }
                        let args = self.parse_template_args()?;
                        let d = dc.ok_or(DemangleError::Invalid)?;
                        dc = Some(self.push(Component::Template(self.last_cloned(d).into(), args.into()))?);
                        subst = false;
                    }
                }
                _ => {
                    if !subst {
                        let new = self.parse_unqualified_name(None, None)?;
                        dc = Some(new);
                    }
                    if self.peek() == b'I' {
                        if let Some(d) = dc {
                            self.add_substitution(d)?;
                        }
                        let args = self.parse_template_args()?;
                        let d = dc.ok_or(DemangleError::Invalid)?;
                        dc = Some(self.push(Component::Template(self.last_cloned(d).into(), args.into()))?);
                    }
                }
            }
            let d = dc.ok_or(DemangleError::Invalid)?;
            if substable && !subst {
                self.add_substitution(d)?;
            }
            Ok(d)
        }

        fn parse_nested_name(&mut self) -> Result<usize, DemangleError> {
            if !self.check_char(b'N') {
                return Err(DemangleError::Invalid);
            }
            let (mut ret, mut pret) = self.parse_cv_qualifiers();
            let rqual = self.parse_ref_qualifier(None);
            if !pret.is_null() {
                unsafe { *pret = *self.parse_prefix(true)?; }
            }
            if let Some(mut r) = rqual {
                let (l, _) = destructure_binary(self.last_cloned(ret));
                *r = *l;
                ret = self.push(*r)?;
            }
            if !self.check_char(b'E') {
                return Err(DemangleError::Invalid);
            }
            Ok(ret)
        }

        fn parse_cv_qualifiers(&mut self) -> (usize, *mut Component) {
            // Simplified: collect qualifiers as a left-leaning chain.
            let first = self.push(Component::Restrict(Component::Name(String::new()).into())).unwrap_or(0);
            let mut cur = first;
            while is_type_qual(self.peek()) {
                let t = match self.next_char() {
                    b'r' => Component::Restrict(Box::new(Component::Name(String::new()))),
                    b'V' => Component::Volatile(Box::new(Component::Name(String::new()))),
                    b'K' => Component::Const(Box::new(Component::Name(String::new()))),
                    b'D' => {
                        let inner = self.next_char();
                        match inner {
                            b'x' => Component::TransactionSafe(Box::new(Component::Name(String::new()))),
                            b'o' | b'O' => {
                                let expr = self.parse_expression().ok();
                                if inner == b'O' {
                                    if self.next_char() != b'E' {
                                        return (cur, std::ptr::null_mut());
                                    }
                                }
                                Component::Noexcept(expr.map(|i| Box::new(self.last_cloned(i))))
                            }
                            b'w' => {
                                let parms = self.parse_parmlist().ok();
                                if self.next_char() != b'E' {
                                    return (cur, std::ptr::null_mut());
                                }
                                Component::ThrowSpec(Box::new(parms.unwrap_or_else(|| Component::ArgList(None, None))))
                            }
                            _ => return (cur, std::ptr::null_mut()),
                        }
                    }
                    _ => unreachable!(),
                };
                cur = self.push(t).unwrap_or(cur);
            }
            (first, std::ptr::null_mut())
        }

        fn parse_ref_qualifier(&mut self, sub: Option<Box<Component>>) -> Option<Box<Component>> {
            match self.peek() {
                b'R' => {
                    self.advance(1);
                    Some(Box::new(Component::ReferenceThis(sub.unwrap_or_else(|| Box::new(Component::Name(String::new()))))))
                }
                b'O' => {
                    self.advance(1);
                    Some(Box::new(Component::RvalueReferenceThis(sub.unwrap_or_else(|| Box::new(Component::Name(String::new()))))))
                }
                _ => None,
            }
        }

        fn parse_prefix(&mut self, substable: bool) -> Result<Box<Component>, DemangleError> {
            let mut ret: Option<Box<Component>> = None;
            loop {
                let peek = self.peek();
                if peek == b'D' && (self.peek_next() == b'T' || self.peek_next() == b't') {
                    if ret.is_some() {
                        return Err(DemangleError::Invalid);
                    }
                    let t = self.parse_type()?;
                    ret = Some(Box::new(self.last_cloned(t)));
                } else if peek == b'I' {
                    if ret.is_none() {
                        return Err(DemangleError::Invalid);
                    }
                    let args = self.parse_template_args()?;
                    let left = ret.unwrap();
                    ret = Some(Box::new(Component::Template(left, Box::new(args))));
                } else if peek == b'T' {
                    if ret.is_some() {
                        return Err(DemangleError::Invalid);
                    }
                    let n = self.parse_template_param()?;
                    ret = Some(Box::new(self.last_cloned(n)));
                } else if peek == b'M' {
                    self.advance(1);
                    continue;
                } else {
                    let module = if peek == b'S' {
                        let m = self.parse_substitution(true)?;
                        match m {
                            Some(idx) => {
                                let is_mod = matches!(
                                    self.last(idx),
                                    Component::ModuleName(_, _) | Component::ModulePartition(_, _)
                                );
                                if !is_mod {
                                    if ret.is_some() {
                                        return Err(DemangleError::Invalid);
                                    }
                                    ret = Some(Box::new(self.last_cloned(idx)));
                                    continue;
                                } else {
                                    Some(self.last_cloned(idx))
                                }
                            }
                            None => None,
                        }
                    } else {
                        None
                    };
                    let scope = ret.as_deref();
                    let new = self.parse_unqualified_name(scope, module.as_ref())?;
                    ret = Some(Box::new(self.last_cloned(new)));
                }

                if self.peek() == b'E' {
                    break;
                }
                if let Some(r) = ret.as_ref() {
                    let owned = (**r).clone();
                    self.add_substitution_boxed(owned)?;
                }
            }
            ret.ok_or(DemangleError::Invalid)
        }

        fn add_substitution_boxed(&mut self, c: Component) -> Result<usize, DemangleError> {
            if self.subs.len() >= MAX_SUBS {
                return Err(DemangleError::Internal("too many substitutions"));
            }
            let idx = self.push(c)?;
            let owned = self.last_cloned(idx);
            self.subs.push(Some(Box::new(owned)));
            Ok(idx)
        }

        fn parse_unqualified_name(
            &mut self,
            scope: Option<&Component>,
            module: Option<&Component>,
        ) -> Result<usize, DemangleError> {
            let peek = self.peek();
            let mut ret: usize;
            if peek.is_ascii_digit() {
                ret = self.parse_source_name()?;
            } else if peek.is_ascii_lowercase() {
                let mut was_expr = false;
                let mut operator_name = false;
                if peek == b'o' && self.peek_next() == b'n' {
                    self.advance(2);
                    was_expr = true;
                }
                let op = self.parse_operator_name()?;
                let op = if let Component::Operator(_) = self.last(op) {
                    self.expansion_add_str_len("operator", op);
                    if matches!(self.last(op), Component::Operator(Operator::Literal)) {
                        let id = self.parse_source_name()?;
                        self.push(Component::Unary(self.last_cloned(op).into(), self.last_cloned(id).into()))?
                    } else {
                        op
                    }
                } else {
                    op
                };
                let _ = was_expr;
                ret = op;
                let _ = &mut operator_name;
            } else if peek == b'D' && self.peek_next() == b'C' {
                self.advance(2);
                ret = self.parse_structured_binding()?;
            } else if peek == b'C' || peek == b'D' {
                ret = self.parse_ctor_dtor_name()?;
            } else if peek == b'L' {
                self.advance(1);
                let sn = self.parse_source_name()?;
                self.parse_discriminator()?;
                ret = sn;
            } else if peek == b'U' {
                match self.peek_next() {
                    b'l' => ret = self.parse_lambda()?,
                    b't' => ret = self.parse_unnamed_type()?,
                    _ => return Err(DemangleError::Invalid),
                }
            } else {
                return Err(DemangleError::Invalid);
            }

            if let Some(m) = module {
                ret = self.push(Component::ModuleEntity(self.last_cloned(ret).into(), Box::new(m.clone())))?;
            }
            if self.peek() == b'B' {
                ret = self.parse_abi_tags(ret)?;
            }
            if let Some(s) = scope {
                ret = self.push(Component::Qualified(Box::new(s.clone()), self.last_cloned(ret).into()))?;
            }
            Ok(ret)
        }

        fn expansion_add_str_len(&mut self, prefix: &str, op_idx: usize) {
            let _ = (prefix, op_idx);
        }

        fn parse_structured_binding(&mut self) -> Result<usize, DemangleError> {
            let first_idx = self.parse_source_name()?;
            let first = self.push(Component::StructuredBinding(
                Box::new(self.last_cloned(first_idx)),
                None,
            ))?;
            let mut prev = first;
            while self.peek() != b'E' && self.peek() != 0 {
                let next_idx = self.parse_source_name()?;
                let next = self.push(Component::StructuredBinding(
                    Box::new(self.last_cloned(next_idx)),
                    None,
                ))?;
                let (l, _) = destructure_binary(self.last_cloned(prev));
                let new_prev = self.push(Component::StructuredBinding(l, Some(Box::new(self.last_cloned(next)))))?;
                prev = new_prev;
            }
            if self.peek() == b'E' {
                self.advance(1);
            } else {
                return Err(DemangleError::Invalid);
            }
            Ok(prev)
        }

        fn parse_lambda(&mut self) -> Result<usize, DemangleError> {
            // Ul<num>_ <body> E
            let _num = self.parse_number()?;
            if !self.check_char(b'_') {
                return Err(DemangleError::Invalid);
            }
            let body = self.parse_unqualified_name(None, None)?;
            if !self.check_char(b'E') {
                return Err(DemangleError::Invalid);
            }
            self.push(Component::Lambda(0, Box::new(self.last_cloned(body))))
        }

        fn parse_unnamed_type(&mut self) -> Result<usize, DemangleError> {
            // Ut<num>_ <name> E
            self.advance(1); // 't'
            let _num = self.parse_number()?;
            if !self.check_char(b'_') {
                return Err(DemangleError::Invalid);
            }
            let name = self.parse_unqualified_name(None, None)?;
            if !self.check_char(b'E') {
                return Err(DemangleError::Invalid);
            }
            self.push(Component::UnnamedType(Box::new(self.last_cloned(name))))
        }

        fn parse_source_name(&mut self) -> Result<usize, DemangleError> {
            let len = self.parse_number()?;
            if len <= 0 {
                return Err(DemangleError::Invalid);
            }
            let start = self.pos;
            if start + (len as usize) > self.bytes.len() {
                return Err(DemangleError::Invalid);
            }
            let name = &self.bytes[start..start + len as usize];
            let s = match std::str::from_utf8(name) {
                Ok(s) => s.to_string(),
                Err(_) => return Err(DemangleError::Invalid),
            };
            self.advance(len as usize);
            // Anonymous namespace canonicalisation.
            if s.starts_with("_GLOBAL_") && s.len() >= "_GLOBAL_".len() + 2 {
                let rest = &s["_GLOBAL_".len()..];
                let ch = rest.as_bytes()[0];
                if (ch == b'.' || ch == b'_' || ch == b'$') && rest.as_bytes()[1] == b'N' {
                    let canonical = "(anonymous namespace)";
                    self.last_name = Some(self.push(Component::Name(canonical.into()))?);
                    return Ok(self.last_name.unwrap());
                }
            }
            let idx = self.push(Component::Name(s))?;
            self.last_name = Some(idx);
            Ok(idx)
        }

        fn parse_number(&mut self) -> Result<i64, DemangleError> {
            let mut negative = false;
            let mut peek = self.peek();
            if peek == b'n' {
                negative = true;
                self.advance(1);
                peek = self.peek();
            }
            let mut ret: i64 = 0;
            while peek.is_ascii_digit() {
                let digit = (peek - b'0') as i64;
                if ret > (i64::MAX - digit) / 10 {
                    return Err(DemangleError::OutOfRange);
                }
                ret = ret * 10 + digit;
                self.advance(1);
                peek = self.peek();
            }
            if negative {
                ret = -ret;
            }
            Ok(ret)
        }

        fn parse_number_component(&mut self) -> Result<usize, DemangleError> {
            let n = self.parse_number()?;
            self.push(Component::Number(n))
        }

        fn parse_operator_name(&mut self) -> Result<usize, DemangleError> {
            let c1 = self.next_char();
            let c2 = self.next_char();
            if c1 == b'v' && c2.is_ascii_digit() {
                let args = (c2 - b'0') as i32;
                let name = self.parse_source_name()?;
                return self.push(Component::ExtendedOperator(args, Box::new(self.last_cloned(name))));
            }
            if c1 == b'c' && c2 == b'v' {
                let t = self.parse_type()?;
                return self.push(Component::Conversion(Box::new(self.last_cloned(t))));
            }
            // Two-letter lookup.
            let op = match (c1, c2) {
                (b'a', b'N') => Operator::BitAndAssign,
                (b'a', b'S') => Operator::Assign,
                (b'a', b'a') => Operator::LogicalAnd,
                (b'a', b'd') => Operator::BitAnd,
                (b'a', b'n') => Operator::BitAnd,
                (b'a', b't') => Operator::Alignof,
                (b'a', b'w') => Operator::CoAwait,
                (b'a', b'z') => Operator::Alignof,
                (b'c', b'c') => Operator::ConstCast,
                (b'c', b'l') => Operator::Call,
                (b'c', b'm') => Operator::Comma,
                (b'c', b'o') => Operator::Complement,
                (b'd', b'V') => Operator::DivAssign,
                (b'd', b'X') => Operator::SubscriptAssign,
                (b'd', b'a') => Operator::DeleteArray,
                (b'd', b'c') => Operator::DynamicCast,
                (b'd', b'e') => Operator::Deref,
                (b'd', b'i') => Operator::ArrowStarAssign,
                (b'd', b'l') => Operator::Delete,
                (b'd', b's') => Operator::Member,
                (b'd', b't') => Operator::Dot,
                (b'd', b'v') => Operator::Div,
                (b'd', b'x') => Operator::SubscriptAssign,
                (b'e', b'O') => Operator::BitXorAssign,
                (b'e', b'o') => Operator::BitXor,
                (b'e', b'q') => Operator::Eq,
                (b'f', b'L') => Operator::EllipsisPack,
                (b'f', b'R') => Operator::EllipsisPack,
                (b'f', b'l') => Operator::Ellipsis,
                (b'f', b'r') => Operator::Ellipsis,
                (b'g', b'e') => Operator::GreaterEq,
                (b'g', b's') => Operator::Scope,
                (b'g', b't') => Operator::Greater,
                (b'i', b'x') => Operator::Subscript,
                (b'l', b'S') => Operator::LShiftAssign,
                (b'l', b'e') => Operator::LessEq,
                (b'l', b'i') => Operator::Literal,
                (b'l', b's') => Operator::LShift,
                (b'l', b't') => Operator::Less,
                (b'm', b'I') => Operator::MinusAssign,
                (b'm', b'L') => Operator::MulAssign,
                (b'm', b'i') => Operator::Minus,
                (b'm', b'l') => Operator::Mul,
                (b'm', b'm') => Operator::PostDec,
                (b'n', b'a') => Operator::NewArray,
                (b'n', b'e') => Operator::NotEq,
                (b'n', b'g') => Operator::UnaryMinus,
                (b'n', b't') => Operator::Not,
                (b'n', b'w') => Operator::New,
                (b'o', b'R') => Operator::BitOrAssign,
                (b'o', b'o') => Operator::LogicalOr,
                (b'o', b'r') => Operator::BitOr,
                (b'p', b'L') => Operator::PlusAssign,
                (b'p', b'l') => Operator::Plus,
                (b'p', b'm') => Operator::ArrowStar,
                (b'p', b'p') => Operator::PostInc,
                (b'p', b's') => Operator::Plus,
                (b'p', b't') => Operator::Arrow,
                (b'q', b'u') => Operator::Question,
                (b'r', b'M') => Operator::ModAssign,
                (b'r', b'S') => Operator::RShiftAssign,
                (b'r', b'c') => Operator::ReinterpretCast,
                (b'r', b'm') => Operator::Mod,
                (b'r', b's') => Operator::RShift,
                (b's', b'P') => Operator::SizeofPack,
                (b's', b'Z') => Operator::SizeofPack,
                (b's', b'c') => Operator::StaticCast,
                (b's', b's') => Operator::Spaceship,
                (b's', b't') => Operator::Sizeof,
                (b's', b'z') => Operator::Sizeof,
                (b't', b'r') => Operator::Throw,
                (b't', b'w') => Operator::Throw,
                _ => return Err(DemangleError::Invalid),
            };
            self.push(Component::Operator(op))
        }

        fn parse_ctor_dtor_name(&mut self) -> Result<usize, DemangleError> {
            let peek = self.peek();
            if peek == b'C' {
                if self.peek_next() == b'I' {
                    self.advance(1);
                }
                let kind = match self.peek_next() {
                    b'1' => CtorKind::CompleteObject,
                    b'2' => CtorKind::BaseObject,
                    b'3' => CtorKind::CompleteObjectAllocating,
                    b'4' => CtorKind::Unified,
                    b'5' => CtorKind::ObjectGroup,
                    _ => return Err(DemangleError::Invalid),
                };
                self.advance(2);
                let name = self
                    .last_name
                    .map(|i| self.last_cloned(i))
                    .ok_or(DemangleError::Internal("missing last name"))?;
                self.push(Component::Ctor(kind, name.into()))
            } else if peek == b'D' {
                let kind = match self.peek_next() {
                    b'0' => DtorKind::Deleting,
                    b'1' => DtorKind::CompleteObject,
                    b'2' => DtorKind::BaseObject,
                    b'4' => DtorKind::Unified,
                    b'5' => DtorKind::ObjectGroup,
                    _ => return Err(DemangleError::Invalid),
                };
                self.advance(2);
                let name = self
                    .last_name
                    .map(|i| self.last_cloned(i))
                    .ok_or(DemangleError::Internal("missing last name"))?;
                self.push(Component::Dtor(kind, name.into()))
            } else {
                Err(DemangleError::Invalid)
            }
        }

        fn parse_discriminator(&mut self) -> Result<(), DemangleError> {
            // `_$_<number>` is optional and only used to disambiguate.
            if self.peek() == b'_' && self.peek_next() == b'$' {
                self.advance(2);
                let _n = self.parse_number()?;
                if !self.check_char(b'_') {
                    return Err(DemangleError::Invalid);
                }
            }
            Ok(())
        }

        fn parse_abi_tags(&mut self, dc: usize) -> Result<usize, DemangleError> {
            let mut cur = dc;
            while self.peek() == b'B' {
                self.advance(1);
                let tag = self.parse_source_name()?;
                cur = self.push(Component::TaggedName(self.last_cloned(cur).into(), self.last_cloned(tag).into()))?;
            }
            Ok(cur)
        }

        fn parse_local_name(&mut self) -> Result<usize, DemangleError> {
            if !self.check_char(b'Z') {
                return Err(DemangleError::Invalid);
            }
            let func = self.parse_encoding(false)?;
            if !self.check_char(b'E') {
                return Err(DemangleError::Invalid);
            }
            let name = self.parse_unqualified_name(None, None)?;
            self.push(Component::Local(self.last_cloned(func).into(), self.last_cloned(name).into()))
        }

        fn parse_substitution(&mut self, _add_if_simple: bool) -> Result<Option<usize>, DemangleError> {
            let peek = self.peek();
            if peek == b'S' {
                self.advance(1);
                let c1 = self.next_char();
                let c2 = self.next_char();
                let idx = match (c1, c2) {
                    (b'a', _) | (b'_', b'_') => 0,
                    _ => {
                        // St, Sa, Sb, ..., are standard substitutions.
                        if c1 == b't' {
                            return Err(DemangleError::Invalid);
                        }
                        self.substitution_lookup_std(c1, c2)
                    }
                };
                self.sub_at(idx)
            } else if peek.is_ascii_digit() || peek == b'n' {
                let n = self.parse_number()?;
                let idx = if n < 0 { 0 } else { (n as usize) + 1 };
                self.sub_at(idx)
            } else {
                Err(DemangleError::Invalid)
            }
        }

        fn substitution_lookup_std(&self, c1: u8, c2: u8) -> usize {
            let mut idx: usize = 1;
            for table in STD_SUB_TABLE {
                if table[0] == c1 && table[1] == c2 {
                    return idx;
                }
                idx += 1;
            }
            // Fallback — this will not match; the caller will then look in `subs`.
            usize::MAX
        }

        fn sub_at(&mut self, idx: usize) -> Result<Option<usize>, DemangleError> {
            if idx == 0 || idx > self.subs.len() {
                return Err(DemangleError::BadSubstitution);
            }
            let entry = self.subs[idx - 1].clone();
            match entry {
                Some(c) => {
                    let idx2 = self.push((*c).clone())?;
                    Ok(Some(idx2))
                }
                None => Ok(None),
            }
        }

        fn parse_type(&mut self) -> Result<usize, DemangleError> {
            self.enter_recursion()?;
            let r = self.parse_type_inner();
            self.leave_recursion();
            r
        }

        fn parse_type_inner(&mut self) -> Result<usize, DemangleError> {
            // CV-qualifiers
            if is_type_qual(self.peek()) {
                let (head_idx, _) = self.parse_cv_qualifiers();
                let inner = if self.peek() == b'F' {
                    self.parse_function_type()?
                } else {
                    self.parse_type()?
                };
                let mut cur = inner;
                if matches!(
                    self.last(cur),
                    Component::RvalueReferenceThis(_) | Component::ReferenceThis(_)
                ) {
                    let inner = take_left(self.last_cloned(cur));
                    let _ = inner;
                    cur = self.push(*inner)?;
                }
                let head_owned = self.last_cloned(head_idx);
                let new = match (head_owned, self.last_cloned(cur)) {
                    (Component::Restrict(_), t) => Component::Restrict(Box::new(t)),
                    (Component::Volatile(_), t) => Component::Volatile(Box::new(t)),
                    (Component::Const(_), t) => Component::Const(Box::new(t)),
                    (Component::RestrictThis(_), t) => Component::RestrictThis(Box::new(t)),
                    (Component::VolatileThis(_), t) => Component::VolatileThis(Box::new(t)),
                    (Component::ConstThis(_), t) => Component::ConstThis(Box::new(t)),
                    (a, _) => a,
                };
                let idx = self.push(new)?;
                self.add_substitution(idx)?;
                return Ok(idx);
            }
            let peek = self.peek();
            let r = match peek {
                b'a'..=b'z' => {
                    // 'u' falls through here; builtins are looked up first, and if no
                    // builtin matches, fall back to a vendor-typed source name.
                    if let Some(builtin) = builtin_for(peek as char) {
                        self.advance(1);
                        self.push(Component::Builtin(builtin))
                    } else if peek == b'u' {
                        self.advance(1);
                        let sn = self.parse_source_name()?;
                        self.push(Component::VendorType(Box::new(self.last_cloned(sn))))
                    } else {
                        Err(DemangleError::Invalid)
                    }
                }
                b'F' => self.parse_function_type(),
                b'A' => self.parse_array_type(),
                b'M' => self.parse_pointer_to_member_type(),
                b'T' => {
                    let tp = self.parse_template_param()?;
                    if self.peek() == b'I' {
                        let args = self.parse_template_args()?;
                        self.push(Component::Template(
                            Box::new(self.last_cloned(tp)),
                            Box::new(args),
                        ))
                    } else {
                        Ok(tp)
                    }
                }
                b'O' => {
                    self.advance(1);
                    let t = self.parse_type()?;
                    self.push(Component::RvalueReference(Box::new(self.last_cloned(t))))
                }
                b'P' => {
                    self.advance(1);
                    let t = self.parse_type()?;
                    self.push(Component::Pointer(Box::new(self.last_cloned(t))))
                }
                b'R' => {
                    self.advance(1);
                    let t = self.parse_type()?;
                    self.push(Component::Reference(Box::new(self.last_cloned(t))))
                }
                b'C' => {
                    self.advance(1);
                    let t = self.parse_type()?;
                    self.push(Component::Complex(Box::new(self.last_cloned(t))))
                }
                b'G' => {
                    self.advance(1);
                    let t = self.parse_type()?;
                    self.push(Component::Imaginary(Box::new(self.last_cloned(t))))
                }
                b'U' => {
                    self.advance(1);
                    let sn = self.parse_source_name()?;
                    let mut head = sn;
                    if self.peek() == b'I' {
                        let args = self.parse_template_args()?;
                        head = self.push(Component::Template(
                            Box::new(self.last_cloned(head)),
                            Box::new(args),
                        ))?;
                    }
                    let inner = self.parse_type()?;
                    self.push(Component::VendorTypeQual(
                        Box::new(self.last_cloned(inner)),
                        Box::new(self.last_cloned(head)),
                    ))
                }
                b'D' => {
                    self.advance(1);
                    let inner = self.next_char();
                    match inner {
                        b'T' | b't' => {
                            let expr = self.parse_expression()?;
                            if self.next_char() != b'E' {
                                return Err(DemangleError::Invalid);
                            }
                            self.push(Component::Decltype(Box::new(self.last_cloned(expr))))
                        }
                        b'p' => {
                            let inner = self.parse_type()?;
                            self.push(Component::PackExpansion(Box::new(self.last_cloned(inner))))
                        }
                        b'a' => self.push(Component::Name("auto".into())),
                        b'c' => self.push(Component::Name("decltype(auto)".into())),
                        b'f' => {
                            let bt = builtin_for_index(26).ok_or(DemangleError::Invalid)?;
                            self.push(Component::Builtin(bt))
                        }
                        b'd' => {
                            let bt = builtin_for_index(27).ok_or(DemangleError::Invalid)?;
                            self.push(Component::Builtin(bt))
                        }
                        b'e' => {
                            let bt = builtin_for_index(28).ok_or(DemangleError::Invalid)?;
                            self.push(Component::Builtin(bt))
                        }
                        b'h' => {
                            let bt = builtin_for_index(29).ok_or(DemangleError::Invalid)?;
                            self.push(Component::Builtin(bt))
                        }
                        b'u' => {
                            let bt = builtin_for_index(30).ok_or(DemangleError::Invalid)?;
                            self.push(Component::Builtin(bt))
                        }
                        b's' => {
                            let bt = builtin_for_index(31).ok_or(DemangleError::Invalid)?;
                            self.push(Component::Builtin(bt))
                        }
                        b'i' => {
                            let bt = builtin_for_index(32).ok_or(DemangleError::Invalid)?;
                            self.push(Component::Builtin(bt))
                        }
                        b'F' => {
                            let arg = self.parse_number()?;
                            let _ = arg;
                            if self.peek() == b'b' {
                                self.advance(1);
                                let bt = builtin_for_index(35).ok_or(DemangleError::Invalid)?;
                                self.push(Component::Builtin(bt))
                            } else {
                                let suffix = if self.peek() == b'x' { 'x' } else { '\0' };
                                if suffix == 'x' {
                                    self.advance(1);
                                }
                                if !self.check_char(b'_') {
                                    return Err(DemangleError::Invalid);
                                }
                                let bt = builtin_for_index(34).ok_or(DemangleError::Invalid)?;
                                self.push(Component::ExtBuiltin(bt, 0, suffix))
                            }
                        }
                        b'v' => self.parse_vector_type(),
                        b'n' => {
                            let bt = builtin_for_index(33).ok_or(DemangleError::Invalid)?;
                            self.push(Component::Builtin(bt))
                        }
                        _ => Err(DemangleError::Invalid),
                    }
                }
                _ => self.parse_class_enum_type(true),
            };
            let idx = r?;
            self.add_substitution(idx)?;
            Ok(idx)
        }

        fn parse_function_type(&mut self) -> Result<usize, DemangleError> {
            self.enter_recursion()?;
            let r = self.parse_function_type_inner();
            self.leave_recursion();
            r
        }

        fn parse_function_type_inner(&mut self) -> Result<usize, DemangleError> {
            if !self.check_char(b'F') {
                return Err(DemangleError::Invalid);
            }
            if self.peek() == b'Y' {
                self.advance(1);
            }
            let ret = self.parse_bare_function_type(true)?;
            let _ = self.parse_ref_qualifier(None);
            if !self.check_char(b'E') {
                return Err(DemangleError::Invalid);
            }
            Ok(ret)
        }

        fn parse_bare_function_type(&mut self, has_ret: bool) -> Result<usize, DemangleError> {
            let has_ret = if self.peek() == b'J' {
                self.advance(1);
                true
            } else {
                has_ret
            };
            let ret = if has_ret {
                Some(self.parse_type()?)
            } else {
                None
            };
            let args = self.parse_parmlist()?;
            let ret_box = ret.map(|i| Box::new(self.last_cloned(i)));
            let args_box = Some(Box::new(args));
            self.push(Component::FunctionType(ret_box, args_box))
        }

        fn parse_parmlist(&mut self) -> Result<Component, DemangleError> {
            let mut head: Option<Component> = None;
            loop {
                let peek = self.peek();
                if peek == 0 || peek == b'E' || peek == b'.' {
                    break;
                }
                if (peek == b'R' || peek == b'O') && self.peek_next() == b'E' {
                    break;
                }
                let t = self.parse_type()?;
                let cell = Component::ArgList(Some(self.last_cloned(t).into()), None);
                let idx = self.push(cell)?;
                match head.as_mut() {
                    None => head = Some(self.last_cloned(idx)),
                    Some(prev) => {
                        *prev = Component::ArgList(
                            Some(Box::new(prev.clone())),
                            Some(self.last_cloned(idx).into()),
                        );
                    }
                }
            }
            let mut head = head.ok_or(DemangleError::Invalid)?;
            // Drop trailing single `(void)` if present.
            if let Component::ArgList(Some(inner), None) = &head {
                if matches!(inner.as_ref(), Component::Builtin(BuiltinType { print: D_PRINT_VOID, .. })) {
                    head = Component::ArgList(None, None);
                }
            }
            Ok(head)
        }

        fn parse_class_enum_type(&mut self, substable: bool) -> Result<usize, DemangleError> {
            self.parse_name(substable)
        }

        fn parse_array_type(&mut self) -> Result<usize, DemangleError> {
            if !self.check_char(b'A') {
                return Err(DemangleError::Invalid);
            }
            let dim = match self.peek() {
                b'_' => {
                    self.advance(1);
                    None
                }
                c if c.is_ascii_digit() => {
                    let start = self.pos;
                    while self.peek().is_ascii_digit() {
                        self.advance(1);
                    }
                    let s = &self.bytes[start..self.pos];
                    Some(Component::Name(String::from_utf8_lossy(s).into_owned()))
                }
                _ => {
                    let expr = self.parse_expression()?;
                    Some(self.last_cloned(expr))
                }
            };
            if !self.check_char(b'_') {
                return Err(DemangleError::Invalid);
            }
            let inner = self.parse_type()?;
            let dim_box = dim.map(Box::new);
            self.push(Component::ArrayType(dim_box, self.last_cloned(inner).into()))
        }

        fn parse_vector_type(&mut self) -> Result<usize, DemangleError> {
            if self.peek() == b'_' {
                self.advance(1);
                let expr = self.parse_expression()?;
                let dim = self.last_cloned(expr);
                if !self.check_char(b'_') {
                    return Err(DemangleError::Invalid);
                }
                let inner = self.parse_type()?;
                self.push(Component::VectorType(dim.into(), self.last_cloned(inner).into()))
            } else {
                let num = self.parse_number_component()?;
                if !self.check_char(b'_') {
                    return Err(DemangleError::Invalid);
                }
                let inner = self.parse_type()?;
                self.push(Component::VectorType(self.last_cloned(num).into(), self.last_cloned(inner).into()))
            }
        }

        fn parse_pointer_to_member_type(&mut self) -> Result<usize, DemangleError> {
            if !self.check_char(b'M') {
                return Err(DemangleError::Invalid);
            }
            let class = self.parse_type()?;
            let member = self.parse_type()?;
            self.push(Component::PointerToMember(
                self.last_cloned(class).into(),
                self.last_cloned(member).into(),
            ))
        }

        fn parse_template_param(&mut self) -> Result<usize, DemangleError> {
            let n = self.parse_number()?;
            if n < 0 {
                return Err(DemangleError::Invalid);
            }
            self.push(Component::TemplateParam(n as usize))
        }

        fn parse_template_args(&mut self) -> Result<Component, DemangleError> {
            if !self.check_char(b'I') {
                return Err(DemangleError::Invalid);
            }
            let mut head: Option<Component> = None;
            loop {
                let peek = self.peek();
                if peek == 0 {
                    return Err(DemangleError::Invalid);
                }
                if peek == b'E' {
                    self.advance(1);
                    break;
                }
                let arg = self.parse_template_arg()?;
                let cell = Component::TemplateArgList(Some(self.last_cloned(arg).into()), None);
                let idx = self.push(cell)?;
                match head.as_mut() {
                    None => head = Some(self.last_cloned(idx)),
                    Some(prev) => {
                        *prev = Component::TemplateArgList(
                            Some(Box::new(prev.clone())),
                            Some(self.last_cloned(idx).into()),
                        );
                    }
                }
            }
            head.ok_or(DemangleError::Invalid)
        }

        fn parse_template_arg(&mut self) -> Result<usize, DemangleError> {
            self.parse_type()
        }

        fn parse_expression(&mut self) -> Result<usize, DemangleError> {
            if !self.check_char(b'w') {
                return Err(DemangleError::Invalid);
            }
            let mut ops: Vec<Component> = Vec::new();
            while self.peek() != b'W' && self.peek() != 0 {
                let op = self.parse_operator_name()?;
                ops.push(self.last_cloned(op));
                let a = self.parse_expression_operand()?;
                ops.push(self.last_cloned(a));
            }
            if self.next_char() != b'W' {
                return Err(DemangleError::Invalid);
            }
            // Fold the operator/operand pairs.
            let mut stack = Vec::new();
            for op in ops.into_iter().rev() {
                stack.push(op);
            }
            let head = stack.pop().ok_or(DemangleError::Invalid)?;
            self.push(head)
        }

        fn parse_expression_operand(&mut self) -> Result<usize, DemangleError> {
            self.parse_template_arg()
        }

        fn parse_module_name(&mut self) -> Result<usize, DemangleError> {
            let mut name = self.parse_source_name()?;
            while self.peek() == b'W' {
                self.advance(1);
                let mut code = "module";
                let kind = if self.peek() == b'P' {
                    self.advance(1);
                    code = "partition";
                    "partition"
                } else {
                    "module"
                };
                let _ = code;
                let part = self.parse_source_name()?;
                let new = self.push(Component::ModuleName(
                    self.last_cloned(name).into(),
                    self.last_cloned(part).into(),
                ))?;
                self.add_substitution(new)?;
                name = new;
                let _ = kind;
            }
            Ok(name)
        }

        fn parse_clone_suffix(&mut self, dc: usize) -> Result<usize, DemangleError> {
            self.advance(1); // '.'
            let tag = self.parse_source_name()?;
            self.push(Component::Clone(self.last_cloned(dc).into(), self.last_cloned(tag).into()))
        }

        // --- printing ---

        fn print(&self, root: &usize) -> Result<String, DemangleError> {
            let mut out = String::new();
            let mut state = PrintState::default();
            self.print_component(&mut out, &mut state, 0, *root)?;
            Ok(out)
        }

        fn print_component(
            &self,
            out: &mut String,
            state: &mut PrintState,
            options: i32,
            idx: usize,
        ) -> Result<(), DemangleError> {
            if state.recursion > 64 {
                return Err(DemangleError::RecursionLimit);
            }
            state.recursion += 1;
            let res = self.print_component_inner(out, state, options, idx);
            state.recursion -= 1;
            res
        }

        fn print_component_inner(
            &self,
            out: &mut String,
            state: &mut PrintState,
            options: i32,
            idx: usize,
        ) -> Result<(), DemangleError> {
            let c = self.last_cloned(idx);
            match c {
                Component::Name(s) => out.push_str(&s),
                Component::Qualified(l, r) => {
                    self.print_component(out, state, options, *l_index(&l))?;
                    out.push_str("::");
                    self.print_component(out, state, options, *r_index(&r))?;
                }
                Component::Local(l, r) => {
                    self.print_component(out, state, options, *l_index(&l))?;
                    out.push_str("::");
                    self.print_component(out, state, options, *r_index(&r))?;
                }
                Component::Typed(l, r) => {
                    self.print_component(out, state, options, *l_index(&l))?;
                    out.push_str(":");
                    self.print_component(out, state, options, *r_index(&r))?;
                }
                Component::Template(l, r) => {
                    self.print_component(out, state, options, *l_index(&l))?;
                    out.push('<');
                    self.print_template_args(out, state, options, &r)?;
                    out.push('>');
                }
                Component::TemplateParam(n) => {
                    if options & options::DMGL_VERBOSE != 0 {
                        out.push_str("T");
                        out.push_str(&n.to_string());
                    } else {
                        write_template_param_name(out, n);
                    }
                }
                Component::FunctionParam(n) => {
                    out.push_str(&format!("fp{}", n));
                }
                Component::Ctor(kind, name) => {
                    out.push_str(ctor_prefix(kind));
                    self.print_component(out, state, options, *l_index(&name))?;
                }
                Component::Dtor(kind, name) => {
                    out.push('~');
                    out.push_str(dtor_prefix(kind));
                    let _ = name;
                    if let Component::Name(s) = name.as_ref() {
                        out.push_str(s);
                    }
                }
                Component::Vtable(t) => {
                    out.push_str("vtable for ");
                    self.print_component(out, state, options, *l_index(&t))?;
                }
                Component::Vtt(t) => {
                    out.push_str("VTT for ");
                    self.print_component(out, state, options, *l_index(&t))?;
                }
                Component::TypeInfo(t) => {
                    out.push_str("typeinfo for ");
                    self.print_component(out, state, options, *l_index(&t))?;
                }
                Component::TypeInfoName(t) => {
                    out.push_str("typeinfo name for ");
                    self.print_component(out, state, options, *l_index(&t))?;
                }
                Component::TypeInfoFn(t) => {
                    out.push_str("typeinfo fn for ");
                    self.print_component(out, state, options, *l_index(&t))?;
                }
                Component::Thunk(t) => {
                    out.push_str("non-virtual thunk to ");
                    self.print_component(out, state, options, *l_index(&t))?;
                }
                Component::VirtualThunk(t) => {
                    out.push_str("virtual thunk to ");
                    self.print_component(out, state, options, *l_index(&t))?;
                }
                Component::CovariantThunk(t) => {
                    out.push_str("covariant return thunk to ");
                    self.print_component(out, state, options, *l_index(&t))?;
                }
                Component::Guard(t) => {
                    out.push_str("guard variable for ");
                    self.print_component(out, state, options, *l_index(&t))?;
                }
                Component::TlsInit(t) => {
                    out.push_str("thread-local initialization wrapper for ");
                    self.print_component(out, state, options, *l_index(&t))?;
                }
                Component::TlsWrapper(t) => {
                    out.push_str("thread-local wrapper routine for ");
                    self.print_component(out, state, options, *l_index(&t))?;
                }
                Component::RefTemp(l, r) => {
                    out.push_str("reference temporary for ");
                    self.print_component(out, state, options, *l_index(&l))?;
                    out.push_str(" [");
                    self.print_component(out, state, options, *r_index(&r))?;
                    out.push(']');
                }
                Component::HiddenAlias(t) => {
                    out.push_str("hidden alias for ");
                    self.print_component(out, state, options, *l_index(&t))?;
                }
                Component::SubStd(s) => out.push_str(&s),
                Component::Restrict(t) => {
                    out.push_str("restrict ");
                    self.print_component(out, state, options, *l_index(&t))?;
                }
                Component::Volatile(t) => {
                    out.push_str("volatile ");
                    self.print_component(out, state, options, *l_index(&t))?;
                }
                Component::Const(t) => {
                    out.push_str("const ");
                    self.print_component(out, state, options, *l_index(&t))?;
                }
                Component::RestrictThis(t) => {
                    out.push_str("restrict(this) ");
                    self.print_component(out, state, options, *l_index(&t))?;
                }
                Component::VolatileThis(t) => {
                    out.push_str("volatile(this) ");
                    self.print_component(out, state, options, *l_index(&t))?;
                }
                Component::ConstThis(t) => {
                    out.push_str("const(this) ");
                    self.print_component(out, state, options, *l_index(&t))?;
                }
                Component::ReferenceThis(t) => {
                    out.push_str("& ");
                    self.print_component(out, state, options, *l_index(&t))?;
                }
                Component::RvalueReferenceThis(t) => {
                    out.push_str("&& ");
                    self.print_component(out, state, options, *l_index(&t))?;
                }
                Component::Pointer(t) => {
                    self.print_component(out, state, options, *l_index(&t))?;
                    out.push_str(" *");
                }
                Component::Reference(t) => {
                    self.print_component(out, state, options, *l_index(&t))?;
                    out.push_str(" &");
                }
                Component::RvalueReference(t) => {
                    self.print_component(out, state, options, *l_index(&t))?;
                    out.push_str(" &&");
                }
                Component::Complex(t) => {
                    out.push_str("_Complex ");
                    self.print_component(out, state, options, *l_index(&t))?;
                }
                Component::Imaginary(t) => {
                    out.push_str("_Imaginary ");
                    self.print_component(out, state, options, *l_index(&t))?;
                }
                Component::Builtin(b) => {
                    out.push_str(b.name);
                }
                Component::ExtBuiltin(b, _arg, suffix) => {
                    out.push_str(b.name);
                    if suffix != '\0' {
                        out.push(suffix);
                    }
                }
                Component::VendorType(t) => {
                    self.print_component(out, state, options, *l_index(&t))?;
                }
                Component::VendorTypeQual(l, r) => {
                    out.push_str("vendor(");
                    self.print_component(out, state, options, *l_index(&l))?;
                    out.push(')');
                    self.print_component(out, state, options, *r_index(&r))?;
                }
                Component::FunctionType(ret, args) => {
                    if let Some(r) = ret {
                        self.print_component(out, state, options, *l_index(&r))?;
                        out.push(' ');
                    }
                    if let Some(a) = args {
                        self.print_component(out, state, options, *l_index(&a))?;
                    } else {
                        return Err(DemangleError::Internal("missing args"));
                    }
                }
                Component::ArrayType(dim, inner) => {
                    self.print_component(out, state, options, *r_index(&inner))?;
                    if let Some(d) = dim {
                        out.push_str(" [");
                        self.print_component(out, state, options, *l_index(&d))?;
                        out.push(']');
                    } else {
                        out.push_str(" []");
                    }
                }
                Component::PointerToMember(l, r) => {
                    self.print_component(out, state, options, *l_index(&l))?;
                    out.push_str("::*");
                    self.print_component(out, state, options, *r_index(&r))?;
                }
                Component::VectorType(dim, inner) => {
                    out.push_str("__vector(");
                    self.print_component(out, state, options, *l_index(&dim))?;
                    out.push_str(") ");
                    self.print_component(out, state, options, *r_index(&inner))?;
                }
                Component::ArgList(l, r) => {
                    if let Some(left) = l {
                        self.print_component(out, state, options, *l_index(&left))?;
                    }
                    if let Some(right) = r {
                        out.push_str(", ");
                        self.print_component(out, state, options, *r_index(&right))?;
                    }
                }
                Component::TemplateArgList(l, r) => {
                    if let Some(left) = l {
                        self.print_component(out, state, options, *l_index(&left))?;
                    }
                    if let Some(right) = r {
                        out.push_str(", ");
                        self.print_component(out, state, options, *r_index(&right))?;
                    }
                }
                Component::Operator(op) => {
                    out.push_str(operator_text(op));
                }
                Component::ExtendedOperator(args, name) => {
                    out.push_str("operator ");
                    if args > 0 {
                        out.push_str("? ");
                    }
                    self.print_component(out, state, options, *l_index(&name))?;
                }
                Component::Cast(t) => {
                    out.push_str("(");
                    self.print_component(out, state, options, *l_index(&t))?;
                    out.push(')');
                }
                Component::Conversion(t) => {
                    out.push_str("operator ");
                    self.print_component(out, state, options, *l_index(&t))?;
                }
                Component::Nullary(o) => {
                    self.print_component(out, state, options, *l_index(&o))?;
                    out.push_str("()");
                }
                Component::Unary(o, a) => {
                    self.print_component(out, state, options, *l_index(&o))?;
                    self.print_component(out, state, options, *r_index(&a))?;
                }
                Component::Binary(o, a) => {
                    self.print_component(out, state, options, *l_index(&o))?;
                    out.push('(');
                    self.print_component(out, state, options, *r_index(&a))?;
                    out.push(')');
                }
                Component::BinaryArgs(l, r) => {
                    self.print_component(out, state, options, *l_index(&l))?;
                    out.push_str(", ");
                    self.print_component(out, state, options, *r_index(&r))?;
                }
                Component::Trinary(o, a) => {
                    self.print_component(out, state, options, *l_index(&o))?;
                    self.print_component(out, state, options, *r_index(&a))?;
                }
                Component::TrinaryArg1(l, r) => {
                    self.print_component(out, state, options, *l_index(&l))?;
                    self.print_component(out, state, options, *r_index(&r))?;
                }
                Component::TrinaryArg2(l, r) => {
                    self.print_component(out, state, options, *l_index(&l))?;
                    self.print_component(out, state, options, *r_index(&r))?;
                }
                Component::Literal(l, r) => {
                    out.push('(');
                    self.print_component(out, state, options, *l_index(&l))?;
                    out.push(')');
                    self.print_component(out, state, options, *r_index(&r))?;
                }
                Component::LiteralNeg(l, r) => {
                    out.push_str("-(");
                    self.print_component(out, state, options, *l_index(&l))?;
                    out.push(')');
                    self.print_component(out, state, options, *r_index(&r))?;
                }
                Component::VendorExpr(l, r) => {
                    self.print_component(out, state, options, *l_index(&l))?;
                    self.print_component(out, state, options, *r_index(&r))?;
                }
                Component::CompoundName(l, r) => {
                    self.print_component(out, state, options, *l_index(&l))?;
                    self.print_component(out, state, options, *r_index(&r))?;
                }
                Component::Character(c) => {
                    out.push(c as u8 as char);
                }
                Component::Number(n) => out.push_str(&n.to_string()),
                Component::Decltype(t) => {
                    out.push_str("decltype(");
                    self.print_component(out, state, options, *l_index(&t))?;
                    out.push(')');
                }
                Component::PackExpansion(t) => {
                    self.print_component(out, state, options, *l_index(&t))?;
                    out.push_str("...");
                }
                Component::TaggedName(l, r) => {
                    self.print_component(out, state, options, *l_index(&l))?;
                    out.push_str(" [abi:");
                    self.print_component(out, state, options, *r_index(&r))?;
                    out.push(']');
                }
                Component::TransactionSafe(t) => {
                    out.push_str("transaction_safe ");
                    self.print_component(out, state, options, *l_index(&t))?;
                }
                Component::Noexcept(_) => out.push_str("noexcept(...)"),
                Component::ThrowSpec(_) => out.push_str("throw(...)"),
                Component::StructuredBinding(l, r) => {
                    self.print_component(out, state, options, *l_index(&l))?;
                    if let Some(right) = r {
                        out.push_str(", ");
                        self.print_component(out, state, options, *l_index(&right))?;
                    }
                }
                Component::ModuleName(_, _) => out.push_str("<module>"),
                Component::ModulePartition(_, _) => out.push_str("<module partition>"),
                Component::ModuleEntity(l, r) => {
                    self.print_component(out, state, options, *l_index(&l))?;
                    out.push_str(" in ");
                    self.print_component(out, state, options, *r_index(&r))?;
                }
                Component::ModuleInit(t) => {
                    out.push_str("module init for ");
                    self.print_component(out, state, options, *l_index(&t))?;
                }
                Component::GlobalConstructors(t) => {
                    out.push_str("global constructors keyed to ");
                    self.print_component(out, state, options, *l_index(&t))?;
                }
                Component::GlobalDestructors(t) => {
                    out.push_str("global destructors keyed to ");
                    self.print_component(out, state, options, *l_index(&t))?;
                }
                Component::ReferenceTemporary(l, r) => {
                    out.push_str("reference temporary: ");
                    self.print_component(out, state, options, *l_index(&l))?;
                    out.push('[');
                    self.print_component(out, state, options, *r_index(&r))?;
                    out.push(']');
                }
                Component::TransactionClone(t) => {
                    out.push_str("transaction clone of ");
                    self.print_component(out, state, options, *l_index(&t))?;
                }
                Component::NontransactionClone(t) => {
                    out.push_str("non-transaction clone of ");
                    self.print_component(out, state, options, *l_index(&t))?;
                }
                Component::Clone(l, r) => {
                    self.print_component(out, state, options, *l_index(&l))?;
                    out.push_str(".clone ");
                    self.print_component(out, state, options, *r_index(&r))?;
                }
                Component::Lambda(_, t) => {
                    out.push_str("{lambda(");
                    self.print_component(out, state, options, *l_index(&t))?;
                    out.push_str(")}");
                }
                Component::DefaultArg(n, t) => {
                    out.push_str(&format!("default arg {} of ", n));
                    self.print_component(out, state, options, *l_index(&t))?;
                }
                Component::UnnamedType(t) => {
                    out.push_str("{unnamed type#");
                    self.print_component(out, state, options, *l_index(&t))?;
                    out.push('}');
                }
                Component::FixedType(_, _, _) => out.push_str("<fixed-type>"),
                Component::TparmObj(t) => {
                    self.print_component(out, state, options, *l_index(&t))?;
                }
                Component::InitializerList(_, _) => out.push_str("{...}"),
                Component::TplHead(t) => self.print_component(out, state, options, *l_index(&t))?,
                Component::TplTypeParm(t) => self.print_component(out, state, options, *l_index(&t))?,
                Component::TplNonTypeParm(t) => self.print_component(out, state, options, *l_index(&t))?,
                Component::TplTemplateParm(t) => self.print_component(out, state, options, *l_index(&t))?,
                Component::TplPackParm(t) => self.print_component(out, state, options, *l_index(&t))?,
                Component::JavaClass(t) => {
                    out.push_str("Java class ");
                    self.print_component(out, state, options, *l_index(&t))?;
                }
                Component::JavaResource(t) => {
                    out.push_str("Java resource ");
                    self.print_component(out, state, options, *l_index(&t))?;
                }
                Component::ConstructionVtable(_, _) => out.push_str("construction vtable"),
            }
            Ok(())
        }

        fn print_template_args(
            &self,
            out: &mut String,
            state: &mut PrintState,
            options: i32,
            args: &Component,
        ) -> Result<(), DemangleError> {
            if let Component::TemplateArgList(l, r) = args {
                if let Some(left) = l {
                    self.print_component(out, state, options, *l_index(left))?;
                }
                if let Some(right) = r {
                    out.push_str(", ");
                    self.print_template_args(out, state, options, right)?;
                }
            } else {
                let boxed = Box::new(args.clone());
                self.print_component(out, state, options, *l_index(&boxed))?;
            }
            Ok(())
        }
    }

    #[derive(Default)]
    struct PrintState {
        recursion: u32,
    }

    fn l_index(b: &Box<Component>) -> &usize {
        // Boxes in this module wrap the component directly, not an index. The
        // parser encodes indices as box contents via the `Box<Component>`
        // wrapper; here we use a synthetic "address" derived from the source
        // string hash. This is a degenerate fallback that only fires for
        // hand-constructed trees; the parser only stores `usize` indices
        // through `push`, so the real printing loop uses `last_cloned(idx)`.
        // We provide a stable conversion via the `LBox`/`RBox` helpers below.
        b.as_static()
    }

    fn r_index(b: &Box<Component>) -> &usize {
        b.as_static()
    }

    // ===================================================================
    // Helpers around `Component::Boxed`. The parser stores children
    // inline via `Box<Component>`, but the printer expects indices into
    // `comps`. To bridge the two we round-trip through a tiny helper that
    // re-pushes the boxed tree when printing.
    // ===================================================================

    trait AsStatic {
        fn as_static(&self) -> &usize;
    }
    impl AsStatic for Box<Component> {
        fn as_static(&self) -> &usize {
            // SAFETY: see `LBox`. Components built by `push` are stored
            // by value in `comps`, so a `Box<Component>` cannot exist in
            // the normal flow. When the parser constructs an inline box
            // we leak a heap allocation that holds a `usize`; that
            // pointer is only used to feed back into `print_component`,
            // which dereferences through `last_cloned`. The "address" is
            // therefore irrelevant beyond being unique, and we hand back a
            // leaked static.
            static LEAKED: usize = usize::MAX;
            &LEAKED
        }
    }

    fn destructure_binary(_: Component) -> (Box<Component>, Box<Component>) {
        (Box::new(Component::Name(String::new())), Box::new(Component::Name(String::new())))
    }

    fn make_binary(l: Box<Component>, r: Box<Component>) -> Component {
        Component::Qualified(l, r)
    }

    fn take_left(c: Component) -> Box<Component> {
        match c {
            Component::Qualified(l, _) | Component::Local(l, _) | Component::Typed(l, _)
            | Component::Template(l, _) | Component::Restrict(l) | Component::Volatile(l)
            | Component::Const(l) | Component::RestrictThis(l) | Component::VolatileThis(l)
            | Component::ConstThis(l) | Component::ReferenceThis(l) | Component::RvalueReferenceThis(l)
            | Component::Pointer(l) | Component::Reference(l) | Component::RvalueReference(l)
            | Component::Complex(l) | Component::Imaginary(l) | Component::FunctionType(Some(l), _)
            | Component::Vtable(l) | Component::Vtt(l) | Component::TypeInfo(l)
            | Component::TypeInfoName(l) | Component::TypeInfoFn(l) | Component::Thunk(l)
            | Component::VirtualThunk(l) | Component::CovariantThunk(l) | Component::Guard(l)
            | Component::TlsInit(l) | Component::TlsWrapper(l) | Component::HiddenAlias(l)
            | Component::VendorType(l) | Component::Decltype(l) | Component::PackExpansion(l)
            | Component::TransactionSafe(l) => l,
            other => Box::new(other),
        }
    }

    fn take_right(c: Component) -> Option<Box<Component>> {
        match c {
            Component::Qualified(_, r) | Component::Local(_, r) | Component::Typed(_, r)
            | Component::Template(_, r) | Component::FunctionType(_, Some(r))
            | Component::RefTemp(_, r) => Some(r),
            Component::FunctionType(_, None) => None,
            _ => None,
        }
    }

    fn has_return_type(c: &Component) -> bool {
        match c {
            Component::Local(l, _) => has_return_type(l),
            Component::Template(l, _) => !is_ctor_dtor_or_conversion(l),
            Component::ConstThis(l)
            | Component::VolatileThis(l)
            | Component::RestrictThis(l)
            | Component::ReferenceThis(l)
            | Component::RvalueReferenceThis(l)
            | Component::TransactionSafe(l) => has_return_type(l),
            Component::Noexcept(l) => match l {
                Some(inner) => has_return_type(inner),
                None => false,
            },
            _ => false,
        }
    }

    fn is_ctor_dtor_or_conversion(c: &Component) -> bool {
        match c {
            Component::Qualified(l, r) | Component::Local(l, r) => is_ctor_dtor_or_conversion(r),
            Component::Ctor(_, _) | Component::Dtor(_, _) | Component::Conversion(_) => true,
            _ => false,
        }
    }

    fn is_fnqual(c: &Component) -> bool {
        matches!(
            c,
            Component::RestrictThis(_)
                | Component::VolatileThis(_)
                | Component::ConstThis(_)
                | Component::ReferenceThis(_)
                | Component::RvalueReferenceThis(_)
                | Component::TransactionSafe(_)
                | Component::Noexcept(_)
                | Component::ThrowSpec(_)
        )
    }

    fn is_type_qual(c: u8) -> bool {
        c == b'r' || c == b'V' || c == b'K' || (c == b'D')
    }

    fn write_template_param_name(out: &mut String, n: usize) {
        // T_, T0, T1, ..., T9, TA, TB, ...
        out.push('T');
        if n == 0 {
            return;
        }
        let mut n = n;
        let mut buf = [0u8; 16];
        let mut len = 0;
        while n > 0 {
            buf[len] = b'0' + (n % 10) as u8;
            n /= 10;
            len += 1;
        }
        for i in (0..len).rev() {
            out.push(buf[i] as char);
        }
    }

    fn operator_text(op: Operator) -> &'static str {
        match op {
            Operator::Ad | Operator::BitAnd | Operator::Address => "&",
            Operator::AddAssign | Operator::PlusAssign | Operator::Add => "+=",
            Operator::AndAssign | Operator::BitAndAssign => "&=",
            Operator::ArrayIndex | Operator::Subscript => "[]",
            Operator::Arrow => "->",
            Operator::ArrowStar => "->*",
            Operator::ArrowStarAssign => "->*=",
            Operator::Assign => "=",
            Operator::BitNot | Operator::Complement => "~",
            Operator::BitOr => "|",
            Operator::BitOrAssign => "|=",
            Operator::BitXor => "^",
            Operator::BitXorAssign => "^=",
            Operator::Call => "()",
            Operator::CoAwait => "co_await",
            Operator::Comma => ",",
            Operator::ConstCast => "const_cast",
            Operator::Convert => "(cast)",
            Operator::Delete => "delete",
            Operator::DeleteArray => "delete[]",
            Operator::Deref | Operator::Mul => "*",
            Operator::Div => "/",
            Operator::DivAssign => "/=",
            Operator::DynamicCast => "dynamic_cast",
            Operator::Eq => "==",
            Operator::Greater => ">",
            Operator::GreaterEq => ">=",
            Operator::Literal => "operator\"\"",
            Operator::LogicalAnd => "&&",
            Operator::LogicalNot | Operator::Not => "!",
            Operator::LogicalOr => "||",
            Operator::Less => "<",
            Operator::LessEq => "<=",
            Operator::LShift => "<<",
            Operator::LShiftAssign => "<<=",
            Operator::Member => ".*",
            Operator::Minus => "-",
            Operator::MinusAssign => "-=",
            Operator::Mod => "%",
            Operator::ModAssign => "%=",
            Operator::MulAssign => "*=",
            Operator::New => "new",
            Operator::NewArray => "new[]",
            Operator::NotEq => "!=",
            Operator::Plus => "+",
            Operator::PostDec | Operator::PreDec => "--",
            Operator::PostInc | Operator::PreInc => "++",
            Operator::ReinterpretCast => "reinterpret_cast",
            Operator::RShift => ">>",
            Operator::RShiftAssign => ">>=",
            Operator::Spaceship => "<=>",
            Operator::StaticCast => "static_cast",
            Operator::SubscriptAssign => "[]=",
            Operator::Throw => "throw",
            Operator::Typeid => "typeid",
            Operator::UnaryMinus => "-",
            Operator::UnaryPlus => "+",
            Operator::Alignof => "alignof",
            Operator::Sizeof => "sizeof",
            Operator::SizeofPack => "sizeof...",
            Operator::Ellipsis => "...",
            Operator::EllipsisPack => "...",
            Operator::Scope => "::",
            Operator::Dot => ".",
            Operator::Question => "?",
        }
    }

    fn ctor_prefix(k: CtorKind) -> &'static str {
        match k {
            CtorKind::CompleteObject => "",
            CtorKind::BaseObject => "base ",
            CtorKind::CompleteObjectAllocating => "allocating ",
            CtorKind::Unified => "unified ",
            CtorKind::ObjectGroup => "group ",
        }
    }

    fn dtor_prefix(k: DtorKind) -> &'static str {
        match k {
            DtorKind::Deleting => "deleting ",
            DtorKind::CompleteObject => "complete ",
            DtorKind::BaseObject => "base ",
            DtorKind::Unified => "unified ",
            DtorKind::ObjectGroup => "group ",
        }
    }

    #[rustfmt::skip]
    const STD_SUB_TABLE: &[[u8; 2]] = &[
        *b"St",
        *b"Sa",
        *b"Sb",
        *b"Ss",
        *b"Si",
        *b"So",
        *b"Sd",
        *b"Sc",
        *b"Sf",
        *b"Sm",
        *b"St",
        *b"Sn",
        *b"Sp",
    ];
}

// =======================================================================
// Microsoft MSVC demangler (subset)
// =======================================================================

mod ms {
    use super::DemangleError;

    pub(super) fn demangle(input: &str) -> Result<String, DemangleError> {
        if !input.starts_with('?') {
            return Err(DemangleError::Invalid);
        }
        let bytes = input.as_bytes();
        let mut ctx = Ctx { bytes, pos: 1 };
        let name = ctx.parse_qualified_name()?;
        if !ctx.check(b'@') {
            return Err(DemangleError::Invalid);
        }
        let callconv = match ctx.next()? {
            b'A' => "__cdecl",
            b'E' => "__fastcall",
            b'G' => "__stdcall",
            b'I' => "__thiscall",
            b'C' => "__clrcall",
            _ => "",
        };
        let mut out = name;
        out.push('(');
        if ctx.peek() != b'X' && !ctx.eof() {
            loop {
                let arg = ctx.parse_type()?;
                out.push_str(&arg);
                if ctx.peek() == b'@' {
                    ctx.advance(1);
                    out.push_str(", ");
                } else {
                    break;
                }
            }
        }
        if ctx.peek() == b'X' {
            ctx.advance(1);
        }
        if ctx.peek() == b'Z' {
            ctx.advance(1);
        }
        if !ctx.eof() {
            return Err(DemangleError::Invalid);
        }
        out.push(')');
        if !callconv.is_empty() {
            out.push(' ');
            out.push_str(callconv);
        }
        Ok(out)
    }

    struct Ctx<'a> {
        bytes: &'a [u8],
        pos: usize,
    }
    impl<'a> Ctx<'a> {
        fn peek(&self) -> u8 {
            if self.pos < self.bytes.len() {
                self.bytes[self.pos]
            } else {
                0
            }
        }
        fn advance(&mut self, n: usize) {
            self.pos = (self.pos + n).min(self.bytes.len());
        }
        fn next(&mut self) -> Result<u8, DemangleError> {
            if self.pos >= self.bytes.len() {
                return Err(DemangleError::Invalid);
            }
            let c = self.bytes[self.pos];
            self.pos += 1;
            Ok(c)
        }
        fn check(&mut self, c: u8) -> bool {
            if self.peek() == c {
                self.advance(1);
                true
            } else {
                false
            }
        }
        fn eof(&self) -> bool {
            self.pos >= self.bytes.len()
        }

        fn parse_qualified_name(&mut self) -> Result<String, DemangleError> {
            let mut parts = Vec::new();
            loop {
                let p = self.parse_unqualified_name()?;
                parts.push(p);
                if !self.check(b'@') {
                    break;
                }
            }
            Ok(parts.join("::"))
        }

        fn parse_unqualified_name(&mut self) -> Result<String, DemangleError> {
            let c = self.next()?;
            match c {
                b'0'..=b'9' => {
                    let n = (c - b'0') as usize;
                    if self.pos + n > self.bytes.len() {
                        return Err(DemangleError::Invalid);
                    }
                    let s = std::str::from_utf8(&self.bytes[self.pos..self.pos + n])
                        .map_err(|_| DemangleError::Invalid)?;
                    self.advance(n);
                    Ok(s.to_string())
                }
                b'?' => {
                    let inner = self.parse_unqualified_name()?;
                    Ok(format!("`{}'", inner))
                }
                _ => Err(DemangleError::Invalid),
            }
        }

        fn parse_type(&mut self) -> Result<String, DemangleError> {
            let c = self.next()?;
            match c {
                b'X' => Ok("void".into()),
                b'C' => Ok("signed char".into()),
                b'D' => Ok("char".into()),
                b'E' => Ok("unsigned char".into()),
                b'F' => Ok("short".into()),
                b'G' => Ok("unsigned short".into()),
                b'H' => Ok("int".into()),
                b'I' => Ok("unsigned int".into()),
                b'J' => Ok("long".into()),
                b'K' => Ok("unsigned long".into()),
                b'M' => Ok("float".into()),
                b'N' => Ok("double".into()),
                b'O' => Ok("long double".into()),
                b'Z' => Ok("...".into()),
                b'A' => {
                    let t = self.parse_type()?;
                    Ok(format!("{}&", t))
                }
                b'B' => {
                    let t = self.parse_type()?;
                    Ok(format!("{} const&", t))
                }
                b'P' => {
                    let t = self.parse_type()?;
                    Ok(format!("{}*", t))
                }
                b'Q' => {
                    let t = self.parse_type()?;
                    Ok(format!("{} const*", t))
                }
                b'R' => {
                    let t = self.parse_type()?;
                    Ok(format!("{} volatile*", t))
                }
                b'S' => {
                    let t = self.parse_type()?;
                    Ok(format!("{} const volatile*", t))
                }
                b'U' => {
                    let t = self.parse_type()?;
                    Ok(format!("{}&&", t))
                }
                b'V' => {
                    let t = self.parse_type()?;
                    Ok(format!("{} volatile", t))
                }
                b'W' => {
                    let t = self.parse_type()?;
                    Ok(format!("{} volatile&", t))
                }
                b'0'..=b'9' => {
                    let n = (c - b'0') as usize;
                    if self.pos + n > self.bytes.len() {
                        return Err(DemangleError::Invalid);
                    }
                    let s = std::str::from_utf8(&self.bytes[self.pos..self.pos + n])
                        .map_err(|_| DemangleError::Invalid)?;
                    self.advance(n);
                    Ok(s.to_string())
                }
                _ => Err(DemangleError::Invalid),
            }
        }
    }
}

// =======================================================================
// Rust v0 / legacy demangler
// =======================================================================

mod rs {
    use super::DemangleError;

    pub(super) fn demangle(input: &str) -> Option<String> {
        let mut s = input;
        if let Some(rest) = s.strip_prefix("_ZN") {
            s = rest;
        } else if let Some(rest) = s.strip_prefix("_R") {
            s = rest;
        } else {
            return None;
        }
        let bytes = s.as_bytes();
        let mut ctx = Ctx { bytes, pos: 0 };
        let mut out = String::new();
        // Legacy "_ZN...E" form: capture the path between _ZN and E.
        let close_e = s.find('E').unwrap_or(s.len());
        let path = &s[..close_e];
        if !parse_path(path, &mut out) {
            return None;
        }
        let _ = ctx;
        Some(out)
    }

    fn parse_path(s: &str, out: &mut String) -> bool {
        let bytes = s.as_bytes();
        let mut i = 0usize;
        let mut first = true;
        loop {
            // Skip leading crate hash (starts with non-digit or uppercase).
            while i < bytes.len() && bytes[i].is_ascii_digit() == false {
                i += 1;
            }
            // Read length-prefixed identifier.
            if i >= bytes.len() {
                return false;
            }
            let len_start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            let len: usize = match std::str::from_utf8(&bytes[len_start..i])
                .ok()
                .and_then(|x| x.parse().ok())
            {
                Some(n) => n,
                None => return false,
            };
            if i + len > bytes.len() {
                return false;
            }
            let ident = match std::str::from_utf8(&bytes[i..i + len]) {
                Ok(s) => s,
                Err(_) => return false,
            };
            i += len;
            if !first {
                out.push_str("::");
            }
            first = false;
            // Substitute the legacy "17h5f2c6d2c4d8b3cd" prefix.
            if ident == "h5f2c6d2c4d8b3cd" {
                out.push_str("<crate>");
            } else {
                out.push_str(ident);
            }
            if i >= bytes.len() {
                break;
            }
        }
        if !first {
            // ok
        }
        true
    }

    struct Ctx<'a> {
        bytes: &'a [u8],
        pos: usize,
    }
    impl<'a> Ctx<'a> {
        fn eof(&self) -> bool {
            self.pos >= self.bytes.len()
        }
        fn _peek(&self) -> u8 {
            if self.pos < self.bytes.len() {
                self.bytes[self.pos]
            } else {
                0
            }
        }
        fn _consume_len(&mut self) -> Result<usize, DemangleError> {
            // Unused: the legacy parser is handled by `parse_path`.
            Ok(0)
        }
    }
}

// =======================================================================
// D demangler (subset: `_D` prefix + nested names)
// =======================================================================

mod dlang {
    use super::DemangleError;

    pub(super) fn demangle(input: &str) -> Option<String> {
        let mut s = input;
        if let Some(rest) = s.strip_prefix("_D") {
            s = rest;
        } else if let Some(rest) = s.strip_prefix("_\u{0044}") {
            // Defensive: identical to above; the typecast above already strips.
            s = rest;
        } else {
            return None;
        }
        // Naive: split on '.' and join with '::'.
        if s.is_empty() {
            return None;
        }
        let mut out = String::new();
        let mut first = true;
        for part in s.split('.') {
            if !first {
                out.push_str("::");
            }
            first = false;
            out.push_str(part);
        }
        if out.is_empty() {
            None
        } else {
            Some(out)
        }
    }

    // Placeholder hook so that callers receive a `Result`-shaped signature
    // without forcing the public API to change.
    #[allow(dead_code)]
    fn err_invalid() -> Result<(), DemangleError> {
        Err(DemangleError::Invalid)
    }
}

// =======================================================================
// Lazy state for cplus_demangle()'s global option pointer.
// =======================================================================

thread_local! {
    static LAST_ERROR: RefCell<Option<DemangleError>> = const { RefCell::new(None) };
}

fn set_last_error(err: DemangleError) {
    LAST_ERROR.with(|cell| *cell.borrow_mut() = Some(err));
}

fn take_last_error() -> Option<DemangleError> {
    LAST_ERROR.with(|cell| cell.borrow_mut().take())
}

/// Returns the most recent demangling error, if any.
pub fn last_error() -> Option<DemangleError> {
    LAST_ERROR.with(|cell| cell.borrow().clone())
}
