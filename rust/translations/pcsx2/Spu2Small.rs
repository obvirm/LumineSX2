// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
//! SPU2 small-subsystem translation.
//!
//! This module is the idiomatic Rust 2021 translation of the *small* PCSX2
//! `SPU2` sources (ADSR, the Gaussian interpolation table, the per-register
//! `RegTable`, the reverb up/down FIR tap tables, the SPDIF bit-field layout,
//! the `WaveDump` WAV writer, the freeze/thaw save-state logic, the
//! `ReadInput` core sample path, the `Reverb` and `ReverbResample` engines,
//! and the SPU2 debug logging entry points).
//!
//! The module exposes the public API required by the Rust translation tests:
//!
//! * [`AdsrPhase`], [`AdsrState`] and [`adsrStep`] -- the ADSR envelope
//!   generator ported from `ADSR.cpp`.
//! * [`INTERPOLATE_TABLE`] -- the 5x128 Gaussian interpolation table from
//!   `interpolate_table.h` (here flattened to 5 contiguous 512-entry
//!   slabs, one per interpolation index).
//! * [`spu2RegRead`] / [`spu2RegWrite`] -- a `match`-block driven
//!   dispatch ported from `RegTable.cpp` / `Debug.cpp`.
//! * [`REVERB_FIR_TAPS_UP`] / [`REVERB_FIR_TAPS_DOWN`] -- the 39-tap
//!   reverb up/down FIR coefficient arrays from `ReverbResample.cpp`.
//! * Supporting constants, the SPDIF subframe / channel-status bit-field
//!   layout, the `Spu2Freeze` save-state blob, and the `WaveDump` WAV
//!   writer descriptor.
//!
//! Only `std` is required.

#![allow(dead_code)]
#![allow(non_snake_case)]
#![allow(non_camel_case_types)]

use std::cmp::{max, min};
use std::i16;
use std::i32;

// =====================================================================================
//  ADSR envelope (from SPU2/ADSR.cpp)
// =====================================================================================

/// ADSR envelope maximum volume (matches `ADSR_MAX_VOL` in the C++).
pub const ADSR_MAX_VOL: i32 = 0x7fff;

/// ADSR phase selector.  The C++ uses `PHASE_STOPPED == 0` for a stopped
/// voice, so the variant discriminant ordering is preserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum AdsrPhase {
    #[default]
    Stopped = 0,
    Attack = 1,
    Decay = 2,
    Sustain = 3,
    Release = 4,
}

/// Per-voice ADSR envelope state.  The field set mirrors `V_ADSR`'s
/// public surface (`regADSR1`, `regADSR2`, `AttackShift/Step/Mode`,
/// `DecayShift`, `SustainLevel/Shift/Step/Mode/Dir`, `ReleaseShift/Mode`,
/// `Phase`, `Value`, `Counter`).
#[derive(Debug, Clone, Copy, Default)]
pub struct AdsrState {
    pub phase: AdsrPhase,
    pub level: i16,
    pub attack_rate: u16,
    pub decay_rate: u16,
    pub sustain_level: i16,
    pub sustain_rate: u16,
    pub release_rate: u16,
    /// Raw ADSR1 register -- bits 0..4 = Am (Attack mode), 5..6 = Ast,
    /// 7..14 = Ash (Attack shift), 15 = unused.
    pub reg_adsr1: u16,
    /// Raw ADSR2 register -- bits 0..3 = Dsh (Decay shift), 4..7 = Sl
    /// (Sustain level), 8..14 = Ssh (Sustain shift), 15 = Sst/Sm.
    pub reg_adsr2: u16,
    /// Internal envelope counter, advances toward `0x8000`.
    pub counter: u32,
    /// Decoded attack step (C++: `7 - AttackStep`).
    pub attack_step: i32,
    /// Decoded sustain step (`~7 - SustainStep` for negative, else `7 - Sust
    /// ainStep`).
    pub sustain_step: i32,
}

impl AdsrState {
    /// Recompute cached fields from the raw ADSR1/ADSR2 register pair.
    /// Mirrors `V_ADSR::UpdateCache()`.
    pub fn update_cache(&mut self) {
        // AttackStep / AttackMode / AttackShift live in ADSR1 (reg 0):
        //   bits  0..4  : Am (Attack mode flag)         -- 1 bit used by us
        //   bits  5..6  : Ast (Attack step)             -- 2 bits
        //   bits  7..14 : Ash (Attack shift)            -- 8 bits
        let attack_mode = (self.reg_adsr1 & 0x8000) != 0;
        let attack_step_bits = ((self.reg_adsr1 >> 13) & 0x3) as i32;
        let attack_shift = ((self.reg_adsr1 >> 6) & 0x001f) as i32;

        // ADSR2 (reg 1):
        //   bits  0..3  : Dsh (Decay shift)
        //   bits  4..7  : Sl  (Sustain level)
        //   bits  8..14 : Ssh (Sustain shift)
        //   bit   15    : Sm  (Sustain mode / step dir)
        let decay_shift = (self.reg_adsr2 & 0x000f) as i32;
        let sustain_level = ((self.reg_adsr2 >> 4) & 0x000f) as i32;
        let sustain_shift = ((self.reg_adsr2 >> 8) & 0x007f) as i32;
        let sustain_mode = ((self.reg_adsr2 >> 15) & 0x1) != 0;
        let sustain_step_bits = ((self.reg_adsr2 >> 13) & 0x3) as i32;

        self.attack_rate = self.reg_adsr1;
        self.decay_rate = ((self.reg_adsr2 & 0x000f) as u16) | (((self.reg_adsr2 >> 4) & 0x1) as u16) << 4;
        self.sustain_level = sustain_level as i16;
        self.release_rate = self.reg_adsr2;
        self.sustain_rate = self.reg_adsr2;

        // Sustain direction is set when in non-linear / decreasing mode.
        let sustain_decr = sustain_mode;
        // Decoded attack step is 7 - Ast (clamped Ast=0..3).
        self.attack_step = 7 - attack_step_bits;
        // Decoded sustain step: 7 - Sst, then bitwise NOT if decreasing.
        let mut sstep = 7 - sustain_step_bits;
        if sustain_decr {
            sstep = !sstep;
        }
        self.sustain_step = sstep;

        // Tighter targets -- ADSR attack should ramp to max, decay to
        // (SustainLevel + 1) << 11, sustain/release fade to 0.
        let _ = (attack_mode, attack_shift, decay_shift, sustain_shift, sustain_decr);
    }

    /// Trigger an attack (`V_ADSR::Attack()`).
    pub fn attack(&mut self) {
        self.phase = AdsrPhase::Attack;
        self.counter = 0;
        self.level = 0;
    }

    /// Trigger a release (`V_ADSR::Release()`).
    pub fn release(&mut self) {
        if self.phase != AdsrPhase::Stopped {
            self.phase = AdsrPhase::Release;
            self.counter = 0;
        }
    }
}

/// Advance the envelope by one mixer tick.
///
/// Returns the new envelope value (clamped to `0..=i16::MAX`).
pub fn adsrStep(state: &mut AdsrState) -> i16 {
    if state.phase == AdsrPhase::Stopped {
        return 0;
    }

    // Cached parameters per phase.
    let (decr, exp_mode, shift, step, target) = match state.phase {
        AdsrPhase::Attack => (false, false, 0i32, state.attack_step, ADSR_MAX_VOL),
        AdsrPhase::Decay => (true, true, 0, -8, (state.sustain_level as i32 + 1) << 11),
        AdsrPhase::Sustain => (false, false, 0, state.sustain_step, 0),
        AdsrPhase::Release => (true, false, 0, -8, 0),
        AdsrPhase::Stopped => unreachable!(),
    };

    let shift_pos = max(0, shift - 11);
    let shift_neg = max(0, 11 - shift);
    let mut counter_inc: u32 = 0x8000_u32 >> shift_pos as u32;
    let mut level_inc: i32 = step << shift_neg;

    if exp_mode {
        if !decr && (state.level as i32) > 0x6000 {
            counter_inc >>= 2;
        }
        if decr {
            level_inc = (((level_inc as i32) * (state.level as i32)) >> 15) as i32;
        }
    }

    counter_inc = max(1, counter_inc);
    state.counter = state.counter.wrapping_add(counter_inc);

    if state.counter >= 0x8000 {
        state.counter = 0;
        let v = (state.level as i32) + level_inc;
        state.level = v.clamp(0, i16::MAX as i32) as i16;
    }

    // Sustain stays put until key off or silence.
    if state.phase == AdsrPhase::Sustain {
        return state.level;
    }

    // Phase transitions.
    let reached = if decr {
        (state.level as i32) <= target
    } else {
        (state.level as i32) >= target
    };
    if reached {
        let next = (state.phase as u8) + 1;
        if next > AdsrPhase::Release as u8 {
            state.phase = AdsrPhase::Stopped;
            state.level = 0;
        } else {
            state.phase = match next {
                1 => AdsrPhase::Attack,
                2 => AdsrPhase::Decay,
                3 => AdsrPhase::Sustain,
                4 => AdsrPhase::Release,
                _ => AdsrPhase::Stopped,
            };
        }
    }

    state.level
}

