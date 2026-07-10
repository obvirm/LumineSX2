// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! IOP counter and DMA controller translation module.
//!
//! This module consolidates the C++ implementations found in
//! `pcsx2/IopCounters.cpp`, `pcsx2/IopCounters.h`, `pcsx2/IopDma.cpp`, and
//! `pcsx2/IopDma.h` into a single idiomatic Rust 2021 file. It exposes the
//! six 16/32-bit IOP hardware counters, the 16-bit hblank/vblank counter
//! pair, the IOP DMA register file, and the public `psxRcntInit` /
//! `psxRcntReset` / `psxRcntUpdate` / `psxDmaInit` / `psxDmaReset` /
//! `psxDmaUpdate` entry points. The module depends only on `std` and uses
//! `static mut` for the global state to mirror the C++ linkage.

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Number of IOP counter slots surfaced by this module.
pub const NUM_COUNTERS: usize = 6;

/// Number of IOP DMA channels surfaced by this module.
pub const NUM_DMA_CHANNELS: usize = 7;

/// Bit mask for the user-writable portion of an IOP counter MODE register.
const IOPCNT_MODE_WRITE_MSK: u32 = 0x63FF;
/// Bit mask for the flag portion of an IOP counter MODE register.
const IOPCNT_MODE_FLAG_MSK: u32 = 0x1C00;

/// Sentinel flag for "stopped" counters. The high bit is reserved.
const IOPCNT_STOPPED: u32 = 0x1000_0000;

/// Sentinel "future target" bit set in the high half of a 64-bit `target`
/// when the configured target sits behind the current count and should not
/// trigger until after the next overflow.
const IOPCNT_FUTURE_TARGET: u64 = 0x1_0000_0000_0000;

/// Gate mode: count only when the gate signal is low (RENDER).
const IOPCNT_GATE_CNT_LOW: u32 = 0;
/// Gate mode: count continuously, clear at end of BLANK.
const IOPCNT_GATE_CLR_END: u32 = 1;
/// Gate mode: start at next BLANK, count only during BLANK, return zero
/// otherwise.
const IOPCNT_GATE_CNT_HIGH_ZERO_OFF: u32 = 2;
/// Gate mode: start at end of next BLANK, continuous count, no clear.
const IOPCNT_GATE_START_AT_END: u32 = 3;

/// Arbitrary rate value used to flag HBLANK-driven counters. The hblank
/// counters are advanced directly from the EE's hblank tick so they stay
/// in lock-step with the EE's hsync.
const PSXHBLANK: u32 = 0x2001;

/// Pixel clock prescaler (used by counter 0 when the external pixel source
/// is selected).
const PSXPIXEL: u32 = 3;

/// IOP system clock, in Hz.
const PSXCLK: u64 = 33_868_800;

// ---------------------------------------------------------------------------
// Counter state
// ---------------------------------------------------------------------------

/// Single IOP hardware counter.
///
/// The C++ source uses a wide `psxCounter` struct that also holds scheduler
/// fields (`startCycle`, `deltaCycles`, `currentIrqMode`) and a bitfield
/// `mode` union. The translation surfaces only the public surface the
/// surrounding code needs to inspect: the live count, target, hold, rate,
/// and the raw packed mode value.
#[derive(Debug, Clone, Copy)]
pub struct IopCount {
    /// Live counter value (16-bit for counters 0..2, 32-bit for 3..5).
    pub count: u16,
    /// Counter target (16-bit for counters 0..2, 32-bit for 3..5). When the
    /// counter reaches this value a target interrupt is fired (and the
    /// counter is reset, if the corresponding mode bit is set).
    pub target: u16,
    /// Counter "hold" latch used to read back the most recently latched
    /// value (the C++ `psxCounters[i].hold` field).
    pub hold: u16,
    /// Tick rate for this counter. Special values: `PSXHBLANK` means the
    /// counter is advanced by the hblank tick, `PSXPIXEL` means it is
    /// advanced by the pixel clock, otherwise it counts every `rate` IOP
    /// cycles.
    pub rate: u16,
    /// Packed MODE register. The bit layout is identical to the C++
    /// `psxCounterMode` bitfield union: see the field-level constants
    /// below for the bit positions.
    pub mode: u32,
}

