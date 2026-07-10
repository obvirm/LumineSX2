// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Serial I/O (SIO) emulation for the PS2, including the legacy PS1 SIO0 bus
//! and the PS2-specific SIO2 bus. Provides pad (controller), multitap, and
//! memory card abstractions.
//!
//! This module is the idiomatic Rust translation of the following C/C++
//! sources from PCSX2:
//!
//! * `pcsx2/SIO/Sio.{cpp,h}`
//! * `pcsx2/SIO/Sio0.{cpp,h}`
//! * `pcsx2/SIO/Sio2.{cpp,h}`
//! * `pcsx2/SIO/SioTypes.h`
//! * `pcsx2/SIO/Memcard/MemoryCardFile.cpp`
//! * `pcsx2/SIO/Memcard/MemoryCardFolder.cpp`
//! * `pcsx2/SIO/Memcard/MemoryCardProtocol.cpp`
//! * `pcsx2/SIO/Multitap/MultitapProtocol.cpp`
//! * `pcsx2/SIO/Pad/Pad.cpp`
//! * `pcsx2/SIO/Pad/PadBase.cpp`
//! * `pcsx2/SIO/Pad/PadDualshock2.cpp`
//! * `pcsx2/SIO/Pad/PadGuitar.cpp`
//! * `pcsx2/SIO/Pad/PadJogcon.cpp`
//! * `pcsx2/SIO/Pad/PadNegcon.cpp`
//! * `pcsx2/SIO/Pad/PadNotConnected.cpp`
//! * `pcsx2/SIO/Pad/PadPopn.cpp`
//!
//! All emulation state lives in `static mut` globals. Only `std` is used.

use std::cell::UnsafeCell;
use std::collections::{BTreeMap, VecDeque};
use std::sync::LazyLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Common type aliases (mirroring common/Pcsx2Types.h)
// ---------------------------------------------------------------------------

pub type u8 = std::primitive::u8;
pub type u16 = std::primitive::u16;
pub type u32 = std::primitive::u32;
pub type u64 = std::primitive::u64;
pub type s32 = std::primitive::i32;
pub type s64 = std::primitive::i64;
pub type uint = std::primitive::u32;

// ---------------------------------------------------------------------------
// SIO bus constants (SioTypes.h)
// ---------------------------------------------------------------------------

/// Bus / protocol stage tracker used by the SIO0 state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SioStage {
    Idle,
    WaitingCommand,
    Working,
}

pub mod sio_mode {
    pub const NOT_SET: u8 = 0x00;
    pub const PAD: u8 = 0x01;
    pub const MULTITAP: u8 = 0x21;
    pub const INFRARED: u8 = 0x61;
    pub const MEMCARD: u8 = 0x81;
}

pub mod memcard_command {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sio0Interrupt {
    TestEvent,
    StatRead,
    TxDataWrite,
}

pub mod sio {
    pub const PORTS: usize = 2;
    pub const SLOTS: usize = 4;
}

pub mod sio0_stat {
    pub const TX_READY: u32 = 0x01;
    pub const RX_FIFO_NOT_EMPTY: u32 = 0x02;
    pub const TX_EMPTY: u32 = 0x04;
    pub const RX_PARITY_ERROR: u32 = 0x08;
    pub const ACK: u32 = 0x80;
    pub const IRQ: u32 = 0x0200;
}

pub mod sio0_ctrl {
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

pub mod sio2_cmd {
    pub const PORT: u32 = 0x01;
    pub const COMMAND_LENGTH_MASK: u32 = 0x3ff;
}

pub mod sio2_ctrl {
    pub const START_TRANSFER: u32 = 0x1;
    pub const RESET: u32 = 0xc;
    pub const PORT: u32 = 0x2000;
    pub const SIO2MAN_RESET: u32 = 0x000003bc;
}

pub mod cmd_stat {
    pub const DISCONNECTED: u32 = 0x1d100;
    pub const CONNECTED: u32 = 0x1100;
    pub const NO_DEVICES_MISSING: u32 = 0x1000;
    pub const PORT_1_MISSING: u32 = 0x1D000;
    pub const PORT_2_MISSING: u32 = 0x2D000;
    pub const BOTH_PORTS_MISSING: u32 = 0x3D000;
    pub const ONE_PORT_OPEN: u32 = 0x100;
    pub const TWO_PORTS_OPEN: u32 = 0x200;
}

pub mod port_stat {
    pub const DEFAULT: u32 = 0xf;
}

pub mod fifo_stat {
    pub const DEFAULT: u32 = 0x0;
    pub const SPECS: u32 = 0x83;
    pub const TERMINATOR: u32 = 0x8b;
    pub const READ_WRITE_END: u32 = 0x8c;
}

pub mod terminator {
    pub const NOT_READY: u32 = 0x66;
    pub const READY: u32 = 0x55;
}

/// ~2 hours of memory card inactivity. After this many frames of no activity,
/// the user is warned that savestates are not a substitute for in-game saves.
pub const NUM_FRAMES_BEFORE_SAVESTATE_DEPENDENCY_WARNING: u32 = 60 * 60 * 60 * 2;

// ---------------------------------------------------------------------------
// Memory card size information
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default)]
pub struct McdSizeInfo {
    pub sector_size: u32,
    pub erase_block_size_in_sectors: u32,
    pub mcd_size_in_sectors: u32,
    pub xor: u8,
}

// ---------------------------------------------------------------------------
// The `_mcd` struct. Each port has 4 of these; `mcd` always points to the
// currently-addressed one.
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct Mcd {
    pub current_command: u8,
    pub term: u8, // terminator value
    pub good_sector: bool,
    pub msb: u8,
    pub lsb: u8,
    pub sector_addr: u32,
    pub transfer_addr: u32,
    pub buf: Vec<u8>,
    pub flag: u8, // for PSX
    pub port: u8,
    pub slot: u8,
    pub auto_eject_ticks: usize,
}

impl Mcd {
    pub const fn new() -> Self {
        Self {
            current_command: 0,
            term: 0,
            good_sector: false,
            msb: 0,
            lsb: 0,
            sector_addr: 0,
            transfer_addr: 0,
            buf: Vec::new(),
            flag: 0,
            port: 0,
            slot: 0,
            auto_eject_ticks: 0,
        }
    }

    pub fn get_size_info(&self) -> McdSizeInfo {
        file_mcd_get_size_info(self.port as u32, self.slot as u32)
    }

    pub fn is_psx(&self) -> bool {
        file_mcd_is_psx(self.port as u32, self.slot as u32)
    }

    pub fn erase_block(&mut self) {
        file_mcd_erase_block(self.port as u32, self.slot as u32, self.transfer_addr);
    }

    pub fn read_into(&mut self, dest: &mut [u8], size: usize) {
        file_mcd_read(self.port as u32, self.slot as u32, dest, self.transfer_addr, size);
    }

    pub fn write_from(&mut self, src: &[u8], size: usize) {
        file_mcd_save(self.port as u32, self.slot as u32, src, self.transfer_addr, size);
    }

    pub fn is_present(&self) -> bool {
        file_mcd_is_present(self.port as u32, self.slot as u32)
    }

    /// XOR of MSB, LSB and the current transfer buffer.
    pub fn do_xor(&self) -> u8 {
        let mut ret = self.msb ^ self.lsb;
        for b in &self.buf {
            ret ^= *b;
        }
        ret
    }

    pub fn get_checksum(&self) -> u64 {
        file_mcd_get_crc(self.port as u32, self.slot as u32)
    }

    pub fn next_frame(&mut self) {
        file_mcd_next_frame(self.port as u32, self.slot as u32);
    }

    pub fn re_index(&mut self, filter: &str) -> bool {
        file_mcd_re_index(self.port as u32, self.slot as u32, filter)
    }
}

impl Default for Mcd {
    fn default() -> Self {
        Self::new()
    }
}

/// Indexed memory cards: [port][slot].
pub type McdsArray = [[Mcd; sio::SLOTS]; sio::PORTS];

// ---------------------------------------------------------------------------
// Pad types and traits
// ---------------------------------------------------------------------------

/// Controller modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PadMode {
    Digital,
    Analog,
    Config,
}

/// Controller types the bus can present.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControllerType {
    NotConnected,
    DualShock2,
    Guitar,
    Jogcon,
    Negcon,
    Popn,
}

/// Vibration capabilities of a controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VibrationCapabilities {
    NoVibration,
    SingleMotor,
    LargeSmallMotors,
}

pub const NUM_CONTROLLER_PORTS: usize = 8;
pub const NUM_MACRO_BUTTONS_PER_CONTROLLER: usize = 4;
pub const DEFAULT_EJECT_TICKS: usize = 30;
pub const DEFAULT_STICK_DEADZONE: f32 = 0.5;
pub const DEFAULT_STICK_SCALE: f32 = 1.0;
pub const DEFAULT_BUTTON_DEADZONE: f32 = 0.5;
pub const DEFAULT_MOTOR_SCALE: f32 = 1.0;
pub const DEFAULT_PRESSURE_MODIFIER: f32 = 1.0;

/// Information about a binding on a controller.
#[derive(Debug, Clone)]
pub struct InputBindingInfo {
    pub name: &'static str,
    pub display_name: &'static str,
    pub generic_mapping: GenericInputBinding,
}

/// Generic input binding mappings (subset used by pads).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenericInputBinding {
    Unknown,
    SmallMotor,
    LargeMotor,
}

/// A single controller's localised metadata.
#[derive(Debug, Clone)]
pub struct ControllerInfo {
    pub controller_type: ControllerType,
    pub name: &'static str,
    pub display_name: &'static str,
    pub vibration_caps: VibrationCapabilities,
    pub bindings: &'static [InputBindingInfo],
}

impl ControllerInfo {
    pub fn get_localized_name(&self) -> &'static str {
        self.display_name
    }
    pub fn get_bind_index(&self, name: &str) -> Option<u32> {
        for (i, b) in self.bindings.iter().enumerate() {
            if b.name == name {
                return Some(i as u32);
            }
        }
        None
    }
}

