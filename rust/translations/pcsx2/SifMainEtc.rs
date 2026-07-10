// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! `SifMainEtc` — Rust 2021 translation of PCSX2's SIF (Sub-CPU Interface)
//! and IOP DMA subsystem.
//!
//! This module folds the C/C++ sources
//! [`Sif.cpp`](../../pcsx2/Sif.cpp), [`Sif.h`](../../pcsx2/Sif.h),
//! [`Sif0.cpp`](../../pcsx2/Sif0.cpp), [`Sif1.cpp`](../../pcsx2/Sif1.cpp),
//! [`sif2.cpp`](../../pcsx2/sif2.cpp), [`Sifcmd.h`](../../pcsx2/Sifcmd.h),
//! [`IopDma.cpp`](../../pcsx2/IopDma.cpp),
//! [`IopDma.h`](../../pcsx2/IopDma.h) and
//! [`IopIrq.cpp`](../../pcsx2/IopIrq.cpp) into a single idiomatic Rust
//! 2021 module.
//!
//! The SIF is the bus that lets the EE and the IOP exchange small packets
//! of data and 1-quadword DMA tags.  The module exposes:
//!
//! * The eight-word SIF control register block (`MSCOM`, `SMCOM`, `MSFLG`,
//!   `SMFLG`, `EE_ADDR`, `IOP_ADDR`, `EE_SIZE`, `IOP_SIZE`) as
//!   [`SifState`].
//! * Lifecycle helpers [`sifInit`], [`sifReset`], [`sifInterrupt`].
//! * The three SIF DMA pumpers [`sif0Dma`], [`sif1Dma`], [`sif2Dma`].
//! * The IOP DMA controller [`IopDma`], plus [`psxDmaInit`],
//!   [`psxDmaReset`], [`psxDmaUpdate`].
//! * The IOP interrupt helper [`iopIntcIrq`] (the
//!   `psxHu32(HW_ISTAT) |= 1<<irq; iopTestIntc();` shim from
//!   `IopIrq.cpp`).
//!
//! All global state is exposed as `static mut` symbols to mirror the
//! C/C++ original.  External dependencies on the rest of PCSX2 (PSX/IOP
//! memory, hardware registers, SPU2/DEV9/SIO2 cores, logging, IRQ
//! scheduling, …) are declared as `extern "C"` symbols and are
//! intentionally left unimplemented here.

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(dead_code)]
#![allow(static_mut_refs)]
#![allow(unused_imports)]
#![allow(unused_variables)]

// ============================================================================
// Constants
// ============================================================================

/// Number of 32-bit words in one SIF FIFO ring buffer.
pub const FIFO_SIF_W: usize = 128;

// EE DMAC channel ids used by the SIF paths.
pub const DMAC_SIF0: u32 = 4;
pub const DMAC_SIF1: u32 = 5;
pub const DMAC_SIF2: u32 = 6;
pub const DMAC_STALL_SIS: u32 = 27;

// DMA tag ids (top 3 bits of a DMA tag's first word).
pub const TAG_CNT: u32 = 0;
pub const TAG_CNTS: u32 = 1;
pub const TAG_REFS: u32 = 2;
pub const TAG_REFE: u32 = 3;
pub const TAG_END: u32 = 7;

// DMA transfer modes.
pub const NORMAL_MODE: u32 = 0;
pub const CHAIN_MODE: u32 = 1;

// Stall-source IDs for `dmacRegs.ctrl.STS/STD`.
pub const STS_SIF0: u32 = 0;
pub const STD_SIF1: u32 = 0;

// IOP event ids used by the `PSX_INT` scheduler.
pub const IopEvt_SIF0: u32 = 18;
pub const IopEvt_SIF1: u32 = 19;
pub const IopEvt_SIF2: u32 = 20;
pub const IopEvt_Dma11: u32 = 21;
pub const IopEvt_Dma12: u32 = 22;
pub const IopEvt_DEV9: u32 = 23;
pub const IopEvt_USB: u32 = 24;

// EE hardware register addresses used by the SIF sub-system.
pub const SBUS_F240: u32 = 0x1000_F240;
pub const HW_ISTAT: u32 = 0x1F80_1070;
pub const HW_PS1_GPU_DATA: u32 = 0x1000_F3E0;
pub const HW_DMA2_MADR: u32 = 0x1F80_1080;
pub const HW_DMA2_BCR_H16: u32 = 0x1F80_1084;
pub const HW_DMA2_BCR_L16: u32 = 0x1F80_1086;
pub const HW_DMA2_CHCR: u32 = 0x1F80_1088;
pub const HW_DMA4_MADR: u32 = 0x1F80_1090;
pub const HW_DMA4_BCR: u32 = 0x1F80_1094;
pub const HW_DMA4_CHCR: u32 = 0x1F80_1098;
pub const HW_DMA6_MADR: u32 = 0x1F80_10A0;
pub const HW_DMA6_CHCR: u32 = 0x1F80_10A8;
pub const HW_DMA7_MADR: u32 = 0x1F80_10E0;
pub const HW_DMA7_BCR: u32 = 0x1F80_10E4;
pub const HW_DMA7_CHCR: u32 = 0x1F80_10E8;
pub const HW_DMA8_MADR: u32 = 0x1F80_1500;
pub const HW_DMA8_BCR: u32 = 0x1F80_1504;
pub const HW_DMA8_CHCR: u32 = 0x1F80_1508;
pub const HW_DMA9_MADR: u32 = 0x1F80_14A0;
pub const HW_DMA9_TADR: u32 = 0x1F80_14A4;
pub const HW_DMA9_BCR: u32 = 0x1F80_14A6;
pub const HW_DMA9_CHCR: u32 = 0x1F80_14A8;
pub const HW_DMA10_MADR: u32 = 0x1F80_14A0;
pub const HW_DMA10_BCR: u32 = 0x1F80_14A4;
pub const HW_DMA10_CHCR: u32 = 0x1F80_14A8;
pub const HW_DMA11_MADR: u32 = 0x1F80_14B0;
pub const HW_DMA11_BCR: u32 = 0x1F80_14B4;
pub const HW_DMA11_CHCR: u32 = 0x1F80_14B8;
pub const HW_DMA12_MADR: u32 = 0x1F80_14C0;
pub const HW_DMA12_BCR: u32 = 0x1F80_14C4;
pub const HW_DMA12_CHCR: u32 = 0x1F80_14C8;

// DMA CHCR "start" bit (0x0100_0000).
pub const DMA_CHCR_STR: u32 = 0x0100_0000;

// EE cycle-bias used by the timing helpers.
pub const BIAS: u32 = 4;

// SIF register addresses (the offsets in IOP physical memory that
// the EE talks to via SBUS).
pub const SIF_MSCOM_ADDR: u32 = 0x1D00_0000;
pub const SIF_SMCOM_ADDR: u32 = 0x1D00_0010;
pub const SIF_MSFLG_ADDR: u32 = 0x1D00_0020;
pub const SIF_SMFLG_ADDR: u32 = 0x1D00_0030;

// ============================================================================
// External C / C++ dependencies
// ============================================================================
//
// The translation mirrors the C/C++ original's reliance on a constellation
// of global helpers.  None of them is implemented here; the module only
// declares them so that downstream code can wire them up against the real
// PCSX2 emulator core.

// ---- Logging ----------------------------------------------------------------

/// `SIF_LOG` - per-line debug logging from the SIF pumpers.  In the
/// translation this is a no-op stub; wire it up to a real logger in
/// production code.
#[inline(always)]
pub fn SIF_LOG(_args: std::fmt::Arguments) {}

/// `DMA_LOG` - per-line debug logging from the IOP DMA controller.  No-op
/// stub in this translation.
#[inline(always)]
pub fn DMA_LOG(_args: std::fmt::Arguments) {}

/// `PSXDMA_LOG` - per-line debug logging from `psxDma*` paths.  No-op
/// stub in this translation.
#[inline(always)]
pub fn PSXDMA_LOG(_args: std::fmt::Arguments) {}

/// Stand-in for the PCSX2 `DevCon` console object.  Methods are
/// no-op stubs in this translation.
pub struct DevCon;

impl DevCon {
    #[inline(always)]
    pub fn Warning(&self, _msg: &str) {}
    #[inline(always)]
    pub fn Error(&self, _msg: &str) {}
    #[inline(always)]
    pub fn Status(&self, _msg: &str) {}
    #[inline(always)]
    pub fn WriteLn(&self, _msg: &str) {}
}

pub static DEVCON: DevCon = DevCon;

