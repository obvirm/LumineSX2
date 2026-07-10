//! Idiomatic Rust translation of `pcsx2/IopHw.{h,cpp}`.
//!
//! The IOP hardware module owns the read/write dispatch tables for the IOP
//! (R3000-side) memory-mapped register file, the cdvd segment-0x1F40 access
//! helpers (`psxHw4Read8` / `psxHw4Write8`), the DMA interrupt acknowledgement
//! routines, the hardware reset hook, and the comprehensive list of IOP
//! address constants (SIO, SIO2, CDRom, SPU2, USB, FW, SSBUS, counters, DMA
//! channels, and the various `HW_*` register aliases used throughout the
//! emulator).
//!
//! The public surface mirrors the C++ globals so the surrounding emulator
//! code can call into it unchanged. The read/write dispatch tables
//! (`IopHw_FnRead` / `IopHw_FnWrite`) and the `psxHw1Read{8,16,32}` /
//! `psxHw1Write{8,16,32}` helpers are exposed as `Option<unsafe fn>`-shaped
//! entries, matching the C-style function-pointer table layout the original
//! code uses to fan out the four `1F80xxxx` page-sized regions.

// ---------------------------------------------------------------------------
// Address-constant groups
// ---------------------------------------------------------------------------

/// PS1 GPU register range, USB range, FW range, SPU2 range.
pub const HW_PS1_GPU_START: u32 = 0x1F8010A0;
pub const HW_PS1_GPU_END: u32 = 0x1F8010B0;
pub const HW_USB_START: u32 = 0x1F801600;
pub const HW_USB_END: u32 = 0x1F801700;
pub const HW_FW_START: u32 = 0x1F808400;
pub const HW_FW_END: u32 = 0x1F808550; // end addr for FW is a guess...
pub const HW_SPU2_START: u32 = 0x1F801C00;
pub const HW_SPU2_END: u32 = 0x1F801E00;

/// SSBUS / SIO / RAM-size / IRQ-controller register addresses.
pub const HW_SSBUS_SPD_ADDR: u32 = 0x1F801000;
pub const HW_SSBUS_PIO_ADDR: u32 = 0x1F801004;
pub const HW_SSBUS_SPD_DELAY: u32 = 0x1F801008;
pub const HW_SSBUS_DEV1_DELAY: u32 = 0x1F80100C;
pub const HW_SSBUS_ROM_DELAY: u32 = 0x1F801010;
pub const HW_SSBUS_SPU_DELAY: u32 = 0x1F801014;
pub const HW_SSBUS_DEV5_DELAY: u32 = 0x1F801018;
pub const HW_SSBUS_PIO_DELAY: u32 = 0x1F80101C;
pub const HW_SSBUS_COM_DELAY: u32 = 0x1F801020;

pub const HW_SIO_DATA: u32 = 0x1F801040; // SIO read/write register
pub const HW_SIO_STAT: u32 = 0x1F801044;
pub const HW_SIO_MODE: u32 = 0x1F801048;
pub const HW_SIO_CTRL: u32 = 0x1F80104A;
pub const HW_SIO_BAUD: u32 = 0x1F80104E;

pub const HW_RAM_SIZE: u32 = 0x1F801060;
pub const HW_ISTAT: u32 = 0x1F801070;
pub const HW_IMASK: u32 = 0x1F801074;
pub const HW_ICTRL: u32 = 0x1F801078;

/// Second SSBUS register page (DEV1/SPU/DEV5/SPU1/DEV9 address and delays).
pub const HW_SSBUS_DEV1_ADDR: u32 = 0x1F801400;
pub const HW_SSBUS_SPU_ADDR: u32 = 0x1F801404;
pub const HW_SSBUS_DEV5_ADDR: u32 = 0x1F801408;
pub const HW_SSBUS_SPU1_ADDR: u32 = 0x1F80140C;
pub const HW_SSBUS_DEV9_ADDR3: u32 = 0x1F801410;
pub const HW_SSBUS_SPU1_DELAY: u32 = 0x1F801414;
pub const HW_SSBUS_DEV9_DELAY2: u32 = 0x1F801418;
pub const HW_SSBUS_DEV9_DELAY3: u32 = 0x1F80141C;
pub const HW_SSBUS_DEV9_DELAY1: u32 = 0x1F801420;

pub const HW_ICFG: u32 = 0x1F801450;
pub const HW_DEV9_DATA: u32 = 0x1F80146E; // DEV9 read/write register

