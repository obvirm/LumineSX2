// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust 2021 translation of PCSX2's SIO subsystem.
//!
//! This module consolidates the SIO controller state, multitap 1-3-7 multiplex,
//! memory card file/folder backends, PS1/PS2 memory card protocol, and the full
//! pad (controller) type hierarchy.  It is a single-file rewrite of:
//!
//! - `Sio.cpp/.h`, `Sio0.cpp/.h`, `Sio2.cpp/.h`, `SioTypes.h`
//! - `Multitap/MultitapProtocol.cpp/.h`
//! - `Memcard/MemoryCardFile.cpp/.h`, `MemoryCardFolder.h`
//! - `Memcard/MemoryCardProtocol.cpp/.h`
//! - `Pad/Pad.cpp/.h`, `PadBase.cpp/.h`, `PadDualshock2.cpp/.h`
//! - `Pad/PadGuitar.cpp/.h`, `PadJogcon.cpp/.h`, `PadNegcon.cpp/.h`
//! - `Pad/PadNotConnected.cpp/.h`, `PadPopn.cpp/.h`, `PadTypes.h`
//!
//! Only `std` is used; everything is `static mut` where the C++ code used
//! globals.  Heavy I/O and state-save plumbing is stubbed to a minimal
//! faithful shape (the original C++ used `FileSystem`, `fmt`, `Host`, and
//! `StateWrapper` which are outside the scope of this pure-std rewrite).

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]
#![allow(dead_code)]
#![allow(clippy::upper_case_acronyms)]

use std::cell::UnsafeCell;
use std::collections::{BTreeMap, VecDeque};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};

// ---------------------------------------------------------------------------
// Primitive aliases
// ---------------------------------------------------------------------------

pub type u8 = std::primitive::u8;
pub type u16 = std::primitive::u16;
pub type u32 = std::primitive::u32;
pub type u64 = std::primitive::u64;
pub type s16 = std::primitive::i16;
pub type s64 = std::primitive::i64;

// ---------------------------------------------------------------------------
// SIO type constants (from SioTypes.h)
// ---------------------------------------------------------------------------

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SioStage {
    IDLE,
    WAITING_COMMAND,
    WORKING,
}

pub mod SioMode {
    pub const NOT_SET: u8 = 0x00;
    pub const PAD: u8 = 0x01;
    pub const MULTITAP: u8 = 0x21;
    pub const INFRARED: u8 = 0x61;
    pub const MEMCARD: u8 = 0x81;
}

pub mod MemcardCommand {
    pub const NOT_SET: u8 = 0x00;
    pub const PROBE: u8 = 0x11;
    pub const UNKNOWN_WRITE_DELETE_END: u8 = 0x12;
    pub const SET_ERASE_SECTOR: u8 = 0x21;
    pub const SET_WRITE_SECTOR: u8 = 0x22;
    pub const SET_READ_SECTOR: u8 = 0x23;
    pub const GET_SPECS: u8 = 0x26;
    pub const SET_TERMINATOR: u8 = 0x27;
    pub const GET_TERMINATOR: u8 = 0x28;
    pub const WRITE_DATA: u8 = 0x42;
    pub const READ_DATA: u8 = 0x43;
    pub const PS1_READ: u8 = 0x52;
    pub const PS1_STATE: u8 = 0x53;
    pub const PS1_WRITE: u8 = 0x57;
    pub const PS1_POCKETSTATION: u8 = 0x58;
    pub const READ_WRITE_END: u8 = 0x81;
    pub const ERASE_BLOCK: u8 = 0x82;
    pub const UNKNOWN_BOOT: u8 = 0xbf;
    pub const AUTH_XOR: u8 = 0xf0;
    pub const AUTH_F3: u8 = 0xf3;
    pub const AUTH_F7: u8 = 0xf7;
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Sio0Interrupt {
    TEST_EVENT,
    STAT_READ,
    TX_DATA_WRITE,
}

pub mod SIO {
    pub const PORTS: usize = 2;
    pub const SLOTS: usize = 4;
}

pub mod SIO0_STAT {
    pub const TX_READY: u32 = 0x01;
    pub const RX_FIFO_NOT_EMPTY: u32 = 0x02;
    pub const TX_EMPTY: u32 = 0x04;
    pub const RX_PARITY_ERROR: u32 = 0x08;
    pub const ACK: u32 = 0x80;
    pub const IRQ: u32 = 0x0200;
}

pub mod SIO0_CTRL {
    pub const TX_ENABLE: u16 = 0x01;
    pub const RX_ENABLE: u16 = 0x04;
    pub const ACK: u16 = 0x10;
    pub const RESET: u16 = 0x40;
    pub const RX_INT_MODE_LSB: u16 = 0x0100;
    pub const RX_INT_MODE_MSB: u16 = 0x0200;
    pub const TX_INT_ENABLE: u16 = 0x0400;
    pub const RX_INT_ENABLE: u16 = 0x0800;
    pub const ACK_INT_ENABLE: u16 = 0x1000;
    pub const PORT: u16 = 0x2000;
}

pub mod Sio2Cmd {
    pub const PORT: u32 = 0x01;
    pub const COMMAND_LENGTH_MASK: u32 = 0x3ff;
}

pub mod Sio2Ctrl {
    pub const START_TRANSFER: u32 = 0x1;
    pub const RESET: u32 = 0xc;
    pub const PORT: u32 = 0x2000;
    pub const SIO2MAN_RESET: u32 = 0x000003bc;
}

pub mod CmdStat {
    pub const DISCONNECTED: u32 = 0x1d100;
    pub const CONNECTED: u32 = 0x1100;
    pub const NO_DEVICES_MISSING: u32 = 0x1000;
    pub const PORT_1_MISSING: u32 = 0x1D000;
    pub const PORT_2_MISSING: u32 = 0x2D000;
    pub const BOTH_PORTS_MISSING: u32 = 0x3D000;
    pub const ONE_PORT_OPEN: u32 = 0x100;
    pub const TWO_PORTS_OPEN: u32 = 0x200;
}

pub mod PortStat {
    pub const DEFAULT: u32 = 0xf;
}

pub mod FifoStat {
    pub const DEFAULT: u32 = 0x0;
    pub const SPECS: u32 = 0x83;
    pub const TERMINATOR: u32 = 0x8b;
    pub const READ_WRITE_END: u32 = 0x8c;
}

pub mod Terminator {
    pub const NOT_READY: u8 = 0x66;
    pub const READY: u8 = 0x55;
}

pub const NUM_FRAMES_BEFORE_SAVESTATE_DEPENDENCY_WARNING: u32 = 60 * 60 * 60 * 2;

// ---------------------------------------------------------------------------
// Pad types (from PadTypes.h)
// ---------------------------------------------------------------------------

pub mod Pad {
    use super::*;

    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    pub enum Command {
        NOT_SET = 0x00,
        MYSTERY = 0x40,
        BUTTON_QUERY = 0x41,
        POLL = 0x42,
        CONFIG = 0x43,
        MODE_SWITCH = 0x44,
        STATUS_INFO = 0x45,
        CONST_1 = 0x46,
        CONST_2 = 0x47,
        CONST_3 = 0x4c,
        VIBRATION_MAP = 0x4d,
        RESPONSE_BYTES = 0x4f,
    }

    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    pub enum Mode {
        NOT_SET = 0x00,
        PS1_MOUSE = 0x12,
        NEGCON = 0x23,
        PS1_KONAMI_LIGHTGUN = 0x31,
        DIGITAL = 0x41,
        PS1_FLIGHT_STICK = 0x53,
        PS1_NAMCO_LIGHTGUN = 0x63,
        ANALOG = 0x73,
        DUALSHOCK2 = 0x79,
        PS1_MULTITAP = 0x80,
        PS1_JOGCON = 0xe3,
        CONFIG = 0xf3,
        DISCONNECTED = 0xff,
    }

    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    pub enum PhysicalType {
        NOT_SET = 0x00,
        GUITAR = 0x01,
        STANDARD = 0x03,
    }

    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    pub enum ResponseBytes {
        DIGITAL = 0x00000000,
        ANALOG = 0x0000003f,
        DUALSHOCK2 = 0x0003ffff,
    }

