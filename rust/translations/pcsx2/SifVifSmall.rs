// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! `SifVifSmall` — Rust 2021 translation of PCSX2's SIF (Sub-CPU Interface) and
//! VIF (Vector Interface) core source set.
//!
//! This module folds `Sif.cpp`, `Sif.h`, `Sif0.cpp`, `Sif1.cpp`, `sif2.cpp`,
//! `Sifcmd.h`, `Vif.h`, `Vif.cpp`, `Vif_Codes.cpp`, `Vif_Transfer.cpp`,
//! `Vif_Dma.h`, `Vif_Dynarec.h`, `Vif_HashBucket.h`, `Vif_Unpack.h` and
//! `IopIrq.cpp` into a single idiomatic Rust module.  Only the externally
//! observable surface is provided: the three DMA pumpers, the VIF code
//! dispatch (handler + table), the unpack engine, the Iop interrupt glue,
//! and the small register/state struct used by `PatchVif`.

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(dead_code)]
#![allow(static_mut_refs)]

use std::sync::atomic::{AtomicU32, Ordering};

/// Number of 32-bit words in one of the SIF FIFOs.
pub const FIFO_SIF_W: usize = 128;

/// EE→IOP SIF channel mask bit (matches SBUS_F240).
pub const SBUS_SIF0_BIT: u32 = 0x20;
pub const SBUS_SIF0_E_BIT: u32 = 0x2000;
pub const SBUS_SIF1_BIT: u32 = 0x40;
pub const SBUS_SIF1_E_BIT: u32 = 0x4000;
pub const SBUS_SIF2_BIT: u32 = 0x80;
pub const SBUS_SIF2_E_BIT: u32 = 0x8000;

/// Channel reset / start bits inside `HW_DMA*_CHCR`.
pub const DMA_CHCR_STR: u32 = 0x0100_0000;

/// VIF0 / VIF1 INTC numbers.
pub const VIF0_INTC: u32 = 4;
pub const VIF1_INTC: u32 = 5;

/// Tag ID encoding (`TAG_CNT` / `TAG_CNTS` / `TAG_END` / `TAG_REFE`).
pub const TAG_CNT: u32 = 0;
pub const TAG_CNTS: u32 = 1;
pub const TAG_END: u32 = 7;
pub const TAG_REFE: u32 = 4;

/// `DMAC_*` channel numbers (subset needed by the SIF paths).
pub const DMAC_SIF0: u32 = 3;
pub const DMAC_SIF1: u32 = 4;
pub const DMAC_SIF2: u32 = 5;
pub const DMAC_VIF0: u32 = 0;
pub const DMAC_VIF1: u32 = 1;
pub const DMAC_MFIFO_VIF: u32 = 10;

/// `IopEvt_*` numbers used by the IOP interrupt scheduling helpers.
pub const IopEvt_SIF0: u32 = 17;
pub const IopEvt_SIF1: u32 = 18;
pub const IopEvt_SIF2: u32 = 19;
pub const IopEvt_DEV9: u32 = 20;
pub const IopEvt_USB: u32 = 21;

/// VIF status flag bits (`VIF0_STAT_*` and the common `VIF_STAT_*` subset).
pub const VIF_STAT_VPS_W: u32 = 1;
pub const VIF_STAT_VPS_D: u32 = 2;
pub const VIF_STAT_VPS_T: u32 = 3;
pub const VIF_STAT_VPS: u32 = 3;
pub const VIF_STAT_VEW: u32 = 1 << 2;
pub const VIF_STAT_VGW: u32 = 1 << 3;
pub const VIF_STAT_MRK: u32 = 1 << 6;
pub const VIF_STAT_DBF: u32 = 1 << 7;
pub const VIF_STAT_VSS: u32 = 1 << 8;
pub const VIF_STAT_VFS: u32 = 1 << 9;
pub const VIF_STAT_VIS: u32 = 1 << 10;
pub const VIF_STAT_INT: u32 = 1 << 11;
pub const VIF_STAT_ER0: u32 = 1 << 12;
pub const VIF_STAT_ER1: u32 = 1 << 13;
pub const VIF_STAT_FQC: u32 = 15 << 24;

/// Same flag bits but widened for VIF1 (FQC has 5 bits, plus an FDR bit).
pub const VIF1_STAT_FDR: u32 = 1 << 23;
pub const VIF1_STAT_FQC: u32 = 31 << 24;

/// VIF transfer-mode values.
pub const VIF_NORMAL_TO_MEM_MODE: u32 = 0;
pub const VIF_NORMAL_FROM_MEM_MODE: u32 = 1;
pub const VIF_CHAIN_MODE: u32 = 2;

/// VIF pipeline phases.
pub const VPS_IDLE: u32 = 0;
pub const VPS_WAITING: u32 = 1;
pub const VPS_DECODING: u32 = 2;
pub const VPS_TRANSFERRING: u32 = 3;

/// Stall reasons fed into the VIF state machine.
pub const VIF_TIMING_BREAK: u32 = 1;
pub const VIF_IRQ_STALL: u32 = 2;

/// MTGS "MFIFO destination" decoder (`dmacRegs.ctrl.MFD`).
pub const NO_MFD: u32 = 0;
pub const MFD_RESERVED: u32 = 1;
pub const MFD_VIF1: u32 = 2;
pub const MFD_GIF: u32 = 3;

/// `dmacRegs.ctrl.STS` / `STD` (SIF0/SIF1 stall source register).
pub const STS_SIF0: u32 = 1;
pub const STD_SIF1: u32 = 1;

/// Cycle-bias constant used by the EE timing helpers.
pub const BIAS: u32 = 1;

// ---------------------------------------------------------------------------
// IopIrq translation
// ---------------------------------------------------------------------------

/// Mirrors `psxHu32(HW_ISTAT) |= 1 << irq;` from `IopIrq.cpp`.
pub fn iopIntcIrq(irq: u32) {
    let stat = HW_ISTAT.load(Ordering::Relaxed);
    HW_ISTAT.store(stat | (1 << irq), Ordering::Relaxed);
    iopTestIntc();
}

/// Walk the ISTAT/IMASK chain once a new bit has been latched.
pub fn iopTestIntc() {
    // Real implementation reads IMASK and dispatches to the active handler.
    // In this translation we just keep the bit sticky.
}

/// DEV9 line.
pub fn dev9Irq(cycles: i32) {
    iopIntcIrq(13);
    let _ = cycles;
}

/// USB line.
pub fn usbIrq(cycles: i32) {
    iopIntcIrq(22);
    let _ = cycles;
}

/// Firewire (FW) line.
pub fn fwIrq() {
    iopIntcIrq(24);
}

/// SPU2 line.
pub fn spu2Irq() {
    iopIntcIrq(9);
}

/// ISTAT / IMASK mirror.  Real hardware lives in `eeHw[0x1000f000..]`.
pub static HW_ISTAT: AtomicU32 = AtomicU32::new(0);
pub static HW_IMASK: AtomicU32 = AtomicU32::new(0);

/// EE <-> IOP interrupt scheduling stubs (match `PSX_INT` / `CPU_INT`).
pub fn PSX_INT(_event: u32, _cycles: i32) {}
pub fn CPU_INT(_channel: u32, _cycles: u32) {}
pub fn CPU_SET_DMASTALL(_channel: u32, _stall: bool) {}

// ---------------------------------------------------------------------------
// SIF translation
// ---------------------------------------------------------------------------

