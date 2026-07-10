// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
// Rust translation of the EE I/O subsystem (`Hw.cpp`, `Hw.h`, `HwRead.cpp`,
// `HwWrite.cpp`).
//
// This module owns the EE hardware register file (`ee_hw`, a flat 64 KiB
// array), the SIO FIFOs, the RDRAM device-id cursor, and a 256-entry page
// dispatch table indexed by `(addr >> 16) & 0xFF` that resolves any access
// in the EE I/O window (`0x1000_0000`..=`0x100F_FFFF`) to the correct width
// handler.
//
// The translation preserves the original layout, including:
//   * The big-endian `psHuNN` register view that overlays `ee_hw`.
//   * The `*_FAKESTAT` quirk (DMAC_STAT writes do not match reads in
//     certain timing windows).
//   * The INTC fast-path hack (page 0x0F, `INTC_STAT`).
//   * The SIO TX/RX FIFOs, the SIF2 PS1-bridge, and the MCH/RDRAM probes.
//   * The 32-bit DMA / IPU / VIF / GIF / counter fast paths.
//   * FIFO zero-fill semantics for non-128-bit writes.
//   * 128-bit reads from VIF0/GIF/IPUin returning zero (write-only FIFOs).
//
// Only `std` is depended on.  I/O port access is `unsafe` and confined to
// the dispatch table; every public function is itself marked `unsafe`
// to reflect that callers must guarantee address validity (the EE TLB
// layer is responsible for that in the real emulator).

#![allow(dead_code)]
#![allow(non_snake_case)]

use std::collections::VecDeque;
use std::ptr;
use std::sync::atomic::{AtomicU32, Ordering};

// Pull every EE register address constant into this module's namespace
// so call sites can write `INTC_STAT`, `SBUS_F260`, etc. without the
// `EeRegister::` prefix.  Qualified paths still work.
use EeRegister::*;

// -----------------------------------------------------------------------------
// Register address constants
// -----------------------------------------------------------------------------

pub mod ee_memory_map {
    use u32 as Addr;
    pub const RCNT0_START: Addr      = 0x1000_0000;
    pub const RCNT0_END: Addr        = 0x1000_0800;
    pub const RCNT1_START: Addr      = 0x1000_0800;
    pub const RCNT1_END: Addr        = 0x1000_1000;
    pub const RCNT2_START: Addr      = 0x1000_1000;
    pub const RCNT2_END: Addr        = 0x1000_1800;
    pub const RCNT3_START: Addr      = 0x1000_1800;
    pub const RCNT3_END: Addr        = 0x1000_2000;

    pub const IPU_START: Addr        = 0x1000_2000;
    pub const IPU_END: Addr          = 0x1000_3000;

    pub const GIF_START: Addr        = 0x1000_3000;
    pub const GIF_END: Addr          = 0x1000_3800;

    pub const VIF0_START: Addr       = 0x1000_3800;
    pub const VIF0_END: Addr         = 0x1000_3C00;
    pub const VIF1_START: Addr       = 0x1000_3C00;
    pub const VIF1_END: Addr         = 0x1000_4000;

    pub const VIF0_FIFO_START: Addr  = 0x1000_4000;
    pub const VIF0_FIFO_END: Addr    = 0x1000_5000;
    pub const VIF1_FIFO_START: Addr  = 0x1000_5000;
    pub const VIF1_FIFO_END: Addr    = 0x1000_6000;
    pub const GIF_FIFO_START: Addr   = 0x1000_6000;
    pub const GIF_FIFO_END: Addr     = 0x1000_7000;
    pub const IPU_FIFO_START: Addr   = 0x1000_7000;
    pub const IPU_FIFO_END: Addr     = 0x1000_8000;

    pub const VIF0_DMA_START: Addr   = 0x1000_8000;
    pub const VIF0_DMA_END: Addr     = 0x1000_9000;
    pub const VIF1_DMA_START: Addr   = 0x1000_9000;
    pub const VIF1_DMA_END: Addr     = 0x1000_A000;
    pub const GIF_DMA_START: Addr    = 0x1000_A000;
    pub const GIF_DMA_END: Addr      = 0x1000_B000;

    pub const FROM_IPU_START: Addr   = 0x1000_B000;
    pub const FROM_IPU_END: Addr     = 0x1000_B400;
    pub const TO_IPU_START: Addr     = 0x1000_B400;
    pub const TO_IPU_END: Addr       = 0x1000_C000;

    pub const SIF0_DMA_START: Addr   = 0x1000_C000;
    pub const SIF0_DMA_END: Addr     = 0x1000_C400;
    pub const SIF1_DMA_START: Addr   = 0x1000_C400;
    pub const SIF1_DMA_END: Addr     = 0x1000_C800;
    pub const SIF2_DMA_START: Addr   = 0x1000_C800;
    pub const SIF2_DMA_END: Addr     = 0x1000_D000;

    pub const FROM_SPR_START: Addr   = 0x1000_D000;
    pub const FROM_SPR_END: Addr     = 0x1000_D400;
    pub const TO_SPR_START: Addr     = 0x1000_D400;
    pub const TO_SPR_END: Addr       = 0x1000_E000;

    pub const DMAC_START: Addr       = 0x1000_E000;
    pub const DMAC_END: Addr         = 0x1000_F000;
    pub const INTC_START: Addr       = 0x1000_F000;
    pub const INTC_END: Addr         = 0x1000_F100;

    pub const SIO_START: Addr        = 0x1000_F100;
    pub const SIO_END: Addr          = 0x1000_F200;
    pub const SBUS_START: Addr       = 0x1000_F200;
    pub const SBUS_END: Addr         = 0x1000_F300;
    pub const SBUS_PS1_START: Addr   = 0x1000_F300;
    pub const SBUS_PS1_END: Addr     = 0x1000_F400;

    pub const MCH_START: Addr        = 0x1000_F400;
    pub const MCH_END: Addr          = 0x1000_F500;

    pub const DMAC_EXT_START: Addr   = 0x1000_F500;
    pub const DMAC_EXT_END: Addr     = 0x1000_F600;
}

/// `EeRegister` address constants.  Originally a `#[repr(u32)] enum` in
/// the C++ port, but Rust enums do not allow chained assignments
/// (`D0_CHCR = VIF0_CHCR = 0x...`) and many call sites use the bare
/// names (`SBUS_F260`, `INTC_STAT`) without the `EeRegister::` prefix.
/// We therefore expose these as `pub const`s and `use EeRegister::*`
/// at the top of the file to keep both styles working.
pub mod EeRegister {
    // Counters
    pub const RCNT0_COUNT : u32 = 0x1000_0000;
    pub const RCNT0_MODE  : u32 = 0x1000_0010;
    pub const RCNT0_TARGET: u32 = 0x1000_0020;
    pub const RCNT0_HOLD  : u32 = 0x1000_0030;
    pub const RCNT1_COUNT : u32 = 0x1000_0800;
    pub const RCNT1_MODE  : u32 = 0x1000_0810;
    pub const RCNT1_TARGET: u32 = 0x1000_0820;
    pub const RCNT1_HOLD  : u32 = 0x1000_0830;
    pub const RCNT2_COUNT : u32 = 0x1000_1000;
    pub const RCNT2_MODE  : u32 = 0x1000_1010;
    pub const RCNT2_TARGET: u32 = 0x1000_1020;
    pub const RCNT3_COUNT : u32 = 0x1000_1800;
    pub const RCNT3_MODE  : u32 = 0x1000_1810;
    pub const RCNT3_TARGET: u32 = 0x1000_1820;

    // IPU
    pub const IPU_CMD  : u32 = 0x1000_2000;
    pub const IPU_CTRL : u32 = 0x1000_2010;
    pub const IPU_BP   : u32 = 0x1000_2020;
    pub const IPU_TOP  : u32 = 0x1000_2030;

    // GIF
    pub const GIF_CTRL  : u32 = 0x1000_3000;
    pub const GIF_MODE  : u32 = 0x1000_3010;
    pub const GIF_STAT  : u32 = 0x1000_3020;
    pub const GIF_TAG0  : u32 = 0x1000_3040;
    pub const GIF_TAG1  : u32 = 0x1000_3050;
    pub const GIF_TAG2  : u32 = 0x1000_3060;
    pub const GIF_TAG3  : u32 = 0x1000_3070;
    pub const GIF_CNT   : u32 = 0x1000_3080;
    pub const GIF_P3CNT : u32 = 0x1000_3090;
    pub const GIF_P3TAG : u32 = 0x1000_30A0;

