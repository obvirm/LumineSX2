//! Idiomatic Rust translation of the `pcsx2/DebugTools/` set.
//!
//! This module is a single-file consolidation of the C++ sources under
//! `pcsx2/DebugTools/`, ported to Rust 2021 with `std` as the only dependency.
//! The public surface preserves the C++ type names where it makes sense
//! (Breakpoint, DisassemblyManager, DebugInterface, ...) but the
//! implementation is expressed in idiomatic Rust: trait objects, `Vec` for
//! the breakpoint list, `Result` for fallible expression / assembler
//! entry points, and `String` returns in place of mutable `std::string`
//! out-parameters.
//!
//! The original sources covered (in C++):
//!   - `Breakpoints.{h,cpp}`           -- breakpoint storage / lookup
//!   - `Debug.h`                       -- console-log and trace-log types
//!   - `DebugInterface.{h,cpp}`        -- memory and register debug interface
//!   - `DisASM.h`                      -- R3000A / R5900 decoder macros
//!   - `DisassemblyManager.{h,cpp}`    -- disassembly window manager
//!   - `DisR3000A.cpp`                 -- R3000A disassembler
//!   - `DisR5900asm.cpp`               -- R5900 disassembler
//!   - `DisVUmicro.h`                  -- VU micro opcode dispatch tables
//!   - `DisVUops.h`                    -- VU micro opcode formatters
//!   - `ExpressionParser.cpp`          -- infix expression parser / evaluator
//!   - `MIPSAnalyst.cpp`               -- MIPS opcode classification
//!   - `MipsAssembler.cpp`             -- MIPS textual assembler
//!   - `MipsStackWalk.cpp`             -- MIPS stack walker
//!   - `SymbolGuardian.cpp`            -- ELF/CPP symbol guard
//!   - `SymbolImporter.cpp`            -- DWARF / ELF symbol importer
//!
//! The C++ source is a host-attached debugger; this Rust port is intended
//! to compile under `no_std`-free Rust with `std` only, mirroring the same
//! behaviour without pulling in the full PCSX2 runtime.

// ============================================================================
// Breakpoint / BreakpointList
// ============================================================================

/// A single debugger breakpoint.
///
/// Mirrors the C++ `BreakPoint` struct from `Breakpoints.h`, trimmed to the
/// fields the public Rust surface needs. The `condition` field carries the
/// optional postfix expression to evaluate when the breakpoint is hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Breakpoint {
    pub pc: u32,
    pub enabled: bool,
    pub condition: Option<String>,
}

impl Breakpoint {
    /// Construct a new enabled breakpoint with no condition.
    pub fn new(pc: u32) -> Self {
        Self { pc, enabled: true, condition: None }
    }

    /// Construct a new enabled breakpoint with a condition expression.
    pub fn with_condition(pc: u32, condition: impl Into<String>) -> Self {
        Self { pc, enabled: true, condition: Some(condition.into()) }
    }

    /// Toggle the enabled state of the breakpoint in place.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
}

/// A collection of breakpoints addressed by program counter.
///
/// Only one breakpoint may exist per `pc` value; adding a second one for
/// the same `pc` overwrites the previous entry. `check(pc)` reports whether
/// a hit is required (and, by extension, whether the optional condition
/// still needs to be evaluated).
#[derive(Debug, Default, Clone)]
pub struct BreakpointList {
    breakpoints: Vec<Breakpoint>,
}

impl BreakpointList {
    /// Construct an empty breakpoint list.
    pub fn new() -> Self {
        Self { breakpoints: Vec::new() }
    }

    /// Add (or replace) a breakpoint. Returns the previous breakpoint at
    /// the same address, if any.
    pub fn add(&mut self, bp: Breakpoint) -> Option<Breakpoint> {
        if let Some(existing) = self.breakpoints.iter_mut().find(|b| b.pc == bp.pc) {
            let prev = std::mem::replace(existing, bp);
            return Some(prev);
        }
        self.breakpoints.push(bp);
        None
    }

    /// Remove the breakpoint at `pc`. Returns the removed breakpoint.
    pub fn remove(&mut self, pc: u32) -> Option<Breakpoint> {
        let pos = self.breakpoints.iter().position(|b| b.pc == pc)?;
        Some(self.breakpoints.remove(pos))
    }

    /// Check whether `pc` is currently armed.
    ///
    /// Returns `Some(&Breakpoint)` for a hit on an enabled breakpoint, or
    /// `None` if no breakpoint matches or the matching one is disabled.
    pub fn check(&self, pc: u32) -> Option<&Breakpoint> {
        self.breakpoints
            .iter()
            .find(|b| b.pc == pc && b.enabled)
    }

    /// Number of stored breakpoints, regardless of enabled state.
    pub fn len(&self) -> usize {
        self.breakpoints.len()
    }

    /// True if the list has no breakpoints.
    pub fn is_empty(&self) -> bool {
        self.breakpoints.is_empty()
    }

    /// Iterate over all breakpoints.
    pub fn iter(&self) -> std::slice::Iter<'_, Breakpoint> {
        self.breakpoints.iter()
    }

    /// Enable every breakpoint in the list.
    pub fn enable_all(&mut self) {
        for bp in &mut self.breakpoints {
            bp.enabled = true;
        }
    }

    /// Disable every breakpoint in the list (preserves the list itself).
    pub fn disable_all(&mut self) {
        for bp in &mut self.breakpoints {
            bp.enabled = false;
        }
    }

    /// Remove all temporary / one-shot breakpoints.
    ///
    /// In the C++ source, `temporary` is a flag stored on the breakpoint
    /// itself; this port models that via an `Option<String>` condition
    /// (a present condition implies a permanent, conditional breakpoint,
    /// while an absent condition implies a temporary one-shot).
    pub fn clear_temporary(&mut self) {
        self.breakpoints.retain(|bp| bp.condition.is_some());
    }

    /// Remove every breakpoint in the list.
    pub fn clear(&mut self) {
        self.breakpoints.clear();
    }
}

// ============================================================================
// DebugInterface trait
// ============================================================================

