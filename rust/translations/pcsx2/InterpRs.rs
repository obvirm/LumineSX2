// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.+
//
//! Idiomatic Rust 2021 translation of the PCSX2 MIPS interpreter entry points.
//!
//! This module is a single-file Rust port of the C/C++ sources that implement
//! the R3000A (IOP) and R5900 (EE) interpreter state, the global register
//! banks, and the dispatch glue used by the rest of the emulator.
//!
//! Files translated:
//!   * `pcsx2/R3000A.{h,cpp}`
//!   * `pcsx2/R3000AInterpreter.cpp`
//!   * `pcsx2/R3000AOpcodeTables.cpp`
//!   * `pcsx2/R5900.{h,cpp}`
//!   * `pcsx2/Interpreter.cpp`
//!
//! Only the high level state and interpreter loop/dispatch are translated
//! here.  Memory accessors, COP2 GTE helpers, and the event test machinery
//! that depends on the rest of PCSX2 are left as `unimplemented!()` stubs
//! so that the module compiles in isolation.

// ============================================================================
// Global CPU state
// ============================================================================

/// IOP (R3000A) general-purpose / HI / LO / PC / cycle state.
///
/// In the C++ original these live in a `psxRegisters` struct with separate
/// GPR/CP0/CP2 unions.  The Rust port keeps only the fields that the
/// interpreter and dispatch use; the CP0 and CP2 banks are exposed as
/// `cp0` and `cp2` byte arrays for the same reason the original was an
/// `alignas(16)` struct - 16-byte alignment is needed for the
/// recompiler.
#[derive(Copy, Clone)]
pub struct IopState {
    /// General purpose registers.  Index 0 is hardwired to zero, the rest
    /// follow the standard MIPS R3000A ABI order
    /// (at, v0-v1, a0-a3, t0-t9, s0-s7, t8-t9, k0-k1, gp, sp, s8, ra).
    pub regs: [u32; 32],
    /// Program counter - the address of the next instruction to execute.
    pub pc: u32,
    /// Multiply / divide high result.
    pub hi: u32,
    /// Multiply / divide low result.
    pub lo: u32,
    /// IOP cycle counter.  Incremented per executed instruction.
    pub cycle: u64,
    /// Scratch holding the currently-decoded instruction word.  Matches
    /// `psxRegs.code` in the original.
    pub code: u32,
    /// COP0 register bank (32 x u32).
    pub cp0: [u32; 32],
    /// COP2 data register bank (32 x u32).  The original keeps the GTE
    /// vector/matrix overlays as a union; we only model the flat storage.
    pub cp2_data: [u32; 32],
    /// COP2 control register bank (32 x u32).
    pub cp2_ctrl: [u32; 32],
    /// Currently signaled IOP interrupts bitmap.
    pub interrupt: u32,
    /// Cycle at which each event was first raised.
    pub s_cycle: [u64; 32],
    /// Delta from `s_cycle` at which each event fires.
    pub e_cycle: [i32; 32],
    /// Next event cycle.  When `cycle` reaches this value the
    /// interpreter pauses to dispatch IOP events.
    pub iop_next_event_cycle: u64,
    /// Number of EE cycles the IOP should run for in the current slice.
    pub iop_cycle_ee: i32,
    /// Carry accumulator for the EE/IOP cycle ratio.
    pub iop_cycle_ee_carry: u32,
    /// Break delta returned to the EE when an IOP exception needs EE
    /// attention.
    pub iop_break: i32,
}

impl Default for IopState {
    fn default() -> Self {
        IopState {
            regs: [0u32; 32],
            pc: 0,
            hi: 0,
            lo: 0,
            cycle: 0,
            code: 0,
            cp0: [0u32; 32],
            cp2_data: [0u32; 32],
            cp2_ctrl: [0u32; 32],
            interrupt: 0,
            s_cycle: [0u64; 32],
            e_cycle: [0i32; 32],
            iop_next_event_cycle: 0,
            iop_cycle_ee: 0,
            iop_cycle_ee_carry: 0,
            iop_break: 0,
        }
    }
}

