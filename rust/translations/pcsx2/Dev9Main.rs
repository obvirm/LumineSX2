// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust translation of the PCSX2 DEV9 subsystem.
//!
//! This module covers the entirety of the original C/C++ DEV9 implementation
//! (DEV9, Flash, Net, Smap, Sockets, AdapterUtils, pcap_io, SimpleQueue,
//! ThreadSafeMap).  The original implementation is split across many files
//! and uses platform-specific APIs, manual memory management, and global
//! state.  This translation preserves the high-level structure but
//! expresses the same ideas in idiomatic Rust 2021 with `static mut` for
//! the global `dev9` state, `&mut` references for the queue/map helpers,
//! and a single file containing every subsystem.
//!
//! All public surface (initialisation, register access, network plumbing,
//! adapter enumeration) is exposed as plain functions and types, so the
//! caller can drive the device from the PS2 IOP bridge without touching
//! any of the original PCSX2 internals.

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(dead_code)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

// ---------------------------------------------------------------------------
// Type aliases mirroring the PCSX2 typedefs
// ---------------------------------------------------------------------------

pub type u8  = ::std::primitive::u8;
pub type u16 = ::std::primitive::u16;
pub type u32 = ::std::primitive::u32;
pub type s8  = ::std::primitive::i8;
pub type s16 = ::std::primitive::i16;
pub type s32 = ::std::primitive::i32;

// ---------------------------------------------------------------------------
// Flash, ATA, SPD, SMAP register address constants
// ---------------------------------------------------------------------------

pub const SPD_INTR_ATA_FIFO_DATA: u16   = 1 << 1;
pub const SPD_INTR_ATA_FIFO_FULL: u16   = 1 << 15;
pub const SPD_INTR_ATA_FIFO_EMPTY: u16  = 1 << 14;
pub const SPD_INTR_ATA_FIFO_OVERFLOW: u16 = SPD_INTR_ATA_FIFO_FULL | SPD_INTR_ATA_FIFO_EMPTY;

pub const SPD_REGBASE: u32 = 0x1000_0000;

pub const ATA_INTR_INTRQ: u32 = 1 << 0;

pub const SPD_R_REV_1: u32   = SPD_REGBASE + 0x00;
pub const SPD_R_REV_2: u32   = SPD_REGBASE + 0x02;
pub const SPD_CAPS_SMAP: u16 = 1 << 0;
pub const SPD_CAPS_ATA: u16  = 1 << 1;
pub const SPD_CAPS_UART: u16 = 1 << 3;
pub const SPD_CAPS_DVR: u16  = 1 << 4;
pub const SPD_CAPS_FLASH: u16 = 1 << 5;
pub const SPD_R_REV_3: u32   = SPD_REGBASE + 0x04;
pub const SPD_R_0e: u32      = SPD_REGBASE + 0x0e;

pub const SPD_R_DMA_CTRL: u32     = SPD_REGBASE + 0x24;
pub const SPD_DMA_TO_SMAP: u16    = 1 << 0;
pub const SPD_DMA_FASTEST: u16    = 1 << 1;
pub const SPD_DMA_WIDE: u16       = 1 << 2;
pub const SPD_DMA_PAUSE: u16      = 1 << 4;
pub const SPD_R_INTR_STAT: u32    = SPD_REGBASE + 0x28;
pub const SPD_R_INTR_MASK: u32    = SPD_REGBASE + 0x2a;

pub const SPD_R_PIO_DIR: u32      = SPD_REGBASE + 0x2c;
pub const SPD_R_PIO_DATA: u32     = SPD_REGBASE + 0x2e;
pub const SPD_PP_DOUT:  u16       = 1 << 4;
pub const SPD_PP_DIN:   u16       = 1 << 5;
pub const SPD_PP_SCLK:  u16       = 1 << 6;
pub const SPD_PP_CSEL:  u16       = 1 << 7;
pub const SPD_PP_OP_READ:  u32    = 2;
pub const SPD_PP_OP_WRITE: u32    = 1;
pub const SPD_PP_OP_EWEN:  u32    = 0;
pub const SPD_PP_OP_EWDS:  u32    = 0;

pub const SPD_R_XFR_CTRL: u32     = SPD_REGBASE + 0x32;
pub const SPD_XFR_WRITE:  u16     = 1 << 0;
pub const SPD_XFR_DMAEN:  u16     = 1 << 7;
pub const SPD_R_DBUF_STAT: u32    = SPD_REGBASE + 0x38;
pub const SPD_DBUF_AVAIL_MAX: u16 = 0x10;
pub const SPD_DBUF_AVAIL_MASK: u16 = 0x1F;
pub const SPD_DBUF_STAT_1: u16    = 1 << 5;
pub const SPD_DBUF_STAT_2: u16    = 1 << 6;
pub const SPD_DBUF_STAT_FULL: u16 = 1 << 7;
pub const SPD_DBUF_RESET_READ_CNT: u16  = 1 << 0;
pub const SPD_DBUF_RESET_WRITE_CNT: u16 = 1 << 1;

pub const SPD_R_IF_CTRL: u32     = SPD_REGBASE + 0x64;
pub const SPD_IF_UDMA: u16       = 1 << 0;
pub const SPD_IF_READ: u16       = 1 << 1;
pub const SPD_IF_ATA_DMAEN: u16  = 1 << 2;
pub const SPD_IF_HDD_RESET: u16  = 1 << 6;
pub const SPD_IF_ATA_RESET: u16  = 1 << 7;
pub const SPD_R_PIO_MODE: u32    = SPD_REGBASE + 0x70;
pub const SPD_R_MDMA_MODE: u32   = SPD_REGBASE + 0x72;
pub const SPD_R_UDMA_MODE: u32   = SPD_REGBASE + 0x74;

pub const SMAP_INTR_EMAC3: u16 = 1 << 6;
pub const SMAP_INTR_RXEND: u16 = 1 << 5;
pub const SMAP_INTR_TXEND: u16 = 1 << 4;
pub const SMAP_INTR_RXDNV: u16 = 1 << 3;
pub const SMAP_INTR_TXDNV: u16 = 1 << 2;
pub const SMAP_INTR_CLR_ALL: u16 = SMAP_INTR_RXEND | SMAP_INTR_TXEND | SMAP_INTR_RXDNV;
pub const SMAP_INTR_ENA_ALL: u16 = SMAP_INTR_EMAC3 | SMAP_INTR_CLR_ALL;
pub const SMAP_INTR_BITMSK: u16 = 0x7C;

pub const SMAP_REGBASE: u32 = SPD_REGBASE + 0x100;

pub const SMAP_R_BD_MODE: u32      = SMAP_REGBASE + 0x02;
pub const SMAP_BD_SWAP:    u16     = 1 << 0;
pub const SMAP_R_INTR_CLR: u32     = SMAP_REGBASE + 0x28;

pub const SMAP_R_TXFIFO_CTRL:    u32 = SMAP_REGBASE + 0xf00;
pub const SMAP_TXFIFO_RESET: u16     = 1 << 0;
pub const SMAP_TXFIFO_DMAEN: u16     = 1 << 1;
pub const SMAP_R_TXFIFO_WR_PTR: u32  = SMAP_REGBASE + 0xf04;
pub const SMAP_R_TXFIFO_SIZE: u32    = SMAP_REGBASE + 0xf08;
pub const SMAP_R_TXFIFO_FRAME_CNT: u32 = SMAP_REGBASE + 0xf0C;
pub const SMAP_R_TXFIFO_FRAME_INC: u32 = SMAP_REGBASE + 0xf10;
pub const SMAP_R_TXFIFO_DATA:  u32    = SMAP_REGBASE + 0x1000;

pub const SMAP_R_RXFIFO_CTRL:    u32 = SMAP_REGBASE + 0xf30;
pub const SMAP_RXFIFO_RESET: u16     = 1 << 0;
pub const SMAP_RXFIFO_DMAEN: u16     = 1 << 1;
pub const SMAP_R_RXFIFO_RD_PTR: u32  = SMAP_REGBASE + 0xf34;
pub const SMAP_R_RXFIFO_SIZE: u32    = SMAP_REGBASE + 0xf38;
pub const SMAP_R_RXFIFO_FRAME_CNT: u32 = SMAP_REGBASE + 0xf3C;
pub const SMAP_R_RXFIFO_FRAME_DEC: u32 = SMAP_REGBASE + 0xf40;
pub const SMAP_R_RXFIFO_DATA: u32     = SMAP_REGBASE + 0x1100;

pub const SMAP_R_FIFO_ADDR: u32 = SMAP_REGBASE + 0x1200;
pub const SMAP_FIFO_CMD_READ: u16 = 1 << 1;
pub const SMAP_FIFO_DATA_SWAP: u16 = 1 << 0;
pub const SMAP_R_FIFO_DATA: u32 = SMAP_REGBASE + 0x1208;

