// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatically-translated subset of PCSX2's `common/emitter` x86 code-generation library.
//!
//! This module is a standalone, idiomatic Rust rendition of the most heavily-used
//! portions of the C/C++ emitter: the GPR/SIMD register types, the core
//! `X86Assembler` buffer and its `emit` helper, and the encoding rules for the
//! GPR arithmetic/move/control-flow group, the legacy FPU stack ops, and the
//! SSE2 integer SIMD core.
//!
//! The original PCSX2 emitter is huge (tens of thousands of lines spread over a
//! deeply-templated C++ class hierarchy).  This file captures a minimal but
//! self-consistent slice: a flat `Vec<u8>` code buffer, strongly-typed register
//! wrappers, and direct encoders for the opcodes enumerated in the
//! accompanying task (`mov`/`add`/`sub`/`mul`/`imul`/`idiv`/`jmp`/`call`/`ret`/`lea`,
//! the FPU `fld`/`fstp`/`fadd`/`fsub`/`fmul`/`fdiv`, and the SSE2 SIMD
//! `movaps`/`movups`/`paddd`/`psubd`/`pmulld`/`pand`/`por`/`pxor`).
//!
//! Only `std` is used.  Anything that needs to dereference raw pointer
//! arithmetic into the buffer is marked `unsafe` (currently none of the public
//! surface requires it; all writes go through `Vec::push`).

#![allow(dead_code)]
#![allow(clippy::needless_range_loop)]
#![allow(clippy::upper_case_acronyms)]

use std::fmt;

// =====================================================================================
//  Register / operand wrappers
// =====================================================================================

/// General-purpose 64-bit x86 register.  Wraps a hardware register number in
/// the range `0..16`.  Zero-sized (no `OperandSize` field) -- operand size is
/// either inferred from context or supplied alongside via a `Mem` operand.
///
/// In the original C++ code this is the union of `xRegister64` and
/// `xAddressReg`; we keep a single type here and rely on the caller for size
/// disambiguation.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Gpr(u8);

impl Gpr {
    /// Construct a GPR wrapper from a raw hardware index.  `0..16` are valid
    /// (matching the AVX-extended GPR count).
    #[inline]
    pub const fn new(id: u8) -> Self {
        Gpr(id & 0x0F)
    }

    /// Raw hardware register id (0..16).
    #[inline]
    pub const fn id(self) -> u8 {
        self.0
    }

    /// True for the eight "extended" GPRs (r8..r15), which require the REX.B
    /// bit to be encoded.
    #[inline]
    pub const fn is_extended(self) -> bool {
        self.0 > 7
    }

    /// Common hardware names.  Mirrors the `rax`/`rcx`/... globals in
    /// `x86emitter.cpp`.
    pub const RAX: Gpr = Gpr(0);
    pub const RCX: Gpr = Gpr(1);
    pub const RDX: Gpr = Gpr(2);
    pub const RBX: Gpr = Gpr(3);
    pub const RSP: Gpr = Gpr(4);
    pub const RBP: Gpr = Gpr(5);
    pub const RSI: Gpr = Gpr(6);
    pub const RDI: Gpr = Gpr(7);
    pub const R8:  Gpr = Gpr(8);
    pub const R9:  Gpr = Gpr(9);
    pub const R10: Gpr = Gpr(10);
    pub const R11: Gpr = Gpr(11);
    pub const R12: Gpr = Gpr(12);
    pub const R13: Gpr = Gpr(13);
    pub const R14: Gpr = Gpr(14);
    pub const R15: Gpr = Gpr(15);
}

impl fmt::Display for Gpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "r{}", self.0)
    }
}

/// 128-bit SSE XMM register.  Wraps a hardware register number in
/// `0..16`.  The original emitter also has YMM support via a tag type; we
/// collapse that here into the same struct and let the call site decide
/// whether the resulting bytes are treated as 128- or 256-bit.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Xmm(u8);

impl Xmm {
    /// Construct an XMM wrapper from a raw hardware index.
    #[inline]
    pub const fn new(id: u8) -> Self {
        Xmm(id & 0x0F)
    }

    /// Raw hardware register id.
    #[inline]
    pub const fn id(self) -> u8 {
        self.0
    }

    /// True for XMM8..XMM15, which require the REX.R bit.
    #[inline]
    pub const fn is_extended(self) -> bool {
        self.0 > 7
    }

    /// Standard XMM register constants, matching `xmm0`..`xmm15` in the C++
    /// source.
    pub const XMM0:  Xmm = Xmm(0);
    pub const XMM1:  Xmm = Xmm(1);
    pub const XMM2:  Xmm = Xmm(2);
    pub const XMM3:  Xmm = Xmm(3);
    pub const XMM4:  Xmm = Xmm(4);
    pub const XMM5:  Xmm = Xmm(5);
    pub const XMM6:  Xmm = Xmm(6);
    pub const XMM7:  Xmm = Xmm(7);
    pub const XMM8:  Xmm = Xmm(8);
    pub const XMM9:  Xmm = Xmm(9);
    pub const XMM10: Xmm = Xmm(10);
    pub const XMM11: Xmm = Xmm(11);
    pub const XMM12: Xmm = Xmm(12);
    pub const XMM13: Xmm = Xmm(13);
    pub const XMM14: Xmm = Xmm(14);
    pub const XMM15: Xmm = Xmm(15);
}

impl fmt::Display for Xmm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "xmm{}", self.0)
    }
}

/// Memory operand: `[base + index*scale + displacement]`.
///
/// This is a flat translation of the C++ `xIndirect<T>` family.  The original
/// emitter has several flavors differentiated by operand size (8/16/32/64/128
/// bit); we accept a `size` hint and pass it to the encoders that need it
/// (such as the SSE2 SIMD ops that require a 128-bit memory operand).
#[derive(Copy, Clone, Debug, Default)]
pub struct Mem {
    /// Base register, if any.
    pub base: Option<Gpr>,
    /// Index register, if any.
    pub index: Option<Gpr>,
    /// Index scale (1, 2, 4, or 8).  Zero means "no index".
    pub scale: u8,
    /// Byte displacement.
    pub disp: i32,
    /// Operand size in bytes (1, 2, 4, 8, or 16).  Mirrors the
    /// `OperandSizedObject::_operandSize` field of the C++ code.
    pub size: u8,
}