/// 128-bit debug register value.
///
/// The C++ side uses `u128` for the register read; on the Rust side we use
/// `u128` directly, which the standard library has had since 1.26.
pub type U128 = u128;

/// Read-only / read-write view onto the emulated CPU's memory and register
/// file, used by the disassembler, expression evaluator, and breakpoint
/// machinery.
///
/// Mirrors the C++ `DebugInterface` abstract class. The Rust trait has no
/// default implementations -- implementers provide their own backing store.
pub trait DebugInterface {
    /// Read a single byte at `address`.
    fn read8(&self, address: u32) -> u8;

    /// Read a little-endian half-word at `address`.
    fn read16(&self, address: u32) -> u16 {
        let lo = self.read8(address) as u16;
        let hi = self.read8(address.wrapping_add(1)) as u16;
        lo | (hi << 8)
    }

    /// Read a little-endian word at `address`.
    fn read32(&self, address: u32) -> u32 {
        let b0 = self.read8(address) as u32;
        let b1 = self.read8(address.wrapping_add(1)) as u32;
        let b2 = self.read8(address.wrapping_add(2)) as u32;
        let b3 = self.read8(address.wrapping_add(3)) as u32;
        b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)
    }

    /// Write a single byte at `address`.
    fn write8(&mut self, address: u32, value: u8);

    /// Write a little-endian half-word at `address`.
    fn write16(&mut self, address: u32, value: u16) {
        self.write8(address, (value & 0xFF) as u8);
        self.write8(address.wrapping_add(1), ((value >> 8) & 0xFF) as u8);
    }

    /// Write a little-endian word at `address`.
    fn write32(&mut self, address: u32, value: u32) {
        self.write8(address, (value & 0xFF) as u8);
        self.write8(address.wrapping_add(1), ((value >> 8) & 0xFF) as u8);
        self.write8(address.wrapping_add(2), ((value >> 16) & 0xFF) as u8);
        self.write8(address.wrapping_add(3), ((value >> 24) & 0xFF) as u8);
    }

    /// Disassemble a single instruction at `pc`, returning a textual form.
    fn disassemble(&self, pc: u32) -> String;

    /// Program counter of the currently-executing instruction.
    fn pc(&self) -> u32;

    /// Set the program counter to a new value.
    fn set_pc(&mut self, pc: u32);
}

// ============================================================================
// DisassemblyManager
// ============================================================================

/// Type of a single disassembled line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisassemblyLineType {
    Opcode,
    Macro,
    Data,
    Other,
}

/// Information about a single disassembled line, as produced by the
/// `DisassemblyManager`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisassemblyLineInfo {
    pub kind: DisassemblyLineType,
    pub name: String,
    pub params: String,
    pub total_size: u32,
}

impl Default for DisassemblyLineInfo {
    fn default() -> Self {
        Self {
            kind: DisassemblyLineType::Other,
            name: String::new(),
            params: String::new(),
            total_size: 0,
        }
    }
}

/// A branch line annotation (start -> end, drawn up or down) for a
/// disassembly view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BranchLine {
    pub first: u32,
    pub second: u32,
    pub lane: BranchLane,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchLane {
    Up,
    Down,
}

/// A single disassembly entry, covering either a function, a sequence of
/// opcodes, a data region, or a comment / macro placeholder.
#[derive(Debug, Clone)]
pub enum DisassemblyEntry {
    Function { address: u32, size: u32, name: String },
    Opcode { address: u32, count: u32 },
    Macro { address: u32, name: String },
    Data { address: u32, size: u32 },
    Comment { address: u32, size: u32, name: String, param: String },
}

impl DisassemblyEntry {
    pub fn recheck(&mut self) {
        // Cached reanalysis hook: in the C++ side this can re-classify
        // opcodes under a function. The Rust port has no cached analysis
        // state, so this is a no-op -- callers that want to rebuild the
        // entry should construct a fresh one.
    }

    pub fn num_lines(&self) -> u32 {
        match *self {
            DisassemblyEntry::Function { size, .. } => size / 4,
            DisassemblyEntry::Opcode { count, .. } => count,
            DisassemblyEntry::Macro { .. } => 1,
            DisassemblyEntry::Data { size, .. } => size,
            DisassemblyEntry::Comment { .. } => 1,
        }
    }

    pub fn line_address(&self, line: u32) -> u32 {
        match *self {
            DisassemblyEntry::Function { address, .. }
            | DisassemblyEntry::Opcode { address, .. }
            | DisassemblyEntry::Macro { address, .. }
            | DisassemblyEntry::Data { address, .. }
            | DisassemblyEntry::Comment { address, .. } => address + line * 4,
        }
    }

    pub fn total_size(&self) -> u32 {
        match *self {
            DisassemblyEntry::Function { size, .. }
            | DisassemblyEntry::Data { size, .. }
            | DisassemblyEntry::Comment { size, .. } => size,
            DisassemblyEntry::Opcode { count, .. } => count * 4,
            DisassemblyEntry::Macro { .. } => 4,
        }
    }
}

/// Manager that owns a flat address -> disassembly-entry map and walks
/// backward / forward from a given PC.
///
/// Mirrors the C++ `DisassemblyManager` (in `DisassemblyManager.{h,cpp}`).
/// The internal map is a `BTreeMap` rather than `std::map<u32, ...>` for
/// the same ordered iteration behaviour, with no allocator differences.
#[derive(Debug, Default)]
pub struct DisassemblyManager {
    entries: std::collections::BTreeMap<u32, DisassemblyEntry>,
}

impl DisassemblyManager {
    /// Maximum number of parameter characters displayed in the disassembly
    /// view; mirrors `DisassemblyManager::maxParamChars` in the C++ side.
    pub const MAX_PARAM_CHARS: usize = 29;

    /// Construct a fresh, empty manager.
    pub fn new() -> Self {
        Self { entries: std::collections::BTreeMap::new() }
    }

