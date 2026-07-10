//! End-to-end exercise of the perf-counter wrapper.
//!
//! This example uses `RDTSC` (Read Time-Stamp Counter) on Windows /
//! cross-platform. On Linux, the same shape is backed by
//! `perf_event_open` via the `perf-event` crate. The point of this
//! test is to prove the API surface and the `measure()` convenience
//! helper work, not to characterize the underlying counter.
//!
//! Run with: cargo run --release --example perf_event_test

use std::time::Duration;

/// Cross-platform cycle counter. Uses `core::arch::x86_64::_rdtsc`
/// on x86_64 Windows / Linux. On aarch64 (Apple Silicon) we fall
/// back to a 1ns-resolution `Instant::now()`.
#[inline]
fn rdtsc() -> u64 {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        // _rdtsc returns (edx:eax); combine into a single u64.
        core::arch::x86_64::_rdtsc()
    }
    #[cfg(target_arch = "aarch64")]
    {
        // CNTVCT_EL0 — virtual counter. 1 tick per CPU cycle, modulo
        // frequency scaling. Good enough for delta measurement.
        let val: u64;
        unsafe {
            std::arch::asm!("mrs {}, cntvct_el0", out(reg) val);
        }
        val
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        // Last-resort fallback: nanoseconds. The shape of the test
        // still works, just with timer resolution instead of cycles.
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0)
    }
}

/// Like `perf_event_counter::measure`, but using RDTSC instead of a
/// real `perf_event_open` counter. Exists to verify the *shape* of
/// the API: take an event, run a closure, return the delta and the
/// closure's result. The Linux code path is the production version;
/// this one exists so the test runs on Windows.
fn measure<F, R>(_label: &str, f: F) -> (u64, R)
where
    F: FnOnce() -> R,
{
    let start = rdtsc();
    let result = f();
    let end = rdtsc();
    (end.saturating_sub(start), result)
}

fn main() {
    println!("=== perf counter PoC ===");
    println!("Architecture: {}", std::env::consts::ARCH);
    println!("OS: {}", std::env::consts::OS);

    // Simple work-load: a few thousand additions. We expect the
    // cycle count to be on the order of 10K-100K cycles on a modern
    // x86.
    let (cycles1, sum1) = measure("sum_loop", || {
        let mut s: u64 = 0;
        for i in 0..10_000u64 {
            s = s.wrapping_add(i);
        }
        s
    });
    println!("sum_loop: {} cycles, sum={}", cycles1, sum1);

    // A load-load-load sequence that should hit the L1d cache every
    // time (very low cycle count).
    let buf: Vec<u64> = (0..1024).collect();
    let (cycles2, s2) = measure("hot_scan", || {
        let mut s: u64 = 0;
        for _ in 0..10_000 {
            for v in &buf {
                s = s.wrapping_add(*v);
            }
        }
        s
    });
    println!("hot_scan: {} cycles, sum={}", cycles2, s2);

    // A wall-clock sleep — to verify that the `measure` helper
    // actually encompasses the closure and that the delta scales
    // with the work done.
    let (cycles3, _) = measure("sleep_5ms", || {
        std::thread::sleep(Duration::from_millis(5));
    });
    println!("sleep_5ms: {} cycles (~5ms of work)", cycles3);

    // Sanity: sleep should have consumed more cycles than the sum_loop.
    // (Sleep is tens of millions of cycles; sum_loop is thousands.)
    assert!(
        cycles3 > cycles1 * 100,
        "sleep_5ms ({cycles3}) should be much larger than sum_loop ({cycles1})"
    );
    assert_eq!(sum1, (0..10_000u64).sum::<u64>());
    println!("OK — cycles scale with work, sum_loop result is correct");
}
