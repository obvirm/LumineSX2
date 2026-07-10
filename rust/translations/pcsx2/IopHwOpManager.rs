// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust translation of `pcsx2/IopHw_OpManager.{h,cpp}`.
//!
//! The IOP hardware op manager owns the lifetime of the per-page function
//! pointer dispatch tables that the IOP bus walks on every read/write.
//! Three entry points form the public surface:
//!
//! * [`InitIopHwOpManager`] wires the dispatch tables up; in this Rust
//!   translation that means zeroing the previously-published page slots so
//!   the bus starts from a known state.
//! * [`ShutdownIopHwOpManager`] is the symmetric teardown — any handler the
//!   manager still owns is cleared so subsequent re-`Init`s start clean.
//! * [`RegisterOp`] installs a per-page handler. The `addr` argument's
//!   bits [19:16] select the 64 KiB sub-page of the IOP HW region that the
//!   handler will service; `RegisterOp` masks the address down to that
//!   sub-page and stashes the handler in [`IopHw_FnRead`] /
//!   [`IopHw_FnWrite`].
//!
//! The translation re-uses the [`IopHwReadFn`] / [`IopHwWriteFn`] aliases
//! and the [`IopHw_FnRead`] / [`IopHw_FnWrite`] static dispatch tables that
//! live in [`crate::pcsx2::IopHw`]. The C++ side keeps the manager in its
//! own translation unit to avoid pulling the device-specific handler
//! implementations into every translation unit that needs to talk to the
//! IOP bus; the Rust port keeps that boundary by making this module the
//! only place that mutates the dispatch tables.

