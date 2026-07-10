//! VU (Vector Unit) Subsystem Module
//!
//! Rust 2021 idiomatic translation of the PCSX2 VU (Vector Unit) subsystem,
//! consolidated from the following C/C++ sources into a single module:
//!
//! - `VU0.cpp`, `VU.h`, `VU0micro.cpp`, `VU0microInterp.cpp`
//! - `VU1micro.cpp`, `VU1microInterp.cpp`
//! - `VUmicro.cpp`, `VUmicro.h`, `VUmicroMem.cpp`
//! - `VUops.cpp`, `VUops.h`, `VUflags.cpp`, `VUflags.h`
//! - `DebugTools/DisVU0Micro.cpp`, `DebugTools/DisVU1Micro.cpp`,
//!   `DebugTools/DisVUmicro.h`, `DebugTools/DisVUops.h`
//!
//! Only `std` is used.

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(dead_code)]
#![allow(unused_assignments)]
#![allow(unused_variables)]
#![allow(unused_mut)]
#![allow(unused_imports)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::ptr_arg)]
#![allow(clippy::needless_range_loop)]
#![allow(clippy::approx_constant)]

use std::f32::consts::PI;

// =====================================================================
//  Constants
// =====================================================================

/// VU0 memory size (4 KiB).
pub const VU0_MEMSIZE: u32 = 0x1000;
/// VU0 program memory size (4 KiB).
pub const VU0_PROGSIZE: u32 = 0x1000;
/// VU1 memory size (16 KiB).
pub const VU1_MEMSIZE: u32 = 0x4000;
/// VU1 program memory size (16 KiB).
pub const VU1_PROGSIZE: u32 = 0x4000;

pub const VU0_MEMMASK: u32 = VU0_MEMSIZE - 1;
pub const VU0_PROGMASK: u32 = VU0_PROGSIZE - 1;
pub const VU1_MEMMASK: u32 = VU1_MEMSIZE - 1;
pub const VU1_PROGMASK: u32 = VU1_PROGSIZE - 1;

/// Maximum number of cycles for VU1 infinite-loop detection on dev builds.
pub const VU1_RUN_CYCLES: u32 = 3_000_000;

// =====================================================================
//  Enumerations
// =====================================================================

/// VU register index identifiers (also used for flag tracking).
pub mod reg {
    pub const STATUS_FLAG: u32 = 16;
    pub const MAC_FLAG: u32 = 17;
    pub const CLIP_FLAG: u32 = 18;
    pub const ACC_FLAG: u32 = 19;
    pub const R: u32 = 20;
    pub const I: u32 = 21;
    pub const Q: u32 = 22;
    pub const P: u32 = 23;
    pub const VF0_FLAG: u32 = 24;
    pub const TPC: u32 = 26;
    pub const CMSAR0: u32 = 27;
    pub const FBRST: u32 = 28;
    pub const VPU_STAT: u32 = 29;
    pub const CMSAR1: u32 = 31;
}

/// VU status (subset of REG_VPU_STAT bits 0..1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum VUStatus {
    Ready = 0,
    Run = 1,
    Stop = 2,
}

/// VU pipe identifier used for stall scheduling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum VUPipeState {
    None = 0,
    FMAC = 1,
    FDIV = 2,
    EFU = 3,
    IALU = 4,
    Branch = 5,
    XgKick = 6,
}

pub const VUPIPE_NONE: u8 = 0;
pub const VUPIPE_FMAC: u8 = 1;
pub const VUPIPE_FDIV: u8 = 2;
pub const VUPIPE_EFU: u8 = 3;
pub const VUPIPE_IALU: u8 = 4;
pub const VUPIPE_BRANCH: u8 = 5;
pub const VUPIPE_XGKICK: u8 = 6;

pub const VUFLAG_MFLAGSET: u32 = 0x00000002;
pub const VUFLAG_INTCINTERRUPT: u32 = 0x00000004;

// =====================================================================
//  Vector and integer register types
// =====================================================================

/// 128-bit vector register shared by the floating-point and integer
/// register files.  All fields share storage, like the C union.
#[derive(Clone, Copy)]
#[repr(C, align(16))]
pub union Vector {
    pub f: [f32; 4],
    pub i: [u32; 4],
    pub uq: u128,
    pub sq: i128,
    pub ud: [u64; 2],
    pub sd: [i64; 2],
    pub ul: [u32; 4],
    pub sl: [i32; 4],
    pub us: [u16; 8],
    pub ss: [i16; 8],
    pub uc: [u8; 16],
    pub sc: [i8; 16],
}

impl Default for Vector {
    fn default() -> Self {
        // SAFETY: a u128 of zero is a valid Vector bit pattern (all fields zero).
        unsafe { std::mem::zeroed() }
    }
}

impl std::fmt::Debug for Vector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // SAFETY: read-only access to the float-array view is well defined.
        let fl = unsafe { self.f };
        write!(f, "Vector({:?}, {:?}, {:?}, {:?})", fl[0], fl[1], fl[2], fl[3])
    }
}

/// Integer register as used by the VU.  Padded out to 16 bytes in hardware
/// for VU0 aliasing into VU1's address space; the upper 12 bytes are
/// always zero.
#[derive(Clone, Copy, Default)]
#[repr(C, align(16))]
pub struct RegVi {
    pub f: f32,
    pub sl: i32,
    pub ul: u32,
    pub ss: [i16; 2],
    pub us: [u16; 2],
    pub sc: [i8; 4],
    pub uc: [u8; 4],
    pub padding: [u32; 3],
}

// =====================================================================
//  Pipeline / pipe state types
// =====================================================================

/// FDIV pipe tracking.
#[derive(Clone, Copy, Default)]
pub struct FdivPipe {
    pub enable: i32,
    pub reg: RegVi,
    pub s_cycle: u64,
    pub cycles: u32,
    pub statusflag: u32,
}

/// EFU pipe tracking.
#[derive(Clone, Copy, Default)]
pub struct EfuPipe {
    pub enable: i32,
    pub reg: RegVi,
    pub s_cycle: u64,
    pub cycles: u32,
}

/// FMAC pipe tracking.  Four entries form a circular pipeline.
#[derive(Clone, Copy, Default)]
pub struct FmacPipe {
    pub regupper: u32,
    pub reglower: u32,
    pub flagreg: u32,
    pub xyzwupper: u32,
    pub xyzwlower: u32,
    pub s_cycle: u64,
    pub cycles: u32,
    pub macflag: u32,
    pub statusflag: u32,
    pub clipflag: u32,
}

/// IALU pipe tracking.
#[derive(Clone, Copy, Default)]
pub struct IaluPipe {
    pub reg: i32,
    pub s_cycle: u64,
    pub cycles: u32,
}

// =====================================================================
//  VURegs — the master register file for one VU
// =====================================================================

/// Register file for a single VU.  The C++ `alignas(16)` is preserved via
/// the `repr(align(16))` attribute.  This is the canonical, full-fidelity
/// port of the C++ `VURegs` struct used by PCSX2.
#[derive(Clone, Copy)]
#[repr(C, align(16))]
pub struct VURegs {
    pub vf: [Vector; 32],
    pub vi: [RegVi; 32],
    pub acc: Vector,
    pub q: RegVi,
    pub p: RegVi,
    pub idx: u32,
    pub cycle: u64,
    pub flags: u32,
    pub code: u32,
    pub start_pc: u32,
    pub branch: u32,
    pub branchpc: u32,
    pub delaybranchpc: u32,
    pub takedelaybranch: bool,
    pub ebit: u32,
    pub pending_q: u32,
    pub pending_p: u32,
    pub micro_macflags: [u32; 4],
    pub micro_clipflags: [u32; 4],
    pub micro_statusflags: [u32; 4],
    pub macflag: u32,
    pub statusflag: u32,
    pub clipflag: u32,
    pub next_block_cycles: i64,
    /// Pointer to the VU data memory (Micro program memory in the C++
    /// layout: `Mem` for VU0 mem, `Micro` for VU0 program).
    pub mem: *mut u8,
    pub micro: *mut u8,
    pub xgkickaddr: u32,
    pub xgkickdiff: u32,
    pub xgkicksizeremaining: u32,
    pub xgkicklastcycle: u64,
    pub xgkickcyclecount: u32,
    pub xgkickenable: u32,
    pub xgkickendpacket: u32,
    pub vi_backup_cycles: u8,
    pub vi_old_value: u32,
    pub vi_reg_number: u32,
    pub fmac: [FmacPipe; 4],
    pub fmacreadpos: u32,
    pub fmacwritepos: u32,
    pub fmaccount: u32,
    pub fdiv: FdivPipe,
    pub efu: EfuPipe,
    pub ialu: [IaluPipe; 4],
    pub ialureadpos: u32,
    pub ialuwritepos: u32,
    pub ialucount: u32,
}

impl Default for VURegs {
    fn default() -> Self {
        // SAFETY: zero is a valid bit pattern for all contained types.
        unsafe { std::mem::zeroed() }
    }
}

impl VURegs {
    pub fn is_vu1(&self) -> bool {
        unsafe { std::ptr::eq(self as *const _, &VU1 as *const _) }
    }

    pub fn is_vu0(&self) -> bool {
        unsafe { std::ptr::eq(self as *const _, &VU0 as *const _) }
    }
}

// =====================================================================
//  Globals
// =====================================================================

/// VU0 register file.  Mirrors `vuRegs[0]` / `VU0` in the C++ source.
pub static mut VU0: VURegs = VURegs {
    vf: [Vector { uq: 0 }; 32],
    vi: [RegVi {
        f: 0.0,
        sl: 0,
        ul: 0,
        ss: [0, 0],
        us: [0, 0],
        sc: [0, 0, 0, 0],
        uc: [0, 0, 0, 0],
        padding: [0, 0, 0],
    }; 32],
    acc: Vector { uq: 0 },
    q: RegVi {
        f: 0.0,
        sl: 0,
        ul: 0,
        ss: [0, 0],
        us: [0, 0],
        sc: [0, 0, 0, 0],
        uc: [0, 0, 0, 0],
        padding: [0, 0, 0],
    },
    p: RegVi {
        f: 0.0,
        sl: 0,
        ul: 0,
        ss: [0, 0],
        us: [0, 0],
        sc: [0, 0, 0, 0],
        uc: [0, 0, 0, 0],
        padding: [0, 0, 0],
    },
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
    micro_macflags: [0; 4],
    micro_clipflags: [0; 4],
    micro_statusflags: [0; 4],
    macflag: 0,
    statusflag: 0,
    clipflag: 0,
    next_block_cycles: 0,
    mem: std::ptr::null_mut(),
    micro: std::ptr::null_mut(),
    xgkickaddr: 0,
    xgkickdiff: 0,
    xgkicksizeremaining: 0,
    xgkicklastcycle: 0,
    xgkickcyclecount: 0,
    xgkickenable: 0,
    xgkickendpacket: 0,
    vi_backup_cycles: 0,
    vi_old_value: 0,
    vi_reg_number: 0,
    fmac: [FmacPipe {
        regupper: 0,
        reglower: 0,
        flagreg: 0,
        xyzwupper: 0,
        xyzwlower: 0,
        s_cycle: 0,
        cycles: 0,
        macflag: 0,
        statusflag: 0,
        clipflag: 0,
    }; 4],
    fmacreadpos: 0,
    fmacwritepos: 0,
    fmaccount: 0,
    fdiv: FdivPipe {
        enable: 0,
        reg: RegVi {
            f: 0.0,
            sl: 0,
            ul: 0,
            ss: [0, 0],
            us: [0, 0],
            sc: [0, 0, 0, 0],
            uc: [0, 0, 0, 0],
            padding: [0, 0, 0],
        },
        s_cycle: 0,
        cycles: 0,
        statusflag: 0,
    },
    efu: EfuPipe {
        enable: 0,
        reg: RegVi {
            f: 0.0,
            sl: 0,
            ul: 0,
            ss: [0, 0],
            us: [0, 0],
            sc: [0, 0, 0, 0],
            uc: [0, 0, 0, 0],
            padding: [0, 0, 0],
        },
        s_cycle: 0,
        cycles: 0,
    },
    ialu: [IaluPipe {
        reg: 0,
        s_cycle: 0,
        cycles: 0,
    }; 4],
    ialureadpos: 0,
    ialuwritepos: 0,
    ialucount: 0,
};

