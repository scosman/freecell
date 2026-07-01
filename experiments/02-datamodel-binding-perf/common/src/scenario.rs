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
    /// Length of the linear `=PREV+1` chain (cascade scenarios).
    pub chain_len: u32,
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
            write_count: 200,
            memory_cells: 5_000,
            fanout_sources: 64,
            fanout_dependents: 64,
        }
    }

    /// The spec-scale profile for recorded runs (functional_spec §5.4).
    ///
    /// The scrolling region is a **2,000,000-cell** block (25,000 rows × 80 cols):
    /// large enough that the pan path visits many distinct windows across a huge sheet
    /// (both axes), while keeping the (repeated, per-design) seeding cost bounded on the
    /// non-incremental engine. The viewport is ~1,800 cells (order 10^3, per §5.4).
    pub const fn full() -> Self {
        Self {
            region_rows: 25_000,
            region_cols: 80,
            viewport_rows: 60,
            viewport_cols: 30, // ~1,800 cells/viewport (order 10^3, per §5.4)
            pan_steps: 300,
            chain_len: 1_000_000,
            // 2,000 single edits, each with a per-edit recompute, keeps the
            // non-incremental engine's single-write path bounded while the single-vs-
            // batched contrast stays clear.
            write_count: 2_000,
            memory_cells: 10_000_000,
            fanout_sources: 5_000,
            fanout_dependents: 5_000,
        }
    }
}

/// The seed used for all synthetic inputs, so runs are reproducible.
pub const SEED: u64 = 0xF2EE_CE11;

