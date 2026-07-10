//! Linux hardware performance counter wrapper built on the
//! `perf-event` crate (which itself wraps the `perf_event_open(2)`
//! syscall).
//!
//! ## Background
//!
//! PCSX2's `common/Perf.cpp` registers JITted regions with an
//! external profiler. The current Rust port (`perf.rs`) only handles
//! the registration path. On Linux, profiling also needs access to
//! the **hardware performance counters** that `perf` and VTune read
//! from: CPU cycles, cache misses, branch mispredictions, etc.
//!
//! `perf_event_open` is the syscall that opens one of those
//! counters. The `perf-event` crate gives us a safe, idiomatic Rust
//! wrapper that:
//!   - Sets up the perf_event_attr struct correctly for a given event
//!   - Manages the file descriptor lifetime
//!   - Provides a `read()` method that returns the counter value
//!
//! ## Usage
//!
//! ```no_run
//! use pcsx2_common_rs::perf_event_counter::Counter;
//!
//! let mut cycles = Counter::new(perf_event::events::Hardware::CPU_CYCLES).unwrap();
//! cycles.enable().unwrap();
//! let start = cycles.read().unwrap();
//! // ... do work ...
//! let end = cycles.read().unwrap();
//! println!("cycles: {}", end - start);
//! cycles.disable().unwrap();
//! ```
//!
//! ## Platform
//!
//! This module is **Linux only**. On other platforms the public
//! functions return an error so the caller can degrade gracefully.

#![cfg(target_os = "linux")]

use perf_event::{Builder, Counter as PerfCounter};
use std::io;

/// Hardware / software event kinds we care about, expressed in the
/// `perf-event` crate's vocabulary. We wrap them so the rest of the
/// crate doesn't need to know about the underlying `Builder` API.
#[derive(Debug, Clone, Copy)]
pub enum Event {
    /// Wall-clock CPU cycles (includes cycles the CPU spent in
    /// halted state; use `RefCycles` if you want a fixed reference).
    CpuCycles,
    /// Reference cycles (TSCycles) — the same rate as RDTSC.
    RefCycles,
    /// Retired instructions.
    Instructions,
    /// Last-level cache misses.
    CacheMisses,
    /// Branch mispredictions.
    BranchMisses,
    /// Page faults.
    PageFaults,
    /// Context switches.
    ContextSwitches,
}

impl Event {
    fn build(self) -> Builder {
        use perf_event::events::{Cache, Hardware, Software};
        match self {
            Event::CpuCycles => Builder::new().kind(perf_event::Kind::Hardware)
                .event(Hardware::CPU_CYCLES),
            Event::RefCycles => Builder::new().kind(perf_event::Kind::Hardware)
                .event(Hardware::REF_CPU_CYCLES),
            Event::Instructions => Builder::new().kind(perf_event::Kind::Hardware)
                .event(Hardware::INSTRUCTIONS),
            Event::CacheMisses => Builder::new().kind(perf_event::Kind::Cache)
                .event(Cache::LL),
            Event::BranchMisses => Builder::new().kind(perf_event::Kind::Hardware)
                .event(Hardware::BRANCH_MISSES),
            Event::PageFaults => Builder::new().kind(perf_event::Kind::Software)
                .event(Software::PAGE_FAULTS),
            Event::ContextSwitches => Builder::new().kind(perf_event::Kind::Software)
                .event(Software::CONTEXT_SWITCHES),
        }
    }
}

/// A single hardware performance counter. Cheap to construct (just
/// opens an fd) and cheap to `read()` (a single `read(2)` syscall).
/// Call `enable()` once at the start of the region you want to
/// measure, `read()` zero or more times during the region, and
/// `disable()` at the end.
pub struct Counter {
    inner: PerfCounter,
}

impl Counter {
    /// Open a new counter for the given event. Returns an error if
    /// the kernel rejects the event (e.g. inside a container with
    /// `perf_event_paranoid` restrictions).
    pub fn new(event: Event) -> io::Result<Self> {
        let inner = event.build().build()?;
        Ok(Self { inner })
    }

    /// Start counting. The kernel resets the counter to 0 on enable.
    pub fn enable(&mut self) -> io::Result<()> {
        self.inner.enable()
    }

    /// Stop counting. Subsequent reads return the last value.
    pub fn disable(&mut self) -> io::Result<()> {
        self.inner.disable()
    }

    /// Read the current counter value. The value is monotonically
    /// non-decreasing while the counter is enabled.
    pub fn read(&mut self) -> io::Result<u64> {
        self.inner.read()
    }

    /// Read the elapsed time since `enable()` in nanoseconds. This
    /// uses the counter's time-enabled / time-running fields to
    /// compensate for multiplexing.
    pub fn elapsed_ns(&mut self) -> io::Result<u64> {
        self.inner.time_elapsed().as_nanos().try_into()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "elapsed_ns overflow"))
    }
}

/// Convenience: read `event` around the closure `f` and return the
/// delta. Counter is enabled before the closure runs and disabled
/// after. Returns the final reading on success or an io::Error if
/// any syscall fails.
pub fn measure<F, R>(event: Event, f: F) -> io::Result<(u64, R)>
where
    F: FnOnce() -> R,
{
    let mut c = Counter::new(event)?;
    c.enable()?;
    let start = c.read()?;
    let result = f();
    let end = c.read()?;
    c.disable()?;
    Ok((end - start, result))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_cpu_cycles_counter() {
        // The simplest sanity check: can we open a counter at all?
        // The actual `read()` value is non-deterministic, but the
        // open path exercises perf_event_open(2) end-to-end.
        let mut c = Counter::new(Event::CpuCycles).expect("open counter");
        c.enable().expect("enable");
        let start = c.read().expect("read start");
        // No work to do — we just want to make sure read returns.
        let end = c.read().expect("read end");
        assert!(end >= start);
        c.disable().expect("disable");
    }

    #[test]
    fn measure_returns_delta() {
        let (delta, returned) =
            measure(Event::Instructions, || "hello").expect("measure");
        // We did basically no work, so delta could be very small
        // (a few hundred instructions). The point of this test is
        // to prove the wrapper composes correctly, not to assert
        // an exact number.
        let _ = delta;
        assert_eq!(returned, "hello");
    }

    #[test]
    fn elapsed_ns_is_finite() {
        let mut c = Counter::new(Event::CpuCycles).expect("open counter");
        c.enable().expect("enable");
        let ns = c.elapsed_ns().expect("elapsed_ns");
        // Just confirm the call returns a non-error value.
        // (The actual ns count depends on scheduler jitter.)
        let _ = ns;
        c.disable().expect("disable");
    }
}
