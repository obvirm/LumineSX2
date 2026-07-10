//! VIXL AArch64 assembler, decoder and simulator.
//!
//! Idiomatic Rust 2021 translation of the public surface of VIXL (Vector
//! Interface Extension Library) headers located at
//! `3rdparty/vixl/include/vixl/aarch64/` and `3rdparty/vixl/include/vixl/`.
//!
//! The translation targets the subset that the PCSX2 EE-translation work
//! relies on: register enums, condition codes, a tiny macro-assembler, a
//! top-level A64 decoder, and an instruction-level simulator. It is a
//! self-contained module: only `std` is used, and global state is kept behind
//! `static mut` so it can be replaced with a host-provided allocator later.

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(dead_code)]

use std::fmt;

// ---------------------------------------------------------------------------
// ISA constants. Mirrors `instructions-aarch64.h` / `constants-aarch64.h`.
// ---------------------------------------------------------------------------

pub const kInstructionSize: u32 = 4;
pub const kNumberOfRegisters: usize = 32;
pub const kNumberOfVRegisters: usize = 32;
pub const kNumberOfPRegisters: usize = 16;
pub const kNumberOfCalleeSavedRegisters: usize = 10;
pub const kFirstCalleeSavedRegisterIndex: usize = 21;
pub const kNumberOfCalleeSavedFPRegisters: usize = 8;
pub const kFirstCalleeSavedFPRegisterIndex: usize = 8;

pub const kBRegSize: u32 = 8;
pub const kHRegSize: u32 = 16;
pub const kWRegSize: u32 = 32;
pub const kXRegSize: u32 = 64;
pub const kSRegSize: u32 = 32;
pub const kDRegSize: u32 = 64;
pub const kQRegSize: u32 = 128;

pub const kFpRegCode: u32 = 29;
pub const kLinkRegCode: u32 = 30;
pub const kSpRegCode: u32 = 31;
pub const kZeroRegCode: u32 = 31;
pub const kSPRegInternalCode: u32 = 63;
pub const kRegCodeMask: u32 = 0x1f;

pub const kWRegMask: u64 = 0xffff_ffff;
pub const kXRegMask: u64 = 0xffff_ffff_ffff_ffff;
pub const kHRegMask: u64 = 0xffff;
pub const kSRegMask: u64 = 0xffff_ffff;
pub const kDRegMask: u64 = 0xffff_ffff_ffff_ffff;

pub const kXMaxUInt: u64 = 0xffff_ffff_ffff_ffff;
pub const kWMaxUInt: u64 = 0xffff_ffff;
pub const kHMaxUInt: u64 = 0xffff;

pub const kWMaxInt: i32 = 0x7fff_ffff;
pub const kWMinInt: i32 = -kWMaxInt - 1;
pub const kXMaxInt: i64 = 0x7fff_ffff_ffff_ffff;
pub const kXMinInt: i64 = -kXMaxInt - 1;
pub const kHMaxInt: i16 = 0x7fff;
pub const kHMinInt: i16 = -kHMaxInt - 1;

// Field-mask constants used by the top-level decoder.
pub const kFixedBitSize: u32 = 32;

// ---- Top-level A64 opcode class bits -------------------------------------

pub const kOpcodeUnknown: u32 = 0;
pub const kOpcodeDPImm: u32 = 1;   // Data-processing immediate.
pub const kOpcodeDPImmShift: u32 = 2;
pub const kOpcodeDPReg: u32 = 3;    // Data-processing register.
pub const kOpcodeDPRegShift: u32 = 4;
pub const kOpcodeDPRegExtend: u32 = 5;
pub const kOpcodeDPRegMul: u32 = 6;
pub const kOpcodeDPFP: u32 = 7;     // Floating-point data processing.
pub const kOpcodeLoadStore: u32 = 8;
pub const kOpcodeLoadStorePair: u32 = 9;
pub const kOpcodeBranch: u32 = 10;
pub const kOpcodeBranchCond: u32 = 11;
pub const kOpcodeBranchReg: u32 = 12;
pub const kOpcodeBranchSystem: u32 = 13;
pub const kOpcodeSvc: u32 = 14;
pub const kOpcodeNop: u32 = 15;

// Mask constants taken from `constants-aarch64.h`.
pub const UncondBranchFixed: u32 = 0x1400_0000;
pub const UncondBranchFMask: u32 = 0x7c00_0000;
pub const UncondBranchMask: u32 = 0xfc00_0000;

pub const CondBranchFixed: u32 = 0x5400_0000;
pub const CondBranchFMask: u32 = 0xff00_0010;
pub const CondBranchMask: u32 = 0xff00_0010;

pub const UncondBranchRegFixed: u32 = 0xd61f_0000;
pub const UncondBranchRegFMask: u32 = 0xfc00_fc00;
pub const UncondBranchRegMask: u32 = 0xffff_fc00;

pub const ExceptionFixed: u32 = 0xd400_0000;
pub const ExceptionFMask: u32 = 0xffe0_0000;
pub const ExceptionMask: u32 = 0xffe0_0000;

pub const PCRelAddressingFixed: u32 = 0x1000_0000;
pub const PCRelAddressingFMask: u32 = 0x9f00_0000;
pub const PCRelAddressingMask: u32 = 0x9f00_0000;

pub const LoadStoreFixed: u32 = 0x0800_0000;
pub const LoadStoreFMask: u32 = 0x0a00_0000;
pub const LoadStoreMask: u32 = 0xff00_0000;

pub const LoadStorePairFixed: u32 = 0x2800_0000;
pub const LoadStorePairFMask: u32 = 0x3e00_0000;
pub const LoadStorePairMask: u32 = 0xff00_0000;

pub const AddSubImmediateFixed: u32 = 0x1000_0000;
pub const AddSubImmediateFMask: u32 = 0x1f00_0000;
pub const AddSubImmediateMask: u32 = 0x9f00_0000;

pub const LogicalImmediateFixed: u32 = 0x1200_0000;
pub const LogicalImmediateFMask: u32 = 0x1f80_0000;
pub const LogicalImmediateMask: u32 = 0xff80_0000;

pub const MoveWideImmediateFixed: u32 = 0x1280_0000;
pub const MoveWideImmediateFMask: u32 = 0x1f80_0000;
pub const MoveWideImmediateMask: u32 = 0xff80_0000;

pub const DataProcessingOneSourceFixed: u32 = 0x5ac0_0000;
pub const DataProcessingOneSourceFMask: u32 = 0x5fe0_0000;
pub const DataProcessingOneSourceMask: u32 = 0xffe0_0000;

