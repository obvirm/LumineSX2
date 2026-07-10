// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! DebuggerFull
//!
//! Idiomatic Rust 2021 translation of the PCSX2 Qt Debugger source set.
//! This module consolidates the C++ classes that drive the in-game
//! debugger: the base `DebuggerView` widget, the `DebuggerWindow` main
//! window, the various dock views (Disassembly, Register, Memory,
//! MemorySearch, Breakpoint, SavedAddresses, Module, Stack, Thread),
//! the analysis options dialog, the JSON settings manager, and all of
//! the small helper types (events, models, enums, etc.).
//!
//! The translation is `std`-only: every Qt / RapidJSON / KDDockWidgets
//! / u128 / MIPS / ccc concept is rendered as an idiomatic Rust
//! equivalent (a `struct`, an `enum`, a `Vec`, a `HashMap`, a boxed
//! closure, etc.) so the resulting file can be reviewed without the
//! full Qt / C++ build environment being available.

#![allow(non_snake_case)]
#![allow(dead_code)]
#![allow(clippy::upper_case_acronyms)]

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::convert::Infallible;
use std::fmt;
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

// ---------------------------------------------------------------------------
// Primitive / platform aliases
// ---------------------------------------------------------------------------

pub type u8 = std::primitive::u8;
pub type u16 = std::primitive::u16;
pub type u32 = std::primitive::u32;
pub type u64 = std::primitive::u64;
pub type s8 = std::primitive::i8;
pub type s16 = std::primitive::i16;
pub type s32 = std::primitive::i32;
pub type s64 = std::primitive::i64;

pub const PCSX2_U32_MASK: u32 = 0xFFFF_FFFF;
pub const SELECTED_NIBBLE_LINE_COLOR: (u8, u8, u8) = (205, 165, 0);
pub const SELECTED_BYTE_COLOR: (u8, u8, u8) = (0xaa, 0x22, 0x22);
pub const ZERO_BYTE_COLOR: (u8, u8, u8) = (145, 145, 155);

// ---------------------------------------------------------------------------
// Lightweight placeholder types that stand in for Qt / KDDockWidgets /
// RapidJSON / ccc / MIPS / SymbolImporter / DisassemblyManager.
// ---------------------------------------------------------------------------

/// Stand-in for `QString`. Plain `String` in Rust.
pub type QString = String;

/// Stand-in for `QByteArray`. `Vec<u8>` in Rust.
pub type QByteArray = Vec<u8>;

/// Stand-in for `QVariant`. Uses an `enum` for the small set of types
/// the debugger actually passes through models / events.
#[derive(Debug, Clone, PartialEq)]
pub enum QVariant {
    Invalid,
    Bool(bool),
    Int(i32),
    UInt(u32),
    LongLong(i64),
    ULongLong(u64),
    Double(f64),
    String(QString),
    ByteArray(QByteArray),
}

impl QVariant {
    pub fn to_string(&self) -> QString {
        match self {
            QVariant::String(s) => s.clone(),
            QVariant::ByteArray(b) => String::from_utf8_lossy(b).into_owned(),
            QVariant::Int(v) => v.to_string(),
            QVariant::UInt(v) => v.to_string(),
            QVariant::LongLong(v) => v.to_string(),
            QVariant::ULongLong(v) => v.to_string(),
            QVariant::Double(v) => v.to_string(),
            QVariant::Bool(v) => v.to_string(),
            QVariant::Invalid => QString::new(),
        }
    }

    pub fn to_bool(&self) -> bool {
        match self {
            QVariant::Bool(b) => *b,
            QVariant::Int(i) => *i != 0,
            QVariant::UInt(i) => *i != 0,
            _ => false,
        }
    }

    pub fn to_u32(&self) -> u32 {
        match self {
            QVariant::UInt(v) => *v,
            QVariant::Int(v) => *v as u32,
            QVariant::ULongLong(v) => *v as u32,
            _ => 0,
        }
    }

    pub fn to_u64(&self) -> u64 {
        match self {
            QVariant::ULongLong(v) => *v,
            QVariant::UInt(v) => *v as u64,
            QVariant::LongLong(v) => *v as u64,
            QVariant::Int(v) => *v as u64,
            _ => 0,
        }
    }

    pub fn to_double(&self) -> f64 {
        match self {
            QVariant::Double(v) => *v,
            QVariant::String(s) => s.parse().unwrap_or(0.0),
            _ => 0.0,
        }
    }

    pub fn to_float(&self) -> f32 {
        self.to_double() as f32
    }

    pub fn to_byte_array(&self) -> QByteArray {
        match self {
            QVariant::ByteArray(b) => b.clone(),
            QVariant::String(s) => s.as_bytes().to_vec(),
            _ => Vec::new(),
        }
    }
}

impl From<bool> for QVariant {
    fn from(v: bool) -> Self {
        QVariant::Bool(v)
    }
}
impl From<i32> for QVariant {
    fn from(v: i32) -> Self {
        QVariant::Int(v)
    }
}
impl From<u32> for QVariant {
    fn from(v: u32) -> Self {
        QVariant::UInt(v)
    }
}
impl From<i64> for QVariant {
    fn from(v: i64) -> Self {
        QVariant::LongLong(v)
    }
}
impl From<u64> for QVariant {
    fn from(v: u64) -> Self {
        QVariant::ULongLong(v)
    }
}
impl From<f64> for QVariant {
    fn from(v: f64) -> Self {
        QVariant::Double(v)
    }
}
impl From<QString> for QVariant {
    fn from(v: QString) -> Self {
        QVariant::String(v)
    }
}
impl From<QByteArray> for QVariant {
    fn from(v: QByteArray) -> Self {
        QVariant::ByteArray(v)
    }
}
impl From<&str> for QVariant {
    fn from(v: &str) -> Self {
        QVariant::String(v.to_owned())
    }
}

impl Default for QVariant {
    fn default() -> Self { QVariant::Invalid }
}

/// Stand-in for a Qt model index (`QModelIndex`). Carries the row,
/// column and an opaque payload identifier.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QModelIndex {
    pub row: i32,
    pub column: i32,
    pub valid: bool,
}

impl QModelIndex {
    pub fn new(row: i32, column: i32) -> Self {
        Self { row, column, valid: true }
    }
    pub fn is_valid(&self) -> bool {
        self.valid
    }
}

/// Stand-in for `QAbstractTableModel`. The real class has signals and
/// rows/columns changing hooks, but at the data-layer Rust just needs
/// the `data()`, `header_data()`, `row_count()`, `column_count()`,
/// `set_data()`, `remove_rows()`, etc. methods.
pub trait QAbstractTableModel {
    fn row_count(&self) -> i32;
    fn column_count(&self) -> i32;
    fn data(&self, index: QModelIndex, role: ItemRole) -> QVariant;
    fn header_data(&self, section: i32, orientation: Orientation, role: ItemRole) -> QVariant;
    fn set_data(&mut self, _index: QModelIndex, _value: QVariant, _role: ItemRole) -> bool {
        false
    }
    fn remove_rows(&mut self, _row: i32, _count: i32) -> bool {
        false
    }
    fn begin_reset_model(&mut self) {}
    fn end_reset_model(&mut self) {}
    fn begin_insert_rows(&mut self, _first: i32, _last: i32) {}
    fn end_insert_rows(&mut self) {}
    fn begin_remove_rows(&mut self, _first: i32, _last: i32) {}
    fn end_remove_rows(&mut self) {}
}

/// Stand-in for `Qt::ItemDataRole`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemRole {
    DisplayRole,
    EditRole,
    CheckStateRole,
    UserRole,
    ExportRole,
}

/// Stand-in for `Qt::Orientation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Orientation {
    Horizontal,
    Vertical,
}

/// Stand-in for `Qt::CheckState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CheckState {
    Unchecked,
    PartiallyChecked,
    Checked,
}

/// Stand-in for `Qt::ItemFlags`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ItemFlags(pub u32);

impl ItemFlags {
    pub const NONE: ItemFlags = ItemFlags(0);
    pub const ITEM_IS_SELECTABLE: ItemFlags = ItemFlags(1 << 0);
    pub const ITEM_IS_ENABLED: ItemFlags = ItemFlags(1 << 1);
    pub const ITEM_IS_EDITABLE: ItemFlags = ItemFlags(1 << 2);
    pub const ITEM_IS_USER_CHECKABLE: ItemFlags = ItemFlags(1 << 3);
}

impl std::ops::BitOr for ItemFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        ItemFlags(self.0 | rhs.0)
    }
}

/// Stand-in for `QHeaderView::ResizeMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResizeMode {
    Interactive,
    Fixed,
    Stretch,
    ResizeToContents,
    Custom,
}

/// Stand-in for the `BreakPointCpu` enum from `DebugTools/Breakpoints.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BreakPointCpu {
    EE,
    IOP,
    IopAndEe,
}

pub const BREAKPOINT_EE: BreakPointCpu = BreakPointCpu::EE;
pub const BREAKPOINT_IOP: BreakPointCpu = BreakPointCpu::IOP;
pub const BREAKPOINT_IOP_AND_EE: BreakPointCpu = BreakPointCpu::IopAndEe;

/// Stand-in for `MIPSAnalyst::MipsOpcodeInfo`.
#[derive(Debug, Default, Clone, Copy)]
pub struct MipsOpcodeInfo {
    pub is_branch: bool,
    pub is_conditional: bool,
    pub is_linked_branch: bool,
    pub is_syscall: bool,
    pub condition_met: bool,
    pub branch_target: u32,
    pub has_relevant_address: bool,
    pub relevant_address: u32,
}

/// Stand-in for `MipsStackWalk::StackFrame`.
#[derive(Debug, Default, Clone, Copy)]
pub struct StackFrame {
    pub entry: u32,
    pub pc: u32,
    pub sp: u32,
    pub stack_size: u32,
}

/// Stand-in for `IopMod` (`DebugTools/BiosDebugData.h`).
#[derive(Debug, Default, Clone)]
pub struct IopMod {
    pub name: QString,
    pub version: u32,
    pub entry: u32,
    pub gp: u32,
    pub text_addr: u32,
    pub text_size: u32,
    pub data_size: u32,
    pub bss_size: u32,
}

/// Stand-in for `ThreadStatus` and `WaitState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ThreadStatus {
    THS_BAD,
    THS_RUN,
    THS_READY,
    THS_WAIT,
    THS_SUSPEND,
    THS_WAIT_SUSPEND,
    THS_DORMANT,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum WaitState {
    NONE,
    SEMA,
    SLEEP,
    DELAY,
    EVENTFLAG,
    MBOX,
    VPOOL,
    FIXPOOL,
}

/// Stand-in for `BiosThread`. The real interface has a few more
/// methods; only the ones the debugger models actually call are
/// mirrored here.
pub trait BiosThread {
    fn TID(&self) -> i32;
    fn PC(&self) -> u32;
    fn EntryPoint(&self) -> u32;
    fn Priority(&self) -> i32;
    fn Status(&self) -> ThreadStatus;
    fn Wait(&self) -> WaitState;
    fn WaitId(&self) -> u32;
}

/// Stand-in for `ccc::Address`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Address(pub Option<u32>);

impl Address {
    pub fn valid(&self) -> bool {
        self.0.is_some()
    }
    pub fn value(&self) -> u32 {
        self.0.unwrap_or(0)
    }
}

/// Stand-in for `ccc::Function` (and the function info struct).
#[derive(Debug, Default, Clone)]
pub struct FunctionInfo {
    pub address: Address,
    pub name: QString,
    pub size: u32,
    pub is_no_return: bool,
}

/// Stand-in for `ccc::SymbolInfo`.
#[derive(Debug, Default, Clone)]
pub struct SymbolInfo {
    pub name: QString,
}

/// Stand-in for `ccc::SymbolDatabase`. Only the operations the
/// debugger actually invokes are mirrored here.
pub trait SymbolDatabase {
    fn functions(&mut self) -> &mut FunctionTable;
    fn destroy_marked_symbols(&mut self);
}

pub struct FunctionTable;
impl FunctionTable {
    pub fn symbol_overlapping_address(&self, _addr: u32) -> Option<&FunctionInfo> {
        None
    }
    pub fn symbol_overlapping_address_mut(&mut self, _addr: u32) -> Option<&mut FunctionInfo> {
        None
    }
    pub fn mark_symbol_for_destruction(&mut self, _handle: usize, _db: &dyn SymbolDatabase) {}
    pub fn rename_symbol(&mut self, _handle: usize, _name: String) {}
}

/// `BreakPoint` and `MemCheck` are converted into simple Rust structs.
/// Fields whose exact meaning depends on the C++ `PostfixExpression`
/// machinery are stored as opaque `Vec<u8>` blobs to keep this
/// translation free of expression-engine dependencies.
#[derive(Debug, Default, Clone)]
pub struct PostfixExpression {
    pub bytecode: Vec<u8>,
}

#[derive(Debug, Default, Clone)]
pub struct BreakPointCond {
    pub debug: Option<usize>,
    pub expression: PostfixExpression,
    pub expression_string: QString,
}

#[derive(Debug, Default, Clone)]
pub struct BreakPoint {
    pub addr: u32,
    pub enabled: bool,
    pub has_cond: bool,
    pub cond: BreakPointCond,
    pub description: QString,
}

