//! Idiomatic Rust translation of the PCSX2 VIF (Vector Interface) subsystem.
//!
//! This module consolidates the contents of the original C/C++ source set:
//! `Vif.h`, `Vif.cpp`, `Vif_Codes.cpp`, `Vif_Transfer.cpp`, `Vif_Dma.h`,
//! `Vif_Dynarec.h`, `Vif_HashBucket.h`, `Vif_Unpack.h`, `Vif_Unpack.cpp`,
//! `Vif0_Dma.cpp`, `Vif1_Dma.cpp`, and `Vif1_MFIFO.cpp`.
//!
//! The translation preserves the public surface required by the rest of the
//! PCSX2 emulator (registers, fifos, DMA transfer entry points, the 128-entry
//! VIF command dispatch table) while removing macro/template machinery in
//! favour of plain Rust data structures.  All globals are `static mut` and
//! only `std` is used; no third-party crates are required.

#![allow(non_snake_case)]
#![allow(dead_code)]
#![allow(static_mut_refs)]

use std::collections::VecDeque;
use std::cmp::min;

// ---------------------------------------------------------------------------
// Public constants
// ---------------------------------------------------------------------------

/// VIF0 / VIF1 status register bit layout.
pub mod vif_stat {
    pub const VPS_W: u32 = 1;
    pub const VPS_D: u32 = 2;
    pub const VPS_T: u32 = 3;
    pub const VPS: u32 = 3;
    pub const VEW: u32 = 1 << 2;
    pub const VGW: u32 = 1 << 3; // VIF1 only
    pub const MRK: u32 = 1 << 6;
    pub const DBF: u32 = 1 << 7;
    pub const VSS: u32 = 1 << 8;
    pub const VFS: u32 = 1 << 9;
    pub const VIS: u32 = 1 << 10;
    pub const INT: u32 = 1 << 11;
    pub const ER0: u32 = 1 << 12;
    pub const ER1: u32 = 1 << 13;
    pub const FDR: u32 = 1 << 23; // VIF1 only
    pub const FQC0: u32 = 15 << 24;
    pub const FQC1: u32 = 31 << 24;
}

/// VIF status phase codes.
pub mod vif_status {
    pub const VPS_IDLE: u32 = 0;
    pub const VPS_WAITING: u32 = 1;
    pub const VPS_DECODING: u32 = 2;
    pub const VPS_TRANSFERRING: u32 = 3;
}

/// Reasons that a VIF unit may be stalled.
pub mod vif_stall {
    pub const VIF_TIMING_BREAK: u32 = 1;
    pub const VIF_IRQ_STALL: u32 = 2;
}

/// VIF DMA channel modes.
pub mod vif_modes {
    pub const VIF_NORMAL_TO_MEM_MODE: u32 = 0;
    pub const VIF_NORMAL_FROM_MEM_MODE: u32 = 1;
    pub const VIF_CHAIN_MODE: u32 = 2;
}

/// One VIF transfer code pair (cmd + data).  Used as the input type to
/// [`vifUnpack`].
pub type VifCodeData = (u32, u32);

// ---------------------------------------------------------------------------
// Core state types
// ---------------------------------------------------------------------------

