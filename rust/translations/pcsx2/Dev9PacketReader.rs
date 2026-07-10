//! Network packet reader for PCSX2's DEV9 emulation.
//!
//! Idiomatic Rust translation of the C++ `PacketReader` subsystem. Parses and
//! serializes the on-wire protocols that the PS2's network adapter can
//! produce: Ethernet II, ARP, IPv4, ICMPv4, TCP, UDP, BOOTP/DHCP and DNS.
//!
//! Each protocol is provided as a fully-decoded struct (`from_bytes` /
//! `to_bytes`) and a `*Editor` view that gives typed getters/setters over a
//! borrowed mutable byte buffer, mirroring the original C++ *Editor classes
//! used for in-place rewriting of captured packets.
//!
//! The original implementation lived in `pcsx2/DEV9/PacketReader/` and used a
//! `Payload` type-erased base for nested protocols. The Rust version simply
//! inlines the inner payload as a `Vec<u8>` (or `&[u8]` for the editors), and
//! replaces the polymorphic `*Option` hierarchies with enums (`IpOption`,
//! `TcpOption`, `DhcpOption`).
//!
//! All integer fields are kept in network byte order (big-endian) on the
//! wire; getters/setters perform the conversion for you.

#![allow(clippy::upper_case_acronyms)]

use std::convert::TryInto;

// -----------------------------------------------------------------------------
// Common types
// -----------------------------------------------------------------------------

/// Ethernet MAC address (6 bytes, big-endian on the wire).
pub type MacAddress = [u8; 6];
/// IPv4 address (4 bytes, network byte order).
pub type IpAddress = [u8; 4];

// -----------------------------------------------------------------------------
// Internal helpers (translated from `NetLib`)
// -----------------------------------------------------------------------------

#[inline]
fn read_u8(buf: &[u8], off: &mut usize) -> u8 {
    let v = buf[*off];
    *off += 1;
    v
}

#[inline]
fn read_u16(buf: &[u8], off: &mut usize) -> u16 {
    let v = u16::from_be_bytes(
        buf[*off..*off + 2]
            .try_into()
            .expect("read_u16: buffer too short"),
    );
    *off += 2;
    v
}

#[inline]
fn read_u32(buf: &[u8], off: &mut usize) -> u32 {
    let v = u32::from_be_bytes(
        buf[*off..*off + 4]
            .try_into()
            .expect("read_u32: buffer too short"),
    );
    *off += 4;
    v
}

#[inline]
fn write_u8(buf: &mut [u8], off: &mut usize, v: u8) {
    buf[*off] = v;
    *off += 1;
}

#[inline]
fn write_u16(buf: &mut [u8], off: &mut usize, v: u16) {
    buf[*off..*off + 2].copy_from_slice(&v.to_be_bytes());
    *off += 2;
}

#[inline]
fn write_u32(buf: &mut [u8], off: &mut usize, v: u32) {
    buf[*off..*off + 4].copy_from_slice(&v.to_be_bytes());
    *off += 4;
}

#[inline]
fn write_bytes(buf: &mut [u8], off: &mut usize, src: &[u8]) {
    buf[*off..*off + src.len()].copy_from_slice(src);
    *off += src.len();
}

#[inline]
fn align_up(value: usize, align: usize) -> usize {
    debug_assert!(align.is_power_of_two());
    (value + align - 1) & !(align - 1)
}

/// Standard Internet checksum (RFC 1071) -- translated from
/// `IP_Packet::InternetChecksum`.
pub fn internet_checksum(buf: &[u8], length: usize) -> u16 {
    let mut i = 0;
    let mut len = length;
    let mut sum: u32 = 0;
    while len > 1 {
        let hi = (buf[i] as u32) << 8;
        let lo = (buf[i + 1] as u32) & 0xFF;
        sum = sum.wrapping_add(hi | lo);
        if (sum & 0xFFFF_0000) != 0 {
            sum = (sum & 0xFFFF).wrapping_add(1);
        }
        i += 2;
        len -= 2;
    }
    if len > 0 {
        sum = sum.wrapping_add((buf[i] as u32) << 8);
        if (sum & 0xFFFF_0000) != 0 {
            sum = (sum & 0xFFFF).wrapping_add(1);
        }
    }
    (!(sum & 0xFFFF)) as u16
}

// =============================================================================
// Ethernet II frame
// =============================================================================

/// Length of the Ethernet II header (6 + 6 + 2 = 14 bytes).
pub const ETHERNET_HEADER_LEN: usize = 14;

/// A parsed Ethernet II frame.
pub struct EthernetFrame {
    pub dst: [u8; 6],
    pub src: [u8; 6],
    pub type_: u16,
    pub payload: Vec<u8>,
}

impl EthernetFrame {
    /// Parse an Ethernet II frame from the given buffer.
    pub fn from_bytes(buf: &[u8]) -> Self {
        let mut off = 0;
        let dst: [u8; 6] = buf[off..off + 6].try_into().expect("EthernetFrame: short dst");
        off += 6;
        let src: [u8; 6] = buf[off..off + 6].try_into().expect("EthernetFrame: short src");
        off += 6;
        let type_ = read_u16(buf, &mut off);
        let payload = buf[off..].to_vec();
        Self { dst, src, type_, payload }
    }

    /// Serialize the frame into a freshly allocated byte vector.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(ETHERNET_HEADER_LEN + self.payload.len());
        v.extend_from_slice(&self.dst);
        v.extend_from_slice(&self.src);
        v.extend_from_slice(&self.type_.to_be_bytes());
        v.extend_from_slice(&self.payload);
        v
    }

    /// Construct an editor view over a mutable backing buffer.
    pub fn editor<'a>(buffer: &'a mut [u8]) -> EthernetFrameEditor<'a> {
        EthernetFrameEditor::new(buffer)
    }
}

/// Mutable in-place editor for an Ethernet II frame held in a borrowed buffer.
pub struct EthernetFrameEditor<'a> {
    buffer: &'a mut [u8],
}

impl<'a> EthernetFrameEditor<'a> {
    pub fn new(buffer: &'a mut [u8]) -> Self {
        assert!(
            buffer.len() >= ETHERNET_HEADER_LEN,
            "EthernetFrameEditor: buffer smaller than header"
        );
        Self { buffer }
    }

    pub fn dst(&self) -> [u8; 6] {
        self.buffer[0..6].try_into().unwrap()
    }
    pub fn set_dst(&mut self, v: [u8; 6]) {
        self.buffer[0..6].copy_from_slice(&v);
    }

    pub fn src(&self) -> [u8; 6] {
        self.buffer[6..12].try_into().unwrap()
    }
    pub fn set_src(&mut self, v: [u8; 6]) {
        self.buffer[6..12].copy_from_slice(&v);
    }

    pub fn type_(&self) -> u16 {
        u16::from_be_bytes(self.buffer[12..14].try_into().unwrap())
    }
    pub fn set_type(&mut self, v: u16) {
        self.buffer[12..14].copy_from_slice(&v.to_be_bytes());
    }

    pub fn payload(&self) -> &[u8] {
        &self.buffer[ETHERNET_HEADER_LEN..]
    }
    pub fn payload_mut(&mut self) -> &mut [u8] {
        &mut self.buffer[ETHERNET_HEADER_LEN..]
    }
}

// =============================================================================
// ARP
// =============================================================================

/// An ARP packet (RFC 826).
pub struct ArpPacket {
    pub hardware_type: u16,
    pub protocol: u16,
    pub hardware_address_length: u8,
    pub protocol_address_length: u8,
    pub op: u16,
    pub sender_hardware_address: Vec<u8>,
    pub sender_protocol_address: Vec<u8>,
    pub target_hardware_address: Vec<u8>,
    pub target_protocol_address: Vec<u8>,
}

impl ArpPacket {
    /// Parse an ARP packet from the given buffer.
    pub fn from_bytes(buf: &[u8]) -> Self {
        let mut off = 0;
        let hardware_type = read_u16(buf, &mut off);
        let protocol = read_u16(buf, &mut off);
        let hardware_address_length = read_u8(buf, &mut off);
        let protocol_address_length = read_u8(buf, &mut off);
        let op = read_u16(buf, &mut off);

        let mut sender_hardware_address = vec![0u8; hardware_address_length as usize];
        let mut sender_protocol_address = vec![0u8; protocol_address_length as usize];
        let mut target_hardware_address = vec![0u8; hardware_address_length as usize];
        let mut target_protocol_address = vec![0u8; protocol_address_length as usize];

        sender_hardware_address.copy_from_slice(&buf[off..off + hardware_address_length as usize]);
        off += hardware_address_length as usize;
        sender_protocol_address.copy_from_slice(&buf[off..off + protocol_address_length as usize]);
        off += protocol_address_length as usize;
        target_hardware_address.copy_from_slice(&buf[off..off + hardware_address_length as usize]);
        off += hardware_address_length as usize;
        target_protocol_address.copy_from_slice(&buf[off..off + protocol_address_length as usize]);

        Self {
            hardware_type,
            protocol,
            hardware_address_length,
            protocol_address_length,
            op,
            sender_hardware_address,
            sender_protocol_address,
            target_hardware_address,
            target_protocol_address,
        }
    }

    /// Construct an ARP packet with the given hardware/protocol address sizes
    /// (all sender/target buffers are allocated but zero-filled).
    pub fn new(hardware_address_length: u8, protocol_address_length: u8) -> Self {
        Self {
            hardware_type: 0,
            protocol: 0,
            hardware_address_length,
            protocol_address_length,
            op: 0,
            sender_hardware_address: vec![0; hardware_address_length as usize],
            sender_protocol_address: vec![0; protocol_address_length as usize],
            target_hardware_address: vec![0; hardware_address_length as usize],
            target_protocol_address: vec![0; protocol_address_length as usize],
        }
    }

    /// On-wire length in bytes (including fixed 8-byte header).
    pub fn length(&self) -> usize {
        8 + 2 * self.hardware_address_length as usize + 2 * self.protocol_address_length as usize
    }

    /// Serialize the packet into a freshly allocated byte vector.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = vec![0u8; self.length()];
        self.write_bytes(&mut buf, &mut 0);
        buf
    }

    /// Serialize the packet into the provided buffer at `off`.
    pub fn write_bytes(&self, buf: &mut [u8], off: &mut usize) {
        write_u16(buf, off, self.hardware_type);
        write_u16(buf, off, self.protocol);
        write_u8(buf, off, self.hardware_address_length);
        write_u8(buf, off, self.protocol_address_length);
        write_u16(buf, off, self.op);
        write_bytes(buf, off, &self.sender_hardware_address);
        write_bytes(buf, off, &self.sender_protocol_address);
        write_bytes(buf, off, &self.target_hardware_address);
        write_bytes(buf, off, &self.target_protocol_address);
    }

    /// Construct an editor view over a mutable backing buffer.
    pub fn editor<'a>(buffer: &'a mut [u8]) -> ArpPacketEditor<'a> {
        ArpPacketEditor::new(buffer)
    }
}

/// Mutable in-place editor for an ARP packet held in a borrowed buffer.
pub struct ArpPacketEditor<'a> {
    buffer: &'a mut [u8],
}