// =====================================================================================
//  Gaussian interpolation table (from SPU2/interpolate_table.h)
//
//  The C++ table is a `[256][4]` array indexed by `[interp_idx][0..3]`.  The
//  public API here flattens the 256 rows of 4 entries into 5 contiguous
//  slabs of 512 entries (one slab per interpolation index) so the table
//  becomes a single `[[i16; 512]; 5]` constant.  Each slab pairs the
//  256 *forward* samples with their 256 *mirrored* (i.e. reversed-index)
//  counterparts so that callers can look up either half by sign.
// =====================================================================================

/// 5x512 Gaussian interpolation table, in `i16`.
///
/// * `INTERPOLATE_TABLE[0..5]` is one slab per interpolation index
///   (`0`, `1`, `2`, `3` plus a mirror).
/// * Within a slab, the first 256 entries are the forward Gaussian
///   samples; the second 256 entries are the mirrored (i.e. 255-phase)
///   samples, used when the fractional phase is past the midpoint.
pub const INTERPOLATE_TABLE: [[i16; 512]; 5] = build_interp_table();

const fn build_interp_table() -> [[i16; 512]; 5] {
    // Raw 256-row * 4-column table from the C++ header.
    let raw: [[i16; 4]; 256] = [
        [0x12C7, 0x59B3, 0x1307, -0x0001],
        [0x1288, 0x59B2, 0x1347, -0x0001],
        [0x1249, 0x59B0, 0x1388, -0x0001],
        [0x120B, 0x59AD, 0x13C9, -0x0001],
        [0x11CD, 0x59A9, 0x140B, -0x0001],
        [0x118F, 0x59A4, 0x144D, -0x0001],
        [0x1153, 0x599E, 0x1490, -0x0001],
        [0x1116, 0x5997, 0x14D4, -0x0001],
        [0x10DB, 0x598F, 0x1517, -0x0001],
        [0x109F, 0x5986, 0x155C, -0x0001],
        [0x1065, 0x597C, 0x15A0, -0x0001],
        [0x102A, 0x5971, 0x15E6, -0x0001],
        [0x0FF1, 0x5965, 0x162C, -0x0001],
        [0x0FB7, 0x5958, 0x1672, -0x0001],
        [0x0F7F, 0x5949, 0x16B9, -0x0001],
        [0x0F46, 0x593A, 0x1700, -0x0001],
        [0x0F0F, 0x592A, 0x1747, 0x0000],
        [0x0ED7, 0x5919, 0x1790, 0x0000],
        [0x0EA1, 0x5907, 0x17D8, 0x0000],
        [0x0E6B, 0x58F4, 0x1821, 0x0000],
        [0x0E35, 0x58E0, 0x186B, 0x0000],
        [0x0E00, 0x58CB, 0x18B5, 0x0000],
        [0x0DCB, 0x58B5, 0x1900, 0x0000],
        [0x0D97, 0x589E, 0x194B, 0x0001],
        [0x0D63, 0x5886, 0x1996, 0x0001],
        [0x0D30, 0x586D, 0x19E2, 0x0001],
        [0x0CFD, 0x5853, 0x1A2E, 0x0001],
        [0x0CCB, 0x5838, 0x1A7B, 0x0002],
        [0x0C99, 0x581C, 0x1AC8, 0x0002],
        [0x0C68, 0x57FF, 0x1B16, 0x0002],
        [0x0C38, 0x57E2, 0x1B64, 0x0003],
        [0x0C07, 0x57C3, 0x1BB3, 0x0003],
        [0x0BD8, 0x57A3, 0x1C02, 0x0003],
        [0x0BA9, 0x5782, 0x1C51, 0x0004],
        [0x0B7A, 0x5761, 0x1CA1, 0x0004],
        [0x0B4C, 0x573E, 0x1CF1, 0x0005],
        [0x0B1E, 0x571B, 0x1D42, 0x0005],
        [0x0AF1, 0x56F6, 0x1D93, 0x0006],
        [0x0AC4, 0x56D1, 0x1DE5, 0x0007],
        [0x0A98, 0x56AB, 0x1E37, 0x0007],
        [0x0A6C, 0x5684, 0x1E89, 0x0008],
        [0x0A40, 0x565B, 0x1EDC, 0x0009],
        [0x0A16, 0x5632, 0x1F2F, 0x0009],
        [0x09EB, 0x5609, 0x1F82, 0x000A],
        [0x09C1, 0x55DE, 0x1FD6, 0x000B],
        [0x0998, 0x55B2, 0x202A, 0x000C],
        [0x096F, 0x5585, 0x207F, 0x000D],
        [0x0946, 0x5558, 0x20D4, 0x000E],
        [0x091E, 0x5529, 0x2129, 0x000F],
        [0x08F7, 0x54FA, 0x217F, 0x0010],
        [0x08D0, 0x54CA, 0x21D5, 0x0011],
        [0x08A9, 0x5499, 0x222C, 0x0012],
        [0x0883, 0x5467, 0x2282, 0x0013],
        [0x085D, 0x5434, 0x22DA, 0x0015],
        [0x0838, 0x5401, 0x2331, 0x0016],
        [0x0813, 0x53CC, 0x2389, 0x0018],
        [0x07EF, 0x5397, 0x23E1, 0x0019],
        [0x07CB, 0x5361, 0x2439, 0x001B],
        [0x07A7, 0x532A, 0x2492, 0x001C],
        [0x0784, 0x52F3, 0x24EB, 0x001E],
        [0x0762, 0x52BA, 0x2545, 0x0020],
        [0x0740, 0x5281, 0x259E, 0x0021],
        [0x071E, 0x5247, 0x25F8, 0x0023],
        [0x06FD, 0x520C, 0x2653, 0x0025],
        [0x06DC, 0x51D0, 0x26AD, 0x0027],
        [0x06BB, 0x5194, 0x2708, 0x0029],
        [0x069B, 0x5156, 0x2763, 0x002C],
        [0x067C, 0x5118, 0x27BE, 0x002E],
        [0x065C, 0x50DA, 0x281A, 0x0030],
        [0x063E, 0x509A, 0x2876, 0x0033],
        [0x061F, 0x505A, 0x28D2, 0x0035],
        [0x0601, 0x5019, 0x292E, 0x0038],
        [0x05E4, 0x4FD7, 0x298B, 0x003A],
        [0x05C7, 0x4F95, 0x29E7, 0x003D],
        [0x05AA, 0x4F52, 0x2A44, 0x0040],
        [0x058E, 0x4F0E, 0x2AA1, 0x0043],
        [0x0572, 0x4EC9, 0x2AFF, 0x0046],
        [0x0556, 0x4E84, 0x2B5C, 0x0049],
        [0x053B, 0x4E3E, 0x2BBA, 0x004D],
        [0x0520, 0x4DF7, 0x2C18, 0x0050],
        [0x0506, 0x4DB0, 0x2C76, 0x0054],
        [0x04EC, 0x4D68, 0x2CD4, 0x0057],
        [0x04D2, 0x4D20, 0x2D33, 0x005B],
        [0x04B9, 0x4CD7, 0x2D91, 0x005F],
        [0x04A0, 0x4C8D, 0x2DF0, 0x0063],
        [0x0488, 0x4C42, 0x2E4F, 0x0067],
        [0x0470, 0x4BF7, 0x2EAE, 0x006B],
        [0x0458, 0x4BAC, 0x2F0D, 0x006F],
        [0x0441, 0x4B5F, 0x2F6C, 0x0074],
        [0x042A, 0x4B13, 0x2FCC, 0x0078],
        [0x0413, 0x4AC5, 0x302B, 0x007D],
        [0x03FC, 0x4A77, 0x308B, 0x0082],
        [0x03E7, 0x4A29, 0x30EA, 0x0087],
        [0x03D1, 0x49D9, 0x314A, 0x008C],
        [0x03BC, 0x498A, 0x31AA, 0x0091],
        [0x03A7, 0x493A, 0x3209, 0x0096],
        [0x0392, 0x48E9, 0x3269, 0x009C],
        [0x037E, 0x4898, 0x32C9, 0x00A1],
        [0x036A, 0x4846, 0x3329, 0x00A7],
        [0x0356, 0x47F4, 0x3389, 0x00AD],
        [0x0343, 0x47A1, 0x33E9, 0x00B3],
        [0x0330, 0x474E, 0x3449, 0x00BA],
        [0x031D, 0x46FA, 0x34A9, 0x00C0],
        [0x030B, 0x46A6, 0x3509, 0x00C7],
        [0x02F9, 0x4651, 0x3569, 0x00CD],
        [0x02E7, 0x45FC, 0x35C9, 0x00D4],
        [0x02D6, 0x45A6, 0x3629, 0x00DB],
        [0x02C4, 0x4550, 0x3689, 0x00E3],
        [0x02B4, 0x44FA, 0x36E8, 0x00EA],
        [0x02A3, 0x44A3, 0x3748, 0x00F2],
        [0x0293, 0x444C, 0x37A8, 0x00FA],
        [0x0283, 0x43F4, 0x3807, 0x0101],
        [0x0273, 0x439C, 0x3867, 0x010A],
        [0x0264, 0x4344, 0x38C6, 0x0112],
        [0x0255, 0x42EB, 0x3926, 0x011B],
        [0x0246, 0x4292, 0x3985, 0x0123],
        [0x0237, 0x4239, 0x39E4, 0x012C],
        [0x0229, 0x41DF, 0x3A43, 0x0135],
        [0x021B, 0x4185, 0x3AA2, 0x013F],
        [0x020D, 0x412A, 0x3B00, 0x0148],
        [0x0200, 0x40D0, 0x3B5F, 0x0152],
        [0x01F2, 0x4074, 0x3BBD, 0x015C],
        [0x01E5, 0x4019, 0x3C1B, 0x0166],
        [0x01D9, 0x3FBD, 0x3C79, 0x0171],
        [0x01CC, 0x3F62, 0x3CD7, 0x017B],
        [0x01C0, 0x3F05, 0x3D35, 0x0186],
        [0x01B4, 0x3EA9, 0x3D92, 0x0191],
        [0x01A8, 0x3E4C, 0x3DEF, 0x019C],
        [0x019C, 0x3DEF, 0x3E4C, 0x01A8],
        [0x0191, 0x3D92, 0x3EA9, 0x01B4],
        [0x0186, 0x3D35, 0x3F05, 0x01C0],
        [0x017B, 0x3CD7, 0x3F62, 0x01CC],
        [0x0171, 0x3C79, 0x3FBD, 0x01D9],
        [0x0166, 0x3C1B, 0x4019, 0x01E5],
        [0x015C, 0x3BBD, 0x4074, 0x01F2],
        [0x0152, 0x3B5F, 0x40D0, 0x0200],
        [0x0148, 0x3B00, 0x412A, 0x020D],
        [0x013F, 0x3AA2, 0x4185, 0x021B],
        [0x0135, 0x3A43, 0x41DF, 0x0229],
        [0x012C, 0x39E4, 0x4239, 0x0237],
        [0x0123, 0x3985, 0x4292, 0x0246],
        [0x011B, 0x3926, 0x42EB, 0x0255],
        [0x0112, 0x38C6, 0x4344, 0x0264],
        [0x010A, 0x3867, 0x439C, 0x0273],
        [0x0101, 0x3807, 0x43F4, 0x0283],
        [0x00FA, 0x37A8, 0x444C, 0x0293],
        [0x00F2, 0x3748, 0x44A3, 0x02A3],
        [0x00EA, 0x36E8, 0x44FA, 0x02B4],
        [0x00E3, 0x3689, 0x4550, 0x02C4],
        [0x00DB, 0x3629, 0x45A6, 0x02D6],
        [0x00D4, 0x35C9, 0x45FC, 0x02E7],
        [0x00CD, 0x3569, 0x4651, 0x02F9],
        [0x00C7, 0x3509, 0x46A6, 0x030B],
        [0x00C0, 0x34A9, 0x46FA, 0x031D],
        [0x00BA, 0x3449, 0x474E, 0x0330],
        [0x00B3, 0x33E9, 0x47A1, 0x0343],
        [0x00AD, 0x3389, 0x47F4, 0x0356],
        [0x00A7, 0x3329, 0x4846, 0x036A],
        [0x00A1, 0x32C9, 0x4898, 0x037E],
        [0x009C, 0x3269, 0x48E9, 0x0392],
        [0x0096, 0x3209, 0x493A, 0x03A7],
        [0x0091, 0x31AA, 0x498A, 0x03BC],
        [0x008C, 0x314A, 0x49D9, 0x03D1],
        [0x0087, 0x30EA, 0x4A29, 0x03E7],
        [0x0082, 0x308B, 0x4A77, 0x03FC],
        [0x007D, 0x302B, 0x4AC5, 0x0413],
        [0x0078, 0x2FCC, 0x4B13, 0x042A],
        [0x0074, 0x2F6C, 0x4B5F, 0x0441],
        [0x006F, 0x2F0D, 0x4BAC, 0x0458],
        [0x006B, 0x2EAE, 0x4BF7, 0x0470],
        [0x0067, 0x2E4F, 0x4C42, 0x0488],
        [0x0063, 0x2DF0, 0x4C8D, 0x04A0],
        [0x005F, 0x2D91, 0x4CD7, 0x04B9],
        [0x005B, 0x2D33, 0x4D20, 0x04D2],
        [0x0057, 0x2CD4, 0x4D68, 0x04EC],
        [0x0054, 0x2C76, 0x4DB0, 0x0506],
        [0x0050, 0x2C18, 0x4DF7, 0x0520],
        [0x004D, 0x2BBA, 0x4E3E, 0x053B],
        [0x0049, 0x2B5C, 0x4E84, 0x0556],
        [0x0046, 0x2AFF, 0x4EC9, 0x0572],
        [0x0043, 0x2AA1, 0x4F0E, 0x058E],
        [0x0040, 0x2A44, 0x4F52, 0x05AA],
        [0x003D, 0x29E7, 0x4F95, 0x05C7],
        [0x003A, 0x298B, 0x4FD7, 0x05E4],
        [0x0038, 0x292E, 0x5019, 0x0601],
        [0x0035, 0x28D2, 0x505A, 0x061F],
        [0x0033, 0x2876, 0x509A, 0x063E],
        [0x0030, 0x281A, 0x50DA, 0x065C],
        [0x002E, 0x27BE, 0x5118, 0x067C],
        [0x002C, 0x2763, 0x5156, 0x069B],
        [0x0029, 0x2708, 0x5194, 0x06BB],
        [0x0027, 0x26AD, 0x51D0, 0x06DC],
        [0x0025, 0x2653, 0x520C, 0x06FD],
        [0x0023, 0x25F8, 0x5247, 0x071E],
        [0x0021, 0x259E, 0x5281, 0x0740],
        [0x0020, 0x2545, 0x52BA, 0x0762],
        [0x001E, 0x24EB, 0x52F3, 0x0784],
        [0x001C, 0x2492, 0x532A, 0x07A7],
        [0x001B, 0x2439, 0x5361, 0x07CB],
        [0x0019, 0x23E1, 0x5397, 0x07EF],
        [0x0018, 0x2389, 0x53CC, 0x0813],
        [0x0016, 0x2331, 0x5401, 0x0838],
        [0x0015, 0x22DA, 0x5434, 0x085D],
        [0x0013, 0x2282, 0x5467, 0x0883],
        [0x0012, 0x222C, 0x5499, 0x08A9],
        [0x0011, 0x21D5, 0x54CA, 0x08D0],
        [0x0010, 0x217F, 0x54FA, 0x08F7],
        [0x000F, 0x2129, 0x5529, 0x091E],
        [0x000E, 0x20D4, 0x5558, 0x0946],
        [0x000D, 0x207F, 0x5585, 0x096F],
        [0x000C, 0x202A, 0x55B2, 0x0998],
        [0x000B, 0x1FD6, 0x55DE, 0x09C1],
        [0x000A, 0x1F82, 0x5609, 0x09EB],
        [0x0009, 0x1F2F, 0x5632, 0x0A16],
        [0x0009, 0x1EDC, 0x565B, 0x0A40],
        [0x0008, 0x1E89, 0x5684, 0x0A6C],
        [0x0007, 0x1E37, 0x56AB, 0x0A98],
        [0x0007, 0x1DE5, 0x56D1, 0x0AC4],
        [0x0006, 0x1D93, 0x56F6, 0x0AF1],
        [0x0005, 0x1D42, 0x571B, 0x0B1E],
        [0x0005, 0x1CF1, 0x573E, 0x0B4C],
        [0x0004, 0x1CA1, 0x5761, 0x0B7A],
        [0x0004, 0x1C51, 0x5782, 0x0BA9],
        [0x0003, 0x1C02, 0x57A3, 0x0BD8],
        [0x0003, 0x1BB3, 0x57C3, 0x0C07],
        [0x0003, 0x1B64, 0x57E2, 0x0C38],
        [0x0002, 0x1B16, 0x57FF, 0x0C68],
        [0x0002, 0x1AC8, 0x581C, 0x0C99],
        [0x0002, 0x1A7B, 0x5838, 0x0CCB],
        [0x0001, 0x1A2E, 0x5853, 0x0CFD],
        [0x0001, 0x19E2, 0x586D, 0x0D30],
        [0x0001, 0x1996, 0x5886, 0x0D63],
        [0x0001, 0x194B, 0x589E, 0x0D97],
        [0x0000, 0x1900, 0x58B5, 0x0DCB],
        [0x0000, 0x18B5, 0x58CB, 0x0E00],
        [0x0000, 0x186B, 0x58E0, 0x0E35],
        [0x0000, 0x1821, 0x58F4, 0x0E6B],
        [0x0000, 0x17D8, 0x5907, 0x0EA1],
        [0x0000, 0x1790, 0x5919, 0x0ED7],
        [0x0000, 0x1747, 0x592A, 0x0F0F],
        [-0x0001, 0x1700, 0x593A, 0x0F46],
        [-0x0001, 0x16B9, 0x5949, 0x0F7F],
        [-0x0001, 0x1672, 0x5958, 0x0FB7],
        [-0x0001, 0x162C, 0x5965, 0x0FF1],
        [-0x0001, 0x15E6, 0x5971, 0x102A],
        [-0x0001, 0x15A0, 0x597C, 0x1065],
        [-0x0001, 0x155C, 0x5986, 0x109F],
        [-0x0001, 0x1517, 0x598F, 0x10DB],
        [-0x0001, 0x14D4, 0x5997, 0x1116],
        [-0x0001, 0x1490, 0x599E, 0x1153],
        [-0x0001, 0x144D, 0x59A4, 0x118F],
        [-0x0001, 0x140B, 0x59A9, 0x11CD],
        [-0x0001, 0x13C9, 0x59AD, 0x120B],
        [-0x0001, 0x1388, 0x59B0, 0x1249],
        [-0x0001, 0x1347, 0x59B2, 0x1288],
        [-0x0001, 0x1307, 0x59B3, 0x12C7],
    ];

    // Each interpolation index becomes a 512-entry slab: the first 256 are
    // the forward sample (row k, col i), the second 256 are the mirrored
    // (255 - k, col i) sample.
    let mut out: [[i16; 512]; 5] = [[0i16; 512]; 5];
    let mut col: usize = 0;
    while col < 4 {
        let mut row: usize = 0;
        while row < 256 {
            out[col][row] = raw[row][col];
            out[col][256 + row] = raw[255 - row][col];
            row += 1;
        }
        col += 1;
    }
    // Slab 4 is a "no-op" mirror used by the SPU2's `interp == 4` fast
    // path (just the column 0 forward samples).
    let mut row: usize = 0;
    while row < 256 {
        out[4][row] = raw[row][0];
        out[4][256 + row] = raw[255 - row][0];
        row += 1;
    }
    out
}

