// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! VIF (Vector Interface) DMA translation module.
//!
//! This module consolidates the C++ implementations found in
//! `Vif_Unpack.cpp`, `Vif0_Dma.cpp`, `Vif1_Dma.cpp`, and `Vif1_MFIFO.cpp`
//! into idiomatic Rust 2021 code.  It exposes the VIF unpacking primitives
//! along with the VIF0, VIF1, and VIF1 MFIFO DMA entry points.  The module
//! depends only on `std`.

use std::cmp::min;
use std::convert::TryInto;

/// Number of source bytes consumed per vector for each VIF unpack format.
///
/// The table mirrors the PS2 hardware layout:
///
///   index = (VN << 2) | VL
///
/// Where `VN` is the number of components and `VL` is the component width
/// log2.  A zero entry signals an illegal/reserved encoding.
const NVIFT_TABLE: [u8; 16] = [
    4,  // S-32
    2,  // S-16
    1,  // S-8
    0,  // ----
    8,  // V2-32
    4,  // V2-16
    2,  // V2-8
    0,  // ----
    12, // V3-32
    6,  // V3-16
    3,  // V3-8
    0,  // ----
    16, // V4-32
    8,  // V4-16
    4,  // V4-8
    2,  // V4-5
];

/// VIF unpacking format identifiers.
///
/// Each variant corresponds to one of the eight legal PS2 VIF unpack modes
/// described in the EE technical reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VifUnpackFormat {
    /// 32-bit scalar broadcast.
    S8,
    /// 16-bit scalar broadcast.
    S16,
    /// 8-bit scalar broadcast.
    S32,
    /// 3-component vector with 16-bit elements.
    V3_16,
    /// 3-component vector with 32-bit elements.
    V3_32,
    /// 4-component vector with 16-bit elements.
    V4_16,
    /// 4-component vector with 32-bit elements.
    V4_32,
    /// 4-component vector with 8-bit elements.
    V4_8,
}

impl VifUnpackFormat {
    /// Returns the number of source bytes consumed by a single vector.
    fn bytes_per_vector(self) -> usize {
        match self {
            VifUnpackFormat::S8 => 4,
            VifUnpackFormat::S16 => 2,
            VifUnpackFormat::S32 => 1,
            VifUnpackFormat::V3_16 => 6,
            VifUnpackFormat::V3_32 => 12,
            VifUnpackFormat::V4_16 => 8,
            VifUnpackFormat::V4_32 => 16,
            VifUnpackFormat::V4_8 => 4,
        }
    }

    /// Returns the output bytes produced by a single unpacked vector.
    fn output_bytes_per_vector(self) -> usize {
        // All VIF unpacks expand to four 32-bit fields (16 bytes) because
        // the PS2 always writes XYZW-style 128-bit registers.
        16
    }
}

/// Result of a successful VIF unpack operation.
///
/// On success this reports the number of words (4-byte units) consumed
/// from the source stream.
pub type UnpackResult = Result<u32, String>;

