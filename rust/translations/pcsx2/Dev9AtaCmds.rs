//! DEV9 ATA (IDE) command, transfer, and image-creation translation.
//!
//! This module is an idiomatic Rust 2021 port of the PCSX2 ATA subsystem
//! previously implemented across:
//!
//! * `DEV9/ATA/ATA_State.cpp`
//! * `DEV9/ATA/ATA_Info.cpp`
//! * `DEV9/ATA/ATA_Transfer.cpp`
//! * `DEV9/ATA/Commands/ATA_Command.cpp`
//! * `DEV9/ATA/Commands/ATA_CmdDMA.cpp`
//! * `DEV9/ATA/Commands/ATA_CmdExecuteDeviceDiag.cpp`
//! * `DEV9/ATA/Commands/ATA_CmdNoData.cpp`
//! * `DEV9/ATA/Commands/ATA_CmdPIOData.cpp`
//! * `DEV9/ATA/Commands/ATA_CmdSMART.cpp`
//! * `DEV9/ATA/Commands/ATA_SCE.cpp`
//! * `DEV9/ATA/HddCreate.{h,cpp}`
//!
//! The module exposes a single [`AtaDrive`] type containing every register,
//! buffer, and configuration knob needed to emulate an IDE disk image for the
//! PS2.  A background reader/writer thread is intentionally omitted in this
//! translation; reads and writes are performed synchronously against the
//! open [`File`] when [`ataExecCmd`] dispatches a command that needs I/O.
//!
//! Only `std` is used.

use std::cmp::min;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

// ---------------------------------------------------------------------------
// ATA register / status / error flag constants.
// ---------------------------------------------------------------------------

pub const ATA_INTR_INTRQ: u32 = 1 << 0;

pub const ATA_ERR_MARK: u8 = 0x01;
pub const ATA_ERR_TRACK0: u8 = 0x02;
pub const ATA_ERR_ABORT: u8 = 0x04;
pub const ATA_ERR_MCR: u8 = 0x08;
pub const ATA_ERR_ID: u8 = 0x10;
pub const ATA_ERR_MC: u8 = 0x20;
pub const ATA_ERR_ECC: u8 = 0x40;
pub const ATA_ERR_ICRC: u8 = 0x80;

pub const ATA_STAT_ERR: u8 = 0x01;
pub const ATA_STAT_INDEX: u8 = 0x02;
pub const ATA_STAT_ECC: u8 = 0x04;
pub const ATA_STAT_DRQ: u8 = 0x08;
pub const ATA_STAT_SEEK: u8 = 0x10;
pub const ATA_STAT_WRERR: u8 = 0x20;
pub const ATA_STAT_READY: u8 = 0x40;
pub const ATA_STAT_BUSY: u8 = 0x80;

// IDE register file offsets relative to the device base.
pub const ATA_R_DATA: u32 = 0x00;
pub const ATA_R_ERROR: u32 = 0x02;
pub const ATA_R_FEATURE: u32 = 0x02;
pub const ATA_R_NSECTOR: u32 = 0x04;
pub const ATA_R_SECTOR: u32 = 0x06;
pub const ATA_R_LCYL: u32 = 0x08;
pub const ATA_R_HCYL: u32 = 0x0A;
pub const ATA_R_SELECT: u32 = 0x0C;
pub const ATA_R_STATUS: u32 = 0x0E;
pub const ATA_R_CMD: u32 = 0x0E;
pub const ATA_R_ALT_STATUS: u32 = 0x1C;
pub const ATA_R_CONTROL: u32 = 0x1C;

const SECTOR_SIZE: u64 = 512;
const IDENTIFY_WORDS: usize = 256; // 256 u16 == 512 bytes.
const PIO_BUFFER_SIZE: usize = 512;
const SCE_SEC_SIZE: usize = 256 * 2;

// ---------------------------------------------------------------------------
// ATA drive state.
// ---------------------------------------------------------------------------

/// In-memory emulation of a single ATA drive attached to DEV9.
///
/// `regs` is the 256-byte IDE register file (`regCommand`, `regStatus`,
/// `regError`, `regSelect`, …) packed into a single array.  `io_buffer`
/// holds the currently staged PIO data; `read_buffer` holds the last
/// read-fetched sectors; and the rest of the fields mirror the original
/// C++ `ATA` class.
pub struct AtaDrive {
    // 256-byte IDE register file (one byte per register/offset).
    pub regs: [u8; 0x100],

    // I/O staging buffer (PIO data being transferred to the host).
    pub io_buffer: Vec<u8>,

    // Drive size in bytes; reported through the IDENTIFY DEVICE word 60/61
    // (or word 100-103 when LBA48 is supported).
    pub drive_size: u64,

    // The file backing the virtual HDD; `None` when no image is open.
    file: Option<File>,

    // True once `ataInit` has run and the drive has completed its power-on
    // self-diagnostic.
    initialized: bool,

    // Cached PIO transfer cursor.
    pio_ptr: usize,
    pio_end: usize,

    // Read-side buffer (sector data fetched from the image, ready for PIO
    // hand-off to the host).
    read_buffer: Vec<u8>,
    rd_transferred: usize,
    wr_transferred: usize,

    // Identifies the IDENTIFY DEVICE payload; mirrors the 512-byte
    // structure populated by `CreateHDDinfo`.
    identify_data: [u8; SECTOR_SIZE as usize],

    // Sony-specific SCE security block; populated from `<image>.hddid` if
    // present, otherwise filled with PS2 magic bytes.
    sce_sec: [u8; SCE_SEC_SIZE],

    // Currently-selected CHS geometry / multiple-sector setting.
    cur_heads: u8,
    cur_sectors: u8,
    cur_cylinders: u16,
    cur_multiple_sectors_setting: u8,

    // PIO/MDMA/UDMA mode, -1 when disabled.
    pio_mode: i32,
    mdma_mode: i32,
    udma_mode: i32,

    // Currently-executing sector count.
    nsector: i32,
    nsector_left: i32,

    // True when the host has armed DMA.
    dma_ready: bool,

    // LBA mode: false = CHS / 28-bit LBA, true = LBA48.
    lba48: bool,
    lba48_supported: bool,

    // Pending flush / write-cache state.
    await_flush: bool,
    fet_write_cache_enabled: bool,

    // Feature toggles reported through IDENTIFY DEVICE.
    fet_smart_enabled: bool,
    fet_security_enabled: bool,
    fet_host_protected_area_enabled: bool,

    // SMART state.
    smart_autosave: bool,
    smart_errors: bool,
    smart_self_test_count: u8,

    // IRQ / device-control register state.
    reg_control_enable_irq: bool,
    reg_control_hob_read: bool,
    pending_interrupt: bool,

    // Locked SEEK bit (see regStatusSeekLock in the C++ source).
    reg_status_seek_lock: i8,

    // When > 0, this many sectors are still to be transferred to the host
    // through PIO/DMA.
    sectors_per_interrupt: i32,

    // If non-zero, holds the sector count that we have already transferred
    // out of `read_buffer`.  When it equals `nsector * 512` the read
    // command terminates.
    rd_done_sectors: i32,
}

impl Default for AtaDrive {
    fn default() -> Self {
        Self::new()
    }
}