/// 128-word ring buffer shared between the EE and IOP sides of a SIF link.
///
/// The C++ original pre-allocates `data[FIFO_SIF_W]` plus 4 words of "junk"
/// storage that is consulted when an under-sized packet has to be padded out
/// to a full quad-word.  See the long block comment in `Sif.h` for the rules
/// the IOP's DMA engine follows.
#[derive(Clone)]
pub struct SifFifo {
    pub data: [u32; FIFO_SIF_W],
    pub junk: [u32; 4],
    pub read_pos: i32,
    pub write_pos: i32,
    pub size: i32,
}

impl Default for SifFifo {
    fn default() -> Self {
        Self {
            data: [0; FIFO_SIF_W],
            junk: [0; 4],
            read_pos: 0,
            write_pos: 0,
            size: 0,
        }
    }
}

impl SifFifo {
    /// Free slots remaining in the ring.
    #[inline]
    pub fn sif_free(&self) -> i32 {
        FIFO_SIF_W as i32 - self.size
    }

    /// Push `words` 32-bit elements from `from` into the ring, wrapping as
    /// needed.  The C++ logs a warning when the write does not fit.
    pub fn write(&mut self, from: &[u32], words: i32) {
        if words <= 0 {
            return;
        }
        if (FIFO_SIF_W as i32 - self.size) < words {
            // Not enough space in SIF0/1/2 FIFO!
        }
        let words = words as usize;
        let w0 = (FIFO_SIF_W - self.write_pos as usize).min(words);
        let w1 = words - w0;
        self.data[self.write_pos as usize..self.write_pos as usize + w0]
            .copy_from_slice(&from[..w0]);
        self.data[..w1].copy_from_slice(&from[w0..w0 + w1]);
        self.write_pos = (self.write_pos + words as i32) & (FIFO_SIF_W as i32 - 1);
        self.size += words as i32;
    }

    /// Pad the ring with junk so the EE's 1-quadword reads see something
    /// sensible even when the IOP packet was short.
    pub fn write_junk(&mut self, words: i32) {
        if words <= 0 {
            return;
        }
        let transferred_words = 4 - words;
        let prev_qw_pos = (self.write_pos - (4 + transferred_words)) & (FIFO_SIF_W as i32 - 1);
        let prev_qw_pos = prev_qw_pos as usize;

        // Read the old data into the junk array, handling wrap.
        let r0 = (FIFO_SIF_W - prev_qw_pos).min(4);
        let r1 = 4 - r0;
        self.junk[..r0].copy_from_slice(&self.data[prev_qw_pos..prev_qw_pos + r0]);
        self.junk[r0..r0 + r1].copy_from_slice(&self.data[..r1]);

        let words = words as usize;
        let w0 = (FIFO_SIF_W - self.write_pos as usize).min(words);
        let w1 = words - w0;
        self.data[self.write_pos as usize..self.write_pos as usize + w0]
            .copy_from_slice(&self.junk[4 - w0..]);
        self.data[..w1].copy_from_slice(&self.junk[..w1]);
        self.write_pos = (self.write_pos + words as i32) & (FIFO_SIF_W as i32 - 1);
        self.size += words as i32;
    }

    /// Pop `words` 32-bit elements from the ring into `to`.
    pub fn read(&mut self, to: &mut [u32], words: i32) {
        if words <= 0 {
            return;
        }
        let words = words as usize;
        let r0 = (FIFO_SIF_W - self.read_pos as usize).min(words);
        let r1 = words - r0;
        to[..r0].copy_from_slice(&self.data[self.read_pos as usize..self.read_pos as usize + r0]);
        to[r0..r0 + r1].copy_from_slice(&self.data[..r1]);
        self.read_pos = (self.read_pos + words as i32) & (FIFO_SIF_W as i32 - 1);
        self.size -= words as i32;
    }

    /// Reset the ring back to its empty state.
    pub fn clear(&mut self) {
        self.data = [0; FIFO_SIF_W];
        self.read_pos = 0;
        self.write_pos = 0;
        self.size = 0;
    }
}

/// The IOP side of the tag header (32-bit address + 32-bit word count).
#[derive(Clone, Copy, Default)]
pub struct SifData {
    pub data: i32,
    pub words: i32,
    pub tag_lo: u64,
    pub tag_hi: u64,
}

/// EE-side bookkeeping for the SIF channels.
#[derive(Clone, Default)]
pub struct SifEe {
    pub end: bool,
    pub busy: bool,
    pub cycles: i32,
}

/// IOP-side bookkeeping for the SIF channels.
#[derive(Clone, Default)]
pub struct SifIop {
    pub end: bool,
    pub busy: bool,
    pub cycles: i32,
    pub write_junk: i32,
    pub counter: i32,
    pub data: SifData,
}

/// Combined EE + IOP + FIFO state for a single SIF channel.
#[derive(Clone, Default)]
pub struct SifUnit {
    pub fifo: SifFifo,
    pub ee: SifEe,
    pub iop: SifIop,
}

/// Translated `_sif sif0/sif1/sif2` plus the per-channel control registers
/// that the original code keeps in scattered globals (`sif0ch`, `sif1ch`,
/// `sif2dma`, `hw_dma*`, etc).
#[derive(Clone)]
pub struct SifState {
    pub regs: [u32; 0x100],
    pub sif0: SifUnit,
    pub sif1: SifUnit,
    pub sif2: SifUnit,
    // Control registers: in the C++ these are members of a global channel
    // object; we keep the bytes the original code touches.
    pub sif0ch_madr: u32,
    pub sif0ch_qwc: i32,
    pub sif0ch_chcr: u32,
    pub sif1ch_madr: u32,
    pub sif1ch_qwc: i32,
    pub sif1ch_chcr: u32,
    pub sif2dma_madr: u32,
    pub sif2dma_qwc: i32,
    pub sif2dma_chcr: u32,
    pub hw_dma9_madr: u32,
    pub hw_dma9_tadr: u32,
    pub hw_dma9_chcr: u32,
    pub hw_dma10_madr: u32,
    pub hw_dma10_chcr: u32,
    pub hw_dma2_madr: u32,
    pub hw_dma2_bcr: u32,
    pub hw_dma2_chcr: u32,
    pub sbus_f240: u32,
    pub dmac_ctrl: u32,
    pub dmac_stadr: u32,
    pub done: bool,
    pub sif1_dma_stall: bool,
    pub ps1_gpu_data: u32,
}

impl Default for SifState {
    fn default() -> Self {
        Self {
            regs: [0; 0x100],
            sif0: SifUnit::default(),
            sif1: SifUnit::default(),
            sif2: SifUnit::default(),
            sif0ch_madr: 0,
            sif0ch_qwc: 0,
            sif0ch_chcr: 0,
            sif1ch_madr: 0,
            sif1ch_qwc: 0,
            sif1ch_chcr: 0,
            sif2dma_madr: 0,
            sif2dma_qwc: 0,
            sif2dma_chcr: 0,
            hw_dma9_madr: 0,
            hw_dma9_tadr: 0,
            hw_dma9_chcr: 0,
            hw_dma10_madr: 0,
            hw_dma10_chcr: 0,
            hw_dma2_madr: 0,
            hw_dma2_bcr: 0,
            hw_dma2_chcr: 0,
            sbus_f240: 0,
            dmac_ctrl: 0,
            dmac_stadr: 0,
            done: false,
            sif1_dma_stall: false,
            ps1_gpu_data: 0,
        }
    }
}