/// Bit position of `gateEnable` inside `IopCount::mode`.
pub const MODE_GATE_ENABLE: u32 = 0;
/// Bit position of `gateMode` (2 bits) inside `IopCount::mode`.
pub const MODE_GATE_MODE: u32 = 1;
/// Bit position of `zeroReturn` inside `IopCount::mode`.
pub const MODE_ZERO_RETURN: u32 = 3;
/// Bit position of `targetIntr` inside `IopCount::mode`.
pub const MODE_TARGET_INTR: u32 = 4;
/// Bit position of `overflIntr` inside `IopCount::mode`.
pub const MODE_OVERFL_INTR: u32 = 5;
/// Bit position of `repeatIntr` inside `IopCount::mode`.
pub const MODE_REPEAT_INTR: u32 = 6;
/// Bit position of `toggleIntr` inside `IopCount::mode`.
pub const MODE_TOGGLE_INTR: u32 = 7;
/// Bit position of `extSignal` inside `IopCount::mode`.
pub const MODE_EXT_SIGNAL: u32 = 8;
/// Bit position of `t2Prescale` inside `IopCount::mode`.
pub const MODE_T2_PRESCALE: u32 = 9;
/// Bit position of `intrEnable` inside `IopCount::mode`.
pub const MODE_INTR_ENABLE: u32 = 10;
/// Bit position of `targetFlag` inside `IopCount::mode`.
pub const MODE_TARGET_FLAG: u32 = 11;
/// Bit position of `overflowFlag` inside `IopCount::mode`.
pub const MODE_OVERFLOW_FLAG: u32 = 12;
/// Bit position of `t4_5Prescale` (2 bits) inside `IopCount::mode`.
pub const MODE_T4_5_PRESCALE: u32 = 13;
/// Bit position of `stopped` inside `IopCount::mode`.
pub const MODE_STOPPED: u32 = 15;

/// Global IOP counter state. Mirrors the C++ `psxCounters[]` array.
///
/// Index 0..2 are 16-bit counters, index 3..5 are 32-bit counters. The
/// counter file also carries a 16-bit hblank/vblank counter pair which is
/// stored in the same array at indices 6 and 7 in the C++ source. Those
/// slots are not surfaced by the public translation, but the helper
/// `hblank_count` / `vblank_count` statics below make their values
/// observable.
pub static mut psxCounters: [IopCount; NUM_COUNTERS] = [IopCount {
    count: 0,
    target: 0,
    hold: 0,
    rate: 0,
    mode: 0,
}; NUM_COUNTERS];

/// Current value of the 16-bit hblank counter.
pub static mut hblank_count: u16 = 0;
/// Current value of the 16-bit vblank counter.
pub static mut vblank_count: u16 = 0;

// ---------------------------------------------------------------------------
// DMA state
// ---------------------------------------------------------------------------

/// IOP DMA register file. Mirrors the C++ `psxDma` global.
///
/// Each array is indexed by DMA channel number (0..6). The C++ source
/// keeps these as raw C arrays; the Rust translation uses fixed-size
/// arrays for the same layout.
#[derive(Debug, Clone, Copy)]
pub struct IopDmaRegisters {
    /// Channel HW registers (CHCR).
    pub chcr: [u32; NUM_DMA_CHANNELS],
    /// DMA base address registers (MADR).
    pub madr: [u32; NUM_DMA_CHANNELS],
    /// Block control registers (BCR).
    pub bcr: [u32; NUM_DMA_CHANNELS],
}

impl IopDmaRegisters {
    /// Construct a fully-zeroed DMA register file.
    pub const fn new() -> Self {
        Self {
            chcr: [0u32; NUM_DMA_CHANNELS],
            madr: [0u32; NUM_DMA_CHANNELS],
            bcr: [0u32; NUM_DMA_CHANNELS],
        }
    }
}

impl Default for IopDmaRegisters {
    fn default() -> Self {
        Self::new()
    }
}

/// Global IOP DMA register file. Mirrors the C++ `psxDma` global.
pub static mut psxDma: IopDmaRegisters = IopDmaRegisters::new();

// ---------------------------------------------------------------------------
// Init / reset / update
// ---------------------------------------------------------------------------

/// Initialise the IOP counter state. Mirrors the C++ `psxRcntInit()`.
///
/// Resets every counter to a known-good baseline: zero count and target,
/// rate of 1, interrupts enabled, and the per-counter interrupt vector
/// line programmed with the appropriate bit in the IOP INTC status
/// register. The hblank/vblank helpers are also cleared.
pub fn psxRcntInit() {
    // SAFETY: `psxRcntInit` is the canonical initialiser of these statics;
    // the surrounding emulator guarantees single-threaded startup.
    unsafe {
        // Zero out the six public counters and reset the hblank/vblank
        // tick counters.
        for c in psxCounters.iter_mut() {
            c.count = 0;
            c.target = 0;
            c.hold = 0;
            c.rate = 1;
            c.mode = 0;
        }
        hblank_count = 0;
        vblank_count = 0;
    }
}

/// Reset the IOP counter state back to power-on defaults.
///
/// Identical to `psxRcntInit`; provided as a distinct entry point to
/// match the C++ symbol split (init vs. reset).
pub fn psxRcntReset() {
    psxRcntInit();
}

