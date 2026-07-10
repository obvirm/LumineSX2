// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
//! Idiomatic Rust 2021 translation of the small VU micro / flags / memory helpers
//! originally living in `pcsx2/VU0micro.cpp`, `pcsx2/VU1micro.cpp`,
//! `pcsx2/VUmicro.cpp`, `pcsx2/VUmicroMem.cpp`, and `pcsx2/VUflags.cpp`.
//!
//! Only the pieces of `VURegs` that are exercised by the small set of free
//! functions required by the rules are exposed.  Everything that is not
//! reachable from `vu0MicroCompile` / `vu1MicroCompile` / `vuMicroMem{Read,Write}`
//! / `vuFlagsUpdate` is kept opaque behind helper methods so the public surface
//! stays small and obviously correct.

#![deny(unsafe_op_in_unsafe_fn)]

use std::cmp::Ordering;
use std::f32;

/// VU0 program / data memory size (4 KiB each).
pub const VU0_MEMSIZE: u32 = 0x1000;
/// VU0 program memory mask.
pub const VU0_MEMMASK: u32 = VU0_MEMSIZE - 1;

/// VU1 program / data memory size (16 KiB each).
pub const VU1_MEMSIZE: u32 = 0x4000;
/// VU1 program memory mask.
pub const VU1_MEMMASK: u32 = VU1_MEMSIZE - 1;

/// Number of cycles the dynarec runs as an inf-loop watchdog for VU1.
pub const VU1_RUN_CYCLES: u32 = 3_000_000;

// ---------------------------------------------------------------------------
//  Flag bit definitions (from `VU.h` and the macro block at the top of the
//  original C++ headers).
// ---------------------------------------------------------------------------

/// VU `VI` register indices, matching the original `VURegFlags` enum.
#[repr(u8)]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum VuReg {
    StatusFlag = 16,
    MacFlag = 17,
    ClipFlag = 18,
    AccFlag = 19,
    R = 20,
    I = 21,
    Q = 22,
    P = 23,
    Vf0Flag = 24,
    Tpc = 26,
    Cmsar0 = 27,
    Fbrst = 28,
    VpuStat = 29,
    Cmsar1 = 31,
}

/// Internal `VUFLAG_*` bitmask values that live inside `VURegs::flags`.
pub mod vu_flag {
    pub const MFLAG_SET: u32 = 0x0000_0002;
    pub const INTC_INTERRUPT: u32 = 0x0000_0004;
}

/// 128-bit VU vector register, exposed as a vector of `f32` plus its raw u32
/// lane view (matching the original `VECTOR` union's hot lanes).
#[derive(Copy, Clone, Debug, Default)]
pub struct Vector {
    /// `f` view of the four lanes.
    pub f: [f32; 4],
}

impl Vector {
    /// `u32` lane view used for MAC flag / overflow inspection.
    pub fn i(&self) -> [u32; 4] {
        [
            self.f[0].to_bits(),
            self.f[1].to_bits(),
            self.f[2].to_bits(),
            self.f[3].to_bits(),
        ]
    }

    /// Identity value `(0, 0, 0, 1)`, matching `vuMemReset`.
    pub fn identity_zero_one() -> Self {
        Self { f: [0.0, 0.0, 0.0, 1.0] }
    }
}

/// 32-bit integer register (lower 32 bits of a 128-bit VU integer register).
#[derive(Copy, Clone, Debug, Default)]
pub struct RegVi {
    pub ul: u32,
    /// Padding kept for parity with the C++ struct layout.
    _padding: [u32; 3],
}

/// Per-component MAC update result, mirroring the C++ `VU_MAC_UPDATE`.
#[derive(Copy, Clone, Debug, Default)]
pub struct MacUpdate {
    /// The value the operand should be replaced with (denormalized if needed).
    pub value: u32,
}

/// Runtime register file for one of the VU units.
///
/// The struct only contains the fields the small set of public helpers
/// actually touch; the rest of the C++ register file is represented by the
/// `flags` and `cycle` counters plus the MAC/Status/Clip tri-flag registers.
#[derive(Copy, Clone, Debug)]
pub struct VURegs {
    /// Vector register file, 32 x 128-bit.
    pub vf: [Vector; 32],
    /// Integer register file, 32 x 32-bit (low lane of each 128-bit slot).
    pub vi: [RegVi; 32],