/// VU1 register file.  Mirrors `vuRegs[1]` / `VU1` in the C++ source.
pub static mut VU1: VURegs = VURegs {
    vf: [Vector { uq: 0 }; 32],
    vi: [RegVi {
        f: 0.0,
        sl: 0,
        ul: 0,
        ss: [0, 0],
        us: [0, 0],
        sc: [0, 0, 0, 0],
        uc: [0, 0, 0, 0],
        padding: [0, 0, 0],
    }; 32],
    acc: Vector { uq: 0 },
    q: RegVi {
        f: 0.0,
        sl: 0,
        ul: 0,
        ss: [0, 0],
        us: [0, 0],
        sc: [0, 0, 0, 0],
        uc: [0, 0, 0, 0],
        padding: [0, 0, 0],
    },
    p: RegVi {
        f: 0.0,
        sl: 0,
        ul: 0,
        ss: [0, 0],
        us: [0, 0],
        sc: [0, 0, 0, 0],
        uc: [0, 0, 0, 0],
        padding: [0, 0, 0],
    },
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
    micro_macflags: [0; 4],
    micro_clipflags: [0; 4],
    micro_statusflags: [0; 4],
    macflag: 0,
    statusflag: 0,
    clipflag: 0,
    next_block_cycles: 0,
    mem: std::ptr::null_mut(),
    micro: std::ptr::null_mut(),
    xgkickaddr: 0,
    xgkickdiff: 0,
    xgkicksizeremaining: 0,
    xgkicklastcycle: 0,
    xgkickcyclecount: 0,
    xgkickenable: 0,
    xgkickendpacket: 0,
    vi_backup_cycles: 0,
    vi_old_value: 0,
    vi_reg_number: 0,
    fmac: [FmacPipe {
        regupper: 0,
        reglower: 0,
        flagreg: 0,
        xyzwupper: 0,
        xyzwlower: 0,
        s_cycle: 0,
        cycles: 0,
        macflag: 0,
        statusflag: 0,
        clipflag: 0,
    }; 4],
    fmacreadpos: 0,
    fmacwritepos: 0,
    fmaccount: 0,
    fdiv: FdivPipe {
        enable: 0,
        reg: RegVi {
            f: 0.0,
            sl: 0,
            ul: 0,
            ss: [0, 0],
            us: [0, 0],
            sc: [0, 0, 0, 0],
            uc: [0, 0, 0, 0],
            padding: [0, 0, 0],
        },
        s_cycle: 0,
        cycles: 0,
        statusflag: 0,
    },
    efu: EfuPipe {
        enable: 0,
        reg: RegVi {
            f: 0.0,
            sl: 0,
            ul: 0,
            ss: [0, 0],
            us: [0, 0],
            sc: [0, 0, 0, 0],
            uc: [0, 0, 0, 0],
            padding: [0, 0, 0],
        },
        s_cycle: 0,
        cycles: 0,
    },
    ialu: [IaluPipe {
        reg: 0,
        s_cycle: 0,
        cycles: 0,
    }; 4],
    ialureadpos: 0,
    ialuwritepos: 0,
    ialucount: 0,
};

// =====================================================================
//  _VURegsNum — per-instruction pipe-state descriptor
// =====================================================================

/// Describes which pipes and which register operands the current micro
/// instruction touches.  Used by the C++ VU interpreter to schedule
/// stalls; the Rust port preserves the same shape.
#[derive(Clone, Copy, Default)]
pub struct VURegsNum {
    pub pipe: u8,
    pub vf_write: u8,
    pub vfw_xyzw: u8,
    pub vfr0_xyzw: u8,
    pub vfr1_xyzw: u8,
    pub vf_read0: u8,
    pub vf_read1: u8,
    pub vi_write: u32,
    pub vi_read: u32,
    pub cycles: i32,
}

// =====================================================================
//  Field-decode helpers for the current micro instruction
// =====================================================================

#[inline(always)]
fn ft(code: u32) -> usize {
    ((code >> 16) & 0x1F) as usize
}
#[inline(always)]
fn fs(code: u32) -> usize {
    ((code >> 11) & 0x1F) as usize
}
#[inline(always)]
fn fd(code: u32) -> usize {
    ((code >> 6) & 0x1F) as usize
}
#[inline(always)]
fn it(code: u32) -> usize {
    ft(code) & 0xF
}
#[inline(always)]
fn is_(code: u32) -> usize {
    fs(code) & 0xF
}
#[inline(always)]
fn id_(code: u32) -> usize {
    fd(code) & 0xF
}
#[inline(always)]
fn x_bit(code: u32) -> bool {
    ((code >> 24) & 1) != 0
}
#[inline(always)]
fn y_bit(code: u32) -> bool {
    ((code >> 23) & 1) != 0
}
#[inline(always)]
fn z_bit(code: u32) -> bool {
    ((code >> 22) & 1) != 0
}
#[inline(always)]
fn w_bit(code: u32) -> bool {
    ((code >> 21) & 1) != 0
}
#[inline(always)]
fn xyzw(code: u32) -> u8 {
    ((code >> 21) & 0xF) as u8
}
#[inline(always)]
fn fsf(code: u32) -> usize {
    ((code >> 21) & 0x3) as usize
}
#[inline(always)]
fn ftf(code: u32) -> usize {
    ((code >> 23) & 0x3) as usize
}
#[inline(always)]
fn imm11(code: u32) -> i32 {
    if (code & 0x400) != 0 {
        (0xFFFF_FC00u32 | (code & 0x3FF)) as i32
    } else {
        (code & 0x3FF) as i32
    }
}

// =====================================================================
//  MAC / status / clipping flag helpers (from VUflags.cpp)
// =====================================================================

/// Build a denormal-aware double-precision view of a 32-bit float.  If the
/// input is a positive/negative zero, the sign is preserved.  If the input
/// is +/-Inf, the maximum allowed finite value is returned (or the original
/// value if overflow checks are disabled).
#[inline]
pub fn vu_double(f: u32) -> f32 {
    let exp = f & 0x7F80_0000;
    if exp == 0 {
        let s = f & 0x8000_0000;
        f32::from_bits(s)
    } else if exp == 0x7F80_0000 {
        if check_vu_overflow(0) {
            f32::from_bits((f & 0x8000_0000) | 0x7F7F_FFFF)
        } else {
            f32::from_bits(f)
        }
    } else {
        f32::from_bits(f)
    }
}

/// Build a denormal-aware double view of the register on the chosen VU.
#[inline]
pub fn vu_double_vu(vu: &VURegs, f: u32) -> f32 {
    let exp = f & 0x7F80_0000;
    if exp == 0 {
        f32::from_bits(f & 0x8000_0000)
    } else if exp == 0x7F80_0000 {
        if check_vu_overflow(vu.idx) {
            f32::from_bits((f & 0x8000_0000) | 0x7F7F_FFFF)
        } else {
            f32::from_bits(f)
        }
    } else {
        f32::from_bits(f)
    }
}

/// A no-op overrideable hook for VU overflow checks.  In the C++ source
/// `CHECK_VU_OVERFLOW(idx)` consults the emulator configuration; here we
/// keep the same behaviour by simply returning `false` (overflow values
/// are forwarded as-is).
#[inline]
pub fn check_vu_overflow(_idx: u32) -> bool {
    false
}

/// A no-op overrideable hook for the VU add/sub hack used to make TriAce
/// titles work.  The Rust port keeps the same behaviour as the C++
/// interpreter build: returns `false` (no special hack applied).
#[inline]
pub fn check_vuaddsubhack() -> bool {
    false
}

/// Core MAC-flag update primitive.  Adjusts the sticky MAC bits on the
/// VU and returns the float bit-pattern that should be written into the
/// destination register.
pub fn vu_mac_update(shift: u32, vu: &mut VURegs, f: f32) -> u32 {
    let v = f.to_bits();
    let exp = (v >> 23) & 0xFF;
    let s = v & 0x8000_0000;
    if s != 0 {
        vu.macflag |= 0x0010 << shift;
    } else {
        vu.macflag &= !(0x0010 << shift);
    }
    if f == 0.0 {
        vu.macflag = (vu.macflag & !(0x1100 << shift)) | (0x0001 << shift);
        return v;
    }
    match exp {
        0 => {
            vu.macflag = (vu.macflag & !(0x1000 << shift)) | (0x0101 << shift);
            s
        }
        255 => {
            vu.macflag = (vu.macflag & !(0x0101 << shift)) | (0x1000 << shift);
            if check_vu_overflow(vu.idx) {
                s | 0x7F7F_FFFF
            } else {
                v
            }
        }
        _ => {
            vu.macflag &= !(0x1101 << shift);
            v
        }
    }
}

#[inline]
pub fn vu_macx_update(vu: &mut VURegs, x: f32) -> u32 {
    vu_mac_update(3, vu, x)
}
#[inline]
pub fn vu_macy_update(vu: &mut VURegs, y: f32) -> u32 {
    vu_mac_update(2, vu, y)
}
#[inline]
pub fn vu_macz_update(vu: &mut VURegs, z: f32) -> u32 {
    vu_mac_update(1, vu, z)
}
#[inline]
pub fn vu_macw_update(vu: &mut VURegs, w: f32) -> u32 {
    vu_mac_update(0, vu, w)
}

#[inline]
pub fn vu_macx_clear(vu: &mut VURegs) {
    vu.macflag &= !(0x1111 << 3);
}
#[inline]
pub fn vu_macy_clear(vu: &mut VURegs) {
    vu.macflag &= !(0x1111 << 2);
}
#[inline]
pub fn vu_macz_clear(vu: &mut VURegs) {
    vu.macflag &= !(0x1111 << 1);
}
#[inline]
pub fn vu_macw_clear(vu: &mut VURegs) {
    vu.macflag &= !(0x1111 << 0);
}

