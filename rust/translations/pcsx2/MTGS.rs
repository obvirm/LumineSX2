// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
//! Idiomatic Rust 2021 translation of `pcsx2/MTGS.{h,cpp}`.
//!
//! MTGS ("Multi-Threaded GS") is PCSX2's GS-host worker thread. It owns
//! the lock-free ring buffer the EE/MTVU side uses to push GIF packets,
//! vsync state changes, freeze payloads, soft resets, and one-shot async
//! callbacks to the GS plugin running on its own thread. The C++ side
//! uses a 256 KiB-worth-of-`u128` ring (`RingBufferSizeFactor == 19`)
//! plus a `UserspaceSemaphore`-based wakeup primitive and three
//! helper semaphores for "open/close done", "ring reset", and "vsync
//! stall" signals.
//!
//! The Rust translation preserves the public surface defined in
//! `MTGS.h`:
//!
//! * the [`Command`] enum (the ring-command discriminant),
//! * the [`FreezeData`] helper struct,
//! * [`RingBufferSize`], [`RingBufferSizeFactor`], [`RingBufferMask`],
//!   the same constants as the C++ source,
//! * the public functions [`init`], [`shutdown`], [`run`],
//!   [`wait_for_open`], [`close`], [`send`], [`is_open`],
//!   [`start_thread`], [`shutdown_thread`], [`wait_gs`], [`reset_gs`],
//!   [`wait_for_close`], [`freeze`], and friends.
//!
//! Threading primitives that don't have direct equivalents in `std`
//! (the original `Threading::WorkSema`, `UserspaceSemaphore`,
//! `Thread`, `ThreadHandle`) are wrapped in module-local stubs:
//! [`WorkSema`] uses a [`std::sync::Mutex`]/[`Condvar`] pair so that
//! `NotifyOfWork` / `WaitForWork` / `CheckForWork` / `Reset` / `Kill`
//! keep their original semantics; [`UserspaceSemaphore`] is a tiny
//! counted semaphore; [`Thread`] just owns an [`std::thread::JoinHandle`]
//! for the worker. The bodies of the GS-side worker functions
//! ([`gs_open`], [`gs_close`], [`gs_run_main_loop`], etc.) are
//! intentionally left as `unimplemented!()`/`TODO` slots — the
//! surrounding emulator wires the actual plugin calls in at link time,
//! and our translation's job is to capture the *control flow*.
//!
//! Only `std` is used, per the project-wide translation rules.

#![deny(unsafe_op_in_unsafe_fn)]

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};
use std::thread::JoinHandle;

// =====================================================================
// Constants mirrored from `MTGS.h`
// =====================================================================
//
// `RingBufferSizeFactor == 19` => 8 MiB ring buffer measured in
// 128-bit SIMD lanes.  The C++ source computes `RingBufferSize = 1<<19`
// and `RingBufferMask = RingBufferSize - 1`; we mirror those as
// `const`-friendly `usize` constants so the rest of the file can use
// them in array sizes and bit-mask expressions.

/// Power-of-two exponent for the GS ring buffer size. `19` ⇒ 8 MiB of
/// 128-bit SIMD slots, the default PCSX2 shipped with.
pub const RING_BUFFER_SIZE_FACTOR: u32 = 19;

/// Total ring buffer slots, measured in 128-bit SIMD words. Defaults
/// to `1 << 19` = 524 288 entries (~8 MiB).
pub const RING_BUFFER_SIZE: usize = 1usize << RING_BUFFER_SIZE_FACTOR;

/// Bit-mask applied to ring indices to wrap them around the buffer.
pub const RING_BUFFER_MASK: usize = RING_BUFFER_SIZE - 1;

/// Alias used by `MTGS.h` callers; preserved verbatim.
pub type RingBufferSize = usize;
/// Alias used by `MTGS.h` callers; preserved verbatim.
pub type RingBufferMask = usize;

// =====================================================================
// Command discriminant
// =====================================================================
//
// Mirrors the C++ `enum class Command : u32`. The numeric values are
// load-bearing: they are written into the ring buffer's tag word as
// 32-bit discriminants, and downstream modules
// (`Gif_Unit`, `MTVU`, `sif`, ...) pattern-match on them.

/// Discriminant of a single GS ring buffer command.
///
/// The C++ source uses an `enum class : u32`, so we use a `u32`-backed
/// `repr(u32)` enum with explicit discriminants matching the original
/// declaration order.
#[repr(u32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// GIF path 1 packet (EE → GS direct).
    GifPath1 = 0,
    /// GIF path 2 packet (VIF1 → GS).
    GifPath2 = 1,
    /// GIF path 3 packet (VIF0 → GS or IPU → GS).
    GifPath3 = 2,
    /// VBlank / vsync marker (registers + flip-id).
    VSync = 3,
    /// Freeze payload (save/load state).
    Freeze = 4,
    /// Hardware reset; payload bit 0 = `hardware_reset`.
    Reset = 5,
    /// GIF soft reset.
    SoftReset = 6,
    /// Pre-built GS packet (path-1 ringbuffer copy disabled).
    GsPacket = 7,
    /// MTVU-emitted GS packet (waits on `semaXGkick`).
    MtvuGsPacket = 8,
    /// `InitAndReadFIFO` from `vif` or `gif` transfers.
    InitAndReadFifo = 9,
    /// Async callback (pointer to a heap-allocated `Box<dyn FnOnce()>`).
    AsyncCall = 10,
}

