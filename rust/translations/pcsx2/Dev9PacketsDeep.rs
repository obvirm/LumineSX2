//! DEEP translation of `pcsx2/DEV9/PacketReader/**` (Ethernet, IP, ICMP, TCP, UDP,
//! ARP, DNS, DHCP) into a single idiomatic Rust 2021 module.
//!
//! The C++ source is an inheritance-heavy OOP hierarchy built around three payload
//! flavours (`Payload` / `IP_Payload` / `Editor`) and a `NetLib` namespace of
//! `htons/htonl`-flavoured byte readers/writers. This module collapses that
//! hierarchy into a small set of concrete structs and enums, preserving
//! field-for-field wire compatibility while using idiomatic Rust ownership and
//! pattern matching.
//!
//! Wire format notes preserved from the C++:
//!  * All multi-byte integers are big-endian on the wire.
//!  * `IP_Packet`, `TCP_Packet`, `UDP_Packet`, `ICMP_Packet` and the option
//!    parsers all align headers to 4-byte boundaries.
//!  * `DHCP_Packet::WriteBytes` fills its fixed-size container (default
//!    `maxLength = 576`) and appends an `END` option if options overflow.
//!  * DNS name compression (pointers) is supported during read but not emitted
//!    on write (matches the C++ `WriteDNS_String` behaviour).

#![allow(clippy::upper_case_acronyms)]

use std::fmt;

// ---------------------------------------------------------------------------
// Primitive aliases
// ---------------------------------------------------------------------------

pub type u8 = std::primitive::u8;
pub type u16 = std::primitive::u16;
pub type u32 = std::primitive::u32;
pub type i32 = std::primitive::i32;

// ---------------------------------------------------------------------------
// Byte-order helpers (NetLib)
// ---------------------------------------------------------------------------

/// Read a big-endian `u16` at `buf[*offset..]` and advance the offset.
#[inline]
pub fn read_u16(buf: &[u8], offset: &mut usize) -> u16 {
    let v = u16::from_be_bytes([buf[*offset], buf[*offset + 1]]);
    *offset += 2;
    v
}

/// Read a big-endian `u32` at `buf[*offset..]` and advance the offset.
#[inline]
pub fn read_u32(buf: &[u8], offset: &mut usize) -> u32 {
    let v = u32::from_be_bytes([
        buf[*offset],
        buf[*offset + 1],
        buf[*offset + 2],
        buf[*offset + 3],
    ]);
    *offset += 4;
    v
}

/// Read a single byte and advance the offset.
#[inline]
pub fn read_u8(buf: &[u8], offset: &mut usize) -> u8 {
    let v = buf[*offset];
    *offset += 1;
    v
}

/// Read `len` bytes from `buf[*offset..]` into a `Vec<u8>`.
#[inline]
pub fn read_bytes(buf: &[u8], offset: &mut usize, len: usize) -> Vec<u8> {
    let out = buf[*offset..*offset + len].to_vec();
    *offset += len;
    out
}

/// Write a big-endian `u16` to `buf[*offset..]`.
#[inline]
pub fn write_u16(buf: &mut [u8], offset: &mut usize, value: u16) {
    buf[*offset..*offset + 2].copy_from_slice(&value.to_be_bytes());
    *offset += 2;
}

/// Write a big-endian `u32` to `buf[*offset..]`.
#[inline]
pub fn write_u32(buf: &mut [u8], offset: &mut usize, value: u32) {
    buf[*offset..*offset + 4].copy_from_slice(&value.to_be_bytes());
    *offset += 4;
}

/// Write a single byte.
#[inline]
pub fn write_u8(buf: &mut [u8], offset: &mut usize, value: u8) {
    buf[*offset] = value;
    *offset += 1;
}

/// Write a raw byte slice.
#[inline]
pub fn write_bytes(buf: &mut [u8], offset: &mut usize, value: &[u8]) {
    buf[*offset..*offset + value.len()].copy_from_slice(value);
    *offset += value.len();
}

/// Mirror of `Common::AlignUpPow2` used by TCP/IP header length rounding.
#[inline]
pub fn align_up_pow2(value: usize, align: usize) -> usize {
    (value + align - 1) & !(align - 1)
}

/// One's-complement internet checksum (RFC 1071). Returns the final
/// complemented sum suitable for storage in a header field.
pub fn internet_checksum(buf: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;
    let mut len = buf.len();
    while len > 1 {
        let word = u16::from_be_bytes([buf[i], buf[i + 1]]) as u32;
        sum += word;
        if (sum & 0xFFFF_0000) != 0 {
            sum = (sum & 0xFFFF) + 1;
        }
        i += 2;
        len -= 2;
    }
    if len > 0 {
        sum += (buf[i] as u32) << 8;
        if (sum & 0xFFFF_0000) != 0 {
            sum = (sum & 0xFFFF) + 1;
        }
    }
    (!(sum & 0xFFFF) & 0xFFFF) as u16
}

// ---------------------------------------------------------------------------
// MAC_Address
// ---------------------------------------------------------------------------

/// 6-byte IEEE 802 MAC address, packed in network byte order on the wire.
#[derive(Clone, Copy, Default, Eq)]
pub struct MacAddress {
    bytes: [u8; 6],
}

impl MacAddress {
    pub const ZERO: Self = Self { bytes: [0; 6] };

    #[inline]
    pub const fn new(b: [u8; 6]) -> Self {
        Self { bytes: b }
    }

    #[inline]
    pub fn from_bytes(buf: &[u8]) -> Self {
        let mut bytes = [0u8; 6];
        bytes.copy_from_slice(&buf[..6]);
        Self { bytes }
    }

    #[inline]
    pub fn write_to(&self, buf: &mut [u8]) {
        buf[..6].copy_from_slice(&self.bytes);
    }

    #[inline]
    pub fn as_bytes(&self) -> &[u8; 6] {
        &self.bytes
    }
}

impl PartialEq for MacAddress {
    fn eq(&self, other: &Self) -> bool {
        self.bytes == other.bytes
    }
}

impl fmt::Debug for MacAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            self.bytes[0], self.bytes[1], self.bytes[2], self.bytes[3], self.bytes[4], self.bytes[5]
        )
    }
}

// ---------------------------------------------------------------------------
// IP_Address
// ---------------------------------------------------------------------------

/// 4-byte IPv4 address stored in network byte order (same as the C++ union).
#[derive(Clone, Copy, Default, Eq)]
pub struct IpAddress {
    bytes: [u8; 4],
}

impl IpAddress {
    pub const ZERO: Self = Self { bytes: [0; 4] };

    #[inline]
    pub const fn new(b: [u8; 4]) -> Self {
        Self { bytes: b }
    }

    #[inline]
    pub fn from_bytes(buf: &[u8]) -> Self {
        let mut bytes = [0u8; 4];
        bytes.copy_from_slice(&buf[..4]);
        Self { bytes }
    }

    #[inline]
    pub fn write_to(&self, buf: &mut [u8]) {
        buf[..4].copy_from_slice(&self.bytes);
    }

    #[inline]
    pub fn as_bytes(&self) -> &[u8; 4] {
        &self.bytes
    }
}

impl PartialEq for IpAddress {
    fn eq(&self, other: &Self) -> bool {
        self.bytes == other.bytes
    }
}

impl fmt::Debug for IpAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}.{}.{}.{}",
            self.bytes[0], self.bytes[1], self.bytes[2], self.bytes[3]
        )
    }
}

// ---------------------------------------------------------------------------
// Payload (opaque byte buffer with `pointer` / `set_pointer`)
// ---------------------------------------------------------------------------

/// Mirrors the C++ `Payload` class — the abstract base of every packet type.
///
/// `data` is an opaque owned byte buffer. The C++ had three flavours
/// (`Payload`, `PayloadData`, `PayloadPtr`); we model them with a single
/// `Payload` whose `pointer` is a `usize` cursor into the buffer.
#[derive(Clone, Debug, Default)]
pub struct Payload {
    data: Vec<u8>,
    pointer: usize,
}

impl Payload {
    /// Construct a payload that owns `len` zero-initialised bytes.
    pub fn new(len: usize) -> Self {
        Self {
            data: vec![0u8; len],
            pointer: 0,
        }
    }

    /// Construct a payload that owns a copy of `data`.
    pub fn from_vec(data: Vec<u8>) -> Self {
        Self { data, pointer: 0 }
    }

    /// Construct a payload over an existing slice (copy semantics).
    pub fn from_slice(data: &[u8]) -> Self {
        Self {
            data: data.to_vec(),
            pointer: 0,
        }
    }

    /// Current read/write cursor (matches the C++ `offset` field).
    #[inline]
    pub fn pointer(&self) -> usize {
        self.pointer
    }

    /// Set the read/write cursor.
    #[inline]
    pub fn set_pointer(&mut self, p: usize) {
        self.pointer = p;
    }

    /// Total buffer length (matches `Payload::GetLength`).
    #[inline]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// `true` when the buffer is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Borrow the underlying buffer.
    #[inline]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Mutable borrow of the underlying buffer.
    #[inline]
    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    /// Append a byte at the current pointer and advance.
    pub fn write_u8_at(&mut self, value: u8) {
        self.data[self.pointer] = value;
        self.pointer += 1;
    }

    /// Read a byte at the current pointer and advance.
    pub fn read_u8_at(&mut self) -> u8 {
        let v = self.data[self.pointer];
        self.pointer += 1;
        v
    }
}

// ---------------------------------------------------------------------------
// EtherType
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum EtherType {
    Null = 0x0000,
    IPv4 = 0x0800,
    Arp = 0x0806,
    VlanQTag = 0x8100,
    VlanServiceQTag = 0x88A8,
    VlanDoubleQTag = 0x9100,
}

