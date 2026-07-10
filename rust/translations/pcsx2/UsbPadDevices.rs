// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust 2021 translation of the PCSX2 USB pad device sources:
//! `usb-buzz.cpp`, `usb-gametrak.cpp`, `usb-pad-ff.cpp`, `usb-pad-sdl-ff.cpp`,
//! `usb-realplay.cpp`, `usb-seamic.cpp`, `usb-train.cpp`,
//! `usb-trance-vibrator.cpp`, `usb-turntable.cpp` and `lg/lg_ff.cpp`.
//!
//! The module exposes one public struct per device (`UsbBuzz`, `UsbGametrak`,
//! `UsbPadFF`, `UsbPadSdlFF`, `UsbRealPlay`, `UsbSeamic`, `UsbTrain`,
//! `UsbTranceVibrator`, `UsbTurntable`, `LgFF`). Each device provides the
//! lifecycle methods `init()`, `shutdown()`, `update()` together with the
//! `USBPad::Update` style methods. State, binding helpers and the on-wire
//! protocol constants from the original C++ sources are kept as closely as
//! possible so that the original semantics around hat-switch encoding, axis
//! clamping and force-feedback coefficient conversion are preserved.

#![allow(dead_code)]
#![allow(non_snake_case)]

use std::cmp::{max, min};

// ---------------------------------------------------------------------------
// Shared enums / constants
// ---------------------------------------------------------------------------

/// Effect identifiers understood by the pad force-feedback pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectID {
    Constant,
    Spring,
    Damper,
    Friction,
    Rumble,
    Unknown,
}

/// Force-feedback command slots (Logitech encoding).
pub mod ff_cmd {
    pub const CMD_DOWNLOAD: u8 = 0x00;
    pub const CMD_DOWNLOAD_AND_PLAY: u8 = 0x01;
    pub const CMD_STOP: u8 = 0x03;
    pub const CMD_DEFAULT_SPRING_ON: u8 = 0x04;
    pub const CMD_DEFAULT_SPRING_OFF: u8 = 0x05;
    pub const CMD_NORMAL_MODE: u8 = 0x08;
    pub const CMD_SET_LED: u8 = 0x09;
    pub const CMD_RAW_MODE: u8 = 0x0B;
    pub const CMD_SET_DEFAULT_SPRING: u8 = 0x0E;
    pub const CMD_SET_DEAD_BAND: u8 = 0x0F;
    pub const CMD_EXTENDED_CMD: u8 = 0xF8;
}

/// Logitech force-feedback effect type IDs.
pub mod ff_type {
    pub const FTYPE_CONSTANT: u8 = 0x01;
    pub const FTYPE_SPRING: u8 = 0x02;
    pub const FTYPE_VARIABLE: u8 = 0x03;
    pub const FTYPE_FRICTION: u8 = 0x04;
    pub const FTYPE_DAMPER: u8 = 0x05;
    pub const FTYPE_HIGH_RESOLUTION_SPRING: u8 = 0x06;
    pub const FTYPE_HIGH_RESOLUTION_DAMPER: u8 = 0x07;
    pub const FTYPE_AUTO_CENTER_SPRING: u8 = 0x08;
}

/// Logitech capability flags.
pub mod ff_caps {
    pub const FF_LG_CAPS_HIGH_RES_COEF: u8 = 0x01;
    pub const FF_LG_CAPS_OLD_LOW_RES_COEF: u8 = 0x02;
    pub const FF_LG_CAPS_HIGH_RES_DEADBAND: u8 = 0x04;
    pub const FF_LG_CAPS_DAMPER_CLIP: u8 = 0x08;
}

/// Extended FF wheel range commands.
pub const EXT_CMD_WHEEL_RANGE_200_DEGREES: u8 = 0x01;
pub const EXT_CMD_WHEEL_RANGE_900_DEGREES: u8 = 0x02;

/// RealPlay sub-types.
pub mod realplay_type {
    pub const REALPLAY_RACING: u32 = 0;
    pub const REALPLAY_SPHERE: u32 = 1;
    pub const REALPLAY_GOLF: u32 = 2;
    pub const REALPLAY_POOL: u32 = 3;
}

/// Train device sub-types.
pub mod train_type {
    pub const TRAIN_TYPE2: u32 = 0;
    pub const TRAIN_SHINKANSEN: u32 = 1;
    pub const TRAIN_RYOJOUHEN: u32 = 2;
    pub const TRAIN_MASCON: u32 = 3;
    pub const MASTER_CONTROLLER: u32 = 4;
}

// ---------------------------------------------------------------------------
// Generic binding / setting info
// ---------------------------------------------------------------------------

/// Lightweight stand-in for PCSX2's `InputBindingInfo`. The full UI/HID
/// metadata is unnecessary for the translation; the fields are preserved
/// to keep the call sites identifiable.
#[derive(Debug, Clone)]
pub struct InputBindingInfo {
    pub name: &'static str,
    pub display_name: &'static str,
    pub icon: Option<&'static str>,
    pub kind: BindingType,
    pub id: u32,
    pub generic: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingType {
    Button,
    Axis,
    HalfAxis,
    Motor,
}

/// Stand-in for PCSX2's `SettingInfo`.
#[derive(Debug, Clone)]
pub struct SettingInfo {
    pub kind: SettingType,
    pub name: &'static str,
    pub display_name: &'static str,
    pub description: &'static str,
    pub default_value: &'static str,
    pub min_value: Option<&'static str>,
    pub max_value: Option<&'static str>,
    pub step_value: Option<&'static str>,
    pub format: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingType {
    Boolean,
    Integer,
    Float,
    StringList,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// `std::clamp` analogue operating on any `Ord` value.
pub fn clamp<T: Ord>(value: T, lo: T, hi: T) -> T {
    if value < lo {
        lo
    } else if value > hi {
        hi
    } else {
        value
    }
}

/// `std::lroundf` analogue: rounds to the nearest integer with ties away
/// from zero.
pub fn lroundf(value: f32) -> i64 {
    let v = value as f64;
    if v >= 0.0 {
        (v + 0.5) as i64
    } else {
        (v - 0.5) as i64
    }
}

/// Convert a binding `value` in `[0.0, 1.0]` to an unsigned 12-bit (0..=4095)
/// axis with optional axis inversion.
pub fn to_axis_u12(value: f32, invert: bool) -> u16 {
    let raw = clamp(lroundf(value * 4095.0), 0, 4095) as i64;
    let axis = if invert { 4095 - raw } else { raw };
    clamp(axis, 0, 4095) as u16
}

/// Convert a binding `value` in `[0.0, 1.0]` to a 12-bit axis range of
/// 0..=2047 (Gametrak stick axes) with optional inversion.
pub fn to_axis_u11(value: f32, invert: bool) -> u16 {
    let raw = clamp(lroundf(value * 2047.0), 0, 2047) as i64;
    let axis = if invert { 2047 - raw } else { raw };
    clamp(axis, 0, 2047) as u16
}

/// Convert a binding `value` in `[0.0, 1.0]` to a 12-bit axis range of
/// 0..=`limit` (used for Gametrak Z axis).
pub fn to_axis_u32(value: f32, limit: i64, invert: bool) -> u32 {
    let raw = clamp(lroundf((value as f64 * limit as f64) as f32), 0, limit) as i64;
    let axis = if invert { limit - raw } else { raw };
    clamp(axis, 0, limit) as u32
}

/// Boolean binding (>= 0.5 == pressed).
pub fn to_button(value: f32) -> bool {
    value >= 0.5
}

/// 8-bit encoding of a unit binding.
pub fn to_u8(value: f32) -> u8 {
    clamp(lroundf(value * 255.0), 0, 255) as u8
}

/// 9-bit / 512 unit encoding of a unit binding.
pub fn to_u9(value: f32) -> u32 {
    clamp(lroundf(value * 512.0), 0, 512) as u32
}

/// 7-bit / 128 unit encoding of a unit binding with multiplier.
pub fn to_u7(value: f32, multiplier: f32) -> u32 {
    let raw = (value as f64) * (multiplier as f64) * 128.0;
    clamp(lroundf_double(raw), 0, 128) as u32
}

fn lroundf_double(v: f64) -> i64 {
    if v >= 0.0 {
        (v + 0.5) as i64
    } else {
        (v - 0.5) as i64
    }
}

/// Linear encoding of a Logitech `ff_u8` constant force (0..=255) into a
/// signed 16-bit force level (signed offset around 0).
pub fn lg_u8_to_s16(force: i32) -> i16 {
    clamp(force, 0, 255) as i16
}

/// Linear encoding of a Logitech `ff_u8` value into a `u16` of `0..=0xFFFF`.
pub fn lg_u8_to_u16(value: i32) -> u16 {
    (clamp(value, 0, 255) as i32 * 0xFFFF / 255) as u16
}

/// Inverse of [`lg_u8_to_s16`] for symmetry with the C++ API.
pub fn lg_u16_to_s16(value: i32) -> i16 {
    clamp(value, 0, 0xFFFF) as i16
}

// ---------------------------------------------------------------------------
// Parsed force-feedback data structures (Logitech)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct SpringForce {
    pub k1: u8,
    pub s1: u8,
    pub d1: u8,
    pub dead1: u8,
    pub k2: u8,
    pub s2: u8,
    pub d2: u8,
    pub dead2: u8,
    pub clip: u8,
}

#[derive(Debug, Clone, Default)]
pub struct DamperForce {
    pub k1: u8,
    pub s1: u8,
    pub k2: u8,
    pub s2: u8,
    pub clip: u8,
}

#[derive(Debug, Clone, Default)]
pub struct FrictionForce {
    pub k1: u8,
    pub s1: u8,
    pub k2: u8,
    pub s2: u8,
    pub clip: u8,
}

#[derive(Debug, Clone, Default)]
pub struct AutoCenterForce {
    pub k1: u8,
    pub k2: u8,
    pub clip: u8,
}

#[derive(Debug, Clone, Default)]
pub struct VariableForce {
    pub l1: u8,
    pub t1: u8,
    pub s1: u8,
    pub d1: u8,
    pub l2: u8,
    pub t2: u8,
    pub s2: u8,
    pub d2: u8,
}

#[derive(Debug, Clone, Default)]
pub struct ConditionData {
    pub left_saturation: u16,
    pub right_saturation: u16,
    pub left_coeff: i16,
    pub right_coeff: i16,
    pub center: i16,
    pub deadband: u16,
}

#[derive(Debug, Clone, Default)]
pub struct ParsedFfData {
    pub condition: ConditionData,
    pub constant: i16,
    pub autocenter: i32,
}

#[derive(Debug, Clone)]
pub struct FfData {
    pub cmdslot: u8,
    pub kind: u8,
    pub spring: SpringForce,
    pub damper: DamperForce,
    pub friction: FrictionForce,
    pub autocenter: AutoCenterForce,
    pub variable: VariableForce,
    pub params: [u8; 4],
    pub padd0: u8,
}

// ---------------------------------------------------------------------------
// Common force-feedback device trait
// ---------------------------------------------------------------------------

/// Trait exposed by every FF-capable device. The original C++ base class
/// `FFDevice` is preserved here as a Rust trait so that the per-driver
/// implementations can be substituted for one another in tests.
pub trait FFDevice {
    fn set_constant_force(&mut self, level: i16);
    fn set_spring_force(&mut self, ff: &ParsedFfData);
    fn set_damper_force(&mut self, ff: &ParsedFfData);
    fn set_friction_force(&mut self, ff: &ParsedFfData);
    fn set_auto_center(&mut self, value: i32);
    fn disable_force(&mut self, effect: EffectID);
}

// ---------------------------------------------------------------------------
// UsbBuzz
// ---------------------------------------------------------------------------

/// State for the Logitech Buzz controller. Each player has 5 booleans
/// (red/blue/orange/green/yellow) replicated four times in the report.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BuzzReport {
    pub head1: u8,
    pub head2: u8,
    pub tail: u8,
    pub player1_red: bool,
    pub player1_blue: bool,
    pub player1_orange: bool,
    pub player1_green: bool,
    pub player1_yellow: bool,
    pub player2_red: bool,
    pub player2_blue: bool,
    pub player2_orange: bool,
    pub player2_green: bool,
    pub player2_yellow: bool,
    pub player3_red: bool,
    pub player3_blue: bool,
    pub player3_orange: bool,
    pub player3_green: bool,
    pub player3_yellow: bool,
    pub player4_red: bool,
    pub player4_blue: bool,
    pub player4_orange: bool,
    pub player4_green: bool,
    pub player4_yellow: bool,
}

pub mod buzz_cid {
    pub const CID_BUZZ_PLAYER1_RED: u32 = 0;
    pub const CID_BUZZ_PLAYER1_BLUE: u32 = 1;
    pub const CID_BUZZ_PLAYER1_ORANGE: u32 = 2;
    pub const CID_BUZZ_PLAYER1_GREEN: u32 = 3;
    pub const CID_BUZZ_PLAYER1_YELLOW: u32 = 4;
    pub const CID_BUZZ_PLAYER2_RED: u32 = 5;
    pub const CID_BUZZ_PLAYER2_BLUE: u32 = 6;
    pub const CID_BUZZ_PLAYER2_ORANGE: u32 = 7;
    pub const CID_BUZZ_PLAYER2_GREEN: u32 = 8;
    pub const CID_BUZZ_PLAYER2_YELLOW: u32 = 9;
    pub const CID_BUZZ_PLAYER3_RED: u32 = 10;
    pub const CID_BUZZ_PLAYER3_BLUE: u32 = 11;
    pub const CID_BUZZ_PLAYER3_ORANGE: u32 = 12;
    pub const CID_BUZZ_PLAYER3_GREEN: u32 = 13;
    pub const CID_BUZZ_PLAYER3_YELLOW: u32 = 14;
    pub const CID_BUZZ_PLAYER4_RED: u32 = 15;
    pub const CID_BUZZ_PLAYER4_BLUE: u32 = 16;
    pub const CID_BUZZ_PLAYER4_ORANGE: u32 = 17;
    pub const CID_BUZZ_PLAYER4_GREEN: u32 = 18;
    pub const CID_BUZZ_PLAYER4_YELLOW: u32 = 19;
}

pub struct UsbBuzz {
    port: u32,
    data: BuzzReport,
    last_data: BuzzReport,
}

impl UsbBuzz {
    pub fn new(port: u32) -> Self {
        Self {
            port,
            data: BuzzReport {
                head1: 0x7f,
                head2: 0x7f,
                tail: 0x0f,
                ..Default::default()
            },
            last_data: BuzzReport {
                head1: 0x7f,
                head2: 0x7f,
                tail: 0x0f,
                ..Default::default()
            },
        }
    }

    pub fn port(&self) -> u32 {
        self.port
    }

    pub fn data(&self) -> &BuzzReport {
        &self.data
    }

