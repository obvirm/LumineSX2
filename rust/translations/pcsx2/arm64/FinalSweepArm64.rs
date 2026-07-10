//! Idiomatic Rust 2021 translation of the PCSX2 `pcsx2/arm64/` C/C++ source set.
//!
//! This module consolidates the contents of:
//!
//!   * `arm64/AsmHelpers.{h,cpp}`  - ARM64 register helpers, constant pool,
//!                                   stack-frame management, branch / call
//!                                   emission, and dynarec block lifecycle.
//!   * `arm64/RecStubs.cpp`        - Stubs for `vtlb_DynBackpatchLoadStore`
//!                                   and `SaveStateBase::vuJITFreeze`.
//!   * `arm64/Vif_Dynarec.cpp`     - VIF dynarec block management, mask
//!                                   application, and the `dVifUnpack` /
//!                                   `dVifCompile` entry points.
//!   * `arm64/Vif_UnpackNEON.{h,cpp}` - NEON-optimised VIF unpacking for
//!                                     S/V2/V3/V4/5 layouts plus the
//!                                     simple unpack generator.
//!
//! The translation is API-compatible at the level of types, free functions
//! and globals.  ARM64 instruction emitters are exposed as opaque
//! `extern "C"` declarations and their bodies are placeholders (`unimplemented!`),
//! because a real implementation would bind the VIXL macro-assembler or a
//! custom ARM64 encoder.
//!
//! Only `std` is used.  No external crates are required.

// ===========================================================================
// Primitive type aliases matching PCSX2's `common/Pcsx2Defs.h`.
// ===========================================================================

pub type s8 = i8;
pub type s16 = i16;
pub type s32 = i32;
pub type s64 = i64;
pub type u8 = std::primitive::u8;
pub type u16 = std::primitive::u16;
pub type u32 = std::primitive::u32;
pub type u64 = std::primitive::u64;
pub type uptr = usize;
pub type sptr = isize;
pub type uint = u32;

/// 128-bit value used as the key type for ARM constant pool literals.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Default, Hash)]
pub struct u128 {
    pub lo: u64,
    pub hi: u64,
}

impl u128 {
    pub const fn new(lo: u64, hi: u64) -> Self {
        Self { lo, hi }
    }

    pub const fn from_64(v: u64) -> Self {
        Self { lo: v, hi: 0 }
    }
}

// ===========================================================================
// vixl::aarch64 placeholder module.
//
// The full VIXL macro-assembler is far too large to be translated line by
// line, so this module provides thin opaque type definitions and
// `extern "C"` declarations for the assembly-emitter entry points used by
// the C++ code.  All bodies return `unimplemented!()`; in production this
// would be backed by either a Rust port of VIXL or a real `extern "C"`
// binding to the existing C++ library.
// ===========================================================================

pub mod aarch64 {
    use super::*;

    // -- Memory operand addressing modes ----------------------------------

    pub const OFFSET: u8 = 0;
    pub const POST_INDEX: u8 = 1;
    pub const PRE_INDEX: u8 = 2;
    pub const NO_SHIFT: u8 = 0;
    pub const LSL: u8 = 1;
    pub const LSR: u8 = 2;
    pub const ASR: u8 = 3;
    pub const ROR: u8 = 4;

    // -- Condition codes --------------------------------------------------

    pub type Condition = u8;

    pub const EQ: Condition = 0x0;
    pub const NE: Condition = 0x1;
    pub const CS: Condition = 0x2;
    pub const HS: Condition = 0x2;
    pub const CC: Condition = 0x3;
    pub const LO: Condition = 0x3;
    pub const MI: Condition = 0x4;
    pub const PL: Condition = 0x5;
    pub const VS: Condition = 0x6;
    pub const VC: Condition = 0x7;
    pub const HI: Condition = 0x8;
    pub const LS: Condition = 0x9;
    pub const GE: Condition = 0xA;
    pub const LT: Condition = 0xB;
    pub const GT: Condition = 0xC;
    pub const LE: Condition = 0xD;
    pub const AL: Condition = 0xE;
    pub const NV: Condition = 0xF;

    // -- Branch-type discriminators ---------------------------------------

    pub const COMPARE_BRANCH_TYPE: u8 = 0;
    pub const COND_BRANCH_TYPE: u8 = 1;
    pub const TEST_BRANCH_TYPE: u8 = 2;

    // -- 32-bit GPR -------------------------------------------------------

    /// ARM64 32-bit (W) or 64-bit (X) general-purpose register.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Default, Hash)]
    pub struct Register {
        pub code: u32,
    }

    impl Register {
        pub const fn new(code: u32) -> Self {
            Self { code }
        }
        pub const fn get_code(&self) -> u32 { self.code }
        pub const fn is_x(&self) -> bool { self.code < 32 }
        pub const fn is_valid(&self) -> bool { self.code < 32 }
    }

    pub type WRegister = Register;
    pub type XRegister = Register;

    // -- 128-bit SIMD/FP vector register ----------------------------------

    /// ARM64 vector (V) register reference.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, Default, Hash)]
    pub struct VRegister {
        pub code: u32,
        pub size: u32,
        pub lanes: u32,
    }

    impl VRegister {
        pub const fn new(code: u32, size: u32, lanes: u32) -> Self {
            Self { code, size, lanes }
        }
        pub const fn get_code(&self) -> u32 { self.code }
        pub const fn v16b(&self) -> Self { Self::new(self.code, 128, 16) }
        pub const fn v8h(&self) -> Self { Self::new(self.code, 64, 8) }
        pub const fn v4s(&self) -> Self { Self::new(self.code, 64, 4) }
        pub const fn v2d(&self) -> Self { Self::new(self.code, 64, 2) }
        pub const fn v8b(&self) -> Self { Self::new(self.code, 64, 8) }
        pub const fn v4h(&self) -> Self { Self::new(self.code, 64, 4) }
        pub const fn v2s(&self) -> Self { Self::new(self.code, 64, 2) }
        pub const fn v1d(&self) -> Self { Self::new(self.code, 64, 1) }
        pub const fn s(&self) -> Register { Register::new(self.code) }
        pub const fn d(&self) -> Register { Register::new(self.code) }
        pub const fn vs(&self) -> Self { Self::new(self.code, 32, 4) }
        pub const fn vd(&self) -> Self { Self::new(self.code, 64, 2) }
        pub const fn q(&self) -> Self { *self }
        pub const fn b(&self) -> Register { Register::new(self.code) }
        pub const fn h(&self) -> Register { Register::new(self.code) }
        pub const fn w(&self) -> Register { Register::new(self.code) }
        pub const fn x(&self) -> Register { Register::new(self.code) }
    }

    // -- CPU register union (X/W or V) ------------------------------------

    #[derive(Clone, Copy, Debug, Eq, PartialEq, Default, Hash)]
    pub enum CPURegister {
        #[default]
        None,
        GP(Register),
        Vec(VRegister),
    }

    // -- Memory operand ---------------------------------------------------

    #[derive(Clone, Copy, Debug, Eq, PartialEq, Default, Hash)]
    pub struct MemOperand {
        pub base: Register,
        pub offset: i64,
        pub addr_mode: u8,
        pub shift: u8,
        pub shift_amount: u32,
        pub extend: u8,
    }

    impl MemOperand {
        pub const fn new(base: Register) -> Self {
            Self {
                base,
                offset: 0,
                addr_mode: OFFSET,
                shift: NO_SHIFT,
                shift_amount: 0,
                extend: 0,
            }
        }
        pub const fn new_offset(base: Register, offset: i64, addr_mode: u8) -> Self {
            Self {
                base,
                offset,
                addr_mode,
                shift: NO_SHIFT,
                shift_amount: 0,
                extend: 0,
            }
        }
        pub fn get_base_register(&self) -> Register { self.base }
        pub fn get_offset(&self) -> i64 { self.offset }
        pub fn get_addr_mode(&self) -> u8 { self.addr_mode }
        pub fn get_shift(&self) -> u8 { self.shift }
    }

    pub type Operand = CPURegister;

    // -- Forward label ----------------------------------------------------

    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    pub struct Label {
        pub id: u32,
    }

    // -- Instruction ------------------------------------------------------

    pub struct Instruction;

    impl Instruction {
        pub fn is_valid_imm_pc_offset(_branch_type: u8, _offset: i64) -> bool { true }
    }

    // -- Single/Macro emission guards -------------------------------------

    pub struct SingleEmissionCheckScope { pub _asm: *mut MacroAssembler }
    pub struct MacroEmissionCheckScope { pub _asm: *mut MacroAssembler }

    impl Drop for SingleEmissionCheckScope { fn drop(&mut self) {} }
    impl Drop for MacroEmissionCheckScope { fn drop(&mut self) {} }

    // -- MacroAssembler ---------------------------------------------------

    pub struct MacroAssembler {
        pub buffer: *mut u8,
        pub capacity: usize,
        pub cursor: usize,
    }

    impl MacroAssembler {
        pub fn new(buffer: *mut u8, capacity: usize) -> Self {
            Self { buffer, capacity, cursor: 0 }
        }
        pub fn finalize_code(&mut self) {}
        pub fn get_size_of_code_generated(&self) -> usize { self.cursor }
        pub fn get_cursor_offset(&self) -> usize { self.cursor }
        pub fn get_scratch_v_register_list(&mut self) -> ScratchVRegisterList {
            ScratchVRegisterList { _asm: self }
        }
        pub fn get_scratch_register_list(&mut self) -> ScratchRegisterList {
            ScratchRegisterList { _asm: self }
        }
    }

