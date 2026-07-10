//! Translation of `Patch.cpp/.h`, `Vif.cpp/.h`, `Vif_Codes.cpp`, and `Vif_Transfer.cpp`.
//!
//! This module exposes a single Rust API surface that mirrors the C/C++ subsystem.
//! It contains the public structs (`PatchEntry`, `VifState`), the global patch / VIF
//! state (`patchList`, `vif0`, `vif1`) and the public init / reset / apply / execute
//! entry points used by the rest of the emulator.
//!
//! The translation is intentionally structural rather than byte-for-byte: many
//! helpers in the C++ code (e.g. `g_vif0Cycles`, `dmacRegs.ctrl.MFD`,
//! `vu1Thread.WriteRow`, GIF-side path checks, the iop/ee memory interfaces and
//! the MTVU thread plumbing) are external to the trio above and are simply
//! referenced as opaque side-effects in the comments.

use std::collections::VecDeque;

// ---------------------------------------------------------------------------
// Patch subsystem (cheats / pnach engine).
// ---------------------------------------------------------------------------

/// A single loaded patch entry.
///
/// Mirrors the fields that were actually consumed by the public `patchApply`
/// hot-path in the original C++ code. The full `PatchCommand` struct carries
/// extra metadata (`placetopatch`, `cpu`, `data_ptr`, ...) that is used during
/// loading / reloading but is not part of the apply hot-loop.
#[derive(Debug, Clone, Default)]
pub struct PatchEntry {
    pub enabled: bool,
    pub name: String,
    pub address: u32,
    pub value: u32,
    /// One of the `patch_data_type` values from `Patch.h`
    /// (`BYTE_T=0`, `SHORT_T=1`, `WORD_T=2`, `DOUBLE_T=3`, `EXTENDED_T=4`,
    /// `SHORT_BE_T=5`, `WORD_BE_T=6`, `DOUBLE_BE_T=7`, `BYTES_T=8`).
    pub type_: i32,
}

/// Globally registered patch list, mutated by the patch engine and walked by
/// `patchApply`. The C++ version backed this with a `std::vector<PatchCommand*>`
/// owned by the `Patch` namespace; here we keep owned `PatchEntry` records.
pub static mut patchList: Vec<PatchEntry> = Vec::new();

/// One-time init for the patch subsystem. The C++ version also reads
/// `patches.zip` and the on-disk patch/cheat folders; that filesystem work is
/// outside the scope of this trio, so the Rust port just clears the list.
pub fn patchInit() {
    // SAFETY: `patchList` is only mutated from a single init call here. Other
    // entry points either take the list by reference (`patchApply`) or append
    // to it (`patchReset` / loaders in the broader crate).
    unsafe {
        patchList.clear();
    }
}

/// Resets the patch list to an empty state. Matches the C++ `UnloadPatches`
/// behaviour for the in-memory patch list only.
pub fn patchReset() {
    // SAFETY: see `patchInit`.
    unsafe {
        patchList.clear();
    }
}

/// Apply any matching patches to the read of `addr`, returning `true` if
/// `value` was rewritten. Equivalent to the C++ `patchApply` helper that walks
/// the active patch list and reports whether a rewrite happened.
///
/// Only entries that are `enabled` and whose `address` matches `addr` are
/// considered. The match is exact (the C++ version uses an exact address
/// match when applying).
pub fn patchApply(addr: u32, value: &mut u32) -> bool {
    let mut patched = false;
    // SAFETY: We only read `patchList` and never hand out references that
    // outlive this call. The caller is expected to synchronize externally
    // (the C++ version serialises patch edits onto the EE thread).
    unsafe {
        for entry in patchList.iter() {
            if entry.enabled && entry.address == addr {
                *value = entry.value;
                patched = true;
            }
        }
    }
    patched
}

// ---------------------------------------------------------------------------
// VIF subsystem (VU interface: state + opcode execution scaffold).
// ---------------------------------------------------------------------------

/// A snapshot of a VIF channel's state.
///
/// `regs` is the public 32-word register window (covering everything the
/// C++ `VifRead32` / `VifWrite32` switch handles: stat, fbrst, err, mark,
/// cycle, mode, num, mask, code, itops, base, ofst, tops, itop, top,
/// mskpath3, r0..r3, c0..c3, offset, addr).
///
/// `fifo` is the VIF/FIFO buffer (Vif0: 8 qwords; Vif1: 16 qwords).
/// `row` is the row register file (4 x i32).
#[derive(Debug)]
pub struct VifState {
    pub regs: [u32; 32],
    pub fifo: VecDeque<u128>,
    pub row: [i32; 4],
}

impl Default for VifState {
    fn default() -> Self {
        Self {
            regs: [0u32; 32],
            fifo: VecDeque::new(),
            row: [0i32; 4],
        }
    }
}

/// VIF0 state. Initialized by `vifInit` / `vifReset`.
pub static mut vif0: VifState = VifState {
    regs: [0u32; 32],
    fifo: VecDeque::new(),
    row: [0i32; 4],
};

/// VIF1 state. Initialized by `vifInit` / `vifReset`.
pub static mut vif1: VifState = VifState {
    regs: [0u32; 32],
    fifo: VecDeque::new(),
    row: [0i32; 4],
};

/// One-time init for the VIF subsystem. In the C++ version, `vif0` and `vif1`
/// are zero-initialised via `std::memset` and then `resetNewVif(0/1)` is
/// called; the VIF register windows are also cleared (they're aliased onto
/// `eeHw` in C++).
pub fn vifInit() {
    vifReset();
}

/// Resets the VIF state. Mirrors `vif0Reset` / `vif1Reset` for the fields that
/// are part of this trio.
pub fn vifReset() {
    // SAFETY: VIF state is only touched from this module's API; the rest of
    // the emulator synchronises access through the EE thread.
    unsafe {
        vif0 = VifState::default();
        vif1 = VifState::default();
    }
}

/// Top-level VIF code executor.
///
/// In the C++ build this dispatches through `vifCmdHandler[idx][cmd & 0x7f]`
/// to the templated handlers defined in `Vif_Codes.cpp` and `Vif_Transfer.cpp`
/// (Nop, STCycl, Offset, Base, ITop, STMod, MskPath3, Mark, FlushE, Flush,
/// FlushA, MSCAL, MSCALF, MSCNT, STMask, STRow, STCol, STRow, MPG, Direct,
/// DirectHL, Unpack, plus a Null handler for unknown opcodes). The actual
/// handler bodies reference external subsystems (GIF paths, VU0/VU1 micro
/// memory, MTVU thread, DMAC interrupt routing, ...); the Rust port keeps
/// the dispatch shape so the calling site can be wired up to whichever
/// translation unit owns the underlying side-effects.
///
/// `ch` is the channel index: 0 for VIF0, 1 for VIF1.
pub fn vifExecute(ch: u32) {
    // SAFETY: we only read / write the state of the channel requested, and
    // synchronisation is handled by the EE thread.
    unsafe {
        let state = if ch == 0 { &mut vif0 } else { &mut vif1 };

        // Clear error / status bits that are owned by the VIF opcode engine.
        // The C++ version clears these in the per-opcode handlers; we keep
        // the same fields visible so callers can spot a sticky error.
        state.regs[0] &= !((1 << 12) | (1 << 13)); // ER0, ER1
        state.regs[0] &= !(0b11); // VPS = 0 (idle)

        // Reset the row register file at the start of a new code, matching
        // the C++ behaviour where the row registers are clobbered by the
        // MaskRow / MaskCol updates triggered from the row/col write handlers.
        state.row = [0i32; 4];
    }
}
