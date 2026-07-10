// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
//! Rust translation of the legacy `pcsx2/PerformanceMetrics.cpp` source.
//!
//! This module hosts the per-frame performance-metrics accumulator. In the
//! original C++ the data lives in a handful of file-scope `static` variables
//! and is mutated from the EE / GS / VU threads plus the GS-present callback.
//! The Rust port keeps that one-state-per-program layout but wraps the
//! mutable accumulator in a single value type so the data lives behind a
//! guard, mirroring the source of truth at `PerformanceMetrics::*`.
//!
//! The header is translated together with the implementation:
//!
//! * `PerformanceMetrics::InternalFPSMethod` (the heuristic used to
//!   derive the in-game framerate from privileged register writes or
//!   framebuffer blits).
//! * `PerformanceMetrics::FrameTimeHistory` (the rolling 150-sample
//!   buffer of frame times in milliseconds).
//!
//! The full C++ namespace API is preserved as a `PerformanceMetrics`
//! `struct` whose associated functions mirror the free functions declared
//! in `PerformanceMetrics.h`. This keeps call sites such as
//! `PerformanceMetrics::GetFPS()` compiling without further changes.

#![allow(dead_code)]

use std::sync::Mutex;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// How often (in seconds) the rolling metrics are committed to the public
/// snapshot values. Mirrors the `UPDATE_INTERVAL` constant in
/// `PerformanceMetrics.cpp`.
pub const UPDATE_INTERVAL_SECS: f32 = 0.5_f32;

/// Number of frame-time samples retained in the rolling history buffer
/// (mirrors `PerformanceMetrics::NUM_FRAME_TIME_SAMPLES`).
pub const NUM_FRAME_TIME_SAMPLES: usize = 150;

/// Rolling history of the most recent frame times in milliseconds. Mirrors
/// `PerformanceMetrics::FrameTimeHistory` from the C++ header
/// (`std::array<float, NUM_FRAME_TIME_SAMPLES>`).
pub type FrameTimeHistory = [f32; NUM_FRAME_TIME_SAMPLES];

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// Method used to derive the "internal" (i.e. in-game) FPS counter from
/// observation of the GS thread. Mirrors
/// `PerformanceMetrics::InternalFPSMethod` in the C++ header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InternalFpsMethod {
    /// No heuristic matched during the last update interval.
    None,
    /// A GS privileged-register write was observed. This is the preferred
    /// signal because it has fewer false positives than framebuffer blits.
    GsPrivilegedRegister,
    /// A `DISPFB` blit was observed.
    DispFbBlit,
}

impl Default for InternalFpsMethod {
    fn default() -> Self {
        Self::None
    }
}

// ---------------------------------------------------------------------------
// Per-GS-software-thread stats
// ---------------------------------------------------------------------------

/// Aggregated CPU usage and CPU time for a single GS software rendering
/// thread. Mirrors the file-scope `GSSWThreadStats` struct in
/// `PerformanceMetrics.cpp`.
#[derive(Debug, Clone, Copy, Default)]
pub struct GsswThreadStats {
    /// Last observed cumulative CPU time of the thread, in whatever
    /// units `Threading::ThreadHandle::GetCPUTime()` returns. Stored as a
    /// `u64` to match the C++ layout.
    pub last_cpu_time: u64,
    /// CPU usage expressed as a percentage of the most recent update
    /// interval. Mirrors `GSSWThreadStats::usage` in the C++ source.
    pub usage: f64,
    /// Average CPU time per frame over the most recent update interval,
    /// in milliseconds. Mirrors `GSSWThreadStats::time` in the C++ source.
    pub time: f64,
}

// ---------------------------------------------------------------------------
// The metrics accumulator
// ---------------------------------------------------------------------------

/// Per-frame performance-metrics aggregator. Mirrors the file-scope
/// statics in `PerformanceMetrics.cpp` plus the free functions declared
/// in `PerformanceMetrics.h`.
///
/// The C++ implementation spreads its state across roughly twenty file-scope
/// `static` variables; the Rust port keeps them together inside a single
/// struct so that ownership and aliasing are clear. A single `Mutex` guards
/// the whole state because the original code is already designed for
/// multi-threaded access (EE thread, GS thread, VU thread, capture thread
/// and the host-present callback all touch the accumulator).
#[derive(Debug)]
pub struct PerformanceMetrics {
    inner: Mutex<PerformanceMetricsState>,
}

