//! USB stack (qemu-usb fork) translated from PCSX2's C++ into idiomatic Rust 2021.
//!
//! This module is a single-file translation of the qemu-USB host controller
//! subsystem that PCSX2 uses to back the PS2's two OHCI USB ports.  It exposes
//! the public surface required by the surrounding emulator core:
//!
//! * [`USBReg`] and the global [`usbReg`] instance, plus
//!   [`usbInit`], [`usbReset`], [`usbShutdown`].
//! * Bus-level memory hooks [`usbRead8`] / [`usbRead16`] / [`usbRead32`]
//!   and [`usbWrite8`] / [`usbWrite16`] / [`usbWrite32`].
//! * [`UsbBus`] with `attach` / `detach` / `find_device` operations.
//! * [`UsbDevice`] with `setup_packet` / `handle_data` / `realize`.
//! * [`OHCIState`] — the OpenHCI register file and the frame / TD machinery
//!   that walks it.
//! * [`HIDState`] and the HID descriptor and event types, plus the
//!   `QKeyCode` -> scancode mapping inherited from the qemu input keymap.
//!
//! All globals that the C source declares as file-scope statics are modelled
//! with `static mut`, matching the lifetime of the original.  Only `std` is
//! used; no external crates are required.

// =====================================================================
// 1.  Standard USB constants
// =====================================================================

pub const USB_TOKEN_SETUP: u8 = 0x2d;
pub const USB_TOKEN_IN: u8 = 0x69;
pub const USB_TOKEN_OUT: u8 = 0xe1;

pub const USB_MSG_ATTACH: u32 = 0x100;
pub const USB_MSG_DETACH: u32 = 0x101;
pub const USB_MSG_RESET: u32 = 0x102;

pub const USB_RET_SUCCESS: i32 = 0;
pub const USB_RET_NODEV: i32 = -1;
pub const USB_RET_NAK: i32 = -2;
pub const USB_RET_STALL: i32 = -3;
pub const USB_RET_BABBLE: i32 = -4;
pub const USB_RET_IOERROR: i32 = -5;
pub const USB_RET_ASYNC: i32 = -6;
pub const USB_RET_ADD_TO_QUEUE: i32 = -7;
pub const USB_RET_REMOVE_FROM_QUEUE: i32 = -8;

pub const USB_SPEED_LOW: u8 = 0;
pub const USB_SPEED_FULL: u8 = 1;
pub const USB_SPEED_HIGH: u8 = 2;
pub const USB_SPEED_MASK_LOW: u8 = 1u8 << USB_SPEED_LOW;
pub const USB_SPEED_MASK_FULL: u8 = 1u8 << USB_SPEED_FULL;

pub const USB_STATE_NOTATTACHED: i32 = 0;
pub const USB_STATE_ATTACHED: i32 = 1;
pub const USB_STATE_DEFAULT: i32 = 3;
pub const USB_STATE_SUSPENDED: i32 = 6;

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

pub const DEVICE_REQUEST: u32 = ((USB_DIR_IN | USB_TYPE_STANDARD | USB_RECIP_DEVICE) as u32) << 8;
pub const DEVICE_OUT_REQUEST: u32 = ((USB_DIR_OUT | USB_TYPE_STANDARD | USB_RECIP_DEVICE) as u32) << 8;
pub const VENDOR_DEVICE_REQUEST: u32 = ((USB_DIR_IN | USB_TYPE_VENDOR | USB_RECIP_DEVICE) as u32) << 8;
pub const VENDOR_DEVICE_OUT_REQUEST: u32 = ((USB_DIR_OUT | USB_TYPE_VENDOR | USB_RECIP_DEVICE) as u32) << 8;
pub const INTERFACE_REQUEST: u32 = ((USB_DIR_IN | USB_TYPE_STANDARD | USB_RECIP_INTERFACE) as u32) << 8;
pub const INTERFACE_OUT_REQUEST: u32 = ((USB_DIR_OUT | USB_TYPE_STANDARD | USB_RECIP_INTERFACE) as u32) << 8;
pub const ENDPOINT_REQUEST: u32 = ((USB_DIR_IN | USB_TYPE_STANDARD | USB_RECIP_ENDPOINT) as u32) << 8;
pub const ENDPOINT_OUT_REQUEST: u32 = ((USB_DIR_OUT | USB_TYPE_STANDARD | USB_RECIP_ENDPOINT) as u32) << 8;
pub const CLASS_INTERFACE_REQUEST: u32 = ((USB_DIR_IN | USB_TYPE_CLASS | USB_RECIP_INTERFACE) as u32) << 8;
pub const CLASS_INTERFACE_OUT_REQUEST: u32 = ((USB_DIR_OUT | USB_TYPE_CLASS | USB_RECIP_INTERFACE) as u32) << 8;
pub const CLASS_ENDPOINT_REQUEST: u32 = ((USB_DIR_IN | USB_TYPE_CLASS | USB_RECIP_ENDPOINT) as u32) << 8;
pub const CLASS_ENDPOINT_OUT_REQUEST: u32 = ((USB_DIR_OUT | USB_TYPE_CLASS | USB_RECIP_ENDPOINT) as u32) << 8;
pub const VENDOR_INTERFACE_REQUEST: u32 = ((USB_DIR_IN | USB_TYPE_VENDOR | USB_RECIP_INTERFACE) as u32) << 8;
pub const VENDOR_INTERFACE_OUT_REQUEST: u32 = ((USB_DIR_OUT | USB_TYPE_VENDOR | USB_RECIP_INTERFACE) as u32) << 8;

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

pub const USB_DEVICE_SELF_POWERED: u8 = 0;
pub const USB_DEVICE_REMOTE_WAKEUP: u8 = 1;

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

pub const USB_DEV_CAP_WIRELESS: u8 = 0x01;
pub const USB_DEV_CAP_USB2_EXT: u8 = 0x02;
pub const USB_DEV_CAP_SUPERSPEED: u8 = 0x03;

pub const USB_CFG_ATT_ONE: u8 = 1 << 7;
pub const USB_CFG_ATT_SELFPOWER: u8 = 1 << 6;
pub const USB_CFG_ATT_WAKEUP: u8 = 1 << 5;
pub const USB_CFG_ATT_BATTERY: u8 = 1 << 4;

pub const USB_ENDPOINT_XFER_CONTROL: u8 = 0;
pub const USB_ENDPOINT_XFER_ISOC: u8 = 1;
pub const USB_ENDPOINT_XFER_BULK: u8 = 2;
pub const USB_ENDPOINT_XFER_INT: u8 = 3;
pub const USB_ENDPOINT_XFER_INVALID: u8 = 255;

pub const USB_INTERFACE_INVALID: u8 = 255;
pub const USB_INTERFACE_INVALID_I32: i32 = -1;

pub const GET_REPORT: u32 = 0xa101;
pub const GET_IDLE: u32 = 0xa102;
pub const GET_PROTOCOL: u32 = 0xa103;
pub const SET_REPORT: u32 = 0x2109;
pub const SET_IDLE: u32 = 0x210a;
pub const SET_PROTOCOL: u32 = 0x210b;

pub const USB_DEVICE_DESC_SIZE: usize = 18;
pub const USB_CONFIGURATION_DESC_SIZE: usize = 9;
pub const USB_INTERFACE_DESC_SIZE: usize = 9;
pub const USB_ENDPOINT_DESC_SIZE: usize = 7;
pub const USB_DESC_FLAG_SUPER: i32 = 0x01;

// =====================================================================
// 2.  Size limits and queue macros
// =====================================================================

pub const USB_MAX_ENDPOINTS: usize = 15;
pub const USB_MAX_INTERFACES: usize = 16;
pub const OHCI_MAX_PORTS: usize = 15;

/// PSX bus clock (36.864 MHz) — used by the OHCI tick conversion helpers.
pub const PSXCLK: i32 = 36_864_000;
pub const USB_HZ: u32 = 12_000_000;

// =====================================================================
// 3.  HID constants
// =====================================================================

pub const HID_USAGE_ERROR_ROLLOVER: u8 = 0x01;
pub const HID_USAGE_POSTFAIL: u8 = 0x02;
pub const HID_USAGE_ERROR_UNDEFINED: u8 = 0x03;

pub const HID_KEYBOARD: i32 = 0;
pub const HID_MOUSE: i32 = 1;
pub const HID_TABLET: i32 = 2;

pub const QUEUE_LENGTH: usize = 16;
pub const QUEUE_MASK: usize = QUEUE_LENGTH - 1;

pub const INPUT_EVENT_KIND_REL: i32 = 0;
pub const INPUT_EVENT_KIND_ABS: i32 = 1;
pub const INPUT_EVENT_KIND_BTN: i32 = 2;
pub const INPUT_EVENT_KIND_KEY: i32 = 3;

pub const INPUT_AXIS_X: i32 = 0;
pub const INPUT_AXIS_Y: i32 = 1;

pub const INPUT_BUTTON_LEFT: i32 = 0;
pub const INPUT_BUTTON_MIDDLE: i32 = 1;
pub const INPUT_BUTTON_RIGHT: i32 = 2;
pub const INPUT_BUTTON_WHEEL_UP: i32 = 3;
pub const INPUT_BUTTON_WHEEL_DOWN: i32 = 4;
pub const INPUT_BUTTON__MAX: usize = 6;

pub const KEY_VALUE_KIND_QCODE: i32 = 0;
pub const KEY_VALUE_KIND_NUMBER: i32 = 1;

pub const SCANCODE_GREY: i32 = 0x80;
pub const SCANCODE_UP: i32 = 0x80;
pub const SCANCODE_EMUL0: i32 = 0xe0;

// =====================================================================
// 4.  OHCI register bits
// =====================================================================

pub const OHCI_CTL_CBSR: u32 = (1 << 0) | (1 << 1);
pub const OHCI_CTL_PLE: u32 = 1 << 2;
pub const OHCI_CTL_IE: u32 = 1 << 3;
pub const OHCI_CTL_CLE: u32 = 1 << 4;
pub const OHCI_CTL_BLE: u32 = 1 << 5;
pub const OHCI_CTL_HCFS: u32 = (1 << 6) | (1 << 7);
pub const OHCI_USB_RESET: u32 = 0x00;
pub const OHCI_USB_RESUME: u32 = 0x40;
pub const OHCI_USB_OPERATIONAL: u32 = 0x80;
pub const OHCI_USB_SUSPEND: u32 = 0xc0;
pub const OHCI_CTL_IR: u32 = 1 << 8;
pub const OHCI_CTL_RWC: u32 = 1 << 9;
pub const OHCI_CTL_RWE: u32 = 1 << 10;

pub const OHCI_STATUS_HCR: u32 = 1 << 0;
pub const OHCI_STATUS_CLF: u32 = 1 << 1;
pub const OHCI_STATUS_BLF: u32 = 1 << 2;
pub const OHCI_STATUS_OCR: u32 = 1 << 3;
pub const OHCI_STATUS_SOC: u32 = (1 << 6) | (1 << 7);

pub const OHCI_INTR_SO: u32 = 1 << 0;
pub const OHCI_INTR_WD: u32 = 1 << 1;
pub const OHCI_INTR_SF: u32 = 1 << 2;
pub const OHCI_INTR_RD: u32 = 1 << 3;
pub const OHCI_INTR_UE: u32 = 1 << 4;
pub const OHCI_INTR_FNO: u32 = 1 << 5;
pub const OHCI_INTR_RHSC: u32 = 1 << 6;
pub const OHCI_INTR_OC: u32 = 1 << 30;
pub const OHCI_INTR_MIE: u32 = 1 << 31;

pub const OHCI_HCCA_SIZE: u32 = 0x100;
pub const OHCI_HCCA_MASK: u32 = 0xffffff00;
pub const OHCI_EDPTR_MASK: u32 = 0xfffffff0;
pub const OHCI_FMI_FI: u32 = 0x0000_3fff;
pub const OHCI_FMI_FSMPS: u32 = 0xffff_0000;
pub const OHCI_FMI_FIT: u32 = 0x8000_0000;
pub const OHCI_FR_RT: u32 = 1 << 31;
pub const OHCI_LS_THRESH: u32 = 0x628;

pub const OHCI_RHA_RW_MASK: u32 = 0x0000_0000;
pub const OHCI_RHA_PSM: u32 = 1 << 8;
pub const OHCI_RHA_NPS: u32 = 1 << 9;
pub const OHCI_RHA_DT: u32 = 1 << 10;
pub const OHCI_RHA_OCPM: u32 = 1 << 11;
pub const OHCI_RHA_NOCP: u32 = 1 << 12;
pub const OHCI_RHA_POTPGT_MASK: u32 = 0xff00_0000;

pub const OHCI_RHS_LPS: u32 = 1 << 0;
pub const OHCI_RHS_OCI: u32 = 1 << 1;
pub const OHCI_RHS_DRWE: u32 = 1 << 15;
pub const OHCI_RHS_LPSC: u32 = 1 << 16;
pub const OHCI_RHS_OCIC: u32 = 1 << 17;
pub const OHCI_RHS_CRWE: u32 = 1 << 31;

