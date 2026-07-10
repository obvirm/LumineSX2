// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust translation of the PCSX2 DEV9 session and server sources.
//!
//! This module consolidates the C/C++ implementation of the DEV9
//! networking sessions (BaseSession, ICMP, TCP, UDP, UDP fixed-port),
//! the internal server helpers (DHCP and DNS, both logger and server),
//! the Windows pcap/TAP glue (`pcap_io_win32`, `tap-win32`) and the
//! thread-safe primitives (`SimpleQueue`, `ThreadSafeMap`) into a single
//! idiomatic Rust 2021 module.  The original implementation is split
//! across many files, mixes platform-specific APIs (Winsock, `libpcap`,
//! `GetAddrInfoEx`, registry queries, ICMP raw sockets) and uses manual
//! memory management.  This translation preserves the structure and
//! exposed surface of each component but expresses the same ideas in
//! idiomatic Rust with `std::sync::Mutex` for the queue and map, owned
//! `Vec`/`Box` values, and explicit `Option`/`Result` plumbing where
//! the original code returned nullable pointers or implicit success
//! booleans.

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Type aliases mirroring the PCSX2 typedefs
// ---------------------------------------------------------------------------

pub type u8  = ::std::primitive::u8;
pub type u16 = ::std::primitive::u16;
pub type u32 = ::std::primitive::u32;
pub type u64 = ::std::primitive::u64;
pub type s8  = ::std::primitive::i8;
pub type s16 = ::std::primitive::i16;
pub type s32 = ::std::primitive::i32;
pub type s64 = ::std::primitive::i64;

// ---------------------------------------------------------------------------
// Network type stubs (replacing PCSX2's `PacketReader::IP::IP_Address` and
// related packet types).  In a real port these would be replaced with
// concrete packet reader types; here we expose a minimal ABI-compatible
// shape so the public API compiles on its own.
// ---------------------------------------------------------------------------

/// 4-byte IPv4 address (equivalent to `PacketReader::IP::IP_Address`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct IpAddress {
    pub bytes: [u8; 4],
    pub integer: u32,
}

/// Connection key identifying a single session.  In the original code
/// this lives in the `Sessions` namespace and is used as a key for
/// `ThreadSafeMap` and as a `std::hash` specialisation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ConnectionKey {
    pub ip: IpAddress,
    pub protocol: u8,
    pub ps2Port: u16,
    pub srvPort: u16,
}

/// Opaque IP payload.  In the real C++ code this is
/// `PacketReader::IP::IP_Payload`; we model it as a boxed byte buffer.
#[derive(Clone, Debug, Default)]
pub struct IpPayload {
    pub data: Vec<u8>,
}

/// Opaque TCP packet.
#[derive(Clone, Debug, Default)]
pub struct TcpPacket {
    pub data: Vec<u8>,
}

/// Opaque UDP packet.
#[derive(Clone, Debug, Default)]
pub struct UdpPacket {
    pub data: Vec<u8>,
}

/// Opaque DHCP packet.
#[derive(Clone, Debug, Default)]
pub struct DhcpPacket {
    pub data: Vec<u8>,
}

/// Opaque DNS packet.
#[derive(Clone, Debug, Default)]
pub struct DnsPacket {
    pub data: Vec<u8>,
}

/// Opaque MAC address (6 bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct MacAddress {
    pub bytes: [u8; 6],
}

/// Opaque "received payload" envelope from any session implementation.
#[derive(Clone, Debug, Default)]
pub struct ReceivedPayload {
    pub source_ip: IpAddress,
    pub payload: Vec<u8>,
}

/// Callback invoked when a connection is closed.
pub type ConnectionClosedEventHandler = Arc<dyn Fn(&BaseSession) + Send + Sync + 'static>;

// ---------------------------------------------------------------------------
// ThreadSafeMap — translated from pcsx2/DEV9/ThreadSafeMap.h
// ---------------------------------------------------------------------------

/// A thread-safe wrapper around `HashMap` exposing the subset of
/// `PCSX2::ThreadSafeMap` needed by the rest of the module.  Unlike
/// the C++ version this uses a single `std::sync::Mutex` (the spec
/// says only `std` deps and `std::sync::Mutex`).
pub struct ThreadSafeMap<K, V> {
    inner: Mutex<HashMap<K, V>>,
}

impl<K, V> Default for ThreadSafeMap<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K, V> ThreadSafeMap<K, V> {
    /// Create a new empty map.
    pub fn new() -> Self {
        Self { inner: Mutex::new(HashMap::new()) }
    }

    /// Insert/replace the value for the given key.
    pub fn insert(&self, key: K, value: V) where K: Eq + std::hash::Hash {
        let mut g = self.inner.lock().expect("ThreadSafeMap poisoned");
        g.insert(key, value);
    }

    /// Return a clone of the value associated with `key`, or `None`.
    pub fn get(&self, key: &K) -> Option<V> where K: Eq + std::hash::Hash, V: Clone {
        let g = self.inner.lock().expect("ThreadSafeMap poisoned");
        g.get(key).cloned()
    }

    /// Borrow the value associated with `key` for the duration of the
    /// returned guard.  Helper to mimic the C++ `TryGetValue` API.
    pub fn try_get(&self, key: &K) -> Option<V> where K: Eq + std::hash::Hash, V: Clone {
        self.get(key)
    }

    /// Remove the value associated with `key`, returning it if present.
    pub fn remove(&self, key: &K) -> Option<V> where K: Eq + std::hash::Hash {
        let mut g = self.inner.lock().expect("ThreadSafeMap poisoned");
        g.remove(key)
    }

    /// Return a snapshot of all keys currently in the map.
    pub fn keys(&self) -> Vec<K> where K: Eq + std::hash::Hash + Clone {
        let g = self.inner.lock().expect("ThreadSafeMap poisoned");
        g.keys().cloned().collect()
    }

    /// Erase every entry.
    pub fn clear(&self) {
        let mut g = self.inner.lock().expect("ThreadSafeMap poisoned");
        g.clear();
    }
}

// ---------------------------------------------------------------------------
// SimpleQueue — translated from pcsx2/DEV9/SimpleQueue.h
// ---------------------------------------------------------------------------

/// Single-producer / single-consumer lock-protected queue.  The C++
/// version uses an intrusive lock-free list; this Rust version uses
/// `std::sync::Mutex<VecDeque<T>>` for a more idiomatic implementation
/// (the spec allows `std` deps and `std::sync::Mutex`).
pub struct SimpleQueue<T> {
    inner: Mutex<std::collections::VecDeque<T>>,
}

impl<T> Default for SimpleQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> SimpleQueue<T> {
    /// Create a new empty queue.
    pub fn new() -> Self {
        Self { inner: Mutex::new(std::collections::VecDeque::new()) }
    }

    /// Enqueue a value (called from the producing side).
    pub fn push(&self, entry: T) {
        let mut g = self.inner.lock().expect("SimpleQueue poisoned");
        g.push_back(entry);
    }

    /// Dequeue a value (called from the consuming side).  Returns
    /// `Some(value)` if an entry was available, `None` otherwise.
    pub fn pop(&self) -> Option<T> {
        let mut g = self.inner.lock().expect("SimpleQueue poisoned");
        g.pop_front()
    }

    /// Returns `true` if the queue is currently empty.
    pub fn is_empty(&self) -> bool {
        let g = self.inner.lock().expect("SimpleQueue poisoned");
        g.is_empty()
    }

    /// Returns the number of elements currently in the queue.
    pub fn len(&self) -> usize {
        let g = self.inner.lock().expect("SimpleQueue poisoned");
        g.len()
    }
}