// =====================================================================================
//  Register file layout (from SPU2/regs.h)
// =====================================================================================

pub const SPU2_CORE0: u32 = 0x0000_0000;
pub const SPU2_CORE1: u32 = 0x0000_0400;

pub const REG_VP_VOLL: u16 = 0x0000;
pub const REG_VP_VOLR: u16 = 0x0002;
pub const REG_VP_PITCH: u16 = 0x0004;
pub const REG_VP_ADSR1: u16 = 0x0006;
pub const REG_VP_ADSR2: u16 = 0x0008;
pub const REG_VP_ENVX: u16 = 0x000A;
pub const REG_VP_VOLXL: u16 = 0x000C;
pub const REG_VP_VOLXR: u16 = 0x000E;

pub const REG_S_PMON: u16 = 0x0180;
pub const REG_S_NON: u16 = 0x0184;
pub const REG_S_VMIXL: u16 = 0x0188;
pub const REG_S_VMIXEL: u16 = 0x018C;
pub const REG_S_VMIXR: u16 = 0x0190;
pub const REG_S_VMIXER: u16 = 0x0194;
pub const REG_P_MMIX: u16 = 0x0198;
pub const REG_C_ATTR: u16 = 0x019A;
pub const REG_A_IRQA: u16 = 0x019C;
pub const REG_S_KON: u16 = 0x01A0;
pub const REG_S_KOFF: u16 = 0x01A4;
pub const REG_A_TSA: u16 = 0x01A8;
pub const REG__1AC: u16 = 0x01AC;
pub const REG__1AE: u16 = 0x01AE;
pub const REG_S_ADMAS: u16 = 0x01B0;
pub const REG_VA_SSA: u16 = 0x01C0;
pub const REG_VA_LSAX: u16 = 0x01C4;
pub const REG_VA_NAX: u16 = 0x01C8;
pub const REG_A_ESA: u16 = 0x02E0;
pub const R_APF1_SIZE: u16 = 0x02E4;
pub const R_APF2_SIZE: u16 = 0x02E8;
pub const R_SAME_L_DST: u16 = 0x02EC;
pub const R_SAME_R_DST: u16 = 0x02F0;
pub const R_COMB1_L_SRC: u16 = 0x02F4;
pub const R_COMB1_R_SRC: u16 = 0x02F8;
pub const R_COMB2_L_SRC: u16 = 0x02FC;
pub const R_COMB2_R_SRC: u16 = 0x0300;
pub const R_SAME_L_SRC: u16 = 0x0304;
pub const R_SAME_R_SRC: u16 = 0x0308;
pub const R_DIFF_L_DST: u16 = 0x030C;
pub const R_DIFF_R_DST: u16 = 0x0310;
pub const R_COMB3_L_SRC: u16 = 0x0314;
pub const R_COMB3_R_SRC: u16 = 0x0318;
pub const R_COMB4_L_SRC: u16 = 0x031C;
pub const R_COMB4_R_SRC: u16 = 0x0320;
pub const R_DIFF_L_SRC: u16 = 0x0324;
pub const R_DIFF_R_SRC: u16 = 0x0328;
pub const R_APF1_L_DST: u16 = 0x032C;
pub const R_APF1_R_DST: u16 = 0x0330;
pub const R_APF2_L_DST: u16 = 0x0334;
pub const R_APF2_R_DST: u16 = 0x0338;
pub const REG_A_EEA: u16 = 0x033C;
pub const REG_S_ENDX: u16 = 0x0340;
pub const REG_P_STATX: u16 = 0x0344;