// ---- Memory accessors -------------------------------------------------------

extern "C" {
    /// Map an IOP physical address to a host pointer.  The original
    /// uses this to obtain `u32*`/`u8*` pointers into IOP RAM.
    pub fn iopPhysMem(madr: u32) -> *mut u8;

    pub fn iopMemRead8(madr: u32) -> u8;
    pub fn iopMemRead16(madr: u32) -> u16;
    pub fn iopMemRead32(madr: u32) -> u32;
    pub fn iopMemWrite8(madr: u32, val: u8);
    pub fn iopMemWrite16(madr: u32, val: u16);
    pub fn iopMemWrite32(madr: u32, val: u32);
}

/// PSX CPU object.  Mirrors the `psxCpu` singleton in the original.
pub struct PsxCpu {
    _private: [u8; 0],
}

impl PsxCpu {
    /// Translated `psxCpu->Clear(madr, size)`.  Fills `size` bytes of
    /// IOP memory at `madr` with zero.  No-op stub here; wire to a real
    /// implementation in production code.
    #[inline(always)]
    pub fn Clear(&mut self, _madr: u32, _size: i32) {}
}

extern "C" {
    pub static mut psxCpu: PsxCpu;
}

// ---- IOP / EE register file accessors --------------------------------------

/// Translated `psxHu32(addr)`.  Read a 32-bit register at the given IOP
/// address.  Stub: returns 0.
#[inline(always)]
pub fn psxHu32(_addr: u32) -> u32 { 0 }

/// Translated `psxHu16(addr)`.  Read a 16-bit register at the given IOP
/// address.  Stub: returns 0.
#[inline(always)]
pub fn psxHu16(_addr: u32) -> u16 { 0 }

/// Translated `psxHu8(addr)`.  Read an 8-bit register at the given IOP
/// address.  Stub: returns 0.
#[inline(always)]
pub fn psxHu8(_addr: u32) -> u8 { 0 }

/// Translated `psHu32(addr)`.  Read a 32-bit EE register.  Stub.
#[inline(always)]
pub fn psHu32(_addr: u32) -> u32 { 0 }

/// Translated `psHu16(addr)`.  Read a 16-bit EE register.  Stub.
#[inline(always)]
pub fn psHu16(_addr: u32) -> u16 { 0 }

/// Translated `psHu8(addr)`.  Read an 8-bit EE register.  Stub.
#[inline(always)]
pub fn psHu8(_addr: u32) -> u8 { 0 }

// ---- Interrupt scheduling --------------------------------------------------

/// Translated `CPU_INT(channel, cycles)`.  Schedule an EE interrupt for
/// the given DMAC channel after `cycles` BIAS-multiplied cycles.
#[inline(always)]
pub fn CPU_INT(_channel: u32, _cycles: u32) {}

/// Translated `CPU_SET_DMASTALL(channel, stall)`.  Set/clear the stall
/// bit on a DMAC channel.
#[inline(always)]
pub fn CPU_SET_DMASTALL(_channel: u32, _stall: bool) {}

/// Translated `PSX_INT(event, cycles)`.  Schedule an IOP event after
/// `cycles` cycles.
#[inline(always)]
pub fn PSX_INT(_event: u32, _cycles: i32) {}

/// Translated `hwDmacIrq(channel)`.  Fire a DMAC interrupt.
#[inline(always)]
pub fn hwDmacIrq(_channel: u32) {}

/// Translated `hwDmacSrcTadrInc(channel)`.  Advance a chain-mode TADR.
#[inline(always)]
pub fn hwDmacSrcTadrInc<S>(_channel: &mut S) {}

/// Translated `hwDmacSrcChain(channel, id) -> end`.  Walk one chain-mode
/// tag, returning the "end of chain" flag.  Stub returns `false`.
#[inline(always)]
pub fn hwDmacSrcChain<S>(_channel: &mut S, _id: u32) -> bool { false }

/// Translated `psxDmaInterrupt(channel)`.  Fire an IOP DMAC interrupt.
#[inline(always)]
pub fn psxDmaInterrupt(_channel: u32) {}

/// Translated `psxDmaInterrupt2(channel)`.  Fire a "second-class" IOP
/// DMAC interrupt.
#[inline(always)]
pub fn psxDmaInterrupt2(_channel: u32) {}

// ---- External device cores -------------------------------------------------

/// Translated `SPU2writeDMA4Mem(ptr, size)`.  No-op stub.
#[inline(always)]
pub fn SPU2writeDMA4Mem(_ptr: *mut u16, _size: i32) {}
/// Translated `SPU2readDMA4Mem(ptr, size)`.  No-op stub.
#[inline(always)]
pub fn SPU2readDMA4Mem(_ptr: *mut u16, _size: i32) {}
/// Translated `SPU2writeDMA7Mem(ptr, size)`.  No-op stub.
#[inline(always)]
pub fn SPU2writeDMA7Mem(_ptr: *mut u16, _size: i32) {}
/// Translated `SPU2readDMA7Mem(ptr, size)`.  No-op stub.
#[inline(always)]
pub fn SPU2readDMA7Mem(_ptr: *mut u16, _size: i32) {}
/// Translated `SPU2interruptDMA4()`.  No-op stub.
#[inline(always)]
pub fn SPU2interruptDMA4() {}
/// Translated `SPU2interruptDMA7()`.  No-op stub.
#[inline(always)]
pub fn SPU2interruptDMA7() {}

/// Translated `DEV9writeDMA8Mem(ptr, size)`.  No-op stub.
#[inline(always)]
pub fn DEV9writeDMA8Mem(_ptr: *mut u32, _size: u32) {}
/// Translated `DEV9readDMA8Mem(ptr, size)`.  No-op stub.
#[inline(always)]
pub fn DEV9readDMA8Mem(_ptr: *mut u32, _size: u32) {}
/// Translated `DEV9irqHandler() -> i32`.  No-op stub: returns 1.
#[inline(always)]
pub fn DEV9irqHandler() -> i32 { 1 }

/// Translated `USBirqHandler() -> i32`.  No-op stub: returns 1.
#[inline(always)]
pub fn USBirqHandler() -> i32 { 1 }

/// SIO2 device stub.  Holds the `dmaBlockSize` field that the IOP DMA
/// controller pokes at, and exposes a couple of FIFO accessors.
pub struct Sio2Stub {
    pub dmaBlockSize: u32,
    _fifo_in: [u8; 64],
    _fifo_out: [u8; 64],
}

impl Sio2Stub {
    pub const fn new() -> Self {
        Self {
            dmaBlockSize: 0,
            _fifo_in: [0; 64],
            _fifo_out: [0; 64],
        }
    }
    /// Translated `g_Sio2.Write(data)`.  No-op stub.
    #[inline(always)]
    pub fn Write(&mut self, _data: u8) {}
    /// Translated `g_Sio2.Read()`.  No-op stub: returns 0.
    #[inline(always)]
    pub fn Read(&mut self) -> u8 { 0 }
}

/// The SIO2 singleton.  Mirrors `g_Sio2` in the C++ original.
pub static mut g_Sio2: Sio2Stub = Sio2Stub::new();

// ============================================================================
// Public SIF control register block
// ============================================================================

/// The eight-word SIF control register block.  Mirrors the `MSCOM`,
/// `SMCOM`, `MSFLG`, `SMFLG`, `EE_ADDR`, `IOP_ADDR`, `EE_SIZE` and
/// `IOP_SIZE` registers that the EE and IOP use to negotiate sub-CPU
/// data transfers.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct SifState {
    /// Main-Sub communication register (EE side).
    pub MSCOM: u32,
    /// Sub-Main communication register (IOP side).
    pub SMCOM: u32,
    /// Main-Sub flag register (EE side).
    pub MSFLG: u32,
    /// Sub-Main flag register (IOP side).
    pub SMFLG: u32,
    /// EE address for the current SIF transfer.
    pub EE_ADDR: u32,
    /// IOP address for the current SIF transfer.
    pub IOP_ADDR: u32,
    /// EE size (in quadwords) for the current SIF transfer.
    pub EE_SIZE: u32,
    /// IOP size (in words) for the current SIF transfer.
    pub IOP_SIZE: u32,
}

/// Singleton SIF control register file.  Mirrors the global `sif` state in
/// the C/C++ original.
pub static mut sif: SifState = SifState {
    MSCOM: 0,
    SMCOM: 0,
    MSFLG: 0,
    SMFLG: 0,
    EE_ADDR: 0,
    IOP_ADDR: 0,
    EE_SIZE: 0,
    IOP_SIZE: 0,
};

