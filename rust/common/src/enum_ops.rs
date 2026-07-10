// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Enum operations — Rust translation of `common/EnumOps.h`.
//!
//! This module provides the building blocks for working with C-style
//! enumerations in idiomatic Rust:
//!
//! - [`EnumRepr`] — exposes an enum's underlying integer type and offers
//!   the [`EnumRepr::repr`] / [`EnumRepr::from_repr`] conversions. Use the
//!   free function [`enum_cast`] for C++-style call sites.
//! - [`EnumFlags`] — a marker trait for enums that behave as bit-flag
//!   sets. Once an enum implements the required `core::ops` traits, an
//!   empty `impl EnumFlags for MyEnum {}` unlocks the higher-level
//!   helpers (`is_empty`, `intersects`, `contains`, `contains_any`).
//! - An `impl_enum_flags!` macro that wires all the bitwise operator
//!   impls in one line (the Rust equivalent of C++'s
//!   `MARK_ENUM_AS_FLAGS(T)`).
//! - A handful of `#[no_mangle] extern "C"` shims so the C++ core can
//!   reach the bitwise operators without depending on Rust generics.

use core::fmt::Debug;
use core::ops::{
    BitAnd, BitAndAssign, BitOr, BitOrAssign, BitXor, BitXorAssign, Not,
};

// ---------------------------------------------------------------------------
// EnumRepr
// ---------------------------------------------------------------------------

/// Exposes an enumeration's underlying integer representation.
///
/// Implement this on any enum whose variants you need to round-trip
/// between Rust and C/C++. The associated type [`Repr`](Self::Repr) is
/// typically `u8`, `u16`, `u32`, or `u64`.
///
/// The [`repr`](Self::repr) method is the Rust analogue of C++'s
/// `enum_cast<Enum>(value)`. The reverse conversion
/// ([`from_repr`](Self::from_repr)) is user-supplied because
/// `#[repr($repr)]` enums in Rust have no built-in safe way to convert
/// an arbitrary integer back into the enum type.
///
/// # Example
/// ```
/// use pcsx2_common_rs::enum_ops::{EnumRepr, enum_cast};
///
/// #[derive(Copy, Clone, Debug, PartialEq, Eq)]
/// #[repr(u8)]
/// enum Side { Left = 0, Right = 1 }
///
/// impl EnumRepr for Side {
///     type Repr = u8;
///     fn repr(self) -> u8 { self as u8 }
///     fn from_repr(repr: u8) -> Self {
///         if repr == 0 { Side::Left } else { Side::Right }
///     }
/// }
///
/// assert_eq!(enum_cast(Side::Right), 1u8);
/// assert_eq!(Side::Right, Side::from_repr(1u8));
/// ```
pub trait EnumRepr: Copy {
    /// The underlying integer representation of the enum.
    type Repr: Copy + Eq + Debug;

    /// Returns the underlying integer value of `self`.
    fn repr(self) -> Self::Repr;

    /// Reconstructs the enum from its underlying integer value.
    ///
    /// Implementations are free to saturate, wrap, or panic on values
    /// outside the defined variants — the contract is "round-trip with
    /// [`repr`](Self::repr) for any value that came from a valid
    /// variant".
    fn from_repr(repr: Self::Repr) -> Self;
}

/// C++-style convenience wrapper around [`EnumRepr::repr`].
///
/// `enum_cast(value)` reads more naturally than `value.repr()` at call
/// sites that already speak in terms of casts. It is zero-cost;
/// `const` is not yet possible because [`EnumRepr::repr`] is a trait
/// method (const traits are unstable on stable Rust).
#[inline]
pub fn enum_cast<E: EnumRepr>(value: E) -> E::Repr {
    value.repr()
}

// ---------------------------------------------------------------------------
// EnumFlags
// ---------------------------------------------------------------------------