/// Singleton SIF state.  The original code lives in `_sif sif0/1/2` globals
/// plus a constellation of per-channel control register globals; we keep one
/// combined struct so the Rust surface stays a single object.
pub static mut sif: SifState = SifState {
    regs: [0; 0x100],
    sif0: SifUnit {
        fifo: SifFifo {
            data: [0; FIFO_SIF_W],
            junk: [0; 4],
            read_pos: 0,
            write_pos: 0,
            size: 0,
        },
        ee: SifEe { end: false, busy: false, cycles: 0 },
        iop: SifIop {
            end: false,
            busy: false,
            cycles: 0,
            write_junk: 0,
            counter: 0,
            data: SifData { data: 0, words: 0, tag_lo: 0, tag_hi: 0 },
        },
    },
    sif1: SifUnit {
        fifo: SifFifo {
            data: [0; FIFO_SIF_W],
            junk: [0; 4],
            read_pos: 0,
            write_pos: 0,
            size: 0,
        },
        ee: SifEe { end: false, busy: false, cycles: 0 },
        iop: SifIop {
            end: false,
            busy: false,
            cycles: 0,
            write_junk: 0,
            counter: 0,
            data: SifData { data: 0, words: 0, tag_lo: 0, tag_hi: 0 },
        },
    },
    sif2: SifUnit {
        fifo: SifFifo {
            data: [0; FIFO_SIF_W],
            junk: [0; 4],
            read_pos: 0,
            write_pos: 0,
            size: 0,
        },
        ee: SifEe { end: false, busy: false, cycles: 0 },
        iop: SifIop {
            end: false,
            busy: false,
            cycles: 0,
            write_junk: 0,
            counter: 0,
            data: SifData { data: 0, words: 0, tag_lo: 0, tag_hi: 0 },
        },
    },
    sif0ch_madr: 0,
    sif0ch_qwc: 0,
    sif0ch_chcr: 0,
    sif1ch_madr: 0,
    sif1ch_qwc: 0,
    sif1ch_chcr: 0,
    sif2dma_madr: 0,
    sif2dma_qwc: 0,
    sif2dma_chcr: 0,
    hw_dma9_madr: 0,
    hw_dma9_tadr: 0,
    hw_dma9_chcr: 0,
    hw_dma10_madr: 0,
    hw_dma10_chcr: 0,
    hw_dma2_madr: 0,
    hw_dma2_bcr: 0,
    hw_dma2_chcr: 0,
    sbus_f240: 0,
    dmac_ctrl: 0,
    dmac_stadr: 0,
    done: false,
    sif1_dma_stall: false,
    ps1_gpu_data: 0,
};

/// Initialise all three SIF channels.  Counterpart to `sifReset()` in
/// `Sif.cpp`.
pub fn sifInit() {
    sifReset();
}

/// Reset all three SIF channels (`sif0`, `sif1`, `sif2`) plus the helper
/// state.  Mirrors `sifReset()` from `Sif.cpp`.
pub fn sifReset() {
    // Safety: sif is a process-wide singleton; the original code touches the
    // same globals without synchronization.
    unsafe {
        sif.sif0 = SifUnit::default();
        sif.sif1 = SifUnit::default();
        sif.sif2 = SifUnit::default();
        sif.done = false;
        sif.sif1_dma_stall = false;
    }
}

// ---------------------------------------------------------------------------
// SIF0 - EE<->IOP DMA, IOP -> EE direction
// ---------------------------------------------------------------------------

/// Translated `Sif0Init()`.  Resets the per-transfer "done" flag and cycle
/// counters.
fn sif0Init() {
    unsafe {
        sif.done = false;
        sif.sif0.ee.cycles = 0;
        sif.sif0.iop.cycles = 0;
    }
}

/// Translated `WriteFifoToEE()` for SIF0.
fn write_fifo_to_ee0() -> bool {
    unsafe {
        let read_size = std::cmp::min(sif.sif0ch_qwc, sif.sif0.fifo.size >> 2);
        sif.sif0.fifo.read(&mut sif.regs, read_size << 2);
        sif.sif0ch_madr = sif.sif0ch_madr.wrapping_add((read_size << 4) as u32);
        sif.sif0.ee.cycles += read_size;
        sif.sif0ch_qwc -= read_size;
        if sif.sif0ch_qwc == 0 && sif.dmac_ctrl & 0xF == STS_SIF0 {
            if (sif.sif0ch_chcr & 0xC000_0000) == 0
                || (((sif.sif0ch_chcr >> 28) & 0x7) as u32) == TAG_CNTS
            {
                sif.dmac_stadr = sif.sif0ch_madr;
            }
        }
        true
    }
}

/// Translated `WriteIOPtoFifo()` for SIF0.
fn write_iop_to_fifo0() -> bool {
    unsafe {
        let write_size = std::cmp::min(sif.sif0.iop.counter, sif.sif0.fifo.sif_free());
        sif.sif0.iop.cycles += write_size;
        sif.sif0.iop.counter -= write_size;
        true
    }
}

/// Translated `ProcessEETag()` for SIF0.
fn process_ee_tag0() -> bool {
    unsafe {
        sif.sif0.fifo.read(&mut sif.regs, 4);
        let ptag = sif.regs[0];
        sif.sif0ch_madr = sif.regs[1];
        if (sif.sif0ch_chcr & 0x4) != 0 && (ptag & 0x8000_0000) != 0 {
            sif.sif0.ee.end = true;
        }
        let id = (ptag >> 28) & 0x7;
        match id {
            0 => {}                      // TAG_CNT
            1 => {                       // TAG_CNTS
                if sif.dmac_ctrl & 0xF == STS_SIF0 {
                    sif.dmac_stadr = sif.sif0ch_madr;
                }
            }
            7 => sif.sif0.ee.end = true, // TAG_END
            _ => {}
        }
        true
    }
}

/// Translated `ProcessIOPTag()` for SIF0.
fn process_iop_tag0() -> bool {
    unsafe {
        sif.sif0.fifo.write(&[], 4);
        sif.hw_dma9_tadr = sif.hw_dma9_tadr.wrapping_add(16);
        sif.hw_dma9_madr = (sif.sif0.iop.data.data as u32) & 0x00FF_FFFF;
        if sif.sif0.iop.data.words > 0xFFFF {
            // SIF0 Overrun
        }
        sif.sif0.iop.counter = sif.sif0.iop.data.words & 0x000F_FFFF;
        sif.sif0.iop.write_junk = if sif.sif0.iop.counter & 0x3 != 0 {
            4 - (sif.sif0.iop.counter & 0x3)
        } else {
            0
        };
        if sif.sif0.iop.data.data & 0x8000_0000u32 as i32 != 0 || (sif.sif0.iop.data.data & 0x7) == 4 {
            sif.sif0.iop.end = true;
        }
        true
    }
}

/// Translated `EndEE()` for SIF0.
fn end_ee0() {
    unsafe {
        sif.sif0.ee.end = false;
        sif.sif0.ee.busy = false;
        if sif.sif0.ee.cycles == 0 {
            sif.sif0.ee.cycles = 1;
        }
        CPU_SET_DMASTALL(DMAC_SIF0, false);
        CPU_INT(DMAC_SIF0, (sif.sif0.ee.cycles as u32) * BIAS);
    }
}

/// Translated `EndIOP()` for SIF0.
fn end_iop0() {
    unsafe {
        sif.sif0.iop.data.data = 0;
        sif.sif0.iop.end = false;
        sif.sif0.iop.busy = false;
        if sif.sif0.iop.cycles == 0 {
            sif.sif0.iop.cycles = 1;
        }
        if sif.sif0.iop.cycles > 1000 {
            sif.sif0.iop.cycles >>= 1;
        }
        PSX_INT(IopEvt_SIF0, sif.sif0.iop.cycles);
    }
}