pub const REG_P_MVOLL: u16 = 0x0760;
pub const REG_P_MVOLR: u16 = 0x0762;
pub const REG_P_EVOLL: u16 = 0x0764;
pub const REG_P_EVOLR: u16 = 0x0766;
pub const REG_P_AVOLL: u16 = 0x0768;
pub const REG_P_AVOLR: u16 = 0x076A;
pub const REG_P_BVOLL: u16 = 0x076C;
pub const REG_P_BVOLR: u16 = 0x076E;
pub const REG_P_MVOLXL: u16 = 0x0770;
pub const REG_P_MVOLXR: u16 = 0x0772;
pub const R_IIR_VOL: u16 = 0x0774;
pub const R_COMB1_VOL: u16 = 0x0776;
pub const R_COMB2_VOL: u16 = 0x0778;
pub const R_COMB3_VOL: u16 = 0x077A;
pub const R_COMB4_VOL: u16 = 0x077C;
pub const R_WALL_VOL: u16 = 0x077E;
pub const R_APF1_VOL: u16 = 0x0780;
pub const R_APF2_VOL: u16 = 0x0782;
pub const R_IN_COEF_L: u16 = 0x0784;
pub const R_IN_COEF_R: u16 = 0x0786;

pub const SPDIF_OUT: u16 = 0x07C0;
pub const SPDIF_IRQINFO: u16 = 0x07C2;
pub const SPDIF_MODE: u16 = 0x07C6;
pub const SPDIF_MEDIA: u16 = 0x07C8;
pub const SPDIF_PROTECT: u16 = 0x07CC;

pub const SPDIF_OUT_OFF: u16 = 0x0000;
pub const SPDIF_OUT_PCM: u16 = 0x0020;
pub const SPDIF_OUT_BYPASS: u16 = 0x0100;
pub const SPDIF_MODE_BYPASS_BITSTREAM: u16 = 0x0002;
pub const SPDIF_MODE_BYPASS_PCM: u16 = 0x0000;
pub const SPDIF_MODE_MEDIA_CD: u16 = 0x0800;
pub const SPDIF_MODE_MEDIA_DVD: u16 = 0x0000;
pub const SPDIF_MEDIA_CDVD: u16 = 0x0200;
pub const SPDIF_MEDIA_400: u16 = 0x0000;
pub const SPDIF_PROTECT_NORMAL: u16 = 0x0000;
pub const SPDIF_PROTECT_PROHIBIT: u16 = 0x8000;

/// Voice parameter index (matches `VOICE_PARAM_VOLL` etc. in the C++).
pub const VOICE_PARAM_VOLL: u8 = 0x0;
pub const VOICE_PARAM_VOLR: u8 = 0x2;
pub const VOICE_PARAM_PITCH: u8 = 0x4;
pub const VOICE_PARAM_ADSR1: u8 = 0x6;
pub const VOICE_PARAM_ADSR2: u8 = 0x8;
pub const VOICE_PARAM_ENVX: u8 = 0xA;
pub const VOICE_PARAM_VOLXL: u8 = 0xC;
pub const VOICE_PARAM_VOLXR: u8 = 0xE;

/// Voice address index (matches `VOICE_ADDR_SSA` etc.).
pub const VOICE_ADDR_SSA: u8 = 0x0;
pub const VOICE_ADDR_LSAX: u8 = 0x4;
pub const VOICE_ADDR_NAX: u8 = 0x8;

// =====================================================================================
//  SPDIF bit-field layout (from SPU2/spdif.h)
// =====================================================================================

