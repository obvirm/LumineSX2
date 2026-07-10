//! PCSX2 SIO subsystem translation (Sio0, Sio2, Multitap, Memory Card).
//!
//! Single-file idiomatic Rust translation of the PlayStation 2 Serial I/O bus
//! components from PCSX2, including the Sio0 (PS1-style) and Sio2 (PS2
//! multi-protocol) controllers, the Multitap protocol that lets a single
//! physical port fan out to four pads, and the Memory Card protocol with both
//! PS1 and PS2 command handling. Cross-cutting bus state (the 2x4 memcard slot
//! array, the active memcard pointer, the Sio2 FIFOs, the multitap instances
//! and the memory-card protocol, plus the Sio0 STAT register that the PS1
//! command path writes back into) is kept in a lazily-initialised
//! [`RefCell`](std::cell::RefCell) that mirrors the C++ globals without
//! leaking them into the public type signatures. The 1-3-7 multiplex refers
//! to the pad-address layout: one direct pad + three multitap slots per
//! physical port, across two physical ports (0 and 1).
//!
//! Non-`std` PCSX2 dependencies (the Pad subsystem, file-backed memcard I/O,
//! host UI, the state wrapper, logging and the IOP interrupt controller) are
//! stubbed with trait-bounded helpers so this module compiles in isolation.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

// =========================================================================
// Constants and enums (translated from SioTypes.h)
// =========================================================================

/// SioMode byte values used in the SIO bus address.
pub mod sio_mode {
    pub const NOT_SET: u8 = 0x00;
    pub const PAD: u8 = 0x01;
    pub const MULTITAP: u8 = 0x21;
    pub const INFRARED: u8 = 0x61;
    pub const MEMCARD: u8 = 0x81;
}

/// Memory card protocol command bytes.
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

/// Top-level SIO topology constants.
pub mod sio {
    pub const PORTS: usize = 2;
    pub const SLOTS: usize = 4;
}

/// SIO0 STAT register bits.
pub mod sio0_stat {
    pub const TX_READY: u32 = 0x01;
    pub const RX_FIFO_NOT_EMPTY: u32 = 0x02;
    pub const TX_EMPTY: u32 = 0x04;
    pub const RX_PARITY_ERROR: u32 = 0x08;
    pub const ACK: u32 = 0x80;
    pub const IRQ: u32 = 0x0200;
}

/// SIO0 CTRL register bits.
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

/// SIO2 command-descriptor fields.
pub mod sio2_cmd {
    pub const PORT: u32 = 0x01;
    pub const COMMAND_LENGTH_MASK: u16 = 0x3ff;
}

/// SIO2 CTRL register bits.
pub mod sio2_ctrl {
    pub const START_TRANSFER: u32 = 0x1;
    pub const RESET: u32 = 0xc;
    pub const PORT: u32 = 0x2000;
    /// The value which SIO2MAN resets SIO2_CTRL to after a system reset.
    pub const SIO2MAN_RESET: u32 = 0x0000_03bc;
}

/// CMD_STAT bits reported to the IOP after a SIO2 transfer.
pub mod cmd_stat {
    // Deprecated
    pub const DISCONNECTED: u32 = 0x0001_d100;
    // Deprecated
    pub const CONNECTED: u32 = 0x0000_1100;

    pub const NO_DEVICES_MISSING: u32 = 0x1000;
    pub const PORT_1_MISSING: u32 = 0x0001_d000;
    pub const PORT_2_MISSING: u32 = 0x0002_d000;
    pub const BOTH_PORTS_MISSING: u32 = 0x0003_d000;
    pub const ONE_PORT_OPEN: u32 = 0x100;
    pub const TWO_PORTS_OPEN: u32 = 0x200;
}

/// PORT_STAT register value.
pub mod port_stat {
    pub const DEFAULT: u32 = 0xf;
}

/// FIFO_STAT register values. The C++ namespace documents most of these as
/// mysterious / largely unused, but they are preserved here verbatim.
pub mod fifo_stat {
    pub const DEFAULT: u32 = 0x0;
    /// Set when getting memcard specs.
    pub const SPECS: u32 = 0x83;
    /// Set when getting or setting the terminator byte.
    pub const TERMINATOR: u32 = 0x8b;
    /// Set when setting the read/write sector.
    pub const READ_WRITE_END: u32 = 0x8c;
}

/// Memcard terminator values.
pub mod terminator {
    pub const NOT_READY: u32 = 0x66;
    pub const READY: u32 = 0x55;
}

/// Drive the SIO bus through its three top-level stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SioStage {
    Idle,
    WaitingCommand,
    Working,
}

/// Interrupt reasons dispatched by [`Sio0::interrupt`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sio0Interrupt {
    TestEvent,
    StatRead,
    TxDataWrite,
}

/// Multitap sub-protocol selector byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MultitapMode {
    NotSet = 0xff,
    PadSupportCheck = 0x12,
    MemcardSupportCheck = 0x13,
    SelectPad = 0x21,
    SelectMemcard = 0x22,
}

// =========================================================================
// Memcard slot state (translated from `_mcd` in Sio.h)
// =========================================================================

/// Mirrors the `McdSizeInfo` struct used by the file-backed memcard I/O.
#[derive(Debug, Clone, Default)]
pub struct McdSizeInfo {
    pub sector_size: u32,
    pub erase_block_size_in_sectors: u32,
    pub mcd_size_in_sectors: u32,
    pub xor: u8,
}

/// Per-slot memory card state, translated from `_mcd` in `Sio.h`.
///
/// All file-backed operations are stubbed; the shape of the API matches the
/// C++ struct so that a real backing store can be dropped in later.
pub struct Memcard {
    pub current_command: u8,
    /// Terminator value, e.g. [`terminator::READY`].
    pub term: u8,
    /// XOR sector check, set by `SetSector` and `RecalculatePS1Addr`.
    pub good_sector: bool,
    pub msb: u8,
    pub lsb: u8,
    pub sector_addr: u32,
    pub transfer_addr: u32,
    pub buf: Vec<u8>,
    /// PS1 flag byte. Bit 3 (mask `0x08`) is the "directory unread" flag.
    pub flag: u8,
    pub port: u8,
    pub slot: u8,
    pub auto_eject_ticks: u32,
}

impl Memcard {
    /// Construct a slot for `(port, slot)`, matching the C++ `Initialize`
    /// defaults: term = `0x55`, flag = `0x08`, autoEjectTicks = 0.
    pub fn new(port: u8, slot: u8) -> Self {
        Self {
            current_command: 0,
            term: 0x55,
            good_sector: false,
            msb: 0,
            lsb: 0,
            sector_addr: 0,
            transfer_addr: 0,
            buf: Vec::new(),
            flag: 0x08,
            port,
            slot,
            auto_eject_ticks: 0,
        }
    }

    /// Stub for `FileMcd_GetSizeInfo`. Returns an empty `McdSizeInfo`.
    pub fn get_size_info(&self) -> McdSizeInfo {
        McdSizeInfo::default()
    }

    /// Stub for `FileMcd_IsPSX`.
    pub fn is_psx(&self) -> bool {
        false
    }

    /// Stub for `FileMcd_EraseBlock`.
    pub fn erase_block(&mut self) {}

    /// Stub for `FileMcd_Read`. `dest` is zeroed to match "no card present".
    pub fn read(&self, dest: &mut [u8]) {
        dest.fill(0);
    }

    /// Stub for `FileMcd_Save`. The source buffer is discarded.
    pub fn write(&mut self, _src: &[u8]) {}

    /// Stub for `FileMcd_IsPresent`. Returns `false` so the PS2 sees an
    /// empty slot, matching the C++ behaviour with no card file mounted.
    pub fn is_present(&self) -> bool {
        false
    }

    /// Fold `msb ^ lsb ^ buf[0] ^ buf[1] ^ ...`, as `DoXor()` does.
    pub fn do_xor(&self) -> u8 {
        let mut ret = self.msb ^ self.lsb;
        for &b in &self.buf {
            ret ^= b;
        }
        ret
    }