/// EE (R5900) general-purpose / HI / LO / PC / cycle state.
///
/// R5900 GPRs are 128 bits wide.  We model the flat `u128` view; the
/// original union of `u64[2]` / `s64[2]` / `u32[4]` / etc. is provided
/// for callers that need a particular lane.
#[derive(Copy, Clone)]
pub struct EeState {
    /// 128-bit general purpose registers, ABI-ordered (same layout as
    /// the IOP for the first 32 entries).
    pub regs: [u128; 32],
    /// Program counter.
    pub pc: u32,
    /// 128-bit HI register.
    pub hi: u128,
    /// 128-bit LO register.
    pub lo: u128,
    /// EE cycle counter.
    pub cycle: u64,
    /// Scratch holding the currently-decoded instruction word.
    pub code: u32,
    /// COP0 register bank (32 x u32).  The original keeps a status
    /// bitfield overlay; we expose the raw storage.
    pub cp0: [u32; 32],
    /// FPU register bank - 32 single-precision (we keep the raw
    /// `u32` representation, like `fpuRegisters.fpr[]`).
    pub fpr: [u32; 32],
    /// FPU control registers (`fprc[0..32]` in the original).
    pub fprc: [u32; 32],
    /// FPU ACC register.
    pub fpu_acc: u32,
    /// FPU ACC overflow flag.
    pub fpu_acc_flag: u32,
    /// Performance counter control.
    pub perf: [u32; 4],
    /// Interrupt event deltas.
    pub e_cycle: [u32; 32],
    /// Interrupt event start cycles.
    pub s_cycle: [u64; 32],
    /// Currently signaled EE interrupts bitmap.
    pub interrupt: u32,
    /// DMA stall bitmap.
    pub dmastall: u32,
    /// 1 when executing inside a delay slot, 0 otherwise.
    pub branch: i32,
    /// 1 when a branch is pending in the interpreter.
    pub opmode: i32,
    /// Cycle at which the next event test should fire.
    pub next_event_cycle: u64,
    /// Cycle of the last event test.
    pub last_event_cycle: u64,
    /// Cycle of the last COP0 update (used for the count/compare timer).
    pub last_cop0_cycle: u64,
    /// Cycle of the last performance-counter update, per counter.
    pub last_perf_cycle: [u64; 2],
}

impl Default for EeState {
    fn default() -> Self {
        EeState {
            regs: [0u128; 32],
            pc: 0,
            hi: 0,
            lo: 0,
            cycle: 0,
            code: 0,
            cp0: [0u32; 32],
            fpr: [0u32; 32],
            fprc: [0u32; 32],
            fpu_acc: 0,
            fpu_acc_flag: 0,
            perf: [0u32; 4],
            e_cycle: [0u32; 32],
            s_cycle: [0u64; 32],
            interrupt: 0,
            dmastall: 0,
            branch: 0,
            opmode: 0,
            next_event_cycle: 0,
            last_event_cycle: 0,
            last_cop0_cycle: 0,
            last_perf_cycle: [0u64; 2],
        }
    }
}

/// Global IOP register bank.  `alignas(16)` in the original to satisfy the
/// recompiler; in safe Rust we use a `#[repr(C)]` struct so the layout is
/// stable and the per-thread static mirrors the C ABI.
#[repr(C, align(16))]
pub struct IopStateAligned(pub IopState);

/// Global EE register bank.  Mirrors `cpuRegistersPack` in the original
/// (the C struct is just `cpuRegisters` + `fpuRegisters` laid out in two
/// 16-byte-aligned halves).
#[repr(C, align(16))]
pub struct EeStateAligned(pub EeState);