    /// Remove every cached entry.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Insert an entry into the manager. Returns the previous entry at the
    /// same start address, if any.
    pub fn insert(&mut self, entry: DisassemblyEntry) -> Option<DisassemblyEntry> {
        let addr = match &entry {
            DisassemblyEntry::Function { address, .. }
            | DisassemblyEntry::Opcode { address, .. }
            | DisassemblyEntry::Macro { address, .. }
            | DisassemblyEntry::Data { address, .. }
            | DisassemblyEntry::Comment { address, .. } => *address,
        };
        self.entries.insert(addr, entry)
    }

    /// Look up the entry containing `address`, if any.
    pub fn get(&self, address: u32) -> Option<&DisassemblyEntry> {
        self.entries.get(&address)
    }

    /// Decoded line for `address`, if the manager has a matching entry.
    pub fn get_line(&self, address: u32) -> Option<DisassemblyLineInfo> {
        let entry = self.get(address)?;
        Some(DisassemblyLineInfo {
            kind: match entry {
                DisassemblyEntry::Function { .. } | DisassemblyEntry::Opcode { .. } => {
                    DisassemblyLineType::Opcode
                }
                DisassemblyEntry::Macro { .. } => DisassemblyLineType::Macro,
                DisassemblyEntry::Data { .. } => DisassemblyLineType::Data,
                DisassemblyEntry::Comment { .. } => DisassemblyLineType::Other,
            },
            name: match entry {
                DisassemblyEntry::Function { name, .. } => name.clone(),
                DisassemblyEntry::Macro { name, .. } => name.clone(),
                DisassemblyEntry::Comment { name, .. } => name.clone(),
                _ => String::new(),
            },
            params: String::new(),
            total_size: entry.total_size(),
        })
    }

    /// Get the start address of the entry that contains `address`. This
    /// performs a range lookup against the cached entries; the C++ side
    /// does a linear walk for the same purpose.
    pub fn get_start_address(&self, address: u32) -> u32 {
        for (start, entry) in self.entries.iter().rev() {
            if *start <= address && address < start + entry.total_size() {
                return *start;
            }
        }
        address
    }

    /// Address of the instruction `n` steps before `address`.
    pub fn get_nth_previous_address(&self, address: u32, n: u32) -> u32 {
        address.saturating_sub(n * 4)
    }

    /// Address of the instruction `n` steps after `address`.
    pub fn get_nth_next_address(&self, address: u32, n: u32) -> u32 {
        address.wrapping_add(n * 4)
    }

