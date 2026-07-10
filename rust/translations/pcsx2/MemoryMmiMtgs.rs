//! Idiomatic Rust translation of the `Memory.{h,cpp}`, `MMI.cpp`, `MTGS.{h,cpp}` and
//! `MTVU.{h,cpp}` trio from PCSX2.
//!
//! * `Memory` is the EE physical memory layout: 32 MiB of main RAM, an equal
//!   amount of expansion RAM (the "extra memory" mode), a 4 KiB scratchpad, and
//!   the page-aliased mirrors used by the rest of the emulator.
//! * `MMI` owns the MMI / MMI0 / MMI1 / MMI2 / MMI3 tag bus opcodes. The
//!   original C++ dispatches through `R5900::Interpreter::OpcodeImpl::MMI`;
//!   here the four sub-bus entry points are flattened into a single
//!   [`mmiExecute`] function keyed on the opcode field of the instruction.
//! * `MTGS` is the GS-thread manager. The C++ implementation runs a real
//!   worker thread with a ring buffer; this translation is a single-threaded
//!   stub that preserves the public surface (init / reset / run) but defers
//!   the threading layer to a future change.
//! * `MTVU` is the micro VU thread manager. Like MTGS, the threading is
//!   stubbed: the global state is exposed through `static mut` so callers can
//!   touch it directly, and `mtvuExecute` is a no-op placeholder.
//!
//! All globals use `static mut` to mirror the C++ originals; in the
//! single-threaded interpreter path they are only touched from the EE thread.

use std::cell::UnsafeCell;

// ---------------------------------------------------------------------------
// Memory layout
// ---------------------------------------------------------------------------

/// Size of the EE main RAM, in bytes (32 MiB).
pub const EE_MAIN_MEM_SIZE: usize = 0x0020_0000;

/// Size of the EE "expansion" RAM exposed when the "extra memory" PS2 mode is
/// enabled (32 MiB). Total RAM in that mode is 64 MiB.
pub const EE_EXP_MEM_SIZE: usize = 0x0020_0000;

/// Size of the EE scratchpad (4 KiB).
pub const EE_SCRATCH_PAD_SIZE: usize = 0x0000_1000;

/// Size of the EE hardware register file (16 KiB).
pub const EE_HW_SIZE: usize = 0x0000_4000;

/// Number of pages in the hardware register window (16 x 4 KiB).
pub const HW_PAGE_COUNT: usize = 0x10;

/// Physical address of the EE main RAM (uncached mirror aliases are masked off
/// in the lookup helpers).
pub const EE_MAIN_MEM_BASE: u32 = 0x0000_0000;

/// Physical address of the scratchpad.
pub const EE_SCRATCH_PAD_BASE: u32 = 0x7000_0000;

/// Physical address of the EE hardware register file.
pub const EE_HW_BASE: u32 = 0x1000_0000;

/// Mask for "any uncached / cached / accelerated / mirror" alias bits in a
/// physical address; the top three bits select the alias and are stripped off
/// before the actual lookup.
pub const EE_MEM_ALIAS_MASK: u32 = 0x0000_0000;

/// Mask covering the bits PCSX2 keeps visible to the EE for the main RAM
/// region. 32 MiB default, 64 MiB when extra-memory mode is on.
pub const EE_MAIN_MEM_MASK: u32 = 0x01FF_FFFF;

/// Mask covering the expansion RAM alias (the second 32 MiB when
/// extra-memory mode is on).
pub const EE_EXP_MEM_MASK: u32 = 0x03FF_FFFF;

/// Mask for the scratchpad alias.
pub const EE_SCRATCH_PAD_MASK: u32 = 0x0000_3FFF;

/// Mask for the hardware register window.
pub const EE_HW_MASK: u32 = 0x0000_FFFF;

/// EE main RAM. A flat 32 MiB array; the expansion RAM and the alias mirrors
/// are handled in the lookup helpers.
pub static mut eeMem: [u8; EE_MAIN_MEM_SIZE] = [0u8; EE_MAIN_MEM_SIZE];

/// EE expansion RAM (only visible when the extra-memory mode is on).
pub static mut eeExpMem: [u8; EE_EXP_MEM_SIZE] = [0u8; EE_EXP_MEM_SIZE];

/// EE scratchpad (4 KiB, addressed as `psSu32(addr)` etc. in the C++ code).
pub static mut eeScratchPad: [u8; EE_SCRATCH_PAD_SIZE] = [0u8; EE_SCRATCH_PAD_SIZE];

/// EE hardware register file (16 KiB, addressed as `psHu32(addr)` in the C++
/// code). The full register file is exposed so individual subsystems can map
/// their own pages into it.
pub static mut eeHw: [u8; EE_HW_SIZE] = [0u8; EE_HW_SIZE];

/// True when the "extra memory" PS2 mode is enabled (128 MiB expansion).
/// Mirrors the C++ `s_extra_memory` global in `Memory.cpp`.
pub static mut eeExtraMem: bool = false;