    /// `USBPad::Update` analogue: pulls a fresh report if any button changed
    /// since the last call. Mirrors the C++ logic of returning NAK when no
    /// state has changed.
    pub fn update(&mut self, buffer: &mut [u8]) -> i32 {
        if self.data == self.last_data {
            return -1; // USB_RET_NAK
        }

        // Re-prepend the 7f 7f 0f framing.
        self.data.head1 = 0x7f;
        self.data.head2 = 0x7f;
        self.data.tail = 0x0f;

        let bytes = buzz_report_to_bytes(&self.data);
        let len = min(bytes.len(), buffer.len());
        buffer[..len].copy_from_slice(&bytes[..len]);
        self.last_data = self.data.clone();
        len as i32
    }

    pub fn get_binding_value(&self, bind_index: u32) -> f32 {
        use buzz_cid::*;
        match bind_index {
            CID_BUZZ_PLAYER1_RED => f32::from(self.data.player1_red as u8),
            CID_BUZZ_PLAYER1_BLUE => f32::from(self.data.player1_blue as u8),
            CID_BUZZ_PLAYER1_ORANGE => f32::from(self.data.player1_orange as u8),
            CID_BUZZ_PLAYER1_GREEN => f32::from(self.data.player1_green as u8),
            CID_BUZZ_PLAYER1_YELLOW => f32::from(self.data.player1_yellow as u8),
            CID_BUZZ_PLAYER2_RED => f32::from(self.data.player2_red as u8),
            CID_BUZZ_PLAYER2_BLUE => f32::from(self.data.player2_blue as u8),
            CID_BUZZ_PLAYER2_ORANGE => f32::from(self.data.player2_orange as u8),
            CID_BUZZ_PLAYER2_GREEN => f32::from(self.data.player2_green as u8),
            CID_BUZZ_PLAYER2_YELLOW => f32::from(self.data.player2_yellow as u8),
            CID_BUZZ_PLAYER3_RED => f32::from(self.data.player3_red as u8),
            CID_BUZZ_PLAYER3_BLUE => f32::from(self.data.player3_blue as u8),
            CID_BUZZ_PLAYER3_ORANGE => f32::from(self.data.player3_orange as u8),
            CID_BUZZ_PLAYER3_GREEN => f32::from(self.data.player3_green as u8),
            CID_BUZZ_PLAYER3_YELLOW => f32::from(self.data.player3_yellow as u8),
            CID_BUZZ_PLAYER4_RED => f32::from(self.data.player4_red as u8),
            CID_BUZZ_PLAYER4_BLUE => f32::from(self.data.player4_blue as u8),
            CID_BUZZ_PLAYER4_ORANGE => f32::from(self.data.player4_orange as u8),
            CID_BUZZ_PLAYER4_GREEN => f32::from(self.data.player4_green as u8),
            CID_BUZZ_PLAYER4_YELLOW => f32::from(self.data.player4_yellow as u8),
            _ => 0.0,
        }
    }

    pub fn set_binding_value(&mut self, bind_index: u32, value: f32) {
        use buzz_cid::*;
        let v = to_button(value);
        match bind_index {
            CID_BUZZ_PLAYER1_RED => self.data.player1_red = v,
            CID_BUZZ_PLAYER1_BLUE => self.data.player1_blue = v,
            CID_BUZZ_PLAYER1_ORANGE => self.data.player1_orange = v,
            CID_BUZZ_PLAYER1_GREEN => self.data.player1_green = v,
            CID_BUZZ_PLAYER1_YELLOW => self.data.player1_yellow = v,
            CID_BUZZ_PLAYER2_RED => self.data.player2_red = v,
            CID_BUZZ_PLAYER2_BLUE => self.data.player2_blue = v,
            CID_BUZZ_PLAYER2_ORANGE => self.data.player2_orange = v,
            CID_BUZZ_PLAYER2_GREEN => self.data.player2_green = v,
            CID_BUZZ_PLAYER2_YELLOW => self.data.player2_yellow = v,
            CID_BUZZ_PLAYER3_RED => self.data.player3_red = v,
            CID_BUZZ_PLAYER3_BLUE => self.data.player3_blue = v,
            CID_BUZZ_PLAYER3_ORANGE => self.data.player3_orange = v,
            CID_BUZZ_PLAYER3_GREEN => self.data.player3_green = v,
            CID_BUZZ_PLAYER3_YELLOW => self.data.player3_yellow = v,
            CID_BUZZ_PLAYER4_RED => self.data.player4_red = v,
            CID_BUZZ_PLAYER4_BLUE => self.data.player4_blue = v,
            CID_BUZZ_PLAYER4_ORANGE => self.data.player4_orange = v,
            CID_BUZZ_PLAYER4_GREEN => self.data.player4_green = v,
            CID_BUZZ_PLAYER4_YELLOW => self.data.player4_yellow = v,
            _ => {}
        }
    }

    pub fn bindings() -> &'static [InputBindingInfo] {
        use buzz_cid::*;
        &[
            InputBindingInfo { name: "Red1",    display_name: "Player 1 Red",    icon: None, kind: BindingType::Button, id: CID_BUZZ_PLAYER1_RED,    generic: 0 },
            InputBindingInfo { name: "Blue1",   display_name: "Player 1 Blue",   icon: None, kind: BindingType::Button, id: CID_BUZZ_PLAYER1_BLUE,   generic: 0 },
            InputBindingInfo { name: "Orange1", display_name: "Player 1 Orange", icon: None, kind: BindingType::Button, id: CID_BUZZ_PLAYER1_ORANGE, generic: 0 },
            InputBindingInfo { name: "Green1",  display_name: "Player 1 Green",  icon: None, kind: BindingType::Button, id: CID_BUZZ_PLAYER1_GREEN,  generic: 0 },
            InputBindingInfo { name: "Yellow1", display_name: "Player 1 Yellow", icon: None, kind: BindingType::Button, id: CID_BUZZ_PLAYER1_YELLOW, generic: 0 },
            InputBindingInfo { name: "Red2",    display_name: "Player 2 Red",    icon: None, kind: BindingType::Button, id: CID_BUZZ_PLAYER2_RED,    generic: 0 },
            InputBindingInfo { name: "Blue2",   display_name: "Player 2 Blue",   icon: None, kind: BindingType::Button, id: CID_BUZZ_PLAYER2_BLUE,   generic: 0 },
            InputBindingInfo { name: "Orange2", display_name: "Player 2 Orange", icon: None, kind: BindingType::Button, id: CID_BUZZ_PLAYER2_ORANGE, generic: 0 },
            InputBindingInfo { name: "Green2",  display_name: "Player 2 Green",  icon: None, kind: BindingType::Button, id: CID_BUZZ_PLAYER2_GREEN,  generic: 0 },
            InputBindingInfo { name: "Yellow2", display_name: "Player 2 Yellow", icon: None, kind: BindingType::Button, id: CID_BUZZ_PLAYER2_YELLOW, generic: 0 },
            InputBindingInfo { name: "Red3",    display_name: "Player 3 Red",    icon: None, kind: BindingType::Button, id: CID_BUZZ_PLAYER3_RED,    generic: 0 },
            InputBindingInfo { name: "Blue3",   display_name: "Player 3 Blue",   icon: None, kind: BindingType::Button, id: CID_BUZZ_PLAYER3_BLUE,   generic: 0 },
            InputBindingInfo { name: "Orange3", display_name: "Player 3 Orange", icon: None, kind: BindingType::Button, id: CID_BUZZ_PLAYER3_ORANGE, generic: 0 },
            InputBindingInfo { name: "Green3",  display_name: "Player 3 Green",  icon: None, kind: BindingType::Button, id: CID_BUZZ_PLAYER3_GREEN,  generic: 0 },
            InputBindingInfo { name: "Yellow3", display_name: "Player 3 Yellow", icon: None, kind: BindingType::Button, id: CID_BUZZ_PLAYER3_YELLOW, generic: 0 },
            InputBindingInfo { name: "Red4",    display_name: "Player 4 Red",    icon: None, kind: BindingType::Button, id: CID_BUZZ_PLAYER4_RED,    generic: 0 },
            InputBindingInfo { name: "Blue4",   display_name: "Player 4 Blue",   icon: None, kind: BindingType::Button, id: CID_BUZZ_PLAYER4_BLUE,   generic: 0 },
            InputBindingInfo { name: "Orange4", display_name: "Player 4 Orange", icon: None, kind: BindingType::Button, id: CID_BUZZ_PLAYER4_ORANGE, generic: 0 },
            InputBindingInfo { name: "Green4",  display_name: "Player 4 Green",  icon: None, kind: BindingType::Button, id: CID_BUZZ_PLAYER4_GREEN,  generic: 0 },
            InputBindingInfo { name: "Yellow4", display_name: "Player 4 Yellow", icon: None, kind: BindingType::Button, id: CID_BUZZ_PLAYER4_YELLOW, generic: 0 },
        ]
    }

    pub fn settings() -> &'static [SettingInfo] {
        &[]
    }
}

impl UsbBuzz {
    pub fn init(&mut self) {
        // No-op: state is already zeroed by the constructor.
    }
    pub fn shutdown(&mut self) {}
    pub fn update_pad(&mut self) {
        // Periodic hook: a real driver would call the host to fetch button
        // state here. The translation just normalises the head/tail bytes.
        self.data.head1 = 0x7f;
        self.data.head2 = 0x7f;
        self.data.tail = 0x0f;
    }
}

fn buzz_report_to_bytes(r: &BuzzReport) -> [u8; 12] {
    // Layout used by the on-wire report: 7f 7f 0f [buttons...].
    let mut out = [0u8; 12];
    out[0] = r.head1;
    out[1] = r.head2;
    out[2] = r.tail;
    let mut bits = 0u32;
    let map: [(bool, u8); 20] = [
        (r.player1_red, 0),
        (r.player1_blue, 1),
        (r.player1_orange, 2),
        (r.player1_green, 3),
        (r.player1_yellow, 4),
        (r.player2_red, 5),
        (r.player2_blue, 6),
        (r.player2_orange, 7),
        (r.player2_green, 8),
        (r.player2_yellow, 9),
        (r.player3_red, 10),
        (r.player3_blue, 11),
        (r.player3_orange, 12),
        (r.player3_green, 13),
        (r.player3_yellow, 14),
        (r.player4_red, 15),
        (r.player4_blue, 16),
        (r.player4_orange, 17),
        (r.player4_green, 18),
        (r.player4_yellow, 19),
    ];
    for (pressed, bit) in map.iter() {
        if *pressed {
            bits |= 1u32 << bit;
        }
    }
    out[3] = (bits & 0xFF) as u8;
    out[4] = ((bits >> 8) & 0xFF) as u8;
    out[5] = ((bits >> 16) & 0x0F) as u8;
    out
}

// ---------------------------------------------------------------------------
// UsbGametrak
// ---------------------------------------------------------------------------

pub mod gametrak_cid {
    pub const CID_GT_BUTTON: u32 = 0;
    pub const CID_GT_LEFT_X: u32 = 1;
    pub const CID_GT_LEFT_Y: u32 = 2;
    pub const CID_GT_LEFT_Z: u32 = 3;
    pub const CID_GT_RIGHT_X: u32 = 4;
    pub const CID_GT_RIGHT_Y: u32 = 5;
    pub const CID_GT_RIGHT_Z: u32 = 6;
}

#[derive(Debug, Clone, Default)]
pub struct GametrakReport {
    pub k1: u8,
    pub k2: u8,
    pub k3: u8,
    pub k4: u8,
    pub k5: u8,
    pub k6: u8,
    pub left_x: u16,
    pub left_y: u16,
    pub left_z: u32,
    pub right_x: u16,
    pub right_y: u16,
    pub right_z: u32,
    pub button: bool,
}

pub struct UsbGametrak {
    port: u32,
    state: u32,
    key: u32,
    data: GametrakReport,
    invert_x: bool,
    invert_y: bool,
    invert_z: bool,
    limit_z: i64,
}

impl UsbGametrak {
    pub fn new(port: u32) -> Self {
        Self {
            port,
            state: 0,
            key: 0,
            data: GametrakReport::default(),
            invert_x: false,
            invert_y: false,
            invert_z: false,
            limit_z: 4095,
        }
    }

    pub fn port(&self) -> u32 {
        self.port
    }

    /// Compute the next 24-bit Gametrak key by transforming the running
    /// key and return the upper 8 bits (the value sent to the console).
    pub fn compute_key(key: &mut u32) -> u32 {
        let v = *key;
        let mut ret: u32 = 0;
        ret |= (v << 2) & 0x00FC0000;
        ret |= (v << 17) & 0x00020000;
        ret ^= (v << 16) & 0x00FE0000;
        ret |= v & 0x00010000;
        ret |= (v >> 9) & 0x00007F7F;
        ret |= (v << 7) & 0x00008080;
        *key = ret;
        ret >> 16
    }

    /// Handle a `SET_REPORT` USB control transfer from the host. The
    /// authentication sequence unlocks the device with the `"Gametrak"`
    /// secret or feeds a new key byte.
    pub fn handle_set_report(&mut self, data: &[u8]) {
        const SECRET: &[u8; 8] = b"Gametrak";
        if data.len() == 8 && &data[..8] == SECRET {
            self.state = 0;
            self.key = 0;
        } else if data.len() == 2 {
            if data[0] == 0x45 {
                self.key = (data[1] as u32) << 16;
            }
            if (self.key >> 16) == data[1] as u32 {
                UsbGametrak::compute_key(&mut self.key);
            }
        }
    }

    /// Emulate the device-side data phase for endpoint 1. When `state == 0`
    /// the device has just been authenticated and the first response is
    /// the secret string; subsequent responses pack the report bytes.
    pub fn token_in(&mut self, buffer: &mut [u8]) -> usize {
        const SECRET: &[u8; 16] = b"Gametrak\0\0\0\0\0\0\0\0";
        if self.state == 0 {
            self.state = 1;
            let len = min(SECRET.len(), buffer.len());
            buffer[..len].copy_from_slice(&SECRET[..len]);
            return len;
        }
        // Refresh the per-tick key bits embedded in the report.
        self.data.k1 = ((self.key >> 16) & 1) as u8;
        self.data.k2 = ((self.key >> 17) & 1) as u8;
        self.data.k3 = ((self.key >> 18) & 1) as u8;
        self.data.k4 = ((self.key >> 19) & 1) as u8;
        self.data.k5 = ((self.key >> 20) & 1) as u8;
        self.data.k6 = ((self.key >> 21) & 1) as u8;
        let bytes = gametrak_report_to_bytes(&self.data);
        let len = min(bytes.len(), buffer.len());
        buffer[..len].copy_from_slice(&bytes[..len]);
        len
    }

    pub fn get_binding_value(&self, bind_index: u32) -> f32 {
        use gametrak_cid::*;
        match bind_index {
            CID_GT_BUTTON => f32::from(self.data.button as u8),
            _ => 0.0,
        }
    }