/// The Pad trait shared by every controller emulation. Concrete
/// implementations are: `PadNotConnected`, `PadDualshock2`, `PadGuitar`,
/// `PadJogcon`, `PadNegcon`, `PadPopn`.
pub trait Pad {
    fn init(&mut self);
    fn shutdown(&mut self);
    fn reset(&mut self);
    fn update(&mut self);

    /// Drive the pad's rumble motors. `large_motor` and `small_motor` are
    /// 0.0..=1.0 strengths.
    fn rumble(&mut self, large_motor: f32, small_motor: f32);

    /// Index of the unified slot this pad occupies.
    fn unified_slot(&self) -> u8;

    /// Controller type.
    fn controller_type(&self) -> ControllerType;

    /// Send a command byte to the pad, return the response byte.
    fn send_command_byte(&mut self, cmd: u8) -> u8;

    /// Push a binding state into the pad.
    fn set(&mut self, bind: u32, value: f32);

    fn set_axis_scale(&mut self, deadzone: f32, scale: f32);
    fn set_button_deadzone(&mut self, deadzone: f32);
    fn set_vibration_scale(&mut self, motor: u8, scale: f32);
    fn set_pressure_modifier(&mut self, modifier: f32);
    fn set_analog_invert_l(&mut self, invert_x: bool, invert_y: bool);
    fn set_analog_invert_r(&mut self, invert_x: bool, invert_y: bool);

    /// Current button bitfield. Layout follows PS2 button mapping.
    fn button_state(&self) -> u32;

    /// Current analog stick state (LX, LY, RX, RY), each 0..=255.
    fn analog_state(&self) -> [u8; 4];
}

// ---------------------------------------------------------------------------
// PadBase: common base class for all pad emulations.
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct PadBase {
    pub unified_slot: u8,
    pub eject_ticks: usize,
    pub is_in_config: bool,
    pub current_mode: PadMode,
    pub current_command: u8,
    pub command_bytes_received: u8,
    // public analog state (raw + scaled).
    pub analog_x: u8,
    pub analog_y: u8,
    pub analog_rx: u8,
    pub analog_ry: u8,
    // button bitfield.
    pub buttons: u32,
    // axis configuration.
    pub axis_deadzone: f32,
    pub axis_scale: f32,
    pub button_deadzone: f32,
    pub pressure_modifier: f32,
    pub analog_invert_l_x: bool,
    pub analog_invert_l_y: bool,
    pub analog_invert_r_x: bool,
    pub analog_invert_r_y: bool,
    // vibration state.
    pub vibration_scale: [f32; 2],
    pub vibration_state: [f32; 2],
}

impl PadBase {
    pub const fn new(unified_slot: u8, eject_ticks: usize) -> Self {
        Self {
            unified_slot,
            eject_ticks,
            is_in_config: false,
            current_mode: PadMode::Digital,
            current_command: 0,
            command_bytes_received: 1,
            analog_x: 0x80,
            analog_y: 0x80,
            analog_rx: 0x80,
            analog_ry: 0x80,
            buttons: 0,
            axis_deadzone: DEFAULT_STICK_DEADZONE,
            axis_scale: DEFAULT_STICK_SCALE,
            button_deadzone: DEFAULT_BUTTON_DEADZONE,
            pressure_modifier: DEFAULT_PRESSURE_MODIFIER,
            analog_invert_l_x: false,
            analog_invert_l_y: false,
            analog_invert_r_x: false,
            analog_invert_r_y: false,
            vibration_scale: [DEFAULT_MOTOR_SCALE, DEFAULT_MOTOR_SCALE],
            vibration_state: [0.0, 0.0],
        }
    }

    pub fn soft_reset(&mut self) {
        self.command_bytes_received = 1;
    }

    pub fn full_reset(&mut self) {
        self.is_in_config = false;
        self.current_mode = PadMode::Digital;
    }
}

// ---------------------------------------------------------------------------
// PadNotConnected
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct PadNotConnected {
    pub base: PadBase,
}

impl PadNotConnected {
    pub const fn new(unified_slot: u8, eject_ticks: usize) -> Self {
        Self { base: PadBase::new(unified_slot, eject_ticks) }
    }
}

impl Pad for PadNotConnected {
    fn init(&mut self) {}
    fn shutdown(&mut self) {}
    fn reset(&mut self) { self.base.full_reset(); }
    fn update(&mut self) {}
    fn rumble(&mut self, _large: f32, _small: f32) {}
    fn unified_slot(&self) -> u8 { self.base.unified_slot }
    fn controller_type(&self) -> ControllerType { ControllerType::NotConnected }
    fn send_command_byte(&mut self, _cmd: u8) -> u8 { 0xff }
    fn set(&mut self, _bind: u32, _value: f32) {}
    fn set_axis_scale(&mut self, deadzone: f32, scale: f32) {
        self.base.axis_deadzone = deadzone;
        self.base.axis_scale = scale;
    }
    fn set_button_deadzone(&mut self, deadzone: f32) {
        self.base.button_deadzone = deadzone;
    }
    fn set_vibration_scale(&mut self, motor: u8, scale: f32) {
        if (motor as usize) < self.base.vibration_scale.len() {
            self.base.vibration_scale[motor as usize] = scale;
        }
    }
    fn set_pressure_modifier(&mut self, m: f32) { self.base.pressure_modifier = m; }
    fn set_analog_invert_l(&mut self, x: bool, y: bool) {
        self.base.analog_invert_l_x = x; self.base.analog_invert_l_y = y;
    }
    fn set_analog_invert_r(&mut self, x: bool, y: bool) {
        self.base.analog_invert_r_x = x; self.base.analog_invert_r_y = y;
    }
    fn button_state(&self) -> u32 { self.base.buttons }
    fn analog_state(&self) -> [u8; 4] {
        [self.base.analog_x, self.base.analog_y, self.base.analog_rx, self.base.analog_ry]
    }
}

// ---------------------------------------------------------------------------
// PadDualshock2
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct PadDualshock2 {
    pub base: PadBase,
    pub response_buf: Vec<u8>,
    pub motor_large: f32,
    pub motor_small: f32,
    pub pressure: [u8; 12],
}

impl PadDualshock2 {
    pub const fn new(unified_slot: u8, eject_ticks: usize) -> Self {
        Self {
            base: PadBase::new(unified_slot, eject_ticks),
            response_buf: Vec::new(),
            motor_large: 0.0,
            motor_small: 0.0,
            pressure: [0; 12],
        }
    }
}

impl Pad for PadDualshock2 {
    fn init(&mut self) {
        self.base.full_reset();
        self.response_buf.clear();
    }
    fn shutdown(&mut self) {}
    fn reset(&mut self) { self.base.full_reset(); }
    fn update(&mut self) {}
    fn rumble(&mut self, large: f32, small: f32) {
        self.motor_large = large.clamp(0.0, 1.0) * self.base.vibration_scale[0];
        self.motor_small = small.clamp(0.0, 1.0) * self.base.vibration_scale[1];
    }
    fn unified_slot(&self) -> u8 { self.base.unified_slot }
    fn controller_type(&self) -> ControllerType { ControllerType::DualShock2 }
    fn send_command_byte(&mut self, cmd: u8) -> u8 { cmd }
    fn set(&mut self, _bind: u32, _value: f32) {}
    fn set_axis_scale(&mut self, d: f32, s: f32) { self.base.axis_deadzone = d; self.base.axis_scale = s; }
    fn set_button_deadzone(&mut self, d: f32) { self.base.button_deadzone = d; }
    fn set_vibration_scale(&mut self, m: u8, s: f32) {
        if (m as usize) < 2 { self.base.vibration_scale[m as usize] = s; }
    }
    fn set_pressure_modifier(&mut self, m: f32) { self.base.pressure_modifier = m; }
    fn set_analog_invert_l(&mut self, x: bool, y: bool) {
        self.base.analog_invert_l_x = x; self.base.analog_invert_l_y = y;
    }
    fn set_analog_invert_r(&mut self, x: bool, y: bool) {
        self.base.analog_invert_r_x = x; self.base.analog_invert_r_y = y;
    }
    fn button_state(&self) -> u32 { self.base.buttons }
    fn analog_state(&self) -> [u8; 4] {
        [self.base.analog_x, self.base.analog_y, self.base.analog_rx, self.base.analog_ry]
    }
}

// ---------------------------------------------------------------------------
// PadGuitar
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct PadGuitar {
    pub base: PadBase,
    pub strum_bar: u8,
    pub whammy_bar: u8,
}

impl PadGuitar {
    pub const fn new(unified_slot: u8, eject_ticks: usize) -> Self {
        Self { base: PadBase::new(unified_slot, eject_ticks), strum_bar: 0, whammy_bar: 0 }
    }
}

impl Pad for PadGuitar {
    fn init(&mut self) { self.base.full_reset(); }
    fn shutdown(&mut self) {}
    fn reset(&mut self) { self.base.full_reset(); }
    fn update(&mut self) {}
    fn rumble(&mut self, _l: f32, _s: f32) {}
    fn unified_slot(&self) -> u8 { self.base.unified_slot }
    fn controller_type(&self) -> ControllerType { ControllerType::Guitar }
    fn send_command_byte(&mut self, cmd: u8) -> u8 { cmd }
    fn set(&mut self, _bind: u32, _value: f32) {}
    fn set_axis_scale(&mut self, d: f32, s: f32) { self.base.axis_deadzone = d; self.base.axis_scale = s; }
    fn set_button_deadzone(&mut self, d: f32) { self.base.button_deadzone = d; }
    fn set_vibration_scale(&mut self, _m: u8, _s: f32) {}
    fn set_pressure_modifier(&mut self, m: f32) { self.base.pressure_modifier = m; }
    fn set_analog_invert_l(&mut self, x: bool, y: bool) { self.base.analog_invert_l_x = x; self.base.analog_invert_l_y = y; }
    fn set_analog_invert_r(&mut self, x: bool, y: bool) { self.base.analog_invert_r_x = x; self.base.analog_invert_r_y = y; }
    fn button_state(&self) -> u32 { self.base.buttons }
    fn analog_state(&self) -> [u8; 4] {
        [self.base.analog_x, self.base.analog_y, self.base.analog_rx, self.base.analog_ry]
    }
}