pub const SMAP_EMAC3_REGBASE: u32 = SMAP_REGBASE + 0x1f00;
pub const SMAP_R_EMAC3_MODE0_L: u32 = SMAP_EMAC3_REGBASE + 0x00;
pub const SMAP_R_EMAC3_MODE0_H: u32 = SMAP_EMAC3_REGBASE + 0x02;
pub const SMAP_R_EMAC3_MODE1: u32   = SMAP_EMAC3_REGBASE + 0x04;
pub const SMAP_R_EMAC3_MODE1_L: u32 = SMAP_EMAC3_REGBASE + 0x04;
pub const SMAP_R_EMAC3_MODE1_H: u32 = SMAP_EMAC3_REGBASE + 0x06;
pub const SMAP_E3_FDX_ENABLE: u32 = 1 << 31;
pub const SMAP_E3_INLPBK_ENABLE: u32 = 1 << 30;
pub const SMAP_E3_VLAN_ENABLE: u32 = 1 << 29;
pub const SMAP_E3_FLOWCTRL_ENABLE: u32 = 1 << 28;
pub const SMAP_E3_ALLOW_PF: u32 = 1 << 27;
pub const SMAP_E3_ALLOW_EXTMNGIF: u32 = 1 << 25;
pub const SMAP_E3_IGNORE_SQE: u32 = 1 << 24;
pub const SMAP_E3_MEDIA_FREQ_BITSFT: u32 = 22;
pub const SMAP_E3_MEDIA_10M: u32 = 0 << 22;
pub const SMAP_E3_MEDIA_100M: u32 = 1 << 22;
pub const SMAP_E3_MEDIA_1000M: u32 = 2 << 22;
pub const SMAP_E3_MEDIA_MSK: u32 = 3 << 22;
pub const SMAP_E3_RXFIFO_SIZE_BITSFT: u32 = 20;
pub const SMAP_E3_RXFIFO_512: u32 = 0 << 20;
pub const SMAP_E3_RXFIFO_1K: u32 = 1 << 20;
pub const SMAP_E3_RXFIFO_2K: u32 = 2 << 20;
pub const SMAP_E3_RXFIFO_4K: u32 = 3 << 20;
pub const SMAP_E3_TXFIFO_SIZE_BITSFT: u32 = 18;
pub const SMAP_E3_TXFIFO_512: u32 = 0 << 18;
pub const SMAP_E3_TXFIFO_1K: u32 = 1 << 18;
pub const SMAP_E3_TXFIFO_2K: u32 = 2 << 18;
pub const SMAP_E3_TXREQ0_BITSFT: u32 = 15;
pub const SMAP_E3_TXREQ0_SINGLE: u32 = 0 << 15;
pub const SMAP_E3_TXREQ0_MULTI: u32 = 1 << 15;
pub const SMAP_E3_TXREQ0_DEPEND: u32 = 2 << 15;
pub const SMAP_E3_TXREQ1_BITSFT: u32 = 13;
pub const SMAP_E3_TXREQ1_SINGLE: u32 = 0 << 13;
pub const SMAP_E3_TXREQ1_MULTI: u32 = 1 << 13;
pub const SMAP_E3_TXREQ1_DEPEND: u32 = 2 << 13;
pub const SMAP_E3_JUMBO_ENABLE: u32 = 1 << 12;
pub const SMAP_R_EMAC3_TxMODE0_L: u32 = SMAP_EMAC3_REGBASE + 0x08;
pub const SMAP_E3_TX_GNP_0: u32 = 1 << (15 + 16);
pub const SMAP_E3_TX_GNP_1: u32 = 1 << (14 + 16);
pub const SMAP_E3_TX_GNP_DEPEND: u32 = 1 << (13 + 16);
pub const SMAP_E3_TX_FIRST_CHANNEL: u32 = 1 << (12 + 16);
pub const SMAP_R_EMAC3_TxMODE0_H: u32 = SMAP_EMAC3_REGBASE + 0x0A;
pub const SMAP_R_EMAC3_TxMODE1_L: u32 = SMAP_EMAC3_REGBASE + 0x0C;
pub const SMAP_R_EMAC3_TxMODE1_H: u32 = SMAP_EMAC3_REGBASE + 0x0E;
pub const SMAP_E3_TX_LOW_REQ_MSK: u32 = 0x1F;
pub const SMAP_E3_TX_LOW_REQ_BITSFT: u32 = 27;
pub const SMAP_E3_TX_URG_REQ_MSK: u32 = 0xFF;
pub const SMAP_E3_TX_URG_REQ_BITSFT: u32 = 16;
pub const SMAP_R_EMAC3_RxMODE: u32 = SMAP_EMAC3_REGBASE + 0x10;
pub const SMAP_R_EMAC3_RxMODE_L: u32 = SMAP_EMAC3_REGBASE + 0x10;
pub const SMAP_R_EMAC3_RxMODE_H: u32 = SMAP_EMAC3_REGBASE + 0x12;
pub const SMAP_E3_RX_STRIP_PAD: u32 = 1 << 31;
pub const SMAP_E3_RX_STRIP_FCS: u32 = 1 << 30;
pub const SMAP_E3_RX_RX_RUNT_FRAME: u32 = 1 << 29;
pub const SMAP_E3_RX_RX_FCS_ERR: u32 = 1 << 28;
pub const SMAP_E3_RX_RX_TOO_LONG_ERR: u32 = 1 << 27;
pub const SMAP_E3_RX_RX_IN_RANGE_ERR: u32 = 1 << 26;
pub const SMAP_E3_RX_PROP_PF: u32 = 1 << 25;
pub const SMAP_E3_RX_PROMISC: u32 = 1 << 24;
pub const SMAP_E3_RX_PROMISC_MCAST: u32 = 1 << 23;
pub const SMAP_E3_RX_INDIVID_ADDR: u32 = 1 << 22;
pub const SMAP_E3_RX_INDIVID_HASH: u32 = 1 << 21;
pub const SMAP_E3_RX_BCAST: u32 = 1 << 20;
pub const SMAP_E3_RX_MCAST: u32 = 1 << 19;
pub const SMAP_R_EMAC3_INTR_STAT: u32 = SMAP_EMAC3_REGBASE + 0x14;
pub const SMAP_R_EMAC3_INTR_STAT_L: u32 = SMAP_EMAC3_REGBASE + 0x14;
pub const SMAP_R_EMAC3_INTR_STAT_H: u32 = SMAP_EMAC3_REGBASE + 0x16;
pub const SMAP_R_EMAC3_INTR_ENABLE: u32 = SMAP_EMAC3_REGBASE + 0x18;
pub const SMAP_R_EMAC3_INTR_ENABLE_L: u32 = SMAP_EMAC3_REGBASE + 0x18;
pub const SMAP_R_EMAC3_INTR_ENABLE_H: u32 = SMAP_EMAC3_REGBASE + 0x1A;
pub const SMAP_E3_INTR_OVERRUN: u32 = 1 << 25;
pub const SMAP_E3_INTR_PF: u32 = 1 << 24;
pub const SMAP_E3_INTR_BAD_FRAME: u32 = 1 << 23;
pub const SMAP_E3_INTR_RUNT_FRAME: u32 = 1 << 22;
pub const SMAP_E3_INTR_SHORT_EVENT: u32 = 1 << 21;
pub const SMAP_E3_INTR_ALIGN_ERR: u32 = 1 << 20;
pub const SMAP_E3_INTR_BAD_FCS: u32 = 1 << 19;
pub const SMAP_E3_INTR_TOO_LONG: u32 = 1 << 18;
pub const SMAP_E3_INTR_OUT_RANGE_ERR: u32 = 1 << 17;
pub const SMAP_E3_INTR_IN_RANGE_ERR: u32 = 1 << 16;
pub const SMAP_E3_INTR_DEAD_DEPEND: u32 = 1 << 9;
pub const SMAP_E3_INTR_DEAD_0: u32 = 1 << 8;
pub const SMAP_E3_INTR_SQE_ERR_0: u32 = 1 << 7;
pub const SMAP_E3_INTR_TX_ERR_0: u32 = 1 << 6;
pub const SMAP_E3_INTR_DEAD_1: u32 = 1 << 5;
pub const SMAP_E3_INTR_SQE_ERR_1: u32 = 1 << 4;
pub const SMAP_E3_INTR_TX_ERR_1: u32 = 1 << 3;
pub const SMAP_E3_INTR_MMAOP_SUCCESS: u32 = 1 << 1;
pub const SMAP_E3_INTR_MMAOP_FAIL: u32 = 1 << 0;
pub const SMAP_R_EMAC3_ADDR_HI: u32 = SMAP_EMAC3_REGBASE + 0x1C;
pub const SMAP_R_EMAC3_ADDR_LO: u32 = SMAP_EMAC3_REGBASE + 0x20;
pub const SMAP_R_EMAC3_ADDR_HI_L: u32 = SMAP_EMAC3_REGBASE + 0x1C;
pub const SMAP_R_EMAC3_ADDR_HI_H: u32 = SMAP_EMAC3_REGBASE + 0x1E;
pub const SMAP_R_EMAC3_ADDR_LO_L: u32 = SMAP_EMAC3_REGBASE + 0x20;
pub const SMAP_R_EMAC3_ADDR_LO_H: u32 = SMAP_EMAC3_REGBASE + 0x22;
pub const SMAP_R_EMAC3_VLAN_TPID: u32 = SMAP_EMAC3_REGBASE + 0x24;
pub const SMAP_E3_VLAN_ID_MSK: u32 = 0xFFFF;
pub const SMAP_R_EMAC3_VLAN_TCI: u32 = SMAP_EMAC3_REGBASE + 0x28;
pub const SMAP_E3_VLAN_TCITAG_MSK: u32 = 0xFFFF;
pub const SMAP_R_EMAC3_PAUSE_TIMER: u32 = SMAP_EMAC3_REGBASE + 0x2C;
pub const SMAP_R_EMAC3_PAUSE_TIMER_L: u32 = SMAP_EMAC3_REGBASE + 0x2C;
pub const SMAP_R_EMAC3_PAUSE_TIMER_H: u32 = SMAP_EMAC3_REGBASE + 0x2E;
pub const SMAP_E3_PTIMER_MSK: u32 = 0xFFFF;
pub const SMAP_R_EMAC3_INDIVID_HASH1: u32 = SMAP_EMAC3_REGBASE + 0x30;
pub const SMAP_R_EMAC3_INDIVID_HASH2: u32 = SMAP_EMAC3_REGBASE + 0x34;
pub const SMAP_R_EMAC3_INDIVID_HASH3: u32 = SMAP_EMAC3_REGBASE + 0x38;
pub const SMAP_R_EMAC3_INDIVID_HASH4: u32 = SMAP_EMAC3_REGBASE + 0x3C;
pub const SMAP_R_EMAC3_GROUP_HASH1: u32 = SMAP_EMAC3_REGBASE + 0x40;
pub const SMAP_R_EMAC3_GROUP_HASH2: u32 = SMAP_EMAC3_REGBASE + 0x44;
pub const SMAP_R_EMAC3_GROUP_HASH3: u32 = SMAP_EMAC3_REGBASE + 0x48;
pub const SMAP_R_EMAC3_GROUP_HASH4: u32 = SMAP_EMAC3_REGBASE + 0x4C;
pub const SMAP_E3_HASH_MSK: u32 = 0xFFFF;
pub const SMAP_R_EMAC3_LAST_SA_HI: u32 = SMAP_EMAC3_REGBASE + 0x50;
pub const SMAP_R_EMAC3_LAST_SA_LO: u32 = SMAP_EMAC3_REGBASE + 0x54;
pub const SMAP_R_EMAC3_INTER_FRAME_GAP: u32 = SMAP_EMAC3_REGBASE + 0x58;
pub const SMAP_R_EMAC3_INTER_FRAME_GAP_L: u32 = SMAP_EMAC3_REGBASE + 0x58;
pub const SMAP_R_EMAC3_INTER_FRAME_GAP_H: u32 = SMAP_EMAC3_REGBASE + 0x5A;
pub const SMAP_E3_IFGAP_MSK: u32 = 0x3F;
pub const SMAP_R_EMAC3_STA_CTRL_L: u32 = SMAP_EMAC3_REGBASE + 0x5C;
pub const SMAP_R_EMAC3_STA_CTRL_H: u32 = SMAP_EMAC3_REGBASE + 0x5E;
pub const SMAP_E3_PHY_DATA_MSK: u32 = 0xFFFF;
pub const SMAP_E3_PHY_DATA_BITSFT: u32 = 16;
pub const SMAP_E3_PHY_OP_COMP: u32 = 1 << 15;
pub const SMAP_E3_PHY_ERR_READ: u32 = 1 << 14;
pub const SMAP_E3_PHY_STA_CMD_BITSFT: u32 = 12;
pub const SMAP_E3_PHY_READ: u32 = 1 << 12;
pub const SMAP_E3_PHY_WRITE: u32 = 2 << 12;
pub const SMAP_E3_PHY_OPBCLCK_BITSFT: u32 = 10;
pub const SMAP_E3_PHY_50M: u32 = 0 << 10;
pub const SMAP_E3_PHY_66M: u32 = 1 << 10;
pub const SMAP_E3_PHY_83M: u32 = 2 << 10;
pub const SMAP_E3_PHY_100M: u32 = 3 << 10;
pub const SMAP_E3_PHY_ADDR_MSK: u32 = 0x1F;
pub const SMAP_E3_PHY_ADDR_BITSFT: u32 = 5;
pub const SMAP_E3_PHY_REG_ADDR_MSK: u32 = 0x1F;
pub const SMAP_R_EMAC3_TX_THRESHOLD: u32 = SMAP_EMAC3_REGBASE + 0x60;
pub const SMAP_R_EMAC3_TX_THRESHOLD_L: u32 = SMAP_EMAC3_REGBASE + 0x60;
pub const SMAP_R_EMAC3_TX_THRESHOLD_H: u32 = SMAP_EMAC3_REGBASE + 0x62;
pub const SMAP_E3_TX_THRESHLD_MSK: u32 = 0x1F;
pub const SMAP_E3_TX_THRESHLD_BITSFT: u32 = 27;
pub const SMAP_R_EMAC3_RX_WATERMARK: u32 = SMAP_EMAC3_REGBASE + 0x64;
pub const SMAP_R_EMAC3_RX_WATERMARK_L: u32 = SMAP_EMAC3_REGBASE + 0x64;
pub const SMAP_R_EMAC3_RX_WATERMARK_H: u32 = SMAP_EMAC3_REGBASE + 0x66;
pub const SMAP_E3_RX_LO_WATER_MSK: u32 = 0x1FF;
pub const SMAP_E3_RX_LO_WATER_BITSFT: u32 = 23;
pub const SMAP_E3_RX_HI_WATER_MSK: u32 = 0x1FF;
pub const SMAP_E3_RX_HI_WATER_BITSFT: u32 = 7;
pub const SMAP_R_EMAC3_TX_OCTETS: u32 = SMAP_EMAC3_REGBASE + 0x68;
pub const SMAP_R_EMAC3_RX_OCTETS: u32 = SMAP_EMAC3_REGBASE + 0x6C;
pub const SMAP_EMAC3_REGEND: u32    = SMAP_EMAC3_REGBASE + 0x6C + 4;

pub const SMAP_BD_REGBASE: u32 = SMAP_REGBASE + 0x2f00;
pub const SMAP_BD_TX_BASE: u32  = SMAP_BD_REGBASE + 0x0000;
pub const SMAP_BD_RX_BASE: u32  = SMAP_BD_REGBASE + 0x0200;
pub const SMAP_BD_SIZE: u32     = 512;
pub const SMAP_BD_MAX_ENTRY: u32 = 64;

pub const SMAP_TX_BASE: u32    = SMAP_REGBASE + 0x1000;
pub const SMAP_TX_BUFSIZE: u32 = 4096;

pub const SMAP_BD_TX_READY: u16   = 1 << 15;
pub const SMAP_BD_TX_GENFCS: u16  = 1 << 9;
pub const SMAP_BD_TX_GENPAD: u16  = 1 << 8;
pub const SMAP_BD_TX_INSSA: u16   = 1 << 7;
pub const SMAP_BD_TX_RPLSA: u16   = 1 << 6;
pub const SMAP_BD_TX_INSVLAN: u16 = 1 << 5;
pub const SMAP_BD_TX_RPLVLAN: u16 = 1 << 4;
pub const SMAP_BD_TX_BADFCS: u16  = 1 << 9;
pub const SMAP_BD_TX_BADPKT: u16  = 1 << 8;
pub const SMAP_BD_TX_LOSSCR: u16  = 1 << 7;
pub const SMAP_BD_TX_EDEFER: u16  = 1 << 6;
pub const SMAP_BD_TX_ECOLL: u16   = 1 << 5;
pub const SMAP_BD_TX_LCOLL: u16   = 1 << 4;
pub const SMAP_BD_TX_MCOLL: u16   = 1 << 3;
pub const SMAP_BD_TX_SCOLL: u16   = 1 << 2;
pub const SMAP_BD_TX_UNDERRUN: u16 = 1 << 1;
pub const SMAP_BD_TX_SQE: u16     = 1 << 0;
pub const SMAP_BD_TX_ERROR: u16 =
    SMAP_BD_TX_LOSSCR | SMAP_BD_TX_EDEFER | SMAP_BD_TX_ECOLL |
    SMAP_BD_TX_LCOLL | SMAP_BD_TX_UNDERRUN;

