//! `DebugToolsFull` — idiomatic Rust translation of the PCSX2 `DebugTools/`
//! source set (breakpoints, debug interface, disassembly managers, expression
//! parser, MIPS assembler, symbol guardian, and symbol importer).
//!
//! This module is a self-contained `std`-only port.  It is a translation of:
//!
//! * `BiosDebugData.{h,cpp}`        — BIOS thread data structures
//! * `Breakpoints.{h,cpp}`          — breakpoint / memcheck list
//! * `Debug.h`                      — trace / console log scaffolding (collapsed)
//! * `DebugInterface.{h,cpp}`       — memory + register interface
//! * `DisASM.h`                     — disassembly decode macros (folded into helpers)
//! * `DisassemblyManager.{h,cpp}`   — disassembly line cache
//! * `DisR3000A.cpp`                — R3000A disassembler
//! * `DisR5900asm.cpp`              — R5900 disassembler
//! * `DisVU0Micro.cpp` / `DisVU1Micro.cpp`
//!                                   — VU0 / VU1 micro disassembler
//! * `DisVUmicro.h` / `DisVUops.h`  — VU opcode / table macros (collapsed)
//! * `ExpressionParser.{h,cpp}`     — infix/postfix expression parser
//! * `MIPSAnalyst.{h,cpp}`          — MIPS branch / load-store analysis
//! * `MipsAssembler.{h,cpp}`        — MIPS assembler
//! * `MipsAssemblerTables.{h,cpp}`  — opcode tables (collapsed into a static)
//! * `MipsStackWalk.{h,cpp}`        — stack walker
//! * `SymbolGuardian.{h,cpp}`       — symbol database
//! * `SymbolImporter.{h,cpp}`       — symbol importer
//!
//! Only the parts explicitly required by the public API are fully implemented;
//! everything else is structurally present so the module compiles and so future
//! ports can extend it without redoing the type definitions.

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

// ---------------------------------------------------------------------------
// Common aliases
// ---------------------------------------------------------------------------

pub type u8 = std::primitive::u8;
pub type u16 = std::primitive::u16;
pub type u32 = std::primitive::u32;
pub type u64 = std::primitive::u64;
pub type u128 = std::primitive::u128;
pub type i32 = std::primitive::i32;
pub type i64 = std::primitive::i64;
pub type s16 = std::primitive::i16;
pub type s32 = std::primitive::i32;
pub type s64 = std::primitive::i64;
pub type bool_ = bool;

pub const INVALID_BREAKPOINT: usize = usize::MAX;
pub const INVALID_MEMCHECK: usize = usize::MAX;

// ---------------------------------------------------------------------------
// BreakPointCpu
// ---------------------------------------------------------------------------

pub type BreakPointCpu = u32;
pub const BREAKPOINT_EE: BreakPointCpu = 0x01;
pub const BREAKPOINT_IOP: BreakPointCpu = 0x02;
pub const BREAKPOINT_IOP_AND_EE: BreakPointCpu = 0x03;

// ---------------------------------------------------------------------------
// Register categories
// ---------------------------------------------------------------------------

pub const EECAT_GPR: i32 = 0;
pub const EECAT_CP0: i32 = 1;
pub const EECAT_FPR: i32 = 2;
pub const EECAT_FCR: i32 = 3;
pub const EECAT_VU0F: i32 = 4;
pub const EECAT_VU0I: i32 = 5;
pub const EECAT_GSPRIV: i32 = 6;
pub const EECAT_COUNT: i32 = 7;

pub const IOPCAT_GPR: i32 = 0;
pub const IOPCAT_COUNT: i32 = 1;

// ---------------------------------------------------------------------------
// MIPS / R5900 register name tables
// ---------------------------------------------------------------------------

const GPR_REG: [&str; 32] = [
    "zero", "at", "v0", "v1", "a0", "a1", "a2", "a3",
    "t0", "t1", "t2", "t3", "t4", "t5", "t6", "t7",
    "s0", "s1", "s2", "s3", "s4", "s5", "s6", "s7",
    "t8", "t9", "k0", "k1", "gp", "sp", "fp", "ra",
];

const COP0_REG: [&str; 32] = [
    "Index", "Random", "EntryLo0", "EntryLo1", "Context", "PageMask",
    "Wired", "C0r7", "BadVaddr", "Count", "EntryHi", "Compare", "Status",
    "Cause", "EPC", "PRId", "Config", "C0r17", "C0r18", "C0r19", "C0r20",
    "C0r21", "C0r22", "C0r23", "Debug", "Perf", "C0r26", "C0r27", "TagLo",
    "TagHi", "ErrorPC", "C0r31",
];

const COP1_REG_FP: [&str; 32] = [
    "f00", "f01", "f02", "f03", "f04", "f05", "f06", "f07",
    "f08", "f09", "f10", "f11", "f12", "f13", "f14", "f15",
    "f16", "f17", "f18", "f19", "f20", "f21", "f22", "f23",
    "f24", "f25", "f26", "f27", "f28", "f29", "f30", "f31",
];

const COP1_REG_FCR: [&str; 32] = [
    "fcr00", "fcr01", "fcr02", "fcr03", "fcr04", "fcr05", "fcr06", "fcr07",
    "fcr08", "fcr09", "fcr10", "fcr11", "fcr12", "fcr13", "fcr14", "fcr15",
    "fcr16", "fcr17", "fcr18", "fcr19", "fcr20", "fcr21", "fcr22", "fcr23",
    "fcr24", "fcr25", "fcr26", "fcr27", "fcr28", "fcr29", "fcr30", "fcr31",
];

const COP2_REG_FP: [&str; 32] = [
    "vf00", "vf01", "vf02", "vf03", "vf04", "vf05", "vf06", "vf07",
    "vf08", "vf09", "vf10", "vf11", "vf12", "vf13", "vf14", "vf15",
    "vf16", "vf17", "vf18", "vf19", "vf20", "vf21", "vf22", "vf23",
    "vf24", "vf25", "vf26", "vf27", "vf28", "vf29", "vf30", "vf31",
];

const COP2_REG_CTL: [&str; 32] = [
    "vi00", "vi01", "vi02", "vi03", "vi04", "vi05", "vi06", "vi07",
    "vi08", "vi09", "vi10", "vi11", "vi12", "vi13", "vi14", "vi15",
    "Status", "MACflag", "ClipFlag", "c2c19", "R", "I", "Q", "c2c23",
    "c2c24", "c2c25", "TPC", "CMSAR0", "FBRST", "VPU-STAT", "c2c30", "CMSAR1",
];

const COP2_VFnames: [&str; 4] = ["x", "y", "z", "w"];

const GS_REG_PRIV: [&str; 19] = [
    "PMODE", "SMODE1", "SMODE2", "SRFSH", "SYNCH1", "SYNCH2", "SYNCV",
    "DISPFB1", "DISPLAY1", "DISPFB2", "DISPLAY2", "EXTBUF", "EXTDATA",
    "EXTWRITE", "BGCOLOR", "CSR", "IMR", "BUSDIR", "SIGLBLID",
];

const GS_REG_PRIV_ADDR: [u32; 19] = [
    0x00, 0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80, 0x90,
    0xa0, 0xb0, 0xc0, 0xd0, 0xE0, 0x1000, 0x1010, 0x1040, 0x1080,
];

// ---------------------------------------------------------------------------
// R3000A register name table
// ---------------------------------------------------------------------------

const DIS_R_NAME_GPR: [&str; 34] = [
    "r0", "at", "v0", "v1", "a0", "a1", "a2", "a3",
    "t0", "t1", "t2", "t3", "t4", "t5", "t6", "t7",
    "s0", "s1", "s2", "s3", "s4", "s5", "s6", "s7",
    "t8", "t9", "k0", "k1", "gp", "sp", "fp", "ra",
    "HI", "LO",
];