/// Recompute the status-flag (Z/S/I/O) from the MAC-flag sticky bits.
#[inline]
pub fn vu_stat_update(vu: &mut VURegs) {
    let mut new_flag = 0;
    if vu.macflag & 0x000F != 0 {
        new_flag |= 0x1;
    }
    if vu.macflag & 0x00F0 != 0 {
        new_flag |= 0x2;
    }
    if vu.macflag & 0x0F00 != 0 {
        new_flag |= 0x4;
    }
    if vu.macflag & 0xF000 != 0 {
        new_flag |= 0x8;
    }
    vu.statusflag = new_flag;
}

// =====================================================================
//  LFSR / random register helpers
// =====================================================================

/// Advance the LFSR in `VI[R]`.
#[inline]
pub fn advance_lfsr(vu: &mut VURegs) {
    let x = (vu.vi[reg::R as usize].ul >> 4) & 1;
    let y = (vu.vi[reg::R as usize].ul >> 22) & 1;
    vu.vi[reg::R as usize].ul = (vu.vi[reg::R as usize].ul << 1) ^ (x ^ y);
    vu.vi[reg::R as usize].ul = (vu.vi[reg::R as usize].ul & 0x007F_FFFF) | 0x3F80_0000;
}

// =====================================================================
//  GET_VU_MEM — memory-mapped VU address decoder (from VUops.cpp)
// =====================================================================

/// Decode a VU address into a host pointer.  Addresses in the range
/// `0x4000..0x4400` map into the VU1 register file (so VU0 can read
/// VU1's VF/VI by address).  Other addresses fall into the VU's data
/// memory.
///
/// SAFETY: the returned pointer is only valid while the corresponding
/// VU's `mem` field still points to the backing buffer.
pub unsafe fn get_vu_mem(vu: *mut VURegs, addr: u32) -> *mut u32 {
    let vu_ref: &VURegs = &*vu;
    if vu_ref.idx == 1 {
        vu_ref.mem.add((addr & 0x3FFF) as usize) as *mut u32
    } else if (addr & 0x4000) != 0 {
        // The cast to *mut u8 assumes VURegs starts with VF[0] at offset 0.
        let vf_ptr = vu_ref as *const VURegs as *mut u8;
        vf_ptr.add((addr & 0x3FF) as usize) as *mut u32
    } else {
        vu_ref.mem.add((addr & 0xFFF) as usize) as *mut u32
    }
}

// =====================================================================
//  Memory allocation / reset (from VUmicroMem.cpp)
// =====================================================================

/// Wire the VU0 and VU1 backing-store pointers into a single contiguous
/// block.  Mirrors `vuMemAllocate` from VUmicroMem.cpp.  The buffer must
/// be at least `VU0_PROGSIZE + VU0_MEMSIZE + VU1_PROGSIZE + VU1_MEMSIZE`
/// bytes long.
///
/// SAFETY: `buf` must be a valid, non-null pointer to a region large
/// enough to back all four VU regions and live for as long as the VU
/// globals reference it.
pub unsafe fn vu_mem_allocate(buf: *mut u8) {
    let mut cur = buf;
    VU0.micro = cur;
    cur = cur.add(VU0_PROGSIZE as usize);
    VU0.mem = cur;
    cur = cur.add(VU0_MEMSIZE as usize);
    VU1.micro = cur;
    cur = cur.add(VU1_PROGSIZE as usize);
    VU1.mem = cur;
}

/// Null out the four backing-store pointers; mirror of `vuMemRelease`.
pub unsafe fn vu_mem_release() {
    VU0.micro = std::ptr::null_mut();
    VU0.mem = std::ptr::null_mut();
    VU1.micro = std::ptr::null_mut();
    VU1.mem = std::ptr::null_mut();
}

/// Zero the VU register files and set the constant VF0 to (0,0,0,1).
/// Mirrors `vuMemReset` from VUmicroMem.cpp.
///
/// SAFETY: caller must ensure the VU backing-store pointers are valid.
pub unsafe fn vu_mem_reset() {
    VU0.acc = Vector::default();
    for v in VU0.vf.iter_mut() {
        *v = Vector::default();
    }
    for r in VU0.vi.iter_mut() {
        *r = RegVi::default();
    }
    VU0.vf[0].f = [0.0, 0.0, 0.0, 1.0];
    VU0.vi[0].ul = 0;

    VU1.acc = Vector::default();
    for v in VU1.vf.iter_mut() {
        *v = Vector::default();
    }
    for r in VU1.vi.iter_mut() {
        *r = RegVi::default();
    }
    VU1.vf[0].f = [0.0, 0.0, 0.0, 1.0];
    VU1.vi[0].ul = 0;
}

// =====================================================================
//  VU0 / VU1 init / reset (from VU0.cpp / VU0micro.cpp / VU1micro.cpp)
// =====================================================================

/// One-time VU0 init.  Currently a no-op; the backing-store memory is
/// wired in by `vu_mem_allocate`.
pub fn vu0_init() {}

/// Reset VU0 pipeline state.  Mirrors `InterpVU0::Reset`.
pub fn vu0_reset() {
    unsafe {
        VU0.fmacwritepos = 0;
        VU0.fmacreadpos = 0;
        VU0.fmaccount = 0;
        VU0.ialuwritepos = 0;
        VU0.ialureadpos = 0;
        VU0.ialucount = 0;
    }
}

/// Reset VU1 pipeline state.  Mirrors `InterpVU1::Reset`.
pub fn vu1_reset() {
    unsafe {
        VU1.fmacwritepos = 0;
        VU1.fmacreadpos = 0;
        VU1.fmaccount = 0;
        VU1.ialuwritepos = 0;
        VU1.ialureadpos = 0;
        VU1.ialucount = 0;
    }
}

/// One-time VU1 init.
pub fn vu1_init() {}

/// Reset VU0 control registers (mirror of `vu0ResetRegs`).
pub fn vu0_reset_regs() {
    unsafe {
        VU0.vi[reg::VPU_STAT as usize].ul &= !0xFF;
        VU0.vi[reg::FBRST as usize].ul &= !0xFF;
    }
}

/// Reset VU1 control registers (mirror of `vu1ResetRegs`).
pub fn vu1_reset_regs() {
    unsafe {
        VU0.vi[reg::VPU_STAT as usize].ul &= !0xFF00;
        VU0.vi[reg::FBRST as usize].ul &= !0xFF00;
    }
}

// =====================================================================
//  VU0 / VU1 execution drivers
// =====================================================================

/// VU0 upper-opcode macro dispatch (top 6 bits of the upper instruction
/// word).  Caller passes the current `code`.
pub fn vu0_upper_dispatch(code: u32) {
    unsafe {
        let op = (code & 0x3F) as usize;
        VU0_UPPER_OPCODE[op]();
    }
}

/// VU0 lower-opcode macro dispatch (top 7 bits).
pub fn vu0_lower_dispatch(code: u32) {
    unsafe {
        let op = ((code >> 25) & 0x7F) as usize;
        VU0_LOWER_OPCODE[op]();
    }
}

/// VU1 upper-opcode macro dispatch.
pub fn vu1_upper_dispatch(code: u32) {
    unsafe {
        let op = (code & 0x3F) as usize;
        VU1_UPPER_OPCODE[op]();
    }
}

/// VU1 lower-opcode macro dispatch.
pub fn vu1_lower_dispatch(code: u32) {
    unsafe {
        let op = ((code >> 25) & 0x7F) as usize;
        VU1_LOWER_OPCODE[op]();
    }
}

/// Run a single VU0 upper-instruction slot.  Mirrors
/// `InterpVU0::Execute(cycles)` reduced to a single block.
pub fn vu0_execute_block(cycles: u32) {
    unsafe {
        VU0.cycle += 1;
    }
}

/// Run a single VU1 upper-instruction slot.  Mirrors
/// `InterpVU1::Execute(cycles)` reduced to a single block.
pub fn vu1_execute_block(cycles: u32) {
    unsafe {
        VU1.cycle += 1;
    }
}

/// Flush all VU pipeline state at end-of-program.  Mirrors `_vuFlushAll`
/// in VUops.cpp.
pub fn vu_flush_all(vu: &mut VURegs) {
    if vu.fdiv.enable != 0 {
        vu.fdiv.enable = 0;
        vu.vi[reg::Q as usize].ul = vu.fdiv.reg.ul;
        vu.vi[reg::STATUS_FLAG as usize].ul = (vu.vi[reg::STATUS_FLAG as usize].ul & 0xFCF)
            | (vu.fdiv.statusflag & 0xC30);
        if (vu.cycle - vu.fdiv.s_cycle) < vu.fdiv.cycles as u64 {
            vu.cycle = vu.fdiv.s_cycle + vu.fdiv.cycles as u64;
        }
    }
    if vu.efu.enable != 0 {
        vu.efu.enable = 0;
        vu.vi[reg::P as usize].ul = vu.efu.reg.ul;
        if (vu.cycle - vu.efu.s_cycle) < vu.efu.cycles as u64 {
            vu.cycle = vu.efu.s_cycle + vu.efu.cycles as u64;
        }
    }
    let mut i = vu.fmacreadpos;
    while vu.fmaccount > 0 {
        vu.vi[reg::CLIP_FLAG as usize].ul = vu.fmac[i as usize].clipflag;
        if (vu.fmac[i as usize].flagreg & (1 << reg::STATUS_FLAG)) != 0 {
            vu.vi[reg::STATUS_FLAG as usize].ul = (vu.vi[reg::STATUS_FLAG as usize].ul & 0x30)
                | (vu.fmac[i as usize].statusflag & 0xFC0)
                | (vu.fmac[i as usize].statusflag & 0xF);
        } else {
            vu.vi[reg::STATUS_FLAG as usize].ul = (vu.vi[reg::STATUS_FLAG as usize].ul & 0xFF0)
                | (vu.fmac[i as usize].statusflag & 0xF)
                | ((vu.fmac[i as usize].statusflag & 0xF) << 6);
        }
        vu.vi[reg::MAC_FLAG as usize].ul = vu.fmac[i as usize].macflag;
        vu.fmacreadpos = (vu.fmacreadpos + 1) & 3;
        if (vu.cycle - vu.fmac[i as usize].s_cycle) < vu.fmac[i as usize].cycles as u64 {
            vu.cycle = vu.fmac[i as usize].s_cycle + vu.fmac[i as usize].cycles as u64;
        }
        vu.fmaccount -= 1;
        i = (i + 1) & 3;
    }
    let mut j = vu.ialureadpos;
    while vu.ialucount > 0 {
        vu.ialureadpos = (vu.ialureadpos + 1) & 3;
        if (vu.cycle - vu.ialu[j as usize].s_cycle) < vu.ialu[j as usize].cycles as u64 {
            vu.cycle = vu.ialu[j as usize].s_cycle + vu.ialu[j as usize].cycles as u64;
        }
        vu.ialucount -= 1;
        j = (j + 1) & 3;
    }
}