impl EtherType {
    pub fn from_u16(v: u16) -> Self {
        match v {
            0x0800 => Self::IPv4,
            0x0806 => Self::Arp,
            0x8100 => Self::VlanQTag,
            0x88A8 => Self::VlanServiceQTag,
            0x9100 => Self::VlanDoubleQTag,
            _ => Self::Null,
        }
    }

    pub fn to_u16(self) -> u16 {
        self as u16
    }
}

// ---------------------------------------------------------------------------
// IP protocol numbers
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum IpType {
    ICMP = 0x01,
    IGMP = 0x02,
    TCP = 0x06,
    UDP = 0x11,
}

impl IpType {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x01 => Self::ICMP,
            0x02 => Self::IGMP,
            0x06 => Self::TCP,
            0x11 => Self::UDP,
            _ => Self::ICMP,
        }
    }

    pub fn to_u8(self) -> u8 {
        self as u8
    }
}

// ===========================================================================
// Ethernet frame
// ===========================================================================

/// Mirrors `EthernetFrame` from `DEV9/PacketReader/EthernetFrame.{h,cpp}`.
#[derive(Clone, Debug)]
pub struct EthernetFrame {
    pub destination_mac: MacAddress,
    pub source_mac: MacAddress,
    pub protocol: EtherType,
    pub header_length: usize,
    pub payload: Payload,
}

impl EthernetFrame {
    pub const HEADER_LENGTH: usize = 14;

    /// Build a frame around an opaque payload.
    pub fn new(payload: Payload) -> Self {
        Self {
            destination_mac: MacAddress::ZERO,
            source_mac: MacAddress::ZERO,
            protocol: EtherType::Null,
            header_length: Self::HEADER_LENGTH,
            payload,
        }
    }

    /// Parse a frame from `buf`.
    pub fn from_bytes(buf: &[u8]) -> Self {
        let destination_mac = MacAddress::from_bytes(&buf[0..6]);
        let source_mac = MacAddress::from_bytes(&buf[6..12]);
        let protocol = EtherType::from_u16(u16::from_be_bytes([buf[12], buf[13]]));
        let payload = Payload::from_slice(&buf[Self::HEADER_LENGTH..]);
        Self {
            destination_mac,
            source_mac,
            protocol,
            header_length: Self::HEADER_LENGTH,
            payload,
        }
    }

    /// Serialise the frame into `out`, returning the number of bytes written.
    pub fn to_bytes(&self, out: &mut [u8]) -> usize {
        let mut off = 0;
        self.destination_mac.write_to(&mut out[off..]);
        off += 6;
        self.source_mac.write_to(&mut out[off..]);
        off += 6;
        out[off..off + 2].copy_from_slice(&self.protocol.to_u16().to_be_bytes());
        off += 2;
        let p = self.payload.data();
        out[off..off + p.len()].copy_from_slice(p);
        off += p.len();
        off
    }

    pub fn get_payload(&self) -> &Payload {
        &self.payload
    }

    pub fn get_payload_mut(&mut self) -> &mut Payload {
        &mut self.payload
    }
}

/// Read-only view over an in-place ethernet frame, mirroring
/// `EthernetFrameEditor`.
#[derive(Debug)]
pub struct EthernetFrameEditor<'a> {
    pub destination_mac: &'a mut MacAddress,
    pub source_mac: &'a mut MacAddress,
    pub protocol: &'a mut u16,
    pub header_length: usize,
}

impl<'a> EthernetFrameEditor<'a> {
    pub fn new(buf: &'a mut [u8]) -> Self {
        let dst = unsafe { &mut *(buf.as_mut_ptr() as *mut MacAddress) };
        let src = unsafe { &mut *(buf[6..].as_mut_ptr() as *mut MacAddress) };
        let proto = unsafe { &mut *(buf[12..].as_mut_ptr() as *mut u16) };
        Self {
            destination_mac: dst,
            source_mac: src,
            protocol: proto,
            header_length: EthernetFrame::HEADER_LENGTH,
        }
    }

    pub fn get_destination_mac(&self) -> MacAddress {
        *self.destination_mac
    }

    pub fn set_destination_mac(&mut self, value: MacAddress) {
        *self.destination_mac = value;
    }

    pub fn get_source_mac(&self) -> MacAddress {
        *self.source_mac
    }

    pub fn set_source_mac(&mut self, value: MacAddress) {
        *self.source_mac = value;
    }

    pub fn get_protocol(&self) -> EtherType {
        EtherType::from_u16(u16::from_be(*self.protocol))
    }

    pub fn payload<'b>(&self, buf: &'b [u8]) -> &'b [u8] {
        &buf[self.header_length..]
    }
}

// ===========================================================================
// ARP packet
// ===========================================================================

/// Mirrors `ARP_Packet`. The address fields are stored as `Vec<u8>` so the
/// packet can represent any (hardwareAddressLength, protocolAddressLength)
/// combination, not just MAC+IPv4.
#[derive(Clone, Debug)]
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
    pub fn new(hw_addr_len: u8, proto_addr_len: u8) -> Self {
        Self {
            hardware_type: 0,
            protocol: 0,
            hardware_address_length: hw_addr_len,
            protocol_address_length: proto_addr_len,
            op: 0,
            sender_hardware_address: vec![0; hw_addr_len as usize],
            sender_protocol_address: vec![0; proto_addr_len as usize],
            target_hardware_address: vec![0; hw_addr_len as usize],
            target_protocol_address: vec![0; proto_addr_len as usize],
        }
    }

    pub fn from_bytes(buf: &[u8]) -> Self {
        let mut off = 0;
        let hardware_type = read_u16(buf, &mut off);
        let protocol = read_u16(buf, &mut off);
        let hardware_address_length = read_u8(buf, &mut off);
        let protocol_address_length = read_u8(buf, &mut off);
        let op = read_u16(buf, &mut off);
        let sender_hardware_address = read_bytes(buf, &mut off, hardware_address_length as usize);
        let sender_protocol_address = read_bytes(buf, &mut off, protocol_address_length as usize);
        let target_hardware_address = read_bytes(buf, &mut off, hardware_address_length as usize);
        let target_protocol_address = read_bytes(buf, &mut off, protocol_address_length as usize);
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

    pub fn get_length(&self) -> usize {
        8 + 2 * self.hardware_address_length as usize + 2 * self.protocol_address_length as usize
    }

    pub fn write_bytes(&self, buf: &mut [u8]) {
        let mut off = 0;
        write_u16(buf, &mut off, self.hardware_type);
        write_u16(buf, &mut off, self.protocol);
        write_u8(buf, &mut off, self.hardware_address_length);
        write_u8(buf, &mut off, self.protocol_address_length);
        write_u16(buf, &mut off, self.op);
        write_bytes(buf, &mut off, &self.sender_hardware_address);
        write_bytes(buf, &mut off, &self.sender_protocol_address);
        write_bytes(buf, &mut off, &self.target_hardware_address);
        write_bytes(buf, &mut off, &self.target_protocol_address);
    }

    pub fn clone_packet(&self) -> Self {
        self.clone()
    }
}

/// Read-only view over an in-place ARP packet, mirroring `ARP_PacketEditor`.
#[derive(Clone, Copy, Debug)]
pub struct ArpPacketEditor<'a> {
    pub data: &'a [u8],
}

impl<'a> ArpPacketEditor<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    pub fn get_hardware_type(&self) -> u16 {
        u16::from_be_bytes([self.data[0], self.data[1]])
    }
    pub fn get_protocol(&self) -> u16 {
        u16::from_be_bytes([self.data[2], self.data[3]])
    }
    pub fn get_hardware_address_length(&self) -> u8 {
        self.data[4]
    }
    pub fn get_protocol_address_length(&self) -> u8 {
        self.data[5]
    }
    pub fn get_op(&self) -> u16 {
        u16::from_be_bytes([self.data[6], self.data[7]])
    }

    pub fn sender_hardware_address(&self) -> &[u8] {
        &self.data[8..8 + self.get_hardware_address_length() as usize]
    }

    pub fn sender_protocol_address(&self) -> &[u8] {
        let o = 8 + self.get_hardware_address_length() as usize;
        &self.data[o..o + self.get_protocol_address_length() as usize]
    }

    pub fn target_hardware_address(&self) -> &[u8] {
        let o = 8 + self.get_hardware_address_length() as usize + self.get_protocol_address_length() as usize;
        &self.data[o..o + self.get_hardware_address_length() as usize]
    }

    pub fn target_protocol_address(&self) -> &[u8] {
        let o = 8
            + 2 * self.get_hardware_address_length() as usize
            + self.get_protocol_address_length() as usize;
        &self.data[o..o + self.get_protocol_address_length() as usize]
    }

    pub fn get_length(&self) -> usize {
        8 + 2 * self.get_hardware_address_length() as usize + 2 * self.get_protocol_address_length() as usize
    }
}

// ===========================================================================
// IP options
// ===========================================================================

/// One IP-option as decoded from a packet (mirrors `IP_Options.h`).
#[derive(Clone, Debug)]
pub enum IpOption {
    /// End-of-option-list marker (code 0).
    End,
    /// No-operation (code 1, length 1).
    Nop,
    /// Router alert (code 0x94 / 148).
    RouterAlert { value: u16 },
    /// Unknown option; raw kind/length/value preserved for round-tripping.
    Unknown { code: u8, length: u8, value: Vec<u8> },
}

impl IpOption {
    pub fn code(&self) -> u8 {
        match self {
            Self::End => 0,
            Self::Nop => 1,
            Self::RouterAlert { .. } => 148,
            Self::Unknown { code, .. } => *code,
        }
    }

    pub fn length(&self) -> u8 {
        match self {
            Self::End => 1,
            Self::Nop => 1,
            Self::RouterAlert { .. } => 4,
            Self::Unknown { length, .. } => *length,
        }
    }

    pub fn is_copy_on_fragment(&self) -> bool {
        (self.code() & 0x80) != 0
    }

    pub fn class(&self) -> u8 {
        (self.code() >> 5) & 0x3
    }