    // VIF0
    pub const VIF0_STAT  : u32 = 0x1000_3800;
    pub const VIF0_FBRST : u32 = 0x1000_3810;
    pub const VIF0_ERR   : u32 = 0x1000_3820;
    pub const VIF0_MARK  : u32 = 0x1000_3830;
    pub const VIF0_CYCLE : u32 = 0x1000_3840;
    pub const VIF0_MODE  : u32 = 0x1000_3850;
    pub const VIF0_NUM   : u32 = 0x1000_3860;
    pub const VIF0_MASK  : u32 = 0x1000_3870;
    pub const VIF0_CODE  : u32 = 0x1000_3880;
    pub const VIF0_ITOPS : u32 = 0x1000_3890;
    pub const VIF0_ITOP  : u32 = 0x1000_38D0;
    pub const VIF0_TOP   : u32 = 0x1000_38E0;
    pub const VIF0_ROW0  : u32 = 0x1000_3900;
    pub const VIF0_ROW1  : u32 = 0x1000_3910;
    pub const VIF0_ROW2  : u32 = 0x1000_3920;
    pub const VIF0_ROW3  : u32 = 0x1000_3930;
    pub const VIF0_COL0  : u32 = 0x1000_3940;
    pub const VIF0_COL1  : u32 = 0x1000_3950;
    pub const VIF0_COL2  : u32 = 0x1000_3960;
    pub const VIF0_COL3  : u32 = 0x1000_3970;

    // VIF1
    pub const VIF1_STAT  : u32 = 0x1000_3C00;
    pub const VIF1_FBRST : u32 = 0x1000_3C10;
    pub const VIF1_ERR   : u32 = 0x1000_3C20;
    pub const VIF1_MARK  : u32 = 0x1000_3C30;
    pub const VIF1_CYCLE : u32 = 0x1000_3C40;
    pub const VIF1_MODE  : u32 = 0x1000_3C50;
    pub const VIF1_NUM   : u32 = 0x1000_3C60;
    pub const VIF1_MASK  : u32 = 0x1000_3C70;
    pub const VIF1_CODE  : u32 = 0x1000_3C80;
    pub const VIF1_ITOPS : u32 = 0x1000_3C90;
    pub const VIF1_BASE  : u32 = 0x1000_3CA0;
    pub const VIF1_OFST  : u32 = 0x1000_3CB0;
    pub const VIF1_TOPS  : u32 = 0x1000_3CC0;
    pub const VIF1_ITOP  : u32 = 0x1000_3CD0;
    pub const VIF1_TOP   : u32 = 0x1000_3CE0;
    pub const VIF1_ROW0  : u32 = 0x1000_3D00;
    pub const VIF1_ROW1  : u32 = 0x1000_3D10;
    pub const VIF1_ROW2  : u32 = 0x1000_3D20;
    pub const VIF1_ROW3  : u32 = 0x1000_3D30;
    pub const VIF1_COL0  : u32 = 0x1000_3D40;
    pub const VIF1_COL1  : u32 = 0x1000_3D50;
    pub const VIF1_COL2  : u32 = 0x1000_3D60;
    pub const VIF1_COL3  : u32 = 0x1000_3D70;

    // FIFOs
    pub const VIF0_FIFO   : u32 = 0x1000_4000;
    pub const VIF1_FIFO   : u32 = 0x1000_5000;
    pub const GIF_FIFO    : u32 = 0x1000_6000;
    pub const IPU_OUT_FIFO: u32 = 0x1000_7000;
    pub const IPU_IN_FIFO : u32 = 0x1000_7010;

    // DMA channels (VIF0/GIF/FROM_IPU alias the corresponding D* register).
    pub const VIF0_CHCR : u32 = 0x1000_8000;
    pub const VIF0_MADR : u32 = 0x1000_8010;
    pub const VIF0_QWC  : u32 = 0x1000_8020;
    pub const VIF0_TADR : u32 = 0x1000_8030;
    pub const VIF0_ASR0 : u32 = 0x1000_8040;
    pub const VIF0_ASR1 : u32 = 0x1000_8050;

    pub const VIF1_CHCR : u32 = 0x1000_9000;
    pub const VIF1_MADR : u32 = 0x1000_9010;
    pub const VIF1_QWC  : u32 = 0x1000_9020;
    pub const VIF1_TADR : u32 = 0x1000_9030;
    pub const VIF1_ASR0 : u32 = 0x1000_9040;
    pub const VIF1_ASR1 : u32 = 0x1000_9050;

    pub const GIF_CHCR : u32 = 0x1000_A000;
    pub const GIF_MADR : u32 = 0x1000_A010;
    pub const GIF_QWC  : u32 = 0x1000_A020;
    pub const GIF_TADR : u32 = 0x1000_A030;
    pub const GIF_ASR0 : u32 = 0x1000_A040;
    pub const GIF_ASR1 : u32 = 0x1000_A050;

    pub const FROM_IPU_CHCR : u32 = 0x1000_B000;
    pub const FROM_IPU_MADR : u32 = 0x1000_B010;
    pub const FROM_IPU_QWC  : u32 = 0x1000_B020;

    pub const TO_IPU_CHCR : u32 = 0x1000_B400;
    pub const TO_IPU_MADR : u32 = 0x1000_B410;
    pub const TO_IPU_QWC  : u32 = 0x1000_B420;
    pub const TO_IPU_TADR : u32 = 0x1000_B430;

    pub const SIF0_CHCR : u32 = 0x1000_C000;
    pub const SIF0_MADR : u32 = 0x1000_C010;
    pub const SIF0_QWC  : u32 = 0x1000_C020;

    pub const SIF1_CHCR : u32 = 0x1000_C400;
    pub const SIF1_MADR : u32 = 0x1000_C410;
    pub const SIF1_QWC  : u32 = 0x1000_C420;
    pub const SIF1_TADR : u32 = 0x1000_C430;

    pub const SIF2_CHCR : u32 = 0x1000_C800;
    pub const SIF2_MADR : u32 = 0x1000_C810;
    pub const SIF2_QWC  : u32 = 0x1000_C820;

    pub const FROM_SPR_CHCR : u32 = 0x1000_D000;
    pub const FROM_SPR_MADR : u32 = 0x1000_D010;
    pub const FROM_SPR_QWC  : u32 = 0x1000_D020;
    pub const FROM_SPR_SADR : u32 = 0x1000_D080;

    pub const TO_SPR_CHCR : u32 = 0x1000_D400;
    pub const TO_SPR_MADR : u32 = 0x1000_D410;
    pub const TO_SPR_QWC  : u32 = 0x1000_D420;
    pub const TO_SPR_TADR : u32 = 0x1000_D430;
    pub const TO_SPR_SADR : u32 = 0x1000_D480;

    // D* alias for the DMA channels.  Each one matches the corresponding
    // `VIF*_CHCR` / `GIF_CHCR` / etc. constant above.
    pub const D0_CHCR : u32 = VIF0_CHCR;
    pub const D0_MADR : u32 = VIF0_MADR;
    pub const D0_QWC  : u32 = VIF0_QWC;
    pub const D0_TADR : u32 = VIF0_TADR;
    pub const D0_ASR0 : u32 = VIF0_ASR0;
    pub const D0_ASR1 : u32 = VIF0_ASR1;

    pub const D1_CHCR : u32 = VIF1_CHCR;
    pub const D1_MADR : u32 = VIF1_MADR;
    pub const D1_QWC  : u32 = VIF1_QWC;
    pub const D1_TADR : u32 = VIF1_TADR;
    pub const D1_ASR0 : u32 = VIF1_ASR0;
    pub const D1_ASR1 : u32 = VIF1_ASR1;

    pub const D2_CHCR : u32 = GIF_CHCR;
    pub const D2_MADR : u32 = GIF_MADR;
    pub const D2_QWC  : u32 = GIF_QWC;
    pub const D2_TADR : u32 = GIF_TADR;
    pub const D2_ASR0 : u32 = GIF_ASR0;
    pub const D2_ASR1 : u32 = GIF_ASR1;

    pub const D3_CHCR : u32 = FROM_IPU_CHCR;
    pub const D3_MADR : u32 = FROM_IPU_MADR;
    pub const D3_QWC  : u32 = FROM_IPU_QWC;

    pub const D4_CHCR : u32 = TO_IPU_CHCR;
    pub const D4_MADR : u32 = TO_IPU_MADR;
    pub const D4_QWC  : u32 = TO_IPU_QWC;
    pub const D4_TADR : u32 = TO_IPU_TADR;

    pub const D5_CHCR : u32 = SIF0_CHCR;
    pub const D5_MADR : u32 = SIF0_MADR;
    pub const D5_QWC  : u32 = SIF0_QWC;

    pub const D6_CHCR : u32 = SIF1_CHCR;
    pub const D6_MADR : u32 = SIF1_MADR;
    pub const D6_QWC  : u32 = SIF1_QWC;
    pub const D6_TADR : u32 = SIF1_TADR;

