// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
//! DMAC (DMA Controller) translation module.
//!
//! This module consolidates the C++ implementations found in `pcsx2/Dmac.cpp`
//! and `pcsx2/Dmac.h` into a single idiomatic Rust 2021 file. It exposes the
//! EE's DMA controller register state (10 channels plus the controller-level
//! DMAC register file), the PFIFO queue fields (CHCR / QWC / TADR per
//! channel), and the public `dmacInit` / `dmacReset` / `dmacUpdate` /
//! `dmacInterrupt` entry points. The module depends only on `std` and uses
//! `static mut` for the global state to mirror the C++ linkage.

// ---------------------------------------------------------------------------
// Enums mirrored from `Dmac.h`.
// ---------------------------------------------------------------------------

/// PCE (PCE) values for DMA chain tags. The PCSX2 chain-tag encoding puts a
/// 2-bit PCE field in the upper half of the tag that controls prefetch
/// behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum PceValue {
    /// No prefetch control.
    Nothing = 0,
    /// Reserved encoding.
    Reserved = 1,
    /// Prefetch disabled.
    Disabled = 2,
    /// Prefetch enabled.
    Enabled = 3,
}

/// DMA chain tag identifier.
///
/// The PS2 DMA chain tag's 3-bit `ID` field selects the transfer mode
/// (CNT, NEXT, REF, etc.). The list is ordered to match the C++ source for
/// direct numeric comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum TagId {
    /// Tag with `ID == 0` -- "REFE" (transfer and end) in the PS2 docs.
    Refe = 0,
    /// Transfer the QWC words following the tag.
    Cnt = 1,
    /// Transfer the QWC words following the tag and load the next tag
    /// from `ADDR`.
    Next = 2,
    /// Transfer the QWC words from the address in `ADDR`.
    Ref = 3,
    /// Same as `Ref` but with stall-control applied.
    Refs = 4,
    /// Transfer the QWC words following the tag and push the next tag's
    /// address onto the address stack.
    Call = 5,
    /// Transfer the QWC words following the tag and pop the next tag from
    /// the address stack.
    Ret = 6,
    /// Transfer the QWC words following the tag and end the chain.
    End = 7,
}

/// Memory FIFO drain channel (`MFD` field of `DMAC_CTRL`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum MfdType {
    NoMfd = 0,
    Reserved = 1,
    Vif1 = 2,
    Gif = 3,
}

/// Stall-control source channel (`STS` field of `DMAC_CTRL`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum StsType {
    NoSts = 0,
    Sif0 = 1,
    FromSpr = 2,
    FromIpu = 3,
}

/// Stall-control drain channel (`STD` field of `DMAC_CTRL`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum StdType {
    NoStd = 0,
    Vif1 = 1,
    Gif = 2,
    Sif1 = 3,
}

/// Logical transfer mode (`MOD` field of each channel's `CHCR`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum LogicalTransferMode {
    Normal = 0,
    Chain = 1,
    Interleave = 2,
    /// The PS2 hardware only encodes modes 0..2; the C++ source still
    /// leaves the `3` slot as a sentinel.
    Undefined = 3,
}

/// INTC interrupt lines. The C++ side uses these as indices into the
/// `INTC_STAT` / `INTC_MASK` packed bitfields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum IntcIrq {
    Gs = 0,
    Sbus = 1,
    VblankS = 2,
    VblankE = 3,
    Vif0 = 4,
    Vif1 = 5,
    Vu0 = 6,
    Vu1 = 7,
    Ipu = 8,
    Tim0 = 9,
    Tim1 = 10,
    Tim2 = 11,
    Tim3 = 12,
    Sfifo = 13,
    Vu0Wd = 14,
}

/// DMAC condition bits (`DMAC_STAT` / `DMAC_PCR` layout).
pub mod dmac_conditions {
    /// Stall interrupt condition.
    pub const DMAC_STAT_SIS: u32 = 1 << 13;
    /// MFIFO-empty interrupt condition.
    pub const DMAC_STAT_MEIS: u32 = 1 << 14;
    /// Bus-error interrupt condition.
    pub const DMAC_STAT_BEIS: u32 = 1 << 15;
    /// Stall mask bit.
    pub const DMAC_STAT_SIM: u32 = 1 << 29;
    /// MFIFO mask bit.
    pub const DMAC_STAT_MEIM: u32 = 1 << 30;
}