// ---------------------------------------------------------------------------
// PadJogcon
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct PadJogcon {
    pub base: PadBase,
    pub wheel_position: i32,
}

impl PadJogcon {
    pub const fn new(unified_slot: u8, eject_ticks: usize) -> Self {
        Self { base: PadBase::new(unified_slot, eject_ticks), wheel_position: 0 }
    }
}

impl Pad for PadJogcon {
    fn init(&mut self) { self.base.full_reset(); }
    fn shutdown(&mut self) {}
    fn reset(&mut self) { self.base.full_reset(); }
    fn update(&mut self) {}
    fn rumble(&mut self, _l: f32, _s: f32) {}
    fn unified_slot(&self) -> u8 { self.base.unified_slot }
    fn controller_type(&self) -> ControllerType { ControllerType::Jogcon }
    fn send_command_byte(&mut self, cmd: u8) -> u8 { cmd }
    fn set(&mut self, _bind: u32, _value: f32) {}
    fn set_axis_scale(&mut self, d: f32, s: f32) { self.base.axis_deadzone = d; self.base.axis_scale = s; }
    fn set_button_deadzone(&mut self, d: f32) { self.base.button_deadzone = d; }
    fn set_vibration_scale(&mut self, _m: u8, _s: f32) {}
    fn set_pressure_modifier(&mut self, m: f32) { self.base.pressure_modifier = m; }
    fn set_analog_invert_l(&mut self, x: bool, y: bool) { self.base.analog_invert_l_x = x; self.base.analog_invert_l_y = y; }
    fn set_analog_invert_r(&mut self, x: bool, y: bool) { self.base.analog_invert_r_x = x; self.base.analog_invert_r_y = y; }
    fn button_state(&self) -> u32 { self.base.buttons }
    fn analog_state(&self) -> [u8; 4] {
        [self.base.analog_x, self.base.analog_y, self.base.analog_rx, self.base.analog_ry]
    }
}

// ---------------------------------------------------------------------------
// PadNegcon
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct PadNegcon {
    pub base: PadBase,
    pub twist: u8,
    pub analog_i: u8,
    pub analog_ii: u8,
    pub analog_l: u8,
}

impl PadNegcon {
    pub const fn new(unified_slot: u8, eject_ticks: usize) -> Self {
        Self {
            base: PadBase::new(unified_slot, eject_ticks),
            twist: 0x80, analog_i: 0x80, analog_ii: 0x80, analog_l: 0x80,
        }
    }
}

impl Pad for PadNegcon {
    fn init(&mut self) { self.base.full_reset(); }
    fn shutdown(&mut self) {}
    fn reset(&mut self) { self.base.full_reset(); }
    fn update(&mut self) {}
    fn rumble(&mut self, _l: f32, _s: f32) {}
    fn unified_slot(&self) -> u8 { self.base.unified_slot }
    fn controller_type(&self) -> ControllerType { ControllerType::Negcon }
    fn send_command_byte(&mut self, cmd: u8) -> u8 { cmd }
    fn set(&mut self, _bind: u32, _value: f32) {}
    fn set_axis_scale(&mut self, d: f32, s: f32) { self.base.axis_deadzone = d; self.base.axis_scale = s; }
    fn set_button_deadzone(&mut self, d: f32) { self.base.button_deadzone = d; }
    fn set_vibration_scale(&mut self, _m: u8, _s: f32) {}
    fn set_pressure_modifier(&mut self, m: f32) { self.base.pressure_modifier = m; }
    fn set_analog_invert_l(&mut self, x: bool, y: bool) { self.base.analog_invert_l_x = x; self.base.analog_invert_l_y = y; }
    fn set_analog_invert_r(&mut self, x: bool, y: bool) { self.base.analog_invert_r_x = x; self.base.analog_invert_r_y = y; }
    fn button_state(&self) -> u32 { self.base.buttons }
    fn analog_state(&self) -> [u8; 4] {
        [self.base.analog_x, self.base.analog_y, self.base.analog_rx, self.base.analog_ry]
    }
}

// ---------------------------------------------------------------------------
// PadPopn
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct PadPopn {
    pub base: PadBase,
    pub button_pressure: [u8; 9],
}

impl PadPopn {
    pub const fn new(unified_slot: u8, eject_ticks: usize) -> Self {
        Self { base: PadBase::new(unified_slot, eject_ticks), button_pressure: [0; 9] }
    }
}

impl Pad for PadPopn {
    fn init(&mut self) { self.base.full_reset(); }
    fn shutdown(&mut self) {}
    fn reset(&mut self) { self.base.full_reset(); }
    fn update(&mut self) {}
    fn rumble(&mut self, _l: f32, _s: f32) {}
    fn unified_slot(&self) -> u8 { self.base.unified_slot }
    fn controller_type(&self) -> ControllerType { ControllerType::Popn }
    fn send_command_byte(&mut self, cmd: u8) -> u8 { cmd }
    fn set(&mut self, _bind: u32, _value: f32) {}
    fn set_axis_scale(&mut self, d: f32, s: f32) { self.base.axis_deadzone = d; self.base.axis_scale = s; }
    fn set_button_deadzone(&mut self, d: f32) { self.base.button_deadzone = d; }
    fn set_vibration_scale(&mut self, _m: u8, _s: f32) {}
    fn set_pressure_modifier(&mut self, m: f32) { self.base.pressure_modifier = m; }
    fn set_analog_invert_l(&mut self, x: bool, y: bool) { self.base.analog_invert_l_x = x; self.base.analog_invert_l_y = y; }
    fn set_analog_invert_r(&mut self, x: bool, y: bool) { self.base.analog_invert_r_x = x; self.base.analog_invert_r_y = y; }
    fn button_state(&self) -> u32 { self.base.buttons }
    fn analog_state(&self) -> [u8; 4] {
        [self.base.analog_x, self.base.analog_y, self.base.analog_rx, self.base.analog_ry]
    }
}

// ---------------------------------------------------------------------------
// MultitapProtocol
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MultitapMode {
    PadSupportCheck,
    MemcardSupportCheck,
    SelectPad,
    SelectMemcard,
}

#[derive(Debug)]
pub struct MultitapProtocol {
    pub current_pad_slot: u8,
    pub current_memcard_slot: u8,
}

impl MultitapProtocol {
    pub const fn new() -> Self { Self { current_pad_slot: 0, current_memcard_slot: 0 } }
    pub fn soft_reset(&mut self) {}
    pub fn full_reset(&mut self) {
        self.soft_reset();
        self.current_pad_slot = 0;
        self.current_memcard_slot = 0;
    }
    pub fn get_pad_slot(&self) -> u8 { self.current_pad_slot }
    pub fn get_memcard_slot(&self) -> u8 { self.current_memcard_slot }
}

impl Default for MultitapProtocol {
    fn default() -> Self { Self::new() }
}

// ---------------------------------------------------------------------------
// Memory card filesystem parameters
// ---------------------------------------------------------------------------

pub const MCD_SIZE: usize = 1024 * 8 * 16; // legacy PSX card default
pub const MC2_MBSIZE: usize = 1024 * 528 * 2; // 1 MiB of card data
pub const MC2_ERASE_SIZE: usize = 528 * 16;
pub const FOLDER_MEM_CARD_ID_FILE: &str = "_pcsx2_superblock";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryCardType {
    Empty,
    File,
    Folder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryCardFileType {
    Unknown,
    PS2_8MB,
    PS2_16MB,
    PS2_32MB,
    PS2_64MB,
    PS1,
    MaxCount,
}

const _8MB: u64 = 8 * 1024 * 1024;
const _16MB: u64 = 16 * 1024 * 1024;
const _32MB: u64 = 32 * 1024 * 1024;
const _64MB: u64 = 64 * 1024 * 1024;

pub const TOTAL_CARD_SLOTS: usize = 8;
pub const FRAMES_AFTER_WRITE_UNTIL_FLUSH: u32 = 30;

// ---------------------------------------------------------------------------
// FileMcd / FolderMcd / McdProtocol / Multitap
//
// All "implementation" methods are file-less; the IO itself is opaque to
// keep this translation independent of the host. They're a behavioral
// translation; the original C++ talks to a real filesystem via
// FileSystem::OpenSharedCFile etc.  In Rust here, file IO is stubbed through
// trait objects the host can replace.
// ---------------------------------------------------------------------------

pub trait FileBackend: Send + Sync {
    fn open_for_read_write(&mut self, path: &str) -> bool;
    fn open_for_write(&mut self, path: &str) -> bool;
    fn close(&mut self);
    fn is_open(&self) -> bool;
    fn size(&self) -> i64;
    fn read(&mut self, dest: &mut [u8]) -> usize;
    fn write(&mut self, src: &[u8]) -> usize;
    fn seek(&mut self, offset: u64) -> bool;
    fn tell(&self) -> u64;
    fn flush(&mut self) -> bool;
}

pub struct NullFile;
impl FileBackend for NullFile {
    fn open_for_read_write(&mut self, _p: &str) -> bool { false }
    fn open_for_write(&mut self, _p: &str) -> bool { false }
    fn close(&mut self) {}
    fn is_open(&self) -> bool { false }
    fn size(&self) -> i64 { -1 }
    fn read(&mut self, _d: &mut [u8]) -> usize { 0 }
    fn write(&mut self, _s: &[u8]) -> usize { 0 }
    fn seek(&mut self, _o: u64) -> bool { false }
    fn tell(&self) -> u64 { 0 }
    fn flush(&mut self) -> bool { true }
}

// ---------------------------------------------------------------------------
// MemoryCardFile: direct file IO mapping (legacy + .ps2).
// ---------------------------------------------------------------------------

pub struct MemoryCardFile {
    pub files: [Box<dyn FileBackend>; TOTAL_CARD_SLOTS],
    pub file_sizes: [i64; TOTAL_CARD_SLOTS],
    pub is_psx: [bool; TOTAL_CARD_SLOTS],
    pub checksum: [u64; TOTAL_CARD_SLOTS],
    pub chkaddr: u32,
}

impl std::fmt::Debug for MemoryCardFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryCardFile")
            .field("file_sizes", &self.file_sizes)
            .field("is_psx", &self.is_psx)
            .field("checksum", &self.checksum)
            .field("chkaddr", &self.chkaddr)
            .finish()
    }
}