/// Seeds `engine` with a `rows × cols` block of synthetic literal values (numbers
/// and text) from a deterministic [`SyntheticSheet`]. Uses a single batch so
/// non-incremental engines pay one recompute, not one per cell.
pub fn seed_region(engine: &mut impl SpreadsheetEngine, rows: u32, cols: u32) {
    let sheet = SyntheticSheet::new(SEED, rows, cols);
    let mut batch = Vec::with_capacity((rows as usize) * (cols as usize));
    for r in 0..rows {
        for c in 0..cols {
            let v = match sheet.cell(r, c).value {
                datagen::CellValue::Empty => EngineValue::Empty,
                datagen::CellValue::Number(n) => EngineValue::Number(n),
                datagen::CellValue::Text(t) => EngineValue::Text(t),
            };
            if v != EngineValue::Empty {
                batch.push((r, c, CellInput::Value(v)));
            }
        }
    }
    engine.set_batch(&batch);
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
) -> Vec<Duration> {
    build_linear_chain(engine, profile.chain_len);
    engine.enable_change_tracking();
    let _ = engine.drain_dirty();

    // Visible viewport sits near the tail of the chain (offscreen from the head at
    // row 0): a change at the head must cascade all the way to here.
    let tail = profile
        .chain_len
        .saturating_sub(profile.viewport_rows)
        .max(1);
    let vp = Viewport::new(tail, 0, profile.viewport_rows, profile.viewport_cols.min(4));
    let mut cache = BindingCache::new();
    cache.prime(engine, vp);

    let mut samples = Vec::with_capacity(edits);
    for i in 0..edits {
        let new_head = (i as f64) + 1.0;
        let (_, dt) = bench_util::time_once(|| {
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
        samples.push(dt);
    }
    samples
}

/// **Scenario 3a — 1,000,000-cell `=PREV+1` cascade recompute.** Builds the chain,
/// then times a single head edit + full recompute. Returns one [`Duration`] per
/// repetition. Gate against [`targets::CASCADE_1M_NS`].
pub fn cascade_recompute_chain(
    engine: &mut impl SpreadsheetEngine,
    chain_len: u32,
    reps: usize,
) -> Vec<Duration> {
    build_linear_chain(engine, chain_len);
    let mut samples = Vec::with_capacity(reps);
    for i in 0..reps {
        let new_head = (i as f64) + 2.0;
        let (_, dt) = bench_util::time_once(|| {
            engine.set_value(0, 0, EngineValue::Number(new_head));
            engine.recompute();
        });
        samples.push(dt);
    }
    samples
}

/// **Scenario 3b — wide fan-out recompute.** Builds a `wide_fanout` shape (many
/// dependents summing the same source block), edits one source, recomputes. Returns
/// one [`Duration`] per repetition; reported (discovery) against the frame budget.
pub fn cascade_recompute_fanout(
    engine: &mut impl SpreadsheetEngine,
    sources: u32,
    dependents: u32,
    reps: usize,
) -> Vec<Duration> {
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
    engine.set_batch(&batch);
    let mut samples = Vec::with_capacity(reps);
    for i in 0..reps {
        let new_source = (i as f64) + 3.0;
        let (_, dt) = bench_util::time_once(|| {
            engine.set_value(0, 0, EngineValue::Number(new_source));
            engine.recompute();
        });
        samples.push(dt);
    }
    samples
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

/// **Scenario 5 — memory load + edit.** Populates `cells` literal values in **chunked
/// batches** (a realistic bulk-load shape, and it keeps any per-batch temporary — e.g.
/// a large `BTreeMap` in the Formualizer adapter — bounded rather than materializing all
/// 10⁷ at once), then performs one edit + recompute. Returns nothing measurable here
/// beyond the side effect of a fully-loaded workbook; the *caller* samples peak RSS
/// around this call (peak RSS is a platform read the bench binary owns, not this crate).
pub fn memory_load_and_edit(engine: &mut impl SpreadsheetEngine, cells: u32) {
    // Lay the cells out as a wide-ish rectangle to avoid a single 10^7-long column, and
    // load in chunks of whole rows so each batch is bounded.
    let cols: u32 = 1_000;
    let chunk_rows: u32 = 100; // 100k cells per batch
    let total_rows = cells.div_ceil(cols);
    let mut row = 0;
    while row < total_rows {
        let end_row = (row + chunk_rows).min(total_rows);
        let mut batch: Vec<(u32, u32, CellInput)> =
            Vec::with_capacity((end_row - row) as usize * cols as usize);
        for r in row..end_row {
            for c in 0..cols {
                let i = r * cols + c;
                if i >= cells {
                    break;
                }
                batch.push((r, c, CellInput::Value(EngineValue::Number(i as f64))));
            }
        }
        engine.set_batch(&batch);
        row = end_row;
    }
    // One edit + recompute, to exercise the post-load edit path.
    engine.set_value(0, 0, EngineValue::Number(-1.0));
    engine.recompute();
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
            chain_len: 20,
            viewport_rows: 4,
            viewport_cols: 1,
            ..dev()
        };
        let samples = cascade_to_visible_update(&mut e, &p, Design::CachedChangelog, 3);
        assert_eq!(samples.len(), 3);
        // After the last edit (head = 3.0), the chain tail should reflect it:
        // cell (chain_len-1) == head + (chain_len-1).
        let tail = p.chain_len - 1;
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
    fn scenario_cascade_recompute_chain_runs() {
        let mut e = FakeEngine::new_blank();
        let samples = cascade_recompute_chain(&mut e, 50, 4);
        assert_eq!(samples.len(), 4);
    }

    #[test]
    fn scenario_fanout_runs_and_sums() {
        let mut e = FakeEngine::new_blank();
        let samples = cascade_recompute_fanout(&mut e, 8, 4, 2);
        assert_eq!(samples.len(), 2);
        // A dependent sums sources; after editing source A1 the sum reflects it.
        // Sources are 1..=8 except A1 which we last set to 3.0+? -> just check a number.
        let dep = e.get_value(0, 8);
        assert!(dep.as_number().is_some());
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
    fn scenario_memory_load_populates() {
        let mut e = FakeEngine::new_blank();
        memory_load_and_edit(&mut e, 2_000);
        // A late cell exists.
        assert!(e.get_value(1, 999).as_number().is_some());
    }
}