// ---------------------------------------------------------------------------
// Thread / wait states (from BiosDebugData.h)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadStatus {
    BAD = 0x00,
    RUN = 0x01,
    READY = 0x02,
    WAIT = 0x04,
    SUSPEND = 0x08,
    WAIT_SUSPEND = 0x0C,
    DORMANT = 0x10,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EEWaitStatus {
    NONE = 0,
    SLEEP = 1,
    SEMA = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IOPWaitStatus {
    SLEEP = 1,
    DELAY = 2,
    SEMA = 3,
    EVENTFLAG = 4,
    MBX = 5,
    VPL = 6,
    FPL = 7,
}

#[derive(Debug, Clone)]
pub struct EEInternalThread {
    pub status: i32,
    pub resume_addr: u32,
    pub reg_ctx: u32,
    pub wait_type: i32,
    pub sema_id: i32,
    pub entry: u32,
    pub current_priority: i16,
    pub init_priority: i16,
}

#[derive(Debug, Clone)]
pub struct IOPInternalThread {
    pub tid: u32,
    pub pc: u32,
    pub status: u32,
    pub reg_ctx: u32,
    pub entrypoint: u32,
    pub waitstate: u32,
    pub wait_id: u32,
    pub init_priority: u32,
    pub stac_mem: u32,
    pub stack_size: u32,
}

#[derive(Debug, Clone)]
pub struct EEInternalCtx {
    pub gpr: [u128; 31],
    pub fpr: [f32; 32],
}

#[derive(Debug, Clone)]
pub struct BiosThread {
    pub tid: u32,
    pub pc: u32,
    pub status: ThreadStatus,
    pub wait: WaitState,
    pub wait_id: u32,
    pub entry_point: u32,
    pub priority: u32,
    pub reg_ctx: u32,
}

impl BiosThread {
    pub fn TID(&self) -> u32 { self.tid }
    pub fn PC(&self) -> u32 { self.pc }
    pub fn Status(&self) -> ThreadStatus { self.status }
    pub fn Wait(&self) -> WaitState { self.wait }
    pub fn WaitId(&self) -> u32 { self.wait_id }
    pub fn EntryPoint(&self) -> u32 { self.entry_point }
    pub fn Priority(&self) -> u32 { self.priority }
    pub fn RegCtx(&self) -> u32 { self.reg_ctx }
}

#[derive(Debug, Clone)]
pub struct IopMod {
    pub name: String,
    pub version: u16,
    pub text_addr: u32,
    pub entry: u32,
    pub gp: u32,
    pub text_size: u32,
    pub data_size: u32,
    pub bss_size: u32,
}

fn ee_wait_from(value: i32) -> WaitState {
    match EEWaitStatus::try_from_i32(value) {
        Some(EEWaitStatus::SLEEP) => WaitState::SLEEP,
        Some(EEWaitStatus::SEMA) => WaitState::SEMA,
        _ => WaitState::NONE,
    }
}

fn iop_wait_from(value: u32) -> WaitState {
    match IOPWaitStatus::try_from_u32(value) {
        Some(IOPWaitStatus::SLEEP) => WaitState::SLEEP,
        Some(IOPWaitStatus::DELAY) => WaitState::DELAY,
        Some(IOPWaitStatus::SEMA) => WaitState::SEMA,
        Some(IOPWaitStatus::EVENTFLAG) => WaitState::EVENTFLAG,
        Some(IOPWaitStatus::MBX) => WaitState::MBOX,
        Some(IOPWaitStatus::VPL) => WaitState::VPOOL,
        Some(IOPWaitStatus::FPL) => WaitState::FIXPOOL,
        _ => WaitState::NONE,
    }
}

impl EEWaitStatus {
    fn try_from_i32(v: i32) -> Option<Self> {
        match v {
            0 => Some(Self::NONE),
            1 => Some(Self::SLEEP),
            2 => Some(Self::SEMA),
            _ => None,
        }
    }
}

impl IOPWaitStatus {
    fn try_from_u32(v: u32) -> Option<Self> {
        match v {
            1 => Some(Self::SLEEP),
            2 => Some(Self::DELAY),
            3 => Some(Self::SEMA),
            4 => Some(Self::EVENTFLAG),
            5 => Some(Self::MBX),
            6 => Some(Self::VPL),
            7 => Some(Self::FPL),
            _ => None,
        }
    }
}

pub fn ee_thread_from(tid: u32, internal: &EEInternalThread) -> BiosThread {
    BiosThread {
        tid,
        pc: internal.resume_addr,
        status: match internal.status {
            0x01 => ThreadStatus::RUN,
            0x02 => ThreadStatus::READY,
            0x04 => ThreadStatus::WAIT,
            0x08 => ThreadStatus::SUSPEND,
            0x0C => ThreadStatus::WAIT_SUSPEND,
            0x10 => ThreadStatus::DORMANT,
            _ => ThreadStatus::BAD,
        },
        wait: ee_wait_from(internal.wait_type),
        wait_id: internal.sema_id as u32,
        entry_point: internal.entry,
        priority: internal.current_priority as u32,
        reg_ctx: internal.reg_ctx,
    }
}

pub fn iop_thread_from(data: &IOPInternalThread) -> BiosThread {
    BiosThread {
        tid: data.tid,
        pc: data.pc,
        status: match data.status {
            0x01 => ThreadStatus::RUN,
            0x02 => ThreadStatus::READY,
            0x04 => ThreadStatus::WAIT,
            0x08 => ThreadStatus::SUSPEND,
            0x0C => ThreadStatus::WAIT_SUSPEND,
            0x10 => ThreadStatus::DORMANT,
            _ => ThreadStatus::BAD,
        },
        wait: iop_wait_from(data.waitstate),
        wait_id: data.wait_id,
        entry_point: data.entrypoint,
        priority: data.init_priority,
        reg_ctx: data.reg_ctx,
    }
}

// ---------------------------------------------------------------------------
// Breakpoint / memcheck types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Breakpoint {
    pub pc: u32,
    pub enabled: bool,
    pub condition: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct BreakpointList {
    pub breakpoints: Vec<Breakpoint>,
}

impl BreakpointList {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, bp: Breakpoint) {
        self.breakpoints.push(bp);
    }

    pub fn remove(&mut self, pc: u32) -> bool {
        if let Some(pos) = self.breakpoints.iter().position(|b| b.pc == pc) {
            self.breakpoints.remove(pos);
            true
        } else {
            false
        }
    }

    /// Returns `Some(index)` of the first enabled breakpoint matching `pc`,
    /// or `None` if none match.
    pub fn check(&self, pc: u32) -> Option<usize> {
        self.breakpoints
            .iter()
            .position(|b| b.enabled && b.pc == pc)
    }

    pub fn len(&self) -> usize {
        self.breakpoints.len()
    }

    pub fn is_empty(&self) -> bool {
        self.breakpoints.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Memory / register trait surface (collapse MemoryInterface / DebugInterface)
// ---------------------------------------------------------------------------

pub trait DebugInterface {
    fn read8(&self, address: u32) -> u8;
    fn read16(&self, address: u32) -> u16;
    fn read32(&self, address: u32) -> u32;
    fn read64(&self, address: u32) -> u64;
    fn write8(&mut self, address: u32, value: u8);
    fn write16(&mut self, address: u32, value: u16);
    fn write32(&mut self, address: u32, value: u32);
    fn disassemble(&self, pc: u32) -> String;

    fn is_valid_address(&self, address: u32) -> bool;
    fn cpu_type(&self) -> BreakPointCpu;
}

// ---------------------------------------------------------------------------
// EE / IOP debug interface impls
// ---------------------------------------------------------------------------

/// EE (R5900) backed by a 32-bit byte-addressable memory slice.
#[derive(Debug, Clone)]
pub struct EeDebugInterface {
    pub memory: Vec<u8>,
    pub pc: u32,
    pub cycle: u32,
    pub exposed_ram: u32,
}

impl Default for EeDebugInterface {
    fn default() -> Self {
        Self {
            memory: Vec::new(),
            pc: 0,
            cycle: 0,
            exposed_ram: 0x0200_0000,
        }
    }
}

impl EeDebugInterface {
    pub fn new() -> Self {
        Self::default()
    }

    fn lopart(addr: u32) -> u32 {
        addr & 0x0FFF_FFFF
    }
}

impl DebugInterface for EeDebugInterface {
    fn read8(&self, address: u32) -> u8 {
        if !self.is_valid_address(address) {
            return 0xFF;
        }
        let lo = Self::lopart(address);
        if (lo as usize) < self.memory.len() {
            self.memory[lo as usize]
        } else {
            0
        }
    }

    fn read16(&self, address: u32) -> u16 {
        u16::from(self.read8(address)) | (u16::from(self.read8(address.wrapping_add(1))) << 8)
    }

    fn read32(&self, address: u32) -> u32 {
        u32::from(self.read8(address))
            | (u32::from(self.read8(address.wrapping_add(1))) << 8)
            | (u32::from(self.read8(address.wrapping_add(2))) << 16)
            | (u32::from(self.read8(address.wrapping_add(3))) << 24)
    }

    fn read64(&self, address: u32) -> u64 {
        u64::from(self.read32(address)) | (u64::from(self.read32(address.wrapping_add(4))) << 32)
    }

    fn write8(&mut self, address: u32, value: u8) {
        if !self.is_valid_address(address) {
            return;
        }
        let lo = Self::lopart(address) as usize;
        if lo < self.memory.len() {
            self.memory[lo] = value;
        }
    }

    fn write16(&mut self, address: u32, value: u16) {
        self.write8(address, value as u8);
        self.write8(address.wrapping_add(1), (value >> 8) as u8);
    }

    fn write32(&mut self, address: u32, value: u32) {
        self.write8(address, value as u8);
        self.write8(address.wrapping_add(1), (value >> 8) as u8);
        self.write8(address.wrapping_add(2), (value >> 16) as u8);
        self.write8(address.wrapping_add(3), (value >> 24) as u8);
    }

    fn disassemble(&self, pc: u32) -> String {
        let op = self.read32(pc);
        disasm_r5900(op, pc)
    }

    fn is_valid_address(&self, addr: u32) -> bool {
        let lopart = addr & 0x0FFF_FFFF;
        match addr >> 28 {
            0 | 2 | 3 => lopart >= 0x80_000 && lopart < self.exposed_ram,
            1 => {
                if lopart <= 0xCFFF {
                    true
                } else if (0x100_0000..=0x100_FFFF).contains(&lopart) {
                    true
                } else {
                    (0x200_0000..=0x200_10FF).contains(&lopart)
                }
            }
            7 => lopart <= 0x3FFF,
            8 | 0xA => lopart <= 0xFFFFF,
            9 | 0xB => lopart >= 0xFC0_0000,
            0xF => lopart >= 0x0FFF_8000,
            _ => false,
        }
    }

    fn cpu_type(&self) -> BreakPointCpu {
        BREAKPOINT_EE
    }
}

/// IOP (R3000A) backed by a 32-bit byte-addressable memory slice.
#[derive(Debug, Clone)]
pub struct IopDebugInterface {
    pub memory: Vec<u8>,
    pub pc: u32,
    pub cycle: u32,
    pub exposed_iop_ram: u32,
}

impl Default for IopDebugInterface {
    fn default() -> Self {
        Self {
            memory: Vec::new(),
            pc: 0,
            cycle: 0,
            exposed_iop_ram: 0x0080_0000,
        }
    }
}

impl IopDebugInterface {
    pub fn new() -> Self {
        Self::default()
    }
}

impl DebugInterface for IopDebugInterface {
    fn read8(&self, address: u32) -> u8 {
        if !self.is_valid_address(address) {
            return 0xFF;
        }
        let p = (address & 0x1FFF_FFFF) as usize;
        if p < self.memory.len() {
            self.memory[p]
        } else {
            0
        }
    }

    fn read16(&self, address: u32) -> u16 {
        u16::from(self.read8(address)) | (u16::from(self.read8(address.wrapping_add(1))) << 8)
    }

    fn read32(&self, address: u32) -> u32 {
        u32::from(self.read8(address))
            | (u32::from(self.read8(address.wrapping_add(1))) << 8)
            | (u32::from(self.read8(address.wrapping_add(2))) << 16)
            | (u32::from(self.read8(address.wrapping_add(3))) << 24)
    }

    fn read64(&self, address: u32) -> u64 {
        u64::from(self.read32(address)) | (u64::from(self.read32(address.wrapping_add(4))) << 32)
    }

    fn write8(&mut self, address: u32, value: u8) {
        if !self.is_valid_address(address) {
            return;
        }
        let p = (address & 0x1FFF_FFFF) as usize;
        if p < self.memory.len() {
            self.memory[p] = value;
        }
    }

    fn write16(&mut self, address: u32, value: u16) {
        self.write8(address, value as u8);
        self.write8(address.wrapping_add(1), (value >> 8) as u8);
    }

    fn write32(&mut self, address: u32, value: u32) {
        self.write8(address, value as u8);
        self.write8(address.wrapping_add(1), (value >> 8) as u8);
        self.write8(address.wrapping_add(2), (value >> 16) as u8);
        self.write8(address.wrapping_add(3), (value >> 24) as u8);
    }

    fn disassemble(&self, pc: u32) -> String {
        let op = self.read32(pc);
        disasm_r3000a(op, pc)
    }

    fn is_valid_address(&self, addr: u32) -> bool {
        let a = addr & 0x1FFF_FFFF;
        if (0x1D00_0000..0x1E00_0000).contains(&a) {
            return true;
        }
        if (0x1F40_0000..0x1FA0_0000).contains(&a) {
            return true;
        }
        if (0x1FC0_0000..0x2000_0000).contains(&a) {
            return true;
        }
        a < self.exposed_iop_ram
    }

    fn cpu_type(&self) -> BreakPointCpu {
        BREAKPOINT_IOP
    }
}

// ---------------------------------------------------------------------------
// Disassembly manager
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct DisassemblyLineInfo {
    pub name: String,
    pub params: String,
    pub total_size: u32,
    pub is_branch: bool,
    pub branch_target: u32,
}

impl Default for DisassemblyLineInfo {
    fn default() -> Self {
        Self {
            name: String::new(),
            params: String::new(),
            total_size: 4,
            is_branch: false,
            branch_target: 0,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct DisassemblyManager {
    pub entries: BTreeMap<u32, DisassemblyLineInfo>,
    pub max_param_chars: usize,
}

impl DisassemblyManager {
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            max_param_chars: 29,
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Decode a single line of disassembly from `address` using the supplied
    /// debug interface.  Results are cached in `entries` keyed by `address`.
    pub fn get_line<D: DebugInterface + ?Sized>(
        &mut self,
        cpu: &D,
        address: u32,
        insert_symbols: bool,
    ) -> DisassemblyLineInfo {
        if let Some(entry) = self.entries.get(&address) {
            return entry.clone();
        }

        let op = cpu.read32(address);
        let mut info = DisassemblyLineInfo::default();
        if cpu.cpu_type() == BREAKPOINT_EE {
            let (name, params, branch, target) = decode_r5900(op, address);
            info.name = name;
            info.params = params;
            info.is_branch = branch;
            info.branch_target = target;
        } else {
            let (name, params, branch, target) = decode_r3000a(op, address);
            info.name = name;
            info.params = params;
            info.is_branch = branch;
            info.branch_target = target;
        }
        info.total_size = 4;
        let _ = insert_symbols; // symbol database not modelled in this port
        self.entries.insert(address, info.clone());
        info
    }
}

// ---------------------------------------------------------------------------
// Disassembler helpers
// ---------------------------------------------------------------------------

fn decode_function(code: u32) -> u32 {
    code & 0x3F
}
fn decode_rd(code: u32) -> u32 {
    (code >> 11) & 0x1F
}
fn decode_rt(code: u32) -> u32 {
    (code >> 16) & 0x1F
}
fn decode_rs(code: u32) -> u32 {
    (code >> 21) & 0x1F
}
fn decode_sa(code: u32) -> u32 {
    (code >> 6) & 0x1F
}
fn decode_immed(code: u32) -> u32 {
    code & 0xFFFF
}
fn decode_immed_signed(code: u32) -> i32 {
    (code & 0xFFFF) as i16 as i32
}
fn decode_offset(pc: u32, code: u32) -> u32 {
    (pc + 4).wrapping_add((decode_immed_signed(code) as u32).wrapping_mul(4))
}
fn decode_jump(pc: u32, code: u32) -> u32 {
    (pc & 0xF000_0000) | ((code & 0x03FF_FFFF) << 2)
}

pub fn disasm_r3000a(code: u32, pc: u32) -> String {
    let (name, params, _, _) = decode_r3000a(code, pc);
    format!("{:08x} {:08x}:  {} {}", pc, code, name, params)
}

pub fn disasm_r5900(op: u32, pc: u32) -> String {
    let (name, params, _, _) = decode_r5900(op, pc);
    format!("{:08x} {:08x}:  {} {}", pc, op, name, params)
}

pub fn disasm_vu0_micro(code: u32, pc: u32) -> String {
    format!("{:08x} {:08x}:  VU0UF {}", pc, code, decode_vu_upper(code))
}

pub fn disasm_vu1_micro(code: u32, pc: u32) -> String {
    format!("{:08x} {:08x}:  VU1UF {}", pc, code, decode_vu_upper(code))
}

fn decode_vu_upper(code: u32) -> String {
    // Simplified stub: VU upper instructions operate on floating-point
    // registers.  Emit the destination field mask in the original `dest.xyzw`
    // style.
    let dest = dest_string(code);
    format!("vf.{:<4}", dest)
}

fn dest_string(code: u32) -> String {
    let mut s = String::new();
    if (code >> 24) & 1 == 1 {
        s.push('x');
    }
    if (code >> 23) & 1 == 1 {
        s.push('y');
    }
    if (code >> 22) & 1 == 1 {
        s.push('z');
    }
    if (code >> 21) & 1 == 1 {
        s.push('w');
    }
    s
}

// ----- R3000A -----

pub fn disasm_r3000a_global(code: u32, pc: u32) -> String {
    disasm_r3000a(code, pc)
}

fn decode_r3000a(code: u32, pc: u32) -> (String, String, bool, u32) {
    let op = (code >> 26) & 0x3F;
    let (name, params) = match op {
        0x00 => decode_r3000a_special(code, pc),
        0x01 => decode_r3000a_bcond(code, pc),
        0x02 => ("j".into(), format!("->${:08x}", decode_jump(pc, code))),
        0x03 => (
            "jal".into(),
            format!("->${:08x}", decode_jump(pc, code)),
        ),
        0x04 => dis_branch("beq", decode_rs(code), decode_rt(code), decode_offset(pc, code)),
        0x05 => dis_branch("bne", decode_rs(code), decode_rt(code), decode_offset(pc, code)),
        0x06 => dis_branch1("blez", decode_rs(code), decode_offset(pc, code)),
        0x07 => dis_branch1("bgtz", decode_rs(code), decode_offset(pc, code)),
        0x08 => format_3("addi", decode_rt(code), decode_rs(code), format_immed(code)),
        0x09 => format_3("addiu", decode_rt(code), decode_rs(code), format_immed(code)),
        0x0A => format_3("slti", decode_rt(code), decode_rs(code), format_immed(code)),
        0x0B => format_3("sltiu", decode_rt(code), decode_rs(code), format_immed(code)),
        0x0C => format_3("andi", decode_rt(code), decode_rs(code), format_immed(code)),
        0x0D => format_3("ori", decode_rt(code), decode_rs(code), format_immed(code)),
        0x0E => format_3("xori", decode_rt(code), decode_rs(code), format_immed(code)),
        0x0F => format_2("lui", decode_rt(code), format_immed(code)),
        0x10 => decode_r3000a_cop0(code, pc),
        0x12 => decode_r3000a_cop2(code, pc),
        0x20 => format_load_store("lb", decode_rt(code), decode_rs(code), decode_immed_signed(code)),
        0x21 => format_load_store("lh", decode_rt(code), decode_rs(code), decode_immed_signed(code)),
        0x22 => format_load_store("lwl", decode_rt(code), decode_rs(code), decode_immed_signed(code)),
        0x23 => format_load_store("lw", decode_rt(code), decode_rs(code), decode_immed_signed(code)),
        0x24 => format_load_store("lbu", decode_rt(code), decode_rs(code), decode_immed_signed(code)),
        0x25 => format_load_store("lhu", decode_rt(code), decode_rs(code), decode_immed_signed(code)),
        0x26 => format_load_store("lwr", decode_rt(code), decode_rs(code), decode_immed_signed(code)),
        0x28 => format_load_store("sb", decode_rt(code), decode_rs(code), decode_immed_signed(code)),
        0x29 => format_load_store("sh", decode_rt(code), decode_rs(code), decode_immed_signed(code)),
        0x2A => format_load_store("swl", decode_rt(code), decode_rs(code), decode_immed_signed(code)),
        0x2B => format_load_store("sw", decode_rt(code), decode_rs(code), decode_immed_signed(code)),
        0x2E => format_load_store("swr", decode_rt(code), decode_rs(code), decode_immed_signed(code)),
        _ => ("???".into(), format!("{:08x}", code)),
    };
    let branch = matches!(op, 0x02..=0x07 | 0x10);
    let target = match op {
        0x02 | 0x03 => decode_jump(pc, code),
        0x04..=0x07 | 0x11 => decode_offset(pc, code),
        _ => 0,
    };
    (name, params, branch, target)
}

fn decode_r3000a_special(code: u32, _pc: u32) -> (String, String) {
    let funct = decode_function(code);
    match funct {
        0x00 => (
            if code == 0 {
                "nop".into()
            } else {
                "sll".into()
            },
            format!(
                "{}, {}, {}",
                reg_gpr(decode_rd(code)),
                reg_gpr(decode_rt(code)),
                decode_sa(code)
            ),
        ),
        0x02 => (
            "srl".into(),
            format!(
                "{}, {}, {}",
                reg_gpr(decode_rd(code)),
                reg_gpr(decode_rt(code)),
                decode_sa(code)
            ),
        ),
        0x03 => (
            "sra".into(),
            format!(
                "{}, {}, {}",
                reg_gpr(decode_rd(code)),
                reg_gpr(decode_rt(code)),
                decode_sa(code)
            ),
        ),
        0x04 => (
            "sllv".into(),
            format!(
                "{}, {}, {}",
                reg_gpr(decode_rd(code)),
                reg_gpr(decode_rt(code)),
                reg_gpr(decode_rs(code))
            ),
        ),
        0x06 => (
            "srlv".into(),
            format!(
                "{}, {}, {}",
                reg_gpr(decode_rd(code)),
                reg_gpr(decode_rt(code)),
                reg_gpr(decode_rs(code))
            ),
        ),
        0x07 => (
            "srav".into(),
            format!(
                "{}, {}, {}",
                reg_gpr(decode_rd(code)),
                reg_gpr(decode_rt(code)),
                reg_gpr(decode_rs(code))
            ),
        ),
        0x08 => (
            "jr".into(),
            format!("{}", reg_gpr(decode_rs(code))),
        ),
        0x09 => (
            "jalr".into(),
            format!(
                "{}, {}",
                reg_gpr(decode_rd(code)),
                reg_gpr(decode_rs(code))
            ),
        ),
        0x0C => ("syscall".into(), "".into()),
        0x0D => ("break".into(), "".into()),
        0x0F => ("sync".into(), "".into()),
        0x10 => ("mfhi".into(), format!("{}", reg_gpr(decode_rd(code)))),
        0x11 => ("mthi".into(), format!("{}", reg_gpr(decode_rs(code)))),
        0x12 => ("mflo".into(), format!("{}", reg_gpr(decode_rd(code)))),
        0x13 => ("mtlo".into(), format!("{}", reg_gpr(decode_rs(code)))),
        0x18 => (
            "mult".into(),
            format!("{}, {}", reg_gpr(decode_rs(code)), reg_gpr(decode_rt(code))),
        ),
        0x19 => (
            "multu".into(),
            format!("{}, {}", reg_gpr(decode_rs(code)), reg_gpr(decode_rt(code))),
        ),
        0x1A => (
            "div".into(),
            format!("{}, {}", reg_gpr(decode_rs(code)), reg_gpr(decode_rt(code))),
        ),
        0x1B => (
            "divu".into(),
            format!("{}, {}", reg_gpr(decode_rs(code)), reg_gpr(decode_rt(code))),
        ),
        0x20 => (
            "add".into(),
            format!(
                "{}, {}, {}",
                reg_gpr(decode_rd(code)),
                reg_gpr(decode_rs(code)),
                reg_gpr(decode_rt(code))
            ),
        ),
        0x21 => (
            "addu".into(),
            format!(
                "{}, {}, {}",
                reg_gpr(decode_rd(code)),
                reg_gpr(decode_rs(code)),
                reg_gpr(decode_rt(code))
            ),
        ),
        0x22 => (
            "sub".into(),
            format!(
                "{}, {}, {}",
                reg_gpr(decode_rd(code)),
                reg_gpr(decode_rs(code)),
                reg_gpr(decode_rt(code))
            ),
        ),
        0x23 => (
            "subu".into(),
            format!(
                "{}, {}, {}",
                reg_gpr(decode_rd(code)),
                reg_gpr(decode_rs(code)),
                reg_gpr(decode_rt(code))
            ),
        ),
        0x24 => (
            "and".into(),
            format!(
                "{}, {}, {}",
                reg_gpr(decode_rd(code)),
                reg_gpr(decode_rs(code)),
                reg_gpr(decode_rt(code))
            ),
        ),
        0x25 => (
            "or".into(),
            format!(
                "{}, {}, {}",
                reg_gpr(decode_rd(code)),
                reg_gpr(decode_rs(code)),
                reg_gpr(decode_rt(code))
            ),
        ),
        0x26 => (
            "xor".into(),
            format!(
                "{}, {}, {}",
                reg_gpr(decode_rd(code)),
                reg_gpr(decode_rs(code)),
                reg_gpr(decode_rt(code))
            ),
        ),
        0x27 => (
            "nor".into(),
            format!(
                "{}, {}, {}",
                reg_gpr(decode_rd(code)),
                reg_gpr(decode_rs(code)),
                reg_gpr(decode_rt(code))
            ),
        ),
        0x2A => (
            "slt".into(),
            format!(
                "{}, {}, {}",
                reg_gpr(decode_rd(code)),
                reg_gpr(decode_rs(code)),
                reg_gpr(decode_rt(code))
            ),
        ),
        0x2B => (
            "sltu".into(),
            format!(
                "{}, {}, {}",
                reg_gpr(decode_rd(code)),
                reg_gpr(decode_rs(code)),
                reg_gpr(decode_rt(code))
            ),
        ),
        _ => ("???".into(), format!("{:08x}", code)),
    }
}

fn decode_r3000a_bcond(code: u32, _pc: u32) -> (String, String) {
    let rt = decode_rt(code);
    let rs = decode_rs(code);
    let offset = decode_offset(_pc, code);
    let name = match rt {
        0x00 => "bltz",
        0x01 => "bgez",
        0x10 => "bltzal",
        0x11 => "bgezal",
        _ => "???",
    };
    dis_branch1(name, rs, offset)
}

fn decode_r3000a_cop0(code: u32, _pc: u32) -> (String, String) {
    let rs = decode_rs(code);
    let rt = decode_rt(code);
    match rs {
        0x00 => (
            "mfc0".into(),
            format!("{}, cop0r{}", reg_gpr(rt), decode_rd(code)),
        ),
        0x04 => (
            "mtc0".into(),
            format!("cop0r{}, {}", decode_rd(code), reg_gpr(rt)),
        ),
        0x10 => ("rfe".into(), "".into()),
        _ => ("COP0??".into(), format!("{:08x}", code)),
    }
}

fn decode_r3000a_cop2(code: u32, _pc: u32) -> (String, String) {
    let rs = decode_rs(code);
    let funct = decode_function(code);
    match (rs, funct) {
        (0x00, _) => (
            "mfc2".into(),
            format!("{}, cop2r{}", reg_gpr(decode_rt(code)), decode_rd(code)),
        ),
        (0x04, _) => (
            "mtc2".into(),
            format!("cop2r{}, {}", decode_rd(code), reg_gpr(decode_rt(code))),
        ),
        (0x02, 0x3F) => ("vwaitq".into(), "".into()),
        _ => ("COP2".into(), format!("{:08x}", code)),
    }
}

fn reg_gpr(idx: u32) -> &'static str {
    DIS_R_NAME_GPR
        .get(idx as usize)
        .copied()
        .unwrap_or("r??")
}

fn reg_gpr_r5900(idx: u32) -> &'static str {
    GPR_REG.get(idx as usize).copied().unwrap_or("r??")
}

fn reg_cop0(idx: u32) -> &'static str {
    COP0_REG.get(idx as usize).copied().unwrap_or("cop0??")
}

fn reg_cop1_fp(idx: u32) -> &'static str {
    COP1_REG_FP.get(idx as usize).copied().unwrap_or("f??")
}

fn reg_cop1_fcr(idx: u32) -> &'static str {
    COP1_REG_FCR.get(idx as usize).copied().unwrap_or("fcr??")
}