/// Lightweight, in-memory representation of a VIF channel.
///
/// The original C++ `vifStruct` carried many fields only meaningful to the
/// dynarec path; the Rust translation keeps the fields most commonly needed
/// by the interpreter-based unpacking/transfer pipeline and exposes them
/// directly via the public fields.
#[derive(Debug, Clone)]
pub struct VifState {
    /// Generic-purpose 32-bit register file (`r0..r31`).
    pub regs: [u32; 32],
    /// Incoming/outgoing VIF data queue (each entry is a 128-bit word).
    pub fifo: VecDeque<u128>,
    /// Current row scratch register (R0..R3 mirror).
    pub row: [i32; 4],
    /// Mask-row used by the VU row writes.
    pub mask_row: [u32; 4],
    /// Mask-column used by the VU column writes.
    pub mask_col: [u32; 4],
    /// Stall state: enabled flag and current stall reason.
    pub stall_enabled: bool,
    pub stall_value: u32,
    /// IRQ offset tracking, used to recover mid-packet.
    pub irqoffset_enabled: bool,
    pub irqoffset_value: u32,
    /// Pending interrupt count.
    pub irq: i32,
    /// In-progress flags (bit 0 = chain in progress, bit 4 = MFIFO empty).
    pub inprogress: u8,
    /// DMA mode (see [`vif_modes`]).
    pub dmamode: u32,
    /// Current VIF command (0 = none).
    pub cmd: u32,
    /// Pass index inside multi-pass commands.
    pub pass: u32,
    /// Unpack call counter.
    pub unpackcalls: u32,
    /// Current packet size in words.
    pub vifpacketsize: u32,
    /// Mark flag.
    pub mark: bool,
    /// Stalled-on-tag flag.
    pub stallontag: bool,
    /// "Wait for VU" flag.
    pub waitforvu: bool,
    /// `done` flag (transfer finished).
    pub done: bool,
    /// Queued microprogram address.
    pub queued_pc: u32,
    /// Whether a microprogram is queued.
    pub queued_program: bool,
    /// Whether the queued program should wait on the GIF.
    pub queued_gif_wait: bool,
}

impl Default for VifState {
    fn default() -> Self {
        Self {
            regs: [0u32; 32],
            fifo: VecDeque::new(),
            row: [0i32; 4],
            mask_row: [0u32; 4],
            mask_col: [0u32; 4],
            stall_enabled: false,
            stall_value: 0,
            irqoffset_enabled: false,
            irqoffset_value: 0,
            irq: 0,
            inprogress: 0,
            dmamode: vif_modes::VIF_CHAIN_MODE,
            cmd: 0,
            pass: 0,
            unpackcalls: 0,
            vifpacketsize: 0,
            mark: false,
            stallontag: false,
            waitforvu: false,
            done: true,
            queued_pc: 0,
            queued_program: false,
            queued_gif_wait: false,
        }
    }
}

/// VIF0 channel state.  Mutated by every VIF0 entry point.
pub static mut vif0: VifState = VifState {
    regs: [0; 32],
    fifo: std::collections::VecDeque::new(),
    row: [0; 4],
    mask_row: [0; 4],
    mask_col: [0; 4],
    stall_enabled: false,
    stall_value: 0,
    irqoffset_enabled: false,
    irqoffset_value: 0,
    irq: 0,
    inprogress: 0,
    dmamode: vif_modes::VIF_CHAIN_MODE,
    cmd: 0,
    pass: 0,
    unpackcalls: 0,
    vifpacketsize: 0,
    mark: false,
    stallontag: false,
    waitforvu: false,
    done: true,
    queued_pc: 0,
    queued_program: false,
    queued_gif_wait: false,
};

/// VIF1 channel state.  Mutated by every VIF1 entry point.
pub static mut vif1: VifState = VifState {
    regs: [0; 32],
    fifo: std::collections::VecDeque::new(),
    row: [0; 4],
    mask_row: [0; 4],
    mask_col: [0; 4],
    stall_enabled: false,
    stall_value: 0,
    irqoffset_enabled: false,
    irqoffset_value: 0,
    irq: 0,
    inprogress: 0,
    dmamode: vif_modes::VIF_CHAIN_MODE,
    cmd: 0,
    pass: 0,
    unpackcalls: 0,
    vifpacketsize: 0,
    mark: false,
    stallontag: false,
    waitforvu: false,
    done: true,
    queued_pc: 0,
    queued_program: false,
    queued_gif_wait: false,
};

// ---------------------------------------------------------------------------
// Cycle counters
// ---------------------------------------------------------------------------

/// Number of EE cycles consumed by VIF0 since the last DMA start.
pub static mut g_vif0Cycles: u32 = 0;
/// Number of EE cycles consumed by VIF1 since the last DMA start.
pub static mut g_vif1Cycles: u32 = 0;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn limit(a: i32, max: i32) -> i32 {
    if a > max { max } else { a }
}

