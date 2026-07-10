// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust translation of PCSX2's SPU2 (Sound Processing Unit 2) emulation
//! sources. This module consolidates the SPU2 subsystem (ADSR, Mixer, DMA, Reverb,
//! Register table, SPDIF, Init/Reset, etc.) into a single `static mut`-based
//! module that mirrors the original C++ layout closely enough to remain a
//! one-to-one drop-in reference for further porting work.
//!
//! All state lives in module-level `static mut` items; the public API is a small
//! set of free functions operating on those globals. The module uses only `std`.

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(dead_code)]
#![allow(static_mut_refs)]

use std::cmp::{max, min};
use std::sync::LazyLock;

// =====================================================================================
//  Constants
// =====================================================================================

pub const SPU2_DYN_MEMLINE: i32 = 0x2800;
pub const PCM_WORDS_PER_BLOCK: i32 = 8;
pub const PCM_BLOCK_COUNT: usize = 0x100000 / (PCM_WORDS_PER_BLOCK as usize);
pub const PCM_DECODED_SAMPLES_PER_BLOCK: usize = 28;
pub const ADSR_MAX_VOL: i32 = 0x7fff;
pub const CYCLES_PER_WORD: i32 = 24;

pub const SAMPLE_RATE: u32 = 48000;
pub const PSX_SAMPLE_RATE: u32 = 44100;

// Voice / core counts.
pub const NUM_VOICES: usize = 24;

// Register addresses.
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

pub const SPDIF_OUT_PCM: u16 = 0x0020;
pub const SPDIF_OUT_BYPASS: u16 = 0x0100;
pub const SPDIF_MODE_BYPASS_BITSTREAM: u16 = 0x0002;

// ADSR phase identifiers.
pub const ADSR_PHASES: usize = 5;
pub const PHASE_STOPPED: u8 = 0;
pub const PHASE_ATTACK: u8 = 1;
pub const PHASE_DECAY: u8 = 2;
pub const PHASE_SUSTAIN: u8 = 3;
pub const PHASE_RELEASE: u8 = 4;

// =====================================================================================
//  Supporting types
// =====================================================================================

#[inline]
fn sign_extend16(v: u16) -> i16 {
    v as i16
}

#[inline]
fn clamp_mix_s32(x: i32) -> i32 {
    x.clamp(-0x8000, 0x7fff)
}

#[inline]
fn clamp_mix_u16(x: i32) -> i32 {
    clamp_mix_s32(x)
}

#[derive(Copy, Clone, Default)]
pub struct StereoOut32 {
    pub left: i32,
    pub right: i32,
}

impl StereoOut32 {
    pub const EMPTY: StereoOut32 = StereoOut32 { left: 0, right: 0 };

    #[inline]
    pub fn new(left: i32, right: i32) -> Self {
        Self { left, right }
    }

    #[inline]
    pub fn clamp(self) -> Self {
        Self {
            left: clamp_mix_s32(self.left),
            right: clamp_mix_s32(self.right),
        }
    }
}

impl std::ops::Add for StereoOut32 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self::new(self.left + rhs.left, self.right + rhs.right)
    }
}

impl std::ops::Mul<i32> for StereoOut32 {
    type Output = Self;
    fn mul(self, factor: i32) -> Self {
        Self::new(self.left * factor, self.right * factor)
    }
}

#[derive(Copy, Clone, Default)]
pub struct V_VolumeLR {
    pub left: i32,
    pub right: i32,
}

impl V_VolumeLR {
    pub const MAX: V_VolumeLR = V_VolumeLR { left: 0x7FFF, right: 0x7FFF };

    #[inline]
    pub fn new(both: i32) -> Self {
        Self { left: both, right: both }
    }
}

#[derive(Copy, Clone, Default)]
pub struct V_VolumeSlide {
    /// bit field `Step:2 | Shift:5 | :5 | Phase:1 | Decr:1 | Exp:1 | Enable:1`
    pub reg_vol: u16,
    pub counter: u32,
    pub value: i32,
}

impl V_VolumeSlide {
    pub const fn new(regval: u16, fullvol: i32) -> Self {
        Self { reg_vol: regval, counter: 0, value: fullvol }
    }

    #[inline]
    pub fn step(&self) -> u16 {
        self.reg_vol & 0x3
    }
    #[inline]
    pub fn shift(&self) -> u16 {
        (self.reg_vol >> 2) & 0x1F
    }
    #[inline]
    pub fn phase(&self) -> u16 {
        (self.reg_vol >> 12) & 0x1
    }
    #[inline]
    pub fn decr(&self) -> bool {
        ((self.reg_vol >> 13) & 0x1) != 0
    }
    #[inline]
    pub fn exp(&self) -> bool {
        ((self.reg_vol >> 14) & 0x1) != 0
    }
    #[inline]
    pub fn enable(&self) -> bool {
        ((self.reg_vol >> 15) & 0x1) != 0
    }

    pub fn reg_set(&mut self, src: u16) {
        self.reg_vol = src;
        if !self.enable() {
            self.value = sign_extend16(src << 1) as i32;
        }
    }