impl<'a> ArpPacketEditor<'a> {
    pub fn new(buffer: &'a mut [u8]) -> Self {
        assert!(buffer.len() >= 8, "ArpPacketEditor: buffer smaller than 8 bytes");
        Self { buffer }
    }

    pub fn hardware_type(&self) -> u16 {
        u16::from_be_bytes(self.buffer[0..2].try_into().unwrap())
    }
    pub fn set_hardware_type(&mut self, v: u16) {
        self.buffer[0..2].copy_from_slice(&v.to_be_bytes());
    }

    pub fn protocol(&self) -> u16 {
        u16::from_be_bytes(self.buffer[2..4].try_into().unwrap())
    }
    pub fn set_protocol(&mut self, v: u16) {
        self.buffer[2..4].copy_from_slice(&v.to_be_bytes());
    }

    pub fn hardware_address_length(&self) -> u8 {
        self.buffer[4]
    }
    pub fn protocol_address_length(&self) -> u8 {
        self.buffer[5]
    }

    pub fn op(&self) -> u16 {
        u16::from_be_bytes(self.buffer[6..8].try_into().unwrap())
    }
    pub fn set_op(&mut self, v: u16) {
        self.buffer[6..8].copy_from_slice(&v.to_be_bytes());
    }

    pub fn sender_hardware_address(&self) -> &[u8] {
        let len = self.hardware_address_length() as usize;
        &self.buffer[8..8 + len]
    }
    pub fn set_sender_hardware_address(&mut self, v: &[u8]) {
        let len = self.hardware_address_length() as usize;
        self.buffer[8..8 + len].copy_from_slice(&v[..len]);
    }

    pub fn sender_protocol_address(&self) -> &[u8] {
        let off = 8 + self.hardware_address_length() as usize;
        let len = self.protocol_address_length() as usize;
        &self.buffer[off..off + len]
    }
    pub fn set_sender_protocol_address(&mut self, v: &[u8]) {
        let off = 8 + self.hardware_address_length() as usize;
        let len = self.protocol_address_length() as usize;
        self.buffer[off..off + len].copy_from_slice(&v[..len]);
    }

    pub fn target_hardware_address(&self) -> &[u8] {
        let off = 8 + self.hardware_address_length() as usize + self.protocol_address_length() as usize;
        let len = self.hardware_address_length() as usize;
        &self.buffer[off..off + len]
    }
    pub fn set_target_hardware_address(&mut self, v: &[u8]) {
        let off = 8 + self.hardware_address_length() as usize + self.protocol_address_length() as usize;
        let len = self.hardware_address_length() as usize;
        self.buffer[off..off + len].copy_from_slice(&v[..len]);
    }

    pub fn target_protocol_address(&self) -> &[u8] {
        let off = 8 + 2 * self.hardware_address_length() as usize + self.protocol_address_length() as usize;
        let len = self.protocol_address_length() as usize;
        &self.buffer[off..off + len]
    }
    pub fn set_target_protocol_address(&mut self, v: &[u8]) {
        let off = 8 + 2 * self.hardware_address_length() as usize + self.protocol_address_length() as usize;
        let len = self.protocol_address_length() as usize;
        self.buffer[off..off + len].copy_from_slice(&v[..len]);
    }

    /// Total on-wire length.
    pub fn length(&self) -> usize {
        8 + 2 * self.hardware_address_length() as usize + 2 * self.protocol_address_length() as usize
    }
}

// =============================================================================
// IPv4
// =============================================================================

/// Upper nibble of the first byte of an IPv4 header -- version 4.
pub const IP_VERSION_4: u8 = 0x40;
/// Fixed part of an IPv4 header (20 bytes).
pub const IP_HEADER_MIN_LEN: usize = 20;

/// An IPv4 option (translated from the C++ `IPOption` hierarchy).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpOption {
    /// 1-byte NOP (kind = 1).
    Nop,
    /// Router Alert (kind = 148, length = 4).
    RouterAlert { value: u16 },
    /// Unrecognised option (kind != 0, 1, 148).
    Unknown { code: u8, value: Vec<u8> },
}

impl IpOption {
    pub fn code(&self) -> u8 {
        match self {
            IpOption::Nop => 1,
            IpOption::RouterAlert { .. } => 148,
            IpOption::Unknown { code, .. } => *code,
        }
    }

    /// On-wire length in bytes.
    pub fn length(&self) -> usize {
        match self {
            IpOption::Nop => 1,
            IpOption::RouterAlert { .. } => 4,
            IpOption::Unknown { value, .. } => 2 + value.len(),
        }
    }

    fn write_bytes(&self, buf: &mut [u8], off: &mut usize) {
        match self {
            IpOption::Nop => write_u8(buf, off, 1),
            IpOption::RouterAlert { value } => {
                write_u8(buf, off, 148);
                write_u8(buf, off, 4);
                write_u16(buf, off, *value);
            }
            IpOption::Unknown { code, value } => {
                write_u8(buf, off, *code);
                write_u8(buf, off, (2 + value.len()) as u8);
                write_bytes(buf, off, value);
            }
        }
    }
}

/// A parsed IPv4 packet.
pub struct IpPacket {
    pub dscp_ecn: u8,
    pub identification: u16,
    pub fragment_flags1: u8,
    pub fragment_flags2: u8,
    pub time_to_live: u8,
    pub protocol: u8,
    pub checksum: u16,
    pub source_ip: IpAddress,
    pub destination_ip: IpAddress,
    pub options: Vec<IpOption>,
    pub payload: Vec<u8>,
}

impl IpPacket {
    /// Parse an IPv4 packet from the given buffer.
    pub fn from_bytes(buf: &[u8]) -> Self {
        Self::from_bytes_inner(buf, false)
    }

    fn from_bytes_inner(buf: &[u8], from_icmp: bool) -> Self {
        let mut off = 0;
        let v_hl = read_u8(buf, &mut off);
        let header_length = ((v_hl & 0x0F) as usize) << 2;
        let dscp_ecn = read_u8(buf, &mut off);
        let mut length = read_u16(buf, &mut off) as usize;
        if length > buf.len() {
            if !from_icmp {
                // Matches the original `Console.Error` warning; in Rust we
                // silently truncate to the actual buffer.
            }
            length = buf.len();
        }
        let identification = read_u16(buf, &mut off);
        let fragment_flags1 = read_u8(buf, &mut off);
        let fragment_flags2 = read_u8(buf, &mut off);
        let time_to_live = read_u8(buf, &mut off);
        let protocol = read_u8(buf, &mut off);
        let checksum = read_u16(buf, &mut off);
        let source_ip: IpAddress = buf[off..off + 4].try_into().expect("IpPacket: short src ip");
        off += 4;
        let destination_ip: IpAddress = buf[off..off + 4].try_into().expect("IpPacket: short dst ip");
        off += 4;

        let mut options = Vec::new();
        if header_length > IP_HEADER_MIN_LEN {
            let mut ooff = IP_HEADER_MIN_LEN;
            while ooff < header_length {
                let op_kind = buf[ooff];
                if op_kind == 0 {
                    break;
                }
                if op_kind == 1 {
                    options.push(IpOption::Nop);
                    ooff += 1;
                } else {
                    let op_len = buf[ooff + 1] as usize;
                    let opt = match op_kind {
                        148 => {
                            let v = u16::from_be_bytes(
                                buf[ooff + 2..ooff + 4].try_into().unwrap(),
                            );
                            IpOption::RouterAlert { value: v }
                        }
                        _ => {
                            let value_start = ooff + 2;
                            let value_end = (ooff + op_len).min(buf.len());
                            let value = if value_end > value_start {
                                buf[value_start..value_end].to_vec()
                            } else {
                                Vec::new()
                            };
                            IpOption::Unknown {
                                code: op_kind,
                                value,
                            }
                        }
                    };
                    options.push(opt);
                    ooff += op_len;
                }
            }
        }

        let payload_start = header_length.min(length).min(buf.len());
        let payload = buf[payload_start..length.min(buf.len())].to_vec();

        Self {
            dscp_ecn,
            identification,
            fragment_flags1,
            fragment_flags2,
            time_to_live,
            protocol,
            checksum,
            source_ip,
            destination_ip,
            options,
            payload,
        }
    }

    /// Compute the IPv4 header length (in bytes) for the current options.
    pub fn header_length(&self) -> usize {
        let mut ooff = IP_HEADER_MIN_LEN;
        for o in &self.options {
            ooff += o.length();
        }
        align_up(ooff, 4)
    }

    /// Total on-wire length in bytes.
    pub fn length(&self) -> usize {
        self.header_length() + self.payload.len()
    }

    /// Serialize the packet into a freshly allocated byte vector.
    pub fn to_bytes(&mut self) -> Vec<u8> {
        let total = self.length();
        let mut buf = vec![0u8; total];
        let mut off = 0;
        self.write_bytes(&mut buf, &mut off);
        debug_assert_eq!(off, total);
        buf
    }

    /// Serialize the packet into the provided buffer at `off`.  Computes the
    /// IPv4 header checksum on the way out.
    pub fn write_bytes(&mut self, buf: &mut [u8], off: &mut usize) {
        self.calculate_checksum();
        let header_length = self.header_length();

        let start = *off;
        write_u8(buf, off, IP_VERSION_4 | ((header_length >> 2) as u8));
        write_u8(buf, off, self.dscp_ecn);
        write_u16(buf, off, self.length() as u16);
        write_u16(buf, off, self.identification);
        write_u8(buf, off, self.fragment_flags1);
        write_u8(buf, off, self.fragment_flags2);
        write_u8(buf, off, self.time_to_live);
        write_u8(buf, off, self.protocol);
        write_u16(buf, off, self.checksum);
        write_bytes(buf, off, &self.source_ip);
        write_bytes(buf, off, &self.destination_ip);

        let opts_start = *off;
        for o in &self.options {
            o.write_bytes(buf, off);
        }
        if *off != opts_start + (header_length - IP_HEADER_MIN_LEN) {
            // pad with zeros up to the header length
            let pad = (start + header_length) - *off;
            for b in &mut buf[*off..*off + pad] {
                *b = 0;
            }
            *off += pad;
        }
        *off = start + header_length;

        write_bytes(buf, off, &self.payload);
    }

    /// Recompute the IPv4 header checksum and store it in `self.checksum`.
    pub fn calculate_checksum(&mut self) {
        let header_length = self.header_length();
        let mut segment = vec![0u8; header_length];
        let mut counter = 0;
        write_u8(&mut segment, &mut counter, IP_VERSION_4 | ((header_length >> 2) as u8));
        write_u8(&mut segment, &mut counter, self.dscp_ecn);
        write_u16(&mut segment, &mut counter, self.length() as u16);
        write_u16(&mut segment, &mut counter, self.identification);
        write_u8(&mut segment, &mut counter, self.fragment_flags1);
        write_u8(&mut segment, &mut counter, self.fragment_flags2);
        write_u8(&mut segment, &mut counter, self.time_to_live);
        write_u8(&mut segment, &mut counter, self.protocol);
        write_u16(&mut segment, &mut counter, 0); // checksum field zero
        write_bytes(&mut segment, &mut counter, &self.source_ip);
        write_bytes(&mut segment, &mut counter, &self.destination_ip);
        for o in &self.options {
            o.write_bytes(&mut segment, &mut counter);
        }
        if counter != header_length {
            for b in &mut segment[counter..header_length] {
                *b = 0;
            }
        }
        self.checksum = internet_checksum(&segment, header_length);
    }

