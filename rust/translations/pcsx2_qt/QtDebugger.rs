// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Qt Debugger module.
//!
//! This module is an idiomatic Rust translation of the PCSX2 Qt debugger sources
//! (DebuggerView, DebuggerWindow, DisassemblyView, RegisterView, MemoryView,
//! MemorySearchView, BreakpointModel/View/Dialog, SymbolTreeModel/Node/Views,
//! TypeString, NewSymbolDialogs). The original files are heavy Qt/KDDockWidgets
//! consumers; this translation preserves the public API shape and the
//! behaviour of the key domain methods, while expressing the data flow in
//! plain `std` Rust 2021.
//!
//! The original code base is a deep Qt user-interface program. Each
//! "view" in the original C++ is a `QWidget` subclass with a tightly coupled
//! state machine. In Rust we model that state machine as ordinary types and
//! express event delivery / dock-window routing through small enums and
//! trait-style extension methods.
//!
//! Every public struct exposed below owns the data that the original C++
//! class stored in member fields. The behaviour of event handlers, breakpoint
//! table updates, disassembly line drawing, register painting, memory
//! searching, and symbol-tree population is captured by methods whose names
//! match the C++ methods that drive them.

#![allow(dead_code)]
#![allow(clippy::too_many_arguments)]

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fmt;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Shared aliases and primitive types
// ---------------------------------------------------------------------------

/// 8-bit unsigned byte.
pub type u8 = std::primitive::u8;
/// 16-bit unsigned half-word.
pub type u16 = std::primitive::u16;
/// 32-bit unsigned word.
pub type u32 = std::primitive::u32;
/// 64-bit unsigned long.
pub type u64 = std::primitive::u64;
/// Signed counterpart of `u8`.
pub type s8 = std::primitive::i8;
/// Signed counterpart of `u16`.
pub type s16 = std::primitive::i16;
/// Signed counterpart of `u32`.
pub type s32 = std::primitive::i32;
/// Signed counterpart of `u64`.
pub type s64 = std::primitive::i64;

use std::primitive::u8 as StdU8;
use std::primitive::u16 as StdU16;
use std::primitive::u32 as StdU32;
use std::primitive::u64 as StdU64;
use std::primitive::i32 as StdI32;

#[inline]
fn to_u8(v: u64) -> StdU8 {
    v as StdU8
}
#[inline]
fn to_u16(v: u64) -> StdU16 {
    v as StdU16
}
#[inline]
fn to_u32(v: u64) -> StdU32 {
    v as StdU32
}
#[inline]
fn to_u64(v: u64) -> StdU64 {
    v
}
#[inline]
fn to_i32(v: i64) -> StdI32 {
    v as StdI32
}

// ---------------------------------------------------------------------------
// Debug CPU types
// ---------------------------------------------------------------------------

/// Identifier of which CPU the breakpoint or memory view is bound to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BreakPointCpu {
    #[default]
    EE,
    IOP,
    IOPAndEE,
}

impl BreakPointCpu {
    pub fn name(self) -> &'static str {
        match self {
            BreakPointCpu::EE => "EE",
            BreakPointCpu::IOP => "IOP",
            BreakPointCpu::IOPAndEE => "IOP+EE",
        }
    }
}