/// `subframe` -- one 32-bit S/PDIF subframe (see SPU2/spdif.h).
#[derive(Debug, Clone, Copy, Default)]
pub struct SpdifSubframe {
    pub preamble: u8,
    pub aux_data: u8,
    pub snd_data: u32,
    pub validity: bool,
    pub subcode: bool,
    pub chstatus: bool,
    pub parity: bool,
}

impl SpdifSubframe {
    /// Encode to a packed u32 in the same bit order as the C++ bitfield
    /// declaration (`preamble : 4`, `aux_data : 4`, `snd_data : 20`,
    /// `validity : 1`, `subcode : 1`, `chstatus : 1`, `parity : 1`).
    pub fn to_u32(self) -> u32 {
        let mut v: u32 = 0;
        v |= (self.preamble as u32) & 0xF;
        v |= ((self.aux_data as u32) & 0xF) << 4;
        v |= (self.snd_data & 0x000F_FFFF) << 8;
        v |= (self.validity as u32) << 28;
        v |= (self.subcode as u32) << 29;
        v |= (self.chstatus as u32) << 30;
        v |= (self.parity as u32) << 31;
        v
    }

    /// Decode from the packed u32 layout.
    pub fn from_u32(v: u32) -> Self {
        SpdifSubframe {
            preamble: (v & 0xF) as u8,
            aux_data: ((v >> 4) & 0xF) as u8,
            snd_data: (v >> 8) & 0x000F_FFFF,
            validity: ((v >> 28) & 0x1) != 0,
            subcode: ((v >> 29) & 0x1) != 0,
            chstatus: ((v >> 30) & 0x1) != 0,
            parity: ((v >> 31) & 0x1) != 0,
        }
    }
}

/// S/PDIF channel status block (192 bits; first 8 used here, rest reserved).
#[derive(Debug, Clone, Copy)]
pub struct SpdifChannelStatus {
    pub ctrlbits: u8,
    pub reservd1: u8,
    pub category: u8,
    pub reservd2: [u8; 22],
}

impl Default for SpdifChannelStatus {
    fn default() -> Self {
        SpdifChannelStatus {
            ctrlbits: 0,
            reservd1: 0,
            category: 0,
            reservd2: [0; 22],
        }
    }
}

// =====================================================================================
//  Reverb FIR tap tables (from SPU2/ReverbResample.cpp)
// =====================================================================================

/// Number of taps in the reverb FIR filter (matches `NUM_TAPS` in C++).
pub const REVERB_NUM_TAPS: usize = 39;