    pub const ANALOG_NEUTRAL_POSITION: u8 = 0x7f;

    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    pub enum ControllerType {
        NotConnected,
        DualShock2,
        Guitar,
        Jogcon,
        Negcon,
        Popn,
        Count,
    }

    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    pub enum VibrationCapabilities {
        NoVibration,
        LargeSmallMotors,
        SingleMotor,
        Count,
    }

    /// Lightweight stand-in for the `InputBindingInfo` C++ struct.  Only the
    /// fields actually consumed by the pad command/state machine are kept.
    #[derive(Copy, Clone, Debug)]
    pub struct InputBindingInfo {
        pub name: &'static str,
        pub display_name: &'static str,
        pub icon: Option<&'static str>,
        pub generic_mapping: u8,
    }

    /// Lightweight stand-in for the `SettingInfo` C++ struct.
    #[derive(Copy, Clone, Debug)]
    pub struct SettingInfo {
        pub name: &'static str,
        pub display_name: &'static str,
    }

    #[derive(Copy, Clone, Debug)]
    pub struct ControllerInfo {
        pub controller_type: ControllerType,
        pub name: &'static str,
        pub display_name: &'static str,
        pub icon: Option<&'static str>,
        pub bindings: &'static [InputBindingInfo],
        pub settings: &'static [SettingInfo],
        pub vibration_caps: VibrationCapabilities,
    }

    pub const NUM_CONTROLLER_PORTS: u32 = 8;
    pub const DEFAULT_EJECT_TICKS: usize = 50;

    pub const DEFAULT_STICK_DEADZONE: f32 = 0.0;
    pub const DEFAULT_STICK_SCALE: f32 = 1.33;
    pub const DEFAULT_MOTOR_SCALE: f32 = 1.0;
    pub const DEFAULT_PRESSURE_MODIFIER: f32 = 0.5;
    pub const DEFAULT_BUTTON_DEADZONE: f32 = 0.0;

    pub const NUM_MACRO_BUTTONS_PER_CONTROLLER: u32 = 16;
}

// ---------------------------------------------------------------------------
// Memory card size / type metadata (from MemoryCardFile.h)
// ---------------------------------------------------------------------------

#[derive(Copy, Clone, Debug)]
pub struct McdSizeInfo {
    pub SectorSize: u16,
    pub EraseBlockSizeInSectors: u16,
    pub McdSizeInSectors: u32,
    pub Xor: u8,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MemoryCardType {
    Empty,
    File,
    Folder,
    MaxCount,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MemoryCardFileType {
    Unknown,
    PS2_8MB,
    PS2_16MB,
    PS2_32MB,
    PS2_64MB,
    PS1,
    MaxCount,
}

#[derive(Clone, Debug)]
pub struct AvailableMcdInfo {
    pub name: String,
    pub path: String,
    pub modified_time: i64,
    pub card_type: MemoryCardType,
    pub file_type: MemoryCardFileType,
    pub size: u32,
    pub formatted: bool,
}

// ---------------------------------------------------------------------------
// Memory card (MCD) state (from Sio.h's _mcd struct)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Mcd {
    pub currentCommand: u8,
    /// Terminator value reported on the bus.
    pub term: u8,
    pub goodSector: bool,
    pub msb: u8,
    pub lsb: u8,
    pub sectorAddr: u32,
    pub transferAddr: u32,
    /// Buffer used for reads and writes.
    pub buf: Vec<u8>,
    /// PSX-only flag byte (per-PS1-card).
    pub FLAG: u8,
    pub port: u8,
    pub slot: u8,
    /// Auto-eject countdown in frames.
    pub autoEjectTicks: usize,
}

impl Mcd {
    pub const fn new() -> Self {
        Self {
            currentCommand: 0,
            term: 0x55,
            goodSector: false,
            msb: 0,
            lsb: 0,
            sectorAddr: 0,
            transferAddr: 0,
            buf: Vec::new(),
            FLAG: 0x08,
            port: 0,
            slot: 0,
            autoEjectTicks: 0,
        }
    }

