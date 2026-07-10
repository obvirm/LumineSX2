// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust 2021 translation of a focused subset of the Chaos Compiler
//! Collection (CCC) 3rdparty C/C++ library. Only the surface area that is
//! used by PCSX2's EE translation tooling is exposed here:
//!
//! * [`AstKind`] / [`AstNode`] / [`TranslationUnit`] / [`FunctionDecl`] /
//!   [`VarDecl`] / [`Expr`] / [`Stmt`] — a flat, clang-flavoured view of the
//!   C/C++ type system reconstructed from `ccc/ast.h`.
//! * [`ast_parse`] / [`ast_print`] — the parse / print entry points that
//!   mirror `stabs_to_ast` + the JSON / C++ printers in `ccc`.
//! * [`ElfSymtab`] / [`Symbol`] / [`elf_read`] — the ELF symbol-table
//!   reader from `ccc/elf_symtab.cpp` and `ccc/elf.cpp`.
//!
//! Globals (e.g. the custom error callback) are exposed through `static mut`
//! items per the project convention. Only the `std` crate is used.

#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals)]

use std::cmp::Ordering;
use std::fmt;
use std::io::{self, Write};
use std::ptr;

// =======================================================================
// Primitive type aliases — `ccc/util.h` `using u8 = unsigned char;` etc.
// =======================================================================

pub type u8 = std::os::raw::c_uchar;
pub type u16 = std::os::raw::c_ushort;
pub type u32 = std::os::raw::c_uint;
pub type u64 = std::os::raw::c_ulonglong;
pub type s8 = std::os::raw::c_schar;
pub type s16 = std::os::raw::c_short;
pub type s32 = std::os::raw::c_int;
pub type s64 = std::os::raw::c_longlong;

// =======================================================================
// Error reporting — `ccc/util.{h,cpp}`
// =======================================================================

/// Severity level for the [`Error`] callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorLevel {
    Error,
    Warning,
}

/// An error / warning record with source location, mirroring `ccc::Error`.
#[derive(Debug, Clone)]
pub struct Error {
    pub message: String,
    pub source_file: &'static str,
    pub source_line: s32,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}:{}] {}", self.source_file, self.source_line, self.message)
    }
}

/// Type of the custom error callback installed via [`set_custom_error_callback`].
pub type CustomErrorCallback = fn(error: &Error, level: ErrorLevel);

// `custom_error_callback` from `util.cpp` — `static CustomErrorCallback ...`.
// Exposed as a `static mut` per the project convention.
static mut custom_error_callback: Option<CustomErrorCallback> = None;

/// Install (or clear, when passed `None`) the custom error callback used by
/// [`report_error`] and [`report_warning`].
pub fn set_custom_error_callback(callback: Option<CustomErrorCallback>) {
    unsafe {
        custom_error_callback = callback;
    }
}

/// Build a formatted [`Error`] at the given source location. Mirrors
/// `ccc::format_error`.
pub fn format_error(
    source_file: &'static str,
    source_line: s32,
    fmt_: fmt::Arguments<'_>,
) -> Error {
    let mut message = String::new();
    let _ = fmt::write(&mut message, fmt_);
    Error {
        message,
        source_file,
        source_line,
    }
}

/// Report a [`ErrorLevel::Error`].
pub fn report_error(error: &Error) {
    unsafe {
        if let Some(cb) = custom_error_callback {
            cb(error, ErrorLevel::Error);
            return;
        }
    }
    let _ = writeln!(
        io::stderr(),
        "[{}:{}] error: {}",
        error.source_file, error.source_line, error.message
    );
}

/// Report a [`ErrorLevel::Warning`].
pub fn report_warning(warning: &Error) {
    unsafe {
        if let Some(cb) = custom_error_callback {
            cb(warning, ErrorLevel::Warning);
            return;
        }
    }
    let _ = writeln!(
        io::stderr(),
        "[{}:{}] warning: {}",
        warning.source_file, warning.source_line, warning.message
    );
}

// =======================================================================
// Address and storage plumbing — `ccc/util.h`
// =======================================================================

/// 32-bit address wrapper. `value == u32::MAX` is the "invalid" sentinel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Address {
    pub value: u32,
}

impl Address {
    /// The invalid address sentinel.
    pub const INVALID: Address = Address { value: u32::MAX };

    /// Wrap a raw address. `0` becomes an *invalid* address when used with
    /// [`Address::non_zero`] but is valid here — use [`Address::non_zero`]
    /// to match CCC's `Address::non_zero` semantics.
    pub const fn new(value: u32) -> Self {
        Address { value }
    }

    /// Returns `true` unless this is the `u32::MAX` sentinel.
    pub fn valid(&self) -> bool {
        self.value != u32::MAX
    }

    /// Returns the wrapped value or `0` when invalid.
    pub fn get_or_zero(&self) -> u32 {
        if self.valid() {
            self.value
        } else {
            0
        }
    }

    /// Mirrors `ccc::Address::non_zero`: `0` -> invalid, anything else -> valid.
    pub fn non_zero(address: u32) -> Self {
        if address != 0 {
            Address { value: address }
        } else {
            Address { value: u32::MAX }
        }
    }
}

impl PartialOrd for Address {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Address {
    fn cmp(&self, other: &Self) -> Ordering {
        self.value.cmp(&other.value)
    }
}

impl From<u32> for Address {
    fn from(value: u32) -> Self {
        Address { value }
    }
}

/// Half-open address range. Mirrors `ccc::AddressRange`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AddressRange {
    pub low: Address,
    pub high: Address,
}

impl AddressRange {
    pub fn new(low: Address, high: Address) -> Self {
        AddressRange { low, high }
    }
}

impl PartialOrd for AddressRange {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for AddressRange {
    fn cmp(&self, other: &Self) -> Ordering {
        self.low.cmp(&other.low).then(self.high.cmp(&other.high))
    }
}

/// `ccc::StabsTypeNumber` — `(file, type)` pair used to uniquely identify
/// a STABS type within a translation unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct StabsTypeNumber {
    pub file: s32,
    pub ty: s32,
}

impl StabsTypeNumber {
    pub fn valid(&self) -> bool {
        self.ty > -1
    }
}

impl PartialOrd for StabsTypeNumber {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for StabsTypeNumber {
    fn cmp(&self, other: &Self) -> Ordering {
        self.file.cmp(&other.file).then(self.ty.cmp(&other.ty))
    }
}

/// `ccc::StorageClass`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum StorageClass {
    None = 0,
    Typedef = 1,
    Extern = 2,
    Static = 3,
    Auto = 4,
    Register = 5,
}

/// `ccc::AccessSpecifier`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AccessSpecifier {
    Public = 0,
    Protected = 1,
    Private = 2,
}

impl AccessSpecifier {
    pub fn as_str(&self) -> &'static str {
        match self {
            AccessSpecifier::Public => "public",
            AccessSpecifier::Protected => "protected",
            AccessSpecifier::Private => "private",
        }
    }
}

