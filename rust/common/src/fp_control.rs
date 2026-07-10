// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Floating-point control register abstractions: MXCSR on x86, FPCR on AArch64.
//!
//! Pure-Rust port of `common/FPControl.h`. The original C++ file is split
//! into two `#ifdef` blocks (one per architecture) but exposes a single
//! `FPControlRegister` struct on each, plus a `FPControlRegisterBackup`
//! RAII helper that swaps in a new value on construction and restores
//! the previous value on destruction.
//!
//! # Strategy
//!
//! The Rust port preserves the same split-arch surface using
//! `#[cfg(target_arch = "...")]` gates, with the per-architecture
//! register read/write factored into a private `imp` module. The public
//! API is 100% safe Rust: all inline-assembly details and the
//! arch-specific bitfield layout live in `FpState`'s methods.
//!
//! The x86 path mirrors the C++ source's use of MXCSR (the SSE control
//! register) rather than the older x87 `fnstenv`/`fldenv` pair. The two
//! registers are distinct: MXCSR controls SSE/AVX rounding and the
//! six SIMD exception masks; the x87 control word controls x87 only.
//! PCSX2's `FPControl.h` documents its register as MXCSR explicitly,
//! so the inline asm emits `stmxcsr` / `ldmxcsr` (always-available on
//! x86_64, no feature gating required).
//!
//! The AArch64 path mirrors the C++ inline asm `mrs x, FPCR` /
//! `msr FPCR, x`.
//!
//! # Rounding-mode encoding
//!
//! The encoding for the rounding-mode field differs between MXCSR
//! (x86) and FPCR (AArch64):
//!
//! | Mode           | x86 MXCSR | AArch64 FPCR |
//! |----------------|-----------|--------------|
//! | Nearest        | 0b00      | 0b00         |
//! | NegativeInf    | 0b01      | 0b10         |
//! | PositiveInf    | 0b10      | 0b01         |
//! | ChopZero       | 0b11      | 0b11         |
//!
//! i.e. the "infinity" modes swap between architectures. The Rust
//! `FpRoundMode` enum follows the C++ canonical order (matching x86);
//! the `get_round_mode` / `set_round_mode` methods translate on the
//! AArch64 path so callers see the same enum on every host.
//!
//! # RAII guard
//!
//! [`FpStateGuard`] captures the current control register on
//! construction, optionally installs a new value, and restores the
//! captured state in `Drop`. The constructor mirrors the C++
//! `FPControlRegisterBackup::FPControlRegisterBackup(FPControlRegister)`
//! contract. The zero-arg `FpStateGuard::new()` form installs the
//! arch's safe default (all exceptions masked, round-to-nearest); use
//! [`FpStateGuard::with`] for an arbitrary replacement.
//!
//! # `unsafe` discipline
//!
//! All `core::arch::asm!` blocks are confined to the private
//! `imp::read` / `imp::write` functions and gated by
//! `#[cfg(target_arch = "...")]`. The public `FpState` API exposes
//! only safe methods; the `unsafe` block is contained and justified
//! inline.
//!
//! # FFI exports
//!
//! **None.** The C++ side already owns the canonical
//! `FPControlRegister` / `FPControlRegisterBackup` types and reads the
//! control register via intrinsics (`_mm_getcsr`) or inline asm
//! (`mrs FPCR`). Cross-FFI coordination is unnecessary for a
//! per-thread register that both sides already manipulate directly.

#![cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]

// ============================================================================
// Architecture-specific register access
// ============================================================================
//
// The `imp` module holds the per-arch register read/write functions
// plus all the bitfield constants. The public API in `FpState`
// delegates to it. Keeping the constants private means callers cannot
// reach in and poke individual bits in ways that would not round-trip
// through `FpState`'s accessors on a different architecture.

#[cfg(target_arch = "x86_64")]
mod imp {
    /// Width of the MXCSR register.
    pub(super) type ControlWord = u32;

    /// Bitmask covering the six MXCSR exception-mask bits (7..=12).
    pub(super) const EXCEPTION_MASK: u32 = 0x3Fu32 << 7;