    /// Stub for `FileMcd_GetCRC`. Returns 0 when no file is mounted.
    pub fn get_checksum(&self) -> u64 {
        0
    }

    /// Stub for `FileMcd_NextFrame`.
    pub fn next_frame(&mut self) {}

    /// Stub for `FileMcd_ReIndex`. Returns `false` because no real file
    /// backing is attached.
    pub fn re_index(&mut self, _filter: &str) -> bool {
        false
    }
}

// =========================================================================
// Bus-shared state (mirrors the C++ globals: `mcds`, `mcd`, `g_Sio2FifoIn`,
// `g_Sio2FifoOut`, `g_MultitapArr`, `g_MemoryCardProtocol`,
// `sioLastFrameMcdBusy`, `EmuConfig.Pad.IsMultitapPortEnabled`)
// =========================================================================

/// Bus-shared state container. The bus holds a Sio0 mirror of the STAT
/// register so that the PS1 memcard command path can flip ACK without
/// needing a `&mut Sio0` reference at hand.
struct BusShared {
    /// 2 physical ports x 4 slots per port.
    mcds: [[Memcard; sio::SLOTS]; sio::PORTS],
    /// `(port, slot)` of the active memcard, matching the C++ `mcd` pointer.
    mcd_index: (u8, u8),
    fifo_in: VecDeque<u8>,
    fifo_out: VecDeque<u8>,
    multitap: [MultitapProtocol; sio::PORTS],
    memory_card: MemoryCardProtocol,
    /// `g_Sio2.port`.
    sio2_port: u8,
    /// `g_Sio2.commandLength`.
    sio2_command_length: usize,
    /// `g_Sio2.dmaBlockSize`.
    sio2_dma_block_size: usize,
    /// Pad-eject ticks, decremented per `pad()` call. Stub of
    /// `PadBase::ejectTicks`.
    pad_eject_ticks: [u32; sio::PORTS],
    /// Per-port "multitap enabled" config flag (stub of
    /// `EmuConfig.Pad.IsMultitapPortEnabled`). Defaults to `false`.
    multitap_enabled: [bool; sio::PORTS],
    /// Sio0 STAT mirror. The PS1 memcard path (`PS1Read`, `PS1Write`,
    /// `PS1Pocketstation`) calls `g_Sio0.SetAcknowledge()` which only
    /// touches the ACK bit, so a single u32 here is sufficient.
    sio0_stat: u32,
    /// Frame counter stub for `sioLastFrameMcdBusy`.
    last_frame_mcd_busy: u32,
}

impl BusShared {
    fn new() -> Self {
        let mcds = std::array::from_fn(|port| {
            std::array::from_fn(|slot| Memcard::new(port as u8, slot as u8))
        });
        Self {
            mcds,
            mcd_index: (0, 0),
            fifo_in: VecDeque::new(),
            fifo_out: VecDeque::new(),
            multitap: [MultitapProtocol::new(), MultitapProtocol::new()],
            memory_card: MemoryCardProtocol::new(),
            sio2_port: 0,
            sio2_command_length: 0,
            sio2_dma_block_size: 0,
            pad_eject_ticks: [0; sio::PORTS],
            multitap_enabled: [false; sio::PORTS],
            sio0_stat: 0,
            last_frame_mcd_busy: 0,
        }
    }

    fn mcd(&self) -> &Memcard {
        &self.mcds[self.mcd_index.0 as usize][self.mcd_index.1 as usize]
    }

    fn mcd_mut(&mut self) -> &mut Memcard {
        &mut self.mcds[self.mcd_index.0 as usize][self.mcd_index.1 as usize]
    }
}

fn bus() -> &'static Mutex<RefCell<BusShared>> {
    static BUS: OnceLock<Mutex<RefCell<BusShared>>> = OnceLock::new();
    BUS.get_or_init(|| Mutex::new(RefCell::new(BusShared::new())))
}

// =========================================================================
// Public address-conversion helpers (translated from Sio.cpp)
// =========================================================================

/// Convert a global pad index (`0..=7`) to `(port, slot)`.
///
/// The 1-3-7 multiplex layout is:
/// - `0` -> port 0, slot 0 (1A)
/// - `1` -> port 1, slot 0 (2A)
/// - `2..=4` -> port 0, slot `index - 1` (1B, 1C, 1D)
/// - `5..=7` -> port 1, slot `index - 4` (2B, 2C, 2D)
pub fn sio_convert_pad_to_port_and_slot(index: u32) -> (u32, u32) {
    if index > 4 {
        (1, index - 4)
    } else if index > 1 {
        (0, index - 1)
    } else {
        (index, 0)
    }
}

/// Convert `(port, slot)` back to a global pad index in `0..=7`.
pub fn sio_convert_port_and_slot_to_pad(port: u32, slot: u32) -> u32 {
    if slot == 0 {
        port
    } else if port == 0 {
        slot + 1
    } else {
        slot + 4
    }
}

/// Returns `true` when the given global pad index is a multitap slot.
pub fn sio_pad_is_multitap_slot(index: u32) -> bool {
    index >= 2
}

/// Returns `true` when the given `(port, slot)` is a multitap slot.
pub fn sio_port_and_slot_is_multitap(_port: u32, slot: u32) -> bool {
    slot != 0
}

/// Tick the auto-eject counter on every memcard slot, exactly once per frame.
pub fn sio_next_frame() {
    let binding = bus().lock().unwrap();
    let mut b = binding.borrow_mut();
    for port in 0..sio::PORTS {
        for slot in 0..sio::SLOTS {
            b.mcds[port][slot].next_frame();
        }
    }
}

/// Re-index every memcard against the given game serial.
pub fn sio_set_game_serial(serial: &str) {
    let binding = bus().lock().unwrap();
    let mut b = binding.borrow_mut();
    for port in 0..sio::PORTS {
        for slot in 0..sio::SLOTS {
            if b.mcds[port][slot].re_index(serial) {
                // Mirrors `AutoEject::Set` in the C++: schedule a reinsert.
                b.mcds[port][slot].auto_eject_ticks = 60;
                b.mcds[port][slot].term = terminator::NOT_READY as u8;
            }
        }
    }
}

// =========================================================================
// Sio0 (PS1-compatible controller)
// =========================================================================

/// State of the Sio0 register file, translated from the C++ `Sio0` class.
#[derive(Debug, Clone)]
pub struct Sio0 {
    tx_data: u32,
    rx_data: u32,
    stat: u32,
    mode: u16,
    ctrl: u16,
    baud: u16,
    pub flag: u8,
    pub sio_stage: SioStage,
    pub sio_mode: u8,
    pub sio_command: u8,
    pub pad_started: bool,
    pub rx_data_set: bool,
    pub port: u8,
    pub slot: u8,
}

impl Default for Sio0 {
    fn default() -> Self {
        Self {
            tx_data: 0,
            rx_data: 0,
            stat: 0,
            mode: 0,
            ctrl: 0,
            baud: 0,
            flag: 0,
            sio_stage: SioStage::Idle,
            sio_mode: sio_mode::NOT_SET,
            sio_command: 0,
            pad_started: false,
            rx_data_set: false,
            port: 0,
            slot: 0,
        }
    }
}

impl Sio0 {
    /// Default constructor, equivalent to the C++ `Sio0()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Initialise the Sio0 controller and the memcard slot array.
    ///
    /// Equivalent to `Sio0::Initialize()`. Returns `true` on success.
    pub fn init(&mut self) -> bool {
        self.soft_reset();
        self.port = 0;
        self.slot = 0;

        let binding = bus().lock().unwrap();
        let mut b = binding.borrow_mut();
        for port in 0..sio::PORTS {
            for slot in 0..sio::SLOTS {
                b.mcds[port][slot].term = 0x55;
                b.mcds[port][slot].port = port as u8;
                b.mcds[port][slot].slot = slot as u8;
                b.mcds[port][slot].flag = 0x08;
                b.mcds[port][slot].auto_eject_ticks = 0;
            }
        }
        b.mcd_index = (0, 0);
        b.memory_card.reset_ps1_state();
        b.sio0_stat = 0;
        true
    }