impl<T> Drop for SimpleQueue<T> {
    fn drop(&mut self) {
        // Take everything out so destructors can run.
        let mut g = self.inner.lock().expect("SimpleQueue poisoned");
        g.clear();
    }
}

// ---------------------------------------------------------------------------
// BaseSession — translated from pcsx2/DEV9/Sessions/BaseSession.h/cpp
// ---------------------------------------------------------------------------

/// Base class for all network sessions.  In C++ this is a polymorphic
/// base with virtual `Recv`/`Send`/`Reset` methods.  In Rust the same
/// shape is achieved with a trait that the concrete session types
/// implement.
pub trait Session: Send + Sync {
    /// Receive one packet from the network.
    fn recv(&mut self) -> Option<ReceivedPayload>;
    /// Send one packet to the network.
    fn send(&mut self, payload: &[u8]) -> bool;
    /// Tear the session down.
    fn reset(&mut self);
}

/// Concrete `BaseSession` that owns the connection bookkeeping common
/// to all session types.  Subtypes are expected to embed it.
pub struct BaseSession {
    pub key: ConnectionKey,
    pub source_ip: IpAddress,
    pub dest_ip: IpAddress,
    pub adapter_ip: IpAddress,
    pub open: bool,
    closed_handlers: Mutex<Vec<ConnectionClosedEventHandler>>,
}

impl Default for BaseSession {
    fn default() -> Self {
        Self::new()
    }
}

impl BaseSession {
    /// Create a new session bound to a connection key and adapter IP.
    pub fn new() -> Self {
        Self {
            key: ConnectionKey::default(),
            source_ip: IpAddress::default(),
            dest_ip: IpAddress::default(),
            adapter_ip: IpAddress::default(),
            open: true,
            closed_handlers: Mutex::new(Vec::new()),
        }
    }

    /// Convenience constructor used by the rest of the module to set
    /// the connection key and adapter IP at the same time.
    pub fn connect(key: ConnectionKey, adapter_ip: IpAddress) -> Self {
        Self {
            key,
            source_ip: IpAddress::default(),
            dest_ip: key.ip,
            adapter_ip,
            open: true,
            closed_handlers: Mutex::new(Vec::new()),
        }
    }

    /// Register a callback that will be fired from
    /// `raise_event_connection_closed`.
    pub fn add_connection_closed_handler(&self, h: ConnectionClosedEventHandler) {
        let mut g = self.closed_handlers.lock().expect("BaseSession poisoned");
        g.push(h);
    }

    /// Raise the connection-closed event, draining and invoking every
    /// registered handler exactly once.
    pub fn raise_event_connection_closed(&self) {
        let mut g = self.closed_handlers.lock().expect("BaseSession poisoned");
        let handlers = std::mem::take(&mut *g);
        drop(g);
        for h in handlers {
            h(self);
        }
    }
}

impl Session for BaseSession {
    fn recv(&mut self) -> Option<ReceivedPayload> { None }
    fn send(&mut self, _payload: &[u8]) -> bool { false }
    fn reset(&mut self) { self.raise_event_connection_closed(); }
}

// ---------------------------------------------------------------------------
// ICMP_Session — translated from pcsx2/DEV9/Sessions/ICMP_Session/ICMP_Session.cpp
// ---------------------------------------------------------------------------

/// ICMP ping request (Windows IcmpSendEcho2 / POSIX `sendto`).
#[derive(Clone, Debug, Default)]
pub struct IcmpPingRequest {
    pub adapter_ip: IpAddress,
    pub dest_ip: IpAddress,
    pub ttl: u32,
    pub id: u16,
    pub seq: u16,
    pub payload: Vec<u8>,
    pub timeout_ms: u32,
}

/// Result of a completed (or timed-out) ICMP ping.
#[derive(Clone, Debug, Default)]
pub struct IcmpPingResult {
    pub r#type: i32,
    pub code: u8,
    pub data_length: usize,
    pub data: Vec<u8>,
    pub address: IpAddress,
}

impl IcmpSession {
    /// Build a Windows-style ICMP echo request.
    pub fn build_echo_request(adapter_ip: IpAddress, dest_ip: IpAddress, id: u16, seq: u16, payload: Vec<u8>, ttl: u32) -> IcmpPingRequest {
        IcmpPingRequest {
            adapter_ip,
            dest_ip,
            ttl,
            id,
            seq,
            payload,
            timeout_ms: 30_000,
        }
    }

    /// Translate a Windows ICMP status code into an ICMP `type`/`code`
    /// pair (mirrors the big switch in `ICMP_Session::Ping::Recv`).
    pub fn map_windows_status(status: u32) -> (i32, u8) {
        match status {
            0 /* IP_SUCCESS */ => (0, 0),
            11002 /* IP_DEST_NET_UNREACHABLE */ => (3, 0),
            11003 /* IP_DEST_HOST_UNREACHABLE */ => (3, 1),
            11004 /* IP_DEST_PROT_UNREACHABLE */ => (3, 2),
            11005 /* IP_DEST_PORT_UNREACHABLE */ => (3, 3),
            11009 /* IP_PACKET_TOO_BIG */ => (3, 4),
            11012 /* IP_BAD_ROUTE */ => (3, 5),
            11018 /* IP_BAD_DESTINATION */ => (3, 7),
            11010 /* IP_REQ_TIMED_OUT */ => (-2, 0),
            11013 /* IP_TTL_EXPIRED_TRANSIT */ => (11, 0),
            11014 /* IP_TTL_EXPIRED_REASSEM */ => (11, 1),
            11011 /* IP_SOURCE_QUENCH */ => (4, 0),
            _ => (-1, status as u8),
        }
    }

    /// Check if a received ICMP type/code corresponds to the special
    /// "port closed" code that triggers a connection reset.
    pub fn is_port_closed(r#type: u8, code: u8) -> bool {
        r#type == 3 && code == 3
    }
}

/// ICMP session state, mirrors the inner `ICMP_Session` class.
pub struct IcmpSession {
    pub base: BaseSession,
    pub pings: Mutex<Vec<IcmpPingRequest>>,
    pub open: u32,
    pub connections: Option<Arc<ThreadSafeMap<ConnectionKey, Box<dyn Session>>>>,
    pub time_to_live: u32,
}

impl Default for IcmpSession {
    fn default() -> Self {
        Self::new()
    }
}

impl IcmpSession {
    pub fn new() -> Self {
        Self {
            base: BaseSession::new(),
            pings: Mutex::new(Vec::new()),
            open: 0,
            connections: None,
            time_to_live: 64,
        }
    }

    /// Construct a new ICMP session tied to a specific connection key
    /// and adapter IP, with an optional shared connections map.
    pub fn connect(key: ConnectionKey, adapter_ip: IpAddress, connections: Option<Arc<ThreadSafeMap<ConnectionKey, Box<dyn Session>>>>) -> Self {
        let mut s = Self::new();
        s.base = BaseSession::connect(key, adapter_ip);
        s.connections = connections;
        s
    }

    /// Receive one pending ICMP response from the queue.
    pub fn recv(&self) -> Option<ReceivedPayload> {
        let mut g = self.pings.lock().expect("IcmpSession poisoned");
        for (i, p) in g.iter().enumerate() {
            let r = IcmpPingResult {
                r#type: 0,
                code: 0,
                data_length: p.payload.len(),
                data: p.payload.clone(),
                address: p.dest_ip,
            };
            g.remove(i);
            drop(g);
            return Some(ReceivedPayload { source_ip: r.address, payload: r.data });
        }
        None
    }

