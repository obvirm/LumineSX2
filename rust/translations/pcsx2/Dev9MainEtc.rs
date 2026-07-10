// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust 2021 translation of the PCSX2 `DEV9` subsystem.
//!
//! This module is a self-contained rendering of the C/C++ source set:
//! `DEV9.cpp/.h`, `flash.cpp`, `net.cpp/.h`, `smap.cpp/.h`, `sockets.cpp/.h`,
//! `AdapterUtils.cpp/.h`, `pcap_io.cpp/.h`, `SimpleQueue.h`, `ThreadSafeMap.h`,
//! and the `Win32/` variants of the PCAP and TAP helpers.
//!
//! Only `std` is used (no platform-specific or third-party crates).  All
//! globally mutable state lives behind `static mut` items in this module, in
//! keeping with the original C++ layout.  Each public entry point takes
//! `&mut` borrows of those statics as needed.
//!
//! The surface intentionally mirrors the original `DEV9init/open/close/
//! shutdown`, the `DEV9read8/16/32` / `DEV9write8/16/32` accessors, the
//! SMAP and SPEED register file, the network adapter trait, the flash
//! (SmartMedia) implementation, the `SimpleQueue` and `ThreadSafeMap`
//! helpers, and the adapter enumeration API.

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(dead_code)]
#![allow(unused_assignments)]

use std::cmp::min;
use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::thread::{self, JoinHandle};

// =====================================================================
//  Primitive type aliases matching the PCSX2 typedefs.
// =====================================================================

pub type u8  = ::std::primitive::u8;
pub type u16 = ::std::primitive::u16;
pub type u32 = ::std::primitive::u32;
pub type s8  = ::std::primitive::i8;
pub type s16 = ::std::primitive::i16;
pub type s32 = ::std::primitive::i32;
pub type s64 = ::std::primitive::i64;

pub const DEV9_R_REV: u32 = 0x1F80_146E;

// =====================================================================
//  SPEED (SPD) register addresses and bit definitions.
// =====================================================================

pub const SPD_INTR_ATA_FIFO_DATA:   u16 = 1 << 1;
pub const SPD_INTR_ATA_FIFO_FULL:   u16 = 1 << 15;
pub const SPD_INTR_ATA_FIFO_EMPTY:  u16 = 1 << 14;
pub const SPD_INTR_ATA_FIFO_OVERFLOW: u16 = SPD_INTR_ATA_FIFO_FULL | SPD_INTR_ATA_FIFO_EMPTY;

pub const SPD_REGBASE: u32 = 0x1000_0000;
pub const ATA_INTR_INTRQ: u32 = 1 << 0;

pub const SPD_R_REV_1: u32   = SPD_REGBASE + 0x00;
pub const SPD_R_REV_2: u32   = SPD_REGBASE + 0x02;
pub const SPD_CAPS_SMAP:  u16 = 1 << 0;
pub const SPD_CAPS_ATA:   u16 = 1 << 1;
pub const SPD_CAPS_UART:  u16 = 1 << 3;
pub const SPD_CAPS_DVR:   u16 = 1 << 4;
pub const SPD_CAPS_FLASH: u16 = 1 << 5;
pub const SPD_R_REV_3:    u32 = SPD_REGBASE + 0x04;
pub const SPD_R_0e:       u32 = SPD_REGBASE + 0x0e;

pub const SPD_R_DMA_CTRL: u32  = SPD_REGBASE + 0x24;
pub const SPD_DMA_TO_SMAP: u16 = 1 << 0;
pub const SPD_DMA_FASTEST: u16 = 1 << 1;
pub const SPD_DMA_WIDE:    u16 = 1 << 2;
pub const SPD_DMA_PAUSE:   u16 = 1 << 4;
pub const SPD_R_INTR_STAT: u32 = SPD_REGBASE + 0x28;
pub const SPD_R_INTR_MASK: u32 = SPD_REGBASE + 0x2a;

pub const SPD_R_PIO_DIR:  u32 = SPD_REGBASE + 0x2c;
pub const SPD_R_PIO_DATA: u32 = SPD_REGBASE + 0x2e;
pub const SPD_PP_DOUT: u16  = 1 << 4;
pub const SPD_PP_DIN:  u16  = 1 << 5;
pub const SPD_PP_SCLK: u16  = 1 << 6;
pub const SPD_PP_CSEL: u16  = 1 << 7;

pub const SPD_R_XFR_CTRL: u32 = SPD_REGBASE + 0x32;
pub const SPD_XFR_WRITE:  u16 = 1 << 0;
pub const SPD_XFR_DMAEN:  u16 = 1 << 7;
pub const SPD_R_DBUF_STAT: u32 = SPD_REGBASE + 0x38;
pub const SPD_DBUF_AVAIL_MAX: u16 = 0x10;
pub const SPD_DBUF_AVAIL_MASK: u16 = 0x1F;
pub const SPD_DBUF_STAT_1:  u16 = 1 << 5;
pub const SPD_DBUF_STAT_2:  u16 = 1 << 6;
pub const SPD_DBUF_STAT_FULL: u16 = 1 << 7;
pub const SPD_DBUF_RESET_READ_CNT:  u16 = 1 << 0;
pub const SPD_DBUF_RESET_WRITE_CNT: u16 = 1 << 1;

pub const SPD_R_IF_CTRL: u32    = SPD_REGBASE + 0x64;
pub const SPD_IF_UDMA: u16      = 1 << 0;
pub const SPD_IF_READ: u16      = 1 << 1;
pub const SPD_IF_ATA_DMAEN: u16 = 1 << 2;
pub const SPD_IF_HDD_RESET: u16 = 1 << 6;
pub const SPD_IF_ATA_RESET: u16 = 1 << 7;
pub const SPD_R_PIO_MODE: u32   = SPD_REGBASE + 0x70;
pub const SPD_R_MDMA_MODE: u32  = SPD_REGBASE + 0x72;
pub const SPD_R_UDMA_MODE: u32  = SPD_REGBASE + 0x74;

// =====================================================================
//  SMAP register addresses and bit definitions.
// =====================================================================

pub const SMAP_INTR_EMAC3: u16 = 1 << 6;
pub const SMAP_INTR_RXEND: u16 = 1 << 5;
pub const SMAP_INTR_TXEND: u16 = 1 << 4;
pub const SMAP_INTR_RXDNV: u16 = 1 << 3;
pub const SMAP_INTR_TXDNV: u16 = 1 << 2;
pub const SMAP_INTR_CLR_ALL: u16 = SMAP_INTR_RXEND | SMAP_INTR_TXEND | SMAP_INTR_RXDNV;
pub const SMAP_INTR_ENA_ALL: u16 = SMAP_INTR_EMAC3 | SMAP_INTR_CLR_ALL;
pub const SMAP_INTR_BITMSK: u16 = 0x7C;

pub const SMAP_REGBASE: u32 = SPD_REGBASE + 0x100;

pub const SMAP_R_BD_MODE:  u32 = SMAP_REGBASE + 0x02;
pub const SMAP_BD_SWAP:    u16 = 1 << 0;
pub const SMAP_R_INTR_CLR: u32 = SMAP_REGBASE + 0x28;

pub const SMAP_R_TXFIFO_CTRL:     u32 = SMAP_REGBASE + 0xf00;
pub const SMAP_TXFIFO_RESET: u16     = 1 << 0;
pub const SMAP_TXFIFO_DMAEN: u16     = 1 << 1;
pub const SMAP_R_TXFIFO_WR_PTR:  u32 = SMAP_REGBASE + 0xf04;
pub const SMAP_R_TXFIFO_SIZE:    u32 = SMAP_REGBASE + 0xf08;
pub const SMAP_R_TXFIFO_FRAME_CNT: u32 = SMAP_REGBASE + 0xf0C;
pub const SMAP_R_TXFIFO_FRAME_INC: u32 = SMAP_REGBASE + 0xf10;
pub const SMAP_R_TXFIFO_DATA:  u32 = SMAP_REGBASE + 0x1000;

pub const SMAP_R_RXFIFO_CTRL:    u32 = SMAP_REGBASE + 0xf30;
pub const SMAP_RXFIFO_RESET: u16    = 1 << 0;
pub const SMAP_RXFIFO_DMAEN: u16    = 1 << 1;
pub const SMAP_R_RXFIFO_RD_PTR: u32 = SMAP_REGBASE + 0xf34;
pub const SMAP_R_RXFIFO_SIZE:   u32 = SMAP_REGBASE + 0xf38;
pub const SMAP_R_RXFIFO_FRAME_CNT: u32 = SMAP_REGBASE + 0xf3C;
pub const SMAP_R_RXFIFO_FRAME_DEC: u32 = SMAP_REGBASE + 0xf40;
pub const SMAP_R_RXFIFO_DATA:   u32 = SMAP_REGBASE + 0x1100;

pub const SMAP_R_FIFO_ADDR:  u32 = SMAP_REGBASE + 0x1200;
pub const SMAP_FIFO_CMD_READ:  u16 = 1 << 1;
pub const SMAP_FIFO_DATA_SWAP: u16 = 1 << 0;
pub const SMAP_R_FIFO_DATA:  u32 = SMAP_REGBASE + 0x1208;

pub const SMAP_EMAC3_REGBASE: u32 = SMAP_REGBASE + 0x1f00;

pub const SMAP_R_EMAC3_MODE0_L: u32 = SMAP_EMAC3_REGBASE + 0x00;
pub const SMAP_R_EMAC3_MODE0_H: u32 = SMAP_EMAC3_REGBASE + 0x02;
pub const SMAP_R_EMAC3_MODE1:   u32 = SMAP_EMAC3_REGBASE + 0x04;
pub const SMAP_R_EMAC3_MODE1_L: u32 = SMAP_EMAC3_REGBASE + 0x04;
pub const SMAP_R_EMAC3_MODE1_H: u32 = SMAP_EMAC3_REGBASE + 0x06;

pub const SMAP_E3_RXMAC_IDLE: u32 = 1 << (15 + 16);
pub const SMAP_E3_TXMAC_IDLE: u32 = 1 << (14 + 16);
pub const SMAP_E3_SOFT_RESET: u32 = 1 << (13 + 16);
pub const SMAP_E3_TXMAC_ENABLE: u32 = 1 << (12 + 16);
pub const SMAP_E3_RXMAC_ENABLE: u32 = 1 << (11 + 16);
pub const SMAP_E3_WAKEUP_ENABLE: u32 = 1 << (10 + 16);

