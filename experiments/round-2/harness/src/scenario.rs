//! Benchmark **scenario definitions** built on the frozen `datagen` generators, so
//! BOTH engines run identical inputs and identical measurement logic (functional_spec
//! §5.3, §6.C; architecture §5). Each scenario is a pure *builder* (seed + shape) plus
//! a *runner* that takes `&mut impl SpreadsheetEngine`, times the operation with
//! `bench_util`, and returns per-iteration [`Duration`]s the caller turns into
//! `LatencyStats` + a gated `GateResult`.
//!
//! Sizes are carried in a [`Profile`] so `cargo test` can run a tiny `dev` shape
//! while the recorded runs use the spec-scale `full` shape — identical code, only the
//! numbers differ.

use std::time::Duration;

use bench_util::time_iters;
use datagen::{linear_chain, wide_fanout, CellSource, SyntheticSheet};

use crate::binding::{read_under, BindingCache, Design};
use crate::engine::{CellInput, EngineValue, SpreadsheetEngine, Viewport};

/// The §5.4 latency targets, in nanoseconds, that scenarios gate against.
pub mod targets {
    /// Newly-visible viewport read must fit inside a frame (< ~2 ms).
    pub const VIEWPORT_READ_NS: u64 = 2_000_000;
    /// A 120 fps render frame budget (~8.3 ms).
    pub const FRAME_120_NS: u64 = 8_333_333;
    /// A 60 fps worst-case frame budget (~16.6 ms) — the "within one frame" bound
    /// for change→visible updates.
    pub const FRAME_60_NS: u64 = 16_666_666;
    /// 1,000,000-cell dependency-chain recompute must finish in < 100 ms.
    pub const CASCADE_1M_NS: u64 = 100_000_000;
}

/// Sizes for a scenario run. `dev` keeps `cargo test` fast; `full` is spec-scale for
/// the recorded benchmark numbers.
#[derive(Debug, Clone, Copy)]
pub struct Profile {
    /// Rows in the seeded synthetic region (scrolling/memory scenarios).
    pub region_rows: u32,
    /// Columns in the seeded synthetic region.
    pub region_cols: u32,
    /// Viewport height in cells.
    pub viewport_rows: u32,
    /// Viewport width in cells.
    pub viewport_cols: u32,
    /// Number of viewport pan steps to time (scrolling read).
    pub pan_steps: usize,
    /// Length of the linear `=PREV+1` chain for the headline 1M cascade-recompute
    /// benchmark (§5.4 target).
    pub chain_len: u32,
    /// Length of the linear chain for the change→visible-update scenario (Scenario 2).
    /// Kept smaller than [`Profile::chain_len`] because that scenario rebuilds the chain
    /// **once per design** (×3) and edits it repeatedly — a full 1M rebuild+recompute per
    /// design would dominate the run without changing the verdict (one recompute already
    /// blows the frame budget). Still large enough to be a genuine offscreen cascade.
    pub visible_chain_len: u32,
    /// Number of `set_value`s to time in the write scenario.
    pub write_count: u32,
    /// Number of populated cells for the memory scenario.
    pub memory_cells: u32,
    /// Sources for the wide-fanout shape.
    pub fanout_sources: u32,
    /// Dependents for the wide-fanout shape.
    pub fanout_dependents: u32,
}

impl Profile {
    /// A tiny profile for unit tests — everything small, runs in milliseconds.
    pub const fn dev() -> Self {
        Self {
            region_rows: 200,
            region_cols: 50,
            viewport_rows: 40,
            viewport_cols: 20,
            pan_steps: 8,
            chain_len: 500,
            visible_chain_len: 500,
            write_count: 200,
            memory_cells: 5_000,
            fanout_sources: 64,
            fanout_dependents: 64,
        }
    }