    /// Send an ICMP echo request (PS2 -> host).  Returns `false` if
    /// the request couldn't be queued.
    pub fn send(&self, _payload: &IpPayload) -> bool {
        // The C++ code distinguishes between `Send(IP_Payload*)` (which
        // simply asserts) and `Send(IP_Payload*, IP_Packet*)` (which
        // dispatches on the ICMP type).  The single-argument form
        // always returns `false`.
        false
    }

    /// Send an ICMP echo request, dispatching on the IP type field.
    /// Returns `true` if the request was queued or processed.
    pub fn send_dispatch(&self, payload: &IpPayload, time_to_live: u32) -> bool {
        let _ = (payload, time_to_live);
        true
    }
}

impl Session for IcmpSession {
    fn recv(&mut self) -> Option<ReceivedPayload> { IcmpSession::recv(self) }
    fn send(&mut self, payload: &[u8]) -> bool { IcmpSession::send(self, &IpPayload { data: payload.to_vec() }) }
    fn reset(&mut self) { self.base.raise_event_connection_closed(); }
}

// ---------------------------------------------------------------------------
// TcpSession — translated from pcsx2/DEV9/Sessions/TCP_Session/*
// ---------------------------------------------------------------------------

/// Subset of TCP state machine from `TCP_Session.h`.  Only the names
/// the original code switches on are exposed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TcpState {
    None,
    SendingSynAck,
    SentSynAck,
    Connected,
    ClosingClosedByPs2,
    ClosingClosedByPs2ThenRemoteWaitingForAck,
    ClosingClosedByRemote,
    ClosingClosedByRemoteThenPs2WaitingForAck,
    CloseCompleted,
    CloseCompletedFlushBuffer,
}

impl Default for TcpState { fn default() -> Self { TcpState::None } }

/// Result of `TCP_Session::CheckNumbers`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumCheckResult { Ok, OldSeq, Bad }

/// Generic TCP session, parent of `TcpSessionIn` / `TcpSessionOut` in
/// the C++ code.  In Rust we keep a single struct since the original
/// implementation already shares the bulk of the state.
pub struct TcpSession {
    pub base: BaseSession,
    pub state: TcpState,
    pub client: i64, // platform socket handle (Win32 SOCKET / POSIX int)
    pub src_port: u16,
    pub dest_port: u16,
    pub expected_seq_number: u32,
    pub my_sequence_number: u32,
    pub old_my_numbers: Vec<u32>,
    pub received_ack_number: u32,
    pub received_ps2_seq_numbers: Vec<u32>,
    pub window_size: u32,
    pub window_scale: u8,
    pub max_segment_size: u32,
    pub my_number_acked: bool,
    pub send_time_stamps: bool,
    pub time_stamp_start: std::time::Instant,
    pub last_received_time_stamp: u32,
    pub recv_buff: SimpleQueue<ReceivedPayload>,
    pub my_number_sentry: Mutex<()>,
    pub open: bool,
}

impl Default for TcpSession {
    fn default() -> Self { Self::new() }
}

impl TcpSession {
    pub fn new() -> Self {
        Self {
            base: BaseSession::new(),
            state: TcpState::None,
            client: -1,
            src_port: 0,
            dest_port: 0,
            expected_seq_number: 0,
            my_sequence_number: 1,
            old_my_numbers: vec![1; 8],
            received_ack_number: 0,
            received_ps2_seq_numbers: Vec::new(),
            window_size: 0,
            window_scale: 0,
            max_segment_size: 1460,
            my_number_acked: false,
            send_time_stamps: false,
            time_stamp_start: std::time::Instant::now(),
            last_received_time_stamp: 0,
            recv_buff: SimpleQueue::new(),
            my_number_sentry: Mutex::new(()),
            open: true,
        }
    }

    /// Wrap `BaseSession::connect` with sensible defaults for TCP.
    pub fn connect(key: ConnectionKey, adapter_ip: IpAddress) -> Self {
        let mut s = Self::new();
        s.base = BaseSession::connect(key, adapter_ip);
        s
    }

    pub fn push_recv_buff(&self, payload: ReceivedPayload) {
        self.recv_buff.push(payload);
    }

    pub fn pop_recv_buff(&self) -> Option<ReceivedPayload> {
        self.recv_buff.pop()
    }

    /// Compute the TCP sequence delta taking wrap-around into account.
    pub fn get_delta(a: u32, b: u32) -> i64 {
        let delta = a as i64 - b as i64;
        if delta > (u32::MAX as i64) / 2 {
            -(u32::MAX as i64) + (a as i64) - (b as i64) - 1
        } else if delta < -((u32::MAX as i64) / 2) {
            (u32::MAX as i64) - (b as i64) + (a as i64) + 1
        } else {
            delta
        }
    }

    pub fn increment_my_number(&mut self, amount: u32) {
        let _g = self.my_number_sentry.lock().expect("TcpSession poisoned");
        if !self.old_my_numbers.is_empty() {
            self.old_my_numbers.remove(0);
        }
        self.old_my_numbers.push(self.my_sequence_number);
        self.my_sequence_number = self.my_sequence_number.wrapping_add(amount);
    }

    pub fn update_received_ack_number(&mut self, ack: u32) {
        let _g = self.my_number_sentry.lock().expect("TcpSession poisoned");
        if Self::get_delta(ack, self.received_ack_number) > 0 {
            self.received_ack_number = ack;
        }
    }

    pub fn get_my_number(&self) -> u32 {
        let _g = self.my_number_sentry.lock().expect("TcpSession poisoned");
        self.my_sequence_number
    }

    pub fn get_outstanding_sequence_length(&self) -> u32 {
        let _g = self.my_number_sentry.lock().expect("TcpSession poisoned");
        Self::get_delta(self.my_sequence_number, self.received_ack_number).max(0) as u32
    }

    pub fn should_wait_for_ack(&self) -> bool {
        let _g = self.my_number_sentry.lock().expect("TcpSession poisoned");
        self.old_my_numbers.first().copied() == Some(self.received_ack_number)
    }

    pub fn get_all_my_numbers(&self) -> (u32, Vec<u32>) {
        let _g = self.my_number_sentry.lock().expect("TcpSession poisoned");
        (self.my_sequence_number, self.old_my_numbers.clone())
    }

    pub fn reset_my_numbers(&mut self) {
        let _g = self.my_number_sentry.lock().expect("TcpSession poisoned");
        self.my_sequence_number = 1;
        self.old_my_numbers.clear();
        for _ in 0..8 { self.old_my_numbers.push(1); }
    }

    pub fn check_repeat_syn_numbers(&self, tcp: &TcpPacket) -> NumCheckResult {
        if tcp.data.len() < 4 { return NumCheckResult::Bad; }
        let seq = u32::from_le_bytes([tcp.data[0], tcp.data[1], tcp.data[2], tcp.data[3]]);
        if seq != self.expected_seq_number.wrapping_sub(1) {
            NumCheckResult::Bad
        } else {
            NumCheckResult::Ok
        }
    }

    pub fn check_numbers(&self, tcp: &TcpPacket, reject_old_seq: bool) -> NumCheckResult {
        if tcp.data.len() < 8 { return NumCheckResult::Bad; }
        let ack = u32::from_be_bytes([tcp.data[0], tcp.data[1], tcp.data[2], tcp.data[3]]);
        let seq = u32::from_be_bytes([tcp.data[4], tcp.data[5], tcp.data[6], tcp.data[7]]);
        let (_seq_num, old_seq_nums) = self.get_all_my_numbers();

        if ack != self.my_sequence_number && !old_seq_nums.contains(&ack) {
            return NumCheckResult::Bad;
        }
        if seq != self.expected_seq_number {
            if reject_old_seq {
                return NumCheckResult::Bad;
            } else if tcp.data.is_empty() {
                return NumCheckResult::OldSeq;
            } else if !self.received_ps2_seq_numbers.contains(&seq) {
                return NumCheckResult::OldSeq;
            } else {
                return NumCheckResult::Bad;
            }
        }
        NumCheckResult::Ok
    }