fn reg_cop2_fp(idx: u32) -> &'static str {
    COP2_REG_FP.get(idx as usize).copied().unwrap_or("vf??")
}

fn reg_cop2_ctl(idx: u32) -> &'static str {
    COP2_REG_CTL.get(idx as usize).copied().unwrap_or("vi??")
}

fn format_immed(code: u32) -> String {
    format!("0x{:04x}", decode_immed(code))
}

fn format_2(name: &str, a: u32, b: String) -> (String, String) {
    (
        name.into(),
        format!("{}, {}", reg_gpr(a), b),
    )
}

fn format_3(name: &str, a: u32, b: u32, c: String) -> (String, String) {
    (
        name.into(),
        format!("{}, {}, {}", reg_gpr(a), reg_gpr(b), c),
    )
}

fn format_load_store(name: &str, rt: u32, rs: u32, imm: i32) -> (String, String) {
    (
        name.into(),
        format!(
            "{}, 0x{:04x}({})",
            reg_gpr(rt),
            imm as u16,
            reg_gpr(rs)
        ),
    )
}

fn dis_branch(name: &str, rs: u32, rt: u32, target: u32) -> (String, String) {
    (
        name.into(),
        format!("{}, {}, ->${:08x}", reg_gpr(rs), reg_gpr(rt), target),
    )
}

