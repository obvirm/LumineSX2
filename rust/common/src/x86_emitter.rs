//! Full x86-64 emitter backed by `iced-x86`.
//!
//! Replaces PCSX2's `common/emitter/*` (34 C++ files) with a pure-Rust
//! emitter providing the instruction set PCSX2's recompilers actually use.
//!
//! Architecture:
//!   - `Emitter` holds a `Vec<u8>` buffer and writes machine code using
//!     the `iced-x86::Encoder`.
//!   - Register types (`xRegister32`, `xRegister64`, `xRegisterSSE`)
//!     mirror the C++ `x86types.h` constructs.
//!   - Memory addressing: `xIndirectAddress` with base+index*scale+disp.
//!   - Labels support forward jumps with rel32 fixups.

use iced_x86::{Code, Instruction, MemoryOperand, Register};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const SHADOW_STACK_SIZE: usize = 32; // Win64 ABI

// ---------------------------------------------------------------------------
// x86 Pointer / Thread-Local (mimics PCSX2's x86Ptr)
// ---------------------------------------------------------------------------

pub static mut x86Ptr: *mut u8 = std::ptr::null_mut();
pub static mut g_xmmtypes: [u32; 16] = [0; 16];

pub fn xSetPtr(ptr: *mut u8) {
    unsafe { x86Ptr = ptr; }
}
pub fn xGetPtr() -> *mut u8 {
    unsafe { x86Ptr }
}
pub fn xWrite8(val: u8) {
    unsafe {
        *x86Ptr = val;
        x86Ptr = x86Ptr.add(1);
    }
}
pub fn xWrite16(val: u16) {
    xWrite8((val & 0xFF) as u8);
    xWrite8((val >> 8) as u8);
}
pub fn xWrite32(val: u32) {
    xWrite16((val & 0xFFFF) as u16);
    xWrite16((val >> 16) as u16);
}
pub fn xWrite64(val: u64) {
    xWrite32((val & 0xFFFFFFFF) as u32);
    xWrite32((val >> 32) as u32);
}
pub fn xAlignPtr(bytes: u32) {
    unsafe {
        let p = x86Ptr as usize;
        let aligned = (p + bytes as usize - 1) & !(bytes as usize - 1);
        while (x86Ptr as usize) < aligned {
            *x86Ptr = 0x90; // NOP
            x86Ptr = x86Ptr.add(1);
        }
    }
}
pub fn xAdvancePtr(bytes: u32) {
    unsafe { x86Ptr = x86Ptr.add(bytes as usize); }
}
pub fn xGetAlignedCallTarget() -> *mut u8 {
    unsafe { x86Ptr }
}

// ---------------------------------------------------------------------------
// Register types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum xRegisterType { GPR32, GPR64, XMM }

/// Generic x86 register.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct xRegisterBase {
    pub id: i32,
    pub size: u32,
    pub reg_type: xRegisterType,
}

impl xRegisterBase {
    pub const fn new(id: i32, size: u32, reg_type: xRegisterType) -> Self {
        Self { id, size, reg_type }
    }
    pub fn is_extended(&self) -> bool { self.id >= 8 && self.id < 16 }
    pub fn is_empty(&self) -> bool { self.id < 0 }
    pub fn is_simd(&self) -> bool { matches!(self.reg_type, xRegisterType::XMM) }
}

pub type xRegister32 = xRegisterBase;
pub type xRegister64 = xRegisterBase;
pub type xRegisterSSE = xRegisterBase;

// GPR32 constants
pub const EAX: xRegister32 = xRegisterBase::new(0, 4, xRegisterType::GPR32);
pub const ECX: xRegister32 = xRegisterBase::new(1, 4, xRegisterType::GPR32);
pub const EDX: xRegister32 = xRegisterBase::new(2, 4, xRegisterType::GPR32);
pub const EBX: xRegister32 = xRegisterBase::new(3, 4, xRegisterType::GPR32);
pub const ESP: xRegister32 = xRegisterBase::new(4, 4, xRegisterType::GPR32);
pub const EBP: xRegister32 = xRegisterBase::new(5, 4, xRegisterType::GPR32);
pub const ESI: xRegister32 = xRegisterBase::new(6, 4, xRegisterType::GPR32);
pub const EDI: xRegister32 = xRegisterBase::new(7, 4, xRegisterType::GPR32);

// GPR64 constants
pub const RAX: xRegister64 = xRegisterBase::new(0, 8, xRegisterType::GPR64);
pub const RCX: xRegister64 = xRegisterBase::new(1, 8, xRegisterType::GPR64);
pub const RDX: xRegister64 = xRegisterBase::new(2, 8, xRegisterType::GPR64);
pub const RBX: xRegister64 = xRegisterBase::new(3, 8, xRegisterType::GPR64);
pub const RSP: xRegister64 = xRegisterBase::new(4, 8, xRegisterType::GPR64);
pub const RBP: xRegister64 = xRegisterBase::new(5, 8, xRegisterType::GPR64);
pub const RSI: xRegister64 = xRegisterBase::new(6, 8, xRegisterType::GPR64);
pub const RDI: xRegister64 = xRegisterBase::new(7, 8, xRegisterType::GPR64);
pub const R8:  xRegister64 = xRegisterBase::new(8, 8, xRegisterType::GPR64);
pub const R9:  xRegister64 = xRegisterBase::new(9, 8, xRegisterType::GPR64);
pub const R10: xRegister64 = xRegisterBase::new(10, 8, xRegisterType::GPR64);
pub const R11: xRegister64 = xRegisterBase::new(11, 8, xRegisterType::GPR64);
pub const R12: xRegister64 = xRegisterBase::new(12, 8, xRegisterType::GPR64);
pub const R13: xRegister64 = xRegisterBase::new(13, 8, xRegisterType::GPR64);
pub const R14: xRegister64 = xRegisterBase::new(14, 8, xRegisterType::GPR64);
pub const R15: xRegister64 = xRegisterBase::new(15, 8, xRegisterType::GPR64);