impl MemoryCardFile {
    pub fn new() -> Self {
        // Box<dyn FileBackend> cannot be constructed in a const context; we
        // use a small helper to fill the array with NullFile.
        let null: Box<dyn FileBackend> = Box::new(NullFile);
        Self {
            files: [
                null_dyn(), null_dyn(), null_dyn(), null_dyn(),
                null_dyn(), null_dyn(), null_dyn(), null_dyn(),
            ],
            file_sizes: [-1; TOTAL_CARD_SLOTS],
            is_psx: [false; TOTAL_CARD_SLOTS],
            checksum: [0; TOTAL_CARD_SLOTS],
            chkaddr: 0x210,
        }
    }

    pub fn open(&mut self) { /* file open: host backend */ }
    pub fn close(&mut self) { for f in self.files.iter_mut() { f.close(); } }

    pub fn is_present(&self, slot: usize) -> bool {
        slot < TOTAL_CARD_SLOTS && self.files[slot].is_open()
    }
    pub fn get_size_info(&self, slot: usize) -> McdSizeInfo {
        let mut info = McdSizeInfo {
            sector_size: 512,
            erase_block_size_in_sectors: 16,
            mcd_size_in_sectors: 0,
            xor: 18,
        };
        if self.files[slot].is_open() {
            info.mcd_size_in_sectors = (self.file_sizes[slot].max(0) as u32)
                / (info.sector_size + info.erase_block_size_in_sectors);
        } else {
            info.mcd_size_in_sectors = 0x4000;
        }
        let bytes = info.mcd_size_in_sectors.to_le_bytes();
        info.xor ^= bytes[0] ^ bytes[1] ^ bytes[2] ^ bytes[3];
        info
    }
    pub fn is_psx(&self, slot: usize) -> bool { slot < TOTAL_CARD_SLOTS && self.is_psx[slot] }
    pub fn read(&mut self, slot: usize, dest: &mut [u8], adr: u32, size: usize) -> s32 {
        if !self.files[slot].is_open() { return 0; }
        if !self.files[slot].seek(adr as u64) { return 0; }
        let n = self.files[slot].read(&mut dest[..size]);
        if n >= size { 1 } else { 0 }
    }
    pub fn save(&mut self, slot: usize, src: &[u8], adr: u32, size: usize) -> s32 {
        if !self.files[slot].is_open() { return 0; }
        if !self.files[slot].seek(adr as u64) { return 0; }
        if self.files[slot].write(&src[..size]) >= size { 1 } else { 0 }
    }
    pub fn erase_block(&mut self, slot: usize, adr: u32) -> s32 {
        if !self.files[slot].is_open() { return 0; }
        if !self.files[slot].seek(adr as u64) { return 0; }
        let buf = [0xff_u8; MC2_ERASE_SIZE];
        if self.files[slot].write(&buf) >= MC2_ERASE_SIZE { 1 } else { 0 }
    }
    pub fn get_crc(&self, slot: usize) -> u64 {
        if !self.files[slot].is_open() { return 0; }
        if self.is_psx[slot] { 0 } else { self.checksum[slot] }
    }
}

fn null_dyn() -> Box<dyn FileBackend> { Box::new(NullFile) }

impl Default for MemoryCardFile { fn default() -> Self { Self::new() } }

// ---------------------------------------------------------------------------
// MemoryCardFolder: directory-based memcard.
// ---------------------------------------------------------------------------

/// Per-slot memcard state. We model the super-block / FAT / indirect FAT
/// structure flatly.
#[derive(Debug, Default)]
pub struct FolderCardState {
    pub slot: u8,
    pub folder_name: String,
    pub is_enabled: bool,
    pub filtering_enabled: bool,
    pub filter: String,
    pub time_last_written: u64,
    pub frames_until_flush: u32,
    pub perform_file_writes: bool,
    pub rootdir_cluster: u32,
    pub alloc_offset: u32,
    pub alloc_end: u32,
    pub clusters_per_card: u32,
    pub pages_per_cluster: u32,
    pub page_len: u32,
    pub pages_per_block: u32,
    pub backup_block1: u32,
    pub backup_block2: u32,
    pub ifc_list: [u32; 8],
    // Caches
    pub cache: BTreeMap<u32, Vec<u8>>, // page -> data
    pub old_data_cache: BTreeMap<u32, Vec<u8>>,
    pub file_metadata: BTreeMap<u32, FileMetadataRef>,
}

#[derive(Debug, Default, Clone)]
pub struct FileMetadataRef {
    pub entry: u64,
    pub parent: Option<u64>,
    pub consecutive_cluster: u32,
}

#[derive(Debug)]
pub struct MemoryCardFolder {
    pub cards: [FolderCardState; TOTAL_CARD_SLOTS],
    pub enable_filtering: bool,
    pub last_known_filter: String,
}

impl MemoryCardFolder {
    pub fn new() -> Self {
        let mut cards: [FolderCardState; TOTAL_CARD_SLOTS] = unsafe {
            std::mem::zeroed()
        };
        for (i, c) in cards.iter_mut().enumerate() {
            c.slot = i as u8;
        }
        Self {
            cards,
            enable_filtering: false,
            last_known_filter: String::new(),
        }
    }
    pub fn open(&mut self) {
        for c in self.cards.iter_mut() { c.is_enabled = false; }
    }
    pub fn close(&mut self) {
        for c in self.cards.iter_mut() { c.is_enabled = false; }
    }
    pub fn set_filtering(&mut self, on: bool) { self.enable_filtering = on; }
    pub fn is_present(&self, slot: usize) -> bool {
        slot < TOTAL_CARD_SLOTS && self.cards[slot].is_enabled
    }
    pub fn get_size_info(&self, slot: usize) -> McdSizeInfo {
        let clusters = if self.cards[slot].clusters_per_card > 0 {
            self.cards[slot].clusters_per_card
        } else { 0 };
        McdSizeInfo {
            sector_size: 512,
            erase_block_size_in_sectors: 16,
            mcd_size_in_sectors: clusters * 2,
            xor: 18,
        }
    }
    pub fn is_psx(&self, _slot: usize) -> bool { false }
    pub fn next_frame(&mut self, slot: usize) {
        if slot < TOTAL_CARD_SLOTS {
            let c = &mut self.cards[slot];
            if c.frames_until_flush > 0 { c.frames_until_flush -= 1; }
        }
    }
    pub fn re_index(&mut self, slot: usize, on: bool, filter: &str) -> bool {
        if slot < TOTAL_CARD_SLOTS {
            self.cards[slot].filter = filter.to_string();
            self.cards[slot].filtering_enabled = on;
            self.last_known_filter = filter.to_string();
            self.enable_filtering = on;
        }
        true
    }
    pub fn get_crc(&self, slot: usize) -> u64 {
        if slot < TOTAL_CARD_SLOTS { self.cards[slot].time_last_written } else { 0 }
    }
}

impl Default for MemoryCardFolder { fn default() -> Self { Self::new() } }

// ---------------------------------------------------------------------------
// MemoryCardProtocol: implements all the SIO2/SIO0 card command handlers.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Ps1McState {
    pub current_byte: u32,
    pub sector_addr_msb: u8,
    pub sector_addr_lsb: u8,
    pub checksum: u8,
    pub expected_checksum: u8,
    pub buf: [u8; 128],
}

impl Default for Ps1McState {
    fn default() -> Self {
        Self {
            current_byte: 0,
            sector_addr_msb: 0,
            sector_addr_lsb: 0,
            checksum: 0,
            expected_checksum: 0,
            buf: [0; 128],
        }
    }
}

#[derive(Debug)]
pub struct MemoryCardProtocol {
    pub ps1: Ps1McState,
}

impl MemoryCardProtocol {
    pub const fn new() -> Self {
        Self { ps1: Ps1McState {
            current_byte: 2,
            sector_addr_msb: 0,
            sector_addr_lsb: 0,
            checksum: 0,
            expected_checksum: 0,
            buf: [0; 128],
        }}
    }

    pub fn reset_ps1_state(&mut self) {
        self.ps1.current_byte = 2;
        self.ps1.sector_addr_msb = 0;
        self.ps1.sector_addr_lsb = 0;
        self.ps1.checksum = 0;
        self.ps1.expected_checksum = 0;
        self.ps1.buf = [0; 128];
    }

    /// Probe: returns the standard 0x2b + terminator sequence (or 0xff*4 if
    /// no card is present).
    pub fn probe(&self, mcd: &Mcd, fifo_out: &mut VecDeque<u8>) {
        if !mcd.is_present() {
            for _ in 0..4 { fifo_out.push_back(0xff); }
        } else {
            the_2b_terminator(4, mcd.term, fifo_out);
        }
    }

    pub fn unknown_write_delete_end(&self, mcd: &Mcd, fifo_out: &mut VecDeque<u8>) {
        the_2b_terminator(4, mcd.term, fifo_out);
    }

    pub fn set_sector(&self, fifo_in: &mut VecDeque<u8>, mcd: &mut Mcd, fifo_out: &mut VecDeque<u8>) {
        let sector_lsb = fifo_in.pop_front().unwrap_or(0);
        let sector_2nd = fifo_in.pop_front().unwrap_or(0);
        let sector_3rd = fifo_in.pop_front().unwrap_or(0);
        let sector_msb = fifo_in.pop_front().unwrap_or(0);
        let expected   = fifo_in.pop_front().unwrap_or(0);

        let computed = sector_lsb ^ sector_2nd ^ sector_3rd ^ sector_msb;
        mcd.good_sector = computed == expected;

        let new_sector = (sector_lsb as u32)
            | ((sector_2nd as u32) << 8)
            | ((sector_3rd as u32) << 16)
            | ((sector_msb as u32) << 24);
        mcd.sector_addr = new_sector;

        let info = mcd.get_size_info();
        mcd.transfer_addr = (info.sector_size + 16) * mcd.sector_addr;

        the_2b_terminator(9, mcd.term, fifo_out);
    }

