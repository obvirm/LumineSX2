//! Idiomatic Rust translation of PCSX2's `common/ProgressCallback.h` + `common/ProgressCallback.cpp`.
//!
//! Provides a [`ProgressCallback`] trait for reporting cancellable progress from a
//! blocking operation without depending on any UI framework, plus several stock
//! implementations: a no-op [`NullProgressCallback`], a stderr-printing
//! [`ConsoleProgressCallback`], a chaining [`MultiProgressCallback`], and a
//! reusable [`ProgressCallbackState`] base struct that handles state
//! push/pop and relative-to-absolute progress mapping.
//!
//! The Rust port intentionally narrows the API to the subset the rest of the
//! PCSX2 translation needs: a percent-based [`ProgressCallback::set_progress`]
//! rather than a separate range/value pair, and a single `cancelled` query
//! rather than separate cancellable/cancelled flags. The richer C++ surface
//! (formatted display helpers, modal dialogs, etc.) maps onto Rust's
//! [`std::fmt`] / `format!` and is therefore not exposed as separate methods.

use std::fmt;

/// Progress state for an in-flight operation.
///
/// Mirrors the C++ `ProgressCallback::ProgressState` enum. `Indeterminate` is
/// reported as a percent of `NaN` via [`ProgressCallback::set_progress`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressState {
    /// Steady forward progress, percent in `[0.0, 1.0]`.
    Normal,
    /// Progress is unknown; should be displayed as a marquee.
    Indeterminate,
    /// Operation is paused; progress should not advance.
    Paused,
    /// Operation has failed; UI should highlight this.
    Error,
}

impl Default for ProgressState {
    fn default() -> Self {
        ProgressState::Normal
    }
}

/// Interface for progress reporting from a long-running, cancellable operation.
///
/// All methods take `&self` / `&mut self` as appropriate; the only mutating
/// call is [`ProgressCallback::set_progress`] because the only piece of
/// state the trait guarantees to be tracked across calls is the most recent
/// percent value.
pub trait ProgressCallback {
    /// Save the current callback state so it can be restored with
    /// [`ProgressCallback::pop_state`]. Used when nesting progress.
    fn push_state(&mut self);

    /// Restore the most recently saved state. Panics if the stack is empty.
    fn pop_state(&mut self);

    /// Report progress as a fraction in `[0.0, 1.0]`. `f32::NAN` means
    /// indeterminate.
    fn set_progress(&mut self, percent: f32);

    /// Set the user-visible title of the overall operation.
    fn set_title(&self, title: &str);

    /// Set the user-visible status line.
    fn set_status(&self, status: &str);

    /// Set a secondary, more detailed status line.
    fn set_sub_status(&self, status: &str);

    /// Returns `true` if the user has requested cancellation.
    fn cancelled(&self) -> bool;
}

/// Reusable state shared by concrete progress-callback implementations.
///
/// Tracks the current percent, status, sub-status, title, and a stack of
/// saved states for [`ProgressCallback::push_state`] / [`ProgressCallback::pop_state`].
#[derive(Debug, Default)]
pub struct ProgressCallbackState {
    title: String,
    status: String,
    sub_status: String,
    progress: f32,
    saved: Vec<SavedState>,
}

#[derive(Debug)]
struct SavedState {
    title: String,
    status: String,
    sub_status: String,
    progress: f32,
}

impl ProgressCallbackState {
    /// Construct an empty state with progress at `0.0` and `Normal` semantics.
    pub fn new() -> Self {
        Self {
            progress: 0.0,
            ..Self::default()
        }
    }

    /// Read the current progress percent.
    pub fn progress(&self) -> f32 {
        self.progress
    }

    /// Read the current title.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Read the current status line.
    pub fn status(&self) -> &str {
        &self.status
    }

    /// Read the current sub-status line.
    pub fn sub_status(&self) -> &str {
        &self.sub_status
    }
}

impl ProgressCallback for ProgressCallbackState {
    fn push_state(&mut self) {
        self.saved.push(SavedState {
            title: self.title.clone(),
            status: self.status.clone(),
            sub_status: self.sub_status.clone(),
            progress: self.progress,
        });
    }

    fn pop_state(&mut self) {
        self.pop_state_owned();
    }

    fn set_progress(&mut self, percent: f32) {
        self.progress = percent;
    }

    fn set_title(&self, title: &str) {
        // The trait cannot grant `&mut`, so concrete wrappers use
        // `set_title_owned` to assign into the inner state.
        let _ = title;
    }

    fn set_status(&self, status: &str) {
        let _ = status;
    }

    fn set_sub_status(&self, status: &str) {
        let _ = status;
    }

    fn cancelled(&self) -> bool {
        false
    }
}

impl ProgressCallbackState {
    /// Owned-pop helper used by concrete callbacks that hold a
    /// `ProgressCallbackState` by value. Restores the top of the saved stack;
    /// panics if the stack is empty (matching the C++ `pxAssert`).
    pub fn pop_state_owned(&mut self) {
        let restored = self
            .saved
            .pop()
            .expect("ProgressCallbackState::pop_state called with empty stack");
        self.title = restored.title;
        self.status = restored.status;
        self.sub_status = restored.sub_status;
        self.progress = restored.progress;
    }