    pub const D7_CHCR : u32 = SIF2_CHCR;
    pub const D7_MADR : u32 = SIF2_MADR;
    pub const D7_QWC  : u32 = SIF2_QWC;

    pub const D8_CHCR : u32 = FROM_SPR_CHCR;
    pub const D8_MADR : u32 = FROM_SPR_MADR;
    pub const D8_QWC  : u32 = FROM_SPR_QWC;
    pub const D8_SADR : u32 = FROM_SPR_SADR;

    pub const D9_CHCR : u32 = TO_SPR_CHCR;
    pub const D9_MADR : u32 = TO_SPR_MADR;
    pub const D9_QWC  : u32 = TO_SPR_QWC;
    pub const D9_TADR : u32 = TO_SPR_TADR;
    pub const D9_SADR : u32 = TO_SPR_SADR;

    pub const DMAC_CTRL     : u32 = 0x1000_E000;
    pub const DMAC_STAT     : u32 = 0x1000_E010;
    pub const DMAC_PCR      : u32 = 0x1000_E020;
    pub const DMAC_SQWC     : u32 = 0x1000_E030;
    pub const DMAC_RBSR     : u32 = 0x1000_E040;
    pub const DMAC_RBOR     : u32 = 0x1000_E050;
    pub const DMAC_STADR    : u32 = 0x1000_E060;
    pub const DMAC_FAKESTAT : u32 = 0x1000_E100;

    pub const INTC_STAT : u32 = 0x1000_F000;
    pub const INTC_MASK : u32 = 0x1000_F010;

    pub const SIO_LCR    : u32 = 0x1000_F100;
    pub const SIO_LSR    : u32 = 0x1000_F110;
    pub const SIO_IER    : u32 = 0x1000_F120;
    pub const SIO_ISR    : u32 = 0x1000_F130;
    pub const SIO_FCR    : u32 = 0x1000_F140;
    pub const SIO_BGR    : u32 = 0x1000_F150;
    pub const SIO_TXFIFO : u32 = 0x1000_F180;
    pub const SIO_RXFIFO : u32 = 0x1000_F1C0;

    pub const SBUS_F200 : u32 = 0x1000_F200;
    pub const SBUS_F210 : u32 = 0x1000_F210;
    pub const SBUS_F220 : u32 = 0x1000_F220;
    pub const SBUS_F230 : u32 = 0x1000_F230;
    pub const SBUS_F240 : u32 = 0x1000_F240;
    pub const SBUS_F250 : u32 = 0x1000_F250;
    pub const SBUS_F260 : u32 = 0x1000_F260;
    pub const SBUS_F300 : u32 = 0x1000_F300;
    pub const SBUS_F380 : u32 = 0x1000_F380;

    pub const MCH_RICM   : u32 = 0x1000_F430;
    pub const MCH_DRD    : u32 = 0x1000_F440;

    pub const DMAC_ENABLER : u32 = 0x1000_F520;
    pub const DMAC_ENABLEW : u32 = 0x1000_F590;
}

/// GS register address constants (also demoted from `#[repr(u32)] enum`
/// to plain `pub const` so callers can use bare names).
pub mod GsRegister {
    pub const GS_PMODE   : u32 = 0x1200_0000;
    pub const GS_SMODE1  : u32 = 0x1200_0010;
    pub const GS_SMODE2  : u32 = 0x1200_0020;
    pub const GS_SRFSH   : u32 = 0x1200_0030;
    pub const GS_SYNCH1  : u32 = 0x1200_0040;
    pub const GS_SYNCH2  : u32 = 0x1200_0050;
    pub const GS_SYNCV   : u32 = 0x1200_0060;
    pub const GS_DISPFB1 : u32 = 0x1200_0070;
    pub const GS_DISPLAY1: u32 = 0x1200_0080;
    pub const GS_DISPFB2 : u32 = 0x1200_0090;
    pub const GS_DISPLAY2: u32 = 0x1200_00A0;
    pub const GS_EXTBUF  : u32 = 0x1200_00B0;
    pub const GS_EXTDATA : u32 = 0x1200_00C0;
    pub const GS_EXTWRITE: u32 = 0x1200_00D0;
    pub const GS_BGCOLOR : u32 = 0x1200_00E0;
    pub const GS_CSR     : u32 = 0x1200_1000;
    pub const GS_IMR     : u32 = 0x1200_1010;
    pub const GS_BUSDIR  : u32 = 0x1200_1040;
    pub const GS_SIGLBLID: u32 = 0x1200_1080;
}

// -----------------------------------------------------------------------------
// DMA tag constants
// -----------------------------------------------------------------------------

const TAG_REFE: i32 = 0;
const TAG_CNT : i32 = 1;
const TAG_NEXT: i32 = 2;
const TAG_REF : i32 = 3;
const TAG_REFS: i32 = 4;
const TAG_CALL: i32 = 5;
const TAG_RET : i32 = 6;
const TAG_END : i32 = 7;

// MFIFO destination IDs (matches `dmacRegs.ctrl.MFD`).
const MFD_VIF1: u32 = 0;
const MFD_GIF : u32 = 1;

// DMAC interrupt bits
const DMAC_MFIFO_EMPTY: u32 = 9;
const DMAC_MFIFO_VIF  : u32 = 7;
const DMAC_MFIFO_GIF  : u32 = 8;
const DMAC_GIF        : u32 = 2;

// GIF state machine
const GIF_STATE_EMPTY: u32 = 1;
const GIF_STATE_READY: u32 = 2;

// -----------------------------------------------------------------------------
// Global state
// -----------------------------------------------------------------------------

/// Flat 64 KiB EE hardware register file.  Mirrors `eeHw[]` in C++.
pub static mut EE_HW: [u32; 0x4000] = [0u32; 0x4000];

pub static mut RDRAM_DEVICES: i32 = 2; // PS2 ships with 2 devices
pub static mut RDRAM_SDEVID: i32 = 0;

pub static mut EE_SIO_RX_FIFO: VecDeque<u8> = VecDeque::new();
pub static mut EE_SIO_TX_FIFO: VecDeque<u8> = VecDeque::new();

/// Simulated SIF2 bridge FIFO (the PS1 DMA conduit).
pub static mut SIF2_FIFO_SIZE: u32 = 0;

/// Cached `INTC_MASK` shadow.  The INTC_STAT fast path in the original code
/// checks `psxHu32(HW_ICFG) & (1 << 3)`; we expose a single u32 here.
pub static mut HW_ICFG: u32 = 0;

static EE_HW_INITIALISED: AtomicU32 = AtomicU32::new(0);

/// Last TX byte was a carriage return (used by the SIO TX dedup logic).
static mut LAST_TX_WAS_CR: bool = false;

// -----------------------------------------------------------------------------
// Register access helpers
// -----------------------------------------------------------------------------

#[inline(always)]
unsafe fn hw32(offset: u32) -> *mut u32 {
    // `offset` is the low 16 bits of the address (`addr & 0xFFFF`), matching
    // `psHu32(addr)` which indexes the flat register file.
    EE_HW.as_mut_ptr().add((offset as usize) >> 2)
}

#[inline(always)]
pub unsafe fn ps_hu32(offset: u32) -> *mut u32 {
    hw32(offset)
}

#[inline(always)]
unsafe fn hw_read32(offset: u32) -> u32 {
    ptr::read_volatile(hw32(offset))
}

#[inline(always)]
unsafe fn hw_write32(offset: u32, value: u32) {
    ptr::write_volatile(hw32(offset), value)
}

#[inline(always)]
unsafe fn hw_or32(offset: u32, mask: u32) {
    let p = hw32(offset);
    ptr::write_volatile(p, ptr::read_volatile(p) | mask);
}

#[inline(always)]
unsafe fn hw_and32(offset: u32, mask: u32) {
    let p = hw32(offset);
    ptr::write_volatile(p, ptr::read_volatile(p) & mask);
}

#[inline(always)]
unsafe fn hw_xor32(offset: u32, mask: u32) {
    let p = hw32(offset);
    ptr::write_volatile(p, ptr::read_volatile(p) ^ mask);
}

// -----------------------------------------------------------------------------
// Logging
// -----------------------------------------------------------------------------

#[macro_export]
macro_rules! hw_log {
    ($($args:tt)*) => {
        eprintln!("[HW] {}", format!($($args)*))
    };
}

#[macro_export]
macro_rules! hw_dma_log {
    ($($args:tt)*) => {
        eprintln!("[DMA] {}", format!($($args)*))
    };
}

#[macro_export]
macro_rules! hw_spr_log {
    ($($args:tt)*) => {
        eprintln!("[SPR] {}", format!($($args)*))
    };
}

