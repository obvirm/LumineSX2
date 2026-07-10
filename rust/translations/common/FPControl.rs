// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Floating-point control abstraction for the R5900 (EE) translation.
//!
//! Provides an RAII helper to set the x87/SSE FP control register (rounding
//! mode and exception masks). On `x86_64` this targets the MXCSR register
//! through SSE intrinsics. On other architectures the API is preserved but
//! the helpers are no-ops, so callers can compile and run unchanged.

#![allow(dead_code)]

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::{_mm_getcsr, _mm_setcsr};

/// Rounding modes supported by the FPU/MXCSR control register.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FPRoundMode {
    Nearest = 0,
    NegativeInfinity = 1,
    PositiveInfinity = 2,
    ChopZero = 3,
}

/// Denormal handling mode. The x86 MXCSR exposes a single DaZ bit; we model
/// the common configurations explicitly for readability at call sites.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenormalMode {
    /// Preserve denormals in input and output (default).
    Preserve,
    /// Flush denormals to zero in output only.
    FlushToZero,
    /// Treat denormals as zero on input and flush to zero on output.
    DenormalsAreZero,
}

/// A value-type wrapper around the platform floating-point control register.
///
/// On `x86_64` this is a 32-bit MXCSR-shaped bitmask; on other targets it
/// carries an opaque `u32` so the type stays uniform across platforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct FPControlRegister {
    bitmask: u32,
}

impl FPControlRegister {
    // MXCSR layout (x86_64).
    const EXCEPTION_MASK: u32 = 0x3F_u32 << 7;
    const ROUNDING_CONTROL_SHIFT: u32 = 13;
    const ROUNDING_CONTROL_MASK: u32 = 3;
    const ROUNDING_CONTROL_BITS: u32 = Self::ROUNDING_CONTROL_MASK << Self::ROUNDING_CONTROL_SHIFT;
    const DENORMALS_ARE_ZERO_BIT: u32 = 1 << 6;
    const FLUSH_TO_ZERO_BIT: u32 = 1 << 15;

    /// Build a register value from a raw bitmask. Use [`Self::default`] for
    /// the canonical "all exceptions masked, nearest rounding" configuration.
    #[inline]
    pub const fn from_bits(bits: u32) -> Self {
        Self { bitmask: bits }
    }

    /// Read the current hardware control register.
    #[inline]
    pub fn get_current() -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            // SAFETY: `_mm_getcsr` has no preconditions and reads the
            // thread-local MXCSR. It is safe to call at any point.
            let bits = unsafe { _mm_getcsr() };
            Self { bitmask: bits }
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Self::default()
        }
    }

    /// Write this value back to the hardware control register.
    #[inline]
    pub fn set_current(self) {
        #[cfg(target_arch = "x86_64")]
        {
            // SAFETY: `_mm_setcsr` is safe to call provided reserved bits
            // (low 6 bits) are zero, which `_mm_setcsr` itself masks off in
            // practice. We only ever construct values from typed methods.
            unsafe { _mm_setcsr(self.bitmask) };
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            let _ = self.bitmask;
        }
    }

    /// Canonical default: 0x1f80 — all exceptions masked, nearest rounding.
    #[inline]
    pub const fn get_default() -> Self {
        Self { bitmask: 0x1f80 }
    }

    #[inline]
    pub const fn enable_exceptions(&mut self) -> &mut Self {
        self.bitmask &= !Self::EXCEPTION_MASK;
        self
    }

    #[inline]
    pub fn disable_exceptions(&mut self) -> &mut Self {
        self.bitmask |= Self::EXCEPTION_MASK;
        self
    }

    #[inline]
    pub const fn get_round_mode(self) -> FPRoundMode {
        let bits = (self.bitmask >> Self::ROUNDING_CONTROL_SHIFT) & Self::ROUNDING_CONTROL_MASK;
        // SAFETY: bits is in 0..=3, which matches the discriminants of FPRoundMode.
        match bits {
            0 => FPRoundMode::Nearest,
            1 => FPRoundMode::NegativeInfinity,
            2 => FPRoundMode::PositiveInfinity,
            _ => FPRoundMode::ChopZero,
        }
    }

    /// The MXCSR encoding for `FPRoundMode` matches the enum discriminants.
    #[inline]
    pub const fn set_round_mode(&mut self, mode: FPRoundMode) -> &mut Self {
        let bits = (mode as u32) & Self::ROUNDING_CONTROL_MASK;
        self.bitmask = (self.bitmask & !Self::ROUNDING_CONTROL_BITS)
            | (bits << Self::ROUNDING_CONTROL_SHIFT);
        self
    }

    #[inline]
    pub const fn get_denormals_are_zero(self) -> bool {
        (self.bitmask & Self::DENORMALS_ARE_ZERO_BIT) != 0
    }

    #[inline]
    pub const fn set_denormals_are_zero(&mut self, daz: bool) -> &mut Self {
        if daz {
            self.bitmask |= Self::DENORMALS_ARE_ZERO_BIT;
        } else {
            self.bitmask &= !Self::DENORMALS_ARE_ZERO_BIT;
        }
        self
    }

    #[inline]
    pub const fn get_flush_to_zero(self) -> bool {
        (self.bitmask & Self::FLUSH_TO_ZERO_BIT) != 0
    }

    #[inline]
    pub const fn set_flush_to_zero(&mut self, ftz: bool) -> &mut Self {
        if ftz {
            self.bitmask |= Self::FLUSH_TO_ZERO_BIT;
        } else {
            self.bitmask &= !Self::FLUSH_TO_ZERO_BIT;
        }
        self
    }

    #[inline]
    pub const fn bits(self) -> u32 {
        self.bitmask
    }
}