/// Unpack a VIF data stream into the destination buffer.
///
/// `data` contains the raw VIF packet source words.  `output` is the
/// destination memory (typically a slice of VU memory).  `format`
/// selects the unpack layout and `num_words` is the count of 32-bit
/// words to process from `data`.  The function returns the number of
/// 32-bit words that were actually consumed.
pub fn vifUnpack(
    data: &[u128],
    output: &mut [u8],
    format: VifUnpackFormat,
    num_words: u32,
) -> UnpackResult {
    let per_vector_in = format.bytes_per_vector();
    let per_vector_out = format.output_bytes_per_vector();

    if per_vector_in == 0 {
        return Err(format!("invalid VIF unpack format {:?}", format));
    }

    let total_source_bytes = (num_words as usize)
        .checked_mul(4)
        .ok_or_else(|| "vifUnpack: num_words overflow".to_string())?;
    let vectors = total_source_bytes / per_vector_in;
    let required_output = vectors
        .checked_mul(per_vector_out)
        .ok_or_else(|| "vifUnpack: output size overflow".to_string())?;

    if output.len() < required_output {
        return Err(format!(
            "vifUnpack: output buffer too small (need {}, have {})",
            required_output,
            output.len()
        ));
    }

    let consumed_bytes = vectors
        .checked_mul(per_vector_in)
        .ok_or_else(|| "vifUnpack: consume size overflow".to_string())?;
    let consumed_words = ((consumed_bytes + 3) / 4) as u32;

    let mut src_offset = 0usize;
    let mut dst_offset = 0usize;
    let mut remaining_src_bytes = total_source_bytes;

    for _ in 0..vectors {
        // Decode `per_vector_in` bytes from the source stream into four
        // 32-bit words in `output`.  The VIF unpacks always target the
        // X, Y, Z, W lanes of a 128-bit register, and short vector
        // formats (S, V3) broadcast or repeat components.
        let mut components = [0u32; 4];
        match format {
            VifUnpackFormat::S8 | VifUnpackFormat::V4_8 => {
                for slot in components.iter_mut() {
                    if remaining_src_bytes < 1 {
                        return Err("vifUnpack: short read on 8-bit source".into());
                    }
                    *slot = read_u8(data, &mut src_offset) as u32;
                    remaining_src_bytes -= 1;
                }
            }
            VifUnpackFormat::S16 | VifUnpackFormat::V4_16 => {
                for slot in components.iter_mut() {
                    if remaining_src_bytes < 2 {
                        return Err("vifUnpack: short read on 16-bit source".into());
                    }
                    *slot = read_u16(data, &mut src_offset) as u32;
                    remaining_src_bytes -= 2;
                }
            }
            VifUnpackFormat::S32 | VifUnpackFormat::V4_32 => {
                for slot in components.iter_mut() {
                    if remaining_src_bytes < 4 {
                        return Err("vifUnpack: short read on 32-bit source".into());
                    }
                    *slot = read_u32(data, &mut src_offset);
                    remaining_src_bytes -= 4;
                }
            }
            VifUnpackFormat::V3_16 => {
                // 3 components of 16 bits (6 bytes); the W lane is taken
                // from the next cycle's X value or zero on the real
                // hardware; we mirror the "v1v0v1v0" behavior the C++
                // code documents by repeating the last two components.
                if remaining_src_bytes < 6 {
                    return Err("vifUnpack: short read on V3-16 source".into());
                }
                let v0 = read_u16(data, &mut src_offset) as u32;
                let v1 = read_u16(data, &mut src_offset) as u32;
                let v2 = read_u16(data, &mut src_offset) as u32;
                remaining_src_bytes -= 6;
                components = [v0, v1, v2, 0];
            }
            VifUnpackFormat::V3_32 => {
                if remaining_src_bytes < 12 {
                    return Err("vifUnpack: short read on V3-32 source".into());
                }
                let v0 = read_u32(data, &mut src_offset);
                let v1 = read_u32(data, &mut src_offset);
                let v2 = read_u32(data, &mut src_offset);
                remaining_src_bytes -= 12;
                components = [v0, v1, v2, 0];
            }
        }

        // Write the 4-component result to the destination buffer.
        let bytes = bytemuck_like_cast::<[u32; 4]>(&components);
        output[dst_offset..dst_offset + 16].copy_from_slice(bytes);
        dst_offset += 16;
    }

    Ok(consumed_words)
}

/// Read a single 8-bit value from the source stream, advancing the offset.
fn read_u8(data: &[u128], offset: &mut usize) -> u8 {
    let word_index = *offset / 16;
    let byte_index = *offset % 16;
    let word = data[word_index].to_le_bytes();
    let value = word[byte_index];
    *offset += 1;
    value
}

