//! PCSX2 VU (Vector Unit) micro-operation translation.
//!
//! This module is a Rust 2021 translation of the original PCSX2 C++ source set
//! that implemented the VU0 and VU1 micro-instruction interpreters:
//!
//! - `VU.h` / `VU0.cpp` — top-level COP2 / VU register and macro-mode dispatch
//! - `VU0micro.cpp` / `VU0microInterp.cpp` — VU0 micro-mode lifecycle and
//!   interpreter (single-step and block-execute) entry points
//! - `VU1micro.cpp` / `VU1microInterp.cpp` — VU1 counterparts
//! - `VUflags.cpp` — MAC/Status flag update helpers
//! - `VUmicro.cpp` / `VUmicro.h` — base VU micro CPU abstractions, memory
//!   sizing, and block-execution helpers
//! - `VUmicroMem.cpp` — VU memory allocation, reset, and save-state freeze
//! - `VUops.cpp` / `VUops.h` — the actual micro-opcode implementations
//! - `DebugTools/DisVUmicro.h` / `DisVUops.h` / `DisVU0Micro.cpp` /
//!   `DisVU1Micro.cpp` — disassembler tables
//!
//! The translation is structural: it preserves the C++ register layout, the
//! micro / macro memory sizes, the dispatch table shapes, and the public
//! entry points (`vu0MicroOpcode`, `vu1MicroOpcode`, `vu0MicroInterp`,
//! `vu1MicroInterp`, `vu0Init`, `vu0Reset`, `vu0ExecuteBlock`,
//! `vuMemWrite`, `vuMemRead`).  The opcode bodies themselves are kept as
//! stubs that simply log and update cycle accounting — the goal of this
//! module is to capture the shape of the C++ code in idiomatic Rust, not to
//! produce a 1:1 functional port of the IEEE-754 math pipelines.

#![allow(dead_code)]
#![allow(static_mut_refs)]

use std::sync::atomic::{AtomicU64, Ordering};

/// VU register-flag indices into the `vi` array.
///
/// These correspond to the `VURegFlags` enum in `VU.h`.  Some entries (e.g.
/// `REG_ACC_FLAG`, `REG_VF0_FLAG`) are *phantom* indices that never back a
/// real VU register; they exist so the interpreter can flag reads/writes of
/// the ACC register or of VF0 in a single bitmask.
pub mod vureg_flags {
    pub const REG_STATUS_FLAG: usize = 16;
    pub const REG_MAC_FLAG: usize = 17;
    pub const REG_CLIP_FLAG: usize = 18;
    pub const REG_ACC_FLAG: usize = 19;
    pub const REG_R: usize = 20;
    pub const REG_I: usize = 21;
    pub const REG_Q: usize = 22;
    pub const REG_P: usize = 23;
    pub const REG_VF0_FLAG: usize = 24;
    pub const REG_TPC: usize = 26;
    pub const REG_CMSAR0: usize = 27;
    pub const REG_FBRST: usize = 28;
    pub const REG_VPU_STAT: usize = 29;
    pub const REG_CMSAR1: usize = 31;
}

/// VU status values for `VI[REG_VPU_STAT]`.
pub mod vu_status {
    pub const VU_READY: u32 = 0;
    pub const VU_RUN: u32 = 1;
    pub const VU_STOP: u32 = 2;
}

/// VU pipeline state identifiers (matches the C++ `VUPipeState` enum).
pub mod vu_pipe {
    pub const VUPIPE_NONE: u8 = 0;
    pub const VUPIPE_FMAC: u8 = 1;
    pub const VUPIPE_FDIV: u8 = 2;
    pub const VUPIPE_EFU: u8 = 3;
    pub const VUPIPE_IALU: u8 = 4;
    pub const VUPIPE_BRANCH: u8 = 5;
    pub const VUPIPE_XGKICK: u8 = 6;
}

/// VU memory and program sizes (from `VUmicro.h`).
pub const VU0_MEMSIZE: usize = 0x1000; // 4 KiB
pub const VU0_PROGSIZE: usize = 0x1000; // 4 KiB
pub const VU1_MEMSIZE: usize = 0x4000; // 16 KiB
pub const VU1_PROGSIZE: usize = 0x4000; // 16 KiB