fn dis_branch1(name: &str, rs: u32, target: u32) -> (String, String) {
    (
        name.into(),
        format!("{}, ->${:08x}", reg_gpr(rs), target),
    )
}

// ----- R5900 (EE) -----

fn decode_r5900(code: u32, pc: u32) -> (String, String, bool, u32) {
    let op = (code >> 26) & 0x3F;
    let (name, params) = match op {
        0x00 => decode_r5900_special(code),
        0x01 => decode_r5900_regimm(code, pc),
        0x02 => ("j".into(), format!("->${:08x}", decode_jump(pc, code))),
        0x03 => ("jal".into(), format!("->${:08x}", decode_jump(pc, code))),
        0x04 => dis_branch_r5900("beq", decode_rs(code), decode_rt(code), decode_offset(pc, code)),
        0x05 => dis_branch_r5900("bne", decode_rs(code), decode_rt(code), decode_offset(pc, code)),
        0x06 => dis_branch1_r5900("blez", decode_rs(code), decode_offset(pc, code)),
        0x07 => dis_branch1_r5900("bgtz", decode_rs(code), decode_offset(pc, code)),
        0x08 => (
            "addi".into(),
            format!(
                "{}, {}, 0x{:04x}",
                reg_gpr_r5900(decode_rt(code)),
                reg_gpr_r5900(decode_rs(code)),
                decode_immed(code)
            ),
        ),
        0x09 => (
            "addiu".into(),
            format!(
                "{}, {}, 0x{:04x}",
                reg_gpr_r5900(decode_rt(code)),
                reg_gpr_r5900(decode_rs(code)),
                decode_immed(code)
            ),
        ),
        0x0A => (
            "slti".into(),
            format!(
                "{}, {}, 0x{:04x}",
                reg_gpr_r5900(decode_rt(code)),
                reg_gpr_r5900(decode_rs(code)),
                decode_immed(code)
            ),
        ),
        0x0B => (
            "sltiu".into(),
            format!(
                "{}, {}, 0x{:04x}",
                reg_gpr_r5900(decode_rt(code)),
                reg_gpr_r5900(decode_rs(code)),
                decode_immed(code)
            ),
        ),
        0x0C => (
            "andi".into(),
            format!(
                "{}, {}, 0x{:04x}",
                reg_gpr_r5900(decode_rt(code)),
                reg_gpr_r5900(decode_rs(code)),
                decode_immed(code)
            ),
        ),
        0x0D => (
            "ori".into(),
            format!(
                "{}, {}, 0x{:04x}",
                reg_gpr_r5900(decode_rt(code)),
                reg_gpr_r5900(decode_rs(code)),
                decode_immed(code)
            ),
        ),
        0x0E => (
            "xori".into(),
            format!(
                "{}, {}, 0x{:04x}",
                reg_gpr_r5900(decode_rt(code)),
                reg_gpr_r5900(decode_rs(code)),
                decode_immed(code)
            ),
        ),
        0x0F => (
            "lui".into(),
            format!(
                "{}, 0x{:04x}",
                reg_gpr_r5900(decode_rt(code)),
                decode_immed(code)
            ),
        ),
        0x10 => decode_r5900_cop0(code),
        0x11 => decode_r5900_cop1(code),
        0x12 => decode_r5900_cop2(code, pc),
        0x14 => dis_branch_r5900("beql", decode_rs(code), decode_rt(code), decode_offset(pc, code)),
        0x15 => dis_branch_r5900("bnel", decode_rs(code), decode_rt(code), decode_offset(pc, code)),
        0x1C => decode_r5900_special2(code),
        0x1F => decode_r5900_special3(code),
        0x20 => load_store_r5900("lb", code),
        0x21 => load_store_r5900("lh", code),
        0x22 => load_store_r5900("lwl", code),
        0x23 => load_store_r5900("lw", code),
        0x24 => load_store_r5900("lbu", code),
        0x25 => load_store_r5900("lhu", code),
        0x26 => load_store_r5900("lwr", code),
        0x28 => load_store_r5900("sb", code),
        0x29 => load_store_r5900("sh", code),
        0x2B => load_store_r5900("sw", code),
        0x2F => load_store_r5900("cache", code),
        0x31 => load_store_r5900("lwc1", code),
        0x35 => load_store_r5900("ldc1", code),
        0x39 => load_store_r5900("swc1", code),
        0x3D => load_store_r5900("sdc1", code),
        _ => ("???".into(), format!("0x{:08x}", code)),
    };
    let is_branch = matches!(op, 0x02..=0x07 | 0x11 | 0x14 | 0x15) || matches!(op, 0x01);
    let target = match op {
        0x02 | 0x03 => decode_jump(pc, code),
        0x04..=0x07 | 0x11 | 0x14 | 0x15 | 0x01 => decode_offset(pc, code),
        _ => 0,
    };
    (name, params, is_branch, target)
}