use crate::pcsx2::IopHw::{
    IopHwReadFn, IopHwWriteFn, IopHw_FnRead, IopHw_FnWrite,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Number of 0x10000-byte sub-pages the IOP hardware region is split into.
///
/// Bits [19:16] of an IOP HW address select one of these sub-pages, so the
/// dispatch tables in [`IopHw_FnRead`] / [`IopHw_FnWrite`] must have exactly
/// `IOP_HW_PAGE_COUNT` slots. The constant is re-exported here so callers
/// that only know about the op manager don't need to import the IopHw
/// module's table size directly.
pub const IOP_HW_PAGE_COUNT: usize = 16;

/// Bit shift that converts a full IOP HW address into its sub-page index.
///
/// Equivalent to `log2(IOP_HW_PAGE_SIZE)` where `IOP_HW_PAGE_SIZE = 0x10000`
/// (64 KiB). Bits [19:16] of the address select the page; bits [15:0] are
/// the page-relative offset passed to the handler.
pub const IOP_HW_PAGE_SHIFT: u32 = 16;

/// Mask that strips the page-select bits from an IOP HW address, leaving
/// only the page-relative offset that is forwarded to a registered handler.
pub const IOP_HW_PAGE_OFFSET_MASK: u32 = 0xFFFF;

/// Sentinel address that means "the caller's handler covers every page".
///
/// Some device handlers (notably the cdvd segment-0x1F40 accessors)
/// service traffic to several disjoint sub-pages. Passing this value to
/// [`RegisterOp`] broadcasts the registration to every slot. The C++ side
/// achieves the same with a switch-on-page index; this port just iterates.
pub const IOP_HW_ALL_PAGES: u32 = 0xFFFF_FFFF;

// ---------------------------------------------------------------------------
// Init / Shutdown
// ---------------------------------------------------------------------------

/// Initialise the IOP hardware op manager.
///
/// In the C++ source this routine sets up the dispatch tables, hands them
/// out to the rest of the IOP subsystem, and (re-)registers the default
/// handlers for the well-known pages. The Rust port keeps the same
/// lifecycle but expresses it in terms of the existing [`IopHw_FnRead`] /
/// [`IopHw_FnWrite`] tables:
///
/// 1. Clear both tables so a stale handler from a previous run cannot
///    accidentally serve the bus.
/// 2. Publish the (now empty) tables for the rest of the crate.
///
/// The function is idempotent; calling it twice has the same observable
/// effect as calling it once.
pub fn InitIopHwOpManager() {
    clear_dispatch_tables();
}

/// Tear down the IOP hardware op manager.
///
/// In the C++ source this is the matching shutdown: it unhooks every
/// per-page handler the manager owns and (where appropriate) frees any
/// resources they referenced. The Rust port has no heap allocations to
/// release — the dispatch tables are `static`, and the handlers they
/// point at are expected to be `'static` too — so this is just a
/// stronger version of the init step: zero every slot so the bus is
/// guaranteed to return zeros until the next [`InitIopHwOpManager`] call.
///
/// Calling this without a matching init is safe; the tables are already
/// `Option<fn>` and a fresh crate boots with all slots set to `None`.
pub fn ShutdownIopHwOpManager() {
    clear_dispatch_tables();
}

/// Reset every slot in both dispatch tables to `None`.
///
/// Centralised helper for [`InitIopHwOpManager`] and
/// [`ShutdownIopHwOpManager`] so the two routines cannot drift apart.
fn clear_dispatch_tables() {
    // Safety: the dispatch tables are `static mut` and we are the only
    // module that mutates them — the contract documented at the top of
    // this file. Each slot is an `Option<fn>` and is being replaced with
    // the canonical "no handler" value.
    unsafe {
        for slot in &mut IopHw_FnRead {
            *slot = None;
        }
        for slot in &mut IopHw_FnWrite {
            *slot = None;
        }
    }
}

// ---------------------------------------------------------------------------
// RegisterOp
// ---------------------------------------------------------------------------

/// Register a read handler for a single IOP HW sub-page.
///
/// `addr`'s bits [19:16] select which 64 KiB sub-page of the IOP HW
/// region the handler will service; the remaining bits are masked away
/// and are not interpreted here. The handler receives the page-relative
/// offset (bits [15:0] of the original `addr`) on every dispatch.
///
/// # Examples
///
/// ```ignore
/// // Install a handler for the 0x1F801xxx page (sub-page index 1).
/// RegisterOp(0x1F801000, iopHwRead8_Page1);
/// ```
pub fn RegisterOp(addr: u32, handler: IopHwReadFn) {
    install_read_handler(addr, handler);
}

/// Register a write handler for a single IOP HW sub-page.
///
/// `addr`'s bits [19:16] select which 64 KiB sub-page of the IOP HW
/// region the handler will service; the remaining bits are masked away
/// and are not interpreted here. The handler receives the page-relative
/// offset (bits [15:0] of the original `addr`) and the 32-bit value the
/// guest wrote.
pub fn RegisterWriteOp(addr: u32, handler: IopHwWriteFn) {
    install_write_handler(addr, handler);
}

/// Register the same read handler across every IOP HW sub-page.
///
/// Convenience for device drivers that need to service traffic to any
/// page (e.g. the cdvd segment-0x1F40 helper, which the C++ side exposes
/// through [`IopHw::psxHw4Read8`]). Equivalent to calling
/// [`RegisterOp`] with each sub-page index in turn.
pub fn RegisterOpAllPages(handler: IopHwReadFn) {
    for slot in 0..IOP_HW_PAGE_COUNT {
        install_read_handler_at_index(slot, handler);
    }
}

/// Register the same write handler across every IOP HW sub-page.
///
/// See [`RegisterOpAllPages`] for the read-side counterpart.
pub fn RegisterWriteOpAllPages(handler: IopHwWriteFn) {
    for slot in 0..IOP_HW_PAGE_COUNT {
        install_write_handler_at_index(slot, handler);
    }
}

/// Install `handler` into the read dispatch table for the sub-page that
/// `addr` belongs to.
fn install_read_handler(addr: u32, handler: IopHwReadFn) {
    let index = page_index(addr);
    install_read_handler_at_index(index, handler);
}

/// Install `handler` into the write dispatch table for the sub-page that
/// `addr` belongs to.
fn install_write_handler(addr: u32, handler: IopHwWriteFn) {
    let index = page_index(addr);
    install_write_handler_at_index(index, handler);
}

/// Write `handler` into the read dispatch table at `index`. Out-of-range
/// indices are silently dropped so a misbehaving caller cannot panic the
/// emulator.
fn install_read_handler_at_index(index: usize, handler: IopHwReadFn) {
    if index >= IOP_HW_PAGE_COUNT {
        return;
    }
    // Safety: see [`clear_dispatch_tables`]. Only this module mutates the
    // tables, and `index` is bounded to `IOP_HW_PAGE_COUNT`.
    unsafe {
        IopHw_FnRead[index] = Some(handler);
    }
}

/// Write `handler` into the write dispatch table at `index`. Out-of-range
/// indices are silently dropped so a misbehaving caller cannot panic the
/// emulator.
fn install_write_handler_at_index(index: usize, handler: IopHwWriteFn) {
    if index >= IOP_HW_PAGE_COUNT {
        return;
    }
    // Safety: see [`clear_dispatch_tables`].
    unsafe {
        IopHw_FnWrite[index] = Some(handler);
    }
}

/// Extract the 4-bit sub-page index from an IOP HW address.
///
/// Returns 0 for any address that does not have bits [19:16] set; the
/// IOP HW region lives entirely in the 0x1F80xxxx range, where the page
/// index is always between 0 and 15.
#[inline]
fn page_index(addr: u32) -> usize {
    ((addr >> IOP_HW_PAGE_SHIFT) & ((IOP_HW_PAGE_COUNT as u32) - 1)) as usize
}

// ---------------------------------------------------------------------------
// Inspection helpers
// ---------------------------------------------------------------------------

/// Look up the read handler currently installed for `addr`'s sub-page.
///
/// Returns `None` if no handler has been registered (the slot is empty,
/// or `addr`'s sub-page is out of range for the 16-entry table). This
/// routine is mostly useful for tests and diagnostics; production
/// callers should go straight through the [`IopHw::psxHw1Read8`] /
/// [`IopHw::psxHw1Write8`] helpers.
#[inline]
pub fn get_read_handler(addr: u32) -> Option<IopHwReadFn> {
    let index = page_index(addr);
    if index >= IOP_HW_PAGE_COUNT {
        None
    } else {
        IopHw_FnRead[index]
    }
}

/// Look up the write handler currently installed for `addr`'s sub-page.
///
/// See [`get_read_handler`] for the read-side counterpart.
#[inline]
pub fn get_write_handler(addr: u32) -> Option<IopHwWriteFn> {
    let index = page_index(addr);
    if index >= IOP_HW_PAGE_COUNT {
        None
    } else {
        IopHw_FnWrite[index]
    }
}

/// `true` if the manager has at least one handler installed.
pub fn has_any_handler() -> bool {
    let read = IopHw_FnRead.iter().any(|slot| slot.is_some());
    let write = IopHw_FnWrite.iter().any(|slot| slot.is_some());
    read || write
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_index_extracts_subpage_bits() {
        // 0x1F801000 -> sub-page 1 (bits [19:16]).
        assert_eq!(page_index(0x1F801000), 1);
        // 0x1F808400 -> sub-page 8 (bits [19:16]).
        assert_eq!(page_index(0x1F808400), 8);
        // 0x1F800000 -> sub-page 0.
        assert_eq!(page_index(0x1F800000), 0);
    }

    #[test]
    fn page_index_clamps_to_table_size() {
        // Bits outside [19:16] are ignored; the upper bits of the
        // address must not bleed into the page index.
        for shift in 0..IOP_HW_PAGE_SHIFT {
            let addr = 1u32 << shift;
            assert!(page_index(addr) < IOP_HW_PAGE_COUNT);
        }
    }

    #[test]
    fn register_op_installs_handler() {
        // Installing a handler and then immediately reading it back
        // should yield the same function pointer. The slot is shared
        // with other tests, so we only assert the round-trip — not that
        // the slot was empty before.
        fn sentinel_read(_addr: u32) -> u32 {
            0xDEAD_BEEF
        }
        fn sentinel_write(_addr: u32, _value: u32) {}
        let page = 0x1F801000u32;
        RegisterOp(page, sentinel_read);
        RegisterWriteOp(page, sentinel_write);
        assert_eq!(get_read_handler(page), Some(sentinel_read as IopHwReadFn));
        assert_eq!(
            get_write_handler(page),
            Some(sentinel_write as IopHwWriteFn)
        );
    }
}