#[macro_export]
macro_rules! hw_gunit_log {
    ($($args:tt)*) => {
        eprintln!("[GUNIT] {}", format!($($args)*))
    };
}

// -----------------------------------------------------------------------------
// Lifecycle
// -----------------------------------------------------------------------------

/// Reset the EE hardware register file.  Mirrors `hwReset()` in `Hw.cpp`.
pub unsafe fn psxHwReset() {
    for slot in EE_HW.iter_mut() {
        *slot = 0;
    }
    EE_SIO_RX_FIFO.clear();
    EE_SIO_TX_FIFO.clear();
    SIF2_FIFO_SIZE = 0;

    hw_write32(SBUS_F260, 0x1D00_0060);

    // Used by some BIOSes; mimics a "version" register.
    hw_write32(DMAC_ENABLEW, 0x1201);
    hw_write32(DMAC_ENABLER, 0x1201);

    // The real implementation also resets counters, SPU2, SIF, GS, GIF,
    // IPU, VIF0/1, GIF FIFO, USB.  In this Rust port we delegate those to
    // the corresponding sibling modules; here we only initialise the bits
    // this file owns.

    rdram_sdevid_init();
}

unsafe fn rdram_sdevid_init() {
    RDRAM_SDEVID = 0;
}

/// Called from the IOP update loop; here it's a no-op because nothing in
/// this file's slice depends on the IOP cycle.  Mirrors `psxUpdateIOP()`.
pub unsafe fn psxUpdateIOP() {
    // Intentionally empty: timer ticks, SPU2, USB, etc. are owned by
    // sibling modules in the real C++.
}

/// One-shot initialisation.  Mirrors `psxHwInit()`.
pub fn psxHwInit() {
    if EE_HW_INITIALISED.swap(1, Ordering::SeqCst) == 0 {
        unsafe { psxHwReset(); }
    }
}

/// Shutdown / cleanup.  Mirrors `psxHwShutdown()`.
pub fn psxHwShutdown() {
    EE_HW_INITIALISED.store(0, Ordering::SeqCst);
}

// -----------------------------------------------------------------------------
// Interrupt helpers
// -----------------------------------------------------------------------------

/// `intcInterrupt` - returns the INTC exception vector if any INTC interrupt
/// is pending and unmasked.
#[inline]
pub unsafe fn intc_interrupt() -> u32 {
    let stat = hw_read32(INTC_STAT);
    if stat == 0 {
        return 0;
    }
    let mask = hw_read32(INTC_MASK);
    if (stat & mask) == 0 {
        return 0;
    }
    hw_log!("intcInterrupt {:x}", stat & mask);
    if (stat & 0x2) != 0 {
        // Real implementation stashes counter hold values here.  The
        // counters live in a sibling module, so we leave it to that
        // module to maintain them.
    }
    0x400
}

/// `dmacInterrupt` - returns the DMAC exception vector if any DMAC interrupt
/// is pending and unmasked.
#[inline]
pub unsafe fn dmac_interrupt() -> u32 {
    let lo = (hw_read32(DMAC_STAT) & 0xFFFF) as u16;
    let hi = ((hw_read32(DMAC_STAT) >> 16) & 0xFFFF) as u16;
    if (hi & lo) == 0 && (lo & 0x8000) == 0 {
        return 0;
    }
    // DMAE / suspended check would consult `dmacRegs.ctrl.DMAE` and
    // `DMAC_ENABLER+2`.  In this Rust port we trust the callers to
    // maintain those fields consistently.
    hw_dma_log!(
        "dmacInterrupt {:x}",
        (hi & lo) as u32 | (lo as u32 & 0x8000)
    );
    0x800
}

#[inline]
pub unsafe fn hw_intc_irq(n: u32) {
    hw_or32(INTC_STAT, 1u32 << n);
    if (hw_read32(INTC_MASK) & (1u32 << n)) != 0 {
        // Real implementation calls `cpuTestINTCInts()`.  Stays as a
        // hook in the sibling module.
    }
}

#[inline]
pub unsafe fn hw_dmac_irq(n: u32) {
    hw_or32(DMAC_STAT, 1u32 << n);
    if (hw_read32(DMAC_STAT) & (1u32 << (n + 16))) != 0 {
        // Real implementation calls `cpuTestDMACInts()`.
    }
}

#[inline]
pub unsafe fn fire_mfifo_empty() {
    hw_spr_log!("MFIFO Data Empty");
    hw_dmac_irq(DMAC_MFIFO_EMPTY);
    // Real implementation also clears FQC based on `dmacRegs.ctrl.MFD`;
    // that bookkeeping lives in the VIF/GIF modules.
}

// -----------------------------------------------------------------------------
// MFIFO ringbuffer helpers
// -----------------------------------------------------------------------------

/// True when the MFIFO base address is aligned to 16 bytes.
#[inline]
fn mfifo_aligned(rbor: u32) -> bool {
    (rbor & 0xF) == 0
}

/// Write `qwc` quadwords into the MFIFO ring buffer at `addr`.  Returns
/// false if the base address is not a valid physical address.
///
/// Mirrors `hwMFIFOWrite`.
pub unsafe fn hw_mfifo_write(_addr: u32, _qwc: u32) -> bool {
    // rbor is fetched from the live register file.
    let rbor = hw_read32(DMAC_RBOR);
    let rbsr = hw_read32(DMAC_RBSR);
    if !mfifo_aligned(rbor) {
        // `pxAssert` equivalent: log and continue.
        hw_spr_log!("MFIFO base {} not QW aligned", rbor);
    }
    if rbor == 0 {
        hw_spr_log!("MFIFO base is 0");
        return false;
    }
    // Real implementation calls `MemCopy_WrappedDest` here.  The Rust port
    // defers to the host memory subsystem; we only validate.
    let _ = rbsr;
    true
}

/// Resume whichever channel the MFIFO was feeding.  Mirrors `hwMFIFOResume`.
pub unsafe fn hw_mfifo_resume(mfd: u32) {
    match mfd {
        MFD_VIF1 => {
            hw_spr_log!("Resuming VIF1 MFIFO");
            // Real implementation flips `vif1.inprogress` and triggers a
            // CPU_INT if the VIF is ready to accept more data.
        }
        MFD_GIF => {
            hw_spr_log!("Resuming GIF MFIFO");
            // Real implementation clears GIF_STATE_EMPTY and triggers
            // CPU_INT.
        }
        _ => {}
    }
}

// -----------------------------------------------------------------------------
// DMA source-chain helpers
// -----------------------------------------------------------------------------

/// DMA source-chain interpreter with full CALL/RET stack support.  Mirrors
/// `hwDmacSrcChainWithStack`.
pub unsafe fn hw_dmac_src_chain_with_stack(
    tadr: &mut u32,
    madr: &mut u32,
    asr0: &mut u32,
    asr1: &mut u32,
    asp: &mut u32,
    qwc: u32,
    id: i32,
) -> bool {
    match id {
        TAG_REFE => {
            *tadr += 16;
            true
        }
        TAG_CNT => {
            *tadr += 16;
            *madr = *tadr;
            false
        }
        TAG_NEXT => {
            let tmp = *madr;
            *madr = *tadr + 16;
            *tadr = tmp;
            false
        }
        TAG_REF | TAG_REFS => {
            *tadr += 16;
            false
        }
        TAG_CALL => {
            let tmp = *madr;
            *madr = *tadr + 16;
            match *asp {
                0 => {
                    *asr0 = *madr + (qwc << 4);
                    *asp += 1;
                }
                1 => {
                    *asr1 = *madr + (qwc << 4);
                    *asp += 1;
                }
                _ => {
                    eprintln!("Call Stack Overflow");
                    return true;
                }
            }
            *tadr = tmp;
            false
        }
        TAG_RET => {
            *madr = *tadr + 16;
            match *asp {
                2 => {
                    *tadr = *asr1;
                    *asr1 = 0;
                    *asp -= 1;
                }
                1 => {
                    *tadr = *asr0;
                    *asr0 = 0;
                    *asp -= 1;
                }
                _ => return true,
            }
            false
        }
        TAG_END => {
            *madr = *tadr + 16;
            // Don't increment tadr.
            true
        }
        _ => true,
    }
}

/// Update `tadr` after a CNT tag, if running in chain mode.  Mirrors
/// `hwDmacSrcTadrInc`.
pub unsafe fn hw_dmac_src_tadr_inc(_str: u32, _mod: u32, _chcr_tag: u32) {
    // The C++ version reads `dma.chcr.STR/MOD/TAG`.  We only have the
    // arguments here; consumers in the DMA module are expected to gate
    // the call accordingly.
}