/// Marker trait for enums that support the full set of bitwise operators.
///
/// In C++ these operators are enabled per-enum via
/// `MARK_ENUM_AS_FLAGS(MyEnum)`. In Rust the equivalent is to:
///
/// 1. Implement `EnumRepr` so the underlying integer type is known.
/// 2. Implement `core::ops::{BitOr, BitAnd, BitXor, BitOrAssign,
///    BitAndAssign, BitXorAssign, Not}` for the enum.
/// 3. Add an empty `impl EnumFlags for MyEnum {}`.
///
/// `EnumFlags` then provides `is_empty`, `intersects`, `contains`, and
/// `contains_any` for free, plus a default implementation of `all()`.
///
/// Most users will prefer the [`impl_enum_flags!`] macro, which performs
/// all three steps at once. See its documentation for the canonical
/// example.
pub trait EnumFlags:
    Copy
    + Eq
    + BitOr<Output = Self>
    + BitOrAssign
    + BitAnd<Output = Self>
    + BitAndAssign
    + BitXor<Output = Self>
    + BitXorAssign
    + Not<Output = Self>
{
    /// Identity element — the variant whose underlying value is zero.
    fn empty() -> Self;

    /// Element with every flag bit set. Defaults to `!Self::empty()`,
    /// which is the natural inverse for binary flag enums.
    #[inline]
    fn all() -> Self {
        !Self::empty()
    }

    /// `true` when no bits are set.
    #[inline]
    fn is_empty(self) -> bool {
        self == Self::empty()
    }

    /// `true` when `self` and `other` share at least one bit.
    #[inline]
    fn intersects(self, other: Self) -> bool {
        !(self & other).is_empty()
    }

    /// `true` when every bit in `other` is also set in `self`.
    #[inline]
    fn contains(self, other: Self) -> bool {
        (self & other) == other
    }

    /// `true` when any bit of `other` is set in `self` (alias for
    /// [`intersects`](Self::intersects); kept for readability at call
    /// sites where "contains" reads better than "intersects").
    #[inline]
    fn contains_any(self, other: Self) -> bool {
        self.intersects(other)
    }
}

// ---------------------------------------------------------------------------
// impl_enum_flags! macro
// ---------------------------------------------------------------------------