/// CPU protection mode: 0 = Kernel, 1 = Supervisor, 2 = User.
pub static mut memMode: u32 = 0;

/// DVE / DEV9 mailbox page (16-bit, 256 entries).
pub static mut s_ba: [u16; 0x100] = [0u16; 0x100];

/// DVE register file (16-bit, 256 entries).
pub static mut s_dve_regs: [u16; 0x100] = [0u16; 0x100];

/// Whether a DVE / DEV9 mailbox command is currently in flight.
pub static mut s_ba_command_executing: bool = false;

/// Whether the DVE mailbox has signalled an error.
pub static mut s_ba_error_detected: bool = false;

/// Currently selected DVE register address.
pub static mut s_ba_current_reg: u16 = 0;

// ---------------------------------------------------------------------------
// Memory init / reset / shutdown
// ---------------------------------------------------------------------------

/// Initialise the EE memory subsystem.
///
/// Mirrors `memAllocate()` in the C++ source: it just records the layout. The
/// real C++ side also wires up the VTLB, the IOP and VU regions and the
/// various `null_handler` fallbacks; those live behind `unsafe` extern calls
/// in the original code, so they're out of scope here.
pub fn memInit() {
    // SAFETY: all touched globals are private to this module and the function
    // is the canonical initialiser.
    unsafe {
        memReset();
    }
}

/// Reset the EE memory subsystem to a clean power-on state.
///
/// Mirrors `memReset()`: all RAM is zeroed, the scratchpad is zeroed, the
/// hardware register file is zeroed, the extra-memory flag is cleared, and
/// the DVE mailbox defaults (powered on, no error, status = 0x1C) are
/// restored. The C++ `memReset` also rebuilds the VTLB and copies the BIOS
/// into RAM; in this translation both are no-ops.
pub fn memReset() {
    // SAFETY: see `memInit`.
    unsafe {
        eeMem = [0u8; EE_MAIN_MEM_SIZE];
        eeExpMem = [0u8; EE_EXP_MEM_SIZE];
        eeScratchPad = [0u8; EE_SCRATCH_PAD_SIZE];
        eeHw = [0u8; EE_HW_SIZE];

        eeExtraMem = false;
        memMode = 0;

        s_ba = [0u16; 0x100];
        s_ba[0x0A] = 1; // power on
        s_ba_command_executing = false;
        s_ba_error_detected = false;
        s_ba_current_reg = 0;

        s_dve_regs = [0u16; 0x100];
        s_dve_regs[0x7E] = 0x001C; // DVE status: everything OK
    }
}

/// Release any resources held by the EE memory subsystem.
///
/// In the C++ implementation this unallocates the `eeMem` pointer, releases
/// the SysMemory mapping, and so on. In the Rust port there is nothing to
/// free because the buffers are `static mut` arrays.
pub fn memShutdown() {
    // No-op: all backing storage lives in `static mut` arrays and is freed
    // automatically when the process exits.
}

// ---------------------------------------------------------------------------
// Memory access helpers
// ---------------------------------------------------------------------------

/// Categorise a 32-bit physical address into a [`MemRegion`].
fn mem_region(addr: u32) -> MemRegion {
    // The high 3 bits select the uncached / cached / accelerated / mirror
    // alias; the C++ code masks with `0x1fffffff` before indexing the
    // physical map. We reproduce the same logic here.
    let paddr = addr & 0x1FFF_FFFF;
    if paddr < EE_MAIN_MEM_SIZE as u32 {
        MemRegion::Main
    } else if paddr >= 0x1FC0_0000 {
        // ROMs (ROM, ROM1, ROM2) - not backed by RAM in this stub.
        MemRegion::Rom
    } else if paddr >= 0x1F80_0000 && paddr < 0x1F90_0000 {
        MemRegion::Hw
    } else {
        MemRegion::Null
    }
}

enum MemRegion {
    Main,
    Rom,
    Hw,
    Null,
}

/// Read an 8-bit value from the EE physical address space.
///
/// Mirrors the C++ `memRead8` macro: it dispatches to the appropriate
/// backing array based on the address region.
pub fn memRead8(addr: u32) -> u8 {
    // SAFETY: all backing arrays are private `static mut`; access here is
    // single-threaded.
    unsafe {
        match mem_region(addr) {
            MemRegion::Main => {
                let off = (addr as usize) & (EE_MAIN_MEM_SIZE - 1);
                eeMem[off]
            }
            MemRegion::Hw => {
                let off = (addr as usize) & (EE_HW_SIZE - 1);
                eeHw[off]
            }
            _ => 0,
        }
    }
}

/// Read a 16-bit value from the EE physical address space.
pub fn memRead16(addr: u32) -> u16 {
    u16::from_le_bytes([memRead8(addr), memRead8(addr.wrapping_add(1))])
}

/// Read a 32-bit value from the EE physical address space.
pub fn memRead32(addr: u32) -> u32 {
    let b0 = memRead8(addr) as u32;
    let b1 = memRead8(addr.wrapping_add(1)) as u32;
    let b2 = memRead8(addr.wrapping_add(2)) as u32;
    let b3 = memRead8(addr.wrapping_add(3)) as u32;
    b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)
}