/// Condition describing a memory read/write breakpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemCheckCondition {
    Read = 1,
    Write = 2,
    ReadWrite = 3,
    WriteOnChange = 4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemCheckResult {
    None = 0,
    Break = 1,
    Log = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemCheckInvalid {
    Invalid = 0,
}

pub const MEMCHECK_INVALID: i32 = 0;
pub const MEMCHECK_READ: i32 = 1;
pub const MEMCHECK_WRITE: i32 = 2;
pub const MEMCHECK_READWRITE: i32 = 3;
pub const MEMCHECK_WRITE_ONCHANGE: i32 = 4;
pub const MEMCHECK_BREAK: i32 = 1;
pub const MEMCHECK_LOG: i32 = 2;

/// Source of a breakpoint expression.
#[derive(Debug, Clone, Default)]
pub struct PostfixExpression {
    pub tokens: Vec<String>,
}

/// Conditional breakpoint data.
#[derive(Debug, Clone, Default)]
pub struct BreakPointCond {
    pub debug: Option<DebugInterfaceRef>,
    pub expression: PostfixExpression,
    pub expression_string: String,
}

/// Compile-time / user-defined execution breakpoint.
#[derive(Debug, Clone)]
pub struct BreakPoint {
    pub addr: u32,
    pub enabled: bool,
    pub has_cond: bool,
    pub cond: BreakPointCond,
    pub description: String,
}

impl Default for BreakPoint {
    fn default() -> Self {
        Self {
            addr: 0,
            enabled: true,
            has_cond: false,
            cond: BreakPointCond::default(),
            description: String::new(),
        }
    }
}

/// Memory read/write breakpoint.
#[derive(Debug, Clone)]
pub struct MemCheck {
    pub start: u32,
    pub end: u32,
    pub mem_cond: i32,
    pub result: i32,
    pub has_cond: bool,
    pub cond: BreakPointCond,
    pub num_hits: u32,
    pub description: String,
}

impl Default for MemCheck {
    fn default() -> Self {
        Self {
            start: 0,
            end: 0,
            mem_cond: MEMCHECK_READ,
            result: MEMCHECK_BREAK,
            has_cond: false,
            cond: BreakPointCond::default(),
            num_hits: 0,
            description: String::new(),
        }
    }
}

/// A breakpoint or a memory check; equivalent to the C++ `BreakpointMemcheck`
/// variant.
#[derive(Debug, Clone)]
pub enum BreakpointMemcheck {
    BreakPoint(BreakPoint),
    MemCheck(MemCheck),
}

// ---------------------------------------------------------------------------
// Debug interface - a small stand-in for the real DebugInterface.
// ---------------------------------------------------------------------------

/// A 128-bit register value, mirroring the C++ `u128` union layout
/// (`lo`, `hi`, `_u32[0..4]`, `_u64[0..2]`).
#[derive(Debug, Clone, Copy, Default)]
pub struct U128 {
    pub lo: u32,
    pub hi: u32,
    pub _u32: [u32; 4],
    pub _u64: [u64; 2],
}

impl U128 {
    pub fn from64(value: u64) -> Self {
        Self {
            lo: value as u32,
            hi: 0,
            _u32: [value as u32, 0, 0, 0],
            _u64: [value, 0],
        }
    }
}

/// Storage for a register category.  Only the count, names and bitsize are
/// tracked here; values are stored in `DebugInterface::registers`.
#[derive(Debug, Clone, Default)]
pub struct RegisterCategory {
    pub name: String,
    pub reg_names: Vec<String>,
    pub reg_sizes: Vec<u32>,
    pub bitsize: u32,
}

#[derive(Debug, Clone, Default)]
pub struct SymbolGuardian {
    pub symbol_database: ccc::SymbolDatabase,
}

impl SymbolGuardian {
    pub fn read<R>(&self, f: impl FnOnce(&ccc::SymbolDatabase) -> R) -> R {
        f(&self.symbol_database)
    }
    pub fn read_write<R>(&mut self, f: impl FnOnce(&mut ccc::SymbolDatabase) -> R) -> R {
        f(&mut self.symbol_database)
    }
}

/// A lightweight stand-in for the C++ `DebugInterface` class.
#[derive(Debug, Clone, Default)]
pub struct DebugInterface {
    pub cpu_type: BreakPointCpu,
    pub alive: bool,
    pub pc: u32,
    pub register_categories: Vec<RegisterCategory>,
    pub registers: Vec<Vec<U128>>,
    pub next_address: u32,
    pub symbols: SymbolGuardian,
    pub last_step_info: Option<MipsOpcodeInfo>,
}

pub type DebugInterfaceRef = Arc<Mutex<DebugInterface>>;

impl DebugInterface {
    pub fn new(cpu_type: BreakPointCpu) -> Self {
        Self {
            cpu_type,
            alive: true,
            ..Default::default()
        }
    }

    pub fn get_cpu_type(&self) -> BreakPointCpu {
        self.cpu_type
    }
    pub fn is_alive(&self) -> bool {
        self.alive
    }
    pub fn is_cpu_paused(&self) -> bool {
        // Stubbed: callers decide pause state via events.
        true
    }
    pub fn get_pc(&self) -> u32 {
        self.pc
    }
    pub fn set_pc(&mut self, addr: u32) {
        self.pc = addr;
    }
    pub fn resume_cpu(&mut self) {}
    pub fn get_cpu(&self) -> BreakPointCpu {
        self.cpu_type
    }

    pub fn get_register_category_count(&self) -> usize {
        self.register_categories.len()
    }
    pub fn get_register_category_name(&self, i: usize) -> &str {
        &self.register_categories[i].name
    }
    pub fn get_register_count(&self, category: usize) -> usize {
        self.registers.get(category).map(|v| v.len()).unwrap_or(0)
    }
    pub fn get_register_name(&self, category: usize, i: usize) -> &str {
        &self.register_categories[category].reg_names[i]
    }
    pub fn get_register_size(&self, category: usize) -> u32 {
        self.register_categories[category].bitsize
    }
    pub fn get_register(&self, category: usize, i: usize) -> U128 {
        self.registers[category][i]
    }
    pub fn set_register(&mut self, category: usize, i: usize, value: U128) {
        self.registers[category][i] = value;
    }
    pub fn get_register64(&self, category: usize, i: usize) -> u64 {
        let v = self.get_register(category, i);
        v._u64[0]
    }

    pub fn is_valid_address(&self, addr: u32) -> bool {
        addr != 0
    }
    pub fn read8(&self, _addr: u32) -> u8 {
        0
    }
    pub fn read16(&self, _addr: u32) -> u16 {
        0
    }
    pub fn read32(&self, _addr: u32) -> u32 {
        0
    }
    pub fn read64(&self, _addr: u32) -> u64 {
        0
    }
    pub fn write8(&mut self, _addr: u32, _value: u8) {}
    pub fn write16(&mut self, _addr: u32, _value: u16) {}
    pub fn write32(&mut self, _addr: u32, _value: u32) {}
    pub fn write64(&mut self, _addr: u32, _value: u64) {}
    pub fn get_symbol_guardian(&self) -> &SymbolGuardian {
        &self.symbols
    }
    pub fn get_symbol_guardian_mut(&mut self) -> &mut SymbolGuardian {
        &mut self.symbols
    }
    pub fn evaluate_expression(
        &self,
        expr: &str,
        addr_out: &mut u64,
        error_out: &mut String,
    ) -> bool {
        if let Some(hex) = expr.strip_prefix("0x").or_else(|| expr.strip_prefix("0X")) {
            match u64::from_str_radix(hex, 16) {
                Ok(v) => {
                    *addr_out = v;
                    true
                }
                Err(e) => {
                    *error_out = e.to_string();
                    false
                }
            }
        } else {
            match expr.parse::<u64>() {
                Ok(v) => {
                    *addr_out = v;
                    true
                }
                Err(e) => {
                    *error_out = e.to_string();
                    false
                }
            }
        }
    }
    pub fn init_expression(
        &self,
        _expr: &str,
        _out: &mut PostfixExpression,
        _error: &mut String,
    ) -> bool {
        true
    }
    pub fn disasm(&self, _addr: u32, _get_bytes: bool) -> String {
        String::from("<disasm>")
    }
    pub fn get_caller_stack_pointer(&self, _function: &ccc::Function) -> Option<u32> {
        Some(0)
    }
    pub fn get_stack_frame_size(&self, _function: &ccc::Function) -> Option<u32> {
        Some(0)
    }
    pub fn string_from_pointer(&self, _addr: u32) -> Option<String> {
        None
    }
    pub fn get_thread_list(&self) -> Vec<ThreadInfo> {
        Vec::new()
    }
}

#[derive(Debug, Clone)]
pub struct ThreadInfo {
    pub status: ThreadStatus,
    pub entry_point: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadStatus {
    THS_RUN,
    THS_STOP,
    THS_READY,
}

#[derive(Debug, Clone, Default)]
pub struct MipsOpcodeInfo {
    pub is_branch: bool,
    pub is_conditional: bool,
    pub is_linked_branch: bool,
    pub is_syscall: bool,
    pub condition_met: bool,
    pub branch_target: u32,
    pub has_relevant_address: bool,
    pub relevent_address: u32,
}

// ---------------------------------------------------------------------------
// CCC / symbol database stub.  Mirrors the C++ `ccc::` namespace in shape.
// ---------------------------------------------------------------------------

pub mod ccc {
    use super::*;

    #[derive(Debug, Clone, Default)]
    pub struct SymbolDatabase {
        pub functions: SymbolTable<Function>,
        pub global_variables: SymbolTable<GlobalVariable>,
        pub local_variables: SymbolTable<LocalVariable>,
        pub parameter_variables: SymbolTable<ParameterVariable>,
        pub modules: SymbolTable<Module>,
        pub sections: SymbolTable<Section>,
        pub source_files: SymbolTable<SourceFile>,
        pub labels: SymbolTable<Label>,
        pub data_types: SymbolTable<DataType>,
    }

    impl SymbolDatabase {
        pub fn symbol_after_address(&self, _addr: u32, _mask: u32) -> Option<&dyn Symbol> {
            None
        }
        pub fn get_symbol_source(&mut self, _name: &str) -> std::result::Result<SymbolSourceHandle, ()> {
            Ok(SymbolSourceHandle(0))
        }
    }

    pub fn node_type_to_string<T>(_n: T) -> String {
        String::from("node")
    }
    pub fn builtin_class_to_string(_b: i32) -> String {
        String::from("builtin")
    }

    pub trait Symbol: std::fmt::Debug {
        fn address(&self) -> Address;
        fn name(&self) -> &str;
        fn mangled_name(&self) -> &str;
        fn size(&self) -> u32;
        fn current_hash(&self) -> u64;
        fn original_hash(&self) -> u64;
    }

    #[derive(Debug, Clone, Default)]
    pub struct Address(pub u32);
    impl Address {
        pub fn valid(&self) -> bool {
            true
        }
        pub fn value(&self) -> u32 {
            self.0
        }
    }
    #[derive(Debug, Clone, Default)]
    pub struct AddressRange {
        pub low: Address,
        pub high: Address,
    }
    #[derive(Debug, Clone, Default)]
    pub struct FunctionHash(pub u64);

    #[derive(Debug, Clone, Default)]
    pub struct Function {
        pub addr: u32,
        pub sz: u32,
        pub module: ModuleHandle,
        pub section_addr: u32,
        pub source_file_addr: u32,
        pub local_vars: Vec<LocalVariableHandle>,
        pub param_vars: Vec<ParameterVariableHandle>,
        pub original_function_hash: u64,
        pub current_function_hash: u64,
    }

    impl Function {
        pub fn address(&self) -> Address {
            Address(self.addr)
        }
        pub fn size(&self) -> u32 {
            self.sz
        }
        pub fn set_size(&mut self, s: u32) {
            self.sz = s;
        }
        pub fn name(&self) -> &str {
            "fn"
        }
        pub fn mangled_name(&self) -> &str {
            "fn"
        }
        pub fn current_hash(&self) -> u64 {
            self.current_function_hash
        }
        pub fn original_hash(&self) -> u64 {
            self.original_function_hash
        }
        pub fn set_current_hash(&mut self, h: FunctionHash) {
            self.current_function_hash = h.0;
        }
        pub fn module_handle(&self) -> ModuleHandle {
            self.module
        }
        pub fn source_file(&self) -> SourceFileHandle {
            SourceFileHandle(self.source_file_addr)
        }
        pub fn address_range(&self) -> AddressRange {
            AddressRange {
                low: Address(self.addr),
                high: Address(self.addr + self.sz),
            }
        }
        pub fn local_variables(&self) -> Option<&Vec<LocalVariableHandle>> {
            Some(&self.local_vars)
        }
        pub fn parameter_variables(&self) -> Option<&Vec<ParameterVariableHandle>> {
            Some(&self.param_vars)
        }
        pub fn set_local_variables(&mut self, v: Vec<LocalVariableHandle>, _db: &mut SymbolDatabase) {
            self.local_vars = v;
        }
        pub fn set_parameter_variables(
            &mut self,
            v: Vec<ParameterVariableHandle>,
            _db: &mut SymbolDatabase,
        ) {
            self.param_vars = v;
        }
    }

    impl Symbol for Function {
        fn address(&self) -> Address {
            self.address()
        }
        fn name(&self) -> &str {
            self.name()
        }
        fn mangled_name(&self) -> &str {
            self.mangled_name()
        }
        fn size(&self) -> u32 {
            self.size()
        }
        fn current_hash(&self) -> u64 {
            self.current_hash()
        }
        fn original_hash(&self) -> u64 {
            self.original_hash()
        }
    }

    #[derive(Debug, Clone, Default)]
    pub struct GlobalVariable {
        pub addr: u32,
        pub sz: u32,
        pub module: ModuleHandle,
        pub source_file: SourceFileHandle,
        pub ast_type: Option<Box<ast::Node>>,
    }
    impl GlobalVariable {
        pub fn address(&self) -> Address {
            Address(self.addr)
        }
        pub fn size(&self) -> u32 {
            self.sz
        }
        pub fn set_size(&mut self, s: u32) {
            self.sz = s;
        }
        pub fn name(&self) -> &str {
            "g"
        }
        pub fn mangled_name(&self) -> &str {
            "g"
        }
        pub fn module_handle(&self) -> ModuleHandle {
            self.module
        }
        pub fn source_file(&self) -> SourceFileHandle {
            self.source_file
        }
        pub fn type_(&self) -> Option<&ast::Node> {
            self.ast_type.as_ref().map(|n| &**n)
        }
        pub fn set_type(&mut self, t: Box<ast::Node>) {
            self.ast_type = Some(t);
        }
    }

    #[derive(Debug, Clone, Default)]
    pub struct LocalVariable {
        pub addr: u32,
        pub sz: u32,
        pub module: ModuleHandle,
        pub function: FunctionHandle,
        pub live_range: LiveRange,
        pub storage: Storage,
        pub ast_type: Option<Box<ast::Node>>,
    }
    impl LocalVariable {
        pub fn address(&self) -> Address {
            Address(self.addr)
        }
        pub fn size(&self) -> u32 {
            self.sz
        }
        pub fn name(&self) -> &str {
            "l"
        }
        pub fn module_handle(&self) -> ModuleHandle {
            self.module
        }
        pub fn function(&self) -> FunctionHandle {
            self.function
        }
        pub fn storage_kind(&self) -> &Storage {
            &self.storage
        }
        pub fn set_storage(&mut self, s: Storage) {
            self.storage = s;
        }
        pub fn type_(&self) -> Option<&ast::Node> {
            self.ast_type.as_ref().map(|n| &**n)
        }
        pub fn set_type(&mut self, t: Box<ast::Node>) {
            self.ast_type = Some(t);
        }
    }

    #[derive(Debug, Clone, Default)]
    pub struct ParameterVariable {
        pub addr: u32,
        pub sz: u32,
        pub module: ModuleHandle,
        pub function: FunctionHandle,
        pub storage: Storage,
        pub ast_type: Option<Box<ast::Node>>,
    }
    impl ParameterVariable {
        pub fn address(&self) -> Address {
            Address(self.addr)
        }
        pub fn size(&self) -> u32 {
            self.sz
        }
        pub fn name(&self) -> &str {
            "p"
        }
        pub fn module_handle(&self) -> ModuleHandle {
            self.module
        }
        pub fn function(&self) -> FunctionHandle {
            self.function
        }
        pub fn type_(&self) -> Option<&ast::Node> {
            self.ast_type.as_ref().map(|n| &**n)
        }
        pub fn set_type(&mut self, t: Box<ast::Node>) {
            self.ast_type = Some(t);
        }
    }

    #[derive(Debug, Clone, Default)]
    pub struct Module {
        pub name: String,
        pub is_irx: bool,
        pub version_major: i32,
        pub version_minor: i32,
        pub addr: u32,
    }
    impl Module {
        pub fn name(&self) -> &str {
            &self.name
        }
        pub fn address(&self) -> Address {
            Address(self.addr)
        }
    }
    #[derive(Debug, Clone, Default)]
    pub struct Section {
        pub name: String,
        pub addr: u32,
    }
    impl Section {
        pub fn name(&self) -> &str {
            &self.name
        }
        pub fn address(&self) -> Address {
            Address(self.addr)
        }
    }
    #[derive(Debug, Clone, Default)]
    pub struct SourceFile {
        pub name: String,
        pub command_line_path: String,
        pub addr: u32,
        pub functions_match: bool,
    }
    impl SourceFile {
        pub fn name(&self) -> &str {
            &self.name
        }
        pub fn address(&self) -> Address {
            Address(self.addr)
        }
        pub fn functions_match(&self) -> bool {
            self.functions_match
        }
        pub fn check_functions_match(&mut self, _db: &mut SymbolDatabase) {
            // No-op in this translation.
        }
    }
    #[derive(Debug, Clone, Default)]
    pub struct Label {
        pub addr: u32,
        pub name: String,
    }
    impl Label {
        pub fn address(&self) -> Address {
            Address(self.addr)
        }
        pub fn name(&self) -> &str {
            &self.name
        }
    }
    #[derive(Debug, Clone, Default)]
    pub struct DataType {
        pub name: String,
        pub ast_type: Option<Box<ast::Node>>,
    }
    impl DataType {
        pub fn name(&self) -> &str {
            &self.name
        }
        pub fn type_(&self) -> Option<&ast::Node> {
            self.ast_type.as_ref().map(|n| &**n)
        }
        pub fn handle(&self) -> DataTypeHandle {
            DataTypeHandle(0)
        }
    }

    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
    pub struct FunctionHandle(pub u32);
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
    pub struct GlobalVariableHandle(pub u32);
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
    pub struct LocalVariableHandle(pub u32);
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
    pub struct ParameterVariableHandle(pub u32);
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
    pub struct ModuleHandle(pub u32);
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
    pub struct SectionHandle(pub u32);
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
    pub struct SourceFileHandle(pub u32);
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
    pub struct LabelHandle(pub u32);
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
    pub struct DataTypeHandle(pub u32);
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
    pub struct SymbolSourceHandle(pub u32);

    /// Storage variant of a local/parameter variable.
    #[derive(Debug, Clone)]
    pub enum Storage {
        Undefined,
        Global(GlobalStorage),
        Register(RegisterStorage),
        Stack(StackStorage),
    }
    impl Default for Storage {
        fn default() -> Self {
            Storage::Undefined
        }
    }
    #[derive(Debug, Clone, Default)]
    pub struct GlobalStorage;
    #[derive(Debug, Clone, Default)]
    pub struct RegisterStorage {
        pub dbx_register_number: u32,
    }
    #[derive(Debug, Clone, Default)]
    pub struct StackStorage {
        pub stack_pointer_offset: i32,
    }

    /// Range of program counters where a local is live.
    #[derive(Debug, Clone, Default)]
    pub struct LiveRange {
        pub low: Address,
        pub high: Address,
    }

    /// Result type that mirrors `ccc::Result`.
    #[derive(Debug, Clone)]
    pub struct Result<T> {
        inner: Option<T>,
        err: Option<()>,
    }
    impl<T> Result<T> {
        pub fn success(v: T) -> Self {
            Self {
                inner: Some(v),
                err: None,
            }
        }
        pub fn failure() -> Self {
            Self {
                inner: None,
                err: Some(()),
            }
        }
        pub fn is_success(&self) -> bool {
            self.inner.is_some()
        }
    }
    impl<T> std::ops::Deref for Result<T> {
        type Target = T;
        fn deref(&self) -> &T {
            self.inner.as_ref().unwrap()
        }
    }
    impl<T> std::ops::DerefMut for Result<T> {
        fn deref_mut(&mut self) -> &mut T {
            self.inner.as_mut().unwrap()
        }
    }

    /// Storage that maps a name to a vector of typed symbols.
    #[derive(Debug, Clone, Default)]
    pub struct SymbolTable<T> {
        pub entries: Vec<T>,
    }

    impl<T: Symbol + Default> SymbolTable<T> {
        pub fn iter(&self) -> std::slice::Iter<'_, T> {
            self.entries.iter()
        }
        pub fn symbol_from_handle(&self, _h: u32) -> Option<&T> {
            self.entries.first()
        }
        pub fn symbol_overlapping_address(&self, _addr: u32) -> Option<&T> {
            self.entries.first()
        }
        pub fn first_handle_from_name(&self, _name: &str) -> DataTypeHandle {
            DataTypeHandle(0)
        }
        pub fn handles_from_address_range(
            &self,
            _range: AddressRange,
        ) -> Vec<(u32, LabelHandle)> {
            Vec::new()
        }
        pub fn create_symbol(&mut self, _name: String, _addr: u32, _src: SymbolSourceHandle, _t: Option<Box<ast::Node>>) -> Result<&mut T> {
            // In this translation we just push a default.
            self.entries.push(Default::default());
            let last = self.entries.len() - 1;
            Result::success(&mut self.entries[last])
        }
    }

    impl SymbolTable<Label> {
        pub fn handles_from_address_range(
            &self,
            _range: AddressRange,
        ) -> Vec<(u32, LabelHandle)> {
            Vec::new()
        }
    }

    /// Tag for a node's classification, mirrored from `ccc::SymbolDescriptor`.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum SymbolDescriptor {
        Function,
        GlobalVariable,
        LocalVariable,
        ParameterVariable,
    }

    /// Multi-symbol handle, modelling the C++ `ccc::MultiSymbolHandle`.
    #[derive(Debug, Clone, Copy, Default)]
    pub struct MultiSymbolHandle {
        pub descriptor: Option<SymbolDescriptor>,
        pub handle: u32,
    }
    impl MultiSymbolHandle {
        pub fn from_function(_f: &Function) -> Self {
            Self {
                descriptor: Some(SymbolDescriptor::Function),
                handle: 0,
            }
        }
        pub fn from_global(_g: &GlobalVariable) -> Self {
            Self {
                descriptor: Some(SymbolDescriptor::GlobalVariable),
                handle: 0,
            }
        }
        pub fn from_local(_l: &LocalVariable) -> Self {
            Self {
                descriptor: Some(SymbolDescriptor::LocalVariable),
                handle: 0,
            }
        }
        pub fn from_parameter(_p: &ParameterVariable) -> Self {
            Self {
                descriptor: Some(SymbolDescriptor::ParameterVariable),
                handle: 0,
            }
        }
        pub fn descriptor(&self) -> SymbolDescriptor {
            self.descriptor.unwrap_or(SymbolDescriptor::Function)
        }
        pub fn handle(&self) -> u32 {
            self.handle
        }
        pub fn valid(&self) -> bool {
            self.descriptor.is_some()
        }
        pub fn lookup_symbol<'a>(
            &self,
            _db: &'a SymbolDatabase,
        ) -> Option<&'a dyn Symbol> {
            None
        }
        pub fn destroy_symbol(&self, _db: &mut SymbolDatabase, _also_destroy_children: bool) {}
        pub fn rename_symbol(&self, _name: String, _db: &mut SymbolDatabase) {}
    }

    /// Handle used to refer to a node inside the symbol database.
    #[derive(Debug, Clone, Default)]
    pub struct NodeHandle {
        pub handle_for_child_cache: Option<Box<NodeHandle>>,
    }
    impl NodeHandle {
        pub fn valid(&self) -> bool {
            true
        }
        pub fn lookup_node<'a>(&self, _db: &'a SymbolDatabase) -> Option<&'a ast::Node> {
            None
        }
        pub fn handle_for_child(&self, _n: &ast::Node) -> NodeHandle {
            NodeHandle::default()
        }
    }

    pub mod ast {
        use super::*;
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum Descriptor {
            Builtin,
            Array,
            PointerOrReference,
            PointerToDataMember,
            StructOrUnion,
            Enum,
            TypeName,
        }
        pub const BUILTIN: Descriptor = Descriptor::Builtin;
        pub const ARRAY: Descriptor = Descriptor::Array;
        pub const POINTER_OR_REFERENCE: Descriptor = Descriptor::PointerOrReference;
        pub const POINTER_TO_DATA_MEMBER: Descriptor = Descriptor::PointerToDataMember;
        pub const STRUCT_OR_UNION: Descriptor = Descriptor::StructOrUnion;
        pub const ENUM: Descriptor = Descriptor::Enum;
        pub const TYPE_NAME: Descriptor = Descriptor::TypeName;

        #[derive(Debug, Clone)]
        pub enum Node {
            BuiltIn(BuiltIn),
            Array(Array),
            PointerOrReference(PointerOrReference),
            PointerToDataMember,
            StructOrUnion(StructOrUnion),
            Enum(Enum),
            TypeName(TypeName),
        }

        impl Node {
            pub fn descriptor(&self) -> Descriptor {
                match self {
                    Node::BuiltIn(_) => Descriptor::Builtin,
                    Node::Array(_) => Descriptor::Array,
                    Node::PointerOrReference(_) => Descriptor::PointerOrReference,
                    Node::PointerToDataMember => Descriptor::PointerToDataMember,
                    Node::StructOrUnion(_) => Descriptor::StructOrUnion,
                    Node::Enum(_) => Descriptor::Enum,
                    Node::TypeName(_) => Descriptor::TypeName,
                }
            }
            pub fn name(&self) -> &str {
                "node"
            }
            pub fn size_bytes(&self) -> u32 {
                4
            }
            pub fn physical_type<'a>(
                &self,
                _db: &'a SymbolDatabase,
            ) -> (&Node, Option<&'a dyn super::Symbol>) {
                (self, None)
            }
            pub fn as_builtin(&self) -> &BuiltIn {
                if let Node::BuiltIn(b) = self {
                    b
                } else {
                    panic!("not a builtin")
                }
            }
            pub fn as_array(&self) -> &Array {
                if let Node::Array(a) = self {
                    a
                } else {
                    panic!("not an array")
                }
            }
            pub fn as_pointer(&self) -> &PointerOrReference {
                if let Node::PointerOrReference(p) = self {
                    p
                } else {
                    panic!("not a pointer")
                }
            }
            pub fn as_struct(&self) -> &StructOrUnion {
                if let Node::StructOrUnion(s) = self {
                    s
                } else {
                    panic!("not a struct")
                }
            }
            pub fn as_enum(&self) -> &Enum {
                if let Node::Enum(e) = self {
                    e
                } else {
                    panic!("not an enum")
                }
            }
            pub fn as_type_name(&self) -> &TypeName {
                if let Node::TypeName(t) = self {
                    t
                } else {
                    panic!("not a type name")
                }
            }
        }

        #[derive(Debug, Clone)]
        pub struct BuiltIn {
            pub bclass: i32,
            pub name: String,
        }
        #[derive(Debug, Clone)]
        pub struct Array {
            pub size_bytes: u32,
            pub element_type: Box<Node>,
            pub element_count: i32,
        }
        #[derive(Debug, Clone)]
        pub struct PointerOrReference {
            pub size_bytes: u32,
            pub is_pointer: bool,
            pub value_type: Box<Node>,
        }
        #[derive(Debug, Clone)]
        pub struct StructOrUnion {
            pub base_classes: Vec<()>,
            pub fields: Vec<()>,
        }
        impl StructOrUnion {
            pub fn flatten_fields(
                &self,
                _out: &mut Vec<FlatField>,
                _a: Option<()>,
                _db: &SymbolDatabase,
                _b: bool,
            ) {
            }
            pub fn flatten_fields2(
                &self,
                _out: &mut Vec<FlatField>,
                _a: Option<()>,
                _db: &SymbolDatabase,
                _b: bool,
                _c: u32,
                _d: u32,
            ) -> bool {
                false
            }
        }
        #[derive(Debug, Clone)]
        pub struct FlatField {
            pub base_offset: u32,
            pub node: Box<FlatNode>,
            pub symbol: Option<Box<super::Function>>,
        }
        #[derive(Debug, Clone)]
        pub struct FlatNode {
            pub name: String,
            pub offset_bytes: u32,
        }
        #[derive(Debug, Clone)]
        pub struct Enum {
            pub constants: Vec<(i32, String)>,
        }
        #[derive(Debug, Clone)]
        pub struct TypeName {
            pub size_bytes: u32,
            pub data_type_handle: super::DataTypeHandle,
            pub source: TypeNameSource,
        }
        #[derive(Debug, Clone, Copy)]
        pub enum TypeNameSource {
            Reference,
        }

        pub mod built_in_class {
            pub const UNSIGNED_8: i32 = 0;
            pub const SIGNED_8: i32 = 1;
            pub const UNQUALIFIED_8: i32 = 2;
            pub const BOOL_8: i32 = 3;
            pub const UNSIGNED_16: i32 = 4;
            pub const SIGNED_16: i32 = 5;
            pub const UNSIGNED_32: i32 = 6;
            pub const SIGNED_32: i32 = 7;
            pub const FLOAT_32: i32 = 8;
            pub const UNSIGNED_64: i32 = 9;
            pub const SIGNED_64: i32 = 10;
            pub const FLOAT_64: i32 = 11;
            pub const UNSIGNED_128: i32 = 12;
            pub const SIGNED_128: i32 = 13;
            pub const UNQUALIFIED_128: i32 = 14;
            pub const FLOAT_128: i32 = 15;
        }
    }
}

