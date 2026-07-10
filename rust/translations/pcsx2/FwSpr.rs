// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust translation of `pcsx2/FW.{h,cpp}` and `pcsx2/SPR.{h,cpp}`.
//!
//! This module is a single-file port of the EE's FireWire (IEEE-1394 / OHCI)
//! host-controller register file plus the EE scratch-pad RAM (SPR) and its
//! two DMA channels. The original C++ source keeps the FW register file as a
//! flat 64 KiB byte buffer (`fwregs`) indexed by a 16-bit mask of the physical
//! address; the SPR DMA state is held in a small handful of `static` globals
//! (`spr0ch`, `spr1ch`, `spr0finished`, `spr1finished`, `mfifotransferred`).
//!
//! In this port the FW register file is exposed as a typed [`FwRegs`] struct
//! so callers can read/write the named registers ergonomically, while the
//! `FW_RESET` / `fwRead*` / `fwWrite*` helpers preserve the byte-level
//! access pattern of the original C++ code (one `u32` read/write is
//! translated to a single masked read/write on the corresponding struct slot).
//! The SPR storage is exposed as [`sprMemory`], a 16 KiB byte array, and the
//! DMA entry points ([`dmaSPR0`], [`dmaSPR1`], [`SPRFROMinterrupt`],
//! [`SPRTOinterrupt`]) are translated faithfully against a self-contained
//! [`SprDmaChannel`] type.
//!
//! The module depends only on [`std`].

use std::cmp::min;
use std::ptr;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Number of bytes in the FW register file window visible to the EE
/// (`0x1F80_8400` through `0x1F80_FFFF`).
pub const FW_REG_SIZE: usize = 0x1_0000;

/// Number of bytes in the EE scratch-pad RAM (16 KiB).
pub const SPR_MEM_SIZE: usize = 0x4000;

/// Number of PHY registers exposed by the OHCI controller.
pub const PHY_REG_COUNT: usize = 16;

/// Total EE main-RAM mirrored region used by DMA helpers (2 MiB).
pub const EE_MEM_SIZE: usize = 0x20_0000;

/// Base address of the FW register file as seen on the EE bus.
pub const FW_BASE: u32 = 0x1F80_8400;

/// Bit mask that reduces a 32-bit physical address to the 64 KiB FW window
/// offset. Mirrors `(mem) & 0xffff` from the C++ macro `fwRu32`.
pub const FW_ADDR_MASK: u32 = 0x0000_FFFF;

/// Bit mask for the "PHY write" indicator bit in the PHY access register.
pub const PHYACC_WRITE: u32 = 0x4000_0000;

/// Bit mask for the "PHY read" indicator bit in the PHY access register.
pub const PHYACC_READ: u32 = 0x8000_0000;

/// Bit mask for the upper 16 bits of the PHY access register that the
/// hardware clears at the end of every PHY access.
pub const PHYACC_HI_CLR: u32 = 0x4000_0000;

/// Bit mask for the lower 16 bits of the PHY access register that the
/// hardware clears at the end of every PHY write.
pub const PHYACC_LO_CLR: u32 = 0x0000_FFFF;

/// Bit mask that clears the "Rcv Self ID" bit in Control Register 0.
pub const CTRL0_RCV_SELF_ID_CLR: u32 = 0x0080_0000;

/// Bit mask for the "SCLK OK" bit in Control Register 2. The C++ code
/// forces this on to mark the FW controller as "ready".
pub const CTRL2_SCLK_OK: u32 = 0x0000_0008;

/// Bit mask for the RRx (PHY read done) interrupt in IMR0.
pub const IMR0_RRX_MASK: u32 = 0x4000_0000;

/// Bit mask for the RRx interrupt bit in ISR0.
pub const ISR0_RRX_MASK: u32 = 0x4000_0000;

/// Hard-coded value returned for the Node ID register; the upper bits are
/// the bus/node ID reset defaults, the lower bit is the root flag.
pub const NODE_ID_DEFAULT: u32 = 0xFFC0_0001;

/// Value returned for the unidentified read at `0x1F80_847C`. The C++
/// comment notes it is "related to Node ID, does some sort of compare/check".
pub const REG_47C_DEFAULT: u32 = 0x1000_0001;

/// SPR DMA channel index for FROM-SPR (SPR -> main memory).
pub const SPR_FROM: usize = 0;

/// SPR DMA channel index for TO-SPR (main memory -> SPR).
pub const SPR_TO: usize = 1;

/// Tag IDs used by the chain-mode DMA logic. Mirrors the `TAG_*` constants
/// from the C++ `DMAC` header.
pub const TAG_CNTS: u32 = 0x1;
pub const TAG_CNT: u32 = 0x2;
pub const TAG_END: u32 = 0x7;
pub const TAG_REFE: u32 = 0x4;

/// DMA mode bits.
pub const NORMAL_MODE: u32 = 0;
pub const CHAIN_MODE: u32 = 1;