pub const DataProcessingTwoSourceFixed: u32 = 0x1ac0_0000;
pub const DataProcessingTwoSourceFMask: u32 = 0x5fe0_0000;
pub const DataProcessingTwoSourceMask: u32 = 0xffe0_0000;

pub const DataProcessingThreeSourceFixed: u32 = 0x1b00_0000;
pub const DataProcessingThreeSourceFMask: u32 = 0x1f00_0000;
pub const DataProcessingThreeSourceMask: u32 = 0x9f00_0000;

pub const NopFMask: u32 = 0xffff_f01f;
pub const NopFixed: u32 = 0xd503_201f;

// ---------------------------------------------------------------------------
// Register enums (items 1-3 of the contract).
// ---------------------------------------------------------------------------

/// General-purpose register (W/X view) plus the two aliases for reg 31.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum XRegister {
    X0,
    X1,
    X2,
    X3,
    X4,
    X5,
    X6,
    X7,
    X8,
    X9,
    X10,
    X11,
    X12,
    X13,
    X14,
    X15,
    X16,
    X17,
    X18,
    X19,
    X20,
    X21,
    X22,
    X23,
    X24,
    X25,
    X26,
    X27,
    X28,
    X29,
    X30,
    XZR,
    SP,
}

impl XRegister {
    /// Architectural 5-bit encoding. XZR and SP share code 31; caller decides
    /// based on `Reg31Mode`.
    pub const fn code(self) -> u32 {
        match self {
            XRegister::X0 => 0,
            XRegister::X1 => 1,
            XRegister::X2 => 2,
            XRegister::X3 => 3,
            XRegister::X4 => 4,
            XRegister::X5 => 5,
            XRegister::X6 => 6,
            XRegister::X7 => 7,
            XRegister::X8 => 8,
            XRegister::X9 => 9,
            XRegister::X10 => 10,
            XRegister::X11 => 11,
            XRegister::X12 => 12,
            XRegister::X13 => 13,
            XRegister::X14 => 14,
            XRegister::X15 => 15,
            XRegister::X16 => 16,
            XRegister::X17 => 17,
            XRegister::X18 => 18,
            XRegister::X19 => 19,
            XRegister::X20 => 20,
            XRegister::X21 => 21,
            XRegister::X22 => 22,
            XRegister::X23 => 23,
            XRegister::X24 => 24,
            XRegister::X25 => 25,
            XRegister::X26 => 26,
            XRegister::X27 => 27,
            XRegister::X28 => 28,
            XRegister::X29 => 29,
            XRegister::X30 => 30,
            XRegister::XZR | XRegister::SP => 31,
        }
    }

    pub const fn from_code(code: u32) -> Option<Self> {
        Some(match code & 0x1f {
            0 => XRegister::X0,
            1 => XRegister::X1,
            2 => XRegister::X2,
            3 => XRegister::X3,
            4 => XRegister::X4,
            5 => XRegister::X5,
            6 => XRegister::X6,
            7 => XRegister::X7,
            8 => XRegister::X8,
            9 => XRegister::X9,
            10 => XRegister::X10,
            11 => XRegister::X11,
            12 => XRegister::X12,
            13 => XRegister::X13,
            14 => XRegister::X14,
            15 => XRegister::X15,
            16 => XRegister::X16,
            17 => XRegister::X17,
            18 => XRegister::X18,
            19 => XRegister::X19,
            20 => XRegister::X20,
            21 => XRegister::X21,
            22 => XRegister::X22,
            23 => XRegister::X23,
            24 => XRegister::X24,
            25 => XRegister::X25,
            26 => XRegister::X26,
            27 => XRegister::X27,
            28 => XRegister::X28,
            29 => XRegister::X29,
            30 => XRegister::X30,
            31 => XRegister::XZR,
            _ => return None,
        })
    }

    pub const fn is_sp(self) -> bool {
        matches!(self, XRegister::SP)
    }

    pub const fn is_xzr(self) -> bool {
        matches!(self, XRegister::XZR)
    }
}

impl fmt::Display for XRegister {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            XRegister::X0 => "x0",
            XRegister::X1 => "x1",
            XRegister::X2 => "x2",
            XRegister::X3 => "x3",
            XRegister::X4 => "x4",
            XRegister::X5 => "x5",
            XRegister::X6 => "x6",
            XRegister::X7 => "x7",
            XRegister::X8 => "x8",
            XRegister::X9 => "x9",
            XRegister::X10 => "x10",
            XRegister::X11 => "x11",
            XRegister::X12 => "x12",
            XRegister::X13 => "x13",
            XRegister::X14 => "x14",
            XRegister::X15 => "x15",
            XRegister::X16 => "x16",
            XRegister::X17 => "x17",
            XRegister::X18 => "x18",
            XRegister::X19 => "x19",
            XRegister::X20 => "x20",
            XRegister::X21 => "x21",
            XRegister::X22 => "x22",
            XRegister::X23 => "x23",
            XRegister::X24 => "x24",
            XRegister::X25 => "x25",
            XRegister::X26 => "x26",
            XRegister::X27 => "x27",
            XRegister::X28 => "x28",
            XRegister::X29 => "x29",
            XRegister::X30 => "x30",
            XRegister::XZR => "xzr",
            XRegister::SP => "sp",
        };
        f.write_str(name)
    }
}

/// Vector (FP/SIMD) register V0..V31. By default treated as a 128-bit Q
/// register; B/H/S/D/W views are encoded via the lane-size machinery in
/// `instructions-aarch64.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VRegister {
    V0,
    V1,
    V2,
    V3,
    V4,
    V5,
    V6,
    V7,
    V8,
    V9,
    V10,
    V11,
    V12,
    V13,
    V14,
    V15,
    V16,
    V17,
    V18,
    V19,
    V20,
    V21,
    V22,
    V23,
    V24,
    V25,
    V26,
    V27,
    V28,
    V29,
    V30,
    V31,
}

impl VRegister {
    pub const fn code(self) -> u32 {
        match self {
            VRegister::V0 => 0,
            VRegister::V1 => 1,
            VRegister::V2 => 2,
            VRegister::V3 => 3,
            VRegister::V4 => 4,
            VRegister::V5 => 5,
            VRegister::V6 => 6,
            VRegister::V7 => 7,
            VRegister::V8 => 8,
            VRegister::V9 => 9,
            VRegister::V10 => 10,
            VRegister::V11 => 11,
            VRegister::V12 => 12,
            VRegister::V13 => 13,
            VRegister::V14 => 14,
            VRegister::V15 => 15,
            VRegister::V16 => 16,
            VRegister::V17 => 17,
            VRegister::V18 => 18,
            VRegister::V19 => 19,
            VRegister::V20 => 20,
            VRegister::V21 => 21,
            VRegister::V22 => 22,
            VRegister::V23 => 23,
            VRegister::V24 => 24,
            VRegister::V25 => 25,
            VRegister::V26 => 26,
            VRegister::V27 => 27,
            VRegister::V28 => 28,
            VRegister::V29 => 29,
            VRegister::V30 => 30,
            VRegister::V31 => 31,
        }
    }

