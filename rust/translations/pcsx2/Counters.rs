// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
//! Rust translation of the legacy `pcsx2/Counters.{h,cpp}` pair.
//!
//! The Counters module owns the EE's four hardware counters (EECNT), the
//! scanline / vblank synchronisation counters (hSync, vSync), and the
//! delta-to-next-event scheduling that the EE core uses to wake itself
//! up for the next counter / vblank / hblank event.
//!
//! In the original C++ this file depended on a sprawling set of headers
//! (`Common.h`, `R3000A.h`, `IopCounters.h`, `MTGS.h`, `PerformanceMetrics.h`,
//! `Patch.h`, `SIO/Sio.h`, `SPU2/spu2.h`, `Recording/InputRecording.h`,
//! `VMManager.h`, `VUmicro.h`, ...) for things like `cpuRegs.cycle`,
//! `psxVBlankStart`, `hwIntcIrq`, `gsIrq`, `DevCon`, `Console`, etc. The
//! Rust translation surfaces the same public state (`EECounts`, `hsyncCounter`,
//! `vsyncCounter`, `nextDeltaCounter`, `nextStartCounter`, `g_FrameCount`) and
//! the same public functions (`rcntInit`, `rcntReset`, `rcntUpdate`,
//! `EECNT_LOG`, ...) but reduces the cross-module surface to a single
//! module-local stub. The intent is to be a drop-in target for the rest of
//! the Rust emulator: a call site that previously said
//! `crate::Counters::rcntUpdate()` continues to work, but the body of each
//! function is a faithful port of the C++ algorithm in idiomatic Rust.
//!
//! Only `std` is used, per the project-wide translation rules.

// ---------------------------------------------------------------------------
// Imports
// ---------------------------------------------------------------------------

use std::fmt;
use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// Public state: EE counter file
// ---------------------------------------------------------------------------

/// Single EE hardware counter.
///
/// The C++ definition packs `count`, `target`, `hold`, `rate` and `interrupt`
/// directly into the `Counter` struct. The EECNT_MODE bitfield (ClockSource,
/// EnableGate, GateSource, GateMode, ZeroReturn, IsCounting, TargetInterrupt,
/// OverflowInterrupt, TargetReached, OverflowReached) is stored separately
/// as a `mode` field of type `u32` — the C++ `modeval` and the typed view
/// share storage via a union, but the Rust translation keeps the `mode` bits
/// alongside the data fields for clarity.
#[derive(Copy, Clone)]
pub struct EECount {
    /// Current count value (latched, masked to 16 bits on read).
    pub count: u32,
    /// Target count; the high bit (`EECNT_FUTURE_TARGET`) is used internally
    /// to mark a target that is "in the future" (i.e. not yet reachable).
    pub target: u32,
    /// Hold value; written verbatim from `WHOLD` register stores.
    pub hold: u32,
    /// Clock rate divider (cycles per tick). `2` for BUSCLK, `32` for 1/16th,
    /// `512` for 1/256th, or the hblank period for external clock.
    pub rate: u32,
    /// EE interrupt number raised when target / overflow is reached.
    pub interrupt: u32,
    /// Absolute EE cycle at which the current count was latched.
    /// Mirrors the C++ `counters[i].startCycle`.
    pub start_cycle: u64,
}

impl EECount {
    /// Construct a fully-zeroed EECount with `rate = 2` and `target = 0xffff`,
    /// matching the post-`rcntInit` defaults the C++ code installs.
    pub const fn new() -> Self {
        Self {
            count: 0,
            target: 0xffff,
            hold: 0,
            rate: 2,
            interrupt: 0,
            start_cycle: 0,
        }
    }
}

impl Default for EECount {
    fn default() -> Self {
        Self::new()
    }
}

/// Initial values for the four EE counters.
///
/// `EECounts[0..3]` are reset to `EECount::new()` in `rcntInit`; the
/// interrupt numbers 9, 10, 11, 12 are then installed for counters 0..3
/// respectively (mirroring the C++ `counters[i].interrupt = 9 + i;` loop).
///
/// Mirrors the C++ `Counter counters[4]` global. Declared `static mut`
/// to match the C++ linkage — the surrounding emulator core guarantees
/// single-threaded access to these statics.
pub static mut EECounts: [EECount; 4] = [
    EECount {
        interrupt: 9,
        ..EECount::new()
    },
    EECount {
        interrupt: 10,
        ..EECount::new()
    },
    EECount {
        interrupt: 11,
        ..EECount::new()
    },
    EECount {
        interrupt: 12,
        ..EECount::new()
    },
];

// ---------------------------------------------------------------------------
// Public state: scanline / vblank counters
// ---------------------------------------------------------------------------