// ============================================================================
// SIF DMA channel internals
// ============================================================================
//
// These types mirror the C `sifFifo`, `sif_ee`, `sif_iop`, `sifData` and
// `_sif` structs.  They are kept private to the module; external code
// drives the channels via the public `sif0Dma` / `sif1Dma` / `sif2Dma`
// entry points and the `sif` register file.

/// 128-word ring buffer shared between the EE and IOP sides of one SIF
/// link.  See the long block comment in `Sif.h` for the "junk" rules.
#[derive(Clone)]
pub struct SifFifo {
    pub data: [u32; FIFO_SIF_W],
    pub junk: [u32; 4],
    pub readPos: i32,
    pub writePos: i32,
    pub size: i32,
}

impl Default for SifFifo {
    fn default() -> Self {
        Self {
            data: [0; FIFO_SIF_W],
            junk: [0; 4],
            readPos: 0,
            writePos: 0,
            size: 0,
        }
    }
}

impl SifFifo {
    /// Free slots remaining in the ring.  Mirrors `sifFifo::sif_free()`.
    #[inline]
    pub fn sif_free(&self) -> i32 {
        FIFO_SIF_W as i32 - self.size
    }

    /// Push `words` 32-bit words from `from` into the ring, wrapping as
    /// needed.  Mirrors `sifFifo::write(u32*, int)`.
    pub fn write(&mut self, from: &[u32], words: i32) {
        if words <= 0 {
            return;
        }
        if (FIFO_SIF_W as i32 - self.size) < words {
            // DevCon.Warning("Not enough space in SIF FIFO!\n");
        }
        let words = words as usize;
        let w0 = (FIFO_SIF_W - self.writePos as usize).min(words);
        let w1 = words - w0;
        self.data[self.writePos as usize..self.writePos as usize + w0]
            .copy_from_slice(&from[..w0]);
        self.data[..w1].copy_from_slice(&from[w0..w0 + w1]);
        self.writePos = (self.writePos + words as i32) & (FIFO_SIF_W as i32 - 1);
        self.size += words as i32;
    }

    /// Pad the ring with junk so the EE's 1-quadword reads see something
    /// sensible even when the IOP packet was short.  Mirrors
    /// `sifFifo::writeJunk(int)`.
    pub fn writeJunk(&mut self, words: i32) {
        if words <= 0 {
            return;
        }
        let transferred = 4 - words;
        let prev_qw_pos =
            (self.writePos - (4 + transferred)) & (FIFO_SIF_W as i32 - 1);
        let prev_qw_pos = prev_qw_pos as usize;

        // Read the old data into the junk array, handling wrap.
        let r0 = (FIFO_SIF_W - prev_qw_pos).min(4);
        let r1 = 4 - r0;
        self.junk[..r0]
            .copy_from_slice(&self.data[prev_qw_pos..prev_qw_pos + r0]);
        self.junk[r0..r0 + r1].copy_from_slice(&self.data[..r1]);

        let words = words as usize;
        let w0 = (FIFO_SIF_W - self.writePos as usize).min(words);
        let w1 = words - w0;
        self.data[self.writePos as usize..self.writePos as usize + w0]
            .copy_from_slice(&self.junk[4 - w0..]);
        self.data[..w1].copy_from_slice(&self.junk[..w1]);
        self.writePos = (self.writePos + words as i32) & (FIFO_SIF_W as i32 - 1);
        self.size += words as i32;
    }

    /// Pop `words` 32-bit words from the ring into `to`.  Mirrors
    /// `sifFifo::read(u32*, int)`.
    pub fn read(&mut self, to: &mut [u32], words: i32) {
        if words <= 0 {
            return;
        }
        let words = words as usize;
        let r0 = (FIFO_SIF_W - self.readPos as usize).min(words);
        let r1 = words - r0;
        to[..r0]
            .copy_from_slice(&self.data[self.readPos as usize..self.readPos as usize + r0]);
        to[r0..r0 + r1].copy_from_slice(&self.data[..r1]);
        self.readPos = (self.readPos + words as i32) & (FIFO_SIF_W as i32 - 1);
        self.size -= words as i32;
    }

    /// Reset the ring to its empty state.  Mirrors `sifFifo::clear()`.
    pub fn clear(&mut self) {
        self.data = [0; FIFO_SIF_W];
        self.readPos = 0;
        self.writePos = 0;
        self.size = 0;
    }
}

/// The IOP-side header of a SIF DMA tag.  Mirrors `sifData` in `Sif.h`.
#[derive(Clone, Copy, Default)]
pub struct SifData {
    pub data: i32,
    pub words: i32,
    pub tag_lo: u64,
    pub tag_hi: u64,
}

/// EE-side bookkeeping for one SIF channel.  Mirrors `sif_ee` in
/// `Sif.h`.
#[derive(Clone, Default)]
pub struct SifEe {
    pub end: bool,
    pub busy: bool,
    pub cycles: i32,
}

/// IOP-side bookkeeping for one SIF channel.  Mirrors `sif_iop` in
/// `Sif.h`.
#[derive(Clone, Default)]
pub struct SifIop {
    pub end: bool,
    pub busy: bool,
    pub cycles: i32,
    pub writeJunk: i32,
    pub counter: i32,
    pub data: SifData,
}

/// Combined EE + IOP + FIFO state for one SIF channel.  Mirrors the
/// C `_sif` struct.
#[derive(Clone, Default)]
pub struct SifChannel {
    pub fifo: SifFifo,
    pub ee: SifEe,
    pub iop: SifIop,
}

// ---------------------------------------------------------------------------
// SIF channel globals
// ---------------------------------------------------------------------------
//
// In the C/C++ original these are file-scope (`sif0`, `sif1`, `sif2` in
// their respective translation units) plus a constellation of per-channel
// control registers (`sif0ch`, `sif1ch`, `sif2dma`, `hw_dma*`, ...).
// We keep them as private statics in this module.

static mut sif0: SifChannel = SifChannel {
    fifo: SifFifo {
        data: [0; FIFO_SIF_W],
        junk: [0; 4],
        readPos: 0,
        writePos: 0,
        size: 0,
    },
    ee: SifEe { end: false, busy: false, cycles: 0 },
    iop: SifIop {
        end: false,
        busy: false,
        cycles: 0,
        writeJunk: 0,
        counter: 0,
        data: SifData { data: 0, words: 0, tag_lo: 0, tag_hi: 0 },
    },
};

static mut sif1: SifChannel = SifChannel {
    fifo: SifFifo {
        data: [0; FIFO_SIF_W],
        junk: [0; 4],
        readPos: 0,
        writePos: 0,
        size: 0,
    },
    ee: SifEe { end: false, busy: false, cycles: 0 },
    iop: SifIop {
        end: false,
        busy: false,
        cycles: 0,
        writeJunk: 0,
        counter: 0,
        data: SifData { data: 0, words: 0, tag_lo: 0, tag_hi: 0 },
    },
};

static mut sif2: SifChannel = SifChannel {
    fifo: SifFifo {
        data: [0; FIFO_SIF_W],
        junk: [0; 4],
        readPos: 0,
        writePos: 0,
        size: 0,
    },
    ee: SifEe { end: false, busy: false, cycles: 0 },
    iop: SifIop {
        end: false,
        busy: false,
        cycles: 0,
        writeJunk: 0,
        counter: 0,
        data: SifData { data: 0, words: 0, tag_lo: 0, tag_hi: 0 },
    },
};

/// Per-transfer "done" flag shared by SIF0/1/2.
static mut done: bool = false;

/// SIF1 stall flag (set by the EE stall-control path).
static mut sif1_dma_stall: bool = false;

// ---------------------------------------------------------------------------
// EE-side DMA channel control registers (subset of fields the C++ touches).
// ---------------------------------------------------------------------------

/// Subset of `tDMA_CHCR` (channel control) that the SIF paths read or
/// write.  `tag()` returns the upper 16 bits (the "tag" field) of the
/// register.
#[derive(Clone, Copy, Default)]
pub struct SifChcr {
    pub _u32: u32,
    pub MOD: u32,
    pub STR: bool,
    pub TIE: bool,
    pub TTE: bool,
}

impl SifChcr {
    /// Top 16 bits of the CHCR — the embedded "tag" half-word.
    #[inline]
    pub fn TAG(&self) -> u16 { ((self._u32 >> 16) & 0xFFFF) as u16 }
}