    /// Tear-down hook. Returns `true`; mirrors the C++ `Sio0::Shutdown()`.
    pub fn shutdown(&mut self) -> bool {
        true
    }

    /// Reset the Sio0 transaction state without touching the memcard array.
    pub fn soft_reset(&mut self) {
        self.pad_started = false;
        self.sio_mode = sio_mode::NOT_SET;
        self.sio_command = 0;
        self.sio_stage = SioStage::Idle;
        bus().lock().unwrap().borrow_mut().memory_card.reset_ps1_state();
    }

    /// Toggle the ACK line in the SIO0 STAT register.
    pub fn set_acknowledge(&mut self, ack: bool) {
        if ack {
            self.stat |= sio0_stat::ACK;
        } else {
            self.stat &= !sio0_stat::ACK;
        }
        // Mirror into the bus so PS1-protocol calls without an Sio0 handle
        // (i.e. those going through Sio2) still see the latest ACK state.
        bus().lock().unwrap().borrow_mut().sio0_stat = self.stat;
    }

    fn clear_stat_acknowledge(&mut self) {
        self.stat &= !sio0_stat::ACK;
        bus().lock().unwrap().borrow_mut().sio0_stat = self.stat;
    }

    /// Dispatch a SIO0 interrupt. The actual `iopIntcIrq` / `PSX_INT` calls
    /// are stubbed; the structural state changes are preserved.
    pub fn interrupt(&mut self, sio0_interrupt: Sio0Interrupt) {
        match sio0_interrupt {
            Sio0Interrupt::TestEvent => {
                // Stub: iopIntcIrq(7)
            }
            Sio0Interrupt::StatRead => {
                self.clear_stat_acknowledge();
            }
            Sio0Interrupt::TxDataWrite => {}
        }
        // Stub: PSX_INT(IopEvt_SIO, PSXCLK / 250000)
    }

    pub fn get_tx_data(&self) -> u8 {
        self.tx_data as u8
    }

    /// Pop the RX byte and update the FIFO-empty / TX-ready flags.
    pub fn get_rx_data(&mut self) -> u8 {
        let v = self.rx_data as u8;
        self.stat |= sio0_stat::TX_READY | sio0_stat::TX_EMPTY;
        self.stat &= !sio0_stat::RX_FIFO_NOT_EMPTY;
        bus().lock().unwrap().borrow_mut().sio0_stat = self.stat;
        v
    }

    /// Read STAT, which also fires [`Sio0Interrupt::StatRead`].
    pub fn get_stat(&mut self) -> u32 {
        let ret = self.stat;
        self.interrupt(Sio0Interrupt::StatRead);
        ret
    }

    pub fn get_mode(&self) -> u16 {
        self.mode
    }

    pub fn get_ctrl(&self) -> u16 {
        self.ctrl
    }

    pub fn get_baud(&self) -> u16 {
        self.baud
    }

    pub fn set_rx_data(&mut self, value: u8) {
        self.rx_data = value as u32;
    }

    /// TX_DATA write. This is the main command-byte entry point and
    /// dispatches based on [`Sio0::sio_mode`].
    pub fn set_tx_data(&mut self, cmd: u8) {
        self.stat |= sio0_stat::TX_READY | sio0_stat::TX_EMPTY;
        self.stat |= sio0_stat::RX_FIFO_NOT_EMPTY;
        bus().lock().unwrap().borrow_mut().sio0_stat = self.stat;

        if self.ctrl & sio0_ctrl::TX_ENABLE == 0 {
            // CTRL in illegal state; bail out without dispatching.
            return;
        }

        self.tx_data = cmd as u32;

        match self.sio_mode {
            sio_mode::NOT_SET => {
                // The first byte after reset selects the peripheral mode.
                self.sio_mode = cmd;
                let port = self.port;
                let slot = self.slot;
                bus().lock().unwrap().borrow_mut().mcd_index = (port, slot);
                // Stub: Pad::GetPad(port, slot)->SoftReset()
                self.set_acknowledge(true);
            }
            sio_mode::PAD => {
                // Stub: Pad::GetPad(port, slot)->SendCommandByte(cmd).
                // Returns 0xff in the absence of a real pad driver.
                self.set_acknowledge(true);
                self.set_rx_data(0xff);
            }
            sio_mode::MEMCARD => {
                let (is_mcd_cmd, present, is_psx) = {
                    let binding = bus().lock().unwrap();
                    let b = binding.borrow();
                    (
                        Self::is_memcard_command(cmd),
                        b.mcd().is_present(),
                        b.mcd().is_psx(),
                    )
                };
                if self.sio_command == memcard_command::NOT_SET {
                    if is_mcd_cmd && present && is_psx {
                        self.sio_command = cmd;
                        self.set_acknowledge(true);
                        self.set_rx_data(self.flag);
                    } else {
                        self.set_acknowledge(false);
                        self.set_rx_data(0x00);
                    }
                } else {
                    let response = {
                        let binding = bus().lock().unwrap();
                        let mut b = binding.borrow_mut();
                        b.memory_card.ps1_dispatch(self.sio_command, cmd)
                    };
                    self.set_rx_data(response);
                }
            }
            _ => {
                self.set_rx_data(0xff);
                self.set_acknowledge(false);
            }
        }

        if self.stat & sio0_stat::ACK == 0 {
            self.soft_reset();
        }

        self.interrupt(Sio0Interrupt::TxDataWrite);
    }

    pub fn set_stat(&mut self, _value: u32) {
        // The C++ implementation just logs here.
    }

    pub fn set_mode(&mut self, value: u16) {
        self.mode = value;
    }

    /// CTRL write. Switching the port or asserting RESET/ACK each has its
    /// own effect on the controller state.
    pub fn set_ctrl(&mut self, value: u16) {
        self.ctrl = value;
        self.port = if self.ctrl & sio0_ctrl::PORT != 0 { 1 } else { 0 };

        // CTRL is set to 0 between transactions; treat that as a soft reset
        // so the noisy boot-time memcard probes work correctly.
        if self.ctrl == 0 {
            bus().lock().unwrap().borrow_mut().memory_card.reset_ps1_state();
            self.soft_reset();
        }

        if self.ctrl & sio0_ctrl::ACK != 0 {
            self.stat &= !(sio0_stat::IRQ | sio0_stat::RX_PARITY_ERROR);
        }

        if self.ctrl & sio0_ctrl::RESET != 0 {
            self.stat = 0;
            self.ctrl = 0;
            self.mode = 0;
            self.soft_reset();
        }
        bus().lock().unwrap().borrow_mut().sio0_stat = self.stat;
    }

    pub fn set_baud(&mut self, value: u16) {
        self.baud = value;
    }

    /// Returns `true` if `command` falls in the documented pad command range.
    pub fn is_pad_command(_command: u8) -> bool {
        // C++ bounds: MYSTERY..=RESPONSE_BYTES. The exact enum values live
        // in the (out-of-scope) Pad subsystem; the behaviour is structural.
        false
    }

    /// Returns `true` for the three PS1 memcard commands routed through
    /// Sio0 (PS1_READ, PS1_STATE, PS1_WRITE).
    pub fn is_memcard_command(command: u8) -> bool {
        command == memcard_command::PS1_READ
            || command == memcard_command::PS1_STATE
            || command == memcard_command::PS1_WRITE
    }

    /// Returns `true` for the PS1 PocketStation command.
    pub fn is_pocketstation_command(command: u8) -> bool {
        command == memcard_command::PS1_POCKETSTATION
    }