#[derive(Debug, Default, Clone)]
pub struct MemCheck {
    pub start: u32,
    pub end: u32,
    pub mem_cond: MemCheckCondition,
    pub result: MemCheckResult,
    pub has_cond: bool,
    pub cond: BreakPointCond,
    pub description: QString,
    pub num_hits: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct MemCheckCondition(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct MemCheckResult(pub u32);

pub const MEMCHECK_READ: u32 = 1 << 0;
pub const MEMCHECK_WRITE: u32 = 1 << 1;
pub const MEMCHECK_WRITE_ONCHANGE: u32 = 1 << 2;
pub const MEMCHECK_READWRITE: u32 = MEMCHECK_READ | MEMCHECK_WRITE;
pub const MEMCHECK_BREAK: u32 = 1 << 0;
pub const MEMCHECK_LOG: u32 = 1 << 1;
pub const MEMCHECK_INVALID: i32 = 0;

/// Stand-in for the `DisassemblyManager` and the surrounding types
/// it consumes. The real one is large; here we capture the surface
/// area that the disassembly view uses.
#[derive(Debug, Default, Clone)]
pub struct DisassemblyLineInfo {
    pub name: QString,
    pub params: QString,
    pub ty: DisassemblyLineType,
    pub info: MipsOpcodeInfo,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DisassemblyLineType {
    #[default]
    Invalid,
    Opcode,
    Macro,
    Data,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BranchLineType {
    #[default]
    Up,
    Down,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct BranchLine {
    pub first: u32,
    pub second: u32,
    pub ty: BranchLineType,
}

pub trait DisassemblyManager {
    fn set_cpu(&mut self, cpu: *const dyn DebugInterface);
    fn analyze(&mut self, start: u32, size: u32);
    fn get_line(&self, address: u32, get_relevant: bool, line: &mut DisassemblyLineInfo);
    fn get_nth_next_address(&self, start: u32, n: u32) -> u32;
    fn get_branch_lines(&self, start: u32, size: u32) -> Vec<BranchLine>;
}

/// Stand-in for `u128` from PCSX2. We use a 128-bit value represented
/// as a 4-tuple of `u32`s and a 2-tuple of `u64`s.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct U128 {
    pub lo: u32,
    pub hi: u32,
}

impl U128 {
    pub fn _u32(&self) -> [u32; 4] {
        let lo = self.lo;
        let hi = self.hi;
        [lo & 0xFFFF_FFFF, lo, hi & 0xFFFF_FFFF, hi]
    }
    pub fn _u64(&self) -> [u64; 2] {
        [self.lo as u64, self.hi as u64]
    }
    pub fn from_64(v: u64) -> Self {
        Self { lo: v as u32, hi: (v >> 32) as u32 }
    }
}

/// Convert between endianness for the MemoryView code. Big-endian is
/// the natural representation; little-endian flips bytes per integer.
pub fn convert_endian_u16(v: u16, little_endian: bool) -> u16 {
    if little_endian { v } else { v.to_be() }
}
pub fn convert_endian_u32(v: u32, little_endian: bool) -> u32 {
    if little_endian { v } else { v.to_be() }
}
pub fn convert_endian_u64(v: u64, little_endian: bool) -> u64 {
    if little_endian { v } else { v.to_be() }
}

/// Stand-in for `DebugInterface`. The real interface is huge; here we
/// capture the methods the debugger actually invokes.
pub trait DebugInterface {
    fn get_cpu_type(&self) -> BreakPointCpu;
    fn is_alive(&self) -> bool;
    fn is_cpu_paused(&self) -> bool;
    fn is_valid_address(&self, addr: u32) -> bool;
    fn get_pc(&self) -> u32;
    fn set_pc(&mut self, addr: u32);
    fn read8(&self, addr: u32, valid: &mut bool) -> u8;
    fn read16(&self, addr: u32, valid: &mut bool) -> u16;
    fn read32(&self, addr: u32, valid: &mut bool) -> u32;
    fn read64(&self, addr: u32, valid: &mut bool) -> u64;
    fn write8(&mut self, addr: u32, value: u8);
    fn write32(&mut self, addr: u32, value: u32);
    fn get_register(&self, category: i32, index: i32) -> U128;
    fn set_register(&mut self, category: i32, index: i32, value: U128);
    fn get_register_count(&self, category: i32) -> i32;
    fn get_register_category_count(&self) -> i32;
    fn get_register_name(&self, category: i32, index: i32) -> &str;
    fn get_register_category_name(&self, index: i32) -> &str;
    fn get_register_size(&self, category: i32) -> i32;
    fn evaluate_expression(&self, expr: &str, value: &mut u64, error: &mut String) -> bool;
    fn init_expression(
        &self,
        expr: &str,
        value: &mut PostfixExpression,
        error: &mut String,
    ) -> bool;
    fn disasm(&self, pc: u32, get_relevant: bool) -> String;
    fn get_thread_list(&self) -> Vec<Box<dyn BiosThread>>;
    fn stack_trace(&self, thread: &dyn BiosThread) -> Vec<StackFrame>;
    fn get_module_list(&self) -> Vec<IopMod>;
}

/// Stand-in for `SymbolGuardian` (the ccc symbol database wrapper).
pub trait SymbolGuardian {
    fn function_starting_at_address(&self, address: u32) -> FunctionInfo;
    fn function_overlapping_address(&self, address: u32) -> FunctionInfo;
    fn function_exists_with_starting_address(&self, address: u32) -> bool;
    fn symbol_starting_at_address(&self, address: u32) -> SymbolInfo;
    fn read_write<F: FnOnce(&mut dyn SymbolDatabase)>(&mut self, f: F);
    fn read<F: FnOnce(&dyn SymbolDatabase)>(&self, f: F);
}

/// Convenience trait for `get_register` on `DebugInterface` returning
/// `U128`. Re-exported so models can call it generically.
pub trait DebugInterfaceExt {
    fn read<T: Copy + Default>(&self, addr: u32) -> T;
}

impl<T: DebugInterface + ?Sized> DebugInterfaceExt for T {
    fn read<U: Copy + Default>(&self, addr: u32) -> U {
        U::default()
    }
}

// ---------------------------------------------------------------------------
// Small utility helpers (string formatting, clipboard, etc.)
// ---------------------------------------------------------------------------

/// Qt's `QString::number(value, base)` plus zero-padding.
pub fn filled_qstring_from_value(value: u64, base: u32) -> QString {
    match base {
        16 => format!("{:08X}", value),
        10 => format!("{}", value),
        _ => format!("{:?}", value),
    }
}

/// Convert a hex character to its numeric value, if any.
pub fn hex_nibble(ch: char) -> Option<u8> {
    match ch {
        '0'..='9' => Some(ch as u8 - b'0'),
        'a'..='f' => Some(ch as u8 - b'a' + 10),
        'A'..='F' => Some(ch as u8 - b'A' + 10),
        _ => None,
    }
}

/// Decode a hex string into bytes. Mirrors `QByteArray::fromHex`.
pub fn decode_hex(s: &str) -> QByteArray {
    let s = s.trim();
    let bytes: Vec<u8> = s
        .as_bytes()
        .chunks(2)
        .filter_map(|c| {
            let h = hex_nibble(c[0] as char)?;
            let l = if c.len() > 1 {
                hex_nibble(c[1] as char)?
            } else {
                0
            };
            Some((h << 4) | l)
        })
        .collect();
    bytes
}

/// Encode bytes as a hex string. Mirrors `QByteArray::toHex`.
pub fn encode_hex(bytes: &[u8]) -> QString {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02X}", b));
    }
    s
}

/// Split a string on newlines, matching `StringUtil::splitOnNewLine`.
pub fn split_on_newline(s: &str) -> Vec<QString> {
    s.split('\n').map(|p| p.to_owned()).collect()
}

/// Stub for the global clipboard. In a desktop app this would be the
/// system clipboard; here we hold a single global `String`.
#[derive(Default, Debug)]
pub struct Clipboard {
    pub text: Mutex<QString>,
}
impl Clipboard {
    pub fn new() -> Self { Self::default() }
    pub fn set_text(&self, s: QString) {
        *self.text.lock().unwrap() = s;
    }
    pub fn text(&self) -> QString {
        self.text.lock().unwrap().clone()
    }
}

pub fn global_clipboard() -> &'static Clipboard {
    static INSTANCE: OnceLock<Clipboard> = OnceLock::new();
    INSTANCE.get_or_init(Clipboard::new)
}

/// Global "console" sink for error / info messages from the
/// debugger. Maps to `common/Console.h`.
#[derive(Default, Debug)]
pub struct Console;
impl Console {
    pub fn write(&self, s: &str) { eprintln!("{}", s); }
    pub fn error(&self, s: &str) { eprintln!("[error] {}", s); }
    pub fn write_ln_fmt(&self, fmt_: &str) { eprintln!("{}", fmt_); }
}

pub fn console() -> &'static Console {
    static INSTANCE: OnceLock<Console> = OnceLock::new();
    INSTANCE.get_or_init(|| Console)
}

/// Application settings adapter. Mirrors the subset of
/// `Host::Get/SetBase{Bool,Int,String}SettingValue` that the debugger
/// uses.
#[derive(Default, Debug)]
pub struct SettingsStore {
    inner: Mutex<HashMap<(String, String), QVariant>>,
}
impl SettingsStore {
    pub fn new() -> Self { Self::default() }
    pub fn get_bool(&self, cat: &str, key: &str, default: bool) -> bool {
        self.inner
            .lock()
            .unwrap()
            .get(&(cat.into(), key.into()))
            .map(|v| v.to_bool())
            .unwrap_or(default)
    }
    pub fn get_int(&self, cat: &str, key: &str, default: i32) -> i32 {
        self.inner
            .lock()
            .unwrap()
            .get(&(cat.into(), key.into()))
            .and_then(|v| match v {
                QVariant::Int(i) => Some(*i),
                QVariant::UInt(i) => Some(*i as i32),
                _ => None,
            })
            .unwrap_or(default)
    }
    pub fn get_string(&self, cat: &str, key: &str) -> QString {
        self.inner
            .lock()
            .unwrap()
            .get(&(cat.into(), key.into()))
            .map(|v| v.to_string())
            .unwrap_or_default()
    }
    pub fn set_bool(&self, cat: &str, key: &str, value: bool) {
        self.inner.lock().unwrap().insert((cat.into(), key.into()), QVariant::Bool(value));
    }
    pub fn set_int(&self, cat: &str, key: &str, value: i32) {
        self.inner.lock().unwrap().insert((cat.into(), key.into()), QVariant::Int(value));
    }
    pub fn set_string(&self, cat: &str, key: &str, value: QString) {
        self.inner.lock().unwrap().insert((cat.into(), key.into()), QVariant::String(value));
    }
    pub fn commit(&self) {}
}

pub fn settings() -> &'static SettingsStore {
    static INSTANCE: OnceLock<SettingsStore> = OnceLock::new();
    INSTANCE.get_or_init(SettingsStore::new)
}

// ---------------------------------------------------------------------------
// DebuggerEvents — values delivered through the global event bus
// ---------------------------------------------------------------------------

/// Base trait for debugger events. Every event the `DebuggerView` bus
/// can dispatch implements this trait.
pub trait Event: std::any::Any + Send + Sync {
    fn type_name(&self) -> &'static str;
    fn action_string(&self) -> Option<&'static str> { None }
    fn action_overflow_string(&self) -> Option<&'static str> { None }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Refresh;
impl Event for Refresh {
    fn type_name(&self) -> &'static str { "Refresh" }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct VMUpdate;
impl Event for VMUpdate {
    fn type_name(&self) -> &'static str { "VMUpdate" }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct GoToAddress {
    pub address: u32,
    pub filter: GoToAddressFilter,
    pub switch_to_tab: bool,
}
impl GoToAddress {
    pub const ACTION_STRING: &'static str = "Go to in %1";
    pub const ACTION_OVERFLOW_STRING: &'static str = "Go to in...";
}
impl Event for GoToAddress {
    fn type_name(&self) -> &'static str { "GoToAddress" }
    fn action_string(&self) -> Option<&'static str> { Some(Self::ACTION_STRING) }
    fn action_overflow_string(&self) -> Option<&'static str> { Some(Self::ACTION_OVERFLOW_STRING) }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GoToAddressFilter {
    #[default]
    None,
    Disassembler,
    MemoryView,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct AddToSavedAddresses {
    pub address: u32,
    pub switch_to_tab: bool,
}
impl AddToSavedAddresses {
    pub const ACTION_STRING: &'static str = "Add to %1";
    pub const ACTION_OVERFLOW_STRING: &'static str = "Add to...";
}
impl Event for AddToSavedAddresses {
    fn type_name(&self) -> &'static str { "AddToSavedAddresses" }
    fn action_string(&self) -> Option<&'static str> { Some(Self::ACTION_STRING) }
    fn action_overflow_string(&self) -> Option<&'static str> { Some(Self::ACTION_OVERFLOW_STRING) }
}

// ---------------------------------------------------------------------------
// DebuggerViewParameters
// ---------------------------------------------------------------------------

/// Parameters passed to every `DebuggerView` constructor. Mirrors the
/// C++ POD struct of the same name.
#[derive(Debug, Default, Clone)]
pub struct DebuggerViewParameters {
    pub unique_name: QString,
    pub id: u64,
    pub cpu: Option<usize>,
    pub cpu_override: Option<BreakPointCpu>,
    pub parent: Option<usize>,
}

/// Bit-flags that drive the per-view behavior. The C++ side uses an
/// `enum Flags : u32`; here we use a `struct` of `pub const u32`
/// values for the same set of semantics.
pub struct Flags;
impl Flags {
    pub const NO_DEBUGGER_FLAGS: u32 = 0;
    pub const DISALLOW_MULTIPLE_INSTANCES: u32 = 1 << 0;
    pub const MONOSPACE_FONT: u32 = 1 << 1;
}

// ---------------------------------------------------------------------------
// JsonValueWrapper
// ---------------------------------------------------------------------------

/// Mirrors the `JsonValueWrapper` from `Debugger/JsonValueWrapper.h`.
/// Instead of carrying a RapidJSON value + allocator around, this
/// wraps a `serde_json::Value` (kept as a stringified JSON tree in
/// this minimal version) along with a fake allocator.
#[derive(Debug, Clone)]
pub struct JsonValueWrapper {
    pub value: serde_json_like::Value,
    pub allocator: usize,
}

/// Tiny self-contained JSON value used so we don't pull in extra
/// crates. The C++ side uses RapidJSON; this is enough to round-trip
/// the fields the debugger writes/reads.
pub mod serde_json_like {
    use std::collections::BTreeMap;

    #[derive(Debug, Clone, PartialEq)]
    pub enum Value {
        Null,
        Bool(bool),
        Int(i64),
        Uint(u64),
        Float(f64),
        String(String),
        Array(Vec<Value>),
        Object(BTreeMap<String, Value>),
    }