pub const OHCI_PORT_CCS: u32 = 1 << 0;
pub const OHCI_PORT_PES: u32 = 1 << 1;
pub const OHCI_PORT_PSS: u32 = 1 << 2;
pub const OHCI_PORT_POCI: u32 = 1 << 3;
pub const OHCI_PORT_PRS: u32 = 1 << 4;
pub const OHCI_PORT_PPS: u32 = 1 << 8;
pub const OHCI_PORT_LSDA: u32 = 1 << 9;
pub const OHCI_PORT_CSC: u32 = 1 << 16;
pub const OHCI_PORT_PESC: u32 = 1 << 17;
pub const OHCI_PORT_PSSC: u32 = 1 << 18;
pub const OHCI_PORT_OCIC: u32 = 1 << 19;
pub const OHCI_PORT_PRSC: u32 = 1 << 20;
pub const OHCI_PORT_WTC: u32 = OHCI_PORT_CSC | OHCI_PORT_PESC | OHCI_PORT_PSSC | OHCI_PORT_OCIC | OHCI_PORT_PRSC;

pub const OHCI_TD_DIR_SETUP: u32 = 0x0;
pub const OHCI_TD_DIR_OUT: u32 = 0x1;
pub const OHCI_TD_DIR_IN: u32 = 0x2;
pub const OHCI_TD_DIR_RESERVED: u32 = 0x3;

pub const OHCI_CC_NOERROR: u32 = 0x0;
pub const OHCI_CC_CRC: u32 = 0x1;
pub const OHCI_CC_BITSTUFFING: u32 = 0x2;
pub const OHCI_CC_DATATOGGLEMISMATCH: u32 = 0x3;
pub const OHCI_CC_STALL: u32 = 0x4;
pub const OHCI_CC_DEVICENOTRESPONDING: u32 = 0x5;
pub const OHCI_CC_PIDCHECKFAILURE: u32 = 0x6;
pub const OHCI_CC_UNDEXPETEDPID: u32 = 0x7;
pub const OHCI_CC_DATAOVERRUN: u32 = 0x8;
pub const OHCI_CC_DATAUNDERRUN: u32 = 0x9;
pub const OHCI_CC_BUFFEROVERRUN: u32 = 0xc;
pub const OHCI_CC_BUFFERUNDERRUN: u32 = 0xd;

// ED/TD bit fields
pub const OHCI_ED_FA_SHIFT: u32 = 0;
pub const OHCI_ED_FA_MASK: u32 = 0x7f << OHCI_ED_FA_SHIFT;
pub const OHCI_ED_EN_SHIFT: u32 = 7;
pub const OHCI_ED_EN_MASK: u32 = 0xf << OHCI_ED_EN_SHIFT;
pub const OHCI_ED_D_SHIFT: u32 = 11;
pub const OHCI_ED_D_MASK: u32 = 3 << OHCI_ED_D_SHIFT;
pub const OHCI_ED_S: u32 = 1 << 13;
pub const OHCI_ED_K: u32 = 1 << 14;
pub const OHCI_ED_F: u32 = 1 << 15;
pub const OHCI_ED_MPS_SHIFT: u32 = 16;
pub const OHCI_ED_MPS_MASK: u32 = 0x7ff << OHCI_ED_MPS_SHIFT;
pub const OHCI_ED_H: u32 = 1;
pub const OHCI_ED_C: u32 = 2;
pub const OHCI_TD_R: u32 = 1 << 18;
pub const OHCI_TD_DP_SHIFT: u32 = 19;
pub const OHCI_TD_DP_MASK: u32 = 3 << OHCI_TD_DP_SHIFT;
pub const OHCI_TD_DI_SHIFT: u32 = 21;
pub const OHCI_TD_DI_MASK: u32 = 7 << OHCI_TD_DI_SHIFT;
pub const OHCI_TD_T0: u32 = 1 << 24;
pub const OHCI_TD_T1: u32 = 1 << 24;
pub const OHCI_TD_EC_SHIFT: u32 = 26;
pub const OHCI_TD_EC_MASK: u32 = 3 << OHCI_TD_EC_SHIFT;
pub const OHCI_TD_CC_SHIFT: u32 = 28;
pub const OHCI_TD_CC_MASK: u32 = 0xf << OHCI_TD_CC_SHIFT;
pub const OHCI_TD_SF_SHIFT: u32 = 0;
pub const OHCI_TD_SF_MASK: u32 = 0xffff << OHCI_TD_SF_SHIFT;
pub const OHCI_TD_FC_SHIFT: u32 = 24;
pub const OHCI_TD_FC_MASK: u32 = 7 << OHCI_TD_FC_SHIFT;
pub const OHCI_TD_PSW_CC_SHIFT: u32 = 12;
pub const OHCI_TD_PSW_CC_MASK: u32 = 0xf << OHCI_TD_PSW_CC_SHIFT;
pub const OHCI_TD_PSW_SIZE_SHIFT: u32 = 0;
pub const OHCI_TD_PSW_SIZE_MASK: u32 = 0xfff << OHCI_TD_PSW_SIZE_SHIFT;

pub const OHCI_PAGE_MASK: u32 = 0xffff_f000;
pub const OHCI_OFFSET_MASK: u32 = 0x0000_0fff;
pub const OHCI_DPTR_MASK: u32 = 0xfffffff0;
pub const ED_WBACK_OFFSET: usize = 8;
pub const ED_WBACK_SIZE: usize = 4;
pub const HCCA_WRITEBACK_OFFSET: usize = 128;
pub const HCCA_WRITEBACK_SIZE: usize = 8;
pub const ED_LINK_LIMIT: usize = 32;
pub const DMA_DIRECTION_TO_DEVICE: i32 = 0;
pub const DMA_DIRECTION_FROM_DEVICE: i32 = 1;
pub const MIN_IRQ_INTERVAL: i64 = 64;

// =====================================================================
// 5.  Setup state machine
// =====================================================================

pub const SETUP_STATE_IDLE: i32 = 0;
pub const SETUP_STATE_SETUP: i32 = 1;
pub const SETUP_STATE_DATA: i32 = 2;
pub const SETUP_STATE_ACK: i32 = 3;
pub const SETUP_STATE_PARAM: i32 = 4;

#[inline]
pub fn usb_lo(v: u16) -> u8 { (v & 0xFF) as u8 }

#[inline]
pub fn usb_hi(v: u16) -> u8 { ((v >> 8) & 0xFF) as u8 }

// =====================================================================
// 6.  Packet / device / bus / port types
// =====================================================================

/// Bit values for [`UsbDeviceFlags`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum USBDeviceFlag {
    FullPath,
    IsHost,
    MsosDescEnable,
    MsosDescInUse,
}

/// State of an in-flight packet — mirrors QEMU's `USBPacketState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum UsbPacketState {
    Undefined = 0,
    Setup,
    Queued,
    Async,
    Complete,
    Canceled,
}

/// Active USB packet.  All fields mirror the C struct exactly; the `id`
/// is used to disambiguate packets that share an endpoint when cancelling.
#[derive(Debug, Clone)]
pub struct UsbPacket {
    pub pid: i32,
    pub id: u64,
    pub ep: Option<usize>,
    pub stream: u32,
    pub buffer_size: u32,
    pub buffer_ptr: Option<Vec<u8>>,
    pub parameter: u64,
    pub short_not_ok: bool,
    pub int_req: bool,
    pub status: i32,
    pub actual_length: i32,
    pub state: UsbPacketState,
    /// Doubly-linked queue of packets on a single endpoint — implemented
    /// with indices into the device's packet arena.
    pub queue_next: Option<usize>,
    pub queue_prev: Option<usize>,
    pub combined_next: Option<usize>,
    pub combined_prev: Option<usize>,
}

impl Default for UsbPacket {
    fn default() -> Self {
        Self {
            pid: 0,
            id: 0,
            ep: None,
            stream: 0,
            buffer_size: 0,
            buffer_ptr: None,
            parameter: 0,
            short_not_ok: false,
            int_req: false,
            status: USB_RET_SUCCESS,
            actual_length: 0,
            state: UsbPacketState::Undefined,
            queue_next: None,
            queue_prev: None,
            combined_next: None,
            combined_prev: None,
        }
    }
}

/// Endpoint descriptor — one per control endpoint + per direction data EP.
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
    pub dev: usize,
    /// Head of the per-endpoint packet queue (dllist).
    pub queue_head: Option<usize>,
    pub queue_tail: Option<usize>,
}

impl Default for UsbEndpoint {
    fn default() -> Self {
        Self {
            nr: 0,
            pid: 0,
            r#type: USB_ENDPOINT_XFER_INVALID,
            ifnum: USB_INTERFACE_INVALID,
            max_packet_size: 0,
            max_streams: 0,
            pipeline: false,
            halted: false,
            dev: 0,
            queue_head: None,
            queue_tail: None,
        }
    }
}

/// Virtual method table for a device class.
#[derive(Debug, Clone, Default)]
pub struct UsbDeviceClass {
    pub realize: Option<fn(&mut UsbDevice)>,
    pub unrealize: Option<fn(&mut UsbDevice)>,
    pub find_device: Option<fn(&UsbDevice, u8) -> Option<usize>>,
    pub cancel_packet: Option<fn(&UsbDevice, &mut UsbPacket)>,
    pub handle_attach: Option<fn(&UsbDevice)>,
    pub handle_reset: Option<fn(&UsbDevice)>,
    pub handle_control: Option<fn(&UsbDevice, &mut UsbPacket, i32, i32, i32, i32, &mut [u8])>,
    pub handle_data: Option<fn(&UsbDevice, &mut UsbPacket)>,
    pub set_interface: Option<fn(&UsbDevice, i32, i32, i32)>,
    pub flush_ep_queue: Option<fn(&UsbDevice, &UsbEndpoint)>,
    pub ep_stopped: Option<fn(&UsbDevice, &UsbEndpoint)>,
    pub alloc_streams: Option<fn(&UsbDevice, &mut [UsbEndpoint], i32) -> i32>,
    pub free_streams: Option<fn(&UsbDevice, &mut [UsbEndpoint])>,
    pub product_desc: Option<String>,
    pub usb_desc: Option<usize>,
    pub attached_settable: bool,
}

/// USB device — the full QEMU device object with class vtable, ports,
/// endpoints, and the per-device state required to do enumeration.
#[derive(Debug, Clone)]
pub struct UsbDevice {
    pub klass: UsbDeviceClass,
    pub port: Option<usize>,
    pub bus: Option<usize>,
    pub opaque: Option<usize>,
    pub flags: u32,
    pub speed: i32,
    pub speedmask: i32,
    pub addr: u8,
    pub product_desc: String,
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

    pub usb_desc: Option<usize>,
    pub device: Option<usize>,

    pub configuration: i32,
    pub ninterfaces: i32,
    pub altsetting: [i32; USB_MAX_INTERFACES],
    pub config: Option<usize>,
    pub ifaces: [Option<usize>; USB_MAX_INTERFACES],
}

impl Default for UsbDevice {
    fn default() -> Self {
        Self {
            klass: UsbDeviceClass::default(),
            port: None,
            bus: None,
            opaque: None,
            flags: 0,
            speed: 0,
            speedmask: 0,
            addr: 0,
            product_desc: String::new(),
            auto_attach: 0,
            attached: false,

            state: USB_STATE_NOTATTACHED,
            setup_buf: [0; 8],
            data_buf: [0; 4096],
            remote_wakeup: 0,
            setup_state: SETUP_STATE_IDLE,
            setup_len: 0,
            setup_index: 0,

            ep_ctl: UsbEndpoint { nr: 0, pid: 0, r#type: USB_ENDPOINT_XFER_CONTROL, ..Default::default() },
            ep_in: std::array::from_fn(|_| UsbEndpoint::default()),
            ep_out: std::array::from_fn(|_| UsbEndpoint::default()),

            usb_desc: None,
            device: None,

            configuration: 0,
            ninterfaces: 0,
            altsetting: [0; USB_MAX_INTERFACES],
            config: None,
            ifaces: [None; USB_MAX_INTERFACES],
        }
    }
}

impl UsbDevice {
    /// "Realize" a freshly created device: install endpoints and reset
    /// state.  This is the C `realize` callback surface.
    pub fn realize(&mut self) {
        usb_ep_init(self);
        if let Some(cb) = self.klass.realize {
            cb(self);
        }
    }

    /// Process a SETUP packet through the descriptor state machine.
    pub fn setup_packet(&mut self, p: &mut UsbPacket) {
        if p.buffer_size != 8 {
            p.status = USB_RET_STALL;
            return;
        }
        if (p.buffer_size as usize) > self.setup_buf.len() {
            p.status = USB_RET_STALL;
            return;
        }
        self.setup_buf.copy_from_slice(&p.buffer_ptr.as_deref().unwrap_or(&[])[..8]);
        self.setup_index = 0;
        p.actual_length = 0;
        self.setup_len = ((self.setup_buf[7] as i32) << 8) | (self.setup_buf[6] as i32);
        if self.setup_len > self.data_buf.len() as i32 {
            p.status = USB_RET_STALL;
            return;
        }

        let request = ((self.setup_buf[0] as i32) << 8) | (self.setup_buf[1] as i32);
        let value = ((self.setup_buf[3] as i32) << 8) | (self.setup_buf[2] as i32);
        let index = ((self.setup_buf[5] as i32) << 8) | (self.setup_buf[4] as i32);

        if self.setup_buf[0] & USB_DIR_IN != 0 {
            if let Some(cb) = self.klass.handle_control {
                let mut data = [0u8; 4096];
                cb(self, p, request, value, index, self.setup_len, &mut data);
                if p.status == USB_RET_ASYNC {
                    self.setup_state = SETUP_STATE_SETUP;
                    return;
                }
                if p.status != USB_RET_SUCCESS {
                    return;
                }
                if p.actual_length < self.setup_len {
                    self.setup_len = p.actual_length;
                }
                self.data_buf[..self.setup_len as usize].copy_from_slice(&data[..self.setup_len as usize]);
            }
            self.setup_state = SETUP_STATE_DATA;
        } else {
            self.setup_state = if self.setup_len == 0 { SETUP_STATE_ACK } else { SETUP_STATE_DATA };
        }
        p.actual_length = 8;
    }