    /// Dispatch a memcard command byte to [`MemoryCardProtocol`].
    pub fn memcard(&mut self, value: u8) -> u8 {
        let response = {
            let binding = bus().lock().unwrap();
            let mut b = binding.borrow_mut();
            b.memory_card.ps1_dispatch(self.sio_command, value)
        };
        // Mirror the C++ "unhandled command" soft-reset behaviour.
        if self.sio_command != memcard_command::PS1_READ
            && self.sio_command != memcard_command::PS1_STATE
            && self.sio_command != memcard_command::PS1_WRITE
            && self.sio_command != memcard_command::PS1_POCKETSTATION
        {
            self.soft_reset();
        }
        response
    }

    // ----- The user-requested Write8 / Write16 / Write32 entry points -----

    /// 8-bit write: forwards to [`Sio0::set_tx_data`] (the TX_DATA register
    /// is the only 8-bit register on the SIO0 bus).
    pub fn write8(&mut self, value: u8) {
        self.set_tx_data(value);
    }

    /// 16-bit write: forwards to [`Sio0::set_ctrl`], the canonical
    /// 16-bit SIO0 register (MODE, CTRL and BAUD are all 16-bit, CTRL is
    /// the one whose side-effects are interesting to model).
    pub fn write16(&mut self, value: u16) {
        self.set_ctrl(value);
    }

    /// 32-bit write: forwards to [`Sio0::set_stat`], the 32-bit STAT
    /// register.
    pub fn write32(&mut self, value: u32) {
        self.set_stat(value);
    }
}

// =========================================================================
// Sio2 (PS2 multi-protocol controller)
// =========================================================================

/// State of the Sio2 register file, translated from the C++ `Sio2` class.
#[derive(Debug, Clone)]
pub struct Sio2 {
    cmd_queue: [u32; 16],
    port_ctrl0: [u32; 4],
    port_ctrl1: [u32; 4],
    data_in: u32,
    data_out: u32,
    ctrl: u32,
    cmd_stat: u32,
    port_stat: u32,
    fifo_stat: u32,
    fifo_tx_pos: u32,
    fifo_rx_pos: u32,
    i_stat: u32,
    pub port: u8,
    queue_read: bool,
    queue_position: usize,
    command_length: usize,
    processed_length: usize,
    dma_block_size: usize,
    queue_complete: bool,
}

impl Default for Sio2 {
    fn default() -> Self {
        Self {
            cmd_queue: [0; 16],
            port_ctrl0: [0; 4],
            port_ctrl1: [0; 4],
            data_in: 0,
            data_out: 0,
            ctrl: 0,
            cmd_stat: 0,
            port_stat: 0,
            fifo_stat: 0,
            fifo_tx_pos: 0,
            fifo_rx_pos: 0,
            i_stat: 0,
            port: 0,
            queue_read: false,
            queue_position: 0,
            command_length: 0,
            processed_length: 0,
            dma_block_size: 0,
            queue_complete: false,
        }
    }
}

impl Sio2 {
    /// Default constructor, equivalent to the C++ `Sio2()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Initialise the Sio2 controller. Equivalent to `Sio2::Initialize()`.
    pub fn init(&mut self) -> bool {
        self.soft_reset();
        self.cmd_queue = [0; 16];
        self.port_ctrl0 = [0; 4];
        self.port_ctrl1 = [0; 4];
        self.data_in = 0;
        self.data_out = 0;
        self.set_ctrl(sio2_ctrl::SIO2MAN_RESET);
        self.set_cmd_stat(cmd_stat::DISCONNECTED);
        self.port_stat = port_stat::DEFAULT;
        self.fifo_stat = fifo_stat::DEFAULT;
        self.fifo_tx_pos = 0;
        self.fifo_rx_pos = 0;
        self.i_stat = 0;
        self.port = 0;

        {
            let binding = bus().lock().unwrap();
            let mut b = binding.borrow_mut();
            b.fifo_out.clear();
            for port in 0..sio::PORTS {
                for slot in 0..sio::SLOTS {
                    b.mcds[port][slot].term = 0x55;
                    b.mcds[port][slot].port = port as u8;
                    b.mcds[port][slot].slot = slot as u8;
                    b.mcds[port][slot].flag = 0x08;
                    b.mcds[port][slot].auto_eject_ticks = 0;
                }
            }
            b.mcd_index = (0, 0);
        }
        true
    }

    pub fn shutdown(&mut self) -> bool {
        true
    }

    /// Reset Sio2 transaction state and drain the input FIFO.
    pub fn soft_reset(&mut self) {
        self.queue_read = false;
        self.queue_position = 0;
        self.command_length = 0;
        self.processed_length = 0;
        self.dma_block_size = 0;
        self.queue_complete = false;
        // cmd_stat is reassembled per packet; do not carry it across.
        self.cmd_stat = 0;

        let binding = bus().lock().unwrap();
        let mut b = binding.borrow_mut();
        b.fifo_in.clear();
        b.sio2_port = self.port;
        b.sio2_command_length = self.command_length;
        b.sio2_dma_block_size = self.dma_block_size;
    }

    /// Fire the SIO2 interrupt line. Stubs the `iopIntcIrq(17)` call.
    pub fn interrupt(&mut self) {
        if self.i_stat == 0 {
            // Stub: iopIntcIrq(17)
        } else {
            // "Nearly sent double SIO2 IRQ" warning elided.
        }
        self.i_stat |= 1;
    }

    pub fn set_ctrl(&mut self, value: u32) {
        self.ctrl = value;
        if self.ctrl & sio2_ctrl::START_TRANSFER != 0 {
            self.interrupt();
        }
    }

    /// Write one entry of the 16-entry command queue.
    pub fn set_cmd(&mut self, position: usize, value: u32) {
        if position < self.cmd_queue.len() {
            self.cmd_queue[position] = value;
        }
        if position == 0 {
            self.soft_reset();
        }
    }

    pub fn set_cmd_stat(&mut self, value: u32) {
        self.cmd_stat = value;
    }

    /// PAD-mode command. The real pad dispatch is stubbed; the
    /// cmd_stat / fifo_out side effects are preserved.
    pub fn pad(&mut self) {
        let port = self.port as usize;
        {
            let binding = bus().lock().unwrap();
            let mut b = binding.borrow_mut();
            let pad_slot = b.multitap[port].get_pad_slot();
            let _ = pad_slot; // stub: Pad::GetPad lookup
        }

        // Walk the cmd_stat nibble: 1 -> 2, 0 -> 1.
        if self.cmd_stat & cmd_stat::ONE_PORT_OPEN != 0 {
            self.cmd_stat &= !cmd_stat::ONE_PORT_OPEN;
            self.cmd_stat |= cmd_stat::TWO_PORTS_OPEN;
        } else {
            self.cmd_stat |= cmd_stat::ONE_PORT_OPEN;
        }
        self.cmd_stat |= cmd_stat::NO_DEVICES_MISSING;

        let binding = bus().lock().unwrap();
        let mut b = binding.borrow_mut();
        // Stub: pad->GetType() == NotConnected || pad->ejectTicks
        if b.pad_eject_ticks[port] != 0 {
            if port == 0 {
                self.cmd_stat |= cmd_stat::PORT_1_MISSING;
            } else {
                self.cmd_stat |= cmd_stat::PORT_2_MISSING;
            }
        }

        b.fifo_out.push_back(0xff);
        // Stub: pad->SoftReset()
        // Forward every byte currently in the input FIFO to the
        // (stubbed) pad and echo its response into the output FIFO.
        while let Some(_byte) = b.fifo_in.pop_front() {
            if b.pad_eject_ticks[port] != 0 {
                b.fifo_out.push_back(0xff);
            } else {
                // Stub: pad->SendCommandByte(byte)
                b.fifo_out.push_back(0xff);
            }
        }
        // Decrement eject ticks AFTER draining, mirroring the C++.
        if b.pad_eject_ticks[port] > 0 {
            b.pad_eject_ticks[port] -= 1;
        }
    }

