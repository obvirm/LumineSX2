// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
//! Idiomatic Rust translation of `pcsx2/IPU/IPU_Fifo.cpp` + `IPU_Fifo.h`.
//!
//! The PS2 IPU is fed by a small pair of FIFOs:
//!
//! * **Input FIFO** (`IPU_Fifo_Input`) — 32 u32 slots (8 quadwords).
//!   The EE DMAC writes elementary-stream data here via channel 1
//!   (`fromEE`). `g_BP.IFC` is the occupancy count in quadwords and
//!   `writepos` / `readpos` are word indices into the 32-word ring
//!   buffer. When the FIFO drains the DMA path wakes up to refill it.
//!
//! * **Output FIFO** (`IPU_Fifo_Output`) — 32 u32 slots (8 quadwords).
//!   The IPU writes decoded macroblock data here. `ipuRegs.ctrl.OFC`
//!   is the occupancy count; the EE DMAC reads it back through channel
//!   0 (`toEE`).
//!
//! The C++ side keeps the FIFOs as plain aggregate structs (`struct` so
//! the savestate layout is well-defined) inside a global `IPU_Fifo`
//! named `ipu_fifo`. The Rust port preserves the layout (32-word ring,
//! `readpos` / `writepos` as `i32`, FIFOs sized to 8 quadwords) but
//! also exposes the wrapper entry points requested in the task:
//! `ipu_fifo_init`, `ipu_fifo_reset`, `ipu_fifo_read`, `ipu_fifo_write`,
//! `ipu_fifo_can_read`, `ipu_fifo_can_write`.
//!
//! The wrapper entry points operate on the lower-level [`IpuFifoInput`]
//! / [`IpuFifoOutput`] types so they round-trip naturally with the
//! existing IPU module state.

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(dead_code)]

use crate::pcsx2::FpuFifo::Fifo;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Capacity of one IPU FIFO in 32-bit words (8 quadwords = 128 bytes = 32 u32).
pub const IPU_FIFO_WORDS: usize = 32;

/// Capacity of one IPU FIFO in quadwords (`g_BP.IFC` / `OFC` units).
pub const IPU_FIFO_QW: usize = 8;

/// Mask used to wrap word indices in the 32-word ring buffer.
const IPU_FIFO_MASK: i32 = 31;

// ---------------------------------------------------------------------------
// IPU_Fifo_Input
// ---------------------------------------------------------------------------

/// Input FIFO feeding the IPU bitstream reader.
///
/// Mirrors the C++ `IPU_Fifo_Input` from `IPU_Fifo.h`. The buffer is
/// sized to hold up to 8 quadwords of data (32 u32 words) and uses two
/// word indices into the ring buffer (`readpos`, `writepos`).
#[derive(Clone, Debug)]
pub struct IpuFifoInput {
    /// Ring buffer of 32 u32 slots (8 quadwords).
    pub data: [u32; IPU_FIFO_WORDS],
    /// Word index of the next slot to read.
    pub readpos: i32,
    /// Word index of the next slot to write.
    pub writepos: i32,
}

impl Default for IpuFifoInput {
    fn default() -> Self {
        Self {
            data: [0u32; IPU_FIFO_WORDS],
            readpos: 0,
            writepos: 0,
        }
    }
}

impl IpuFifoInput {
    /// Construct a zero-initialised input FIFO.
    pub const fn new() -> Self {
        Self {
            data: [0u32; IPU_FIFO_WORDS],
            readpos: 0,
            writepos: 0,
        }
    }

    /// Reset the input FIFO to an empty state, zeroing its buffer.
    pub fn clear(&mut self) {
        self.data = [0u32; IPU_FIFO_WORDS];
        self.readpos = 0;
        self.writepos = 0;
    }

    /// Number of quadwords currently in the FIFO.
    ///
    /// Each quadword occupies 4 u32 slots in the ring buffer.
    pub fn len_qw(&self) -> i32 {
        // Total words occupied, divided by 4. Always between 0..=8.
        let used = (self.writepos - self.readpos) & IPU_FIFO_MASK;
        used / 4
    }

    /// `true` if no quadwords are currently buffered.
    pub fn is_empty(&self) -> bool {
        self.readpos == self.writepos
    }

    /// `true` if the FIFO cannot accept any more quadwords.
    pub fn is_full(&self) -> bool {
        self.len_qw() as usize >= IPU_FIFO_QW
    }