    /// Bit position of the rounding-control field (RC, bits 13..=14).
    pub(super) const ROUNDING_CONTROL_SHIFT: u32 = 13;
    /// Mask of the rounding-control field (two bits).
    pub(super) const ROUNDING_CONTROL_MASK: u32 = 0x3;
    /// Bitmask of the rounding-control field at its shifted position.
    pub(super) const ROUNDING_CONTROL_BITS: u32 =
        ROUNDING_CONTROL_MASK << ROUNDING_CONTROL_SHIFT;

    /// "Denormals Are Zero" bit (DAZ, bit 6).
    pub(super) const DENORMALS_ARE_ZERO_BIT: u32 = 1u32 << 6;
    /// "Flush To Zero" bit (FTZ, bit 15).
    pub(super) const FLUSH_TO_ZERO_BIT: u32 = 1u32 << 15;

    /// All-exceptions-masked, round-to-nearest, FTZ/DAZ cleared: 0x1F80.
    pub(super) const DEFAULT: u32 = 0x1F80;

    /// Read the current MXCSR register.
    ///
    /// `stmxcsr [mem]` stores the 32-bit MXCSR to memory. The operand
    /// is a memory operand, so we hand the asm a pointer via `in(reg)`.
    #[inline]
    pub(super) fn read() -> ControlWord {
        let mut value: ControlWord = 0;
        // SAFETY: `stmxcsr` writes a 32-bit value to the memory
        // location referenced by the operand. The `&mut value` pointer
        // is valid for a 4-byte write and properly aligned (u32 is
        // 4-byte aligned; the local lives on the stack). No other
        // memory is touched, and the instruction clobbers no flags.
        unsafe {
            core::arch::asm!(
                "stmxcsr [{}]",
                in(reg) &mut value,
                options(nostack, preserves_flags),
            );
        }
        value
    }

    /// Write a value into the MXCSR register.
    ///
    /// `ldmxcsr [mem]` loads the 32-bit MXCSR from memory. Reserved
    /// bits in the supplied value may raise #GP; we do not mask them,
    /// matching the C++ `_mm_setcsr` behaviour (which also trusts the
    /// caller to supply a well-formed control word).
    #[inline]
    pub(super) fn write(value: ControlWord) {
        // SAFETY: `ldmxcsr` reads a 32-bit value from the memory
        // location referenced by the operand. The `&value` pointer is
        // valid for a 4-byte read and properly aligned. The instruction
        // clobbers no flags.
        unsafe {
            core::arch::asm!(
                "ldmxcsr [{}]",
                in(reg) &value,
                options(nostack, preserves_flags),
            );
        }
    }
}

#[cfg(target_arch = "aarch64")]
mod imp {
    /// Width of the FPCR register (only the low 32 bits are defined,
    /// but the architectural register is 64-bit wide; we use `u64` to
    /// match the C++ source's `u64 bitmask`).
    pub(super) type ControlWord = u64;

    /// "Flush To Zero" bit (FZ, bit 24). On AArch64 the single FZ bit
    /// covers both input and output flushing (there is no separate
    /// DAZ bit on cores without FEAT_AFP, including Apple Silicon).
    pub(super) const FZ_BIT: u64 = 1u64 << 24;

    /// Bit position of the rounding-mode field (RM, bits 22..=23).
    pub(super) const RMODE_SHIFT: u32 = 22;
    /// Mask of the rounding-mode field (two bits).
    pub(super) const RMODE_MASK: u64 = 0x3;
    /// Bitmask of the rounding-mode field at its shifted position.
    pub(super) const RMODE_BITS: u64 = RMODE_MASK << RMODE_SHIFT;

    /// Bitmask covering the exception-mask bits (5..=10, IDE/DZE/IOE/...).
    pub(super) const EXCEPTION_MASK: u64 = 0x3Fu << 5;

    /// All-exceptions-masked, round-to-nearest, FZ cleared: 0x0.
    pub(super) const DEFAULT: u64 = 0x0;

    /// Read the current FPCR register.
    #[inline]
    pub(super) fn read() -> ControlWord {
        let value: ControlWord;
        // SAFETY: `mrs x, FPCR` reads the FP control register into a
        // general-purpose register. The instruction has no memory
        // operands and no preconditions; it does not alter any flags
        // or memory the caller could observe.
        unsafe {
            core::arch::asm!(
                "mrs {}, FPCR",
                out(reg) value,
                options(nostack, preserves_flags),
            );
        }
        value
    }