    /// MULTITAP-mode command. Mirrors `Sio2::Multitap()`.
    pub fn multitap(&mut self) {
        let port = self.port as usize;
        let multitap_enabled = bus().lock().unwrap().borrow().multitap_enabled[port];

        if self.cmd_stat & cmd_stat::ONE_PORT_OPEN != 0 {
            self.cmd_stat &= !cmd_stat::ONE_PORT_OPEN;
            self.cmd_stat |= cmd_stat::TWO_PORTS_OPEN;
        } else {
            self.cmd_stat |= cmd_stat::ONE_PORT_OPEN;
        }
        self.cmd_stat |= cmd_stat::NO_DEVICES_MISSING;

        if !multitap_enabled {
            // MTAPMAN only honours the PORT_1_MISSING bit.
            self.cmd_stat |= cmd_stat::PORT_1_MISSING;
        }

        bus().lock().unwrap().borrow_mut().multitap[port].send_to_multitap();
    }

    /// INFRARED-mode command. Always reports disconnect and dead air.
    pub fn infrared(&mut self) {
        self.set_cmd_stat(cmd_stat::DISCONNECTED);
        let binding = bus().lock().unwrap();
        let mut b = binding.borrow_mut();
        b.fifo_in.pop_front();
        while b.fifo_out.len() < self.command_length {
            b.fifo_out.push_back(0xff);
        }
    }

    /// MEMCARD-mode command: dispatches the leading byte to
    /// [`MemoryCardProtocol`].
    pub fn memcard(&mut self) {
        let port = self.port as usize;

        // The C++ uses the multitap-selected memcard slot as the active one.
        let memcard_slot = bus().lock().unwrap().borrow().multitap[port].get_memcard_slot();
        {
            let binding = bus().lock().unwrap();
            let mut b = binding.borrow_mut();
            b.mcd_index = (port as u8, memcard_slot);
        }
        let (mcd_present, auto_eject_ticks) = {
            let binding = bus().lock().unwrap();
            let b = binding.borrow();
            (b.mcd().is_present(), b.mcd().auto_eject_ticks)
        };

        if auto_eject_ticks > 0 {
            self.set_cmd_stat(cmd_stat::DISCONNECTED);
            let binding = bus().lock().unwrap();
            let mut b = binding.borrow_mut();
            b.fifo_out.push_back(0xff);
            while let Some(_) = b.fifo_in.pop_front() {
                b.fifo_out.push_back(0xff);
            }
            return;
        }

        self.set_cmd_stat(if mcd_present {
            cmd_stat::CONNECTED
        } else {
            cmd_stat::DISCONNECTED
        });

        let command_byte = {
            let binding = bus().lock().unwrap();
            let mut b = binding.borrow_mut();
            match b.fifo_in.pop_front() {
                Some(v) => v,
                None => return,
            }
        };
        {
            let binding = bus().lock().unwrap();
            let mut b = binding.borrow_mut();
            let response_byte = if mcd_present { 0x00 } else { 0xff };
            b.fifo_out.push_back(response_byte);
            b.fifo_out.push_back(response_byte);
        }

        match command_byte {
            memcard_command::PROBE => bus().lock().unwrap().borrow_mut().memory_card.probe(),
            memcard_command::UNKNOWN_WRITE_DELETE_END => {
                bus().lock().unwrap().borrow_mut().memory_card.unknown_write_delete_end()
            }
            memcard_command::SET_ERASE_SECTOR
            | memcard_command::SET_WRITE_SECTOR
            | memcard_command::SET_READ_SECTOR => {
                bus().lock().unwrap().borrow_mut().memory_card.set_sector()
            }
            memcard_command::GET_SPECS => bus().lock().unwrap().borrow_mut().memory_card.get_specs(),
            memcard_command::SET_TERMINATOR => {
                bus().lock().unwrap().borrow_mut().memory_card.set_terminator()
            }
            memcard_command::GET_TERMINATOR => {
                bus().lock().unwrap().borrow_mut().memory_card.get_terminator()
            }
            memcard_command::WRITE_DATA => bus().lock().unwrap().borrow_mut().memory_card.write_data(),
            memcard_command::READ_DATA => bus().lock().unwrap().borrow_mut().memory_card.read_data(),
            memcard_command::PS1_READ => {
                ps1_drain_fifo(|mcd, b| mcd.ps1_read(b));
            }
            memcard_command::PS1_STATE => {
                ps1_drain_fifo(|mcd, b| mcd.ps1_state(b));
            }
            memcard_command::PS1_WRITE => {
                ps1_drain_fifo(|mcd, b| mcd.ps1_write(b));
            }
            memcard_command::PS1_POCKETSTATION => {
                ps1_drain_fifo(|mcd, b| mcd.ps1_pocketstation(b));
            }
            memcard_command::READ_WRITE_END => {
                bus().lock().unwrap().borrow_mut().memory_card.read_write_end()
            }
            memcard_command::ERASE_BLOCK => bus().lock().unwrap().borrow_mut().memory_card.erase_block(),
            memcard_command::UNKNOWN_BOOT => bus().lock().unwrap().borrow_mut().memory_card.unknown_boot(),
            memcard_command::AUTH_XOR => bus().lock().unwrap().borrow_mut().memory_card.auth_xor(),
            memcard_command::AUTH_F3 => bus().lock().unwrap().borrow_mut().memory_card.auth_f3(),
            memcard_command::AUTH_F7 => bus().lock().unwrap().borrow_mut().memory_card.auth_f7(),
            _ => {}
        }
    }

    /// Main SIO2 write entry point. Equivalent to `Sio2::Write(u8)`.
    pub fn write(&mut self, data: u8) {
        if !self.queue_read {
            if self.queue_position > self.cmd_queue.len() {
                return;
            }
            let current_cmd = self.cmd_queue[self.queue_position];
            self.port = (current_cmd & sio2_cmd::PORT) as u8;
            self.command_length =
                ((current_cmd >> 8) & sio2_cmd::COMMAND_LENGTH_MASK as u32) as usize;
            self.queue_read = true;

            if self.command_length == 0 {
                self.queue_complete = true;
            }

            // Drop any stale input from the prior command.
            bus().lock().unwrap().borrow_mut().fifo_in.clear();
        }

        if self.queue_complete {
            return;
        }

        let fifo_in_len = {
            let binding = bus().lock().unwrap();
            let mut b = binding.borrow_mut();
            b.fifo_in.push_back(data);
            b.fifo_in.len()
        };
        if (fifo_in_len == self.command_length && self.dma_block_size == 0)
            || fifo_in_len == self.dma_block_size
        {
            // Prep for the next command queue entry.
            self.queue_read = false;
            self.queue_position += 1;

            let sio_mode = {
                let binding = bus().lock().unwrap();
                let mut b = binding.borrow_mut();
                match b.fifo_in.pop_front() {
                    Some(v) => v,
                    None => return,
                }
            };
            bus().lock().unwrap().borrow_mut().sio2_port = self.port;
            bus().lock().unwrap().borrow_mut().sio2_command_length = self.command_length;
            bus().lock().unwrap().borrow_mut().sio2_dma_block_size = self.dma_block_size;

            match sio_mode {
                sio_mode::PAD => self.pad(),
                sio_mode::MULTITAP => self.multitap(),
                sio_mode::INFRARED => self.infrared(),
                sio_mode::MEMCARD => self.memcard(),
                _ => {
                    bus().lock().unwrap().borrow_mut().fifo_out.push_back(0xff);
                    self.set_cmd_stat(cmd_stat::DISCONNECTED);
                }
            }

            // If the command arrived over DMA, pad the output up to the
            // DMA block boundary.
            let (fifo_out_len, dma_block_size) = {
                let binding = bus().lock().unwrap();
                let b = binding.borrow();
                (b.fifo_out.len(), self.dma_block_size)
            };
            if dma_block_size > 0 {
                let rem = fifo_out_len % dma_block_size;
                if rem > 0 {
                    let pad = dma_block_size - rem;
                    let binding = bus().lock().unwrap();
                    let mut b = binding.borrow_mut();
                    for _ in 0..pad {
                        b.fifo_out.push_back(0x00);
                    }
                }
            }
        }
    }