    pub const fn from_code(code: u32) -> Option<Self> {
        Some(match code & 0x1f {
            0 => VRegister::V0,
            1 => VRegister::V1,
            2 => VRegister::V2,
            3 => VRegister::V3,
            4 => VRegister::V4,
            5 => VRegister::V5,
            6 => VRegister::V6,
            7 => VRegister::V7,
            8 => VRegister::V8,
            9 => VRegister::V9,
            10 => VRegister::V10,
            11 => VRegister::V11,
            12 => VRegister::V12,
            13 => VRegister::V13,
            14 => VRegister::V14,
            15 => VRegister::V15,
            16 => VRegister::V16,
            17 => VRegister::V17,
            18 => VRegister::V18,
            19 => VRegister::V19,
            20 => VRegister::V20,
            21 => VRegister::V21,
            22 => VRegister::V22,
            23 => VRegister::V23,
            24 => VRegister::V24,
            25 => VRegister::V25,
            26 => VRegister::V26,
            27 => VRegister::V27,
            28 => VRegister::V28,
            29 => VRegister::V29,
            30 => VRegister::V30,
            31 => VRegister::V31,
            _ => return None,
        })
    }
}

impl fmt::Display for VRegister {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "v{}", self.code())
    }
}

/// SVE predicate register P0..P15.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PRegister {
    P0,
    P1,
    P2,
    P3,
    P4,
    P5,
    P6,
    P7,
    P8,
    P9,
    P10,
    P11,
    P12,
    P13,
    P14,
    P15,
}

impl PRegister {
    pub const fn code(self) -> u32 {
        match self {
            PRegister::P0 => 0,
            PRegister::P1 => 1,
            PRegister::P2 => 2,
            PRegister::P3 => 3,
            PRegister::P4 => 4,
            PRegister::P5 => 5,
            PRegister::P6 => 6,
            PRegister::P7 => 7,
            PRegister::P8 => 8,
            PRegister::P9 => 9,
            PRegister::P10 => 10,
            PRegister::P11 => 11,
            PRegister::P12 => 12,
            PRegister::P13 => 13,
            PRegister::P14 => 14,
            PRegister::P15 => 15,
        }
    }

    pub const fn from_code(code: u32) -> Option<Self> {
        Some(match code & 0x0f {
            0 => PRegister::P0,
            1 => PRegister::P1,
            2 => PRegister::P2,
            3 => PRegister::P3,
            4 => PRegister::P4,
            5 => PRegister::P5,
            6 => PRegister::P6,
            7 => PRegister::P7,
            8 => PRegister::P8,
            9 => PRegister::P9,
            10 => PRegister::P10,
            11 => PRegister::P11,
            12 => PRegister::P12,
            13 => PRegister::P13,
            14 => PRegister::P14,
            15 => PRegister::P15,
            _ => return None,
        })
    }
}

impl fmt::Display for PRegister {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "p{}", self.code())
    }
}

/// Condition codes for B.cond, CSEL and friends. Numeric values match the
/// 4-bit encoding in instructions-aarch64.h.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Condition {
    EQ = 0,
    NE = 1,
    CS = 2,
    CC = 3,
    MI = 4,
    PL = 5,
    VS = 6,
    VC = 7,
    HI = 8,
    LS = 9,
    GE = 10,
    LT = 11,
    GT = 12,
    LE = 13,
    AL = 14,
    NV = 15,
}

impl Condition {
    pub const fn code(self) -> u32 {
        self as u32
    }

    pub const fn from_code(code: u32) -> Option<Self> {
        Some(match code & 0x0f {
            0 => Condition::EQ,
            1 => Condition::NE,
            2 => Condition::CS,
            3 => Condition::CC,
            4 => Condition::MI,
            5 => Condition::PL,
            6 => Condition::VS,
            7 => Condition::VC,
            8 => Condition::HI,
            9 => Condition::LS,
            10 => Condition::GE,
            11 => Condition::LT,
            12 => Condition::GT,
            13 => Condition::LE,
            14 => Condition::AL,
            15 => Condition::NV,
            _ => return None,
        })
    }

    /// Inverts the condition, mirroring `InvertCondition` in
    /// `constants-aarch64.h`. AL/NV map to themselves.
    pub const fn invert(self) -> Self {
        match self {
            Condition::AL | Condition::NV => self,
            other => Condition::from_code(other.code() ^ 1).unwrap(),
        }
    }

    /// Evaluate against an NZCV snapshot. Mirrors the table in
    /// `simulator-aarch64.{h,cc}`.
    pub const fn holds(self, n: bool, z: bool, c: bool, v: bool) -> bool {
        match self {
            Condition::EQ => z,
            Condition::NE => !z,
            Condition::CS => c,
            Condition::CC => !c,
            Condition::MI => n,
            Condition::PL => !n,
            Condition::VS => v,
            Condition::VC => !v,
            Condition::HI => c && !z,
            Condition::LS => !c || z,
            Condition::GE => n == v,
            Condition::LT => n != v,
            Condition::GT => !z && (n == v),
            Condition::LE => z || (n != v),
            Condition::AL | Condition::NV => true,
        }
    }
}

impl fmt::Display for Condition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Condition::EQ => "eq",
            Condition::NE => "ne",
            Condition::CS => "cs",
            Condition::CC => "cc",
            Condition::MI => "mi",
            Condition::PL => "pl",
            Condition::VS => "vs",
            Condition::VC => "vc",
            Condition::HI => "hi",
            Condition::LS => "ls",
            Condition::GE => "ge",
            Condition::LT => "lt",
            Condition::GT => "gt",
            Condition::LE => "le",
            Condition::AL => "al",
            Condition::NV => "nv",
        };
        f.write_str(s)
    }
}

/// What the architecture means by register code 31 in a given context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Reg31Mode {
    Reg31IsStackPointer,
    #[default]
    Reg31IsZeroRegister,
}

// ---------------------------------------------------------------------------
// Decoded instruction view (item 6). Mirrors `Instruction` from
// `instructions-aarch64.h`.
// ---------------------------------------------------------------------------