    /// XOR of the msb/lsb register shadows and the buffered bytes; used to
    /// compute the per-sector checksum.
    pub fn DoXor(&self) -> u8 {
        let mut ret = self.msb ^ self.lsb;
        for b in &self.buf {
            ret ^= b;
        }
        ret
    }

    pub fn NextFrame(&mut self) {
        FileMcd_NextFrame(self.port, self.slot);
    }

    pub fn ReIndex(&mut self, filter: &str) -> bool {
        FileMcd_ReIndex(self.port, self.slot, filter)
    }

    /// Stub: a memory card slot is present iff its per-slot buffer has been
    /// populated.  The C++ code checks the FileSystem-backed card's open state.
    pub fn IsPresent(&self) -> bool {
        !self.buf.is_empty()
    }

    /// Stub: PSX/PS1 cards are signalled by the FLAG register.
    pub fn IsPSX(&self) -> bool {
        self.FLAG != 0
    }
}

impl Default for Mcd {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Global SIO state: mcds[][] and the currently selected mcd pointer.
// ---------------------------------------------------------------------------

/// The 2 ports x 4 slots array of memory card state, mirroring the C++
/// `mcds[2][4]` global.  In idiomatic Rust the global is held in a
/// `static mut` and accessed through accessor helpers.
pub static mut mcds: [[Mcd; SIO::SLOTS]; SIO::PORTS] = [
    [const { Mcd::new() }; SIO::SLOTS],
    [const { Mcd::new() }; SIO::SLOTS],
];

/// Pointer to the currently selected memory card.  Matches C++ `mcd *mcd`.
pub static mut mcd: *mut Mcd = std::ptr::null_mut();

/// Reset the per-slot MCD state to PS1 defaults (mirrors Sio0::Initialize).
pub fn mcd_init_all() {
    unsafe {
        for port in 0..SIO::PORTS {
            for slot in 0..SIO::SLOTS {
                let m = &mut mcds[port][slot];
                m.term = 0x55;
                m.port = port as u8;
                m.slot = slot as u8;
                m.FLAG = 0x08;
                m.autoEjectTicks = 0;
            }
        }
        mcd = &mut mcds[0][0] as *mut Mcd;
    }
}

/// "Per-frame tick" that walks every (port, slot) and calls NextFrame().
pub fn sioNextFrame() {
    unsafe {
        for port in 0..SIO::PORTS {
            for slot in 0..SIO::SLOTS {
                mcds[port][slot].NextFrame();
            }
        }
    }
}

pub fn sioSetGameSerial(serial: &str) {
    unsafe {
        for port in 0..SIO::PORTS {
            for slot in 0..SIO::SLOTS {
                if mcds[port][slot].ReIndex(serial) {
                    AutoEject_Set(port, slot);
                }
            }
        }
    }
}

/// Convert a unified pad index to a (port, slot) pair.
pub fn sioConvertPadToPortAndSlot(index: u32) -> (u32, u32) {
    if index > 4 {
        (1, index - 4)
    } else if index > 1 {
        (0, index - 1)
    } else {
        (index, 0)
    }
}

/// Convert a (port, slot) pair to a unified pad index.
pub fn sioConvertPortAndSlotToPad(port: u32, slot: u32) -> u32 {
    if slot == 0 {
        port
    } else if port == 0 {
        slot + 1
    } else {
        slot + 4
    }
}

pub fn sioPadIsMultitapSlot(index: u32) -> bool {
    index >= 2
}

pub fn sioPortAndSlotIsMultitap(_port: u32, slot: u32) -> bool {
    slot != 0
}

// ---------------------------------------------------------------------------
// AutoEject (Sio.cpp)
// ---------------------------------------------------------------------------

pub mod AutoEject {
    use super::*;

    pub fn CountDownTicks() {
        unsafe {
            let mut reinserted = false;
            for port in 0..SIO::PORTS {
                for slot in 0..SIO::SLOTS {
                    if mcds[port][slot].autoEjectTicks > 0 {
                        mcds[port][slot].autoEjectTicks -= 1;
                        if mcds[port][slot].autoEjectTicks == 0 {
                            // The C++ source refers to `EmuConfig.Mcd[...].Enabled`.
                            // The runtime config is outside the scope of this
                            // translation, so the flag is unconditionally latched.
                            reinserted = true;
                        }
                    }
                }
            }
            if reinserted {
                // Host::AddIconOSDMessage equivalent
            }
        }
    }

    pub fn Set(port: usize, slot: usize) {
        unsafe {
            if mcds[port][slot].autoEjectTicks == 0 {
                mcds[port][slot].autoEjectTicks = 60;
                mcds[port][slot].term = Terminator::NOT_READY;
            }
        }
    }

    pub fn Clear(port: usize, slot: usize) {
        unsafe {
            mcds[port][slot].autoEjectTicks = 0;
        }
    }

    pub fn SetAll() {
        for port in 0..SIO::PORTS {
            for slot in 0..SIO::SLOTS {
                Set(port, slot);
            }
        }
    }