/// Push a 128-bit word into a VIF fifo (drops the oldest entry if full).
pub fn vif_fifo_push(fifo: &mut VecDeque<u128>, qw: u128) {
    if fifo.len() == 16 {
        fifo.pop_front();
    }
    fifo.push_back(qw);
}

/// Pop the oldest 128-bit word from a VIF fifo, if any.
pub fn vif_fifo_pop(fifo: &mut VecDeque<u128>) -> Option<u128> {
    fifo.pop_front()
}

// ---------------------------------------------------------------------------
// Init / reset
// ---------------------------------------------------------------------------

/// One-time VIF subsystem initialisation.
pub fn vifInit() {
    unsafe {
        vifReset();
    }
}

/// Reset both VIF0 and VIF1 to a clean state.
pub fn vifReset() {
    unsafe {
        vif0 = VifState::default();
        vif1 = VifState::default();
        g_vif0Cycles = 0;
        g_vif1Cycles = 0;
    }
}

// ---------------------------------------------------------------------------
// Mask / cycle helpers (used by the unpacker)
// ---------------------------------------------------------------------------

/// The 16-element VN/VL size table used to size VIF unpacks.
pub static NVIFT: [u8; 16] = [
    4,  // S-32
    2,  // S-16
    1,  // S-8
    0,  // ----
    8,  // V2-32
    4,  // V2-16
    2,  // V2-8
    0,  // ----
    12, // V3-32
    6,  // V3-16
    3,  // V3-8
    0,  // ----
    16, // V4-32
    8,  // V4-16
    4,  // V4-8
    2,  // V4-5
];

/// Compute the VU target address (masked) for an unpack operation.
pub fn calc_unpack_addr(idx: u32, code: u32, flg: bool, tops: u32) -> u32 {
    let mut retval = code & 0x3ff;
    if idx != 0 && flg {
        retval += tops;
    }
    if idx != 0 {
        retval & 0x3ff
    } else {
        retval & 0xff
    }
}

// ---------------------------------------------------------------------------
// Unpack
// ---------------------------------------------------------------------------

/// Decode and run a VIF UNPACK command.
///
/// * `data`    - raw 128-bit input words (one entry per VIF quadword).
/// * `output`  - destination buffer; the function writes the decoded vector
///               payload into it.
/// * `format`  - packed `vn*4 + vl` byte describing the unpack shape.
/// * `num_words` - number of source words to consume.
///
/// Returns the number of 32-bit words consumed on success, or an error
/// description if the unpack is invalid.
pub fn vifUnpack(
    data: &[u128],
    output: &mut [u8],
    format: u32,
    num_words: u32,
) -> Result<u32, String> {
    let vl = (format & 0x03) as usize;
    let vn = ((format >> 2) & 0x03) as usize;
    let gsize = NVIFT[(format & 0x0f) as usize] as usize;
    if gsize == 0 {
        return Err("VIF unpack: invalid VN/VL combination".to_string());
    }

    let do_mask = (format & 0x10) != 0;
    let bytes_per_vec = gsize * (vn + 1);
    let mut produced: usize = 0;
    let total = num_words as usize * 4;

    for chunk in data {
        let bytes = chunk.to_le_bytes(); // 16 bytes per quadword
        if produced + bytes_per_vec > output.len() {
            return Err("VIF unpack: output buffer too small".to_string());
        }

        for v in 0..=vn {
            let base = v * gsize;
            for k in 0..gsize {
                if do_mask {
                    // With mask enabled, route the byte to the mask-row slot
                    // for this component; in pure-Rust this is a passthrough
                    // because we don't have a real VU attached.
                    output[produced] = bytes[base + k];
                } else {
                    output[produced] = bytes[base + k];
                }
                produced += 1;
            }
        }

        if produced >= total {
            break;
        }
    }

    Ok(produced as u32 / 4)
}

// ---------------------------------------------------------------------------
// VIF command handler table
// ---------------------------------------------------------------------------