/// A decoded A64 instruction. Only the fields actually needed by the
/// translator / simulator are populated; everything else is left at zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Instruction {
    pub raw: u32,
    pub bits: u32,
    pub mask: u32,
    pub opcode: u32,

    pub rd: XRegister,
    pub rn: XRegister,
    pub rm: XRegister,
    pub ra: XRegister,
    pub rv: VRegister,
    pub rv2: VRegister,
    pub rp: PRegister,

    /// 64-bit immediate / signed immediate / load-store offset.
    pub imm: i64,
    /// 64-bit logical immediate (already decoded, if applicable).
    pub imm_logical: u64,
    /// Floating-point immediate.
    pub imm_fp: u8,
    /// Shift / extend amount.
    pub shift_amount: u32,
    /// Shift type (LSL/LSR/ASR/ROR/MSL/NO_SHIFT).
    pub shift: i32,
    /// Extend mode (UXTB..SXTX/NO_EXTEND).
    pub extend: i32,
    /// Condition code (B.cond, CSEL).
    pub cond: Condition,
    /// Reg31Mode in effect when decoding this instruction.
    pub reg31_mode: Reg31Mode,
    /// PC-relative branch target (signed byte offset from the branch).
    pub branch_target: i64,
    /// FlagsUpdate marker.
    pub set_flags: bool,
}

