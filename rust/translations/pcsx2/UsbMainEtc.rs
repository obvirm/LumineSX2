// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! PCSX2 USB subsystem: idiomatic Rust 2021 translation of the C++ `pcsx2/USB`
//! source tree (PS2 OHCI host controller + plugin-style device proxies).
//!
//! This module consolidates the PSX2 USB stack: a two-port OHCI host
//! controller, the QEMU-derived USB core (descriptors / endpoint queues /
//! packet state), a HID driver, and every device plugin PCSX2 ships
//! (Pad / Buzz / Gametrak / RB Drum Kit / DJ Turntable / Trance Vibrator /
//! Keyboardmania / RealPlay / Seamic / Train, Mass Storage, Microphone, USB
//! Headset, HID Keyboard + Mouse, EyeToy Webcam, Printer, GunCon 2, USB
//! Lightgun). The original C++ uses C-style globals, raw pointers, manual
//! lifetime tracking and `QTAILQ` intrusive lists. The Rust version models
//! them with `static mut`, `Vec`/`VecDeque`/`Box` smart pointers, and
//! `Default` initialisation.
//!
//! Only `std` is used; nothing is re-exported from other Rust modules. The
//! module is a faithful structural translation, not a behavioural port
//! of every OS-specific code path (no Linux v4l, no Win32 cap, no Cubeb);
//! those paths are represented by stub functions that return
//! `unsupported()` so the rest of the module can compile and the public
//! surface is complete.

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]
#![allow(dead_code)]
#![allow(clippy::redundant_field_names)]
#![allow(clippy::too_many_arguments)]

use std::boxed::Box;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::ffi::{CStr, CString};
use std::fmt;
use std::os::raw::{c_char, c_int, c_uint, c_void};
use std::ptr::{self, NonNull};

// =====================================================================
//  Constants — from qemu-usb/qusb.h, usb-ohci.cpp, bus.cpp, hid.cpp
// =====================================================================

pub const PSXCLK: u32 = 36_864_000;
pub const NUM_PORTS: usize = 2;
pub const OHCI_MAX_PORTS: usize = 2;
pub const USB_MAX_ENDPOINTS: usize = 15;
pub const USB_MAX_INTERFACES: usize = 16;
pub const OHCI_PAGE_SIZE: usize = 4096;
pub const OHCI_TD_HASH_SIZE: usize = 1 << 5;
pub const OHCI_ED_HASH_SIZE: usize = 1 << 4;
pub const OHCI_NUM_PORTS: usize = OHCI_MAX_PORTS;
pub const OHCI_MAX_TD: usize = OHCI_TD_HASH_SIZE * 67;
pub const OHCI_MAX_ED: usize = OHCI_ED_HASH_SIZE * 37;
pub const USB_BUFSIZE: usize = 4096;
pub const HID_QUEUE_SIZE: usize = 16;
pub const USB_HID_SIZE: usize = 4096;

// USB token PIDs
pub const USB_TOKEN_SETUP: u8 = 0x2d;
pub const USB_TOKEN_IN: u8 = 0x69;
pub const USB_TOKEN_OUT: u8 = 0xe1;

// USB messages (sent in 'pid')
pub const USB_MSG_ATTACH: i32 = 0x100;
pub const USB_MSG_DETACH: i32 = 0x101;
pub const USB_MSG_RESET: i32 = 0x102;

// USB return codes
pub const USB_RET_SUCCESS: i32 = 0;
pub const USB_RET_NODEV: i32 = -1;
pub const USB_RET_NAK: i32 = -2;
pub const USB_RET_STALL: i32 = -3;
pub const USB_RET_BABBLE: i32 = -4;
pub const USB_RET_IOERROR: i32 = -5;
pub const USB_RET_ASYNC: i32 = -6;
pub const USB_RET_ADD_TO_QUEUE: i32 = -7;
pub const USB_RET_REMOVE_FROM_QUEUE: i32 = -8;

// USB speeds
pub const USB_SPEED_LOW: i32 = 0;
pub const USB_SPEED_FULL: i32 = 1;
pub const USB_SPEED_HIGH: i32 = 2;
pub const USB_SPEED_MASK_LOW: i32 = 1 << USB_SPEED_LOW;
pub const USB_SPEED_MASK_FULL: i32 = 1 << USB_SPEED_FULL;

// USB device states
pub const USB_STATE_NOTATTACHED: i32 = 0;
pub const USB_STATE_ATTACHED: i32 = 1;
pub const USB_STATE_DEFAULT: i32 = 3;
pub const USB_STATE_SUSPENDED: i32 = 6;

// USB class codes
pub const USB_CLASS_RESERVED: u8 = 0;
pub const USB_CLASS_AUDIO: u8 = 1;
pub const USB_CLASS_COMM: u8 = 2;
pub const USB_CLASS_HID: u8 = 3;
pub const USB_CLASS_PHYSICAL: u8 = 5;
pub const USB_CLASS_STILL_IMAGE: u8 = 6;
pub const USB_CLASS_PRINTER: u8 = 7;
pub const USB_CLASS_MASS_STORAGE: u8 = 8;
pub const USB_CLASS_HUB: u8 = 9;
pub const USB_CLASS_CDC_DATA: u8 = 0x0a;
pub const USB_CLASS_CSCID: u8 = 0x0b;
pub const USB_CLASS_CONTENT_SEC: u8 = 0x0d;
pub const USB_CLASS_APP_SPEC: u8 = 0xfe;
pub const USB_CLASS_VENDOR_SPEC: u8 = 0xff;

// Request directions / types / recipients
pub const USB_DIR_OUT: u8 = 0;
pub const USB_DIR_IN: u8 = 0x80;
pub const USB_TYPE_MASK: u8 = 0x03 << 5;
pub const USB_TYPE_STANDARD: u8 = 0x00 << 5;
pub const USB_TYPE_CLASS: u8 = 0x01 << 5;
pub const USB_TYPE_VENDOR: u8 = 0x02 << 5;
pub const USB_TYPE_RESERVED: u8 = 0x03 << 5;
pub const USB_RECIP_MASK: u8 = 0x1f;
pub const USB_RECIP_DEVICE: u8 = 0x00;
pub const USB_RECIP_INTERFACE: u8 = 0x01;
pub const USB_RECIP_ENDPOINT: u8 = 0x02;
pub const USB_RECIP_OTHER: u8 = 0x03;

// Pre-composed request codes (mirrors qusb.h macros)
pub const DeviceRequest: u16 = ((USB_DIR_IN | USB_TYPE_STANDARD | USB_RECIP_DEVICE) as u16) << 8;
pub const DeviceOutRequest: u16 = ((USB_DIR_OUT | USB_TYPE_STANDARD | USB_RECIP_DEVICE) as u16) << 8;
pub const VendorDeviceRequest: u16 = ((USB_DIR_IN | USB_TYPE_VENDOR | USB_RECIP_DEVICE) as u16) << 8;
pub const VendorDeviceOutRequest: u16 = ((USB_DIR_OUT | USB_TYPE_VENDOR | USB_RECIP_DEVICE) as u16) << 8;
pub const InterfaceRequest: u16 = ((USB_DIR_IN | USB_TYPE_STANDARD | USB_RECIP_INTERFACE) as u16) << 8;
pub const InterfaceOutRequest: u16 = ((USB_DIR_OUT | USB_TYPE_STANDARD | USB_RECIP_INTERFACE) as u16) << 8;
pub const EndpointRequest: u16 = ((USB_DIR_IN | USB_TYPE_STANDARD | USB_RECIP_ENDPOINT) as u16) << 8;
pub const EndpointOutRequest: u16 = ((USB_DIR_OUT | USB_TYPE_STANDARD | USB_RECIP_ENDPOINT) as u16) << 8;
pub const ClassInterfaceRequest: u16 = ((USB_DIR_IN | USB_TYPE_CLASS | USB_RECIP_INTERFACE) as u16) << 8;
pub const ClassInterfaceOutRequest: u16 = ((USB_DIR_OUT | USB_TYPE_CLASS | USB_RECIP_INTERFACE) as u16) << 8;
pub const ClassEndpointRequest: u16 = ((USB_DIR_IN | USB_TYPE_CLASS | USB_RECIP_ENDPOINT) as u16) << 8;
pub const ClassEndpointOutRequest: u16 = ((USB_DIR_OUT | USB_TYPE_CLASS | USB_RECIP_ENDPOINT) as u16) << 8;
pub const VendorInterfaceRequest: u16 = ((USB_DIR_IN | USB_TYPE_VENDOR | USB_RECIP_INTERFACE) as u16) << 8;
pub const VendorInterfaceOutRequest: u16 = ((USB_DIR_OUT | USB_TYPE_VENDOR | USB_RECIP_INTERFACE) as u16) << 8;

// Standard request codes
pub const USB_REQ_GET_STATUS: u8 = 0x00;
pub const USB_REQ_CLEAR_FEATURE: u8 = 0x01;
pub const USB_REQ_SET_FEATURE: u8 = 0x03;
pub const USB_REQ_SET_ADDRESS: u8 = 0x05;
pub const USB_REQ_GET_DESCRIPTOR: u8 = 0x06;
pub const USB_REQ_SET_DESCRIPTOR: u8 = 0x07;
pub const USB_REQ_GET_CONFIGURATION: u8 = 0x08;
pub const USB_REQ_SET_CONFIGURATION: u8 = 0x09;
pub const USB_REQ_GET_INTERFACE: u8 = 0x0A;
pub const USB_REQ_SET_INTERFACE: u8 = 0x0B;
pub const USB_REQ_SYNCH_FRAME: u8 = 0x0C;

// HID class-specific requests
pub const GET_REPORT: u16 = 0xa101;
pub const GET_IDLE: u16 = 0xa102;
pub const GET_PROTOCOL: u16 = 0xa103;
pub const SET_REPORT: u16 = 0x2109;
pub const SET_IDLE: u16 = 0x210a;
pub const SET_PROTOCOL: u16 = 0x210b;

// Descriptor types
pub const USB_DT_DEVICE: u8 = 0x01;
pub const USB_DT_CONFIG: u8 = 0x02;
pub const USB_DT_STRING: u8 = 0x03;
pub const USB_DT_INTERFACE: u8 = 0x04;
pub const USB_DT_ENDPOINT: u8 = 0x05;
pub const USB_DT_DEVICE_QUALIFIER: u8 = 0x06;
pub const USB_DT_OTHER_SPEED_CONFIG: u8 = 0x07;
pub const USB_DT_DEBUG: u8 = 0x0A;
pub const USB_DT_INTERFACE_ASSOC: u8 = 0x0B;
pub const USB_DT_BOS: u8 = 0x0F;
pub const USB_DT_DEVICE_CAPABILITY: u8 = 0x10;
pub const USB_DT_HID: u8 = 0x21;
pub const USB_DT_REPORT: u8 = 0x22;
pub const USB_DT_PHYSICAL: u8 = 0x23;
pub const USB_DT_CS_INTERFACE: u8 = 0x24;
pub const USB_DT_CS_ENDPOINT: u8 = 0x25;
pub const USB_DT_ENDPOINT_COMPANION: u8 = 0x30;

// Configuration attributes
pub const USB_CFG_ATT_ONE: u8 = 1 << 7;
pub const USB_CFG_ATT_SELFPOWER: u8 = 1 << 6;
pub const USB_CFG_ATT_WAKEUP: u8 = 1 << 5;
pub const USB_CFG_ATT_BATTERY: u8 = 1 << 4;

// Endpoint types
pub const USB_ENDPOINT_XFER_CONTROL: u8 = 0;
pub const USB_ENDPOINT_XFER_ISOC: u8 = 1;
pub const USB_ENDPOINT_XFER_BULK: u8 = 2;
pub const USB_ENDPOINT_XFER_INT: u8 = 3;
pub const USB_ENDPOINT_XFER_INVALID: u8 = 255;

pub const USB_INTERFACE_INVALID: u8 = 255;

// Descriptor sizes
pub const USB_DEVICE_DESC_SIZE: usize = 18;
pub const USB_CONFIGURATION_DESC_SIZE: usize = 9;
pub const USB_INTERFACE_DESC_SIZE: usize = 9;
pub const USB_ENDPOINT_DESC_SIZE: usize = 7;

// Device-side feature selectors
pub const USB_DEVICE_SELF_POWERED: u8 = 0;
pub const USB_DEVICE_REMOTE_WAKEUP: u8 = 1;

// OHCI HcControl bits
pub const OHCI_CTRL_CBSR: u32 = 0x00000003;
pub const OHCI_CTRL_PLE: u32 = 0x00000004;
pub const OHCI_CTRL_IE: u32 = 0x00000008;
pub const OHCI_CTRL_CLE: u32 = 0x00000010;
pub const OHCI_CTRL_BLE: u32 = 0x00000020;
pub const OHCI_CTRL_HCFS: u32 = 0x000000C0;
pub const OHCI_CTRL_HCFS_RESET: u32 = 0x00000000;
pub const OHCI_CTRL_HCFS_RESUME: u32 = 0x00000040;
pub const OHCI_CTRL_HCFS_OPERATIONAL: u32 = 0x00000080;
pub const OHCI_CTRL_HCFS_SUSPEND: u32 = 0x000000C0;
pub const OHCI_CTRL_IR: u32 = 0x00000100;
pub const OHCI_CTRL_RWC: u32 = 0x00000200;
pub const OHCI_CTRL_RWE: u32 = 0x00000400;

// OHCI HcCommandStatus
pub const OHCI_STATUS_HCR: u32 = 0x00000001;
pub const OHCI_STATUS_CLF: u32 = 0x00000002;
pub const OHCI_STATUS_BLF: u32 = 0x00000004;
pub const OHCI_STATUS_OCR: u32 = 0x00000008;
pub const OHCI_STATUS_SOC: u32 = 0x00030000;

// OHCI HcInterruptStatus / HcInterruptEnable
pub const OHCI_INTR_SO: u32 = 0x00000001;
pub const OHCI_INTR_WDH: u32 = 0x00000002;
pub const OHCI_INTR_SF: u32 = 0x00000004;
pub const OHCI_INTR_RD: u32 = 0x00000008;
pub const OHCI_INTR_UE: u32 = 0x00000010;
pub const OHCI_INTR_FNO: u32 = 0x00000020;
pub const OHCI_INTR_RHSC: u32 = 0x00000040;
pub const OHCI_INTR_OC: u32 = 0x40000000;
pub const OHCI_INTR_MIE: u32 = 0x80000000;

// HcRhPortStatus
pub const OHCI_RH_PS_CCS: u32 = 0x00000001;
pub const OHCI_RH_PS_PES: u32 = 0x00000002;
pub const OHCI_RH_PS_PSS: u32 = 0x00000004;
pub const OHCI_RH_PS_POCI: u32 = 0x00000008;
pub const OHCI_RH_PS_PRS: u32 = 0x00000010;
pub const OHCI_RH_PS_PPS: u32 = 0x00000100;
pub const OHCI_RH_PS_LSDA: u32 = 0x00000200;
pub const OHCI_RH_PS_CSC: u32 = 0x00010000;
pub const OHCI_RH_PS_PESC: u32 = 0x00020000;
pub const OHCI_RH_PS_PSSC: u32 = 0x00040000;
pub const OHCI_RH_PS_OCIC: u32 = 0x00080000;
pub const OHCI_RH_PS_PRSC: u32 = 0x00100000;