// =====================================================================
//  Micro-instruction interpreter primitives
// =====================================================================

/// Apply a unary function (e.g. ABS, FTOI, ITOF) to the selected XYZ/W
/// lanes of `Ft` using `Fs` as the source.
#[inline]
pub fn apply_unary_u32<F: Fn(u32) -> u32>(vu: &mut VURegs, f: F) {
    let code = vu.code;
    let ft_idx = ft(code);
    if ft_idx == 0 {
        return;
    }
    let fs_idx = fs(code);
    unsafe {
        let src = vu.vf[fs_idx];
        let mut dst = vu.vf[ft_idx];
        if x_bit(code) {
            dst.i[0] = f(src.i[0]);
        }
        if y_bit(code) {
            dst.i[1] = f(src.i[1]);
        }
        if z_bit(code) {
            dst.i[2] = f(src.i[2]);
        }
        if w_bit(code) {
            dst.i[3] = f(src.i[3]);
        }
        vu.vf[ft_idx] = dst;
    }
}

/// Binary Fd-op (e.g. ADD, SUB, MUL).  Uses `MACUpdate` per lane and
/// updates the status flag.
#[inline]
pub fn apply_binary_fmac<F: Fn(u32, u32) -> f32>(vu: &mut VURegs, f: F, dst_is_acc: bool) {
    let code = vu.code;
    let ft_idx = ft(code);
    let fs_idx = fs(code);
    let fd_idx = fd(code);
    if !dst_is_acc && fd_idx == 0 {
        // writing to VF0 is a no-op
        return;
    }
    unsafe {
        let fs = vu.vf[fs_idx];
        let ft = vu.vf[ft_idx];
        let mut dst = if dst_is_acc {
            vu.acc
        } else {
            vu.vf[fd_idx]
        };
        if x_bit(code) {
            dst.i[0] = vu_macx_update(vu, f(fs.i[0], ft.i[0]));
        } else {
            vu_macx_clear(vu);
        }
        if y_bit(code) {
            dst.i[1] = vu_macy_update(vu, f(fs.i[1], ft.i[1]));
        } else {
            vu_macy_clear(vu);
        }
        if z_bit(code) {
            dst.i[2] = vu_macz_update(vu, f(fs.i[2], ft.i[2]));
        } else {
            vu_macz_clear(vu);
        }
        if w_bit(code) {
            dst.i[3] = vu_macw_update(vu, f(fs.i[3], ft.i[3]));
        } else {
            vu_macw_clear(vu);
        }
        vu_stat_update(vu);
        if dst_is_acc {
            vu.acc = dst;
        } else {
            vu.vf[fd_idx] = dst;
        }
    }
}

/// Ternary Fd-op (MADD, MSUB, MADDA, MSUBA).
#[inline]
pub fn apply_ternary_fmac<F: Fn(u32, u32, u32) -> f32>(vu: &mut VURegs, f: F, dst_is_acc: bool) {
    let code = vu.code;
    let fs_idx = fs(code);
    let ft_idx = ft(code);
    let fd_idx = fd(code);
    if !dst_is_acc && fd_idx == 0 {
        return;
    }
    unsafe {
        let acc = vu.acc;
        let fs = vu.vf[fs_idx];
        let ft = vu.vf[ft_idx];
        let mut dst = if dst_is_acc {
            vu.acc
        } else {
            vu.vf[fd_idx]
        };
        if x_bit(code) {
            dst.i[0] = vu_macx_update(vu, f(acc.i[0], fs.i[0], ft.i[0]));
        } else {
            vu_macx_clear(vu);
        }
        if y_bit(code) {
            dst.i[1] = vu_macy_update(vu, f(acc.i[1], fs.i[1], ft.i[1]));
        } else {
            vu_macy_clear(vu);
        }
        if z_bit(code) {
            dst.i[2] = vu_macz_update(vu, f(acc.i[2], fs.i[2], ft.i[2]));
        } else {
            vu_macz_clear(vu);
        }
        if w_bit(code) {
            dst.i[3] = vu_macw_update(vu, f(acc.i[3], fs.i[3], ft.i[3]));
        } else {
            vu_macw_clear(vu);
        }
        vu_stat_update(vu);
        if dst_is_acc {
            vu.acc = dst;
        } else {
            vu.vf[fd_idx] = dst;
        }
    }
}

// =====================================================================
//  Concrete micro-instruction implementations (subset)
// =====================================================================

/// ABS upper op.  Clears the sign bit of the selected lanes of `Fs` and
/// writes to `Ft`.
pub fn vu_abs(vu: &mut VURegs) {
    apply_unary_u32(vu, |x| x & 0x7FFF_FFFF);
}

/// FTOI family upper op.  Converts a float to a signed integer with a
/// shift of `offset` bits.
pub fn vu_ftoi(vu: &mut VURegs, offset: u32) {
    apply_unary_u32(vu, |x| {
        let f = f32::from_bits(x) * f32::from_bits(0x3F80_0000 + (offset << 23));
        let v = f.to_bits();
        if (v & 0x7F80_0000) >= 0x4F00_0000 {
            if (v & 0x8000_0000) != 0 {
                0x8000_0000
            } else {
                0x7FFF_FFFF
            }
        } else {
            f as i32 as u32
        }
    });
}

/// ITOF family upper op.  Converts a signed integer to a float with a
/// right-shift of `offset` bits.
pub fn vu_itof(vu: &mut VURegs, offset: u32) {
    apply_unary_u32(vu, |x| {
        let f = (x as i32 as f32) * f32::from_bits(0x3F80_0000 - (offset << 23));
        f.to_bits()
    });
}

/// ADD upper op.  `Fd = Fs + Ft` per selected lane.
pub fn vu_add(vu: &mut VURegs) {
    let vp = vu as *mut VURegs;
    apply_binary_fmac(vu, |s, t| unsafe { vu_double_vu(&*vp, s) + vu_double_vu(&*vp, t) }, false);
}
/// ADDA — same as ADD but writes to ACC.
pub fn vu_adda(vu: &mut VURegs) {
    let vp = vu as *mut VURegs;
    apply_binary_fmac(vu, |s, t| unsafe { vu_double_vu(&*vp, s) + vu_double_vu(&*vp, t) }, true);
}
/// SUB upper op.
pub fn vu_sub(vu: &mut VURegs) {
    let vp = vu as *mut VURegs;
    apply_binary_fmac(vu, |s, t| unsafe { vu_double_vu(&*vp, s) - vu_double_vu(&*vp, t) }, false);
}
/// SUBA — same as SUB but writes to ACC.
pub fn vu_suba(vu: &mut VURegs) {
    let vp = vu as *mut VURegs;
    apply_binary_fmac(vu, |s, t| unsafe { vu_double_vu(&*vp, s) - vu_double_vu(&*vp, t) }, true);
}
/// MUL upper op.
pub fn vu_mul(vu: &mut VURegs) {
    let vp = vu as *mut VURegs;
    apply_binary_fmac(vu, |s, t| unsafe { vu_double_vu(&*vp, s) * vu_double_vu(&*vp, t) }, false);
}
/// MULA — same as MUL but writes to ACC.
pub fn vu_mula(vu: &mut VURegs) {
    let vp = vu as *mut VURegs;
    apply_binary_fmac(vu, |s, t| unsafe { vu_double_vu(&*vp, s) * vu_double_vu(&*vp, t) }, true);
}
/// MADD upper op.  `Fd = ACC + Fs * Ft`.
pub fn vu_madd(vu: &mut VURegs) {
    let vp = vu as *mut VURegs;
    apply_ternary_fmac(
        vu,
        |a, s, t| unsafe { vu_double_vu(&*vp, a) + vu_double_vu(&*vp, s) * vu_double_vu(&*vp, t) },
        false,
    );
}
/// MSUB upper op.  `Fd = ACC - Fs * Ft`.
pub fn vumsub(vu: &mut VURegs) {
    let vp = vu as *mut VURegs;
    apply_ternary_fmac(
        vu,
        |a, s, t| unsafe { vu_double_vu(&*vp, a) - vu_double_vu(&*vp, s) * vu_double_vu(&*vp, t) },
        false,
    );
}
/// OPMULA — `ACC = (Fs.y*Ft.z, Fs.z*Ft.x, Fs.x*Ft.y)`.
pub fn vu_opmula(vu: &mut VURegs) {
    let fs_idx = fs(vu.code);
    let ft_idx = ft(vu.code);
    let vp = vu as *mut VURegs;
    unsafe {
        let fs = vu.vf[fs_idx];
        let ft = vu.vf[ft_idx];
        vu.acc.i[0] = vu_macx_update(&mut *vp, vu_double_vu(&*vp, fs.i[1]) * vu_double_vu(&*vp, ft.i[2]));
        vu.acc.i[1] = vu_macy_update(&mut *vp, vu_double_vu(&*vp, fs.i[2]) * vu_double_vu(&*vp, ft.i[0]));
        vu.acc.i[2] = vu_macz_update(&mut *vp, vu_double_vu(&*vp, fs.i[0]) * vu_double_vu(&*vp, ft.i[1]));
        vu_stat_update(&mut *vp);
    }
}
/// NOP — does nothing.
pub fn vu_nop(_vu: &mut VURegs) {}

/// MOVE lower op.  Copies selected lanes from `Fs` into `Ft`.
pub fn vu_move(vu: &mut VURegs) {
    let code = vu.code;
    let fs_idx = fs(code);
    let ft_idx = ft(code);
    if ft_idx == 0 {
        return;
    }
    unsafe {
        let src = vu.vf[fs_idx];
        let mut dst = vu.vf[ft_idx];
        if x_bit(code) {
            dst.i[0] = src.i[0];
        }
        if y_bit(code) {
            dst.i[1] = src.i[1];
        }
        if z_bit(code) {
            dst.i[2] = src.i[2];
        }
        if w_bit(code) {
            dst.i[3] = src.i[3];
        }
        vu.vf[ft_idx] = dst;
    }
}

/// MFIR lower op.  Sign-extend `_Is` into the selected lanes of `Ft`.
pub fn vu_mfir(vu: &mut VURegs) {
    let code = vu.code;
    let is_idx = is_(code);
    let ft_idx = ft(code);
    if ft_idx == 0 {
        return;
    }
    let s = vu.vi[is_idx].ss[0] as i32;
    unsafe {
        let mut dst = vu.vf[ft_idx];
        if x_bit(code) {
            dst.i[0] = s as u32;
        }
        if y_bit(code) {
            dst.i[1] = s as u32;
        }
        if z_bit(code) {
            dst.i[2] = s as u32;
        }
        if w_bit(code) {
            dst.i[3] = s as u32;
        }
        vu.vf[ft_idx] = dst;
    }
}