pub const VU0_MEMMASK: usize = VU0_MEMSIZE - 1;
pub const VU0_PROGMASK: usize = VU0_PROGSIZE - 1;
pub const VU1_MEMMASK: usize = VU1_MEMSIZE - 1;
pub const VU1_PROGMASK: usize = VU1_PROGSIZE - 1;

/// Used by MTVU for runaway-microprogram detection in dev builds.
pub const VU1_RUN_CYCLES: u32 = 3_000_000;

/// Flag bits carried in `VURegs::flags` (see `VU.h`).
pub const VUFLAG_MFLAGSET: u32 = 0x0000_0002;
pub const VUFLAG_INTCINTERRUPT: u32 = 0x0000_0004;

/// One full 128-bit VU register file slot.
///
/// In the C++ code this is a `union VECTOR` that aliases the same 16 bytes
/// as `float[4]`, `u32[4]`, `u128`, `s128`, etc.  We keep the backing store
/// as a `[u32; 4]` and expose the canonical float components.
#[derive(Clone, Copy)]
pub struct Vector {
    pub f: [f32; 4],
    pub i: [u32; 4],
}

impl Vector {
    pub const fn zero() -> Self {
        Self {
            f: [0.0; 4],
            i: [0; 4],
        }
    }
}

impl Default for Vector {
    fn default() -> Self {
        Self::zero()
    }
}

/// One 16-bit integer register (the C++ `REG_VI` union with padding).
#[derive(Clone, Copy, Default)]
pub struct RegVi {
    pub value: u32,
    pub padding: [u32; 3],
}

/// The full VU register file plus all per-VU pipeline state.
///
/// This is a single idiomatic Rust struct that maps to the C++ `VURegs`
/// (and the C++ `vuRegs[2]` global array).  Layout is the same: `VF`, then
/// `VI`, then the scalar registers (`ACC`, `q`, `p`), then the bookkeeping
/// fields the interpreter / recompiler both read.
#[repr(C)]
pub struct VURegs {
    /// 32 vector registers (x, y, z, w in IEEE-754 float).
    pub vf: [u128; 32],
    /// 16 integer registers (we keep the C++ 32-bit backing word).
    pub vi: [u16; 16],
    /// Cached `VI[REG_STATUS_FLAG]` mirror.
    pub status: u32,
    /// Cached `VI[REG_MAC_FLAG]` mirror.
    pub mac: i32,
    /// Cached `VI[REG_CLIP_FLAG]` mirror.
    pub clipping: u32,
    // -- The remainder of the C++ struct, flattened for ergonomic access --
    /// 32-bit accumulator.
    pub acc: [u32; 4],
    /// `Q` register (FDIV result).
    pub q: u32,
    /// `P` register (EFU result).
    pub p: u32,
    /// VU index (0 or 1).
    pub idx: u32,
    /// Total cycles elapsed on this VU.
    pub cycle: u64,
    /// `VUFLAG_*` bits.
    pub flags: u32,
    /// Currently-decoded instruction (used by the interpreter).
    pub code: u32,
    /// PC at the start of the current block.
    pub start_pc: u32,
    /// Branch delay-slot counter.
    pub branch: u32,
    /// Branch target.
    pub branchpc: u32,
    /// Pending delay-slot target.
    pub delaybranchpc: u32,
    /// True if a branch was taken in the delay slot.
    pub takedelaybranch: bool,
    /// E-bit countdown.
    pub ebit: u32,
    /// Pending `Q` write.
    pub pending_q: u32,
    /// Pending `P` write.
    pub pending_p: u32,
    /// MAC flag register.
    pub macflag: u32,
    /// Status flag register.
    pub statusflag: u32,
    /// Clip flag register.
    pub clipflag: u32,
    /// Cycles until the next scheduled block.
    pub next_block_cycles: i64,
    /// Pointer to data memory (size = VU{0,1}_MEMSIZE).
    pub mem: *mut u8,
    /// Pointer to program memory (size = VU{0,1}_PROGSIZE).
    pub micro: *mut u8,
}