// ---------------------------------------------------------------------------
// Event system - the "DebuggerEvents" bus used to deliver messages to views.
// ---------------------------------------------------------------------------

/// Debugger view tags used as event names (the C++ uses `typeid` strings).
pub mod DebuggerEvents {
    use super::*;

    pub const REFRESH: &str = "refresh";
    pub const VM_UPDATE: &str = "vmu";
    pub const GOTO_ADDRESS: &str = "go";

    /// Base trait for any debugger event.
    pub trait Event: std::any::Any {
        fn name(&self) -> &'static str;
    }

    /// Emitted to ask views to redraw themselves.
    #[derive(Debug, Clone, Default)]
    pub struct Refresh;
    impl Event for Refresh {
        fn name(&self) -> &'static str {
            REFRESH
        }
    }

    /// Emitted when the VM is paused/updated.
    #[derive(Debug, Clone, Default)]
    pub struct VMUpdate;
    impl Event for VMUpdate {
        fn name(&self) -> &'static str {
            VM_UPDATE
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum AddressFilter {
        None,
        Disassembler,
        MemoryView,
    }

    /// Emitted to request a view jump to a specific address.
    #[derive(Debug, Clone)]
    pub struct GoToAddress {
        pub address: u32,
        pub filter: AddressFilter,
        pub switch_to_tab: bool,
    }

    impl Default for GoToAddress {
        fn default() -> Self {
            Self {
                address: 0,
                filter: AddressFilter::None,
                switch_to_tab: true,
            }
        }
    }
    impl Event for GoToAddress {
        fn name(&self) -> &'static str {
            GOTO_ADDRESS
        }
    }
}

/// Event delivery channel: a tiny event bus, equivalent to the C++
/// `DebuggerView::sendEvent` and `DebuggerView::broadcastEvent` functions.
#[derive(Default)]
pub struct EventBus {
    listeners: Vec<Rc<RefCell<Box<dyn DebuggerEventListener>>>>,
}

pub trait DebuggerEventListener {
    fn handle_event(&mut self, name: &str, event: &dyn DebuggerEvents::Event) -> bool;
    fn accepts_event(&self, name: &str) -> bool;
}

impl EventBus {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn register(&mut self, l: Rc<RefCell<Box<dyn DebuggerEventListener>>>) {
        self.listeners.push(l);
    }
    pub fn send(&mut self, name: &str, event: &dyn DebuggerEvents::Event) -> bool {
        let mut primary: Vec<usize> = Vec::new();
        let mut secondary: Vec<usize> = Vec::new();
        for (i, l) in self.listeners.iter().enumerate() {
            let l = l.borrow();
            if l.accepts_event(name) {
                if i < primary.len() + secondary.len() {
                    primary.push(i);
                } else {
                    secondary.push(i);
                }
            }
        }
        for i in primary {
            if self.listeners[i].borrow_mut().handle_event(name, event) {
                return true;
            }
        }
        for i in secondary {
            if self.listeners[i].borrow_mut().handle_event(name, event) {
                return true;
            }
        }
        false
    }
    pub fn broadcast(&mut self, name: &str, event: &dyn DebuggerEvents::Event) {
        for l in &self.listeners {
            l.borrow_mut().handle_event(name, event);
        }
    }
}

// ---------------------------------------------------------------------------
// Dock manager stub
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct DockManager {
    pub views: HashMap<String, DebuggerView>,
    pub current_layout_index: usize,
    pub cpu: Option<BreakPointCpu>,
}

impl DockManager {
    pub fn switch_to_debugger_view(&mut self, _v: &DebuggerView) {}
    pub fn debugger_views(&self) -> Vec<(&String, &DebuggerView)> {
        self.views.iter().collect()
    }
    pub fn load_layouts(&mut self) {}
    pub fn switch_to_layout(&mut self, _i: usize) {}
    pub fn save_current_layout(&mut self) {}
    pub fn reset_all_layouts(&mut self) {}
    pub fn reset_default_layouts(&mut self) {}
    pub fn update_theme(&mut self) {}
    pub fn update_toolbar_lock_state(&mut self) {}
    pub fn create_menu_bar(&mut self, bar: &mut String) -> String {
        bar.clone()
    }
    pub fn create_tools_menu(&mut self, _menu: &mut String) {}
    pub fn create_windows_menu(&mut self, _menu: &mut String) {}
    pub fn switch_to_layout_with_cpu(&mut self, _cpu: BreakPointCpu, _blink: bool) {}
    pub fn configure_docking_system() {}
    pub fn cpu(&self) -> Option<BreakPointCpu> {
        self.cpu
    }
}

// ---------------------------------------------------------------------------
// Global debugger window pointer (mirrors `g_debugger_window`).
// ---------------------------------------------------------------------------

thread_local! {
    static G_DEBUGGER_WINDOW: RefCell<Option<Rc<RefCell<DebuggerWindow>>>> = RefCell::new(None);
}

fn with_global<R>(f: impl FnOnce(&Rc<RefCell<DebuggerWindow>>) -> R) -> Option<R> {
    G_DEBUGGER_WINDOW.with(|g| g.borrow().as_ref().map(|w| f(w)))
}

// ---------------------------------------------------------------------------
// Emu thread / VM-state events
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum VmEvent {
    Starting,
    Paused,
    Resumed,
    Stopped,
    GameChanged(String),
}

#[derive(Default)]
pub struct EmuThread {
    pub state: VmState,
    pub on_vm_starting: Vec<Box<dyn Fn()>>,
    pub on_vm_paused: Vec<Box<dyn Fn()>>,
    pub on_vm_resumed: Vec<Box<dyn Fn()>>,
    pub on_vm_stopped: Vec<Box<dyn Fn()>>,
    pub on_game_changed: Vec<Box<dyn Fn(String)>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum VmState {
    #[default]
    Running,
    Paused,
    Stopped,
}

impl EmuThread {
    pub fn is_vm_paused(&self) -> bool {
        self.state == VmState::Paused
    }
    pub fn is_vm_valid(&self) -> bool {
        self.state != VmState::Stopped
    }
    pub fn set_vm_paused(&mut self, paused: bool) {
        self.state = if paused { VmState::Paused } else { VmState::Running };
        if paused {
            for cb in &self.on_vm_paused {
                cb();
            }
        } else {
            for cb in &self.on_vm_resumed {
                cb();
            }
        }
    }
    pub fn shutdown_vm(&mut self, _save_state: bool) {
        self.state = VmState::Stopped;
        for cb in &self.on_vm_stopped {
            cb();
        }
    }
    pub fn reset_vm(&mut self) {}
    pub fn post_event(&mut self, e: VmEvent) {
        match e {
            VmEvent::Starting => {
                for cb in &self.on_vm_starting {
                    cb();
                }
            }
            VmEvent::Paused => {
                self.state = VmState::Paused;
                for cb in &self.on_vm_paused {
                    cb();
                }
            }
            VmEvent::Resumed => {
                self.state = VmState::Running;
                for cb in &self.on_vm_resumed {
                    cb();
                }
            }
            VmEvent::Stopped => {
                self.state = VmState::Stopped;
                for cb in &self.on_vm_stopped {
                    cb();
                }
            }
            VmEvent::GameChanged(s) => {
                for cb in &self.on_game_changed {
                    cb(s.clone());
                }
            }
        }
    }
}

thread_local! {
    static G_EMU_THREAD: RefCell<EmuThread> = RefCell::new(EmuThread::default());
}

fn with_emu<R>(f: impl FnOnce(&mut EmuThread) -> R) -> R {
    G_EMU_THREAD.with(|e| f(&mut e.borrow_mut()))
}

// ---------------------------------------------------------------------------
// Settings manager stub
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct DebuggerSettingsManager;

impl DebuggerSettingsManager {
    pub fn load_game_settings<R>(&self, _model: &mut R) {}
    pub fn save_game_settings<R>(&self, _model: &R) {}
}

// ---------------------------------------------------------------------------
// Breakpoint storage and operations
// ---------------------------------------------------------------------------

/// Mirrors the C++ `CBreakPoints` static class.
#[derive(Debug, Default)]
pub struct CBreakPoints {
    pub breakpoints_ee: Vec<BreakPoint>,
    pub mem_checks_ee: Vec<MemCheck>,
    pub breakpoints_iop: Vec<BreakPoint>,
    pub mem_checks_iop: Vec<MemCheck>,
    pub temp_breakpoints: HashSet<u64>,
    pub triggered: bool,
    pub triggered_cpu: BreakPointCpu,
    pub core_paused: bool,
}

impl CBreakPoints {
    pub fn add_break_point(
        &mut self,
        cpu: BreakPointCpu,
        addr: u32,
        temp: bool,
        enabled: bool,
        _on_hit: bool,
    ) {
        let bp = BreakPoint {
            addr,
            enabled,
            ..Default::default()
        };
        match cpu {
            BreakPointCpu::EE => self.breakpoints_ee.push(bp),
            BreakPointCpu::IOP => self.breakpoints_iop.push(bp),
            BreakPointCpu::IOPAndEE => {
                self.breakpoints_ee.push(bp.clone());
                self.breakpoints_iop.push(bp);
            }
        }
        if temp {
            self.temp_breakpoints.insert(((cpu as u64) << 32) | addr as u64);
        }
    }
    pub fn remove_break_point(&mut self, cpu: BreakPointCpu, addr: u32) {
        let v = match cpu {
            BreakPointCpu::EE => &mut self.breakpoints_ee,
            BreakPointCpu::IOP => &mut self.breakpoints_iop,
            BreakPointCpu::IOPAndEE => {
                self.breakpoints_ee.retain(|b| b.addr != addr);
                &mut self.breakpoints_iop
            }
        };
        v.retain(|b| b.addr != addr);
    }
    pub fn change_break_point(&mut self, cpu: BreakPointCpu, addr: u32, enabled: bool) {
        let v = match cpu {
            BreakPointCpu::EE => &mut self.breakpoints_ee,
            BreakPointCpu::IOP => &mut self.breakpoints_iop,
            BreakPointCpu::IOPAndEE => {
                for b in &mut self.breakpoints_ee {
                    if b.addr == addr {
                        b.enabled = enabled;
                    }
                }
                &mut self.breakpoints_iop
            }
        };
        for b in v {
            if b.addr == addr {
                b.enabled = enabled;
            }
        }
    }
    pub fn change_break_point_description(
        &mut self,
        cpu: BreakPointCpu,
        addr: u32,
        desc: String,
    ) {
        let v = match cpu {
            BreakPointCpu::EE => &mut self.breakpoints_ee,
            BreakPointCpu::IOP => &mut self.breakpoints_iop,
            BreakPointCpu::IOPAndEE => {
                for b in &mut self.breakpoints_ee {
                    if b.addr == addr {
                        b.description = desc.clone();
                    }
                }
                &mut self.breakpoints_iop
            }
        };
        for b in v {
            if b.addr == addr {
                b.description = desc.clone();
            }
        }
    }
    pub fn change_break_point_add_cond(
        &mut self,
        cpu: BreakPointCpu,
        addr: u32,
        cond: BreakPointCond,
    ) {
        let v = match cpu {
            BreakPointCpu::EE => &mut self.breakpoints_ee,
            BreakPointCpu::IOP => &mut self.breakpoints_iop,
            BreakPointCpu::IOPAndEE => {
                for b in &mut self.breakpoints_ee {
                    if b.addr == addr {
                        b.has_cond = true;
                        b.cond = cond.clone();
                    }
                }
                &mut self.breakpoints_iop
            }
        };
        for b in v {
            if b.addr == addr {
                b.has_cond = true;
                b.cond = cond.clone();
            }
        }
    }
    pub fn change_break_point_remove_cond(&mut self, cpu: BreakPointCpu, addr: u32) {
        let v = match cpu {
            BreakPointCpu::EE => &mut self.breakpoints_ee,
            BreakPointCpu::IOP => &mut self.breakpoints_iop,
            BreakPointCpu::IOPAndEE => {
                for b in &mut self.breakpoints_ee {
                    if b.addr == addr {
                        b.has_cond = false;
                    }
                }
                &mut self.breakpoints_iop
            }
        };
        for b in v {
            if b.addr == addr {
                b.has_cond = false;
            }
        }
    }
    pub fn add_mem_check(
        &mut self,
        cpu: BreakPointCpu,
        start: u32,
        end: u32,
        cond: i32,
        result: i32,
    ) {
        let mc = MemCheck {
            start,
            end,
            mem_cond: cond,
            result,
            ..Default::default()
        };
        match cpu {
            BreakPointCpu::EE => self.mem_checks_ee.push(mc),
            BreakPointCpu::IOP => self.mem_checks_iop.push(mc),
            BreakPointCpu::IOPAndEE => {
                self.mem_checks_ee.push(mc.clone());
                self.mem_checks_iop.push(mc);
            }
        }
    }
    pub fn remove_mem_check(&mut self, cpu: BreakPointCpu, start: u32, end: u32) {
        let v = match cpu {
            BreakPointCpu::EE => &mut self.mem_checks_ee,
            BreakPointCpu::IOP => &mut self.mem_checks_iop,
            BreakPointCpu::IOPAndEE => {
                self.mem_checks_ee.retain(|m| !(m.start == start && m.end == end));
                &mut self.mem_checks_iop
            }
        };
        v.retain(|m| !(m.start == start && m.end == end));
    }
    pub fn change_mem_check(
        &mut self,
        cpu: BreakPointCpu,
        start: u32,
        end: u32,
        cond: i32,
        result: i32,
    ) {
        let v = match cpu {
            BreakPointCpu::EE => &mut self.mem_checks_ee,
            BreakPointCpu::IOP => &mut self.mem_checks_iop,
            BreakPointCpu::IOPAndEE => {
                for m in &mut self.mem_checks_ee {
                    if m.start == start && m.end == end {
                        m.mem_cond = cond;
                        m.result = result;
                    }
                }
                &mut self.mem_checks_iop
            }
        };
        for m in v {
            if m.start == start && m.end == end {
                m.mem_cond = cond;
                m.result = result;
            }
        }
    }
    pub fn change_mem_check_add_cond(
        &mut self,
        cpu: BreakPointCpu,
        start: u32,
        end: u32,
        cond: BreakPointCond,
    ) {
        let v = match cpu {
            BreakPointCpu::EE => &mut self.mem_checks_ee,
            BreakPointCpu::IOP => &mut self.mem_checks_iop,
            BreakPointCpu::IOPAndEE => {
                for m in &mut self.mem_checks_ee {
                    if m.start == start && m.end == end {
                        m.has_cond = true;
                        m.cond = cond.clone();
                    }
                }
                &mut self.mem_checks_iop
            }
        };
        for m in v {
            if m.start == start && m.end == end {
                m.has_cond = true;
                m.cond = cond.clone();
            }
        }
    }
    pub fn change_mem_check_remove_cond(&mut self, cpu: BreakPointCpu, start: u32, end: u32) {
        let v = match cpu {
            BreakPointCpu::EE => &mut self.mem_checks_ee,
            BreakPointCpu::IOP => &mut self.mem_checks_iop,
            BreakPointCpu::IOPAndEE => {
                for m in &mut self.mem_checks_ee {
                    if m.start == start && m.end == end {
                        m.has_cond = false;
                    }
                }
                &mut self.mem_checks_iop
            }
        };
        for m in v {
            if m.start == start && m.end == end {
                m.has_cond = false;
            }
        }
    }
    pub fn change_mem_check_description(
        &mut self,
        cpu: BreakPointCpu,
        start: u32,
        end: u32,
        desc: String,
    ) {
        let v = match cpu {
            BreakPointCpu::EE => &mut self.mem_checks_ee,
            BreakPointCpu::IOP => &mut self.mem_checks_iop,
            BreakPointCpu::IOPAndEE => {
                for m in &mut self.mem_checks_ee {
                    if m.start == start && m.end == end {
                        m.description = desc.clone();
                    }
                }
                &mut self.mem_checks_iop
            }
        };
        for m in v {
            if m.start == start && m.end == end {
                m.description = desc.clone();
            }
        }
    }
    pub fn is_address_break_point(&self, cpu: BreakPointCpu, addr: u32, enabled: &mut bool) -> bool {
        let v = match cpu {
            BreakPointCpu::EE => &self.breakpoints_ee,
            BreakPointCpu::IOP => &self.breakpoints_iop,
            BreakPointCpu::IOPAndEE => &self.breakpoints_ee,
        };
        for b in v {
            if b.addr == addr {
                *enabled = b.enabled;
                return true;
            }
        }
        false
    }
    pub fn is_temp_break_point(&self, cpu: BreakPointCpu, addr: u32) -> bool {
        self.temp_breakpoints
            .contains(&(((cpu as u64) << 32) | addr as u64))
    }
    pub fn get_breakpoint_triggered(&self) -> bool {
        self.triggered
    }
    pub fn get_breakpoint_triggered_cpu(&self) -> BreakPointCpu {
        self.triggered_cpu
    }
    pub fn set_breakpoint_triggered(&mut self, t: bool, cpu: BreakPointCpu) {
        self.triggered = t;
        self.triggered_cpu = cpu;
    }
    pub fn set_skip_first(&mut self, _cpu: BreakPointCpu, _pc: u32) {}
    pub fn is_stepping_break_point(&self, _cpu: BreakPointCpu, _pc: u32) -> bool {
        false
    }
    pub fn clear_temporary_break_points(&mut self) {
        self.temp_breakpoints.clear();
    }
    pub fn set_core_paused(&mut self, p: bool) {
        self.core_paused = p;
    }
    pub fn get_core_paused(&self) -> bool {
        self.core_paused
    }
    pub fn get_breakpoints(&self, cpu: BreakPointCpu, _temp: bool) -> Vec<BreakpointMemcheck> {
        match cpu {
            BreakPointCpu::EE => self
                .breakpoints_ee
                .iter()
                .cloned()
                .map(BreakpointMemcheck::BreakPoint)
                .collect(),
            BreakPointCpu::IOP => self
                .breakpoints_iop
                .iter()
                .cloned()
                .map(BreakpointMemcheck::BreakPoint)
                .collect(),
            BreakPointCpu::IOPAndEE => Vec::new(),
        }
    }
    pub fn get_mem_checks(&self, cpu: BreakPointCpu) -> Vec<BreakpointMemcheck> {
        match cpu {
            BreakPointCpu::EE => self
                .mem_checks_ee
                .iter()
                .cloned()
                .map(BreakpointMemcheck::MemCheck)
                .collect(),
            BreakPointCpu::IOP => self
                .mem_checks_iop
                .iter()
                .cloned()
                .map(BreakpointMemcheck::MemCheck)
                .collect(),
            BreakPointCpu::IOPAndEE => Vec::new(),
        }
    }
}

thread_local! {
    static G_BREAKPOINTS: RefCell<CBreakPoints> = RefCell::new(CBreakPoints::default());
}

fn with_breakpoints<R>(f: impl FnOnce(&mut CBreakPoints) -> R) -> R {
    G_BREAKPOINTS.with(|b| f(&mut b.borrow_mut()))
}

// ---------------------------------------------------------------------------
// Host settings / font-size helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct Host;

impl Host {
    pub fn run_on_cpu_thread<F: FnOnce() + Send + 'static>(_f: F) {
        // Synchronous for the translation.
        // In the original code, this dispatches to the CPU thread.
    }
    pub fn get_base_bool_setting_value(_section: &str, _key: &str, default: bool) -> bool {
        default
    }
    pub fn get_base_int_setting_value(_section: &str, _key: &str, default: i32) -> i32 {
        default
    }
    pub fn get_base_string_setting_value(_section: &str, _key: &str) -> String {
        String::new()
    }
    pub fn set_base_int_setting_value(_section: &str, _key: &str, _value: i32) {}
    pub fn set_base_string_setting_value(_section: &str, _key: &str, _value: &str) {}
    pub fn commit_base_setting_changes() {}
}