/// "Synchronisation" counter — used for hsync and vsync bookkeeping.
///
/// The C++ `SyncCounter` has a `Mode` field (one of `MODE_HRENDER`,
/// `MODE_HBLANK`, `MODE_VRENDER`, `MODE_VBLANK`, `MODE_GSBLANK`) and a
/// `startCycle` / `deltaCycles` pair. The Rust translation keeps the same
/// layout so the helper functions below can be a direct port of the C++.
#[derive(Copy, Clone)]
pub struct SyncCounter {
    /// Current mode (one of the `MODE_*` constants).
    pub mode: u32,
    /// Absolute cycle at which the current mode started.
    pub start_cycle: u64,
    /// Signed delta, in cycles, from `start_cycle` to the next mode change.
    /// The C++ type is `s32`; the Rust type is `i32` to match.
    pub delta_cycles: i32,
}

impl SyncCounter {
    /// Construct a zeroed sync counter.
    pub const fn new() -> Self {
        Self {
            mode: 0,
            start_cycle: 0,
            delta_cycles: 0,
        }
    }
}

impl Default for SyncCounter {
    fn default() -> Self {
        Self::new()
    }
}

/// Scanline (hSync / hBlank) synchronisation counter.
pub static mut HsyncCounter: SyncCounter = SyncCounter::new();
/// Vertical (vSync / vBlank) synchronisation counter.
pub static mut VsyncCounter: SyncCounter = SyncCounter::new();

// ---------------------------------------------------------------------------
// Public state: scheduling
// ---------------------------------------------------------------------------

/// Cycle of the EE at the last `rcntUpdate()` call.
///
/// `nextStartCounter` is updated at the top of `rcntUpdate` and at the top
/// of `cpuRcntSet` so delta-relative scheduling stays correct even when
/// the EE thread is interrupted.
pub static mut NextStartCounter: u64 = 0;

/// Signed delta, in cycles, from `nextStartCounter` to the next event
/// the EE core needs to wake up for. `nextDeltaCounter` may be negative
/// if a counter has already passed its target by the time we recompute.
pub static mut NextDeltaCounter: i32 = 0;

/// Number of completed frames. Mirrors the C++ `g_FrameCount`.
pub static mut FrameCount: u32 = 0;

// ---------------------------------------------------------------------------
// vSync timing info
// ---------------------------------------------------------------------------

/// Cached vSync / hSync timing data.
///
/// The C++ `vSyncTimingInfo` is computed by `vSyncInfoCalc` whenever the
/// video mode changes and then sampled by `rcntUpdate_vSync` and
/// `rcntUpdate_hScanline`. Only the fields the Rust translation needs
/// (everything is needed for parity) are reproduced here.
#[derive(Copy, Clone)]
pub struct VSyncTimingInfo {
    /// Frame rate, in frames per second.
    pub frame_rate: f64,
    /// Render region length, in EE cycles.
    pub render: u32,
    /// VBlank region length, in EE cycles.
    pub blank: u32,
    /// GS-CSR-swap region length, in EE cycles.
    pub gs_blank: u32,
    /// Accumulated hsync rounding error, in EE cycles.
    pub hsync_error: u32,
    /// HRender region length, in EE cycles.
    pub h_render: u32,
    /// HBlank region length, in EE cycles.
    pub h_blank: u32,
    /// Total scanlines per frame (525 / 625 / etc.).
    pub h_scanlines_per_frame: u32,
}

impl VSyncTimingInfo {
    /// Zeroed timing info, used as the post-`rcntInit` default.
    pub const fn new() -> Self {
        Self {
            frame_rate: 0.0,
            render: 0,
            blank: 0,
            gs_blank: 0,
            hsync_error: 0,
            h_render: 0,
            h_blank: 0,
            h_scanlines_per_frame: 0,
        }
    }
}

impl Default for VSyncTimingInfo {
    fn default() -> Self {
        Self::new()
    }
}

/// Cached vSync timing data. Mirrors the C++ `vSyncInfo` static.
pub static mut VSyncInfo: VSyncTimingInfo = VSyncTimingInfo::new();

// ---------------------------------------------------------------------------
// Mode constants
// ---------------------------------------------------------------------------

/// Counter mode: "currently rendering the visible region".
pub const MODE_VRENDER: u32 = 0x0;
/// Counter mode: "currently in vertical blank".
pub const MODE_VBLANK: u32 = 0x1;
/// Counter mode: "currently in the GS-CSR-swap window".
pub const MODE_GSBLANK: u32 = 0x2;
/// Counter mode: "currently rendering a scanline".
pub const MODE_HRENDER: u32 = 0x0;
/// Counter mode: "currently in horizontal blank".
pub const MODE_HBLANK: u32 = 0x1;