    /// Returns `true` if the on-wire header checksum is correct.
    pub fn verify_checksum(&self) -> bool {
        let header_length = self.header_length();
        let mut segment = vec![0u8; header_length];
        let mut counter = 0;
        write_u8(&mut segment, &mut counter, IP_VERSION_4 | ((header_length >> 2) as u8));
        write_u8(&mut segment, &mut counter, self.dscp_ecn);
        write_u16(&mut segment, &mut counter, self.length() as u16);
        write_u16(&mut segment, &mut counter, self.identification);
        write_u8(&mut segment, &mut counter, self.fragment_flags1);
        write_u8(&mut segment, &mut counter, self.fragment_flags2);
        write_u8(&mut segment, &mut counter, self.time_to_live);
        write_u8(&mut segment, &mut counter, self.protocol);
        write_u16(&mut segment, &mut counter, self.checksum);
        write_bytes(&mut segment, &mut counter, &self.source_ip);
        write_bytes(&mut segment, &mut counter, &self.destination_ip);
        for o in &self.options {
            o.write_bytes(&mut segment, &mut counter);
        }
        if counter != header_length {
            for b in &mut segment[counter..header_length] {
                *b = 0;
            }
        }
        internet_checksum(&segment, header_length) == 0
    }

    /// DSCP value (6 bits, upper part of `dscp_ecn`).
    pub fn dscp(&self) -> u8 {
        (self.dscp_ecn >> 2) & 0x3F
    }
    pub fn set_dscp(&mut self, v: u8) {
        self.dscp_ecn = (self.dscp_ecn & !(0x3F << 2)) | ((v & 0x3F) << 2);
    }
    /// ECN value (2 bits, lower part of `dscp_ecn`).
    pub fn ecn(&self) -> u8 {
        self.dscp_ecn & 0x3
    }
    pub fn set_ecn(&mut self, v: u8) {
        self.dscp_ecn = (self.dscp_ecn & !0x3) | (v & 0x3);
    }
    pub fn do_not_fragment(&self) -> bool {
        (self.fragment_flags1 & (1 << 6)) != 0
    }
    pub fn set_do_not_fragment(&mut self, v: bool) {
        self.fragment_flags1 = (self.fragment_flags1 & !(1 << 6)) | (((v as u8) & 0x1) << 6);
    }
    pub fn more_fragments(&self) -> bool {
        (self.fragment_flags1 & (1 << 5)) != 0
    }
    pub fn set_more_fragments(&mut self, v: bool) {
        self.fragment_flags1 = (self.fragment_flags1 & !(1 << 5)) | (((v as u8) & 0x1) << 5);
    }
    pub fn fragment_offset(&self) -> u16 {
        let lo = self.fragment_flags1 & 0x1F;
        let bytes = [lo, self.fragment_flags2];
        u16::from_be_bytes(bytes)
    }

    /// Construct an editor view over a mutable backing buffer.
    pub fn editor<'a>(buffer: &'a mut [u8]) -> IpPacketEditor<'a> {
        IpPacketEditor::new(buffer)
    }
}

/// Mutable in-place editor for an IPv4 packet held in a borrowed buffer.
pub struct IpPacketEditor<'a> {
    buffer: &'a mut [u8],
}

impl<'a> IpPacketEditor<'a> {
    pub fn new(buffer: &'a mut [u8]) -> Self {
        assert!(
            buffer.len() >= IP_HEADER_MIN_LEN,
            "IpPacketEditor: buffer smaller than 20 bytes"
        );
        Self { buffer }
    }

    pub fn version_and_header_length(&self) -> u8 {
        self.buffer[0]
    }
    pub fn set_version_and_header_length(&mut self, v: u8) {
        self.buffer[0] = v;
    }
    pub fn dscp_ecn(&self) -> u8 {
        self.buffer[1]
    }
    pub fn set_dscp_ecn(&mut self, v: u8) {
        self.buffer[1] = v;
    }
    pub fn total_length(&self) -> u16 {
        u16::from_be_bytes(self.buffer[2..4].try_into().unwrap())
    }
    pub fn set_total_length(&mut self, v: u16) {
        self.buffer[2..4].copy_from_slice(&v.to_be_bytes());
    }
    pub fn identification(&self) -> u16 {
        u16::from_be_bytes(self.buffer[4..6].try_into().unwrap())
    }
    pub fn set_identification(&mut self, v: u16) {
        self.buffer[4..6].copy_from_slice(&v.to_be_bytes());
    }
    pub fn fragment_flags1(&self) -> u8 {
        self.buffer[6]
    }
    pub fn set_fragment_flags1(&mut self, v: u8) {
        self.buffer[6] = v;
    }
    pub fn fragment_flags2(&self) -> u8 {
        self.buffer[7]
    }
    pub fn set_fragment_flags2(&mut self, v: u8) {
        self.buffer[7] = v;
    }
    pub fn time_to_live(&self) -> u8 {
        self.buffer[8]
    }
    pub fn set_time_to_live(&mut self, v: u8) {
        self.buffer[8] = v;
    }
    pub fn protocol(&self) -> u8 {
        self.buffer[9]
    }
    pub fn set_protocol(&mut self, v: u8) {
        self.buffer[9] = v;
    }
    pub fn checksum(&self) -> u16 {
        u16::from_be_bytes(self.buffer[10..12].try_into().unwrap())
    }
    pub fn set_checksum(&mut self, v: u16) {
        self.buffer[10..12].copy_from_slice(&v.to_be_bytes());
    }
    pub fn src(&self) -> IpAddress {
        self.buffer[12..16].try_into().unwrap()
    }
    pub fn set_src(&mut self, v: IpAddress) {
        self.buffer[12..16].copy_from_slice(&v);
    }
    pub fn dst(&self) -> IpAddress {
        self.buffer[16..20].try_into().unwrap()
    }
    pub fn set_dst(&mut self, v: IpAddress) {
        self.buffer[16..20].copy_from_slice(&v);
    }
    /// Header length in bytes (derived from the lower nibble of byte 0).
    pub fn header_length(&self) -> usize {
        ((self.buffer[0] & 0x0F) as usize) << 2
    }
    pub fn options(&self) -> &[u8] {
        let h = self.header_length();
        &self.buffer[IP_HEADER_MIN_LEN..h]
    }
    pub fn options_mut(&mut self) -> &mut [u8] {
        let h = self.header_length();
        &mut self.buffer[IP_HEADER_MIN_LEN..h]
    }
    pub fn payload(&self) -> &[u8] {
        &self.buffer[self.header_length()..]
    }
    pub fn payload_mut(&mut self) -> &mut [u8] {
        let h = self.header_length();
        &mut self.buffer[h..]
    }
}

// =============================================================================
// ICMPv4
// =============================================================================

/// Fixed ICMPv4 header length in bytes (type + code + checksum + 4 bytes).
pub const ICMP_HEADER_LEN: usize = 8;

/// A parsed ICMPv4 packet.
pub struct IcmpPacket {
    pub type_: u8,
    pub code: u8,
    pub checksum: u16,
    pub header_data: [u8; 4],
    pub payload: Vec<u8>,
}

impl IcmpPacket {
    /// Parse an ICMPv4 packet from the given buffer.
    pub fn from_bytes(buf: &[u8]) -> Self {
        let mut off = 0;
        let type_ = read_u8(buf, &mut off);
        let code = read_u8(buf, &mut off);
        let checksum = read_u16(buf, &mut off);
        let header_data: [u8; 4] = buf[off..off + 4].try_into().expect("IcmpPacket: short header");
        off += 4;
        let payload = buf[off..].to_vec();
        Self { type_, code, checksum, header_data, payload }
    }

    /// Serialize into a freshly allocated byte vector.
    pub fn to_bytes(&self) -> Vec<u8> {
        let total = ICMP_HEADER_LEN + self.payload.len();
        let mut buf = vec![0u8; total];
        let mut off = 0;
        self.write_bytes(&mut buf, &mut off);
        buf
    }

    /// Total on-wire length in bytes.
    pub fn length(&self) -> usize {
        ICMP_HEADER_LEN + self.payload.len()
    }

    /// Serialize the packet into the provided buffer at `off`.
    pub fn write_bytes(&self, buf: &mut [u8], off: &mut usize) {
        write_u8(buf, off, self.type_);
        write_u8(buf, off, self.code);
        write_u16(buf, off, self.checksum);
        write_bytes(buf, off, &self.header_data);
        write_bytes(buf, off, &self.payload);
    }

    /// Compute the ICMPv4 checksum (over a synthesized segment, not a pseudo
    /// header).
    pub fn calculate_checksum(&mut self) {
        let mut p_len = self.length();
        if p_len & 1 != 0 {
            p_len += 1;
        }
        let mut segment = vec![0u8; p_len];
        let mut counter = 0;
        self.checksum = 0;
        self.write_bytes(&mut segment, &mut counter);
        if counter != p_len {
            write_u8(&mut segment, &mut counter, 0);
        }
        self.checksum = internet_checksum(&segment, p_len);
    }

    /// Returns `true` if the on-wire checksum is correct.
    pub fn verify_checksum(&self) -> bool {
        let mut p_len = self.length();
        if p_len & 1 != 0 {
            p_len += 1;
        }
        let mut segment = vec![0u8; p_len];
        let mut counter = 0;
        self.write_bytes(&mut segment, &mut counter);
        if counter != p_len {
            write_u8(&mut segment, &mut counter, 0);
        }
        internet_checksum(&segment, p_len) == 0
    }

    /// Construct an editor view over a mutable backing buffer.
    pub fn editor<'a>(buffer: &'a mut [u8]) -> IcmpPacketEditor<'a> {
        IcmpPacketEditor::new(buffer)
    }
}

/// Mutable in-place editor for an ICMPv4 packet held in a borrowed buffer.
pub struct IcmpPacketEditor<'a> {
    buffer: &'a mut [u8],
}

impl<'a> IcmpPacketEditor<'a> {
    pub fn new(buffer: &'a mut [u8]) -> Self {
        assert!(buffer.len() >= ICMP_HEADER_LEN, "IcmpPacketEditor: short buffer");
        Self { buffer }
    }

    pub fn type_(&self) -> u8 {
        self.buffer[0]
    }
    pub fn set_type(&mut self, v: u8) {
        self.buffer[0] = v;
    }
    pub fn code(&self) -> u8 {
        self.buffer[1]
    }
    pub fn set_code(&mut self, v: u8) {
        self.buffer[1] = v;
    }
    pub fn checksum(&self) -> u16 {
        u16::from_be_bytes(self.buffer[2..4].try_into().unwrap())
    }
    pub fn set_checksum(&mut self, v: u16) {
        self.buffer[2..4].copy_from_slice(&v.to_be_bytes());
    }
    pub fn header_data(&self) -> [u8; 4] {
        self.buffer[4..8].try_into().unwrap()
    }
    pub fn set_header_data(&mut self, v: [u8; 4]) {
        self.buffer[4..8].copy_from_slice(&v);
    }
    pub fn payload(&self) -> &[u8] {
        &self.buffer[ICMP_HEADER_LEN..]
    }
    pub fn payload_mut(&mut self) -> &mut [u8] {
        &mut self.buffer[ICMP_HEADER_LEN..]
    }
}