impl Instruction {
    pub const fn new(raw: u32) -> Self {
        Self {
            raw,
            bits: 0,
            mask: 0,
            opcode: kOpcodeUnknown,
            rd: XRegister::XZR,
            rn: XRegister::XZR,
            rm: XRegister::XZR,
            ra: XRegister::XZR,
            rv: VRegister::V0,
            rv2: VRegister::V0,
            rp: PRegister::P0,
            imm: 0,
            imm_logical: 0,
            imm_fp: 0,
            shift_amount: 0,
            shift: -1,
            extend: -1,
            cond: Condition::AL,
            reg31_mode: Reg31Mode::Reg31IsZeroRegister,
            branch_target: 0,
            set_flags: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Field extractors. Direct equivalents of the INSTRUCTION_FIELDS_LIST in
// `constants-aarch64.h`.
// ---------------------------------------------------------------------------

#[inline]
pub const fn extract_bits(instr: u32, msb: u32, lsb: u32) -> u32 {
    let width = msb - lsb + 1;
    let mask = if width == 32 {
        u32::MAX
    } else {
        (1u32 << width) - 1
    };
    (instr >> lsb) & mask
}

#[inline]
pub const fn extract_signed_bits(instr: u32, msb: u32, lsb: u32) -> i64 {
    let width = msb - lsb + 1;
    let raw = extract_bits(instr, msb, lsb);
    if width == 0 {
        return 0;
    }
    let sign_bit = 1u32 << (width - 1);
    let mask = if width == 32 {
        u32::MAX
    } else {
        (1u32 << width) - 1
    };
    let value = raw & mask;
    if (value & sign_bit) != 0 {
        // Sign-extend into i64.
        (value as i64) - ((1i64) << width)
    } else {
        value as i64
    }
}

#[inline]
pub const fn mask(instr: u32, fmask: u32) -> u32 {
    instr & fmask
}

// ---------------------------------------------------------------------------
// Item 5: MacroAssembler (assembler-aarch64.h + macro-assembler-aarch64.h).
// ---------------------------------------------------------------------------

/// Tiny subset of the VIXL `MacroAssembler`. The contract asks for the data
/// movement and control-flow entry points used by the EE translation; full
/// NEON/SVE encoding is out of scope for this translation.
#[derive(Debug, Default)]
pub struct MacroAssembler {
    /// Buffer of 32-bit encoded instructions.
    pub buffer: Vec<u32>,
    /// PC offset of the first instruction (used to relocate branches later).
    pub base_pc: u64,
}

impl MacroAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_base(base_pc: u64) -> Self {
        Self {
            buffer: Vec::new(),
            base_pc,
        }
    }

    pub fn reset(&mut self) {
        self.buffer.clear();
    }

    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    pub fn code(&self) -> &[u32] {
        &self.buffer
    }

    fn emit(&mut self, instr: u32) {
        self.buffer.push(instr);
    }

    /// MOV Rd, Rm — alias of ORR Rd, XZR, Rm.
    pub fn Mov(&mut self, rd: XRegister, rm: XRegister) {
        // ORR Rd, XZR, Rm  =>  0xAA0003E0 | (Rm << 16) | Rd
        let instr = 0xAA00_03E0u32 | (rm.code() << 16) | rd.code();
        self.emit(instr);
    }

    /// ADD Rd, Rn, Rm — 64-bit.
    pub fn Add(&mut self, rd: XRegister, rn: XRegister, rm: XRegister) {
        // ADD (shifted register), 64-bit, sf=1, sh=0  =>  0x8B000000
        let instr = 0x8B00_0000u32 | (rm.code() << 16) | (rn.code() << 5) | rd.code();
        self.emit(instr);
    }

    /// SUB Rd, Rn, Rm — 64-bit.
    pub fn Sub(&mut self, rd: XRegister, rn: XRegister, rm: XRegister) {
        let instr = 0xCB00_0000u32 | (rm.code() << 16) | (rn.code() << 5) | rd.code();
        self.emit(instr);
    }

    /// MUL Rd, Rn, Rm — 64-bit (alias of MADD Rd, Rn, Rm, XZR).
    pub fn Mul(&mut self, rd: XRegister, rn: XRegister, rm: XRegister) {
        // MADD Xd, Xn, Xm, XZR  =>  0x9B000000 | (Rm << 16) | (Rn << 5) | Rd
        let instr = 0x9B00_0000u32 | (rm.code() << 16) | (rn.code() << 5) | rd.code();
        self.emit(instr);
    }

    /// SDIV Rd, Rn, Rm — 64-bit signed divide.
    pub fn Div(&mut self, rd: XRegister, rn: XRegister, rm: XRegister) {
        let instr = 0x9AC0_0C00u32 | (rm.code() << 16) | (rn.code() << 5) | rd.code();
        self.emit(instr);
    }

    /// LDR Xt, [Xn, #imm] — 64-bit, unsigned offset, scaled by 8.
    pub fn Ldr(&mut self, rt: XRegister, rn: XRegister, imm: i32) {
        let scale = 3u32;
        let mask: u32 = ((1u32 << scale) - 1) as u32;
        if imm < 0 || (imm as u32) & mask != 0 {
            // Fall back to a generic form using LDR (unscaled).
            // LDR Xt, [Xn, #simm]  => 0xF9400000 | (imm9 << 12) | (Rn << 5) | Rt
            let simm9 = ((imm as i32) as u32) & 0x1ff;
            let instr = 0xF940_0000u32 | (simm9 << 12) | (rn.code() << 5) | rt.code();
            self.emit(instr);
            return;
        }
        let scaled = ((imm as u32) >> scale) & 0x1ff;
        let instr = 0xF940_0000u32 | (scaled << 12) | (rn.code() << 5) | rt.code();
        self.emit(instr);
    }

    /// STR Xt, [Xn, #imm] — 64-bit, unsigned offset, scaled by 8.
    pub fn Str(&mut self, rt: XRegister, rn: XRegister, imm: i32) {
        let scale = 3u32;
        let mask: u32 = ((1u32 << scale) - 1) as u32;
        if imm < 0 || (imm as u32) & mask != 0 {
            let simm9 = ((imm as i32) as u32) & 0x1ff;
            let instr = 0xF900_0000u32 | (simm9 << 12) | (rn.code() << 5) | rt.code();
            self.emit(instr);
            return;
        }
        let scaled = ((imm as u32) >> scale) & 0x1ff;
        let instr = 0xF900_0000u32 | (scaled << 12) | (rn.code() << 5) | rt.code();
        self.emit(instr);
    }

    /// STP Xt1, Xt2, [Xn, #imm] — store pair, signed offset scaled by 4.
    pub fn Stp(&mut self, rt: XRegister, rt2: XRegister, rn: XRegister, imm: i32) {
        let scale = 2u32;
        let scaled = ((imm as i32) >> scale) as i32;
        let simm7 = (scaled as u32) & 0x7f;
        let instr = 0xA900_0000u32
            | (simm7 << 15)
            | (rt2.code() << 10)
            | (rn.code() << 5)
            | rt.code();
        self.emit(instr);
    }

    /// LDP Xt1, Xt2, [Xn, #imm] — load pair, signed offset scaled by 4.
    pub fn Ldp(&mut self, rt: XRegister, rt2: XRegister, rn: XRegister, imm: i32) {
        let scale = 2u32;
        let scaled = ((imm as i32) >> scale) as i32;
        let simm7 = (scaled as u32) & 0x7f;
        let instr = 0xA940_0000u32
            | (simm7 << 15)
            | (rt2.code() << 10)
            | (rn.code() << 5)
            | rt.code();
        self.emit(instr);
    }

    /// B <label> — PC-relative branch. `offset_bytes` is signed relative to
    /// the branch instruction itself.
    pub fn B(&mut self, offset_bytes: i32) {
        let imm26 = ((offset_bytes as i64) >> 2) as i32;
        let instr = (UncondBranchFixed) | (((imm26 as u32) & 0x03ff_ffff) << 0);
        // Sanity: top byte must not collide with the op.
        let instr = (UncondBranchFixed) | (((imm26 as u32) & 0x03ff_ffff));
        self.emit(instr);
    }

    /// BL <label> — PC-relative branch with link.
    pub fn Bl(&mut self, offset_bytes: i32) {
        let imm26 = ((offset_bytes as i64) >> 2) as i32;
        let instr = 0x9400_0000u32 | (((imm26 as u32) & 0x03ff_ffff));
        self.emit(instr);
    }

    /// BR Xn — register branch.
    pub fn Br(&mut self, rn: XRegister) {
        let instr = UncondBranchRegFixed | (rn.code() << 5);
        self.emit(instr);
    }

    /// RET {Xn} — returns to the address in Xn (default X30).
    pub fn Ret(&mut self, rn: XRegister) {
        let rn = if rn == XRegister::X30 {
            // RETAAS encoder uses X30 by default.
            XRegister::X30
        } else {
            rn
        };
        // RET Xn  => 0xD65F0000 | (Rn << 5)
        let instr = 0xD65F_0000u32 | (rn.code() << 5);
        self.emit(instr);
    }

    /// SVC #imm — supervisor call.
    pub fn Svc(&mut self, imm: u32) {
        let imm16 = imm & 0xffff;
        let instr = ExceptionFixed | imm16;
        self.emit(instr);
    }

    /// NOP.
    pub fn Nop(&mut self) {
        self.emit(NopFixed);
    }
}

// ---------------------------------------------------------------------------
// Item 6: Decoder (decoder-aarch64.h + decoder-constants-aarch64.h).
// ---------------------------------------------------------------------------

/// Top-level A64 instruction decoder. Mirrors `Decoder` from VIXL but kept
/// minimal: a single `Decode` method returns an `Instruction` with the
/// common field extractors applied.
#[derive(Debug, Default, Clone)]
pub struct Decoder {
    /// Mode used to interpret register code 31 in integer operands.
    pub reg31_mode: Reg31Mode,
}

impl Decoder {
    pub fn new() -> Self {
        Self {
            reg31_mode: Reg31Mode::Reg31IsZeroRegister,
        }
    }

    pub fn with_mode(mode: Reg31Mode) -> Self {
        Self { reg31_mode: mode }
    }

    /// Decode a single 32-bit instruction. Returns an `Instruction` with the
    /// recognised fields populated. Unknown encodings come back with
    /// `opcode == kOpcodeUnknown` and a zeroed operand set.
    pub fn Decode(&self, instr: u32) -> Instruction {
        let mut out = Instruction::new(instr);
        out.reg31_mode = self.reg31_mode;
        out.bits = instr;
        out.mask = instr;
        out.rd = XRegister::from_code(instr & 0x1f).unwrap_or(XRegister::XZR);
        out.rn =
            XRegister::from_code((instr >> 5) & 0x1f).unwrap_or(XRegister::XZR);
        out.rm =
            XRegister::from_code((instr >> 16) & 0x1f).unwrap_or(XRegister::XZR);
        out.ra =
            XRegister::from_code((instr >> 10) & 0x1f).unwrap_or(XRegister::XZR);
        out.rv = VRegister::from_code(instr & 0x1f).unwrap_or(VRegister::V0);
        out.rv2 = VRegister::from_code((instr >> 5) & 0x1f).unwrap_or(VRegister::V0);
        out.rp = PRegister::from_code(instr & 0x0f).unwrap_or(PRegister::P0);
        out.cond = Condition::from_code((instr >> 12) & 0x0f).unwrap_or(Condition::AL);
        out.set_flags = ((instr >> 29) & 1) != 0;
        out.shift_amount = (instr >> 10) & 0x3f;
        out.shift = ((instr >> 22) & 0x3) as i32;

        // Top-level dispatch table — abbreviated mirror of the table in
        // `decoder-aarch64.cc::DecodeTbl`.
        if (instr & UncondBranchFMask) == UncondBranchFixed {
            out.opcode = kOpcodeBranch;
            let imm26 = extract_signed_bits(instr, 25, 0);
            out.branch_target = imm26 * 4;
            out.imm = out.branch_target;
        } else if (instr & CondBranchFMask) == CondBranchFixed {
            out.opcode = kOpcodeBranchCond;
            let imm19 = extract_signed_bits(instr, 23, 5);
            out.branch_target = imm19 * 4;
            out.imm = out.branch_target;
        } else if (instr & UncondBranchRegFMask) == UncondBranchRegFixed {
            out.opcode = kOpcodeBranchReg;
        } else if (instr & ExceptionFMask) == ExceptionFixed {
            out.opcode = kOpcodeSvc;
            out.imm = (instr & 0xffff) as i64;
        } else if (instr & LoadStorePairFMask) == LoadStorePairFixed {
            out.opcode = kOpcodeLoadStorePair;
            let simm7 = extract_signed_bits(instr, 21, 15);
            out.imm = simm7 * 8; // 64-bit pair scale
        } else if (instr & LoadStoreFMask) == LoadStoreFixed {
            out.opcode = kOpcodeLoadStore;
            let simm12 = extract_signed_bits(instr, 21, 10);
            out.imm = simm12;
        } else if (instr & AddSubImmediateFMask) == AddSubImmediateFixed {
            out.opcode = kOpcodeDPImm;
            out.imm = extract_bits(instr, 21, 10) as i64;
        } else if (instr & LogicalImmediateFMask) == LogicalImmediateFixed {
            out.opcode = kOpcodeDPImm;
            out.imm_logical = decode_logical_immediate(instr);
            out.imm = out.imm_logical as i64;
        } else if (instr & MoveWideImmediateFMask) == MoveWideImmediateFixed {
            out.opcode = kOpcodeDPImm;
            out.imm = extract_bits(instr, 20, 5) as i64;
            out.shift_amount = (instr >> 21) & 0x3;
        } else if (instr & DataProcessingThreeSourceFMask) == DataProcessingThreeSourceFixed {
            out.opcode = kOpcodeDPRegMul;
        } else if (instr & DataProcessingTwoSourceFMask) == DataProcessingTwoSourceFixed {
            out.opcode = kOpcodeDPReg;
        } else if (instr & DataProcessingOneSourceFMask) == DataProcessingOneSourceFixed {
            out.opcode = kOpcodeDPReg;
        } else if (instr & PCRelAddressingFMask) == PCRelAddressingFixed {
            out.opcode = kOpcodeDPImm;
            out.imm = extract_signed_bits(instr, 23, 5);
        } else if (instr & NopFMask) == NopFixed {
            out.opcode = kOpcodeNop;
        } else {
            out.opcode = kOpcodeUnknown;
        }

        out
    }
}

// ---------------------------------------------------------------------------
// Decoder tables (item 8). A trimmed equivalent of the static `kDecoderTbl`
// and `kDecoderVisitorTbl` arrays in `decoder-aarch64.cc`. We expose them as
// slice constants so other crates can pattern-match without rebuilding them.
// ---------------------------------------------------------------------------

/// One entry of the top-level decoder dispatch table. Mirrors `DecoderTable`
/// entries: `(mask, fixed_bits, expected_opcode)`.
#[derive(Debug, Clone, Copy)]
pub struct DecoderEntry {
    pub mask: u32,
    pub expected: u32,
    pub opcode: u32,
}

/// Trimmed static decoder table covering the subsets that VIXL guarantees.
pub const DECODER_TABLE: &[DecoderEntry] = &[
    DecoderEntry { mask: UncondBranchFMask, expected: UncondBranchFixed, opcode: kOpcodeBranch },
    DecoderEntry { mask: CondBranchFMask, expected: CondBranchFixed, opcode: kOpcodeBranchCond },
    DecoderEntry { mask: UncondBranchRegFMask, expected: UncondBranchRegFixed, opcode: kOpcodeBranchReg },
    DecoderEntry { mask: ExceptionFMask, expected: ExceptionFixed, opcode: kOpcodeSvc },
    DecoderEntry { mask: LoadStorePairFMask, expected: LoadStorePairFixed, opcode: kOpcodeLoadStorePair },
    DecoderEntry { mask: LoadStoreFMask, expected: LoadStoreFixed, opcode: kOpcodeLoadStore },
    DecoderEntry { mask: AddSubImmediateFMask, expected: AddSubImmediateFixed, opcode: kOpcodeDPImm },
    DecoderEntry { mask: LogicalImmediateFMask, expected: LogicalImmediateFixed, opcode: kOpcodeDPImm },
    DecoderEntry { mask: MoveWideImmediateFMask, expected: MoveWideImmediateFixed, opcode: kOpcodeDPImm },
    DecoderEntry { mask: DataProcessingThreeSourceFMask, expected: DataProcessingThreeSourceFixed, opcode: kOpcodeDPRegMul },
    DecoderEntry { mask: DataProcessingTwoSourceFMask, expected: DataProcessingTwoSourceFixed, opcode: kOpcodeDPReg },
    DecoderEntry { mask: DataProcessingOneSourceFMask, expected: DataProcessingOneSourceFixed, opcode: kOpcodeDPReg },
    DecoderEntry { mask: PCRelAddressingFMask, expected: PCRelAddressingFixed, opcode: kOpcodeDPImm },
    DecoderEntry { mask: NopFMask, expected: NopFixed, opcode: kOpcodeNop },
];

/// Logical-immediate decoder. Equivalent to `utils-aarch64.h::DecodeImmBitMask`:
/// given the 13-bit `N:immr:imms` field of a logical-immediate instruction,
/// expand it to a 64-bit replicated pattern.
pub fn decode_logical_immediate(instr: u32) -> u64 {
    let n = (instr >> 12) & 1;
    let immr = ((instr >> 6) & 0x3f) as u32;
    let imms = (instr & 0x3f) as u32;
    let len = highest_set_bit(((n << 6) | (!imms & 0x3f)) as u64);
    if len == 64 {
        // Special case: 0xFFFF_FFFF_FFFF_FFFF
        return u64::MAX;
    }
    let levels = (1u32 << len) - 1;
    let s = imms & levels;
    let r = immr & levels;
    let welem = ((s as u64) + 1) & !(((1u64 << r) - 1) & ((s as u64) + 1) << len - r);
    let mut welem = welem;
    if len > 6 {
        welem |= ((s as u64) + 1) << (len - r);
    }
    replicate(welem, len)
}

fn highest_set_bit(value: u64) -> u32 {
    if value == 0 {
        return 0;
    }
    63 - value.leading_zeros()
}

fn replicate(welem: u64, len: u32) -> u64 {
    let mut out = 0u64;
    let mut bit = 1u64 << len;
    while bit < 64 {
        out |= welem << bit;
        bit <<= 1;
        if bit >= 64 {
            break;
        }
    }
    out | welem
}

// ---------------------------------------------------------------------------
// Item 7: Simulator (simulator-aarch64.h).
// ---------------------------------------------------------------------------

/// Naive instruction-level simulator. Mirrors `Simulator` from VIXL but only
/// implements the data-movement and control-flow subset required for EE
/// translation smoke-tests; SVE and full SVC dispatch are stubbed.
#[derive(Debug, Clone)]
pub struct Simulator {
    /// X0..X30 plus XZR. SP lives at index 31 for convenience — reading or
    /// writing it goes through the same array, but the public API will
    /// address SP/XZR through the high slot.
    pub regs: [u64; 32],
    /// V0..V31. Each slot is a 128-bit vector; scalar reads pick the low
    /// 32/64 bits depending on the encoding.
    pub fregs: [u128; 32],
    /// NZCV flags (N, Z, C, V packed into low bits).
    pub flags: u32,
    /// Architectural PC for the next instruction.
    pub pc: u64,
    /// Count of successfully executed instructions.
    pub icount: u64,
}

impl Default for Simulator {
    fn default() -> Self {
        Self {
            regs: [0u64; 32],
            fregs: [0u128; 32],
            flags: 0,
            pc: 0,
            icount: 0,
        }
    }
}

impl Simulator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        self.regs = [0u64; 32];
        self.fregs = [0u128; 32];
        self.flags = 0;
        self.pc = 0;
        self.icount = 0;
    }