    pub fn number(&self) -> u8 {
        self.code() & 0x1F
    }

    pub fn write_bytes(&self, buf: &mut [u8]) {
        let mut off = 0;
        self.write_into(&mut off, buf);
    }

    fn write_into(&self, off: &mut usize, buf: &mut [u8]) {
        match self {
            Self::End => {
                write_u8(buf, off, 0);
            }
            Self::Nop => {
                write_u8(buf, off, 1);
            }
            Self::RouterAlert { value } => {
                write_u8(buf, off, 148);
                write_u8(buf, off, 4);
                write_u16(buf, off, *value);
            }
            Self::Unknown { code, length, value } => {
                write_u8(buf, off, *code);
                write_u8(buf, off, *length);
                write_bytes(buf, off, value);
            }
        }
    }
}

// ===========================================================================
// IP_Packet
// ===========================================================================

/// Mirrors `IP_Packet` from `DEV9/PacketReader/IP/IP_Packet.{h,cpp}`.
#[derive(Clone, Debug)]
pub struct IpPacket {
    pub dscp: u8,
    pub id: u16,
    pub fragment_flags1: u8,
    pub fragment_flags2: u8,
    pub time_to_live: u8,
    pub protocol: IpType,
    pub checksum: u16,
    pub source_ip: IpAddress,
    pub destination_ip: IpAddress,
    pub options: Vec<IpOption>,
    /// Owning payload.
    pub payload: Payload,
    /// Cached header length (mirrors the C++ `headerLength` field).
    pub header_length: usize,
    /// `_verHi` is fixed to `0x40` (version 4) plus the IHL nibble.
    pub version_ihl: u8,
}

impl Default for IpPacket {
    fn default() -> Self {
        Self {
            dscp: 0,
            id: 0,
            fragment_flags1: 0,
            fragment_flags2: 0,
            time_to_live: 0,
            protocol: IpType::ICMP,
            checksum: 0,
            source_ip: IpAddress::ZERO,
            destination_ip: IpAddress::ZERO,
            options: Vec::new(),
            payload: Payload::new(0),
            header_length: 20,
            version_ihl: 0x45,
        }
    }
}

impl IpPacket {
    pub const DEFAULT_HEADER_LENGTH: usize = 20;

    pub fn new(payload: Payload, protocol: IpType) -> Self {
        Self {
            protocol,
            payload,
            ..Self::default()
        }
    }

    pub fn from_bytes(buf: &[u8]) -> Self {
        Self::from_bytes_inner(buf, false)
    }

    fn from_bytes_inner(buf: &[u8], _from_icmp: bool) -> Self {
        let mut off = 0;
        let v_hl = read_u8(buf, &mut off);
        let header_length = ((v_hl & 0x0F) as usize) << 2;
        let dscp = read_u8(buf, &mut off);
        let length = read_u16(buf, &mut off);
        let length = length as usize;
        let id = read_u16(buf, &mut off);
        let fragment_flags1 = read_u8(buf, &mut off);
        let fragment_flags2 = read_u8(buf, &mut off);
        let time_to_live = read_u8(buf, &mut off);
        let protocol = IpType::from_u8(read_u8(buf, &mut off));
        let checksum = read_u16(buf, &mut off);
        let source_ip = IpAddress::from_bytes(&buf[off..off + 4]);
        off += 4;
        let destination_ip = IpAddress::from_bytes(&buf[off..off + 4]);
        off += 4;

        let mut options = Vec::new();
        if header_length > Self::DEFAULT_HEADER_LENGTH {
            let mut done = false;
            while !done && off < header_length {
                let op_kind = buf[off];
                let op_len = if off + 1 < buf.len() { buf[off + 1] } else { 0 };
                match op_kind {
                    0 => {
                        options.push(IpOption::End);
                        done = true;
                    }
                    1 => {
                        options.push(IpOption::Nop);
                        off += 1;
                    }
                    148 => {
                        let mut lo = off + 2;
                        let value = read_u16(buf, &mut lo);
                        options.push(IpOption::RouterAlert { value });
                        off += op_len as usize;
                    }
                    _ => {
                        let value_len = op_len.saturating_sub(2) as usize;
                        let value = if off + 2 + value_len <= buf.len() {
                            buf[off + 2..off + 2 + value_len].to_vec()
                        } else {
                            Vec::new()
                        };
                        options.push(IpOption::Unknown {
                            code: op_kind,
                            length: op_len,
                            value,
                        });
                        off += op_len as usize;
                    }
                }
                if off == header_length {
                    done = true;
                }
            }
            off = header_length;
        } else {
            off = Self::DEFAULT_HEADER_LENGTH;
        }

        // _ = length: useful only for warning behaviour in C++. Use it to clip.
        let payload_len = length.saturating_sub(off).min(buf.len().saturating_sub(off));
        let payload = Payload::from_slice(&buf[off..off + payload_len]);

        Self {
            dscp,
            id,
            fragment_flags1,
            fragment_flags2,
            time_to_live,
            protocol,
            checksum,
            source_ip,
            destination_ip,
            options,
            payload,
            header_length,
            version_ihl: v_hl,
        }
    }

    pub fn get_header_length(&self) -> usize {
        self.header_length
    }

    pub fn get_dscp_value(&self) -> u8 {
        (self.dscp >> 2) & 0x3F
    }
    pub fn set_dscp_value(&mut self, value: u8) {
        self.dscp = (self.dscp & !(0x3F << 2)) | ((value & 0x3F) << 2);
    }

    pub fn get_dscp_ecn(&self) -> u8 {
        self.dscp & 0x3
    }
    pub fn set_dscp_ecn(&mut self, value: u8) {
        self.dscp = (self.dscp & !0x3) | (value & 0x3);
    }

    pub fn get_do_not_fragment(&self) -> bool {
        (self.fragment_flags1 & (1 << 6)) != 0
    }
    pub fn set_do_not_fragment(&mut self, value: bool) {
        let bit = (value as u8) & 1;
        self.fragment_flags1 = (self.fragment_flags1 & !(0x1 << 6)) | (bit << 6);
    }

    pub fn get_more_fragments(&self) -> bool {
        (self.fragment_flags1 & (1 << 5)) != 0
    }
    pub fn set_more_fragments(&mut self, value: bool) {
        let bit = (value as u8) & 1;
        self.fragment_flags1 = (self.fragment_flags1 & !(0x1 << 5)) | (bit << 5);
    }

    pub fn get_fragment_offset(&self) -> u16 {
        let masked = self.fragment_flags1 & 0x1F;
        u16::from_be_bytes([masked, self.fragment_flags2])
    }

    pub fn get_length(&mut self) -> usize {
        self.recompute_header_len();
        self.header_length + self.payload.len()
    }

    fn recompute_header_len(&mut self) {
        let mut op_offset = Self::DEFAULT_HEADER_LENGTH;
        for opt in &self.options {
            op_offset += opt.length() as usize;
        }
        self.header_length = align_up_pow2(op_offset, 4);
    }

    pub fn calculate_checksum(&mut self) {
        self.recompute_header_len();
        let mut hdr = vec![0u8; self.header_length];
        let mut counter = 0;
        write_u8(&mut hdr, &mut counter, 0x40 | ((self.header_length >> 2) as u8));
        write_u8(&mut hdr, &mut counter, self.dscp);
        write_u16(&mut hdr, &mut counter, self.get_length() as u16);
        write_u16(&mut hdr, &mut counter, self.id);
        write_u8(&mut hdr, &mut counter, self.fragment_flags1);
        write_u8(&mut hdr, &mut counter, self.fragment_flags2);
        write_u8(&mut hdr, &mut counter, self.time_to_live);
        write_u8(&mut hdr, &mut counter, self.protocol.to_u8());
        write_u16(&mut hdr, &mut counter, 0);
        self.source_ip.write_to(&mut hdr[counter..counter + 4]);
        counter += 4;
        self.destination_ip.write_to(&mut hdr[counter..counter + 4]);
        counter += 4;
        for opt in &self.options {
            opt.write_into(&mut counter, &mut hdr);
        }
        self.checksum = internet_checksum(&hdr);
    }

    pub fn verify_checksum(&mut self) -> bool {
        self.recompute_header_len();
        let mut hdr = vec![0u8; self.header_length];
        let mut counter = 0;
        write_u8(&mut hdr, &mut counter, 0x40 | ((self.header_length >> 2) as u8));
        write_u8(&mut hdr, &mut counter, self.dscp);
        write_u16(&mut hdr, &mut counter, self.get_length() as u16);
        write_u16(&mut hdr, &mut counter, self.id);
        write_u8(&mut hdr, &mut counter, self.fragment_flags1);
        write_u8(&mut hdr, &mut counter, self.fragment_flags2);
        write_u8(&mut hdr, &mut counter, self.time_to_live);
        write_u8(&mut hdr, &mut counter, self.protocol.to_u8());
        write_u16(&mut hdr, &mut counter, self.checksum);
        self.source_ip.write_to(&mut hdr[counter..counter + 4]);
        counter += 4;
        self.destination_ip.write_to(&mut hdr[counter..counter + 4]);
        counter += 4;
        for opt in &self.options {
            opt.write_into(&mut counter, &mut hdr);
        }
        internet_checksum(&hdr) == 0
    }

    pub fn write_bytes(&mut self, buf: &mut [u8]) {
        self.recompute_header_len();
        let start = 0;
        let mut counter = start;
        write_u8(buf, &mut counter, 0x40 | ((self.header_length >> 2) as u8));
        write_u8(buf, &mut counter, self.dscp);
        write_u16(buf, &mut counter, (self.header_length + self.payload.len()) as u16);
        write_u16(buf, &mut counter, self.id);
        write_u8(buf, &mut counter, self.fragment_flags1);
        write_u8(buf, &mut counter, self.fragment_flags2);
        write_u8(buf, &mut counter, self.time_to_live);
        write_u8(buf, &mut counter, self.protocol.to_u8());
        write_u16(buf, &mut counter, self.checksum);
        self.source_ip.write_to(&mut buf[counter..counter + 4]);
        counter += 4;
        self.destination_ip.write_to(&mut buf[counter..counter + 4]);
        counter += 4;
        for opt in &self.options {
            opt.write_into(&mut counter, buf);
        }
        let p = self.payload.data();
        buf[counter..counter + p.len()].copy_from_slice(p);
        counter += p.len();
        let _ = counter;
    }