// =======================================================================
// AST — `ccc/ast.h` + `ccc/ast.cpp`
// =======================================================================

/// Top-level "kind" of an [`AstNode`]. Maps CCC's descriptor enum to a
/// clang-style top-level surface that PCSX2 consumers iterate over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AstKind {
    /// Root of an entire translation unit.
    TranslationUnit(TranslationUnit),
    /// A function declaration (with optional body, parameters, return type).
    FunctionDecl(FunctionDecl),
    /// A variable declaration (global / static / local).
    VarDecl(VarDecl),
    /// A field declaration inside a struct / union.
    FieldDecl(FieldDecl),
    /// A `struct` / `union` declaration.
    RecordDecl(RecordDecl),
    /// An `enum` declaration.
    EnumDecl(EnumDecl),
    /// A type name reference.
    TypeRef(TypeRef),
    /// An expression node.
    Expr(Expr),
    /// A statement node.
    Stmt(Box<Stmt>),
    /// Anything that couldn't be classified — an `ast::Error` in CCC terms.
    Error(String),
}

impl AstKind {
    /// Human-readable tag for the variant, e.g. `"FunctionDecl"`.
    pub fn tag(&self) -> &'static str {
        match self {
            AstKind::TranslationUnit(_) => "TranslationUnit",
            AstKind::FunctionDecl(_) => "FunctionDecl",
            AstKind::VarDecl(_) => "VarDecl",
            AstKind::FieldDecl(_) => "FieldDecl",
            AstKind::RecordDecl(_) => "RecordDecl",
            AstKind::EnumDecl(_) => "EnumDecl",
            AstKind::TypeRef(_) => "TypeRef",
            AstKind::Expr(_) => "Expr",
            AstKind::Stmt(_) => "Stmt",
            AstKind::Error(_) => "Error",
        }
    }
}

/// A node in the AST. This is a thin, owning wrapper around an [`AstKind`]
/// that tracks common qualifiers lifted from `ccc::ast::Node`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AstNode {
    pub kind: Box<AstKind>,
    pub name: String,
    pub size_bytes: s32,
    pub offset_bytes: s32,
    pub size_bits: s32,
    pub is_const: bool,
    pub is_volatile: bool,
    pub storage_class: StorageClass,
    pub access_specifier: AccessSpecifier,
}

impl AstNode {
    /// Construct a new node with sensible defaults (sentinel sizes, no
    /// qualifiers, public access).
    pub fn new(kind: AstKind) -> Self {
        AstNode {
            kind: Box::new(kind),
            name: String::new(),
            size_bytes: -1,
            offset_bytes: -1,
            size_bits: -1,
            is_const: false,
            is_volatile: false,
            storage_class: StorageClass::None,
            access_specifier: AccessSpecifier::Public,
        }
    }

    /// Attach a name and return `self` for builder-style construction.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Attach a byte size and return `self`.
    pub fn with_size(mut self, size_bytes: s32) -> Self {
        self.size_bytes = size_bytes;
        self
    }
}

// -- Top-level declarations -------------------------------------------------

/// A translation unit: the root of a parsed AST.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TranslationUnit {
    pub decls: Vec<AstNode>,
}

impl TranslationUnit {
    pub fn new() -> Self {
        TranslationUnit { decls: Vec::new() }
    }

    /// Append a top-level declaration.
    pub fn push(&mut self, node: AstNode) {
        self.decls.push(node);
    }
}

/// A function declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionDecl {
    pub return_type: Option<Box<AstNode>>,
    pub parameters: Vec<AstNode>,
    pub body: Option<Vec<Stmt>>,
    pub is_static: bool,
    pub is_inline: bool,
    pub is_member: bool,
    pub vtable_index: s32,
}

impl FunctionDecl {
    pub fn new() -> Self {
        FunctionDecl {
            return_type: None,
            parameters: Vec::new(),
            body: None,
            is_static: false,
            is_inline: false,
            is_member: false,
            vtable_index: -1,
        }
    }
}

impl Default for FunctionDecl {
    fn default() -> Self {
        FunctionDecl::new()
    }
}

/// A variable declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VarDecl {
    pub ty: Option<Box<AstNode>>,
    pub init: Option<Expr>,
}

impl VarDecl {
    pub fn new() -> Self {
        VarDecl {
            ty: None,
            init: None,
        }
    }
}

impl Default for VarDecl {
    fn default() -> Self {
        VarDecl::new()
    }
}

/// A field declaration inside a struct or union.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldDecl {
    pub ty: Option<Box<AstNode>>,
    pub bitfield_width: Option<u32>,
}

impl FieldDecl {
    pub fn new() -> Self {
        FieldDecl {
            ty: None,
            bitfield_width: None,
        }
    }
}

impl Default for FieldDecl {
    fn default() -> Self {
        FieldDecl::new()
    }
}

/// A `struct` or `union` declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordDecl {
    pub is_union: bool,
    pub base_classes: Vec<AstNode>,
    pub fields: Vec<AstNode>,
    pub member_functions: Vec<AstNode>,
}

impl RecordDecl {
    pub fn new_struct() -> Self {
        RecordDecl {
            is_union: false,
            base_classes: Vec::new(),
            fields: Vec::new(),
            member_functions: Vec::new(),
        }
    }

    pub fn new_union() -> Self {
        RecordDecl {
            is_union: true,
            base_classes: Vec::new(),
            fields: Vec::new(),
            member_functions: Vec::new(),
        }
    }
}

/// An `enum` declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumDecl {
    /// `(value, name)` pairs in declaration order.
    pub constants: Vec<(s32, String)>,
}

impl EnumDecl {
    pub fn new() -> Self {
        EnumDecl {
            constants: Vec::new(),
        }
    }
}

impl Default for EnumDecl {
    fn default() -> Self {
        EnumDecl::new()
    }
}

/// A reference to a named type (e.g. `MyStruct *`, `typedef int I32`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeRef {
    pub name: String,
}

impl TypeRef {
    pub fn new(name: impl Into<String>) -> Self {
        TypeRef { name: name.into() }
    }
}

// -- Expressions ------------------------------------------------------------

/// A side-effect-free expression node. The enum is intentionally
/// conservative — PCSX2 only needs a tiny subset to drive its EE
/// translation. Add more variants here as callers grow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    /// A literal integer constant.
    IntLit(i128),
    /// A literal string constant.
    StrLit(String),
    /// Reference to a named entity (variable, function, label).
    DeclRef(String),
    /// A binary operation `op(lhs, rhs)`.
    Binary { op: BinOp, lhs: Box<Expr>, rhs: Box<Expr> },
    /// A unary operation `op(operand)`.
    Unary { op: UnOp, operand: Box<Expr> },
    /// A call `callee(args...)`.
    Call { callee: Box<Expr>, args: Vec<Expr> },
    /// A member access `base.field` (or `->` when `is_arrow`).
    Member {
        base: Box<Expr>,
        field: String,
        is_arrow: bool,
    },
    /// A cast `ty(expr)`.
    Cast { ty: Box<AstNode>, expr: Box<Expr> },
    /// A ternary `cond ? then : els`.
    Ternary {
        cond: Box<Expr>,
        then_branch: Box<Expr>,
        else_branch: Box<Expr>,
    },
    /// The literal `this` pointer (in C++ member functions).
    This,
}