impl Default for FPControlRegister {
    #[inline]
    fn default() -> Self {
        Self::get_default()
    }
}

/// Convenience helpers for the common operations performed on the FP control
/// register. These wrap [`FPControlRegister`] to keep call sites terse and
/// match the original C++ ergonomics.
pub struct FPControl;

impl FPControl {
    /// Construct a new control register with the canonical defaults.
    #[inline]
    pub const fn new() -> Self {
        Self
    }

    /// Apply a rounding mode to the hardware control register.
    #[inline]
    pub fn set_round_mode(mode: FPRoundMode) {
        let mut reg = FPControlRegister::get_current();
        reg.set_round_mode(mode);
        reg.set_current();
    }

    /// Toggle the DaZ bit (denormals-are-zero) on the hardware control register.
    #[inline]
    pub fn set_denormals_are_zero(daz: bool) {
        let mut reg = FPControlRegister::get_current();
        reg.set_denormals_are_zero(daz);
        reg.set_current();
    }

    /// Apply a named denormal handling mode (preserves / FTZ / DaZ+FTZ).
    #[inline]
    pub fn set_denormal_mode(mode: DenormalMode) {
        let mut reg = FPControlRegister::get_current();
        match mode {
            DenormalMode::Preserve => {
                reg.set_flush_to_zero(false);
                reg.set_denormals_are_zero(false);
            }
            DenormalMode::FlushToZero => {
                reg.set_flush_to_zero(true);
                reg.set_denormals_are_zero(false);
            }
            DenormalMode::DenormalsAreZero => {
                reg.set_flush_to_zero(true);
                reg.set_denormals_are_zero(true);
            }
        }
        reg.set_current();
    }
}

impl Default for FPControl {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

/// RAII guard that installs a new FP control register on construction and
/// restores the previous value on drop. Mirrors the C++ `FPControlRegisterBackup`.
pub struct ScopedFPControl {
    previous: FPControlRegister,
}

impl ScopedFPControl {
    /// Capture the current hardware value, then install `new_value`.
    #[inline]
    pub fn new(new_value: FPControlRegister) -> Self {
        let previous = FPControlRegister::get_current();
        new_value.set_current();
        Self { previous }
    }

    /// Construct a guard from a default register value (all exceptions masked,
    /// nearest rounding).
    #[inline]
    pub fn from_default() -> Self {
        Self::new(FPControlRegister::get_default())
    }

    /// Explicitly restore the saved value before the guard goes out of scope.
    #[inline]
    pub fn restore(self) {
        // Move `previous` out so the destructor is a no-op when run.
        let prev = self.previous;
        prev.set_current();
        std::mem::forget(self);
    }
}

impl Drop for ScopedFPControl {
    #[inline]
    fn drop(&mut self) {
        self.previous.set_current();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_mode_roundtrip() {
        let mut reg = FPControlRegister::get_default();
        for mode in [
            FPRoundMode::Nearest,
            FPRoundMode::NegativeInfinity,
            FPRoundMode::PositiveInfinity,
            FPRoundMode::ChopZero,
        ] {
            reg.set_round_mode(mode);
            assert_eq!(reg.get_round_mode(), mode);
        }
    }

    #[test]
    fn denormal_bits_toggle() {
        let mut reg = FPControlRegister::get_default();
        assert!(!reg.get_denormals_are_zero());
        assert!(!reg.get_flush_to_zero());
        reg.set_denormals_are_zero(true);
        assert!(reg.get_denormals_are_zero());
        reg.set_denormals_are_zero(false);
        assert!(!reg.get_denormals_are_zero());
    }
}