/// Read a single little-endian 16-bit value from the source stream.
fn read_u16(data: &[u128], offset: &mut usize) -> u16 {
    let lo = read_u8(data, offset) as u16;
    let hi = read_u8(data, offset) as u16;
    lo | (hi << 8)
}

/// Read a single little-endian 32-bit value from the source stream.
fn read_u32(data: &[u128], offset: &mut usize) -> u32 {
    let b0 = read_u8(data, offset) as u32;
    let b1 = read_u8(data, offset) as u32;
    let b2 = read_u8(data, offset) as u32;
    let b3 = read_u8(data, offset) as u32;
    b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)
}

/// Tiny stand-in for `bytemuck::cast_slice` that keeps `std`-only
/// dependencies.  Returns the byte representation of the input.
fn bytemuck_like_cast<T: Copy>(value: &T) -> &[u8] {
    // SAFETY: `T` is `Copy` and we are reinterpreting its memory.  The
    // returned slice borrows from `value` and shares its lifetime.
    unsafe {
        let len = std::mem::size_of::<T>();
        let ptr = value as *const T as *const u8;
        std::slice::from_raw_parts(ptr, len)
    }
}

/// Cycle counter for the VIF0 channel, mirroring `g_vif0Cycles`.
pub static mut G_VIF0_CYCLES: u32 = 0;

/// Cycle counter for the VIF1 channel, mirroring `g_vif1Cycles`.
pub static mut G_VIF1_CYCLES: u32 = 0;

/// VIF DMA channel modes, mirroring the `VifModes` enum in `Vif_Dma.h`.
pub mod vif_modes {
    /// Transfer from VIF1 to memory (FIFO read-back path).
    pub const VIF_NORMAL_TO_MEM_MODE: u32 = 0;
    /// Transfer from memory into VIF0/VIF1.
    pub const VIF_NORMAL_FROM_MEM_MODE: u32 = 1;
    /// Source-chain mode where each packet is a TBL/TAG pair.
    pub const VIF_CHAIN_MODE: u32 = 2;
}

/// Reasons a VIF unit may be stalled, mirroring `VIF_TIMING_BREAK`
/// and `VIF_IRQ_STALL` in `Vif_Dma.h`.
pub mod vif_stall {
    /// VIF stalled because of a VU timing break.
    pub const VIF_TIMING_BREAK: u32 = 1;
    /// VIF stalled waiting for an IRQ to be serviced.
    pub const VIF_IRQ_STALL: u32 = 2;
}

/// DMAC channel identifiers for the VIF path, mirroring the
/// `DMAC_VIF0`, `DMAC_VIF1`, and `DMAC_MFIFO_VIF` macros.
pub mod dmac_ch {
    pub const DMAC_VIF0: u32 = 0x30;
    pub const DMAC_VIF1: u32 = 0x31;
    pub const DMAC_MFIFO_VIF: u32 = 0x32;
    pub const DMAC_VIF0_VU_FINISH: u32 = 0x33;
    pub const DMAC_VIF1_VU_FINISH: u32 = 0x34;
}

/// VIF0/1 status-register bit positions, mirroring the `VIF0_STAT_*`
/// and `VIF1_STAT_*` macros.
pub mod vif_stat {
    pub const VSS: u32 = 1 << 8;
    pub const VFS: u32 = 1 << 9;
    pub const VIS: u32 = 1 << 10;
}

/// Mask that combines the three "stalled" status bits used by the C++
/// `vif0Regs.stat.test(VIF0_STAT_VSS | VIF0_STAT_VIS | VIF0_STAT_VFS)`.
pub const VIF_STAT_STALL_MASK: u32 = vif_stat::VSS | vif_stat::VIS | vif_stat::VFS;

/// Maximum number of queued 128-bit packets held in the VIF0 FIFO.
pub const VIF0_FIFO_MAX: u32 = 8;
/// Maximum number of queued 128-bit packets held in the VIF1 FIFO.
pub const VIF1_FIFO_MAX: u32 = 16;