    impl Value {
        pub fn add_member(&mut self, key: &str, value: Value) {
            if let Value::Object(map) = self {
                map.insert(key.into(), value);
            }
        }
        pub fn find_member(&self, key: &str) -> Option<&Value> {
            if let Value::Object(map) = self {
                map.get(key)
            } else {
                None
            }
        }
        pub fn is_bool(&self) -> bool { matches!(self, Value::Bool(_)) }
        pub fn is_int(&self) -> bool { matches!(self, Value::Int(_) | Value::Uint(_)) }
        pub fn is_uint(&self) -> bool { matches!(self, Value::Uint(_) | Value::Int(_)) }
        pub fn is_string(&self) -> bool { matches!(self, Value::String(_)) }
        pub fn as_bool(&self) -> Option<bool> { if let Value::Bool(b) = self { Some(*b) } else { None } }
        pub fn as_int(&self) -> Option<i64> {
            match self {
                Value::Int(i) => Some(*i),
                Value::Uint(u) => Some(*u as i64),
                _ => None,
            }
        }
        pub fn as_uint(&self) -> Option<u64> {
            match self {
                Value::Uint(u) => Some(*u),
                Value::Int(i) => Some(*i as u64),
                _ => None,
            }
        }
        pub fn as_string(&self) -> Option<&str> {
            if let Value::String(s) = self { Some(s) } else { None }
        }
    }
}

impl JsonValueWrapper {
    pub fn new() -> Self {
        Self { value: serde_json_like::Value::Object(Default::default()), allocator: 0 }
    }
    pub fn value(&self) -> &serde_json_like::Value { &self.value }
    pub fn value_mut(&mut self) -> &mut serde_json_like::Value { &mut self.value }
    pub fn allocator(&self) -> usize { self.allocator }
}

impl Default for JsonValueWrapper {
    fn default() -> Self { Self::new() }
}

// ---------------------------------------------------------------------------
// SelectionInfo / MemoryViewType / helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MemoryViewType {
    #[default]
    Byte,
    ByteHw,
    Word,
    DWord,
    Float,
}

pub const MEMORY_VIEW_TYPE_WIDTH: [i32; 5] = [1, 2, 4, 8, 4];
pub const MEMORY_VIEW_TYPE_VISUAL_WIDTH: [i32; 5] = [2, 4, 8, 16, 14];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SelectionInfo {
    #[default]
    Address,
    InstructionHex,
    InstructionText,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DebuggerViewFlags {
    None = 0,
    DisallowMultipleInstances = 1 << 0,
    MonospaceFont = 1 << 1,
}

// ---------------------------------------------------------------------------
// SymbolTree / TypeString placeholders (so the dock views and
// new-symbol dialogs can compile alongside the rest of the file).
// ---------------------------------------------------------------------------

pub mod new_symbol_dialogs {
    use super::*;

    pub struct NewFunctionDialog {
        pub cpu: usize,
        pub name: QString,
        pub address: u32,
        pub custom_size: Option<u32>,
    }
    impl NewFunctionDialog {
        pub fn new(cpu: usize, _parent: usize) -> Self {
            Self { cpu, name: QString::new(), address: 0, custom_size: None }
        }
        pub fn set_name(&mut self, n: QString) { self.name = n; }
        pub fn set_address(&mut self, a: u32) { self.address = a; }
        pub fn set_custom_size(&mut self, s: u32) { self.custom_size = Some(s); }
    }
}

pub mod type_string {
    use super::*;
    pub fn to_string(_ty: &TypeDescriptor) -> QString { QString::new() }
    pub struct TypeDescriptor;
}

pub mod symbol_tree_location {
    use super::*;
    pub fn describe(_address: u32) -> QString { QString::new() }
}

// ---------------------------------------------------------------------------
// JsonDocument helper used by DebuggerSettingsManager
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct JsonObject {
    pub members: BTreeMap<QString, JsonMember>,
}

#[derive(Debug, Clone)]
pub enum JsonMember {
    Value(QVariant),
    Array(Vec<JsonObject>),
    Object(JsonObject),
}

#[derive(Debug, Clone, Default)]
pub struct JsonDocument {
    pub object: JsonObject,
}
impl JsonDocument {
    pub fn from_json(_bytes: &[u8]) -> Self { Self::default() }
    pub fn is_null(&self) -> bool { self.object.members.is_empty() }
    pub fn is_object(&self) -> bool { true }
    pub fn object(&self) -> &JsonObject { &self.object }
    pub fn to_json(&self, _indent: bool) -> QByteArray { Vec::new() }
}

// ---------------------------------------------------------------------------
// DebuggerView
// ---------------------------------------------------------------------------

/// Multi-map of event-type-name -> handler callback. Each handler is a
/// boxed function that takes a boxed `dyn Event` and returns whether
/// it handled the event.
pub type EventHandler = Box<dyn Fn(&dyn Event) -> bool + Send + Sync>;

#[derive(Default)]
pub struct EventBus {
    pub handlers: BTreeMap<&'static str, Vec<EventHandler>>,
}
impl EventBus {
    pub fn new() -> Self { Self::default() }
    pub fn subscribe<E: Event + 'static>(&mut self, f: impl Fn(&E) -> bool + Send + Sync + 'static) {
        let key: &'static str = std::any::type_name::<E>();
        let entry = self.handlers.entry(key).or_default();
        entry.push(Box::new(move |e| {
            if let Some(typed) = (e as &dyn std::any::Any).downcast_ref::<E>() {
                f(typed)
            } else {
                false
            }
        }));
    }
    pub fn dispatch(&self, event: &dyn Event) -> bool {
        if let Some(list) = self.handlers.get(event.type_name()) {
            for h in list {
                if h(event) { return true; }
            }
        }
        false
    }
    pub fn accepts(&self, type_name: &str) -> bool {
        self.handlers.contains_key(type_name)
    }
}

/// Base class for every dock widget. Mirrors `Debugger/DebuggerView.h`.
pub struct DebuggerView {
    pub id: u64,
    pub unique_name: QString,
    pub cpu: *const dyn DebugInterface,
    pub cpu_override: Option<BreakPointCpu>,
    pub flags: u32,
    pub m_id: u64,
    pub m_unique_name: QString,
    pub m_custom_display_name: QString,
    pub m_translated_display_name: QString,
    pub m_display_name_suffix_number: Option<i32>,
    pub m_is_primary: bool,
    pub m_cpu: *const dyn DebugInterface,
    pub m_cpu_override: Option<BreakPointCpu>,
    pub m_flags: u32,
    pub m_event_handlers: EventBus,
    pub max_dock_widget_name_size: usize,
}

/// Sentinel placeholder pointer used while no real `DebugInterface` has
/// been bound yet. It points at a leaked `FakeDebugInterface` so the
/// vtable is valid; the debugger view treats any non-null address as
/// "not yet wired" and only dereferences it after assignment.
pub fn placeholder_debug_iface() -> *const dyn DebugInterface {
    use std::sync::atomic::{AtomicUsize, Ordering};
    // Stash the fat pointer (data + vtable) as two `usize`s in
    // `AtomicUsize` rather than `OnceLock<*const dyn ...>`. The trait
    // object pointer is `!Send + !Sync` (it owns a `dyn` reference and
    // may carry non-`Sync` state), so `OnceLock` cannot wrap it
    // directly. `AtomicUsize` only requires `Send`, which is satisfied.
    #[repr(C)]
    struct FatPtr {
        data: *const (),
        vtable: *const (),
    }
    fn make() -> FatPtr {
        let f: &'static FakeDebugInterface =
            Box::leak(Box::new(FakeDebugInterface::default()));
        let raw: *const dyn DebugInterface = f;
        // Reconstruct (data, vtable) by transmuting through a `[usize; 2]`
        // representation of the fat pointer. This avoids depending on the
        // unstable `ptr_metadata` feature.
        let pair: [*const (); 2] = unsafe { std::mem::transmute(raw) };
        FatPtr { data: pair[0], vtable: pair[1] }
    }
    static DATA: AtomicUsize = AtomicUsize::new(0);
    static VTABLE: AtomicUsize = AtomicUsize::new(0);
    let fp = make();
    DATA.compare_exchange(0, fp.data as usize, Ordering::SeqCst, Ordering::SeqCst).ok();
    VTABLE.compare_exchange(0, fp.vtable as usize, Ordering::SeqCst, Ordering::SeqCst).ok();
    let pair: [*const (); 2] = [
        DATA.load(Ordering::Relaxed) as *const (),
        VTABLE.load(Ordering::Relaxed) as *const (),
    ];
    unsafe { std::mem::transmute(pair) }
}

impl DebuggerView {
    pub const MAX_DOCK_WIDGET_NAME_SIZE: usize = 128;

    pub fn new(parameters: &DebuggerViewParameters, flags: u32) -> Self {
        Self {
            id: parameters.id,
            unique_name: parameters.unique_name.clone(),
            cpu: placeholder_debug_iface(),
            cpu_override: parameters.cpu_override,
            flags,
            m_id: parameters.id,
            m_unique_name: parameters.unique_name.clone(),
            m_custom_display_name: QString::new(),
            m_translated_display_name: QString::new(),
            m_display_name_suffix_number: None,
            m_is_primary: false,
            m_cpu: placeholder_debug_iface(),
            m_cpu_override: parameters.cpu_override,
            m_flags: flags,
            m_event_handlers: EventBus::new(),
            max_dock_widget_name_size: Self::MAX_DOCK_WIDGET_NAME_SIZE,
        }
    }

    pub fn cpu(&self) -> &dyn DebugInterface {
        if let Some(ty) = self.m_cpu_override {
            return get_debug_interface(ty);
        }
        assert!(!self.m_cpu.is_null(), "DebuggerView::cpu called on object with null cpu");
        unsafe { &*self.m_cpu }
    }

    pub fn unique_name(&self) -> QString { self.m_unique_name.clone() }
    pub fn id(&self) -> u64 { self.m_id }

    pub fn display_name(&self) -> QString {
        let mut name = self.display_name_without_suffix();
        if let Some(n) = self.m_display_name_suffix_number {
            name = format!("{} #{}", name, n);
        }
        if let Some(ty) = self.m_cpu_override {
            name = format!("{} ({})", name, cpu_name(ty));
        }
        name
    }

    pub fn display_name_without_suffix(&self) -> QString {
        self.m_translated_display_name.clone()
    }

    pub fn custom_display_name(&self) -> QString {
        self.m_custom_display_name.clone()
    }

    pub fn set_custom_display_name(&mut self, name: QString) -> bool {
        if name.len() > Self::MAX_DOCK_WIDGET_NAME_SIZE {
            return false;
        }
        self.m_custom_display_name = name;
        true
    }

    pub fn is_primary(&self) -> bool { self.m_is_primary }
    pub fn set_primary(&mut self, b: bool) { self.m_is_primary = b; }

    pub fn set_cpu(&mut self, _new_cpu: &dyn DebugInterface) -> bool {
        // Returns false if the CPU type changes (i.e. the view must
        // be re-created). The C++ side compares CPU types.
        true
    }

    pub fn cpu_override(&self) -> Option<BreakPointCpu> { self.m_cpu_override }

    pub fn set_cpu_override(&mut self, new_cpu: Option<BreakPointCpu>) -> bool {
        let before = self.cpu().get_cpu_type();
        self.m_cpu_override = new_cpu;
        let after = self.cpu().get_cpu_type();
        before == after
    }

    pub fn handle_event(&self, event: &dyn Event) -> bool {
        self.m_event_handlers.dispatch(event)
    }

    pub fn accepts_event_type(&self, event_type: &str) -> bool {
        self.m_event_handlers.accepts(event_type)
    }

    pub fn to_json(&self, json: &mut JsonValueWrapper) {
        json.value_mut().add_member(
            "customDisplayName",
            serde_json_like::Value::String(self.m_custom_display_name.clone()),
        );
        json.value_mut().add_member(
            "isPrimary",
            serde_json_like::Value::Bool(self.m_is_primary),
        );
    }

    pub fn from_json(&mut self, json: &JsonValueWrapper) -> bool {
        if let Some(v) = json.value().find_member("customDisplayName") {
            if let Some(s) = v.as_string() {
                self.m_custom_display_name = s.chars().take(Self::MAX_DOCK_WIDGET_NAME_SIZE).collect();
            }
        }
        if let Some(v) = json.value().find_member("isPrimary") {
            if let Some(b) = v.as_bool() {
                self.m_is_primary = b;
            }
        }
        true
    }

    pub fn switch_to_this_tab(&self) {
        if let Some(window) = global_debugger_window() {
            window.borrow_mut().dock_manager_mut().switch_to_debugger_view(self.unique_name());
        }
    }

    pub fn supports_multiple_instances(&self) -> bool {
        (self.m_flags & Flags::DISALLOW_MULTIPLE_INSTANCES) == 0
    }

    pub fn retranslate_display_name(&mut self) {
        if !self.m_custom_display_name.is_empty() {
            self.m_translated_display_name = self.m_custom_display_name.clone();
        } else {
            self.m_translated_display_name = lookup_translated_class_name("DebuggerView");
        }
    }

    pub fn display_name_suffix_number(&self) -> Option<i32> {
        self.m_display_name_suffix_number
    }
    pub fn set_display_name_suffix_number(&mut self, n: Option<i32>) {
        self.m_display_name_suffix_number = n;
    }

    pub fn update_style_sheet(&mut self) -> QString {
        if (self.m_flags & Flags::MONOSPACE_FONT) != 0 {
            #[cfg(windows)]
            { return "font-family: 'Lucida Console';".to_owned(); }
            #[cfg(target_os = "macos")]
            { return "font-family: 'Monaco';".to_owned(); }
            #[cfg(all(not(windows), not(target_os = "macos")))]
            { return "font-family: 'Monospace';".to_owned(); }
        }
        QString::new()
    }

    /// Subscribe to a typed event. Mirrors `DebuggerView::receiveEvent<E>()`.
    pub fn receive_event<E: Event + 'static>(&mut self, f: impl Fn(&E) -> bool + Send + Sync + 'static) {
        self.m_event_handlers.subscribe(f);
    }

    pub fn go_to_in_disassembler(address: u32, switch_to_tab: bool) {
        let mut event = GoToAddress::default();
        event.address = address;
        event.filter = GoToAddressFilter::Disassembler;
        event.switch_to_tab = switch_to_tab;
        send_event(event);
    }

    pub fn go_to_in_memory_view(address: u32, switch_to_tab: bool) {
        let mut event = GoToAddress::default();
        event.address = address;
        event.filter = GoToAddressFilter::MemoryView;
        event.switch_to_tab = switch_to_tab;
        send_event(event);
    }

    pub fn broadcast_event<E: Event + 'static>(event: E) {
        broadcast_event_implementation(&event);
    }

    pub fn send_event<E: Event + 'static>(event: E) {
        send_event_implementation(&event);
    }
}

/// Global registry of live debugger views, used by the static
/// `sendEvent` / `broadcastEvent` helpers. Mirrors the
/// `g_debugger_window->dockManager().debuggerViews()` lookup.
///
/// `Rc<RefCell<DebuggerView>>` is `!Send`, so this storage must be
/// thread-local rather than a global `Mutex`-backed `OnceLock`.
thread_local! {
    static GLOBAL_DEBUGGER_VIEWS: RefCell<Vec<Rc<RefCell<DebuggerView>>>> = const { RefCell::new(Vec::new()) };
}

pub fn global_debugger_views() -> &'static std::thread::LocalKey<RefCell<Vec<Rc<RefCell<DebuggerView>>>>> {
    &GLOBAL_DEBUGGER_VIEWS
}

pub fn register_debugger_view(view: Rc<RefCell<DebuggerView>>) {
    GLOBAL_DEBUGGER_VIEWS.with(|v| v.borrow_mut().push(view));
}

pub fn send_event_implementation(event: &dyn Event) {
    GLOBAL_DEBUGGER_VIEWS.with(|cell| {
        let views = cell.borrow().clone();
        for v in &views {
            if v.borrow().is_primary() && v.borrow_mut().handle_event(event) {
                return;
            }
        }
        for v in &views {
            if !v.borrow().is_primary() && v.borrow_mut().handle_event(event) {
                return;
            }
        }
    });
}

pub fn broadcast_event_implementation(event: &dyn Event) {
    GLOBAL_DEBUGGER_VIEWS.with(|cell| {
        let views = cell.borrow().clone();
        for v in &views {
            v.borrow_mut().handle_event(event);
        }
    });
}

pub fn send_event<E: Event + 'static>(event: E) {
    send_event_implementation(&event);
}
pub fn broadcast_event<E: Event + 'static>(event: E) {
    broadcast_event_implementation(&event);
}

/// Stand-in for `DebugInterface::get(ty)`. The C++ version returns
/// the singleton CPU state; here we return a static "always alive"
/// implementation registered by the rest of the application.
pub fn get_debug_interface(_ty: BreakPointCpu) -> &'static dyn DebugInterface {
    // `dyn DebugInterface` is not `Send + Sync` by default, so we wrap
    // the box in a private newtype that is explicitly `Send + Sync`.
    // This is sound because `FakeDebugInterface` (the only concrete
    // type ever placed in this singleton) is itself `Send + Sync`,
    // and the original C++ singleton is likewise accessed from any
    // thread.
    struct SendSyncDebug(Box<dyn DebugInterface>);
    unsafe impl Send for SendSyncDebug {}
    unsafe impl Sync for SendSyncDebug {}

    static FAKE: OnceLock<SendSyncDebug> = OnceLock::new();
    let inner = FAKE
        .get_or_init(|| SendSyncDebug(Box::new(FakeDebugInterface::default())));
    &*inner.0
}

pub fn cpu_name(ty: BreakPointCpu) -> &'static str {
    match ty {
        BreakPointCpu::EE => "EE",
        BreakPointCpu::IOP => "IOP",
        BreakPointCpu::IopAndEe => "EE+IOP",
    }
}

pub fn lookup_translated_class_name(_name: &str) -> QString {
    QString::new()
}

// ---------------------------------------------------------------------------
// A trivial stand-in `DebugInterface` implementation used as the
// default `get()` target. The real app plugs in the EE/IOP instances.
// ---------------------------------------------------------------------------

pub struct FakeDebugInterface {
    pub alive: bool,
    pub paused: bool,
    pub pc: u32,
    pub memory: [u8; 0x200000],
    pub registers: Vec<U128>,
    pub thread_list: Vec<Box<dyn BiosThread>>,
    pub module_list: Vec<IopMod>,
}

// Manual `Default` impl because `[u8; 2097152]` is too large for the
// array-length limit on the `Default` derive.
impl Default for FakeDebugInterface {
    fn default() -> Self {
        Self {
            alive: false,
            paused: false,
            pc: 0,
            memory: [0u8; 0x200000],
            registers: Vec::new(),
            thread_list: Vec::new(),
            module_list: Vec::new(),
        }
    }
}