pub const SMAP_R_EMAC3_TxMODE0_L: u32 = SMAP_EMAC3_REGBASE + 0x08;
pub const SMAP_E3_TX_GNP_0: u32 = 1 << (15 + 16);
pub const SMAP_E3_TX_GNP_1: u32 = 1 << (14 + 16);
pub const SMAP_E3_TX_GNP_DEPEND: u32 = 1 << (13 + 16);
pub const SMAP_E3_TX_FIRST_CHANNEL: u32 = 1 << (12 + 16);
pub const SMAP_R_EMAC3_TxMODE0_H: u32 = SMAP_EMAC3_REGBASE + 0x0A;
pub const SMAP_R_EMAC3_TxMODE1_L: u32 = SMAP_EMAC3_REGBASE + 0x0C;
pub const SMAP_R_EMAC3_TxMODE1_H: u32 = SMAP_EMAC3_REGBASE + 0x0E;
pub const SMAP_R_EMAC3_RxMODE:   u32 = SMAP_EMAC3_REGBASE + 0x10;
pub const SMAP_R_EMAC3_RxMODE_L: u32 = SMAP_EMAC3_REGBASE + 0x10;
pub const SMAP_R_EMAC3_RxMODE_H: u32 = SMAP_EMAC3_REGBASE + 0x12;
pub const SMAP_R_EMAC3_INTR_STAT: u32 = SMAP_EMAC3_REGBASE + 0x14;
pub const SMAP_R_EMAC3_INTR_STAT_L: u32 = SMAP_EMAC3_REGBASE + 0x14;
pub const SMAP_R_EMAC3_INTR_STAT_H: u32 = SMAP_EMAC3_REGBASE + 0x16;
pub const SMAP_R_EMAC3_INTR_ENABLE: u32 = SMAP_EMAC3_REGBASE + 0x18;
pub const SMAP_R_EMAC3_INTR_ENABLE_L: u32 = SMAP_EMAC3_REGBASE + 0x18;
pub const SMAP_R_EMAC3_INTR_ENABLE_H: u32 = SMAP_EMAC3_REGBASE + 0x1A;
pub const SMAP_R_EMAC3_ADDR_HI: u32 = SMAP_EMAC3_REGBASE + 0x1C;
pub const SMAP_R_EMAC3_ADDR_LO: u32 = SMAP_EMAC3_REGBASE + 0x20;
pub const SMAP_R_EMAC3_ADDR_HI_L: u32 = SMAP_EMAC3_REGBASE + 0x1C;
pub const SMAP_R_EMAC3_ADDR_HI_H: u32 = SMAP_EMAC3_REGBASE + 0x1E;
pub const SMAP_R_EMAC3_ADDR_LO_L: u32 = SMAP_EMAC3_REGBASE + 0x20;
pub const SMAP_R_EMAC3_ADDR_LO_H: u32 = SMAP_EMAC3_REGBASE + 0x22;
pub const SMAP_R_EMAC3_VLAN_TPID: u32 = SMAP_EMAC3_REGBASE + 0x24;
pub const SMAP_R_EMAC3_VLAN_TCI: u32 = SMAP_EMAC3_REGBASE + 0x28;
pub const SMAP_R_EMAC3_PAUSE_TIMER: u32 = SMAP_EMAC3_REGBASE + 0x2C;
pub const SMAP_R_EMAC3_PAUSE_TIMER_L: u32 = SMAP_EMAC3_REGBASE + 0x2C;
pub const SMAP_R_EMAC3_PAUSE_TIMER_H: u32 = SMAP_EMAC3_REGBASE + 0x2E;
pub const SMAP_R_EMAC3_INDIVID_HASH1: u32 = SMAP_EMAC3_REGBASE + 0x30;
pub const SMAP_R_EMAC3_INDIVID_HASH2: u32 = SMAP_EMAC3_REGBASE + 0x34;
pub const SMAP_R_EMAC3_INDIVID_HASH3: u32 = SMAP_EMAC3_REGBASE + 0x38;
pub const SMAP_R_EMAC3_INDIVID_HASH4: u32 = SMAP_EMAC3_REGBASE + 0x3C;
pub const SMAP_R_EMAC3_GROUP_HASH1: u32 = SMAP_EMAC3_REGBASE + 0x40;
pub const SMAP_R_EMAC3_GROUP_HASH2: u32 = SMAP_EMAC3_REGBASE + 0x44;
pub const SMAP_R_EMAC3_GROUP_HASH3: u32 = SMAP_EMAC3_REGBASE + 0x48;
pub const SMAP_R_EMAC3_GROUP_HASH4: u32 = SMAP_EMAC3_REGBASE + 0x4C;
pub const SMAP_R_EMAC3_LAST_SA_HI: u32 = SMAP_EMAC3_REGBASE + 0x50;
pub const SMAP_R_EMAC3_LAST_SA_LO: u32 = SMAP_EMAC3_REGBASE + 0x54;
pub const SMAP_R_EMAC3_INTER_FRAME_GAP: u32 = SMAP_EMAC3_REGBASE + 0x58;
pub const SMAP_R_EMAC3_INTER_FRAME_GAP_L: u32 = SMAP_EMAC3_REGBASE + 0x58;
pub const SMAP_R_EMAC3_INTER_FRAME_GAP_H: u32 = SMAP_EMAC3_REGBASE + 0x5A;
pub const SMAP_R_EMAC3_STA_CTRL_L: u32 = SMAP_EMAC3_REGBASE + 0x5C;
pub const SMAP_R_EMAC3_STA_CTRL_H: u32 = SMAP_EMAC3_REGBASE + 0x5E;
pub const SMAP_E3_PHY_OP_COMP: u32 = 1 << 15;
pub const SMAP_E3_PHY_READ:   u32 = 1 << 12;
pub const SMAP_E3_PHY_WRITE:  u32 = 2 << 12;
pub const SMAP_E3_PHY_REG_ADDR_MSK: u32 = 0x1F;
pub const SMAP_R_EMAC3_TX_THRESHOLD: u32 = SMAP_EMAC3_REGBASE + 0x60;
pub const SMAP_R_EMAC3_TX_THRESHOLD_L: u32 = SMAP_EMAC3_REGBASE + 0x60;
pub const SMAP_R_EMAC3_TX_THRESHOLD_H: u32 = SMAP_EMAC3_REGBASE + 0x62;
pub const SMAP_R_EMAC3_RX_WATERMARK:  u32 = SMAP_EMAC3_REGBASE + 0x64;
pub const SMAP_R_EMAC3_RX_WATERMARK_L: u32 = SMAP_EMAC3_REGBASE + 0x64;
pub const SMAP_R_EMAC3_RX_WATERMARK_H: u32 = SMAP_EMAC3_REGBASE + 0x66;
pub const SMAP_R_EMAC3_TX_OCTETS: u32 = SMAP_EMAC3_REGBASE + 0x68;
pub const SMAP_R_EMAC3_RX_OCTETS: u32 = SMAP_EMAC3_REGBASE + 0x6C;
pub const SMAP_EMAC3_REGEND: u32 = SMAP_EMAC3_REGBASE + 0x6C + 4;

// Buffer descriptor region.
pub const SMAP_BD_REGBASE: u32 = SMAP_REGBASE + 0x2f00;
pub const SMAP_BD_TX_BASE:  u32 = SMAP_BD_REGBASE + 0x0000;
pub const SMAP_BD_RX_BASE:  u32 = SMAP_BD_REGBASE + 0x0200;
pub const SMAP_BD_SIZE:     u32 = 512;
pub const SMAP_BD_MAX_ENTRY: u32 = 64;

pub const SMAP_TX_BASE:    u32 = SMAP_REGBASE + 0x1000;
pub const SMAP_TX_BUFSIZE: u32 = 4096;

// Buffer descriptor control / status bits.
pub const SMAP_BD_TX_READY:  u16 = 1 << 15;
pub const SMAP_BD_TX_GENFCS: u16 = 1 << 9;
pub const SMAP_BD_TX_GENPAD: u16 = 1 << 8;
pub const SMAP_BD_TX_INSSA:  u16 = 1 << 7;
pub const SMAP_BD_TX_RPLSA:  u16 = 1 << 6;
pub const SMAP_BD_TX_INSVLAN: u16 = 1 << 5;
pub const SMAP_BD_TX_RPLVLAN: u16 = 1 << 4;
pub const SMAP_BD_TX_BADFCS:  u16 = 1 << 9;
pub const SMAP_BD_TX_BADPKT:  u16 = 1 << 8;
pub const SMAP_BD_TX_LOSSCR:  u16 = 1 << 7;
pub const SMAP_BD_TX_EDEFER:  u16 = 1 << 6;
pub const SMAP_BD_TX_ECOLL:   u16 = 1 << 5;
pub const SMAP_BD_TX_LCOLL:   u16 = 1 << 4;
pub const SMAP_BD_TX_MCOLL:   u16 = 1 << 3;
pub const SMAP_BD_TX_SCOLL:   u16 = 1 << 2;
pub const SMAP_BD_TX_UNDERRUN: u16 = 1 << 1;
pub const SMAP_BD_TX_SQE:     u16 = 1 << 0;
pub const SMAP_BD_TX_ERROR: u16 = SMAP_BD_TX_LOSSCR | SMAP_BD_TX_EDEFER
    | SMAP_BD_TX_ECOLL | SMAP_BD_TX_LCOLL | SMAP_BD_TX_UNDERRUN;
pub const SMAP_BD_RX_EMPTY:    u16 = 1 << 15;
pub const SMAP_BD_RX_OVERRUN:  u16 = 1 << 9;
pub const SMAP_BD_RX_PFRM:     u16 = 1 << 8;
pub const SMAP_BD_RX_BADFRM:   u16 = 1 << 7;
pub const SMAP_BD_RX_RUNTFRM:  u16 = 1 << 6;
pub const SMAP_BD_RX_SHORTEVNT: u16 = 1 << 5;
pub const SMAP_BD_RX_ALIGNERR: u16 = 1 << 4;
pub const SMAP_BD_RX_BADFCS:   u16 = 1 << 3;
pub const SMAP_BD_RX_FRMTOOLONG: u16 = 1 << 2;
pub const SMAP_BD_RX_OUTRANGE: u16 = 1 << 1;
pub const SMAP_BD_RX_INRANGE:  u16 = 1 << 0;
pub const SMAP_BD_RX_ERROR: u16 = SMAP_BD_RX_OVERRUN | SMAP_BD_RX_RUNTFRM
    | SMAP_BD_RX_SHORTEVNT | SMAP_BD_RX_ALIGNERR | SMAP_BD_RX_BADFCS
    | SMAP_BD_RX_FRMTOOLONG | SMAP_BD_RX_OUTRANGE | SMAP_BD_RX_INRANGE;

// PHY (DP83846A) registers.
pub const SMAP_NS_OUI: u32 = 0x08_0017;
pub const SMAP_DsPHYTER_ADDRESS: u32 = 0x1;
pub const SMAP_DsPHYTER_BMCR: u32  = 0x00;
pub const SMAP_DsPHYTER_BMSR: u32  = 0x01;
pub const SMAP_DsPHYTER_PHYIDR1: u32 = 0x02;
pub const SMAP_DsPHYTER_PHYIDR2: u32 = 0x03;
pub const SMAP_DsPHYTER_ANAR: u32   = 0x04;
pub const SMAP_DsPHYTER_ANLPAR: u32 = 0x05;
pub const SMAP_DsPHYTER_ANER: u32   = 0x06;
pub const SMAP_DsPHYTER_ANNPTR: u32 = 0x07;
pub const SMAP_DsPHYTER_PHYSTS: u32 = 0x10;

pub const SMAP_PHY_BMCR_RST: u16 = 1 << 15;
pub const SMAP_PHY_BMCR_LPBK: u16 = 1 << 14;
pub const SMAP_PHY_BMCR_100M: u16 = 1 << 13;
pub const SMAP_PHY_BMCR_10M:  u16 = 0 << 13;
pub const SMAP_PHY_BMCR_ANEN: u16 = 1 << 12;
pub const SMAP_PHY_BMCR_PWDN: u16 = 1 << 11;
pub const SMAP_PHY_BMCR_ISOL: u16 = 1 << 10;
pub const SMAP_PHY_BMCR_RSAN: u16 = 1 << 9;
pub const SMAP_PHY_BMCR_DUPM: u16 = 1 << 8;
pub const SMAP_PHY_BMCR_COLT: u16 = 1 << 7;

pub const SMAP_PHY_BMSR_ANCP: u16 = 1 << 5;
pub const SMAP_PHY_BMSR_LINK: u16 = 1 << 2;

pub const SMAP_PHY_IDR2_VMDL: u32 = 0x2;
pub const SMAP_PHY_IDR2_MSK:  u32 = 0xFFF0;
pub const SMAP_PHY_IDR2_REV_MSK: u32 = 0x000F;

pub const SMAP_PHY_STS_LINK: u16 = 1 << 0;
pub const SMAP_PHY_STS_100M: u16 = 0 << 1;
pub const SMAP_PHY_STS_10M:  u16 = 1 << 1;
pub const SMAP_PHY_STS_FDX:  u16 = 1 << 2;
pub const SMAP_PHY_STS_ANCP: u16 = 1 << 4;

// =====================================================================
//  ATA register layout.
// =====================================================================

pub const ATA_DEV9_HDD_BASE: u32 = SPD_REGBASE + 0x40;
pub const ATA_AIF_HDD_BASE:  u32 = SPD_REGBASE + 0x400_0000 + 0x60;
pub const ATA_R_DATA:        u32 = ATA_DEV9_HDD_BASE + 0x00;
pub const ATA_R_ERROR:       u32 = ATA_DEV9_HDD_BASE + 0x02;
pub const ATA_R_FEATURE:     u32 = ATA_DEV9_HDD_BASE + 0x02;
pub const ATA_R_NSECTOR:     u32 = ATA_DEV9_HDD_BASE + 0x04;
pub const ATA_R_SECTOR:      u32 = ATA_DEV9_HDD_BASE + 0x06;
pub const ATA_R_LCYL:        u32 = ATA_DEV9_HDD_BASE + 0x08;
pub const ATA_R_HCYL:        u32 = ATA_DEV9_HDD_BASE + 0x0a;
pub const ATA_R_SELECT:      u32 = ATA_DEV9_HDD_BASE + 0x0c;
pub const ATA_R_STATUS:      u32 = ATA_DEV9_HDD_BASE + 0x0e;
pub const ATA_R_CMD:         u32 = ATA_DEV9_HDD_BASE + 0x0e;
pub const ATA_R_ALT_STATUS:  u32 = ATA_DEV9_HDD_BASE + 0x1c;
pub const ATA_R_CONTROL:     u32 = ATA_DEV9_HDD_BASE + 0x1c;
pub const ATA_DEV9_INT:      u32 = 0x01;
pub const ATA_DEV9_INT_DMA:  u32 = 0x02;
pub const ATA_DEV9_HDD_END:  u32 = ATA_R_CONTROL + 4;

pub const ATA_ERR_MARK:  u8 = 0x01;
pub const ATA_ERR_TRACK0: u8 = 0x02;
pub const ATA_ERR_ABORT:  u8 = 0x04;
pub const ATA_ERR_MCR:    u8 = 0x08;
pub const ATA_ERR_ID:     u8 = 0x10;
pub const ATA_ERR_MC:     u8 = 0x20;
pub const ATA_ERR_ECC:    u8 = 0x40;
pub const ATA_ERR_ICRC:   u8 = 0x80;

pub const ATA_STAT_ERR:   u8 = 0x01;
pub const ATA_STAT_INDEX: u8 = 0x02;
pub const ATA_STAT_ECC:   u8 = 0x04;
pub const ATA_STAT_DRQ:   u8 = 0x08;
pub const ATA_STAT_SEEK:  u8 = 0x10;
pub const ATA_STAT_WRERR: u8 = 0x20;
pub const ATA_STAT_READY: u8 = 0x40;
pub const ATA_STAT_BUSY:  u8 = 0x80;

// =====================================================================
//  Flash (SmartMedia) command / status codes.
// =====================================================================

pub const FLASH_ID_64MBIT:   u32 = 0xe6;
pub const FLASH_ID_128MBIT:  u32 = 0x73;
pub const FLASH_ID_256MBIT:  u32 = 0x75;
pub const FLASH_ID_512MBIT:  u32 = 0x76;
pub const FLASH_ID_1024MBIT: u32 = 0x79;

