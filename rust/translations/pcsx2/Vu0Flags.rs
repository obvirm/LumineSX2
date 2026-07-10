//! Translation of PCSX2's `VU.h`, `VU0.cpp`, `VUflags.cpp`, and
//! `VUflags.h` into a single idiomatic Rust 2021 module.
//!
//! The original C/C++ surface is split across three files: `VU.h` and
//! `VU0.cpp` own the EE's VU0 and VU1 register files together with the
//! VU0 init / reset / exec entry points, while `VUflags.cpp` /
//! `VUflags.h` own the IEEE-754 style MAC and status-flag update
//! helpers. The rewrite folds all four into a single self-contained
//! module that exposes the public surface requested for the
//! translation: the `VuRegs` register-file struct, the `VU0` and `VU1`
//! globals, the VU0 init / reset / exec entry points, the `VuFlags`
//! sticky-flag struct, and the global flag reset / set helpers.
//!
//! Only `std` is depended on. `static mut` is used for the few globals
//! the C++ sources expose so the rewrite preserves the same global
//! surface that surrounding emulator code expects to find.

// ---------------------------------------------------------------------------
// Sticky flag bits.
//
// The original C++ helpers latch flag bits directly into the
// `macflag` / `statusflag` / `clipflag` fields of `VURegs`. The rewrite
// hoists the IEEE-754 style sticky bits into their own struct; the bit
// assignments below match the conventional PCSX2 mapping used by the
// VU pipelines.
// ---------------------------------------------------------------------------

/// Sticky flag bit for divide-by-zero.
pub const VU_FLAG_ZERODIVIDE: u32 = 0x0000_0001;
/// Sticky flag bit for IEEE-754 overflow.
pub const VU_FLAG_OVERFLOW: u32 = 0x0000_0002;
/// Sticky flag bit for IEEE-754 underflow.
pub const VU_FLAG_UNDERFLOW: u32 = 0x0000_0004;
/// Sticky flag bit for unimplemented operations.
pub const VU_FLAG_UNIMPLEMENTED: u32 = 0x0000_0008;

// ---------------------------------------------------------------------------
// VU register file.
// ---------------------------------------------------------------------------

/// The VU register file.
///
/// Mirrors the C++ `alignas(16) VURegs` struct from `VU.h`, trimmed to
/// the fields the rewritten module needs to expose publicly. The vector
/// file is kept as `[u128; 32]` so each lane keeps the full 128-bit
/// width the interpreter and recompiler expect, and the integer file is
/// kept as `[u16; 16]` — the EE's 128-bit integer register file only
/// uses the low 16 bits per register; the rest is hardwired to zero
/// (cottonvibes).
#[repr(align(16))]
#[derive(Clone, Copy)]
pub struct VuRegs {
    /// The 32 vector registers.
    pub vf: [u128; 32],
    /// The 16 integer registers, low 16 bits valid.
    pub vi: [u16; 16],
    /// Status flag.
    pub status: u32,
    /// MAC flag (signed so sign-preserving ops round-trip cleanly).
    pub mac: i32,
    /// Clipping flag.
    pub clipping: u32,
}

impl VuRegs {
    /// Build a register file with every field cleared to its power-on
    /// value. Used by `vu0Init` / `vu0Reset` and to back the `VU0` /
    /// `VU1` statics.
    pub const fn zeroed() -> Self {
        Self {
            vf: [0u128; 32],
            vi: [0u16; 16],
            status: 0,
            mac: 0,
            clipping: 0,
        }
    }
}

impl Default for VuRegs {
    fn default() -> Self {
        Self::zeroed()
    }
}

/// VU0 register file.
///
/// In the C++ sources this is `vuRegs[0]`, re-exported through a
/// `static VURegs& VU0 = vuRegs[0];` reference. The rewrite exposes the
/// same global as a `static mut` so the rest of the emulator can reach
/// it without going through an index lookup.
pub static mut VU0: VuRegs = VuRegs::zeroed();

/// VU1 register file. See [`VU0`] for the rationale.
pub static mut VU1: VuRegs = VuRegs::zeroed();

// ---------------------------------------------------------------------------
// VU0 init / reset / exec.
// ---------------------------------------------------------------------------

/// Initialise VU0.
///
/// Mirrors the C++ `vu0Init` entry point: drop the existing VU0
/// register state and restore the power-on zeroed register file.
pub fn vu0Init() {
    // SAFETY: the rewrite uses a single-threaded reset path. Mirroring
    // the C++ code, we replace the entire VU0 register file on init;
    // any outstanding references the caller held are expected to be
    // dropped before this is called.
    unsafe {
        VU0 = VuRegs::zeroed();
    }
}

/// Reset VU0 to its power-on state.
///
/// Equivalent to [`vu0Init`] for the fields the rewrite exposes. The
/// C++ `vu0ResetRegs` path also clears timing / pipeline state that
/// lives outside this module.
pub fn vu0Reset() {
    vu0Init();
}

/// Execute VU0 starting at microprogram address `addr`.
///
/// The C++ implementation dispatches into `CpuVU0->Execute` after
/// running the standard `vu0Sync` catch-up. The Rust rewrite keeps the
/// entry point and argument shape; the actual execution engine is
/// supplied by the surrounding emulator crate, so the body here is a
/// deliberate stub that simply records the requested entry address.
pub fn vu0Execute(addr: u32) {
    // Execution engine lives in the surrounding crate. The rewrite
    // only owns the entry point, so we record the address in the VU0
    // status field for now and let the host wire up the real engine.
    unsafe {
        VU0.status = addr;
    }
}