impl Expr {
    /// Pretty-print an [`Expr`] as a single line.
    pub fn dump(&self) -> String {
        match self {
            Expr::IntLit(v) => format!("{}", v),
            Expr::StrLit(s) => format!("{:?}", s),
            Expr::DeclRef(n) => n.clone(),
            Expr::Binary { op, lhs, rhs } => format!("({} {} {})", op.symbol(), lhs.dump(), rhs.dump()),
            Expr::Unary { op, operand } => format!("({}{})", op.symbol(), operand.dump()),
            Expr::Call { callee, args } => {
                let parts: Vec<String> = args.iter().map(|a| a.dump()).collect();
                format!("{}({})", callee.dump(), parts.join(", "))
            }
            Expr::Member { base, field, is_arrow } => {
                if *is_arrow {
                    format!("({}->{})", base.dump(), field)
                } else {
                    format!("({}.{})", base.dump(), field)
                }
            }
            Expr::Cast { ty, expr } => format!("(({}{}) {})", ty.name, ty.is_const.then_some(" const").unwrap_or(""), expr.dump()),
            Expr::Ternary { cond, then_branch, else_branch } => {
                format!("({} ? {} : {})", cond.dump(), then_branch.dump(), else_branch.dump())
            }
            Expr::This => "this".to_string(),
        }
    }
}

/// Binary operator kind, mapped onto the C/C++ grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Assign,
    Comma,
}

impl BinOp {
    pub fn symbol(&self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Mod => "%",
            BinOp::And => "&&",
            BinOp::Or => "||",
            BinOp::BitAnd => "&",
            BinOp::BitOr => "|",
            BinOp::BitXor => "^",
            BinOp::Shl => "<<",
            BinOp::Shr => ">>",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::Assign => "=",
            BinOp::Comma => ",",
        }
    }
}

/// Unary operator kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Plus,
    Not,
    BitNot,
    Deref,
    AddrOf,
    PreInc,
    PreDec,
    PostInc,
    PostDec,
}

impl UnOp {
    pub fn symbol(&self) -> &'static str {
        match self {
            UnOp::Neg => "-",
            UnOp::Plus => "+",
            UnOp::Not => "!",
            UnOp::BitNot => "~",
            UnOp::Deref => "*",
            UnOp::AddrOf => "&",
            UnOp::PreInc => "++",
            UnOp::PreDec => "--",
            UnOp::PostInc => "++",
            UnOp::PostDec => "--",
        }
    }
}

// -- Statements -------------------------------------------------------------

/// A statement node. Like [`Expr`], the variant set is the small subset
/// PCSX2's EE translation actually consumes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stmt {
    /// `return expr?;`
    Return(Option<Expr>),
    /// `decl;`
    Decl(Box<AstNode>),
    /// `expr;`
    Expr(Expr),
    /// `{ stmts... }`
    Compound(Vec<Stmt>),
    /// `if (cond) then else else?`
    If {
        cond: Expr,
        then_branch: Box<Stmt>,
        else_branch: Option<Box<Stmt>>,
    },
    /// `while (cond) body`
    While { cond: Expr, body: Box<Stmt> },
    /// `do body while (cond);`
    DoWhile { body: Box<Stmt>, cond: Expr },
    /// `for (init; cond; step) body`
    For {
        init: Option<Box<Stmt>>,
        cond: Option<Expr>,
        step: Option<Expr>,
        body: Box<Stmt>,
    },
    /// `break;`
    Break,
    /// `continue;`
    Continue,
    /// `goto label;`
    Goto(String),
    /// `label: stmt`
    Label { name: String, stmt: Box<Stmt> },
    /// `switch (expr) { cases... }`
    Switch {
        expr: Expr,
        cases: Vec<SwitchCase>,
    },
}

/// One `case` / `default` arm inside a [`Stmt::Switch`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchCase {
    pub value: Option<Expr>,
    pub body: Vec<Stmt>,
}

// =======================================================================
// AST parse / print
// =======================================================================

/// Error returned by [`ast_parse`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AstParseError {
    /// The input did not contain any top-level declarations.
    Empty,
    /// A structural error (missing brace, unterminated string, ...).
    Syntax(String),
}

impl fmt::Display for AstParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AstParseError::Empty => f.write_str("empty translation unit"),
            AstParseError::Syntax(msg) => write!(f, "syntax error: {}", msg),
        }
    }
}

impl std::error::Error for AstParseError {}

/// Parse a source string into a [`TranslationUnit`].
///
/// This is a deliberately small structural parser — enough to round-trip
/// the fragments the EE translation pipeline actually generates. It is
/// *not* a full C/C++ front-end. The parser tokenises with a [`Lexer`]
/// and then drives a single recursive-descent pass.
pub fn ast_parse(src: &str) -> Result<TranslationUnit, AstParseError> {
    let mut parser = Parser::new(src);
    let unit = parser.parse_translation_unit()?;
    if unit.decls.is_empty() {
        return Err(AstParseError::Empty);
    }
    Ok(unit)
}

/// Print a [`TranslationUnit`] (or any [`AstNode`]) using C/C++ syntax.
///
/// `out` may be any `Write` implementor; in tests [`std::io::stdout()`]
/// is the common choice.
pub fn ast_print<W: Write>(out: &mut W, unit: &TranslationUnit) -> io::Result<()> {
    for decl in &unit.decls {
        print_node(out, decl, 0)?;
    }
    out.write_all(b"\n")
}

// -- Printer helpers --------------------------------------------------------

fn indent(out: &mut dyn Write, level: usize) -> io::Result<()> {
    for _ in 0..level {
        out.write_all(b"  ")?;
    }
    Ok(())
}