    /// Write a value into the FPCR register.
    #[inline]
    pub(super) fn write(value: ControlWord) {
        // SAFETY: `msr FPCR, x` writes the FP control register from a
        // general-purpose register. The instruction has no memory
        // operands and no preconditions; reserved bits in `value` are
        // architecturally required to read as zero on readback, so
        // there is no undefined-behaviour hazard from caller-supplied
        // patterns.
        unsafe {
            core::arch::asm!(
                "msr FPCR, {}",
                in(reg) value,
                options(nostack, preserves_flags),
            );
        }
    }
}

// ============================================================================
// Public rounding-mode enum
// ============================================================================

/// Floating-point rounding mode.
///
/// The encoding follows the canonical PCSX2 / x86 MXCSR ordering; on
/// AArch64 the `FpState` accessors translate to/from the FPCR-native
/// encoding transparently.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum FpRoundMode {
    /// Round to nearest, ties to even.
    Nearest = 0,
    /// Round towards -infinity.
    NegativeInfinity = 1,
    /// Round towards +infinity.
    PositiveInfinity = 2,
    /// Round towards zero (truncate).
    ChopZero = 3,
}

impl FpRoundMode {
    /// Number of valid rounding modes (excluding `MaxCount`).
    ///
    /// The C++ enum has a `MaxCount` sentinel that we omit because the
    /// `repr(u8)` type makes the array-bound `usize::from` cast cheap
    /// and obvious at the call site. This constant exists so callers
    /// iterating over all modes still have an upper bound.
    pub const COUNT: usize = 4;
}

// ============================================================================
// Public state type
// ============================================================================

/// Saved floating-point control register state.
///
/// On x86_64 the wrapped value is a 32-bit MXCSR; on AArch64 it is the
/// full 64-bit FPCR (only the low 32 bits are defined, but AArch64
/// treats it as a 64-bit register and the C++ source uses `u64`). The
/// `repr(transparent)` attribute keeps the layout identical to the
/// underlying integer type so the type can be transmuted freely by
/// trusted callers (e.g. FFI) without any surprise padding.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct FpState {
    control_word: imp::ControlWord,
}

impl FpState {
    // ---- Direct register access ----------------------------------------

    /// Read the current floating-point control register.
    ///
    /// On x86_64 this is MXCSR; on AArch64 this is FPCR.
    #[inline]
    pub fn get_current() -> Self {
        Self {
            control_word: imp::read(),
        }
    }

    /// Write this state into the floating-point control register.
    ///
    /// On x86_64 the supplied value is loaded as MXCSR (reserved bits
    /// may raise #GP). On AArch64 the value is loaded as FPCR.
    #[inline]
    pub fn set_current(self) {
        imp::write(self.control_word);
    }

    /// All-exceptions-masked, round-to-nearest default.
    ///
    /// Mirrors `FPControlRegister::GetDefault`:
    /// * x86_64: `0x1F80`
    /// * AArch64: `0x0`
    #[inline]
    pub const fn default() -> Self {
        Self {
            control_word: imp::DEFAULT,
        }
    }

    /// Construct a state from a raw control-word value.
    ///
    /// Useful when reconstructing a saved register from an external
    /// source (snapshot file, debug dump, FFI hand-off). Callers are
    /// responsible for ensuring the bit pattern is well-formed on the
    /// target architecture; on x86_64 a malformed MXCSR may raise #GP
    /// when written via [`Self::set_current`].
    ///
    /// Marked `pub(crate)` because the parameter type
    /// ([`imp::ControlWord`]) is not visible outside this module.
    #[allow(dead_code)] // exercised by tests
    #[inline]
    pub(crate) const fn from_control_word(word: imp::ControlWord) -> Self {
        Self { control_word: word }
    }

    /// Extract the raw control-word value.
    ///
    /// Marked `pub(crate)` for the same reason as
    /// [`Self::from_control_word`].
    #[allow(dead_code)] // exercised by tests
    #[inline]
    pub(crate) const fn control_word(self) -> imp::ControlWord {
        self.control_word
    }

    // ---- Exception-mask manipulation -----------------------------------

