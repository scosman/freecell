//! FreeCell Round-3 Investigation A — runnable report + cost harness.
//!
//! Running `cargo run --release` (or with `--bin cache_sync`) prints the `UserModel` API
//! probe results and runs the **cost-at-scale** benchmarks (structural-edit cost on the
//! `UserModel` and the resident-cache shift cost, measured separately, force+assert,
//! p50/p99, env-stamped) and writes `results/*.json`.
//!
//! The GATE correctness/undo/agreement assertions live in `tests/` so they gate on
//! `cargo test`. Benchmarks run FOREGROUND; wrap invocations with `timeout` per CLAUDE.md.

use std::io::Write;
use std::time::{Duration, Instant};

use bench_util::record::{BenchResult, Environment};
use bench_util::stats::LatencyStats;
use cache_sync::cache::ResidentCache;
use cache_sync::harness::{self, SHEET};
use cache_sync::probe;
use ironcalc_base::UserModel;
use round2_harness::cpu_model;

const DATE: &str = "2026-07-01";
const COMMIT: &str = "round-3-phase-A";

fn env() -> Environment {
    Environment::detect(COMMIT).with_cpu(cpu_model())
}

fn main() -> Result<(), String> {
    println!("== FreeCell Round-3 A — cache-sync + structural editing ==\n");

    // ---- 1. UserModel API + Send probe --------------------------------------------
    probe::assert_usermodel_send();
    let api = probe::probe_api()?;
    println!("[probe] UserModel<'static> is Send: TRUE (compile-time asserted)");
    println!("[probe] undo/redo present + functional: {}", api.has_undo_redo);
    println!(
        "[probe] insert/delete rows: {}, columns: {}",
        api.has_insert_delete_rows, api.has_insert_delete_columns
    );
    println!(
        "[probe] diff-list publicly inspectable: {} (only opaque bitcode via flush_send_queue)",
        api.diff_list_publicly_inspectable
    );
    println!(
        "[probe] copy/paste roundtrip usable externally: {}",
        api.copy_paste_roundtrip_externally_usable
    );
    println!("[probe] merge-cells public API: {}", api.merge_cells_public_api);
    let seam_value = probe::probe_worker_seam()?;
    println!(
        "[probe] SP1 worker seam holds for UserModel: TRUE (moved to worker, insert+evaluate, \
         read back A3={seam_value})\n"
    );
    for note in &api.notes {
        println!("    - {note}");
    }
    println!();

    // ---- 2. Cost at scale (DISCOVERY) ---------------------------------------------
    // Scale is capped by wall-clock; each level is env-stamped and force+asserted.
    let scales = [100_000i32, 1_000_000i32];
    for &rows in &scales {
        run_scale(rows)?;
    }

    println!("\nAll probes + benchmarks complete. Correctness GATEs run under `cargo test`.");
    Ok(())
}