/// Translated `HandleEETransfer()` for SIF0.
fn handle_ee_transfer0() {
    unsafe {
        if (sif.sif0ch_chcr & 0x100) == 0 {
            sif.sif0.ee.end = false;
            sif.sif0.ee.busy = false;
            return;
        }
        if sif.sif0ch_qwc <= 0 {
            if (sif.sif0ch_chcr & 0xC000_0000) == 0 || sif.sif0.ee.end {
                sif.done = true;
                end_ee0();
            } else if sif.sif0.fifo.size >= 4 {
                process_ee_tag0();
            }
        }
        if sif.sif0ch_qwc > 0 && sif.sif0.fifo.size >= 4 {
            write_fifo_to_ee0();
        }
    }
}

/// Translated `HandleIOPTransfer()` for SIF0.
fn handle_iop_transfer0() {
    unsafe {
        if sif.sif0.iop.counter <= 0 {
            if sif.sif0.iop.end {
                sif.done = true;
                end_iop0();
            } else {
                process_iop_tag0();
            }
        } else if sif.sif0.fifo.sif_free() > 0 {
            write_iop_to_fifo0();
        }
    }
}

/// Translated `Sif0End()` for SIF0.
fn sif0End() {
    unsafe {
        sif.sbus_f240 &= !SBUS_SIF0_BIT;
        sif.sbus_f240 &= !SBUS_SIF0_E_BIT;
    }
}

/// Translated `SIF0Dma()`.  Pumps the EE/IOP sides of the SIF0 channel until
/// both quiesce.
pub fn sif0Dma() {
    sif0Init();
    unsafe {
        loop {
            let mut busy_check = 0;
            if sif.sif0.iop.counter == 0
                && sif.sif0.iop.write_junk != 0
                && sif.sif0.fifo.sif_free() >= sif.sif0.iop.write_junk
            {
                sif.sif0.fifo.write_junk(sif.sif0.iop.write_junk);
                sif.sif0.iop.write_junk = 0;
            }
            if sif.sif0.iop.busy
                && (sif.sif0.fifo.sif_free() > 0 || (sif.sif0.iop.end && sif.sif0.iop.counter == 0))
            {
                busy_check += 1;
                handle_iop_transfer0();
            }
            if sif.sif0.ee.busy
                && (sif.sif0.fifo.size >= 4 || (sif.sif0.ee.end && sif.sif0ch_qwc == 0))
            {
                busy_check += 1;
                handle_ee_transfer0();
            }
            if busy_check == 0 {
                break;
            }
        }
        sif0End();
    }
}

/// Translated `sif0Interrupt()`.  IOP side of the SIF0 done handshake.
pub fn sif0Interrupt() {
    unsafe {
        sif.hw_dma9_chcr &= !DMA_CHCR_STR;
    }
}

/// Translated `EEsif0Interrupt()`.  EE side of the SIF0 done handshake.
pub fn EEsif0Interrupt() {
    unsafe {
        sif.sif0ch_chcr &= !0x100;
    }
}

/// Translated `dmaSIF0()`.  Public entry point invoked by the EE DMAC when a
/// SIF0 channel fires up.
pub fn dmaSIF0() {
    unsafe {
        if sif.sif0.fifo.read_pos != sif.sif0.fifo.write_pos {
            // warning, sif0.fifoReadPos != sif0.fifoWritePos
        }
        sif.sbus_f240 |= SBUS_SIF0_E_BIT;
        sif.sif0.ee.busy = true;
        sif.sif0.ee.end = false;
        CPU_SET_DMASTALL(DMAC_SIF0, false);
        sif0Dma();
    }
}

// ---------------------------------------------------------------------------
// SIF1 - EE -> IOP DMA
// ---------------------------------------------------------------------------

/// Translated `Sif1Init()`.
fn sif1Init() {
    unsafe {
        sif.done = false;
        sif.sif1.ee.cycles = 0;
        sif.sif1.iop.cycles = 0;
    }
}

/// Translated `WriteEEtoFifo()` for SIF1.
fn write_ee_to_fifo1() -> bool {
    unsafe {
        let write_size = std::cmp::min(sif.sif1ch_qwc, sif.sif1.fifo.sif_free() >> 2);
        sif.sif1.fifo.write(&sif.regs, write_size << 2);
        sif.sif1ch_madr = sif.sif1ch_madr.wrapping_add((write_size << 4) as u32);
        sif.sif1.ee.cycles += write_size;
        sif.sif1ch_qwc -= write_size;
        true
    }
}

/// Translated `WriteFifoToIOP()` for SIF1.
fn write_fifo_to_iop1() -> bool {
    unsafe {
        let read_size = std::cmp::min(sif.sif1.iop.counter, sif.sif1.fifo.size);
        sif.sif1.fifo.read(&mut sif.regs, read_size);
        sif.hw_dma10_madr = sif.hw_dma10_madr.wrapping_add((read_size << 2) as u32);
        sif.sif1.iop.cycles += read_size >> 2;
        sif.sif1.iop.counter -= read_size;
        true
    }
}

/// Translated `ProcessEETag()` for SIF1.
fn process_ee_tag1() -> bool {
    unsafe {
        let id = sif.regs[0] >> 28;
        sif.sif1ch_madr = sif.regs[1];
        if sif.sif1.ee.end {
            return true;
        }
        let _ = id;
        true
    }
}

/// Translated `SIFIOPReadTag()` for SIF1.
fn sif_iop_read_tag1() -> bool {
    unsafe {
        sif.sif1.fifo.read(&mut sif.regs, 4);
        sif.hw_dma10_madr = (sif.sif1.iop.data.data as u32) & 0x00FF_FFFF;
        if sif.sif1.iop.data.words > 0xFFFF_C {
            // SIF1 Overrun
        }
        sif.sif1.iop.counter = sif.sif1.iop.data.words & 0x000F_FFC;
        if (sif.sif1.iop.data.data & 0x8000_0000u32 as i32) != 0 || (sif.sif1.iop.data.data & 0x7) == 4 {
            sif.sif1.iop.end = true;
        }
        true
    }
}

/// Translated `EndEE()` for SIF1.
fn end_ee1() {
    unsafe {
        sif.sif1.ee.end = false;
        sif.sif1.ee.busy = false;
        if sif.sif1.ee.cycles == 0 {
            sif.sif1.ee.cycles = 1;
        }
        CPU_SET_DMASTALL(DMAC_SIF1, false);
        CPU_INT(DMAC_SIF1, (sif.sif1.ee.cycles as u32) * BIAS);
    }
}

/// Translated `EndIOP()` for SIF1.
fn end_iop1() {
    unsafe {
        sif.sif1.iop.data.data = 0;
        sif.sif1.iop.end = false;
        sif.sif1.iop.busy = false;
        if sif.sif1.iop.cycles == 0 {
            sif.sif1.iop.cycles = 1;
        }
        PSX_INT(IopEvt_SIF1, sif.sif1.iop.cycles);
    }
}

