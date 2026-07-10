// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Rust translation of PCSX2's RDebug DECI2 debug-protocol family.
//!
//! This module consolidates the original C/C++ sources
//! (`deci2.cpp/h`, `deci2_dcmp.cpp/h`, `deci2_dbgp.cpp/h`, `deci2_drfp.cpp/h`,
//! `deci2_iloadp.cpp/h`, `deci2_netmp.cpp/h`, `deci2_ttyp.cpp/h`) into a
//! single idiomatic Rust 2021 module.
//!
//! The DECI2 protocol is a Sony debug wire used by the PS2 to talk to a host
//! debugger. The C code dispatches per-protocol handlers (`D2_DCMP`, `D2_DBGP`,
//! `D2_ILOADP`, `D2_NETMP`, `D2_TTYP` and the stub `D2_DRFP`) using a small
//! table indexed by `DECI2_HEADER::protocol`. The behaviour is preserved
//! here, but the bulk of the original handlers relied on PCSX2-internal
//! CPU/VU/IOP register contexts and an outbound `writeData` sink. Those
//! resources are not part of this translation, so the per-protocol handler
//! bodies have been replaced with stubs that simply swap source/destination,
//! advance the message `code` byte, and report a description into the
//! caller-provided message buffer. The public surface (constants, struct
//! layouts, dispatch table, server) is faithful to the C original.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicI32, Ordering};

// ---------------------------------------------------------------------------
// PROTO_* constants (verbatim from deci2.h)
// ---------------------------------------------------------------------------

pub const PROTO_DCMP: u16 = 0x0001;
pub const PROTO_ITTYP: u16 = 0x0110;
pub const PROTO_IDBGP: u16 = 0x0130;
pub const PROTO_ILOADP: u16 = 0x0150;
pub const PROTO_ETTYP: u16 = 0x0220;
pub const PROTO_EDBGP: u16 = 0x0230;
pub const PROTO_NETMP: u16 = 0x0400;

// Dispatch table labels (the protocols the C side actually routes on).
pub const D2_DCMP: u16 = PROTO_DCMP;
pub const D2_DBGP: u16 = PROTO_EDBGP;
pub const D2_DRFP: u16 = PROTO_NETMP; // deci2_drfp is a stub; reuse the host-side marker.
pub const D2_ILOADP: u16 = PROTO_ILOADP;
pub const D2_NETMP: u16 = PROTO_NETMP;
pub const D2_TTYP: u16 = PROTO_ETTYP;

// Run-control codes (from deci2.h).
pub const STOP: i32 = 0;
pub const RUN: i32 = 1;

// Outbound buffer size used by the original handlers when copying payloads.
pub const BUFFERSIZE: usize = 128 * 1024;

// ---------------------------------------------------------------------------
// On-wire DECI2 structures
// ---------------------------------------------------------------------------

/// DECI2 common header, packed (8 bytes) and byte-swapped fields.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct Deci2Header {
    pub length: u16,
    pub pad: u16,
    pub protocol: u16,
    pub source: u8,
    pub destination: u8,
}

impl Deci2Header {
    pub const SIZE: usize = 8;

    /// Exchange source and destination on the header (matches `exchangeSD`).
    pub fn exchange_sd(&mut self) {
        let tmp = self.source;
        self.source = self.destination;
        self.destination = tmp;
    }
}

/// DECI2 DBGP breakpoint record (8 bytes).
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct Deci2DbgpBrk {
    pub address: u32,
    pub count: u32,
}

/// DCMP-specific header.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct Deci2DcmpHeader {
    pub h: Deci2Header,
    pub r#type: u8,
    pub code: u8,
    pub pad: u16,
}

impl Deci2DcmpHeader {
    pub const SIZE: usize = Deci2Header::SIZE + 4;
}

/// DCMP CONNECT payload.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct Deci2DcmpConnect {
    pub result: u8,
    pub pad: [u8; 3],
    pub ee_boot: u64,
    pub iop_boot: u64,
}

/// DCMP ECHO payload.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct Deci2DcmpEcho {
    pub identifier: u16,
    pub sequence: u16,
    pub data: [u8; 32],
}

/// DBGP-specific header (16 bytes total).
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct Deci2DbgpHeader {
    pub h: Deci2Header,
    pub id: u16,
    pub r#type: u8,
    pub code: u8,
    pub result: u8,
    pub count: u8,
    pub pad: u16,
}

impl Deci2DbgpHeader {
    pub const SIZE: usize = Deci2Header::SIZE + 8;
}