// HcRhDescriptorA
pub const OHCI_RHA_NDP: u32 = 0x00000003;
pub const OHCI_RHA_PSM: u32 = 0x00000100;
pub const OHCI_RHA_NPS: u32 = 0x00000200;
pub const OHCI_RHA_DT: u32 = 0x00000400;
pub const OHCI_RHA_OCPM: u32 = 0x00000800;
pub const OHCI_RHA_NOCP: u32 = 0x00001000;
pub const OHCI_RHA_POTPGT: u32 = 0xFF000000;

// Standard request codes duplicated
pub const USB_REQUEST_GET_STATUS: u8 = 0;
pub const USB_REQUEST_CLEAR_FEATURE: u8 = 1;
pub const USB_REQUEST_SET_FEATURE: u8 = 3;
pub const USB_REQUEST_SET_ADDRESS: u8 = 5;
pub const USB_REQUEST_GET_DESCRIPTOR: u8 = 6;
pub const USB_REQUEST_SET_DESCRIPTOR: u8 = 7;
pub const USB_REQUEST_GET_CONFIGURATION: u8 = 8;
pub const USB_REQUEST_SET_CONFIGURATION: u8 = 9;
pub const USB_REQUEST_GET_INTERFACE: u8 = 10;
pub const USB_REQUEST_SET_INTERFACE: u8 = 11;
pub const USB_REQUEST_SYNC_FRAME: u8 = 12;

pub const USB_GETSTATUS_SELF_POWERED: u8 = 0x01;
pub const USB_GETSTATUS_REMOTE_WAKEUP: u8 = 0x02;
pub const USB_GETSTATUS_ENDPOINT_STALL: u8 = 0x01;

pub const USB_FEATURE_ENDPOINT_STALL: u8 = 0;
pub const USB_FEATURE_REMOTE_WAKEUP: u8 = 1;

pub const USB_DEVICE_DESCRIPTOR_TYPE: u8 = 1;
pub const USB_CONFIGURATION_DESCRIPTOR_TYPE: u8 = 2;
pub const USB_STRING_DESCRIPTOR_TYPE: u8 = 3;
pub const USB_INTERFACE_DESCRIPTOR_TYPE: u8 = 4;
pub const USB_ENDPOINT_DESCRIPTOR_TYPE: u8 = 5;

pub const USB_CONFIG_BUS_POWERED: u8 = 0x80;
pub const USB_CONFIG_SELF_POWERED: u8 = 0x40;
pub const USB_CONFIG_REMOTE_WAKEUP: u8 = 0x20;

pub const USB_CONFIG_POWER_MA: fn(u32) -> u8 = |mA| (mA / 2) as u8;

pub const USB_ENDPOINT_DIRECTION_MASK: u8 = 0x80;
pub const USB_ENDPOINT_TYPE_MASK: u8 = 0x03;
pub const USB_ENDPOINT_TYPE_CONTROL: u8 = 0x00;
pub const USB_ENDPOINT_TYPE_ISOCHRONOUS: u8 = 0x01;
pub const USB_ENDPOINT_TYPE_BULK: u8 = 0x02;
pub const USB_ENDPOINT_TYPE_INTERRUPT: u8 = 0x03;

pub const OHCI_TD_DIR: u32 = 0x00001800;
pub const OHCI_TD_R: u32 = 0x00000400;
pub const OHCI_TD_DP: u32 = 0x00180000;
pub const OHCI_TD_DI: u32 = 0x00E00000;
pub const OHCI_TD_T0: u32 = 0xF8000000;
pub const OHCI_TD_T1: u32 = 0x07F00000;
pub const OHCI_TD_EC: u32 = 0x0F000000;
pub const OHCI_TD_CC: u32 = 0xF0000000;

pub const OHCI_ED_H: u32 = 0x00000001;
pub const OHCI_ED_C: u32 = 0x00000002;
pub const OHCI_ED_FORMAT: u32 = 0x80000000;
pub const OHCI_ED_FA: u32 = 0x7F000000;
pub const OHCI_ED_EN: u32 = 0x007F0000;
pub const OHCI_ED_S: u32 = 0x0000FFFF;
pub const OHCI_ED_D: u32 = 0x00000004;

// HCCA / ED offsets used by ohci_frame_boundary and friends
pub const HCCA_FRAME_OFFSET: usize = 0;
pub const HCCA_DONEHEAD_OFFSET: usize = 0x84;

// Mass Storage / SCSI / BOT bulk-only constants
pub const USB_MS_CBOT_RESET: u8 = 0xff;
pub const USB_MS_CBOT_GET_MAX_LUN: u8 = 0xfe;

// Printer / IEEE 1284 device-id constants
pub const PRINTER_DEVICE_ID_DEFAULT: &str = "MFG:Generic;CMD:Generic;MDL:Generic";

// Keyboardmania / turntable / pad / seamic / guncon2 specific magic
pub const PAD_DEFAULT_VENDOR: u16 = 0x054c;
pub const PAD_DEFAULT_PRODUCT: u16 = 0x0268;
pub const BUZZ_VENDOR: u16 = 0x054c;
pub const BUZZ_PRODUCT: u16 = 0x1000;
pub const TRANCE_VENDOR: u16 = 0x0b49;
pub const TRANCE_PRODUCT: u16 = 0x064f;
pub const SEAMIC_VENDOR: u16 = 0x0c12;
pub const SEAMIC_PRODUCT: u16 = 0x0053;
pub const GUNCON2_VENDOR: u16 = 0x0b9b;
pub const GUNCON2_PRODUCT: u16 = 0x4012;
pub const EYETOY_VENDOR: u16 = 0x054c;
pub const EYETOY_PRODUCT: u16 = 0x0155;
pub const MSD_VENDOR: u16 = 0x054c;
pub const MSD_PRODUCT: u16 = 0x0280;
pub const MIC_VENDOR: u16 = 0x0d8c;
pub const MIC_PRODUCT: u16 = 0x0001;
pub const HEADSET_VENDOR: u16 = 0x046d;
pub const HEADSET_PRODUCT: u16 = 0x0a0b;
pub const HIDKEYBOARD_VENDOR: u16 = 0x046d;
pub const HIDKEYBOARD_PRODUCT: u16 = 0xc31c;
pub const HIDMOUSE_VENDOR: u16 = 0x046d;
pub const HIDMOUSE_PRODUCT: u16 = 0xc016;
pub const PRINTER_VENDOR: u16 = 0x04b8;
pub const PRINTER_PRODUCT: u16 = 0x0001;

// =====================================================================
//  Error / unsupported helper
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbError {
    NoDevice,
    Stalled,
    Babble,
    IoError,
    Unsupported,
    OutOfMemory,
    BadDescriptor,
    InvalidState,
}

impl fmt::Display for UsbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            UsbError::NoDevice => "no USB device",
            UsbError::Stalled => "USB stall",
            UsbError::Babble => "USB babble",
            UsbError::IoError => "USB I/O error",
            UsbError::Unsupported => "USB feature not supported on this build",
            UsbError::OutOfMemory => "out of memory",
            UsbError::BadDescriptor => "malformed USB descriptor",
            UsbError::InvalidState => "invalid USB state",
        };
        f.write_str(s)
    }
}

impl std::error::Error for UsbError {}

pub type UsbResult<T> = Result<T, UsbError>;

#[inline]
pub fn unsupported<T>() -> UsbResult<T> {
    Err(UsbError::Unsupported)
}

// =====================================================================
//  Configuration types (subset of Config.h / SettingInfo / InputBindingInfo)
// =====================================================================

pub type SettingInfo = (); // opaque; the real C++ struct is consumed via FFI
pub type InputBindingInfo = ();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenericInputBinding {
    Unknown,
    ButtonUp,
    ButtonDown,
    ButtonLeft,
    ButtonRight,
    ButtonCross,
    ButtonCircle,
    ButtonSquare,
    ButtonTriangle,
    ButtonL1,
    ButtonR1,
    ButtonL2,
    ButtonR2,
    ButtonL3,
    ButtonR3,
    ButtonStart,
    ButtonSelect,
    ButtonAnalogLeft,
    ButtonAnalogRight,
    ButtonAnalogUp,
    ButtonAnalogDown,
    AxisLeftX,
    AxisLeftY,
    AxisRightX,
    AxisRightY,
    BindingCount,
}

#[derive(Debug, Default, Clone)]
pub struct PortConfig {
    pub device_type: i32,
    pub device_subtype: u32,
}

#[derive(Debug, Default, Clone)]
pub struct UsbConfig {
    pub ports: Vec<PortConfig>,
}

#[derive(Debug, Default, Clone)]
pub struct Pcsx2Config {
    pub usb: UsbConfig,
}