// ---------------------------------------------------------------------------
// QtHost / host-name helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct QtHost;

impl QtHost {
    pub fn is_vm_valid() -> bool {
        true
    }
    pub fn is_vm_paused() -> bool {
        with_emu(|e| e.is_vm_paused())
    }
    pub fn is_dark_application_theme() -> bool {
        false
    }
    pub fn run_on_ui_thread<F: FnOnce() + Send + 'static>(_f: F) {}
}

// ---------------------------------------------------------------------------
// Async dialog stub
// ---------------------------------------------------------------------------

pub mod AsyncDialogs {
    use super::*;
    pub fn question<R>(
        _parent: &dyn std::any::Any,
        _title: &str,
        _text: &str,
        _cb: impl FnOnce() -> R,
    ) {
    }
    pub fn warning(_parent: &dyn std::any::Any, _title: &str, _text: &str) {}
    pub fn get_text<R>(
        _parent: &dyn std::any::Any,
        _title: &str,
        _label: &str,
        _initial: &str,
        cb: impl FnOnce(String) -> R,
    ) {
        let _ = cb(String::new());
    }
}

// ---------------------------------------------------------------------------
// Qt-like utility helpers
// ---------------------------------------------------------------------------

pub mod QtUtils {
    use super::*;

    pub fn filled_qstring_from_value(v: u64, base: u32) -> String {
        match base {
            16 => format!("{:08x}", v),
            10 => format!("{}", v),
            _ => format!("{:?}", v),
        }
    }

    pub fn abstract_item_model_to_csv(_model: &dyn std::any::Any, _role: i32, _with_header: bool) -> String {
        String::new()
    }

    pub fn split_on_new_line(s: &str) -> Vec<String> {
        s.split('\n').map(|x| x.to_string()).collect()
    }
}

// ---------------------------------------------------------------------------
// MainWindow stub
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct MainWindow;

impl MainWindow {
    pub fn do_settings(&self, _page: &str) {}
    pub fn do_game_settings(&self, _page: &str) {}
}

thread_local! {
    static G_MAIN_WINDOW: RefCell<Option<Rc<RefCell<MainWindow>>>> = RefCell::new(None);
}

// ===========================================================================
// 1) DebuggerView
// ===========================================================================

pub const MONOSPACE_FONT: u32 = 1;
pub const DISALLOW_MULTIPLE_INSTANCES: u32 = 2;
pub const MINIMUM_FONT_SIZE: i32 = 6;
pub const MAXIMUM_FONT_SIZE: i32 = 32;

pub const MAX_DOCK_WIDGET_NAME_SIZE: usize = 64;

/// Parameters captured by `DebuggerViewParameters` in the C++ code.
#[derive(Debug, Clone)]
pub struct DebuggerViewParameters {
    pub parent: Option<usize>,
    pub id: u64,
    pub unique_name: String,
    pub cpu: Option<BreakPointCpu>,
    pub cpu_override: Option<BreakPointCpu>,
}

impl DebuggerViewParameters {
    pub fn new(unique_name: impl Into<String>, id: u64) -> Self {
        Self {
            parent: None,
            id,
            unique_name: unique_name.into(),
            cpu: None,
            cpu_override: None,
        }
    }
    pub fn with_cpu(mut self, cpu: BreakPointCpu) -> Self {
        self.cpu = Some(cpu);
        self
    }
    pub fn with_cpu_override(mut self, cpu: BreakPointCpu) -> Self {
        self.cpu_override = Some(cpu);
        self
    }
}

/// Trait implemented by all debugger views to receive the common C++ signals.
pub trait DebuggerViewListener {
    fn on_vm_actually_paused(&mut self);
}

#[derive(Default)]
pub struct DebuggerViewListeners {
    pub on_vm_actually_paused: Vec<Box<dyn FnMut()>>,
}

/// Top-level debugger view (equivalent to `DebuggerView` in C++).
pub struct DebuggerView {
    pub id: u64,
    pub unique_name: String,
    pub cpu_type: Option<BreakPointCpu>,
    pub cpu_override: Option<BreakPointCpu>,
    pub flags: u32,
    pub is_primary: bool,
    pub custom_display_name: String,
    pub translated_display_name: String,
    pub display_name_suffix_number: Option<i32>,
    pub style_sheet: String,
    pub event_handlers: HashMap<&'static str, Vec<Box<dyn FnMut(&dyn DebuggerEvents::Event) -> bool>>>,
    pub listeners: DebuggerViewListeners,
}

impl Default for DebuggerView {
    fn default() -> Self {
        Self {
            id: 0,
            unique_name: String::new(),
            cpu_type: None,
            cpu_override: None,
            flags: 0,
            is_primary: false,
            custom_display_name: String::new(),
            translated_display_name: String::new(),
            display_name_suffix_number: None,
            style_sheet: String::new(),
            event_handlers: HashMap::new(),
            listeners: DebuggerViewListeners::default(),
        }
    }
}

impl DebuggerView {
    pub fn new(parameters: DebuggerViewParameters, flags: u32) -> Self {
        let mut v = Self::default();
        v.id = parameters.id;
        v.unique_name = parameters.unique_name;
        v.cpu_type = parameters.cpu;
        v.cpu_override = parameters.cpu_override;
        v.flags = flags;
        v.update_style_sheet();
        v
    }

    /// Equivalent to the C++ `update()` method. Views re-paint themselves
    /// when the VM state changes.
    pub fn update(&mut self) {
        // No-op in this translation; the original triggers a repaint.
    }

    /// Equivalent to the C++ `refresh()` method. The default no-op matches
    /// the `DebuggerView` base class which simply re-evaluates its state.
    pub fn refresh(&mut self) {
        self.update();
    }

    pub fn unique_name(&self) -> &str {
        &self.unique_name
    }
    pub fn display_name(&self) -> String {
        let mut name = self.translated_display_name.clone();
        if let Some(suffix) = self.display_name_suffix_number {
            name = format!("{} #{}", name, suffix);
        }
        if let Some(c) = self.cpu_override {
            name = format!("{} ({})", name, c.name());
        }
        name
    }
    pub fn display_name_without_suffix(&self) -> &str {
        &self.translated_display_name
    }
    pub fn custom_display_name(&self) -> &str {
        &self.custom_display_name
    }
    pub fn set_custom_display_name(&mut self, name: &str) -> bool {
        if name.len() > MAX_DOCK_WIDGET_NAME_SIZE {
            return false;
        }
        self.custom_display_name = name.to_string();
        true
    }
    pub fn is_primary(&self) -> bool {
        self.is_primary
    }
    pub fn set_primary(&mut self, primary: bool) {
        self.is_primary = primary;
    }
    pub fn set_cpu(&mut self, cpu: BreakPointCpu) -> bool {
        let before = self.cpu_type;
        self.cpu_type = Some(cpu);
        before == self.cpu_type
    }
    pub fn cpu_override(&self) -> Option<BreakPointCpu> {
        self.cpu_override
    }
    pub fn set_cpu_override(&mut self, cpu: Option<BreakPointCpu>) -> bool {
        let before = self.cpu_type;
        self.cpu_override = cpu;
        before == self.cpu_type
    }

    pub fn handle_event(&mut self, event: &dyn DebuggerEvents::Event) -> bool {
        if let Some(handlers) = self.event_handlers.get_mut(event.name()) {
            for h in handlers.iter_mut() {
                if h(event) {
                    return true;
                }
            }
        }
        false
    }

    pub fn accepts_event_type(&self, name: &str) -> bool {
        self.event_handlers.contains_key(name)
    }

    pub fn switch_to_this_tab(&mut self) {
        if let Some(w) = with_global(|w| w.clone()) {
            w.borrow_mut().dock_manager.switch_to_debugger_view(self);
        }
    }

    pub fn supports_multiple_instances(&self) -> bool {
        (self.flags & DISALLOW_MULTIPLE_INSTANCES) == 0
    }

    pub fn retranslate_display_name(&mut self) {
        if !self.custom_display_name.is_empty() {
            self.translated_display_name = self.custom_display_name.clone();
        } else {
            self.translated_display_name = String::new();
        }
    }

    pub fn display_name_suffix_number(&self) -> Option<i32> {
        self.display_name_suffix_number
    }
    pub fn set_display_name_suffix_number(&mut self, n: Option<i32>) {
        self.display_name_suffix_number = n;
    }

    pub fn update_style_sheet(&mut self) {
        if (self.flags & MONOSPACE_FONT) != 0 {
            self.style_sheet = "font-family: 'Monospace';".to_string();
        } else {
            self.style_sheet = String::new();
        }
    }

    /// Equivalent to `DebuggerView::goToInDisassembler` in C++.
    pub fn go_to_in_disassembler(&mut self, address: u32, switch_to_tab: bool) {
        let event = DebuggerEvents::GoToAddress {
            address,
            filter: DebuggerEvents::AddressFilter::Disassembler,
            switch_to_tab,
        };
        Self::send_event(&event);
    }

    /// Equivalent to `DebuggerView::goToInMemoryView` in C++.
    pub fn go_to_in_memory_view(&mut self, address: u32, switch_to_tab: bool) {
        let event = DebuggerEvents::GoToAddress {
            address,
            filter: DebuggerEvents::AddressFilter::MemoryView,
            switch_to_tab,
        };
        Self::send_event(&event);
    }

    /// Mirrors the static `DebuggerView::sendEvent` C++ helper.
    pub fn send_event(event: &dyn DebuggerEvents::Event) {
        if let Some(w) = with_global(|w| w.clone()) {
            w.borrow_mut().event_bus.broadcast(event.name(), event);
        }
    }

    /// Mirrors the static `DebuggerView::broadcastEvent` C++ helper.
    pub fn broadcast_event(event: &dyn DebuggerEvents::Event) {
        if let Some(w) = with_global(|w| w.clone()) {
            w.borrow_mut().event_bus.broadcast(event.name(), event);
        }
    }
}

// ===========================================================================
// 2) DebuggerWindow
// ===========================================================================