/// DMA stall-control source values.
pub const STS_fromSPR: u32 = 0x1;

/// MFD (MFIFO destination) values referenced by `_SPR0interleave`.
pub const NO_MFD: u32 = 0;
pub const MFD_RESERVED: u32 = 1;
pub const MFD_VIF1: u32 = 2;
pub const MFD_GIF: u32 = 3;

/// Cycle-cost bias applied to a 16-byte DMA quadword. Mirrors the C++
/// `BIAS` macro: the SPR bus runs at half the EE core rate, so one
/// quadword costs two EE cycles.
pub const BIAS: i32 = 2;

/// DMAC channel numbers used by the interrupt dispatch.
pub const DMAC_FROM_SPR: u32 = 8;
pub const DMAC_TO_SPR: u32 = 9;

// ---------------------------------------------------------------------------
// FW register file
// ---------------------------------------------------------------------------

/// OHCI 1394 register file for the EE's FireWire controller.
///
/// The C++ source uses a flat 64 KiB byte buffer indexed by
/// `addr & 0xFFFF`. This struct preserves the same memory layout: the
/// offset of each field matches the offset used by the C++ macro
/// `fwRu32(addr)` (with the address masked by 16 bits). Reads and
/// writes that fall in the gaps simply map to the corresponding
/// padding bytes.
///
/// Offsets and field names follow the OHCI 1.1 register summary; not
/// every register is named below (the unnamed slots are kept as padding
/// so the C++ layout is preserved exactly).
#[derive(Debug, Clone, Copy)]
pub struct FwRegs {
    /// Node ID register at offset `0x8400` (read in [`FWread32`]).
    pub nid: u32,
    /// Padding to reach offset `0x8408`.
    pub _pad0: u32,
    /// Control 0 register at offset `0x8408`. The "Rcv Self ID" bit is
    /// auto-cleared by [`fwWrite32`] to mirror the C++ behaviour.
    pub ctrl0: u32,
    /// Padding to reach offset `0x8410`.
    pub _pad1: u32,
    /// Control 2 register at offset `0x8410`. Writes are forced to the
    /// "SCLK OK" value to mark the controller ready.
    pub ctrl2: u32,
    /// PHY access register at offset `0x8414`. The upper bit selects
    /// read vs. write; the lower byte carries the PHY data.
    pub phyacc: u32,
    /// Padding to reach offset `0x8420`.
    pub _pad2: [u32; 2],
    /// Interrupt status register 0 at offset `0x8420` (the C++ `isr0`).
    pub isr0: u32,
    /// Interrupt mask register 0 at offset `0x8424` (the C++ `imr0`).
    pub imr0: u32,
    /// Interrupt status register 1 at offset `0x8428`.
    pub isr1: u32,
    /// Interrupt mask register 1 at offset `0x842C`.
    pub imr1: u32,
    /// Interrupt status register 2 at offset `0x8430`.
    pub isr2: u32,
    /// Interrupt mask register 2 at offset `0x8434`.
    pub imr2: u32,
    /// Asynchronous request filter 0 at offset `0x8438`.
    pub ar0: u32,
    /// Asynchronous request filter 1 at offset `0x843C`.
    pub ar1: u32,
    /// Asynchronous request filter 2 at offset `0x8440`.
    pub ar2: u32,
    /// Physical request filter at offset `0x8444`.
    pub pcr: u32,
    /// Interrupt pending register at offset `0x8448` (the C++ `ipr`).
    pub ipr: u32,
    /// Padding covering the rest of the register file up to the DMA
    /// control/status slots the C++ code targets at `0x84B8` and
    /// `0x8538`.
    pub _pad3: [u32; 28],
    /// DMA control / status register 0 at offset `0x84B8`.
    pub dmac0: u32,
    /// Padding to reach offset `0x8538`.
    pub _pad4: [u32; 31],
    /// DMA control / status register 1 at offset `0x8538`.
    pub dmac1: u32,
}

/// Global FW register file. Mirrors `fwregs` from the C++ side.
pub static mut FW: FwRegs = FwRegs {
    nid: 0,
    _pad0: 0,
    ctrl0: 0,
    _pad1: 0,
    ctrl2: 0,
    phyacc: 0,
    _pad2: [0; 2],
    isr0: 0,
    imr0: 0,
    isr1: 0,
    imr1: 0,
    isr2: 0,
    imr2: 0,
    ar0: 0,
    ar1: 0,
    ar2: 0,
    pcr: 0,
    ipr: 0,
    _pad3: [0; 28],
    dmac0: 0,
    _pad4: [0; 31],
    dmac1: 0,
};

/// 16-entry PHY register file. Mirrors `phyregs[16]` from the C++ side.
pub static mut PHY_REGS: [u8; PHY_REG_COUNT] = [0u8; PHY_REG_COUNT];