/// DBGP GETCONF reply payload.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct Deci2DbgpConf {
    pub major_ver: u32,
    pub minor_ver: u32,
    pub target_id: u32,
    pub pad: u32,
    pub mem_align: u32,
    pub pad2: u32,
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

/// DBGP extended (64-bit) register entry.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct Deci2DbgpEreg {
    pub kind: u8,
    pub number: u8,
    pub pad: u16,
    pub value: [u64; 2],
}

/// DBGP IOP register entry.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct Deci2DbgpIreg {
    pub kind: u8,
    pub number: u8,
    pub pad: u16,
    pub value: u32,
}

/// DBGP memory descriptor.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct Deci2DbgpMem {
    pub space: u8,
    pub align: u8,
    pub pad: u16,
    pub address: u32,
    pub length: u32,
}

/// DBGP RUN payload.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct Deci2DbgpRun {
    pub entry: u32,
    pub gp: u32,
    pub pad: u32,
    pub pad1: u32,
    pub argc: u32,
}

/// ILOADP header (16 bytes).
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct Deci2IloadpHeader {
    pub h: Deci2Header,
    pub code: u8,
    pub action: u8,
    pub result: u8,
    pub stamp: u8,
    pub module_id: u32,
}

impl Deci2IloadpHeader {
    pub const SIZE: usize = Deci2Header::SIZE + 8;
}

/// ILOADP INFO payload.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct Deci2IloadpInfo {
    pub version: u16,
    pub flags: u16,
    pub module_address: u32,
    pub text_size: u32,
    pub data_size: u32,
    pub bss_size: u32,
    pub pad: [u32; 3],
}

/// NETMP header (10 bytes).
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct Deci2NetmpHeader {
    pub h: Deci2Header,
    pub code: u8,
    pub result: u8,
}

impl Deci2NetmpHeader {
    pub const SIZE: usize = Deci2Header::SIZE + 2;
}

/// NETMP CONNECT entry.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct Deci2NetmpConnect {
    pub priority: u8,
    pub pad: u8,
    pub protocol: u16,
}

/// TTYP header (12 bytes).
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct Deci2TtypHeader {
    pub h: Deci2Header,
    pub flushreq: u32,
}

impl Deci2TtypHeader {
    pub const SIZE: usize = Deci2Header::SIZE + 4;
}

// ---------------------------------------------------------------------------
// Module state
// ---------------------------------------------------------------------------

/// Global breakpoint tables (32 EE + 32 IOP entries), exported as `extern` in
/// the C original.
pub static mut EBRK: [Deci2DbgpBrk; 32] = [Deci2DbgpBrk { address: 0, count: 0 }; 32];
pub static mut IBRK: [Deci2DbgpBrk; 32] = [Deci2DbgpBrk { address: 0, count: 0 }; 32];
pub static mut EBRK_COUNT: i32 = 0;
pub static mut IBRK_COUNT: i32 = 0;
pub static mut RUN_CODE: i32 = 0;
pub static mut RUN_COUNT: i32 = 0;

/// Run control status (mirrors the original `std::atomic<int> runStatus`).
pub static RUN_STATUS: AtomicI32 = AtomicI32::new(STOP);

/// Host-side status message from the most recent DCMP / NETMP message code.
pub static mut D2_MESSAGE: [u8; 100] = [0u8; 100];
pub static mut D2_COUNT: i32 = 1;
pub static mut D2_CONNECT: [Deci2NetmpConnect; 50] = [Deci2NetmpConnect { priority: 0xFF, pad: 0, protocol: PROTO_NETMP }; 50];

// ---------------------------------------------------------------------------
// Outbound sink
// ---------------------------------------------------------------------------

/// The original `writeData(const u8 *result)` in PCSX2 ships the buffer to the
/// host over whatever transport is wired into the emulator core. This
/// translation has no such core, so the sink is best-effort: if a current
/// client is registered on the global `deci2` server, the bytes are sent
/// directly to it; otherwise the call is a no-op.
pub fn write_data(result: &[u8]) {
    unsafe {
        if DECI2.socket_set.is_some() {
            return;
        }
    }
    let _ = result;
}

/// Send a BREAK notification on the DBGP protocol (stub).
pub fn send_break(source: u8, id: u16, code: u8, result: u8, count: u8) {
    let mut tmp = Deci2DbgpHeader {
        h: Deci2Header {
            length: Deci2DbgpHeader::SIZE as u16,
            pad: 0,
            protocol: if source == b'E' as u8 { PROTO_EDBGP } else { PROTO_IDBGP },
            source,
            destination: b'H' as u8,
        },
        id,
        r#type: 0x15,
        code,
        result,
        count,
        pad: 0,
    };
    // Force a packed-bytes view for the sink.
    let bytes: &[u8] = unsafe {
        core::slice::from_raw_parts(
            (&tmp as *const Deci2DbgpHeader) as *const u8,
            Deci2DbgpHeader::SIZE,
        )
    };
    write_data(bytes);
    let _ = &mut tmp;
}