/// Wires `EnumRepr` (using `mem::transmute` for `from_repr`) and the
/// bitwise operator traits for an enum, and registers it as
/// `EnumFlags`. This is the Rust equivalent of C++'s
/// `MARK_ENUM_AS_FLAGS(MyEnum)`.
///
/// The macro expects the enum to:
///
/// - Be `#[repr($repr)]` (the macro uses `as` casts in one direction
///   and `mem::transmute` in the other; both are sound for
///   `#[repr(primitive)]` enums).
/// - Have a variant equal to `$empty` (typically the all-zero variant).
///
/// The generated `from_repr` uses `mem::transmute`, which is sound
/// for any `#[repr($repr)]` enum because the bit pattern is always
/// a valid representation. Values that don't correspond to a named
/// variant are still legal `Self` values — they just compare unequal
/// to every named variant.
///
/// Users who want to reject out-of-range bit patterns should write
/// their own `EnumRepr` impl (the macro's auto-generated impl can be
/// shadowed with a manual one).
///
/// # Example
/// ```
/// use pcsx2_common_rs::enum_ops::{EnumFlags, EnumRepr};
/// use pcsx2_common_rs::impl_enum_flags;
///
/// #[derive(Copy, Clone, Debug, PartialEq, Eq)]
/// #[repr(u8)]
/// #[allow(clippy::enum_variant_names)]
/// enum Perm {
///     None  = 0x0,
///     Read  = 0x1,
///     Write = 0x2,
///     Exec  = 0x4,
/// }
///
/// impl_enum_flags!(Perm: u8 = Perm::None);
///
/// let rwx = Perm::Read | Perm::Write | Perm::Exec;
/// assert_eq!(rwx.repr(), 0x7);
/// assert!(rwx.contains(Perm::Write));
/// assert!(rwx.intersects(Perm::Exec));
/// assert!(Perm::None.is_empty());
/// assert_eq!(Perm::all().repr(), 0xFF);
/// ```
#[macro_export]
macro_rules! impl_enum_flags {
    ($enum:ty : $repr:ty = $empty:expr) => {
        impl $crate::enum_ops::EnumRepr for $enum {
            type Repr = $repr;
            #[inline]
            fn repr(self) -> $repr {
                self as $repr
            }
            #[inline]
            fn from_repr(repr: $repr) -> Self {
                // Sound: for `#[repr($repr)]` enums, every $repr value is
                // a legal bit pattern. The compiler considers values
                // outside the named discriminants "uninhabited" but the
                // memory layout is well-defined, so the bit-for-bit
                // cast is safe. We use `transmute_copy` rather than
                // `transmute` to bypass the runtime enum-validity
                // assertion that `transmute` adds for enums in debug
                // builds (which would abort on values like
                // `Read | Write = 0x3` that don't correspond to a single
                // named variant).
                unsafe {
                    ::core::mem::transmute_copy::<$repr, $enum>(&repr)
                }
            }
        }

        impl ::core::ops::BitOr for $enum {
            type Output = $enum;
            #[inline]
            fn bitor(self, rhs: $enum) -> $enum {
                <Self as $crate::enum_ops::EnumRepr>::from_repr(self as $repr | rhs as $repr)
            }
        }

        impl ::core::ops::BitAnd for $enum {
            type Output = $enum;
            #[inline]
            fn bitand(self, rhs: $enum) -> $enum {
                <Self as $crate::enum_ops::EnumRepr>::from_repr(self as $repr & rhs as $repr)
            }
        }

        impl ::core::ops::BitXor for $enum {
            type Output = $enum;
            #[inline]
            fn bitxor(self, rhs: $enum) -> $enum {
                <Self as $crate::enum_ops::EnumRepr>::from_repr(self as $repr ^ rhs as $repr)
            }
        }

        impl ::core::ops::BitOrAssign for $enum {
            #[inline]
            fn bitor_assign(&mut self, rhs: $enum) {
                *self = *self | rhs;
            }
        }

        impl ::core::ops::BitAndAssign for $enum {
            #[inline]
            fn bitand_assign(&mut self, rhs: $enum) {
                *self = *self & rhs;
            }
        }

        impl ::core::ops::BitXorAssign for $enum {
            #[inline]
            fn bitxor_assign(&mut self, rhs: $enum) {
                *self = *self ^ rhs;
            }
        }

        impl ::core::ops::Not for $enum {
            type Output = $enum;
            #[inline]
            fn not(self) -> $enum {
                <Self as $crate::enum_ops::EnumRepr>::from_repr(!(self as $repr))
            }
        }

        impl $crate::enum_ops::EnumFlags for $enum {
            #[inline]
            fn empty() -> Self {
                $empty
            }
        }
    };
}

// ---------------------------------------------------------------------------
// FFI shims (u32 only — the only width PCSX2 passes through EnumOps in
// the original C++ codebase today).
// ---------------------------------------------------------------------------

/// FFI: identity cast passthrough.
///
/// Useful from C++ when a generic `<T>` template would otherwise have to
/// dispatch on the type at the call site — `enum_cast` always widens to
/// `u32` here so the C++ side never has to know the Rust underlying type.
#[no_mangle]
pub extern "C" fn pcsx2_enum_cast_u32(value: u32) -> u32 {
    value
}

/// FFI: bitwise OR.
#[no_mangle]
pub extern "C" fn pcsx2_enum_or_u32(a: u32, b: u32) -> u32 {
    a | b
}

/// FFI: bitwise AND.
#[no_mangle]
pub extern "C" fn pcsx2_enum_and_u32(a: u32, b: u32) -> u32 {
    a & b
}

/// FFI: bitwise XOR.
#[no_mangle]
pub extern "C" fn pcsx2_enum_xor_u32(a: u32, b: u32) -> u32 {
    a ^ b
}

/// FFI: bitwise NOT (one's complement).
#[no_mangle]
pub extern "C" fn pcsx2_enum_not_u32(a: u32) -> u32 {
    !a
}

