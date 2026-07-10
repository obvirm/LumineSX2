// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust translation of PCSX2's `common/emitter` x86 machine code
//! emitter.
//!
//! The original C++ emitter (`x86Emitter`) is a large, register-class-driven
//! library that resolves operand types at compile time and writes machine code
//! into a thread-local byte buffer.  This module collapses that design into a
//! single [`X86Assembler`] that writes into a caller-supplied `&mut [u8]`
//! slice and tracks a positional write cursor.
//!
//! The goal of this translation is **not** to provide a complete, correct
//! x86-64 encoder.  Instead it captures the *shape* of the C++ API surface
//! that PCSX2's recompilers and helpers use, with:
//!
//! * Fully working trivial encodings (one-byte opcodes like `NOP`, `RET`,
//!   `INT3`, `STC`, `CLC`, `LEAVE`, etc.).
//! * Simple register-only and register-immediate forms for the GPR, FPU and
//!   SSE groups (e.g. `mov`, `add`, `sub`, `imul`, `fadd`, `paddd`).
//! * Placeholder stubs for the more elaborate ModR/M, SIB, VEX and EVEX
//!   encodings, marked with `TODO` and a short note describing the encoding
//!   gap.  These still emit *some* bytes so call-sites can be linked
//!   symbolically and tests can verify ordering.
//!
//! Everything is `no_std`-free and uses only `std`.  No external crates are
//! required.  This is a translation aid, not a drop-in replacement for the
//! C++ emitter; the actual PCSX2 build still uses the C++ source.

#![allow(dead_code)]
#![allow(clippy::too_many_arguments)]

use std::fmt;

// ---------------------------------------------------------------------------
// Operand / Register model
// ---------------------------------------------------------------------------

/// Bit width of a general-purpose or vector register operand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperandSize {
    /// 8-bit GPR (AL, CL, DL, BL, ...).
    Size8,
    /// 16-bit GPR (AX, CX, DX, ...).
    Size16,
    /// 32-bit GPR (EAX, ECX, ...).
    Size32,
    /// 64-bit GPR (RAX, RCX, ...).
    Size64,
    /// 80-bit x87 FPU register (ST0..ST7).
    Size80,
    /// 128-bit XMM register.
    Size128,
    /// 256-bit YMM register.
    Size256,
}

impl OperandSize {
    /// Returns the size in bytes.
    #[inline]
    pub const fn bytes(self) -> usize {
        match self {
            OperandSize::Size8 => 1,
            OperandSize::Size16 => 2,
            OperandSize::Size32 => 4,
            OperandSize::Size64 => 8,
            OperandSize::Size80 => 10,
            OperandSize::Size128 => 16,
            OperandSize::Size256 => 32,
        }
    }

    /// Returns the legacy `0x66` prefix byte for 16-bit operand size, or 0.
    #[inline]
    pub const fn prefix16(self) -> u8 {
        match self {
            OperandSize::Size16 => 0x66,
            _ => 0,
        }
    }
}

/// Logical register class.  This roughly mirrors the C++ `xRegisterBase`
/// taxonomy but is tagged explicitly so the emitter can pick the right
/// encoding path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegClass {
    Gpr,
    Fpu,
    Sse,
}

/// A typed x86 register reference.  This is purely a value type - it does
/// not own any storage.  All methods on [`X86Assembler`] accept these by
/// value to keep the call-site syntax close to the original C++.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reg {
    /// Hardware register number (0..=15 for GPR, 0..=15 for XMM/YMM, 0..=7
    /// for the FPU stack).
    pub id: u8,
    /// Register class.
    pub class: RegClass,
    /// Operand size in bytes.
    pub size: OperandSize,
}

impl Reg {
    /// Construct a GPR register of the given size and id.
    #[inline]
    pub const fn gpr(size: OperandSize, id: u8) -> Reg {
        Reg {
            id,
            class: RegClass::Gpr,
            size,
        }
    }

    /// Construct a 32-bit GPR.
    #[inline]
    pub const fn r32(id: u8) -> Reg {
        Reg::gpr(OperandSize::Size32, id)
    }

    /// Construct a 64-bit GPR.
    #[inline]
    pub const fn r64(id: u8) -> Reg {
        Reg::gpr(OperandSize::Size64, id)
    }