/// DMA interrupt and mask constants (`DMAInter` enum in the C++ source).
pub mod dma_inter {
    pub const BEISintr: u32 = 0x0000_8000;
    pub const VIF0intr: u32 = 0x0001_0001;
    pub const VIF1intr: u32 = 0x0002_0002;
    pub const GIFintr: u32 = 0x0004_0004;
    pub const IPU0intr: u32 = 0x0008_0008;
    pub const IPU1intr: u32 = 0x0010_0010;
    pub const SIF0intr: u32 = 0x0020_0020;
    pub const SIF1intr: u32 = 0x0040_0040;
    pub const SIF2intr: u32 = 0x0080_0080;
    pub const SPR0intr: u32 = 0x0100_0100;
    pub const SPR1intr: u32 = 0x0200_0200;
    pub const SISintr: u32 = 0x2000_2000;
    pub const MEISintr: u32 = 0x4000_4000;
}

// ---------------------------------------------------------------------------
// DMA tag / channel primitives.
// ---------------------------------------------------------------------------

/// A single DMA chain tag.
///
/// Mirrors the C++ `tDMA_TAG` union: a 32-bit word that is interpreted
/// either as `QWC`/`PCE`/`ID`/`IRQ` (the upper-half layout) or as `ADDR`
/// plus the `SPR` flag (the lower-half layout). The full 32-bit raw value
/// is always accessible through `raw`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DmaTag(pub u32);

impl DmaTag {
    /// Quadrant word count (low 16 bits).
    #[inline]
    pub fn qwc(self) -> u16 {
        (self.0 & 0xFFFF) as u16
    }

    /// Set the low 16-bit QWC field.
    #[inline]
    pub fn set_qwc(&mut self, v: u16) {
        self.0 = (self.0 & 0xFFFF_0000) | (v as u32);
    }

    /// `PCE` field (bits 26..28).
    #[inline]
    pub fn pce(self) -> u32 {
        (self.0 >> 26) & 0x3
    }

    /// Set the `PCE` field.
    #[inline]
    pub fn set_pce(&mut self, v: u32) {
        self.0 = (self.0 & !(0x3 << 26)) | ((v & 0x3) << 26);
    }

    /// 3-bit `ID` field.
    #[inline]
    pub fn id(self) -> u32 {
        (self.0 >> 28) & 0x7
    }

    /// Set the 3-bit `ID` field.
    #[inline]
    pub fn set_id(&mut self, v: u32) {
        self.0 = (self.0 & !(0x7 << 28)) | ((v & 0x7) << 28);
    }

    /// `IRQ` bit (bit 31).
    #[inline]
    pub fn irq(self) -> bool {
        (self.0 & (1 << 31)) != 0
    }

    /// Set the `IRQ` bit.
    #[inline]
    pub fn set_irq(&mut self, on: bool) {
        if on {
            self.0 |= 1 << 31;
        } else {
            self.0 &= !(1 << 31);
        }
    }

    /// 31-bit address field (low half of the tag's other interpretation).
    #[inline]
    pub fn addr(self) -> u32 {
        self.0 & 0x7FFF_FFFF
    }

    /// Set the 31-bit address field.
    #[inline]
    pub fn set_addr(&mut self, v: u32) {
        self.0 = (self.0 & !0x7FFF_FFFF) | (v & 0x7FFF_FFFF);
    }

    /// `SPR` bit (bit 31 of the address interpretation).
    #[inline]
    pub fn spr(self) -> bool {
        (self.0 & (1 << 31)) != 0
    }

    /// Set the `SPR` bit.
    #[inline]
    pub fn set_spr(&mut self, on: bool) {
        if on {
            self.0 |= 1 << 31;
        } else {
            self.0 &= !(1 << 31);
        }
    }

    /// Upper 16 bits (the high half of the tag's packed layout). This is
    /// the value copied into `CHCR.TAG` by the C++ `chcrTransfer` helper.
    #[inline]
    pub fn upper(self) -> u16 {
        (self.0 >> 16) as u16
    }

    /// Lower 16 bits of the raw value.
    #[inline]
    pub fn lower(self) -> u16 {
        self.0 as u16
    }

    /// Reset the tag back to zero.
    #[inline]
    pub fn reset(&mut self) {
        self.0 = 0;
    }