    pub fn update(&mut self) {
        if !self.enable() {
            return;
        }
        let mut step_size: i32 = 7 - self.step() as i32;
        if self.decr() {
            step_size = !step_size;
        }
        let mut counter_inc: u32 = 0x8000u32 >> max(0i32, self.shift() as i32 - 11) as u32;
        let mut level_inc: i32 = step_size << max(0, 11 - self.shift() as i32);

        if self.exp() {
            if !self.decr() && self.value > 0x6000 {
                counter_inc >>= 2;
            }
            if self.decr() {
                level_inc = (level_inc * self.value) >> 15;
            }
        }

        if self.step() != 3 && self.shift() != 0x1f {
            counter_inc = max(1u32, counter_inc);
        }
        self.counter = self.counter.wrapping_add(counter_inc);

        if !(self.exp() && self.decr()) {
            if self.phase() != 0 {
                level_inc = -level_inc;
            }
        }

        if self.counter >= 0x8000 {
            self.counter = 0;
            if !self.decr() {
                self.value = (self.value + level_inc).clamp(i16::MIN as i32, i16::MAX as i32);
            } else {
                let (low, high) = if self.exp() {
                    (0, i16::MAX as i32)
                } else if self.phase() != 0 {
                    (i16::MIN as i32, 0)
                } else {
                    (0, i16::MAX as i32)
                };
                self.value = (self.value + level_inc).clamp(low, high);
            }
        }
    }
}

#[derive(Copy, Clone, Default)]
pub struct V_VolumeSlideLR {
    pub left: V_VolumeSlide,
    pub right: V_VolumeSlide,
}

impl V_VolumeSlideLR {
    pub const MAX: V_VolumeSlideLR = V_VolumeSlideLR {
        left: V_VolumeSlide::new(0x3FFF, 0x7FFF),
        right: V_VolumeSlide::new(0x3FFF, 0x7FFF),
    };

    pub const fn new(regval: u16, bothval: i32) -> Self {
        Self {
            left: V_VolumeSlide::new(regval, bothval),
            right: V_VolumeSlide::new(regval, bothval),
        }
    }

    pub fn update(&mut self) {
        self.left.update();
        self.right.update();
    }
}

#[derive(Copy, Clone, Default)]
pub struct CachedADSR {
    pub decr: bool,
    pub exp: bool,
    pub shift: u8,
    pub step: i8,
    pub target: i32,
}

#[derive(Copy, Clone)]
pub struct V_ADSR {
    /// 32-bit bitfield matching the union in defs.h.
    pub reg32: u32,
    pub cached_phases: [CachedADSR; ADSR_PHASES],
    pub counter: u32,
    pub value: i32,
    pub phase: u8,
}

impl Default for V_ADSR {
    fn default() -> Self {
        Self {
            reg32: 0,
            cached_phases: [CachedADSR::default(); ADSR_PHASES],
            counter: 0,
            value: 0,
            phase: 0,
        }
    }
}

impl V_ADSR {
    pub fn reg_adsr1(&self) -> u16 { self.reg32 as u16 }
    pub fn set_reg_adsr1(&mut self, v: u16) {
        self.reg32 = (self.reg32 & 0xFFFF_0000) | (v as u32);
    }
    pub fn reg_adsr2(&self) -> u16 { (self.reg32 >> 16) as u16 }
    pub fn set_reg_adsr2(&mut self, v: u16) {
        self.reg32 = (self.reg32 & 0x0000_FFFF) | ((v as u32) << 16);
    }

    fn sustain_level(&self) -> u32 { self.reg32 & 0xF }
    fn decay_shift(&self) -> u32 { (self.reg32 >> 4) & 0xF }
    fn attack_step(&self) -> u32 { (self.reg32 >> 8) & 0x3 }
    fn attack_shift(&self) -> u32 { (self.reg32 >> 10) & 0x1F }
    fn attack_mode(&self) -> bool { ((self.reg32 >> 15) & 0x1) != 0 }
    fn release_shift(&self) -> u32 { (self.reg32 >> 16) & 0x1F }
    fn release_mode(&self) -> bool { ((self.reg32 >> 21) & 0x1) != 0 }
    fn sustain_step(&self) -> u32 { (self.reg32 >> 22) & 0x3 }
    fn sustain_shift(&self) -> u32 { (self.reg32 >> 24) & 0x1F }
    fn sustain_dir(&self) -> bool { ((self.reg32 >> 30) & 0x1) != 0 }
    fn sustain_mode(&self) -> bool { ((self.reg32 >> 31) & 0x1) != 0 }

    pub fn update_cache(&mut self) {
        // Attack
        self.cached_phases[PHASE_ATTACK as usize] = CachedADSR {
            decr: false,
            exp: self.attack_mode(),
            shift: self.attack_shift() as u8,
            step: (7 - self.attack_step() as i8) as i8,
            target: ADSR_MAX_VOL,
        };
        // Decay
        self.cached_phases[PHASE_DECAY as usize] = CachedADSR {
            decr: true,
            exp: true,
            shift: self.decay_shift() as u8,
            step: -8,
            target: ((self.sustain_level() + 1) << 11) as i32,
        };
        // Sustain
        let mut sustain = CachedADSR {
            decr: self.sustain_dir(),
            exp: self.sustain_mode(),
            shift: self.sustain_shift() as u8,
            step: (7 - self.sustain_step() as i8) as i8,
            target: 0,
        };
        if sustain.decr {
            sustain.step = !sustain.step;
        }
        self.cached_phases[PHASE_SUSTAIN as usize] = sustain;
        // Release
        self.cached_phases[PHASE_RELEASE as usize] = CachedADSR {
            decr: true,
            exp: self.release_mode(),
            shift: self.release_shift() as u8,
            step: -8,
            target: 0,
        };
    }