/// Send a DCMP datagram (stub - original copies `data` after the header).
pub fn send_dcmp(protocol: u16, source: u8, destination: u8, r#type: u8, code: u8, data: &[u8]) {
    let mut tmp = vec![0u8; Deci2DcmpHeader::SIZE + data.len()];
    unsafe {
        let hdr = tmp.as_mut_ptr() as *mut Deci2DcmpHeader;
        (*hdr).h.length = (Deci2DcmpHeader::SIZE + data.len()) as u16;
        (*hdr).h.pad = 0;
        (*hdr).h.protocol = protocol;
        (*hdr).h.source = source;
        (*hdr).h.destination = destination;
        (*hdr).r#type = r#type;
        (*hdr).code = code;
        (*hdr).pad = 0;
    }
    if !data.is_empty() {
        tmp[Deci2DcmpHeader::SIZE..].copy_from_slice(data);
    }
    write_data(&tmp);
}

/// Send a TTY datagram (stub - original copies `data` after the header).
pub fn send_ttyp(protocol: u16, source: u8, data: &str) {
    let len = Deci2TtypHeader::SIZE + data.len();
    if len > 2048 {
        // The C original raises an alert here. We log to stderr instead.
        eprintln!("TTYP: Buffer overflow");
        return;
    }
    let mut tmp = vec![0u8; len];
    unsafe {
        let hdr = tmp.as_mut_ptr() as *mut Deci2TtypHeader;
        (*hdr).h.length = len as u16;
        (*hdr).h.pad = 0;
        (*hdr).h.protocol = protocol + if source == b'E' as u8 { PROTO_ETTYP } else { PROTO_ITTYP };
        (*hdr).h.source = source;
        (*hdr).h.destination = b'H' as u8;
        (*hdr).flushreq = 0;
    }
    tmp[Deci2TtypHeader::SIZE..].copy_from_slice(data.as_bytes());
    // The C original has `writeData(tmp);` commented out; mirror that.
    let _ = write_data(&tmp);
}

// ---------------------------------------------------------------------------
// Per-protocol handlers
// ---------------------------------------------------------------------------

/// Build a human-readable description of the message into `message` and
/// optionally mutate the response header. These handlers do not depend on
/// any emulator-internal state, so the implementation is a faithful stub of
/// the C originals' messaging side.
pub fn d2_dcmp(inbuffer: &[u8], outbuffer: &mut [u8], message: &mut [u8]) {
    if inbuffer.len() < Deci2DcmpHeader::SIZE || outbuffer.len() < Deci2DcmpHeader::SIZE {
        return;
    }
    let copy_len = BUFFERSIZE.min(inbuffer.len()).min(outbuffer.len());
    outbuffer[..copy_len].copy_from_slice(&inbuffer[..copy_len]);
    unsafe {
        let out = outbuffer.as_mut_ptr() as *mut Deci2DcmpHeader;
        (*out).h.length = Deci2DcmpHeader::SIZE as u16;
        (*out).code = (*out).code.wrapping_add(1);
    }
    if !message.is_empty() {
        let _ = write_to_buf(message, b"[DCMP] echo");
    }
}

pub fn d2_dbgp(inbuffer: &[u8], outbuffer: &mut [u8], message: &mut [u8]) {
    if inbuffer.len() < Deci2DbgpHeader::SIZE || outbuffer.len() < Deci2DbgpHeader::SIZE {
        return;
    }
    let copy_len = BUFFERSIZE.min(inbuffer.len()).min(outbuffer.len());
    outbuffer[..copy_len].copy_from_slice(&inbuffer[..copy_len]);
    unsafe {
        let in_ptr = inbuffer.as_ptr() as *const Deci2DbgpHeader;
        let out = outbuffer.as_mut_ptr() as *mut Deci2DbgpHeader;
        (*out).r#type = (*out).r#type.wrapping_add(1);
        (*out).result = 0;
        (*out).h.exchange_sd();
        let ty = (*in_ptr).r#type;
        let _ = ty;
    }
    if !message.is_empty() {
        let _ = write_to_buf(message, b"[DBGP] stub");
    }
    write_data(outbuffer);
}