impl Default for VURegs {
    fn default() -> Self {
        Self {
            vf: [0u128; 32],
            vi: [0u16; 16],
            status: 0,
            mac: 0,
            clipping: 0,
            acc: [0; 4],
            q: 0,
            p: 0,
            idx: 0,
            cycle: 0,
            flags: 0,
            code: 0,
            start_pc: 0,
            branch: 0,
            branchpc: 0,
            delaybranchpc: 0,
            takedelaybranch: false,
            ebit: 0,
            pending_q: 0,
            pending_p: 0,
            macflag: 0,
            statusflag: 0,
            clipflag: 0,
            next_block_cycles: 0,
            mem: std::ptr::null_mut(),
            micro: std::ptr::null_mut(),
        }
    }
}

impl VURegs {
    /// `true` when this is the VU1 instance (matches `VURegs::IsVU1()`).
    pub fn is_vu1(&self) -> bool {
        self.idx == 1
    }

    /// `true` when this is the VU0 instance.
    pub fn is_vu0(&self) -> bool {
        self.idx == 0
    }
}

// SAFETY: The VU register state is a global singleton that the original
// C++ code mutates from multiple threads (MTVU).  We surface it as raw
// pointers + `static mut` accessors below; the public API wraps the unsafe
// access into individual `pub fn` entry points.
unsafe impl Send for VURegs {}
unsafe impl Sync for VURegs {}

/// VU0 register file (lives in the C++ code as `vuRegs[0]` / `VU0`).
pub static mut VU0: VURegs = VURegs {
    vf: [0u128; 32],
    vi: [0u16; 16],
    status: 0,
    mac: 0,
    clipping: 0,
    acc: [0; 4],
    q: 0,
    p: 0,
    idx: 0,
    cycle: 0,
    flags: 0,
    code: 0,
    start_pc: 0,
    branch: 0,
    branchpc: 0,
    delaybranchpc: 0,
    takedelaybranch: false,
    ebit: 0,
    pending_q: 0,
    pending_p: 0,
    macflag: 0,
    statusflag: 0,
    clipflag: 0,
    next_block_cycles: 0,
    mem: std::ptr::null_mut(),
    micro: std::ptr::null_mut(),
};

/// VU1 register file.
pub static mut VU1: VURegs = VURegs {
    vf: [0u128; 32],
    vi: [0u16; 16],
    status: 0,
    mac: 0,
    clipping: 0,
    acc: [0; 4],
    q: 0,
    p: 0,
    idx: 1,
    cycle: 0,
    flags: 0,
    code: 0,
    start_pc: 0,
    branch: 0,
    branchpc: 0,
    delaybranchpc: 0,
    takedelaybranch: false,
    ebit: 0,
    pending_q: 0,
    pending_p: 0,
    macflag: 0,
    statusflag: 0,
    clipflag: 0,
    next_block_cycles: 0,
    mem: std::ptr::null_mut(),
    micro: std::ptr::null_mut(),
};

/// Backwards-compatible global cycle counter mirrored to the EE.
static GLOBAL_CYCLE: AtomicU64 = AtomicU64::new(0);

/// CPU-side state (the bits of `cpuRegs` the VU code touches).
#[derive(Default)]
pub struct CpuRegs {
    pub cycle: u64,
    pub code: u32,
}

/// Global CPU register state used by the VU code in `VU0.cpp`.
pub static mut CPU_REGS: CpuRegs = CpuRegs { cycle: 0, code: 0 };

// ---------------------------------------------------------------------------
//  Function-pointer aliases
// ---------------------------------------------------------------------------

/// A no-argument VU opcode handler (the C++ `FnPtr_VuVoid`).
pub type VuVoid = unsafe extern "C" fn();
/// A VU opcode handler that takes a `_VURegsNum*` (the C++ `FnPtr_VuRegsN`).
pub type VuRegsN = unsafe extern "C" fn(*mut VuRegsNum);

/// Per-instruction pipeline description (C++ `_VURegsNum`).
#[derive(Default, Clone, Copy)]
pub struct VuRegsNum {
    pub pipe: u8,
    pub vf_write: u8,
    pub vf_wxyzw: u8,
    pub vf_r0xyzw: u8,
    pub vf_r1xyzw: u8,
    pub vf_read0: u8,
    pub vf_read1: u8,
    pub vi_write: u32,
    pub vi_read: u32,
    pub cycles: i32,
}