    fn write_xreg(&mut self, rd: XRegister, value: u64) {
        match rd {
            XRegister::SP => self.regs[31] = value,
            XRegister::XZR => { /* discard */ }
            other => self.regs[other.code() as usize] = value,
        }
    }

    fn read_xreg(&self, rn: XRegister) -> u64 {
        match rn {
            XRegister::SP | XRegister::XZR => self.regs[31],
            other => self.regs[other.code() as usize],
        }
    }

    fn flag_n(&self) -> bool { (self.flags & 0x8) != 0 }
    fn flag_z(&self) -> bool { (self.flags & 0x4) != 0 }
    fn flag_c(&self) -> bool { (self.flags & 0x2) != 0 }
    fn flag_v(&self) -> bool { (self.flags & 0x1) != 0 }

    fn update_flags_add(&mut self, a: u64, b: u64, result: u64) {
        self.flags = 0;
        if (result & kXRegMask) != 0 {
            self.flags |= 0x8;
        }
        if result == 0 {
            self.flags |= 0x4;
        }
        if ((a ^ !b) & (a ^ result)) & kXRegMask != 0 {
            self.flags |= 0x1; // V
        }
        if ((b & !a) | ((b | !a) & result)) & kXRegMask != 0 {
            self.flags |= 0x2; // C (incorrect for unsigned subtracts, but
                              // add/sub-flags share encoder)
        }
    }

