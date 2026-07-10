// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! GPU readback spin manager.
//!
//! Keeps a list of registered callbacks and provides a wakeup mechanism used
//! by the GPU readback path. Callbacks are invoked when
//! [`ReadbackSpinManager::notify`] is called, which signals that readback
//! work has progressed and any waiters should re-evaluate their state.
//!
//! The internal state is guarded by a single [`std::sync::Mutex`], making the
//! type safe to share between threads (for example, the GPU thread calling
//! `notify` and a CPU thread registering/unregistering callbacks).

use std::sync::Mutex;

/// A manager for callbacks that need to be notified when the GPU readback
/// path makes progress.
///
/// Consumers register `Fn() + Send + 'static` callbacks via
/// [`ReadbackSpinManager::register_callback`] and receive a stable `u64` id.
/// The id can later be used with [`ReadbackSpinManager::unregister_callback`]
/// to remove the callback. Calling [`ReadbackSpinManager::notify`] invokes
/// every currently-registered callback in registration order.
pub struct ReadbackSpinManager {
    callbacks: Mutex<Vec<(u64, Box<dyn Fn() + Send + 'static>)>>,
    next_id: Mutex<u64>,
}

impl ReadbackSpinManager {
    /// Create a new, empty `ReadbackSpinManager`.
    pub fn new() -> Self {
        Self {
            callbacks: Mutex::new(Vec::new()),
            next_id: Mutex::new(0),
        }
    }

    /// Register a new callback to be invoked on [`Self::notify`].
    ///
    /// Returns a unique id that can be passed to [`Self::unregister_callback`]
    /// to remove the callback. Ids are assigned monotonically and wrap on
    /// overflow.
    pub fn register_callback(&mut self, cb: impl Fn() + Send + 'static) -> u64 {
        let mut next_id_guard = self
            .next_id
            .lock()
            .expect("ReadbackSpinManager next_id mutex poisoned");
        let id = *next_id_guard;
        *next_id_guard = next_id_guard.wrapping_add(1);
        drop(next_id_guard);

        let mut callbacks_guard = self
            .callbacks
            .lock()
            .expect("ReadbackSpinManager callbacks mutex poisoned");
        callbacks_guard.push((id, Box::new(cb)));

        id
    }

    /// Remove a previously-registered callback by its id.
    ///
    /// If the id does not match any registered callback, this is a no-op.
    pub fn unregister_callback(&mut self, id: u64) {
        let mut callbacks_guard = self
            .callbacks
            .lock()
            .expect("ReadbackSpinManager callbacks mutex poisoned");
        if let Some(pos) = callbacks_guard.iter().position(|(cb_id, _)| *cb_id == id) {
            callbacks_guard.swap_remove(pos);
        }
    }

    /// Notify all currently-registered callbacks in registration order.
    ///
    /// The lock is held for the duration of the iteration; callbacks must
    /// not call back into this manager (e.g. to register or unregister)
    /// while running, as that would deadlock. A future revision could
    /// snapshot the list first if reentrant notification is required.
    pub fn notify(&self) {
        let guard = self
            .callbacks
            .lock()
            .expect("ReadbackSpinManager callbacks mutex poisoned");
        for (_, cb) in guard.iter() {
            (cb)();
        }
    }
}

impl Default for ReadbackSpinManager {
    fn default() -> Self {
        Self::new()
    }
}