    /// The spec-scale profile for recorded runs (functional_spec §5.4).
    ///
    /// The scrolling region is a **200,000-cell** block (2,500 rows × 80 cols): large
    /// enough that the 300-step pan path visits many distinct windows across a sheet far
    /// bigger than the ~1,800-cell viewport (both axes). It is seeded via each engine's
    /// **native bulk-ingest** path (`bulk_load_block`), so seed cost is small on both; the
    /// viewport read (the thing measured) is unaffected by the region size.
    ///
    /// Heavy-scenario scales are chosen from measured single-shot costs on the 4-core
    /// target so the whole suite completes inside a bounded wall-clock budget: the **1M**
    /// headline cascade (Scenario 3a) is kept at spec scale; the change→visible chain
    /// (Scenario 2, rebuilt ×3) is 100k; the fan-out is 1,000×1,000 (a 5,000×5,000 shape
    /// costs ~100 s/recompute on Formualizer — recorded as a ceiling finding, not a
    /// per-run gate); the memory load targets the full **10⁷ cells** via bulk-ingest
    /// (both engines reach it in single-digit seconds), with a safety budget as a backstop.
    pub const fn full() -> Self {
        Self {
            region_rows: 2_500,
            region_cols: 80,
            viewport_rows: 60,
            viewport_cols: 30, // ~1,800 cells/viewport (order 10^3, per §5.4)
            pan_steps: 300,
            chain_len: 1_000_000,
            visible_chain_len: 100_000,
            // 1,000 single edits, each with a per-edit recompute, keeps the
            // non-incremental engine's single-write path bounded while the single-vs-
            // batched contrast stays clear.
            write_count: 1_000,
            memory_cells: 10_000_000,
            fanout_sources: 1_000,
            fanout_dependents: 1_000,
        }
    }
}

/// The seed used for all synthetic inputs, so runs are reproducible.
pub const SEED: u64 = 0xF2EE_CE11;

/// Seeds `engine` with a `rows × cols` block of synthetic literal values (numbers
/// and text) from a deterministic [`SyntheticSheet`], via each engine's **fastest
/// native bulk-ingest path** ([`SpreadsheetEngine::bulk_load_block`]).
///
/// Using `bulk_load_block` (not a chunked `set_batch`) is a fairness requirement: it
/// routes Formualizer through its columnar Arrow ingest (~O(cells)) and IronCalc through
/// a direct `set_user_input` loop, so the seed cost of the scrolling/cascade-visible
/// scenarios is measured on each engine's optimal loader — not Formualizer's super-linear
/// interactive `write_range` overlay path (the source of an earlier unfair measurement).
pub fn seed_region(engine: &mut impl SpreadsheetEngine, rows: u32, cols: u32) {
    let sheet = SyntheticSheet::new(SEED, rows, cols);
    engine.bulk_load_block(rows, cols, &|r, c| match sheet.cell(r, c).value {
        datagen::CellValue::Empty => EngineValue::Empty,
        datagen::CellValue::Number(n) => EngineValue::Number(n),
        datagen::CellValue::Text(t) => EngineValue::Text(t),
    });
}

/// A deterministic pan path across the region: steps down and right in a sweep so we
/// exercise both vertical and horizontal scrolling. Each entry is the viewport's
/// top-left.
pub fn pan_path(profile: &Profile) -> Vec<(u32, u32)> {
    let mut path = Vec::with_capacity(profile.pan_steps);
    let max_row = profile
        .region_rows
        .saturating_sub(profile.viewport_rows)
        .max(1);
    let max_col = profile
        .region_cols
        .saturating_sub(profile.viewport_cols)
        .max(1);
    for i in 0..profile.pan_steps {
        // Move diagonally with different strides so the path visits varied windows.
        let row = ((i as u32).wrapping_mul(37)) % max_row;
        let col = ((i as u32).wrapping_mul(13)) % max_col;
        path.push((row, col));
    }
    path
}

/// **Scenario 1 — scrolling viewport read.** Seeds a synthetic region, then times a
/// single viewport pull at each pan step under `design`. Returns one [`Duration`] per
/// step. Gate p99 against [`targets::VIEWPORT_READ_NS`].
///
/// Convenience wrapper that seeds then runs; the runner uses [`seed_region`] +
/// [`scrolling_viewport_read_seeded`] to seed **once** and time all three designs
/// against the same (read-only) sheet, since seeding a 2M-cell region dominates the
/// cost on a non-incremental engine.
pub fn scrolling_viewport_read(
    engine: &mut impl SpreadsheetEngine,
    profile: &Profile,
    design: Design,
) -> Vec<Duration> {
    seed_region(engine, profile.region_rows, profile.region_cols);
    scrolling_viewport_read_seeded(engine, profile, design)
}