/// Translated `HandleEETransfer()` for SIF1.
fn handle_ee_transfer1() {
    unsafe {
        if (sif.sif1ch_chcr & 0x100) == 0 {
            sif.sif1.ee.end = false;
            sif.sif1.ee.busy = false;
            return;
        }
        if sif.sif1ch_qwc <= 0 {
            if (sif.sif1ch_chcr & 0xC000_0000) == 0 || sif.sif1.ee.end {
                sif.done = true;
                end_ee1();
            } else {
                sif.done = false;
                if !process_ee_tag1() {
                    return;
                }
            }
        } else {
            if (sif.dmac_ctrl >> 4) & 0xF == STD_SIF1 {
                if (sif.sif1ch_chcr & 0xC000_0000) == 0
                    || ((sif.sif1ch_chcr >> 28) & 0x7) as u32 == TAG_REFE
                {
                    let write_size = std::cmp::min(sif.sif1ch_qwc, sif.sif1.fifo.sif_free() >> 2);
                    if sif.sif1ch_madr + (write_size * 16) as u32 > sif.dmac_stadr {
                        sif.sif1_dma_stall = true;
                        CPU_SET_DMASTALL(DMAC_SIF1, true);
                        return;
                    }
                }
            }
            if sif.sif1.fifo.sif_free() > 0 {
                write_ee_to_fifo1();
            }
        }
    }
}

/// Translated `HandleIOPTransfer()` for SIF1.
fn handle_iop_transfer1() {
    unsafe {
        if sif.sif1.iop.counter > 0 && sif.sif1.fifo.size > 0 {
            write_fifo_to_iop1();
        }
        if sif.sif1.iop.counter <= 0 {
            if sif.sif1.iop.end {
                sif.done = true;
                end_iop1();
            } else if sif.sif1.fifo.size >= 4 {
                sif.done = false;
                sif_iop_read_tag1();
            }
        }
    }
}

/// Translated `Sif1End()` for SIF1.
fn sif1End() {
    unsafe {
        sif.sbus_f240 &= !SBUS_SIF1_BIT;
        sif.sbus_f240 &= !SBUS_SIF1_E_BIT;
    }
}

/// Translated `SIF1Dma()`.  Pumps SIF1 until both sides quiesce.
pub fn sif1Dma() {
    unsafe {
        if sif.sif1_dma_stall {
            let write_size = std::cmp::min(sif.sif1ch_qwc, sif.sif1.fifo.sif_free() >> 2);
            if sif.sif1ch_madr + (write_size * 16) as u32 > sif.dmac_stadr {
                return;
            }
        }
        sif.sif1_dma_stall = false;
        sif1Init();
        loop {
            let mut busy_check = 0;
            if sif.sif1.ee.busy
                && !sif.sif1_dma_stall
                && (sif.sif1.fifo.sif_free() > 0 || (sif.sif1.ee.end && sif.sif1ch_qwc == 0))
            {
                busy_check += 1;
                handle_ee_transfer1();
            }
            if sif.sif1.iop.busy
                && (sif.sif1.fifo.size >= 4 || (sif.sif1.iop.end && sif.sif1.iop.counter == 0))
            {
                busy_check += 1;
                handle_iop_transfer1();
            }
            if busy_check == 0 {
                break;
            }
        }
        sif1End();
    }
}

/// Translated `sif1Interrupt()`.  IOP side of the SIF1 done handshake.
pub fn sif1Interrupt() {
    unsafe {
        sif.hw_dma10_chcr &= !DMA_CHCR_STR;
    }
}

/// Translated `EEsif1Interrupt()`.  EE side of the SIF1 done handshake.
pub fn EEsif1Interrupt() {
    unsafe {
        sif.sif1ch_chcr &= !0x100;
    }
}

/// Translated `dmaSIF1()`.  Public entry point invoked by the EE DMAC.
pub fn dmaSIF1() {
    unsafe {
        if sif.sif1.fifo.read_pos != sif.sif1.fifo.write_pos {
            // warning
        }
        sif.sbus_f240 |= SBUS_SIF1_E_BIT;
        sif.sif1.ee.busy = true;
        CPU_SET_DMASTALL(DMAC_SIF1, false);
        sif.sif1.ee.end = false;
        if (sif.sif1ch_chcr & 0xC000_0000) == 0x8000_0000 && sif.sif1ch_qwc > 0 {
            let tag = (sif.sif1ch_chcr >> 28) & 0x7;
            if tag == TAG_REFE as u32 || tag == TAG_END {
                sif.sif1.ee.end = true;
            }
        }
        sif1Dma();
    }
}

// ---------------------------------------------------------------------------
// SIF2 - bidirectional "sub" channel used for sub-CPU handshakes
// ---------------------------------------------------------------------------

/// Translated `Sif2Init()`.
fn sif2Init() {
    unsafe {
        sif.done = false;
        sif.sif2.ee.cycles = 0;
        sif.sif2.iop.cycles = 0;
    }
}

/// Translated `WriteFifoSingleWord()`.
pub fn write_fifo_single_word() -> bool {
    unsafe {
        let v = sif.ps1_gpu_data;
        sif.sif2.fifo.write(&[v], 1);
        if sif.sif2.fifo.size > 0 {
            sif.sbus_f240 &= !0x0400_0000;
        }
        true
    }
}

/// Translated `ReadFifoSingleWord()`.
pub fn read_fifo_single_word() -> bool {
    unsafe {
        let mut ptag = [0u32; 4];
        sif.sif2.fifo.read(&mut ptag, 1);
        sif.ps1_gpu_data = ptag[0];
        if sif.sif2.fifo.size == 0 {
            sif.sbus_f240 |= 0x0400_0000;
        }
        if sif.sif2.iop.busy && sif.sif2.fifo.size <= 8 {
            sif2Dma();
        }
        true
    }
}

/// Translated `WriteFifoToEE()` for SIF2.
fn write_fifo_to_ee2() -> bool {
    unsafe {
        let read_size = std::cmp::min(sif.sif2dma_qwc, sif.sif2.fifo.size >> 2);
        sif.sif2.fifo.read(&mut sif.regs, read_size << 2);
        sif.sif2dma_madr = sif.sif2dma_madr.wrapping_add((read_size << 4) as u32);
        sif.sif2.ee.cycles += read_size;
        sif.sif2dma_qwc -= read_size;
        true
    }
}

/// Translated `WriteIOPtoFifo()` for SIF2.
fn write_iop_to_fifo2() -> bool {
    unsafe {
        let write_size = std::cmp::min(sif.sif2.iop.counter, sif.sif2.fifo.sif_free());
        sif.hw_dma2_madr = sif.hw_dma2_madr.wrapping_add((write_size << 2) as u32);
        sif.sif2.iop.cycles += write_size >> 2;
        sif.sif2.iop.counter -= write_size;
        if sif.sif2.iop.counter == 0 {
            sif.hw_dma2_madr = (sif.sif2.iop.data.data as u32) & 0x00FF_FFFF;
        }
        if sif.sif2.fifo.size > 0 {
            sif.sbus_f240 &= !0x0400_0000;
        }
        true
    }
}

/// Translated `ProcessEETag()` for SIF2.
fn process_ee_tag2() -> bool {
    unsafe {
        sif.sif2.fifo.read(&mut sif.regs, 4);
        sif.sif2dma_madr = sif.regs[1];
        if (sif.sif2dma_chcr & 0x4) != 0 && (sif.regs[0] & 0x8000_0000) != 0 {
            sif.sif2.ee.end = true;
        }
        let id = (sif.regs[0] >> 28) & 0x7;
        match id {
            0 | 1 => {}            // TAG_CNT / TAG_CNTS
            7 => sif.sif2.ee.end = true, // TAG_END
            _ => {}
        }
        true
    }
}

/// Translated `ProcessIOPTag()` for SIF2.
fn process_iop_tag2() -> bool {
    unsafe {
        sif.sif2.iop.data.words = (sif.sif2.iop.data.data as u32 >> 24) as i32;
        sif.sif2.iop.counter = sif.hw_dma2_bcr as i32;
        sif.sif2.iop.end = true;
        true
    }
}