    /// Read the next byte from the SIO2 output FIFO.
    pub fn read(&mut self) -> u8 {
        match bus().lock().unwrap().borrow_mut().fifo_out.pop_front() {
            Some(v) => v,
            None => 0xff,
        }
    }
}

/// Drain the input FIFO through a PS1-style per-byte memcard command, pushing
/// the response into the output FIFO. Used by [`Sio2::memcard`] for the
/// `PS1_READ / PS1_STATE / PS1_WRITE / PS1_POCKETSTATION` cases.
fn ps1_drain_fifo<F: FnMut(&mut MemoryCardProtocol, u8) -> u8>(mut f: F) {
    let binding = bus().lock().unwrap();
    let mut b = binding.borrow_mut();
    b.memory_card.reset_ps1_state();
    while let Some(byte) = b.fifo_in.pop_front() {
        let response = f(&mut b.memory_card, byte);
        b.fifo_out.push_back(response);
    }
}

// =========================================================================
// MultitapProtocol (1-3-7 multiplex on each physical port)
// =========================================================================

/// Multitap protocol state, one per physical port.
///
/// Multitap adds three extra slots on top of the direct pad, giving a
/// 1-3-7 multiplex: 1 direct + 3 multitap slots, repeated for the second
/// physical port.
pub struct MultitapProtocol {
    current_pad_slot: u8,
    current_memcard_slot: u8,
}

impl Default for MultitapProtocol {
    fn default() -> Self {
        Self {
            current_pad_slot: 0,
            current_memcard_slot: 0,
        }
    }
}

impl MultitapProtocol {
    pub fn new() -> Self {
        Self::default()
    }

    /// Soft reset is a no-op for the multitap state machine.
    pub fn soft_reset(&mut self) {}

    /// Reset both slot counters back to 0.
    pub fn full_reset(&mut self) {
        self.soft_reset();
        self.current_pad_slot = 0;
        self.current_memcard_slot = 0;
    }

    pub fn get_pad_slot(&self) -> u8 {
        self.current_pad_slot
    }

    pub fn get_memcard_slot(&self) -> u8 {
        self.current_memcard_slot
    }

    /// Multitap command entry point, equivalent to `MultitapProtocol::SendToMultitap()`.
    pub fn send_to_multitap(&mut self) {
        let (command_byte, port) = {
            let binding = bus().lock().unwrap();
            let b = binding.borrow();
            match b.fifo_in.front() {
                Some(&v) => (v, b.sio2_port),
                None => return,
            }
        };
        let _ = port;

        let mode = match command_byte {
            0x12 => MultitapMode::PadSupportCheck,
            0x13 => MultitapMode::MemcardSupportCheck,
            0x21 => MultitapMode::SelectPad,
            0x22 => MultitapMode::SelectMemcard,
            _ => MultitapMode::NotSet,
        };

        // Drop the leading command byte.
        bus().lock().unwrap().borrow_mut().fifo_in.pop_front();

        match mode {
            MultitapMode::PadSupportCheck | MultitapMode::MemcardSupportCheck => {
                self.support_check();
            }
            MultitapMode::SelectPad => self.select(MultitapMode::SelectPad),
            MultitapMode::SelectMemcard => self.select(MultitapMode::SelectMemcard),
            MultitapMode::NotSet => {}
        }
    }

    fn support_check(&self) {
        let port = bus().lock().unwrap().borrow().sio2_port as usize;
        let enabled = bus().lock().unwrap().borrow().multitap_enabled[port];
        let binding = bus().lock().unwrap();
        let mut b = binding.borrow_mut();
        if enabled {
            b.fifo_out.push_back(0xff);
            b.fifo_out.push_back(0x80);
            b.fifo_out.push_back(0x5a);
            b.fifo_out.push_back(0x04);
            b.fifo_out.push_back(0x00);
            b.fifo_out.push_back(0x5a);
        } else {
            for _ in 0..6 {
                b.fifo_out.push_back(0xff);
            }
        }
    }

    fn select(&mut self, mode: MultitapMode) {
        let port = bus().lock().unwrap().borrow().sio2_port as usize;
        let enabled = bus().lock().unwrap().borrow().multitap_enabled[port];
        if !enabled {
            let binding = bus().lock().unwrap();
            let mut b = binding.borrow_mut();
            for _ in 0..7 {
                b.fifo_out.push_back(0xff);
            }
            return;
        }

        let new_slot = {
            let binding = bus().lock().unwrap();
            let mut b = binding.borrow_mut();
            match b.fifo_in.pop_front() {
                Some(s) => s,
                None => return,
            }
        };
        let in_bounds = (new_slot as usize) < sio::SLOTS;
        if in_bounds {
            match mode {
                MultitapMode::SelectPad => self.current_pad_slot = new_slot,
                MultitapMode::SelectMemcard => self.current_memcard_slot = new_slot,
                _ => {}
            }
        }
        let binding = bus().lock().unwrap();
        let mut b = binding.borrow_mut();
        b.fifo_out.push_back(0xff);
        b.fifo_out.push_back(0x80);
        b.fifo_out.push_back(0x5a);
        b.fifo_out.push_back(0x00);
        b.fifo_out.push_back(0x00);
        b.fifo_out.push_back(if in_bounds { new_slot } else { 0xff });
        b.fifo_out.push_back(if in_bounds { 0x5a } else { 0x66 });
    }
}

// =========================================================================
// MemoryCardProtocol (PS1 + PS2 command set)
// =========================================================================

/// Internal PS1-protocol cursor state, translated from `PS1MemoryCardState`.
#[derive(Debug, Clone)]
pub struct Ps1MemoryCardState {
    pub current_byte: usize,
    pub sector_addr_msb: u8,
    pub sector_addr_lsb: u8,
    pub checksum: u8,
    pub expected_checksum: u8,
    pub buf: [u8; 128],
}

impl Default for Ps1MemoryCardState {
    fn default() -> Self {
        Self {
            current_byte: 2,
            sector_addr_msb: 0,
            sector_addr_lsb: 0,
            checksum: 0,
            expected_checksum: 0,
            buf: [0; 128],
        }
    }
}

/// Memory card protocol command set, translated from `MemoryCardProtocol`.
pub struct MemoryCardProtocol {
    ps1_state: Ps1MemoryCardState,
}

impl Default for MemoryCardProtocol {
    fn default() -> Self {
        Self {
            ps1_state: Ps1MemoryCardState::default(),
        }
    }
}

impl MemoryCardProtocol {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset the PS1 cursor to the start of a fresh transaction.
    pub fn reset_ps1_state(&mut self) {
        self.ps1_state = Ps1MemoryCardState::default();
    }

    fn ps1_fail(&self) -> bool {
        let (mcd_psx, cmd_len) = {
            let binding = bus().lock().unwrap();
            let b = binding.borrow();
            (b.mcd().is_psx(), b.sio2_command_length)
        };
        if mcd_psx && cmd_len > 0 {
            let binding = bus().lock().unwrap();
            let mut b = binding.borrow_mut();
            while b.fifo_out.len() < cmd_len {
                b.fifo_out.push_back(0x00);
            }
            true
        } else {
            false
        }
    }

    /// Pad the output FIFO with zero bytes then 0x2b + the memcard
    /// terminator, as the `0x2b` + term pattern does throughout PS2 memcard
    /// command handling.
    fn the_2b_terminator(&self, length: usize) {
        let term = bus().lock().unwrap().borrow().mcd().term;
        let binding = bus().lock().unwrap();
        let mut b = binding.borrow_mut();
        while b.fifo_out.len() < length.saturating_sub(2) {
            b.fifo_out.push_back(0x00);
        }
        b.fifo_out.push_back(0x2b);
        b.fifo_out.push_back(term);
    }