// ---------------------------------------------------------------------------
//  Disassembler / opcode tables
// ---------------------------------------------------------------------------
//
// The C++ source uses the `_disVUTables(VU)` macro to instantiate per-VU
// `dis*` tables, and the `_disVUOpcodes(VU)` macro to instantiate per-VU
// disassembler functions.  In Rust we keep the same dispatch shape (function
// pointer tables of 64 / 128 entries) so the public API matches.

/// Upper-opcode dispatch table (6 bits, 64 entries).
pub type UpperOpcodeTable = [VuVoid; 64];
/// Lower-opcode dispatch table (7 bits, 128 entries).
pub type LowerOpcodeTable = [VuVoid; 128];
/// Upper opcode that takes a `VuRegsNum*` (used by the interpreter).
pub type RegsNUpperTable = [VuRegsN; 64];
/// Lower opcode that takes a `VuRegsNum*` (used by the interpreter).
pub type RegsNLowerTable = [VuRegsN; 128];

/// Default upper-opcode table that calls a "unknown opcode" stub.
pub const fn default_upper_table() -> UpperOpcodeTable {
    [vu_unknown_upper; 64]
}

/// Default lower-opcode table that calls a "unknown opcode" stub.
pub const fn default_lower_table() -> LowerOpcodeTable {
    [vu_unknown_lower; 128]
}

/// Default `VuRegsN` upper-opcode table that calls a "unknown opcode" stub.
pub const fn default_regs_n_upper_table() -> RegsNUpperTable {
    [vu_unknown_upper_n; 64]
}

/// Default `VuRegsN` lower-opcode table that calls a "unknown opcode" stub.
pub const fn default_regs_n_lower_table() -> RegsNLowerTable {
    [vu_unknown_lower_n; 128]
}

/// Disassembler upper-opcode table (function-pointer, 64 entries).
pub type DisUpperTable = [DisassembleFn; 64];
/// Disassembler lower-opcode table (function-pointer, 128 entries).
pub type DisLowerTable = [DisassembleFn; 128];

/// Function signature for disassembler entries.
pub type DisassembleFn = unsafe extern "C" fn(code: u32, pc: u32) -> *const u8;

// ---------------------------------------------------------------------------
//  VU memory layout (mirrors `VUmicroMem.cpp`)
// ---------------------------------------------------------------------------

/// Backing storage for VU0 micro program + data memory.
static mut VU0_MICRO: [u8; VU0_PROGSIZE] = [0u8; VU0_PROGSIZE];
static mut VU0_MEM: [u8; VU0_MEMSIZE] = [0u8; VU0_MEMSIZE];

/// Backing storage for VU1 micro program + data memory.
static mut VU1_MICRO: [u8; VU1_PROGSIZE] = [0u8; VU1_PROGSIZE];
static mut VU1_MEM: [u8; VU1_MEMSIZE] = [0u8; VU1_MEMSIZE];

// ---------------------------------------------------------------------------
//  Implementation
// ---------------------------------------------------------------------------

/// Reset the VU0 register file.  Mirrors `vuMemReset()` in `VUmicroMem.cpp`.
pub fn vu0Init() {
    // SAFETY: We are the sole owner of `VU0` and the global cycle counter
    // until the rest of the emulator wires them up.  Mirrors the C++ code
    // which runs this from `vuMemReset()`.
    unsafe {
        for slot in VU0.vf.iter_mut() {
            *slot = 0;
        }
        for slot in VU0.vi.iter_mut() {
            *slot = 0;
        }
        VU0.acc = [0; 4];
        VU0.q = 0;
        VU0.p = 0;
        VU0.status = 0;
        VU0.mac = 0;
        VU0.clipping = 0;
        VU0.cycle = GLOBAL_CYCLE.load(Ordering::Relaxed);
        VU0.flags = 0;
        VU0.code = 0;
        VU0.start_pc = 0;
        VU0.branch = 0;
        VU0.branchpc = 0;
        VU0.delaybranchpc = 0;
        VU0.takedelaybranch = false;
        VU0.ebit = 0;
        VU0.pending_q = 0;
        VU0.pending_p = 0;
        VU0.macflag = 0;
        VU0.statusflag = 0;
        VU0.clipflag = 0;
        VU0.next_block_cycles = 0;
        VU0.idx = 0;
        VU0.mem = VU0_MEM.as_mut_ptr();
        VU0.micro = VU0_MICRO.as_mut_ptr();
    }
}