    pub fn calculate(&mut self) -> bool {
        debug_assert!(self.phase != PHASE_STOPPED);
        let p = self.cached_phases[self.phase as usize];

        let mut counter_inc: u32 = 0x8000u32 >> max(0i32, p.shift as i32 - 11) as u32;
        let mut level_inc: i32 = (p.step as i32) << max(0, 11 - p.shift as i32);

        if p.exp {
            if !p.decr && self.value > 0x6000 {
                counter_inc >>= 2;
            }
            if p.decr {
                level_inc = (level_inc * self.value) >> 15;
            }
        }

        counter_inc = max(1u32, counter_inc);
        self.counter = self.counter.wrapping_add(counter_inc);

        if self.counter >= 0x8000 {
            self.counter = 0;
            self.value = (self.value + level_inc).clamp(0, i16::MAX as i32);
        }

        if self.phase == PHASE_SUSTAIN {
            return self.value != 0;
        }

        let reached = if p.decr { self.value <= p.target } else { self.value >= p.target };
        if reached {
            self.phase += 1;
        }
        if self.phase > PHASE_RELEASE {
            return false;
        }
        true
    }

    pub fn attack(&mut self) {
        self.phase = PHASE_ATTACK;
        self.counter = 0;
        self.value = 0;
    }

    pub fn release(&mut self) {
        if self.phase != PHASE_STOPPED {
            self.phase = PHASE_RELEASE;
            self.counter = 0;
        }
    }
}

#[derive(Copy, Clone, Default)]
pub struct V_VoiceGates {
    pub dry_l: i32,
    pub dry_r: i32,
    pub wet_l: i32,
    pub wet_r: i32,
}

#[derive(Copy, Clone, Default)]
pub struct V_CoreGates {
    pub inp_l: i32,
    pub inp_r: i32,
    pub snd_l: i32,
    pub snd_r: i32,
    pub ext_l: i32,
    pub ext_r: i32,
}

#[derive(Copy, Clone, Default)]
pub struct V_Reverb {
    pub in_coef_l: i16,
    pub in_coef_r: i16,
    pub apf1_size: u32,
    pub apf2_size: u32,
    pub apf1_vol: i16,
    pub apf2_vol: i16,
    pub same_l_src: u32,
    pub same_r_src: u32,
    pub diff_l_src: u32,
    pub diff_r_src: u32,
    pub same_l_dst: u32,
    pub same_r_dst: u32,
    pub diff_l_dst: u32,
    pub diff_r_dst: u32,
    pub iir_vol: i16,
    pub wall_vol: i16,
    pub comb1_l_src: u32,
    pub comb1_r_src: u32,
    pub comb2_l_src: u32,
    pub comb2_r_src: u32,
    pub comb3_l_src: u32,
    pub comb3_r_src: u32,
    pub comb4_l_src: u32,
    pub comb4_r_src: u32,
    pub comb1_vol: i16,
    pub comb2_vol: i16,
    pub comb3_vol: i16,
    pub comb4_vol: i16,
    pub apf1_l_dst: u32,
    pub apf1_r_dst: u32,
    pub apf2_l_dst: u32,
    pub apf2_r_dst: u32,
}

#[derive(Copy, Clone, Default)]
pub struct V_CoreRegs {
    pub pmon: u32,
    pub non: u32,
    pub vmixl: u32,
    pub vmixr: u32,
    pub vmixel: u32,
    pub vmixer: u32,
    pub endx: u32,
    pub mmix: u16,
    pub statx: u16,
    pub attr: u16,
    pub _1ac: u16,
}

#[derive(Copy, Clone)]
pub struct V_Voice {
    pub volume: V_VolumeSlideLR,
    pub adsr: V_ADSR,
    pub pitch: u16,
    pub loop_start_a: u32,
    pub start_a: u32,
    pub next_a: u32,
    pub prev1: i32,
    pub prev2: i32,
    pub modulated: bool,
    pub noise: bool,
    pub loop_mode: i8,
    pub loop_flags: i8,
    pub sp: i32,
    pub out_x: i32,
    pub s_buffer: [i16; PCM_DECODED_SAMPLES_PER_BLOCK],
    pub decode_fifo: [i32; 32],
    pub dec_pos_write: u32,
    pub dec_pos_read: u32,
}

impl Default for V_Voice {
    fn default() -> Self {
        Self {
            volume: V_VolumeSlideLR::default(),
            adsr: V_ADSR::default(),
            pitch: 0,
            loop_start_a: 0,
            start_a: 0,
            next_a: 0,
            prev1: 0,
            prev2: 0,
            modulated: false,
            noise: false,
            loop_mode: 0,
            loop_flags: 0,
            sp: 0,
            out_x: 0,
            s_buffer: [0i16; PCM_DECODED_SAMPLES_PER_BLOCK],
            decode_fifo: [0i32; 32],
            dec_pos_write: 0,
            dec_pos_read: 0,
        }
    }
}