/// Convenience: encode an ICMPv4 echo/echo-reply identifier + sequence pair
/// into the 4-byte header_data slot.
pub fn icmp_encode_header_data(identifier: u16, sequence_number: u16) -> [u8; 4] {
    let mut out = [0u8; 4];
    out[0..2].copy_from_slice(&identifier.to_be_bytes());
    out[2..4].copy_from_slice(&sequence_number.to_be_bytes());
    out
}

/// Convenience: decode an ICMPv4 echo/echo-reply identifier + sequence pair
/// from the 4-byte header_data slot.
pub fn icmp_decode_header_data(header_data: [u8; 4]) -> (u16, u16) {
    let identifier = u16::from_be_bytes(header_data[0..2].try_into().unwrap());
    let sequence_number = u16::from_be_bytes(header_data[2..4].try_into().unwrap());
    (identifier, sequence_number)
}

// =============================================================================
// TCP
// =============================================================================

/// Fixed TCP header length in bytes (without options).
pub const TCP_HEADER_MIN_LEN: usize = 20;

/// A single TCP option (translated from the C++ `TCPOption` hierarchy plus the
/// `IPopUnk` reuse for unknown kinds).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TcpOption {
    /// 1-byte NOP (kind = 1).
    Nop,
    /// Maximum Segment Size (kind = 2, length = 4).
    Mss { max_segment_size: u16 },
    /// Window Scale (kind = 3, length = 3).
    WindowScale { window_scale: u8 },
    /// Timestamps (kind = 8, length = 10).
    Timestamp { sender_timestamp: u32, echo_timestamp: u32 },
    /// Unrecognised option (stored verbatim).
    Unknown { code: u8, value: Vec<u8> },
}

impl TcpOption {
    pub fn code(&self) -> u8 {
        match self {
            TcpOption::Nop => 1,
            TcpOption::Mss { .. } => 2,
            TcpOption::WindowScale { .. } => 3,
            TcpOption::Timestamp { .. } => 8,
            TcpOption::Unknown { code, .. } => *code,
        }
    }

    /// On-wire length in bytes.
    pub fn length(&self) -> usize {
        match self {
            TcpOption::Nop => 1,
            TcpOption::Mss { .. } => 4,
            TcpOption::WindowScale { .. } => 3,
            TcpOption::Timestamp { .. } => 10,
            TcpOption::Unknown { value, .. } => 2 + value.len(),
        }
    }

    fn write_bytes(&self, buf: &mut [u8], off: &mut usize) {
        match self {
            TcpOption::Nop => write_u8(buf, off, 1),
            TcpOption::Mss { max_segment_size } => {
                write_u8(buf, off, 2);
                write_u8(buf, off, 4);
                write_u16(buf, off, *max_segment_size);
            }
            TcpOption::WindowScale { window_scale } => {
                write_u8(buf, off, 3);
                write_u8(buf, off, 3);
                write_u8(buf, off, *window_scale);
            }
            TcpOption::Timestamp { sender_timestamp, echo_timestamp } => {
                write_u8(buf, off, 8);
                write_u8(buf, off, 10);
                write_u32(buf, off, *sender_timestamp);
                write_u32(buf, off, *echo_timestamp);
            }
            TcpOption::Unknown { code, value } => {
                write_u8(buf, off, *code);
                write_u8(buf, off, (2 + value.len()) as u8);
                write_bytes(buf, off, value);
            }
        }
    }
}

/// A parsed TCP segment.
pub struct TcpPacket {
    pub source_port: u16,
    pub destination_port: u16,
    pub sequence_number: u32,
    pub acknowledgement_number: u32,
    pub data_offset_and_ns_flag: u8,
    pub flags: u8,
    pub window_size: u16,
    pub checksum: u16,
    pub urgent_pointer: u16,
    pub options: Vec<TcpOption>,
    pub payload: Vec<u8>,
}

impl TcpPacket {
    /// Parse a TCP segment from the given buffer.
    pub fn from_bytes(buf: &[u8]) -> Self {
        let mut off = 0;
        let source_port = read_u16(buf, &mut off);
        let destination_port = read_u16(buf, &mut off);
        let sequence_number = read_u32(buf, &mut off);
        let acknowledgement_number = read_u32(buf, &mut off);
        let data_offset_and_ns_flag = read_u8(buf, &mut off);
        let header_length = ((data_offset_and_ns_flag >> 4) as usize) << 2;
        let flags = read_u8(buf, &mut off);
        let window_size = read_u16(buf, &mut off);
        let checksum = read_u16(buf, &mut off);
        let urgent_pointer = read_u16(buf, &mut off);

        let mut options = Vec::new();
        if header_length > TCP_HEADER_MIN_LEN {
            let mut ooff = TCP_HEADER_MIN_LEN;
            while ooff < header_length {
                let op_kind = buf[ooff];
                if op_kind == 0 {
                    break;
                }
                if op_kind == 1 {
                    options.push(TcpOption::Nop);
                    ooff += 1;
                } else {
                    let op_len = buf[ooff + 1] as usize;
                    let opt = match op_kind {
                        2 => TcpOption::Mss {
                            max_segment_size: u16::from_be_bytes(
                                buf[ooff + 2..ooff + 4].try_into().unwrap(),
                            ),
                        },
                        3 => TcpOption::WindowScale {
                            window_scale: buf[ooff + 2],
                        },
                        8 => TcpOption::Timestamp {
                            sender_timestamp: u32::from_be_bytes(
                                buf[ooff + 2..ooff + 6].try_into().unwrap(),
                            ),
                            echo_timestamp: u32::from_be_bytes(
                                buf[ooff + 6..ooff + 10].try_into().unwrap(),
                            ),
                        },
                        _ => {
                            let value_start = ooff + 2;
                            let value_end = (ooff + op_len).min(buf.len());
                            let value = if value_end > value_start {
                                buf[value_start..value_end].to_vec()
                            } else {
                                Vec::new()
                            };
                            TcpOption::Unknown { code: op_kind, value }
                        }
                    };
                    options.push(opt);
                    ooff += op_len;
                }
            }
        }

        let payload = if header_length <= buf.len() {
            buf[header_length..].to_vec()
        } else {
            Vec::new()
        };

        Self {
            source_port,
            destination_port,
            sequence_number,
            acknowledgement_number,
            data_offset_and_ns_flag,
            flags,
            window_size,
            checksum,
            urgent_pointer,
            options,
            payload,
        }
    }

    /// Compute the on-wire header length (in bytes) for the current options.
    pub fn header_length(&mut self) -> usize {
        let mut ooff = TCP_HEADER_MIN_LEN;
        for o in &self.options {
            ooff += o.length();
        }
        let aligned = align_up(ooff, 4);
        let ns = self.data_offset_and_ns_flag & 1;
        self.data_offset_and_ns_flag = ((aligned >> 2) as u8) << 4;
        self.data_offset_and_ns_flag |= ns;
        aligned
    }

    /// Total on-wire length in bytes.
    pub fn length(&mut self) -> usize {
        self.header_length() + self.payload.len()
    }

    /// Serialize the segment into a freshly allocated byte vector.
    pub fn to_bytes(&mut self) -> Vec<u8> {
        let total = self.length();
        let mut buf = vec![0u8; total];
        let mut off = 0;
        self.write_bytes(&mut buf, &mut off);
        debug_assert_eq!(off, total);
        buf
    }

    /// Serialize the segment into the provided buffer at `off`.
    pub fn write_bytes(&mut self, buf: &mut [u8], off: &mut usize) {
        let header_length = self.header_length();
        let start = *off;

        write_u16(buf, off, self.source_port);
        write_u16(buf, off, self.destination_port);
        write_u32(buf, off, self.sequence_number);
        write_u32(buf, off, self.acknowledgement_number);
        write_u8(buf, off, self.data_offset_and_ns_flag);
        write_u8(buf, off, self.flags);
        write_u16(buf, off, self.window_size);
        write_u16(buf, off, self.checksum);
        write_u16(buf, off, self.urgent_pointer);

        let opts_start = *off;
        for o in &self.options {
            o.write_bytes(buf, off);
        }
        if *off != opts_start + (header_length - TCP_HEADER_MIN_LEN) {
            let pad = (start + header_length) - *off;
            for b in &mut buf[*off..*off + pad] {
                *b = 0;
            }
            *off += pad;
        }
        *off = start + header_length;

        write_bytes(buf, off, &self.payload);
    }

    // -- flag accessors --
    pub fn ns(&self) -> bool {
        (self.data_offset_and_ns_flag & 1) != 0
    }
    pub fn set_ns(&mut self, v: bool) {
        self.data_offset_and_ns_flag = (self.data_offset_and_ns_flag & !1) | (v as u8 & 1);
    }
    pub fn cwr(&self) -> bool {
        (self.flags & (1 << 7)) != 0
    }
    pub fn set_cwr(&mut self, v: bool) {
        self.flags = (self.flags & !(1 << 7)) | (((v as u8) & 1) << 7);
    }
    pub fn ece(&self) -> bool {
        (self.flags & (1 << 6)) != 0
    }
    pub fn set_ece(&mut self, v: bool) {
        self.flags = (self.flags & !(1 << 6)) | (((v as u8) & 1) << 6);
    }
    pub fn urg(&self) -> bool {
        (self.flags & (1 << 5)) != 0
    }
    pub fn set_urg(&mut self, v: bool) {
        self.flags = (self.flags & !(1 << 5)) | (((v as u8) & 1) << 5);
    }
    pub fn ack(&self) -> bool {
        (self.flags & (1 << 4)) != 0
    }
    pub fn set_ack(&mut self, v: bool) {
        self.flags = (self.flags & !(1 << 4)) | (((v as u8) & 1) << 4);
    }
    pub fn psh(&self) -> bool {
        (self.flags & (1 << 3)) != 0
    }
    pub fn set_psh(&mut self, v: bool) {
        self.flags = (self.flags & !(1 << 3)) | (((v as u8) & 1) << 3);
    }
    pub fn rst(&self) -> bool {
        (self.flags & (1 << 2)) != 0
    }
    pub fn set_rst(&mut self, v: bool) {
        self.flags = (self.flags & !(1 << 2)) | (((v as u8) & 1) << 2);
    }
    pub fn syn(&self) -> bool {
        (self.flags & (1 << 1)) != 0
    }
    pub fn set_syn(&mut self, v: bool) {
        self.flags = (self.flags & !(1 << 1)) | (((v as u8) & 1) << 1);
    }
    pub fn fin(&self) -> bool {
        (self.flags & 1) != 0
    }
    pub fn set_fin(&mut self, v: bool) {
        self.flags = (self.flags & !1) | ((v as u8) & 1);
    }