    pub fn set_binding_value(&mut self, bind_index: u32, value: f32) {
        use gametrak_cid::*;
        match bind_index {
            CID_GT_BUTTON => self.data.button = to_button(value),
            CID_GT_LEFT_X => self.data.left_x = to_axis_u11(value, self.invert_x),
            CID_GT_LEFT_Y => self.data.left_y = to_axis_u11(value, self.invert_y),
            CID_GT_LEFT_Z => self.data.left_z = to_axis_u32(value, self.limit_z, self.invert_z),
            CID_GT_RIGHT_X => self.data.right_x = to_axis_u11(value, self.invert_x),
            CID_GT_RIGHT_Y => self.data.right_y = to_axis_u11(value, self.invert_y),
            CID_GT_RIGHT_Z => self.data.right_z = to_axis_u32(value, self.limit_z, self.invert_z),
            _ => {}
        }
    }

    pub fn update_settings(&mut self, invert_x: bool, invert_y: bool, invert_z: bool, limit_z: i64) {
        self.invert_x = invert_x;
        self.invert_y = invert_y;
        self.invert_z = invert_z;
        self.limit_z = clamp(limit_z, 100, 4095);
    }

    pub fn bindings() -> &'static [InputBindingInfo] {
        use gametrak_cid::*;
        &[
            InputBindingInfo { name: "FootPedal", display_name: "Foot Pedal", icon: None, kind: BindingType::Button, id: CID_GT_BUTTON,  generic: 0 },
            InputBindingInfo { name: "LeftX",     display_name: "Left X",     icon: None, kind: BindingType::Axis,   id: CID_GT_LEFT_X,  generic: 0 },
            InputBindingInfo { name: "LeftY",     display_name: "Left Y",     icon: None, kind: BindingType::Axis,   id: CID_GT_LEFT_Y,  generic: 0 },
            InputBindingInfo { name: "LeftZ",     display_name: "Left Z",     icon: None, kind: BindingType::Axis,   id: CID_GT_LEFT_Z,  generic: 0 },
            InputBindingInfo { name: "RightX",    display_name: "Right X",    icon: None, kind: BindingType::Axis,   id: CID_GT_RIGHT_X, generic: 0 },
            InputBindingInfo { name: "RightY",    display_name: "Right Y",    icon: None, kind: BindingType::Axis,   id: CID_GT_RIGHT_Y, generic: 0 },
            InputBindingInfo { name: "RightZ",    display_name: "Right Z",    icon: None, kind: BindingType::Axis,   id: CID_GT_RIGHT_Z, generic: 0 },
        ]
    }

    pub fn settings() -> &'static [SettingInfo] {
        &[
            SettingInfo { kind: SettingType::Boolean, name: "invert_x_axis", display_name: "Invert X axis", description: "Invert X axis", default_value: "false", min_value: None, max_value: None, step_value: None, format: None },
            SettingInfo { kind: SettingType::Boolean, name: "invert_y_axis", display_name: "Invert Y axis", description: "Invert Y axis", default_value: "false", min_value: None, max_value: None, step_value: None, format: None },
            SettingInfo { kind: SettingType::Boolean, name: "invert_z_axis", display_name: "Invert Z axis", description: "Invert Z axis", default_value: "false", min_value: None, max_value: None, step_value: None, format: None },
            SettingInfo { kind: SettingType::Integer, name: "limit_z_axis",  display_name: "Limit Z axis [100-4095]", description: "- 4095 for original Gametrak controllers\n- 1790 for standard gamepads", default_value: "4095", min_value: Some("100"), max_value: Some("4095"), step_value: Some("1"), format: Some("%d") },
        ]
    }

    pub fn init(&mut self) {
        self.state = 0;
        self.key = 0;
    }
    pub fn shutdown(&mut self) {}
    pub fn update(&mut self) {
        // Tick: refresh key derivation.
        UsbGametrak::compute_key(&mut self.key);
    }
}

fn gametrak_report_to_bytes(r: &GametrakReport) -> [u8; 24] {
    let mut out = [0u8; 24];
    let mut flags = 0u8;
    flags |= (r.k1 & 1) << 0;
    flags |= (r.k2 & 1) << 1;
    flags |= (r.k3 & 1) << 2;
    flags |= (r.k4 & 1) << 3;
    flags |= (r.k5 & 1) << 4;
    flags |= (r.k6 & 1) << 5;
    out[0] = flags;
    out[1] = r.button as u8;
    let mut p = 2;
    for axis in [r.left_x, r.left_y, r.right_x, r.right_y] {
        out[p]     = (axis & 0xFF) as u8;
        out[p + 1] = ((axis >> 8) & 0xFF) as u8;
        p += 2;
    }
    for axis in [r.left_z, r.right_z] {
        let bytes = (axis as u32).to_le_bytes();
        out[p]     = bytes[0];
        out[p + 1] = bytes[1];
        out[p + 2] = bytes[2];
        p += 3;
    }
    out
}

// ---------------------------------------------------------------------------
// UsbPadFF - generic force-feedback driver
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct FfSlotState {
    pub slot_type: [u8; 4],
    pub slot_force: [u8; 4],
}

pub struct UsbPadFF {
    ff_state: FfSlotState,
    ff_dev: Option<Box<dyn FFDevice>>,
    use_dropout_workaround: bool,
    warned_variable: bool,
}

impl UsbPadFF {
    pub fn new() -> Self {
        Self {
            ff_state: FfSlotState::default(),
            ff_dev: None,
            use_dropout_workaround: false,
            warned_variable: false,
        }
    }

    pub fn with_device(dev: Box<dyn FFDevice>) -> Self {
        let mut s = Self::new();
        s.ff_dev = Some(dev);
        s
    }

    pub fn set_ff_device(&mut self, dev: Box<dyn FFDevice>) {
        self.ff_dev = Some(dev);
    }

    pub fn set_use_dropout_workaround(&mut self, enabled: bool) {
        self.use_dropout_workaround = enabled;
    }

    pub fn slot_state(&self) -> &FfSlotState {
        &self.ff_state
    }

    pub fn init(&mut self) {
        self.ff_state = FfSlotState::default();
    }

    pub fn shutdown(&mut self) {
        if let Some(dev) = self.ff_dev.as_mut() {
            dev.set_constant_force(0);
            dev.disable_force(EffectID::Constant);
            dev.disable_force(EffectID::Spring);
            dev.disable_force(EffectID::Damper);
            dev.disable_force(EffectID::Friction);
            dev.set_auto_center(0);
        }
    }

    pub fn update(&mut self) {
        // No periodic work; the device is event driven.
    }

    /// Parse a single FF command from the host and dispatch to the
    /// currently installed FF device, if any.
    pub fn parse_ff_data(&mut self, ff: &FfData, is_dfp: bool) {
        if self.ff_dev.is_none() {
            return;
        }
        if ff.cmdslot == ff_cmd::CMD_EXTENDED_CMD {
            // Wheel range extended commands are accepted silently.
            return;
        }
        let slots = (ff.cmdslot & 0xF0) >> 4;
        let cmd = ff.cmdslot & 0x0F;
        let dev = self.ff_dev.as_mut().unwrap();
        match cmd {
            ff_cmd::CMD_DOWNLOAD => {
                for i in 0..4 {
                    if slots & (1 << i) != 0 {
                        self.ff_state.slot_type[i] = ff.kind;
                    }
                }
            }
            ff_cmd::CMD_DOWNLOAD_AND_PLAY => {
                for i in 0..4 {
                    if slots & (1 << i) != 0 {
                        self.ff_state.slot_type[i] = ff.kind;
                        if ff.kind == ff_type::FTYPE_CONSTANT {
                            self.ff_state.slot_force[i] = ff.params[i];
                        }
                    }
                }
                match ff.kind {
                    ff_type::FTYPE_CONSTANT => {
                        if slots == 0xF {
                            let mut force: i32 = 0;
                            for i in 0..4 {
                                let mut t = ff.params[i] as i32;
                                if t < 128 {
                                    t += 1;
                                }
                                force = clamp(force + t - 128, -128, 127);
                            }
                            dev.set_constant_force(lg_u8_to_s16(128 + force) as i16);
                        } else {
                            for i in 0..4 {
                                if slots == (1 << i) {
                                    dev.set_constant_force(ff.params[i] as i16);
                                }
                            }
                        }
                    }
                    ff_type::FTYPE_SPRING => {
                        let mut spring = ParsedFfData::default();
                        spring.condition.left_saturation = lg_u8_to_u16(ff.spring.clip as i32);
                        spring.condition.right_saturation = lg_u8_to_u16(ff.spring.clip as i32);
                        spring.condition.left_coeff = lg_get_condition_coef(
                            if is_dfp { 0 } else { ff_caps::FF_LG_CAPS_OLD_LOW_RES_COEF },
                            ff.spring.k1, ff.spring.s1, i16::MAX,
                        );
                        spring.condition.right_coeff = lg_get_condition_coef(
                            if is_dfp { 0 } else { ff_caps::FF_LG_CAPS_OLD_LOW_RES_COEF },
                            ff.spring.k2, ff.spring.s2, i16::MAX,
                        );
                        let center = ((ff.spring.dead1 as i32 + ff.spring.dead2 as i32) / 2) as i32;
                        let deadband = (ff.spring.dead2 as i32 - ff.spring.dead1 as i32) as i32;
                        spring.condition.center = lg_u16_to_s16(center);
                        spring.condition.deadband = clamp(deadband, 0, i16::MAX as i32) as u16;
                        dev.set_spring_force(&spring);
                    }
                    ff_type::FTYPE_HIGH_RESOLUTION_SPRING => {
                        let mut spring = ParsedFfData::default();
                        let caps = ff_caps::FF_LG_CAPS_HIGH_RES_COEF | ff_caps::FF_LG_CAPS_HIGH_RES_DEADBAND;
                        spring.condition.left_saturation = lg_u8_to_u16(ff.spring.clip as i32);
                        spring.condition.right_saturation = lg_u8_to_u16(ff.spring.clip as i32);
                        spring.condition.left_coeff = lg_get_condition_coef(caps, ff.spring.k1, ff.spring.s1, i16::MAX);
                        spring.condition.right_coeff = lg_get_condition_coef(caps, ff.spring.k2, ff.spring.s2, i16::MAX);
                        let d1 = lg_get_spring_deadband(caps, ff.spring.dead1, (ff.spring.s1 >> 1) & 0x7, u16::MAX);
                        let d2 = lg_get_spring_deadband(caps, ff.spring.dead2, (ff.spring.s2 >> 1) & 0x7, u16::MAX);
                        spring.condition.center = ((d1 as i32 + d2 as i32) / 2) as i16;
                        spring.condition.deadband = d2.saturating_sub(d1);
                        dev.set_spring_force(&spring);
                    }
                    ff_type::FTYPE_VARIABLE => {
                        if slots & 1 != 0 {
                            if ff.variable.t1 != 0 && ff.variable.s1 != 0 {
                                if !self.warned_variable {
                                    eprintln!("variable force cannot be converted to constant force");
                                    self.warned_variable = true;
                                }
                            } else {
                                dev.set_constant_force(ff.variable.l1 as i16);
                            }
                        } else if slots & (1 << 2) != 0 {
                            if ff.variable.t2 != 0 && ff.variable.s2 != 0 {
                                if !self.warned_variable {
                                    eprintln!("variable force cannot be converted to constant force");
                                    self.warned_variable = true;
                                }
                            } else {
                                dev.set_constant_force(ff.variable.l2 as i16);
                            }
                        }
                    }
                    ff_type::FTYPE_FRICTION => {
                        let mut friction = ParsedFfData::default();
                        let s1: i32 = if ff.friction.s1 & 1 != 0 { -1 } else { 1 };
                        let s2: i32 = if ff.friction.s2 & 1 != 0 { -1 } else { 1 };
                        friction.condition.left_coeff = ((ff.friction.k1 as i32) * 0x7FFF / 255 * s1) as i16;
                        friction.condition.right_coeff = ((ff.friction.k2 as i32) * 0x7FFF / 255 * s2) as i16;
                        let sat = (0x7FFF * (ff.friction.clip as i32) / 255) as u16;
                        friction.condition.left_saturation = sat;
                        friction.condition.right_saturation = sat;
                        dev.set_friction_force(&friction);
                    }
                    ff_type::FTYPE_DAMPER => {
                        let mut damper = ParsedFfData::default();
                        damper.condition.left_saturation = u16::MAX;
                        damper.condition.right_saturation = u16::MAX;
                        damper.condition.left_coeff = lg_get_condition_coef(0, ff.damper.k1, ff.damper.s1, i16::MAX);
                        damper.condition.right_coeff = lg_get_condition_coef(0, ff.damper.k2, ff.damper.s2, i16::MAX);
                        dev.set_damper_force(&damper);
                    }
                    ff_type::FTYPE_HIGH_RESOLUTION_DAMPER => {
                        let mut caps = ff_caps::FF_LG_CAPS_HIGH_RES_COEF;
                        if is_dfp { caps |= ff_caps::FF_LG_CAPS_DAMPER_CLIP; }
                        let mut damper = ParsedFfData::default();
                        let clip = if caps & ff_caps::FF_LG_CAPS_DAMPER_CLIP != 0 {
                            ff.damper.clip as i32 * u16::MAX as i32 / 255
                        } else {
                            u16::MAX as i32
                        };
                        damper.condition.left_saturation = clip as u16;
                        damper.condition.right_saturation = clip as u16;
                        damper.condition.left_coeff = lg_get_condition_coef(caps, ff.damper.k1, ff.damper.s1, i16::MAX);
                        damper.condition.right_coeff = lg_get_condition_coef(caps, ff.damper.k2, ff.damper.s2, i16::MAX);
                        dev.set_damper_force(&damper);
                    }
                    ff_type::FTYPE_AUTO_CENTER_SPRING => {
                        let v = (ff.autocenter.k1 as i32) * (ff.autocenter.clip as i32) / 255 * 100 / 255;
                        dev.set_auto_center(v);
                    }
                    _ => {
                        eprintln!("CMD_DOWNLOAD_AND_PLAY: unhandled force type 0x{:02X}", ff.kind);
                    }
                }
            }
            ff_cmd::CMD_STOP => {
                for i in 0..4 {
                    if slots & (1 << i) != 0 {
                        match self.ff_state.slot_type[i] {
                            ff_type::FTYPE_CONSTANT | ff_type::FTYPE_VARIABLE => {
                                dev.disable_force(EffectID::Constant);
                            }
                            ff_type::FTYPE_SPRING | ff_type::FTYPE_HIGH_RESOLUTION_SPRING => {
                                dev.disable_force(EffectID::Spring);
                            }
                            ff_type::FTYPE_AUTO_CENTER_SPRING => {
                                dev.set_auto_center(0);
                            }
                            ff_type::FTYPE_FRICTION => {
                                dev.disable_force(EffectID::Friction);
                            }
                            ff_type::FTYPE_DAMPER | ff_type::FTYPE_HIGH_RESOLUTION_DAMPER => {
                                dev.disable_force(EffectID::Damper);
                            }
                            _ => {
                                eprintln!("CMD_STOP: unhandled force type 0x{:02X}", ff.kind);
                            }
                        }
                    }
                }
            }
            ff_cmd::CMD_DEFAULT_SPRING_OFF => {
                if slots == 0x0F {
                    dev.set_constant_force(127);
                }
            }
            _ => {}
        }
    }
}