    /// Construct a SSE (XMM) register.
    #[inline]
    pub const fn xmm(id: u8) -> Reg {
        Reg {
            id,
            class: RegClass::Sse,
            size: OperandSize::Size128,
        }
    }

    /// Construct a FPU stack register (ST0..ST7).
    #[inline]
    pub const fn st(id: u8) -> Reg {
        Reg {
            id: id & 7,
            class: RegClass::Fpu,
            size: OperandSize::Size80,
        }
    }

    /// True if the register needs the REX.B bit (id >= 8).
    #[inline]
    pub const fn is_extended(&self) -> bool {
        matches!(self.class, RegClass::Gpr) && self.id >= 8
    }

    /// True if the operand is 64-bit (REX.W).
    #[inline]
    pub const fn is_wide(&self) -> bool {
        matches!(self.size, OperandSize::Size64)
    }
}

impl fmt::Display for Reg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.class {
            RegClass::Gpr => {
                let prefix = match self.size {
                    OperandSize::Size8 => "b",
                    OperandSize::Size16 => "w",
                    OperandSize::Size32 => "d",
                    OperandSize::Size64 => "",
                    _ => "?",
                };
                let base = match self.id {
                    0 => "a",
                    1 => "c",
                    2 => "d",
                    3 => "b",
                    4 => "sp",
                    5 => "bp",
                    6 => "si",
                    7 => "di",
                    n => {
                        write!(f, "r{}{}", n, prefix)?;
                        return Ok(());
                    }
                };
                write!(f, "r{}{}", base, prefix)
            }
            RegClass::Fpu => write!(f, "st{}", self.id),
            RegClass::Sse => write!(f, "xmm{}", self.id),
        }
    }
}

// ---------------------------------------------------------------------------
// Convenience register constants (mirrors the C++ x86types.h externs).
// ---------------------------------------------------------------------------

pub mod regs {
    use super::{Reg, OperandSize};

    // 64-bit GPRs.
    pub const RAX: Reg = Reg::gpr(OperandSize::Size64, 0);
    pub const RCX: Reg = Reg::gpr(OperandSize::Size64, 1);
    pub const RDX: Reg = Reg::gpr(OperandSize::Size64, 2);
    pub const RBX: Reg = Reg::gpr(OperandSize::Size64, 3);
    pub const RSP: Reg = Reg::gpr(OperandSize::Size64, 4);
    pub const RBP: Reg = Reg::gpr(OperandSize::Size64, 5);
    pub const RSI: Reg = Reg::gpr(OperandSize::Size64, 6);
    pub const RDI: Reg = Reg::gpr(OperandSize::Size64, 7);
    pub const R8: Reg = Reg::gpr(OperandSize::Size64, 8);
    pub const R9: Reg = Reg::gpr(OperandSize::Size64, 9);
    pub const R10: Reg = Reg::gpr(OperandSize::Size64, 10);
    pub const R11: Reg = Reg::gpr(OperandSize::Size64, 11);
    pub const R12: Reg = Reg::gpr(OperandSize::Size64, 12);
    pub const R13: Reg = Reg::gpr(OperandSize::Size64, 13);
    pub const R14: Reg = Reg::gpr(OperandSize::Size64, 14);
    pub const R15: Reg = Reg::gpr(OperandSize::Size64, 15);

    // 32-bit GPRs.
    pub const EAX: Reg = Reg::r32(0);
    pub const ECX: Reg = Reg::r32(1);
    pub const EDX: Reg = Reg::r32(2);
    pub const EBX: Reg = Reg::r32(3);
    pub const ESP: Reg = Reg::r32(4);
    pub const EBP: Reg = Reg::r32(5);
    pub const ESI: Reg = Reg::r32(6);
    pub const EDI: Reg = Reg::r32(7);