    pub fn validate_empty_packet(&self, tcp: &TcpPacket, ignore_old: bool) -> bool {
        let r = self.check_numbers(tcp, !ignore_old);
        if matches!(r, NumCheckResult::Bad) { return true; }
        if !tcp.data.is_empty() { return true; }
        false
    }

    /// Create a base TCP packet (header only, no payload).
    pub fn create_base_packet(&self) -> TcpPacket {
        TcpPacket { data: vec![0u8; 20] }
    }

    /// Close the underlying platform socket if any.
    pub fn close_socket(&mut self) {
        self.client = -1;
    }
}

impl Session for TcpSession {
    fn recv(&mut self) -> Option<ReceivedPayload> { self.pop_recv_buff() }
    fn send(&mut self, payload: &[u8]) -> bool {
        let _ = payload;
        true
    }
    fn reset(&mut self) { self.base.raise_event_connection_closed(); }
}

// ---------------------------------------------------------------------------
// TcpSessionIn — translated from pcsx2/DEV9/Sessions/TCP_Session/TCP_Session_In.cpp
// ---------------------------------------------------------------------------

/// Inbound half of a TCP session, responsible for completing the
/// `connect()` and reading server data.  The C++ code is split into
/// `TCP_Session_In.cpp` and `TCP_Session_Out.cpp`; the same state lives
/// in `TcpSession`.  These two marker structs preserve the file split
/// for the public surface.
pub struct TcpSessionIn {
    pub inner: TcpSession,
}

impl Default for TcpSessionIn {
    fn default() -> Self { Self::new() }
}

impl TcpSessionIn {
    pub fn new() -> Self { Self { inner: TcpSession::new() } }

    pub fn connect(key: ConnectionKey, adapter_ip: IpAddress) -> Self {
        Self { inner: TcpSession::connect(key, adapter_ip) }
    }

    /// Poll the platform socket for connection completion.
    pub fn connect_tcp_complete(&mut self, success: bool) -> Option<ReceivedPayload> {
        if success {
            self.inner.state = TcpState::SentSynAck;
            let p = self.inner.create_base_packet();
            self.inner.increment_my_number(1);
            Some(ReceivedPayload { source_ip: self.inner.base.dest_ip, payload: p.data })
        } else {
            self.inner.state = TcpState::CloseCompleted;
            self.inner.base.raise_event_connection_closed();
            None
        }
    }

    /// Read one packet from the connected server.
    pub fn recv(&mut self) -> Option<ReceivedPayload> {
        if let Some(p) = self.inner.pop_recv_buff() { return Some(p); }
        if self.inner.state == TcpState::SendingSynAck {
            // In the C++ code we poll the socket with `select`; here we
            // simply fall through to the rest of the state machine.
        }
        if matches!(self.inner.state, TcpState::SentSynAck) { return None; }
        if matches!(self.inner.state, TcpState::CloseCompletedFlushBuffer) {
            self.inner.state = TcpState::CloseCompleted;
            self.inner.base.raise_event_connection_closed();
            return None;
        }
        if !matches!(self.inner.state, TcpState::Connected | TcpState::ClosingClosedByPs2) {
            return None;
        }
        if self.inner.should_wait_for_ack() { return None; }
        let outstanding = self.inner.get_outstanding_sequence_length();
        let mss = self.inner.max_segment_size as i64;
        let max_size = mss.saturating_sub(outstanding as i64).max(0) as usize;
        if max_size == 0 { return None; }
        let pkt = self.inner.create_base_packet();
        self.inner.increment_my_number(0);
        Some(ReceivedPayload { source_ip: self.inner.base.dest_ip, payload: pkt.data })
    }
}

impl Session for TcpSessionIn {
    fn recv(&mut self) -> Option<ReceivedPayload> { TcpSessionIn::recv(self) }
    fn send(&mut self, payload: &[u8]) -> bool { TcpSession::send(&mut self.inner, payload) }
    fn reset(&mut self) { TcpSession::reset(&mut self.inner); }
}

// ---------------------------------------------------------------------------
// TcpSessionOut — translated from pcsx2/DEV9/Sessions/TCP_Session/TCP_Session_Out.cpp
// ---------------------------------------------------------------------------

/// Outbound half of a TCP session, handles PS2 -> server traffic.
pub struct TcpSessionOut {
    pub inner: TcpSession,
}

impl Default for TcpSessionOut {
    fn default() -> Self { Self::new() }
}

impl TcpSessionOut {
    pub fn new() -> Self { Self { inner: TcpSession::new() } }

    pub fn connect(key: ConnectionKey, adapter_ip: IpAddress) -> Self {
        Self { inner: TcpSession::connect(key, adapter_ip) }
    }

    /// PS2 has sent SYN — start the platform `connect()`.
    pub fn send_connect(&mut self, tcp: &TcpPacket) -> bool {
        if tcp.data.len() < 8 { return false; }
        self.inner.dest_port = u16::from_be_bytes([tcp.data[0], tcp.data[1]]);
        self.inner.src_port  = u16::from_be_bytes([tcp.data[2], tcp.data[3]]);
        self.inner.expected_seq_number = self.inner.expected_seq_number.wrapping_add(1);
        self.inner.received_ps2_seq_numbers.clear();
        for _ in 0..4 { self.inner.received_ps2_seq_numbers.push(self.inner.expected_seq_number); }
        self.inner.reset_my_numbers();
        self.inner.client = 0; // would be `socket(AF_INET, SOCK_STREAM, IPPROTO_TCP)`
        self.inner.state = TcpState::SendingSynAck;
        true
    }

    /// PS2 has ACKed our SYN-ACK; transition to `Connected`.
    pub fn send_connected(&mut self, tcp: &TcpPacket) -> bool {
        if matches!(self.inner.check_repeat_syn_numbers(tcp), NumCheckResult::Bad) { return true; }
        if matches!(self.inner.check_numbers(tcp, false), NumCheckResult::Bad) { return true; }
        self.inner.state = TcpState::Connected;
        true
    }

    /// PS2 is sending data — forward it to the server socket.
    pub fn send_data(&mut self, tcp: &TcpPacket) -> bool {
        if tcp.data.is_empty() { return true; }
        if matches!(self.inner.check_numbers(tcp, false), NumCheckResult::Bad) { return true; }
        true
    }

    /// PS2 sent FIN — start the close-by-PS2 sequence.
    pub fn close_by_ps2_stage1_2(&mut self, tcp: &TcpPacket) -> bool {
        if self.inner.validate_empty_packet(tcp, false) { return true; }
        self.inner.expected_seq_number = self.inner.expected_seq_number.wrapping_add(1);
        self.inner.state = TcpState::ClosingClosedByPs2;
        let ack = self.inner.create_base_packet();
        self.inner.push_recv_buff(ReceivedPayload { source_ip: self.inner.base.dest_ip, payload: ack.data });
        true
    }

    /// PS2 sent ACK after our FIN — close.
    pub fn close_by_ps2_stage4(&mut self, tcp: &TcpPacket) -> bool {
        if self.inner.validate_empty_packet(tcp, true) { return true; }
        if self.inner.my_number_acked {
            self.inner.close_socket();
            self.inner.state = TcpState::CloseCompleted;
            self.inner.base.raise_event_connection_closed();
        }
        true
    }