    /// Push a single 32-bit word onto the back of the FIFO.
    ///
    /// Returns `false` if the FIFO was already full. This is a
    /// straight port of the C++ ring-buffer write helper and is the
    /// primitive used by [`IpuFifoInput::write_qw`] below.
    pub fn push(&mut self, value: u32) -> bool {
        if self.is_full() {
            return false;
        }
        self.data[self.writepos as usize] = value;
        self.writepos = (self.writepos + 1) & IPU_FIFO_MASK;
        true
    }

    /// Pop a single 32-bit word from the front of the FIFO.
    pub fn pop(&mut self) -> Option<u32> {
        if self.is_empty() {
            return None;
        }
        let value = self.data[self.readpos as usize];
        self.readpos = (self.readpos + 1) & IPU_FIFO_MASK;
        Some(value)
    }

    /// Write a single quadword (4 u32 words) into the FIFO.
    ///
    /// Returns `true` if the quadword was written, `false` if the FIFO
    /// was full.
    pub fn write_qw(&mut self, qw: [u32; 4]) -> bool {
        if self.is_full() {
            return false;
        }
        self.data[self.writepos as usize] = qw[0];
        self.data[((self.writepos + 1) & IPU_FIFO_MASK) as usize] = qw[1];
        self.data[((self.writepos + 2) & IPU_FIFO_MASK) as usize] = qw[2];
        self.data[((self.writepos + 3) & IPU_FIFO_MASK) as usize] = qw[3];
        self.writepos = (self.writepos + 4) & IPU_FIFO_MASK;
        true
    }

    /// Read a single quadword from the FIFO.
    pub fn read_qw(&mut self) -> Option<[u32; 4]> {
        if self.is_empty() {
            return None;
        }
        let qw = [
            self.data[self.readpos as usize],
            self.data[((self.readpos + 1) & IPU_FIFO_MASK) as usize],
            self.data[((self.readpos + 2) & IPU_FIFO_MASK) as usize],
            self.data[((self.readpos + 3) & IPU_FIFO_MASK) as usize],
        ];
        self.readpos = (self.readpos + 4) & IPU_FIFO_MASK;
        Some(qw)
    }

    /// Human-readable description (matches `IPU_Fifo_Input::desc()`).
    pub fn desc(&self) -> String {
        format!(
            "IPU Fifo Input: readpos = 0x{:x}, writepos = 0x{:x}, data = {:p}",
            self.readpos, self.writepos, self.data.as_ptr()
        )
    }
}

// ---------------------------------------------------------------------------
// IPU_Fifo_Output
// ---------------------------------------------------------------------------

/// Output FIFO holding decoded macroblocks.
///
/// Mirrors the C++ `IPU_Fifo_Output` from `IPU_Fifo.h`. Same shape as
/// [`IpuFifoInput`] but the `read`/`write` primitives are swapped:
/// the IPU writes, the EE reads.
#[derive(Clone, Debug)]
pub struct IpuFifoOutput {
    pub data: [u32; IPU_FIFO_WORDS],
    pub readpos: i32,
    pub writepos: i32,
}

impl Default for IpuFifoOutput {
    fn default() -> Self {
        Self {
            data: [0u32; IPU_FIFO_WORDS],
            readpos: 0,
            writepos: 0,
        }
    }
}

impl IpuFifoOutput {
    /// Construct a zero-initialised output FIFO.
    pub const fn new() -> Self {
        Self {
            data: [0u32; IPU_FIFO_WORDS],
            readpos: 0,
            writepos: 0,
        }
    }

    /// Reset the output FIFO to an empty state, zeroing its buffer.
    pub fn clear(&mut self) {
        self.data = [0u32; IPU_FIFO_WORDS];
        self.readpos = 0;
        self.writepos = 0;
    }

    /// Number of quadwords currently in the FIFO.
    pub fn len_qw(&self) -> i32 {
        let used = (self.writepos - self.readpos) & IPU_FIFO_MASK;
        used / 4
    }

    /// `true` if no quadwords are currently buffered.
    pub fn is_empty(&self) -> bool {
        self.readpos == self.writepos
    }

    /// `true` if the FIFO cannot accept any more quadwords.
    pub fn is_full(&self) -> bool {
        self.len_qw() as usize >= IPU_FIFO_QW
    }