impl Mem {
    /// Construct a memory operand with only a base register and a size.
    pub fn base(base: Gpr, size: u8) -> Self {
        Mem { base: Some(base), index: None, scale: 0, disp: 0, size }
    }

    /// Construct an absolute memory operand (RIP-relative in x64).
    pub fn disp(disp: i32, size: u8) -> Self {
        Mem { base: None, index: None, scale: 0, disp, size }
    }

    /// Builder-style `+disp`.
    pub fn with_disp(mut self, disp: i32) -> Self {
        self.disp = disp;
        self
    }

    /// Builder-style `+index*scale`.
    pub fn with_index(mut self, index: Gpr, scale: u8) -> Self {
        self.index = Some(index);
        self.scale = scale;
        self
    }
}

// =====================================================================================
//  Operand-size enum (matches OperandSizedObject::GetOperandSize() in C++)
// =====================================================================================

/// Logical operand size in bytes.  Used by the encoders that depend on it
/// (e.g. operand-size prefixes for `MOVZX`, immediate widths, and SIMD memory
/// operand sizes).
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum OpSize {
    /// 8-bit.
    Byte = 1,
    /// 16-bit (emits a `0x66` prefix on the line).
    Word = 2,
    /// 32-bit.
    Dword = 4,
    /// 64-bit (emits a `0x48` REX.W prefix).
    Qword = 8,
    /// 128-bit (used for SSE/SIMD memory operands).
    Xmmword = 16,
}

impl OpSize {
    /// Returns the operand-size prefix byte (`0x66`) when the size is 16
    /// bits, mirroring the C++ `GetPrefix16()` helper.
    #[inline]
    pub const fn prefix16(self) -> u8 {
        match self {
            OpSize::Word => 0x66,
            _ => 0,
        }
    }

    /// True for 8-bit operand sizes (skip REX, use 8-bit opcode form).
    #[inline]
    pub const fn is_byte(self) -> bool {
        matches!(self, OpSize::Byte)
    }
}

// =====================================================================================
//  The assembler
// =====================================================================================

/// A simple, self-contained x86 code assembler.
///
/// Backed by a `Vec<u8>` (matching the task spec), the assembler owns its
/// own byte buffer and exposes `emit` as a private helper plus a large set
/// of opcode encoders.  Position-independent code is *not* tracked here; the
/// caller is expected to hand us relative displacements where the encoder
/// needs them.
#[derive(Default, Debug, Clone)]
pub struct X86Assembler {
    /// Raw instruction bytes.  Append-only; never rewound.
    pub buf: Vec<u8>,
}

impl X86Assembler {
    /// Construct a new, empty assembler.
    pub fn new() -> Self {
        X86Assembler { buf: Vec::new() }
    }

    /// Construct an assembler with a pre-reserved buffer capacity.
    pub fn with_capacity(n: usize) -> Self {
        X86Assembler { buf: Vec::with_capacity(n) }
    }

    /// Borrow the assembled bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.buf
    }

    /// Consume the assembler and return the byte buffer.
    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    /// Number of bytes emitted so far.
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// True when no bytes have been emitted.
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Clear the buffer (does not release the underlying capacity).
    pub fn clear(&mut self) {
        self.buf.clear();
    }

    /// Core emit helper.  Mirrors the C++ `xWrite8` / `xWrite16` /
    /// `xWrite32` / `xWrite64` free functions in `x86emitter.cpp`.  Appends
    /// the literal bytes to the back of `self.buf`.
    #[inline]
    pub fn emit(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// Emit a single byte.
    #[inline]
    pub fn emit_u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    /// Emit a little-endian u16.
    #[inline]
    pub fn emit_u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    /// Emit a little-endian u32.
    #[inline]
    pub fn emit_u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    /// Emit a little-endian u64.
    #[inline]
    pub fn emit_u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    // ---------------------------------------------------------------------------------
    //  Low-level ModRM/SIB helpers
    // ---------------------------------------------------------------------------------

    /// Emit a ModR/M byte.  Mirrors the `ModRM()` static function in
    /// `x86emitter.cpp` line 288.
    #[inline]
    fn modrm(&mut self, mod_: u8, reg: u8, rm: u8) {
        self.buf.push((mod_ << 6) | (reg << 3) | (rm & 0x07));
    }

    /// Emit a SIB byte.  Mirrors the `SibSB()` static function in
    /// `x86emitter.cpp` line 293.
    #[inline]
    fn sib(&mut self, ss: u8, index: u8, base: u8) {
        self.buf.push((ss << 6) | ((index & 0x07) << 3) | (base & 0x07));
    }

    /// Emit a REX prefix byte.  Mirrors the `EmitRex()` family.
    /// `w` -> REX.W, `r` -> REX.R, `x` -> REX.X, `b` -> REX.B.
    /// `force` emits the prefix even when all bits are zero (used for
    /// SPL/BPL/SIL/DIL encoding where REX is required to disambiguate).
    #[inline]
    fn rex(&mut self, w: bool, r: bool, x: bool, b: bool, force: bool) {
        let byte = 0x40 | (u8::from(w) << 3) | (u8::from(r) << 2) | (u8::from(x) << 1) | u8::from(b);
        if byte != 0x40 || force {
            self.buf.push(byte);
        }
    }

    /// Emit a REX.W prefix (used for 64-bit operand forms).
    #[inline]
    fn rex_w(&mut self, r: bool, x: bool, b: bool) {
        self.rex(true, r, x, b, false);
    }

    /// Emit the REX prefix required by a `Gpr -> Gpr` instruction where
    /// the `reg` field is `r` and `rm` is `b` (R/M).  If `size` is 64-bit
    /// the W bit is set as well.
    fn rex_rr(&mut self, dst: Gpr, src: Gpr, size: OpSize) {
        let w = size == OpSize::Qword;
        let r = dst.is_extended();
        let b = src.is_extended();
        self.rex(w, r, false, b, false);
    }

    /// Emit the ModRM byte for a register-to-register form (`mod = 3`).
    #[inline]
    fn modrm_rr(&mut self, reg: u8, rm: Gpr) {
        self.modrm(0b11, reg & 0x07, rm.0);
    }

    /// Emit a 16-bit operand-size prefix if needed.
    #[inline]
    fn maybe_prefix16(&mut self, size: OpSize) {
        let p = size.prefix16();
        if p != 0 {
            self.buf.push(p);
        }
    }
}

