// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Easing function library ported from `common/Easing.h`.
//!
//! Each function takes a progress value `t` in `[0.0, 1.0]` and returns the
//! eased value. They are designed to be cheap to inline and have no side
//! effects.
//!
//! Provided easings:
//! - Sine family: [`InSine`], [`OutSine`], [`InOutSine`]
//! - Quad family: [`InQuad`], [`OutQuad`], [`InOutQuad`]
//! - Cubic family: [`InCubic`], [`OutCubic`], [`InOutCubic`]
//! - Quart family: [`InQuart`], [`OutQuart`], [`InOutQuart`]
//! - Quint family: [`InQuint`], [`OutQuint`], [`InOutQuint`]
//! - Expo family: [`InExpo`], [`OutExpo`], [`InOutExpo`]
//! - Circ family: [`InCirc`], [`OutCirc`], [`InOutCirc`]
//! - Back family: [`InBack`], [`OutBack`], [`InOutBack`]
//! - Elastic family: [`InElastic`], [`OutElastic`], [`InOutElastic`]
//! - Bounce family: [`InBounce`], [`OutBounce`], [`InOutBounce`]
//!
//! Translated from
//! <https://github.com/nicolausYes/easing-functions/blob/master/src/easing.cpp>.

use std::f32::consts::PI;

/// Pi truncated to a single-precision constant for parity with the original
/// C++ implementation, which declared its own `pi = 3.1415926545f`.
const PI_F: f32 = 3.1415926545_f32;

#[inline]
pub fn InSine(t: f32) -> f32 {
    (1.5707963_f32 * t).sin()
}

#[inline]
pub fn OutSine(t: f32) -> f32 {
    let t = t - 1.0;
    1.0 + (1.5707963_f32 * t).sin()
}

#[inline]
pub fn InOutSine(t: f32) -> f32 {
    0.5 * (1.0 + (PI * (t - 0.5)).sin())
}

#[inline]
pub fn InQuad(t: f32) -> f32 {
    t * t
}

#[inline]
pub fn OutQuad(t: f32) -> f32 {
    t * (2.0 - t)
}

#[inline]
pub fn InOutQuad(t: f32) -> f32 {
    if t < 0.5 {
        2.0 * t * t
    } else {
        t * (4.0 - 2.0 * t) - 1.0
    }
}

#[inline]
pub fn InCubic(t: f32) -> f32 {
    t * t * t
}

#[inline]
pub fn OutCubic(t: f32) -> f32 {
    let t = t - 1.0;
    1.0 + t * t * t
}

#[inline]
pub fn InOutCubic(t: f32) -> f32 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        // C++: `1 + (--t) * (2 * (--t)) * (2 * t)`. Under C++17 left-to-right
        // operand evaluation of the `*` chain, the two `--t` decrement t
        // twice before the final `2 * t` reads it.
        let first = t - 1.0;
        let second = t - 2.0;
        1.0 + first * (2.0 * second) * (2.0 * second)
    }
}

#[inline]
pub fn InQuart(t: f32) -> f32 {
    let t = t * t;
    t * t
}

#[inline]
pub fn OutQuart(t: f32) -> f32 {
    // C++: `t = (--t) * t;` -- the second operand is the already-decremented t.
    let new_t = t - 1.0;
    let t = new_t * new_t;
    1.0 - t * t
}

#[inline]
pub fn InOutQuart(t: f32) -> f32 {
    if t < 0.5 {
        let t = t * t;
        8.0 * t * t
    } else {
        // C++: `t = (--t) * t;` -- both operands are the decremented t.
        let new_t = t - 1.0;
        let t = new_t * new_t;
        1.0 - 8.0 * t * t
    }
}

#[inline]
pub fn InQuint(t: f32) -> f32 {
    let t2 = t * t;
    t * t2 * t2
}

#[inline]
pub fn OutQuint(t: f32) -> f32 {
    // C++: `T t2 = (--t) * t;` -- both operands are the decremented t.
    let new_t = t - 1.0;
    let t = new_t;
    let t2 = new_t * new_t;
    1.0 + t * t2 * t2
}