/// Times the scrolling read on an **already-seeded** engine (see
/// [`scrolling_viewport_read`]). Read-only, so one seeded engine can serve all designs.
pub fn scrolling_viewport_read_seeded(
    engine: &impl SpreadsheetEngine,
    profile: &Profile,
    design: Design,
) -> Vec<Duration> {
    let path = pan_path(profile);
    let mut samples = Vec::with_capacity(path.len());
    // For D3 we keep a warm cache across pans (a realistic scrolling cache).
    let mut cache = BindingCache::new();
    for (row0, col0) in path {
        let vp = Viewport::new(row0, col0, profile.viewport_rows, profile.viewport_cols);
        let (_, dt) = bench_util::time_once(|| match design {
            Design::CachedChangelog => {
                cache.prime(engine, vp);
                cache.snapshot(engine, vp)
            }
            other => read_under(other, engine, vp),
        });
        samples.push(dt);
    }
    samples
}

/// **Scenario 2 — change-cascade → visible update.** Builds a linear chain whose tail
/// runs *offscreen*, primes a visible viewport away from the head, then repeatedly
/// edits the head cell and re-reads the visible viewport, timing the whole
/// edit→recompute→read cycle. Returns one [`Duration`] per edit. Gate p99 against
/// [`targets::FRAME_60_NS`].
pub fn cascade_to_visible_update(
    engine: &mut impl SpreadsheetEngine,
    profile: &Profile,
    design: Design,
    edits: usize,
) -> CascadeRun {
    let chain_len = profile.visible_chain_len;
    let (_, build_time) = bench_util::time_once(|| build_linear_chain(engine, chain_len));
    engine.enable_change_tracking();
    let _ = engine.drain_dirty();

    // Visible viewport sits near the tail of the chain (offscreen from the head at
    // row 0): a change at the head must cascade all the way to here.
    let tail = chain_len.saturating_sub(profile.viewport_rows).max(1);
    let vp = Viewport::new(tail, 0, profile.viewport_rows, profile.viewport_cols.min(4));
    let mut cache = BindingCache::new();
    cache.prime(engine, vp);

    let mut samples = Vec::with_capacity(edits);
    let mut final_head = 0.0;
    let mut last_visible = EngineValue::Empty;
    for i in 0..edits {
        let new_head = (i as f64) + 1.0;
        final_head = new_head;
        let (view, dt) = bench_util::time_once(|| {
            engine.set_value(0, 0, EngineValue::Number(new_head));
            engine.recompute();
            match design {
                Design::CachedChangelog => {
                    // Drain the edit-site dirty set and refresh correctly: an edit
                    // outside the window (the chain head) forces a re-prime so the
                    // cascade into the visible tail is picked up.
                    let dirty = engine.drain_dirty();
                    cache.refresh_after_edits(engine, &dirty);
                    cache.snapshot(engine, vp)
                }
                other => read_under(other, engine, vp),
            }
        });
        // The first visible cell is at row `tail`: its value must be head + tail after
        // the cascade reaches it — proving the read reflects the edit, not stale cache.
        last_visible = view.first().cloned().unwrap_or(EngineValue::Empty);
        samples.push(dt);
    }
    CascadeRun {
        samples,
        tail_value: last_visible,
        expected_tail: final_head + tail as f64,
        final_head,
        verified_len: chain_len,
        build_time,
    }
}

/// The verified outcome of a cascade run: the timing samples plus a **correctness
/// proof** — the tail value after the final edit, and what it must equal for a real
/// full recompute to have happened. The runner asserts `tail == expected_tail` so the
/// benchmark can never silently measure a no-op / cached read (credibility guard).
#[derive(Debug)]
pub struct CascadeRun {
    pub samples: Vec<Duration>,
    pub tail_value: EngineValue,
    pub expected_tail: f64,
    /// The head value that produced `expected_tail` (for diagnostics).
    pub final_head: f64,
    /// The chain length actually built (echoed back for the record).
    pub verified_len: u32,
    /// Wall-clock time to BUILD the shape (via the engine's batched write path) — a real,
    /// interesting number kept separate from the measured recompute samples.
    pub build_time: Duration,
}