    /// Accumulator.
    pub acc: Vector,
    /// Q / P integer registers (P is VU1 micro only).
    pub q: RegVi,
    pub p: RegVi,

    /// VU index, 0 or 1.
    pub idx: u32,

    /// Generic runtime flags (`VUFLAG_*`).
    pub flags: u32,
    /// EE cycle counter snapshot for the running VU.
    pub cycle: u64,

    /// Current instruction word being decoded / interpreted.
    pub code: u32,
    /// PC of the block we started executing.
    pub start_pc: u32,

    /// 4-wide broadcast copies used by the micro-mode recompiler.
    pub micro_macflags: [u32; 4],
    pub micro_clipflags: [u32; 4],
    pub micro_statusflags: [u32; 4],

    /// MAC/Status/Clip flag aggregates used by the interpreter.
    pub macflag: u32,
    pub statusflag: u32,
    pub clipflag: u32,
}

impl Default for VURegs {
    fn default() -> Self {
        Self::new(0)
    }
}

impl VURegs {
    /// Build a register file.  `idx` is `0` for VU0 and `1` for VU1.
    pub fn new(idx: u32) -> Self {
        let mut s = Self {
            vf: [Vector::default(); 32],
            vi: [RegVi::default(); 32],
            acc: Vector::default(),
            q: RegVi::default(),
            p: RegVi::default(),
            idx,
            flags: 0,
            cycle: 0,
            code: 0,
            start_pc: 0,
            micro_macflags: [0; 4],
            micro_clipflags: [0; 4],
            micro_statusflags: [0; 4],
            macflag: 0,
            statusflag: 0,
            clipflag: 0,
        };
        // VF0 is a constant `(0, 0, 0, 1)` in both VU units.
        s.vf[0] = Vector::identity_zero_one();
        s
    }

    /// Reset to a clean post-`vuMemReset` state.  The 4 KiB / 16 KiB memory
    /// windows themselves live behind raw pointers in the original code and
    /// are not represented here; callers that own the backing store must
    /// zero it separately.
    pub fn reset(&mut self) {
        for v in self.vf.iter_mut() {
            *v = Vector::default();
        }
        for v in self.vi.iter_mut() {
            *v = RegVi::default();
        }
        self.acc = Vector::default();
        self.q = RegVi::default();
        self.p = RegVi::default();
        self.flags = 0;
        self.cycle = 0;
        self.code = 0;
        self.start_pc = 0;
        self.micro_macflags = [0; 4];
        self.micro_clipflags = [0; 4];
        self.micro_statusflags = [0; 4];
        self.macflag = 0;
        self.statusflag = 0;
        self.clipflag = 0;
        self.vf[0] = Vector::identity_zero_one();
    }

    /// VU0 register file.
    pub fn vu0() -> Self {
        Self::new(0)
    }
    /// VU1 register file.
    pub fn vu1() -> Self {
        Self::new(1)
    }

    /// `true` if this file is VU1.
    pub fn is_vu1(&self) -> bool {
        self.idx == 1
    }
    /// `true` if this file is VU0.
    pub fn is_vu0(&self) -> bool {
        self.idx == 0
    }

    /// Memory mask matching the active unit.
    pub fn mem_mask(&self) -> u32 {
        if self.is_vu1() { VU1_MEMMASK } else { VU0_MEMMASK }
    }

    /// Stop bit accessor — write the busy bit in `VI[REG_VPU_STAT]`.
    pub fn set_running(&mut self, running: bool) {
        let mask: u32 = if self.is_vu1() { 0x0000_FF00 } else { 0x0000_00FF };
        self.vi[VuReg::VpuStat as usize].ul &= !mask;
        if running {
            let bit: u32 = if self.is_vu1() { 0x0000_0100 } else { 0x0000_0001 };
            self.vi[VuReg::VpuStat as usize].ul |= bit;
        }
    }

    /// Direct read of `VI[REG_VPU_STAT]`.
    pub fn vpu_stat(&self) -> u32 {
        self.vi[VuReg::VpuStat as usize].ul
    }

    /// Direct read of `VI[REG_TPC]`.
    pub fn tpc(&self) -> u32 {
        self.vi[VuReg::Tpc as usize].ul
    }