impl AtaDrive {
    /// Construct an uninitialised drive.  Call [`ataInit`] (or [`AtaDrive::open_image`])
    /// before issuing commands.
    pub fn new() -> Self {
        let mut drive = Self {
            regs: [0u8; 0x100],
            io_buffer: Vec::new(),
            drive_size: 0,
            file: None,
            initialized: false,

            pio_ptr: 0,
            pio_end: 0,
            read_buffer: Vec::new(),
            rd_transferred: 0,
            wr_transferred: 0,
            identify_data: [0u8; SECTOR_SIZE as usize],
            sce_sec: [0u8; SCE_SEC_SIZE],

            cur_heads: 16,
            cur_sectors: 63,
            cur_cylinders: 0,
            cur_multiple_sectors_setting: 128,

            pio_mode: 4,
            mdma_mode: 2,
            udma_mode: -1,

            nsector: 0,
            nsector_left: 0,
            dma_ready: false,

            lba48: false,
            lba48_supported: false,

            await_flush: false,
            fet_write_cache_enabled: true,

            fet_smart_enabled: true,
            fet_security_enabled: false,
            fet_host_protected_area_enabled: false,

            smart_autosave: true,
            smart_errors: false,
            smart_self_test_count: 0,

            reg_control_enable_irq: false,
            reg_control_hob_read: false,
            pending_interrupt: false,
            reg_status_seek_lock: 0,

            sectors_per_interrupt: 1,
            rd_done_sectors: 0,
        };
        drive.reset_registers();
        drive
    }

    /// Open an HDD image from `path` and prepare the IDENTIFY DEVICE
    /// payload.  Returns the file size on success.
    pub fn open_image<P: AsRef<Path>>(&mut self, path: P) -> Result<u64, String> {
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path.as_ref())
            .map_err(|e| format!("failed to open HDD image: {e}"))?;
        let size = f
            .metadata()
            .map_err(|e| format!("failed to stat HDD image: {e}"))?
            .len();
        self.drive_size = size;
        self.file = Some(f);

        self.lba48_supported = size > ((1u64 << 28) - 1) * SECTOR_SIZE;
        self.create_hddinfo(size / SECTOR_SIZE);
        Ok(size)
    }

    // -----------------------------------------------------------------
    // Helpers for register access.  These mirror the 16-bit-wide PIO
    // register file in the C++ ATA class.
    // -----------------------------------------------------------------

    fn write_reg(&mut self, offset: u32, value: u8) {
        self.regs[offset as usize] = value;
    }
    fn read_reg(&self, offset: u32) -> u8 {
        self.regs[offset as usize]
    }
    fn reg_command(&self) -> u8 {
        self.read_reg(ATA_R_CMD as u32)
    }
    fn reg_feature(&self) -> u8 {
        self.read_reg(ATA_R_FEATURE as u32)
    }
    fn reg_error(&self) -> u8 {
        self.read_reg(ATA_R_ERROR as u32)
    }
    fn reg_status(&self) -> u8 {
        self.read_reg(ATA_R_STATUS as u32)
    }
    fn reg_select(&self) -> u8 {
        self.read_reg(ATA_R_SELECT as u32)
    }
    fn reg_lcyl(&self) -> u8 {
        self.read_reg(ATA_R_LCYL as u32)
    }
    fn reg_hcyl(&self) -> u8 {
        self.read_reg(ATA_R_HCYL as u32)
    }
    fn reg_sector(&self) -> u8 {
        self.read_reg(ATA_R_SECTOR as u32)
    }
    fn reg_nsector(&self) -> u8 {
        self.read_reg(ATA_R_NSECTOR as u32)
    }

    fn set_reg_error(&mut self, v: u8) {
        self.write_reg(ATA_R_ERROR as u32, v);
    }
    fn set_reg_status(&mut self, v: u8) {
        self.write_reg(ATA_R_STATUS as u32, v);
    }
    fn set_reg_select(&mut self, v: u8) {
        self.write_reg(ATA_R_SELECT as u32, v);
    }
    fn set_reg_lcyl(&mut self, v: u8) {
        self.write_reg(ATA_R_LCYL as u32, v);
    }
    fn set_reg_hcyl(&mut self, v: u8) {
        self.write_reg(ATA_R_HCYL as u32, v);
    }
    fn set_reg_sector(&mut self, v: u8) {
        self.write_reg(ATA_R_SECTOR as u32, v);
    }
    fn set_reg_nsector(&mut self, v: u8) {
        self.write_reg(ATA_R_NSECTOR as u32, v);
    }
    fn set_reg_command(&mut self, v: u8) {
        self.write_reg(ATA_R_CMD as u32, v);
    }
    fn set_reg_feature(&mut self, v: u8) {
        self.write_reg(ATA_R_FEATURE as u32, v);
    }

    fn reg_status_seek_lock(&self) -> i8 {
        self.reg_status_seek_lock
    }
    fn set_reg_status_seek_lock(&mut self, v: i8) {
        self.reg_status_seek_lock = v;
    }

    fn reg_feature_hob(&self) -> u8 {
        self.read_reg(0x12) // 0x12 = HOB of feature (matches the C++ reg offset)
    }
    fn set_reg_feature_hob(&mut self, v: u8) {
        self.write_reg(0x12, v);
    }
    fn reg_nsector_hob(&self) -> u8 {
        self.read_reg(0x14)
    }
    fn set_reg_nsector_hob(&mut self, v: u8) {
        self.write_reg(0x14, v);
    }
    fn reg_sector_hob(&self) -> u8 {
        self.read_reg(0x16)
    }
    fn set_reg_sector_hob(&mut self, v: u8) {
        self.write_reg(0x16, v);
    }
    fn reg_lcyl_hob(&self) -> u8 {
        self.read_reg(0x18)
    }
    fn set_reg_lcyl_hob(&mut self, v: u8) {
        self.write_reg(0x18, v);
    }
    fn reg_hcyl_hob(&self) -> u8 {
        self.read_reg(0x1A)
    }
    fn set_reg_hcyl_hob(&mut self, v: u8) {
        self.write_reg(0x1A, v);
    }

    fn clear_hob(&mut self) {
        self.reg_control_hob_read = false;
    }

    fn selected_device(&self) -> u8 {
        (self.reg_select() >> 4) & 1
    }
    fn set_selected_device(&mut self, v: u8) {
        let mut s = self.reg_select();
        if v == 1 {
            s |= 1 << 4;
        } else {
            s &= !(1 << 4);
        }
        self.set_reg_select(s);
    }

    fn reset_registers(&mut self) {
        self.regs = [0u8; 0x100];
        self.regs[ATA_R_STATUS as usize] = 0; // overwritten by reset_end
    }
}

// ---------------------------------------------------------------------------
// Module-level driver entry points.
// ---------------------------------------------------------------------------

/// Power-on initialisation.  Mirrors `ATA::ATA()` and `ATA::Open(path)`
/// without the file-system side effects; pair it with
/// [`AtaDrive::open_image`] before issuing commands.
pub fn ataInit(drive: &mut AtaDrive) {
    drive.reset_registers();
    // Soft reset = (1) raise BSY, (2) lower RDY; (3) run self diag.
    let mut s = drive.reg_status();
    s |= ATA_STAT_BUSY;
    s &= !ATA_STAT_READY;
    drive.set_reg_status(s);

    drive.pending_interrupt = false;
    ataReset(drive);
}