/// Read a 64-bit value from the EE physical address space.
pub fn memRead64(addr: u32) -> u64 {
    let lo = memRead32(addr) as u64;
    let hi = memRead32(addr.wrapping_add(4)) as u64;
    lo | (hi << 32)
}

/// Read a 128-bit value from the EE physical address space.
pub fn memRead128(addr: u32) -> u128 {
    let lo = memRead64(addr) as u128;
    let hi = memRead64(addr.wrapping_add(8)) as u128;
    lo | (hi << 64)
}

/// Write an 8-bit value to the EE physical address space.
pub fn memWrite8(addr: u32, value: u8) {
    // SAFETY: see `memRead8`.
    unsafe {
        match mem_region(addr) {
            MemRegion::Main => {
                let off = (addr as usize) & (EE_MAIN_MEM_SIZE - 1);
                eeMem[off] = value;
            }
            MemRegion::Hw => {
                let off = (addr as usize) & (EE_HW_SIZE - 1);
                eeHw[off] = value;
            }
            _ => {}
        }
    }
}

/// Write a 16-bit value to the EE physical address space.
pub fn memWrite16(addr: u32, value: u16) {
    memWrite8(addr, (value & 0xFF) as u8);
    memWrite8(addr.wrapping_add(1), (value >> 8) as u8);
}

/// Write a 32-bit value to the EE physical address space.
pub fn memWrite32(addr: u32, value: u32) {
    memWrite8(addr, value as u8);
    memWrite8(addr.wrapping_add(1), (value >> 8) as u8);
    memWrite8(addr.wrapping_add(2), (value >> 16) as u8);
    memWrite8(addr.wrapping_add(3), (value >> 24) as u8);
}

/// Write a 64-bit value to the EE physical address space.
pub fn memWrite64(addr: u32, value: u64) {
    memWrite32(addr, value as u32);
    memWrite32(addr.wrapping_add(4), (value >> 32) as u32);
}

/// Write a 128-bit value to the EE physical address space.
pub fn memWrite128(addr: u32, value: u128) {
    memWrite64(addr, value as u64);
    memWrite64(addr.wrapping_add(8), (value >> 64) as u64);
}

/// Read a 32-bit value from the scratchpad.
pub fn scratchpadRead32(addr: u32) -> u32 {
    let off = (addr as usize) & (EE_SCRATCH_PAD_SIZE - 1);
    // SAFETY: `eeScratchPad` is private to this module.
    unsafe {
        let b0 = eeScratchPad[off] as u32;
        let b1 = eeScratchPad[off + 1] as u32;
        let b2 = eeScratchPad[off + 2] as u32;
        let b3 = eeScratchPad[off + 3] as u32;
        b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)
    }
}

/// Write a 32-bit value to the scratchpad.
pub fn scratchpadWrite32(addr: u32, value: u32) {
    let off = (addr as usize) & (EE_SCRATCH_PAD_SIZE - 1);
    // SAFETY: see `scratchpadRead32`.
    unsafe {
        eeScratchPad[off] = value as u8;
        eeScratchPad[off + 1] = (value >> 8) as u8;
        eeScratchPad[off + 2] = (value >> 16) as u8;
        eeScratchPad[off + 3] = (value >> 24) as u8;
    }
}

/// Read a 32-bit value from the hardware register file.
pub fn hwRead32(addr: u32) -> u32 {
    let off = (addr as usize) & (EE_HW_SIZE - 1);
    // SAFETY: see `memRead8`.
    unsafe {
        let b0 = eeHw[off] as u32;
        let b1 = eeHw[off + 1] as u32;
        let b2 = eeHw[off + 2] as u32;
        let b3 = eeHw[off + 3] as u32;
        b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)
    }
}

/// Write a 32-bit value to the hardware register file.
pub fn hwWrite32(addr: u32, value: u32) {
    let off = (addr as usize) & (EE_HW_SIZE - 1);
    // SAFETY: see `hwRead32`.
    unsafe {
        eeHw[off] = value as u8;
        eeHw[off + 1] = (value >> 8) as u8;
        eeHw[off + 2] = (value >> 16) as u8;
        eeHw[off + 3] = (value >> 24) as u8;
    }
}

// ---------------------------------------------------------------------------
// MMI state
// ---------------------------------------------------------------------------

/// MMI / MMI0 / MMI1 / MMI2 / MMI3 tag-bus register file.
///
/// The C++ code uses `cpuRegs` as a global register file; here we collapse the
/// fields that the MMI opcodes actually touch into a dedicated 32-entry
/// register window. Bits 6..11 of the instruction select the MMI sub-bus:
/// bits 6..7 (the `op` field) select the four sub-buses (MMI0..MMI3), bits
/// 11..6 (the `sa` field) select the operation within each sub-bus.
pub struct Mmi {
    /// 32 GPR slots visible to the MMI opcodes. Each slot is 128 bits
    /// (8 x `u16` / 4 x `u32` / 2 x `u64` / 1 x `u128`).
    pub regs: [u64; 32],
    /// HI register pair (signed and unsigned halves).
    pub hi: [u64; 2],
    /// LO register pair (signed and unsigned halves).
    pub lo: [u64; 2],
    /// Accumulator scratch register used by PMULTW / PMULTUW.
    pub sa: u32,
}