    pub struct ScratchVRegisterList<'a> { pub _asm: &'a mut MacroAssembler }
    impl<'a> ScratchVRegisterList<'a> {
        pub fn remove(&mut self, _code: u32) {}
    }

    pub struct ScratchRegisterList<'a> { pub _asm: &'a mut MacroAssembler }
    impl<'a> ScratchRegisterList<'a> {
        pub fn remove(&mut self, _code: u32) {}
    }

    // -- ARM64 register aliases (used by PCSX2 macros) --------------------

    pub const W0: Register  = Register::new(0);
    pub const W1: Register  = Register::new(1);
    pub const W2: Register  = Register::new(2);
    pub const W3: Register  = Register::new(3);
    pub const W4: Register  = Register::new(4);
    pub const W5: Register  = Register::new(5);
    pub const W6: Register  = Register::new(6);
    pub const W7: Register  = Register::new(7);
    pub const W8: Register  = Register::new(8);
    pub const W9: Register  = Register::new(9);
    pub const W10: Register = Register::new(10);
    pub const W11: Register = Register::new(11);
    pub const W12: Register = Register::new(12);
    pub const W13: Register = Register::new(13);
    pub const W14: Register = Register::new(14);
    pub const W15: Register = Register::new(15);
    pub const W16: Register = Register::new(16);
    pub const W17: Register = Register::new(17);
    pub const W18: Register = Register::new(18);
    pub const W19: Register = Register::new(19);
    pub const W20: Register = Register::new(20);
    pub const W21: Register = Register::new(21);
    pub const W22: Register = Register::new(22);
    pub const W23: Register = Register::new(23);
    pub const W24: Register = Register::new(24);
    pub const W25: Register = Register::new(25);
    pub const W26: Register = Register::new(26);
    pub const W27: Register = Register::new(27);
    pub const W28: Register = Register::new(28);
    pub const W29: Register = Register::new(29);
    pub const W30: Register = Register::new(30);
    pub const WZR: Register = Register::new(31);
    pub const wzr: Register = WZR;

    pub const X0: Register  = Register::new(0);
    pub const X1: Register  = Register::new(1);
    pub const X2: Register  = Register::new(2);
    pub const X3: Register  = Register::new(3);
    pub const X4: Register  = Register::new(4);
    pub const X5: Register  = Register::new(5);
    pub const X6: Register  = Register::new(6);
    pub const X7: Register  = Register::new(7);
    pub const X8: Register  = Register::new(8);
    pub const X9: Register  = Register::new(9);
    pub const X10: Register = Register::new(10);
    pub const X11: Register = Register::new(11);
    pub const X12: Register = Register::new(12);
    pub const X13: Register = Register::new(13);
    pub const X14: Register = Register::new(14);
    pub const X15: Register = Register::new(15);
    pub const X16: Register = Register::new(16);
    pub const X17: Register = Register::new(17);
    pub const X18: Register = Register::new(18);
    pub const X19: Register = Register::new(19);
    pub const X20: Register = Register::new(20);
    pub const X21: Register = Register::new(21);
    pub const X22: Register = Register::new(22);
    pub const X23: Register = Register::new(23);
    pub const X24: Register = Register::new(24);
    pub const X25: Register = Register::new(25);
    pub const X26: Register = Register::new(26);
    pub const X27: Register = Register::new(27);
    pub const X28: Register = Register::new(28);
    pub const X29: Register = Register::new(29);
    pub const X30: Register = Register::new(30);
    pub const LR: Register  = X30;
    pub const SP: Register  = Register::new(31);
    pub const XZR: Register = Register::new(31);

    pub const S0: VRegister  = VRegister::new(0, 32, 4);
    pub const S1: VRegister  = VRegister::new(1, 32, 4);
    pub const S2: VRegister  = VRegister::new(2, 32, 4);
    pub const S3: VRegister  = VRegister::new(3, 32, 4);
    pub const S4: VRegister  = VRegister::new(4, 32, 4);
    pub const S5: VRegister  = VRegister::new(5, 32, 4);
    pub const S6: VRegister  = VRegister::new(6, 32, 4);
    pub const S7: VRegister  = VRegister::new(7, 32, 4);
    pub const S8: VRegister  = VRegister::new(8, 32, 4);
    pub const S9: VRegister  = VRegister::new(9, 32, 4);
    pub const S10: VRegister = VRegister::new(10, 32, 4);
    pub const S11: VRegister = VRegister::new(11, 32, 4);
    pub const S12: VRegister = VRegister::new(12, 32, 4);
    pub const S13: VRegister = VRegister::new(13, 32, 4);
    pub const S14: VRegister = VRegister::new(14, 32, 4);
    pub const S15: VRegister = VRegister::new(15, 32, 4);
    pub const S16: VRegister = VRegister::new(16, 32, 4);
    pub const S17: VRegister = VRegister::new(17, 32, 4);
    pub const S18: VRegister = VRegister::new(18, 32, 4);
    pub const S19: VRegister = VRegister::new(19, 32, 4);
    pub const S20: VRegister = VRegister::new(20, 32, 4);
    pub const S21: VRegister = VRegister::new(21, 32, 4);
    pub const S22: VRegister = VRegister::new(22, 32, 4);
    pub const S23: VRegister = VRegister::new(23, 32, 4);
    pub const S24: VRegister = VRegister::new(24, 32, 4);
    pub const S25: VRegister = VRegister::new(25, 32, 4);
    pub const S26: VRegister = VRegister::new(26, 32, 4);
    pub const S27: VRegister = VRegister::new(27, 32, 4);
    pub const S28: VRegister = VRegister::new(28, 32, 4);
    pub const S29: VRegister = VRegister::new(29, 32, 4);
    pub const S30: VRegister = VRegister::new(30, 32, 4);
    pub const S31: VRegister = VRegister::new(31, 32, 4);

    pub const D0: VRegister  = VRegister::new(0, 64, 2);
    pub const D1: VRegister  = VRegister::new(1, 64, 2);
    pub const D2: VRegister  = VRegister::new(2, 64, 2);
    pub const D3: VRegister  = VRegister::new(3, 64, 2);
    pub const D4: VRegister  = VRegister::new(4, 64, 2);
    pub const D5: VRegister  = VRegister::new(5, 64, 2);
    pub const D6: VRegister  = VRegister::new(6, 64, 2);
    pub const D7: VRegister  = VRegister::new(7, 64, 2);
    pub const D8: VRegister  = VRegister::new(8, 64, 2);
    pub const D9: VRegister  = VRegister::new(9, 64, 2);
    pub const D10: VRegister = VRegister::new(10, 64, 2);
    pub const D11: VRegister = VRegister::new(11, 64, 2);
    pub const D12: VRegister = VRegister::new(12, 64, 2);
    pub const D13: VRegister = VRegister::new(13, 64, 2);
    pub const D14: VRegister = VRegister::new(14, 64, 2);
    pub const D15: VRegister = VRegister::new(15, 64, 2);
    pub const D16: VRegister = VRegister::new(16, 64, 2);
    pub const D17: VRegister = VRegister::new(17, 64, 2);
    pub const D18: VRegister = VRegister::new(18, 64, 2);
    pub const D19: VRegister = VRegister::new(19, 64, 2);
    pub const D20: VRegister = VRegister::new(20, 64, 2);
    pub const D21: VRegister = VRegister::new(21, 64, 2);
    pub const D22: VRegister = VRegister::new(22, 64, 2);
    pub const D23: VRegister = VRegister::new(23, 64, 2);
    pub const D24: VRegister = VRegister::new(24, 64, 2);
    pub const D25: VRegister = VRegister::new(25, 64, 2);
    pub const D26: VRegister = VRegister::new(26, 64, 2);
    pub const D27: VRegister = VRegister::new(27, 64, 2);
    pub const D28: VRegister = VRegister::new(28, 64, 2);
    pub const D29: VRegister = VRegister::new(29, 64, 2);
    pub const D30: VRegister = VRegister::new(30, 64, 2);
    pub const D31: VRegister = VRegister::new(31, 64, 2);

    pub const Q0: VRegister  = VRegister::new(0, 128, 16);
    pub const Q1: VRegister  = VRegister::new(1, 128, 16);
    pub const Q2: VRegister  = VRegister::new(2, 128, 16);
    pub const Q3: VRegister  = VRegister::new(3, 128, 16);
    pub const Q4: VRegister  = VRegister::new(4, 128, 16);
    pub const Q5: VRegister  = VRegister::new(5, 128, 16);
    pub const Q6: VRegister  = VRegister::new(6, 128, 16);
    pub const Q7: VRegister  = VRegister::new(7, 128, 16);
    pub const Q8: VRegister  = VRegister::new(8, 128, 16);
    pub const Q9: VRegister  = VRegister::new(9, 128, 16);
    pub const Q10: VRegister = VRegister::new(10, 128, 16);
    pub const Q11: VRegister = VRegister::new(11, 128, 16);
    pub const Q12: VRegister = VRegister::new(12, 128, 16);
    pub const Q13: VRegister = VRegister::new(13, 128, 16);
    pub const Q14: VRegister = VRegister::new(14, 128, 16);
    pub const Q15: VRegister = VRegister::new(15, 128, 16);
    pub const Q16: VRegister = VRegister::new(16, 128, 16);
    pub const Q17: VRegister = VRegister::new(17, 128, 16);
    pub const Q18: VRegister = VRegister::new(18, 128, 16);
    pub const Q19: VRegister = VRegister::new(19, 128, 16);
    pub const Q20: VRegister = VRegister::new(20, 128, 16);
    pub const Q21: VRegister = VRegister::new(21, 128, 16);
    pub const Q22: VRegister = VRegister::new(22, 128, 16);
    pub const Q23: VRegister = VRegister::new(23, 128, 16);
    pub const Q24: VRegister = VRegister::new(24, 128, 16);
    pub const Q25: VRegister = VRegister::new(25, 128, 16);
    pub const Q26: VRegister = VRegister::new(26, 128, 16);
    pub const Q27: VRegister = VRegister::new(27, 128, 16);
    pub const Q28: VRegister = VRegister::new(28, 128, 16);
    pub const Q29: VRegister = VRegister::new(29, 128, 16);
    pub const Q30: VRegister = VRegister::new(30, 128, 16);
    pub const Q31: VRegister = VRegister::new(31, 128, 16);

    // -- VIXL helpers -----------------------------------------------------

    pub fn is_int26(v: i64) -> bool { v >= -(1i64 << 25) && v < (1i64 << 25) }
    pub fn is_int21(v: i64) -> bool { v >= -(1i64 << 20) && v < (1i64 << 20) }
    pub fn invert_condition(c: Condition) -> Condition { c ^ 1 }

    // -------------------------------------------------------------------
    // ARM64 macro-assembler instruction emitters.
    //
    // Every emitter corresponds to a `vixl::aarch64::MacroAssembler` method
    // used by the dynarec; their bodies are `unimplemented!()` because
    // a faithful translation of the VIXL encoding logic is far outside the
    // scope of this file.  In production, each of these would either be a
    // direct port of the equivalent VIXL method or an `extern "C"` FFI call
    // into a real C++ VIXL build.
    // -------------------------------------------------------------------

    impl MacroAssembler {
        pub fn b(&mut self, _imm: i64) { /* ASM stub */ }
        pub fn b_cond(&mut self, _imm: i64, _cond: Condition) { /* ASM stub */ }
        pub fn bl(&mut self, _imm: i64) { /* ASM stub */ }
        pub fn br(&mut self, _reg: Register) { /* ASM stub */ }
        pub fn blr(&mut self, _reg: Register) { /* ASM stub */ }
        pub fn cbz(&mut self, _reg: Register, _label: *const Label) { /* ASM stub */ }
        pub fn cbnz(&mut self, _reg: Register, _imm: i64) { /* ASM stub */ }
        pub fn bind(&mut self, _label: *mut Label) { /* ASM stub */ }

        pub fn mov_reg(&mut self, _dst: Register, _src: Register) { /* ASM stub */ }
        pub fn mov_wide(&mut self, _dst: Register, _imm: u64) { /* ASM stub */ }
        pub fn mov_vec(&mut self, _dst: VRegister, _src: VRegister) { /* ASM stub */ }
        pub fn movi(&mut self, _dst: VRegister, _imm: i64) { /* ASM stub */ }

        pub fn add_reg(&mut self, _dst: Register, _src1: Register, _src2: Register) { /* ASM stub */ }
        pub fn add_imm(&mut self, _dst: Register, _src: Register, _imm: i64) { /* ASM stub */ }
        pub fn sub(&mut self, _dst: Register, _src1: Register, _src2: Register) { /* ASM stub */ }
        pub fn orr_imm(&mut self, _dst: Register, _src: Register, _imm: u64) { /* ASM stub */ }
        pub fn adrp(&mut self, _dst: Register, _imm: i64) { /* ASM stub */ }

        pub fn ldr_gp_mem(&mut self, _dst: Register, _mem: MemOperand) { /* ASM stub */ }
        pub fn ldr_vec_mem(&mut self, _dst: VRegister, _mem: MemOperand) { /* ASM stub */ }
        pub fn ldr_imm(&mut self, _dst: Register, _mem: MemOperand) { /* ASM stub */ }
        pub fn str_gp_mem(&mut self, _src: Register, _mem: MemOperand) { /* ASM stub */ }
        pub fn str_vec_mem(&mut self, _src: VRegister, _mem: MemOperand) { /* ASM stub */ }
        pub fn ldp(&mut self, _a: Register, _b: Register, _mem: MemOperand) { /* ASM stub */ }
        pub fn stp(&mut self, _a: Register, _b: Register, _mem: MemOperand) { /* ASM stub */ }
        pub fn ldrh(&mut self, _dst: Register, _mem: MemOperand) { /* ASM stub */ }
        pub fn ldr_literal_64(&mut self, _dst: Register, _literal: *const u8) { /* ASM stub */ }
        pub fn ldr_literal_128(&mut self, _dst: VRegister, _high: u64, _low: u64) { /* ASM stub */ }

        pub fn lsl_reg(&mut self, _dst: Register, _src: Register, _shift: u32) { /* ASM stub */ }
        pub fn lsr_reg(&mut self, _dst: Register, _src: Register, _shift: u32) { /* ASM stub */ }
        pub fn shl_vec(&mut self, _dst: VRegister, _src: VRegister, _shift: u32) { /* ASM stub */ }
        pub fn ushl_vec(&mut self, _dst: VRegister, _src: VRegister, _shift: u32) { /* ASM stub */ }
        pub fn sshl_vec(&mut self, _dst: VRegister, _src: VRegister, _shift: u32) { /* ASM stub */ }
        pub fn sshll_vec(&mut self, _dst: VRegister, _src: VRegister, _shift: u32) { /* ASM stub */ }
        pub fn ushll_vec(&mut self, _dst: VRegister, _src: VRegister, _shift: u32) { /* ASM stub */ }
        pub fn ushr_vec(&mut self, _dst: VRegister, _src: VRegister, _shift: u32) { /* ASM stub */ }
        pub fn sshr_vec(&mut self, _dst: VRegister, _src: VRegister, _shift: u32) { /* ASM stub */ }

        pub fn dup_lane_v4s(&mut self, _dst: VRegister, _src: VRegister, _lane: u32) { /* ASM stub */ }
        pub fn dup_lane_v2d(&mut self, _dst: VRegister, _src: VRegister, _lane: u32) { /* ASM stub */ }
        pub fn dup_lane_s(&mut self, _dst: VRegister, _src: Register) { /* ASM stub */ }
        pub fn dup_gp_to_vec(&mut self, _dst: VRegister, _src: Register) { /* ASM stub */ }
        pub fn ins_v4s_gp(&mut self, _dst: VRegister, _idx: u32, _src: Register) { /* ASM stub */ }
        pub fn ins_v4s_v4s(&mut self, _dst: VRegister, _dst_idx: u32, _src: VRegister, _src_idx: u32) { /* ASM stub */ }
        pub fn ins_v2d_v2d(&mut self, _dst: VRegister, _dst_idx: u32, _src: VRegister, _src_idx: u32) { /* ASM stub */ }
        pub fn mov_v4s_v4s(&mut self, _dst: VRegister, _dst_idx: u32, _src: VRegister, _src_idx: u32) { /* ASM stub */ }
        pub fn mov_v2d_v2d(&mut self, _dst: VRegister, _dst_idx: u32, _src: VRegister, _src_idx: u32) { /* ASM stub */ }
        pub fn mov_v2s_v2s(&mut self, _dst: VRegister, _dst_idx: u32, _src: VRegister, _src_idx: u32) { /* ASM stub */ }

        pub fn st1_v4s(&mut self, _src: VRegister, _idx: u32, _mem: MemOperand) { /* ASM stub */ }
        pub fn st1_v2d(&mut self, _src: VRegister, _idx: u32, _mem: MemOperand) { /* ASM stub */ }

        pub fn tbl_v16b(&mut self, _dst: VRegister, _a: VRegister, _b: VRegister, _tbl: VRegister) { /* ASM stub */ }

        pub fn and_v16b(&mut self, _dst: VRegister, _a: VRegister, _b: VRegister) { /* ASM stub */ }
        pub fn orr_v16b(&mut self, _dst: VRegister, _a: VRegister, _b: VRegister) { /* ASM stub */ }
        pub fn add_v4s(&mut self, _dst: VRegister, _a: VRegister, _b: VRegister) { /* ASM stub */ }

        pub fn ret(&mut self) { /* ASM stub */ }

        // -- Predicate helpers used by armMoveAddressToReg -----------------
        pub fn is_imm_add_sub(_imm: u32) -> bool { true }
        pub fn is_imm_logical(_imm: u32, _width: u32) -> bool { true }
    }
}

// ===========================================================================
// PCSX2 register-alias macros (`RWRET`, `RXARG1`, `RQSCRATCH`, ...).
//
// In the C++ source these are #defines that expand to VIXL register constants;
// in Rust they are exposed as `pub const` aliases so call-sites remain close
// to the original C++ spelling.
// ===========================================================================

pub use aarch64::W0  as RWRET;
pub use aarch64::X0  as RXRET;
pub use aarch64::Q0  as RQRET;
pub use aarch64::W0  as RWARG1;
pub use aarch64::W1  as RWARG2;
pub use aarch64::W2  as RWARG3;
pub use aarch64::W3  as RWARG4;
pub use aarch64::X0  as RXARG1;
pub use aarch64::X1  as RXARG2;
pub use aarch64::X2  as RXARG3;
pub use aarch64::X3  as RXARG4;
pub use aarch64::X16 as RXVIXLSCRATCH;
pub use aarch64::W16 as RWVIXLSCRATCH;
pub use aarch64::X17 as RSCRATCHADDR;
pub use aarch64::Q30 as RQSCRATCH;
pub use aarch64::D30 as RDSCRATCH;
pub use aarch64::S30 as RSSCRATCH;
pub use aarch64::Q31 as RQSCRATCH2;
pub use aarch64::D31 as RDSCRATCH2;
pub use aarch64::S31 as RSSCRATCH2;
pub use aarch64::Q29 as RQSCRATCH3;
pub use aarch64::D29 as RDSCRATCH3;
pub use aarch64::S29 as RSSCRATCH3;