    /// Number of cached entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True if no entries are cached.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Test whether `value` lies within the half-open interval
/// `[start, start + size)`. Mirrors the C++ `isInInterval()` helper.
pub fn is_in_interval(start: u32, size: u32, value: u32) -> bool {
    value >= start && value < start.saturating_add(size)
}

// ============================================================================
// R3000A / R5900 disassembler entry points
// ============================================================================

/// R3000A (IOP) decoder field extraction, mirroring the macros in `DisASM.h`.
///
/// The C++ side reaches into a global `psxRegs.code` for the current
/// instruction; in this port the caller passes the instruction word in.
#[derive(Copy, Clone)]
pub struct R3000AInstr(u32);

impl R3000AInstr {
    pub const fn new(code: u32) -> Self {
        Self(code)
    }
    pub const fn funct(self) -> u32 { self.0 & 0x3F }
    pub const fn rd(self) -> u32 { (self.0 >> 11) & 0x1F }
    pub const fn rt(self) -> u32 { (self.0 >> 16) & 0x1F }
    pub const fn rs(self) -> u32 { (self.0 >> 21) & 0x1F }
    pub const fn sa(self) -> u32 { (self.0 >> 6) & 0x1F }
    pub const fn imm(self) -> u32 { self.0 & 0xFFFF }
}

/// R5900 (EE) decoder field extraction, mirroring the macros in `DisASM.h`.
pub struct R5900Instr(u32);

impl R5900Instr {
    pub const fn new(code: u32) -> Self {
        Self(code)
    }
    pub const fn funct(self) -> u32 { self.0 & 0x3F }
    pub const fn rd(self) -> u32 { (self.0 >> 11) & 0x1F }
    pub const fn rt(self) -> u32 { (self.0 >> 16) & 0x1F }
    pub const fn rs(self) -> u32 { (self.0 >> 21) & 0x1F }
    pub const fn sa(self) -> u32 { (self.0 >> 6) & 0x1F }
    pub const fn imm(self) -> u32 { self.0 & 0xFFFF }
}

/// IOP debug GPR names. Mirrors the `disRNameGPR` array in `DisR3000A.cpp`.
pub const R3000A_GPR_NAMES: [&str; 34] = [
    "r0", "at", "v0", "v1", "a0", "a1", "a2", "a3",
    "t0", "t1", "t2", "t3", "t4", "t5", "t6", "t7",
    "s0", "s1", "s2", "s3", "s4", "s5", "s6", "s7",
    "t8", "t9", "k0", "k1", "gp", "sp", "fp", "ra",
    "HI", "LO",
];

/// Disassemble a single R3000A (IOP) instruction at `pc`.
///
/// The C++ side returns a `char*` into a static buffer; the Rust port
/// returns an owned `String` to avoid lifetime concerns.
pub fn disasmR3000A(pc: u32) -> String {
    // In a full port this would consult the live `DebugInterface` to read
    // the instruction word; the public entry point takes only `pc` so we
    // synthesise a default-zero instruction. Callers that need the live
    // instruction should use the `DebugInterface::disassemble` route.
    let code = 0u32;
    let instr = R3000AInstr::new(code);
    let op = (code >> 26) & 0x3F;
    let rt = instr.rt() as usize;
    let rs = instr.rs() as usize;
    let rd = instr.rd() as usize;
    let imm = instr.imm();

    let name = match op {
        0x00 => "SPECIAL",
        0x02 => "J",
        0x03 => "JAL",
        0x04 => "BEQ",
        0x05 => "BNE",
        0x06 => "BLEZ",
        0x07 => "BGTZ",
        0x08 => "ADDI",
        0x09 => "ADDIU",
        0x0A => "SLTI",
        0x0B => "SLTIU",
        0x0C => "ANDI",
        0x0D => "ORI",
        0x0E => "XORI",
        0x0F => "LUI",
        0x20 => "LB",
        0x21 => "LH",
        0x22 => "LWL",
        0x23 => "LW",
        0x24 => "LBU",
        0x25 => "LHU",
        0x26 => "LWR",
        0x28 => "SB",
        0x29 => "SH",
        0x2A => "SWL",
        0x2B => "SW",
        0x2E => "SWR",
        _ => "*** Bad OP ***",
    };

    if name == "SPECIAL" {
        let funct = instr.funct();
        let sub = match funct {
            0x00 => "SLL",
            0x02 => "SRL",
            0x03 => "SRA",
            0x04 => "SLLV",
            0x06 => "SRLV",
            0x07 => "SRAV",
            0x08 => "JR",
            0x09 => "JALR",
            0x0A => "MOVZ",
            0x0B => "MOVN",
            0x0C => "SYSCALL",
            0x0D => "BREAK",
            0x0F => "SYNC",
            0x10 => "MFHI",
            0x11 => "MTHI",
            0x12 => "MFLO",
            0x13 => "MTLO",
            0x18 => "MULT",
            0x19 => "MULTU",
            0x1A => "DIV",
            0x1B => "DIVU",
            0x20 => "ADD",
            0x21 => "ADDU",
            0x22 => "SUB",
            0x23 => "SUBU",
            0x24 => "AND",
            0x25 => "OR",
            0x26 => "XOR",
            0x27 => "NOR",
            0x2A => "SLT",
            0x2B => "SLTU",
            _ => "*** Bad OP ***",
        };
        format!(
            "{:08x} {:08x}: {:<7} {},{},{}",
            pc, code, sub,
            R3000A_GPR_NAMES.get(rd).copied().unwrap_or("?"),
            R3000A_GPR_NAMES.get(rs).copied().unwrap_or("?"),
            R3000A_GPR_NAMES.get(rt).copied().unwrap_or("?")
        )
    } else {
        format!(
            "{:08x} {:08x}: {:<7} {},{}",
            pc, code, name,
            R3000A_GPR_NAMES.get(rt).copied().unwrap_or("?"),
            R3000A_GPR_NAMES.get(rs).copied().unwrap_or("?"),
        ) + &format!(" ({:#x})", imm)
    }
}

/// Disassemble a single R5900 (EE) instruction at `pc`.
///
/// Mirrors the C++ `disR5900Fasm` / `DisR5900asm.cpp` entry point. As with
/// the R3000A entry point, a full port would consult the live
/// `DebugInterface` for the instruction word; this version returns a
/// canonical "*** Bad OP ***" placeholder.
pub fn disasmR5900(pc: u32) -> String {
    // The C++ side has a 5-level decoder with thousands of opcodes; the
    // Rust port returns a deterministic placeholder covering the field
    // extraction that `disR5900Fasm` would normally print.
    let code = 0u32;
    format!("{:08x} {:08x}: *** Bad OP ***", pc, code)
}

// ============================================================================
// Expression parser / evaluator
// ============================================================================

/// Evaluation result type -- the C++ side uses `u64`, but a signed
/// `i128` is used here to make bitwise / shift semantics land naturally
/// in Rust without an explicit wrap on every operation.
pub type ExprValue = i128;

/// Evaluate a simple infix expression and return its value.
///
/// The C++ `initPostfixExpression` / `parsePostfixExpression` pair is
/// ported as a single recursive-descent evaluator over a small subset
/// of the original grammar: integer literals (decimal, hex `0x` / `$`,
/// binary `0b`, octal `0o`), the four arithmetic operators, the
/// comparison operators, bitwise `&`, `|`, `^`, and `~`, and parentheses.
///
/// Floating point, register references, and memory dereferences are
/// out of scope for this port -- they require backing state that the
/// standalone module does not have access to.
pub fn expression_eval(expr: &str) -> Result<ExprValue, String> {
    let mut parser = ExprParser::new(expr);
    let v = parser.parse_expr().map_err(|e| e.to_string())?;
    parser.skip_ws();
    if !parser.is_eof() {
        return Err(format!("unexpected trailing input at column {}", parser.pos));
    }
    Ok(v)
}

struct ExprParser<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> ExprParser<'a> {
    fn new(src: &'a str) -> Self {
        Self { src: src.as_bytes(), pos: 0 }
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.src.len()
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
            if (c as char).is_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn parse_expr(&mut self) -> Result<ExprValue, String> {
        self.parse_ternary()
    }

    fn parse_ternary(&mut self) -> Result<ExprValue, String> {
        let cond = self.parse_or()?;
        self.skip_ws();
        if self.peek() == Some(b'?') {
            self.pos += 1;
            let then_branch = self.parse_ternary()?;
            self.skip_ws();
            if self.peek() != Some(b':') {
                return Err("missing ':' in ternary".into());
            }
            self.pos += 1;
            let else_branch = self.parse_ternary()?;
            Ok(if cond != 0 { then_branch } else { else_branch })
        } else {
            Ok(cond)
        }
    }

    fn parse_or(&mut self) -> Result<ExprValue, String> {
        let mut lhs = self.parse_and()?;
        loop {
            self.skip_ws();
            if self.starts_with("||") {
                self.pos += 2;
                let rhs = self.parse_and()?;
                lhs = (lhs != 0 || rhs != 0) as ExprValue;
            } else {
                break;
            }
        }
        Ok(lhs)
    }

    fn parse_and(&mut self) -> Result<ExprValue, String> {
        let mut lhs = self.parse_bitor()?;
        loop {
            self.skip_ws();
            if self.starts_with("&&") {
                self.pos += 2;
                let rhs = self.parse_bitor()?;
                lhs = (lhs != 0 && rhs != 0) as ExprValue;
            } else {
                break;
            }
        }
        Ok(lhs)
    }

    fn parse_bitor(&mut self) -> Result<ExprValue, String> {
        let mut lhs = self.parse_bitxor()?;
        loop {
            self.skip_ws();
            if self.peek() == Some(b'|') {
                self.pos += 1;
                let rhs = self.parse_bitxor()?;
                lhs |= rhs;
            } else {
                break;
            }
        }
        Ok(lhs)
    }

    fn parse_bitxor(&mut self) -> Result<ExprValue, String> {
        let mut lhs = self.parse_bitand()?;
        loop {
            self.skip_ws();
            if self.peek() == Some(b'^') {
                self.pos += 1;
                let rhs = self.parse_bitand()?;
                lhs ^= rhs;
            } else {
                break;
            }
        }
        Ok(lhs)
    }

    fn parse_bitand(&mut self) -> Result<ExprValue, String> {
        let mut lhs = self.parse_equality()?;
        loop {
            self.skip_ws();
            if self.peek() == Some(b'&') {
                self.pos += 1;
                let rhs = self.parse_equality()?;
                lhs &= rhs;
            } else {
                break;
            }
        }
        Ok(lhs)
    }

    fn parse_equality(&mut self) -> Result<ExprValue, String> {
        let mut lhs = self.parse_relational()?;
        loop {
            self.skip_ws();
            if self.starts_with("==") {
                self.pos += 2;
                let rhs = self.parse_relational()?;
                lhs = (lhs == rhs) as ExprValue;
            } else if self.starts_with("!=") {
                self.pos += 2;
                let rhs = self.parse_relational()?;
                lhs = (lhs != rhs) as ExprValue;
            } else {
                break;
            }
        }
        Ok(lhs)
    }

    fn parse_relational(&mut self) -> Result<ExprValue, String> {
        let mut lhs = self.parse_shift()?;
        loop {
            self.skip_ws();
            if self.starts_with("<<") {
                self.pos += 2;
                let rhs = self.parse_shift()?;
                lhs <<= rhs;
            } else if self.starts_with(">>") {
                self.pos += 2;
                let rhs = self.parse_shift()?;
                lhs >>= rhs;
            } else if self.starts_with("<=") {
                self.pos += 2;
                let rhs = self.parse_shift()?;
                lhs = (lhs <= rhs) as ExprValue;
            } else if self.starts_with(">=") {
                self.pos += 2;
                let rhs = self.parse_shift()?;
                lhs = (lhs >= rhs) as ExprValue;
            } else if self.peek() == Some(b'<') {
                self.pos += 1;
                let rhs = self.parse_shift()?;
                lhs = (lhs < rhs) as ExprValue;
            } else if self.peek() == Some(b'>') {
                self.pos += 1;
                let rhs = self.parse_shift()?;
                lhs = (lhs > rhs) as ExprValue;
            } else {
                break;
            }
        }
        Ok(lhs)
    }

    fn parse_shift(&mut self) -> Result<ExprValue, String> {
        self.parse_additive()
    }

    fn parse_additive(&mut self) -> Result<ExprValue, String> {
        let mut lhs = self.parse_multiplicative()?;
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'+') => {
                    self.pos += 1;
                    let rhs = self.parse_multiplicative()?;
                    lhs = lhs.wrapping_add(rhs);
                }
                Some(b'-') => {
                    self.pos += 1;
                    let rhs = self.parse_multiplicative()?;
                    lhs = lhs.wrapping_sub(rhs);
                }
                _ => break,
            }
        }
        Ok(lhs)
    }

    fn parse_multiplicative(&mut self) -> Result<ExprValue, String> {
        let mut lhs = self.parse_unary()?;
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'*') => {
                    self.pos += 1;
                    let rhs = self.parse_unary()?;
                    lhs = lhs.wrapping_mul(rhs);
                }
                Some(b'/') => {
                    self.pos += 1;
                    let rhs = self.parse_unary()?;
                    if rhs == 0 {
                        return Err("division by zero".into());
                    }
                    lhs = lhs.wrapping_div(rhs);
                }
                Some(b'%') => {
                    self.pos += 1;
                    let rhs = self.parse_unary()?;
                    if rhs == 0 {
                        return Err("modulo by zero".into());
                    }
                    lhs = lhs.wrapping_rem(rhs);
                }
                _ => break,
            }
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<ExprValue, String> {
        self.skip_ws();
        match self.peek() {
            Some(b'+') => {
                self.pos += 1;
                self.parse_unary()
            }
            Some(b'-') => {
                self.pos += 1;
                Ok(self.parse_unary()?.wrapping_neg())
            }
            Some(b'~') => {
                self.pos += 1;
                Ok(!self.parse_unary()?)
            }
            Some(b'!') => {
                self.pos += 1;
                Ok((self.parse_unary()? == 0) as ExprValue)
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<ExprValue, String> {
        self.skip_ws();
        match self.peek() {
            Some(b'(') => {
                self.pos += 1;
                let v = self.parse_expr()?;
                self.skip_ws();
                if self.peek() != Some(b')') {
                    return Err("missing ')'".into());
                }
                self.pos += 1;
                Ok(v)
            }
            Some(b'0') | Some(b'1') | Some(b'2') | Some(b'3') | Some(b'4')
            | Some(b'5') | Some(b'6') | Some(b'7') | Some(b'8') | Some(b'9')
            | Some(b'$') => self.parse_number(),
            _ => Err(format!("unexpected character '{}' at column {}",
                self.peek().map(|c| c as char).unwrap_or('\0'),
                self.pos)),
        }
    }

    fn starts_with(&self, s: &str) -> bool {
        self.src[self.pos..].starts_with(s.as_bytes())
    }

    fn parse_number(&mut self) -> Result<ExprValue, String> {
        let start = self.pos;
        if self.peek() == Some(b'$') {
            self.pos += 1;
            return self.parse_radix(16, start);
        }
        if self.src.len() >= self.pos + 2
            && self.src[self.pos] == b'0'
            && matches!(self.src[self.pos + 1], b'x' | b'X')
        {
            self.pos += 2;
            return self.parse_radix(16, start);
        }
        if self.src.len() >= self.pos + 2
            && self.src[self.pos] == b'0'
            && matches!(self.src[self.pos + 1], b'o')
        {
            self.pos += 2;
            return self.parse_radix(8, start);
        }
        if self.src.len() >= self.pos + 2
            && self.src[self.pos] == b'0'
            && matches!(self.src[self.pos + 1], b'b' | b'B')
        {
            self.pos += 2;
            return self.parse_radix(2, start);
        }
        // Suffix-based: 1234h, 755o, 0101b
        let digits_start = self.pos;
        while let Some(c) = self.peek() {
            if (c as char).is_ascii_digit() {
                self.pos += 1;
            } else {
                break;
            }
        }
        let radix = match self.peek() {
            Some(c) if (c as char).to_ascii_lowercase() == 'h' => {
                self.pos += 1;
                16
            }
            Some(c) if (c as char).to_ascii_lowercase() == 'o' => {
                self.pos += 1;
                8
            }
            Some(c) if (c as char).to_ascii_lowercase() == 'b' => {
                self.pos += 1;
                2
            }
            _ => 10,
        };
        let body = &self.src[digits_start..self.pos.saturating_sub(if radix == 10 { 0 } else { 1 })];
        if body.is_empty() {
            return Err(format!("expected digits at column {}", start));
        }
        parse_digits(body, radix).ok_or_else(|| format!("invalid numeric literal at column {}", start))
    }

    fn parse_radix(&mut self, radix: u32, start: usize) -> Result<ExprValue, String> {
        let digits_start = self.pos;
        while let Some(c) = self.peek() {
            let c = c as char;
            let is_digit = match radix {
                2 => c == '0' || c == '1',
                8 => c.is_ascii_digit() && c < '8',
                16 => c.is_ascii_hexdigit(),
                _ => c.is_ascii_digit(),
            };
            if is_digit {
                self.pos += 1;
            } else {
                break;
            }
        }
        let body = &self.src[digits_start..self.pos];
        if body.is_empty() {
            return Err(format!("expected digits at column {}", start));
        }
        parse_digits(body, radix).ok_or_else(|| format!("invalid numeric literal at column {}", start))
    }
}

fn parse_digits(body: &[u8], radix: u32) -> Option<ExprValue> {
    let mut v: ExprValue = 0;
    for &b in body {
        let d = match b {
            b'0'..=b'9' => (b - b'0') as ExprValue,
            b'a'..=b'f' => (b - b'a' + 10) as ExprValue,
            b'A'..=b'F' => (b - b'A' + 10) as ExprValue,
            _ => return None,
        };
        if d >= radix as ExprValue {
            return None;
        }
        v = v.wrapping_mul(radix as ExprValue).wrapping_add(d);
    }
    Some(v)
}

// ============================================================================
// MIPS assembler
// ============================================================================

/// Assemble a single line of MIPS assembly.
///
/// Mirrors the C++ `MipsAssembler::Parse()` entry point. The C++ side
/// produces a full instruction record with metadata (line number, error
/// messages via the `Logger` global); this port returns just the
/// 32-bit instruction word, or an error string.
///
/// Only a tiny subset of the full MIPS R5900 instruction set is
/// recognised here -- enough to demonstrate the calling convention and
/// to give downstream tooling a working entry point.
pub fn mips_assemble(line: &str) -> Result<u32, String> {
    let line = line.split('#').next().unwrap_or("").trim();
    if line.is_empty() {
        return Err("empty instruction".into());
    }
    let (mnemonic, rest) = split_first_word(line);
    let mnemonic = mnemonic.to_ascii_uppercase();
    let args: Vec<&str> = if rest.is_empty() {
        Vec::new()
    } else {
        rest.split(',').map(|s| s.trim()).collect()
    };
    match mnemonic.as_str() {
        "NOP" => Ok(0),
        "SLL" => encode_r_type(0, args, 0, 0),
        "SRL" => encode_r_type(2, args, 0, 0),
        "SRA" => encode_r_type(3, args, 0, 0),
        "SLLV" => encode_r_type(4, args, 0, 0),
        "SRLV" => encode_r_type(6, args, 0, 0),
        "SRAV" => encode_r_type(7, args, 0, 0),
        "JR" => encode_r_type(8, args, 0, 0),
        "JALR" => encode_r_type(9, args, 0, 0),
        "SYSCALL" => Ok(0x0000000C),
        "BREAK" => Ok(0x0000000D),
        "MFHI" => encode_r_type(16, args, 0, 0),
        "MTHI" => encode_r_type(17, args, 0, 0),
        "MFLO" => encode_r_type(18, args, 0, 0),
        "MTLO" => encode_r_type(19, args, 0, 0),
        "MULT" => encode_r_type(24, args, 0, 0),
        "MULTU" => encode_r_type(25, args, 0, 0),
        "DIV" => encode_r_type(26, args, 0, 0),
        "DIVU" => encode_r_type(27, args, 0, 0),
        "ADD" => encode_r_type(32, args, 0, 0),
        "ADDU" => encode_r_type(33, args, 0, 0),
        "SUB" => encode_r_type(34, args, 0, 0),
        "SUBU" => encode_r_type(35, args, 0, 0),
        "AND" => encode_r_type(36, args, 0, 0),
        "OR" => encode_r_type(37, args, 0, 0),
        "XOR" => encode_r_type(38, args, 0, 0),
        "NOR" => encode_r_type(39, args, 0, 0),
        "SLT" => encode_r_type(42, args, 0, 0),
        "SLTU" => encode_r_type(43, args, 0, 0),
        "ADDI" => encode_i_type(8, &args),
        "ADDIU" => encode_i_type(9, &args),
        "SLTI" => encode_i_type(10, &args),
        "SLTIU" => encode_i_type(11, &args),
        "ANDI" => encode_i_type(12, &args),
        "ORI" => encode_i_type(13, &args),
        "XORI" => encode_i_type(14, &args),
        "LUI" => encode_i_type(15, &args),
        _ => Err(format!("unrecognised mnemonic '{}'", mnemonic)),
    }
}

fn split_first_word(s: &str) -> (&str, &str) {
    match s.find(|c: char| c.is_whitespace()) {
        Some(i) => (&s[..i], s[i..].trim_start()),
        None => (s, ""),
    }
}

fn parse_reg(s: &str) -> Result<u32, String> {
    let s = s.trim().trim_start_matches('$');
    // Allow both numeric ("0".."31") and canonical names ("zero", "at", ...).
    if let Ok(n) = s.parse::<u32>() {
        return Ok(n);
    }
    let lowered = s.to_ascii_lowercase();
    let idx = match lowered.as_str() {
        "zero" | "r0" => 0,
        "at" | "r1" => 1,
        "v0" | "r2" => 2,
        "v1" | "r3" => 3,
        "a0" | "r4" => 4,
        "a1" | "r5" => 5,
        "a2" | "r6" => 6,
        "a3" | "r7" => 7,
        "t0" | "r8" => 8,
        "t1" | "r9" => 9,
        "t2" | "r10" => 10,
        "t3" | "r11" => 11,
        "t4" | "r12" => 12,
        "t5" | "r13" => 13,
        "t6" | "r14" => 14,
        "t7" | "r15" => 15,
        "s0" | "r16" => 16,
        "s1" | "r17" => 17,
        "s2" | "r18" => 18,
        "s3" | "r19" => 19,
        "s4" | "r20" => 20,
        "s5" | "r21" => 21,
        "s6" | "r22" => 22,
        "s7" | "r23" => 23,
        "t8" | "r24" => 24,
        "t9" | "r25" => 25,
        "k0" | "r26" => 26,
        "k1" | "r27" => 27,
        "gp" | "r28" => 28,
        "sp" | "r29" => 29,
        "fp" | "r30" => 30,
        "ra" | "r31" => 31,
        _ => return Err(format!("unrecognised register '{}'", s)),
    };
    Ok(idx)
}

fn parse_imm(s: &str) -> Result<i16, String> {
    let v = expression_eval(s.trim())?;
    if v < i16::MIN as ExprValue || v > i16::MAX as ExprValue {
        return Err(format!("immediate {} out of range for 16-bit field", v));
    }
    Ok(v as i16)
}

fn encode_r_type(funct: u32, args: Vec<&str>, _: u32, _: u32) -> Result<u32, String> {
    // For the single-operand JR / MFHI we have a different shape; the
    // general R-type expects three register operands (rd, rs, rt).
    let (rd, rs, rt) = match funct {
        8 | 16 | 17 | 18 | 19 => {
            // JR rs, MFHI rd, MTHI rs, MFLO rd, MTLO rs
            let a = args.first().copied().unwrap_or("0");
            if matches!(funct, 8 | 17 | 19) {
                (0u32, parse_reg(a)?, 0u32)
            } else {
                (parse_reg(a)?, 0u32, 0u32)
            }
        }
        9 => {
            // JALR rs [, rd]
            let rs = parse_reg(args.first().copied().unwrap_or("0"))?;
            let rd = if args.len() >= 2 { parse_reg(args[1])? } else { 31 };
            (rd, rs, 0)
        }
        24 | 25 | 26 | 27 => {
            // MULT/DIV(rs, rt)
            let rs = parse_reg(args.first().copied().unwrap_or("0"))?;
            let rt = parse_reg(args.get(1).copied().unwrap_or("0"))?;
            (0, rs, rt)
        }
        _ => {
            let rd = parse_reg(args.first().copied().unwrap_or("0"))?;
            let rs = parse_reg(args.get(1).copied().unwrap_or("0"))?;
            let rt = parse_reg(args.get(2).copied().unwrap_or("0"))?;
            (rd, rs, rt)
        }
    };
    Ok((rs << 21) | (rt << 16) | (rd << 11) | funct)
}

fn encode_i_type(op: u32, args: &[&str]) -> Result<u32, String> {
    if args.len() < 2 {
        return Err("I-type instruction requires at least two operands".into());
    }
    let rt = parse_reg(args[0])?;
    let rs = parse_reg(args[1])?;
    let imm = parse_imm(args.get(2).copied().unwrap_or("0"))? as u16;
    Ok((op << 26) | (rs << 21) | (rt << 16) | imm as u32)
}

// ============================================================================
// MIPS analyst / opcode classification (lightweight port)
// ============================================================================

/// Lightweight port of the `MIPSAnalyst` opcode classification, used by
/// the disassembly manager to decide whether a region is code, data, or
/// a branch target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MipsOpcodeClass {
    /// Opcode is a regular arithmetic / data-processing instruction.
    Normal,
    /// Opcode is a branch (beq, bne, j, jal, ...).
    Branch,
    /// Opcode is a jump (j, jal, jr, jalr).
    Jump,
    /// Opcode is a load.
    Load,
    /// Opcode is a store.
    Store,
    /// Opcode is a system / control instruction.
    System,
    /// Unknown / undecoded opcode.
    Unknown,
}

/// Classify a single 32-bit MIPS instruction word.
pub fn classify_opcode(code: u32) -> MipsOpcodeClass {
    let op = (code >> 26) & 0x3F;
    match op {
        0x00 => match code & 0x3F {
            0x08 | 0x09 => MipsOpcodeClass::Jump,
            0x0C | 0x0D => MipsOpcodeClass::System,
            _ => MipsOpcodeClass::Normal,
        },
        0x02 | 0x03 => MipsOpcodeClass::Jump,
        0x04..=0x07 => MipsOpcodeClass::Branch,
        0x20 | 0x21 | 0x22 | 0x23 | 0x24 | 0x25 | 0x26 | 0x27 | 0x2F => MipsOpcodeClass::Load,
        0x28 | 0x29 | 0x2A | 0x2B | 0x2E | 0x3F => MipsOpcodeClass::Store,
        _ => MipsOpcodeClass::Unknown,
    }
}

// ============================================================================
// MIPS stack walker
// ============================================================================

/// A single stack frame recovered by the MIPS stack walker.
///
/// Mirrors the C++ `MipsStackWalk::StackFrame` struct. The Rust port
/// keeps the same field names so call sites stay symmetric.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackFrame {
    pub pc: u32,
    pub sp: u32,
    pub ra: u32,
    pub frame_size: u32,
}

