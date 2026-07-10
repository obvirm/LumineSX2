// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! `RDebugRec` — Idiomatic Rust 2021 translation of the PCSX2 RDebug (Deci2
//! debug-over-ethernet protocol) and Recording (input recording) subsystems.
//!
//! This module preserves the structural shape of the original C/C++ sources:
//! the Deci2 protocol dispatch (DCMP, DBGP, ILOADP, NETMP, TTYP, DRFP) and
//! the input recording pipeline (`InputRecording`, `InputRecordingFile`,
//! `InputRecordingControls`, `PadData`, `InputRecordingLogger`). Only `std` is
//! used; C runtime helpers are replaced with their idiomatic Rust equivalents
//! and globals are exposed as `static mut` to mirror the original linkage.

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(dead_code)]
#![allow(static_mut_refs)]

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::path::Path;
use std::sync::atomic::{AtomicI32, Ordering};

// ---------------------------------------------------------------------------
// Shared primitives (replacing `u8`, `u16`, `u32`, `u64`, `s32` aliases).
// ---------------------------------------------------------------------------

pub type u8 = std::primitive::u8;
pub type u16 = std::primitive::u16;
pub type u32 = std::primitive::u32;
pub type u64 = std::primitive::u64;
pub type s32 = std::primitive::i32;

/// A drop-in stand-in for the C `Socket` handle. The original C++ code mixed
/// TCP and UDP depending on the caller; we expose both as a tagged enum so
/// the dispatch can be unit-tested without an actual kernel socket.
pub enum Socket {
    Tcp(TcpStream),
    Udp(UdpSocket),
    Listener(TcpListener),
    None,
}

impl Default for Socket {
    fn default() -> Self {
        Socket::None
    }
}

impl Socket {
    pub fn is_open(&self) -> bool {
        !matches!(self, Socket::None)
    }

    pub fn close(&mut self) {
        *self = Socket::None;
    }
}

// ---------------------------------------------------------------------------
// Deci2 protocol constants and header types
// ---------------------------------------------------------------------------

pub const PROTO_DCMP: u16 = 0x0001;
pub const PROTO_ITTYP: u16 = 0x0110;
pub const PROTO_IDBGP: u16 = 0x0130;
pub const PROTO_ILOADP: u16 = 0x0150;
pub const PROTO_ETTYP: u16 = 0x0220;
pub const PROTO_EDBGP: u16 = 0x0230;
pub const PROTO_NETMP: u16 = 0x0400;

pub const STOP: i32 = 0;
pub const RUN: i32 = 1;

/// DECI2 wire header (8 bytes, packed).
#[repr(C, packed)]
#[derive(Clone, Copy, Default)]
pub struct DECI2_HEADER {
    pub length: u16,
    pub _pad: u16,
    pub protocol: u16,
    pub source: u8,
    pub destination: u8,
}

/// DECI2 DBGP breakpoint entry (8 bytes, packed).
#[repr(C, packed)]
#[derive(Clone, Copy, Default)]
pub struct DECI2_DBGP_BRK {
    pub address: u32,
    pub count: u32,
}

// ---------------------------------------------------------------------------
// Deci2 globals
// ---------------------------------------------------------------------------

/// EE / IOP breakpoint tables and control state shared with the EE/IOP
/// emulator cores. Indexed from the DBGP dispatch in `D2_DBGP`.
pub static mut ebrk: [DECI2_DBGP_BRK; 32] = [DECI2_DBGP_BRK { address: 0, count: 0 }; 32];
pub static mut ibrk: [DECI2_DBGP_BRK; 32] = [DECI2_DBGP_BRK { address: 0, count: 0 }; 32];
pub static mut ebrk_count: s32 = 0;
pub static mut ibrk_count: s32 = 0;
pub static mut runCode: s32 = STOP;
pub static mut runCount: s32 = 0;

/// Stand-in for `Threading::KernelSemaphore` used by the DBGP run-loop.
pub static mut run_event_posted: bool = false;

/// 1 = a debugger client is attached, 0 = not connected.
pub static mut connected: s32 = 0;

/// Run-status atomic used by the EE thread to coordinate stepping.
pub static RUN_STATUS: AtomicI32 = AtomicI32::new(STOP);

/// Top-level Deci2 state. The `server` socket is the listening socket used
/// for TCP-based debuggers; `host` and `port` are configured at open-time.
pub struct Deci2 {
    pub server: Socket,
    pub host: String,
    pub port: u16,
    pub active_protocols: Vec<u16>,
    pub d2_message: [u8; 100],
    pub d2_count: i32,
    pub d2_connect: Vec<NetmpConnect>,
    pub d2_protocol: u16,
    pub d2_source: u8,
    pub d2_destination: u8,
    pub d2_ee_boot: u64,
    pub d2_iop_boot: u64,
}

impl Default for Deci2 {
    fn default() -> Self {
        Self {
            server: Socket::None,
            host: String::from("127.0.0.1"),
            port: 0,
            active_protocols: Vec::new(),
            d2_message: [0u8; 100],
            d2_count: 1,
            d2_connect: vec![NetmpConnect { priority: 0xFF, _pad: 0, protocol: 0x400 }],
            d2_protocol: 0,
            d2_source: 0,
            d2_destination: 0,
            d2_ee_boot: 0,
            d2_iop_boot: 0,
        }
    }
}

pub static mut deci2: Deci2 = Deci2 {
    server: Socket::None,
    host: String::new(),
    port: 0,
    active_protocols: Vec::new(),
    d2_message: [0u8; 100],
    d2_count: 1,
    d2_connect: Vec::new(),
    d2_protocol: 0,
    d2_source: 0,
    d2_destination: 0,
    d2_ee_boot: 0,
    d2_iop_boot: 0,
};

/// NETMP connect descriptor (4 bytes).
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct NetmpConnect {
    pub priority: u8,
    pub _pad: u8,
    pub protocol: u16,
}

impl Default for NetmpConnect {
    fn default() -> Self {
        Self { priority: 0xFF, _pad: 0, protocol: 0x0400 }
    }
}

// ---------------------------------------------------------------------------
// Deci2 sub-protocol headers
// ---------------------------------------------------------------------------

#[repr(C, packed)]
#[derive(Clone, Copy, Default)]
pub struct DECI2_DCMP_HEADER {
    pub h: DECI2_HEADER,
    pub r#type: u8,
    pub code: u8,
    pub _pad: u16,
}