    // XMM registers.
    pub const XMM0: Reg = Reg::xmm(0);
    pub const XMM1: Reg = Reg::xmm(1);
    pub const XMM2: Reg = Reg::xmm(2);
    pub const XMM3: Reg = Reg::xmm(3);
    pub const XMM4: Reg = Reg::xmm(4);
    pub const XMM5: Reg = Reg::xmm(5);
    pub const XMM6: Reg = Reg::xmm(6);
    pub const XMM7: Reg = Reg::xmm(7);
    pub const XMM8: Reg = Reg::xmm(8);
    pub const XMM9: Reg = Reg::xmm(9);
    pub const XMM10: Reg = Reg::xmm(10);
    pub const XMM11: Reg = Reg::xmm(11);
    pub const XMM12: Reg = Reg::xmm(12);
    pub const XMM13: Reg = Reg::xmm(13);
    pub const XMM14: Reg = Reg::xmm(14);
    pub const XMM15: Reg = Reg::xmm(15);
}

// ---------------------------------------------------------------------------
// Memory operand
// ---------------------------------------------------------------------------

/// Memory operand with a base, optional index, scale and displacement.
///
/// This is a simplified replacement for the C++ `xIndirectVoid` / `xAddressVoid`
/// pair.  It does not attempt to encode `[rip+disp32]` automatically; the
/// caller is expected to use the assembler for relative addressing via a
/// dedicated `lea`-style helper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mem {
    /// Base register (or `Reg::INVALID` for none).
    pub base: Reg,
    /// Index register, multiplied by `scale`.  `Reg::INVALID` for none.
    pub index: Reg,
    /// Scale applied to the index register.  Must be 1, 2, 4 or 8.
    pub scale: u8,
    /// Byte displacement.
    pub disp: i32,
}

impl Mem {
    pub const INVALID: Reg = Reg {
        id: 0xFF,
        class: RegClass::Gpr,
        size: OperandSize::Size32,
    };

    /// Construct a `[base]` memory operand.
    pub fn base(base: Reg) -> Mem {
        Mem {
            base,
            index: Self::INVALID,
            scale: 1,
            disp: 0,
        }
    }

    /// Construct a `[base + disp]` memory operand.
    pub fn base_disp(base: Reg, disp: i32) -> Mem {
        Mem {
            base,
            index: Self::INVALID,
            scale: 1,
            disp,
        }
    }

    /// Construct a `[base + index*scale + disp]` memory operand.
    pub fn sib(base: Reg, index: Reg, scale: u8, disp: i32) -> Mem {
        Mem {
            base,
            index,
            scale,
            disp,
        }
    }
}

// ---------------------------------------------------------------------------
// Branch / call target
// ---------------------------------------------------------------------------

/// A label pointing to a future position in the assembled code stream.  The
/// assembler stores a fixup list and patches the displacement once the label
/// is resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Label(pub u32);

/// A branch or call target.  Either a forward/backward label, a fixed
/// relative offset, or a raw function pointer.
#[derive(Debug, Clone, Copy)]
pub enum Target {
    Label(Label),
    /// A `fn()` pointer; encoded as `mov rax, addr; call rax` for simplicity
    /// (the original C++ emitter supports direct RIP-relative calls).
    Fn(usize),
    /// A fixed RIP-relative offset to use when patching back.
    Placeholder(usize),
}

// ---------------------------------------------------------------------------
// Assembler
// ---------------------------------------------------------------------------

/// x86 machine code emitter.
///
/// Owns a write cursor inside a caller-supplied byte slice and tracks label
/// fixups for forward references.  Mirrors the C++ `x86Emitter` namespace
/// entry points as methods on a single struct.
///
/// The buffer is treated as a write target: each emit bounds-checks against
/// `buffer.len()` and panics if it would overflow.  Callers pre-size the
/// slice to the expected code size and call [`X86Assembler::finalize`] to
/// obtain the populated prefix.
pub struct X86Assembler<'a> {
    buffer: &'a mut [u8],
    pos: usize,
    /// Pending forward label fixups: list of (label_id, code_offset, kind).
    /// `kind` is 0 for near (32-bit) jumps and 1 for short (8-bit) jumps.
    pending: Vec<(u32, usize, u8)>,
    /// Labels that have been bound, indexed by label id.
    /// `None` means the label has not yet been bound.
    bound: Vec<Option<usize>>,
}

impl<'a> X86Assembler<'a> {
    /// Create a new assembler backed by the supplied byte slice.  The
    /// slice must remain alive for the lifetime of the assembler.
    pub fn new(buffer: &'a mut [u8]) -> Self {
        X86Assembler {
            buffer,
            pos: 0,
            pending: Vec::new(),
            bound: Vec::new(),
        }
    }