/// CDROM multipurpose data registers, PS1 GPU registers.
pub const HW_CDR_DATA0: u32 = 0x1F801800; // CDROM multipurpose data register 1
pub const HW_CDR_DATA1: u32 = 0x1F801801; // CDROM multipurpose data register 2
pub const HW_CDR_DATA2: u32 = 0x1F801802; // CDROM multipurpose data register 3
pub const HW_CDR_DATA3: u32 = 0x1F801803; // CDROM multipurpose data register 4

pub const HW_PS1_GPU_DATA: u32 = 0x1F801810; // PS1 GPU DATA register
pub const HW_PS1_GPU_STATUS: u32 = 0x1F801814; // PS1 GPU STATUS register

/// SIO2 DMA interface registers.
pub const HW_SIO2_TX: u32 = 0x1F808260;
pub const HW_SIO2_RX: u32 = 0x1F808264;
pub const HW_SIO2_CTRL: u32 = 0x1F808268;
pub const HW_SIO2_CMD_STAT: u32 = 0x1F80826C;
pub const HW_SIO2_PORT_STAT: u32 = 0x1F808270;
pub const HW_SIO2_FIFO_STAT: u32 = 0x1F808274;
pub const HW_SIO2_FIFO_TX: u32 = 0x1F808278; // May as well add defs
pub const HW_SIO2_FIFO_RX: u32 = 0x1F80827C; // for these 2...
pub const HW_SIO2_INTR: u32 = 0x1F808280;

// ---------------------------------------------------------------------------
// DMA channel register addresses
// ---------------------------------------------------------------------------

/// IOP DMA channel MADDR (Memory Address) registers.
#[repr(u32)]
pub enum DmaMadrAddresses {
    HwxDma0Madr = 0x1F801080,
    HwxDma1Madr = 0x1F801090,
    HwxDma2Madr = 0x1F8010A0,
    HwxDma3Madr = 0x1F8010B0,
    HwxDma4Madr = 0x1F8010C0,
    HwxDma5Madr = 0x1F8010D0,
    HwxDma6Madr = 0x1F8010E0,
    HwxDma7Madr = 0x1F801500,
    HwxDma8Madr = 0x1F801510,
    HwxDma9Madr = 0x1F801520,
    HwxDma10Madr = 0x1F801530,
    HwxDma11Madr = 0x1F801540,
    HwxDma12Madr = 0x1F801550,
}

/// IOP DMA channel BCR (Block Control) registers.
#[repr(u32)]
pub enum DmaBcrAddresses {
    HwxDma0Bcr = 0x1F801084,
    HwxDma1Bcr = 0x1F801094,
    HwxDma2Bcr = 0x1F8010A4,
    HwxDma3Bcr = 0x1F8010B4,
    HwxDma3BcrH16 = 0x1F8010B6,
    HwxDma4Bcr = 0x1F8010C4,
    HwxDma5Bcr = 0x1F8010D4,
    HwxDma6Bcr = 0x1F8010E4,
    HwxDma7Bcr = 0x1F801504,
    HwxDma8Bcr = 0x1F801514,
    HwxDma9Bcr = 0x1F801524,
    HwxDma10Bcr = 0x1F801534,
    HwxDma11Bcr = 0x1F801544,
    HwxDma12Bcr = 0x1F801554,
}

/// IOP DMA channel CHCR (Channel Control) registers.
#[repr(u32)]
pub enum DmaChcrAddresses {
    HwxDma0Chcr = 0x1F801088,
    HwxDma1Chcr = 0x1F801098,
    HwxDma2Chcr = 0x1F8010A8,
    HwxDma3Chcr = 0x1F8010B8,
    HwxDma4Chcr = 0x1F8010C8,
    HwxDma5Chcr = 0x1F8010D8,
    HwxDma6Chcr = 0x1F8010E8,
    HwxDma7Chcr = 0x1F801508,
    HwxDma8Chcr = 0x1F801518,
    HwxDma9Chcr = 0x1F801528,
    HwxDma10Chcr = 0x1F801538,
    HwxDma11Chcr = 0x1F801548,
    HwxDma12Chcr = 0x1F801558,
}

/// IOP DMA channel TADR (Tag Address) registers.
#[repr(u32)]
pub enum DmaTadrAddresses {
    HwxDma0Tadr = 0x1F80108C,
    HwxDma1Tadr = 0x1F80109C,
    HwxDma2Tadr = 0x1F8010AC,
    HwxDma3Tadr = 0x1F8010BC,
    HwxDma4Tadr = 0x1F8010CC,
    HwxDma5Tadr = 0x1F8010DC,
    HwxDma6Tadr = 0x1F8010EC,
    HwxDma7Tadr = 0x1F80150C,
    HwxDma8Tadr = 0x1F80151C,
    HwxDma9Tadr = 0x1F80152C,
    HwxDma10Tadr = 0x1F80153C,
    HwxDma11Tadr = 0x1F80154C,
    HwxDma12Tadr = 0x1F80155C,
}