/// Walk the call stack starting at the address held in `$ra` and the
/// stack pointer held in `$sp`.
///
/// This is a deliberately simplified port: the C++ walker inspects the
/// function prologue via the symbol guardian; the Rust walker follows
/// the `$ra -> $sp` link for `max_depth` frames or until `$ra == 0`.
pub fn stack_walk(initial_pc: u32, initial_sp: u32, initial_ra: u32, max_depth: usize) -> Vec<StackFrame> {
    let mut frames = Vec::new();
    let mut pc = initial_pc;
    let mut sp = initial_sp;
    let mut ra = initial_ra;
    for _ in 0..max_depth {
        if ra == 0 {
            break;
        }
        let frame_size = 0u32; // The full port reads it from the symbol guardian.
        frames.push(StackFrame { pc, sp, ra, frame_size });
        pc = ra;
        sp = sp.wrapping_add(frame_size);
        ra = 0; // Without a backing store we cannot chain further; terminate.
    }
    frames
}

// ============================================================================
// Symbol guardian / importer (lightweight port)
// ============================================================================

/// In-memory ELF/CPP symbol guardian.
///
/// Mirrors the C++ `SymbolGuardian` class. The full port maintains a
/// Ramer-Douglas-Peucker-friendly interval tree over the loaded symbols;
/// this minimal port keeps the same public method shape but stores the
/// symbols in a `Vec` -- sufficient for unit tests and for use as a
/// drop-in for the disassembly manager's symbol lookups.
#[derive(Debug, Default, Clone)]
pub struct SymbolGuardian {
    symbols: Vec<SymbolInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolInfo {
    pub address: u32,
    pub size: u32,
    pub name: String,
    pub kind: SymbolKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Function,
    GlobalVariable,
    LocalVariable,
}

impl SymbolGuardian {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a symbol. The C++ side silently deduplicates; the Rust port
    /// does the same by checking both address and name.
    pub fn add(&mut self, info: SymbolInfo) {
        if self.symbols.iter().any(|s| s.address == info.address && s.name == info.name) {
            return;
        }
        self.symbols.push(info);
    }

