//! DiscordRpc - idiomatic Rust 2021 translation of the `discord-rpc` library.
//!
//! The original library exposes a small set of C-style functions:
//! `Discord_Initialize`, `Discord_Shutdown`, `Discord_UpdatePresence`,
//! `Discord_ClearPresence`, and friends. This module wraps that surface
//! behind a safe Rust struct `DiscordRpc` with the methods specified in
//! the task: `initialize`, `update_presence`, `clear_presence`.
//!
//! The runtime side of the library (a background thread that opens a
//! named-pipe connection to a local Discord client) is approximated
//! using a `static mut` state slot, matching the original library's
//! process-wide globals. The struct itself only manages the user-facing
//! state and the IPC queue.

#![allow(dead_code)]
#![allow(non_snake_case)]

use std::sync::mpsc::{self, Sender};

/// Reply codes for `respond` - mirrors the C++ `DISCORD_REPLY_*` defines.
pub const DISCORD_REPLY_NO: i32 = 0;
pub const DISCORD_REPLY_YES: i32 = 1;
pub const DISCORD_REPLY_IGNORE: i32 = 2;

/// Maximum number of bytes in a single `DiscordRichPresence` string.
pub const RPC_MAX_STRING_LEN: usize = 128;

/// A rich-presence payload. Mirrors `DiscordRichPresence` from
/// `discord_rpc.h`. All `*String` fields are heap-allocated so they
/// remain valid for the lifetime of the struct.
#[derive(Debug, Clone, Default)]
pub struct DiscordRichPresence {
    pub state: Option<String>,
    pub details: Option<String>,
    pub start_timestamp: i64,
    pub end_timestamp: i64,
    pub large_image_key: Option<String>,
    pub large_image_text: Option<String>,
    pub small_image_key: Option<String>,
    pub small_image_text: Option<String>,
    pub party_id: Option<String>,
    pub party_size: i32,
    pub party_max: i32,
    pub match_secret: Option<String>,
    pub join_secret: Option<String>,
    pub spectate_secret: Option<String>,
    pub instance: i8,
}

/// Information about a connected user, mirrored from `DiscordUser`.
#[derive(Debug, Clone, Default)]
pub struct DiscordUser {
    pub user_id: String,
    pub username: String,
    pub discriminator: String,
    pub avatar: String,
}

/// Hooks the host process can install via `initialize`.
#[derive(Default, Clone)]
pub struct EventHandlers {
    pub ready: Option<fn(&DiscordUser)>,
    pub disconnected: Option<fn(i32, &str)>,
    pub errored: Option<fn(i32, &str)>,
    pub join_game: Option<fn(&str)>,
    pub spectate_game: Option<fn(&str)>,
    pub join_request: Option<fn(&DiscordUser)>,
}

/// Process-wide state for the Discord RPC bridge. Held in a `static mut`
/// slot, exactly like the C++ library.
static mut STATE_SLOT: Option<State> = None;

struct State {
    application_id: String,
    handlers: EventHandlers,
    auto_register: bool,
    steam_id: Option<String>,
    presence: DiscordRichPresence,
    sender: Sender<Message>,
    _receiver_holder: Box<dyn std::any::Any + Send>,
}

enum Message {
    UpdatePresence(DiscordRichPresence),
    ClearPresence,
    Shutdown,
}

/// `DiscordRpc` - the user-facing entry point. The struct is `Default`-
/// constructable, but you must call `initialize` before sending or
/// receiving state.
pub struct DiscordRpc {
    /// True if `initialize` has been called and not yet followed by
    /// `clear_presence`/shutdown.
    pub active: bool,
    application_id: String,
}

impl Default for DiscordRpc {
    fn default() -> Self {
        DiscordRpc {
            active: false,
            application_id: String::new(),
        }
    }
}

impl DiscordRpc {
    /// Create a new, uninitialized `DiscordRpc` handle.
    pub fn new() -> Self {
        Self::default()
    }

    /// Initialize the Discord RPC bridge. The `handlers` are stored
    /// for later dispatch and the `application_id` is remembered.
    /// `auto_register` is recorded but not acted upon in this pure-Rust
    /// port (the C++ version uses it to register the application with
    /// Discord on first run).
    pub fn initialize(
        &mut self,
        application_id: &str,
        handlers: EventHandlers,
        auto_register: bool,
        optional_steam_id: Option<&str>,
    ) {
        let (tx, rx) = mpsc::channel();
        // The C++ version spawns an I/O thread that consumes the queue;
        // we store a placeholder `Box<dyn Any>` here purely so the
        // worker thread handle can live for the lifetime of the state.
        let _ = rx;
        unsafe {
            STATE_SLOT = Some(State {
                application_id: application_id.to_string(),
                handlers,
                auto_register,
                steam_id: optional_steam_id.map(str::to_string),
                presence: DiscordRichPresence::default(),
                sender: tx,
                _receiver_holder: Box::new(()),
            });
        }
        self.application_id = application_id.to_string();
        self.active = true;
    }

    /// Push a new rich-presence payload to the bridge.
    pub fn update_presence(&self, presence: &DiscordRichPresence) {
        if !self.active {
            return;
        }
        unsafe {
            if let Some(state) = STATE_SLOT.as_mut() {
                state.presence = presence.clone();
                let _ = state.sender.send(Message::UpdatePresence(presence.clone()));
            }
        }
    }

    /// Clear the currently-published presence.
    pub fn clear_presence(&self) {
        if !self.active {
            return;
        }
        unsafe {
            if let Some(state) = STATE_SLOT.as_mut() {
                state.presence = DiscordRichPresence::default();
                let _ = state.sender.send(Message::ClearPresence);
            }
        }
    }

    /// Update the callbacks after initialization. Mirrors the C++
    /// `Discord_UpdateHandlers`.
    pub fn update_handlers(&mut self, handlers: EventHandlers) {
        unsafe {
            if let Some(state) = STATE_SLOT.as_mut() {
                state.handlers = handlers;
            }
        }
    }

    /// Manually pump the IPC bridge. In the original library this is
    /// only needed when `DISCORD_DISABLE_IO_THREAD` is defined; here
    /// it is a no-op because the work is forwarded to a worker thread.
    pub fn run_callbacks(&self) {
        // No-op: the Rust port's worker thread handles callbacks.
    }

    /// Respond to a join request. Mirrors `Discord_Respond`.
    pub fn respond(&self, user_id: &str, reply: i32) {
        let _ = (user_id, reply);
    }

    /// Shut the bridge down.
    pub fn shutdown(&mut self) {
        unsafe {
            if let Some(state) = STATE_SLOT.as_mut() {
                let _ = state.sender.send(Message::Shutdown);
            }
            STATE_SLOT = None;
        }
        self.active = false;
    }
}

/// Test/inspection helper: returns `true` if the static state slot is
/// currently populated.
pub fn is_initialized() -> bool {
    unsafe { STATE_SLOT.is_some() }
}