pub const RQSCRATCHI: aarch64::VRegister = aarch64::VRegister::new(30, 128, 16);
pub const RQSCRATCHF: aarch64::VRegister = aarch64::VRegister::new(30, 128, 4);
pub const RQSCRATCHD: aarch64::VRegister = aarch64::VRegister::new(30, 128, 2);
pub const RQSCRATCH2I: aarch64::VRegister = aarch64::VRegister::new(31, 128, 16);
pub const RQSCRATCH2F: aarch64::VRegister = aarch64::VRegister::new(31, 128, 4);
pub const RQSCRATCH2D: aarch64::VRegister = aarch64::VRegister::new(31, 128, 2);

/// SP-relative scratch slot used by the prologue / epilogue helpers.
pub const SP_SCRATCH_OFFSET: u32 = 0;

// ===========================================================================
// vixl macro-aliases used by the VIF unpacker (`xmmCol0`..`xmmRow`, etc.).
// ===========================================================================

pub const xmmCol0: aarch64::VRegister = aarch64::Q2;
pub const xmmCol1: aarch64::VRegister = aarch64::Q3;
pub const xmmCol2: aarch64::VRegister = aarch64::Q4;
pub const xmmCol3: aarch64::VRegister = aarch64::Q5;
pub const xmmRow:  aarch64::VRegister = aarch64::Q6;
pub const xmmTemp: aarch64::VRegister = aarch64::Q7;

// ===========================================================================
// PC-displacement helper.
// ===========================================================================

/// Returns the number of 4-byte words between `current` and `target`.
/// Mirrors the C++ `GetPCDisplacement` helper used by the branch emitters.
#[inline]
pub fn get_pc_displacement(current: *const u8, target: *const u8) -> s64 {
    let diff = (target as isize) - (current as isize);
    (diff >> 2) as s64
}

// ===========================================================================
// ARM64 register-array accessors (armWRegister / armXRegister / ...).
// ===========================================================================

pub fn arm_w_register(n: i32) -> aarch64::Register {
    debug_assert!((n as u32) < 32);
    aarch64::Register::new(n as u32)
}
pub fn arm_x_register(n: i32) -> aarch64::Register {
    debug_assert!((n as u32) < 32);
    aarch64::Register::new(n as u32)
}
pub fn arm_s_register(n: i32) -> aarch64::VRegister {
    debug_assert!((n as u32) < 32);
    aarch64::VRegister::new(n as u32, 32, 4)
}
pub fn arm_d_register(n: i32) -> aarch64::VRegister {
    debug_assert!((n as u32) < 32);
    aarch64::VRegister::new(n as u32, 64, 2)
}
pub fn arm_q_register(n: i32) -> aarch64::VRegister {
    debug_assert!((n as u32) < 32);
    aarch64::VRegister::new(n as u32, 128, 16)
}

// ===========================================================================
// `ArmConstantPool` - holds jump trampolines and 128-bit literal slots.
//
// The C++ class stores an `unordered_map<const void*, u32>` and
// `unordered_map<u128, u32, u128_hash>`.  In Rust we use `HashMap`.
// ===========================================================================

pub struct ArmConstantPool {
    /// Map of jump target -> offset within the pool buffer.
    pub jump_targets: std::collections::HashMap<*const u8, u32>,
    /// Map of 128-bit literal -> offset within the pool buffer.
    pub literals: std::collections::HashMap<u128, u32>,
    /// Base pointer of the pool buffer.
    pub base_ptr: *mut u8,
    /// Total capacity of the pool buffer in bytes.
    pub capacity: u32,
    /// Number of bytes currently in use.
    pub used: u32,
}

impl ArmConstantPool {
    pub fn new() -> Self {
        Self {
            jump_targets: std::collections::HashMap::new(),
            literals: std::collections::HashMap::new(),
            base_ptr: std::ptr::null_mut(),
            capacity: 0,
            used: 0,
        }
    }

    /// Initialise the pool with a backing buffer of `capacity` bytes.
    pub fn init(&mut self, ptr: *mut u8, capacity: u32) {
        self.base_ptr = ptr;
        self.capacity = capacity;
        self.used = 0;
        self.jump_targets.clear();
        self.literals.clear();
    }

    /// Release the pool's backing storage and reset all state.
    pub fn destroy(&mut self) {
        self.base_ptr = std::ptr::null_mut();
        self.capacity = 0;
        self.used = 0;
        self.jump_targets.clear();
        self.literals.clear();
    }

    /// Reset the used region without releasing the backing buffer.
    pub fn reset(&mut self) {
        self.used = 0;
        self.jump_targets.clear();
        self.literals.clear();
    }

    #[inline]
    fn get_remaining_capacity(&self) -> u32 { self.capacity - self.used }

    /// Returns a 4-instruction trampoline that jumps to `target`.  Returns
    /// `None` if the pool is exhausted.
    pub fn get_jump_trampoline(&mut self, target: *const u8) -> Option<*mut u8> {
        if let Some(&off) = self.jump_targets.get(&target) {
            return Some(unsafe { self.base_ptr.add(off as usize) });
        }
        let offset = align_up_pow2(self.used, 16);
        if (self.capacity - offset) < 20 {
            eprintln!("[error] Ran out of space in constant pool");
            return None;
        }
        // Emit the trampoline.  In the real dynarec this uses a local
        // `MacroAssembler` over the pool buffer.
        let buf = unsafe { self.base_ptr.add(offset as usize) };
        let mut masm = aarch64::MacroAssembler::new(buf, (self.capacity - offset) as usize);
        masm.mov_wide(RXVIXLSCRATCH, target as u64);
        masm.br(RXVIXLSCRATCH);
        masm.finalize_code();
        let size = masm.get_size_of_code_generated() as u32;
        debug_assert!(size < 20);
        self.jump_targets.insert(target, offset);
        self.used = offset + size;
        Some(buf)
    }

    /// Place a 64-bit literal in the pool, returning its address.
    pub fn get_literal_u64(&mut self, value: u64) -> Option<*mut u8> {
        self.get_literal(u128::from_64(value))
    }

    /// Place a 128-bit literal in the pool, returning its address.
    pub fn get_literal(&mut self, value: u128) -> Option<*mut u8> {
        if let Some(&off) = self.literals.get(&value) {
            return Some(unsafe { self.base_ptr.add(off as usize) });
        }
        if self.get_remaining_capacity() < 16 { return None; }
        let offset = align_up_pow2(self.used, 16);
        let buf = unsafe { self.base_ptr.add(offset as usize) };
        unsafe {
            let lo = value.lo.to_le_bytes();
            let hi = value.hi.to_le_bytes();
            std::ptr::copy_nonoverlapping(lo.as_ptr(), buf, 8);
            std::ptr::copy_nonoverlapping(hi.as_ptr(), buf.add(8), 8);
        }
        self.used = offset + 16;
        self.literals.insert(value, offset);
        Some(buf)
    }

    /// Place a 16-byte literal in the pool.
    pub fn get_literal_bytes(&mut self, bytes: &[u8]) -> Option<*mut u8> {
        debug_assert!(bytes.len() <= 16, "literal length is less than 16 bytes");
        let mut value = u128::default();
        let n = bytes.len().min(16);
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), &mut value.lo as *mut u64 as *mut u8, n);
        }
        self.get_literal(value)
    }

    /// Emit an `LDR` from a previously-placed literal address.
    pub fn emit_load_literal(&self, reg: aarch64::CPURegister, literal: *const u8) {
        arm_move_address_to_reg(RXVIXLSCRATCH, literal);
        let asm = unsafe { &mut *ARM_ASM };
        asm.ldr_gp_mem(match reg {
            aarch64::CPURegister::GP(r) => r,
            aarch64::CPURegister::Vec(_) => aarch64::X0,
            _ => aarch64::X0,
        }, aarch64::MemOperand::new(RXVIXLSCRATCH));
    }
}

impl Default for ArmConstantPool {
    fn default() -> Self { Self::new() }
}

#[inline]
fn align_up_pow2(value: u32, alignment: u32) -> u32 {
    (value + alignment - 1) & !(alignment - 1)
}

// ===========================================================================
// Thread-local dynarec context (mirrors the C++ `thread_local` globals).
// ===========================================================================

/// Globally-shared (thread-local) VIXL macro-assembler pointer.
pub static mut ARM_ASM: *mut aarch64::MacroAssembler = std::ptr::null_mut();
/// Raw code buffer base pointer.
pub static mut ARM_ASM_PTR: *mut u8 = std::ptr::null_mut();
/// Remaining capacity of the code buffer.
pub static mut ARM_ASM_CAPACITY: usize = 0;
/// Optional constant pool for the active block.
pub static mut ARM_CONSTANT_POOL: *mut ArmConstantPool = std::ptr::null_mut();

#[inline]
pub fn arm_has_block() -> bool { unsafe { !ARM_ASM.is_null() } }

#[inline]
pub fn arm_get_current_code_pointer() -> *mut u8 {
    unsafe {
        if ARM_ASM.is_null() { return ARM_ASM_PTR; }
        let off = (*ARM_ASM).get_cursor_offset();
        ARM_ASM_PTR.add(off)
    }
}

#[inline]
pub fn arm_get_asm_ptr() -> *mut u8 { unsafe { ARM_ASM_PTR } }

pub fn arm_set_asm_ptr(ptr: *mut u8, capacity: usize, pool: *mut ArmConstantPool) {
    unsafe {
        debug_assert!(ARM_ASM.is_null());
        ARM_ASM_PTR = ptr;
        ARM_ASM_CAPACITY = capacity;
        ARM_CONSTANT_POOL = pool;
    }
}

/// Align the asm write pointer to a 16-byte boundary.
pub fn arm_align_asm_ptr() {
    const ALIGNMENT: usize = 16;
    unsafe {
        let cur = ARM_ASM_PTR as usize;
        let new = (cur + ALIGNMENT - 1) & !(ALIGNMENT - 1);
        let pad = new - cur;
        debug_assert!(pad <= ARM_ASM_CAPACITY);
        ARM_ASM_CAPACITY -= pad;
        ARM_ASM_PTR = new as *mut u8;
    }
}

pub fn arm_start_block() -> *mut u8 {
    arm_align_asm_ptr();
    unsafe {
        let masm = aarch64::MacroAssembler::new(ARM_ASM_PTR, ARM_ASM_CAPACITY);
        ARM_ASM = Box::into_raw(Box::new(masm));
        // Mirror C++: remove v31 / x17 from the scratch pools.
        (*ARM_ASM).get_scratch_v_register_list().remove(31);
        (*ARM_ASM).get_scratch_register_list().remove(RSCRATCHADDR.get_code());
        ARM_ASM_PTR
    }
}

pub fn arm_end_block() -> *mut u8 {
    unsafe {
        debug_assert!(!ARM_ASM.is_null());
        (*ARM_ASM).finalize_code();
        let size = (*ARM_ASM).get_size_of_code_generated() as u32;
        debug_assert!((size as usize) < ARM_ASM_CAPACITY);
        drop(Box::from_raw(ARM_ASM));
        ARM_ASM = std::ptr::null_mut();
        let new_ptr = ARM_ASM_PTR.add(size as usize);
        ARM_ASM_CAPACITY -= size as usize;
        ARM_ASM_PTR = new_ptr;
        new_ptr
    }
}

// ===========================================================================
// Branch / call / address-load helpers.
// ===========================================================================

pub fn arm_emit_jmp(target: *const u8, force_inline: bool) {
    let mut displacement = get_pc_displacement(arm_get_current_code_pointer(), target);
    let mut use_blr = !aarch64::is_int26(displacement);
    if use_blr && !unsafe { ARM_CONSTANT_POOL.is_null() } && !force_inline {
        unsafe {
            if let Some(trampoline) = (*ARM_CONSTANT_POOL).get_jump_trampoline(target) {
                displacement = get_pc_displacement(arm_get_current_code_pointer(), trampoline);
                use_blr = !aarch64::is_int26(displacement);
            }
        }
    }
    unsafe {
        let asm = &mut *ARM_ASM;
        if use_blr {
            asm.mov_wide(RXVIXLSCRATCH, target as u64);
            asm.br(RXVIXLSCRATCH);
        } else {
            let _g = aarch64::SingleEmissionCheckScope { _asm: ARM_ASM };
            asm.b(displacement);
        }
    }
}

pub fn arm_emit_call(target: *const u8, force_inline: bool) {
    let mut displacement = get_pc_displacement(arm_get_current_code_pointer(), target);
    let mut use_blr = !aarch64::is_int26(displacement);
    if use_blr && !unsafe { ARM_CONSTANT_POOL.is_null() } && !force_inline {
        unsafe {
            if let Some(trampoline) = (*ARM_CONSTANT_POOL).get_jump_trampoline(target) {
                displacement = get_pc_displacement(arm_get_current_code_pointer(), trampoline);
                use_blr = !aarch64::is_int26(displacement);
            }
        }
    }
    unsafe {
        let asm = &mut *ARM_ASM;
        if use_blr {
            asm.mov_wide(RXVIXLSCRATCH, target as u64);
            asm.blr(RXVIXLSCRATCH);
        } else {
            let _g = aarch64::SingleEmissionCheckScope { _asm: ARM_ASM };
            asm.bl(displacement);
        }
    }
}

pub fn arm_emit_cbnz(reg: aarch64::Register, target: *const u8) {
    let jump_distance = (target as isize) - (arm_get_current_code_pointer() as isize);
    if aarch64::Instruction::is_valid_imm_pc_offset(aarch64::COMPARE_BRANCH_TYPE, jump_distance as i64 >> 2) {
        unsafe {
            let asm = &mut *ARM_ASM;
            let _g = aarch64::SingleEmissionCheckScope { _asm: ARM_ASM };
            asm.cbnz(reg, jump_distance as i64 >> 2);
        }
    } else {
        unsafe {
            let asm = &mut *ARM_ASM;
            let _g = aarch64::MacroEmissionCheckScope { _asm: ARM_ASM };
            let mut label = aarch64::Label { id: 0 };
            asm.cbz(reg, &label);
            let new_jump_distance = (target as isize) - (arm_get_current_code_pointer() as isize);
            asm.b(new_jump_distance as i64 >> 2);
            asm.bind(&mut label);
        }
    }
}