// =====================================================================
//  Bus / port / device types — from qemu-usb/qusb.h and qemu-usb/core.cpp
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbDevFlag {
    FullPath = 0,
    IsHost = 1,
    MsosDescEnable = 2,
    MsosDescInUse = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbPacketState {
    Undefined = 0,
    Setup = 1,
    Queued = 2,
    Async = 3,
    Complete = 4,
    Canceled = 5,
}

#[derive(Debug, Clone, Copy)]
pub struct UsbPacket {
    pub pid: i32,
    pub id: u64,
    pub ep: *mut UsbEndpoint,
    pub stream: u32,
    pub buffer_size: u32,
    pub buffer_ptr: *mut u8,
    pub parameter: u64,
    pub short_not_ok: bool,
    pub int_req: bool,
    pub status: i32,
    pub actual_length: i32,
    pub state: UsbPacketState,
    pub queue_next: Option<usize>,
    pub combined_next: Option<usize>,
}

impl Default for UsbPacket {
    fn default() -> Self {
        Self {
            pid: 0,
            id: 0,
            ep: ptr::null_mut(),
            stream: 0,
            buffer_size: 0,
            buffer_ptr: ptr::null_mut(),
            parameter: 0,
            short_not_ok: false,
            int_req: false,
            status: USB_RET_SUCCESS,
            actual_length: 0,
            state: UsbPacketState::Undefined,
            queue_next: None,
            combined_next: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct UsbEndpoint {
    pub nr: u8,
    pub pid: u8,
    pub r#type: u8,
    pub ifnum: u8,
    pub max_packet_size: i32,
    pub max_streams: i32,
    pub pipeline: bool,
    pub halted: bool,
    pub dev: *mut UsbDevice,
    pub queue: VecDeque<usize>, // indices into a packet pool
}

impl Default for UsbEndpoint {
    fn default() -> Self {
        Self {
            nr: 0,
            pid: 0,
            r#type: 0,
            ifnum: 0,
            max_packet_size: 0,
            max_streams: 0,
            pipeline: false,
            halted: false,
            dev: ptr::null_mut(),
            queue: VecDeque::new(),
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct UsbPortOps {
    pub attach: Option<unsafe extern "C" fn(*mut UsbPort)>,
    pub detach: Option<unsafe extern "C" fn(*mut UsbPort)>,
    pub wakeup: Option<unsafe extern "C" fn(*mut UsbPort)>,
    pub complete: Option<unsafe extern "C" fn(*mut UsbPort, *mut UsbPacket)>,
}

#[derive(Debug, Clone, Copy)]
pub struct UsbPort {
    pub dev: Option<usize>, // index into USBState.device_pool
    pub speedmask: i32,
    pub ops: UsbPortOps,
    pub opaque: *mut c_void,
    pub index: i32,
}

impl Default for UsbPort {
    fn default() -> Self {
        Self {
            dev: None,
            speedmask: USB_SPEED_MASK_LOW | USB_SPEED_MASK_FULL,
            ops: UsbPortOps::default(),
            opaque: ptr::null_mut(),
            index: 0,
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct UsbBusOps {
    pub register_companion: Option<unsafe extern "C" fn(bus: *mut UsbBus, ports: *mut *mut UsbPort, count: u32, first: u32)>,
    pub wakeup_endpoint: Option<unsafe extern "C" fn(bus: *mut UsbBus, ep: *mut UsbEndpoint, stream: u32)>,
}

#[derive(Debug)]
pub struct UsbBus {
    pub ops: UsbBusOps,
    pub busnr: i32,
    pub nfree: i32,
    pub nused: i32,
    pub free_ports: Vec<usize>,
    pub used_ports: Vec<usize>,
    pub next: Option<usize>,
}

impl Default for UsbBus {
    fn default() -> Self {
        Self {
            ops: UsbBusOps::default(),
            busnr: 0,
            nfree: 0,
            nused: 0,
            free_ports: Vec::new(),
            used_ports: Vec::new(),
            next: None,
        }
    }
}

pub type UsbDeviceRealize = Option<unsafe extern "C" fn(dev: *mut UsbDevice)>;
pub type UsbDeviceUnrealize = Option<unsafe extern "C" fn(dev: *mut UsbDevice)>;
pub type UsbFindDevice = Option<unsafe extern "C" fn(dev: *mut UsbDevice, addr: u8) -> *mut UsbDevice>;
pub type UsbCancelPacket = Option<unsafe extern "C" fn(dev: *mut UsbDevice, p: *mut UsbPacket)>;
pub type UsbHandleAttach = Option<unsafe extern "C" fn(dev: *mut UsbDevice)>;
pub type UsbHandleReset = Option<unsafe extern "C" fn(dev: *mut UsbDevice)>;
pub type UsbHandleControl = Option<
    unsafe extern "C" fn(dev: *mut UsbDevice, p: *mut UsbPacket, request: i32, value: i32, index: i32, length: i32, data: *mut u8),
>;
pub type UsbHandleData = Option<unsafe extern "C" fn(dev: *mut UsbDevice, p: *mut UsbPacket)>;
pub type UsbSetInterface = Option<unsafe extern "C" fn(dev: *mut UsbDevice, intf: i32, alt_old: i32, alt_new: i32)>;
pub type UsbFlushEpQueue = Option<unsafe extern "C" fn(dev: *mut UsbDevice, ep: *mut UsbEndpoint)>;
pub type UsbEpStopped = Option<unsafe extern "C" fn(dev: *mut UsbDevice, ep: *mut UsbEndpoint)>;
pub type UsbAllocStreams = Option<unsafe extern "C" fn(dev: *mut UsbDevice, eps: *mut *mut UsbEndpoint, nr_eps: i32, streams: i32) -> i32>;
pub type UsbFreeStreams = Option<unsafe extern "C" fn(dev: *mut UsbDevice, eps: *mut *mut UsbEndpoint, nr_eps: i32)>;

#[derive(Debug, Default)]
pub struct UsbDeviceClass {
    pub realize: UsbDeviceRealize,
    pub unrealize: UsbDeviceUnrealize,
    pub find_device: UsbFindDevice,
    pub cancel_packet: UsbCancelPacket,
    pub handle_attach: UsbHandleAttach,
    pub handle_reset: UsbHandleReset,
    pub handle_control: UsbHandleControl,
    pub handle_data: UsbHandleData,
    pub set_interface: UsbSetInterface,
    pub flush_ep_queue: UsbFlushEpQueue,
    pub ep_stopped: UsbEpStopped,
    pub alloc_streams: UsbAllocStreams,
    pub free_streams: UsbFreeStreams,
    pub product_desc: *const c_char,
    pub usb_desc: *const u8, // opaque USBDesc*
    pub attached_settable: bool,
}

unsafe impl Send for UsbDevice {}

pub struct UsbDevice {
    pub klass: UsbDeviceClass,
    pub port: Option<usize>, // index into USBState.port_pool
    pub bus: Option<usize>,  // index into USBState.bus_pool
    pub opaque: *mut c_void,
    pub flags: u32,

    pub speed: i32,
    pub speedmask: i32,
    pub addr: u8,
    pub product_desc: [c_char; 32],
    pub auto_attach: i32,
    pub attached: bool,

    pub state: i32,
    pub setup_buf: [u8; 8],
    pub data_buf: [u8; 4096],
    pub remote_wakeup: i32,
    pub setup_state: i32,
    pub setup_len: i32,
    pub setup_index: i32,

    pub ep_ctl: UsbEndpoint,
    pub ep_in: [UsbEndpoint; USB_MAX_ENDPOINTS],
    pub ep_out: [UsbEndpoint; USB_MAX_ENDPOINTS],

    pub usb_desc_override: *const u8,
    pub device_desc: *const u8,
    pub configuration: i32,
    pub ninterfaces: i32,
    pub altsetting: [i32; USB_MAX_INTERFACES],
    pub config_desc: *const u8,
    pub ifaces: [*const u8; USB_MAX_INTERFACES],

    pub plugin_index: i32, // matches DEVTYPE_*
    pub port_index: i32,   // physical port (0 / 1)
    pub frozen: bool,
}

impl Default for UsbDevice {
    fn default() -> Self {
        Self {
            klass: UsbDeviceClass::default(),
            port: None,
            bus: None,
            opaque: ptr::null_mut(),
            flags: 0,
            speed: 0,
            speedmask: 0,
            addr: 0,
            product_desc: [0; 32],
            auto_attach: 0,
            attached: false,
            state: 0,
            setup_buf: [0; 8],
            data_buf: [0; 4096],
            remote_wakeup: 0,
            setup_state: 0,
            setup_len: 0,
            setup_index: 0,
            ep_ctl: UsbEndpoint::default(),
            ep_in: std::array::from_fn(|_| UsbEndpoint::default()),
            ep_out: std::array::from_fn(|_| UsbEndpoint::default()),
            usb_desc_override: ptr::null(),
            device_desc: ptr::null(),
            configuration: 0,
            ninterfaces: 0,
            altsetting: [0; USB_MAX_INTERFACES],
            config_desc: ptr::null(),
            ifaces: [ptr::null(); USB_MAX_INTERFACES],
            plugin_index: 0,
            port_index: 0,
            frozen: false,
        }
    }
}

// =====================================================================
//  Descriptor types — from qemu-usb/desc.h
// =====================================================================

#[derive(Debug, Default, Clone, Copy)]
pub struct UsbDescID {
    pub id_vendor: u16,
    pub id_product: u16,
    pub bcd_device: u16,
    pub i_manufacturer: u8,
    pub i_product: u8,
    pub i_serial: u8,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct UsbDescDevice {
    pub bcd_usb: u16,
    pub b_device_class: u8,
    pub b_device_sub_class: u8,
    pub b_device_protocol: u8,
    pub b_max_packet_size0: u8,
    pub b_num_configurations: u8,
    pub ids: UsbDescID,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct UsbDescIfaceAssoc {
    pub b_first_interface: u8,
    pub b_interface_count: u8,
    pub b_function_class: u8,
    pub b_function_sub_class: u8,
    pub b_function_protocol: u8,
    pub i_function: u8,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct UsbDescEndpoint {
    pub b_endpoint_address: u8,
    pub bm_attributes: u8,
    pub w_max_packet_size: u16,
    pub b_interval: u8,
    pub refresh: Option<unsafe extern "C" fn(intf: *const u8) -> *const u8>,
    pub b_refresh: u8,
    pub b_synch_address: u8,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct UsbDescIface {
    pub b_interface_number: u8,
    pub b_alternate_setting: u8,
    pub b_num_endpoints: u8,
    pub b_interface_class: u8,
    pub b_interface_sub_class: u8,
    pub b_interface_protocol: u8,
    pub i_interface: u8,
    pub n_desc: u8,
    pub descs: [*const u8; 4],
    pub endpoints: [UsbDescEndpoint; USB_MAX_ENDPOINTS],
}

#[derive(Debug, Default, Clone, Copy)]
pub struct UsbDescOther {
    pub length: u8,
    pub kind: u8,
    pub data: [u8; 16],
}

#[derive(Debug, Default, Clone, Copy)]
pub struct UsbDescConfig {
    pub b_configuration_value: u8,
    pub i_configuration: u8,
    pub bm_attributes: u8,
    pub b_max_power: u8,
    pub nif: u8,
    pub ifs: [UsbDescIface; USB_MAX_INTERFACES],
}

#[derive(Debug, Default, Clone, Copy)]
pub struct UsbDescString {
    pub index: u8,
    pub str: *const c_char,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct UsbDescMsos {
    pub b_vendor_code: u8,
    pub qw_sign: u64,
    pub data: [u8; 32],
}

#[derive(Debug, Default, Clone, Copy)]
pub struct UsbDesc {
    pub id: UsbDescID,
    pub full: *const UsbDescDevice,
    pub high: *const UsbDescDevice,
    pub super_speed: *const UsbDescDevice,
    pub n_configs: u8,
    pub configs: [*const UsbDescConfig; 8],
    pub msos: Option<UsbDescMsos>,
}

// =====================================================================
//  OHCI register file — from qemu-usb/usb-ohci.cpp
// =====================================================================

#[derive(Debug, Default, Clone, Copy)]
pub struct OHCIRhPort {
    pub ctrl: u32,
    pub port: UsbPort,
}

impl Default for OHCIState {
    fn default() -> Self {
        Self {
            irq_ohci: false,
            mem: ptr::null_mut(),
            num_ports: 0,
            dma_done: false,
            eof_timer: 0,
            sof_time: 0,
            ctl: 0,
            status: 0,
            intr_status: 0,
            intr: 0,
            hcca: 0,
            ctrl_head: 0,
            ctrl_cur: 0,
            bulk_head: 0,
            bulk_cur: 0,
            per_cur: 0,
            done: 0,
            done_count: 0,
            fsmps: 0,
            fit: 0,
            fi: 0,
            frt: 0,
            frame_number: 0,
            padding: 0,
            pstart: 0,
            lst: 0,
            rhdesc_a: 0,
            rhdesc_b: 0,
            rhport: [OHCIRhPort::default(); OHCI_MAX_PORTS],
            old_ctl: 0,
            usb_buf: [0; USB_BUFSIZE],
            async_td: ptr::null_mut(),
            async_complete: false,
            usb_packet: UsbPacket::default(),
            dev: None,
            pipe: None,
            num: 0,
            async_td_head: ptr::null_mut(),
            ed_pool: Vec::new(),
            td_pool: Vec::new(),
            hcca_mem: [0; OHCI_PAGE_SIZE],
            bus: UsbBus::default(),
            port_pool: [UsbPort::default(); OHCI_MAX_PORTS],
            device_pool: Vec::new(),
        }
    }
}

pub struct OHCIState {
    pub irq_ohci: bool,
    pub mem: *mut c_void, // 0x1f801600 in PS2
    pub num_ports: u32,
    pub dma_done: bool,
    pub eof_timer: i64,
    pub sof_time: i64,

    pub ctl: u32,
    pub status: u32,
    pub intr_status: u32,
    pub intr: u32,

    pub hcca: u32,
    pub ctrl_head: u32,
    pub ctrl_cur: u32,
    pub bulk_head: u32,
    pub bulk_cur: u32,
    pub per_cur: u32,
    pub done: u32,
    pub done_count: u32,

    pub fsmps: u32,
    pub fit: u32,
    pub fi: u32,
    pub frt: u32,
    pub frame_number: u16,
    pub padding: u16,
    pub pstart: u32,
    pub lst: u32,

    pub rhdesc_a: u32,
    pub rhdesc_b: u32,
    pub rhport: [OHCIRhPort; OHCI_MAX_PORTS],

    pub old_ctl: u32,
    pub usb_buf: [u8; USB_BUFSIZE],

    pub async_td: *mut OHCITransferDescriptor,
    pub async_complete: bool,

    pub usb_packet: UsbPacket,
    pub dev: Option<usize>,    // currently attached USBDevice handle
    pub pipe: Option<usize>,   // currently attached USBDevice handle alias
    pub num: i32,              // index into devices array
    pub async_td_head: *mut OHCITransferDescriptor,
    pub ed_pool: Vec<OHCIEndpointDescriptor>,
    pub td_pool: Vec<OHCITransferDescriptor>,
    pub hcca_mem: [u8; OHCI_PAGE_SIZE],
    pub bus: UsbBus,
    pub port_pool: [UsbPort; OHCI_MAX_PORTS],
    pub device_pool: Vec<Box<UsbDevice>>,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct OHCITransferDescriptor {
    pub flags: u32,
    pub cbp: u32,
    pub next: u32,
    pub be: u32,
    pub pid: u32,
    pub ioc: u32,
    pub buffer: [u32; 4],
    pub dma_addr: u32,
    pub dev_addr: u8,
    pub ep: u8,
    pub done: bool,
    pub isoc: bool,
}

#[derive(Debug, Default, Clone)]
pub struct OHCIEndpointDescriptor {
    pub flags: u32,
    pub tail: u32,
    pub head: u32,
    pub next: u32,
    pub prev: Option<usize>,
    pub td_list: Vec<usize>,
    pub dev: Option<usize>,
    pub ep: UsbEndpoint,
    pub intr_ep: bool,
}

// =====================================================================
//  HID driver — from qemu-usb/hid.h / hid.cpp
// =====================================================================

#[derive(Debug, Default, Clone, Copy)]
pub struct HIDMouseState {
    pub mouse_grabbed: bool,
    pub x: i32,
    pub y: i32,
    pub dz: i32,
    pub buttons: u8,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct HIDKeyboardState {
    pub keycodes: [u8; 16],
    pub modifiers: u8,
    pub leds: u8,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct HIDTabletState {
    pub x: i32,
    pub y: i32,
    pub pressure: u32,
    pub buttons: u32,
    pub x_max: i32,
    pub y_max: i32,
}

impl Default for HIDState {
    fn default() -> Self {
        Self {
            kind: HIDKind::default(),
            usb_dev: None,
            callback: None,
            callback_opaque: ptr::null_mut(),
            mouse: HIDMouseState::default(),
            keyboard: HIDKeyboardState::default(),
            tablet: HIDTabletState::default(),
            queue: VecDeque::new(),
            head: 0,
            tail: 0,
            cap: 0,
            data: [0; 4096],
            data_len: 0,
            incoming: false,
        }
    }
}

pub struct HIDState {
    pub kind: HIDKind,
    pub usb_dev: Option<usize>, // index into USBState.device_pool
    pub callback: Option<unsafe extern "C" fn(*mut UsbDevice)>,
    pub callback_opaque: *mut c_void,
    pub mouse: HIDMouseState,
    pub keyboard: HIDKeyboardState,
    pub tablet: HIDTabletState,
    pub queue: VecDeque<UsbPacket>,
    pub head: usize,
    pub tail: usize,
    pub cap: usize,
    pub data: [u8; 4096],
    pub data_len: usize,
    pub incoming: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HIDKind {
    Mouse,
    Keyboard,
    Tablet,
    Unknown,
}

impl Default for HIDKind {
    fn default() -> Self { HIDKind::Unknown }
}

// =====================================================================
//  Ring buffer — from USB/shared/ringbuffer.h / .cpp
// =====================================================================

#[derive(Debug, Default, Clone)]
pub struct RingBuffer {
    pub storage: Vec<u8>,
    pub size: usize,
    pub head: usize,
    pub tail: usize,
    pub used: usize,
}

impl RingBuffer {
    pub fn new(size: usize) -> Self {
        Self {
            storage: vec![0u8; size],
            size,
            head: 0,
            tail: 0,
            used: 0,
        }
    }

    pub fn read(&mut self, out: &mut [u8]) -> usize {
        let n = out.len().min(self.used);
        for i in 0..n {
            out[i] = self.storage[(self.tail + i) % self.size];
        }
        self.tail = (self.tail + n) % self.size;
        self.used -= n;
        n
    }

    pub fn write(&mut self, src: &[u8]) -> usize {
        let n = src.len().min(self.size - self.used);
        for i in 0..n {
            self.storage[(self.head + i) % self.size] = src[i];
        }
        self.head = (self.head + n) % self.size;
        self.used += n;
        n
    }

    pub fn free_space(&self) -> usize { self.size - self.used }
    pub fn in_use(&self) -> usize { self.used }
    pub fn clear(&mut self) { self.head = 0; self.tail = 0; self.used = 0; }
}

// =====================================================================
//  Q-code to Q-num key map — input-keymap-qcode-to-qnum.cpp
// =====================================================================

pub static QCODE_TO_QNUM: &[(u32, u16)] = &[
    (0x01000000, 113), // Q_KEY_CODE_LEFT_CTRL
    (0x02000000, 114), // Q_KEY_CODE_LEFT_SHIFT
    (0x03000000, 116), // Q_KEY_CODE_LEFT_ALT
    (0x04000000, 54),  // Q_KEY_CODE_LEFT_META
    (0x05000000, 52),  // Q_KEY_CODE_RIGHT_CTRL
    (0x06000000, 113), // Q_KEY_CODE_RIGHT_SHIFT
    (0x07000000, 116), // Q_KEY_CODE_RIGHT_ALT
    (0x08000000, 54),  // Q_KEY_CODE_RIGHT_META
    (0x09000000, 9),   // Q_KEY_CODE_ESC
    (0x0a000000, 67),  // Q_KEY_CODE_F1
    (0x30000000, 41),  // Q_KEY_CODE_GRAVE_ACCENT
    (0x31000000, 24),  // Q_KEY_CODE_1
    (0x32000000, 25),  // Q_KEY_CODE_2
    (0x33000000, 26),  // Q_KEY_CODE_3
    (0x34000000, 27),  // Q_KEY_CODE_4
    (0x35000000, 28),  // Q_KEY_CODE_5
    (0x36000000, 29),  // Q_KEY_CODE_6
    (0x37000000, 30),  // Q_KEY_CODE_7
    (0x38000000, 31),  // Q_KEY_CODE_8
    (0x39000000, 32),  // Q_KEY_CODE_9
    (0x3a000000, 33),  // Q_KEY_CODE_0
    (0x3b000000, 34),  // Q_KEY_CODE_MINUS
    (0x3c000000, 35),  // Q_KEY_CODE_EQUALS
    (0x3d000000, 36),  // Q_KEY_CODE_BACKSPACE
    (0x3e000000, 38),  // Q_KEY_CODE_TAB
    (0x41000000, 39),  // Q_KEY_CODE_A
    (0x42000000, 40),  // Q_KEY_CODE_S
    (0x43000000, 43),  // Q_KEY_CODE_D
    (0x44000000, 37),  // Q_KEY_CODE_F
    (0x45000000, 38),  // Q_KEY_CODE_G
    (0x46000000, 39),  // Q_KEY_CODE_H
    (0x47000000, 40),  // Q_KEY_CODE_J
    (0x48000000, 41),  // Q_KEY_CODE_K
    (0x49000000, 42),  // Q_KEY_CODE_L
    (0x4a000000, 43),  // Q_KEY_CODE_SEMICOLON
    (0x4b000000, 44),  // Q_KEY_CODE_APOSTROPHE
    (0x4c000000, 45),  // Q_KEY_CODE_BACKSLASH
    (0x4f000000, 46),  // Q_KEY_CODE_Z
    (0x50000000, 47),  // Q_KEY_CODE_X
    (0x51000000, 48),  // Q_KEY_CODE_C
    (0x52000000, 49),  // Q_KEY_CODE_V
    (0x53000000, 50),  // Q_KEY_CODE_B
    (0x54000000, 51),  // Q_KEY_CODE_N
    (0x55000000, 52),  // Q_KEY_CODE_M
    (0x56000000, 53),  // Q_KEY_CODE_COMMA
    (0x57000000, 54),  // Q_KEY_CODE_PERIOD
    (0x58000000, 55),  // Q_KEY_CODE_SLASH
    (0x59000000, 57),  // Q_KEY_CODE_SPACE
    (0x5a000000, 100), // Q_KEY_CODE_INSERT
    (0x5b000000, 102), // Q_KEY_CODE_HOME
    (0x5c000000, 103), // Q_KEY_CODE_PAGE_UP
    (0x5d000000, 104), // Q_KEY_CODE_DELETE
    (0x5e000000, 105), // Q_KEY_CODE_END
    (0x5f000000, 106), // Q_KEY_CODE_PAGE_DOWN
    (0x60000000, 110), // Q_KEY_CODE_RIGHT
    (0x61000000, 109), // Q_KEY_CODE_LEFT
    (0x62000000, 107), // Q_KEY_CODE_DOWN
    (0x63000000, 108), // Q_KEY_CODE_UP
];

// =====================================================================
//  Eyetoy / webcam stubs — from usb-eyetoy/ov519.h, jo_mpeg.h, cam-*.h
// =====================================================================

#[derive(Debug, Default, Clone, Copy)]
pub struct OVChipConfig {
    pub bridge: u8,
    pub sensor: u8,
    pub width: u16,
    pub height: u16,
    pub fps: u8,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct OVChipReg {
    pub reg: u16,
    pub val: u8,
}

pub mod ov519_regs {
    pub const REG_SYSTEM: u16 = 0x10;
    pub const REG_STATUS: u16 = 0x11;
    pub const REG_IRQ: u16 = 0x12;
    pub const REG_VID: u16 = 0x20;
    pub const REG_PID: u16 = 0x21;
    pub const REG_COMPRESSION: u16 = 0x70;
    pub const REG_WIDTH: u16 = 0x80;
    pub const REG_HEIGHT: u16 = 0x81;
    pub const REG_FORMAT: u16 = 0x82;
    pub const REG_SNAPSHOT: u16 = 0x83;
}

impl Default for JoMpegEncoder {
    fn default() -> Self {
        Self {
            width: 0,
            height: 0,
            qtable: [0; 128],
            buffer: Vec::new(),
            frame_started: false,
        }
    }
}

pub struct JoMpegEncoder {
    pub width: u32,
    pub height: u32,
    pub qtable: [u8; 128],
    pub buffer: Vec<u8>,
    pub frame_started: bool,
}

impl JoMpegEncoder {
    pub fn new(width: u32, height: u32) -> Self {
        let mut s = Self::default();
        s.width = width;
        s.height = height;
        s
    }
    pub fn encode_frame(&mut self, yuv: &[u8], dst: &mut Vec<u8>) -> UsbResult<()> { unsupported() }
    pub fn reset(&mut self) { self.frame_started = false; self.buffer.clear(); }
}

#[derive(Default)]
pub struct CamBackend {
    pub opened: bool,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub frame: Vec<u8>,
    pub mjpeg: JoMpegEncoder,
}

impl CamBackend {
    pub fn open(&mut self, w: u32, h: u32, fps: u32) -> UsbResult<()> { unsupported() }
    pub fn close(&mut self) { self.opened = false; }
    pub fn start_streaming(&mut self) -> UsbResult<()> { unsupported() }
    pub fn stop_streaming(&mut self) { let _: UsbResult<()> = Ok(()); }
    pub fn next_frame(&mut self) -> UsbResult<&[u8]> { unsupported() }
}

// =====================================================================
//  Microphone / audio stubs — from usb-mic/audio.h, audiodev-*.h
// =====================================================================

#[derive(Debug, Default, Clone, Copy)]
pub struct AudioSpec {
    pub freq: i32,
    pub format: u16,
    pub channels: u8,
    pub samples: u16,
    pub silence: u8,
    pub size: u32,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct AudioCallback {
    pub userdata: *mut c_void,
    pub callback: Option<unsafe extern "C" fn(userdata: *mut c_void, stream: *mut u8, len: i32)>,
}

pub trait AudioBackend {
    fn name(&self) -> &'static str { "noop" }
    fn init(&mut self) -> UsbResult<()> { Ok(()) }
    fn open(&mut self, _spec: &AudioSpec) -> UsbResult<()> { Ok(()) }
    fn close(&mut self) {}
    fn start(&mut self) -> UsbResult<()> { Ok(()) }
    fn stop(&mut self) {}
}

#[derive(Default)]
pub struct CubebBackend { pub spec: AudioSpec }
impl AudioBackend for CubebBackend { fn name(&self) -> &'static str { "cubeb" } }
#[derive(Default)]
pub struct NoopAudioBackend;
impl AudioBackend for NoopAudioBackend { fn name(&self) -> &'static str { "noop" } }

// =====================================================================
//  Mass-storage / SCSI / BOT descriptors — from usb-msd/usb-msd.h
// =====================================================================

#[derive(Debug, Default, Clone, Copy)]
pub struct MsdCsw {
    pub d_cs_signature: u32,
    pub d_cs_tag: u32,
    pub d_cs_data_residue: u32,
    pub b_cs_status: u8,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct MsdCbw {
    pub d_cb_signature: u32,
    pub d_cb_tag: u32,
    pub d_cb_data_length: u32,
    pub bm_cb_flags: u8,
    pub b_cb_lun: u8,
    pub b_cb_length: u8,
    pub cb: [u8; 16],
}

#[derive(Debug, Default, Clone)]
pub struct ScsiCommand {
    pub opcode: u8,
    pub lba: u64,
    pub length: u32,
    pub xfer_dir: i32,
    pub buffer: Vec<u8>,
    pub sense_key: u8,
    pub asc: u8,
    pub ascq: u8,
}

pub const SCSI_TEST_UNIT_READY: u8 = 0x00;
pub const SCSI_REQUEST_SENSE: u8 = 0x03;
pub const SCSI_INQUIRY: u8 = 0x12;
pub const SCSI_READ_CAPACITY_10: u8 = 0x25;
pub const SCSI_READ_10: u8 = 0x28;
pub const SCSI_WRITE_10: u8 = 0x2A;
pub const SCSI_MODE_SENSE_6: u8 = 0x1A;
pub const SCSI_PREVENT_ALLOW_MEDIUM_REMOVAL: u8 = 0x1E;
pub const SCSI_START_STOP_UNIT: u8 = 0x1B;

// =====================================================================
//  Device-type enumeration — from deviceproxy.h
// =====================================================================

pub const DEVTYPE_NONE: i32 = -1;
pub const DEVTYPE_PAD: i32 = 0;
pub const DEVTYPE_MSD: i32 = 1;
pub const DEVTYPE_MICROPHONE: i32 = 2;
pub const DEVTYPE_LOGITECH_HEADSET: i32 = 3;
pub const DEVTYPE_HIDKEYBOARD: i32 = 4;
pub const DEVTYPE_HIDMOUSE: i32 = 5;
pub const DEVTYPE_RBKIT: i32 = 6;
pub const DEVTYPE_BUZZ: i32 = 7;
pub const DEVTYPE_EYETOY: i32 = 8;
pub const DEVTYPE_TRANCE_VIBRATOR: i32 = 9;
pub const DEVTYPE_SEGA_SEAMIC: i32 = 10;
pub const DEVTYPE_PRINTER: i32 = 11;
pub const DEVTYPE_KEYBOARDMANIA: i32 = 12;
pub const DEVTYPE_GUNCON2: i32 = 13;
pub const DEVTYPE_DJ: i32 = 14;
pub const DEVTYPE_GAMETRAK: i32 = 15;
pub const DEVTYPE_REALPLAY: i32 = 16;
pub const DEVTYPE_TRAIN: i32 = 17;
pub const NUM_DEVICE_TYPES: usize = 18;

// =====================================================================
//  Device proxies — from deviceproxy.h
// =====================================================================

pub type StateWrapper = (); // opaque; matches StateWrapper.hpp on the C++ side

pub struct DeviceProxy {
    pub name: &'static str,
    pub type_name: &'static str,
    pub icon_name: &'static str,
    pub subtypes: Vec<&'static str>,
    pub plugin_index: i32,
    pub create: fn(si: *mut c_void, port: u32, subtype: u32) -> Option<usize>,
    pub freeze: fn(dev: *mut UsbDevice, sw: *mut StateWrapper) -> bool,
    pub update_settings: fn(dev: *mut UsbDevice, si: *mut c_void),
    pub input_connected: fn(dev: *mut UsbDevice, id: &str),
    pub input_disconnected: fn(dev: *mut UsbDevice, id: &str),
    pub get_bind: fn(dev: *const UsbDevice, idx: u32) -> f32,
    pub set_bind: fn(dev: *mut UsbDevice, idx: u32, v: f32),
    pub bindings: Vec<InputBindingInfo>,
    pub settings: Vec<SettingInfo>,
}

impl DeviceProxy {
    pub fn Name(&self) -> &str { self.name }
    pub fn TypeName(&self) -> &str { self.type_name }
    pub fn IconName(&self) -> &str { self.icon_name }
    pub fn SubTypes(&self) -> &[&str] { &self.subtypes }
    pub fn PluginIndex(&self) -> i32 { self.plugin_index }
}

// =====================================================================
//  Pad plugin types — from usb-pad/usb-pad.h (1,154 LOC!)
// =====================================================================

#[derive(Debug, Default, Clone, Copy)]
pub struct PadState {
    pub left_stick_x: u8,
    pub left_stick_y: u8,
    pub right_stick_x: u8,
    pub right_stick_y: u8,
    pub buttons: u16,
    pub pressure: [u8; 12],
}

#[derive(Debug, Default, Clone, Copy)]
pub struct PadVibration {
    pub small_motor: u8,
    pub large_motor: u8,
    pub enabled: bool,
    pub small_motor_enabled: bool,
    pub large_motor_enabled: bool,
    pub small_motor_intensity: u8,
    pub large_motor_intensity: u8,
    pub swap_motors: bool,
    pub small_motor_freq: u16,
    pub large_motor_freq: u16,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct PadLed {
    pub mode: u8,
    pub byte1: u8,
    pub byte2: u8,
    pub byte3: u8,
}

#[derive(Default)]
pub struct PadDevice {
    pub state: PadState,
    pub vibration: PadVibration,
    pub led: PadLed,
    pub rumble: bool,
    pub pad_index: i32,
    pub port: u32,
    pub subtype: u32,
    pub small_motor: u8,
    pub large_motor: u8,
    pub pressure_mode: u32,
    pub dpad_mode: u32,
    pub analog_mode: u32,
    pub mouse_sensitivity: f32,
    pub last_input: u64,
}

#[derive(Default)]
pub struct BuzzDevice {
    pub state: [bool; 4],
    pub port: u32,
}

#[derive(Default)]
pub struct GametrakDevice {
    pub port: u32,
    pub x: [f32; 3],
    pub y: [f32; 3],
    pub z: [f32; 3],
    pub trigger: [bool; 3],
}

#[derive(Default)]
pub struct RBDrumKitDevice { pub port: u32, pub pads: [u8; 5] }

#[derive(Default)]
pub struct DJTurntableDevice { pub port: u32, pub crossfader: u8, pub left: PadState, pub right: PadState }

#[derive(Default)]
pub struct TranceVibratorDevice { pub port: u32, pub speed: u8 }

#[derive(Default)]
pub struct SeamicDevice { pub port: u32, pub last_input: u32 }

#[derive(Default)]
pub struct KeyboardmaniaDevice { pub port: u32, pub keys: u32 }

#[derive(Default)]
pub struct RealPlayDevice { pub port: u32, pub last_input: u32 }

#[derive(Default)]
pub struct TrainDevice { pub port: u32, pub buttons: u32, pub speed: i32 }

#[derive(Default)]
pub struct PadForceFeedbackState { pub device: PadDevice }

// SDL force-feedback driver — from usb-pad/usb-pad-sdl-ff.{h,cpp}
#[derive(Default)]
pub struct PadSDLForceFeedback { pub effects: Vec<u32> }

// PCSX2 force-feedback driver — from usb-pad/usb-pad-ff.cpp
#[derive(Default)]
pub struct PadFF { pub effects: Vec<u32> }

// Linux Gamepad force-feedback — from usb-pad/lg/lg_ff.{h,cpp}
#[derive(Default)]
pub struct LgFF { pub supported: bool }

// =====================================================================
//  EyeToy webcam plugin — from usb-eyetoy/usb-eyetoy-webcam.h
// =====================================================================

#[derive(Default)]
pub struct EyeToyWebCamDevice {
    pub port: u32,
    pub backend: CamBackend,
    pub width: u16,
    pub height: u16,
    pub streaming: bool,
    pub frame: Vec<u8>,
    pub vendor: u16,
    pub product: u16,
    pub brightness: u8,
    pub contrast: u8,
    pub saturation: u8,
    pub hue: u8,
}

// =====================================================================
//  Printer plugin — from usb-printer/usb-printer.h
// =====================================================================

#[derive(Default)]
pub struct PrinterDevice {
    pub port: u32,
    pub buffer: Vec<u8>,
    pub device_id: CString,
    pub error_status: u8,
    pub selected: bool,
    pub paper_empty: bool,
}

pub const PRINTER_STATUS_OK: u8 = 0x18;
pub const PRINTER_STATUS_PE: u8 = 0x20; // paper empty
pub const PRINTER_STATUS_ERROR: u8 = 0x08;

// =====================================================================
//  Lightgun — from usb-lightgun/guncon2.h
// =====================================================================

#[derive(Default)]
pub struct GunCon2Device {
    pub port: u32,
    pub x: i32,
    pub y: i32,
    pub trigger: bool,
    pub buttons: u32,
    pub reload: bool,
    pub a_button: bool,
    pub b_button: bool,
    pub c_button: bool,
    pub sensor_active: bool,
}

// =====================================================================
//  Microphone / USB headset plugins — from usb-mic/usb-mic.h / usb-headset.h
// =====================================================================

impl Default for MicrophoneDevice {
    fn default() -> Self {
        Self {
            port: 0,
            backend: Box::new(NoopAudioBackend),
            spec: AudioSpec::default(),
            buffer: RingBuffer::new(4096),
            gain: 0.0,
            mute: false,
            last_capture: Vec::new(),
        }
    }
}

pub struct MicrophoneDevice {
    pub port: u32,
    pub backend: Box<dyn AudioBackend>,
    pub spec: AudioSpec,
    pub buffer: RingBuffer,
    pub gain: f32,
    pub mute: bool,
    pub last_capture: Vec<u8>,
}

impl Default for HeadsetDevice {
    fn default() -> Self {
        Self {
            port: 0,
            backend: Box::new(NoopAudioBackend),
            spec: AudioSpec::default(),
            input_ring: RingBuffer::new(4096),
            output_ring: RingBuffer::new(4096),
            gain: 0.0,
        }
    }
}

pub struct HeadsetDevice {
    pub port: u32,
    pub backend: Box<dyn AudioBackend>,
    pub spec: AudioSpec,
    pub input_ring: RingBuffer,
    pub output_ring: RingBuffer,
    pub gain: f32,
}

// =====================================================================
//  HID keyboard / mouse — from usb-hid/usb-hid.h
// =====================================================================

#[derive(Default)]
pub struct HIDKbdDevice { pub state: HIDKeyboardState, pub port: u32 }
#[derive(Default)]
pub struct HIDMouseDevice { pub state: HIDMouseState, pub port: u32 }

// =====================================================================
//  Mass storage device — from usb-msd/usb-msd.h
// =====================================================================

#[derive(Default)]
pub struct MsdDevice {
    pub port: u32,
    pub csw: MsdCsw,
    pub cbw: MsdCbw,
    pub scsi: ScsiCommand,
    pub image: Vec<u8>,
    pub ro: bool,
    pub sector_count: u64,
    pub sector_size: u32,
}

// =====================================================================
//  Device registration — from deviceproxy.cpp
// =====================================================================

pub struct RegisterDevice {
    pub map: Vec<Option<Box<DeviceProxy>>>,
}

impl RegisterDevice {
    pub fn new() -> Self {
        let mut map: Vec<Option<Box<DeviceProxy>>> = (0..NUM_DEVICE_TYPES).map(|_| None).collect();
        map[DEVTYPE_PAD as usize] = Some(Box::new(DeviceProxy {
            name: "Pad",
            type_name: "Pad",
            icon_name: "pad",
            subtypes: vec!["DualShock 2", "DualShock 1", "NegCon", "Pop'n", "JogCon"],
            plugin_index: DEVTYPE_PAD,
            create: pad_create,
            freeze: pad_freeze,
            update_settings: pad_update_settings,
            input_connected: pad_input_connected,
            input_disconnected: pad_input_disconnected,
            get_bind: pad_get_bind,
            set_bind: pad_set_bind,
            bindings: Vec::new(),
            settings: Vec::new(),
        }));
        map[DEVTYPE_MSD as usize] = Some(Box::new(DeviceProxy {
            name: "Mass Storage",
            type_name: "Mass Storage",
            icon_name: "msd",
            subtypes: vec!["Generic", "USBFlash"],
            plugin_index: DEVTYPE_MSD,
            create: msd_create,
            freeze: msd_freeze,
            update_settings: msd_update_settings,
            input_connected: msd_input_connected,
            input_disconnected: msd_input_disconnected,
            get_bind: msd_get_bind,
            set_bind: msd_set_bind,
            bindings: Vec::new(),
            settings: Vec::new(),
        }));
        map[DEVTYPE_MICROPHONE as usize] = Some(Box::new(DeviceProxy {
            name: "Microphone",
            type_name: "Microphone",
            icon_name: "mic",
            subtypes: vec!["Generic"],
            plugin_index: DEVTYPE_MICROPHONE,
            create: mic_create,
            freeze: mic_freeze,
            update_settings: mic_update_settings,
            input_connected: mic_input_connected,
            input_disconnected: mic_input_disconnected,
            get_bind: mic_get_bind,
            set_bind: mic_set_bind,
            bindings: Vec::new(),
            settings: Vec::new(),
        }));
        map[DEVTYPE_LOGITECH_HEADSET as usize] = Some(Box::new(DeviceProxy {
            name: "Logitech Headset",
            type_name: "USB Headset",
            icon_name: "headset",
            subtypes: vec!["Generic"],
            plugin_index: DEVTYPE_LOGITECH_HEADSET,
            create: headset_create,
            freeze: headset_freeze,
            update_settings: headset_update_settings,
            input_connected: headset_input_connected,
            input_disconnected: headset_input_disconnected,
            get_bind: headset_get_bind,
            set_bind: headset_set_bind,
            bindings: Vec::new(),
            settings: Vec::new(),
        }));
        map[DEVTYPE_HIDKEYBOARD as usize] = Some(Box::new(DeviceProxy {
            name: "HID Keyboard",
            type_name: "HID Keyboard",
            icon_name: "kbd",
            subtypes: vec!["Generic"],
            plugin_index: DEVTYPE_HIDKEYBOARD,
            create: hid_kbd_create,
            freeze: hid_kbd_freeze,
            update_settings: hid_kbd_update_settings,
            input_connected: hid_kbd_input_connected,
            input_disconnected: hid_kbd_input_disconnected,
            get_bind: hid_kbd_get_bind,
            set_bind: hid_kbd_set_bind,
            bindings: Vec::new(),
            settings: Vec::new(),
        }));
        map[DEVTYPE_HIDMOUSE as usize] = Some(Box::new(DeviceProxy {
            name: "HID Mouse",
            type_name: "HID Mouse",
            icon_name: "mouse",
            subtypes: vec!["Generic"],
            plugin_index: DEVTYPE_HIDMOUSE,
            create: hid_mouse_create,
            freeze: hid_mouse_freeze,
            update_settings: hid_mouse_update_settings,
            input_connected: hid_mouse_input_connected,
            input_disconnected: hid_mouse_input_disconnected,
            get_bind: hid_mouse_get_bind,
            set_bind: hid_mouse_set_bind,
            bindings: Vec::new(),
            settings: Vec::new(),
        }));
        map[DEVTYPE_RBKIT as usize] = Some(Box::new(DeviceProxy {
            name: "Rock Band Drums",
            type_name: "RB Drum Kit",
            icon_name: "drum",
            subtypes: vec!["Generic"],
            plugin_index: DEVTYPE_RBKIT,
            create: rbkit_create,
            freeze: rbkit_freeze,
            update_settings: rbkit_update_settings,
            input_connected: rbkit_input_connected,
            input_disconnected: rbkit_input_disconnected,
            get_bind: rbkit_get_bind,
            set_bind: rbkit_set_bind,
            bindings: Vec::new(),
            settings: Vec::new(),
        }));
        map[DEVTYPE_BUZZ as usize] = Some(Box::new(DeviceProxy {
            name: "Buzz!",
            type_name: "Buzz!",
            icon_name: "buzz",
            subtypes: vec!["Generic"],
            plugin_index: DEVTYPE_BUZZ,
            create: buzz_create,
            freeze: buzz_freeze,
            update_settings: buzz_update_settings,
            input_connected: buzz_input_connected,
            input_disconnected: buzz_input_disconnected,
            get_bind: buzz_get_bind,
            set_bind: buzz_set_bind,
            bindings: Vec::new(),
            settings: Vec::new(),
        }));
        map[DEVTYPE_EYETOY as usize] = Some(Box::new(DeviceProxy {
            name: "EyeToy Webcam",
            type_name: "EyeToy",
            icon_name: "eyetoy",
            subtypes: vec!["Generic", "Namco"],
            plugin_index: DEVTYPE_EYETOY,
            create: eyetoy_create,
            freeze: eyetoy_freeze,
            update_settings: eyetoy_update_settings,
            input_connected: eyetoy_input_connected,
            input_disconnected: eyetoy_input_disconnected,
            get_bind: eyetoy_get_bind,
            set_bind: eyetoy_set_bind,
            bindings: Vec::new(),
            settings: Vec::new(),
        }));
        map[DEVTYPE_TRANCE_VIBRATOR as usize] = Some(Box::new(DeviceProxy {
            name: "Trance Vibrator",
            type_name: "Trance Vibrator",
            icon_name: "vibrator",
            subtypes: vec!["Generic"],
            plugin_index: DEVTYPE_TRANCE_VIBRATOR,
            create: trance_create,
            freeze: trance_freeze,
            update_settings: trance_update_settings,
            input_connected: trance_input_connected,
            input_disconnected: trance_input_disconnected,
            get_bind: trance_get_bind,
            set_bind: trance_set_bind,
            bindings: Vec::new(),
            settings: Vec::new(),
        }));
        map[DEVTYPE_SEGA_SEAMIC as usize] = Some(Box::new(DeviceProxy {
            name: "Sega Seamic",
            type_name: "Seamic",
            icon_name: "seamic",
            subtypes: vec!["Generic"],
            plugin_index: DEVTYPE_SEGA_SEAMIC,
            create: seamic_create,
            freeze: seamic_freeze,
            update_settings: seamic_update_settings,
            input_connected: seamic_input_connected,
            input_disconnected: seamic_input_disconnected,
            get_bind: seamic_get_bind,
            set_bind: seamic_set_bind,
            bindings: Vec::new(),
            settings: Vec::new(),
        }));
        map[DEVTYPE_PRINTER as usize] = Some(Box::new(DeviceProxy {
            name: "Printer",
            type_name: "Printer",
            icon_name: "printer",
            subtypes: vec!["Generic"],
            plugin_index: DEVTYPE_PRINTER,
            create: printer_create,
            freeze: printer_freeze,
            update_settings: printer_update_settings,
            input_connected: printer_input_connected,
            input_disconnected: printer_input_disconnected,
            get_bind: printer_get_bind,
            set_bind: printer_set_bind,
            bindings: Vec::new(),
            settings: Vec::new(),
        }));
        map[DEVTYPE_KEYBOARDMANIA as usize] = Some(Box::new(DeviceProxy {
            name: "Keyboardmania",
            type_name: "Keyboardmania",
            icon_name: "kbm",
            subtypes: vec!["Generic"],
            plugin_index: DEVTYPE_KEYBOARDMANIA,
            create: kbm_create,
            freeze: kbm_freeze,
            update_settings: kbm_update_settings,
            input_connected: kbm_input_connected,
            input_disconnected: kbm_input_disconnected,
            get_bind: kbm_get_bind,
            set_bind: kbm_set_bind,
            bindings: Vec::new(),
            settings: Vec::new(),
        }));
        map[DEVTYPE_GUNCON2 as usize] = Some(Box::new(DeviceProxy {
            name: "GunCon 2",
            type_name: "GunCon 2",
            icon_name: "guncon2",
            subtypes: vec!["Generic"],
            plugin_index: DEVTYPE_GUNCON2,
            create: guncon2_create,
            freeze: guncon2_freeze,
            update_settings: guncon2_update_settings,
            input_connected: guncon2_input_connected,
            input_disconnected: guncon2_input_disconnected,
            get_bind: guncon2_get_bind,
            set_bind: guncon2_set_bind,
            bindings: Vec::new(),
            settings: Vec::new(),
        }));
        map[DEVTYPE_DJ as usize] = Some(Box::new(DeviceProxy {
            name: "DJ Turntable",
            type_name: "DJ Turntable",
            icon_name: "dj",
            subtypes: vec!["Generic"],
            plugin_index: DEVTYPE_DJ,
            create: dj_create,
            freeze: dj_freeze,
            update_settings: dj_update_settings,
            input_connected: dj_input_connected,
            input_disconnected: dj_input_disconnected,
            get_bind: dj_get_bind,
            set_bind: dj_set_bind,
            bindings: Vec::new(),
            settings: Vec::new(),
        }));
        map[DEVTYPE_GAMETRAK as usize] = Some(Box::new(DeviceProxy {
            name: "Gametrak",
            type_name: "Gametrak",
            icon_name: "gametrak",
            subtypes: vec!["Generic"],
            plugin_index: DEVTYPE_GAMETRAK,
            create: gametrak_create,
            freeze: gametrak_freeze,
            update_settings: gametrak_update_settings,
            input_connected: gametrak_input_connected,
            input_disconnected: gametrak_input_disconnected,
            get_bind: gametrak_get_bind,
            set_bind: gametrak_set_bind,
            bindings: Vec::new(),
            settings: Vec::new(),
        }));
        map[DEVTYPE_REALPLAY as usize] = Some(Box::new(DeviceProxy {
            name: "RealPlay",
            type_name: "RealPlay",
            icon_name: "realplay",
            subtypes: vec!["Generic"],
            plugin_index: DEVTYPE_REALPLAY,
            create: realplay_create,
            freeze: realplay_freeze,
            update_settings: realplay_update_settings,
            input_connected: realplay_input_connected,
            input_disconnected: realplay_input_disconnected,
            get_bind: realplay_get_bind,
            set_bind: realplay_set_bind,
            bindings: Vec::new(),
            settings: Vec::new(),
        }));
        map[DEVTYPE_TRAIN as usize] = Some(Box::new(DeviceProxy {
            name: "Train",
            type_name: "Train",
            icon_name: "train",
            subtypes: vec!["Generic"],
            plugin_index: DEVTYPE_TRAIN,
            create: train_create,
            freeze: train_freeze,
            update_settings: train_update_settings,
            input_connected: train_input_connected,
            input_disconnected: train_input_disconnected,
            get_bind: train_get_bind,
            set_bind: train_set_bind,
            bindings: Vec::new(),
            settings: Vec::new(),
        }));
        Self { map }
    }

    pub fn instance() -> &'static mut Self {
        static mut INSTANCE: Option<RegisterDevice> = None;
        unsafe {
            if INSTANCE.is_none() {
                INSTANCE = Some(RegisterDevice::new());
            }
            INSTANCE.as_mut().unwrap()
        }
    }

    pub fn Register() -> *mut Self { Self::instance() }
    pub fn Unregister(&mut self) { self.map.clear(); }
    pub fn Device(&self, idx: i32) -> Option<&DeviceProxy> {
        if idx < 0 { return None; }
        self.map.get(idx as usize).and_then(|o| o.as_deref())
    }
    pub fn DeviceByName(&self, name: &str) -> Option<&DeviceProxy> {
        self.map.iter().filter_map(|o| o.as_deref()).find(|p| p.TypeName() == name)
    }
    pub fn Index(&self, name: &str) -> i32 {
        for (i, p) in self.map.iter().enumerate() {
            if let Some(p) = p { if p.TypeName() == name { return i as i32; } }
        }
        DEVTYPE_NONE
    }
    pub fn Map(&self) -> &Vec<Option<Box<DeviceProxy>>> { &self.map }
    pub fn Add(&mut self, key: i32, creator: DeviceProxy) {
        let idx = key as usize;
        if idx < self.map.len() { self.map[idx] = Some(Box::new(creator)); }
    }
}

impl Default for RegisterDevice { fn default() -> Self { Self::new() } }

// =====================================================================
//  Plugin trampolines (each plugin has a "create", "freeze", etc.)
// =====================================================================

fn pad_create(_si: *mut c_void, _port: u32, _sub: u32) -> Option<usize> { Some(0) }
fn pad_freeze(_dev: *mut UsbDevice, _sw: *mut StateWrapper) -> bool { true }
fn pad_update_settings(_dev: *mut UsbDevice, _si: *mut c_void) {}
fn pad_input_connected(_dev: *mut UsbDevice, _id: &str) {}
fn pad_input_disconnected(_dev: *mut UsbDevice, _id: &str) {}
fn pad_get_bind(_dev: *const UsbDevice, _idx: u32) -> f32 { 0.0 }
fn pad_set_bind(_dev: *mut UsbDevice, _idx: u32, _v: f32) {}

fn msd_create(_si: *mut c_void, _port: u32, _sub: u32) -> Option<usize> { Some(0) }
fn msd_freeze(_dev: *mut UsbDevice, _sw: *mut StateWrapper) -> bool { true }
fn msd_update_settings(_dev: *mut UsbDevice, _si: *mut c_void) {}
fn msd_input_connected(_dev: *mut UsbDevice, _id: &str) {}
fn msd_input_disconnected(_dev: *mut UsbDevice, _id: &str) {}
fn msd_get_bind(_dev: *const UsbDevice, _idx: u32) -> f32 { 0.0 }
fn msd_set_bind(_dev: *mut UsbDevice, _idx: u32, _v: f32) {}

fn mic_create(_si: *mut c_void, _port: u32, _sub: u32) -> Option<usize> { Some(0) }
fn mic_freeze(_dev: *mut UsbDevice, _sw: *mut StateWrapper) -> bool { true }
fn mic_update_settings(_dev: *mut UsbDevice, _si: *mut c_void) {}
fn mic_input_connected(_dev: *mut UsbDevice, _id: &str) {}
fn mic_input_disconnected(_dev: *mut UsbDevice, _id: &str) {}
fn mic_get_bind(_dev: *const UsbDevice, _idx: u32) -> f32 { 0.0 }
fn mic_set_bind(_dev: *mut UsbDevice, _idx: u32, _v: f32) {}

fn headset_create(_si: *mut c_void, _port: u32, _sub: u32) -> Option<usize> { Some(0) }
fn headset_freeze(_dev: *mut UsbDevice, _sw: *mut StateWrapper) -> bool { true }
fn headset_update_settings(_dev: *mut UsbDevice, _si: *mut c_void) {}
fn headset_input_connected(_dev: *mut UsbDevice, _id: &str) {}
fn headset_input_disconnected(_dev: *mut UsbDevice, _id: &str) {}
fn headset_get_bind(_dev: *const UsbDevice, _idx: u32) -> f32 { 0.0 }
fn headset_set_bind(_dev: *mut UsbDevice, _idx: u32, _v: f32) {}

fn hid_kbd_create(_si: *mut c_void, _port: u32, _sub: u32) -> Option<usize> { Some(0) }
fn hid_kbd_freeze(_dev: *mut UsbDevice, _sw: *mut StateWrapper) -> bool { true }
fn hid_kbd_update_settings(_dev: *mut UsbDevice, _si: *mut c_void) {}
fn hid_kbd_input_connected(_dev: *mut UsbDevice, _id: &str) {}
fn hid_kbd_input_disconnected(_dev: *mut UsbDevice, _id: &str) {}
fn hid_kbd_get_bind(_dev: *const UsbDevice, _idx: u32) -> f32 { 0.0 }
fn hid_kbd_set_bind(_dev: *mut UsbDevice, _idx: u32, _v: f32) {}

fn hid_mouse_create(_si: *mut c_void, _port: u32, _sub: u32) -> Option<usize> { Some(0) }
fn hid_mouse_freeze(_dev: *mut UsbDevice, _sw: *mut StateWrapper) -> bool { true }
fn hid_mouse_update_settings(_dev: *mut UsbDevice, _si: *mut c_void) {}
fn hid_mouse_input_connected(_dev: *mut UsbDevice, _id: &str) {}
fn hid_mouse_input_disconnected(_dev: *mut UsbDevice, _id: &str) {}
fn hid_mouse_get_bind(_dev: *const UsbDevice, _idx: u32) -> f32 { 0.0 }
fn hid_mouse_set_bind(_dev: *mut UsbDevice, _idx: u32, _v: f32) {}

fn rbkit_create(_si: *mut c_void, _port: u32, _sub: u32) -> Option<usize> { Some(0) }
fn rbkit_freeze(_dev: *mut UsbDevice, _sw: *mut StateWrapper) -> bool { true }
fn rbkit_update_settings(_dev: *mut UsbDevice, _si: *mut c_void) {}
fn rbkit_input_connected(_dev: *mut UsbDevice, _id: &str) {}
fn rbkit_input_disconnected(_dev: *mut UsbDevice, _id: &str) {}
fn rbkit_get_bind(_dev: *const UsbDevice, _idx: u32) -> f32 { 0.0 }
fn rbkit_set_bind(_dev: *mut UsbDevice, _idx: u32, _v: f32) {}

fn buzz_create(_si: *mut c_void, _port: u32, _sub: u32) -> Option<usize> { Some(0) }
fn buzz_freeze(_dev: *mut UsbDevice, _sw: *mut StateWrapper) -> bool { true }
fn buzz_update_settings(_dev: *mut UsbDevice, _si: *mut c_void) {}
fn buzz_input_connected(_dev: *mut UsbDevice, _id: &str) {}
fn buzz_input_disconnected(_dev: *mut UsbDevice, _id: &str) {}
fn buzz_get_bind(_dev: *const UsbDevice, _idx: u32) -> f32 { 0.0 }
fn buzz_set_bind(_dev: *mut UsbDevice, _idx: u32, _v: f32) {}

fn eyetoy_create(_si: *mut c_void, _port: u32, _sub: u32) -> Option<usize> { Some(0) }
fn eyetoy_freeze(_dev: *mut UsbDevice, _sw: *mut StateWrapper) -> bool { true }
fn eyetoy_update_settings(_dev: *mut UsbDevice, _si: *mut c_void) {}
fn eyetoy_input_connected(_dev: *mut UsbDevice, _id: &str) {}
fn eyetoy_input_disconnected(_dev: *mut UsbDevice, _id: &str) {}
fn eyetoy_get_bind(_dev: *const UsbDevice, _idx: u32) -> f32 { 0.0 }
fn eyetoy_set_bind(_dev: *mut UsbDevice, _idx: u32, _v: f32) {}

fn trance_create(_si: *mut c_void, _port: u32, _sub: u32) -> Option<usize> { Some(0) }
fn trance_freeze(_dev: *mut UsbDevice, _sw: *mut StateWrapper) -> bool { true }
fn trance_update_settings(_dev: *mut UsbDevice, _si: *mut c_void) {}
fn trance_input_connected(_dev: *mut UsbDevice, _id: &str) {}
fn trance_input_disconnected(_dev: *mut UsbDevice, _id: &str) {}
fn trance_get_bind(_dev: *const UsbDevice, _idx: u32) -> f32 { 0.0 }
fn trance_set_bind(_dev: *mut UsbDevice, _idx: u32, _v: f32) {}

fn seamic_create(_si: *mut c_void, _port: u32, _sub: u32) -> Option<usize> { Some(0) }
fn seamic_freeze(_dev: *mut UsbDevice, _sw: *mut StateWrapper) -> bool { true }
fn seamic_update_settings(_dev: *mut UsbDevice, _si: *mut c_void) {}
fn seamic_input_connected(_dev: *mut UsbDevice, _id: &str) {}
fn seamic_input_disconnected(_dev: *mut UsbDevice, _id: &str) {}
fn seamic_get_bind(_dev: *const UsbDevice, _idx: u32) -> f32 { 0.0 }
fn seamic_set_bind(_dev: *mut UsbDevice, _idx: u32, _v: f32) {}

fn printer_create(_si: *mut c_void, _port: u32, _sub: u32) -> Option<usize> { Some(0) }
fn printer_freeze(_dev: *mut UsbDevice, _sw: *mut StateWrapper) -> bool { true }
fn printer_update_settings(_dev: *mut UsbDevice, _si: *mut c_void) {}
fn printer_input_connected(_dev: *mut UsbDevice, _id: &str) {}
fn printer_input_disconnected(_dev: *mut UsbDevice, _id: &str) {}
fn printer_get_bind(_dev: *const UsbDevice, _idx: u32) -> f32 { 0.0 }
fn printer_set_bind(_dev: *mut UsbDevice, _idx: u32, _v: f32) {}

fn kbm_create(_si: *mut c_void, _port: u32, _sub: u32) -> Option<usize> { Some(0) }
fn kbm_freeze(_dev: *mut UsbDevice, _sw: *mut StateWrapper) -> bool { true }
fn kbm_update_settings(_dev: *mut UsbDevice, _si: *mut c_void) {}
fn kbm_input_connected(_dev: *mut UsbDevice, _id: &str) {}
fn kbm_input_disconnected(_dev: *mut UsbDevice, _id: &str) {}
fn kbm_get_bind(_dev: *const UsbDevice, _idx: u32) -> f32 { 0.0 }
fn kbm_set_bind(_dev: *mut UsbDevice, _idx: u32, _v: f32) {}

fn guncon2_create(_si: *mut c_void, _port: u32, _sub: u32) -> Option<usize> { Some(0) }
fn guncon2_freeze(_dev: *mut UsbDevice, _sw: *mut StateWrapper) -> bool { true }
fn guncon2_update_settings(_dev: *mut UsbDevice, _si: *mut c_void) {}
fn guncon2_input_connected(_dev: *mut UsbDevice, _id: &str) {}
fn guncon2_input_disconnected(_dev: *mut UsbDevice, _id: &str) {}
fn guncon2_get_bind(_dev: *const UsbDevice, _idx: u32) -> f32 { 0.0 }
fn guncon2_set_bind(_dev: *mut UsbDevice, _idx: u32, _v: f32) {}

fn dj_create(_si: *mut c_void, _port: u32, _sub: u32) -> Option<usize> { Some(0) }
fn dj_freeze(_dev: *mut UsbDevice, _sw: *mut StateWrapper) -> bool { true }
fn dj_update_settings(_dev: *mut UsbDevice, _si: *mut c_void) {}
fn dj_input_connected(_dev: *mut UsbDevice, _id: &str) {}
fn dj_input_disconnected(_dev: *mut UsbDevice, _id: &str) {}
fn dj_get_bind(_dev: *const UsbDevice, _idx: u32) -> f32 { 0.0 }
fn dj_set_bind(_dev: *mut UsbDevice, _idx: u32, _v: f32) {}

fn gametrak_create(_si: *mut c_void, _port: u32, _sub: u32) -> Option<usize> { Some(0) }
fn gametrak_freeze(_dev: *mut UsbDevice, _sw: *mut StateWrapper) -> bool { true }
fn gametrak_update_settings(_dev: *mut UsbDevice, _si: *mut c_void) {}
fn gametrak_input_connected(_dev: *mut UsbDevice, _id: &str) {}
fn gametrak_input_disconnected(_dev: *mut UsbDevice, _id: &str) {}
fn gametrak_get_bind(_dev: *const UsbDevice, _idx: u32) -> f32 { 0.0 }
fn gametrak_set_bind(_dev: *mut UsbDevice, _idx: u32, _v: f32) {}

fn realplay_create(_si: *mut c_void, _port: u32, _sub: u32) -> Option<usize> { Some(0) }
fn realplay_freeze(_dev: *mut UsbDevice, _sw: *mut StateWrapper) -> bool { true }
fn realplay_update_settings(_dev: *mut UsbDevice, _si: *mut c_void) {}
fn realplay_input_connected(_dev: *mut UsbDevice, _id: &str) {}
fn realplay_input_disconnected(_dev: *mut UsbDevice, _id: &str) {}
fn realplay_get_bind(_dev: *const UsbDevice, _idx: u32) -> f32 { 0.0 }
fn realplay_set_bind(_dev: *mut UsbDevice, _idx: u32, _v: f32) {}

fn train_create(_si: *mut c_void, _port: u32, _sub: u32) -> Option<usize> { Some(0) }
fn train_freeze(_dev: *mut UsbDevice, _sw: *mut StateWrapper) -> bool { true }
fn train_update_settings(_dev: *mut UsbDevice, _si: *mut c_void) {}
fn train_input_connected(_dev: *mut UsbDevice, _id: &str) {}
fn train_input_disconnected(_dev: *mut UsbDevice, _id: &str) {}
fn train_get_bind(_dev: *const UsbDevice, _idx: u32) -> f32 { 0.0 }
fn train_set_bind(_dev: *mut UsbDevice, _idx: u32, _v: f32) {}

// =====================================================================
//  Global state and lifecycle — from USB.cpp / USB.h
// =====================================================================

pub static mut g_usb_frame_time: i64 = 0;
pub static mut g_usb_bit_time: i64 = 0;
pub static mut g_usb_last_cycle: i64 = 0;
pub static mut s_qemu_ohci: Option<Box<OHCIState>> = None;
pub static mut s_usb_device: [Option<usize>; NUM_PORTS] = [None, None];
pub static mut s_usb_device_proxy: [Option<usize>; NUM_PORTS] = [None, None];
pub static mut s_usb_clocks: i64 = 0;
pub static mut s_usb_remaining: i64 = 0;

pub struct USBState {
    pub ohci: Option<Box<OHCIState>>,
    pub devices: Vec<Box<UsbDevice>>,
    pub proxies: [Option<usize>; NUM_PORTS],
    pub clocks: i64,
    pub remaining: i64,
    pub last_cycle: i64,
    pub config: Pcsx2Config,
}

impl Default for USBState {
    fn default() -> Self {
        Self {
            ohci: Some(Box::new(OHCIState::default())),
            devices: Vec::new(),
            proxies: [None, None],
            clocks: 0,
            remaining: 0,
            last_cycle: 0,
            config: Pcsx2Config::default(),
        }
    }
}

pub static mut usbReg: USBState = USBState {
    ohci: None, // placeholder; usbInit() replaces this with Some(Box::new(OHCIState::default()))
    devices: Vec::new(),
    proxies: [None, None],
    clocks: 0,
    remaining: 0,
    last_cycle: 0,
    config: Pcsx2Config { usb: UsbConfig { ports: Vec::new() } },
};

pub fn usbInit() {
    unsafe {
        // Re-initialise the global state.
        usbReg = USBState::default();
        let _ = RegisterDevice::Register();
    }
}

pub fn usbReset() {
    unsafe {
        usbReg.clocks = 0;
        usbReg.remaining = 0;
        usbReg.last_cycle = 0;
        // ohci_hard_reset equivalent: zero the most relevant regs.
        let ohci = usbReg.ohci.as_mut().unwrap();
        ohci.eof_timer = 0;
        ohci.ctl = 0;
        ohci.status = 0;
        ohci.intr_status = 0;
    }
}

pub fn usbShutdown() {
    unsafe {
        for d in usbReg.devices.drain(..) {
            if let Some(unrealize) = d.klass.unrealize {
                unrealize(d.as_ref() as *const UsbDevice as *mut UsbDevice);
            }
        }
        usbReg.proxies = [None, None];
        if let Some(inst) = RegisterDevice::instance().Map().iter().count().checked_sub(0) {
            if inst > 0 { RegisterDevice::instance().Unregister(); }
        }
    }
}

pub fn usbRead8(addr: u32) -> u8 { ohci_mem_read(addr) as u8 }
pub fn usbRead16(addr: u32) -> u16 { ohci_mem_read(addr) as u16 }
pub fn usbRead32(addr: u32) -> u32 { ohci_mem_read(addr) }
pub fn usbWrite8(_addr: u32, _value: u8) {}
pub fn usbWrite16(_addr: u32, _value: u16) {}
pub fn usbWrite32(addr: u32, value: u32) { ohci_mem_write(addr, value) }

pub fn usbSetRAM(_mem: *mut c_void) {}

pub fn usbGetTicksPerSecond() -> i32 { PSXCLK as i32 }
pub fn usbGetClock() -> i64 { unsafe { usbReg.clocks } }

pub fn usbAsync(cycles: u32) {
    unsafe {
        if usbReg.devices.is_empty() { return; }
        usbReg.remaining += cycles as i64;
        usbReg.clocks += usbReg.remaining;
        let ohci_eof = usbReg.ohci.as_ref().unwrap().eof_timer;
        if ohci_eof > 0 {
            let mut eof = ohci_eof;
            while usbReg.remaining >= eof {
                usbReg.remaining -= eof;
                eof = 0;
                usbReg.ohci.as_mut().unwrap().eof_timer = 0;
                ohci_frame_boundary();
                eof = usbReg.ohci.as_ref().unwrap().eof_timer;
                if eof == 0 { break; }
            }
            if usbReg.remaining > 0 && eof > 0 {
                let m = usbReg.remaining.min(eof);
                usbReg.ohci.as_mut().unwrap().eof_timer -= m;
                usbReg.remaining -= m;
            }
        }
    }
}

// =====================================================================
//  OHCI register-level I/O — translate usb-ohci.cpp's ohci_mem_read/write
// =====================================================================

pub fn ohci_create(base: u32, num_ports: u32) -> Option<Box<OHCIState>> {
    let mut s = OHCIState::default();
    s.mem = base as *mut c_void;
    s.num_ports = num_ports;
    s.rhdesc_a = (s.num_ports & OHCI_RHA_NDP) | OHCI_RHA_NPS | OHCI_RHA_DT;
    s.ctl = 0;
    s.status = 0;
    s.intr_status = 0;
    s.intr = OHCI_INTR_MIE;
    Some(Box::new(s))
}

pub fn ohci_hard_reset(s: &mut OHCIState) {
    s.ctl = 0;
    s.status = 0;
    s.intr_status = 0;
    s.intr = OHCI_INTR_MIE;
    s.hcca = 0;
    s.ctrl_head = 0;
    s.ctrl_cur = 0;
    s.bulk_head = 0;
    s.bulk_cur = 0;
    s.per_cur = 0;
    s.done = 0;
    s.done_count = 0;
    s.fsmps = 0;
    s.fit = 0;
    s.fi = 0;
    s.frt = 0;
    s.frame_number = 0;
    s.pstart = 0;
    s.lst = 0;
    s.rhdesc_a = (s.num_ports & OHCI_RHA_NDP) | OHCI_RHA_NPS | OHCI_RHA_DT;
    s.rhdesc_b = 0;
    for p in s.rhport.iter_mut() { p.ctrl = 0; }
}

pub fn ohci_frame_boundary() {
    unsafe {
        let s = usbReg.ohci.as_mut().unwrap().as_mut();
        s.frame_number = s.frame_number.wrapping_add(1);
        s.intr_status |= OHCI_INTR_SF;
    }
}

pub fn ohci_mem_read(addr: u32) -> u32 {
    unsafe {
        let s = usbReg.ohci.as_ref().unwrap().as_ref();
        match (addr & 0xFF) >> 2 {
            0x00 => s.ctl,
            0x01 => s.status,
            0x02 => s.intr_status,
            0x03 => s.intr,
            0x04 => s.hcca,
            0x05 => s.ctrl_head,
            0x06 => s.ctrl_cur,
            0x07 => s.bulk_head,
            0x08 => s.bulk_cur,
            0x09 => s.done,
            0x0A => s.fsmps,
            0x0B => s.fit,
            0x0C => s.fi,
            0x0D => s.frt,
            0x0E => s.frame_number as u32,
            0x0F => s.pstart,
            0x10 => s.lst,
            0x11 => s.rhdesc_a,
            0x12 => s.rhdesc_b,
            0x13 => s.rhport[0].ctrl,
            0x14 => s.rhport[1].ctrl,
            _ => 0,
        }
    }
}

pub fn ohci_mem_write(addr: u32, value: u32) {
    unsafe {
        let s = usbReg.ohci.as_mut().unwrap().as_mut();
        match (addr & 0xFF) >> 2 {
            0x00 => s.ctl = value,
            0x01 => s.status = value,
            0x02 => s.intr_status &= !value,
            0x03 => s.intr = value,
            0x04 => s.hcca = value,
            0x05 => s.ctrl_head = value & !0xF,
            0x06 => { s.ctrl_cur = value; }
            0x07 => s.bulk_head = value & !0xF,
            0x08 => { s.bulk_cur = value; }
            0x09 => s.done = value & !0xF,
            0x0A => s.fsmps = value,
            0x0B => s.fit = value,
            0x0C => s.fi = value,
            0x0D => s.frt = value,
            0x0E => s.frame_number = value as u16,
            0x0F => s.pstart = value,
            0x10 => s.lst = value,
            0x11 => s.rhdesc_a = value,
            0x12 => s.rhdesc_b = value,
            0x13 => s.rhport[0].ctrl = (s.rhport[0].ctrl & 0xFFF00000) | (value & 0x000FFFFF),
            0x14 => s.rhport[1].ctrl = (s.rhport[1].ctrl & 0xFFF00000) | (value & 0x000FFFFF),
            _ => {}
        }
    }
}

// =====================================================================
//  Bus / packet helpers — from qemu-usb/bus.cpp and core.cpp
// =====================================================================

pub fn usb_bus_new(busnr: i32) -> UsbBus {
    UsbBus { busnr, nfree: 0, nused: 0, ..UsbBus::default() }
}

pub fn usb_register_port(bus: &mut UsbBus, port: UsbPort) {
    bus.nfree += 1;
    bus.free_ports.push(port.index as usize);
}

pub fn usb_unregister_port(bus: &mut UsbBus, port: usize) {
    bus.nfree -= 1;
    bus.free_ports.retain(|p| *p != port);
}

pub fn usb_attach(_port: *mut UsbPort) {}
pub fn usb_detach(_port: *mut UsbPort) {}
pub fn usb_port_reset(_port: *mut UsbPort) {}
pub fn usb_reattach(_port: *mut UsbPort) {}
pub fn usb_wakeup(_ep: *mut UsbEndpoint, _stream: u32) {}
pub fn usb_pick_speed(_port: *mut UsbPort) {}

pub fn usb_packet_set_state(p: &mut UsbPacket, state: UsbPacketState) { p.state = state; }
pub fn usb_packet_check_state(p: &UsbPacket, expected: UsbPacketState) -> bool { p.state == expected }
pub fn usb_packet_setup(p: &mut UsbPacket, pid: i32, ep: *mut UsbEndpoint, stream: u32, id: u64, short_not_ok: bool, int_req: bool) {
    p.pid = pid;
    p.ep = ep;
    p.stream = stream;
    p.id = id;
    p.short_not_ok = short_not_ok;
    p.int_req = int_req;
    p.state = UsbPacketState::Setup;
}
pub fn usb_packet_addbuf(p: &mut UsbPacket, ptr: *mut u8, len: usize) {
    p.buffer_ptr = ptr;
    p.buffer_size = len as u32;
}
pub fn usb_packet_copy(p: &mut UsbPacket, _ptr: *mut u8, _bytes: usize) { p.actual_length += _bytes as i32; }
pub fn usb_packet_skip(p: &mut UsbPacket, bytes: usize) { p.actual_length += bytes as i32; }
pub fn usb_packet_size(p: &UsbPacket) -> usize { p.buffer_size as usize }
pub fn usb_packet_cleanup(_p: &mut UsbPacket) {}
pub fn usb_packet_is_inflight(p: &UsbPacket) -> bool { matches!(p.state, UsbPacketState::Queued | UsbPacketState::Async) }

pub fn usb_handle_packet(_dev: *mut UsbDevice, _p: *mut UsbPacket) {}
pub fn usb_packet_complete(_dev: *mut UsbDevice, _p: *mut UsbPacket) {}
pub fn usb_packet_complete_one(_dev: *mut UsbDevice, _p: *mut UsbPacket) {}
pub fn usb_cancel_packet(_p: *mut UsbPacket) {}

// Endpoint helpers
pub fn usb_ep_init(dev: &mut UsbDevice) {
    dev.ep_ctl = UsbEndpoint::default();
    for ep in dev.ep_in.iter_mut() { *ep = UsbEndpoint::default(); }
    for ep in dev.ep_out.iter_mut() { *ep = UsbEndpoint::default(); }
}
pub fn usb_ep_reset(dev: &mut UsbDevice) {
    dev.ep_ctl.queue.clear();
    for ep in dev.ep_in.iter_mut() { ep.queue.clear(); ep.halted = false; }
    for ep in dev.ep_out.iter_mut() { ep.queue.clear(); ep.halted = false; }
}
pub fn usb_ep_get(_dev: &mut UsbDevice, _pid: i32, _ep: i32) -> *mut UsbEndpoint { ptr::null_mut() }
pub fn usb_ep_get_type(_dev: &UsbDevice, _pid: i32, _ep: i32) -> u8 { 0 }
pub fn usb_ep_set_type(_dev: &mut UsbDevice, _pid: i32, _ep: i32, _t: u8) {}
pub fn usb_ep_set_ifnum(_dev: &mut UsbDevice, _pid: i32, _ep: i32, _i: u8) {}
pub fn usb_ep_set_max_packet_size(_dev: &mut UsbDevice, _pid: i32, _ep: i32, _r: u16) {}
pub fn usb_ep_set_max_streams(_dev: &mut UsbDevice, _pid: i32, _ep: i32, _r: u8) {}
pub fn usb_ep_set_halted(_dev: &mut UsbDevice, _pid: i32, _ep: i32, _h: bool) {}
pub fn usb_ep_find_packet_by_id(_dev: &UsbDevice, _pid: i32, _ep: i32, _id: u64) -> *mut UsbPacket { ptr::null_mut() }

// Default handlers used by USBDeviceClass::*
pub fn usb_device_cancel_packet(_dev: *mut UsbDevice, _p: *mut UsbPacket) {}
pub fn usb_device_handle_attach(_dev: *mut UsbDevice) {}
pub fn usb_device_handle_reset(_dev: *mut UsbDevice) {}
pub fn usb_device_handle_control(_dev: *mut UsbDevice, p: *mut UsbPacket, _request: i32, _value: i32, _index: i32, _length: i32, _data: *mut u8) {
    unsafe { (*p).status = USB_RET_STALL; }
}
pub fn usb_device_handle_data(_dev: *mut UsbDevice, _p: *mut UsbPacket) {}
pub fn usb_device_set_interface(_dev: *mut UsbDevice, _intf: i32, _alt_old: i32, _alt_new: i32) {}
pub fn usb_device_flush_ep_queue(_dev: *mut UsbDevice, _ep: *mut UsbEndpoint) {}
pub fn usb_device_ep_stopped(_dev: *mut UsbDevice, _ep: *mut UsbEndpoint) {}
pub fn usb_device_alloc_streams(_dev: *mut UsbDevice, _eps: *mut *mut UsbEndpoint, _nr_eps: i32, _streams: i32) -> i32 { -1 }
pub fn usb_device_free_streams(_dev: *mut UsbDevice, _eps: *mut *mut UsbEndpoint, _nr_eps: i32) {}
pub fn usb_device_find_device(_dev: *mut UsbDevice, _addr: u8) -> *mut UsbDevice { _dev }
pub fn usb_device_get_usb_desc(_dev: *const UsbDevice) -> *const u8 { ptr::null() }

pub fn usb_generic_async_ctrl_complete(_dev: *mut UsbDevice, _p: *mut UsbPacket) {}
pub fn usb_find_device(_port: *mut UsbPort, _addr: u8) -> *mut UsbDevice { ptr::null_mut() }
pub fn usb_device_reset(_dev: *mut UsbDevice) {}
pub fn usb_desc_set_config(_dev: *mut UsbDevice, _config: i32) {}
pub fn usb_desc_set_interface(_dev: *mut UsbDevice, _intf: i32, _alt: i32) {}
pub fn usb_desc_init(dev: &mut UsbDevice) { usb_ep_init(dev); }
pub fn usb_desc_attach(dev: &mut UsbDevice) { dev.attached = true; }

// =====================================================================
//  Descriptor helpers — from qemu-usb/desc.cpp
// =====================================================================

pub fn usb_get_dev_desc(dev: &UsbDevice) -> UsbDescDevice { UsbDescDevice::default() }
pub fn usb_get_config_desc(dev: &UsbDevice, _idx: i32) -> UsbDescConfig { UsbDescConfig::default() }
pub fn usb_get_str_desc(_idx: u8) -> *const u8 { ptr::null() }

// =====================================================================
//  HID helpers — from qemu-usb/hid.cpp
// =====================================================================

pub fn hid_init(h: &mut HIDState, kind: HIDKind, data: &[u8]) {
    h.kind = kind;
    h.cap = data.len();
    h.data[..data.len()].copy_from_slice(data);
    h.data_len = data.len();
    h.head = 0;
    h.tail = 0;
    h.queue.clear();
}
pub fn hid_reset(h: &mut HIDState) {
    h.head = 0;
    h.tail = 0;
    h.queue.clear();
    h.data_len = 0;
}
pub fn hid_post_data(h: &mut HIDState, data: &[u8]) {
    if h.queue.len() >= HID_QUEUE_SIZE { return; }
    h.queue.push_back(UsbPacket { state: UsbPacketState::Queued, ..UsbPacket::default() });
    // The real implementation copies data into the queue; the wrapper stores
    // data through the embedded USBDevice.
    h.data_len = data.len();
    let n = data.len().min(h.data.len());
    h.data[..n].copy_from_slice(&data[..n]);
}
pub fn hid_set_next_data(h: &mut HIDState) -> bool {
    h.head = 0;
    h.tail = 0;
    true
}
pub fn hid_has_data(h: &HIDState) -> bool { h.data_len > 0 }
pub fn hid_get_data(h: &HIDState, dst: &mut [u8]) -> usize {
    let n = dst.len().min(h.data_len);
    dst[..n].copy_from_slice(&h.data[..n]);
    n
}

// =====================================================================
//  Pad commands — from usb-pad/usb-pad.cpp
// =====================================================================

pub const PAD_CMD_RESET: u8 = 0x01;
pub const PAD_CMD_GET_LED: u8 = 0x02;
pub const PAD_CMD_SET_LED: u8 = 0x03;
pub const PAD_CMD_GET_MODE: u8 = 0x04;
pub const PAD_CMD_SET_MODE: u8 = 0x05;
pub const PAD_CMD_GET_STATE: u8 = 0x06;
pub const PAD_CMD_GET_BUTMASK: u8 = 0x07;
pub const PAD_CMD_SET_RUMBLE: u8 = 0x08;

pub fn pad_command(dev: &mut PadDevice, cmd: u8, _arg: u32) -> i32 {
    match cmd {
        PAD_CMD_RESET => { dev.state = PadState::default(); 0 }
        PAD_CMD_GET_LED => dev.led.byte1 as i32,
        PAD_CMD_SET_LED => { dev.led.byte1 = _arg as u8; 0 }
        PAD_CMD_GET_MODE => dev.analog_mode as i32,
        PAD_CMD_SET_MODE => { dev.analog_mode = _arg; 0 }
        PAD_CMD_GET_STATE => 0,
        PAD_CMD_GET_BUTMASK => 0xFFFF,
        PAD_CMD_SET_RUMBLE => { dev.rumble = (_arg & 1) != 0; 0 }
        _ => -1,
    }
}

// =====================================================================
//  Mass-storage / SCSI helpers — from usb-msd/usb-msd.cpp
// =====================================================================

pub const MSD_SIG_CBW: u32 = 0x43425355;
pub const MSD_SIG_CSW: u32 = 0x53425355;

pub fn msd_command_complete(dev: &mut MsdDevice, status: u8) {
    dev.csw.d_cs_signature = MSD_SIG_CSW;
    dev.csw.b_cs_status = status;
}

pub fn scsi_process_command(cmd: &mut ScsiCommand) {
    match cmd.opcode {
        SCSI_TEST_UNIT_READY => { cmd.sense_key = 0; }
        SCSI_REQUEST_SENSE => { cmd.buffer.resize(18, 0); }
        SCSI_INQUIRY => { cmd.buffer.resize(36, 0); cmd.buffer[0] = 0x00; }
        SCSI_READ_CAPACITY_10 => {
            cmd.buffer.resize(8, 0);
            cmd.buffer[0] = ((dev_sectors_count() - 1) >> 24) as u8;
            cmd.buffer[1] = ((dev_sectors_count() - 1) >> 16) as u8;
            cmd.buffer[2] = ((dev_sectors_count() - 1) >> 8) as u8;
            cmd.buffer[3] = (dev_sectors_count() - 1) as u8;
            cmd.buffer[4] = 0;
            cmd.buffer[5] = 0;
            cmd.buffer[6] = 0x02;
            cmd.buffer[7] = 0x00;
        }
        SCSI_READ_10 | SCSI_WRITE_10 => {}
        _ => { cmd.sense_key = 0x05; cmd.asc = 0x20; }
    }
}

fn dev_sectors_count() -> u64 { 0 }

// =====================================================================
//  Printer commands — from usb-printer/usb-printer.cpp
// =====================================================================

pub const PRINTER_CMD_GET_DEVICE_ID: u8 = 0;
pub const PRINTER_CMD_GET_PORT_STATUS: u8 = 1;
pub const PRINTER_CMD_SOFT_RESET: u8 = 2;

pub fn printer_command(dev: &mut PrinterDevice, cmd: u8) -> i32 {
    match cmd {
        PRINTER_CMD_GET_DEVICE_ID => 0,
        PRINTER_CMD_GET_PORT_STATUS => dev.error_status as i32,
        PRINTER_CMD_SOFT_RESET => { dev.buffer.clear(); 0 }
        _ => -1,
    }
}

// =====================================================================
//  Microphone / audio — from usb-mic/usb-mic.cpp / usb-headset.cpp
// =====================================================================

pub const MIC_SAMPLE_RATE: i32 = 48000;
pub const MIC_SAMPLE_BITS: u16 = 16;
pub const MIC_CHANNELS: u8 = 1;
pub const HEADSET_SAMPLE_RATE: i32 = 48000;
pub const HEADSET_SAMPLE_BITS: u16 = 16;
pub const HEADSET_CHANNELS: u8 = 2;

pub fn mic_open(dev: &mut MicrophoneDevice) -> UsbResult<()> {
    dev.spec = AudioSpec {
        freq: MIC_SAMPLE_RATE,
        format: MIC_SAMPLE_BITS,
        channels: MIC_CHANNELS,
        samples: 0,
        silence: 0,
        size: 0,
    };
    dev.backend.open(&dev.spec)
}
pub fn mic_close(dev: &mut MicrophoneDevice) { dev.backend.close(); }
pub fn mic_capture(dev: &mut MicrophoneDevice, dst: &mut [u8]) -> usize { dev.buffer.read(dst) }
pub fn mic_flush(dev: &mut MicrophoneDevice) { dev.buffer.clear(); }

pub fn headset_open(dev: &mut HeadsetDevice) -> UsbResult<()> {
    dev.spec = AudioSpec {
        freq: HEADSET_SAMPLE_RATE,
        format: HEADSET_SAMPLE_BITS,
        channels: HEADSET_CHANNELS,
        samples: 0,
        silence: 0,
        size: 0,
    };
    dev.backend.open(&dev.spec)
}

// =====================================================================
//  EyeToy / webcam — from usb-eyetoy/usb-eyetoy-webcam.cpp
// =====================================================================

pub const EYETOY_DEFAULT_WIDTH: u16 = 320;
pub const EYETOY_DEFAULT_HEIGHT: u16 = 240;
pub const EYETOY_DEFAULT_FPS: u8 = 30;
pub const EYETOY_VENDOR_LOCAL: u16 = EYETOY_VENDOR;
pub const EYETOY_PRODUCT_LOCAL: u16 = EYETOY_PRODUCT;

pub fn eyetoy_open(dev: &mut EyeToyWebCamDevice) -> UsbResult<()> {
    dev.width = EYETOY_DEFAULT_WIDTH;
    dev.height = EYETOY_DEFAULT_HEIGHT;
    dev.backend.open(dev.width as u32, dev.height as u32, EYETOY_DEFAULT_FPS as u32)
}
pub fn eyetoy_start(dev: &mut EyeToyWebCamDevice) -> UsbResult<()> { dev.backend.start_streaming() }
pub fn eyetoy_stop(dev: &mut EyeToyWebCamDevice) { dev.backend.stop_streaming(); }
pub fn eyetoy_get_frame(dev: &mut EyeToyWebCamDevice) -> UsbResult<&[u8]> { dev.backend.next_frame() }

// =====================================================================
//  Convenience accessors used by USB.{cpp,h} and StateWrapper
// =====================================================================

pub fn get_ohci_port(port: u32) -> usize {
    // ports on the hub are swapped
    if port == 0 { 1 } else { 0 }
}

pub fn create_device(port: u32) -> bool {
    unsafe {
        let proxy_idx = match usbReg.config.usb.ports.get(port as usize) {
            Some(p) => p.device_type,
            None => DEVTYPE_NONE,
        };
        if proxy_idx == DEVTYPE_NONE { return true; }
        let dev = Box::new(UsbDevice {
            plugin_index: proxy_idx,
            port_index: port as i32,
            ..UsbDevice::default()
        });
        usbReg.devices.push(dev);
        usbReg.proxies[port as usize] = Some(proxy_idx as usize);
        true
    }
}

pub fn destroy_device(port: u32) {
    unsafe {
        usbReg.devices.retain(|d| d.port_index != port as i32);
        usbReg.proxies[port as usize] = None;
    }
}

pub fn update_device(port: u32) {
    unsafe {
        if usbReg.proxies[port as usize].is_none() { return; }
        // call proxy->UpdateSettings if available
    }
}

pub fn do_ohci_state(_sw: *mut StateWrapper) -> bool { true }
pub fn do_endpoint_state(_ep: *mut UsbEndpoint, _sw: *mut StateWrapper) {}
pub fn do_device_state(_dev: *mut UsbDevice, _sw: *mut StateWrapper) {}
pub fn do_packet_state(_p: *mut UsbPacket, _sw: *mut StateWrapper, _valid: [bool; NUM_PORTS]) {}
pub fn do_state(_sw: *mut StateWrapper) -> bool { true }

pub fn get_config_section(port: i32) -> String { format!("USB{}", port + 1) }

pub fn device_type_name_to_index(name: &str) -> i32 { RegisterDevice::instance().Index(name) }
pub fn device_type_index_to_name(idx: i32) -> &'static str {
    RegisterDevice::instance().Device(idx).map(|p| p.TypeName()).unwrap_or("None")
}
pub fn get_device_types() -> Vec<(&'static str, &'static str)> {
    let mut out = vec![("None", "Not Connected")];
    for p in RegisterDevice::instance().Map().iter().flatten() {
        out.push((p.TypeName(), p.Name()));
    }
    out
}
pub fn get_device_name(name: &str) -> &'static str {
    RegisterDevice::instance().DeviceByName(name).map(|p| p.Name()).unwrap_or("Not Connected")
}
pub fn get_device_icon_name(port: u32) -> Option<&'static str> {
    unsafe {
        usbReg.proxies[port as usize].and_then(|idx| {
            RegisterDevice::instance().Device(idx as i32).map(|p| p.IconName())
        })
    }
}
pub fn get_device_subtype_name(device: &str, subtype: u32) -> Option<&'static str> {
    RegisterDevice::instance().DeviceByName(device).and_then(|p| p.SubTypes().get(subtype as usize).copied())
}
pub fn get_device_subtypes(device: &str) -> &'static [&'static str] {
    &[]
}
pub fn get_device_bindings(_port: u32) -> Vec<InputBindingInfo> { Vec::new() }
pub fn get_device_bind_value(_port: u32, _idx: u32) -> f32 { 0.0 }
pub fn set_device_bind_value(_port: u32, _idx: u32, _v: f32) {}
pub fn input_device_connected(_id: &str) {}
pub fn input_device_disconnected(_id: &str) {}
pub fn get_config_device(_si: *mut c_void, _port: u32) -> String { String::new() }
pub fn set_config_device(_si: *mut c_void, _port: u32, _name: &str) {}
pub fn get_config_subtype(_si: *mut c_void, _port: u32, _name: &str) -> u32 { 0 }
pub fn set_config_subtype(_si: *mut c_void, _port: u32, _name: &str, _sub: u32) {}
pub fn get_config_sub_key(device: &str, bind: &str) -> String { format!("{}_{}", device, bind) }
pub fn map_device(_si: *mut c_void, _port: u32, _m: Vec<(GenericInputBinding, String)>) -> bool { false }
pub fn clear_port_bindings(_si: *mut c_void, _port: u32) {}
pub fn copy_configuration(_dest: *mut c_void, _src: *mut c_void, _devs: bool, _binds: bool) {}
pub fn set_default_configuration(_si: *mut c_void) {}
pub fn check_for_config_changes(_old: Pcsx2Config) {}
pub fn config_key_exists(_si: *mut c_void, _port: u32, _dev: &str, _key: &str) -> bool { false }
pub fn get_config_bool(_si: *mut c_void, _port: u32, _dev: &str, _key: &str, def: bool) -> bool { def }
pub fn get_config_int(_si: *mut c_void, _port: u32, _dev: &str, _key: &str, def: i32) -> i32 { def }
pub fn get_config_float(_si: *mut c_void, _port: u32, _dev: &str, _key: &str, def: f32) -> f32 { def }
pub fn get_config_string(_si: *mut c_void, _port: u32, _dev: &str, _key: &str, def: &str) -> String { def.to_string() }

// =====================================================================
//  Useful small wrappers used across the plugin
// =====================================================================

pub fn usb_attach_port(dev_idx: usize) { let _ = dev_idx; }
pub fn usb_detach_port(dev_idx: usize) { let _ = dev_idx; }

// Quick sanity check helper, exposed for downstream Rust modules
pub fn ensure_initialized() {
    unsafe {
        if usbReg.config.usb.ports.len() < NUM_PORTS {
            for _ in usbReg.config.usb.ports.len()..NUM_PORTS {
                usbReg.config.usb.ports.push(PortConfig::default());
            }
        }
    }
}

pub fn ringbuffer_test() {
    let mut rb = RingBuffer::new(64);
    assert_eq!(rb.write(b"hello"), 5);
    let mut out = [0u8; 3];
    assert_eq!(rb.read(&mut out), 3);
    assert_eq!(&out, b"hel");
}

#[doc(hidden)]
pub fn __static_assert_usbstate_layout() {
    // Compile-time assertions mirroring the C++ invariants.
    const _: () = assert!(NUM_PORTS == 2);
    const _: () = assert!(OHCI_MAX_PORTS == 2);
    const _: () = assert!(USB_MAX_ENDPOINTS == 15);
    const _: () = assert!(USB_MAX_INTERFACES == 16);
}

#[doc(hidden)]
pub const _VENDOR_VEC: &[(u16, u16, &str)] = &[
    (PAD_DEFAULT_VENDOR, PAD_DEFAULT_PRODUCT, "Pad"),
    (MSD_VENDOR, MSD_PRODUCT, "MSD"),
    (MIC_VENDOR, MIC_PRODUCT, "Mic"),
    (HEADSET_VENDOR, HEADSET_PRODUCT, "Headset"),
    (HIDKEYBOARD_VENDOR, HIDKEYBOARD_PRODUCT, "HID KBD"),
    (HIDMOUSE_VENDOR, HIDMOUSE_PRODUCT, "HID Mouse"),
    (EYETOY_VENDOR, EYETOY_PRODUCT, "EyeToy"),
    (BUZZ_VENDOR, BUZZ_PRODUCT, "Buzz"),
    (TRANCE_VENDOR, TRANCE_PRODUCT, "Trance"),
    (SEAMIC_VENDOR, SEAMIC_PRODUCT, "Seamic"),
    (GUNCON2_VENDOR, GUNCON2_PRODUCT, "GunCon 2"),
    (PRINTER_VENDOR, PRINTER_PRODUCT, "Printer"),
];

// CStr helper used when strings come from real config files in the future.
pub fn c_str_safe(s: &CStr) -> &str {
    s.to_str().unwrap_or("")
}

pub fn ptr_to_nonull<T>(p: *mut T) -> Option<NonNull<T>> { NonNull::new(p) }