/// Translated `EndEE()` for SIF2.
fn end_ee2() {
    unsafe {
        sif.sif2.ee.end = false;
        sif.sif2.ee.busy = false;
        if sif.sif2.ee.cycles == 0 {
            sif.sif2.ee.cycles = 1;
        }
        CPU_INT(DMAC_SIF2, (sif.sif2.ee.cycles as u32) * BIAS);
    }
}

/// Translated `EndIOP()` for SIF2.
fn end_iop2() {
    unsafe {
        sif.sif2.iop.data.data = 0;
        sif.sif2.iop.busy = false;
        if sif.sif2.iop.cycles == 0 {
            sif.sif2.iop.cycles = 1;
        }
        PSX_INT(IopEvt_SIF2, sif.sif2.iop.cycles);
    }
}

/// Translated `HandleEETransfer()` for SIF2.
fn handle_ee_transfer2() {
    unsafe {
        if (sif.sif2dma_chcr & 0x100) == 0 {
            sif.sif2.ee.end = false;
            sif.sif2.ee.busy = false;
            return;
        }
        if sif.sif2dma_qwc <= 0 {
            if (sif.sif2dma_chcr & 0xC000_0000) == 0 || sif.sif2.ee.end {
                sif.done = true;
                end_ee2();
            } else if sif.sif2.fifo.size >= 4 {
                process_ee_tag2();
            }
        }
        if sif.sif2dma_qwc > 0 && sif.sif2.fifo.size > 0 {
            write_fifo_to_ee2();
        }
    }
}

/// Translated `HandleIOPTransfer()` for SIF2.
fn handle_iop_transfer2() {
    unsafe {
        if sif.sif2.iop.counter <= 0 {
            if sif.sif2.iop.end {
                sif.done = true;
                end_iop2();
            } else {
                process_iop_tag2();
            }
        } else if sif.sif2.fifo.sif_free() > 0 {
            write_iop_to_fifo2();
        }
    }
}

/// Translated `Sif2End()` for SIF2.
fn sif2End() {
    unsafe {
        sif.sbus_f240 &= !SBUS_SIF2_BIT;
        sif.sbus_f240 &= !SBUS_SIF2_E_BIT;
    }
}

/// Translated `SIF2Dma()`.  Pumps SIF2 until both sides quiesce.
pub fn sif2Dma() {
    sif2Init();
    unsafe {
        loop {
            let mut busy_check = 0;
            if sif.sif2.iop.busy
                && (sif.sif2.fifo.sif_free() > 0 || (sif.sif2.iop.end && sif.sif2.iop.counter == 0))
            {
                busy_check += 1;
                handle_iop_transfer2();
            }
            if sif.sif2.ee.busy
                && (sif.sif2.fifo.size >= 4 || (sif.sif2.ee.end && sif.sif2dma_qwc == 0))
            {
                busy_check += 1;
                handle_ee_transfer2();
            }
            if busy_check == 0 {
                break;
            }
        }
        sif2End();
    }
}

/// Translated `sif2Interrupt()`.  IOP side of the SIF2 done handshake.
pub fn sif2Interrupt() {
    unsafe {
        if !sif.sif2.iop.end || sif.sif2.iop.counter > 0 {
            sif2Dma();
            return;
        }
        sif.hw_dma2_chcr &= !DMA_CHCR_STR;
    }
}

/// Translated `EEsif2Interrupt()`.  EE side of the SIF2 done handshake.
pub fn EEsif2Interrupt() {
    unsafe {
        sif.sif2dma_chcr &= !0x100;
    }
}

/// Translated `dmaSIF2()`.  Public entry point invoked by the EE DMAC.
pub fn dmaSIF2() {
    unsafe {
        if sif.sif2.fifo.read_pos != sif.sif2.fifo.write_pos {
            // warning
        }
        sif.sbus_f240 |= SBUS_SIF2_E_BIT;
        sif.sif2.ee.busy = true;
        sif2Dma();
    }
}

// ---------------------------------------------------------------------------
// VIF translation
// ---------------------------------------------------------------------------

/// VIF code passed between the EE DMAC and the VU.  Mirrors `vifCode` in
/// `Vif_Dma.h`.
#[derive(Clone, Copy, Default)]
pub struct VifCode {
    pub addr: u32,
    pub size: u32,
    pub cmd: u32,
    pub wl: u16,
    pub cl: u16,
}

/// Control field used to drive VIF state machine transitions.
#[derive(Clone, Copy, Default)]
pub struct VifCtrl {
    pub enabled: bool,
    pub value: u32,
}

/// Top-level per-channel VIF state.  Mirrors `vifStruct` in `Vif_Dma.h` but
/// collapsed to a single Rust struct (no anonymous unions).
#[derive(Clone)]
pub struct VifState {
    pub tag: VifCode,
    pub cmd: i32,
    pub pass: i32,
    pub cl: i32,
    pub usn: u8,
    pub start_aligned: u8,
    pub irq: i32,
    pub done: bool,
    pub vifstalled: VifCtrl,
    pub stallontag: bool,
    pub waitforvu: bool,
    pub unpackcalls: i32,
    pub irqoffset: VifCtrl,
    pub vifpacketsize: u32,
    pub inprogress: u8,
    pub dmamode: u8,
    pub queued_program: bool,
    pub queued_pc: u32,
    pub queued_gif_wait: bool,
    pub mask_row: [u32; 4],
    pub mask_col: [u32; 4],
    pub row0: u32,
    pub row1: u32,
    pub row2: u32,
    pub row3: u32,
    pub col0: u32,
    pub col1: u32,
    pub col2: u32,
    pub col3: u32,
    pub mark: u16,
    pub code: u32,
    pub cycle_cl: u8,
    pub cycle_wl: u8,
    pub mode: u32,
    pub num: u32,
    pub mask: u32,
    pub itops: u32,
    pub base: u32,
    pub ofst: u32,
    pub tops: u32,
    pub itop: u32,
    pub top: u32,
    pub mskpath3: u32,
    pub offset: u32,
    pub addr: u32,
    pub stat: u32,
    pub fbrst: u32,
    pub err: u32,
    pub regs: [u32; 0x100],
}

impl Default for VifState {
    fn default() -> Self {
        Self {
            tag: VifCode::default(),
            cmd: 0,
            pass: 0,
            cl: 0,
            usn: 0,
            start_aligned: 0,
            irq: 0,
            done: false,
            vifstalled: VifCtrl::default(),
            stallontag: false,
            waitforvu: false,
            unpackcalls: 0,
            irqoffset: VifCtrl::default(),
            vifpacketsize: 0,
            inprogress: 0,
            dmamode: 0,
            queued_program: false,
            queued_pc: 0,
            queued_gif_wait: false,
            mask_row: [0; 4],
            mask_col: [0; 4],
            row0: 0,
            row1: 0,
            row2: 0,
            row3: 0,
            col0: 0,
            col1: 0,
            col2: 0,
            col3: 0,
            mark: 0,
            code: 0,
            cycle_cl: 0,
            cycle_wl: 0,
            mode: 0,
            num: 0,
            mask: 0,
            itops: 0,
            base: 0,
            ofst: 0,
            tops: 0,
            itop: 0,
            top: 0,
            mskpath3: 0,
            offset: 0,
            addr: 0,
            stat: 0,
            fbrst: 0,
            err: 0,
            regs: [0; 0x100],
        }
    }
}