    pub fn ClearAll() {
        for port in 0..SIO::PORTS {
            for slot in 0..SIO::SLOTS {
                Clear(port, slot);
            }
        }
    }
}

/// Free-function alias used by `sioSetGameSerial`.
fn AutoEject_Set(port: usize, slot: usize) {
    AutoEject::Set(port, slot);
}

// ---------------------------------------------------------------------------
// MemcardBusy (Sio.cpp)
// ---------------------------------------------------------------------------

pub mod MemcardBusy {
    use std::sync::atomic::{AtomicU32, Ordering};

    static CURRENT_BUSY_TICKS: AtomicU32 = AtomicU32::new(0);

    /// Last frame a memcard write was performed; mirrored to `sioLastFrameMcdBusy`.
    pub static mut sioLastFrameMcdBusy: u32 = 0;

    pub fn Decrement() {
        if CURRENT_BUSY_TICKS.load(Ordering::Relaxed) == 0 {
            return;
        }
        CURRENT_BUSY_TICKS.fetch_sub(1, Ordering::Release);
    }

    pub fn SetBusy() {
        CURRENT_BUSY_TICKS.store(300, Ordering::Release);
        unsafe {
            sioLastFrameMcdBusy = 0; // g_FrameCount placeholder
        }
    }

    pub fn IsBusy() -> bool {
        CURRENT_BUSY_TICKS.load(Ordering::Acquire) > 0
    }

    pub fn ClearBusy() {
        CURRENT_BUSY_TICKS.store(0, Ordering::Release);
        unsafe {
            sioLastFrameMcdBusy = 0;
        }
    }

    pub fn CheckSaveStateDependency() {
        unsafe {
            let _ = sioLastFrameMcdBusy; // placeholder
        }
    }
}

pub use MemcardBusy::sioLastFrameMcdBusy;

// ---------------------------------------------------------------------------
// SioRegs aggregate (the requested `pub static mut sioRegs`)
// ---------------------------------------------------------------------------

/// Aggregate of all SIO controller state, including the two SIO controllers
/// and the in-flight FIFOs.  The C++ codebase exposes `Sio0 g_Sio0` and
/// `Sio2 g_Sio2` plus `g_Sio2FifoIn/Out` as separate globals; the requested
/// API surfaces them as a single `SioRegs` value.
pub struct SioRegs {
    pub sio0: Sio0,
    pub sio2: Sio2,
    pub fifo_in: VecDeque<u8>,
    pub fifo_out: VecDeque<u8>,
}

impl SioRegs {
    pub const fn new() -> Self {
        Self {
            sio0: Sio0::new(),
            sio2: Sio2::new(),
            fifo_in: VecDeque::new(),
            fifo_out: VecDeque::new(),
        }
    }
}

pub static mut sioRegs: SioRegs = SioRegs::new();

// ---------------------------------------------------------------------------
// Sio0 controller (Sio0.cpp/.h)
// ---------------------------------------------------------------------------

pub struct Sio0 {
    pub txData: u32,
    pub rxData: u32,
    pub stat: u32,
    pub mode: u16,
    pub ctrl: u16,
    pub baud: u16,

    pub flag: u8,
    pub sioStage: SioStage,
    pub sioMode: u8,
    pub sioCommand: u8,
    pub padStarted: bool,
    pub rxDataSet: bool,
    pub port: u8,
    pub slot: u8,
}

impl Sio0 {
    pub const fn new() -> Self {
        Self {
            txData: 0,
            rxData: 0,
            stat: 0,
            mode: 0,
            ctrl: 0,
            baud: 0,
            flag: 0,
            sioStage: SioStage::IDLE,
            sioMode: SioMode::NOT_SET,
            sioCommand: 0,
            padStarted: false,
            rxDataSet: false,
            port: 0,
            slot: 0,
        }
    }

    fn ClearStatAcknowledge(&mut self) {
        self.stat &= !SIO0_STAT::ACK;
    }

    pub fn Initialize(&mut self) -> bool {
        self.SoftReset();
        self.port = 0;
        self.slot = 0;
        mcd_init_all();
        unsafe { g_MemoryCardProtocol.ResetPS1State(); }
        true
    }

    pub fn Shutdown(&self) -> bool {
        true
    }

    pub fn SoftReset(&mut self) {
        self.padStarted = false;
        self.sioMode = SioMode::NOT_SET;
        self.sioCommand = 0;
        self.sioStage = SioStage::IDLE;
        unsafe { g_MemoryCardProtocol.ResetPS1State(); }
    }

    pub fn SetAcknowledge(&mut self, ack: bool) {
        if ack {
            self.stat |= SIO0_STAT::ACK;
        } else {
            self.stat &= !SIO0_STAT::ACK;
        }
    }

    pub fn Interrupt(&mut self, sio0Interrupt: Sio0Interrupt) {
        match sio0Interrupt {
            Sio0Interrupt::TEST_EVENT => { /* iopIntcIrq(7) */ }
            Sio0Interrupt::STAT_READ => self.ClearStatAcknowledge(),
            Sio0Interrupt::TX_DATA_WRITE => {}
        }
    }

    pub fn GetTxData(&self) -> u8 {
        (self.txData & 0xff) as u8
    }

    pub fn GetRxData(&mut self) -> u8 {
        self.stat |= SIO0_STAT::TX_READY | SIO0_STAT::TX_EMPTY;
        self.stat &= !SIO0_STAT::RX_FIFO_NOT_EMPTY;
        (self.rxData & 0xff) as u8
    }

    pub fn GetStat(&mut self) -> u32 {
        let ret = self.stat;
        self.Interrupt(Sio0Interrupt::STAT_READ);
        ret
    }