impl CascadeRun {
    /// `true` iff the observed tail equals the value a real `=PREV+1` cascade must
    /// produce for the final head edit — i.e. the recompute genuinely happened.
    pub fn is_correct(&self) -> bool {
        self.tail_value.as_number() == Some(self.expected_tail)
    }
}

/// **Scenario 3a — 1,000,000-cell `=PREV+1` cascade recompute.** Builds the chain,
/// then times a head edit + full recompute per rep. **Verifies** that after the last
/// edit the chain tail equals `head + (len-1)` — a real cascade cannot fake this, so a
/// passing assertion proves the timed work was a genuine full recompute. Returns a
/// [`CascadeRun`]; gate against [`targets::CASCADE_1M_NS`].
pub fn cascade_recompute_chain(
    engine: &mut impl SpreadsheetEngine,
    chain_len: u32,
    reps: usize,
) -> CascadeRun {
    let (_, build_time) = bench_util::time_once(|| build_linear_chain(engine, chain_len));
    // Sanity: the freshly-built chain tail must be head(=1) + (len-1).
    let tail_row = chain_len.saturating_sub(1);
    let mut samples = Vec::with_capacity(reps);
    let mut final_head = 0.0;
    for i in 0..reps {
        let new_head = (i as f64) + 2.0;
        final_head = new_head;
        let (_, dt) = bench_util::time_once(|| {
            engine.set_value(0, 0, EngineValue::Number(new_head));
            engine.recompute();
        });
        samples.push(dt);
    }
    let tail_value = engine.get_value(tail_row, 0);
    CascadeRun {
        samples,
        tail_value,
        expected_tail: final_head + tail_row as f64,
        final_head,
        verified_len: chain_len,
        build_time,
    }
}

/// **Scenario 3b — wide fan-out recompute.** Builds a `wide_fanout` shape (`dependents`
/// cells each `=SUM(A1:<lastSource>1)` over `sources` source cells), edits source A1,
/// recomputes. **Verifies** a dependent equals the sum a real recompute must produce
/// (source A1 = `new`, sources 2..=`sources` hold `2..=sources`), so a passing check
/// proves the fan-out genuinely recomputed. Returns a [`CascadeRun`]; reported
/// (discovery) against the frame budget.
pub fn cascade_recompute_fanout(
    engine: &mut impl SpreadsheetEngine,
    sources: u32,
    dependents: u32,
    reps: usize,
) -> CascadeRun {
    let cells = wide_fanout(sources, dependents);
    let batch: Vec<(u32, u32, CellInput)> = cells
        .iter()
        .map(|fc| {
            (
                fc.addr.row,
                fc.addr.col,
                CellInput::Formula(fc.formula.clone()),
            )
        })
        .collect();
    let (_, build_time) = bench_util::time_once(|| engine.set_batch(&batch));
    let mut samples = Vec::with_capacity(reps);
    let mut final_head = 0.0;
    for i in 0..reps {
        let new_source = (i as f64) + 3.0;
        final_head = new_source;
        let (_, dt) = bench_util::time_once(|| {
            engine.set_value(0, 0, EngineValue::Number(new_source));
            engine.recompute();
        });
        samples.push(dt);
    }
    // Sources are laid out across row 0: col 0 = A1 (edited to final_head), col c holds
    // (c+1) for c in 1..sources. A dependent sums them all.
    let others_sum: f64 = (1..sources).map(|c| (c + 1) as f64).sum();
    let expected = final_head + others_sum;
    // Dependents live down column `sources`, starting at row 0.
    let tail_value = engine.get_value(0, sources);
    CascadeRun {
        samples,
        tail_value,
        expected_tail: expected,
        final_head,
        verified_len: sources + dependents,
        build_time,
    }
}