/// Global IOP state.  `static mut` is used to mirror the C global.
pub static mut psxRegs: IopState = IopState {
    regs: [0u32; 32],
    pc: 0,
    hi: 0,
    lo: 0,
    cycle: 0,
    code: 0,
    cp0: [0u32; 32],
    cp2_data: [0u32; 32],
    cp2_ctrl: [0u32; 32],
    interrupt: 0,
    s_cycle: [0u64; 32],
    e_cycle: [0i32; 32],
    iop_next_event_cycle: 0,
    iop_cycle_ee: 0,
    iop_cycle_ee_carry: 0,
    iop_break: 0,
};

/// Global EE state.  `static mut` is used to mirror the C global.
pub static mut cpuRegs: EeState = EeState {
    regs: [0u128; 32],
    pc: 0,
    hi: 0,
    lo: 0,
    cycle: 0,
    code: 0,
    cp0: [0u32; 32],
    fpr: [0u32; 32],
    fprc: [0u32; 32],
    fpu_acc: 0,
    fpu_acc_flag: 0,
    perf: [0u32; 4],
    e_cycle: [0u32; 32],
    s_cycle: [0u64; 32],
    interrupt: 0,
    dmastall: 0,
    branch: 0,
    opmode: 0,
    next_event_cycle: 0,
    last_event_cycle: 0,
    last_cop0_cycle: 0,
    last_perf_cycle: [0u64; 2],
};

/// `nextDeltaCounter` analogue for the EE counters.  Held in a single
/// mutable static so the rest of the emulator can update it from the
/// counter implementation.
pub static mut psxNextDeltaCounter: i32 = 0;

/// `psxNextStartCounter` analogue.
pub static mut psxNextStartCounter: u64 = 0;

/// `iopEventAction` analogue - flag the EE that the IOP needs attention.
pub static mut iopEventAction: bool = false;

/// `eeEventTestIsActive` analogue.
pub static mut eeEventTestIsActive: bool = false;

/// `iopEventTestIsActive` analogue.
pub static mut iopEventTestIsActive: bool = false;

/// `iopIsDelaySlot` analogue.
pub static mut iopIsDelaySlot: bool = false;

/// IOP oscillator frequency.  Mirrors `PSXCLK`.
pub static mut PSXCLK: u32 = 36_864_000;

/// EE oscillator frequency.  Mirrors `PS2CLK`.
pub static mut PS2CLK: u32 = 294_912_000;

// ============================================================================
// IOP memory accessors
// ============================================================================
//
// The original calls out to `iopMemRead8/16/32` and `iopMemWrite8/16/32`,
// which are implemented elsewhere in the emulator.  For this isolated
// translation we provide small stub functions so the dispatch compiles.
// In a full build these would forward to the VTLB memory map.

/// Read an 8-bit value from the IOP memory map.  Stub: returns 0.
pub fn iop_mem_read8(addr: u32) -> u8 {
    let _ = addr;
    0
}

/// Read a 16-bit value from the IOP memory map.  Stub: returns 0.
pub fn iop_mem_read16(addr: u32) -> u16 {
    let _ = addr;
    0
}

/// Read a 32-bit value from the IOP memory map.  Stub: returns 0.
pub fn iop_mem_read32(addr: u32) -> u32 {
    let _ = addr;
    0
}

/// Write an 8-bit value to the IOP memory map.  Stub.
pub fn iop_mem_write8(addr: u32, value: u8) {
    let _ = (addr, value);
}

/// Write a 16-bit value to the IOP memory map.  Stub.
pub fn iop_mem_write16(addr: u32, value: u16) {
    let _ = (addr, value);
}

/// Write a 32-bit value to the IOP memory map.  Stub.
pub fn iop_mem_write32(addr: u32, value: u32) {
    let _ = (addr, value);
}

/// Read a 32-bit IOP hardware register (the original `psxHu32`).
pub fn psx_hu32(addr: u32) -> u32 {
    // In the real emulator this dispatches to the IOP hardware register
    // page at `0x1F80_0000`.  We just zero so the stubs compile.
    let _ = addr;
    0
}

/// Write a 32-bit IOP hardware register (the original `psxHu32` setter).
pub fn psx_hu32_set(addr: u32, value: u32) {
    let _ = (addr, value);
}