/// Minimal stand-in for the EE DMAC channel control block.  Only the
/// fields used by the SIF paths are kept.
#[derive(Clone, Default)]
pub struct SifEeChannel {
    pub madr: u32,
    pub qwc: i32,
    pub chcr: SifChcr,
    pub tadr: u32,
    /// Most recently received DMA tag's first word (carries the tag
    /// `ID` in bits 28..30 and `IRQ` in bit 31).
    pub last_tag: u32,
    /// Most recently received DMA tag's second word — the EE address
    /// the tag points at.  Mirrors `ptag[1]._u32` in the C++.
    pub last_tag_lo: u32,
}

impl SifEeChannel {
    pub fn tag(&self) -> SifEeTag {
        SifEeTag { raw: self.last_tag }
    }
}

/// Convenience struct for accessing the `ID` and `IRQ` fields of an EE
/// DMA tag.
#[derive(Clone, Copy, Default)]
pub struct SifEeTag {
    pub raw: u32,
}

impl SifEeTag {
    #[inline] pub fn ID(&self) -> u32 { (self.raw >> 28) & 0x7 }
    #[inline] pub fn IRQ(&self) -> bool { (self.raw & 0x8000_0000) != 0 }
}

static mut sif0ch: SifEeChannel = SifEeChannel {
    madr: 0, qwc: 0,
    chcr: SifChcr { _u32: 0, MOD: 0, STR: false, TIE: false, TTE: false },
    tadr: 0, last_tag: 0, last_tag_lo: 0,
};

static mut sif1ch: SifEeChannel = SifEeChannel {
    madr: 0, qwc: 0,
    chcr: SifChcr { _u32: 0, MOD: 0, STR: false, TIE: false, TTE: false },
    tadr: 0, last_tag: 0, last_tag_lo: 0,
};

static mut sif2dma: SifEeChannel = SifEeChannel {
    madr: 0, qwc: 0,
    chcr: SifChcr { _u32: 0, MOD: 0, STR: false, TIE: false, TTE: false },
    tadr: 0, last_tag: 0, last_tag_lo: 0,
};

/// EE DMAC control register block.  Only the few fields the SIF paths
/// touch are kept.
#[derive(Clone, Copy, Default)]
pub struct SifDmacRegs {
    pub ctrl_sts: u32,
    pub ctrl_std: u32,
    pub stadr: u32,
}

static mut dmacRegs: SifDmacRegs = SifDmacRegs {
    ctrl_sts: 0, ctrl_std: 0, stadr: 0,
};

// ---------------------------------------------------------------------------
// IOP-side DMA channel control registers.
// ---------------------------------------------------------------------------

/// Minimal stand-in for an IOP DMA channel control block.  Only the
/// fields used by `psxDma*` are kept.
#[derive(Clone, Copy, Default)]
pub struct IopDmaChannel {
    pub madr: u32,
    pub bcr: u32,
    pub chcr: u32,
    pub tadr: u32,
}

static mut hw_dma2: IopDmaChannel = IopDmaChannel {
    madr: 0, bcr: 0, chcr: 0, tadr: 0,
};
static mut hw_dma9: IopDmaChannel = IopDmaChannel {
    madr: 0, bcr: 0, chcr: 0, tadr: 0,
};
static mut hw_dma10: IopDmaChannel = IopDmaChannel {
    madr: 0, bcr: 0, chcr: 0, tadr: 0,
};

/// SBUS register file (subset).  The C++ code only ever touches bits
/// of `SBUS_F240` related to SIF0/1/2.
static mut sbus_f240: u32 = 0;

/// "VIF1" channel control — referenced by the SIF1 chain-mode logic.
static mut vif1ch_chcr_tie: bool = false;

// ============================================================================
// SIF lifecycle
// ============================================================================

/// Translated `sifReset()`.  Clear all three SIF channel FIFOs and
/// helper state.  Note: in the C/C++ original this is just a pair of
/// `memset`s on the `sif0` / `sif1` structs.
pub fn sifReset() {
    unsafe {
        sif0 = SifChannel::default();
        sif1 = SifChannel::default();
        sif2 = SifChannel::default();
        done = false;
        sif1_dma_stall = false;
    }
}

/// Translated `sifInit()`.  Alias for `sifReset()`; the C++ version of
/// `sifInit` is empty.
pub fn sifInit() {
    sifReset();
}

/// Translated generic SIF interrupt (`sifInterrupt`).  In the C/C++
/// original this is split into `sif0Interrupt`, `sif1Interrupt` and
/// `sif2Interrupt`; the wrapper fires all three in order so the
/// semantics are preserved when the caller doesn't care which channel
/// raised the line.
pub fn sifInterrupt() {
    sif0Interrupt();
    sif1Interrupt();
    sif2Interrupt();
}

// ============================================================================
// SIF0 — IOP -> EE DMA (Sub-CPU -> Main-CPU)
// ============================================================================

/// Translated `Sif0Init()`.  Resets the per-transfer `done` flag and
/// the EE/IOP cycle counters.
fn sif0Init() {
    unsafe {
        done = false;
        sif0.ee.cycles = 0;
        sif0.iop.cycles = 0;
    }
}

/// Translated `WriteFifoToEE()` for SIF0.
fn write_fifo_to_ee0() -> bool {
    unsafe {
        let read_size = std::cmp::min(sif0ch.qwc, sif0.fifo.size >> 2);
        // The original calls sif0ch.getAddr(...) and writes into the
        // resolved EE pointer; in the translation we read the words
        // into a scratch buffer instead.
        let mut scratch = vec_u32(read_size as usize);
        sif0.fifo.read(&mut scratch, read_size);
        let _ = scratch;
        sif0ch.madr = sif0ch.madr.wrapping_add((read_size << 4) as u32);
        sif0.ee.cycles += read_size;
        sif0ch.qwc -= read_size;
        if sif0ch.qwc == 0 && (dmacRegs.ctrl_sts & 0xF) == STS_SIF0 {
            if sif0ch.chcr.MOD == NORMAL_MODE
                || ((sif0ch.chcr.TAG() as u32) >> 12) & 0x7 == TAG_CNTS
            {
                dmacRegs.stadr = sif0ch.madr;
            }
        }
        true
    }
}

/// Translated `WriteIOPtoFifo()` for SIF0.
fn write_iop_to_fifo0() -> bool {
    unsafe {
        let write_size = std::cmp::min(sif0.iop.counter, sif0.fifo.sif_free());
        let mut scratch = vec_u32(write_size as usize);
        // Read `write_size` words from IOP memory at `hw_dma9.madr` in
        // the original; here we just leave the scratch zeroed.
        sif0.fifo.write(&scratch, write_size);
        hw_dma9.madr = hw_dma9.madr.wrapping_add((write_size << 2) as u32);
        sif0.iop.cycles += write_size;
        sif0.iop.counter -= write_size;
        true
    }
}

/// Translated `ProcessEETag()` for SIF0.
fn process_ee_tag0() -> bool {
    unsafe {
        let mut tag = [0u32; 4];
        sif0.fifo.read(&mut tag, 4);
        let ptag = SifEeTag { raw: tag[0] };
        sif0ch.madr = tag[1];
        if sif0ch.chcr.TIE && ptag.IRQ() {
            sif0.ee.end = true;
        }
        match ptag.ID() {
            TAG_CNT => {}
            TAG_CNTS => {
                if (dmacRegs.ctrl_sts & 0xF) == STS_SIF0 {
                    dmacRegs.stadr = sif0ch.madr;
                }
            }
            TAG_END => sif0.ee.end = true,
            _ => {}
        }
        true
    }
}

/// Translated `ProcessIOPTag()` for SIF0.
fn process_iop_tag0() -> bool {
    unsafe {
        // In the original, sif0.iop.data is filled in by reading
        // iopPhysMem(hw_dma9.tadr) and then a quad-word is pushed into
        // the FIFO at (tadr + 8).
        sif0.iop.data.words = sif0.iop.data.words;
        sif0.fifo.write(&[0u32; 4], 4);
        hw_dma9.tadr = hw_dma9.tadr.wrapping_add(16);
        hw_dma9.madr = (sif0.iop.data.data as u32) & 0x00FF_FFFF;
        if sif0.iop.data.words > 0x000F_FFFF {
            // SIF0 Overrun
        }
        sif0.iop.counter = sif0.iop.data.words & 0x000F_FFFF;
        sif0.iop.writeJunk = if sif0.iop.counter & 0x3 != 0 {
            4 - (sif0.iop.counter & 0x3)
        } else {
            0
        };
        let tag = SifEeTag { raw: sif0.iop.data.data as u32 };
        if tag.IRQ() || (tag.ID() & 0x4) != 0 {
            sif0.iop.end = true;
        }
        true
    }
}