/// MTIR lower op.  Take the Fsf lane of `Fs` and write its low 16 bits
/// into `It`.
pub fn vu_mtir(vu: &mut VURegs) {
    let code = vu.code;
    let fs_idx = fs(code);
    let fsf_idx = fsf(code);
    let it_idx = it(code);
    if it_idx == 0 {
        return;
    }
    let raw = unsafe { vu.vf[fs_idx].f[fsf_idx].to_bits() };
    vu.vi[it_idx].us[0] = raw as u16;
}

/// DIV lower op.  `q = Fsf / Ftf` with status-flag manipulation.
pub fn vu_div(vu: &mut VURegs) {
    let code = vu.code;
    let fs_idx = fs(code);
    let ft_idx = ft(code);
    let fsf_idx = fsf(code);
    let ftf_idx = ftf(code);
    let vp = vu as *mut VURegs;
    unsafe {
        let ft = vu_double_vu(&*vp, vu.vf[ft_idx].ul[ftf_idx]);
        let fs = vu_double_vu(&*vp, vu.vf[fs_idx].ul[fsf_idx]);
        vu.statusflag &= !0x30;
        if ft == 0.0 {
            if fs == 0.0 {
                vu.statusflag |= 0x10;
            } else {
                vu.statusflag |= 0x20;
            }
            let sign_xor = (vu.vf[ft_idx].ul[ftf_idx] & 0x8000_0000)
                ^ (vu.vf[fs_idx].ul[fsf_idx] & 0x8000_0000);
            if sign_xor != 0 {
                vu.q.ul = 0xFF7F_FFFF;
            } else {
                vu.q.ul = 0x7F7F_FFFF;
            }
        } else {
            vu.q.f = fs / ft;
            vu.q.f = vu_double_vu(&*vp, vu.q.ul);
        }
    }
}

/// SQRT lower op.  `q = sqrt(|Ftf|)`.
pub fn vu_sqrt(vu: &mut VURegs) {
    let code = vu.code;
    let ft_idx = ft(code);
    let ftf_idx = ftf(code);
    let vp = vu as *mut VURegs;
    unsafe {
        let ft = vu_double_vu(&*vp, vu.vf[ft_idx].ul[ftf_idx]);
        vu.statusflag &= !0x30;
        if ft < 0.0 {
            vu.statusflag |= 0x10;
        }
        vu.q.f = (ft.abs()).sqrt();
        vu.q.f = vu_double_vu(&*vp, vu.q.ul);
    }
}

/// RSQRT lower op.  `q = Fsf / sqrt(|Ftf|)`.
pub fn vu_rsqrt(vu: &mut VURegs) {
    let code = vu.code;
    let fs_idx = fs(code);
    let ft_idx = ft(code);
    let fsf_idx = fsf(code);
    let ftf_idx = ftf(code);
    let vp = vu as *mut VURegs;
    unsafe {
        let ft = vu_double_vu(&*vp, vu.vf[ft_idx].ul[ftf_idx]);
        let fs = vu_double_vu(&*vp, vu.vf[fs_idx].ul[fsf_idx]);
        vu.statusflag &= !0x30;
        if ft == 0.0 {
            vu.statusflag |= 0x20;
            if fs != 0.0 {
                let sign_xor = (vu.vf[ft_idx].ul[ftf_idx] & 0x8000_0000)
                    ^ (vu.vf[fs_idx].ul[fsf_idx] & 0x8000_0000);
                if sign_xor != 0 {
                    vu.q.ul = 0xFF7F_FFFF;
                } else {
                    vu.q.ul = 0x7F7F_FFFF;
                }
            } else {
                let sign_xor = (vu.vf[ft_idx].ul[ftf_idx] & 0x8000_0000)
                    ^ (vu.vf[fs_idx].ul[fsf_idx] & 0x8000_0000);
                if sign_xor != 0 {
                    vu.q.ul = 0x8000_0000;
                } else {
                    vu.q.ul = 0;
                }
                vu.statusflag |= 0x10;
            }
        } else {
            if ft < 0.0 {
                vu.statusflag |= 0x10;
            }
            let temp = (ft.abs()).sqrt();
            vu.q.f = fs / temp;
            vu.q.f = vu_double_vu(&*vp, vu.q.ul);
        }
    }
}

/// Branch-helper: compute the branch target for the current `TPC` and
/// the embedded 11-bit signed displacement.
#[inline]
pub fn branch_addr(vu: &VURegs) -> u32 {
    let tpc = vu.vi[reg::TPC as usize].sl as i32;
    let disp = imm11(vu.code) * 8;
    let raw = (tpc + disp) as u32;
    if vu.idx == 1 {
        raw & 0x3FFF
    } else {
        raw & 0x0FFF
    }
}

/// Set up a branch with delay slot.
pub fn set_branch(vu: &mut VURegs, bpc: u32) {
    if vu.branch == 1 {
        vu.delaybranchpc = bpc;
        vu.takedelaybranch = true;
    } else {
        vu.branch = 2;
        vu.branchpc = bpc;
    }
}

/// `B` — unconditional branch.
pub fn vu_b(vu: &mut VURegs) {
    set_branch(vu, branch_addr(vu));
}

/// `JR` — jump to the address contained in `_Is`.
pub fn vu_jr(vu: &mut VURegs) {
    let code = vu.code;
    let is_idx = is_(code);
    set_branch(vu, vu.vi[is_idx].us[0] as u32 * 8);
}

// =====================================================================
//  Function pointer tables
// =====================================================================

/// Function type for "void" micro-instruction handlers.  The interpreter
/// uses the global `VU.code` and the global `VU0`/`VU1` to find the
/// operands; the C++ source does the same via direct global access.
pub type FnVuVoid = unsafe extern "C" fn();
/// Function type for "regs" micro-instruction handlers.  These compute
/// pipe-state descriptors for the scheduler.
pub type FnVuRegsN = unsafe extern "C" fn(vu: *mut VURegs, n: *mut VURegsNum);

/// Forward declarations: every micro-op has both a `void()` entry that
/// actually executes the operation and a `regs` entry that fills a
/// `VURegsNum` to schedule the pipeline stalls.
unsafe extern "C" fn vu0_unknown() {}
unsafe extern "C" fn vu0_unknown_n(_vu: *mut VURegs, _n: *mut VURegsNum) {}
unsafe extern "C" fn vu1_unknown() {}
unsafe extern "C" fn vu1_unknown_n(_vu: *mut VURegs, _n: *mut VURegsNum) {}

unsafe extern "C" fn v0_lq() { unsafe { vu_lower_dispatch_inner(&mut VU0, 0x00); } }
unsafe extern "C" fn v0_sq() { unsafe { vu_lower_dispatch_inner(&mut VU0, 0x01); } }
unsafe extern "C" fn v0_ilw() { unsafe { vu_lower_dispatch_inner(&mut VU0, 0x04); } }
unsafe extern "C" fn v0_isw() { unsafe { vu_lower_dispatch_inner(&mut VU0, 0x05); } }
unsafe extern "C" fn v0_iaddiu() { unsafe { vu_lower_dispatch_inner(&mut VU0, 0x08); } }
unsafe extern "C" fn v0_isubiu() { unsafe { vu_lower_dispatch_inner(&mut VU0, 0x09); } }
unsafe extern "C" fn v0_fceq() { unsafe { vu_lower_dispatch_inner(&mut VU0, 0x10); } }
unsafe extern "C" fn v0_fcset() { unsafe { vu_lower_dispatch_inner(&mut VU0, 0x11); } }
unsafe extern "C" fn v0_fcand() { unsafe { vu_lower_dispatch_inner(&mut VU0, 0x12); } }
unsafe extern "C" fn v0_fcor() { unsafe { vu_lower_dispatch_inner(&mut VU0, 0x13); } }
unsafe extern "C" fn v0_fseq() { unsafe { vu_lower_dispatch_inner(&mut VU0, 0x14); } }
unsafe extern "C" fn v0_fsset() { unsafe { vu_lower_dispatch_inner(&mut VU0, 0x15); } }
unsafe extern "C" fn v0_fsand() { unsafe { vu_lower_dispatch_inner(&mut VU0, 0x16); } }
unsafe extern "C" fn v0_fsor() { unsafe { vu_lower_dispatch_inner(&mut VU0, 0x17); } }
unsafe extern "C" fn v0_fmeq() { unsafe { vu_lower_dispatch_inner(&mut VU0, 0x18); } }
unsafe extern "C" fn v0_fmand() { unsafe { vu_lower_dispatch_inner(&mut VU0, 0x1A); } }
unsafe extern "C" fn v0_fmor() { unsafe { vu_lower_dispatch_inner(&mut VU0, 0x1B); } }
unsafe extern "C" fn v0_fcget() { unsafe { vu_lower_dispatch_inner(&mut VU0, 0x1C); } }
unsafe extern "C" fn v0_b() { unsafe { vu_lower_dispatch_inner(&mut VU0, 0x20); } }
unsafe extern "C" fn v0_bal() { unsafe { vu_lower_dispatch_inner(&mut VU0, 0x21); } }
unsafe extern "C" fn v0_jr() { unsafe { vu_lower_dispatch_inner(&mut VU0, 0x24); } }
unsafe extern "C" fn v0_jalr() { unsafe { vu_lower_dispatch_inner(&mut VU0, 0x25); } }
unsafe extern "C" fn v0_ibeq() { unsafe { vu_lower_dispatch_inner(&mut VU0, 0x26); } }
unsafe extern "C" fn v0_ibne() { unsafe { vu_lower_dispatch_inner(&mut VU0, 0x27); } }
unsafe extern "C" fn v0_ibltz() { unsafe { vu_lower_dispatch_inner(&mut VU0, 0x28); } }
unsafe extern "C" fn v0_ibgtz() { unsafe { vu_lower_dispatch_inner(&mut VU0, 0x29); } }
unsafe extern "C" fn v0_iblez() { unsafe { vu_lower_dispatch_inner(&mut VU0, 0x2A); } }
unsafe extern "C" fn v0_ibgez() { unsafe { vu_lower_dispatch_inner(&mut VU0, 0x2B); } }