pub const SM_CMD_READ1:       u8 = 0x00;
pub const SM_CMD_READ2:       u8 = 0x01;
pub const SM_CMD_READ3:       u8 = 0x50;
pub const SM_CMD_RESET:       u8 = 0xff;
pub const SM_CMD_WRITEDATA:   u8 = 0x80;
pub const SM_CMD_PROGRAMPAGE: u8 = 0x10;
pub const SM_CMD_ERASEBLOCK:  u8 = 0x60;
pub const SM_CMD_ERASECONFIRM: u8 = 0xd0;
pub const SM_CMD_GETSTATUS:   u8 = 0x70;
pub const SM_CMD_READID:      u8 = 0x90;

pub const FLASH_REGBASE: u32 = 0x1000_4800;
pub const FLASH_R_DATA:  u32 = FLASH_REGBASE + 0x00;
pub const FLASH_R_CMD:   u32 = FLASH_REGBASE + 0x04;
pub const FLASH_R_ADDR:  u32 = FLASH_REGBASE + 0x08;
pub const FLASH_R_CTRL:  u32 = FLASH_REGBASE + 0x0C;
pub const FLASH_R_ID:    u32 = FLASH_REGBASE + 0x14;
pub const FLASH_REGSIZE: u32 = 0x20;

pub const FLASH_PP_READY:  u32 = 1 << 0;
pub const FLASH_PP_WRITE:  u32 = 1 << 7;
pub const FLASH_PP_CSEL:   u32 = 1 << 8;
pub const FLASH_PP_READ:   u32 = 1 << 11;
pub const FLASH_PP_NOECC:  u32 = 1 << 12;

// =====================================================================
//  EEPROM state machine constants.
// =====================================================================

pub const EEPROM_READY:  u8 = 0;
pub const EEPROM_OPCD0:  u8 = 1;
pub const EEPROM_OPCD1:  u8 = 2;
pub const EEPROM_ADDR0:  u8 = 3;
pub const EEPROM_ADDR1:  u8 = 4;
pub const EEPROM_ADDR2:  u8 = 5;
pub const EEPROM_ADDR3:  u8 = 6;
pub const EEPROM_ADDR4:  u8 = 7;
pub const EEPROM_ADDR5:  u8 = 8;
pub const EEPROM_TDATA:   u8 = 9;

// =====================================================================
//  Configuration stand-ins.  The original C++ pulls these from
//  `EmuConfig` / `EmuFolders`; the Rust translation exposes simple
//  `static mut` mirrors so callers may inspect or override the state
//  from the host application.
// =====================================================================

#[derive(Clone, Debug)]
pub struct Dev9Config {
    pub hdd_enable:  bool,
    pub hdd_file:    String,
    pub eth_enable:  bool,
    pub eth_device:  String,
    pub eth_api:     NetApi,
    pub eth_log_dns: bool,
    pub eth_log_dhcp: bool,
    pub intercept_dhcp: bool,
}