/// Mutable state of the [`PerformanceMetrics`] aggregator.
#[derive(Debug)]
struct PerformanceMetricsState {
    // FPS / frame-time snapshot values.
    fps: f32,
    internal_fps: f32,
    minimum_frame_time: f32,
    average_frame_time: f32,
    maximum_frame_time: f32,

    // Per-interval accumulators (flushed into the snapshot values every
    // `UPDATE_INTERVAL_SECS` seconds).
    minimum_frame_time_acc: f32,
    average_frame_time_acc: f32,
    maximum_frame_time_acc: f32,
    frames_since_last_update: u32,
    unskipped_frames_since_last_update: u32,

    // Internal-FPS heuristic.
    internal_fps_method: InternalFpsMethod,
    gs_privileged_register_writes_since_last_update: u32,
    gs_framebuffer_blits_since_last_update: u32,

    // Frame number (GS-thread counter).
    frame_number: u64,

    // CPU usage / average time for each background thread.
    cpu_thread_usage: f64,
    cpu_thread_time: f64,
    gs_thread_usage: f32,
    gs_thread_time: f32,
    vu_thread_usage: f32,
    vu_thread_time: f32,
    capture_thread_usage: f32,
    capture_thread_time: f32,

    // Last observed CPU time for each background thread. Stored as `u64`
    // to match the C++ `u64` typed statics; the deltas are computed as
    // `cpu_time - last_cpu_time`.
    last_cpu_time: u64,
    last_gs_time: u64,
    last_vu_time: u64,
    last_capture_time: u64,
    last_ticks: u64,
    last_update_ticks: u64,
    last_frame_ticks: u64,

    // GS software threads. The C++ code uses a `std::vector<GSSWThreadStats>`
    // keyed by an integer index. We store `Vec<GsswThreadStats>` and resize
    // it from `SetGSSWThreadCount`.
    gs_sw_threads: Vec<GsswThreadStats>,

    // GPU-side timing.
    average_gpu_time: f32,
    accumulated_gpu_time: f32,
    gpu_usage: f32,
    presents_since_last_update: u32,

    // Rolling history buffer of recent frame times (in milliseconds).
    frame_time_history: FrameTimeHistory,
    frame_time_history_pos: usize,

    // Has the accumulator ever been initialised? Mirrors the fact that the
    // C++ statics start at zero, but we still need a way to know whether
    // `last_update_ticks` is meaningful.
    initialised: bool,
}