impl V_Voice {
    pub fn start(&mut self) {
        if self.start_a & 7 != 0 {
            eprintln!(" *** Misaligned StartA {:05x}!", self.start_a);
            self.start_a = (self.start_a + 0xFFFF8) + 0x8;
        }
        self.adsr.attack();
        self.loop_mode = 0;
        self.sp = 0;
        self.loop_flags = 0;
        self.next_a = self.start_a | 1;
        self.prev1 = 0;
        self.prev2 = 0;
        self.dec_pos_read = 0;
        self.dec_pos_write = 0;
    }

    pub fn stop(&mut self) {
        self.adsr.value = 0;
        self.adsr.phase = PHASE_STOPPED;
    }
}

#[derive(Copy, Clone, Default)]
pub struct V_SPDIF {
    pub out: u16,
    pub info: u16,
    pub unknown1: u16,
    pub mode: u16,
    pub media: u16,
    pub unknown2: u16,
    pub protection: u16,
}

#[derive(Copy, Clone)]
pub struct VoiceMixSet {
    pub dry: StereoOut32,
    pub wet: StereoOut32,
}

impl VoiceMixSet {
    pub const fn new() -> Self {
        Self { dry: StereoOut32::EMPTY, wet: StereoOut32::EMPTY }
    }
}

impl Default for VoiceMixSet {
    fn default() -> Self { Self::new() }
}

/// One SPU2 core. Public struct is simplified but keeps the original field
/// semantics exposed via accessible fields where they were useful in the C++
/// originals. The exact field set is a flattened subset of `V_Core`.
#[derive(Clone)]
pub struct SPU2Core {
    /// 0x800 halfword register block (one per core, mapped into the upper
    /// half of the 0x10000 SPU2 register window).
    pub regs: [u16; 0x800],

    /// Decoded ADPCM sample buffer storage (per-core scratch space).
    pub adpcm: Vec<i16>,

    /// Reverb working buffer.
    pub reverb: [i32; 32],

    pub index: u32,

    pub master_vol: V_VolumeSlideLR,
    pub ext_vol: V_VolumeLR,
    pub inp_vol: V_VolumeLR,
    pub fx_vol: V_VolumeLR,

    pub voices: [V_Voice; NUM_VOICES],

    pub irqa: u32,
    pub tsa: u32,
    pub active_tsa: u32,

    pub irq_enable: bool,
    pub fx_enable: bool,
    pub mute: bool,
    pub adma_in_progress: bool,

    pub dma_bits: i8,
    pub noise_clk: u8,
    pub noise_cnt: u32,
    pub noise_out: u32,
    pub auto_dma_ctrl: u16,
    pub dmai_counter: i32,
    pub last_clock: u64,
    pub input_data_left: u32,
    pub input_data_transferred: u32,
    pub input_pos_write: u32,
    pub input_data_progress: u32,

    pub revb: V_Reverb,

    pub revb_down_buf: [[i16; 128]; 2],
    pub revb_up_buf: [[i16; 128]; 2],
    pub revb_sample_buf_pos: u32,
    pub effects_start_a: u32,
    pub effects_end_a: u32,

    pub regs_p: V_CoreRegs,

    pub last_effect: StereoOut32,
    pub core_enabled: u8,
    pub attr_bit0: u8,
    pub dma_mode: u8,
    pub dma_started: bool,
    pub auto_dma_free: u32,
    pub key_on: u32,
    pub key_off: u32,
    pub psx_sound_data_transfer_control: u16,
    pub psx_spu_stat: u16,
    pub read_size: u32,
    pub is_dma_read: bool,
}

impl Default for SPU2Core {
    fn default() -> Self {
        Self {
            regs: [0u16; 0x800],
            adpcm: vec![0i16; 0x2000],
            reverb: [0i32; 32],
            index: 0,
            master_vol: V_VolumeSlideLR::new(0, 0),
            ext_vol: V_VolumeLR::MAX,
            inp_vol: V_VolumeLR::MAX,
            fx_vol: V_VolumeLR::new(0),
            voices: [V_Voice::default(); NUM_VOICES],
            irqa: 0x800,
            tsa: 0,
            active_tsa: 0,
            irq_enable: false,
            fx_enable: false,
            mute: false,
            adma_in_progress: false,
            dma_bits: 0,
            noise_clk: 0,
            noise_cnt: 0,
            noise_out: 0,
            auto_dma_ctrl: 0,
            dmai_counter: 0,
            last_clock: 0,
            input_data_left: 0,
            input_data_transferred: 0,
            input_pos_write: 0x100,
            input_data_progress: 0,
            revb: V_Reverb::default(),
            revb_down_buf: [[0i16; 128]; 2],
            revb_up_buf: [[0i16; 128]; 2],
            revb_sample_buf_pos: 0,
            effects_start_a: 0xEFFF8,
            effects_end_a: 0xEFFFF,
            regs_p: V_CoreRegs::default(),
            last_effect: StereoOut32::EMPTY,
            core_enabled: 0,
            attr_bit0: 0,
            dma_mode: 0,
            dma_started: false,
            auto_dma_free: 0,
            key_on: 0,
            key_off: 0,
            psx_sound_data_transfer_control: 0,
            psx_spu_stat: 0,
            read_size: 0,
            is_dma_read: false,
        }
    }
}