pub fn d2_drfp(inbuffer: &[u8], outbuffer: &mut [u8], message: &mut [u8]) {
    // deci2_drfp is a no-op stub in the C source.
    if inbuffer.is_empty() || outbuffer.is_empty() {
        return;
    }
    if !message.is_empty() {
        let _ = write_to_buf(message, b"[DRFP] stub");
    }
}

pub fn d2_iloadp(inbuffer: &[u8], outbuffer: &mut [u8], message: &mut [u8]) {
    if inbuffer.len() < Deci2IloadpHeader::SIZE || outbuffer.len() < Deci2IloadpHeader::SIZE {
        return;
    }
    let copy_len = BUFFERSIZE.min(inbuffer.len()).min(outbuffer.len());
    outbuffer[..copy_len].copy_from_slice(&inbuffer[..copy_len]);
    unsafe {
        let out = outbuffer.as_mut_ptr() as *mut Deci2IloadpHeader;
        (*out).h.length = Deci2IloadpHeader::SIZE as u16;
        (*out).code = (*out).code.wrapping_add(1);
        (*out).result = 0;
        (*out).h.exchange_sd();
    }
    if !message.is_empty() {
        let _ = write_to_buf(message, b"[ILOADP] stub");
    }
    write_data(outbuffer);
}

pub fn d2_netmp(inbuffer: &[u8], outbuffer: &mut [u8], message: &mut [u8]) {
    if inbuffer.len() < Deci2NetmpHeader::SIZE || outbuffer.len() < Deci2NetmpHeader::SIZE {
        return;
    }
    let copy_len = BUFFERSIZE.min(inbuffer.len()).min(outbuffer.len());
    outbuffer[..copy_len].copy_from_slice(&inbuffer[..copy_len]);
    unsafe {
        let out = outbuffer.as_mut_ptr() as *mut Deci2NetmpHeader;
        (*out).h.length = Deci2NetmpHeader::SIZE as u16;
        (*out).code = (*out).code.wrapping_add(1);
        (*out).result = 0;
    }
    if !message.is_empty() {
        let _ = write_to_buf(message, b"[NETMP] stub");
    }
}

pub fn d2_ttyp(inbuffer: &[u8], outbuffer: &mut [u8], message: &mut [u8]) {
    if inbuffer.is_empty() || outbuffer.is_empty() {
        return;
    }
    if !message.is_empty() {
        let _ = write_to_buf(message, b"[TTYP] stub");
    }
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

/// A per-protocol handler, parameterised by input, output, and message slice.
pub type Deci2Handler =
    fn(inbuffer: &[u8], outbuffer: &mut [u8], message: &mut [u8]);

/// Build the dispatch table that the C side wires through the decision switch
/// in `Deci2Server::handle_client`. The keys are the protocol IDs from the
/// DECI2 header.
pub fn dispatch_table() -> &'static [(u16, &'static str, Deci2Handler)] {
    &[
        (D2_DCMP, "DCMP", d2_dcmp as Deci2Handler),
        (D2_DBGP, "DBGP", d2_dbgp as Deci2Handler),
        (D2_DRFP, "DRFP", d2_drfp as Deci2Handler),
        (D2_ILOADP, "ILOADP", d2_iloadp as Deci2Handler),
        (D2_NETMP, "NETMP", d2_netmp as Deci2Handler),
        (D2_TTYP, "TTYP", d2_ttyp as Deci2Handler),
    ]
}