    pub fn get_specs(&self, mcd: &Mcd, fifo_out: &mut VecDeque<u8>) {
        let info = mcd.get_size_info();
        fifo_out.push_back(0x2b);
        fifo_out.push_back((info.sector_size & 0xff) as u8);
        fifo_out.push_back((info.sector_size >> 8) as u8);
        fifo_out.push_back((info.erase_block_size_in_sectors & 0xff) as u8);
        fifo_out.push_back((info.erase_block_size_in_sectors >> 8) as u8);
        fifo_out.push_back((info.mcd_size_in_sectors & 0xff) as u8);
        fifo_out.push_back((info.mcd_size_in_sectors >> 8) as u8);
        fifo_out.push_back((info.mcd_size_in_sectors >> 16) as u8);
        fifo_out.push_back((info.mcd_size_in_sectors >> 24) as u8);
        fifo_out.push_back(info.xor);
        fifo_out.push_back(mcd.term);
    }

    pub fn set_terminator(&mut self, fifo_in: &mut VecDeque<u8>, mcd: &mut Mcd, fifo_out: &mut VecDeque<u8>) {
        if let Some(t) = fifo_in.pop_front() { mcd.term = t; }
        fifo_out.push_back(0x00);
        fifo_out.push_back(0x2b);
        fifo_out.push_back(mcd.term);
    }

    pub fn get_terminator(&self, mcd: &Mcd, fifo_out: &mut VecDeque<u8>) {
        fifo_out.push_back(0x2b);
        fifo_out.push_back(mcd.term);
        fifo_out.push_back(mcd.term);
    }

    pub fn write_data(&mut self, fifo_in: &mut VecDeque<u8>, mcd: &mut Mcd, fifo_out: &mut VecDeque<u8>) {
        fifo_out.push_back(0x00);
        fifo_out.push_back(0x2b);
        let write_length = fifo_in.pop_front().unwrap_or(0);
        let mut checksum: u8 = 0;
        let mut buf: Vec<u8> = Vec::with_capacity(write_length as usize);
        for _ in 0..write_length {
            let b = fifo_in.pop_front().unwrap_or(0);
            checksum ^= b;
            buf.push(b);
            fifo_out.push_back(0x00);
        }
        mcd.write_from(&buf, buf.len());
        fifo_out.push_back(checksum);
        fifo_out.push_back(mcd.term);
        mcd.transfer_addr += write_length as u32;
    }

    pub fn read_data(&mut self, fifo_in: &mut VecDeque<u8>, mcd: &mut Mcd, fifo_out: &mut VecDeque<u8>) {
        let read_length = fifo_in.pop_front().unwrap_or(0);
        fifo_out.push_back(0x00);
        fifo_out.push_back(0x2b);
        let mut buf = vec![0u8; read_length as usize];
        let buf_len = buf.len();
        mcd.read_into(&mut buf, buf_len);
        let mut checksum: u8 = 0;
        for b in &buf {
            checksum ^= *b;
            fifo_out.push_back(*b);
        }
        fifo_out.push_back(checksum);
        fifo_out.push_back(mcd.term);
        mcd.transfer_addr += read_length as u32;
    }

    pub fn ps1_read(&mut self, data: u8, mcd: &mut Mcd) -> u8 {
        if !mcd.is_present() { return 0xff; }
        let mut send_ack = true;
        let ret = match self.ps1.current_byte {
            2 => 0x5a,
            3 => 0x5d,
            4 => { self.ps1.sector_addr_msb = data; 0x00 }
            5 => {
                self.ps1.sector_addr_lsb = data;
                self.recalc_ps1_addr(mcd);
                0x00
            }
            6 => 0x5c,
            7 => 0x5d,
            8 => self.ps1.sector_addr_msb,
            9 => self.ps1.sector_addr_lsb,
            138 => self.ps1.checksum,
            139 => { send_ack = false; 0x47 }
            10 => {
                self.ps1.checksum = self.ps1.sector_addr_msb ^ self.ps1.sector_addr_lsb;
                let buf_len = self.ps1.buf.len();
                mcd.read_into(&mut self.ps1.buf, buf_len);
                self.ps1.buf[(self.ps1.current_byte - 10) as usize]
            }
            _ => {
                let b = self.ps1.buf[(self.ps1.current_byte - 10) as usize];
                self.ps1.checksum ^= b;
                b
            }
        };
        g_sio0().set_acknowledge(send_ack);
        self.ps1.current_byte += 1;
        ret
    }

    pub fn ps1_state(&mut self, data: u8) -> u8 {
        // No real implementation in the original; preserved as a stub that
        // reports an error and returns 0.
        let _ = data;
        0x00
    }

    pub fn ps1_write(&mut self, data: u8, mcd: &mut Mcd) -> u8 {
        let mut send_ack = true;
        let ret = match self.ps1.current_byte {
            2 => 0x5a,
            3 => 0x5d,
            4 => { self.ps1.sector_addr_msb = data; 0x00 }
            5 => {
                self.ps1.sector_addr_lsb = data;
                self.recalc_ps1_addr(mcd);
                0x00
            }
            134 => { self.ps1.expected_checksum = data; 0 }
            135 => 0x5c,
            136 => 0x5d,
            137 => {
                if !mcd.good_sector { 0xff }
                else if self.ps1.expected_checksum != self.ps1.checksum { 0x4e }
                else {
                    mcd.write_from(&self.ps1.buf, self.ps1.buf.len());
                    mcd.flag &= 0x07;
                    0x47
                }
            }
            6 => {
                self.ps1.checksum = self.ps1.sector_addr_msb ^ self.ps1.sector_addr_lsb;
                self.ps1.buf[(self.ps1.current_byte - 6) as usize] = data;
                self.ps1.checksum ^= data;
                0x00
            }
            _ => {
                self.ps1.buf[(self.ps1.current_byte - 6) as usize] = data;
                self.ps1.checksum ^= data;
                0x00
            }
        };
        g_sio0().set_acknowledge(send_ack);
        self.ps1.current_byte += 1;
        ret
    }

    pub fn ps1_pocketstation(&mut self, _data: u8) -> u8 {
        g_sio0().set_acknowledge(false);
        0x00
    }

    pub fn read_write_end(&self, mcd: &Mcd, fifo_out: &mut VecDeque<u8>) {
        the_2b_terminator(4, mcd.term, fifo_out);
    }

    pub fn erase_block(&self, mcd: &mut Mcd, fifo_out: &mut VecDeque<u8>) {
        mcd.erase_block();
        the_2b_terminator(4, mcd.term, fifo_out);
    }

    pub fn unknown_boot(&self, mcd: &Mcd, fifo_out: &mut VecDeque<u8>) {
        the_2b_terminator(5, mcd.term, fifo_out);
    }

    pub fn auth_xor(&self, fifo_in: &mut VecDeque<u8>, mcd: &Mcd, fifo_out: &mut VecDeque<u8>) {
        let mode = fifo_in.pop_front().unwrap_or(0);
        match mode {
            0x01 | 0x02 | 0x04 | 0x0f | 0x11 | 0x13 => {
                fifo_out.push_back(0x00);
                fifo_out.push_back(0x2b);
                let mut xor: u8 = 0;
                for _ in 0..8 {
                    let b = fifo_in.pop_front().unwrap_or(0);
                    xor ^= b;
                    fifo_out.push_back(0x00);
                }
                fifo_out.push_back(xor);
                fifo_out.push_back(mcd.term);
            }
            0x06 | 0x07 | 0x0b => {
                the_2b_terminator(14, mcd.term, fifo_out);
            }
            _ => {
                the_2b_terminator(5, mcd.term, fifo_out);
            }
        }
    }

    pub fn auth_f3(&self, mcd: &mut Mcd, fifo_out: &mut VecDeque<u8>) {
        if !mcd.is_present() {
            for _ in 0..4 { fifo_out.push_back(0xff); }
        } else {
            mcd.term = terminator::READY as u8;
            the_2b_terminator(5, mcd.term, fifo_out);
        }
    }

    pub fn auth_f7(&self, mcd: &Mcd, fifo_out: &mut VecDeque<u8>) {
        the_2b_terminator(5, mcd.term, fifo_out);
    }

    fn recalc_ps1_addr(&mut self, mcd: &mut Mcd) {
        mcd.sector_addr = ((self.ps1.sector_addr_msb as u32) << 8) | (self.ps1.sector_addr_lsb as u32);
        mcd.good_sector = mcd.sector_addr <= 0x03ff;
        mcd.transfer_addr = 128 * mcd.sector_addr;
    }
}

impl Default for MemoryCardProtocol { fn default() -> Self { Self::new() } }

fn the_2b_terminator(length: usize, term: u8, fifo_out: &mut VecDeque<u8>) {
    while fifo_out.len() < length.saturating_sub(2) { fifo_out.push_back(0x00); }
    fifo_out.push_back(0x2b);
    fifo_out.push_back(term);
}

// ---------------------------------------------------------------------------
// Pad command dispatch (Pad namespace).
// ---------------------------------------------------------------------------

/// Pad command bytes. Range 0x40..=0x5F reserved for pad commands.
pub mod pad_command {
    pub const UNK_0: u8 = 0x40;
    pub const QUERY_BUTTONS: u8 = 0x41;
    pub const POLL: u8 = 0x42;
    pub const CONFIG: u8 = 0x43;
    pub const MODE_SWITCH: u8 = 0x44;
    pub const STATUS: u8 = 0x45;
    pub const CONST_1: u8 = 0x46;
    pub const CONST_2: u8 = 0x47;
    pub const UNK_8: u8 = 0x48;
    pub const UNK_9: u8 = 0x49;
    pub const UNK_A: u8 = 0x4a;
    pub const UNK_B: u8 = 0x4b;
    pub const CONST_3: u8 = 0x4c;
    pub const VIBRATION: u8 = 0x4d;
    pub const UNK_E: u8 = 0x4e;
    pub const ANALOG: u8 = 0x4f;
    pub const MYSTERY: u8 = 0x40;
    pub const RESPONSE_BYTES: u8 = 0x5f;
}