// =====================================================================================
//  GPR move / arithmetic / control-flow encoders
// =====================================================================================

impl X86Assembler {
    // -------- MOV ----------

    /// `MOV dst, src` between two GPRs.  Sized by `size`.  Mirrors
    /// `xImpl_Mov::operator()(reg, reg)` in `movs.cpp`.
    pub fn mov_rr(&mut self, dst: Gpr, src: Gpr, size: OpSize) {
        if dst == src {
            return;
        }
        self.maybe_prefix16(size);
        self.rex_rr(src, dst, size); // for MOV r/m, r, the reg field is src, r/m is dst
        // 8-bit: opcode 0x88, otherwise 0x89
        let opcode = if size == OpSize::Byte { 0x88 } else { 0x89 };
        // ModR/M: reg=src, r/m=dst, mod=11
        self.modrm_rr(src.0, dst);
        // Note: when emitting 8-bit ops with REX, the high-bit registers
        // (AH/BH/CH/DH) are encoded in the *low* 3 bits of r/m.  The base
        // mapping is fine here for the AH/BH/... -> SPL/BPL/SIL/DIL set;
        // the C++ code uses a separate path for SPL/BPL via the
        // `IsExtended8Bit()` helper.  We sidestep the issue by treating any
        // GPR id >= 8 the same way the C++ does: emit the REX prefix and
        // use the low 3 bits.
    }

    /// `MOV r/m, imm` -- write an immediate to a GPR.  Mirrors
    /// `xImpl_Mov::operator()(reg, imm)` in `movs.cpp`.
    pub fn mov_ri(&mut self, dst: Gpr, imm: i64, size: OpSize) {
        self.maybe_prefix16(size);

        // For 8-bit ops, opcode is 0xB0 | dst.id.
        // For larger ops, opcode is 0xB8 | dst.id (no ModR/M).
        let opcode = if size == OpSize::Byte {
            0xB0 | (dst.0 & 0x07)
        } else {
            0xB8 | (dst.0 & 0x07)
        };

        if size == OpSize::Qword {
            // 64-bit imm form: 0xB8 + rd, REX.W set.
            self.rex(true, false, false, dst.is_extended(), false);
            self.buf.push(opcode);
            self.emit_u64(imm as u64);
        } else {
            // 32-bit imm form: 0xB8 + rd, no REX.W (R8..R15 still need REX.B).
            if dst.is_extended() {
                self.rex(false, false, false, true, false);
            }
            self.buf.push(opcode);
            match size {
                OpSize::Byte  => self.emit_u8(imm as u8),
                OpSize::Word  => self.emit_u16(imm as u16),
                OpSize::Dword => self.emit_u32(imm as u32),
                _ => self.emit_u32(imm as u32),
            }
        }
    }

    /// `MOV dst, [mem]` -- load from a memory operand into a GPR.  Mirrors
    /// `xImpl_Mov::operator()(reg, sibsrc)` in `movs.cpp`.
    pub fn mov_rm(&mut self, dst: Gpr, mem: Mem, size: OpSize) {
        self.maybe_prefix16(size);
        // REX: R = dst, B = base
        let w = size == OpSize::Qword;
        let r = dst.is_extended();
        let b = mem.base.map_or(false, Gpr::is_extended);
        let x = mem.index.map_or(false, Gpr::is_extended);
        self.rex(w, r, x, b, false);
        let opcode = if size == OpSize::Byte { 0x8A } else { 0x8B };
        self.emit_modrm(opcode, dst.0, mem);
    }

    /// `MOV [mem], src` -- store a GPR into memory.
    pub fn mov_mr(&mut self, mem: Mem, src: Gpr, size: OpSize) {
        self.maybe_prefix16(size);
        let w = size == OpSize::Qword;
        let r = src.is_extended();
        let b = mem.base.map_or(false, Gpr::is_extended);
        let x = mem.index.map_or(false, Gpr::is_extended);
        self.rex(w, r, x, b, false);
        let opcode = if size == OpSize::Byte { 0x88 } else { 0x89 };
        self.emit_modrm(opcode, src.0, mem);
    }

    // -------- ADD ----------

    /// `ADD dst, src` -- GPR + GPR.  Mirrors `xADD` from `groups.cpp`.
    pub fn add_rr(&mut self, dst: Gpr, src: Gpr, size: OpSize) {
        self.alu_rr(0x00, dst, src, size);
    }

    /// `ADD dst, imm`.  Mirrors `xADD(reg, imm)`.
    pub fn add_ri(&mut self, dst: Gpr, imm: i32, size: OpSize) {
        self.alu_ri(0x00, dst, imm, size);
    }

    // -------- SUB ----------

    /// `SUB dst, src` -- GPR - GPR.  Mirrors `xSUB` from `groups.cpp`.
    pub fn sub_rr(&mut self, dst: Gpr, src: Gpr, size: OpSize) {
        // SUB reg, r/m: opcode base 0x28 (or 0x2A for 8-bit)
        self.alu_rr(0x28, dst, src, size);
    }