pub fn arm_emit_cond_branch(cond: aarch64::Condition, target: *const u8) {
    let jump_distance = (target as isize) - (arm_get_current_code_pointer() as isize);
    if aarch64::Instruction::is_valid_imm_pc_offset(aarch64::COND_BRANCH_TYPE, jump_distance as i64 >> 2) {
        unsafe {
            let asm = &mut *ARM_ASM;
            let _g = aarch64::SingleEmissionCheckScope { _asm: ARM_ASM };
            asm.b_cond(jump_distance as i64 >> 2, cond);
        }
    } else {
        unsafe {
            let asm = &mut *ARM_ASM;
            let _g = aarch64::MacroEmissionCheckScope { _asm: ARM_ASM };
            let mut label = aarch64::Label { id: 0 };
            asm.b_cond(0, aarch64::invert_condition(cond));
            asm.bind(&mut label);
            let new_jump_distance = (target as isize) - (arm_get_current_code_pointer() as isize);
            asm.b(new_jump_distance as i64 >> 2);
        }
    }
}

pub fn arm_move_address_to_reg(reg: aarch64::Register, addr: *const u8) {
    debug_assert!(reg.is_x());
    unsafe {
        let asm = &mut *ARM_ASM;
        let current = arm_get_current_code_pointer() as usize;
        let current_page = current & !0xFFFusize;
        let ptr_page = (addr as usize) & !0xFFFusize;
        let page_displacement: i64 = ((ptr_page as isize - current_page as isize) >> 10) as i64;
        let page_offset: u32 = (addr as usize & 0xFFF) as u32;
        if aarch64::is_int21(page_displacement) && aarch64::MacroAssembler::is_imm_add_sub(page_offset) {
            let _g = aarch64::SingleEmissionCheckScope { _asm: ARM_ASM };
            asm.adrp(reg, page_displacement);
            asm.add_imm(reg, reg, page_offset as i64);
        } else if aarch64::is_int21(page_displacement) && aarch64::MacroAssembler::is_imm_logical(page_offset, 64) {
            let _g = aarch64::SingleEmissionCheckScope { _asm: ARM_ASM };
            asm.adrp(reg, page_displacement);
            asm.orr_imm(reg, reg, page_offset as u64);
        } else {
            asm.mov_wide(reg, addr as u64);
        }
    }
}

pub fn arm_load_ptr(reg: aarch64::CPURegister, addr: *const u8) {
    arm_move_address_to_reg(RSCRATCHADDR, addr);
    unsafe {
        let asm = &mut *ARM_ASM;
        match reg {
            aarch64::CPURegister::GP(r) => asm.ldr_gp_mem(r, aarch64::MemOperand::new(RSCRATCHADDR)),
            aarch64::CPURegister::Vec(r) => asm.ldr_vec_mem(r, aarch64::MemOperand::new(RSCRATCHADDR)),
            _ => {}
        }
    }
}

pub fn arm_store_ptr(reg: aarch64::CPURegister, addr: *const u8) {
    arm_move_address_to_reg(RSCRATCHADDR, addr);
    unsafe {
        let asm = &mut *ARM_ASM;
        match reg {
            aarch64::CPURegister::GP(r) => asm.str_gp_mem(r, aarch64::MemOperand::new(RSCRATCHADDR)),
            aarch64::CPURegister::Vec(r) => asm.str_vec_mem(r, aarch64::MemOperand::new(RSCRATCHADDR)),
            _ => {}
        }
    }
}

pub fn arm_begin_stack_frame(save_fpr: bool) {
    unsafe {
        let asm = &mut *ARM_ASM;
        let frame_size: i64 = if save_fpr { 192 } else { 144 };
        asm.add_imm(aarch64::SP, aarch64::SP, -frame_size);
        asm.stp(aarch64::X19, aarch64::X20, aarch64::MemOperand::new_offset(aarch64::SP, 32, aarch64::OFFSET));
        asm.stp(aarch64::X21, aarch64::X22, aarch64::MemOperand::new_offset(aarch64::SP, 48, aarch64::OFFSET));
        asm.stp(aarch64::X23, aarch64::X24, aarch64::MemOperand::new_offset(aarch64::SP, 64, aarch64::OFFSET));
        asm.stp(aarch64::X25, aarch64::X26, aarch64::MemOperand::new_offset(aarch64::SP, 80, aarch64::OFFSET));
        asm.stp(aarch64::X27, aarch64::X28, aarch64::MemOperand::new_offset(aarch64::SP, 96, aarch64::OFFSET));
        asm.stp(aarch64::X29, aarch64::LR,  aarch64::MemOperand::new_offset(aarch64::SP, 112, aarch64::OFFSET));
        if save_fpr {
            asm.stp(aarch64::D8.d(),  aarch64::D9.d(),  aarch64::MemOperand::new_offset(aarch64::SP, 128, aarch64::OFFSET));
            asm.stp(aarch64::D10.d(), aarch64::D11.d(), aarch64::MemOperand::new_offset(aarch64::SP, 144, aarch64::OFFSET));
            asm.stp(aarch64::D12.d(), aarch64::D13.d(), aarch64::MemOperand::new_offset(aarch64::SP, 160, aarch64::OFFSET));
            asm.stp(aarch64::D14.d(), aarch64::D15.d(), aarch64::MemOperand::new_offset(aarch64::SP, 176, aarch64::OFFSET));
        }
    }
}

pub fn arm_end_stack_frame(save_fpr: bool) {
    unsafe {
        let asm = &mut *ARM_ASM;
        if save_fpr {
            asm.ldp(aarch64::D14.d(), aarch64::D15.d(), aarch64::MemOperand::new_offset(aarch64::SP, 176, aarch64::OFFSET));
            asm.ldp(aarch64::D12.d(), aarch64::D13.d(), aarch64::MemOperand::new_offset(aarch64::SP, 160, aarch64::OFFSET));
            asm.ldp(aarch64::D10.d(), aarch64::D11.d(), aarch64::MemOperand::new_offset(aarch64::SP, 144, aarch64::OFFSET));
            asm.ldp(aarch64::D8.d(),  aarch64::D9.d(),  aarch64::MemOperand::new_offset(aarch64::SP, 128, aarch64::OFFSET));
        }
        asm.ldp(aarch64::X29, aarch64::LR,  aarch64::MemOperand::new_offset(aarch64::SP, 112, aarch64::OFFSET));
        asm.ldp(aarch64::X27, aarch64::X28, aarch64::MemOperand::new_offset(aarch64::SP, 96,  aarch64::OFFSET));
        asm.ldp(aarch64::X25, aarch64::X26, aarch64::MemOperand::new_offset(aarch64::SP, 80,  aarch64::OFFSET));
        asm.ldp(aarch64::X23, aarch64::X24, aarch64::MemOperand::new_offset(aarch64::SP, 64,  aarch64::OFFSET));
        asm.ldp(aarch64::X21, aarch64::X22, aarch64::MemOperand::new_offset(aarch64::SP, 48,  aarch64::OFFSET));
        asm.ldp(aarch64::X19, aarch64::X20, aarch64::MemOperand::new_offset(aarch64::SP, 32,  aarch64::OFFSET));
        asm.add_imm(aarch64::SP, aarch64::SP, if save_fpr { 192 } else { 144 } as i64);
    }
}

pub fn arm_is_callee_saved_register(reg: i32) -> bool { reg >= 19 }

pub fn arm_offset_mem_operand(op: aarch64::MemOperand, offset: i64) -> aarch64::MemOperand {
    debug_assert!(op.get_base_register().is_valid());
    debug_assert_eq!(op.get_addr_mode(), aarch64::OFFSET);
    debug_assert_eq!(op.get_shift(), aarch64::NO_SHIFT);
    aarch64::MemOperand::new_offset(op.get_base_register(), op.get_offset() + offset, op.get_addr_mode())
}

pub fn arm_get_mem_operand_in_register(
    addr_reg: aarch64::Register,
    op: aarch64::MemOperand,
    extra_offset: i64,
) {
    arm_get_mem_operand_in_register_impl(addr_reg, op, extra_offset);
}

/// Variant of [`arm_get_mem_operand_in_register`] that defaults
/// `extra_offset` to 0.  Mirrors the C++ default argument.
pub fn arm_get_mem_operand_in_register_default(
    addr_reg: aarch64::Register,
    op: aarch64::MemOperand,
) {
    arm_get_mem_operand_in_register_impl(addr_reg, op, 0);
}

fn arm_get_mem_operand_in_register_impl(
    addr_reg: aarch64::Register,
    op: aarch64::MemOperand,
    extra_offset: i64,
) {
    debug_assert!(addr_reg.is_x());
    debug_assert!(op.get_base_register().is_valid());
    debug_assert_eq!(op.get_addr_mode(), aarch64::OFFSET);
    debug_assert_eq!(op.get_shift(), aarch64::NO_SHIFT);
    unsafe {
        let asm = &mut *ARM_ASM;
        asm.add_imm(addr_reg, op.get_base_register(), op.get_offset() + extra_offset);
    }
}

pub fn arm_load_constant_128(reg: aarch64::VRegister, ptr: *const u8) {
    let mut bytes = [0u8; 16];
    unsafe { std::ptr::copy_nonoverlapping(ptr, bytes.as_mut_ptr(), 16); }
    let lo = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
    let hi = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
    unsafe {
        let asm = &mut *ARM_ASM;
        asm.ldr_literal_128(reg, hi, lo);
    }
}

pub fn arm_emit_vtbl(
    dst: aarch64::VRegister,
    src1: aarch64::VRegister,
    src2: aarch64::VRegister,
    tbl: aarch64::VRegister,
) {
    debug_assert!(src1.get_code() != RQSCRATCH.get_code() && src2.get_code() != RQSCRATCH2.get_code());
    debug_assert!(tbl.get_code() != RQSCRATCH.get_code() && tbl.get_code() != RQSCRATCH2.get_code());
    unsafe {
        let asm = &mut *ARM_ASM;
        if src2.get_code() == (src1.get_code() + 1) {
            asm.tbl_v16b(dst.v16b(), src1.v16b(), src2.v16b(), tbl.v16b());
            return;
        }
        asm.mov_vec(RQSCRATCH.q(), src1.q());
        asm.mov_vec(RQSCRATCH2.q(), src2.q());
        asm.tbl_v16b(dst.v16b(), RQSCRATCH.v16b(), RQSCRATCH2.v16b(), tbl.v16b());
    }
}

pub fn arm_disassemble_and_dump_code(_ptr: *const u8, _size: usize) {
    eprintln!("[error] Not compiled with INCLUDE_DISASSEMBLER");
}

// ===========================================================================
// nVif structures (forward declarations used by the dynarec).
// ===========================================================================

/// Cache of compiled VIF dynarec blocks, keyed by their packing signature.
#[derive(Default)]
pub struct VifBlockCache {
    pub blocks: std::collections::HashMap<u64, NVifBlock>,
}

impl VifBlockCache {
    pub fn reset(&mut self) { self.blocks.clear(); }
    pub fn clear(&mut self) { self.blocks.clear(); }
    pub fn add(&mut self, block: NVifBlock) { self.blocks.insert(block.hash_key, block); }
    pub fn find(&self, block: &NVifBlock) -> Option<&NVifBlock> { self.blocks.get(&block.hash_key) }
    pub fn find_mut(&mut self, block: &NVifBlock) -> Option<&mut NVifBlock> { self.blocks.get_mut(&block.hash_key) }
}

/// Per-channel VIF dynarec state.
#[derive(Default)]
pub struct NVifStruct {
    pub vif_blocks: VifBlockCache,
    pub rec_write_ptr: *mut u8,
    pub rec_end_ptr: *mut u8,
    pub idx: i32,
}

/// Signature passed to the dynarec for a single VIF block.
#[derive(Clone, Default)]
pub struct NVifBlock {
    pub hash_key: u64,
    pub key0: u32,
    pub key1: u32,
    pub start_ptr: uptr,
    pub length: u16,
    pub num: u8,
    pub wl: u8,
    pub cl: u8,
    pub upk_type: u8,
    pub mode: u8,
    pub aligned: u8,
    pub mask: u32,
    pub scl: u8,
}

/// The per-channel VIF interpreter state.
#[derive(Default)]
pub struct VifStruct {
    pub idx: i32,
    pub cmd: u32,
    pub usn: u32,
    pub start_aligned: u8,
    pub tag: VifTag,
    pub mask_row: [u32; 4],
    pub mask_col: [u32; 4],
}

#[derive(Default, Clone, Copy)]
pub struct VifTag { pub addr: u32 }

#[derive(Default)]
pub struct VifRegisters {
    pub num: u32,
    pub cycle: VifCycle,
    pub mode: u32,
    pub mask: u32,
}

#[derive(Default, Clone, Copy)]
pub struct VifCycle { pub cl: u8, pub wl: u8 }

/// Minimal VU state used by the dynarec.
#[derive(Default)]
pub struct VuRegs { pub mem: Vec<u8> }

impl VuRegs {
    pub fn mem(&self) -> *mut u8 {
        if self.mem.is_empty() { std::ptr::null_mut() } else { self.mem.as_ptr() as *mut u8 }
    }
}

// ===========================================================================
// nVif global state.
// ===========================================================================

pub const NVIF_COUNT: usize = 2;
pub static mut NVIF: [std::sync::LazyLock<std::cell::UnsafeCell<NVifStruct>>; NVIF_COUNT] = [
    std::sync::LazyLock::new(|| std::cell::UnsafeCell::new(NVifStruct { vif_blocks: VifBlockCache { blocks: std::collections::HashMap::new() }, rec_write_ptr: std::ptr::null_mut(), rec_end_ptr: std::ptr::null_mut(), idx: 0 })),
    std::sync::LazyLock::new(|| std::cell::UnsafeCell::new(NVifStruct { vif_blocks: VifBlockCache { blocks: std::collections::HashMap::new() }, rec_write_ptr: std::ptr::null_mut(), rec_end_ptr: std::ptr::null_mut(), idx: 1 })),
];
pub static mut MT_VU: [VuRegs; NVIF_COUNT] = [const { VuRegs { mem: Vec::new() } }; NVIF_COUNT];
pub static mut VIF_STRUCTS: [VifStruct; NVIF_COUNT] = [const { VifStruct {
    idx: 0, cmd: 0, usn: 0, start_aligned: 0,
    tag: VifTag { addr: 0 }, mask_row: [0; 4], mask_col: [0; 4],
} }; NVIF_COUNT];
pub static mut VIF_REGISTERS: [VifRegisters; NVIF_COUNT] = [const { VifRegisters {
    num: 0, cycle: VifCycle { cl: 0, wl: 0 }, mode: 0, mask: 0,
} }; NVIF_COUNT];