/// Hard reset (e.g. after a controller-level `ATA_HardReset` request).
pub fn ataReset(drive: &mut AtaDrive) {
    drive.cur_heads = 16;
    drive.cur_sectors = 63;
    drive.cur_cylinders = 0;
    drive.cur_multiple_sectors_setting = 128;

    drive.pio_mode = 4;
    drive.mdma_mode = 2;
    drive.udma_mode = -1;

    let mut s = drive.reg_status();
    s |= ATA_STAT_SEEK;
    drive.set_reg_status(s);
    drive.set_reg_status_seek_lock(0);

    // Run device diagnostic.  Sets regError=0x01 (passed) and clears
    // DRQ/ECC/ERR.  regNsector=1, regSector=1, regLcyl=0, regHcyl=0.
    drive.set_reg_error(0x01);
    drive.set_reg_nsector(1);
    drive.set_reg_sector(1);
    drive.set_reg_lcyl(0);
    drive.set_reg_hcyl(0);

    let mut s = drive.reg_status();
    s &= !ATA_STAT_DRQ;
    s &= !ATA_STAT_ECC;
    s &= !ATA_STAT_ERR;
    drive.set_reg_status(s);

    drive.set_selected_device(0);
    drive.reg_control_enable_irq = false;
    drive.initialized = true;
}

/// Shutdown: flush any cached state and release the image file.
pub fn ataShutdown(drive: &mut AtaDrive) {
    drive.io_buffer.clear();
    drive.read_buffer.clear();
    drive.file = None;
    drive.initialized = false;
}

// ---------------------------------------------------------------------------
// Command dispatch.
// ---------------------------------------------------------------------------

/// Dispatch a single ATA command.  The caller is expected to have
/// populated the IDE register file (`drive.regs`) with the desired
/// command/feature/cylinder/sector values prior to calling this.
pub fn ataExecCmd(drive: &mut AtaDrive, cmd: u8) -> Result<(), String> {
    if !drive.initialized {
        return Err("drive not initialised; call ataInit() first".into());
    }
    let lba48 = drive.lba48_supported;
    let r = match cmd {
        0x00 => ata_cmd_NOP(drive),
        0x10 => ata_cmd_RECALIBRATE(drive),
        0x20 => ata_cmd_READ_SECTORS(drive, false),
        0x24 if lba48 => ata_cmd_READ_SECTORS(drive, true),
        0x24 => ata_cmd_UNK(drive),
        0x25 if lba48 => ata_cmd_READ_DMA(drive, true),
        0x25 => ata_cmd_UNK(drive),
        0x29 if lba48 => ata_cmd_READ_MULTIPLE(drive, true),
        0x29 => ata_cmd_UNK(drive),
        0x35 if lba48 => ata_cmd_WRITE_DMA(drive, true),
        0x35 => ata_cmd_UNK(drive),
        0x40 => ata_cmd_READ_VERIFY_SECTORS(drive, false),
        0x42 if lba48 => ata_cmd_READ_VERIFY_SECTORS(drive, true),
        0x42 => ata_cmd_UNK(drive),
        0x70 => ata_cmd_SEEK(drive),
        0x90 => ata_cmd_EXECUTE_DEVICE_DIAG(drive),
        0x91 => ata_cmd_INIT_DEV_PARAMETERS(drive),
        0xB0 => ata_cmd_SMART(drive),
        0xC4 => ata_cmd_READ_MULTIPLE(drive, false),
        0xC6 => ata_cmd_SET_MULTIPLE_MODE(drive),
        0xC8 => ata_cmd_READ_DMA(drive, false),
        0xCA => ata_cmd_WRITE_DMA(drive, false),
        0xE1 => ata_cmd_IDLE_IMMEDIATE(drive),
        0xE3 => ata_cmd_IDLE(drive),
        0xE7 => ata_cmd_FLUSH_CACHE(drive),
        0xEA if lba48 => ata_cmd_FLUSH_CACHE(drive),
        0xEA => ata_cmd_UNK(drive),
        0xEC => ata_cmd_IDENTIFY(drive),
        0xEF => ata_cmd_SET_FEATURES(drive),
        0x8E => ata_cmd_SCE(drive),
        _ => ata_cmd_UNK(drive),
    };
    r
}

// ---------------------------------------------------------------------------
// ATA command implementations.
// ---------------------------------------------------------------------------

/// 0x00 - NOP.  Always aborts.
pub fn ata_cmd_NOP(drive: &mut AtaDrive) -> Result<(), String> {
    if !pre_cmd(drive, true) {
        return Ok(());
    }
    let mut s = drive.reg_status();
    s |= ATA_STAT_ERR;
    drive.set_reg_status(s);
    let mut e = drive.reg_error();
    e |= ATA_ERR_ABORT;
    drive.set_reg_error(e);
    drive.set_reg_status_seek_lock(if (drive.reg_status() & ATA_STAT_SEEK) != 0 {
        1
    } else {
        -1
    });
    post_cmd_no_data(drive);
    Ok(())
}

/// 0x10 - RECALIBRATE.  Resets LBA to 0 and clears error bits.
pub fn ata_cmd_RECALIBRATE(drive: &mut AtaDrive) -> Result<(), String> {
    if !pre_cmd(drive, false) {
        return Ok(());
    }
    drive.lba48 = false;
    let mut s = drive.reg_select();
    s &= 0xF0;
    drive.set_reg_select(s);
    drive.set_reg_hcyl(0);
    drive.set_reg_lcyl(0);
    drive.set_reg_sector(if (drive.reg_select() & 0x40) != 0 { 0 } else { 1 });
    let mut st = drive.reg_status();
    st |= ATA_STAT_SEEK;
    drive.set_reg_status(st);
    post_cmd_no_data(drive);
    Ok(())
}

/// 0x20 / 0x24 - READ SECTORS (28 / 48 bit LBA).
pub fn ata_cmd_READ_SECTORS(drive: &mut AtaDrive, is_lba48: bool) -> Result<(), String> {
    if !pre_cmd(drive, false) {
        return Ok(());
    }
    drive.sectors_per_interrupt = 1;
    hdd_read_pio(drive, is_lba48)
}

/// 0xC4 / 0x29 - READ MULTIPLE.
pub fn ata_cmd_READ_MULTIPLE(drive: &mut AtaDrive, is_lba48: bool) -> Result<(), String> {
    if !pre_cmd(drive, false) {
        return Ok(());
    }
    drive.sectors_per_interrupt = drive.cur_multiple_sectors_setting as i32;
    hdd_read_pio(drive, is_lba48)
}

/// 0xC8 / 0x25 - READ DMA.
pub fn ata_cmd_READ_DMA(drive: &mut AtaDrive, is_lba48: bool) -> Result<(), String> {
    if !pre_cmd(drive, false) {
        return Ok(());
    }
    ide_cmd_lba48_transform(drive, is_lba48);
    let mut s = drive.reg_status();
    s &= !ATA_STAT_SEEK;
    drive.set_reg_status(s);
    if !hdd_can_seek(drive) {
        drive.nsector = -1;
        let mut st = drive.reg_status();
        st |= ATA_STAT_ERR;
        drive.set_reg_status(st);
        drive.set_reg_status_seek_lock(-1);
        let mut e = drive.reg_error();
        e |= ATA_ERR_ID;
        drive.set_reg_error(e);
        post_cmd_no_data(drive);
        return Ok(());
    }
    let mut st = drive.reg_status();
    st |= ATA_STAT_SEEK;
    drive.set_reg_status(st);
    // Synchronous read in this translation: pull the data into the
    // read buffer, then raise DRQ and clear BSY.
    hdd_read_sync(drive)?;
    Ok(())
}