// ============================================================================
// EE memory accessors
// ============================================================================

/// Read a 32-bit value from the EE memory map.  Stub: returns 0.
pub fn ee_mem_read32(addr: u32) -> u32 {
    let _ = addr;
    0
}

/// Write a 32-bit value to the EE memory map.  Stub.
pub fn ee_mem_write32(addr: u32, value: u32) {
    let _ = (addr, value);
}

/// Read a 32-bit EE hardware register (the original `psHu32`).
pub fn ps_hu32(addr: u32) -> u32 {
    let _ = addr;
    0
}

/// Read a 16-bit EE hardware register (the original `psHu16`).
pub fn ps_hu16(addr: u32) -> u16 {
    let _ = addr;
    0
}

/// Read an 8-bit EE hardware register (the original `psHu8`).
pub fn ps_hu8(addr: u32) -> u8 {
    let _ = addr;
    0
}

// ============================================================================
// IRX import table helper
// ============================================================================
//
// The original `irxImportExec` / `irxImportTableAddr` are part of
// `IopBios`.  The interpreter calls them from the J opcode handler; in
// the Rust port we just expose inert stubs.

/// Stubbed `irxImportTableAddr`.  Returns the PC unchanged.
pub fn irx_import_table_addr(pc: u32) -> u32 {
    pc
}

/// Stubbed `irxImportExec`.  Returns false to indicate that no import
/// table hack was applied.
pub fn irx_import_exec(_table: u32, _index: u16) -> bool {
    false
}

/// Stubbed `irxImportExec` overload matching the original call site
/// signature.
pub fn irx_import_exec_word(_pc: u32) -> bool {
    false
}

// ============================================================================
// Misc. IOP helpers
// ============================================================================

/// Stubbed IOP hardware reset hook (`psxHwReset`).
pub fn psx_hw_reset() {}

/// Stubbed IOP BIOS reset hook (`psxBiosReset`).
pub fn psx_bios_reset() {}

/// Stubbed IOP BIOS call hook (`psxBiosCall`).
pub fn psx_bios_call() {}

/// Stubbed EE hardware reset hook (`pgifInit`).
pub fn pgif_init() {}

/// Stubbed Deci2 reset hook.
pub fn deci2_reset() {}

/// Stubbed IopCounters reset.
pub fn psx_rcnt_update() {}

/// Stubbed `psxException` for the R3000A.  The original dispatches to
/// the BEV-aware handler, but we leave this as a placeholder so the
/// interpreter compiles in isolation.
pub fn psx_exception(code: u32, bd: u32) {
    let _ = (code, bd);
}

/// Stubbed `cpuException` for the R5900.
pub fn cpu_exception(code: u32, bd: u32) {
    let _ = (code, bd);
}

// ============================================================================
// PSXCLK / PS2CLK configuration bits
// ============================================================================

/// Bits used to drive EE cycle-rate scaling.  Mirror the
/// `CHECK_EEREC` / `CHECK_EETIMINGHACK` / `CHECK_INSTANTDMAHACK`
/// configuration toggles in the original.
pub static mut CHECK_EEREC: bool = false;
pub static mut CHECK_EETIMINGHACK: bool = false;
pub static mut CHECK_INSTANTDMAHACK: bool = false;

/// `EEsCycle` analogue - the EE-side cycle delta the IOP owes the EE.
pub static mut EEsCycle: i32 = 0;

/// `EEoCycle` analogue - the EE cycle at the start of the current slice.
pub static mut EEoCycle: u64 = 0;

// ============================================================================
// Branch helpers
// ============================================================================

/// Place a branch target in the IOP delay slot.  This is a one-shot
/// setter - the next call to `psxExecuteBlock` uses the value.
pub static mut iop_branch_pc: u32 = 0;

/// Whether the IOP is in the delay slot of a branch.
pub static mut iop_branch_pending: bool = false;