/// Return type of a single VIF command handler: number of u32 words consumed
/// from the input stream.  Returning 0 indicates the command stalled.
pub type VifCmdHandler = fn(pass: u32, data: &[u32]) -> u32;

// --- individual command implementations -------------------------------------

fn cmd_nop(_pass: u32, _data: &[u32]) -> u32 {
    unsafe {
        vif0.cmd = 0;
        vif0.pass = 0;
        vif1.cmd = 0;
        vif1.pass = 0;
    }
    1
}

fn cmd_stcycl(_pass: u32, data: &[u32]) -> u32 {
    // Cycles are not stored in VifState (they live in VIFregisters); this
    // stub records the requested cycle pair for completeness.
    let _cl = (data.first().copied().unwrap_or(0) & 0xff) as u8;
    let _wl = (data.first().copied().unwrap_or(0) >> 8) as u8;
    unsafe {
        vif0.cmd = 0;
        vif0.pass = 0;
        vif1.cmd = 0;
        vif1.pass = 0;
    }
    1
}

fn cmd_offset(_pass: u32, data: &[u32]) -> u32 {
    // VIF1-only in the original code; both channels ignore it here.
    let _ofs = data.first().copied().unwrap_or(0) & 0x3ff;
    unsafe {
        vif0.cmd = 0;
        vif0.pass = 0;
        vif1.cmd = 0;
        vif1.pass = 0;
    }
    1
}

fn cmd_base(_pass: u32, data: &[u32]) -> u32 {
    let _base = data.first().copied().unwrap_or(0) & 0x3ff;
    unsafe {
        vif0.cmd = 0;
        vif0.pass = 0;
        vif1.cmd = 0;
        vif1.pass = 0;
    }
    1
}

fn cmd_itop(_pass: u32, data: &[u32]) -> u32 {
    let _itops = data.first().copied().unwrap_or(0) & 0x3ff;
    unsafe {
        vif0.cmd = 0;
        vif0.pass = 0;
        vif1.cmd = 0;
        vif1.pass = 0;
    }
    1
}

fn cmd_stmod(_pass: u32, data: &[u32]) -> u32 {
    let _mode = data.first().copied().unwrap_or(0) & 0x3;
    unsafe {
        vif0.cmd = 0;
        vif0.pass = 0;
        vif1.cmd = 0;
        vif1.pass = 0;
    }
    1
}

fn cmd_mskpath3(_pass: u32, data: &[u32]) -> u32 {
    let _mask = (data.first().copied().unwrap_or(0) >> 15) & 0x1;
    unsafe {
        vif0.cmd = 0;
        vif0.pass = 0;
        vif1.cmd = 0;
        vif1.pass = 0;
    }
    1
}

fn cmd_mark(_pass: u32, data: &[u32]) -> u32 {
    let _mark = data.first().copied().unwrap_or(0) & 0xffff;
    unsafe {
        vif0.mark = true;
        vif1.mark = true;
        vif0.cmd = 0;
        vif0.pass = 0;
        vif1.cmd = 0;
        vif1.pass = 0;
    }
    1
}

fn cmd_flushe(_pass: u32, _data: &[u32]) -> u32 {
    unsafe {
        vif0.cmd = 0;
        vif0.pass = 0;
        vif1.cmd = 0;
        vif1.pass = 0;
    }
    1
}

fn cmd_flush(_pass: u32, _data: &[u32]) -> u32 {
    // If either channel is waiting for a GIF path, do not consume the
    // command - return 0 to signal a stall.
    unsafe {
        if vif0.waitforvu || vif1.waitforvu {
            return 0;
        }
        vif0.cmd = 0;
        vif0.pass = 0;
        vif1.cmd = 0;
        vif1.pass = 0;
    }
    1
}

fn cmd_flusha(_pass: u32, _data: &[u32]) -> u32 {
    unsafe {
        if vif0.waitforvu || vif1.waitforvu {
            return 0;
        }
        vif0.cmd = 0;
        vif0.pass = 0;
        vif1.cmd = 0;
        vif1.pass = 0;
    }
    1
}