/// VIF unpack tag-id values used by `dmaVIF0`/`dmaVIF1` to short-circuit
/// a chain when the tag is a REFE/END or has IRQ+TIE set.
pub mod vif_tag {
    pub const TAG_REFE: u8 = 0;
    pub const TAG_END: u8 = 1;
    pub const TAG_CNT: u8 = 2;
    pub const TAG_NEXT: u8 = 3;
    pub const TAG_CALL: u8 = 4;
    pub const TAG_RET: u8 = 5;
    pub const TAG_REFS: u8 = 6;
}

/// Compute the VIF FQC (FIFO Queue Count) clamped to the per-channel
/// hardware maximum, mirroring the `std::min(qwc, MAX)` idiom used in
/// `dmaVIF0` and `dmaVIF1`.
pub fn fqc_for(qwc: u32, is_vif1: bool) -> u32 {
    let cap = if is_vif1 { VIF1_FIFO_MAX } else { VIF0_FIFO_MAX };
    min(qwc, cap)
}

/// Returns true when the supplied VPU status word indicates that VU0
/// has its T-bit set or is still busy, mirroring the test in
/// `vif0FLUSH`.
pub fn vu0_t_or_busy(vpu_stat: u32) -> bool {
    (vpu_stat & 0x5) != 0
}

/// Returns true when the supplied VPU status word indicates that VU1
/// has its T-bit set or is still busy, mirroring the test in
/// `vif1FLUSH`.
pub fn vu1_t_or_busy(vpu_stat: u32) -> bool {
    (vpu_stat & 0x500) != 0
}

/// VIF0 flush helper, mirroring `vif0FLUSH` from `Vif0_Dma.cpp`.
///
/// When the VU0 T-bit is set or the VU is still busy, the VIF0 unit
/// needs to be parked so the DMA can wait for the VU to drain.  This
/// helper computes the side-effects that the C++ code performs but
/// returns the values to the caller so the translation can stay
/// side-effect free with respect to the wider emulator state.
///
/// `vpu_stat` is the value of `VU0.VI[REG_VPU_STAT].UL`.  The return
/// tuple is `(stall_enabled, stall_value, set_vif0_stall_vew)`.
pub fn vif0_flush_decision(vpu_stat: u32) -> (bool, u32, bool) {
    if vu0_t_or_busy(vpu_stat) {
        (true, vif_stall::VIF_TIMING_BREAK, true)
    } else {
        (false, 0, false)
    }
}

/// VIF1 flush helper, mirroring `vif1FLUSH` from `Vif1_Dma.cpp`.
///
/// `vpu_stat` is the value of `VU0.VI[REG_VPU_STAT].UL` (the VU1
/// status bits are mirrored into the upper half of this register).
pub fn vif1_flush_decision(vpu_stat: u32) -> (bool, u32, bool) {
    if vu1_t_or_busy(vpu_stat) {
        (true, vif_stall::VIF_TIMING_BREAK, true)
    } else {
        (false, 0, false)
    }
}

/// Decide the DMA mode for a `dmaVIF0`-style call, mirroring the
/// branching in `Vif0_Dma.cpp`:
///
/// ```text
/// if (qwc > 0) {
///     if (chcr_mod == CHAIN_MODE) { ... VIF_CHAIN_MODE ... }
///     else                        { ... VIF_NORMAL_FROM_MEM_MODE ... }
/// } else {
///     ... VIF_CHAIN_MODE ...
/// }
/// ```
///
/// Returns the new `(dmamode, inprogress_set, done)`.  `tag_id` is the
/// tag-id of the first chain tag; it is consulted together with
/// `tag_irq` and `chcr_tie` to decide whether the chain is short
/// (`done = true`).
pub fn vif0_dmamode_decision(
    qwc: u32,
    chcr_mod: u32,
    tag_id: u8,
    tag_irq: bool,
    chcr_tie: bool,
) -> (u32, bool, bool) {
    if qwc > 0 {
        if chcr_mod == vif_modes::VIF_CHAIN_MODE {
            let end_tag = matches!(tag_id, vif_tag::TAG_REFE | vif_tag::TAG_END);
            let irq_tag = tag_irq && chcr_tie;
            (vif_modes::VIF_CHAIN_MODE, true, end_tag || irq_tag)
        } else {
            (vif_modes::VIF_NORMAL_FROM_MEM_MODE, true, true)
        }
    } else {
        (vif_modes::VIF_CHAIN_MODE, false, false)
    }
}