    /// Enable floating-point exceptions so the corresponding FP
    /// exception traps fire on the next offending instruction.
    ///
    /// The polarity of the underlying bits differs between
    /// architectures:
    ///
    /// * **x86_64 (MXCSR):** the exception bits are *mask* bits
    ///   (`1` = masked/suppressed). Enabling therefore *clears* the
    ///   mask bits.
    /// * **AArch64 (FPCR):** the exception bits are *enable* bits
    ///   (`1` = enabled/trapped). Enabling therefore *sets* the
    ///   enable bits.
    ///
    /// The caller-visible behaviour is identical on both
    /// architectures: after `enable_exceptions`, FP exceptions will be
    /// delivered (assuming the kernel/OS also forwards them).
    #[inline]
    pub fn enable_exceptions(&mut self) -> &mut Self {
        // x86: clear the mask bits to unmask.  AArch64: set the
        // enable bits to enable.  The two architectures use opposite
        // polarity, so the operation is the inverse of `disable`.
        #[cfg(target_arch = "x86_64")]
        {
            self.control_word &= !imp::EXCEPTION_MASK;
        }
        #[cfg(target_arch = "aarch64")]
        {
            self.control_word |= imp::EXCEPTION_MASK;
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            // Unreachable in practice; preserved so the body is total.
        }
        self
    }

    /// Disable floating-point exceptions so the corresponding FP
    /// exception traps are suppressed.
    ///
    /// The polarity is the inverse of [`Self::enable_exceptions`]:
    /// on x86_64 the mask bits are *set*; on AArch64 the enable bits
    /// are *cleared*.
    #[inline]
    pub fn disable_exceptions(&mut self) -> &mut Self {
        #[cfg(target_arch = "x86_64")]
        {
            self.control_word |= imp::EXCEPTION_MASK;
        }
        #[cfg(target_arch = "aarch64")]
        {
            self.control_word &= !imp::EXCEPTION_MASK;
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            // Unreachable in practice; preserved so the body is total.
        }
        self
    }

    /// Returns `true` if every exception is masked.
    ///
    /// Equivalent to "no FP exceptions can be raised".
    #[inline]
    pub fn exceptions_disabled(&self) -> bool {
        (self.control_word & imp::EXCEPTION_MASK) == imp::EXCEPTION_MASK
    }

    /// Returns `true` if every exception is unmasked.
    #[inline]
    pub fn exceptions_enabled(&self) -> bool {
        (self.control_word & imp::EXCEPTION_MASK) == 0
    }

    // ---- Rounding mode -------------------------------------------------