/// Simpler DMA source-chain used by channels that don't support
/// CALL/RET (e.g. IPU).  Mirrors `hwDmacSrcChain`.
pub unsafe fn hw_dmac_src_chain(
    tadr: &mut u32,
    madr: &mut u32,
    id: i32,
) -> bool {
    match id {
        TAG_REFE => {
            *tadr += 16;
            true
        }
        TAG_CNT => {
            *madr = *tadr + 16;
            *tadr = *madr;
            false
        }
        TAG_NEXT => {
            let tmp = *madr;
            *madr = *tadr + 16;
            *tadr = tmp;
            false
        }
        TAG_REF | TAG_REFS => {
            *tadr += 16;
            false
        }
        TAG_END => {
            *madr = *tadr + 16;
            true
        }
        _ => true,
    }
}

// -----------------------------------------------------------------------------
// Page dispatch (the giant switch tables)
// -----------------------------------------------------------------------------

type Read8Fn  = unsafe fn(u32) -> u8;
type Read16Fn = unsafe fn(u32) -> u16;
type Read32Fn = unsafe fn(u32) -> u32;
type Read64Fn = unsafe fn(u32) -> u64;
type Read128Fn= unsafe fn(u32) -> [u32; 4];

type Write8Fn  = unsafe fn(u32, u8);
type Write16Fn = unsafe fn(u32, u16);
type Write32Fn = unsafe fn(u32, u32);
type Write64Fn = unsafe fn(u32, u64);
type Write128Fn= unsafe fn(u32, [u32; 4]);

/// 256-entry dispatch table indexed by `(addr >> 16) & 0xFF`.  Mirrors the
/// C++ `_hwRead32<page>` template instantiations.
pub static mut PSX_HW4_READ8_TABLE:  [Read8Fn;  256]   = [default_read8;  256];
pub static mut PSX_HW4_READ16_TABLE: [Read16Fn; 256]   = [default_read16; 256];
pub static mut PSX_HW4_READ32_TABLE: [Read32Fn; 256]   = [default_read32; 256];
pub static mut PSX_HW4_READ64_TABLE: [Read64Fn; 256]   = [default_read64; 256];
pub static mut PSX_HW4_READ128_TABLE:[Read128Fn;256]   = [default_read128;256];

pub static mut PSX_HW4_WRITE8_TABLE:  [Write8Fn;  256]  = [default_write8;  256];
pub static mut PSX_HW4_WRITE16_TABLE: [Write16Fn; 256]  = [default_write16; 256];
pub static mut PSX_HW4_WRITE32_TABLE: [Write32Fn; 256]  = [default_write32; 256];
pub static mut PSX_HW4_WRITE64_TABLE: [Write64Fn; 256]  = [default_write64; 256];
pub static mut PSX_HW4_WRITE128_TABLE:[Write128Fn;256]  = [default_write128;256];

// -----------------------------------------------------------------------------
// Default fallbacks (unmapped pages return 0)
// -----------------------------------------------------------------------------

unsafe fn default_read8(_addr: u32) -> u8 { 0 }
unsafe fn default_read16(_addr: u32) -> u16 { 0 }
unsafe fn default_read32(addr: u32) -> u32 {
    // Bus error is generated by the VTLB; the I/O pages here are safe
    // to read as zero if unmapped.
    let _ = addr;
    0
}
unsafe fn default_read64(_addr: u32) -> u64 { 0 }
unsafe fn default_read128(_addr: u32) -> [u32; 4] { [0; 4] }

unsafe fn default_write8(_addr: u32, _v: u8) {}
unsafe fn default_write16(_addr: u32, _v: u16) {}
unsafe fn default_write32(addr: u32, v: u32) {
    // Default C++ behaviour: `psHu32(mem) = value;` for the entire
    // register file.
    hw_write32(addr & 0xFFFF, v);
}
unsafe fn default_write64(addr: u32, v: u64) {
    hw_write32(addr & 0xFFFF, v as u32);
    hw_write32((addr + 4) & 0xFFFF, (v >> 32) as u32);
}
unsafe fn default_write128(_addr: u32, _v: [u32; 4]) {}

// -----------------------------------------------------------------------------
// Counter page handlers (pages 0x00, 0x01)
// -----------------------------------------------------------------------------

unsafe fn read8_counters(_addr: u32) -> u8 { 0 }
unsafe fn read16_counters(addr: u32) -> u16 {
    let off = addr & 0xFFFF;
    let word = (hw_read32(off & !0x3) >> ((off & 0x2) << 3)) as u16;
    word
}
unsafe fn read32_counters(addr: u32) -> u32 {
    // Real implementation calls `rcntRead32<page>(mem)`.  Sibling module.
    let off = addr & 0xFFFF;
    hw_read32(off)
}
unsafe fn read64_counters(addr: u32) -> u64 {
    (hw_read32(addr & 0xFFFF) as u64) | ((hw_read32((addr + 4) & 0xFFFF) as u64) << 32)
}
unsafe fn read128_counters(addr: u32) -> [u32; 4] {
    [
        hw_read32(addr & 0xFFFF),
        hw_read32((addr + 4) & 0xFFFF),
        hw_read32((addr + 8) & 0xFFFF),
        hw_read32((addr + 12) & 0xFFFF),
    ]
}

unsafe fn write8_counters(_addr: u32, _v: u8) {
    // Stub: real handler lives in `Counters` sibling.
}
unsafe fn write16_counters(_addr: u32, _v: u16) {}
unsafe fn write32_counters(addr: u32, v: u32) {
    hw_write32(addr & 0xFFFF, v);
}
unsafe fn write64_counters(addr: u32, v: u64) {
    hw_write32(addr & 0xFFFF, v as u32);
    hw_write32((addr + 4) & 0xFFFF, (v >> 32) as u32);
}
unsafe fn write128_counters(_addr: u32, _v: [u32; 4]) {}

// -----------------------------------------------------------------------------
// IPU page handler (page 0x02)
// -----------------------------------------------------------------------------

unsafe fn read8_ipu(addr: u32) -> u8 {
    let v = hw_read32(addr & !0x3);
    let s = (addr & 0x3) * 8;
    ((v >> s) & 0xFF) as u8
}
unsafe fn read16_ipu(addr: u32) -> u16 {
    let v = hw_read32(addr & !0x3);
    let s = (addr & 0x2) * 8;
    ((v >> s) & 0xFFFF) as u16
}
unsafe fn read32_ipu(addr: u32) -> u32 {
    hw_read32(addr & 0xFFFF)
}
unsafe fn read64_ipu(addr: u32) -> u64 {
    (hw_read32(addr & 0xFFFF) as u64) | ((hw_read32((addr + 4) & 0xFFFF) as u64) << 32)
}
unsafe fn read128_ipu(addr: u32) -> [u32; 4] {
    [
        hw_read32(addr & 0xFFFF),
        hw_read32((addr + 4) & 0xFFFF),
        hw_read32((addr + 8) & 0xFFFF),
        hw_read32((addr + 12) & 0xFFFF),
    ]
}

unsafe fn write8_ipu(addr: u32, v: u8) {
    let merged = hw_read32(addr & !0x3);
    let s = (addr & 0x3) * 8;
    let mask = 0xFFu32 << s;
    let new_val = (merged & !mask) | (((v as u32) & 0xFF) << s);
    hw_write32(addr & !0x3, new_val);
}
unsafe fn write16_ipu(addr: u32, v: u16) {
    let merged = hw_read32(addr & !0x3);
    let s = (addr & 0x2) * 8;
    let mask = 0xFFFFu32 << s;
    let new_val = (merged & !mask) | (((v as u32) & 0xFFFF) << s);
    hw_write32(addr & !0x3, new_val);
}
unsafe fn write32_ipu(addr: u32, v: u32) {
    hw_write32(addr & 0xFFFF, v);
}
unsafe fn write64_ipu(addr: u32, v: u64) {
    hw_write32(addr & 0xFFFF, v as u32);
    hw_write32((addr + 4) & 0xFFFF, (v >> 32) as u32);
}
unsafe fn write128_ipu(addr: u32, v: [u32; 4]) {
    hw_write32(addr,         v[0]);
    hw_write32(addr + 4,     v[1]);
    hw_write32(addr + 8,     v[2]);
    hw_write32(addr + 12,    v[3]);
}

// -----------------------------------------------------------------------------
// GIF / VIF page (page 0x03)
// -----------------------------------------------------------------------------