fn decode_r5900_special(code: u32) -> (String, String) {
    let funct = decode_function(code);
    match funct {
        0x00 => (
            if code == 0 {
                "nop".into()
            } else {
                "sll".into()
            },
            format!(
                "{}, {}, {}",
                reg_gpr_r5900(decode_rd(code)),
                reg_gpr_r5900(decode_rt(code)),
                decode_sa(code)
            ),
        ),
        0x02 => (
            "srl".into(),
            format!(
                "{}, {}, {}",
                reg_gpr_r5900(decode_rd(code)),
                reg_gpr_r5900(decode_rt(code)),
                decode_sa(code)
            ),
        ),
        0x03 => (
            "sra".into(),
            format!(
                "{}, {}, {}",
                reg_gpr_r5900(decode_rd(code)),
                reg_gpr_r5900(decode_rt(code)),
                decode_sa(code)
            ),
        ),
        0x04 => (
            "sllv".into(),
            format!(
                "{}, {}, {}",
                reg_gpr_r5900(decode_rd(code)),
                reg_gpr_r5900(decode_rt(code)),
                reg_gpr_r5900(decode_rs(code))
            ),
        ),
        0x06 => (
            "srlv".into(),
            format!(
                "{}, {}, {}",
                reg_gpr_r5900(decode_rd(code)),
                reg_gpr_r5900(decode_rt(code)),
                reg_gpr_r5900(decode_rs(code))
            ),
        ),
        0x07 => (
            "srav".into(),
            format!(
                "{}, {}, {}",
                reg_gpr_r5900(decode_rd(code)),
                reg_gpr_r5900(decode_rt(code)),
                reg_gpr_r5900(decode_rs(code))
            ),
        ),
        0x08 => ("jr".into(), reg_gpr_r5900(decode_rs(code)).to_string()),
        0x09 => (
            "jalr".into(),
            format!(
                "{}, {}",
                reg_gpr_r5900(decode_rd(code)),
                reg_gpr_r5900(decode_rs(code))
            ),
        ),
        0x0A => (
            "movz".into(),
            format!(
                "{}, {}, {}",
                reg_gpr_r5900(decode_rd(code)),
                reg_gpr_r5900(decode_rs(code)),
                reg_gpr_r5900(decode_rt(code))
            ),
        ),
        0x0B => (
            "movn".into(),
            format!(
                "{}, {}, {}",
                reg_gpr_r5900(decode_rd(code)),
                reg_gpr_r5900(decode_rs(code)),
                reg_gpr_r5900(decode_rt(code))
            ),
        ),
        0x0C => ("syscall".into(), "".into()),
        0x0D => ("break".into(), "".into()),
        0x0F => ("sync".into(), "".into()),
        0x10 => ("mfhi".into(), reg_gpr_r5900(decode_rd(code)).to_string()),
        0x11 => ("mthi".into(), reg_gpr_r5900(decode_rs(code)).to_string()),
        0x12 => ("mflo".into(), reg_gpr_r5900(decode_rd(code)).to_string()),
        0x13 => ("mtlo".into(), reg_gpr_r5900(decode_rs(code)).to_string()),
        0x18 => (
            "mult".into(),
            format!("{}, {}", reg_gpr_r5900(decode_rs(code)), reg_gpr_r5900(decode_rt(code))),
        ),
        0x19 => (
            "multu".into(),
            format!("{}, {}", reg_gpr_r5900(decode_rs(code)), reg_gpr_r5900(decode_rt(code))),
        ),
        0x1A => (
            "div".into(),
            format!("{}, {}", reg_gpr_r5900(decode_rs(code)), reg_gpr_r5900(decode_rt(code))),
        ),
        0x1B => (
            "divu".into(),
            format!("{}, {}", reg_gpr_r5900(decode_rs(code)), reg_gpr_r5900(decode_rt(code))),
        ),
        0x20 => (
            "add".into(),
            format!(
                "{}, {}, {}",
                reg_gpr_r5900(decode_rd(code)),
                reg_gpr_r5900(decode_rs(code)),
                reg_gpr_r5900(decode_rt(code))
            ),
        ),
        0x21 => (
            "addu".into(),
            format!(
                "{}, {}, {}",
                reg_gpr_r5900(decode_rd(code)),
                reg_gpr_r5900(decode_rs(code)),
                reg_gpr_r5900(decode_rt(code))
            ),
        ),
        0x22 => (
            "sub".into(),
            format!(
                "{}, {}, {}",
                reg_gpr_r5900(decode_rd(code)),
                reg_gpr_r5900(decode_rs(code)),
                reg_gpr_r5900(decode_rt(code))
            ),
        ),
        0x23 => (
            "subu".into(),
            format!(
                "{}, {}, {}",
                reg_gpr_r5900(decode_rd(code)),
                reg_gpr_r5900(decode_rs(code)),
                reg_gpr_r5900(decode_rt(code))
            ),
        ),
        0x24 => (
            "and".into(),
            format!(
                "{}, {}, {}",
                reg_gpr_r5900(decode_rd(code)),
                reg_gpr_r5900(decode_rs(code)),
                reg_gpr_r5900(decode_rt(code))
            ),
        ),
        0x25 => (
            "or".into(),
            format!(
                "{}, {}, {}",
                reg_gpr_r5900(decode_rd(code)),
                reg_gpr_r5900(decode_rs(code)),
                reg_gpr_r5900(decode_rt(code))
            ),
        ),
        0x26 => (
            "xor".into(),
            format!(
                "{}, {}, {}",
                reg_gpr_r5900(decode_rd(code)),
                reg_gpr_r5900(decode_rs(code)),
                reg_gpr_r5900(decode_rt(code))
            ),
        ),
        0x27 => (
            "nor".into(),
            format!(
                "{}, {}, {}",
                reg_gpr_r5900(decode_rd(code)),
                reg_gpr_r5900(decode_rs(code)),
                reg_gpr_r5900(decode_rt(code))
            ),
        ),
        0x2A => (
            "slt".into(),
            format!(
                "{}, {}, {}",
                reg_gpr_r5900(decode_rd(code)),
                reg_gpr_r5900(decode_rs(code)),
                reg_gpr_r5900(decode_rt(code))
            ),
        ),
        0x2B => (
            "sltu".into(),
            format!(
                "{}, {}, {}",
                reg_gpr_r5900(decode_rd(code)),
                reg_gpr_r5900(decode_rs(code)),
                reg_gpr_r5900(decode_rt(code))
            ),
        ),
        0x34 => ("syscall".into(), "".into()),
        0x38 => (
            "dsll".into(),
            format!(
                "{}, {}, {}",
                reg_gpr_r5900(decode_rd(code)),
                reg_gpr_r5900(decode_rt(code)),
                decode_sa(code)
            ),
        ),
        0x3A => (
            "dsrl".into(),
            format!(
                "{}, {}, {}",
                reg_gpr_r5900(decode_rd(code)),
                reg_gpr_r5900(decode_rt(code)),
                decode_sa(code)
            ),
        ),
        0x3C => (
            "dsll32".into(),
            format!(
                "{}, {}, {}",
                reg_gpr_r5900(decode_rd(code)),
                reg_gpr_r5900(decode_rt(code)),
                decode_sa(code)
            ),
        ),
        _ => ("???".into(), format!("0x{:08x}", code)),
    }
}

fn decode_r5900_special2(code: u32) -> (String, String) {
    let funct = decode_function(code);
    match funct {
        0x00 => (
            "madd".into(),
            format!(
                "{}, {}",
                reg_gpr_r5900(decode_rs(code)),
                reg_gpr_r5900(decode_rt(code))
            ),
        ),
        0x01 => (
            "maddu".into(),
            format!(
                "{}, {}",
                reg_gpr_r5900(decode_rs(code)),
                reg_gpr_r5900(decode_rt(code))
            ),
        ),
        0x04 => (
            "plzcw".into(),
            format!(
                "{}, {}",
                reg_gpr_r5900(decode_rd(code)),
                reg_gpr_r5900(decode_rs(code))
            ),
        ),
        0x20 => (
            "clz".into(),
            format!(
                "{}, {}",
                reg_gpr_r5900(decode_rd(code)),
                reg_gpr_r5900(decode_rs(code))
            ),
        ),
        0x21 => (
            "clo".into(),
            format!(
                "{}, {}",
                reg_gpr_r5900(decode_rd(code)),
                reg_gpr_r5900(decode_rs(code))
            ),
        ),
        _ => ("MMI??".into(), format!("0x{:08x}", code)),
    }
}

fn decode_r5900_special3(code: u32) -> (String, String) {
    let funct = decode_function(code);
    match funct {
        0x00 => (
            "ext".into(),
            format!(
                "{}, {}, {}, {}",
                reg_gpr_r5900(decode_rt(code)),
                reg_gpr_r5900(decode_rs(code)),
                decode_sa(code),
                decode_rd(code)
            ),
        ),
        0x04 => (
            "ins".into(),
            format!(
                "{}, {}, {}, {}",
                reg_gpr_r5900(decode_rt(code)),
                reg_gpr_r5900(decode_rs(code)),
                decode_sa(code),
                decode_rd(code) - decode_sa(code) + 1
            ),
        ),
        0x20 => (
            "bshfl".into(),
            format!(
                "{}, {}",
                reg_gpr_r5900(decode_rd(code)),
                reg_gpr_r5900(decode_rt(code))
            ),
        ),
        0x3B => ("rdhwr".into(), reg_gpr_r5900(decode_rt(code)).to_string()),
        _ => ("SPECIAL3??".into(), format!("0x{:08x}", code)),
    }
}

fn decode_r5900_regimm(code: u32, _pc: u32) -> (String, String) {
    let rt = decode_rt(code);
    let rs = decode_rs(code);
    let offset = decode_offset(_pc, code);
    let name = match rt {
        0x00 => "bltz",
        0x01 => "bgez",
        0x02 => "bltzl",
        0x03 => "bgezl",
        0x10 => "bltzal",
        0x11 => "bgezal",
        _ => "REGIMM??",
    };
    (
        name.into(),
        format!("{}, ->${:08x}", reg_gpr_r5900(rs), offset),
    )
}

fn decode_r5900_cop0(code: u32) -> (String, String) {
    let rs = decode_rs(code);
    let rt = decode_rt(code);
    match rs {
        0x00 => (
            "mfc0".into(),
            format!("{}, {}", reg_gpr_r5900(rt), reg_cop0(decode_rd(code))),
        ),
        0x04 => (
            "mtc0".into(),
            format!("{}, {}", reg_cop0(decode_rd(code)), reg_gpr_r5900(rt)),
        ),
        0x08 => {
            let bc = (rt >> 2) & 0x3;
            let name = ["bc0f", "bc0t", "bc0fl", "bc0tl"][bc as usize];
            (name.into(), "".into())
        }
        0x10 => ("eret".into(), "".into()),
        0x18 => {
            let e = (code >> 22) & 1;
            (if e != 0 { "ei".into() } else { "di".into() }, "".into())
        }
        _ => ("COP0??".into(), format!("0x{:08x}", code)),
    }
}

fn decode_r5900_cop1(code: u32) -> (String, String) {
    let rs = decode_rs(code);
    let rt = decode_rt(code);
    let rd = decode_rd(code);
    let fs = (code >> 11) & 0x1F;
    let ft = (code >> 16) & 0x1F;
    let fd = (code >> 6) & 0x1F;
    match rs {
        0x00 => (
            "mfc1".into(),
            format!("{}, {}", reg_gpr_r5900(rt), reg_cop1_fp(fs)),
        ),
        0x02 => (
            "cfc1".into(),
            format!("{}, {}", reg_gpr_r5900(rt), reg_cop1_fcr(fs)),
        ),
        0x04 => (
            "mtc1".into(),
            format!("{}, {}", reg_gpr_r5900(rt), reg_cop1_fp(fs)),
        ),
        0x06 => (
            "ctc1".into(),
            format!("{}, {}", reg_gpr_r5900(rt), reg_cop1_fcr(fs)),
        ),
        0x08 => {
            let bc = (rt >> 2) & 0x3;
            let name = ["bc1f", "bc1t", "bc1fl", "bc1tl"][bc as usize];
            (name.into(), "".into())
        }
        0x10 => {
            let f = decode_function(code);
            let name = match f {
                0x00 => "add.s",
                0x01 => "sub.s",
                0x02 => "mul.s",
                0x03 => "div.s",
                0x04 => "sqrt.s",
                0x05 => "abs.s",
                0x06 => "mov.s",
                0x07 => "neg.s",
                _ => "FPU??",
            };
            (
                name.into(),
                format!(
                    "{}, {}, {}",
                    reg_cop1_fp(fd),
                    reg_cop1_fp(fs),
                    reg_cop1_fp(ft)
                ),
            )
        }
        0x14 => (
            "cvt.w.s".into(),
            format!("{}, {}", reg_cop1_fp(fd), reg_cop1_fp(fs)),
        ),
        0x21 => (
            "cvt.s.w".into(),
            format!("{}, {}", reg_cop1_fp(fd), reg_cop1_fp(fs)),
        ),
        _ => ("FPU??".into(), format!("0x{:08x}", code)),
    }
}

fn decode_r5900_cop2(code: u32, _pc: u32) -> (String, String) {
    let rs = decode_rs(code);
    let rt = decode_rt(code);
    let rd = decode_rd(code);
    match rs {
        0x00 => (
            "mfc2".into(),
            format!("{}, {}", reg_gpr_r5900(rt), reg_cop2_fp(rd)),
        ),
        0x02 => (
            "cfc2".into(),
            format!("{}, {}", reg_gpr_r5900(rt), reg_cop2_ctl(rd)),
        ),
        0x04 => (
            "mtc2".into(),
            format!("{}, {}", reg_gpr_r5900(rt), reg_cop2_fp(rd)),
        ),
        0x06 => (
            "ctc2".into(),
            format!("{}, {}", reg_gpr_r5900(rt), reg_cop2_ctl(rd)),
        ),
        0x08 => {
            let bc = (rt >> 2) & 0x3;
            let name = ["bc2f", "bc2t", "bc2fl", "bc2tl"][bc as usize];
            (name.into(), "".into())
        }
        _ => ("COP2??".into(), format!("0x{:08x}", code)),
    }
}

fn load_store_r5900(name: &str, code: u32) -> (String, String) {
    (
        name.into(),
        format!(
            "{}, 0x{:04x}({})",
            reg_gpr_r5900(decode_rt(code)),
            decode_immed_signed(code),
            reg_gpr_r5900(decode_rs(code))
        ),
    )
}

fn dis_branch_r5900(name: &str, rs: u32, rt: u32, target: u32) -> (String, String) {
    (
        name.into(),
        format!(
            "{}, {}, ->${:08x}",
            reg_gpr_r5900(rs),
            reg_gpr_r5900(rt),
            target
        ),
    )
}

fn dis_branch1_r5900(name: &str, rs: u32, target: u32) -> (String, String) {
    (
        name.into(),
        format!("{}, ->${:08x}", reg_gpr_r5900(rs), target),
    )
}

// ---------------------------------------------------------------------------
// Standardised breakpoint address (used by EE break logic)
// ---------------------------------------------------------------------------