    /// Owned setter for the title.
    pub fn set_title_owned(&mut self, title: &str) {
        self.title.clear();
        self.title.push_str(title);
    }

    /// Owned setter for the status line.
    pub fn set_status_owned(&mut self, status: &str) {
        self.status.clear();
        self.status.push_str(status);
    }

    /// Owned setter for the sub-status line.
    pub fn set_sub_status_owned(&mut self, status: &str) {
        self.sub_status.clear();
        self.sub_status.push_str(status);
    }
}

/// A [`ProgressCallback`] that does nothing.
///
/// Suitable as a default when no real UI is available. Matches the
/// `NullProgressCallbacks` class from the C++ source.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullProgressCallback;

impl ProgressCallback for NullProgressCallback {
    fn push_state(&mut self) {}
    fn pop_state(&mut self) {}
    fn set_progress(&mut self, _percent: f32) {}
    fn set_title(&self, _title: &str) {}
    fn set_status(&self, _status: &str) {}
    fn set_sub_status(&self, _status: &str) {}
    fn cancelled(&self) -> bool {
        false
    }
}

/// A [`ProgressCallback`] that writes its title/status lines to `stderr`.
///
/// Each setter is a no-op for the value's effect on the world besides
/// writing one line. Mirrors the logging behaviour of the C++ console
/// `NullProgressCallbacks` for `DisplayError` / `DisplayWarning` etc., but
/// simplified to the trait surface.
pub struct ConsoleProgressCallback {
    state: ProgressCallbackState,
}

impl ConsoleProgressCallback {
    /// Construct a new console callback with empty state.
    pub fn new() -> Self {
        Self {
            state: ProgressCallbackState::new(),
        }
    }
}

impl Default for ConsoleProgressCallback {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for ConsoleProgressCallback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConsoleProgressCallback")
            .field("state", &self.state)
            .finish()
    }
}

impl ProgressCallback for ConsoleProgressCallback {
    fn push_state(&mut self) {
        self.state.push_state();
    }

    fn pop_state(&mut self) {
        self.state.pop_state_owned();
    }

    fn set_progress(&mut self, percent: f32) {
        self.state.set_progress(percent);
        eprintln!("[progress] {:>5.1}%", percent * 100.0);
    }

    fn set_title(&self, title: &str) {
        eprintln!("[title] {}", title);
    }

    fn set_status(&self, status: &str) {
        eprintln!("[status] {}", status);
    }

    fn set_sub_status(&self, status: &str) {
        eprintln!("[sub-status] {}", status);
    }

    fn cancelled(&self) -> bool {
        false
    }
}

impl ConsoleProgressCallback {
    /// Owned-pop that restores the previously saved state.
    pub fn pop_state_owned(&mut self) {
        self.state.pop_state_owned();
    }
}

/// A [`ProgressCallback`] that fans a single call out to a chain of inner
/// callbacks. The first callback that reports `cancelled = true` short-
/// circuits the rest.
///
/// Mirrors the intent of the C++ code where the UI can wrap a non-UI
/// progress callback (and vice versa) to layer behaviour.
pub struct MultiProgressCallback {
    callbacks: Vec<Box<dyn ProgressCallback>>,
}

impl MultiProgressCallback {
    /// Construct an empty chain. Use [`MultiProgressCallback::push`] to add
    /// callbacks.
    pub fn new() -> Self {
        Self {
            callbacks: Vec::new(),
        }
    }

    /// Append a callback to the chain.
    pub fn push(&mut self, cb: Box<dyn ProgressCallback>) {
        self.callbacks.push(cb);
    }

    /// Number of callbacks currently in the chain.
    pub fn len(&self) -> usize {
        self.callbacks.len()
    }

    /// Whether the chain is empty.
    pub fn is_empty(&self) -> bool {
        self.callbacks.is_empty()
    }
}

impl Default for MultiProgressCallback {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for MultiProgressCallback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MultiProgressCallback")
            .field("callbacks", &self.callbacks.len())
            .finish()
    }
}

impl ProgressCallback for MultiProgressCallback {
    fn push_state(&mut self) {
        for cb in &mut self.callbacks {
            cb.push_state();
        }
    }

    fn pop_state(&mut self) {
        for cb in &mut self.callbacks {
            cb.pop_state();
        }
    }

    fn set_progress(&mut self, percent: f32) {
        for cb in &mut self.callbacks {
            cb.set_progress(percent);
        }
    }

    fn set_title(&self, title: &str) {
        for cb in &self.callbacks {
            cb.set_title(title);
        }
    }

    fn set_status(&self, status: &str) {
        for cb in &self.callbacks {
            cb.set_status(status);
        }
    }

    fn set_sub_status(&self, status: &str) {
        for cb in &self.callbacks {
            cb.set_sub_status(status);
        }
    }

    fn cancelled(&self) -> bool {
        self.callbacks.iter().any(|cb| cb.cancelled())
    }
}
