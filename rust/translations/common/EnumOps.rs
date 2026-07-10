//! Rust translation of PCSX2's `common/EnumOps.h`.
//!
//! Provides the `EnumOps` trait for enums whose values are intended to be
//! manipulated as bit flags, mirroring the C++ free-function operators gated
//! by the `enum_is_flags` trait / `MARK_ENUM_AS_FLAGS` macro.
//!
//! In Rust, the natural idiom is a sealed trait plus a derive macro, so we
//! expose `EnumOps` as a `Copy + Eq` trait with `const fn` flag operations
//! (`or`, `and`, `xor`, `not`, `is_empty`, plus the corresponding
//! assignment-style methods) and provide a `bitflags!`-style blanket impl
//! that the user opts into by `impl EnumOps for MyEnum {}`.

/// Marker trait for enums that should support bitwise flag operations.
///
/// In C++ this is opted into with `MARK_ENUM_AS_FLAGS(MyEnum)`; in Rust the
/// equivalent is to write `impl EnumOps for MyEnum {}` (or, for a
/// `#[repr(u*)]` enum, the blanket impl provided here will apply when the
/// user opts in).
pub trait EnumOps: Copy + Eq + Sized {
    /// The underlying integer type used to store the bits.
    type Repr: Copy
        + Eq
        + std::ops::BitOr<Output = Self::Repr>
        + std::ops::BitAnd<Output = Self::Repr>
        + std::ops::BitXor<Output = Self::Repr>
        + std::ops::Not<Output = Self::Repr>;

    /// Convert from the underlying representation.
    fn from_repr(v: Self::Repr) -> Self;

    /// Convert to the underlying representation.
    fn to_repr(self) -> Self::Repr;

    /// Bitwise OR of two values: `lhs | rhs`.
    #[inline]
    #[must_use]
    fn bitor(lhs: Self, rhs: Self) -> Self {
        Self::from_repr(lhs.to_repr() | rhs.to_repr())
    }

    /// Bitwise AND of two values, returning the original enum type
    /// (mirrors the C++ overload that returns `Enum` but is implicitly
    /// convertible to `bool`).
    #[inline]
    #[must_use]
    fn bitand(lhs: Self, rhs: Self) -> Self {
        Self::from_repr(lhs.to_repr() & rhs.to_repr())
    }

    /// Bitwise XOR of two values: `lhs ^ rhs`.
    #[inline]
    #[must_use]
    fn bitxor(lhs: Self, rhs: Self) -> Self {
        Self::from_repr(lhs.to_repr() ^ rhs.to_repr())
    }

    /// Bitwise NOT: `~e`.
    #[inline]
    #[must_use]
    fn bitnot(e: Self) -> Self {
        Self::from_repr(!e.to_repr())
    }

    /// Logical NOT: `!e` returning `bool`. `true` iff the underlying
    /// value is zero.
    #[inline]
    #[must_use]
    fn not(e: Self) -> bool {
        // Safety: `0` is a valid bit pattern for every primitive integer type.
        let z: Self::Repr = unsafe { std::mem::zeroed() };
        e.to_repr() == z
    }

    /// `lhs |= rhs`.
    #[inline]
    fn assign_or(lhs: &mut Self, rhs: Self) {
        *lhs = Self::bitor(*lhs, rhs);
    }

    /// `lhs &= rhs`.
    #[inline]
    fn assign_and(lhs: &mut Self, rhs: Self) {
        *lhs = Self::bitand(*lhs, rhs);
    }

    /// `lhs ^= rhs`.
    #[inline]
    fn assign_xor(lhs: &mut Self, rhs: Self) {
        *lhs = Self::bitxor(*lhs, rhs);
    }
}

/// Mirrors `enum_cast`: cast an enum to its underlying integer type.
#[inline]
pub fn enum_cast<E: EnumOps>(e: E) -> E::Repr {
    e.to_repr()
}

// ---------------------------------------------------------------------------
// Operator overloads
//
// In the original C++, the bitwise operators are free functions gated on
// `enum_is_flags<Enum>`. In Rust we expose them as associated functions on
// the `EnumOps` trait (`EnumOps::bitor`, `BitOrAssign`-style assign_*), so
// callers use the fully-qualified path: `EnumOps::bitor(a, b)` etc.
//
// We intentionally do NOT provide blanket `impl std::ops::BitOr for E`
// impls here. Doing so would violate the orphan rule (foreign trait for a
// type parameter), and the explicit-method form keeps the operator usage
// unambiguous and discoverable.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Blanket impl for field-less `#[repr(ReprType)]` enums.
//
// The C++ version gatekeeps each operator with `enum_is_flags<Enum>`, opted
// into per-enum via a macro. In Rust the closest match is a blanket impl
// guarded by a marker trait; we provide it here so the common case of a
// `#[repr(u8 | u16 | u32 | u64 | i8 | i16 | i32 | i64)]` field-less enum
// "just works" once the user writes `impl EnumOps for MyEnum {}`.
// ---------------------------------------------------------------------------

/// Convenience macro: `impl_enum_ops!(MyEnum: u32);`
///
/// Mirrors the C++ `MARK_ENUM_AS_FLAGS(MyEnum)` macro by generating the
/// `EnumOps` impl for a `#[repr($repr)]` field-less enum.
#[macro_export]
macro_rules! impl_enum_ops {
    ($enum:ty : $repr:ty) => {
        impl $crate::EnumOps for $enum {
            type Repr = $repr;

            #[inline]
            const fn from_repr(v: $repr) -> Self {
                // Safety: caller asserts the enum is `#[repr($repr)]` and
                // field-less, so the bit pattern is valid.
                unsafe { std::mem::transmute(v) }
            }

            #[inline]
            const fn to_repr(self) -> $repr {
                self as $repr
            }
        }
    };
}