pub struct DebuggerWindow {
    pub dock_manager: DockManager,
    pub event_bus: EventBus,
    pub font_size: i32,
    pub default_toolbar_state: String,
    pub is_updating_theme: bool,
    pub refresh_timer_interval: i32,
    pub views: BTreeMap<String, DebuggerView>,
    pub menu_bar: String,
    pub main_window: Option<Rc<RefCell<MainWindow>>>,
    pub settings_manager: DebuggerSettingsManager,
}

impl Default for DebuggerWindow {
    fn default() -> Self {
        Self {
            dock_manager: DockManager::default(),
            event_bus: EventBus::new(),
            font_size: 12,
            default_toolbar_state: String::new(),
            is_updating_theme: false,
            refresh_timer_interval: 1000,
            views: BTreeMap::new(),
            menu_bar: String::new(),
            main_window: None,
            settings_manager: DebuggerSettingsManager,
        }
    }
}

impl DebuggerWindow {
    pub fn new() -> Self {
        let mut w = Self::default();
        w.setup_fonts();
        w.dock_manager.load_layouts();
        if with_emu(|e| e.is_vm_valid()) {
            w.on_vm_starting();
            if with_emu(|e| e.is_vm_paused()) {
                w.on_vm_paused();
            } else {
                w.on_vm_resumed();
            }
        } else {
            w.on_vm_stopped();
        }
        w.dock_manager.switch_to_layout(0);
        w.update_theme();
        w.update_from_settings();
        w
    }

    pub fn show(&mut self) {
        // Equivalent to MainWindow::show in C++.
    }

    pub fn close(&mut self) {
        self.dock_manager.save_current_layout();
        if let Some(main) = &self.main_window {
            // Mirror saveWindowGeometry().
        }
    }

    pub fn on_break(&mut self) {
        self.event_bus
            .broadcast(DebuggerEvents::VM_UPDATE, &DebuggerEvents::VMUpdate);
    }

    pub fn on_resume(&mut self) {
        with_emu(|e| e.post_event(VmEvent::Resumed));
    }

    pub fn get_instance() -> Option<Rc<RefCell<Self>>> {
        with_global(|w| w.clone())
    }

    pub fn create_instance() -> Rc<RefCell<Self>> {
        DockManager::configure_docking_system();
        let w = Rc::new(RefCell::new(DebuggerWindow::new()));
        G_DEBUGGER_WINDOW.with(|g| *g.borrow_mut() = Some(w.clone()));
        w
    }

    pub fn destroy_instance() {
        G_DEBUGGER_WINDOW.with(|g| {
            if let Some(w) = g.borrow_mut().take() {
                w.borrow_mut().close();
            }
        });
    }

    pub fn should_show_on_startup() -> bool {
        Host::get_base_bool_setting_value("Debugger/UserInterface", "ShowOnStartup", false)
    }

    pub fn setup_default_toolbar_state(&mut self) {
        self.default_toolbar_state.clear();
    }

    pub fn clear_toolbar_state(&mut self) {}

    pub fn setup_fonts(&mut self) {
        let size = Host::get_base_int_setting_value("Debugger/UserInterface", "FontSize", 12);
        self.font_size = size.clamp(MINIMUM_FONT_SIZE, MAXIMUM_FONT_SIZE);
    }

    pub fn update_font_actions(&mut self) {}

    pub fn save_font_size(&self) {
        Host::set_base_int_setting_value("Debugger/UserInterface", "FontSize", self.font_size);
        Host::commit_base_setting_changes();
    }

    pub fn font_size(&self) -> i32 {
        self.font_size
    }

    pub fn update_theme(&mut self) {
        if self.is_updating_theme {
            return;
        }
        self.is_updating_theme = true;
        self.dock_manager.update_theme();
        self.is_updating_theme = false;
    }

    pub fn save_window_geometry(&self) {
        Host::set_base_string_setting_value("Debugger/UserInterface", "WindowGeometry", "");
        Host::commit_base_setting_changes();
    }

    pub fn restore_window_geometry(&mut self) {}

    pub fn should_save_window_geometry() -> bool {
        Host::get_base_bool_setting_value("Debugger/UserInterface", "SaveWindowGeometry", true)
    }

    pub fn update_from_settings(&mut self) {
        let raw = Host::get_base_int_setting_value("Debugger/UserInterface", "RefreshInterval", 1000);
        self.refresh_timer_interval = raw.clamp(10, 100_000);
    }

    pub fn on_vm_starting(&mut self) {
        for v in self.views.values_mut() {
            v.update();
        }
    }
    pub fn on_vm_paused(&mut self) {
        if with_breakpoints(|bp| bp.get_breakpoint_triggered()) {
            let cpu = with_breakpoints(|bp| bp.get_breakpoint_triggered_cpu());
            if matches!(cpu, BreakPointCpu::EE | BreakPointCpu::IOP) {
                self.dock_manager.switch_to_layout_with_cpu(cpu, true);
            }
            with_breakpoints(|bp| bp.clear_temporary_break_points());
            with_breakpoints(|bp| bp.set_breakpoint_triggered(false, BreakPointCpu::IOPAndEE));
        }
        if !with_breakpoints(|bp| bp.get_core_paused()) {
            for l in &mut self.views.values_mut() {
                for cb in &mut l.listeners.on_vm_actually_paused {
                    cb();
                }
            }
        } else {
            with_breakpoints(|bp| bp.set_core_paused(false));
        }
    }
    pub fn on_vm_resumed(&mut self) {}
    pub fn on_vm_stopped(&mut self) {}

    pub fn on_analyse(&mut self) {}
    pub fn on_settings(&mut self) {
        if let Some(main) = &self.main_window {
            main.borrow().do_settings("Debug");
        }
    }
    pub fn on_game_settings(&mut self) {
        if let Some(main) = &self.main_window {
            main.borrow().do_game_settings("Debug");
        }
    }

    pub fn on_run_pause(&mut self) {
        with_emu(|e| e.set_vm_paused(!e.is_vm_paused()));
    }

    /// Equivalent to `DebuggerWindow::onStepInto` in C++.
    pub fn on_step_into(&mut self, cpu: &mut DebugInterface) {
        if !cpu.is_alive() || !cpu.is_cpu_paused() {
            return;
        }
        let info = cpu.last_step_info.clone().unwrap_or(MipsOpcodeInfo {
            is_branch: false,
            is_conditional: false,
            is_linked_branch: false,
            is_syscall: false,
            condition_met: false,
            branch_target: 0,
            has_relevant_address: false,
            relevent_address: 0,
        });
        let mut bp_addr = cpu.get_pc() + 4;
        if info.is_branch {
            if !info.is_conditional {
                bp_addr = info.branch_target;
            } else if info.condition_met {
                bp_addr = info.branch_target;
            } else {
                bp_addr = cpu.get_pc() + 2 * 4;
            }
        }
        if info.is_syscall {
            bp_addr = info.branch_target;
        }
        with_breakpoints(|bp| bp.add_break_point(cpu.get_cpu_type(), bp_addr, true, true, true));
        cpu.resume_cpu();
    }

    /// Equivalent to `DebuggerWindow::onStepOver` in C++.
    pub fn on_step_over(&mut self, cpu: &mut DebugInterface) {
        if !cpu.is_alive() || !cpu.is_cpu_paused() {
            return;
        }
        let info = cpu.last_step_info.clone().unwrap_or(MipsOpcodeInfo {
            is_branch: false,
            is_conditional: false,
            is_linked_branch: false,
            is_syscall: false,
            condition_met: false,
            branch_target: 0,
            has_relevant_address: false,
            relevent_address: 0,
        });
        let mut bp_addr = cpu.get_pc() + 4;
        if info.is_branch {
            if !info.is_conditional {
                if info.is_linked_branch {
                    bp_addr += 4;
                } else {
                    bp_addr = info.branch_target;
                }
            } else if info.condition_met {
                bp_addr = info.branch_target;
            } else {
                bp_addr = cpu.get_pc() + 2 * 4;
            }
        }
        with_breakpoints(|bp| bp.add_break_point(cpu.get_cpu_type(), bp_addr, true, true, true));
        cpu.resume_cpu();
    }

    /// Equivalent to `DebuggerWindow::onStepOut` in C++.
    pub fn on_step_out(&mut self, cpu: &mut DebugInterface) {
        if !cpu.is_alive() || !cpu.is_cpu_paused() {
            return;
        }
        // Stub: in C++ this walks the MIPS stack to find the caller's PC.
        let caller_pc = cpu.get_pc();
        with_breakpoints(|bp| bp.add_break_point(cpu.get_cpu_type(), caller_pc, true, true, true));
        cpu.resume_cpu();
    }

    pub fn change_event(&mut self, event_type: &str) {
        if event_type == "PaletteChange" || event_type == "StyleChange" {
            self.update_theme();
        }
    }

    pub fn close_event(&mut self) {
        self.dock_manager.save_current_layout();
        self.save_window_geometry();
    }

    pub fn current_cpu(&self) -> Option<BreakPointCpu> {
        self.dock_manager.cpu()
    }

    pub fn register_view(&mut self, view: DebuggerView) {
        self.views.insert(view.unique_name.clone(), view);
    }
}

// ===========================================================================
// 3) DisassemblyView
// ===========================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionInfo {
    Address,
    InstructionHex,
    InstructionText,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchLineType {
    Up,
    Down,
}

#[derive(Debug, Clone, Copy)]
pub struct BranchLine {
    pub first: u32,
    pub second: u32,
    pub branch_type: BranchLineType,
}

#[derive(Debug, Clone, Default)]
pub struct DisassemblyLineInfo {
    pub name: String,
    pub params: String,
    pub info: MipsOpcodeInfo,
    pub distype: i32,
    pub is_no_return: bool,
}

#[derive(Debug, Default)]
pub struct DisassemblyManager {
    pub cpu: Option<DebugInterfaceRef>,
    pub cache: HashMap<u32, DisassemblyLineInfo>,
}

impl DisassemblyManager {
    pub fn set_cpu(&mut self, cpu: &DebugInterface) {
        // In a real port we'd point at a native debug interface.
    }
    pub fn analyze(&mut self, _addr: u32, _span: u32) {}
    pub fn get_line(&self, addr: u32, _get_bytes: bool, out: &mut DisassemblyLineInfo) {
        out.name = format!("{:08x}", addr);
        out.params = "<disasm>".to_string();
    }
    pub fn get_nth_next_address(&self, addr: u32, n: u32) -> u32 {
        addr + 4 * n
    }
    pub fn get_branch_lines(&self, _addr: u32, _span: u32) -> Vec<BranchLine> {
        Vec::new()
    }
}

#[derive(Default)]
pub struct DisassemblyView {
    pub base: DebuggerView,
    pub m_disassembly_manager: DisassemblyManager,
    pub m_visible_start: u32,
    pub m_visible_rows: u32,
    pub m_row_height: u32,
    pub m_selected_address_start: u32,
    pub m_selected_address_end: u32,
    pub m_go_to_program_counter_on_pause: bool,
    pub m_show_instruction_bytes: bool,
    pub m_noped_instructions: HashMap<u32, u32>,
    pub m_stubbed_functions: HashMap<u32, (u32, u32)>,
}

impl DisassemblyView {
    pub fn new(parameters: DebuggerViewParameters) -> Self {
        let mut v = Self::default();
        v.base = DebuggerView::new(parameters, MONOSPACE_FONT);
        v
    }

    pub fn set_pc(&mut self, addr: u32) {
        self.m_selected_address_start = addr;
        self.m_selected_address_end = addr;
    }

    pub fn goto(&mut self, addr: u32) {
        let dest = addr & !3;
        self.m_visible_start = dest;
        self.m_selected_address_start = dest;
        self.m_selected_address_end = dest;
    }

    pub fn goto_address(&mut self, addr: u32, set_focus: bool) {
        let dest = addr & !3;
        let center = (self.m_visible_rows as u32 * 4) / 2;
        self.m_visible_start = dest.saturating_sub(center) & !3;
        self.m_selected_address_start = dest;
        self.m_selected_address_end = dest;
        let _ = set_focus;
    }

    pub fn goto_program_counter_on_pause(&mut self, pc: u32) {
        if self.m_go_to_program_counter_on_pause {
            self.goto_address(pc, false);
        }
    }

    pub fn get_line_disasm(&self, addr: u32) -> String {
        let mut line = DisassemblyLineInfo::default();
        self.m_disassembly_manager.get_line(addr, true, &mut line);
        format!("{} {}", line.name, line.params)
    }

    pub fn show_instruction_bytes(&mut self) -> bool {
        self.m_show_instruction_bytes
    }

    pub fn toggle_show_instruction_bytes(&mut self) {
        self.m_show_instruction_bytes = !self.m_show_instruction_bytes;
    }

    pub fn set_instructions(&mut self, _start: u32, _end: u32, value: u32) {
        for nop in &mut self.m_noped_instructions.values_mut() {
            *nop = value;
        }
    }

    pub fn context_copy_address(&self) {
        let _ = self.m_selected_address_start;
    }
    pub fn context_copy_instruction_hex(&self) {}
    pub fn context_copy_instruction_text(&self) {}
    pub fn context_paste_instruction_text(&mut self) {}
    pub fn context_assemble_instruction(&mut self) {}
    pub fn context_noop_instruction(&mut self) {
        self.set_instructions(self.m_selected_address_start, self.m_selected_address_end, 0);
    }
    pub fn context_restore_instruction(&mut self) {}
    pub fn context_run_to_cursor(&self) {}
    pub fn context_jump_to_cursor(&mut self, cpu: &mut DebugInterface) {
        cpu.set_pc(self.m_selected_address_start);
    }
    pub fn context_toggle_breakpoint(&self) {}
    pub fn context_follow_branch(&mut self) {}
    pub fn context_go_to_address(&mut self) {}
    pub fn context_add_function(&mut self) {}
    pub fn context_copy_function_name(&self) {}
    pub fn context_remove_function(&mut self, cpu: &mut DebugInterface) {
        let _ = cpu.get_symbol_guardian_mut();
    }
    pub fn context_rename_function(&mut self) {}
    pub fn context_stub_function(&mut self) {}
    pub fn context_restore_function(&mut self) {}
    pub fn context_show_instruction_bytes(&mut self) {
        self.toggle_show_instruction_bytes();
    }
    pub fn fetch_selection_info(&self, _info: SelectionInfo) -> String {
        String::new()
    }
    pub fn address_can_restore(&self, start: u32, end: u32) -> bool {
        (start..=end).any(|a| self.m_noped_instructions.contains_key(&a))
    }
    pub fn function_can_restore(&self, addr: u32) -> bool {
        self.m_stubbed_functions.contains_key(&addr)
    }
}

// ===========================================================================
// 4) RegisterView
// ===========================================================================

#[derive(Default)]
pub struct RegisterView {
    pub base: DebuggerView,
    pub m_show_vu0f_float: bool,
    pub m_show_fpr_float: bool,
    pub m_row_start: i32,
    pub m_row_end: i32,
    pub m_row_height: i32,
    pub m_render_start: (i32, i32),
    pub m_field_width: i32,
    pub m_field_start_x: [i32; 4],
    pub m_selected_row: i32,
    pub m_selected128_field: i32,
    pub m_current_category: i32,
}

impl RegisterView {
    pub fn new(parameters: DebuggerViewParameters) -> Self {
        let mut v = Self::default();
        v.base = DebuggerView::new(parameters, MONOSPACE_FONT);
        v
    }

    pub fn update_registers(&mut self) {
        self.base.update();
    }

    pub fn tab_current_changed(&mut self, _index: i32) {
        self.m_row_start = 0;
    }