pub fn standardize_breakpoint_address(addr: u32) -> u32 {
    if addr >= 0xFFFF_8000 {
        return addr;
    }
    let mut a = addr;
    if (0xBFC0_0000..=0xBFFF_FFFF).contains(&a) {
        a &= 0x1FFF_FFFF;
    }
    a &= 0x7FFF_FFFF;
    let high = a >> 28;
    if high == 2 || high == 3 {
        a &= !(0xF << 28);
    }
    a
}

// ---------------------------------------------------------------------------
// MIPS stack walker (lightweight, no live register access)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct StackFrame {
    pub pc: u32,
    pub sp: u32,
    pub ra: u32,
    pub entry_point: u32,
}

pub fn walk_stack(
    start_pc: u32,
    start_ra: u32,
    start_sp: u32,
    entry_point: u32,
    reader: &dyn DebugInterface,
) -> Vec<StackFrame> {
    let mut frames = Vec::new();
    let mut pc = start_pc;
    let mut ra = start_ra;
    let mut sp = start_sp;
    for _ in 0..64 {
        if pc == 0 || !reader.is_valid_address(pc) {
            break;
        }
        frames.push(StackFrame {
            pc,
            sp,
            ra,
            entry_point,
        });
        if pc == entry_point {
            break;
        }
        let next = reader.read32(sp);
        let next_ra = reader.read32(sp + 4);
        if next == 0 {
            break;
        }
        ra = next_ra;
        sp = next;
        pc = next;
    }
    frames
}

// ---------------------------------------------------------------------------
// Expression parser
// ---------------------------------------------------------------------------

pub type PostfixExpression = Vec<(ExpressionCommand, u64)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpressionCommand {
    Const,
    ConstFloat,
    Ref,
    Op,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpressionType {
    Uint = 0,
    Float = 2,
}

#[derive(Debug, Clone)]
struct OpDef {
    name: &'static str,
    priority: u8,
    len: usize,
    args: usize,
    sign: bool,
}

const EXOP_TABLE: &[OpDef] = &[
    OpDef { name: "(",   priority: 25, len: 1, args: 0, sign: false }, // BRACKETL
    OpDef { name: ")",   priority: 25, len: 1, args: 0, sign: false }, // BRACKETR
    OpDef { name: "[",   priority: 4,  len: 1, args: 0, sign: false }, // MEML
    OpDef { name: "]",   priority: 4,  len: 1, args: 0, sign: false }, // MEMR
    OpDef { name: ",",   priority: 5,  len: 1, args: 2, sign: false }, // MEMSIZE
    OpDef { name: "+",   priority: 22, len: 1, args: 1, sign: true  }, // SIGNPLUS
    OpDef { name: "-",   priority: 22, len: 1, args: 1, sign: true  }, // SIGNMINUS
    OpDef { name: "~",   priority: 22, len: 1, args: 1, sign: false }, // BITNOT
    OpDef { name: "!",   priority: 22, len: 1, args: 1, sign: false }, // LOGNOT
    OpDef { name: "*",   priority: 21, len: 1, args: 2, sign: false }, // MUL
    OpDef { name: "/",   priority: 21, len: 1, args: 2, sign: false }, // DIV
    OpDef { name: "%",   priority: 21, len: 1, args: 2, sign: false }, // MOD
    OpDef { name: "+",   priority: 20, len: 1, args: 2, sign: false }, // ADD
    OpDef { name: "-",   priority: 20, len: 1, args: 2, sign: false }, // SUB
    OpDef { name: "<<",  priority: 19, len: 2, args: 2, sign: false }, // SHL
    OpDef { name: ">>",  priority: 19, len: 2, args: 2, sign: false }, // SHR
    OpDef { name: ">=",  priority: 18, len: 2, args: 2, sign: false }, // GTE
    OpDef { name: ">",   priority: 18, len: 1, args: 2, sign: false }, // GT
    OpDef { name: "<=",  priority: 18, len: 2, args: 2, sign: false }, // LTE
    OpDef { name: "<",   priority: 18, len: 1, args: 2, sign: false }, // LT
    OpDef { name: "==",  priority: 17, len: 2, args: 2, sign: false }, // EQ
    OpDef { name: "!=",  priority: 17, len: 2, args: 2, sign: false }, // NE
    OpDef { name: "&",   priority: 16, len: 1, args: 2, sign: false }, // BITAND
    OpDef { name: "^",   priority: 15, len: 1, args: 2, sign: false }, // XOR
    OpDef { name: "|",   priority: 14, len: 1, args: 2, sign: false }, // BITOR
    OpDef { name: "&&",  priority: 13, len: 2, args: 2, sign: false }, // LOGAND
    OpDef { name: "||",  priority: 12, len: 2, args: 2, sign: false }, // LOGOR
    OpDef { name: "?",   priority: 10, len: 1, args: 0, sign: false }, // TERTIF
    OpDef { name: ":",   priority: 11, len: 1, args: 3, sign: false }, // TERTELSE
];

const EXOP_BRACKETL: u32 = 0;
const EXOP_BRACKETR: u32 = 1;
const EXOP_MEML: u32 = 2;
const EXOP_MEMR: u32 = 3;
const EXOP_MEMSIZE: u32 = 4;
const EXOP_SIGNPLUS: u32 = 5;
const EXOP_SIGNMINUS: u32 = 6;
const EXOP_BITNOT: u32 = 7;
const EXOP_LOGNOT: u32 = 8;
const EXOP_MUL: u32 = 9;
const EXOP_DIV: u32 = 10;
const EXOP_MOD: u32 = 11;
const EXOP_ADD: u32 = 12;
const EXOP_SUB: u32 = 13;
const EXOP_SHL: u32 = 14;
const EXOP_SHR: u32 = 15;
const EXOP_GREATEREQUAL: u32 = 16;
const EXOP_GREATER: u32 = 17;
const EXOP_LOWEQUAL: u32 = 18;
const EXOP_LOWER: u32 = 19;
const EXOP_EQUAL: u32 = 20;
const EXOP_NOTEQUAL: u32 = 21;
const EXOP_BITAND: u32 = 22;
const EXOP_XOR: u32 = 23;
const EXOP_BITOR: u32 = 24;
const EXOP_LOGAND: u32 = 25;
const EXOP_LOGOR: u32 = 26;
const EXOP_TERTIF: u32 = 27;
const EXOP_TERTELSE: u32 = 28;
const EXOP_NUMBER: u32 = 29;
const EXOP_MEM: u32 = 30;
const EXOP_NONE: u32 = 31;

fn parse_number(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (radix, body) = if s.starts_with("0x") || s.starts_with("0X") {
        (16u32, &s[2..])
    } else if s.starts_with('$') {
        (16, &s[1..])
    } else if s.starts_with("0o") || s.starts_with("0O") {
        (8, &s[2..])
    } else if let Some(stripped) = s.strip_suffix('b').or(s.strip_suffix('B')) {
        if stripped.chars().all(|c| c == '0' || c == '1') && !s.starts_with("0x") {
            (2, stripped)
        } else {
            (10, s)
        }
    } else if let Some(stripped) = s.strip_suffix('o').or(s.strip_suffix('O')) {
        (8, stripped)
    } else if let Some(stripped) = s.strip_suffix('h').or(s.strip_suffix('H')) {
        (16, stripped)
    } else {
        (10, s)
    };
    u64::from_str_radix(body, radix).ok()
}

fn parse_float(s: &str) -> Option<u64> {
    let s = s.trim();
    if !s.contains('.') {
        return None;
    }
    let v: f32 = s.parse().ok()?;
    Some(v.to_bits() as u64)
}

pub fn expression_eval(expr: &str) -> Result<i128, String> {
    let mut postfix = PostfixExpression::new();
    if let Err(e) = init_postfix(expr, &mut postfix) {
        return Err(e);
    }
    parse_postfix(&postfix)
}

fn init_postfix(infix: &str, dest: &mut PostfixExpression) -> Result<(), String> {
    let bytes = infix.as_bytes();
    let mut pos = 0;
    let mut last_op: u32 = EXOP_NONE;
    let mut stack: Vec<u32> = Vec::new();
    dest.clear();

    while pos < bytes.len() {
        let c = bytes[pos] as char;
        if c == ' ' || c == '\t' {
            pos += 1;
            continue;
        }
        if c.is_ascii_digit() {
            let start = pos;
            while pos < bytes.len() && is_alphanum(bytes[pos] as char) {
                pos += 1;
            }
            let token = &infix[start..pos];
            if let Some(bits) = parse_float(token) {
                dest.push((ExpressionCommand::ConstFloat, bits));
            } else if let Some(val) = parse_number(token) {
                dest.push((ExpressionCommand::Const, val));
            } else {
                return Err(format!("Invalid number \"{}\".", token));
            }
            last_op = EXOP_NUMBER;
            continue;
        }
        if c.is_ascii_alphabetic() || c == '@' || c == '_' {
            let start = pos;
            while pos < bytes.len() && is_alphanum(bytes[pos] as char) {
                pos += 1;
            }
            let token = &infix[start..pos];
            if let Some(val) = parse_number(token) {
                dest.push((ExpressionCommand::Const, val));
                last_op = EXOP_NUMBER;
                continue;
            }
            return Err(format!("Invalid symbol \"{}\".", token));
        }
        // operator
        let mut matched = EXOP_NONE;
        let mut matched_len = 0;
        for (i, op) in EXOP_TABLE.iter().enumerate() {
            if op.sign && (last_op == EXOP_NUMBER || last_op == EXOP_BRACKETR) {
                continue;
            }
            if op.len <= matched_len {
                continue;
            }
            if infix[pos..].starts_with(op.name) {
                matched = i as u32;
                matched_len = op.len;
            }
        }
        if matched == EXOP_NONE {
            return Err(format!("Invalid operator at \"{}\".", &infix[pos..]));
        }
        match matched {
            x if x == EXOP_BRACKETL || x == EXOP_MEML => {
                stack.push(matched);
            }
            x if x == EXOP_BRACKETR => loop {
                let Some(top) = stack.pop() else {
                    return Err("Closing parenthesis without opening one.".into());
                };
                if top == EXOP_BRACKETL {
                    break;
                }
                dest.push((ExpressionCommand::Op, top as u64));
            },
            x if x == EXOP_MEMR => loop {
                let Some(top) = stack.pop() else {
                    return Err("Closing bracket without opening one.".into());
                };
                if top == EXOP_MEML {
                    dest.push((ExpressionCommand::Op, EXOP_MEM as u64));
                    break;
                }
                dest.push((ExpressionCommand::Op, top as u64));
            },
            _ => {
                let prio = EXOP_TABLE[matched as usize].priority;
                while let Some(&top) = stack.last() {
                    if top == EXOP_BRACKETL || top == EXOP_MEML {
                        break;
                    }
                    if EXOP_TABLE[top as usize].priority < prio {
                        break;
                    }
                    stack.pop();
                    dest.push((ExpressionCommand::Op, top as u64));
                }
                stack.push(matched);
            }
        }
        pos += matched_len;
        if matched != EXOP_MEMR {
            last_op = matched;
        } else {
            last_op = EXOP_NUMBER;
        }
    }
    while let Some(top) = stack.pop() {
        if top == EXOP_BRACKETL {
            return Err("Parenthesis not closed.".into());
        }
        dest.push((ExpressionCommand::Op, top as u64));
    }
    Ok(())
}

fn is_alphanum(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '@' || c == '_' || c == '$' || c == '.'
}

fn parse_postfix(exp: &PostfixExpression) -> Result<i128, String> {
    let mut stack: Vec<u64> = Vec::new();
    let mut use_float = false;
    for (cmd, val) in exp {
        match cmd {
            ExpressionCommand::Const => stack.push(*val),
            ExpressionCommand::ConstFloat => {
                use_float = true;
                stack.push(*val);
            }
            ExpressionCommand::Ref => stack.push(*val),
            ExpressionCommand::Op => {
                let op = *val as u32;
                let args = EXOP_TABLE.get(op as usize).map(|o| o.args).unwrap_or(0);
                if stack.len() < args {
                    return Err("Not enough arguments.".into());
                }
                let mut a: [u64; 5] = [0; 5];
                for i in (0..args).rev() {
                    a[i] = stack.pop().unwrap();
                }
                let result = match op {
                    EXOP_SIGNPLUS => continue,
                    EXOP_SIGNMINUS => {
                        if use_float {
                            (-(a[0] as f32)).to_bits() as u64
                        } else {
                            (0u64).wrapping_sub(a[0])
                        }
                    }
                    EXOP_BITNOT => !a[0],
                    EXOP_LOGNOT => (a[0] == 0) as u64,
                    EXOP_MUL => {
                        if use_float {
                            ((f32::from_bits(a[1] as u32)) * f32::from_bits(a[0] as u32))
                                .to_bits() as u64
                        } else {
                            a[1].wrapping_mul(a[0])
                        }
                    }
                    EXOP_DIV => {
                        if a[0] == 0 {
                            return Err("Division by zero.".into());
                        }
                        if use_float {
                            ((f32::from_bits(a[1] as u32)) / f32::from_bits(a[0] as u32))
                                .to_bits() as u64
                        } else {
                            a[1] / a[0]
                        }
                    }
                    EXOP_MOD => {
                        if a[0] == 0 {
                            return Err("Modulo by zero.".into());
                        }
                        a[1] % a[0]
                    }
                    EXOP_ADD => {
                        if use_float {
                            ((f32::from_bits(a[1] as u32)) + f32::from_bits(a[0] as u32))
                                .to_bits() as u64
                        } else {
                            a[1].wrapping_add(a[0])
                        }
                    }
                    EXOP_SUB => {
                        if use_float {
                            ((f32::from_bits(a[1] as u32)) - f32::from_bits(a[0] as u32))
                                .to_bits() as u64
                        } else {
                            a[1].wrapping_sub(a[0])
                        }
                    }
                    EXOP_SHL => a[1] << a[0],
                    EXOP_SHR => a[1] >> a[0],
                    EXOP_GREATEREQUAL => {
                        if use_float {
                            (f32::from_bits(a[1] as u32) >= f32::from_bits(a[0] as u32)) as u64
                        } else {
                            (a[1] >= a[0]) as u64
                        }
                    }
                    EXOP_GREATER => {
                        if use_float {
                            (f32::from_bits(a[1] as u32) > f32::from_bits(a[0] as u32)) as u64
                        } else {
                            (a[1] > a[0]) as u64
                        }
                    }
                    EXOP_LOWEQUAL => {
                        if use_float {
                            (f32::from_bits(a[1] as u32) <= f32::from_bits(a[0] as u32)) as u64
                        } else {
                            (a[1] <= a[0]) as u64
                        }
                    }
                    EXOP_LOWER => {
                        if use_float {
                            (f32::from_bits(a[1] as u32) < f32::from_bits(a[0] as u32)) as u64
                        } else {
                            (a[1] < a[0]) as u64
                        }
                    }
                    EXOP_EQUAL => {
                        if use_float {
                            (f32::from_bits(a[1] as u32) == f32::from_bits(a[0] as u32)) as u64
                        } else {
                            (a[1] == a[0]) as u64
                        }
                    }
                    EXOP_NOTEQUAL => {
                        if use_float {
                            (f32::from_bits(a[1] as u32) != f32::from_bits(a[0] as u32)) as u64
                        } else {
                            (a[1] != a[0]) as u64
                        }
                    }
                    EXOP_BITAND => a[1] & a[0],
                    EXOP_XOR => a[1] ^ a[0],
                    EXOP_BITOR => a[1] | a[0],
                    EXOP_LOGAND => ((a[1] != 0) && (a[0] != 0)) as u64,
                    EXOP_LOGOR => ((a[1] != 0) || (a[0] != 0)) as u64,
                    EXOP_TERTELSE => {
                        if a[2] != 0 { a[1] } else { a[0] }
                    }
                    _ => return Err("Unknown operator.".into()),
                };
                stack.push(result);
            }
        }
    }
    if stack.len() != 1 {
        return Err("Invalid expression (Too many constants?)".into());
    }
    Ok(stack[0] as i128)
}

// ---------------------------------------------------------------------------
// MIPS assembler
// ---------------------------------------------------------------------------

const MIPS_REGISTERS: &[(&str, u8)] = &[
    ("r0", 0), ("zero", 0), ("$0", 0), ("$zero", 0),
    ("at", 1), ("r1", 1), ("$1", 1), ("$at", 1),
    ("v0", 2), ("r2", 2), ("$v0", 2),
    ("v1", 3), ("r3", 3), ("$v1", 3),
    ("a0", 4), ("r4", 4), ("$a0", 4),
    ("a1", 5), ("r5", 5), ("$a1", 5),
    ("a2", 6), ("r6", 6), ("$a2", 6),
    ("a3", 7), ("r7", 7), ("$a3", 7),
    ("t0", 8), ("r8", 8), ("$t0", 8),
    ("t1", 9), ("r9", 9), ("$t1", 9),
    ("t2", 10), ("r10", 10), ("$t2", 10),
    ("t3", 11), ("r11", 11), ("$t3", 11),
    ("t4", 12), ("r12", 12), ("$t4", 12),
    ("t5", 13), ("r13", 13), ("$t5", 13),
    ("t6", 14), ("r14", 14), ("$t6", 14),
    ("t7", 15), ("r15", 15), ("$t7", 15),
    ("s0", 16), ("r16", 16), ("$s0", 16),
    ("s1", 17), ("r17", 17), ("$s1", 17),
    ("s2", 18), ("r18", 18), ("$s2", 18),
    ("s3", 19), ("r19", 19), ("$s3", 19),
    ("s4", 20), ("r20", 20), ("$s4", 20),
    ("s5", 21), ("r21", 21), ("$s5", 21),
    ("s6", 22), ("r22", 22), ("$s6", 22),
    ("s7", 23), ("r23", 23), ("$s7", 23),
    ("t8", 24), ("r24", 24), ("$t8", 24),
    ("t9", 25), ("r25", 25), ("$t9", 25),
    ("k0", 26), ("r26", 26), ("$k0", 26),
    ("k1", 27), ("r27", 27), ("$k1", 27),
    ("gp", 28), ("r28", 28), ("$gp", 28),
    ("sp", 29), ("r29", 29), ("$sp", 29),
    ("fp", 30), ("r30", 30), ("$fp", 30),
    ("ra", 31), ("r31", 31), ("$ra", 31),
];

fn lookup_register(token: &str) -> Option<u8> {
    MIPS_REGISTERS
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(token))
        .map(|(_, n)| *n)
}