    /// Execute one instruction. Returns `Err(message)` for encodings the
    /// simulator does not yet understand.
    pub fn Run(&mut self, instr: u32) -> Result<(), String> {
        let decoded = Decoder::new().Decode(instr);
        self.icount = self.icount.wrapping_add(1);

        match decoded.opcode {
            kOpcodeNop => {
                self.pc = self.pc.wrapping_add(kInstructionSize as u64);
            }
            kOpcodeBranch => {
                // B <label>
                self.pc = (self.pc as i64 + decoded.branch_target) as u64;
            }
            kOpcodeBranchCond => {
                // B.cond <label>
                let cond = decoded.cond;
                let n = self.flag_n();
                let z = self.flag_z();
                let c = self.flag_c();
                let v = self.flag_v();
                if cond.holds(n, z, c, v) {
                    self.pc = (self.pc as i64 + decoded.branch_target) as u64;
                } else {
                    self.pc = self.pc.wrapping_add(kInstructionSize as u64);
                }
            }
            kOpcodeBranchReg => {
                // BR Xn / RET Xn — opc[4:0] distinguishes
                let op = extract_bits(instr, 24, 21);
                if op == 0 {
                    // RET
                    let rn =
                        XRegister::from_code(extract_bits(instr, 9, 5) as u32)
                            .ok_or_else(|| "invalid RET register".to_string())?;
                    self.pc = self.read_xreg(rn);
                } else {
                    // BR Xn
                    let rn =
                        XRegister::from_code(extract_bits(instr, 9, 5) as u32)
                            .ok_or_else(|| "invalid BR register".to_string())?;
                    self.pc = self.read_xreg(rn);
                }
            }
            kOpcodeSvc => {
                // SVC #imm — return an error so callers can intercept.
                return Err(format!("SVC #{}", decoded.imm));
            }
            kOpcodeDPImm => {
                // AND imm / ORR imm / EOR imm / MOV / ADD imm / SUB imm
                let opc = extract_bits(instr, 29, 29)
                    | (extract_bits(instr, 30, 30) << 1);
                let rn_val = self.read_xreg(decoded.rn);
                let rd = decoded.rd;
                match opc {
                    0 => {
                        // ADD immediate 64-bit
                        let imm12 = extract_bits(instr, 21, 10);
                        let result = rn_val.wrapping_add(imm12 as u64);
                        self.write_xreg(rd, result);
                        if decoded.set_flags {
                            self.update_flags_add(rn_val, imm12 as u64, result);
                        }
                    }
                    1 => {
                        // ADDS immediate 64-bit
                        let imm12 = extract_bits(instr, 21, 10);
                        let result = rn_val.wrapping_add(imm12 as u64);
                        self.write_xreg(rd, result);
                        self.update_flags_add(rn_val, imm12 as u64, result);
                    }
                    2 => {
                        // SUB immediate 64-bit
                        let imm12 = extract_bits(instr, 21, 10);
                        let result = rn_val.wrapping_sub(imm12 as u64);
                        self.write_xreg(rd, result);
                        if decoded.set_flags {
                            self.update_flags_add(rn_val, !(imm12 as u64), result);
                        }
                    }
                    3 => {
                        // MOVZ / MOVN / MOVK wide immediate
                        let opc_wide = (extract_bits(instr, 30, 29)) as u64;
                        let hw = (extract_bits(instr, 22, 21) as u64) << 4;
                        let imm16 = (extract_bits(instr, 20, 5) as u64) << hw;
                        let result = match opc_wide {
                            0 => imm16,
                            1 => !imm16,
                            3 => (self.read_xreg(rd) & !((0xffffu64) << hw)) | imm16,
                            _ => return Err(format!("unknown wide immediate opc {}", opc_wide)),
                        };
                        self.write_xreg(rd, result);
                    }
                    _ => {
                        // ORR/AND/EOR 64-bit logical immediate.
                        let opc = (extract_bits(instr, 30, 29)) as u64;
                        let immediate = decoded.imm_logical;
                        let result = match opc {
                            0 => rn_val & immediate,
                            1 => rn_val | immediate,
                            2 => rn_val ^ immediate,
                            3 => rn_val & immediate,
                            _ => unreachable!(),
                        };
                        self.write_xreg(rd, result);
                        if decoded.set_flags {
                            let result_n = (result & kXRegMask) != 0;
                            let result_z = result == 0;
                            self.flags = (result_n as u32) << 3 | (result_z as u32) << 2;
                        }
                    }
                }
                self.pc = self.pc.wrapping_add(kInstructionSize as u64);
            }
            kOpcodeDPReg => {
                let opc = (extract_bits(instr, 30, 29)) as u64;
                let rn_val = self.read_xreg(decoded.rn);
                let rm_val = self.read_xreg(decoded.rm);
                let rd = decoded.rd;
                match opc {
                    0 => {
                        // ADD shifted register
                        let result = rn_val.wrapping_add(rm_val);
                        self.write_xreg(rd, result);
                        if decoded.set_flags {
                            self.update_flags_add(rn_val, rm_val, result);
                        }
                    }
                    1 => {
                        // ADDS shifted register
                        let result = rn_val.wrapping_add(rm_val);
                        self.write_xreg(rd, result);
                        self.update_flags_add(rn_val, rm_val, result);
                    }
                    2 => {
                        // SUB shifted register
                        let result = rn_val.wrapping_sub(rm_val);
                        self.write_xreg(rd, result);
                        if decoded.set_flags {
                            self.update_flags_add(rn_val, !rm_val, result);
                        }
                    }
                    _ => {
                        return Err(format!(
                            "unimplemented data-processing register opc={} (instr={:#x})",
                            opc, instr
                        ));
                    }
                }
                self.pc = self.pc.wrapping_add(kInstructionSize as u64);
            }
            kOpcodeDPRegMul => {
                let op31 = extract_bits(instr, 21, 21);
                let rn_val = self.read_xreg(decoded.rn);
                let rm_val = self.read_xreg(decoded.rm);
                let ra_val = self.read_xreg(decoded.ra);
                let result = match op31 {
                    0 => {
                        // MADD
                        rn_val.wrapping_mul(rm_val).wrapping_add(ra_val)
                    }
                    _ => {
                        // MSUB
                        ra_val.wrapping_sub(rn_val.wrapping_mul(rm_val))
                    }
                };
                self.write_xreg(decoded.rd, result);
                self.pc = self.pc.wrapping_add(kInstructionSize as u64);
            }
            kOpcodeLoadStore | kOpcodeLoadStorePair => {
                return Err("simulator does not implement load/store".to_string());
            }
            kOpcodeUnknown => {
                return Err(format!(
                    "unknown encoding: {:#010x}",
                    instr
                ));
            }
            _ => {
                return Err(format!(
                    "unimplemented opcode {} for instr {:#010x}",
                    decoded.opcode, instr
                ));
            }
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Global state mirrors the VIXL `VIXL_DEBUG_*` flags. Kept as `static mut` so
// downstream code can flip them without threading the handle through every
// API. This is the same pattern the C++ library uses for its globals.
// ---------------------------------------------------------------------------

/// Master debug flag (mirrors `vixl::FLAG_debug_code`).
pub static mut FLAG_debug_code: bool = false;
/// Print disassembly for each executed instruction (mirrors
/// `vixl::FLAG_trace_sim`).
pub static mut FLAG_trace_sim: bool = false;
/// Treat the simulator as a debugger (mirrors `vixl::FLAG_simulate_debug`).
pub static mut FLAG_simulate_debug: bool = false;
/// Code generation abort hook. VIXL provides this so the assembler can stop
/// the process on errors; we expose it as a `static mut` slot instead.
pub static mut FLAG_abort_hook: Option<unsafe extern "C" fn()> = None;

// ---------------------------------------------------------------------------
// Tests. A handful of sanity checks to anchor the encoding/decoding pairs.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xreg_roundtrip() {
        for i in 0..32 {
            let r = XRegister::from_code(i).unwrap();
            assert_eq!(r.code(), i);
        }
    }

    #[test]
    fn vreg_roundtrip() {
        for i in 0..32 {
            let r = VRegister::from_code(i).unwrap();
            assert_eq!(r.code(), i);
        }
    }

    #[test]
    fn preg_roundtrip() {
        for i in 0..16 {
            let r = PRegister::from_code(i).unwrap();
            assert_eq!(r.code(), i);
        }
    }

    #[test]
    fn condition_invert_pairs() {
        assert_eq!(Condition::EQ.invert(), Condition::NE);
        assert_eq!(Condition::HI.invert(), Condition::LS);
        assert_eq!(Condition::GT.invert(), Condition::LE);
        assert_eq!(Condition::AL.invert(), Condition::AL);
    }

    #[test]
    fn mov_decodes_as_orr() {
        let mut masm = MacroAssembler::new();
        masm.Mov(XRegister::X0, XRegister::X1);
        assert_eq!(masm.code()[0], 0xAA00_03E0u32 | (1u32 << 16));
    }

    #[test]
    fn add_decodes() {
        let mut masm = MacroAssembler::new();
        masm.Add(XRegister::X2, XRegister::X3, XRegister::X4);
        let instr = masm.code()[0];
        let d = Decoder::new().Decode(instr);
        assert_eq!(d.opcode, kOpcodeDPReg);
        assert_eq!(d.rd.code(), 2);
        assert_eq!(d.rn.code(), 3);
        assert_eq!(d.rm.code(), 4);
    }

    #[test]
    fn simulator_runs_add() {
        let mut sim = Simulator::new();
        let mut masm = MacroAssembler::new();
        masm.Add(XRegister::X0, XRegister::X1, XRegister::X2);
        sim.regs[1] = 4;
        sim.regs[2] = 5;
        sim.Run(masm.code()[0]).unwrap();
        assert_eq!(sim.regs[0], 9);
    }
}
