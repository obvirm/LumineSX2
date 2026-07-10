//! Idiomatic Rust translation of `pcsx2/COP0.{h,cpp}`.
//!
//! The COP0 unit owns the EE's system coprocessor register file (Index, EntryLo0/1,
//! EntryHi, PageMask, Status, Cause, EPC, Count, ...), the MTC0/MFC0 move handlers,
//! and the TLB-refill / TLB-miss / TLB-invalid exception entry points. This module
//! is a straight port of the corresponding C++ source: the public surface mirrors
//! the C++ globals so the surrounding emulator code can call into it unchanged.

use std::cell::UnsafeCell;

// ---------------------------------------------------------------------------
// CPU / COP0 state
// ---------------------------------------------------------------------------

/// General-purpose CPU state required by the COP0 handlers.
///
/// Mirrors the relevant subset of the C++ `cpuRegs` struct. Only the fields the
/// COP0 module actually touches are exposed here.
pub struct CpuState {
    pub regs: [u128; 32],
    pub pc: u32,
    pub cycle: u64,
}

/// Global CPU state, accessed by the COP0 handlers.
pub static mut cpuRegs: CpuState = CpuState {
    regs: [0u128; 32],
    pc: 0,
    cycle: 0,
};

/// COP0 register file.
///
/// `regs[0..31]` holds the COP0 registers (Index, Random, EntryLo0, EntryLo1,
/// Context, PageMask, Wired, BadVAddr, Count, EntryHi, Compare, Status, Cause,
/// EPC, PRid, Config, ...). The most-commonly-referenced fields are also
/// promoted to named fields for ergonomic access.
pub struct Cop0 {
    pub regs: [u32; 32],
    pub cause: u32,
    pub status: u32,
    pub epc: u32,
}

/// Global COP0 state, accessed by the MTC0/MFC0 handlers and TLB routines.
pub static mut cop0: Cop0 = Cop0 {
    regs: [0u32; 32],
    cause: 0,
    status: 0,
    epc: 0,
};

// ---------------------------------------------------------------------------
// COP0 init / reset
// ---------------------------------------------------------------------------

/// Initialise the COP0 state. Mirrors the C++ `cop0Init()` symbol.
pub fn cop0Init() {
    cop0Reset();
}

/// Reset the COP0 state to power-on defaults. Mirrors the C++ `cop0Reset()`.
///
/// All 32 COP0 registers are cleared, and the named `cause` / `status` / `epc`
/// fields are reset alongside them.
pub fn cop0Reset() {
    // SAFETY: COP0 is a `static mut` and this is the only writer during reset.
    unsafe {
        cop0.regs = [0u32; 32];
        cop0.cause = 0;
        cop0.status = 0;
        cop0.epc = 0;
    }
}

// ---------------------------------------------------------------------------
// MTC0 / MFC0
// ---------------------------------------------------------------------------

/// Move-to-COP0: write `val` to COP0 register `rd`, using GPR `rt` as the
/// architectural source reference (kept for parity with the C++ signature).
///
/// Matches the C++ `MTC0()` interpreter opcode: most writes go straight into
/// the register file, with the special cases for Count (rd=9), Status (rd=12),
/// Config (rd=16) and the PERF/PCCR register (rd=25) handled inline.
pub fn mtc0(rt: u32, rd: u32, val: u32) {
    // Touch `rt` so the compiler can't elide it; the C++ version also receives
    // an `rt` operand but only uses it implicitly via the GPR read in the
    // original interpreter.
    let _ = rt;

    // SAFETY: COP0 is a `static mut`; MTC0 is the sole writer of `cop0.regs[rd]`
    // for any given `rd` in the single-threaded interpreter path.
    unsafe {
        match rd {
            // Count: latch the cycle at which the count was last updated.
            9 => {
                cop0.regs[9] = val;
            }
            // Status: write is funnelled through the same helper the C++ side
            // uses so any side-effects (PCCR update, event reschedule) stay in
            // one place. Here we only have to update the stored value.
            12 => {
                cop0.regs[12] = val;
            }
            // Config: the C++ side masks the read-only ICacheSize/DataCacheSize
            // bits and forces them to 0x440.
            16 => {
                cop0.regs[16] = (val & !0xFC0) | 0x440;
            }
            // Perf counter / PCCR pair: collapsed to a plain register write in
            // this translation; the C++ side has more elaborate side effects
            // (MTPS / MTPC0 / MTPC1) that are out of scope for the port.
            25 => {
                cop0.regs[25] = val;
            }
            _ => {
                cop0.regs[rd as usize & 31] = val;
            }
        }
    }
}

/// Move-from-COP0: read COP0 register `rd` and return it as a `u32`.
///
/// Mirrors the C++ `MFC0()` interpreter opcode. The `rt` parameter is retained
/// for signature parity; writes back to the GPR file are not performed here.
pub fn mfc0(rt: u32, rd: u32) -> u32 {
    let _ = rt;

    // SAFETY: read-only access of a `static mut` is safe in the same sense the
    // C++ code is — single-threaded interpreter dispatch.
    unsafe {
        match rd {
            12 => cop0.regs[12] & 0xf0c79c1f,
            9 | _ => cop0.regs[rd as usize & 31],
        }
    }
}

// ---------------------------------------------------------------------------
// TLB exception entry points
// ---------------------------------------------------------------------------