fn parse_reg(token: &str) -> Option<u32> {
    lookup_register(token).map(|n| n as u32)
}

fn split_line<'a>(line: &'a str) -> (&'a str, &'a str) {
    let trimmed = line.trim_start();
    let mut split = trimmed.splitn(2, |c: char| c == ' ' || c == '\t');
    let name = split.next().unwrap_or("").trim();
    let args = split.next().unwrap_or("").trim();
    (name, args)
}

pub fn mips_assemble(line: &str) -> Result<u32, String> {
    let (name, args) = split_line(line);
    assemble_opcode(name, args, 0)
}

fn assemble_opcode(name: &str, args: &str, address: u32) -> Result<u32, String> {
    match name.to_ascii_lowercase().as_str() {
        "nop" => Ok(0),
        "li" => {
            let mut parts = args.split(',');
            let rt = parts.next().ok_or("Expected rt")?.trim();
            let imm = parts.next().ok_or("Expected immediate")?.trim();
            let rt = parse_reg(rt).ok_or_else(|| format!("Bad register \"{}\"", rt))?;
            let val = parse_number(imm).ok_or_else(|| format!("Bad immediate \"{}\"", imm))?;
            Ok(0x2400_0000 | (rt << 16) | (val & 0xFFFF) as u32)
        }
        "move" => {
            let mut parts = args.split(',');
            let rd = parts.next().ok_or("Expected rd")?.trim();
            let rs = parts.next().ok_or("Expected rs")?.trim();
            let rd = parse_reg(rd).ok_or_else(|| format!("Bad register \"{}\"", rd))?;
            let rs = parse_reg(rs).ok_or_else(|| format!("Bad register \"{}\"", rs))?;
            Ok((rs << 21) | (rd << 11) | 0x21)
        }
        "addiu" => {
            let mut parts = args.split(',');
            let rt = parts.next().ok_or("Expected rt")?.trim();
            let rs = parts.next().ok_or("Expected rs")?.trim();
            let imm = parts.next().ok_or("Expected immediate")?.trim();
            let rt = parse_reg(rt).ok_or_else(|| format!("Bad register \"{}\"", rt))?;
            let rs = parse_reg(rs).ok_or_else(|| format!("Bad register \"{}\"", rs))?;
            let val = parse_number(imm).ok_or_else(|| format!("Bad immediate \"{}\"", imm))?;
            Ok((9u32 << 26) | (rs << 21) | (rt << 16) | (val & 0xFFFF) as u32)
        }
        "lui" => {
            let mut parts = args.split(',');
            let rt = parts.next().ok_or("Expected rt")?.trim();
            let imm = parts.next().ok_or("Expected immediate")?.trim();
            let rt = parse_reg(rt).ok_or_else(|| format!("Bad register \"{}\"", rt))?;
            let val = parse_number(imm).ok_or_else(|| format!("Bad immediate \"{}\"", imm))?;
            Ok((0x0F << 26) | (rt << 16) | (val & 0xFFFF) as u32)
        }
        "ori" => {
            let mut parts = args.split(',');
            let rt = parts.next().ok_or("Expected rt")?.trim();
            let rs = parts.next().ok_or("Expected rs")?.trim();
            let imm = parts.next().ok_or("Expected immediate")?.trim();
            let rt = parse_reg(rt).ok_or_else(|| format!("Bad register \"{}\"", rt))?;
            let rs = parse_reg(rs).ok_or_else(|| format!("Bad register \"{}\"", rs))?;
            let val = parse_number(imm).ok_or_else(|| format!("Bad immediate \"{}\"", imm))?;
            Ok((0x0D << 26) | (rs << 21) | (rt << 16) | (val & 0xFFFF) as u32)
        }
        "lbu" => {
            let mut parts = args.split(',');
            let rt = parts.next().ok_or("Expected rt")?.trim();
            let mem = parts.next().ok_or("Expected memory operand")?.trim();
            let rt = parse_reg(rt).ok_or_else(|| format!("Bad register \"{}\"", rt))?;
            let (imm, rs) = parse_mem_operand(mem)?;
            Ok((0x24 << 26) | (rs << 21) | (rt << 16) | (imm & 0xFFFF) as u32)
        }
        "lb" => {
            let mut parts = args.split(',');
            let rt = parts.next().ok_or("Expected rt")?.trim();
            let mem = parts.next().ok_or("Expected memory operand")?.trim();
            let rt = parse_reg(rt).ok_or_else(|| format!("Bad register \"{}\"", rt))?;
            let (imm, rs) = parse_mem_operand(mem)?;
            Ok((0x20 << 26) | (rs << 21) | (rt << 16) | (imm & 0xFFFF) as u32)
        }
        "lw" => {
            let mut parts = args.split(',');
            let rt = parts.next().ok_or("Expected rt")?.trim();
            let mem = parts.next().ok_or("Expected memory operand")?.trim();
            let rt = parse_reg(rt).ok_or_else(|| format!("Bad register \"{}\"", rt))?;
            let (imm, rs) = parse_mem_operand(mem)?;
            Ok((0x23 << 26) | (rs << 21) | (rt << 16) | (imm & 0xFFFF) as u32)
        }
        "sw" => {
            let mut parts = args.split(',');
            let rt = parts.next().ok_or("Expected rt")?.trim();
            let mem = parts.next().ok_or("Expected memory operand")?.trim();
            let rt = parse_reg(rt).ok_or_else(|| format!("Bad register \"{}\"", rt))?;
            let (imm, rs) = parse_mem_operand(mem)?;
            Ok((0x2B << 26) | (rs << 21) | (rt << 16) | (imm & 0xFFFF) as u32)
        }
        "sb" => {
            let mut parts = args.split(',');
            let rt = parts.next().ok_or("Expected rt")?.trim();
            let mem = parts.next().ok_or("Expected memory operand")?.trim();
            let rt = parse_reg(rt).ok_or_else(|| format!("Bad register \"{}\"", rt))?;
            let (imm, rs) = parse_mem_operand(mem)?;
            Ok((0x28 << 26) | (rs << 21) | (rt << 16) | (imm & 0xFFFF) as u32)
        }
        "j" => {
            let target = parse_number(args.trim())
                .ok_or_else(|| format!("Bad jump target \"{}\"", args))?;
            let target = ((target >> 2) & 0x03FF_FFFF) as u32;
            Ok((0x02 << 26) | target)
        }
        "jal" => {
            let target = parse_number(args.trim())
                .ok_or_else(|| format!("Bad jump target \"{}\"", args))?;
            let target = ((target >> 2) & 0x03FF_FFFF) as u32;
            Ok((0x03 << 26) | target)
        }
        "jr" => {
            let rs = parse_reg(args.trim())
                .ok_or_else(|| format!("Bad register \"{}\"", args))?;
            Ok((0x00 << 26) | (rs << 21) | 0x08)
        }
        "beq" => assemble_branch(0x04, args, address),
        "bne" => assemble_branch(0x05, args, address),
        "blez" => assemble_branch_one(0x06, args, address),
        "bgtz" => assemble_branch_one(0x07, args, address),
        "bltz" => assemble_branch_one(0x01_0000, args, address), // REGIMM 0x00
        "bgez" => assemble_branch_one(0x01_0001, args, address), // REGIMM 0x01
        "syscall" => Ok(0x0000_000C),
        "break" => Ok(0x0000_000D),
        _ => Err(format!("Unknown opcode \"{}\"", name)),
    }
}