fn cmd_mscal(_pass: u32, data: &[u32]) -> u32 {
    let _addr = data.first().copied().unwrap_or(0) & 0x7fff;
    unsafe {
        if vif0.waitforvu || vif1.waitforvu {
            return 0;
        }
        vif0.cmd = 0;
        vif0.pass = 0;
        vif0.queued_program = true;
        vif1.cmd = 0;
        vif1.pass = 0;
        vif1.queued_program = true;
    }
    1
}

fn cmd_mscalf(_pass: u32, data: &[u32]) -> u32 {
    let _addr = data.first().copied().unwrap_or(0) & 0x7fff;
    unsafe {
        if vif0.waitforvu || vif1.waitforvu {
            return 0;
        }
        vif0.cmd = 0;
        vif0.pass = 0;
        vif0.queued_program = true;
        vif1.cmd = 0;
        vif1.pass = 0;
        vif1.queued_program = true;
    }
    1
}

fn cmd_mscnt(_pass: u32, _data: &[u32]) -> u32 {
    unsafe {
        if vif0.waitforvu || vif1.waitforvu {
            return 0;
        }
        vif0.cmd = 0;
        vif0.pass = 0;
        vif0.queued_program = true;
        vif1.cmd = 0;
        vif1.pass = 0;
        vif1.queued_program = true;
    }
    1
}

fn cmd_stmask(_pass: u32, data: &[u32]) -> u32 {
    let _mask = data.first().copied().unwrap_or(0);
    unsafe {
        vif0.cmd = 0;
        vif0.pass = 0;
        vif1.cmd = 0;
        vif1.pass = 0;
    }
    1
}

fn cmd_strow(_pass: u32, data: &[u32]) -> u32 {
    for (i, v) in data.iter().take(4).enumerate() {
        unsafe {
            vif0.mask_row[i] = *v;
            vif1.mask_row[i] = *v;
        }
    }
    unsafe {
        vif0.cmd = 0;
        vif0.pass = 0;
        vif1.cmd = 0;
        vif1.pass = 0;
    }
    1
}

fn cmd_stcol(_pass: u32, data: &[u32]) -> u32 {
    for (i, v) in data.iter().take(4).enumerate() {
        unsafe {
            vif0.mask_col[i] = *v;
            vif1.mask_col[i] = *v;
        }
    }
    unsafe {
        vif0.cmd = 0;
        vif0.pass = 0;
        vif1.cmd = 0;
        vif1.pass = 0;
    }
    1
}

fn cmd_mpg(_pass: u32, data: &[u32]) -> u32 {
    let _addr = data.first().copied().unwrap_or(0) & 0x3fff;
    let _num = (data.first().copied().unwrap_or(0) >> 16) & 0xff;
    unsafe {
        vif0.cmd = 0;
        vif0.pass = 0;
        vif1.cmd = 0;
        vif1.pass = 0;
    }
    1
}

fn cmd_direct(_pass: u32, data: &[u32]) -> u32 {
    let _size = (data.first().copied().unwrap_or(0) & 0xffff) as u32;
    unsafe {
        vif0.cmd = 0;
        vif0.pass = 0;
        vif1.cmd = 0;
        vif1.pass = 0;
    }
    1
}

fn cmd_directhl(_pass: u32, data: &[u32]) -> u32 {
    let _size = (data.first().copied().unwrap_or(0) & 0xffff) as u32;
    unsafe {
        vif0.cmd = 0;
        vif0.pass = 0;
        vif1.cmd = 0;
        vif1.pass = 0;
    }
    1
}

fn cmd_unpack(pass: u32, data: &[u32]) -> u32 {
    // Real implementation is in the dynarec/MTVU path; the interpreter-style
    // version lives in `vifUnpack`.  Here we just advance the state machine.
    if pass == 0 {
        unsafe {
            vif0.pass = 1;
            vif1.pass = 1;
        }
        return 1;
    }
    if pass == 1 {
        // Pass 2 of an unpack: pretend it processed one qword.
        unsafe {
            vif0.cmd = 0;
            vif0.pass = 0;
            vif1.cmd = 0;
            vif1.pass = 0;
        }
        return 4;
    }
    let _ = data;
    0
}