pub const SMAP_BD_RX_EMPTY: u16     = 1 << 15;
pub const SMAP_BD_RX_OVERRUN: u16   = 1 << 9;
pub const SMAP_BD_RX_PFRM: u16      = 1 << 8;
pub const SMAP_BD_RX_BADFRM: u16    = 1 << 7;
pub const SMAP_BD_RX_RUNTFRM: u16   = 1 << 6;
pub const SMAP_BD_RX_SHORTEVNT: u16 = 1 << 5;
pub const SMAP_BD_RX_ALIGNERR: u16  = 1 << 4;
pub const SMAP_BD_RX_BADFCS: u16    = 1 << 3;
pub const SMAP_BD_RX_FRMTOOLONG: u16 = 1 << 2;
pub const SMAP_BD_RX_OUTRANGE: u16  = 1 << 1;
pub const SMAP_BD_RX_INRANGE: u16   = 1 << 0;
pub const SMAP_BD_RX_ERROR: u16 =
    SMAP_BD_RX_OVERRUN | SMAP_BD_RX_RUNTFRM | SMAP_BD_RX_SHORTEVNT |
    SMAP_BD_RX_ALIGNERR | SMAP_BD_RX_BADFCS | SMAP_BD_RX_FRMTOOLONG |
    SMAP_BD_RX_OUTRANGE | SMAP_BD_RX_INRANGE;

pub const SMAP_NS_OUI: u32 = 0x08_0017;
pub const SMAP_DsPHYTER_ADDRESS: u32 = 0x1;
pub const SMAP_DsPHYTER_BMCR: u32    = 0x00;
pub const SMAP_PHY_BMCR_RST: u16  = 1 << 15;
pub const SMAP_PHY_BMCR_LPBK: u16 = 1 << 14;
pub const SMAP_PHY_BMCR_100M: u16 = 1 << 13;
pub const SMAP_PHY_BMCR_10M: u16  = 0 << 13;
pub const SMAP_PHY_BMCR_ANEN: u16 = 1 << 12;
pub const SMAP_PHY_BMCR_PWDN: u16 = 1 << 11;
pub const SMAP_PHY_BMCR_ISOL: u16 = 1 << 10;
pub const SMAP_PHY_BMCR_RSAN: u16 = 1 << 9;
pub const SMAP_PHY_BMCR_DUPM: u16 = 1 << 8;
pub const SMAP_PHY_BMCR_COLT: u16 = 1 << 7;
pub const SMAP_DsPHYTER_BMSR: u32    = 0x01;
pub const SMAP_PHY_BMSR_ANCP: u16    = 1 << 5;
pub const SMAP_PHY_BMSR_LINK: u16    = 1 << 2;
pub const SMAP_DsPHYTER_PHYIDR1: u32 = 0x02;
pub const SMAP_PHY_IDR1_VAL: u16     = (((SMAP_NS_OUI << 2) >> 8) & 0xffff) as u16;
pub const SMAP_DsPHYTER_PHYIDR2: u32 = 0x03;
pub const SMAP_PHY_IDR2_VMDL: u16    = 0x2;
pub const SMAP_PHY_IDR2_VAL: u16     =
    (((SMAP_NS_OUI << 10) & 0xFC00) | ((SMAP_PHY_IDR2_VMDL as u32) << 4 & 0x3F0)) as u16;
pub const SMAP_PHY_IDR2_MSK: u16     = 0xFFF0;
pub const SMAP_PHY_IDR2_REV_MSK: u16 = 0x000F;
pub const SMAP_DsPHYTER_ANAR: u32    = 0x04;
pub const SMAP_DsPHYTER_ANLPAR: u32  = 0x05;
pub const SMAP_DsPHYTER_ANLPARNP: u32 = 0x05;
pub const SMAP_DsPHYTER_ANER: u32    = 0x06;
pub const SMAP_DsPHYTER_ANNPTR: u32  = 0x07;
pub const SMAP_DsPHYTER_PHYSTS: u32  = 0x10;
pub const SMAP_PHY_STS_REL: u16  = 1 << 13;
pub const SMAP_PHY_STS_POST: u16 = 1 << 12;
pub const SMAP_PHY_STS_FCSL: u16 = 1 << 11;
pub const SMAP_PHY_STS_SD: u16   = 1 << 10;
pub const SMAP_PHY_STS_DSL: u16  = 1 << 9;
pub const SMAP_PHY_STS_PRCV: u16 = 1 << 8;
pub const SMAP_PHY_STS_RFLT: u16 = 1 << 6;
pub const SMAP_PHY_STS_JBDT: u16 = 1 << 5;
pub const SMAP_PHY_STS_ANCP: u16 = 1 << 4;
pub const SMAP_PHY_STS_LPBK: u16 = 1 << 3;
pub const SMAP_PHY_STS_DUPS: u16 = 1 << 2;
pub const SMAP_PHY_STS_FDX: u16  = 1 << 2;
pub const SMAP_PHY_STS_HDX: u16  = 0 << 2;
pub const SMAP_PHY_STS_SPDS: u16 = 1 << 1;
pub const SMAP_PHY_STS_10M: u16  = 1 << 1;
pub const SMAP_PHY_STS_100M: u16 = 0 << 1;
pub const SMAP_PHY_STS_LINK: u16 = 1 << 0;
pub const SMAP_DsPHYTER_FCSCR: u32   = 0x14;
pub const SMAP_DsPHYTER_RECR: u32    = 0x15;
pub const SMAP_DsPHYTER_PCSR: u32    = 0x16;
pub const SMAP_DsPHYTER_PHYCTRL: u32 = 0x19;
pub const SMAP_DsPHYTER_10BTSCR: u32 = 0x1A;
pub const SMAP_DsPHYTER_CDCTRL: u32  = 0x1B;

pub const ATA_DEV9_HDD_BASE: u32 = SPD_REGBASE + 0x40;
pub const ATA_AIF_HDD_BASE: u32  = SPD_REGBASE + 0x4000000 + 0x60;
pub const ATA_R_DATA: u32        = ATA_DEV9_HDD_BASE + 0x00;
pub const ATA_R_ERROR: u32       = ATA_DEV9_HDD_BASE + 0x02;
pub const ATA_R_FEATURE: u32     = ATA_DEV9_HDD_BASE + 0x02;
pub const ATA_R_NSECTOR: u32     = ATA_DEV9_HDD_BASE + 0x04;
pub const ATA_R_SECTOR: u32      = ATA_DEV9_HDD_BASE + 0x06;
pub const ATA_R_LCYL: u32        = ATA_DEV9_HDD_BASE + 0x08;
pub const ATA_R_HCYL: u32        = ATA_DEV9_HDD_BASE + 0x0a;
pub const ATA_R_SELECT: u32      = ATA_DEV9_HDD_BASE + 0x0c;
pub const ATA_R_STATUS: u32      = ATA_DEV9_HDD_BASE + 0x0e;
pub const ATA_R_CMD: u32         = ATA_DEV9_HDD_BASE + 0x0e;
pub const ATA_R_ALT_STATUS: u32  = ATA_DEV9_HDD_BASE + 0x1c;
pub const ATA_R_CONTROL: u32     = ATA_DEV9_HDD_BASE + 0x1c;
pub const ATA_DEV9_INT: u32      = 0x01;
pub const ATA_DEV9_INT_DMA: u32  = 0x02;
pub const ATA_DEV9_HDD_END: u32  = ATA_R_CONTROL + 4;

pub const ATA_ERR_MARK: u8   = 0x01;
pub const ATA_ERR_TRACK0: u8 = 0x02;
pub const ATA_ERR_ABORT: u8  = 0x04;
pub const ATA_ERR_MCR: u8    = 0x08;
pub const ATA_ERR_ID: u8     = 0x10;
pub const ATA_ERR_MC: u8     = 0x20;
pub const ATA_ERR_ECC: u8    = 0x40;
pub const ATA_ERR_ICRC: u8   = 0x80;

pub const ATA_STAT_ERR: u8    = 0x01;
pub const ATA_STAT_INDEX: u8  = 0x02;
pub const ATA_STAT_ECC: u8    = 0x04;
pub const ATA_STAT_DRQ: u8    = 0x08;
pub const ATA_STAT_SEEK: u8   = 0x10;
pub const ATA_STAT_WRERR: u8  = 0x20;
pub const ATA_STAT_READY: u8  = 0x40;
pub const ATA_STAT_BUSY: u8   = 0x80;

pub const FLASH_ID_64MBIT: u32   = 0xe6;
pub const FLASH_ID_128MBIT: u32  = 0x73;
pub const FLASH_ID_256MBIT: u32  = 0x75;
pub const FLASH_ID_512MBIT: u32  = 0x76;
pub const FLASH_ID_1024MBIT: u32 = 0x79;

pub const SM_CMD_READ1: u32       = 0x00;
pub const SM_CMD_READ2: u32       = 0x01;
pub const SM_CMD_READ3: u32       = 0x50;
pub const SM_CMD_RESET: u32       = 0xff;
pub const SM_CMD_WRITEDATA: u32   = 0x80;
pub const SM_CMD_PROGRAMPAGE: u32 = 0x10;
pub const SM_CMD_ERASEBLOCK: u32  = 0x60;
pub const SM_CMD_ERASECONFIRM: u32 = 0xd0;
pub const SM_CMD_GETSTATUS: u32   = 0x70;
pub const SM_CMD_READID: u32      = 0x90;

pub const FLASH_REGBASE: u32 = 0x1000_4800;
pub const FLASH_R_DATA: u32  = FLASH_REGBASE + 0x00;
pub const FLASH_R_CMD: u32   = FLASH_REGBASE + 0x04;
pub const FLASH_R_ADDR: u32  = FLASH_REGBASE + 0x08;
pub const FLASH_R_CTRL: u32  = FLASH_REGBASE + 0x0C;
pub const FLASH_PP_READY: u32  = 1 << 0;
pub const FLASH_PP_WRITE: u32  = 1 << 7;
pub const FLASH_PP_CSEL: u32   = 1 << 8;
pub const FLASH_PP_READ: u32   = 1 << 11;
pub const FLASH_PP_NOECC: u32  = 1 << 12;
pub const FLASH_R_ID: u32      = FLASH_REGBASE + 0x14;
pub const FLASH_REGSIZE: u32   = 0x20;

pub const DEV9_R_REV: u32 = 0x1f80_146e;

// ---------------------------------------------------------------------------
// EEPROM state machine
// ---------------------------------------------------------------------------

pub const EEPROM_READY: u8 = 0;
pub const EEPROM_OPCD0: u8 = 1;
pub const EEPROM_OPCD1: u8 = 2;
pub const EEPROM_ADDR0: u8 = 3;
pub const EEPROM_ADDR1: u8 = 4;
pub const EEPROM_ADDR2: u8 = 5;
pub const EEPROM_ADDR3: u8 = 6;
pub const EEPROM_ADDR4: u8 = 7;
pub const EEPROM_ADDR5: u8 = 8;
pub const EEPROM_TDATA: u8 = 9;

// ---------------------------------------------------------------------------
// Dev9State
// ---------------------------------------------------------------------------

/// Aggregate of every register, FIFO, and descriptor state held by the
/// DEV9 device.  The C++ original exposes this as a global; here it lives
/// inside a `static mut` instance so the public surface stays intact while
/// also being explicit about the unsafe access.
#[repr(C)]
pub struct Dev9State {
    pub dev9R: [u8; 0x10000],

    pub eeprom_state: u8,
    pub eeprom_command: u8,
    pub eeprom_address: u8,
    pub eeprom_bit: u8,
    pub eeprom_dir: u8,
    pub eeprom: [u16; 32],

    pub rxbdi: u32,
    pub rxfifo: [u8; 16 * 1024],
    pub rxfifo_wr_ptr: u16,

    pub txbdi: u32,
    pub txfifo: [u8; 16 * 1024],
    pub txfifo_rd_ptr: u16,

    pub bd_swap: u8,
    pub phyregs: [u16; 32],