    /// Get the current rounding mode.
    #[inline]
    pub fn get_round_mode(&self) -> FpRoundMode {
        let raw = self.round_mode_raw();

        #[cfg(target_arch = "x86_64")]
        {
            // x86 encoding matches the canonical enum.
            decode_round_mode(raw)
        }

        #[cfg(target_arch = "aarch64")]
        {
            // AArch64: 00=Nearest, 01=+Inf, 10=-Inf, 11=Zero.
            // Canonical: 00=Nearest, 01=-Inf, 10=+Inf, 11=Zero.
            // 00 and 11 stay; 01 and 10 swap. The C++ expression
            // `(RMode == 0b00 || RMode == 0b11) ? RMode : RMode ^ 0b11`
            // is equivalent to XORing 0b11 when `raw` is 01 or 10.
            let canonical = if raw == 0b00 || raw == 0b11 {
                raw
            } else {
                raw ^ 0b11
            };
            decode_round_mode(canonical)
        }

        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            // Unreachable in practice (the file is gated to the two
            // supported arches above), but keeps the function
            // total so callers don't need to special-case it.
            let _ = raw;
            FpRoundMode::Nearest
        }
    }

    /// Set the rounding mode.
    ///
    /// The supplied mode is in canonical (x86-style) encoding; the
    /// implementation translates to the architecture-native encoding
    /// on AArch64.
    #[inline]
    pub fn set_round_mode(&mut self, mode: FpRoundMode) -> &mut Self {
        let canonical = mode as u8 as u64;

        #[cfg(target_arch = "x86_64")]
        let raw = canonical & (imp::ROUNDING_CONTROL_MASK as u64);

        #[cfg(target_arch = "aarch64")]
        let raw = if mode == FpRoundMode::Nearest || mode == FpRoundMode::ChopZero {
            canonical
        } else {
            canonical ^ 0b11
        } & imp::RMODE_MASK;

        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        let raw = canonical;

        self.write_round_mode_raw(raw);
        self
    }

    // ---- Denormals-are-zero / flush-to-zero ----------------------------

    /// Get the "denormals are zero" (DAZ) configuration.
    ///
    /// On x86_64 this reads the dedicated DAZ bit (bit 6 of MXCSR).
    /// On AArch64, most cores do not have a separate DAZ bit; the FZ
    /// bit (bit 24) controls both input and output flushing, so this
    /// method reports `true` when FZ is set, matching the C++ source
    /// and PCSX2's existing semantics on Apple Silicon (which
    /// implements x86-like behaviour via a vendor-specific extension
    /// not accessible from usermode).
    #[inline]
    pub fn get_denormals_are_zero(&self) -> bool {
        #[cfg(target_arch = "x86_64")]
        {
            (self.control_word & imp::DENORMALS_ARE_ZERO_BIT) != 0
        }

        #[cfg(target_arch = "aarch64")]
        {
            (self.control_word & imp::FZ_BIT) != 0
        }

        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            false
        }
    }

    /// Set the "denormals are zero" (DAZ) configuration.
    ///
    /// See [`Self::get_denormals_are_zero`] for the per-architecture
    /// notes about how DAZ/FZ are coupled on AArch64.
    #[inline]
    pub fn set_denormals_are_zero(&mut self, daz: bool) -> &mut Self {
        #[cfg(target_arch = "x86_64")]
        {
            if daz {
                self.control_word |= imp::DENORMALS_ARE_ZERO_BIT;
            } else {
                self.control_word &= !imp::DENORMALS_ARE_ZERO_BIT;
            }
        }

        #[cfg(target_arch = "aarch64")]
        {
            if daz {
                self.control_word |= imp::FZ_BIT;
            } else {
                self.control_word &= !imp::FZ_BIT;
            }
        }

        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            let _ = daz;
        }

        self
    }

    /// Get the "flush to zero" (FTZ) configuration.
    ///
    /// On x86_64 this reads the dedicated FTZ bit (bit 15 of MXCSR).
    /// On AArch64 this reads the FZ bit (bit 24), which covers both
    /// input and output flushing.
    #[inline]
    pub fn get_flush_to_zero(&self) -> bool {
        #[cfg(target_arch = "x86_64")]
        {
            (self.control_word & imp::FLUSH_TO_ZERO_BIT) != 0
        }

        #[cfg(target_arch = "aarch64")]
        {
            (self.control_word & imp::FZ_BIT) != 0
        }

        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            false
        }
    }

    /// Set the "flush to zero" (FTZ) configuration.
    ///
    /// See [`Self::get_flush_to_zero`] for the per-architecture
    /// notes.
    #[inline]
    pub fn set_flush_to_zero(&mut self, ftz: bool) -> &mut Self {
        #[cfg(target_arch = "x86_64")]
        {
            if ftz {
                self.control_word |= imp::FLUSH_TO_ZERO_BIT;
            } else {
                self.control_word &= !imp::FLUSH_TO_ZERO_BIT;
            }
        }

        #[cfg(target_arch = "aarch64")]
        {
            if ftz {
                self.control_word |= imp::FZ_BIT;
            } else {
                self.control_word &= !imp::FZ_BIT;
            }
        }

        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            let _ = ftz;
        }

        self
    }

    // ---- Internal: arch-specific round-mode bitfield -------------------

    /// Read the raw rounding-mode bits in the architecture-native
    /// encoding.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    fn round_mode_raw(&self) -> u64 {
        ((self.control_word >> imp::ROUNDING_CONTROL_SHIFT) & imp::ROUNDING_CONTROL_MASK) as u64
    }

    /// Read the raw rounding-mode bits in the architecture-native
    /// encoding.
    #[inline]
    #[cfg(target_arch = "aarch64")]
    fn round_mode_raw(&self) -> u64 {
        (self.control_word >> imp::RMODE_SHIFT) & imp::RMODE_MASK
    }

    /// Read the raw rounding-mode bits (fallback for non-supported
    /// architectures).
    #[inline]
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    fn round_mode_raw(&self) -> u64 {
        0
    }

    /// Write the raw rounding-mode bits in the architecture-native
    /// encoding.
    #[inline]
    #[cfg(target_arch = "x86_64")]
    fn write_round_mode_raw(&mut self, raw: u64) {
        let mask = imp::ROUNDING_CONTROL_BITS;
        self.control_word =
            (self.control_word & !mask) | (((raw as u32) & imp::ROUNDING_CONTROL_MASK) << imp::ROUNDING_CONTROL_SHIFT);
    }

    /// Write the raw rounding-mode bits in the architecture-native
    /// encoding.
    #[inline]
    #[cfg(target_arch = "aarch64")]
    fn write_round_mode_raw(&mut self, raw: u64) {
        self.control_word = (self.control_word & !imp::RMODE_BITS) | ((raw & imp::RMODE_MASK) << imp::RMODE_SHIFT);
    }

    /// Write the raw rounding-mode bits (fallback for non-supported
    /// architectures).
    #[inline]
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    fn write_round_mode_raw(&mut self, _raw: u64) {}
}