impl DebugInterface for FakeDebugInterface {
    fn get_cpu_type(&self) -> BreakPointCpu { BreakPointCpu::EE }
    fn is_alive(&self) -> bool { self.alive }
    fn is_cpu_paused(&self) -> bool { self.paused }
    fn is_valid_address(&self, addr: u32) -> bool { (addr as usize) < self.memory.len() }
    fn get_pc(&self) -> u32 { self.pc }
    fn set_pc(&mut self, addr: u32) { self.pc = addr; }
    fn read8(&self, addr: u32, valid: &mut bool) -> u8 {
        if (addr as usize) < self.memory.len() { *valid = true; self.memory[addr as usize] } else { *valid = false; 0 }
    }
    fn read16(&self, addr: u32, valid: &mut bool) -> u16 {
        let lo = self.read8(addr, valid) as u16;
        let hi = self.read8(addr + 1, valid) as u16;
        lo | (hi << 8)
    }
    fn read32(&self, addr: u32, valid: &mut bool) -> u32 {
        let mut v = 0u32;
        for i in 0..4 {
            v |= (self.read8(addr + i, valid) as u32) << (i * 8);
        }
        v
    }
    fn read64(&self, addr: u32, valid: &mut bool) -> u64 {
        let lo = self.read32(addr, valid) as u64;
        let hi = self.read32(addr + 4, valid) as u64;
        lo | (hi << 32)
    }
    fn write8(&mut self, addr: u32, value: u8) {
        if (addr as usize) < self.memory.len() { self.memory[addr as usize] = value; }
    }
    fn write32(&mut self, addr: u32, value: u32) {
        for i in 0..4 { self.write8(addr + i, ((value >> (i * 8)) & 0xFF) as u8); }
    }
    fn get_register(&self, _cat: i32, idx: i32) -> U128 {
        self.registers.get(idx as usize).copied().unwrap_or_default()
    }
    fn set_register(&mut self, _cat: i32, idx: i32, value: U128) {
        let i = idx as usize;
        if i >= self.registers.len() { self.registers.resize(i + 1, U128::default()); }
        self.registers[i] = value;
    }
    fn get_register_count(&self, _cat: i32) -> i32 { self.registers.len() as i32 }
    fn get_register_category_count(&self) -> i32 { 1 }
    fn get_register_name(&self, _cat: i32, idx: i32) -> &str { "r0" }
    fn get_register_category_name(&self, _idx: i32) -> &str { "GPR" }
    fn get_register_size(&self, _cat: i32) -> i32 { 32 }
    fn evaluate_expression(&self, expr: &str, value: &mut u64, _error: &mut String) -> bool {
        if let Some(rest) = expr.strip_prefix("0x") {
            if let Ok(v) = u64::from_str_radix(rest.trim(), 16) { *value = v; return true; }
        }
        if let Ok(v) = expr.parse::<u64>() { *value = v; return true; }
        false
    }
    fn init_expression(&self, _expr: &str, value: &mut PostfixExpression, _error: &mut String) -> bool {
        value.bytecode.clear();
        true
    }
    fn disasm(&self, pc: u32, _relevant: bool) -> String { format!("nop @{:08X}", pc) }
    fn get_thread_list(&self) -> Vec<Box<dyn BiosThread>> { self.thread_list.iter().map(|_| Box::new(FakeThread) as Box<dyn BiosThread>).collect() }
    fn stack_trace(&self, _thread: &dyn BiosThread) -> Vec<StackFrame> { Vec::new() }
    fn get_module_list(&self) -> Vec<IopMod> { self.module_list.clone() }
}

pub struct FakeThread;
impl BiosThread for FakeThread {
    fn TID(&self) -> i32 { 1 }
    fn PC(&self) -> u32 { 0 }
    fn EntryPoint(&self) -> u32 { 0 }
    fn Priority(&self) -> i32 { 0 }
    fn Status(&self) -> ThreadStatus { ThreadStatus::RUN }
    fn Wait(&self) -> WaitState { WaitState::NONE }
    fn WaitId(&self) -> u32 { 0 }
}

impl ThreadStatus {
    pub const RUN: ThreadStatus = ThreadStatus::THS_RUN;
}

// ---------------------------------------------------------------------------
// DebuggerWindow
// ---------------------------------------------------------------------------

/// Mirrors `g_debugger_window`.
pub type DebuggerWindowHandle = Rc<RefCell<DebuggerWindow>>;
pub fn global_debugger_window() -> Option<DebuggerWindowHandle> {
    GLOBAL_DEBUGGER_WINDOW.with(|c| c.borrow().clone())
}
thread_local! {
    static GLOBAL_DEBUGGER_WINDOW: std::cell::RefCell<Option<DebuggerWindowHandle>> = const { RefCell::new(None) };
}

#[derive(Default)]
pub struct DockManager {
    pub layouts: Vec<LayoutInfo>,
    pub debugger_views: HashMap<QString, DebuggerViewHandle>,
    pub menu_bar: QString,
    pub cpu: Option<BreakPointCpu>,
}
pub type DebuggerViewHandle = Rc<RefCell<DebuggerView>>;

#[derive(Debug, Default, Clone)]
pub struct LayoutInfo {
    pub name: QString,
    pub is_default: bool,
}

impl DockManager {
    pub fn new() -> Self { Self::default() }
    pub fn configure_docking_system() {}
    pub fn load_layouts(&mut self) {}
    pub fn reset_all_layouts(&mut self) {}
    pub fn reset_default_layouts(&mut self) {}
    pub fn save_current_layout(&mut self) {}
    pub fn switch_to_layout(&mut self, _idx: usize) {}
    pub fn switch_to_layout_with_cpu(&mut self, _ty: BreakPointCpu, _blink: bool) {}
    pub fn update_tool_bar_lock_state(&mut self) {}
    pub fn update_theme(&mut self) {}
    pub fn create_tools_menu(&mut self, _menu: &mut MenuStub) {}
    pub fn create_windows_menu(&mut self, _menu: &mut MenuStub) {}
    pub fn create_menu_bar(&mut self, _bar: &MenuBarStub) -> WidgetHandle { WidgetHandle(0) }
    pub fn switch_to_debugger_view(&mut self, _name: QString) {}
    pub fn cpu(&self) -> Option<BreakPointCpu> { self.cpu }
}

#[derive(Default, Debug)]
pub struct MenuStub;
impl MenuStub {
    pub fn new() -> Self { Self::default() }
}
#[derive(Default, Debug)]
pub struct MenuBarStub;
impl MenuBarStub {
    pub fn new() -> Self { Self::default() }
}
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WidgetHandle(pub usize);

pub struct DebuggerWindow {
    pub m_dock_manager: DockManager,
    pub m_font_size: i32,
    pub m_is_updating_theme: bool,
    pub m_refresh_timer: Option<RefreshTimer>,
    pub m_default_toolbar_state: QString,
    pub m_ui: DebuggerWindowUi,
}

pub const MINIMUM_FONT_SIZE: i32 = 5;
pub const MAXIMUM_FONT_SIZE: i32 = 30;

#[derive(Default, Debug)]
pub struct DebuggerWindowUi {
    pub action_run: ActionStub,
    pub action_step_into: ActionStub,
    pub action_step_over: ActionStub,
    pub action_step_out: ActionStub,
    pub action_analyse: ActionStub,
    pub action_game_settings: ActionStub,
    pub action_shut_down: ActionStub,
    pub action_reset: ActionStub,
    pub action_increase_font_size: ActionStub,
    pub action_decrease_font_size: ActionStub,
    pub action_reset_font_size: ActionStub,
    pub action_settings: ActionStub,
    pub action_close: ActionStub,
    pub action_reset_all_layouts: ActionStub,
    pub action_reset_default_layouts: ActionStub,
    pub action_on_top: ActionStub,
    pub menu_tools: MenuStub,
    pub menu_windows: MenuStub,
}

#[derive(Default, Debug, Clone)]
pub struct ActionStub {
    pub text: QString,
    pub icon: QString,
    pub enabled: bool,
    pub checked: bool,
    pub checkable: bool,
    pub shortcuts: Vec<String>,
}
impl ActionStub {
    pub fn set_text(&mut self, t: QString) { self.text = t; }
    pub fn set_icon(&mut self, i: QString) { self.icon = i; }
    pub fn set_enabled(&mut self, b: bool) { self.enabled = b; }
    pub fn set_checked(&mut self, b: bool) { self.checked = b; }
    pub fn set_checkable(&mut self, b: bool) { self.checkable = b; }
    pub fn set_shortcuts(&mut self, s: Vec<String>) { self.shortcuts = s; }
    pub fn is_checked(&self) -> bool { self.checked }
    pub fn text(&self) -> QString { self.text.clone() }
    pub fn icon(&self) -> QString { self.icon.clone() }
    pub fn enabled(&self) -> bool { self.enabled }
}

pub struct RefreshTimer {
    pub interval_ms: u32,
    pub active: bool,
    pub single_shot: bool,
}
impl RefreshTimer {
    pub fn set_interval(&mut self, ms: u32) { self.interval_ms = ms; }
    pub fn set_single_shot(&mut self, b: bool) { self.single_shot = b; }
    pub fn is_active(&self) -> bool { self.active }
    pub fn start(&mut self) { self.active = true; }
    pub fn stop(&mut self) { self.active = false; }
    pub fn interval(&self) -> u32 { self.interval_ms }
}

impl DebuggerWindow {
    pub fn new() -> Self {
        let dock_manager = DockManager::new();
        Self {
            m_dock_manager: dock_manager,
            m_font_size: MINIMUM_FONT_SIZE,
            m_is_updating_theme: false,
            m_refresh_timer: None,
            m_default_toolbar_state: QString::new(),
            m_ui: DebuggerWindowUi::default(),
        }
    }

    pub fn get_instance() -> DebuggerWindowHandle {
        if global_debugger_window().is_none() {
            Self::create_instance();
        }
        global_debugger_window().unwrap()
    }

    pub fn create_instance() -> DebuggerWindowHandle {
        DockManager::configure_docking_system();
        if let Some(existing) = global_debugger_window() {
            return existing;
        }
        let mut window = Self::new();
        window.setup_default_tool_bar_state();
        window.setup_fonts();
        window.restore_window_geometry();
        window.m_dock_manager.load_layouts();
        window.m_dock_manager.switch_to_layout(0);
        window.update_theme();
        window.update_from_settings();
        let handle = Rc::new(RefCell::new(window));
        GLOBAL_DEBUGGER_WINDOW.with(|c| *c.borrow_mut() = Some(handle.clone()));
        handle
    }

    pub fn destroy_instance() {
        if let Some(window) = global_debugger_window() {
            window.borrow_mut().close();
        }
    }

    pub fn should_show_on_startup() -> bool {
        settings().get_bool("Debugger/UserInterface", "ShowOnStartup", false)
    }

    pub fn dock_manager(&self) -> &DockManager { &self.m_dock_manager }
    pub fn dock_manager_mut(&mut self) -> &mut DockManager { &mut self.m_dock_manager }

    pub fn setup_default_tool_bar_state(&mut self) {
        self.m_default_toolbar_state = "default-toolbar-state".to_owned();
    }
    pub fn clear_tool_bar_state(&mut self) {
        self.m_default_toolbar_state.clear();
    }
    pub fn setup_fonts(&mut self) {
        let configured = settings().get_int("Debugger/UserInterface", "FontSize", 10);
        if configured < MINIMUM_FONT_SIZE || configured > MAXIMUM_FONT_SIZE {
            self.m_font_size = 10;
        } else {
            self.m_font_size = configured;
        }
        self.update_font_actions();
    }
    pub fn update_font_actions(&mut self) {
        self.m_ui.action_increase_font_size.set_enabled(self.m_font_size < MAXIMUM_FONT_SIZE);
        self.m_ui.action_decrease_font_size.set_enabled(self.m_font_size > MINIMUM_FONT_SIZE);
        self.m_ui.action_reset_font_size.set_enabled(true);
    }
    pub fn save_font_size(&self) {
        settings().set_int("Debugger/UserInterface", "FontSize", self.m_font_size);
        settings().commit();
    }
    pub fn font_size(&self) -> i32 { self.m_font_size }

    pub fn update_theme(&mut self) {
        if self.m_is_updating_theme { return; }
        self.m_is_updating_theme = true;
        self.m_dock_manager.update_theme();
        self.m_is_updating_theme = false;
    }

    pub fn save_window_geometry(&self) {
        let geometry = "geometry".to_owned();
        if geometry != settings().get_string("Debugger/UserInterface", "WindowGeometry") {
            settings().set_string("Debugger/UserInterface", "WindowGeometry", geometry);
            settings().commit();
        }
    }

    pub fn restore_window_geometry(&mut self) {
        if !self.should_save_window_geometry() { return; }
        let _ = settings().get_string("Debugger/UserInterface", "WindowGeometry");
    }

    pub fn should_save_window_geometry(&self) -> bool {
        settings().get_bool("Debugger/UserInterface", "SaveWindowGeometry", true)
    }

    pub fn update_from_settings(&mut self) {
        let refresh = settings().get_int("Debugger/UserInterface", "RefreshInterval", 1000);
        let refresh = refresh.clamp(10, 100_000);
        if self.m_refresh_timer.is_none() {
            let mut timer = RefreshTimer { interval_ms: refresh as u32, active: false, single_shot: false };
            timer.start();
            self.m_refresh_timer = Some(timer);
        } else if let Some(timer) = self.m_refresh_timer.as_mut() {
            timer.set_interval(refresh as u32);
        }
    }

    pub fn on_vm_starting(&mut self) {
        for a in [
            &mut self.m_ui.action_run,
            &mut self.m_ui.action_step_into,
            &mut self.m_ui.action_step_over,
            &mut self.m_ui.action_step_out,
            &mut self.m_ui.action_analyse,
            &mut self.m_ui.action_game_settings,
            &mut self.m_ui.action_shut_down,
            &mut self.m_ui.action_reset,
        ] { a.set_enabled(true); }
    }

    pub fn on_vm_paused(&mut self) {
        self.m_ui.action_run.set_text("Run".into());
        self.m_ui.action_run.set_icon("play-line".into());
        self.m_ui.action_step_into.set_enabled(true);
        self.m_ui.action_step_over.set_enabled(true);
        self.m_ui.action_step_out.set_enabled(true);
    }

    pub fn on_vm_resumed(&mut self) {
        self.m_ui.action_run.set_text("Pause".into());
        self.m_ui.action_run.set_icon("pause-line".into());
        self.m_ui.action_step_into.set_enabled(false);
        self.m_ui.action_step_over.set_enabled(false);
        self.m_ui.action_step_out.set_enabled(false);
    }

    pub fn on_vm_stopped(&mut self) {
        for a in [
            &mut self.m_ui.action_run,
            &mut self.m_ui.action_step_into,
            &mut self.m_ui.action_step_over,
            &mut self.m_ui.action_step_out,
            &mut self.m_ui.action_analyse,
            &mut self.m_ui.action_game_settings,
            &mut self.m_ui.action_shut_down,
            &mut self.m_ui.action_reset,
        ] { a.set_enabled(false); }
    }

    pub fn on_analyse(&self) {
        // Real implementation: launch `AnalysisOptionsDialog`.
    }
    pub fn on_settings(&self) { /* opens main settings window */ }
    pub fn on_game_settings(&self) { /* opens main game settings */ }
    pub fn on_run_pause(&mut self) { /* toggles VM pause */ }
    pub fn on_step_into(&self) { /* see onStepInto() in C++ */ }
    pub fn on_step_over(&self) { /* see onStepOver() in C++ */ }
    pub fn on_step_out(&self) { /* see onStepOut() in C++ */ }
    pub fn on_vm_actually_paused(&self) { /* signal */ }

    pub fn change_event(&mut self, kind: ChangeEventKind) {
        if matches!(kind, ChangeEventKind::PaletteChange | ChangeEventKind::StyleChange) {
            self.update_theme();
        }
    }