fn print_node(out: &mut dyn Write, node: &AstNode, depth: usize) -> io::Result<()> {
    match &*node.kind {
        AstKind::TranslationUnit(tu) => {
            for d in &tu.decls {
                print_node(out, d, depth)?;
            }
        }
        AstKind::FunctionDecl(fd) => {
            indent(out, depth)?;
            if fd.is_static {
                out.write_all(b"static ")?;
            }
            if fd.is_inline {
                out.write_all(b"inline ")?;
            }
            if let Some(ret) = &fd.return_type {
                write!(out, "{} ", ret.name)?;
            } else {
                out.write_all(b"void ")?;
            }
            write!(out, "{}(", node.name)?;
            for (i, p) in fd.parameters.iter().enumerate() {
                if i > 0 {
                    out.write_all(b", ")?;
                }
                write!(out, "{}", p.name)?;
            }
            out.write_all(b")")?;
            match &fd.body {
                Some(body) => {
                    out.write_all(b" {\n")?;
                    for s in body {
                        print_stmt(out, s, depth + 1)?;
                    }
                    indent(out, depth)?;
                    out.write_all(b"}\n")?;
                }
                None => {
                    out.write_all(b";\n")?;
                }
            }
        }
        AstKind::VarDecl(vd) => {
            indent(out, depth)?;
            if let Some(ty) = &vd.ty {
                write!(out, "{} ", ty.name)?;
            }
            write!(out, "{}", node.name)?;
            if let Some(init) = &vd.init {
                write!(out, " = {}", init.dump())?;
            }
            out.write_all(b";\n")?;
        }
        AstKind::FieldDecl(fd) => {
            indent(out, depth)?;
            if let Some(ty) = &fd.ty {
                write!(out, "{} ", ty.name)?;
            }
            write!(out, "{}", node.name)?;
            if let Some(w) = fd.bitfield_width {
                write!(out, " : {}", w)?;
            }
            out.write_all(b";\n")?;
        }
        AstKind::RecordDecl(rd) => {
            indent(out, depth)?;
            if rd.is_union {
                out.write_all(b"union ")?;
            } else {
                out.write_all(b"struct ")?;
            }
            writeln!(out, "{} {{", node.name)?;
            for f in &rd.fields {
                print_node(out, f, depth + 1)?;
            }
            indent(out, depth)?;
            out.write_all(b"};\n")?;
        }
        AstKind::EnumDecl(ed) => {
            indent(out, depth)?;
            writeln!(out, "enum {} {{", node.name)?;
            for (v, n) in &ed.constants {
                indent(out, depth + 1)?;
                writeln!(out, "{} = {},", n, v)?;
            }
            indent(out, depth)?;
            out.write_all(b"};\n")?;
        }
        AstKind::TypeRef(tr) => {
            indent(out, depth)?;
            writeln!(out, "typedef {} {};", tr.name, node.name)?;
        }
        AstKind::Expr(e) => {
            indent(out, depth)?;
            writeln!(out, "{};", e.dump())?;
        }
        AstKind::Stmt(s) => {
            print_stmt(out, s, depth)?;
        }
        AstKind::Error(msg) => {
            indent(out, depth)?;
            writeln!(out, "// ast error: {}", msg)?;
        }
    }
    Ok(())
}

fn print_stmt(out: &mut dyn Write, s: &Stmt, depth: usize) -> io::Result<()> {
    match s {
        Stmt::Return(e) => {
            indent(out, depth)?;
            if let Some(e) = e {
                writeln!(out, "return {};", e.dump())?;
            } else {
                out.write_all(b"return;\n")?;
            }
        }
        Stmt::Decl(d) => {
            print_node(out, d, depth)?;
        }
        Stmt::Expr(e) => {
            indent(out, depth)?;
            writeln!(out, "{};", e.dump())?;
        }
        Stmt::Compound(stmts) => {
            indent(out, depth)?;
            out.write_all(b"{\n")?;
            for s in stmts {
                print_stmt(out, s, depth + 1)?;
            }
            indent(out, depth)?;
            out.write_all(b"}\n")?;
        }
        Stmt::If { cond, then_branch, else_branch } => {
            indent(out, depth)?;
            writeln!(out, "if ({}) ", cond.dump())?;
            print_stmt(out, then_branch, depth)?;
            if let Some(els) = else_branch {
                indent(out, depth)?;
                out.write_all(b"else ")?;
                print_stmt(out, els, depth)?;
            }
        }
        Stmt::While { cond, body } => {
            indent(out, depth)?;
            writeln!(out, "while ({}) ", cond.dump())?;
            print_stmt(out, body, depth)?;
        }
        Stmt::DoWhile { body, cond } => {
            indent(out, depth)?;
            out.write_all(b"do ")?;
            print_stmt(out, body, depth)?;
            indent(out, depth)?;
            writeln!(out, "while ({});", cond.dump())?;
        }
        Stmt::For { init, cond, step, body } => {
            indent(out, depth)?;
            out.write_all(b"for (")?;
            if let Some(i) = init {
                let mut buf: Vec<u8> = Vec::new();
                print_stmt(&mut buf, i, 0)?;
                let s = String::from_utf8_lossy(&buf);
                write!(out, "{} ", s.trim_end())?;
            }
            if let Some(c) = cond {
                write!(out, "{}; ", c.dump())?;
            } else {
                out.write_all(b"; ")?;
            }
            if let Some(s) = step {
                write!(out, "{}", s.dump())?;
            }
            out.write_all(b") ")?;
            print_stmt(out, body, depth)?;
        }
        Stmt::Break => {
            indent(out, depth)?;
            out.write_all(b"break;\n")?;
        }
        Stmt::Continue => {
            indent(out, depth)?;
            out.write_all(b"continue;\n")?;
        }
        Stmt::Goto(l) => {
            indent(out, depth)?;
            writeln!(out, "goto {};", l)?;
        }
        Stmt::Label { name, stmt } => {
            indent(out, depth)?;
            writeln!(out, "{}:", name)?;
            print_stmt(out, stmt, depth + 1)?;
        }
        Stmt::Switch { expr, cases } => {
            indent(out, depth)?;
            writeln!(out, "switch ({}) {{", expr.dump())?;
            for c in cases {
                indent(out, depth + 1)?;
                match &c.value {
                    Some(v) => writeln!(out, "case {}:", v.dump())?,
                    None => out.write_all(b"default:\n")?,
                }
                for s in &c.body {
                    print_stmt(out, s, depth + 2)?;
                }
            }
            indent(out, depth)?;
            out.write_all(b"}\n")?;
        }
    }
    Ok(())
}

// -- Parser -----------------------------------------------------------------

struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str) -> Self {
        Lexer { src: src.as_bytes(), pos: 0 }
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let c = self.peek()?;
        self.pos += 1;
        Some(c)
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_ascii_whitespace() {
                self.pos += 1;
            } else if c == b'/' && self.src.get(self.pos + 1) == Some(&b'/') {
                while self.peek().map(|x| x != b'\n').unwrap_or(false) {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    fn next_token(&mut self) -> Option<Token> {
        self.skip_ws();
        let c = self.peek()?;
        Some(match c {
            b'{' => { self.pos += 1; Token::LBrace }
            b'}' => { self.pos += 1; Token::RBrace }
            b'(' => { self.pos += 1; Token::LParen }
            b')' => { self.pos += 1; Token::RParen }
            b';' => { self.pos += 1; Token::Semi }
            b',' => { self.pos += 1; Token::Comma }
            b':' => { self.pos += 1; Token::Colon }
            b'=' => { self.pos += 1; Token::Eq }
            b'+' => { self.pos += 1; Token::Plus }
            b'-' => { self.pos += 1; Token::Minus }
            b'*' => { self.pos += 1; Token::Star }
            b'&' => { self.pos += 1; Token::Amp }
            b'"' => self.lex_string(),
            b'0'..=b'9' => self.lex_number(),
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => self.lex_ident(),
            _ => {
                self.pos += 1;
                Token::Punct(c as char)
            }
        })
    }

    fn lex_string(&mut self) -> Token {
        // Opening quote already verified.
        self.pos += 1;
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c == b'"' {
                let s = std::str::from_utf8(&self.src[start..self.pos])
                    .unwrap_or("")
                    .to_string();
                self.pos += 1;
                return Token::Str(s);
            }
            self.pos += 1;
        }
        Token::Str(String::new())
    }

    fn lex_number(&mut self) -> Token {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                self.pos += 1;
            } else {
                break;
            }
        }
        let n: i128 = std::str::from_utf8(&self.src[start..self.pos])
            .unwrap_or("0")
            .parse()
            .unwrap_or(0);
        Token::Int(n)
    }

    fn lex_ident(&mut self) -> Token {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == b'_' {
                self.pos += 1;
            } else {
                break;
            }
        }
        let s = std::str::from_utf8(&self.src[start..self.pos])
            .unwrap_or("")
            .to_string();
        match s.as_str() {
            "return" => Token::Return,
            "if" => Token::If,
            "else" => Token::Else,
            "while" => Token::While,
            "do" => Token::Do,
            "for" => Token::For,
            "break" => Token::Break,
            "continue" => Token::Continue,
            "goto" => Token::Goto,
            "switch" => Token::Switch,
            "case" => Token::Case,
            "default" => Token::Default,
            "struct" => Token::Struct,
            "union" => Token::Union,
            "enum" => Token::Enum,
            "typedef" => Token::Typedef,
            "static" => Token::Static,
            _ => Token::Ident(s),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Ident(String),
    Int(i128),
    Str(String),
    LBrace,
    RBrace,
    LParen,
    RParen,
    Semi,
    Comma,
    Colon,
    Eq,
    Plus,
    Minus,
    Star,
    Amp,
    Return,
    If,
    Else,
    While,
    Do,
    For,
    Break,
    Continue,
    Goto,
    Switch,
    Case,
    Default,
    Struct,
    Union,
    Enum,
    Typedef,
    Static,
    /// Catch-all for any punctuation we don't model directly.
    Punct(char),
}

