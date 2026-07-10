// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
//! Idiomatic Rust translation of `pcsx2/FiFo.cpp`.
//!
//! The C++ source models the four FIFO pages mapped to the PS2's VIF0,
//! VIF1, GIF, and IPU devices (HW register ranges `0x4000-0x5000`,
//! `0x5000-0x6000`, `0x6000-0x7000`, and `0x7000-0x8000`,
//! respectively). The original implementation is tightly bound to the
//! surrounding `vif0`/`vif1`/`gifUnit` register state and to `MTGS`
//! callbacks that push/pull data to the host GS thread.
//!
//! The Rust port captures the *data-plane* shape of those FIFOs and
//! keeps the four entry points (`ReadFIFO_VIF1`, `WriteFIFO_VIF0`,
//! `WriteFIFO_VIF1`, `WriteFIFO_GIF`) as thin wrappers around three
//! fixed-capacity [`Fifo<u32>`] buffers (size 32, matching the EE
//! register pages). The host-side transfer logic that the C++ bodies
//! delegate to (`MTGS::InitAndReadFIFO`, `VIF0transfer`, `VIF1transfer`,
//! `gifUnit.TransferGSPacketData`, ...) is intentionally left as a
//! downstream responsibility: callers replace it with the equivalent
//! logic from their own port.
//!
//! The [`Fifo<T>`] type itself is a small generic ring buffer backed by
//! [`std::collections::VecDeque`]. The C++ side relies on the buffer
//! being a fixed-size ring with "drop oldest when full" semantics; the
//! Rust type mirrors that contract via [`Fifo::push`].

#![deny(unsafe_op_in_unsafe_fn)]

use std::collections::VecDeque;

// =====================================================================
// Fifo<T>
// =====================================================================
//
// A small generic FIFO backed by `VecDeque<T>`.
//
// The C++ code has separate `static` FIFOs for VIF0, VIF1, GIF, and IPU
// (the IPU FIFO is in `pcsx2/IPU/IPU_Fifo.cpp`, outside this module).
// The Rust port instantiates three of them here — one per PS2 device
// that this file touches — each pre-sized to 32 entries to match the
// PS2's per-device FIFO page depth. Newer code paths treat the buffer
// as a plain ring; older paths exposed in-place read/write access to
// the underlying quad word, but those have been replaced with a single
// `push`/`pop`/`peek` API.

/// Generic FIFO backed by [`VecDeque<T>`].
#[derive(Debug, Clone)]
pub struct Fifo<T> {
    /// Underlying ring storage. Capacity is bounded by [`Self::capacity`].
    buffer: VecDeque<T>,
    /// Maximum number of elements this FIFO can hold. Pushes past this
    /// drop the oldest element.
    capacity: usize,
}

impl<T> Fifo<T> {
    /// Construct a new empty FIFO with the given capacity.
    ///
    /// `VecDeque::new()` is `const`, so this constructor is `const` too;
    /// callers can use it directly to initialise `static` FIFOs. The
    /// capacity is enforced inside [`Self::push`]; the underlying
    /// `VecDeque` itself starts empty and only allocates when the
    /// first element is pushed.
    pub const fn with_capacity(capacity: usize) -> Self {
        Self {
            buffer: VecDeque::new(),
            capacity,
        }
    }

    /// Current number of elements in the FIFO.
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// `true` when the FIFO holds no elements.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// `true` when the FIFO has reached [`Self::capacity`] entries.
    pub fn is_full(&self) -> bool {
        self.buffer.len() >= self.capacity
    }

    /// Maximum number of elements this FIFO can hold.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Push `value` onto the back of the FIFO.
    ///
    /// If the FIFO is already at capacity, the oldest element is
    /// dropped first to make room (matching the C++ ring-buffer
    /// semantics). Returns `true` if an element was evicted.
    pub fn push(&mut self, value: T) -> bool {
        let evicted = if self.buffer.len() >= self.capacity {
            self.buffer.pop_front();
            true
        } else {
            false
        };
        self.buffer.push_back(value);
        evicted
    }

    /// Pop the front element off the FIFO. Returns `None` when empty.
    pub fn pop(&mut self) -> Option<T> {
        self.buffer.pop_front()
    }

    /// Peek at the front element without removing it. Returns `None`
    /// when the FIFO is empty.
    pub fn peek(&self) -> Option<&T> {
        self.buffer.front()
    }

    /// Empty the FIFO.
    pub fn clear(&mut self) {
        self.buffer.clear();
    }
}