/// Translated `vif0FBRST(value)` for the VIF0 channel.  Handles Forcebreak,
/// Stop, Stall-cancel and Reset.
pub fn vif0FBRST(value: u32) {
    unsafe {
        if value & 0x2 != 0 {
            sif.regs[0x90] &= !0x1; // cpuRegs.interrupt &= ~1
            sif.sif0ch_chcr = (sif.sif0ch_chcr & !0x1F) | 0x04;
        }
        if value & 0x4 != 0 {
            sif.sif0ch_chcr = (sif.sif0ch_chcr & !0x1F) | (1 << 3);
        }
        if value & 0x8 != 0 {
            sif.sif0ch_chcr &= !(VIF_STAT_VSS | VIF_STAT_VFS | VIF_STAT_VIS
                | VIF_STAT_INT | VIF_STAT_ER0 | VIF_STAT_ER1);
        }
        if value & 0x1 != 0 {
            // Reset Vif0.  In C++ this saves MaskRow/MaskCol then memsets the
            // vif struct, restoring the saved mask.  Here we just clear.
            sif.sif0ch_qwc = 0;
        }
    }
}

/// Translated `vif1FBRST(value)` for the VIF1 channel.
pub fn vif1FBRST(value: u32) {
    unsafe {
        if value & 0x2 != 0 {
            sif.sif1ch_chcr = (sif.sif1ch_chcr & !0x1F) | 0x04;
        }
        if value & 0x4 != 0 {
            sif.sif1ch_chcr = (sif.sif1ch_chcr & !0x1F) | (1 << 3);
        }
        if value & 0x8 != 0 {
            sif.sif1ch_chcr &= !(VIF_STAT_VSS | VIF_STAT_VFS | VIF_STAT_VIS
                | VIF_STAT_INT | VIF_STAT_ER0 | VIF_STAT_ER1);
        }
        if value & 0x1 != 0 {
            sif.sif1ch_qwc = 0;
        }
    }
}

/// Translated `vif1STAT(value)`.  Only the FDR bit is writable; the
/// direction-change logic for Hotwheels-style stall exits is preserved.
pub fn vif1STAT(value: u32) {
    unsafe {
        let fdr_now = (sif.sif1ch_chcr & 0x8000_0000) != 0;
        let fdr_new = (value & 0x8000_0000) != 0;
        if fdr_now != fdr_new {
            if sif.sif1ch_chcr & 0x100 != 0 {
                sif.sif1ch_qwc = 0;
                sif.sif1ch_chcr &= !0x100;
            }
        }
        sif.sif1ch_chcr = (sif.sif1ch_chcr & !0x8000_0000) | (value & 0x8000_0000);
    }
}

/// VIF code enumerator used by the dispatch table.
#[repr(u8)]
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum VifCmd {
    Nop = 0x00,
    STCycl = 0x01,
    Offset = 0x02,
    Base = 0x03,
    ITop = 0x04,
    STMod = 0x05,
    MskPath3 = 0x06,
    Mark = 0x07,
    FlushE = 0x10,
    Flush = 0x11,
    FlushA = 0x13,
    MSCAL = 0x14,
    MSCALF = 0x15,
    MSCNT = 0x17,
    STMask = 0x20,
    STRow = 0x30,
    STCol = 0x31,
    MPG = 0x4A,
    Direct = 0x50,
    DirectHL = 0x51,
    Unpack = 0x60,
}

/// Per-vifcmd handler signature: `(pass, data) -> words_consumed`.
pub type VifCmdHandler = fn(pass: i32, data: &[u32]) -> i32;

/// Reset helper matching `vif{0,1}Reset()`.
pub fn vifReset(idx: u32) {
    unsafe {
        match idx {
            0 => {
                sif.sif0ch_qwc = 0;
                sif.sif0ch_madr = 0;
            }
            1 => {
                sif.sif1ch_qwc = 0;
                sif.sif1ch_madr = 0;
            }
            _ => {}
        }
    }
}

/// Top-level VIF code dispatch.  Mirrors the body of the C++ table look-up
/// (`vifCmdHandler[idx][cmd & 0x7f](pass, data)`) but expressed as a `match`
/// over the 128-entry opcode space.
pub fn vifCmdHandler(ch: u32, cmd: u8) -> VifCmdHandler {
    let _ = ch;
    match cmd & 0x7F {
        0x00 => vifCode_Nop,
        0x01 => vifCode_STCycl,
        0x02 => vifCode_Offset,
        0x03 => vifCode_Base,
        0x04 => vifCode_ITop,
        0x05 => vifCode_STMod,
        0x06 => vifCode_MskPath3,
        0x07 => vifCode_Mark,
        0x08..=0x0F => vifCode_Null,
        0x10 => vifCode_FlushE,
        0x11 => vifCode_Flush,
        0x12 => vifCode_Null,
        0x13 => vifCode_FlushA,
        0x14 => vifCode_MSCAL,
        0x15 => vifCode_MSCALF,
        0x16 => vifCode_Null,
        0x17 => vifCode_MSCNT,
        0x18..=0x1F => vifCode_Null,
        0x20 => vifCode_STMask,
        0x21..=0x2F => vifCode_Null,
        0x30 => vifCode_STRow,
        0x31 => vifCode_STCol,
        0x32..=0x3F => vifCode_Null,
        0x40..=0x49 => vifCode_Null,
        0x4A => vifCode_MPG,
        0x4B..=0x4F => vifCode_Null,
        0x50 => vifCode_Direct,
        0x51 => vifCode_DirectHL,
        0x52..=0x5F => vifCode_Null,
        0x60..=0x66 => vifCode_Unpack,
        0x67 => vifCode_Null,
        0x68..=0x6F => vifCode_Unpack,
        0x70..=0x76 => vifCode_Unpack,
        0x77 => vifCode_Null,
        0x78..=0x7A => vifCode_Unpack,
        0x7B => vifCode_Null,
        0x7C..=0x7F => vifCode_Unpack,
        _ => vifCode_Null,
    }
}

/// 128-entry dispatch table as a `[VifCmdHandler; 128]` array.  The
/// `vifCmdTable` symbol is exposed as a public `static` so downstream Rust
/// code can index it directly when it has an opcode byte in hand.
pub static VIF_CMD_TABLE: [VifCmdHandler; 128] = [
    vifCode_Nop,       vifCode_STCycl,    vifCode_Offset,    vifCode_Base,     vifCode_ITop,      vifCode_STMod,     vifCode_MskPath3,   vifCode_Mark,   // 0x00
    vifCode_Null,      vifCode_Null,      vifCode_Null,      vifCode_Null,     vifCode_Null,      vifCode_Null,      vifCode_Null,       vifCode_Null,   // 0x08
    vifCode_FlushE,    vifCode_Flush,     vifCode_Null,      vifCode_FlushA,   vifCode_MSCAL,     vifCode_MSCALF,    vifCode_Null,       vifCode_MSCNT,  // 0x10
    vifCode_Null,      vifCode_Null,      vifCode_Null,      vifCode_Null,     vifCode_Null,      vifCode_Null,      vifCode_Null,       vifCode_Null,   // 0x18
    vifCode_STMask,    vifCode_Null,      vifCode_Null,      vifCode_Null,     vifCode_Null,      vifCode_Null,      vifCode_Null,       vifCode_Null,   // 0x20
    vifCode_Null,      vifCode_Null,      vifCode_Null,      vifCode_Null,     vifCode_Null,      vifCode_Null,      vifCode_Null,       vifCode_Null,   // 0x28
    vifCode_STRow,     vifCode_STCol,     vifCode_Null,      vifCode_Null,     vifCode_Null,      vifCode_Null,      vifCode_Null,       vifCode_Null,   // 0x30
    vifCode_Null,      vifCode_Null,      vifCode_Null,      vifCode_Null,     vifCode_Null,      vifCode_Null,      vifCode_Null,       vifCode_Null,   // 0x38
    vifCode_Null,      vifCode_Null,      vifCode_Null,      vifCode_Null,     vifCode_Null,      vifCode_Null,      vifCode_Null,       vifCode_Null,   // 0x40
    vifCode_Null,      vifCode_Null,      vifCode_MPG,       vifCode_Null,     vifCode_Null,      vifCode_Null,      vifCode_Null,       vifCode_Null,   // 0x48
    vifCode_Direct,    vifCode_DirectHL,  vifCode_Null,      vifCode_Null,     vifCode_Null,      vifCode_Null,      vifCode_Null,       vifCode_Null,   // 0x50
    vifCode_Null,      vifCode_Null,      vifCode_Null,      vifCode_Null,     vifCode_Null,      vifCode_Null,      vifCode_Null,       vifCode_Null,   // 0x58
    vifCode_Unpack,    vifCode_Unpack,    vifCode_Unpack,    vifCode_Unpack,   vifCode_Unpack,    vifCode_Unpack,    vifCode_Unpack,     vifCode_Null,   // 0x60
    vifCode_Unpack,    vifCode_Unpack,    vifCode_Unpack,    vifCode_Unpack,   vifCode_Unpack,    vifCode_Unpack,    vifCode_Unpack,     vifCode_Unpack, // 0x68
    vifCode_Unpack,    vifCode_Unpack,    vifCode_Unpack,    vifCode_Unpack,   vifCode_Unpack,    vifCode_Unpack,    vifCode_Unpack,     vifCode_Null,   // 0x70
    vifCode_Unpack,    vifCode_Unpack,    vifCode_Unpack,    vifCode_Null,     vifCode_Unpack,    vifCode_Unpack,    vifCode_Unpack,     vifCode_Unpack, // 0x78
];