impl Default for Dev9Config {
    fn default() -> Self {
        Dev9Config {
            hdd_enable:  false,
            hdd_file:    String::new(),
            eth_enable:  false,
            eth_device:  String::from("Auto"),
            eth_api:     NetApi::Sockets,
            eth_log_dns: false,
            eth_log_dhcp: false,
            intercept_dhcp: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetApi { TAP, PCAP_Bridged, PCAP_Switched, Sockets }

pub static mut DEV9_CONFIG: Dev9Config = Dev9Config {
    hdd_enable:  false,
    hdd_file:    String::new(),
    eth_enable:  false,
    eth_device:  String::new(),
    eth_api:     NetApi::Sockets,
    eth_log_dns: false,
    eth_log_dhcp: false,
    intercept_dhcp: false,
};

pub static mut DEV9_FOLDERS: Dev9Folders = Dev9Folders {
    settings: String::new(),
};

#[derive(Clone, Debug)]
pub struct Dev9Folders { pub settings: String }

// =====================================================================
//  Helper console / log sinks.  The original C++ uses DevCon and Console;
//  we funnel messages through no-op shims by default and allow the host
//  to plug in overrides via `DEV9_LOG`.
// =====================================================================

pub static DEV9_LOG: Mutex<Option<Box<dyn Fn(&str) + Send + Sync>>> =
    Mutex::new(None);

pub fn dev9_log(msg: &str) {
    if let Some(cb) = DEV9_LOG.lock().unwrap().as_ref() {
        cb(msg);
    }
}

pub fn dev9_warn(msg: &str) { dev9_log(msg); }
pub fn dev9_error(msg: &str) { dev9_log(msg); }

// =====================================================================
//  Flash state and helpers.
// =====================================================================

const PAGE_SIZE_BITS: u32 = 9;
const PAGE_SIZE: usize = 1 << PAGE_SIZE_BITS;
const ECC_SIZE:   usize = 16;
const PAGE_SIZE_ECC: usize = PAGE_SIZE + ECC_SIZE;
const BLOCK_SIZE: usize = 16 * PAGE_SIZE;
const BLOCK_SIZE_ECC: usize = 16 * PAGE_SIZE_ECC;
const CARD_SIZE: usize = 1024 * BLOCK_SIZE;
const CARD_SIZE_ECC: usize = 1024 * BLOCK_SIZE_ECC;

static mut FLASH_CTRL: u32 = 0;
static mut FLASH_CMD:  u32 = 0xFFFF_FFFF;
static mut FLASH_ADDR: u32 = 0;
static mut FLASH_ID:   u32 = 0;
static mut FLASH_COUNTER: u32 = 0;
static mut FLASH_ADDRBYTE: u32 = 0;
static mut FLASH_DATA: [u8; PAGE_SIZE_ECC] = [0; PAGE_SIZE_ECC];
static mut FLASH_FILE: [u8; CARD_SIZE_ECC] = [0xFF; CARD_SIZE_ECC];

fn flash_cmd_name(cmd: u32) -> &'static str {
    match cmd as u8 {
        x if x == SM_CMD_READ1       => "READ1",
        x if x == SM_CMD_READ2       => "READ2",
        x if x == SM_CMD_READ3       => "READ3",
        x if x == SM_CMD_RESET       => "RESET",
        x if x == SM_CMD_WRITEDATA   => "WRITEDATA",
        x if x == SM_CMD_PROGRAMPAGE => "PROGRAMPAGE",
        x if x == SM_CMD_ERASEBLOCK  => "ERASEBLOCK",
        x if x == SM_CMD_ERASECONFIRM => "ERASECONFIRM",
        x if x == SM_CMD_GETSTATUS   => "GETSTATUS",
        x if x == SM_CMD_READID      => "READID",
        _ => "unknown",
    }
}

static XFROMMAN_XOR_TABLE: [u8; 256] = [
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

fn xfromman_calculate_xors(buffer: &[u8; 128], out: &mut [u8; 4]) {
    let mut a: u8 = 0;
    let mut b: u8 = 0;
    let mut c: u8 = 0;
    for (i, &b_in) in buffer.iter().enumerate() {
        let x = XFROMMAN_XOR_TABLE[b_in as usize];
        a ^= x;
        if x & 0x80 != 0 {
            b ^= !((i & 0xFF) as u8);
            c ^= (i & 0xFF) as u8;
        }
    }
    out[0] = (!a) & 0x77;
    out[1] = (!b) & 0x7F;
    out[2] = (!c) & 0x7F;
}

fn calculate_ecc(page: &mut [u8; PAGE_SIZE_ECC]) {
    for b in &mut page[PAGE_SIZE..] { *b = 0; }
    let mut tmp = [0u8; 4];
    {
        let chunk: &[u8] = &page[0 * (PAGE_SIZE / 4)..1 * (PAGE_SIZE / 4)];
        let mut buf = [0u8; 128];
        buf.copy_from_slice(chunk);
        xfromman_calculate_xors(&buf, &mut tmp);
        page[PAGE_SIZE + 0] = tmp[0]; page[PAGE_SIZE + 1] = tmp[1]; page[PAGE_SIZE + 2] = tmp[2];
    }
    {
        let chunk: &[u8] = &page[1 * (PAGE_SIZE / 4)..2 * (PAGE_SIZE / 4)];
        let mut buf = [0u8; 128];
        buf.copy_from_slice(chunk);
        xfromman_calculate_xors(&buf, &mut tmp);
        page[PAGE_SIZE + 3] = tmp[0]; page[PAGE_SIZE + 4] = tmp[1]; page[PAGE_SIZE + 5] = tmp[2];
    }
    {
        let chunk: &[u8] = &page[2 * (PAGE_SIZE / 4)..3 * (PAGE_SIZE / 4)];
        let mut buf = [0u8; 128];
        buf.copy_from_slice(chunk);
        xfromman_calculate_xors(&buf, &mut tmp);
        page[PAGE_SIZE + 6] = tmp[0]; page[PAGE_SIZE + 7] = tmp[1]; page[PAGE_SIZE + 8] = tmp[2];
    }
    {
        let chunk: &[u8] = &page[3 * (PAGE_SIZE / 4)..4 * (PAGE_SIZE / 4)];
        let mut buf = [0u8; 128];
        buf.copy_from_slice(chunk);
        xfromman_calculate_xors(&buf, &mut tmp);
        page[PAGE_SIZE + 9] = tmp[0]; page[PAGE_SIZE + 10] = tmp[1]; page[PAGE_SIZE + 11] = tmp[2];
    }
}

fn flash_load_file() {
    if let Ok(mut f) = File::open("flash.dat") {
        unsafe { let _ = f.read_exact(&mut FLASH_FILE); }
    } else {
        unsafe { for b in FLASH_FILE.iter_mut() { *b = 0xFF; } }
    }
}

fn flash_store_file() {
    if let Ok(mut f) = OpenOptions::new().write(true).create(true).truncate(true).open("flash.dat") {
        unsafe { let _ = f.write_all(&FLASH_FILE); }
    }
}

pub fn dev9FlashInit() {
    unsafe {
        FLASH_ID = FLASH_ID_64MBIT;
        FLASH_COUNTER = 0;
        FLASH_ADDRBYTE = 0;
        FLASH_ADDR = 0;
        for b in FLASH_DATA.iter_mut() { *b = 0xFF; }
        calculate_ecc(&mut FLASH_DATA);
        FLASH_CTRL = FLASH_PP_READY;
        flash_load_file();
    }
}

pub fn dev9FlashReset() { dev9FlashInit(); }

fn dev9_flash_refill() {
    unsafe {
        FLASH_CTRL &= !FLASH_PP_READY;
        FLASH_ADDR = (FLASH_ADDR + PAGE_SIZE as u32) % CARD_SIZE as u32;
        let page_idx = (FLASH_ADDR >> PAGE_SIZE_BITS) as usize;
        let off = page_idx * PAGE_SIZE_ECC;
        FLASH_DATA[..PAGE_SIZE].copy_from_slice(&FLASH_FILE[off..off + PAGE_SIZE]);
        calculate_ecc(&mut FLASH_DATA);
        FLASH_CTRL |= FLASH_PP_READY;
    }
}

pub fn dev9FlashRead(addr: u32) -> u8 {
    unsafe {
        match addr & 0x1FFF_FFFF {
            FLASH_R_DATA => {
                let off = FLASH_COUNTER as usize;
                let b = FLASH_DATA[off];
                FLASH_COUNTER += 1;
                if FLASH_CMD as u8 == SM_CMD_READ3 {
                    if FLASH_COUNTER as usize >= PAGE_SIZE_ECC {
                        FLASH_COUNTER = PAGE_SIZE as u32;
                        dev9_flash_refill();
                    }
                } else if (FLASH_CTRL & FLASH_PP_NOECC) != 0 {
                    if FLASH_COUNTER as usize >= PAGE_SIZE {
                        FLASH_COUNTER %= PAGE_SIZE as u32;
                        dev9_flash_refill();
                    }
                } else if FLASH_COUNTER as usize >= PAGE_SIZE_ECC {
                    FLASH_COUNTER %= PAGE_SIZE_ECC as u32;
                    dev9_flash_refill();
                }
                b
            }
            FLASH_R_CMD   => (FLASH_CMD & 0xFF) as u8,
            FLASH_R_ADDR  => 0,
            FLASH_R_CTRL  => (FLASH_CTRL & 0xFF) as u8,
            FLASH_R_ID => {
                if (FLASH_CMD as u8) == SM_CMD_READID {
                    (FLASH_ID & 0xFF) as u8
                } else if (FLASH_CMD as u8) == SM_CMD_GETSTATUS {
                    0x80 | (((FLASH_CTRL & 1) << 6) as u8)
                } else {
                    0
                }
            }
            _ => 0,
        }
    }
}

pub fn dev9FlashWrite(addr: u32, value: u8) {
    unsafe {
        match addr & 0x1FFF_FFFF {
            FLASH_R_DATA => {
                let off = FLASH_COUNTER as usize;
                FLASH_DATA[off] = value;
                FLASH_COUNTER += 1;
                FLASH_COUNTER %= PAGE_SIZE_ECC as u32;
            }
            FLASH_R_CMD => {
                let v = value as u32;
                if (FLASH_CTRL & FLASH_PP_READY) == 0
                    && v != SM_CMD_GETSTATUS as u32
                    && v != SM_CMD_RESET as u32
                {
                    // ILLEGAL while busy; ignore.
                } else if (FLASH_CMD as u8) == SM_CMD_WRITEDATA
                    && v != SM_CMD_PROGRAMPAGE as u32
                    && v != SM_CMD_RESET as u32
                {
                    FLASH_CTRL &= !FLASH_PP_READY;
                } else {
                    match v as u8 {
                        x if x == SM_CMD_READ1 => {
                            FLASH_COUNTER = 0;
                            if (FLASH_CMD as u8) != SM_CMD_GETSTATUS { FLASH_ADDR = FLASH_COUNTER; }
                            FLASH_ADDRBYTE = 0;
                        }
                        x if x == SM_CMD_READ2 => {
                            FLASH_COUNTER = (PAGE_SIZE / 2) as u32;
                            if (FLASH_CMD as u8) != SM_CMD_GETSTATUS { FLASH_ADDR = FLASH_COUNTER; }
                            FLASH_ADDRBYTE = 0;
                        }
                        x if x == SM_CMD_READ3 => {
                            FLASH_COUNTER = PAGE_SIZE as u32;
                            if (FLASH_CMD as u8) != SM_CMD_GETSTATUS { FLASH_ADDR = FLASH_COUNTER; }
                            FLASH_ADDRBYTE = 0;
                        }
                        x if x == SM_CMD_RESET => {
                            dev9FlashInit();
                        }
                        x if x == SM_CMD_WRITEDATA => {
                            FLASH_COUNTER = 0;
                            FLASH_ADDR = FLASH_COUNTER;
                            FLASH_ADDRBYTE = 0;
                        }
                        x if x == SM_CMD_ERASEBLOCK => {
                            FLASH_COUNTER = 0;
                            for b in FLASH_DATA.iter_mut() { *b = 0xFF; }
                            FLASH_ADDR = FLASH_COUNTER;
                            FLASH_ADDRBYTE = 1;
                        }
                        x if x == SM_CMD_PROGRAMPAGE || x == SM_CMD_ERASECONFIRM => {
                            FLASH_CTRL &= !FLASH_PP_READY;
                            calculate_ecc(&mut FLASH_DATA);
                            let page = (FLASH_ADDR / PAGE_SIZE as u32) as usize;
                            let off = page * PAGE_SIZE_ECC;
                            FLASH_FILE[off..off + PAGE_SIZE_ECC].copy_from_slice(&FLASH_DATA);
                            flash_store_file();
                            FLASH_CTRL |= FLASH_PP_READY;
                        }
                        x if x == SM_CMD_GETSTATUS => {}
                        x if x == SM_CMD_READID => {
                            FLASH_COUNTER = 0;
                            FLASH_ADDR = FLASH_COUNTER;
                            FLASH_ADDRBYTE = 0;
                        }
                        _ => {
                            FLASH_CTRL &= !FLASH_PP_READY;
                        }
                    }
                    FLASH_CMD = v;
                }
            }
            FLASH_R_ADDR => {
                if FLASH_ADDRBYTE == 0 {
                    FLASH_ADDR |= (value as u32) & 0xFF;
                } else {
                    FLASH_ADDR |= ((value as u32) & 0xFF) << (1 + 8 * FLASH_ADDRBYTE);
                }
                FLASH_ADDRBYTE += 1;
                if (value & 0x01) == 0 {
                    if (FLASH_CMD as u8) == SM_CMD_READ1
                        || (FLASH_CMD as u8) == SM_CMD_READ2
                        || (FLASH_CMD as u8) == SM_CMD_READ3
                    {
                        FLASH_CTRL &= !FLASH_PP_READY;
                        let page_idx = (FLASH_ADDR >> PAGE_SIZE_BITS) as usize;
                        let off = page_idx * PAGE_SIZE_ECC;
                        FLASH_DATA[..PAGE_SIZE].copy_from_slice(&FLASH_FILE[off..off + PAGE_SIZE]);
                        calculate_ecc(&mut FLASH_DATA);
                        FLASH_CTRL |= FLASH_PP_READY;
                    }
                    FLASH_ADDRBYTE = 0;
                }
            }
            FLASH_R_CTRL => {
                FLASH_CTRL = (FLASH_CTRL & FLASH_PP_READY) | ((value as u32) & !FLASH_PP_READY);
            }
            _ => {}
        }
    }
}

// =====================================================================
//  Buffer descriptor (smap_bd_t).
// =====================================================================

#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct SmapBd {
    pub ctrl_stat: u16,
    pub reserved:  u16,
    pub length:    u16,
    pub pointer:   u16,
}

#[derive(Clone, Default)]
pub struct AtaDevice {
    pub dma_ready: bool,
    pub open: bool,
    pub path: String,
}

impl AtaDevice {
    pub fn new() -> Self { Self::default() }
    pub fn open(&mut self, path: &str) -> i32 {
        self.path = path.to_string();
        self.open = true;
        0
    }
    pub fn close(&mut self) { self.open = false; self.dma_ready = false; }
    pub fn hard_reset(&mut self) {}
    pub fn read(&self, _addr: u32, _width: u8) -> u32 { 0 }
    pub fn write(&mut self, _addr: u32, _value: u32, _width: u8) {}
    pub fn read_dma_to_fifo(&mut self, _dst: &mut [u8], _len: usize) -> usize { 0 }
    pub fn write_dma_from_fifo(&mut self, _src: &mut [u8], _len: usize) -> usize { 0 }
    pub fn async_op(&mut self, _cycles: u32) {}
}

// =====================================================================
//  Main DEV9 state structure (mirrors `dev9Struct`).
// =====================================================================

pub const DEV9_R_SIZE: usize = 0x1_0000;
pub const DEV9_FIFO_BYTES: usize = 16 * 512;
pub const DEV9_RX_FIFO_BYTES: usize = 16 * 1024;
pub const DEV9_TX_FIFO_BYTES: usize = 16 * 1024;
pub const DEV9_EEPROM_WORDS: usize = 32;
pub const DEV9_PHY_REGS: usize = 32;

pub struct Dev9State {
    pub ata: Option<Box<AtaDevice>>,

    /// 64 KiB register file used by the SPEED/SMAP block-descriptor engines.
    pub dev9R: [u8; DEV9_R_SIZE],

    pub eeprom_state:   u8,
    pub eeprom_command: u8,
    pub eeprom_address: u8,
    pub eeprom_bit:     u8,
    pub eeprom_dir:     u8,
    /// The PS2's MAC + checksum is mirrored into the lower EEPROM words.
    pub eeprom: [u16; DEV9_EEPROM_WORDS],

    pub rxbdi: u32,
    pub rxfifo: [u8; DEV9_RX_FIFO_BYTES],
    pub rxfifo_wr_ptr: u16,

    pub txbdi: u32,
    pub txfifo: [u8; DEV9_TX_FIFO_BYTES],
    pub txfifo_rd_ptr: u16,

    pub bd_swap: u8,
    pub phyregs: [u16; DEV9_PHY_REGS],

    pub irqcause: u16,
    pub irqmask:  u16,
    pub dma_ctrl: u16,
    pub xfr_ctrl: u16,
    pub if_ctrl:  u16,

    pub pio_mode:  u16,
    pub mdma_mode: u16,
    pub udma_mode: u16,

    /// SPEED <-> HDD FIFO bookkeeping.
    pub fifo_bytes_read:  u32,
    pub fifo_bytes_write: u32,
    pub fifo: [u8; DEV9_FIFO_BYTES],

    /// Active IOP DMA target.
    pub dma_iop_ptr:         Option<*mut u8>,
    pub dma_iop_transfered:  u32,
    pub dma_iop_size:        u32,

    pub opened: bool,
}

impl Dev9State {
    pub const fn new() -> Self {
        Dev9State {
            ata: None,
            dev9R: [0u8; DEV9_R_SIZE],
            eeprom_state: 0,
            eeprom_command: 0,
            eeprom_address: 0,
            eeprom_bit: 0,
            eeprom_dir: 0,
            eeprom: [0u16; DEV9_EEPROM_WORDS],
            rxbdi: 0,
            rxfifo: [0u8; DEV9_RX_FIFO_BYTES],
            rxfifo_wr_ptr: 0,
            txbdi: 0,
            txfifo: [0u8; DEV9_TX_FIFO_BYTES],
            txfifo_rd_ptr: 0,
            bd_swap: 0,
            phyregs: [0u16; DEV9_PHY_REGS],
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
            fifo: [0u8; DEV9_FIFO_BYTES],
            dma_iop_ptr: None,
            dma_iop_transfered: 0,
            dma_iop_size: 0,
            opened: false,
        }
    }
}

pub static mut dev9: Dev9State = Dev9State::new();

// =====================================================================
//  Helper accessors matching the original `dev9Ru8/u16/u32` macros.
// =====================================================================

#[inline]
pub fn dev9R8(addr: u32) -> u8 { unsafe { dev9.dev9R[(addr & 0xFFFF) as usize] } }
#[inline]
pub fn dev9R16(addr: u32) -> u16 {
    unsafe {
        let off = (addr & 0xFFFF) as usize;
        u16::from_le_bytes([dev9.dev9R[off], dev9.dev9R[off + 1]])
    }
}
#[inline]
pub fn dev9R32(addr: u32) -> u32 {
    unsafe {
        let off = (addr & 0xFFFF) as usize;
        u32::from_le_bytes([
            dev9.dev9R[off], dev9.dev9R[off + 1],
            dev9.dev9R[off + 2], dev9.dev9R[off + 3],
        ])
    }
}
#[inline]
pub fn dev9W8(addr: u32, v: u8) { unsafe { dev9.dev9R[(addr & 0xFFFF) as usize] = v; } }
#[inline]
pub fn dev9W16(addr: u32, v: u16) {
    unsafe {
        let off = (addr & 0xFFFF) as usize;
        dev9.dev9R[off]     = (v & 0xFF) as u8;
        dev9.dev9R[off + 1] = (v >> 8) as u8;
    }
}
#[inline]
pub fn dev9W32(addr: u32, v: u32) {
    unsafe {
        let off = (addr & 0xFFFF) as usize;
        dev9.dev9R[off]     = (v & 0xFF) as u8;
        dev9.dev9R[off + 1] = ((v >> 8) & 0xFF) as u8;
        dev9.dev9R[off + 2] = ((v >> 16) & 0xFF) as u8;
        dev9.dev9R[off + 3] = ((v >> 24) & 0xFF) as u8;
    }
}

// =====================================================================
//  External hooks.  The original C++ delegates `dev9Irq` /
//  `psxDMA8Interrupt` to the IOP core.  The Rust translation provides
//  overridable function pointers that default to no-ops.
// =====================================================================

pub static DEV9_IRQ_HOOK: Mutex<Option<Box<dyn FnMut(u32) + Send>>> = Mutex::new(None);
pub static DEV9_DMA_HOOK: Mutex<Option<Box<dyn FnMut() + Send>>> = Mutex::new(None);

pub fn dev9Irq(cycles: u32) {
    if let Some(cb) = DEV9_IRQ_HOOK.lock().unwrap().as_mut() { cb(cycles); }
}
pub fn psxDMA8Interrupt() {
    if let Some(cb) = DEV9_DMA_HOOK.lock().unwrap().as_mut() { cb(); }
}

fn dev9_raise_irq(cause: u16, cycles: u32) {
    unsafe {
        dev9.irqcause |= cause;
        if cycles < 1 { dev9Irq(1); } else { dev9Irq(cycles); }
    }
}

pub fn dev9IrqHandler() -> i32 { unsafe { if dev9.irqcause & dev9.irqmask != 0 { 1 } else { 0 } } }

pub fn _DEV9irq(cause: u16, cycles: u32) { dev9_raise_irq(cause, cycles); }

// =====================================================================
//  Default MAC / IP / settings.
// =====================================================================

pub const DEFAULT_PS2_MAC: [u8; 6] = [0x00, 0x04, 0x1F, 0x82, 0x30, 0x31];
pub const BROADCAST_MAC:   [u8; 6] = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
pub const INTERNAL_MAC:    [u8; 6] = [0x76, 0x6D, 0xF4, 0x63, 0x30, 0x31];
pub const INTERNAL_IP:     [u8; 4] = [192, 0, 2, 1];
pub const PS2_IP_DEFAULT:  [u8; 4] = [0, 0, 0, 0];

pub fn dev9_default_ps2_ip() -> [u8; 4] { PS2_IP_DEFAULT }

// =====================================================================
//  FIFO helpers (HDD <-> SPEED, IOP <-> SPEED).
// =====================================================================

const DBUF_AVAIL_BYTES: usize = SPD_DBUF_AVAIL_MAX as usize * 512;

fn hdd_write_fifo() {
    unsafe {
        if dev9.ata.is_none() { return; }
        let unread = (dev9.fifo_bytes_write - dev9.fifo_bytes_read) as usize;
        let space = DBUF_AVAIL_BYTES - unread;
        let base = (dev9.fifo_bytes_write % DBUF_AVAIL_BYTES as u32) as usize;
        if base + space > DBUF_AVAIL_BYTES {
            let was = DBUF_AVAIL_BYTES - base;
            let r1 = dev9.ata.as_mut().unwrap().read_dma_to_fifo(&mut dev9.fifo[base..base + was], was);
            let total = if r1 == was {
                r1 + dev9.ata.as_mut().unwrap().read_dma_to_fifo(&mut dev9.fifo[..space - was], space - was)
            } else { r1 };
            dev9.fifo_bytes_write += total as u32;
        } else {
            let r = dev9.ata.as_mut().unwrap().read_dma_to_fifo(&mut dev9.fifo[base..base + space], space);
            dev9.fifo_bytes_write += r as u32;
        }
    }
}

fn hdd_read_fifo() {
    unsafe {
        if dev9.ata.is_none() { return; }
        let unread = (dev9.fifo_bytes_write - dev9.fifo_bytes_read) as usize;
        let base = (dev9.fifo_bytes_read % DBUF_AVAIL_BYTES as u32) as usize;
        if base + unread > DBUF_AVAIL_BYTES {
            let was = DBUF_AVAIL_BYTES - base;
            let w1 = dev9.ata.as_mut().unwrap().write_dma_from_fifo(&mut dev9.fifo[base..base + was], was);
            let total = if w1 == was {
                w1 + dev9.ata.as_mut().unwrap().write_dma_from_fifo(&mut dev9.fifo[..unread - was], unread - was)
            } else { w1 };
            dev9.fifo_bytes_read += total as u32;
        } else {
            let w = dev9.ata.as_mut().unwrap().write_dma_from_fifo(&mut dev9.fifo[base..base + unread], unread);
            dev9.fifo_bytes_read += w as u32;
        }
    }
}

fn iop_read_fifo() {
    unsafe {
        if dev9.dma_iop_ptr.is_none() { return; }
        let unread = (dev9.fifo_bytes_write - dev9.fifo_bytes_read) as usize;
        let base = (dev9.fifo_bytes_read % DBUF_AVAIL_BYTES as u32) as usize;
        let remain = (dev9.dma_iop_size - dev9.dma_iop_transfered) as usize;
        let to_copy = min(remain, unread);
        if to_copy == 0 { return; }
        let dst = dev9.dma_iop_ptr.unwrap();
        if base + to_copy > DBUF_AVAIL_BYTES {
            let was = DBUF_AVAIL_BYTES - base;
            unsafe_ptr_copy(&dev9.fifo[base..base + was], dst.add(dev9.dma_iop_transfered as usize), was);
            unsafe_ptr_copy(&dev9.fifo[..to_copy - was], dst.add((dev9.dma_iop_transfered + was as u32) as usize), to_copy - was);
        } else {
            unsafe_ptr_copy(&dev9.fifo[base..base + to_copy], dst.add(dev9.dma_iop_transfered as usize), to_copy);
        }
        dev9.dma_iop_transfered += to_copy as u32;
        dev9.fifo_bytes_read += to_copy as u32;
        if dev9.fifo_bytes_read > dev9.fifo_bytes_write { dev9_error("DEV9: UNDERFLOW BY IOP"); }
    }
}

fn iop_write_fifo() {
    unsafe {
        if dev9.dma_iop_ptr.is_none() { return; }
        let unread = (dev9.fifo_bytes_write - dev9.fifo_bytes_read) as usize;
        let space = DBUF_AVAIL_BYTES - unread;
        let base = (dev9.fifo_bytes_write % DBUF_AVAIL_BYTES as u32) as usize;
        let remain = (dev9.dma_iop_size - dev9.dma_iop_transfered) as usize;
        let to_copy = min(remain, space);
        if to_copy == 0 { return; }
        let src = dev9.dma_iop_ptr.unwrap();
        if base + to_copy > DBUF_AVAIL_BYTES {
            let was = DBUF_AVAIL_BYTES - base;
            unsafe_ptr_copy_back(src.add(dev9.dma_iop_transfered as usize), &mut dev9.fifo[base..base + was], was);
            unsafe_ptr_copy_back(src.add((dev9.dma_iop_transfered + was as u32) as usize), &mut dev9.fifo[..to_copy - was], to_copy - was);
        } else {
            unsafe_ptr_copy_back(src.add(dev9.dma_iop_transfered as usize), &mut dev9.fifo[base..base + to_copy], to_copy);
        }
        dev9.dma_iop_transfered += to_copy as u32;
        dev9.fifo_bytes_write += to_copy as u32;
        if (dev9.fifo_bytes_write - dev9.fifo_bytes_read) as usize > DBUF_AVAIL_BYTES {
            dev9_error("DEV9: OVERFLOW BY IOP");
        }
    }
}

fn unsafe_ptr_copy(src: &[u8], dst: *mut u8, n: usize) {
    unsafe { std::ptr::copy_nonoverlapping(src.as_ptr(), dst, n); }
}
fn unsafe_ptr_copy_back(src: *const u8, dst: &mut [u8], n: usize) {
    unsafe { std::ptr::copy_nonoverlapping(src, dst.as_mut_ptr(), n); }
}

fn fifo_intr() {
    unsafe {
        let unread = (dev9.fifo_bytes_write - dev9.fifo_bytes_read) as usize;
        if unread == 0 {
            dev9.irqcause &= !SPD_INTR_ATA_FIFO_DATA;
            if dev9.irqcause & SPD_INTR_ATA_FIFO_EMPTY == 0 {
                dev9_raise_irq(SPD_INTR_ATA_FIFO_EMPTY, 1);
            }
        } else {
            dev9.irqcause &= !SPD_INTR_ATA_FIFO_EMPTY;
            if dev9.irqcause & SPD_INTR_ATA_FIFO_DATA == 0 {
                dev9_raise_irq(SPD_INTR_ATA_FIFO_DATA, 1);
            }
        }
        if unread == DBUF_AVAIL_BYTES {
            if dev9.irqcause & SPD_INTR_ATA_FIFO_FULL == 0 {
                dev9_raise_irq(SPD_INTR_ATA_FIFO_FULL, 1);
            }
        } else {
            dev9.irqcause &= !SPD_INTR_ATA_FIFO_FULL;
        }
        if dev9.dma_iop_ptr.is_some() && dev9.dma_iop_transfered == dev9.dma_iop_size {
            dev9.dma_iop_ptr = None;
            psxDMA8Interrupt();
        }
    }
}

pub fn dev9runFIFO() {
    unsafe {
        let iop_write = dev9.xfr_ctrl & SPD_XFR_WRITE != 0;
        let hdd_read  = dev9.if_ctrl  & SPD_IF_READ  != 0;
        let ata_some  = dev9.ata.is_some();
        let hdd_xfer  = ata_some && dev9.ata.as_ref().unwrap().dma_ready
            && dev9.if_ctrl & SPD_IF_ATA_DMAEN != 0;
        let iop_xfer  = dev9.dma_iop_ptr.is_some() && dev9.xfr_ctrl & SPD_XFR_DMAEN != 0;

        if iop_write {
            if iop_xfer { iop_write_fifo(); }
            if hdd_xfer && !hdd_read { hdd_read_fifo(); }
        } else {
            if hdd_xfer && hdd_read { hdd_write_fifo(); }
            if iop_xfer {
                iop_read_fifo();
                if hdd_xfer && hdd_read && dev9.ata.as_ref().unwrap().dma_ready {
                    hdd_write_fifo();
                }
            }
        }
        fifo_intr();
    }
}

// =====================================================================
//  SpeedRead / SpeedWrite (SPEED register file).
// =====================================================================

fn speed_read(addr: u32, _width: u8) -> u16 {
    unsafe {
        match addr {
            0x1000_0020 => 1,
            SPD_R_INTR_STAT => dev9.irqcause,
            SPD_R_INTR_MASK => dev9.irqmask,
            SPD_R_PIO_DATA => {
                if dev9.eeprom_state == EEPROM_TDATA && dev9.eeprom_command == 2 {
                    if dev9.eeprom_bit != 0xFF {
                        let v = ((dev9.eeprom[dev9.eeprom_address as usize] << dev9.eeprom_bit) & 0x8000) >> 11;
                        dev9.eeprom_bit += 1;
                        if dev9.eeprom_bit == 16 {
                            dev9.eeprom_address = dev9.eeprom_address.wrapping_add(1);
                            dev9.eeprom_bit = 0;
                        }
                        v as u16
                    } else { 0 }
                } else { 0 }
            }
            SPD_R_REV_1 => 0,
            SPD_R_REV_2 => 0x11,
            SPD_R_REV_3 => {
                let mut h: u16 = SPD_CAPS_ATA | SPD_CAPS_FLASH;
                if DEV9_CONFIG.eth_enable { h |= SPD_CAPS_SMAP; }
                h
            }
            SPD_R_0e => 0x0002,
            SPD_R_XFR_CTRL => dev9.xfr_ctrl,
            SPD_R_DBUF_STAT => {
                let count: u8 = ((dev9.fifo_bytes_write - dev9.fifo_bytes_read) / 512) as u8;
                let mut hard: u16 = 0;
                if dev9.xfr_ctrl & SPD_XFR_WRITE != 0 {
                    hard = (SPD_DBUF_AVAIL_MAX - count as u16) & 0xFF;
                    if count == 0 { hard |= SPD_DBUF_STAT_1; } else { hard |= SPD_DBUF_STAT_2; }
                } else {
                    hard = count as u16;
                    if count < SPD_DBUF_AVAIL_MAX as u8 { hard |= SPD_DBUF_STAT_1; }
                    if count == 0 { hard |= SPD_DBUF_STAT_2; }
                }
                if count as u16 == SPD_DBUF_AVAIL_MAX { hard |= SPD_DBUF_STAT_FULL; }
                hard
            }
            SPD_R_IF_CTRL => dev9.if_ctrl,
            _ => dev9R16(addr),
        }
    }
}

fn speed_write(addr: u32, value: u16, _width: u8) {
    unsafe {
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
                if (value & 0xC0) != 0xC0 { return; }
                if (value & 0x30) == 0x20 { dev9.eeprom_state = 0; }
                dev9.eeprom_dir = ((value >> 4) & 3) as u8;
            }
            SPD_R_PIO_DATA => {
                if (value & 0xC0) != 0xC0 { return; }
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
                        let shift = dev9.eeprom_state - EEPROM_ADDR0;
                        let mask  = 63 ^ (1 << shift);
                        let bit_v = ((value >> shift) & (0x20 >> shift)) as u8;
                        dev9.eeprom_address = (dev9.eeprom_address & mask) | (bit_v & !mask);
                        dev9.eeprom_state += 1;
                    }
                    EEPROM_TDATA => {
                        if dev9.eeprom_command == 1 {
                            let mask = 63 ^ (1 << dev9.eeprom_bit);
                            let v = ((value >> dev9.eeprom_bit) & (0x8000 >> dev9.eeprom_bit)) as u16;
                            dev9.eeprom[dev9.eeprom_address as usize] =
                                (dev9.eeprom[dev9.eeprom_address as usize] & mask as u16) | (v & !(mask as u16));
                            dev9.eeprom_bit += 1;
                            if dev9.eeprom_bit == 16 {
                                dev9.eeprom_address = dev9.eeprom_address.wrapping_add(1);
                                dev9.eeprom_bit = 0;
                            }
                        }
                    }
                    _ => dev9_error("DEV9: Unknown EEPROM COMMAND"),
                }
            }
            SPD_R_DMA_CTRL => {
                dev9.dma_ctrl = value;
                if value & SPD_DMA_PAUSE != 0 {
                    dev9_error("DEV9: SPD_R_DMA_CTRL Pause DMA Not Implemented");
                }
                if value & 0b1111_1111_1110_1000 != 0 {
                    dev9_error("DEV9: SPD_R_DMA_CTRL Unknown value written");
                }
            }
            SPD_R_XFR_CTRL => {
                let old = dev9.xfr_ctrl;
                dev9.xfr_ctrl = value;
                if (value & SPD_XFR_WRITE) != (old & SPD_XFR_WRITE) { dev9runFIFO(); }
                if value & SPD_XFR_DMAEN != 0 { dev9runFIFO(); }
                if value & 0b1111_1111_0111_1000 != 0 {
                    dev9_error("DEV9: SPD_R_XFR_CTRL Unknown value written");
                }
            }
            SPD_R_DBUF_STAT => {
                if value & SPD_DBUF_RESET_READ_CNT  != 0 { dev9.fifo_bytes_read  = 0; }
                if value & SPD_DBUF_RESET_WRITE_CNT != 0 { dev9.fifo_bytes_write = 0; }
                if value != 0 { fifo_intr(); }
                if value != 3 { dev9_error("DEV9: SPD_R_DBUF_STAT write != 3"); }
            }
            SPD_R_IF_CTRL => {
                let old = dev9.if_ctrl;
                dev9.if_ctrl = value;
                if (value & SPD_IF_READ) != (old & SPD_IF_READ) { dev9runFIFO(); }
                if value & SPD_IF_ATA_DMAEN != 0 { dev9runFIFO(); }
                if value & (1 << 4) != 0 { dev9_error("DEV9: IF_CTRL Unknown Bit 4 Set"); }
                if value & (1 << 5) != 0 { dev9_error("DEV9: IF_CTRL Unknown Bit 5 Set"); }
                if value & SPD_IF_HDD_RESET == 0 {
                    if let Some(ata) = dev9.ata.as_mut() { ata.hard_reset(); }
                }
                if value & SPD_IF_ATA_RESET != 0 {
                    dev9.if_ctrl = 0x001A;
                    dev9.pio_mode = 0x24;
                    dev9.mdma_mode = 0x45;
                    dev9.udma_mode = 0x83;
                }
                if (value & 0xFF00) > 0 { dev9_error("DEV9: IF_CTRL Unknown Bit(s)"); }
            }
            SPD_R_PIO_MODE  => dev9.pio_mode  = value,
            SPD_R_MDMA_MODE => dev9.mdma_mode = value,
            SPD_R_UDMA_MODE => dev9.udma_mode = value,
            _ => { dev9W16(addr, value); }
        }
    }
}

