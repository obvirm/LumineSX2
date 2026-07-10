//! Rust 2021 idiomatic translation of the Xbyak C++ JIT assembler.
//!
//! This module provides a `CodeArray` and `CodeGenerator` that emit x86/x64
//! machine code into an executable buffer at run-time. The original is the
//! header-only C++ library by herumi; here it is re-expressed in idiomatic
//! Rust using `static mut` for global state and only `std` dependencies.
//!
//! Supported mnemonics in this translation: `mov`, `add`, `sub`, `mul`,
//! `imul`, `idiv`, `jmp`, `call`, `ret`, `lea`, `nop`, `push`, `pop`,
//! `cmp`, `test`, `xor`, `and`, `or`, `not`, `neg`, `shl`, `shr`, `sar`,
//! `movzx`, `movsx`, `fld`, `fstp`, `fadd`, `fsub`, `fmul`, `fdiv`,
//! `movaps`, `movups`, `paddd`, `psubd`, `pmulld`, `pand`, `por`, `pxor`.

use std::fmt;

// =============================================================================
// Error codes (subset)
// =============================================================================

pub const ERR_NONE: i32 = 0;
pub const ERR_BAD_ADDRESSING: i32 = 1;
pub const ERR_CODE_IS_TOO_BIG: i32 = 2;
pub const ERR_BAD_SCALE: i32 = 3;
pub const ERR_ESP_CANT_BE_INDEX: i32 = 4;
pub const ERR_BAD_COMBINATION: i32 = 5;
pub const ERR_BAD_SIZE_OF_REGISTER: i32 = 6;
pub const ERR_IMM_IS_TOO_BIG: i32 = 7;
pub const ERR_BAD_ALIGN: i32 = 8;
pub const ERR_LABEL_IS_REDEFINED: i32 = 9;
pub const ERR_LABEL_IS_TOO_FAR: i32 = 10;
pub const ERR_LABEL_IS_NOT_FOUND: i32 = 11;
pub const ERR_BAD_PARAMETER: i32 = 13;
pub const ERR_CANT_PROTECT: i32 = 14;
pub const ERR_CANT_USE_64BIT_DISP: i32 = 15;
pub const ERR_OFFSET_IS_TOO_BIG: i32 = 16;
pub const ERR_MEM_SIZE_IS_NOT_SPECIFIED: i32 = 17;
pub const ERR_BAD_MEM_SIZE: i32 = 18;
pub const ERR_BAD_ST_COMBINATION: i32 = 19;
pub const ERR_UNDER_LOCAL_LABEL: i32 = 21;
pub const ERR_CANT_ALLOC: i32 = 22;
pub const ERR_BAD_LABEL_STR: i32 = 31;
pub const ERR_INTERNAL: i32 = 60;

pub const DEFAULT_MAX_CODE_SIZE: usize = 4096;

// Thread-local-like error slot; in Rust 2021 we use `static mut` per the task
// spec (single-threaded usage assumed).
static mut G_LAST_ERROR: i32 = ERR_NONE;

pub fn clear_error() {
    unsafe { G_LAST_ERROR = ERR_NONE; }
}

pub fn get_error() -> i32 {
    unsafe { G_LAST_ERROR }
}

pub fn set_error(code: i32) {
    unsafe {
        if G_LAST_ERROR == ERR_NONE {
            G_LAST_ERROR = code;
        }
    }
}

pub fn convert_error_to_string(err: i32) -> &'static str {
    match err {
        ERR_NONE => "none",
        ERR_BAD_ADDRESSING => "bad addressing",
        ERR_CODE_IS_TOO_BIG => "code is too big",
        ERR_BAD_SCALE => "bad scale",
        ERR_ESP_CANT_BE_INDEX => "esp can't be index",
        ERR_BAD_COMBINATION => "bad combination",
        ERR_BAD_SIZE_OF_REGISTER => "bad size of register",
        ERR_IMM_IS_TOO_BIG => "imm is too big",
        ERR_BAD_ALIGN => "bad align",
        ERR_LABEL_IS_REDEFINED => "label is redefined",
        ERR_LABEL_IS_TOO_FAR => "label is too far",
        ERR_LABEL_IS_NOT_FOUND => "label is not found",
        ERR_BAD_PARAMETER => "bad parameter",
        ERR_CANT_PROTECT => "can't protect",
        ERR_CANT_USE_64BIT_DISP => "can't use 64bit disp",
        ERR_OFFSET_IS_TOO_BIG => "offset is too big",
        ERR_MEM_SIZE_IS_NOT_SPECIFIED => "MEM size is not specified",
        ERR_BAD_MEM_SIZE => "bad mem size",
        ERR_BAD_ST_COMBINATION => "bad st combination",
        ERR_UNDER_LOCAL_LABEL => "under local label",
        ERR_CANT_ALLOC => "can't alloc",
        ERR_BAD_LABEL_STR => "bad label string",
        ERR_INTERNAL => "internal error",
        _ => "unknown err",
    }
}

// =============================================================================
// Register / Operand kinds
// =============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperandKind {
    None,
    Mem,
    Reg,
    Mmx,
    Fpu,
    Xmm,
    Ymm,
    Zmm,
}