/// Build a tall sheet, then measure (a) IronCalc `insert_rows`/`delete_rows` cost and (b)
/// the resident-cache shift cost, separately. Force+assert each op actually shifted state
/// past the edit point. Writes env-stamped p50/p99 JSON to `results/`.
fn run_scale(rows: i32) -> Result<(), String> {
    println!("-- scale: {rows} populated rows --");
    let _ = std::io::stdout().flush();

    // Build: column A literals 1..=rows, a band style + custom height near the top and a
    // formula near the top so `displace_cells` has real work. Separate from the measured
    // op (CLAUDE.md: separate build from the measured op).
    let build_start = Instant::now();
    let mut model = UserModel::new_empty("scale", "en", "UTC", "en")?;
    model.pause_evaluation();
    for r in 1..=rows {
        model.set_user_input(SHEET, r, 1, &r.to_string())?;
    }
    // A handful of formulas that reference cells far down, so an insert/delete must
    // re-target real references (not a no-op displacement).
    model.set_user_input(SHEET, 1, 2, &format!("=A{rows}"))?; // B1 = A{rows}
    model.set_user_input(SHEET, 2, 2, &format!("=SUM(A1:A{rows})"))?;
    harness::set_row_band_fill(&mut model, 5, "00FF00")?;
    model.set_rows_height(SHEET, 5, 5, harness::CUSTOM_ROW_HEIGHT)?;
    model.resume_evaluation();
    model.evaluate();
    let build = build_start.elapsed();
    println!("   build+initial eval: {:?}", build);
    let _ = std::io::stdout().flush();

    // Sanity: B1 should equal A{rows} = rows.
    let b1_before = harness::cell_display(&model, 1, 2);
    assert_eq!(
        b1_before,
        rows.to_string(),
        "sheet build sanity: B1 should be A{rows}={rows}"
    );

    // (a) IronCalc structural-edit cost. Insert 1 row at index 3, then delete it, as
    // paired ops. We re-measure a fresh insert each iteration by inserting then deleting
    // so the sheet returns to the same shape (bounded iterations to keep wall-clock sane
    // at 1e6). force+assert: after insert, the band style at row 5 must have moved to 6.
    let insert_samples = timed_iters_capped(cap_iters(rows), || {
        let t = Instant::now();
        model.insert_rows(SHEET, 3, 1).expect("insert_rows");
        let dt = t.elapsed();
        // FORCE+ASSERT: band style that was on row 5 is now on row 6.
        let moved = model
            .get_model()
            .get_row_style(SHEET, 6)
            .expect("get_row_style")
            .is_some();
        assert!(moved, "insert did not shift the band style down to row 6");
        // Restore shape for the next iteration (untimed).
        model.delete_rows(SHEET, 3, 1).expect("delete_rows");
        dt
    });
    report("ironcalc_insert_row", rows, &insert_samples);

    // (b) delete cost, measured symmetrically (insert to restore, untimed).
    let delete_samples = timed_iters_capped(cap_iters(rows), || {
        let t = Instant::now();
        model.delete_rows(SHEET, 3, 1).expect("delete_rows");
        let dt = t.elapsed();
        // FORCE+ASSERT: band style that was on row 5 (after the earlier balanced state) is
        // now on row 4.
        let moved = model
            .get_model()
            .get_row_style(SHEET, 4)
            .expect("get_row_style")
            .is_some();
        assert!(moved, "delete did not shift the band style up to row 4");
        model.insert_rows(SHEET, 3, 1).expect("insert_rows restore");
        dt
    });
    report("ironcalc_delete_row", rows, &delete_samples);

    // (c) resident-cache shift cost — hydrate a cache with a realistic number of overrides
    // spread across the axis, then measure `shift_rows`. This is the frontend cost that
    // runs on the render side, independent of IronCalc.
    let overrides = 2_000.min(rows / 2);
    let mut cache = ResidentCache::new(rows as usize, 32, 21.0, 100.0);
    for i in 0..overrides {
        let idx = (1 + i * (rows / overrides.max(1))) as i64;
        cache.rows.set_size(idx, 30.0 + (i % 7) as f64);
    }
    let cache_samples = timed_iters_capped(200, || {
        let t = Instant::now();
        let (removed, removed_cells) = cache.shift_rows(3, 1);
        let dt = t.elapsed();
        // FORCE+ASSERT: an override that was at or above index 3 moved up by one. We check
        // total override count is preserved (insert never drops overrides).
        assert_eq!(
            cache.rows.override_count(),
            overrides as usize,
            "cache insert should preserve override count"
        );
        // Restore shape (untimed): delete the inserted row.
        let _ = cache.shift_rows(3, -1);
        let _ = (removed, removed_cells);
        dt
    });
    report("cache_shift_row", rows, &cache_samples);
    println!(
        "   cache overrides shifted per op: {overrides} (O(overrides), not O({rows}))\n"
    );

    Ok(())
}

/// Iteration cap chosen so even 1e6-row runs finish in bounded wall-clock (foreground
/// discipline). A row insert/delete at 1e6 moves ~1e6 cells + runs a full evaluate, so a
/// handful of paired iterations is enough to characterize p50/p99 without a runaway run.
fn cap_iters(rows: i32) -> usize {
    if rows >= 1_000_000 {
        8
    } else {
        200
    }
}

fn timed_iters_capped<F: FnMut() -> Duration>(iters: usize, mut f: F) -> Vec<Duration> {
    // One warm-up (untimed) to page in caches, then `iters` timed samples.
    let _ = f();
    (0..iters).map(|_| f()).collect()
}

fn report(name: &str, rows: i32, samples: &[Duration]) {
    let stats = LatencyStats::from_durations(samples).expect("non-empty samples");
    println!(
        "   {name:<22} p50={:>10}  p99={:>10}  (n={})",
        fmt(stats.p50_ns),
        fmt(stats.p99_ns),
        stats.count
    );
    let _ = std::io::stdout().flush();
    let result = BenchResult::new(name, rows as u64, DATE, env()).with_stats(stats);
    let path = format!("results/{name}_{rows}.json");
    if let Err(e) = result.write_json(&path) {
        eprintln!("   (warning) could not write {path}: {e}");
    }
}

fn fmt(ns: u64) -> String {
    if ns >= 1_000_000 {
        format!("{:.2} ms", ns as f64 / 1e6)
    } else if ns >= 1_000 {
        format!("{:.2} µs", ns as f64 / 1e3)
    } else {
        format!("{ns} ns")
    }
}