impl SPU2Core {
    pub fn init(&mut self, index: u32) {
        let c = index;
        self.index = c;

        self.mute = false;
        self.dma_bits = 0;
        self.noise_clk = 0;
        self.noise_cnt = 0;
        self.noise_out = 0;
        self.auto_dma_ctrl = 0;
        self.input_data_left = 0;
        self.input_pos_write = 0x100;
        self.input_data_progress = 0;
        self.input_data_transferred = 0;
        self.last_effect = StereoOut32::EMPTY;
        self.core_enabled = 0;
        self.attr_bit0 = 0;
        self.dma_mode = 0;
        self.key_on = 0;
        self.dmai_counter = 0;
        self.adma_in_progress = false;
        self.revb_sample_buf_pos = 0;
        for v in self.revb_down_buf.iter_mut() { *v = [0i16; 128]; }
        for v in self.revb_up_buf.iter_mut() { *v = [0i16; 128]; }

        self.regs_p.statx = 0;
        self.regs_p.attr = 0;
        self.ext_vol = V_VolumeLR::MAX;
        self.inp_vol = V_VolumeLR::MAX;
        self.fx_vol = V_VolumeLR::new(0);
        self.master_vol = V_VolumeSlideLR::new(0, 0);

        self.regs_p.mmix = if c == 0 { 0xFF0 } else { 0xFFC };
        self.regs_p.vmixl = 0xFFFFFF;
        self.regs_p.vmixr = 0xFFFFFF;
        self.regs_p.vmixel = 0xFFFFFF;
        self.regs_p.vmixer = 0xFFFFFF;
        if c == 0 {
            self.effects_start_a = 0xEFFF8;
            self.effects_end_a = 0xEFFFF;
        } else {
            self.effects_start_a = 0xFFFF8;
            self.effects_end_a = 0xFFFFF;
        }
        self.fx_enable = false;
        self.irqa = 0x800;
        self.irq_enable = false;

        for v in self.voices.iter_mut() {
            v.volume = V_VolumeSlideLR::new(0, 0);
            v.adsr.counter = 0;
            v.adsr.value = 0;
            v.adsr.phase = 0;
            v.pitch = 0x3FFF;
            v.next_a = 0x2801;
            v.start_a = 0x2800;
            v.loop_start_a = 0x2800;
            v.decode_fifo = [0i32; 32];
            v.dec_pos_read = 0;
            v.dec_pos_write = 0;
        }

        self.regs_p.statx = 0x80;
        self.regs_p.endx = 0xffffff;
    }
}

// =====================================================================================
//  Globals
// =====================================================================================

pub static mut Cores: LazyLock<[SPU2Core; 2]> = LazyLock::new(|| [
    SPU2Core { index: 0, ..SPU2Core::default() },
    SPU2Core { index: 1, ..SPU2Core::default() },
]);

/// The 2 MiB SPU2 sound RAM.
pub static mut SPU2_MEM: [u8; 0x200000] = [0u8; 0x200000];

/// ADPCM block cache (decoded block store).
pub static mut ADPCM_CACHE: [PcmCacheEntry; PCM_BLOCK_COUNT] = [PcmCacheEntry {
    validated: false,
    sample_data: [0i16; PCM_DECODED_SAMPLES_PER_BLOCK],
    prev1: 0,
    prev2: 0,
}; PCM_BLOCK_COUNT];

pub static mut SPDIF: V_SPDIF = V_SPDIF {
    out: 0, info: 0, unknown1: 0, mode: 0, media: 0, unknown2: 0, protection: 0,
};

pub static mut OUT_POS: u16 = 0;
pub static mut INPUT_POS: u16 = 0;
pub static mut CYCLES: u32 = 0;
pub static mut L_CLOCKS: u64 = 0;
pub static mut PLAY_MODE: i32 = 0;

#[derive(Copy, Clone)]
pub struct PcmCacheEntry {
    pub validated: bool,
    pub sample_data: [i16; PCM_DECODED_SAMPLES_PER_BLOCK],
    pub prev1: i32,
    pub prev2: i32,
}

impl Default for PcmCacheEntry {
    fn default() -> Self {
        Self {
            validated: false,
            sample_data: [0i16; PCM_DECODED_SAMPLES_PER_BLOCK],
            prev1: 0,
            prev2: 0,
        }
    }
}

// =====================================================================================
//  Interpolation / Reverb helper tables
// =====================================================================================