/// Place a branch target in the EE delay slot.
pub static mut ee_branch_pc: u32 = 0;
pub static mut ee_branch_pending: bool = false;

// ============================================================================
// Interpreter dispatch glue
// ============================================================================

/// Opcode dispatch table size.  The original is `psxBSC[64]`.
pub const PSX_BSC_TABLE_LEN: usize = 64;
/// Special-function dispatch table size (`psxSPC[64]`).
pub const PSX_SPC_TABLE_LEN: usize = 64;
/// REGIMM dispatch table size (`psxREG[32]`).
pub const PSX_REG_TABLE_LEN: usize = 32;
/// COP0 dispatch table size (`psxCP0[32]`).
pub const PSX_CP0_TABLE_LEN: usize = 32;
/// COP2 dispatch table size (`psxCP2[64]`).
pub const PSX_CP2_TABLE_LEN: usize = 64;
/// COP2 BASIC dispatch table size (`psxCP2BSC[32]`).
pub const PSX_CP2_BSC_TABLE_LEN: usize = 32;

/// Placeholder for `psxBSC[64]` - a function pointer table indexed by
/// the top 6 bits of the 32-bit instruction word.
pub static mut PSX_BSC: [Option<unsafe fn()>; PSX_BSC_TABLE_LEN] =
    [None; PSX_BSC_TABLE_LEN];

/// Placeholder for `psxSPC[64]`.
pub static mut PSX_SPC: [Option<unsafe fn()>; PSX_SPC_TABLE_LEN] =
    [None; PSX_SPC_TABLE_LEN];

/// Placeholder for `psxREG[32]`.
pub static mut PSX_REG: [Option<unsafe fn()>; PSX_REG_TABLE_LEN] =
    [None; PSX_REG_TABLE_LEN];

/// Placeholder for `psxCP0[32]`.
pub static mut PSX_CP0: [Option<unsafe fn()>; PSX_CP0_TABLE_LEN] =
    [None; PSX_CP0_TABLE_LEN];

/// Placeholder for `psxCP2[64]`.
pub static mut PSX_CP2: [Option<unsafe fn()>; PSX_CP2_TABLE_LEN] =
    [None; PSX_CP2_TABLE_LEN];

/// Placeholder for `psxCP2BSC[32]`.
pub static mut PSX_CP2_BSC: [Option<unsafe fn()>; PSX_CP2_BSC_TABLE_LEN] =
    [None; PSX_CP2_BSC_TABLE_LEN];

// ============================================================================
// Instruction decode helpers
// ============================================================================

/// Funct field (low 6 bits).
#[inline(always)]
pub const fn funct(code: u32) -> u32 {
    code & 0x3F
}

/// Rd field (bits 11..16).
#[inline(always)]
pub const fn rd(code: u32) -> u32 {
    (code >> 11) & 0x1F
}

/// Rt field (bits 16..21).
#[inline(always)]
pub const fn rt(code: u32) -> u32 {
    (code >> 16) & 0x1F
}

/// Rs field (bits 21..26).
#[inline(always)]
pub const fn rs(code: u32) -> u32 {
    (code >> 21) & 0x1F
}

/// Sa field (bits 6..11).
#[inline(always)]
pub const fn sa(code: u32) -> u32 {
    (code >> 6) & 0x1F
}

/// Opcode field (bits 26..32).
#[inline(always)]
pub const fn opcode_field(code: u32) -> u32 {
    code >> 26
}

/// Sign-extended 16-bit immediate.
#[inline(always)]
pub const fn imm_s(code: u32) -> i32 {
    (code & 0xFFFF) as i16 as i32
}

/// Zero-extended 16-bit immediate.
#[inline(always)]
pub const fn imm_u(code: u32) -> u32 {
    code & 0xFFFF
}

/// Jump target - low 26 bits shifted left 2, combined with the upper
/// 4 bits of the current PC.
#[inline(always)]
pub const fn jump_target(code: u32, pc: u32) -> u32 {
    ((code & 0x03FF_FFFF) << 2) | (pc & 0xF000_0000)
}