impl Default for UsbPadFF {
    fn default() -> Self { Self::new() }
}

// ---------------------------------------------------------------------------
// Logitech FF helpers (mirroring `lg/lg_ff.cpp`).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct LgCoef {
    pub num: u8,
    pub den: u8,
}

pub struct LgFF;

impl LgFF {
    pub fn new() -> Self { Self }
    pub fn init(&mut self) {}
    pub fn shutdown(&mut self) {}
    pub fn update(&mut self) {}
}

impl Default for LgFF {
    fn default() -> Self { Self }
}

/// Compute a Logitech spring or damper force coefficient. Returns the
/// signed 16-bit coefficient for the given capability flags and selector
/// bits. Negative values are produced by `s = 1`.
pub fn lg_get_condition_coef(caps: u8, k: u8, s: u8, max: i16) -> i16 {
    let coef = lg_get_force_coefficient(caps, k);
    let sign = if s != 0 { -1 } else { 1 };
    ((sign as i32) * (max as i32) * (coef.num as i32) / (coef.den as i32)) as i16
}

/// Compute a Logitech spring deadband, using the high-resolution mode when
/// the corresponding capability bit is set.
pub fn lg_get_spring_deadband(caps: u8, d: u8, dl: u8, max: u16) -> u16 {
    if caps & ff_caps::FF_LG_CAPS_HIGH_RES_DEADBAND != 0 {
        let v = ((d as u32) << 3) | (dl as u32);
        ((v * max as u32) / 0x7FF) as u16
    } else {
        ((d as u32) * max as u32 / 255) as u16
    }
}

/// Compute a Logitech damper clip.
pub fn lg_get_damper_clip(caps: u8, c: u8) -> u16 {
    if caps & ff_caps::FF_LG_CAPS_DAMPER_CLIP != 0 {
        ((c as u32) * u16::MAX as u32 / 255) as u16
    } else {
        u16::MAX
    }
}

fn lg_get_force_coefficient(caps: u8, k: u8) -> LgCoef {
    if caps & ff_caps::FF_LG_CAPS_HIGH_RES_COEF != 0 {
        return LgCoef { num: k, den: 0x0F };
    }
    if caps & ff_caps::FF_LG_CAPS_OLD_LOW_RES_COEF != 0 {
        return match k & 0x7 {
            0 => LgCoef { num: 1, den: 16 },
            1 => LgCoef { num: 1, den: 8 },
            2 => LgCoef { num: 3, den: 16 },
            3 => LgCoef { num: 1, den: 4 },
            4 => LgCoef { num: 3, den: 8 },
            5 => LgCoef { num: 3, den: 4 },
            6 => LgCoef { num: 2, den: 4 },
            _ => LgCoef { num: 4, den: 4 },
        };
    }
    match k & 0x7 {
        0 => LgCoef { num: 1, den: 16 },
        1 => LgCoef { num: 1, den: 8 },
        2 => LgCoef { num: 3, den: 16 },
        3 => LgCoef { num: 1, den: 4 },
        4 => LgCoef { num: 3, den: 8 },
        5 => LgCoef { num: 2, den: 4 },
        6 => LgCoef { num: 3, den: 4 },
        _ => LgCoef { num: 4, den: 4 },
    }
}

// ---------------------------------------------------------------------------
// UsbPadSdlFF - SDL backed force feedback device
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default)]
pub struct SdlHapticEffect {
    pub kind: u32,
    pub length: u32,
    // Constant effect
    pub constant_level: i16,
    // Condition effect
    pub left_sat: [u16; 3],
    pub right_sat: [u16; 3],
    pub left_coeff: [i16; 3],
    pub right_coeff: [i16; 3],
    pub deadband: [u16; 3],
    pub center: [i16; 3],
}

pub struct UsbPadSdlFF {
    haptic_open: bool,
    constant: SdlHapticEffect,
    spring: SdlHapticEffect,
    damper: SdlHapticEffect,
    friction: SdlHapticEffect,
    constant_id: i32,
    spring_id: i32,
    damper_id: i32,
    friction_id: i32,
    constant_running: bool,
    spring_running: bool,
    damper_running: bool,
    friction_running: bool,
    autocenter_supported: bool,
    use_dropout_workaround: bool,
}

impl UsbPadSdlFF {
    pub fn new() -> Self {
        Self {
            haptic_open: false,
            constant: SdlHapticEffect::default(),
            spring: SdlHapticEffect::default(),
            damper: SdlHapticEffect::default(),
            friction: SdlHapticEffect::default(),
            constant_id: -1,
            spring_id: -1,
            damper_id: -1,
            friction_id: -1,
            constant_running: false,
            spring_running: false,
            damper_running: false,
            friction_running: false,
            autocenter_supported: false,
            use_dropout_workaround: false,
        }
    }

    pub fn set_use_dropout_workaround(&mut self, v: bool) {
        self.use_dropout_workaround = v;
    }

    /// Create the underlying haptic effects. Mirrors the supported-feature
    /// probing done in `SDLFFDevice::CreateEffects`. The caller provides a
    /// bitmask of the supported effect kinds and, optionally, an
    /// autocenter-capable flag.
    pub fn create_effects(&mut self, supported: u32, has_autocenter: bool) {
        const HAPTIC_CONSTANT: u32 = 1 << 0;
        const HAPTIC_SPRING: u32   = 1 << 3;
        const HAPTIC_DAMPER: u32   = 1 << 4;
        const HAPTIC_FRICTION: u32 = 1 << 5;

        if supported & HAPTIC_CONSTANT != 0 {
            self.constant.kind = HAPTIC_CONSTANT;
            self.constant.length = u32::MAX; // INFINITY
            self.constant_id = 1;
        }
        if supported & HAPTIC_SPRING != 0 {
            self.spring.kind = HAPTIC_SPRING;
            self.spring.length = u32::MAX;
            self.spring_id = 2;
        }
        if supported & HAPTIC_DAMPER != 0 {
            self.damper.kind = HAPTIC_DAMPER;
            self.damper.length = u32::MAX;
            self.damper_id = 3;
        }
        if supported & HAPTIC_FRICTION != 0 {
            self.friction.kind = HAPTIC_FRICTION;
            self.friction.length = u32::MAX;
            self.friction_id = 4;
        }
        self.autocenter_supported = has_autocenter;
        self.haptic_open = true;
    }

    pub fn destroy_effects(&mut self) {
        if self.friction_id >= 0 {
            self.friction_id = -1;
            self.friction_running = false;
        }
        if self.damper_id >= 0 {
            self.damper_id = -1;
            self.damper_running = false;
        }
        if self.spring_id >= 0 {
            self.spring_id = -1;
            self.spring_running = false;
        }
        if self.constant_id >= 0 {
            self.constant_id = -1;
            self.constant_running = false;
        }
        self.haptic_open = false;
    }
}

impl Default for UsbPadSdlFF {
    fn default() -> Self { Self::new() }
}

impl FFDevice for UsbPadSdlFF {
    fn set_constant_force(&mut self, level: i16) {
        if self.constant_id < 0 { return; }
        let new_level = clamp(level, i16::MIN, i16::MAX);
        if self.constant.constant_level != new_level {
            self.constant.constant_level = new_level;
        }
        if !self.constant_running || self.use_dropout_workaround {
            self.constant_running = true;
        }
    }
    fn set_spring_force(&mut self, ff: &ParsedFfData) {
        if self.spring_id < 0 { return; }
        self.spring.left_sat[0]   = clamp(ff.condition.left_saturation, 0, u16::MAX);
        self.spring.left_coeff[0] = clamp(ff.condition.left_coeff, i16::MIN, i16::MAX);
        self.spring.right_sat[0]  = clamp(ff.condition.right_saturation, 0, u16::MAX);
        self.spring.right_coeff[0]= clamp(ff.condition.right_coeff, i16::MIN, i16::MAX);
        self.spring.deadband[0]   = clamp(ff.condition.deadband, 0, u16::MAX);
        self.spring.center[0]     = clamp(ff.condition.center, i16::MIN, i16::MAX);
        if !self.spring_running {
            self.spring_running = true;
        }
    }
    fn set_damper_force(&mut self, ff: &ParsedFfData) {
        if self.damper_id < 0 { return; }
        self.damper.left_sat[0]   = clamp(ff.condition.left_saturation, 0, u16::MAX);
        self.damper.left_coeff[0] = clamp(ff.condition.left_coeff, i16::MIN, i16::MAX);
        self.damper.right_sat[0]  = clamp(ff.condition.right_saturation, 0, u16::MAX);
        self.damper.right_coeff[0]= clamp(ff.condition.right_coeff, i16::MIN, i16::MAX);
        self.damper.deadband[0]   = clamp(ff.condition.deadband, 0, u16::MAX);
        self.damper.center[0]     = clamp(ff.condition.center, i16::MIN, i16::MAX);
        if !self.damper_running {
            self.damper_running = true;
        }
    }
    fn set_friction_force(&mut self, ff: &ParsedFfData) {
        if self.friction_id < 0 { return; }
        self.friction.left_sat[0]   = clamp(ff.condition.left_saturation, 0, u16::MAX);
        self.friction.left_coeff[0] = clamp(ff.condition.left_coeff, i16::MIN, i16::MAX);
        self.friction.right_sat[0]  = clamp(ff.condition.right_saturation, 0, u16::MAX);
        self.friction.right_coeff[0]= clamp(ff.condition.right_coeff, i16::MIN, i16::MAX);
        self.friction.deadband[0]   = clamp(ff.condition.deadband, 0, u16::MAX);
        self.friction.center[0]     = clamp(ff.condition.center, i16::MIN, i16::MAX);
        if !self.friction_running {
            self.friction_running = true;
        }
    }
    fn set_auto_center(&mut self, value: i32) {
        if self.autocenter_supported {
            // SDL_SetHapticAutocenter would be called here.
            let _ = value;
        }
    }
    fn disable_force(&mut self, effect: EffectID) {
        match effect {
            EffectID::Constant => self.constant_running = false,
            EffectID::Spring => self.spring_running = false,
            EffectID::Damper => self.damper_running = false,
            EffectID::Friction => self.friction_running = false,
            EffectID::Rumble | EffectID::Unknown => {}
        }
    }
}

impl UsbPadSdlFF {
    pub fn init(&mut self) {
        self.haptic_open = false;
    }
    pub fn shutdown(&mut self) {
        self.destroy_effects();
    }
    pub fn update(&mut self) {
        // No periodic work: SDL is event driven.
    }
}

// ---------------------------------------------------------------------------
// UsbRealPlay
// ---------------------------------------------------------------------------

pub mod realplay_cid {
    pub const CID_RP_DPAD_UP: u32 = 0;
    pub const CID_RP_DPAD_DOWN: u32 = 1;
    pub const CID_RP_DPAD_LEFT: u32 = 2;
    pub const CID_RP_DPAD_RIGHT: u32 = 3;
    pub const CID_RP_RED: u32 = 4;
    pub const CID_RP_GREEN: u32 = 5;
    pub const CID_RP_YELLOW: u32 = 6;
    pub const CID_RP_BLUE: u32 = 7;
    pub const CID_RP_ACC_X: u32 = 8;
    pub const CID_RP_ACC_Y: u32 = 9;
    pub const CID_RP_ACC_Z: u32 = 10;
}

#[derive(Debug, Clone, Default)]
pub struct RealPlayReport {
    pub dpad_up: bool,
    pub dpad_down: bool,
    pub dpad_left: bool,
    pub dpad_right: bool,
    pub btn_red: bool,
    pub btn_green: bool,
    pub btn_yellow: bool,
    pub btn_blue: bool,
    pub acc_x: u16,
    pub acc_y: u16,
    pub acc_z: u16,
}

pub struct UsbRealPlay {
    port: u32,
    kind: u32,
    state: u8,
    data: RealPlayReport,
    invert_x: bool,
    invert_y: bool,
    invert_z: bool,
}

impl UsbRealPlay {
    pub fn new(port: u32, kind: u32) -> Self {
        Self {
            port,
            kind,
            state: 0,
            data: RealPlayReport::default(),
            invert_x: false,
            invert_y: false,
            invert_z: false,
        }
    }

    pub fn port(&self) -> u32 { self.port }
    pub fn kind(&self) -> u32 { self.kind }

    pub fn update_settings(&mut self, invert_x: bool, invert_y: bool, invert_z: bool) {
        self.invert_x = invert_x;
        self.invert_y = invert_y;
        self.invert_z = invert_z;
    }

    pub fn get_binding_value(&self, bind_index: u32) -> f32 {
        use realplay_cid::*;
        match bind_index {
            CID_RP_DPAD_UP => f32::from(self.data.dpad_up as u8),
            CID_RP_DPAD_DOWN => f32::from(self.data.dpad_down as u8),
            CID_RP_DPAD_LEFT => f32::from(self.data.dpad_left as u8),
            CID_RP_DPAD_RIGHT => f32::from(self.data.dpad_right as u8),
            CID_RP_RED => f32::from(self.data.btn_red as u8),
            CID_RP_GREEN => f32::from(self.data.btn_green as u8),
            CID_RP_YELLOW => f32::from(self.data.btn_yellow as u8),
            CID_RP_BLUE => f32::from(self.data.btn_blue as u8),
            _ => 0.0,
        }
    }

    pub fn set_binding_value(&mut self, bind_index: u32, value: f32) {
        use realplay_cid::*;
        match bind_index {
            CID_RP_DPAD_UP => self.data.dpad_up = to_button(value),
            CID_RP_DPAD_DOWN => self.data.dpad_down = to_button(value),
            CID_RP_DPAD_LEFT => self.data.dpad_left = to_button(value),
            CID_RP_DPAD_RIGHT => self.data.dpad_right = to_button(value),
            CID_RP_RED => self.data.btn_red = to_button(value),
            CID_RP_GREEN => self.data.btn_green = to_button(value),
            CID_RP_YELLOW => self.data.btn_yellow = to_button(value),
            CID_RP_BLUE => self.data.btn_blue = to_button(value),
            CID_RP_ACC_X => self.data.acc_x = to_axis_u12(value, self.invert_x),
            CID_RP_ACC_Y => self.data.acc_y = to_axis_u12(value, self.invert_y),
            CID_RP_ACC_Z => self.data.acc_z = to_axis_u12(value, self.invert_z),
            _ => {}
        }
    }

