//! The in-app "Run Test" harness (functional_spec §6.E, architecture §7).
//!
//! The harness owns a **scripted sequence of viewport positions** — scroll down,
//! fast-scroll, horizontal pan, jump-to-cell, random-jump — and hands them to the
//! rendering shell one frame at a time. The shell applies each viewport, times its own
//! render and its newly-visible-cell provider pulls, and reports a [`FrameSample`] back.
//! When the script is exhausted the shell finalizes the run (see [`crate::report`]).
//!
//! Everything here is **deterministic and gpui-free**: the script is a pure function of
//! the [`PocConfig`] and a fixed seed, so the same run is reproducible and the harness
//! logic is unit-testable in the headless container. Timing is done by the shell (it
//! owns the real render), never here — this module only sequences viewports and records
//! the numbers the shell measures.

use std::ops::Range;

use crate::config::PocConfig;

/// A viewport position: the top-left scroll offset (px) of the content area.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub scroll_x: f64,
    pub scroll_y: f64,
}

impl Viewport {
    pub const ORIGIN: Viewport = Viewport {
        scroll_x: 0.0,
        scroll_y: 0.0,
    };
}

/// One measured frame, filled in by the shell after it applies a scripted viewport.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameSample {
    /// Wall-clock nanoseconds the shell spent producing this frame's render.
    pub frame_render_ns: u64,
    /// Wall-clock nanoseconds the shell spent pulling `CellData` for the cells that
    /// newly entered the viewport this frame.
    pub cell_load_ns: u64,
    /// How many cells newly entered the viewport this frame (for context in results).
    pub newly_visible: u32,
}

/// The named phases of the canonical script, so results can attribute frames to a move
/// type if desired.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Move {
    ScrollDown,
    FastScroll,
    Horizontal,
    JumpToCell,
    RandomJump,
}

/// The scripted "Run Test" harness. Advances a fixed viewport sequence frame-by-frame
/// and accumulates the shell's per-frame samples.
#[derive(Debug, Clone)]
pub struct Harness {
    steps: Vec<(Move, Viewport)>,
    cursor: usize,
    samples: Vec<FrameSample>,
}

impl Harness {
    /// Builds the canonical scripted run for `cfg`: a mix of every move type, sized so
    /// each phase produces enough frames for a stable p50/p99 while staying quick to run
    /// on the Mac (a few hundred frames total).
    pub fn scripted(cfg: &PocConfig) -> Self {
        let mut steps = Vec::new();
        let total_h = axis_total_estimate(cfg.rows, avg_row_height(cfg));
        let total_w = axis_total_estimate(cfg.cols, avg_col_width(cfg));
        let vh = cfg.content_height() as f64;
        let vw = cfg.content_width() as f64;

        // 1) Steady scroll down: 120 frames of one-viewport-per-second-ish stepping.
        push_linear(&mut steps, Move::ScrollDown, 120, |t| Viewport {
            scroll_x: 0.0,
            scroll_y: t * vh * 0.35, // ~third of a viewport per frame
        });

        // 2) Fast scroll down: 60 frames of multi-viewport jumps (stresses overscan).
        let fast_base = 120.0 * vh * 0.35;
        push_linear(&mut steps, Move::FastScroll, 60, |t| Viewport {
            scroll_x: 0.0,
            scroll_y: (fast_base + t * vh * 3.0).min(total_h - vh).max(0.0),
        });

        // 3) Horizontal pan across the (very wide, variable-width) columns.
        push_linear(&mut steps, Move::Horizontal, 80, |t| Viewport {
            scroll_x: (t * vw * 0.5).min((total_w - vw).max(0.0)),
            scroll_y: fast_base,
        });

        // 4) Deterministic jump-to-cell: land on a spread of far-flung cells.
        let jump_targets = deterministic_jumps(cfg, 24);
        for vp in jump_targets {
            steps.push((Move::JumpToCell, vp));
        }

        // 5) Seeded random jumps across the full grid.
        let random_targets = seeded_random_jumps(cfg, 64, total_w, total_h, vw, vh);
        for vp in random_targets {
            steps.push((Move::RandomJump, vp));
        }

        let frame_count = steps.len();
        Self {
            steps,
            cursor: 0,
            samples: Vec::with_capacity(frame_count),
        }
    }

    /// The total number of scripted frames.
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// The move type of the frame about to be produced, if any.
    pub fn current_move(&self) -> Option<Move> {
        self.steps.get(self.cursor).map(|(m, _)| *m)
    }

    /// Advances to the next scripted viewport, or `None` when the script is exhausted.
    /// The shell should apply the returned viewport, render, measure, and call
    /// [`Harness::record`].
    pub fn next_viewport(&mut self) -> Option<Viewport> {
        let vp = self.steps.get(self.cursor).map(|(_, vp)| *vp);
        if vp.is_some() {
            self.cursor += 1;
        }
        vp
    }

    /// Records the shell's measurement for the frame just rendered.
    pub fn record(&mut self, sample: FrameSample) {
        self.samples.push(sample);
    }

    /// All recorded samples so far.
    pub fn samples(&self) -> &[FrameSample] {
        &self.samples
    }
}

/// Cells that newly enter the viewport when the visible range moves from `prev` to
/// `cur` on one axis — the count the shell must *load* this frame on that axis. For the
/// 2D grid, a shell multiplies row-newly-visible × current-col-span (and vice-versa)
/// or, more simply, times the pulls it actually performs; this helper gives the axis
/// delta the harness records for context.
pub fn newly_visible(prev: &Range<u32>, cur: &Range<u32>) -> u32 {
    if prev.start >= prev.end {
        return cur.end.saturating_sub(cur.start);
    }
    let overlap_start = cur.start.max(prev.start);
    let overlap_end = cur.end.min(prev.end);
    let overlap = overlap_end.saturating_sub(overlap_start.min(overlap_end));
    let cur_len = cur.end.saturating_sub(cur.start);
    cur_len.saturating_sub(overlap)
}