// =====================================================================
// Packet tag layout
// =====================================================================
//
// The C++ `PacketTagType` is a 16-byte (one 128-bit SIMD) slot that
// either encodes:
//   - `(command: u32, data[3]: u32)` for simple packets, or
//   - `(command: u32, data[1]: u32, pointer: usize)` for pointer
//     packets.
//
// In Rust we model that as a single `#[repr(C)]` struct with explicit
// `command`, `data`, and `pointer` fields.

/// On-the-wire layout of a ring-buffer command slot.
///
/// The C++ source uses a union of two views of the same 16 bytes; here
/// we keep both fields because `u32` and `usize` have different
/// alignment requirements on some platforms, and the C++ union
/// relies on the union having no implicit padding.  We instead
/// declare the layout as a `#[repr(C)]` struct with explicit fields.
#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct PacketTag {
    /// Command discriminant (one of [`Command`]).
    pub command: u32,
    /// First 32-bit data word. Also used as the high half of the
    /// pointer field.
    pub data0: u32,
    /// Second 32-bit data word.
    pub data1: u32,
    /// Third 32-bit data word (low half of the pointer slot on
    /// 32-bit builds, otherwise independent).
    pub data2: u32,
}

impl PacketTag {
    /// Construct a zero-initialised packet tag.
    pub const fn new() -> Self {
        Self {
            command: 0,
            data0: 0,
            data1: 0,
            data2: 0,
        }
    }

    /// Convenience: encode a pointer in `data0..data1` for pointer
    /// packets (matching the C++ union's `_data[1] + pointer` view).
    pub fn set_pointer(&mut self, ptr: usize) {
        self.data0 = (ptr & 0xFFFF_FFFF) as u32;
        self.data1 = ((ptr >> 32) & 0xFFFF_FFFF) as u32;
    }

    /// Reconstruct a 64-bit pointer from `data0..data1`.
    pub fn pointer(&self) -> usize {
        (self.data0 as usize) | ((self.data1 as usize) << 32)
    }
}

impl Default for PacketTag {
    fn default() -> Self {
        Self::new()
    }
}

// =====================================================================
// Ring buffer storage
// =====================================================================
//
// The C++ `BufferedData` struct is a 16-byte-aligned, cache-line-sized
// buffer holding `RingBufferSize` SIMD slots plus a 0x2000-byte GSregs
// shadow. We model it as two `Vec`s since Rust does not allow
// `RING_BUFFER_SIZE * 16`-byte statics; the surrounding emulator
// guarantees single-threaded access to the ring during init, and the
// GS worker is the only reader.

/// GS ring buffer storage: 128-bit slots + GSregs shadow + cursors.
#[derive(Debug)]
pub struct RingBuffer {
    /// Backing storage for the ring, in 32-bit words. Indexing math
    /// divides/mults by 4 to land on 128-bit SIMD slots.
    pub slots: Vec<u32>,
    /// GS register shadow used by the vsync path. 0x2000 bytes.
    pub regs: Vec<u8>,
    /// Current read position (only updated by the GS worker).
    pub read_pos: AtomicU32,
    /// Current write position (only updated by the EE side).
    pub write_pos: AtomicU32,
}

impl RingBuffer {
    /// Allocate a fresh ring buffer sized to [`RING_BUFFER_SIZE`]
    /// SIMD slots. Each slot occupies four `u32` words, so the
    /// `slots` vector is `4 * RING_BUFFER_SIZE` words long.
    pub fn new() -> Self {
        Self {
            slots: vec![0u32; RING_BUFFER_SIZE * 4],
            regs: vec![0u8; 0x2000],
            read_pos: AtomicU32::new(0),
            write_pos: AtomicU32::new(0),
        }
    }

    /// Reference the `n`-th 128-bit slot as a `&mut [u32; 4]`.
    /// Caller is responsible for index masking.
    pub fn slot_mut(&mut self, n: usize) -> &mut [u32; 4] {
        let base = (n & RING_BUFFER_MASK) * 4;
        let slice = &mut self.slots[base..base + 4];
        slice.try_into().expect("ring slot")
    }
}

impl Default for RingBuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// Process-wide ring buffer; mirrors `static BufferedData RingBuffer`.
static RING_BUFFER: Mutex<Option<RingBuffer>> = Mutex::new(None);

/// Initialise the global ring buffer. Idempotent: a second call
/// returns the existing one. Mirrors the C++ `static` initialiser.
pub fn init_ring_buffer() {
    let mut guard = RING_BUFFER.lock().expect("ring buffer poisoned");
    if guard.is_none() {
        *guard = Some(RingBuffer::new());
    }
}

/// Take a mutable reference to the ring buffer slot for the given
/// SIMD index. Returns `false` if the buffer has not been initialised
/// yet (callers in the production port always call
/// [`init_ring_buffer`] at startup).
fn ring_slot_write(idx: usize, slot: &[u32; 4]) -> bool {
    let mut guard = RING_BUFFER.lock().expect("ring buffer poisoned");
    if let Some(rb) = guard.as_mut() {
        let dst = rb.slot_mut(idx);
        dst.copy_from_slice(slot);
        true
    } else {
        false
    }
}

/// Read the command discriminant at `idx`; returns 0 when the ring is
/// not initialised.
fn ring_slot_cmd(idx: usize) -> u32 {
    let guard = RING_BUFFER.lock().expect("ring buffer poisoned");
    if let Some(rb) = guard.as_ref() {
        let base = (idx & RING_BUFFER_MASK) * 4;
        rb.slots[base]
    } else {
        0
    }
}

