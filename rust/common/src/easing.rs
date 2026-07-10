// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
// Pure-Rust port of `common/Easing.h`.
// Original C++ source: https://github.com/nicolausYes/easing-functions
//   (MIT, Copyright (c) 2016 Nicolaus Yes).

//! Easing functions for animation curves.
//!
//! Each function maps a normalized time `t` (typically in `[0, 1]`) to a
//! normalized progress value (also typically in `[0, 1]`). Functions are
//! grouped into three flavors per curve family:
//!
//! - `in_*`     — slow start, fast finish.
//! - `out_*`    — fast start, slow finish.
//! - `in_out_*` — slow start AND slow finish, fast middle.
//!
//! The trait [`Easing`] exposes all 30 functions as methods so callers can
//! write `Easing::in_sine(t)` or, equivalently, `<f32 as Easing>::in_sine(t)`.
//! The free functions `in_sine(t)`, `out_sine(t)`, etc. are also provided
//! for callers who prefer them, and `f32` is the canonical concrete type.
//!
//! ## FFI
//!
//! Every curve is also exposed as a `pcsx2_easing_*` extern "C" function
//! taking and returning `f32` for use from the C++ UI animation layer. The
//! `f32` and the `f64` instantiations of the free functions are produced by
//! the generic definitions; the FFI trampolines just specialise to `f32`.

// ============================================================================
// Constants
// ============================================================================

/// Pi, matching the original C++ constant exactly (10 significant digits).
/// The C++ header uses `3.1415926545f`, so we mirror that bit-for-bit here
/// rather than `std::f32::consts::PI` (which is `3.14159274`) or
/// `std::f64::consts::PI`.
pub const PI: f32 = 3.1415926545;

/// Pi divided by 2 (`1.5707963f` in the C++ source). Used by the sine
/// family.
const HALF_PI: f32 = 1.5707963;

// ============================================================================
// Easing trait
// ============================================================================

/// Easing curves, parameterised over any float type.
///
/// All implementations are pure-Rust and `#[inline]` so the optimiser can
/// fold them into the caller; this is what the C++ source did via
/// `__ri` (always-inline hint).
pub trait Easing {
    // -- Sine ---------------------------------------------------------------
    /// `1 - cos(t * pi/2)`, but expressed as `sin(t * pi/2)` which is
    /// numerically equivalent and one less trig call.
    fn in_sine(t: Self) -> Self;
    /// `sin((t - 1) * pi/2) + 1`.
    fn out_sine(t: Self) -> Self;
    /// `0.5 * (1 + sin((t - 0.5) * pi))`.
    fn in_out_sine(t: Self) -> Self;

    // -- Quadratic ----------------------------------------------------------
    /// `t * t`.
    fn in_quad(t: Self) -> Self;
    /// `t * (2 - t)`.
    fn out_quad(t: Self) -> Self;
    /// Spline at `t = 0.5`; equivalent to `2*t*t` for `t < 0.5`,
    /// `t*(4 - 2*t) - 1` otherwise.
    fn in_out_quad(t: Self) -> Self;

    // -- Cubic --------------------------------------------------------------
    /// `t^3`.
    fn in_cubic(t: Self) -> Self;
    /// `1 + (t - 1)^3`.
    fn out_cubic(t: Self) -> Self;
    /// Symmetric cubic blend.
    fn in_out_cubic(t: Self) -> Self;

    // -- Quartic ------------------------------------------------------------
    /// `t^4`.
    fn in_quart(t: Self) -> Self;
    /// `1 - (t - 1)^4`.
    fn out_quart(t: Self) -> Self;
    /// Symmetric quartic blend.
    fn in_out_quart(t: Self) -> Self;

    // -- Quintic ------------------------------------------------------------
    /// `t^5`.
    fn in_quint(t: Self) -> Self;
    /// `1 + (t - 1)^5`.
    fn out_quint(t: Self) -> Self;
    /// Symmetric quintic blend.
    fn in_out_quint(t: Self) -> Self;

    // -- Exponential --------------------------------------------------------
    /// `(2^(8t) - 1) / 255`.
    fn in_expo(t: Self) -> Self;
    /// `1 - 2^(-8t)`.
    fn out_expo(t: Self) -> Self;
    /// Spline blend; uses `2^(16t) / 510` for the first half.
    fn in_out_expo(t: Self) -> Self;

    // -- Circular -----------------------------------------------------------
    /// `1 - sqrt(1 - t)`.
    fn in_circ(t: Self) -> Self;
    /// `sqrt(t)`.
    fn out_circ(t: Self) -> Self;
    /// Symmetric circular blend.
    fn in_out_circ(t: Self) -> Self;