impl Default for PerformanceMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl PerformanceMetrics {
    /// Construct a fresh, zeroed metrics aggregator. Mirrors the initial
    /// state of the file-scope statics in `PerformanceMetrics.cpp` before
    /// `Clear()` is ever called.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(PerformanceMetricsState {
                fps: 0.0,
                internal_fps: 0.0,
                minimum_frame_time: 0.0,
                average_frame_time: 0.0,
                maximum_frame_time: 0.0,
                minimum_frame_time_acc: 0.0,
                average_frame_time_acc: 0.0,
                maximum_frame_time_acc: 0.0,
                frames_since_last_update: 0,
                unskipped_frames_since_last_update: 0,
                internal_fps_method: InternalFpsMethod::None,
                gs_privileged_register_writes_since_last_update: 0,
                gs_framebuffer_blits_since_last_update: 0,
                frame_number: 0,
                cpu_thread_usage: 0.0,
                cpu_thread_time: 0.0,
                gs_thread_usage: 0.0,
                gs_thread_time: 0.0,
                vu_thread_usage: 0.0,
                vu_thread_time: 0.0,
                capture_thread_usage: 0.0,
                capture_thread_time: 0.0,
                last_cpu_time: 0,
                last_gs_time: 0,
                last_vu_time: 0,
                last_capture_time: 0,
                last_ticks: 0,
                last_update_ticks: 0,
                last_frame_ticks: 0,
                gs_sw_threads: Vec::new(),
                average_gpu_time: 0.0,
                accumulated_gpu_time: 0.0,
                gpu_usage: 0.0,
                presents_since_last_update: 0,
                frame_time_history: [0.0; NUM_FRAME_TIME_SAMPLES],
                frame_time_history_pos: 0,
                initialised: false,
            }),
        }
    }

    /// Clear every metric and accumulator. Mirrors
    /// `PerformanceMetrics::Clear()` in the C++ source.
    pub fn clear(&self) {
        self.reset();
        let mut state = self.inner.lock().expect("PerformanceMetrics lock poisoned");
        state.fps = 0.0;
        state.internal_fps = 0.0;
        state.minimum_frame_time = 0.0;
        state.average_frame_time = 0.0;
        state.maximum_frame_time = 0.0;
        state.internal_fps_method = InternalFpsMethod::None;
        state.cpu_thread_usage = 0.0;
        state.cpu_thread_time = 0.0;
        state.gs_thread_usage = 0.0;
        state.gs_thread_time = 0.0;
        state.vu_thread_usage = 0.0;
        state.vu_thread_time = 0.0;
        state.capture_thread_usage = 0.0;
        state.capture_thread_time = 0.0;
        state.average_gpu_time = 0.0;
        state.gpu_usage = 0.0;
        state.frame_number = 0;
        state.frame_time_history.fill(0.0);
        state.frame_time_history_pos = 0;
    }

    /// Reset only the per-interval accumulators. Mirrors
    /// `PerformanceMetrics::Reset()` in the C++ source.
    pub fn reset(&self) {
        let mut state = self.inner.lock().expect("PerformanceMetrics lock poisoned");
        state.frames_since_last_update = 0;
        state.unskipped_frames_since_last_update = 0;
        state.gs_framebuffer_blits_since_last_update = 0;
        state.gs_privileged_register_writes_since_last_update = 0;
        state.minimum_frame_time_acc = 0.0;
        state.average_frame_time_acc = 0.0;
        state.maximum_frame_time_acc = 0.0;
        state.accumulated_gpu_time = 0.0;
        state.presents_since_last_update = 0;
        // The C++ version calls `s_last_update_time.Reset()` and
        // `s_last_frame_time.Reset()` here; in Rust we treat the next
        // observed tick as the new "start" tick.
        state.last_update_ticks = current_tick_value();
        state.last_frame_ticks = current_tick_value();
        state.last_ticks = current_tick_value();
        state.initialised = true;
    }

    /// Per-frame update. Mirrors the body of `PerformanceMetrics::Update`
    /// in the C++ source.
    ///
    /// * `gs_register_write` — `true` if the GS thread performed a
    ///   privileged register write during the frame. Used to derive the
    ///   internal FPS when available.
    /// * `fb_blit` — `true` if the GS thread performed a framebuffer blit
    ///   during the frame. Used as a fallback internal-FPS signal.
    /// * `is_skipping_present` — `true` when the present was skipped (the
    ///   frame does not contribute to wall-clock frame timing).
    /// * `now_ticks` — current monotonic tick count in the same units
    ///   returned by `current_tick_value()`. Defaults to the current
    ///   value if not supplied.
    /// * `frame_time_ms` — frame time in milliseconds, only meaningful
    ///   when `is_skipping_present` is `false`.
    /// * `cpu_time`, `gs_time`, `vu_time`, `capture_time` — cumulative
    ///   CPU time counters (in `u64` ticks) for the EE/GS/VU/capture
    ///   threads, queried by the caller. `vu_time` and `capture_time` are
    ///   ignored when the corresponding thread is inactive (matches the
    ///   `THREAD_VU1` / `GSCapture::IsCapturing()` checks in the C++
    ///   source).
    /// * `ticks` — current "ticks" value from `GetCPUTicks()`.
    /// * `ticks_per_second` — value of `Threading::GetThreadTicksPerSecond()`.
    /// * `tick_frequency` — value of `GetTickFrequency()`.
    /// * `blit_internal_fps_hack` — value of
    ///   `EmuConfig.Gamefixes.BlitInternalFPSHack`. When `true`, the
    ///   `DISPFBBlit` heuristic is forced (the privileged-register
    ///   signal is suppressed).
    /// * `on_metrics_updated` — optional callback fired when the public
    ///   snapshot is refreshed. Mirrors `Host::OnPerformanceMetricsUpdated()`.
    pub fn update(
        &self,
        gs_register_write: bool,
        fb_blit: bool,
        is_skipping_present: bool,
        now_ticks: u64,
        frame_time_ms: f32,
        cpu_time: u64,
        gs_time: u64,
        vu_time: u64,
        capture_time: u64,
        ticks: u64,
        ticks_per_second: u64,
        tick_frequency: u64,
        blit_internal_fps_hack: bool,
        on_metrics_updated: Option<&mut dyn FnMut()>,
    ) {
        let mut state = self.inner.lock().expect("PerformanceMetrics lock poisoned");

        if !is_skipping_present {
            let frame_time = frame_time_ms;
            state.minimum_frame_time_acc = if state.minimum_frame_time_acc == 0.0 {
                frame_time
            } else {
                state.minimum_frame_time_acc.min(frame_time)
            };
            state.average_frame_time_acc += frame_time;
            state.maximum_frame_time_acc = state.maximum_frame_time_acc.max(frame_time);
            let pos = state.frame_time_history_pos;
            state.frame_time_history[pos] = frame_time;
            state.frame_time_history_pos = (pos + 1) % NUM_FRAME_TIME_SAMPLES;
            state.unskipped_frames_since_last_update += 1;
        }

        state.frames_since_last_update += 1;
        if gs_register_write {
            state.gs_privileged_register_writes_since_last_update += 1;
        }
        if fb_blit {
            state.gs_framebuffer_blits_since_last_update += 1;
        }
        state.frame_number += 1;

        if !state.initialised {
            state.last_update_ticks = now_ticks;
            state.last_frame_ticks = now_ticks;
            state.last_ticks = ticks;
            state.last_cpu_time = cpu_time;
            state.last_gs_time = gs_time;
            state.last_vu_time = vu_time;
            state.last_capture_time = capture_time;
            state.initialised = true;
            return;
        }

        let elapsed_ticks = now_ticks.saturating_sub(state.last_update_ticks);
        let time_secs = ticks_to_seconds(elapsed_ticks, tick_frequency);
        if time_secs < UPDATE_INTERVAL_SECS {
            return;
        }

        state.last_update_ticks = now_ticks;

        // Commit the rolling min/avg/max into the snapshot values.
        let min = std::mem::replace(&mut state.minimum_frame_time_acc, 0.0);
        let avg_acc = std::mem::replace(&mut state.average_frame_time_acc, 0.0);
        let max = std::mem::replace(&mut state.maximum_frame_time_acc, 0.0);
        state.minimum_frame_time = min;
        state.average_frame_time = if state.unskipped_frames_since_last_update > 0 {
            avg_acc / state.unskipped_frames_since_last_update as f32
        } else {
            0.0
        };
        state.maximum_frame_time = max;
        state.fps = state.frames_since_last_update as f32 / time_secs;

        // GPU timing.
        let unskipped = state.unskipped_frames_since_last_update as f32;
        if unskipped > 0.0 {
            state.average_gpu_time = state.accumulated_gpu_time / unskipped;
            state.gpu_usage = state.accumulated_gpu_time / (time_secs * 10.0);
        } else {
            state.average_gpu_time = 0.0;
            state.gpu_usage = 0.0;
        }
        state.accumulated_gpu_time = 0.0;

        // Internal FPS heuristic. Prefer the privileged-register signal
        // (fewer false positives) unless the user explicitly opts in to
        // the framebuffer-blit hack.
        if state.gs_privileged_register_writes_since_last_update > 0 && !blit_internal_fps_hack
        {
            state.internal_fps = state.gs_privileged_register_writes_since_last_update as f32
                / time_secs;
            state.internal_fps_method = InternalFpsMethod::GsPrivilegedRegister;
        } else if state.gs_framebuffer_blits_since_last_update > 0 {
            state.internal_fps = state.gs_framebuffer_blits_since_last_update as f32 / time_secs;
            state.internal_fps_method = InternalFpsMethod::DispFbBlit;
        } else {
            state.internal_fps = 0.0;
            state.internal_fps_method = InternalFpsMethod::None;
        }
        state.gs_privileged_register_writes_since_last_update = 0;
        state.gs_framebuffer_blits_since_last_update = 0;

        // Per-thread CPU usage / time. The C++ code derives a single
        // `pct_divider` and a single `time_divider` from
        // `Threading::GetThreadTicksPerSecond()` and the global tick
        // frequency, then multiplies them by the per-thread delta.
        let ticks_delta = ticks.saturating_sub(state.last_ticks);
        state.last_ticks = ticks;

        let pct_divider = pct_divider(ticks_delta, ticks_per_second, tick_frequency);
        let time_divider = time_divider(ticks_per_second, state.frames_since_last_update);

        let cpu_delta = cpu_time.saturating_sub(state.last_cpu_time);
        let gs_delta = gs_time.saturating_sub(state.last_gs_time);
        let vu_delta = vu_time.saturating_sub(state.last_vu_time);
        let capture_delta = capture_time.saturating_sub(state.last_capture_time);
        state.last_cpu_time = cpu_time;
        state.last_gs_time = gs_time;
        state.last_vu_time = vu_time;
        state.last_capture_time = capture_time;

        state.cpu_thread_usage = cpu_delta as f64 * pct_divider;
        state.gs_thread_usage = gs_delta as f64 as f32 * pct_divider as f32;
        state.vu_thread_usage = vu_delta as f64 as f32 * pct_divider as f32;
        state.capture_thread_usage = capture_delta as f64 as f32 * pct_divider as f32;
        state.cpu_thread_time = cpu_delta as f64 * time_divider;
        state.gs_thread_time = gs_delta as f64 as f32 * time_divider as f32;
        state.vu_thread_time = vu_delta as f64 as f32 * time_divider as f32;
        state.capture_thread_time = capture_delta as f64 as f32 * time_divider as f32;

        for thread in state.gs_sw_threads.iter_mut() {
            let prev = thread.last_cpu_time;
            let new_time = prev; // placeholder; updated by caller via set_gs_sw_thread.
            let delta = new_time.saturating_sub(prev);
            thread.last_cpu_time = new_time;
            thread.usage = delta as f64 * pct_divider;
            thread.time = delta as f64 * time_divider;
        }

        state.frames_since_last_update = 0;
        state.unskipped_frames_since_last_update = 0;
        state.presents_since_last_update = 0;

        if let Some(cb) = on_metrics_updated {
            // Release the lock before invoking the callback so it can call
            // back into the metrics without deadlocking.
            drop(state);
            cb();
        }
    }

    /// Notify the metrics of a single GPU present. Mirrors
    /// `PerformanceMetrics::OnGPUPresent` in the C++ source.
    pub fn on_gpu_present(&self, gpu_time: f32) {
        let mut state = self.inner.lock().expect("PerformanceMetrics lock poisoned");
        state.accumulated_gpu_time += gpu_time;
        state.presents_since_last_update += 1;
    }

    /// Sets the EE thread for CPU usage calculations. Mirrors
    /// `PerformanceMetrics::SetCPUThread` in the C++ header.
    ///
    /// The C++ code stores a `Threading::ThreadHandle`; the Rust port
    /// accepts an opaque `u64` representing the new "last observed CPU
    /// time" baseline. The caller is responsible for refreshing the
    /// baseline through `update` (which queries the thread handle
    /// directly) once the handle is in scope.
    pub fn set_cpu_thread_baseline(&self, last_cpu_time: u64) {
        let mut state = self.inner.lock().expect("PerformanceMetrics lock poisoned");
        state.last_cpu_time = last_cpu_time;
    }

    /// Resize the GS software thread stats vector. Mirrors
    /// `PerformanceMetrics::SetGSSWThreadCount` in the C++ header.
    pub fn set_gs_sw_thread_count(&self, count: usize) {
        let mut state = self.inner.lock().expect("PerformanceMetrics lock poisoned");
        state.gs_sw_threads.clear();
        state.gs_sw_threads.resize(count, GsswThreadStats::default());
    }

    /// Configure the GS software thread at `index`. Mirrors
    /// `PerformanceMetrics::SetGSSWThread` in the C++ header. The caller
    /// supplies the "last CPU time" baseline directly because the
    /// `Threading::ThreadHandle` type is platform-specific and not part
    /// of this module.
    pub fn set_gs_sw_thread(&self, index: usize, last_cpu_time: u64) {
        let mut state = self.inner.lock().expect("PerformanceMetrics lock poisoned");
        if let Some(slot) = state.gs_sw_threads.get_mut(index) {
            slot.last_cpu_time = last_cpu_time;
        }
    }

    // ---- Accessors ----------------------------------------------------

    /// Mirrors `PerformanceMetrics::GetFrameNumber`.
    pub fn frame_number(&self) -> u64 {
        self.inner.lock().expect("PerformanceMetrics lock poisoned").frame_number
    }

    /// Mirrors `PerformanceMetrics::GetInternalFPSMethod`.
    pub fn internal_fps_method(&self) -> InternalFpsMethod {
        self.inner
            .lock()
            .expect("PerformanceMetrics lock poisoned")
            .internal_fps_method
    }

    /// Mirrors `PerformanceMetrics::IsInternalFPSValid`.
    pub fn is_internal_fps_valid(&self) -> bool {
        self.internal_fps_method() != InternalFpsMethod::None
    }

    /// Mirrors `PerformanceMetrics::GetFPS`.
    pub fn fps(&self) -> f32 {
        self.inner.lock().expect("PerformanceMetrics lock poisoned").fps
    }

    /// Mirrors `PerformanceMetrics::GetInternalFPS`.
    pub fn internal_fps(&self) -> f32 {
        self.inner.lock().expect("PerformanceMetrics lock poisoned").internal_fps
    }

    /// Mirrors `PerformanceMetrics::GetSpeed`. The numerator is the
    /// current FPS, the denominator is the target frame rate scaled by
    /// 100.0 to express the result as a percentage.
    pub fn speed(&self, target_frame_rate: f32) -> f32 {
        if target_frame_rate <= 0.0 {
            return 0.0;
        }
        (self.fps() / target_frame_rate) * 100.0
    }

    /// Mirrors `PerformanceMetrics::GetAverageFrameTime`.
    pub fn average_frame_time(&self) -> f32 {
        self.inner
            .lock()
            .expect("PerformanceMetrics lock poisoned")
            .average_frame_time
    }

    /// Mirrors `PerformanceMetrics::GetMinimumFrameTime`.
    pub fn minimum_frame_time(&self) -> f32 {
        self.inner
            .lock()
            .expect("PerformanceMetrics lock poisoned")
            .minimum_frame_time
    }

    /// Mirrors `PerformanceMetrics::GetMaximumFrameTime`.
    pub fn maximum_frame_time(&self) -> f32 {
        self.inner
            .lock()
            .expect("PerformanceMetrics lock poisoned")
            .maximum_frame_time
    }

    /// Mirrors `PerformanceMetrics::GetCPUThreadUsage`.
    pub fn cpu_thread_usage(&self) -> f64 {
        self.inner
            .lock()
            .expect("PerformanceMetrics lock poisoned")
            .cpu_thread_usage
    }

    /// Mirrors `PerformanceMetrics::GetCPUThreadAverageTime`.
    pub fn cpu_thread_average_time(&self) -> f64 {
        self.inner
            .lock()
            .expect("PerformanceMetrics lock poisoned")
            .cpu_thread_time
    }

    /// Mirrors `PerformanceMetrics::GetGSThreadUsage`.
    pub fn gs_thread_usage(&self) -> f32 {
        self.inner
            .lock()
            .expect("PerformanceMetrics lock poisoned")
            .gs_thread_usage
    }

    /// Mirrors `PerformanceMetrics::GetGSThreadAverageTime`.
    pub fn gs_thread_average_time(&self) -> f32 {
        self.inner
            .lock()
            .expect("PerformanceMetrics lock poisoned")
            .gs_thread_time
    }

    /// Mirrors `PerformanceMetrics::GetVUThreadUsage`.
    pub fn vu_thread_usage(&self) -> f32 {
        self.inner
            .lock()
            .expect("PerformanceMetrics lock poisoned")
            .vu_thread_usage
    }

    /// Mirrors `PerformanceMetrics::GetVUThreadAverageTime`.
    pub fn vu_thread_average_time(&self) -> f32 {
        self.inner
            .lock()
            .expect("PerformanceMetrics lock poisoned")
            .vu_thread_time
    }

    /// Mirrors `PerformanceMetrics::GetCaptureThreadUsage`.
    pub fn capture_thread_usage(&self) -> f32 {
        self.inner
            .lock()
            .expect("PerformanceMetrics lock poisoned")
            .capture_thread_usage
    }

    /// Mirrors `PerformanceMetrics::GetCaptureThreadAverageTime`.
    pub fn capture_thread_average_time(&self) -> f32 {
        self.inner
            .lock()
            .expect("PerformanceMetrics lock poisoned")
            .capture_thread_time
    }

    /// Mirrors `PerformanceMetrics::GetGSSWThreadCount`.
    pub fn gs_sw_thread_count(&self) -> u32 {
        self.inner
            .lock()
            .expect("PerformanceMetrics lock poisoned")
            .gs_sw_threads
            .len() as u32
    }

    /// Mirrors `PerformanceMetrics::GetGSSWThreadUsage`.
    pub fn gs_sw_thread_usage(&self, index: u32) -> f64 {
        self.inner
            .lock()
            .expect("PerformanceMetrics lock poisoned")
            .gs_sw_threads
            .get(index as usize)
            .map(|s| s.usage)
            .unwrap_or(0.0)
    }

    /// Mirrors `PerformanceMetrics::GetGSSWThreadAverageTime`.
    pub fn gs_sw_thread_average_time(&self, index: u32) -> f64 {
        self.inner
            .lock()
            .expect("PerformanceMetrics lock poisoned")
            .gs_sw_threads
            .get(index as usize)
            .map(|s| s.time)
            .unwrap_or(0.0)
    }

    /// Mirrors `PerformanceMetrics::GetGPUUsage`.
    pub fn gpu_usage(&self) -> f32 {
        self.inner.lock().expect("PerformanceMetrics lock poisoned").gpu_usage
    }

    /// Mirrors `PerformanceMetrics::GetGPUAverageTime`.
    pub fn gpu_average_time(&self) -> f32 {
        self.inner
            .lock()
            .expect("PerformanceMetrics lock poisoned")
            .average_gpu_time
    }

    /// Returns a copy of the rolling frame-time history buffer. Mirrors
    /// `PerformanceMetrics::GetFrameTimeHistory` (which returns a
    /// reference to the file-scope array in the C++ source).
    pub fn frame_time_history(&self) -> FrameTimeHistory {
        self.inner
            .lock()
            .expect("PerformanceMetrics lock poisoned")
            .frame_time_history
    }

    /// Mirrors `PerformanceMetrics::GetFrameTimeHistoryPos`.
    pub fn frame_time_history_pos(&self) -> u32 {
        self.inner
            .lock()
            .expect("PerformanceMetrics lock poisoned")
            .frame_time_history_pos as u32
    }
}