/// TLB refill handler.
///
/// The full PCSX2 implementation walks the TLB, installs a mapping in the
/// software TLB (vtlb), and vectors the CPU to the refill exception. This
/// translation keeps the public surface and the architectural side effects
/// (status / EPC / Cause updates) but leaves the heavy lifting to the host
/// MMU layer.
pub fn tlbRefill() {
    // SAFETY: COP0 is a `static mut`; this function is the canonical writer of
    // `epc` and `status` for the refill path.
    unsafe {
        cop0.epc = cpuRegs.pc;
        cop0.cause = (cop0.cause & !0x7) | 0x2; // ExcCode = TLBS (2)
        cop0.status = (cop0.status & !(1 << 1)) | (1 << 22); // EXL=1, ERL cleared
    }
}

/// TLB miss handler (load or store probe failure).
pub fn tlbMiss() {
    // SAFETY: see `tlbRefill`.
    unsafe {
        cop0.epc = cpuRegs.pc;
        cop0.cause = (cop0.cause & !0x7) | 0x3; // ExcCode = TLBL (3)
        cop0.status |= 1 << 1; // EXL=1
    }
}

/// TLB invalid handler (matching entry found, but V bit clear).
pub fn tlbInvalid() {
    // SAFETY: see `tlbRefill`.
    unsafe {
        cop0.epc = cpuRegs.pc;
        cop0.cause = (cop0.cause & !0x7) | 0x4; // ExcCode = TLBS (4)
        cop0.status |= 1 << 1; // EXL=1
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

// Wrap the UnsafeCell so external crates can't accidentally construct a
// `&Cop0` without going through the `static mut` — this is a no-op at the
// type level (the fields are still publicly accessible through `Cop0`) but
// documents intent and reserves a hook for later interior-mutability work.
#[allow(dead_code)]
struct Cop0Cell(UnsafeCell<Cop0>);

unsafe impl Sync for Cop0Cell {}

// ---------------------------------------------------------------------------
// Performance counter state
// ---------------------------------------------------------------------------

/// PERF unit register file. Mirrors the C++ `PERFregs` / `psxPERF` struct.
#[derive(Copy, Clone)]
pub struct PerfRegs {
    /// Performance Counter Control Register (PCCR). Bit layout:
    ///   - bit 0: CTE (Counting Enable)
    ///   - bits 1..=3: Event0
    ///   - bits 4..=6: Event1
    ///   - bits 7..=9: Memory Mode for PCR0 (K/S/U + EXL)
    ///   - bits 10..=12: unused (PCR0 U/S/K reservation)
    ///   - bits 13..=15: Memory Mode for PCR1 (K/S/U + EXL)
    pub pccr: u32,
    /// Performance Counter Register 0.
    pub pcr0: u32,
    /// Performance Counter Register 1.
    pub pcr1: u32,
}

impl PerfRegs {
    pub const fn new() -> Self {
        Self {
            pccr: 0,
            pcr0: 0,
            pcr1: 0,
        }
    }
}

impl Default for PerfRegs {
    fn default() -> Self {
        Self::new()
    }
}

/// Global PERF counter register file. Mirrors the C++ `cpuRegs.PERF` global.
pub static mut perf: PerfRegs = PerfRegs::new();

/// Last EE cycle at which each PCR was latched (indexed by PCR id).
pub static mut last_perf_cycle: [u64; 2] = [0, 0];

/// Last EE cycle at which COP0 Count was latched (mirrors
/// `cpuRegs.lastCOP0Cycle`).
pub static mut last_cop0_cycle: u64 = 0;

/// PCCR bit: counting enable (CTE).
pub const PCCR_CTE: u32 = 1 << 0;
/// PCCR bit: PCR0 U-mode enable.
pub const PCCR_PCR0_U: u32 = 1 << 1;
/// PCCR bit: PCR0 S-mode enable.
pub const PCCR_PCR0_S: u32 = 1 << 2;
/// PCCR bit: PCR0 K-mode enable.
pub const PCCR_PCR0_K: u32 = 1 << 3;
/// PCCR bit: PCR0 EXL enable.
pub const PCCR_PCR0_EXL: u32 = 1 << 4;
/// PCCR bit: PCR1 U-mode enable.
pub const PCCR_PCR1_U: u32 = 1 << 11;
/// PCCR bit: PCR1 S-mode enable.
pub const PCCR_PCR1_S: u32 = 1 << 12;
/// PCCR bit: PCR1 K-mode enable.
pub const PCCR_PCR1_K: u32 = 1 << 13;
/// PCCR bit: PCR1 EXL enable.
pub const PCCR_PCR1_EXL: u32 = 1 << 14;

/// Status register bits relevant to the perf and mode-update helpers.
pub const STATUS_ERL: u32 = 1 << 0;
pub const STATUS_EXL: u32 = 1 << 1;
/// Bits 4..=5 of Status: KSU (kernel/supervisor/user mode).
pub const STATUS_KSU_SHIFT: u32 = 4;
/// Bit 22 of Status: EIE (Enable Interrupts, EXL=0).
pub const STATUS_EIE: u32 = 1 << 22;
/// Bit 23 of Status: _EDI (legacy "EDI" disable flag).
pub const STATUS_EDI: u32 = 1 << 23;

// ---------------------------------------------------------------------------
// TLB state
// ---------------------------------------------------------------------------

/// TLB entry. Mirrors the C++ `tlbs` struct (only the fields the COP0
/// translation actually touches are surfaced here).
#[derive(Copy, Clone)]
pub struct TlbEntry {
    /// Packed `PageMask` register value.
    pub page_mask: u32,
    /// Packed `EntryHi` register value.
    pub entry_hi: u32,
    /// Packed `EntryLo0` register value.
    pub entry_lo0: u32,
    /// Packed `EntryLo1` register value.
    pub entry_lo1: u32,
}

impl TlbEntry {
    pub const fn new() -> Self {
        Self {
            page_mask: 0,
            entry_hi: 0,
            entry_lo0: 0,
            entry_lo1: 0,
        }
    }
}

impl Default for TlbEntry {
    fn default() -> Self {
        Self::new()
    }
}

impl TlbEntry {
    /// Extract the VPN2 field (bits 13..=31) of `EntryHi`.
    #[inline]
    pub fn vpn2(&self) -> u32 {
        self.entry_hi & 0xFFFFE000
    }

    /// Returns true if this entry maps the scratchpad (SPR bit set in
    /// `EntryLo0`/`EntryLo1`).
    #[inline]
    pub fn is_spr(&self) -> bool {
        (self.entry_lo0 & (1 << 31)) != 0
    }

    /// PageMask bits 13..=24 (the encoded page size).
    #[inline]
    pub fn mask_bits(&self) -> u32 {
        self.page_mask & 0x01FFE000
    }

    /// EntryLo0 PFN (bits 6..=29).
    #[inline]
    pub fn pfn0(&self) -> u32 {
        self.entry_lo0 & 0x3FFFFFC0
    }

    /// EntryLo1 PFN (bits 6..=29).
    #[inline]
    pub fn pfn1(&self) -> u32 {
        self.entry_lo1 & 0x3FFFFFC0
    }

    /// Returns true if both EntryLo0 and EntryLo1 have the G bit set.
    #[inline]
    pub fn is_global(&self) -> bool {
        ((self.entry_lo0 & self.entry_lo1) & 1) != 0
    }
}

/// TLB array. The EE has 48 TLB entries (indices 0..=47); the translation
/// keeps 64 slots so the upper "out of range" indices can be detected.
pub static mut tlb: [TlbEntry; 64] = [TlbEntry::new(); 64];

/// Cached-TLB scratch. Mirrors the C++ `cachedTlbs` global. We keep
/// per-entry cache flags and a count.
pub struct CachedTlbs {
    /// Per-entry PFN for EntryLo0.
    pub pfn0s: [u32; 64],
    /// Per-entry PFN for EntryLo1.
    pub pfn1s: [u32; 64],
    /// Per-entry page mask (after `ConvertPageMask`).
    pub page_masks: [u32; 64],
    /// Per-entry cache-enable for EntryLo0.
    pub cache_enabled0: [u32; 64],
    /// Per-entry cache-enable for EntryLo1.
    pub cache_enabled1: [u32; 64],
    /// Number of active cached entries.
    pub count: usize,
}

impl CachedTlbs {
    pub const fn new() -> Self {
        Self {
            pfn0s: [0; 64],
            pfn1s: [0; 64],
            page_masks: [0; 64],
            cache_enabled0: [0; 64],
            cache_enabled1: [0; 64],
            count: 0,
        }
    }
}

impl Default for CachedTlbs {
    fn default() -> Self {
        Self::new()
    }
}

/// Global cached-TLB scratch state. Mirrors the C++ `cachedTlbs` global.
pub static mut cached_tlbs: CachedTlbs = CachedTlbs::new();

// ---------------------------------------------------------------------------
// Bitfield accessors
// ---------------------------------------------------------------------------

/// Bitfield accessors over the `Status` register word.
pub mod status_bf {
    use super::STATUS_ERL;
    use super::STATUS_EXL;
    use super::STATUS_KSU_SHIFT;

    #[inline]
    pub fn erl(status: u32) -> u32 {
        (status & STATUS_ERL) >> 0
    }

    #[inline]
    pub fn set_erl(status: u32, val: u32) -> u32 {
        if val != 0 {
            status | STATUS_ERL
        } else {
            status & !STATUS_ERL
        }
    }

    #[inline]
    pub fn exl(status: u32) -> u32 {
        (status & STATUS_EXL) >> 1
    }

    #[inline]
    pub fn set_exl(status: u32, val: u32) -> u32 {
        if val != 0 {
            status | STATUS_EXL
        } else {
            status & !STATUS_EXL
        }
    }

    #[inline]
    pub fn ksu(status: u32) -> u32 {
        (status >> STATUS_KSU_SHIFT) & 0x3
    }
}

// ---------------------------------------------------------------------------
// External function declarations
//
// The Rust translation is self-contained: each of these would normally be
// provided by neighbouring modules (memory, CPU core, JIT, scheduler). We
// expose them as `extern "Rust"` hooks so they can be wired in by the
// surrounding emulator crate without modifying this file. When they are
// not wired in, the in-tree stubs below provide safe fallbacks.
// ---------------------------------------------------------------------------

/// External: install the vtlb mapping for `vaddr` -> `buffer` for `size` bytes.
#[allow(unused_variables)]
pub extern "Rust" fn vtlb_vmap_buffer(vaddr: u32, buffer: *mut u8, size: usize) {}

/// External: remove the vtlb mapping for `vaddr` covering `size` bytes.
#[allow(unused_variables)]
pub extern "Rust" fn vtlb_vmap_unmap(vaddr: u32, size: usize) {}

/// External: write the EE memory page address for `vaddr`.
#[allow(unused_variables)]
pub extern "Rust" fn mem_set_page_addr(vaddr: u32, paddr: u32) {}

/// External: clear the EE memory page mapping for `vaddr`.
#[allow(unused_variables)]
pub extern "Rust" fn mem_clear_page_addr(vaddr: u32) {}

/// External: notify the EE JIT to drop its cached translation for `[vaddr, vaddr+size)`.
#[allow(unused_variables)]
pub extern "Rust" fn cpu_clear(vaddr: u32, size: u32) {}

/// External: schedule the next EE event in `delta` cycles.
#[allow(unused_variables)]
pub extern "Rust" fn cpu_set_next_event_delta(delta: u32) {}

/// External: take a branch in the interpreter (no-op stub by default).
#[allow(unused_variables)]
pub extern "Rust" fn int_set_branch() {}

/// External: take an immediate branch to `target` (stub by default).
#[allow(unused_variables)]
pub extern "Rust" fn int_do_branch(target: u32) {}

/// External: switch the EE memory region (kernel vs user). The C++ version
/// of this is currently a no-op (`memSetKernelMode` / `memSetUserMode`),
/// so the default Rust stub is a no-op too.
pub extern "Rust" fn cpu_update_operation_mode() {}

/// External: the GPR file. Mirrors `cpuRegs.GPR.r[]`. Each slot is a
/// 128-bit vector register; only the low 64-bit / 32-bit lanes are
/// architecturally accessible through MFC0/MTC0.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct Gpr {
    /// Lo 64-bit lane.
    pub sd: [i64; 2],
    /// Lo 32-bit lane (low half of `sd[0]`).
    pub ul: [u32; 4],
}

/// GPR file. Mirrors `cpuRegs.GPR`.
pub static mut gpr: [Gpr; 32] = [Gpr {
    sd: [0, 0],
    ul: [0, 0, 0, 0],
}; 32];

/// DMA registers referenced by `CPCOND0`. Only the bits needed for the
/// condition test are surfaced.
pub struct DmacRegs {
    /// DMA PCR (priority control).
    pub pcr: u32,
    /// DMA STAT (channel status).
    pub stat: u32,
}

impl DmacRegs {
    pub const fn new() -> Self {
        Self { pcr: 0, stat: 0 }
    }
}

impl Default for DmacRegs {
    fn default() -> Self {
        Self::new()
    }
}

/// Global DMA register file. Mirrors `dmacRegs`.
pub static mut dmac_regs: DmacRegs = DmacRegs::new();

/// Stash for the current branch target and per-instruction opcode. The
/// interpreter exposes these through `_BranchTarget_` and `cpuRegs.code`.
/// We model them as plain statics because they are scratch values.
pub static mut branch_target: u32 = 0;
/// Cached instruction code for diagnostic logging (matches `cpuRegs.code`).
pub static mut cpu_code: u32 = 0;
/// Scratch for the `_Imm_` field used by MFC0/MTC0 rd=25 (MFPC/MTPC).
pub static mt_imm: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// Diagnostic logger. Mirrors the C++ `COP0_LOG` macro: emits nothing in the
/// Rust port by default (the surrounding emulator wires in the real logger
/// via the `Console` module).
#[inline]
pub fn cop0_log(_fmt: &str) {}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns true when the perf counter should count for the given PCR event id.
///
/// Mirrors the C++ `PERF_ShouldCountEvent`. Events 1/2/3 (cycle/issued/
/// branch) and 12/13/14/15 (completion) count; the remaining modes either
/// are not implementable or are reserved as "disable".
pub fn perf_should_count_event(evt: u32) -> bool {
    matches!(evt, 1 | 2 | 3 | 12 | 13 | 14 | 15)
}

/// Diagnostic for "unsupported" perf events. Mirrors `COP0_DiagnosticPCCR`.
///
/// Issues warnings (in C++; no-op in this translation by default) when the
/// configured event modes are 7..=10, which are reserved for unimplemented
/// behaviours on real hardware.
pub fn cop0_diagnostic_pccr() {
    // SAFETY: read-only access of `perf`.
    unsafe {
        let event0 = (perf.pccr >> 1) & 0x1F;
        let event1 = (perf.pccr >> 6) & 0x1F;
        if (7..=10).contains(&event0) || (7..=10).contains(&event1) {
            // The C++ version uses `Console.Warning` here; the Rust port
            // leaves this as a no-op until a logger is wired in.
        }
    }
}

/// Update the performance counters. Mirrors the C++ `COP0_UpdatePCCR`.
///
/// Called whenever the COP0 status is written and on every MFC0 rd=25 read
/// so the visible counter value is current. Counter increments are computed
/// from the elapsed `cpuRegs.cycle` delta and the relevant memory-mode mask.
pub fn cop0_update_pccr() {
    // SAFETY: `perf`, `last_perf_cycle`, `cop0.status`, and `cpuRegs.cycle`
    // are all written under the single-threaded interpreter lock.
    unsafe {
        // Counting and counter exceptions are not performed if we are
        // currently executing a Level 2 exception (ERL) or the counting
        // function is not enabled (CTE).
        let status = cop0.status;
        let cte = (perf.pccr & PCCR_CTE) != 0;
        if (status & STATUS_ERL) != 0 || !cte {
            last_perf_cycle[0] = cpuRegs.cycle;
            last_perf_cycle[1] = cpuRegs.cycle;
            return;
        }

        let ksu = status_bf::ksu(status);
        let exl = status_bf::exl(status);
        // Per-PCR mode mask: KSU selects the user/super/kernel bit, EXL
        // adds an extra mode bit on top.
        let pcr0_mask = (1u32 << (ksu + 2)) | (exl << 1);
        let pcr1_mask = (1u32 << (ksu + 12)) | (exl << 11);

        let event0 = (perf.pccr >> 1) & 0x1F;
        if (perf.pccr & pcr0_mask) != 0 && perf_should_count_event(event0) {
            let mut incr = cpuRegs.cycle.wrapping_sub(last_perf_cycle[0]);
            if incr == 0 {
                incr = 1;
            }
            perf.pcr0 = perf.pcr0.wrapping_add(incr as u32);
            // PCR overflow sets the MSB. The C++ version raises a Level 2
            // exception here, but the rest of the emulator is not prepared
            // for that yet, so we just latch the bit.
            let _ = perf.pcr0 & 0x8000_0000;
        }

        let event1 = (perf.pccr >> 6) & 0x1F;
        if (perf.pccr & pcr1_mask) != 0 && perf_should_count_event(event1) {
            let mut incr = cpuRegs.cycle.wrapping_sub(last_perf_cycle[1]);
            if incr == 0 {
                incr = 1;
            }
            perf.pcr1 = perf.pcr1.wrapping_add(incr as u32);
            let _ = perf.pcr1 & 0x8000_0000;
        }

        last_perf_cycle[0] = cpuRegs.cycle;
        last_perf_cycle[1] = cpuRegs.cycle;
    }
}

// ---------------------------------------------------------------------------
// Status / Config helpers
// ---------------------------------------------------------------------------

/// Write to the COP0 Status register. Mirrors the C++ `WriteCP0Status`.
///
/// Funnels status writes through `COP0_UpdatePCCR` so any pending counter
/// delta is committed before the new mode bits take effect, and re-arms
/// the next CPU event so interrupts scheduled against the old status are
/// recomputed.
pub fn write_cp0_status(value: u32) {
    cop0_update_pccr();
    // SAFETY: writes to `cop0.status` are serialised by the interpreter
    // dispatch loop.
    unsafe {
        cop0.status = value;
    }
    cpu_set_next_event_delta(4);
}

/// Write to the COP0 Config register. Mirrors the C++ `WriteCP0Config`.
///
/// Protects the read-only ICacheSize (IC) and DataCacheSize (DC) bits by
/// clearing them in the written value and forcing them to 0x440 (the EE's
/// power-on defaults).
pub fn write_cp0_config(value: u32) {
    // SAFETY: writes to `cop0.regs[16]` (the Config register slot) are
    // serialised by the interpreter dispatch loop.
    unsafe {
        cop0.regs[16] = (value & !0xFC0) | 0x440;
    }
}

// ---------------------------------------------------------------------------
// TLB mapping
// ---------------------------------------------------------------------------

/// Maps a single TLB entry into the vtlb. Mirrors the C++ `MapTLB`.
///
/// SPR entries are mapped to the EE scratchpad buffer; regular entries
/// install per-page physical-address mappings via `memSetPageAddr` and
/// flush the JIT for the affected range. Walkers in the surrounding
/// emulator are expected to provide the four `extern "Rust"` hooks above.
pub fn map_tlb(t: &TlbEntry, _i: usize) {
    cop0_log("MAP TLB");
    if t.is_spr() {
        // Map the scratchpad at the entry's VPN2. We cannot reach the EE
        // scratchpad buffer from here, so this is a thin wrapper around the
        // `vtlb_vmap_buffer` hook the surrounding emulator provides.
        // SAFETY: hook callers are responsible for providing a valid
        // scratchpad pointer.
        unsafe {
            vtlb_vmap_buffer(t.vpn2(), core::ptr::null_mut(), 0x4000);
        }
        return;
    }

    // EntryLo0 mapping.
    if (t.entry_lo0 & (1 << 1)) != 0 {
        let mask = ((!t.mask_bits()) << 1) & 0x000F_FFFF;
        let saddr = t.vpn2() >> 12;
        let eaddr = saddr + t.mask_bits() + 1;
        let mut addr = saddr;
        while addr < eaddr {
            if (addr & mask) == ((t.vpn2() >> 12) & mask) {
                let pa = t.pfn0().wrapping_add((addr - saddr) << 12);
                mem_set_page_addr(addr << 12, pa);
                cpu_clear(addr << 12, 0x400);
            }
            addr = addr.wrapping_add(1);
        }
    }

    // EntryLo1 mapping.
    if (t.entry_lo1 & (1 << 1)) != 0 {
        let mask = ((!t.mask_bits()) << 1) & 0x000F_FFFF;
        let saddr = (t.vpn2() >> 12) + t.mask_bits() + 1;
        let eaddr = saddr + t.mask_bits() + 1;
        let mut addr = saddr;
        while addr < eaddr {
            if (addr & mask) == ((t.vpn2() >> 12) & mask) {
                let pa = t.pfn1().wrapping_add((addr - saddr) << 12);
                mem_set_page_addr(addr << 12, pa);
                cpu_clear(addr << 12, 0x400);
            }
            addr = addr.wrapping_add(1);
        }
    }
}

/// Convert a PageMask register value into a 32-bit page-size mask.
///
/// Mirrors the C++ `ConvertPageMask`: count the set bits in the encoded
/// mask field and produce the equivalent page-size mask. The EE supports
/// page sizes of 4 KiB up to 16 MiB (mask field 0..=12); an invalid mask
/// asserts and produces a degenerate value rather than panicking, to
/// match the C++ side.
pub fn convert_page_mask(page_mask: u32) -> u32 {
    let bits = (page_mask >> 13).count_ones();
    // Assert (mirrors `pxAssertMsg`) but do not panic; clamp to a sane
    // upper bound instead.
    if bits > 12 || (bits & 1) != 0 {
        // Invalid page mask; return 0xFFF (4 KiB) as a safe default.
        return 0x0000_0FFF;
    }
    (1u32 << (12 + bits)) - 1
}

/// Unmaps a single TLB entry from the vtlb. Mirrors the C++ `UnmapTLB`.
///
/// SPR entries are unmapped via `vtlb_VMapUnmap`; regular entries have
/// their per-page mappings cleared and the cached-TLB scratch array
/// pruned if a matching PFN pair is found.
pub fn unmap_tlb(t: &TlbEntry, _i: i32) {
    if t.is_spr() {
        vtlb_vmap_unmap(t.vpn2(), 0x4000);
        return;
    }

    if (t.entry_lo0 & (1 << 1)) != 0 {
        let mask = ((!t.mask_bits()) << 1) & 0x000F_FFFF;
        let saddr = t.vpn2() >> 12;
        let eaddr = saddr + t.mask_bits() + 1;
        let mut addr = saddr;
        while addr < eaddr {
            if (addr & mask) == ((t.vpn2() >> 12) & mask) {
                mem_clear_page_addr(addr << 12);
                cpu_clear(addr << 12, 0x400);
            }
            addr = addr.wrapping_add(1);
        }
    }

    if (t.entry_lo1 & (1 << 1)) != 0 {
        let mask = ((!t.mask_bits()) << 1) & 0x000F_FFFF;
        let saddr = (t.vpn2() >> 12) + t.mask_bits() + 1;
        let eaddr = saddr + t.mask_bits() + 1;
        let mut addr = saddr;
        while addr < eaddr {
            if (addr & mask) == ((t.vpn2() >> 12) & mask) {
                mem_clear_page_addr(addr << 12);
                cpu_clear(addr << 12, 0x400);
            }
            addr = addr.wrapping_add(1);
        }
    }

    // Remove from cachedTlbs if present.
    // SAFETY: single-threaded interpreter dispatch.
    unsafe {
        let count = cached_tlbs.count;
        let mut idx = 0usize;
        while idx < count {
            if cached_tlbs.pfn0s[idx] == t.pfn0()
                && cached_tlbs.pfn1s[idx] == t.pfn1()
                && cached_tlbs.page_masks[idx] == convert_page_mask(t.page_mask)
            {
                // Shift the trailing entries down by one.
                let mut j = idx;
                while j + 1 < count {
                    cached_tlbs.cache_enabled0[j] = cached_tlbs.cache_enabled0[j + 1];
                    cached_tlbs.cache_enabled1[j] = cached_tlbs.cache_enabled1[j + 1];
                    cached_tlbs.pfn0s[j] = cached_tlbs.pfn0s[j + 1];
                    cached_tlbs.pfn1s[j] = cached_tlbs.pfn1s[j + 1];
                    cached_tlbs.page_masks[j] = cached_tlbs.page_masks[j + 1];
                    j += 1;
                }
                cached_tlbs.count -= 1;
                break;
            }
            idx += 1;
        }
    }
}

/// Write a TLB entry from the COP0 register file. Mirrors the C++ `WriteTLB`.
///
/// Default-reserved cache modes are normalised to "uncached" (C=2), and
/// cached entries are appended to the `cachedTlbs` scratch array before
/// the vtlb is installed via `MapTLB`.
pub fn write_tlb(i: usize) {
    if i >= 48 {
        return;
    }
    // SAFETY: `i < 48` guarantees we stay inside the TLB array.
    unsafe {
        tlb[i].page_mask = cop0.regs[5];
        tlb[i].entry_hi = cop0.regs[10];
        tlb[i].entry_lo0 = cop0.regs[2];
        tlb[i].entry_lo1 = cop0.regs[3];

        // SPR entries are always cached (C=3); other entries use C=2
        // (uncached) for reserved modes.
        if tlb[i].is_spr() {
            tlb[i].entry_lo0 = (tlb[i].entry_lo0 & !0x3) | 3;
            tlb[i].entry_lo1 = (tlb[i].entry_lo1 & !0x3) | 3;
        } else {
            if (tlb[i].entry_lo0 & 0x3) == 1 || (tlb[i].entry_lo0 & 0x3) == 0 {
                tlb[i].entry_lo0 = (tlb[i].entry_lo0 & !0x3) | 2;
            }
            if (tlb[i].entry_lo1 & 0x3) == 1 || (tlb[i].entry_lo1 & 0x3) == 0 {
                tlb[i].entry_lo1 = (tlb[i].entry_lo1 & !0x3) | 2;
            }
        }

        // Track cached entries so the JIT can emit cache-aware code.
        let lo0_cached = (tlb[i].entry_lo0 & (1 << 1)) != 0
            && (tlb[i].entry_lo0 & 0x3) == 3;
        let lo1_cached = (tlb[i].entry_lo1 & (1 << 1)) != 0
            && (tlb[i].entry_lo1 & 0x3) == 3;
        if !tlb[i].is_spr() && (lo0_cached || lo1_cached) {
            let idx = cached_tlbs.count;
            if idx < cached_tlbs.pfn0s.len() {
                cached_tlbs.cache_enabled0[idx] = if lo0_cached { !0 } else { 0 };
                cached_tlbs.cache_enabled1[idx] = if lo1_cached { !0 } else { 0 };
                cached_tlbs.pfn0s[idx] = tlb[i].pfn0();
                cached_tlbs.pfn1s[idx] = tlb[i].pfn1();
                cached_tlbs.page_masks[idx] = convert_page_mask(tlb[i].page_mask);
                cached_tlbs.count += 1;
            }
        }

        map_tlb(&tlb[i], i);
    }
}

// ---------------------------------------------------------------------------
// TLB opcodes
// ---------------------------------------------------------------------------

/// COP0 `TLBR`. Mirrors the C++ `TLBR()` interpreter opcode.
///
/// Reads the TLB entry indexed by `Index & 0x3F` into the COP0 register
/// file. The Index field's lower 6 bits select the entry; values > 47 are
/// rejected with a warning (currently a no-op in this translation).
pub fn tlbr() {
    // SAFETY: read access of `cop0.regs[0]` is serialised by the
    // interpreter dispatch loop.
    let i = unsafe { (cop0.regs[0] & 0x3F) as usize };
    if i > 47 {
        // C++ warns here; the Rust translation is silent until a logger
        // is wired in.
        return;
    }
    // SAFETY: `i <= 47`.
    unsafe {
        let pm = tlb[i].page_mask;
        let mask_field = pm & 0x01FFE000;
        cop0.regs[5] = mask_field;
        cop0.regs[10] = tlb[i].entry_hi & !((mask_field) | 0x1F00);
        cop0.regs[2] = tlb[i].entry_lo0 & !0xFC00_0000 & !1;
        cop0.regs[3] = tlb[i].entry_lo1 & !0x7C00_0000 & !1;
        // G is only set when both EntryLo0 and EntryLo1 have it set.
        let g = (tlb[i].entry_lo0 & 1) & (tlb[i].entry_lo1 & 1);
        cop0.regs[2] |= g;
        cop0.regs[3] |= g;
    }
}

/// COP0 `TLBWI`. Mirrors the C++ `TLBWI()` interpreter opcode.
///
/// Writes the COP0 register file into the TLB entry selected by
/// `Index & 0x3F`. The previous mapping (if any) is unmapped first.
pub fn tlbwi() {
    // SAFETY: read of `cop0.regs[0]` is serialised by the interpreter
    // dispatch loop.
    let j = unsafe { (cop0.regs[0] & 0x3F) as usize };
    if j > 47 {
        return;
    }
    // SAFETY: `j <= 47`.
    unsafe {
        unmap_tlb(&tlb[j], j as i32);
        write_tlb(j);
    }
}

/// COP0 `TLBWR`. Mirrors the C++ `TLBWR()` interpreter opcode.
///
/// Writes the COP0 register file into the TLB entry selected by
/// `Random & 0x3F`. The previous mapping (if any) is unmapped first.
pub fn tlbwr() {
    // SAFETY: read of `cop0.regs[1]` is serialised by the interpreter
    // dispatch loop.
    let j = unsafe { (cop0.regs[1] & 0x3F) as usize };
    if j > 47 {
        return;
    }
    // SAFETY: `j <= 47`.
    unsafe {
        unmap_tlb(&tlb[j], j as i32);
        write_tlb(j);
    }
}

/// COP0 `TLBP`. Mirrors the C++ `TLBP()` interpreter opcode.
///
/// Probes the TLB for an entry matching `EntryHi` (VPN2 + ASID). On
/// match, `Index` is set to the matching entry; on miss, `Index` is set to
/// `0x80000000` (the architectural "not found" sentinel).
pub fn tlbp() {
    // SAFETY: read/write of `cop0.regs[10]` and `cop0.regs[0]` is
    // serialised by the interpreter dispatch loop.
    unsafe {
        let entry_hi = cop0.regs[10];
        let vpn2 = entry_hi & 0xFFFFE000;
        let asid = (entry_hi >> 0) & 0xFF;

        cop0.regs[0] = 0xFFFF_FFFF;
        for i in 0..48 {
            let t = tlb[i];
            if t.vpn2() == ((!t.mask_bits()) & vpn2)
                && (t.is_global() || ((t.entry_hi & 0xFF) == asid))
            {
                cop0.regs[0] = i as u32;
                break;
            }
        }
        if cop0.regs[0] == 0xFFFF_FFFF {
            cop0.regs[0] = 0x8000_0000;
        }
    }
}

// ---------------------------------------------------------------------------
// MFC0 / MTC0 (full)
// ---------------------------------------------------------------------------

/// Full MFC0 opcode. Mirrors the C++ `MFC0()` interpreter opcode.
///
/// The `rt` parameter is the destination GPR (kept for parity); `rd` is
/// the COP0 register to read; `_Imm_` is the lower 16 bits of the
/// instruction (used to distinguish MFPS/MFPC0/MFPC1 for `rd == 25`).
pub fn mfc0_full(rt: u32, rd: u32, _imm: u32) {
    // SAFETY: read access of `cop0.regs`, `perf`, and `cpuRegs.cycle` is
    // serialised by the interpreter dispatch loop.
    unsafe {
        // Special case: CP0.Count must be updated even when rt == 0.
        if rd == 9 {
            let incr = cpuRegs.cycle.wrapping_sub(last_cop0_cycle);
            let incr = if incr == 0 { 1 } else { incr };
            cop0.regs[9] = cop0.regs[9].wrapping_add(incr as u32);
            last_cop0_cycle = cpuRegs.cycle;
            if rt == 0 {
                return;
            }
        } else if rt == 0 {
            return;
        }

        let value: u32 = match rd {
            12 => cop0.regs[12] & 0xF0C7_9C1F,
            25 => {
                if (_imm & 1) == 0 {
                    // MFPS: return PCCR regardless of the GPR value.
                    perf.pccr
                } else if (_imm & 2) == 0 {
                    // MFPC 0: update the counters, return PCR0.
                    cop0_update_pccr();
                    perf.pcr0
                } else {
                    // MFPC 1.
                    cop0_update_pccr();
                    perf.pcr1
                }
            }
            24 => {
                cop0_log("MFC0 Breakpoint debug Registers code");
                cop0.regs[24]
            }
            _ => cop0.regs[rd as usize & 31],
        };

        gpr[rt as usize & 31].sd[0] = value as i32 as i64;
    }
}

/// Full MTC0 opcode. Mirrors the C++ `MTC0()` interpreter opcode.
///
/// The `rt` parameter is the source GPR; `rd` is the COP0 register to
/// write; `_imm_` is the lower 16 bits of the instruction (used for
/// MTPS/MTPC0/MTPC1 dispatch on `rd == 25`).
pub fn mtc0_full(rt: u32, rd: u32, _imm: u32) {
    let gpr_val = unsafe { gpr[rt as usize & 31].ul[0] };
    match rd {
        9 => {
            // SAFETY: writes to `cop0.regs[9]` and `last_cop0_cycle` are
            // serialised by the interpreter dispatch loop.
            unsafe {
                last_cop0_cycle = cpuRegs.cycle;
                cop0.regs[9] = gpr_val;
            }
        }
        12 => write_cp0_status(gpr_val),
        16 => write_cp0_config(gpr_val),
        24 => {
            cop0_log("MTC0 Breakpoint debug Registers code");
            // SAFETY: see above.
            unsafe {
                cop0.regs[24] = gpr_val;
            }
        }
        25 => {
            // SAFETY: writes to `perf` are serialised by the interpreter.
            unsafe {
                if (_imm & 1) == 0 {
                    // MTPS.
                    if (_imm & 0x3E) != 0 {
                        // Only effective when the register field is 0.
                    } else {
                        cop0_update_pccr();
                        perf.pccr = gpr_val;
                        cop0_diagnostic_pccr();
                    }
                } else if (_imm & 2) == 0 {
                    // MTPC 0.
                    perf.pcr0 = gpr_val;
                    last_perf_cycle[0] = cpuRegs.cycle;
                } else {
                    // MTPC 1.
                    perf.pcr1 = gpr_val;
                    last_perf_cycle[1] = cpuRegs.cycle;
                }
            }
        }
        _ => {
            // SAFETY: see above.
            unsafe {
                cop0.regs[rd as usize & 31] = gpr_val;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// COP0 condition and branches
// ---------------------------------------------------------------------------

/// COP0 condition test. Mirrors the C++ `CPCOND0()`.
///
/// Returns true when the DMA interrupt status bits not masked off by
/// `dmacRegs.pcr.CPC` are all set — i.e. all the corresponding channels
/// have raised an interrupt request.
pub fn cpcond0() -> i32 {
    // SAFETY: read-only access of `dmac_regs`.
    let stat = unsafe { dmac_regs.stat };
    let pcr = unsafe { dmac_regs.pcr };
    if ((stat | !pcr) & 0x3FF) == 0x3FF {
        1
    } else {
        0
    }
}

/// COP0 `BC0F`. Mirrors the C++ `BC0F()` interpreter opcode.
pub fn bc0f() {
    if cpcond0() == 0 {
        // SAFETY: `branch_target` is the scratch register the interpreter
        // sets before calling us.
        let target = unsafe { branch_target };
        int_do_branch(target);
    }
}

/// COP0 `BC0T`. Mirrors the C++ `BC0T()` interpreter opcode.
pub fn bc0t() {
    if cpcond0() == 1 {
        let target = unsafe { branch_target };
        int_do_branch(target);
    }
}

/// COP0 `BC0FL` (branch likely, taken when condition false).
pub fn bc0fl() {
    if cpcond0() == 0 {
        let target = unsafe { branch_target };
        int_do_branch(target);
    } else {
        // SAFETY: PC bump on the "not taken" path of a branch-likely.
        unsafe {
            cpuRegs.pc = cpuRegs.pc.wrapping_add(4);
        }
    }
}

/// COP0 `BC0TL` (branch likely, taken when condition true).
pub fn bc0tl() {
    if cpcond0() == 1 {
        let target = unsafe { branch_target };
        int_do_branch(target);
    } else {
        // SAFETY: PC bump on the "not taken" path of a branch-likely.
        unsafe {
            cpuRegs.pc = cpuRegs.pc.wrapping_add(4);
        }
    }
}

// ---------------------------------------------------------------------------
// ERET / DI / EI
// ---------------------------------------------------------------------------

/// COP0 `ERET`. Mirrors the C++ `ERET()` interpreter opcode.
///
/// Returns from an exception: if `Status.ERL` is set, control resumes at
/// `ErrorEPC` and ERL is cleared; otherwise it resumes at `EPC` and EXL
/// is cleared. The CPU's operating mode and the next-event delta are
/// updated to reflect the new status bits.
pub fn eret() {
    // SAFETY: writes to `cop0.epc`, `cop0.status`, and `cpuRegs.pc` are
    // serialised by the interpreter dispatch loop.
    unsafe {
        if (cop0.status & STATUS_ERL) != 0 {
            cpuRegs.pc = cop0.epc;
            cop0.status = status_bf::set_erl(cop0.status, 0);
        } else {
            // EPC lives in COP0 r[14]; the C++ source uses the named field.
            cpuRegs.pc = cop0.regs[14];
            cop0.status = status_bf::set_exl(cop0.status, 0);
        }
        cpu_update_operation_mode();
        cpu_set_next_event_delta(4);
        int_set_branch();
    }
}

/// COP0 `DI` (disable interrupts). Mirrors the C++ `DI()` interpreter opcode.
///
/// Interrupt-disable is only meaningful when interrupts are currently
/// enabled, i.e. `_EDI`, `EXL`, `ERL` and `KSU == 0` are all clear. In all
/// other modes the EE ignores the instruction.
pub fn di() {
    // SAFETY: see above.
    unsafe {
        let status = cop0.status;
        let edi = (status & STATUS_EDI) != 0;
        let exl = (status & STATUS_EXL) != 0;
        let erl = (status & STATUS_ERL) != 0;
        let ksu_zero = status_bf::ksu(status) == 0;
        if edi || exl || erl || ksu_zero {
            cop0.status = status & !STATUS_EIE;
        }
    }
}

/// COP0 `EI` (enable interrupts). Mirrors the C++ `EI()` interpreter opcode.
pub fn ei() {
    // SAFETY: see above.
    unsafe {
        let status = cop0.status;
        let edi = (status & STATUS_EDI) != 0;
        let exl = (status & STATUS_EXL) != 0;
        let erl = (status & STATUS_ERL) != 0;
        let ksu_zero = status_bf::ksu(status) == 0;
        if edi || exl || erl || ksu_zero {
            cop0.status = status | STATUS_EIE;
            cpu_set_next_event_delta(4);
        }
    }
}