/// Branch target - sign-extended 16-bit offset shifted left 2,
/// added to the current PC.
#[inline(always)]
pub const fn branch_target(code: u32, pc: u32) -> u32 {
    ((imm_s(code) as u32) << 2).wrapping_add(pc)
}

/// Sign bit of the immediate field.  Used by SLTI/SLTIU.
#[inline(always)]
pub const fn imm_sb(code: u32) -> u32 {
    code & 0x8000
}

// ============================================================================
// IOP state init / reset / execute block
// ============================================================================

/// Initialise the IOP.  Mirrors the C `psxCpu` / IOP init.  We do not
/// allocate any state here - the global `psxRegs` is a `static mut`
/// so it lives for the program's lifetime.
pub fn psxInit() {
    // No-op: the global is zero-initialised.
    unsafe {
        psxRegs = IopState {
            regs: [0u32; 32],
            pc: 0xBFC0_0000, // bootstrap vector
            hi: 0,
            lo: 0,
            cycle: 0,
            code: 0,
            cp0: [0u32; 32],
            cp2_data: [0u32; 32],
            cp2_ctrl: [0u32; 32],
            interrupt: 0,
            s_cycle: [0u64; 32],
            e_cycle: [0i32; 32],
            iop_next_event_cycle: 0,
            iop_cycle_ee: 0,
            iop_cycle_ee_carry: 0,
            iop_break: 0,
        };
        // CP0.Status = 0x0040_0000 (BEV=1).
        psxRegs.cp0[12] = 0x0040_0000;
        // CP0.PRid - revision id.
        psxRegs.cp0[15] = 0x0000_001F;
        psxRegs.iop_next_event_cycle = psxRegs.cycle + 4;
        psx_hw_reset();
        PSXCLK = 36_864_000;
        psx_bios_reset();
    }
}

/// Reset the IOP.  Mirrors `psxReset`.
pub fn psxReset() {
    unsafe {
        std::ptr::write_bytes(
            &mut psxRegs as *mut IopState,
            0u8,
            1,
        );
        psxRegs.pc = 0xBFC0_0000;
        psxRegs.cp0[12] = 0x0040_0000; // Status: BEV=1
        psxRegs.cp0[15] = 0x0000_001F; // PRid
        psxRegs.iop_break = 0;
        psxRegs.iop_cycle_ee = -1;
        psxRegs.iop_cycle_ee_carry = 0;
        psxRegs.iop_next_event_cycle = psxRegs.cycle + 4;
        psx_hw_reset();
        PSXCLK = 36_864_000;
        psx_bios_reset();
    }
}

/// Execute a block of IOP instructions, bounded by the number of EE
/// cycles the IOP is allowed to consume.  Mirrors `intExecuteBlock`.
///
/// The original loops over `execI()` until either `iopCycleEE` is
/// exhausted or a branch is taken.  We provide a structural translation
/// that runs the same shape but defers real instruction execution to
/// the dispatch function in this module.
pub fn psxExecuteBlock(cycles: u32) {
    unsafe {
        psxRegs.iop_break = 0;
        psxRegs.iop_cycle_ee = cycles as i32;
        let mut last_iop_cycle: u64 = 0;
        while psxRegs.iop_cycle_ee > 0 {
            last_iop_cycle = psxRegs.cycle;
            // PS1-mode BIOS call detection - same bit pattern as the
            // original.
            if (psx_hu32(0x1F80_4000) & 8) != 0
                && ((psxRegs.pc & 0x1FFF_FFFF) == 0xA0
                    || (psxRegs.pc & 0x1FFF_FFFF) == 0xB0
                    || (psxRegs.pc & 0x1FFF_FFFF) == 0xC0)
            {
                psx_bios_call();
            }
            iop_branch_pending = false;
            while !iop_branch_pending {
                psxExecuteOne();
                if psxRegs.iop_cycle_ee <= 0 {
                    break;
                }
            }
            // EE<->IOP cycle accounting.
            if (psx_hu32(0x1F80_4000) & (1 << 3)) != 0 {
                // PS1 mode: cnum=1280, cdenom=147.
                let t = 1280u32
                    .wrapping_mul(
                        (psxRegs.cycle - last_iop_cycle) as u32,
                    )
                    .wrapping_add(psxRegs.iop_cycle_ee_carry);
                psxRegs.iop_cycle_ee = psxRegs.iop_cycle_ee.wrapping_sub((t / 147) as i32);
                psxRegs.iop_cycle_ee_carry = t % 147;
            } else {
                // PS2 mode: 8 EE cycles per IOP cycle.
                psxRegs.iop_cycle_ee = psxRegs
                    .iop_cycle_ee
                    .wrapping_sub(((psxRegs.cycle - last_iop_cycle) as i32) * 8);
            }
        }
        psxRegs.iop_break + psxRegs.iop_cycle_ee;
    }
}