#[repr(C, packed)]
#[derive(Clone, Copy, Default)]
pub struct DECI2_DBGP_HEADER {
    pub h: DECI2_HEADER,
    pub id: u16,
    pub r#type: u8,
    pub code: u8,
    pub result: u8,
    pub count: u8,
    pub _pad: u16,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct DECI2_DBGP_CONF {
    pub major_ver: u32,
    pub minor_ver: u32,
    pub target_id: u32,
    pub _pad: u32,
    pub mem_align: u32,
    pub _pad2: u32,
    pub reg_size: u32,
    pub nreg: u32,
    pub nbrkpt: u32,
    pub ncont: u32,
    pub nstep: u32,
    pub nnext: u32,
    pub mem_limit_align: u32,
    pub mem_limit_size: u32,
    pub run_stop_state: u32,
    pub hdbg_area_addr: u32,
    pub hdbg_area_size: u32,
}

#[repr(C, packed)]
#[derive(Clone, Copy, Default)]
pub struct DECI2_DBGP_EREG {
    pub kind: u8,
    pub number: u8,
    pub _pad: u16,
    pub value: [u64; 2],
}

#[repr(C, packed)]
#[derive(Clone, Copy, Default)]
pub struct DECI2_DBGP_IREG {
    pub kind: u8,
    pub number: u8,
    pub _pad: u16,
    pub value: u32,
}

#[repr(C, packed)]
#[derive(Clone, Copy, Default)]
pub struct DECI2_DBGP_MEM {
    pub space: u8,
    pub align: u8,
    pub _pad: u16,
    pub address: u32,
    pub length: u32,
}

#[repr(C, packed)]
#[derive(Clone, Copy, Default)]
pub struct DECI2_DBGP_RUN {
    pub entry: u32,
    pub gp: u32,
    pub _pad: u32,
    pub _pad1: u32,
    pub argc: u32,
}

#[repr(C, packed)]
#[derive(Clone, Copy, Default)]
pub struct DECI2_ILOADP_HEADER {
    pub h: DECI2_HEADER,
    pub code: u8,
    pub action: u8,
    pub result: u8,
    pub stamp: u8,
    pub moduleId: u32,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct DECI2_ILOADP_INFO {
    pub version: u16,
    pub flags: u16,
    pub module_address: u32,
    pub text_size: u32,
    pub data_size: u32,
    pub bss_size: u32,
    pub _pad: [u32; 3],
}

#[repr(C, packed)]
#[derive(Clone, Copy, Default)]
pub struct DECI2_NETMP_HEADER {
    pub h: DECI2_HEADER,
    pub code: u8,
    pub result: u8,
}

#[repr(C, packed)]
#[derive(Clone, Copy, Default)]
pub struct DECI2_TTYP_HEADER {
    pub h: DECI2_HEADER,
    pub flushreq: u32,
}

// ---------------------------------------------------------------------------
// Deci2 configuration tables
// ---------------------------------------------------------------------------

/// Configuration tables reported by DBGP GETCONF for each target.
pub static mut CPU_CONF: DECI2_DBGP_CONF = DECI2_DBGP_CONF {
    major_ver: 3, minor_ver: 0, target_id: PROTO_EDBGP as u32, _pad: 0,
    mem_align: 0x41F, _pad2: 0, reg_size: 1, nreg: 7,
    nbrkpt: 32, ncont: 32, nstep: 1, nnext: 0xFF,
    mem_limit_align: 0xFF, mem_limit_size: 0x1F,
    run_stop_state: 0x400, hdbg_area_addr: 0x80020c70, hdbg_area_size: 0x100,
};
pub static mut VU0_CONF: DECI2_DBGP_CONF = DECI2_DBGP_CONF {
    major_ver: 3, minor_ver: 0, target_id: PROTO_EDBGP as u32, _pad: 0,
    mem_align: 0x41F, _pad2: 0, reg_size: 1, nreg: 7,
    nbrkpt: 32, ncont: 32, nstep: 1, nnext: 0xFF,
    mem_limit_align: 0xFF, mem_limit_size: 0x1F,
    run_stop_state: 0x400, hdbg_area_addr: 0x80020c70, hdbg_area_size: 0x100,
};
pub static mut VU1_CONF: DECI2_DBGP_CONF = DECI2_DBGP_CONF {
    major_ver: 3, minor_ver: 0, target_id: PROTO_EDBGP as u32, _pad: 0,
    mem_align: 0x41F, _pad2: 0, reg_size: 1, nreg: 7,
    nbrkpt: 32, ncont: 32, nstep: 1, nnext: 0xFF,
    mem_limit_align: 0xFF, mem_limit_size: 0x1F,
    run_stop_state: 0x400, hdbg_area_addr: 0x80020c70, hdbg_area_size: 0x100,
};
pub static mut IOP_CONF: DECI2_DBGP_CONF = DECI2_DBGP_CONF {
    major_ver: 3, minor_ver: 0, target_id: PROTO_IDBGP as u32, _pad: 0,
    mem_align: 0x00F, _pad2: 0, reg_size: 1, nreg: 5,
    nbrkpt: 62, ncont: 32, nstep: 1, nnext: 0x00,
    mem_limit_align: 0x00, mem_limit_size: 0x07,
    run_stop_state: 0x200, hdbg_area_addr: 0x0001E670, hdbg_area_size: 0x100,
};

// ---------------------------------------------------------------------------
// Deci2 helpers
// ---------------------------------------------------------------------------

/// Swap the source and destination fields of a DECI2 header (C `exchangeSD`).
pub fn exchangeSD(h: &mut DECI2_HEADER) {
    let tmp = h.source;
    h.source = h.destination;
    h.destination = tmp;
}

/// Buffer size used by the original C++ for input/output packet copies.
pub const BUFFER_SIZE: usize = 128 * 1024;

/// Send a DCMP packet (used by NETMP for protocol enumeration).
pub fn sendDCMP(protocol: u16, source: u8, destination: u8, r#type: u8, code: u8, data: &[u8]) {
    let mut tmp = vec![0u8; std::mem::size_of::<DECI2_DCMP_HEADER>() + data.len()];
    let hdr = unsafe { &mut *(tmp.as_mut_ptr() as *mut DECI2_DCMP_HEADER) };
    hdr.h.length = (std::mem::size_of::<DECI2_DCMP_HEADER>() + data.len()) as u16;
    hdr.h._pad = 0;
    hdr.h.protocol = protocol;
    hdr.h.source = source;
    hdr.h.destination = destination;
    hdr.r#type = r#type;
    hdr.code = code;
    hdr._pad = 0;
    let off = std::mem::size_of::<DECI2_DCMP_HEADER>();
    tmp[off..off + data.len()].copy_from_slice(data);
    writeData(&tmp);
}

/// Send a TTY packet (currently a no-op write, mirroring the C++ comment).
pub fn sendTTYP(protocol: u16, source: u8, data: &str) {
    let mut tmp = vec![0u8; std::mem::size_of::<DECI2_TTYP_HEADER>() + data.len()];
    let hdr = unsafe { &mut *(tmp.as_mut_ptr() as *mut DECI2_TTYP_HEADER) };
    hdr.h.length = (std::mem::size_of::<DECI2_TTYP_HEADER>() + data.len()) as u16;
    hdr.h._pad = 0;
    hdr.h.protocol = protocol + if source == b'E' { PROTO_ETTYP } else { PROTO_ITTYP };
    hdr.h.source = source;
    hdr.h.destination = b'H';
    hdr.flushreq = 0;
    if hdr.h.length as usize > 2048 {
        eprintln!("TTYP: buffer overflow");
        return;
    }
    let off = std::mem::size_of::<DECI2_TTYP_HEADER>();
    tmp[off..off + data.len()].copy_from_slice(data.as_bytes());
}

/// Send a DBGP BREAK reply to the host.
pub fn sendBREAK(source: u8, id: u16, code: u8, result: u8, count: u8) {
    let mut tmp = DECI2_DBGP_HEADER::default();
    tmp.h.length = std::mem::size_of::<DECI2_DBGP_HEADER>() as u16;
    tmp.h._pad = 0;
    tmp.h.protocol = if source == b'E' { PROTO_EDBGP } else { PROTO_IDBGP };
    tmp.h.source = source;
    tmp.h.destination = b'H';
    tmp.id = id;
    tmp.r#type = 0x15;
    tmp.code = code;
    tmp.result = result;
    tmp.count = count;
    tmp._pad = 0;
    let bytes = unsafe {
        std::slice::from_raw_parts(
            (&tmp as *const DECI2_DBGP_HEADER) as *const u8,
            std::mem::size_of::<DECI2_DBGP_HEADER>(),
        )
    };
    writeData(bytes);
}

/// Stub for the C `writeData` — the original wrote to the active socket. We
/// route it through `deci2.server` when present, otherwise drop it.
pub fn writeData(_data: &[u8]) -> bool {
    let server = unsafe { &mut deci2.server };
    match server {
        Socket::Tcp(stream) => stream.write(_data).is_ok(),
        Socket::Udp(socket) => {
            // A real implementation would resolve the connected target;
            // we just stage the send as a no-op for the rewrite.
            let _ = socket.send(_data);
            true
        }
        _ => true,
    }
}

// ---------------------------------------------------------------------------
// Deci2 dispatch
// ---------------------------------------------------------------------------

/// Initialise the global Deci2 state.
pub fn deci2Init() {
    unsafe {
        deci2 = Deci2::default();
        connected = 0;
        ebrk_count = 0;
        ibrk_count = 0;
        runCode = STOP;
        runCount = 0;
        run_event_posted = false;
    }
    RUN_STATUS.store(STOP, Ordering::SeqCst);
}

/// Reset the Deci2 protocol state to a clean connect/breakpoint table.
pub fn deci2Reset() {
    unsafe {
        for b in ebrk.iter_mut() { *b = DECI2_DBGP_BRK::default(); }
        for b in ibrk.iter_mut() { *b = DECI2_DBGP_BRK::default(); }
        ebrk_count = 0;
        ibrk_count = 0;
        runCode = STOP;
        runCount = 0;
        run_event_posted = false;
        deci2.d2_message = [0u8; 100];
        deci2.d2_count = 1;
        deci2.d2_connect.clear();
        deci2.d2_connect.push(NetmpConnect::default());
    }
}

/// Open the Deci2 server. In the original C++ this calls WinSock's `socket`
/// and `bind`; here we use the Rust standard library equivalent.
pub fn deci2ServerOpen(host: &str, port: u16) -> std::io::Result<()> {
    let addr: SocketAddr = format!("{}:{}", host, port).parse().unwrap_or(
        SocketAddr::from(([127, 0, 0, 1], port)),
    );
    let listener = TcpListener::bind(addr)?;
    listener.set_nonblocking(true)?;
    unsafe {
        deci2.server = Socket::Listener(listener);
        deci2.host = host.to_string();
        deci2.port = port;
    }
    Ok(())
}

/// DCMP dispatch (originally `D2_DCMP` in `deci2_dcmp.cpp`).
pub fn D2_DCMP(inbuffer: &[u8], outbuffer: &mut Vec<u8>, message: &mut String) {
    if inbuffer.len() < std::mem::size_of::<DECI2_DCMP_HEADER>() {
        return;
    }
    let in_hdr = unsafe { &*(inbuffer.as_ptr() as *const DECI2_DCMP_HEADER) };
    outbuffer.resize(BUFFER_SIZE, 0);
    let copy_len = inbuffer.len().min(BUFFER_SIZE);
    outbuffer[..copy_len].copy_from_slice(&inbuffer[..copy_len]);
    let out_hdr = unsafe { &mut *(outbuffer.as_mut_ptr() as *mut DECI2_DCMP_HEADER) };
    out_hdr.h.length = std::mem::size_of::<DECI2_DCMP_HEADER>() as u16;
    out_hdr.code = in_hdr.code.wrapping_add(1);
    let _ = in_hdr.r#type;
    let _ = message;
}

/// DRFP dispatch (originally `D2_DCMP` in `deci2_drfp.cpp` — a separate
/// shim that pulled text messages out of the wire buffer).
pub fn D2_DRFP(inbuffer: &[u8], outbuffer: &mut Vec<u8>, message: &mut String) {
    if inbuffer.len() < std::mem::size_of::<DECI2_DCMP_HEADER>() {
        return;
    }
    let in_hdr = unsafe { &*(inbuffer.as_ptr() as *const DECI2_DCMP_HEADER) };
    outbuffer.resize(BUFFER_SIZE, 0);
    let copy_len = inbuffer.len().min(BUFFER_SIZE);
    outbuffer[..copy_len].copy_from_slice(&inbuffer[..copy_len]);
    let out_hdr = unsafe { &mut *(outbuffer.as_mut_ptr() as *mut DECI2_DCMP_HEADER) };
    out_hdr.h.length = std::mem::size_of::<DECI2_DCMP_HEADER>() as u16;
    let data_off = std::mem::size_of::<DECI2_DCMP_HEADER>();
    let data = &inbuffer[data_off..];
    match in_hdr.r#type {
        4 => {
            let nul = data.iter().position(|&b| b == 0).unwrap_or(data.len());
            let text = String::from_utf8_lossy(&data[..nul]).into_owned();
            *message = format!("  [DCMP] code=MESSAGE {}", text);
            unsafe {
                let dst = &mut deci2.d2_message[..text.len()];
                dst.copy_from_slice(text.as_bytes());
                deci2.d2_message[text.len()] = 0;
            }
        }
        _ => {
            *message = format!(
                "  [DCMP] code={}[unknown] result={}",
                in_hdr.code, in_hdr.r#type
            );
        }
    }
    out_hdr.code = out_hdr.code.wrapping_add(1);
    out_hdr._pad = 0;
}

/// DBGP dispatch (originally `D2_DBGP` in `deci2_dbgp.cpp`). The body is a
/// close port of the original switch; register / memory operations are kept
/// as `todo!()`-style placeholders because they reach into the EE/IOP core.
pub fn D2_DBGP(
    inbuffer: &[u8],
    outbuffer: &mut Vec<u8>,
    message: &mut String,
    eepc: &mut String,
    ioppc: &mut String,
    eecy: &mut String,
    iopcy: &mut String,
) {
    if inbuffer.len() < std::mem::size_of::<DECI2_DBGP_HEADER>() {
        return;
    }
    let in_hdr = unsafe { &*(inbuffer.as_ptr() as *const DECI2_DBGP_HEADER) };
    outbuffer.resize(BUFFER_SIZE, 0);
    let copy_len = inbuffer.len().min(BUFFER_SIZE);
    outbuffer[..copy_len].copy_from_slice(&inbuffer[..copy_len]);
    let out_hdr = unsafe { &mut *(outbuffer.as_mut_ptr() as *mut DECI2_DBGP_HEADER) };
    out_hdr.r#type = in_hdr.r#type.wrapping_add(1);
    out_hdr.result = 0;
    exchangeSD(&mut out_hdr.h);
    let mut line = String::new();
    let target = match in_hdr.id {
        0 => "CPU",
        1 => "VU0",
        _ => "VU1",
    };
    match in_hdr.r#type {
        0x00 => {
            line = format!("{}/GETCONF", target);
            let in_dest_is_i = in_hdr.h.destination == b'I';
            if in_dest_is_i {
                let conf = unsafe { IOP_CONF };
                let off = std::mem::size_of::<DECI2_DBGP_HEADER>();
                let conf_bytes = unsafe {
                    std::slice::from_raw_parts(
                        (&conf as *const DECI2_DBGP_CONF) as *const u8,
                        std::mem::size_of::<DECI2_DBGP_CONF>(),
                    )
                };
                outbuffer[off..off + conf_bytes.len()].copy_from_slice(conf_bytes);
            } else {
                let conf = unsafe {
                    match in_hdr.id {
                        0 => CPU_CONF,
                        1 => VU0_CONF,
                        _ => VU1_CONF,
                    }
                };
                let off = std::mem::size_of::<DECI2_DBGP_HEADER>();
                let conf_bytes = unsafe {
                    std::slice::from_raw_parts(
                        (&conf as *const DECI2_DBGP_CONF) as *const u8,
                        std::mem::size_of::<DECI2_DBGP_CONF>(),
                    )
                };
                outbuffer[off..off + conf_bytes.len()].copy_from_slice(conf_bytes);
            }
        }
        0x02 => line = format!("{}/2", target),
        0x04 | 0x06 => {
            line = format!(
                "{}/{}REG count={} kind[0]={} number[0]={}",
                target,
                if in_hdr.r#type == 0x04 { "GET" } else { "PUT" },
                in_hdr.count,
                0,
                0,
            );
            // Register read/write would be dispatched through the EE / IOP
            // core; we leave it as a no-op so the wire format stays correct.
        }
        0x08 | 0x0A => {
            let mem = unsafe {
                &*(inbuffer.as_ptr().add(std::mem::size_of::<DECI2_DBGP_HEADER>())
                    as *const DECI2_DBGP_MEM)
            };
            let mem_address = mem.address;
            let mem_length = mem.length;
            line = format!(
                "{}/{}MEM {:08X}/{:X}",
                target,
                if in_hdr.r#type == 0x08 { "RD" } else { "WR" },
                mem_address,
                mem_length,
            );
        }
        0x10 => {
            line = format!("{}/GETBRKPT count={}", target, in_hdr.count);
            let is_i = in_hdr.h.destination == b'I';
            out_hdr.count = if is_i {
                unsafe { ibrk_count as u8 }
            } else {
                unsafe { ebrk_count as u8 }
            };
            let off = std::mem::size_of::<DECI2_DBGP_HEADER>();
            let count = out_hdr.count as usize;
            let bytes = if is_i {
                unsafe {
                    std::slice::from_raw_parts(
                        ebrk.as_ptr() as *const u8,
                        count * std::mem::size_of::<DECI2_DBGP_BRK>(),
                    )
                }
            } else {
                unsafe {
                    std::slice::from_raw_parts(
                        ibrk.as_ptr() as *const u8,
                        count * std::mem::size_of::<DECI2_DBGP_BRK>(),
                    )
                }
            };
            outbuffer[off..off + bytes.len()].copy_from_slice(bytes);
        }
        0x12 => {
            line = format!("{}/PUTBRKPT count={}", target, in_hdr.count);
            if in_hdr.count > 32 {
                out_hdr.result = 1;
                line.push_str(" TOO MANY");
            } else {
                let off = std::mem::size_of::<DECI2_DBGP_HEADER>();
                let copy_len = (in_hdr.count as usize)
                    * std::mem::size_of::<DECI2_DBGP_BRK>();
                let is_i = in_hdr.h.destination == b'I';
                if is_i {
                    unsafe {
                        let src = &inbuffer[off..off + copy_len];
                        let dst = std::slice::from_raw_parts_mut(
                            ibrk.as_mut_ptr() as *mut u8,
                            copy_len,
                        );
                        dst.copy_from_slice(src);
                        ibrk_count = in_hdr.count as s32;
                    }
                } else {
                    unsafe {
                        let src = &inbuffer[off..off + copy_len];
                        let dst = std::slice::from_raw_parts_mut(
                            ebrk.as_mut_ptr() as *mut u8,
                            copy_len,
                        );
                        dst.copy_from_slice(src);
                        ebrk_count = in_hdr.count as s32;
                    }
                }
                out_hdr.count = 0;
            }
        }
        0x14 => {
            line = format!("{}/BREAK count={}", target, in_hdr.count);
            if in_hdr.h.destination != b'I' {
                let prev = RUN_STATUS.swap(STOP, Ordering::SeqCst);
                out_hdr.result = if prev == STOP { 0x20 } else { 0x21 };
                out_hdr.code = 0xFF;
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
        0x16 => {
            line = format!(
                "{}/CONTINUE code={} count={}",
                target, in_hdr.code, in_hdr.count,
            );
            if in_hdr.h.destination != b'I' {
                RUN_STATUS.store(STOP, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(100));
                unsafe {
                    runCount = in_hdr.count as s32;
                    runCode = in_hdr.code as s32;
                    run_event_posted = true;
                }
            }
        }
        0x18 => {
            let run = unsafe {
                &*(inbuffer.as_ptr().add(std::mem::size_of::<DECI2_DBGP_HEADER>())
                    as *const DECI2_DBGP_RUN)
            };
            let run_entry = run.entry;
            let run_gp = run.gp;
            let run_argc = run.argc;
            line = format!(
                "{}/RUN code={} count={} entry=0x{:08X} gp=0x{:08X} argc={}",
                target, in_hdr.code, in_hdr.count, run_entry, run_gp, run_argc,
            );
            RUN_STATUS.store(STOP, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(1000));
            unsafe {
                runCount = 0;
                runCode = 0xFF;
                run_event_posted = true;
            }
        }
        _ => {
            line = format!(
                "type=0x{:02X} code={} count={} [unknown]",
                in_hdr.r#type, in_hdr.code, in_hdr.count,
            );
        }
    }
    let in_hdr_h_length = in_hdr.h.length;
    *message = format!(
        "[DBGP {}->{}/{:04X}] {}",
        in_hdr.h.source as char, in_hdr.h.destination as char, in_hdr_h_length, line
    );
    *eepc = format!("{:08X}", 0u32);
    *ioppc = format!("{:08X}", 0u32);
    *eecy = format!("{}", 0u32);
    *iopcy = format!("{}", 0u32);
    let _ = writeData(outbuffer);
}

/// ILOADP dispatch (originally `D2_ILOADP` in `deci2_iloadp.cpp`).
pub fn D2_ILOADP(inbuffer: &[u8], outbuffer: &mut Vec<u8>, message: &mut String) {
    if inbuffer.len() < std::mem::size_of::<DECI2_ILOADP_HEADER>() {
        return;
    }
    let in_hdr = unsafe { &*(inbuffer.as_ptr() as *const DECI2_ILOADP_HEADER) };
    let in_module_id = in_hdr.moduleId;
    let in_h_length = in_hdr.h.length;
    outbuffer.resize(BUFFER_SIZE, 0);
    let copy_len = inbuffer.len().min(BUFFER_SIZE);
    outbuffer[..copy_len].copy_from_slice(&inbuffer[..copy_len]);
    let out_hdr = unsafe { &mut *(outbuffer.as_mut_ptr() as *mut DECI2_ILOADP_HEADER) };
    out_hdr.h.length = std::mem::size_of::<DECI2_ILOADP_HEADER>() as u16;
    out_hdr.code = in_hdr.code.wrapping_add(1);
    out_hdr.result = 0;
    exchangeSD(&mut out_hdr.h);
    let line = match in_hdr.code {
        0 => format!(
            "code=START action={} stamp={} moduleId=0x{:X}",
            in_hdr.action, in_hdr.stamp, in_module_id
        ),
        2 => format!(
            "code=REMOVE action={} stamp={} moduleId=0x{:X}",
            in_hdr.action, in_hdr.stamp, in_module_id
        ),
        4 | 6 | 8 => format!(
            "code={} action={} stamp={} moduleId=0x{:X}",
            match in_hdr.code {
                4 => "LIST",
                6 => "INFO",
                _ => "WATCH",
            },
            in_hdr.action, in_hdr.stamp, in_module_id
        ),
        _ => format!("code={}[unknown]", in_hdr.code),
    };
    *message = format!(
        "[ILOADP {}->{}/{:04X}] {}",
        in_hdr.h.source as char, in_hdr.h.destination as char, in_h_length, line
    );
    let _ = writeData(outbuffer);
}

/// NETMP dispatch (originally `D2_NETMP` in `deci2_netmp.cpp`). This is the
/// top-level handshake that registers the host's supported protocols.
pub fn D2_NETMP(inbuffer: &[u8], outbuffer: &mut Vec<u8>, message: &mut String) {
    if inbuffer.len() < std::mem::size_of::<DECI2_NETMP_HEADER>() {
        return;
    }
    let in_hdr = unsafe { &*(inbuffer.as_ptr() as *const DECI2_NETMP_HEADER) };
    outbuffer.resize(BUFFER_SIZE, 0);
    let copy_len = inbuffer.len().min(BUFFER_SIZE);
    outbuffer[..copy_len].copy_from_slice(&inbuffer[..copy_len]);
    let out_hdr = unsafe { &mut *(outbuffer.as_mut_ptr() as *mut DECI2_NETMP_HEADER) };
    out_hdr.h.length = std::mem::size_of::<DECI2_NETMP_HEADER>() as u16;
    out_hdr.code = in_hdr.code.wrapping_add(1);
    out_hdr.result = 0;
    let header_size = std::mem::size_of::<DECI2_NETMP_HEADER>();
    let payload = &inbuffer[header_size..];
    let mut line = String::new();
    match in_hdr.code {
        0 => {
            let n = (in_hdr.h.length as usize - header_size)
                / std::mem::size_of::<NetmpConnect>();
            line.push_str("code=CONNECT");
            unsafe {
                deci2.d2_count += n as i32;
                let dst_start = deci2.d2_connect.len();
                let new_len = dst_start + n;
                deci2.d2_connect.resize(new_len, NetmpConnect::default());
                let conn_bytes = unsafe {
                    std::slice::from_raw_parts(
                        payload.as_ptr() as *const NetmpConnect,
                        n,
                    )
                };
                deci2.d2_connect[dst_start..].copy_from_slice(conn_bytes);
            }
            let _ = writeData(outbuffer);
        }
        2 => {
            // RESET: read EE / IOP boot vectors, then advertise protocols.
            let mut offset = header_size + 2;
            let ee_boot = u64::from_le_bytes(payload[offset..offset + 8].try_into().unwrap_or([0; 8]));
            offset += 8;
            let iop_boot = u64::from_le_bytes(payload[offset..offset + 8].try_into().unwrap_or([0; 8]));
            line = format!("code=RESET EE=0x{:X} IOP=0x{:X}", ee_boot, iop_boot);
            unsafe {
                deci2.d2_ee_boot = ee_boot;
                deci2.d2_iop_boot = iop_boot;
            }
            let _ = writeData(outbuffer);
            let node: u16 = b'I' as u16;
            sendDCMP(PROTO_DCMP, b'H', b'H', 2, 0, &node.to_le_bytes());
            let node: u16 = b'E' as u16;
            sendDCMP(PROTO_DCMP, b'H', b'H', 2, 0, &node.to_le_bytes());
            sendDCMP(PROTO_DCMP, b'I', b'H', 2, 1, &PROTO_ILOADP.to_le_bytes());
            for i in 0..10u16 {
                let node = PROTO_ETTYP + i;
                sendDCMP(PROTO_DCMP, b'E', b'H', 2, 1, &node.to_le_bytes());
                let node = PROTO_ITTYP + i;
                sendDCMP(PROTO_DCMP, b'E', b'H', 2, 1, &node.to_le_bytes());
            }
            sendDCMP(PROTO_DCMP, b'E', b'H', 2, 1, &(PROTO_ETTYP + 0xF).to_le_bytes());
            sendDCMP(PROTO_DCMP, b'E', b'H', 2, 1, &(PROTO_ITTYP + 0xF).to_le_bytes());
        }
        4 => {
            let nul = payload.iter().position(|&b| b == 0).unwrap_or(payload.len());
            let text = String::from_utf8_lossy(&payload[..nul]).into_owned();
            line = format!("code=MESSAGE {}", text);
            unsafe {
                deci2.d2_message[..text.len()].copy_from_slice(text.as_bytes());
                deci2.d2_message[text.len()] = 0;
            }
            let _ = writeData(outbuffer);
        }
        6 => {
            line.push_str("code=STATUS");
            let _ = writeData(outbuffer);
        }
        8 => {
            let proto = u16::from_le_bytes(payload[..2].try_into().unwrap_or([0; 2]));
            line = format!("code=KILL protocol=0x{:04X}", proto);
            let _ = writeData(outbuffer);
        }
        10 => {
            let nul = payload.iter().position(|&b| b == 0).unwrap_or(payload.len());
            line = format!(
                "code=VERSION {}",
                String::from_utf8_lossy(&payload[..nul])
            );
            let off = std::mem::size_of::<DECI2_NETMP_HEADER>();
            let version = b"0.2.0";
            outbuffer[off..off + version.len()].copy_from_slice(version);
            out_hdr.h.length = (off + version.len()) as u16;
            let _ = writeData(outbuffer);
        }
        _ => {
            line = format!("code={}[unknown] result={}", in_hdr.code, in_hdr.result);
            let _ = writeData(outbuffer);
        }
    }
    // `DECI2_NETMP_HEADER` and its inner `DECI2_HEADER` are both
    // `#[repr(C, packed)]`, so reading the `u16` `length` field directly
    // would form a misaligned reference. Read the two little-endian bytes
    // straight from the source buffer instead. The `u8` fields are
    // 1-byte aligned and can be read normally.
    let length = u16::from_le_bytes([inbuffer[0], inbuffer[1]]);
    *message = format!(
        "[NETMP {}->{}/{:04X}] {}",
        in_hdr.h.source as char, in_hdr.h.destination as char, length, line
    );
}

// ---------------------------------------------------------------------------
// Recording — input recording types
// ---------------------------------------------------------------------------

/// State machine for the recording subsystem.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Idle,
    Recording,
    Replaying,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecType {
    PowerOn,
    FromSavestate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Analog(u8, u8);

/// One DualShock2 button — pressure byte plus pressed flag.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PressureButton {
    pub pressed: bool,
    pub pressure: u8,
}

/// The full controller state snapshot used by the recording format.
#[derive(Clone, Debug, Default)]
pub struct PadData {
    pub port: i32,
    pub slot: i32,
    /// Aggregated 8-bit button state.
    pub buttons: u32,
    /// Aggregate rumble (motors 0/1).
    pub rumble: [u8; 2],
    /// Per-button pressure values for all 16 face/shoulder buttons.
    pub analog: [u8; 16],
}

impl PadData {
    pub fn new(port: i32, slot: i32) -> Self {
        let mut p = PadData {
            port, slot, buttons: 0, rumble: [0; 2], analog: [0; 16],
        };
        p.refresh();
        p
    }

    /// Refresh from the live controller — in the original C++ this queried
    /// `Pad::GetPad(ext_port)`. We leave it as a no-op stub so the wire
    /// format stays correct without depending on the pad core.
    pub fn refresh(&mut self) {
        // The original populated m_compactPressFlagsGroupOne/Two and the
        // analog tuples here. We zero everything and let callers overwrite.
        self.buttons = 0;
        self.rumble = [0, 0];
        self.analog = [0; 16];
    }

    /// Push the recorded values into a `Pad` — `OverrideActualController`
    /// in the C++ source. Stubbed for the same reason as `refresh`.
    pub fn OverrideActualController(&self) {
        // Wired through `Pad::GetPad(ext_port)->Set...` in the original.
    }

    /// Pretty-print the current pad data to the recording log.
    pub fn LogPadData(&self) {
        log(&format!(
            "[PAD {}:{}] buttons={:#x} rumble={:?} analog={:?}",
            self.port, self.slot, self.buttons, self.rumble, self.analog,
        ));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecState {
    Stopped,
    Recording,
    Replaying,
}

pub struct InputRecordingControls {
    pub state: Mode,
    queue: Vec<Box<dyn FnOnce() + Send + Sync>>,
}

impl Default for InputRecordingControls {
    fn default() -> Self {
        Self { state: Mode::Idle, queue: Vec::new() }
    }
}

impl InputRecordingControls {
    pub fn new() -> Self { Self::default() }

    pub fn setRecordMode(&mut self) { self.state = Mode::Recording; }
    pub fn setReplayMode(&mut self) { self.state = Mode::Replaying; }

    pub fn toggleRecordMode(&mut self) {
        if self.isReplaying() { self.setRecordMode(); } else { self.setReplayMode(); }
    }

    pub fn isRecording(&self) -> bool { self.state == Mode::Recording }
    pub fn isReplaying(&self) -> bool { self.state == Mode::Replaying }

    pub fn processControlQueue(&mut self) {
        while let Some(cb) = self.queue.pop() { cb(); }
    }
}

/// Per-frame pad write descriptor used by `writePadData`.
#[derive(Clone, Debug, Default)]
pub struct PadFrameWrite {
    pub frame: u32,
    pub port: u32,
    pub slot: u32,
    pub data: PadData,
}

const CONTROLLER_INPUT_BYTES: usize = 18;
const HEADER_SIZE: usize = std::mem::size_of::<u32>() * 2 + 64;
const INPUT_BYTES_PER_FRAME: usize = 2 * CONTROLLER_INPUT_BYTES;
const SEEKPOINT_TOTAL_FRAMES: u64 = 4;
const SEEKPOINT_UNDO_COUNT: u64 = 8;

/// File header for input recordings.
#[derive(Clone, Debug)]
pub struct InputRecordingFileHeader {
    pub file_version: u32,
    pub emulator_version: String,
    pub author: String,
    pub game_name: String,
}

impl InputRecordingFileHeader {
    pub fn init(&mut self) {
        self.file_version = 1;
        self.emulator_version.clear();
        self.author.clear();
        self.game_name.clear();
    }
}

impl Default for InputRecordingFileHeader {
    fn default() -> Self {
        let mut h = Self {
            file_version: 0,
            emulator_version: String::new(),
            author: String::new(),
            game_name: String::new(),
        };
        h.init();
        h
    }
}

/// The recording file wrapper.
pub struct InputRecordingFile {
    pub filename: String,
    pub header: InputRecordingFileHeader,
    pub total_frames: u32,
    pub undo_count: u32,
    pub savestate: bool,
    pub file: Option<File>,
}

impl Default for InputRecordingFile {
    fn default() -> Self {
        Self {
            filename: String::new(),
            header: InputRecordingFileHeader::default(),
            total_frames: 0,
            undo_count: 0,
            savestate: false,
            file: None,
        }
    }
}

impl InputRecordingFile {
    pub fn new() -> Self { Self::default() }

    pub fn setEmulatorVersion(&mut self, version: &str) {
        self.header.emulator_version = format!("PCSX2-{}", version);
    }
    pub fn setAuthor(&mut self, author: &str) { self.header.author = author.to_string(); }
    pub fn setGameName(&mut self, name: &str) { self.header.game_name = name.to_string(); }

    pub fn getEmulatorVersion(&self) -> &str { &self.header.emulator_version }
    pub fn getAuthor(&self) -> &str { &self.header.author }
    pub fn getGameName(&self) -> &str { &self.header.game_name }
    pub fn getFilename(&self) -> &str { &self.filename }
    pub fn getTotalFrames(&self) -> u32 { self.total_frames }
    pub fn getUndoCount(&self) -> u32 { self.undo_count }
    pub fn fromSaveState(&self) -> bool { self.savestate }

    /// Open a new recording file for writing.
    pub fn openNew(&mut self, path: &str, from_savestate: bool) -> std::io::Result<()> {
        let f = OpenOptions::new()
            .read(true).write(true).create(true).truncate(true)
            .open(path)?;
        self.filename = path.to_string();
        self.total_frames = 0;
        self.undo_count = 0;
        self.header.init();
        self.savestate = from_savestate;
        self.file = Some(f);
        Ok(())
    }

    /// Open an existing recording for reading/writing.
    pub fn openExisting(&mut self, path: &str) -> std::io::Result<()> {
        let f = OpenOptions::new().read(true).write(true).open(path)?;
        self.filename = path.to_string();
        self.file = Some(f);
        self.verifyRecordingFileHeader()
    }

    /// Close the underlying file.
    pub fn close(&mut self) -> bool {
        if self.file.is_none() { return false; }
        self.file = None;
        self.filename.clear();
        true
    }

    /// Read a single pad data block at the given frame.
    pub fn readPadData(&mut self, frame: u32, port: u32, slot: u32) -> Option<PadData> {
        let seek = self.getRecordingBlockSeekPoint(frame) + (CONTROLLER_INPUT_BYTES as u64) * (port as u64);
        let f = self.file.as_mut()?;
        f.seek(SeekFrom::Start(seek)).ok()?;
        let mut buf = [0u8; 18];
        if f.read_exact(&mut buf).is_err() { return None; }
        Some(PadData { port: port as i32, slot: slot as i32, buttons: u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]), rumble: [buf[4], buf[5]], analog: buf[6..22.min(buf.len())].to_vec().try_into().unwrap_or([0u8; 16]) })
    }

    /// Write a single pad data block at the given frame.
    pub fn writePadData(&mut self, frame: u32, data: &PadData) -> std::io::Result<()> {
        let seek = self.getRecordingBlockSeekPoint(frame)
            + (CONTROLLER_INPUT_BYTES as u64) * (data.port as u64);
        let f = self.file.as_mut().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::Other, "not open"))?;
        f.seek(SeekFrom::Start(seek))?;
        f.write_all(&data.buttons.to_le_bytes())?;
        f.write_all(&data.rumble)?;
        let mut analog = [0u8; 16];
        for (i, b) in data.analog.iter().enumerate() {
            if i < analog.len() { analog[i] = *b; }
        }
        f.write_all(&analog)?;
        f.flush()
    }