// ---------------------------------------------------------------------------
// Memory addressing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct xIndirectAddress {
    pub base: Option<xRegister64>,
    pub index: Option<xRegister64>,
    pub scale: u32,
    pub displacement: i64,
    pub size: u32,
}

impl xIndirectAddress {
    pub fn new(size: u32) -> Self {
        Self { base: None, index: None, scale: 1, displacement: 0, size }
    }
    pub fn with_base(mut self, reg: xRegister64) -> Self { self.base = Some(reg); self }
    pub fn with_index(mut self, reg: xRegister64, scale: u32) -> Self { self.index = Some(reg); self.scale = scale; self }
    pub fn with_disp(mut self, disp: i64) -> Self { self.displacement = disp; self }
}

/// Build a complex address: disp + base + index*scale.
pub fn xComplexAddress(size: u32, base: xRegister64, index: Option<xRegister64>, scale: u32, displacement: i64) -> xIndirectAddress {
    xIndirectAddress::new(size).with_base(base).with_disp(displacement).with_index(index.unwrap_or(base), scale)
}

pub struct xAddressVoid;

// ---------------------------------------------------------------------------
// Converters
// ---------------------------------------------------------------------------

fn reg_to_iced(reg: xRegisterBase) -> Register {
    let base_id = match reg.reg_type {
        xRegisterType::GPR32 => Register::EAX as u32,
        xRegisterType::GPR64 => Register::RAX as u32,
        xRegisterType::XMM => Register::XMM0 as u32,
    };
    Register::try_from((base_id + reg.id as u32) as usize).unwrap()
}

fn mem_to_memop(addr: &xIndirectAddress) -> MemoryOperand {
    let base = addr.base.map(reg_to_iced).unwrap_or(Register::None);
    let index = addr.index.map(reg_to_iced).unwrap_or(Register::None);
    // displ_size: auto-detect
    let displ_size = if addr.displacement == 0 { 0 }
        else if (addr.displacement as i8 as i64) == addr.displacement { 1 }
        else if (addr.displacement as i32 as i64) == addr.displacement { 4 }
        else { 8 };
    MemoryOperand::with_base_index_scale_displ_size(base, index, addr.scale, addr.displacement, displ_size)
}

// ---------------------------------------------------------------------------
// Emitter — in-memory code buffer
// ---------------------------------------------------------------------------

pub struct Emitter {
    buf: Vec<u8>,
    next_label: u32,
    placed: HashMap<u32, usize>,
    branches: Vec<(usize, u32)>,
}

impl Emitter {
    pub fn new() -> Self {
        Self {
            buf: Vec::with_capacity(4096),
            next_label: 0,
            placed: HashMap::new(),
            branches: Vec::new(),
        }
    }
    pub fn len(&self) -> usize { self.buf.len() }
    pub fn is_empty(&self) -> bool { self.buf.is_empty() }

    pub fn define_label(&mut self) -> u32 {
        let id = self.next_label;
        self.next_label += 1;
        id
    }
    pub fn place_label(&mut self, label: u32) {
        self.placed.insert(label, self.buf.len());
    }

    fn emit(&mut self, instr: Instruction) {
        let mut enc = iced_x86::Encoder::new(64);
        enc.encode(&instr, self.buf.len() as u64).expect("iced-x86 encode");
        let bytes = enc.take_buffer();
        if !bytes.is_empty() {
            self.buf.extend_from_slice(&bytes);
        }
    }
    fn emit_code(&mut self, code: Code) {
        self.emit(Instruction::with(code));
    }
    fn emit_rr(&mut self, code: Code, dst: Register, src: Register) {
        self.emit(Instruction::with2(code, dst, src).expect("with2"));
    }
    fn emit_ri(&mut self, code: Code, dst: Register, imm: u64) {
        self.emit(Instruction::with2(code, dst, imm).expect("with2 imm"));
    }
    fn emit_rm(&mut self, code: Code, dst: Register, addr: &xIndirectAddress) {
        self.emit(Instruction::with2(code, dst, mem_to_memop(addr)).expect("with2 mem"));
    }
    fn emit_mr(&mut self, code: Code, addr: &xIndirectAddress, src: Register) {
        self.emit(Instruction::with2(code, mem_to_memop(addr), src).expect("with2 mr"));
    }
    fn emit_branch(&mut self, code: Code, label: u32) {
        let target = self.placed.get(&label).copied().unwrap_or(0);
        let instr = Instruction::with_branch(code, target as u64).expect("with_branch");
        self.emit(instr);
        if !self.placed.contains_key(&label) {
            self.branches.push((self.buf.len() - 5, label));
        }
    }

    // ── Finalize ──
    pub fn take(mut self) -> Vec<u8> {
        for (instr_offset, label_id) in self.branches.drain(..) {
            let target = self.placed.get(&label_id).copied().expect("unresolved label");
            let next_ip = instr_offset + 5;
            let rel = (target as i64) - (next_ip as i64);
            let rel32 = rel as i32 as u32;
            self.buf[instr_offset + 1..instr_offset + 5].copy_from_slice(&rel32.to_le_bytes());
        }
        self.buf
    }
    pub fn as_bytes(&self) -> &[u8] { &self.buf }