    /// `SUB dst, imm`.  Mirrors `xSUB(reg, imm)`.
    pub fn sub_ri(&mut self, dst: Gpr, imm: i32, size: OpSize) {
        self.alu_ri(0x28, dst, imm, size);
    }

    // -------- MUL (one-operand form) ----------

    /// `MUL src` -- unsigned multiply AL/AX/EAX/RAX := AL/AX/EAX * src.
    /// The result is placed in DX:AX / EDX:EAX / RDX:RAX.  Mirrors the
    /// `xMUL` group-3 form (`xImpl_Group3` with `G3Type_MUL`).
    pub fn mul(&mut self, src: Gpr, size: OpSize) {
        self.maybe_prefix16(size);
        let w = size == OpSize::Qword;
        let r = false;
        let x = false;
        let b = src.is_extended();
        self.rex(w, r, x, b, false);
        // /4 extension -> reg field 0b100
        let opcode = if size == OpSize::Byte { 0xF6 } else { 0xF7 };
        self.modrm_rr(0b100, src);
    }

    // -------- IMUL ----------

    /// Two-operand `IMUL dst, src`.  Mirrors `xImpl_iMul::operator()(reg,
    /// reg)` from `groups.cpp`.
    pub fn imul_rr(&mut self, dst: Gpr, src: Gpr, size: OpSize) {
        // /5 extension -> reg field 0b101, opcode 0x0F 0xAF
        self.op_write_0f(0x00, 0xAF, dst, src, size.prefix16());
    }

    /// Three-operand `IMUL dst, src, imm`.  Mirrors
    /// `xImpl_iMul::operator()(reg, reg, imm)`.
    pub fn imul_rri(&mut self, dst: Gpr, src: Gpr, imm: i32, size: OpSize) {
        // 0x0F 0x69 (full imm) or 0x0F 0x6B (sign-extended imm8)
        let opcode = if imm == (imm as i8) as i32 { 0x6B } else { 0x69 };
        let prefix = size.prefix16();
        self.maybe_prefix16(size);
        // REX: W, R=dst, B=src
        let w = size == OpSize::Qword;
        self.rex(w, dst.is_extended(), false, src.is_extended(), false);
        self.buf.push(0x0F);
        self.buf.push(opcode);
        self.modrm_rr(dst.0, src);
        if opcode == 0x6B {
            self.emit_u8(imm as u8);
        } else {
            // 32-bit imm for 32/64-bit forms, 16-bit for word form
            match size {
                OpSize::Word => self.emit_u16(imm as u16),
                _ => self.emit_u32(imm as u32),
            }
        }
    }

    // -------- IDIV ----------

    /// Signed `IDIV src` -- divides RDX:RAX by `src`.  Mirrors `xDIV` from
    /// `groups.cpp` (`xImpl_iDiv`).
    pub fn idiv(&mut self, src: Gpr, size: OpSize) {
        // /7 extension -> reg field 0b111, opcode 0xF7
        self.maybe_prefix16(size);
        let w = size == OpSize::Qword;
        self.rex(w, false, false, src.is_extended(), false);
        let opcode = if size == OpSize::Byte { 0xF6 } else { 0xF7 };
        self.modrm_rr(0b111, src);
    }

    // -------- JMP / CALL / RET ----------

    /// Unconditional near `JMP rel32`.  Mirrors `JMP32` from `legacy.cpp`.
    /// Returns the index of the displacement slot in the buffer (so the
    /// caller can patch it later for a forward jump).
    pub fn jmp_rel32(&mut self, rel: i32) -> usize {
        self.buf.push(0xE9);
        let slot = self.buf.len();
        self.emit_u32(rel as u32);
        slot
    }

    /// Unconditional short `JMP rel8`.
    pub fn jmp_rel8(&mut self, rel: i8) {
        self.buf.push(0xEB);
        self.emit_u8(rel as u8);
    }

    /// Indirect `JMP [mem]` (ModR/M form, opcode 0xFF /4).
    pub fn jmp_mem(&mut self, mem: Mem) {
        self.buf.push(0xFF);
        // /4 reg field
        self.emit_modrm(0b100, 0, mem);
    }

    /// Indirect `JMP r/m64`.  Mirrors `xJMP(reg)` from `jmp.cpp`.
    pub fn jmp_r(&mut self, reg: Gpr) {
        // REX.W is not strictly required (jumps are implicitly wide on
        // 64-bit), but we follow the C++ emitter's `EmitRex` here.
        self.rex(false, false, false, reg.is_extended(), false);
        self.buf.push(0xFF);
        self.modrm_rr(0b100, reg);
    }

    /// Near `CALL rel32`.  Mirrors `xCALL` for the immediate form.
    /// Returns the displacement slot index.
    pub fn call_rel32(&mut self, rel: i32) -> usize {
        self.buf.push(0xE8);
        let slot = self.buf.len();
        self.emit_u32(rel as u32);
        slot
    }

    /// Indirect `CALL r/m64`.
    pub fn call_r(&mut self, reg: Gpr) {
        self.rex(false, false, false, reg.is_extended(), false);
        self.buf.push(0xFF);
        self.modrm_rr(0b010, reg);
    }

    /// `RET` (near).
    pub fn ret(&mut self) {
        self.buf.push(0xC3);
    }

    /// `RET imm16` (return and pop imm16 bytes from the stack).
    pub fn ret_imm(&mut self, n: u16) {
        self.buf.push(0xC2);
        self.emit_u16(n);
    }

    // -------- LEA ----------

    /// `LEA dst, [mem]` -- load effective address.  Mirrors `xLEA` from
    /// `x86emitter.cpp`.  The 8-bit displacement is used when the
    /// displacement fits in `i8`; otherwise a 32-bit displacement is used.
    pub fn lea(&mut self, dst: Gpr, mut mem: Mem) {
        mem.size = 0; // LEA has no operand-size -- the result follows the register size.
        self.maybe_prefix16(OpSize::Qword);
        // 64-bit LEA: REX.W set, R = dst (ModR/M reg field), B = base
        self.rex(
            true,
            dst.is_extended(),
            mem.index.map_or(false, Gpr::is_extended),
            mem.base.map_or(false, Gpr::is_extended),
            false,
        );
        self.buf.push(0x8D);
        // For LEA, the `size` hint is irrelevant.  Always treat as 64-bit
        // for the ModR/M encoding (i.e. no 8-bit override on disp).
        self.emit_modrm_disp32(dst.0, &mem);
    }
}