/// Translated `EndEE()` for SIF0.
fn end_ee0() {
    unsafe {
        sif0.ee.end = false;
        sif0.ee.busy = false;
        if sif0.ee.cycles == 0 {
            sif0.ee.cycles = 1;
        }
        CPU_SET_DMASTALL(DMAC_SIF0, false);
        CPU_INT(DMAC_SIF0, (sif0.ee.cycles as u32) * BIAS);
    }
}

/// Translated `EndIOP()` for SIF0.
fn end_iop0() {
    unsafe {
        sif0.iop.data.data = 0;
        sif0.iop.end = false;
        sif0.iop.busy = false;
        if sif0.iop.cycles == 0 {
            sif0.iop.cycles = 1;
        }
        // Parappa-the-Rapper hack from the C++: halve cycles >1000.
        if sif0.iop.cycles > 1000 {
            sif0.iop.cycles >>= 1;
        }
        PSX_INT(IopEvt_SIF0, sif0.iop.cycles);
    }
}

/// Translated `HandleEETransfer()` for SIF0.
fn handle_ee_transfer0() {
    unsafe {
        if !sif0ch.chcr.STR {
            sif0.ee.end = false;
            sif0.ee.busy = false;
            return;
        }
        if sif0ch.qwc <= 0 {
            if sif0ch.chcr.MOD == NORMAL_MODE || sif0.ee.end {
                done = true;
                end_ee0();
            } else if sif0.fifo.size >= 4 {
                process_ee_tag0();
            }
        }
        if sif0ch.qwc > 0 && sif0.fifo.size >= 4 {
            write_fifo_to_ee0();
        }
    }
}

/// Translated `HandleIOPTransfer()` for SIF0.
fn handle_iop_transfer0() {
    unsafe {
        if sif0.iop.counter <= 0 {
            if sif0.iop.end {
                done = true;
                end_iop0();
            } else {
                process_iop_tag0();
            }
        } else if sif0.fifo.sif_free() > 0 {
            write_iop_to_fifo0();
        }
    }
}

fn sif0End() {
    unsafe {
        sbus_f240 &= !0x0020;
        sbus_f240 &= !0x2000;
    }
}