    /// Persist the file header, total frames, undo count, and savestate flag.
    pub fn writeHeader(&mut self) -> std::io::Result<()> {
        let f = self.file.as_mut().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::Other, "not open"))?;
        f.rewind()?;
        f.write_all(&self.header.file_version.to_le_bytes())?;
        let mut ver = [0u8; 32];
        let bytes = self.header.emulator_version.as_bytes();
        let n = bytes.len().min(ver.len());
        ver[..n].copy_from_slice(&bytes[..n]);
        f.write_all(&ver)?;
        let mut author = [0u8; 16];
        let bytes = self.header.author.as_bytes();
        let n = bytes.len().min(author.len());
        author[..n].copy_from_slice(&bytes[..n]);
        f.write_all(&author)?;
        let mut game = [0u8; 32];
        let bytes = self.header.game_name.as_bytes();
        let n = bytes.len().min(game.len());
        game[..n].copy_from_slice(&bytes[..n]);
        f.write_all(&game)?;
        f.write_all(&self.total_frames.to_le_bytes())?;
        f.write_all(&self.undo_count.to_le_bytes())?;
        f.write_all(&[self.savestate as u8])?;
        Ok(())
    }

    /// Update the recorded frame count on disk.
    pub fn setTotalFrames(&mut self, frame: u32) -> std::io::Result<()> {
        if self.file.is_none() { return Ok(()); }
        self.total_frames = frame;
        let f = self.file.as_mut().unwrap();
        f.seek(SeekFrom::Start(SEEKPOINT_TOTAL_FRAMES))?;
        f.write_all(&self.total_frames.to_le_bytes())
    }

    /// Bump the undo counter on disk.
    pub fn incrementUndoCount(&mut self) -> std::io::Result<()> {
        if self.file.is_none() { return Ok(()); }
        self.undo_count = self.undo_count.saturating_add(1);
        let f = self.file.as_mut().unwrap();
        f.seek(SeekFrom::Start(SEEKPOINT_UNDO_COUNT))?;
        f.write_all(&self.undo_count.to_le_bytes())
    }

    /// Read the header back from the file and verify the magic version.
    pub fn verifyRecordingFileHeader(&mut self) -> std::io::Result<()> {
        let f = self.file.as_mut().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::Other, "not open"))?;
        f.rewind()?;
        let mut ver = [0u8; 4];
        f.read_exact(&mut ver)?;
        self.header.file_version = u32::from_le_bytes(ver);
        if self.header.file_version != 1 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "unsupported recording file version"));
        }
        let mut buf4 = [0u8; 4];
        f.read_exact(&mut buf4)?; self.total_frames = u32::from_le_bytes(buf4);
        f.read_exact(&mut buf4)?; self.undo_count = u32::from_le_bytes(buf4);
        let mut b = [0u8; 1];
        f.read_exact(&mut b)?; self.savestate = b[0] != 0;
        Ok(())
    }

    /// Compute the seek offset for the given frame.
    pub fn getRecordingBlockSeekPoint(&self, frame: u32) -> u64 {
        (HEADER_SIZE as u64) + 1 + (frame as u64) * (INPUT_BYTES_PER_FRAME as u64)
    }

    /// Bulk-read a range of pad data for the given port.
    pub fn bulkReadPadData(&mut self, start: u32, end: u32, port: u32) -> Vec<PadData> {
        let mut out = Vec::new();
        if self.file.is_none() || end < start { return out; }
        for f in start..end {
            if let Some(p) = self.readPadData(f, port, 0) {
                out.push(p);
            }
        }
        out
    }

    /// Print the metadata block to the recording console log.
    pub fn logRecordingMetadata(&self) {
        consoleMultiLog(&[
            format!("File: {}", self.filename),
            format!("PCSX2 Version Used: {}", self.header.emulator_version),
            format!("Recording File Version: {}", self.header.file_version),
            format!("Associated Game Name or ISO Filename: {}", self.header.game_name),
            format!("Author: {}", self.header.author),
            format!("Total Frames: {}", self.total_frames),
            format!("Undo Count: {}", self.undo_count),
        ]);
    }
}