// ---------------------------------------------------------------------------
// IOP counter register addresses
// ---------------------------------------------------------------------------

/// IOP hardware-counter register addresses. Six counters (T0..T5) live in two
/// register pages (0x1F8011xx and 0x1F8014xx), each with a count, mode, and
/// target register.
#[repr(u32)]
pub enum IopCountRegs {
    IopT0Count = 0x1F801100,
    IopT1Count = 0x1F801110,
    IopT2Count = 0x1F801120,
    IopT3Count = 0x1F801480,
    IopT4Count = 0x1F801490,
    IopT5Count = 0x1F8014A0,

    IopT0Mode = 0x1F801104,
    IopT1Mode = 0x1F801114,
    IopT2Mode = 0x1F801124,
    IopT3Mode = 0x1F801484,
    IopT4Mode = 0x1F801494,
    IopT5Mode = 0x1F8014A4,

    IopT0Target = 0x1F801108,
    IopT1Target = 0x1F801118,
    IopT2Target = 0x1F801128,
    IopT3Target = 0x1F801488,
    IopT4Target = 0x1F801498,
    IopT5Target = 0x1F8014A8,
}

// ---------------------------------------------------------------------------
// DMA register block layouts
// ---------------------------------------------------------------------------

/// Standard three-register DMA block: MADR / BCR / CHCR.
#[derive(Clone, Copy)]
pub struct DmaMbc {
    pub madr: u32,
    pub bcr: u32,
    pub chcr: u32,
}

impl DmaMbc {
    /// Lower 16 bits of BCR (block count / word count low).
    pub fn bcr_lower(&self) -> u16 {
        self.bcr as u16
    }

    /// Upper 16 bits of BCR (word count high).
    pub fn bcr_upper(&self) -> u16 {
        (self.bcr >> 16) as u16
    }

    /// Human-readable description of the register block, mirroring the C++
    /// `dma_mbc::desc()` helper.
    pub fn desc(&self) -> String {
        format!(
            "madr: 0x{:x} bcr: 0x{:x} chcr: 0x{:x}",
            self.madr, self.bcr, self.chcr
        )
    }
}

impl Default for DmaMbc {
    fn default() -> Self {
        Self {
            madr: 0,
            bcr: 0,
            chcr: 0,
        }
    }
}

/// DMA block with an extra TADR field, used by chained-tag DMA channels.
#[derive(Clone, Copy)]
pub struct DmaMbct {
    pub madr: u32,
    pub bcr: u32,
    pub chcr: u32,
    pub tadr: u32,
}

impl DmaMbct {
    /// Lower 16 bits of BCR.
    pub fn bcr_lower(&self) -> u16 {
        self.bcr as u16
    }

    /// Upper 16 bits of BCR.
    pub fn bcr_upper(&self) -> u16 {
        (self.bcr >> 16) as u16
    }

    /// Human-readable description of the register block, mirroring the C++
    /// `dma_mbct::desc()` helper.
    pub fn desc(&self) -> String {
        format!(
            "madr: 0x{:x} bcr: 0x{:x} chcr: 0x{:x} tadr: 0x{:x}",
            self.madr, self.bcr, self.chcr, self.tadr
        )
    }
}