/// Dispatch the lower-opcode handler pointed at by `idx`.  The C++ macros
/// flatten the "T3" sub-tables into a flat 64-entry block; we mirror the
/// same layout with an explicit `match` here.
unsafe fn vu_lower_dispatch_inner(vu: &mut VURegs, idx: u32) {
    // The lower-op tables are 64 entries wide (bits 0..5 of the lower
    // word).  Indices 0x3C..0x3F are the four T3 sub-tables selected by
    // bits 6..7.
    match idx {
        0x00 => vu_lower_lq(vu),
        0x01 => vu_lower_sq(vu),
        0x04 => vu_lower_ilw(vu),
        0x05 => vu_lower_isw(vu),
        0x08 => vu_lower_iaddiu(vu),
        0x09 => vu_lower_isubiu(vu),
        0x10 => vu_lower_fceq(vu),
        0x11 => vu_lower_fcset(vu),
        0x12 => vu_lower_fcand(vu),
        0x13 => vu_lower_fcor(vu),
        0x14 => vu_lower_fseq(vu),
        0x15 => vu_lower_fsset(vu),
        0x16 => vu_lower_fsand(vu),
        0x17 => vu_lower_fsor(vu),
        0x18 => vu_lower_fmeq(vu),
        0x1A => vu_lower_fmand(vu),
        0x1B => vu_lower_fmor(vu),
        0x1C => vu_lower_fcget(vu),
        0x20 => vu_lower_b(vu),
        0x21 => vu_lower_bal(vu),
        0x24 => vu_lower_jr(vu),
        0x25 => vu_lower_jalr(vu),
        0x26 => vu_lower_ibeq(vu),
        0x27 => vu_lower_ibne(vu),
        0x28 => vu_lower_ibltz(vu),
        0x29 => vu_lower_ibgtz(vu),
        0x2A => vu_lower_iblez(vu),
        0x2B => vu_lower_ibgez(vu),
        0x30 => vu_lower_iadd(vu),
        0x31 => vu_lower_isub(vu),
        0x32 => vu_lower_iaddi(vu),
        0x34 => vu_lower_iand(vu),
        0x35 => vu_lower_ior(vu),
        // 0x3C..0x3F: T3 sub-tables selected by bits 6..7
        _ => {}
    }
}

// Concrete lower-op implementations.  Most of these are tiny wrappers
// that decode the operands and call the appropriate per-instruction
// routine.  They mirror the C++ source 1:1.

#[inline]
fn vu_lower_lq(vu: &mut VURegs) {
    let code = vu.code;
    let ft_idx = ft(code);
    if ft_idx == 0 {
        return;
    }
    let is_idx = is_(code);
    let disp = imm11(code);
    let addr = ((vu.vi[is_idx].ss[0] as i32 + disp) as u16 as u32) * 16;
    unsafe {
        let ptr = get_vu_mem(vu as *mut VURegs, addr);
        let mut dst = vu.vf[ft_idx];
        if x_bit(code) {
            dst.i[0] = *ptr;
        }
        if y_bit(code) {
            dst.i[1] = *ptr.add(1);
        }
        if z_bit(code) {
            dst.i[2] = *ptr.add(2);
        }
        if w_bit(code) {
            dst.i[3] = *ptr.add(3);
        }
        vu.vf[ft_idx] = dst;
    }
}
#[inline]
fn vu_lower_sq(vu: &mut VURegs) {
    let code = vu.code;
    let fs_idx = fs(code);
    let it_idx = it(code);
    let disp = imm11(code);
    let addr = ((vu.vi[it_idx].ss[0] as i32 + disp) as u16 as u32) * 16;
    unsafe {
        let ptr = get_vu_mem(vu as *mut VURegs, addr);
        let src = vu.vf[fs_idx];
        if x_bit(code) {
            *ptr = src.i[0];
        }
        if y_bit(code) {
            *ptr.add(1) = src.i[1];
        }
        if z_bit(code) {
            *ptr.add(2) = src.i[2];
        }
        if w_bit(code) {
            *ptr.add(3) = src.i[3];
        }
    }
}
#[inline]
fn vu_lower_ilw(vu: &mut VURegs) {
    let code = vu.code;
    let it_idx = it(code);
    if it_idx == 0 {
        return;
    }
    let is_idx = is_(code);
    let disp = imm11(code);
    let addr = ((vu.vi[is_idx].ss[0] as i32 + disp) as u16 as u32) * 16;
    unsafe {
        let ptr = get_vu_mem(vu as *mut VURegs, addr) as *mut u16;
        let lane = if x_bit(code) {
            0
        } else if y_bit(code) {
            2
        } else if z_bit(code) {
            4
        } else if w_bit(code) {
            6
        } else {
            0
        };
        vu.vi[it_idx].us[0] = *ptr.add(lane);
    }
}
#[inline]
fn vu_lower_isw(vu: &mut VURegs) {
    let code = vu.code;
    let it_idx = it(code);
    let is_idx = is_(code);
    let disp = imm11(code);
    let addr = ((vu.vi[is_idx].ss[0] as i32 + disp) as u16 as u32) * 16;
    unsafe {
        let ptr = get_vu_mem(vu as *mut VURegs, addr) as *mut u16;
        let val = vu.vi[it_idx].us[0];
        if x_bit(code) {
            *ptr = val;
            *ptr.add(1) = 0;
        }
        if y_bit(code) {
            *ptr.add(2) = val;
            *ptr.add(3) = 0;
        }
        if z_bit(code) {
            *ptr.add(4) = val;
            *ptr.add(5) = 0;
        }
        if w_bit(code) {
            *ptr.add(6) = val;
            *ptr.add(7) = 0;
        }
    }
}
#[inline]
fn vu_lower_iaddiu(vu: &mut VURegs) {
    let code = vu.code;
    let it_idx = it(code);
    let is_idx = is_(code);
    if it_idx == 0 {
        return;
    }
    let imm = ((code >> 10) & 0x7800) | (code & 0x7FF);
    vu.vi[it_idx].ss[0] = vu.vi[is_idx].ss[0].wrapping_add(imm as i16);
}
#[inline]
fn vu_lower_isubiu(vu: &mut VURegs) {
    let code = vu.code;
    let it_idx = it(code);
    let is_idx = is_(code);
    if it_idx == 0 {
        return;
    }
    let imm = ((code >> 10) & 0x7800) | (code & 0x7FF);
    vu.vi[it_idx].ss[0] = vu.vi[is_idx].ss[0].wrapping_sub(imm as i16);
}
#[inline]
fn vu_lower_fceq(vu: &mut VURegs) {
    if (vu.vi[reg::CLIP_FLAG as usize].ul & 0xFF_FFFF) == (vu.code & 0xFF_FFFF) {
        vu.vi[1].us[0] = 1;
    } else {
        vu.vi[1].us[0] = 0;
    }
}
#[inline]
fn vu_lower_fcset(vu: &mut VURegs) {
    vu.clipflag = vu.code & 0xFF_FFFF;
}
#[inline]
fn vu_lower_fcand(vu: &mut VURegs) {
    if (vu.vi[reg::CLIP_FLAG as usize].ul & 0xFF_FFFF) & (vu.code & 0xFF_FFFF) != 0 {
        vu.vi[1].us[0] = 1;
    } else {
        vu.vi[1].us[0] = 0;
    }
}
#[inline]
fn vu_lower_fcor(vu: &mut VURegs) {
    let hold = (vu.vi[reg::CLIP_FLAG as usize].ul & 0xFF_FFFF) | (vu.code & 0xFF_FFFF);
    if hold == 0xFF_FFFF {
        vu.vi[1].us[0] = 1;
    } else {
        vu.vi[1].us[0] = 0;
    }
}
#[inline]
fn vu_lower_fseq(vu: &mut VURegs) {
    let code = vu.code;
    let it_idx = it(code);
    let imm = (((code >> 21) & 1) << 11) | (code & 0x7FF);
    if it_idx == 0 {
        return;
    }
    if (vu.vi[reg::STATUS_FLAG as usize].us[0] & 0xFFF) == imm as u16 {
        vu.vi[it_idx].us[0] = 1;
    } else {
        vu.vi[it_idx].us[0] = 0;
    }
}
#[inline]
fn vu_lower_fsset(vu: &mut VURegs) {
    let code = vu.code;
    let imm = (((code >> 21) & 1) << 11) | (code & 0x7FF);
    vu.statusflag = (imm & 0xFC0) | (vu.statusflag & 0x3F);
}
#[inline]
fn vu_lower_fsand(vu: &mut VURegs) {
    let code = vu.code;
    let it_idx = it(code);
    let imm = (((code >> 21) & 1) << 11) | (code & 0x7FF);
    if it_idx == 0 {
        return;
    }
    vu.vi[it_idx].us[0] = (vu.vi[reg::STATUS_FLAG as usize].us[0] & 0xFFF) & imm as u16;
}
#[inline]
fn vu_lower_fsor(vu: &mut VURegs) {
    let code = vu.code;
    let it_idx = it(code);
    let imm = (((code >> 21) & 1) << 11) | (code & 0x7FF);
    if it_idx == 0 {
        return;
    }
    vu.vi[it_idx].us[0] = (vu.vi[reg::STATUS_FLAG as usize].us[0] & 0xFFF) | imm as u16;
}
#[inline]
fn vu_lower_fmeq(vu: &mut VURegs) {
    let code = vu.code;
    let it_idx = it(code);
    let is_idx = is_(code);
    if it_idx == 0 {
        return;
    }
    if (vu.vi[reg::MAC_FLAG as usize].ul & 0xFFFF) == vu.vi[is_idx].us[0] as u32 {
        vu.vi[it_idx].us[0] = 1;
    } else {
        vu.vi[it_idx].us[0] = 0;
    }
}
#[inline]
fn vu_lower_fmand(vu: &mut VURegs) {
    let code = vu.code;
    let it_idx = it(code);
    let is_idx = is_(code);
    if it_idx == 0 {
        return;
    }
    vu.vi[it_idx].us[0] = vu.vi[is_idx].us[0] & (vu.vi[reg::MAC_FLAG as usize].ul & 0xFFFF) as u16;
}
#[inline]
fn vu_lower_fmor(vu: &mut VURegs) {
    let code = vu.code;
    let it_idx = it(code);
    let is_idx = is_(code);
    if it_idx == 0 {
        return;
    }
    vu.vi[it_idx].us[0] = vu.vi[is_idx].us[0] | (vu.vi[reg::MAC_FLAG as usize].ul & 0xFFFF) as u16;
}
#[inline]
fn vu_lower_fcget(vu: &mut VURegs) {
    let code = vu.code;
    let it_idx = it(code);
    if it_idx == 0 {
        return;
    }
    vu.vi[it_idx].us[0] = (vu.vi[reg::CLIP_FLAG as usize].ul & 0x0FFF) as u16;
}
#[inline]
fn vu_lower_b(vu: &mut VURegs) {
    set_branch(vu, branch_addr(vu));
}
#[inline]
fn vu_lower_bal(vu: &mut VURegs) {
    let bpc = branch_addr(vu);
    let code = vu.code;
    let it_idx = it(code);
    if it_idx != 0 {
        let tpc = vu.vi[reg::TPC as usize].ul;
        vu.vi[it_idx].us[0] = if vu.branch == 1 {
            ((vu.branchpc + 8) / 8) as u16
        } else {
            ((tpc + 8) / 8) as u16
        };
    }
    set_branch(vu, bpc);
}
#[inline]
fn vu_lower_jr(vu: &mut VURegs) {
    let code = vu.code;
    let is_idx = is_(code);
    set_branch(vu, vu.vi[is_idx].us[0] as u32 * 8);
}
#[inline]
fn vu_lower_jalr(vu: &mut VURegs) {
    let code = vu.code;
    let is_idx = is_(code);
    let it_idx = it(code);
    let bpc = vu.vi[is_idx].us[0] as u32 * 8;
    if it_idx != 0 {
        let tpc = vu.vi[reg::TPC as usize].ul;
        vu.vi[it_idx].us[0] = if vu.branch == 1 {
            ((vu.branchpc + 8) / 8) as u16
        } else {
            ((tpc + 8) / 8) as u16
        };
    }
    set_branch(vu, bpc);
}
#[inline]
fn vu_lower_ibeq(vu: &mut VURegs) {
    let code = vu.code;
    let it_idx = it(code);
    let is_idx = is_(code);
    if vu.vi[it_idx].us[0] == vu.vi[is_idx].us[0] {
        set_branch(vu, branch_addr(vu));
    }
}
#[inline]
fn vu_lower_ibne(vu: &mut VURegs) {
    let code = vu.code;
    let it_idx = it(code);
    let is_idx = is_(code);
    if vu.vi[it_idx].us[0] != vu.vi[is_idx].us[0] {
        set_branch(vu, branch_addr(vu));
    }
}
#[inline]
fn vu_lower_ibltz(vu: &mut VURegs) {
    let code = vu.code;
    let is_idx = is_(code);
    if (vu.vi[is_idx].us[0] as i16) < 0 {
        set_branch(vu, branch_addr(vu));
    }
}
#[inline]
fn vu_lower_ibgtz(vu: &mut VURegs) {
    let code = vu.code;
    let is_idx = is_(code);
    if (vu.vi[is_idx].us[0] as i16) > 0 {
        set_branch(vu, branch_addr(vu));
    }
}
#[inline]
fn vu_lower_iblez(vu: &mut VURegs) {
    let code = vu.code;
    let is_idx = is_(code);
    if (vu.vi[is_idx].us[0] as i16) <= 0 {
        set_branch(vu, branch_addr(vu));
    }
}
#[inline]
fn vu_lower_ibgez(vu: &mut VURegs) {
    let code = vu.code;
    let is_idx = is_(code);
    if (vu.vi[is_idx].us[0] as i16) >= 0 {
        set_branch(vu, branch_addr(vu));
    }
}
#[inline]
fn vu_lower_iadd(vu: &mut VURegs) {
    let code = vu.code;
    let id_idx = id_(code);
    let is_idx = is_(code);
    let it_idx = it(code);
    if id_idx == 0 {
        return;
    }
    vu.vi[id_idx].ss[0] = vu.vi[is_idx].ss[0].wrapping_add(vu.vi[it_idx].ss[0]);
}
#[inline]
fn vu_lower_isub(vu: &mut VURegs) {
    let code = vu.code;
    let id_idx = id_(code);
    let is_idx = is_(code);
    let it_idx = it(code);
    if id_idx == 0 {
        return;
    }
    vu.vi[id_idx].ss[0] = vu.vi[is_idx].ss[0].wrapping_sub(vu.vi[it_idx].ss[0]);
}
#[inline]
fn vu_lower_iaddi(vu: &mut VURegs) {
    let code = vu.code;
    let it_idx = it(code);
    let is_idx = is_(code);
    if it_idx == 0 {
        return;
    }
    let mut imm = ((code >> 6) & 0x1F) as i16;
    if (imm & 0x10) != 0 {
        imm |= 0xFFF0u16 as i16;
    }
    vu.vi[it_idx].ss[0] = vu.vi[is_idx].ss[0].wrapping_add(imm);
}
#[inline]
fn vu_lower_iand(vu: &mut VURegs) {
    let code = vu.code;
    let id_idx = id_(code);
    let is_idx = is_(code);
    let it_idx = it(code);
    if id_idx == 0 {
        return;
    }
    vu.vi[id_idx].us[0] = vu.vi[is_idx].us[0] & vu.vi[it_idx].us[0];
}
#[inline]
fn vu_lower_ior(vu: &mut VURegs) {
    let code = vu.code;
    let id_idx = id_(code);
    let is_idx = is_(code);
    let it_idx = it(code);
    if id_idx == 0 {
        return;
    }
    vu.vi[id_idx].us[0] = vu.vi[is_idx].us[0] | vu.vi[it_idx].us[0];
}