/// The high-level input recording controller.
pub struct InputRecording {
    pub file: InputRecordingFile,
    pub controls: InputRecordingControls,
    pub frame_counter: u32,
    pub frame_counter_stateless: u32,
    pub starting_frame: u32,
    pub recording_type: RecType,
    pub is_active: bool,
    pub initial_load_complete: bool,
    pub watching_for_rerecords: bool,
    pub record_queue: VecDeque<Box<dyn FnOnce() + Send + Sync>>,
}

impl Default for InputRecording {
    fn default() -> Self {
        Self {
            file: InputRecordingFile::new(),
            controls: InputRecordingControls::new(),
            frame_counter: 0,
            frame_counter_stateless: 0,
            starting_frame: 0,
            recording_type: RecType::PowerOn,
            is_active: false,
            initial_load_complete: false,
            watching_for_rerecords: false,
            record_queue: VecDeque::new(),
        }
    }
}

impl InputRecording {
    pub fn new() -> Self { Self::default() }

    /// Create a new recording on disk. `from_savestate` toggles the
    /// pre-existing-save-state recording format.
    pub fn create(&mut self, file_name: &str, from_save_state: bool, author: &str) -> std::io::Result<()> {
        self.file.openNew(file_name, from_save_state)?;
        self.controls.setRecordMode();
        if from_save_state {
            self.recording_type = RecType::FromSavestate;
            self.is_active = true;
            self.initial_load_complete = true;
            self.watching_for_rerecords = true;
            self.starting_frame = 0;
        } else {
            self.starting_frame = 0;
            self.recording_type = RecType::PowerOn;
            self.initial_load_complete = false;
            self.is_active = true;
        }
        self.file.setEmulatorVersion("git");
        self.file.setAuthor(author);
        self.file.setGameName("");
        self.file.writeHeader()?;
        self.initializeState();
        Ok(())
    }