    /// `USBPad::Update` analogue for endpoint 1. Mirrors the C++ behaviour
    /// of toggling bit 0 of the first byte to defeat a "disconnected"
    /// protection check in some games.
    pub fn update(&mut self, buffer: &mut [u8]) -> usize {
        let bytes = realplay_report_to_bytes(&self.data);
        let len = min(bytes.len(), buffer.len());
        buffer[..len].copy_from_slice(&bytes[..len]);
        if let Some(first) = buffer.first_mut() {
            *first ^= self.state;
        }
        self.state ^= 1;
        len
    }

    pub fn bindings() -> &'static [InputBindingInfo] {
        use realplay_cid::*;
        &[
            InputBindingInfo { name: "DPadUp", display_name: "D-Pad Up", icon: None, kind: BindingType::Button, id: CID_RP_DPAD_UP, generic: 0 },
            InputBindingInfo { name: "DPadDown", display_name: "D-Pad Down", icon: None, kind: BindingType::Button, id: CID_RP_DPAD_DOWN, generic: 0 },
            InputBindingInfo { name: "DPadLeft", display_name: "D-Pad Left", icon: None, kind: BindingType::Button, id: CID_RP_DPAD_LEFT, generic: 0 },
            InputBindingInfo { name: "DPadRight", display_name: "D-Pad Right", icon: None, kind: BindingType::Button, id: CID_RP_DPAD_RIGHT, generic: 0 },
            InputBindingInfo { name: "Red", display_name: "Red", icon: None, kind: BindingType::Button, id: CID_RP_RED, generic: 0 },
            InputBindingInfo { name: "Green", display_name: "Green", icon: None, kind: BindingType::Button, id: CID_RP_GREEN, generic: 0 },
            InputBindingInfo { name: "Yellow", display_name: "Yellow", icon: None, kind: BindingType::Button, id: CID_RP_YELLOW, generic: 0 },
            InputBindingInfo { name: "Blue", display_name: "Blue", icon: None, kind: BindingType::Button, id: CID_RP_BLUE, generic: 0 },
            InputBindingInfo { name: "AccelX", display_name: "Accel X", icon: None, kind: BindingType::Axis, id: CID_RP_ACC_X, generic: 0 },
            InputBindingInfo { name: "AccelY", display_name: "Accel Y", icon: None, kind: BindingType::Axis, id: CID_RP_ACC_Y, generic: 0 },
            InputBindingInfo { name: "AccelZ", display_name: "Accel Z", icon: None, kind: BindingType::Axis, id: CID_RP_ACC_Z, generic: 0 },
        ]
    }

    pub fn settings() -> &'static [SettingInfo] {
        &[
            SettingInfo { kind: SettingType::Boolean, name: "invert_x_axis", display_name: "Invert X axis", description: "Invert X axis", default_value: "false", min_value: None, max_value: None, step_value: None, format: None },
            SettingInfo { kind: SettingType::Boolean, name: "invert_y_axis", display_name: "Invert Y axis", description: "Invert Y axis", default_value: "false", min_value: None, max_value: None, step_value: None, format: None },
            SettingInfo { kind: SettingType::Boolean, name: "invert_z_axis", display_name: "Invert Z axis", description: "Invert Z axis", default_value: "false", min_value: None, max_value: None, step_value: None, format: None },
        ]
    }

    pub fn sub_types() -> &'static [&'static str] {
        &["RealPlay Racing", "RealPlay Sphere", "RealPlay Golf", "RealPlay Pool"]
    }

    pub fn init(&mut self) { self.state = 0; }
    pub fn shutdown(&mut self) {}
    pub fn update_pad(&mut self) { /* periodic hook */ }
}

fn realplay_report_to_bytes(r: &RealPlayReport) -> [u8; 16] {
    let mut out = [0u8; 16];
    // X/Y/Z are 12-bit, packed little-endian as 16-bit LE.
    out[0]  = (r.acc_x & 0xFF) as u8;
    out[1]  = ((r.acc_x >> 8) & 0x0F) as u8;
    out[2]  = (r.acc_y & 0xFF) as u8;
    out[3]  = ((r.acc_y >> 8) & 0x0F) as u8;
    out[4]  = (r.acc_z & 0xFF) as u8;
    out[5]  = ((r.acc_z >> 8) & 0x0F) as u8;
    out[6]  = 0; // dpad - not part of this struct
    let mut buttons: u8 = 0;
    buttons |= (r.dpad_up as u8)    << 0;
    buttons |= (r.dpad_down as u8)  << 1;
    buttons |= (r.dpad_left as u8)  << 2;
    buttons |= (r.dpad_right as u8) << 3;
    buttons |= (r.btn_red as u8)    << 4;
    buttons |= (r.btn_green as u8)  << 5;
    buttons |= (r.btn_yellow as u8) << 6;
    buttons |= (r.btn_blue as u8)   << 7;
    out[7]  = buttons;
    out
}

// ---------------------------------------------------------------------------
// UsbSeamic
// ---------------------------------------------------------------------------

pub mod seamic_cid {
    pub const CID_STEERING_L: u32 = 0;
    pub const CID_STEERING_R: u32 = 1;
    pub const CID_THROTTLE: u32 = 2;
    pub const CID_BRAKE: u32 = 3;
    pub const CID_BUTTON0: u32 = 4;
    pub const CID_BUTTON1: u32 = 5;
    pub const CID_BUTTON2: u32 = 6;
    pub const CID_BUTTON3: u32 = 7;
    pub const CID_BUTTON4: u32 = 8;
    pub const CID_BUTTON5: u32 = 9;
    pub const CID_BUTTON6: u32 = 10;
    pub const CID_BUTTON7: u32 = 11;
    pub const CID_BUTTON8: u32 = 12;
    pub const CID_BUTTON9: u32 = 13;
    pub const CID_DPAD_UP: u32 = 14;
    pub const CID_DPAD_DOWN: u32 = 15;
    pub const CID_DPAD_LEFT: u32 = 16;
    pub const CID_DPAD_RIGHT: u32 = 17;
}

#[derive(Debug, Clone, Default)]
pub struct SeamicReport {
    pub stick_left: u8,
    pub stick_right: u8,
    pub stick_up: u8,
    pub stick_down: u8,
    pub buttons: u16,
    pub dpad_up: bool,
    pub dpad_down: bool,
    pub dpad_left: bool,
    pub dpad_right: bool,
}

pub struct UsbSeamic {
    port: u32,
    data: SeamicReport,
}

impl UsbSeamic {
    pub fn new(port: u32) -> Self {
        Self { port, data: SeamicReport::default() }
    }

    pub fn port(&self) -> u32 { self.port }
    pub fn data(&self) -> &SeamicReport { &self.data }

    pub fn reset(&mut self) {
        self.data = SeamicReport::default();
    }

    pub fn token_in(&self, buffer: &mut [u8]) -> i32 {
        let bytes = seamic_report_to_bytes(&self.data);
        let len = min(bytes.len(), buffer.len());
        buffer[..len].copy_from_slice(&bytes[..len]);
        len as i32
    }

    pub fn token_out(&mut self, data: &[u8]) {
        // The original driver does not interpret OUT data; it is forwarded
        // to the underlying microphone device and to the pad state. The
        // translation keeps the signature but ignores the payload.
        let _ = data;
    }

    pub fn bindings() -> &'static [InputBindingInfo] {
        use seamic_cid::*;
        &[
            InputBindingInfo { name: "StickLeft", display_name: "Stick Left",  icon: None, kind: BindingType::HalfAxis, id: CID_STEERING_L, generic: 0 },
            InputBindingInfo { name: "StickRight",display_name: "Stick Right", icon: None, kind: BindingType::HalfAxis, id: CID_STEERING_R, generic: 0 },
            InputBindingInfo { name: "StickUp",   display_name: "Stick Up",    icon: None, kind: BindingType::HalfAxis, id: CID_THROTTLE,   generic: 0 },
            InputBindingInfo { name: "StickDown", display_name: "Stick Down",  icon: None, kind: BindingType::HalfAxis, id: CID_BRAKE,      generic: 0 },
            InputBindingInfo { name: "A",         display_name: "A",           icon: None, kind: BindingType::Button,   id: CID_BUTTON0,     generic: 0 },
            InputBindingInfo { name: "B",         display_name: "B",           icon: None, kind: BindingType::Button,   id: CID_BUTTON1,     generic: 0 },
            InputBindingInfo { name: "C",         display_name: "C",           icon: None, kind: BindingType::Button,   id: CID_BUTTON2,     generic: 0 },
            InputBindingInfo { name: "X",         display_name: "X",           icon: None, kind: BindingType::Button,   id: CID_BUTTON3,     generic: 0 },
            InputBindingInfo { name: "Y",         display_name: "Y",           icon: None, kind: BindingType::Button,   id: CID_BUTTON4,     generic: 0 },
            InputBindingInfo { name: "Z",         display_name: "Z",           icon: None, kind: BindingType::Button,   id: CID_BUTTON5,     generic: 0 },
            InputBindingInfo { name: "L",         display_name: "L",           icon: None, kind: BindingType::Button,   id: CID_BUTTON6,     generic: 0 },
            InputBindingInfo { name: "R",         display_name: "R",           icon: None, kind: BindingType::Button,   id: CID_BUTTON7,     generic: 0 },
            InputBindingInfo { name: "Select",    display_name: "Select",      icon: None, kind: BindingType::Button,   id: CID_BUTTON8,     generic: 0 },
            InputBindingInfo { name: "Start",     display_name: "Start",       icon: None, kind: BindingType::Button,   id: CID_BUTTON9,     generic: 0 },
            InputBindingInfo { name: "DPadUp",    display_name: "D-Pad Up",    icon: None, kind: BindingType::Button,   id: CID_DPAD_UP,     generic: 0 },
            InputBindingInfo { name: "DPadDown",  display_name: "D-Pad Down",  icon: None, kind: BindingType::Button,   id: CID_DPAD_DOWN,   generic: 0 },
            InputBindingInfo { name: "DPadLeft",  display_name: "D-Pad Left",  icon: None, kind: BindingType::Button,   id: CID_DPAD_LEFT,   generic: 0 },
            InputBindingInfo { name: "DPadRight", display_name: "D-Pad Right", icon: None, kind: BindingType::Button,   id: CID_DPAD_RIGHT,  generic: 0 },
        ]
    }

    pub fn settings() -> &'static [SettingInfo] {
        &[
            SettingInfo { kind: SettingType::StringList, name: "input_device_name", display_name: "Input Device", description: "Selects the device to read audio from.", default_value: "", min_value: None, max_value: None, step_value: None, format: None },
            SettingInfo { kind: SettingType::Integer,     name: "input_latency",     display_name: "Input Latency", description: "Specifies the latency to the host input device.", default_value: "20", min_value: Some("1"), max_value: Some("1000"), step_value: Some("1"), format: Some("%dms") },
        ]
    }

    pub fn init(&mut self) { self.reset(); }
    pub fn shutdown(&mut self) {}
    pub fn update(&mut self) { /* periodic hook */ }
}

fn seamic_report_to_bytes(r: &SeamicReport) -> [u8; 8] {
    let mut out = [0u8; 8];
    out[0] = r.stick_left;
    out[1] = r.stick_right;
    out[2] = r.stick_up;
    out[3] = r.stick_down;
    out[4] = (r.buttons & 0xFF) as u8;
    out[5] = ((r.buttons >> 8) & 0x03) as u8;
    out[5] |= (r.dpad_up as u8)    << 2;
    out[5] |= (r.dpad_down as u8)  << 3;
    out[5] |= (r.dpad_left as u8)  << 4;
    out[5] |= (r.dpad_right as u8) << 5;
    out
}

// ---------------------------------------------------------------------------
// UsbTrain
// ---------------------------------------------------------------------------

pub mod train_cid {
    pub const CID_TC_POWER: u32 = 0;
    pub const CID_TC_BRAKE: u32 = 1;
    pub const CID_TC_UP: u32 = 2;
    pub const CID_TC_RIGHT: u32 = 3;
    pub const CID_TC_DOWN: u32 = 4;
    pub const CID_TC_LEFT: u32 = 5;
    pub const CID_TC_B: u32 = 6;
    pub const CID_TC_A: u32 = 7;
    pub const CID_TC_C: u32 = 8;
    pub const CID_TC_D: u32 = 9;
    pub const CID_TC_SELECT: u32 = 10;
    pub const CID_TC_START: u32 = 11;
    pub const CID_TC_CAMERA: u32 = 12;
    pub const CID_TC_L: u32 = CID_TC_C;
    pub const CID_TC_R: u32 = CID_TC_D;
    pub const CID_TC_ATS: u32 = CID_TC_D;
    pub const CID_TC_CLOSE: u32 = CID_TC_CAMERA;
    pub const CID_TC_POWER_UP: u32 = 13;
    pub const CID_TC_POWER_DOWN: u32 = 14;
    pub const CID_TC_REVERSER_UP: u32 = 15;
    pub const CID_TC_REVERSER_DOWN: u32 = 16;
    pub const BUTTONS_OFFSET: u32 = CID_TC_B;
}

#[derive(Debug, Clone, Default)]
pub struct TrainData {
    pub power: u32,
    pub brake: u32,
    pub hat_up: u8,
    pub hat_down: u8,
    pub hat_left: u8,
    pub hat_right: u8,
    pub hatswitch: u8,
    pub buttons: u16,
}

pub struct UsbTrain {
    port: u32,
    kind: u32,
    data: TrainData,
    prev_buttons: u16,
    handle: i32,
    reverser: i32,
    last_handle: i32,
    last_reverser: i32,
    passthrough: bool,
    power_notches: i32,
    brake_notches: i32,
}

impl UsbTrain {
    pub fn new(port: u32, kind: u32) -> Self {
        Self {
            port,
            kind,
            data: TrainData { power: 0, brake: 0, hatswitch: 8, ..Default::default() },
            prev_buttons: 0,
            handle: 0,
            reverser: 0,
            last_handle: -1,
            last_reverser: -1,
            passthrough: false,
            power_notches: 5,
            brake_notches: 8,
        }
    }

    pub fn port(&self) -> u32 { self.port }
    pub fn kind(&self) -> u32 { self.kind }

    pub fn update_settings(&mut self, passthrough: bool, power_notches: i32, brake_notches: i32) {
        self.passthrough = passthrough;
        self.power_notches = power_notches;
        self.brake_notches = brake_notches;
    }

    pub fn reset(&mut self) {
        self.data.power = 0;
        self.data.brake = 0;
    }

    pub fn update_hat_switch(&mut self) {
        let u = self.data.hat_up != 0;
        let d = self.data.hat_down != 0;
        let l = self.data.hat_left != 0;
        let r = self.data.hat_right != 0;
        self.data.hatswitch = match (u, d, l, r) {
            (true, false, false, true) => 1,
            (false, false, false, true) => 2,
            (false, true, false, true) => 3,
            (false, true, false, false) => 4,
            (false, true, true, false) => 5,
            (false, false, true, false) => 6,
            (true, false, true, false) => 7,
            (true, false, false, false) => 0,
            _ => 8,
        };
    }