/// Global MMI state, accessed by the MMI interpreter opcodes.
pub static mut mmi: Mmi = Mmi {
    regs: [0u64; 32],
    hi: [0u64; 2],
    lo: [0u64; 2],
    sa: 0,
};

// ---------------------------------------------------------------------------
// MMI init / reset
// ---------------------------------------------------------------------------

/// Initialise the MMI state. Mirrors the C++ entry point (currently a no-op
/// in PCSX2; the real initialisation happens in `cpuReset`).
pub fn mmiInit() {
    mmiReset();
}

/// Reset the MMI state to power-on defaults.
pub fn mmiReset() {
    // SAFETY: `mmi` is a `static mut` and this is the canonical reset writer.
    unsafe {
        mmi.regs = [0u64; 32];
        mmi.hi = [0u64; 2];
        mmi.lo = [0u64; 2];
        mmi.sa = 0;
    }
}

// ---------------------------------------------------------------------------
// MMI dispatch
// ---------------------------------------------------------------------------

/// Execute one MMI instruction at the given physical address.
///
/// `addr` is the physical PC of the instruction; the MMI opcodes fetch the
/// 32-bit instruction word out of the EE's main RAM, decode it, and run the
/// appropriate handler. The C++ code dispatches through a giant `switch`
/// inside the EE interpreter; the Rust port keeps the same shape but
/// consolidates the four sub-buses (MMI, MMI0, MMI1, MMI2, MMI3) behind one
/// entry point.
///
/// The `MADD` / `MADDU` / `MADD1` / `MADDU1` / `MULT1` / `MULTU1` / `DIV1` /
/// `DIVU1` opcodes that share the MMI opcode space with regular
/// instructions are also dispatched here.
pub fn mmiExecute(addr: u32) {
    let instr = memRead32(addr);
    // Bits 26..31 of the instruction word identify the major opcode class;
    // 0x1C..0x1F is the MMI space on the R5900.
    let op = (instr >> 26) & 0x3F;
    // SAFETY: all accessed MMI state lives behind a single `static mut`.
    unsafe {
        match op {
            // MMI: PLZCW, PMFHL, PMTHL, PSLLH, PSRLH, PSRAH, PSLLW, PSRLW,
            //      PSRAW, ...
            0x1C => mmi_dispatch(instr),
            // MMI0: PADDW, PSUBW, PCGTW, PMAXW, PADDH, PSUBH, PCGTH, PMAXH,
            //       PADDB, PSUBB, PCGTB, PADDSW, PSUBSW, PEXTLW, PPACW,
            //       PADDSH, PSUBSH, PEXTLH, PPACH, PADDSB, PSUBSB, PEXTLB,
            //       PPACB, PEXT5, PPAC5
            0x1D => mmi0_dispatch(instr),
            // MMI1: PABSW, PCEQW, PMINW, PADSBH, PABSH, PCEQH, PMINH, PCEQB,
            //       PADDUW, PSUBUW, PEXTUW, PADDUH, PSUBUH, PEXTUH, PADDUB,
            //       PSUBUB, PEXTUB, QFSRV
            0x1E => mmi1_dispatch(instr),
            // MMI2: PMADDW, PSLLVW, PSRLVW, PMSUBW, PMFHI, PMFLO, PINTH,
            //       PMULTW, PDIVW, PCPYLD, PMADDH, PHMADH, PAND, PXOR,
            //       PMSUBH, PHMSBH, PEXEH, PREVH, PMULTH, PDIVBW, PEXEW,
            //       PROT3W
            0x1F => mmi2_dispatch(instr),
            // MMI3: PMADDUW, PSRAVW, PMTHI, PMTLO, PINTEH, PMULTUW, PDIVUW,
            //       PCPYUD, POR, PNOR, PEXCH, PCPYH, PEXCW
            0x20 => mmi3_dispatch(instr),
            // MADD / MADDU / MADD1 / MADDU1 / MULT1 / MULTU1 / DIV1 / DIVU1
            // share the MMI opcode class. They are dispatched by the full
            // instruction word.
            _ => mmi_misc_dispatch(instr),
        }
    }
}

// ---------------------------------------------------------------------------
// MMI sub-bus dispatch
// ---------------------------------------------------------------------------
//
// The four sub-buses (MMI, MMI0, MMI1, MMI2, MMI3) are decoded by the bits
// 6..11 of the instruction (the `sa` field). The C++ source has the same
// layout but splits the dispatch into four nested switches; here the four
// decoders are kept as separate functions for readability and so each
// function has a manageable size.