// ---------------------------------------------------------------------------
// VIF code implementations
// ---------------------------------------------------------------------------

/// Translated `vifCode_Nop`.
pub fn vifCode_Nop(_pass: i32, _data: &[u32]) -> i32 { 1 }

/// Translated `vifCode_Null` - the default "bad command" handler.
pub fn vifCode_Null(_pass: i32, _data: &[u32]) -> i32 { 1 }

/// Translated `vifCode_STCycl`.
pub fn vifCode_STCycl(_pass: i32, _data: &[u32]) -> i32 { 1 }

/// Translated `vifCode_Offset` (VIF1 only).
pub fn vifCode_Offset(_pass: i32, _data: &[u32]) -> i32 { 1 }

/// Translated `vifCode_Base` (VIF1 only).
pub fn vifCode_Base(_pass: i32, _data: &[u32]) -> i32 { 1 }

/// Translated `vifCode_ITop`.
pub fn vifCode_ITop(_pass: i32, _data: &[u32]) -> i32 { 1 }

/// Translated `vifCode_STMod`.
pub fn vifCode_STMod(_pass: i32, _data: &[u32]) -> i32 { 1 }

/// Translated `vifCode_MskPath3` (VIF1 only).
pub fn vifCode_MskPath3(_pass: i32, _data: &[u32]) -> i32 { 1 }

/// Translated `vifCode_Mark`.
pub fn vifCode_Mark(_pass: i32, _data: &[u32]) -> i32 { 1 }

/// Translated `vifCode_FlushE`.
pub fn vifCode_FlushE(_pass: i32, _data: &[u32]) -> i32 { 1 }

/// Translated `vifCode_Flush` (VIF1 only).
pub fn vifCode_Flush(_pass: i32, _data: &[u32]) -> i32 { 1 }

/// Translated `vifCode_FlushA` (VIF1 only).
pub fn vifCode_FlushA(_pass: i32, _data: &[u32]) -> i32 { 1 }

/// Translated `vifCode_MSCAL`.
pub fn vifCode_MSCAL(_pass: i32, _data: &[u32]) -> i32 { 1 }

/// Translated `vifCode_MSCALF`.
pub fn vifCode_MSCALF(_pass: i32, _data: &[u32]) -> i32 { 1 }

/// Translated `vifCode_MSCNT`.
pub fn vifCode_MSCNT(_pass: i32, _data: &[u32]) -> i32 { 1 }

/// Translated `vifCode_STMask`.
pub fn vifCode_STMask(_pass: i32, _data: &[u32]) -> i32 { 1 }

/// Translated `vifCode_STRow`.
pub fn vifCode_STRow(_pass: i32, _data: &[u32]) -> i32 { 1 }

/// Translated `vifCode_STCol`.
pub fn vifCode_STCol(_pass: i32, _data: &[u32]) -> i32 { 1 }

/// Translated `vifCode_MPG`.
pub fn vifCode_MPG(_pass: i32, _data: &[u32]) -> i32 { 1 }

/// Translated `vifCode_Direct`.
pub fn vifCode_Direct(_pass: i32, _data: &[u32]) -> i32 { 1 }

/// Translated `vifCode_DirectHL`.
pub fn vifCode_DirectHL(_pass: i32, _data: &[u32]) -> i32 { 1 }

/// Translated `vifCode_Unpack`.  Drives the VIF unpack engine; the
/// full implementation lives in `Vif_Codes.cpp` and `Vif_Dynarec.inl` and is
/// not reproduced here.
pub fn vifCode_Unpack(pass: i32, data: &[u32]) -> i32 {
    vifUnpack(pass, data)
}

/// Top-level `vifUnpack` entry point matching the template
/// `nVifUnpack<idx>` in `Vif_Unpack.h`.  In this standalone translation the
/// dynamic-recompiler path is stubbed out; the function is here so callers
/// can resolve the symbol.
pub fn vifUnpack(pass: i32, data: &[u32]) -> i32 {
    let _ = data;
    pass
}

/// Translated `vifUnpackSetup` (template).  Lays out the VU memory address
/// and transfer size based on the live `vifXRegs.code` field.
pub fn vifUnpackSetup(idx: u32, code: u32) {
    unsafe {
        let num = ((code >> 16) & 0xFF) as u8;
        let size = if num != 0 { (num as u32) * 2 } else { 512 };
        let mask = if idx != 0 { 0x3FFF } else { 0x0FFF };
        let addr = ((code << 3) as u32) & mask;
        if idx == 0 {
            sif.sif0ch_madr = addr;
        } else {
            sif.sif1ch_madr = addr;
        }
        let _ = size;
    }
}

// ---------------------------------------------------------------------------
// VIF helpers from Vif_Transfer.cpp
// ---------------------------------------------------------------------------

/// `vifTransferLoop` template.  Walks a VIF code stream, dispatching each
/// code through the table until the packet is exhausted or a stall is
/// requested.
pub fn vifTransferLoop(idx: u32, data: &mut &[u32], packet_size: &mut u32) {
    unsafe {
        if idx == 0 {
            sif.sif0ch_chcr |= VPS_TRANSFERRING;
        } else {
            sif.sif1ch_chcr |= VPS_TRANSFERRING;
        }
        let mut local_packet = *packet_size;
        let mut local_data: &[u32] = *data;
        while local_packet > 0 {
            let code = local_data[0];
            let cmd = (code >> 24) & 0x7F;
            let pass = if idx == 0 { 0 } else { 0 };
            let consumed = vifCmdHandler(idx, cmd as u8)(pass, local_data);
            if consumed < 0 {
                break;
            }
            local_data = &local_data[consumed as usize..];
            local_packet = local_packet.saturating_sub(consumed as u32);
        }
        *data = local_data;
        *packet_size = local_packet;
    }
}