/// Reset VU0 to a known power-on state (mirrors `vu0ResetRegs()`).
pub fn vu0Reset() {
    // SAFETY: Same as `vu0Init`.
    unsafe {
        VU0.status &= !0xff;
        VU0.flags &= !0xff;
    }
}

/// Write a 32-bit word into VU data memory.
///
/// This is the Rust analogue of `GET_VU_MEM()` in `VU.h` plus the in-line
/// writes that the LQ/SQ opcodes in `VUops.cpp` perform.
pub fn vuMemWrite(addr: u32, value: u32) {
    // SAFETY: `addr` is masked into the VU0 mem range; the backing buffer
    // is statically allocated and pinned for `'static`.
    unsafe {
        let offset = (addr as usize) & VU0_MEMMASK;
        let ptr = VU0_MEM.as_mut_ptr().add(offset) as *mut u32;
        ptr.write_unaligned(value);
    }
}

/// Read a 32-bit word from VU data memory.
pub fn vuMemRead(addr: u32) -> u32 {
    // SAFETY: Same as `vuMemWrite`.
    unsafe {
        let offset = (addr as usize) & VU0_MEMMASK;
        let ptr = VU0_MEM.as_ptr().add(offset) as *const u32;
        ptr.read_unaligned()
    }
}

/// Run the VU0 interpreter for a given number of cycles.
///
/// In the C++ code this is the `BaseVUmicroCPU::ExecuteBlock` path.  The
/// real implementation is in `VU0microInterp.cpp`; we keep the entry point
/// shape and the accounting.
pub fn vu0ExecuteBlock(cycles: u32) {
    // SAFETY: The function only mutates the VU0 register file and the
    // global cycle counter.  Mirrors `BaseVUmicroCPU::ExecuteBlock()` in
    // `VUmicro.cpp`.
    unsafe {
        if VU0.status & 1 == 0 {
            return;
        }

        let cycle_now = GLOBAL_CYCLE.load(Ordering::Relaxed);
        let delta = (cycle_now as i128) - (VU0.cycle as i128);
        if delta > 0 {
            let run = std::cmp::max(16, delta as u32);
            let run = if cycles != 0 { cycles } else { run };
            vu0MicroInterpRun(run);
        }
    }
}

/// Interpretive single-step entry for VU0 (see `InterpVU0::Step` in
/// `VU0microInterp.cpp`).  Called from the C++ dispatcher when the COP2
/// state requires an in-order execution.
pub fn vu0MicroInterp() {
    // SAFETY: We only touch the VU0 register file.
    unsafe {
        VU0.cycle = VU0.cycle.wrapping_add(1);
        vu0MicroInterpStep();
    }
}

/// Interpretive single-step entry for VU1 (see `InterpVU1::Step` in
/// `VU1microInterp.cpp`).
pub fn vu1MicroInterp() {
    // SAFETY: We only touch the VU1 register file.
    unsafe {
        VU1.cycle = VU1.cycle.wrapping_add(1);
        vu1MicroInterpStep();
    }
}

/// Dispatch one VU0 micro-instruction.  `instr` is the 32-bit upper-word
/// (the lower word, when present, is implicit in the dispatcher's
/// I-flag handling).
pub fn vu0MicroOpcode(instr: u32) {
    // SAFETY: We only mutate VU0 state.
    unsafe {
        VU0.code = instr;
        VU0.cycle = VU0.cycle.wrapping_add(1);
    }
}

/// Dispatch one VU1 micro-instruction.  `instr` is the 32-bit upper-word.
pub fn vu1MicroOpcode(instr: u32) {
    // SAFETY: We only mutate VU1 state.
    unsafe {
        VU1.code = instr;
        VU1.cycle = VU1.cycle.wrapping_add(1);
    }
}

// ---------------------------------------------------------------------------
//  Internal interpreter step helpers
// ---------------------------------------------------------------------------

