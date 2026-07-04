//! Perf-harness core — the scripted "Run Test" sequence, latency stats, and gates
//! (`architecture.md §4, §9`; ported from `experiments/04-ui-poc/poc-core`).
//!
//! Engine-free and gpui-free: the script is a pure function of a [`PerfConfig`] + a fixed
//! seed, so a run is reproducible and every piece here is unit-testable in the headless
//! container. The gpui shell (the real [`GridView`](../../freecell_app) + engine-backed
//! sources) owns the actual measurement — it applies each scripted [`Viewport`], times its
//! own build path, and hands back a [`FrameSample`]. This module only sequences viewports
//! and reduces the recorded samples into p50/p99/max + PASS/FAIL gates.
//!
//! JSON recording is deliberately NOT here: `freecell-core` stays dependency-free (no
//! serde). The perf binary (`render-tests`) stamps the environment and writes the report.

use std::ops::Range;

// --- §4 true budgets (real-hardware product truth) ----------------------------------

/// Sustained-120 fps frame budget: `1_000_000_000 / 120 ≈ 8_333_333` ns
/// (`architecture.md §4`).
pub const FRAME_TARGET_NS: u64 = 8_333_333;

/// Worst-case (never worse than 60 fps under fast scroll / jump) frame budget:
/// `1_000_000_000 / 60 ≈ 16_666_667` ns (`architecture.md §4`).
pub const FRAME_WORST_NS: u64 = 16_666_667;

/// Newly-visible-cell load budget: resolving styles + values for the cells entering the
/// viewport must fit well inside a frame — `< 2 ms` (`architecture.md §4`).
pub const CELL_LOAD_TARGET_NS: u64 = 2_000_000;

// --- Configuration ------------------------------------------------------------------

/// Configuration for a perf run: grid dimensions, viewport, header sizes, and seed. The
/// defaults target the **1M×100 styled fixture** (`architecture.md §4`, `components/grid.md`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PerfConfig {
    /// Logical rows in the grid (default: Excel-max 1,048,576).
    pub rows: u32,
    /// Logical columns of interest (default: 100 — the styled fixture's populated band).
    pub cols: u32,
    /// Deterministic seed for the scripted random jumps.
    pub seed: u64,
    /// Viewport width in logical px (whole grid area, incl. the row-header gutter).
    pub viewport_width: f32,
    /// Viewport height in logical px (incl. the column-header strip).
    pub viewport_height: f32,
    /// Left row-header gutter width (px) — used only to place scroll targets.
    pub row_header_width: f32,
    /// Top column-header strip height (px) — used only to place scroll targets.
    pub col_header_height: f32,
    /// Average row height (px) — the script's cheap total-extent estimate (real clamping
    /// happens against the grid's real axes in the shell).
    pub avg_row_height: f32,
    /// Average column width (px) — as above.
    pub avg_col_width: f32,
}

impl Default for PerfConfig {
    fn default() -> Self {
        Self {
            rows: 1_048_576,
            cols: 100,
            seed: 0xF9EE_C011,
            viewport_width: 1440.0,
            viewport_height: 900.0,
            row_header_width: 56.0,
            col_header_height: 24.0,
            avg_row_height: 24.0,
            avg_col_width: 120.0,
        }
    }
}

impl PerfConfig {
    /// The content viewport height available for data rows (viewport minus the col-header strip).
    pub fn content_height(&self) -> f32 {
        (self.viewport_height - self.col_header_height).max(0.0)
    }

    /// The content viewport width available for data columns (viewport minus the row gutter).
    pub fn content_width(&self) -> f32 {
        (self.viewport_width - self.row_header_width).max(0.0)
    }
}

// --- Script -------------------------------------------------------------------------

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

/// The named phases of the canonical script, so results can attribute frames to a move type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Move {
    ScrollDown,
    FastScroll,
    Horizontal,
    JumpToCell,
    RandomJump,
}