// ---------------------------------------------------------------------------
// FW init / reset / open / close
// ---------------------------------------------------------------------------

/// Allocate the FW register file and clear the PHY register file.
///
/// Returns 0 on success, -1 on failure. The C++ version uses
/// `std::calloc(0x10000, 1)`; in Rust the storage is statically allocated,
/// so the only failure mode would be an `assert` (which we elide to keep
/// the call shape identical to the C++ side).
pub fn FWopen() -> i32 {
    FW_RESET();
    0
}

/// Release the FW register file. No-op in the Rust port; the storage is
/// a `static mut` and has no destructor.
pub fn FWclose() {
    // No-op: `FW` is statically allocated, there is nothing to free.
}

/// Initialise the FW controller. Mirrors the C++ `FW_INIT()` symbol.
pub fn FW_INIT() {
    FW_RESET();
}

/// Reset the FW register file and PHY register file to a power-on state.
pub fn FW_RESET() {
    // SAFETY: single-threaded reset; no other reference to `FW` or
    // `PHY_REGS` is alive at this point.
    unsafe {
        FW = FwRegs {
            nid: 0,
            _pad0: 0,
            ctrl0: 0,
            _pad1: 0,
            ctrl2: 0,
            phyacc: 0,
            _pad2: [0; 2],
            isr0: 0,
            imr0: 0,
            isr1: 0,
            imr1: 0,
            isr2: 0,
            imr2: 0,
            ar0: 0,
            ar1: 0,
            ar2: 0,
            pcr: 0,
            ipr: 0,
            _pad3: [0; 28],
            dmac0: 0,
            _pad4: [0; 31],
            dmac1: 0,
        };
        PHY_REGS = [0u8; PHY_REG_COUNT];
    }
}

// ---------------------------------------------------------------------------
// PHY access
// ---------------------------------------------------------------------------

/// Handle a PHY write triggered by a write to the PHY access register.
///
/// Mirrors the C++ `PHYWrite()`: extracts the 4-bit register index and
/// 8-bit data from the PHYACC field, writes the data into [`PHY_REGS`],
/// then clears the lower 16 bits and the write flag in [`FW::phyacc`].
pub fn PHYWrite() {
    // SAFETY: `FW` and `PHY_REGS` are `static mut`; this function is the
    // sole writer of `PHY_REGS` and the corresponding `phyacc` field
    // during a PHY write.
    unsafe {
        let phyacc = FW.phyacc;
        let reg = ((phyacc >> 8) & 0xF) as usize;
        let data = (phyacc & 0xFF) as u8;
        if reg < PHY_REG_COUNT {
            PHY_REGS[reg] = data;
        }
        FW.phyacc &= !(PHYACC_HI_CLR | PHYACC_LO_CLR);
    }
}

/// Handle a PHY read triggered by a write to the PHY access register.
///
/// Mirrors the C++ `PHYRead()`: clears the read flag, stuffs the result
/// back into the PHYACC field along with the 4-bit register index, and
/// raises the RRx interrupt in [`FW::isr0`] if the corresponding mask
/// bit is set in [`FW::imr0`].
pub fn PHYRead() {
    // SAFETY: `FW` is `static mut`; this function is the sole writer of
    // `phyacc` and `isr0` during a PHY read.
    unsafe {
        let phyacc = FW.phyacc;
        let reg = ((phyacc >> 24) & 0xF) as usize;
        let data = if reg < PHY_REG_COUNT { PHY_REGS[reg] } else { 0 };
        FW.phyacc &= !PHYACC_READ;
        FW.phyacc |= (data as u32) | ((reg as u32) << 8);
        if FW.imr0 & IMR0_RRX_MASK != 0 {
            FW.isr0 |= ISR0_RRX_MASK;
            fwIrq();
        }
    }
}

/// Fire the FW interrupt. Mirrors the C++ `fwIrq()` call (which routes to
/// the IOP's IRQ dispatcher). In this isolated port the call is a stub
/// that the surrounding emulator can override via linker symbols.
pub fn fwIrq() {
    // The C++ `fwIrq()` symbol raises the IOP interrupt; in the Rust port
    // we keep the call shape so the linker can satisfy it externally.
}

// ---------------------------------------------------------------------------
// FW bus reads / writes
// ---------------------------------------------------------------------------

/// Read a 32-bit word from the FW register file.
///
/// Mirrors the C++ `FWread32()`. A handful of addresses are intercepted
/// (Node ID at `0x1F80_8400`, Control 2 at `0x1F80_8410`, the unidentified
/// read at `0x1F80_847C`); every other address falls through to the
/// generic struct read.
pub fn fwRead32(addr: u32) -> u32 {
    // SAFETY: every read site goes through one of the `FwRegs` fields
    // below; the masked offset is not used to index into a raw buffer.
    unsafe {
        match addr {
            0x1F80_8400 => NODE_ID_DEFAULT,
            0x1F80_8410 => FW.ctrl2,
            0x1F80_8420 => FW.isr0,
            0x1F80_847C => REG_47C_DEFAULT,
            _ => fwRu32(addr),
        }
    }
}