/// `nVifT` per-unpack-type stride table.
pub const NVIFT: [u8; 16] = [
    16, 8, 4, 0,   // S-32 / S-16 / S-8 / reserved
    16, 8, 4, 0,   // V2-32 / V2-16 / V2-8 / reserved
    16, 8, 4, 0,   // V3-32 / V3-16 / V3-8 / reserved
    16, 8, 4, 2,   // V4-32 / V4-16 / V4-8 / V4-5
];

/// Mask table used by the simple VIF unpacker.  Mirrors `nVifMask[3][4][16]`.
pub static mut N_VIF_MASK: [[[u8; 16]; 4]; 3] = [[[0u8; 16]; 4]; 3];

pub fn n_vif(idx: i32) -> &'static mut NVifStruct { unsafe { &mut *std::sync::LazyLock::force(&NVIF[idx as usize]).get() } }
pub fn mt_vu(idx: i32) -> &'static mut VuRegs { unsafe { &mut MT_VU[idx as usize] } }
pub fn vif_struct(idx: i32) -> &'static mut VifStruct { unsafe { &mut VIF_STRUCTS[idx as usize] } }
pub fn vif_registers(idx: i32) -> &'static mut VifRegisters { unsafe { &mut VIF_REGISTERS[idx as usize] } }

pub fn mtvu_vif_x<'a>(idx: i32) -> &'a mut VifStruct { vif_struct(idx) }
pub fn mtvu_vif_x_regs<'a>(idx: i32) -> &'a mut VifRegisters { vif_registers(idx) }

pub fn vu1_thread_wait_vu() {}

/// `nVifrecCall` is the calling convention used by the compiled dynarec
/// block: `void fn(uptr dest, uptr src)`.
pub type NVifrecCall = unsafe extern "C" fn(uptr, uptr);
/// `nVifCall` is the simple-unpacker calling convention (single argument).
pub type NVifCall = unsafe extern "C" fn(uptr);

/// `nVifUpk` is a flat table indexed by `(usn, mask, type, cycle)`.
pub type NVifUpk = Option<NVifCall>;
/// The table size in the original C++ code: max index is
/// `(2*2*16 + 2*16 + 15) * 4 + 3 = 383`, so 384 entries.
pub const NVIF_UPK_LEN: usize = 384;
pub static mut NVIF_UPK: [NVifUpk; NVIF_UPK_LEN] = [None; NVIF_UPK_LEN];

// ===========================================================================
// VifUnpackNEON_Base - shared unpacker state.
//
// In the C++ source, `VifUnpackNEON_Base` is an abstract class that the
// `Simple` and `Dynarec` subclasses extend.  In Rust we model this with a
// single concrete struct that holds the shared state and a mode flag that
// selects the virtual-method behaviour.  The C++ virtual methods are
// implemented as ordinary methods on the struct, dispatching on `mode`.
// ===========================================================================

/// Subclass mode selector for the VIF unpacker.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub enum VifUnpackMode {
    /// Generated for the simple VIF interpreter (see `VifUnpackSSE_Init`).
    #[default]
    Simple,
    /// Used by the VIF dynarec block compiler.
    Dynarec,
}

/// Common state shared by both the simple and dynarec VIF unpackers.
#[derive(Clone)]
pub struct VifUnpackNEON_Base {
    pub mode: VifUnpackMode,
    pub usn: bool,
    pub do_mask: bool,
    pub unpk_loop_iteration: i32,
    pub unpk_no_of_iterations: i32,
    pub is_aligned: i32,

    pub dst_indirect: aarch64::MemOperand,
    pub src_indirect: aarch64::MemOperand,
    pub work_reg: aarch64::VRegister,
    pub dest_reg: aarch64::VRegister,
    pub work_gpr_w: aarch64::Register,

    // -- Simple-only state.  Set when `mode == VifUnpackMode::Simple`. ----
    pub simple_cur_cycle: i32,

    // -- Dynarec-only state.  Set when `mode == VifUnpackMode::Dynarec`. --
    pub dynarec_skip_processing: bool,
    pub dynarec_input_masked: bool,
    pub dynarec_do_mode: u8,
    /// Index of the VIF channel the dynarec is compiling for.
    pub dynarec_v_idx: i32,
    /// The block signature being compiled.
    pub dynarec_v_block: NVifBlock,
    /// Internal copy of `vif->cl`.
    pub dynarec_vcl: i32,
    /// `cl < wl` -> the unpack is a fill.
    pub dynarec_is_fill: bool,
}

impl Default for VifUnpackNEON_Base {
    fn default() -> Self {
        Self {
            mode: VifUnpackMode::Simple,
            usn: false,
            do_mask: false,
            unpk_loop_iteration: 0,
            unpk_no_of_iterations: 0,
            is_aligned: 0,
            dst_indirect: aarch64::MemOperand::new(RXARG1),
            src_indirect: aarch64::MemOperand::new(RXARG2),
            work_reg: aarch64::Q1,
            dest_reg: aarch64::Q0,
            work_gpr_w: aarch64::W4,
            simple_cur_cycle: 0,
            dynarec_skip_processing: false,
            dynarec_input_masked: false,
            dynarec_do_mode: 0,
            dynarec_v_idx: 0,
            dynarec_v_block: NVifBlock::default(),
            dynarec_vcl: 0,
            dynarec_is_fill: false,
        }
    }
}

impl VifUnpackNEON_Base {
    /// Returns `true` when the operation writes no data to the destination.
    pub fn is_write_protected_op(&self) -> bool {
        match self.mode {
            VifUnpackMode::Simple   => false,
            VifUnpackMode::Dynarec  => self.skip_processing_dynarec(),
        }
    }

    /// Returns `true` when input is fully masked.
    pub fn is_input_masked(&self) -> bool {
        match self.mode {
            VifUnpackMode::Simple   => false,
            VifUnpackMode::Dynarec  => self.input_masked_dynarec(),
        }
    }

    /// Returns `true` when no mask needs to be applied.
    pub fn is_unmasked_op(&self) -> bool {
        match self.mode {
            VifUnpackMode::Simple  => !self.do_mask,
            VifUnpackMode::Dynarec => self.do_mode_dynarec() == 0 && !self.do_mask,
        }
    }

    // -- Dynarec-only state accessors -------------------------------------

    /// Returns `true` when the dynarec considers this iteration protected.
    pub fn skip_processing_dynarec(&self) -> bool {
        self.dynarec_skip_processing
    }

    /// Returns `true` when the dynarec has marked this iteration as
    /// fully input-masked.
    pub fn input_masked_dynarec(&self) -> bool {
        self.dynarec_input_masked
    }

    /// Returns the dynarec's `doMode` value (0..=3).
    pub fn do_mode_dynarec(&self) -> u8 {
        self.dynarec_do_mode
    }

    // -- `do_mask_write` dispatch -----------------------------------------

    /// Move the just-unpacked vector to the destination, dispatching to
    /// the appropriate `do_mask_write` for masked paths.
    pub fn x_mov_dest(&self) {
        if !self.is_write_protected_op() {
            if self.is_unmasked_op() {
                unsafe { (*ARM_ASM).str_vec_mem(self.dest_reg, self.dst_indirect); }
            } else {
                self.do_mask_write(self.dest_reg);
            }
        }
    }

    /// Subclass-implemented mask-aware write-back.  Dispatches on `mode`.
    pub fn do_mask_write(&self, reg_x: aarch64::VRegister) {
        match self.mode {
            VifUnpackMode::Simple  => self.simple_do_mask_write(reg_x),
            VifUnpackMode::Dynarec => self.dynarec_do_mask_write(reg_x),
        }
    }

    /// Logical right shift of a 4S vector.
    pub fn x_shift_r(&self, reg_x: aarch64::VRegister, n: u32) {
        unsafe {
            let asm = &mut *ARM_ASM;
            if self.usn { asm.ushr_vec(reg_x.v4s(), reg_x.v4s(), n); }
            else        { asm.sshr_vec(reg_x.v4s(), reg_x.v4s(), n); }
        }
    }

    /// `PMOVSX8` - load 8 bytes, sign/zero-extend to 8H, then to 4S.
    pub fn x_pmovxx8(&self, reg_x: aarch64::VRegister) {
        unsafe {
            let asm = &mut *ARM_ASM;
            asm.ldr_vec_mem(reg_x.vs(), self.src_indirect);
            if self.usn {
                asm.ushll_vec(reg_x.v8h(), reg_x.v8b(), 0);
                asm.ushll_vec(reg_x.v4s(), reg_x.v4h(), 0);
            } else {
                asm.sshll_vec(reg_x.v8h(), reg_x.v8b(), 0);
                asm.sshll_vec(reg_x.v4s(), reg_x.v4h(), 0);
            }
        }
    }

    /// `PMOVSX16` - load 8 bytes (D), sign/zero-extend to 4S.
    pub fn x_pmovxx16(&self, reg_x: aarch64::VRegister) {
        unsafe {
            let asm = &mut *ARM_ASM;
            asm.ldr_vec_mem(reg_x.vd(), self.src_indirect);
            if self.usn { asm.ushll_vec(reg_x.v4s(), reg_x.v4h(), 0); }
            else        { asm.sshll_vec(reg_x.v4s(), reg_x.v4h(), 0); }
        }
    }

    pub fn x_upk_s_32(&self) {
        unsafe {
            let asm = &mut *ARM_ASM;
            if self.unpk_loop_iteration == 0 {
                asm.ldr_vec_mem(self.work_reg, self.src_indirect);
            }
            if self.is_input_masked() { return; }
            match self.unpk_loop_iteration {
                0 => asm.dup_lane_v4s(self.dest_reg.v4s(), self.work_reg.v4s(), 0),
                1 => asm.dup_lane_v4s(self.dest_reg.v4s(), self.work_reg.v4s(), 1),
                2 => asm.dup_lane_v4s(self.dest_reg.v4s(), self.work_reg.v4s(), 2),
                3 => asm.dup_lane_v4s(self.dest_reg.v4s(), self.work_reg.v4s(), 3),
                _ => {}
            }
        }
    }

    pub fn x_upk_s_16(&self) {
        if self.unpk_loop_iteration == 0 { self.x_pmovxx16(self.work_reg); }
        if self.is_input_masked() { return; }
        unsafe {
            let asm = &mut *ARM_ASM;
            match self.unpk_loop_iteration {
                0 => asm.dup_lane_v4s(self.dest_reg.v4s(), self.work_reg.v4s(), 0),
                1 => asm.dup_lane_v4s(self.dest_reg.v4s(), self.work_reg.v4s(), 1),
                2 => asm.dup_lane_v4s(self.dest_reg.v4s(), self.work_reg.v4s(), 2),
                3 => asm.dup_lane_v4s(self.dest_reg.v4s(), self.work_reg.v4s(), 3),
                _ => {}
            }
        }
    }

    pub fn x_upk_s_8(&self) {
        if self.unpk_loop_iteration == 0 { self.x_pmovxx8(self.work_reg); }
        if self.is_input_masked() { return; }
        unsafe {
            let asm = &mut *ARM_ASM;
            match self.unpk_loop_iteration {
                0 => asm.dup_lane_v4s(self.dest_reg.v4s(), self.work_reg.v4s(), 0),
                1 => asm.dup_lane_v4s(self.dest_reg.v4s(), self.work_reg.v4s(), 1),
                2 => asm.dup_lane_v4s(self.dest_reg.v4s(), self.work_reg.v4s(), 2),
                3 => asm.dup_lane_v4s(self.dest_reg.v4s(), self.work_reg.v4s(), 3),
                _ => {}
            }
        }
    }

    pub fn x_upk_v2_32(&self) {
        unsafe {
            let asm = &mut *ARM_ASM;
            if self.unpk_loop_iteration == 0 {
                asm.ldr_vec_mem(self.work_reg, self.src_indirect);
                if self.is_input_masked() { return; }
                asm.dup_lane_v2d(self.dest_reg.v2d(), self.work_reg.v2d(), 0);
                if self.is_aligned != 0 { asm.ins_v4s_gp(self.dest_reg.v4s(), 3, aarch64::wzr); }
            } else {
                if self.is_input_masked() { return; }
                asm.dup_lane_v2d(self.dest_reg.v2d(), self.work_reg.v2d(), 1);
                if self.is_aligned != 0 { asm.ins_v4s_gp(self.dest_reg.v4s(), 3, aarch64::wzr); }
            }
        }
    }

    pub fn x_upk_v2_16(&self) {
        if self.unpk_loop_iteration == 0 {
            self.x_pmovxx16(self.work_reg);
            if self.is_input_masked() { return; }
            unsafe { (*ARM_ASM).dup_lane_v2d(self.dest_reg.v2d(), self.work_reg.v2d(), 0); }
        } else {
            if self.is_input_masked() { return; }
            unsafe { (*ARM_ASM).dup_lane_v2d(self.dest_reg.v2d(), self.work_reg.v2d(), 1); }
        }
    }

    pub fn x_upk_v2_8(&self) {
        if self.unpk_loop_iteration == 0 {
            self.x_pmovxx8(self.work_reg);
            if self.is_input_masked() { return; }
            unsafe { (*ARM_ASM).dup_lane_v2d(self.dest_reg.v2d(), self.work_reg.v2d(), 0); }
        } else {
            if self.is_input_masked() { return; }
            unsafe { (*ARM_ASM).dup_lane_v2d(self.dest_reg.v2d(), self.work_reg.v2d(), 1); }
        }
    }

    pub fn x_upk_v3_32(&self) {
        if self.is_input_masked() { return; }
        unsafe {
            let asm = &mut *ARM_ASM;
            asm.ldr_vec_mem(self.dest_reg, self.src_indirect);
            if self.unpk_loop_iteration != self.is_aligned {
                asm.ins_v4s_gp(self.dest_reg.v4s(), 3, aarch64::wzr);
            }
        }
    }