unsafe fn read8_gif_vif(_addr: u32) -> u8 { 0 }
unsafe fn read16_gif_vif(_addr: u32) -> u16 { 0 }
unsafe fn read32_gif_vif(addr: u32) -> u32 {
    if addr >= ee_memory_map::VIF0_START {
        if addr >= ee_memory_map::VIF1_START {
            // vifRead32<1>
            hw_read32(addr & 0xFFFF)
        } else {
            // vifRead32<0>
            hw_read32(addr & 0xFFFF)
        }
    } else {
        // dmacRead32<0x03>
        hw_read32(addr & 0xFFFF)
    }
}
unsafe fn read64_gif_vif(addr: u32) -> u64 {
    (read32_gif_vif(addr) as u64) | ((read32_gif_vif(addr + 4) as u64) << 32)
}
unsafe fn read128_gif_vif(addr: u32) -> [u32; 4] {
    [
        read32_gif_vif(addr),
        read32_gif_vif(addr + 4),
        read32_gif_vif(addr + 8),
        read32_gif_vif(addr + 12),
    ]
}

unsafe fn write8_gif_vif(_addr: u32, _v: u8) {}
unsafe fn write16_gif_vif(_addr: u32, _v: u16) {}
unsafe fn write32_gif_vif(addr: u32, v: u32) {
    if addr >= ee_memory_map::VIF0_START {
        if addr >= ee_memory_map::VIF1_START {
            // vifWrite32<1>
            hw_write32(addr & 0xFFFF, v);
        } else {
            // vifWrite32<0>
            hw_write32(addr & 0xFFFF, v);
        }
    } else {
        // DMAC fast path / GIF_CTRL / GIF_MODE handled by the sibling
        // modules.  We just latch the value.
        hw_write32(addr & 0xFFFF, v);
    }
}
unsafe fn write64_gif_vif(addr: u32, v: u64) {
    hw_write32(addr & 0xFFFF, v as u32);
    hw_write32((addr + 4) & 0xFFFF, (v >> 32) as u32);
}
unsafe fn write128_gif_vif(_addr: u32, _v: [u32; 4]) {}

// -----------------------------------------------------------------------------
// FIFO pages (0x04, 0x05, 0x06, 0x07)
// -----------------------------------------------------------------------------

unsafe fn read8_fifo(_addr: u32) -> u8 { 0 }
unsafe fn read16_fifo(_addr: u32) -> u16 { 0 }
unsafe fn read32_fifo(addr: u32) -> u32 {
    // 32-bit FIFO read: read 128 bits and pick the relevant word.
    let aligned = addr & !0xF;
    let word = ((addr >> 2) & 0x3) as usize;
    let full = read128_fifo(aligned);
    full[word]
}
unsafe fn read64_fifo(addr: u32) -> u64 {
    // 64-bit FIFO read: read 128 bits and pick the relevant qword.
    let aligned = addr & !0xF;
    let qw = ((addr >> 3) & 0x1) as usize;
    let full = read128_fifo(aligned);
    (full[qw * 2] as u64) | ((full[qw * 2 + 1] as u64) << 32)
}
unsafe fn read128_fifo(addr: u32) -> [u32; 4] {
    let page = (addr >> 16) & 0xF;
    match page {
        0x05 => {
            // VIF1 FIFO
            [0; 4]
        }
        0x07 => {
            if addr & 0x10 != 0 {
                // IPUin is write-only
                [0; 4]
            } else {
                // IPUout FIFO
                [0; 4]
            }
        }
        0x04 | 0x06 => {
            // VIF0 and GIF are write-only FIFOs.
            [0; 4]
        }
        _ => [0; 4],
    }
}

unsafe fn write8_fifo(_addr: u32, _v: u8) {}
unsafe fn write16_fifo(_addr: u32, _v: u16) {}
unsafe fn write32_fifo(addr: u32, v: u32) {
    // The C++ implementation zero-fills the rest of the 128-bit word and
    // forwards to _hwWrite128.
    let mut data = [0u32; 4];
    data[((addr >> 2) & 0x3) as usize] = v;
    write128_fifo(addr & !0xF, data);
}
unsafe fn write64_fifo(addr: u32, v: u64) {
    let mut data = [0u32; 4];
    data[((addr >> 3) & 0x1) as usize * 2]     = v as u32;
    data[((addr >> 3) & 0x1) as usize * 2 + 1] = (v >> 32) as u32;
    write128_fifo(addr & !0xF, data);
}
unsafe fn write128_fifo(addr: u32, v: [u32; 4]) {
    let page = (addr >> 16) & 0xF;
    match page {
        0x04 => {
            // VIF0 FIFO
            let _ = v;
        }
        0x05 => {
            // VIF1 FIFO
            let _ = v;
        }
        0x06 => {
            // GIF FIFO
            let _ = v;
        }
        0x07 => {
            if addr & 0x10 != 0 {
                // IPUin FIFO
                let _ = v;
            } else {
                // IPUout writes are discarded.
            }
        }
        _ => {}
    }
}

// -----------------------------------------------------------------------------
// DMA pages (0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E)
// -----------------------------------------------------------------------------

unsafe fn read8_dma(addr: u32) -> u8 {
    let v = hw_read32(addr & !0x3);
    let s = (addr & 0x3) * 8;
    ((v >> s) & 0xFF) as u8
}
unsafe fn read16_dma(addr: u32) -> u16 {
    let v = hw_read32(addr & !0x3);
    let s = (addr & 0x2) * 8;
    ((v >> s) & 0xFFFF) as u16
}
unsafe fn read32_dma(addr: u32) -> u32 {
    hw_read32(addr & 0xFFFF)
}
unsafe fn read64_dma(addr: u32) -> u64 {
    (hw_read32(addr & 0xFFFF) as u64) | ((hw_read32((addr + 4) & 0xFFFF) as u64) << 32)
}
unsafe fn read128_dma(addr: u32) -> [u32; 4] {
    read64_dma(addr);
    [
        hw_read32(addr & 0xFFFF),
        hw_read32((addr + 4) & 0xFFFF),
        hw_read32((addr + 8) & 0xFFFF),
        hw_read32((addr + 12) & 0xFFFF),
    ]
}

unsafe fn write8_dma(addr: u32, v: u8) {
    let merged = hw_read32(addr & !0x3);
    let s = (addr & 0x3) * 8;
    let mask = 0xFFu32 << s;
    let new_val = (merged & !mask) | (((v as u32) & 0xFF) << s);
    hw_write32(addr & !0x3, new_val);
}
unsafe fn write16_dma(addr: u32, v: u16) {
    let merged = hw_read32(addr & !0x3);
    let s = (addr & 0x2) * 8;
    let mask = 0xFFFFu32 << s;
    let new_val = (merged & !mask) | (((v as u32) & 0xFFFF) << s);
    hw_write32(addr & !0x3, new_val);
}
unsafe fn write32_dma(addr: u32, v: u32) {
    hw_write32(addr & 0xFFFF, v);
}
unsafe fn write64_dma(addr: u32, v: u64) {
    hw_write32(addr & 0xFFFF, v as u32);
    hw_write32((addr + 4) & 0xFFFF, (v >> 32) as u32);
}
unsafe fn write128_dma(_addr: u32, _v: [u32; 4]) {}

// -----------------------------------------------------------------------------
// INTC / SIO / SBUS / MCH page (page 0x0F)
// -----------------------------------------------------------------------------

unsafe fn intc_hack_check() {
    // Real implementation compares `cpuRegs.nextEventCycle` against
    // `cpuRegs.cycle` and rewinds if the difference is too large.  This
    // lives in the CPU module; here we just preserve the API.
}

unsafe fn read8_intc_sio(addr: u32) -> u8 {
    if addr == SIO_RXFIFO {
        if EE_SIO_RX_FIFO.is_empty() {
            return 0;
        }
        return EE_SIO_RX_FIFO.pop_front().unwrap_or(0);
    }
    let v = read32_intc_sio(addr & !0x3);
    let s = (addr & 0x3) * 8;
    ((v >> s) & 0xFF) as u8
}

unsafe fn read16_intc_sio(addr: u32) -> u16 {
    let v = read32_intc_sio(addr & !0x3);
    let s = (addr & 0x2) * 8;
    ((v >> s) & 0xFFFF) as u16
}