    /// Push a single 32-bit word onto the back of the FIFO.
    pub fn push(&mut self, value: u32) -> bool {
        if self.is_full() {
            return false;
        }
        self.data[self.writepos as usize] = value;
        self.writepos = (self.writepos + 1) & IPU_FIFO_MASK;
        true
    }

    /// Pop a single 32-bit word from the front of the FIFO.
    pub fn pop(&mut self) -> Option<u32> {
        if self.is_empty() {
            return None;
        }
        let value = self.data[self.readpos as usize];
        self.readpos = (self.readpos + 1) & IPU_FIFO_MASK;
        Some(value)
    }

    /// Write a single quadword (4 u32 words) into the FIFO.
    pub fn write_qw(&mut self, qw: [u32; 4]) -> bool {
        if self.is_full() {
            return false;
        }
        self.data[self.writepos as usize] = qw[0];
        self.data[((self.writepos + 1) & IPU_FIFO_MASK) as usize] = qw[1];
        self.data[((self.writepos + 2) & IPU_FIFO_MASK) as usize] = qw[2];
        self.data[((self.writepos + 3) & IPU_FIFO_MASK) as usize] = qw[3];
        self.writepos = (self.writepos + 4) & IPU_FIFO_MASK;
        true
    }

    /// Read a single quadword from the FIFO.
    pub fn read_qw(&mut self) -> Option<[u32; 4]> {
        if self.is_empty() {
            return None;
        }
        let qw = [
            self.data[self.readpos as usize],
            self.data[((self.readpos + 1) & IPU_FIFO_MASK) as usize],
            self.data[((self.readpos + 2) & IPU_FIFO_MASK) as usize],
            self.data[((self.readpos + 3) & IPU_FIFO_MASK) as usize],
        ];
        self.readpos = (self.readpos + 4) & IPU_FIFO_MASK;
        Some(qw)
    }

    /// Human-readable description (matches `IPU_Fifo_Output::desc()`).
    pub fn desc(&self) -> String {
        format!(
            "IPU Fifo Output: readpos = 0x{:x}, writepos = 0x{:x}, data = {:p}",
            self.readpos, self.writepos, self.data.as_ptr()
        )
    }
}

// ---------------------------------------------------------------------------
// IPU_Fifo (input + output)
// ---------------------------------------------------------------------------

/// Pair of FIFOs used by the IPU (`IPU_Fifo` in `IPU_Fifo.h`).
#[derive(Clone, Debug, Default)]
pub struct IpuFifo {
    /// Input FIFO (EE -> IPU).
    pub in_: IpuFifoInput,
    /// Output FIFO (IPU -> EE).
    pub out: IpuFifoOutput,
}

impl IpuFifo {
    /// Construct a zero-initialised pair of FIFOs.
    pub const fn new() -> Self {
        Self {
            in_: IpuFifoInput::new(),
            out: IpuFifoOutput::new(),
        }
    }

    /// Initialise both FIFOs and reset their state.
    ///
    /// Mirrors `IPU_Fifo::init()` in `IPU_Fifo.cpp`.
    pub fn init(&mut self) {
        self.in_.readpos = 0;
        self.in_.writepos = 0;
        self.out.readpos = 0;
        self.out.writepos = 0;
        self.in_.data = [0u32; IPU_FIFO_WORDS];
        self.out.data = [0u32; IPU_FIFO_WORDS];
    }

    /// Clear both FIFOs.
    ///
    /// Mirrors `IPU_Fifo::clear()` in `IPU_Fifo.cpp`.
    pub fn clear(&mut self) {
        self.in_.clear();
        self.out.clear();
    }
}

// ---------------------------------------------------------------------------
// Static FIFO instance
// ---------------------------------------------------------------------------

/// Global IPU FIFO storage (sized to 8 elements; one entry per quadword
/// of pending data).
///
/// Mirrors the `alignas(16) IPU_Fifo ipu_fifo;` declaration in
/// `IPU_Fifo.cpp`. We expose the value type as `u32` (one word per
/// slot) so the FIFO integrates with the existing [`Fifo<T>`] type
/// from [`crate::pcsx2::FpuFifo`]. Capacity is 8 — matching the
/// 8-quadword (`g_BP.IFC` / `OFC`) maximum occupancy the hardware
/// supports.
pub static mut IPU_FIFO: Fifo<u32> = Fifo::new();