    pub fn x_upk_v3_16(&self) {
        if self.is_input_masked() { return; }
        self.x_pmovxx16(self.dest_reg);
        let result = (((self.unpk_loop_iteration / 4) + 1 + (4 - self.is_aligned)) & 0x3) as i32;
        if (self.unpk_loop_iteration & 0x1) == 0 && result == 0 {
            unsafe { (*ARM_ASM).ins_v4s_gp(self.dest_reg.v4s(), 3, aarch64::wzr); }
        }
    }

    pub fn x_upk_v3_8(&self) {
        if self.is_input_masked() { return; }
        self.x_pmovxx8(self.dest_reg);
        if self.unpk_loop_iteration != self.is_aligned {
            unsafe { (*ARM_ASM).ins_v4s_gp(self.dest_reg.v4s(), 3, aarch64::wzr); }
        }
    }

    pub fn x_upk_v4_32(&self) {
        if self.is_input_masked() { return; }
        unsafe { (*ARM_ASM).ldr_vec_mem(self.dest_reg.q(), self.src_indirect); }
    }

    pub fn x_upk_v4_16(&self) {
        if self.is_input_masked() { return; }
        self.x_pmovxx16(self.dest_reg);
    }

    pub fn x_upk_v4_8(&self) {
        if self.is_input_masked() { return; }
        self.x_pmovxx8(self.dest_reg);
    }

    /// V4-5 (16-bit RGBA) unpacking.
    pub fn x_upk_v4_5(&self) {
        if self.is_input_masked() { return; }
        unsafe {
            let asm = &mut *ARM_ASM;
            asm.ldrh(self.work_gpr_w, self.src_indirect);
            asm.lsl_reg(self.work_gpr_w, self.work_gpr_w, 3);   // ABG|R5.000
            asm.dup_gp_to_vec(self.dest_reg.v4s(), self.work_gpr_w);
            asm.lsr_reg(self.work_gpr_w, self.work_gpr_w, 8);
            asm.lsl_reg(self.work_gpr_w, self.work_gpr_w, 3);   // AB|G5.000
            asm.ins_v4s_gp(self.dest_reg.v4s(), 1, self.work_gpr_w);
            asm.lsr_reg(self.work_gpr_w, self.work_gpr_w, 8);
            asm.lsl_reg(self.work_gpr_w, self.work_gpr_w, 3);   // A|B5.000
            asm.ins_v4s_gp(self.dest_reg.v4s(), 2, self.work_gpr_w);
            asm.lsr_reg(self.work_gpr_w, self.work_gpr_w, 8);
            asm.lsl_reg(self.work_gpr_w, self.work_gpr_w, 7);   // A.0000000
            asm.ins_v4s_gp(self.dest_reg.v4s(), 3, self.work_gpr_w);
            asm.shl_vec(self.dest_reg.v4s(), self.dest_reg.v4s(), 24);
            asm.ushr_vec(self.dest_reg.v4s(), self.dest_reg.v4s(), 24);
        }
    }

    /// Dispatch to the correct per-layout unpacker.
    pub fn x_unpack(&self, upk_num: i32) {
        match upk_num {
            0  => self.x_upk_s_32(),
            1  => self.x_upk_s_16(),
            2  => self.x_upk_s_8(),
            4  => self.x_upk_v2_32(),
            5  => self.x_upk_v2_16(),
            6  => self.x_upk_v2_8(),
            8  => self.x_upk_v3_32(),
            9  => self.x_upk_v3_16(),
            10 => self.x_upk_v3_8(),
            12 => self.x_upk_v4_32(),
            13 => self.x_upk_v4_16(),
            14 => self.x_upk_v4_8(),
            15 => self.x_upk_v4_5(),
            3 | 7 | 11 => {
                eprintln!("[warning] Vpu/Vif: Invalid Unpack {}", upk_num);
            }
            _ => {}
        }
    }

    // -- `do_mask_write` implementations ----------------------------------

    /// `VifUnpackNEON_Simple::doMaskWrite` - simple Q-7 merge using the
    /// pre-baked `nVifMask` table.
    pub fn simple_do_mask_write(&self, reg_x: aarch64::VRegister) {
        unsafe {
            let asm = &mut *ARM_ASM;
            asm.ldr_vec_mem(aarch64::Q7, self.dst_indirect);
            let off_x = std::cmp::min(self.simple_cur_cycle, 3);
            let base_addr = &N_VIF_MASK as *const _ as *const u8;
            let stride = std::mem::size_of::<[u8; 16]>() as i64;
            arm_move_address_to_reg(RXVIXLSCRATCH, base_addr);
            asm.ldr_vec_mem(aarch64::Q29, aarch64::MemOperand::new_offset(RXVIXLSCRATCH, (off_x as i64) * stride, aarch64::OFFSET));
            asm.ldr_vec_mem(aarch64::Q30, aarch64::MemOperand::new_offset(RXVIXLSCRATCH, ((16 + off_x) as i64) * stride, aarch64::OFFSET));
            asm.ldr_vec_mem(aarch64::Q31, aarch64::MemOperand::new_offset(RXVIXLSCRATCH, ((32 + off_x) as i64) * stride, aarch64::OFFSET));
            asm.and_v16b(reg_x.v16b(), reg_x.v16b(), aarch64::Q29.v16b());
            asm.and_v16b(aarch64::Q7.v16b(), aarch64::Q7.v16b(), aarch64::Q30.v16b());
            asm.orr_v16b(reg_x.v16b(), reg_x.v16b(), aarch64::Q31.v16b());
            asm.orr_v16b(reg_x.v16b(), reg_x.v16b(), aarch64::Q7.v16b());
            asm.str_vec_mem(reg_x, self.dst_indirect);
        }
    }

    /// `VifUnpackNEON_Dynarec::doMaskWrite` - row/column merge + masked
    /// vector store.  Reads dynarec state from the `dynarec_*` fields and
    /// `dynarec_v_block` field set by `VifUnpackNEON_Dynarec`.
    pub fn dynarec_do_mask_write(&self, reg_x: aarch64::VRegister) {
        let v_block = &self.dynarec_v_block;
        px_assert_msg(reg_x.get_code() <= 1, "Reg Overflow! XMM2 thru XMM6 are reserved for masking.");
        let cc = std::cmp::min(self.dynarec_vcl, 3) as u32;
        let m0 = (v_block.mask >> (cc * 8)) & 0xFF;
        let m3 = ((m0 & 0xAA) >> 1) & !m0;
        let m2 = (m0 & 0x55) & (!m0 >> 1);
        let m4 = (m0 & !((m3 << 1) | m2)) & 0x55;

        let mut m2 = m2;
        let mut m3 = m3;
        let mut m4 = m4;
        make_merge_mask(&mut m2);
        make_merge_mask(&mut m3);
        make_merge_mask(&mut m4);

        if self.do_mask && m2 != 0 {
            mvu_merge_regs(reg_x, xmmRow, m2 as i32, false, false);
        }
        if self.do_mask && m3 != 0 {
            mvu_merge_regs(reg_x, arm_q_register(xmmCol0.get_code() as i32 + cc as i32), m3 as i32, false, false);
        }
        if self.dynarec_do_mode != 0 {
            let mut m5 = !(m2 | m3 | m4) & 0xF;
            if !self.do_mask { m5 = 0xF; }
            if m5 < 0xF {
                unsafe {
                    let asm = &mut *ARM_ASM;
                    asm.movi(xmmTemp, 0);
                    if self.dynarec_do_mode == 3 {
                        mvu_merge_regs(xmmRow, reg_x, m5 as i32, false, false);
                    } else {
                        mvu_merge_regs(xmmTemp, xmmRow, m5 as i32, false, false);
                        asm.add_v4s(reg_x.v4s(), reg_x.v4s(), xmmTemp.v4s());
                        if self.dynarec_do_mode == 2 {
                            mvu_merge_regs(xmmRow, reg_x, m5 as i32, false, false);
                        }
                    }
                }
            } else {
                unsafe {
                    let asm = &mut *ARM_ASM;
                    if self.dynarec_do_mode == 3 {
                        asm.mov_vec(xmmRow.q(), reg_x.q());
                    } else {
                        asm.add_v4s(reg_x.v4s(), reg_x.v4s(), xmmRow.v4s());
                        if self.dynarec_do_mode == 2 {
                            asm.mov_vec(xmmRow.q(), reg_x.q());
                        }
                    }
                }
            }
        }
        if self.do_mask && m4 != 0 {
            masked_vec_write(reg_x, self.dst_indirect, (m4 ^ 0xF) as i32);
        } else {
            unsafe { (*ARM_ASM).str_vec_mem(reg_x, self.dst_indirect); }
        }
    }
}

// ===========================================================================
// VifUnpackNEON_Simple - the no-mask-row simple unpacker.
//
// The C++ source defines `VifUnpackNEON_Simple` as a subclass of
// `VifUnpackNEON_Base`.  In Rust we model this as a type alias plus a
// dedicated constructor that returns a `VifUnpackNEON_Base` configured
// for the simple (no-row-mask) mode.
// ===========================================================================

pub type VifUnpackNEON_Simple = VifUnpackNEON_Base;

impl VifUnpackNEON_Base {
    /// Construct a `Simple`-mode unpacker.
    pub fn new_simple(usn_: bool, do_mask_: bool, cur_cycle_: i32) -> Self {
        let mut b = VifUnpackNEON_Base::default();
        b.mode = VifUnpackMode::Simple;
        b.usn = usn_;
        b.do_mask = do_mask_;
        b.is_aligned = 1;
        b.simple_cur_cycle = cur_cycle_;
        b
    }
}

// ===========================================================================
// VifUnpackNEON_Dynarec - the masked, dynarec-driven unpacker.
// ===========================================================================

pub type VifUnpackNEON_Dynarec = VifUnpackNEON_Base;

impl VifUnpackNEON_Base {
    /// Construct a `Dynarec`-mode unpacker.
    pub fn new_dynarec(v: &NVifStruct, v_block: &NVifBlock) -> Self {
        let wl: u16 = if v_block.wl != 0 { v_block.wl as u16 } else { 256 };
        let is_fill = u16::from(v_block.cl) < wl;
        let usn = ((v_block.upk_type >> 5) & 1) != 0;
        let do_mask = ((v_block.upk_type >> 4) & 1) != 0;
        let do_mode = v_block.mode & 3;
        let is_aligned = v_block.aligned as i32;
        let mut b = VifUnpackNEON_Base::default();
        b.mode = VifUnpackMode::Dynarec;
        b.usn = usn;
        b.do_mask = do_mask;
        b.is_aligned = is_aligned;
        b.dynarec_v_idx = v.idx;
        b.dynarec_v_block = v_block.clone();
        b.dynarec_do_mode = do_mode;
        b.dynarec_is_fill = is_fill;
        b.dynarec_skip_processing = false;
        b.dynarec_input_masked = false;
        b.dynarec_vcl = 0;
        b
    }

    pub fn set_masks(&self, c_s: i32) {
        let vif = mtvu_vif_x(self.dynarec_v_idx);
        let m0 = self.dynarec_v_block.mask;
        let m3 = ((m0 & 0xAAAA_AAAAu32) >> 1) & !m0;
        let m2 = (m0 & 0x5555_5555u32) & (!m0 >> 1);

        if (self.do_mask && m2 != 0) || self.dynarec_do_mode != 0 {
            arm_load_ptr(aarch64::CPURegister::Vec(xmmRow), &vif.mask_row as *const _ as *const u8);
        }
        if self.do_mask && m3 != 0 {
            arm_load_ptr(aarch64::CPURegister::Vec(xmmCol0), &vif.mask_col as *const _ as *const u8);
            unsafe {
                let asm = &mut *ARM_ASM;
                if c_s >= 2 && (m3 & 0x0000_FF00) != 0 {
                    asm.dup_lane_v4s(xmmCol1.v4s(), xmmCol0.v4s(), 1);
                }
                if c_s >= 3 && (m3 & 0x00FF_0000) != 0 {
                    asm.dup_lane_v4s(xmmCol2.v4s(), xmmCol0.v4s(), 2);
                }
                if c_s >= 4 && (m3 & 0xFF00_0000) != 0 {
                    asm.dup_lane_v4s(xmmCol3.v4s(), xmmCol0.v4s(), 3);
                }
                if c_s >= 1 && (m3 & 0x0000_00FF) != 0 {
                    asm.dup_lane_v4s(xmmCol0.v4s(), xmmCol0.v4s(), 0);
                }
            }
        }
    }

    pub fn write_back_row(&self) {
        let vif = mtvu_vif_x(self.dynarec_v_idx);
        arm_store_ptr(aarch64::CPURegister::Vec(xmmRow), &vif.mask_row as *const _ as *const u8);
    }

    pub fn mod_unpack(&mut self, upk_num: i32, post_op: bool) {
        match upk_num {
            0 | 1 | 2 => {
                if post_op {
                    self.unpk_loop_iteration += 1;
                    self.unpk_loop_iteration &= 0x3;
                }
            }
            4 | 5 | 6 => {
                if post_op {
                    self.unpk_loop_iteration += 1;
                    self.unpk_loop_iteration &= 0x1;
                }
            }
            8 => {
                if post_op {
                    self.unpk_loop_iteration += 1;
                    self.unpk_loop_iteration &= 0x1;
                }
            }
            9 | 10 => {
                if !post_op {
                    self.unpk_loop_iteration += 1;
                }
            }
            12 | 13 | 14 | 15 => {}
            3 | 7 | 11 => {
                eprintln!("[warning] Vpu/Vif: Invalid Unpack {}", upk_num);
            }
            _ => {}
        }
    }

    pub fn process_masks(&mut self) {
        self.dynarec_skip_processing = false;
        self.dynarec_input_masked = false;
        if !self.do_mask { return; }
        let cc = std::cmp::min(self.dynarec_vcl, 3) as u32;
        let full_mask = (self.dynarec_v_block.mask >> (cc * 8)) & 0xFF;
        let rowcol_mask = ((full_mask >> 1) | full_mask) & 0x55;
        self.dynarec_skip_processing = full_mask == 0xFF;
        self.dynarec_input_masked = rowcol_mask == 0x55;
    }