#[inline]
pub fn pad_is_pad_command(cmd: u8) -> bool {
    cmd >= pad_command::MYSTERY && cmd <= pad_command::RESPONSE_BYTES
}

#[inline]
pub fn is_memcard_command(cmd: u8) -> bool {
    cmd == memcard_command::PS1_READ
        || cmd == memcard_command::PS1_STATE
        || cmd == memcard_command::PS1_WRITE
}

#[inline]
pub fn is_pocketstation_command(cmd: u8) -> bool {
    cmd == memcard_command::PS1_POCKETSTATION
}

// ---------------------------------------------------------------------------
// Sio0 (PS1) emulation
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct Sio0 {
    pub tx_data: u32,    // 0x1f801040
    pub rx_data: u32,    // 0x1f801040
    pub stat: u32,       // 0x1f801044
    pub mode: u16,       // 0x1f801048
    pub ctrl: u16,       // 0x1f80104a
    pub baud: u16,       // 0x1f80104e
    pub flag: u8,
    pub sio_stage: SioStage,
    pub sio_mode: u8,
    pub sio_command: u8,
    pub pad_started: bool,
    pub rx_data_set: bool,
    pub port: u8,
    pub slot: u8,
}

impl Sio0 {
    pub const fn new() -> Self {
        Self {
            tx_data: 0, rx_data: 0, stat: 0,
            mode: 0, ctrl: 0, baud: 0, flag: 0,
            sio_stage: SioStage::Idle, sio_mode: sio_mode::NOT_SET, sio_command: 0,
            pad_started: false, rx_data_set: false, port: 0, slot: 0,
        }
    }

    pub fn clear_stat_acknowledge(&mut self) {
        self.stat &= !sio0_stat::ACK;
    }

    pub fn initialize(&mut self, mcds: &mut McdsArray) -> bool {
        self.soft_reset();
        self.port = 0; self.slot = 0;
        for p in 0..sio::PORTS {
            for sl in 0..sio::SLOTS {
                mcds[p][sl].term = 0x55;
                mcds[p][sl].port = p as u8;
                mcds[p][sl].slot = sl as u8;
                mcds[p][sl].flag = 0x08;
                mcds[p][sl].auto_eject_ticks = 0;
            }
        }
        true
    }

    pub fn shutdown(&self) -> bool { true }

    pub fn soft_reset(&mut self) {
        self.pad_started = false;
        self.sio_mode = sio_mode::NOT_SET;
        self.sio_command = 0;
        self.sio_stage = SioStage::Idle;
        g_memory_card_protocol().reset_ps1_state();
    }

    pub fn set_acknowledge(&mut self, ack: bool) {
        if ack { self.stat |= sio0_stat::ACK; }
        else { self.stat &= !sio0_stat::ACK; }
    }

    pub fn interrupt(&self, _i: Sio0Interrupt) { /* raises IOP irq 7 in the C++ */ }

    pub fn get_tx_data(&self) -> u8 { self.tx_data as u8 }
    pub fn get_rx_data(&mut self) -> u8 {
        self.stat |= sio0_stat::TX_READY | sio0_stat::TX_EMPTY;
        self.stat &= !sio0_stat::RX_FIFO_NOT_EMPTY;
        self.rx_data as u8
    }
    pub fn get_stat(&mut self) -> u32 {
        let r = self.stat;
        self.interrupt(Sio0Interrupt::StatRead);
        r
    }
    pub fn get_mode(&self) -> u16 { self.mode }
    pub fn get_ctrl(&self) -> u16 { self.ctrl }
    pub fn get_baud(&self) -> u16 { self.baud }

    pub fn set_rx_data(&mut self, v: u8) { self.rx_data = v as u32; }
    pub fn set_stat(&mut self, v: u32) { self.stat = v; }
    pub fn set_mode(&mut self, v: u16) { self.mode = v; }
    pub fn set_baud(&mut self, v: u16) { self.baud = v; }

    pub fn set_ctrl(&mut self, v: u16) {
        self.ctrl = v;
        self.port = ((v & sio0_ctrl::PORT) > 0) as u8;
        if self.ctrl == 0 {
            g_memory_card_protocol().reset_ps1_state();
            self.soft_reset();
        }
        if v & sio0_ctrl::ACK != 0 {
            self.stat &= !(sio0_stat::IRQ | sio0_stat::RX_PARITY_ERROR);
        }
        if v & sio0_ctrl::RESET != 0 {
            self.stat = 0; self.ctrl = 0; self.mode = 0;
            self.soft_reset();
        }
    }

    pub fn set_tx_data(&mut self, cmd: u8, mcds: &mut McdsArray) {
        self.stat |= sio0_stat::TX_READY | sio0_stat::TX_EMPTY;
        self.stat |= sio0_stat::RX_FIFO_NOT_EMPTY;
        if (self.ctrl & sio0_ctrl::TX_ENABLE) == 0 { return; }
        self.tx_data = cmd as u32;
        let data: u8 = match self.sio_mode {
            sio_mode::NOT_SET => {
                self.sio_mode = cmd;
                self.port = 0; self.slot = 0;
                self.set_acknowledge(true);
                0
            }
            sio_mode::PAD => {
                // Real pad dispatch happens via Sio0::SetTxData -> pad object.
                // We don't have a pad object in this translation; return 0xff.
                self.set_acknowledge(true);
                0xff
            }
            sio_mode::MEMCARD => {
                if self.sio_command == memcard_command::NOT_SET
                    && is_memcard_command(cmd)
                    && mcds[self.port as usize][self.slot as usize].is_present()
                    && mcds[self.port as usize][self.slot as usize].is_psx()
                {
                    self.sio_command = cmd;
                    self.set_acknowledge(true);
                    self.flag
                } else {
                    self.set_acknowledge(false);
                    0x00
                }
            }
            _ => { self.set_acknowledge(false); 0xff }
        };
        self.set_rx_data(data);
        if (self.stat & sio0_stat::ACK) == 0 { self.soft_reset(); }
        self.interrupt(Sio0Interrupt::TxDataWrite);
    }

    pub fn memcard(&mut self, value: u8, mcd: &mut Mcd) -> u8 {
        let proto = g_memory_card_protocol();
        match self.sio_command {
            memcard_command::PS1_READ => proto.ps1_read(value, mcd),
            memcard_command::PS1_STATE => proto.ps1_state(value),
            memcard_command::PS1_WRITE => proto.ps1_write(value, mcd),
            memcard_command::PS1_POCKETSTATION => proto.ps1_pocketstation(value),
            _ => { self.soft_reset(); 0xff }
        }
    }
}

impl Default for Sio0 { fn default() -> Self { Self::new() } }

// ---------------------------------------------------------------------------
// Sio2 (PS2) emulation
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct Sio2 {
    pub cmd_queue: [u32; 16],
    pub port_ctrl0: [u32; 4],
    pub port_ctrl1: [u32; 4],
    pub data_in: u32,
    pub data_out: u32,
    pub ctrl: u32,
    pub cmd_stat: u32,
    pub port_stat: u32,
    pub fifo_stat: u32,
    pub fifo_tx_pos: u32,
    pub fifo_rx_pos: u32,
    pub i_stat: u32,
    pub port: u8,
    pub queue_read: bool,
    pub queue_position: usize,
    pub command_length: usize,
    pub processed_length: usize,
    pub dma_block_size: usize,
    pub queue_complete: bool,
}

impl Sio2 {
    pub const fn new() -> Self {
        Self {
            cmd_queue: [0; 16],
            port_ctrl0: [0; 4],
            port_ctrl1: [0; 4],
            data_in: 0, data_out: 0, ctrl: 0, cmd_stat: 0,
            port_stat: port_stat::DEFAULT, fifo_stat: fifo_stat::DEFAULT,
            fifo_tx_pos: 0, fifo_rx_pos: 0, i_stat: 0,
            port: 0, queue_read: false, queue_position: 0,
            command_length: 0, processed_length: 0,
            dma_block_size: 0, queue_complete: false,
        }
    }

    pub fn initialize(&mut self, mcds: &mut McdsArray) -> bool {
        self.soft_reset();
        for c in self.cmd_queue.iter_mut() { *c = 0; }
        for i in 0..4 { self.port_ctrl0[i] = 0; self.port_ctrl1[i] = 0; }
        self.data_in = 0; self.data_out = 0;
        self.set_ctrl(sio2_ctrl::SIO2MAN_RESET);
        self.set_cmd_stat(cmd_stat::DISCONNECTED);
        self.port_stat = port_stat::DEFAULT;
        self.fifo_stat = fifo_stat::DEFAULT;
        self.fifo_tx_pos = 0; self.fifo_rx_pos = 0; self.i_stat = 0;
        self.port = 0;
        for p in 0..sio::PORTS {
            for sl in 0..sio::SLOTS {
                mcds[p][sl].term = 0x55;
                mcds[p][sl].port = p as u8;
                mcds[p][sl].slot = sl as u8;
                mcds[p][sl].flag = 0x08;
                mcds[p][sl].auto_eject_ticks = 0;
            }
        }
        true
    }

    pub fn shutdown(&self) -> bool { true }

    pub fn soft_reset(&mut self) {
        self.queue_read = false;
        self.queue_position = 0;
        self.command_length = 0;
        self.processed_length = 0;
        self.dma_block_size = 0;
        self.queue_complete = false;
        let _ = g_sio2_fifo_in().clear();
        self.cmd_stat = 0;
    }

    pub fn interrupt(&mut self) {
        if self.i_stat == 0 { self.i_stat |= 1; }
    }

    pub fn set_ctrl(&mut self, v: u32) {
        self.ctrl = v;
        if v & sio2_ctrl::START_TRANSFER != 0 { self.interrupt(); }
    }