#[inline]
pub fn InOutQuint(t: f32) -> f32 {
    if t < 0.5 {
        let t2 = t * t;
        16.0 * t * t2 * t2
    } else {
        // C++: `t2 = (--t) * t;` -- both operands are the decremented t.
        let new_t = t - 1.0;
        let t = new_t;
        let t2 = new_t * new_t;
        1.0 + 16.0 * t * t2 * t2
    }
}

#[inline]
pub fn InExpo(t: f32) -> f32 {
    (2f32.powf(8.0 * t) - 1.0) / 255.0
}

#[inline]
pub fn OutExpo(t: f32) -> f32 {
    1.0 - 2f32.powf(-8.0 * t)
}

#[inline]
pub fn InOutExpo(t: f32) -> f32 {
    if t < 0.5 {
        (2f32.powf(16.0 * t) - 1.0) / 510.0
    } else {
        1.0 - 0.5 * 2f32.powf(-16.0 * (t - 0.5))
    }
}

#[inline]
pub fn InCirc(t: f32) -> f32 {
    1.0 - (1.0 - t).sqrt()
}

#[inline]
pub fn OutCirc(t: f32) -> f32 {
    t.sqrt()
}

#[inline]
pub fn InOutCirc(t: f32) -> f32 {
    if t < 0.5 {
        (1.0 - (1.0 - 2.0 * t).sqrt()) * 0.5
    } else {
        (1.0 + (2.0 * t - 1.0).sqrt()) * 0.5
    }
}

#[inline]
pub fn InBack(t: f32) -> f32 {
    t * t * (2.70158_f32 * t - 1.70158_f32)
}

#[inline]
pub fn OutBack(t: f32) -> f32 {
    let t = t - 1.0;
    1.0 + t * t * (2.70158_f32 * t + 1.70158_f32)
}

#[inline]
pub fn InOutBack(t: f32) -> f32 {
    if t < 0.5 {
        t * t * (7.0 * t - 2.5) * 2.0
    } else {
        // C++: `1 + (--t) * t * 2 * (7 * t + 2.5f)` -- only one `--t`,
        // so every subsequent `t` reads the decremented value.
        let t = t - 1.0;
        1.0 + t * t * 2.0 * (7.0 * t + 2.5)
    }
}

#[inline]
pub fn InElastic(t: f32) -> f32 {
    let t2 = t * t;
    t2 * t2 * (t * PI_F * 4.5).sin()
}

#[inline]
pub fn OutElastic(t: f32) -> f32 {
    let t2 = (t - 1.0) * (t - 1.0);
    1.0 - t2 * t2 * (t * PI_F * 4.5).cos()
}

#[inline]
pub fn InOutElastic(t: f32) -> f32 {
    if t < 0.45 {
        let t2 = t * t;
        8.0 * t2 * t2 * (t * PI_F * 9.0).sin()
    } else if t < 0.55 {
        0.5 + 0.75 * (t * PI_F * 4.0).sin()
    } else {
        let t2 = (t - 1.0) * (t - 1.0);
        1.0 - 8.0 * t2 * t2 * (t * PI_F * 9.0).sin()
    }
}

#[inline]
pub fn InBounce(t: f32) -> f32 {
    2f32.powf(6.0 * (t - 1.0)) * (t * PI_F * 3.5).sin().abs()
}

#[inline]
pub fn OutBounce(t: f32) -> f32 {
    1.0 - 2f32.powf(-6.0 * t) * (t * PI_F * 3.5).cos().abs()
}

#[inline]
pub fn InOutBounce(t: f32) -> f32 {
    if t < 0.5 {
        8.0 * 2f32.powf(8.0 * (t - 1.0)) * (t * PI_F * 7.0).sin().abs()
    } else {
        1.0 - 8.0 * 2f32.powf(-8.0 * t) * (t * PI_F * 7.0).sin().abs()
    }
}