// ============================================================================
// Rounding-mode decoder
// ============================================================================
//
// Pulled out of `FpState::get_round_mode` so the test suite can hit it
// directly with all four canonical codes. Unreachable in practice
// (the canonical enum has exactly four variants), but the `debug_assert!`
// catches accidental enum-extension bugs in test builds.

#[inline]
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn decode_round_mode(raw: u64) -> FpRoundMode {
    debug_assert!(raw < FpRoundMode::COUNT as u64, "invalid raw round-mode bits");
    match raw as u8 {
        0 => FpRoundMode::Nearest,
        1 => FpRoundMode::NegativeInfinity,
        2 => FpRoundMode::PositiveInfinity,
        _ => FpRoundMode::ChopZero,
    }
}

// ============================================================================
// Standalone save / restore
// ============================================================================

/// Capture the current floating-point control register.
///
/// This is the explicit (non-RAII) counterpart to [`FpStateGuard`].
/// Pair with [`restore_fp_state`] to bracket a code region that needs
/// to modify the control register without affecting the surrounding
/// code:
///
/// ```ignore
/// let saved = save_fp_state();
/// let mut working = FpState::default();
/// working.enable_exceptions();
/// working.set_current();
/// // ... FP-sensitive work ...
/// restore_fp_state(&saved);
/// ```
#[inline]
pub fn save_fp_state() -> FpState {
    FpState::get_current()
}

/// Restore a previously-saved floating-point control register.
///
/// See [`save_fp_state`] for the canonical use pattern.
#[inline]
pub fn restore_fp_state(state: &FpState) {
    state.set_current();
}

// ============================================================================
// RAII guard
// ============================================================================

/// RAII guard that swaps in a new floating-point control register and
/// restores the previous one on drop.
///
/// The constructor captures the *current* register, then installs the
/// supplied `new_state`. When the guard is dropped (normally or via
/// panic unwind), the original register is restored verbatim.
///
/// This mirrors the C++ `FPControlRegisterBackup` class, which has the
/// same constructor signature and the same restore-on-destruction
/// semantics.
///
/// # Example
///
/// ```ignore
/// fn fp_sensitive_work() {
///     let mut desired = FpState::default();
///     desired.enable_exceptions();
///     let _guard = FpStateGuard::with(desired);
///     // FP exceptions are unmasked for the duration of this scope.
///     // ... work ...
/// } // guard drops here, original MXCSR/FPCR restored.
/// ```
///
/// The guard is `!Clone` and not `Copy`: copying it would allow two
/// guards to race for "last drop wins" on the same register, which
/// is rarely what the caller wants.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub struct FpStateGuard {
    /// The state to restore on drop. Captured by value (it's `Copy`).
    prev: FpState,
    /// Set to `false` by [`Self::disarm`] to suppress the restore.
    active: bool,
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
impl FpStateGuard {
    /// Capture the current control register and install the arch's
    /// default state (all exceptions masked, round-to-nearest).
    ///
    /// Equivalent to `FpStateGuard::with(FpState::default())`.
    #[inline]
    pub fn new() -> Self {
        Self::with(FpState::default())
    }