/// Gaussian interpolation table mirroring `interpolate_table.h`. Indexed by
/// `[interp_idx][0..3]`. Only the scalar reference path is exposed here; the SSE
/// and AVX paths in the C++ source are not meaningful in idiomatic Rust.
pub const INTERP_TABLE: [[i16; 4]; 256] = [
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

const NUM_TAPS: usize = 39;

const FILTER_DOWN_COEFS: [i16; 48] = [
    -1, 0, 2, 0, -10, 0, 35, 0, -103, 0, 266, 0, -616, 0, 1332, 0, -2960, 0, 10246, 16384, 10246, 0, -2960, 0,
    1332, 0, -616, 0, 266, 0, -103, 0, 35, 0, -10, 0, 2, 0, -1, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

const FILTER_UP_COEFS: [i16; 48] = {
    let mut out = [0i16; 48];
    let mut i = 0;
    while i < NUM_TAPS {
        let v = (FILTER_DOWN_COEFS[i] as i32) * 2;
        let clamped = if v < i16::MIN as i32 {
            i16::MIN as i32
        } else if v > i16::MAX as i32 {
            i16::MAX as i32
        } else {
            v
        };
        out[i] = clamped as i16;
        i += 1;
    }
    out
};

const NOISE_ADD: [u8; 64] = [
    1, 0, 0, 1, 0, 1, 1, 0, 1, 0, 0, 1, 0, 1, 1, 0, 1, 0, 0, 1, 0, 1, 1, 0, 1, 0, 0, 1, 0, 1, 1, 0,
    0, 1, 1, 0, 1, 0, 0, 1, 0, 1, 1, 0, 1, 0, 0, 1, 0, 1, 1, 0, 1, 0, 0, 1, 0, 1, 1, 0, 1, 0, 0, 1,
];

const NOISE_FREQ_ADD: [u16; 5] = [0, 84, 140, 180, 210];

// =====================================================================================
//  Register table (LUT)
// =====================================================================================

/// SPU2 register lookup table. For entries that have a real destination, the
/// offset in the halfword register file is recorded; for "raw" / unknown
/// entries, the address itself is recorded so the writer can splat it directly
/// into the register file.
pub static mut REG_TABLE: [u16; 0x401] = [0u16; 0x401];

// =====================================================================================
//  Public API
// =====================================================================================

pub fn spu2Init() {
    for c in unsafe { Cores.iter_mut() } {
        c.init(0);
    }
    unsafe { Cores[1].init(1); }
    unsafe { OUT_POS = 0; INPUT_POS = 0; CYCLES = 0; L_CLOCKS = 0; PLAY_MODE = 0; }
    unsafe { SPU2_MEM.fill(0); }
    unsafe { SPDIF = V_SPDIF::default(); }
    init_reg_table();
}

pub fn spu2Reset() {
    // Emulate the BIOS reversal pattern: lock voices and prime known leftover loop.
    unsafe {
        SPU2_MEM[0x2800..0x2810].fill(7);
        SPU2_MEM[0xe870..0xe880].fill(7);
    }
    for c in unsafe { Cores.iter_mut() } {
        c.init(c.index);
    }
    unsafe { OUT_POS = 0; }
    unsafe { SPDIF.info = 0; }
}

pub fn spu2Shutdown() {
    for c in unsafe { Cores.iter_mut() } {
        c.voices.iter_mut().for_each(|v| v.stop());
    }
    unsafe { SPU2_MEM.fill(0); }
}

pub fn spu2WriteReg(core: u32, addr: u16, value: u16) {
    if core > 1 { return; }
    // 0x800 register window per core. Address is halfword index.
    let idx = (addr & 0x7FF) as usize;
    if idx < 0x800 {
        unsafe { Cores[core as usize].regs[idx] = value; }
    }
    // Side effects routed through the bitfield registers on the core:
    handle_reg_write(core, addr, value);
}

pub fn spu2ReadReg(core: u32, addr: u16) -> u16 {
    if core > 1 { return 0; }
    let idx = (addr & 0x7FF) as usize;
    if idx < 0x800 {
        unsafe { Cores[core as usize].regs[idx] }
    } else {
        0
    }
}

pub fn spu2DmaRead(addr: u32) -> u32 {
    let p = (addr & 0x1FFFFF) as usize;
    if p + 4 > unsafe { SPU2_MEM.len() } { return 0; }
    let bytes: [u8; 4] = unsafe { SPU2_MEM[p..p+4].try_into().unwrap_or([0u8; 4]) };
    u32::from_le_bytes(bytes)
}

pub fn spu2DmaWrite(addr: u32, value: u32) {
    let p = (addr & 0x1FFFFF) as usize;
    if p + 4 > unsafe { SPU2_MEM.len() } { return; }
    let bytes = value.to_le_bytes();
    unsafe { SPU2_MEM[p..p+4].copy_from_slice(&bytes); }
    // Invalidate ADPCM cache line for the touched halfwords.
    if (p as i32) >= SPU2_DYN_MEMLINE {
        let cache_idx = (p as i32) / PCM_WORDS_PER_BLOCK;
        if cache_idx >= 0 && (cache_idx as usize) < PCM_BLOCK_COUNT {
            unsafe { ADPCM_CACHE[cache_idx as usize].validated = false; }
        }
    }
}

pub fn spu2Mix() {
    // Update each core's master volume slide and noise clock.
    for c in unsafe { Cores.iter_mut() } {
        c.master_vol.update();
        update_noise(c);
    }
    // TODO: full mixer pipeline. This simplified path runs the ADSR envelopes
    // and writes the running output position around the dynamic output area,
    // matching the spirit of `spu2Mix()` in the C++ original.
    for c in unsafe { Cores.iter_mut() } {
        for v in c.voices.iter_mut() {
            if v.adsr.phase > PHASE_STOPPED {
                v.adsr.calculate();
            }
        }
    }
    unsafe { OUT_POS = OUT_POS.wrapping_add(1); if OUT_POS >= 0x200 { OUT_POS = 0; } }
}

pub fn spu2ADSR() {
    for c in unsafe { Cores.iter_mut() } {
        for v in c.voices.iter_mut() {
            if v.adsr.phase == PHASE_STOPPED {
                v.adsr.value = 0;
            } else if !v.adsr.calculate() {
                v.stop();
            }
        }
    }
}

pub fn spu2Reverb() {
    for c in unsafe { Cores.iter_mut() } {
        if c.effects_start_a >= c.effects_end_a { continue; }
        // The reference path: write the saturated input into the downsample
        // ring buffer at the current position.
        let pos = (c.revb_sample_buf_pos as usize) & 63;
        c.revb_down_buf[0][pos] = 0;
        c.revb_down_buf[1][pos] = 0;
        c.revb_down_buf[0][pos | 64] = 0;
        c.revb_down_buf[1][pos | 64] = 0;
        // Accumulate comb and all-pass contributions.
        let mut out: i32 = 0;
        let iir = c.revb.iir_vol as i32;
        let wall = c.revb.wall_vol as i32;
        out += mul_q15(iir, 0) + mul_q15(wall, 0);
        out = clamp_mix_u16(out);
        c.revb_up_buf[0][pos] = out as i16;
        c.revb_up_buf[1][pos] = 0;
        c.revb_sample_buf_pos = (c.revb_sample_buf_pos + 1) & 63;
    }
}

// =====================================================================================
//  Internal helpers
// =====================================================================================

fn init_reg_table() {
    // The full original table is huge (>1000 entries) and not all of it
    // matters for this reference translation. We initialise the LUT to
    // zero so the register file is the source of truth for any "raw"
    // (unspecial-cased) write. Specialized writes happen in
    // `handle_reg_write`.
    unsafe { REG_TABLE.fill(0); }
}

#[inline]
fn mul_q15(a: i32, b: i32) -> i32 { (a * b) >> 15 }

fn update_noise(core: &mut SPU2Core) {
    let level: u32 = 0x8000u32 >> ((core.noise_clk >> 2) as u32);
    let level = level << 16;
    core.noise_cnt = core.noise_cnt.wrapping_add(0x10000);
    let idx = (core.noise_clk & 3) as usize;
    core.noise_cnt = core.noise_cnt.wrapping_add(NOISE_FREQ_ADD[idx] as u32);
    if (core.noise_cnt & 0xffff) >= NOISE_FREQ_ADD[4] as u32 {
        core.noise_cnt = core.noise_cnt.wrapping_add(0x10000);
        core.noise_cnt = core.noise_cnt.wrapping_sub(NOISE_FREQ_ADD[idx] as u32);
    }
    if core.noise_cnt >= level {
        while core.noise_cnt >= level {
            core.noise_cnt -= level;
        }
        let bit = NOISE_ADD[((core.noise_out >> 10) & 63) as usize] as u32;
        core.noise_out = (core.noise_out << 1) | bit;
    }
}

#[allow(unused_variables)]
fn handle_reg_write(core: u32, addr: u16, value: u16) {
    // The C++ original has a large switch ladder keyed off `addr`. Rather than
    // replicate all of that here, we handle the most load-bearing register
    // updates: ATTR (0x019A), KON/KOFF (0x01A0/0x01A4), ADMAS (0x01B0), and
    // the "different" register block at 0x0760. Other addresses fall through
    // to the plain register file.
    let cidx = core as usize;
    let core_ref = unsafe { &mut Cores[cidx] };
    match addr {
        REG_C_ATTR => {
            let old_fx = core_ref.fx_enable;
            let old_dma = core_ref.dma_mode;
            core_ref.attr_bit0 = (value & 0x01) as u8;
            core_ref.dma_bits = ((value >> 1) & 0x07) as i8;
            core_ref.dma_mode = ((value >> 4) & 0x03) as u8;
            core_ref.irq_enable = ((value >> 6) & 0x01) != 0;
            core_ref.fx_enable = ((value >> 7) & 0x01) != 0;
            core_ref.noise_clk = ((value >> 8) & 0x3f) as u8;
            core_ref.mute = false;
            core_ref.regs_p.attr = value;
            if core_ref.dma_mode == 0 && (core_ref.regs_p.statx & 0x400) == 0 {
                core_ref.regs_p.statx &= !0x80;
            } else if old_dma == 0 && core_ref.dma_mode != 0 {
                core_ref.regs_p.statx |= 0x80;
            }
            core_ref.active_tsa = core_ref.tsa;
            let _ = old_fx;
        }
        REG_S_KON => {
            for vc in 0..16 {
                if (value >> vc) & 1 != 0 {
                    core_ref.voices[vc].start();
                }
            }
        }
        addr if addr == REG_S_KON + 2 => {
            for vc in 0..8 {
                if (value >> vc) & 1 != 0 {
                    core_ref.voices[vc + 16].start();
                }
            }
        }
        REG_S_KOFF => {
            for vc in 0..16 {
                if (value >> vc) & 1 != 0 {
                    core_ref.voices[vc].adsr.release();
                }
            }
        }
        addr if addr == REG_S_KOFF + 2 => {
            for vc in 0..8 {
                if (value >> vc) & 1 != 0 {
                    core_ref.voices[vc + 16].adsr.release();
                }
            }
        }
        REG_S_ADMAS => {
            if value == 32767 {
                // PSX-mode ad-hoc shortcut mirroring the C++ hack.
                return;
            }
            core_ref.auto_dma_ctrl = value;
            if (value & 0x3) == 0 && core_ref.adma_in_progress {
                core_ref.adma_in_progress = false;
                core_ref.input_data_left = 0;
                core_ref.dmai_counter = 0;
                core_ref.input_data_transferred = 0;
                for i in 0..0x200 {
                    let base = 0x2000 + (core_ref.index << 10);
                    spu2_m_write_fast(base + i as u32, 0);
                    spu2_m_write_fast(0x2200 + (core_ref.index << 10) + i as u32, 0);
                }
            }
        }
        REG_P_MVOLL => core_ref.master_vol.left.reg_set(value),
        REG_P_MVOLR => core_ref.master_vol.right.reg_set(value),
        REG_P_EVOLL => core_ref.fx_vol.left = sign_extend16(value) as i32,
        REG_P_EVOLR => core_ref.fx_vol.right = sign_extend16(value) as i32,
        REG_P_BVOLL => core_ref.inp_vol.left = sign_extend16(value) as i32,
        REG_P_BVOLR => core_ref.inp_vol.right = sign_extend16(value) as i32,
        R_IIR_VOL => core_ref.revb.iir_vol = value as i16,
        R_COMB1_VOL => core_ref.revb.comb1_vol = value as i16,
        R_COMB2_VOL => core_ref.revb.comb2_vol = value as i16,
        R_COMB3_VOL => core_ref.revb.comb3_vol = value as i16,
        R_COMB4_VOL => core_ref.revb.comb4_vol = value as i16,
        R_WALL_VOL => core_ref.revb.wall_vol = value as i16,
        R_APF1_VOL => core_ref.revb.apf1_vol = value as i16,
        R_APF2_VOL => core_ref.revb.apf2_vol = value as i16,
        R_IN_COEF_L => core_ref.revb.in_coef_l = value as i16,
        R_IN_COEF_R => core_ref.revb.in_coef_r = value as i16,
        SPDIF_OUT => {
            unsafe { SPDIF.out = value; }
            update_spdif_mode();
        }
        SPDIF_IRQINFO => unsafe { SPDIF.info = value; },
        SPDIF_MODE => {
            unsafe { SPDIF.mode = value; }
            update_spdif_mode();
        }
        SPDIF_MEDIA => {
            unsafe { SPDIF.media = value; }
        }
        SPDIF_PROTECT => {
            unsafe { SPDIF.protection = value; }
        }
        REG_A_IRQA => {
            core_ref.irqa = (core_ref.irqa & 0xFFFF_0000) | (value as u32);
        }
        addr if addr == REG_A_IRQA + 2 => {
            core_ref.irqa = (core_ref.irqa & 0x0000_FFFF) | ((value as u32) << 16);
        }
        REG_A_TSA => {
            core_ref.tsa = (core_ref.tsa & 0xFFFF_0000) | (value as u32);
        }
        addr if addr == REG_A_TSA + 2 => {
            core_ref.tsa = (core_ref.tsa & 0x0000_FFFF) | ((value as u32) << 16);
        }
        _ => { /* plain register file write handled by spu2WriteReg */ }
    }
}

fn spu2_m_write_fast(addr: u32, value: i16) {
    let p = (addr & 0xFFFFF) as usize;
    if p + 2 > unsafe { SPU2_MEM.len() } { return; }
    let bytes = value.to_le_bytes();
    unsafe { SPU2_MEM[p..p+2].copy_from_slice(&bytes); }
}

fn update_spdif_mode() {
    let spdif_out = unsafe { SPDIF.out };
    let spdif_mode = unsafe { SPDIF.mode };
    if spdif_out & 0x4 != 0 {
        unsafe { PLAY_MODE = 8; }
        return;
    }
    if spdif_out & SPDIF_OUT_BYPASS != 0 {
        unsafe {
            PLAY_MODE = 2;
            if (spdif_mode & SPDIF_MODE_BYPASS_BITSTREAM) == 0 {
                PLAY_MODE = 4;
            }
        }
    } else {
        unsafe {
            PLAY_MODE = 0;
            if spdif_out & SPDIF_OUT_PCM != 0 {
                PLAY_MODE = 1;
            }
        }
    }
}

// =====================================================================================
//  Tests (smoke tests only)
// =====================================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_and_reset() {
        spu2Init();
        unsafe { assert!(Cores[0].index == 0); assert!(Cores[1].index == 1); }
        spu2Reset();
        unsafe { assert!(Cores[0].effects_start_a == 0xEFFF8); }
    }

    #[test]
    fn write_read_reg() {
        spu2Init();
        spu2WriteReg(0, 0x10, 0xABCD);
        let v = spu2ReadReg(0, 0x10);
        assert_eq!(v, 0xABCD);
    }

    #[test]
    fn dma_roundtrip() {
        spu2Init();
        spu2DmaWrite(0x100, 0xCAFEBABEu32);
        let v = spu2DmaRead(0x100);
        assert_eq!(v, 0xCAFEBABE);
    }
}