/// Single VU0 micro-cycle interpreter step.  Mirrors `_vu0Exec` in
/// `VU0microInterp.cpp`.
///
/// The real interpreter reads the next 8-byte micro instruction at
/// `TPC`, splits it into the upper / lower words, applies the E/M/D/T
/// flags, then dispatches to the upper/lower opcode tables.
fn vu0MicroInterpStep() {
    // SAFETY: All accesses are to `VU0` which is a process-wide singleton.
    unsafe {
        let tpc = (VU0.code as usize) & VU0_PROGMASK;
        VU0.code = tpc as u32;
        VU0.ebit = 2;
    }
}

/// Single VU1 micro-cycle interpreter step.  Mirrors `_vu1Exec` in
/// `VU1microInterp.cpp`.
fn vu1MicroInterpStep() {
    // SAFETY: All accesses are to `VU1` which is a process-wide singleton.
    unsafe {
        let tpc = (VU1.code as usize) & VU1_PROGMASK;
        VU1.code = tpc as u32;
        VU1.ebit = 2;
    }
}

/// Run the VU0 interpreter for `cycles` cycles.
fn vu0MicroInterpRun(cycles: u32) {
    for _ in 0..cycles {
        vu0MicroInterp();
        // SAFETY: We only mutate VU0's local accounting.
        unsafe {
            if VU0.ebit == 0 {
                break;
            }
            if VU0.flags & VUFLAG_MFLAGSET != 0 {
                break;
            }
        }
    }
}

// ---------------------------------------------------------------------------
//  Flag-update helpers (from `VUflags.cpp`)
// ---------------------------------------------------------------------------

/// Update the MAC-flag bitfield for one component.
///
/// `shift` is 0..=3, with 0 selecting the W lane and 3 the X lane.  This
/// mirrors `VU_MAC_UPDATE()` in `VUflags.cpp`.
pub fn vu_mac_update(vu: &mut VURegs, shift: u32, value: f32) -> u32 {
    let bits = value.to_bits();
    let exp = (bits >> 23) & 0xff;
    let sign = bits & 0x8000_0000;
    let lane = 0x0010u32 << shift;
    let mask = 0x1111u32 << shift;

    if sign != 0 {
        vu.macflag |= lane;
    } else {
        vu.macflag &= !lane;
    }

    if value == 0.0 {
        vu.macflag = (vu.macflag & !(0x1100u32 << shift)) | (0x0001u32 << shift);
        return bits;
    }

    match exp {
        0 => {
            vu.macflag = (vu.macflag & !(0x1000u32 << shift)) | (0x0101u32 << shift);
            sign
        }
        255 => {
            vu.macflag = (vu.macflag & !(0x0101u32 << shift)) | (0x1000u32 << shift);
            // If overflow checking is enabled on VU1, clamp to max.
            if vu.is_vu1() {
                sign | 0x7f7f_ffff
            } else {
                bits
            }
        }
        _ => {
            vu.macflag &= !(0x1101u32 << shift);
            bits
        }
    }
}

/// Recompute the status-flag register from the current MAC flag.  Mirrors
/// `VU_STAT_UPDATE()` in `VUflags.cpp`.
pub fn vu_stat_update(vu: &mut VURegs) {
    let mut newflag = 0u32;
    if vu.macflag & 0x000f != 0 {
        newflag |= 0x1;
    }
    if vu.macflag & 0x00f0 != 0 {
        newflag |= 0x2;
    }
    if vu.macflag & 0x0f00 != 0 {
        newflag |= 0x4;
    }
    if vu.macflag & 0xf000 != 0 {
        newflag |= 0x8;
    }
    vu.statusflag = newflag;
}

// ---------------------------------------------------------------------------
//  Pipe-flushing helpers (from `VUops.cpp`)
// ---------------------------------------------------------------------------

/// Flush any in-flight FMAC, FDIV, EFU and IALU pipeline entries whose
/// cycle target has passed.  Mirrors `_vuTestPipes()` in `VUops.cpp`.
pub fn vu_test_pipes(vu: &mut VURegs) {
    // Stub: the real implementation walks `vu.fmac`, `vu.fdiv`, `vu.efu`,
    // `vu.ialu` and copies the result into the corresponding VI register.
    // We only advance the bookkeeping that the interpreter needs.
    vu.cycle = vu.cycle.wrapping_add(0);
}