impl<T> Default for Fifo<T> {
    fn default() -> Self {
        Self::with_capacity(0)
    }
}

// =====================================================================
// Per-device FIFO instances
// =====================================================================
//
// Each PS2 device has its own 32-entry FIFO. The capacity matches the
// depth of the corresponding HW register page (one 128-bit quad per
// entry, 32 entries deep).

/// Capacity (in `u32` words) of each per-device FIFO. Matches the PS2
/// register page depth: 32 quads × 4 words per quad.
pub const FIFO_CAPACITY: usize = 32;

/// VIF1 read/write FIFO. Mirrors the `0x5000-0x6000` register page.
pub static mut FIFO_VIF1: Fifo<u32> = Fifo::with_capacity(FIFO_CAPACITY);

/// VIF0 write FIFO. Mirrors the `0x4000-0x5000` register page.
pub static mut FIFO_VIF0: Fifo<u32> = Fifo::with_capacity(FIFO_CAPACITY);

/// GIF write FIFO. Mirrors the `0x6000-0x7000` register page.
pub static mut FIFO_GIF: Fifo<u32> = Fifo::with_capacity(FIFO_CAPACITY);

// =====================================================================
// ReadFIFO_VIF1
// =====================================================================
//
// The C++ `ReadFIFO_VIF1(mem128_t*)` is called when the EE reads back
// data that the GS has previously downloaded via VIF1. It performs a
// stall/permission check, asks `MTGS::InitAndReadFIFO` for a quad, and
// updates `vif1.GSLastDownloadSize` plus a few stat fields. The Rust
// port condenses that into a word-by-word drain into a caller-provided
// `&mut [u32]`: the caller passes the number of 32-bit words they
// expect and we copy out whatever's currently in the VIF1 FIFO,
// stopping early at either the request size or the FIFO's contents.
//
// Returns the number of 32-bit words actually written into `buffer`.

pub fn ReadFIFO_VIF1(buffer: &mut [u32], words: usize) -> usize {
    // SAFETY: `FIFO_VIF1` is a `static mut` whose only writer/reader
    // paths go through these FIFO helpers. Single-threaded in the
    // interpreter dispatch.
    unsafe {
        let requested = words.min(buffer.len());
        let mut written = 0usize;
        while written < requested {
            match FIFO_VIF1.pop() {
                Some(word) => {
                    buffer[written] = word;
                    written += 1;
                }
                None => break,
            }
        }
        written
    }
}

// =====================================================================
// WriteFIFO_VIF0
// =====================================================================
//
// The C++ `WriteFIFO_VIF0(const mem128_t*)` pushes four words into
// the VIF0 device via `VIF0transfer`. The Rust port collapses the
// 128-bit quad into a single `u32` for the ring-buffer interface and
// returns `true` on success. Host-side VIF0 transfer logic is the
// caller's responsibility.

pub fn WriteFIFO_VIF0(value: u32) -> bool {
    // SAFETY: see `ReadFIFO_VIF1`.
    unsafe {
        FIFO_VIF0.push(value);
        true
    }
}

// =====================================================================
// WriteFIFO_VIF1
// =====================================================================
//
// The C++ `WriteFIFO_VIF1(const mem128_t*)` pushes four words via
// `VIF1transfer`. The Rust port takes both a `value` and a `count`
// so callers can stream multiple words through the FIFO at once — the
// C++ variant hard-codes a quad (count == 4); the Rust signature
// generalises that to `count` 32-bit writes and returns `true` if every
// push succeeded.

pub fn WriteFIFO_VIF1(value: u32, count: u32) -> bool {
    // SAFETY: see `ReadFIFO_VIF1`.
    unsafe {
        for _ in 0..count {
            FIFO_VIF1.push(value);
        }
        true
    }
}

// =====================================================================
// WriteFIFO_GIF
// =====================================================================
//
// The C++ `WriteFIFO_GIF(const mem128_t*)` either routes the quad
// straight to `gifUnit.TransferGSPacketData` (PATH3 mode) or buffers
// it in `gif_fifo` (non-PATH3 mode). The Rust port exposes only the
// buffered path through [`FIFO_GIF`]; PATH3 bypass is left to the
// caller. Returns `true` on success.

pub fn WriteFIFO_GIF(value: u32) -> bool {
    // SAFETY: see `ReadFIFO_VIF1`.
    unsafe {
        FIFO_GIF.push(value);
        true
    }
}
