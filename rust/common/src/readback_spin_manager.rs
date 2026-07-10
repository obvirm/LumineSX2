// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
// Pure-Rust port of `common/ReadbackSpinManager.h` / `common/ReadbackSpinManager.cpp`.
//
// The C++ class is *not* a generic spin-wait coordinator; it is a heuristic
// recommender that keeps the GPU busy while waiting for readbacks, without
// tripping OS power-down heuristics. It tracks submissions (draws and
// readbacks) across the last three rolling frames, picks the most similar
// historical frame as a "reference", and — once enough spin-cycle timing
// data has been observed — uses that reference frame to estimate how many
// GPU spin cycles to perform to fill the gap between two consecutive draws.
//
// The maths is intentionally ported verbatim; behaviour changes are limited
// to idiomatic Rust (no `std::vector`, no signed-into-unsigned tricks, no
// C-style lambda capture).
//
// 100% safe Rust. No FFI exports: the C++ side does not call into this
// module directly (GS-thread coordination is internal to the renderer).

/// Number of rolling frames the manager keeps history for. Matches the C++
/// `m_frames[3]` array.
const FRAME_COUNT: usize = 3;

/// Mask for the per-frame event index packed into the low 28 bits of an id.
/// Matches the C++ `(1 << 28) - 1` mask.
const ID_INDEX_MASK: u32 = (1u32 << 28) - 1;

/// Number of bits reserved for the frame index in the packed id.
const ID_FRAME_SHIFT: u32 = 28;

/// Decay factor used in the running-average estimate of spins-per-unit-time.
const SPIN_DECAY: f64 = 15.0 / 16.0;

/// One submission event recorded against a frame.
///
/// * `size >= 0`   — a draw submission; `size` is an arbitrary work metric
///                   (draw-call count, command-encoder count, ...).
/// * `size == -1`  — a readback submission (a negative size is the C++
///                   convention; here we model it as a separate variant for
///                   clarity while keeping the same wire layout).
#[derive(Clone, Copy, Debug, Default)]
struct Event {
    /// Signed because readbacks are encoded as `size = -1` to distinguish
    /// them from draws. `i64` matches the C++ `s64` field.
    size: i64,
    begin: u32,
    end: u32,
}

impl Event {
    fn is_readback(&self) -> bool {
        self.size < 0
    }

    fn is_completed(&self) -> bool {
        self.begin != self.end
    }
}

/// Return value of [`ReadbackSpinManager::draw_submitted`].
#[derive(Clone, Copy, Debug, Default)]
pub struct DrawSubmittedReturn {
    /// Identifier to pass back to [`ReadbackSpinManager::draw_completed`].
    pub id: u32,
    /// Recommended number of spin cycles for the GPU to perform.
    pub recommended_spin: u32,
}

/// A class for calculating optimal spin values to trick OSes into not
/// powering down GPUs while waiting for readbacks.
#[derive(Debug)]
pub struct ReadbackSpinManager {
    /// Rolling submission history for the last [`FRAME_COUNT`] frames.
    frames: [Vec<Event>; FRAME_COUNT],
    /// Currently-being-recorded frame.
    current_frame: u32,
    /// Frame selected as the most similar historical reference.
    reference_frame: u32,
    /// Index into `frames[reference_frame]` of the next readback to align to.
    reference_frame_idx: u32,
    /// Estimated spin cycles per unit of time on the GPU.
    spins_per_unit_time: f64,
    /// Running-average numerator (decayed).
    total_spin_cycles: f64,
    /// Running-average denominator (decayed).
    total_spin_time: f64,
}

impl Default for ReadbackSpinManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ReadbackSpinManager {
    pub fn new() -> Self {
        Self {
            frames: Default::default(),
            current_frame: 0,
            reference_frame: 0,
            reference_frame_idx: 0,
            spins_per_unit_time: 0.0,
            total_spin_cycles: 0.0,
            total_spin_time: 0.0,
        }
    }