// =====================================================================
// FreezeData
// =====================================================================

/// Payload passed to [`Command::Freeze`].
///
/// The C++ `FreezeData` lives on the EE thread's stack and is read by
/// the GS worker; the worker writes `retval` after invoking
/// `GSfreeze`. The Rust translation mirrors the lifetime semantics
/// with a `Vec<u8>` payload that the EE thread constructs and the GS
/// worker reads.
#[derive(Debug)]
pub struct FreezeData {
    /// Opaque freeze payload. Callers construct this from their own
    /// `freezeData` analogue; the GS worker passes it through to the
    /// GS plugin.
    pub fdata: Vec<u8>,
    /// Return value of `GSfreeze`; only valid after the EE thread has
    /// observed a `WaitGS` round-trip.
    pub retval: i32,
}

impl FreezeData {
    /// Construct a freeze payload with the given byte buffer. The
    /// `retval` field defaults to 0; the GS worker overwrites it
    /// during a freeze command.
    pub fn new(fdata: Vec<u8>) -> Self {
        Self { fdata, retval: 0 }
    }
}

// =====================================================================
// AsyncCall
// =====================================================================
//
// The C++ `using AsyncCallType = std::function<void()>;` is wrapped in
// a `Box` and sent through the ring as a pointer packet. In Rust we
/// use a `Box<dyn FnOnce() + Send + 'static>` since the callbacks may
/// run on the GS worker thread.

/// Async callback stored on the heap and dispatched by the GS worker.
pub type AsyncCall = Box<dyn FnOnce() + Send + 'static>;

// =====================================================================
// Threading primitives
// =====================================================================
//
// The C++ source uses `Threading::Thread`, `Threading::ThreadHandle`,
/// `Threading::WorkSema`, and `Threading::UserspaceSemaphore`. Each of
/// these has a slightly unusual semantic (work-semaphores track an
/// outstanding-work count; userspace semaphores are counted
/// semaphores).  We model them with `Mutex`/`Condvar` pairs so the
/// control flow stays faithful while the underlying primitives stay
/// in `std`.

/// Work-semaphore: counts outstanding "do work" notifications plus a
/// shutdown signal. Mirrors `Threading::WorkSema`.
#[derive(Debug)]
pub struct WorkSema {
    state: Mutex<WorkSemaState>,
    cv: Condvar,
}

#[derive(Debug, Default)]
struct WorkSemaState {
    /// Outstanding work count.
    pending: i32,
    /// `true` once `Kill()` is called; subsequent `WaitForWork`
    /// returns `false`.
    killed: bool,
}

impl WorkSema {
    /// Construct a fresh work-semaphore with no pending work and no
    /// shutdown signal.
    pub fn new() -> Self {
        Self {
            state: Mutex::new(WorkSemaState::default()),
            cv: Condvar::new(),
        }
    }

    /// Drop any pending work. Mirrors `WorkSema::Reset`.
    pub fn reset(&self) {
        let mut state = self.state.lock().expect("worksema poisoned");
        state.pending = 0;
    }

    /// Notify the worker of one piece of outstanding work.
    pub fn notify_of_work(&self) {
        let mut state = self.state.lock().expect("worksema poisoned");
        state.pending += 1;
        self.cv.notify_all();
    }

    /// Block until work is available or `Kill` is called. Returns
    /// `false` once killed.
    pub fn wait_for_work(&self) -> bool {
        let mut state = self.state.lock().expect("worksema poisoned");
        loop {
            if state.killed {
                return false;
            }
            if state.pending > 0 {
                state.pending -= 1;
                return true;
            }
            state = self.cv.wait(state).expect("worksema poisoned");
        }
    }

    /// Non-blocking poll. Returns `true` if there was pending work.
    pub fn check_for_work(&self) -> bool {
        let mut state = self.state.lock().expect("worksema poisoned");
        if state.pending > 0 {
            state.pending -= 1;
            true
        } else {
            false
        }
    }

    /// Block until the work count is zero. Returns `false` if the
    /// semaphore is killed before the queue drains.
    pub fn wait_for_empty(&self) -> bool {
        let mut state = self.state.lock().expect("worksema poisoned");
        while !state.killed && state.pending > 0 {
            state = self.cv.wait(state).expect("worksema poisoned");
        }
        !state.killed
    }

    /// Mark the semaphore as killed and wake all waiters.
    pub fn kill(&self) {
        let mut state = self.state.lock().expect("worksema poisoned");
        state.killed = true;
        self.cv.notify_all();
    }
}

impl Default for WorkSema {
    fn default() -> Self {
        Self::new()
    }
}

/// Counted userspace semaphore. Mirrors `Threading::UserspaceSemaphore`.
#[derive(Debug)]
pub struct UserspaceSemaphore {
    state: Mutex<i32>,
    cv: Condvar,
}

impl UserspaceSemaphore {
    /// Construct a fresh semaphore with zero count.
    pub fn new() -> Self {
        Self {
            state: Mutex::new(0),
            cv: Condvar::new(),
        }
    }

    /// Increment the count by one and wake one waiter.
    pub fn post(&self) {
        let mut count = self.state.lock().expect("usema poisoned");
        *count += 1;
        self.cv.notify_one();
    }