    /// Return a human-readable decoding of the tag.
    pub fn tag_to_str(&self) -> String {
        match self.id() {
            0 => format!("REFE {:08X}", self.0),
            1 => "CNT".to_string(),
            2 => format!("NEXT {:08X}", self.0),
            3 => format!("REF {:08X}", self.0),
            4 => format!("REFS {:08X}", self.0),
            5 => "CALL".to_string(),
            6 => "RET".to_string(),
            7 => "END".to_string(),
            _ => "????".to_string(),
        }
    }
}

impl Default for DmaTag {
    fn default() -> Self {
        DmaTag(0)
    }
}

// ---------------------------------------------------------------------------
// DMACh — per-channel register view.
// ---------------------------------------------------------------------------

/// Per-channel DMA register file.
///
/// Mirrors the C++ `DMACh` struct. The packed layout in the original C++
/// uses 32-bit CHCR/MADR/QWC/TADR/ASR0/ASR1/SADR registers interleaved
/// with padding words. The Rust translation uses a 128-bit per-channel
/// "chcr" slot (matching the PFIFO queue view) and a single 16-bit QWC,
/// 32-bit MADR, 32-bit TADR, etc. field for the rest.
#[derive(Debug, Clone, Copy)]
pub struct DMACh {
    /// Channel CHCR (and packed CHCR layout). The C++ union packs STR /
    /// MOD / TTE / TIE / TAG into a 32-bit word; the translation widens
    /// the value to a `u128` slot to keep the PFIFO-queue view aligned.
    pub chcr: u128,
    /// Channel MADR (memory address).
    pub madr: u32,
    /// Channel QWC (quadrant word count, low 16 bits valid).
    pub qwc: u16,
    /// Channel TADR (chain tag address).
    pub tadr: u32,
    /// Address-stack register 0.
    pub asr0: u32,
    /// Address-stack register 1.
    pub asr1: u32,
    /// Channel SADR (scratchpad address register, used by the SPR DMA
    /// channels).
    pub sadr: u32,
}

impl DMACh {
    /// Construct a zero-initialised channel.
    pub const fn new() -> Self {
        Self {
            chcr: 0,
            madr: 0,
            qwc: 0,
            tadr: 0,
            asr0: 0,
            asr1: 0,
            sadr: 0,
        }
    }

    /// Copy the upper 16 bits of the supplied tag into `CHCR.TAG`. Mirrors
    /// the C++ `chcrTransfer` helper.
    #[inline]
    pub fn chcr_transfer(&mut self, ptag: &[DmaTag; 1]) {
        let upper = (ptag[0].0 >> 16) as u16 as u128;
        // CHCR layout: bits 0..7 = control, bits 16..31 = TAG. Clear the
        // upper 16 bits and OR in the new TAG value.
        self.chcr = (self.chcr & 0x0000_0000_FFFF_FFFFu128) | (upper << 16);
    }

    /// Copy the QWC field of the supplied tag into the channel's `QWC`.
    /// Mirrors the C++ `qwcTransfer` helper.
    #[inline]
    pub fn qwc_transfer(&mut self, ptag: &[DmaTag; 1]) {
        self.qwc = ptag[0].qwc();
    }

    /// Set the `STR` (start) bit in the channel's CHCR register. Mirrors
    /// the C++ `chcr.STR = on;` bitfield assignment, where `STR` lives at
    /// bit 8 of the packed CHCR layout.
    #[inline]
    pub fn set_str(&mut self, on: bool) {
        const STR_MASK: u128 = 1u128 << 8;
        if on {
            self.chcr |= STR_MASK;
        } else {
            self.chcr &= !STR_MASK;
        }
    }

    /// Return the value of the `STR` (start) bit.
    #[inline]
    pub fn str(&self) -> bool {
        (self.chcr & (1u128 << 8)) != 0
    }

    /// Transfer a tag into the channel, validating the supplied pointer.
    /// Mirrors the C++ `DMACh::transfer(const char *s, tDMA_TAG* ptag)`.
    ///
    /// Returns `true` on success. If `ptag` is `None` this raises a bus
    /// error (mirroring the C++ `throwBusError(s)` call) and returns
    /// `false`.
    #[inline]
    pub fn transfer(&mut self, ptag: Option<&DmaTag>) -> bool {
        match ptag {
            None => {
                throw_bus_error();
                false
            }
            Some(tag) => {
                self.unsafe_transfer(tag);
                true
            }
        }
    }

