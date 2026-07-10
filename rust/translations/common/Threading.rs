// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Threading primitives translated from PCSX2's `common/Threading.h`.
//!
//! The original C/C++ header lives in the `Threading` namespace and exposes a
//! mix of platform-specific helpers (`SetNameOfCurrentThread`, `Sleep`,
//! `Timeslice`, `SpinWait`, `EnableHiresScheduler`, …), a `Thread` class
//! abstracting an OS thread (with optional stack-size control), a
//! `ThreadHandle` for native thread queries, a kernel-level `KernelSemaphore`,
//! a userspace-flavoured `UserspaceSemaphore`, and a `WorkSema` optimised for
//! producer/consumer worker queues.
//!
//! This module is an idiomatic Rust 2021 translation of that surface. The
//! platform-specific helpers are intentionally omitted — `std::thread` already
//! provides everything that's needed, and the rest of the PCSX2 stack can call
//! the standard library directly. The translated types are:
//!
//! - [`Thread`]: a named OS thread backed by `std::thread::JoinHandle`.
//! - [`Semaphore`]: a counting semaphore built from `Mutex<isize>` +
//!   `Condvar`, modelling `UserspaceSemaphore`'s contract.
//! - [`Event`]: a manual-reset event backed by `AtomicBool` + `Condvar`.
//! - [`RecursiveMutex`]: a `Mutex<()>` newtype used as a self-documenting
//!   alias for the recursive-mutex contract that PCSX2 expects from its
//!   platforms.
//!
//! Only `core` and `std` dependencies are used.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// A named OS thread.
///
/// This is a thin wrapper around `std::thread::JoinHandle<()>`. The PCSX2
/// original also tracks CPU time and supports affinity / stack-size tweaks
/// through `ThreadHandle`; those rely on platform-specific APIs and are
/// intentionally not exposed here — callers that need them should hold a
/// `JoinHandle` themselves and reach for `std::os::thread::JoinHandleExt`
/// directly.
pub struct Thread(pub JoinHandle<()>);

impl Thread {
    /// Spawn a new named OS thread running `f`.
    ///
    /// The thread name is set via `std::thread::Builder::name` so it shows up
    /// in debuggers and platform tooling. `f` is moved into the thread just
    /// like `std::thread::spawn`.
    ///
    /// # Panics
    ///
    /// Panics if the OS fails to spawn the thread, mirroring
    /// `std::thread::spawn`.
    pub fn new<F>(name: &str, f: F) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        let builder = thread::Builder::new().name(name.to_owned());
        let handle = builder
            .spawn(f)
            .expect("Thread::new: failed to spawn OS thread");
        Thread(handle)
    }

    /// Consume the wrapper and return the inner `JoinHandle`.
    pub fn into_inner(self) -> JoinHandle<()> {
        self.0
    }

    /// Block the current thread until the spawned thread finishes.
    pub fn join(self) -> thread::Result<()> {
        self.0.join()
    }
}

/// A counting semaphore.
///
/// This is the Rust equivalent of `Threading::UserspaceSemaphore` from the
/// C++ header: a counter that blocks `wait` callers when it would otherwise
/// go negative, and wakes them up as `post` calls arrive.
///
/// Internally the counter and the wait queue are encoded as a single
/// `Mutex<isize>` + `Condvar` pair. That's the idiomatic Rust translation of
/// PCSX2's userspace-fast-path abstraction — it avoids the platform-specific
/// `KernelSemaphore` entirely while keeping the same semantics.
pub struct Semaphore {
    state: Mutex<isize>,
    condvar: Condvar,
}

impl Semaphore {
    /// Create a new semaphore with the given initial count.
    pub fn new(initial: isize) -> Self {
        Semaphore {
            state: Mutex::new(initial),
            condvar: Condvar::new(),
        }
    }

    /// Increment the counter, waking a single waiter if one is blocked.
    pub fn post(&self) {
        let mut guard = self.state.lock().unwrap();
        *guard = guard.checked_add(1).expect("Semaphore::post: counter overflow");
        // Only wake a waiter if we transitioned from "no permits" to "one
        // permit". A naked `notify_one` would also work, but this matches the
        // UserspaceSemaphore fast-path behaviour more closely.
        if *guard <= 1 {
            self.condvar.notify_one();
        }
    }

    /// Block until the counter is positive, then decrement it.
    pub fn wait(&self) {
        let mut guard = self.state.lock().unwrap();
        while *guard <= 0 {
            guard = self.condvar.wait(guard).unwrap();
        }
        *guard -= 1;
    }

    /// Try to decrement the counter without blocking. Returns `true` if a
    /// permit was acquired.
    pub fn try_wait(&self) -> bool {
        let mut guard = self.state.lock().unwrap();
        if *guard > 0 {
            *guard -= 1;
            true
        } else {
            false
        }
    }