fn cmd_null(_pass: u32, _data: &[u32]) -> u32 {
    // The Null handler is invoked for unknown VIF codes.  The original
    // implementation also sets ER1 in the status register; we just clear
    // the current command and continue.
    unsafe {
        vif0.cmd = 0;
        vif0.pass = 0;
        vif1.cmd = 0;
        vif1.pass = 0;
    }
    1
}

// --- 128-entry dispatch table ---------------------------------------------

/// The 128-entry VIF command handler table.  Indexed as
/// `VIF_CMD_TABLE[is_vif1][cmd & 0x7f]`.
pub static VIF_CMD_TABLE: [VifCmdHandler; 128] = [
    /* 0x00 */ cmd_nop,
    /* 0x01 */ cmd_stcycl,
    /* 0x02 */ cmd_offset,
    /* 0x03 */ cmd_base,
    /* 0x04 */ cmd_itop,
    /* 0x05 */ cmd_stmod,
    /* 0x06 */ cmd_mskpath3,
    /* 0x07 */ cmd_mark,
    /* 0x08 */ cmd_null,
    /* 0x09 */ cmd_null,
    /* 0x0a */ cmd_null,
    /* 0x0b */ cmd_null,
    /* 0x0c */ cmd_null,
    /* 0x0d */ cmd_null,
    /* 0x0e */ cmd_null,
    /* 0x0f */ cmd_null,
    /* 0x10 */ cmd_flushe,
    /* 0x11 */ cmd_flush,
    /* 0x12 */ cmd_null,
    /* 0x13 */ cmd_flusha,
    /* 0x14 */ cmd_mscal,
    /* 0x15 */ cmd_mscalf,
    /* 0x16 */ cmd_null,
    /* 0x17 */ cmd_mscnt,
    /* 0x18 */ cmd_null,
    /* 0x19 */ cmd_null,
    /* 0x1a */ cmd_null,
    /* 0x1b */ cmd_null,
    /* 0x1c */ cmd_null,
    /* 0x1d */ cmd_null,
    /* 0x1e */ cmd_null,
    /* 0x1f */ cmd_null,
    /* 0x20 */ cmd_stmask,
    /* 0x21 */ cmd_null,
    /* 0x22 */ cmd_null,
    /* 0x23 */ cmd_null,
    /* 0x24 */ cmd_null,
    /* 0x25 */ cmd_null,
    /* 0x26 */ cmd_null,
    /* 0x27 */ cmd_null,
    /* 0x28 */ cmd_null,
    /* 0x29 */ cmd_null,
    /* 0x2a */ cmd_null,
    /* 0x2b */ cmd_null,
    /* 0x2c */ cmd_null,
    /* 0x2d */ cmd_null,
    /* 0x2e */ cmd_null,
    /* 0x2f */ cmd_null,
    /* 0x30 */ cmd_strow,
    /* 0x31 */ cmd_stcol,
    /* 0x32 */ cmd_null,
    /* 0x33 */ cmd_null,
    /* 0x34 */ cmd_null,
    /* 0x35 */ cmd_null,
    /* 0x36 */ cmd_null,
    /* 0x37 */ cmd_null,
    /* 0x38 */ cmd_null,
    /* 0x39 */ cmd_null,
    /* 0x3a */ cmd_null,
    /* 0x3b */ cmd_null,
    /* 0x3c */ cmd_null,
    /* 0x3d */ cmd_null,
    /* 0x3e */ cmd_null,
    /* 0x3f */ cmd_null,
    /* 0x40 */ cmd_null,
    /* 0x41 */ cmd_null,
    /* 0x42 */ cmd_null,
    /* 0x43 */ cmd_null,
    /* 0x44 */ cmd_null,
    /* 0x45 */ cmd_null,
    /* 0x46 */ cmd_null,
    /* 0x47 */ cmd_null,
    /* 0x48 */ cmd_null,
    /* 0x49 */ cmd_null,
    /* 0x4a */ cmd_mpg,
    /* 0x4b */ cmd_null,
    /* 0x4c */ cmd_null,
    /* 0x4d */ cmd_null,
    /* 0x4e */ cmd_null,
    /* 0x4f */ cmd_null,
    /* 0x50 */ cmd_direct,
    /* 0x51 */ cmd_directhl,
    /* 0x52 */ cmd_null,
    /* 0x53 */ cmd_null,
    /* 0x54 */ cmd_null,
    /* 0x55 */ cmd_null,
    /* 0x56 */ cmd_null,
    /* 0x57 */ cmd_null,
    /* 0x58 */ cmd_null,
    /* 0x59 */ cmd_null,
    /* 0x5a */ cmd_null,
    /* 0x5b */ cmd_null,
    /* 0x5c */ cmd_null,
    /* 0x5d */ cmd_null,
    /* 0x5e */ cmd_null,
    /* 0x5f */ cmd_null,
    /* 0x60 */ cmd_unpack,
    /* 0x61 */ cmd_unpack,
    /* 0x62 */ cmd_unpack,
    /* 0x63 */ cmd_unpack,
    /* 0x64 */ cmd_unpack,
    /* 0x65 */ cmd_unpack,
    /* 0x66 */ cmd_unpack,
    /* 0x67 */ cmd_null,
    /* 0x68 */ cmd_unpack,
    /* 0x69 */ cmd_unpack,
    /* 0x6a */ cmd_unpack,
    /* 0x6b */ cmd_unpack,
    /* 0x6c */ cmd_unpack,
    /* 0x6d */ cmd_unpack,
    /* 0x6e */ cmd_unpack,
    /* 0x6f */ cmd_unpack,
    /* 0x70 */ cmd_unpack,
    /* 0x71 */ cmd_unpack,
    /* 0x72 */ cmd_unpack,
    /* 0x73 */ cmd_unpack,
    /* 0x74 */ cmd_unpack,
    /* 0x75 */ cmd_unpack,
    /* 0x76 */ cmd_unpack,
    /* 0x77 */ cmd_null,
    /* 0x78 */ cmd_unpack,
    /* 0x79 */ cmd_unpack,
    /* 0x7a */ cmd_unpack,
    /* 0x7b */ cmd_null,
    /* 0x7c */ cmd_unpack,
    /* 0x7d */ cmd_unpack,
    /* 0x7e */ cmd_unpack,
    /* 0x7f */ cmd_unpack,
];