/// FFI: test if any bit of `b` is set in `a`.
///
/// Equivalent to C++ `!((a & b) == 0)`; the `bool` return type matches
/// the C `bool` ABI used by PCSX2 (single byte on every supported
/// platform).
#[no_mangle]
pub extern "C" fn pcsx2_enum_test_u32(a: u32, b: u32) -> bool {
    (a & b) != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffi_shims_match_core_ops() {
        assert_eq!(pcsx2_enum_cast_u32(0xDEAD_BEEF), 0xDEAD_BEEF);
        assert_eq!(pcsx2_enum_or_u32(0x0F, 0xF0), 0xFF);
        assert_eq!(pcsx2_enum_and_u32(0xFF, 0x0F), 0x0F);
        assert_eq!(pcsx2_enum_xor_u32(0xFF, 0x0F), 0xF0);
        assert_eq!(pcsx2_enum_not_u32(0), u32::MAX);
        assert!(pcsx2_enum_test_u32(0b1010, 0b0011));
        assert!(!pcsx2_enum_test_u32(0b1010, 0b0101));
        // edge cases: no bits set, no overlap
        assert!(!pcsx2_enum_test_u32(0, 0));
        assert!(!pcsx2_enum_test_u32(0xFFFF_FFFF, 0));
    }

    #[test]
    fn enum_cast_round_trip() {
        #[derive(Copy, Clone, Debug, PartialEq, Eq)]
        #[repr(u8)]
        enum Direction { Up = 0, Down = 1, Left = 2, Right = 3 }

        impl EnumRepr for Direction {
            type Repr = u8;
            fn repr(self) -> u8 { self as u8 }
            fn from_repr(repr: u8) -> Self {
                match repr {
                    0 => Direction::Up,
                    1 => Direction::Down,
                    2 => Direction::Left,
                    _ => Direction::Right,
                }
            }
        }

        for d in [Direction::Up, Direction::Down, Direction::Left, Direction::Right] {
            assert_eq!(Direction::from_repr(enum_cast(d)), d);
        }
        assert_eq!(enum_cast(Direction::Up), 0u8);
        assert_eq!(enum_cast(Direction::Right), 3u8);
    }

    #[test]
    fn impl_enum_flags_macro_wires_all_operators() {
        #[derive(Copy, Clone, Debug, PartialEq, Eq)]
        #[repr(u8)]
        #[allow(clippy::enum_variant_names)]
        enum Perm {
            None  = 0x0,
            Read  = 0x1,
            Write = 0x2,
            Exec  = 0x4,
        }

        impl_enum_flags!(Perm: u8 = Perm::None);

        // `|` and assignment
        let mut p = Perm::Read;
        p |= Perm::Write;
        assert_eq!(p, Perm::Read | Perm::Write);

        // `&` masks
        assert_eq!(p & Perm::Write, Perm::Write);
        assert_eq!(p & Perm::Exec, Perm::None);

        // `^` toggles
        assert_eq!(p ^ Perm::Exec, Perm::Read | Perm::Write | Perm::Exec);

        // `!` flips all bits
        let inverted = !Perm::None;
        assert_eq!(inverted.repr(), !0u8);

        // EnumFlags helpers
        assert!(Perm::None.is_empty());
        assert!(!p.is_empty());
        assert!(p.intersects(Perm::Read));
        assert!(!p.intersects(Perm::Exec));
        assert!(p.contains(Perm::Write));
        assert!(!p.contains(Perm::Exec));
        assert!(!p.contains_any(Perm::Exec));
        assert_eq!(Perm::all().repr(), 0xFF);

        // Combine all three flags
        let rwx = Perm::Read | Perm::Write | Perm::Exec;
        assert_eq!(rwx.repr(), 0x7);
        assert!(rwx.contains(Perm::Write));
        assert!(rwx.intersects(Perm::Exec));

        // EnumRepr round-trip via the macro-generated impl
        assert_eq!(Perm::from_repr(0x3).repr(), 0x3);
    }
}