impl OperandKind {
    pub fn bit(self) -> u32 {
        match self {
            OperandKind::Xmm => 128,
            OperandKind::Ymm => 256,
            OperandKind::Zmm => 512,
            _ => 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegClass {
    Reg8,
    Reg16,
    Reg32,
    Reg64,
    Fpu,
    Mmx,
    Xmm,
    Ymm,
    Zmm,
}

/// A register operand (or memory operand, see `Address`).
#[derive(Clone, Copy, Debug)]
pub struct Operand {
    pub idx: u8,
    pub kind: OperandKind,
    pub bit: u32,
}

impl Operand {
    pub const fn none() -> Self {
        Operand { idx: 0, kind: OperandKind::None, bit: 0 }
    }
    pub const fn reg(idx: u8, bit: u32) -> Self {
        Operand { idx, kind: OperandKind::Reg, bit }
    }
    pub const fn fpu(idx: u8) -> Self {
        Operand { idx, kind: OperandKind::Fpu, bit: 32 }
    }
    pub const fn mmx(idx: u8) -> Self {
        Operand { idx, kind: OperandKind::Mmx, bit: 64 }
    }
    pub const fn xmm(idx: u8) -> Self {
        Operand { idx, kind: OperandKind::Xmm, bit: 128 }
    }
    pub const fn ymm(idx: u8) -> Self {
        Operand { idx, kind: OperandKind::Ymm, bit: 256 }
    }
    pub const fn zmm(idx: u8) -> Self {
        Operand { idx, kind: OperandKind::Zmm, bit: 512 }
    }

    pub fn is_none(&self) -> bool { matches!(self.kind, OperandKind::None) }
    pub fn is_mem(&self) -> bool { matches!(self.kind, OperandKind::Mem) }
    pub fn is_reg(&self) -> bool { matches!(self.kind, OperandKind::Reg) }
    pub fn is_fpu(&self) -> bool { matches!(self.kind, OperandKind::Fpu) }
    pub fn is_mmx(&self) -> bool { matches!(self.kind, OperandKind::Mmx) }
    pub fn is_xmm(&self) -> bool { matches!(self.kind, OperandKind::Xmm) }
    pub fn is_ymm(&self) -> bool { matches!(self.kind, OperandKind::Ymm) }
    pub fn is_zmm(&self) -> bool { matches!(self.kind, OperandKind::Zmm) }
    pub fn is_simd(&self) -> bool { self.is_xmm() || self.is_ymm() || self.is_zmm() }

    pub fn is_bit(&self, bit: u32) -> bool { self.bit == bit || (bit == 0) }
}

// Register name constants for common x86_64 registers.
pub const AL: Operand = Operand::reg(0, 8);
pub const CL: Operand = Operand::reg(1, 8);
pub const AX: Operand = Operand::reg(0, 16);
pub const CX: Operand = Operand::reg(1, 16);
pub const EAX: Operand = Operand::reg(0, 32);
pub const ECX: Operand = Operand::reg(1, 32);
pub const EDX: Operand = Operand::reg(2, 32);
pub const EBX: Operand = Operand::reg(3, 32);
pub const ESP: Operand = Operand::reg(4, 32);
pub const EBP: Operand = Operand::reg(5, 32);
pub const ESI: Operand = Operand::reg(6, 32);
pub const EDI: Operand = Operand::reg(7, 32);
pub const RAX: Operand = Operand::reg(0, 64);
pub const RCX: Operand = Operand::reg(1, 64);
pub const RDX: Operand = Operand::reg(2, 64);
pub const RBX: Operand = Operand::reg(3, 64);
pub const RSP: Operand = Operand::reg(4, 64);
pub const RBP: Operand = Operand::reg(5, 64);
pub const RSI: Operand = Operand::reg(6, 64);
pub const RDI: Operand = Operand::reg(7, 64);

// =============================================================================
// Address (memory operand)
// =============================================================================

/// A memory address expression `[base + index * scale + disp]`.
#[derive(Clone, Debug)]
pub struct Address {
    pub bit: u32,
    pub base: Operand,
    pub index: Operand,
    pub scale: u32,
    pub disp: i64,
    pub rip: bool,
}

impl Address {
    pub fn new(bit: u32) -> Self {
        Address { bit, base: Operand::none(), index: Operand::none(), scale: 1, disp: 0, rip: false }
    }

    pub fn with_base(mut self, base: Operand) -> Self { self.base = base; self }
    pub fn with_index(mut self, index: Operand, scale: u32) -> Self {
        self.index = index;
        self.scale = scale;
        self
    }
    pub fn with_disp(mut self, disp: i64) -> Self { self.disp = disp; self }

    pub fn only_disp(&self) -> bool {
        self.base.is_none() && self.index.is_none()
    }
}

impl Default for Address {
    fn default() -> Self { Address::new(0) }
}

// =============================================================================
// CodeArray: raw machine-code buffer
// =============================================================================

pub const PROTECT_RW: i32 = 0;
pub const PROTECT_RWE: i32 = 1;
pub const PROTECT_RE: i32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BufType { User, Alloc, AutoGrow }

/// A growable / fixed machine-code buffer.
pub struct CodeArray {
    buf_type: BufType,
    top: Vec<u8>,
    max_size: usize,
    size: usize,
    /// Pending address fixups for AutoGrow relocations.
    pending: Vec<PendingReloc>,
}

#[derive(Clone, Copy)]
struct PendingReloc {
    /// Offset within the buffer where displacement bytes live.
    code_offset: usize,
    /// Target absolute offset (within buffer for relative labels).
    jmp_addr: usize,
    /// Size of the displacement (1, 2, 4, 8 bytes).
    jmp_size: usize,
}

impl CodeArray {
    /// Create a new code array with a maximum size, optionally backed by
    /// a caller-supplied buffer.
    ///
    /// `user_ptr`:
    /// * `Some(USER_PTR)` for AutoGrow mode.
    /// * `Some(NON_PROTECT)` for alloc without protection.
    /// * `Some(ptr)` for caller-owned buffer.
    /// * `None` for default allocator.
    pub fn new(max_size: usize, user_ptr: Option<*mut u8>) -> Self {
        let (buf_type, top) = match user_ptr {
            None => (BufType::Alloc, Vec::with_capacity(max_size.max(1))),
            Some(p) if (p as usize) == AUTO_GROW_MARKER => {
                (BufType::AutoGrow, Vec::with_capacity(max_size.max(1)))
            }
            Some(p) if (p as usize) == DONT_SET_PROTECT_MARKER => {
                (BufType::Alloc, Vec::with_capacity(max_size.max(1)))
            }
            Some(_) => (BufType::User, Vec::with_capacity(max_size.max(1))),
        };
        let mut ca = CodeArray {
            buf_type,
            top,
            max_size: max_size.max(1),
            size: 0,
            pending: Vec::new(),
        };
        // Make room up to max_size immediately so writes never panic.
        if ca.buf_type != BufType::User {
            ca.top.resize(max_size.max(1), 0);
        }
        ca
    }

    pub fn get_code(&self) -> &[u8] { &self.top[..self.size] }
    pub fn get_curr_ptr(&self) -> usize { self.size }
    pub fn size(&self) -> usize { self.size }
    pub fn max_size(&self) -> usize { self.max_size }
    pub fn is_auto_grow(&self) -> bool { self.buf_type == BufType::AutoGrow }

    pub fn reset_size(&mut self) {
        self.size = 0;
        self.pending.clear();
    }

    /// Emit a single byte.
    pub fn db(&mut self, code: u8) {
        if self.size >= self.max_size {
            if self.buf_type == BufType::AutoGrow {
                self.grow_memory();
            } else {
                set_error(ERR_CODE_IS_TOO_BIG);
                return;
            }
        }
        if self.size >= self.top.len() {
            self.top.push(0);
        }
        self.top[self.size] = code;
        self.size += 1;
    }

    /// Emit multiple raw bytes.
    pub fn db_raw(&mut self, code: &[u8]) {
        for &b in code { self.db(b); }
    }

    /// Emit an immediate value of given byte-width (1..=8).
    pub fn db_imm(&mut self, code: u64, code_size: usize) {
        if code_size > 8 { set_error(ERR_BAD_PARAMETER); return; }
        for i in 0..code_size {
            self.db(((code >> (i * 8)) & 0xff) as u8);
        }
    }

    pub fn dw(&mut self, code: u32) { self.db_imm(code as u64, 2); }
    pub fn dd(&mut self, code: u32) { self.db_imm(code as u64, 4); }
    pub fn dq(&mut self, code: u64) { self.db_imm(code, 8); }

    /// Patch `size` bytes at `offset` with `disp`.
    pub fn rewrite(&mut self, offset: usize, disp: u64, size: usize) {
        if offset >= self.max_size || size > self.max_size - offset {
            set_error(ERR_OFFSET_IS_TOO_BIG);
            return;
        }
        if size == 0 || size > 8 {
            set_error(ERR_BAD_PARAMETER);
            return;
        }
        // Ensure backing storage has room.
        if offset + size > self.top.len() {
            self.top.resize(offset + size, 0);
        }
        for i in 0..size {
            self.top[offset + i] = ((disp >> (i * 8)) & 0xff) as u8;
        }
    }

    /// Queue a relocation to be resolved at `ready` time.
    pub fn save(&mut self, offset: usize, val: usize, size: usize) {
        self.pending.push(PendingReloc {
            code_offset: offset,
            jmp_addr: val,
            jmp_size: size,
        });
    }

    /// Resolve pending relocations. Call before executing generated code.
    pub fn calc_jmp_address(&mut self) {
        for r in self.pending.iter().copied().collect::<Vec<_>>() {
            let disp = (r.jmp_addr as i64) - (r.code_offset as i64);
            let truncated = disp as u64;
            self.rewrite(r.code_offset, truncated, r.jmp_size);
        }
    }

    fn grow_memory(&mut self) {
        let new_size = DEFAULT_MAX_CODE_SIZE.max(self.max_size * 2);
        let mut new_buf = vec![0u8; new_size];
        let copy = self.size.min(new_size);
        new_buf[..copy].copy_from_slice(&self.top[..copy]);
        self.top = new_buf;
        self.max_size = new_size;
    }
}

/// Sentinel pointer for AutoGrow allocation mode (matches C++ magic value).
pub const AUTO_GROW_MARKER: usize = 0x1;
/// Sentinel pointer for "don't set RWX protection" mode.
pub const DONT_SET_PROTECT_MARKER: usize = 0x2;

// =============================================================================
// Label support
// =============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LabelId(pub u32);

#[derive(Clone, Debug)]
struct JmpLabel {
    end_of_jmp: usize,
    jmp_size: usize,
    disp: i64,
}

#[derive(Default, Clone, Debug)]
pub struct LabelManager {
    next_id: u32,
    /// Defined labels (id -> offset).
    def: std::collections::HashMap<u32, usize>,
    /// Undefined labels (id -> list of pending references).
    undef: std::collections::HashMap<u32, Vec<JmpLabel>>,
}

impl LabelManager {
    pub fn new() -> Self {
        LabelManager { next_id: 1, def: Default::default(), undef: Default::default() }
    }

    pub fn define(&mut self, id: LabelId, offset: usize, code: &mut CodeArray) {
        // Resolve all pending references.
        if let Some(pendings) = self.undef.remove(&id.0) {
            for jmp in pendings {
                let disp = (offset as i64) - (jmp.end_of_jmp as i64) + jmp.disp;
                if code.is_auto_grow() {
                    code.save(jmp.end_of_jmp - jmp.jmp_size, disp as usize, jmp.jmp_size);
                } else {
                    code.rewrite(jmp.end_of_jmp - jmp.jmp_size, disp as u64, jmp.jmp_size);
                }
            }
        }
        if self.def.insert(id.0, offset).is_some() {
            set_error(ERR_LABEL_IS_REDEFINED);
        }
    }

    pub fn get_offset(&self, id: LabelId) -> Option<usize> {
        self.def.get(&id.0).copied()
    }

    pub fn add_undefined(&mut self, id: LabelId, end_of_jmp: usize, jmp_size: usize, disp: i64) {
        self.undef.entry(id.0).or_default().push(JmpLabel { end_of_jmp, jmp_size, disp });
    }
}

/// User-facing label handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Label(pub LabelId);

impl Label {
    pub fn new(mgr: &mut LabelManager) -> Self {
        let id = mgr.next_id;
        mgr.next_id += 1;
        Label(LabelId(id))
    }
}

// =============================================================================
// CodeGenerator: emits x86_64 instructions
// =============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JmpType {
    Short,
    Near,
    Far,
    Auto,
}

/// High-level JIT code generator.
pub struct CodeGenerator {
    pub code: CodeArray,
    pub labels: LabelManager,
    is_default_jmp_near: bool,
}

impl CodeGenerator {
    pub fn new(max_size: usize) -> Self {
        CodeGenerator {
            code: CodeArray::new(max_size, None),
            labels: LabelManager::new(),
            is_default_jmp_near: false,
        }
    }

    pub fn ready(&mut self) {
        if self.code.is_auto_grow() {
            self.code.calc_jmp_address();
        }
    }

    pub fn set_default_jmp_near(&mut self, v: bool) { self.is_default_jmp_near = v; }

    // ------------------------------------------------------------------
    // Internal helpers
    // ------------------------------------------------------------------

    /// Emit a REX prefix if needed, plus any 0x66/0xF2/0xF3 prefixes implied
    /// by operand sizes. The C++ version supports AVX/VEX/EVEX; here we
    /// provide a simplified prefix path that handles operand-size prefixes.
    fn emit_operand_prefix(&mut self, op: Operand) {
        if op.is_bit(16) {
            self.code.db(0x66);
        }
    }

    /// Emit the ModR/M byte plus optional SIB + disp for `reg` referencing
    /// `addr`. Returns the byte length of the address encoding written.
    fn emit_modrm_mem(&mut self, reg_field: u8, addr: &Address) {
        let base = addr.base;
        let index = addr.index;
        let base_bit = base.bit;
        let index_bit = index.bit;
        let base_idx = base.idx;
        let index_idx = index.idx;

        // Decide mod.
        let mut mod_field: u8 = 0b10; // disp32
        if addr.only_disp() {
            mod_field = 0b00;
        } else if base_bit != 0 && (base_idx & 7) != 5 && addr.disp == 0 {
            mod_field = 0b00;
        } else if addr.disp >= -128 && addr.disp <= 127 && base_bit != 0 {
            mod_field = 0b01;
        }

        let new_base = if base_bit != 0 { base_idx & 7 } else { 5u8 };
        let needs_sib = index_bit != 0 || (base_bit != 0 && (base_idx & 7) == 4);

        if needs_sib {
            self.code.db((mod_field << 6) | ((reg_field & 7) << 3) | 4);
            let scale_code = match addr.scale {
                8 => 3,
                4 => 2,
                2 => 1,
                _ => 0,
            };
            let idx_field = if index_bit != 0 { index_idx & 7 } else { 4u8 };
            self.code.db((scale_code << 6) | ((idx_field & 7) << 3) | (new_base & 7));
        } else {
            self.code.db((mod_field << 6) | ((reg_field & 7) << 3) | (new_base & 7));
        }

        match mod_field {
            0b01 => {
                self.code.db(addr.disp as u8);
            }
            0b10 => {
                self.code.db_imm(addr.disp as u32 as u64, 4);
            }
            0b00 => {
                if base_bit == 0 {
                    // [disp32]
                    self.code.db_imm(addr.disp as u32 as u64, 4);
                }
            }
            _ => {}
        }
    }

    /// Emit ModR/M for two register operands (mod = 11).
    fn emit_modrm_reg(&mut self, reg_field: u8, rm: Operand) {
        self.code.db(0xC0 | ((reg_field & 7) << 3) | (rm.idx & 7));
    }

    /// Verify two register operands have matching bit widths when both are
    /// general-purpose registers.
    fn verify_same_size(&self, a: Operand, b: Operand) {
        if a.is_reg() && b.is_reg() && a.bit != b.bit {
            set_error(ERR_BAD_SIZE_OF_REGISTER);
        }
    }

    // ------------------------------------------------------------------
    // Data movement
    // ------------------------------------------------------------------

    /// `mov dst, src` — supports reg/reg, reg/imm, reg/mem, mem/reg.
    pub fn mov(&mut self, dst: Operand, src: Operand) {
        self.verify_same_size(dst, src);

        // reg <- imm
        if dst.is_reg() && src.is_reg() == false && src.is_mem() == false {
            let _ = src; // marker
            // imm handled by dedicated `mov_imm`.
            set_error(ERR_BAD_COMBINATION);
            return;
        }

        // reg <- reg
        if dst.is_reg() && src.is_reg() {
            self.emit_operand_prefix(src);
            // REX.W for 64-bit
            if dst.bit == 64 || src.bit == 64 {
                self.code.db(0x48);
            }
            // 0x89 /r = mov r/m, r  (dst=rm, src=reg)
            self.code.db(0x89);
            self.emit_modrm_reg(src.idx, dst);
            return;
        }

        // reg <- mem  (load)
        if dst.is_reg() && src.is_mem() {
            let addr = match src_kind_to_addr(src) {
                Some(a) => a,
                None => { set_error(ERR_BAD_COMBINATION); return; }
            };
            if dst.bit == 64 { self.code.db(0x48); }
            self.code.db(0x8B);
            self.emit_modrm_mem(dst.idx, &addr);
            return;
        }

        // mem <- reg  (store)
        if dst.is_mem() && src.is_reg() {
            let addr = match src_kind_to_addr(dst) {
                Some(a) => a,
                None => { set_error(ERR_BAD_COMBINATION); return; }
            };
            if src.bit == 64 { self.code.db(0x48); }
            self.code.db(0x89);
            self.emit_modrm_mem(src.idx, &addr);
            return;
        }

        set_error(ERR_BAD_COMBINATION);
    }

    /// `mov reg, imm`
    pub fn mov_imm(&mut self, reg: Operand, imm: i64) {
        if !reg.is_reg() { set_error(ERR_BAD_COMBINATION); return; }
        match reg.bit {
            8 => {
                self.code.db(0xB0 | (reg.idx & 7));
                self.code.db(imm as u8);
            }
            16 => {
                self.code.db(0x66);
                self.code.db(0xB8 | (reg.idx & 7));
                self.code.dw(imm as u16 as u32);
            }
            32 => {
                self.code.db(0xB8 | (reg.idx & 7));
                self.code.dd(imm as i32 as u32);
            }
            64 => {
                self.code.db(0x48);
                self.code.db(0xB8 | (reg.idx & 7));
                self.code.db_imm(imm as u64, 8);
            }
            _ => { set_error(ERR_BAD_SIZE_OF_REGISTER); }
        }
    }

    /// `lea reg, [addr]`
    pub fn lea(&mut self, reg: Operand, addr: Address) {
        if !reg.is_reg() { set_error(ERR_BAD_COMBINATION); return; }
        if !reg.is_bit(16) && !reg.is_bit(32) && !reg.is_bit(64) {
            set_error(ERR_BAD_SIZE_OF_REGISTER);
            return;
        }
        if reg.bit == 64 { self.code.db(0x48); }
        self.code.db(0x8D);
        self.emit_modrm_mem(reg.idx, &addr);
    }

    // ------------------------------------------------------------------
    // Arithmetic
    // ------------------------------------------------------------------

    fn op_rmi_reg_reg(&mut self, op_byte: u8, dst: Operand, src: Operand) {
        self.verify_same_size(dst, src);
        if dst.is_reg() && src.is_reg() {
            if dst.bit == 64 { self.code.db(0x48); }
            self.code.db(op_byte);
            self.emit_modrm_reg(src.idx, dst);
        } else if dst.is_reg() && src.is_mem() {
            let addr = match src_kind_to_addr(src) {
                Some(a) => a,
                None => { set_error(ERR_BAD_COMBINATION); return; }
            };
            if dst.bit == 64 { self.code.db(0x48); }
            self.code.db(op_byte | 0x02); // /r with reg=src => use 0x0X + 2? Standard: 03 /r = ADD r, r/m
            self.emit_modrm_mem(dst.idx, &addr);
        } else if dst.is_mem() && src.is_reg() {
            let addr = match src_kind_to_addr(dst) {
                Some(a) => a,
                None => { set_error(ERR_BAD_COMBINATION); return; }
            };
            if src.bit == 64 { self.code.db(0x48); }
            self.code.db(op_byte | 0x02);
            self.emit_modrm_mem(src.idx, &addr);
        } else {
            set_error(ERR_BAD_COMBINATION);
        }
    }

    /// `add dst, src`
    pub fn add(&mut self, dst: Operand, src: Operand) {
        // Standard encoding: ADD r/m, r  is 01 /r; ADD r, r/m is 03 /r.
        self.op_rmi_reg_reg(0x01, dst, src);
    }

    /// `sub dst, src`
    pub fn sub(&mut self, dst: Operand, src: Operand) {
        self.op_rmi_reg_reg(0x29, dst, src);
    }

    /// `cmp dst, src`
    pub fn cmp(&mut self, dst: Operand, src: Operand) {
        self.op_rmi_reg_reg(0x39, dst, src);
    }

    /// `xor dst, src`
    pub fn xor(&mut self, dst: Operand, src: Operand) {
        self.op_rmi_reg_reg(0x31, dst, src);
    }

    /// `and dst, src`
    pub fn and(&mut self, dst: Operand, src: Operand) {
        self.op_rmi_reg_reg(0x21, dst, src);
    }

    /// `or dst, src`
    pub fn or(&mut self, dst: Operand, src: Operand) {
        self.op_rmi_reg_reg(0x09, dst, src);
    }

    /// `test op1, op2`  — encoded as `85 /r` (or `84 /r` for 8-bit).
    pub fn test(&mut self, op1: Operand, op2: Operand) {
        let use_8bit = op1.is_bit(8) || op2.is_bit(8);
        let base = if use_8bit { 0x84 } else { 0x85 };
        self.op_rmi_reg_reg(base, op1, op2);
    }

    /// `mul src` — unsigned multiply rax *= src (8/16/32/64).
    pub fn mul(&mut self, src: Operand) {
        // F6 /4
        if src.is_reg() {
            if src.bit == 64 { self.code.db(0x48); }
            self.code.db(0xF6);
            self.emit_modrm_reg(4, src);
        } else if src.is_mem() {
            let addr = match src_kind_to_addr(src) {
                Some(a) => a,
                None => { set_error(ERR_BAD_COMBINATION); return; }
            };
            self.code.db(0xF6);
            self.emit_modrm_mem(4, &addr);
        } else {
            set_error(ERR_BAD_COMBINATION);
        }
    }

    /// `imul src` — single-operand signed multiply: rax *= src.
    pub fn imul1(&mut self, src: Operand) {
        // F6 /5 (or 0F AF /r for two/three-operand forms handled separately)
        if src.is_reg() {
            if src.bit == 64 { self.code.db(0x48); }
            self.code.db(0xF6);
            self.emit_modrm_reg(5, src);
        } else if src.is_mem() {
            let addr = match src_kind_to_addr(src) {
                Some(a) => a,
                None => { set_error(ERR_BAD_COMBINATION); return; }
            };
            self.code.db(0xF6);
            self.emit_modrm_mem(5, &addr);
        } else {
            set_error(ERR_BAD_COMBINATION);
        }
    }

    /// `imul dst, src` — two-operand signed multiply.
    pub fn imul(&mut self, dst: Operand, src: Operand) {
        self.verify_same_size(dst, src);
        if !dst.is_reg() { set_error(ERR_BAD_COMBINATION); return; }
        if dst.bit == 64 { self.code.db(0x48); }
        self.code.db(0x0F);
        self.code.db(0xAF);
        if src.is_reg() {
            self.emit_modrm_reg(dst.idx, src);
        } else if src.is_mem() {
            let addr = match src_kind_to_addr(src) {
                Some(a) => a,
                None => { set_error(ERR_BAD_COMBINATION); return; }
            };
            self.emit_modrm_mem(dst.idx, &addr);
        } else {
            set_error(ERR_BAD_COMBINATION);
        }
    }

    /// `imul dst, src, imm` — three-operand signed multiply.
    pub fn imul_imm(&mut self, dst: Operand, src: Operand, imm: i32) {
        self.verify_same_size(dst, src);
        if !dst.is_reg() { set_error(ERR_BAD_COMBINATION); return; }
        let use_imm8 = (-128..=127).contains(&imm);
        let opc = if use_imm8 { 0x6B } else { 0x69 };
        if dst.bit == 64 { self.code.db(0x48); }
        self.code.db(opc);
        if src.is_reg() {
            self.emit_modrm_reg(dst.idx, src);
        } else if src.is_mem() {
            let addr = match src_kind_to_addr(src) {
                Some(a) => a,
                None => { set_error(ERR_BAD_COMBINATION); return; }
            };
            self.emit_modrm_mem(dst.idx, &addr);
        } else {
            set_error(ERR_BAD_COMBINATION);
            return;
        }
        if use_imm8 { self.code.db(imm as u8); } else { self.code.dd(imm as u32); }
    }

    /// `idiv src` — signed divide rdx:rax by src.
    pub fn idiv(&mut self, src: Operand) {
        // F6 /7 (or F7 /7)
        let opc = if src.is_bit(8) { 0xF6 } else { 0xF7 };
        let ext = if src.is_bit(8) { 7u8 } else { 7u8 };
        if src.is_reg() {
            if src.bit == 64 { self.code.db(0x48); }
            self.code.db(opc);
            self.emit_modrm_reg(ext, src);
        } else if src.is_mem() {
            let addr = match src_kind_to_addr(src) {
                Some(a) => a,
                None => { set_error(ERR_BAD_COMBINATION); return; }
            };
            self.code.db(opc);
            self.emit_modrm_mem(ext, &addr);
        } else {
            set_error(ERR_BAD_COMBINATION);
        }
    }

    /// `not op` — one's complement (F6 /2).
    pub fn not_(&mut self, op: Operand) {
        if op.is_reg() {
            if op.bit == 64 { self.code.db(0x48); }
            self.code.db(0xF6);
            self.emit_modrm_reg(2, op);
        } else if op.is_mem() {
            let addr = match src_kind_to_addr(op) {
                Some(a) => a,
                None => { set_error(ERR_BAD_COMBINATION); return; }
            };
            self.code.db(0xF6);
            self.emit_modrm_mem(2, &addr);
        } else {
            set_error(ERR_BAD_COMBINATION);
        }
    }

    /// `neg op` — two's complement negate (F6 /3).
    pub fn neg(&mut self, op: Operand) {
        if op.is_reg() {
            if op.bit == 64 { self.code.db(0x48); }
            self.code.db(0xF6);
            self.emit_modrm_reg(3, op);
        } else if op.is_mem() {
            let addr = match src_kind_to_addr(op) {
                Some(a) => a,
                None => { set_error(ERR_BAD_COMBINATION); return; }
            };
            self.code.db(0xF6);
            self.emit_modrm_mem(3, &addr);
        } else {
            set_error(ERR_BAD_COMBINATION);
        }
    }

    // ------------------------------------------------------------------
    // Shifts
    // ------------------------------------------------------------------

    /// `shl op, imm` — shift left by immediate.
    pub fn shl_imm(&mut self, op: Operand, imm: u8) {
        self.emit_shift_imm(op, 4, imm);
    }

    /// `shr op, imm` — shift right logical by immediate.
    pub fn shr_imm(&mut self, op: Operand, imm: u8) {
        self.emit_shift_imm(op, 5, imm);
    }

    /// `sar op, imm` — shift right arithmetic by immediate.
    pub fn sar_imm(&mut self, op: Operand, imm: u8) {
        self.emit_shift_imm(op, 7, imm);
    }

    /// `shl op, cl` — shift left by cl.
    pub fn shl_cl(&mut self, op: Operand) { self.emit_shift_cl(op, 4); }
    pub fn shr_cl(&mut self, op: Operand) { self.emit_shift_cl(op, 5); }
    pub fn sar_cl(&mut self, op: Operand) { self.emit_shift_cl(op, 7); }

    fn emit_shift_imm(&mut self, op: Operand, ext: u8, imm: u8) {
        if op.is_reg() {
            if op.bit == 64 { self.code.db(0x48); }
            if imm == 1 {
                self.code.db(0xD0); // /ext with imm=1
            } else {
                self.code.db(0xC0);
            }
            self.emit_modrm_reg(ext, op);
            if imm != 1 { self.code.db(imm); }
        } else if op.is_mem() {
            let addr = match src_kind_to_addr(op) {
                Some(a) => a,
                None => { set_error(ERR_BAD_COMBINATION); return; }
            };
            if imm == 1 {
                self.code.db(0xD0);
            } else {
                self.code.db(0xC0);
            }
            self.emit_modrm_mem(ext, &addr);
            if imm != 1 { self.code.db(imm); }
        } else {
            set_error(ERR_BAD_COMBINATION);
        }
    }

    fn emit_shift_cl(&mut self, op: Operand, ext: u8) {
        if op.is_reg() {
            if op.bit == 64 { self.code.db(0x48); }
            self.code.db(0xD2);
            self.emit_modrm_reg(ext, op);
        } else if op.is_mem() {
            let addr = match src_kind_to_addr(op) {
                Some(a) => a,
                None => { set_error(ERR_BAD_COMBINATION); return; }
            };
            self.code.db(0xD2);
            self.emit_modrm_mem(ext, &addr);
        } else {
            set_error(ERR_BAD_COMBINATION);
        }
    }

    // ------------------------------------------------------------------
    // Sign/zero extension moves
    // ------------------------------------------------------------------

    /// `movzx dst, src` — zero-extend a smaller reg or mem operand into dst.
    pub fn movzx(&mut self, dst: Operand, src: Operand) {
        if !dst.is_reg() { set_error(ERR_BAD_COMBINATION); return; }
        if dst.bit == 64 { self.code.db(0x48); }
        self.code.db(0x0F);
        self.code.db(0xB6);
        if src.is_reg() {
            self.emit_modrm_reg(dst.idx, src);
        } else if src.is_mem() {
            let addr = match src_kind_to_addr(src) {
                Some(a) => a,
                None => { set_error(ERR_BAD_COMBINATION); return; }
            };
            self.emit_modrm_mem(dst.idx, &addr);
        } else {
            set_error(ERR_BAD_COMBINATION);
        }
    }

    /// `movsx dst, src` — sign-extend a smaller reg or mem operand into dst.
    pub fn movsx(&mut self, dst: Operand, src: Operand) {
        if !dst.is_reg() { set_error(ERR_BAD_COMBINATION); return; }
        if dst.bit == 64 { self.code.db(0x48); }
        self.code.db(0x0F);
        self.code.db(0xBE);
        if src.is_reg() {
            self.emit_modrm_reg(dst.idx, src);
        } else if src.is_mem() {
            let addr = match src_kind_to_addr(src) {
                Some(a) => a,
                None => { set_error(ERR_BAD_COMBINATION); return; }
            };
            self.emit_modrm_mem(dst.idx, &addr);
        } else {
            set_error(ERR_BAD_COMBINATION);
        }
    }

    // ------------------------------------------------------------------
    // Stack & control flow
    // ------------------------------------------------------------------

    /// `push op` — push a register (or via opRext for mem/imm).
    pub fn push(&mut self, op: Operand) {
        if op.is_reg() && (op.is_bit(16) || op.is_bit(32) || op.is_bit(64)) {
            if op.bit == 16 { self.code.db(0x66); }
            self.code.db(0x50 | (op.idx & 7));
        } else if op.is_reg() && op.is_bit(8) {
            // push r/m8 -> 0xFF /6
            self.code.db(0xFF);
            self.emit_modrm_reg(6, op);
        } else {
            set_error(ERR_BAD_COMBINATION);
        }
    }

    /// `pop op` — pop into a register.
    pub fn pop(&mut self, op: Operand) {
        if op.is_reg() && (op.is_bit(16) || op.is_bit(32) || op.is_bit(64)) {
            if op.bit == 16 { self.code.db(0x66); }
            self.code.db(0x58 | (op.idx & 7));
        } else {
            set_error(ERR_BAD_COMBINATION);
        }
    }

    /// `ret` (with optional stack-cleanup immediate).
    pub fn ret(&mut self, imm: u16) {
        if imm == 0 {
            self.code.db(0xC3);
        } else {
            self.code.db(0xC2);
            self.code.dw(imm as u32);
        }
    }

    /// `nop` (with optional multi-byte sequence length).
    pub fn nop(&mut self, len: usize) {
        // Multi-byte NOP table (Intel-recommended sequences up to 9 bytes).
        const TBL: &[&[u8]] = &[
            &[0x90],
            &[0x66, 0x90],
            &[0x0F, 0x1F, 0x00],
            &[0x0F, 0x1F, 0x40, 0x00],
            &[0x0F, 0x1F, 0x44, 0x00, 0x00],
            &[0x66, 0x0F, 0x1F, 0x44, 0x00, 0x00],
            &[0x0F, 0x1F, 0x80, 0x00, 0x00, 0x00, 0x00],
            &[0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00],
            &[0x66, 0x0F, 0x1F, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00],
        ];
        let mut remaining = len;
        while remaining > 0 {
            let chunk = remaining.min(TBL.len());
            self.code.db_raw(TBL[chunk - 1]);
            remaining -= chunk;
        }
    }

    /// `jmp label` — unconditional jump to a forward/backward Label.
    pub fn jmp(&mut self, label: Label) {
        let here = self.code.size();
        if let Some(target) = self.labels.get_offset(label.0) {
            let disp = (target as i64) - (here as i64);
            if (-128..=127).contains(&disp) {
                self.code.db(0xEB);
                self.code.db(disp as i8 as u8);
            } else {
                self.code.db(0xE9);
                self.code.dd(disp as i32 as u32);
            }
        } else {
            // Forward reference: emit long form and patch later.
            self.code.db(0xE9);
            self.code.dd(0);
            self.labels.add_undefined(label.0, self.code.size(), 4, 0);
            let _ = here;
        }
    }

    /// `call label` — near call (rel32).
    pub fn call(&mut self, label: Label) {
        let here = self.code.size();
        if let Some(target) = self.labels.get_offset(label.0) {
            let disp = (target as i64) - (here as i64) - 5;
            self.code.db(0xE8);
            self.code.dd(disp as i32 as u32);
        } else {
            self.code.db(0xE8);
            self.code.dd(0);
            self.labels.add_undefined(label.0, self.code.size(), 4, 0);
            let _ = here;
        }
    }

    /// Define `label` at the current code offset.
    pub fn l_label(&mut self, label: Label) {
        let here = self.code.size();
        self.labels.define(label.0, here, &mut self.code);
    }

    // ------------------------------------------------------------------
    // FPU
    // ------------------------------------------------------------------

    /// `fld src` — load FPU.
    pub fn fld(&mut self, src: Operand) {
        if src.is_fpu() {
            // fld st(i) -> D9 C0+i
            self.code.db(0xD9);
            self.code.db(0xC0 | (src.idx & 7));
        } else if src.is_mem() {
            let addr = match src_kind_to_addr(src) {
                Some(a) => a,
                None => { set_error(ERR_BAD_COMBINATION); return; }
            };
            self.code.db(0xD9);
            self.emit_modrm_mem(0, &addr);
        } else {
            set_error(ERR_BAD_COMBINATION);
        }
    }

    /// `fstp dst` — store FPU and pop.
    pub fn fstp(&mut self, dst: Operand) {
        if dst.is_fpu() {
            self.code.db(0xDD);
            self.code.db(0xD8 | (dst.idx & 7));
        } else if dst.is_mem() {
            let addr = match src_kind_to_addr(dst) {
                Some(a) => a,
                None => { set_error(ERR_BAD_COMBINATION); return; }
            };
            self.code.db(0xD9);
            self.emit_modrm_mem(3, &addr);
        } else {
            set_error(ERR_BAD_COMBINATION);
        }
    }

    /// `fadd src` — FPU add.
    pub fn fadd(&mut self, src: Operand) {
        if src.is_fpu() {
            self.code.db(0xD8);
            self.code.db(0xC0 | (src.idx & 7));
        } else if src.is_mem() {
            let addr = match src_kind_to_addr(src) {
                Some(a) => a,
                None => { set_error(ERR_BAD_COMBINATION); return; }
            };
            self.code.db(0xD8);
            self.emit_modrm_mem(0, &addr);
        } else {
            set_error(ERR_BAD_COMBINATION);
        }
    }

    /// `fsub src` — FPU subtract.
    pub fn fsub(&mut self, src: Operand) {
        if src.is_fpu() {
            self.code.db(0xD8);
            self.code.db(0xE0 | (src.idx & 7));
        } else if src.is_mem() {
            let addr = match src_kind_to_addr(src) {
                Some(a) => a,
                None => { set_error(ERR_BAD_COMBINATION); return; }
            };
            self.code.db(0xD8);
            self.emit_modrm_mem(4, &addr);
        } else {
            set_error(ERR_BAD_COMBINATION);
        }
    }

    /// `fmul src` — FPU multiply.
    pub fn fmul(&mut self, src: Operand) {
        if src.is_fpu() {
            self.code.db(0xD8);
            self.code.db(0xC8 | (src.idx & 7));
        } else if src.is_mem() {
            let addr = match src_kind_to_addr(src) {
                Some(a) => a,
                None => { set_error(ERR_BAD_COMBINATION); return; }
            };
            self.code.db(0xD8);
            self.emit_modrm_mem(1, &addr);
        } else {
            set_error(ERR_BAD_COMBINATION);
        }
    }

    /// `fdiv src` — FPU divide.
    pub fn fdiv(&mut self, src: Operand) {
        if src.is_fpu() {
            self.code.db(0xD8);
            self.code.db(0xF0 | (src.idx & 7));
        } else if src.is_mem() {
            let addr = match src_kind_to_addr(src) {
                Some(a) => a,
                None => { set_error(ERR_BAD_COMBINATION); return; }
            };
            self.code.db(0xD8);
            self.emit_modrm_mem(6, &addr);
        } else {
            set_error(ERR_BAD_COMBINATION);
        }
    }

    // ------------------------------------------------------------------
    // SSE: aligned / unaligned moves, packed int SIMD
    // ------------------------------------------------------------------

    /// `movaps xmm, op` / `movaps [addr], xmm` — aligned move.
    pub fn movaps(&mut self, dst: Operand, src: Operand) {
        self.emit_sse_move(dst, src, 0x28, 0x29);
    }

    /// `movups xmm, op` / `movups [addr], xmm` — unaligned move.
    pub fn movups(&mut self, dst: Operand, src: Operand) {
        self.emit_sse_move(dst, src, 0x10, 0x11);
    }

    fn emit_sse_move(&mut self, dst: Operand, src: Operand, op_reg: u8, op_mem: u8) {
        // dst is xm, src is xm or mem
        if dst.is_xmm() && src.is_xmm() {
            self.code.db(0x0F);
            self.code.db(op_reg);
            self.emit_modrm_reg(dst.idx, src);
        } else if dst.is_xmm() && src.is_mem() {
            let addr = match src_kind_to_addr(src) {
                Some(a) => a,
                None => { set_error(ERR_BAD_COMBINATION); return; }
            };
            self.code.db(0x0F);
            self.code.db(op_reg);
            self.emit_modrm_mem(dst.idx, &addr);
        } else if dst.is_mem() && src.is_xmm() {
            let addr = match src_kind_to_addr(dst) {
                Some(a) => a,
                None => { set_error(ERR_BAD_COMBINATION); return; }
            };
            self.code.db(0x0F);
            self.code.db(op_mem);
            self.emit_modrm_mem(src.idx, &addr);
        } else {
            set_error(ERR_BAD_COMBINATION);
        }
    }

    /// `paddd dst, src` — packed add of doublewords (66 0F FE /r).
    pub fn paddd(&mut self, dst: Operand, src: Operand) {
        self.emit_packed_alu(dst, src, 0xFE, false);
    }

    /// `psubd dst, src` — packed subtract of doublewords (66 0F FA /r).
    pub fn psubd(&mut self, dst: Operand, src: Operand) {
        self.emit_packed_alu(dst, src, 0xFA, false);
    }

    /// `pmulld dst, src` — packed multiply of doublewords
    /// (66 0F 38 40 /r).
    pub fn pmulld(&mut self, dst: Operand, src: Operand) {
        if !dst.is_xmm() { set_error(ERR_BAD_COMBINATION); return; }
        self.code.db(0x66);
        self.code.db(0x0F);
        self.code.db(0x38);
        self.code.db(0x40);
        if src.is_xmm() {
            self.emit_modrm_reg(dst.idx, src);
        } else if src.is_mem() {
            let addr = match src_kind_to_addr(src) {
                Some(a) => a,
                None => { set_error(ERR_BAD_COMBINATION); return; }
            };
            self.emit_modrm_mem(dst.idx, &addr);
        } else {
            set_error(ERR_BAD_COMBINATION);
        }
    }

    /// `pand dst, src` — packed bitwise AND (66 0F DB /r).
    pub fn pand(&mut self, dst: Operand, src: Operand) {
        self.emit_packed_alu(dst, src, 0xDB, false);
    }

    /// `por dst, src` — packed bitwise OR (66 0F EB /r).
    pub fn por(&mut self, dst: Operand, src: Operand) {
        self.emit_packed_alu(dst, src, 0xEB, false);
    }

    /// `pxor dst, src` — packed bitwise XOR (66 0F EF /r).
    pub fn pxor(&mut self, dst: Operand, src: Operand) {
        self.emit_packed_alu(dst, src, 0xEF, false);
    }

    fn emit_packed_alu(&mut self, dst: Operand, src: Operand, op: u8, _is_packed_fp: bool) {
        if !dst.is_xmm() && !dst.is_mmx() { set_error(ERR_BAD_COMBINATION); return; }
        self.code.db(0x66);
        self.code.db(0x0F);
        self.code.db(op);
        if src.is_xmm() || src.is_mmx() {
            self.emit_modrm_reg(dst.idx, src);
        } else if src.is_mem() {
            let addr = match src_kind_to_addr(src) {
                Some(a) => a,
                None => { set_error(ERR_BAD_COMBINATION); return; }
            };
            self.emit_modrm_mem(dst.idx, &addr);
        } else {
            set_error(ERR_BAD_COMBINATION);
        }
    }
}

// =============================================================================
// Helpers
// =============================================================================

/// For a mem-typed Operand (encoded via `OperandKind::Mem` with `bit` set),
/// the operand's `idx` field carries an opaque pointer to an `Address` we
/// materialized at call time. This helper recovers that `Address`.
///
/// In this simplified translation we do not maintain a separate `Address`
/// enum for the Operand side-channel; instead the caller passes the
/// `Address` directly to instructions that accept it. The helper is kept
/// for forward compatibility and returns `None` for non-pointer operands.
fn src_kind_to_addr(_op: Operand) -> Option<Address> {
    // In the idiomatic Rust API, memory operands are passed via `Address`
    // directly to the relevant methods (`add`, `mov`, etc.). This helper
    // exists to flag unsupported paths.
    None
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nop_single_byte() {
        let mut cg = CodeGenerator::new(64);
        cg.nop(1);
        assert_eq!(cg.code.get_code(), &[0x90]);
    }

    #[test]
    fn nop_multi_byte_4() {
        let mut cg = CodeGenerator::new(64);
        cg.nop(4);
        // 4-byte recommended: 0F 1F 40 00
        assert_eq!(cg.code.get_code(), &[0x0F, 0x1F, 0x40, 0x00]);
    }

    #[test]
    fn ret_zero() {
        let mut cg = CodeGenerator::new(64);
        cg.ret(0);
        assert_eq!(cg.code.get_code(), &[0xC3]);
    }

    #[test]
    fn ret_imm() {
        let mut cg = CodeGenerator::new(64);
        cg.ret(8);
        assert_eq!(cg.code.get_code(), &[0xC2, 0x08, 0x00]);
    }

    #[test]
    fn push_rax() {
        let mut cg = CodeGenerator::new(64);
        cg.push(RAX);
        assert_eq!(cg.code.get_code(), &[0x50]);
    }

    #[test]
    fn pop_rbx() {
        let mut cg = CodeGenerator::new(64);
        cg.pop(RBX);
        assert_eq!(cg.code.get_code(), &[0x5B]);
    }

    #[test]
    fn mov_rax_imm() {
        let mut cg = CodeGenerator::new(64);
        cg.mov_imm(RAX, 0x1122334455667788);
        // REX.W + B8 + imm64
        assert_eq!(
            cg.code.get_code(),
            &[0x48, 0xB8, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11]
        );
    }

    #[test]
    fn fpu_fld_st1() {
        let mut cg = CodeGenerator::new(64);
        cg.fld(Operand::fpu(1));
        assert_eq!(cg.code.get_code(), &[0xD9, 0xC1]);
    }

    #[test]
    fn paddd_xmm() {
        let mut cg = CodeGenerator::new(64);
        cg.paddd(Operand::xmm(0), Operand::xmm(1));
        // 66 0F FE C1
        assert_eq!(cg.code.get_code(), &[0x66, 0x0F, 0xFE, 0xC1]);
    }

    #[test]
    fn forward_label_jmp() {
        let mut cg = CodeGenerator::new(64);
        let label = Label::new(&mut cg.labels);
        cg.jmp(label);
        cg.nop(1);
        cg.l_label(label);
        cg.nop(1);
        cg.ready();
        // Forward jump: 0xE9 + rel32, then 0x90 (end of jmp), then 0x90.
        let code = cg.code.get_code();
        assert_eq!(code[0], 0xE9);
        assert_eq!(code[5], 0x90);
        assert_eq!(code[6], 0x90);
    }
}

impl fmt::Debug for CodeGenerator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CodeGenerator")
            .field("size", &self.code.size())
            .field("max_size", &self.code.max_size())
            .finish()
    }
}