    /// Calculate the TCP checksum over the standard IPv4 pseudo-header.
    pub fn calculate_checksum(&mut self, src_ip: IpAddress, dst_ip: IpAddress) {
        let header_length = self.header_length();
        let mut p_len = 12 + header_length + self.payload.len();
        if p_len & 1 != 0 {
            p_len += 1;
        }
        let mut segment = vec![0u8; p_len];
        let mut counter = 0;
        write_bytes(&mut segment, &mut counter, &src_ip);
        write_bytes(&mut segment, &mut counter, &dst_ip);
        write_u8(&mut segment, &mut counter, 0);
        write_u8(&mut segment, &mut counter, 6);
        write_u16(&mut segment, &mut counter, self.length() as u16);
        self.checksum = 0;
        self.write_bytes(&mut segment, &mut counter);
        if counter != p_len {
            write_u8(&mut segment, &mut counter, 0);
        }
        self.checksum = internet_checksum(&segment, p_len);
    }

    /// Returns `true` if the on-wire TCP checksum is correct.
    pub fn verify_checksum(&mut self, src_ip: IpAddress, dst_ip: IpAddress) -> bool {
        let header_length = self.header_length();
        let mut p_len = 12 + header_length + self.payload.len();
        if p_len & 1 != 0 {
            p_len += 1;
        }
        let mut segment = vec![0u8; p_len];
        let mut counter = 0;
        write_bytes(&mut segment, &mut counter, &src_ip);
        write_bytes(&mut segment, &mut counter, &dst_ip);
        write_u8(&mut segment, &mut counter, 0);
        write_u8(&mut segment, &mut counter, 6);
        write_u16(&mut segment, &mut counter, self.length() as u16);
        self.write_bytes(&mut segment, &mut counter);
        if counter != p_len {
            write_u8(&mut segment, &mut counter, 0);
        }
        internet_checksum(&segment, p_len) == 0
    }

    /// Construct an editor view over a mutable backing buffer.
    pub fn editor<'a>(buffer: &'a mut [u8]) -> TcpPacketEditor<'a> {
        TcpPacketEditor::new(buffer)
    }
}

/// Mutable in-place editor for a TCP segment held in a borrowed buffer.
pub struct TcpPacketEditor<'a> {
    buffer: &'a mut [u8],
}

impl<'a> TcpPacketEditor<'a> {
    pub fn new(buffer: &'a mut [u8]) -> Self {
        assert!(buffer.len() >= TCP_HEADER_MIN_LEN, "TcpPacketEditor: short buffer");
        Self { buffer }
    }

    pub fn source_port(&self) -> u16 {
        u16::from_be_bytes(self.buffer[0..2].try_into().unwrap())
    }
    pub fn set_source_port(&mut self, v: u16) {
        self.buffer[0..2].copy_from_slice(&v.to_be_bytes());
    }
    pub fn destination_port(&self) -> u16 {
        u16::from_be_bytes(self.buffer[2..4].try_into().unwrap())
    }
    pub fn set_destination_port(&mut self, v: u16) {
        self.buffer[2..4].copy_from_slice(&v.to_be_bytes());
    }
    pub fn sequence_number(&self) -> u32 {
        u32::from_be_bytes(self.buffer[4..8].try_into().unwrap())
    }
    pub fn set_sequence_number(&mut self, v: u32) {
        self.buffer[4..8].copy_from_slice(&v.to_be_bytes());
    }
    pub fn acknowledgement_number(&self) -> u32 {
        u32::from_be_bytes(self.buffer[8..12].try_into().unwrap())
    }
    pub fn set_acknowledgement_number(&mut self, v: u32) {
        self.buffer[8..12].copy_from_slice(&v.to_be_bytes());
    }
    pub fn data_offset_and_ns_flag(&self) -> u8 {
        self.buffer[12]
    }
    pub fn set_data_offset_and_ns_flag(&mut self, v: u8) {
        self.buffer[12] = v;
    }
    pub fn flags(&self) -> u8 {
        self.buffer[13]
    }
    pub fn set_flags(&mut self, v: u8) {
        self.buffer[13] = v;
    }
    pub fn window_size(&self) -> u16 {
        u16::from_be_bytes(self.buffer[14..16].try_into().unwrap())
    }
    pub fn set_window_size(&mut self, v: u16) {
        self.buffer[14..16].copy_from_slice(&v.to_be_bytes());
    }
    pub fn checksum(&self) -> u16 {
        u16::from_be_bytes(self.buffer[16..18].try_into().unwrap())
    }
    pub fn set_checksum(&mut self, v: u16) {
        self.buffer[16..18].copy_from_slice(&v.to_be_bytes());
    }
    pub fn urgent_pointer(&self) -> u16 {
        u16::from_be_bytes(self.buffer[18..20].try_into().unwrap())
    }
    pub fn set_urgent_pointer(&mut self, v: u16) {
        self.buffer[18..20].copy_from_slice(&v.to_be_bytes());
    }
    pub fn header_length(&self) -> usize {
        ((self.buffer[12] >> 4) as usize) << 2
    }
    pub fn options(&self) -> &[u8] {
        let h = self.header_length();
        &self.buffer[TCP_HEADER_MIN_LEN..h]
    }
    pub fn options_mut(&mut self) -> &mut [u8] {
        let h = self.header_length();
        &mut self.buffer[TCP_HEADER_MIN_LEN..h]
    }
    pub fn payload(&self) -> &[u8] {
        &self.buffer[self.header_length()..]
    }
    pub fn payload_mut(&mut self) -> &mut [u8] {
        let h = self.header_length();
        &mut self.buffer[h..]
    }
}

// =============================================================================
// UDP
// =============================================================================

/// Fixed UDP header length in bytes.
pub const UDP_HEADER_LEN: usize = 8;

/// A parsed UDP datagram.
pub struct UdpPacket {
    pub source_port: u16,
    pub destination_port: u16,
    pub length: u16,
    pub checksum: u16,
    pub payload: Vec<u8>,
}

impl UdpPacket {
    /// Parse a UDP datagram from the given buffer.
    pub fn from_bytes(buf: &[u8]) -> Self {
        let mut off = 0;
        let source_port = read_u16(buf, &mut off);
        let destination_port = read_u16(buf, &mut off);
        let mut length = read_u16(buf, &mut off) as usize;
        let checksum = read_u16(buf, &mut off);
        if length > buf.len() {
            length = buf.len();
        }
        let payload = buf[off..length].to_vec();
        Self { source_port, destination_port, length: length as u16, checksum, payload }
    }

    /// Total on-wire length in bytes.
    pub fn length(&self) -> usize {
        UDP_HEADER_LEN + self.payload.len()
    }

    /// Serialize the datagram into a freshly allocated byte vector.
    pub fn to_bytes(&self) -> Vec<u8> {
        let total = self.length();
        let mut buf = vec![0u8; total];
        let mut off = 0;
        self.write_bytes(&mut buf, &mut off);
        buf
    }

    /// Serialize the datagram into the provided buffer at `off`.
    pub fn write_bytes(&self, buf: &mut [u8], off: &mut usize) {
        write_u16(buf, off, self.source_port);
        write_u16(buf, off, self.destination_port);
        write_u16(buf, off, self.length);
        write_u16(buf, off, self.checksum);
        write_bytes(buf, off, &self.payload);
    }

    /// Calculate the UDP checksum over the standard IPv4 pseudo-header.
    pub fn calculate_checksum(&mut self, src_ip: IpAddress, dst_ip: IpAddress) {
        let mut p_len = 12 + UDP_HEADER_LEN + self.payload.len();
        if p_len & 1 != 0 {
            p_len += 1;
        }
        let mut segment = vec![0u8; p_len];
        let mut counter = 0;
        write_bytes(&mut segment, &mut counter, &src_ip);
        write_bytes(&mut segment, &mut counter, &dst_ip);
        write_u8(&mut segment, &mut counter, 0);
        write_u8(&mut segment, &mut counter, 17);
        write_u16(&mut segment, &mut counter, self.length() as u16);
        self.checksum = 0;
        self.write_bytes(&mut segment, &mut counter);
        if counter != p_len {
            write_u8(&mut segment, &mut counter, 0);
        }
        self.checksum = internet_checksum(&segment, p_len);
    }

    /// Returns `true` if the on-wire UDP checksum is correct.
    pub fn verify_checksum(&self, src_ip: IpAddress, dst_ip: IpAddress) -> bool {
        let mut p_len = 12 + UDP_HEADER_LEN + self.payload.len();
        if p_len & 1 != 0 {
            p_len += 1;
        }
        let mut segment = vec![0u8; p_len];
        let mut counter = 0;
        write_bytes(&mut segment, &mut counter, &src_ip);
        write_bytes(&mut segment, &mut counter, &dst_ip);
        write_u8(&mut segment, &mut counter, 0);
        write_u8(&mut segment, &mut counter, 17);
        write_u16(&mut segment, &mut counter, self.length() as u16);
        self.write_bytes(&mut segment, &mut counter);
        if counter != p_len {
            write_u8(&mut segment, &mut counter, 0);
        }
        internet_checksum(&segment, p_len) == 0
    }

    /// Construct an editor view over a mutable backing buffer.
    pub fn editor<'a>(buffer: &'a mut [u8]) -> UdpPacketEditor<'a> {
        UdpPacketEditor::new(buffer)
    }
}

/// Mutable in-place editor for a UDP datagram held in a borrowed buffer.
pub struct UdpPacketEditor<'a> {
    buffer: &'a mut [u8],
}

impl<'a> UdpPacketEditor<'a> {
    pub fn new(buffer: &'a mut [u8]) -> Self {
        assert!(buffer.len() >= UDP_HEADER_LEN, "UdpPacketEditor: short buffer");
        Self { buffer }
    }

    pub fn src_port(&self) -> u16 {
        u16::from_be_bytes(self.buffer[0..2].try_into().unwrap())
    }
    pub fn set_src_port(&mut self, v: u16) {
        self.buffer[0..2].copy_from_slice(&v.to_be_bytes());
    }
    pub fn dst_port(&self) -> u16 {
        u16::from_be_bytes(self.buffer[2..4].try_into().unwrap())
    }
    pub fn set_dst_port(&mut self, v: u16) {
        self.buffer[2..4].copy_from_slice(&v.to_be_bytes());
    }
    pub fn length(&self) -> u16 {
        u16::from_be_bytes(self.buffer[4..6].try_into().unwrap())
    }
    pub fn set_length(&mut self, v: u16) {
        self.buffer[4..6].copy_from_slice(&v.to_be_bytes());
    }
    pub fn checksum(&self) -> u16 {
        u16::from_be_bytes(self.buffer[6..8].try_into().unwrap())
    }
    pub fn set_checksum(&mut self, v: u16) {
        self.buffer[6..8].copy_from_slice(&v.to_be_bytes());
    }
    pub fn payload(&self) -> &[u8] {
        &self.buffer[UDP_HEADER_LEN..]
    }
    pub fn payload_mut(&mut self) -> &mut [u8] {
        &mut self.buffer[UDP_HEADER_LEN..]
    }
}

// =============================================================================
// BOOTP / DHCP
// =============================================================================

/// Fixed BOOTP/DHCP base length in bytes (op..magic cookie).
pub const DHCP_FIXED_LEN: usize = 240;
/// Standard DHCP packet size.
pub const DHCP_DEFAULT_MAX_LENGTH: usize = 548;