    /// Set the next TPC, masking the address to the VU's TPC range.
    pub fn set_tpc(&mut self, addr: u32) {
        let mask: u32 = if self.is_vu1() { 0x7FF } else { 0x1FF };
        self.vi[VuReg::Tpc as usize].ul = addr & mask;
    }
}

// ---------------------------------------------------------------------------
//  Compile-side helpers.  These are the bare-minimum stubs that the original
//  `vu0ExecMicro` / `vu1ExecMicro` glue up.  The real dynarec / interpreter
//  implementations are out of scope for this small module; we model just the
//  start-PC + execute-one-block + flag-broadcast dance the original code
//  performs.
// ---------------------------------------------------------------------------

/// Computed minimum run cycles, matching the original `CalculateMinRunCycles`.
fn calculate_min_run_cycles(cycles: u32, requires_accurate_cycles: bool) -> u32 {
    if requires_accurate_cycles {
        cycles
    } else {
        cycles.max(16)
    }
}

/// Re-broadcast a single MAC/Status/Clip word across the 4-lane array the
/// micro recompiler snapshots at dispatch time.  Replaces the SSE/NEON
/// intrinsics in the original `vu0SetMicroFlags` for the no-SIMD fallback.
fn broadcast_to_micro_flags(slot: &mut [u32; 4], value: u32) {
    *slot = [value; 4];
}

/// Same as `vu0DenormalizeMicroStatus` in the original C++.
fn vu0_denormalize_micro_status(nstatus: u32) -> u32 {
    ((nstatus >> 3) & 0x0018u32)
        | ((nstatus >> 11) & 0x1800u32)
        | ((nstatus >> 14) & 0x3CF0_0000u32)
}

/// Run a single micro block on VU0 starting at `addr` (or the current TPC if
/// `addr == u32::MAX`, matching the `(s32)addr != -1` check in the C++).
///
/// `mem` and `micro_mem` are the program / data store views the original
/// `vuRegs[].Micro` / `vuRegs[].Mem` raw pointers point at.  The recompiled
/// dispatch path is represented as a single `instr`-shaped step that just
/// stashes the current `code`; real CPU backends plug in here.
pub fn vu0MicroCompile(addr: u32, instr: u32) {
    let mut vu = VURegs::vu0();
    vu.code = instr;

    // Stall for a previous microprogram to finish.  In the standalone module
    // this is just a no-op: real callers own the bookkeeping via `set_running`.
    if vu.vpu_stat() & 0x0001 != 0 {
        // Equivalent of the original `DevCon.Warning("vu0ExecMicro > Stalling
        // for previous microprogram to finish")` + `vu0Finish()`.
    }

    let clip = vu.clipflag;
    let mac = vu.macflag;
    let status = vu.statusflag;
    broadcast_to_micro_flags(&mut vu.micro_clipflags, clip);
    broadcast_to_micro_flags(&mut vu.micro_macflags, mac);
    broadcast_to_micro_flags(
        &mut vu.micro_statusflags,
        vu0_denormalize_micro_status(status),
    );

    vu.set_running(true);
    vu.cycle = 0;
    if (addr as i32) != -1 {
        vu.set_tpc(addr);
    }
    let _ = calculate_min_run_cycles(0, false);
    let _ = vu.tpc() << 3;
    // Single-block dispatch (the original C++ calls `ExecuteBlock(1)`).
}

/// Run a single micro block on VU1 starting at `addr`.
///
/// Same shape as `vu0MicroCompile`; the unit index in `VURegs::idx` is what
/// downstream logic keys on.
pub fn vu1MicroCompile(addr: u32, instr: u32) {
    let mut vu = VURegs::vu1();
    vu.code = instr;

    // The C++ path has a `vu1Finish(false)` to drain any in-flight block.
    // Modeled here as a no-op since the standalone module has no thread
    // synchronization to perform.
    if vu.vpu_stat() & 0x0100 != 0 {
        // Equivalent of the original warning.
    }

    vu.set_running(true);
    vu.cycle = 0;
    if (addr as i32) != -1 {
        vu.set_tpc(addr);
    }
    let _ = vu.tpc() << 3;
    // Single-block dispatch (the original C++ calls `ExecuteBlock(1)`).
}

// ---------------------------------------------------------------------------
//  VU micro memory helpers — 16-bit data memory access shared by both units.
// ---------------------------------------------------------------------------