    fn read_write_increment(&mut self, length: usize) {
        bus().lock().unwrap().borrow_mut().mcd_mut().transfer_addr += length as u32;
    }

    fn recalculate_ps1_addr(&mut self) {
        let binding = bus().lock().unwrap();
        let mut b = binding.borrow_mut();
        let mcd = b.mcd_mut();
        mcd.sector_addr = ((self.ps1_state.sector_addr_msb as u32) << 8)
            | (self.ps1_state.sector_addr_lsb as u32);
        mcd.good_sector = mcd.sector_addr <= 0x03ff;
        mcd.transfer_addr = 128 * mcd.sector_addr;
    }

    pub fn probe(&mut self) {
        if self.ps1_fail() {
            return;
        }
        let (present, term) = {
            let binding = bus().lock().unwrap();
            let b = binding.borrow();
            (b.mcd().is_present(), b.mcd().term)
        };
        let binding = bus().lock().unwrap();
        let mut b = binding.borrow_mut();
        if !present {
            for _ in 0..4 {
                b.fifo_out.push_back(0xff);
            }
        } else {
            while b.fifo_out.len() < 4 - 2 {
                b.fifo_out.push_back(0x00);
            }
            b.fifo_out.push_back(0x2b);
            b.fifo_out.push_back(term);
        }
    }

    pub fn unknown_write_delete_end(&mut self) {
        if self.ps1_fail() {
            return;
        }
        let length = 4;
        let term = bus().lock().unwrap().borrow().mcd().term;
        let binding = bus().lock().unwrap();
        let mut b = binding.borrow_mut();
        while b.fifo_out.len() < length - 2 {
            b.fifo_out.push_back(0x00);
        }
        b.fifo_out.push_back(0x2b);
        b.fifo_out.push_back(term);
    }

    pub fn set_sector(&mut self) {
        if self.ps1_fail() {
            return;
        }
        let (sector_lsb, sector_2nd, sector_3rd, sector_msb, expected_checksum) = {
            let binding = bus().lock().unwrap();
            let mut b = binding.borrow_mut();
            let lsb = b.fifo_in.pop_front().unwrap_or(0);
            let s2 = b.fifo_in.pop_front().unwrap_or(0);
            let s3 = b.fifo_in.pop_front().unwrap_or(0);
            let msb = b.fifo_in.pop_front().unwrap_or(0);
            let chk = b.fifo_in.pop_front().unwrap_or(0);
            (lsb, s2, s3, msb, chk)
        };
        let computed_checksum = sector_lsb ^ sector_2nd ^ sector_3rd ^ sector_msb;
        let good_sector = computed_checksum == expected_checksum;
        let new_sector = (sector_lsb as u32)
            | ((sector_2nd as u32) << 8)
            | ((sector_3rd as u32) << 16)
            | ((sector_msb as u32) << 24);

        {
            let binding = bus().lock().unwrap();
            let mut b = binding.borrow_mut();
            let mcd = b.mcd_mut();
            mcd.good_sector = good_sector;
            mcd.sector_addr = new_sector;
            let info = mcd.get_size_info();
            mcd.transfer_addr = (info.sector_size + 16) * mcd.sector_addr;
        }
        self.the_2b_terminator(9);
    }

    pub fn get_specs(&mut self) {
        if self.ps1_fail() {
            return;
        }
        let (info, term) = {
            let binding = bus().lock().unwrap();
            let b = binding.borrow();
            (b.mcd().get_size_info(), b.mcd().term)
        };
        let binding = bus().lock().unwrap();
        let mut b = binding.borrow_mut();
        b.fifo_out.push_back(0x2b);
        b.fifo_out.push_back((info.sector_size & 0xff) as u8);
        b.fifo_out.push_back((info.sector_size >> 8) as u8);
        b.fifo_out
            .push_back((info.erase_block_size_in_sectors & 0xff) as u8);
        b.fifo_out
            .push_back((info.erase_block_size_in_sectors >> 8) as u8);
        b.fifo_out.push_back((info.mcd_size_in_sectors & 0xff) as u8);
        b.fifo_out.push_back((info.mcd_size_in_sectors >> 8) as u8);
        b.fifo_out.push_back((info.mcd_size_in_sectors >> 16) as u8);
        b.fifo_out.push_back((info.mcd_size_in_sectors >> 24) as u8);
        b.fifo_out.push_back(info.xor);
        b.fifo_out.push_back(term);
    }

    pub fn set_terminator(&mut self) {
        if self.ps1_fail() {
            return;
        }
        let term = {
            let binding = bus().lock().unwrap();
            let mut b = binding.borrow_mut();
            let new_term = b.fifo_in.pop_front().unwrap_or(terminator::READY as u8);
            b.mcd_mut().term = new_term;
            new_term
        };
        let binding = bus().lock().unwrap();
        let mut b = binding.borrow_mut();
        b.fifo_out.push_back(0x00);
        b.fifo_out.push_back(0x2b);
        b.fifo_out.push_back(term);
    }

    pub fn get_terminator(&mut self) {
        if self.ps1_fail() {
            return;
        }
        let term = bus().lock().unwrap().borrow().mcd().term;
        let binding = bus().lock().unwrap();
        let mut b = binding.borrow_mut();
        b.fifo_out.push_back(0x2b);
        b.fifo_out.push_back(term);
        b.fifo_out.push_back(term);
    }

    pub fn write_data(&mut self) {
        if self.ps1_fail() {
            return;
        }
        let write_length;
        let mut checksum: u8 = 0;
        let mut buf: Vec<u8>;
        {
            let binding = bus().lock().unwrap();
            let mut b = binding.borrow_mut();
            b.fifo_out.push_back(0x00);
            b.fifo_out.push_back(0x2b);
            write_length = b.fifo_in.pop_front().unwrap_or(0);
            buf = Vec::with_capacity(write_length as usize);
            for _ in 0..write_length {
                let byte = b.fifo_in.pop_front().unwrap_or(0);
                checksum ^= byte;
                buf.push(byte);
                b.fifo_out.push_back(0x00);
            }
            b.mcd_mut().write(&buf);
            b.fifo_out.push_back(checksum);
            let term = b.mcd().term;
            b.fifo_out.push_back(term);
        }

        self.read_write_increment(write_length as usize);
        memcard_busy_set_busy();
    }

    pub fn read_data(&mut self) {
        if self.ps1_fail() {
            return;
        }
        let read_length;
        {
            let binding = bus().lock().unwrap();
            let mut b = binding.borrow_mut();
            read_length = b.fifo_in.pop_front().unwrap_or(0);
            b.fifo_out.push_back(0x00);
            b.fifo_out.push_back(0x2b);
        }
        let mut buf = vec![0u8; read_length as usize];
        {
            let binding = bus().lock().unwrap();
            let mut b = binding.borrow_mut();
            b.mcd_mut().read(&mut buf);
        }
        let mut checksum: u8 = 0;
        {
            let binding = bus().lock().unwrap();
            let mut b = binding.borrow_mut();
            for &b_byte in &buf {
                checksum ^= b_byte;
                b.fifo_out.push_back(b_byte);
            }
            b.fifo_out.push_back(checksum);
            let term = b.mcd().term;
            b.fifo_out.push_back(term);
        }
        self.read_write_increment(read_length as usize);
    }

    /// Dispatch helper for the PS1 memcard commands routed through Sio0.
    pub fn ps1_dispatch(&mut self, command: u8, data: u8) -> u8 {
        match command {
            memcard_command::PS1_READ => self.ps1_read(data),
            memcard_command::PS1_STATE => self.ps1_state(data),
            memcard_command::PS1_WRITE => self.ps1_write(data),
            memcard_command::PS1_POCKETSTATION => self.ps1_pocketstation(data),
            _ => 0xff,
        }
    }