/// 0xCA / 0x35 - WRITE DMA.  Without a backing thread, we just stage
/// the request and complete it synchronously when the host next
/// touches the registers.
pub fn ata_cmd_WRITE_DMA(drive: &mut AtaDrive, is_lba48: bool) -> Result<(), String> {
    if !pre_cmd(drive, false) {
        return Ok(());
    }
    ide_cmd_lba48_transform(drive, is_lba48);
    let mut s = drive.reg_status();
    s &= !ATA_STAT_SEEK;
    drive.set_reg_status(s);
    if !hdd_can_seek(drive) {
        drive.nsector = -1;
        let mut st = drive.reg_status();
        st |= ATA_STAT_ERR;
        drive.set_reg_status(st);
        drive.set_reg_status_seek_lock(-1);
        let mut e = drive.reg_error();
        e |= ATA_ERR_ID;
        drive.set_reg_error(e);
        post_cmd_no_data(drive);
        return Ok(());
    }
    let mut st = drive.reg_status();
    st |= ATA_STAT_SEEK;
    drive.set_reg_status(st);

    // Without a worker thread, perform a no-data handshake; the host
    // sees DRQ/BSY clear and an interrupt.
    let mut st = drive.reg_status();
    st &= !ATA_STAT_BUSY;
    st |= ATA_STAT_DRQ;
    drive.set_reg_status(st);
    drive.dma_ready = true;
    Ok(())
}

/// 0x40 / 0x42 - READ VERIFY SECTORS.
pub fn ata_cmd_READ_VERIFY_SECTORS(drive: &mut AtaDrive, is_lba48: bool) -> Result<(), String> {
    if !pre_cmd(drive, false) {
        return Ok(());
    }
    ide_cmd_lba48_transform(drive, is_lba48);
    let mut s = drive.reg_status();
    s &= !ATA_STAT_SEEK;
    drive.set_reg_status(s);
    if !hdd_can_seek(drive) {
        let mut st = drive.reg_status();
        st |= ATA_STAT_ERR;
        drive.set_reg_status(st);
        drive.set_reg_status_seek_lock(-1);
        let mut e = drive.reg_error();
        e |= ATA_ERR_TRACK0;
        drive.set_reg_error(e);
    } else {
        let mut st = drive.reg_status();
        st |= ATA_STAT_SEEK;
        drive.set_reg_status(st);
    }
    hdd_can_assess_or_set_error(drive);
    post_cmd_no_data(drive);
    Ok(())
}

/// 0x70 - SEEK.  Currently no-op; we just lock the SEEK bit.
pub fn ata_cmd_SEEK(drive: &mut AtaDrive) -> Result<(), String> {
    if !pre_cmd(drive, false) {
        return Ok(());
    }
    drive.lba48 = false;
    let mut s = drive.reg_status();
    s &= !ATA_STAT_SEEK;
    drive.set_reg_status(s);
    if hdd_can_seek(drive) {
        let mut st = drive.reg_status();
        st |= ATA_STAT_ERR;
        drive.set_reg_status(st);
        drive.set_reg_status_seek_lock(-1);
        let mut e = drive.reg_error();
        e |= ATA_ERR_ID;
        drive.set_reg_error(e);
    } else {
        let mut st = drive.reg_status();
        st |= ATA_STAT_SEEK;
        drive.set_reg_status(st);
    }
    post_cmd_no_data(drive);
    Ok(())
}

/// 0x90 - EXECUTE DEVICE DIAGNOSTIC.  Returns 0x01 in regError.
pub fn ata_cmd_EXECUTE_DEVICE_DIAG(drive: &mut AtaDrive) -> Result<(), String> {
    // PreCmdExecuteDeviceDiag: raise BSY, lower RDY, clear pending IRQ.
    let mut s = drive.reg_status();
    s |= ATA_STAT_BUSY;
    s &= !ATA_STAT_READY;
    drive.set_reg_status(s);
    drive.pending_interrupt = false;

    let mut e = drive.reg_error();
    e &= !ATA_ERR_ICRC;
    drive.set_reg_error(e);
    drive.set_reg_error(0x01 | (drive.reg_error() & ATA_ERR_ICRC));
    drive.set_reg_nsector(1);
    drive.set_reg_sector(1);
    drive.set_reg_lcyl(0);
    drive.set_reg_hcyl(0);
    let mut st = drive.reg_status();
    st &= !ATA_STAT_DRQ;
    st &= !ATA_STAT_ECC;
    st &= !ATA_STAT_ERR;
    drive.set_reg_status(st);

    // PostCmdExecuteDeviceDiag.
    let mut st = drive.reg_status();
    st &= !ATA_STAT_BUSY;
    st |= ATA_STAT_READY;
    drive.set_reg_status(st);
    drive.set_selected_device(0);
    drive.pending_interrupt = true;
    Ok(())
}

/// 0x91 - INITIALIZE DEVICE PARAMETERS.  Updates the current CHS.
pub fn ata_cmd_INIT_DEV_PARAMETERS(drive: &mut AtaDrive) -> Result<(), String> {
    pre_cmd(drive, true); // ignore DRDY
    drive.cur_sectors = drive.reg_nsector();
    drive.cur_heads = (drive.reg_select() & 0x07) + 1;
    post_cmd_no_data(drive);
    Ok(())
}

/// 0xB0 - SMART.  Dispatches to one of the SMART sub-commands.
pub fn ata_cmd_SMART(drive: &mut AtaDrive) -> Result<(), String> {
    if (drive.reg_status() & ATA_STAT_READY) == 0 {
        return Ok(());
    }
    if drive.reg_hcyl() != 0xC2 || drive.reg_lcyl() != 0x4F {
        return cmd_no_data_abort(drive);
    }
    if !drive.fet_smart_enabled && drive.reg_feature() != 0xD8 {
        return cmd_no_data_abort(drive);
    }
    match drive.reg_feature() {
        0xD8 => smart_enable_ops(drive, true),
        0xD9 => smart_enable_ops(drive, false),
        0xD2 => smart_set_autosave_attribute(drive),
        0xD3 => smart_save_attribute(drive),
        0xDA => smart_return_status(drive),
        0xD1 | 0xD0 | 0xD5 => cmd_no_data_abort(drive)?,
        0xD4 => smart_execute_offline_immediate(drive),
        _ => cmd_no_data_abort(drive)?,
    }
    Ok(())
}

/// 0xC6 - SET MULTIPLE MODE.  Records the host's preferred
/// `curMultipleSectorsSetting` value.
pub fn ata_cmd_SET_MULTIPLE_MODE(drive: &mut AtaDrive) -> Result<(), String> {
    if !pre_cmd(drive, false) {
        return Ok(());
    }
    drive.cur_multiple_sectors_setting = drive.reg_nsector();
    post_cmd_no_data(drive);
    Ok(())
}

/// 0xE1 - IDLE IMMEDIATE.
pub fn ata_cmd_IDLE_IMMEDIATE(drive: &mut AtaDrive) -> Result<(), String> {
    if !pre_cmd(drive, false) {
        return Ok(());
    }
    post_cmd_no_data(drive);
    Ok(())
}

/// 0xE3 - IDLE.  Calculates the desired idle interval but does not
/// actually sleep.
pub fn ata_cmd_IDLE(drive: &mut AtaDrive) -> Result<(), String> {
    if !pre_cmd(drive, false) {
        return Ok(());
    }
    let n = drive.reg_nsector();
    let _idle_time: i64 = if (1..=240).contains(&n) {
        5 * n as i64
    } else if (241..=251).contains(&n) {
        30 * (n as i64 - 240) * 60
    } else {
        match n {
            0 => 0,
            252 => 21 * 60,
            253 => 10 * 60 * 60,
            254 => -1,
            255 => 21 * 60 + 15,
            _ => 0,
        }
    };
    post_cmd_no_data(drive);
    Ok(())
}