    /// Open an existing recording for replay.
    pub fn play(&mut self, file_name: &str) -> std::io::Result<()> {
        self.file.openExisting(file_name)?;
        if self.file.fromSaveState() {
            self.recording_type = RecType::FromSavestate;
            self.initial_load_complete = false;
            self.is_active = true;
        } else {
            self.starting_frame = 0;
            self.recording_type = RecType::PowerOn;
            self.initial_load_complete = false;
            self.is_active = true;
        }
        self.controls.setReplayMode();
        self.initializeState();
        self.file.logRecordingMetadata();
        Ok(())
    }

    /// Stop recording (queued if mid-frame).
    pub fn stop(&mut self) {
        if self.is_active {
            self.closeActiveFile();
        }
    }

    /// Start a recording using the given file path as a backing store.
    pub fn start(&mut self, path: &Path) -> std::io::Result<()> {
        let path_str = path.to_string_lossy().into_owned();
        self.create(&path_str, false, "")
    }

    /// Save a snapshot of the current recording to the given path.
    pub fn save(&mut self, path: &Path) -> std::io::Result<()> {
        let mut copy = InputRecordingFile::new();
        copy.openNew(&path.to_string_lossy(), self.file.fromSaveState())?;
        copy.setEmulatorVersion(self.file.getEmulatorVersion());
        copy.setAuthor(self.file.getAuthor());
        copy.setGameName(self.file.getGameName());
        copy.total_frames = self.file.total_frames;
        copy.undo_count = self.file.undo_count;
        copy.savestate = self.file.savestate;
        copy.writeHeader()?;
        for f in 0..self.file.total_frames {
            if let Some(p) = self.file.readPadData(f, 0, 0) {
                copy.writePadData(f, &p)?;
            }
            if let Some(p) = self.file.readPadData(f, 1, 0) {
                copy.writePadData(f, &p)?;
            }
        }
        copy.close();
        Ok(())
    }