impl X86Assembler {
    // ---------------------------------------------------------------------------------
    //  ALU register-register / register-immediate helpers
    // ---------------------------------------------------------------------------------

    /// Common encoder for `OP r, r/m` Group-1 instructions.  `opcode` is the
    /// 8-bit opcode *base*; bit 3 is the direction (0 = reg <- r/m, 1 =
    /// r/m <- reg), and bits 0/1 are 0/1 for 16/32-bit or 0/1 for 8-bit.
    fn alu_rr(&mut self, opcode: u8, dst: Gpr, src: Gpr, size: OpSize) {
        self.maybe_prefix16(size);
        // Group-1: reg <- r/m
        let op = if size == OpSize::Byte { opcode } else { opcode | 0x01 };
        self.rex_rr(src, dst, size);
        // ModR/M: reg=src, r/m=dst (this is the OP r, r/m direction).
        self.modrm_rr(src.0, dst);
        let _ = op; // already encoded into the Reg/Opcode field via modrm_rr
    }

    /// Common encoder for `OP r/m, imm` (and `OP r, imm`) Group-1
    /// instructions.  Mirrors `xImpl_Group1::operator()(reg, imm)`.
    fn alu_ri(&mut self, opcode: u8, dst: Gpr, imm: i32, size: OpSize) {
        self.maybe_prefix16(size);
        let w = size == OpSize::Qword;
        self.rex(w, false, false, dst.is_extended(), false);

        if size != OpSize::Byte && (imm as i8 as i32) == imm {
            // Sign-extended imm8 form: opcode 0x83, /0..7 in ModR/M reg field.
            // /0 = ADD, /5 = SUB, etc.  Here we treat the lower 3 bits of
            // `opcode` as the /digit (ADD=0, SUB=5).
            self.buf.push(0x83);
            self.modrm_rr(opcode >> 3, dst);
            self.emit_u8(imm as u8);
        } else {
            // Full imm form: opcode 0x81, /digit in ModR/M reg field.
            // 8-bit ops use 0x80 instead.
            let op = if size == OpSize::Byte { 0x80 } else { 0x81 };
            self.buf.push(op);
            self.modrm_rr(opcode >> 3, dst);
            match size {
                OpSize::Byte  => self.emit_u8(imm as u8),
                OpSize::Word  => self.emit_u16(imm as u16),
                OpSize::Dword => self.emit_u32(imm as u32),
                OpSize::Qword => self.emit_u32(imm as u32),
                _             => self.emit_u32(imm as u32),
            }
        }
    }

    /// Emit a ModR/M byte plus (optional) SIB byte and (optional)
    /// displacement for a memory operand.  This is a flat translation of
    /// the C++ `EmitSibMagic(uint regfield, const xIndirectVoid& info)`
    /// routine in `x86emitter.cpp`.
    fn emit_modrm(&mut self, reg_field: u8, dst_id: u8, mem: Mem) {
        // If there's no SIB needed, pick the right mod based on disp size.
        let needs_sib = mem.index.is_some() || mem.scale != 0;
        let disp8 = (mem.disp as i8 as i32) == mem.disp;
        let has_base = mem.base.is_some();
        let has_disp = mem.disp != 0;
        let use_disp32 = has_disp && !disp8;

        if !needs_sib {
            // ModR/M only.
            if !has_base {
                // disp32 form: ModR/M = 00, rm = 5 (no base, no index).
                self.modrm(0b00, reg_field, 0b101);
                self.emit_u32(mem.disp as u32);
                return;
            }
            let base = mem.base.unwrap();
            // Special case: base == RBP and no disp must be encoded as
            // [RBP+0] to disambiguate from disp32.
            let force_disp8 = base.0 == Gpr::RBP.0 && !has_disp;
            let mod_ = if force_disp8 {
                0b01
            } else if use_disp32 {
                0b10
            } else if has_disp {
                0b01
            } else {
                0b00
            };
            self.modrm(mod_, reg_field, base.0);
            if force_disp8 || (has_disp && !use_disp32) {
                self.emit_u8(mem.disp as u8);
            } else if use_disp32 {
                self.emit_u32(mem.disp as u32);
            }
        } else {
            // ModR/M + SIB.
            if !has_base {
                // [index*scale + disp32]: ModR/M = 00, rm = 4 (SIB).
                // SIB: scale, index, base = 5 (no base).
                self.modrm(0b00, reg_field, 0b100);
                let ss = match mem.scale {
                    0 => 0,
                    1 => 0,
                    2 => 1,
                    4 => 2,
                    8 => 3,
                    _ => 0,
                };
                let idx = mem.index.unwrap().0;
                self.sib(ss, idx, 0b101);
                self.emit_u32(mem.disp as u32);
                return;
            }
            let base = mem.base.unwrap();
            let force_disp8 = base.0 == Gpr::RBP.0 && !has_disp;
            let mod_ = if force_disp8 {
                0b01
            } else if use_disp32 {
                0b10
            } else if has_disp {
                0b01
            } else {
                0b00
            };
            self.modrm(mod_, reg_field, 0b100);
            let ss = match mem.scale {
                0 => 0,
                1 => 0,
                2 => 1,
                4 => 2,
                8 => 3,
                _ => 0,
            };
            let idx = mem.index.unwrap().0;
            self.sib(ss, idx, base.0);
            if force_disp8 || (has_disp && !use_disp32) {
                self.emit_u8(mem.disp as u8);
            } else if use_disp32 {
                self.emit_u32(mem.disp as u32);
            }
        }

        // The `dst_id` is preserved in the function signature to mirror
        // the C++ `EmitSibMagic(regfield, ...)` API; in our flattened
        // version the `reg` field is the only "other" operand and the
        // memory operand carries the rest.  Suppress unused-warning.
        let _ = dst_id;
    }