    /// Capture the current control register and install `new_state`.
    ///
    /// The captured value is restored in `Drop`.
    #[inline]
    pub fn with(new_state: FpState) -> Self {
        let prev = FpState::get_current();
        new_state.set_current();
        Self {
            prev,
            active: true,
        }
    }

    /// Disable the restore-on-drop behaviour, consuming the guard.
    ///
    /// After `disarm`, the current control register is left untouched
    /// when the guard would otherwise drop. Useful when the caller has
    /// already restored the original state manually (e.g. via
    /// [`restore_fp_state`]) and wants to avoid a redundant second
    /// write.
    #[inline]
    pub fn disarm(mut self) {
        self.active = false;
    }

    /// Borrow the saved "previous" state.
    ///
    /// Allows the caller to inspect what will be restored on drop
    /// without committing to restoring it (use [`Self::disarm`] to
    /// cancel the restore).
    #[inline]
    pub fn previous(&self) -> FpState {
        self.prev
    }

    /// Returns `true` if the guard will still restore on drop.
    ///
    /// `false` only after [`Self::disarm`].
    #[inline]
    pub fn is_active(&self) -> bool {
        self.active
    }
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
impl Drop for FpStateGuard {
    #[inline]
    fn drop(&mut self) {
        if self.active {
            self.prev.set_current();
        }
    }
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
impl Default for FpStateGuard {
    /// Equivalent to [`FpStateGuard::new`].
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(all(test, target_arch = "x86_64"))]
mod tests {
    use super::*;

    /// `default()` matches the C++ "0x1f80" magic value.
    #[test]
    fn default_value_is_0x1f80() {
        assert_eq!(FpState::default().control_word(), 0x1f80);
    }

    /// Default state has all exceptions masked and round-to-nearest.
    #[test]
    fn default_state_masks_exceptions_and_rounds_nearest() {
        let s = FpState::default();
        assert!(s.exceptions_disabled());
        assert_eq!(s.get_round_mode(), FpRoundMode::Nearest);
        assert!(!s.get_flush_to_zero());
        assert!(!s.get_denormals_are_zero());
    }

    /// Round-trip: save, mutate, restore, observe original value.
    #[test]
    fn round_trip_via_explicit_save_restore() {
        let original = save_fp_state();

        let mut modified = original;
        modified
            .set_round_mode(FpRoundMode::ChopZero)
            .set_flush_to_zero(true);
        modified.set_current();

        let observed = save_fp_state();
        assert_eq!(observed.get_round_mode(), FpRoundMode::ChopZero);
        assert!(observed.get_flush_to_zero());

        restore_fp_state(&original);

        let recovered = save_fp_state();
        assert_eq!(recovered, original);
    }

    /// RAII: guard restores on drop.
    #[test]
    fn guard_restores_on_drop() {
        let original = save_fp_state();

        {
            let mut desired = FpState::default();
            desired
                .set_round_mode(FpRoundMode::NegativeInfinity)
                .set_flush_to_zero(true)
                .enable_exceptions();
            let _g = FpStateGuard::with(desired);

            let observed = save_fp_state();
            assert_eq!(observed.get_round_mode(), FpRoundMode::NegativeInfinity);
            assert!(observed.get_flush_to_zero());
            assert!(observed.exceptions_enabled());
        }

        let recovered = save_fp_state();
        assert_eq!(recovered, original, "guard should have restored original state");
    }

    /// RAII: `disarm` suppresses restore.
    #[test]
    fn guard_disarm_suppresses_restore() {
        let original = save_fp_state();

        {
            let desired = FpState::default();
            let g = FpStateGuard::with(desired);
            assert!(g.is_active(), "guard should be active before disarm");
            g.disarm();
        }

        // Guard disarmed: default should still be installed.
        let observed = save_fp_state();
        assert_eq!(observed.get_round_mode(), FpRoundMode::Nearest);
        assert!(observed.exceptions_disabled());

        // Tidy up for subsequent tests.
        restore_fp_state(&original);
    }

    /// `FpStateGuard::new()` installs the default state.
    #[test]
    fn guard_new_installs_default() {
        let original = save_fp_state();
        {
            let _g = FpStateGuard::new();
            let observed = save_fp_state();
            assert_eq!(observed, FpState::default());
        }
        let recovered = save_fp_state();
        assert_eq!(recovered, original);
    }