/// A single DHCP option (translated from the C++ `DHCPOption` hierarchy).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DhcpOption {
    /// 1-byte NOP (code = 0).
    Nop,
    /// 1-byte end-of-options marker (code = 255).
    End,
    /// 1 = Subnet Mask.
    Subnet { subnet_mask: IpAddress },
    /// 3 = Router.
    Router { routers: Vec<IpAddress> },
    /// 6 = DNS.
    Dns { dns_servers: Vec<IpAddress> },
    /// 12 = Hostname.
    HostName { host_name: String },
    /// 15 = DNS Domain Name.
    DnsName { domain_name: String },
    /// 28 = Broadcast Address.
    Broadcast { broadcast_ip: IpAddress },
    /// 46 = NetBIOS Node Type.
    NbiosType {
        h_node: bool,
        m_node: bool,
        p_node: bool,
        b_node: bool,
    },
    /// 50 = Requested IP Address.
    RequestedIp { requested_ip: IpAddress },
    /// 51 = IP Address Lease Time.
    IpLeaseTime { lease_time: u32 },
    /// 53 = DHCP Message Type.
    Message { message: u8 },
    /// 54 = Server Identifier.
    ServerIp { server_ip: IpAddress },
    /// 55 = Parameter Request List.
    ParameterRequestList { requests: Vec<u8> },
    /// 56 = DHCP Message (string).
    MessageString { message: String },
    /// 57 = Maximum DHCP Message Size.
    MaxMessageSize { max_message_size: u16 },
    /// 58 = Renewal (T1) Time Value.
    RenewalTimeT1 { t1: u32 },
    /// 59 = Rebinding (T2) Time Value.
    RebindingTimeT2 { t2: u32 },
    /// 60 = Class Identifier.
    ClassId { class_id: String },
    /// 61 = Client Identifier.
    ClientId { client_id: Vec<u8> },
}

impl DhcpOption {
    pub fn code(&self) -> u8 {
        match self {
            DhcpOption::Nop => 0,
            DhcpOption::End => 255,
            DhcpOption::Subnet { .. } => 1,
            DhcpOption::Router { .. } => 3,
            DhcpOption::Dns { .. } => 6,
            DhcpOption::HostName { .. } => 12,
            DhcpOption::DnsName { .. } => 15,
            DhcpOption::Broadcast { .. } => 28,
            DhcpOption::NbiosType { .. } => 46,
            DhcpOption::RequestedIp { .. } => 50,
            DhcpOption::IpLeaseTime { .. } => 51,
            DhcpOption::Message { .. } => 53,
            DhcpOption::ServerIp { .. } => 54,
            DhcpOption::ParameterRequestList { .. } => 55,
            DhcpOption::MessageString { .. } => 56,
            DhcpOption::MaxMessageSize { .. } => 57,
            DhcpOption::RenewalTimeT1 { .. } => 58,
            DhcpOption::RebindingTimeT2 { .. } => 59,
            DhcpOption::ClassId { .. } => 60,
            DhcpOption::ClientId { .. } => 61,
        }
    }

    /// On-wire length in bytes (including the code byte).
    pub fn length(&self) -> usize {
        match self {
            DhcpOption::Nop | DhcpOption::End => 1,
            DhcpOption::Subnet { .. } => 6,
            DhcpOption::Router { routers } => 2 + 4 * routers.len(),
            DhcpOption::Dns { dns_servers } => 2 + 4 * dns_servers.len(),
            DhcpOption::HostName { host_name } => 2 + host_name.len(),
            DhcpOption::DnsName { domain_name } => 2 + domain_name.len(),
            DhcpOption::Broadcast { .. } => 6,
            DhcpOption::NbiosType { .. } => 3,
            DhcpOption::RequestedIp { .. } => 6,
            DhcpOption::IpLeaseTime { .. } => 6,
            DhcpOption::Message { .. } => 3,
            DhcpOption::ServerIp { .. } => 6,
            DhcpOption::ParameterRequestList { requests } => 2 + requests.len(),
            DhcpOption::MessageString { message } => 2 + message.len(),
            DhcpOption::MaxMessageSize { .. } => 4,
            DhcpOption::RenewalTimeT1 { .. } => 6,
            DhcpOption::RebindingTimeT2 { .. } => 6,
            DhcpOption::ClassId { class_id } => 2 + class_id.len(),
            DhcpOption::ClientId { client_id } => 2 + client_id.len(),
        }
    }

    fn write_bytes(&self, buf: &mut [u8], off: &mut usize) {
        match self {
            DhcpOption::Nop => write_u8(buf, off, 0),
            DhcpOption::End => write_u8(buf, off, 255),
            DhcpOption::Subnet { subnet_mask } => {
                write_u8(buf, off, 1);
                write_u8(buf, off, 4);
                write_bytes(buf, off, subnet_mask);
            }
            DhcpOption::Router { routers } => {
                write_u8(buf, off, 3);
                write_u8(buf, off, (4 * routers.len()) as u8);
                for r in routers {
                    write_bytes(buf, off, r);
                }
            }
            DhcpOption::Dns { dns_servers } => {
                write_u8(buf, off, 6);
                write_u8(buf, off, (4 * dns_servers.len()) as u8);
                for d in dns_servers {
                    write_bytes(buf, off, d);
                }
            }
            DhcpOption::HostName { host_name } => {
                write_u8(buf, off, 12);
                write_u8(buf, off, host_name.len() as u8);
                write_bytes(buf, off, host_name.as_bytes());
            }
            DhcpOption::DnsName { domain_name } => {
                write_u8(buf, off, 15);
                write_u8(buf, off, domain_name.len() as u8);
                write_bytes(buf, off, domain_name.as_bytes());
            }
            DhcpOption::Broadcast { broadcast_ip } => {
                write_u8(buf, off, 28);
                write_u8(buf, off, 4);
                write_bytes(buf, off, broadcast_ip);
            }
            DhcpOption::NbiosType { h_node, m_node, p_node, b_node } => {
                let mut t: u8 = 0;
                if *h_node { t |= 1 << 3; }
                if *m_node { t |= 1 << 2; }
                if *p_node { t |= 1 << 1; }
                if *b_node { t |= 1; }
                write_u8(buf, off, 46);
                write_u8(buf, off, 1);
                write_u8(buf, off, t);
            }
            DhcpOption::RequestedIp { requested_ip } => {
                write_u8(buf, off, 50);
                write_u8(buf, off, 4);
                write_bytes(buf, off, requested_ip);
            }
            DhcpOption::IpLeaseTime { lease_time } => {
                write_u8(buf, off, 51);
                write_u8(buf, off, 4);
                write_u32(buf, off, *lease_time);
            }
            DhcpOption::Message { message } => {
                write_u8(buf, off, 53);
                write_u8(buf, off, 1);
                write_u8(buf, off, *message);
            }
            DhcpOption::ServerIp { server_ip } => {
                write_u8(buf, off, 54);
                write_u8(buf, off, 4);
                write_bytes(buf, off, server_ip);
            }
            DhcpOption::ParameterRequestList { requests } => {
                write_u8(buf, off, 55);
                write_u8(buf, off, requests.len() as u8);
                write_bytes(buf, off, requests);
            }
            DhcpOption::MessageString { message } => {
                write_u8(buf, off, 56);
                write_u8(buf, off, message.len() as u8);
                write_bytes(buf, off, message.as_bytes());
            }
            DhcpOption::MaxMessageSize { max_message_size } => {
                write_u8(buf, off, 57);
                write_u8(buf, off, 2);
                write_u16(buf, off, *max_message_size);
            }
            DhcpOption::RenewalTimeT1 { t1 } => {
                write_u8(buf, off, 58);
                write_u8(buf, off, 4);
                write_u32(buf, off, *t1);
            }
            DhcpOption::RebindingTimeT2 { t2 } => {
                write_u8(buf, off, 59);
                write_u8(buf, off, 4);
                write_u32(buf, off, *t2);
            }
            DhcpOption::ClassId { class_id } => {
                write_u8(buf, off, 60);
                write_u8(buf, off, class_id.len() as u8);
                write_bytes(buf, off, class_id.as_bytes());
            }
            DhcpOption::ClientId { client_id } => {
                write_u8(buf, off, 61);
                write_u8(buf, off, client_id.len() as u8);
                write_bytes(buf, off, client_id);
            }
        }
    }
}

/// A parsed BOOTP/DHCP packet.
pub struct DhcpPacket {
    pub op: u8,
    pub hardware_type: u8,
    pub hardware_address_length: u8,
    pub hops: u8,
    pub transaction_id: u32,
    pub seconds: u16,
    pub flags: u16,
    pub client_ip: IpAddress,
    pub your_ip: IpAddress,
    pub server_ip: IpAddress,
    pub gateway_ip: IpAddress,
    pub client_hardware_address: [u8; 16],
    pub magic_cookie: u32,
    /// Total on-wire length of the packet (default 548).  Used to size the
    /// serialized options area and to pad it out with zeros.
    pub max_length: usize,
    pub options: Vec<DhcpOption>,
}

impl Default for DhcpPacket {
    fn default() -> Self {
        Self {
            op: 0,
            hardware_type: 0,
            hardware_address_length: 0,
            hops: 0,
            transaction_id: 0,
            seconds: 0,
            flags: 0,
            client_ip: [0; 4],
            your_ip: [0; 4],
            server_ip: [0; 4],
            gateway_ip: [0; 4],
            client_hardware_address: [0; 16],
            magic_cookie: 0,
            max_length: DHCP_DEFAULT_MAX_LENGTH,
            options: Vec::new(),
        }
    }
}