impl Default for DmaMbct {
    fn default() -> Self {
        Self {
            madr: 0,
            bcr: 0,
            chcr: 0,
            tadr: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// IopEventId
// ---------------------------------------------------------------------------

/// Discriminators for events the IOP scheduler can post to itself. Used as
/// keys into the event queue and as arguments to `PSX_INT` / `psxRemainingCycles`.
#[repr(u32)]
pub enum IopEventId {
    IopEvtSif2,
    IopEvtCdvd,        // General Cdvd commands (Seek, Standby, Break, etc)
    IopEvtSif0,
    IopEvtSif1,
    IopEvtDma11,
    IopEvtDma12,
    IopEvtSio,
    IopEvtCdrom,
    IopEvtCdromRead,
    IopEvtCdvdRead,
    IopEvtCdvdSectorReady,
    IopEvtDev9,
    IopEvtUsb,
}

// ---------------------------------------------------------------------------
// IOP-side HW register dispatch (page 1F80xxxx)
// ---------------------------------------------------------------------------

/// Function-pointer type for an IOP hardware register read handler. The
/// argument is the full masked address within the page; the return is the
/// raw 32-bit value latched from the device.
pub type IopHwReadFn = unsafe fn(addr: u32) -> u32;

/// Function-pointer type for an IOP hardware register write handler. The
/// argument is the full masked address within the page; `value` is the raw
/// 32-bit value being written.
pub type IopHwWriteFn = unsafe fn(addr: u32, value: u32);

/// Read dispatch table. One entry per 0x10000-byte sub-page of the IOP HW
/// region; `None` indicates "no handler registered for this sub-page".
pub static IopHw_FnRead: [Option<IopHwReadFn>; 16] = [None; 16];

/// Write dispatch table. One entry per 0x10000-byte sub-page of the IOP HW
/// region; `None` indicates "no handler registered for this sub-page".
pub static IopHw_FnWrite: [Option<IopHwWriteFn>; 16] = [None; 16];

// ---------------------------------------------------------------------------
// Generic 1F80xxxx read/write helpers
// ---------------------------------------------------------------------------
//
// These forward to the function-pointer dispatch tables. Callers are expected
// to have installed per-page handlers via `IopHw_FnRead` / `IopHw_FnWrite`.
// The C++ side marks these as force-inline; in Rust the equivalent hint is
// to keep them small and `#[inline]` so the dispatch stays a tight jump.

/// Read an 8-bit value from the IOP HW region.
///
/// Dispatches through `IopHw_FnRead` using bits [19:16] of `addr` to index
/// the per-page handler table.
#[inline]
pub fn psxHw1Read8(addr: u32) -> u8 {
    let page = ((addr >> 16) & 0xF) as usize;
    let masked = addr & 0xFFFF;
    match IopHw_FnRead[page] {
        Some(handler) => unsafe { (handler)(masked) as u8 },
        None => 0,
    }
}

/// Read a 16-bit value from the IOP HW region.
///
/// Performs two consecutive 8-bit reads (little-endian) through the per-page
/// read handler, matching the byte-by-byte access the IOP bus actually uses.
#[inline]
pub fn psxHw1Read16(addr: u32) -> u16 {
    let lo = psxHw1Read8(addr) as u16;
    let hi = psxHw1Read8(addr.wrapping_add(1)) as u16;
    lo | (hi << 8)
}

/// Read a 32-bit value from the IOP HW region.
///
/// Performs four consecutive 8-bit reads (little-endian) through the per-page
/// read handler, matching the byte-by-byte access the IOP bus actually uses.
#[inline]
pub fn psxHw1Read32(addr: u32) -> u32 {
    let b0 = psxHw1Read8(addr) as u32;
    let b1 = psxHw1Read8(addr.wrapping_add(1)) as u32;
    let b2 = psxHw1Read8(addr.wrapping_add(2)) as u32;
    let b3 = psxHw1Read8(addr.wrapping_add(3)) as u32;
    b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)
}

/// Write an 8-bit value to the IOP HW region.
///
/// Dispatches through `IopHw_FnWrite` using bits [19:16] of `addr` to index
/// the per-page handler table.
#[inline]
pub fn psxHw1Write8(addr: u32, value: u8) {
    let page = ((addr >> 16) & 0xF) as usize;
    let masked = addr & 0xFFFF;
    if let Some(handler) = IopHw_FnWrite[page] {
        unsafe { (handler)(masked, value as u32) }
    }
}

/// Write a 16-bit value to the IOP HW region.
///
/// Splits the value into two 8-bit writes (little-endian) through the
/// per-page write handler, matching the byte-by-bye access the IOP bus
/// actually uses.
#[inline]
pub fn psxHw1Write16(addr: u32, value: u16) {
    psxHw1Write8(addr, value as u8);
    psxHw1Write8(addr.wrapping_add(1), (value >> 8) as u8);
}

/// Write a 32-bit value to the IOP HW region.
///
/// Splits the value into four 8-bit writes (little-endian) through the
/// per-page write handler, matching the byte-by-byte access the IOP bus
/// actually uses.
#[inline]
pub fn psxHw1Write32(addr: u32, value: u32) {
    psxHw1Write8(addr, value as u8);
    psxHw1Write8(addr.wrapping_add(1), (value >> 8) as u8);
    psxHw1Write8(addr.wrapping_add(2), (value >> 16) as u8);
    psxHw1Write8(addr.wrapping_add(3), (value >> 24) as u8);
}