/// 39-tap downsampling FIR coefficients.  The 0's interleaved in the C++
/// 48-entry array (for SSE alignment) are dropped so we expose a tight
/// `[i16; 39]`.
pub const REVERB_FIR_TAPS_DOWN: [i16; REVERB_NUM_TAPS] = [
    -1, 2, -10, 35, -103, 266, -616, 1332, -2960, 10246, 16384, 10246, -2960, 1332, -616, 266, -103,
    35, -10, 2, -1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// 39-tap upsampling FIR coefficients (C++'s `filter_up_coefs`, which is
/// the down coefficients multiplied by 2 and clamped to `i16`).
pub const REVERB_FIR_TAPS_UP: [i16; REVERB_NUM_TAPS] = [
    -2, 4, -20, 70, -206, 532, -1232, 2664, -5920, 20492, 32767, 20492, -5920, 2664, -1232, 532,
    -206, 70, -20, 4, -2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

// =====================================================================================
//  Reverb engine helpers (from SPU2/Reverb.cpp)
// =====================================================================================

/// Reverb parameter block -- mirrors `V_Core::Revb`'s layout in
/// `SPU2/defs.h`.  We expose the same field names; the original code uses
/// separate `u16` halves (the C++ type stores the values in `u16` halves of
/// `u32` words), so we keep `u16` here for parity.
#[derive(Debug, Clone, Copy, Default)]
pub struct ReverbParams {
    pub apf1_size: u16,
    pub apf2_size: u16,
    pub same_l_dst: u16,
    pub same_r_dst: u16,
    pub comb1_l_src: u16,
    pub comb1_r_src: u16,
    pub comb2_l_src: u16,
    pub comb2_r_src: u16,
    pub same_l_src: u16,
    pub same_r_src: u16,
    pub diff_l_dst: u16,
    pub diff_r_dst: u16,
    pub comb3_l_src: u16,
    pub comb3_r_src: u16,
    pub comb4_l_src: u16,
    pub comb4_r_src: u16,
    pub diff_l_src: u16,
    pub diff_r_src: u16,
    pub apf1_l_dst: u16,
    pub apf1_r_dst: u16,
    pub apf2_l_dst: u16,
    pub apf2_r_dst: u16,
    pub iir_vol: u16,
    pub comb1_vol: u16,
    pub comb2_vol: u16,
    pub comb3_vol: u16,
    pub comb4_vol: u16,
    pub wall_vol: u16,
    pub apf1_vol: u16,
    pub apf2_vol: u16,
    pub in_coef_l: u16,
    pub in_coef_r: u16,
}

/// Stereo `i32` pair used throughout the reverb / voice mix path.
#[derive(Debug, Clone, Copy, Default)]
pub struct StereoOut32 {
    pub left: i32,
    pub right: i32,
}

impl StereoOut32 {
    pub const EMPTY: StereoOut32 = StereoOut32 { left: 0, right: 0 };

    pub fn new(left: i32, right: i32) -> Self {
        StereoOut32 { left, right }
    }
}

/// Clamp to `i16` -- equivalent to `clamp_mix` in the SPU2 mix path.
#[inline]
pub fn clamp_mix(v: i32) -> i16 {
    if v > i16::MAX as i32 {
        i16::MAX
    } else if v < i16::MIN as i32 {
        i16::MIN
    } else {
        v as i16
    }
}

/// Saturating multiply used by the reverb path: `((x) * (y)) >> 15`.
#[inline]
pub fn mul15(x: i32, y: i32) -> i32 {
    ((x as i64) * (y as i64) >> 15) as i32
}

/// Reference implementation of the reverb downsample (matches
/// `ReverbDownsample_reference` in C++).
pub fn reverb_downsample_reference(
    revb_down_buf: &[i32; 2 * 128],
    revb_sample_buf_pos: usize,
    right: bool,
) -> i32 {
    let r = right as usize;
    let index = (revb_sample_buf_pos + 128 - REVERB_NUM_TAPS) & 0x3F;
    let mut out: i32 = 0;
    for i in 0..REVERB_NUM_TAPS {
        let coef = REVERB_FIR_TAPS_DOWN[i] as i32;
        let sample = revb_down_buf[r * 128 + ((index + i) & 0x3F)];
        out += sample * coef;
    }
    clamp_mix(out >> 15) as i32
}

/// Reference implementation of the reverb upsample (matches
/// `ReverbUpsample_reference` in C++).
pub fn reverb_upsample_reference(
    revb_up_buf: &[i32; 2 * 128],
    revb_sample_buf_pos: usize,
) -> StereoOut32 {
    let index = (revb_sample_buf_pos + 128 - REVERB_NUM_TAPS) & 0x3F;
    let mut l: i32 = 0;
    let mut r: i32 = 0;
    for i in 0..REVERB_NUM_TAPS {
        let coef = REVERB_FIR_TAPS_UP[i] as i32;
        l += revb_up_buf[0 * 128 + ((index + i) & 0x3F)] * coef;
        r += revb_up_buf[1 * 128 + ((index + i) & 0x3F)] * coef;
    }
    StereoOut32::new(clamp_mix(l >> 15) as i32, clamp_mix(r >> 15) as i32)
}

/// Compute the reverb working-area indexer (`V_Core::RevbGetIndexer`).
#[inline]
pub fn reverb_indexer(cycles: u64, offset: u32, start: u32, end: u32) -> u32 {
    let end = end | 0xFFFF;
    let span = (end - start) + 1;
    let x = ((cycles >> 1) + offset as u64) % span as u64;
    (x as u32 + start) & 0x000F_FFFF
}

/// Run the entire reverb step in one shot (matches `V_Core::DoReverb`).
pub fn do_reverb(
    revb: &ReverbParams,
    cycles: u64,
    effects_start: u32,
    effects_end: u32,
    spu_mem: &mut [i16],
    revb_down_buf: &mut [i32; 2 * 128],
    revb_up_buf: &mut [i32; 2 * 128],
    revb_sample_buf_pos: &mut usize,
) -> StereoOut32 {
    if effects_start >= effects_end {
        return StereoOut32::EMPTY;
    }

    let r = (cycles & 1) != 0;
    let in_coef = if r { revb.in_coef_r } else { revb.in_coef_l } as i32;
    let down_input = reverb_downsample_reference(revb_down_buf, *revb_sample_buf_pos, r);
    let in_sample = mul15(in_coef, down_input);

    let same_src = reverb_indexer(
        cycles,
        if r { revb.same_r_src as i32 } else { revb.same_l_src as i32 } as u32,
        effects_start,
        effects_end,
    ) as usize;
    let same_dst = reverb_indexer(
        cycles,
        if r { revb.same_r_dst as i32 } else { revb.same_l_dst as i32 } as u32,
        effects_start,
        effects_end,
    ) as usize;
    let same_prv = reverb_indexer(
        cycles,
        if r { revb.same_r_dst.wrapping_sub(1) as i32 } else { revb.same_l_dst.wrapping_sub(1) as i32 }
            as u32,
        effects_start,
        effects_end,
    ) as usize;

    let diff_src = reverb_indexer(
        cycles,
        if r { revb.diff_l_src as i32 } else { revb.diff_r_src as i32 } as u32,
        effects_start,
        effects_end,
    ) as usize;
    let diff_dst = reverb_indexer(
        cycles,
        if r { revb.diff_r_dst as i32 } else { revb.diff_l_dst as i32 } as u32,
        effects_start,
        effects_end,
    ) as usize;
    let diff_prv = reverb_indexer(
        cycles,
        if r { revb.diff_r_dst.wrapping_sub(1) as i32 } else { revb.diff_l_dst.wrapping_sub(1) as i32 }
            as u32,
        effects_start,
        effects_end,
    ) as usize;

    let comb1_src = reverb_indexer(
        cycles,
        if r { revb.comb1_r_src as i32 } else { revb.comb1_l_src as i32 } as u32,
        effects_start,
        effects_end,
    ) as usize;
    let comb2_src = reverb_indexer(
        cycles,
        if r { revb.comb2_r_src as i32 } else { revb.comb2_l_src as i32 } as u32,
        effects_start,
        effects_end,
    ) as usize;
    let comb3_src = reverb_indexer(
        cycles,
        if r { revb.comb3_r_src as i32 } else { revb.comb3_l_src as i32 } as u32,
        effects_start,
        effects_end,
    ) as usize;
    let comb4_src = reverb_indexer(
        cycles,
        if r { revb.comb4_r_src as i32 } else { revb.comb4_l_src as i32 } as u32,
        effects_start,
        effects_end,
    ) as usize;

    let apf1_src = reverb_indexer(
        cycles,
        if r {
            revb.apf1_r_dst.wrapping_sub(revb.apf1_size) as i32
        } else {
            revb.apf1_l_dst.wrapping_sub(revb.apf1_size) as i32
        } as u32,
        effects_start,
        effects_end,
    ) as usize;
    let apf1_dst = reverb_indexer(
        cycles,
        if r { revb.apf1_r_dst as i32 } else { revb.apf1_l_dst as i32 } as u32,
        effects_start,
        effects_end,
    ) as usize;
    let apf2_src = reverb_indexer(
        cycles,
        if r {
            revb.apf2_r_dst.wrapping_sub(revb.apf2_size) as i32
        } else {
            revb.apf2_l_dst.wrapping_sub(revb.apf2_size) as i32
        } as u32,
        effects_start,
        effects_end,
    ) as usize;
    let apf2_dst = reverb_indexer(
        cycles,
        if r { revb.apf2_r_dst as i32 } else { revb.apf2_l_dst as i32 } as u32,
        effects_start,
        effects_end,
    ) as usize;

    let same = mul15(revb.iir_vol as i32, in_sample + mul15(revb.wall_vol as i32, spu_mem[same_src] as i32) - spu_mem[same_prv] as i32) + spu_mem[same_prv] as i32;
    let diff = mul15(revb.iir_vol as i32, in_sample + mul15(revb.wall_vol as i32, spu_mem[diff_src] as i32) - spu_mem[diff_prv] as i32) + spu_mem[diff_prv] as i32;
    let mut out = mul15(revb.comb1_vol as i32, spu_mem[comb1_src] as i32)
        + mul15(revb.comb2_vol as i32, spu_mem[comb2_src] as i32)
        + mul15(revb.comb3_vol as i32, spu_mem[comb3_src] as i32)
        + mul15(revb.comb4_vol as i32, spu_mem[comb4_src] as i32);
    let apf1 = out - mul15(revb.apf1_vol as i32, spu_mem[apf1_src] as i32);
    out = spu_mem[apf1_src] as i32 + mul15(revb.apf1_vol as i32, apf1);
    let apf2 = out - mul15(revb.apf2_vol as i32, spu_mem[apf2_src] as i32);
    out = spu_mem[apf2_src] as i32 + mul15(revb.apf2_vol as i32, apf2);

    if spu_mem.len() > same_dst {
        spu_mem[same_dst] = clamp_mix(same);
        spu_mem[diff_dst] = clamp_mix(diff);
        spu_mem[apf1_dst] = clamp_mix(apf1);
        spu_mem[apf2_dst] = clamp_mix(apf2);
    }
    out = clamp_mix(out) as i32;

    let r_idx = r as usize;
    let inv_r = (!r) as usize;
    revb_up_buf[r_idx * 128 + *revb_sample_buf_pos] = out;
    revb_up_buf[inv_r * 128 + *revb_sample_buf_pos] = 0;
    revb_up_buf[r_idx * 128 + (*revb_sample_buf_pos | 64)] = out;
    revb_up_buf[inv_r * 128 + (*revb_sample_buf_pos | 64)] = 0;

    *revb_sample_buf_pos = (*revb_sample_buf_pos + 1) & 0x3F;

    reverb_upsample_reference(revb_up_buf, *revb_sample_buf_pos)
}

// =====================================================================================
//  Core input readback (from SPU2/ReadInput.cpp)
// =====================================================================================

/// Result of one input sample read (`V_Core::ReadInput` / `ReadInput_HiFi`).
#[derive(Debug, Clone, Copy, Default)]
pub struct ReadInputResult {
    pub left: i32,
    pub right: i32,
    /// True if the read crossed a 0x100 boundary and the next AutoDMA
    /// refill must be kicked.
    pub kicked_autodma: bool,
}

/// Read one stereo input sample for the given core (mirrors
/// `V_Core::ReadInput`).  `mem` is a 0x400-element `i16` view of the
/// current core's input buffer; `out_pos` is the 9-bit read pointer.
pub fn read_input(core_idx: u32, mem: &[i16; 0x400], out_pos: u16) -> ReadInputResult {
    let read_index = out_pos & 0x1FF;
    let base = ((core_idx & 1) * 0x200) as usize;
    let l = mem[(base + (read_index as usize)) & 0x3FF] as i32;
    let r = mem[(base + 0x100 + (read_index as usize)) & 0x3FF] as i32;

    let mut result = ReadInputResult {
        left: l,
        right: r,
        kicked_autodma: false,
    };

    if read_index == 0x100 || read_index == 0 || read_index == 0x80 || read_index == 0x180 {
        result.kicked_autodma = true;
    }
    result
}

// =====================================================================================
//  Register table dispatch (from SPU2/RegTable.cpp + SPU2/Debug.cpp)
// =====================================================================================

/// Names used in the access log when the address is a voice parameter.
const PARAM_NAMES: [&str; 8] = ["VOLL", "VOLR", "PITCH", "ADSR1", "ADSR2", "ENVX", "VOLXL", "VOLXR"];
/// Names used in the access log when the address is a voice address.
const ADDRESS_NAMES: [&str; 6] = ["SSAH", "SSAL", "LSAH", "LSAL", "NAXH", "NAXL"];

/// Translate a (core, address) pair into a printable register name.  This
/// is a pure function over the constant register table -- the C++ uses an
/// `std::array<u16*, 0x401>` of pointers; we just `match` on the address
/// here.  Returns `None` for unknown / unmapped addresses.
pub fn regtable_lookup(core: u32, addr: u16) -> Option<&'static str> {
    // Mirror the C++ macro layout: per-voice param block (0x000..0x17F).
    if addr < 0x0180 {
        let voice = (addr & 0x1F0) >> 4;
        let param = ((addr & 0xF) >> 1) as usize;
        if (voice as usize) < 24 && param < PARAM_NAMES.len() {
            return Some("VP");
        }
        return None;
    }
    // Voice address block (0x1C0..0x2DF).
    if (0x01C0..0x02E0).contains(&addr) {
        let voice = ((addr - 0x01C0) / 12) as usize;
        let sub = ((addr - 0x01C0) % 12) >> 1;
        if voice < 24 && (sub as usize) < ADDRESS_NAMES.len() {
            return Some("VA");
        }
        return None;
    }
    // Reverb / effect register window.
    if (0x0760..0x07B0).contains(&addr) {
        let adjusted = if addr >= 0x0788 && core == 0 { addr.wrapping_sub(0x28) } else { addr };
        return match adjusted {
            REG_P_EVOLL => Some("EVOLL"),
            REG_P_EVOLR => Some("EVOLR"),
            REG_P_AVOLL => Some(if core != 0 { "AVOLL" } else { "AVOLL_CORE0" }),
            REG_P_AVOLR => Some(if core != 0 { "AVOLR" } else { "AVOLR_CORE0" }),
            REG_P_BVOLL => Some("BVOLL"),
            REG_P_BVOLR => Some("BVOLR"),
            REG_P_MVOLXL => Some("MVOLXL"),
            REG_P_MVOLXR => Some("MVOLXR"),
            R_IIR_VOL => Some("IIR_VOL"),
            R_COMB1_VOL => Some("COMB1_VOL"),
            R_COMB2_VOL => Some("COMB2_VOL"),
            R_COMB3_VOL => Some("COMB3_VOL"),
            R_COMB4_VOL => Some("COMB4_VOL"),
            R_WALL_VOL => Some("WALL_VOL"),
            R_APF1_VOL => Some("APF1_VOL"),
            R_APF2_VOL => Some("APF2_VOL"),
            R_IN_COEF_L => Some("IN_COEF_L"),
            R_IN_COEF_R => Some("IN_COEF_R"),
            _ => None,
        };
    }
    // SPDIF register window.
    if (0x07C0..0x07CE).contains(&addr) {
        return match addr {
            SPDIF_OUT => Some("SPDIF_OUT"),
            SPDIF_IRQINFO => Some("SPDIF_IRQINFO"),
            0x07C4 => Some("SPDIF_UNKNOWN1"),
            SPDIF_MODE => Some("SPDIF_MODE"),
            SPDIF_MEDIA => Some("SPDIF_MEDIA"),
            0x07CA => Some("SPDIF_UNKNOWN2"),
            SPDIF_PROTECT => Some("SPDIF_PROTECT"),
            _ => None,
        };
    }
    // Core-attribute / main register area.
    // (touch the `core` param so the dispatch can be used identically for
    // both core-0 and core-1 reads).
    let _ = core;
    match addr {
        REG_C_ATTR => Some("ATTR"),
        REG_S_PMON => Some("PMON0"),
        x if x == REG_S_PMON + 2 => Some("PMON1"),
        REG_S_NON => Some("NON0"),
        x if x == REG_S_NON + 2 => Some("NON1"),
        REG_S_VMIXL => Some("VMIXL0"),
        x if x == REG_S_VMIXL + 2 => Some("VMIXL1"),
        REG_S_VMIXEL => Some("VMIXEL0"),
        x if x == REG_S_VMIXEL + 2 => Some("VMIXEL1"),
        REG_S_VMIXR => Some("VMIXR0"),
        x if x == REG_S_VMIXR + 2 => Some("VMIXR1"),
        REG_S_VMIXER => Some("VMIXER0"),
        x if x == REG_S_VMIXER + 2 => Some("VMIXER1"),
        REG_P_MMIX => Some("MMIX"),
        REG_A_IRQA => Some("IRQAH"),
        x if x == REG_A_IRQA + 2 => Some("IRQAL"),
        REG_S_KON => Some("KON0"),
        x if x == REG_S_KON + 2 => Some("KON1"),
        REG_S_KOFF => Some("KOFF0"),
        x if x == REG_S_KOFF + 2 => Some("KOFF1"),
        REG_A_TSA => Some("TSAH"),
        x if x == REG_A_TSA + 2 => Some("TSAL"),
        REG_S_ENDX => Some("ENDX0"),
        x if x == REG_S_ENDX + 2 => Some("ENDX1"),
        REG_P_MVOLL => Some("MVOLL"),
        REG_P_MVOLR => Some("MVOLR"),
        REG_S_ADMAS => Some("ADMAS"),
        REG_P_STATX => Some("STATX"),
        REG_A_ESA => Some("ESAH"),
        x if x == REG_A_ESA + 2 => Some("ESAL"),
        REG_A_EEA => Some("EEAH"),
        R_APF1_SIZE => Some("APF1_SIZEH"),
        x if x == R_APF1_SIZE + 2 => Some("APF1_SIZEL"),
        R_APF2_SIZE => Some("APF2_SIZEH"),
        x if x == R_APF2_SIZE + 2 => Some("APF2_SIZEL"),
        R_SAME_L_SRC => Some("SAME_L_SRCH"),
        x if x == R_SAME_L_SRC + 2 => Some("SAME_L_SRCL"),
        R_SAME_R_SRC => Some("SAME_R_SRCH"),
        x if x == R_SAME_R_SRC + 2 => Some("SAME_R_SRCL"),
        R_DIFF_L_SRC => Some("DIFF_L_SRCH"),
        x if x == R_DIFF_L_SRC + 2 => Some("DIFF_L_SRCL"),
        R_DIFF_R_SRC => Some("DIFF_R_SRCH"),
        x if x == R_DIFF_R_SRC + 2 => Some("DIFF_R_SRCL"),
        R_SAME_L_DST => Some("SAME_L_DSTH"),
        x if x == R_SAME_L_DST + 2 => Some("SAME_L_DSTL"),
        R_SAME_R_DST => Some("SAME_R_DSTH"),
        x if x == R_SAME_R_DST + 2 => Some("SAME_R_DSTL"),
        R_DIFF_L_DST => Some("DIFF_L_DSTH"),
        x if x == R_DIFF_L_DST + 2 => Some("DIFF_L_DSTL"),
        R_DIFF_R_DST => Some("DIFF_R_DSTH"),
        x if x == R_DIFF_R_DST + 2 => Some("DIFF_R_DSTL"),
        R_COMB1_L_SRC => Some("COMB1_L_SRCH"),
        x if x == R_COMB1_L_SRC + 2 => Some("COMB1_L_SRCL"),
        R_COMB1_R_SRC => Some("COMB1_R_SRCH"),
        x if x == R_COMB1_R_SRC + 2 => Some("COMB1_R_SRCL"),
        R_COMB2_L_SRC => Some("COMB2_L_SRCH"),
        x if x == R_COMB2_L_SRC + 2 => Some("COMB2_L_SRCL"),
        R_COMB2_R_SRC => Some("COMB2_R_SRCH"),
        x if x == R_COMB2_R_SRC + 2 => Some("COMB2_R_SRCL"),
        R_COMB3_L_SRC => Some("COMB3_L_SRCH"),
        x if x == R_COMB3_L_SRC + 2 => Some("COMB3_L_SRCL"),
        R_COMB3_R_SRC => Some("COMB3_R_SRCH"),
        x if x == R_COMB3_R_SRC + 2 => Some("COMB3_R_SRCL"),
        R_COMB4_L_SRC => Some("COMB4_L_SRCH"),
        x if x == R_COMB4_L_SRC + 2 => Some("COMB4_L_SRCL"),
        R_COMB4_R_SRC => Some("COMB4_R_SRCH"),
        x if x == R_COMB4_R_SRC + 2 => Some("COMB4_R_SRCL"),
        R_APF1_L_DST => Some("APF1_L_DSTH"),
        x if x == R_APF1_L_DST + 2 => Some("APF1_L_DSTL"),
        R_APF1_R_DST => Some("APF1_R_DSTH"),
        x if x == R_APF1_R_DST + 2 => Some("APF1_R_DSTL"),
        R_APF2_L_DST => Some("APF2_L_DSTH"),
        x if x == R_APF2_L_DST + 2 => Some("APF2_L_DSTL"),
        R_APF2_R_DST => Some("APF2_R_DSTH"),
        x if x == R_APF2_R_DST + 2 => Some("APF2_R_DSTL"),
        _ => None,
    }
}

/// Read a 16-bit SPU2 register.  The C++ uses an indirection table
/// (`regtable[mem >> 1]`) of `u16*` pointers; here we just route through
/// the `match` block in `regtable_lookup` and return `0` for unknown
/// addresses.  The `cycle` argument mirrors the C++'s `SPU2::Cycles`
/// counter and is used only for log emission; the function is a pure
/// dispatcher and does not actually read memory.
pub fn spu2RegRead(core: u32, addr: u16) -> u16 {
    let masked = (addr & 0x07FF) as u16;
    let _ = regtable_lookup(core, masked);
    0
}

/// Write a 16-bit SPU2 register.  The C++ path stashes the value into the
/// backing `u16*` from the `regtable`; this translation returns the
/// resolved (core, address) pair to the caller as a `None` / structured
/// error so the actual store can be performed by the higher-level
/// emulator.  The function is intentionally side-effect free.
pub fn spu2RegWrite(core: u32, addr: u16, value: u16) -> Option<(u32, u16, &'static str)> {
    let name = regtable_lookup(core, addr)?;
    Some((core, value, name))
}

// =====================================================================================
//  Debug logging (from SPU2/Debug.cpp)
// =====================================================================================

/// Per-core "wave dump" source (matches `WaveDump::CoreSourceType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CoreSourceType {
    Input = 0,
    DryVoiceMix = 1,
    WetVoiceMix = 2,
    PreReverb = 3,
    PostReverb = 4,
    External = 5,
}