    pub fn clone_packet(&self) -> Self {
        self.clone()
    }
}

// ===========================================================================
// ICMP packet
// ===========================================================================

/// Mirrors `ICMP_Packet` (header is type/code/checksum + 4 bytes of
/// header-data + owned payload).
#[derive(Clone, Debug)]
pub struct IcmpPacket {
    pub r#type: u8,
    pub code: u8,
    pub checksum: u16,
    pub header_data: [u8; 4],
    pub payload: Payload,
}

impl IcmpPacket {
    pub const HEADER_LENGTH: usize = 8;

    pub fn new(payload: Payload) -> Self {
        Self {
            r#type: 0,
            code: 0,
            checksum: 0,
            header_data: [0; 4],
            payload,
        }
    }

    pub fn from_bytes(buf: &[u8]) -> Self {
        let mut off = 0;
        let r#type = read_u8(buf, &mut off);
        let code = read_u8(buf, &mut off);
        let checksum = read_u16(buf, &mut off);
        let mut header_data = [0u8; 4];
        header_data.copy_from_slice(&buf[off..off + 4]);
        off += 4;
        let payload = if off < buf.len() {
            Payload::from_slice(&buf[off..])
        } else {
            Payload::new(0)
        };
        Self {
            r#type,
            code,
            checksum,
            header_data,
            payload,
        }
    }

    pub fn get_length(&self) -> usize {
        Self::HEADER_LENGTH + self.payload.len()
    }

    pub fn write_bytes(&self, buf: &mut [u8]) {
        let mut off = 0;
        write_u8(buf, &mut off, self.r#type);
        write_u8(buf, &mut off, self.code);
        write_u16(buf, &mut off, self.checksum);
        buf[off..off + 4].copy_from_slice(&self.header_data);
        off += 4;
        let p = self.payload.data();
        buf[off..off + p.len()].copy_from_slice(p);
        off += p.len();
        let _ = off;
    }

    pub fn clone_packet(&self) -> Self {
        self.clone()
    }

    pub fn get_protocol(&self) -> IpType {
        IpType::ICMP
    }

    pub fn calculate_checksum(&mut self, _src_ip: IpAddress, _dst_ip: IpAddress) {
        let mut p_hdr_len = Self::HEADER_LENGTH + self.payload.len();
        if p_hdr_len & 1 != 0 {
            p_hdr_len += 1;
        }
        let mut segment = vec![0u8; p_hdr_len];
        let saved = self.checksum;
        self.checksum = 0;
        self.write_bytes(&mut segment);
        if segment.len() != p_hdr_len {
            // Pad with zero to keep checksum stable.
            segment.push(0);
        }
        self.checksum = internet_checksum(&segment[..p_hdr_len]);
        let _ = saved;
    }

    pub fn verify_checksum(&mut self, _src_ip: IpAddress, _dst_ip: IpAddress) -> bool {
        let mut p_hdr_len = Self::HEADER_LENGTH + self.payload.len();
        if p_hdr_len & 1 != 0 {
            p_hdr_len += 1;
        }
        let mut segment = vec![0u8; p_hdr_len];
        self.write_bytes(&mut segment);
        if segment.len() != p_hdr_len {
            segment.push(0);
        }
        internet_checksum(&segment[..p_hdr_len]) == 0
    }

    /// Helper: pack/unpack the "identifier/sequence" header data block used by
    /// echo request/reply.
    pub fn decode_header_data_identifier(&self) -> IcmpHeaderDataIdentifier {
        let mut off = 0;
        IcmpHeaderDataIdentifier {
            identifier: read_u16(&self.header_data, &mut off),
            sequence_number: read_u16(&self.header_data, &mut off),
        }
    }
}

/// Mirrors `ICMP_HeaderDataIdentifier`.
#[derive(Clone, Copy, Debug, Default)]
pub struct IcmpHeaderDataIdentifier {
    pub identifier: u16,
    pub sequence_number: u16,
}

impl IcmpHeaderDataIdentifier {
    pub fn write_header_data(&self, header_data: &mut [u8; 4]) {
        let mut off = 0;
        write_u16(header_data, &mut off, self.identifier);
        write_u16(header_data, &mut off, self.sequence_number);
    }
}

// ===========================================================================
// TCP options
// ===========================================================================

#[derive(Clone, Debug)]
pub enum TcpOption {
    End,
    Nop,
    Mss { max_segment_size: u16 },
    Ws { window_scale: u8 },
    Ts { sender_time_stamp: u32, echo_time_stamp: u32 },
    Unknown { code: u8, length: u8, value: Vec<u8> },
}

impl TcpOption {
    pub fn code(&self) -> u8 {
        match self {
            Self::End => 0,
            Self::Nop => 1,
            Self::Mss { .. } => 2,
            Self::Ws { .. } => 3,
            Self::Ts { .. } => 8,
            Self::Unknown { code, .. } => *code,
        }
    }

    pub fn length(&self) -> u8 {
        match self {
            Self::End | Self::Nop => 1,
            Self::Mss { .. } => 4,
            Self::Ws { .. } => 3,
            Self::Ts { .. } => 10,
            Self::Unknown { length, .. } => *length,
        }
    }

    pub fn write_bytes(&self, buf: &mut [u8]) {
        let mut off = 0;
        self.write_into(&mut off, buf);
    }

    fn write_into(&self, off: &mut usize, buf: &mut [u8]) {
        match self {
            Self::End => write_u8(buf, off, 0),
            Self::Nop => write_u8(buf, off, 1),
            Self::Mss { max_segment_size } => {
                write_u8(buf, off, 2);
                write_u8(buf, off, 4);
                write_u16(buf, off, *max_segment_size);
            }
            Self::Ws { window_scale } => {
                write_u8(buf, off, 3);
                write_u8(buf, off, 3);
                write_u8(buf, off, *window_scale);
            }
            Self::Ts {
                sender_time_stamp,
                echo_time_stamp,
            } => {
                write_u8(buf, off, 8);
                write_u8(buf, off, 10);
                write_u32(buf, off, *sender_time_stamp);
                write_u32(buf, off, *echo_time_stamp);
            }
            Self::Unknown {
                code,
                length,
                value,
            } => {
                write_u8(buf, off, *code);
                write_u8(buf, off, *length);
                write_bytes(buf, off, value);
            }
        }
    }
}

// ===========================================================================
// TCP_Packet
// ===========================================================================

/// Mirrors `TCP_Packet` from `DEV9/PacketReader/IP/TCP/TCP_Packet.{h,cpp}`.
#[derive(Clone, Debug)]
pub struct TcpPacket {
    pub source_port: u16,
    pub destination_port: u16,
    pub sequence_number: u32,
    pub acknowledgement_number: u32,
    /// Data offset (4 bits) | reserved (3 bits) | NS (1 bit).
    pub data_offset_and_ns_flag: u8,
    pub header_length: usize,
    pub flags: u8,
    pub window_size: u16,
    pub checksum: u16,
    pub urgent_pointer: u16,
    pub options: Vec<TcpOption>,
    pub payload: Payload,
}

impl Default for TcpPacket {
    fn default() -> Self {
        Self {
            source_port: 0,
            destination_port: 0,
            sequence_number: 0,
            acknowledgement_number: 0,
            data_offset_and_ns_flag: 0x50,
            header_length: 20,
            flags: 0,
            window_size: 0,
            checksum: 0,
            urgent_pointer: 0,
            options: Vec::new(),
            payload: Payload::new(0),
        }
    }
}

impl TcpPacket {
    pub const DEFAULT_HEADER_LENGTH: usize = 20;

    pub fn new(payload: Payload) -> Self {
        Self {
            payload,
            ..Self::default()
        }
    }

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
        if header_length > Self::DEFAULT_HEADER_LENGTH {
            let mut done = false;
            while !done && off < header_length {
                let op_kind = buf[off];
                let op_len = if off + 1 < buf.len() { buf[off + 1] } else { 0 };
                match op_kind {
                    0 => {
                        options.push(TcpOption::End);
                        done = true;
                    }
                    1 => {
                        options.push(TcpOption::Nop);
                        off += 1;
                    }
                    2 => {
                        let mut lo = off + 2;
                        let mss = read_u16(buf, &mut lo);
                        options.push(TcpOption::Mss { max_segment_size: mss });
                        off += op_len as usize;
                    }
                    3 => {
                        let mut lo = off + 2;
                        let ws = read_u8(buf, &mut lo);
                        options.push(TcpOption::Ws { window_scale: ws });
                        off += op_len as usize;
                    }
                    8 => {
                        let mut lo = off + 2;
                        let s = read_u32(buf, &mut lo);
                        let e = read_u32(buf, &mut lo);
                        options.push(TcpOption::Ts {
                            sender_time_stamp: s,
                            echo_time_stamp: e,
                        });
                        off += op_len as usize;
                    }
                    _ => {
                        let value_len = op_len.saturating_sub(2) as usize;
                        let value = if off + 2 + value_len <= buf.len() {
                            buf[off + 2..off + 2 + value_len].to_vec()
                        } else {
                            Vec::new()
                        };
                        options.push(TcpOption::Unknown {
                            code: op_kind,
                            length: op_len,
                            value,
                        });
                        off += op_len as usize;
                    }
                }
                if off == header_length {
                    done = true;
                }
            }
            off = header_length;
        } else {
            off = Self::DEFAULT_HEADER_LENGTH;
        }