    /// PS1 read command byte. Mirrors `MemoryCardProtocol::PS1Read()`.
    pub fn ps1_read(&mut self, data: u8) -> u8 {
        if !bus().lock().unwrap().borrow().mcd().is_present() {
            return 0xff;
        }

        let mut send_ack = true;
        let ret: u8 = match self.ps1_state.current_byte {
            2 => 0x5a,
            3 => 0x5d,
            4 => {
                self.ps1_state.sector_addr_msb = data;
                0x00
            }
            5 => {
                self.ps1_state.sector_addr_lsb = data;
                self.recalculate_ps1_addr();
                0x00
            }
            6 => 0x5c,
            7 => 0x5d,
            8 => self.ps1_state.sector_addr_msb,
            9 => self.ps1_state.sector_addr_lsb,
            138 => self.ps1_state.checksum,
            139 => {
                send_ack = false;
                0x47
            }
            10 => {
                self.ps1_state.checksum =
                    self.ps1_state.sector_addr_msb ^ self.ps1_state.sector_addr_lsb;
                {
                    let binding = bus().lock().unwrap();
                    let mut b = binding.borrow_mut();
                    b.mcd_mut().read(&mut self.ps1_state.buf);
                }
                let v = self.ps1_state.buf[self.ps1_state.current_byte - 10];
                self.ps1_state.checksum ^= v;
                v
            }
            _ => {
                let v = self.ps1_state.buf[self.ps1_state.current_byte - 10];
                self.ps1_state.checksum ^= v;
                v
            }
        };

        // Mirror the C++ `g_Sio0.SetAcknowledge(sendAck)` call.
        if send_ack {
            bus().lock().unwrap().borrow_mut().sio0_stat |= sio0_stat::ACK;
        } else {
            bus().lock().unwrap().borrow_mut().sio0_stat &= !sio0_stat::ACK;
        }
        self.ps1_state.current_byte += 1;
        ret
    }

    /// PS1 state command. The C++ `PS1State` is a stub that simply
    /// `Console.Error`s and returns 0; we preserve that behaviour.
    pub fn ps1_state(&mut self, _data: u8) -> u8 {
        0x00
    }

    /// PS1 write command byte. Mirrors `MemoryCardProtocol::PS1Write()`.
    pub fn ps1_write(&mut self, data: u8) -> u8 {
        let mut send_ack = true;
        let ret: u8 = match self.ps1_state.current_byte {
            2 => 0x5a,
            3 => 0x5d,
            4 => {
                self.ps1_state.sector_addr_msb = data;
                0x00
            }
            5 => {
                self.ps1_state.sector_addr_lsb = data;
                self.recalculate_ps1_addr();
                0x00
            }
            134 => {
                self.ps1_state.expected_checksum = data;
                0
            }
            135 => 0x5c,
            136 => 0x5d,
            137 => {
                let (good_sector, expected, actual) = {
                    let binding = bus().lock().unwrap();
                    let b = binding.borrow();
                    (
                        b.mcd().good_sector,
                        self.ps1_state.expected_checksum,
                        self.ps1_state.checksum,
                    )
                };
                send_ack = false;
                if !good_sector {
                    0xff
                } else if expected != actual {
                    0x4e
                } else {
                    {
                        let binding = bus().lock().unwrap();
                        let mut b = binding.borrow_mut();
                        b.mcd_mut().write(&self.ps1_state.buf);
                        // Clear the "directory unread" bit of the flag byte.
                        b.mcd_mut().flag &= 0x07;
                    }
                    0x47
                }
            }
            6 => {
                self.ps1_state.checksum =
                    self.ps1_state.sector_addr_msb ^ self.ps1_state.sector_addr_lsb;
                self.ps1_state.buf[self.ps1_state.current_byte - 6] = data;
                self.ps1_state.checksum ^= data;
                0x00
            }
            _ => {
                self.ps1_state.buf[self.ps1_state.current_byte - 6] = data;
                self.ps1_state.checksum ^= data;
                0x00
            }
        };

        // Mirror the C++ `g_Sio0.SetAcknowledge(sendAck)` call.
        if send_ack {
            bus().lock().unwrap().borrow_mut().sio0_stat |= sio0_stat::ACK;
        } else {
            bus().lock().unwrap().borrow_mut().sio0_stat &= !sio0_stat::ACK;
        }
        self.ps1_state.current_byte += 1;
        memcard_busy_set_busy();
        ret
    }

    /// PS1 PocketStation command. The C++ simply clears ACK and returns 0.
    pub fn ps1_pocketstation(&mut self, _data: u8) -> u8 {
        bus().lock().unwrap().borrow_mut().sio0_stat &= !sio0_stat::ACK;
        0x00
    }

    pub fn read_write_end(&mut self) {
        if self.ps1_fail() {
            return;
        }
        self.the_2b_terminator(4);
    }

    pub fn erase_block(&mut self) {
        if self.ps1_fail() {
            return;
        }
        {
            let binding = bus().lock().unwrap();
            let mut b = binding.borrow_mut();
            b.mcd_mut().erase_block();
        }
        self.the_2b_terminator(4);
        memcard_busy_set_busy();
    }

    pub fn unknown_boot(&mut self) {
        if self.ps1_fail() {
            return;
        }
        self.the_2b_terminator(5);
    }

    pub fn auth_xor(&mut self) {
        if self.ps1_fail() {
            return;
        }
        let mode_byte = {
            let binding = bus().lock().unwrap();
            let mut b = binding.borrow_mut();
            b.fifo_in.pop_front().unwrap_or(0)
        };
        match mode_byte {
            0x01 | 0x02 | 0x04 | 0x0f | 0x11 | 0x13 => {
                // Long + XOR
                let term = bus().lock().unwrap().borrow().mcd().term;
                let binding = bus().lock().unwrap();
                let mut b = binding.borrow_mut();
                b.fifo_out.push_back(0x00);
                b.fifo_out.push_back(0x2b);
                let mut xor_result: u8 = 0;
                for _ in 0..8 {
                    let byte = b.fifo_in.pop_front().unwrap_or(0);
                    xor_result ^= byte;
                    b.fifo_out.push_back(0x00);
                }
                b.fifo_out.push_back(xor_result);
                b.fifo_out.push_back(term);
            }
            0x00 | 0x03 | 0x05 | 0x08 | 0x09 | 0x0a | 0x0c | 0x0d | 0x0e | 0x10 | 0x12 | 0x14 => {
                // Short + No XOR
                self.the_2b_terminator(5);
            }
            0x06 | 0x07 | 0x0b => {
                // Long + No XOR
                self.the_2b_terminator(14);
            }
            _ => {}
        }
    }

    pub fn auth_f3(&mut self) {
        if self.ps1_fail() {
            return;
        }
        let present = bus().lock().unwrap().borrow().mcd().is_present();
        if !present {
            let binding = bus().lock().unwrap();
            let mut b = binding.borrow_mut();
            for _ in 0..4 {
                b.fifo_out.push_back(0xff);
            }
        } else {
            {
                let binding = bus().lock().unwrap();
                let mut b = binding.borrow_mut();
                b.mcd_mut().term = terminator::READY as u8;
            }
            self.the_2b_terminator(5);
        }
    }

    pub fn auth_f7(&mut self) {
        if self.ps1_fail() {
            return;
        }
        self.the_2b_terminator(5);
    }
}

// =========================================================================
// Stub of `MemcardBusy::SetBusy()`. Bumps the "last busy" frame counter;
// a real PCSX2 port would set a 300-tick busy window against `g_FrameCount`.
// =========================================================================

fn memcard_busy_set_busy() {
    let binding = bus().lock().unwrap();
    let mut b = binding.borrow_mut();
    b.last_frame_mcd_busy = b.last_frame_mcd_busy.wrapping_add(1);
}