    // -- Back (overshoot) ----------------------------------------------------
    /// `t^2 * (c1 * t - c2)` with `c1 = 2.70158`, `c2 = 1.70158`.
    fn in_back(t: Self) -> Self;
    /// `1 + (t - 1)^2 * (c1 * (t - 1) + c2)`.
    fn out_back(t: Self) -> Self;
    /// Symmetric back blend; uses the larger `c1 = 7`, `c2 = 2.5`.
    fn in_out_back(t: Self) -> Self;

    // -- Elastic (spring) ----------------------------------------------------
    /// `t^4 * sin(t * pi * 4.5)`.
    fn in_elastic(t: Self) -> Self;
    /// `1 - (t - 1)^4 * cos(t * pi * 4.5)`.
    fn out_elastic(t: Self) -> Self;
    /// Three-segment elastic; uses `sin` for the outer segments and a flat
    /// offset for the centre, matching the original C++ implementation.
    fn in_out_elastic(t: Self) -> Self;

    // -- Bounce --------------------------------------------------------------
    /// `2^(6(t - 1)) * |sin(t * pi * 3.5)|`.
    fn in_bounce(t: Self) -> Self;
    /// `1 - 2^(-6t) * |cos(t * pi * 3.5)|`.
    fn out_bounce(t: Self) -> Self;
    /// Symmetric bounce blend.
    fn in_out_bounce(t: Self) -> Self;
}

// ============================================================================
// `f32` implementation
// ============================================================================

/// Standard `f32` instantiation of every easing curve.
///
/// All methods are `#[inline]` (via `#[inline(always)]`) to mirror the C++
/// `__ri` ("always inline") annotation on the originals.
impl Easing for f32 {
    #[inline(always)]
    fn in_sine(t: f32) -> f32 {
        (HALF_PI * t).sin()
    }

    #[inline(always)]
    fn out_sine(t: f32) -> f32 {
        1.0 + ((HALF_PI * (t - 1.0))).sin()
    }

    #[inline(always)]
    fn in_out_sine(t: f32) -> f32 {
        0.5 * (1.0 + (PI * (t - 0.5)).sin())
    }

    #[inline(always)]
    fn in_quad(t: f32) -> f32 {
        t * t
    }

    #[inline(always)]
    fn out_quad(t: f32) -> f32 {
        t * (2.0 - t)
    }

    #[inline(always)]
    fn in_out_quad(t: f32) -> f32 {
        if t < 0.5 {
            2.0 * t * t
        } else {
            t * (4.0 - 2.0 * t) - 1.0
        }
    }

    #[inline(always)]
    fn in_cubic(t: f32) -> f32 {
        t * t * t
    }

    #[inline(always)]
    fn out_cubic(t: f32) -> f32 {
        let t = t - 1.0;
        1.0 + t * t * t
    }

    #[inline(always)]
    fn in_out_cubic(t: f32) -> f32 {
        if t < 0.5 {
            4.0 * t * t * t
        } else {
            let t = t - 1.0;
            let u = 2.0 * t;
            1.0 + t * u * u
        }
    }

    #[inline(always)]
    fn in_quart(t: f32) -> f32 {
        let t2 = t * t;
        t2 * t2
    }

    #[inline(always)]
    fn out_quart(t: f32) -> f32 {
        let t = (t - 1.0) * t;
        1.0 - t * t
    }

    #[inline(always)]
    fn in_out_quart(t: f32) -> f32 {
        if t < 0.5 {
            let t2 = t * t;
            8.0 * t2 * t2
        } else {
            let t = (t - 1.0) * t;
            1.0 - 8.0 * t * t
        }
    }

    #[inline(always)]
    fn in_quint(t: f32) -> f32 {
        let t2 = t * t;
        t * t2 * t2
    }

    #[inline(always)]
    fn out_quint(t: f32) -> f32 {
        let t2 = (t - 1.0) * t;
        1.0 + t * t2 * t2
    }

    #[inline(always)]
    fn in_out_quint(t: f32) -> f32 {
        if t < 0.5 {
            let t2 = t * t;
            16.0 * t * t2 * t2
        } else {
            let t2 = (t - 1.0) * t;
            1.0 + 16.0 * t * t2 * t2
        }
    }

    #[inline(always)]
    fn in_expo(t: f32) -> f32 {
        (2f32.powf(8.0 * t) - 1.0) / 255.0
    }

    #[inline(always)]
    fn out_expo(t: f32) -> f32 {
        1.0 - (-8.0 * t).exp2()
    }