struct Parser<'a> {
    lex: Lexer<'a>,
    lookahead: Option<Token>,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        let mut lex = Lexer::new(src);
        let lookahead = lex.next_token();
        Parser { lex, lookahead }
    }

    fn bump(&mut self) -> Option<Token> {
        let t = self.lookahead.take()?;
        self.lookahead = self.lex.next_token();
        Some(t)
    }

    fn peek(&self) -> Option<&Token> {
        self.lookahead.as_ref()
    }

    fn eat_ident(&mut self) -> Option<String> {
        if let Some(Token::Ident(s)) = self.peek() {
            let s = s.clone();
            self.bump();
            Some(s)
        } else {
            None
        }
    }

    fn expect(&mut self, expected: &Token) -> Result<(), AstParseError> {
        match self.peek() {
            Some(t) if t == expected => {
                self.bump();
                Ok(())
            }
            Some(t) => Err(AstParseError::Syntax(format!(
                "expected {:?}, got {:?}",
                expected, t
            ))),
            None => Err(AstParseError::Syntax(format!(
                "expected {:?}, got EOF",
                expected
            ))),
        }
    }

    fn parse_translation_unit(&mut self) -> Result<TranslationUnit, AstParseError> {
        let mut tu = TranslationUnit::new();
        while self.peek().is_some() {
            let node = self.parse_top_level_decl()?;
            tu.push(node);
        }
        Ok(tu)
    }

    fn parse_top_level_decl(&mut self) -> Result<AstNode, AstParseError> {
        // Optional `static`
        let is_static = if matches!(self.peek(), Some(Token::Static)) {
            self.bump();
            true
        } else {
            false
        };

        let ty_name = self
            .eat_ident()
            .ok_or_else(|| AstParseError::Syntax("expected type".into()))?;
        let name = self
            .eat_ident()
            .ok_or_else(|| AstParseError::Syntax("expected identifier".into()))?;

        // Function: `<ty> <name> ( ... ) { ... }` or `( ... );`
        if matches!(self.peek(), Some(Token::LParen)) {
            let mut fd = FunctionDecl::new();
            fd.is_static = is_static;
            fd.return_type = Some(Box::new(AstNode::new(AstKind::TypeRef(TypeRef::new(ty_name)))));
            self.bump(); // '('
            if !matches!(self.peek(), Some(Token::RParen)) {
                loop {
                    let p_ty = self
                        .eat_ident()
                        .ok_or_else(|| AstParseError::Syntax("expected param type".into()))?;
                    let p_name = self.eat_ident().unwrap_or_default();
                    let p = AstNode::new(AstKind::VarDecl(VarDecl {
                        ty: Some(Box::new(AstNode::new(AstKind::TypeRef(TypeRef::new(p_ty))))),
                        init: None,
                    }))
                    .with_name(p_name);
                    fd.parameters.push(p);
                    if matches!(self.peek(), Some(Token::Comma)) {
                        self.bump();
                    } else {
                        break;
                    }
                }
            }
            self.expect(&Token::RParen)?;
            let body = if matches!(self.peek(), Some(Token::LBrace)) {
                self.bump();
                let mut stmts = Vec::new();
                while !matches!(self.peek(), Some(Token::RBrace) | None) {
                    stmts.push(self.parse_stmt()?);
                }
                self.expect(&Token::RBrace)?;
                Some(stmts)
            } else {
                self.expect(&Token::Semi)?;
                None
            };
            fd.body = body;
            return Ok(AstNode::new(AstKind::FunctionDecl(fd)).with_name(name));
        }

        // Variable: `<ty> <name> [= init];`
        self.expect(&Token::Semi)?;
        let vd = VarDecl { ty: Some(Box::new(AstNode::new(AstKind::TypeRef(TypeRef::new(ty_name))))), init: None };
        Ok(AstNode::new(AstKind::VarDecl(vd)).with_name(name))
    }

    fn parse_stmt(&mut self) -> Result<Stmt, AstParseError> {
        match self.peek() {
            Some(Token::Return) => {
                self.bump();
                if matches!(self.peek(), Some(Token::Semi)) {
                    self.bump();
                    Ok(Stmt::Return(None))
                } else {
                    let e = self.parse_expr()?;
                    self.expect(&Token::Semi)?;
                    Ok(Stmt::Return(Some(e)))
                }
            }
            Some(Token::LBrace) => {
                self.bump();
                let mut stmts = Vec::new();
                while !matches!(self.peek(), Some(Token::RBrace) | None) {
                    stmts.push(self.parse_stmt()?);
                }
                self.expect(&Token::RBrace)?;
                Ok(Stmt::Compound(stmts))
            }
            Some(Token::If) => {
                self.bump();
                self.expect(&Token::LParen)?;
                let cond = self.parse_expr()?;
                self.expect(&Token::RParen)?;
                let then_branch = Box::new(self.parse_stmt()?);
                let else_branch = if matches!(self.peek(), Some(Token::Else)) {
                    self.bump();
                    Some(Box::new(self.parse_stmt()?))
                } else {
                    None
                };
                Ok(Stmt::If { cond, then_branch, else_branch })
            }
            Some(Token::While) => {
                self.bump();
                self.expect(&Token::LParen)?;
                let cond = self.parse_expr()?;
                self.expect(&Token::RParen)?;
                let body = Box::new(self.parse_stmt()?);
                Ok(Stmt::While { cond, body })
            }
            Some(Token::Do) => {
                self.bump();
                let body = Box::new(self.parse_stmt()?);
                self.expect(&Token::While)?;
                self.expect(&Token::LParen)?;
                let cond = self.parse_expr()?;
                self.expect(&Token::RParen)?;
                self.expect(&Token::Semi)?;
                Ok(Stmt::DoWhile { body, cond })
            }
            Some(Token::For) => {
                self.bump();
                self.expect(&Token::LParen)?;
                let init = if matches!(self.peek(), Some(Token::Semi)) {
                    self.bump();
                    None
                } else {
                    let s = self.parse_stmt()?;
                    Some(Box::new(s))
                };
                let cond = if matches!(self.peek(), Some(Token::Semi)) {
                    self.bump();
                    None
                } else {
                    let e = self.parse_expr()?;
                    self.expect(&Token::Semi)?;
                    Some(e)
                };
                let step = if matches!(self.peek(), Some(Token::RParen)) {
                    None
                } else {
                    Some(self.parse_expr()?)
                };
                self.expect(&Token::RParen)?;
                let body = Box::new(self.parse_stmt()?);
                Ok(Stmt::For { init, cond, step, body })
            }
            Some(Token::Break) => { self.bump(); self.expect(&Token::Semi)?; Ok(Stmt::Break) }
            Some(Token::Continue) => { self.bump(); self.expect(&Token::Semi)?; Ok(Stmt::Continue) }
            Some(Token::Goto) => {
                self.bump();
                let l = self.eat_ident().ok_or_else(|| AstParseError::Syntax("expected label".into()))?;
                self.expect(&Token::Semi)?;
                Ok(Stmt::Goto(l))
            }
            Some(Token::Switch) => {
                self.bump();
                self.expect(&Token::LParen)?;
                let expr = self.parse_expr()?;
                self.expect(&Token::RParen)?;
                self.expect(&Token::LBrace)?;
                let mut cases = Vec::new();
                while !matches!(self.peek(), Some(Token::RBrace) | None) {
                    let value = if matches!(self.peek(), Some(Token::Case)) {
                        self.bump();
                        let v = self.parse_expr()?;
                        self.expect(&Token::Colon)?;
                        Some(v)
                    } else if matches!(self.peek(), Some(Token::Default)) {
                        self.bump();
                        self.expect(&Token::Colon)?;
                        None
                    } else {
                        return Err(AstParseError::Syntax("expected case/default".into()));
                    };
                    let mut body = Vec::new();
                    while !matches!(self.peek(), Some(Token::Case) | Some(Token::Default) | Some(Token::RBrace) | None) {
                        body.push(self.parse_stmt()?);
                    }
                    cases.push(SwitchCase { value, body });
                }
                self.expect(&Token::RBrace)?;
                Ok(Stmt::Switch { expr, cases })
            }
            Some(Token::Ident(_)) => {
                // Declaration: `<ty> <name>;`
                let ty_name = self.eat_ident().unwrap();
                let name = self.eat_ident().unwrap_or_default();
                self.expect(&Token::Semi)?;
                let vd = VarDecl {
                    ty: Some(Box::new(AstNode::new(AstKind::TypeRef(TypeRef::new(ty_name))))),
                    init: None,
                };
                Ok(Stmt::Decl(Box::new(AstNode::new(AstKind::VarDecl(vd)).with_name(name))))
            }
            _ => {
                let e = self.parse_expr()?;
                self.expect(&Token::Semi)?;
                Ok(Stmt::Expr(e))
            }
        }
    }

    fn parse_expr(&mut self) -> Result<Expr, AstParseError> {
        let lhs = self.parse_primary()?;
        if matches!(self.peek(), Some(Token::Eq)) {
            self.bump();
            let rhs = self.parse_expr()?;
            return Ok(Expr::Binary {
                op: BinOp::Assign,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            });
        }
        Ok(lhs)
    }

    fn parse_primary(&mut self) -> Result<Expr, AstParseError> {
        let t = self.bump().ok_or_else(|| AstParseError::Syntax("expected expression".into()))?;
        match t {
            Token::Int(n) => Ok(Expr::IntLit(n)),
            Token::Str(s) => Ok(Expr::StrLit(s)),
            Token::Ident(name) => {
                if matches!(self.peek(), Some(Token::LParen)) {
                    self.bump();
                    let mut args = Vec::new();
                    if !matches!(self.peek(), Some(Token::RParen)) {
                        loop {
                            args.push(self.parse_expr()?);
                            if matches!(self.peek(), Some(Token::Comma)) {
                                self.bump();
                            } else {
                                break;
                            }
                        }
                    }
                    self.expect(&Token::RParen)?;
                    Ok(Expr::Call { callee: Box::new(Expr::DeclRef(name)), args })
                } else {
                    Ok(Expr::DeclRef(name))
                }
            }
            other => Err(AstParseError::Syntax(format!("unexpected token {:?}", other))),
        }
    }
}

// =======================================================================
// ELF I/O — `ccc/elf.h` + `ccc/elf.cpp`
// =======================================================================

/// ELF identification header (32-bit, MIPS / little-endian layout).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElfIdentHeader {
    pub magic: u32,
    pub e_class: u8,
    pub endianness: u8,
    pub version: u8,
    pub os_abi: u8,
    pub abi_version: u8,
}

impl ElfIdentHeader {
    pub const SIZE: usize = 16;
}

/// `e_type` field of the ELF file header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum ElfFileType {
    None = 0x00,
    Rel = 0x01,
    Exec = 0x02,
    Dyn = 0x03,
    Core = 0x04,
}

/// `e_machine` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum ElfMachine {
    Mips = 0x08,
}

/// File header immediately after the ident header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElfFileHeader {
    pub ty: ElfFileType,
    pub machine: ElfMachine,
    pub version: u32,
    pub entry: u32,
    pub phoff: u32,
    pub shoff: u32,
    pub flags: u32,
    pub ehsize: u16,
    pub phentsize: u16,
    pub phnum: u16,
    pub shentsize: u16,
    pub shnum: u16,
    pub shstrndx: u16,
}