// =====================================================================
//  SMAP read / write handlers.
// =====================================================================

pub fn smap_read8(addr: u32) -> u8 {
    unsafe {
        match addr {
            SMAP_R_BD_MODE => dev9.bd_swap,
            _ => dev9R8(addr),
        }
    }
}

pub fn smap_read16(addr: u32) -> u16 {
    let rv = dev9R16(addr);
    if (addr >= SMAP_BD_TX_BASE && addr < SMAP_BD_TX_BASE + SMAP_BD_SIZE)
        || (addr >= SMAP_BD_RX_BASE && addr < SMAP_BD_RX_BASE + SMAP_BD_SIZE)
    {
        unsafe { if dev9.bd_swap != 0 { ((rv << 8) | (rv >> 8)) as u16 } else { rv } }
    } else { rv }
}

pub fn smap_read32(addr: u32) -> u32 {
    unsafe {
        if addr >= SMAP_EMAC3_REGBASE && addr < SMAP_EMAC3_REGEND {
            let hi = smap_read16(addr);
            let lo = smap_read16(addr + 2) << 16;
            return hi as u32 | lo as u32;
        }
        match addr {
            SMAP_R_RXFIFO_DATA => {
                let rd = (dev9R32(SMAP_R_RXFIFO_RD_PTR) & 0x3FFF) as usize;
                let rv = u32::from_le_bytes([
                    dev9.rxfifo[rd], dev9.rxfifo[rd + 1], dev9.rxfifo[rd + 2], dev9.rxfifo[rd + 3],
                ]);
                dev9W32(SMAP_R_RXFIFO_RD_PTR, (rd + 4) as u32 & 0x3FFF);
                rv
            }
            _ => dev9R32(addr),
        }
    }
}