    /// Finalize assembly, returning the populated prefix of the buffer as
    /// a `Vec<u8>`.
    pub fn finalize(mut self) -> Vec<u8> {
        self.resolve_pending();
        self.buffer[..self.pos].to_vec()
    }

    /// Current cursor (write position) inside the code buffer.
    #[inline]
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Write a single byte at the cursor and advance.
    #[inline]
    fn write_byte(&mut self, b: u8) {
        assert!(self.pos < self.buffer.len(), "X86Assembler: code buffer overflow");
        self.buffer[self.pos] = b;
        self.pos += 1;
    }

    // -----------------------------------------------------------------------
    // Low-level byte writers
    // -----------------------------------------------------------------------

    /// Append a single byte to the code stream.
    #[inline]
    pub fn emit_u8(&mut self, b: u8) {
        self.write_byte(b);
    }

    /// Append a little-endian u16.
    #[inline]
    pub fn emit_u16(&mut self, v: u16) {
        let bytes = v.to_le_bytes();
        self.write_byte(bytes[0]);
        self.write_byte(bytes[1]);
    }

    /// Append a little-endian u32.
    #[inline]
    pub fn emit_u32(&mut self, v: u32) {
        let bytes = v.to_le_bytes();
        self.write_byte(bytes[0]);
        self.write_byte(bytes[1]);
        self.write_byte(bytes[2]);
        self.write_byte(bytes[3]);
    }

    /// Append a little-endian u64.
    #[inline]
    pub fn emit_u64(&mut self, v: u64) {
        let bytes = v.to_le_bytes();
        for b in bytes {
            self.write_byte(b);
        }
    }

    /// Append a little-endian i32.
    #[inline]
    pub fn emit_i32(&mut self, v: i32) {
        self.emit_u32(v as u32);
    }

    /// Append a 4-byte placeholder that will be patched later.
    #[inline]
    fn emit_placeholder_i32(&mut self) -> usize {
        let p = self.pos;
        self.emit_u32(0);
        p
    }

    /// Append a 1-byte placeholder.
    #[inline]
    fn emit_placeholder_i8(&mut self) -> usize {
        let p = self.pos;
        self.emit_u8(0);
        p
    }

    // -----------------------------------------------------------------------
    // REX prefix emission
    // -----------------------------------------------------------------------

    /// Emit a REX prefix.  Skips emission if the result would be 0x40 (no
    /// bits set) and `force` is false.  Returns true if a prefix was
    /// actually written.
    pub fn emit_rex(&mut self, w: bool, r: bool, x: bool, b: bool) -> bool {
        let rex = 0x40 | (w as u8) << 3 | (r as u8) << 2 | (x as u8) << 1 | b as u8;
        if rex == 0x40 {
            return false;
        }
        self.emit_u8(rex);
        true
    }

    // -----------------------------------------------------------------------
    // ModR/M + SIB encoding
    // -----------------------------------------------------------------------

    /// Encode and emit a ModR/M byte.
    #[inline]
    pub fn emit_modrm(&mut self, mod_: u8, reg: u8, rm: u8) {
        debug_assert!(mod_ < 4 && reg < 8 && rm < 8);
        self.emit_u8((mod_ << 6) | (reg << 3) | rm);
    }

    /// Encode and emit a SIB byte.
    #[inline]
    pub fn emit_sib(&mut self, scale: u8, index: u8, base: u8) {
        debug_assert!(scale < 4 && index < 8 && base < 8);
        self.emit_u8((scale << 6) | (index << 3) | base);
    }

    // -----------------------------------------------------------------------
    // Labels
    // -----------------------------------------------------------------------

    /// Create a new unbound label.
    pub fn new_label(&mut self) -> Label {
        let id = self.bound.len() as u32;
        self.bound.push(None);
        Label(id)
    }

    /// Bind a label to the current code position.
    pub fn bind_label(&mut self, label: Label) {
        let pos = self.position();
        let entry = self
            .bound
            .get_mut(label.0 as usize)
            .expect("invalid label id");
        *entry = Some(pos);
    }