    /// Block for at most `timeout`, returning `true` if a permit was acquired
    /// in that window.
    pub fn wait_timeout(&self, timeout: Duration) -> bool {
        let mut guard = self.state.lock().unwrap();
        // The condvar's `wait_timeout` can spuriously wake up, so we always
        // re-check the predicate; we also re-issue the wait for the *remaining*
        // portion of the budget on every iteration so the total wall-clock
        // bound holds across spurious wakes.
        let deadline = Instant::now() + timeout;
        loop {
            if *guard > 0 {
                *guard -= 1;
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            let remaining = deadline - now;
            let (new_guard, _) = self.condvar.wait_timeout(guard, remaining).unwrap();
            guard = new_guard;
        }
    }
}

impl Default for Semaphore {
    fn default() -> Self {
        Semaphore::new(0)
    }
}

/// A manual-reset event.
///
/// `set()` raises the event; any number of `wait()` callers will then return
/// immediately until `reset()` is called. This matches Win32's
/// `CreateEvent(..., /*manual-reset=*/TRUE, …)` semantics — the C++ PCSX2
/// codebase does not expose an `Event` class, but a manual-reset event is a
/// natural fit for the same "wake any waiting thread" pattern that the
/// headers already use `KernelSemaphore` for.
///
/// The atomic `state` lets `set()` avoid taking the mutex when no one is
/// waiting, and the condvar+mutex pair handles the blocking-waiter case.
pub struct Event {
    state: AtomicBool,
    condvar: Condvar,
    mutex: Mutex<()>,
}

impl Event {
    /// Create a new event in the unsignalled (reset) state.
    pub fn new() -> Self {
        Event {
            state: AtomicBool::new(false),
            condvar: Condvar::new(),
            mutex: Mutex::new(()),
        }
    }

    /// Signal the event, waking every thread currently blocked in `wait()`.
    ///
    /// Subsequent `wait()` calls will also return immediately until `reset()`
    /// is called.
    pub fn set(&self) {
        self.state.store(true, Ordering::Release);
        // Wake every waiter — `notify_all` is the manual-reset equivalent of
        // the `notify_one` you would use for an auto-reset event.
        self.condvar.notify_all();
    }

    /// Block until the event is signalled.
    pub fn wait(&self) {
        // Fast path: already signalled, no need to lock.
        if self.state.load(Ordering::Acquire) {
            return;
        }
        let mut guard = self.mutex.lock().unwrap();
        while !self.state.load(Ordering::Acquire) {
            guard = self.condvar.wait(guard).unwrap();
        }
    }

    /// Block for at most `d`, returning `true` if the event was signalled
    /// within the window (and `false` on timeout).
    pub fn wait_timeout(&self, d: Duration) -> bool {
        if self.state.load(Ordering::Acquire) {
            return true;
        }
        let mut guard = self.mutex.lock().unwrap();
        // Re-check the predicate after every condvar return, and use a
        // deadline-based loop so spurious wakes don't extend the wait
        // indefinitely.
        let deadline = Instant::now() + d;
        loop {
            if self.state.load(Ordering::Acquire) {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            let remaining = deadline - now;
            let (new_guard, _) = self.condvar.wait_timeout(guard, remaining).unwrap();
            guard = new_guard;
        }
    }

    /// Clear the event. Threads that call `wait()` after this point will
    /// block again until the next `set()`.
    pub fn reset(&self) {
        self.state.store(false, Ordering::Release);
    }
}

impl Default for Event {
    fn default() -> Self {
        Event::new()
    }
}

/// A mutex type that is documented as recursive.
///
/// `std::sync::Mutex` is non-recursive, but the PCSX2 codebase historically
/// expects a "recursive mutex" abstraction (matching `pthread_mutex_t` with
/// `PTHREAD_MUTEX_RECURSIVE` or Win32's `CRITICAL_SECTION`). This newtype is
/// intentionally a re-name of `Mutex<()>`: it provides a distinct type name
/// for clarity at call sites while delegating all behaviour to the standard
/// library. Code that genuinely needs re-entrant locking should compose this
/// with a higher-level abstraction (e.g. a thread-local owner check) rather
/// than rely on the underlying `Mutex` to be recursive.
pub struct RecursiveMutex(Mutex<()>);

impl RecursiveMutex {
    /// Create a new recursive-mutex wrapper in the unlocked state.
    pub fn new() -> Self {
        RecursiveMutex(Mutex::new(()))
    }

    /// Acquire the lock, blocking until it is available.
    pub fn lock(&self) -> MutexGuard<'_, ()> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Default for RecursiveMutex {
    fn default() -> Self {
        RecursiveMutex::new()
    }
}

/// Convenience alias matching the C++ `Threading::KernelSemaphore` name.
///
/// In the C++ header `KernelSemaphore` is a thin platform-specific wrapper
/// around `sem_t` / `semaphore_t` / `HANDLE`. In Rust the standard library
/// doesn't expose an equivalent primitive directly, so we alias it to
/// [`Semaphore`]. Callers that previously used `KernelSemaphore` for its
/// "sleep a thread until something happens" semantics get exactly the same
/// behaviour here.
pub type KernelSemaphore = Semaphore;