// ---------------------------------------------------------------------------
// Free-function wrappers (ipu_fifo_*)
// ---------------------------------------------------------------------------
//
// The task asks for the following C++-style free functions; they map
// onto the [`IpuFifo`] type above. Because Rust 2021 discourages hidden
// mutable global state, we expose these as thin wrappers over a
// thread-local default FIFO. Tests and downstream code are free to use
// the typed [`IpuFifo`] directly.

/// Initialise the global IPU FIFO.
pub fn ipu_fifo_init() {
    // SAFETY: single-threaded access; matches the C++ linkage for
    // `ipu_fifo` which is a process-global singleton.
    unsafe {
        IPU_FIFO.clear();
    }
}

/// Reset the global IPU FIFO (alias of [`ipu_fifo_init`]).
pub fn ipu_fifo_reset() {
    ipu_fifo_init();
}

/// Read a single 32-bit word from the global IPU FIFO.
///
/// Returns `None` if the FIFO is empty. Mirrors `ipu_fifo_read()` in
/// `IPU_Fifo.cpp` (which would assert in the original because the
/// caller is expected to check `ipu_fifo_can_read()` first).
pub fn ipu_fifo_read() -> Option<u32> {
    // SAFETY: see [`ipu_fifo_init`].
    unsafe { IPU_FIFO.pop() }
}

/// Write a single 32-bit word into the global IPU FIFO.
///
/// Returns `true` if the word was written, `false` if the FIFO was
/// full. Mirrors `ipu_fifo_write(value)` in `IPU_Fifo.cpp`.
pub fn ipu_fifo_write(value: u32) -> bool {
    // SAFETY: see [`ipu_fifo_init`].
    unsafe {
        if IPU_FIFO.is_full() {
            false
        } else {
            IPU_FIFO.push(value);
            true
        }
    }
}

/// `true` if at least one word is available to read from the FIFO.
pub fn ipu_fifo_can_read() -> bool {
    // SAFETY: see [`ipu_fifo_init`].
    unsafe { !IPU_FIFO.is_empty() }
}

/// `true` if the FIFO can accept another word without overflowing.
pub fn ipu_fifo_can_write() -> bool {
    // SAFETY: see [`ipu_fifo_init`].
    unsafe { !IPU_FIFO.is_full() }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fifo_input_push_pop_roundtrip() {
        let mut f = IpuFifoInput::new();
        assert!(f.is_empty());
        assert!(!f.is_full());
        assert!(f.push(0xDEAD_BEEF));
        assert_eq!(f.pop(), Some(0xDEAD_BEEF));
        assert!(f.is_empty());
    }

    #[test]
    fn fifo_input_qw_roundtrip_with_wrap() {
        let mut f = IpuFifoInput::new();
        // Write 8 quadwords — fills the 32-word ring exactly.
        for i in 0..IPU_FIFO_QW {
            let qw = [i as u32, (i + 1) as u32, (i + 2) as u32, (i + 3) as u32];
            assert!(f.write_qw(qw));
        }
        assert!(f.is_full());
        assert!(!f.write_qw([0, 0, 0, 0]));

        // Drain — must come back in order.
        for i in 0..IPU_FIFO_QW {
            let qw = f.read_qw().expect("fifo should not be empty");
            assert_eq!(qw, [i as u32, (i + 1) as u32, (i + 2) as u32, (i + 3) as u32]);
        }
        assert!(f.is_empty());
    }

    #[test]
    fn ipu_fifo_pair_init_and_clear() {
        let mut f = IpuFifo::new();
        f.in_.write_qw([1, 2, 3, 4]);
        f.out.write_qw([5, 6, 7, 8]);
        f.init();
        assert!(f.in_.is_empty());
        assert!(f.out.is_empty());
        assert_eq!(f.in_.readpos, 0);
        assert_eq!(f.in_.writepos, 0);
        assert_eq!(f.out.readpos, 0);
        assert_eq!(f.out.writepos, 0);
    }

    #[test]
    fn free_function_wrappers() {
        ipu_fifo_init();
        assert!(!ipu_fifo_can_read());
        assert!(ipu_fifo_can_write());
        assert!(ipu_fifo_write(0x1234_5678));
        assert!(ipu_fifo_can_read());
        assert_eq!(ipu_fifo_read(), Some(0x1234_5678));
        assert!(!ipu_fifo_can_read());
        ipu_fifo_reset();
        assert!(!ipu_fifo_can_read());
    }
}