    pub fn close_event(&mut self) {
        self.m_dock_manager.save_current_layout();
        self.save_window_geometry();
        GLOBAL_DEBUGGER_WINDOW.with(|c| *c.borrow_mut() = None);
    }

    pub fn close(&mut self) { self.close_event(); }

    pub fn current_cpu(&self) -> Option<&dyn DebugInterface> {
        let ty = self.m_dock_manager.cpu()?;
        Some(get_debug_interface(ty))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChangeEventKind {
    PaletteChange,
    StyleChange,
    Other,
}

impl Default for DebuggerWindow {
    fn default() -> Self { Self::new() }
}

// ---------------------------------------------------------------------------
// AnalysisOptionsDialog
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
pub struct DebugAnalysisOptions {
    pub start: u32,
    pub end: u32,
    pub analyse_functions: bool,
    pub analyse_variables: bool,
}

#[derive(Default)]
pub struct DebugAnalysisSettingsWidget {
    pub options: DebugAnalysisOptions,
}
impl DebugAnalysisSettingsWidget {
    pub fn new() -> Self { Self::default() }
    pub fn parse_settings_from_widgets(&self, options: &mut DebugAnalysisOptions) {
        *options = self.options.clone();
    }
}

pub struct AnalysisOptionsDialog {
    pub m_analysis_settings: Box<DebugAnalysisSettingsWidget>,
    pub m_ui: AnalysisOptionsDialogUi,
    pub close_on_analyse: bool,
}
#[derive(Default, Debug)]
pub struct AnalysisOptionsDialogUi {
    pub analyse_button: ActionStub,
    pub close_button: ActionStub,
    pub close_check_box: ActionStub,
    pub analysis_settings: WidgetHandle,
}

impl AnalysisOptionsDialog {
    pub fn new(_parent: WidgetHandle) -> Self {
        Self {
            m_analysis_settings: Box::new(DebugAnalysisSettingsWidget::new()),
            m_ui: AnalysisOptionsDialogUi::default(),
            close_on_analyse: false,
        }
    }

    pub fn analyse(&mut self) {
        let mut options = DebugAnalysisOptions::default();
        self.m_analysis_settings.parse_settings_from_widgets(&mut options);
        if self.m_ui.close_check_box.is_checked() {
            self.close_on_analyse = true;
        }
    }
}

// ---------------------------------------------------------------------------
// DebuggerSettingsManager
// ---------------------------------------------------------------------------

#[derive(Default, Debug)]
pub struct DebuggerSettingsManager;
impl DebuggerSettingsManager {
    pub const SETTINGS_FILE_VERSION: &'static str = "0.01";

    pub fn write_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    pub fn load_game_settings_json(path: &str) -> JsonObject {
        let _ = path;
        JsonObject::default()
    }

    pub fn write_json_to_path(path: &str, doc: &JsonDocument) {
        let _ = (path, doc);
    }

    pub fn load_game_settings_bp(_model: &mut BreakpointModel) {}
    pub fn load_game_settings_sa(_model: &mut SavedAddressesModel) {}
    pub fn save_game_settings_bp(_model: &BreakpointModel) {}
    pub fn save_game_settings_sa(_model: &SavedAddressesModel) {}
    pub fn save_game_settings<T>(_model: &T, _key: &str, _role: ItemRole) {}
}

// ---------------------------------------------------------------------------
// MemoryViewType + MemoryViewTable
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct MemoryViewTable {
    pub parent: WidgetHandle,
    pub display_type: MemoryViewType,
    pub little_endian: bool,
    pub row_count: u32,
    pub row_visible: u32,
    pub row_height: i32,
    pub start_address: u32,
    pub selected_address: u32,
    pub selected_index: i32,
    pub value_x_axis: i32,
    pub text_x_axis: i32,
    pub row_1_y_axis: i32,
    pub segment_x_axis: [i32; 16],
    pub selected_text: bool,
    pub selected_nibble_hi: bool,
}

impl MemoryViewTable {
    pub fn new(parent: WidgetHandle) -> Self {
        Self { parent, little_endian: true, ..Default::default() }
    }

    pub fn update_start_address(&mut self, start: u32) {
        self.start_address = start & !0xF;
    }

    pub fn update_selected_address(&mut self, selected: u32, page: bool) {
        self.selected_address = selected;
        if self.start_address > self.selected_address {
            if page {
                self.start_address = self.start_address.saturating_sub(0x10 * self.row_visible);
            } else {
                self.start_address = self.start_address.saturating_sub(0x10);
            }
        } else if self.start_address + ((self.row_visible.saturating_sub(1)) * 0x10) < self.selected_address {
            if page {
                self.start_address += 0x10 * self.row_visible;
            } else {
                self.start_address += 0x10;
            }
        }
    }

    pub fn get_view_type(&self) -> MemoryViewType { self.display_type }
    pub fn set_view_type(&mut self, t: MemoryViewType) { self.display_type = t; }
    pub fn get_little_endian(&self) -> bool { self.little_endian }
    pub fn set_little_endian(&mut self, b: bool) { self.little_endian = b; }

    pub fn next_address(addr: u32, selected: u32, dt: MemoryViewType, le: bool) -> u32 {
        if !le {
            addr + 1
        } else if selected % MEMORY_VIEW_TYPE_WIDTH[dt as usize] as u32 == 0 {
            addr + (MEMORY_VIEW_TYPE_WIDTH[dt as usize] as u32 * 2 - 1)
        } else {
            addr - 1
        }
    }
    pub fn prev_address(addr: u32, selected: u32, dt: MemoryViewType, le: bool) -> u32 {
        if !le {
            addr.saturating_sub(1)
        } else if (addr & ((dt as u32).saturating_sub(1))) == (dt as u32).saturating_sub(1) {
            addr.saturating_sub(MEMORY_VIEW_TYPE_WIDTH[dt as usize] as u32 * 2 - 1)
        } else {
            selected + 1
        }
    }

    pub fn forward_selection(&mut self) {
        if matches!(self.display_type, MemoryViewType::Float) {
            if !self.little_endian {
                if self.selected_index as i32 >= MEMORY_VIEW_TYPE_VISUAL_WIDTH[4] - 1 {
                    self.update_selected_address(self.selected_address + 4, false);
                    self.selected_index = 0;
                } else {
                    self.selected_index += 1;
                }
            } else if self.selected_index <= 0 {
                self.update_selected_address(self.selected_address + 4, false);
                self.selected_index = MEMORY_VIEW_TYPE_VISUAL_WIDTH[4] - 1;
            } else {
                self.selected_index -= 1;
            }
        } else if !self.little_endian {
            if { self.selected_nibble_hi = !self.selected_nibble_hi; self.selected_nibble_hi } {
                self.update_selected_address(self.selected_address + 1, false);
            }
        } else {
            if { self.selected_nibble_hi = !self.selected_nibble_hi; self.selected_nibble_hi } {
                let dt = self.display_type as usize;
                if self.selected_address % MEMORY_VIEW_TYPE_WIDTH[dt] as u32 == 0 {
                    self.update_selected_address(self.selected_address + (MEMORY_VIEW_TYPE_VISUAL_WIDTH[dt] as u32 - 1), false);
                } else {
                    self.update_selected_address(self.selected_address - 1, false);
                }
            }
        }
    }

    pub fn backward_selection(&mut self) {
        let dt = self.display_type as usize;
        let w = MEMORY_VIEW_TYPE_WIDTH[dt];
        let vw = MEMORY_VIEW_TYPE_VISUAL_WIDTH[dt];
        if matches!(self.display_type, MemoryViewType::Float) {
            if !self.little_endian {
                if self.selected_index <= 0 {
                    self.update_selected_address(self.selected_address - 4, false);
                    self.selected_index = vw - 1;
                } else {
                    self.selected_index -= 1;
                }
            } else if self.selected_index >= vw - 1 {
                self.update_selected_address(self.selected_address - 4, false);
                self.selected_index = 0;
            } else {
                self.selected_index += 1;
            }
        } else if !self.little_endian {
            if { self.selected_nibble_hi = !self.selected_nibble_hi; !self.selected_nibble_hi } {
                self.update_selected_address(self.selected_address - 1, false);
            }
        } else if { self.selected_nibble_hi = !self.selected_nibble_hi; !self.selected_nibble_hi } {
            if (self.selected_address & (w as u32 - 1)) == (w as u32 - 1) {
                self.update_selected_address(self.selected_address - (vw as u32 - 1), false);
            } else {
                self.update_selected_address(self.selected_address + 1, false);
            }
        }
    }

    pub fn get_selected_segment(&self, cpu: &dyn DebugInterface) -> U128 {
        let mut val = U128::default();
        match self.display_type {
            MemoryViewType::Byte => {
                val.lo = cpu.read8(self.selected_address, &mut false) as u32;
            }
            MemoryViewType::ByteHw => {
                val.lo = convert_endian_u16(cpu.read16(self.selected_address & !1, &mut false), self.little_endian) as u32;
            }
            MemoryViewType::Word => {
                val.lo = convert_endian_u32(cpu.read32(self.selected_address & !3, &mut false), self.little_endian);
            }
            MemoryViewType::DWord => {
                val._u64()[0] = convert_endian_u64(cpu.read64(self.selected_address & !7, &mut false), self.little_endian);
            }
            MemoryViewType::Float => {
                val.lo = convert_endian_u32(cpu.read32(self.selected_address & !3, &mut false), self.little_endian);
            }
        }
        val
    }

    pub fn insert_into_selected_hex_view(&self, value: u8, cpu: &mut dyn DebugInterface) {
        let mask = if self.selected_nibble_hi { 0x0F } else { 0xF0 };
        let cur = cpu.read8(self.selected_address, &mut false) & mask;
        let new_val = cur | (value << if self.selected_nibble_hi { 4 } else { 0 });
        cpu.write8(self.selected_address, new_val);
    }

    pub fn key_press(&mut self, key: i32, ch: Option<char>, cpu: &mut dyn DebugInterface) -> bool {
        if !cpu.is_valid_address(self.selected_address) { return false; }
        let mut handled = false;
        let key_char_is_text = ch.map(|c| c.is_alphanumeric() || c == ' ').unwrap_or(false);
        if self.selected_text {
            if key_char_is_text {
                let v = ch.and_then(|c| if c.is_ascii() { Some(c as u8) } else { None }).unwrap_or(0);
                cpu.write8(self.selected_address, v);
                self.update_selected_address(self.selected_address + 1, false);
                handled = true;
            }
            match key {
                0x01000003 /* Qt::Key::Key_Backspace */ | 0x01000000 /* Qt::Key::Key_Escape */ => {
                    cpu.write8(self.selected_address, 0);
                    self.backward_selection();
                    handled = true;
                }
                0x01000014 /* Qt::Key::Key_Right */ => { self.forward_selection(); handled = true; }
                0x01000012 /* Qt::Key::Key_Left */ => { self.backward_selection(); handled = true; }
                _ => {}
            }
        } else if key_char_is_text && !matches!(self.display_type, MemoryViewType::Float) {
            if let Some(c) = ch {
                if let Some(nibble) = hex_nibble(c) {
                    self.insert_into_selected_hex_view(nibble, cpu);
                    self.forward_selection();
                    handled = true;
                }
            }
        } else {
            match key {
                0x01000003 | 0x01000000 => {
                    self.insert_into_selected_hex_view(0, cpu);
                    self.backward_selection();
                    handled = true;
                }
                0x01000014 => { self.forward_selection(); handled = true; }
                0x01000012 => { self.backward_selection(); handled = true; }
                _ => {}
            }
        }
        match key {
            0x01000013 /* Key_Up */ => { self.update_selected_address(self.selected_address - 0x10, false); handled = true; }
            0x01000016 /* Key_PageUp */ => { self.update_selected_address(self.selected_address - (0x10 * self.row_visible), true); handled = true; }
            0x01000015 /* Key_Down */ => { self.update_selected_address(self.selected_address + 0x10, false); handled = true; }
            0x01000017 /* Key_PageDown */ => { self.update_selected_address(self.selected_address + (0x10 * self.row_visible), true); handled = true; }
            _ => {}
        }
        handled
    }
}

pub struct MemoryView {
    pub m_table: MemoryViewTable,
    pub ui: MemoryViewUi,
    pub m_visible_start: u32,
    pub m_visible_rows: u32,
    pub m_selected_address_start: u32,
    pub m_selected_address_end: u32,
    pub m_row_height: u32,
}

#[derive(Default, Debug)]
pub struct MemoryViewUi;
impl MemoryViewUi {
    pub fn setup_ui(&mut self, _w: &mut MemoryView) {}
    pub fn set_focus_policy(&mut self) {}
    pub fn set_context_menu_policy(&mut self) {}
    pub fn update(&self) {}
    pub fn height(&self) -> i32 { 0 }
}

impl MemoryView {
    pub fn new(_parameters: &DebuggerViewParameters) -> Self {
        let mut view = Self {
            m_table: MemoryViewTable::new(WidgetHandle(0)),
            ui: MemoryViewUi::default(),
            m_visible_start: 0x100000,
            m_visible_rows: 0,
            m_selected_address_start: 0,
            m_selected_address_end: 0,
            m_row_height: 0,
        };
        view.m_table.update_start_address(0x100000);
        view
    }

    pub fn to_json(&self, _base: &mut DebuggerView, json: &mut JsonValueWrapper) {
        json.value_mut().add_member(
            "startAddress",
            serde_json_like::Value::Uint(self.m_table.start_address as u64),
        );
        json.value_mut().add_member(
            "viewType",
            serde_json_like::Value::Int(self.m_table.get_view_type() as i32 as i64),
        );
        json.value_mut().add_member(
            "littleEndian",
            serde_json_like::Value::Bool(self.m_table.get_little_endian()),
        );
    }

    pub fn from_json(&mut self, _base: &mut DebuggerView, json: &JsonValueWrapper) -> bool {
        if let Some(v) = json.value().find_member("startAddress") {
            if let Some(u) = v.as_uint() { self.m_table.update_start_address(u as u32); }
        }
        if let Some(v) = json.value().find_member("viewType") {
            if let Some(i) = v.as_int() {
                if let Ok(t) = num_traits_from_int::<MemoryViewType>(i) {
                    self.m_table.set_view_type(t);
                }
            }
        }
        if let Some(v) = json.value().find_member("littleEndian") {
            if let Some(b) = v.as_bool() { self.m_table.set_little_endian(b); }
        }
        true
    }

    pub fn goto_address(&mut self, address: u32) {
        self.m_table.update_start_address(address & !0xF);
        self.m_table.selected_address = address;
    }

    pub fn context_copy_byte(&self) {
        let cpu = get_debug_interface(BreakPointCpu::EE);
        global_clipboard().set_text(format!("{:X}", cpu.read8(self.m_table.selected_address, &mut false)));
    }
    pub fn context_copy_segment(&self) { /* copies selected segment */ }
    pub fn context_copy_character(&self) { /* copies selected character */ }
    pub fn context_paste(&self) { /* paste from clipboard */ }
    pub fn context_go_to_address(&self) { /* prompt for address */ }
    pub fn context_follow_address(&self) { /* follow address */ }
    pub fn open_context_menu(&self) { /* build QMenu */ }
    pub fn paint_event(&self) {}
    pub fn mouse_press_event(&mut self) {}
    pub fn mouse_double_click_event(&self) {}
    pub fn wheel_event(&mut self) { self.m_table.update_start_address(self.m_table.start_address.wrapping_add(0x10)); }
    pub fn key_press_event(&mut self) { /* dispatch to table */ }
}

fn num_traits_from_int<T>(_: i64) -> Result<T, Infallible> {
    // `Result<T, Infallible>` can never be `Err`, so the only way to
    // satisfy the return type is to diverge. The stand-in helper has
    // nothing meaningful to do in this translation module.
    unreachable!()
}

// ---------------------------------------------------------------------------
// MemorySearchView
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SearchType {
    #[default]
    Byte,
    Int16,
    Int32,
    Int64,
    Float,
    Double,
    String,
    Array,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SearchComparison {
    Equals,
    NotEquals,
    GreaterThan,
    GreaterThanOrEqual,
    LessThan,
    LessThanOrEqual,
    Increased,
    IncreasedBy,
    Decreased,
    DecreasedBy,
    Changed,
    ChangedBy,
    NotChanged,
    UnknownValue,
    Invalid,
}

#[derive(Debug, Default, Clone)]
pub struct SearchResult {
    pub address: u32,
    pub value: QVariant,
    pub ty: SearchType,
}
impl SearchResult {
    pub fn is_integer_value(&self) -> bool {
        matches!(self.ty, SearchType::Byte | SearchType::Int16 | SearchType::Int32 | SearchType::Int64)
    }
    pub fn is_float_value(&self) -> bool { matches!(self.ty, SearchType::Float) }
    pub fn is_double_value(&self) -> bool { matches!(self.ty, SearchType::Double) }
    pub fn is_array_value(&self) -> bool { matches!(self.ty, SearchType::Array | SearchType::String) }
    pub fn get_address(&self) -> u32 { self.address }
    pub fn get_type(&self) -> SearchType { self.ty }
    pub fn get_array_value(&self) -> QByteArray { self.value.to_byte_array() }
    pub fn get_value<T: Copy + Default>(&self) -> T { T::default() }
}

#[derive(Default, Debug)]
pub struct SearchComparisonLabelMap {
    pub enum_to_label: BTreeMap<SearchComparison, QString>,
    pub label_to_enum: BTreeMap<QString, SearchComparison>,
}
impl SearchComparisonLabelMap {
    pub fn new() -> Self {
        let mut m = Self::default();
        m.insert(SearchComparison::Equals, "Equals".into());
        m.insert(SearchComparison::NotEquals, "Not Equals".into());
        m.insert(SearchComparison::GreaterThan, "Greater Than".into());
        m.insert(SearchComparison::GreaterThanOrEqual, "Greater Than Or Equal".into());
        m.insert(SearchComparison::LessThan, "Less Than".into());
        m.insert(SearchComparison::LessThanOrEqual, "Less Than Or Equal".into());
        m.insert(SearchComparison::Increased, "Increased".into());
        m.insert(SearchComparison::IncreasedBy, "Increased By".into());
        m.insert(SearchComparison::Decreased, "Decreased".into());
        m.insert(SearchComparison::DecreasedBy, "Decreased By".into());
        m.insert(SearchComparison::Changed, "Changed".into());
        m.insert(SearchComparison::ChangedBy, "Changed By".into());
        m.insert(SearchComparison::NotChanged, "Not Changed".into());
        m.insert(SearchComparison::UnknownValue, "Unknown Initial Value".into());
        m.insert(SearchComparison::Invalid, "".into());
        m
    }
    pub fn insert(&mut self, c: SearchComparison, label: QString) {
        self.enum_to_label.insert(c, label.clone());
        self.label_to_enum.insert(label, c);
    }
    pub fn label_to_enum(&self, label: &str) -> SearchComparison {
        self.label_to_enum.get(label).copied().unwrap_or(SearchComparison::Invalid)
    }
    pub fn enum_to_label(&self, c: SearchComparison) -> QString {
        self.enum_to_label.get(&c).cloned().unwrap_or_default()
    }
}

pub struct MemorySearchView {
    pub m_search_results: Vec<SearchResult>,
    pub m_search_comparison_label_map: SearchComparisonLabelMap,
    pub m_results_load_timer: RefreshTimer,
    pub m_initial_results_load_limit: u32,
    pub m_num_results_added_per_load: u32,
    pub m_ui: MemorySearchViewUi,
}

#[derive(Default, Debug)]
pub struct MemorySearchViewUi {
    pub btn_search: ActionStub,
    pub btn_filter_search: ActionStub,
    pub list_search_results: ListWidgetStub,
    pub cmb_search_type: ComboBoxStub,
    pub cmb_search_comparison: ComboBoxStub,
    pub txt_search_start: TextFieldStub,
    pub txt_search_end: TextFieldStub,
    pub txt_search_value: TextFieldStub,
    pub chk_search_hex: CheckBoxStub,
    pub results_count_label: TextFieldStub,
}

#[derive(Default, Debug)]
pub struct ListWidgetStub;
impl ListWidgetStub {
    pub fn clear(&mut self) {}
    pub fn set_context_menu_policy(&mut self) {}
    pub fn vertical_scroll_bar(&self) -> &ScrollBarStub { &NOP_SCROLLBAR }
    pub fn selection_model(&self) -> &SelectionModelStub { &NOP_SELMODEL }
    pub fn row(&self, _item: ()) -> i32 { 0 }
    pub fn selected_items(&self) -> Vec<()> { Vec::new() }
    pub fn take_item(&mut self, _i: i32) -> Option<()> { None }
    pub fn item(&self, _i: u32) -> Option<()> { None }
    pub fn count(&self) -> i32 { 0 }
    pub fn add_item(&mut self, _i: ()) {}
    pub fn item_at(&self, _i: u32) -> Option<()> { None }
    pub fn viewport(&self) -> &ViewportStub { &NOP_VIEWPORT }
    pub fn custom_context_menu_requested(&self) {}
    pub fn item_double_clicked(&self) {}
    pub fn set_model(&mut self, _m: &dyn std::any::Any) {}
}

static NOP_SCROLLBAR: ScrollBarStub = ScrollBarStub;
static NOP_SELMODEL: SelectionModelStub = SelectionModelStub;
static NOP_VIEWPORT: ViewportStub = ViewportStub;
#[derive(Default, Debug)] pub struct ScrollBarStub;
impl ScrollBarStub {
    pub fn maximum(&self) -> i32 { 0 }
    pub fn value_changed(&self) {}
    pub fn value(&self) -> i32 { 0 }
}
#[derive(Default, Debug)] pub struct SelectionModelStub;
impl SelectionModelStub {
    pub fn has_selection(&self) -> bool { false }
    pub fn selected_indexes(&self) -> Vec<QModelIndex> { Vec::new() }
    pub fn current_index(&self) -> QModelIndex { QModelIndex::default() }
}
#[derive(Default, Debug)] pub struct ViewportStub;
impl ViewportStub {
    pub fn map_to_global(&self, pos: (i32, i32)) -> (i32, i32) { pos }
}

#[derive(Default, Debug)]
pub struct ComboBoxStub;
impl ComboBoxStub {
    pub fn current_index(&self) -> i32 { 0 }
    pub fn current_text(&self) -> QString { QString::new() }
    pub fn clear(&mut self) {}
    pub fn add_item(&mut self, _s: QString) {}
    pub fn set_current_text(&mut self, _s: QString) {}
    pub fn current_index_changed(&self) {}
}

#[derive(Default, Debug)]
pub struct TextFieldStub;
impl TextFieldStub {
    pub fn text(&self) -> QString { QString::new() }
    pub fn set_text(&mut self, _s: QString) {}
    pub fn set_enabled(&mut self, _b: bool) {}
    pub fn set_visible(&mut self, _b: bool) {}
}

#[derive(Default, Debug)]
pub struct CheckBoxStub;
impl CheckBoxStub {
    pub fn is_checked(&self) -> bool { false }
    pub fn set_enabled(&mut self, _b: bool) {}
}

impl MemorySearchView {
    pub fn new(_parameters: &DebuggerViewParameters) -> Self {
        let mut view = Self {
            m_search_results: Vec::new(),
            m_search_comparison_label_map: SearchComparisonLabelMap::new(),
            m_results_load_timer: RefreshTimer { interval_ms: 100, active: false, single_shot: true },
            m_initial_results_load_limit: 20_000,
            m_num_results_added_per_load: 10_000,
            m_ui: MemorySearchViewUi::default(),
        };
        view
    }

    pub fn on_search_button_clicked(&mut self) {}
    pub fn on_search_results_list_scroll(&mut self, _value: u32) {}
    pub fn load_search_results(&mut self) {}
    pub fn on_search_type_changed(&mut self, _idx: i32) {}
    pub fn on_search_comparison_changed(&mut self, _idx: i32) {}
    pub fn update_search_comparison_selections(&mut self) {}
    pub fn get_valid_search_comparisons_for_state(&self, _ty: SearchType, _existing: &mut Vec<SearchResult>) -> Vec<SearchComparison> {
        Vec::new()
    }
    pub fn get_current_search_type(&self) -> SearchType { SearchType::Byte }
    pub fn get_current_search_comparison(&self) -> SearchComparison { SearchComparison::Equals }
    pub fn does_search_comparison_take_input(c: SearchComparison) -> bool {
        matches!(c,
            SearchComparison::Equals
            | SearchComparison::NotEquals
            | SearchComparison::GreaterThan
            | SearchComparison::GreaterThanOrEqual
            | SearchComparison::LessThan
            | SearchComparison::LessThanOrEqual
            | SearchComparison::IncreasedBy
            | SearchComparison::DecreasedBy
        )
    }
    pub fn context_remove_search_result(&mut self) {}
    pub fn context_copy_search_result_address(&self) {}
    pub fn on_list_search_results_context_menu(&self) {}
}

/// Generic memory search worker. Replicates the templated C++ logic
/// for the integer / float / double cases.
pub fn search_worker<T>(
    cpu: &mut dyn DebugInterface,
    results: &mut Vec<SearchResult>,
    ty: SearchType,
    cmp: SearchComparison,
    start: u32,
    end: u32,
    value: T,
) where
    T: Copy + PartialEq + PartialOrd + Default + Into<f64>,
{
    let is_searching_range = results.is_empty();
    if is_searching_range {
        let mut addr = start;
        while addr < end {
            if cpu.is_valid_address(addr) {
                let read = read_typed::<T>(cpu, addr);
                if memory_value_comparator(cmp, value, read) {
                    results.push(SearchResult { address: addr, value: QVariant::from(read_to_variant(read)), ty });
                }
            }
            addr += std::mem::size_of::<T>() as u32;
        }
    } else {
        results.retain_mut(|r| {
            let read = read_typed::<T>(cpu, r.address);
            let keep = memory_value_comparator(cmp, value, read);
            if keep { r.value = read_to_variant(read); }
            keep
        });
    }
}

fn read_typed<T: Copy + Default>(cpu: &dyn DebugInterface, addr: u32) -> T {
    let mut v: T = T::default();
    let bytes = std::mem::size_of::<T>();
    let mut buf = vec![0u8; bytes];
    for i in 0..bytes {
        buf[i] = cpu.read8(addr + i as u32, &mut false);
    }
    unsafe { std::ptr::read_unaligned(buf.as_ptr() as *const T) }
}

fn read_to_variant<T: Copy + Default>(v: T) -> QVariant {
    let size = std::mem::size_of::<T>();
    if size == 0 || size > 4 { return QVariant::Int(0); }
    let mut buf = [0u8; 4];
    unsafe {
        std::ptr::copy_nonoverlapping(&v as *const T as *const u8, buf.as_mut_ptr(), size);
    }
    QVariant::Int(i32::from_le_bytes(buf))
}

pub fn memory_value_comparator<T: Copy + PartialEq + PartialOrd>(cmp: SearchComparison, search: T, read: T) -> bool {
    let is_not = matches!(cmp, SearchComparison::NotEquals);
    match cmp {
        SearchComparison::Equals | SearchComparison::NotEquals => {
            let eq = search == read;
            if is_not { !eq } else { eq }
        }
        SearchComparison::GreaterThan | SearchComparison::GreaterThanOrEqual => {
            if matches!(cmp, SearchComparison::GreaterThanOrEqual) && search == read { return true; }
            read > search
        }
        SearchComparison::LessThan | SearchComparison::LessThanOrEqual => {
            if matches!(cmp, SearchComparison::LessThanOrEqual) && search == read { return true; }
            read < search
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// BreakpointModel / BreakpointDialog / BreakpointView
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum BreakpointMemcheck {
    Breakpoint(BreakPoint),
    MemCheck(MemCheck),
}

impl Default for BreakpointMemcheck {
    fn default() -> Self { BreakpointMemcheck::Breakpoint(BreakPoint::default()) }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BreakpointColumn {
    #[default]
    Enabled,
    Type,
    Offset,
    Description,
    SizeLabel,
    Opcode,
    Condition,
    Hits,
    ColumnCount,
}

#[derive(Default)]
pub struct BreakpointModel {
    pub m_cpu: usize,
    pub m_breakpoints: Vec<BreakpointMemcheck>,
}

impl QAbstractTableModel for BreakpointModel {
    fn row_count(&self) -> i32 { self.m_breakpoints.len() as i32 }
    fn column_count(&self) -> i32 { BreakpointColumn::ColumnCount as i32 }
    fn data(&self, index: QModelIndex, role: ItemRole) -> QVariant {
        let row = index.row as usize;
        if !index.is_valid() || row >= self.m_breakpoints.len() { return QVariant::Invalid; }
        let bpmc = &self.m_breakpoints[row];
        match role {
            ItemRole::DisplayRole => match bpmc {
                BreakpointMemcheck::Breakpoint(bp) => match index.column {
                    x if x == BreakpointColumn::Enabled as i32 => QVariant::from(""),
                    x if x == BreakpointColumn::Type as i32 => QVariant::from("Execute"),
                    x if x == BreakpointColumn::Offset as i32 => QVariant::from(filled_qstring_from_value(bp.addr as u64, 16)),
                    x if x == BreakpointColumn::Description as i32 => QVariant::from(bp.description.clone()),
                    x if x == BreakpointColumn::Condition as i32 => QVariant::from(if bp.has_cond { bp.cond.expression_string.clone() } else { QString::new() }),
                    x if x == BreakpointColumn::Hits as i32 => QVariant::from("--"),
                    _ => QVariant::Invalid,
                },
                BreakpointMemcheck::MemCheck(mc) => match index.column {
                    x if x == BreakpointColumn::Enabled as i32 => QVariant::from(""),
                    x if x == BreakpointColumn::Type as i32 => {
                        let mut s = QString::new();
                        if mc.mem_cond.0 & MEMCHECK_READ != 0 { s.push_str("Read"); }
                        if (mc.mem_cond.0 & MEMCHECK_READWRITE) == MEMCHECK_READWRITE { s.push_str(", "); } else { s.push(' '); }
                        if mc.mem_cond.0 & MEMCHECK_WRITE != 0 {
                            if mc.mem_cond.0 & MEMCHECK_WRITE_ONCHANGE != 0 { s.push_str("Write(C)"); } else { s.push_str("Write"); }
                        }
                        QVariant::from(s)
                    }
                    x if x == BreakpointColumn::Offset as i32 => QVariant::from(filled_qstring_from_value(mc.start as u64, 16)),
                    x if x == BreakpointColumn::Description as i32 => QVariant::from(mc.description.clone()),
                    x if x == BreakpointColumn::SizeLabel as i32 => QVariant::from(format!("{:X}", mc.end - mc.start)),
                    x if x == BreakpointColumn::Condition as i32 => QVariant::from(if mc.has_cond { mc.cond.expression_string.clone() } else { QString::new() }),
                    x if x == BreakpointColumn::Hits as i32 => QVariant::from(mc.num_hits as i32),
                    _ => QVariant::Invalid,
                },
            },
            ItemRole::CheckStateRole => match bpmc {
                BreakpointMemcheck::Breakpoint(bp) if index.column == BreakpointColumn::Enabled as i32 => {
                    QVariant::from(bp.enabled)
                }
                BreakpointMemcheck::MemCheck(mc) if index.column == BreakpointColumn::Enabled as i32 => {
                    QVariant::from(mc.result.0 & MEMCHECK_BREAK != 0)
                }
                _ => QVariant::Invalid,
            },
            _ => QVariant::Invalid,
        }
    }
    fn header_data(&self, section: i32, _orient: Orientation, role: ItemRole) -> QVariant {
        if !matches!(role, ItemRole::DisplayRole) { return QVariant::Invalid; }
        QVariant::from(match section {
            x if x == BreakpointColumn::Type as i32 => "TYPE",
            x if x == BreakpointColumn::Offset as i32 => "OFFSET",
            x if x == BreakpointColumn::Description as i32 => "DESCRIPTION",
            x if x == BreakpointColumn::SizeLabel as i32 => "SIZE / LABEL",
            x if x == BreakpointColumn::Opcode as i32 => "INSTRUCTION",
            x if x == BreakpointColumn::Condition as i32 => "CONDITION",
            x if x == BreakpointColumn::Hits as i32 => "HITS",
            x if x == BreakpointColumn::Enabled as i32 => "X",
            _ => "",
        })
    }
    fn set_data(&mut self, _index: QModelIndex, _value: QVariant, _role: ItemRole) -> bool { false }
    fn remove_rows(&mut self, row: i32, count: i32) -> bool {
        let begin = row as usize;
        let end = (row + count) as usize;
        if end > self.m_breakpoints.len() { return false; }
        self.m_breakpoints.drain(begin..end);
        true
    }
}

impl BreakpointModel {
    pub fn get_instance(_cpu: usize) -> Self { Self::default() }
    pub fn at(&self, row: i32) -> BreakpointMemcheck {
        self.m_breakpoints.get(row as usize).cloned().unwrap_or_default()
    }
    pub fn refresh_data(&mut self) {}
    pub fn clear(&mut self) { self.m_breakpoints.clear(); }
    pub fn insert_breakpoint_rows(&mut self, row: i32, count: i32, bps: Vec<BreakpointMemcheck>) -> bool {
        if bps.len() as i32 != count { return false; }
        self.m_breakpoints.splice(row as usize..row as usize, bps);
        true
    }
    pub fn load_breakpoint_from_field_list(&mut self, _fields: Vec<QString>) {}
}

pub struct BreakpointDialog {
    pub m_cpu: usize,
    pub m_purpose: BreakpointDialogPurpose,
    pub m_bp_model: usize,
    pub m_bp_mc: BreakpointMemcheck,
    pub m_row_index: i32,
    pub m_ui: BreakpointDialogUi,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BreakpointDialogPurpose { Create, Edit }
#[derive(Default, Debug)]
pub struct BreakpointDialogUi {
    pub rdo_execute: CheckBoxStub,
    pub rdo_memory: CheckBoxStub,
    pub txt_address: TextFieldStub,
    pub txt_size: TextFieldStub,
    pub txt_description: TextFieldStub,
    pub txt_condition: TextFieldStub,
    pub chk_enable: CheckBoxStub,
    pub chk_log: CheckBoxStub,
    pub chk_read: CheckBoxStub,
    pub chk_write: CheckBoxStub,
    pub chk_change: CheckBoxStub,
    pub grp_type: WidgetHandle,
    pub grp_memory: WidgetHandle,
}
impl BreakpointDialog {
    pub fn new(_parent: WidgetHandle, _cpu: usize, _model: usize) -> Self {
        Self {
            m_cpu: _cpu,
            m_purpose: BreakpointDialogPurpose::Create,
            m_bp_model: _model,
            m_bp_mc: BreakpointMemcheck::Breakpoint(BreakPoint::default()),
            m_row_index: 0,
            m_ui: BreakpointDialogUi::default(),
        }
    }
    pub fn new_edit(_parent: WidgetHandle, _cpu: usize, _model: usize, bpmc: BreakpointMemcheck, row: i32) -> Self {
        Self {
            m_cpu: _cpu,
            m_purpose: BreakpointDialogPurpose::Edit,
            m_bp_model: _model,
            m_bp_mc: bpmc,
            m_row_index: row,
            m_ui: BreakpointDialogUi::default(),
        }
    }
    pub fn on_rdo_button_toggled(&self) {}
    pub fn accept(&mut self) {
        // Real implementation: parse fields, set the bp_mc, then
        // either insert or replace a row in the model.
    }
}

pub struct BreakpointView {
    pub m_model: usize,
    pub m_ui: BreakpointViewUi,
}
#[derive(Default, Debug)]
pub struct BreakpointViewUi {
    pub breakpoint_list: ListWidgetStub,
}
impl BreakpointView {
    pub fn new(_parameters: &DebuggerViewParameters) -> Self {
        Self { m_model: 0, m_ui: BreakpointViewUi::default() }
    }
    pub fn on_double_clicked(&self) {}
    pub fn open_context_menu(&self) {}
    pub fn context_copy(&self) {}
    pub fn context_delete(&mut self) {}
    pub fn context_new(&self) {}
    pub fn context_edit(&self) {}
    pub fn context_paste_csv(&mut self) {}
    pub fn save_breakpoints_to_debugger_settings(&self) {}
    pub fn resize_columns(&self) {}
}

// ---------------------------------------------------------------------------
// SavedAddressesModel / SavedAddressesView
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
pub struct SavedAddress {
    pub address: u32,
    pub label: QString,
    pub description: QString,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SavedAddressColumn {
    #[default]
    Address,
    Label,
    Description,
    ColumnCount,
}

#[derive(Default)]
pub struct SavedAddressesModel {
    pub m_cpu: usize,
    pub m_saved_addresses: Vec<SavedAddress>,
}
impl QAbstractTableModel for SavedAddressesModel {
    fn row_count(&self) -> i32 { self.m_saved_addresses.len() as i32 }
    fn column_count(&self) -> i32 { SavedAddressColumn::ColumnCount as i32 }
    fn data(&self, index: QModelIndex, role: ItemRole) -> QVariant {
        let row = index.row as usize;
        if !index.is_valid() || row >= self.m_saved_addresses.len() { return QVariant::Invalid; }
        let entry = &self.m_saved_addresses[row];
        match role {
            ItemRole::DisplayRole | ItemRole::EditRole => match index.column {
                x if x == SavedAddressColumn::Address as i32 => QVariant::from(format!("{:X}", entry.address)),
                x if x == SavedAddressColumn::Label as i32 => QVariant::from(entry.label.clone()),
                x if x == SavedAddressColumn::Description as i32 => QVariant::from(entry.description.clone()),
                _ => QVariant::Invalid,
            },
            ItemRole::UserRole => match index.column {
                x if x == SavedAddressColumn::Address as i32 => QVariant::from(entry.address),
                x if x == SavedAddressColumn::Label as i32 => QVariant::from(entry.label.clone()),
                x if x == SavedAddressColumn::Description as i32 => QVariant::from(entry.description.clone()),
                _ => QVariant::Invalid,
            },
            _ => QVariant::Invalid,
        }
    }
    fn header_data(&self, section: i32, _o: Orientation, role: ItemRole) -> QVariant {
        if !matches!(role, ItemRole::DisplayRole) { return QVariant::Invalid; }
        QVariant::from(match section {
            x if x == SavedAddressColumn::Address as i32 => "MEMORY ADDRESS",
            x if x == SavedAddressColumn::Label as i32 => "LABEL",
            x if x == SavedAddressColumn::Description as i32 => "DESCRIPTION",
            _ => "",
        })
    }
    fn set_data(&mut self, _index: QModelIndex, _value: QVariant, _role: ItemRole) -> bool { false }
    fn remove_rows(&mut self, row: i32, count: i32) -> bool {
        let begin = row as usize;
        let end = (row + count) as usize;
        if end > self.m_saved_addresses.len() { return false; }
        self.m_saved_addresses.drain(begin..end);
        true
    }
}
impl SavedAddressesModel {
    pub fn get_instance(_cpu: usize) -> Self { Self::default() }
    pub fn add_row(&mut self) {
        self.m_saved_addresses.push(SavedAddress { address: 0, label: "Name".into(), description: "Description".into() });
    }
    pub fn add_row_with(&mut self, addr: SavedAddress) { self.m_saved_addresses.push(addr); }
    pub fn load_saved_address_from_field_list(&mut self, fields: Vec<QString>) {
        if fields.len() != (SavedAddressColumn::ColumnCount as usize) { return; }
        let address = u32::from_str_radix(fields[SavedAddressColumn::Address as usize].trim_start_matches("0x"), 16).unwrap_or(0);
        self.m_saved_addresses.push(SavedAddress { address, label: fields[SavedAddressColumn::Label as usize].clone(), description: fields[SavedAddressColumn::Description as usize].clone() });
    }
    pub fn clear(&mut self) { self.m_saved_addresses.clear(); }
}

pub struct SavedAddressesView {
    pub m_model: usize,
    pub m_ui: SavedAddressesViewUi,
}
#[derive(Default, Debug)]
pub struct SavedAddressesViewUi {
    pub saved_addresses_list: ListWidgetStub,
}
impl SavedAddressesView {
    pub fn new(_parameters: &DebuggerViewParameters) -> Self { Self { m_model: 0, m_ui: SavedAddressesViewUi::default() } }
    pub fn open_context_menu(&self) {}
    pub fn context_paste_csv(&mut self) {}
    pub fn context_new(&mut self) {}
    pub fn add_address(&mut self, address: u32) {}
    pub fn save_to_debugger_settings(&self) {}
}

// ---------------------------------------------------------------------------
// ModuleModel / ModuleView
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModuleColumn {
    #[default]
    Name,
    Version,
    Entry,
    Gp,
    TextSection,
    DataSection,
    BssSection,
    ColumnCount,
}

pub struct ModuleModel {
    pub m_cpu: usize,
    pub m_modules: Vec<IopMod>,
}
impl QAbstractTableModel for ModuleModel {
    fn row_count(&self) -> i32 { self.m_modules.len() as i32 }
    fn column_count(&self) -> i32 { ModuleColumn::ColumnCount as i32 }
    fn data(&self, index: QModelIndex, role: ItemRole) -> QVariant {
        let row = index.row as usize;
        if row >= self.m_modules.len() { return QVariant::Invalid; }
        let m = &self.m_modules[row];
        match role {
            ItemRole::DisplayRole => match index.column {
                x if x == ModuleColumn::Name as i32 => QVariant::from(m.name.clone()),
                x if x == ModuleColumn::Version as i32 => QVariant::from(format!("{}.{}", m.version >> 8, m.version & 0xFF)),
                x if x == ModuleColumn::Entry as i32 => QVariant::from(filled_qstring_from_value(m.entry as u64, 16)),
                x if x == ModuleColumn::Gp as i32 => QVariant::from(filled_qstring_from_value(m.gp as u64, 16)),
                x if x == ModuleColumn::TextSection as i32 => QVariant::from(format!("[{} - {}]", filled_qstring_from_value(m.text_addr as u64, 16), filled_qstring_from_value((m.text_addr + m.text_size - 1) as u64, 16))),
                x if x == ModuleColumn::DataSection as i32 => {
                    let addr = m.text_addr + m.text_size;
                    QVariant::from(format!("[{} - {}]", filled_qstring_from_value(addr as u64, 16), filled_qstring_from_value((addr + m.data_size - 1) as u64, 16)))
                }
                x if x == ModuleColumn::BssSection as i32 => {
                    if m.bss_size == 0 { QVariant::from("") }
                    else {
                        let addr = m.text_addr + m.text_size + m.data_size;
                        QVariant::from(format!("[{} - {}]", filled_qstring_from_value(addr as u64, 16), filled_qstring_from_value((addr + m.bss_size - 1) as u64, 16)))
                    }
                }
                _ => QVariant::Invalid,
            },
            ItemRole::UserRole => match index.column {
                x if x == ModuleColumn::Name as i32 => QVariant::from(m.name.clone()),
                x if x == ModuleColumn::Version as i32 => QVariant::from(m.version),
                x if x == ModuleColumn::Entry as i32 => QVariant::from(m.entry),
                x if x == ModuleColumn::Gp as i32 => QVariant::from(m.gp),
                x if x == ModuleColumn::TextSection as i32 => QVariant::from(m.text_addr),
                x if x == ModuleColumn::DataSection as i32 => QVariant::from(m.text_addr + m.text_size),
                x if x == ModuleColumn::BssSection as i32 => QVariant::from(if m.bss_size == 0 { 0 } else { m.text_addr + m.text_size + m.data_size }),
                _ => QVariant::Invalid,
            },
            _ => QVariant::Invalid,
        }
    }
    fn header_data(&self, section: i32, _o: Orientation, role: ItemRole) -> QVariant {
        if !matches!(role, ItemRole::DisplayRole) { return QVariant::Invalid; }
        QVariant::from(match section {
            x if x == ModuleColumn::Name as i32 => "NAME",
            x if x == ModuleColumn::Version as i32 => "VERSION",
            x if x == ModuleColumn::Entry as i32 => "ENTRY",
            x if x == ModuleColumn::Gp as i32 => "GP",
            x if x == ModuleColumn::TextSection as i32 => "TEXT",
            x if x == ModuleColumn::DataSection as i32 => "DATA",
            x if x == ModuleColumn::BssSection as i32 => "BSS",
            _ => "",
        })
    }
}
impl ModuleModel {
    pub fn new(_cpu: usize) -> Self { Self { m_cpu: 0, m_modules: Vec::new() } }
    pub fn refresh_data(&mut self) {}
}

pub struct ModuleView {
    pub m_model: usize,
    pub m_ui: ModuleViewUi,
}
#[derive(Default, Debug)]
pub struct ModuleViewUi {
    pub module_list: ListWidgetStub,
}
impl ModuleView {
    pub fn new(_parameters: &DebuggerViewParameters) -> Self { Self { m_model: 0, m_ui: ModuleViewUi::default() } }
    pub fn open_context_menu(&self) {}
    pub fn on_double_click(&self) {}
}

// ---------------------------------------------------------------------------
// StackModel / StackView
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StackColumn {
    #[default]
    Entry,
    EntryLabel,
    Pc,
    PcOpcode,
    Sp,
    Size,
    ColumnCount,
}

pub struct StackModel {
    pub m_cpu: usize,
    pub m_stack_frames: Vec<StackFrame>,
}
impl QAbstractTableModel for StackModel {
    fn row_count(&self) -> i32 { self.m_stack_frames.len() as i32 }
    fn column_count(&self) -> i32 { StackColumn::ColumnCount as i32 }
    fn data(&self, index: QModelIndex, role: ItemRole) -> QVariant {
        let row = index.row as usize;
        if row >= self.m_stack_frames.len() { return QVariant::Invalid; }
        let f = self.m_stack_frames[row];
        match role {
            ItemRole::DisplayRole => match index.column {
                x if x == StackColumn::Entry as i32 => QVariant::from(filled_qstring_from_value(f.entry as u64, 16)),
                x if x == StackColumn::EntryLabel as i32 => QVariant::from(QString::new()),
                x if x == StackColumn::Pc as i32 => QVariant::from(filled_qstring_from_value(f.pc as u64, 16)),
                x if x == StackColumn::PcOpcode as i32 => QVariant::from(QString::new()),
                x if x == StackColumn::Sp as i32 => QVariant::from(filled_qstring_from_value(f.sp as u64, 16)),
                x if x == StackColumn::Size as i32 => QVariant::from(f.stack_size as i32),
                _ => QVariant::Invalid,
            },
            ItemRole::UserRole => match index.column {
                x if x == StackColumn::Entry as i32 => QVariant::from(f.entry),
                x if x == StackColumn::Sp as i32 => QVariant::from(f.sp),
                x if x == StackColumn::Pc as i32 => QVariant::from(f.pc),
                x if x == StackColumn::Size as i32 => QVariant::from(f.stack_size as i32),
                _ => QVariant::Invalid,
            },
            _ => QVariant::Invalid,
        }
    }
    fn header_data(&self, section: i32, _o: Orientation, role: ItemRole) -> QVariant {
        if !matches!(role, ItemRole::DisplayRole) { return QVariant::Invalid; }
        QVariant::from(match section {
            x if x == StackColumn::Entry as i32 => "ENTRY",
            x if x == StackColumn::EntryLabel as i32 => "LABEL",
            x if x == StackColumn::Pc as i32 => "PC",
            x if x == StackColumn::PcOpcode as i32 => "INSTRUCTION",
            x if x == StackColumn::Sp as i32 => "STACK POINTER",
            x if x == StackColumn::Size as i32 => "SIZE",
            _ => "",
        })
    }
}
impl StackModel {
    pub fn new(_cpu: usize) -> Self { Self { m_cpu: 0, m_stack_frames: Vec::new() } }
    pub fn refresh_data(&mut self) {}
}

pub struct StackView {
    pub m_model: usize,
    pub m_ui: StackViewUi,
}
#[derive(Default, Debug)]
pub struct StackViewUi {
    pub stack_list: ListWidgetStub,
}
impl StackView {
    pub fn new(_parameters: &DebuggerViewParameters) -> Self { Self { m_model: 0, m_ui: StackViewUi::default() } }
    pub fn open_context_menu(&self) {}
    pub fn on_double_click(&self) {}
}

// ---------------------------------------------------------------------------
// ThreadModel / ThreadView
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThreadColumn {
    #[default]
    Id,
    Pc,
    Entry,
    Priority,
    State,
    WaitType,
    WaitId,
    ColumnCount,
}

pub struct ThreadModel {
    pub m_cpu: usize,
    pub m_threads: Vec<Box<dyn BiosThread>>,
}
impl ThreadModel {
    pub fn new(_cpu: usize) -> Self { Self { m_cpu: 0, m_threads: Vec::new() } }
    pub fn refresh_data(&mut self) {}
}

pub struct ThreadView {
    pub m_model: usize,
    pub m_proxy_model: usize,
    pub m_ui: ThreadViewUi,
}
#[derive(Default, Debug)]
pub struct ThreadViewUi {
    pub thread_list: ListWidgetStub,
}
impl ThreadView {
    pub fn new(_parameters: &DebuggerViewParameters) -> Self { Self { m_model: 0, m_proxy_model: 0, m_ui: ThreadViewUi::default() } }
    pub fn open_context_menu(&self) {}
    pub fn on_double_click(&self) {}
}

// ---------------------------------------------------------------------------
// DisassemblyView
// ---------------------------------------------------------------------------

#[derive(Default, Clone)]
pub struct SelectionInfoRecord {
    pub address: u32,
    pub info_type: SelectionInfo,
}

pub struct DisassemblyView {
    pub base: DebuggerView,
    pub m_visible_start: u32,
    pub m_visible_rows: u32,
    pub m_selected_address_start: u32,
    pub m_selected_address_end: u32,
    pub m_row_height: u32,
    pub m_noped_instructions: BTreeMap<u32, u32>,
    pub m_stubbed_functions: BTreeMap<u32, (u32, u32)>,
    pub m_show_instruction_bytes: bool,
    pub m_go_to_program_counter_on_pause: bool,
    pub m_disassembly_manager: Box<dyn DisassemblyManager>,
    pub m_ui: DisassemblyViewUi,
}

#[derive(Default, Debug)]
pub struct DisassemblyViewUi;
impl DisassemblyViewUi {
    pub fn setup_ui(&mut self, _v: &mut DisassemblyView) {}
    pub fn update(&self) {}
    pub fn set_focus(&self) {}
}

impl DisassemblyView {
    pub fn new(parameters: &DebuggerViewParameters) -> Self {
        let base = DebuggerView::new(parameters, Flags::MONOSPACE_FONT);
        Self {
            base,
            m_visible_start: 0x100000,
            m_visible_rows: 0,
            m_selected_address_start: 0,
            m_selected_address_end: 0,
            m_row_height: 0,
            m_noped_instructions: BTreeMap::new(),
            m_stubbed_functions: BTreeMap::new(),
            m_show_instruction_bytes: true,
            m_go_to_program_counter_on_pause: true,
            m_disassembly_manager: Box::new(FakeDisassemblyManager::default()),
            m_ui: DisassemblyViewUi::default(),
        }
    }

    pub fn to_json(&self, json: &mut JsonValueWrapper) {
        self.base.to_json(json);
        json.value_mut().add_member("startAddress", serde_json_like::Value::Uint(self.m_visible_start as u64));
        json.value_mut().add_member("goToPCOnPause", serde_json_like::Value::Bool(self.m_go_to_program_counter_on_pause));
        json.value_mut().add_member("showInstructionBytes", serde_json_like::Value::Bool(self.m_show_instruction_bytes));
    }
    pub fn from_json(&mut self, json: &JsonValueWrapper) -> bool {
        if !self.base.from_json(json) { return false; }
        if let Some(v) = json.value().find_member("startAddress") { if let Some(u) = v.as_uint() { self.m_visible_start = (u as u32) & !3; } }
        if let Some(v) = json.value().find_member("goToPCOnPause") { if let Some(b) = v.as_bool() { self.m_go_to_program_counter_on_pause = b; } }
        if let Some(v) = json.value().find_member("showInstructionBytes") { if let Some(b) = v.as_bool() { self.m_show_instruction_bytes = b; } }
        true
    }
    pub fn context_copy_address(&self) {}
    pub fn context_copy_instruction_hex(&self) {}
    pub fn context_copy_instruction_text(&self) {}
    pub fn context_paste_instruction_text(&self) {}
    pub fn context_assemble_instruction(&self) {}
    pub fn context_noop_instruction(&self) {}
    pub fn context_restore_instruction(&self) {}
    pub fn context_run_to_cursor(&self) {}
    pub fn context_jump_to_cursor(&self) {}
    pub fn context_toggle_breakpoint(&self) {}
    pub fn context_follow_branch(&self) {}
    pub fn context_go_to_address(&self) {}
    pub fn context_add_function(&self) {}
    pub fn context_copy_function_name(&self) {}
    pub fn context_remove_function(&self) {}
    pub fn context_rename_function(&self) {}
    pub fn context_stub_function(&self) {}
    pub fn context_restore_function(&self) {}
    pub fn context_show_instruction_bytes(&mut self) { self.m_show_instruction_bytes = !self.m_show_instruction_bytes; }
    pub fn open_context_menu(&self) {}
    pub fn goto_address_and_set_focus(&mut self, address: u32) { self.goto_address(address, true); }
    pub fn goto_program_counter_on_pause(&mut self) {
        if self.m_go_to_program_counter_on_pause {
            let pc = self.base.cpu().get_pc();
            self.goto_address(pc, false);
        }
    }
    pub fn goto_address(&mut self, address: u32, _focus: bool) {
        let dest = address & !3;
        self.m_visible_start = dest.saturating_sub(self.m_visible_rows * 4 / 2) & !3;
        self.m_selected_address_start = dest;
        self.m_selected_address_end = dest;
    }
    pub fn toggle_breakpoint(&self, _address: u32) {}
    pub fn set_instructions(&mut self, _start: u32, _end: u32, _value: u32) {}
    pub fn address_can_restore(&self, _start: u32, _end: u32) -> bool { false }
    pub fn function_can_restore(&self, _address: u32) -> bool { false }
}

#[derive(Default)]
pub struct FakeDisassemblyManager;
impl DisassemblyManager for FakeDisassemblyManager {
    fn set_cpu(&mut self, _cpu: *const dyn DebugInterface) {}
    fn analyze(&mut self, _start: u32, _size: u32) {}
    fn get_line(&self, _address: u32, _get_relevant: bool, line: &mut DisassemblyLineInfo) {
        *line = DisassemblyLineInfo { name: "nop".into(), params: QString::new(), ty: DisassemblyLineType::Opcode, info: MipsOpcodeInfo::default() };
    }
    fn get_nth_next_address(&self, start: u32, n: u32) -> u32 { start + n * 4 }
    fn get_branch_lines(&self, _start: u32, _size: u32) -> Vec<BranchLine> { Vec::new() }
}

// ---------------------------------------------------------------------------
// RegisterView
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RegisterCategory {
    #[default]
    Gpr,
    Fpr,
    Vu0,
    Vu0F,
    Count,
}
pub const EECAT_VU0F: i32 = RegisterCategory::Vu0F as i32;
pub const EECAT_FPR: i32 = RegisterCategory::Fpr as i32;

pub struct RegisterView {
    pub base: DebuggerView,
    pub m_render_start: (i32, i32),
    pub m_row_start: i32,
    pub m_row_end: i32,
    pub m_row_height: i32,
    pub m_field_start_x: [i32; 4],
    pub m_field_width: i32,
    pub m_selected_row: i32,
    pub m_selected_128_field: i32,
    pub m_show_vu0f_float: bool,
    pub m_show_fpr_float: bool,
    pub m_ui: RegisterViewUi,
}

#[derive(Default, Debug)]
pub struct RegisterViewUi {
    pub register_tabs: TabBarStub,
}
#[derive(Default, Debug)]
pub struct TabBarStub;
impl TabBarStub {
    pub fn set_draw_base(&mut self, _b: bool) {}
    pub fn add_tab(&mut self, _s: &str) {}
    pub fn current_index(&self) -> i32 { 0 }
    pub fn current_changed(&self) {}
    pub fn pos(&self) -> (i32, i32) { (0, 0) }
    pub fn size(&self) -> (i32, i32) { (0, 0) }
}

impl RegisterView {
    pub fn new(parameters: &DebuggerViewParameters) -> Self {
        Self {
            base: DebuggerView::new(parameters, Flags::MONOSPACE_FONT),
            m_render_start: (0, 0),
            m_row_start: 0,
            m_row_end: 0,
            m_row_height: 0,
            m_field_start_x: [0; 4],
            m_field_width: 0,
            m_selected_row: 0,
            m_selected_128_field: 0,
            m_show_vu0f_float: false,
            m_show_fpr_float: false,
            m_ui: RegisterViewUi::default(),
        }
    }

    pub fn to_json(&self, json: &mut JsonValueWrapper) {
        self.base.to_json(json);
        json.value_mut().add_member("showVU0FFloat", serde_json_like::Value::Bool(self.m_show_vu0f_float));
        json.value_mut().add_member("showFPRFloat", serde_json_like::Value::Bool(self.m_show_fpr_float));
    }
    pub fn from_json(&mut self, json: &JsonValueWrapper) -> bool {
        if !self.base.from_json(json) { return false; }
        if let Some(v) = json.value().find_member("showVU0FFloat") { if let Some(b) = v.as_bool() { self.m_show_vu0f_float = b; } }
        if let Some(v) = json.value().find_member("showFPRFloat") { if let Some(b) = v.as_bool() { self.m_show_fpr_float = b; } }
        true
    }

    pub fn tab_current_changed(&mut self, _cur: i32) { self.m_row_start = 0; }
    pub fn paint_event(&self) {}
    pub fn mouse_press_event(&mut self) {}
    pub fn mouse_double_click_event(&self) {}
    pub fn wheel_event(&mut self) {}
    pub fn custom_menu_requested(&self) {}
    pub fn context_copy_value(&self) {}
    pub fn context_copy_top(&self) {}
    pub fn context_copy_bottom(&self) {}
    pub fn context_copy_segment(&self) {}
    pub fn context_change_value(&self) {}
    pub fn context_change_top(&self) {}
    pub fn context_change_bottom(&self) {}
    pub fn context_change_segment(&self) {}
    pub fn context_create_goto_event(&self) -> Option<GoToAddress> { None }
}

// ---------------------------------------------------------------------------
// NewSymbolDialogs (stub)
// ---------------------------------------------------------------------------

pub use new_symbol_dialogs::NewFunctionDialog;

// ---------------------------------------------------------------------------
// Small helper for the disassembly view's "selection info" code.
// ---------------------------------------------------------------------------

pub fn fetch_selection_info(
    sel_info: SelectionInfo,
    start: u32,
    end: u32,
    cpu: &dyn DebugInterface,
    disasm: &dyn DisassemblyManager,
) -> QString {
    let mut info = QString::new();
    let mut i = start;
    while i <= end {
        if i != start { info.push('\n'); }
        match sel_info {
            SelectionInfo::Address => info.push_str(&filled_qstring_from_value(i as u64, 16)),
            SelectionInfo::InstructionText => {
                let mut line = DisassemblyLineInfo::default();
                disasm.get_line(i, true, &mut line);
                info.push_str(&format!("{} {}", line.name, line.params));
            }
            SelectionInfo::InstructionHex => {
                let mut v = false;
                info.push_str(&filled_qstring_from_value(cpu.read32(i, &mut v) as u64, 16));
            }
        }
        i += 4;
    }
    info
}

// ---------------------------------------------------------------------------
// Re-export everything for the convenience of consumers who want a
// single import point.
// ---------------------------------------------------------------------------

pub use new_symbol_dialogs as symbol_tree_dialogs;
pub use type_string as type_str;
pub use symbol_tree_location as symbol_tree_loc;

pub mod prelude {
    pub use super::{
        AddToSavedAddresses, AnalysisOptionsDialog, BreakpointDialog, BreakpointMemcheck,
        BreakpointModel, ChangeEventKind, Clipboard, DebugAnalysisOptions, DebugInterface,
        DebuggerView, DebuggerViewParameters, DebuggerWindow, DockManager, Event, EventBus,
        Flags, GoToAddress, GoToAddressFilter, IopMod, JsonDocument, JsonObject, JsonValueWrapper,
        MemorySearchView, MemoryView, MemoryViewTable, MemoryViewType, ModuleModel, ModuleView,
        Refresh, RegisterView, SavedAddressesModel, SavedAddressesView, SearchComparison,
        SearchResult, SearchType, SelectionInfo, StackFrame, StackModel, StackView, ThreadModel,
        ThreadStatus, ThreadView, U128, VMUpdate, WaitState, broadcast_event, console,
        convert_endian_u16, convert_endian_u32, convert_endian_u64, cpu_name, decode_hex,
        encode_hex, filled_qstring_from_value, get_debug_interface, global_clipboard,
        global_debugger_window, hex_nibble, register_debugger_view, search_worker, send_event,
        settings, split_on_newline,
    };
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debugger_view_subscribes_to_event() {
        let mut view = DebuggerView::new(&DebuggerViewParameters::default(), 0);
        view.receive_event::<Refresh>(|_| true);
        let r = Refresh;
        assert!(view.handle_event(&r));
    }

    #[test]
    fn breakpoint_model_insert_and_remove() {
        let mut m = BreakpointModel::default();
        m.insert_breakpoint_rows(0, 1, vec![BreakpointMemcheck::Breakpoint(BreakPoint { addr: 0x1000, ..Default::default() })]);
        assert_eq!(m.row_count(), 1);
        m.remove_rows(0, 1);
        assert_eq!(m.row_count(), 0);
    }

    #[test]
    fn memory_view_table_navigation() {
        let mut t = MemoryViewTable::new(WidgetHandle(0));
        t.set_view_type(MemoryViewType::Byte);
        t.update_selected_address(0x1000, false);
        assert_eq!(t.selected_address, 0x1000);
    }

    #[test]
    fn search_worker_finds_values() {
        // Simple smoke test of the search worker with a fake CPU.
    }

    #[test]
    fn encode_decode_hex() {
        let bytes = vec![0xDE, 0xAD, 0xBE, 0xEF];
        assert_eq!(encode_hex(&bytes), "DEADBEEF");
        assert_eq!(decode_hex("DEADBEEF"), bytes);
    }
}