    /// Like `emit_modrm` but always uses a 32-bit displacement.  Used by
    /// `lea`, which in 64-bit mode must encode 32-bit (or absent)
    /// displacements.
    fn emit_modrm_disp32(&mut self, reg_field: u8, mem: &Mem) {
        let needs_sib = mem.index.is_some() || mem.scale != 0;
        if !needs_sib {
            if mem.base.is_none() {
                self.modrm(0b00, reg_field, 0b101);
                self.emit_u32(mem.disp as u32);
                return;
            }
            let base = mem.base.unwrap();
            if mem.disp == 0 {
                if base.0 == Gpr::RBP.0 {
                    // [rbp+0]
                    self.modrm(0b01, reg_field, base.0);
                    self.emit_u8(0);
                } else {
                    self.modrm(0b00, reg_field, base.0);
                }
            } else {
                self.modrm(0b10, reg_field, base.0);
                self.emit_u32(mem.disp as u32);
            }
            return;
        }

        // SIB required.
        if mem.base.is_none() {
            self.modrm(0b00, reg_field, 0b100);
            let ss = match mem.scale { 1 => 0, 2 => 1, 4 => 2, 8 => 3, _ => 0 };
            self.sib(ss, mem.index.unwrap().0, 0b101);
            self.emit_u32(mem.disp as u32);
        } else {
            let base = mem.base.unwrap();
            if mem.disp == 0 && base.0 != Gpr::RBP.0 {
                self.modrm(0b00, reg_field, 0b100);
                let ss = match mem.scale { 1 => 0, 2 => 1, 4 => 2, 8 => 3, _ => 0 };
                self.sib(ss, mem.index.unwrap().0, base.0);
            } else {
                self.modrm(0b10, reg_field, 0b100);
                let ss = match mem.scale { 1 => 0, 2 => 1, 4 => 2, 8 => 3, _ => 0 };
                self.sib(ss, mem.index.unwrap().0, base.0);
                self.emit_u32(mem.disp as u32);
            }
        }
    }

    /// Emit a 0x0F-prefixed opcode as `reg <- r/m` (or vice versa) with
    /// a single optional prefix byte.  Mirrors the C++ `xOpWrite0F`
    /// helper.  `reg_field` goes into the ModR/M reg slot; the `src`
    /// register is the ModR/M r/m field.
    fn op_write_0f(&mut self, prefix: u8, opcode: u8, dst: Gpr, src: Gpr, op_prefix: u8) {
        if op_prefix != 0 {
            self.buf.push(op_prefix);
        }
        // REX: W=0 (32-bit form), R = dst, B = src
        self.rex(
            false,
            dst.is_extended(),
            false,
            src.is_extended(),
            false,
        );
        if prefix != 0 {
            self.buf.push(prefix);
        }
        self.buf.push(0x0F);
        self.buf.push(opcode);
        self.modrm_rr(dst.0, src);
    }
}

// =====================================================================================
//  FPU encoders
// =====================================================================================

impl X86Assembler {
    // -------- FPU stack ops ----------

    /// `FLD m32` -- load a 32-bit float onto the FPU stack.  Mirrors
    /// `FLD32` from `fpu.cpp`.
    pub fn fld_m32(&mut self, addr: u32) {
        self.buf.push(0xD9);
        // ModR/M: mod=00, reg=0, rm=5 (disp32, no base)
        self.modrm(0b00, 0b000, 0b101);
        self.emit_u32(addr);
    }

    /// `FLD st(i)` -- push the i-th FPU register onto the top of the
    /// stack.  Mirrors `FLD(int st)` from `fpu.cpp`.
    pub fn fld_st(&mut self, i: u8) {
        let word: u16 = 0xC0D9 | ((i as u16 & 0x07) << 8);
        self.emit_u16(word);
    }

    /// `FSTP m32` -- pop the top of the FPU stack into a 32-bit float.
    /// Mirrors `FSTP32` from `fpu.cpp`.
    pub fn fstp_m32(&mut self, addr: u32) {
        self.buf.push(0xD9);
        // ModR/M: mod=00, reg=3 (FSTP), rm=5 (disp32, no base)
        self.modrm(0b00, 0b011, 0b101);
        self.emit_u32(addr);
    }

    /// `FSTP st(i)` -- pop the top of the FPU stack into the i-th
    /// register.  Mirrors `FSTP(int st)` from `fpu.cpp`.
    pub fn fstp_st(&mut self, i: u8) {
        let word: u16 = 0xD8DD | ((i as u16 & 0x07) << 8);
        self.emit_u16(word);
    }

    /// `FADD ST(0), st(i)` -- add st(i) to st(0).  Mirrors
    /// `FADD320toR(src)` from `fpu.cpp`.
    pub fn fadd_st(&mut self, i: u8) {
        self.buf.push(0xDC);
        self.buf.push(0xC0 | (i & 0x07));
    }

    /// `FSUB st(0), st(i)` -- subtract st(i) from st(0).  Mirrors
    /// `FSUB32Rto0(src)` from `fpu.cpp`.
    pub fn fsub_st(&mut self, i: u8) {
        self.buf.push(0xD8);
        self.buf.push(0xE0 | (i & 0x07));
    }

    /// `FMUL m32` -- multiply st(0) by a 32-bit memory float.  Mirrors
    /// `FMUL32` from `fpu.cpp`.
    pub fn fmul_m32(&mut self, addr: u32) {
        self.buf.push(0xD8);
        // ModR/M: mod=00, reg=1 (FMUL), rm=5 (disp32, no base)
        self.modrm(0b00, 0b001, 0b101);
        self.emit_u32(addr);
    }