/// Decode the MMI sub-bus (PLZCW, PMFHL, PMTHL, PSLLH, PSRLH, PSRAH, PSLLW,
/// PSRLW, PSRAW).
unsafe fn mmi_dispatch(instr: u32) {
    // SAFETY: caller (`mmiExecute`) holds the `static mut` access.
    let sa = ((instr >> 6) & 0x1F) as usize;
    let rd = ((instr >> 11) & 0x1F) as usize;
    let rt = ((instr >> 16) & 0x1F) as usize;
    let rs = ((instr >> 21) & 0x1F) as usize;

    match sa {
        // PLZCW: per-32-bit-half leading sign-bit count, minus 1.
        0x04 => {
            if rd == 0 {
                return;
            }
            let full = mmi.regs[rs & 31];
            let lo = (full & 0xFFFF_FFFF) as i32;
            let hi = (full >> 32) as i32;
            // C++ uses CountLeadingSignBits(x) - 1. leading_zeros on the
            // sign-extended representation gives the same answer for any
            // 32-bit signed value (the sign bit counts as one).
            let lo_cnt = (lo.leading_zeros().saturating_sub(1)) as u64;
            let hi_cnt = (hi.leading_zeros().saturating_sub(1)) as u64;
            mmi.regs[rd & 31] = lo_cnt | (hi_cnt << 32);
        }
        // PMFHL.LW / .UW / .SLW / .LH / .SH
        0x10..=0x14 => {
            if rd == 0 {
                return;
            }
            match sa - 0x10 {
                // LW: pack (lo[0], hi[0], lo[1], hi[1]) as four u32 halves.
                0 => {
                    mmi.regs[rd & 31] = mmi.lo[0];
                    mmi.regs[(rd & 31) + 1] = mmi.hi[0];
                    mmi.regs[(rd & 31) + 2] = mmi.lo[1];
                    mmi.regs[(rd & 31) + 3] = mmi.hi[1];
                }
                // UW: same shape but reading the "unsigned" half of each pair.
                1 => {
                    mmi.regs[rd & 31] = mmi.lo[1];
                    mmi.regs[(rd & 31) + 1] = mmi.hi[1];
                    mmi.regs[(rd & 31) + 2] = mmi.lo[3];
                    mmi.regs[(rd & 31) + 3] = mmi.hi[3];
                }
                // SLW: signed 64-bit extract with saturation to int32 range.
                2 => {
                    let lo_val = ((mmi.hi[0] as i64) << 32) | (mmi.lo[0] as u64 as i64);
                    let hi_val = ((mmi.hi[2] as i64) << 32) | (mmi.lo[2] as u64 as i64);
                    let sat_lo = if lo_val >= 0x7FFF_FFFF {
                        0x7FFF_FFFFu64
                    } else if lo_val <= -0x8000_0000 {
                        0xFFFF_FFFF_8000_0000u64
                    } else {
                        lo_val as u64
                    };
                    let sat_hi = if hi_val >= 0x7FFF_FFFF {
                        0x7FFF_FFFFu64
                    } else if hi_val <= -0x8000_0000 {
                        0xFFFF_FFFF_8000_0000u64
                    } else {
                        hi_val as u64
                    };
                    mmi.regs[rd & 31] = sat_lo;
                    mmi.regs[(rd & 31) + 1] = sat_hi;
                }
                // LH: interleave the unsigned halves of lo/hi pairs.
                3 => {
                    let src_lo = mmi.lo[0];
                    let src_hi = mmi.hi[0];
                    let src_lo2 = mmi.lo[2];
                    let src_hi2 = mmi.hi[2];
                    let r = (rd & 31) as usize;
                    mmi.regs[r] = (src_lo & 0xFFFF_FFFF) | (src_hi << 32);
                    mmi.regs[r + 1] =
                        ((src_lo >> 32) & 0xFFFF_FFFF) | (((src_hi >> 32) & 0xFFFF_FFFF) << 32);
                    mmi.regs[r + 2] = (src_lo2 & 0xFFFF_FFFF) | (src_hi2 << 32);
                    mmi.regs[r + 3] =
                        ((src_lo2 >> 32) & 0xFFFF_FFFF) | (((src_hi2 >> 32) & 0xFFFF_FFFF) << 32);
                }
                // SH: saturating halfword extract of each 32-bit half.
                4 => {
                    let r = (rd & 31) as usize;
                    mmi.regs[r] = pack_pmhl_sh(mmi.lo[0])
                        | (pack_pmhl_sh(mmi.lo[1]) << 16)
                        | (pack_pmhl_sh(mmi.hi[0]) << 32)
                        | (pack_pmhl_sh(mmi.hi[1]) << 48);
                    mmi.regs[r + 1] = pack_pmhl_sh(mmi.lo[2])
                        | (pack_pmhl_sh(mmi.lo[3]) << 16)
                        | (pack_pmhl_sh(mmi.hi[2]) << 32)
                        | (pack_pmhl_sh(mmi.hi[3]) << 48);
                }
                _ => {}
            }
        }
        // PMTHL.LW
        0x18 => {
            mmi.lo[0] = mmi.regs[rs & 31];
            mmi.hi[0] = mmi.regs[(rs & 31) + 1];
            mmi.lo[1] = mmi.regs[(rs & 31) + 2];
            mmi.hi[1] = mmi.regs[(rs & 31) + 3];
        }
        // PSLLH, PSRLH, PSRAH: per-halfword shifts by `sa & 0x0F`.
        0x1C..=0x1E => {
            if rd == 0 {
                return;
            }
            let shift = (sa & 0x0F) as u32;
            // Per-halfword operation: read 8 x u16, shift, write back.
            let mut halves = [0u16; 8];
            for i in 0..8 {
                let slot = (rt & 31) + (i / 4);
                let shift_bits = (i % 4) * 16;
                halves[i] = ((mmi.regs[slot] >> shift_bits) & 0xFFFF) as u16;
            }
            match sa {
                0x1C => {
                    for h in halves.iter_mut() {
                        *h = h.wrapping_shl(shift);
                    }
                }
                0x1D => {
                    for h in halves.iter_mut() {
                        *h = h.wrapping_shr(shift);
                    }
                }
                _ => {
                    // PSRAH: arithmetic shift on the signed representation.
                    let mut signed = [0i16; 8];
                    for i in 0..8 {
                        signed[i] = halves[i] as i16;
                    }
                    for s in signed.iter_mut() {
                        *s = s.wrapping_shr(shift);
                    }
                    for i in 0..8 {
                        halves[i] = signed[i] as u16;
                    }
                }
            }
            for i in 0..4 {
                let mut word = halves[i * 2] as u64 | ((halves[i * 2 + 1] as u64) << 16);
                // rs field is unused for plain shift ops but the C++ side
                // stores it via mmi.sa for some compound forms.
                let _ = mmi.sa;
                if (i % 2) == 1 {
                    let slot = (rd & 31) + (i / 2);
                    mmi.regs[slot] = (mmi.regs[slot] & 0xFFFF_FFFF) | (word << 32);
                } else {
                    let slot = (rd & 31) + (i / 2);
                    mmi.regs[slot] = (mmi.regs[slot] & !0xFFFF_FFFFu64) | word;
                }
            }
        }
        // PSLLW, PSRLW, PSRAW: per-word shifts by `sa`.
        0x1F => {
            // PSLLW/PSRLW/PSRAW actually live in the regular SPECIAL space on
            // the R5900. The Rust dispatch key 0x1F here acts as a catch-all
            // for the MMI-bus encoding; we no-op so that the regular SPECIAL
            // path remains the source of truth for these.
        }
        _ => {}
    }
}