    fn button_mask(bind_index: u32) -> u16 {
        1u16 << (bind_index - train_cid::BUTTONS_OFFSET)
    }

    fn button_at(value: u16, bind_index: u32) -> u16 {
        value & Self::button_mask(bind_index)
    }

    pub fn get_binding_value(&self, bind_index: u32) -> f32 {
        use train_cid::*;
        match bind_index {
            CID_TC_POWER => self.data.power as f32 / 255.0,
            CID_TC_BRAKE => self.data.brake as f32 / 255.0,
            CID_TC_UP => f32::from(self.data.hat_up),
            CID_TC_DOWN => f32::from(self.data.hat_down),
            CID_TC_LEFT => f32::from(self.data.hat_left),
            CID_TC_RIGHT => f32::from(self.data.hat_right),
            CID_TC_A | CID_TC_B | CID_TC_C | CID_TC_D | CID_TC_SELECT | CID_TC_START
            | CID_TC_CAMERA | CID_TC_POWER_UP | CID_TC_POWER_DOWN
            | CID_TC_REVERSER_UP | CID_TC_REVERSER_DOWN => {
                if Self::button_at(self.data.buttons, bind_index) != 0 { 1.0 } else { 0.0 }
            }
            _ => 0.0,
        }
    }

    pub fn set_binding_value(&mut self, bind_index: u32, value: f32) {
        use train_cid::*;
        match bind_index {
            CID_TC_POWER => {
                self.data.power = clamp(lroundf(value * 255.0), 0, 255) as u32;
            }
            CID_TC_BRAKE => {
                self.data.brake = clamp(lroundf(value * 255.0), 0, 255) as u32;
            }
            CID_TC_UP => { self.data.hat_up = to_u8(value); self.update_hat_switch(); }
            CID_TC_DOWN => { self.data.hat_down = to_u8(value); self.update_hat_switch(); }
            CID_TC_LEFT => { self.data.hat_left = to_u8(value); self.update_hat_switch(); }
            CID_TC_RIGHT => { self.data.hat_right = to_u8(value); self.update_hat_switch(); }
            CID_TC_A | CID_TC_B | CID_TC_C | CID_TC_D | CID_TC_SELECT | CID_TC_START
            | CID_TC_CAMERA | CID_TC_POWER_UP | CID_TC_POWER_DOWN
            | CID_TC_REVERSER_UP | CID_TC_REVERSER_DOWN => {
                let mask = Self::button_mask(bind_index);
                if value >= 0.5 {
                    self.data.buttons |= mask;
                } else {
                    self.data.buttons &= !mask;
                }
            }
            _ => {}
        }
    }

    pub fn sub_types() -> &'static [&'static str] {
        &["Type 2", "Shinkansen", "Ryojōhen", "Train Mascon", "Master Controller"]
    }

    pub fn bindings(subtype: u32) -> &'static [InputBindingInfo] {
        use train_cid::*;
        match subtype {
            train_type::TRAIN_TYPE2 | train_type::TRAIN_SHINKANSEN => &[
                InputBindingInfo { name: "Power",  display_name: "Power",       icon: Some("AXIS_DOWN"), kind: BindingType::Axis,   id: CID_TC_POWER,  generic: 0 },
                InputBindingInfo { name: "Brake",  display_name: "Brake",       icon: Some("AXIS_UP"),   kind: BindingType::Axis,   id: CID_TC_BRAKE,  generic: 0 },
                InputBindingInfo { name: "Up",     display_name: "D-Pad Up",    icon: Some("DPAD_UP"),   kind: BindingType::Button, id: CID_TC_UP,     generic: 0 },
                InputBindingInfo { name: "Down",   display_name: "D-Pad Down",  icon: Some("DPAD_DOWN"), kind: BindingType::Button, id: CID_TC_DOWN,   generic: 0 },
                InputBindingInfo { name: "Left",   display_name: "D-Pad Left",  icon: Some("DPAD_LEFT"), kind: BindingType::Button, id: CID_TC_LEFT,   generic: 0 },
                InputBindingInfo { name: "Right",  display_name: "D-Pad Right", icon: Some("DPAD_RIGHT"),kind: BindingType::Button, id: CID_TC_RIGHT,  generic: 0 },
                InputBindingInfo { name: "A",      display_name: "A Button",    icon: Some("KEY_A"),     kind: BindingType::Button, id: CID_TC_A,      generic: 0 },
                InputBindingInfo { name: "B",      display_name: "B Button",    icon: Some("KEY_B"),     kind: BindingType::Button, id: CID_TC_B,      generic: 0 },
                InputBindingInfo { name: "C",      display_name: "C Button",    icon: Some("KEY_C"),     kind: BindingType::Button, id: CID_TC_C,      generic: 0 },
                InputBindingInfo { name: "D",      display_name: "D Button",    icon: Some("KEY_D"),     kind: BindingType::Button, id: CID_TC_D,      generic: 0 },
                InputBindingInfo { name: "Select", display_name: "Select",      icon: Some("SELECT"),    kind: BindingType::Button, id: CID_TC_SELECT, generic: 0 },
                InputBindingInfo { name: "Start",  display_name: "Start",       icon: Some("START"),     kind: BindingType::Button, id: CID_TC_START,  generic: 0 },
            ],
            train_type::TRAIN_RYOJOUHEN => &[
                InputBindingInfo { name: "Power",       display_name: "Power",         icon: Some("AXIS_DOWN"), kind: BindingType::Axis,   id: CID_TC_POWER,  generic: 0 },
                InputBindingInfo { name: "Brake",       display_name: "Brake",         icon: Some("AXIS_UP"),   kind: BindingType::Axis,   id: CID_TC_BRAKE,  generic: 0 },
                InputBindingInfo { name: "Up",          display_name: "D-Pad Up",      icon: Some("DPAD_UP"),   kind: BindingType::Button, id: CID_TC_UP,     generic: 0 },
                InputBindingInfo { name: "Down",        display_name: "D-Pad Down",    icon: Some("DPAD_DOWN"), kind: BindingType::Button, id: CID_TC_DOWN,   generic: 0 },
                InputBindingInfo { name: "Left",        display_name: "D-Pad Left",    icon: Some("DPAD_LEFT"), kind: BindingType::Button, id: CID_TC_LEFT,   generic: 0 },
                InputBindingInfo { name: "Right",       display_name: "D-Pad Right",   icon: Some("DPAD_RIGHT"),kind: BindingType::Button, id: CID_TC_RIGHT,  generic: 0 },
                InputBindingInfo { name: "Announce",    display_name: "Announce",      icon: Some("KEY_B"),     kind: BindingType::Button, id: CID_TC_A,      generic: 0 },
                InputBindingInfo { name: "Horn",        display_name: "Horn",          icon: Some("KEY_A"),     kind: BindingType::Button, id: CID_TC_B,      generic: 0 },
                InputBindingInfo { name: "LeftDoor",    display_name: "Left Door",     icon: Some("KEY_L"),     kind: BindingType::Button, id: CID_TC_L,      generic: 0 },
                InputBindingInfo { name: "RightDoor",   display_name: "Right Door",    icon: Some("KEY_R"),     kind: BindingType::Button, id: CID_TC_R,      generic: 0 },
                InputBindingInfo { name: "Camera",      display_name: "Camera Button", icon: Some("CAMERA"),    kind: BindingType::Button, id: CID_TC_CAMERA, generic: 0 },
                InputBindingInfo { name: "Select",      display_name: "Select",        icon: Some("SELECT"),    kind: BindingType::Button, id: CID_TC_SELECT, generic: 0 },
                InputBindingInfo { name: "Start",       display_name: "Start",         icon: Some("START"),     kind: BindingType::Button, id: CID_TC_START,  generic: 0 },
            ],
            train_type::TRAIN_MASCON => &[
                InputBindingInfo { name: "PowerUp",     display_name: "Power Up",      icon: None, kind: BindingType::Button, id: CID_TC_POWER_UP,     generic: 0 },
                InputBindingInfo { name: "PowerDown",   display_name: "Power Down",    icon: None, kind: BindingType::Button, id: CID_TC_POWER_DOWN,   generic: 0 },
                InputBindingInfo { name: "ReverserUp",  display_name: "Reverser Up",   icon: None, kind: BindingType::Button, id: CID_TC_REVERSER_UP,  generic: 0 },
                InputBindingInfo { name: "ReverserDown",display_name: "Reverser Down", icon: None, kind: BindingType::Button, id: CID_TC_REVERSER_DOWN,generic: 0 },
                InputBindingInfo { name: "Up",          display_name: "D-Pad Up",      icon: Some("DPAD_UP"),   kind: BindingType::Button, id: CID_TC_UP,     generic: 0 },
                InputBindingInfo { name: "Down",        display_name: "D-Pad Down",    icon: Some("DPAD_DOWN"), kind: BindingType::Button, id: CID_TC_DOWN,   generic: 0 },
                InputBindingInfo { name: "Left",        display_name: "D-Pad Left",    icon: Some("DPAD_LEFT"), kind: BindingType::Button, id: CID_TC_LEFT,   generic: 0 },
                InputBindingInfo { name: "Right",       display_name: "D-Pad Right",   icon: Some("DPAD_RIGHT"),kind: BindingType::Button, id: CID_TC_RIGHT,  generic: 0 },
                InputBindingInfo { name: "ATS",         display_name: "ATS",           icon: None, kind: BindingType::Button, id: CID_TC_ATS,    generic: 0 },
                InputBindingInfo { name: "Close",       display_name: "Close",         icon: None, kind: BindingType::Button, id: CID_TC_CLOSE,  generic: 0 },
                InputBindingInfo { name: "A",           display_name: "A Button",      icon: Some("KEY_A"),     kind: BindingType::Button, id: CID_TC_A,      generic: 0 },
                InputBindingInfo { name: "B",           display_name: "B Button",      icon: Some("KEY_B"),     kind: BindingType::Button, id: CID_TC_B,      generic: 0 },
                InputBindingInfo { name: "C",           display_name: "C Button",      icon: Some("KEY_C"),     kind: BindingType::Button, id: CID_TC_C,      generic: 0 },
                InputBindingInfo { name: "Select",      display_name: "Select",        icon: Some("SELECT"),    kind: BindingType::Button, id: CID_TC_SELECT, generic: 0 },
                InputBindingInfo { name: "Start",       display_name: "Start",         icon: Some("START"),     kind: BindingType::Button, id: CID_TC_START,  generic: 0 },
            ],
            train_type::MASTER_CONTROLLER => &[
                InputBindingInfo { name: "PowerUp",     display_name: "Power Up",      icon: None, kind: BindingType::Button, id: CID_TC_POWER_UP,     generic: 0 },
                InputBindingInfo { name: "PowerDown",   display_name: "Power Down",    icon: None, kind: BindingType::Button, id: CID_TC_POWER_DOWN,   generic: 0 },
                InputBindingInfo { name: "ReverserUp",  display_name: "Reverser Up",   icon: None, kind: BindingType::Button, id: CID_TC_REVERSER_UP,  generic: 0 },
                InputBindingInfo { name: "ReverserDown",display_name: "Reverser Down", icon: None, kind: BindingType::Button, id: CID_TC_REVERSER_DOWN,generic: 0 },
                InputBindingInfo { name: "S",           display_name: "S",             icon: Some("KEY_S"), kind: BindingType::Button, id: CID_TC_D, generic: 0 },
                InputBindingInfo { name: "A",           display_name: "A",             icon: Some("KEY_A"), kind: BindingType::Button, id: CID_TC_A, generic: 0 },
                InputBindingInfo { name: "B",           display_name: "B",             icon: Some("KEY_B"), kind: BindingType::Button, id: CID_TC_B, generic: 0 },
                InputBindingInfo { name: "C",           display_name: "C",             icon: Some("KEY_C"), kind: BindingType::Button, id: CID_TC_C, generic: 0 },
            ],
            _ => &[],
        }
    }

    pub fn settings(subtype: u32) -> &'static [SettingInfo] {
        match subtype {
            train_type::TRAIN_TYPE2 | train_type::TRAIN_SHINKANSEN | train_type::TRAIN_RYOJOUHEN => &[
                SettingInfo { kind: SettingType::Boolean, name: "Passthrough", display_name: "Axes Passthrough", description: "Passes through the unprocessed input axis to the game.", default_value: "false", min_value: None, max_value: None, step_value: None, format: None },
            ],
            train_type::MASTER_CONTROLLER => &[
                SettingInfo { kind: SettingType::Integer, name: "power_notches", display_name: "Power notches", description: "Selects the number of power notches (3-6)", default_value: "5", min_value: Some("3"), max_value: Some("6"), step_value: None, format: None },
                SettingInfo { kind: SettingType::Integer, name: "brake_notches", display_name: "Brake notches", description: "Selects the number of brake notches (5-8)", default_value: "8", min_value: Some("5"), max_value: Some("8"), step_value: None, format: None },
            ],
            _ => &[],
        }
    }

    fn update_handles(&mut self, max_power: u8, max_brake: u8) {
        let cur = self.data.buttons;
        let prev = self.prev_buttons;
        if Self::button_at(prev, train_cid::CID_TC_POWER_UP) == 0
            && Self::button_at(cur, train_cid::CID_TC_POWER_UP) != 0
            && self.handle < (max_brake as i32) + 1 + (max_power as i32)
        {
            self.handle += 1;
        }
        if Self::button_at(prev, train_cid::CID_TC_POWER_DOWN) == 0
            && Self::button_at(cur, train_cid::CID_TC_POWER_DOWN) != 0
            && self.handle > 0
        {
            self.handle -= 1;
        }
        if Self::button_at(prev, train_cid::CID_TC_REVERSER_UP) == 0
            && Self::button_at(cur, train_cid::CID_TC_REVERSER_UP) != 0
            && self.reverser < 2
        {
            self.reverser += 1;
        }
        if Self::button_at(prev, train_cid::CID_TC_REVERSER_DOWN) == 0
            && Self::button_at(cur, train_cid::CID_TC_REVERSER_DOWN) != 0
            && self.reverser > 0
        {
            self.reverser -= 1;
        }
    }

    /// `USBPad::Update` analogue. Returns the report bytes for the given
    /// endpoint and the configured sub-type.
    pub fn update(&mut self, endpoint: u8, buffer: &mut [u8]) -> i32 {
        self.update_hat_switch();
        if self.kind < train_type::MASTER_CONTROLLER {
            if endpoint != 1 { return -1; }
        }
        match self.kind {
            train_type::TRAIN_TYPE2 => {
                let out = TrainConDataType2 {
                    control: 0x01,
                    brake: if self.passthrough { self.data.brake as u8 } else { dct01_brake(self.data.brake as u8) },
                    power: if self.passthrough { self.data.power as u8 } else { dct01_power(self.data.power as u8) },
                    horn: 0xFF,
                    hat: self.data.hatswitch,
                    buttons: dct01_buttons(self.data.buttons as u8),
                };
                let bytes = out.to_bytes();
                let len = min(bytes.len(), buffer.len());
                buffer[..len].copy_from_slice(&bytes[..len]);
                len as i32
            }
            train_type::TRAIN_SHINKANSEN => {
                let out = TrainConDataShinkansen {
                    brake: if self.passthrough { self.data.brake as u8 } else { dct02_brake(self.data.brake as u8) },
                    power: if self.passthrough { self.data.power as u8 } else { dct02_power(self.data.power as u8) },
                    horn: 0xFF,
                    hat: self.data.hatswitch,
                    buttons: dct02_buttons(self.data.data_buttons_u8()),
                };
                let bytes = out.to_bytes();
                let len = min(bytes.len(), buffer.len());
                buffer[..len].copy_from_slice(&bytes[..len]);
                len as i32
            }
            train_type::TRAIN_RYOJOUHEN => {
                let out = TrainConDataRyojouhen {
                    brake: if self.passthrough { self.data.brake as u8 } else { dct03_brake(self.data.brake as u8) },
                    power: if self.passthrough { self.data.power as u8 } else { dct03_power(self.data.power as u8) },
                    horn: 0xFF,
                    hat: self.data.hatswitch & 0x0F,
                    buttons: dct03_buttons(self.data.data_buttons_u8()),
                };
                let bytes = out.to_bytes();
                let len = min(bytes.len(), buffer.len());
                buffer[..len].copy_from_slice(&bytes[..len]);
                len as i32
            }
            train_type::TRAIN_MASCON => {
                self.update_handles(5, 6);
                self.prev_buttons = self.data.buttons;
                let out = TrainConDataTrainMascon {
                    one: 0x01,
                    handle: 1 + self.handle,
                    reverser: if self.reverser < 2 { if self.reverser == 0 { 1 } else { 0 } } else { self.reverser },
                    ats: Self::button_at(self.data.buttons, train_cid::CID_TC_ATS) != 0,
                    close: Self::button_at(self.data.buttons, train_cid::CID_TC_CLOSE) != 0,
                    button_a_soft: Self::button_at(self.data.buttons, train_cid::CID_TC_A) != 0,
                    button_a_hard: Self::button_at(self.data.buttons, train_cid::CID_TC_A) != 0,
                    button_b: Self::button_at(self.data.buttons, train_cid::CID_TC_B) != 0,
                    button_c: Self::button_at(self.data.buttons, train_cid::CID_TC_C) != 0,
                    start: Self::button_at(self.data.buttons, train_cid::CID_TC_START) != 0,
                    select: Self::button_at(self.data.buttons, train_cid::CID_TC_SELECT) != 0,
                    dpad_up: self.data.hat_up,
                    dpad_down: self.data.hat_down,
                    dpad_left: self.data.hat_left,
                    dpad_right: self.data.hat_right,
                };
                let bytes = out.to_bytes();
                let len = min(bytes.len(), buffer.len());
                buffer[..len].copy_from_slice(&bytes[..len]);
                len as i32
            }
            train_type::MASTER_CONTROLLER => {
                if endpoint == 1 { return -1; }
                if endpoint == 2 {
                    // Bulk OUT: re-emit current handle/reverser on next IN.
                    self.last_handle = -1;
                    self.last_reverser = -1;
                    return 0;
                }
                self.update_handles(self.power_notches as u8, self.brake_notches as u8);
                let mut data = [0u8; 100];
                let mut pos: usize = 0;
                if self.last_handle != self.handle {
                    let s = MC_HANDLE[self.handle as usize + 8 - self.brake_notches as usize];
                    let bytes = s.as_bytes();
                    let n = min(bytes.len(), data.len() - pos - 1);
                    data[pos..pos + n].copy_from_slice(&bytes[..n]);
                    pos += n;
                    data[pos] = 0x0d;
                    pos += 1;
                    self.last_handle = self.handle;
                }
                if self.last_reverser != self.reverser {
                    let s = MC_REVERSER[self.reverser as usize];
                    let bytes = s.as_bytes();
                    let n = min(bytes.len(), data.len() - pos - 1);
                    data[pos..pos + n].copy_from_slice(&bytes[..n]);
                    pos += n;
                    data[pos] = 0x0d;
                    pos += 1;
                    self.last_reverser = self.reverser;
                }
                for i in 0..4 {
                    let cur = Self::button_at(self.data.buttons, train_cid::BUTTONS_OFFSET + i as u32) != 0;
                    let prev = Self::button_at(self.prev_buttons, train_cid::BUTTONS_OFFSET + i as u32) != 0;
                    let label = if !prev && cur {
                        MC_BUTTON_PRESSED[i]
                    } else if prev && !cur {
                        MC_BUTTON_RELEASED[i]
                    } else {
                        ""
                    };
                    if !label.is_empty() {
                        let bytes = label.as_bytes();
                        let n = min(bytes.len(), data.len() - pos - 1);
                        data[pos..pos + n].copy_from_slice(&bytes[..n]);
                        pos += n;
                        data[pos] = 0x0d;
                        pos += 1;
                    }
                }
                self.prev_buttons = self.data.buttons;
                let len = min(pos, buffer.len());
                buffer[..len].copy_from_slice(&data[..len]);
                len as i32
            }
            _ => -1,
        }
    }

    pub fn init(&mut self) { self.reset(); }
    pub fn shutdown(&mut self) {}
    pub fn update_pad(&mut self) { self.update_hat_switch(); }
}