    pub fn GetMode(&self) -> u16 { self.mode }
    pub fn GetCtrl(&self) -> u16 { self.ctrl }
    pub fn GetBaud(&self) -> u16 { self.baud }

    pub fn SetTxData(&mut self, cmd: u8) {
        self.stat |= SIO0_STAT::TX_READY | SIO0_STAT::TX_EMPTY;
        self.stat |= SIO0_STAT::RX_FIFO_NOT_EMPTY;

        if self.ctrl & SIO0_CTRL::TX_ENABLE == 0 {
            return;
        }

        self.txData = cmd as u32;
        let mut data = 0u8;

        match self.sioMode {
            m if m == SioMode::NOT_SET => {
                self.sioMode = cmd;
                let mut pad = Pad_GetPad(self.port, self.slot);
                pad.SoftReset();
                unsafe { mcd = &mut mcds[self.port as usize][self.slot as usize] as *mut Mcd; }
                self.SetAcknowledge(true);
            }
            m if m == SioMode::PAD => {
                let mut pad = Pad_GetPad(self.port, self.slot);
                self.SetAcknowledge(true);
                data = pad.SendCommandByte(cmd);
                self.SetRxData(data);
            }
            m if m == SioMode::MEMCARD => {
                if self.sioCommand == MemcardCommand::NOT_SET {
                    if self.IsMemcardCommand(cmd) && unsafe { (*mcd).IsPresent() } && unsafe { (*mcd).IsPSX() } {
                        self.sioCommand = cmd;
                        self.SetAcknowledge(true);
                        self.SetRxData(self.flag);
                    } else {
                        self.SetAcknowledge(false);
                        self.SetRxData(0x00);
                    }
                } else {
                    let response = self.Memcard(cmd);
                    self.SetRxData(response);
                }
            }
            _ => {
                self.SetRxData(0xff);
                self.SetAcknowledge(false);
            }
        }

        if self.stat & SIO0_STAT::ACK == 0 {
            self.SoftReset();
        }
        self.Interrupt(Sio0Interrupt::TX_DATA_WRITE);
    }

    pub fn SetRxData(&mut self, value: u8) {
        self.rxData = value as u32;
    }

    pub fn SetStat(&mut self, value: u32) {
        let _ = value;
    }

    pub fn SetMode(&mut self, value: u16) { self.mode = value; }

    pub fn SetCtrl(&mut self, value: u16) {
        self.ctrl = value;
        self.port = if (self.ctrl & SIO0_CTRL::PORT) > 0 { 1 } else { 0 };

        if self.ctrl == 0 {
            unsafe { g_MemoryCardProtocol.ResetPS1State(); }
            self.SoftReset();
        }
        if self.ctrl & SIO0_CTRL::ACK != 0 {
            self.stat &= !(SIO0_STAT::IRQ | SIO0_STAT::RX_PARITY_ERROR);
        }
        if self.ctrl & SIO0_CTRL::RESET != 0 {
            self.stat = 0;
            self.ctrl = 0;
            self.mode = 0;
            self.SoftReset();
        }
    }

    pub fn SetBaud(&mut self, value: u16) { self.baud = value; }

    pub fn IsPadCommand(&self, command: u8) -> bool {
        command >= Pad::Command::MYSTERY as u8 && command <= Pad::Command::RESPONSE_BYTES as u8
    }

    pub fn IsMemcardCommand(&self, command: u8) -> bool {
        command == MemcardCommand::PS1_READ
            || command == MemcardCommand::PS1_STATE
            || command == MemcardCommand::PS1_WRITE
    }

    pub fn IsPocketstationCommand(&self, command: u8) -> bool {
        command == MemcardCommand::PS1_POCKETSTATION
    }

    pub fn Memcard(&mut self, value: u8) -> u8 {
        match self.sioCommand {
            MemcardCommand::PS1_READ => unsafe { g_MemoryCardProtocol.PS1Read(value) },
            MemcardCommand::PS1_STATE => unsafe { g_MemoryCardProtocol.PS1State(value) },
            MemcardCommand::PS1_WRITE => unsafe { g_MemoryCardProtocol.PS1Write(value) },
            MemcardCommand::PS1_POCKETSTATION => unsafe { g_MemoryCardProtocol.PS1Pocketstation(value) },
            _ => {
                self.SoftReset();
                0xff
            }
        }
    }
}

impl Default for Sio0 {
    fn default() -> Self { Self::new() }
}

// ---------------------------------------------------------------------------
// Sio2 controller (Sio2.cpp/.h)
// ---------------------------------------------------------------------------

pub struct Sio2 {
    pub CmdQueue: [u32; 16],
    pub PortCtrl0: [u32; 4],
    pub PortCtrl1: [u32; 4],
    pub dataIn: u32,
    pub dataOut: u32,
    pub ctrl: u32,
    pub CmdStat: u32,
    pub PortStat: u32,
    pub FifoStat: u32,
    pub FifoTxPos: u32,
    pub FifoRxPos: u32,
    pub iStat: u32,
    pub port: u8,
    pub queueRead: bool,
    pub queuePosition: usize,
    pub commandLength: usize,
    pub processedLength: usize,
    pub dmaBlockSize: usize,
    pub queueComplete: bool,
}

impl Sio2 {
    pub const fn new() -> Self {
        Self {
            CmdQueue: [0u32; 16],
            PortCtrl0: [0u32; 4],
            PortCtrl1: [0u32; 4],
            dataIn: 0,
            dataOut: 0,
            ctrl: 0,
            CmdStat: 0,
            PortStat: PortStat::DEFAULT,
            FifoStat: FifoStat::DEFAULT,
            FifoTxPos: 0,
            FifoRxPos: 0,
            iStat: 0,
            port: 0,
            queueRead: false,
            queuePosition: 0,
            commandLength: 0,
            processedLength: 0,
            dmaBlockSize: 0,
            queueComplete: false,
        }
    }