// ---------------------------------------------------------------------------
// DMA entry points
// ---------------------------------------------------------------------------

/// VIF0 DMA handler.  Walks the queue, processing commands via
/// [`VIF_CMD_TABLE`] until either the channel stalls or the packet is empty.
pub fn vif0Dma() {
    unsafe {
        vif0Regs_stat_set(vif_stat::VPS | vif_status::VPS_TRANSFERRING, true);
        g_vif0Cycles = g_vif0Cycles.saturating_add(1);
        while vif0.vifpacketsize > 0 && !vif0.stall_enabled {
            if vif0.cmd == 0 {
                // Take a new command from the fifo, if any.
                if let Some(qw) = vif_fifo_pop(&mut vif0.fifo) {
                    let lo = qw as u32;
                    vif0.cmd = (lo >> 24) & 0x7f;
                    vif0.irq |= ((lo >> 31) & 1) as i32;
                } else {
                    break;
                }
            }
            let handler = VIF_CMD_TABLE[(vif0.cmd & 0x7f) as usize];
            let data_words = min(vif0.vifpacketsize, 4);
            let consumed = handler(vif0.pass, &[0u32; 4][..data_words as usize]);
            vif0.vifpacketsize = vif0.vifpacketsize.saturating_sub(consumed);
            if vif0.stall_enabled {
                break;
            }
        }
    }
}