        let payload = if off < buf.len() {
            Payload::from_slice(&buf[off..])
        } else {
            Payload::new(0)
        };

        Self {
            source_port,
            destination_port,
            sequence_number,
            acknowledgement_number,
            data_offset_and_ns_flag,
            header_length,
            flags,
            window_size,
            checksum,
            urgent_pointer,
            options,
            payload,
        }
    }

    fn recompute_header_len(&mut self) {
        let mut op_offset = Self::DEFAULT_HEADER_LENGTH;
        for opt in &self.options {
            op_offset += opt.length() as usize;
        }
        self.header_length = align_up_pow2(op_offset, 4);
        let ns = self.data_offset_and_ns_flag & 1;
        self.data_offset_and_ns_flag = ((self.header_length >> 2) as u8) << 4;
        self.data_offset_and_ns_flag |= ns;
    }

    // Flag accessors ------------------------------------------------------

    pub fn get_ns(&self) -> bool { (self.data_offset_and_ns_flag & 1) != 0 }
    pub fn set_ns(&mut self, v: bool) {
        self.data_offset_and_ns_flag = (self.data_offset_and_ns_flag & !0x1) | ((v as u8) & 1);
    }

    pub fn get_cwr(&self) -> bool { (self.flags & (1 << 7)) != 0 }
    pub fn set_cwr(&mut self, v: bool) {
        let bit = ((v as u8) & 1) << 7;
        self.flags = (self.flags & !(0x1 << 7)) | bit;
    }

    pub fn get_ece(&self) -> bool { (self.flags & (1 << 6)) != 0 }
    pub fn set_ece(&mut self, v: bool) {
        let bit = ((v as u8) & 1) << 6;
        self.flags = (self.flags & !(0x1 << 6)) | bit;
    }

    pub fn get_urg(&self) -> bool { (self.flags & (1 << 5)) != 0 }
    pub fn set_urg(&mut self, v: bool) {
        let bit = ((v as u8) & 1) << 5;
        self.flags = (self.flags & !(0x1 << 5)) | bit;
    }

    pub fn get_ack(&self) -> bool { (self.flags & (1 << 4)) != 0 }
    pub fn set_ack(&mut self, v: bool) {
        let bit = ((v as u8) & 1) << 4;
        self.flags = (self.flags & !(0x1 << 4)) | bit;
    }

    pub fn get_psh(&self) -> bool { (self.flags & (1 << 3)) != 0 }
    pub fn set_psh(&mut self, v: bool) {
        let bit = ((v as u8) & 1) << 3;
        self.flags = (self.flags & !(0x1 << 3)) | bit;
    }

    pub fn get_rst(&self) -> bool { (self.flags & (1 << 2)) != 0 }
    pub fn set_rst(&mut self, v: bool) {
        let bit = ((v as u8) & 1) << 2;
        self.flags = (self.flags & !(0x1 << 2)) | bit;
    }

    pub fn get_syn(&self) -> bool { (self.flags & (1 << 1)) != 0 }
    pub fn set_syn(&mut self, v: bool) {
        let bit = ((v as u8) & 1) << 1;
        self.flags = (self.flags & !(0x1 << 1)) | bit;
    }

    pub fn get_fin(&self) -> bool { (self.flags & 1) != 0 }
    pub fn set_fin(&mut self, v: bool) {
        self.flags = (self.flags & !0x1) | ((v as u8) & 1);
    }

    pub fn get_length(&mut self) -> usize {
        self.recompute_header_len();
        self.header_length + self.payload.len()
    }

    pub fn write_bytes(&mut self, buf: &mut [u8]) {
        let start = 0;
        let mut counter = start;
        write_u16(buf, &mut counter, self.source_port);
        write_u16(buf, &mut counter, self.destination_port);
        write_u32(buf, &mut counter, self.sequence_number);
        write_u32(buf, &mut counter, self.acknowledgement_number);
        write_u8(buf, &mut counter, self.data_offset_and_ns_flag);
        write_u8(buf, &mut counter, self.flags);
        write_u16(buf, &mut counter, self.window_size);
        write_u16(buf, &mut counter, self.checksum);
        write_u16(buf, &mut counter, self.urgent_pointer);
        for opt in &self.options {
            opt.write_into(&mut counter, buf);
        }
        if counter != start + self.header_length {
            for b in &mut buf[counter..start + self.header_length] {
                *b = 0;
            }
            counter = start + self.header_length;
        }
        let p = self.payload.data();
        buf[counter..counter + p.len()].copy_from_slice(p);
        counter += p.len();
        let _ = counter;
    }

    pub fn clone_packet(&self) -> Self {
        self.clone()
    }

    pub fn get_protocol(&self) -> IpType {
        IpType::TCP
    }

    fn pseudo_header(&self, src: IpAddress, dst: IpAddress, total_len: u16) -> [u8; 12] {
        let mut h = [0u8; 12];
        src.write_to(&mut h[0..4]);
        dst.write_to(&mut h[4..8]);
        h[9] = self.get_protocol().to_u8();
        h[10..12].copy_from_slice(&total_len.to_be_bytes());
        h
    }

    pub fn calculate_checksum(&mut self, src_ip: IpAddress, dst_ip: IpAddress) {
        self.recompute_header_len();
        let p_header_len = 12 + self.header_length + self.payload.len();
        let mut p_header_len = p_header_len;
        if p_header_len & 1 != 0 {
            p_header_len += 1;
        }
        let mut segment = vec![0u8; p_header_len];
        let mut counter = 0;
        let total_len = self.get_length() as u16;
        let pseudo = self.pseudo_header(src_ip, dst_ip, total_len);
        segment[0..12].copy_from_slice(&pseudo);
        counter += 12;
        self.checksum = 0;
        self.write_bytes(&mut segment[counter..]);
        counter += self.header_length + self.payload.len();
        if counter < p_header_len {
            segment[counter] = 0;
        }
        self.checksum = internet_checksum(&segment[..p_header_len]);
    }

    pub fn verify_checksum(&mut self, src_ip: IpAddress, dst_ip: IpAddress) -> bool {
        self.recompute_header_len();
        let p_header_len = 12 + self.header_length + self.payload.len();
        let mut p_header_len = p_header_len;
        if p_header_len & 1 != 0 {
            p_header_len += 1;
        }
        let mut segment = vec![0u8; p_header_len];
        let mut counter = 0;
        let total_len = self.get_length() as u16;
        let pseudo = self.pseudo_header(src_ip, dst_ip, total_len);
        segment[0..12].copy_from_slice(&pseudo);
        counter += 12;
        self.write_bytes(&mut segment[counter..]);
        counter += self.header_length + self.payload.len();
        if counter < p_header_len {
            segment[counter] = 0;
        }
        internet_checksum(&segment[..p_header_len]) == 0
    }
}

// ===========================================================================
// UDP_Packet
// ===========================================================================

/// Mirrors `UDP_Packet`. The 16-bit `length` field is recomputed on the wire
/// from `get_length()` (matches the C++).
#[derive(Clone, Debug)]
pub struct UdpPacket {
    pub source_port: u16,
    pub destination_port: u16,
    pub checksum: u16,
    pub payload: Payload,
}

impl UdpPacket {
    pub const HEADER_LENGTH: usize = 8;

    pub fn new(payload: Payload) -> Self {
        Self {
            source_port: 0,
            destination_port: 0,
            checksum: 0,
            payload,
        }
    }

    pub fn from_bytes(buf: &[u8]) -> Self {
        let mut off = 0;
        let source_port = read_u16(buf, &mut off);
        let destination_port = read_u16(buf, &mut off);
        let length = read_u16(buf, &mut off) as usize;
        let checksum = read_u16(buf, &mut off);
        let pl_len = length.saturating_sub(off).min(buf.len().saturating_sub(off));
        let payload = Payload::from_slice(&buf[off..off + pl_len]);
        Self {
            source_port,
            destination_port,
            checksum,
            payload,
        }
    }

    pub fn get_length(&self) -> usize {
        Self::HEADER_LENGTH + self.payload.len()
    }

    pub fn write_bytes(&self, buf: &mut [u8]) {
        let mut off = 0;
        write_u16(buf, &mut off, self.source_port);
        write_u16(buf, &mut off, self.destination_port);
        write_u16(buf, &mut off, self.get_length() as u16);
        write_u16(buf, &mut off, self.checksum);
        let p = self.payload.data();
        buf[off..off + p.len()].copy_from_slice(p);
        off += p.len();
        let _ = off;
    }

    pub fn clone_packet(&self) -> Self {
        self.clone()
    }

    pub fn get_protocol(&self) -> IpType {
        IpType::UDP
    }

    fn pseudo_header(&self, src: IpAddress, dst: IpAddress, total_len: u16) -> [u8; 12] {
        let mut h = [0u8; 12];
        src.write_to(&mut h[0..4]);
        dst.write_to(&mut h[4..8]);
        h[9] = self.get_protocol().to_u8();
        h[10..12].copy_from_slice(&total_len.to_be_bytes());
        h
    }

    pub fn calculate_checksum(&mut self, src_ip: IpAddress, dst_ip: IpAddress) {
        let p_header_len = 12 + Self::HEADER_LENGTH + self.payload.len();
        let mut p_header_len = p_header_len;
        if p_header_len & 1 != 0 {
            p_header_len += 1;
        }
        let mut segment = vec![0u8; p_header_len];
        let pseudo = self.pseudo_header(src_ip, dst_ip, self.get_length() as u16);
        segment[0..12].copy_from_slice(&pseudo);
        let mut counter = 12;
        self.checksum = 0;
        self.write_bytes(&mut segment[counter..]);
        counter += Self::HEADER_LENGTH + self.payload.len();
        if counter < p_header_len {
            segment[counter] = 0;
        }
        self.checksum = internet_checksum(&segment[..p_header_len]);
    }