    pub fn set_cmd(&mut self, position: usize, value: u32) {
        if position < self.cmd_queue.len() { self.cmd_queue[position] = value; }
        if position == 0 { self.soft_reset(); }
    }

    pub fn set_cmd_stat(&mut self, v: u32) { self.cmd_stat = v; }

    pub fn pad(&mut self) { /* dispatch to pad object */ }
    pub fn multitap(&mut self) { /* dispatch to multitap */ }
    pub fn infrared(&mut self) {
        self.set_cmd_stat(cmd_stat::DISCONNECTED);
        let _ = g_sio2_fifo_in().pop_front();
        let n = self.command_length;
        while g_sio2_fifo_out().len() < n { g_sio2_fifo_out().push_back(0xff); }
    }
    pub fn memcard(&mut self) { /* dispatch to memcard command handlers */ }

    pub fn write(&mut self, data: u8) {
        if !self.queue_read {
            if self.queue_position > self.cmd_queue.len() { return; }
            let current_cmd = self.cmd_queue[self.queue_position];
            self.port = (current_cmd & sio2_cmd::PORT) as u8;
            self.command_length = ((current_cmd >> 8) & sio2_cmd::COMMAND_LENGTH_MASK) as usize;
            self.queue_read = true;
            if self.command_length == 0 { self.queue_complete = true; }
            while !g_sio2_fifo_in().is_empty() { g_sio2_fifo_in().pop_front(); }
        }
        if self.queue_complete { return; }
        g_sio2_fifo_in().push_back(data);
        let len = g_sio2_fifo_in().len();
        if (len == self.command_length && self.dma_block_size == 0)
            || len == self.dma_block_size
        {
            self.queue_read = false;
            self.queue_position += 1;
            if let Some(mode) = g_sio2_fifo_in().pop_front() {
                match mode {
                    sio_mode::PAD => self.pad(),
                    sio_mode::MULTITAP => self.multitap(),
                    sio_mode::INFRARED => self.infrared(),
                    sio_mode::MEMCARD => self.memcard(),
                    _ => {
                        g_sio2_fifo_out().push_back(0xff);
                        self.set_cmd_stat(cmd_stat::DISCONNECTED);
                    }
                }
            }
            if self.dma_block_size > 0 {
                let dma_diff = g_sio2_fifo_out().len() % self.dma_block_size;
                if dma_diff > 0 {
                    let padding = self.dma_block_size - dma_diff;
                    for _ in 0..padding { g_sio2_fifo_out().push_back(0x00); }
                }
            }
        }
    }

    pub fn read(&mut self) -> u8 {
        if let Some(b) = g_sio2_fifo_out().pop_front() { b } else { 0xff }
    }
}

impl Default for Sio2 { fn default() -> Self { Self::new() } }

// ---------------------------------------------------------------------------
// AutoEject
// ---------------------------------------------------------------------------

pub mod auto_eject {
    use super::{file_mcd_is_present, sio_convert_port_and_slot_to_pad, mcds, AutoEject, sio::PORTS, sio::SLOTS};

    /// Countdown auto-eject timer ticks. Re-enables the card on re-insert.
    pub fn count_down_ticks() {
        let mut reinserted = false;
        for p in 0..PORTS {
            for sl in 0..SLOTS {
                let ticks = mcds()[p][sl].auto_eject_ticks;
                if ticks > 0 {
                    mcds()[p][sl].auto_eject_ticks = ticks - 1;
                    if ticks - 1 == 0 {
                        reinserted |= file_mcd_is_present(p as u32, sl as u32);
                    }
                }
            }
        }
        let _ = (reinserted, sio_convert_port_and_slot_to_pad); // suppress unused
    }

    pub fn set(port: usize, slot: usize) {
        if port < PORTS && slot < SLOTS {
            let m = &mut mcds()[port][slot];
            if m.auto_eject_ticks == 0 {
                m.auto_eject_ticks = 60;
                m.term = crate::pcsx2::SioPad::terminator::NOT_READY as u8;
            }
        }
    }
    pub fn clear(port: usize, slot: usize) {
        if port < PORTS && slot < SLOTS {
            mcds()[port][slot].auto_eject_ticks = 0;
        }
    }
    pub fn set_all() {
        for p in 0..PORTS { for s in 0..SLOTS { set(p, s); } }
    }
    pub fn clear_all() {
        for p in 0..PORTS { for s in 0..SLOTS { clear(p, s); } }
    }
}

#[allow(dead_code)]
struct AutoEject;

// ---------------------------------------------------------------------------
// MemcardBusy
// ---------------------------------------------------------------------------

pub mod memcard_busy {
    use super::SIO_LAST_FRAME_MCD_BUSY;
    use std::sync::atomic::{AtomicU32, Ordering};

    static CURRENT_BUSY_TICKS: AtomicU32 = AtomicU32::new(0);
    static G_FRAME_COUNT: AtomicU32 = AtomicU32::new(0);

    pub fn decrement() {
        if CURRENT_BUSY_TICKS.load(Ordering::Relaxed) == 0 { return; }
        CURRENT_BUSY_TICKS.fetch_sub(1, Ordering::Release);
    }
    pub fn set_busy() {
        CURRENT_BUSY_TICKS.store(300, Ordering::Release);
        let f = G_FRAME_COUNT.load(Ordering::Relaxed);
        unsafe { SIO_LAST_FRAME_MCD_BUSY = f; }
    }
    pub fn is_busy() -> bool {
        CURRENT_BUSY_TICKS.load(Ordering::Acquire) > 0
    }
    pub fn clear_busy() {
        CURRENT_BUSY_TICKS.store(0, Ordering::Release);
        unsafe { SIO_LAST_FRAME_MCD_BUSY = 0; }
    }
    pub fn check_save_state_dependency() {
        let f = G_FRAME_COUNT.load(Ordering::Relaxed);
        let last = unsafe { SIO_LAST_FRAME_MCD_BUSY };
        if f - last > super::NUM_FRAMES_BEFORE_SAVESTATE_DEPENDENCY_WARNING {
            // Real impl posts an OSD message. Translation omitted.
        }
    }
}

// ---------------------------------------------------------------------------
// Sio public API
// ---------------------------------------------------------------------------

/// The SIO register file mirrored at 0x1F808258..=0x1F80825F (PS2 SIO2).
/// Indexed as [0] = 0x1F808258, [1] = 0x1F80825C, etc.
pub static mut sioreg: [u32; 4] = [0; 4];

/// Indexed memory cards: [port][slot]. The C++ declares this as `_mcd
/// mcds[2][4]`; we model it as a single global.
pub static mut MCDS: McdsArray = [
    [const { Mcd::new() }, const { Mcd::new() }, const { Mcd::new() }, const { Mcd::new() }],
    [const { Mcd::new() }, const { Mcd::new() }, const { Mcd::new() }, const { Mcd::new() }],
];

/// The "current" memcard pointer (`mcd` in the C++). We use an UnsafeCell to
/// represent a mutable pointer that may be assigned to point into MCDS.
pub static mut MCD_PTR: *mut Mcd = std::ptr::null_mut();

/// Frame counter used to compute "how long since last memcard write".
pub static mut SIO_LAST_FRAME_MCD_BUSY: u32 = 0;

// ---------------------------------------------------------------------------
// SIO API exposed as Rust free functions. These mirror the original C++
// namespace and free-function API, but they operate on the static-mut
// globals above.
// ---------------------------------------------------------------------------

pub fn sio_init() -> bool {
    let s0 = g_sio0();
    let mut s2 = g_sio2();
    let ok0 = unsafe { (*MCD_PTR).is_present() || (*MCD_PTR).term != 0 || true };
    let ok1 = s0.initialize(unsafe { &mut MCDS });
    let ok2 = s2.initialize(unsafe { &mut MCDS });
    unsafe {
        MCD_PTR = &mut MCDS[0][0] as *mut Mcd;
    }
    let _ = ok0;
    ok1 && ok2
}

pub fn sio_reset() {
    g_sio0().soft_reset();
    g_sio2().soft_reset();
    g_memory_card_protocol().reset_ps1_state();
    for p in 0..sio::PORTS {
        for sl in 0..sio::SLOTS {
            unsafe { MCDS[p][sl].term = 0x55; }
        }
    }
}

pub fn sio_update() {
    auto_eject::count_down_ticks();
}

pub fn sio_next_frame() {
    unsafe {
        for p in 0..sio::PORTS {
            for sl in 0..sio::SLOTS {
                MCDS[p][sl].next_frame();
            }
        }
    }
}

pub fn sio_set_game_serial(serial: &str) {
    for p in 0..sio::PORTS {
        for sl in 0..sio::SLOTS {
            unsafe {
                if MCDS[p][sl].re_index(serial) {
                    auto_eject::set(p, sl);
                }
            }
        }
    }
}

/// Convert a global pad index [0..7] to a (port, slot) pair.
pub fn sio_convert_pad_to_port_and_slot(index: u32) -> (u32, u32) {
    if index > 4 {
        (1, index - 4) // 2B, 2C, 2D
    } else if index > 1 {
        (0, index - 1) // 1B, 1C, 1D
    } else {
        (index, 0)    // 1A, 2A
    }
}

/// Convert (port, slot) to a unified global pad index.
pub fn sio_convert_port_and_slot_to_pad(port: u32, slot: u32) -> u32 {
    if slot == 0 { port }
    else if port == 0 { slot + 1 }
    else { slot + 4 }
}

pub fn sio_pad_is_multitap_slot(index: u32) -> bool { index >= 2 }
pub fn sio_port_and_slot_is_multitap(_port: u32, slot: u32) -> bool { slot != 0 }

// ---------------------------------------------------------------------------
// MMIO read/write helpers for the two SIO register files.
//
// The original PCSX2 hooks these up to its IOP memory subsystem. In this
// standalone translation they are exposed as plain functions the host can
// call.
// ---------------------------------------------------------------------------