/// Execute a single IOP instruction.  Mirrors `execI`.
pub fn psxExecuteOne() {
    unsafe {
        // Read instruction at current PC.
        psxRegs.code = iop_mem_read32(psxRegs.pc);
        // Advance past the current instruction.
        psxRegs.pc = psxRegs.pc.wrapping_add(4);
        psxRegs.cycle = psxRegs.cycle.wrapping_add(1);
        // Dispatch to the top-level opcode handler.
        psxInterpreter(psxRegs.code);
    }
}

/// Dispatch a single decoded IOP instruction.  Mirrors the
/// `psxBSC[code >> 26]()` call in the original `execI`.
///
/// This is the structural translation of the dispatch - actual
/// per-opcode implementations live in `R3000AOpcodeTables.cpp` and
/// are not duplicated here.
pub fn psxInterpreter(instr: u32) {
    let op = opcode_field(instr) as usize;
    unsafe {
        // SAFETY: PSX_BSC is the same size as the C `psxBSC[64]`.
        // We dispatch via a function pointer table identical to the
        // original.  Slots that are `None` in this minimal port are
        // silently dropped (the original `psxNULL` is also a no-op
        // aside from logging the unimplemented opcode).
        if let Some(handler) = PSX_BSC.get(op).and_then(|h| *h) {
            handler();
        }
        // The original also has SPECIAL / REGIMM / COP0 / COP2 / BASIC
        // dispatch chains.  These cascade from inside their respective
        // top-level handlers, so the table-of-tables is implicit in
        // those handlers' bodies.
    }
}

// ============================================================================
// EE state init / reset / execute block
// ============================================================================

/// Initialise the EE and the IOP.  Mirrors `cpuReset`.
pub fn eeInit() {
    unsafe {
        std::ptr::write_bytes(&mut cpuRegs as *mut EeState, 0u8, 1);
        cpuRegs.pc = 0xBFC0_0000;
        cpuRegs.cp0[16] = 0x440; // Config
        cpuRegs.cp0[12] = 0x7040_0004; // Status (BEV=1, TS=1, EIE=1, IE=0)
        cpuRegs.cp0[15] = 0x0000_2E20; // PRid
        cpuRegs.fprc[0] = 0x0000_2E30; // FCR0 (revision)
        cpuRegs.fprc[31] = 0x0100_0001; // FCR31 (control/status)
        cpuRegs.next_event_cycle = cpuRegs.cycle + 4;
        EEsCycle = 0;
        EEoCycle = cpuRegs.cycle;
        psxReset();
        pgif_init();
        deci2_reset();
    }
}

/// Reset the EE.  Mirrors `cpuReset`.
pub fn eeReset() {
    unsafe {
        std::ptr::write_bytes(&mut cpuRegs as *mut EeState, 0u8, 1);
        cpuRegs.pc = 0xBFC0_0000;
        cpuRegs.cp0[16] = 0x440;
        cpuRegs.cp0[12] = 0x7040_0004;
        cpuRegs.cp0[15] = 0x0000_2E20;
        cpuRegs.fprc[0] = 0x0000_2E30;
        cpuRegs.fprc[31] = 0x0100_0001;
        cpuRegs.next_event_cycle = cpuRegs.cycle + 4;
        EEsCycle = 0;
        EEoCycle = cpuRegs.cycle;
        psxReset();
        pgif_init();
        deci2_reset();
    }
}