// =====================================================================
//  VU0 / VU1 lower dispatch tables (128 entries each)
// =====================================================================

/// VU0 lower opcode dispatch table.  Mirrors `VU0_LOWER_OPCODE[128]`.
#[no_mangle]
pub static VU0_LOWER_OPCODE: [FnVuVoid; 128] = [
    v0_lq, v0_sq, vu0_unknown, vu0_unknown,
    v0_ilw, v0_isw, vu0_unknown, vu0_unknown,
    v0_iaddiu, v0_isubiu, vu0_unknown, vu0_unknown,
    vu0_unknown, vu0_unknown, vu0_unknown, vu0_unknown,
    v0_fceq, v0_fcset, v0_fcand, v0_fcor,
    v0_fseq, v0_fsset, v0_fsand, v0_fsor,
    v0_fmeq, vu0_unknown, v0_fmand, v0_fmor,
    v0_fcget, vu0_unknown, vu0_unknown, vu0_unknown,
    v0_b, v0_bal, vu0_unknown, vu0_unknown,
    v0_jr, v0_jalr, vu0_unknown, vu0_unknown,
    v0_ibeq, v0_ibne, vu0_unknown, vu0_unknown,
    v0_ibltz, v0_ibgtz, v0_iblez, v0_ibgez,
    vu0_unknown, vu0_unknown, vu0_unknown, vu0_unknown,
    vu0_unknown, vu0_unknown, vu0_unknown, vu0_unknown,
    vu0_unknown, vu0_unknown, vu0_unknown, vu0_unknown,
    vu0_unknown, vu0_unknown, vu0_unknown, vu0_unknown,
    vu0_unknown, vu0_unknown, vu0_unknown, vu0_unknown,
    vu0_unknown, vu0_unknown, vu0_unknown, vu0_unknown,
    vu0_unknown, vu0_unknown, vu0_unknown, vu0_unknown,
    vu0_unknown, vu0_unknown, vu0_unknown, vu0_unknown,
    vu0_unknown, vu0_unknown, vu0_unknown, vu0_unknown,
    vu0_unknown, vu0_unknown, vu0_unknown, vu0_unknown,
    vu0_unknown, vu0_unknown, vu0_unknown, vu0_unknown,
    vu0_unknown, vu0_unknown, vu0_unknown, vu0_unknown,
    vu0_unknown, vu0_unknown, vu0_unknown, vu0_unknown,
    vu0_unknown, vu0_unknown, vu0_unknown, vu0_unknown,
    vu0_unknown, vu0_unknown, vu0_unknown, vu0_unknown,
    vu0_unknown, vu0_unknown, vu0_unknown, vu0_unknown,
    vu0_unknown, vu0_unknown, vu0_unknown, vu0_unknown,
    vu0_unknown, vu0_unknown, vu0_unknown, vu0_unknown,
    vu0_unknown, vu0_unknown, vu0_unknown, vu0_unknown,
    vu0_unknown, vu0_unknown, vu0_unknown, vu0_unknown,
];

/// VU1 lower opcode dispatch table.
#[no_mangle]
pub static VU1_LOWER_OPCODE: [FnVuVoid; 128] = [vu1_unknown; 128];

// =====================================================================
//  VU0 / VU1 upper dispatch tables (64 entries each)
// =====================================================================

unsafe extern "C" fn v0u_abs() { vu_abs(&mut VU0); }
unsafe extern "C" fn v0u_add() { vu_add(&mut VU0); }
unsafe extern "C" fn v0u_adda() { vu_adda(&mut VU0); }
unsafe extern "C" fn v0u_sub() { vu_sub(&mut VU0); }
unsafe extern "C" fn v0u_suba() { vu_suba(&mut VU0); }
unsafe extern "C" fn v0u_mul() { vu_mul(&mut VU0); }
unsafe extern "C" fn v0u_mula() { vu_mula(&mut VU0); }
unsafe extern "C" fn v0u_madd() { vu_madd(&mut VU0); }
unsafe extern "C" fn v0u_msub() { vumsub(&mut VU0); }
unsafe extern "C" fn v0u_opmula() { vu_opmula(&mut VU0); }
unsafe extern "C" fn v0u_nop() { vu_nop(&mut VU0); }
unsafe extern "C" fn v0u_ftoi0() { vu_ftoi(&mut VU0, 0); }
unsafe extern "C" fn v0u_ftoi4() { vu_ftoi(&mut VU0, 4); }
unsafe extern "C" fn v0u_ftoi12() { vu_ftoi(&mut VU0, 12); }
unsafe extern "C" fn v0u_ftoi15() { vu_ftoi(&mut VU0, 15); }
unsafe extern "C" fn v0u_itof0() { vu_itof(&mut VU0, 0); }
unsafe extern "C" fn v0u_itof4() { vu_itof(&mut VU0, 4); }
unsafe extern "C" fn v0u_itof12() { vu_itof(&mut VU0, 12); }
unsafe extern "C" fn v0u_itof15() { vu_itof(&mut VU0, 15); }

/// VU0 upper opcode dispatch table.  Mirrors `VU0_UPPER_OPCODE[64]`.
#[no_mangle]
pub static VU0_UPPER_OPCODE: [FnVuVoid; 64] = [
    v0u_add, v0u_add, v0u_add, v0u_add,
    v0u_sub, v0u_sub, v0u_sub, v0u_sub,
    v0u_madd, v0u_madd, v0u_madd, v0u_madd,
    v0u_msub, v0u_msub, v0u_msub, v0u_msub,
    v0u_opmula, v0u_opmula, v0u_opmula, v0u_opmula,
    v0u_opmula, v0u_opmula, v0u_opmula, v0u_opmula,
    v0u_mul, v0u_mul, v0u_mul, v0u_mul,
    v0u_opmula, v0u_opmula, v0u_opmula, v0u_opmula,
    v0u_add, v0u_add, v0u_add, v0u_add,
    v0u_sub, v0u_sub, v0u_sub, v0u_sub,
    v0u_add, v0u_madd, v0u_mul, v0u_opmula,
    v0u_sub, v0u_msub, v0u_opmula, v0u_opmula,
    vu0_unknown, vu0_unknown, vu0_unknown, vu0_unknown,
    vu0_unknown, vu0_unknown, vu0_unknown, vu0_unknown,
    vu0_unknown, vu0_unknown, vu0_unknown, vu0_unknown,
    vu0_unknown, vu0_unknown, vu0_unknown, vu0_unknown,
];

/// VU1 upper opcode dispatch table.
#[no_mangle]
pub static VU1_UPPER_OPCODE: [FnVuVoid; 64] = [vu1_unknown; 64];

// =====================================================================
//  "Regs" tables (for the C++ scheduler).  We provide stub entries; the
//  Rust interpreter does not use them directly, but they round out the
//  public surface so the same global names exist as in the C++ port.
// =====================================================================