impl DhcpPacket {
    /// Parse a BOOTP/DHCP packet from the given buffer.  `max_length` is
    /// seeded from the buffer length (matching the C++ behaviour where the
    /// caller provides the bound).
    pub fn from_bytes(buf: &[u8]) -> Self {
        let mut off = 0;
        let op = read_u8(buf, &mut off);
        let hardware_type = read_u8(buf, &mut off);
        let hardware_address_length = read_u8(buf, &mut off);
        let hops = read_u8(buf, &mut off);
        let transaction_id = read_u32(buf, &mut off);
        let seconds = read_u16(buf, &mut off);
        let flags = read_u16(buf, &mut off);
        let client_ip: IpAddress = buf[off..off + 4].try_into().unwrap();
        off += 4;
        let your_ip: IpAddress = buf[off..off + 4].try_into().unwrap();
        off += 4;
        let server_ip: IpAddress = buf[off..off + 4].try_into().unwrap();
        off += 4;
        let gateway_ip: IpAddress = buf[off..off + 4].try_into().unwrap();
        off += 4;
        let mut client_hardware_address = [0u8; 16];
        client_hardware_address.copy_from_slice(&buf[off..off + 16]);
        off += 16;
        // Skip 192 bytes of unused BOOTP legacy fields (sname + file).
        off += 192;
        let magic_cookie = read_u32(buf, &mut off);

        let mut options = Vec::new();
        while off < buf.len() {
            let op_kind = buf[off];
            if op_kind == 255 {
                options.push(DhcpOption::End);
                off += 1;
                break;
            }
            if off + 1 >= buf.len() {
                options.push(DhcpOption::End);
                break;
            }
            let op_len = buf[off + 1] as usize;
            let opt = match op_kind {
                0 => {
                    options.push(DhcpOption::Nop);
                    off += 1;
                    continue;
                }
                1 => DhcpOption::Subnet {
                    subnet_mask: buf[off + 2..off + 6].try_into().unwrap(),
                },
                3 => {
                    let n = op_len / 4;
                    let mut routers = Vec::with_capacity(n);
                    for i in 0..n {
                        let start = off + 2 + 4 * i;
                        routers.push(buf[start..start + 4].try_into().unwrap());
                    }
                    DhcpOption::Router { routers }
                }
                6 => {
                    let n = op_len / 4;
                    let mut dns_servers = Vec::with_capacity(n);
                    for i in 0..n {
                        let start = off + 2 + 4 * i;
                        dns_servers.push(buf[start..start + 4].try_into().unwrap());
                    }
                    DhcpOption::Dns { dns_servers }
                }
                12 => {
                    let start = off + 2;
                    let end = (off + 2 + op_len).min(buf.len());
                    let host_name = String::from_utf8_lossy(&buf[start..end]).into_owned();
                    DhcpOption::HostName { host_name }
                }
                15 => {
                    let start = off + 2;
                    let end = (off + 2 + op_len).min(buf.len());
                    let domain_name = String::from_utf8_lossy(&buf[start..end]).into_owned();
                    DhcpOption::DnsName { domain_name }
                }
                28 => DhcpOption::Broadcast {
                    broadcast_ip: buf[off + 2..off + 6].try_into().unwrap(),
                },
                46 => {
                    let t = buf[off + 2];
                    DhcpOption::NbiosType {
                        h_node: (t & (1 << 3)) != 0,
                        m_node: (t & (1 << 2)) != 0,
                        p_node: (t & (1 << 1)) != 0,
                        b_node: (t & 1) != 0,
                    }
                }
                50 => DhcpOption::RequestedIp {
                    requested_ip: buf[off + 2..off + 6].try_into().unwrap(),
                },
                51 => DhcpOption::IpLeaseTime {
                    lease_time: read_u32(buf, &mut (off + 2)),
                },
                53 => DhcpOption::Message { message: buf[off + 2] },
                54 => DhcpOption::ServerIp {
                    server_ip: buf[off + 2..off + 6].try_into().unwrap(),
                },
                55 => {
                    let start = off + 2;
                    let end = (off + 2 + op_len).min(buf.len());
                    DhcpOption::ParameterRequestList {
                        requests: buf[start..end].to_vec(),
                    }
                }
                56 => {
                    let start = off + 2;
                    let end = (off + 2 + op_len).min(buf.len());
                    let message = String::from_utf8_lossy(&buf[start..end]).into_owned();
                    DhcpOption::MessageString { message }
                }
                57 => DhcpOption::MaxMessageSize {
                    max_message_size: read_u16(buf, &mut (off + 2)),
                },
                58 => DhcpOption::RenewalTimeT1 {
                    t1: read_u32(buf, &mut (off + 2)),
                },
                59 => DhcpOption::RebindingTimeT2 {
                    t2: read_u32(buf, &mut (off + 2)),
                },
                60 => {
                    let start = off + 2;
                    let end = (off + 2 + op_len).min(buf.len());
                    let class_id = String::from_utf8_lossy(&buf[start..end]).into_owned();
                    DhcpOption::ClassId { class_id }
                }
                61 => {
                    let start = off + 2;
                    let end = (off + 2 + op_len).min(buf.len());
                    DhcpOption::ClientId {
                        client_id: buf[start..end].to_vec(),
                    }
                }
                _ => {
                    // Unknown -- skip without adding.
                    off += 2 + op_len;
                    continue;
                }
            };
            options.push(opt);
            off += 2 + op_len;
            if off >= buf.len() {
                // Bail out gracefully: original code logged an error.
                break;
            }
        }

        Self {
            op,
            hardware_type,
            hardware_address_length,
            hops,
            transaction_id,
            seconds,
            flags,
            client_ip,
            your_ip,
            server_ip,
            gateway_ip,
            client_hardware_address,
            magic_cookie,
            max_length: buf.len(),
            options,
        }
    }

    /// On-wire length of the options area (excludes the 240-byte fixed
    /// header).  Matches the C++ `GetLength()`.
    pub fn length(&self) -> usize {
        self.max_length - (8 + 20)
    }

    /// Serialize the packet into a freshly allocated byte vector of
    /// `max_length` bytes, padding the options area with zeros as needed.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = vec![0u8; self.max_length];
        let mut off = 0;
        self.write_bytes(&mut buf, &mut off);
        debug_assert_eq!(off, self.max_length);
        buf
    }

    /// Serialize the packet into the provided buffer at `off`.
    pub fn write_bytes(&self, buf: &mut [u8], off: &mut usize) {
        let start = *off;
        write_u8(buf, off, self.op);
        write_u8(buf, off, self.hardware_type);
        write_u8(buf, off, self.hardware_address_length);
        write_u8(buf, off, self.hops);
        write_u32(buf, off, self.transaction_id);
        write_u16(buf, off, self.seconds);
        write_u16(buf, off, self.flags);
        write_bytes(buf, off, &self.client_ip);
        write_bytes(buf, off, &self.your_ip);
        write_bytes(buf, off, &self.server_ip);
        write_bytes(buf, off, &self.gateway_ip);
        write_bytes(buf, off, &self.client_hardware_address);
        // Skip 64 (sname) + 128 (file) of legacy zeros (already zero in buf).
        *off += 64 + 128;
        write_u32(buf, off, self.magic_cookie);

        let options_total = self.length();
        let mut used = 0usize;
        let mut i = 0;
        while i < self.options.len() {
            let need = self.options[i].length();
            if DHCP_FIXED_LEN + used + need < self.max_length {
                self.options[i].write_bytes(buf, off);
                used += need;
                i += 1;
            } else {
                // Oversized -- close out with End and stop.
                DhcpOption::End.write_bytes(buf, off);
                used = options_total;
                break;
            }
        }
        if i == self.options.len() {
            // Always emit an End marker.
            DhcpOption::End.write_bytes(buf, off);
        }
        // Pad the remainder of the options area with zeros.
        let end = start + self.length();
        if *off < end {
            for b in &mut buf[*off..end] {
                *b = 0;
            }
            *off = end;
        } else {
            *off = end;
        }
    }

    /// Construct an editor view over a mutable backing buffer.
    pub fn editor<'a>(buffer: &'a mut [u8]) -> DhcpPacketEditor<'a> {
        DhcpPacketEditor::new(buffer)
    }
}

/// Mutable in-place editor for a BOOTP/DHCP packet held in a borrowed buffer.
pub struct DhcpPacketEditor<'a> {
    buffer: &'a mut [u8],
}

impl<'a> DhcpPacketEditor<'a> {
    pub fn new(buffer: &'a mut [u8]) -> Self {
        assert!(buffer.len() >= DHCP_FIXED_LEN, "DhcpPacketEditor: short buffer");
        Self { buffer }
    }

    pub fn op(&self) -> u8 { self.buffer[0] }
    pub fn set_op(&mut self, v: u8) { self.buffer[0] = v; }
    pub fn hardware_type(&self) -> u8 { self.buffer[1] }
    pub fn hardware_address_length(&self) -> u8 { self.buffer[2] }
    pub fn hops(&self) -> u8 { self.buffer[3] }
    pub fn set_hops(&mut self, v: u8) { self.buffer[3] = v; }
    pub fn transaction_id(&self) -> u32 {
        u32::from_be_bytes(self.buffer[4..8].try_into().unwrap())
    }
    pub fn set_transaction_id(&mut self, v: u32) {
        self.buffer[4..8].copy_from_slice(&v.to_be_bytes());
    }
    pub fn seconds(&self) -> u16 {
        u16::from_be_bytes(self.buffer[8..10].try_into().unwrap())
    }
    pub fn set_seconds(&mut self, v: u16) {
        self.buffer[8..10].copy_from_slice(&v.to_be_bytes());
    }
    pub fn flags(&self) -> u16 {
        u16::from_be_bytes(self.buffer[10..12].try_into().unwrap())
    }
    pub fn set_flags(&mut self, v: u16) {
        self.buffer[10..12].copy_from_slice(&v.to_be_bytes());
    }
    pub fn client_ip(&self) -> IpAddress {
        self.buffer[12..16].try_into().unwrap()
    }
    pub fn set_client_ip(&mut self, v: IpAddress) {
        self.buffer[12..16].copy_from_slice(&v);
    }
    pub fn your_ip(&self) -> IpAddress {
        self.buffer[16..20].try_into().unwrap()
    }
    pub fn set_your_ip(&mut self, v: IpAddress) {
        self.buffer[16..20].copy_from_slice(&v);
    }
    pub fn server_ip(&self) -> IpAddress {
        self.buffer[20..24].try_into().unwrap()
    }
    pub fn set_server_ip(&mut self, v: IpAddress) {
        self.buffer[20..24].copy_from_slice(&v);
    }
    pub fn gateway_ip(&self) -> IpAddress {
        self.buffer[24..28].try_into().unwrap()
    }
    pub fn set_gateway_ip(&mut self, v: IpAddress) {
        self.buffer[24..28].copy_from_slice(&v);
    }
    pub fn client_hardware_address(&self) -> [u8; 16] {
        self.buffer[28..44].try_into().unwrap()
    }
    pub fn set_client_hardware_address(&mut self, v: [u8; 16]) {
        self.buffer[28..44].copy_from_slice(&v);
    }
    pub fn magic_cookie(&self) -> u32 {
        u32::from_be_bytes(self.buffer[236..240].try_into().unwrap())
    }
    pub fn set_magic_cookie(&mut self, v: u32) {
        self.buffer[236..240].copy_from_slice(&v.to_be_bytes());
    }
    pub fn options(&self) -> &[u8] {
        &self.buffer[DHCP_FIXED_LEN..]
    }
    pub fn options_mut(&mut self) -> &mut [u8] {
        &mut self.buffer[DHCP_FIXED_LEN..]
    }
}

// =============================================================================
// DNS
// =============================================================================

/// A single DNS question entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DnsQuestionEntry {
    pub name: String,
    pub entry_type: u16,
    pub entry_class: u16,
}

impl DnsQuestionEntry {
    pub fn from_bytes(buf: &[u8], off: &mut usize) -> Self {
        let name = read_dns_name(buf, off);
        let entry_type = read_u16(buf, off);
        let entry_class = read_u16(buf, off);
        Self { name, entry_type, entry_class }
    }

    pub fn write_bytes(&self, buf: &mut [u8], off: &mut usize) {
        write_dns_name(buf, off, &self.name);
        write_u16(buf, off, self.entry_type);
        write_u16(buf, off, self.entry_class);
    }

    /// On-wire length in bytes (sum of: label-length prefix + label chars,
    /// null terminator, 2 bytes type, 2 bytes class).
    pub fn length(&self) -> usize {
        let label_bytes: usize = self
            .name
            .split('.')
            .filter(|s| !s.is_empty())
            .map(|s| 1 + s.len())
            .sum();
        label_bytes + 1 + 4
    }
}