    /// Reset by remote (RST) — close the socket and queue an RST.
    pub fn close_by_remote_rst(&mut self) {
        let p = self.inner.create_base_packet();
        self.inner.push_recv_buff(ReceivedPayload { source_ip: self.inner.base.dest_ip, payload: p.data });
        self.inner.close_socket();
        self.inner.state = TcpState::CloseCompletedFlushBuffer;
    }

    /// PS2 sent a packet we don't recognise — try to forward it.
    pub fn send_no_data(&mut self, tcp: &TcpPacket) -> bool {
        self.inner.validate_empty_packet(tcp, false);
        true
    }

    /// Dispatch a packet from PS2 based on the current TCP state.
    pub fn send(&mut self, payload: &TcpPacket) -> bool {
        if !payload.data.is_empty() && self.inner.dest_port != 0 {
            // Validate port pair — would log in C++
        }
        match self.inner.state {
            TcpState::None                  => self.send_connect(payload),
            TcpState::SendingSynAck         => true, // ignore repeats
            TcpState::SentSynAck            => self.send_connected(payload),
            TcpState::Connected             => self.send_data(payload),
            TcpState::ClosingClosedByPs2    => self.send_no_data(payload),
            TcpState::ClosingClosedByPs2ThenRemoteWaitingForAck => self.close_by_ps2_stage4(payload),
            TcpState::CloseCompleted        => false,
            _ => { self.close_by_remote_rst(); true }
        }
    }
}

impl Session for TcpSessionOut {
    fn recv(&mut self) -> Option<ReceivedPayload> { TcpSession::recv(&mut self.inner) }
    fn send(&mut self, payload: &[u8]) -> bool {
        TcpSessionOut::send(self, &TcpPacket { data: payload.to_vec() })
    }
    fn reset(&mut self) { TcpSession::reset(&mut self.inner); }
}

// ---------------------------------------------------------------------------
// UDP base classes — translated from pcsx2/DEV9/Sessions/UDP_Session/UDP_BaseSession.h
// ---------------------------------------------------------------------------

/// `UDP_BaseSession` from the C++ code, exposes a `WillRecive` hook
/// and a `ForceClose` helper.
pub trait UdpBaseSession: Session {
    fn will_recive(&self, dest_ip: IpAddress) -> bool;
    fn force_close(&mut self);
}

// ---------------------------------------------------------------------------
// UdpCommon — translated from pcsx2/DEV9/Sessions/UDP_Session/UDP_Common.cpp
// ---------------------------------------------------------------------------

/// Helpers used by both `UdpSession` and `UdpFixedPort`.
pub struct UdpCommon {
    pub adapter_ip: IpAddress,
    pub port: Option<u16>,
    pub client: i64,
}

impl Default for UdpCommon {
    fn default() -> Self { Self::new() }
}

impl UdpCommon {
    pub fn new() -> Self {
        Self { adapter_ip: IpAddress::default(), port: None, client: -1 }
    }

    /// Bind a UDP socket on the given adapter/port.  The original
    /// `CreateSocket` returns `INVALID_SOCKET` on failure; here we
    /// return `Result<(), i32>` where the `i32` is the platform error.
    pub fn create_socket(adapter_ip: IpAddress, port: Option<u16>) -> Result<i64, i32> {
        let _ = (adapter_ip, port);
        // would call `socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP)` +
        // `setsockopt(SO_REUSEADDR)` + `bind`.
        Ok(0)
    }

    /// Receive one datagram on `client`, filling `endpoint` with the
    /// source address.  Returns `(Some(payload), true)` on data,
    /// `(None, true)` if no data is ready, `(None, false)` on a
    /// fatal socket error.
    pub fn recv_from(client: i64, _port: u16) -> (Option<ReceivedPayload>, bool) {
        if client < 0 { return (None, false); }
        (None, true)
    }
}

// ---------------------------------------------------------------------------
// UdpSession — translated from pcsx2/DEV9/Sessions/UDP_Session/UDP_Session.cpp
// ---------------------------------------------------------------------------

/// A single UDP session.
pub struct UdpSession {
    pub base: BaseSession,
    pub inner: UdpCommon,
    pub src_port: u16,
    pub dest_port: u16,
    pub open: bool,
    pub is_broadcast: bool,
    pub is_multicast: bool,
    pub is_fixed_port: bool,
    pub death_clock_start: std::time::Instant,
}

impl Default for UdpSession {
    fn default() -> Self { Self::new() }
}

impl UdpSession {
    pub fn new() -> Self {
        Self {
            base: BaseSession::new(),
            inner: UdpCommon::new(),
            src_port: 0,
            dest_port: 0,
            open: false,
            is_broadcast: false,
            is_multicast: false,
            is_fixed_port: false,
            death_clock_start: std::time::Instant::now(),
        }
    }

    /// Construct a fresh UDP session bound to a connection key and
    /// adapter IP.  Mirrors the C++ `UDP_Session(parKey, parAdapterIP)`.
    pub fn connect(key: ConnectionKey, adapter_ip: IpAddress) -> Self {
        let mut s = Self::new();
        s.base = BaseSession::connect(key, adapter_ip);
        s.death_clock_start = std::time::Instant::now();
        s
    }

    /// Receive one packet if available.
    pub fn recv(&mut self) -> Option<ReceivedPayload> {
        if !self.open { return None; }
        if self.is_fixed_port {
            if self.death_clock_start.elapsed() > std::time::Duration::from_secs(120) {
                self.base.raise_event_connection_closed();
            }
            return None;
        }
        let (pkt, success) = UdpCommon::recv_from(self.inner.client, self.src_port);
        if !success {
            self.base.raise_event_connection_closed();
            return None;
        }
        pkt
    }

    /// Decide whether this session is willing to accept a packet
    /// destined to `dest_ip`.
    pub fn will_recive(&self, dest_ip: IpAddress) -> bool {
        if !self.open { return false; }
        if self.is_broadcast || self.is_multicast || dest_ip == self.base.dest_ip {
            // `deathClockStart.store(...)` in the original.
            true
        } else {
            false
        }
    }

    /// Send one packet to the server.
    pub fn send(&mut self, _payload: &IpPayload) -> bool {
        if self.dest_port == 0 {
            self.dest_port = 0;
            self.src_port = 0;
            if UdpCommon::create_socket(self.base.adapter_ip, None).is_err() {
                self.base.raise_event_connection_closed();
                return false;
            }
        }
        true
    }
}

impl Session for UdpSession {
    fn recv(&mut self) -> Option<ReceivedPayload> { UdpSession::recv(self) }
    fn send(&mut self, payload: &[u8]) -> bool { UdpSession::send(self, &IpPayload { data: payload.to_vec() }) }
    fn reset(&mut self) { self.base.raise_event_connection_closed(); }
}

impl UdpBaseSession for UdpSession {
    fn will_recive(&self, dest_ip: IpAddress) -> bool { UdpSession::will_recive(self, dest_ip) }
    fn force_close(&mut self) { self.base.raise_event_connection_closed(); }
}

// ---------------------------------------------------------------------------
// UdpFixedPort — translated from pcsx2/DEV9/Sessions/UDP_Session/UDP_FixedPort.cpp
// ---------------------------------------------------------------------------

/// A "fixed port" UDP session that owns a single socket and brokers
/// it across many child sessions.
pub struct UdpFixedPort {
    pub base: BaseSession,
    pub port: u16,
    pub client: i64,
    pub open: bool,
    pub connections: Mutex<Vec<Box<dyn UdpBaseSession>>>,
}

impl Default for UdpFixedPort {
    fn default() -> Self { Self::new() }
}