    #[inline(always)]
    fn in_out_expo(t: f32) -> f32 {
        if t < 0.5 {
            (16.0 * t).exp2() / 510.0
        } else {
            1.0 - 0.5 * (-16.0 * (t - 0.5)).exp2()
        }
    }

    #[inline(always)]
    fn in_circ(t: f32) -> f32 {
        1.0 - (1.0 - t).sqrt()
    }

    #[inline(always)]
    fn out_circ(t: f32) -> f32 {
        t.sqrt()
    }

    #[inline(always)]
    fn in_out_circ(t: f32) -> f32 {
        if t < 0.5 {
            (1.0 - (1.0 - 2.0 * t).sqrt()) * 0.5
        } else {
            (1.0 + (2.0 * t - 1.0).sqrt()) * 0.5
        }
    }

    #[inline(always)]
    fn in_back(t: f32) -> f32 {
        t * t * (2.70158 * t - 1.70158)
    }

    #[inline(always)]
    fn out_back(t: f32) -> f32 {
        let t = t - 1.0;
        1.0 + t * t * (2.70158 * t + 1.70158)
    }

    #[inline(always)]
    fn in_out_back(t: f32) -> f32 {
        if t < 0.5 {
            t * t * (7.0 * t - 2.5) * 2.0
        } else {
            let t = t - 1.0;
            1.0 + t * t * 2.0 * (7.0 * t + 2.5)
        }
    }

    #[inline(always)]
    fn in_elastic(t: f32) -> f32 {
        let t2 = t * t;
        t2 * t2 * (t * PI * 4.5).sin()
    }

    #[inline(always)]
    fn out_elastic(t: f32) -> f32 {
        let t2 = (t - 1.0) * (t - 1.0);
        1.0 - t2 * t2 * (t * PI * 4.5).cos()
    }

    #[inline(always)]
    fn in_out_elastic(t: f32) -> f32 {
        if t < 0.45 {
            let t2 = t * t;
            8.0 * t2 * t2 * (t * PI * 9.0).sin()
        } else if t < 0.55 {
            0.5 + 0.75 * (t * PI * 4.0).sin()
        } else {
            let t2 = (t - 1.0) * (t - 1.0);
            1.0 - 8.0 * t2 * t2 * (t * PI * 9.0).sin()
        }
    }

    #[inline(always)]
    fn in_bounce(t: f32) -> f32 {
        (6.0 * (t - 1.0)).exp2() * (t * PI * 3.5).sin().abs()
    }

    #[inline(always)]
    fn out_bounce(t: f32) -> f32 {
        1.0 - (-6.0 * t).exp2() * (t * PI * 3.5).cos().abs()
    }

    #[inline(always)]
    fn in_out_bounce(t: f32) -> f32 {
        if t < 0.5 {
            8.0 * (8.0 * (t - 1.0)).exp2() * (t * PI * 7.0).sin().abs()
        } else {
            1.0 - 8.0 * (-8.0 * t).exp2() * (t * PI * 7.0).sin().abs()
        }
    }
}

// ============================================================================
// Free-function convenience wrappers (delegating to the trait)
// ============================================================================

/// `Easing::in_sine` for any float type. Provided as a free function for
/// callers that prefer it over `Easing::in_sine(t)`.
#[inline]
pub fn in_sine<T: Easing>(t: T) -> T {
    <T as Easing>::in_sine(t)
}

/// See [`in_sine`].
#[inline]
pub fn out_sine<T: Easing>(t: T) -> T {
    <T as Easing>::out_sine(t)
}

/// See [`in_sine`].
#[inline]
pub fn in_out_sine<T: Easing>(t: T) -> T {
    <T as Easing>::in_out_sine(t)
}

/// See [`in_sine`].
#[inline]
pub fn in_quad<T: Easing>(t: T) -> T {
    <T as Easing>::in_quad(t)
}

/// See [`in_sine`].
#[inline]
pub fn out_quad<T: Easing>(t: T) -> T {
    <T as Easing>::out_quad(t)
}

/// See [`in_sine`].
#[inline]
pub fn in_out_quad<T: Easing>(t: T) -> T {
    <T as Easing>::in_out_quad(t)
}

/// See [`in_sine`].
#[inline]
pub fn in_cubic<T: Easing>(t: T) -> T {
    <T as Easing>::in_cubic(t)
}

/// See [`in_sine`].
#[inline]
pub fn out_cubic<T: Easing>(t: T) -> T {
    <T as Easing>::out_cubic(t)
}