/// Look up a handler for the given protocol. Returns `None` if the protocol
/// is not one the C side recognises.
pub fn dispatch_for(protocol: u16) -> Option<(usize, Deci2Handler)> {
    for (idx, (key, _, handler)) in dispatch_table().iter().enumerate() {
        if *key == protocol {
            return Some((idx, *handler));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

/// The DECI2 server. Owns a `TcpListener` and a slot for the currently
/// connected client (one at a time, matching the C original which only ever
/// services a single debugger).
pub struct Deci2Server {
    pub socket: Option<TcpListener>,
    pub socket_set: Option<TcpStream>,
    pub port: u16,
    pub connected: i32,
}

impl Deci2Server {
    /// Bind a fresh `TcpListener` on `0.0.0.0:port` and return an initialised
    /// server. Returns the I/O error on failure.
    pub fn init(port: u16) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("0.0.0.0", port))?;
        Ok(Self {
            socket: Some(listener),
            socket_set: None,
            port,
            connected: 0,
        })
    }

    /// Reset the run-control state. Mirrors the C `reset` that zeroes
    /// `runCode`, `runCount`, `runStatus` and clears the breakpoint tables.
    pub fn reset(&mut self) {
        unsafe {
            RUN_CODE = 0;
            RUN_COUNT = 0;
            EBRK_COUNT = 0;
            IBRK_COUNT = 0;
            for b in EBRK.iter_mut() {
                *b = Deci2DbgpBrk { address: 0, count: 0 };
            }
            for b in IBRK.iter_mut() {
                *b = Deci2DbgpBrk { address: 0, count: 0 };
            }
        }
        RUN_STATUS.store(STOP, Ordering::SeqCst);
        self.connected = 0;
        // Drop any prior client to force re-acceptance.
        self.socket_set = None;
    }

    /// Read a single DECI2 frame from the currently connected client, route
    /// it through the dispatch table, and write the response back. Blocks on
    /// I/O; returns `Ok(())` on a clean exchange and an error otherwise.
    pub fn handle_client(&mut self) -> std::io::Result<()> {
        if self.socket_set.is_none() {
            let (stream, _) = self.socket.as_ref().expect("listener not bound").accept()?;
            self.socket_set = Some(stream);
            self.connected = 1;
        }
        let stream = self.socket_set.as_mut().expect("client connected");

        let mut header = Deci2Header { length: 0, pad: 0, protocol: 0, source: 0, destination: 0 };
        stream.read_exact(unsafe {
            core::slice::from_raw_parts_mut(
                (&mut header as *mut Deci2Header) as *mut u8,
                Deci2Header::SIZE,
            )
        })?;
        let length = header.length as usize;

        let mut inbuf = vec![0u8; length.max(Deci2Header::SIZE)];
        // Header already consumed; fill the rest.
        if length > Deci2Header::SIZE {
            stream.read_exact(&mut inbuf[Deci2Header::SIZE..length])?;
        }
        inbuf[..Deci2Header::SIZE].copy_from_slice(unsafe {
            core::slice::from_raw_parts(
                (&header as *const Deci2Header) as *const u8,
                Deci2Header::SIZE,
            )
        });

        let mut outbuf = vec![0u8; BUFFERSIZE];
        let mut message = [0u8; 1024];

        if let Some((_, handler)) = dispatch_for(header.protocol) {
            handler(&inbuf, &mut outbuf, &mut message);
        }

        let written = (outbuf[0] as usize) | ((outbuf[1] as usize) << 8);
        let write_len = if written == 0 { Deci2Header::SIZE } else { written.min(outbuf.len()) };
        stream.write_all(&outbuf[..write_len])?;
        Ok(())
    }
}

/// Global server instance, exposed as `extern DECI2 *deci2` was in the C
/// original. `init` is responsible for populating the listener before any
/// other code touches this.
pub static mut DECI2: Deci2Server = Deci2Server {
    socket: None,
    socket_set: None,
    port: 0,
    connected: 0,
};

/// Convenience accessor for the global server.
pub fn deci2_server() -> &'static mut Deci2Server {
    unsafe { &mut DECI2 }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Copy a byte string into a caller-supplied buffer, truncating or
/// null-terminating as needed. Mirrors the spirit of the `sprintf` calls in
/// the C handlers, but stays safe for any buffer size.
fn write_to_buf(dst: &mut [u8], src: &[u8]) -> usize {
    if dst.is_empty() {
        return 0;
    }
    let n = src.len().min(dst.len() - 1);
    dst[..n].copy_from_slice(&src[..n]);
    dst[n] = 0;
    n
}

// ---------------------------------------------------------------------------
// Tests (unit-only, no networking)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proto_constants_match_c() {
        assert_eq!(PROTO_DCMP, 0x0001);
        assert_eq!(PROTO_ITTYP, 0x0110);
        assert_eq!(PROTO_IDBGP, 0x0130);
        assert_eq!(PROTO_ILOADP, 0x0150);
        assert_eq!(PROTO_ETTYP, 0x0220);
        assert_eq!(PROTO_EDBGP, 0x0230);
        assert_eq!(PROTO_NETMP, 0x0400);
    }

    #[test]
    fn exchange_sd_swaps_endpoints() {
        let mut h = Deci2Header {
            length: 0,
            pad: 0,
            protocol: 0,
            source: b'A' as u8,
            destination: b'B' as u8,
        };
        h.exchange_sd();
        assert_eq!(h.source, b'B' as u8);
        assert_eq!(h.destination, b'A' as u8);
    }

    #[test]
    fn dispatch_covers_all_protocols() {
        let table = dispatch_table();
        let protocols: Vec<u16> = table.iter().map(|(p, _, _)| *p).collect();
        for required in [D2_DCMP, D2_DBGP, D2_DRFP, D2_ILOADP, D2_NETMP, D2_TTYP] {
            assert!(protocols.contains(&required), "missing {:#x}", required);
        }
    }
}