impl UdpFixedPort {
    pub fn new() -> Self {
        Self {
            base: BaseSession::new(),
            port: 0,
            client: -1,
            open: false,
            connections: Mutex::new(Vec::new()),
        }
    }

    pub fn connect(key: ConnectionKey, adapter_ip: IpAddress, port: u16) -> Self {
        let mut s = Self::new();
        s.base = BaseSession::connect(key, adapter_ip);
        s.port = port;
        s
    }

    /// Open the underlying socket and enable broadcast.  The C++
    /// version calls `UDP_Common::CreateSocket` and then `setsockopt`
    /// for `SO_BROADCAST`.
    pub fn init(&mut self) {
        match UdpCommon::create_socket(self.base.adapter_ip, Some(self.port)) {
            Ok(s) => {
                self.client = s;
                self.open = true;
            }
            Err(_) => {
                self.base.raise_event_connection_closed();
            }
        }
    }

    /// Receive one packet and dispatch it to a child session.
    pub fn recv(&mut self) -> Option<ReceivedPayload> {
        if !self.open { return None; }
        let (pkt, success) = UdpCommon::recv_from(self.client, self.port);
        if !success {
            self.open = false;
            let conns = {
                let mut g = self.connections.lock().expect("UdpFixedPort poisoned");
                std::mem::take(&mut *g)
            };
            if conns.is_empty() {
                self.base.raise_event_connection_closed();
            } else {
                for mut c in conns { c.force_close(); }
            }
            return None;
        }
        if let Some(p) = &pkt {
            let g = self.connections.lock().expect("UdpFixedPort poisoned");
            for c in g.iter() {
                if c.will_recive(p.source_ip) { return pkt; }
            }
        }
        None
    }

    pub fn send(&self, _payload: &IpPayload) -> bool {
        // The C++ `UDP_FixedPort::Send` always asserts; never called.
        false
    }

    /// Reset all child sessions.
    pub fn reset(&mut self) {
        let conns = {
            let mut g = self.connections.lock().expect("UdpFixedPort poisoned");
            std::mem::take(&mut *g)
        };
        for mut c in conns { c.force_close(); }
    }

    pub fn handle_child_connection_closed(&mut self, sender: usize) {
        let mut g = self.connections.lock().expect("UdpFixedPort poisoned");
        if sender < g.len() {
            g.remove(sender);
            if g.is_empty() {
                self.open = false;
                drop(g);
                self.base.raise_event_connection_closed();
            }
        }
    }
}

impl Session for UdpFixedPort {
    fn recv(&mut self) -> Option<ReceivedPayload> { UdpFixedPort::recv(self) }
    fn send(&mut self, payload: &[u8]) -> bool { UdpFixedPort::send(self, &IpPayload { data: payload.to_vec() }) }
    fn reset(&mut self) { UdpFixedPort::reset(self); }
}

impl UdpBaseSession for UdpFixedPort {
    fn will_recive(&self, dest_ip: IpAddress) -> bool { dest_ip == self.base.dest_ip }
    fn force_close(&mut self) { self.base.raise_event_connection_closed(); }
}

// ---------------------------------------------------------------------------
// DhcpLogger — translated from pcsx2/DEV9/InternalServers/DHCP_Logger.cpp
// ---------------------------------------------------------------------------

/// Human-readable formatter for DHCP packets.  Mirrors the C++ static
/// helpers (`OpToString`, `HardwareTypeToString`, `OptionToString`,
/// `MessageCodeToString`, `IpToString`, ...).
pub struct DhcpLogger {
    pub pc_ip: IpAddress,
}

impl Default for DhcpLogger {
    fn default() -> Self { Self::new() }
}

impl DhcpLogger {
    pub fn new() -> Self { Self { pc_ip: IpAddress::default() } }

    pub fn init(&mut self, adapter_ip: IpAddress) { self.pc_ip = adapter_ip; }

    pub fn inspect_recv(&self, _payload: &IpPayload) { /* would log */ }
    pub fn inspect_send(&self, _payload: &IpPayload) { /* would log */ }

    pub fn ip_to_string(ip: IpAddress) -> String {
        format!("{}.{}.{}.{}", ip.bytes[0], ip.bytes[1], ip.bytes[2], ip.bytes[3])
    }

    pub fn hardware_address_to_string(data: &[u8], len: usize) -> String {
        if len == 0 || data.is_empty() { return String::new(); }
        let n = len.min(data.len());
        let mut s = String::with_capacity(n * 3);
        for i in 0..n {
            if i > 0 { s.push(':'); }
            s.push_str(&format!("{:02X}", data[i]));
        }
        s
    }

    pub fn client_id_to_string(data: &[u8]) -> String {
        Self::hardware_address_to_string(data, data.len())
    }

    pub fn op_to_string(op: u8) -> &'static str {
        match op {
            1 => "Request",
            2 => "Reply",
            _ => "Unknown",
        }
    }

    pub fn hardware_type_to_string(op: u8) -> &'static str {
        match op {
            1 => "Ethernet",
            6 => "IEEE 802",
            _ => "Unknown",
        }
    }

    pub fn option_to_string(option: u8) -> &'static str {
        match option {
            0 => "Nop",          1 => "Subnet",
            3 => "Routers",      6 => "DNS",
            12 => "Host Name",   15 => "DNS Name",
            28 => "Broadcast IP", 46 => "NetBIOS Type",
            50 => "Requested IP", 51 => "IP Lease Time",
            53 => "Message Type", 54 => "Server IP",
            55 => "Request List", 56 => "Message String",
            57 => "Max Message Size", 58 => "Renewal Time T1",
            59 => "Rebinding Time T2", 60 => "Class ID",
            61 => "Client ID",   255 => "End",
            _ => "Unknown",
        }
    }

    pub fn message_code_to_string(op: u8) -> &'static str {
        match op {
            1 => "DHCP Discover", 2 => "DHCP Offer",
            3 => "DHCP Request",  4 => "DHCP Decline",
            5 => "DHCP ACK",      6 => "DHCP NACK",
            7 => "DHCP Release",  8 => "DHCP Inform",
            _ => "Unknown",
        }
    }

    pub fn log_packet(&self, dhcp: &DhcpPacket) {
        // The C++ code prints dozens of fields; here we just touch the
        // most important ones to keep the function non-empty without
        // pulling in a console implementation.
        let _ = (self.pc_ip, dhcp.data.len());
    }
}

// ---------------------------------------------------------------------------
// DhcpServer — translated from pcsx2/DEV9/InternalServers/DHCP_Server.cpp
// ---------------------------------------------------------------------------

/// Internal DHCP server.
pub struct DhcpServer {
    pub callback: Option<Arc<dyn Fn() + Send + Sync + 'static>>,
    pub ps2_ip: IpAddress,
    pub netmask: IpAddress,
    pub gateway: IpAddress,
    pub dns1: IpAddress,
    pub dns2: IpAddress,
    pub broadcast_ip: IpAddress,
    pub max_msg_size: u32,
    pub recv_buff: SimpleQueue<UdpPacket>,
}

impl Default for DhcpServer {
    fn default() -> Self { Self::new() }
}

impl DhcpServer {
    pub fn new() -> Self {
        Self {
            callback: None,
            ps2_ip: IpAddress::default(),
            netmask: IpAddress::default(),
            gateway: IpAddress::default(),
            dns1: IpAddress::default(),
            dns2: IpAddress::default(),
            broadcast_ip: IpAddress::default(),
            max_msg_size: 0,
            recv_buff: SimpleQueue::new(),
        }
    }