impl TrainData {
    fn data_buttons_u8(&self) -> u8 {
        (self.buttons & 0xFF) as u8
    }
}

#[derive(Debug, Clone, Copy)]
struct TrainConDataType2 {
    control: u8,
    brake: u8,
    power: u8,
    horn: u8,
    hat: u8,
    buttons: u8,
}

impl TrainConDataType2 { fn to_bytes(&self) -> [u8; 6] { [self.control, self.brake, self.power, self.horn, self.hat, self.buttons] } }

#[derive(Debug, Clone, Copy)]
struct TrainConDataShinkansen {
    brake: u8,
    power: u8,
    horn: u8,
    hat: u8,
    buttons: u8,
}

impl TrainConDataShinkansen { fn to_bytes(&self) -> [u8; 5] { [self.brake, self.power, self.horn, self.hat, self.buttons] } }

#[derive(Debug, Clone, Copy)]
struct TrainConDataRyojouhen {
    brake: u8,
    power: u8,
    horn: u8,
    hat: u8,
    buttons: u8,
}

impl TrainConDataRyojouhen { fn to_bytes(&self) -> [u8; 5] { [self.brake, self.power, self.horn, self.hat, self.buttons] } }

#[derive(Debug, Clone, Copy)]
struct TrainConDataTrainMascon {
    one: u8,
    handle: i32,
    reverser: i32,
    ats: bool,
    close: bool,
    button_a_soft: bool,
    button_a_hard: bool,
    button_b: bool,
    button_c: bool,
    start: bool,
    select: bool,
    dpad_up: u8,
    dpad_down: u8,
    dpad_left: u8,
    dpad_right: u8,
}

impl TrainConDataTrainMascon {
    fn to_bytes(&self) -> [u8; 16] {
        [
            self.one,
            (self.handle & 0xFF) as u8,
            ((self.handle >> 8) & 0xFF) as u8,
            (self.reverser & 0xFF) as u8,
            self.ats as u8,
            self.close as u8,
            self.button_a_soft as u8,
            self.button_a_hard as u8,
            self.button_b as u8,
            self.button_c as u8,
            self.start as u8,
            self.select as u8,
            self.dpad_up,
            self.dpad_down,
            self.dpad_left,
            self.dpad_right,
        ]
    }
}

const MC_HANDLE: [&str; 16] = [
    "EB-7", "EB-6", "EB-5", "EB-4", "B7", "B6", "B5", "B4", "B3", "B2", "B1", "Off", "P1", "P2", "P3", "P4",
];
const MC_REVERSER: [&str; 3] = ["F", "N", "R"];
const MC_BUTTON_PRESSED: [&str; 4] = ["A_P", "B_P", "C_P", "D_P"];
const MC_BUTTON_RELEASED: [&str; 4] = ["A_R", "B_R", "C_R", "D_R"];

fn dct01_power(v: u8) -> u8 {
    const NOTCHES: &[(u8, u8)] = &[
        (0xF8, 0x00), (0xC8, 0x21), (0x98, 0x3F), (0x58, 0x54), (0x28, 0x6D), (0x00, 0x81),
    ];
    for (a, b) in NOTCHES { if v >= *a { return *b; } }
    NOTCHES.last().unwrap().1
}

fn dct01_brake(v: u8) -> u8 {
    const NOTCHES: &[(u8, u8)] = &[
        (0xF8, 0xB9), (0xE6, 0xB5), (0xCA, 0xB2), (0xAE, 0xAF), (0x92, 0xA8),
        (0x76, 0xA2), (0x5A, 0x9A), (0x3E, 0x94), (0x22, 0x8A), (0x00, 0x79),
    ];
    for (a, b) in NOTCHES { if v >= *a { return *b; } }
    NOTCHES.last().unwrap().1
}

fn dct02_power(v: u8) -> u8 {
    const NOTCHES: &[(u8, u8)] = &[
        (0xF7, 0xFB), (0xE4, 0xE9), (0xD1, 0xD7), (0xBE, 0xC6), (0xAB, 0xB4),
        (0x98, 0xA2), (0x85, 0x90), (0x72, 0x7E), (0x5F, 0x6C), (0x4C, 0x5A),
        (0x39, 0x48), (0x26, 0x36), (0x13, 0x24), (0x00, 0x12),
    ];
    for (a, b) in NOTCHES { if v >= *a { return *b; } }
    NOTCHES.last().unwrap().1
}

fn dct02_brake(v: u8) -> u8 {
    const NOTCHES: &[(u8, u8)] = &[
        (0xF8, 0xFB), (0xCA, 0xDF), (0xAE, 0xC3), (0x92, 0xA7), (0x76, 0x8B),
        (0x5A, 0x70), (0x3E, 0x54), (0x22, 0x38), (0x00, 0x1C),
    ];
    for (a, b) in NOTCHES { if v >= *a { return *b; } }
    NOTCHES.last().unwrap().1
}

fn dct03_power(v: u8) -> u8 {
    const NOTCHES: &[(u8, u8)] = &[
        (0xC0, 0xF0), (0x90, 0xB4), (0x50, 0x78), (0x30, 0x3C), (0x00, 0x00),
    ];
    for (a, b) in NOTCHES { if v >= *a { return *b; } }
    NOTCHES.last().unwrap().1
}

fn dct03_brake(v: u8) -> u8 {
    if 0x18 >= v { return 0x23; }
    if v >= 0xF8 { return 0xD7; }
    let offset = 0x9 + (v as u16 / 85) as u8;
    (v as u16 / 5 * 4 + offset as u16) as u8
}

fn dct01_buttons(b: u8) -> u8 { b }

fn dct02_buttons(b: u8) -> u8 {
    let a = (b & 0x03) << 2;
    let cd = ((b & 0x08) >> 1) | ((b & 0x04) << 1);
    let ss = b & 0xC0;
    a | cd | ss
}

fn dct03_buttons(b: u8) -> u8 {
    let ab = b & 0x03;
    let cam = (b & 0x10) >> 4;
    let cd_ss = (b & 0xFC) << 1;
    ab | cam | cd_ss
}

// ---------------------------------------------------------------------------
// UsbTranceVibrator
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct TranceVibratorState {
    pub port: u32,
    pub motor: u8,
    pub leds: u8,
}

pub struct UsbTranceVibrator {
    state: TranceVibratorState,
}

impl UsbTranceVibrator {
    pub fn new(port: u32) -> Self {
        Self { state: TranceVibratorState { port, ..Default::default() } }
    }

    /// Vendor device OUT request. `value & 0xff` is the vibration level;
    /// `index & 0x7` selects the LEDs.
    pub fn handle_vendor_out(&mut self, value: i32, index: i32) {
        self.state.motor = (value & 0xFF) as u8;
        self.state.leds = (index & 0x7) as u8;
    }

    pub fn get_binding_value(&self, _bind_index: u32) -> f32 { 0.0 }
    pub fn set_binding_value(&mut self, _bind_index: u32, _value: f32) {}

    pub fn bindings() -> &'static [InputBindingInfo] {
        &[InputBindingInfo {
            name: "Motor",
            display_name: "Motor",
            icon: None,
            kind: BindingType::Motor,
            id: 0,
            generic: 0,
        }]
    }

    pub fn settings() -> &'static [SettingInfo] { &[] }

    pub fn init(&mut self) { self.state.motor = 0; self.state.leds = 0; }
    pub fn shutdown(&mut self) { self.state.motor = 0; }
    pub fn update(&mut self) { /* periodic hook */ }
}

// ---------------------------------------------------------------------------
// UsbTurntable (DJ Hero)
// ---------------------------------------------------------------------------

pub mod turntable_cid {
    pub const CID_DJ_CROSSFADER_LEFT: u32 = 0;
    pub const CID_DJ_CROSSFADER_RIGHT: u32 = 1;
    pub const CID_DJ_EFFECTSKNOB_LEFT: u32 = 2;
    pub const CID_DJ_EFFECTSKNOB_RIGHT: u32 = 3;
    pub const CID_DJ_LEFT_TURNTABLE_CW: u32 = 4;
    pub const CID_DJ_LEFT_TURNTABLE_CCW: u32 = 5;
    pub const CID_DJ_RIGHT_TURNTABLE_CW: u32 = 6;
    pub const CID_DJ_RIGHT_TURNTABLE_CCW: u32 = 7;
    pub const CID_DJ_DPAD_UP: u32 = 8;
    pub const CID_DJ_DPAD_DOWN: u32 = 9;
    pub const CID_DJ_DPAD_LEFT: u32 = 10;
    pub const CID_DJ_DPAD_RIGHT: u32 = 11;
    pub const CID_DJ_SQUARE: u32 = 12;
    pub const CID_DJ_CROSS: u32 = 13;
    pub const CID_DJ_CIRCLE: u32 = 14;
    pub const CID_DJ_TRIANGLE: u32 = 15;
    pub const CID_DJ_SELECT: u32 = 16;
    pub const CID_DJ_START: u32 = 17;
    pub const CID_DJ_RIGHT_GREEN: u32 = 18;
    pub const CID_DJ_RIGHT_RED: u32 = 19;
    pub const CID_DJ_RIGHT_BLUE: u32 = 20;
    pub const CID_DJ_LEFT_GREEN: u32 = 21;
    pub const CID_DJ_LEFT_RED: u32 = 22;
    pub const CID_DJ_LEFT_BLUE: u32 = 23;
}