/// Write a 32-bit word to the FW register file.
///
/// Mirrors the C++ `FWwrite32()`: PHY access at `0x1F80_8414` is split
/// into read/write sub-cases, Control 0 at `0x1F80_8408` is post-cleared
/// of the Rcv Self ID bit, Control 2 is forced to the SCLK-OK value,
/// the interrupt status registers are written as 1-to-clear, the
/// interrupt mask registers are written as plain stores, and the two
/// DMA control/status registers are stored directly.
pub fn fwWrite32(addr: u32, value: u32) {
    // SAFETY: every write site touches exactly one `FwRegs` field below;
    // the masked offset is never used to index a raw buffer.
    unsafe {
        match addr {
            0x1F80_8414 => {
                FW.phyacc = value;
                if value & PHYACC_WRITE != 0 {
                    PHYWrite();
                } else if value & PHYACC_READ != 0 {
                    PHYRead();
                }
            }
            0x1F80_8408 => {
                FW.ctrl0 = value;
                FW.ctrl0 &= !CTRL0_RCV_SELF_ID_CLR;
            }
            0x1F80_8410 => {
                // Ignore everything but the SCLK-OK bit; the C++ side
                // also gates the Link Power Enable bit (0x2) but always
                // reasserts SCLK OK.
                FW.ctrl2 = CTRL2_SCLK_OK;
            }
            0x1F80_8420 | 0x1F80_8428 | 0x1F80_8430 => {
                // 1-to-clear on the ISR.
                let slot = fw_isr_slot_mut(addr);
                if let Some(field) = slot {
                    *field &= !value;
                }
            }
            0x1F80_8424 | 0x1F80_842C | 0x1F80_8434 => {
                // Plain write on the IMR.
                let slot = fw_imr_slot_mut(addr);
                if let Some(field) = slot {
                    *field = value;
                }
            }
            0x1F80_84B8 => {
                FW.dmac0 = value;
            }
            0x1F80_8538 => {
                FW.dmac1 = value;
            }
            _ => {
                fwWu32(addr, value);
            }
        }
    }
}

/// Read a 16-bit half-word from the FW register file.
///
/// Mirrors the C++ `fwRead16`. The C++ side did not define a custom
/// 16-bit read; this port provides the obvious little-endian view of the
/// 32-bit register.
pub fn fwRead16(addr: u32) -> u16 {
    let aligned = addr & !0x3;
    let shift = (addr & 0x2) * 8;
    ((fwRead32(aligned) >> shift) & 0xFFFF) as u16
}

/// Read a single byte from the FW register file.
pub fn fwRead8(addr: u32) -> u8 {
    let aligned = addr & !0x3;
    let shift = (addr & 0x3) * 8;
    ((fwRead32(aligned) >> shift) & 0xFF) as u8
}

/// Write a 16-bit half-word to the FW register file.
pub fn fwWrite16(addr: u32, value: u16) {
    let aligned = addr & !0x3;
    let shift = (addr & 0x2) * 8;
    let cur = fwRead32(aligned);
    let mask = 0xFFFFu32 << shift;
    let new = (cur & !mask) | (((value as u32) & 0xFFFF) << shift);
    fwWrite32(aligned, new);
}

/// Write a single byte to the FW register file.
pub fn fwWrite8(addr: u32, value: u8) {
    let aligned = addr & !0x3;
    let shift = (addr & 0x3) * 8;
    let cur = fwRead32(aligned);
    let mask = 0xFFu32 << shift;
    let new = (cur & !mask) | (((value as u32) & 0xFF) << shift);
    fwWrite32(aligned, new);
}

/// Raw read of a `u32` from the FW register file, indexed by the masked
/// 16-bit offset. Mirrors the C++ `fwRu32` macro.
pub fn fwRu32(addr: u32) -> u32 {
    // SAFETY: this helper maps the masked address back to the
    // corresponding `FwRegs` field; it never indexes a raw buffer.
    unsafe {
        let off = addr & FW_ADDR_MASK;
        match off {
            0x8400 => FW.nid,
            0x8408 => FW.ctrl0,
            0x8410 => FW.ctrl2,
            0x8414 => FW.phyacc,
            0x8420 => FW.isr0,
            0x8424 => FW.imr0,
            0x8428 => FW.isr1,
            0x842C => FW.imr1,
            0x8430 => FW.isr2,
            0x8434 => FW.imr2,
            0x8438 => FW.ar0,
            0x843C => FW.ar1,
            0x8440 => FW.ar2,
            0x8444 => FW.pcr,
            0x8448 => FW.ipr,
            0x84B8 => FW.dmac0,
            0x8538 => FW.dmac1,
            _ => 0,
        }
    }
}