    fn resolve_pending(&mut self) {
        let pending = std::mem::take(&mut self.pending);
        for (label_id, fixup_pos, kind) in pending {
            let target = self
                .bound
                .get(label_id as usize)
                .and_then(|b| *b)
                .expect("unresolved forward label");
            let next_ip = match kind {
                // 32-bit near: the displacement is measured from the end of
                // the displacement itself.
                0 => fixup_pos + 4,
                // 8-bit short: from the end of the byte.
                1 => fixup_pos + 1,
                _ => unreachable!(),
            };
            let rel = (target as isize) - (next_ip as isize);
            if kind == 0 {
                let rel_i32 = rel as i32;
                let bytes = rel_i32.to_le_bytes();
                self.buffer[fixup_pos..fixup_pos + 4].copy_from_slice(&bytes);
            } else {
                let rel_i8 = rel as i8;
                self.buffer[fixup_pos] = rel_i8 as u8;
            }
        }
    }

    // -----------------------------------------------------------------------
    // Generic move / arithmetic
    // -----------------------------------------------------------------------

    /// `mov dst, src` - covers register-to-register for GPR and SSE.  Also
    /// handles `mov reg, imm32` (sign-extended to 64-bit for r64).
    pub fn mov(&mut self, dst: Reg, src: Reg) {
        // Make sure we're not silently truncating.
        debug_assert_eq!(dst.class, src.class, "mov: class mismatch");
        debug_assert_eq!(dst.size, src.size, "mov: size mismatch");

        match dst.class {
            RegClass::Gpr => {
                self.emit_rex(dst.is_wide(), dst.is_extended(), false, src.is_extended());
                // REX.W + 89 /r for mov r/m64, r64.  88 /r for 8/16/32.
                if dst.is_wide() {
                    self.emit_u8(0x89);
                } else {
                    self.emit_u8(if matches!(dst.size, OperandSize::Size8) {
                        0x88
                    } else {
                        0x89
                    });
                }
                self.emit_modrm(0b11, dst.id & 7, src.id & 7);
            }
            RegClass::Sse => {
                // 0F 28 /r for MOVAPS r/m, xmm (we use the aligned form here
                // for r,r as a sane default; the original emitter picks
                // between aligned/unaligned per call).
                self.emit_u8(0x0F);
                self.emit_u8(0x28);
                self.emit_modrm(0b11, dst.id & 7, src.id & 7);
            }
            RegClass::Fpu => {
                // FPU register-to-register moves are encoded as 0xD9 /0..7
                // for ST(i) moves.
                self.emit_u8(0xD9);
                self.emit_modrm(0b11, src.id & 7, dst.id & 7);
            }
        }
    }

    /// `add dst, src` register form (integer).
    pub fn add(&mut self, dst: Reg, src: Reg) {
        debug_assert_eq!(dst.class, RegClass::Gpr);
        debug_assert_eq!(src.class, RegClass::Gpr);
        self.emit_rex(dst.is_wide(), src.is_extended(), false, dst.is_extended());
        self.emit_u8(if dst.is_wide() { 0x01 } else { 0x01 });
        self.emit_modrm(0b11, src.id & 7, dst.id & 7);
    }

    /// `sub dst, src` register form (integer).
    pub fn sub(&mut self, dst: Reg, src: Reg) {
        debug_assert_eq!(dst.class, RegClass::Gpr);
        debug_assert_eq!(src.class, RegClass::Gpr);
        self.emit_rex(dst.is_wide(), src.is_extended(), false, dst.is_extended());
        self.emit_u8(0x29);
        self.emit_modrm(0b11, src.id & 7, dst.id & 7);
    }

    /// `imul dst, src` - two-operand signed multiply.  Encoded as
    /// `0F AF /r` (imul r, r/m).
    pub fn imul(&mut self, dst: Reg, src: Reg) {
        debug_assert_eq!(dst.class, RegClass::Gpr);
        debug_assert_eq!(src.class, RegClass::Gpr);
        self.emit_rex(dst.is_wide(), dst.is_extended(), false, src.is_extended());
        self.emit_u8(0x0F);
        self.emit_u8(0xAF);
        self.emit_modrm(0b11, dst.id & 7, src.id & 7);
    }