/// Execute a block of EE instructions bounded by the given number of
/// cycles.  Mirrors `intExecute`/`intExecuteBlock`.
pub fn eeExecuteBlock(cycles: u32) {
    let _ = cycles;
    unsafe {
        // The original interpreter loops `execI` until an exception or
        // event test fires.  In the isolated port we just call the
        // single-step helper - a full build wires this to the same
        // dispatch chain as the IOP.
        eeExecuteOne();
    }
}

/// Execute a single EE instruction.  Mirrors `execI` from `Interpreter.cpp`.
pub fn eeExecuteOne() {
    unsafe {
        // PC is incremented before the memory read so that exceptions
        // see the correct EPC.
        let pc = cpuRegs.pc;
        cpuRegs.pc = cpuRegs.pc.wrapping_add(4);
        cpuRegs.code = ee_mem_read32(pc);
        // Dispatch to the EE opcode interpreter.  In the original this
        // goes through `OPCODE::interpret` from `R5900OpcodeTables.h`.
        // The Rust port leaves the dispatch table as a future addition.
        cpuRegs.cycle = cpuRegs.cycle.wrapping_add(1);
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iop_state_default_is_zero() {
        let s = IopState::default();
        assert_eq!(s.regs, [0u32; 32]);
        assert_eq!(s.hi, 0);
        assert_eq!(s.lo, 0);
        assert_eq!(s.pc, 0);
        assert_eq!(s.cycle, 0);
    }

    #[test]
    fn ee_state_default_is_zero() {
        let s = EeState::default();
        assert_eq!(s.regs, [0u128; 32]);
        assert_eq!(s.hi, 0);
        assert_eq!(s.lo, 0);
        assert_eq!(s.pc, 0);
        assert_eq!(s.cycle, 0);
    }

    #[test]
    fn decode_helpers_match_c_macros() {
        let code: u32 = 0x0232_8021; // ADDU $s1, $s1, $s2
        assert_eq!(opcode_field(code), 0);
        assert_eq!(rs(code), 1);
        assert_eq!(rt(code), 2);
        assert_eq!(rd(code), 3);
        assert_eq!(sa(code), 0);
        assert_eq!(funct(code), 0x21);
        assert_eq!(imm_s(code), 0);
        assert_eq!(imm_u(code), 0);
    }

    #[test]
    fn branch_target_sign_extends() {
        let pc: u32 = 0x0040_0000;
        // BEQ $0, $0, +0x100 (offset 0x40 words).
        let code: u32 = 0x1000_0040;
        assert_eq!(branch_target(code, pc), pc.wrapping_add(0x100));
    }

    #[test]
    fn branch_target_negative() {
        let pc: u32 = 0x0040_0100;
        // BNE with offset -4 (0xFFFC).
        let code: u32 = 0x1400_FFFC;
        assert_eq!(branch_target(code, pc), pc.wrapping_sub(0x10));
    }

    #[test]
    fn jump_target_combines_high_pc_bits() {
        // J 0x1FC_0000 (instruction is J with the low 26 bits of the
        // target / 4).
        let pc: u32 = 0x8000_0000;
        let code: u32 = 0x0800_0000; // J to 0x0000_0000
        assert_eq!(jump_target(code, pc), 0x0000_0000);
    }

    #[test]
    fn psx_init_sets_bootstrap_pc() {
        // SAFETY: test serial - no other threads touch psxRegs.
        unsafe {
            psxInit();
            assert_eq!(psxRegs.pc, 0xBFC0_0000);
            assert_eq!(psxRegs.cp0[12], 0x0040_0000);
        }
    }
}