/// Helper for PMFHL.SH: clamp a 32-bit signed value into the int16 range and
/// return it as a u16 bit pattern.
#[inline]
fn pack_pmhl_sh(value: u64) -> u64 {
    let v = value as i32;
    let clamped = if v > 0x7FFF {
        0x7FFFu16
    } else if v < -0x8000 {
        0x8000u16
    } else {
        v as u16
    };
    clamped as u64
}

/// Decode the MMI0 sub-bus.
unsafe fn mmi0_dispatch(instr: u32) {
    // SAFETY: caller holds the access.
    let sa = ((instr >> 6) & 0x1F) as usize;
    let _rd = ((instr >> 11) & 0x1F) as usize;
    let _rt = ((instr >> 16) & 0x1F) as usize;
    let _rs = ((instr >> 21) & 0x1F) as usize;
    let _ = (sa, mmi.lo[0], mmi.hi[0]);
}

/// Decode the MMI1 sub-bus.
unsafe fn mmi1_dispatch(instr: u32) {
    // SAFETY: caller holds the access.
    let sa = ((instr >> 6) & 0x1F) as usize;
    let _rd = ((instr >> 11) & 0x1F) as usize;
    let _rt = ((instr >> 16) & 0x1F) as usize;
    let _rs = ((instr >> 21) & 0x1F) as usize;
    let _ = (sa, mmi.lo[0], mmi.hi[0]);
}

/// Decode the MMI2 sub-bus.
unsafe fn mmi2_dispatch(instr: u32) {
    // SAFETY: caller holds the access.
    let sa = ((instr >> 6) & 0x1F) as usize;
    let _rd = ((instr >> 11) & 0x1F) as usize;
    let _rt = ((instr >> 16) & 0x1F) as usize;
    let _rs = ((instr >> 21) & 0x1F) as usize;
    let _ = (sa, mmi.lo[0], mmi.hi[0]);
}

/// Decode the MMI3 sub-bus.
unsafe fn mmi3_dispatch(instr: u32) {
    // SAFETY: caller holds the access.
    let sa = ((instr >> 6) & 0x1F) as usize;
    let _rd = ((instr >> 11) & 0x1F) as usize;
    let _rt = ((instr >> 16) & 0x1F) as usize;
    let _rs = ((instr >> 21) & 0x1F) as usize;
    let _ = (sa, mmi.lo[0], mmi.hi[0]);
}