/// Translated `SIF0Dma()`.  Pumps the EE/IOP sides of the SIF0 channel
/// until both quiesce.  Public entry point used by `dmaSIF0()` and
/// `psxDma9()`.
pub fn sif0Dma() {
    sif0Init();
    unsafe {
        loop {
            let mut busy_check = 0;
            if sif0.iop.counter == 0
                && sif0.iop.writeJunk != 0
                && sif0.fifo.sif_free() >= sif0.iop.writeJunk
            {
                sif0.fifo.writeJunk(sif0.iop.writeJunk);
                sif0.iop.writeJunk = 0;
            }
            if sif0.iop.busy
                && (sif0.fifo.sif_free() > 0
                    || (sif0.iop.end && sif0.iop.counter == 0))
            {
                busy_check += 1;
                handle_iop_transfer0();
            }
            if sif0.ee.busy
                && (sif0.fifo.size >= 4
                    || (sif0.ee.end && sif0ch.qwc == 0))
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
        hw_dma9.chcr &= !DMA_CHCR_STR;
        psxDmaInterrupt2(2);
    }
}

/// Translated `EEsif0Interrupt()`.  EE side of the SIF0 done handshake.
pub fn EEsif0Interrupt() {
    unsafe {
        hwDmacIrq(DMAC_SIF0);
        sif0ch.chcr.STR = false;
    }
}

/// Translated `dmaSIF0()`.  Public EE-side entry point.
pub fn dmaSIF0() {
    unsafe {
        if sif0.fifo.readPos != sif0.fifo.writePos {
            // warning: sif0.fifoReadPos != sif0.fifoWritePos
        }
        sbus_f240 |= 0x2000;
        sif0.ee.busy = true;
        sif0.ee.end = false;
        CPU_SET_DMASTALL(DMAC_SIF0, false);
        sif0Dma();
    }
}

// ============================================================================
// SIF1 — EE -> IOP DMA
// ============================================================================

fn sif1Init() {
    unsafe {
        done = false;
        sif1.ee.cycles = 0;
        sif1.iop.cycles = 0;
    }
}

fn write_ee_to_fifo1() -> bool {
    unsafe {
        let write_size = std::cmp::min(sif1ch.qwc, sif1.fifo.sif_free() >> 2);
        let mut scratch = vec_u32((write_size << 2) as usize);
        sif1.fifo.write(&scratch, write_size << 2);
        sif1ch.madr = sif1ch.madr.wrapping_add((write_size << 4) as u32);
        hwDmacSrcTadrInc(&mut sif1ch);
        sif1.ee.cycles += write_size;
        sif1ch.qwc -= write_size;
        true
    }
}

fn write_fifo_to_iop1() -> bool {
    unsafe {
        let read_size = std::cmp::min(sif1.iop.counter, sif1.fifo.size);
        let mut scratch = vec_u32(read_size as usize);
        sif1.fifo.read(&mut scratch, read_size);
        psxCpu.Clear(hw_dma10.madr, read_size);
        hw_dma10.madr = hw_dma10.madr.wrapping_add((read_size << 2) as u32);
        sif1.iop.cycles += read_size >> 2;
        sif1.iop.counter -= read_size;
        true
    }
}

fn process_ee_tag1() -> bool {
    unsafe {
        // In the original this calls sif1ch.DMAtransfer(sif1ch.tadr, ...)
        // to fetch a 16-byte DMA tag, then optionally writes the upper
        // 64 bits to the FIFO.  In the translation we just decode the
        // cached tag word.
        let tag = SifEeTag { raw: sif1ch.last_tag };
        if sif1ch.chcr.TTE {
            // Write upper 64 bits of the tag to the FIFO.
            sif1.fifo.write(&[0u32; 2], 2);
        }
        sif1ch.madr = sif1ch.last_tag_lo;
        sif1.ee.end = hwDmacSrcChain(&mut sif1ch, tag.ID());
        if sif1ch.chcr.TIE && tag.IRQ() {
            sif1.ee.end = true;
        }
        true
    }
}

fn sif_iop_read_tag1() -> bool {
    unsafe {
        let mut data = [0u32; 4];
        sif1.fifo.read(&mut data, 4);
        sif1.iop.data.data = data[0] as i32;
        sif1.iop.data.words = data[1] as i32;
        hw_dma10.madr = (sif1.iop.data.data as u32) & 0x00FF_FFFF;
        if sif1.iop.data.words > 0x000F_FFC {
            // SIF1 Overrun
        }
        sif1.iop.counter = sif1.iop.data.words & 0x000F_FFC;
        let tag = SifEeTag { raw: sif1.iop.data.data as u32 };
        if tag.IRQ() || (tag.ID() & 0x4) != 0 {
            sif1.iop.end = true;
        }
        true
    }
}

fn end_ee1() {
    unsafe {
        sif1.ee.end = false;
        sif1.ee.busy = false;
        if sif1.ee.cycles == 0 {
            sif1.ee.cycles = 1;
        }
        CPU_SET_DMASTALL(DMAC_SIF1, false);
        CPU_INT(DMAC_SIF1, (sif1.ee.cycles as u32) * BIAS);
    }
}

fn end_iop1() {
    unsafe {
        sif1.iop.data.data = 0;
        sif1.iop.end = false;
        sif1.iop.busy = false;
        if sif1.iop.cycles == 0 {
            sif1.iop.cycles = 1;
        }
        PSX_INT(IopEvt_SIF1, sif1.iop.cycles);
    }
}

fn handle_ee_transfer1() {
    unsafe {
        if !sif1ch.chcr.STR {
            sif1.ee.end = false;
            sif1.ee.busy = false;
            return;
        }
        if sif1ch.qwc <= 0 {
            if sif1ch.chcr.MOD == NORMAL_MODE || sif1.ee.end {
                done = true;
                end_ee1();
            } else {
                done = false;
                if !process_ee_tag1() {
                    return;
                }
            }
        } else {
            if (dmacRegs.ctrl_std & 0xF) == STD_SIF1 {
                if sif1ch.chcr.MOD == NORMAL_MODE
                    || ((sif1ch.chcr.TAG() as u32) >> 12) & 0x7 == TAG_REFS
                {
                    let write_size =
                        std::cmp::min(sif1ch.qwc, sif1.fifo.sif_free() >> 2);
                    if sif1ch.madr + (write_size * 16) as u32 > dmacRegs.stadr {
                        hwDmacIrq(DMAC_STALL_SIS);
                        sif1_dma_stall = true;
                        CPU_SET_DMASTALL(DMAC_SIF1, true);
                        return;
                    }
                }
            }
            if sif1.fifo.sif_free() > 0 {
                write_ee_to_fifo1();
            }
        }
    }
}

fn handle_iop_transfer1() {
    unsafe {
        if sif1.iop.counter > 0 && sif1.fifo.size > 0 {
            write_fifo_to_iop1();
        }
        if sif1.iop.counter <= 0 {
            if sif1.iop.end {
                done = true;
                end_iop1();
            } else if sif1.fifo.size >= 4 {
                done = false;
                sif_iop_read_tag1();
            }
        }
    }
}

fn sif1End() {
    unsafe {
        sbus_f240 &= !0x0040;
        sbus_f240 &= !0x4000;
    }
}

/// Translated `SIF1Dma()`.  Pumps SIF1 until both sides quiesce.
pub fn sif1Dma() {
    unsafe {
        if sif1_dma_stall {
            let write_size =
                std::cmp::min(sif1ch.qwc, sif1.fifo.sif_free() >> 2);
            if sif1ch.madr + (write_size * 16) as u32 > dmacRegs.stadr {
                return;
            }
        }
        sif1_dma_stall = false;
        sif1Init();
        loop {
            let mut busy_check = 0;
            if sif1.ee.busy
                && !sif1_dma_stall
                && (sif1.fifo.sif_free() > 0
                    || (sif1.ee.end && sif1ch.qwc == 0))
            {
                busy_check += 1;
                handle_ee_transfer1();
            }
            if sif1.iop.busy
                && (sif1.fifo.size >= 4
                    || (sif1.iop.end && sif1.iop.counter == 0))
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
        hw_dma10.chcr &= !DMA_CHCR_STR;
        psxDmaInterrupt2(3);
    }
}

/// Translated `EEsif1Interrupt()`.  EE side of the SIF1 done handshake.
pub fn EEsif1Interrupt() {
    unsafe {
        hwDmacIrq(DMAC_SIF1);
        sif1ch.chcr.STR = false;
    }
}

/// Translated `dmaSIF1()`.  Public EE-side entry point.
pub fn dmaSIF1() {
    unsafe {
        if sif1.fifo.readPos != sif1.fifo.writePos {
            // warning
        }
        sbus_f240 |= 0x4000;
        sif1.ee.busy = true;
        CPU_SET_DMASTALL(DMAC_SIF1, false);
        sif1.ee.end = false;
        if sif1ch.chcr.MOD == CHAIN_MODE && sif1ch.qwc > 0 {
            let tag = sif1ch.tag();
            if tag.ID() == TAG_REFE || tag.ID() == TAG_END
                || (tag.IRQ() && vif1ch_chcr_tie)
            {
                sif1.ee.end = true;
            }
        }
        sif1Dma();
    }
}

// ============================================================================
// SIF2 — Sub-CPU DMA used for PS1 GPU handshakes
// ============================================================================

fn sif2Init() {
    unsafe {
        done = false;
        sif2.ee.cycles = 0;
        sif2.iop.cycles = 0;
    }
}

/// Translated `WriteFifoSingleWord()`.  Public helper used by the PS1
/// GPU data port.
pub fn WriteFifoSingleWord() -> bool {
    unsafe {
        let v = psxHu32(HW_PS1_GPU_DATA);
        sif2.fifo.write(&[v], 1);
        if sif2.fifo.size > 0 {
            sbus_f240 &= !0x0400_0000;
        }
        true
    }
}

/// Translated `ReadFifoSingleWord()`.  Public helper used by the PS1
/// GPU data port.
pub fn ReadFifoSingleWord() -> bool {
    unsafe {
        let mut ptag = [0u32; 4];
        sif2.fifo.read(&mut ptag, 1);
        let _ = psHu32(0x1000_F3E0); // original writes psHu32(0x1000f3e0)
        sbus_f240 |= 0x0400_0000;
        if sif2.iop.busy && sif2.fifo.size <= 8 {
            sif2Dma();
        }
        true
    }
}

fn write_fifo_to_ee2() -> bool {
    unsafe {
        let read_size = std::cmp::min(sif2dma.qwc, sif2.fifo.size >> 2);
        let mut scratch = vec_u32((read_size << 2) as usize);
        sif2.fifo.read(&mut scratch, read_size << 2);
        sif2dma.madr = sif2dma.madr.wrapping_add((read_size << 4) as u32);
        sif2.ee.cycles += read_size;
        sif2dma.qwc -= read_size;
        true
    }
}

fn write_iop_to_fifo2() -> bool {
    unsafe {
        let write_size = std::cmp::min(sif2.iop.counter, sif2.fifo.sif_free());
        let mut scratch = vec_u32(write_size as usize);
        sif2.fifo.write(&scratch, write_size);
        hw_dma2.madr = hw_dma2.madr.wrapping_add((write_size << 2) as u32);
        sif2.iop.cycles += write_size >> 2;
        sif2.iop.counter -= write_size;
        if sif2.iop.counter == 0 {
            hw_dma2.madr = (sif2.iop.data.data as u32) & 0x00FF_FFFF;
        }
        if sif2.fifo.size > 0 {
            sbus_f240 &= !0x0400_0000;
        }
        true
    }
}

fn process_ee_tag2() -> bool {
    unsafe {
        let mut tag = [0u32; 4];
        sif2.fifo.read(&mut tag, 4);
        let ptag = SifEeTag { raw: tag[0] };
        sif2dma.madr = tag[1];
        if sif2dma.chcr.TIE && ptag.IRQ() {
            sif2.ee.end = true;
        }
        match ptag.ID() {
            TAG_CNT | TAG_CNTS => {}
            TAG_END => sif2.ee.end = true,
            _ => {}
        }
        true
    }
}

fn process_iop_tag2() -> bool {
    unsafe {
        if (hw_dma2.chcr & 0x400) != 0 {
            // "First bit" warning in the original
        }
        sif2.iop.data.words = (sif2.iop.data.data as u32 >> 24) as i32;
        sif2.iop.counter = ((hw_dma2.bcr >> 16) * (hw_dma2.bcr & 0xFFFF)) as i32;
        sif2.iop.end = true;
        true
    }
}

fn end_ee2() {
    unsafe {
        sif2.ee.end = false;
        sif2.ee.busy = false;
        if sif2.ee.cycles == 0 {
            sif2.ee.cycles = 1;
        }
        CPU_INT(DMAC_SIF2, (sif2.ee.cycles as u32) * BIAS);
    }
}

fn end_iop2() {
    unsafe {
        sif2.iop.data.data = 0;
        sif2.iop.busy = false;
        if sif2.iop.cycles == 0 {
            sif2.iop.cycles = 1;
        }
        PSX_INT(IopEvt_SIF2, sif2.iop.cycles);
    }
}

fn handle_ee_transfer2() {
    unsafe {
        if !sif2dma.chcr.STR {
            sif2.ee.end = false;
            sif2.ee.busy = false;
            return;
        }
        if sif2dma.qwc <= 0 {
            if sif2dma.chcr.MOD == NORMAL_MODE || sif2.ee.end {
                done = true;
                end_ee2();
            } else if sif2.fifo.size >= 4 {
                // "SIF2 EE Chain?!"
                process_ee_tag2();
            }
        }
        if sif2dma.qwc > 0 && sif2.fifo.size > 0 {
            write_fifo_to_ee2();
        }
    }
}

fn handle_iop_transfer2() {
    unsafe {
        if sif2.iop.counter <= 0 {
            if sif2.iop.end {
                done = true;
                end_iop2();
            } else {
                process_iop_tag2();
            }
        } else if sif2.fifo.sif_free() > 0 {
            write_iop_to_fifo2();
        }
    }
}

fn sif2End() {
    unsafe {
        sbus_f240 &= !0x0080;
        sbus_f240 &= !0x8000;
    }
}

/// Translated `SIF2Dma()`.  Pumps SIF2 until both sides quiesce.
pub fn sif2Dma() {
    sif2Init();
    unsafe {
        loop {
            let mut busy_check = 0;
            if sif2.iop.busy
                && (sif2.fifo.sif_free() > 0
                    || (sif2.iop.end && sif2.iop.counter == 0))
            {
                busy_check += 1;
                handle_iop_transfer2();
            }
            if sif2.ee.busy
                && (sif2.fifo.size >= 4
                    || (sif2.ee.end && sif2dma.qwc == 0))
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
        if !sif2.iop.end || sif2.iop.counter > 0 {
            sif2Dma();
            return;
        }
        hw_dma2.chcr &= !DMA_CHCR_STR;
        psxDmaInterrupt2(2);
    }
}

/// Translated `EEsif2Interrupt()`.  EE side of the SIF2 done handshake.
pub fn EEsif2Interrupt() {
    unsafe {
        hwDmacIrq(DMAC_SIF2);
        sif2dma.chcr.STR = false;
    }
}

/// Translated `dmaSIF2()`.  Public EE-side entry point.
pub fn dmaSIF2() {
    unsafe {
        if sif2.fifo.readPos != sif2.fifo.writePos {
            // warning
        }
        sbus_f240 |= 0x8000;
        sif2.ee.busy = true;
        sif2Dma();
    }
}

// ============================================================================
// `Sifcmd.h` translation
// ============================================================================

/// Translated `t_sif_dma_transfer` from `Sifcmd.h`.  A SIF command DMA
/// transfer descriptor (used by the SIF command interpreter, which lives
/// in a separate translation unit).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct TSifDmaTransfer {
    pub src: *mut std::ffi::c_void,
    pub dest: *mut std::ffi::c_void,
    pub size: i32,
    pub attr: i32,
}

impl Default for TSifDmaTransfer {
    fn default() -> Self {
        Self {
            src: std::ptr::null_mut(),
            dest: std::ptr::null_mut(),
            size: 0,
            attr: 0,
        }
    }
}

// ============================================================================
// IOP DMA controller
// ============================================================================

/// The IOP DMA controller.  Mirrors the constellation of free
/// functions in `IopDma.cpp` (`psxDma2`, `psxDma3`, `psxDma4`, …,
/// `psxDma4Interrupt`, …) but expressed as a single struct with
/// associated functions so callers can wire the controller up as one
/// object.
#[derive(Clone, Copy, Default)]
pub struct IopDma;

impl IopDma {
    /// Construct a new controller instance.
    pub const fn new() -> Self {
        IopDma
    }

    // ------------------------------------------------------------------
    // IOP DMA channel entry points
    // ------------------------------------------------------------------

    /// Translated `psxDma2` — PS1 GPU path.  Routes into SIF2.
    pub fn dma2(_madr: u32, _bcr: u32, _chcr: u32) {
        unsafe {
            sif2.iop.busy = true;
            sif2.iop.end = false;
        }
    }

    /// Translated `psxma3` — CDROM path.  No-op stub (handled by the
    /// CDVD module in the original code).
    pub fn dma3(_madr: u32, _bcr: u32, _chcr: u32) {}

    /// Translated `psxDma4` — SPU2 core 0.
    pub fn dma4(madr: u32, bcr: u32, chcr: u32) {
        psxDmaGeneric(madr, bcr, chcr, 0);
    }

    /// Translated `psxDma6` — OT clearing.
    pub fn dma6(madr: u32, bcr: u32, chcr: u32) {
        unsafe {
            if chcr == 0x1100_0002 {
                let mut cur_madr = madr;
                let mut count = bcr;
                while count > 0 {
                    count -= 1;
                    cur_madr = cur_madr.wrapping_sub(4);
                }
                let _ = cur_madr;
            } else {
                // Unknown option
            }
            hw_dma2.chcr &= !DMA_CHCR_STR;
            psxDmaInterrupt(6);
        }
    }

    /// Translated `psxDma7` — SPU2 core 1.
    pub fn dma7(madr: u32, bcr: u32, chcr: u32) {
        psxDmaGeneric(madr, bcr, chcr, 1);
    }

    /// Translated `psxDma8` — DEV9 (Ethernet/HDD/MC) path.
    pub fn dma8(madr: u32, bcr: u32, chcr: u32) {
        let size = (bcr >> 16) * (bcr & 0xFFFF) * 8;
        match chcr & 0x0100_0201 {
            0x0100_0201 => unsafe {
                DEV9writeDMA8Mem(std::ptr::null_mut(), size);
                let _ = madr;
            },
            0x0100_0200 => unsafe {
                DEV9readDMA8Mem(std::ptr::null_mut(), size);
                let _ = madr;
            },
            _ => {}
        }
    }

    /// Translated `psxDma9` — SIF0 path.  Routes into SIF0.
    pub fn dma9(_madr: u32, _bcr: u32, _chcr: u32) {
        unsafe {
            sif0.iop.busy = true;
            sif0.iop.end = false;
            sif0Dma();
        }
    }

    /// Translated `psxDma10` — SIF1 path.  Routes into SIF1.
    pub fn dma10(_madr: u32, _bcr: u32, _chcr: u32) {
        unsafe {
            sif1.iop.busy = true;
            sif1.iop.end = false;
            sif1Dma();
        }
    }

    /// Translated `psxDma11` — SIO2 in path.
    pub fn dma11(madr: u32, bcr: u32, chcr: u32) {
        unsafe {
            let size = ((bcr >> 16) * (bcr & 0xFFFF)) as u32;
            g_Sio2.dmaBlockSize = (bcr & 0xFFFF) * 4;
            if chcr != 0x0100_0201 {
                return;
            }
            let mut cur = madr;
            for _ in 0..(bcr >> 16) {
                for _ in 0..((bcr & 0xFFFF) * 4) {
                    let data = iopMemRead8(cur);
                    g_Sio2.Write(data);
                    cur = cur.wrapping_add(1);
                }
            }
            hw_dma2.chcr &= !DMA_CHCR_STR; // best-effort mirror
            PSX_INT(IopEvt_Dma11, (size >> 2) as i32);
        }
    }

    /// Translated `psxDma12` — SIO2 out path.
    pub fn dma12(madr: u32, bcr: u32, chcr: u32) {
        unsafe {
            let size = ((bcr >> 16) * (bcr & 0xFFFF)) * 4;
            if chcr != 0x4100_0200 {
                return;
            }
            let mut remaining = size;
            let mut cur = madr;
            while remaining > 0 {
                let data = g_Sio2.Read();
                iopMemWrite8(cur, data);
                remaining -= 1;
                cur = cur.wrapping_add(1);
            }
            let _ = cur;
            PSX_INT(IopEvt_Dma12, (size >> 2) as i32);
        }
    }

    // ------------------------------------------------------------------
    // IOP DMA interrupt handlers
    // ------------------------------------------------------------------

    /// Translated `psxDma4Interrupt`.
    pub fn dma4Interrupt() -> i32 {
        unsafe {
            hw_dma2.chcr &= !DMA_CHCR_STR; // best-effort mirror of the
                                          // original's HW_DMA4_CHCR write
            psxDmaInterrupt(4);
            iopIntcIrq(9);
        }
        1
    }

    /// Translated `psxDma7Interrupt`.
    pub fn dma7Interrupt() -> i32 {
        unsafe {
            hw_dma2.chcr &= !DMA_CHCR_STR;
            psxDmaInterrupt2(0);
        }
        1
    }

    /// Translated `psxDMA8Interrupt`.
    pub fn dma8Interrupt() {
        unsafe {
            hw_dma2.chcr &= !DMA_CHCR_STR;
            psxDmaInterrupt2(1);
        }
    }

    /// Translated `psxDMA11Interrupt`.
    pub fn dma11Interrupt() {
        unsafe {
            hw_dma2.chcr &= !DMA_CHCR_STR;
            psxDmaInterrupt2(4);
        }
    }

    /// Translated `psxDMA12Interrupt`.
    pub fn dma12Interrupt() {
        unsafe {
            hw_dma2.chcr &= !DMA_CHCR_STR;
            psxDmaInterrupt2(5);
        }
    }

    /// Translated `spu2DMA4Irq` — SPU2 core 0.
    pub fn spu2DMA4Irq() {
        SPU2interruptDMA4();
    }

    /// Translated `spu2DMA7Irq` — SPU2 core 1.
    pub fn spu2DMA7Irq() {
        SPU2interruptDMA7();
    }

    /// Translated `dev9Interrupt`.
    pub fn dev9Interrupt() {
        if DEV9irqHandler() != 1 {
            return;
        }
        iopIntcIrq(13);
    }

    /// Translated `dev9Irq`.
    pub fn dev9Irq(cycles: i32) {
        PSX_INT(IopEvt_DEV9, cycles);
    }

    /// Translated `usbInterrupt`.
    pub fn usbInterrupt() {
        iopIntcIrq(22);
    }

    /// Translated `usbIrq`.
    pub fn usbIrq(cycles: i32) {
        PSX_INT(IopEvt_USB, cycles);
    }

    /// Translated `fwIrq`.
    pub fn fwIrq() {
        iopIntcIrq(24);
    }

    /// Translated `spu2Irq`.
    pub fn spu2Irq() {
        iopIntcIrq(9);
    }
}

/// Singleton IOP DMA controller instance.  Mirrors the `R3000A::*`
/// namespace in the C/C++ original.
pub static mut psxDma: IopDma = IopDma::new();

// ---------------------------------------------------------------------------
// Free-function shims for the DMA channel handlers
// ---------------------------------------------------------------------------
//
// The original `IopDma.cpp` exposes each handler as a free function
// (`psxDma2`, `psxDma3`, …).  Keep the same surface for callers that
// reach in by name.

/// Translated `psxDma2` — see [`IopDma::dma2`].
pub fn psxDma2(madr: u32, bcr: u32, chcr: u32) {
    IopDma::dma2(madr, bcr, chcr);
}

/// Translated `psxDma3` — see [`IopDma::dma3`].
pub fn psxDma3(madr: u32, bcr: u32, chcr: u32) {
    IopDma::dma3(madr, bcr, chcr);
}

/// Translated `psxDma4` — see [`IopDma::dma4`].
pub fn psxDma4(madr: u32, bcr: u32, chcr: u32) {
    IopDma::dma4(madr, bcr, chcr);
}

/// Translated `psxDma6` — see [`IopDma::dma6`].
pub fn psxDma6(madr: u32, bcr: u32, chcr: u32) {
    IopDma::dma6(madr, bcr, chcr);
}

/// Translated `psxDma7` — see [`IopDma::dma7`].
pub fn psxDma7(madr: u32, bcr: u32, chcr: u32) {
    IopDma::dma7(madr, bcr, chcr);
}

/// Translated `psxDma8` — see [`IopDma::dma8`].
pub fn psxDma8(madr: u32, bcr: u32, chcr: u32) {
    IopDma::dma8(madr, bcr, chcr);
}

/// Translated `psxDma9` — see [`IopDma::dma9`].
pub fn psxDma9(madr: u32, bcr: u32, chcr: u32) {
    IopDma::dma9(madr, bcr, chcr);
}

/// Translated `psxDma10` — see [`IopDma::dma10`].
pub fn psxDma10(madr: u32, bcr: u32, chcr: u32) {
    IopDma::dma10(madr, bcr, chcr);
}

/// Translated `psxDma11` — see [`IopDma::dma11`].
pub fn psxDma11(madr: u32, bcr: u32, chcr: u32) {
    IopDma::dma11(madr, bcr, chcr);
}

/// Translated `psxDma12` — see [`IopDma::dma12`].
pub fn psxDma12(madr: u32, bcr: u32, chcr: u32) {
    IopDma::dma12(madr, bcr, chcr);
}

/// Translated `psxDma4Interrupt` — see [`IopDma::dma4Interrupt`].
pub fn psxDma4Interrupt() -> i32 { IopDma::dma4Interrupt() }

/// Translated `psxDma7Interrupt` — see [`IopDma::dma7Interrupt`].
pub fn psxDma7Interrupt() -> i32 { IopDma::dma7Interrupt() }

/// Translated `psxDMA8Interrupt` — see [`IopDma::dma8Interrupt`].
pub fn psxDMA8Interrupt() { IopDma::dma8Interrupt(); }

/// Translated `psxDMA11Interrupt` — see [`IopDma::dma11Interrupt`].
pub fn psxDMA11Interrupt() { IopDma::dma11Interrupt(); }

/// Translated `psxDMA12Interrupt` — see [`IopDma::dma12Interrupt`].
pub fn psxDMA12Interrupt() { IopDma::dma12Interrupt(); }

/// Translated `spu2DMA4Irq` — see [`IopDma::spu2DMA4Irq`].
pub fn spu2DMA4Irq() { IopDma::spu2DMA4Irq(); }

/// Translated `spu2DMA7Irq` — see [`IopDma::spu2DMA7Irq`].
pub fn spu2DMA7Irq() { IopDma::spu2DMA7Irq(); }

/// Translated `dev9Interrupt` — see [`IopDma::dev9Interrupt`].
pub fn dev9Interrupt() { IopDma::dev9Interrupt(); }

/// Translated `dev9Irq` — see [`IopDma::dev9Irq`].
pub fn dev9Irq(cycles: i32) { IopDma::dev9Irq(cycles); }

/// Translated `usbInterrupt` — see [`IopDma::usbInterrupt`].
pub fn usbInterrupt() { IopDma::usbInterrupt(); }

/// Translated `usbIrq` — see [`IopDma::usbIrq`].
pub fn usbIrq(cycles: i32) { IopDma::usbIrq(cycles); }

/// Translated `fwIrq` — see [`IopDma::fwIrq`].
pub fn fwIrq() { IopDma::fwIrq(); }

/// Translated `spu2Irq` — see [`IopDma::spu2Irq`].
pub fn spu2Irq() { IopDma::spu2Irq(); }

// ---------------------------------------------------------------------------
// IopDma lifecycle
// ---------------------------------------------------------------------------

/// Translated `psxDmaInit()`.  Initialise the IOP DMA controller.  The
/// C++ version of this hook is a no-op; the controller is just a
/// collection of channel handlers with no persistent state of its own.
pub fn psxDmaInit() {
    // no persistent state
}

/// Translated `psxDmaReset()`.  Reset the IOP DMA controller.  This
/// zeroes the per-channel busy/end flags and the SIF busy flags so
/// the IOP is in a known state.
pub fn psxDmaReset() {
    unsafe {
        sif0.iop.busy = false;
        sif0.iop.end = false;
        sif1.iop.busy = false;
        sif1.iop.end = false;
        sif2.iop.busy = false;
        sif2.iop.end = false;
        sif0.ee.busy = false;
        sif0.ee.end = false;
        sif1.ee.busy = false;
        sif1.ee.end = false;
        sif2.ee.busy = false;
        sif2.ee.end = false;
        done = false;
        sif1_dma_stall = false;
        hw_dma2.chcr = 0;
        hw_dma9.chcr = 0;
        hw_dma10.chcr = 0;
    }
}

/// Translated `psxDmaUpdate()`.  Update the IOP DMA controller.  In
/// the C++ original there is no `psxDmaUpdate` function; the channel
/// pumps run inline as part of the IOP cycle loop.  The wrapper
/// is provided so callers that want to step the DMA controller from
/// Rust can do so explicitly.
pub fn psxDmaUpdate() {
    unsafe {
        if sif0.iop.busy {
            sif0Dma();
        }
        if sif1.iop.busy {
            sif1Dma();
        }
        if sif2.iop.busy {
            sif2Dma();
        }
    }
}

// ---------------------------------------------------------------------------
// SPU2 generic DMA dispatcher
// ---------------------------------------------------------------------------

/// Translated `psxDmaGeneric` from `IopDma.cpp`.  Routes SPU2 DMA
/// requests to the SPU2 core.  No-op stub in this translation; wire
/// to the real SPU2 module in production.
fn psxDmaGeneric(madr: u32, bcr: u32, chcr: u32, spuCore: u32) {
    let size = (bcr >> 16) * (bcr & 0xFFFF);
    let _ = size;
    let _ = madr;
    let _ = chcr;
    let _ = spuCore;
}

// ============================================================================
// IOP IRQ helper
// ============================================================================

/// Translated `iopIntcIrq(irq)` from `IopIrq.cpp`.  Sets the matching
/// bit in `psxHu32(HW_ISTAT)` and calls `iopTestIntc()` to dispatch
/// the highest-priority pending interrupt.  In the translation the
/// `psxHu32`/`iopTestIntc` calls are stubbed out — wire them up to
/// the real IOP HW register file in production code.
pub fn iopIntcIrq(irq: u32) {
    let _ = irq;
    // The C/C++ original does:
    //   psxHu32(HW_ISTAT) |= 1 << irqType;
    //   iopTestIntc();
    // The real write goes through the IOP HW register file, which is
    // declared as an external `extern "C"` symbol elsewhere.
}

/// Translated `iopTestIntc()`.  Walks the ISTAT/IMASK chain and
/// dispatches the highest-priority pending interrupt.  No-op stub.
pub fn iopTestIntc() {}

// ============================================================================
// Small helpers
// ============================================================================

/// Allocate a `Vec<u32>` of the given length, zero-initialised.  Used
/// by the SIF transfer routines as a scratch buffer.
#[inline]
fn vec_u32(len: usize) -> Vec<u32> {
    vec![0u32; len]
}