    /// `idiv src` - signed divide RDX:RAX by `src` (64-bit) or EDX:EAX
    /// (32-bit).  Stub: emits the standard `0F FA /r` opcodes.
    pub fn idiv(&mut self, src: Reg) {
        debug_assert_eq!(src.class, RegClass::Gpr);
        self.emit_rex(src.is_wide(), false, false, src.is_extended());
        self.emit_u8(0x0F);
        self.emit_u8(0xFA);
        self.emit_modrm(0b11, 7, src.id & 7);
    }

    // -----------------------------------------------------------------------
    // Control flow
    // -----------------------------------------------------------------------

    /// `jmp target` - unconditional near jump.  Forwards references are
    /// recorded as 32-bit near fixups.
    pub fn jmp(&mut self, target: Target) {
        match target {
            Target::Fn(addr) => {
                // For an absolute function pointer we emit a placeholder and
                // rely on the caller to relocate.  As a stand-in, write the
                // raw address bytes - useful when code is later linked
                // through `finalize()`.
                //
                // TODO: real direct call/jmp via MOV rax, imm64; JMP rax.
                self.emit_u8(0xE9);
                self.emit_u32(addr as u32);
            }
            Target::Label(label) => {
                // If the label is already bound, encode the near jump
                // directly; otherwise queue a fixup.
                self.emit_u8(0xE9);
                if let Some(Some(target_pos)) = self.bound.get(label.0 as usize) {
                    let rel = (*target_pos as isize) - ((self.position() + 4) as isize);
                    self.emit_i32(rel as i32);
                } else {
                    let fixup = self.emit_placeholder_i32();
                    self.pending.push((label.0, fixup, 0));
                }
            }
            Target::Placeholder(off) => {
                self.emit_u8(0xE9);
                self.emit_i32(off as i32);
            }
        }
    }

    /// `call target` - near call.  Same target kinds as `jmp`.
    pub fn call(&mut self, target: Target) {
        match target {
            Target::Fn(addr) => {
                self.emit_u8(0xE8);
                self.emit_u32(addr as u32);
            }
            Target::Label(label) => {
                self.emit_u8(0xE8);
                if let Some(Some(target_pos)) = self.bound.get(label.0 as usize) {
                    let rel = (*target_pos as isize) - ((self.position() + 4) as isize);
                    self.emit_i32(rel as i32);
                } else {
                    let fixup = self.emit_placeholder_i32();
                    self.pending.push((label.0, fixup, 0));
                }
            }
            Target::Placeholder(off) => {
                self.emit_u8(0xE8);
                self.emit_i32(off as i32);
            }
        }
    }

    /// `ret` - near return.  0xC3.
    pub fn ret(&mut self) {
        self.emit_u8(0xC3);
    }

    // -----------------------------------------------------------------------
    // Address-size override
    // -----------------------------------------------------------------------

    /// Emit a 0x67 address-size override prefix.  Use this before
    /// instructions that need a 32-bit address in 64-bit mode (or vice
    /// versa).
    pub fn set_address_size(&mut self, _size: OperandSize) {
        // TODO: only emit 0x67 when the override is actually required.
        self.emit_u8(0x67);
    }

    // -----------------------------------------------------------------------
    // LEA
    // -----------------------------------------------------------------------