    /// Transfer a tag into the channel without checking for a null tag.
    /// Mirrors the C++ `DMACh::unsafeTransfer(tDMA_TAG* ptag)`.
    #[inline]
    pub fn unsafe_transfer(&mut self, ptag: &DmaTag) {
        self.chcr_transfer(&[*ptag]);
        self.qwc_transfer(&[*ptag]);
    }

    /// Resolve a physical DMA address into the EE's memory map. Mirrors
    /// the C++ `DMACh::getAddr(u32 addr, u32 num, bool write)`.
    ///
    /// On failure (the address falls outside every recognised region) the
    /// function raises a bus error, marks the channel's completion bit in
    /// the DMAC status register, and clears the channel's `STR` bit
    /// (matching the C++ behaviour).
    pub fn get_addr(&mut self, addr: u32, num: u32, write: bool) -> Option<u32> {
        match dma_get_addr(addr, write) {
            Some(ptr) => Some(ptr),
            None => {
                throw_bus_error();
                set_dmac_stat(num);
                self.set_str(false);
                None
            }
        }
    }

    /// Resolve a physical address and return the resolved pointer.
    /// Mirrors the C++ `DMACh::DMAtransfer(u32 addr, u32 num)`.
    ///
    /// The C++ version then performs a `chcrTransfer` / `qwcTransfer`
    /// using the dereferenced tag. The Rust translation preserves the
    /// address-resolution portion and returns the resolved address; the
    /// surrounding emulator is responsible for reading the tag word and
    /// calling `unsafe_transfer` if it needs the CHCR/QWC updated.
    pub fn dma_transfer(&mut self, addr: u32, num: u32) -> Option<u32> {
        self.get_addr(addr, num, false)
    }

    /// Return the tag embedded in the lower 32 bits of the channel's
    /// CHCR. Mirrors the C++ `DMACh::dma_tag()` / `chcr.tag()`.
    #[inline]
    pub fn dma_tag(&self) -> DmaTag {
        DmaTag(self.chcr as u32)
    }

    /// Human-readable dump of the PFIFO queue entry. Mirrors the C++
    /// `DMACh::cmq_to_str() const`.
    pub fn cmq_to_str(&self) -> String {
        format!(
            "chcr = {:x}, madr = {:x}, qwc  = {:x}",
            self.chcr as u32,
            self.madr,
            self.qwc
        )
    }

    /// Human-readable dump including the channel's TADR. Mirrors the C++
    /// `DMACh::cmqt_to_str() const`.
    pub fn cmqt_to_str(&self) -> String {
        format!(
            "chcr = {:x}, madr = {:x}, qwc  = {:x}, tadr = {:x}",
            self.chcr as u32,
            self.madr,
            self.qwc,
            self.tadr
        )
    }
}