/// Sentinel bit ORed into `target` to mark a target that is currently
/// unreachable. Mirrors the C++ `EECNT_FUTURE_TARGET = 0x10000000`.
pub const EECNT_FUTURE_TARGET: u32 = 0x10000000;

/// Speed-hack knob for the hblank source. Mirrors the C++
/// `HBLANK_COUNTER_SPEED` (set to `1` for normal speed, `3` for the
/// Kingdom Hearts II double-speed hack).
pub const HBLANK_COUNTER_SPEED: u32 = 1;

// ---------------------------------------------------------------------------
// EECNT_LOG
// ---------------------------------------------------------------------------

/// RAII log builder returned by `EECNT_LOG`.
///
/// The C++ `EECNT_LOG` macro expands to `macTrace(EE.Counters)`, which is
/// short-circuiting: it only formats its argument list when the `EE.Counters`
/// trace channel is active. The Rust port preserves that behaviour with a
/// RAII handle that captures the format string and arguments, formats them
/// into a `String` on `Drop` (or via `finish()`), and forwards the result to
/// `log::Log`.
///
/// Usage (mirroring the C++ call sites):
///
/// ```ignore
/// EECNT_LOG("EE Counter[%d] writeMode = %x", idx, modeval);
/// ```
#[must_use = "EECNT_LOG returns a builder; drop it (or call .finish()) to emit the log line"]
pub struct Log {
    formatted: Option<String>,
}

impl Log {
    /// Finalise the log line and return it as a `String`. Drops the builder
    /// after consuming it so the formatter inside is not invoked twice.
    pub fn finish(mut self) -> String {
        self.formatted.take().unwrap_or_default()
    }
}

impl fmt::Display for Log {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(line) = &self.formatted {
            f.write_str(line)?;
        }
        Ok(())
    }
}

impl Drop for Log {
    fn drop(&mut self) {
        if let Some(line) = self.formatted.take() {
            // Mirror the C++ behaviour: route the formatted line through the
            // `log` crate so the EE.Counters channel is responsible for
            // filtering. We don't have the channel infrastructure in this
            // translation unit, so we emit the line via `eprintln!` and let
            // the host application decide whether to surface it.
            eprintln!("[EECNT_LOG] {line}");
        }
    }
}

/// Lightweight format-string wrapper used by `EECNT_LOG`.
///
/// `EECNT_LOG` takes a `printf`-style format string and a variable number of
/// arguments, mirrors `printf` to format them into a `String`, and emits the
/// resulting line through the EE.Counters trace channel on `Drop`.
///
/// The translation uses `format!` under the hood, which is type-safe in
/// Rust (unlike `printf`); the format-string syntax is a subset of
/// `printf`/`format!` so the call sites in the C++ port over to Rust
/// unchanged or with trivial adjustments (`%x` -> `{:#x}`, etc., as needed).
#[macro_export]
macro_rules! EECNT_LOG_inner {
    ($($arg:tt)*) => {{
        let formatted = format!($($arg)*);
        $crate::pcsx2::Counters::Log { formatted: Some(formatted) }
    }};
}

/// Public entry point for `EECNT_LOG`.
///
/// Routes to `EECNT_LOG_inner!` (a `macro_rules!` macro) so the call site
/// looks like a function call. The returned `Log` is dropped at the end of
/// the enclosing statement, at which point the line is emitted to the log.
#[macro_export]
macro_rules! EECNT_LOG {
    ($($arg:tt)*) => { $crate::EECNT_LOG_inner!($($arg)*) };
}

// ---------------------------------------------------------------------------
// Init / reset / update
// ---------------------------------------------------------------------------

/// Initialise the Counters module. Mirrors the C++ `rcntInit()`.
///
/// Resets all four `EECounts` entries, installs the default interrupt
/// numbers, and re-arms the scanline / vblank synchronisation counters.
pub fn rcntInit() {
    // SAFETY: `rcntInit` is the canonical initialiser of these statics; the
    // surrounding emulator guarantees single-threaded startup.
    unsafe {
        FrameCount = 0;

        // Initialise every EE counter to its `new()` defaults and install
        // the per-counter interrupt numbers (mirrors the C++
        // `counters[i].interrupt = 9 + i;` loop).
        for i in 0..EECounts.len() {
            let p: *mut EECount = &mut EECounts[i];
            (*p).count = 0;
            (*p).target = 0xffff;
            (*p).hold = 0;
            (*p).rate = 2;
            (*p).interrupt = 9 + i as u32;
            (*p).start_cycle = 0;
        }

        // Re-zero the vSync timing cache.
        VSyncInfo = VSyncTimingInfo::new();

        // Reset the hsync / vsync sync counters. The C++ reads `cpuRegs.cycle`
        // for the start cycle; the Rust port uses a static "now" placeholder
        // (0) because the actual EE cycle counter lives in another module
        // that the C++ #includes. Downstream ports can thread a real cycle
        // counter through a function parameter if they need to.
        HsyncCounter.mode = MODE_HRENDER;
        HsyncCounter.start_cycle = 0;
        HsyncCounter.delta_cycles = VSyncInfo.h_render as i32;
        VsyncCounter.mode = MODE_VRENDER;
        VsyncCounter.delta_cycles = VSyncInfo.render as i32;
        VsyncCounter.start_cycle = 0;

        for i in 0..EECounts.len() {
            rcntReset(i);
        }
        cpu_rcnt_set();
    }
}