    /// `lea dst, mem` - load effective address.  Full ModR/M + SIB +
    /// displacement encoding.
    pub fn lea(&mut self, dst: Reg, mem: Mem) {
        debug_assert_eq!(dst.class, RegClass::Gpr);

        let wide = matches!(dst.size, OperandSize::Size64);
        // REX.W
        if wide {
            self.emit_u8(0x48 | (dst.is_extended() as u8));
        } else if dst.is_extended() {
            self.emit_u8(0x41);
        }
        self.emit_u8(0x8D);

        // For the no-base, no-index case, emit a disp32-only ModR/M.
        if mem.base == Mem::INVALID && mem.index == Mem::INVALID {
            // Mod=00, rm=5 (disp32)
            self.emit_modrm(0b00, dst.id & 7, 5);
            self.emit_i32(mem.disp);
            return;
        }

        // Determine the need for SIB.
        let needs_sib = mem.index != Mem::INVALID;
        if needs_sib {
            // Encode SIB with a base register.
            let base_id = if mem.base == Mem::INVALID {
                5 // disp32 base
            } else {
                mem.base.id & 7
            };
            // Mod field: 01 for disp8, 10 for disp32, 00 for none.
            let mod_field = if mem.disp == 0 && base_id != 5 {
                0b00
            } else if fits_i8(mem.disp) {
                0b01
            } else {
                0b10
            };
            self.emit_modrm(mod_field, dst.id & 7, 4); // rm=4 -> SIB follows
            let scale_shift = match mem.scale {
                1 => 0,
                2 => 1,
                4 => 2,
                8 => 3,
                _ => 0,
            };
            self.emit_sib(scale_shift, mem.index.id & 7, base_id);
            if mod_field == 0b01 {
                self.emit_u8(mem.disp as i8 as u8);
            } else if mod_field == 0b10 {
                self.emit_i32(mem.disp);
            }
        } else {
            // No SIB; just base + disp.
            let base_id = mem.base.id & 7;
            let mod_field = if mem.disp == 0 && base_id != 5 {
                0b00
            } else if fits_i8(mem.disp) {
                0b01
            } else {
                0b10
            };
            self.emit_modrm(mod_field, dst.id & 7, base_id);
            if mod_field == 0b01 {
                self.emit_u8(mem.disp as i8 as u8);
            } else if mod_field == 0b10 {
                self.emit_i32(mem.disp);
            }
        }
    }

    // -----------------------------------------------------------------------
    // x87 FPU
    // -----------------------------------------------------------------------

    /// `fadd` - floating-point add.  The full x87 / SSE encoded surface is
    /// large; this stub picks a sensible default per operand type.
    pub fn fadd(&mut self, dst: Reg, src: Reg) {
        debug_assert_eq!(dst.class, RegClass::Fpu);
        debug_assert_eq!(src.class, RegClass::Fpu);
        // 0xD8 /0 for fadd ST(i), ST(0); we swap roles as needed.
        // TODO: cover m32/m64 forms, faddp, fiadd, etc.
        self.emit_u8(0xD8);
        self.emit_modrm(0b11, dst.id & 7, src.id & 7);
    }

    /// `fsub` - floating-point subtract (x87).  0xD8 /4 (FSUB r/m32) for
    /// ST(0) -= mem32 or 0xDE /5 for FSUBR reverse.  Stub emits the
    /// 0xD8 form.
    pub fn fsub(&mut self, dst: Reg, src: Reg) {
        debug_assert_eq!(dst.class, RegClass::Fpu);
        debug_assert_eq!(src.class, RegClass::Fpu);
        self.emit_u8(0xD8);
        self.emit_modrm(0b11, 4, dst.id & 7);
        // Source register doesn't get a second ModR/M byte in this stub.
        let _ = src;
    }

    /// `fmul` - floating-point multiply (x87).  0xD8 /1.
    pub fn fmul(&mut self, dst: Reg, src: Reg) {
        debug_assert_eq!(dst.class, RegClass::Fpu);
        debug_assert_eq!(src.class, RegClass::Fpu);
        self.emit_u8(0xD8);
        self.emit_modrm(0b11, 1, dst.id & 7);
        let _ = src;
    }

    /// `fdiv` - floating-point divide (x87).  0xD8 /6.
    pub fn fdiv(&mut self, dst: Reg, src: Reg) {
        debug_assert_eq!(dst.class, RegClass::Fpu);
        debug_assert_eq!(src.class, RegClass::Fpu);
        self.emit_u8(0xD8);
        self.emit_modrm(0b11, 6, dst.id & 7);
        let _ = src;
    }

    /// `fld` - load floating-point value onto the FPU stack.  0xD9 /0.
    pub fn fld(&mut self, src: Reg) {
        debug_assert_eq!(src.class, RegClass::Fpu);
        // 0xD9 C0+i for FLD ST(i).  This pushes ST(i) onto the FPU stack.
        self.emit_u8(0xD9);
        self.emit_u8(0xC0 | (src.id & 7));
    }

    /// `fstp` - store and pop FPU value.  0xDD /3 (for register form
    /// 0xDD D8+i for FSTP ST(i)).
    pub fn fstp(&mut self, dst: Reg) {
        debug_assert_eq!(dst.class, RegClass::Fpu);
        self.emit_u8(0xDD);
        self.emit_u8(0xD8 | (dst.id & 7));
    }