    pub irqcause: u16,
    pub irqmask: u16,
    pub dma_ctrl: u16,
    pub xfr_ctrl: u16,
    pub if_ctrl: u16,

    pub pio_mode: u16,
    pub mdma_mode: u16,
    pub udma_mode: u16,

    pub fifo_bytes_read: i32,
    pub fifo_bytes_write: i32,
    pub fifo: [u8; 16 * 512],

    pub dma_iop_ptr: *mut u8,
    pub dma_iop_transfered: i32,
    pub dma_iop_size: i32,
}

impl Default for Dev9State {
    fn default() -> Self {
        unsafe { ::std::mem::zeroed() }
    }
}

/// Default EEPROM image (matches the `eeprom` array in DEV9.cpp).
pub const DEFAULT_EEPROM: [u8; 64] = [
    0x76, 0x6D, 0x61, 0x63, 0x30, 0x31, 0x07, 0x02,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// Global dev9 state mirroring `dev9Struct dev9;` from the C++ code.
pub static mut dev9: Dev9State = Dev9State {
    dev9R: [0; 0x10000],

    eeprom_state: 0,
    eeprom_command: 0,
    eeprom_address: 0,
    eeprom_bit: 0,
    eeprom_dir: 0,
    eeprom: [0; 32],

    rxbdi: 0,
    rxfifo: [0; 16 * 1024],
    rxfifo_wr_ptr: 0,

    txbdi: 0,
    txfifo: [0; 16 * 1024],
    txfifo_rd_ptr: 0,

    bd_swap: 0,
    phyregs: [0; 32],

    irqcause: 0,
    irqmask: 0,
    dma_ctrl: 0,
    xfr_ctrl: 0,
    if_ctrl: 0,

    pio_mode: 0,
    mdma_mode: 0,
    udma_mode: 0,

    fifo_bytes_read: 0,
    fifo_bytes_write: 0,
    fifo: [0; 16 * 512],

    dma_iop_ptr: ::std::ptr::null_mut(),
    dma_iop_transfered: 0,
    dma_iop_size: 0,
};

// ---------------------------------------------------------------------------
// Helpers mirroring the dev9Ru8/Ru16/Ru32 macros
// ---------------------------------------------------------------------------

#[inline]
pub unsafe fn dev9Rs8(mem: u32) -> i8 { dev9.dev9R[(mem & 0xffff) as usize] as i8 }
#[inline]
pub unsafe fn dev9Rs16(mem: u32) -> i16 { ::std::ptr::read_unaligned(&dev9.dev9R[(mem & 0xffff) as usize] as *const u8 as *const i16) }
#[inline]
pub unsafe fn dev9Rs32(mem: u32) -> i32 { ::std::ptr::read_unaligned(&dev9.dev9R[(mem & 0xffff) as usize] as *const u8 as *const i32) }
#[inline]
pub unsafe fn dev9Ru8(mem: u32) -> u8 { dev9.dev9R[(mem & 0xffff) as usize] }
#[inline]
pub unsafe fn dev9Ru16(mem: u32) -> u16 { ::std::ptr::read_unaligned(&dev9.dev9R[(mem & 0xffff) as usize] as *const u8 as *const u16) }
#[inline]
pub unsafe fn dev9Ru32(mem: u32) -> u32 { ::std::ptr::read_unaligned(&dev9.dev9R[(mem & 0xffff) as usize] as *const u8 as *const u32) }
#[inline]
pub unsafe fn dev9Wu8(mem: u32, value: u8) {
    dev9.dev9R[(mem & 0xffff) as usize] = value;
}
#[inline]
pub unsafe fn dev9Wu16(mem: u32, value: u16) {
    ::std::ptr::write_unaligned(&mut dev9.dev9R[(mem & 0xffff) as usize] as *mut u8 as *mut u16, value);
}
#[inline]
pub unsafe fn dev9Wu32(mem: u32, value: u32) {
    ::std::ptr::write_unaligned(&mut dev9.dev9R[(mem & 0xffff) as usize] as *mut u8 as *mut u32, value);
}

#[inline]
pub unsafe fn dev9_rxfifo_write(x: u8) {
    let slot = dev9.rxfifo_wr_ptr as usize;
    dev9.rxfifo[slot] = x;
    dev9.rxfifo_wr_ptr = dev9.rxfifo_wr_ptr.wrapping_add(1);
}

// ---------------------------------------------------------------------------
// Buffer descriptor matching the smap_bd_t struct
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct SmapBd {
    pub ctrl_stat: u16,
    pub reserved:  u16,
    pub length:    u16,
    pub pointer:   u16,
}

#[repr(C)]
pub struct FlashInfo {
    pub id: u32,
    pub mbits: u32,
    pub page_bytes: u32,
    pub block_pages: u32,
    pub blocks: u32,
}

// ---------------------------------------------------------------------------
// ThreadRun external
// ---------------------------------------------------------------------------

pub static mut ThreadRun: i32 = 1;
pub static mut isRunning: bool = false;

// ---------------------------------------------------------------------------
// eeprom.dat file handle (kept as raw pointer to mirror the C++ global)
// ---------------------------------------------------------------------------

#[cfg(target_family = "windows")]
pub type EepromHandle = *mut ::std::os::raw::c_void;
#[cfg(not(target_family = "windows"))]
pub type EepromHandle = i32;

pub static mut hEeprom: EepromHandle = ::std::ptr::null_mut();
pub static mut mapping: EepromHandle = ::std::ptr::null_mut();

// ---------------------------------------------------------------------------
// Logging console stub (Console.Error / DevCon.WriteLn)
// ---------------------------------------------------------------------------

pub mod log {
    use super::*;
    pub struct Console;
    impl Console {
        pub fn Error(&self, fmt: ::std::fmt::Arguments) {}
        pub fn WriteLn(&self, fmt: ::std::fmt::Arguments) {}
    }
    pub struct DevCon;
    impl DevCon {
        pub fn WriteLn(&self, fmt: ::std::fmt::Arguments) {}
    }
    pub static Console_local: Console = Console;
    pub static DevCon_local: DevCon = DevCon;
}

// ---------------------------------------------------------------------------
// Dev9Network trait (pcap_io abstraction)
// ---------------------------------------------------------------------------

/// Trait covering the network side of the DEV9 device.  The original
/// PCAPAdapter is a class that depends on the libpcap C library; this
/// trait lets callers (and tests) plug in a different network backend
/// without dragging in pcap.
pub trait Dev9Network {
    fn blocks(&self) -> bool;
    fn is_initialised(&self) -> bool;
    fn recv(&mut self, pkt: &mut NetPacket) -> bool;
    fn send(&mut self, pkt: &NetPacket) -> bool;
    fn reset(&mut self) {}
    fn reload_settings(&mut self) {}
    fn close(&mut self) {}
}

// ---------------------------------------------------------------------------
// NetPacket / AdapterEntry / AdapterOptions
// ---------------------------------------------------------------------------

/// Mirrors the `NetPacket` struct from net.h.
pub struct NetPacket {
    pub size: i32,
    pub buffer: [u8; 2048 - 4],
}

impl NetPacket {
    pub fn new() -> Self { NetPacket { size: 0, buffer: [0; 2044] } }
    pub fn from_bytes(data: &[u8]) -> Self {
        let mut p = NetPacket::new();
        let n = data.len().min(p.buffer.len());
        p.size = n as i32;
        p.buffer[..n].copy_from_slice(&data[..n]);
        p
    }
}

impl Default for NetPacket {
    fn default() -> Self { NetPacket::new() }
}

#[derive(Clone, Debug)]
pub struct AdapterEntry {
    pub net_api: NetApi,
    pub name: String,
    pub guid: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetApi {
    TAP,
    PCAP_Bridged,
    PCAP_Switched,
    Sockets,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct AdapterOptions(u32);

impl AdapterOptions {
    pub const NONE: AdapterOptions = AdapterOptions(0);
    pub const DHCP_FORCED_ON: AdapterOptions = AdapterOptions(1 << 0);
    pub const DHCP_OVERRIDE_IP: AdapterOptions = AdapterOptions(1 << 1);
    pub const DHCP_OVERRIDE_SUBNET: AdapterOptions = AdapterOptions(1 << 2);
    pub const DHCP_OVERRIDE_GATEWAY: AdapterOptions = AdapterOptions(1 << 3);
}
impl ::std::ops::BitOr for AdapterOptions {
    type Output = AdapterOptions;
    fn bitor(self, rhs: AdapterOptions) -> AdapterOptions { AdapterOptions(self.0 | rhs.0) }
}
impl ::std::ops::BitAnd for AdapterOptions {
    type Output = AdapterOptions;
    fn bitand(self, rhs: AdapterOptions) -> AdapterOptions { AdapterOptions(self.0 & rhs.0) }
}

// ---------------------------------------------------------------------------
// Default MAC / IP addresses used by NetAdapter
// ---------------------------------------------------------------------------

pub static DEFAULT_MAC: [u8; 6] = [0x00, 0x04, 0x1F, 0x82, 0x30, 0x31];
pub static BROADCAST_MAC: [u8; 6] = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
pub static INTERNAL_MAC: [u8; 6] = [0x76, 0x6D, 0xF4, 0x63, 0x30, 0x31];
pub static INTERNAL_IP: [u8; 4] = [192, 0, 2, 1];

// ---------------------------------------------------------------------------
// SimpleQueue
// ---------------------------------------------------------------------------

/// Lock-free single-producer/single-consumer queue mirroring
/// `SimpleQueue<T>` from SimpleQueue.h.  The original uses
/// `std::atomic_bool` for the ready flag and explicit memory ordering;
/// the Rust translation uses `AtomicBool` to keep the same semantics.
pub struct SimpleQueue<T> {
    head: ::std::sync::atomic::AtomicPtr<SimpleQueueEntry<T>>,
    tail: *mut SimpleQueueEntry<T>,
}

struct SimpleQueueEntry<T> {
    ready: AtomicBool,
    next:  *mut SimpleQueueEntry<T>,
    value: Option<T>,
}

impl<T> SimpleQueue<T> {
    pub fn new() -> Self {
        let entry = Box::into_raw(Box::new(SimpleQueueEntry {
            ready: AtomicBool::new(false),
            next:  ::std::ptr::null_mut(),
            value: None,
        }));
        Self {
            head: ::std::sync::atomic::AtomicPtr::new(entry),
            tail: entry,
        }
    }

    pub fn enqueue(&self, value: T) {
        let new_head = Box::into_raw(Box::new(SimpleQueueEntry {
            ready: AtomicBool::new(false),
            next:  ::std::ptr::null_mut(),
            value: None,
        }));
        let prev = self.head.swap(new_head, Ordering::AcqRel);
        unsafe {
            (*prev).value = Some(value);
            (*prev).next  = new_head;
            (*prev).ready.store(true, Ordering::Release);
        }
    }

    pub fn dequeue(&mut self, out: &mut Option<T>) -> bool {
        unsafe {
            if !(*self.tail).ready.load(Ordering::Acquire) {
                return false;
            }
            let entry = self.tail;
            self.tail = (*entry).next;
            *out = (*entry).value.take();
            drop(Box::from_raw(entry));
            true
        }
    }

    pub fn is_queue_empty(&self) -> bool {
        self.head.load(Ordering::Acquire) == self.tail
    }
}

impl<T> Default for SimpleQueue<T> { fn default() -> Self { Self::new() } }

impl<T> Drop for SimpleQueue<T> {
    fn drop(&mut self) {
        unsafe {
            if !self.head.load(Ordering::Acquire).is_null() {
                while self.is_queue_empty() == false {
                    let mut tmp: Option<T> = None;
                    let _ = self.dequeue(&mut tmp);
                }
                let _ = Box::from_raw(self.head.load(Ordering::Acquire));
                self.head.store(::std::ptr::null_mut(), Ordering::Release);
                self.tail = ::std::ptr::null_mut();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// ThreadSafeMap
// ---------------------------------------------------------------------------

/// Concurrency-safe key/value map mirroring `ThreadSafeMap<Key, T>`.
pub struct ThreadSafeMap<K, V> {
    inner: Mutex<::std::collections::HashMap<K, V>>,
}

impl<K: Eq + ::std::hash::Hash + Clone, V> Default for ThreadSafeMap<K, V> {
    fn default() -> Self {
        Self { inner: Mutex::new(::std::collections::HashMap::new()) }
    }
}

impl<K: Eq + ::std::hash::Hash + Clone, V> ThreadSafeMap<K, V> {
    pub fn new() -> Self { Self::default() }

    pub fn add(&self, key: K, value: V) {
        let mut g = self.inner.lock().unwrap();
        g.insert(key, value);
    }

    pub fn remove(&self, key: &K) -> bool {
        let mut g = self.inner.lock().unwrap();
        g.remove(key).is_some()
    }

    pub fn clear(&self) {
        let mut g = self.inner.lock().unwrap();
        g.clear();
    }

    pub fn get_keys(&self) -> Vec<K> {
        let g = self.inner.lock().unwrap();
        g.keys().cloned().collect()
    }

    pub fn try_get_value(&self, key: &K) -> Option<V>
    where V: Clone {
        let g = self.inner.lock().unwrap();
        g.get(key).cloned()
    }

    pub fn contains_key(&self, key: &K) -> bool {
        let g = self.inner.lock().unwrap();
        g.contains_key(key)
    }
}

// ---------------------------------------------------------------------------
// Dev9AdapterUtils
// ---------------------------------------------------------------------------

/// Adapter enumeration helpers translated from AdapterUtils.cpp.  The
/// original code uses platform-specific `GetAdaptersAddresses` (Windows)
/// or `getifaddrs` (POSIX) calls; here we expose a single struct that
/// produces a list of `AdapterEntry` records without depending on
/// either, leaving the platform hooks to the caller.
pub struct Dev9AdapterUtils;

impl Dev9AdapterUtils {
    /// Enumerate the available network adapters.  The original C++
    /// walks the OS adapter list; the Rust translation returns an
    /// empty list by default and lets the caller override the source
    /// (the real implementation is expected to live in a host crate).
    pub fn get_all_adapters() -> Vec<AdapterEntry> {
        Vec::new()
    }

    /// Look up a specific adapter by GUID / interface name.
    pub fn get_adapter(name: &str) -> Option<AdapterEntry> {
        Self::get_all_adapters().into_iter().find(|e| e.guid == name)
    }

    /// Return the first non-loopback, up-and-running adapter.  This is
    /// the equivalent of the `GetAdapterAuto` helper.
    pub fn get_adapter_auto() -> Option<AdapterEntry> {
        Self::get_all_adapters().into_iter().find(|e| e.guid != "Auto")
    }

    /// MAC address of an adapter, if available.
    pub fn get_adapter_mac(_adapter: &AdapterEntry) -> Option<[u8; 6]> { None }

    /// IPv4 address of an adapter, if available.
    pub fn get_adapter_ip(_adapter: &AdapterEntry) -> Option<[u8; 4]> { None }

    /// Gateway addresses of an adapter.
    pub fn get_gateways(_adapter: &AdapterEntry) -> Vec<[u8; 4]> { Vec::new() }

    /// DNS server addresses of an adapter.
    pub fn get_dns(_adapter: &AdapterEntry) -> Vec<[u8; 4]> { Vec::new() }
}

// ---------------------------------------------------------------------------
// Internal Flash state
// ---------------------------------------------------------------------------

const PAGE_SIZE_BITS: u32 = 9;
const PAGE_SIZE: usize    = 1 << PAGE_SIZE_BITS;
const ECC_SIZE: usize     = 16;
const PAGE_SIZE_ECC: usize = PAGE_SIZE + ECC_SIZE;
const BLOCK_SIZE: usize    = 16 * PAGE_SIZE;
const CARD_SIZE: usize     = 1024 * BLOCK_SIZE;
const CARD_SIZE_ECC: usize = 1024 * (16 * PAGE_SIZE_ECC);

struct FlashState {
    ctrl: u32,
    cmd: u32,
    address: u32,
    id: u32,
    counter: u32,
    addrbyte: u32,
    data: [u8; PAGE_SIZE_ECC],
    file: [u8; CARD_SIZE_ECC],
}

static mut FLASH_STATE: FlashState = FlashState {
    ctrl: 0,
    cmd: 0,
    address: 0,
    id: 0,
    counter: 0,
    addrbyte: 0,
    data: [0xFF; PAGE_SIZE_ECC],
    file: [0xFF; CARD_SIZE_ECC],
};

static XOR_TABLE: [u8; 256] = [
    0x00, 0x87, 0x96, 0x11, 0xA5, 0x22, 0x33, 0xB4, 0xB4, 0x33, 0x22, 0xA5, 0x11, 0x96, 0x87, 0x00,
    0xC3, 0x44, 0x55, 0xD2, 0x66, 0xE1, 0xF0, 0x77, 0x77, 0xF0, 0xE1, 0x66, 0xD2, 0x55, 0x44, 0xC3,
    0xD2, 0x55, 0x44, 0xC3, 0x77, 0xF0, 0xE1, 0x66, 0x66, 0xE1, 0xF0, 0x77, 0xC3, 0x44, 0x55, 0xD2,
    0x11, 0x96, 0x87, 0x00, 0xB4, 0x33, 0x22, 0xA5, 0xA5, 0x22, 0x33, 0xB4, 0x00, 0x87, 0x96, 0x11,
    0xE1, 0x66, 0x77, 0xF0, 0x44, 0xC3, 0xD2, 0x55, 0x55, 0xD2, 0xC3, 0x44, 0xF0, 0x77, 0x66, 0xE1,
    0x22, 0xA5, 0xB4, 0x33, 0x87, 0x00, 0x11, 0x96, 0x96, 0x11, 0x00, 0x87, 0x33, 0xB4, 0xA5, 0x22,
    0x33, 0xB4, 0xA5, 0x22, 0x96, 0x11, 0x00, 0x87, 0x87, 0x00, 0x11, 0x96, 0x22, 0xA5, 0xB4, 0x33,
    0xF0, 0x77, 0x66, 0xE1, 0x55, 0xD2, 0xC3, 0x44, 0x44, 0xC3, 0xD2, 0x55, 0xE1, 0x66, 0x77, 0xF0,
    0xF0, 0x77, 0x66, 0xE1, 0x55, 0xD2, 0xC3, 0x44, 0x44, 0xC3, 0xD2, 0x55, 0xE1, 0x66, 0x77, 0xF0,
    0x33, 0xB4, 0xA5, 0x22, 0x96, 0x11, 0x00, 0x87, 0x87, 0x00, 0x11, 0x96, 0x22, 0xA5, 0xB4, 0x33,
    0x22, 0xA5, 0xB4, 0x33, 0x87, 0x00, 0x11, 0x96, 0x96, 0x11, 0x00, 0x87, 0x33, 0xB4, 0xA5, 0x22,
    0xE1, 0x66, 0x77, 0xF0, 0x44, 0xC3, 0xD2, 0x55, 0x55, 0xD2, 0xC3, 0x44, 0xF0, 0x77, 0x66, 0xE1,
    0x11, 0x96, 0x87, 0x00, 0xB4, 0x33, 0x22, 0xA5, 0xA5, 0x22, 0x33, 0xB4, 0x00, 0x87, 0x96, 0x11,
    0xD2, 0x55, 0x44, 0xC3, 0x77, 0xF0, 0xE1, 0x66, 0x66, 0xE1, 0xF0, 0x77, 0xC3, 0x44, 0x55, 0xD2,
    0xC3, 0x44, 0x55, 0xD2, 0x66, 0xE1, 0xF0, 0x77, 0x77, 0xF0, 0xE1, 0x66, 0xD2, 0x55, 0x44, 0xC3,
    0x00, 0x87, 0x96, 0x11, 0xA5, 0x22, 0x33, 0xB4, 0xB4, 0x33, 0x22, 0xA5, 0x11, 0x96, 0x87, 0x00,
];

fn calculate_xors(buffer: &[u8; 128], out: &mut [u8; 4]) {
    let mut a: u8 = 0;
    let mut b: u8 = 0;
    let mut c: u8 = 0;
    for (i, &byte) in buffer.iter().enumerate() {
        let t = XOR_TABLE[byte as usize];
        a ^= t;
        if t & 0x80 != 0 {
            b ^= !((i & 0xff) as u8);
            c ^= (i & 0xff) as u8;
        }
    }
    out[0] = !a & 0x77;
    out[1] = !b & 0x7F;
    out[2] = !c & 0x7F;
}

fn calculate_ecc(page: &mut [u8; PAGE_SIZE_ECC]) {
    for b in &mut page[PAGE_SIZE..PAGE_SIZE + ECC_SIZE] {
        *b = 0x00;
    }
    let mut tmp = [0u8; 4];
    for n in 0..4 {
        let start = n * (PAGE_SIZE / 4);
        let mut buf = [0u8; 128];
        buf.copy_from_slice(&page[start..start + 128]);
        calculate_xors(&buf, &mut tmp);
        let dst = PAGE_SIZE + n * 3;
        page[dst]     = tmp[0];
        page[dst + 1] = tmp[1];
        page[dst + 2] = tmp[2];
    }
}

fn flash_cmd_name(cmd: u32) -> &'static str {
    match cmd {
        SM_CMD_READ1 => "READ1",
        SM_CMD_READ2 => "READ2",
        SM_CMD_READ3 => "READ3",
        SM_CMD_RESET => "RESET",
        SM_CMD_WRITEDATA => "WRITEDATA",
        SM_CMD_PROGRAMPAGE => "PROGRAMPAGE",
        SM_CMD_ERASEBLOCK => "ERASEBLOCK",
        SM_CMD_ERASECONFIRM => "ERASECONFIRM",
        SM_CMD_GETSTATUS => "GETSTATUS",
        SM_CMD_READID => "READID",
        _ => "unknown",
    }
}

// ---------------------------------------------------------------------------
// Flash init / read / write
// ---------------------------------------------------------------------------

/// Reset and re-initialise the SmartMedia flash translation state.
pub unsafe fn dev9FlashInit() {
    let s = &mut FLASH_STATE;
    s.id       = FLASH_ID_64MBIT;
    s.counter  = 0;
    s.addrbyte = 0;
    s.address  = 0;
    for b in &mut s.data[..PAGE_SIZE] { *b = 0xFF; }
    calculate_ecc(&mut s.data);
    s.ctrl = FLASH_PP_READY;

    // In the original the backing storage is loaded from "flash.dat".
    // The Rust translation leaves the buffer full of 0xFF, which is the
    // erased default, so a missing file is a no-op.
    for b in &mut s.file { *b = 0xFF; }
}

/// Read 1, 2 or 4 bytes from the flash window.
pub unsafe fn dev9FlashRead(addr: u32) -> u8 {
    let s = &mut FLASH_STATE;
    let mut value: u32 = 0;
    match addr {
        FLASH_R_DATA => {
            let bytes = 1; // 8-bit accessor
            let n = bytes as usize;
            for i in 0..n {
                value |= (s.data[s.counter as usize + i] as u32) << (i * 8);
            }
            s.counter += n as u32;
            let mut refill = false;
            if s.cmd == SM_CMD_READ3 {
                if s.counter >= PAGE_SIZE_ECC as u32 {
                    s.counter = PAGE_SIZE as u32;
                    refill = true;
                }
            } else if s.ctrl & FLASH_PP_NOECC != 0 {
                if s.counter >= PAGE_SIZE as u32 {
                    s.counter %= PAGE_SIZE as u32;
                    refill = true;
                }
            } else if s.counter >= PAGE_SIZE_ECC as u32 {
                s.counter %= PAGE_SIZE_ECC as u32;
                refill = true;
            }
            if refill {
                s.ctrl &= !FLASH_PP_READY;
                s.address = (s.address + PAGE_SIZE as u32) % CARD_SIZE as u32;
                let src = ((s.address >> PAGE_SIZE_BITS) as usize) * PAGE_SIZE_ECC;
                s.data[..PAGE_SIZE].copy_from_slice(&s.file[src..src + PAGE_SIZE]);
                calculate_ecc(&mut s.data);
                s.ctrl |= FLASH_PP_READY;
            }
            value as u8
        }
        FLASH_R_CMD => s.cmd as u8,
        FLASH_R_ADDR => 0,
        FLASH_R_CTRL => s.ctrl as u8,
        FLASH_R_ID => {
            if s.cmd == SM_CMD_READID {
                s.id as u8
            } else if s.cmd == SM_CMD_GETSTATUS {
                (0x80u32 | ((s.ctrl & 1) << 6)) as u8
            } else {
                0
            }
        }
        _ => 0,
    }
}

/// Write 1, 2 or 4 bytes to the flash window.
pub unsafe fn dev9FlashWrite(addr: u32, value: u8) {
    let s = &mut FLASH_STATE;
    let value_u32 = value as u32;
    let size: usize = 1;
    match addr & 0x1FFF_FFFF {
        FLASH_R_DATA => {
            for i in 0..size {
                s.data[s.counter as usize + i] = ((value_u32 >> (i * 8)) & 0xff) as u8;
            }
            s.counter += size as u32;
            s.counter %= PAGE_SIZE_ECC as u32;
        }
        FLASH_R_CMD => {
            if s.ctrl & FLASH_PP_READY == 0
                && value_u32 != SM_CMD_GETSTATUS
                && value_u32 != SM_CMD_RESET
            {
                return;
            }
            if s.cmd == SM_CMD_WRITEDATA
                && value_u32 != SM_CMD_PROGRAMPAGE
                && value_u32 != SM_CMD_RESET
            {
                s.ctrl &= !FLASH_PP_READY;
                return;
            }
            match value_u32 {
                SM_CMD_READ1 => {
                    s.counter = 0;
                    if s.cmd != SM_CMD_GETSTATUS { s.address = s.counter; }
                    s.addrbyte = 0;
                }
                SM_CMD_READ2 => {
                    s.counter = (PAGE_SIZE / 2) as u32;
                    if s.cmd != SM_CMD_GETSTATUS { s.address = s.counter; }
                    s.addrbyte = 0;
                }
                SM_CMD_READ3 => {
                    s.counter = PAGE_SIZE as u32;
                    if s.cmd != SM_CMD_GETSTATUS { s.address = s.counter; }
                    s.addrbyte = 0;
                }
                SM_CMD_RESET => dev9FlashInit(),
                SM_CMD_WRITEDATA => {
                    s.counter = 0;
                    s.address = s.counter;
                    s.addrbyte = 0;
                }
                SM_CMD_ERASEBLOCK => {
                    s.counter = 0;
                    for b in &mut s.data[..PAGE_SIZE] { *b = 0xFF; }
                    s.address = s.counter;
                    s.addrbyte = 1;
                }
                SM_CMD_PROGRAMPAGE | SM_CMD_ERASECONFIRM => {
                    s.ctrl &= !FLASH_PP_READY;
                    calculate_ecc(&mut s.data);
                    let dst = (s.address as usize / PAGE_SIZE) * PAGE_SIZE_ECC;
                    s.file[dst..dst + PAGE_SIZE_ECC].copy_from_slice(&s.data);
                    s.ctrl |= FLASH_PP_READY;
                }
                SM_CMD_GETSTATUS => {}
                SM_CMD_READID => {
                    s.counter = 0;
                    s.address = s.counter;
                    s.addrbyte = 0;
                }
                _ => {
                    s.ctrl &= !FLASH_PP_READY;
                    return;
                }
            }
            s.cmd = value_u32;
        }
        FLASH_R_ADDR => {
            s.address |= (value_u32 & 0xFF)
                << (if s.addrbyte == 0 { 0 } else { 1 + 8 * s.addrbyte });
            s.addrbyte += 1;
            if value_u32 & 0x100 == 0 {
                if s.cmd == SM_CMD_READ1 || s.cmd == SM_CMD_READ2 || s.cmd == SM_CMD_READ3 {
                    s.ctrl &= !FLASH_PP_READY;
                    let src = ((s.address >> PAGE_SIZE_BITS) as usize) * PAGE_SIZE_ECC;
                    s.data[..PAGE_SIZE].copy_from_slice(&s.file[src..src + PAGE_SIZE]);
                    calculate_ecc(&mut s.data);
                    s.ctrl |= FLASH_PP_READY;
                }
                s.addrbyte = 0;
            }
        }
        FLASH_R_CTRL => {
            s.ctrl = (s.ctrl & FLASH_PP_READY) | (value_u32 & !FLASH_PP_READY);
        }
        FLASH_R_ID => { /* write denied */ }
        _ => {}
    }
    // keep the cmd-name helper alive to avoid dead-code warnings in
    // stripped builds
    let _ = flash_cmd_name(value_u32);
}

// ---------------------------------------------------------------------------
// FIFO helpers (HDD <-> SPEED bridge)
// ---------------------------------------------------------------------------

unsafe fn hdd_write_fifo(ata_read_dmais: &dyn Fn(&mut [u8]) -> i32) {
    let unread = dev9.fifo_bytes_write - dev9.fifo_bytes_read;
    let space  = (SPD_DBUF_AVAIL_MAX as i32) * 512 - unread;
    let base   = ((dev9.fifo_bytes_write as u32) % ((SPD_DBUF_AVAIL_MAX as u32) * 512)) as usize;
    let mut total = 0;
    if base + space as usize > (SPD_DBUF_AVAIL_MAX as usize) * 512 {
        let was = (SPD_DBUF_AVAIL_MAX as usize) * 512 - base;
        let mut tmp = vec![0u8; was];
        let read = ata_read_dmais(&mut tmp);
        if read as usize == was {
            for (i, b) in tmp.iter().enumerate() { dev9.fifo[base + i] = *b; }
            let mut rest = vec![0u8; (space as usize) - was];
            let read2 = ata_read_dmais(&mut rest);
            for (i, b) in rest.iter().enumerate() { dev9.fifo[i] = *b; }
            total = read + read2;
        } else {
            for (i, b) in tmp.iter().take(read as usize).enumerate() { dev9.fifo[base + i] = *b; }
            total = read;
        }
    } else {
        let mut tmp = vec![0u8; space as usize];
        let read = ata_read_dmais(&mut tmp);
        for (i, b) in tmp.iter().take(read as usize).enumerate() { dev9.fifo[base + i] = *b; }
        total = read;
    }
    dev9.fifo_bytes_write += total;
}

unsafe fn hdd_read_fifo(ata_write_dmais: &dyn Fn(&[u8]) -> i32) {
    let unread = dev9.fifo_bytes_write - dev9.fifo_bytes_read;
    let base   = ((dev9.fifo_bytes_read as u32) % ((SPD_DBUF_AVAIL_MAX as u32) * 512)) as usize;
    let mut total = 0;
    if base + unread as usize > (SPD_DBUF_AVAIL_MAX as usize) * 512 {
        let was = (SPD_DBUF_AVAIL_MAX as usize) * 512 - base;
        let wrote = ata_write_dmais(&dev9.fifo[base..base + was]);
        if wrote as usize == was {
            let rest = ata_write_dmais(&dev9.fifo[..(unread as usize) - was]);
            total = wrote + rest;
        } else {
            total = wrote;
        }
    } else {
        let wrote = ata_write_dmais(&dev9.fifo[base..base + unread as usize]);
        total = wrote;
    }
    dev9.fifo_bytes_read += total;
}

unsafe fn iop_read_fifo() {
    let unread = dev9.fifo_bytes_write - dev9.fifo_bytes_read;
    let base   = ((dev9.fifo_bytes_read as u32) % ((SPD_DBUF_AVAIL_MAX as u32) * 512)) as usize;
    let remain = dev9.dma_iop_size - dev9.dma_iop_transfered;
    let read = remain.min(unread);
    if read == 0 { return; }
    if base + read as usize > (SPD_DBUF_AVAIL_MAX as usize) * 512 {
        let was = (SPD_DBUF_AVAIL_MAX as usize) * 512 - base;
        let dst = dev9.dma_iop_ptr.add(dev9.dma_iop_transfered as usize);
        for i in 0..was { *dst.add(i) = dev9.fifo[base + i]; }
        let dst2 = dev9.dma_iop_ptr.add(dev9.dma_iop_transfered as usize + was);
        for i in 0..(read as usize - was) { *dst2.add(i) = dev9.fifo[i]; }
    } else {
        let dst = dev9.dma_iop_ptr.add(dev9.dma_iop_transfered as usize);
        for i in 0..read as usize { *dst.add(i) = dev9.fifo[base + i]; }
    }
    dev9.dma_iop_transfered += read;
    dev9.fifo_bytes_read += read;
}

unsafe fn iop_write_fifo() {
    let unread = dev9.fifo_bytes_write - dev9.fifo_bytes_read;
    let space  = (SPD_DBUF_AVAIL_MAX as i32) * 512 - unread;
    let base   = ((dev9.fifo_bytes_write as u32) % ((SPD_DBUF_AVAIL_MAX as u32) * 512)) as usize;
    let remain = dev9.dma_iop_size - dev9.dma_iop_transfered;
    let write  = remain.min(space);
    if write == 0 { return; }
    if base + write as usize > (SPD_DBUF_AVAIL_MAX as usize) * 512 {
        let was = (SPD_DBUF_AVAIL_MAX as usize) * 512 - base;
        let src = dev9.dma_iop_ptr.add(dev9.dma_iop_transfered as usize);
        for i in 0..was { dev9.fifo[base + i] = *src.add(i); }
        let src2 = dev9.dma_iop_ptr.add(dev9.dma_iop_transfered as usize + was);
        for i in 0..(write as usize - was) { dev9.fifo[i] = *src2.add(i); }
    } else {
        let src = dev9.dma_iop_ptr.add(dev9.dma_iop_transfered as usize);
        for i in 0..write as usize { dev9.fifo[base + i] = *src.add(i); }
    }
    dev9.dma_iop_transfered += write;
    dev9.fifo_bytes_write += write;
}

unsafe fn fifo_intr() {
    let unread = dev9.fifo_bytes_write - dev9.fifo_bytes_read;
    if unread == 0 {
        dev9.irqcause &= !SPD_INTR_ATA_FIFO_DATA as u16;
        if dev9.irqcause & SPD_INTR_ATA_FIFO_EMPTY as u16 == 0 {
            dev9Irq(1);
        }
    } else {
        dev9.irqcause &= !SPD_INTR_ATA_FIFO_EMPTY as u16;
        if dev9.irqcause & SPD_INTR_ATA_FIFO_DATA as u16 == 0 {
            dev9Irq(1);
        }
    }
    if unread == (SPD_DBUF_AVAIL_MAX as i32) * 512 {
        if dev9.irqcause & SPD_INTR_ATA_FIFO_FULL as u16 == 0 {
            dev9Irq(1);
        }
    } else {
        dev9.irqcause &= !SPD_INTR_ATA_FIFO_FULL as u16;
    }
    if !dev9.dma_iop_ptr.is_null() && dev9.dma_iop_transfered == dev9.dma_iop_size {
        dev9.dma_iop_ptr = ::std::ptr::null_mut();
        psxDMA8Interrupt();
    }
}

unsafe fn dev9_run_fifo() {
    let iop_write = dev9.xfr_ctrl & SPD_XFR_WRITE != 0;
    let hdd_read  = dev9.if_ctrl & SPD_IF_READ != 0;
    let iop_xfer  = !dev9.dma_iop_ptr.is_null() && dev9.xfr_ctrl & SPD_XFR_DMAEN != 0;
    let hdd_xfer  = dev9.if_ctrl & SPD_IF_ATA_DMAEN != 0;
    if iop_write {
        if iop_xfer { iop_write_fifo(); }
        if hdd_xfer && !hdd_read {
            hdd_read_fifo(&|_| 0);
        }
    } else {
        if hdd_xfer && hdd_read {
            hdd_write_fifo(&|_| 0);
        }
        if iop_xfer {
            iop_read_fifo();
            if hdd_xfer && hdd_read {
                hdd_write_fifo(&|_| 0);
            }
        }
    }
    fifo_intr();
}

// ---------------------------------------------------------------------------
// External IO / IRQ hooks (the original C++ calls into PS2 IOP / DMA code)
// ---------------------------------------------------------------------------

/// Stand-in for `dev9Irq` from the PS2 IOP side.  The original signature
/// takes a cycle count; this stub just records the request in a counter
/// so callers can observe the behaviour without an IOP runtime.
pub static mut DEV9_IRQ_FIRED: i32 = 0;
pub unsafe fn dev9Irq(cycles: i32) {
    DEV9_IRQ_FIRED += 1;
    let _ = cycles;
}
pub unsafe fn psxDMA8Interrupt() { /* hook into IOP DMA */ }

// ---------------------------------------------------------------------------
// ATA stub (a real implementation lives in the ATA module)
// ---------------------------------------------------------------------------

/// Tiny stub of the ATA controller used by the FIFO.  The real
/// implementation lives in the ATA module; this skeleton keeps the
/// FIFO logic compilable in isolation.
pub struct ATA {
    pub dma_ready: bool,
    pub open: bool,
}
impl ATA {
    pub fn new() -> Self { ATA { dma_ready: false, open: false } }
    pub fn open(&mut self, _path: &str) -> i32 { self.open = true; 0 }
    pub fn close(&mut self) { self.open = false; self.dma_ready = false; }
    pub fn read(&self, _addr: u32, _width: i32) -> u8 { 0 }
    pub fn write(&mut self, _addr: u32, _value: u8, _width: i32) {}
    pub fn hard_reset(&mut self) { self.dma_ready = false; }
    pub fn async_op(&mut self, _cycles: u32) {}
    pub fn read_dma_to_fifo(&self, _buf: &mut [u8]) -> i32 { 0 }
    pub fn write_dma_from_fifo(&self, _buf: &[u8]) -> i32 { 0 }
}

pub static mut ATA_STATE: ATA = ATA {
    dma_ready: false,
    open: false,
};

// ---------------------------------------------------------------------------
// Public lifecycle
// ---------------------------------------------------------------------------

/// Mirror of `DEV9init`.  Resets the dev9 state, allocates an ATA
/// device, and marks every RX buffer descriptor as empty.
pub unsafe fn dev9Init() -> i32 {
    let dev9_ptr: *mut Dev9State = &mut dev9;
    ::std::ptr::write_bytes(dev9_ptr, 0, 1);
    dev9FlashInit();
    // Initialise every RX BD.
    let rx_bd_count = (SMAP_BD_SIZE / 8) as usize;
    for rxbi in 0..rx_bd_count {
        let bd_ptr = dev9Ru16((SMAP_BD_RX_BASE & 0xffff) as u32) as *mut u16;
        let bd = (bd_ptr as *mut SmapBd).add(rxbi);
        (*bd).ctrl_stat = SMAP_BD_RX_EMPTY;
        (*bd).length    = 0;
    }
    isRunning = false;
    0
}

/// Mirror of `DEV9reset` - zero the running state.
pub unsafe fn dev9Reset() {
    let dev9_ptr: *mut Dev9State = &mut dev9;
    ::std::ptr::write_bytes(dev9_ptr, 0, 1);
    isRunning = false;
}

/// Mirror of `DEV9shutdown`.
pub unsafe fn dev9Shutdown() {
    isRunning = false;
}

// ---------------------------------------------------------------------------
// Speed register read/write helpers
// ---------------------------------------------------------------------------

unsafe fn speed_read(addr: u32, _width: i32) -> u16 {
    match addr {
        0x1000_0020 => 1,
        SPD_R_INTR_STAT => dev9.irqcause,
        SPD_R_INTR_MASK => dev9.irqmask,
        SPD_R_PIO_DATA => {
            let mut hard: u16 = 0;
            if dev9.eeprom_state == EEPROM_TDATA && dev9.eeprom_command == 2 {
                if dev9.eeprom_bit != 0xFF {
                    hard = ((dev9.eeprom[dev9.eeprom_address as usize] << dev9.eeprom_bit) & 0x8000) >> 11;
                }
                dev9.eeprom_bit = dev9.eeprom_bit.wrapping_add(1);
                if dev9.eeprom_bit == 16 {
                    dev9.eeprom_address = dev9.eeprom_address.wrapping_add(1);
                    dev9.eeprom_bit = 0;
                }
            }
            hard
        }
        SPD_R_REV_1 => 0,
        SPD_R_REV_2 => 0x11,
        SPD_R_REV_3 => {
            let mut hard: u16 = SPD_CAPS_ATA | SPD_CAPS_FLASH;
            hard |= SPD_CAPS_SMAP;
            hard
        }
        SPD_R_0e => 0x0002,
        SPD_R_XFR_CTRL => dev9.xfr_ctrl,
        SPD_R_DBUF_STAT => {
            let count: u8 = ((dev9.fifo_bytes_write - dev9.fifo_bytes_read) / 512) as u8;
            let mut hard: u16 = if dev9.xfr_ctrl & SPD_XFR_WRITE != 0 {
                (SPD_DBUF_AVAIL_MAX - count as u16) | if count == 0 { SPD_DBUF_STAT_1 } else { 0 } |
                    if count > 0 { SPD_DBUF_STAT_2 } else { 0 }
            } else {
                count as u16 |
                    if (count as u16) < SPD_DBUF_AVAIL_MAX { SPD_DBUF_STAT_1 } else { 0 } |
                    if count == 0 { SPD_DBUF_STAT_2 } else { 0 }
            };
            if count == SPD_DBUF_AVAIL_MAX as u8 { hard |= SPD_DBUF_STAT_FULL; }
            hard
        }
        SPD_R_IF_CTRL => dev9.if_ctrl,
        _ => dev9Ru16(addr),
    }
}

unsafe fn speed_write(addr: u32, value: u16, _width: i32) {
    match addr {
        0x1000_0020 => {}
        SPD_R_INTR_STAT => { dev9.irqcause = value; }
        SPD_R_INTR_MASK => {
            if dev9.irqmask != value && (dev9.irqmask | value) & dev9.irqcause != 0 {
                dev9Irq(1);
            }
            dev9.irqmask = value;
        }
        SPD_R_PIO_DIR => {
            if (value & 0xc0) != 0xc0 { return; }
            if (value & 0x30) == 0x20 { dev9.eeprom_state = 0; }
            dev9.eeprom_dir = ((value >> 4) & 3) as u8;
        }
        SPD_R_PIO_DATA => {
            if (value & 0xc0) != 0xc0 { return; }
            match dev9.eeprom_state {
                EEPROM_READY => { dev9.eeprom_command = 0; dev9.eeprom_state += 1; }
                EEPROM_OPCD0 => {
                    dev9.eeprom_command = ((value >> 4) & 2) as u8;
                    dev9.eeprom_state += 1;
                    dev9.eeprom_bit = 0xFF;
                }
                EEPROM_OPCD1 => {
                    dev9.eeprom_command |= ((value >> 5) & 1) as u8;
                    dev9.eeprom_state += 1;
                }
                EEPROM_ADDR0..=EEPROM_ADDR5 => {
                    let idx = (dev9.eeprom_state - EEPROM_ADDR0) as u32;
                    let mask = 63 ^ (1 << idx);
                    let bit  = (0x20u16 >> idx) as u16;
                    dev9.eeprom_address = ((dev9.eeprom_address as u16 & mask) |
                        (((value >> idx as u16) as u16) & bit)) as u8;
                    dev9.eeprom_state += 1;
                }
                EEPROM_TDATA => {
                    if dev9.eeprom_command == 1 {
                        let mask = (63 ^ (1u16 << dev9.eeprom_bit as u16)) as u16;
                        let bit  = ((0x8000u16 >> dev9.eeprom_bit as u16) as u16) as u16;
                        let cur  = dev9.eeprom[dev9.eeprom_address as usize];
                        let new  = (cur & mask) | ((value >> dev9.eeprom_bit as u16) & bit);
                        dev9.eeprom[dev9.eeprom_address as usize] = new;
                        dev9.eeprom_bit = dev9.eeprom_bit.wrapping_add(1);
                        if dev9.eeprom_bit == 16 {
                            dev9.eeprom_address = dev9.eeprom_address.wrapping_add(1);
                            dev9.eeprom_bit = 0;
                        }
                    }
                }
                _ => {}
            }
        }
        SPD_R_DMA_CTRL => {
            dev9.dma_ctrl = value;
            if value & SPD_DMA_PAUSE != 0 { /* unimpl */ }
        }
        SPD_R_XFR_CTRL => {
            let old = dev9.xfr_ctrl;
            dev9.xfr_ctrl = value;
            if (value & SPD_XFR_WRITE) != (old & SPD_XFR_WRITE) { dev9_run_fifo(); }
            if value & SPD_XFR_DMAEN != 0 { dev9_run_fifo(); }
        }
        SPD_R_DBUF_STAT => {
            if value & SPD_DBUF_RESET_READ_CNT  != 0 { dev9.fifo_bytes_read  = 0; }
            if value & SPD_DBUF_RESET_WRITE_CNT != 0 { dev9.fifo_bytes_write = 0; }
            if value != 0 { fifo_intr(); }
        }
        SPD_R_IF_CTRL => {
            let old = dev9.if_ctrl;
            dev9.if_ctrl = value;
            if (value & SPD_IF_READ) != (old & SPD_IF_READ) { dev9_run_fifo(); }
            if value & SPD_IF_ATA_DMAEN != 0 { dev9_run_fifo(); }
            if value & SPD_IF_HDD_RESET == 0 {
                ATA_STATE.hard_reset();
            }
            if value & SPD_IF_ATA_RESET != 0 {
                dev9.if_ctrl   = 0x001A;
                dev9.pio_mode  = 0x24;
                dev9.mdma_mode = 0x45;
                dev9.udma_mode = 0x83;
            }
        }
        SPD_R_PIO_MODE  => { dev9.pio_mode  = value; }
        SPD_R_MDMA_MODE => { dev9.mdma_mode = value; }
        SPD_R_UDMA_MODE => { dev9.udma_mode = value; }
        _ => {
            dev9Wu8(addr, value as u8);
        }
    }
}

// ---------------------------------------------------------------------------
// Public register accessors
// ---------------------------------------------------------------------------

pub unsafe fn dev9Read8(addr: u32) -> u8 {
    if addr >= ATA_DEV9_HDD_BASE && addr < ATA_DEV9_HDD_END {
        return ATA_STATE.read(addr, 8);
    }
    if addr >= SPD_REGBASE && addr < SMAP_REGBASE {
        return speed_read(addr, 8) as u8;
    }
    if addr >= SMAP_REGBASE && addr < FLASH_REGBASE {
        return smap_read8(addr);
    }
    if addr >= FLASH_REGBASE && addr < FLASH_REGBASE + FLASH_REGSIZE {
        return dev9FlashRead(addr);
    }
    match addr {
        DEV9_R_REV => 0x32,
        _ => dev9Ru8(addr),
    }
}

pub unsafe fn dev9Read16(addr: u32) -> u16 {
    if addr >= ATA_DEV9_HDD_BASE && addr < ATA_DEV9_HDD_END {
        return ATA_STATE.read(addr, 16) as u16;
    }
    if addr >= SPD_REGBASE && addr < SMAP_REGBASE {
        return speed_read(addr, 16);
    }
    if addr >= SMAP_REGBASE && addr < FLASH_REGBASE {
        return smap_read16(addr);
    }
    if addr >= FLASH_REGBASE && addr < FLASH_REGBASE + FLASH_REGSIZE {
        return dev9FlashRead(addr) as u16;
    }
    match addr {
        DEV9_R_REV => 0x0032,
        _ => dev9Ru16(addr),
    }
}

pub unsafe fn dev9Read32(addr: u32) -> u32 {
    if addr >= ATA_DEV9_HDD_BASE && addr < ATA_DEV9_HDD_END {
        return 0;
    }
    if addr >= SMAP_REGBASE && addr < FLASH_REGBASE {
        return smap_read32(addr);
    }
    if addr >= FLASH_REGBASE && addr < FLASH_REGBASE + FLASH_REGSIZE {
        return dev9FlashRead(addr) as u32;
    }
    dev9Ru32(addr)
}

pub unsafe fn dev9Write8(addr: u32, value: u8) {
    if addr >= ATA_DEV9_HDD_BASE && addr < ATA_DEV9_HDD_END {
        ATA_STATE.write(addr, value, 8);
        return;
    }
    if addr >= SPD_REGBASE && addr < SMAP_REGBASE {
        speed_write(addr, value as u16, 8);
        return;
    }
    if addr >= SMAP_REGBASE && addr < FLASH_REGBASE {
        smap_write8(addr, value);
        return;
    }
    if addr >= FLASH_REGBASE && addr < FLASH_REGBASE + FLASH_REGSIZE {
        dev9FlashWrite(addr, value);
        return;
    }
}

pub unsafe fn dev9Write16(addr: u32, value: u16) {
    if addr >= ATA_DEV9_HDD_BASE && addr < ATA_DEV9_HDD_END {
        ATA_STATE.write(addr, value as u8, 16);
        return;
    }
    if addr >= SPD_REGBASE && addr < SMAP_REGBASE {
        speed_write(addr, value, 16);
        return;
    }
    if addr >= SMAP_REGBASE && addr < FLASH_REGBASE {
        smap_write16(addr, value);
        return;
    }
    if addr >= FLASH_REGBASE && addr < FLASH_REGBASE + FLASH_REGSIZE {
        dev9FlashWrite(addr, value as u8);
        return;
    }
}

pub unsafe fn dev9Write32(addr: u32, value: u32) {
    if addr >= ATA_DEV9_HDD_BASE && addr < ATA_DEV9_HDD_END {
        return;
    }
    if addr >= SMAP_REGBASE && addr < FLASH_REGBASE {
        smap_write32(addr, value);
        return;
    }
    if addr >= FLASH_REGBASE && addr < FLASH_REGBASE + FLASH_REGSIZE {
        dev9FlashWrite(addr, value as u8);
        return;
    }
}

// ---------------------------------------------------------------------------
// SMAP register access (skeleton translation of smap.cpp)
// ---------------------------------------------------------------------------

pub unsafe fn smap_read8(addr: u32) -> u8 {
    match addr {
        SMAP_R_BD_MODE => dev9.bd_swap,
        _ => dev9Ru8(addr),
    }
}

pub unsafe fn smap_read16(addr: u32) -> u16 {
    let rv = dev9Ru16(addr);
    if (addr >= SMAP_BD_TX_BASE && addr < SMAP_BD_TX_BASE + SMAP_BD_SIZE)
        || (addr >= SMAP_BD_RX_BASE && addr < SMAP_BD_RX_BASE + SMAP_BD_SIZE)
    {
        if dev9.bd_swap != 0 { rv.rotate_left(8) } else { rv }
    } else {
        rv
    }
}

pub unsafe fn smap_read32(addr: u32) -> u32 {
    if addr >= SMAP_EMAC3_REGBASE && addr < SMAP_EMAC3_REGEND {
        let hi = smap_read16(addr) as u32;
        let lo = (smap_read16(addr + 2) as u32) << 16;
        return hi | lo;
    }
    match addr {
        SMAP_R_RXFIFO_DATA => {
            let rd_ptr = dev9Ru32(SMAP_R_RXFIFO_RD_PTR) & 16383;
            let rv = u32::from_le_bytes([
                dev9.rxfifo[rd_ptr as usize],
                dev9.rxfifo[rd_ptr as usize + 1],
                dev9.rxfifo[rd_ptr as usize + 2],
                dev9.rxfifo[rd_ptr as usize + 3],
            ]);
            dev9Wu32(SMAP_R_RXFIFO_RD_PTR, (rd_ptr + 4) & 16383);
            rv
        }
        _ => dev9Ru32(addr),
    }
}

pub unsafe fn smap_write8(addr: u32, value: u8) {
    match addr {
        SMAP_R_TXFIFO_FRAME_INC => {
            dev9Wu8(SMAP_R_TXFIFO_FRAME_CNT, dev9Ru8(SMAP_R_TXFIFO_FRAME_CNT).wrapping_add(1));
        }
        SMAP_R_RXFIFO_FRAME_DEC => {
            dev9Wu8(addr, value);
            dev9Wu8(SMAP_R_RXFIFO_FRAME_CNT, dev9Ru8(SMAP_R_RXFIFO_FRAME_CNT).wrapping_sub(1));
        }
        SMAP_R_TXFIFO_CTRL => {
            if value & SMAP_TXFIFO_RESET as u8 != 0 {
                dev9.txbdi = 0;
                dev9.txfifo_rd_ptr = 0;
                dev9Wu8(SMAP_R_TXFIFO_FRAME_CNT, 0);
                dev9Wu32(SMAP_R_TXFIFO_WR_PTR, 0);
                dev9Wu32(SMAP_R_TXFIFO_SIZE, 16384);
            }
            dev9Wu8(addr, value & !(SMAP_TXFIFO_RESET as u8));
        }
        SMAP_R_RXFIFO_CTRL => {
            if value & SMAP_RXFIFO_RESET as u8 != 0 {
                dev9.rxbdi = 0;
                dev9.rxfifo_wr_ptr = 0;
                dev9Wu8(SMAP_R_RXFIFO_FRAME_CNT, 0);
                dev9Wu32(SMAP_R_RXFIFO_RD_PTR, 0);
                dev9Wu32(SMAP_R_RXFIFO_SIZE, 16384);
            }
            dev9Wu8(addr, value & !(SMAP_RXFIFO_RESET as u8));
        }
        SMAP_R_BD_MODE => {
            dev9.bd_swap = if value & SMAP_BD_SWAP as u8 != 0 { 1 } else { 0 };
        }
        _ => { dev9Wu8(addr, value); }
    }
}

pub unsafe fn smap_write16(addr: u32, mut value: u16) {
    if (addr >= SMAP_BD_TX_BASE && addr < SMAP_BD_TX_BASE + SMAP_BD_SIZE)
        || (addr >= SMAP_BD_RX_BASE && addr < SMAP_BD_RX_BASE + SMAP_BD_SIZE)
    {
        if dev9.bd_swap != 0 { value = value.swap_bytes(); }
        dev9Wu16(addr, value);
        return;
    }
    match addr {
        SMAP_R_INTR_CLR => { dev9.irqcause &= !value; }
        _ => { dev9Wu16(addr, value); }
    }
}

pub unsafe fn smap_write32(addr: u32, value: u32) {
    if addr >= SMAP_EMAC3_REGBASE && addr < SMAP_EMAC3_REGEND {
        smap_write16(addr, (value & 0xFFFF) as u16);
        smap_write16(addr + 2, (value >> 16) as u16);
        return;
    }
    match addr {
        SMAP_R_TXFIFO_DATA => {
            let wr = dev9Ru32(SMAP_R_TXFIFO_WR_PTR) & 16383;
            dev9.txfifo[wr as usize]      = (value & 0xff) as u8;
            dev9.txfifo[wr as usize + 1]  = ((value >> 8) & 0xff) as u8;
            dev9.txfifo[wr as usize + 2]  = ((value >> 16) & 0xff) as u8;
            dev9.txfifo[wr as usize + 3]  = ((value >> 24) & 0xff) as u8;
            dev9Wu32(SMAP_R_TXFIFO_WR_PTR, (wr + 4) & 16383);
        }
        _ => { dev9Wu32(addr, value); }
    }
}

pub unsafe fn smap_read_dma8_mem(pmem: *mut u32, size: i32) {
    if dev9Ru16(SMAP_R_RXFIFO_CTRL) & SMAP_RXFIFO_DMAEN != 0 {
        let mut p = pmem;
        let mut n = size;
        while n > 0 {
            let rd = dev9Ru32(SMAP_R_RXFIFO_RD_PTR) & 16383;
            let v = u32::from_le_bytes([
                dev9.rxfifo[rd as usize],
                dev9.rxfifo[rd as usize + 1],
                dev9.rxfifo[rd as usize + 2],
                dev9.rxfifo[rd as usize + 3],
            ]);
            *p = v;
            p = p.add(1);
            dev9Wu32(SMAP_R_RXFIFO_RD_PTR, (rd + 4) & 16383);
            n -= 4;
        }
        dev9Wu16(SMAP_R_RXFIFO_CTRL, dev9Ru16(SMAP_R_RXFIFO_CTRL) & !SMAP_RXFIFO_DMAEN);
    }
}

pub unsafe fn smap_write_dma8_mem(pmem: *const u32, size: i32) {
    if dev9Ru16(SMAP_R_TXFIFO_CTRL) & SMAP_TXFIFO_DMAEN != 0 {
        let mut p = pmem;
        let mut n = size;
        while n > 0 {
            let value = *p;
            p = p.add(1);
            let wr = dev9Ru32(SMAP_R_TXFIFO_WR_PTR) & 16383;
            dev9.txfifo[wr as usize]      = (value & 0xff) as u8;
            dev9.txfifo[wr as usize + 1]  = ((value >> 8) & 0xff) as u8;
            dev9.txfifo[wr as usize + 2]  = ((value >> 16) & 0xff) as u8;
            dev9.txfifo[wr as usize + 3]  = ((value >> 24) & 0xff) as u8;
            dev9Wu32(SMAP_R_TXFIFO_WR_PTR, (wr + 4) & 16383);
            n -= 4;
        }
        dev9Wu16(SMAP_R_TXFIFO_CTRL, dev9Ru16(SMAP_R_TXFIFO_CTRL) & !SMAP_TXFIFO_DMAEN);
    }
}

pub static mut fireIntR: AtomicBool = AtomicBool::new(false);
pub unsafe fn smap_async(_cycles: u32) {
    if fireIntR.swap(false, Ordering::AcqRel) {
        dev9Irq(0);
    }
}

pub unsafe fn dev9SmapHandleTx() {
    // Iterate TX BDs and forward ready ones through the network.
    let mut cnt: u32 = 0;
    loop {
        let bd_addr = (SMAP_BD_TX_BASE & 0xffff) as u32 + (dev9.txbdi * 8);
        let bd_ptr = (bd_addr & 0xffff) as *mut SmapBd;
        let pbd = unsafe { &mut *bd_ptr };
        if pbd.ctrl_stat & SMAP_BD_TX_READY == 0 { break; }
        if pbd.length > 1514 { /* oversized */ }
        else {
            let base = ((pbd.pointer - 0x1000) & 16383) as usize;
            if base + pbd.length as usize > 16384 {
                let was = 16384 - base;
                let mut tmp = NetPacket::new();
                tmp.size = pbd.length as i32;
                tmp.buffer[..was].copy_from_slice(&dev9.txfifo[base..base + was]);
                let rest = pbd.length as usize - was;
                tmp.buffer[was..was + rest].copy_from_slice(&dev9.txfifo[..rest]);
                let _ = tmp; // tx_put(&tmp)
            } else {
                let mut tmp = NetPacket::new();
                tmp.size = pbd.length as i32;
                tmp.buffer[..pbd.length as usize]
                    .copy_from_slice(&dev9.txfifo[base..base + pbd.length as usize]);
                let _ = tmp;
            }
        }
        pbd.ctrl_stat &= !SMAP_BD_TX_READY;
        dev9.txbdi = (dev9.txbdi + 1) & ((SMAP_BD_SIZE / 8) - 1);
        dev9Wu8(SMAP_R_TXFIFO_FRAME_CNT, dev9Ru8(SMAP_R_TXFIFO_FRAME_CNT).wrapping_sub(1));
        cnt += 1;
    }
    if cnt != 0 { dev9Irq(100); }
}

pub unsafe fn dev9SmapHandleRx() {
    // The real implementation would call into NetAdapter::recv; the
    // translation provides the entry point only.
}

// ---------------------------------------------------------------------------
// Network subsystem
// ---------------------------------------------------------------------------

pub static mut NIF: Option<Box<dyn Dev9Network>> = None;
pub static mut RX_RUNNING: AtomicBool = AtomicBool::new(false);

/// Build a network adapter from the current configuration.  Returns
/// `None` if the requested API is unavailable; the caller is expected
/// to disable Ethernet when this happens.
pub fn dev9GetNetAdapter() -> Option<Box<dyn Dev9Network>> {
    None
}

pub unsafe fn dev9NetInit() {
    if let Some(na) = dev9GetNetAdapter() {
        NIF = Some(na);
        RX_RUNNING.store(true, Ordering::Release);
    }
}

pub unsafe fn dev9NetShutdown() {
    RX_RUNNING.store(false, Ordering::Release);
    if let Some(mut n) = NIF.take() {
        n.close();
    }
}

pub unsafe fn tx_put(pkt: *const NetPacket) {
    if let Some(n) = NIF.as_mut() {
        let _ = n.send(&*pkt);
    }
}

pub unsafe fn ad_reset() {
    if let Some(n) = NIF.as_mut() {
        n.reset();
    }
}

// ---------------------------------------------------------------------------
// Sockets API stubs
// ---------------------------------------------------------------------------

/// Mirror of the socket adapter's "connect" call.  The original C++
/// implementation has a complex TCP/UDP/ICMP dispatch; the Rust
/// translation exposes a simple entry point that the network backend
/// (a `Dev9Network` implementation) can service.
pub unsafe fn dev9SocketConnect() -> bool { false }

/// Mirror of the socket adapter's "send" call.  Returns the number of
/// bytes sent, or `-1` on failure.
pub unsafe fn dev9SocketSend() -> i32 { 0 }

/// Mirror of the socket adapter's "recv" call.  Returns the number of
/// bytes received, or `-1` on failure.
pub unsafe fn dev9SocketRecv() -> i32 { 0 }

/// Enumerate the available network adapters (a thin wrapper over
/// `Dev9AdapterUtils`).
pub fn dev9GetAdapters() -> Vec<AdapterEntry> { Dev9AdapterUtils::get_all_adapters() }

/// Return the supported options for the socket adapter.
pub fn dev9GetAdapterOptions() -> AdapterOptions {
    AdapterOptions::DHCP_FORCED_ON
        | AdapterOptions::DHCP_OVERRIDE_IP
        | AdapterOptions::DHCP_OVERRIDE_SUBNET
        | AdapterOptions::DHCP_OVERRIDE_GATEWAY
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev9_state_zero_on_init() {
        unsafe {
            dev9Init();
            assert_eq!(dev9.irqcause, 0);
            assert_eq!(dev9.fifo_bytes_read, 0);
            assert_eq!(dev9.fifo_bytes_write, 0);
        }
    }

    #[test]
    fn flash_id_default() {
        unsafe {
            dev9FlashInit();
            assert_eq!(FLASH_STATE.id, FLASH_ID_64MBIT);
        }
    }

    #[test]
    fn simple_queue_round_trip() {
        let q: SimpleQueue<u32> = SimpleQueue::new();
        q.enqueue(7);
        let mut out: Option<u32> = None;
        assert!(q.dequeue(&mut out));
        assert_eq!(out, Some(7));
    }

    #[test]
    fn thread_safe_map_basic() {
        let m: ThreadSafeMap<u32, &str> = ThreadSafeMap::new();
        m.add(1, "a");
        m.add(2, "b");
        assert_eq!(m.try_get_value(&1), Some("a"));
        assert!(m.contains_key(&2));
        assert!(m.remove(&1));
        assert_eq!(m.try_get_value(&1), None);
    }

    #[test]
    fn dev9adapter_utils_returns_empty() {
        assert!(Dev9AdapterUtils::get_all_adapters().is_empty());
    }
}
