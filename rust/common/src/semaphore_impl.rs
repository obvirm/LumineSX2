// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
// Counting semaphore — delegates to the canonical `threading::Semaphore`.
//
// NOTE: This file exists as a separate module to mirror the C++ file
// `common/Semaphore.cpp` which provides `Threading::KernelSemaphore`.
// The actual implementation lives in `threading.rs`; this module re-exports
// it so that `pub use semaphore_impl::*;` in `lib.rs` works without
// duplicating FFI symbols (which caused the previous conflict).
//
// FFI symbols (`pcsx2_semaphore_*`) are exported exclusively from
// `threading.rs`. This module provides only pure-Rust API surface.
//
// The original C++ uses platform-specific kernel semaphores
// (`CreateSemaphore` / `sem_init`) via `Threading::KernelSemaphore`.
// The Rust `Semaphore` uses a portable `Condvar` + `Mutex<i32>`
// representation that has identical observable semantics: an internal
// signed counter where `post` increments and `wait` blocks while the
// counter is non-positive, atomically decrementing on wake.

/// Portable counting semaphore.
///
/// Re-exported from `threading::Semaphore` to avoid duplicate FFI
/// symbols. See `threading::Semaphore` for full documentation.
pub use crate::threading::Semaphore;

/// Alias matching the C++ naming convention `KernelSemaphore`.
pub type KernelSemaphore = Semaphore;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn post_then_wait_succeeds() {
        let sem = Semaphore::new(0);
        sem.post(1);
        assert!(sem.try_wait());
    }

    #[test]
    fn wait_without_post_times_out() {
        let sem = Semaphore::new(0);
        assert!(!sem.wait_timeout(10));
    }

    #[test]
    fn post_wakes_waiter() {
        let sem = Arc::new(Semaphore::new(0));
        let sem2 = Arc::clone(&sem);
        let started = Arc::new(std::sync::Mutex::new(false));
        let started2 = Arc::clone(&started);

        let h = thread::spawn(move || {
            *started2.lock().unwrap() = true;
            sem2.wait_timeout(5000)
        });

        while !*started.lock().unwrap() {
            thread::yield_now();
        }
        thread::sleep(Duration::from_millis(10));
        sem.post(1);
        assert!(h.join().unwrap());
    }

    #[test]
    fn initial_count_consumed_by_waits() {
        let sem = Semaphore::new(2);
        assert!(sem.try_wait());
        assert!(sem.try_wait());
        assert!(!sem.try_wait());
    }

    #[test]
    fn post_multiple_wakes_multiple() {
        let sem = Semaphore::new(0);
        sem.post(3);
        assert!(sem.try_wait());
        assert!(sem.try_wait());
        assert!(sem.try_wait());
        assert!(!sem.try_wait());
    }

    #[test]
    fn kernel_semaphore_alias() {
        let ks: KernelSemaphore = Semaphore::new(1);
        assert!(ks.try_wait());
        assert!(!ks.try_wait());
    }
}