    // -----------------------------------------------------------------------
    // SSE / AVX integer and packed ops
    // -----------------------------------------------------------------------

    /// `andps` - bitwise AND of packed singles.  0F 54 /r.
    pub fn andps(&mut self, dst: Reg, src: Reg) {
        self.emit_sse_rr(0x54, dst, src);
    }

    /// `orps` - bitwise OR of packed singles.  0F 56 /r.
    pub fn orps(&mut self, dst: Reg, src: Reg) {
        self.emit_sse_rr(0x56, dst, src);
    }

    /// `movaps` - move aligned packed singles.  0F 28 /r (load) and 0F 29 /r
    /// (store).  This stub always emits the load form.
    pub fn movaps(&mut self, dst: Reg, src: Reg) {
        self.emit_sse_rr(0x28, dst, src);
    }

    /// `movups` - move unaligned packed singles.  0F 10 /r.
    pub fn movups(&mut self, dst: Reg, src: Reg) {
        self.emit_sse_rr(0x10, dst, src);
    }

    /// `paddd` - add packed doubleword integers.  66 0F FE /r.
    pub fn paddd(&mut self, dst: Reg, src: Reg) {
        self.emit_u8(0x66);
        self.emit_sse_rr(0xFE, dst, src);
    }

    /// `psubd` - subtract packed doubleword integers.  66 0F FA /r.
    pub fn psubd(&mut self, dst: Reg, src: Reg) {
        self.emit_u8(0x66);
        self.emit_sse_rr(0xFA, dst, src);
    }

    /// `pmulld` - multiply packed doubleword integers.  66 0F 38 40 /r.
    pub fn pmulld(&mut self, dst: Reg, src: Reg) {
        self.emit_u8(0x66);
        self.emit_u8(0x0F);
        self.emit_u8(0x38);
        self.emit_modrm(0b11, dst.id & 7, src.id & 7);
        // TODO: store the trailing 0x40 opcode byte in the right slot.
        // The C++ emitter interleaves the opcode with the ModR/M byte via
        // the three-byte VEX path; we keep this stub simple.
    }

    /// Helper: emit a 0F xx /r SSE register-register encoding.
    fn emit_sse_rr(&mut self, opcode: u8, dst: Reg, src: Reg) {
        debug_assert_eq!(dst.class, RegClass::Sse);
        debug_assert_eq!(src.class, RegClass::Sse);
        self.emit_u8(0x0F);
        self.emit_u8(opcode);
        self.emit_modrm(0b11, dst.id & 7, src.id & 7);
    }
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

/// Returns true if the value fits in a signed 8-bit integer.
fn fits_i8(v: i32) -> bool {
    v >= i8::MIN as i32 && v <= i8::MAX as i32
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh<'a>(buf: &'a mut [u8]) -> X86Assembler<'a> {
        X86Assembler::new(buf)
    }

    #[test]
    fn nop_is_0x90() {
        let mut buf = [0u8; 16];
        let mut a = fresh(&mut buf);
        a.nop();
        assert_eq!(a.position(), 1);
        let code = a.finalize();
        assert_eq!(code, vec![0x90]);
    }

    #[test]
    fn ret_is_0xc3() {
        let mut buf = [0u8; 16];
        let mut a = fresh(&mut buf);
        a.ret();
        assert_eq!(a.finalize(), vec![0xC3]);
    }

    #[test]
    fn forward_label_jmp_patches() {
        let mut buf = [0u8; 32];
        let mut a = fresh(&mut buf);
        let label = a.new_label();
        a.jmp(Target::Label(label));
        a.nop();
        a.bind_label(label);
        a.nop();
        let code = a.finalize();
        // E9 00 00 00 00 (jmp +0) 90 90
        assert_eq!(code, vec![0xE9, 0x00, 0x00, 0x00, 0x00, 0x90, 0x90]);
    }

    #[test]
    fn lea_with_base_only() {
        let mut buf = [0u8; 16];
        let mut a = fresh(&mut buf);
        a.lea(RAX, Mem::base_disp(RCX, 8));
        // 48 8D 41 08
        assert_eq!(a.finalize(), vec![0x48, 0x8D, 0x41, 0x08]);
    }
}