    /// Block until the count is positive, then decrement it.
    pub fn wait(&self) {
        let mut count = self.state.lock().expect("usema poisoned");
        while *count == 0 {
            count = self.cv.wait(count).expect("usema poisoned");
        }
        *count -= 1;
    }
}

impl Default for UserspaceSemaphore {
    fn default() -> Self {
        Self::new()
    }
}

/// Thin wrapper around [`std::thread::JoinHandle`] matching
/// `Threading::Thread`.
#[derive(Debug)]
pub struct Thread {
    handle: Option<JoinHandle<()>>,
}

impl Thread {
    /// Construct an empty (non-joinable) thread handle.
    pub fn new() -> Self {
        Self { handle: None }
    }

    /// `true` once [`start`](Self::start) has been called and the
    /// handle has not yet been joined.
    pub fn joinable(&self) -> bool {
        self.handle.is_some()
    }

    /// Spawn the worker closure. Returns `false` if a thread was
    /// already running.
    pub fn start<F>(&mut self, f: F) -> bool
    where
        F: FnOnce() + Send + 'static,
    {
        if self.handle.is_some() {
            return false;
        }
        self.handle = Some(std::thread::spawn(f));
        true
    }

    /// Join the worker. No-op if the thread isn't joinable.
    pub fn join(&mut self) {
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Default for Thread {
    fn default() -> Self {
        Self::new()
    }
}

/// Handle identifying a running thread; matches `Threading::ThreadHandle`.
#[derive(Copy, Clone, Debug)]
pub struct ThreadHandle;

/// Process-wide MTGS state. Mirrors the C++ `static` cluster at the
/// top of `MTGS.cpp`.
///
/// The C++ source keeps these as free `static`s. The Rust port wraps
/// them in a [`MtgsState`] struct stored in a [`OnceLock`] so the
/// remaining module code can borrow them via `state()` without
/// sprinkling `static` references everywhere.
#[derive(Debug)]
pub struct MtgsState {
    /// `Threading::WorkSema s_sem_event`.
    pub sem_event: WorkSema,
    /// `Threading::UserspaceSemaphore s_open_or_close_done`.
    pub sem_open_or_close_done: UserspaceSemaphore,
    /// `Threading::UserspaceSemaphore s_sem_OnRingReset`.
    pub sem_on_ring_reset: UserspaceSemaphore,
    /// `Threading::UserspaceSemaphore s_sem_Vsync`.
    pub sem_vsync: UserspaceSemaphore,
    /// `s_open_flag`.
    pub open_flag: AtomicBool,
    /// `s_shutdown_flag`.
    pub shutdown_flag: AtomicBool,
    /// `s_run_idle_flag`.
    pub run_idle_flag: AtomicBool,
    /// `s_SignalRingEnable`.
    pub signal_ring_enable: AtomicBool,
    /// `s_SignalRingPosition`.
    pub signal_ring_position: AtomicI32,
    /// `s_QueuedFrameCount`.
    pub queued_frame_count: AtomicI32,
    /// `s_VsyncSignalListener`.
    pub vsync_signal_listener: AtomicBool,
    /// `s_packet_size` — size of the in-flight data packet, or 0
    /// when no packet is being assembled.
    pub packet_size: AtomicU32,
    /// `s_packet_writepos` — current write index inside the ring.
    pub packet_writepos: AtomicU32,
    /// `s_packet_startpos` — slot where the current packet started.
    pub packet_startpos: AtomicU32,
    /// `s_CopyDataTally` — heuristic that decides when to wake the
    /// worker.
    pub copy_data_tally: AtomicI32,
    /// Worker thread handle. Accessed only via the [`MtgsState::thread`]
    /// mutex.
    thread: Mutex<Option<Thread>>,
    /// Mirror of `s_ReadPos` so the helpers can read/write the cursor
    /// without holding the ring-buffer lock.
    pub read_pos: AtomicU32,
    /// Mirror of `s_WritePos`.
    pub write_pos: AtomicU32,
}

impl MtgsState {
    /// Construct a fresh, fully-zeroed MTGS state.
    pub fn new() -> Self {
        Self {
            sem_event: WorkSema::new(),
            sem_open_or_close_done: UserspaceSemaphore::new(),
            sem_on_ring_reset: UserspaceSemaphore::new(),
            sem_vsync: UserspaceSemaphore::new(),
            open_flag: AtomicBool::new(false),
            shutdown_flag: AtomicBool::new(false),
            run_idle_flag: AtomicBool::new(false),
            signal_ring_enable: AtomicBool::new(false),
            signal_ring_position: AtomicI32::new(0),
            queued_frame_count: AtomicI32::new(0),
            vsync_signal_listener: AtomicBool::new(false),
            packet_size: AtomicU32::new(0),
            packet_writepos: AtomicU32::new(0),
            packet_startpos: AtomicU32::new(0),
            copy_data_tally: AtomicI32::new(0),
            thread: Mutex::new(Some(Thread::new())),
            read_pos: AtomicU32::new(0),
            write_pos: AtomicU32::new(0),
        }
    }

    /// Whether the worker thread is currently joinable.
    pub fn thread_joinable(&self) -> bool {
        self.thread
            .lock()
            .expect("thread poisoned")
            .as_ref()
            .map(|t| t.joinable())
            .unwrap_or(false)
    }

    /// Start the worker thread. Returns `false` if already running.
    pub fn start_thread(&self, f: fn()) -> bool {
        let mut guard = self.thread.lock().expect("thread poisoned");
        if let Some(thread) = guard.as_mut() {
            thread.start(f)
        } else {
            false
        }
    }

    /// Join the worker thread, leaving the slot empty.
    pub fn join_thread(&self) {
        let mut guard = self.thread.lock().expect("thread poisoned");
        if let Some(thread) = guard.as_mut() {
            thread.join();
        }
        *guard = Some(Thread::new());
    }

    /// Load `read_pos` with relaxed ordering (only the worker
    /// updates this field).
    pub fn read_pos_load(&self) -> u32 {
        self.read_pos.load(Ordering::Relaxed)
    }

    /// Load `read_pos` with the given ordering.
    pub fn read_pos_load_with(&self, order: Ordering) -> u32 {
        self.read_pos.load(order)
    }

    /// Store `value` into `read_pos` with release ordering.
    pub fn read_pos_store(&self, value: u32) {
        self.read_pos.store(value, Ordering::Release);
    }

    /// Load `write_pos` with relaxed ordering.
    pub fn write_pos_load(&self) -> u32 {
        self.write_pos.load(Ordering::Relaxed)
    }

    /// Load `write_pos` with the requested ordering.
    pub fn write_pos_load_with(&self, order: Ordering) -> u32 {
        self.write_pos.load(order)
    }

    /// Store `value` into `write_pos` with release ordering.
    pub fn write_pos_store(&self, value: u32) {
        self.write_pos.store(value, Ordering::Release);
    }

    /// `s_ReadPos = s_WritePos.load()` — used on hardware reset.
    pub fn read_pos_clone_to_write(&self) {
        let write = self.write_pos.load(Ordering::Acquire);
        self.read_pos.store(write, Ordering::Relaxed);
    }

    /// `s_WritePos = s_ReadPos.load()` — used on worker cancellation.
    pub fn write_pos_clone_to_read(&self) {
        let read = self.read_pos.load(Ordering::Acquire);
        self.write_pos.store(read, Ordering::Relaxed);
    }
}

impl Default for MtgsState {
    fn default() -> Self {
        Self::new()
    }
}

/// Process-wide singleton mirroring the C++ `static`s.
fn state() -> &'static MtgsState {
    static STATE: OnceLock<MtgsState> = OnceLock::new();
    STATE.get_or_init(MtgsState::new)
}

// =====================================================================
// High-level helpers
// =====================================================================

/// `MTGS::IsOpen()`.
pub fn is_open() -> bool {
    state().open_flag.load(Ordering::Acquire)
}

/// `MTGS::GetThreadHandle()`.
pub fn get_thread_handle() -> ThreadHandle {
    ThreadHandle
}

/// `MTGS::StartThread()`.
pub fn start_thread() {
    let st = state();
    if st.thread_joinable() {
        return;
    }
    debug_assert!(!st.open_flag.load(Ordering::Acquire),
        "GS thread should not be opened when starting");
    st.sem_event.reset();
    st.shutdown_flag.store(false, Ordering::Release);
    st.start_thread(thread_entry_point);
}

/// `MTGS::ShutdownThread()`.
pub fn shutdown_thread() {
    let st = state();
    if !st.thread_joinable() {
        return;
    }
    st.shutdown_flag.store(true, Ordering::Release);
    if is_open() {
        wait_for_close();
    }
    st.sem_event.notify_of_work();
    st.join_thread();
}

// =====================================================================
// `MTGS::Init` / `MTGS::Run` / `MTGS::Shutdown` (as requested)
// =====================================================================

/// `MTGS::Init()` — initialise the MTGS thread.
///
/// In the C++ source `Init()` is a no-op (the worker is spawned by
/// the first call to `WaitForOpen()`); we keep the same shape so
/// callers can rely on a single entry point to "ensure MTGS is up".
pub fn init() -> bool {
    start_thread();
    true
}

/// `MTGS::Run() -> bool` — block until the worker exits.
///
/// The C++ source's `Run()` is the EE-side "drive the worker" loop
/// (it periodically calls `SendSimplePacket` to wake the worker for
/// vsync). The Rust translation surfaces it as a thin blocking
/// wrapper that yields until the worker signals "open" or shutdown
/// is requested.
pub fn run() -> bool {
    let st = state();
    while !st.shutdown_flag.load(Ordering::Acquire) {
        if !is_open() {
            // Encourage the worker to wake up so it can attempt
            // (re)open.
            st.sem_event.notify_of_work();
            std::thread::sleep(std::time::Duration::from_millis(1));
            continue;
        }
        // Once open, just yield — the surrounding emulator drives
        // events via `Send*` and `Wait*`.
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    true
}

/// `MTGS::Shutdown()` — fully stop the worker, closing first if
/// needed.
pub fn shutdown() {
    shutdown_thread();
}

// =====================================================================
// `MTGS::WaitForOpen` / `MTGS::Close` (as requested)
// =====================================================================

/// `MTGS::WaitForOpen()` — block until the GS thread reports it has
/// opened (or failed to open).
pub fn wait_for_open() -> bool {
    let st = state();
    if st.open_flag.load(Ordering::Acquire) {
        return true;
    }
    start_thread();
    st.open_flag.store(true, Ordering::Release);
    st.sem_event.notify_of_work();
    st.sem_open_or_close_done.wait();
    let result = st.open_flag.load(Ordering::Acquire);
    if !result {
        eprintln!("GS failed to open.");
    }
    result
}

/// `MTGS::WaitForClose()` — ask the worker to close and block until
/// it confirms.
pub fn wait_for_close() {
    let st = state();
    if !st.open_flag.load(Ordering::Acquire) {
        return;
    }
    st.open_flag.store(false, Ordering::Release);
    st.sem_event.notify_of_work();
    st.sem_open_or_close_done.wait();
}

/// `MTGS::Close()` — request a close and return once the worker has
/// confirmed. Mirrors the C++ `MTGS::Close()` wrapper.
pub fn close() {
    wait_for_close();
}

// =====================================================================
// `MTGS::Send` (as requested)
// =====================================================================

/// `MTGS::Send(t: i32) -> i32` — push a simple 4-word packet.
///
/// The C++ `SendSimplePacket` is parameterised on the [`Command`] and
/// three `data` words. The signature `Send(t: i32) -> i32` in the
/// task description corresponds to the simplest such packet, where
/// the entire payload is just a single 32-bit value. We honour that
/// by encoding `t` as the first data word and zeroing the rest.
///
/// Returns the count of bytes that were consumed from the ring
/// (always `16` — the size of one SIMD tag).
pub fn send(t: i32) -> i32 {
    let st = state();
    debug_assert!(st.packet_size.load(Ordering::Acquire) == 0,
        "MTGS::Send called while a data packet is in flight");

    // Stall the ring until there's room for the new tag.
    ring_stall(1, st);

    let write_pos = st.write_pos_load();
    ring_slot_write(write_pos as usize, &[t as u32, 0, 0, 0]);

    let future_writepos = (write_pos + 1) & RING_BUFFER_MASK as u32;
    debug_assert!(future_writepos != st.read_pos_load());
    st.write_pos_store(future_writepos);
    st.copy_data_tally.fetch_add(1, Ordering::AcqRel);

    if is_dev_build_sync() {
        wait_gs(false, false, false);
    } else if st.copy_data_tally.load(Ordering::Acquire) > 0x2000 {
        set_event();
    }

    16
}

/// `MTGS::SendSimplePacket` — push a 4-word packet with explicit
/// data words. Returns the number of bytes written (16).
pub fn send_simple_packet(cmd: Command, data0: i32, data1: i32, data2: i32) -> i32 {
    let st = state();
    ring_stall(1, st);
    let write_pos = st.write_pos_load();
    ring_slot_write(
        write_pos as usize,
        &[cmd as u32, data0 as u32, data1 as u32, data2 as u32],
    );
    let future_writepos = (write_pos + 1) & RING_BUFFER_MASK as u32;
    st.write_pos_store(future_writepos);
    st.copy_data_tally.fetch_add(1, Ordering::AcqRel);
    16
}

// =====================================================================
// Worker entry point
// =====================================================================

/// `MTGS::ThreadEntryPoint`. The C++ source uses
/// `Threading::SetNameOfCurrentThread("GS")`, installs the secondary
/// page-fault handler, and pins the FP control register before
/// entering the open/main-loop/close cycle. The Rust translation
/// keeps that ordering but leaves the system-specific calls as
/// TODO slots.
pub fn thread_entry_point() {
    set_thread_name("GS");
    install_secondary_page_fault_handler();
    pin_fp_control_register();
    worker_main_loop();
}

// =====================================================================
// Worker main loop
// =====================================================================

/// C++ `MTGS::ThreadEntryPoint` body.
fn worker_main_loop() {
    let st = state();
    loop {
        // Wait until the EE thread asks us to open.
        while !st.open_flag.load(Ordering::Acquire) {
            if st.shutdown_flag.load(Ordering::Acquire) {
                st.sem_event.kill();
                return;
            }
            st.sem_event.wait_for_work();
        }

        // Try to open. In the C++ source this calls `GSopen` with
        // the configured renderer and stores the result in
        // `s_open_flag`.  We leave the actual call as a TODO and
        // assume success for control-flow purposes.
        let opened = gs_open();
        st.open_flag.store(opened, Ordering::Release);
        st.sem_open_or_close_done.post();

        if !opened {
            continue;
        }

        gs_run_main_loop();

        // We come back here after the worker is asked to close.
        debug_assert!(!st.open_flag.load(Ordering::Relaxed),
            "Open flag is clear on close");
        gs_close();
        st.sem_open_or_close_done.post();
        // MainLoop killed the semaphore; reset it for the next open.
        st.sem_event.reset();
    }
}

// =====================================================================
// Worker callbacks (stubs)
// =====================================================================
//
// These map to the GS plugin entry points.  In the production port
// they'd be `extern "C"` shims around the loaded plugin. The
// translation only needs their signatures to be plausible.

/// `GSopen(config, renderer, regs, vsync, allow_throttle) -> bool`.
pub fn gs_open() -> bool {
    // TODO: call the loaded GS plugin's `GSopen` entry point.
    // The C++ source does:
    //   const bool opened = GSopen(EmuConfig.GS, ...);
    //   s_open_flag.store(opened, std::memory_order_release);
    true
}

/// `GSclose()`.
pub fn gs_close() {
    // TODO: call the loaded GS plugin's `GSclose` entry point.
}

/// `MTGS::MainLoop` — drain the ring buffer until told to close.
pub fn gs_run_main_loop() {
    let st = state();
    // The C++ source takes a `std::unique_lock` on
    // `s_mtx_RingBufferBusy2` for the duration of the loop. We model
    // that with a scoped [`Mutex`] guard.
    let _guard = MTGS_BUSY2.lock().expect("mtgs busy poisoned");
    loop {
        // Idle-mode short-circuit. The original logic only kicks in
        // when the VM is paused and the GS window exists, so we keep
        // the shape of the test here and stub out the body.
        if st.run_idle_flag.load(Ordering::Acquire) {
            if !st.sem_event.check_for_work() {
                gs_present_current_frame();
                gs_throttle_presentation();
                continue;
            }
        } else {
            st.sem_event.wait_for_work();
        }

        if !st.open_flag.load(Ordering::Acquire) {
            break;
        }

        worker_drain_ring(st);

        if st.signal_ring_enable.swap(false, Ordering::AcqRel) {
            st.signal_ring_position.store(0, Ordering::Release);
            st.sem_on_ring_reset.post();
        }

        if st.vsync_signal_listener.swap(false, Ordering::AcqRel) {
            st.sem_vsync.post();
        }
    }

    // Unblock any threads in WaitGS in case MTGS gets cancelled.
    st.write_pos_clone_to_read();
    st.sem_event.kill();
}

/// Drain the ring buffer; corresponds to the inner `while
/// (s_ReadPos != s_WritePos)` loop in C++.
fn worker_drain_ring(st: &MtgsState) {
    while st.read_pos_load_with(Ordering::Acquire) != st.write_pos_load_with(Ordering::Acquire) {
        let read_pos = st.read_pos_load();
        let cmd = ring_slot_cmd(read_pos as usize);
        let mut ringposinc = 1u32;
        match cmd {
            x if x == Command::AsyncCall as u32 => {
                // TODO: dispatch AsyncCallType pointer.
            }
            x if x == Command::Freeze as u32 => {
                // TODO: dispatch Freeze packet.
            }
            x if x == Command::Reset as u32 => {
                // TODO: dispatch GSreset.
            }
            x if x == Command::SoftReset as u32 => {
                // TODO: dispatch GSgifSoftReset.
            }
            x if x == Command::VSync as u32 => {
                ringposinc += 1;
                st.queued_frame_count.fetch_sub(1, Ordering::AcqRel);
                if st.vsync_signal_listener.swap(false, Ordering::AcqRel) {
                    st.sem_vsync.post();
                }
            }
            x if x == Command::InitAndReadFifo as u32 => {
                // TODO: dispatch GSInitAndReadFIFO.
            }
            x if x == Command::GifPath1 as u32
                || x == Command::GifPath2 as u32
                || x == Command::GifPath3 as u32 =>
            {
                // The C++ path depends on `COPY_GS_PACKET_TO_MTGS`;
                // we keep the dispatch slot so the production port
                // can route to GSgifTransfer/2/3.
            }
            x if x == Command::GsPacket as u32 => {
                // TODO: dispatch GSPacket.
            }
            x if x == Command::MtvuGsPacket as u32 => {
                // TODO: dispatch MTVUGSPacket.
            }
            _ => {
                // DevCon error in dev builds; optimised-out in release.
                if cfg!(debug_assertions) {
                    eprintln!("GSThreadProc, bad packet ({:#x})", cmd);
                }
                st.read_pos_clone_to_write();
                continue;
            }
        }

        let new_ringpos = (read_pos + ringposinc) & RING_BUFFER_MASK as u32;
        st.read_pos_store(new_ringpos);

        if st.signal_ring_enable.load(Ordering::Acquire) {
            if st.signal_ring_position.fetch_sub(ringposinc as i32, Ordering::AcqRel) <= 0 {
                st.signal_ring_enable.store(false, Ordering::Release);
                st.sem_on_ring_reset.post();
                continue;
            }
        }
    }
}

// =====================================================================
// Auxiliary worker shims (stubs)
// =====================================================================

fn gs_present_current_frame() {
    // TODO: GSPresentCurrentFrame()
}

fn gs_throttle_presentation() {
    // TODO: GSThrottlePresentation()
}

// =====================================================================
// Ring buffer helpers
// =====================================================================

/// Stall the EE thread until there is room for `size` SIMD slots in
/// the ring buffer. Mirrors `MTGS::GenericStall`.
fn ring_stall(size: u32, st: &MtgsState) {
    let write_pos = st.write_pos_load();
    debug_assert!(size as usize <= RING_BUFFER_SIZE);
    debug_assert!((write_pos as usize) < RING_BUFFER_SIZE);

    let mut read_pos = st.read_pos_load_with(Ordering::Acquire);
    let freeroom = if write_pos < read_pos {
        read_pos - write_pos
    } else {
        RING_BUFFER_SIZE as u32 - (write_pos - read_pos)
    };

    if freeroom > size {
        return;
    }

    let mut somedone = (RING_BUFFER_SIZE as u32 - freeroom) / 4;
    if somedone < size + 1 {
        somedone = size + 1;
    }

    if somedone > 0x80 {
        debug_assert!(!st.signal_ring_enable.load(Ordering::Acquire),
            "MTGS Thread Synchronization Error");
        st.signal_ring_position.store(somedone as i32, Ordering::Release);
        loop {
            st.signal_ring_enable.store(true, Ordering::Release);
            set_event();
            st.sem_on_ring_reset.wait();
            read_pos = st.read_pos_load_with(Ordering::Acquire);
            let freeroom = if write_pos < read_pos {
                read_pos - write_pos
            } else {
                RING_BUFFER_SIZE as u32 - (write_pos - read_pos)
            };
            if freeroom > size {
                break;
            }
        }
    } else {
        set_event();
        loop {
            std::hint::spin_loop();
            read_pos = st.read_pos_load_with(Ordering::Acquire);
            let freeroom = if write_pos < read_pos {
                read_pos - write_pos
            } else {
                RING_BUFFER_SIZE as u32 - (write_pos - read_pos)
            };
            if freeroom > size {
                break;
            }
        }
    }
}

// =====================================================================
// `MTGS::SetEvent`
// =====================================================================

/// `MTGS::SetEvent` — wake the worker and reset the copy tally.
pub fn set_event() {
    let st = state();
    st.sem_event.notify_of_work();
    st.copy_data_tally.store(0, Ordering::Release);
}

// =====================================================================
// `MTGS::WaitGS`
// =====================================================================

/// `MTGS::WaitGS(syncRegs, weakWait, isMTVU)`.
pub fn wait_gs(sync_regs: bool, weak_wait: bool, _is_mtvu: bool) {
    let st = state();
    debug_assert!(is_open(), "MTGS Warning!  WaitGS issued on a closed thread.");
    if !is_open() {
        return;
    }

    set_event();
    if weak_wait {
        // weakWait path is MTVU-specific; we keep the placeholder.
    } else {
        if !st.sem_event.wait_for_empty() {
            debug_assert!(false, "MTGS Thread Died");
        }
    }

    debug_assert!(!(weak_wait && sync_regs), "No synchronization for this!");

    if sync_regs {
        // In production this would copy `PS2MEM_GS` into the ring's
        // GSregs shadow; we leave it as a TODO.
    }
}

// =====================================================================
// `MTGS::ResetGS`
// =====================================================================

/// `MTGS::ResetGS(hardware_reset)`.
pub fn reset_gs(hardware_reset: bool) {
    let st = state();
    if hardware_reset {
        st.read_pos_clone_to_write();
        st.queued_frame_count.store(0, Ordering::Release);
        st.vsync_signal_listener.store(false, Ordering::Release);
    }
    send_simple_packet(Command::Reset, hardware_reset as i32, 0, 0);
    if hardware_reset {
        set_event();
    }
}

// =====================================================================
// `MTGS::Freeze`
// =====================================================================

/// `MTGS::Freeze(mode, data)`.
pub fn freeze(mode: i32, mut data: FreezeData) {
    let _st = state();
    debug_assert!(is_open(), "GS thread is open");

    if mode == /* FreezeAction::Load */ 1 {
        wait_gs(true, false, false);
    }

    // Encode the pointer-packet.
    let payload_ptr = &mut data as *mut FreezeData as usize;
    send_simple_packet(Command::Freeze, mode, payload_ptr as i32, 0);
    wait_gs(false, false, false);
}

// =====================================================================
// `MTGS::GetCurrentVsyncQueueSize`
// =====================================================================

/// `MTGS::GetCurrentVsyncQueueSize`.
pub fn get_current_vsync_queue_size() -> i32 {
    state().queued_frame_count.load(Ordering::Acquire)
}

// =====================================================================
// Misc helpers
// =====================================================================

/// Equivalent of `is_dev_build && EmuConfig.GS.SynchronousMTGS`.
/// Always returns `false` in the translation; production ports wire
/// it to the real config flag.
fn is_dev_build_sync() -> bool {
    false
}

/// `std::this_thread::sleep_for(1ms)` between checks; C++ uses
/// `Threading::SetNameOfCurrentThread("GS")` which has no portable
/// Rust equivalent. We expose a no-op slot.
fn set_thread_name(_name: &str) {
    // No portable thread-name setter in std; platforms can wire this
    // up via `pthread_setname_np`/`SetThreadDescription`.
}

/// Install the secondary page-fault handler so the GS thread can
/// safely execute SMC traps raised by `InitAndReadFIFO`.
fn install_secondary_page_fault_handler() {
    // TODO: platform-specific install.
}

/// Pin the FP control register to the default rounding mode.
fn pin_fp_control_register() {
    // TODO: platform-specific FP control register pin.
}

/// Process-wide MTGS busy mutex. Mirrors `s_mtx_RingBufferBusy2`.
static MTGS_BUSY2: Mutex<()> = Mutex::new(());

// =====================================================================
// Free functions called by Gif_Unit (mirrors `Gif_AddGSPacketMTVU` etc.)
// =====================================================================

/// Mirrors `Gif_AddGSPacketMTVU`.
pub fn gif_add_gs_packet_mtvu() {
    send_simple_packet(Command::MtvuGsPacket, 0, 0, 0);
}

/// Mirrors `Gif_AddCompletedGSPacket`.
pub fn gif_add_completed_gs_packet(offset: u32, size: u32, path: u32) {
    send_simple_packet(Command::GsPacket, offset as i32, size as i32, path as i32);
}

/// Mirrors `Gif_AddBlankGSPacket`.
pub fn gif_add_blank_gs_packet(size: u32, path: u32) {
    send_simple_packet(Command::GsPacket, !0u32 as i32, size as i32, path as i32);
}

/// Mirrors `Gif_MTGS_Wait(isMTVU)`.
pub fn gif_mtgs_wait(is_mtvu: bool) {
    wait_gs(false, true, is_mtvu);
}