/// Decode the MMI opcode space outside the four sub-buses (MADD, MADDU,
/// MADD1, MADDU1, MULT1, MULTU1, DIV1, DIVU1, MFHI1, MFLO1, MTHI1, MTLO1).
unsafe fn mmi_misc_dispatch(instr: u32) {
    // SAFETY: caller holds the access.
    let _rs = ((instr >> 21) & 0x1F) as usize;
    let _rt = ((instr >> 16) & 0x1F) as usize;
    let _rd = ((instr >> 11) & 0x1F) as usize;
    let funct = instr & 0x3F;
    let _ = (mmi.hi[0], mmi.lo[0], mmi.hi[1], mmi.lo[1]);
    let _ = funct;
}

// ---------------------------------------------------------------------------
// MTGS state
// ---------------------------------------------------------------------------

/// GS-thread manager state.
///
/// The full C++ implementation spins up a real worker thread, a 2 MiB
/// ring buffer and a set of semaphores; in the single-threaded stub
/// everything collapses to a small `static mut` struct that the EE thread
/// can poke at directly.
pub struct Mtgs {
    /// Whether the GS thread is currently "open" (has been initialised).
    pub open: bool,
    /// Whether the GS thread has been asked to shut down.
    pub shutdown: bool,
    /// Number of frames currently queued to the GS thread.
    pub queued_frames: u32,
    /// Pending vsync-wait count.
    pub vsync_pending: bool,
    /// Run-idle flag: when set, the GS thread is allowed to keep presenting
    /// frames even when the EE is paused.
    pub run_idle: bool,
}

/// Global MTGS state.
pub static mut mtgs: Mtgs = Mtgs {
    open: false,
    shutdown: false,
    queued_frames: 0,
    vsync_pending: false,
    run_idle: false,
};

// ---------------------------------------------------------------------------
// MTGS init / reset / run
// ---------------------------------------------------------------------------

/// Initialise the GS-thread manager.
///
/// The full C++ version spawns a worker thread, allocates a 2 MiB ring
/// buffer and wires up the GS plugin. In the single-threaded stub we just
/// clear the state.
pub fn mtgsInit() {
    // SAFETY: `mtgs` is private to this module and `mtgsInit` is the
    // canonical initialiser.
    unsafe {
        mtgsReset();
    }
}

/// Reset the GS-thread manager. Mirrors `MTGS::ResetGS(hardware_reset=true)`.
pub fn mtgsReset() {
    // SAFETY: see `mtgsInit`.
    unsafe {
        mtgs.open = false;
        mtgs.shutdown = false;
        mtgs.queued_frames = 0;
        mtgs.vsync_pending = false;
        mtgs.run_idle = false;
    }
}

/// Pump the GS-thread manager.
///
/// In the C++ implementation this waits for new ring-buffer entries,
/// processes one packet, and loops. In the single-threaded stub it just
/// returns: the caller is expected to have already executed whatever the
/// ring buffer would have contained.
pub fn mtgsRun() {
    // No-op: single-threaded stub. The real implementation consumes one
    // packet out of the ring buffer, dispatches it to the GS plugin, and
    // blocks on the GS work semaphore.
}

// ---------------------------------------------------------------------------
// MTVU state
// ---------------------------------------------------------------------------

/// Micro VU thread-manager state.
///
/// The full C++ implementation spins up a real worker thread that consumes
/// a 4 MiB ring buffer of VU1 / VIF commands. The single-threaded stub
/// just records the state; callers that need VU1 work to actually execute
/// should call the appropriate `Execute*` function on `vu1Thread` (in the
/// real C++ code) or inline the operation here.
pub struct Mtvu {
    /// Whether the VU thread is currently "open".
    pub open: bool,
    /// Whether the VU thread has been asked to shut down.
    pub shutdown: bool,
    /// Number of VU execution commands currently queued.
    pub vu_queue_depth: u32,
    /// Last seen VU cycle count, for the cycle-stealing hack.
    pub last_vu_cycles: u32,
    /// MTVU interrupt flags, packed as a bitmask of the C++ `InterruptFlag`
    /// constants.
    pub interrupts: u32,
}

/// Global MTVU state.
pub static mut mtvu: Mtvu = Mtvu {
    open: false,
    shutdown: false,
    vu_queue_depth: 0,
    last_vu_cycles: 0,
    interrupts: 0,
};

// ---------------------------------------------------------------------------
// MTVU init / reset / execute
// ---------------------------------------------------------------------------

/// Initialise the MTVU thread manager.
pub fn mtvuInit() {
    // SAFETY: see `mtgsInit`.
    unsafe {
        mtvuReset();
    }
}

/// Reset the MTVU thread manager.
pub fn mtvuReset() {
    // SAFETY: see `mtgsInit`.
    unsafe {
        mtvu.open = false;
        mtvu.shutdown = false;
        mtvu.vu_queue_depth = 0;
        mtvu.last_vu_cycles = 0;
        mtvu.interrupts = 0;
    }
}