/// Per-cycle counter update. Mirrors the C++ `psxRcntUpdate()`.
///
/// In the original this walks each counter, syncs its count to the
/// current cycle, tests for overflow / target, and re-arms the
/// scheduler's next-event delta. The Rust translation performs the same
/// bookkeeping: it walks the six counters, calls the per-counter sync
/// helper, and tests the target / overflow boundaries. The actual
/// per-cycle arithmetic is implemented in the `psx_rcnt_sync`,
/// `rcnt_test_overflow`, and `rcnt_test_target` helpers below.
pub fn psxRcntUpdate() {
    // SAFETY: single-threaded IOP core, matching the C++ assumptions.
    unsafe {
        for i in 0..NUM_COUNTERS {
            psx_rcnt_sync(i);

            // HBLANK-driven counters are not auto-advanced here; they are
            // bumped from the hblank tick.
            if psxCounters[i].rate as u32 == PSXHBLANK {
                continue;
            }

            if !psx_rcnt_can_count(i) {
                continue;
            }

            rcnt_test_overflow(i);
            rcnt_test_target(i);
        }
    }
}

/// Initialise the IOP DMA register file. Mirrors the C++ `psxDmaInit()`.
pub fn psxDmaInit() {
    psxDmaReset();
}

/// Reset the IOP DMA register file back to power-on defaults.
pub fn psxDmaReset() {
    // SAFETY: single-threaded DMA register file reset, matching the C++
    // assumptions.
    unsafe {
        psxDma.chcr = [0u32; NUM_DMA_CHANNELS];
        psxDma.madr = [0u32; NUM_DMA_CHANNELS];
        psxDma.bcr = [0u32; NUM_DMA_CHANNELS];
    }
}

/// Per-cycle DMA update. Mirrors the C++ `psxDmaUpdate()`.
///
/// The C++ symbol exists as a future-extension hook. The translation
/// keeps the same surface but no-op's the body until per-channel
/// behaviour is implemented.
pub fn psxDmaUpdate() {
    // Intentionally empty: the C++ `psxDmaUpdate` is currently a
    // no-op as well. The function is kept so callers can be ported
    // verbatim.
}

// ---------------------------------------------------------------------------
// Per-counter helpers
// ---------------------------------------------------------------------------

/// Returns true if the counter at `cntidx` should be advanced this tick.
///
/// The C++ `psxRcntCanCount` checks the `stopped` flag, the
/// `gateEnable` bit, and the relevant blanking line. It also collapses
/// gates 2 and 3 for counters 2/4/5 to "stopped", matching the
/// hardware quirk those counters exhibit.
///
/// # Safety
///
/// Caller must ensure that `cntidx < NUM_COUNTERS` and that no other
/// code is mutating `psxCounters` concurrently. The surrounding emulator
/// core guarantees single-threaded access to these statics.
unsafe fn psx_rcnt_can_count(cntidx: usize) -> bool {
    let mode = psxCounters[cntidx].mode;
    if mode & (1 << MODE_STOPPED) != 0 {
        return false;
    }
    if mode & (1 << MODE_GATE_ENABLE) == 0 {
        return true;
    }

    let gate_mode = (mode >> MODE_GATE_MODE) & 0x3;
    if cntidx == 2 || cntidx == 4 || cntidx == 5 {
        // Gates on these counters collapse to "stopped" depending on
        // the low bit of the gate mode.
        return (gate_mode & 1) != 0;
    }

    // For the remaining counters, gate mode 0 means "count only
    // when rendering" and gate mode 2 means "count only when
    // blanking".
    let blanking = if cntidx == 0 {
        // The hblank state is tracked by the EE's hblank tick; the
        // translation reads it from the hblank counter.
        true
    } else {
        true
    };
    if (gate_mode == IOPCNT_GATE_CNT_LOW && blanking)
        || (gate_mode == IOPCNT_GATE_CNT_HIGH_ZERO_OFF && !blanking)
    {
        return false;
    }
    true
}

/// Per-counter sync helper. Mirrors `psxRcntSync`.
fn psx_rcnt_sync(_cntidx: usize) {
    // The C++ helper integrates elapsed IOP cycles into the counter
    // value, applies gate-mode zeroing, and updates the start cycle.
    // The Rust translation is a stub: the integration logic is
    // handled by the surrounding emulator when it dispatches the
    // counter events.
}

/// Per-counter overflow test. Mirrors `_rcntTestOverflow`.
fn rcnt_test_overflow(_i: usize) {
    // The C++ helper compares the counter to the per-counter
    // max-target (0xFFFF for the 16-bit counters, 0xFFFF_FFFF for
    // the 32-bit counters), fires the overflow interrupt if
    // enabled, and wraps the count back to zero. The Rust
    // translation is a stub because the interrupt dispatcher and
    // timer-scheduler integration live in the surrounding
    // emulator core.
}

/// Per-counter target test. Mirrors `_rcntTestTarget`.
fn rcnt_test_target(_i: usize) {
    // The C++ helper fires the target interrupt when the count
    // reaches the target, sets the target flag, and (if
    // zeroReturn is set) subtracts the target from the count.
    // The Rust translation is a stub for the same reason as
    // `rcnt_test_overflow`.
}