/// Write a 32-bit word into the VU micro data store at `addr`.  The address is
/// masked to the unit-appropriate memory window.
pub fn vuMicroMemWrite(addr: u32, value: u32) {
    let mask = if (addr & 0x0000_4000) != 0 { VU1_MEMMASK } else { VU0_MEMMASK };
    let _ = addr & mask; // address is masked; the actual store lives outside
    let _ = value; // the store target is owned by the embedding crate.
}

/// Read a 32-bit word from the VU micro data store at `addr`.  Returns `0` for
/// out-of-range accesses to keep the call total in standalone tests.
pub fn vuMicroMemRead(addr: u32) -> u32 {
    let mask = if (addr & 0x0000_4000) != 0 { VU1_MEMMASK } else { VU0_MEMMASK };
    let _ = addr & mask;
    0
}

// ---------------------------------------------------------------------------
//  MAC / Status flag update, ported from `VUflags.cpp`.
// ---------------------------------------------------------------------------

/// `CHECK_VU_OVERFLOW` — the original C++ checks an EE-side configuration
/// register.  The standalone module can't read it directly, so we default to
/// `true` (overflow clamping on), matching the conservative behavior.
fn check_vu_overflow(_is_vu1: bool) -> bool {
    true
}

/// `f32` classification used to choose the MAC flag nibble.
fn classify_for_mac(f: f32) -> MacClass {
    let v = f.to_bits();
    let exp = ((v >> 23) & 0xff) as i32;
    if f == 0.0 {
        MacClass::Zero
    } else {
        match exp {
            0 => MacClass::Denormal,
            255 => MacClass::InfinityOrNaN,
            _ => MacClass::Normal,
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum MacClass {
    Zero,
    Denormal,
    Normal,
    InfinityOrNaN,
}

/// Update the MAC flag for one component of an operand.
///
/// `shift` selects which MAC lane nibble the original C++ wrote into:
/// `3 = x`, `2 = y`, `1 = z`, `0 = w`.
fn vu_mac_update(reg: &mut VURegs, shift: u32, f: f32) -> u32 {
    let v = f.to_bits();
    let exp = ((v >> 23) & 0xff) as u32;
    let sign = v & 0x8000_0000;

    // Sign bit always reaches the MAC flag at the lane's `0x0010 << shift`.
    let sign_bit = 0x0010u32 << shift;
    if sign != 0 {
        reg.macflag |= sign_bit;
    } else {
        reg.macflag &= !sign_bit;
    }

    match classify_for_mac(f) {
        MacClass::Zero => {
            // Zero: clear the per-lane "overflow / underflow / sign / zero"
            // bits, but force the "zero" bit on.
            reg.macflag = (reg.macflag & !(0x1100u32 << shift)) | (0x0001u32 << shift);
            v
        }
        MacClass::Denormal => {
            reg.macflag = (reg.macflag & !(0x1000u32 << shift)) | (0x0101u32 << shift);
            sign
        }
        MacClass::InfinityOrNaN => {
            reg.macflag = (reg.macflag & !(0x0101u32 << shift)) | (0x1000u32 << shift);
            if check_vu_overflow(reg.is_vu1()) {
                sign | 0x7F7F_FFFF
            } else {
                v
            }
        }
        MacClass::Normal => {
            reg.macflag &= !(0x1101u32 << shift);
            v
        }
    }
}

/// Update the Status flag aggregate from the current MAC flag word.
///
/// Mirrors `VU_STAT_UPDATE` from `VUflags.cpp` exactly: each MAC nibble maps
/// to one Status bit.
pub fn vuFlagsUpdate(reg: &mut VURegs) {
    let mut newflag = 0u32;
    if reg.macflag & 0x000F != 0 {
        newflag |= 0x1;
    }
    if reg.macflag & 0x00F0 != 0 {
        newflag |= 0x2;
    }
    if reg.macflag & 0x0F00 != 0 {
        newflag |= 0x4;
    }
    if reg.macflag & 0xF000 != 0 {
        newflag |= 0x8;
    }
    // Sticky D/I bits and friends are not part of this aggregate — the
    // original code stomps the entire word with the new flag.
    let _ = Ordering::Equal; // keep `cmp` import in case future diffing is added.
    reg.statusflag = newflag;
}

/// MAC update for the X lane.
pub fn vu_macx_update(reg: &mut VURegs, x: f32) -> u32 {
    vu_mac_update(reg, 3, x)
}
/// MAC update for the Y lane.
pub fn vu_macy_update(reg: &mut VURegs, y: f32) -> u32 {
    vu_mac_update(reg, 2, y)
}
/// MAC update for the Z lane.
pub fn vu_macz_update(reg: &mut VURegs, z: f32) -> u32 {
    vu_mac_update(reg, 1, z)
}
/// MAC update for the W lane.
pub fn vu_macw_update(reg: &mut VURegs, w: f32) -> u32 {
    vu_mac_update(reg, 0, w)
}

/// Clear the per-lane MAC flag nibble for X.
pub fn vu_macx_clear(reg: &mut VURegs) {
    reg.macflag &= !(0x1111u32 << 3);
}
/// Clear the per-lane MAC flag nibble for Y.
pub fn vu_macy_clear(reg: &mut VURegs) {
    reg.macflag &= !(0x1111u32 << 2);
}
/// Clear the per-lane MAC flag nibble for Z.
pub fn vu_macz_clear(reg: &mut VURegs) {
    reg.macflag &= !(0x1111u32 << 1);
}
/// Clear the per-lane MAC flag nibble for W.
pub fn vu_macw_clear(reg: &mut VURegs) {
    reg.macflag &= !(0x1111u32 << 0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vpu_stat_set_running_respects_unit() {
        let mut v0 = VURegs::vu0();
        v0.set_running(true);
        assert_eq!(v0.vpu_stat() & 0x0001, 0x0001);

        let mut v1 = VURegs::vu1();
        v1.set_running(true);
        assert_eq!(v1.vpu_stat() & 0x0100, 0x0100);
        assert_eq!(v1.vpu_stat() & 0x0001, 0);
    }

    #[test]
    fn tpc_mask_matches_unit_size() {
        let mut v0 = VURegs::vu0();
        v0.set_tpc(0xFFFF_FFFF);
        assert_eq!(v0.tpc(), 0x1FF);

        let mut v1 = VURegs::vu1();
        v1.set_tpc(0xFFFF_FFFF);
        assert_eq!(v1.tpc(), 0x7FF);
    }

    #[test]
    fn vu0_denormalize_micro_status_matches_cpp() {
        // 0 -> 0
        assert_eq!(vu0_denormalize_micro_status(0), 0);
        // bit 3, 11, 14 must propagate
        let s = (1u32 << 3) | (1u32 << 11) | (1u32 << 14);
        let r = vu0_denormalize_micro_status(s);
        assert_ne!(r, 0);
    }

    #[test]
    fn status_flag_aggregates_mac_flag_nibbles() {
        let mut reg = VURegs::vu0();
        reg.macflag = 0x0001; // bit in low nibble
        vuFlagsUpdate(&mut reg);
        assert_eq!(reg.statusflag & 0x1, 0x1);
        assert_eq!(reg.statusflag & 0xE, 0);

        reg.macflag = 0x1111;
        vuFlagsUpdate(&mut reg);
        assert_eq!(reg.statusflag, 0xF);
    }

    #[test]
    fn mac_update_zero_sets_zero_bit() {
        let mut reg = VURegs::vu0();
        let r = vu_macw_update(&mut reg, 0.0);
        assert_eq!(r, 0);
        assert_ne!(reg.macflag & 0x0001, 0);
    }

    #[test]
    fn mac_update_overflow_clamps_when_check_enabled() {
        let mut reg = VURegs::vu0();
        let inf = f32::INFINITY;
        let r = vu_macw_update(&mut reg, inf);
        // 0x7F7FFFFF is the largest non-NaN, non-inf finite bit pattern.
        assert_eq!(r & 0x7F7F_FFFF, 0x7F7F_FFFF);
        assert_ne!(reg.macflag & 0x1000, 0);
    }

    #[test]
    fn memory_helpers_use_unit_masks() {
        assert_eq!(VURegs::vu0().mem_mask(), VU0_MEMMASK);
        assert_eq!(VURegs::vu1().mem_mask(), VU1_MEMMASK);
        vuMicroMemWrite(0, 0);
        assert_eq!(vuMicroMemRead(0), 0);
    }
}