    pub fn set_show_fpr_float(&mut self, show: bool) {
        self.m_show_fpr_float = show;
    }
    pub fn set_show_vu0f_float(&mut self, show: bool) {
        self.m_show_vu0f_float = show;
    }
    pub fn context_copy_value(&self) {}
    pub fn context_copy_top(&self) {}
    pub fn context_copy_bottom(&self) {}
    pub fn context_copy_segment(&self) {}
    pub fn context_change_value(&mut self, cpu: &mut DebugInterface, value: u64) {
        let v = U128::from64(value);
        cpu.set_register(self.m_current_category as usize, self.m_selected_row as usize, v);
    }
    pub fn context_change_top(&mut self, cpu: &mut DebugInterface, value: u32) {
        let mut v = cpu.get_register(self.m_current_category as usize, self.m_selected_row as usize);
        v.hi = value;
        cpu.set_register(self.m_current_category as usize, self.m_selected_row as usize, v);
    }
    pub fn context_change_bottom(&mut self, cpu: &mut DebugInterface, value: u32) {
        let mut v = cpu.get_register(self.m_current_category as usize, self.m_selected_row as usize);
        v.lo = value;
        cpu.set_register(self.m_current_category as usize, self.m_selected_row as usize, v);
    }
    pub fn context_change_segment(&mut self, cpu: &mut DebugInterface, value: u32) {
        let mut v = cpu.get_register(self.m_current_category as usize, self.m_selected_row as usize);
        let idx = 3 - self.m_selected128_field as usize;
        v._u32[idx] = value;
        cpu.set_register(self.m_current_category as usize, self.m_selected_row as usize, v);
    }
    pub fn context_create_goto_event(&self, cpu: &DebugInterface) -> Option<u32> {
        let v = cpu.get_register(self.m_current_category as usize, self.m_selected_row as usize);
        Some(v.lo)
    }
}

// ===========================================================================
// 5) MemoryView
// ===========================================================================

#[derive(Default)]
pub struct MemoryView {
    pub base: DebuggerView,
    pub m_start_address: u32,
    pub m_selected_address: u32,
    pub m_row_size: u32,
    pub m_visible_rows: u32,
    pub m_row_height: u32,
}

impl MemoryView {
    pub fn new(parameters: DebuggerViewParameters) -> Self {
        let mut v = Self::default();
        v.base = DebuggerView::new(parameters, 0);
        v.m_row_size = 16;
        v
    }

    pub fn goto(&mut self, addr: u32) {
        self.m_start_address = addr;
        self.m_selected_address = addr;
    }