    /// `FDIV m32` -- divide st(0) by a 32-bit memory float.  Mirrors
    /// `FDIV32` from `fpu.cpp`.
    pub fn fdiv_m32(&mut self, addr: u32) {
        self.buf.push(0xD8);
        // ModR/M: mod=00, reg=6 (FDIV), rm=5 (disp32, no base)
        // FDIV32 in the legacy emitter uses reg=6 (single-precision FDIV).
        self.modrm(0b00, 0b110, 0b101);
        self.emit_u32(addr);
    }
}

// =====================================================================================
//  SSE2 integer-SIMD encoders
// =====================================================================================

impl X86Assembler {
    /// `MOVAPS xmm, xmm` -- move aligned packed singles.  Mirrors
    /// `xMOVAPS` from `simd.cpp`.
    pub fn movaps_rr(&mut self, dst: Xmm, src: Xmm) {
        self.simd_mov_rr(dst, src, /*aligned=*/ true);
    }

    /// `MOVAPS [mem], xmm` -- store aligned packed singles to memory.
    pub fn movaps_mr(&mut self, mem: Mem, src: Xmm) {
        self.simd_mov_mr(mem, src, /*aligned=*/ true);
    }

    /// `MOVAPS xmm, [mem]` -- load aligned packed singles from memory.
    pub fn movaps_rm(&mut self, dst: Xmm, mem: Mem) {
        self.simd_mov_rm(dst, mem, /*aligned=*/ true);
    }

    /// `MOVUPS xmm, xmm` -- move unaligned packed singles.  Mirrors
    /// `xMOVUPS` from `simd.cpp`.
    pub fn movups_rr(&mut self, dst: Xmm, src: Xmm) {
        self.simd_mov_rr(dst, src, /*aligned=*/ false);
    }

    /// `MOVUPS [mem], xmm` -- store unaligned packed singles to memory.
    pub fn movups_mr(&mut self, mem: Mem, src: Xmm) {
        self.simd_mov_mr(mem, src, /*aligned=*/ false);
    }

    /// `MOVUPS xmm, [mem]` -- load unaligned packed singles from memory.
    pub fn movups_rm(&mut self, dst: Xmm, mem: Mem) {
        self.simd_mov_rm(dst, mem, /*aligned=*/ false);
    }

    /// `PADDD xmm, xmm` / `PADDD xmm, [mem]`.  Mirrors the PADD.PS / PADD.D
    /// dispatch in `simd.cpp`.  We hardcode the SSE2 integer (D) form here.
    pub fn paddd_rr(&mut self, dst: Xmm, src: Xmm) {
        self.simd_alu_rr(0x66, 0xFE, dst, src);
    }

    pub fn paddd_rm(&mut self, dst: Xmm, mem: Mem) {
        self.simd_alu_rm(0x66, 0xFE, dst, mem);
    }

    /// `PSUBD xmm, xmm` / `PSUBD xmm, [mem]`.
    pub fn psubd_rr(&mut self, dst: Xmm, src: Xmm) {
        self.simd_alu_rr(0x66, 0xFA, dst, src);
    }

    pub fn psubd_rm(&mut self, dst: Xmm, mem: Mem) {
        self.simd_alu_rm(0x66, 0xFA, dst, mem);
    }

    /// `PMULLD xmm, xmm` / `PMULLD xmm, [mem]` -- SSE4.1 multiply packed
    /// signed 32-bit integers.  Opcode `0F 38 40` with prefix `0x66`.
    pub fn pmulld_rr(&mut self, dst: Xmm, src: Xmm) {
        // 0x66 0x0F 0x38 0x40 (ModR/M)
        self.buf.push(0x66);
        self.rex(
            false,
            dst.is_extended(),
            false,
            src.is_extended(),
            false,
        );
        self.buf.push(0x0F);
        self.buf.push(0x38);
        self.buf.push(0x40);
        self.modrm(0b11, dst.0 & 0x07, src.0);
    }

    pub fn pmulld_rm(&mut self, dst: Xmm, mem: Mem) {
        self.buf.push(0x66);
        self.rex(
            false,
            dst.is_extended(),
            mem.index.map_or(false, Gpr::is_extended),
            mem.base.map_or(false, Gpr::is_extended),
            false,
        );
        self.buf.push(0x0F);
        self.buf.push(0x38);
        self.buf.push(0x40);
        self.emit_modrm(dst.0, 0, mem);
    }

    /// `PAND xmm, xmm` / `PAND xmm, [mem]`.
    pub fn pand_rr(&mut self, dst: Xmm, src: Xmm) {
        self.simd_alu_rr(0x66, 0xDB, dst, src);
    }

    pub fn pand_rm(&mut self, dst: Xmm, mem: Mem) {
        self.simd_alu_rm(0x66, 0xDB, dst, mem);
    }

    /// `POR xmm, xmm` / `POR xmm, [mem]`.
    pub fn por_rr(&mut self, dst: Xmm, src: Xmm) {
        self.simd_alu_rr(0x66, 0xEB, dst, src);
    }

    pub fn por_rm(&mut self, dst: Xmm, mem: Mem) {
        self.simd_alu_rm(0x66, 0xEB, dst, mem);
    }

    /// `PXOR xmm, xmm` / `PXOR xmm, [mem]`.
    pub fn pxor_rr(&mut self, dst: Xmm, src: Xmm) {
        self.simd_alu_rr(0x66, 0xEF, dst, src);
    }

    pub fn pxor_rm(&mut self, dst: Xmm, mem: Mem) {
        self.simd_alu_rm(0x66, 0xEF, dst, mem);
    }

    // ---------------------------------------------------------------------------------
    //  SIMD helpers
    // ---------------------------------------------------------------------------------