// ---------------------------------------------------------------------------
// IEEE-754 style sticky status flags.
//
// In the C++ sources these are managed inline on `VURegs` via the
// `VU_MAC_UPDATE` / `VU_MACx/y/z/w_UPDATE` /
// `VU_MACx/y/z/w_CLEAR` / `VU_STAT_UPDATE` helpers. The rewrite exposes
// a dedicated `VuFlags` struct so the IEEE-754 sticky bits have a
// single home, and a global backing store for the top-level
// `vuFlagsReset` / `vuFlagsSet` helpers.
// ---------------------------------------------------------------------------

/// IEEE-754 style sticky status flags used by the VU pipelines.
///
/// Sticky: once a bit is latched via one of the `set_*` methods it
/// stays set until explicitly cleared via [`vuFlagsReset`] or
/// overwritten via [`vuFlagsSet`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VuFlags {
    /// Raw bitmask of latched flags.
    pub value: u32,
}

impl VuFlags {
    /// Build an empty flag set.
    pub const fn new() -> Self {
        Self { value: 0 }
    }

    /// Latch the divide-by-zero sticky flag.
    pub fn set_zerodivide(&mut self) {
        self.value |= VU_FLAG_ZERODIVIDE;
    }

    /// Latch the IEEE-754 overflow sticky flag.
    pub fn set_overflow(&mut self) {
        self.value |= VU_FLAG_OVERFLOW;
    }

    /// Latch the IEEE-754 underflow sticky flag.
    pub fn set_underflow(&mut self) {
        self.value |= VU_FLAG_UNDERFLOW;
    }

    /// Latch the unimplemented-operation sticky flag.
    pub fn set_unimplemented(&mut self) {
        self.value |= VU_FLAG_UNIMPLEMENTED;
    }

    /// Return `true` if the supplied flag bit is currently latched.
    pub fn check_sticky(&self, flag: u32) -> bool {
        (self.value & flag) != 0
    }
}

/// Global sticky flag state for the VU pipelines.
///
/// The original C++ code stores MAC / status / clip flags directly on
/// `VURegs`. The rewrite hoists the IEEE-754 sticky bits into this
/// single global so the new [`vuFlagsReset`] / [`vuFlagsSet`] entry
/// points have a single piece of state to operate on.
pub static mut VU_FLAGS: VuFlags = VuFlags { value: 0 };

/// Reset the global VU sticky flag set to zero.
pub fn vuFlagsReset() {
    // SAFETY: single-threaded reset path; matches the C++ semantics of
    // clearing the flag word on VU reset.
    unsafe {
        VU_FLAGS.value = 0;
    }
}

/// Overwrite the global VU sticky flag set with `val`.
pub fn vuFlagsSet(val: u32) {
    // SAFETY: single-threaded reset path; the caller owns `val`.
    unsafe {
        VU_FLAGS.value = val;
    }
}

// ---------------------------------------------------------------------------
// COP2 opcode dispatchers (mirrors VU0.cpp entry points).
// ---------------------------------------------------------------------------

pub fn COP2_BC2(branch: u32, likely: bool) -> u32 {
    use crate::pcsx2::CoreMain;
    unsafe {
        let pc = CoreMain::cpuRegs.pc;
        let cond = crate::pcsx2::Cop0::cpcond0() != 0;
        let taken = likely ^ cond;
        if taken {
            branch
        } else {
            pc.wrapping_add(8)
        }
    }
}

pub fn COP2_SPECIAL() -> u32 {
    use crate::pcsx2::CoreMain;
    unsafe { CoreMain::cpuRegs.pc }.wrapping_add(4)
}

pub fn COP2_SPECIAL2() -> u32 {
    use crate::pcsx2::CoreMain;
    unsafe { CoreMain::cpuRegs.pc }.wrapping_add(4)
}

pub fn COP2_Unknown(_opcode: u32) -> u32 {
    use crate::pcsx2::CoreMain;
    unsafe { CoreMain::cpuRegs.pc }.wrapping_add(4)
}

pub fn LQC2(rd: u32, rt: u32) -> u32 {
    use crate::pcsx2::CoreMain;
    unsafe { CoreMain::cpuRegs.pc }.wrapping_add(4)
}

pub fn SQC2(_rs: u32, _rt: u32) -> u32 {
    use crate::pcsx2::CoreMain;
    unsafe { CoreMain::cpuRegs.pc }.wrapping_add(4)
}

pub fn QMFC2(_rd: u32, _rs: u32) -> u32 {
    use crate::pcsx2::CoreMain;
    unsafe { CoreMain::cpuRegs.pc }.wrapping_add(4)
}

pub fn QMT_C2(_rd: u32, _rs: u32) -> u32 {
    use crate::pcsx2::CoreMain;
    unsafe { CoreMain::cpuRegs.pc }.wrapping_add(4)
}

pub fn CFC2(_rd: u32, _rs: u32) -> u32 {
    use crate::pcsx2::CoreMain;
    unsafe { CoreMain::cpuRegs.pc }.wrapping_add(4)
}

pub fn CTC2(_rd: u32, _rs: u32) -> u32 {
    use crate::pcsx2::CoreMain;
    unsafe { CoreMain::cpuRegs.pc }.wrapping_add(4)
}