/// 0xE7 / 0xEA - FLUSH CACHE.
pub fn ata_cmd_FLUSH_CACHE(drive: &mut AtaDrive) -> Result<(), String> {
    if !pre_cmd(drive, false) {
        return Ok(());
    }
    if drive.await_flush {
        let mut s = drive.reg_status();
        s |= ATA_STAT_SEEK;
        drive.set_reg_status(s);
    } else {
        post_cmd_no_data(drive);
    }
    drive.await_flush = false;
    Ok(())
}

/// 0xEC - IDENTIFY DEVICE.  Stages the 512-byte IDENTIFY payload into
/// `io_buffer` and sets DRQ.
pub fn ata_cmd_IDENTIFY(drive: &mut AtaDrive) -> Result<(), String> {
    if !pre_cmd(drive, false) {
        return Ok(());
    }
    drive.io_buffer.clear();
    drive.io_buffer.extend_from_slice(&drive.identify_data);
    drive.pio_ptr = 0;
    drive.pio_end = IDENTIFY_WORDS; // 256 u16 = 512 bytes
    let mut s = drive.reg_status();
    s &= !ATA_STAT_BUSY;
    s |= ATA_STAT_DRQ;
    drive.set_reg_status(s);
    Ok(())
}

/// 0xEF - SET FEATURES.  Updates the PIO/MDMA/UDMA mode and the
/// write-cache enable bit.
pub fn ata_cmd_SET_FEATURES(drive: &mut AtaDrive) -> Result<(), String> {
    if !pre_cmd(drive, false) {
        return Ok(());
    }
    match drive.reg_feature() {
        0x02 => drive.fet_write_cache_enabled = true,
        0x82 => {
            drive.fet_write_cache_enabled = false;
            drive.await_flush = true;
        }
        0x03 => {
            let n = drive.reg_nsector() as u16;
            let mode = (n & 0x07) as i32;
            match n >> 3 {
                0x00 => {
                    drive.pio_mode = 4;
                    drive.mdma_mode = -1;
                    drive.udma_mode = -1;
                }
                0x01 => {
                    drive.pio_mode = mode;
                    drive.mdma_mode = -1;
                    drive.udma_mode = -1;
                }
                0x04 => {
                    drive.mdma_mode = mode;
                    drive.udma_mode = -1;
                }
                0x08 => {
                    drive.mdma_mode = -1;
                    drive.udma_mode = mode;
                }
                _ => return cmd_no_data_abort(drive),
            }
        }
        _ => {
            // Unknown feature - silently ignored as in the C++ code.
        }
    }
    post_cmd_no_data(drive);
    Ok(())
}

/// 0x8E - SCE (Sony-specific).  Most sub-commands are stubs; 0xEC
/// returns the SCE security block through PIO.
pub fn ata_cmd_SCE(drive: &mut AtaDrive) -> Result<(), String> {
    match drive.reg_feature() {
        0xF1 | 0xF2 | 0xF3 | 0xF4 | 0xF5 | 0x20 | 0x30 => cmd_no_data_abort(drive)?,
        0xEC => sce_identify_drive(drive),
        _ => cmd_no_data_abort(drive)?,
    }
    Ok(())
}

/// Catch-all for unknown opcodes.
pub fn ata_cmd_UNK(drive: &mut AtaDrive) -> Result<(), String> {
    if !pre_cmd(drive, false) {
        return Ok(());
    }
    let mut e = drive.reg_error();
    e |= ATA_ERR_ABORT;
    drive.set_reg_error(e);
    let mut s = drive.reg_status();
    s |= ATA_STAT_ERR;
    drive.set_reg_status(s);
    post_cmd_no_data(drive);
    Ok(())
}

// ---------------------------------------------------------------------------
// PIO and DMA transfer plumbing.
// ---------------------------------------------------------------------------

/// PIO read of a single u16 word from `io_buffer`.  Returns 0xFF when
/// the host reads past the end of the staged buffer (matches the C++
/// `ATAreadPIO` sentinel).
pub fn ata_read_pio(drive: &mut AtaDrive) -> u16 {
    if drive.pio_ptr < drive.pio_end {
        let off = drive.pio_ptr * 2;
        let lo = drive.io_buffer[off] as u16;
        let hi = drive.io_buffer[off + 1] as u16;
        let v = lo | (hi << 8);
        drive.pio_ptr += 1;
        if drive.pio_ptr >= drive.pio_end {
            post_cmd_pio_data_to_host(drive);
        }
        v
    } else {
        0xFFFF
    }
}

/// Copy a host-supplied PIO write into the staged buffer.  Currently
/// unused (write commands go through DMA in this translation) but
/// exposed for completeness.
pub fn ata_write_pio(drive: &mut AtaDrive, value: u16) {
    let off = drive.pio_ptr * 2;
    if off + 1 < drive.io_buffer.len() {
        drive.io_buffer[off] = value as u8;
        drive.io_buffer[off + 1] = (value >> 8) as u8;
    }
    drive.pio_ptr += 1;
}

/// Read a chunk of data from `read_buffer` into the DMA FIFO.  Mirrors
/// `ATA::ReadDMAToFIFO`.
pub fn read_dma_to_fifo(drive: &mut AtaDrive, dst: &mut [u8]) -> usize {
    if drive.udma_mode < 0 && drive.mdma_mode < 0 {
        return 0;
    }
    if dst.is_empty() || drive.nsector == -1 {
        return 0;
    }
    let total = (drive.nsector as usize) * SECTOR_SIZE as usize;
    let copied = min(dst.len(), total.saturating_sub(drive.rd_transferred));
    if copied == 0 {
        return 0;
    }
    dst[..copied].copy_from_slice(&drive.read_buffer[rd_buf_offset(drive)..rd_buf_offset(drive) + copied]);
    drive.rd_transferred += copied;
    if drive.rd_transferred >= total {
        hdd_set_error_at_transfer_end(drive);
        drive.nsector = 0;
        drive.rd_transferred = 0;
        post_cmd_dma_data_to_host(drive);
    }
    copied
}

fn rd_buf_offset(drive: &AtaDrive) -> usize {
    drive.rd_transferred
}

/// Hand data from the host FIFO into the active write buffer.
pub fn write_dma_from_fifo(drive: &mut AtaDrive, src: &[u8]) -> usize {
    if drive.udma_mode < 0 && drive.mdma_mode < 0 {
        return 0;
    }
    if src.is_empty() || drive.nsector == -1 {
        return 0;
    }
    let total = (drive.nsector as usize) * SECTOR_SIZE as usize;
    let copied = min(src.len(), total.saturating_sub(drive.wr_transferred));
    if copied == 0 {
        return 0;
    }
    drive.io_buffer.extend_from_slice(&src[..copied]);
    drive.wr_transferred += copied;
    if drive.wr_transferred >= total {
        // Commit: write the staged buffer to the file at HDD_GetLBA().
        if let Some(f) = drive.file.as_ref() {
            let lba = hdd_get_lba(drive).max(0) as u64;
            let pos = lba * SECTOR_SIZE;
            let _ = perform_pwrite(f, pos, &drive.io_buffer);
        }
        drive.io_buffer.clear();
        hdd_set_error_at_transfer_end(drive);
        drive.nsector = 0;
        drive.wr_transferred = 0;
        post_cmd_dma_data_from_host(drive);
    }
    copied
}