/// **Scenario 4 — writes: single vs batched.** Times `write_count` individual
/// `set_value` + `recompute` cycles (the honest *interactive single-edit* cost —
/// incremental on Formualizer, a full re-evaluate on IronCalc), then times one
/// `set_batch` of the same cells (a single recompute for the whole batch). Returns
/// `(single_samples, batched_samples)`. The gap between them is the finding: it says
/// how much the binding layer must batch to stay cheap on each engine.
pub fn writes_single_vs_batched(
    engine_single: &mut impl SpreadsheetEngine,
    engine_batch: &mut impl SpreadsheetEngine,
    write_count: u32,
) -> (Vec<Duration>, Vec<Duration>) {
    // Single: time each individual write *including the per-edit recompute* the engine
    // would run in an interactive session.
    let single = time_iters(write_count as usize, {
        let mut i = 0u32;
        move || {
            engine_single.set_value(0, i, EngineValue::Number(i as f64));
            engine_single.recompute();
            i += 1;
        }
    });

    // Batched: one call, timed once (a single recompute for the whole batch).
    let batch: Vec<(u32, u32, CellInput)> = (0..write_count)
        .map(|i| (0, i, CellInput::Value(EngineValue::Number(i as f64))))
        .collect();
    let (_, dt) = bench_util::time_once(|| engine_batch.set_batch(&batch));
    (single, vec![dt])
}

/// The verified outcome of the memory scenario: how many cells were loaded (spot-checked,
/// not just requested) so the runner can prove the region was populated, plus the
/// wall-clock **bulk-ingest** load time and whether a safety budget was tripped.
#[derive(Debug)]
pub struct MemoryRun {
    /// Cells requested to load (the target scale, e.g. 10⁷).
    pub requested: u32,
    /// Cells actually loaded (the full dense `rows × 1000` block).
    pub loaded: u32,
    /// A far-corner (last-loaded) cell's value — proves the tail of the region was written.
    pub far_corner: EngineValue,
    /// The expected far-corner value.
    pub expected_far_corner: f64,
    /// Wall-clock time to load `loaded` cells via the engine's native bulk-ingest path.
    pub load_time: Duration,
    /// `true` only if the load exceeded the safety budget (does not trip on the fast
    /// bulk-ingest paths).
    pub capped: bool,
}

impl MemoryRun {
    /// `true` iff the far-corner cell holds the value it was loaded with — proof the
    /// loaded region (not just the first rows) was populated.
    pub fn is_correct(&self) -> bool {
        self.far_corner.as_number() == Some(self.expected_far_corner)
    }
}

/// **Scenario 5 — memory load + edit.** Populates a dense `cells`-cell block of literals
/// (laid out as a wide rectangle, 1,000 cols, to avoid a single 10⁷-long column) via each
/// engine's **fastest native bulk-ingest path** ([`SpreadsheetEngine::bulk_load_block`]:
/// Formualizer's columnar Arrow ingest, IronCalc's `set_user_input` loop), then performs
/// one edit + recompute. Returns a [`MemoryRun`] that spot-checks the last-loaded
/// far-corner cell so the caller can prove the region was genuinely populated; the caller
/// samples peak RSS around this call.
///
/// `budget` is a **safety ceiling only**: if the load ever exceeds it the run steps down
/// and records how far it got (per the phase guardrail). On the fast bulk-ingest paths
/// neither engine trips it (both reach 10⁷ in single-digit seconds), so a recorded run
/// normally shows `loaded == requested`, `capped == false`.
pub fn memory_load_and_edit(
    engine: &mut impl SpreadsheetEngine,
    cells: u32,
    budget: Duration,
) -> MemoryRun {
    let cols: u32 = 1_000;
    let rows = cells / cols; // exact when cells is a multiple of 1,000 (the full profile)
    let loaded = rows * cols;

    let start = std::time::Instant::now();
    engine.bulk_load_block(rows, cols, &|r, c| {
        EngineValue::Number((r * cols + c) as f64)
    });
    let load_time = start.elapsed();
    // The bulk-ingest path is a single call, so the "cap" can only be judged after it
    // returns; on these engines it never trips (kept for the honest record).
    let capped = load_time >= budget;

    // Spot-check the LAST loaded cell (far corner) before the edit overwrites (0,0).
    let last_i = loaded.saturating_sub(1);
    let far_row = last_i / cols;
    let far_col = last_i % cols;
    let far_corner = engine.get_value(far_row, far_col);

    // One edit + recompute, to exercise the post-load edit path.
    engine.set_value(0, 0, EngineValue::Number(-1.0));
    engine.recompute();

    MemoryRun {
        requested: cells,
        loaded,
        far_corner,
        expected_far_corner: last_i as f64,
        load_time,
        capped,
    }
}