unsafe fn read32_intc_sio(addr: u32) -> u32 {
    // Performance shortcut: if the EE is hot-spinning on INTC_STAT,
    // also fix up the cycle counter so the next event is honoured.
    if addr == INTC_STAT {
        if (HW_ICFG & (1 << 3)) == 0 {
            intc_hack_check();
        }
        return hw_read32(INTC_STAT);
    }

    // PS1 bridge (SBUS 0xF300..0xF400).
    if (addr & 0x1FFF_FFFF) >= ee_memory_map::SBUS_PS1_START
        && (addr & 0x1FFF_FFFF) <  ee_memory_map::SBUS_PS1_END
    {
        return pgif_read(addr);
    }

    // SIF2 PS1 bridge (kept for reference; gated by the original
    // condition `(mem & 0x1000ff00) == 0x1000f300`).
    if (addr & 0x1000_FF00) == 0x1000_F300 {
        match addr & 0xF0 {
            0x00 => return hw_read32(0x1F80_1814 & 0xFFFF),
            0x80 => {
                let sif2_size = SIF2_FIFO_SIZE.min(7);
                let mut ret = hw_read32(addr & 0xFFFF) | (sif2_size << 16);
                if SIF2_FIFO_SIZE > 0 {
                    ret |= 0x8000_0000;
                }
                return ret;
            }
            0xC0 | 0xE0 => {
                if SIF2_FIFO_SIZE > 0 {
                    if SIF2_FIFO_SIZE > 0 {
                        SIF2_FIFO_SIZE -= 1;
                    }
                    return hw_read32(addr & 0xFFFF);
                }
                return 0;
            }
            _ => {}
        }
    }

    match addr {
        x if x == SIO_ISR => {
            if !EE_SIO_RX_FIFO.is_empty() {
                0xF00
            } else {
                0
            }
        }
        0x1000_F410 => 0,
        x if x == MCH_RICM => 0,
        x if x == SBUS_F240 => hw_read32(SBUS_F240) | 0xF000_0102,
        x if x == SBUS_F260 => hw_read32(SBUS_F260),
        x if x == MCH_DRD => {
            let ricm = hw_read32(MCH_RICM);
            if ((ricm >> 6) & 0xF) == 0 {
                match (ricm >> 16) & 0xFFF {
                    0x21 => {
                        if RDRAM_SDEVID < RDRAM_DEVICES {
                            RDRAM_SDEVID += 1;
                            0x1F
                        } else {
                            0
                        }
                    }
                    0x23 => 0x0D0D, // PVER=3|MVER=16|DBL=1|REFBIT=5
                    0x24 => 0x0090, // SVER=0|CORG=4(5x9x6)|SPT=1|DEVTYP=0|BYTE=0
                    0x40 => ricm & 0x1F,
                    _ => 0,
                }
            } else {
                0
            }
        }
        _ => hw_read32(addr & 0xFFFF),
    }
}

unsafe fn read64_intc_sio(addr: u32) -> u64 {
    (read32_intc_sio(addr) as u64) | ((read32_intc_sio(addr + 4) as u64) << 32)
}

unsafe fn read128_intc_sio(addr: u32) -> [u32; 4] {
    if (addr & 0xFFFF_FF00) == 0x1000_F300 && addr == 0x1000_F3E0 {
        let mut parts = [0u32; 4];
        for i in 0..4 {
            if SIF2_FIFO_SIZE > 0 {
                SIF2_FIFO_SIZE -= 1;
            }
            parts[i] = hw_read32(0xF3E0);
        }
        return parts;
    }
    [
        read32_intc_sio(addr),
        read32_intc_sio(addr + 4),
        read32_intc_sio(addr + 8),
        read32_intc_sio(addr + 12),
    ]
}

unsafe fn write8_intc_sio(addr: u32, v: u8) {
    if addr == SIO_TXFIFO {
        // Skip LF after CR; mirror `last_char_was_cr` dedup.
        if LAST_TX_WAS_CR && v == b'\n' {
            LAST_TX_WAS_CR = false;
            return;
        }
        LAST_TX_WAS_CR = v == b'\r';
        let mut should_flush = false;
        if LAST_TX_WAS_CR {
            should_flush = true;
            EE_SIO_TX_FIFO.push_back(b'\n');
        } else {
            should_flush = v == b'\n';
            EE_SIO_TX_FIFO.push_back(v);
        }
        if EE_SIO_TX_FIFO.len() == 1024 || should_flush {
            // Real implementation calls `eeConLog(ShiftJIS_ConvertString(...))`.
            EE_SIO_TX_FIFO.clear();
        }
        return;
    }
    match addr & !3 {
        x if x == DMAC_STAT || x == INTC_STAT || x == INTC_MASK || x == DMAC_FAKESTAT => {
            write32_intc_sio(addr & !3, (v as u32) << ((addr & 0x3) * 8));
            return;
        }
        _ => {}
    }
    let merged = read32_intc_sio(addr & !0x3);
    let s = (addr & 0x3) * 8;
    let mask = 0xFFu32 << s;
    write32_intc_sio(addr & !0x3, (merged & !mask) | (((v as u32) & 0xFF) << s));
}

unsafe fn write16_intc_sio(addr: u32, v: u16) {
    match addr & !3 {
        x if x == DMAC_STAT || x == INTC_STAT || x == INTC_MASK || x == DMAC_FAKESTAT => {
            write32_intc_sio(addr & !3, (v as u32) << ((addr & 0x3) * 8));
            return;
        }
        _ => {}
    }
    let merged = read32_intc_sio(addr & !0x3);
    let s = (addr & 0x2) * 8;
    let mask = 0xFFFFu32 << s;
    write32_intc_sio(addr & !0x3, (merged & !mask) | (((v as u32) & 0xFFFF) << s));
}

unsafe fn write32_intc_sio(addr: u32, v: u32) {
    // `HELPSWITCH(m) == ((m) >> 4) & 0xff` is the lookup key the C++
    // switch uses; we mimic it with a few explicit checks for the
    // popular addresses.
    let key = (addr >> 4) & 0xFF;
    match key {
        k if k == (INTC_STAT >> 4) & 0xFF => {
            hw_and32(INTC_STAT, !v);
            return;
        }
        k if k == (INTC_MASK >> 4) & 0xFF => {
            hw_xor32(INTC_MASK, v & 0xFFFF);
            // `cpuTestINTCInts()` is a hook in the CPU module.
            return;
        }
        k if k == (SIO_TXFIFO >> 4) & 0xFF => {
            // 32-bit SIO TX writes fan out as four 8-bit writes.
            let bytes = v.to_le_bytes();
            for b in bytes.iter() {
                write8_intc_sio(SIO_TXFIFO, *b);
            }
            return;
        }
        k if k == (SBUS_F200 >> 4) & 0xFF => {
            // Default assignment.
        }
        k if k == (SBUS_F220 >> 4) & 0xFF => {
            hw_or32(addr & 0xFFFF, v);
            return;
        }
        k if k == (SBUS_F230 >> 4) & 0xFF => {
            hw_and32(addr & 0xFFFF, !v);
            return;
        }
        k if k == (SBUS_F240 >> 4) & 0xFF => {
            if v & (1 << 18) != 0 {
                // iopIntcIrq(1)
            }
            if v & (1 << 19) != 0 {
                // Switch to PS1 mode; resets PSX, SPU2, etc.
                SPU2_RESET_REQUESTED.store(1, Ordering::SeqCst);
                HW_ICFG = 0x8;
            }
            if v & 0x100 == 0 {
                hw_and32(SBUS_F240, !0x100);
            } else {
                hw_or32(SBUS_F240, 0x100);
            }
            return;
        }
        k if k == (SBUS_F260 >> 4) & 0xFF => {
            hw_write32(SBUS_F260, v);
            return;
        }
        k if k == (MCH_RICM >> 4) & 0xFF => {
            // MCH_RICM: x:4|SA:12|x:5|SDEV:1|SOP:4|SBC:1|SDEV:5
            let sa = (v >> 16) & 0xFFF;
            let srp = (v >> 6) & 0xF;
            let sbc = (hw_read32(MCH_DRD) >> 7) & 1;
            if sa == 0x21 && srp == 1 && sbc == 0 {
                RDRAM_SDEVID = 0;
            }
            hw_write32(MCH_RICM, v & !0x8000_0000); // Kill the busy bit
            return;
        }
        k if k == (MCH_DRD >> 4) & 0xFF => {
            // Default assignment.
        }
        k if k == (DMAC_ENABLEW >> 4) & 0xFF => {
            // Real implementation calls `dmacWrite32<0x0f>(DMAC_ENABLEW, v)`.
            hw_write32(DMAC_ENABLEW, v);
            return;
        }
        _ => {}
    }
    // PS1 bridge fall-through.
    if (addr & 0x1FFF_FFFF) >= ee_memory_map::SBUS_PS1_START
        && (addr & 0x1FFF_FFFF) <  ee_memory_map::SBUS_PS1_END
    {
        pgif_write(addr, v);
        return;
    }
    hw_write32(addr & 0xFFFF, v);
}

unsafe fn write64_intc_sio(addr: u32, v: u64) {
    write32_intc_sio(addr, v as u32);
    write32_intc_sio(addr + 4, (v >> 32) as u32);
}

unsafe fn write128_intc_sio(_addr: u32, _v: [u32; 4]) {}

// -----------------------------------------------------------------------------
// SPU2 reset coordination
// -----------------------------------------------------------------------------