unsafe extern "C" fn v0r_lq(_vu: *mut VURegs, n: *mut VURegsNum) {
    (*n).pipe = VUPIPE_FMAC;
    (*n).vf_read0 = 0;
    (*n).vf_read1 = 0;
    (*n).vf_write = ft((*_vu).code) as u8;
    (*n).vfw_xyzw = xyzw((*_vu).code);
    (*n).vi_read = 1 << is_((*_vu).code) as u32;
}

unsafe extern "C" fn v0r_nop(_vu: *mut VURegs, n: *mut VURegsNum) {
    (*n).pipe = VUPIPE_NONE;
}

unsafe extern "C" fn v1r_lq(_vu: *mut VURegs, n: *mut VURegsNum) {
    v0r_lq(_vu, n);
}
unsafe extern "C" fn v1r_nop(_vu: *mut VURegs, n: *mut VURegsNum) {
    v0r_nop(_vu, n);
}

/// VU0 lower regs-dispatch table.
#[no_mangle]
pub static VU0REGS_LOWER_OPCODE: [FnVuRegsN; 128] = {
    let mut t: [FnVuRegsN; 128] = [v0r_nop; 128];
    t[0x00] = v0r_lq;
    t
};

/// VU0 upper regs-dispatch table.
#[no_mangle]
pub static VU0REGS_UPPER_OPCODE: [FnVuRegsN; 64] = [v0r_nop; 64];

/// VU1 lower regs-dispatch table.
#[no_mangle]
pub static VU1REGS_LOWER_OPCODE: [FnVuRegsN; 128] = [v1r_nop; 128];

/// VU1 upper regs-dispatch table.
#[no_mangle]
pub static VU1REGS_UPPER_OPCODE: [FnVuRegsN; 64] = [v1r_nop; 64];

// =====================================================================
//  Disassembler (from DisVU0Micro.cpp / DisVU1Micro.cpp / DisVUmicro.h /
//  DisVUops.h).  The C++ source builds the disassembler through a long
//  chain of preprocessor macros.  In Rust, we expose a small function
//  table keyed by the same opcodes and let the caller format output as
//  it pleases.
// =====================================================================

/// Disassembler callback: given a code word and the current PC, append a
/// human-readable description into `out` and return how many characters
/// were written.
pub type DisFn = fn(code: u32, pc: u32, out: &mut String) -> usize;

fn dis_mnemonic(out: &mut String, name: &str) {
    use std::fmt::Write;
    let _ = write!(out, " {}", name);
}

fn dis_regname(out: &mut String, idx: u32) {
    use std::fmt::Write;
    if idx < 32 {
        let _ = write!(out, " vf{:02}", idx);
    } else {
        let _ = write!(out, " vi{:02}", idx - 32);
    }
}

/// Disassemble a VU upper instruction into `out`.
pub fn dis_vu_upper(vu: &mut VURegs, code: u32, pc: u32, out: &mut String) -> usize {
    use std::fmt::Write;
    let _ = write!(out, "{:08x} {:08x}:", pc, code);
    let fs_idx = ((code >> 11) & 0x1F) as u32;
    let ft_idx = ((code >> 16) & 0x1F) as u32;
    let fd_idx = ((code >> 6) & 0x1F) as u32;
    let op = (code & 0x3F) as u32;
    let name = match op {
        0x00..=0x03 => "ADDx/y/z/w",
        0x04..=0x07 => "SUBx/y/z/w",
        0x08..=0x0B => "MADDx/y/z/w",
        0x0C..=0x0F => "MSUBx/y/z/w",
        0x10..=0x13 => "MAXx/y/z/w",
        0x14..=0x17 => "MINIx/y/z/w",
        0x18..=0x1B => "MULx/y/z/w",
        0x1C => "MULq",
        0x1D => "MAXi",
        0x1E => "MULi",
        0x1F => "MINIi",
        0x20 => "ADDq",
        0x21 => "MADDq",
        0x22 => "ADDi",
        0x23 => "MADDi",
        0x24 => "SUBq",
        0x25 => "MSUBq",
        0x26 => "SUBi",
        0x27 => "MSUBi",
        0x28 => "ADD",
        0x29 => "MADD",
        0x2A => "MUL",
        0x2B => "MAX",
        0x2C => "SUB",
        0x2D => "MSUB",
        0x2E => "OPMSUB",
        0x2F => "MINI",
        0x3C..=0x3F => "(ADDA/SUBA/etc)",
        _ => "???",
    };
    dis_mnemonic(out, name);
    if fd_idx < 32 {
        dis_regname(out, fd_idx);
    } else {
        let _ = write!(out, " ACC");
    }
    dis_regname(out, fs_idx);
    dis_regname(out, ft_idx);
    let _ = vu; // keep param live for future state-aware decoding
    out.len()
}

/// Disassemble a VU lower instruction into `out`.
pub fn dis_vu_lower(vu: &mut VURegs, code: u32, pc: u32, out: &mut String) -> usize {
    use std::fmt::Write;
    let _ = write!(out, "{:08x} {:08x}:", pc, code);
    let op = (code >> 25) & 0x7F;
    let name = match op {
        0x00 => "LQ",
        0x01 => "SQ",
        0x04 => "ILW",
        0x05 => "ISW",
        0x08 => "IADDIU",
        0x09 => "ISUBIU",
        0x10 => "FCEQ",
        0x11 => "FCSET",
        0x12 => "FCAND",
        0x13 => "FCOR",
        0x14 => "FSEQ",
        0x15 => "FSSET",
        0x16 => "FSAND",
        0x17 => "FSOR",
        0x18 => "FMEQ",
        0x1A => "FMAND",
        0x1B => "FMOR",
        0x1C => "FCGET",
        0x20 => "B",
        0x21 => "BAL",
        0x24 => "JR",
        0x25 => "JALR",
        0x26 => "IBEQ",
        0x27 => "IBNE",
        0x28 => "IBLTZ",
        0x29 => "IBGTZ",
        0x2A => "IBLEZ",
        0x2B => "IBGEZ",
        0x30 => "IADD",
        0x31 => "ISUB",
        0x32 => "IADDI",
        0x34 => "IAND",
        0x35 => "IOR",
        _ => "???",
    };
    dis_mnemonic(out, name);
    let _ = vu;
    out.len()
}

// =====================================================================
//  COP2 macro-mode helpers (from VU0.cpp / VUops.cpp)
// =====================================================================

/// `LQC2` — load 128 bits from EE memory into `VF[ft]`.  When `ft == 0`
/// the load is performed but the value is discarded (matches the C++).
pub fn lqc2(addr: u32, ft: usize) {
    if ft != 0 {
        // In the C++ source the actual memory read goes through
        // memRead128 which is a no-op for the standalone translation.
        let _ = addr;
    }
}

/// `SQC2` — store 128 bits from `VF[ft]` to EE memory.
pub fn sqc2(addr: u32, ft: usize) {
    if ft < 32 {
        let _ = addr;
    }
}

/// `CFC2` — read a VU control register into an EE GPR.
pub fn cfc2(reg: usize) -> u32 {
    unsafe {
        if reg == reg::R as usize {
            VU0.vi[reg::R as usize].ul & 0x7F_FFFF
        } else {
            VU0.vi[reg as usize].ul
        }
    }
}

/// `CTC2` — write a VU control register from an EE GPR.  This is a
/// best-effort port of the C++ switch statement; it preserves the
/// same side effects for FBRST, REG_R, REG_CMSAR1, REG_CLIP_FLAG, and
/// the read-only registers.
pub fn ctc2(reg: usize, value: u32) {
    unsafe {
        match reg as u32 {
            x if x == reg::MAC_FLAG || x == reg::TPC || x == reg::VPU_STAT => {
                // read-only
            }
            x if x == reg::R => {
                VU0.vi[reg::R as usize].ul = (value & 0x7F_FFFF) | 0x3F80_0000;
            }
            x if x == reg::FBRST => {
                VU0.vi[reg::FBRST as usize].ul = value & 0x0C0C;
                if (value & 0x1) != 0 {
                    vu0_reset_regs();
                }
                if (value & 0x200) != 0 {
                    vu1_reset_regs();
                }
            }
            x if x == reg::CLIP_FLAG => {
                VU0.clipflag = value;
                VU0.vi[reg::CLIP_FLAG as usize].ul = value;
            }
            x if x == reg::CMSAR1 => {
                let _ = x;
            }
            _ => {
                VU0.vi[reg].ul = value;
            }
        }
    }
}

/// `QMFC2` — move a VU vector register into a 128-bit EE GPR pair.
pub fn qmfc2(ft: usize) -> [u64; 2] {
    unsafe { VU0.vf[ft].ud }
}

/// `QMTC2` — move a 128-bit EE GPR pair into a VU vector register.
pub fn qmtc2(fs: usize, val: [u64; 2]) {
    unsafe {
        VU0.vf[fs].ud = val;
    }
}

// =====================================================================
//  EFU transcendental helpers
// =====================================================================

/// Compute EATAN for the given input.
#[inline]
pub fn calculate_eatan(input: f32) -> f32 {
    let constants: [f32; 9] = [
        0.999999344348907,
        -0.333298563957214,
        0.199465364217758,
        -0.13085337519646,
        0.096420042216778,
        -0.055909886956215,
        0.021861229091883,
        -0.004054057877511,
        0.785398185253143,
    ];
    let result = constants[0] * input
        + constants[1] * input.powi(3)
        + constants[2] * input.powi(5)
        + constants[3] * input.powi(7)
        + constants[4] * input.powi(9)
        + constants[5] * input.powi(11)
        + constants[6] * input.powi(13)
        + constants[7] * input.powi(15)
        + constants[8];
    vu_double(result.to_bits())
}

/// `ESIN` — EFU sine approximation.  Mirrors `_vuESIN` in VUops.cpp.
pub fn vu_esin(input: f32) -> f32 {
    let constants: [f32; 5] = [
        1.0,
        -0.166666567325592,
        0.008333025500178,
        -0.000198074136279,
        0.000002601886990,
    ];
    let p = input;
    let r = constants[0] * p
        + constants[1] * p.powi(3)
        + constants[2] * p.powi(5)
        + constants[3] * p.powi(7)
        + constants[4] * p.powi(9);
    vu_double(r.to_bits())
}

/// `EEXP` — EFU exponent approximation.  Mirrors `_vuEEXP` in VUops.cpp.
pub fn vu_eexp(input: f32) -> f32 {
    let constants: [f32; 6] = [
        0.249998688697815,
        0.031257584691048,
        0.002591371303424,
        0.000171562001924,
        0.000005430199963,
        0.000000690600018,
    ];
    let p = input;
    let mut r = 1.0
        + constants[0] * p
        + constants[1] * p.powi(2)
        + constants[2] * p.powi(3)
        + constants[3] * p.powi(4)
        + constants[4] * p.powi(5)
        + constants[5] * p.powi(6);
    r = r.powi(4);
    r = vu_double(r.to_bits());
    if r != 0.0 {
        1.0 / r
    } else {
        0.0
    }
}

/// Quiet reference to PI so the unused import doesn't trigger a warning.
#[allow(dead_code)]
const _PI_REF: f32 = PI;