/// `sh_type` field of a section header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ElfSectionType {
    NullSection = 0x0,
    Progbits = 0x1,
    Symtab = 0x2,
    Strtab = 0x3,
    Rela = 0x4,
    Hash = 0x5,
    Dynamic = 0x6,
    Note = 0x7,
    Nobits = 0x8,
    Rel = 0x9,
    Shlib = 0xa,
    Dynsym = 0xb,
    MipsDebug = 0x70000005,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElfSectionHeader {
    pub name: u32,
    pub ty: ElfSectionType,
    pub flags: u32,
    pub addr: u32,
    pub offset: u32,
    pub size: u32,
    pub link: u32,
    pub info: u32,
    pub addralign: u32,
    pub entsize: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElfSection {
    pub name: String,
    pub header: ElfSectionHeader,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElfProgramHeader {
    pub ty: u32,
    pub offset: u32,
    pub vaddr: u32,
    pub paddr: u32,
    pub filesz: u32,
    pub memsz: u32,
    pub flags: u32,
    pub align: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElfFile {
    pub file_header: ElfFileHeader,
    pub image: Vec<u8>,
    pub sections: Vec<ElfSection>,
    pub segments: Vec<ElfProgramHeader>,
}

impl ElfFile {
    /// Parse the ELF header, section headers and program headers from
    /// `image`. Returns the populated [`ElfFile`] on success.
    pub fn parse(image: Vec<u8>) -> Result<ElfFile, Error> {
        let mut elf = ElfFile {
            file_header: ElfFileHeader {
                ty: ElfFileType::None,
                machine: ElfMachine::Mips,
                version: 0,
                entry: 0,
                phoff: 0,
                shoff: 0,
                flags: 0,
                ehsize: 0,
                phentsize: 0,
                phnum: 0,
                shentsize: 0,
                shnum: 0,
                shstrndx: 0,
            },
            image,
            sections: Vec::new(),
            segments: Vec::new(),
        };

        let ident = read_struct::<ElfIdentHeader>(&elf.image, 0)
            .ok_or_else(|| format_error(file!(), line!() as s32, format_args!("ELF ident header out of range")))?;
        if ident.magic != ELF_MAGIC {
            return Err(format_error(file!(), line!() as s32, format_args!("Not an ELF file.")));
        }
        if ident.e_class != 1 {
            return Err(format_error(file!(), line!() as s32, format_args!("Wrong ELF class (not 32 bit).")));
        }

        let header = read_struct::<ElfFileHeader>(&elf.image, ElfIdentHeader::SIZE)
            .ok_or_else(|| format_error(file!(), line!() as s32, format_args!("ELF file header out of range")))?;
        elf.file_header = header;

        let shstr_off = header
            .shoff
            .checked_add((header.shstrndx as u32) * (core::mem::size_of::<ElfSectionHeader>() as u32))
            .ok_or_else(|| format_error(file!(), line!() as s32, format_args!("section name offset overflow")))?;
        let shstr_header = read_struct::<ElfSectionHeader>(&elf.image, shstr_off as usize)
            .ok_or_else(|| format_error(file!(), line!() as s32, format_args!("ELF section name header out of range")))?;

        for i in 0..header.shnum {
            let off = header
                .shoff
                .checked_add((i as u32) * (core::mem::size_of::<ElfSectionHeader>() as u32))
                .ok_or_else(|| format_error(file!(), line!() as s32, format_args!("section header offset overflow")))? as usize;
            let sh = read_struct::<ElfSectionHeader>(&elf.image, off)
                .ok_or_else(|| format_error(file!(), line!() as s32, format_args!("ELF section header out of range")))?;
            let name_off = (shstr_header.offset + sh.name) as usize;
            let name = read_cstr(&elf.image, name_off)
                .ok_or_else(|| format_error(file!(), line!() as s32, format_args!("ELF section name out of range")))?;
            elf.sections.push(ElfSection { name, header: sh });
        }

        for i in 0..header.phnum {
            let off = header
                .phoff
                .checked_add((i as u32) * (core::mem::size_of::<ElfProgramHeader>() as u32))
                .ok_or_else(|| format_error(file!(), line!() as s32, format_args!("program header offset overflow")))? as usize;
            let ph = read_struct::<ElfProgramHeader>(&elf.image, off)
                .ok_or_else(|| format_error(file!(), line!() as s32, format_args!("ELF program header out of range")))?;
            elf.segments.push(ph);
        }

        Ok(elf)
    }

    /// Find a section by name.
    pub fn lookup_section(&self, name: &str) -> Option<&ElfSection> {
        self.sections.iter().find(|s| s.name == name)
    }

    /// Convert a file offset to a virtual address via the program header
    /// table. Returns `None` when the offset is not inside any segment.
    pub fn file_offset_to_virtual_address(&self, file_offset: u32) -> Option<u32> {
        for seg in &self.segments {
            if file_offset >= seg.offset && file_offset < seg.offset + seg.filesz {
                return Some(seg.vaddr + file_offset - seg.offset);
            }
        }
        None
    }

    /// Slice of `image` corresponding to the requested virtual address
    /// range. Returns `None` when the range is not fully contained in a
    /// single segment.
    pub fn get_virtual(&self, address: u32, size: u32) -> Option<&[u8]> {
        let end = address.checked_add(size)?;
        for seg in &self.segments {
            if address >= seg.vaddr && end <= seg.vaddr + seg.filesz {
                let begin = (seg.offset + (address - seg.vaddr)) as usize;
                let end_off = begin + size as usize;
                if end_off <= self.image.len() {
                    return Some(&self.image[begin..end_off]);
                }
            }
        }
        None
    }
}

/// Read an ELF image (raw bytes) and return a parsed [`ElfFile`].
///
/// `image` is consumed. This is the canonical entry point used by PCSX2's
/// EE translation tooling to load PS2 binaries.
pub fn elf_read(image: Vec<u8>) -> Result<ElfFile, Error> {
    ElfFile::parse(image)
}

const ELF_MAGIC: u32 = 0x464C457F; // "\x7fELF" little-endian

fn read_struct<T: Copy>(buf: &[u8], offset: usize) -> Option<T> {
    let end = offset.checked_add(core::mem::size_of::<T>())?;
    if end > buf.len() {
        return None;
    }
    let mut tmp = core::mem::MaybeUninit::<T>::uninit();
    unsafe {
        ptr::copy_nonoverlapping(buf[offset..].as_ptr(), tmp.as_mut_ptr() as *mut u8, core::mem::size_of::<T>());
        Some(tmp.assume_init())
    }
}

fn read_cstr(buf: &[u8], offset: usize) -> Option<String> {
    if offset >= buf.len() {
        return None;
    }
    let mut end = offset;
    while end < buf.len() && buf[end] != 0 {
        end += 1;
    }
    std::str::from_utf8(&buf[offset..end]).ok().map(|s| s.to_string())
}

// =======================================================================
// ELF symbol table — `ccc/elf_symtab.h` + `ccc/elf_symtab.cpp`
// =======================================================================

/// Binding strength (`ST_BIND`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SymbolBind {
    Local = 0,
    Global = 1,
    Weak = 2,
    Num = 3,
    GnuUnique = 10,
}

/// Symbol type (`ST_TYPE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SymbolType {
    Notype = 0,
    Object = 1,
    Func = 2,
    Section = 3,
    File = 4,
    Common = 5,
    Tls = 6,
    Num = 7,
    GnuIfunc = 10,
}

/// Visibility (`ST_VISIBILITY`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SymbolVisibility {
    Default = 0,
    Internal = 1,
    Hidden = 2,
    Protected = 3,
}