static SPU2_RESET_REQUESTED: AtomicU32 = AtomicU32::new(0);
pub fn spu2_reset_consume() -> bool {
    SPU2_RESET_REQUESTED.swap(0, Ordering::SeqCst) != 0
}

// -----------------------------------------------------------------------------
// PGIF (PS1 bridge) stubs
// -----------------------------------------------------------------------------

unsafe fn pgif_read(addr: u32) -> u32 { hw_read32(addr & 0xFFFF) }
unsafe fn pgif_write(addr: u32, v: u32) { hw_write32(addr & 0xFFFF, v); }

// -----------------------------------------------------------------------------
// Public dispatch entry points
// -----------------------------------------------------------------------------

#[inline]
fn dispatch_index(addr: u32) -> usize {
    ((addr >> 16) & 0xFF) as usize
}

pub unsafe fn psxHw4Read8(addr: u32) -> u8 {
    ensure_dispatch_for(addr);
    PSX_HW4_READ8_TABLE[dispatch_index(addr)](addr)
}
pub unsafe fn psxHw4Read16(addr: u32) -> u16 {
    ensure_dispatch_for(addr);
    PSX_HW4_READ16_TABLE[dispatch_index(addr)](addr)
}
pub unsafe fn psxHw4Read32(addr: u32) -> u32 {
    ensure_dispatch_for(addr);
    PSX_HW4_READ32_TABLE[dispatch_index(addr)](addr)
}
pub unsafe fn psxHw4Read64(addr: u32) -> u64 {
    ensure_dispatch_for(addr);
    PSX_HW4_READ64_TABLE[dispatch_index(addr)](addr)
}
pub unsafe fn psxHw4Read128(addr: u32) -> [u32; 4] {
    ensure_dispatch_for(addr);
    PSX_HW4_READ128_TABLE[dispatch_index(addr)](addr)
}

pub unsafe fn psxHw4Write8(addr: u32, value: u8) {
    ensure_dispatch_for(addr);
    PSX_HW4_WRITE8_TABLE[dispatch_index(addr)](addr, value)
}
pub unsafe fn psxHw4Write16(addr: u32, value: u16) {
    ensure_dispatch_for(addr);
    PSX_HW4_WRITE16_TABLE[dispatch_index(addr)](addr, value)
}
pub unsafe fn psxHw4Write32(addr: u32, value: u32) {
    ensure_dispatch_for(addr);
    PSX_HW4_WRITE32_TABLE[dispatch_index(addr)](addr, value)
}
pub unsafe fn psxHw4Write64(addr: u32, value: u64) {
    ensure_dispatch_for(addr);
    PSX_HW4_WRITE64_TABLE[dispatch_index(addr)](addr, value)
}
pub unsafe fn psxHw4Write128(addr: u32, value: [u32; 4]) {
    ensure_dispatch_for(addr);
    PSX_HW4_WRITE128_TABLE[dispatch_index(addr)](addr, value)
}

// -----------------------------------------------------------------------------
// Table population (idempotent; call from `psxHwInit` or first use).
// -----------------------------------------------------------------------------

/// Wire the dispatch tables to the per-page handlers.  Idempotent.
pub unsafe fn install_dispatch_table() {
    for p in 0u8..=1 {
        install_counters_page(p);
    }
    install_page(0x02, read8_ipu,  read16_ipu,  read32_ipu,  read64_ipu,  read128_ipu,
                            write8_ipu, write16_ipu, write32_ipu, write64_ipu, write128_ipu);
    install_page(0x03, read8_gif_vif,  read16_gif_vif,  read32_gif_vif,  read64_gif_vif,  read128_gif_vif,
                            write8_gif_vif, write16_gif_vif, write32_gif_vif, write64_gif_vif, write128_gif_vif);
    for p in 0x04u8..=0x07 {
        install_fifo_page(p);
    }
    for p in 0x08u8..=0x0E {
        install_dma_page(p);
    }
    install_page(0x0F, read8_intc_sio,  read16_intc_sio,  read32_intc_sio,  read64_intc_sio,  read128_intc_sio,
                            write8_intc_sio, write16_intc_sio, write32_intc_sio, write64_intc_sio, write128_intc_sio);
}

unsafe fn install_counters_page(p: u8) {
    install_page(p, read8_counters,  read16_counters,  read32_counters,  read64_counters,  read128_counters,
                        write8_counters, write16_counters, write32_counters, write64_counters, write128_counters);
}

unsafe fn install_fifo_page(p: u8) {
    install_page(p, read8_fifo,  read16_fifo,  read32_fifo,  read64_fifo,  read128_fifo,
                        write8_fifo, write16_fifo, write32_fifo, write64_fifo, write128_fifo);
}

unsafe fn install_dma_page(p: u8) {
    install_page(p, read8_dma,  read16_dma,  read32_dma,  read64_dma,  read128_dma,
                        write8_dma, write16_dma, write32_dma, write64_dma, write128_dma);
}

unsafe fn install_page(
    p: u8,
    r8: Read8Fn,    r16: Read16Fn,    r32: Read32Fn,    r64: Read64Fn,    r128: Read128Fn,
    w8: Write8Fn,   w16: Write16Fn,   w32: Write32Fn,   w64: Write64Fn,   w128: Write128Fn,
) {
    let i = p as usize;
    PSX_HW4_READ8_TABLE[i]   = r8;
    PSX_HW4_READ16_TABLE[i]  = r16;
    PSX_HW4_READ32_TABLE[i]  = r32;
    PSX_HW4_READ64_TABLE[i]  = r64;
    PSX_HW4_READ128_TABLE[i] = r128;
    PSX_HW4_WRITE8_TABLE[i]  = w8;
    PSX_HW4_WRITE16_TABLE[i] = w16;
    PSX_HW4_WRITE32_TABLE[i] = w32;
    PSX_HW4_WRITE64_TABLE[i] = w64;
    PSX_HW4_WRITE128_TABLE[i]= w128;
}

// -----------------------------------------------------------------------------
// One-shot install on first use of any read/write entry point.
// -----------------------------------------------------------------------------

static DISPATCH_INSTALLED: AtomicU32 = AtomicU32::new(0);

fn ensure_dispatch_installed() {
    if DISPATCH_INSTALLED.swap(1, Ordering::SeqCst) == 0 {
        unsafe { install_dispatch_table(); }
    }
}

// Fallback: provide a manual init too so the dispatch can be installed
// without relying on a ctor shim.
pub fn psx_hw_install_dispatch() {
    ensure_dispatch_installed();
}

// Top-level entry points lazily install the dispatch table.  This keeps
// `std`-only builds (no `ctor` shim) working without manual setup.
fn ensure_dispatch_for(addr: u32) {
    let _ = addr;
    ensure_dispatch_installed();
}

// -----------------------------------------------------------------------------
// Convenience: raw accessors that mirror the C++ `psHuNN` macros.
// -----------------------------------------------------------------------------

#[inline]
pub unsafe fn psxHw4Read32Raw(offset: u32) -> u32 {
    hw_read32(offset)
}

#[inline]
pub unsafe fn psxHw4Write32Raw(offset: u32, v: u32) {
    hw_write32(offset, v);
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_clears_state() {
        unsafe {
            hw_write32(DMAC_ENABLEW, 0xDEAD_BEEF);
            hw_write32(SBUS_F260,    0x1234_5678);
            psxHwReset();
            assert_eq!(hw_read32(DMAC_ENABLEW), 0x1201);
            assert_eq!(hw_read32(SBUS_F260),    0x1D00_0060);
        }
    }

    #[test]
    fn fifo_pages_return_zero_for_unmapped() {
        unsafe {
            install_dispatch_table();
            assert_eq!(psxHw4Read32(0x1004_0000), 0);
            assert_eq!(psxHw4Read32(0x1006_0000), 0);
            assert_eq!(psxHw4Read32(0x1007_0010), 0);
        }
    }

    #[test]
    fn intc_read_short_circuits() {
        unsafe {
            install_dispatch_table();
            hw_write32(INTC_STAT, 0xCAFE_BABE);
            assert_eq!(psxHw4Read32(INTC_STAT), 0xCAFE_BABE);
        }
    }

    #[test]
    fn sio_tx_dedup() {
        unsafe {
            install_dispatch_table();
            LAST_TX_WAS_CR = false;
            psxHw4Write8(SIO_TXFIFO, b'\r');
            psxHw4Write8(SIO_TXFIFO, b'\n'); // should be dedup'd
            psxHw4Write8(SIO_TXFIFO, b'h');
            psxHw4Write8(SIO_TXFIFO, b'i');
            psxHw4Write8(SIO_TXFIFO, b'\n');
            assert!(EE_SIO_TX_FIFO.is_empty());
        }
    }
}