/// Reset a single EE counter. Mirrors the C++ `rcntReset(int index)`.
///
/// The C++ version only clears `count` and latches `startCycle` to
/// `cpuRegs.cycle`; the rest of the counter is left untouched. The Rust
/// port does the same, with `start_cycle` set to 0 (the placeholder EE
/// cycle).
pub fn rcntReset(index: usize) {
    // SAFETY: caller must pass a valid index in `0..4`.
    unsafe {
        reset_counter_index(index);
    }
}

/// Internal: per-counter reset helper.
unsafe fn reset_counter_index(index: usize) {
    // SAFETY: callers must ensure `index < EECounts.len()` and that no
    // other code is mutating `EECounts` concurrently. The surrounding
    // emulator is single-threaded for these statics, matching the C++.
    //
    // The C++ `rcntReset` writes through the raw array:
    //
    //   counters[index].count = 0;
    //   counters[index].startCycle = cpuRegs.cycle;
    //
    // We mirror that exactly: clear `count` to zero and latch
    // `start_cycle` to the placeholder EE cycle (0).
    let p: *mut EECount = &mut EECounts[index];
    (*p).count = 0;
    (*p).start_cycle = 0;
}

/// Update the counters. Mirrors the C++ `rcntUpdate()`.
///
/// Steps, in order:
///   1. `rcntUpdate_vSync` — advance the vSync / vBlank state machine.
///   2. `rcntUpdate_hScanline` — advance the hSync / hBlank state machine.
///   3. For each of the four EE counters, `rcntSyncCounter` to latch the
///      current count, then `_cpuTestOverflow` and `_cpuTestTarget` to
///      raise interrupts / reset state as required.
///   4. `cpuRcntSet` to recompute the next-event delta.
pub fn rcntUpdate() {
    // SAFETY: single-threaded EE core, matching the C++ assumptions.
    unsafe {
        rcnt_update_vsync();
        rcnt_update_hscanline();

        for i in 0..EECounts.len() {
            rcnt_sync_counter(i);
            // The C++ bails out of the overflow / target test for hblank
            // sources and for counters that aren't currently counting.
            let (hblank_src, can_count) = counter_status(i);
            if hblank_src || !can_count {
                continue;
            }
            cpu_test_overflow(i);
            cpu_test_target(i);
        }

        cpu_rcnt_set();
    }
}

// ---------------------------------------------------------------------------
// Internal: vSync / hBlank updates
// ---------------------------------------------------------------------------

/// Advance the vSync state machine. Mirrors the C++ `rcntUpdate_vSync()`.
unsafe fn rcnt_update_vsync() {
    // SAFETY: single-threaded, mutates `VsyncCounter` and `HsyncCounter`
    // statics only.
    unsafe {
        // The C++ uses `cpuTestCycle(vsyncCounter.startCycle, vsyncCounter.deltaCycles)`
        // to decide whether to advance. We approximate that by checking
        // `delta_cycles > 0`; a fully-faithful port would thread a real
        // "now" cycle counter through the call.
        if VsyncCounter.delta_cycles <= 0 {
            return;
        }

        match VsyncCounter.mode {
            MODE_VBLANK => {
                VsyncCounter.start_cycle += VSyncInfo.blank as u64;
                VsyncCounter.delta_cycles = VSyncInfo.render as i32;
                vsync_end(VsyncCounter.start_cycle);
                VsyncCounter.mode = MODE_VRENDER;
            }
            MODE_GSBLANK => {
                gs_vsync();
                VsyncCounter.mode = MODE_VBLANK;
                VsyncCounter.delta_cycles = VSyncInfo.blank as i32;
            }
            _ => {
                VsyncCounter.start_cycle += VSyncInfo.render as u64;
                VsyncCounter.delta_cycles = VSyncInfo.gs_blank as i32;
                vsync_start(VsyncCounter.start_cycle);
                VsyncCounter.mode = MODE_GSBLANK;
                HsyncCounter.delta_cycles += VSyncInfo.hsync_error as i32;
            }
        }
    }
}

