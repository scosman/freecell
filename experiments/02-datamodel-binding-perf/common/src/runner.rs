//! The shared benchmark **driver**: runs all five scenarios × applicable binding
//! designs against any [`SpreadsheetEngine`], gates each against its §5.4 target, and
//! returns [`ScenarioResult`]s ready for `report::write_all`. Both engine bins call
//! this with a factory closure, so the two engines run byte-for-byte identical
//! driving logic (comparability is the whole point of the bake-off).

use bench_util::{Environment, GateResult, LatencyStats};
use serde_json::json;

use crate::binding::Design;
use crate::engine::SpreadsheetEngine;
use crate::report::ScenarioResult;
use crate::scenario::{self, targets, Profile};

/// How many repetitions of the (expensive) 1M-cell cascade recompute to time. Each rep
/// is a full 1M-deep recompute (~2–3 s per engine on the 4-core target), so we keep the
/// count small: the recompute is deterministic, so a handful of reps already give a
/// stable p50/p99 while bounding the total wall-clock (build ~6–27 s + reps × ~2–3 s).
const CASCADE_REPS: usize = 5;
/// Edits to time for the change→visible scenario (rebuilt once per design, ×3).
const VISIBLE_EDITS: usize = 5;
/// Reps for the (cheaper) fan-out recompute — a 1,000×1,000 shape recomputes in tens of
/// ms (IronCalc) to a few seconds (Formualizer); a few reps bound Formualizer's cost.
const FANOUT_REPS: usize = 5;
/// Wall-clock budget for the memory-load scenario. If an engine can't reach the target
/// cell count within this budget the load steps down and records the ceiling it reached
/// (Formualizer's `write_range` load is super-linear and can't hit 10⁷ in-budget; see
/// findings). ~90 s keeps the whole suite bounded while letting IronCalc reach 10⁷.
const MEMORY_BUDGET: std::time::Duration = std::time::Duration::from_secs(90);