impl SymbolBind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SymbolBind::Local => "LOCAL",
            SymbolBind::Global => "GLOBAL",
            SymbolBind::Weak => "WEAK",
            SymbolBind::Num => "NUM",
            SymbolBind::GnuUnique => "GNU_UNIQUE",
        }
    }
}

impl SymbolType {
    pub fn as_str(&self) -> &'static str {
        match self {
            SymbolType::Notype => "NOTYPE",
            SymbolType::Object => "OBJECT",
            SymbolType::Func => "FUNC",
            SymbolType::Section => "SECTION",
            SymbolType::File => "FILE",
            SymbolType::Common => "COMMON",
            SymbolType::Tls => "TLS",
            SymbolType::Num => "NUM",
            SymbolType::GnuIfunc => "GNU_IFUNC",
        }
    }
}

impl SymbolVisibility {
    pub fn as_str(&self) -> &'static str {
        match self {
            SymbolVisibility::Default => "DEFAULT",
            SymbolVisibility::Internal => "INTERNAL",
            SymbolVisibility::Hidden => "HIDDEN",
            SymbolVisibility::Protected => "PROTECTED",
        }
    }
}

/// One row of the ELF symbol table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Symbol {
    pub name: u32,
    pub value: u32,
    pub size: u32,
    pub info: u8,
    pub other: u8,
    pub shndx: u16,
}

impl Symbol {
    pub const SIZE: usize = 16;

    pub fn ty(&self) -> SymbolType {
        unsafe { core::mem::transmute(self.info & 0x0f) }
    }

    pub fn bind(&self) -> SymbolBind {
        unsafe { core::mem::transmute(self.info >> 4) }
    }

    pub fn visibility(&self) -> SymbolVisibility {
        unsafe { core::mem::transmute(self.other & 0x03) }
    }
}

/// Decoded ELF symbol-table entry with the resolved name attached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElfSym {
    pub raw: Symbol,
    pub name: String,
    pub index: u32,
}

/// A parsed ELF symbol table, ready to be iterated or printed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ElfSymtab {
    pub entries: Vec<ElfSym>,
}

impl ElfSymtab {
    /// Parse `symtab` against the matching `strtab` and return every
    /// well-formed entry. Mirrors `ccc::elf::import_symbols` in spirit but
    /// without the database mutation side effects.
    pub fn parse(symtab: &[u8], strtab: &[u8]) -> Result<ElfSymtab, Error> {
        let count = symtab.len() / Symbol::SIZE;
        let mut entries = Vec::with_capacity(count);
        for i in 0..count {
            let off = i * Symbol::SIZE;
            let raw = read_struct::<Symbol>(symtab, off)
                .ok_or_else(|| format_error(file!(), line!() as s32, format_args!("ELF symtab out of range")))?;
            let name = read_cstr(strtab, raw.name as usize)
                .ok_or_else(|| format_error(file!(), line!() as s32, format_args!("ELF strtab out of range")))?;
            entries.push(ElfSym { raw, name, index: i as u32 });
        }
        Ok(ElfSymtab { entries })
    }

    /// Iterator over the entries in their on-disk order.
    pub fn iter(&self) -> std::slice::Iter<'_, ElfSym> {
        self.entries.iter()
    }

    /// Look up an entry by its (resolved) name.
    pub fn find_by_name(&self, name: &str) -> Option<&ElfSym> {
        self.entries.iter().find(|s| s.name == name)
    }

    /// Print the symbol table using the same column layout as
    /// `ccc::elf::print_symbol_table`.
    pub fn print<W: Write>(&self, out: &mut W) -> io::Result<()> {
        writeln!(out, "ELF SYMBOLS:")?;
        writeln!(
            out,
            "   Num:    Value  Size Type    Bind   Vis      Ndx Name"
        )?;
        for s in &self.entries {
            writeln!(
                out,
                "{:6}: {:08x} {:5} {:<7} {:<7} {:<7} {:3} {}",
                s.index,
                s.raw.value,
                s.raw.size,
                s.raw.ty().as_str(),
                s.raw.bind().as_str(),
                s.raw.visibility().as_str(),
                s.raw.shndx,
                s.name
            )?;
        }
        Ok(())
    }
}

// =======================================================================
// Tests
// =======================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_returns_err() {
        assert!(matches!(ast_parse(""), Err(AstParseError::Empty)));
    }

    #[test]
    fn parse_simple_function_roundtrip() {
        let src = "int add(int a, int b) { return a; }";
        let tu = ast_parse(src).unwrap();
        assert_eq!(tu.decls.len(), 1);
        let mut buf: Vec<u8> = Vec::new();
        ast_print(&mut buf, &tu).unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("int add(int a, int b)"));
        assert!(out.contains("return a;"));
    }

    #[test]
    fn parse_global_var() {
        let tu = ast_parse("int counter;").unwrap();
        assert_eq!(tu.decls.len(), 1);
        let mut buf: Vec<u8> = Vec::new();
        ast_print(&mut buf, &tu).unwrap();
        assert!(String::from_utf8(buf).unwrap().contains("int counter;"));
    }

    #[test]
    fn elf_magic_check() {
        let bad = vec![0u8; 64];
        assert!(elf_read(bad).is_err());
    }

    #[test]
    fn symtab_parse_minimal() {
        // Build a symtab with two entries and a strtab with two NUL-separated names.
        let sym0 = Symbol {
            name: 0,
            value: 0x1000,
            size: 4,
            info: ((SymbolBind::Global as u8) << 4) | (SymbolType::Func as u8),
            other: SymbolVisibility::Default as u8,
            shndx: 1,
        };
        let sym1 = Symbol {
            name: 5,
            value: 0x2000,
            size: 0,
            info: ((SymbolBind::Local as u8) << 4) | (SymbolType::Notype as u8),
            other: SymbolVisibility::Default as u8,
            shndx: 0,
        };
        let mut symtab: Vec<u8> = Vec::new();
        for s in [sym0, sym1] {
            let bytes: [u8; Symbol::SIZE] = unsafe { core::mem::transmute(s) };
            symtab.extend_from_slice(&bytes);
        }
        let strtab = b"main\0start\0";
        let tab = ElfSymtab::parse(&symtab, strtab).unwrap();
        assert_eq!(tab.entries.len(), 2);
        assert_eq!(tab.entries[0].name, "main");
        assert_eq!(tab.entries[0].raw.ty(), SymbolType::Func);
        assert_eq!(tab.entries[1].name, "start");
    }

    #[test]
    fn ast_kind_tag() {
        let n = AstNode::new(AstKind::VarDecl(VarDecl::new()));
        assert_eq!(n.kind.tag(), "VarDecl");
    }
}