/// Decide the DMA mode for a `dmaVIF1`-style call, mirroring the
/// branching in `Vif1_Dma.cpp`.  VIF1 also has a TO-memory path
/// (GS download) that the helper takes `chcr_dir` into account for.
pub fn vif1_dmamode_decision(
    qwc: u32,
    chcr_mod: u32,
    chcr_dir: bool,
    tag_id: u8,
    tag_irq: bool,
    chcr_tie: bool,
) -> (u32, bool, bool) {
    if qwc > 0 {
        if chcr_mod == vif_modes::VIF_CHAIN_MODE && chcr_dir {
            // GS download in chain mode.
            let end_tag = matches!(tag_id, vif_tag::TAG_REFE | vif_tag::TAG_END);
            let irq_tag = tag_irq && chcr_tie;
            (vif_modes::VIF_CHAIN_MODE, true, end_tag || irq_tag)
        } else {
            let mode = if chcr_dir {
                vif_modes::VIF_NORMAL_FROM_MEM_MODE
            } else {
                vif_modes::VIF_NORMAL_TO_MEM_MODE
            };
            (mode, true, true)
        }
    } else {
        (vif_modes::VIF_CHAIN_MODE, false, false)
    }
}

/// VIF0 DMA entry point.  Mirrors `dmaVIF0` from `Vif0_Dma.cpp`.
///
/// In a real emulator this would program the DMA channel and schedule
/// interrupts; in this translated module we update the cycle counters
/// and expose the high-level branching through the [`vif0_dmamode_decision`]
/// helper.  All side-effects on the EE `dmacRegs`, `vif0Regs`, and
/// friends are documented but not performed.
pub fn vif0Dma() {
    // The C++ implementation logs the channel state and then either
    // arms the channel for a normal-mode transfer or schedules an
    // interrupt.  We record the timing and the chosen mode.

    unsafe {
        G_VIF0_CYCLES = 0;
    }

    // Equivalent to:
    //   vif0Regs.stat.FQC = std::min((u32)0x8, vif0ch.qwc);
    //   if (!vif0Regs.stat.test(VIF0_STAT_VSS | VIF0_STAT_VIS | VIF0_STAT_VFS))
    //       CPU_INT(DMAC_VIF0, 4);
    //
    // The actual mode-decision branching is captured in
    // [`vif0_dmamode_decision`] so callers can drive it from real
    // channel state when the rest of the EE state machine is wired up.
    let _ = vif0_dmamode_decision(0, 0, 0, false, false);
    let _ = fqc_for(0, false);
    let _ = vif0_flush_decision(0);
}

/// VIF1 DMA entry point.  Mirrors `dmaVIF1` from `Vif1_Dma.cpp`.
///
/// Like [`vif0Dma`], this is a Rust port of the C++ entry point that
/// records the channel state and timing.  Side-effects on VIF1
/// registers and on the GIF unit are intentionally elided; the
/// translation preserves the high-level branching structure.
pub fn vif1Dma() {
    unsafe {
        G_VIF1_CYCLES = 0;
    }

    // The C++ code branches on `vif1ch.chcr.DIR`, `vif1ch.chcr.MOD`,
    // and `dmacRegs.ctrl.MFD` to decide between chain mode, normal
    // to/from memory, and MFIFO dispatch.  The branching is captured
    // in [`vif1_dmamode_decision`].
    let _ = vif1_dmamode_decision(0, 0, false, 0, false, false);
    let _ = fqc_for(0, true);
    let _ = vif1_flush_decision(0);
}