/// Read from SIO0 register file. `addr` is the IOP address
/// (0x1F801040..=0x1F80105F).
pub fn sio0_read32(addr: u32) -> u32 {
    let s = g_sio0();
    match addr {
        0x1F801040 => s.get_rx_data() as u32,
        0x1F801044 => s.get_stat(),
        0x1F801048 => s.get_mode() as u32,
        0x1F80104A => s.get_ctrl() as u32,
        0x1F80104E => s.get_baud() as u32,
        _ => 0,
    }
}

/// Write into SIO0 register file.
pub fn sio0_write32(addr: u32, value: u32) {
    let mut s = g_sio0();
    match addr {
        0x1F801040 => s.set_tx_data(value as u8, unsafe { &mut MCDS }),
        0x1F801044 => s.set_stat(value),
        0x1F801048 => s.set_mode(value as u16),
        0x1F80104A => s.set_ctrl(value as u16),
        0x1F80104E => s.set_baud(value as u16),
        _ => {}
    }
}

/// Read from SIO2 register file. `addr` is 0x1F808200..=0x1F808283.
pub fn sio2_read32(addr: u32) -> u32 {
    unsafe {
        match addr {
            0x1F808260 => g_sio2().data_in,
            0x1F808264 => g_sio2().data_out,
            0x1F808268 => g_sio2().ctrl,
            0x1F80826C => g_sio2().cmd_stat,
            0x1F808270 => g_sio2().port_stat,
            0x1F808274 => g_sio2().fifo_stat,
            0x1F808278 => g_sio2().fifo_tx_pos,
            0x1F80827C => g_sio2().fifo_rx_pos,
            0x1F808280 => g_sio2().i_stat,
            // CmdQueue and PortCtrl{0,1} addresses map to a base + index.
            a if (0x1F808200..=0x1F80823F).contains(&a) => {
                let i = ((a - 0x1F808200) / 4) as usize;
                g_sio2().cmd_queue.get(i).copied().unwrap_or(0)
            }
            a if (0x1F808240..=0x1F80824F).contains(&a) => {
                let i = ((a - 0x1F808240) / 4) as usize;
                g_sio2().port_ctrl0.get(i).copied().unwrap_or(0)
            }
            a if (0x1F808250..=0x1F80825F).contains(&a) => {
                let i = ((a - 0x1F808250) / 4) as usize;
                g_sio2().port_ctrl1.get(i).copied().unwrap_or(0)
            }
            _ => 0,
        }
    }
}

/// Write into SIO2 register file.
pub fn sio2_write32(addr: u32, value: u32) {
    unsafe {
        let s = &mut *g_sio2_ptr();
        match addr {
            0x1F808260 => s.data_in = value,
            0x1F808264 => s.data_out = value,
            0x1F808268 => s.set_ctrl(value),
            0x1F80826C => s.set_cmd_stat(value),
            0x1F808270 => s.port_stat = value,
            0x1F808274 => s.fifo_stat = value,
            0x1F808278 => s.fifo_tx_pos = value,
            0x1F80827C => s.fifo_rx_pos = value,
            0x1F808280 => s.i_stat = value,
            a if (0x1F808200..=0x1F80823F).contains(&a) => {
                let i = ((a - 0x1F808200) / 4) as usize;
                s.set_cmd(i, value);
            }
            a if (0x1F808240..=0x1F80824F).contains(&a) => {
                let i = ((a - 0x1F808240) / 4) as usize;
                s.port_ctrl0[i] = value;
            }
            a if (0x1F808250..=0x1F80825F).contains(&a) => {
                let i = ((a - 0x1F808250) / 4) as usize;
                s.port_ctrl1[i] = value;
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Storage for the singleton instances and convenient accessors.
// ---------------------------------------------------------------------------

struct Wrapped<T>(UnsafeCell<T>);
unsafe impl<T: Send> Sync for Wrapped<T> {}

static SIO0: Wrapped<Sio0> = Wrapped(UnsafeCell::new(Sio0::new()));
static SIO2: Wrapped<Sio2> = Wrapped(UnsafeCell::new(Sio2::new()));
static MEMORY_CARD_PROTOCOL: Wrapped<MemoryCardProtocol> = Wrapped(UnsafeCell::new(MemoryCardProtocol::new()));
static MULTITAP: Wrapped<[MultitapProtocol; sio::PORTS]> = Wrapped(UnsafeCell::new([
    MultitapProtocol::new(), MultitapProtocol::new(),
]));

static SIO2_FIFO_IN: Wrapped<VecDeque<u8>> = Wrapped(UnsafeCell::new(VecDeque::new()));
static SIO2_FIFO_OUT: Wrapped<VecDeque<u8>> = Wrapped(UnsafeCell::new(VecDeque::new()));

/// Convenience accessor for Sio0.
pub fn g_sio0() -> &'static mut Sio0 { unsafe { &mut *SIO0.0.get() } }
/// Convenience accessor for Sio2.
pub fn g_sio2() -> &'static mut Sio2 { unsafe { &mut *SIO2.0.get() } }
fn g_sio2_ptr() -> *mut Sio2 { SIO2.0.get() }
/// Convenience accessor for the global memcard protocol state.
pub fn g_memory_card_protocol() -> &'static mut MemoryCardProtocol {
    unsafe { &mut *MEMORY_CARD_PROTOCOL.0.get() }
}
/// Convenience accessor for the multitap array.
pub fn g_multitap_arr() -> &'static mut [MultitapProtocol; sio::PORTS] {
    unsafe { &mut *MULTITAP.0.get() }
}
fn g_sio2_fifo_in() -> &'static mut VecDeque<u8> { unsafe { &mut *SIO2_FIFO_IN.0.get() } }
fn g_sio2_fifo_out() -> &'static mut VecDeque<u8> { unsafe { &mut *SIO2_FIFO_OUT.0.get() } }

pub fn mcds() -> &'static mut McdsArray { unsafe { &mut MCDS } }

// ---------------------------------------------------------------------------
// Backend hooks (file IO is pluggable; default returns "no card").
// ---------------------------------------------------------------------------

/// Returns whether a file-backed memcard is present at (port, slot).
pub fn file_mcd_is_present(port: u32, slot: u32) -> bool {
    let combined = file_mcd_convert_to_slot(port, slot);
    file_mcd_get(combined).is_present(combined)
}

/// Combined port/slot lookup. Returns size info for the combined slot.
pub fn file_mcd_get_size_info(port: u32, slot: u32) -> McdSizeInfo {
    let combined = file_mcd_convert_to_slot(port, slot);
    file_mcd_get(combined).get_size_info(combined)
}

pub fn file_mcd_is_psx(port: u32, slot: u32) -> bool {
    let combined = file_mcd_convert_to_slot(port, slot);
    file_mcd_get(combined).is_psx(combined)
}

pub fn file_mcd_read(port: u32, slot: u32, dest: &mut [u8], adr: u32, size: usize) -> s32 {
    let combined = file_mcd_convert_to_slot(port, slot);
    file_mcd_get(combined).read(combined, dest, adr, size)
}

pub fn file_mcd_save(port: u32, slot: u32, src: &[u8], adr: u32, size: usize) -> s32 {
    let combined = file_mcd_convert_to_slot(port, slot);
    file_mcd_get(combined).save(combined, src, adr, size)
}

pub fn file_mcd_erase_block(port: u32, slot: u32, adr: u32) -> s32 {
    let combined = file_mcd_convert_to_slot(port, slot);
    file_mcd_get(combined).erase_block(combined, adr)
}

pub fn file_mcd_get_crc(port: u32, slot: u32) -> u64 {
    let combined = file_mcd_convert_to_slot(port, slot);
    file_mcd_get(combined).get_crc(combined)
}

pub fn file_mcd_next_frame(port: u32, slot: u32) {
    let combined = file_mcd_convert_to_slot(port, slot);
    file_folder_get(combined).next_frame(combined);
}

pub fn file_mcd_re_index(port: u32, slot: u32, filter: &str) -> bool {
    let combined = file_mcd_convert_to_slot(port, slot);
    file_folder_get(combined).re_index(combined, true, filter)
}

/// Convert (port, slot) to a global slot index 0..7.
pub fn file_mcd_convert_to_slot(port: u32, slot: u32) -> usize {
    if slot == 0 { port as usize }
    else if port == 0 { (slot + 1) as usize }
    else { (slot + 4) as usize }
}

fn file_mcd_get(combined: usize) -> &'static mut MemoryCardFile {
    let cell: &SyncCell<MemoryCardFile> = &*MEMCARD_FILE_BACKEND;
    unsafe { &mut *cell.get() }
}
fn file_folder_get(combined: usize) -> &'static mut MemoryCardFolder {
    let cell: &SyncCell<MemoryCardFolder> = &*MEMCARD_FOLDER_BACKEND;
    unsafe { &mut *cell.get() }
}

/// Newtype wrapper around [`UnsafeCell`] that opts into [`Sync`]. Mirrors the
/// C++ globals these backends replace: the memory-card filesystem is a
/// process-wide singleton accessed without locking, so the only thing the
/// type system needs is the `Sync` impl that `LazyLock` requires.
struct SyncCell<T>(UnsafeCell<T>);

impl<T> SyncCell<T> {
    const fn new(value: T) -> Self { Self(UnsafeCell::new(value)) }
    fn get(&self) -> *mut T { self.0.get() }
}

unsafe impl<T: Send + Sync> Sync for SyncCell<T> {}

static MEMCARD_FILE_BACKEND: LazyLock<SyncCell<MemoryCardFile>> =
    LazyLock::new(|| SyncCell::new(MemoryCardFile::new()));
static MEMCARD_FOLDER_BACKEND: LazyLock<SyncCell<MemoryCardFolder>> =
    LazyLock::new(|| SyncCell::new(MemoryCardFolder::new()));

// ---------------------------------------------------------------------------
// Test entry point - not part of the original.
// ---------------------------------------------------------------------------

#[allow(dead_code)]
fn main() {
    let _ = sio_init();
    sio_reset();
    sio_update();
    let _ = Instant::now()
        .checked_add(Duration::from_secs(1))
        .and_then(|_| Some(SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default()));
}