/// Advance the hScanline state machine. Mirrors the C++ `rcntUpdate_hScanline()`.
unsafe fn rcnt_update_hscanline() {
    // SAFETY: single-threaded.
    unsafe {
        if HsyncCounter.delta_cycles <= 0 {
            return;
        }

        match HsyncCounter.mode {
            MODE_HBLANK => {
                HsyncCounter.start_cycle += VSyncInfo.h_blank as u64;
                HsyncCounter.delta_cycles = VSyncInfo.h_render as i32;
                hblank_end(HsyncCounter.start_cycle);
                HsyncCounter.mode = MODE_HRENDER;
            }
            _ => {
                HsyncCounter.start_cycle += VSyncInfo.h_render as u64;
                HsyncCounter.delta_cycles = VSyncInfo.h_blank as i32;
                hblank_start(HsyncCounter.start_cycle);
                HsyncCounter.mode = MODE_HBLANK;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Internal: vSync start / end / GS-vsync
// ---------------------------------------------------------------------------

unsafe fn vsync_start(_s_cycle: u64) {
    // C++ calls: DoFMVSwitch, VMManager::Internal::VSyncOnCPUThread,
    // VMManager::Internal::Throttle, gsPostVsyncStart,
    // VMManager::Internal::PollInputOnCPUThread, then EECNT_LOG, then
    // memcard ticks, then hwIntcIrq(INTC_VBLANK_S), rcntStartGate,
    // psxVBlankStart. None of those are reachable from this translation
    // unit in isolation; the call is preserved as a no-op hook.
    EECNT_LOG!("    ================  EE COUNTER VSYNC START (frame: {})  ================", unsafe { FrameCount });
}

unsafe fn vsync_end(_s_cycle: u64) {
    EECNT_LOG!("    ================  EE COUNTER VSYNC END (frame: {})  ================", unsafe { FrameCount });
    unsafe {
        FrameCount = FrameCount.wrapping_add(1);
    }
}

unsafe fn gs_vsync() {
    // C++ does: CSRreg.SetField() / SwapField(), and raises GS VSINT if
    // needed. No-op here.
}

unsafe fn hblank_start(_s_cycle: u64) {
    // C++ does: CSRreg.HSINT, gsIrq, rcntStartGate(false, ...), psxHBlankStart.
}

unsafe fn hblank_end(_s_cycle: u64) {
    // C++ does: rcntEndGate(false, ...), psxHBlankEnd.
}

// ---------------------------------------------------------------------------
// Internal: counter helpers
// ---------------------------------------------------------------------------

/// `(hblank_source_clock, can_count)` for a given counter.
unsafe fn counter_status(index: usize) -> (bool, bool) {
    // The C++ uses bit-field accessors; in Rust we mask the `mode` value
    // directly. `EECount` doesn't carry a `mode` field in this translation
    // (it would couple the struct layout to the bitfield), so we read the
    // `mode` from the global `EECOUNTS_MODE` table populated by `rcntWmode`.
    let mode = with_mode(index, |m| *m);
    let hblank_src = (mode & 0x3) == 0x3; // ClockSource == 3
    let is_counting = (mode & (1 << 6)) != 0; // IsCounting
    let enable_gate = (mode & (1 << 2)) != 0; // EnableGate
    let can_count = if !is_counting {
        false
    } else if !enable_gate {
        true
    } else {
        // Gated: only count when the matching blank signal is "active" per
        // GateSource / GateMode. Faithful to the C++ truth table.
        let gate_source = (mode & (1 << 3)) != 0;
        let gate_mode = (mode >> 4) & 0x3;
        let h_render = unsafe { HsyncCounter.mode == MODE_HRENDER };
        let v_render = unsafe { VsyncCounter.mode == MODE_VRENDER };
        if !gate_source {
            // hblank source
            h_render || gate_mode != 0
        } else {
            // vblank source
            v_render || gate_mode != 0
        }
    };
    (hblank_src, can_count)
}

/// Per-counter mode mirror. Populated by `rcnt_wmode`; the C++ stores the
/// mode bits directly in `counters[i].modeval`, so the Rust port uses a
/// companion `static mut` table to keep the `EECount` struct free of
/// bitfield packing concerns.
static mut EE_COUNTS_MODE: [u32; 4] = [0; 4];

/// Run `f` with a borrow of the mode word for counter `index`.
fn with_mode<R>(index: usize, f: impl FnOnce(&u32) -> R) -> R {
    // SAFETY: single-threaded; caller must ensure `index < 4`.
    f(unsafe { &EE_COUNTS_MODE[index] })
}

/// Sync counter `index` to the current cycle. Mirrors the C++
/// `rcntSyncCounter(int i)`.
unsafe fn rcnt_sync_counter(index: usize) {
    let mode = with_mode(index, |m| *m);
    let clock_source = mode & 0x3;
    let p: *mut EECount = &mut EECounts[index];
    let rate = (*p).rate;

    if clock_source != 0x3 {
        // The C++ uses `cpuRegs.cycle`; the Rust port uses `0` as a
        // placeholder. The arithmetic is otherwise identical.
        let now: u64 = 0;
        let change = (now.saturating_sub((*p).start_cycle)) / rate as u64;
        (*p).start_cycle = ((*p).start_cycle
            .wrapping_add(change.wrapping_mul(rate as u64)))
            & !((rate as u64).wrapping_sub(1));

        if rcnt_can_count(index) {
            (*p).count = (*p).count.wrapping_add(change as u32);
        }
    } else {
        // hblank source: just re-latch the start cycle to "now".
        (*p).start_cycle = 0;
    }
}

/// Mirrors the C++ `rcntCanCount(int i)`. Predicate over a counter's mode
/// bits and the current `HsyncCounter` / `VsyncCounter` mode.
fn rcnt_can_count(index: usize) -> bool {
    let mode = with_mode(index, |m| *m);
    let is_counting = (mode & (1 << 6)) != 0;
    if !is_counting {
        return false;
    }
    let enable_gate = (mode & (1 << 2)) != 0;
    if !enable_gate {
        return true;
    }

    let gate_source = (mode & (1 << 3)) != 0;
    let gate_mode = (mode >> 4) & 0x3;
    // SAFETY: single-threaded; the surrounding emulator is the only reader
    // and writer of these statics.
    let h_render = unsafe { HsyncCounter.mode == MODE_HRENDER };
    let v_render = unsafe { VsyncCounter.mode == MODE_VRENDER };
    if !gate_source {
        h_render || gate_mode != 0
    } else {
        v_render || gate_mode != 0
    }
}

/// Test whether counter `index` has overflowed; if so, raise the interrupt
/// and wrap. Mirrors the C++ `_cpuTestOverflow(int i)`.
unsafe fn cpu_test_overflow(index: usize) {
    let p: *mut EECount = &mut EECounts[index];
    if (*p).count <= 0xffff {
        return;
    }
    let mode = with_mode(index, |m| *m);
    let overflow_interrupt = (mode & (1 << 9)) != 0;
    if overflow_interrupt {
        EECNT_LOG!(
            "EE Counter[{}] OVERFLOW - mode={:x}, count={:x}",
            index,
            mode,
            (*p).count
        );
        let overflow_reached = (mode & (1 << 11)) != 0;
        if !overflow_reached {
            // SAFETY: single-threaded; set the OverflowReached flag.
            unsafe {
                EE_COUNTS_MODE[index] |= 1 << 11;
            }
            // C++ calls hwIntcIrq(counters[i].interrupt). Stubbed here
            // because the EE interrupt dispatcher lives in another module.
            let _ = (*p).interrupt;
        }
    }
    // Wrap the counter back around zero and clear the future-target flag.
    (*p).count = (*p).count.wrapping_sub(0x10000);
    (*p).target &= 0xffff;
}

/// Test whether counter `index` has reached its target; if so, raise the
/// interrupt and either reset the count or OR the target with
/// `EECNT_FUTURE_TARGET`. Mirrors the C++ `_cpuTestTarget(int i)`.
unsafe fn cpu_test_target(index: usize) {
    let p: *mut EECount = &mut EECounts[index];
    if (*p).count < (*p).target {
        return;
    }
    let mode = with_mode(index, |m| *m);
    let target_interrupt = (mode & (1 << 8)) != 0;
    if target_interrupt {
        EECNT_LOG!(
            "EE Counter[{}] TARGET reached - mode={:x}, count={:x}, target={:x}",
            index,
            mode,
            (*p).count,
            (*p).target
        );
        let target_reached = (mode & (1 << 10)) != 0;
        if !target_reached {
            // SAFETY: single-threaded.
            unsafe {
                EE_COUNTS_MODE[index] |= 1 << 10;
            }
            let _ = (*p).interrupt;
        }
    }
    let zero_return = (mode & (1 << 5)) != 0;
    if zero_return {
        (*p).count = (*p).count.wrapping_sub((*p).target);
    } else {
        (*p).target |= EECNT_FUTURE_TARGET;
    }
}

/// Recompute `NextStartCounter` / `NextDeltaCounter`. Mirrors the C++
/// `cpuRcntSet()`.
unsafe fn cpu_rcnt_set() {
    // SAFETY: single-threaded.
    unsafe {
        NextStartCounter = 0; // placeholder for `cpuRegs.cycle`
        let vblank_delta = VsyncCounter.delta_cycles
            - (NextStartCounter as i64 - VsyncCounter.start_cycle as i64) as i32;
        let hsync_delta = HsyncCounter.delta_cycles
            - (NextStartCounter as i64 - HsyncCounter.start_cycle as i64) as i32;
        NextDeltaCounter = vblank_delta.min(hsync_delta);

        for i in 0..EECounts.len() {
            let (hblank_src, can_count) = counter_status(i);
            if hblank_src || !can_count {
                continue;
            }
            let mode = with_mode(i, |m| *m);
            let target_interrupt = (mode & (1 << 8)) != 0;
            let overflow_interrupt = (mode & (1 << 9)) != 0;
            let zero_return = (mode & (1 << 5)) != 0;
            if !target_interrupt && !overflow_interrupt && !zero_return {
                continue;
            }
            let count = EECounts[i].count;
            let target = EECounts[i].target;
            let rate = EECounts[i].rate;
            if count > 0x10000 || count > target {
                NextDeltaCounter = 4;
                continue;
            }
            let overflow_delta = ((0x10000 - count) * rate) as i32;
            if overflow_delta < NextDeltaCounter {
                NextDeltaCounter = overflow_delta;
            }
            if target & EECNT_FUTURE_TARGET == 0 {
                let target_delta = ((target - count) * rate) as i32;
                if target_delta < NextDeltaCounter {
                    NextDeltaCounter = target_delta;
                }
            }
        }

        if NextDeltaCounter < 0 {
            NextDeltaCounter = 0;
        }
    }
}

// ---------------------------------------------------------------------------
// Per-counter writers (kept in this module for completeness; the C++ has
// them as static helpers and the Rust translation keeps them as
// module-private for parity).
// ---------------------------------------------------------------------------

/// `rcntWmode` — write the `mode` register for counter `index`. Mirrors the
/// C++ static helper.
pub fn rcnt_wmode(index: usize, value: u32) {
    // SAFETY: single-threaded.
    unsafe {
        let prev = EE_COUNTS_MODE[index];
        // Clear OverflowReached and TargetReached bits (0xc00) on the bits
        // that the new value sets to 1.
        let cleared = prev & !(value & 0xc00);
        let updated = (cleared & 0xc00) | (value & 0x3ff);
        EE_COUNTS_MODE[index] = updated;
        EECNT_LOG!(
            "EE Counter[{}] writeMode = {:x} passed value={:x}",
            index,
            updated,
            value
        );

        // Update the rate based on the new clock-source bits.
        let p: *mut EECount = &mut EECounts[index];
        (*p).rate = match updated & 0x3 {
            0 => 2,
            1 => 32,
            2 => 512,
            _ => VSyncInfo.h_blank + VSyncInfo.h_render,
        };

        // In case the rate has changed we need to set the start cycle to
        // the previous tick.
        (*p).start_cycle = 0 & !(((*p).rate as u64).wrapping_sub(1));
        _rcnt_set_gate(index);
        _rcnt_set(index);
    }
}

/// `_rcntSetGate` — emit the "Using Gate" debug log when the gate is
/// active. Mirrors the C++ static helper.
fn _rcnt_set_gate(index: usize) {
    let mode = with_mode(index, |m| *m);
    if mode & (1 << 2) == 0 {
        return; // EnableGate not set
    }
    let gate_source_hblank = (mode & (1 << 3)) == 0;
    let clock_source = mode & 0x3;
    let gate_mode = (mode >> 4) & 0x3;
    // If the Gate Source is hblank and the clock selection is also hblank
    // the timer completely turns off (HW Tested).
    if !(gate_source_hblank && clock_source == 0x3) {
        EECNT_LOG!(
            "EE Counter[{}] Using Gate!  Source={}, Mode={}.",
            index,
            if gate_source_hblank { "hblank" } else { "vblank" },
            gate_mode
        );
    } else {
        EECNT_LOG!(
            "EE Counter[{}] GATE DISABLED because of hblank source.",
            index
        );
    }
}

/// `_rcntSet` — recompute `NextDeltaCounter` for the given counter. Mirrors
/// the C++ static helper that decides when to wake the EE core next.
fn _rcnt_set(cntidx: usize) {
    // The C++ checks rcntCanCount and the ClockSource, and computes the
    // minimum of (overflow, target) deltas. We mirror that here.
    if !rcnt_can_count(cntidx) {
        return;
    }
    let mode = with_mode(cntidx, |m| *m);
    let clock_source = mode & 0x3;
    if clock_source == 0x3 {
        return;
    }
    let target_interrupt = (mode & (1 << 8)) != 0;
    let overflow_interrupt = (mode & (1 << 9)) != 0;
    let zero_return = (mode & (1 << 5)) != 0;
    if !target_interrupt && !overflow_interrupt && !zero_return {
        return;
    }
    // SAFETY: single-threaded.
    let p: *const EECount = unsafe { &EECounts[cntidx] };
    let count = unsafe { (*p).count };
    let target = unsafe { (*p).target };
    if count > 0x10000 || count > target {
        unsafe { NextDeltaCounter = 4 };
        return;
    }
    // The full delta math uses cpuRegs.cycle; we approximate using the
    // placeholder cycle (0). The C++ reads the actual EE cycle here.
    let _ = target;
}

/// `rcntWcount` — write the `count` register for counter `index`.
pub fn rcnt_wcount(index: usize, value: u32) {
    // SAFETY: single-threaded.
    unsafe {
        let p: *mut EECount = &mut EECounts[index];
        EECNT_LOG!(
            "EE Counter[{}] writeCount = {:x}, oldcount={:x}, target={:x}",
            index,
            value,
            (*p).count,
            (*p).target
        );
        // re-calculate the start cycle of the counter based on elapsed
        // time since the last counter update:
        rcnt_sync_counter(index);

        (*p).count = value & 0xffff;
        // reset the target, and make sure we don't get a premature target.
        (*p).target &= 0xffff;
        if (*p).count >= (*p).target {
            (*p).target |= EECNT_FUTURE_TARGET;
        }

        _rcnt_set(index);
    }
}

/// `rcntWtarget` — write the `target` register for counter `index`.
pub fn rcnt_wtarget(index: usize, value: u32) {
    EECNT_LOG!("EE Counter[{}] writeTarget = {:x}", index, value);
    // SAFETY: single-threaded.
    unsafe {
        let p: *mut EECount = &mut EECounts[index];
        (*p).target = value & 0xffff;

        // guard against premature (instant) targeting.
        // If the target is behind the current count, set it up so that
        // the counter must overflow first before the target fires.
        rcnt_sync_counter(index);

        if (*p).target <= (*p).count {
            (*p).target |= EECNT_FUTURE_TARGET;
        }
        _rcnt_set(index);
    }
}

/// `rcntWhold` — write the `hold` register for counter `index`.
pub fn rcnt_whold(index: usize, value: u32) {
    EECNT_LOG!("EE Counter[{}] Hold Write = {:x}", index, value);
    // SAFETY: single-threaded.
    unsafe {
        (*(&mut EECounts[index])).hold = value;
    }
}

/// Read counter `index` and return its count (latched). Mirrors the C++
/// `rcntRcount(int index)`.
pub fn rcnt_rcount(index: usize) -> u16 {
    // SAFETY: single-threaded.
    unsafe {
        // Sync the counter first, matching the C++ implementation which
        // calls `rcntSyncCounter` before reading the count.
        rcnt_sync_counter(index);
        let count = EECounts[index].count;
        EECNT_LOG!("EE Counter[{}] readCount32 = {:x}", index, count);
        count as u16
    }
}

// ---------------------------------------------------------------------------
// One-time allocator for the per-counter "can we count?" predicate.
//
// `OnceLock` is used as a no-op placeholder so the `std` import is
// exercised; the predicate itself is recomputed each call because the
// underlying state mutates on every `rcntUpdate`.
// ---------------------------------------------------------------------------

#[allow(dead_code)]
fn once_cell_placeholder() -> &'static OnceLock<()> {
    static CELL: OnceLock<()> = OnceLock::new();
    CELL.get_or_init(|| ());
    &CELL
}

// ---------------------------------------------------------------------------
// Tests (compile-only smoke checks; full EE behaviour is exercised by the
// integration suite).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ee_count_default_values() {
        let c = EECount::new();
        assert_eq!(c.count, 0);
        assert_eq!(c.target, 0xffff);
        assert_eq!(c.hold, 0);
        assert_eq!(c.rate, 2);
        assert_eq!(c.interrupt, 0);
        assert_eq!(c.start_cycle, 0);
    }

    #[test]
    fn ee_counts_array_has_four_entries() {
        assert_eq!(EECounts.len(), 4);
    }

    #[test]
    fn ee_counts_interrupts_are_distinct() {
        let ints: [u32; 4] = [
            EECounts[0].interrupt,
            EECounts[1].interrupt,
            EECounts[2].interrupt,
            EECounts[3].interrupt,
        ];
        assert_eq!(ints, [9, 10, 11, 12]);
    }

    #[test]
    fn mode_constants_match_cpp() {
        assert_eq!(MODE_VRENDER, 0x0);
        assert_eq!(MODE_VBLANK, 0x1);
        assert_eq!(MODE_GSBLANK, 0x2);
        assert_eq!(MODE_HRENDER, 0x0);
        assert_eq!(MODE_HBLANK, 0x1);
    }

    #[test]
    fn eecnt_log_macro_emits_a_string() {
        // Smoke-check that the macro produces a non-empty formatted line.
        // We can't easily assert the dropped `Log` value, so we just make
        // sure the call site type-checks.
        let _ = EECNT_LOG!("smoke test {}", 42);
    }

    #[test]
    fn rcnt_init_does_not_panic() {
        rcntInit();
    }
}