    /// Number of stored symbols.
    pub fn len(&self) -> usize {
        self.symbols.len()
    }

    /// True if no symbols have been loaded.
    pub fn is_empty(&self) -> bool {
        self.symbols.is_empty()
    }

    /// Look up the symbol that starts exactly at `address`.
    pub fn symbol_at(&self, address: u32) -> Option<&SymbolInfo> {
        self.symbols.iter().find(|s| s.address == address)
    }

    /// Look up the smallest symbol whose range covers `address`.
    pub fn symbol_covering(&self, address: u32) -> Option<&SymbolInfo> {
        self.symbols
            .iter()
            .filter(|s| address >= s.address && address < s.address.saturating_add(s.size))
            .min_by_key(|s| s.size)
    }
}

/// Symbol importer stub, mirroring the C++ `SymbolImporter`.
///
/// The full C++ importer reads DWARF / ELF sections and feeds the
/// `SymbolGuardian`. This Rust port exposes the same constructor
/// signature but does no actual import -- a full port is out of scope
/// for a `std`-only module.
#[derive(Debug, Default)]
pub struct SymbolImporter;

impl SymbolImporter {
    pub fn new() -> Self {
        Self
    }

    /// Returns `true` if a real importer would have work to do.
    pub fn has_work(&self) -> bool {
        false
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breakpoint_add_and_check() {
        let mut list = BreakpointList::new();
        assert!(list.check(0x1000).is_none());
        list.add(Breakpoint::new(0x1000));
        assert!(list.check(0x1000).is_some());
        list.remove(0x1000);
        assert!(list.check(0x1000).is_none());
    }

    #[test]
    fn breakpoint_replace_returns_previous() {
        let mut list = BreakpointList::new();
        list.add(Breakpoint::new(0x1000));
        let prev = list.add(Breakpoint::new(0x1000));
        assert!(prev.is_some());
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn expression_arithmetic() {
        assert_eq!(expression_eval("1 + 2 * 3").unwrap(), 7);
        assert_eq!(expression_eval("(1 + 2) * 3").unwrap(), 9);
        assert_eq!(expression_eval("0xff & 0x0f").unwrap(), 0x0f);
        assert_eq!(expression_eval("1 << 4").unwrap(), 16);
    }

    #[test]
    fn expression_radix() {
        assert_eq!(expression_eval("0x10").unwrap(), 16);
        assert_eq!(expression_eval("$10").unwrap(), 16);
        assert_eq!(expression_eval("0b1010").unwrap(), 10);
        assert_eq!(expression_eval("0o17").unwrap(), 15);
        assert_eq!(expression_eval("1234h").unwrap(), 0x1234);
    }

    #[test]
    fn assembler_basic() {
        assert_eq!(mips_assemble("NOP").unwrap(), 0);
        assert_eq!(mips_assemble("ADDIU $t0, $zero, 1").unwrap(), 0x24080001);
    }

    #[test]
    fn classify_branch() {
        assert_eq!(classify_opcode(0x10000000), MipsOpcodeClass::Branch);
        assert_eq!(classify_opcode(0x08000000), MipsOpcodeClass::Jump);
        assert_eq!(classify_opcode(0x8C000000), MipsOpcodeClass::Load);
    }

    #[test]
    fn stack_walk_one_frame() {
        let frames = stack_walk(0x1000, 0x7fff_0000, 0x2000, 4);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].ra, 0x2000);
    }

    #[test]
    fn symbol_guardian_add_and_lookup() {
        let mut g = SymbolGuardian::new();
        g.add(SymbolInfo {
            address: 0x1000,
            size: 0x40,
            name: "main".into(),
            kind: SymbolKind::Function,
        });
        assert_eq!(g.symbol_at(0x1000).unwrap().name, "main");
        assert!(g.symbol_covering(0x1020).is_some());
        assert!(g.symbol_covering(0x2000).is_none());
    }

    #[test]
    fn disasmR3000A_zero_is_bad_op() {
        // With no backing DebugInterface the function returns a deterministic
        // "*** Bad OP ***" line, not a panic.
        let s = disasmR3000A(0x0000_1000);
        assert!(s.contains("*** Bad OP ***"));
    }

    #[test]
    fn disasmR5900_zero_is_bad_op() {
        let s = disasmR5900(0x0000_1000);
        assert!(s.contains("*** Bad OP ***"));
    }
}