    /// Process a BULK / INTERRUPT / ISO data packet.
    pub fn handle_data(&mut self, p: &mut UsbPacket) {
        if let Some(cb) = self.klass.handle_data {
            cb(self, p);
        }
    }
}

/// USB bus — a QEMU `USBBus` plus the `attach` / `detach` / `find_device`
/// operations the caller is required to expose.
#[derive(Debug, Clone)]
pub struct UsbBus {
    pub ops: Option<usize>,
    pub busnr: i32,
    pub nfree: i32,
    pub nused: i32,
    pub free: Vec<usize>,
    pub used: Vec<usize>,
    /// Doubly-linked list node (used by the qemu bus registry).
    pub next: Option<usize>,
    pub prev: Option<usize>,
}

impl UsbBus {
    /// Attach a device to the bus.  Calls back into the port's
    /// `attach` op so the host controller can update its port status
    /// register.
    pub fn attach(&mut self, port_idx: usize, dev_idx: usize, ports: &mut [UsbPort], dev: &mut UsbDevice) {
        if let Some(p) = ports.get_mut(port_idx) {
            p.dev = Some(dev_idx);
            dev.attached = true;
        }
        usb_attach(ports, port_idx, dev);
    }

    /// Detach a device from a port.  Returns the previous device index
    /// (or `None`).
    pub fn detach(&mut self, port_idx: usize, ports: &mut [UsbPort]) -> Option<usize> {
        let prev = ports.get_mut(port_idx).and_then(|p| p.dev);
        if let Some(p) = ports.get_mut(port_idx) {
            p.dev = None;
        }
        if let Some(di) = prev {
            usb_detach(ports, port_idx);
            Some(di)
        } else {
            None
        }
    }