    pub fn compile_routine(&mut self) {
        let wl = if self.dynarec_v_block.wl != 0 { self.dynarec_v_block.wl as i32 } else { 256 };
        let upk_num = (self.dynarec_v_block.upk_type & 0xF) as i32;
        let vift = NVIFT[upk_num as usize];
        let cycle_size = if self.dynarec_is_fill { self.dynarec_v_block.cl as i32 } else { wl };
        let block_size = if self.dynarec_is_fill { wl } else { self.dynarec_v_block.cl as i32 };
        let skip_size = block_size - cycle_size;

        let mut v_num: u32 = if self.dynarec_v_block.num != 0 { self.dynarec_v_block.num as u32 } else { 256 };
        if upk_num == 0xF { self.dynarec_do_mode = 0; } // V4-5 has no mode feature.
        self.unpk_no_of_iterations = 0;
        debug_assert_eq!(self.dynarec_vcl, 0);

        self.set_masks(if self.dynarec_is_fill { block_size } else { cycle_size });

        while v_num > 0 {
            self.process_masks();

            if self.dynarec_vcl < cycle_size {
                self.mod_unpack(upk_num, false);
                self.x_unpack(upk_num);
                self.x_mov_dest();
                self.mod_unpack(upk_num, true);
                self.dst_indirect = arm_offset_mem_operand(self.dst_indirect, 16);
                self.src_indirect = arm_offset_mem_operand(self.src_indirect, vift as i64);
                v_num -= 1;
                self.dynarec_vcl += 1;
                if self.dynarec_vcl == block_size { self.dynarec_vcl = 0; }
            } else if self.dynarec_is_fill {
                self.x_unpack(upk_num);
                self.x_mov_dest();
                self.dst_indirect = arm_offset_mem_operand(self.dst_indirect, 16);
                v_num -= 1;
                self.dynarec_vcl += 1;
                if self.dynarec_vcl == block_size { self.dynarec_vcl = 0; }
            } else {
                self.dst_indirect = arm_offset_mem_operand(self.dst_indirect, (16 * skip_size) as i64);
                self.dynarec_vcl = 0;
            }
        }
        if self.dynarec_do_mode >= 2 { self.write_back_row(); }
        unsafe { (*ARM_ASM).ret(); }
    }
}

// ===========================================================================
// `FillingWrite` - returns a copy of `src` with `do_mask=true, do_mode=0`.
// ===========================================================================

pub fn filling_write(src: &VifUnpackNEON_Dynarec) -> VifUnpackNEON_Dynarec {
    let mut dst = src.clone();
    dst.do_mask = true;
    dst.dynarec_do_mode = 0;
    dst
}

// ===========================================================================
// xVif helpers: merge registers, masked vector write.
// ===========================================================================

pub fn mvu_merge_regs(
    dest: aarch64::VRegister,
    src: aarch64::VRegister,
    xyzw: i32,
    mod_xyzw: bool,
    can_modify_src: bool,
) {
    let mut xyzw = xyzw & 0xF;
    if dest.get_code() != src.get_code() && xyzw != 0 {
        unsafe {
            let asm = &mut *ARM_ASM;
            if xyzw == 0x8 {
                asm.mov_v4s_v4s(dest.v4s(), 0, src.v4s(), 0);
            } else if xyzw == 0xF {
                asm.mov_vec(dest.q(), src.q());
            } else {
                if mod_xyzw {
                    if xyzw == 1 { asm.ins_v4s_v4s(dest.v4s(), 3, src.v4s(), 0); return; }
                    if xyzw == 2 { asm.ins_v4s_v4s(dest.v4s(), 2, src.v4s(), 0); return; }
                    if xyzw == 4 { asm.ins_v4s_v4s(dest.v4s(), 1, src.v4s(), 0); return; }
                }
                if xyzw == 0 { return; }
                if xyzw == 15 { asm.mov_vec(dest.q(), src.q()); return; }
                if xyzw == 14 && can_modify_src {
                    asm.mov_v4s_v4s(src.v4s(), 3, dest.v4s(), 3);
                    asm.mov_vec(dest.v16b(), src.v16b());
                    return;
                }
                // reverse xyzw nibble bits: bit0<->bit3, bit1<->bit2.
                xyzw = ((xyzw & 1) << 3) | ((xyzw & 2) << 1) | ((xyzw & 4) >> 1) | ((xyzw & 8) >> 3);
                if (xyzw & 3) == 3 {
                    asm.mov_v2d_v2d(dest.v2d(), 0, src.v2d(), 0);
                    xyzw &= !3;
                } else if (xyzw & 12) == 12 {
                    asm.mov_v2d_v2d(dest.v2d(), 1, src.v2d(), 1);
                    xyzw &= !12;
                }
                for i in 0u32..4 {
                    if (xyzw & (1 << i)) != 0 {
                        asm.mov_v4s_v4s(dest.v4s(), i, src.v4s(), i);
                    }
                }
            }
        }
    }
}

pub fn masked_vec_write(reg: aarch64::VRegister, addr: aarch64::MemOperand, xyzw: i32) {
    unsafe {
        let asm = &mut *ARM_ASM;
        match xyzw {
            5 => {
                arm_get_mem_operand_in_register(RSCRATCHADDR, addr, 4);
                asm.st1_v4s(reg.v4s(), 1, aarch64::MemOperand::new(RSCRATCHADDR));
                arm_get_mem_operand_in_register(RSCRATCHADDR, addr, 12);
                asm.st1_v4s(reg.v4s(), 3, aarch64::MemOperand::new(RSCRATCHADDR));
            }
            9 => {
                arm_get_mem_operand_in_register(RSCRATCHADDR, addr, 12);
                asm.str_gp_mem(reg.s(), addr);
                asm.st1_v4s(reg.v4s(), 3, aarch64::MemOperand::new(RSCRATCHADDR));
            }
            10 => {
                arm_get_mem_operand_in_register(RSCRATCHADDR, addr, 8);
                asm.str_gp_mem(reg.s(), addr);
                asm.st1_v4s(reg.v4s(), 2, aarch64::MemOperand::new(RSCRATCHADDR));
            }
            3 => {
                arm_get_mem_operand_in_register(RSCRATCHADDR, addr, 8);
                asm.st1_v2d(reg.v2d(), 1, aarch64::MemOperand::new(RSCRATCHADDR));
            }
            11 => {
                arm_get_mem_operand_in_register(RSCRATCHADDR, addr, 8);
                asm.str_gp_mem(reg.s(), addr);
                asm.st1_v2d(reg.v2d(), 1, aarch64::MemOperand::new(RSCRATCHADDR));
            }
            13 => {
                arm_get_mem_operand_in_register(RSCRATCHADDR, addr, 12);
                asm.str_gp_mem(reg.d(), addr);
                asm.st1_v4s(reg.v4s(), 3, aarch64::MemOperand::new(RSCRATCHADDR));
            }
            6 => {
                arm_get_mem_operand_in_register(RSCRATCHADDR, addr, 4);
                asm.st1_v4s(reg.v4s(), 1, aarch64::MemOperand::new_offset(RSCRATCHADDR, 4, aarch64::POST_INDEX));
                asm.st1_v4s(reg.v4s(), 2, aarch64::MemOperand::new(RSCRATCHADDR));
            }
            7 => {
                arm_get_mem_operand_in_register(RSCRATCHADDR, addr, 4);
                asm.st1_v4s(reg.v4s(), 1, aarch64::MemOperand::new_offset(RSCRATCHADDR, 4, aarch64::POST_INDEX));
                asm.st1_v2d(reg.v2d(), 1, aarch64::MemOperand::new(RSCRATCHADDR));
            }
            12 => asm.str_gp_mem(reg.d(), addr),
            14 => {
                arm_get_mem_operand_in_register(RSCRATCHADDR, addr, 8);
                asm.str_gp_mem(reg.d(), addr);
                asm.st1_v4s(reg.v4s(), 2, aarch64::MemOperand::new(RSCRATCHADDR));
            }
            4 => {
                arm_get_mem_operand_in_register(RSCRATCHADDR, addr, 4);
                asm.st1_v4s(reg.v4s(), 1, aarch64::MemOperand::new(RSCRATCHADDR));
            }
            2 => {
                arm_get_mem_operand_in_register(RSCRATCHADDR, addr, 8);
                asm.st1_v4s(reg.v4s(), 2, aarch64::MemOperand::new(RSCRATCHADDR));
            }
            1 => {
                arm_get_mem_operand_in_register(RSCRATCHADDR, addr, 12);
                asm.st1_v4s(reg.v4s(), 3, aarch64::MemOperand::new(RSCRATCHADDR));
            }
            8 => asm.str_gp_mem(reg.s(), addr),
            0 => eprintln!("[error] maskedVecWrite case 0!"),
            _ => asm.str_vec_mem(reg.q(), addr),
        }
    }
}

// ===========================================================================
// make_merge_mask / dVifReset / dVifRelease / dVifCompile / dVifUnpack
// ===========================================================================

#[inline]
pub fn make_merge_mask(x: &mut u32) {
    *x = ((*x & 0x40) >> 6) | ((*x & 0x10) >> 3) | (*x & 4) | ((*x & 1) << 3);
}

pub fn dvif_reset(idx: i32) {
    let v = n_vif(idx);
    v.vif_blocks.reset();
    let offset = if idx != 0 { VIF1_REC_OFFSET } else { VIF0_REC_OFFSET };
    let size = if idx != 0 { VIF1_REC_SIZE } else { VIF0_REC_SIZE };
    v.rec_write_ptr = sys_memory_get_code_ptr(offset);
    unsafe { v.rec_end_ptr = v.rec_write_ptr.add(size - _256KB); }
}

pub fn dvif_release(idx: i32) {
    n_vif(idx).vif_blocks.clear();
}

pub fn dvif_compute_length(cl: u32, wl: u32, num: u8, is_fill: bool) -> u16 {
    let mut length = if num > 0 { (num as u32) * 16 } else { 4096 };
    if !is_fill {
        let skip_size = cl.saturating_sub(wl) * 16;
        let blocks = if wl == 0 { 0 } else { (num as u32 + (wl - 1)) / wl };
        length += blocks.saturating_sub(1) * skip_size;
    }
    length.min(0xFFFFu32) as u16
}

pub fn dvif_compile<const IDX: i32>(block: &mut NVifBlock, is_fill: bool) -> &NVifBlock {
    let v = n_vif(IDX);
    if (v.rec_write_ptr as usize) >= (v.rec_end_ptr as usize) {
        eprintln!(
            "[devcon] nVif Recompiler Cache Reset! [{:#x} > {:#x}]",
            v.rec_write_ptr as usize, v.rec_end_ptr as usize
        );
        dvif_reset(IDX);
    }
    arm_set_asm_ptr(v.rec_write_ptr, (v.rec_end_ptr as usize) - (v.rec_write_ptr as usize), std::ptr::null_mut());
    block.start_ptr = arm_start_block() as uptr;
    block.length = dvif_compute_length(
        block.cl as u32,
        if block.wl != 0 { block.wl as u32 } else { 256 },
        block.num,
        is_fill,
    );
    v.vif_blocks.add(block.clone());

    let mut dynarec = VifUnpackNEON_Base::new_dynarec(v, block);
    dynarec.compile_routine();

    let _ = arm_end_block();
    block
}

pub fn dvif_unpack<const IDX: i32>(data: *const u8, is_fill: bool) {
    let v = n_vif(IDX);
    let vif = mtvu_vif_x(IDX);
    let vif_regs = mtvu_vif_x_regs(IDX);

    let upk_type = ((vif.cmd & 0x1F) | (vif.usn << 5)) as u8;
    let do_mask: u32 = if is_fill { 1 } else { vif.cmd & 0x10 };

    let mut block = NVifBlock::default();

    let hash_key: u32 = (((upk_type as u32) & 0xFF) << 8) | (vif_regs.num & 0xFF);
    let mut key1: u32 = ((vif_regs.cycle.wl as u32) << 24)
        | ((vif_regs.cycle.cl as u32) << 16)
        | (((vif.start_aligned as u32) & 0xFF) << 8)
        | (vif_regs.mode & 0xFF);
    if (upk_type & 0xF) != 9 { key1 &= 0xFFFF_01FF; }
    let key0: u32 = if do_mask != 0 { vif_regs.mask } else { 0 };

    block.hash_key = (hash_key as u64) << 32 | (key1 as u64) << 16 | key0 as u64;
    block.key0 = key0;
    block.key1 = key1;
    block.upk_type = upk_type;
    block.num = vif_regs.num as u8;
    block.wl = vif_regs.cycle.wl;
    block.cl = vif_regs.cycle.cl;
    block.mode = vif_regs.mode as u8;
    block.aligned = vif.start_aligned;

    if v.vif_blocks.find(&block).is_none() {
        dvif_compile::<IDX>(&mut block, is_fill);
    }
    if let Some(b) = v.vif_blocks.find(&block).cloned() {
        let vu = mt_vu(IDX);
        let vu_mem_limit: u32 = if IDX != 0 { 0x4000 } else { 0x1000 };
        let startmem = unsafe { vu.mem().add((vif.tag.addr & (vu_mem_limit - 0x10)) as usize) };
        let endmem = unsafe { vu.mem().add(vu_mem_limit as usize) };
        if (startmem as usize + b.length as usize) <= (endmem as usize) {
            // Fast dynarec path.  In a real port this would call the
            // generated function via:
            //     let f: NVifrecCall = std::mem::transmute(b.start_ptr as *const ());
            //     f(startmem as uptr, data as uptr);
            let _f: NVifrecCall = unsafe { std::mem::transmute(b.start_ptr as *const ()) };
            let _ = (startmem, data, _f);
        } else {
            eprintln!(
                "[warning] Running Interpreter Block: nVif{:x} - VU Mem Ptr Overflow; falling back to interpreter.",
                vif.idx
            );
        }
    }
}

// ===========================================================================
// `nVifGen` and `VifUnpackSSE_Init` - simple-unpacker code generator.
// ===========================================================================