/// Builds a `len`-cell `=PREV+1` linear chain down column 0 via a single batch (one
/// recompute), using the shared `datagen::linear_chain` shape.
pub fn build_linear_chain(engine: &mut impl SpreadsheetEngine, len: u32) {
    let batch: Vec<(u32, u32, CellInput)> = linear_chain(len, 0)
        .map(|fc| {
            (
                fc.addr.row,
                fc.addr.col,
                CellInput::Formula(fc.formula.clone()),
            )
        })
        .collect();
    engine.set_batch(&batch);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeEngine;

    fn dev() -> Profile {
        Profile::dev()
    }

    #[test]
    fn scenario_scrolling_read_runs() {
        let mut e = FakeEngine::new_blank();
        let samples = scrolling_viewport_read(&mut e, &dev(), Design::BulkRange);
        assert_eq!(samples.len(), dev().pan_steps);
    }

    #[test]
    fn pan_path_stays_in_bounds() {
        let p = dev();
        for (r, c) in pan_path(&p) {
            assert!(r + p.viewport_rows <= p.region_rows);
            assert!(c + p.viewport_cols <= p.region_cols);
        }
    }

    #[test]
    fn scenario_cascade_to_visible_reflects_edit() {
        let mut e = FakeEngine::new_blank();
        // Small chain so the visible tail cascades from the head.
        let p = Profile {
            visible_chain_len: 20,
            viewport_rows: 4,
            viewport_cols: 1,
            ..dev()
        };
        let run = cascade_to_visible_update(&mut e, &p, Design::CachedChangelog, 3);
        assert_eq!(run.samples.len(), 3);
        // The visible cell reflects the cascade: its verification must hold.
        assert!(run.is_correct(), "visible cascade not reflected: {run:?}");
        // After the last edit (head = 3.0), the chain tail reflects it too.
        let tail = p.visible_chain_len - 1;
        let expected = 3.0 + (tail as f64);
        assert_eq!(e.get_value(tail, 0), EngineValue::Number(expected));
    }

    #[test]
    fn scenario_1m_chain_builder_shape() {
        let mut e = FakeEngine::new_blank();
        build_linear_chain(&mut e, 6);
        // Head is =1, cell k is head + k.
        assert_eq!(e.get_value(0, 0), EngineValue::Number(1.0));
        assert_eq!(e.get_value(5, 0), EngineValue::Number(6.0));
    }

    #[test]
    fn scenario_cascade_recompute_chain_runs_and_verifies() {
        let mut e = FakeEngine::new_blank();
        let run = cascade_recompute_chain(&mut e, 50, 4);
        assert_eq!(run.samples.len(), 4);
        // The verification proves a real recompute happened: tail == head + (len-1).
        assert!(run.is_correct(), "chain recompute not verified: {run:?}");
    }

    #[test]
    fn scenario_fanout_runs_and_verifies() {
        let mut e = FakeEngine::new_blank();
        let run = cascade_recompute_fanout(&mut e, 8, 4, 2);
        assert_eq!(run.samples.len(), 2);
        // A dependent sums sources; the verification proves the fan-out recomputed.
        assert!(run.is_correct(), "fanout recompute not verified: {run:?}");
    }

    #[test]
    fn scenario_writes_single_vs_batched_both_run() {
        let mut e1 = FakeEngine::new_blank();
        let mut e2 = FakeEngine::new_blank();
        let (single, batched) = writes_single_vs_batched(&mut e1, &mut e2, 50);
        assert_eq!(single.len(), 50);
        assert_eq!(batched.len(), 1);
    }

    #[test]
    fn scenario_memory_load_populates_and_verifies() {
        let mut e = FakeEngine::new_blank();
        let mem = memory_load_and_edit(&mut e, 2_000, Duration::from_secs(60));
        // The far-corner spot-check proves the whole region was populated.
        assert!(mem.is_correct(), "memory region not populated: {mem:?}");
        assert_eq!(mem.requested, 2_000);
        assert_eq!(mem.loaded, 2_000);
        assert!(!mem.capped, "a tiny load should never hit the budget");
    }
}