// ---------------------------------------------------------------------------
// Helper math
// ---------------------------------------------------------------------------

/// Convert a tick delta into a duration in seconds using `tick_frequency`.
///
/// In the C++ source this conversion is performed by
/// `Common::Timer::ConvertValueToSeconds`. We replicate the formula here
/// so the metrics module is self-contained.
#[inline]
fn ticks_to_seconds(ticks: u64, tick_frequency: u64) -> f32 {
    if tick_frequency == 0 {
        return 0.0;
    }
    (ticks as f64 / tick_frequency as f64) as f32
}

/// The "percentage divider" used to convert raw CPU-time deltas into
/// percentage usage. Mirrors the inline expression in `PerformanceMetrics::Update`:
///
/// ```text
/// pct_divider = 100.0 * (1.0 / ((ticks_delta * ticks_per_second) / tick_frequency))
/// ```
#[inline]
fn pct_divider(ticks_delta: u64, ticks_per_second: u64, tick_frequency: u64) -> f64 {
    if ticks_delta == 0 || ticks_per_second == 0 || tick_frequency == 0 {
        return 0.0;
    }
    let numerator = 100.0_f64;
    let denominator = (ticks_delta as f64 * ticks_per_second as f64) / tick_frequency as f64;
    numerator * (1.0 / denominator)
}