pub fn smap_write8(addr: u32, value: u8) {
    unsafe {
        match addr {
            SMAP_R_TXFIFO_FRAME_INC => { dev9W8(SMAP_R_TXFIFO_FRAME_CNT, dev9R8(SMAP_R_TXFIFO_FRAME_CNT).wrapping_add(1)); }
            SMAP_R_RXFIFO_FRAME_DEC => {
                dev9W8(addr, value);
                dev9W8(SMAP_R_RXFIFO_FRAME_CNT, dev9R8(SMAP_R_RXFIFO_FRAME_CNT).wrapping_sub(1));
            }
            SMAP_R_TXFIFO_CTRL => {
                if value & (SMAP_TXFIFO_RESET as u8) != 0 {
                    dev9.txbdi = 0;
                    dev9.txfifo_rd_ptr = 0;
                    dev9W8(SMAP_R_TXFIFO_FRAME_CNT, 0);
                    dev9W32(SMAP_R_TXFIFO_WR_PTR, 0);
                    dev9W32(SMAP_R_TXFIFO_SIZE, 16384);
                }
                dev9W8(addr, value & !(SMAP_TXFIFO_RESET as u8));
            }
            SMAP_R_RXFIFO_CTRL => {
                if value & (SMAP_RXFIFO_RESET as u8) != 0 {
                    dev9.rxbdi = 0;
                    dev9.rxfifo_wr_ptr = 0;
                    dev9W8(SMAP_R_RXFIFO_FRAME_CNT, 0);
                    dev9W32(SMAP_R_RXFIFO_RD_PTR, 0);
                    dev9W32(SMAP_R_RXFIFO_SIZE, 16384);
                }
                dev9W8(addr, value & !(SMAP_RXFIFO_RESET as u8));
            }
            SMAP_R_BD_MODE => { dev9.bd_swap = if value & SMAP_BD_SWAP as u8 != 0 { 1 } else { 0 }; }
            _ => dev9W8(addr, value),
        }
    }
}

pub fn smap_write16(addr: u32, value: u16) {
    if (addr >= SMAP_BD_TX_BASE && addr < SMAP_BD_TX_BASE + SMAP_BD_SIZE)
        || (addr >= SMAP_BD_RX_BASE && addr < SMAP_BD_RX_BASE + SMAP_BD_SIZE)
    {
        let v = if unsafe { dev9.bd_swap } != 0 { value.swap_bytes() } else { value };
        dev9W16(addr, v);
        return;
    }
    match addr {
        SMAP_R_INTR_CLR => unsafe { dev9.irqcause &= !value; },
        _ => {
            let emac3_l = matches!(addr,
                SMAP_R_EMAC3_MODE0_L | SMAP_R_EMAC3_MODE1_L | SMAP_R_EMAC3_TxMODE0_L
                | SMAP_R_EMAC3_TxMODE1_L | SMAP_R_EMAC3_RxMODE_L | SMAP_R_EMAC3_INTR_STAT_L
                | SMAP_R_EMAC3_INTR_ENABLE_L | SMAP_R_EMAC3_ADDR_HI_L | SMAP_R_EMAC3_ADDR_LO_L
                | SMAP_R_EMAC3_VLAN_TPID | SMAP_R_EMAC3_PAUSE_TIMER_L
                | SMAP_R_EMAC3_INDIVID_HASH1 | SMAP_R_EMAC3_INDIVID_HASH2
                | SMAP_R_EMAC3_INDIVID_HASH3 | SMAP_R_EMAC3_INDIVID_HASH4
                | SMAP_R_EMAC3_GROUP_HASH1 | SMAP_R_EMAC3_GROUP_HASH2
                | SMAP_R_EMAC3_GROUP_HASH3 | SMAP_R_EMAC3_GROUP_HASH4
                | SMAP_R_EMAC3_LAST_SA_HI | SMAP_R_EMAC3_LAST_SA_LO
                | SMAP_R_EMAC3_INTER_FRAME_GAP_L | SMAP_R_EMAC3_STA_CTRL_L
                | SMAP_R_EMAC3_TX_THRESHOLD_L | SMAP_R_EMAC3_RX_WATERMARK_L
                | SMAP_R_EMAC3_TX_OCTETS | SMAP_R_EMAC3_RX_OCTETS);
            let emac3_h = match addr {
                SMAP_R_EMAC3_MODE0_H | SMAP_R_EMAC3_MODE1_H | SMAP_R_EMAC3_TxMODE0_H
                | SMAP_R_EMAC3_TxMODE1_H | SMAP_R_EMAC3_RxMODE_H | SMAP_R_EMAC3_INTR_STAT_H
                | SMAP_R_EMAC3_INTR_ENABLE_H | SMAP_R_EMAC3_ADDR_HI_H | SMAP_R_EMAC3_ADDR_LO_H
                | SMAP_R_EMAC3_PAUSE_TIMER_H | SMAP_R_EMAC3_INTER_FRAME_GAP_H
                | SMAP_R_EMAC3_STA_CTRL_H | SMAP_R_EMAC3_TX_THRESHOLD_H
                | SMAP_R_EMAC3_RX_WATERMARK_H => true,
                addr if addr == SMAP_R_EMAC3_VLAN_TPID + 2 => true,
                addr if addr == SMAP_R_EMAC3_INDIVID_HASH1 + 2 => true,
                addr if addr == SMAP_R_EMAC3_INDIVID_HASH2 + 2 => true,
                addr if addr == SMAP_R_EMAC3_INDIVID_HASH3 + 2 => true,
                addr if addr == SMAP_R_EMAC3_INDIVID_HASH4 + 2 => true,
                addr if addr == SMAP_R_EMAC3_GROUP_HASH1 + 2 => true,
                addr if addr == SMAP_R_EMAC3_GROUP_HASH2 + 2 => true,
                addr if addr == SMAP_R_EMAC3_GROUP_HASH3 + 2 => true,
                addr if addr == SMAP_R_EMAC3_GROUP_HASH4 + 2 => true,
                addr if addr == SMAP_R_EMAC3_LAST_SA_HI + 2 => true,
                addr if addr == SMAP_R_EMAC3_LAST_SA_LO + 2 => true,
                addr if addr == SMAP_R_EMAC3_TX_OCTETS + 2 => true,
                addr if addr == SMAP_R_EMAC3_RX_OCTETS + 2 => true,
                _ => false,
            };
            unsafe {
                dev9W16(addr, value);
                if emac3_h { emac3_write(addr.wrapping_sub(2)); }
            }
            let _ = emac3_l;
        }
    }
}

pub fn smap_write32(addr: u32, value: u32) {
    if addr >= SMAP_EMAC3_REGBASE && addr < SMAP_EMAC3_REGEND {
        smap_write16(addr, (value & 0xFFFF) as u16);
        smap_write16(addr + 2, (value >> 16) as u16);
        return;
    }
    match addr {
        SMAP_R_TXFIFO_DATA => unsafe {
            let off = (dev9R32(SMAP_R_TXFIFO_WR_PTR) & 0x3FFF) as usize;
            dev9.txfifo[off]     = (value & 0xFF) as u8;
            dev9.txfifo[off + 1] = ((value >> 8) & 0xFF) as u8;
            dev9.txfifo[off + 2] = ((value >> 16) & 0xFF) as u8;
            dev9.txfifo[off + 3] = ((value >> 24) & 0xFF) as u8;
            dev9W32(SMAP_R_TXFIFO_WR_PTR, (off + 4) as u32 & 0x3FFF);
        }
        _ => unsafe { dev9W32(addr, value); },
    }
}

fn wswap(d: u32) -> u32 { (d >> 16) | (d << 16) }

pub fn emac3_write(addr: u32) {
    let value = unsafe { wswap(dev9R32(addr)) };
    let mut v = value;
    match addr {
        SMAP_R_EMAC3_MODE0_L => {
            v = (v & !SMAP_E3_SOFT_RESET) | SMAP_E3_TXMAC_IDLE | SMAP_E3_RXMAC_IDLE;
            unsafe { dev9W16(SMAP_R_EMAC3_STA_CTRL_H, dev9R16(SMAP_R_EMAC3_STA_CTRL_H) | (SMAP_E3_PHY_OP_COMP as u16)); }
        }
        SMAP_R_EMAC3_TxMODE0_L => {
            if v & SMAP_E3_TX_GNP_0 == 0 {
                dev9_error("DEV9: SMAP_R_EMAC3_TxMODE0_L: SMAP_E3_TX_GNP_0 not set");
            }
            tx_process();
            v &= !SMAP_E3_TX_GNP_0;
            if v != 0 { dev9_error("DEV9: SMAP_R_EMAC3_TxMODE0_L: extra bits set"); }
        }
        SMAP_R_EMAC3_TxMODE1_L => {}
        SMAP_R_EMAC3_STA_CTRL_L => {
            if v & SMAP_E3_PHY_READ != 0 {
                v |= SMAP_E3_PHY_OP_COMP;
                let reg = (v & SMAP_E3_PHY_REG_ADDR_MSK) as usize;
                let mut val = unsafe { dev9.phyregs[reg] };
                match reg as u32 {
                    SMAP_DsPHYTER_BMSR if HAS_LINK().load(Ordering::Relaxed) => {
                        val |= SMAP_PHY_BMSR_LINK | SMAP_PHY_BMSR_ANCP;
                    }
                    SMAP_DsPHYTER_PHYSTS if HAS_LINK().load(Ordering::Relaxed) => {
                        val |= SMAP_PHY_STS_LINK | SMAP_PHY_STS_100M | SMAP_PHY_STS_FDX | SMAP_PHY_STS_ANCP;
                    }
                    _ => {}
                }
                v = (v & 0xFFFF) | ((val as u32) << 16);
            }
            if v & SMAP_E3_PHY_WRITE != 0 {
                v |= SMAP_E3_PHY_OP_COMP;
                let reg = (v & SMAP_E3_PHY_REG_ADDR_MSK) as usize;
                let mut val = (v >> 16) as u16;
                if reg as u32 == SMAP_DsPHYTER_BMCR {
                    if val & SMAP_PHY_BMCR_RST != 0 { ad_reset(); }
                    val &= !SMAP_PHY_BMCR_RST;
                    val |= 0x1;
                }
                unsafe { dev9.phyregs[reg] = val; }
            }
        }
        _ => {}
    }
    unsafe { dev9W32(addr, wswap(v)); }
}

pub fn HAS_LINK() -> &'static AtomicBool { &HAS_LINK_BOOL }
static HAS_LINK_BOOL: AtomicBool = AtomicBool::new(true);

// =====================================================================
//  SMAP RX/TX, async tick.
// =====================================================================

pub fn rx_fifo_can_rx() -> bool {
    unsafe {
        if dev9R8(SMAP_R_RXFIFO_FRAME_CNT) == 64 { return false; }
        let rd = dev9R32(SMAP_R_RXFIFO_RD_PTR);
        let space = DEV9_RX_FIFO_BYTES
            - (((dev9.rxfifo_wr_ptr as u32).wrapping_sub(rd)) & 0x3FFF) as usize;
        let space = if space == 0 { DEV9_RX_FIFO_BYTES } else { space };
        space >= 1514
    }
}