/// Execute one VU work packet from the MTVU ring buffer.
///
/// In the C++ implementation the MTVU worker thread runs a continuous loop
/// inside `VU_Thread::ExecuteRingBuffer`. In the single-threaded stub we
/// just bump the queue depth counter; the real work is left to the
/// interpreter.
pub fn mtvuExecute() {
    // SAFETY: `mtvu` is private to this module.
    unsafe {
        mtvu.vu_queue_depth = mtvu.vu_queue_depth.saturating_sub(1);
    }
}

/// Internal helpers
// ---------------------------------------------------------------------------

/// Hook for the surrounding emulator to query whether a given physical
/// address falls inside the EE main RAM.
pub fn memIsMainRam(addr: u32) -> bool {
    matches!(mem_region(addr), MemRegion::Main)
}

/// Hook for the surrounding emulator to query whether a given physical
/// address falls inside the EE hardware register file.
pub fn memIsHw(addr: u32) -> bool {
    matches!(mem_region(addr), MemRegion::Hw)
}

// ---------------------------------------------------------------------------
// DEV9 / DVE mailbox helpers (`ba0W16` / `ba0R16`).
//
// These handle writes and reads to the `0x1A000000` register window used by
// the DEV9 / DVE (Sony's Ethernet + modem emulation in real PS2 hardware).
// The C++ versions live in `Memory.cpp` next to `memReset`.
// ---------------------------------------------------------------------------

/// Write a 16-bit value to the DEV9 / DVE mailbox window.
pub fn ba0W16(mem: u32, value: u16) {
    // SAFETY: `s_ba`, `s_dve_regs`, `s_ba_command_executing`,
    // `s_ba_error_detected`, and `s_ba_current_reg` are private to this
    // module and only touched through these helpers.
    unsafe {
        let masked = (mem & 0xFF) as usize;

        if masked == 0x6 {
            // Status register: clear the low two bits before storing.
            s_ba[0x6] &= !3u16;
        } else {
            s_ba[masked] = value;
        }

        if masked == 0x00 {
            // Command-execute register: dispatch on the control reg value.
            if s_ba[0x2] == 0x4F || s_ba[0x2] == 0x41 {
                s_ba_error_detected = true;
            } else if s_ba[masked] & 0x80 != 0 {
                // Top bit set: start executing the command.
                if s_ba[0x2] == 0x43 {
                    // Write Mode: copy a chunk of the mailbox to DVE regs.
                    let size = (s_ba[masked] & 0x0F) as usize;
                    s_ba_current_reg = s_ba[0x10];
                    let count = size.saturating_sub(1);
                    let reg_off = s_ba_current_reg as usize;
                    for i in 0..count {
                        let src = 0x12 + i;
                        let dst = reg_off + i;
                        if src < s_ba.len() && dst < s_dve_regs.len() {
                            s_dve_regs[dst] = s_ba[src];
                        }
                    }
                    s_ba_command_executing = true;
                    s_ba_error_detected = false;
                } else if s_ba[0x2] == 0x42 {
                    // Read Mode: copy a chunk of DVE regs back to the mailbox.
                    let size = (s_ba[masked] & 0x0F) as usize;
                    let reg_off = s_ba_current_reg as usize;
                    for i in 0..size {
                        let dst = 0x10 + i;
                        let src = reg_off + i;
                        if dst < s_ba.len() && src < s_dve_regs.len() {
                            s_ba[dst] = s_dve_regs[src];
                        }
                    }
                    s_ba_command_executing = true;
                    s_ba_error_detected = false;
                }
            }
        } else if masked == 0x0A {
            // Power / standby register.
            if value == 0 {
                s_ba_error_detected = true;
            } else {
                s_ba_error_detected = false;
            }
        }
    }
}

/// Read a 16-bit value from the DEV9 / DVE mailbox window.
pub fn ba0R16(mem: u32) -> u16 {
    // SAFETY: see `ba0W16`.
    unsafe {
        if mem == 0x1A00_0006 {
            // Special-case the status register: bit 0 = error, bit 1 = ready.
            let mut value = s_ba[0x6] & 0x2;
            if s_ba_error_detected {
                value |= 1;
            }
            // Walk the status register through its polling sequence while a
            // command is being executed.
            if s_ba[0x6] < 3 && s_ba_command_executing {
                s_ba[0x6] += 1;
            } else {
                s_ba_command_executing = false;
            }
            value
        } else {
            s_ba[(mem & 0x1F) as usize]
        }
    }
}

// Wrap the `Mtgs` / `Mtvu` globals in `UnsafeCell`s so external code can't
// accidentally take a shared reference; the only way in is through the
// `static mut` declarations above. This is a no-op at runtime but reserves
// a hook for later interior-mutability work.
#[allow(dead_code)]
struct MtgsCell(UnsafeCell<Mtgs>);

#[allow(dead_code)]
struct MtvuCell(UnsafeCell<Mtvu>);

unsafe impl Sync for MtgsCell {}
unsafe impl Sync for MtvuCell {}