    pub fn verify_checksum(&mut self, src_ip: IpAddress, dst_ip: IpAddress) -> bool {
        let p_header_len = 12 + Self::HEADER_LENGTH + self.payload.len();
        let mut p_header_len = p_header_len;
        if p_header_len & 1 != 0 {
            p_header_len += 1;
        }
        let mut segment = vec![0u8; p_header_len];
        let pseudo = self.pseudo_header(src_ip, dst_ip, self.get_length() as u16);
        segment[0..12].copy_from_slice(&pseudo);
        let mut counter = 12;
        self.write_bytes(&mut segment[counter..]);
        counter += Self::HEADER_LENGTH + self.payload.len();
        if counter < p_header_len {
            segment[counter] = 0;
        }
        internet_checksum(&segment[..p_header_len]) == 0
    }
}

// ===========================================================================
// DNS enums + classes
// ===========================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum DnsOpCode {
    Query = 0,
    IQuery = 1,
    Status = 2,
    Reserved = 3,
    Notify = 4,
    Update = 5,
}

impl DnsOpCode {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Query,
            1 => Self::IQuery,
            2 => Self::Status,
            3 => Self::Reserved,
            4 => Self::Notify,
            5 => Self::Update,
            _ => Self::Reserved,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum DnsRCode {
    NoError = 0,
    FormatError = 1,
    ServerFailure = 2,
    NameError = 3,
    NotImplemented = 4,
    Refused = 5,
    YXDomain = 6,
    YXRRSet = 7,
    NXRRSet = 8,
    NotAuth = 9,
    NotZone = 10,
}

impl DnsRCode {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::NoError,
            1 => Self::FormatError,
            2 => Self::ServerFailure,
            3 => Self::NameError,
            4 => Self::NotImplemented,
            5 => Self::Refused,
            6 => Self::YXDomain,
            7 => Self::YXRRSet,
            8 => Self::NXRRSet,
            9 => Self::NotAuth,
            10 => Self::NotZone,
            _ => Self::NoError,
        }
    }
}

/// One question entry, mirrored from `DNS_QuestionEntry`.
#[derive(Clone, Debug, Default)]
pub struct DnsQuestionEntry {
    pub name: String,
    pub entry_type: u16,
    pub entry_class: u16,
}

impl DnsQuestionEntry {
    pub fn new(name: String, q_type: u16, q_class: u16) -> Self {
        Self {
            name,
            entry_type: q_type,
            entry_class: q_class,
        }
    }

    pub fn from_bytes(buf: &[u8]) -> Self {
        let mut off = 0;
        Self::parse(buf, &mut off)
    }

    fn parse(buf: &[u8], off: &mut usize) -> Self {
        let name = read_dns_string(buf, off);
        let entry_type = read_u16(buf, off);
        let entry_class = read_u16(buf, off);
        Self {
            name,
            entry_type,
            entry_class,
        }
    }

    pub fn get_length(&self) -> usize {
        1 + self.name.len() + 1 + 4
    }

    pub fn write_bytes(&self, buf: &mut [u8]) {
        let mut off = 0;
        write_dns_string(buf, &mut off, &self.name);
        write_u16(buf, &mut off, self.entry_type);
        write_u16(buf, &mut off, self.entry_class);
    }
}

/// One resource record, mirrored from `DNS_ResponseEntry`.
#[derive(Clone, Debug, Default)]
pub struct DnsResponseEntry {
    pub name: String,
    pub entry_type: u16,
    pub entry_class: u16,
    pub time_to_live: u32,
    pub data: Vec<u8>,
}

impl DnsResponseEntry {
    pub fn new(name: String, r_type: u16, r_class: u16, r_data: Vec<u8>, r_ttl: u32) -> Self {
        Self {
            name,
            entry_type: r_type,
            entry_class: r_class,
            time_to_live: r_ttl,
            data: r_data,
        }
    }

    pub fn from_bytes(buf: &[u8]) -> Self {
        let mut off = 0;
        let q = DnsQuestionEntry::parse(buf, &mut off);
        let time_to_live = read_u32(buf, &mut off);
        let data_len = read_u16(buf, &mut off) as usize;
        let data = if off + data_len <= buf.len() {
            buf[off..off + data_len].to_vec()
        } else {
            Vec::new()
        };
        off += data_len;
        Self {
            name: q.name,
            entry_type: q.entry_type,
            entry_class: q.entry_class,
            time_to_live,
            data,
        }
    }

    pub fn get_length(&self) -> usize {
        // Mirrors DNS_QuestionEntry::GetLength() + 4 (TTL) + 2 (data len) + data len
        1 + self.name.len() + 1 + 4 + 2 + 4 + self.data.len()
    }

    pub fn write_bytes(&self, buf: &mut [u8]) {
        let mut off = 0;
        write_dns_string(buf, &mut off, &self.name);
        write_u16(buf, &mut off, self.entry_type);
        write_u16(buf, &mut off, self.entry_class);
        write_u32(buf, &mut off, self.time_to_live);
        write_u16(buf, &mut off, self.data.len() as u16);
        buf[off..off + self.data.len()].copy_from_slice(&self.data);
    }
}

fn read_dns_string(buf: &[u8], off: &mut usize) -> String {
    let mut out = String::new();
    while *off < buf.len() {
        let len = buf[*off];
        if len == 0 {
            *off += 1;
            break;
        }
        if len >= 192 {
            // 14-bit pointer relative to the start of the packet.
            if *off + 1 >= buf.len() {
                break;
            }
            let lo = buf[*off + 1] & 0x3F;
            let mut target = (((len & 0x3F) as usize) << 8) | lo as usize;
            out.push_str(&read_dns_string(buf, &mut target));
            *off += 2;
            // Pointers terminate the label sequence.
            return out;
        } else {
            *off += 1;
            if *off + (len as usize) > buf.len() {
                break;
            }
            let part = std::str::from_utf8(&buf[*off..*off + len as usize]).unwrap_or("");
            if !out.is_empty() {
                out.push('.');
            }
            out.push_str(part);
            *off += len as usize;
        }
    }
    out
}

fn write_dns_string(buf: &mut [u8], off: &mut usize, value: &str) {
    let bytes = value.as_bytes();
    let mut segment_start = 0usize;
    let mut segment_len = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'.' {
            if segment_len == 0 {
                continue;
            }
            write_u8(buf, off, segment_len as u8);
            buf[*off..*off + segment_len].copy_from_slice(&bytes[segment_start..segment_start + segment_len]);
            *off += segment_len;
            segment_len = 0;
            segment_start = i + 1;
        } else {
            segment_len += 1;
        }
    }
    if segment_len != 0 {
        write_u8(buf, off, segment_len as u8);
        buf[*off..*off + segment_len].copy_from_slice(&bytes[segment_start..segment_start + segment_len]);
        *off += segment_len;
    }
    write_u8(buf, off, 0);
}

/// Mirrors `DNS_Packet` from `DEV9/PacketReader/IP/UDP/DNS/DNS_Packet.{h,cpp}`.
#[derive(Clone, Debug, Default)]
pub struct DnsPacket {
    pub id: u16,
    pub questions: Vec<DnsQuestionEntry>,
    pub answers: Vec<DnsResponseEntry>,
    pub authorities: Vec<DnsResponseEntry>,
    pub additional: Vec<DnsResponseEntry>,
    flags1: u8,
    flags2: u8,
}

impl DnsPacket {
    pub fn new() -> Self {
        Self::default()
    }

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
            questions.push(DnsQuestionEntry::parse(buf, &mut off));
        }
        let mut answers = Vec::with_capacity(a_count as usize);
        for _ in 0..a_count {
            answers.push(DnsResponseEntry::from_bytes_at(buf, &mut off));
        }
        let mut authorities = Vec::with_capacity(au_count as usize);
        for _ in 0..au_count {
            authorities.push(DnsResponseEntry::from_bytes_at(buf, &mut off));
        }
        let mut additional = Vec::with_capacity(ad_count as usize);
        for _ in 0..ad_count {
            additional.push(DnsResponseEntry::from_bytes_at(buf, &mut off));
        }

        Self {
            id,
            questions,
            answers,
            authorities,
            additional,
            flags1,
            flags2,
        }
    }

    pub fn get_length(&self) -> usize {
        let mut length = 2 * 2 + 4 * 2;
        for q in &self.questions {
            length += q.get_length();
        }
        for r in &self.answers {
            length += r.get_length();
        }
        for r in &self.authorities {
            length += r.get_length();
        }
        for r in &self.additional {
            length += r.get_length();
        }
        length
    }

    pub fn write_bytes(&self, buf: &mut [u8]) {
        let mut off = 0;
        write_u16(buf, &mut off, self.id);
        write_u8(buf, &mut off, self.flags1);
        write_u8(buf, &mut off, self.flags2);
        write_u16(buf, &mut off, self.questions.len() as u16);
        write_u16(buf, &mut off, self.answers.len() as u16);
        write_u16(buf, &mut off, self.authorities.len() as u16);
        write_u16(buf, &mut off, self.additional.len() as u16);
        for q in &self.questions {
            q.write_bytes(&mut buf[off..]);
            off += q.get_length();
        }
        for r in &self.answers {
            r.write_bytes(&mut buf[off..]);
            off += r.get_length();
        }
        for r in &self.authorities {
            r.write_bytes(&mut buf[off..]);
            off += r.get_length();
        }
        for r in &self.additional {
            r.write_bytes(&mut buf[off..]);
            off += r.get_length();
        }
    }

    pub fn clone_packet(&self) -> Self {
        self.clone()
    }

    // Flag accessors ------------------------------------------------------

    pub fn get_qr(&self) -> bool { (self.flags1 & (1 << 7)) != 0 }
    pub fn set_qr(&mut self, v: bool) {
        let bit = ((v as u8) & 1) << 7;
        self.flags1 = (self.flags1 & !(0x1 << 7)) | bit;
    }

    pub fn get_op_code(&self) -> u8 { (self.flags1 >> 3) & 0xF }
    pub fn set_op_code(&mut self, v: u8) {
        self.flags1 = (self.flags1 & !(0xF << 3)) | ((v & 0xF) << 3);
    }

    pub fn get_aa(&self) -> bool { (self.flags1 & (1 << 2)) != 0 }
    pub fn set_aa(&mut self, v: bool) {
        let bit = ((v as u8) & 1) << 2;
        self.flags1 = (self.flags1 & !(0x1 << 2)) | bit;
    }

    pub fn get_tc(&self) -> bool { (self.flags1 & (1 << 1)) != 0 }
    pub fn set_tc(&mut self, v: bool) {
        let bit = ((v as u8) & 1) << 1;
        self.flags1 = (self.flags1 & !(0x1 << 1)) | bit;
    }

    pub fn get_rd(&self) -> bool { (self.flags1 & 1) != 0 }
    pub fn set_rd(&mut self, v: bool) {
        self.flags1 = (self.flags1 & !0x1) | ((v as u8) & 1);
    }

    pub fn get_ra(&self) -> bool { (self.flags2 & (1 << 7)) != 0 }
    pub fn set_ra(&mut self, v: bool) {
        let bit = ((v as u8) & 1) << 7;
        self.flags2 = (self.flags2 & !(0x1 << 7)) | bit;
    }

    pub fn get_z0(&self) -> u8 { (self.flags2 & (1 << 6)) as u8 }
    pub fn set_z0(&mut self, v: u8) {
        let bit = (v & 1) << 6;
        self.flags2 = (self.flags2 & !(0x1 << 6)) | bit;
    }

    pub fn get_ad(&self) -> bool { (self.flags2 & (1 << 5)) != 0 }
    pub fn set_ad(&mut self, v: bool) {
        let bit = ((v as u8) & 1) << 5;
        self.flags2 = (self.flags2 & !(0x1 << 5)) | bit;
    }

    pub fn get_cd(&self) -> bool { (self.flags2 & (1 << 4)) != 0 }
    pub fn set_cd(&mut self, v: bool) {
        let bit = ((v as u8) & 1) << 4;
        self.flags2 = (self.flags2 & !(0x1 << 4)) | bit;
    }

    pub fn get_r_code(&self) -> u8 { self.flags2 & 0xF }
    pub fn set_r_code(&mut self, v: u8) {
        self.flags2 = (self.flags2 & !0xF) | (v & 0xF);
    }
}