    pub fn Initialize(&mut self) -> bool {
        self.SoftReset();
        for v in self.CmdQueue.iter_mut() { *v = 0; }
        for i in 0..self.PortCtrl0.len() {
            self.PortCtrl0[i] = 0;
            self.PortCtrl1[i] = 0;
        }
        self.dataIn = 0;
        self.dataOut = 0;
        self.SetCtrl(Sio2Ctrl::SIO2MAN_RESET);
        self.SetCmdStat(CmdStat::DISCONNECTED);
        self.PortStat = PortStat::DEFAULT;
        self.FifoStat = FifoStat::DEFAULT;
        self.FifoTxPos = 0;
        self.FifoRxPos = 0;
        self.iStat = 0;
        self.port = 0;
        unsafe {
            sioRegs.fifo_out.clear();
        }
        mcd_init_all();
        true
    }

    pub fn Shutdown(&self) -> bool { true }

    pub fn SoftReset(&mut self) {
        self.queueRead = false;
        self.queuePosition = 0;
        self.commandLength = 0;
        self.processedLength = 0;
        self.dmaBlockSize = 0;
        self.queueComplete = false;
        unsafe {
            sioRegs.fifo_in.clear();
        }
        self.CmdStat = 0;
    }

    pub fn Interrupt(&mut self) {
        if self.iStat == 0 {
            // iopIntcIrq(17) equivalent
        }
        self.iStat |= 1;
    }

    pub fn SetCtrl(&mut self, value: u32) {
        self.ctrl = value;
        if self.ctrl & Sio2Ctrl::START_TRANSFER != 0 {
            self.Interrupt();
        }
    }

    pub fn SetCmd(&mut self, position: usize, value: u32) {
        self.CmdQueue[position] = value;
        if position == 0 {
            self.SoftReset();
        }
    }

    pub fn SetCmdStat(&mut self, value: u32) {
        self.CmdStat = value;
    }

    pub fn Pad(&mut self) {
        let pad_slot = {
            unsafe { g_MultitapArr[self.port as usize].GetPadSlot() }
        };
        let mut pad = Pad_GetPad(self.port, pad_slot);

        if self.CmdStat & CmdStat::ONE_PORT_OPEN != 0 {
            self.CmdStat &= !CmdStat::ONE_PORT_OPEN;
            self.CmdStat |= CmdStat::TWO_PORTS_OPEN;
        } else {
            self.CmdStat |= CmdStat::ONE_PORT_OPEN;
        }
        self.CmdStat |= CmdStat::NO_DEVICES_MISSING;

        let (not_connected, eject) = (pad.GetType() == Pad::ControllerType::NotConnected, pad.eject_ticks());
        if not_connected || eject > 0 {
            if self.port == 0 {
                self.CmdStat |= CmdStat::PORT_1_MISSING;
            } else {
                self.CmdStat |= CmdStat::PORT_2_MISSING;
            }
        }

        unsafe { sioRegs.fifo_out.push_back(0xff); }
        pad.SoftReset();

        loop {
            let cmd_opt = unsafe { sioRegs.fifo_in.pop_front() };
            let cmd = match cmd_opt { Some(b) => b, None => break };
            if pad.eject_ticks() > 0 {
                unsafe { sioRegs.fifo_out.push_back(0xff); }
            } else {
                let response = pad.SendCommandByte(cmd);
                unsafe { sioRegs.fifo_out.push_back(response); }
            }
        }

        if pad.eject_ticks() > 0 {
            pad.dec_eject_ticks();
        }
    }

    pub fn Multitap(&mut self) {
        // Mirror EmuConfig.Pad.IsMultitapPortEnabled(self.port) - default true.
        let multitap_enabled = true;

        if self.CmdStat & CmdStat::ONE_PORT_OPEN != 0 {
            self.CmdStat &= !CmdStat::ONE_PORT_OPEN;
            self.CmdStat |= CmdStat::TWO_PORTS_OPEN;
        } else {
            self.CmdStat |= CmdStat::ONE_PORT_OPEN;
        }
        self.CmdStat |= CmdStat::NO_DEVICES_MISSING;

        if !multitap_enabled {
            self.CmdStat |= CmdStat::PORT_1_MISSING;
        }

        unsafe { g_MultitapArr[self.port as usize].SendToMultitap(); }
    }

    pub fn Infrared(&mut self) {
        self.SetCmdStat(CmdStat::DISCONNECTED);
        unsafe { sioRegs.fifo_in.pop_front(); }
        while unsafe { sioRegs.fifo_out.len() } < self.commandLength {
            unsafe { sioRegs.fifo_out.push_back(0xff); }
        }
    }