/// One measured frame, filled in by the shell after it applies a scripted viewport.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameSample {
    /// Wall-clock ns the shell spent building this frame (data resolution + element
    /// construction — the POC's `frame_render` metric; excludes gpui layout/shape/present).
    pub frame_render_ns: u64,
    /// Wall-clock ns spent resolving styles + published values for the cells visible this
    /// frame (the newly-visible-cell "load" analog).
    pub cell_load_ns: u64,
    /// How many cells newly entered the viewport this frame (context in results).
    pub newly_visible: u32,
    /// How many render elements the frame produced — a FORCE + ASSERT witness that the
    /// build measured real work, not a no-op (`CLAUDE.md` benchmark convention).
    pub elements: u32,
}

/// The scripted "Run Test" harness. Advances a fixed viewport sequence frame-by-frame and
/// accumulates the shell's per-frame samples.
#[derive(Debug, Clone)]
pub struct Harness {
    steps: Vec<(Move, Viewport)>,
    cursor: usize,
    samples: Vec<FrameSample>,
}

impl Harness {
    /// Builds the canonical scripted run for `cfg`: steady scroll, fast scroll, horizontal
    /// pan, deterministic far jumps, and seeded random jumps — 348 frames, enough for a
    /// stable p50/p99.
    pub fn scripted(cfg: &PerfConfig) -> Self {
        let mut steps = Vec::new();
        let total_h = cfg.rows as f64 * cfg.avg_row_height as f64;
        let total_w = cfg.cols as f64 * cfg.avg_col_width as f64;
        let vh = cfg.content_height() as f64;
        let vw = cfg.content_width() as f64;

        // 1) Steady scroll down: ~a third of a viewport per frame.
        push_linear(&mut steps, Move::ScrollDown, 120, |t| Viewport {
            scroll_x: 0.0,
            scroll_y: t * vh * 0.35,
        });

        // 2) Fast scroll down: multi-viewport jumps (stresses the value-band boundary).
        let fast_base = 120.0 * vh * 0.35;
        push_linear(&mut steps, Move::FastScroll, 60, |t| Viewport {
            scroll_x: 0.0,
            scroll_y: (fast_base + t * vh * 3.0).min(total_h - vh).max(0.0),
        });

        // 3) Horizontal pan across the wide, variable-width columns.
        push_linear(&mut steps, Move::Horizontal, 80, |t| Viewport {
            scroll_x: (t * vw * 0.5).min((total_w - vw).max(0.0)),
            scroll_y: fast_base,
        });

        // 4) Deterministic far jumps across the grid corners + interior.
        for vp in deterministic_jumps(cfg, 24, total_w, total_h, vw, vh) {
            steps.push((Move::JumpToCell, vp));
        }

        // 5) Seeded random jumps across the full grid.
        for vp in seeded_random_jumps(cfg, 64, total_w, total_h, vw, vh) {
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

/// The number of cells that newly enter the viewport when the visible region moves from
/// `prev_*` to `cur_*` — the count the shell "loads" this frame. On the first frame (empty
/// prev) the whole current region is new.
pub fn newly_visible_2d(
    prev_rows: &Range<u32>,
    prev_cols: &Range<u32>,
    cur_rows: &Range<u32>,
    cur_cols: &Range<u32>,
) -> u32 {
    let cur = span_len(cur_rows) * span_len(cur_cols);
    if prev_rows.start >= prev_rows.end || prev_cols.start >= prev_cols.end {
        return cur;
    }
    let overlap = overlap_len(prev_rows, cur_rows) * overlap_len(prev_cols, cur_cols);
    cur.saturating_sub(overlap)
}

fn span_len(r: &Range<u32>) -> u32 {
    r.end.saturating_sub(r.start)
}

fn overlap_len(a: &Range<u32>, b: &Range<u32>) -> u32 {
    let start = a.start.max(b.start);
    let end = a.end.min(b.end);
    end.saturating_sub(start.min(end))
}

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

fn deterministic_jumps(
    cfg: &PerfConfig,
    n: u32,
    total_w: f64,
    total_h: f64,
    vw: f64,
    vh: f64,
) -> Vec<Viewport> {
    let _ = cfg;
    (0..n)
        .map(|i| {
            let fx = (i as f64) / (n as f64);
            let x = if i % 2 == 0 { fx } else { 1.0 - fx } * (total_w - vw).max(0.0);
            let y = if i % 3 == 0 { 1.0 - fx } else { fx } * (total_h - vh).max(0.0);
            Viewport {
                scroll_x: x.max(0.0),
                scroll_y: y.max(0.0),
            }
        })
        .collect()
}

fn seeded_random_jumps(
    cfg: &PerfConfig,
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
        .map(|_| Viewport {
            scroll_x: next_unit(&mut state) * max_x,
            scroll_y: next_unit(&mut state) * max_y,
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
    (z >> 11) as f64 / (1u64 << 53) as f64
}

// --- Stats + gates ------------------------------------------------------------------

/// A summary of a latency sample set, all in nanoseconds (nearest-rank percentiles,
/// matching the frozen `bench_util` convention so numbers stay comparable to the POC).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LatencyStats {
    pub count: u64,
    pub min_ns: u64,
    pub max_ns: u64,
    pub mean_ns: u64,
    pub p50_ns: u64,
    pub p99_ns: u64,
}

impl LatencyStats {
    /// Stats over `samples` (any order). Returns an all-zero summary for an empty input —
    /// the shell always runs the full script, so an empty set only appears in tests.
    pub fn from_samples(samples: &[u64]) -> Self {
        if samples.is_empty() {
            return Self {
                count: 0,
                min_ns: 0,
                max_ns: 0,
                mean_ns: 0,
                p50_ns: 0,
                p99_ns: 0,
            };
        }
        let mut sorted = samples.to_vec();
        sorted.sort_unstable();
        let count = sorted.len() as u64;
        let sum: u128 = sorted.iter().map(|&v| v as u128).sum();
        Self {
            count,
            min_ns: sorted[0],
            max_ns: *sorted.last().unwrap(),
            mean_ns: (sum / count as u128) as u64,
            p50_ns: percentile_ns(&sorted, 50.0),
            p99_ns: percentile_ns(&sorted, 99.0),
        }
    }
}

/// The nearest-rank percentile (ns) of a **sorted-ascending** slice. `0` for empty.
pub fn percentile_ns(sorted_ascending: &[u64], pct: f64) -> u64 {
    if sorted_ascending.is_empty() {
        return 0;
    }
    let pct = pct.clamp(0.0, 100.0);
    let n = sorted_ascending.len();
    if pct <= 0.0 {
        return sorted_ascending[0];
    }
    let rank = (pct / 100.0 * n as f64).ceil() as usize;
    let idx = rank.saturating_sub(1).min(n - 1);
    sorted_ascending[idx]
}

/// A single gated measurement: a "must not exceed" latency check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gate {
    pub name: String,
    pub measured_ns: u64,
    pub target_ns: u64,
    pub pass: bool,
}

impl Gate {
    /// Gates `measured_ns <= target_ns`.
    pub fn max(name: impl Into<String>, measured_ns: u64, target_ns: u64) -> Self {
        Self {
            name: name.into(),
            measured_ns,
            target_ns,
            pass: measured_ns <= target_ns,
        }
    }

    /// A printable one-line summary (`PASS frame-p99: measured … <= target …`).
    pub fn summary(&self) -> String {
        format!(
            "{} {}: measured {} {} target {}",
            if self.pass { "PASS" } else { "FAIL" },
            self.name,
            fmt_ns(self.measured_ns),
            if self.pass { "<=" } else { ">" },
            fmt_ns(self.target_ns),
        )
    }
}

/// Formats a nanosecond count with a human-friendly unit (ns / µs / ms / s).
pub fn fmt_ns(ns: u64) -> String {
    const US: u64 = 1_000;
    const MS: u64 = 1_000_000;
    const S: u64 = 1_000_000_000;
    if ns < US {
        format!("{ns} ns")
    } else if ns < MS {
        format!("{:.2} µs", ns as f64 / US as f64)
    } else if ns < S {
        format!("{:.2} ms", ns as f64 / MS as f64)
    } else {
        format!("{:.2} s", ns as f64 / S as f64)
    }
}

/// The reduced outcome of a run: the two latency distributions + the gates evaluated
/// against a supplied set of targets. The binary stamps the environment + writes JSON.
#[derive(Debug, Clone)]
pub struct RunReport {
    pub frame_stats: LatencyStats,
    pub cell_load_stats: LatencyStats,
    pub gates: Vec<Gate>,
}

impl RunReport {
    /// Reduces `samples` and gates them against `(frame_p99, frame_max, cell_load_p99)`
    /// targets. Pass the §4 true budgets for the product-truth verdict, or the committed
    /// buffered CI thresholds for the hard gate.
    pub fn build(
        samples: &[FrameSample],
        frame_p99: u64,
        frame_max: u64,
        cell_load_p99: u64,
    ) -> Self {
        let frame_ns: Vec<u64> = samples.iter().map(|s| s.frame_render_ns).collect();
        let cell_ns: Vec<u64> = samples.iter().map(|s| s.cell_load_ns).collect();
        let frame_stats = LatencyStats::from_samples(&frame_ns);
        let cell_load_stats = LatencyStats::from_samples(&cell_ns);
        let gates = vec![
            Gate::max("frame-p99", frame_stats.p99_ns, frame_p99),
            Gate::max("frame-max", frame_stats.max_ns, frame_max),
            Gate::max("cell-load-p99", cell_load_stats.p99_ns, cell_load_p99),
        ];
        Self {
            frame_stats,
            cell_load_stats,
            gates,
        }
    }

    /// `true` iff every gate passed.
    pub fn passed(&self) -> bool {
        self.gates.iter().all(|g| g.pass)
    }

    /// A multi-line human summary.
    pub fn summary(&self, label: &str) -> String {
        let mut out = String::new();
        out.push_str(&format!("=== Run Test: {label} ===\n"));
        out.push_str(&format!("frames measured: {}\n", self.frame_stats.count));
        out.push_str(&format!(
            "frame render  p50={} p99={} max={}\n",
            fmt_ns(self.frame_stats.p50_ns),
            fmt_ns(self.frame_stats.p99_ns),
            fmt_ns(self.frame_stats.max_ns),
        ));
        out.push_str(&format!(
            "cell load     p50={} p99={} max={}\n",
            fmt_ns(self.cell_load_stats.p50_ns),
            fmt_ns(self.cell_load_stats.p99_ns),
            fmt_ns(self.cell_load_stats.max_ns),
        ));
        for g in &self.gates {
            out.push_str(&g.summary());
            out.push('\n');
        }
        out.push_str(&format!(
            "VERDICT: {}\n",
            if self.passed() { "PASS" } else { "FAIL" }
        ));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_is_deterministic_and_covers_all_moves() {
        let cfg = PerfConfig::default();
        let a = Harness::scripted(&cfg);
        let b = Harness::scripted(&cfg);
        assert_eq!(a.steps, b.steps, "same config must yield same script");
        assert_eq!(a.len(), 348, "120 + 60 + 80 + 24 + 64 frames");
        let moves: std::collections::HashSet<Move> = a.steps.iter().map(|(m, _)| *m).collect();
        for expected in [
            Move::ScrollDown,
            Move::FastScroll,
            Move::Horizontal,
            Move::JumpToCell,
            Move::RandomJump,
        ] {
            assert!(moves.contains(&expected), "script missing {expected:?}");
        }
    }

    #[test]
    fn harness_advances_and_terminates() {
        let cfg = PerfConfig::default();
        let mut h = Harness::scripted(&cfg);
        let total = h.len();
        let mut count = 0;
        while h.next_viewport().is_some() {
            h.record(FrameSample {
                frame_render_ns: 1_000_000,
                cell_load_ns: 100_000,
                newly_visible: 10,
                elements: 500,
            });
            count += 1;
            assert!(count <= total + 1, "harness must terminate");
        }
        assert_eq!(count, total);
        assert!(h.next_viewport().is_none());
        assert_eq!(h.samples().len(), total);
    }

    #[test]
    fn viewports_stay_non_negative() {
        let cfg = PerfConfig::default();
        let mut h = Harness::scripted(&cfg);
        while let Some(vp) = h.next_viewport() {
            assert!(vp.scroll_x >= 0.0 && vp.scroll_y >= 0.0, "negative: {vp:?}");
        }
    }

    #[test]
    fn newly_visible_2d_set_difference() {
        // Fully disjoint rows → the whole current region is new.
        assert_eq!(
            newly_visible_2d(&(0..10), &(0..5), &(100..110), &(0..5)),
            50
        );
        // Identical region → nothing new.
        assert_eq!(newly_visible_2d(&(0..10), &(0..5), &(0..10), &(0..5)), 0);
        // First frame (empty prev) → everything new.
        assert_eq!(newly_visible_2d(&(0..0), &(0..0), &(4..9), &(0..3)), 15);
        // Partial row overlap: 3 new rows × 5 cols.
        assert_eq!(newly_visible_2d(&(0..10), &(0..5), &(3..13), &(0..5)), 15);
    }

    #[test]
    fn seeded_random_jumps_are_reproducible() {
        let cfg = PerfConfig::default();
        let a = seeded_random_jumps(&cfg, 16, 1e6, 1e6, 1440.0, 900.0);
        let b = seeded_random_jumps(&cfg, 16, 1e6, 1e6, 1440.0, 900.0);
        assert_eq!(a, b);
    }

    #[test]
    fn percentile_nearest_rank() {
        let sorted: Vec<u64> = (1..=100).collect();
        assert_eq!(percentile_ns(&sorted, 50.0), 50);
        assert_eq!(percentile_ns(&sorted, 99.0), 99);
        assert_eq!(percentile_ns(&sorted, 100.0), 100);
        assert_eq!(percentile_ns(&[], 50.0), 0);
    }

    #[test]
    fn latency_stats_basic() {
        let s = LatencyStats::from_samples(&[10, 20, 30, 40, 50]);
        assert_eq!(s.count, 5);
        assert_eq!(s.min_ns, 10);
        assert_eq!(s.max_ns, 50);
        assert_eq!(s.mean_ns, 30);
        assert_eq!(s.p50_ns, 30);
    }

    fn samples_at(frame_ns: u64, cell_ns: u64, n: usize) -> Vec<FrameSample> {
        (0..n)
            .map(|_| FrameSample {
                frame_render_ns: frame_ns,
                cell_load_ns: cell_ns,
                newly_visible: 5,
                elements: 500,
            })
            .collect()
    }

    #[test]
    fn all_gates_pass_under_target() {
        // 5 ms frame, 0.5 ms cell-load: under every §4 budget.
        let samples = samples_at(5_000_000, 500_000, 200);
        let report = RunReport::build(
            &samples,
            FRAME_TARGET_NS,
            FRAME_WORST_NS,
            CELL_LOAD_TARGET_NS,
        );
        assert!(
            report.passed(),
            "under-target run should PASS:\n{}",
            report.summary("x")
        );
    }

    #[test]
    fn frame_gate_fails_over_120fps_budget_but_within_60fps() {
        // 12 ms frame: over the 8.33 ms p99 gate, under the 16.67 ms worst-case gate.
        let samples = samples_at(12_000_000, 500_000, 200);
        let report = RunReport::build(
            &samples,
            FRAME_TARGET_NS,
            FRAME_WORST_NS,
            CELL_LOAD_TARGET_NS,
        );
        let p99 = report.gates.iter().find(|g| g.name == "frame-p99").unwrap();
        let max = report.gates.iter().find(|g| g.name == "frame-max").unwrap();
        assert!(!p99.pass, "frame-p99 fails at 12 ms");
        assert!(max.pass, "frame-max passes at 12 ms (< 16.67 ms)");
        assert!(!report.passed());
    }

    #[test]
    fn cell_load_gate_fails_over_2ms() {
        let samples = samples_at(5_000_000, 3_000_000, 200);
        let report = RunReport::build(
            &samples,
            FRAME_TARGET_NS,
            FRAME_WORST_NS,
            CELL_LOAD_TARGET_NS,
        );
        let cell = report
            .gates
            .iter()
            .find(|g| g.name == "cell-load-p99")
            .unwrap();
        assert!(!cell.pass, "cell-load-p99 fails at 3 ms");
        assert!(!report.passed());
    }

    #[test]
    fn fmt_ns_units() {
        assert_eq!(fmt_ns(500), "500 ns");
        assert_eq!(fmt_ns(1_500), "1.50 µs");
        assert_eq!(fmt_ns(2_000_000), "2.00 ms");
    }
}