    /// Load a recording from disk.
    pub fn load(&mut self, path: &Path) -> std::io::Result<()> {
        self.play(&path.to_string_lossy())
    }

    /// Close the active recording, flushing pending writes.
    pub fn closeActiveFile(&mut self) {
        if !self.is_active { return; }
        if self.file.close() {
            self.is_active = false;
        }
    }

    /// Drain any queued recording operations (deferred close, etc.).
    pub fn processRecordQueue(&mut self) {
        while let Some(cb) = self.record_queue.pop_front() { cb(); }
    }

    /// Increment the per-frame counter and update replay/record side effects.
    pub fn incFrameCounter(&mut self) {
        if !self.is_active { return; }
        if self.frame_counter == u32::MAX {
            self.stop();
            return;
        }
        self.frame_counter = self.frame_counter.wrapping_add(1);
        if self.controls.isReplaying()
            && self.frame_counter == self.file.getTotalFrames()
        {
            self.watching_for_rerecords = false;
        }
        if self.controls.isRecording() {
            self.frame_counter_stateless = self.frame_counter_stateless.wrapping_add(1);
            let _ = self.file.setTotalFrames(self.frame_counter);
            if self.watching_for_rerecords {
                let _ = self.file.incrementUndoCount();
                self.watching_for_rerecords = false;
            }
        }
    }