    /// Call when a readback is requested.
    pub fn readback_requested(&mut self) {
        self.frames[self.current_frame as usize].push(Event {
            size: -1,
            ..Default::default()
        });

        // Advance reference index to the next readback in the reference
        // frame, then step past it. Re-read `reference_frame_idx` on each
        // iteration, mirroring the C++ which uses the field directly.
        let ref_frame_idx = self.reference_frame as usize;
        let ref_frame = &mut self.frames[ref_frame_idx];
        while (self.reference_frame_idx as usize) < ref_frame.len()
            && !ref_frame[self.reference_frame_idx as usize].is_readback()
        {
            self.reference_frame_idx += 1;
        }
        if (self.reference_frame_idx as usize) < ref_frame.len() {
            self.reference_frame_idx += 1;
        }
    }

    /// Call at the end of a frame.
    pub fn next_frame(&mut self) {
        let total = FRAME_COUNT;
        let prev0 = prev_frame_no(self.current_frame, total);
        let prev1 = prev_frame_no(prev0, total);

        let sim0 = similarity(&self.frames[self.current_frame as usize],
                              &self.frames[prev0 as usize]);
        let sim1 = similarity(&self.frames[self.current_frame as usize],
                              &self.frames[prev1 as usize]);

        self.reference_frame = if sim1 > sim0 { prev0 } else { self.current_frame };
        self.reference_frame_idx = 0;

        self.current_frame = next_frame_no(self.current_frame, total);
        self.frames[self.current_frame as usize].clear();
    }

    /// Call when a command buffer is submitted to the GPU.
    ///
    /// `size` approximates the amount of work in the submission. Returns an
    /// id to pass to [`draw_completed`](Self::draw_completed) and the
    /// recommended number of spin cycles to run on the GPU.
    pub fn draw_submitted(&mut self, size: u64) -> DrawSubmittedReturn {
        let mut out = DrawSubmittedReturn::default();

        let current = self.current_frame as usize;
        out.id = self.frames[current].len() as u32 | (self.current_frame << ID_FRAME_SHIFT);
        self.frames[current].push(Event {
            size: size as i64,
            ..Default::default()
        });

        // Try to recommend a spin count by aligning against the reference
        // frame's corresponding draw.
        if self.reference_frame != self.current_frame {
            let ref_idx = self.reference_frame_idx as usize;
            let ref_frame_len = self.frames[self.reference_frame as usize].len();
            if ref_frame_len > ref_idx
                && !self.frames[self.reference_frame as usize][ref_idx].is_readback()
            {
                let mut next_draw: Option<(usize, usize)> = None;

                // Search for the next draw after the reference index in the
                // reference frame; if none, search the following frame.
                let ref_frame = self.reference_frame as usize;
                if let Some(local) = self.frames[ref_frame]
                    .iter()
                    .skip(ref_idx + 1)
                    .position(|e| !e.is_readback())
                {
                    next_draw = Some((ref_frame, ref_idx + 1 + local));
                } else {
                    let next_f = next_frame_no(self.reference_frame, FRAME_COUNT) as usize;
                    if let Some(local) = self.frames[next_f]
                        .iter()
                        .position(|e| !e.is_readback())
                    {
                        next_draw = Some((next_f, local));
                    }
                }

                let is_one_frame_back =
                    self.reference_frame == prev_frame_no(self.current_frame, FRAME_COUNT);

                let cur_idx = ref_idx;
                let (cur_frame, next_frame_idx, next_frame_frame) = match next_draw {
                    Some((f, i)) => (ref_frame, i, f),
                    None => (ref_frame, 0, ref_frame),
                };

                let cur_completed = self.frames[cur_frame]
                    .get(cur_idx)
                    .map(|e| e.is_completed())
                    .unwrap_or(false);
                let next_completed = self.frames[next_frame_frame]
                    .get(next_frame_idx)
                    .map(|e| e.is_completed())
                    .unwrap_or(false);

                let (mut cur_frame, mut cur_idx) = (cur_frame, cur_idx);
                let (mut next_frame_idx, mut next_frame_frame) = (next_frame_idx, next_frame_frame);

                // Last frame's timing data hasn't arrived; try the same spot
                // in the frame before.
                if (next_draw.is_none() || !cur_completed || !next_completed) && is_one_frame_back {
                    let two_back = prev_frame_no(self.reference_frame, FRAME_COUNT) as usize;
                    if self.frames[two_back].len() > ref_idx
                        && !self.frames[two_back][ref_idx].is_readback()
                    {
                        cur_frame = two_back;
                        cur_idx = ref_idx;
                        if let Some(local) = self.frames[two_back]
                            .iter()
                            .skip(ref_idx + 1)
                            .position(|e| !e.is_readback())
                        {
                            next_frame_frame = two_back;
                            next_frame_idx = ref_idx + 1 + local;
                        } else {
                            let next_f = next_frame_no(two_back as u32, FRAME_COUNT) as usize;
                            if let Some(local) = self.frames[next_f]
                                .iter()
                                .position(|e| !e.is_readback())
                            {
                                next_frame_frame = next_f;
                                next_frame_idx = local;
                            } else {
                                next_frame_frame = two_back;
                                next_frame_idx = self.frames[two_back].len();
                            }
                        }
                    }
                }

                let cur = self.frames[cur_frame].get(cur_idx).copied();
                let nxt = self.frames[next_frame_frame].get(next_frame_idx).copied();
                let cur_completed = cur.map(|e| e.is_completed()).unwrap_or(false);
                let next_completed = nxt.map(|e| e.is_completed()).unwrap_or(false);

                if let (Some(cur), Some(nxt)) = (cur, nxt) {
                    if cur_completed && next_completed && self.spins_per_unit_time != 0.0 {
                        let cur_size = cur.size as u64;
                        let similar = cur_size / 2 <= size && size / 2 <= cur_size;
                        if similar {
                            let current_draw_time = cur.end.wrapping_sub(cur.begin) as i32;
                            let gap = nxt.begin.wrapping_sub(cur.end) as i32;
                            // Give an extra bit of space for the draw to take
                            // a bit longer (1/8 longer).
                            let fill = gap - (current_draw_time >> 3);
                            if fill > 0 {
                                out.recommended_spin =
                                    (fill as f64 * self.spins_per_unit_time) as u32;
                            }
                        }
                    }
                }

                self.reference_frame_idx += 1;
            }
        }

        // No timing data yet: recommend a small spin so we can collect some.
        if self.spins_per_unit_time == 0.0 {
            out.recommended_spin = 128;
        }

        out
    }