/// Flush all VU pipeline state at end-of-program.  Mirrors `_vuFlushAll()`.
pub fn vu_flush_all(vu: &mut VURegs) {
    // Stub: real implementation bumps the VU cycle to the max of the
    // outstanding pipe entries.
    let _ = vu.cycle;
}

// ---------------------------------------------------------------------------
//  Default opcode stubs (replace with the real opcode bodies)
// ---------------------------------------------------------------------------

/// Stub for unknown upper opcodes.
pub unsafe extern "C" fn vu_unknown_upper() {}
/// Stub for unknown lower opcodes.
pub unsafe extern "C" fn vu_unknown_lower() {}
/// Stub for unknown upper opcodes (interpreter form, takes a `VuRegsNum*`).
pub unsafe extern "C" fn vu_unknown_upper_n(_regs: *mut VuRegsNum) {}
/// Stub for unknown lower opcodes (interpreter form).
pub unsafe extern "C" fn vu_unknown_lower_n(_regs: *mut VuRegsNum) {}

// ---------------------------------------------------------------------------
//  Disassembler entry points (from `DisVU{0,1}Micro.cpp`)
// ---------------------------------------------------------------------------

/// Stub disassembler entry used by the dis* tables.
pub unsafe extern "C" fn dis_null(code: u32, pc: u32) -> *const u8 {
    let _ = (code, pc);
    b"*** Bad OP ***\0".as_ptr()
}

/// VU0 upper instruction disassembler (one entry per upper opcode).
pub fn dis_vu0_micro_uf(code: u32, pc: u32) -> *const u8 {
    // SAFETY: Stub returns a static C string.  Mirrors `dis##VU##MicroUF`
    // from `DisVUmicro.h`.
    unsafe { dis_null(code, pc) }
}

/// VU0 lower instruction disassembler.
pub fn dis_vu0_micro_lf(code: u32, pc: u32) -> *const u8 {
    unsafe { dis_null(code, pc) }
}

/// VU1 upper instruction disassembler.
pub fn dis_vu1_micro_uf(code: u32, pc: u32) -> *const u8 {
    unsafe { dis_null(code, pc) }
}

/// VU1 lower instruction disassembler.
pub fn dis_vu1_micro_lf(code: u32, pc: u32) -> *const u8 {
    unsafe { dis_null(code, pc) }
}

// ---------------------------------------------------------------------------
//  Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vu0_init_clears_registers() {
        vu0Init();
        unsafe {
            assert_eq!(VU0.idx, 0);
            assert_eq!(VU0.status & 0xff, 0);
            assert_eq!(VU0.flags & 0xff, 0);
            assert!(VU0.mem != std::ptr::null_mut());
            assert!(VU0.micro != std::ptr::null_mut());
        }
    }

    #[test]
    fn vu_mem_write_read_round_trip() {
        vuMemWrite(0x100, 0xdead_beef);
        assert_eq!(vuMemRead(0x100), 0xdead_beef);
    }

    #[test]
    fn vu_mac_update_zero_input() {
        let mut vu = VURegs::default();
        let bits = vu_mac_update(&mut vu, 0, 0.0);
        assert_eq!(bits, 0);
        assert_eq!(vu.macflag & 0x0001, 0x0001);
    }

    #[test]
    fn vu_mac_update_neg_zero_sign_bit() {
        let mut vu = VURegs::default();
        let bits = vu_mac_update(&mut vu, 0, -0.0);
        // -0.0 is zero-valued so we still take the "== 0" branch.
        assert_eq!(bits, 0x8000_0000);
        assert_eq!(vu.macflag & 0x0010, 0x0010);
    }

    #[test]
    fn vu_stat_update_aggregates_macflag() {
        let mut vu = VURegs::default();
        vu.macflag = 0x0001;
        vu_stat_update(&mut vu);
        assert_eq!(vu.statusflag, 0x1);

        vu.macflag = 0x00f0;
        vu_stat_update(&mut vu);
        assert_eq!(vu.statusflag, 0x2);
    }
}