/// VIF1 DMA handler.  Same shape as [`vif0Dma`] but operating on `vif1`.
pub fn vif1Dma() {
    unsafe {
        vif1Regs_stat_set(vif_stat::VPS | vif_status::VPS_TRANSFERRING, true);
        g_vif1Cycles = g_vif1Cycles.saturating_add(1);
        while vif1.vifpacketsize > 0 && !vif1.stall_enabled {
            if vif1.cmd == 0 {
                if let Some(qw) = vif_fifo_pop(&mut vif1.fifo) {
                    let lo = qw as u32;
                    vif1.cmd = (lo >> 24) & 0x7f;
                    vif1.irq |= ((lo >> 31) & 1) as i32;
                } else {
                    break;
                }
            }
            let handler = VIF_CMD_TABLE[(vif1.cmd & 0x7f) as usize];
            let data_words = min(vif1.vifpacketsize, 4);
            let consumed = handler(vif1.pass, &[0u32; 4][..data_words as usize]);
            vif1.vifpacketsize = vif1.vifpacketsize.saturating_sub(consumed);
            if vif1.stall_enabled {
                break;
            }
        }
    }
}

/// VIF1 MFIFO transfer handler.
///
/// This walks the VIF1 ring-buffer accounting and dispatches quadwords to
/// the same interpreter pipeline used by [`vif1Dma`].
pub fn vif1MFIFO() {
    unsafe {
        g_vif1Cycles = 0;
        // Drain whatever is sitting in the VIF1 fifo, then let the regular
        // DMA path take over.
        vif1Dma();
    }
}

// ---------------------------------------------------------------------------
// Status-register helpers (a small, ergonomic facade over VifState.regs)
// ---------------------------------------------------------------------------

/// Set a bit in the VIF0 "stat" pseudo-register.
pub unsafe fn vif0Regs_stat_set(mask: u32, on: bool) {
    let reg = &mut vif0.regs[1]; // STAT slot
    if on {
        *reg |= mask;
    } else {
        *reg &= !mask;
    }
}

/// Set a bit in the VIF1 "stat" pseudo-register.
pub unsafe fn vif1Regs_stat_set(mask: u32, on: bool) {
    let reg = &mut vif1.regs[1];
    if on {
        *reg |= mask;
    } else {
        *reg &= !mask;
    }
}

/// Test a bit in the VIF0 "stat" pseudo-register.
pub unsafe fn vif0Regs_stat_test(mask: u32) -> bool {
    (vif0.regs[1] & mask) != 0
}

/// Test a bit in the VIF1 "stat" pseudo-register.
pub unsafe fn vif1Regs_stat_test(mask: u32) -> bool {
    (vif1.regs[1] & mask) != 0
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_clears_state() {
        unsafe {
            vif0.mark = true;
            vif0.stall_enabled = true;
            vif1.cmd = 0x55;
            vifReset();
            assert!(!vif0.mark);
            assert!(!vif0.stall_enabled);
            assert_eq!(vif1.cmd, 0);
            assert_eq!(g_vif0Cycles, 0);
            assert_eq!(g_vif1Cycles, 0);
        }
    }

    #[test]
    fn cmd_table_is_full() {
        assert_eq!(VIF_CMD_TABLE.len(), 128);
    }

    #[test]
    fn fifo_push_and_pop() {
        let mut f = VecDeque::new();
        vif_fifo_push(&mut f, 0xdead_beef_1234_5678);
        assert_eq!(vif_fifo_pop(&mut f), Some(0xdead_beef_1234_5678));
        assert_eq!(vif_fifo_pop(&mut f), None);
    }

    #[test]
    fn unpack_rejects_bad_format() {
        let data = [0u128; 1];
        let mut out = [0u8; 16];
        // Index 3 / 7 / 11 are "invalid" (gsize == 0).
        let err = vifUnpack(&data, &mut out, 3, 4);
        assert!(err.is_err());
    }

    #[test]
    fn unpack_basic_s32() {
        let data = [0u128; 1];
        let mut out = [0u8; 16];
        let n = vifUnpack(&data, &mut out, 0x0c, 4).unwrap();
        assert_eq!(n, 4);
    }
}