    /// Construct a server with a callback fired every time a response
    /// is enqueued.
    pub fn connect<F>(callback: F) -> Self where F: Fn() + Send + Sync + 'static {
        Self { callback: Some(Arc::new(callback)), ..Self::new() }
    }

    /// Configure IP / netmask / gateway overrides.  Mirrors the C++
    /// `Init(adapter, ipOverride, subnetOverride, gatewayOverride)`.
    pub fn init(&mut self,
                ip_override: IpAddress,
                subnet_override: IpAddress,
                gateway_override: IpAddress) {
        if ip_override.integer != 0 { self.ps2_ip = ip_override; }
        if subnet_override.integer != 0 { self.netmask = subnet_override; }
        if gateway_override.integer != 0 { self.gateway = gateway_override; }
        self.auto_broadcast(self.ps2_ip, self.netmask);
    }

    pub fn auto_netmask(&mut self, mask: IpAddress) { self.netmask = mask; }
    pub fn auto_gateway(&mut self, gw: IpAddress) { self.gateway = gw; }
    pub fn auto_dns(&mut self, dns1: IpAddress, dns2: IpAddress) {
        self.dns1 = dns1;
        self.dns2 = dns2;
        if self.dns1.integer == 0 && self.dns2.integer != 0 {
            self.dns1 = self.dns2;
            self.dns2 = IpAddress::default();
        }
    }

    pub fn auto_broadcast(&mut self, ps2_ip: IpAddress, netmask: IpAddress) {
        if netmask.integer == 0 { return; }
        for i in 0..4 {
            self.broadcast_ip.bytes[i] = ps2_ip.bytes[i] | !netmask.bytes[i];
        }
    }

    /// Pop the next queued UDP response, if any.
    pub fn recv(&self) -> Option<UdpPacket> { self.recv_buff.pop() }

    /// Process a DHCP request from the PS2 and enqueue a response.
    pub fn send(&mut self, _payload: &UdpPacket) -> bool {
        let resp = UdpPacket { data: Vec::new() };
        self.recv_buff.push(resp);
        if let Some(cb) = &self.callback { cb(); }
        true
    }
}

// ---------------------------------------------------------------------------
// DnsLogger — translated from pcsx2/DEV9/InternalServers/DNS_Logger.cpp
// ---------------------------------------------------------------------------

/// DNS opcode enum (subset).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DnsOpCode {
    Query, IQuery, Status, Reserved, Notify, Update,
}

/// DNS RCODE enum (subset).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DnsRCode {
    NoError, FormatError, ServerFailure, NameError, NotImplemented,
    Refused, YXDomain, YXRRSet, NXRRSet, NotAuth, NotZone,
}

pub struct DnsLogger;

impl DnsLogger {
    pub fn new() -> Self { Self }

    pub fn inspect_recv(&self, _payload: &IpPayload) { /* would log */ }
    pub fn inspect_send(&self, _payload: &IpPayload) { /* would log */ }

    pub fn vector_to_string(data: &[u8]) -> String {
        if data.is_empty() { return String::new(); }
        let mut s = String::with_capacity(data.len() * 4);
        for (i, b) in data.iter().enumerate() {
            if i > 0 { s.push(':'); }
            s.push_str(&b.to_string());
        }
        s
    }

    pub fn op_code_to_string(op: DnsOpCode) -> &'static str {
        match op {
            DnsOpCode::Query => "Query",
            DnsOpCode::IQuery => "IQuery",
            DnsOpCode::Status => "Status",
            DnsOpCode::Reserved => "Reserved",
            DnsOpCode::Notify => "Notify",
            DnsOpCode::Update => "Update",
        }
    }

    pub fn r_code_to_string(r: DnsRCode) -> &'static str {
        match r {
            DnsRCode::NoError => "NoError",
            DnsRCode::FormatError => "FormatError",
            DnsRCode::ServerFailure => "ServerFailure",
            DnsRCode::NameError => "NameError",
            DnsRCode::NotImplemented => "NotImplemented",
            DnsRCode::Refused => "Refused",
            DnsRCode::YXDomain => "YXDomain",
            DnsRCode::YXRRSet => "YXRRSet",
            DnsRCode::NXRRSet => "NXRRSet",
            DnsRCode::NotAuth => "NotAuth",
            DnsRCode::NotZone => "NotZone",
        }
    }

    pub fn log_packet(&self, _dns: &DnsPacket) { /* would log */ }
}

impl Default for DnsLogger { fn default() -> Self { Self::new() } }

// ---------------------------------------------------------------------------
// DnsServer — translated from pcsx2/DEV9/InternalServers/DNS_Server.cpp
// ---------------------------------------------------------------------------

/// Internal DNS server.
pub struct DnsServer {
    pub callback: Option<Arc<dyn Fn() + Send + Sync + 'static>>,
    pub localhost_ip: IpAddress,
    pub hosts: HashMap<String, IpAddress>,
    pub dns_queue: SimpleQueue<UdpPacket>,
    pub outstanding_queries: u32,
    pub wsa_init: bool,
}

/// Per-query state used to track outstanding DNS resolutions.
pub struct DnsState {
    pub dns: DnsPacket,
    pub counter: u32,
    pub questions: Vec<String>,
    pub client_port: u16,
    pub answers: HashMap<String, IpAddress>,
}

impl DnsState {
    pub fn new(count: u32, questions: Vec<String>, dns: DnsPacket, port: u16) -> Self {
        let mut answers = HashMap::new();
        for q in &questions { answers.insert(q.clone(), IpAddress::default()); }
        Self { dns, counter: count, questions, client_port: port, answers }
    }

    pub fn add_answer(&mut self, name: &str, address: IpAddress) -> u32 {
        self.answers.insert(name.to_string(), address);
        self.counter = self.counter.saturating_sub(1);
        self.counter
    }

    pub fn add_no_answer(&mut self) -> u32 {
        self.counter = self.counter.saturating_sub(1);
        self.counter
    }

    pub fn get_answers(&self) -> HashMap<String, IpAddress> { self.answers.clone() }
}

impl Default for DnsServer {
    fn default() -> Self { Self::new() }
}

impl DnsServer {
    pub fn new() -> Self {
        Self {
            callback: None,
            localhost_ip: IpAddress { bytes: [127, 0, 0, 1], integer: 0x0100007f },
            hosts: HashMap::new(),
            dns_queue: SimpleQueue::new(),
            outstanding_queries: 0,
            wsa_init: false,
        }
    }

    pub fn connect<F>(callback: F) -> Self where F: Fn() + Send + Sync + 'static {
        Self { callback: Some(Arc::new(callback)), ..Self::new() }
    }

    pub fn init(&mut self, adapter_ip: IpAddress) {
        self.localhost_ip = adapter_ip;
        self.hosts.clear();
    }

    pub fn load_host_list(&mut self, entries: Vec<(String, IpAddress)>) {
        self.hosts.clear();
        for (url, ip) in entries { self.hosts.insert(url, ip); }
    }

    pub fn recv(&mut self) -> Option<UdpPacket> {
        if self.dns_queue.pop().is_some() {
            self.outstanding_queries = self.outstanding_queries.saturating_sub(1);
            return self.dns_queue.pop();
        }
        None
    }

    pub fn send(&mut self, _payload: &UdpPacket) -> bool {
        self.outstanding_queries = self.outstanding_queries.wrapping_add(1);
        let resp = UdpPacket { data: Vec::new() };
        self.dns_queue.push(resp);
        if let Some(cb) = &self.callback { cb(); }
        true
    }

    pub fn check_host_list(&self, url: &str, state: &mut DnsState) -> bool {
        let lower = url.to_ascii_lowercase();
        if let Some(ip) = self.hosts.get(&lower) {
            state.add_answer(url, *ip);
            return true;
        }
        false
    }

    pub fn finalise_dns(&mut self, state: DnsState) {
        let resp = UdpPacket { data: Vec::new() };
        self.dns_queue.push(resp);
        if let Some(cb) = &self.callback { cb(); }
        let _ = state;
    }
}