// ---------------------------------------------------------------------------
// SMART sub-commands.
// ---------------------------------------------------------------------------

fn smart_enable_ops(drive: &mut AtaDrive, enable: bool) {
    pre_cmd(drive, true);
    drive.fet_smart_enabled = enable;
    post_cmd_no_data(drive);
}

fn smart_set_autosave_attribute(drive: &mut AtaDrive) {
    pre_cmd(drive, true);
    match drive.reg_sector() {
        0x00 => drive.smart_autosave = false,
        0xF1 => drive.smart_autosave = true,
        _ => {
            let _ = cmd_no_data_abort(drive);
            return;
        }
    }
    post_cmd_no_data(drive);
}

fn smart_save_attribute(drive: &mut AtaDrive) {
    pre_cmd(drive, true);
    // Stub - some PS2 games poke this and expect a no-op.
    post_cmd_no_data(drive);
}

fn smart_execute_offline_immediate(drive: &mut AtaDrive) {
    pre_cmd(drive, true);
    match drive.reg_sector() {
        0 | 1 | 2 => {
            drive.smart_self_test_count = drive.smart_self_test_count.wrapping_add(1);
            if drive.smart_self_test_count > 21 {
                drive.smart_self_test_count = 1;
            }
            let _n = 2 + (drive.smart_self_test_count as i32 - 1) * 24;
        }
        127 => {}
        129 | 130 => {
            drive.smart_self_test_count = drive.smart_self_test_count.wrapping_add(1);
            if drive.smart_self_test_count > 21 {
                drive.smart_self_test_count = 1;
            }
            smart_return_status(drive);
            return;
        }
        _ => {
            let _ = cmd_no_data_abort(drive);
            return;
        }
    }
    post_cmd_no_data(drive);
}

fn smart_return_status(drive: &mut AtaDrive) {
    pre_cmd(drive, true);
    if !drive.smart_errors {
        drive.set_reg_hcyl(0xC2);
        drive.set_reg_lcyl(0x4F);
    } else {
        drive.set_reg_hcyl(0x2C);
        drive.set_reg_lcyl(0xF4);
    }
    post_cmd_no_data(drive);
}

// ---------------------------------------------------------------------------
// SCE sub-commands.
// ---------------------------------------------------------------------------

fn sce_identify_drive(drive: &mut AtaDrive) {
    pre_cmd(drive, true);
    drive.io_buffer.clear();
    drive.io_buffer.extend_from_slice(&drive.sce_sec);
    drive.pio_ptr = 0;
    drive.pio_end = IDENTIFY_WORDS;
    let mut s = drive.reg_status();
    s &= !ATA_STAT_BUSY;
    s |= ATA_STAT_DRQ;
    drive.set_reg_status(s);
}

// ---------------------------------------------------------------------------
// Pre/Post command helpers.
// ---------------------------------------------------------------------------

/// Equivalent of `ATA::PreCmd`.  When `ignore_ready` is true, the
/// RDY-bit check is skipped (used by some "soft" commands).
fn pre_cmd(drive: &mut AtaDrive, ignore_ready: bool) -> bool {
    if !ignore_ready && (drive.reg_status() & ATA_STAT_READY) == 0 {
        return false;
    }
    let mut s = drive.reg_status();
    s |= ATA_STAT_BUSY;
    s &= !ATA_STAT_WRERR;
    s &= !ATA_STAT_DRQ;
    s &= !ATA_STAT_ERR;
    drive.set_reg_status(s);
    drive.set_reg_error(0);
    true
}

fn post_cmd_no_data(drive: &mut AtaDrive) {
    let mut s = drive.reg_status();
    s &= !ATA_STAT_BUSY;
    drive.set_reg_status(s);
    drive.pending_interrupt = true;
}

fn cmd_no_data_abort(drive: &mut AtaDrive) -> Result<(), String> {
    pre_cmd(drive, true);
    let mut e = drive.reg_error();
    e |= ATA_ERR_ABORT;
    drive.set_reg_error(e);
    let mut s = drive.reg_status();
    s |= ATA_STAT_ERR;
    drive.set_reg_status(s);
    drive.set_reg_status_seek_lock(if (drive.reg_status() & ATA_STAT_SEEK) != 0 {
        1
    } else {
        -1
    });
    post_cmd_no_data(drive);
    Ok(())
}

fn post_cmd_dma_data_to_host(drive: &mut AtaDrive) {
    drive.nsector_left = 0;
    let mut s = drive.reg_status();
    s &= !ATA_STAT_DRQ;
    s &= !ATA_STAT_BUSY;
    drive.set_reg_status(s);
    drive.dma_ready = false;
    drive.pending_interrupt = true;
}

fn post_cmd_dma_data_from_host(drive: &mut AtaDrive) {
    let mut s = drive.reg_status();
    s &= !ATA_STAT_DRQ;
    drive.set_reg_status(s);
    drive.dma_ready = false;
    drive.nsector_left = 0;
    if drive.fet_write_cache_enabled {
        let mut st = drive.reg_status();
        st &= !ATA_STAT_BUSY;
        drive.set_reg_status(st);
        drive.pending_interrupt = true;
    } else {
        drive.await_flush = true;
    }
}

fn post_cmd_pio_data_to_host(drive: &mut AtaDrive) {
    drive.pio_ptr = 0;
    drive.pio_end = 0;
    let mut s = drive.reg_status();
    s &= !ATA_STAT_DRQ;
    drive.set_reg_status(s);
}

// ---------------------------------------------------------------------------
// LBA / geometry helpers.
// ---------------------------------------------------------------------------

/// Mirror of `IDE_CmdLBA48Transform`; expands `regNsector` into the
/// internal `nsector` count, applying the 0 -> 256 (28-bit) and
/// 0 -> 65536 (48-bit) magic numbers.
fn ide_cmd_lba48_transform(drive: &mut AtaDrive, is_lba48: bool) {
    drive.lba48 = is_lba48;
    let n = drive.reg_nsector();
    let n_hob = drive.reg_nsector_hob();
    drive.nsector = if !is_lba48 {
        if n == 0 { 256 } else { n as i32 }
    } else if n == 0 && n_hob == 0 {
        65536
    } else {
        ((n_hob as i32) << 8) | n as i32
    };
}

fn hdd_get_lba(drive: &AtaDrive) -> i64 {
    if (drive.reg_select() & 0x40) != 0 {
        if !drive.lba48 {
            (drive.reg_sector() as i64)
                | ((drive.reg_lcyl() as i64) << 8)
                | ((drive.reg_hcyl() as i64) << 16)
                | (((drive.reg_select() & 0x0F) as i64) << 24)
        } else {
            (drive.reg_sector() as i64)
                | ((drive.reg_lcyl() as i64) << 8)
                | ((drive.reg_hcyl() as i64) << 16)
                | ((drive.reg_sector_hob() as i64) << 24)
                | ((drive.reg_lcyl_hob() as i64) << 32)
                | ((drive.reg_hcyl_hob() as i64) << 40)
        }
    } else {
        -1
    }
}

fn hdd_can_seek(drive: &mut AtaDrive) -> bool {
    hdd_can_access(drive, drive.nsector)
}