impl Default for DMACh {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// DMAC controller-level register view.
// ---------------------------------------------------------------------------

/// DMAC controller register file.
///
/// Mirrors the C++ `DMACregisters` struct (`ctrl`, `stat`, `pcr`, `sqwc`,
/// `rbsr`, `rbor`, `stadr`). The `pDmac` global below is built from this
/// type.
#[derive(Debug, Clone, Copy)]
pub struct DmacCtrlRegs {
    /// `DMAC_CTRL` register.
    pub ctrl: u32,
    /// `DMAC_STAT` register.
    pub stat: u32,
    /// `DMAC_PCR` register.
    pub pcr: u32,
    /// `DMAC_SQWC` register.
    pub sqwc: u32,
    /// `DMAC_RBSR` register.
    pub rbsr: u32,
    /// `DMAC_RBOR` register.
    pub rbor: u32,
    /// `DMAC_STADR` register.
    pub stadr: u32,
}

impl DmacCtrlRegs {
    /// Construct a zero-initialised register file.
    pub const fn new() -> Self {
        Self {
            ctrl: 0,
            stat: 0,
            pcr: 0,
            sqwc: 0,
            rbsr: 0,
            rbor: 0,
            stadr: 0,
        }
    }
}

impl Default for DmacCtrlRegs {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Global register state.
// ---------------------------------------------------------------------------

/// Number of DMA channels exposed by the EE's DMAC.
pub const NUM_DMA_CHANNELS: usize = 10;

/// DMAC register state.
///
/// Holds the PFIFO queue fields (CHCR / QWC / TADR per channel), the
/// controller-level register file, and the queued-DMA bit set used by
/// `StartQueuedDMA`. The `pDmac` global is the single instance of this
/// struct; the surrounding emulator code references it through the
/// `pub static pDmac` symbol declared at the bottom of this module.
#[derive(Debug, Clone, Copy)]
pub struct DmacRegisters {
    /// Per-channel CHCR. The C++ `tDMA_CHCR` union packs 32 bits of
    /// control plus a 16-bit `TAG` field; the Rust translation uses a
    /// `u128` per channel so the PFIFO queue view can be stored as a
    /// fixed-size array.
    pub chcr: [u128; NUM_DMA_CHANNELS],
    /// Per-channel QWC.
    pub qwc: [u16; NUM_DMA_CHANNELS],
    /// Per-channel TADR.
    pub tadr: [u32; NUM_DMA_CHANNELS],
    /// Controller-level register file.
    pub ctrl: DmacCtrlRegs,
    /// Queued-DMA bit set. One bit per channel plus the SIS / MEIS / BEIS
    /// slots, matching the C++ `tDMAC_QUEUE` union.
    pub queued: u16,
}

impl DmacRegisters {
    /// Construct a fully zero-initialised DMAC register file. The
    /// `pDmac` global below is built from this constructor.
    pub const fn new() -> Self {
        Self {
            chcr: [0u128; NUM_DMA_CHANNELS],
            qwc: [0u16; NUM_DMA_CHANNELS],
            tadr: [0u32; NUM_DMA_CHANNELS],
            ctrl: DmacCtrlRegs::new(),
            queued: 0,
        }
    }
}

impl Default for DmacRegisters {
    fn default() -> Self {
        Self::new()
    }
}

/// Global DMAC register state. Mirrors the C++ `dmacRegs` / `psHu32`
/// register file.
pub static mut pDmac: DmacRegisters = DmacRegisters::new();

// ---------------------------------------------------------------------------
// Init / reset / update / interrupt.
// ---------------------------------------------------------------------------

/// Initialise the DMAC. Mirrors the C++ `dmacInit()`.
///
/// In the original code this would also initialise the channel register
/// windows, set up the INTC masks, and wire the bus-error / mfifo-empty
/// handlers. The Rust translation resets `pDmac` to its power-on state and
/// leaves the surrounding wiring for the rest of the emulator to handle.
pub fn dmacInit() {
    dmacReset();
}

/// Reset the DMAC back to power-on defaults. Mirrors the C++
/// `dmacReset()`.
pub fn dmacReset() {
    unsafe {
        pDmac = DmacRegisters::new();
    }
}

/// Per-cycle DMAC update. Mirrors the C++ `dmacUpdate()`.
///
/// In the original code this is called once per EE cycle to advance the
/// DMAC scheduler. The Rust translation exposes the same entry point so
/// callers can be ported verbatim; the body is intentionally a stub
/// because the surrounding scheduler / interrupt dispatcher still lives
/// in the C++ emulator core.
pub fn dmacUpdate() {
    // Intentionally empty: the C++ `dmacUpdate` body lives next to the
    // interrupt dispatcher and the per-channel DMA work functions
    // (`dmaVIF0`, `dmaGIF`, etc.). Those modules are translated
    // separately.
}

/// Top-level DMAC interrupt entry point. Mirrors the C++ `hwDmacIrq(int n)`.
///
/// `n` selects the channel / interrupt source. The C++ implementation
/// sets the corresponding bit in `dmacRegs.stat` and clears the INTC
/// line; the Rust translation exposes the same entry point and records
/// the requested bit so the surrounding interrupt controller can pick
/// it up.
pub fn dmacInterrupt(n: u32) {
    unsafe {
        pDmac.ctrl.stat |= 1 << n;
    }
}

// ---------------------------------------------------------------------------
// Channel name / number helpers (mirroring `ChcrName` / `ChannelNumber`).
// ---------------------------------------------------------------------------

/// Return the channel number for a given `CHCR` register address. Mirrors
/// the C++ `ChannelNumber`. Returns `None` when the address does not
/// correspond to a known DMA channel.
pub fn channel_number(addr: u32) -> Option<usize> {
    // The C++ `ChannelNumber` switch covers `D0_CHCR` (0x9000-ish) through
    // `D9_CHCR`. The exact base addresses are baked into the C++ build
    // via HwInternal.h; the Rust translation accepts a precomputed
    // 0..=9 channel index packed into the low 4 bits of the address and
    // returns the channel number from there. Callers that need the
    // full address-to-channel mapping should compare against the EE
    // memory map.
    if addr <= 9 {
        Some(addr as usize)
    } else {
        None
    }
}

/// Return the canonical channel name for a given channel number. Mirrors
/// the C++ `ChcrName` switch.
pub fn chcr_name(channel: usize) -> &'static str {
    match channel {
        0 => "Vif 0",
        1 => "Vif 1",
        2 => "GIF",
        3 => "Ipu 0",
        4 => "Ipu 1",
        5 => "Sif 0",
        6 => "Sif 1",
        7 => "Sif 2",
        8 => "SPR 0",
        9 => "SPR 1",
        _ => "???",
    }
}

// ---------------------------------------------------------------------------
// Address-resolution helpers (mirroring `dmaGetAddr` / `SPRdmaGetAddr`).
// ---------------------------------------------------------------------------

/// Mask matching the C++ `Ps2MemSize::Scratch` constant used by the
/// address-resolution helpers (`Scratch - 1` is `0x3FF0`).
pub const SCRATCH_MASK: u32 = 0x3FF0;

/// Mask matching the C++ `Ps2MemSize::ExposedRam` constant. The EE's
/// exposed RAM is 32 MiB in the retail console; the C++ source uses
/// `Ps2MemSize::ExposedRam` directly. The Rust translation uses the same
/// literal `0x0200_0000` so the mask does not depend on the rest of the
/// emulator state.
pub const EXPOSED_RAM_MASK: u32 = 0x0200_0000;

/// Resolve a DMA physical address to a target address space, mirroring
/// the C++ `dmaGetAddr` helper.
///
/// The C++ implementation returns a `tDMA_TAG*` into the EE memory map.
/// The Rust translation performs the same address decoding and returns
/// the masked address (or `None` for unrecognised / unmapped regions);
/// the surrounding emulator is expected to translate the masked address
/// back to a real memory pointer.
pub fn dma_get_addr(addr: u32, write: bool) -> Option<u32> {
    let mut addr = addr;
    // SPR bit -> scratchpad.
    if DmaTag(addr).spr() {
        return Some(addr & SCRATCH_MASK);
    }
    // Mask off the high physical-address bits.
    addr &= 0x1FFF_FFF0;
    if addr < EXPOSED_RAM_MASK {
        Some(addr)
    } else if addr < 0x1000_0000 {
        // Zero-page alias (ZeroRead / ZeroWrite). The C++ code
        // distinguishes read vs. write; the Rust translation collapses
        // both paths into a single result.
        let _ = write;
        Some(0)
    } else if addr < 0x1000_4000 {
        // Secret scratchpad alias.
        Some(addr & SCRATCH_MASK)
    } else {
        None
    }
}

/// Resolve a DMA physical address that is allowed to access the scratchpad
/// directly, mirroring the C++ `SPRdmaGetAddr` helper.
pub fn spr_dma_get_addr(addr: u32, _write: bool) -> Option<u32> {
    let mut addr = addr;
    if (addr & 0x7000_0000) == 0x7000_0000 {
        return Some(addr & SCRATCH_MASK);
    }
    addr &= 0x1FFF_FFF0;
    if addr < EXPOSED_RAM_MASK {
        Some(addr)
    } else if addr < 0x1000_0000 {
        Some(0)
    } else if addr >= 0x1100_0000 && addr < 0x1101_0000 {
        // VU memory region. The C++ code dispatches between VU0 / VU1
        // / VU0 micro / VU1 micro based on the address range. The Rust
        // translation preserves the in-range check and returns the
        // masked address; the surrounding VU module is responsible for
        // the actual sub-region decode.
        Some(addr)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Bus-error helper (mirroring `throwBusError` / `setDmacStat`).
// ---------------------------------------------------------------------------

/// Mark `BEIS` in the DMAC status register. Mirrors the C++
/// `throwBusError` helper.
pub fn throw_bus_error() {
    unsafe {
        pDmac.ctrl.stat |= dmac_conditions::DMAC_STAT_BEIS;
    }
}

/// Set the channel-completion bit for channel `num` in the DMAC status
/// register. Mirrors the C++ `setDmacStat(num)` helper.
pub fn set_dmac_stat(num: u32) {
    unsafe {
        pDmac.ctrl.stat |= 1 << num;
    }
}