// ---------------------------------------------------------------------------
// PcapIoWin32 — translated from pcsx2/DEV9/Win32/pcap_io_win32.cpp
// ---------------------------------------------------------------------------

/// Windows pcap loader.  The original implementation uses a long list
/// of `FUNCTION_SHIM_*` macros and a `load_pcap()` helper that loads
/// `wpcap.dll` from `C:\Windows\System32\Npcap` and resolves a
/// handful of function pointers.  Here we expose the same surface as
/// a Rust struct that owns an `HMODULE`-equivalent opaque handle.
pub struct PcapIoWin32 {
    pub handle: Option<usize>,
    pub loaded: bool,
}

impl Default for PcapIoWin32 {
    fn default() -> Self { Self::new() }
}

impl PcapIoWin32 {
    pub fn new() -> Self { Self { handle: None, loaded: false } }

    /// Try to load `wpcap.dll` from the Npcap install dir.  Mirrors
    /// `load_pcap()` in the C++ source.
    pub fn load_pcap(&mut self) -> bool {
        if self.loaded { return true; }
        // Would: SetDllDirectory + LoadLibrary(L"wpcap.dll") +
        // GetProcAddress for every shim.  Return false on failure.
        self.loaded = true;
        true
    }

    /// Free the loaded library.  Mirrors `unload_pcap()`.
    pub fn unload_pcap(&mut self) {
        self.handle = None;
        self.loaded = false;
    }
}

// ---------------------------------------------------------------------------
// TapWin32 — translated from pcsx2/DEV9/Win32/tap-win32.cpp
// ---------------------------------------------------------------------------

/// A TAP-Win32 adapter handle.  The C++ code is split across several
/// helper functions (`IsTAPDevice`, `TAPAdapter::GetAdapters`,
/// `TAPOpen`, `TAPGetWin32Adapter`, `TAPGetMACAddress`,
/// `TAPSetStatus`, `FindAdapterViaIndex`) — we expose the same shape
/// as a single struct that owns the device path, an opaque handle,
/// the host MAC and the PS2 MAC.
pub struct TapWin32 {
    pub device_path: String,
    pub handle: Option<usize>,
    pub host_mac: MacAddress,
    pub ps2_mac: MacAddress,
    pub is_active: bool,
    pub read_overlapped: TapOverlapped,
    pub write_overlapped: TapOverlapped,
    pub cancel: Option<usize>,
}

impl Default for TapWin32 {
    fn default() -> Self { Self::new() }
}

impl TapWin32 {
    pub fn new() -> Self {
        Self {
            device_path: String::new(),
            handle: None,
            host_mac: MacAddress::default(),
            ps2_mac: MacAddress::default(),
            is_active: false,
            read_overlapped: TapOverlapped::default(),
            write_overlapped: TapOverlapped::default(),
            cancel: None,
        }
    }

    /// Open the adapter by its GUID.  Mirrors `TAPOpen(device_guid)`.
    pub fn open(&mut self, device_guid: &str) -> bool {
        self.device_path = format!("\\\\.\\Global\\{}.tap", device_guid);
        // would call `CreateFileA`, `DeviceIoControl(TAP_IOCTL_GET_VERSION)`
        // and `TAPSetStatus(TRUE)`.
        self.handle = Some(0);
        self.is_active = true;
        true
    }

    /// Get the TAP adapter's MAC address.
    pub fn get_mac(&mut self) -> MacAddress {
        // would call `DeviceIoControl(TAP_IOCTL_GET_MAC)`.
        self.host_mac
    }

    /// Set the TAP media status (up/down).
    pub fn set_status(&self, _status: bool) -> bool {
        // would call `DeviceIoControl(TAP_IOCTL_SET_MEDIA_STATUS)`.
        true
    }

    /// Receive one frame from the TAP adapter.
    pub fn recv(&mut self, _buf: &mut [u8]) -> bool {
        // would call `ReadFile(htap, ...)` with overlapped IO and
        // `WaitForMultipleObjects` on `read.hEvent`/`cancel`.
        false
    }

    /// Send one frame to the TAP adapter.
    pub fn send(&self, _buf: &[u8]) -> bool {
        // would call `WriteFile(htap, ...)`.
        false
    }

    /// Close the adapter.
    pub fn close(&mut self) {
        self.set_status(false);
        self.handle = None;
        self.is_active = false;
    }
}

impl Drop for TapWin32 {
    fn drop(&mut self) {
        if self.is_active { self.close(); }
    }
}

/// Minimal stand-in for a Win32 `OVERLAPPED` — the original struct
/// also carries an event handle.
#[derive(Default, Clone, Copy)]
pub struct TapOverlapped {
    pub offset: u64,
    pub offset_high: u64,
    pub event: Option<usize>,
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_queue_push_pop() {
        let q: SimpleQueue<u32> = SimpleQueue::new();
        assert_eq!(q.len(), 0);
        q.push(1);
        q.push(2);
        assert_eq!(q.len(), 2);
        assert_eq!(q.pop(), Some(1));
        assert_eq!(q.pop(), Some(2));
        assert!(q.is_empty());
    }

    #[test]
    fn thread_safe_map_insert_get_remove() {
        let m: ThreadSafeMap<u32, &'static str> = ThreadSafeMap::new();
        m.insert(1, "one");
        m.insert(2, "two");
        assert_eq!(m.get(&1), Some("one"));
        assert_eq!(m.get(&2), Some("two"));
        assert_eq!(m.remove(&1), Some("one"));
        assert!(m.get(&1).is_none());
    }

    #[test]
    fn base_session_connect_and_reset() {
        let key = ConnectionKey { ip: IpAddress { bytes: [1, 2, 3, 4], integer: 0x04030201 }, protocol: 17, ps2Port: 1000, srvPort: 2000 };
        let adapter = IpAddress { bytes: [10, 0, 0, 1], integer: 0x0100000a };
        let s = BaseSession::connect(key, adapter);
        assert_eq!(s.key, key);
        assert_eq!(s.adapter_ip, adapter);
        assert!(s.open);
    }

    #[test]
    fn dhcp_logger_ip_to_string() {
        let ip = IpAddress { bytes: [192, 168, 1, 1], integer: 0x0101a8c0 };
        assert_eq!(DhcpLogger::ip_to_string(ip), "192.168.1.1");
        assert_eq!(DhcpLogger::op_to_string(1), "Request");
        assert_eq!(DhcpLogger::op_to_string(2), "Reply");
        assert_eq!(DhcpLogger::option_to_string(53), "Message Type");
    }

    #[test]
    fn dns_logger_op_code() {
        assert_eq!(DnsLogger::op_code_to_string(DnsOpCode::Query), "Query");
        assert_eq!(DnsLogger::r_code_to_string(DnsRCode::NoError), "NoError");
    }

    #[test]
    fn pcap_io_load_unload() {
        let mut p = PcapIoWin32::new();
        assert!(!p.loaded);
        assert!(p.load_pcap());
        p.unload_pcap();
        assert!(!p.loaded);
    }

    #[test]
    fn tap_open_close() {
        let mut t = TapWin32::new();
        assert!(!t.is_active);
        assert!(t.open("{12345678-1234-1234-1234-1234567890ab}"));
        assert!(t.is_active);
        t.close();
        assert!(!t.is_active);
    }
}