    /// Call once a draw has been finished by the GPU.
    ///
    /// `begin_time` and `end_time` can be in any consistent unit; rollover
    /// is tolerated as long as it happens less than once every few frames.
    pub fn draw_completed(&mut self, id: u32, begin_time: u32, end_time: u32) {
        let frame_id = id >> ID_FRAME_SHIFT;
        let frame_off = id & ID_INDEX_MASK;
        if (frame_id as usize) < FRAME_COUNT
            && (frame_off as usize) < self.frames[frame_id as usize].len()
        {
            let ev = &mut self.frames[frame_id as usize][frame_off as usize];
            ev.begin = begin_time;
            ev.end = end_time;
        }
    }

    /// Call when a spin completes to help the manager figure out how quickly
    /// the GPU spins.
    pub fn spin_completed(&mut self, cycles: u32, begin_time: u32, end_time: u32) {
        let elapsed = end_time.wrapping_sub(begin_time) as f64;

        // Empirically, a Radeon Pro 5600M and Intel UHD 630 both spin at
        // about 100ns/cycle. We assume spin time is a constant times the
        // number of cycles and use an exponential moving average to track
        // that constant.
        self.total_spin_cycles = self.total_spin_cycles * SPIN_DECAY + cycles as f64;
        self.total_spin_time = self.total_spin_time * SPIN_DECAY + elapsed;
        self.spins_per_unit_time = self.total_spin_cycles / self.total_spin_time;
    }

    /// Get the calculated number of spins per unit of time.
    ///
    /// May be zero when there is insufficient data.
    pub fn spins_per_unit_time(&self) -> f64 {
        self.spins_per_unit_time
    }
}

// ---------------------------------------------------------------------------
// Free helpers — ported verbatim from the C++ file-scope statics.
// ---------------------------------------------------------------------------