    pub fn getFrameCounter(&self) -> u32 { self.frame_counter }
    pub fn getFrameCounterStateless(&self) -> u32 { self.frame_counter_stateless }
    pub fn isActive(&self) -> bool { self.is_active }
    pub fn isTypeSavestate(&self) -> bool { self.recording_type == RecType::FromSavestate }

    pub fn setStartingFrame(&mut self, frame: u32) {
        if self.recording_type == RecType::PowerOn { return; }
        self.starting_frame = frame;
    }
    pub fn getStartingFrame(&self) -> u32 { self.starting_frame }

    pub fn handleReset(&mut self) {
        if self.initial_load_complete {
            self.adjustFrameCounterOnReRecord(0);
        }
        self.initial_load_complete = true;
    }

    pub fn handleLoadingSavestate(&mut self) {
        if self.isTypeSavestate() && !self.initial_load_complete {
            self.setStartingFrame(0);
            self.initial_load_complete = true;
        } else {
            self.adjustFrameCounterOnReRecord(0);
            self.watching_for_rerecords = true;
        }
    }

    pub fn adjustFrameCounterOnReRecord(&mut self, new_frame: u32) {
        if new_frame > self.starting_frame + self.file.getTotalFrames() {
            self.frame_counter = self.file.getTotalFrames();
            return;
        }
        if new_frame < self.starting_frame {
            self.frame_counter = 0;
            return;
        }
        self.frame_counter = new_frame - self.starting_frame;
        if self.frame_counter_stateless > 0 {
            self.frame_counter_stateless -= 1;
        }
        let _ = self.file.setTotalFrames(self.frame_counter);
    }