    /// All four canonical rounding modes round-trip through the bitfield.
    #[test]
    fn rounding_modes_round_trip() {
        for &mode in &[
            FpRoundMode::Nearest,
            FpRoundMode::NegativeInfinity,
            FpRoundMode::PositiveInfinity,
            FpRoundMode::ChopZero,
        ] {
            let mut s = FpState::default();
            s.set_round_mode(mode);
            assert_eq!(s.get_round_mode(), mode, "round-trip failed for {:?}", mode);
        }
    }
}

#[cfg(all(test, target_arch = "aarch64"))]
mod tests {
    use super::*;

    /// `default()` matches the C++ "0x0" magic value.
    #[test]
    fn default_value_is_0() {
        assert_eq!(FpState::default().control_word(), 0);
    }

    /// Default state has all exceptions masked and round-to-nearest.
    #[test]
    fn default_state_masks_exceptions_and_rounds_nearest() {
        let s = FpState::default();
        assert!(s.exceptions_disabled());
        assert_eq!(s.get_round_mode(), FpRoundMode::Nearest);
        assert!(!s.get_flush_to_zero());
    }

    /// The rounding-mode canonical <-> FPCR flip is symmetric.
    #[test]
    fn rounding_mode_canonical_flip_is_symmetric() {
        // The canonical encoding differs from the FPCR encoding for
        // the two infinity modes. Verify all four canonical values
        // round-trip correctly through the FPCR bitfield.
        for &mode in &[
            FpRoundMode::Nearest,
            FpRoundMode::NegativeInfinity,
            FpRoundMode::PositiveInfinity,
            FpRoundMode::ChopZero,
        ] {
            let mut s = FpState::default();
            s.set_round_mode(mode);

            // Compute the expected raw FPCR encoding for `mode`.
            let expected_raw: u64 = match mode {
                FpRoundMode::Nearest => 0b00,
                FpRoundMode::NegativeInfinity => 0b10, // canonical 01 → FPCR 10
                FpRoundMode::PositiveInfinity => 0b01, // canonical 10 → FPCR 01
                FpRoundMode::ChopZero => 0b11,
            };
            let rmode_field =
                (s.control_word() >> imp::RMODE_SHIFT) & imp::RMODE_MASK;
            assert_eq!(
                rmode_field, expected_raw,
                "FPCR RM field for {:?}",
                mode
            );
            assert_eq!(s.get_round_mode(), mode, "round-trip {:?}", mode);
        }
    }

    /// Round-trip: save, mutate, restore.
    #[test]
    fn round_trip_via_explicit_save_restore() {
        let original = save_fp_state();

        let mut modified = original;
        modified.set_round_mode(FpRoundMode::ChopZero);
        modified.set_current();

        let observed = save_fp_state();
        assert_eq!(observed.get_round_mode(), FpRoundMode::ChopZero);

        restore_fp_state(&original);

        let recovered = save_fp_state();
        assert_eq!(recovered, original);
    }

    /// RAII: guard restores on drop.
    #[test]
    fn guard_restores_on_drop() {
        let original = save_fp_state();

        {
            let mut desired = FpState::default();
            desired.set_round_mode(FpRoundMode::NegativeInfinity);
            desired.enable_exceptions();
            let _g = FpStateGuard::with(desired);

            let observed = save_fp_state();
            assert_eq!(observed.get_round_mode(), FpRoundMode::NegativeInfinity);
            assert!(observed.exceptions_enabled());
        }

        let recovered = save_fp_state();
        assert_eq!(recovered, original, "guard should have restored original state");
    }

    /// RAII: `disarm` suppresses restore.
    #[test]
    fn guard_disarm_suppresses_restore() {
        let original = save_fp_state();

        {
            let desired = FpState::default();
            let g = FpStateGuard::with(desired);
            g.disarm();
        }

        let observed = save_fp_state();
        assert_eq!(observed, FpState::default());

        restore_fp_state(&original);
    }

    /// `FpStateGuard::new()` installs the default state.
    #[test]
    fn guard_new_installs_default() {
        let original = save_fp_state();
        {
            let _g = FpStateGuard::new();
            let observed = save_fp_state();
            assert_eq!(observed, FpState::default());
        }
        let recovered = save_fp_state();
        assert_eq!(recovered, original);
    }
}