/// See [`in_sine`].
#[inline]
pub fn in_out_cubic<T: Easing>(t: T) -> T {
    <T as Easing>::in_out_cubic(t)
}

/// See [`in_sine`].
#[inline]
pub fn in_quart<T: Easing>(t: T) -> T {
    <T as Easing>::in_quart(t)
}

/// See [`in_sine`].
#[inline]
pub fn out_quart<T: Easing>(t: T) -> T {
    <T as Easing>::out_quart(t)
}

/// See [`in_sine`].
#[inline]
pub fn in_out_quart<T: Easing>(t: T) -> T {
    <T as Easing>::in_out_quart(t)
}

/// See [`in_sine`].
#[inline]
pub fn in_quint<T: Easing>(t: T) -> T {
    <T as Easing>::in_quint(t)
}

/// See [`in_sine`].
#[inline]
pub fn out_quint<T: Easing>(t: T) -> T {
    <T as Easing>::out_quint(t)
}

/// See [`in_sine`].
#[inline]
pub fn in_out_quint<T: Easing>(t: T) -> T {
    <T as Easing>::in_out_quint(t)
}

/// See [`in_sine`].
#[inline]
pub fn in_expo<T: Easing>(t: T) -> T {
    <T as Easing>::in_expo(t)
}

/// See [`in_sine`].
#[inline]
pub fn out_expo<T: Easing>(t: T) -> T {
    <T as Easing>::out_expo(t)
}

/// See [`in_sine`].
#[inline]
pub fn in_out_expo<T: Easing>(t: T) -> T {
    <T as Easing>::in_out_expo(t)
}

/// See [`in_sine`].
#[inline]
pub fn in_circ<T: Easing>(t: T) -> T {
    <T as Easing>::in_circ(t)
}

/// See [`in_sine`].
#[inline]
pub fn out_circ<T: Easing>(t: T) -> T {
    <T as Easing>::out_circ(t)
}

/// See [`in_sine`].
#[inline]
pub fn in_out_circ<T: Easing>(t: T) -> T {
    <T as Easing>::in_out_circ(t)
}

/// See [`in_sine`].
#[inline]
pub fn in_back<T: Easing>(t: T) -> T {
    <T as Easing>::in_back(t)
}

/// See [`in_sine`].
#[inline]
pub fn out_back<T: Easing>(t: T) -> T {
    <T as Easing>::out_back(t)
}

/// See [`in_sine`].
#[inline]
pub fn in_out_back<T: Easing>(t: T) -> T {
    <T as Easing>::in_out_back(t)
}

/// See [`in_sine`].
#[inline]
pub fn in_elastic<T: Easing>(t: T) -> T {
    <T as Easing>::in_elastic(t)
}

/// See [`in_sine`].
#[inline]
pub fn out_elastic<T: Easing>(t: T) -> T {
    <T as Easing>::out_elastic(t)
}

/// See [`in_sine`].
#[inline]
pub fn in_out_elastic<T: Easing>(t: T) -> T {
    <T as Easing>::in_out_elastic(t)
}

/// See [`in_sine`].
#[inline]
pub fn in_bounce<T: Easing>(t: T) -> T {
    <T as Easing>::in_bounce(t)
}

/// See [`in_sine`].
#[inline]
pub fn out_bounce<T: Easing>(t: T) -> T {
    <T as Easing>::out_bounce(t)
}

/// See [`in_sine`].
#[inline]
pub fn in_out_bounce<T: Easing>(t: T) -> T {
    <T as Easing>::in_out_bounce(t)
}

// ============================================================================
// FFI surface (C++ UI animation layer)
// ============================================================================
//
// Every curve is exported as `pcsx2_easing_<curve>(t: f32) -> f32`. These
// are thin wrappers around the `Easing` trait methods and inherit their
// `#[inline(always)]` behaviour, so callers in the C++ side get the same
// machine code as direct calls to the Rust implementation.