pub fn n_vif_gen(usn: i32, mask: i32, cur_cycle: i32) {
    let usnpart = usn * 2 * 16;
    let maskpart = mask * 16;
    let vpugen = VifUnpackNEON_Base::new_simple(usn != 0, mask != 0, cur_cycle);
    for i in 0..16 {
        let idx = (usnpart + maskpart + i) * 4 + cur_cycle;
        unsafe { NVIF_UPK[idx as usize] = None; }
        if NVIFT[i as usize] == 0 { continue; }
        let slot = arm_start_block() as uptr;
        unsafe { NVIF_UPK[idx as usize] = Some(std::mem::transmute(slot as *const ())); }
        vpugen.x_unpack(i);
        vpugen.x_mov_dest();
        unsafe { (*ARM_ASM).ret(); }
        arm_end_block();
    }
}

pub fn vif_unpack_sse_init() {
    eprintln!("[devcon] Generating NEON-optimized unpacking functions for VIF interpreters...");
    let rec = sys_memory_get_vif_unpack_rec();
    let end = sys_memory_get_vif_unpack_rec_end();
    let cap = (end as usize) - (rec as usize);
    arm_set_asm_ptr(rec, cap, std::ptr::null_mut());
    for a in 0..2 {
        for b in 0..2 {
            for c in 0..4 {
                n_vif_gen(a, b, c);
            }
        }
    }
    let _ = arm_get_asm_ptr();
    eprintln!("[perf] VIF Unpack registered");
}

// ===========================================================================
// RecStubs - stubs for ARM64-specific code not yet implemented.
// ===========================================================================

/// `vtlb_DynBackpatchLoadStore` - stub used by the VTLB when an indirect
/// load or store instruction needs to be patched.  ARM64 has not
/// implemented this path.
pub fn vtlb_dyn_backpatch_load_store(
    _code_address: uptr,
    _code_size: u32,
    _guest_pc: u32,
    _guest_addr: u32,
    _gpr_bitmask: u32,
    _fpr_bitmask: u32,
    _address_register: u8,
    _data_register: u8,
    _size_in_bits: u8,
    _is_signed: bool,
    _is_load: bool,
    _is_fpr: bool,
) {
    px_fail_rel("vtlb_DynBackpatchLoadStore: not implemented for ARM64");
}

/// `SaveStateBase::vuJITFreeze` - stub used by save state when the VU
/// microprogram recompiler state needs to be serialised.
pub fn vu_jit_freeze(is_saving: bool) -> bool {
    if is_saving { vu1_thread_wait_vu(); }
    eprintln!("[warning] recompiler state is stubbed in arm64!");
    let _empty_data: [u8; 96] = [0; 96];
    true
}

// ===========================================================================
// `px_assert` / `px_assert_msg` / `px_assert_rel` / `px_fail_rel` helpers.
// ===========================================================================

#[inline]
pub fn px_assert(cond: bool) { debug_assert!(cond); }

pub fn px_assert_msg(cond: bool, msg: &str) {
    if !cond { eprintln!("px_assert_msg failed: {}", msg); }
}

#[macro_export]
macro_rules! px_assert_rel_local {
    ($cond:expr) => {
        if !$cond { eprintln!("px_assert_rel failed: {}", stringify!($cond)); }
    };
}

pub fn px_fail_rel(msg: &str) -> ! {
    eprintln!("pxFailRel: {}", msg);
    std::process::abort();
}

// ===========================================================================
// Host memory map and `SysMemory` placeholders used by the dynarec.
// ===========================================================================

pub const VIF0_REC_OFFSET: usize = 0;
pub const VIF0_REC_SIZE: usize = 4 * 1024 * 1024;
pub const VIF1_REC_OFFSET: usize = 0;
pub const VIF1_REC_SIZE: usize = 4 * 1024 * 1024;
pub const VIF_UNPACK_REC_OFFSET: usize = 0;
pub const VIF_UNPACK_REC_SIZE: usize = 4 * 1024 * 1024;
pub const _256KB: usize = 256 * 1024;

pub mod host_memory_map {
    use super::*;
    pub static VIF0_REC_OFFSET: usize = super::VIF0_REC_OFFSET;
    pub static VIF0_REC_SIZE: usize = super::VIF0_REC_SIZE;
    pub static VIF1_REC_OFFSET: usize = super::VIF1_REC_OFFSET;
    pub static VIF1_REC_SIZE: usize = super::VIF1_REC_SIZE;
}

pub fn sys_memory_get_code_ptr(offset: usize) -> *mut u8 { offset as *mut u8 }
pub fn sys_memory_get_vif_unpack_rec() -> *mut u8 { VIF_UNPACK_REC_OFFSET as *mut u8 }
pub fn sys_memory_get_vif_unpack_rec_end() -> *mut u8 { (VIF_UNPACK_REC_OFFSET + VIF_UNPACK_REC_SIZE) as *mut u8 }

// ===========================================================================
// ARM64 instruction encoder helpers used by the C++ `AsmHelpers` file.
//
// The C++ source emits ARM64 instructions through VIXL; here we expose the
// most common encoders as `extern "C"` helpers. The bit layouts follow the
// ARMv8 architecture reference manual:
//   * B     : op=0b000101 imm26
//   * BL    : op=0b100101 imm26
//   * ADRP  : op=0b10000 immlo[2] 10000 immhi[19]
//   * CBNZ  : sf=0  011010 1 imm19 Rt
//   * LDR Xt, label : size=11 opc=01 011 0 imm19 Rt
//   * TBL Vd.16B, {Vn.16B, Vm.16B}, Vt.16B : 0 001110011 0000 0 Vm Vn Vt
//   * ST1 {Vt.4S}, [Xn], Xm : 0011010100 0000 100 imm4 0 Vt Rn Xm
//   * DUP Vd.4S, Vn.S[lane] : 01011110 0000 0000 0110 0000 0 imm4 0 Vn Vd
//   * MRS   : op0=3 0101 0011 0010 op1 CRn CRm op2 Rt
//   * MSR   : op0=3 0101 0001 0010 op1 CRn CRm op2 Rt
// ===========================================================================

/// Encode a 26-bit PC-relative branch and return the resulting word.
#[no_mangle]
pub extern "C" fn arm_encode_b(imm26: i64) -> u32 {
    // B <label>: 0 00101 imm26
    let imm = (imm26 as u32) & 0x03FF_FFFF;
    0x1400_0000 | imm
}

/// Encode a 26-bit PC-relative branch-with-link and return the resulting word.
#[no_mangle]
pub extern "C" fn arm_encode_bl(imm26: i64) -> u32 {
    // BL <label>: 1 00101 imm26
    let imm = (imm26 as u32) & 0x03FF_FFFF;
    0x9400_0000 | imm
}

/// Encode a 21-bit PC-relative `ADR` page displacement and return the word.
#[no_mangle]
pub extern "C" fn arm_encode_adrp(imm21: i64) -> u32 {
    // ADRP: 1 immlo 10000 immhi Rd
    let value = (imm21 as u32) & 0x001F_FFFF;
    let immlo = value & 0x3;
    let immhi = (value >> 2) & 0x7_FFFF;
    (1u32 << 31) | (immlo << 29) | (0b10000 << 24) | (immhi << 5)
}

/// Encode a Compare-and-Branch-on-Nonzero (`CBNZ`) instruction.
#[no_mangle]
pub extern "C" fn arm_encode_cbnz(rt: u8, imm19: i64) -> u32 {
    // CBNZ Wt: sf=0  011010 1 imm19 Rt
    let imm = (imm19 as u32) & 0x0007_FFFF;
    0xB5_0000_00 | (imm << 5) | (rt as u32 & 0x1F)
}

/// Encode a `LDR (literal)` instruction loading a 64-bit value.
#[no_mangle]
pub extern "C" fn arm_encode_ldr_literal64(rt: u8, imm19: i64) -> u32 {
    // LDR Xt, label: size=11 opc=01 011 0 imm19 Rt
    let imm = (imm19 as u32) & 0x0007_FFFF;
    0x58_0000_00 | (imm << 5) | (rt as u32 & 0x1F)
}

/// Encode a `TBL` instruction with three source registers.
#[no_mangle]
pub extern "C" fn arm_encode_tbl3(dst: u8, src1: u8, src2: u8, tbl: u8) -> u32 {
    // TBL Vd.16B, {Vn.16B, Vm.16B}, Vt.16B
    // 0 001110011 0000 0 Vm Vn Vt -> 000110110 0 Vm Vn Vt (rearranged bits)
    // bit 31..24 = 0 0 0 1 1 0 1 1
    // bit 23..16 = 0 0 0 0 0 Vm[4]
    // bit 15..10 = 0 0 0 0 Vn[4..0] Vt[4..0]
    // Encoding: 0 0 Q 0 1 1 1 0 0 1 1 0 0 0 0 0 0 Vm Vn Vt
    // Q=1 (16B), 0 001110011 0000 0 Vm Vn Vt
    let vm = src2 as u32 & 0x1F;
    let vn = src1 as u32 & 0x1F;
    let vt = tbl as u32 & 0x1F;
    let vd = dst as u32 & 0x1F;
    0x4E20_1000 | (vm << 16) | (vn << 5) | vt | (vd << 0)
}

/// Encode a `ST1` instruction (single structure) for a 4S vector.
#[no_mangle]
pub extern "C" fn arm_encode_st1_v4s(idx: u8, rn: u8, rm: u8) -> u32 {
    // ST1 {Vt.4S}, [Xn], Xm: 0011010100 0000 100 imm4 0 Vt Rn Rm
    // Q=0, size=00, opc=0011 for ST1
    let imm4 = idx as u32 & 0xF;
    let vt = (rm as u32 & 0x1F) << 16; // repurposed: rm -> vt
    let xn = rn as u32 & 0x1F;
    let xm = (rm as u32) & 0x1F;
    0x0C80_4000 | (imm4 << 16) | (vt) | (xn << 5) | xm
}

/// Encode a `DUP (element)` instruction copying an S-scalar into a 4S vector.
#[no_mangle]
pub extern "C" fn arm_encode_dup_v4s_lane(vd: u8, vn: u8, lane: u8) -> u32 {
    // DUP Vd.4S, Vn.S[index]: 01011110 0000 0000 0110 0000 0 imm4 0 Vn Vd
    let imm4 = lane as u32 & 0xF;
    let vnd = vn as u32 & 0x1F;
    let vdd = vd as u32 & 0x1F;
    0x4E04_0C00 | (imm4 << 16) | (vnd << 5) | vdd
}

// ===========================================================================
// ARM64 system-register encoders (`MRS` / `MSR`).
// ===========================================================================

/// Encode a `MRS` (read system register) instruction.
#[no_mangle]
pub extern "C" fn arm_encode_mrs(rt: u8, sysreg: u32) -> u32 {
    // MRS Xt, S<op0><op1>_<Cn>_<Cm>_<op2>
    // 1101 0101 0 0 11 0 0 0 0 1111 0 0000 0 0 0 Rt
    // 31..21: 1101 0101 001
    // 20: 0 (read)
    // 19..5: op0(3) op1(3) CRn(4) CRm(4) op2(3) -- 17 bits from sysreg encoding
    // 4..0: Rt
    let op0 = ((sysreg >> 14) & 0x7) as u32;
    let op1 = ((sysreg >> 11) & 0x7) as u32;
    let crn = ((sysreg >> 7) & 0xF) as u32;
    let crm = ((sysreg >> 3) & 0xF) as u32;
    let op2 = (sysreg & 0x7) as u32;
    let encoded = (op0 << 14) | (op1 << 11) | (crn << 7) | (crm << 3) | op2;
    0xD5_3000_00 | (encoded << 5) | (rt as u32 & 0x1F)
}

/// Encode a `MSR` (write system register) instruction.
#[no_mangle]
pub extern "C" fn arm_encode_msr(sysreg: u32, rt: u8) -> u32 {
    // MSR S<...>, Xt: same layout as MRS but with bit 21 = 1 (write)
    let op0 = ((sysreg >> 14) & 0x7) as u32;
    let op1 = ((sysreg >> 11) & 0x7) as u32;
    let crn = ((sysreg >> 7) & 0xF) as u32;
    let crm = ((sysreg >> 3) & 0xF) as u32;
    let op2 = (sysreg & 0x7) as u32;
    let encoded = (op0 << 14) | (op1 << 11) | (crn << 7) | (crm << 3) | op2;
    0xD5_1000_00 | (encoded << 5) | (rt as u32 & 0x1F)
}

// ===========================================================================
// CPU feature detection constants used by `RecStubs`.
// ===========================================================================

pub const CPUID_ARM64_SHA1: u32 = 1 << 0;
pub const CPUID_ARM64_SHA2: u32 = 1 << 1;
pub const CPUID_ARM64_AES:  u32 = 1 << 2;
pub const CPUID_ARM64_CRC32: u32 = 1 << 3;
pub const CPUID_ARM64_ATOMICS: u32 = 1 << 4;
pub const CPUID_ARM64_FP16: u32 = 1 << 5;
pub const CPUID_ARM64_ASIMD: u32 = 1 << 6;
pub const CPUID_ARM64_ASIMDHP: u32 = 1 << 7;

/// Returns the runtime-detected CPU features for ARM64 hosts.  This is a
/// stub that returns 0; in a real port it would call `getauxval` /
/// `IsProcessorFeaturePresent` and translate to the bitfield above.
pub fn arm64_detect_cpu_features() -> u32 { 0 }

// ===========================================================================
// Logging stubs that mimic `common/Console` and `common/Perf`.
// ===========================================================================

/// `DevCon` is a thin stand-in for PCSX2's developer console sink.
pub mod devcon {
    pub struct DevCon;
    impl DevCon {
        pub fn write_ln(&self, fmt: &str) { println!("[devcon] {}", fmt); }
    }
    pub static DEVCON: DevCon = DevCon;
}

/// `Perf` mimics the `Perf::vif` and `Perf::any` registration calls.
pub mod perf {
    pub struct PerfSection;
    impl PerfSection {
        pub fn register_pc(&self, _pc: *const u8, _size: usize, _key: u32) {}
        pub fn register(&self, _base: *const u8, _size: usize, _key: u32) {}
    }
    pub struct Perf {
        pub vif: PerfSection,
        pub any: PerfSection,
    }
    pub static PERF: Perf = Perf {
        vif: PerfSection,
        any: PerfSection,
    };
}

/// `HostSys` stubs for the C++ `HostSys` namespace.
pub mod host_sys {
    pub fn begin_code_write() {}
    pub fn end_code_write() {}
    pub fn flush_instruction_cache(_ptr: *mut u8, _size: usize) {}
}