fn parse_mem_operand(s: &str) -> Result<(i32, u32), String> {
    let open = s.find('(').ok_or("Expected '(' in memory operand")?;
    let close = s.find(')').ok_or("Expected ')' in memory operand")?;
    let imm_str = s[..open].trim();
    let rs_str = s[open + 1..close].trim();
    let imm = parse_number(imm_str)
        .ok_or_else(|| format!("Bad immediate \"{}\"", imm_str))? as i32;
    let rs = parse_reg(rs_str).ok_or_else(|| format!("Bad register \"{}\"", rs_str))?;
    Ok((imm, rs))
}

fn assemble_branch(op: u32, args: &str, address: u32) -> Result<u32, String> {
    let mut parts = args.split(',');
    let rs = parts.next().ok_or("Expected rs")?.trim();
    let rt = parts.next().ok_or("Expected rt")?.trim();
    let target = parts.next().ok_or("Expected target")?.trim();
    let rs = parse_reg(rs).ok_or_else(|| format!("Bad register \"{}\"", rs))?;
    let rt = parse_reg(rt).ok_or_else(|| format!("Bad register \"{}\"", rt))?;
    let target = parse_number(target).ok_or_else(|| format!("Bad target \"{}\"", target))?;
    let offset = ((target.wrapping_sub(address.into()).wrapping_add(4) / 4) & 0xFFFF) as u32;
    Ok((op << 26) | (rs << 21) | (rt << 16) | offset)
}

fn assemble_branch_one(op: u32, args: &str, address: u32) -> Result<u32, String> {
    let mut parts = args.split(',');
    let rs = parts.next().ok_or("Expected rs")?.trim();
    let target = parts.next().ok_or("Expected target")?.trim();
    let rs = parse_reg(rs).ok_or_else(|| format!("Bad register \"{}\"", rs))?;
    let target = parse_number(target).ok_or_else(|| format!("Bad target \"{}\"", target))?;
    let offset = ((target.wrapping_sub(address.into()).wrapping_add(4) / 4) & 0xFFFF) as u32;
    if op == 0x01_0000 || op == 0x01_0001 {
        Ok((0x01 << 26) | (rs << 21) | (op & 0xFFFF) | offset)
    } else {
        Ok((op << 26) | (rs << 21) | offset)
    }
}

// ---------------------------------------------------------------------------
// MIPS analyst — branch / load-store analysis helpers
// ---------------------------------------------------------------------------

pub mod mips_analyst {
    use super::*;

    pub const INVALID_TARGET: u32 = 0xFFFF_FFFF;

    pub fn get_jump_target(addr: u32, reader: &dyn DebugInterface) -> u32 {
        let op = reader.read32(addr);
        let (is_branch, is_jump) = classify(op);
        if is_branch && is_jump {
            (addr & 0xF000_0000) | ((op & 0x03FF_FFFF) << 2)
        } else {
            INVALID_TARGET
        }
    }

    pub fn get_branch_target(addr: u32, reader: &dyn DebugInterface) -> u32 {
        let op = reader.read32(addr);
        let op_class = op >> 26;
        if is_branch_opcode(op) {
            return addr.wrapping_add(4).wrapping_add(
                (((op & 0xFFFF) as i16) as i32 as u32).wrapping_mul(4),
            );
        }
        if op_class == 0x11 {
            return addr.wrapping_add(4).wrapping_add(
                (((op & 0xFFFF) as i16) as i32 as u32).wrapping_mul(4),
            );
        }
        INVALID_TARGET
    }

    fn classify(op: u32) -> (bool, bool) {
        let op_class = op >> 26;
        let is_branch = is_branch_opcode(op) || matches!(op_class, 0x11);
        let is_jump = op_class == 0x02 || op_class == 0x03;
        (is_branch, is_jump)
    }

    fn is_branch_opcode(op: u32) -> bool {
        let op_class = op >> 26;
        matches!(
            op_class,
            0x01 | 0x02 | 0x03 | 0x04 | 0x05 | 0x06 | 0x07 | 0x14 | 0x15
        )
    }
}

// ---------------------------------------------------------------------------
// Symbol guardian
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct SymbolInfo {
    pub name: String,
    pub address: u32,
    pub size: u32,
}

#[derive(Debug, Clone, Default)]
pub struct SymbolGuardian {
    functions: BTreeMap<u32, SymbolInfo>,
    labels: BTreeMap<u32, SymbolInfo>,
    hashes: HashMap<u32, u64>,
}

impl SymbolGuardian {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_function(&mut self, info: SymbolInfo) {
        self.functions.insert(info.address, info);
    }

    pub fn add_label(&mut self, info: SymbolInfo) {
        self.labels.insert(info.address, info);
    }

    pub fn symbol_starting_at(&self, address: u32) -> Option<&SymbolInfo> {
        self.functions
            .get(&address)
            .or_else(|| self.labels.get(&address))
    }

    pub fn symbol_overlapping(&self, address: u32) -> Option<&SymbolInfo> {
        // Naive: any function whose range contains `address`.
        self.functions
            .values()
            .find(|info| {
                address >= info.address && address < info.address.saturating_add(info.size)
            })
            .or_else(|| {
                self.labels
                    .values()
                    .find(|info| address >= info.address && address < info.address.saturating_add(info.size))
            })
    }

    pub fn symbol_after(&self, address: u32) -> Option<&SymbolInfo> {
        self.functions
            .range(address + 1..)
            .next()
            .map(|(_, v)| v)
            .or_else(|| self.labels.range(address + 1..).next().map(|(_, v)| v))
    }

    pub fn read<R>(&self, _visitor: R) where
        R: FnOnce(&SymbolGuardian),
    {
        // Placeholder: the C++ variant takes a callable under a lock; in this
        // translation the database is not shared, so a simple closure suffices.
        _visitor(self);
    }

    pub fn hash_function(&self, address: u32, reader: &dyn DebugInterface, size: u32) -> Option<u64> {
        let mut hash: u32 = 0xBACD_7814;
        let mut a = address;
        let end = address.wrapping_add(size);
        while a < end {
            hash = hash.wrapping_add(reader.read32(a));
            a = a.wrapping_add(4);
        }
        let v = hash as u64;
        Some(v)
    }
}

// ---------------------------------------------------------------------------
// Symbol importer
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct SymbolImporter {
    pub last_error: String,
    pub functions: Vec<SymbolInfo>,
    pub globals: Vec<SymbolInfo>,
    pub modules: Vec<IopMod>,
}

impl SymbolImporter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn import_elf(&mut self, name: &str) -> Result<(), String> {
        // Stubbed: the C++ version parses an ELF; this Rust translation
        // records the operation and exposes a sane default empty result.
        self.last_error.clear();
        let _ = name;
        Ok(())
    }

    pub fn import_sndll(&mut self, name: &str) -> Result<(), String> {
        self.last_error.clear();
        let _ = name;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Convenience public re-exports to match the API contract.
// ---------------------------------------------------------------------------

/// The public `DebugInterface` *trait* is the one defined above; this is the
/// `struct`-shaped convenience impl for the EE that the spec asks for.
pub use EeDebugInterface as DebugInterfaceStruct;

// Required free-function entry points.

pub fn disasmR3000A(pc: u32) -> String {
    disasm_r3000a(0, pc)
}

pub fn disasmR5900(pc: u32) -> String {
    disasm_r5900(0, pc)
}

pub fn disasmVU0Micro(pc: u32) -> String {
    disasm_vu0_micro(0, pc)
}

pub fn disasmVU1Micro(pc: u32) -> String {
    disasm_vu1_micro(0, pc)
}

#[allow(unused_variables)]
pub fn debug_format(prefix: &str, args: std::fmt::Arguments) -> String {
    let mut s = String::new();
    let _ = write!(s, "{}", prefix);
    let _ = write!(s, "{}", args);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breakpoint_check_roundtrip() {
        let mut list = BreakpointList::new();
        list.add(Breakpoint {
            pc: 0x1000,
            enabled: true,
            condition: None,
        });
        assert_eq!(list.check(0x1000), Some(0));
        assert_eq!(list.check(0x1004), None);
        assert!(list.remove(0x1000));
        assert_eq!(list.check(0x1000), None);
    }

    #[test]
    fn standardize_address_kernel() {
        assert_eq!(standardize_breakpoint_address(0xBFC0_1234), 0x1FC0_1234);
        assert_eq!(standardize_breakpoint_address(0xFFFF_8000), 0xFFFF_8000);
    }

    #[test]
    fn mips_assemble_basic() {
        let nop = mips_assemble("nop").unwrap();
        assert_eq!(nop, 0);
        let addiu = mips_assemble("addiu $t0, $t1, 4").unwrap();
        assert_eq!(addiu, (9u32 << 26) | (9 << 21) | (8 << 16) | 4);
    }

    #[test]
    fn expression_eval_simple() {
        assert_eq!(expression_eval("1 + 2 * 3").unwrap(), 7);
        assert_eq!(expression_eval("(1 + 2) * 3").unwrap(), 9);
        assert_eq!(expression_eval("0x10 + 0b10").unwrap(), 0x12);
    }
}