macro_rules! ffi_easing {
    ($(#[$meta:meta])* $name:ident, $method:ident) => {
        $(#[$meta])*
        #[no_mangle]
        pub extern "C" fn $name(t: f32) -> f32 {
            <f32 as Easing>::$method(t)
        }
    };
}

ffi_easing!(
    /// `sin(t * pi/2)` — FFI export of [`Easing::in_sine`].
    pcsx2_easing_in_sine, in_sine
);
ffi_easing!(
    /// `1 + sin((t - 1) * pi/2)` — FFI export of [`Easing::out_sine`].
    pcsx2_easing_out_sine, out_sine
);
ffi_easing!(
    /// Symmetric sine blend — FFI export of [`Easing::in_out_sine`].
    pcsx2_easing_in_out_sine, in_out_sine
);

ffi_easing!(
    /// `t * t` — FFI export of [`Easing::in_quad`].
    pcsx2_easing_in_quad, in_quad
);
ffi_easing!(
    /// `t * (2 - t)` — FFI export of [`Easing::out_quad`].
    pcsx2_easing_out_quad, out_quad
);
ffi_easing!(
    /// Symmetric quadratic blend — FFI export of [`Easing::in_out_quad`].
    pcsx2_easing_in_out_quad, in_out_quad
);

ffi_easing!(
    /// `t^3` — FFI export of [`Easing::in_cubic`].
    pcsx2_easing_in_cubic, in_cubic
);
ffi_easing!(
    /// `1 + (t - 1)^3` — FFI export of [`Easing::out_cubic`].
    pcsx2_easing_out_cubic, out_cubic
);
ffi_easing!(
    /// Symmetric cubic blend — FFI export of [`Easing::in_out_cubic`].
    pcsx2_easing_in_out_cubic, in_out_cubic
);

ffi_easing!(
    /// `t^4` — FFI export of [`Easing::in_quart`].
    pcsx2_easing_in_quart, in_quart
);
ffi_easing!(
    /// `1 - (t - 1)^4` — FFI export of [`Easing::out_quart`].
    pcsx2_easing_out_quart, out_quart
);
ffi_easing!(
    /// Symmetric quartic blend — FFI export of [`Easing::in_out_quart`].
    pcsx2_easing_in_out_quart, in_out_quart
);

ffi_easing!(
    /// `t^5` — FFI export of [`Easing::in_quint`].
    pcsx2_easing_in_quint, in_quint
);
ffi_easing!(
    /// `1 + (t - 1)^5` — FFI export of [`Easing::out_quint`].
    pcsx2_easing_out_quint, out_quint
);
ffi_easing!(
    /// Symmetric quintic blend — FFI export of [`Easing::in_out_quint`].
    pcsx2_easing_in_out_quint, in_out_quint
);

ffi_easing!(
    /// `(2^(8t) - 1) / 255` — FFI export of [`Easing::in_expo`].
    pcsx2_easing_in_expo, in_expo
);
ffi_easing!(
    /// `1 - 2^(-8t)` — FFI export of [`Easing::out_expo`].
    pcsx2_easing_out_expo, out_expo
);
ffi_easing!(
    /// Symmetric exponential blend — FFI export of [`Easing::in_out_expo`].
    pcsx2_easing_in_out_expo, in_out_expo
);

ffi_easing!(
    /// `1 - sqrt(1 - t)` — FFI export of [`Easing::in_circ`].
    pcsx2_easing_in_circ, in_circ
);
ffi_easing!(
    /// `sqrt(t)` — FFI export of [`Easing::out_circ`].
    pcsx2_easing_out_circ, out_circ
);
ffi_easing!(
    /// Symmetric circular blend — FFI export of [`Easing::in_out_circ`].
    pcsx2_easing_in_out_circ, in_out_circ
);

ffi_easing!(
    /// Overshooting quadratic, forward — FFI export of [`Easing::in_back`].
    pcsx2_easing_in_back, in_back
);
ffi_easing!(
    /// Overshooting quadratic, backward — FFI export of [`Easing::out_back`].
    pcsx2_easing_out_back, out_back
);
ffi_easing!(
    /// Symmetric back blend — FFI export of [`Easing::in_out_back`].
    pcsx2_easing_in_out_back, in_out_back
);

ffi_easing!(
    /// Spring-style oscillation, forward — FFI export of [`Easing::in_elastic`].
    pcsx2_easing_in_elastic, in_elastic
);
ffi_easing!(
    /// Spring-style oscillation, backward — FFI export of [`Easing::out_elastic`].
    pcsx2_easing_out_elastic, out_elastic
);
ffi_easing!(
    /// Symmetric elastic blend — FFI export of [`Easing::in_out_elastic`].
    pcsx2_easing_in_out_elastic, in_out_elastic
);

ffi_easing!(
    /// Decaying bounce, forward — FFI export of [`Easing::in_bounce`].
    pcsx2_easing_in_bounce, in_bounce
);
ffi_easing!(
    /// Decaying bounce, backward — FFI export of [`Easing::out_bounce`].
    pcsx2_easing_out_bounce, out_bounce
);
ffi_easing!(
    /// Symmetric bounce blend — FFI export of [`Easing::in_out_bounce`].
    pcsx2_easing_in_out_bounce, in_out_bounce
);