/// Score how similar two frames are. Higher = more similar.
///
/// Readbacks count as the structural anchor: matching readback counts and
/// positions are heavily weighted, draw sizes within a factor of two are
/// loosely weighted, and exact matches are weighted more.
///
/// Both slices are taken by shared reference; the C++ version took `b` by
/// mutable reference but never actually mutated it, so this is a faithful
/// (and slightly relaxed) translation.
fn similarity(a: &[Event], b: &[Event]) -> i32 {
    let a_readbacks = a.iter().filter(|e| e.is_readback()).count() as u32;
    let b_readbacks = b.iter().filter(|e| e.is_readback()).count() as u32;

    let mut score: i32 = 0x10 - (a.len() as i32 - b.len() as i32).abs();

    if a_readbacks == b_readbacks {
        score += 0x10000;
    }

    let mut a_idx = 0usize;
    let mut b_idx = 0usize;
    while a_idx < a.len() && b_idx < b.len() {
        let a_ev = &a[a_idx];
        let b_ev = &b[b_idx];
        if a_ev.is_readback() && b_ev.is_readback() {
            // Same number of events between readbacks.
            score += 0x1000;
        } else if a_ev.is_readback() {
            b_idx += 1;
            continue;
        } else if b_ev.is_readback() {
            a_idx += 1;
            continue;
        } else if a_ev.size == b_ev.size {
            score += 0x100;
        } else {
            // "Similar size" if each is within 2x of the other. The C++
            // does integer division on `s64` values; on the wire the field
            // is signed, but draw sizes are always non-negative, so direct
            // casting reproduces the same arithmetic.
            let a_size = a_ev.size as u64;
            let b_size = b_ev.size as u64;
            if a_size / 2 <= b_size && b_size / 2 <= a_size {
                score += 0x10;
            }
        }
        a_idx += 1;
        b_idx += 1;
    }

    // Both iterators hit the end together.
    if a_idx == a.len() && b_idx == b.len() {
        score += 0x1000;
    }

    score
}

fn prev_frame_no(frame: u32, total: usize) -> u32 {
    let prev = frame as i32 - 1;
    if prev < 0 {
        (total - 1) as u32
    } else {
        prev as u32
    }
}

fn next_frame_no(frame: u32, total: usize) -> u32 {
    let next = frame + 1;
    if next as usize >= total {
        0
    } else {
        next
    }
}

// ---------------------------------------------------------------------------
// Tests — exercise the core heuristics against a synthetic workload so the
// logic is regression-safe.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_starts_empty() {
        let m = ReadbackSpinManager::new();
        assert_eq!(m.spins_per_unit_time(), 0.0);
    }

    #[test]
    fn spin_completed_yields_nonzero_rate() {
        let mut m = ReadbackSpinManager::new();
        m.spin_completed(1000, 0, 1000); // 1 cycle / unit time
        assert!(m.spins_per_unit_time() > 0.0);
    }

    #[test]
    fn draw_completed_ignores_unknown_ids() {
        let mut m = ReadbackSpinManager::new();
        // Should not panic on bogus frame indices or out-of-range offsets.
        m.draw_completed(0xFFFF_FFFF, 0, 1);
        m.draw_completed(0, 99, 0);
    }

    #[test]
    fn rolling_frames_advance() {
        let mut m = ReadbackSpinManager::new();
        m.draw_submitted(10);
        m.next_frame();
        m.draw_submitted(20);
        m.next_frame();
        m.draw_submitted(30);
        // After three submissions across three frames, all three buckets
        // should have exactly one draw event each.
        for f in 0..FRAME_COUNT {
            assert_eq!(m.frames[f].len(), 1, "frame {f} should have one event");
        }
    }

    #[test]
    fn readback_requested_advances_reference_index() {
        let mut m = ReadbackSpinManager::new();
        m.draw_submitted(10);
        m.draw_submitted(20);
        m.readback_requested();
        // After advancing past the readback in the reference frame, the
        // reference index should sit beyond the (non-existent) next readback.
        assert!(m.reference_frame_idx >= 1);
    }

    #[test]
    fn similarity_prefers_matching_readback_counts() {
        let a = vec![
            Event { size: 1, ..Default::default() },
            Event { size: -1, ..Default::default() },
            Event { size: 2, ..Default::default() },
        ];
        let b = a.clone();
        // Identical frames should score higher than a frame with the wrong
        // number of readbacks.
        let c = vec![Event { size: 1, ..Default::default() }];
        assert!(similarity(&a, &b) > similarity(&a, &c));
    }

    #[test]
    fn next_and_prev_frame_wrap() {
        assert_eq!(next_frame_no(2, 3), 0);
        assert_eq!(next_frame_no(0, 3), 1);
        assert_eq!(prev_frame_no(0, 3), 2);
        assert_eq!(prev_frame_no(1, 3), 0);
    }
}