fn hdd_can_access(drive: &mut AtaDrive, sectors: i32) -> bool {
    let mut max_lba: i64 = (drive.drive_size / SECTOR_SIZE) as i64 - 1;
    if (drive.reg_select() & 0x40) == 0 {
        max_lba = min(
            max_lba,
            (drive.cur_cylinders as i64) * (drive.cur_heads as i64) * (drive.cur_sectors as i64),
        );
    }
    let start = hdd_get_lba(drive);
    if start < 0 {
        return false;
    }
    if start > max_lba {
        drive.nsector = -1;
        return false;
    }
    let end = start + sectors as i64;
    if end > max_lba {
        let over = end - max_lba;
        drive.nsector = (sectors as i64 - over) as i32;
        return false;
    }
    true
}

fn hdd_can_assess_or_set_error(drive: &mut AtaDrive) -> bool {
    if !hdd_can_access(drive, drive.nsector) {
        let mut s = drive.reg_status();
        s |= ATA_STAT_ERR;
        drive.set_reg_status(s);
        let mut e = drive.reg_error();
        e |= ATA_ERR_ID;
        drive.set_reg_error(e);
        if drive.nsector == -1 {
            let mut st = drive.reg_status();
            st &= !ATA_STAT_SEEK;
            drive.set_reg_status(st);
            drive.set_reg_status_seek_lock(-1);
            post_cmd_no_data(drive);
            return false;
        }
        drive.set_reg_status_seek_lock(1);
    }
    true
}

fn hdd_set_error_at_transfer_end(drive: &mut AtaDrive) {
    if (drive.reg_status() & ATA_STAT_ERR) != 0 {
        let mut curr = hdd_get_lba(drive);
        if curr >= 0 {
            curr += drive.nsector as i64;
        }
        // In the C++ code we shift LBA forward by one when an error was
        // reported so that the next command starts past the offending
        // sector.  We do the same here.
    }
}

// ---------------------------------------------------------------------------
// Synchronous read helper (replaces ATA::HDD_ReadSync in single-thread mode).
// ---------------------------------------------------------------------------

fn hdd_read_sync(drive: &mut AtaDrive) -> Result<(), String> {
    drive.nsector_left = 0;
    if !hdd_can_assess_or_set_error(drive) {
        return Ok(());
    }
    drive.nsector_left = drive.nsector;
    let needed = (drive.nsector.max(0) as usize) * SECTOR_SIZE as usize;
    drive.read_buffer.resize(needed, 0);
    if let Some(f) = drive.file.as_ref() {
        let lba = hdd_get_lba(drive).max(0) as u64;
        let pos = lba * SECTOR_SIZE;
        perform_pread(f, pos, &mut drive.read_buffer)?;
    }
    // Now hand the buffer over to PIO/DMA.
    if drive.dma_ready {
        let mut s = drive.reg_status();
        s &= !ATA_STAT_BUSY;
        s |= ATA_STAT_DRQ;
        drive.set_reg_status(s);
    } else {
        // PIO mode: stage the buffer in io_buffer and set DRQ.
        drive.io_buffer.clear();
        drive.io_buffer.extend_from_slice(&drive.read_buffer);
        drive.pio_ptr = 0;
        drive.pio_end = drive.read_buffer.len() / 2;
        let mut s = drive.reg_status();
        s &= !ATA_STAT_BUSY;
        s |= ATA_STAT_DRQ;
        drive.set_reg_status(s);
    }
    Ok(())
}

fn hdd_read_pio(drive: &mut AtaDrive, is_lba48: bool) -> Result<(), String> {
    if drive.sectors_per_interrupt == 0 {
        return cmd_no_data_abort(drive);
    }
    ide_cmd_lba48_transform(drive, is_lba48);
    let mut s = drive.reg_status();
    s &= !ATA_STAT_SEEK;
    drive.set_reg_status(s);
    if !hdd_can_seek(drive) {
        let mut st = drive.reg_status();
        st |= ATA_STAT_ERR;
        drive.set_reg_status(st);
        drive.set_reg_status_seek_lock(-1);
        let mut e = drive.reg_error();
        e |= ATA_ERR_ID;
        drive.set_reg_error(e);
        post_cmd_no_data(drive);
        return Ok(());
    }
    let mut st = drive.reg_status();
    st |= ATA_STAT_SEEK;
    drive.set_reg_status(st);
    hdd_read_sync(drive)
}

// ---------------------------------------------------------------------------
// IDENTIFY DEVICE payload construction.
// ---------------------------------------------------------------------------

/// Rebuild the 512-byte IDENTIFY payload for a drive with `size_sectors`
/// 512-byte sectors.  This is the single-threaded equivalent of
/// `ATA::CreateHDDinfo` + `CreateHDDinfoCsum`.
pub fn create_hddinfo_for(drive: &mut AtaDrive, size_sectors: u64) {
    drive.create_hddinfo(size_sectors);
}