impl DnsResponseEntry {
    fn from_bytes_at(buf: &[u8], off: &mut usize) -> Self {
        let q = DnsQuestionEntry::parse(buf, off);
        let time_to_live = read_u32(buf, off);
        let data_len = read_u16(buf, off) as usize;
        let data = if *off + data_len <= buf.len() {
            buf[*off..*off + data_len].to_vec()
        } else {
            Vec::new()
        };
        *off += data_len;
        Self {
            name: q.name,
            entry_type: q.entry_type,
            entry_class: q.entry_class,
            time_to_live,
            data,
        }
    }
}

// ===========================================================================
// DHCP options
// ===========================================================================

#[derive(Clone, Debug)]
pub enum DhcpOption {
    End,
    Nop,
    Subnet { mask: IpAddress },
    Router { routers: Vec<IpAddress> },
    Dns { servers: Vec<IpAddress> },
    HostName { host_name: String },
    DnsName { domain_name: String },
    Bcip { broadcast_ip: IpAddress },
    NbiosType { r#type: u8 },
    ReqIp { requested_ip: IpAddress },
    IpLt { ip_lease_time: u32 },
    Msg { message: u8 },
    ServIp { server_ip: IpAddress },
    ReqList { requests: Vec<u8> },
    MsgStr { message: String },
    Mmsgs { max_message_size: u16 },
    T1 { ip_renewal_time_t1: u32 },
    T2 { ip_rebinding_time_t2: u32 },
    ClassId { class_id: String },
    ClientId { client_id: Vec<u8> },
}

impl DhcpOption {
    pub fn code(&self) -> u8 {
        match self {
            Self::End => 255,
            Self::Nop => 0,
            Self::Subnet { .. } => 1,
            Self::Router { .. } => 3,
            Self::Dns { .. } => 6,
            Self::HostName { .. } => 12,
            Self::DnsName { .. } => 15,
            Self::Bcip { .. } => 28,
            Self::NbiosType { .. } => 46,
            Self::ReqIp { .. } => 50,
            Self::IpLt { .. } => 51,
            Self::Msg { .. } => 53,
            Self::ServIp { .. } => 54,
            Self::ReqList { .. } => 55,
            Self::MsgStr { .. } => 56,
            Self::Mmsgs { .. } => 57,
            Self::T1 { .. } => 58,
            Self::T2 { .. } => 59,
            Self::ClassId { .. } => 60,
            Self::ClientId { .. } => 61,
        }
    }

    pub fn length(&self) -> usize {
        // 1 byte for code + 1 byte for length + value bytes.
        // Note: `GetLength()` in the C++ source INCLUDES the code+len header
        // (see DHCP_Options.h), so this matches.
        let body = match self {
            Self::End | Self::Nop => 0,
            Self::Subnet { .. } | Self::Bcip { .. } | Self::ReqIp { .. } | Self::ServIp { .. }
            | Self::IpLt { .. } | Self::Msg { .. } | Self::T1 { .. } | Self::T2 { .. } => 4,
            Self::Msg { .. } => 1,
            Self::Mmsgs { .. } => 2,
            Self::NbiosType { .. } => 1,
            Self::HostName { host_name } => host_name.len(),
            Self::DnsName { domain_name } => domain_name.len(),
            Self::ClassId { class_id } => class_id.len(),
            Self::MsgStr { message } => message.len(),
            Self::ReqList { requests } => requests.len(),
            Self::Router { routers } => 4 * routers.len(),
            Self::Dns { servers } => 4 * servers.len(),
            Self::ClientId { client_id } => client_id.len(),
        };
        2 + body
    }

    pub fn write_bytes(&self, buf: &mut [u8]) {
        let mut off = 0;
        self.write_into(&mut off, buf);
    }

    fn write_into(&self, off: &mut usize, buf: &mut [u8]) {
        match self {
            Self::End => {
                write_u8(buf, off, 255);
            }
            Self::Nop => {
                write_u8(buf, off, 0);
            }
            Self::Subnet { mask } => {
                write_u8(buf, off, 1);
                write_u8(buf, off, 4);
                mask.write_to(&mut buf[*off..*off + 4]);
                *off += 4;
            }
            Self::Router { routers } => {
                write_u8(buf, off, 3);
                write_u8(buf, off, (4 * routers.len()) as u8);
                for r in routers {
                    r.write_to(&mut buf[*off..*off + 4]);
                    *off += 4;
                }
            }
            Self::Dns { servers } => {
                write_u8(buf, off, 6);
                write_u8(buf, off, (4 * servers.len()) as u8);
                for s in servers {
                    s.write_to(&mut buf[*off..*off + 4]);
                    *off += 4;
                }
            }
            Self::HostName { host_name } => {
                write_u8(buf, off, 12);
                write_u8(buf, off, host_name.len() as u8);
                buf[*off..*off + host_name.len()].copy_from_slice(host_name.as_bytes());
                *off += host_name.len();
            }
            Self::DnsName { domain_name } => {
                write_u8(buf, off, 15);
                write_u8(buf, off, domain_name.len() as u8);
                buf[*off..*off + domain_name.len()].copy_from_slice(domain_name.as_bytes());
                *off += domain_name.len();
            }
            Self::Bcip { broadcast_ip } => {
                write_u8(buf, off, 28);
                write_u8(buf, off, 4);
                broadcast_ip.write_to(&mut buf[*off..*off + 4]);
                *off += 4;
            }
            Self::NbiosType { r#type } => {
                write_u8(buf, off, 46);
                write_u8(buf, off, 1);
                write_u8(buf, off, *r#type);
            }
            Self::ReqIp { requested_ip } => {
                write_u8(buf, off, 50);
                write_u8(buf, off, 4);
                requested_ip.write_to(&mut buf[*off..*off + 4]);
                *off += 4;
            }
            Self::IpLt { ip_lease_time } => {
                write_u8(buf, off, 51);
                write_u8(buf, off, 4);
                write_u32(buf, off, *ip_lease_time);
            }
            Self::Msg { message } => {
                write_u8(buf, off, 53);
                write_u8(buf, off, 1);
                write_u8(buf, off, *message);
            }
            Self::ServIp { server_ip } => {
                write_u8(buf, off, 54);
                write_u8(buf, off, 4);
                server_ip.write_to(&mut buf[*off..*off + 4]);
                *off += 4;
            }
            Self::ReqList { requests } => {
                write_u8(buf, off, 55);
                write_u8(buf, off, requests.len() as u8);
                buf[*off..*off + requests.len()].copy_from_slice(requests);
                *off += requests.len();
            }
            Self::MsgStr { message } => {
                write_u8(buf, off, 56);
                write_u8(buf, off, message.len() as u8);
                buf[*off..*off + message.len()].copy_from_slice(message.as_bytes());
                *off += message.len();
            }
            Self::Mmsgs { max_message_size } => {
                write_u8(buf, off, 57);
                write_u8(buf, off, 2);
                write_u16(buf, off, *max_message_size);
            }
            Self::T1 { ip_renewal_time_t1 } => {
                write_u8(buf, off, 58);
                write_u8(buf, off, 4);
                write_u32(buf, off, *ip_renewal_time_t1);
            }
            Self::T2 { ip_rebinding_time_t2 } => {
                write_u8(buf, off, 59);
                write_u8(buf, off, 4);
                write_u32(buf, off, *ip_rebinding_time_t2);
            }
            Self::ClassId { class_id } => {
                write_u8(buf, off, 60);
                write_u8(buf, off, class_id.len() as u8);
                buf[*off..*off + class_id.len()].copy_from_slice(class_id.as_bytes());
                *off += class_id.len();
            }
            Self::ClientId { client_id } => {
                write_u8(buf, off, 61);
                write_u8(buf, off, client_id.len() as u8);
                buf[*off..*off + client_id.len()].copy_from_slice(client_id);
                *off += client_id.len();
            }
        }
    }
}

// ===========================================================================
// DHCP_Packet
// ===========================================================================

/// Mirrors `DHCP_Packet` from `DEV9/PacketReader/IP/UDP/DHCP/DHCP_Packet.{h,cpp}`.
#[derive(Clone, Debug)]
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
    pub options: Vec<DhcpOption>,
    /// Container size used by `get_length` and `write_bytes` (default 576).
    pub max_length: usize,
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
            client_ip: IpAddress::ZERO,
            your_ip: IpAddress::ZERO,
            server_ip: IpAddress::ZERO,
            gateway_ip: IpAddress::ZERO,
            client_hardware_address: [0; 16],
            magic_cookie: 0,
            options: Vec::new(),
            max_length: 576,
        }
    }
}