// --- script construction helpers -------------------------------------------------

fn push_linear(
    steps: &mut Vec<(Move, Viewport)>,
    mv: Move,
    frames: u32,
    f: impl Fn(f64) -> Viewport,
) {
    for i in 0..frames {
        steps.push((mv, f(i as f64)));
    }
}

/// The total extent of an axis is not known here (that lives in [`crate::layout::Axis`],
/// which the shell owns), so the script uses a cheap average-size estimate to place
/// scroll targets. This only affects *where* we scroll, not correctness of measurement.
fn axis_total_estimate(count: u32, avg: f64) -> f64 {
    count as f64 * avg
}

fn avg_row_height(_cfg: &PocConfig) -> f64 {
    // Matches datagen's SyntheticSheet::row_height distribution centre.
    24.0
}

fn avg_col_width(_cfg: &PocConfig) -> f64 {
    // Matches datagen's SyntheticSheet::col_width distribution centre (incl. wide cols).
    110.0
}

/// A deterministic spread of jump targets across the grid corners and interior.
fn deterministic_jumps(cfg: &PocConfig, n: u32) -> Vec<Viewport> {
    let total_h = axis_total_estimate(cfg.rows, avg_row_height(cfg));
    let total_w = axis_total_estimate(cfg.cols, avg_col_width(cfg));
    let vh = cfg.content_height() as f64;
    let vw = cfg.content_width() as f64;
    (0..n)
        .map(|i| {
            let fx = (i as f64) / (n as f64);
            // Bounce across the grid: alternate near-far on each axis.
            let x = if i % 2 == 0 { fx } else { 1.0 - fx } * (total_w - vw).max(0.0);
            let y = if i % 3 == 0 { 1.0 - fx } else { fx } * (total_h - vh).max(0.0);
            Viewport {
                scroll_x: x.max(0.0),
                scroll_y: y.max(0.0),
            }
        })
        .collect()
}

/// Seeded pseudo-random jumps (splitmix64-style), so the sequence is reproducible.
fn seeded_random_jumps(
    cfg: &PocConfig,
    n: u32,
    total_w: f64,
    total_h: f64,
    vw: f64,
    vh: f64,
) -> Vec<Viewport> {
    let mut state = cfg.seed ^ 0xA5A5_1234_DEAD_BEEF;
    let max_x = (total_w - vw).max(0.0);
    let max_y = (total_h - vh).max(0.0);
    (0..n)
        .map(|_| {
            let rx = next_unit(&mut state);
            let ry = next_unit(&mut state);
            Viewport {
                scroll_x: rx * max_x,
                scroll_y: ry * max_y,
            }
        })
        .collect()
}

/// A splitmix64 step returning a `[0, 1)` float. Deterministic; no external RNG.
fn next_unit(state: &mut u64) -> f64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    // Top 53 bits → [0, 1).
    (z >> 11) as f64 / (1u64 << 53) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_is_deterministic_and_covers_all_moves() {
        let cfg = PocConfig::default();
        let a = Harness::scripted(&cfg);
        let b = Harness::scripted(&cfg);
        assert_eq!(a.steps, b.steps, "same config must yield same script");
        assert!(!a.is_empty());

        let moves: std::collections::HashSet<Move> = a.steps.iter().map(|(m, _)| *m).collect();
        for expected in [
            Move::ScrollDown,
            Move::FastScroll,
            Move::Horizontal,
            Move::JumpToCell,
            Move::RandomJump,
        ] {
            assert!(moves.contains(&expected), "script missing move {expected:?}");
        }
    }

    #[test]
    fn harness_advances_and_terminates() {
        let cfg = PocConfig::default();
        let mut h = Harness::scripted(&cfg);
        let total = h.len();
        let mut count = 0;
        while let Some(_vp) = h.next_viewport() {
            h.record(FrameSample {
                frame_render_ns: 1_000_000,
                cell_load_ns: 100_000,
                newly_visible: 10,
            });
            count += 1;
            assert!(count <= total + 1, "harness must terminate");
        }
        assert_eq!(count, total);
        assert!(h.next_viewport().is_none(), "exhausted harness yields None");
        assert_eq!(h.samples().len(), total);
    }

    #[test]
    fn viewports_stay_non_negative() {
        let cfg = PocConfig::default();
        let mut h = Harness::scripted(&cfg);
        while let Some(vp) = h.next_viewport() {
            assert!(vp.scroll_x >= 0.0 && vp.scroll_y >= 0.0, "viewport went negative: {vp:?}");
        }
    }

    #[test]
    fn newly_visible_set_difference() {
        // Overlapping ranges: new cells are only the non-overlapping tail.
        assert_eq!(newly_visible(&(0..10), &(3..13)), 3);
        assert_eq!(newly_visible(&(5..15), &(0..10)), 5);
        // Fully disjoint: the whole new range is newly visible.
        assert_eq!(newly_visible(&(0..10), &(100..120)), 20);
        // First frame (empty prev): everything is new.
        assert_eq!(newly_visible(&(0..0), &(4..9)), 5);
        // Identical range: nothing new.
        assert_eq!(newly_visible(&(2..8), &(2..8)), 0);
    }

    #[test]
    fn seeded_random_jumps_are_reproducible() {
        let cfg = PocConfig::default();
        let a = seeded_random_jumps(&cfg, 16, 1e6, 1e6, 1440.0, 900.0);
        let b = seeded_random_jumps(&cfg, 16, 1e6, 1e6, 1440.0, 900.0);
        assert_eq!(a, b);
    }
}