#[derive(Debug, Clone, Default)]
pub struct TurntableData {
    pub crossfader_left: u32,
    pub crossfader_right: u32,
    pub effectsknob_left: u32,
    pub effectsknob_right: u32,
    pub left_turntable_cw: u32,
    pub left_turntable_ccw: u32,
    pub right_turntable_cw: u32,
    pub right_turntable_ccw: u32,
    pub hat_up: u8,
    pub hat_down: u8,
    pub hat_left: u8,
    pub hat_right: u8,
    pub hatswitch: u8,
    pub buttons: u32,
    pub euphoria_led_state: bool,
}

pub struct UsbTurntable {
    port: u32,
    data: TurntableData,
    turntable_multiplier: f32,
}

impl UsbTurntable {
    pub fn new(port: u32) -> Self {
        Self { port, data: TurntableData::default(), turntable_multiplier: 1.0 }
    }

    pub fn port(&self) -> u32 { self.port }
    pub fn data(&self) -> &TurntableData { &self.data }

    pub fn update_settings(&mut self, multiplier: f32) {
        self.turntable_multiplier = multiplier;
    }

    pub fn update_hat_switch(&mut self) {
        let u = self.data.hat_up != 0;
        let d = self.data.hat_down != 0;
        let l = self.data.hat_left != 0;
        let r = self.data.hat_right != 0;
        self.data.hatswitch = match (u, d, l, r) {
            (true, false, false, true) => 1,
            (false, false, false, true) => 2,
            (false, true, false, true) => 3,
            (false, true, false, false) => 4,
            (false, true, true, false) => 5,
            (false, false, true, false) => 6,
            (true, false, true, false) => 7,
            (true, false, false, false) => 0,
            _ => 8,
        };
    }

    pub fn set_euphoria_led_state(&mut self, state: bool) {
        self.data.euphoria_led_state = state;
    }

    pub fn get_binding_value(&self, bind_index: u32) -> f32 {
        use turntable_cid::*;
        match bind_index {
            CID_DJ_CROSSFADER_LEFT => self.data.crossfader_left as f32 / 512.0,
            CID_DJ_CROSSFADER_RIGHT => self.data.crossfader_right as f32 / 512.0,
            CID_DJ_EFFECTSKNOB_LEFT => self.data.effectsknob_left as f32 / 512.0,
            CID_DJ_EFFECTSKNOB_RIGHT => self.data.effectsknob_right as f32 / 512.0,
            CID_DJ_LEFT_TURNTABLE_CW => self.data.left_turntable_cw as f32 / 128.0,
            CID_DJ_LEFT_TURNTABLE_CCW => self.data.left_turntable_ccw as f32 / 128.0,
            CID_DJ_RIGHT_TURNTABLE_CW => self.data.right_turntable_cw as f32 / 128.0,
            CID_DJ_RIGHT_TURNTABLE_CCW => self.data.right_turntable_ccw as f32 / 128.0,
            CID_DJ_DPAD_UP => f32::from(self.data.hat_up),
            CID_DJ_DPAD_DOWN => f32::from(self.data.hat_down),
            CID_DJ_DPAD_LEFT => f32::from(self.data.hat_left),
            CID_DJ_DPAD_RIGHT => f32::from(self.data.hat_right),
            CID_DJ_SQUARE | CID_DJ_CROSS | CID_DJ_CIRCLE | CID_DJ_TRIANGLE
            | CID_DJ_SELECT | CID_DJ_START | CID_DJ_RIGHT_GREEN | CID_DJ_RIGHT_RED
            | CID_DJ_RIGHT_BLUE | CID_DJ_LEFT_GREEN | CID_DJ_LEFT_RED
            | CID_DJ_LEFT_BLUE => {
                if (self.data.buttons & (1u32 << bind_index)) != 0 { 1.0 } else { 0.0 }
            }
            _ => 0.0,
        }
    }

    pub fn set_binding_value(&mut self, bind_index: u32, value: f32) {
        use turntable_cid::*;
        match bind_index {
            CID_DJ_CROSSFADER_LEFT => self.data.crossfader_left = to_u9(value),
            CID_DJ_CROSSFADER_RIGHT => self.data.crossfader_right = to_u9(value),
            CID_DJ_EFFECTSKNOB_LEFT => self.data.effectsknob_left = to_u9(value),
            CID_DJ_EFFECTSKNOB_RIGHT => self.data.effectsknob_right = to_u9(value),
            CID_DJ_LEFT_TURNTABLE_CW => self.data.left_turntable_cw = to_u7(value, self.turntable_multiplier),
            CID_DJ_LEFT_TURNTABLE_CCW => self.data.left_turntable_ccw = to_u7(value, self.turntable_multiplier),
            CID_DJ_RIGHT_TURNTABLE_CW => self.data.right_turntable_cw = to_u7(value, self.turntable_multiplier),
            CID_DJ_RIGHT_TURNTABLE_CCW => self.data.right_turntable_ccw = to_u7(value, self.turntable_multiplier),
            CID_DJ_DPAD_UP => { self.data.hat_up = to_u8(value); self.update_hat_switch(); }
            CID_DJ_DPAD_DOWN => { self.data.hat_down = to_u8(value); self.update_hat_switch(); }
            CID_DJ_DPAD_LEFT => { self.data.hat_left = to_u8(value); self.update_hat_switch(); }
            CID_DJ_DPAD_RIGHT => { self.data.hat_right = to_u8(value); self.update_hat_switch(); }
            CID_DJ_SQUARE | CID_DJ_CROSS | CID_DJ_CIRCLE | CID_DJ_TRIANGLE
            | CID_DJ_SELECT | CID_DJ_START | CID_DJ_RIGHT_GREEN | CID_DJ_RIGHT_RED
            | CID_DJ_RIGHT_BLUE | CID_DJ_LEFT_GREEN | CID_DJ_LEFT_RED
            | CID_DJ_LEFT_BLUE => {
                let mask = 1u32 << bind_index;
                if value >= 0.5 { self.data.buttons |= mask; } else { self.data.buttons &= !mask; }
            }
            _ => {}
        }
    }

    pub fn update_report(&mut self, buf: &mut [u8]) -> usize {
        if buf.len() < 27 { return 0; }
        self.update_hat_switch();
        let buttons = (self.data.buttons & ((1 << (turntable_cid::CID_DJ_START + 1)) - 1))
            | ((self.data.hatswitch as u32 & 0xF) << 16);
        let mapped = buttons
            | (if (self.data.buttons & (1 << turntable_cid::CID_DJ_LEFT_GREEN)) != 0
                || (self.data.buttons & (1 << turntable_cid::CID_DJ_RIGHT_GREEN)) != 0 {
                1 << turntable_cid::CID_DJ_CROSS
            } else { 0 })
            | (if (self.data.buttons & (1 << turntable_cid::CID_DJ_LEFT_RED)) != 0
                || (self.data.buttons & (1 << turntable_cid::CID_DJ_RIGHT_RED)) != 0 {
                1 << turntable_cid::CID_DJ_CIRCLE
            } else { 0 })
            | (if (self.data.buttons & (1 << turntable_cid::CID_DJ_LEFT_BLUE)) != 0
                || (self.data.buttons & (1 << turntable_cid::CID_DJ_RIGHT_BLUE)) != 0 {
                1 << turntable_cid::CID_DJ_SQUARE
            } else { 0 });
        let b = mapped.to_le_bytes();
        buf[0..4].copy_from_slice(&b);
        let mut crossfader: i32 = 0x0200;
        let mut effectsknob: i32 = 0x0200;
        if self.data.crossfader_left > 0 { crossfader -= self.data.crossfader_left as i32; }
        else { crossfader += self.data.crossfader_right as i32; }
        if self.data.effectsknob_left > 0 { effectsknob -= self.data.effectsknob_left as i32; }
        else { effectsknob += self.data.effectsknob_right as i32; }
        let mut left_turntable: i32 = 0x80;
        let mut right_turntable: i32 = 0x80;
        if self.data.left_turntable_ccw > 0 { left_turntable -= min(self.data.left_turntable_ccw as i32, 0x7F); }
        else { left_turntable += min(self.data.left_turntable_cw as i32, 0x7F); }
        if self.data.right_turntable_ccw > 0 { right_turntable -= min(self.data.right_turntable_ccw as i32, 0x7F); }
        else { right_turntable += min(self.data.right_turntable_cw as i32, 0x7F); }
        buf[3] = 0x80;
        buf[4] = 0x80;
        buf[5] = left_turntable as u8;
        buf[6] = right_turntable as u8;
        buf[19] = (effectsknob & 0xFF) as u8;
        buf[20] = ((effectsknob >> 8) & 0xFF) as u8;
        buf[21] = (crossfader & 0xFF) as u8;
        buf[22] = ((crossfader >> 8) & 0xFF) as u8;
        buf[23] = ((self.data.buttons >> turntable_cid::CID_DJ_RIGHT_GREEN) & 0xFF) as u8;
        buf[24] = 0x02;
        buf[26] = 0x02;
        27
    }

    pub fn bindings() -> &'static [InputBindingInfo] {
        use turntable_cid::*;
        &[
            InputBindingInfo { name: "DPadUp",         display_name: "D-Pad Up",                       icon: None, kind: BindingType::Button,   id: CID_DJ_DPAD_UP,           generic: 0 },
            InputBindingInfo { name: "DPadDown",       display_name: "D-Pad Down",                     icon: None, kind: BindingType::Button,   id: CID_DJ_DPAD_DOWN,         generic: 0 },
            InputBindingInfo { name: "DPadLeft",       display_name: "D-Pad Left",                     icon: None, kind: BindingType::Button,   id: CID_DJ_DPAD_LEFT,         generic: 0 },
            InputBindingInfo { name: "DPadRight",      display_name: "D-Pad Right",                    icon: None, kind: BindingType::Button,   id: CID_DJ_DPAD_RIGHT,        generic: 0 },
            InputBindingInfo { name: "Square",         display_name: "Square",                         icon: None, kind: BindingType::Button,   id: CID_DJ_SQUARE,            generic: 0 },
            InputBindingInfo { name: "Cross",          display_name: "Cross",                          icon: None, kind: BindingType::Button,   id: CID_DJ_CROSS,             generic: 0 },
            InputBindingInfo { name: "Circle",         display_name: "Circle",                         icon: None, kind: BindingType::Button,   id: CID_DJ_CIRCLE,            generic: 0 },
            InputBindingInfo { name: "Triangle",       display_name: "Triangle / Euphoria",            icon: None, kind: BindingType::Button,   id: CID_DJ_TRIANGLE,          generic: 0 },
            InputBindingInfo { name: "Select",         display_name: "Select",                         icon: None, kind: BindingType::Button,   id: CID_DJ_SELECT,            generic: 0 },
            InputBindingInfo { name: "Start",          display_name: "Start",                          icon: None, kind: BindingType::Button,   id: CID_DJ_START,             generic: 0 },
            InputBindingInfo { name: "CrossFaderLeft", display_name: "Crossfader Left",                icon: None, kind: BindingType::HalfAxis, id: CID_DJ_CROSSFADER_LEFT,   generic: 0 },
            InputBindingInfo { name: "CrossFaderRight",display_name: "Crossfader Right",               icon: None, kind: BindingType::HalfAxis, id: CID_DJ_CROSSFADER_RIGHT,  generic: 0 },
            InputBindingInfo { name: "EffectsKnobLeft",display_name: "Effects Knob Left",              icon: None, kind: BindingType::HalfAxis, id: CID_DJ_EFFECTSKNOB_LEFT,  generic: 0 },
            InputBindingInfo { name: "EffectsKnobRight",display_name:"Effects Knob Right",             icon: None, kind: BindingType::HalfAxis, id: CID_DJ_EFFECTSKNOB_RIGHT, generic: 0 },
            InputBindingInfo { name: "LeftTurntableCW",display_name: "Left Turntable Clockwise",       icon: None, kind: BindingType::HalfAxis, id: CID_DJ_LEFT_TURNTABLE_CW, generic: 0 },
            InputBindingInfo { name: "LeftTurntableCCW",display_name:"Left Turntable Counterclockwise",icon: None, kind: BindingType::HalfAxis, id: CID_DJ_LEFT_TURNTABLE_CCW,generic: 0 },
            InputBindingInfo { name: "RightTurntableCW",display_name:"Right Turntable Clockwise",      icon: None, kind: BindingType::HalfAxis, id: CID_DJ_RIGHT_TURNTABLE_CW,generic: 0 },
            InputBindingInfo { name: "RightTurntableCCW",display_name:"Right Turntable Counterclockwise",icon:None, kind: BindingType::HalfAxis, id: CID_DJ_RIGHT_TURNTABLE_CCW,generic:0 },
            InputBindingInfo { name: "LeftTurntableGreen",display_name:"Left Turntable Green",          icon: None, kind: BindingType::Button,   id: CID_DJ_LEFT_GREEN,        generic: 0 },
            InputBindingInfo { name: "LeftTurntableRed",  display_name:"Left Turntable Red",            icon: None, kind: BindingType::Button,   id: CID_DJ_LEFT_RED,          generic: 0 },
            InputBindingInfo { name: "LeftTurntableBlue", display_name:"Left Turntable Blue",           icon: None, kind: BindingType::Button,   id: CID_DJ_LEFT_BLUE,         generic: 0 },
            InputBindingInfo { name: "RightTurntableGreen",display_name:"Right Turntable Green",        icon: None, kind: BindingType::Button,   id: CID_DJ_RIGHT_GREEN,       generic: 0 },
            InputBindingInfo { name: "RightTurntableRed",  display_name:"Right Turntable Red",          icon: None, kind: BindingType::Button,   id: CID_DJ_RIGHT_RED,         generic: 0 },
            InputBindingInfo { name: "RightTurntableBlue", display_name:"Right Turntable Blue",         icon: None, kind: BindingType::Button,   id: CID_DJ_RIGHT_BLUE,        generic: 0 },
        ]
    }

    pub fn settings() -> &'static [SettingInfo] {
        &[SettingInfo {
            kind: SettingType::Float,
            name: "TurntableMultiplier",
            display_name: "Turntable Multiplier",
            description: "Apply a sensitivity multiplier to turntable rotation.",
            default_value: "1.00",
            min_value: Some("0.00"),
            max_value: Some("512.0"),
            step_value: Some("1.0"),
            format: Some("%.0fx"),
        }]
    }

    pub fn init(&mut self) { self.update_hat_switch(); }
    pub fn shutdown(&mut self) {}
    pub fn update(&mut self) { self.update_hat_switch(); }
}

// Silence "unused" warnings for tiny constants pulled in for parity.
#[allow(dead_code)]
const _MAX_USED: usize = {
    let mut m = MC_HANDLE.len();
    if MC_REVERSER.len() > m { m = MC_REVERSER.len(); }
    if MC_BUTTON_PRESSED.len() > m { m = MC_BUTTON_PRESSED.len(); }
    m
};