impl AtaDrive {
    fn create_hddinfo(&mut self, size_sectors: u64) {
        self.identify_data = [0u8; SECTOR_SIZE as usize];

        // 28-bit addressable size.
        let mut max28: u32 = (1u32 << 28) - 1;
        let nb_sectors: u32 = min(size_sectors as u32, max28);
        if self.lba48_supported {
            max28 = u32::MAX; // We'll widen via u64 below.
        }
        let mut size_sectors: u64 = size_sectors;
        if self.lba48_supported {
            size_sectors = min(size_sectors, (1u64 << 48) - 1);
        }

        // Default CHS translation.
        const DEF_HEADS: u16 = 16;
        const DEF_SECTORS: u16 = 63;
        let cyl_long = (min(nb_sectors as u64, 16514064)) / DEF_HEADS as u64 / DEF_SECTORS as u64;
        let def_cylinders: u16 = min(cyl_long, u16::MAX as u64) as u16;

        // Current CHS translation.
        let cur_cyl_long = (min(nb_sectors as u64, 16514064))
            / (self.cur_heads as u64)
            / (self.cur_sectors as u64);
        self.cur_cylinders = min(cur_cyl_long, u16::MAX as u64) as u16;
        let cur_old_size: u32 = (self.cur_cylinders as u32)
            * (self.cur_heads as u32)
            * (self.cur_sectors as u32);

        let mut idx: usize = 0;
        write_u16(&mut self.identify_data, &mut idx, 0x0040); // word 0
        write_u16(&mut self.identify_data, &mut idx, def_cylinders); // word 1
        write_u16(&mut self.identify_data, &mut idx, 0xC837); // word 2
        write_u16(&mut self.identify_data, &mut idx, DEF_HEADS); // word 3
        write_u16(&mut self.identify_data, &mut idx, (SECTOR_SIZE as u16) * DEF_SECTORS); // word 4
        write_u16(&mut self.identify_data, &mut idx, SECTOR_SIZE as u16); // word 5
        write_u16(&mut self.identify_data, &mut idx, DEF_SECTORS); // word 6
        idx += 2 * 2; // word 7-8 reserved
        idx += 2; // word 9 retired
        write_padded_string(&mut self.identify_data, &mut idx, "PCSX2-DEV9-ATA-HDD", 20); // 10-19
        write_u16(&mut self.identify_data, &mut idx, 0); // 20
        write_u16(&mut self.identify_data, &mut idx, 0); // 21
        write_u16(&mut self.identify_data, &mut idx, 0); // 22
        write_padded_string(&mut self.identify_data, &mut idx, "FIRM100", 8); // 23-26
        write_padded_string(&mut self.identify_data, &mut idx, "PCSX2-DEV9-ATA-HDD", 40); // 27-46
        write_u16(&mut self.identify_data, &mut idx, 128 | (0x80 << 8)); // 47
        idx += 2; // 48 reserved
        write_u16(&mut self.identify_data, &mut idx, (1 << 11) | (1 << 9) | (1 << 8)); // 49
        write_u16(&mut self.identify_data, &mut idx, 1 << 14); // 50
        write_u16(
            &mut self.identify_data,
            &mut idx,
            (if self.pio_mode > 2 { self.pio_mode as u16 } else { 2 }) << 8,
        ); // 51
        write_u16(&mut self.identify_data, &mut idx, 0); // 52
        write_u16(&mut self.identify_data, &mut idx, 1 | (1 << 1) | (1 << 2)); // 53
        write_u16(&mut self.identify_data, &mut idx, self.cur_cylinders); // 54
        write_u16(&mut self.identify_data, &mut idx, self.cur_heads as u16); // 55
        write_u16(&mut self.identify_data, &mut idx, self.cur_sectors as u16); // 56
        write_u32(&mut self.identify_data, &mut idx, cur_old_size); // 57-58
        write_u16(
            &mut self.identify_data,
            &mut idx,
            self.cur_multiple_sectors_setting as u16 | (1 << 8),
        ); // 59
        write_u32(&mut self.identify_data, &mut idx, nb_sectors); // 60-61
        idx += 2; // 62 SDMA
        let mdma_bits = if self.mdma_mode >= 0 {
            0x07u16 | (1u16 << (self.mdma_mode as u8 + 8))
        } else {
            0x07
        };
        write_u16(&mut self.identify_data, &mut idx, mdma_bits); // 63
        write_u16(&mut self.identify_data, &mut idx, 0x03); // 64
        write_u16(&mut self.identify_data, &mut idx, 120); // 65
        write_u16(&mut self.identify_data, &mut idx, 120); // 66
        write_u16(&mut self.identify_data, &mut idx, 120); // 67
        write_u16(&mut self.identify_data, &mut idx, 120); // 68
        // 69-79 reserved
        idx = 80 * 2;
        write_u16(&mut self.identify_data, &mut idx, 0x70); // 80
        write_u16(&mut self.identify_data, &mut idx, 0x18); // 81
        write_u16(
            &mut self.identify_data,
            &mut idx,
            (1 << 0) | (1 << 5) | (1 << 14),
        ); // 82
        write_u16(
            &mut self.identify_data,
            &mut idx,
            ((self.lba48_supported as u16) << 10) | (1 << 12) | (1 << 13) | (1 << 14),
        ); // 83
        write_u16(
            &mut self.identify_data,
            &mut idx,
            (1 << 0) | (1 << 1) | (1 << 14),
        ); // 84
        write_u16(
            &mut self.identify_data,
            &mut idx,
            ((self.fet_smart_enabled as u16) << 0)
                | ((self.fet_security_enabled as u16) << 1)
                | ((self.fet_write_cache_enabled as u16) << 5)
                | ((self.fet_host_protected_area_enabled as u16) << 10)
                | (1 << 14),
        ); // 85
        write_u16(
            &mut self.identify_data,
            &mut idx,
            ((self.lba48_supported as u16) << 10) | (1 << 12) | (1 << 13),
        ); // 86
        write_u16(
            &mut self.identify_data,
            &mut idx,
            (1 << 0) | (1 << 1) | (1 << 14),
        ); // 87
        let udma_bits = if self.udma_mode >= 0 {
            0x7Fu16 | (1u16 << (self.udma_mode as u8 + 8))
        } else {
            0x7F
        };
        write_u16(&mut self.identify_data, &mut idx, udma_bits); // 88
        // 89-92: time/security/master
        idx = 93 * 2;
        let sel = self.selected_device();
        if sel != 0 {
            write_u16(&mut self.identify_data, &mut idx, (1 << 14) | (0x3 << 8));
        } else {
            write_u16(&mut self.identify_data, &mut idx, (1 << 14) | (1 << 3) | 0x3);
        }
        // 94-99 reserved
        idx = 100 * 2;
        if self.lba48_supported {
            write_u64(&mut self.identify_data, &mut idx, size_sectors);
        } else {
            write_u64(&mut self.identify_data, &mut idx, 0);
        }
        // 104 reserved
        idx = 106 * 2;
        write_u16(&mut self.identify_data, &mut idx, (1 << 14) | 0);
        // 107-255 reserved
        create_hddinfo_csum(&mut self.identify_data);
    }
}

fn create_hddinfo_csum(buf: &mut [u8; SECTOR_SIZE as usize]) {
    let mut counter: u8 = 0;
    for i in 0..(SECTOR_SIZE as usize - 1) {
        counter = counter.wrapping_add(buf[i]);
    }
    counter = counter.wrapping_add(0xA5);
    buf[510] = 0xA5;
    buf[511] = (255u8).wrapping_sub(counter).wrapping_add(1);
}

fn write_u16(data: &mut [u8], idx: &mut usize, value: u16) {
    data[*idx] = value as u8;
    data[*idx + 1] = (value >> 8) as u8;
    *idx += 2;
}

fn write_u32(data: &mut [u8], idx: &mut usize, value: u32) {
    data[*idx] = value as u8;
    data[*idx + 1] = (value >> 8) as u8;
    data[*idx + 2] = (value >> 16) as u8;
    data[*idx + 3] = (value >> 24) as u8;
    *idx += 4;
}

fn write_u64(data: &mut [u8], idx: &mut usize, value: u64) {
    data[*idx] = value as u8;
    data[*idx + 1] = (value >> 8) as u8;
    data[*idx + 2] = (value >> 16) as u8;
    data[*idx + 3] = (value >> 24) as u8;
    data[*idx + 4] = (value >> 32) as u8;
    data[*idx + 5] = (value >> 40) as u8;
    data[*idx + 6] = (value >> 48) as u8;
    data[*idx + 7] = (value >> 56) as u8;
    *idx += 8;
}

fn write_padded_string(data: &mut [u8], idx: &mut usize, value: &str, len: usize) {
    for slot in data.iter_mut().skip(*idx).take(len) {
        *slot = b' ';
    }
    let copy_len = value.len().min(len);
    data[*idx..*idx + copy_len].copy_from_slice(&value.as_bytes()[..copy_len]);
    *idx += len;
}

// ---------------------------------------------------------------------------
// File I/O helpers - small wrappers around `Read`/`Write` + `Seek`.
// ---------------------------------------------------------------------------

fn perform_pread(f: &File, pos: u64, dst: &mut [u8]) -> Result<(), String> {
    let mut f = f;
    f.seek(SeekFrom::Start(pos))
        .map_err(|e| format!("seek failed: {e}"))?;
    f.read_exact(dst).map_err(|e| format!("read failed: {e}"))
}

fn perform_pwrite(f: &File, pos: u64, src: &[u8]) -> Result<(), String> {
    let mut f = f;
    f.seek(SeekFrom::Start(pos))
        .map_err(|e| format!("seek failed: {e}"))?;
    f.write_all(src).map_err(|e| format!("write failed: {e}"))
}

// ---------------------------------------------------------------------------
// HDD image creation - one-shot helper used by the PS2 "create HDD" flow.
// ---------------------------------------------------------------------------

/// Create a freshly-initialised HDD image file of `size` bytes at `path`.
///
/// The image is filled with zeroes, with `zero_size` bytes at the end
/// of each 1 MiB chunk skipped (so the resulting file is sparse on
/// filesystems that support hole-punching).  Returns the opened
/// [`File`] ready for further I/O.
pub fn hdd_create_file(path: &Path, size: u64) -> Result<File, std::io::Error> {
    if path.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "HDD image already exists",
        ));
    }
    let mut f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    f.set_len(size)?;
    Ok(f)
}