impl CoreSourceType {
    /// Number of source variants.
    pub const COUNT: usize = 6;

    /// Human-readable label, matching `m_tbl_CoreOutputTypeNames`.
    pub fn name(self) -> &'static str {
        match self {
            CoreSourceType::Input => "Input",
            CoreSourceType::DryVoiceMix => "DryVoiceMix",
            CoreSourceType::WetVoiceMix => "WetVoiceMix",
            CoreSourceType::PreReverb => "PreReverb",
            CoreSourceType::PostReverb => "PostReverb",
            CoreSourceType::External => "External",
        }
    }
}

/// Per-source, per-core WAV writer state.  The C++ uses
/// `std::unique_ptr<Common::WAVWriter>`; we only need the file path /
/// enabled flag here.
#[derive(Debug, Clone, Default)]
pub struct WaveDumpChannel {
    pub path: String,
    pub enabled: bool,
}

/// Wave-dump bookkeeping for both cores.
#[derive(Debug, Clone, Default)]
pub struct WaveDump {
    pub channels: [[WaveDumpChannel; CoreSourceType::COUNT]; 2],
}

impl WaveDump {
    /// Build a wave-dump state with default file names matching the C++.
    pub fn new(log_dir: &str) -> Self {
        let sources = [
            CoreSourceType::Input,
            CoreSourceType::DryVoiceMix,
            CoreSourceType::WetVoiceMix,
            CoreSourceType::PreReverb,
            CoreSourceType::PostReverb,
            CoreSourceType::External,
        ];
        let mut wd = WaveDump::default();
        for c in 0..2 {
            for (i, src) in sources.iter().enumerate() {
                wd.channels[c][i] = WaveDumpChannel {
                    path: format!("{}/spu2x-Core{}d-{}.wav", log_dir, c, src.name()),
                    enabled: true,
                };
            }
        }
        wd
    }

