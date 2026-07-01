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

/// How many repetitions of the (expensive) cascade scenarios to time. Kept modest
/// because on a non-incremental engine each rep is a full 1M-cell re-evaluate (~2 s);
/// a dozen reps still give an informative p50/p99 within one process while bounding
/// the total run.
const CASCADE_REPS: usize = 12;
/// Edits to time for the change→visible scenario (same rationale as CASCADE_REPS).
const VISIBLE_EDITS: usize = 12;

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
        let samples =
            scenario::cascade_to_visible_update(&mut engine, profile, design, VISIBLE_EDITS);
        let stats = LatencyStats::from_durations(&samples);
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
            profile.chain_len as u64,
            date,
            env.clone(),
            stats,
            gate,
            json!({ "chain_len": profile.chain_len }),
        ));
    }

    // --- Scenario 3a: 1M =PREV+1 cascade recompute, gate <= 100 ms ---
    {
        let mut engine = make();
        let samples =
            scenario::cascade_recompute_chain(&mut engine, profile.chain_len, CASCADE_REPS);
        let stats = LatencyStats::from_durations(&samples);
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
            json!({ "chain_len": profile.chain_len }),
        ));
    }

    // --- Scenario 3b: wide fan-out recompute (discovery, gate at frame budget) ---
    {
        let mut engine = make();
        let samples = scenario::cascade_recompute_fanout(
            &mut engine,
            profile.fanout_sources,
            profile.fanout_dependents,
            CASCADE_REPS,
        );
        let stats = LatencyStats::from_durations(&samples);
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

    // --- Scenario 5: memory load + edit (discovery: peak RSS) ---
    {
        let rss_before = peak_rss();
        let mut engine = make();
        scenario::memory_load_and_edit(&mut engine, profile.memory_cells);
        let rss_after = peak_rss();
        // Keep the engine alive across the RSS read.
        std::hint::black_box(&engine);
        out.push(ScenarioResult::from_stats(
            &engine_name,
            "memory",
            None,
            profile.memory_cells as u64,
            date,
            env.clone(),
            None,
            None,
            json!({
                "peak_rss_bytes_before": rss_before,
                "peak_rss_bytes_after": rss_after,
                "delta_bytes": rss_after.saturating_sub(rss_before),
                "cells": profile.memory_cells,
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