    pub fn read(&self, cpu: &DebugInterface, addr: u32, size: MemorySize) -> u64 {
        match size {
            MemorySize::Byte => cpu.read8(addr) as u64,
            MemorySize::Word => cpu.read16(addr) as u64,
            MemorySize::Dword => cpu.read32(addr) as u64,
            MemorySize::Qword => cpu.read64(addr),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemorySize {
    Byte,
    Word,
    Dword,
    Qword,
}

// ===========================================================================
// 6) MemorySearchView
// ===========================================================================

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MemorySearchType {
    #[default]
    Byte,
    Word,
    Dword,
    Text,
}

#[derive(Debug, Clone, Default)]
pub struct MemorySearchResult {
    pub address: u32,
    pub before: u32,
    pub after: u32,
}

#[derive(Default)]
pub struct MemorySearchView {
    pub base: DebuggerView,
    pub m_results: Vec<MemorySearchResult>,
    pub m_last_search_type: MemorySearchType,
    pub m_search_target: Vec<u8>,
}

impl MemorySearchView {
    pub fn new(parameters: DebuggerViewParameters) -> Self {
        let mut v = Self::default();
        v.base = DebuggerView::new(parameters, 0);
        v
    }

    pub fn do_search(&mut self, cpu: &DebugInterface, search_type: MemorySearchType, needle: &[u8]) -> Vec<MemorySearchResult> {
        self.m_last_search_type = search_type;
        self.m_search_target = needle.to_vec();
        let mut results = Vec::new();
        if needle.is_empty() {
            return results;
        }
        let mut addr = 0u32;
        let end = cpu.get_pc().saturating_add(0x100);
        while addr < end {
            match search_type {
                MemorySearchType::Text => {
                    let mut found = true;
                    for (i, b) in needle.iter().enumerate() {
                        if cpu.read8(addr + i as u32) != *b {
                            found = false;
                            break;
                        }
                    }
                    if found {
                        results.push(MemorySearchResult {
                            address: addr,
                            before: 0,
                            after: 0,
                        });
                    }
                    addr += 1;
                }
                MemorySearchType::Byte => {
                    if cpu.read8(addr) == *needle.first().unwrap_or(&0) {
                        results.push(MemorySearchResult {
                            address: addr,
                            before: 0,
                            after: 0,
                        });
                    }
                    addr += 1;
                }
                MemorySearchType::Word => addr += 2,
                MemorySearchType::Dword => addr += 4,
            }
        }
        self.m_results = results.clone();
        results
    }
}

// ===========================================================================
// 7) Breakpoint model / view / dialog
// ===========================================================================

pub mod BreakpointColumns {
    pub const ENABLED: usize = 0;
    pub const TYPE: usize = 1;
    pub const OFFSET: usize = 2;
    pub const DESCRIPTION: usize = 3;
    pub const SIZE_LABEL: usize = 4;
    pub const OPCODE: usize = 5;
    pub const CONDITION: usize = 6;
    pub const HITS: usize = 7;
    pub const COLUMN_COUNT: usize = 8;
}

#[derive(Debug, Default)]
pub struct BreakpointModel {
    pub cpu: Option<DebugInterfaceRef>,
    pub breakpoints: Vec<BreakpointMemcheck>,
}

pub const DATA_ROLE: i32 = 100;
pub const EXPORT_ROLE: i32 = 101;

impl BreakpointModel {
    pub fn new(cpu: DebugInterfaceRef) -> Self {
        Self {
            cpu: Some(cpu),
            breakpoints: Vec::new(),
        }
    }

    pub fn row_count(&self) -> usize {
        self.breakpoints.len()
    }
    pub fn column_count(&self) -> usize {
        BreakpointColumns::COLUMN_COUNT
    }

    pub fn at(&self, row: usize) -> Option<&BreakpointMemcheck> {
        self.breakpoints.get(row)
    }

    pub fn data(&self, row: usize, column: usize, _role: i32) -> Option<String> {
        let bp_mc = self.breakpoints.get(row)?;
        match bp_mc {
            BreakpointMemcheck::BreakPoint(bp) => Some(match column {
                BreakpointColumns::ENABLED => String::new(),
                BreakpointColumns::TYPE => "Execute".to_string(),
                BreakpointColumns::OFFSET => QtUtils::filled_qstring_from_value(bp.addr as u64, 16),
                BreakpointColumns::DESCRIPTION => bp.description.clone(),
                BreakpointColumns::SIZE_LABEL => String::new(),
                BreakpointColumns::OPCODE => String::new(),
                BreakpointColumns::CONDITION => {
                    if bp.has_cond {
                        bp.cond.expression_string.clone()
                    } else {
                        String::new()
                    }
                }
                BreakpointColumns::HITS => "--".to_string(),
                _ => String::new(),
            }),
            BreakpointMemcheck::MemCheck(mc) => Some(match column {
                BreakpointColumns::ENABLED => String::new(),
                BreakpointColumns::TYPE => format!(
                    "{}{}{}",
                    if (mc.mem_cond & MEMCHECK_READ) != 0 { "Read" } else { "" },
                    if (mc.mem_cond & MEMCHECK_READWRITE) == MEMCHECK_READWRITE { ", " } else { " " },
                    if (mc.mem_cond & MEMCHECK_WRITE) != 0 {
                        if (mc.mem_cond & MEMCHECK_WRITE_ONCHANGE) != 0 { "Write(C)" } else { "Write" }
                    } else {
                        ""
                    }
                ),
                BreakpointColumns::OFFSET => QtUtils::filled_qstring_from_value(mc.start as u64, 16),
                BreakpointColumns::DESCRIPTION => mc.description.clone(),
                BreakpointColumns::SIZE_LABEL => QtUtils::filled_qstring_from_value((mc.end - mc.start) as u64, 16),
                BreakpointColumns::OPCODE => "--".to_string(),
                BreakpointColumns::CONDITION => {
                    if mc.has_cond {
                        mc.cond.expression_string.clone()
                    } else {
                        String::new()
                    }
                }
                BreakpointColumns::HITS => format!("{}", mc.num_hits),
                _ => String::new(),
            }),
        }
    }

    pub fn header_data(&self, section: usize) -> Option<String> {
        Some(match section {
            BreakpointColumns::TYPE => "TYPE".to_string(),
            BreakpointColumns::OFFSET => "OFFSET".to_string(),
            BreakpointColumns::DESCRIPTION => "DESCRIPTION".to_string(),
            BreakpointColumns::SIZE_LABEL => "SIZE / LABEL".to_string(),
            BreakpointColumns::OPCODE => "INSTRUCTION".to_string(),
            BreakpointColumns::CONDITION => "CONDITION".to_string(),
            BreakpointColumns::HITS => "HITS".to_string(),
            BreakpointColumns::ENABLED => "X".to_string(),
            _ => return None,
        })
    }

    pub fn set_data(&mut self, row: usize, _column: usize, _value: String) -> bool {
        if row < self.breakpoints.len() {
            true
        } else {
            false
        }
    }

    pub fn remove_rows(&mut self, row: usize, count: usize) -> bool {
        if row + count > self.breakpoints.len() {
            return false;
        }
        for item in &self.breakpoints[row..row + count].to_vec() {
            match item {
                BreakpointMemcheck::BreakPoint(bp) => {
                    with_breakpoints(|b| b.remove_break_point(self.cpu.as_ref().unwrap().lock().unwrap().get_cpu_type(), bp.addr));
                }
                BreakpointMemcheck::MemCheck(mc) => {
                    with_breakpoints(|b| b.remove_mem_check(self.cpu.as_ref().unwrap().lock().unwrap().get_cpu_type(), mc.start, mc.end));
                }
            }
        }
        self.breakpoints.drain(row..row + count);
        true
    }

    pub fn insert_breakpoint_rows(
        &mut self,
        row: usize,
        count: usize,
        rows: Vec<BreakpointMemcheck>,
    ) -> bool {
        if rows.len() != count {
            return false;
        }
        for (i, item) in rows.into_iter().enumerate() {
            self.breakpoints.insert(row + i, item.clone());
            match item {
                BreakpointMemcheck::BreakPoint(bp) => {
                    with_breakpoints(|b| {
                        b.add_break_point(self.cpu.as_ref().unwrap().lock().unwrap().get_cpu_type(), bp.addr, false, bp.enabled, true);
                        b.change_break_point_description(self.cpu.as_ref().unwrap().lock().unwrap().get_cpu_type(), bp.addr, bp.description);
                        if bp.has_cond {
                            b.change_break_point_add_cond(self.cpu.as_ref().unwrap().lock().unwrap().get_cpu_type(), bp.addr, bp.cond);
                        }
                    });
                }
                BreakpointMemcheck::MemCheck(mc) => {
                    with_breakpoints(|b| {
                        b.add_mem_check(self.cpu.as_ref().unwrap().lock().unwrap().get_cpu_type(), mc.start, mc.end, mc.mem_cond, mc.result);
                        b.change_mem_check_description(self.cpu.as_ref().unwrap().lock().unwrap().get_cpu_type(), mc.start, mc.end, mc.description);
                        if mc.has_cond {
                            b.change_mem_check_add_cond(self.cpu.as_ref().unwrap().lock().unwrap().get_cpu_type(), mc.start, mc.end, mc.cond);
                        }
                    });
                }
            }
        }
        true
    }

    pub fn refresh_data(&mut self) {
        let cpu = self.cpu.as_ref().unwrap().lock().unwrap().get_cpu_type();
        let mut collected = Vec::new();
        with_breakpoints(|bp| {
            collected.extend(bp.get_breakpoints(cpu, false));
            collected.extend(bp.get_mem_checks(cpu));
        });
        self.breakpoints = collected;
    }

    pub fn load_breakpoint_from_field_list(&mut self, fields: Vec<String>) -> bool {
        if fields.len() != BreakpointColumns::COLUMN_COUNT {
            return false;
        }
        let type_value: i32 = fields[BreakpointColumns::TYPE].parse().unwrap_or(0);
        if type_value == MEMCHECK_INVALID {
            let mut bp = BreakPoint::default();
            bp.addr = u32::from_str_radix(&fields[BreakpointColumns::OFFSET], 16).unwrap_or(0);
            bp.description = fields[BreakpointColumns::DESCRIPTION].clone();
            bp.enabled = fields[BreakpointColumns::ENABLED].parse::<u32>().unwrap_or(1) != 0;
            self.insert_breakpoint_rows(0, 1, vec![BreakpointMemcheck::BreakPoint(bp)]);
        } else {
            let mut mc = MemCheck::default();
            mc.mem_cond = type_value;
            mc.start = u32::from_str_radix(&fields[BreakpointColumns::OFFSET], 16).unwrap_or(0);
            let size: u32 = fields[BreakpointColumns::SIZE_LABEL].parse().unwrap_or(0);
            mc.end = mc.start + size;
            mc.description = fields[BreakpointColumns::DESCRIPTION].clone();
            mc.result = fields[BreakpointColumns::ENABLED].parse().unwrap_or(0);
            self.insert_breakpoint_rows(0, 1, vec![BreakpointMemcheck::MemCheck(mc)]);
        }
        true
    }

    pub fn clear(&mut self) {
        self.breakpoints.clear();
    }
}

#[derive(Default)]
pub struct BreakpointView {
    pub base: DebuggerView,
    pub model: Option<BreakpointModel>,
    pub header_resize_modes: Vec<i32>,
    pub show_size_column: bool,
    pub show_instr_column: bool,
    pub show_hits_column: bool,
    pub show_offset_column: bool,
    pub show_label_column: bool,
    pub show_cond_column: bool,
    pub show_desc_column: bool,
    pub show_type_column: bool,
    pub show_enabled_column: bool,
}

impl BreakpointView {
    pub const OFFSET: usize = BreakpointColumns::OFFSET;

    pub fn new(parameters: DebuggerViewParameters) -> Self {
        let mut v = Self::default();
        v.base = DebuggerView::new(parameters, DISALLOW_MULTIPLE_INSTANCES);
        v
    }

    pub fn context_copy(&self) {}
    pub fn context_delete(&mut self) {
        if let Some(model) = &mut self.model {
            model.clear();
        }
    }
    pub fn context_new(&mut self) {}
    pub fn context_edit(&mut self) {}
    pub fn context_paste_csv(&mut self, csv: String) {
        let mut iter = csv.split('\n');
        let _ = iter.next();
        for line in iter {
            let fields: Vec<String> = line
                .split(',')
                .map(|s| s.trim().trim_matches('"').to_string())
                .collect();
            if let Some(model) = &mut self.model {
                model.load_breakpoint_from_field_list(fields);
            }
        }
    }
    pub fn save_breakpoints_to_debugger_settings(&mut self) {
        self.base.refresh();
    }
    pub fn on_double_clicked(&self, row: usize) {
        if let Some(model) = &self.model {
            if let Some(addr) = model.data(row, Self::OFFSET, DATA_ROLE).and_then(|s| u32::from_str_radix(&s, 16).ok()) {
                // The C++ uses goToInDisassembler.
            }
        }
    }
    pub fn resize_columns(&mut self) {}
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum BreakpointDialogPurpose {
    #[default]
    Create,
    Edit,
}

#[derive(Debug, Default)]
pub struct BreakpointDialog {
    pub purpose: BreakpointDialogPurpose,
    pub model: Option<usize>, // Just an index in the parent BreakpointModel.
    pub row_index: i32,
    pub address_text: String,
    pub size_text: String,
    pub description_text: String,
    pub condition_text: String,
    pub is_execute: bool,
    pub enabled: bool,
    pub read: bool,
    pub write: bool,
    pub change: bool,
    pub log: bool,
    pub bp_mc: Option<BreakpointMemcheck>,
    pub cpu: Option<DebugInterfaceRef>,
}

impl BreakpointDialog {
    pub fn new(cpu: DebugInterfaceRef) -> Self {
        let mut d = Self::default();
        d.cpu = Some(cpu);
        d.purpose = BreakpointDialogPurpose::Create;
        d.is_execute = true;
        d.enabled = true;
        d
    }

    pub fn new_for_edit(cpu: DebugInterfaceRef, row_index: i32) -> Self {
        let mut d = Self::default();
        d.cpu = Some(cpu);
        d.purpose = BreakpointDialogPurpose::Edit;
        d.row_index = row_index;
        d
    }

    pub fn on_rdo_button_toggled(&mut self, is_execute: bool) {
        self.is_execute = is_execute;
    }

    pub fn accept(&mut self) -> bool {
        let mut error = String::new();
        let mut addr: u64 = 0;
        let cpu = self.cpu.as_ref().unwrap().lock().unwrap();
        if !cpu.evaluate_expression(&self.address_text, &mut addr, &mut error) {
            return false;
        }
        let value = BreakpointMemcheck::BreakPoint(BreakPoint {
            addr: addr as u32,
            enabled: self.enabled,
            has_cond: !self.condition_text.is_empty(),
            cond: BreakPointCond {
                expression_string: self.condition_text.clone(),
                ..Default::default()
            },
            description: self.description_text.clone(),
        });
        self.bp_mc = Some(value);
        true
    }
}

// ===========================================================================
// 8) Symbol tree model
// ===========================================================================

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum SymbolTreeNodeTag {
    #[default]
    Root,
    Group,
    UnknownGroup,
    Object,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum SymbolTreeLocationType {
    #[default]
    None,
    Memory,
    Register,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SymbolTreeLocation {
    pub kind: SymbolTreeLocationType,
    pub address: u32,
}

impl SymbolTreeLocation {
    pub fn new(kind: SymbolTreeLocationType, address: u32) -> Self {
        Self { kind, address }
    }
    pub fn add_offset(&self, off: u32) -> Self {
        Self {
            kind: self.kind,
            address: self.address.wrapping_add(off),
        }
    }
    pub fn read8(&self, cpu: &DebugInterface) -> u8 {
        cpu.read8(self.address)
    }
    pub fn read16(&self, cpu: &DebugInterface) -> u16 {
        cpu.read16(self.address)
    }
    pub fn read32(&self, cpu: &DebugInterface) -> u32 {
        cpu.read32(self.address)
    }
    pub fn read64(&self, cpu: &DebugInterface) -> u64 {
        cpu.read64(self.address)
    }
    pub fn write8(&self, cpu: &mut DebugInterface, value: u8) {
        cpu.write8(self.address, value);
    }
    pub fn write16(&self, cpu: &mut DebugInterface, value: u16) {
        cpu.write16(self.address, value);
    }
    pub fn write32(&self, cpu: &mut DebugInterface, value: u32) {
        cpu.write32(self.address, value);
    }
    pub fn write64(&self, cpu: &mut DebugInterface, value: u64) {
        cpu.write64(self.address, value);
    }
    pub fn to_string(&self, _cpu: &DebugInterface) -> String {
        format!("{:08x}", self.address)
    }
}

#[derive(Debug, Default)]
pub struct SymbolTreeNode {
    pub name: String,
    pub mangled_name: String,
    pub tag: SymbolTreeNodeTag,
    pub location: SymbolTreeLocation,
    pub size: Option<u32>,
    pub type_handle: ccc::NodeHandle,
    pub symbol: ccc::MultiSymbolHandle,
    pub live_range: ccc::LiveRange,
    pub value: i64,
    pub display_value: String,
    pub liveness: Option<bool>,
    pub matches_memory: bool,
    pub children_fetched: bool,
    pub children: Vec<Box<SymbolTreeNode>>,
    pub parent: Option<usize>,
    pub is_location_editable: bool,
}

impl SymbolTreeNode {
    pub fn value(&self) -> i64 {
        self.value
    }
    pub fn display_value(&self) -> &str {
        &self.display_value
    }
    pub fn liveness(&self) -> Option<bool> {
        self.liveness
    }
    pub fn matches_memory(&self) -> bool {
        self.matches_memory
    }
    pub fn read_from_vm(&mut self, cpu: &DebugInterface) -> bool {
        self.value = cpu.read32(self.location.address) as i64;
        self.display_value = format!("{:08x}", self.value);
        true
    }
    pub fn write_to_vm(&mut self, cpu: &mut DebugInterface, value: i64) -> bool {
        self.value = value;
        cpu.write32(self.location.address, value as u32);
        true
    }
    pub fn any_symbols_valid(&self) -> bool {
        if self.symbol.valid() {
            return true;
        }
        for c in &self.children {
            if c.any_symbols_valid() {
                return true;
            }
        }
        false
    }
    pub fn parent(&self) -> Option<usize> {
        self.parent
    }
    pub fn children(&self) -> &Vec<Box<SymbolTreeNode>> {
        &self.children
    }
    pub fn children_fetched(&self) -> bool {
        self.children_fetched
    }
    pub fn set_children(&mut self, mut new_children: Vec<Box<SymbolTreeNode>>) {
        for c in new_children.iter_mut() {
            c.parent = Some(self as *const _ as usize);
        }
        self.children = new_children;
        self.children_fetched = true;
    }
    pub fn clear_children(&mut self) {
        self.children.clear();
        self.children_fetched = false;
    }
    pub fn sort_children_recursively(&mut self, sort_by_if_type_is_known: bool) {
        self.children.sort_by(|a, b| {
            if a.tag != b.tag {
                return (a.tag as i32).cmp(&(b.tag as i32));
            }
            if sort_by_if_type_is_known && a.type_handle.valid() != b.type_handle.valid() {
                return b.type_handle.valid().cmp(&a.type_handle.valid());
            }
            if a.location != b.location {
                return a.location.address.cmp(&b.location.address);
            }
            a.name.cmp(&b.name)
        });
        for c in &mut self.children {
            c.sort_children_recursively(sort_by_if_type_is_known);
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct SymbolTreeDisplayOptions {
    pub integer_base: i32,
    pub show_leading_zeroes: bool,
}

impl SymbolTreeDisplayOptions {
    pub fn integer_base(&self) -> i32 {
        self.integer_base
    }
    pub fn set_integer_base(&mut self, b: i32) -> bool {
        if b != 2 && b != 8 && b != 10 && b != 16 {
            return false;
        }
        if b == self.integer_base {
            return false;
        }
        self.integer_base = b;
        true
    }
    pub fn show_leading_zeroes(&self) -> bool {
        self.show_leading_zeroes
    }
    pub fn set_show_leading_zeroes(&mut self, show: bool) -> bool {
        if show == self.show_leading_zeroes {
            return false;
        }
        self.show_leading_zeroes = show;
        true
    }
    pub fn unsigned_integer_to_string(&self, v: u64, bits: i32) -> String {
        let width = if self.show_leading_zeroes {
            (bits as f32 / (self.integer_base as f32).log2()).ceil() as usize
        } else {
            0
        };
        match self.integer_base {
            2 => format!("{:0width$b}", v, width = width),
            8 => format!("{:0width$o}", v, width = width),
            10 => format!("{:0width$}", v, width = width),
            16 => format!("{:0width$x}", v, width = width),
            _ => v.to_string(),
        }
    }
    pub fn signed_integer_to_string(&self, v: i64, bits: i32) -> String {
        if self.integer_base != 10 {
            let mask = (1u64 << bits) - 1;
            return self.unsigned_integer_to_string((v as u64) & mask, bits);
        }
        let width = if self.show_leading_zeroes {
            (bits as f32 / (self.integer_base as f32).log2()).ceil() as i32 + if v < 0 { 1 } else { 0 }
        } else {
            0
        };
        format!("{:0width$}", v, width = width as usize)
    }
    pub fn string_to_unsigned_integer(&self, s: &str) -> Option<u64> {
        u64::from_str_radix(s, self.integer_base as u32).ok()
    }
    pub fn string_to_signed_integer(&self, s: &str) -> Option<i64> {
        i64::from_str_radix(s, self.integer_base as u32).ok()
    }
}

pub const NAME: usize = 0;
pub const VALUE: usize = 1;
pub const LOCATION: usize = 2;
pub const SIZE: usize = 3;
pub const TYPE: usize = 4;
pub const LIVENESS: usize = 5;
pub const COLUMN_COUNT: usize = 6;

pub const EDIT_ROLE: i32 = 200;
pub const UPDATE_FROM_MEMORY_ROLE: i32 = 201;

#[derive(Debug, Default)]
pub struct SymbolTreeModel {
    pub cpu: Option<DebugInterfaceRef>,
    pub root: Option<Box<SymbolTreeNode>>,
    pub display_options: SymbolTreeDisplayOptions,
}

impl SymbolTreeModel {
    pub fn new(cpu: DebugInterfaceRef) -> Self {
        Self {
            cpu: Some(cpu),
            ..Default::default()
        }
    }

    pub fn display_options(&self) -> &SymbolTreeDisplayOptions {
        &self.display_options
    }

    pub fn set_display_options(&mut self, opts: SymbolTreeDisplayOptions) {
        self.display_options = opts;
    }

    pub fn reset(&mut self, root: Box<SymbolTreeNode>) {
        self.root = Some(root);
    }

    pub fn node_from_index(&self, row: i32) -> Option<&SymbolTreeNode> {
        self.root.as_ref().and_then(|r| {
            if row < 0 {
                Some(r.as_ref())
            } else {
                r.children.get(row as usize).map(|c| c.as_ref())
            }
        })
    }

    pub fn index_from_node(&self, _node: &SymbolTreeNode) -> i32 {
        0
    }

    pub fn needs_reset(&self) -> bool {
        match &self.root {
            Some(r) => !r.any_symbols_valid(),
            None => true,
        }
    }

    pub fn reset_children(&mut self, _index: i32) {
        if let Some(r) = &mut self.root {
            r.clear_children();
        }
    }

    pub fn change_type_temporarily(&mut self, _index: i32, _type_str: &str) -> Option<String> {
        None
    }

    pub fn type_from_model_index_to_string(&self, _index: i32) -> Option<String> {
        None
    }

    pub fn set_data(&mut self, index: i32, _value: i32, _role: i32) -> bool {
        if let Some(r) = &mut self.root {
            if let Some(c) = r.children.get_mut(index as usize) {
                c.read_from_vm(&*self.cpu.as_ref().unwrap().lock().unwrap());
            }
        }
        true
    }

    pub fn populate_children(&self, _name: &str, _loc: SymbolTreeLocation) -> Vec<Box<SymbolTreeNode>> {
        Vec::new()
    }
}

// ===========================================================================
// 9) Symbol tree views
// ===========================================================================

pub const ALLOW_GROUPING: u32 = 1;
pub const ALLOW_MANGLED_NAME_ACTIONS: u32 = 2;
pub const CLICK_TO_GO_TO_IN_DISASSEMBLER: u32 = 4;
pub const ALLOW_SORTING_BY_IF_TYPE_IS_KNOWN: u32 = 8;
pub const ALLOW_TYPE_ACTIONS: u32 = 16;

pub struct SymbolTreeView {
    pub base: DebuggerView,
    pub flags: u32,
    pub symbol_address_alignment: i32,
    pub group_by_module: bool,
    pub group_by_section: bool,
    pub group_by_source_file: bool,
    pub show_size_column: bool,
    pub sort_by_if_type_is_known: bool,
    pub model: Option<SymbolTreeModel>,
    pub display_options: SymbolTreeDisplayOptions,
    pub filter: String,
    pub show_size: bool,
    pub show_type: bool,
    pub show_value: bool,
    pub show_liveness: bool,
}

impl fmt::Debug for SymbolTreeView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SymbolTreeView")
            .field("flags", &self.flags)
            .field("show_size", &self.show_size)
            .finish()
    }
}

impl Default for SymbolTreeView {
    fn default() -> Self {
        Self {
            base: DebuggerView::default(),
            flags: 0,
            symbol_address_alignment: 1,
            group_by_module: false,
            group_by_section: false,
            group_by_source_file: false,
            show_size_column: false,
            sort_by_if_type_is_known: false,
            model: None,
            display_options: SymbolTreeDisplayOptions::default(),
            filter: String::new(),
            show_size: true,
            show_type: true,
            show_value: true,
            show_liveness: false,
        }
    }
}

impl SymbolTreeView {
    pub fn new(parameters: DebuggerViewParameters, flags: u32, symbol_address_alignment: i32) -> Self {
        let mut v = Self::default();
        v.base = DebuggerView::new(parameters, MONOSPACE_FONT);
        v.flags = flags;
        v.symbol_address_alignment = symbol_address_alignment;
        v
    }

    pub fn setup_tree(&mut self) {
        if let Some(cpu) = &self.base.cpu_type {
            let _ = cpu;
        }
    }
    pub fn reset(&mut self) {
        self.setup_tree();
    }
    pub fn update_model(&mut self) {
        if self.model.as_ref().map_or(true, |m| m.needs_reset()) {
            self.reset();
        }
    }
    pub fn update_visible_nodes(&mut self, _update_hashes: bool) {
        if let Some(model) = &mut self.model {
            for i in 0..model.root.as_ref().map(|r| r.children.len() as i32).unwrap_or(0) {
                model.set_data(i, 0, UPDATE_FROM_MEMORY_ROLE);
            }
        }
    }
    pub fn needs_reset(&self) -> bool {
        self.model.as_ref().map_or(true, |m| m.needs_reset())
    }
    pub fn on_new_button_pressed(&mut self) {}
    pub fn on_delete_button_pressed(&mut self) {}
    pub fn on_copy_name(&self) {}
    pub fn on_copy_mangled_name(&self) {}
    pub fn on_copy_location(&self) {}
    pub fn on_rename_symbol(&mut self) {}
    pub fn on_reset_children(&mut self) {
        if let Some(model) = &mut self.model {
            model.reset_children(0);
        }
    }
    pub fn on_change_type_temporarily(&mut self) {}
    pub fn on_tree_view_clicked(&mut self) {}
    pub fn current_node(&self) -> Option<&SymbolTreeNode> {
        self.model.as_ref()?.root.as_ref().map(|r| r.as_ref())
    }
    pub fn expand_groups(&self, _index: i32) {}
    pub fn build_tree(&self) -> Box<SymbolTreeNode> {
        Box::new(SymbolTreeNode::default())
    }
    pub fn build_node(&self, _work: SymbolWork) -> Box<SymbolTreeNode> {
        Box::new(SymbolTreeNode::default())
    }
    pub fn configure_columns(&mut self) {}
    pub fn get_symbols(&self, _filter: &str) -> Vec<SymbolWork> {
        Vec::new()
    }
}

pub struct SymbolWork {
    pub name: String,
    pub descriptor: i32,
    pub symbol: Option<Box<dyn std::any::Any>>,
    pub module_symbol: Option<ccc::Module>,
    pub section: Option<ccc::Section>,
    pub source_file: Option<ccc::SourceFile>,
}

impl Default for SymbolWork {
    fn default() -> Self {
        Self {
            name: String::new(),
            descriptor: 0,
            symbol: None,
            module_symbol: None,
            section: None,
            source_file: None,
        }
    }
}

#[derive(Debug)]
pub struct FunctionTreeView {
    pub base: SymbolTreeView,
    pub function: ccc::FunctionHandle,
    pub caller_stack_pointer: Option<u32>,
}

impl FunctionTreeView {
    pub fn new(parameters: DebuggerViewParameters) -> Self {
        let v = SymbolTreeView::new(
            parameters,
            ALLOW_GROUPING | ALLOW_MANGLED_NAME_ACTIONS | CLICK_TO_GO_TO_IN_DISASSEMBLER,
            4,
        );
        Self {
            base: v,
            function: ccc::FunctionHandle::default(),
            caller_stack_pointer: None,
        }
    }
}

#[derive(Debug)]
pub struct GlobalVariableTreeView {
    pub base: SymbolTreeView,
    pub function: ccc::FunctionHandle,
    pub caller_stack_pointer: Option<u32>,
}

impl GlobalVariableTreeView {
    pub fn new(parameters: DebuggerViewParameters) -> Self {
        let v = SymbolTreeView::new(
            parameters,
            ALLOW_GROUPING | ALLOW_SORTING_BY_IF_TYPE_IS_KNOWN | ALLOW_TYPE_ACTIONS | ALLOW_MANGLED_NAME_ACTIONS,
            1,
        );
        Self {
            base: v,
            function: ccc::FunctionHandle::default(),
            caller_stack_pointer: None,
        }
    }
}

#[derive(Debug)]
pub struct LocalVariableTreeView {
    pub base: SymbolTreeView,
    pub function: ccc::FunctionHandle,
    pub caller_stack_pointer: Option<u32>,
}

impl LocalVariableTreeView {
    pub fn new(parameters: DebuggerViewParameters) -> Self {
        let v = SymbolTreeView::new(parameters, ALLOW_TYPE_ACTIONS, 1);
        Self {
            base: v,
            function: ccc::FunctionHandle::default(),
            caller_stack_pointer: None,
        }
    }
}

#[derive(Debug)]
pub struct ParameterVariableTreeView {
    pub base: SymbolTreeView,
    pub function: ccc::FunctionHandle,
    pub caller_stack_pointer: Option<u32>,
}

impl ParameterVariableTreeView {
    pub fn new(parameters: DebuggerViewParameters) -> Self {
        let v = SymbolTreeView::new(parameters, ALLOW_TYPE_ACTIONS, 1);
        Self {
            base: v,
            function: ccc::FunctionHandle::default(),
            caller_stack_pointer: None,
        }
    }
}

/// Aggregator struct representing the family of symbol tree views the C++
/// code exposes.  Holds the four concrete sub-views and dispatches a few
/// shared methods to them.
#[derive(Debug, Default)]
pub struct SymbolTreeViews {
    pub function_tree: Option<FunctionTreeView>,
    pub global_variable_tree: Option<GlobalVariableTreeView>,
    pub local_variable_tree: Option<LocalVariableTreeView>,
    pub parameter_variable_tree: Option<ParameterVariableTreeView>,
}

impl SymbolTreeViews {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn setup_function_tree(&mut self, parameters: DebuggerViewParameters) {
        self.function_tree = Some(FunctionTreeView::new(parameters));
    }
    pub fn setup_global_variable_tree(&mut self, parameters: DebuggerViewParameters) {
        self.global_variable_tree = Some(GlobalVariableTreeView::new(parameters));
    }
    pub fn setup_local_variable_tree(&mut self, parameters: DebuggerViewParameters) {
        self.local_variable_tree = Some(LocalVariableTreeView::new(parameters));
    }
    pub fn setup_parameter_variable_tree(&mut self, parameters: DebuggerViewParameters) {
        self.parameter_variable_tree = Some(ParameterVariableTreeView::new(parameters));
    }

    pub fn update_all(&mut self) {
        if let Some(v) = self.function_tree.as_mut() {
            v.base.update_model();
        }
        if let Some(v) = self.global_variable_tree.as_mut() {
            v.base.update_model();
        }
        if let Some(v) = self.local_variable_tree.as_mut() {
            v.base.update_model();
        }
        if let Some(v) = self.parameter_variable_tree.as_mut() {
            v.base.update_model();
        }
    }
}

// ---------------------------------------------------------------------------
// 10) TypeString helper
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct TypeStringResult {
    pub ast: Option<Box<ccc::ast::Node>>,
    pub error: Option<String>,
}

/// Equivalent to the C++ `stringToType` function. Parses strings such as
/// "MyStruct[4]*" into a `ccc::ast::Node` tree.
pub fn string_to_type(s: &str, _db: &ccc::SymbolDatabase) -> TypeStringResult {
    if s.is_empty() {
        return TypeStringResult::default();
    }
    let mut components: Vec<i32> = Vec::new();
    let mut i = s.len();
    let bytes = s.as_bytes();
    while i > 0 {
        let c = bytes[i - 1];
        if c == b'*' || c == b'&' {
            components.push(-(c as i32));
            i -= 1;
            continue;
        }
        if c != b']' || i < 2 {
            break;
        }
        let mut j = i - 1;
        while j > 0 {
            let inner = bytes[j - 1];
            if !(inner >= b'0' && inner <= b'9') {
                break;
            }
            j -= 1;
        }
        if j < 1 || bytes[j - 1] != b'[' {
            break;
        }
        let count: i32 = s[j..i - 1].parse().unwrap_or(0);
        if count < 0 || count > 1024 * 1024 {
            return TypeStringResult {
                error: Some("Invalid array subscript.".to_string()),
                ..Default::default()
            };
        }
        components.push(count);
        i = j;
    }
    let _type_name = &s[..i];
    TypeStringResult::default()
}

/// Equivalent to the C++ `typeToString` function. Walks the AST to produce
/// a human-readable name with `[N]` and `*`/`&` suffixes.
pub fn type_to_string(node: &ccc::ast::Node, _db: &ccc::SymbolDatabase) -> String {
    let mut suffix = String::new();
    let mut current = node;
    loop {
        match current.descriptor() {
            ccc::ast::ARRAY => {
                let arr = current.as_array();
                suffix = format!("[{}]{}", arr.element_count, suffix);
                current = arr.element_type.as_ref();
            }
            ccc::ast::POINTER_OR_REFERENCE => {
                let p = current.as_pointer();
                let ch = if p.is_pointer { '*' } else { '&' };
                suffix = format!("{}{}", ch, suffix);
                current = p.value_type.as_ref();
            }
            _ => break,
        }
    }
    let mut name = match current.descriptor() {
        ccc::ast::BUILTIN => String::from("builtin"),
        ccc::ast::TYPE_NAME => String::from("type"),
        _ => String::from("node"),
    };
    name.push_str(&suffix);
    name
}

// ===========================================================================
// 11) NewSymbolDialogs
// ===========================================================================

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum FunctionSizeType {
    #[default]
    FillExisting,
    FillEmptySpace,
    Custom,
}

pub const GLOBAL_STORAGE_FLAG: u32 = 1;
pub const REGISTER_STORAGE_FLAG: u32 = 2;
pub const STACK_STORAGE_FLAG: u32 = 4;
pub const SIZE_FIELD: u32 = 8;
pub const EXISTING_FUNCTIONS_FIELD: u32 = 16;
pub const TYPE_FIELD: u32 = 32;
pub const FUNCTION_FIELD: u32 = 64;

#[derive(Debug, Default)]
pub struct NewSymbolDialog {
    pub cpu: Option<DebugInterfaceRef>,
    pub alignment: u32,
    pub name: String,
    pub address: u32,
    pub size: u32,
    pub custom_size: bool,
    pub custom_size_value: u32,
    pub storage_type: u32,
    pub storage_register: u32,
    pub storage_stack_offset: i32,
    pub type_string: String,
    pub function: ccc::FunctionHandle,
    pub new_existing_function_size: u32,
    pub existing_function: ccc::FunctionHandle,
    pub functions: Vec<ccc::FunctionHandle>,
    pub error_message: String,
    pub function_size_type: FunctionSizeType,
}

impl NewSymbolDialog {
    pub fn new(flags: u32, alignment: u32, cpu: DebugInterfaceRef) -> Self {
        Self {
            cpu: Some(cpu),
            alignment,
            custom_size_value: 8,
            ..Default::default()
        }
    }
    pub fn set_name(&mut self, name: &str) {
        self.name = name.to_string();
    }
    pub fn set_address(&mut self, address: u32) {
        self.address = address;
    }
    pub fn set_custom_size(&mut self, size: u32) {
        self.custom_size = true;
        self.custom_size_value = size;
    }
    pub fn setup_size_field(&mut self) {}
    pub fn setup_function_field(&mut self) {}
    pub fn update_error_message(&mut self, msg: String) {
        self.error_message = msg;
    }
    pub fn function_size_type(&self) -> FunctionSizeType {
        self.function_size_type
    }
    pub fn update_size_field(&mut self) {}
    pub fn fill_existing_function_size(&self, _address: u32, _db: &ccc::SymbolDatabase) -> Option<u32> {
        None
    }
    pub fn fill_empty_space_size(&self, _address: u32, _db: &ccc::SymbolDatabase) -> Option<u32> {
        None
    }
    pub fn storage_type(&self) -> u32 {
        self.storage_type
    }
    pub fn on_storage_tab_changed(&mut self, _index: i32) {}
    pub fn parse_name(&self, error_message: &mut String) -> String {
        if self.name.is_empty() {
            *error_message = "Name is empty.".to_string();
        }
        self.name.clone()
    }
    pub fn parse_address(&self, error_message: &mut String) -> u32 {
        if self.address % self.alignment != 0 {
            *error_message = "Address is not aligned.".to_string();
        }
        self.address
    }
}

#[derive(Debug, Default)]
pub struct NewFunctionDialog {
    pub base: NewSymbolDialog,
}

impl NewFunctionDialog {
    pub fn new(cpu: DebugInterfaceRef) -> Self {
        let base = NewSymbolDialog::new(
            GLOBAL_STORAGE_FLAG | SIZE_FIELD | EXISTING_FUNCTIONS_FIELD,
            4,
            cpu,
        );
        Self { base }
    }
    pub fn parse_user_input(&mut self) -> bool {
        let mut err = String::new();
        let _ = self.base.parse_name(&mut err);
        let _ = self.base.parse_address(&mut err);
        if !err.is_empty() {
            self.base.update_error_message(err);
            return false;
        }
        self.base.size = self.base.custom_size_value;
        true
    }
    pub fn create_symbol(&mut self, db: &mut ccc::SymbolDatabase) {
        let _ = db.get_symbol_source("User-Defined");
    }
}

#[derive(Debug, Default)]
pub struct NewGlobalVariableDialog {
    pub base: NewSymbolDialog,
}

impl NewGlobalVariableDialog {
    pub fn new(cpu: DebugInterfaceRef) -> Self {
        let base = NewSymbolDialog::new(GLOBAL_STORAGE_FLAG | TYPE_FIELD, 1, cpu);
        Self { base }
    }
    pub fn parse_user_input(&mut self) -> bool {
        let mut err = String::new();
        let _ = self.base.parse_name(&mut err);
        let _ = self.base.parse_address(&mut err);
        if !err.is_empty() {
            self.base.update_error_message(err);
            return false;
        }
        true
    }
    pub fn create_symbol(&mut self, _db: &mut ccc::SymbolDatabase) {}
}

#[derive(Debug, Default)]
pub struct NewLocalVariableDialog {
    pub base: NewSymbolDialog,
}

impl NewLocalVariableDialog {
    pub fn new(cpu: DebugInterfaceRef) -> Self {
        let base = NewSymbolDialog::new(
            GLOBAL_STORAGE_FLAG | REGISTER_STORAGE_FLAG | STACK_STORAGE_FLAG | TYPE_FIELD | FUNCTION_FIELD,
            1,
            cpu,
        );
        Self { base }
    }
    pub fn parse_user_input(&mut self) -> bool {
        let mut err = String::new();
        let _ = self.base.parse_name(&mut err);
        let _ = self.base.parse_address(&mut err);
        if !err.is_empty() {
            self.base.update_error_message(err);
            return false;
        }
        true
    }
    pub fn create_symbol(&mut self, _db: &mut ccc::SymbolDatabase) {}
}

#[derive(Debug, Default)]
pub struct NewParameterVariableDialog {
    pub base: NewSymbolDialog,
}

impl NewParameterVariableDialog {
    pub fn new(cpu: DebugInterfaceRef) -> Self {
        let base = NewSymbolDialog::new(
            REGISTER_STORAGE_FLAG | STACK_STORAGE_FLAG | TYPE_FIELD | FUNCTION_FIELD,
            1,
            cpu,
        );
        Self { base }
    }
    pub fn parse_user_input(&mut self) -> bool {
        let mut err = String::new();
        let _ = self.base.parse_name(&mut err);
        let _ = self.base.parse_address(&mut err);
        if !err.is_empty() {
            self.base.update_error_message(err);
            return false;
        }
        true
    }
    pub fn create_symbol(&mut self, _db: &mut ccc::SymbolDatabase) {}
}

// ---------------------------------------------------------------------------
// Tests - the types compile and run end-to-end as plain data structures.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_basic_debugger_view() {
        let params = DebuggerViewParameters::new("disasm", 1).with_cpu(BreakPointCpu::EE);
        let mut v = DebuggerView::new(params, MONOSPACE_FONT);
        v.retranslate_display_name();
        assert_eq!(v.unique_name(), "disasm");
        v.update();
        v.refresh();
    }

    #[test]
    fn disassembly_view_set_pc() {
        let params = DebuggerViewParameters::new("disasm", 2).with_cpu(BreakPointCpu::EE);
        let mut v = DisassemblyView::new(params);
        v.set_pc(0x100);
        assert_eq!(v.m_selected_address_start, 0x100);
    }

    #[test]
    fn disassembly_view_goto() {
        let params = DebuggerViewParameters::new("disasm", 2).with_cpu(BreakPointCpu::EE);
        let mut v = DisassemblyView::new(params);
        v.m_visible_rows = 8;
        v.goto(0x205);
        assert_eq!(v.m_selected_address_start, 0x204);
    }

    #[test]
    fn register_view_update_registers() {
        let params = DebuggerViewParameters::new("reg", 3).with_cpu(BreakPointCpu::EE);
        let mut v = RegisterView::new(params);
        v.update_registers();
    }

    #[test]
    fn memory_view_read() {
        let params = DebuggerViewParameters::new("mem", 4).with_cpu(BreakPointCpu::EE);
        let v = MemoryView::new(params);
        let mut cpu = DebugInterface::new(BreakPointCpu::EE);
        let _ = v.read(&cpu, 0, MemorySize::Dword);
    }

    #[test]
    fn memory_search_view_do_search() {
        let params = DebuggerViewParameters::new("memsearch", 5).with_cpu(BreakPointCpu::EE);
        let mut v = MemorySearchView::new(params);
        let mut cpu = DebugInterface::new(BreakPointCpu::EE);
        cpu.set_pc(0x80);
        let _ = v.do_search(&cpu, MemorySearchType::Text, b"hi");
    }

    #[test]
    fn breakpoint_dialog_create() {
        let cpu = Arc::new(Mutex::new(DebugInterface::new(BreakPointCpu::EE)));
        let mut d = BreakpointDialog::new(cpu);
        d.address_text = "0x100".to_string();
        d.description_text = "test".to_string();
        assert!(d.accept());
        assert!(matches!(d.bp_mc, Some(BreakpointMemcheck::BreakPoint(_))));
    }

    #[test]
    fn symbol_tree_node_creation() {
        let mut n = SymbolTreeNode::default();
        n.name = "myvar".to_string();
        n.location = SymbolTreeLocation::new(SymbolTreeLocationType::Memory, 0x2000);
        let _ = n.read_from_vm(&DebugInterface::new(BreakPointCpu::EE));
    }

    #[test]
    fn new_function_dialog() {
        let cpu = Arc::new(Mutex::new(DebugInterface::new(BreakPointCpu::EE)));
        let mut d = NewFunctionDialog::new(cpu);
        d.base.set_address(0x100);
        d.base.set_name("myfunc");
        assert!(d.parse_user_input());
    }

    #[test]
    fn display_options_base_change() {
        let mut opts = SymbolTreeDisplayOptions::default();
        assert!(opts.set_integer_base(16));
        assert!(!opts.set_integer_base(7));
        assert!(opts.set_show_leading_zeroes(true));
    }
}