/// A single DNS resource record (answer / authority / additional).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DnsResponseEntry {
    pub name: String,
    pub entry_type: u16,
    pub entry_class: u16,
    pub time_to_live: u32,
    pub data: Vec<u8>,
}

impl DnsResponseEntry {
    pub fn from_bytes(buf: &[u8], off: &mut usize) -> Self {
        let name = read_dns_name(buf, off);
        let entry_type = read_u16(buf, off);
        let entry_class = read_u16(buf, off);
        let time_to_live = read_u32(buf, off);
        let data_len = read_u16(buf, off) as usize;
        let data = if *off + data_len <= buf.len() {
            buf[*off..*off + data_len].to_vec()
        } else if *off < buf.len() {
            buf[*off..].to_vec()
        } else {
            Vec::new()
        };
        *off += data_len;
        Self { name, entry_type, entry_class, time_to_live, data }
    }

    pub fn write_bytes(&self, buf: &mut [u8], off: &mut usize) {
        write_dns_name(buf, off, &self.name);
        write_u16(buf, off, self.entry_type);
        write_u16(buf, off, self.entry_class);
        write_u32(buf, off, self.time_to_live);
        write_u16(buf, off, self.data.len() as u16);
        write_bytes(buf, off, &self.data);
    }

    pub fn length(&self) -> usize {
        let label_bytes: usize = self
            .name
            .split('.')
            .filter(|s| !s.is_empty())
            .map(|s| 1 + s.len())
            .sum();
        label_bytes + 1 + 4 + 4 + 2 + self.data.len()
    }
}

/// A parsed DNS packet.
pub struct DnsPacket {
    pub id: u16,
    pub flags1: u8,
    pub flags2: u8,
    pub questions: Vec<DnsQuestionEntry>,
    pub answers: Vec<DnsResponseEntry>,
    pub authorities: Vec<DnsResponseEntry>,
    pub additional: Vec<DnsResponseEntry>,
}

impl DnsPacket {
    /// Parse a DNS packet from the given buffer.
    pub fn from_bytes(buf: &[u8]) -> Self {
        let mut off = 0;
        let id = read_u16(buf, &mut off);
        let flags1 = read_u8(buf, &mut off);
        let flags2 = read_u8(buf, &mut off);
        let q_count = read_u16(buf, &mut off);
        let a_count = read_u16(buf, &mut off);
        let au_count = read_u16(buf, &mut off);
        let ad_count = read_u16(buf, &mut off);

        let mut questions = Vec::with_capacity(q_count as usize);
        for _ in 0..q_count {
            questions.push(DnsQuestionEntry::from_bytes(buf, &mut off));
        }
        let mut answers = Vec::with_capacity(a_count as usize);
        for _ in 0..a_count {
            answers.push(DnsResponseEntry::from_bytes(buf, &mut off));
        }
        let mut authorities = Vec::with_capacity(au_count as usize);
        for _ in 0..au_count {
            authorities.push(DnsResponseEntry::from_bytes(buf, &mut off));
        }
        let mut additional = Vec::with_capacity(ad_count as usize);
        for _ in 0..ad_count {
            additional.push(DnsResponseEntry::from_bytes(buf, &mut off));
        }

        Self { id, flags1, flags2, questions, answers, authorities, additional }
    }

    /// Total on-wire length in bytes.
    pub fn length(&self) -> usize {
        let mut length = 2 * 2 + 4 * 2;
        for q in &self.questions {
            length += q.length();
        }
        for a in &self.answers {
            length += a.length();
        }
        for a in &self.authorities {
            length += a.length();
        }
        for a in &self.additional {
            length += a.length();
        }
        length
    }

    /// Serialize the packet into a freshly allocated byte vector.
    pub fn to_bytes(&self) -> Vec<u8> {
        let total = self.length();
        let mut buf = vec![0u8; total];
        let mut off = 0;
        self.write_bytes(&mut buf, &mut off);
        debug_assert_eq!(off, total);
        buf
    }

    /// Serialize the packet into the provided buffer at `off`.
    pub fn write_bytes(&self, buf: &mut [u8], off: &mut usize) {
        write_u16(buf, off, self.id);
        write_u8(buf, off, self.flags1);
        write_u8(buf, off, self.flags2);
        write_u16(buf, off, self.questions.len() as u16);
        write_u16(buf, off, self.answers.len() as u16);
        write_u16(buf, off, self.authorities.len() as u16);
        write_u16(buf, off, self.additional.len() as u16);
        for q in &self.questions {
            q.write_bytes(buf, off);
        }
        for a in &self.answers {
            a.write_bytes(buf, off);
        }
        for a in &self.authorities {
            a.write_bytes(buf, off);
        }
        for a in &self.additional {
            a.write_bytes(buf, off);
        }
    }

    // -- flag accessors --
    pub fn qr(&self) -> bool { (self.flags1 & (1 << 7)) != 0 }
    pub fn set_qr(&mut self, v: bool) {
        self.flags1 = (self.flags1 & !(1 << 7)) | (((v as u8) & 1) << 7);
    }
    pub fn op_code(&self) -> u8 { (self.flags1 >> 3) & 0xF }
    pub fn set_op_code(&mut self, v: u8) {
        self.flags1 = (self.flags1 & !(0xF << 3)) | ((v & 0xF) << 3);
    }
    pub fn aa(&self) -> bool { (self.flags1 & (1 << 2)) != 0 }
    pub fn set_aa(&mut self, v: bool) {
        self.flags1 = (self.flags1 & !(1 << 2)) | (((v as u8) & 1) << 2);
    }
    pub fn tc(&self) -> bool { (self.flags1 & (1 << 1)) != 0 }
    pub fn set_tc(&mut self, v: bool) {
        self.flags1 = (self.flags1 & !(1 << 1)) | (((v as u8) & 1) << 1);
    }
    pub fn rd(&self) -> bool { (self.flags1 & 1) != 0 }
    pub fn set_rd(&mut self, v: bool) {
        self.flags1 = (self.flags1 & !1) | ((v as u8) & 1);
    }
    pub fn ra(&self) -> bool { (self.flags2 & (1 << 7)) != 0 }
    pub fn set_ra(&mut self, v: bool) {
        self.flags2 = (self.flags2 & !(1 << 7)) | (((v as u8) & 1) << 7);
    }
    pub fn z0(&self) -> bool { (self.flags2 & (1 << 6)) != 0 }
    pub fn set_z0(&mut self, v: bool) {
        self.flags2 = (self.flags2 & !(1 << 6)) | (((v as u8) & 1) << 6);
    }
    pub fn ad(&self) -> bool { (self.flags2 & (1 << 5)) != 0 }
    pub fn set_ad(&mut self, v: bool) {
        self.flags2 = (self.flags2 & !(1 << 5)) | (((v as u8) & 1) << 5);
    }
    pub fn cd(&self) -> bool { (self.flags2 & (1 << 4)) != 0 }
    pub fn set_cd(&mut self, v: bool) {
        self.flags2 = (self.flags2 & !(1 << 4)) | (((v as u8) & 1) << 4);
    }
    pub fn r_code(&self) -> u8 { self.flags2 & 0xF }
    pub fn set_r_code(&mut self, v: u8) {
        self.flags2 = (self.flags2 & !0xF) | (v & 0xF);
    }

    /// Construct an editor view over a mutable backing buffer.
    pub fn editor<'a>(buffer: &'a mut [u8]) -> DnsPacketEditor<'a> {
        DnsPacketEditor::new(buffer)
    }
}

/// Mutable in-place editor for a DNS packet held in a borrowed buffer.
pub struct DnsPacketEditor<'a> {
    buffer: &'a mut [u8],
}

impl<'a> DnsPacketEditor<'a> {
    pub fn new(buffer: &'a mut [u8]) -> Self {
        assert!(buffer.len() >= 12, "DnsPacketEditor: buffer smaller than 12 bytes");
        Self { buffer }
    }

    pub fn id(&self) -> u16 {
        u16::from_be_bytes(self.buffer[0..2].try_into().unwrap())
    }
    pub fn set_id(&mut self, v: u16) {
        self.buffer[0..2].copy_from_slice(&v.to_be_bytes());
    }
    pub fn flags1(&self) -> u8 { self.buffer[2] }
    pub fn set_flags1(&mut self, v: u8) { self.buffer[2] = v; }
    pub fn flags2(&self) -> u8 { self.buffer[3] }
    pub fn set_flags2(&mut self, v: u8) { self.buffer[3] = v; }
    pub fn q_count(&self) -> u16 {
        u16::from_be_bytes(self.buffer[4..6].try_into().unwrap())
    }
    pub fn a_count(&self) -> u16 {
        u16::from_be_bytes(self.buffer[6..8].try_into().unwrap())
    }
    pub fn au_count(&self) -> u16 {
        u16::from_be_bytes(self.buffer[8..10].try_into().unwrap())
    }
    pub fn ad_count(&self) -> u16 {
        u16::from_be_bytes(self.buffer[10..12].try_into().unwrap())
    }
    pub fn questions_and_records(&self) -> &[u8] {
        &self.buffer[12..]
    }
    pub fn questions_and_records_mut(&mut self) -> &mut [u8] {
        &mut self.buffer[12..]
    }
}

// -----------------------------------------------------------------------------
// Internal: DNS name codec (with pointer compression support on read).
// -----------------------------------------------------------------------------

fn read_dns_name(buf: &[u8], off: &mut usize) -> String {
    let mut name = String::new();
    let mut current = *off;
    let mut jumped = false;
    let mut return_to = *off;

    loop {
        if current >= buf.len() {
            break;
        }
        let len = buf[current];
        if len == 0 {
            current += 1;
            break;
        }
        if len >= 192 {
            // Pointer: top 2 bits set.
            if current + 1 >= buf.len() {
                break;
            }
            let b0 = buf[current] & 0x3F;
            let b1 = buf[current + 1];
            let ptr = ((b0 as usize) << 8) | (b1 as usize);
            if !jumped {
                return_to = current + 2;
            }
            current = ptr;
            jumped = true;
            continue;
        }
        current += 1;
        if current + (len as usize) > buf.len() {
            break;
        }
        name.push_str(
            std::str::from_utf8(&buf[current..current + len as usize]).unwrap_or(""),
        );
        current += len as usize;
        if current < buf.len() && buf[current] != 0 {
            name.push('.');
        }
    }

    *off = if jumped { return_to } else { current };
    name
}

fn write_dns_name(buf: &mut [u8], off: &mut usize, name: &str) {
    let bytes = name.as_bytes();
    let mut segment_length = 0usize;
    let mut segment_start = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'.' {
            if segment_length == 0 {
                continue;
            }
            write_u8(buf, off, segment_length as u8);
            write_bytes(buf, off, &bytes[segment_start..segment_start + segment_length]);
            segment_length = 0;
            segment_start = i + 1;
        } else {
            segment_length += 1;
        }
    }
    if segment_length != 0 {
        write_u8(buf, off, segment_length as u8);
        write_bytes(buf, off, &bytes[segment_start..segment_start + segment_length]);
    }
    write_u8(buf, off, 0);
}