    pub fn handleControllerDataUpdate(&mut self) {
        for i in 0..2 {
            let mut frame_data = PadData::new(i, 0);
            if self.is_active {
                if self.controls.isRecording() {
                    self.saveControllerData(&frame_data, i, 0);
                } else if self.controls.isReplaying() {
                    if let Some(p) = self.file.readPadData(self.frame_counter, i as u32, 0) {
                        p.OverrideActualController();
                        frame_data = p;
                    }
                }
            }
            frame_data.LogPadData();
        }
    }

    pub fn saveControllerData(&mut self, data: &PadData, port: i32, _slot: i32) {
        let _ = self.file.writePadData(self.frame_counter, data);
        let _ = port;
    }

    pub fn updateControllerData(&mut self, port: i32, _slot: i32) -> Option<PadData> {
        self.file.readPadData(self.frame_counter, port as u32, 0)
    }

    pub fn handleExceededFrameCounter(&mut self) {
        if self.frame_counter >= self.file.getTotalFrames() && self.controls.isReplaying() {
            self.controls.setRecordMode();
        }
    }

    pub fn initializeState(&mut self) {
        self.frame_counter = 0;
        self.watching_for_rerecords = false;
    }
}

// ---------------------------------------------------------------------------
// InputRecordingLogger equivalents
// ---------------------------------------------------------------------------

/// Log to the recording subsystem's console + OSD. Mirrors
/// `InputRec::log` in `InputRecordingLogger.cpp`.
pub fn log(msg: &str) {
    if msg.is_empty() { return; }
    eprintln!("[REC]: {}", msg);
}

/// Log only to the recording console.
pub fn consoleLog(msg: &str) {
    if msg.is_empty() { return; }
    eprintln!("[REC]: {}", msg);
}

/// Log several messages at once.
pub fn consoleMultiLog(messages: &[String]) {
    if messages.is_empty() { return; }
    for m in messages { eprintln!("[REC]: {}", m); }
}

// ---------------------------------------------------------------------------
// Convenience: a top-level static that mirrors the original `g_InputRecording`.
// ---------------------------------------------------------------------------

pub static mut g_InputRecording: InputRecording = InputRecording {
    file: InputRecordingFile {
        filename: String::new(),
        header: InputRecordingFileHeader {
            file_version: 1,
            emulator_version: String::new(),
            author: String::new(),
            game_name: String::new(),
        },
        total_frames: 0,
        undo_count: 0,
        savestate: false,
        file: None,
    },
    controls: InputRecordingControls {
        state: Mode::Idle,
        queue: Vec::new(),
    },
    frame_counter: 0,
    frame_counter_stateless: 0,
    starting_frame: 0,
    recording_type: RecType::PowerOn,
    is_active: false,
    initial_load_complete: false,
    watching_for_rerecords: false,
    record_queue: VecDeque::new(),
};