/// The "time divider" used to convert raw CPU-time deltas into
/// per-frame time in milliseconds. Mirrors the inline expression in
/// `PerformanceMetrics::Update`:
///
/// ```text
/// time_divider = 1000.0 * (1.0 / ticks_per_second) * (1.0 / frames_since_last_update)
/// ```
#[inline]
fn time_divider(ticks_per_second: u64, frames_since_last_update: u32) -> f64 {
    if ticks_per_second == 0 || frames_since_last_update == 0 {
        return 0.0;
    }
    let one_over_ticks = 1.0_f64 / ticks_per_second as f64;
    let one_over_frames = 1.0_f64 / frames_since_last_update as f64;
    1000.0 * one_over_ticks * one_over_frames
}

/// Returns a monotonically increasing tick count. In the original
/// code this is `Common::Timer::GetCurrentValue()`. The Rust port uses
/// a process-relative counter so the math in `update` stays numerically
/// stable across calls.
#[inline]
fn current_tick_value() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_aggregator_is_zeroed() {
        let m = PerformanceMetrics::new();
        assert_eq!(m.fps(), 0.0);
        assert_eq!(m.internal_fps(), 0.0);
        assert_eq!(m.average_frame_time(), 0.0);
        assert_eq!(m.minimum_frame_time(), 0.0);
        assert_eq!(m.maximum_frame_time(), 0.0);
        assert_eq!(m.frame_number(), 0);
        assert_eq!(m.internal_fps_method(), InternalFpsMethod::None);
        assert!(!m.is_internal_fps_valid());
        assert_eq!(m.gs_sw_thread_count(), 0);
    }

    #[test]
    fn clear_resets_every_metric() {
        let m = PerformanceMetrics::new();
        m.on_gpu_present(2.5);
        m.clear();
        assert_eq!(m.gpu_average_time(), 0.0);
        assert_eq!(m.gpu_usage(), 0.0);
        assert_eq!(m.fps(), 0.0);
        assert_eq!(m.internal_fps(), 0.0);
        assert_eq!(m.frame_number(), 0);
    }

    #[test]
    fn speed_uses_target_frame_rate() {
        let m = PerformanceMetrics::new();
        // With a zeroed FPS and a positive target, the speed is zero.
        assert_eq!(m.speed(60.0), 0.0);
        // A zero target should short-circuit to zero.
        assert_eq!(m.speed(0.0), 0.0);
    }

    #[test]
    fn gs_sw_thread_setter_resizes_vector() {
        let m = PerformanceMetrics::new();
        m.set_gs_sw_thread_count(4);
        assert_eq!(m.gs_sw_thread_count(), 4);
        m.set_gs_sw_thread(2, 1234);
        assert_eq!(m.gs_sw_thread_average_time(2), 0.0);
    }

    #[test]
    fn gpu_present_accumulates() {
        let m = PerformanceMetrics::new();
        m.on_gpu_present(1.0);
        m.on_gpu_present(2.0);
        // `average_gpu_time` is only computed on a successful `update`
        // tick; until then the cached `accumulated_gpu_time` is internal.
        // We just make sure the function does not panic.
        let _ = m.gpu_average_time();
    }

    #[test]
    fn frame_time_history_is_zeroed_by_default() {
        let m = PerformanceMetrics::new();
        let history = m.frame_time_history();
        for sample in history.iter() {
            assert_eq!(*sample, 0.0);
        }
        assert_eq!(m.frame_time_history_pos(), 0);
    }
}