    /// Find a device on the bus matching `addr`.
    pub fn find_device(&self, ports: &[UsbPort], devs: &[UsbDevice], addr: u8) -> Option<usize> {
        for (i, p) in ports.iter().enumerate() {
            if p.dev.is_none() { continue; }
            if let Some(idx) = p.dev {
                if let Some(dev) = devs.get(idx) {
                    if dev.addr == addr && dev.attached && dev.state == USB_STATE_DEFAULT {
                        return Some(idx);
                    }
                    if let Some(klass_find) = dev.klass.find_device {
                        if let Some(found) = klass_find(dev, addr) {
                            return Some(found);
                        }
                    }
                }
            }
        }
        None
    }
}

/// Port operations — the callbacks the host controller installs.
#[derive(Debug, Default, Clone, Copy)]
pub struct UsbPortOps {
    pub attach: Option<fn(&mut UsbPort, &mut UsbDevice)>,
    pub detach: Option<fn(&mut UsbPort)>,
    pub wakeup: Option<fn(&mut UsbPort)>,
    pub complete: Option<fn(&mut UsbPort, &mut UsbPacket)>,
}

/// A single USB port on a hub / root hub.
#[derive(Debug, Clone)]
pub struct UsbPort {
    pub dev: Option<usize>,
    pub speedmask: i32,
    pub ops: Option<usize>,
    pub opaque: Option<usize>,
    pub index: i32,
    pub next: Option<usize>,
    pub prev: Option<usize>,
}

// =====================================================================
// 7.  Descriptor types
// =====================================================================

#[derive(Debug, Default, Clone)]
pub struct UsbDescId {
    pub id_vendor: u16,
    pub id_product: u16,
    pub bcd_device: u16,
    pub i_manufacturer: u8,
    pub i_product: u8,
    pub i_serial_number: u8,
}

#[derive(Debug, Default, Clone)]
pub struct UsbDescEndpoint {
    pub b_endpoint_address: u8,
    pub bm_attributes: u8,
    pub w_max_packet_size: u16,
    pub b_interval: u8,
    pub is_audio: bool,
    pub b_refresh: u8,
    pub b_synch_address: u8,
    pub extra: Option<Vec<u8>>,
    pub b_max_burst: u8,
    pub bm_attributes_super: u8,
    pub w_bytes_per_interval: u16,
}

#[derive(Debug, Default, Clone)]
pub struct UsbDescOther {
    pub length: u8,
    pub data: Vec<u8>,
}

#[derive(Debug, Default, Clone)]
pub struct UsbDescIface {
    pub b_interface_number: u8,
    pub b_alternate_setting: u8,
    pub b_num_endpoints: u8,
    pub b_interface_class: u8,
    pub b_interface_sub_class: u8,
    pub b_interface_protocol: u8,
    pub i_interface: u8,
    pub descs: Vec<UsbDescOther>,
    pub eps: Vec<UsbDescEndpoint>,
}

#[derive(Debug, Default, Clone)]
pub struct UsbDescIfaceAssoc {
    pub b_first_interface: u8,
    pub b_interface_count: u8,
    pub b_function_class: u8,
    pub b_function_sub_class: u8,
    pub b_function_protocol: u8,
    pub i_function: u8,
    pub ifs: Vec<UsbDescIface>,
}

#[derive(Debug, Default, Clone)]
pub struct UsbDescConfig {
    pub b_num_interfaces: u8,
    pub b_configuration_value: u8,
    pub i_configuration: u8,
    pub bm_attributes: u8,
    pub b_max_power: u8,
    pub if_groups: Vec<UsbDescIfaceAssoc>,
    pub ifs: Vec<UsbDescIface>,
}

#[derive(Debug, Default, Clone)]
pub struct UsbDescDevice {
    pub bcd_usb: u16,
    pub b_device_class: u8,
    pub b_device_sub_class: u8,
    pub b_device_protocol: u8,
    pub b_max_packet_size0: u8,
    pub b_num_configurations: u8,
    pub confs: Vec<UsbDescConfig>,
}

#[derive(Debug, Default, Clone)]
pub struct UsbDescString {
    pub index: u8,
    pub str: String,
}

#[derive(Debug, Default, Clone)]
pub struct UsbDesc {
    pub id: UsbDescId,
    pub full: Option<UsbDescDevice>,
    pub high: Option<UsbDescDevice>,
    pub super_: Option<UsbDescDevice>,
    pub strs: Vec<UsbDescString>,
}

// =====================================================================
// 8.  HID types
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyValueKind {
    Qcode,
    Number,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QKeyCode {
    _0, _1, _2, _3, _4, _5, _6, _7, _8, _9,
    A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
    AcBack, AcBookmarks, AcForward, AcHome, AcRefresh,
    Again, Alt, AltR, Apostrophe, Asterisk,
    AudioMute, AudioNext, AudioPlay, AudioPrev, AudioStop,
    Backslash, Backspace, BracketLeft, BracketRight,
    Calculator, CapsLock, Comma, Compose, Computer,
    Copy, Ctrl, CtrlR, Cut,
    Delete, Dot, Down,
    End, Equal, Esc,
    F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12,
    Find, Front,
    GraveAccent,
    Help, Henkan, Hiragana, Home,
    Insert,
    KatakanaHiragana,
    Kp0, Kp1, Kp2, Kp3, Kp4, Kp5, Kp6, Kp7, Kp8, Kp9,
    KpAdd, KpComma, KpDecimal, KpDivide, KpEnter, KpEquals,
    KpMultiply, KpSubtract,
    Left, Less, Lf,
    Mail, MediaSelect, Menu, MetaL, MetaR,
    Minus, Muhenkan,
    NumLock,
    Open,
    Paste, Pause, PgDn, PgUp, Power, Print, Props,
    Ret, Right, Ro,
    ScrollLock, Semicolon,
    Shift, ShiftR, Slash, Sleep, Spc, Stop, Sysrq,
    Tab,
    Undo, Up,
    VolumeDown, VolumeUp,
    Wake,
    Yen,
}

#[derive(Debug, Clone, Copy)]
pub struct KeyValue {
    pub kind: KeyValueKind,
    pub qcode: QKeyCode,
    pub number: i32,
}

#[derive(Debug, Clone, Copy)]
pub struct InputMoveEvent {
    pub axis: i32,
    pub value: i32,
}

#[derive(Debug, Clone, Copy)]
pub struct InputBtnEvent {
    pub button: i32,
    pub down: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct InputKeyEvent {
    pub key: KeyValue,
    pub down: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct InputEvent {
    pub kind: i32,
    pub rel: InputMoveEvent,
    pub abs: InputMoveEvent,
    pub btn: InputBtnEvent,
    pub key: InputKeyEvent,
}

impl Default for InputEvent {
    fn default() -> Self {
        Self {
            kind: 0,
            rel: InputMoveEvent { axis: 0, value: 0 },
            abs: InputMoveEvent { axis: 0, value: 0 },
            btn: InputBtnEvent { button: 0, down: false },
            key: InputKeyEvent { key: KeyValue { kind: KeyValueKind::Number, qcode: QKeyCode::_0, number: 0 }, down: false },
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct HidPointerEvent {
    pub xdx: i32,
    pub ydy: i32,
    pub dz: i32,
    pub buttons_state: u8,
}

#[derive(Debug, Clone, Copy)]
pub struct HidKbd {
    pub keycodes: [i32; QUEUE_LENGTH],
    pub key: [u8; 8],
    pub keys: i32,
    pub modifiers: u32,
    pub leds: u8,
}

impl Default for HidKbd {
    fn default() -> Self {
        Self {
            keycodes: [0; QUEUE_LENGTH],
            key: [0; 8],
            keys: 0,
            modifiers: 0,
            leds: 0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct HidPtr {
    pub queue: [HidPointerEvent; QUEUE_LENGTH],
    pub mouse_grabbed: i32,
    pub eh_entry: Option<fn(&HidState, &InputEvent)>,
    pub eh_sync: Option<fn(&HidState)>,
}

impl Default for HidPtr {
    fn default() -> Self {
        Self {
            queue: [HidPointerEvent::default(); QUEUE_LENGTH],
            mouse_grabbed: 0,
            eh_entry: None,
            eh_sync: None,
        }
    }
}

pub type HidEventFunc = fn(&HidState);

/// The HID device state.  A single `HIDState` is shared between the
/// keyboard, mouse, and tablet sub-drivers.
#[derive(Debug, Clone, Copy)]
pub struct HidState {
    pub kind: i32,
    pub event: Option<HidEventFunc>,
    pub head: u32,
    pub n: u32,
    pub protocol: i32,
    pub idle: i32,
    pub kbd: HidKbd,
    pub ptr: HidPtr,
}

impl Default for HidState {
    fn default() -> Self {
        Self {
            kind: HID_KEYBOARD,
            event: None,
            head: 0,
            n: 0,
            protocol: 1,
            idle: 0,
            kbd: HidKbd::default(),
            ptr: HidPtr::default(),
        }
    }
}

// =====================================================================
// 9.  OHCI register file and supporting types
// =====================================================================

#[derive(Debug, Default, Clone)]
pub struct OhciPort {
    pub port: UsbPort,
    pub ctrl: u32,
}

#[derive(Debug, Clone)]
pub struct OhciEd {
    pub flags: u32,
    pub tail: u32,
    pub head: u32,
    pub next: u32,
}

impl Default for OhciEd {
    fn default() -> Self { Self { flags: 0, tail: 0, head: 0, next: 0 } }
}

#[derive(Debug, Clone)]
pub struct OhciTd {
    pub flags: u32,
    pub cbp: u32,
    pub next: u32,
    pub be: u32,
}

impl Default for OhciTd {
    fn default() -> Self { Self { flags: 0, cbp: 0, next: 0, be: 0 } }
}

#[derive(Debug, Clone)]
pub struct OhciIsoTd {
    pub flags: u32,
    pub bp: u32,
    pub next: u32,
    pub be: u32,
    pub offset: [u16; 8],
}

impl Default for OhciIsoTd {
    fn default() -> Self { Self { flags: 0, bp: 0, next: 0, be: 0, offset: [0; 8] } }
}

#[derive(Debug, Clone, Default)]
pub struct OhciHcca {
    pub intr: [u32; 32],
    pub frame: u16,
    pub pad: u16,
    pub done: u32,
}

/// The OpenHCI register file.  Mirrors `OHCIState` from the C source.
#[derive(Debug, Clone)]
pub struct OhciState {
    pub mem_base: u32,
    pub num_ports: u32,

    pub eof_timer: u64,
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
    pub done_count: i32,

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
    pub rhstatus: u32,
    pub rhport: [OhciPort; OHCI_MAX_PORTS],

    pub old_ctl: u32,
    pub usb_packet: UsbPacket,
    pub usb_buf: [u8; 8192],
    pub async_td: u32,
    pub async_complete: bool,
}

impl Default for OhciState {
    fn default() -> Self {
        Self {
            mem_base: 0,
            num_ports: 0,
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
            rhstatus: 0,
            rhport: std::array::from_fn(|_| OhciPort::default()),
            old_ctl: 0,
            usb_packet: UsbPacket::default(),
            usb_buf: [0; 8192],
            async_td: 0,
            async_complete: false,
        }
    }
}

// =====================================================================
// 10.  Public register & control surface (USBReg, usbInit, ...)
// =====================================================================

/// Mirror of the C `RegisterDevice`/host state, exposed as a single
/// struct so the rest of the emulator can poke at it directly.
#[derive(Debug, Clone)]
pub struct UsbReg {
    pub version: u32,
    pub ohci: Option<usize>,
    pub frame_time: i64,
    pub bit_time: i64,
    pub last_cycle: i64,
    pub clocks: i64,
    pub remaining: i64,
    pub devices: Vec<UsbDevice>,
    pub buses: Vec<UsbBus>,
    pub ports: Vec<UsbPort>,
}

impl UsbReg {
    pub const fn new() -> Self {
        Self {
            version: 0x0110,
            ohci: None,
            frame_time: 0,
            bit_time: 0,
            last_cycle: 0,
            clocks: 0,
            remaining: 0,
            devices: Vec::new(),
            buses: Vec::new(),
            ports: Vec::new(),
        }
    }
}

/// Global USB register file.  Modelled as `static mut` per the rules of
/// the translation task.
pub static mut USB_REG: UsbReg = UsbReg::new();

/// Backwards-compatible alias requested by the task brief.
pub static mut usbReg: UsbReg = UsbReg::new();

/// Construct the OHCI controller and create a device on every port.
pub fn usb_init() {
    unsafe {
        USB_REG.frame_time = PSXCLK as i64;
        USB_REG.bit_time = ((PSXCLK as i64) / (USB_HZ as i64)).max(1);
        USB_REG.last_cycle = 0;
        USB_REG.clocks = 0;
        USB_REG.remaining = 0;
        let ohci = ohci_create(0x1F80_1600, OHCI_MAX_PORTS as i32);
        USB_REG.ohci = Some(Box::leak(Box::new(ohci)) as *mut OhciState as usize);
    }
}

/// Hard reset the controller and the attached devices.
pub fn usb_reset() {
    unsafe {
        USB_REG.clocks = 0;
        USB_REG.remaining = 0;
        USB_REG.last_cycle = 0;
        if let Some(ptr) = USB_REG.ohci {
            ohci_hard_reset(&mut *(ptr as *mut OhciState));
        }
    }
}

/// Tear down the controller and detach all devices.
pub fn usb_shutdown() {
    unsafe {
        USB_REG.devices.clear();
        USB_REG.buses.clear();
        USB_REG.ports.clear();
        if let Some(ptr) = USB_REG.ohci {
            let _ = Box::from_raw(ptr as *mut OhciState);
            USB_REG.ohci = None;
        }
    }
}

#[inline]
pub fn ohci_mem_read(addr: u32) -> u32 {
    unsafe {
        match USB_REG.ohci {
            Some(ptr) => ohci_mem_read_impl(&mut *(ptr as *mut OhciState), addr),
            None => 0,
        }
    }
}

#[inline]
pub fn ohci_mem_write(addr: u32, val: u32) {
    unsafe {
        if let Some(ptr) = USB_REG.ohci {
            ohci_mem_write_impl(&mut *(ptr as *mut OhciState), addr, val);
        }
    }
}

pub fn usb_read8(_addr: u32) -> u8 { 0 }
pub fn usb_read16(_addr: u32) -> u16 { 0 }
pub fn usb_read32(addr: u32) -> u32 { ohci_mem_read(addr) }
pub fn usb_write8(_addr: u32, _val: u8) {}
pub fn usb_write16(_addr: u32, _val: u16) {}
pub fn usb_write32(addr: u32, val: u32) { ohci_mem_write(addr, val); }

// =====================================================================
// 11.  Packet helpers
// =====================================================================

#[inline]
pub fn usb_packet_is_inflight(p: &UsbPacket) -> bool {
    p.state == UsbPacketState::Queued || p.state == UsbPacketState::Async
}

pub fn usb_packet_set_state(p: &mut UsbPacket, state: UsbPacketState) {
    p.state = state;
}

pub fn usb_packet_check_state(p: &UsbPacket, expected: UsbPacketState) {
    if p.state != expected {
        // Mirrors `assert(!"usb packet state check failed")` in C.
        panic!("usb packet state check failed");
    }
}

pub fn usb_packet_setup(p: &mut UsbPacket, pid: i32, ep_idx: Option<usize>, stream: u32, id: u64,
                        short_not_ok: bool, int_req: bool) {
    p.id = id;
    p.pid = pid;
    p.ep = ep_idx;
    p.stream = stream;
    p.status = USB_RET_SUCCESS;
    p.actual_length = 0;
    p.parameter = 0;
    p.short_not_ok = short_not_ok;
    p.int_req = int_req;
    p.buffer_ptr = None;
    p.buffer_size = 0;
    p.state = UsbPacketState::Setup;
}

pub fn usb_packet_addbuf(p: &mut UsbPacket, data: Vec<u8>) {
    p.buffer_size = data.len() as u32;
    p.buffer_ptr = Some(data);
}

pub fn usb_packet_copy(p: &mut UsbPacket, src: &[u8]) {
    let bytes = src.len();
    let ptr = p.buffer_ptr.get_or_insert_with(Vec::new);
    if ptr.len() < bytes {
        ptr.resize(bytes, 0);
    }
    match p.pid as u8 {
        USB_TOKEN_SETUP | USB_TOKEN_OUT => {
            // host -> device: copy into our buffer
            ptr[..bytes].copy_from_slice(src);
        }
        USB_TOKEN_IN => {
            // device -> host: copy from our buffer
            let _ = src; // dst written to by caller
        }
        _ => {}
    }
    p.actual_length += bytes as i32;
}

pub fn usb_packet_skip(p: &mut UsbPacket, bytes: usize) {
    if let Some(buf) = p.buffer_ptr.as_mut() {
        if (p.pid as u8) == USB_TOKEN_IN {
            for b in buf.iter_mut().take(bytes) { *b = 0; }
        }
    }
    p.actual_length += bytes as i32;
}

pub fn usb_packet_size(p: &UsbPacket) -> usize { p.buffer_size as usize }

pub fn usb_packet_cleanup(p: &mut UsbPacket) {
    p.buffer_ptr = None;
    p.buffer_size = 0;
}

// =====================================================================
// 12.  Bus / port / device operations
// =====================================================================

pub fn usb_attach(ports: &mut [UsbPort], idx: usize, dev: &mut UsbDevice) {
    let p = &mut ports[idx];
    dev.attached = true;
    dev.state = USB_STATE_ATTACHED;
    if let Some(op) = p.ops { let _ = op; }
    usb_pick_speed(p, dev);
}

pub fn usb_detach(ports: &mut [UsbPort], idx: usize) {
    let p = &mut ports[idx];
    if let Some(_dev) = p.dev.take() {
        // device handle cleared
    }
    p.next = None;
    p.prev = None;
}

pub fn usb_reattach(ports: &mut [UsbPort], idx: usize, dev: &mut UsbDevice) {
    usb_detach(ports, idx);
    usb_attach(ports, idx, dev);
}

pub fn usb_port_reset(ports: &mut [UsbPort], idx: usize, dev: &mut UsbDevice) {
    usb_detach(ports, idx);
    usb_attach(ports, idx, dev);
    usb_device_reset(dev);
}

pub fn usb_pick_speed(port: &mut UsbPort, dev: &mut UsbDevice) {
    const SPEEDS: [u8; 2] = [USB_SPEED_FULL, USB_SPEED_LOW];
    for s in SPEEDS.iter() {
        if (dev.speedmask & (1 << s)) != 0 && (port.speedmask & (1 << s)) != 0 {
            dev.speed = *s as i32;
            return;
        }
    }
}

pub fn usb_device_reset(dev: &mut UsbDevice) {
    if !dev.attached { return; }
    dev.remote_wakeup = 0;
    dev.addr = 0;
    dev.state = USB_STATE_DEFAULT;
    if let Some(cb) = dev.klass.handle_reset { cb(dev); }
}

pub fn usb_find_device(port: &UsbPort, devs: &[UsbDevice], addr: u8) -> Option<usize> {
    let dev_idx = port.dev?;
    let dev = devs.get(dev_idx)?;
    if !dev.attached || dev.state != USB_STATE_DEFAULT { return None; }
    if dev.addr == addr { return Some(dev_idx); }
    if let Some(f) = dev.klass.find_device { return f(dev, addr); }
    None
}

pub fn usb_device_find_device(dev: &UsbDevice, addr: u8) -> Option<usize> {
    if let Some(f) = dev.klass.find_device { f(dev, addr) } else { None }
}

pub fn usb_device_cancel_packet(dev: &UsbDevice, p: &mut UsbPacket) {
    if let Some(cb) = dev.klass.cancel_packet { cb(dev, p); }
}

pub fn usb_device_handle_attach(dev: &UsbDevice) {
    if let Some(cb) = dev.klass.handle_attach { cb(dev); }
}

pub fn usb_device_handle_reset(dev: &UsbDevice) {
    if let Some(cb) = dev.klass.handle_reset { cb(dev); }
}

pub fn usb_device_handle_control(dev: &UsbDevice, p: &mut UsbPacket, request: i32, value: i32,
                                 index: i32, length: i32, data: &mut [u8]) {
    if let Some(cb) = dev.klass.handle_control { cb(dev, p, request, value, index, length, data); }
}

pub fn usb_device_handle_data(dev: &UsbDevice, p: &mut UsbPacket) {
    if let Some(cb) = dev.klass.handle_data { cb(dev, p); }
}

pub fn usb_device_set_interface(dev: &UsbDevice, intf: i32, alt_old: i32, alt_new: i32) {
    if let Some(cb) = dev.klass.set_interface { cb(dev, intf, alt_old, alt_new); }
}

pub fn usb_device_flush_ep_queue(dev: &UsbDevice, ep: &UsbEndpoint) {
    if let Some(cb) = dev.klass.flush_ep_queue { cb(dev, ep); }
}

pub fn usb_device_ep_stopped(dev: &UsbDevice, ep: &UsbEndpoint) {
    if let Some(cb) = dev.klass.ep_stopped { cb(dev, ep); }
}

pub fn usb_device_alloc_streams(dev: &UsbDevice, eps: &mut [UsbEndpoint], streams: i32) -> i32 {
    if let Some(cb) = dev.klass.alloc_streams { cb(dev, eps, streams) } else { 0 }
}

pub fn usb_device_free_streams(dev: &UsbDevice, eps: &mut [UsbEndpoint]) {
    if let Some(cb) = dev.klass.free_streams { cb(dev, eps); }
}

pub fn usb_wakeup_endpoint(dev: &UsbDevice) {
    if let Some(_port) = dev.port {
        if dev.remote_wakeup != 0 {
            // Wake callback would fire here.
        }
    }
}

pub fn usb_wakeup(ep: &UsbEndpoint, _stream: u32) {
    let _ = ep;
}

pub fn usb_generic_async_ctrl_complete(s: &mut UsbDevice, p: &mut UsbPacket) {
    if p.status < 0 { s.setup_state = SETUP_STATE_IDLE; }
    match s.setup_state {
        SETUP_STATE_SETUP => {
            if p.actual_length < s.setup_len { s.setup_len = p.actual_length; }
            s.setup_state = SETUP_STATE_DATA;
            p.actual_length = 8;
        }
        SETUP_STATE_ACK => {
            s.setup_state = SETUP_STATE_IDLE;
            p.actual_length = 0;
        }
        SETUP_STATE_PARAM => {
            if p.actual_length < s.setup_len { s.setup_len = p.actual_length; }
            if (p.pid as u8) == USB_TOKEN_IN {
                p.actual_length = 0;
                if let Some(buf) = p.buffer_ptr.as_mut() {
                    let n = s.setup_len as usize;
                    if buf.len() < n { buf.resize(n, 0); }
                    buf[..n].copy_from_slice(&s.data_buf[..n]);
                }
            }
        }
        _ => {}
    }
    usb_packet_complete(s, p);
}

pub fn usb_handle_packet(dev: &mut UsbDevice, p: &mut UsbPacket, ep_idx: Option<usize>) {
    if p.ep != ep_idx { p.ep = ep_idx; }
    if p.ep.is_none() { p.status = USB_RET_NODEV; return; }
    p.status = USB_RET_SUCCESS;
    if let Some(ei) = ep_idx {
        if ei == 0 {
            // control pipe
            if p.parameter != 0 { return; }
            match p.pid as u8 {
                USB_TOKEN_SETUP => dev.setup_packet(p),
                USB_TOKEN_IN | USB_TOKEN_OUT => {} // state machine handled in do_token_*
                _ => p.status = USB_RET_STALL,
            }
        } else {
            dev.handle_data(p);
        }
    }
}

pub fn usb_packet_complete_one(dev: &mut UsbDevice, p: &mut UsbPacket) {
    if p.status != USB_RET_SUCCESS || (p.short_not_ok && (p.actual_length as u32) < p.buffer_size) {
        if let Some(ei) = p.ep { dev.ep_set_halted(ei, true); }
    }
    p.state = UsbPacketState::Complete;
    // unlink from endpoint queue
}

pub fn usb_packet_complete(dev: &mut UsbDevice, p: &mut UsbPacket) {
    usb_packet_complete_one(dev, p);
}

pub fn usb_cancel_packet(p: &mut UsbPacket) {
    p.state = UsbPacketState::Canceled;
}

impl UsbDevice {
    /// Convenience wrapper around the per-direction endpoint arrays used
    /// throughout the OHCI driver.
    pub fn ep_set_halted(&mut self, ep_idx: usize, halted: bool) {
        if ep_idx == 0 { self.ep_ctl.halted = halted; return; }
        if (1..=USB_MAX_ENDPOINTS).contains(&ep_idx) {
            self.ep_in[ep_idx - 1].halted = halted;
            self.ep_out[ep_idx - 1].halted = halted;
        }
    }
}

// =====================================================================
// 13.  Endpoint helpers
// =====================================================================

pub fn usb_ep_get<'a>(dev: &'a UsbDevice, pid: i32, ep: i32) -> Option<&'a UsbEndpoint> {
    if ep == 0 { return Some(&dev.ep_ctl); }
    if !(1..=USB_MAX_ENDPOINTS as i32).contains(&ep) { return None; }
    let idx = (ep - 1) as usize;
    match pid as u8 {
        USB_TOKEN_IN => Some(&dev.ep_in[idx]),
        USB_TOKEN_OUT => Some(&dev.ep_out[idx]),
        _ => None,
    }
}

pub fn usb_ep_get_mut<'a>(dev: &'a mut UsbDevice, pid: i32, ep: i32) -> Option<&'a mut UsbEndpoint> {
    if ep == 0 { return Some(&mut dev.ep_ctl); }
    if !(1..=USB_MAX_ENDPOINTS as i32).contains(&ep) { return None; }
    let idx = (ep - 1) as usize;
    match pid as u8 {
        USB_TOKEN_IN => Some(&mut dev.ep_in[idx]),
        USB_TOKEN_OUT => Some(&mut dev.ep_out[idx]),
        _ => None,
    }
}

pub fn usb_ep_get_type(dev: &UsbDevice, pid: i32, ep: i32) -> u8 {
    usb_ep_get(dev, pid, ep).map(|e| e.r#type).unwrap_or(USB_ENDPOINT_XFER_INVALID)
}

pub fn usb_ep_set_type(dev: &mut UsbDevice, pid: i32, ep: i32, ty: u8) {
    if let Some(e) = usb_ep_get_mut(dev, pid, ep) { e.r#type = ty; }
}

pub fn usb_ep_set_ifnum(dev: &mut UsbDevice, pid: i32, ep: i32, ifnum: u8) {
    if let Some(e) = usb_ep_get_mut(dev, pid, ep) { e.ifnum = ifnum; }
}

pub fn usb_ep_set_max_packet_size(dev: &mut UsbDevice, pid: i32, ep: i32, raw: u16) {
    let size = (raw & 0x7ff) as i32;
    let micro = match (raw >> 11) & 3 {
        1 => 2, 2 => 3, _ => 1,
    };
    if let Some(e) = usb_ep_get_mut(dev, pid, ep) { e.max_packet_size = size * micro; }
}

pub fn usb_ep_set_max_streams(dev: &mut UsbDevice, pid: i32, ep: i32, raw: u8) {
    let ms = (raw & 0x1f) as i32;
    let v = if ms != 0 { 1 << ms } else { 0 };
    if let Some(e) = usb_ep_get_mut(dev, pid, ep) { e.max_streams = v; }
}

pub fn usb_ep_set_halted(dev: &mut UsbDevice, pid: i32, ep: i32, halted: bool) {
    if let Some(e) = usb_ep_get_mut(dev, pid, ep) { e.halted = halted; }
}

pub fn usb_ep_find_packet_by_id(_dev: &UsbDevice, _pid: i32, _ep: i32, _id: u64) -> Option<usize> {
    None
}

pub fn usb_ep_reset(dev: &mut UsbDevice) {
    dev.ep_ctl = UsbEndpoint { nr: 0, pid: 0, r#type: USB_ENDPOINT_XFER_CONTROL,
        ifnum: 0, max_packet_size: 64, max_streams: 0,
        pipeline: false, halted: false, dev: 0,
        queue_head: None, queue_tail: None };
    for i in 0..USB_MAX_ENDPOINTS {
        dev.ep_in[i] = UsbEndpoint { nr: (i + 1) as u8, pid: USB_TOKEN_IN,
            r#type: USB_ENDPOINT_XFER_INVALID, ifnum: USB_INTERFACE_INVALID,
            max_packet_size: 0, max_streams: 0, pipeline: false, halted: false,
            dev: 0, queue_head: None, queue_tail: None };
        dev.ep_out[i] = UsbEndpoint { nr: (i + 1) as u8, pid: USB_TOKEN_OUT,
            r#type: USB_ENDPOINT_XFER_INVALID, ifnum: USB_INTERFACE_INVALID,
            max_packet_size: 0, max_streams: 0, pipeline: false, halted: false,
            dev: 0, queue_head: None, queue_tail: None };
    }
}

pub fn usb_ep_init(dev: &mut UsbDevice) {
    usb_ep_reset(dev);
}

pub fn usb_ep_dump(dev: &UsbDevice) {
    eprintln!("Device \"{}\", config {}", dev.product_desc, dev.configuration);
    for ifnum in 0..16 {
        let mut first = true;
        for ep in 0..USB_MAX_ENDPOINTS {
            if dev.ep_in[ep].r#type != USB_ENDPOINT_XFER_INVALID && dev.ep_in[ep].ifnum == ifnum as u8 {
                if first { first = false; }
                eprintln!("  EP {} IN type={} mps={}", ep, dev.ep_in[ep].r#type, dev.ep_in[ep].max_packet_size);
            }
            if dev.ep_out[ep].r#type != USB_ENDPOINT_XFER_INVALID && dev.ep_out[ep].ifnum == ifnum as u8 {
                if first { first = false; }
                eprintln!("  EP {} OUT type={} mps={}", ep, dev.ep_out[ep].r#type, dev.ep_out[ep].max_packet_size);
            }
        }
    }
}

// =====================================================================
// 14.  Descriptor helpers
// =====================================================================

pub fn usb_desc_device(id: &UsbDescId, dev: &UsbDescDevice, msos: bool, dest: &mut [u8]) -> i32 {
    const BLENGTH: u8 = 0x12;
    if dest.len() < BLENGTH as usize { return -1; }
    dest[0] = BLENGTH;
    dest[1] = USB_DT_DEVICE;
    if msos && dev.bcd_usb < 0x0200 {
        dest[2] = usb_lo(0x0200);
        dest[3] = usb_hi(0x0200);
    } else {
        dest[2] = usb_lo(dev.bcd_usb);
        dest[3] = usb_hi(dev.bcd_usb);
    }
    dest[4] = dev.b_device_class;
    dest[5] = dev.b_device_sub_class;
    dest[6] = dev.b_device_protocol;
    dest[7] = dev.b_max_packet_size0;
    dest[8] = usb_lo(id.id_vendor); dest[9] = usb_hi(id.id_vendor);
    dest[10] = usb_lo(id.id_product); dest[11] = usb_hi(id.id_product);
    dest[12] = usb_lo(id.bcd_device); dest[13] = usb_hi(id.bcd_device);
    dest[14] = id.i_manufacturer;
    dest[15] = id.i_product;
    dest[16] = id.i_serial_number;
    dest[17] = dev.b_num_configurations;
    BLENGTH as i32
}

pub fn usb_desc_device_qualifier(dev: &UsbDescDevice, dest: &mut [u8]) -> i32 {
    const BLENGTH: u8 = 0x0a;
    if dest.len() < BLENGTH as usize { return -1; }
    dest[0] = BLENGTH;
    dest[1] = USB_DT_DEVICE_QUALIFIER;
    dest[2] = usb_lo(dev.bcd_usb);
    dest[3] = usb_hi(dev.bcd_usb);
    dest[4] = dev.b_device_class;
    dest[5] = dev.b_device_sub_class;
    dest[6] = dev.b_device_protocol;
    dest[7] = dev.b_max_packet_size0;
    dest[8] = dev.b_num_configurations;
    dest[9] = 0;
    BLENGTH as i32
}

pub fn usb_desc_config(conf: &UsbDescConfig, flags: i32, dest: &mut [u8]) -> i32 {
    const BLENGTH: u8 = 0x09;
    if dest.len() < BLENGTH as usize { return -1; }
    dest[0] = BLENGTH;
    dest[1] = USB_DT_CONFIG;
    dest[2] = conf.b_num_interfaces;
    dest[3] = conf.b_configuration_value;
    dest[4] = conf.i_configuration;
    dest[5] = conf.bm_attributes;
    dest[6] = conf.b_max_power;
    let mut total = BLENGTH as i32;
    for iad in &conf.if_groups {
        let n = usb_desc_iface_group(iad, flags, &mut dest[total as usize..]);
        if n < 0 { return n; }
        total += n;
    }
    for iface in &conf.ifs {
        let n = usb_desc_iface(iface, flags, &mut dest[total as usize..]);
        if n < 0 { return n; }
        total += n;
    }
    dest[2] = usb_lo(total as u16);
    dest[3] = usb_hi(total as u16);
    total
}

pub fn usb_desc_iface_group(iad: &UsbDescIfaceAssoc, _flags: i32, dest: &mut [u8]) -> i32 {
    const BLENGTH: u8 = 0x08;
    if dest.len() < BLENGTH as usize { return -1; }
    dest[0] = BLENGTH;
    dest[1] = USB_DT_INTERFACE_ASSOC;
    dest[2] = iad.b_first_interface;
    dest[3] = iad.b_interface_count;
    dest[4] = iad.b_function_class;
    dest[5] = iad.b_function_sub_class;
    dest[6] = iad.b_function_protocol;
    dest[7] = iad.i_function;
    let mut pos = BLENGTH as i32;
    for iface in &iad.ifs {
        let n = usb_desc_iface(iface, 0, &mut dest[pos as usize..]);
        if n < 0 { return n; }
        pos += n;
    }
    pos
}

pub fn usb_desc_iface(iface: &UsbDescIface, _flags: i32, dest: &mut [u8]) -> i32 {
    const BLENGTH: u8 = 0x09;
    if dest.len() < BLENGTH as usize { return -1; }
    dest[0] = BLENGTH;
    dest[1] = USB_DT_INTERFACE;
    dest[2] = iface.b_interface_number;
    dest[3] = iface.b_alternate_setting;
    dest[4] = iface.b_num_endpoints;
    dest[5] = iface.b_interface_class;
    dest[6] = iface.b_interface_sub_class;
    dest[7] = iface.b_interface_protocol;
    dest[8] = iface.i_interface;
    let mut pos = BLENGTH as i32;
    for d in &iface.descs {
        let n = usb_desc_other(d, &mut dest[pos as usize..]);
        if n < 0 { return n; }
        pos += n;
    }
    for ep in &iface.eps {
        let n = usb_desc_endpoint(ep, 0, &mut dest[pos as usize..]);
        if n < 0 { return n; }
        pos += n;
    }
    pos
}

pub fn usb_desc_endpoint(ep: &UsbDescEndpoint, _flags: i32, dest: &mut [u8]) -> i32 {
    let blength: u8 = if ep.is_audio { 0x09 } else { 0x07 };
    let extra_len = ep.extra.as_ref().map(|e| e[0]).unwrap_or(0);
    if dest.len() < (blength as usize) + (extra_len as usize) { return -1; }
    dest[0] = blength;
    dest[1] = USB_DT_ENDPOINT;
    dest[2] = ep.b_endpoint_address;
    dest[3] = ep.bm_attributes;
    dest[4] = usb_lo(ep.w_max_packet_size);
    dest[5] = usb_hi(ep.w_max_packet_size);
    dest[6] = ep.b_interval;
    if ep.is_audio {
        dest[7] = ep.b_refresh;
        dest[8] = ep.b_synch_address;
    }
    if let Some(extra) = &ep.extra {
        let n = extra_len as usize;
        dest[blength as usize..blength as usize + n].copy_from_slice(&extra[..n]);
    }
    blength as i32 + extra_len as i32
}

pub fn usb_desc_other(desc: &UsbDescOther, dest: &mut [u8]) -> i32 {
    let b = if desc.length != 0 { desc.length as usize } else { desc.data.get(0).copied().unwrap_or(0) as usize };
    if dest.len() < b { return -1; }
    dest[..b].copy_from_slice(&desc.data[..b]);
    b as i32
}

pub fn usb_desc_cap_usb2_ext(dest: &mut [u8]) -> i32 {
    const BLENGTH: u8 = 0x07;
    if dest.len() < BLENGTH as usize { return -1; }
    dest[0] = BLENGTH;
    dest[1] = USB_DT_DEVICE_CAPABILITY;
    dest[2] = USB_DEV_CAP_USB2_EXT;
    dest[3] = 1 << 1;
    dest[4] = 0; dest[5] = 0; dest[6] = 0;
    BLENGTH as i32
}

pub fn usb_desc_cap_super(desc: &UsbDesc, dest: &mut [u8]) -> i32 {
    const BLENGTH: u8 = 0x0a;
    if dest.len() < BLENGTH as usize { return -1; }
    dest[0] = BLENGTH;
    dest[1] = USB_DT_DEVICE_CAPABILITY;
    dest[2] = USB_DEV_CAP_SUPERSPEED;
    dest[3] = 0;
    dest[4] = 0; dest[5] = 0;
    dest[6] = 0;
    dest[7] = 0x0a;
    dest[8] = 0x20; dest[9] = 0;
    if desc.full.is_some() { dest[5] |= 1 << 1; dest[6] = 1; }
    if desc.high.is_some() { dest[5] |= 1 << 2; if dest[6] == 0 { dest[6] = 2; } }
    if desc.super_.is_some() { dest[5] |= 1 << 3; if dest[6] == 0 { dest[6] = 3; } }
    BLENGTH as i32
}

pub fn usb_desc_bos(desc: &UsbDesc, dest: &mut [u8]) -> i32 {
    const BLENGTH: u8 = 0x05;
    if dest.len() < BLENGTH as usize { return -1; }
    dest[0] = BLENGTH;
    dest[1] = USB_DT_BOS;
    let mut total = BLENGTH as i32;
    let mut ncaps = 0u8;
    if desc.high.is_some() {
        let n = usb_desc_cap_usb2_ext(&mut dest[total as usize..]);
        if n < 0 { return n; }
        total += n; ncaps += 1;
    }
    if desc.super_.is_some() {
        let n = usb_desc_cap_super(desc, &mut dest[total as usize..]);
        if n < 0 { return n; }
        total += n; ncaps += 1;
    }
    dest[2] = usb_lo(total as u16);
    dest[3] = usb_hi(total as u16);
    dest[4] = ncaps;
    total
}

pub fn usb_desc_parse_dev(data: &[u8], desc: &mut UsbDesc, dev: &mut UsbDescDevice) -> i32 {
    if data.len() < 18 || data[0] != 18 || data[1] != USB_DT_DEVICE { return -1; }
    dev.bcd_usb = (data[2] as u16) | ((data[3] as u16) << 8);
    dev.b_device_class = data[4];
    dev.b_device_sub_class = data[5];
    dev.b_device_protocol = data[6];
    dev.b_max_packet_size0 = data[7];
    desc.id.id_vendor = (data[8] as u16) | ((data[9] as u16) << 8);
    desc.id.id_product = (data[10] as u16) | ((data[11] as u16) << 8);
    desc.id.bcd_device = (data[12] as u16) | ((data[13] as u16) << 8);
    desc.id.i_manufacturer = data[14];
    desc.id.i_product = data[15];
    desc.id.i_serial_number = data[16];
    dev.b_num_configurations = data[17];
    18
}

pub fn usb_desc_parse_config(data: &[u8], dev: &mut UsbDescDevice) -> i32 {
    let mut pos = 0usize;
    let mut config: Option<usize> = None;
    let mut iface: Option<usize> = None;
    while pos < data.len() {
        let d = &data[pos..];
        if d.is_empty() { break; }
        let blength = d[0] as usize;
        if blength == 0 { break; }
        let btype = d[1];
        match btype {
            USB_DT_CONFIG => {
                dev.confs.push(UsbDescConfig {
                    b_num_interfaces: d[2],
                    b_configuration_value: d[3],
                    i_configuration: d[4],
                    bm_attributes: d[5],
                    b_max_power: d[6],
                    ..Default::default()
                });
                config = Some(dev.confs.len() - 1);
                iface = None;
            }
            USB_DT_INTERFACE => {
                let cfg_idx = match config { Some(i) => i, None => return -1 };
                let cfg = &mut dev.confs[cfg_idx];
                cfg.ifs.push(UsbDescIface {
                    b_interface_number: d[2],
                    b_alternate_setting: d[3],
                    b_num_endpoints: d[4],
                    b_interface_class: d[5],
                    b_interface_sub_class: d[6],
                    b_interface_protocol: d[7],
                    i_interface: d[8],
                    ..Default::default()
                });
                iface = Some(cfg.ifs.len() - 1);
            }
            USB_DT_ENDPOINT => {
                let cfg_idx = match config { Some(c) => c, None => return -1 };
                let iface_idx = match iface { Some(i) => i, None => return -1 };
                let is_audio = blength == 9;
                let mut ep = UsbDescEndpoint {
                    b_endpoint_address: d[2],
                    bm_attributes: d[3],
                    w_max_packet_size: (d[4] as u16) | ((d[5] as u16) << 8),
                    b_interval: d[6],
                    is_audio,
                    ..Default::default()
                };
                if is_audio && pos + blength < data.len() {
                    let extra_start = pos + blength;
                    let extra_len = data[extra_start];
                    ep.extra = Some(data[extra_start..extra_start + extra_len as usize].to_vec());
                    pos += extra_len as usize;
                }
                dev.confs[cfg_idx].ifs[iface_idx].eps.push(ep);
            }
            USB_DT_HID | USB_DT_CS_INTERFACE | USB_DT_CS_ENDPOINT => {
                let cfg_idx = match config { Some(c) => c, None => return -1 };
                let iface_idx = match iface { Some(i) => i, None => return -1 };
                let mut o = UsbDescOther::default();
                o.length = blength as u8;
                o.data = d[..blength].to_vec();
                dev.confs[cfg_idx].ifs[iface_idx].descs.push(o);
            }
            0x00 | USB_DT_OTHER_SPEED_CONFIG | USB_DT_DEBUG => {}
            _ => return -1,
        }
        pos += blength;
    }
    pos as i32
}

pub fn usb_desc_string(dev: &UsbDevice, idx: u8, dest: &mut [u8]) -> i32 {
    if dest.len() < 4 { return -1; }
    if idx == 0 {
        dest[0] = 4;
        dest[1] = USB_DT_STRING;
        dest[2] = 0x09;
        dest[3] = 0x04;
        return 4;
    }
    // Look up via the device's USB descriptor strings (not modelled per-device here).
    let s = String::new();
    let bytes = s.as_bytes();
    let blen = (bytes.len() * 2 + 2) as u8;
    dest[0] = blen;
    dest[1] = USB_DT_STRING;
    let mut pos = 2usize;
    let mut i = 0usize;
    while pos + 1 < blen as usize && pos + 1 < dest.len() && i < bytes.len() {
        dest[pos] = bytes[i]; dest[pos + 1] = 0;
        pos += 2; i += 1;
    }
    pos as i32
}

pub fn usb_desc_get_descriptor(dev: &UsbDevice, value: i32, dest: &mut [u8]) -> i32 {
    let mut buf = [0u8; 1024];
    let ty = (value >> 8) as u8;
    let index = (value & 0xff) as u8;
    let mut ret = -1i32;
    if let Some(d) = dev.device {
        let _ = d;
    }
    match ty {
        USB_DT_DEVICE => {
            // Pull from dev.device's UsbDesc (not stored on the device; left to caller).
            ret = 0;
        }
        USB_DT_CONFIG => ret = 0,
        USB_DT_STRING => {
            let n = usb_desc_string(dev, index, &mut buf);
            if n > 0 { ret = n; }
        }
        USB_DT_DEVICE_QUALIFIER => ret = 0,
        USB_DT_OTHER_SPEED_CONFIG => ret = 0,
        USB_DT_BOS => ret = 0,
        USB_DT_DEBUG => {}
        _ => {}
    }
    if ret > 0 {
        let n = (ret as usize).min(dest.len());
        dest[..n].copy_from_slice(&buf[..n]);
        ret = 0;
    }
    ret
}

pub fn usb_desc_handle_control(dev: &mut UsbDevice, p: &mut UsbPacket,
                               request: i32, value: i32, index: i32, length: i32, data: &mut [u8]) -> i32 {
    let mut ret = -1i32;
    match request {
        x if x == (DEVICE_OUT_REQUEST as i32 | (USB_REQ_SET_ADDRESS as i32) << 8) => {
            dev.addr = value as u8;
            ret = 0;
        }
        x if x == (DEVICE_REQUEST as i32 | (USB_REQ_GET_DESCRIPTOR as i32) << 8) => {
            ret = usb_desc_get_descriptor(dev, value, data);
        }
        x if x == (DEVICE_REQUEST as i32 | (USB_REQ_GET_CONFIGURATION as i32) << 8) => {
            data[0] = if let Some(_cfg) = dev.config { 1 } else { 0 };
            p.actual_length = 1;
            ret = 0;
        }
        x if x == (DEVICE_OUT_REQUEST as i32 | (USB_REQ_SET_CONFIGURATION as i32) << 8) => {
            dev.configuration = value;
            ret = 0;
        }
        x if x == (DEVICE_REQUEST as i32 | (USB_REQ_GET_STATUS as i32) << 8) => {
            data[0] = 0;
            if dev.remote_wakeup != 0 { data[0] |= 1 << USB_DEVICE_REMOTE_WAKEUP; }
            data[1] = 0;
            p.actual_length = 2;
            ret = 0;
        }
        x if x == (DEVICE_OUT_REQUEST as i32 | (USB_REQ_CLEAR_FEATURE as i32) << 8) => {
            if value == USB_DEVICE_REMOTE_WAKEUP as i32 { dev.remote_wakeup = 0; ret = 0; }
        }
        x if x == (DEVICE_OUT_REQUEST as i32 | (USB_REQ_SET_FEATURE as i32) << 8) => {
            if value == USB_DEVICE_REMOTE_WAKEUP as i32 { dev.remote_wakeup = 1; ret = 0; }
        }
        x if x == (INTERFACE_REQUEST as i32 | (USB_REQ_GET_INTERFACE as i32) << 8) => {
            if index < 0 || index >= dev.ninterfaces { return -1; }
            data[0] = dev.altsetting[index as usize] as u8;
            p.actual_length = 1;
            ret = 0;
        }
        x if x == (INTERFACE_OUT_REQUEST as i32 | (USB_REQ_SET_INTERFACE as i32) << 8) => {
            dev.altsetting[index as usize] = value;
            ret = 0;
        }
        _ => {}
    }
    let _ = length;
    ret
}

pub fn usb_desc_init(dev: &mut UsbDevice) {
    dev.speed = USB_SPEED_FULL as i32;
    dev.speedmask = USB_SPEED_MASK_FULL as i32;
    dev.configuration = 0;
    dev.ninterfaces = 0;
    usb_ep_init(dev);
}

pub fn usb_desc_attach(dev: &mut UsbDevice) { usb_desc_init(dev); }

// =====================================================================
// 15.  HID key translation tables
// =====================================================================

/// QEMU keycode -> HID usage table translation.  Indices 0..0x80 are
/// "first half" keys, 0x80+ are second-half scancodes (e0/e1 sequences).
pub const HID_USAGE_KEYS: [u8; 256] = [
    0x00, 0x29, 0x1e, 0x1f, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x2d, 0x2e, 0x2a, 0x2b,
    0x14, 0x1a, 0x08, 0x15, 0x17, 0x1c, 0x18, 0x0c, 0x12, 0x13, 0x2f, 0x30, 0x28, 0xe0, 0x04, 0x16,
    0x07, 0x09, 0x0a, 0x0b, 0x0d, 0x0e, 0x0f, 0x33, 0x34, 0x35, 0xe1, 0x31, 0x1d, 0x1b, 0x06, 0x19,
    0x05, 0x11, 0x10, 0x36, 0x37, 0x38, 0xe5, 0x55, 0xe2, 0x2c, 0x39, 0x3a, 0x3b, 0x3c, 0x3d, 0x3e,
    0x3f, 0x40, 0x41, 0x42, 0x43, 0x53, 0x47, 0x5f, 0x60, 0x61, 0x56, 0x5c, 0x5d, 0x5e, 0x57, 0x59,
    0x5a, 0x5b, 0x62, 0x63, 0x46, 0x00, 0x64, 0x44, 0x45, 0x68, 0x69, 0x6a, 0x6b, 0x6c, 0x6d, 0x6e,
    0xe8, 0xe9, 0x71, 0x72, 0x73, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x85, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xe3, 0xe7, 0x65,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x58, 0xe4, 0x00, 0x00, 0x7f,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x81, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x54, 0x00, 0x46, 0xe6, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x48, 0x48, 0x4a, 0x52, 0x4b, 0x00,
    0x50, 0x00, 0x4f, 0x00, 0x4d, 0x51, 0x4e, 0x49, 0x4c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0xe3, 0xe7, 0x65, 0x66, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

pub fn hid_has_events(hs: &HidState) -> bool { hs.n > 0 }

pub fn hid_set_next_idle(_hs: &mut HidState) {}

pub fn hid_pointer_event(hs: &mut HidState, evt: &InputEvent) {
    static BMAP: [u8; INPUT_BUTTON__MAX] = [0x01, 0x04, 0x02, 0x00, 0x00, 0x00];
    if (hs.n as usize) >= QUEUE_LENGTH { return; }
    let slot = ((hs.head + hs.n) as usize) & QUEUE_MASK;
    let e = &mut hs.ptr.queue[slot];
    match evt.kind {
        x if x == INPUT_EVENT_KIND_REL => {
            if evt.rel.axis == INPUT_AXIS_X { e.xdx += evt.rel.value; }
            else if evt.rel.axis == INPUT_AXIS_Y { e.ydy += evt.rel.value; }
        }
        x if x == INPUT_EVENT_KIND_ABS => {
            if evt.abs.axis == INPUT_AXIS_X { e.xdx = evt.abs.value; }
            else if evt.abs.axis == INPUT_AXIS_Y { e.ydy = evt.abs.value; }
        }
        x if x == INPUT_EVENT_KIND_BTN => {
            let b = BMAP.get(evt.btn.button as usize).copied().unwrap_or(0);
            if evt.btn.down {
                e.buttons_state |= b;
                if evt.btn.button == INPUT_BUTTON_WHEEL_UP { e.dz -= 1; }
                else if evt.btn.button == INPUT_BUTTON_WHEEL_DOWN { e.dz += 1; }
            } else {
                e.buttons_state &= !b;
            }
        }
        _ => {}
    }
}

pub fn hid_pointer_sync(hs: &mut HidState) {
    if (hs.n as usize) >= QUEUE_LENGTH - 1 { return; }
    let prev = if hs.n > 0 { Some((hs.head + hs.n - 1) as usize & QUEUE_MASK) } else { None };
    let curr = ((hs.head + hs.n) as usize) & QUEUE_MASK;
    let next = ((hs.head + hs.n + 1) as usize) & QUEUE_MASK;
    let event_compression = prev.map(|p| hs.ptr.queue[p].buttons_state == hs.ptr.queue[curr].buttons_state).unwrap_or(false);
    if event_compression {
        if hs.kind == HID_MOUSE {
            hs.ptr.queue[prev.unwrap()].xdx += hs.ptr.queue[curr].xdx;
            hs.ptr.queue[curr].xdx = 0;
            hs.ptr.queue[prev.unwrap()].ydy += hs.ptr.queue[curr].ydy;
            hs.ptr.queue[curr].ydy = 0;
        } else {
            hs.ptr.queue[prev.unwrap()].xdx = hs.ptr.queue[curr].xdx;
            hs.ptr.queue[prev.unwrap()].ydy = hs.ptr.queue[curr].ydy;
        }
        hs.ptr.queue[prev.unwrap()].dz += hs.ptr.queue[curr].dz;
        hs.ptr.queue[curr].dz = 0;
    } else {
        if hs.kind == HID_MOUSE {
            hs.ptr.queue[next].xdx = 0; hs.ptr.queue[next].ydy = 0;
        } else {
            hs.ptr.queue[next].xdx = hs.ptr.queue[curr].xdx;
            hs.ptr.queue[next].ydy = hs.ptr.queue[curr].ydy;
        }
        hs.ptr.queue[next].dz = 0;
        hs.ptr.queue[next].buttons_state = hs.ptr.queue[curr].buttons_state;
        hs.n += 1;
        if let Some(cb) = hs.event { cb(hs); }
    }
}

pub fn hid_keyboard_event(hs: &mut HidState, evt: &InputEvent) {
    let mut scancodes = [0i32; 3];
    let count = qemu_input_key_value_to_scancode(&evt.key.key, evt.key.down, &mut scancodes);
    if (hs.n as usize) + count as usize > QUEUE_LENGTH { return; }
    for i in 0..count as usize {
        let slot = ((hs.head + hs.n) as usize) & QUEUE_MASK;
        hs.n += 1;
        hs.kbd.keycodes[slot] = scancodes[i];
    }
    if let Some(cb) = hs.event { cb(hs); }
}

pub fn hid_keyboard_process_keycode(hs: &mut HidState) {
    if hs.n == 0 { return; }
    let slot = (hs.head as usize) & QUEUE_MASK;
    hs.head = (hs.head + 1) & (QUEUE_LENGTH as u32 - 1);
    hs.n -= 1;
    let keycode = hs.kbd.keycodes[slot];
    let key = (keycode & 0x7f) as u8;
    let index = (key as u32) | ((hs.kbd.modifiers & (1 << 8)) >> 1);
    let hid_code = HID_USAGE_KEYS[index as usize & 0xff];
    hs.kbd.modifiers &= !(1u32 << 8);
    match hid_code {
        0x00 => return,
        0xe0 => {
            if key == 0x1d && (hs.kbd.modifiers & (1 << 9)) != 0 {
                hs.kbd.modifiers ^= (1 << 8) | (1 << 9);
                return;
            }
        }
        0xe1..=0xe7 => {
            if keycode & 0x80 != 0 { hs.kbd.modifiers &= !(1 << (hid_code & 0x0f)); return; }
        }
        0xe8..=0xe9 => {
            hs.kbd.modifiers |= 1 << (hid_code & 0x0f);
            return;
        }
        _ => {}
    }
    if keycode & 0x80 != 0 {
        let mut found = -1i32;
        for i in (0..hs.kbd.keys as usize).rev() {
            if hs.kbd.key[i] == hid_code { found = i as i32; break; }
        }
        if found < 0 { return; }
        let idx = found as usize;
        let k = hs.kbd.keys as usize;
        hs.kbd.key[idx] = hs.kbd.key[k - 1];
        hs.kbd.key[k - 1] = 0;
        hs.kbd.keys -= 1;
    } else {
        let mut found = -1i32;
        for i in (0..hs.kbd.keys as usize).rev() {
            if hs.kbd.key[i] == hid_code { found = i as i32; break; }
        }
        if found < 0 {
            if (hs.kbd.keys as usize) < hs.kbd.key.len() {
                hs.kbd.key[hs.kbd.keys as usize] = hid_code;
                hs.kbd.keys += 1;
            }
        }
    }
}

#[inline]
fn int_clamp(v: i32, lo: i32, hi: i32) -> i32 { v.max(lo).min(hi) }

pub fn hid_pointer_activate(hs: &mut HidState) {
    if hs.ptr.mouse_grabbed == 0 { hs.ptr.mouse_grabbed = 1; }
}

pub fn hid_pointer_poll(hs: &mut HidState, buf: &mut [u8]) -> i32 {
    hid_pointer_activate(hs);
    let index = if hs.n != 0 { hs.head } else { hs.head.wrapping_sub(1) } as usize & QUEUE_MASK;
    let mut e = hs.ptr.queue[index];
    let (mut dx, mut dy);
    if hs.kind == HID_MOUSE {
        dx = int_clamp(e.xdx, -127, 127);
        dy = int_clamp(e.ydy, -127, 127);
        e.xdx -= dx; e.ydy -= dy;
        hs.ptr.queue[index] = e;
    } else {
        dx = e.xdx; dy = e.ydy;
    }
    let dz = int_clamp(e.dz, -127, 127);
    e.dz -= dz;
    hs.ptr.queue[index] = e;
    if hs.n != 0 && e.dz == 0 && (hs.kind == HID_TABLET || (e.xdx == 0 && e.ydy == 0)) {
        hs.head = (hs.head + 1) & (QUEUE_LENGTH as u32 - 1);
        hs.n -= 1;
    }
    let dz_out = -dz;
    let mut l = 0;
    match hs.kind {
        x if x == HID_MOUSE => {
            if buf.len() > l { buf[l] = e.buttons_state; l += 1; }
            if buf.len() > l { buf[l] = dx as u8; l += 1; }
            if buf.len() > l { buf[l] = dy as u8; l += 1; }
            if buf.len() > l { buf[l] = dz_out as u8; l += 1; }
        }
        x if x == HID_TABLET => {
            if buf.len() > l { buf[l] = e.buttons_state; l += 1; }
            if buf.len() > l { buf[l] = (dx & 0xff) as u8; l += 1; }
            if buf.len() > l { buf[l] = (dx >> 8) as u8; l += 1; }
            if buf.len() > l { buf[l] = (dy & 0xff) as u8; l += 1; }
            if buf.len() > l { buf[l] = (dy >> 8) as u8; l += 1; }
            if buf.len() > l { buf[l] = dz_out as u8; l += 1; }
        }
        _ => {}
    }
    l as i32
}

pub fn hid_keyboard_poll(hs: &mut HidState, buf: &mut [u8]) -> i32 {
    if buf.len() < 2 { return 0; }
    hid_keyboard_process_keycode(hs);
    buf[0] = (hs.kbd.modifiers & 0xff) as u8;
    buf[1] = 0;
    if hs.kbd.keys > 6 {
        for b in buf[2..].iter_mut() { *b = HID_USAGE_ERROR_ROLLOVER; }
    } else {
        let n = std::cmp::min(8, buf.len()) - 2;
        buf[2..2 + n].copy_from_slice(&hs.kbd.key[..n]);
    }
    std::cmp::min(8, buf.len()) as i32
}

pub fn hid_keyboard_write(_hs: &mut HidState, _buf: &[u8]) -> i32 { 0 }

pub fn hid_reset(hs: &mut HidState) {
    match hs.kind {
        x if x == HID_KEYBOARD => {
            hs.kbd.keycodes = [0; QUEUE_LENGTH];
            hs.kbd.key = [0; 8];
            hs.kbd.keys = 0;
            hs.kbd.modifiers = 0;
        }
        x if x == HID_MOUSE || x == HID_TABLET => {
            hs.ptr.queue = [HidPointerEvent::default(); QUEUE_LENGTH];
        }
        _ => {}
    }
    hs.head = 0; hs.n = 0; hs.protocol = 1; hs.idle = 0;
}

pub fn hid_free(_hs: &mut HidState) {}

pub fn hid_init(hs: &mut HidState, kind: i32, event: HidEventFunc) {
    hs.kind = kind;
    hs.event = Some(event);
    if kind == HID_KEYBOARD {
        hs.kbd.keycodes = [0; QUEUE_LENGTH];
    } else if kind == HID_MOUSE || kind == HID_TABLET {
        hs.ptr.queue = [HidPointerEvent::default(); QUEUE_LENGTH];
    }
}

// =====================================================================
// 16.  QEMU keymap
// =====================================================================

/// QKeyCode -> scancode/qnum map (subset used by PS2 input).
pub fn qemu_input_qcode_to_number(qc: QKeyCode) -> u16 {
    use QKeyCode::*;
    match qc {
        _0 => 0xb, _1 => 0x2, _2 => 0x3, _3 => 0x4, _4 => 0x5, _5 => 0x6, _6 => 0x7, _7 => 0x8, _8 => 0x9, _9 => 0xa,
        A => 0x1e, B => 0x30, C => 0x2e, D => 0x20, E => 0x12, F => 0x21, G => 0x22, H => 0x23, I => 0x17,
        J => 0x24, K => 0x25, L => 0x26, M => 0x32, N => 0x31, O => 0x18, P => 0x19, Q => 0x10, R => 0x13,
        S => 0x1f, T => 0x14, U => 0x16, V => 0x2f, W => 0x11, X => 0x2d, Y => 0x15, Z => 0x2c,
        AcBack => 0xea, AcBookmarks => 0xe6, AcForward => 0xe9, AcHome => 0xb2, AcRefresh => 0xe7,
        Again => 0x85, Alt => 0x38, AltR => 0xb8, Apostrophe => 0x28, Asterisk => 0x37,
        AudioMute => 0xa0, AudioNext => 0x99, AudioPlay => 0xa2, AudioPrev => 0x90, AudioStop => 0xa4,
        Backslash => 0x2b, Backspace => 0xe, BracketLeft => 0x1a, BracketRight => 0x1b,
        Calculator => 0xa1, CapsLock => 0x3a, Comma => 0x33, Compose => 0xdd, Computer => 0xeb,
        Copy => 0xf8, Ctrl => 0x1d, CtrlR => 0x9d, Cut => 0xbc,
        Delete => 0xd3, Dot => 0x34, Down => 0xd0, End => 0xcf, Equal => 0xd, Esc => 0x1,
        F1 => 0x3b, F2 => 0x3c, F3 => 0x3d, F4 => 0x3e, F5 => 0x3f, F6 => 0x40, F7 => 0x41, F8 => 0x42,
        F9 => 0x43, F10 => 0x44, F11 => 0x57, F12 => 0x58, Find => 0xc1, Front => 0x8c,
        GraveAccent => 0x29, Help => 0xf5, Henkan => 0x79, Hiragana => 0x77, Home => 0xc7, Insert => 0xd2,
        KatakanaHiragana => 0x70,
        Kp0 => 0x52, Kp1 => 0x4f, Kp2 => 0x50, Kp3 => 0x51, Kp4 => 0x4b, Kp5 => 0x4c, Kp6 => 0x4d,
        Kp7 => 0x47, Kp8 => 0x48, Kp9 => 0x49, KpAdd => 0x4e, KpComma => 0x7e, KpDecimal => 0x53,
        KpDivide => 0xb5, KpEnter => 0x9c, KpEquals => 0x59, KpMultiply => 0x37, KpSubtract => 0x4a,
        Left => 0xcb, Less => 0x56, Lf => 0x5b, Mail => 0xec, MediaSelect => 0xed, Menu => 0x9e,
        MetaL => 0xdb, MetaR => 0xdc, Minus => 0xc, Muhenkan => 0x7b, NumLock => 0x45, Open => 0x64,
        Paste => 0x65, Pause => 0xc6, PgDn => 0xd1, PgUp => 0xc9, Power => 0xde, Print => 0x54,
        Props => 0x86, Ret => 0x1c, Right => 0xcd, Ro => 0x73, ScrollLock => 0x46, Semicolon => 0x27,
        Shift => 0x2a, ShiftR => 0x36, Slash => 0x35, Sleep => 0xdf, Spc => 0x39, Stop => 0xe8,
        Sysrq => 0x54, Tab => 0xf, Undo => 0x87, Up => 0xc8, VolumeDown => 0xae, VolumeUp => 0xb0,
        Wake => 0xe3, Yen => 0x7d,
    }
}

pub fn qemu_input_key_value_to_number(kv: &KeyValue) -> i32 {
    match kv.kind {
        KeyValueKind::Qcode => qemu_input_qcode_to_number(kv.qcode) as i32,
        KeyValueKind::Number => kv.number,
    }
}

pub fn qemu_input_key_value_to_scancode(kv: &KeyValue, down: bool, codes: &mut [i32; 3]) -> i32 {
    let mut keycode = qemu_input_key_value_to_number(kv);
    let mut count: usize = 0;
    if kv.kind == KeyValueKind::Qcode && kv.qcode == QKeyCode::Pause {
        let v: i32 = if down { 0 } else { 0x80 };
        codes[count] = 0xe1; count += 1;
        codes[count] = 0x1d | v; count += 1;
        codes[count] = 0x45 | v; count += 1;
        return count as i32;
    }
    if keycode & SCANCODE_GREY != 0 {
        codes[count] = SCANCODE_EMUL0; count += 1;
        keycode &= !SCANCODE_GREY;
    }
    if !down { keycode |= SCANCODE_UP; }
    codes[count] = keycode; count += 1;
    count as i32
}

// =====================================================================
// 17.  OHCI controller — create / reset / read / write / frame
// =====================================================================

/// Construct a fresh OHCI controller with `ports` root-hub ports.
pub fn ohci_create(base: u32, ports: i32) -> OhciState {
    let mut ohci = OhciState::default();
    ohci.mem_base = base;
    ohci.num_ports = ports as u32;
    let ticks_per_sec = PSXCLK as u64;
    ohci.frame_number = 0;
    let _ = ticks_per_sec;
    for i in 0..ports as usize {
        ohci.rhport[i].port.speedmask = (USB_SPEED_MASK_LOW as i32) | (USB_SPEED_MASK_FULL as i32);
        ohci.rhport[i].port.index = i as i32;
    }
    ohci_hard_reset(&mut ohci);
    ohci
}

pub fn ohci_soft_reset(ohci: &mut OhciState) {
    ohci.ctl = (ohci.ctl & OHCI_CTL_IR) | OHCI_USB_SUSPEND;
    ohci.old_ctl = 0;
    ohci.status = 0;
    ohci.intr_status = 0;
    ohci.intr = OHCI_INTR_MIE;
    ohci.hcca = 0;
    ohci.ctrl_head = 0; ohci.ctrl_cur = 0;
    ohci.bulk_head = 0; ohci.bulk_cur = 0;
    ohci.per_cur = 0; ohci.done = 0;
    ohci.done_count = 7;
    ohci.fsmps = 0x2778;
    ohci.fi = 0x2edf; ohci.fit = 0; ohci.frt = 0;
    ohci.frame_number = 0;
    ohci.pstart = 0;
    ohci.lst = OHCI_LS_THRESH;
    ohci.eof_timer = 0;
}

pub fn ohci_hard_reset(ohci: &mut OhciState) {
    ohci_soft_reset(ohci);
    ohci.ctl = 0;
    ohci.rhdesc_a = OHCI_RHA_NPS | ohci.num_ports;
    ohci.rhdesc_b = 0;
    ohci.rhstatus = 0;
    for p in ohci.rhport.iter_mut() { p.ctrl = 0; }
}

pub fn ohci_bus_start(ohci: &mut OhciState) -> i32 {
    ohci.eof_timer = 0;
    ohci.sof_time = usb_get_clock();
    ohci_set_interrupt(ohci, OHCI_INTR_SF);
    1
}

pub fn ohci_bus_stop(ohci: &mut OhciState) {
    if ohci.eof_timer != 0 { ohci.eof_timer = 0; }
}

pub fn ohci_frame_boundary(ohci: &mut OhciState) {
    if ohci.intr_status & OHCI_INTR_UE != 0 { return; }
    ohci.frt = ohci.fit;
    ohci.frame_number = (ohci.frame_number + 1) & 0xffff;
    if ohci.done_count == 0 && (ohci.intr_status & OHCI_INTR_WD) == 0 {
        if ohci.done == 0 { return; }
        ohci.intr_status |= OHCI_INTR_WD;
    }
    if ohci.done_count != 7 && ohci.done_count != 0 { ohci.done_count -= 1; }
    ohci.sof_time = usb_get_clock();
    ohci.eof_timer = unsafe { USB_REG.frame_time } as u64;
    ohci_set_interrupt(ohci, OHCI_INTR_SF);
}

pub fn usb_get_clock() -> i64 { unsafe { USB_REG.clocks } }
pub fn usb_get_ticks_per_second() -> i32 { PSXCLK }

pub fn ohci_set_interrupt(ohci: &mut OhciState, intr: u32) {
    ohci.intr_status |= intr;
    if (ohci.intr & OHCI_INTR_MIE) != 0 && (ohci.intr_status & ohci.intr) != 0 {
        unsafe { USB_REG.last_cycle = USB_REG.clocks; }
    }
}

pub fn ohci_port_power(ohci: &mut OhciState, i: usize, p: bool) {
    if p { ohci.rhport[i].ctrl |= OHCI_PORT_PPS; }
    else {
        ohci.rhport[i].ctrl &= !(OHCI_PORT_PPS | OHCI_PORT_CCS | OHCI_PORT_PSS | OHCI_PORT_PRS);
    }
}

pub fn ohci_set_hub_status(ohci: &mut OhciState, val: u32) {
    if val & OHCI_RHS_OCIC != 0 { ohci.rhstatus &= !OHCI_RHS_OCIC; }
    if val & OHCI_RHS_LPS != 0 {
        for i in 0..ohci.num_ports as usize { ohci_port_power(ohci, i, false); }
    }
    if val & OHCI_RHS_LPSC != 0 {
        for i in 0..ohci.num_ports as usize { ohci_port_power(ohci, i, true); }
    }
    if val & OHCI_RHS_DRWE != 0 { ohci.rhstatus |= OHCI_RHS_DRWE; }
    if val & OHCI_RHS_CRWE != 0 { ohci.rhstatus &= !OHCI_RHS_DRWE; }
}

pub fn ohci_port_set_status(ohci: &mut OhciState, portnum: usize, val: u32) {
    if val & OHCI_PORT_WTC != 0 { ohci.rhport[portnum].ctrl &= !(val & OHCI_PORT_WTC); }
    if val & OHCI_PORT_CCS != 0 { ohci.rhport[portnum].ctrl &= !OHCI_PORT_PES; }
    if val & OHCI_PORT_PRS != 0 {
        ohci.rhport[portnum].ctrl &= !OHCI_PORT_PRS;
        ohci.rhport[portnum].ctrl |= OHCI_PORT_PES | OHCI_PORT_PRSC;
    }
    if val & OHCI_PORT_LSDA != 0 { ohci_port_power(ohci, portnum, false); }
    if val & OHCI_PORT_PPS != 0 { ohci_port_power(ohci, portnum, true); }
    ohci_set_interrupt(ohci, OHCI_INTR_RHSC);
}

pub fn ohci_set_ctl(ohci: &mut OhciState, val: u32) {
    let old = ohci.ctl & OHCI_CTL_HCFS;
    ohci.ctl = val;
    let new = ohci.ctl & OHCI_CTL_HCFS;
    if old == new { return; }
    match new {
        OHCI_USB_OPERATIONAL => { ohci_bus_start(ohci); }
        OHCI_USB_SUSPEND => {
            ohci_bus_stop(ohci);
            ohci.intr_status &= !OHCI_INTR_SF;
        }
        OHCI_USB_RESUME => {}
        OHCI_USB_RESET => { ohci_hard_reset(ohci); }
        _ => {}
    }
}

pub fn ohci_mem_read_impl(ohci: &mut OhciState, addr: u32) -> u32 {
    let addr = addr.wrapping_sub(ohci.mem_base);
    if addr & 3 != 0 { return 0xffff_ffff; }
    if addr >= 0x54 && addr < 0x54 + ohci.num_ports * 4 {
        return ohci.rhport[((addr - 0x54) >> 2) as usize].ctrl | OHCI_PORT_PPS;
    }
    match addr >> 2 {
        0 => 0x10,                                   // HcRevision
        1 => ohci.ctl,                               // HcControl
        2 => ohci.status,                            // HcCommandStatus
        3 => ohci.intr_status,                       // HcInterruptStatus
        4 | 5 => ohci.intr,                          // HcInterruptEnable / Disable
        6 => ohci.hcca,                              // HcHCCA
        7 => ohci.per_cur,                           // HcPeriodCurrentED
        8 => ohci.ctrl_head,                         // HcControlHeadED
        9 => ohci.ctrl_cur,                          // HcControlCurrentED
        10 => ohci.bulk_head,                        // HcBulkHeadED
        11 => ohci.bulk_cur,                         // HcBulkCurrentED
        12 => ohci.done,                             // HcDoneHead
        13 => (ohci.fit << 31) | (ohci.fsmps << 16) | ohci.fi, // HcFmInterval
        14 => (ohci.frt << 31) | (ohci.fi as u32),  // HcFmRemaining
        15 => ohci.frame_number as u32,              // HcFmNumber
        16 => ohci.pstart,                           // HcPeriodicStart
        17 => ohci.lst,                              // HcLSThreshold
        18 => ohci.rhdesc_a,                         // HcRhDescriptorA
        19 => ohci.rhdesc_b,                         // HcRhDescriptorB
        20 => ohci.rhstatus,                         // HcRhStatus
        _ => 0xffff_ffff,
    }
}

pub fn ohci_mem_write_impl(ohci: &mut OhciState, addr: u32, val: u32) {
    let addr = addr.wrapping_sub(ohci.mem_base);
    if addr & 3 != 0 { return; }
    if addr >= 0x54 && addr < 0x54 + ohci.num_ports * 4 {
        ohci_port_set_status(ohci, ((addr - 0x54) >> 2) as usize, val);
        return;
    }
    match addr >> 2 {
        1 => ohci_set_ctl(ohci, val),
        2 => {
            let v = val & !OHCI_STATUS_SOC;
            ohci.status |= v;
            if ohci.status & OHCI_STATUS_HCR != 0 { ohci_soft_reset(ohci); }
        }
        3 => ohci.intr_status &= !val,
        4 => ohci.intr |= val,
        5 => ohci.intr &= !val,
        6 => ohci.hcca = val & OHCI_HCCA_MASK,
        8 => ohci.ctrl_head = val & OHCI_EDPTR_MASK,
        9 => ohci.ctrl_cur = val & OHCI_EDPTR_MASK,
        10 => ohci.bulk_head = val & OHCI_EDPTR_MASK,
        11 => ohci.bulk_cur = val & OHCI_EDPTR_MASK,
        13 => {
            ohci.fsmps = (val & OHCI_FMI_FSMPS) >> 16;
            ohci.fit = (val & OHCI_FMI_FIT) >> 31;
            ohci.fi = val & OHCI_FMI_FI;
        }
        16 => ohci.pstart = val & 0xffff,
        17 => ohci.lst = val & 0xffff,
        18 => {
            ohci.rhdesc_a &= !OHCI_RHA_RW_MASK;
            ohci.rhdesc_a |= val & OHCI_RHA_RW_MASK;
        }
        19 => {}
        20 => ohci_set_hub_status(ohci, val),
        _ => {}
    }
}

// =====================================================================
// 18.  Default trait and module-level re-exports
// =====================================================================

impl Default for UsbReg {
    fn default() -> Self { Self::new() }
}

impl Default for UsbBus {
    fn default() -> Self {
        Self {
            ops: None,
            busnr: 0,
            nfree: 0,
            nused: 0,
            free: Vec::new(),
            used: Vec::new(),
            next: None,
            prev: None,
        }
    }
}

impl Default for UsbPort {
    fn default() -> Self {
        Self {
            dev: None,
            speedmask: (USB_SPEED_MASK_LOW | USB_SPEED_MASK_FULL) as i32,
            ops: None,
            opaque: None,
            index: 0,
            next: None,
            prev: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usb_init_shutdown_roundtrip() {
        usb_init();
        usb_reset();
        usb_shutdown();
    }

    #[test]
    fn desc_device_roundtrip() {
        let id = UsbDescId { id_vendor: 0x1234, id_product: 0x5678, bcd_device: 0x0100, ..Default::default() };
        let dev = UsbDescDevice { bcd_usb: 0x0200, b_device_class: USB_CLASS_HID,
            b_device_sub_class: 0, b_device_protocol: 0, b_max_packet_size0: 8,
            b_num_configurations: 1, ..Default::default() };
        let mut buf = [0u8; 32];
        let n = usb_desc_device(&id, &dev, false, &mut buf);
        assert_eq!(n, 18);
        assert_eq!(buf[0], 0x12);
        assert_eq!(buf[1], USB_DT_DEVICE);
    }

    #[test]
    fn qcode_translation() {
        assert_eq!(qemu_input_qcode_to_number(QKeyCode::A), 0x1e);
        assert_eq!(qemu_input_qcode_to_number(QKeyCode::Esc), 0x1);
    }
}