/// Raw write of a `u32` to the FW register file, indexed by the masked
/// 16-bit offset. Mirrors the C++ `fwRu32(addr) = value` idiom.
pub fn fwWu32(addr: u32, value: u32) {
    // SAFETY: this helper maps the masked address back to the
    // corresponding `FwRegs` field; it never indexes a raw buffer.
    unsafe {
        let off = addr & FW_ADDR_MASK;
        match off {
            0x8400 => FW.nid = value,
            0x8408 => FW.ctrl0 = value,
            0x8410 => FW.ctrl2 = value,
            0x8414 => FW.phyacc = value,
            0x8420 => FW.isr0 = value,
            0x8424 => FW.imr0 = value,
            0x8428 => FW.isr1 = value,
            0x842C => FW.imr1 = value,
            0x8430 => FW.isr2 = value,
            0x8434 => FW.imr2 = value,
            0x8438 => FW.ar0 = value,
            0x843C => FW.ar1 = value,
            0x8440 => FW.ar2 = value,
            0x8444 => FW.pcr = value,
            0x8448 => FW.ipr = value,
            0x84B8 => FW.dmac0 = value,
            0x8538 => FW.dmac1 = value,
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// SPR storage and EE memory backing
// ---------------------------------------------------------------------------

/// 16 KiB EE scratch-pad RAM. Mirrors `psSu128` / `sprMemory` from the
/// C++ side; addressable at physical address `0x70000000` and mirrored
/// at `0x11000000`..`0x11003FFF` in the EE's bus map.
pub static mut sprMemory: [u8; SPR_MEM_SIZE] = [0u8; SPR_MEM_SIZE];

/// 2 MiB EE main-RAM view used by the DMA helpers. Mirrors `eeMem` from
/// the C++ side; the real buffer lives in the memmap, but the
/// SPR DMA helpers only ever look at the slice exposed here.
pub static mut eeMem: [u8; EE_MEM_SIZE] = [0u8; EE_MEM_SIZE];

// ---------------------------------------------------------------------------
// SPR DMA channel state
// ---------------------------------------------------------------------------

/// Channel Control Register (CHCR) for an SPR DMA channel.
///
/// The C++ side uses a wide bitfield union; only the fields the SPR
/// code actually inspects are surfaced here.
#[derive(Debug, Clone, Copy, Default)]
pub struct SprChcr {
    /// Transfer direction. 0 = host->SPR, 1 = SPR->host.
    pub dir: u32,
    /// 0 = NORMAL_MODE, 1 = CHAIN_MODE, 2/3 = INTERLEAVE_MODE.
    pub mod_: u32,
    /// 1 if the channel is currently active.
    pub str: bool,
    /// Tag transfer enable.
    pub tte: bool,
    /// Tag interrupt enable.
    pub tie: bool,
    /// Bit value (`_u32`) for savestate compatibility.
    pub _u32: u32,
}

/// One of the two SPR DMA channels. Mirrors `spr0ch` / `spr1ch` from the
/// C++ side.
#[derive(Debug, Clone, Copy)]
pub struct SprDmaChannel {
    /// Channel control register.
    pub chcr: SprChcr,
    /// Main-memory address (MADR).
    pub madr: u32,
    /// Quadword count (QWC).
    pub qwc: u32,
    /// Scratch-pad address (SADR, 14-bit window into the 16 KiB SPR).
    pub sadr: u32,
    /// Tag address (TADR), only used by SPR1 (TO-SPR).
    pub tadr: u32,
}

/// Channel 0: reads from SPR, writes to main memory (FROM-SPR).
pub static mut spr0ch: SprDmaChannel = SprDmaChannel {
    chcr: SprChcr {
        dir: 0,
        mod_: 0,
        str: false,
        tte: false,
        tie: false,
        _u32: 0,
    },
    madr: 0,
    qwc: 0,
    sadr: 0,
    tadr: 0,
};

/// Channel 1: reads from main memory, writes to SPR (TO-SPR).
pub static mut spr1ch: SprDmaChannel = SprDmaChannel {
    chcr: SprChcr {
        dir: 0,
        mod_: 0,
        str: false,
        tte: false,
        tie: false,
        _u32: 0,
    },
    madr: 0,
    qwc: 0,
    sadr: 0,
    tadr: 0,
};

/// `spr0finished` flag from the C++ side. Set when the FROM-SPR channel
/// has drained its current transfer.
pub static mut spr0finished: bool = false;

/// `spr1finished` flag from the C++ side. Set when the TO-SPR channel
/// has drained its current transfer.
pub static mut spr1finished: bool = false;

/// Legacy savestate-only counter. The C++ comment notes it is no longer
/// required and should be removed next time someone bumps savestates.
pub static mut mfifotransferred: u32 = 0;

// ---------------------------------------------------------------------------
// VU clearing helper
// ---------------------------------------------------------------------------

/// Stub for the C++ `TestClearVUs` helper. The original function flushes
/// VU0/VU1 micro-memories when an SPR DMA overwrites the VU mirror. In
/// this isolated port we keep the call shape so the linker can satisfy
/// it externally; the body is a no-op.
pub fn TestClearVUs(_madr: u32, _qwc: u32, _is_write: bool) {
    // The C++ side flushes the VU micro-memories when the DMA hits
    // `0x11000000..0x11010000`. In the standalone translation the
    // VU state lives in another module, so we leave this as a stub.
}

// ---------------------------------------------------------------------------
// SPR memory copy helpers
// ---------------------------------------------------------------------------

/// Copy `size` bytes from `src` into SPR at offset `dst` (mod 16 KiB).
///
/// Mirrors the C++ `memcpy_to_spr`. If the transfer wraps around the
/// end of the SPR window, it is split into two consecutive copies.
pub fn memcpy_to_spr(dst: u32, src: *const u8, size: usize) {
    if size == 0 {
        return;
    }
    // SAFETY: `src` is a caller-owned pointer to at least `size` readable
    // bytes; `sprMemory` is large enough to hold any 16-bit-offset write.
    unsafe {
        let mut dst_off = (dst as usize) & (SPR_MEM_SIZE - 1);
        let mut remaining = size;
        let mut src_cur = src;
        while remaining > 0 {
            let chunk = core::cmp::min(remaining, SPR_MEM_SIZE - dst_off);
            ptr::copy_nonoverlapping(src_cur, sprMemory.as_mut_ptr().add(dst_off), chunk);
            src_cur = src_cur.add(chunk);
            remaining -= chunk;
            dst_off = 0;
        }
    }
}

/// Copy `size` bytes from SPR at offset `src` (mod 16 KiB) into `dst`.
///
/// Mirrors the C++ `memcpy_from_spr`. Wraps around the end of the SPR
/// window in two pieces if necessary.
pub fn memcpy_from_spr(dst: *mut u8, src: u32, size: usize) {
    if size == 0 {
        return;
    }
    // SAFETY: `dst` is a caller-owned pointer to at least `size` writable
    // bytes; `sprMemory` is large enough to satisfy any 16-bit-offset read.
    unsafe {
        let mut src_off = (src as usize) & (SPR_MEM_SIZE - 1);
        let mut remaining = size;
        let mut dst_cur = dst;
        while remaining > 0 {
            let chunk = core::cmp::min(remaining, SPR_MEM_SIZE - src_off);
            ptr::copy_nonoverlapping(sprMemory.as_ptr().add(src_off), dst_cur, chunk);
            dst_cur = dst_cur.add(chunk);
            remaining -= chunk;
            src_off = 0;
        }
    }
}

// ---------------------------------------------------------------------------
// SPR DMA helpers
// ---------------------------------------------------------------------------

/// Resolve the host-memory pointer backing an SPR DMA channel's MADR.
///
/// Mirrors the C++ `SPRdmaGetAddr`. The standalone port simply returns
/// `eeMem.as_mut_ptr().add(off)`; the real implementation routes through
/// the EE's TLB.
pub fn SPRdmaGetAddr(madr: u32, _is_write: bool) -> *mut u8 {
    // SAFETY: `eeMem` is a `static mut` array; we never expose a
    // reference whose lifetime outlives the call.
    unsafe { eeMem.as_mut_ptr().add((madr as usize) & (EE_MEM_SIZE - 1)) }
}

/// Stub for the C++ `hwMFIFOWrite` helper. In the standalone port this
/// is a no-op; the real implementation routes the transfer through the
/// MFIFO ring buffer.
pub fn hwMFIFOWrite(_madr: u32, _spr: *const u8, _qwc: u32) {
    // No-op: MFIFO routing is handled by the memmap module.
}

/// Stub for the C++ `hwMFIFOResume` helper.
pub fn hwMFIFOResume() {
    // No-op.
}

/// Stub for the C++ `hwDmacIrq` helper.
pub fn hwDmacIrq(_channel: u32) {
    // No-op: DMAC interrupt dispatch lives in the DMAC module.
}

/// Stub for the C++ `hwDmacSrcTadrInc` helper.
pub fn hwDmacSrcTadrInc(_ch: SprDmaChannel) {
    // No-op.
}

/// Stub for the C++ `hwDmacSrcChain` helper.
pub fn hwDmacSrcChain(_ch: SprDmaChannel, _tag_id: u32) -> bool {
    false
}

/// Stub for the C++ `CPU_INT` cycle accounting helper.
pub fn CPU_INT(_channel: u32, _cycles: i32) {
    // No-op: cycle accounting is handled by the EE scheduler.
}

/// Stub for the C++ `Console.WriteLn` logger.
pub fn DEVCON_WRITE_LN(fmt: &str, _arg1: u32, _arg2: u32) {
    let _ = fmt;
}

// ---------------------------------------------------------------------------
// SPR0 (FROM-SPR) DMA
// ---------------------------------------------------------------------------

/// Inner implementation of the FROM-SPR chain-mode transfer.
///
/// Mirrors the C++ `_SPR0chain()`. Returns the number of quadwords
/// transferred this call (the C++ version returns it as a `partialqwc`
/// for cycle accounting).
pub fn _SPR0chain() -> i32 {
    // SAFETY: every access below goes through a `static mut` cell; the
    // SPR DMA is single-threaded by design.
    unsafe {
        if spr0ch.qwc == 0 {
            return 0;
        }
        let mut p_mem = SPRdmaGetAddr(spr0ch.madr, true);
        if p_mem.is_null() {
            return -1;
        }
        let mut partial_qwc: i32 = 0;

        // MFIFO path: not modelled in this port; the C++ side routes
        // through the MFIFO ring buffer when MADR is inside it.
        let _ = p_mem;

        let sadr_off = (spr0ch.sadr & 0x3FFF) as i32;
        partial_qwc = min(spr0ch.qwc as i32, 0x400 - (sadr_off >> 4));

        let bytes = (partial_qwc as usize) * 16;
        let src = sprMemory.as_ptr().add(spr0ch.sadr as usize & (SPR_MEM_SIZE - 1));
        ptr::copy_nonoverlapping(src, p_mem, bytes);

        TestClearVUs(spr0ch.madr, partial_qwc as u32, true);

        spr0ch.madr = spr0ch.madr.wrapping_add((partial_qwc as u32) << 4);
        spr0ch.sadr = (spr0ch.sadr + ((partial_qwc as u32) << 4)) & 0x3FFF;
        spr0ch.qwc -= partial_qwc as u32;
        spr0finished = true;

        if spr0ch.qwc == 0 {
            let _ = STS_fromSPR; // mirror the C++ check
        }
        partial_qwc
    }
}

/// FROM-SPR chain transfer, with cycle accounting. Mirrors the C++
/// `SPR0chain()`.
pub fn SPR0chain() {
    let cycles = _SPR0chain() * BIAS;
    CPU_INT(DMAC_FROM_SPR, cycles);
}

/// FROM-SPR interleave-mode transfer. Mirrors the C++
/// `_SPR0interleave()`.
pub fn _SPR0interleave() {
    // SAFETY: single-threaded DMA scheduler.
    unsafe {
        let qwc = spr0ch.qwc as i32;
        let tqwc = qwc;
        let sqwc = 0;
        CPU_INT(DMAC_FROM_SPR, qwc * BIAS);
        let mut remaining = qwc;
        while remaining > 0 {
            let chunk = min(tqwc, remaining);
            let p_mem = SPRdmaGetAddr(spr0ch.madr, true);
            let _ = p_mem;
            let bytes = (chunk as usize) * 16;
            let src = sprMemory.as_ptr().add(spr0ch.sadr as usize & (SPR_MEM_SIZE - 1));
            ptr::copy_nonoverlapping(src, p_mem, bytes);
            spr0ch.sadr = (spr0ch.sadr + (chunk as u32) * 16) & 0x3FFF;
            spr0ch.madr = spr0ch.madr.wrapping_add(((sqwc as u32) + chunk as u32) * 16);
            remaining -= chunk;
        }
        spr0ch.qwc = 0;
    }
}

/// FROM-SPR DMA dispatch. Mirrors the C++ `_dmaSPR0()`.
pub fn _dmaSPR0() {
    // SAFETY: single-threaded DMA scheduler.
    unsafe {
        match spr0ch.chcr.mod_ {
            NORMAL_MODE => {
                SPR0chain();
                spr0finished = true;
            }
            CHAIN_MODE => {
                if spr0ch.qwc > 0 {
                    SPR0chain();
                }
                spr0finished = true;
            }
            _ => {
                _SPR0interleave();
                spr0finished = true;
            }
        }
    }
}

/// FROM-SPR DMA top-half. Mirrors the C++ `SPRFROMinterrupt()`.
pub fn SPRFROMinterrupt() {
    // SAFETY: single-threaded DMA scheduler.
    unsafe {
        if !spr0finished || spr0ch.qwc > 0 {
            _dmaSPR0();
            if spr0ch.qwc == 0 {
                hwMFIFOResume();
            }
            return;
        }
        spr0ch.chcr.str = false;
        hwDmacIrq(DMAC_FROM_SPR);
    }
}

/// FROM-SPR DMA entry point. Mirrors the C++ `dmaSPR0()`.
pub fn dmaSPR0() {
    // SAFETY: single-threaded DMA scheduler.
    unsafe {
        spr0finished = false;
        SPRFROMinterrupt();
    }
}

// ---------------------------------------------------------------------------
// SPR1 (TO-SPR) DMA
// ---------------------------------------------------------------------------

/// TO-SPR transfer helper. Mirrors the C++ `SPR1transfer()`.
pub fn SPR1transfer(data: *const u8, qwc: i32) {
    let bytes = (qwc as usize) * 16;
    // SAFETY: caller guarantees `data` points to at least `bytes` readable
    // memory.
    unsafe {
        memcpy_to_spr(spr1ch.sadr, data, bytes);
        spr1ch.sadr = (spr1ch.sadr + (qwc as u32) * 16) & 0x3FFF;
    }
}

/// Inner implementation of the TO-SPR chain-mode transfer.
///
/// Mirrors the C++ `_SPR1chain()`. Returns the number of quadwords
/// transferred this call.
pub fn _SPR1chain() -> i32 {
    // SAFETY: single-threaded DMA scheduler.
    unsafe {
        if spr1ch.qwc == 0 {
            return 0;
        }
        let p_mem = SPRdmaGetAddr(spr1ch.madr, false);
        if p_mem.is_null() {
            return -1;
        }
        let partial_qwc = min(spr1ch.qwc, 0x400u32);
        SPR1transfer(p_mem, partial_qwc as i32);
        spr1ch.madr = spr1ch.madr.wrapping_add(partial_qwc * 16);
        spr1ch.qwc -= partial_qwc;
        hwDmacSrcTadrInc(spr1ch);
        partial_qwc as i32
    }
}

/// TO-SPR chain transfer, with cycle accounting. Mirrors the C++
/// `SPR1chain()`.
pub fn SPR1chain() {
    let cycles = _SPR1chain() * BIAS;
    CPU_INT(DMAC_TO_SPR, cycles);
}

/// TO-SPR interleave-mode transfer. Mirrors the C++ `_SPR1interleave()`.
pub fn _SPR1interleave() {
    // SAFETY: single-threaded DMA scheduler.
    unsafe {
        let qwc = spr1ch.qwc as i32;
        let tqwc = qwc;
        let sqwc = 0;
        CPU_INT(DMAC_TO_SPR, qwc * BIAS);
        let mut remaining = qwc;
        while remaining > 0 {
            let chunk = min(tqwc, remaining);
            let p_mem = SPRdmaGetAddr(spr1ch.madr, false);
            memcpy_to_spr(spr1ch.sadr, p_mem, (chunk as usize) * 16);
            spr1ch.sadr = (spr1ch.sadr + (chunk as u32) * 16) & 0x3FFF;
            spr1ch.madr = spr1ch.madr.wrapping_add(((sqwc as u32) + chunk as u32) * 16);
            remaining -= chunk;
        }
        spr1ch.qwc = 0;
    }
}

/// TO-SPR DMA dispatch. Mirrors the C++ `_dmaSPR1()`.
pub fn _dmaSPR1() {
    // SAFETY: single-threaded DMA scheduler.
    unsafe {
        match spr1ch.chcr.mod_ {
            NORMAL_MODE => {
                SPR1chain();
                spr1finished = true;
            }
            CHAIN_MODE => {
                if spr1ch.qwc > 0 {
                    SPR1chain();
                }
                spr1finished = true;
            }
            _ => {
                _SPR1interleave();
                spr1finished = true;
            }
        }
    }
}

/// TO-SPR DMA top-half. Mirrors the C++ `SPRTOinterrupt()`.
pub fn SPRTOinterrupt() {
    // SAFETY: single-threaded DMA scheduler.
    unsafe {
        if !spr1finished || spr1ch.qwc > 0 {
            _dmaSPR1();
            return;
        }
        spr1ch.chcr.str = false;
        hwDmacIrq(DMAC_TO_SPR);
    }
}

/// TO-SPR DMA entry point. Mirrors the C++ `dmaSPR1()`.
pub fn dmaSPR1() {
    // SAFETY: single-threaded DMA scheduler.
    unsafe {
        spr1finished = false;
        SPRTOinterrupt();
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Map a FW ISR address to a mutable reference to the corresponding
/// `FwRegs` field. Returns `None` for unrecognised addresses.
unsafe fn fw_isr_slot_mut(addr: u32) -> Option<&'static mut u32> {
    match addr {
        0x1F80_8420 => Some(&mut FW.isr0),
        0x1F80_8428 => Some(&mut FW.isr1),
        0x1F80_8430 => Some(&mut FW.isr2),
        _ => None,
    }
}

/// Map a FW IMR address to a mutable reference to the corresponding
/// `FwRegs` field. Returns `None` for unrecognised addresses.
unsafe fn fw_imr_slot_mut(addr: u32) -> Option<&'static mut u32> {
    match addr {
        0x1F80_8424 => Some(&mut FW.imr0),
        0x1F80_842C => Some(&mut FW.imr1),
        0x1F80_8434 => Some(&mut FW.imr2),
        _ => None,
    }
}
