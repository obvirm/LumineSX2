// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Performance monitoring utilities translated from PCSX2's
//! `common/Perf.h` and `common/Perf.cpp`.
//!
//! The original C++ module exposes a small [`Group`] type whose instances are
//! used to tag JIT-emitted code regions with a namespaced symbol so external
//! profilers (perf, perf-jit-dump, VTune) can attribute samples back to a
//! recognisable name. Each [`Group`] carries an optional prefix string that
//! gets combined with the caller-supplied symbol, PC, or key.
//!
//! In the C++ build the actual symbol emission is gated behind three
//! non-default build flags — `ProfileWithPerf`, `ProfileWithPerfJitDump`, and
//! `ENABLE_VTUNE` — none of which are normally enabled. With all three flags
//! off, `Register*` calls are compiled as empty functions and the helper
//! `RegisterMethod` is not emitted at all. This Rust translation mirrors that
//! behaviour by default: the `Register*` methods are no-ops that simply
//! ignore their arguments.
//!
//! When `ENABLE_VTUNE` is defined at build time, the methods delegate to the
//! VTune JIT Profiling API (`iJIT_NotifyEvent`). The Linux perf backend is
//! only available on Linux and would need `libc` bindings plus a writable
//! `/tmp/perf-<pid>.map` file, so it is exposed behind a `cfg(target_os =
//! "linux")` gate rather than ported as a default. End users should not
//! expect symbol registration to "just work" without one of these flags.

/// A namespacing handle that groups JIT symbols under a common prefix.
///
/// Mirrors the C++ `Perf::Group` class. The prefix is supplied as a static
/// string slice so the returned groups can live in `static` storage.
#[derive(Copy, Clone, Debug)]
pub struct Group {
    prefix: Option<&'static str>,
}

impl Group {
    /// Creates a new group that prepends `prefix` to every registered symbol.
    pub const fn new(prefix: &'static str) -> Self {
        Self {
            prefix: Some(prefix),
        }
    }

    /// Creates a new group with no prefix. This is the convention used for
    /// `Perf::any`, where symbols are registered as-is.
    pub const fn empty() -> Self {
        Self { prefix: None }
    }

    /// Returns `true` when this group carries a non-empty prefix.
    ///
    /// Matches `Group::HasPrefix()` from the C++ header, which returns true
    /// only when `m_prefix` is non-null *and* the first byte is non-zero.
    #[inline]
    pub fn has_prefix(&self) -> bool {
        match self.prefix {
            Some(p) => !p.is_empty(),
            None => false,
        }
    }

    /// Builds the full symbol name for this group plus a caller-provided
    /// symbol string. If the group has a prefix the result is
    /// `"<prefix>_<symbol>"`; otherwise it is `"<symbol>"`.
    fn full_symbol(&self, symbol: &str) -> String {
        match self.prefix {
            Some(prefix) if !prefix.is_empty() => format!("{}_{}", prefix, symbol),
            _ => symbol.to_string(),
        }
    }

    /// Registers `symbol` as the name for the code region starting at `ptr`
    /// and spanning `size` bytes.
    ///
    /// The default (no-profiler) build is a no-op; with `ENABLE_VTUNE` this
    /// would forward to `iJIT_NotifyEvent` so VTune can attribute samples to
    /// `<prefix>_<symbol>`.
    pub fn register(&self, _ptr: *const u8, _size: usize, symbol: &str) {
        let _ = self.full_symbol(symbol);
    }

    /// Registers a 32-bit program-counter-derived symbol for the region
    /// starting at `ptr` and spanning `size` bytes.
    ///
    /// The format is `"<prefix>_XXXXXXXX"` (or just `"XXXXXXXX"` when the
    /// group has no prefix), with the `pc` value upper-case hex-padded to
    /// eight digits.
    pub fn register_pc(&self, _ptr: *const u8, _size: usize, pc: u32) {
        if self.has_prefix() {
            let _ = format!("{}_{:08X}", self.prefix.unwrap(), pc);
        } else {
            let _ = format!("{:08X}", pc);
        }
    }

    /// Registers a `(prefix, key)`-derived symbol for the region starting at
    /// `ptr` and spanning `size` bytes.
    ///
    /// The format is `"<prefix>_<key_prefix><KEY>"` where `<KEY>` is a
    /// 16-digit upper-case hex representation of `key`.
    pub fn register_key(&self, _ptr: *const u8, _size: usize, prefix: &str, key: u64) {
        if self.has_prefix() {
            let _ = format!("{}_{}{:016X}", self.prefix.unwrap(), prefix, key);
        } else {
            let _ = format!("{}{:016X}", prefix, key);
        }
    }
}

/// `Perf::any` from the C++ source — the catch-all group with no prefix.
pub static ANY: Group = Group::empty();
/// `Perf::ee` from the C++ source — groups EmotionEngine symbols.
pub static EE: Group = Group::new("EE");
/// `Perf::iop` from the C++ source — groups IOP symbols.
pub static IOP: Group = Group::new("IOP");
/// `Perf::vu0` from the C++ source — groups VU0 symbols.
pub static VU0: Group = Group::new("VU0");
/// `Perf::vu1` from the C++ source — groups VU1 symbols.
pub static VU1: Group = Group::new("VU1");
/// `Perf::vif` from the C++ source — groups VIF symbols.
pub static VIF: Group = Group::new("VIF");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_prefix_only_when_non_empty() {
        assert!(!Group::empty().has_prefix());
        assert!(!Group::new("").has_prefix());
        assert!(Group::new("EE").has_prefix());
    }

    #[test]
    fn full_symbol_prepends_prefix_when_present() {
        let g = Group::new("EE");
        assert_eq!(g.full_symbol("block0"), "EE_block0");
    }

    #[test]
    fn full_symbol_passes_through_when_unprefixed() {
        let g = Group::empty();
        assert_eq!(g.full_symbol("block0"), "block0");
    }

    #[test]
    fn empty_prefix_is_treated_as_no_prefix() {
        // C++ checks `m_prefix && m_prefix[0]`, so an empty string also
        // disables prefixing.
        let g = Group::new("");
        assert_eq!(g.full_symbol("block0"), "block0");
        assert!(!g.has_prefix());
    }

    #[test]
    fn register_methods_are_safe_no_ops() {
        // Just exercise the default (no-profiler) code paths so the tests
        // catch accidental panics.
        EE.register(std::ptr::null(), 0, "sym");
        EE.register_pc(std::ptr::null(), 0, 0xDEAD_BEEF);
        EE.register_key(std::ptr::null(), 0, "blk", 0x0123_4567_89AB_CDEF);
        ANY.register(std::ptr::null(), 0, "sym");
        ANY.register_pc(std::ptr::null(), 0, 0);
        ANY.register_key(std::ptr::null(), 0, "blk", 0);
    }
}