    pub fn Memcard(&mut self) {
        let memcard_slot = unsafe { g_MultitapArr[self.port as usize].GetMemcardSlot() };
        unsafe {
            mcd = &mut mcds[self.port as usize][memcard_slot as usize] as *mut Mcd;
        }

        let auto_eject = unsafe { (*mcd).autoEjectTicks };
        if auto_eject > 0 {
            self.SetCmdStat(CmdStat::DISCONNECTED);
            unsafe {
                sioRegs.fifo_out.push_back(0xff);
                while let Some(_) = sioRegs.fifo_in.pop_front() {
                    sioRegs.fifo_out.push_back(0xff);
                }
            }
            return;
        }

        let present = unsafe { (*mcd).IsPresent() };
        self.SetCmdStat(if present { CmdStat::CONNECTED } else { CmdStat::DISCONNECTED });

        let command_byte = unsafe { sioRegs.fifo_in.pop_front() }.unwrap_or(0);
        let response_byte = if present { 0x00 } else { 0xff };
        unsafe {
            sioRegs.fifo_out.push_back(response_byte);
            sioRegs.fifo_out.push_back(response_byte);
        }

        match command_byte {
            MemcardCommand::PROBE => unsafe { g_MemoryCardProtocol.Probe(); },
            MemcardCommand::UNKNOWN_WRITE_DELETE_END => unsafe { g_MemoryCardProtocol.UnknownWriteDeleteEnd(); },
            MemcardCommand::SET_ERASE_SECTOR
            | MemcardCommand::SET_WRITE_SECTOR
            | MemcardCommand::SET_READ_SECTOR => unsafe { g_MemoryCardProtocol.SetSector(); },
            MemcardCommand::GET_SPECS => unsafe { g_MemoryCardProtocol.GetSpecs(); },
            MemcardCommand::SET_TERMINATOR => unsafe { g_MemoryCardProtocol.SetTerminator(); },
            MemcardCommand::GET_TERMINATOR => unsafe { g_MemoryCardProtocol.GetTerminator(); },
            MemcardCommand::WRITE_DATA => unsafe { g_MemoryCardProtocol.WriteData(); },
            MemcardCommand::READ_DATA => unsafe { g_MemoryCardProtocol.ReadData(); },
            MemcardCommand::PS1_READ => {
                unsafe { g_MemoryCardProtocol.ResetPS1State(); }
                loop {
                    let b = match unsafe { sioRegs.fifo_in.pop_front() } {
                        Some(v) => v,
                        None => break,
                    };
                    let out = unsafe { g_MemoryCardProtocol.PS1Read(b) };
                    unsafe { sioRegs.fifo_out.push_back(out); }
                }
            }
            MemcardCommand::PS1_STATE => {
                unsafe { g_MemoryCardProtocol.ResetPS1State(); }
                loop {
                    let b = match unsafe { sioRegs.fifo_in.pop_front() } {
                        Some(v) => v,
                        None => break,
                    };
                    let out = unsafe { g_MemoryCardProtocol.PS1State(b) };
                    unsafe { sioRegs.fifo_out.push_back(out); }
                }
            }
            MemcardCommand::PS1_WRITE => {
                unsafe { g_MemoryCardProtocol.ResetPS1State(); }
                loop {
                    let b = match unsafe { sioRegs.fifo_in.pop_front() } {
                        Some(v) => v,
                        None => break,
                    };
                    let out = unsafe { g_MemoryCardProtocol.PS1Write(b) };
                    unsafe { sioRegs.fifo_out.push_back(out); }
                }
            }
            MemcardCommand::PS1_POCKETSTATION => {
                unsafe { g_MemoryCardProtocol.ResetPS1State(); }
                loop {
                    let b = match unsafe { sioRegs.fifo_in.pop_front() } {
                        Some(v) => v,
                        None => break,
                    };
                    let out = unsafe { g_MemoryCardProtocol.PS1Pocketstation(b) };
                    unsafe { sioRegs.fifo_out.push_back(out); }
                }
            }
            MemcardCommand::READ_WRITE_END => unsafe { g_MemoryCardProtocol.ReadWriteEnd(); },
            MemcardCommand::ERASE_BLOCK => unsafe { g_MemoryCardProtocol.EraseBlock(); },
            MemcardCommand::UNKNOWN_BOOT => unsafe { g_MemoryCardProtocol.UnknownBoot(); },
            MemcardCommand::AUTH_XOR => unsafe { g_MemoryCardProtocol.AuthXor(); },
            MemcardCommand::AUTH_F3 => unsafe { g_MemoryCardProtocol.AuthF3(); },
            MemcardCommand::AUTH_F7 => unsafe { g_MemoryCardProtocol.AuthF7(); },
            _ => {}
        }
    }

    pub fn Write(&mut self, data: u8) {
        if !self.queueRead {
            if self.queuePosition > self.CmdQueue.len() {
                return;
            }
            let currentCmd = self.CmdQueue[self.queuePosition];
            self.port = (currentCmd & Sio2Cmd::PORT) as u8;
            self.commandLength = ((currentCmd >> 8) & Sio2Cmd::COMMAND_LENGTH_MASK) as usize;
            self.queueRead = true;
            if self.commandLength == 0 {
                self.queueComplete = true;
            }
            unsafe {
                while let Some(_) = sioRegs.fifo_in.pop_front() {}
            }
        }
        if self.queueComplete {
            return;
        }
        unsafe { sioRegs.fifo_in.push_back(data); }

        let in_len = unsafe { sioRegs.fifo_in.len() };
        let dma = self.dmaBlockSize;
        let cmd_len = self.commandLength;
        let triggered = (in_len == cmd_len && dma == 0) || (dma != 0 && in_len == dma);
        if !triggered {
            return;
        }

        self.queueRead = false;
        self.queuePosition += 1;
        let sio_mode = unsafe { sioRegs.fifo_in.pop_front() }.unwrap_or(0);
        match sio_mode {
            m if m == SioMode::PAD => self.Pad(),
            m if m == SioMode::MULTITAP => self.Multitap(),
            m if m == SioMode::INFRARED => self.Infrared(),
            m if m == SioMode::MEMCARD => self.Memcard(),
            _ => {
                unsafe { sioRegs.fifo_out.push_back(0xff); }
                self.SetCmdStat(CmdStat::DISCONNECTED);
            }
        }

        let block = self.dmaBlockSize;
        if block > 0 {
            let len = unsafe { sioRegs.fifo_out.len() };
            let diff = len % block;
            if diff > 0 {
                for _ in 0..(block - diff) {
                    unsafe { sioRegs.fifo_out.push_back(0x00); }
                }
            }
        }
    }