impl DhcpPacket {
    /// 240 = 236 (BOOTP legacy) + 4 (magic cookie). Mirrors the C++ constant.
    const FIXED_HEADER: usize = 240;

    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_bytes(buf: &[u8]) -> Self {
        let mut off = 0;
        let op = read_u8(buf, &mut off);
        let hardware_type = read_u8(buf, &mut off);
        let hardware_address_length = read_u8(buf, &mut off);
        let hops = read_u8(buf, &mut off);
        let transaction_id = read_u32(buf, &mut off);
        let seconds = read_u16(buf, &mut off);
        let flags = read_u16(buf, &mut off);
        let client_ip = IpAddress::from_bytes(&buf[off..off + 4]);
        off += 4;
        let your_ip = IpAddress::from_bytes(&buf[off..off + 4]);
        off += 4;
        let server_ip = IpAddress::from_bytes(&buf[off..off + 4]);
        off += 4;
        let gateway_ip = IpAddress::from_bytes(&buf[off..off + 4]);
        off += 4;
        let mut client_hardware_address = [0u8; 16];
        client_hardware_address.copy_from_slice(&buf[off..off + 16]);
        off += 16;
        // Skip the 192 bytes of BOOTP legacy.
        off += 192;
        let magic_cookie = read_u32(buf, &mut off);

        let mut options = Vec::new();
        let mut done = false;
        while !done {
            if off >= buf.len() {
                break;
            }
            let op_kind = buf[off];
            if op_kind == 255 {
                options.push(DhcpOption::End);
                done = true;
                off += 1;
                continue;
            }
            if off + 1 >= buf.len() {
                options.push(DhcpOption::End);
                done = true;
                continue;
            }
            let op_len = buf[off + 1];
            // The `+ 2` matches the C++ advance after the switch.
            let advance = op_len as usize + 2;
            let opt = match op_kind {
                0 => {
                    off += 1;
                    continue;
                }
                1 => {
                    let mut lo = off + 2;
                    let mask = IpAddress::from_bytes(&buf[lo..lo + 4]);
                    lo += 4;
                    Some(DhcpOption::Subnet { mask })
                }
                3 => {
                    let mut lo = off + 1;
                    let len = read_u8(buf, &mut lo) as usize;
                    let end = (lo + len).min(buf.len());
                    let mut routers = Vec::new();
                    let mut p = lo;
                    while p + 4 <= end {
                        routers.push(IpAddress::from_bytes(&buf[p..p + 4]));
                        p += 4;
                    }
                    Some(DhcpOption::Router { routers })
                }
                6 => {
                    let mut lo = off + 1;
                    let len = read_u8(buf, &mut lo) as usize;
                    let end = (lo + len).min(buf.len());
                    let mut servers = Vec::new();
                    let mut p = lo;
                    while p + 4 <= end {
                        servers.push(IpAddress::from_bytes(&buf[p..p + 4]));
                        p += 4;
                    }
                    Some(DhcpOption::Dns { servers })
                }
                12 => {
                    let mut lo = off + 1;
                    let len = read_u8(buf, &mut lo) as usize;
                    let end = (lo + len).min(buf.len());
                    let s = std::str::from_utf8(&buf[lo..end]).unwrap_or("").to_string();
                    Some(DhcpOption::HostName { host_name: s })
                }
                15 => {
                    let mut lo = off + 1;
                    let len = read_u8(buf, &mut lo) as usize;
                    let end = (lo + len).min(buf.len());
                    let s = std::str::from_utf8(&buf[lo..end]).unwrap_or("").to_string();
                    Some(DhcpOption::DnsName { domain_name: s })
                }
                28 => {
                    let mut lo = off + 2;
                    let ip = IpAddress::from_bytes(&buf[lo..lo + 4]);
                    lo += 4;
                    Some(DhcpOption::Bcip { broadcast_ip: ip })
                }
                46 => {
                    let mut lo = off + 2;
                    let t = read_u8(buf, &mut lo);
                    Some(DhcpOption::NbiosType { r#type: t })
                }
                50 => {
                    let mut lo = off + 2;
                    let ip = IpAddress::from_bytes(&buf[lo..lo + 4]);
                    lo += 4;
                    Some(DhcpOption::ReqIp { requested_ip: ip })
                }
                51 => {
                    let mut lo = off + 2;
                    let t = read_u32(buf, &mut lo);
                    Some(DhcpOption::IpLt { ip_lease_time: t })
                }
                53 => {
                    let mut lo = off + 2;
                    let m = read_u8(buf, &mut lo);
                    Some(DhcpOption::Msg { message: m })
                }
                54 => {
                    let mut lo = off + 2;
                    let ip = IpAddress::from_bytes(&buf[lo..lo + 4]);
                    lo += 4;
                    Some(DhcpOption::ServIp { server_ip: ip })
                }
                55 => {
                    let mut lo = off + 1;
                    let len = read_u8(buf, &mut lo) as usize;
                    let end = (lo + len).min(buf.len());
                    Some(DhcpOption::ReqList { requests: buf[lo..end].to_vec() })
                }
                56 => {
                    let mut lo = off + 1;
                    let len = read_u8(buf, &mut lo) as usize;
                    let end = (lo + len).min(buf.len());
                    let s = std::str::from_utf8(&buf[lo..end]).unwrap_or("").to_string();
                    Some(DhcpOption::MsgStr { message: s })
                }
                57 => {
                    let mut lo = off + 2;
                    let v = read_u16(buf, &mut lo);
                    Some(DhcpOption::Mmsgs { max_message_size: v })
                }
                58 => {
                    let mut lo = off + 2;
                    let v = read_u32(buf, &mut lo);
                    Some(DhcpOption::T1 { ip_renewal_time_t1: v })
                }
                59 => {
                    let mut lo = off + 2;
                    let v = read_u32(buf, &mut lo);
                    Some(DhcpOption::T2 { ip_rebinding_time_t2: v })
                }
                60 => {
                    let mut lo = off + 1;
                    let len = read_u8(buf, &mut lo) as usize;
                    let end = (lo + len).min(buf.len());
                    let s = std::str::from_utf8(&buf[lo..end]).unwrap_or("").to_string();
                    Some(DhcpOption::ClassId { class_id: s })
                }
                61 => {
                    let mut lo = off + 1;
                    let len = read_u8(buf, &mut lo) as usize;
                    let end = (lo + len).min(buf.len());
                    Some(DhcpOption::ClientId { client_id: buf[lo..end].to_vec() })
                }
                _ => None,
            };
            if let Some(o) = opt {
                options.push(o);
            }
            off += advance;
            if off >= buf.len() {
                options.push(DhcpOption::Nop);
                done = true;
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
            options,
            max_length: 576,
        }
    }

    pub fn get_length(&self) -> usize {
        self.max_length - (8 + 20)
    }

    pub fn write_bytes(&self, buf: &mut [u8]) {
        let start = 0;
        let mut counter = start;
        write_u8(buf, &mut counter, self.op);
        write_u8(buf, &mut counter, self.hardware_type);
        write_u8(buf, &mut counter, self.hardware_address_length);
        write_u8(buf, &mut counter, self.hops);
        write_u32(buf, &mut counter, self.transaction_id);
        write_u16(buf, &mut counter, self.seconds);
        write_u16(buf, &mut counter, self.flags);
        self.client_ip.write_to(&mut buf[counter..counter + 4]);
        counter += 4;
        self.your_ip.write_to(&mut buf[counter..counter + 4]);
        counter += 4;
        self.server_ip.write_to(&mut buf[counter..counter + 4]);
        counter += 4;
        self.gateway_ip.write_to(&mut buf[counter..counter + 4]);
        counter += 4;
        buf[counter..counter + 16].copy_from_slice(&self.client_hardware_address);
        counter += 16;
        // 192 bytes of BOOTP legacy (zeroed out).
        for b in &mut buf[counter..counter + 192] {
            *b = 0;
        }
        counter += 192;
        write_u32(buf, &mut counter, self.magic_cookie);

        let mut len = Self::FIXED_HEADER;
        let mut emitted: Vec<DhcpOption> = Vec::new();
        for opt in &self.options {
            if len + opt.length() < self.max_length {
                len += opt.length();
                emitted.push(opt.clone());
            } else {
                // Pad out the rest of the container with END.
                emitted.push(DhcpOption::End);
                break;
            }
        }

        for opt in &emitted {
            opt.write_bytes(&mut buf[counter..]);
            counter += opt.length();
        }

        let end = start + self.get_length();
        if counter < end {
            for b in &mut buf[counter..end] {
                *b = 0;
            }
            counter = end;
        }
    }

    pub fn clone_packet(&self) -> Self {
        self.clone()
    }
}