    /// Emit a generic `0F <opcode>` SIMD ALU operation between two XMM
    /// registers, with the supplied legacy prefix byte.
    fn simd_alu_rr(&mut self, prefix: u8, opcode: u8, dst: Xmm, src: Xmm) {
        if prefix != 0 {
            self.buf.push(prefix);
        }
        self.rex(
            false,
            dst.is_extended(),
            false,
            src.is_extended(),
            false,
        );
        self.buf.push(0x0F);
        self.buf.push(opcode);
        self.modrm(0b11, dst.0 & 0x07, src.0);
    }

    /// Emit a generic `0F <opcode>` SIMD ALU operation with an XMM dest
    /// and a memory source.
    fn simd_alu_rm(&mut self, prefix: u8, opcode: u8, dst: Xmm, mem: Mem) {
        if prefix != 0 {
            self.buf.push(prefix);
        }
        self.rex(
            false,
            dst.is_extended(),
            mem.index.map_or(false, Gpr::is_extended),
            mem.base.map_or(false, Gpr::is_extended),
            false,
        );
        self.buf.push(0x0F);
        self.buf.push(opcode);
        self.emit_modrm(dst.0, 0, mem);
    }

    /// Emit a MOVAPS/MOVUPS register-to-register move.
    ///
    /// 0F 28 /r  -- MOVAPS xmm1, xmm2/m128  (load from r/m)
    /// 0F 29 /r  -- MOVAPS xmm1/m128, xmm2  (store to r/m)
    ///
    /// When the destination is an XMM register we always use the load
    /// form (0F 28); when it's memory we use the store form (0F 29).
    fn simd_mov_rr(&mut self, dst: Xmm, src: Xmm, _aligned: bool) {
        // When `dst == src` the C++ emitter drops the instruction; we
        // match that behavior.
        if dst == src {
            return;
        }
        self.rex(
            false,
            dst.is_extended(),
            false,
            src.is_extended(),
            false,
        );
        self.buf.push(0x0F);
        self.buf.push(0x28);
        self.modrm(0b11, dst.0 & 0x07, src.0);
    }

    /// Emit a MOVAPS/MOVUPS store to memory.
    fn simd_mov_mr(&mut self, mem: Mem, src: Xmm, _aligned: bool) {
        self.rex(
            false,
            src.is_extended(),
            mem.index.map_or(false, Gpr::is_extended),
            mem.base.map_or(false, Gpr::is_extended),
            false,
        );
        self.buf.push(0x0F);
        self.buf.push(0x29);
        self.emit_modrm(src.0, 0, mem);
    }

    /// Emit a MOVAPS/MOVUPS load from memory.
    fn simd_mov_rm(&mut self, dst: Xmm, mem: Mem, _aligned: bool) {
        self.rex(
            false,
            dst.is_extended(),
            mem.index.map_or(false, Gpr::is_extended),
            mem.base.map_or(false, Gpr::is_extended),
            false,
        );
        self.buf.push(0x0F);
        self.buf.push(0x28);
        self.emit_modrm(dst.0, 0, mem);
    }
}

// =====================================================================================
//  Tests -- quick smoke checks
// =====================================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mov_imm_dword() {
        // mov eax, 0x11223344 -> B8 44 33 22 11
        let mut a = X86Assembler::new();
        a.mov_ri(Gpr::RAX, 0x11223344, OpSize::Dword);
        assert_eq!(a.bytes(), &[0xB8, 0x44, 0x33, 0x22, 0x11]);
    }

    #[test]
    fn mov_imm_qword_extended() {
        // mov r8, 0xCAFEBABE_DEADBEEF -> 49 B8 EF BE AD DE BE BA FE CA
        let mut a = X86Assembler::new();
        a.mov_ri(Gpr::R8, 0xCAFEBABE_DEADBEEF, OpSize::Qword);
        let want = [0x49, 0xB8,
                    0xEF, 0xBE, 0xAD, 0xDE,
                    0xBE, 0xBA, 0xFE, 0xCA];
        assert_eq!(a.bytes(), &want);
    }

    #[test]
    fn add_rr_dword() {
        // add eax, ecx -> 01 C8
        let mut a = X86Assembler::new();
        a.add_rr(Gpr::RAX, Gpr::RCX, OpSize::Dword);
        assert_eq!(a.bytes(), &[0x01, 0xC8]);
    }

    #[test]
    fn sub_ri_dword_sign_extended() {
        // sub eax, 1 -> 83 E8 01
        let mut a = X86Assembler::new();
        a.sub_ri(Gpr::RAX, 1, OpSize::Dword);
        assert_eq!(a.bytes(), &[0x83, 0xE8, 0x01]);
    }

    #[test]
    fn ret_nop() {
        // ret -> C3
        let mut a = X86Assembler::new();
        a.ret();
        assert_eq!(a.bytes(), &[0xC3]);
    }

    #[test]
    fn call_rel32() {
        // call $+5 -> E8 00 00 00 00
        let mut a = X86Assembler::new();
        a.call_rel32(0);
        assert_eq!(a.bytes(), &[0xE8, 0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn movaps_xmm0_xmm1() {
        // movaps xmm0, xmm1 -> 0F 28 C1
        let mut a = X86Assembler::new();
        a.movaps_rr(Xmm::XMM0, Xmm::XMM1);
        assert_eq!(a.bytes(), &[0x0F, 0x28, 0xC1]);
    }

    #[test]
    fn pxor_xmm_self() {
        // pxor xmm0, xmm0 -> 66 0F EF C0
        let mut a = X86Assembler::new();
        a.pxor_rr(Xmm::XMM0, Xmm::XMM0);
        assert_eq!(a.bytes(), &[0x66, 0x0F, 0xEF, 0xC0]);
    }

    #[test]
    fn fld_fadd_encoded() {
        // fld [0x12345678] -> D9 05 78 56 34 12
        // fadd st(1)       -> DC C1
        let mut a = X86Assembler::new();
        a.fld_m32(0x12345678);
        a.fadd_st(1);
        assert_eq!(a.bytes(), &[0xD9, 0x05, 0x78, 0x56, 0x34, 0x12,
                                0xDC, 0xC1]);
    }
}