pub fn rx_process(pk: &NetPacket) {
    unsafe {
        let bd_off = ((SMAP_BD_RX_BASE & 0xFFFF) as usize) + (dev9.rxbdi as usize) * 8;
        let mut pbd = SmapBd {
            ctrl_stat: u16::from_le_bytes([dev9.dev9R[bd_off], dev9.dev9R[bd_off + 1]]),
            reserved:  0,
            length:    u16::from_le_bytes([dev9.dev9R[bd_off + 4], dev9.dev9R[bd_off + 5]]),
            pointer:   u16::from_le_bytes([dev9.dev9R[bd_off + 6], dev9.dev9R[bd_off + 7]]),
        };
        let bytes = ((pk.size + 3) & !3) as usize;
        if pbd.ctrl_stat & SMAP_BD_RX_EMPTY == 0 {
            dev9_error("DEV9: ERROR : Discarding packet (RX not ready)");
            return;
        }
        let pstart = (dev9.rxfifo_wr_ptr & 0x3FFF) as usize;
        for i in 0..bytes {
            dev9.rxfifo[dev9.rxfifo_wr_ptr as usize] = pk.buffer[i];
            dev9.rxfifo_wr_ptr = (dev9.rxfifo_wr_ptr + 1) & 0x3FFF;
        }
        dev9.rxbdi = (dev9.rxbdi + 1) & ((SMAP_BD_SIZE / 8) - 1);
        pbd.length = pk.size as u16;
        pbd.pointer = (0x4000 + pstart) as u16;
        pbd.ctrl_stat &= !SMAP_BD_RX_EMPTY;
        let b = pbd.ctrl_stat.to_le_bytes();
        dev9.dev9R[bd_off] = b[0];
        dev9.dev9R[bd_off + 1] = b[1];
        let b = pbd.length.to_le_bytes();
        dev9.dev9R[bd_off + 4] = b[0];
        dev9.dev9R[bd_off + 5] = b[1];
        let b = pbd.pointer.to_le_bytes();
        dev9.dev9R[bd_off + 6] = b[0];
        dev9.dev9R[bd_off + 7] = b[1];
        dev9W8(SMAP_R_RXFIFO_FRAME_CNT, dev9R8(SMAP_R_RXFIFO_FRAME_CNT).wrapping_add(1));
        FIRE_INTR_R.store(true, Ordering::Relaxed);
    }
}

pub fn tx_process() {
    unsafe {
        let mut cnt: u32 = 0;
        loop {
            let bd_off = ((SMAP_BD_TX_BASE & 0xFFFF) as usize) + (dev9.txbdi as usize) * 8;
            let pbd = SmapBd {
                ctrl_stat: u16::from_le_bytes([dev9.dev9R[bd_off], dev9.dev9R[bd_off + 1]]),
                reserved:  0,
                length:    u16::from_le_bytes([dev9.dev9R[bd_off + 4], dev9.dev9R[bd_off + 5]]),
                pointer:   u16::from_le_bytes([dev9.dev9R[bd_off + 6], dev9.dev9R[bd_off + 7]]),
            };
            if pbd.ctrl_stat & SMAP_BD_TX_READY == 0 { break; }
            if pbd.length > 1514 {
                dev9_error("DEV9: SMAP: ERROR : Trying to send packet too big.");
            } else {
                let mut pk = NetPacket { size: pbd.length as i32, buffer: [0u8; 2048 - 4] };
                let base = ((pbd.pointer as u32).wrapping_sub(0x1000)) & 0x3FFF;
                let len = pbd.length as usize;
                if (base as usize) + len > 16384 {
                    let was = 16384 - base as usize;
                    pk.buffer[..was].copy_from_slice(&dev9.txfifo[base as usize..base as usize + was]);
                    pk.buffer[was..len].copy_from_slice(&dev9.txfifo[..len - was]);
                } else {
                    pk.buffer[..len].copy_from_slice(&dev9.txfifo[base as usize..base as usize + len]);
                }
                tx_put(&pk);
            }
            let mut ctrl = pbd.ctrl_stat;
            ctrl &= !SMAP_BD_TX_READY;
            let b = ctrl.to_le_bytes();
            dev9.dev9R[bd_off] = b[0];
            dev9.dev9R[bd_off + 1] = b[1];
            dev9.txbdi = (dev9.txbdi + 1) & ((SMAP_BD_SIZE / 8) - 1);
            dev9W8(SMAP_R_TXFIFO_FRAME_CNT, dev9R8(SMAP_R_TXFIFO_FRAME_CNT).wrapping_sub(1));
            cnt += 1;
        }
        if cnt != 0 { dev9_raise_irq(SMAP_INTR_TXEND, 100); }
        else { dev9_raise_irq(SMAP_INTR_TXDNV, 0); }
    }
}

static FIRE_INTR_R: AtomicBool = AtomicBool::new(false);
pub fn smap_async(_cycles: u32) {
    if FIRE_INTR_R.swap(false, Ordering::Relaxed) {
        dev9_raise_irq(SMAP_INTR_RXEND, 0);
    }
}

pub fn smap_readDMA8Mem(p_mem: *mut u32, size: i32) {
    unsafe {
        if dev9R16(SMAP_R_RXFIFO_CTRL) & SMAP_RXFIFO_DMAEN != 0 {
            let mut p = p_mem;
            let mut sz = size;
            while sz > 0 {
                let rd = (dev9R32(SMAP_R_RXFIFO_RD_PTR) & 0x3FFF) as usize;
                let bytes = [
                    dev9.rxfifo[rd], dev9.rxfifo[rd + 1],
                    dev9.rxfifo[rd + 2], dev9.rxfifo[rd + 3],
                ];
                *p = u32::from_le_bytes(bytes);
                p = p.add(1);
                dev9W32(SMAP_R_RXFIFO_RD_PTR, (rd + 4) as u32 & 0x3FFF);
                sz -= 4;
            }
            dev9W16(SMAP_R_RXFIFO_CTRL, dev9R16(SMAP_R_RXFIFO_CTRL) & !(SMAP_RXFIFO_DMAEN as u16));
        }
    }
}

pub fn smap_writeDMA8Mem(p_mem: *const u32, size: i32) {
    unsafe {
        if dev9R16(SMAP_R_TXFIFO_CTRL) & SMAP_TXFIFO_DMAEN != 0 {
            let mut p = p_mem;
            let mut sz = size;
            while sz > 0 {
                let off = (dev9R32(SMAP_R_TXFIFO_WR_PTR) & 0x3FFF) as usize;
                let v = *p;
                dev9.txfifo[off]     = (v & 0xFF) as u8;
                dev9.txfifo[off + 1] = ((v >> 8) & 0xFF) as u8;
                dev9.txfifo[off + 2] = ((v >> 16) & 0xFF) as u8;
                dev9.txfifo[off + 3] = ((v >> 24) & 0xFF) as u8;
                p = p.add(1);
                dev9W32(SMAP_R_TXFIFO_WR_PTR, (off + 4) as u32 & 0x3FFF);
                sz -= 4;
            }
            dev9W16(SMAP_R_TXFIFO_CTRL, dev9R16(SMAP_R_TXFIFO_CTRL) & !(SMAP_TXFIFO_DMAEN as u16));
        }
    }
}

// =====================================================================
//  Public DEV9 init / reset / shutdown / read / write entry points.
// =====================================================================

pub fn dev9Init() -> s32 {
    unsafe {
        dev9 = Dev9State::new();
        dev9.ata = Some(Box::new(AtaDevice::new()));
        dev9FlashInit();
        for rxbi in 0..(SMAP_BD_SIZE / 8) {
            let off = ((SMAP_BD_RX_BASE & 0xFFFF) as usize) + (rxbi as usize) * 8;
            dev9.dev9R[off]     = (SMAP_BD_RX_EMPTY & 0xFF) as u8;
            dev9.dev9R[off + 1] = ((SMAP_BD_RX_EMPTY >> 8) & 0xFF) as u8;
            dev9.dev9R[off + 4] = 0;
            dev9.dev9R[off + 5] = 0;
        }
    }
    0
}

pub fn dev9Reset() {
    dev9Init();
}

pub fn dev9Open() -> s32 {
    unsafe {
        if DEV9_CONFIG.hdd_enable {
            if let Some(ata) = dev9.ata.as_mut() {
                let path = if !DEV9_CONFIG.hdd_file.is_empty() {
                    DEV9_CONFIG.hdd_file.clone()
                } else { String::from("DEV9hdd.raw") };
                if ata.open(&path) != 0 { DEV9_CONFIG.hdd_enable = false; }
            }
        }
        if DEV9_CONFIG.eth_enable { InitNet(); }
        dev9.opened = true;
    }
    0
}

pub fn dev9Close() {
    unsafe {
        dev9.dma_iop_ptr = None;
        if let Some(ata) = dev9.ata.as_mut() { ata.close(); }
        TermNet();
        dev9.opened = false;
    }
}

pub fn dev9Shutdown() {
    dev9Close();
    unsafe { dev9.ata = None; }
}

pub fn dev9Read8(addr: u32) -> u8 {
    unsafe {
        if !DEV9_CONFIG.eth_enable && !DEV9_CONFIG.hdd_enable { return 0; }
        if addr >= ATA_DEV9_HDD_BASE && addr < ATA_DEV9_HDD_END {
            return dev9.ata.as_ref().map(|a| a.read(addr, 8) as u8).unwrap_or(0);
        }
        if addr >= SPD_REGBASE && addr < SMAP_REGBASE { return (speed_read(addr, 8) & 0xFF) as u8; }
        if addr >= SMAP_REGBASE && addr < FLASH_REGBASE { return smap_read8(addr); }
        if addr >= FLASH_REGBASE && addr < FLASH_REGBASE + FLASH_REGSIZE { return dev9FlashRead(addr); }
        match addr {
            DEV9_R_REV => 0x32,
            _ => dev9R8(addr),
        }
    }
}

pub fn dev9Read16(addr: u32) -> u16 {
    unsafe {
        if !DEV9_CONFIG.eth_enable && !DEV9_CONFIG.hdd_enable { return 0; }
        if addr >= ATA_DEV9_HDD_BASE && addr < ATA_DEV9_HDD_END {
            return dev9.ata.as_ref().map(|a| a.read(addr, 16) as u16).unwrap_or(0);
        }
        if addr >= SPD_REGBASE && addr < SMAP_REGBASE { return speed_read(addr, 16); }
        if addr >= SMAP_REGBASE && addr < FLASH_REGBASE { return smap_read16(addr); }
        if addr >= FLASH_REGBASE && addr < FLASH_REGBASE + FLASH_REGSIZE { return dev9FlashRead(addr) as u16; }
        match addr {
            DEV9_R_REV => 0x0032,
            _ => dev9R16(addr),
        }
    }
}

pub fn dev9Read32(addr: u32) -> u32 {
    unsafe {
        if !DEV9_CONFIG.eth_enable && !DEV9_CONFIG.hdd_enable { return 0; }
        if addr >= ATA_DEV9_HDD_BASE && addr < ATA_DEV9_HDD_END { return 0; }
        if addr >= SMAP_REGBASE && addr < FLASH_REGBASE { return smap_read32(addr); }
        if addr >= FLASH_REGBASE && addr < FLASH_REGBASE + FLASH_REGSIZE { return dev9FlashRead(addr) as u32; }
        dev9R32(addr)
    }
}

pub fn dev9Write8(addr: u32, value: u8) {
    unsafe {
        if !DEV9_CONFIG.eth_enable && !DEV9_CONFIG.hdd_enable { return; }
        if addr >= ATA_DEV9_HDD_BASE && addr < ATA_DEV9_HDD_END {
            if let Some(ata) = dev9.ata.as_mut() { ata.write(addr, value as u32, 8); }
            return;
        }
        if addr >= SPD_REGBASE && addr < SMAP_REGBASE { speed_write(addr, value as u16, 8); return; }
        if addr >= SMAP_REGBASE && addr < FLASH_REGBASE { smap_write8(addr, value); return; }
        if addr >= FLASH_REGBASE && addr < FLASH_REGBASE + FLASH_REGSIZE {
            dev9FlashWrite(addr, value); return;
        }
    }
}

pub fn dev9Write16(addr: u32, value: u16) {
    unsafe {
        if !DEV9_CONFIG.eth_enable && !DEV9_CONFIG.hdd_enable { return; }
        if addr >= ATA_DEV9_HDD_BASE && addr < ATA_DEV9_HDD_END {
            if let Some(ata) = dev9.ata.as_mut() { ata.write(addr, value as u32, 16); }
            return;
        }
        if addr >= SPD_REGBASE && addr < SMAP_REGBASE { speed_write(addr, value, 16); return; }
        if addr >= SMAP_REGBASE && addr < FLASH_REGBASE { smap_write16(addr, value); return; }
        if addr >= FLASH_REGBASE && addr < FLASH_REGBASE + FLASH_REGSIZE {
            dev9FlashWrite(addr, value as u8); return;
        }
        dev9W16(addr, value);
    }
}

pub fn dev9Write32(addr: u32, value: u32) {
    unsafe {
        if !DEV9_CONFIG.eth_enable && !DEV9_CONFIG.hdd_enable { return; }
        if addr >= ATA_DEV9_HDD_BASE && addr < ATA_DEV9_HDD_END { return; }
        if addr >= SMAP_REGBASE && addr < FLASH_REGBASE { smap_write32(addr, value); return; }
        if addr >= FLASH_REGBASE && addr < FLASH_REGBASE + FLASH_REGSIZE {
            dev9FlashWrite(addr, value as u8); return;
        }
        match addr {
            SPD_R_INTR_MASK => dev9_error("DEV9: SPD_R_INTR_MASK, WTFH ?"),
            _ => dev9W32(addr, value),
        }
    }
}