/// Runs the whole scenario suite against a fresh engine per scenario (built by
/// `make`), returning one [`ScenarioResult`] per (scenario, design). `env`/`date`
/// stamp every record; `profile` selects `dev` vs `full` sizes; `peak_rss` reads the
/// process's peak resident memory (bytes) for the memory scenario (platform-specific,
/// supplied by the caller).
pub fn run_suite<E, F>(
    make: F,
    profile: &Profile,
    env: &Environment,
    date: &str,
    peak_rss: fn() -> u64,
) -> Vec<ScenarioResult>
where
    E: SpreadsheetEngine,
    F: Fn() -> E,
{
    let engine_name = make().name().to_string();
    let mut out = Vec::new();

    // --- Scenario 1: scrolling viewport read (D1/D2/D3), gate p99 <= 2 ms ---
    // Seed the (large) region ONCE and time all three designs against it — the read
    // path is non-mutating, and seeding dominates cost on a non-incremental engine.
    let scrolling_engine = {
        let mut e = make();
        scenario::seed_region(&mut e, profile.region_rows, profile.region_cols);
        e
    };
    // CREDIBILITY GUARD: confirm the region really holds populated cells across a huge
    // sheet — read a far-corner viewport and require some non-empty values, so we can't
    // be "fast" by reading an empty grid.
    {
        let far = crate::Viewport::new(
            profile.region_rows.saturating_sub(5).max(1),
            profile.region_cols.saturating_sub(5).max(1),
            5,
            5,
        );
        let far_vals = scrolling_engine.read_viewport(far);
        let non_empty = far_vals
            .iter()
            .filter(|v| !matches!(v, crate::EngineValue::Empty))
            .count();
        assert!(
            non_empty > 0,
            "{engine_name} scrolling region is empty at the far corner (rows={}, cols={}) — \
             the read benchmark would be measuring an empty grid",
            profile.region_rows,
            profile.region_cols,
        );
    }
    for design in Design::ALL {
        let samples = scenario::scrolling_viewport_read_seeded(&scrolling_engine, profile, design);
        let stats = LatencyStats::from_durations(&samples);
        let gate = stats.map(|s| {
            GateResult::max(
                format!("scrolling-read/{}", design.label()),
                s.p99_ns,
                targets::VIEWPORT_READ_NS,
            )
        });
        out.push(ScenarioResult::from_stats(
            &engine_name,
            "scrolling-read",
            Some(design.label()),
            profile.region_rows as u64 * profile.region_cols as u64,
            date,
            env.clone(),
            stats,
            gate,
            json!({ "viewport_cells": profile.viewport_rows * profile.viewport_cols }),
        ));
    }

    // --- Scenario 2: change-cascade -> visible update (D1/D2/D3), gate p99 <= 16.6 ms ---
    for design in Design::ALL {
        let mut engine = make();
        let run = scenario::cascade_to_visible_update(&mut engine, profile, design, VISIBLE_EDITS);
        // CREDIBILITY GUARD: the visible (offscreen-from-head) cell must reflect the
        // cascade from the head edit — proves the read is fresh, not stale cache.
        assert!(
            run.is_correct(),
            "{engine_name} cascade-visible/{} did NOT reflect the cascade: visible={:?}, expected {} (head={}, chain_len={})",
            design.label(),
            run.tail_value,
            run.expected_tail,
            run.final_head,
            run.verified_len,
        );
        let stats = LatencyStats::from_durations(&run.samples);
        let gate = stats.map(|s| {
            GateResult::max(
                format!("cascade-visible/{}", design.label()),
                s.p99_ns,
                targets::FRAME_60_NS,
            )
        });
        out.push(ScenarioResult::from_stats(
            &engine_name,
            "cascade-visible",
            Some(design.label()),
            profile.visible_chain_len as u64,
            date,
            env.clone(),
            stats,
            gate,
            json!({
                "chain_len": profile.visible_chain_len,
                "build_ms": run.build_time.as_millis() as u64,
                "verified_visible": run.tail_value.as_number(),
                "expected_visible": run.expected_tail,
                "cascade_verified": run.is_correct(),
            }),
        ));
    }

    // --- Scenario 3a: 1M =PREV+1 cascade recompute, gate <= 100 ms ---
    {
        let mut engine = make();
        let run = scenario::cascade_recompute_chain(&mut engine, profile.chain_len, CASCADE_REPS);
        // CREDIBILITY GUARD: the timed work must be a genuine full recompute. A real
        // =PREV+1 cascade forces tail == head + (len-1); a no-op / cached read cannot.
        assert!(
            run.is_correct(),
            "{engine_name} cascade-1m-chain did NOT recompute: chain_len={}, tail={:?}, expected {} (head={}). \
             The benchmark would be measuring nothing — refusing to record a bogus number.",
            run.verified_len,
            run.tail_value,
            run.expected_tail,
            run.final_head,
        );
        let stats = LatencyStats::from_durations(&run.samples);
        let gate =
            stats.map(|s| GateResult::max("cascade-1m-chain", s.p50_ns, targets::CASCADE_1M_NS));
        out.push(ScenarioResult::from_stats(
            &engine_name,
            "cascade-1m-chain",
            None,
            profile.chain_len as u64,
            date,
            env.clone(),
            stats,
            gate,
            json!({
                "chain_len": profile.chain_len,
                "build_ms": run.build_time.as_millis() as u64,
                "verified_tail": run.tail_value.as_number(),
                "expected_tail": run.expected_tail,
                "recompute_verified": run.is_correct(),
            }),
        ));
    }

    // --- Scenario 3b: wide fan-out recompute (discovery, gate at frame budget) ---
    {
        let mut engine = make();
        let run = scenario::cascade_recompute_fanout(
            &mut engine,
            profile.fanout_sources,
            profile.fanout_dependents,
            FANOUT_REPS,
        );
        assert!(
            run.is_correct(),
            "{engine_name} cascade-fanout did NOT recompute: dependent={:?}, expected {} (edited source={})",
            run.tail_value,
            run.expected_tail,
            run.final_head,
        );
        let stats = LatencyStats::from_durations(&run.samples);
        let gate = stats.map(|s| GateResult::max("cascade-fanout", s.p50_ns, targets::FRAME_60_NS));
        out.push(ScenarioResult::from_stats(
            &engine_name,
            "cascade-fanout",
            None,
            (profile.fanout_sources + profile.fanout_dependents) as u64,
            date,
            env.clone(),
            stats,
            gate,
            json!({
                "sources": profile.fanout_sources,
                "dependents": profile.fanout_dependents,
                "build_ms": run.build_time.as_millis() as u64,
                "verified_dependent": run.tail_value.as_number(),
                "expected_dependent": run.expected_tail,
                "recompute_verified": run.is_correct(),
            }),
        ));
    }

    // --- Scenario 4: writes single vs batched (discovery; report the ratio) ---
    {
        let mut e_single = make();
        let mut e_batch = make();
        let (single, batched) =
            scenario::writes_single_vs_batched(&mut e_single, &mut e_batch, profile.write_count);
        let single_stats = LatencyStats::from_durations(&single);
        let batched_stats = LatencyStats::from_durations(&batched);
        let ratio = match (single_stats, batched_stats) {
            (Some(s), Some(b)) if b.mean_ns > 0 => {
                // Total single time vs the single batched call.
                let single_total = s.mean_ns.saturating_mul(profile.write_count as u64);
                single_total as f64 / b.mean_ns as f64
            }
            _ => 0.0,
        };
        out.push(ScenarioResult::from_stats(
            &engine_name,
            "writes-single",
            None,
            profile.write_count as u64,
            date,
            env.clone(),
            single_stats,
            None,
            json!({ "batched_ns": batched_stats.map(|b| b.mean_ns), "single_total_vs_batched_ratio": ratio }),
        ));
    }

    // --- Scenario 5: memory load + edit (discovery: peak RSS, wall-clock-budgeted) ---
    {
        let rss_before = peak_rss();
        let mut engine = make();
        let mem = scenario::memory_load_and_edit(&mut engine, profile.memory_cells, MEMORY_BUDGET);
        let rss_after = peak_rss();
        // Keep the engine alive across the RSS read.
        std::hint::black_box(&engine);
        // CREDIBILITY GUARD: the far-corner cell proves the LOADED region was populated
        // (whatever scale the budget allowed), so the recorded RSS is for a genuinely
        // loaded workbook — not an empty grid — even when the load stepped down.
        assert!(
            mem.is_correct(),
            "{engine_name} memory scenario did NOT populate the region: loaded {} of {} cells, \
             far corner={:?}, expected {}",
            mem.loaded,
            mem.requested,
            mem.far_corner,
            mem.expected_far_corner,
        );
        out.push(ScenarioResult::from_stats(
            &engine_name,
            "memory",
            None,
            mem.loaded as u64,
            date,
            env.clone(),
            None,
            None,
            json!({
                "peak_rss_bytes_before": rss_before,
                "peak_rss_bytes_after": rss_after,
                "delta_bytes": rss_after.saturating_sub(rss_before),
                "requested_cells": profile.memory_cells,
                "loaded_cells": mem.loaded,
                "load_capped_by_budget": mem.capped,
                "load_time_ms": mem.load_time.as_millis() as u64,
                "far_corner_verified": mem.far_corner.as_number(),
                "populated_verified": mem.is_correct(),
            }),
        ));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeEngine;

    fn zero_rss() -> u64 {
        0
    }

    #[test]
    fn run_suite_produces_all_scenarios() {
        let env = Environment::detect("test");
        let results = run_suite(
            FakeEngine::new_blank,
            &Profile::dev(),
            &env,
            "2026-07-01",
            zero_rss,
        );
        // 3 (scrolling designs) + 3 (cascade-visible designs) + 1 (1m) + 1 (fanout)
        //   + 1 (writes) + 1 (memory) = 10 records.
        assert_eq!(results.len(), 10);
        assert!(results.iter().any(|r| r.scenario == "scrolling-read"));
        assert!(results.iter().any(|r| r.scenario == "cascade-1m-chain"));
        assert!(results.iter().any(|r| r.scenario == "memory"));
    }
}