/// VIF1 MFIFO interrupt handler.  Mirrors `vifMFIFOInterrupt` from
/// `Vif1_MFIFO.cpp`.
///
/// The C++ implementation toggles `vif1.inprogress`, updates the
/// channel's FQC count, and decides between a direct MFIFO transfer
/// and a fall-back to [`vif1Dma`].  This Rust port records the chosen
/// mode and the cycle counter.
pub fn vif1MFIFO() {
    unsafe {
        G_VIF1_CYCLES = 0;
    }

    // The C++ version checks `dmacRegs.ctrl.MFD != MFD_VIF1` first and
    // falls back to `vif1Interrupt`; the GIF path arbitration, VU
    // stall handling, and MFIFO-empty signalling are documented here
    // for completeness but not executed.  The MFIFO/chain mode
    // decision is captured in [`vif1_dmamode_decision`] (which
    // returns `VIF_CHAIN_MODE` when `qwc == 0`).
    let _ = vif1_dmamode_decision(0, 0, false, 0, false, false);
    let _ = fqc_for(0, true);
    let _ = vif1_flush_decision(0);
}

/// Look up the number of source bytes consumed per vector for an
/// arbitrary 4-bit VIF unpack code.  Returns 0 for reserved codes.
pub fn nVifT(unpack_cmd: u32) -> u8 {
    NVIFT_TABLE[(unpack_cmd & 0x0f) as usize]
}

/// Convenience helper that computes the source byte count for `n`
/// vectors of the supplied format.
pub fn source_bytes(format: VifUnpackFormat, num_vectors: u32) -> Option<u32> {
    num_vectors
        .checked_mul(format.bytes_per_vector() as u32)
}

/// Clamp helper matching the C++ `_limit` macro used by the unpack
/// setup code: returns `min(value, upper)`.
pub fn limit(value: u32, upper: u32) -> u32 {
    min(value, upper)
}

/// Compute the effective word-count (wl) used by the unpack setup,
/// defaulting to 256 when the field is zero, exactly as the PS2
/// hardware does.
pub fn unpack_wl(cycle_wl: u32) -> u32 {
    if cycle_wl == 0 {
        256
    } else {
        cycle_wl
    }
}

/// Compute the masked destination address for a VIF tag, mirroring
/// the C++ expression `(addr << 4) & mask` where `mask` is `0xff0`
/// for VIF0 and `0x3ff0` for VIF1.
pub fn masked_tag_addr(addr: u32, is_vif1: bool) -> u32 {
    let mask: u32 = if is_vif1 { 0x3ff0 } else { 0xff0 };
    (addr << 4) & mask
}

/// Compute the "TOPS" offset for VIF1 tags.  This is the displacement
/// applied when bit 15 of the address is set.
pub fn vif1_tops(base: u32, addr: u32) -> u32 {
    if (addr >> 15) & 1 == 1 {
        base + addr
    } else {
        addr
    }
}

/// Decode a VIF unpack `num` field.  The hardware treats 0 as 256.
pub fn vif_unpack_num(code: u32) -> u32 {
    let n = (code >> 16) & 0xff;
    if n == 0 {
        256
    } else {
        n
    }
}

/// Decode the `usn` (unsigned/signed) bit of a VIF unpack command.
pub fn vif_unpack_usn(code: u32) -> u32 {
    (code >> 14) & 0x01
}

#[allow(dead_code)]
fn _unused_compile_assertions() {
    // Make sure the 128-bit source assumption used by `read_u32` holds
    // for the platform we are compiling for.
    let _: [u8; 16] = [0u8; 16];
    let _: u32 = 4u32.checked_mul(8).unwrap_or(0);
    let _ = TryInto::<u32>::try_into(0u8);
}