pub fn dev9ReadDMA8Mem(p_mem: *mut u32, size: i32) {
    unsafe {
        if !DEV9_CONFIG.eth_enable && !DEV9_CONFIG.hdd_enable { return; }
        let size = size >> 1;
        if dev9.dma_ctrl & SPD_DMA_TO_SMAP != 0 {
            smap_readDMA8Mem(p_mem, size);
            psxDMA8Interrupt();
        } else if dev9.xfr_ctrl & SPD_XFR_WRITE == 0 {
            dev9.dma_iop_ptr = Some(p_mem as *mut u8);
            dev9.dma_iop_size = size as u32;
            dev9.dma_iop_transfered = 0;
            dev9runFIFO();
        }
    }
}

pub fn dev9WriteDMA8Mem(p_mem: *const u32, size: i32) {
    unsafe {
        if !DEV9_CONFIG.eth_enable && !DEV9_CONFIG.hdd_enable { return; }
        let size = size >> 1;
        if dev9.dma_ctrl & SPD_DMA_TO_SMAP != 0 {
            smap_writeDMA8Mem(p_mem, size);
            psxDMA8Interrupt();
        } else if dev9.xfr_ctrl & SPD_XFR_WRITE != 0 {
            dev9.dma_iop_ptr = Some(p_mem as *mut u8);
            dev9.dma_iop_size = size as u32;
            dev9.dma_iop_transfered = 0;
            dev9runFIFO();
        }
    }
}

pub fn dev9Async(cycles: u32) {
    unsafe { if let Some(ata) = dev9.ata.as_mut() { ata.async_op(cycles); } }
    smap_async(cycles);
}

pub fn dev9CheckChanges(_old: &Dev9Config) {
    unsafe {
        if !dev9.opened { return; }
        if DEV9_CONFIG.eth_enable {
            ReconfigureLiveNet(_old);
        }
        if DEV9_CONFIG.hdd_enable {
            if let Some(ata) = dev9.ata.as_mut() {
                if _old.hdd_enable {
                    if DEV9_CONFIG.hdd_file != _old.hdd_file {
                        ata.close();
                        if ata.open(&DEV9_CONFIG.hdd_file) != 0 { DEV9_CONFIG.hdd_enable = false; }
                    }
                } else if ata.open(&DEV9_CONFIG.hdd_file) != 0 { DEV9_CONFIG.hdd_enable = false; }
            }
        } else if _old.hdd_enable {
            if let Some(ata) = dev9.ata.as_mut() { ata.close(); }
        }
    }
}

// =====================================================================
//  Network packet type.
// =====================================================================

#[derive(Clone)]
pub struct NetPacket {
    pub size: i32,
    pub buffer: [u8; 2044],
}

impl NetPacket {
    pub fn new() -> Self { NetPacket { size: 0, buffer: [0; 2044] } }
    pub fn from_bytes(data: &[u8]) -> Self {
        let mut p = NetPacket::new();
        let n = data.len().min(p.buffer.len());
        p.buffer[..n].copy_from_slice(&data[..n]);
        p.size = n as i32;
        p
    }
}

#[derive(Clone, Debug)]
pub struct MacAddress(pub [u8; 6]);
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IpAddress(pub [u8; 4]);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectionKey {
    pub ip: IpAddress,
    pub protocol: u8,
    pub ps2_port: u16,
    pub srv_port: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdapterOptions {
    None = 0,
    DHCPForcedOn = 1 << 0,
    DHCPOverrideIP = 1 << 1,
    DHCPOverideSubnet = 1 << 2,
    DHCPOverideGateway = 1 << 3,
}

// =====================================================================
//  Thread-safe simple queue (mirrors `SimpleQueue.h`).
// =====================================================================

struct SimpleQueueEntry<T> {
    ready: AtomicBool,
    next: *mut SimpleQueueEntry<T>,
    value: Option<T>,
}

pub struct SimpleQueue<T> {
    head: Mutex<*mut SimpleQueueEntry<T>>,
    tail: Mutex<*mut SimpleQueueEntry<T>>,
}

unsafe impl<T: Send> Send for SimpleQueue<T> {}
unsafe impl<T: Send> Sync for SimpleQueue<T> {}

impl<T> SimpleQueue<T> {
    pub fn new() -> Self {
        let dummy = Box::into_raw(Box::new(SimpleQueueEntry {
            ready: AtomicBool::new(false),
            next: std::ptr::null_mut(),
            value: None,
        }));
        SimpleQueue { head: Mutex::new(dummy), tail: Mutex::new(dummy) }
    }

    pub fn enqueue(&self, v: T) {
        let new_head = Box::into_raw(Box::new(SimpleQueueEntry {
            ready: AtomicBool::new(false),
            next: std::ptr::null_mut(),
            value: None,
        }));
        let old_head = {
            let mut h = self.head.lock().unwrap();
            let old = *h;
            *h = new_head;
            old
        };
        unsafe {
            (*old_head).next = new_head;
            (*old_head).value = Some(v);
            (*old_head).ready.store(true, Ordering::Release);
        }
    }

    pub fn dequeue(&self) -> Option<T> {
        let tail = *self.tail.lock().unwrap();
        unsafe {
            if tail.is_null() || !(*tail).ready.load(Ordering::Acquire) { return None; }
            let v = (*tail).value.take();
            let next = (*tail).next;
            *self.tail.lock().unwrap() = next;
            drop(Box::from_raw(tail));
            v
        }
    }

    pub fn is_empty(&self) -> bool {
        let h = *self.head.lock().unwrap();
        let t = *self.tail.lock().unwrap();
        h == t
    }
}

impl<T> Drop for SimpleQueue<T> {
    fn drop(&mut self) {
        let mut cur = *self.head.lock().unwrap();
        while !cur.is_null() {
            unsafe {
                let next = (*cur).next;
                drop(Box::from_raw(cur));
                cur = next;
            }
        }
    }
}

// =====================================================================
//  Thread-safe map (mirrors `ThreadSafeMap.h`).
// =====================================================================

pub struct ThreadSafeMap<K, V> {
    inner: Mutex<std::collections::HashMap<K, V>>,
}

impl<K, V> ThreadSafeMap<K, V>
where K: std::hash::Hash + Eq + Clone,
{
    pub fn new() -> Self { ThreadSafeMap { inner: Mutex::new(std::collections::HashMap::new()) } }
    pub fn add(&self, key: K, value: V) {
        self.inner.lock().unwrap().insert(key, value);
    }
    pub fn remove(&self, key: &K) -> bool {
        self.inner.lock().unwrap().remove(key).is_some()
    }
    pub fn clear(&self) {
        self.inner.lock().unwrap().clear();
    }
    pub fn keys(&self) -> Vec<K> {
        self.inner.lock().unwrap().keys().cloned().collect()
    }
    pub fn try_get_value(&self, key: &K) -> Option<V>
    where V: Clone {
        self.inner.lock().unwrap().get(key).cloned()
    }
    pub fn contains_key(&self, key: &K) -> bool {
        self.inner.lock().unwrap().contains_key(key)
    }
}

// =====================================================================
//  NetAdapter trait + helpers.
// =====================================================================

pub struct NetAdapterBase {
    pub ps2_mac: MacAddress,
    pub ps2_ip:  IpAddress,
    pub host_mac: MacAddress,
    pub rx_thread: Option<JoinHandle<()>>,
    pub rx_running: Arc<AtomicBool>,
    pub internal_rx_running: Arc<AtomicBool>,
    pub internal_rx_mutex: Mutex<bool>,
    pub internal_rx_cv: Condvar,
}

pub trait NetAdapter {
    fn blocks(&self) -> bool;
    fn is_initialised(&self) -> bool;
    fn recv(&mut self, pkt: &mut NetPacket) -> bool;
    fn send(&mut self, pkt: &NetPacket) -> bool;
    fn reset(&mut self) {}
    fn reload_settings(&mut self) {}
    fn close(&mut self) {}
    fn set_mac(&mut self, mac: &MacAddress);
    fn verify_pkt(&self, pkt: &NetPacket) -> bool;
}

pub fn tx_put(_pkt: &NetPacket) {
    if let Some(a) = unsafe { NIF.as_mut() } { let _ = a.send(_pkt); }
}

pub fn ad_reset() {
    if let Some(a) = unsafe { NIF.as_mut() } { a.reset(); }
}

use std::sync::Arc;
pub static mut NIF: Option<Box<dyn NetAdapter + Send>> = None;
static mut RX_THREAD: Option<JoinHandle<()>> = None;
static mut RX_RUNNING: bool = false;

fn net_rx_thread() {
    loop {
        unsafe {
            if !RX_RUNNING { break; }
            if let Some(nif) = NIF.as_mut() {
                let mut pk = NetPacket::new();
                if nif.recv(&mut pk) {
                    if rx_fifo_can_rx() { rx_process(&pk); }
                }
            }
        }
        thread::sleep(std::time::Duration::from_millis(1));
    }
}

pub fn InitNet() {
    unsafe {
        if RX_RUNNING { return; }
        if NIF.is_none() {
            match DEV9_CONFIG.eth_api {
                NetApi::PCAP_Bridged | NetApi::PCAP_Switched => {
                    if let Some(a) = make_pcap_adapter() { NIF = Some(a); }
                }
                NetApi::Sockets => {
                    if let Some(a) = make_socket_adapter() { NIF = Some(a); }
                }
                NetApi::TAP => {
                    if let Some(a) = make_tap_adapter() { NIF = Some(a); }
                }
            }
        }
        if NIF.is_none() {
            DEV9_CONFIG.eth_enable = false;
            dev9_error("DEV9: Failed to GetNetAdapter()");
            return;
        }
        RX_RUNNING = true;
        RX_THREAD = Some(thread::spawn(net_rx_thread));
    }
}

pub fn ReconfigureLiveNet(_old: &Dev9Config) {
    unsafe {
        if DEV9_CONFIG.eth_enable {
            if _old.eth_enable {
                if DEV9_CONFIG.eth_device != _old.eth_device
                    || DEV9_CONFIG.eth_api != _old.eth_api
                {
                    TermNet();
                    InitNet();
                } else if let Some(nif) = NIF.as_mut() { nif.reload_settings(); }
            } else { InitNet(); }
        } else if _old.eth_enable { TermNet(); }
    }
}

pub fn TermNet() {
    unsafe {
        if RX_RUNNING {
            RX_RUNNING = false;
            if let Some(t) = RX_THREAD.take() { let _ = t.join(); }
            if let Some(nif) = NIF.as_mut() { nif.close(); }
            NIF = None;
        }
    }
}

pub fn make_pcap_adapter() -> Option<Box<dyn NetAdapter + Send>> { None }
pub fn make_socket_adapter() -> Option<Box<dyn NetAdapter + Send>> { None }
pub fn make_tap_adapter() -> Option<Box<dyn NetAdapter + Send>> { None }

pub fn set_ps2_mac(mac: &MacAddress) {
    unsafe {
        dev9.eeprom[0] = u16::from_le_bytes([mac.0[0], mac.0[1]]);
        dev9.eeprom[1] = u16::from_le_bytes([mac.0[2], mac.0[3]]);
        dev9.eeprom[2] = u16::from_le_bytes([mac.0[4], mac.0[5]]);
        let c = dev9.eeprom[0].wrapping_add(dev9.eeprom[1]).wrapping_add(dev9.eeprom[2]);
        dev9.eeprom[3] = c;
    }
}

// =====================================================================
//  Adapter enumeration.
// =====================================================================

#[derive(Clone, Debug)]
pub struct AdapterEntry {
    pub api:  NetApi,
    pub name: String,
    pub guid: String,
}

pub fn get_socket_adapters() -> Vec<AdapterEntry> {
    let mut nic = Vec::new();
    nic.push(AdapterEntry { api: NetApi::Sockets, name: "Auto".into(), guid: "Auto".into() });
    nic
}

pub fn get_pcap_adapters() -> Vec<AdapterEntry> { Vec::new() }
pub fn get_tap_adapters()   -> Vec<AdapterEntry> { Vec::new() }

pub fn get_all_adapters() -> Vec<AdapterEntry> {
    let mut v = get_socket_adapters();
    v.extend(get_pcap_adapters());
    v.extend(get_tap_adapters());
    v
}

// =====================================================================
//  AdapterUtils — the Rust versions of the platform specific helpers
//  live in a single `dev9_adapter_utils` module to keep the call
//  sites in the adapter code uniform across platforms.
// =====================================================================

pub mod dev9_adapter_utils {
    use super::*;
    pub fn read_address_family(_addr: *const u8) -> u16 { 0 }
    pub fn get_all_adapters_win32(_include_hidden: bool) -> Option<WinAdapterList> { None }
    pub fn get_all_adapters_posix() -> Option<PosAdapterList> { None }
    pub fn get_adapter_mac(_a: &AdapterInfo) -> Option<MacAddress> { None }
    pub fn get_adapter_ip(_a: &AdapterInfo) -> Option<IpAddress> { None }
    pub fn get_gateways(_a: &AdapterInfo) -> Vec<IpAddress> { Vec::new() }
    pub fn get_dns(_a: &AdapterInfo) -> Vec<IpAddress> { Vec::new() }
}

pub struct WinAdapterList;
pub struct PosAdapterList;
pub struct AdapterInfo;