    // ── Data movement (GPR) ──
    pub fn mov_r64_imm(&mut self, dst: xRegister64, imm: i64) {
        self.emit_ri(Code::Mov_r64_imm64, reg_to_iced(dst), imm as u64);
    }
    pub fn mov_r64_r64(&mut self, dst: xRegister64, src: xRegister64) {
        self.emit_rr(Code::Mov_r64_rm64, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn mov_r32_r32(&mut self, dst: xRegister32, src: xRegister32) {
        self.emit_rr(Code::Mov_r32_rm32, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn mov_r32_imm(&mut self, dst: xRegister32, imm: i32) {
        self.emit_ri(Code::Mov_r32_imm32, reg_to_iced(dst), imm as u64);
    }
    pub fn mov_r64_mem(&mut self, dst: xRegister64, addr: &xIndirectAddress) {
        self.emit_rm(Code::Mov_r64_rm64, reg_to_iced(dst), addr);
    }
    pub fn mov_mem_r64(&mut self, addr: &xIndirectAddress, src: xRegister64) {
        self.emit_mr(Code::Mov_rm64_r64, addr, reg_to_iced(src));
    }
    pub fn mov_mem_r32(&mut self, addr: &xIndirectAddress, src: xRegister32) {
        self.emit_mr(Code::Mov_rm32_r32, addr, reg_to_iced(src));
    }
    pub fn movzx_r64_r8(&mut self, dst: xRegister64, src: xRegister64) {
        self.emit_rr(Code::Movzx_r64_rm8, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn movzx_r64_r16(&mut self, dst: xRegister64, src: xRegister64) {
        self.emit_rr(Code::Movzx_r64_rm16, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn movsx_r64_r8(&mut self, dst: xRegister64, src: xRegister64) {
        self.emit_rr(Code::Movsx_r64_rm8, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn movsx_r64_r16(&mut self, dst: xRegister64, src: xRegister64) {
        self.emit_rr(Code::Movsx_r64_rm16, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn movsxd_r64_r32(&mut self, dst: xRegister64, src: xRegister32) {
        self.emit_rr(Code::Movsxd_r64_rm32, reg_to_iced(dst), reg_to_iced(src));
    }

    // ── SSE data movement ──
    pub fn movaps_xmm_xmm(&mut self, dst: xRegisterSSE, src: xRegisterSSE) {
        self.emit_rr(Code::Movaps_xmm_xmmm128, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn movaps_xmm_mem(&mut self, dst: xRegisterSSE, addr: &xIndirectAddress) {
        self.emit_rm(Code::Movaps_xmm_xmmm128, reg_to_iced(dst), addr);
    }
    pub fn movaps_mem_xmm(&mut self, addr: &xIndirectAddress, src: xRegisterSSE) {
        self.emit_mr(Code::Movaps_xmmm128_xmm, addr, reg_to_iced(src));
    }
    pub fn movdqa_xmm_xmm(&mut self, dst: xRegisterSSE, src: xRegisterSSE) {
        self.emit_rr(Code::Movdqa_xmm_xmmm128, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn movdqa_xmm_mem(&mut self, dst: xRegisterSSE, addr: &xIndirectAddress) {
        self.emit_rm(Code::Movdqa_xmm_xmmm128, reg_to_iced(dst), addr);
    }
    pub fn movdqa_mem_xmm(&mut self, addr: &xIndirectAddress, src: xRegisterSSE) {
        self.emit_mr(Code::Movdqa_xmmm128_xmm, addr, reg_to_iced(src));
    }
    pub fn movd_xmm_r32(&mut self, dst: xRegisterSSE, src: xRegister32) {
        self.emit_rr(Code::Movd_xmm_rm32, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn movd_r32_xmm(&mut self, dst: xRegister32, src: xRegisterSSE) {
        self.emit_rr(Code::Movd_rm32_xmm, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn movq_xmm_xmm(&mut self, dst: xRegisterSSE, src: xRegisterSSE) {
        self.emit_rr(Code::Movq_xmm_xmmm64, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn movss_xmm_xmm(&mut self, dst: xRegisterSSE, src: xRegisterSSE) {
        self.emit_rr(Code::Movss_xmm_xmmm32, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn movss_xmm_mem(&mut self, dst: xRegisterSSE, addr: &xIndirectAddress) {
        self.emit_rm(Code::Movss_xmm_xmmm32, reg_to_iced(dst), addr);
    }
    pub fn movss_mem_xmm(&mut self, addr: &xIndirectAddress, src: xRegisterSSE) {
        self.emit_mr(Code::Movss_xmmm32_xmm, addr, reg_to_iced(src));
    }
    /// Zero-extend 32-bit int to XMM (movss to clear upper, matching C++ MOVSSZX)
    pub fn movsszx_xmm_xmm(&mut self, dst: xRegisterSSE, src: xRegisterSSE) {
        self.xorps_xmm_xmm(dst, dst);
        self.emit_rr(Code::Movss_xmm_xmmm32, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn movups_xmm_mem(&mut self, dst: xRegisterSSE, addr: &xIndirectAddress) {
        self.emit_rm(Code::Movups_xmm_xmmm128, reg_to_iced(dst), addr);
    }
    pub fn movups_mem_xmm(&mut self, addr: &xIndirectAddress, src: xRegisterSSE) {
        self.emit_mr(Code::Movups_xmmm128_xmm, addr, reg_to_iced(src));
    }
    pub fn movdqu_xmm_mem(&mut self, dst: xRegisterSSE, addr: &xIndirectAddress) {
        self.emit_rm(Code::Movdqu_xmm_xmmm128, reg_to_iced(dst), addr);
    }
    pub fn movdqu_mem_xmm(&mut self, addr: &xIndirectAddress, src: xRegisterSSE) {
        self.emit_mr(Code::Movdqu_xmmm128_xmm, addr, reg_to_iced(src));
    }

    // ── ALU (GPR) ──
    pub fn add_r64_r64(&mut self, dst: xRegister64, src: xRegister64) {
        self.emit_rr(Code::Add_r64_rm64, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn add_r32_r32(&mut self, dst: xRegister32, src: xRegister32) {
        self.emit_rr(Code::Add_r32_rm32, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn sub_r64_r64(&mut self, dst: xRegister64, src: xRegister64) {
        self.emit_rr(Code::Sub_r64_rm64, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn sub_r32_r32(&mut self, dst: xRegister32, src: xRegister32) {
        self.emit_rr(Code::Sub_r32_rm32, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn and_r64_r64(&mut self, dst: xRegister64, src: xRegister64) {
        self.emit_rr(Code::And_r64_rm64, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn and_r32_r32(&mut self, dst: xRegister32, src: xRegister32) {
        self.emit_rr(Code::And_r32_rm32, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn or_r64_r64(&mut self, dst: xRegister64, src: xRegister64) {
        self.emit_rr(Code::Or_r64_rm64, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn or_r32_r32(&mut self, dst: xRegister32, src: xRegister32) {
        self.emit_rr(Code::Or_r32_rm32, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn xor_r64_r64(&mut self, dst: xRegister64, src: xRegister64) {
        self.emit_rr(Code::Xor_r64_rm64, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn xor_r32_r32(&mut self, dst: xRegister32, src: xRegister32) {
        self.emit_rr(Code::Xor_r32_rm32, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn cmp_r64_r64(&mut self, dst: xRegister64, src: xRegister64) {
        self.emit_rr(Code::Cmp_r64_rm64, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn cmp_r32_r32(&mut self, dst: xRegister32, src: xRegister32) {
        self.emit_rr(Code::Cmp_r32_rm32, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn test_r64_r64(&mut self, dst: xRegister64, src: xRegister64) {
        self.emit_rr(Code::Test_rm64_r64, reg_to_iced(src), reg_to_iced(dst));
    }
    pub fn test_r32_r32(&mut self, dst: xRegister32, src: xRegister32) {
        self.emit_rr(Code::Test_rm32_r32, reg_to_iced(src), reg_to_iced(dst));
    }

    // ── ALU with immediate ──
    pub fn add_r64_imm(&mut self, dst: xRegister64, imm: i32) {
        self.emit_ri(Code::Add_rm64_imm32, reg_to_iced(dst), imm as u64);
    }
    pub fn sub_r64_imm(&mut self, dst: xRegister64, imm: i32) {
        self.emit_ri(Code::Sub_rm64_imm32, reg_to_iced(dst), imm as u64);
    }
    pub fn cmp_r64_imm(&mut self, dst: xRegister64, imm: i32) {
        self.emit_ri(Code::Cmp_rm64_imm32, reg_to_iced(dst), imm as u64);
    }
    pub fn cmp_r32_imm(&mut self, dst: xRegister32, imm: i32) {
        self.emit_ri(Code::Cmp_rm32_imm32, reg_to_iced(dst), imm as u64);
    }

    // ── Multiplication ──
    pub fn mul_r64(&mut self, src: xRegister64) {
        self.emit_rm(Code::Mul_rm64, reg_to_iced(RAX),
            &xIndirectAddress::new(8).with_base(src));
    }
    pub fn imul_r64_r64(&mut self, dst: xRegister64, src: xRegister64) {
        self.emit_rr(Code::Imul_r64_rm64, reg_to_iced(dst), reg_to_iced(src));
    }

    // ── Bit shifts ──
    pub fn shl_r64_imm(&mut self, dst: xRegister64, imm: u8) {
        self.emit_ri(Code::Shl_rm64_imm8, reg_to_iced(dst), imm as u64);
    }
    pub fn shr_r64_imm(&mut self, dst: xRegister64, imm: u8) {
        self.emit_ri(Code::Shr_rm64_imm8, reg_to_iced(dst), imm as u64);
    }
    pub fn sar_r64_imm(&mut self, dst: xRegister64, imm: u8) {
        self.emit_ri(Code::Sar_rm64_imm8, reg_to_iced(dst), imm as u64);
    }

    // ── NOT / NEG ──
    pub fn not_r64(&mut self, dst: xRegister64) {
        self.emit_rm(Code::Not_rm64, reg_to_iced(dst), &xIndirectAddress::new(8).with_base(dst));
    }
    pub fn neg_r64(&mut self, dst: xRegister64) {
        self.emit_rm(Code::Neg_rm64, reg_to_iced(dst), &xIndirectAddress::new(8).with_base(dst));
    }

    // ── Stack ──
    pub fn push_r64(&mut self, reg: xRegister64) {
        self.emit(Instruction::with1(Code::Push_r64, reg_to_iced(reg)).expect("push"));
    }
    pub fn pop_r64(&mut self, reg: xRegister64) {
        self.emit(Instruction::with1(Code::Pop_r64, reg_to_iced(reg)).expect("pop"));
    }
    pub fn ret(&mut self) {
        self.emit_code(Code::Retnq);
    }

    // ── Control flow ──
    pub fn jmp_label(&mut self, label: u32) { self.emit_branch(Code::Jmp_rel32_64, label); }
    pub fn call_label(&mut self, label: u32) { self.emit_branch(Code::Call_rel32_64, label); }
    pub fn jcc_label(&mut self, code: Code, label: u32) { self.emit_branch(code, label); }

    pub fn je_label(&mut self, label: u32)  { self.jcc_label(Code::Je_rel32_64, label); }
    pub fn jne_label(&mut self, label: u32) { self.jcc_label(Code::Jne_rel32_64, label); }
    pub fn jb_label(&mut self, label: u32)  { self.jcc_label(Code::Jb_rel32_64, label); }
    pub fn jae_label(&mut self, label: u32) { self.jcc_label(Code::Jae_rel32_64, label); }
    pub fn jl_label(&mut self, label: u32)  { self.jcc_label(Code::Jl_rel32_64, label); }
    pub fn jge_label(&mut self, label: u32) { self.jcc_label(Code::Jge_rel32_64, label); }
    pub fn jle_label(&mut self, label: u32) { self.jcc_label(Code::Jle_rel32_64, label); }
    pub fn jg_label(&mut self, label: u32)  { self.jcc_label(Code::Jg_rel32_64, label); }
    pub fn js_label(&mut self, label: u32)  { self.jcc_label(Code::Js_rel32_64, label); }
    pub fn jns_label(&mut self, label: u32) { self.jcc_label(Code::Jns_rel32_64, label); }

    // ── NOP / LEA ──
    pub fn nop(&mut self) { self.emit_code(Code::Nopd); }
    pub fn lea_r64_mem(&mut self, dst: xRegister64, addr: &xIndirectAddress) {
        self.emit_rm(Code::Lea_r64_m, reg_to_iced(dst), addr);
    }

    // ── SSE logical ──
    pub fn xorps_xmm_xmm(&mut self, dst: xRegisterSSE, src: xRegisterSSE) {
        self.emit_rr(Code::Xorps_xmm_xmmm128, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn pxor_xmm_xmm(&mut self, dst: xRegisterSSE, src: xRegisterSSE) {
        self.emit_rr(Code::Pxor_xmm_xmmm128, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn pand_xmm_xmm(&mut self, dst: xRegisterSSE, src: xRegisterSSE) {
        self.emit_rr(Code::Pand_xmm_xmmm128, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn por_xmm_xmm(&mut self, dst: xRegisterSSE, src: xRegisterSSE) {
        self.emit_rr(Code::Por_xmm_xmmm128, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn paddd_xmm_xmm(&mut self, dst: xRegisterSSE, src: xRegisterSSE) {
        self.emit_rr(Code::Paddd_xmm_xmmm128, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn paddw_xmm_xmm(&mut self, dst: xRegisterSSE, src: xRegisterSSE) {
        self.emit_rr(Code::Paddw_xmm_xmmm128, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn psubd_xmm_xmm(&mut self, dst: xRegisterSSE, src: xRegisterSSE) {
        self.emit_rr(Code::Psubd_xmm_xmmm128, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn psubw_xmm_xmm(&mut self, dst: xRegisterSSE, src: xRegisterSSE) {
        self.emit_rr(Code::Psubw_xmm_xmmm128, reg_to_iced(dst), reg_to_iced(src));
    }

    // ── SSE arithmetic ──
    pub fn addss_xmm_xmm(&mut self, dst: xRegisterSSE, src: xRegisterSSE) {
        self.emit_rr(Code::Addss_xmm_xmmm32, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn subss_xmm_xmm(&mut self, dst: xRegisterSSE, src: xRegisterSSE) {
        self.emit_rr(Code::Subss_xmm_xmmm32, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn mulss_xmm_xmm(&mut self, dst: xRegisterSSE, src: xRegisterSSE) {
        self.emit_rr(Code::Mulss_xmm_xmmm32, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn divss_xmm_xmm(&mut self, dst: xRegisterSSE, src: xRegisterSSE) {
        self.emit_rr(Code::Divss_xmm_xmmm32, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn cvtsi2ss_xmm_r64(&mut self, dst: xRegisterSSE, src: xRegister64) {
        self.emit_rr(Code::Cvtsi2ss_xmm_rm64, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn cvtss2si_r64_xmm(&mut self, dst: xRegister64, src: xRegisterSSE) {
        self.emit_rr(Code::Cvtss2si_r64_xmmm32, reg_to_iced(dst), reg_to_iced(src));
    }

    // ── SSE shuffle / pack / unpack ──
    pub fn shufps_xmm_xmm_imm(&mut self, dst: xRegisterSSE, src: xRegisterSSE, imm: u8) {
        self.emit(Instruction::with3(Code::Shufps_xmm_xmmm128_imm8, reg_to_iced(dst), reg_to_iced(src), imm as i32).expect("shufps"));
    }
    pub fn punpckldq_xmm_xmm(&mut self, dst: xRegisterSSE, src: xRegisterSSE) {
        self.emit_rr(Code::Punpckldq_xmm_xmmm128, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn punpckhdq_xmm_xmm(&mut self, dst: xRegisterSSE, src: xRegisterSSE) {
        self.emit_rr(Code::Punpckhdq_xmm_xmmm128, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn punpcklqdq_xmm_xmm(&mut self, dst: xRegisterSSE, src: xRegisterSSE) {
        self.emit_rr(Code::Punpcklqdq_xmm_xmmm128, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn punpckhqdq_xmm_xmm(&mut self, dst: xRegisterSSE, src: xRegisterSSE) {
        self.emit_rr(Code::Punpckhqdq_xmm_xmmm128, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn pshufd_xmm_xmm_imm(&mut self, dst: xRegisterSSE, src: xRegisterSSE, imm: u8) {
        self.emit(Instruction::with3(Code::Pshufd_xmm_xmmm128_imm8, reg_to_iced(dst), reg_to_iced(src), imm as i32).expect("pshufd"));
    }
    pub fn psrldq_xmm_imm(&mut self, dst: xRegisterSSE, imm: u8) {
        self.emit(Instruction::with3(Code::Psrldq_xmm_imm8, reg_to_iced(dst), reg_to_iced(dst), imm as i32).expect("psrldq"));
    }
    pub fn pslldq_xmm_imm(&mut self, dst: xRegisterSSE, imm: u8) {
        self.emit(Instruction::with3(Code::Pslldq_xmm_imm8, reg_to_iced(dst), reg_to_iced(dst), imm as i32).expect("pslldq"));
    }
    pub fn psrlw_xmm_imm(&mut self, dst: xRegisterSSE, imm: u8) {
        self.emit(Instruction::with3(Code::Psrlw_xmm_imm8, reg_to_iced(dst), reg_to_iced(dst), imm as i32).expect("psrlw"));
    }
    pub fn psrld_xmm_imm(&mut self, dst: xRegisterSSE, imm: u8) {
        self.emit(Instruction::with3(Code::Psrld_xmm_imm8, reg_to_iced(dst), reg_to_iced(dst), imm as i32).expect("psrld"));
    }
    pub fn psllw_xmm_imm(&mut self, dst: xRegisterSSE, imm: u8) {
        self.emit(Instruction::with3(Code::Psllw_xmm_imm8, reg_to_iced(dst), reg_to_iced(dst), imm as i32).expect("psllw"));
    }
    pub fn pslld_xmm_imm(&mut self, dst: xRegisterSSE, imm: u8) {
        self.emit(Instruction::with3(Code::Pslld_xmm_imm8, reg_to_iced(dst), reg_to_iced(dst), imm as i32).expect("pslld"));
    }

    // ── SSE comparison ──
    pub fn ucomiss_xmm_xmm(&mut self, dst: xRegisterSSE, src: xRegisterSSE) {
        self.emit_rr(Code::Ucomiss_xmm_xmmm32, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn cmpps_xmm_xmm_imm(&mut self, dst: xRegisterSSE, src: xRegisterSSE, imm: u8) {
        self.emit(Instruction::with3(Code::Cmpps_xmm_xmmm128_imm8, reg_to_iced(dst), reg_to_iced(src), imm as i32).expect("cmpps"));
    }
    pub fn pcmpeqd_xmm_xmm(&mut self, dst: xRegisterSSE, src: xRegisterSSE) {
        self.emit_rr(Code::Pcmpeqd_xmm_xmmm128, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn pcmpeqw_xmm_xmm(&mut self, dst: xRegisterSSE, src: xRegisterSSE) {
        self.emit_rr(Code::Pcmpeqw_xmm_xmmm128, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn pcmpgtd_xmm_xmm(&mut self, dst: xRegisterSSE, src: xRegisterSSE) {
        self.emit_rr(Code::Pcmpgtd_xmm_xmmm128, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn pmaxsw_xmm_xmm(&mut self, dst: xRegisterSSE, src: xRegisterSSE) {
        self.emit_rr(Code::Pmaxsw_xmm_xmmm128, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn pminsw_xmm_xmm(&mut self, dst: xRegisterSSE, src: xRegisterSSE) {
        self.emit_rr(Code::Pminsw_xmm_xmmm128, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn pmaxub_xmm_xmm(&mut self, dst: xRegisterSSE, src: xRegisterSSE) {
        self.emit_rr(Code::Pmaxub_xmm_xmmm128, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn pminub_xmm_xmm(&mut self, dst: xRegisterSSE, src: xRegisterSSE) {
        self.emit_rr(Code::Pminub_xmm_xmmm128, reg_to_iced(dst), reg_to_iced(src));
    }

    // ── Packed sign extension ──
    pub fn pmovsxbd_xmm_xmm(&mut self, dst: xRegisterSSE, src: xRegisterSSE) {
        self.emit_rr(Code::Pmovsxbd_xmm_xmmm32, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn pmovsxbw_xmm_xmm(&mut self, dst: xRegisterSSE, src: xRegisterSSE) {
        self.emit_rr(Code::Pmovsxbw_xmm_xmmm64, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn pmovsxwd_xmm_xmm(&mut self, dst: xRegisterSSE, src: xRegisterSSE) {
        self.emit_rr(Code::Pmovsxwd_xmm_xmmm64, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn pmovzxbd_xmm_xmm(&mut self, dst: xRegisterSSE, src: xRegisterSSE) {
        self.emit_rr(Code::Pmovzxbd_xmm_xmmm32, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn pmovzxbw_xmm_xmm(&mut self, dst: xRegisterSSE, src: xRegisterSSE) {
        self.emit_rr(Code::Pmovzxbw_xmm_xmmm64, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn pmovzxwd_xmm_xmm(&mut self, dst: xRegisterSSE, src: xRegisterSSE) {
        self.emit_rr(Code::Pmovzxwd_xmm_xmmm64, reg_to_iced(dst), reg_to_iced(src));
    }

    // ── MOVMSKPS / PMOVMSKB ──
    pub fn movmskps_r32_xmm(&mut self, dst: xRegister32, src: xRegisterSSE) {
        self.emit_rr(Code::Movmskps_r32_xmm, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn movmskpd_r32_xmm(&mut self, dst: xRegister32, src: xRegisterSSE) {
        self.emit_rr(Code::Movmskpd_r32_xmm, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn pmovmskb_r32_xmm(&mut self, dst: xRegister32, src: xRegisterSSE) {
        self.emit_rr(Code::Pmovmskb_r32_xmm, reg_to_iced(dst), reg_to_iced(src));
    }

    // ── CMPXCHG / XADD ──
    pub fn cmpxchg_mem_r64(&mut self, addr: &xIndirectAddress, src: xRegister64) {
        self.emit_mr(Code::Cmpxchg_rm64_r64, addr, reg_to_iced(src));
    }
    pub fn xadd_mem_r64(&mut self, addr: &xIndirectAddress, src: xRegister64) {
        self.emit_mr(Code::Xadd_rm64_r64, addr, reg_to_iced(src));
    }

    // ── Bit operations ──
    pub fn bsf_r64_r64(&mut self, dst: xRegister64, src: xRegister64) {
        self.emit_rr(Code::Bsf_r64_rm64, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn bsr_r64_r64(&mut self, dst: xRegister64, src: xRegister64) {
        self.emit_rr(Code::Bsr_r64_rm64, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn bt_mem_r32(&mut self, addr: &xIndirectAddress, src: xRegister32) {
        self.emit_mr(Code::Bt_rm32_r32, addr, reg_to_iced(src));
    }

    // ── MXCSR ──
    pub fn ldmxcsr_mem(&mut self, addr: &xIndirectAddress) {
        self.emit_rm(Code::Ldmxcsr_m32, Register::None, addr);
    }
    pub fn stmxcsr_mem(&mut self, addr: &xIndirectAddress) {
        self.emit_mr(Code::Stmxcsr_m32, addr, Register::None);
    }

        // ── Single-operand rm64 helpers ──
    fn emit_rm1(&mut self, code: Code, op: Register) {
        self.emit(Instruction::with1(code, op).expect("emit_rm1"));
    }

    // ── INC / DEC ──
    pub fn inc_rm64(&mut self, dst: xRegister64) {
        self.emit_rm1(Code::Inc_rm64, reg_to_iced(dst));
    }
    pub fn dec_rm64(&mut self, dst: xRegister64) {
        self.emit_rm1(Code::Dec_rm64, reg_to_iced(dst));
    }

    // ── ADC / SBB (register) ──
    pub fn adc_r64_r64(&mut self, dst: xRegister64, src: xRegister64) {
        self.emit_rr(Code::Adc_r64_rm64, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn sbb_r64_r64(&mut self, dst: xRegister64, src: xRegister64) {
        self.emit_rr(Code::Sbb_r64_rm64, reg_to_iced(dst), reg_to_iced(src));
    }

    // ── DIV / IDIV ──
    pub fn div_rm64(&mut self, src: xRegister64) {
        self.emit_rm1(Code::Div_rm64, reg_to_iced(src));
    }
    pub fn idiv_rm64(&mut self, src: xRegister64) {
        self.emit_rm1(Code::Idiv_rm64, reg_to_iced(src));
    }

    // ── CDQE (sign-extend EAX→RAX) ──
    pub fn cdqe(&mut self) {
        self.emit_code(Code::Cdqe);
    }

    // ── CMOVcc ──
    pub fn cmove_r64_r64(&mut self, dst: xRegister64, src: xRegister64) {
        self.emit_rr(Code::Cmove_r64_rm64, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn cmovne_r64_r64(&mut self, dst: xRegister64, src: xRegister64) {
        self.emit_rr(Code::Cmovne_r64_rm64, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn cmovb_r64_r64(&mut self, dst: xRegister64, src: xRegister64) {
        self.emit_rr(Code::Cmovb_r64_rm64, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn cmovae_r64_r64(&mut self, dst: xRegister64, src: xRegister64) {
        self.emit_rr(Code::Cmovae_r64_rm64, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn cmovl_r64_r64(&mut self, dst: xRegister64, src: xRegister64) {
        self.emit_rr(Code::Cmovl_r64_rm64, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn cmovge_r64_r64(&mut self, dst: xRegister64, src: xRegister64) {
        self.emit_rr(Code::Cmovge_r64_rm64, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn cmovle_r64_r64(&mut self, dst: xRegister64, src: xRegister64) {
        self.emit_rr(Code::Cmovle_r64_rm64, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn cmovg_r64_r64(&mut self, dst: xRegister64, src: xRegister64) {
        self.emit_rr(Code::Cmovg_r64_rm64, reg_to_iced(dst), reg_to_iced(src));
    }

    // ── SETcc (set byte to 0/1) ──
    // Note: SETcc operand is always rm8. We map to 8-bit version of the GPR.
    fn r64_to_r8(&self, reg: xRegister64) -> Register {
        // iced-x86 1.21 8-bit register variants:
        // AL, CL, DL, BL, AH, CH, DH, BH, SPL, BPL, SIL, DIL, R8L..R15L
        match reg.id {
            0 => Register::AL,
            1 => Register::CL,
            2 => Register::DL,
            3 => Register::BL,
            4 => Register::AH,
            5 => Register::CH,
            6 => Register::DH,
            7 => Register::BH,
            8 => Register::R8L,
            9 => Register::R9L,
            10 => Register::R10L,
            11 => Register::R11L,
            12 => Register::R12L,
            13 => Register::R13L,
            14 => Register::R14L,
            15 => Register::R15L,
            _ => Register::AL,
        }
    }

    pub fn sete_rm8(&mut self, dst: xRegister64) {
        self.emit_rm1(Code::Sete_rm8, self.r64_to_r8(dst));
    }
    pub fn setne_rm8(&mut self, dst: xRegister64) {
        self.emit_rm1(Code::Setne_rm8, self.r64_to_r8(dst));
    }
    pub fn setb_rm8(&mut self, dst: xRegister64) {
        self.emit_rm1(Code::Setb_rm8, self.r64_to_r8(dst));
    }
    pub fn setae_rm8(&mut self, dst: xRegister64) {
        self.emit_rm1(Code::Setae_rm8, self.r64_to_r8(dst));
    }
    pub fn setl_rm8(&mut self, dst: xRegister64) {
        self.emit_rm1(Code::Setl_rm8, self.r64_to_r8(dst));
    }
    pub fn setge_rm8(&mut self, dst: xRegister64) {
        self.emit_rm1(Code::Setge_rm8, self.r64_to_r8(dst));
    }
    pub fn setle_rm8(&mut self, dst: xRegister64) {
        self.emit_rm1(Code::Setle_rm8, self.r64_to_r8(dst));
    }
    pub fn setg_rm8(&mut self, dst: xRegister64) {
        self.emit_rm1(Code::Setg_rm8, self.r64_to_r8(dst));
    }

    // ── BSWAP ──
    pub fn bswap_r64(&mut self, dst: xRegister64) {
        self.emit_rm1(Code::Bswap_r64, reg_to_iced(dst));
    }

    // ── MOVBE (big-endian move) ──
    pub fn movbe_r64_mem(&mut self, dst: xRegister64, addr: &xIndirectAddress) {
        self.emit_rm(Code::Movbe_r64_m64, reg_to_iced(dst), addr);
    }

    // ── String ops (implicit AL/EAX/RAX) ──
    pub fn stosb(&mut self) { self.emit_code(Code::Stosb_m8_AL); }
    pub fn lodsb(&mut self) { self.emit_code(Code::Lodsb_AL_m8); }

    // ── SSE extra logical ──
    pub fn pandn_xmm_xmm(&mut self, dst: xRegisterSSE, src: xRegisterSSE) {
        self.emit_rr(Code::Pandn_xmm_xmmm128, reg_to_iced(dst), reg_to_iced(src));
    }

    // ── SSE move high/low ──
    pub fn movhlps_xmm_xmm(&mut self, dst: xRegisterSSE, src: xRegisterSSE) {
        self.emit_rr(Code::Movhlps_xmm_xmm, reg_to_iced(dst), reg_to_iced(src));
    }
    pub fn movlhps_xmm_xmm(&mut self, dst: xRegisterSSE, src: xRegisterSSE) {
        self.emit_rr(Code::Movlhps_xmm_xmm, reg_to_iced(dst), reg_to_iced(src));
    }

    // ── Fences ──
    pub fn mfence(&mut self) { self.emit_code(Code::Mfence); }
    pub fn lfence(&mut self) { self.emit_code(Code::Lfence); }
    pub fn sfence(&mut self) { self.emit_code(Code::Sfence); }

    // ── MMX ──
    pub fn emms(&mut self) { self.emit_code(Code::Emms); }

    // ── CPUID ──
    pub fn cpuid(&mut self) { self.emit_code(Code::Cpuid); }
}

impl Default for Emitter {
    fn default() -> Self { Self::new() }
}

// ---------------------------------------------------------------------------
// xFastCall
// ---------------------------------------------------------------------------

/// Generate a Microsoft x64 fastcall sequence.
pub fn xFastCall(emit: &mut Emitter, func_ptr: *mut u8, arg1: Option<xRegister64>, arg2: Option<xRegister64>) {
    emit.sub_r64_imm(RSP, 32);
    if let Some(a1) = arg1 { emit.mov_r64_r64(RCX, a1); }
    if let Some(a2) = arg2 { emit.mov_r64_r64(RDX, a2); }
    emit.mov_r64_imm(RAX, func_ptr as i64);
    emit.emit_rm(Code::Call_rm64, reg_to_iced(RAX), &xIndirectAddress::new(8).with_base(RAX));
    emit.add_r64_imm(RSP, 32);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mov_alu_ret() {
        let mut e = Emitter::new();
        e.mov_r64_imm(RAX, 0x1234);
        e.add_r64_r64(RAX, RCX);
        e.ret();
        let bytes = e.take();
        assert!(bytes.len() >= 10, "got {} bytes", bytes.len());
    }

    #[test]
    fn push_pop() {
        let mut e = Emitter::new();
        e.push_r64(RBP);
        e.mov_r64_imm(RAX, 0xCAFE_BABE);
        e.pop_r64(RBP);
        e.ret();
        let bytes = e.take();
        assert_eq!(bytes.len(), 13, "{:02x?}", bytes);
    }

    #[test]
    fn forward_jmp() {
        let mut e = Emitter::new();
        let l = e.define_label();
        e.jmp_label(l);
        e.nop();
        e.place_label(l);
        e.ret();
        let bytes = e.take();
        assert_eq!(bytes.len(), 7, "{:02x?}", bytes);
        assert_eq!(&bytes[1..5], &[1, 0, 0, 0]);
    }

    #[test]
    fn sse_instructions() {
        let mut e = Emitter::new();
        let xmm0 = xRegisterSSE::new(0, 16, xRegisterType::XMM);
        let xmm1 = xRegisterSSE::new(1, 16, xRegisterType::XMM);
        e.xorps_xmm_xmm(xmm0, xmm0);
        e.movaps_xmm_xmm(xmm1, xmm0);
        e.ret();
        let bytes = e.take();
        assert!(!bytes.is_empty());
    }

    #[test]
    fn memory_addressing() {
        let mut e = Emitter::new();
        let addr = xComplexAddress(8, RSI, Some(RAX), 2, 0x10);
        e.mov_r64_mem(RAX, &addr);
        e.ret();
        let bytes = e.take();
        assert!(!bytes.is_empty());
    }

    #[test]
    fn xfastcall_test() {
        let mut e = Emitter::new();
        let func_ptr = std::ptr::null_mut();
        xFastCall(&mut e, func_ptr, Some(RAX), Some(RCX));
        let bytes = e.take();
        assert!(bytes.len() > 20, "{:02x?}", bytes);
    }
}