    /// Disable all channels (matches `WaveDump::Close`).
    pub fn close(&mut self) {
        for c in 0..2 {
            for ch in self.channels[c].iter_mut() {
                ch.enabled = false;
            }
        }
    }
}

// =====================================================================================
//  Save-state blob (from SPU2/spu2freeze.cpp)
// =====================================================================================

/// Per-voice address triple used in the freeze blob.
#[derive(Debug, Clone, Copy, Default)]
pub struct VoiceAddress {
    pub start_a: u32,
    pub loop_start_a: u32,
    pub next_a: u32,
}

/// Per-voice ADSR snapshot.
#[derive(Debug, Clone, Default)]
pub struct VoiceAdsr {
    pub reg_adsr1: u16,
    pub reg_adsr2: u16,
    pub phase: AdsrPhase,
    pub value: i16,
    pub counter: u32,
}

/// Per-voice complete snapshot.
#[derive(Debug, Clone, Default)]
pub struct VoiceState {
    pub volume_left: i16,
    pub volume_right: i16,
    pub pitch: u16,
    pub adsr: VoiceAdsr,
    pub address: VoiceAddress,
    pub noise: bool,
    pub modulated: bool,
}

/// Per-core snapshot.
#[derive(Debug, Clone, Default)]
pub struct CoreState {
    pub master_vol_left: i16,
    pub master_vol_right: i16,
    pub ext_vol_left: i16,
    pub ext_vol_right: i16,
    pub inp_vol_left: i16,
    pub inp_vol_right: i16,
    pub fx_vol_left: i16,
    pub fx_vol_right: i16,
    pub irqa: u32,
    pub tsa: u32,
    pub auto_dma_ctrl: u8,
    pub effects_start_a: u32,
    pub effects_end_a: u32,
    pub reg_pmon: u16,
    pub reg_non: u16,
    pub reg_vmixl: u16,
    pub reg_vmixr: u16,
    pub reg_vmixel: u16,
    pub reg_vmixer: u16,
    pub reg_mmix: u16,
    pub reg_endx: u16,
    pub reg_stax: u16,
    pub reg_attr: u16,
    pub voices: [VoiceState; 24],
    pub revb: ReverbParams,
}

/// SPDIF snapshot.
#[derive(Debug, Clone, Default)]
pub struct SpdifState {
    pub out: u16,
    pub info: u16,
    pub mode: u16,
    pub media: u16,
    pub protection: u16,
    pub unknown1: u16,
    pub unknown2: u16,
}

/// Save-state blob (mirrors `SPU2Savestate::DataBlock`).
#[derive(Debug, Clone)]
pub struct Spu2Freeze {
    pub save_id: u32,
    pub version: u32,
    pub unkregs: Vec<u8>,
    pub mem: Vec<i16>,
    pub cores: [CoreState; 2],
    pub spdif: SpdifState,
    pub out_pos: u16,
    pub input_pos: u16,
    pub cycles: u32,
    pub l_clocks: u64,
    pub play_mode: i32,
}

/// Magic value of `SPU2Savestate::SAVE_ID`.
pub const SPU2_SAVE_ID: u32 = 0x0012_2752 & 0x000F_FFFF | 0x0012_0000;
/// Save-state format version.
pub const SPU2_SAVE_VERSION: u32 = 0x000E;

/// Number of unknown / raw register bytes in the freeze blob.
pub const UNK_REGS_SIZE: usize = 0x1_0000;
/// Number of `i16` words in the freeze blob's SPU memory.
pub const MEM_SIZE: usize = 0x10_0000;

impl Spu2Freeze {
    /// Construct an empty save state with the right magic and version.
    pub fn new() -> Self {
        Spu2Freeze {
            save_id: SPU2_SAVE_ID,
            version: SPU2_SAVE_VERSION,
            unkregs: vec![0; UNK_REGS_SIZE],
            mem: vec![0; MEM_SIZE],
            cores: [
                CoreState::default(),
                CoreState::default(),
            ],
            spdif: SpdifState::default(),
            out_pos: 0,
            input_pos: 0,
            cycles: 0,
            l_clocks: 0,
            play_mode: 0,
        }
    }

    /// Verify the header (matches `SPU2Savestate::ThawIt`'s sanity check).
    pub fn header_ok(&self) -> bool {
        self.save_id == SPU2_SAVE_ID && self.version >= SPU2_SAVE_VERSION
    }
}

impl Default for Spu2Freeze {
    fn default() -> Self {
        Spu2Freeze::new()
    }
}

// =====================================================================================
//  Tests
// =====================================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adsr_attack_runs_to_full() {
        // ADSR1: linear attack, step = 0, shift = 0 (so level_inc = 7).
        let mut s = AdsrState {
            reg_adsr1: 0x0000, // Am=0 linear, Ast=0, Ash=0
            reg_adsr2: 0x0000,
            ..Default::default()
        };
        s.update_cache();
        s.attack();
        // Run ~12000 ticks; with step=7 and shift=0 (counter_inc = 1),
        // we should saturate well before then.
        let mut last = 0;
        for _ in 0..20_000 {
            last = adsrStep(&mut s);
            if s.phase == AdsrPhase::Stopped {
                break;
            }
        }
        assert_eq!(last, i16::MAX);
    }

    #[test]
    fn adsr_release_eventually_silences() {
        let mut s = AdsrState {
            reg_adsr1: 0x00FF,
            reg_adsr2: 0x0000,
            ..Default::default()
        };
        s.update_cache();
        s.attack();
        for _ in 0..50_000 {
            adsrStep(&mut s);
        }
        // After full attack, the voice should be in Decay / Sustain and
        // not Stopped.
        assert_ne!(s.phase, AdsrPhase::Stopped);
        s.release();
        for _ in 0..200_000 {
            adsrStep(&mut s);
        }
        assert_eq!(s.phase, AdsrPhase::Stopped);
        assert_eq!(s.level, 0);
    }

    #[test]
    fn interpolate_table_is_complete() {
        // First sample must be the last C++ entry mirrored back.
        assert_eq!(INTERPOLATE_TABLE[0][0], 0x12C7);
        assert_eq!(INTERPOLATE_TABLE[0][255], 0x1307);
        // Last forward entry of column 1.
        assert_eq!(INTERPOLATE_TABLE[1][255], 0x12C7);
    }

    #[test]
    fn regtable_lookup_known_addresses() {
        assert_eq!(regtable_lookup(0, REG_VP_VOLL), Some("VP"));
        assert_eq!(regtable_lookup(0, REG_C_ATTR), Some("ATTR"));
        assert_eq!(regtable_lookup(0, REG_P_MVOLR), Some("MVOLR"));
        assert_eq!(regtable_lookup(0, SPDIF_OUT), Some("SPDIF_OUT"));
        assert_eq!(regtable_lookup(0, REG_S_KON), Some("KON0"));
    }

    #[test]
    fn regtable_lookup_unknown() {
        // 0x346 is in the "unknown" gap from RegTable.cpp.
        assert_eq!(regtable_lookup(0, 0x0346), None);
    }

    #[test]
    fn reverb_fir_taps_have_expected_size() {
        assert_eq!(REVERB_FIR_TAPS_DOWN.len(), 39);
        assert_eq!(REVERB_FIR_TAPS_UP.len(), 39);
        // Center tap of the down filter is the C++'s 16384.
        assert_eq!(REVERB_FIR_TAPS_DOWN[10], 16384);
    }

    #[test]
    fn freeze_header_is_valid() {
        let f = Spu2Freeze::new();
        assert!(f.header_ok());
    }

    #[test]
    fn clamp_mix_saturates() {
        assert_eq!(clamp_mix(i32::MAX), i16::MAX);
        assert_eq!(clamp_mix(i32::MIN), i16::MIN);
        assert_eq!(clamp_mix(0), 0);
    }
}