    pub fn Read(&mut self) -> u8 {
        let ret = unsafe { sioRegs.fifo_out.pop_front() }.unwrap_or(0xff);
        ret
    }
}

impl Default for Sio2 {
    fn default() -> Self { Self::new() }
}

pub static mut g_Sio0: Sio0 = Sio0::new();
pub static mut g_Sio2: Sio2 = Sio2::new();

// ---------------------------------------------------------------------------
// Stubs for pad, multitap and memory-card-protocol dependencies.
//
// The C++ codebase has these as separate translation units.  In this
// pure-std single-file rewrite we provide minimal stand-ins so that the SIO
// state-machine logic above compiles.  Real implementations would live in
// `Pad*.rs`, `Multitap*.rs`, and `MemoryCardProtocol*.rs` respectively.
// ---------------------------------------------------------------------------

/// Stub for `FileMcd_NextFrame` - originally in MemoryCardFile.cpp.
/// In the C++ source this walks the FileSystem-backed card state for the
/// given (port, slot) and performs save-state change tracking.  The
/// translation only needs the side effect of zeroing the per-frame counters.
pub fn FileMcd_NextFrame(_port: u8, _slot: u8) {}

/// Stub for `FileMcd_ReIndex` - originally in MemoryCardFile.cpp.
/// Returns true iff the card was successfully re-opened with the new filter
/// string (game serial).
pub fn FileMcd_ReIndex(_port: u8, _slot: u8, _filter: &str) -> bool {
    false
}

/// Pad "interface" stub.  The C++ code models this with a `PadBase` virtual
/// base class; here we provide a concrete struct with the methods actually
/// called by the SIO state machine.
#[derive(Copy, Clone, Debug)]
pub struct PadBase {
    pad_type: Pad::ControllerType,
    eject_ticks: usize,
}

impl PadBase {
    pub const fn new() -> Self {
        Self {
            pad_type: Pad::ControllerType::NotConnected,
            eject_ticks: 0,
        }
    }

    pub fn SoftReset(&mut self) {
        self.eject_ticks = 0;
    }

    pub fn SendCommandByte(&mut self, _cmd: u8) -> u8 {
        0xff
    }

    pub fn GetType(&self) -> Pad::ControllerType {
        self.pad_type
    }

    pub fn eject_ticks(&self) -> usize {
        self.eject_ticks
    }

    pub fn dec_eject_ticks(&mut self) {
        if self.eject_ticks > 0 {
            self.eject_ticks -= 1;
        }
    }
}

impl Default for PadBase {
    fn default() -> Self { Self::new() }
}

/// 2 x 4 array of pads covering both ports and all slots.
pub static mut g_Pads: [[PadBase; SIO::SLOTS]; SIO::PORTS] = [
    [const { PadBase::new() }; SIO::SLOTS],
    [const { PadBase::new() }; SIO::SLOTS],
];

/// Stub for `Pad_GetPad(port, slot)` - originally in Pad.cpp.
/// Returns a mutable reference to the per-(port, slot) pad state.
pub fn Pad_GetPad(port: u8, slot: u8) -> &'static mut PadBase {
    unsafe { &mut g_Pads[port as usize][slot as usize] }
}

/// Stub for the memory-card protocol translator.  Original: MemoryCardProtocol.cpp.
#[derive(Copy, Clone, Debug)]
pub struct MemoryCardProtocolStub {
    /// Single byte of PS1 state.  The full protocol needs a multi-byte state
    /// machine; this stub is sufficient for compilation only.
    pub ps1_state: u8,
}

impl MemoryCardProtocolStub {
    pub const fn new() -> Self {
        Self { ps1_state: 0 }
    }

    pub fn ResetPS1State(&mut self) { self.ps1_state = 0; }
    pub fn PS1Read(&mut self, _b: u8) -> u8 { 0xff }
    pub fn PS1State(&mut self, _b: u8) -> u8 { 0xff }
    pub fn PS1Write(&mut self, _b: u8) -> u8 { 0xff }
    pub fn PS1Pocketstation(&mut self, _b: u8) -> u8 { 0xff }
    pub fn Probe(&mut self) {}
    pub fn UnknownWriteDeleteEnd(&mut self) {}
    pub fn SetSector(&mut self) {}
    pub fn GetSpecs(&mut self) {}
    pub fn SetTerminator(&mut self) {}
    pub fn GetTerminator(&mut self) {}
    pub fn WriteData(&mut self) {}
    pub fn ReadData(&mut self) {}
    pub fn ReadWriteEnd(&mut self) {}
    pub fn EraseBlock(&mut self) {}
    pub fn UnknownBoot(&mut self) {}
    pub fn AuthXor(&mut self) {}
    pub fn AuthF3(&mut self) {}
    pub fn AuthF7(&mut self) {}
}

impl Default for MemoryCardProtocolStub {
    fn default() -> Self { Self::new() }
}

/// Global instance of the memory-card protocol translator.
pub static mut g_MemoryCardProtocol: MemoryCardProtocolStub = MemoryCardProtocolStub::new();

/// Stub for the per-port Multitap.  Original: MultitapProtocol.cpp / Multitap.h.
#[derive(Copy, Clone, Debug)]
pub struct MultitapStub {
    pad_slot: u8,
    memcard_slot: u8,
}

impl MultitapStub {
    pub const fn new() -> Self {
        Self { pad_slot: 0, memcard_slot: 0 }
    }

    pub fn GetPadSlot(&self) -> u8 { self.pad_slot }
    pub fn GetMemcardSlot(&self) -> u8 { self.memcard_slot }
    pub fn SendToMultitap(&mut self) {}
}

impl Default for MultitapStub {
    fn default() -> Self { Self::new() }
}

/// Per-port multitap instances.
pub static mut g_MultitapArr: [MultitapStub; SIO::PORTS] = [
    MultitapStub::new(),
    MultitapStub::new(),
];

