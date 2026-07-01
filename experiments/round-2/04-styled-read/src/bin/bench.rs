//! `bench` — the SP4 styled viewport-read benchmark (foreground).
//!
//! Usage:  `cargo run --release --bin bench`
//!
//! Measures, at **Excel-max positions**, the cost of reading **value + style**
//! (`get_style_for_cell`) per visible cell over two window sizes:
//!   - `viewport`  (~1,800 cells) — the Phase-1-comparable window (60×30, matching the
//!     harness `Profile::full` viewport), so the added style cost is a clean delta over
//!     the value-only 392 µs p99 baseline.
//!   - `overscan`  (~10,000 cells) — the upper end of the spec's 10³–10⁴ band, so the
//!     GATE (p99 < 2 ms) covers the whole window range.
//!
//! Discipline (functional_spec §5.2 / overview §7):
//!   - **Separate build from the measured op.** A styled band is built ONCE at Excel-max
//!     (values + a mix of per-cell/row/column-band styles), then wrapped read-only; only
//!     the read is timed.
//!   - **Force + assert the reads are real.** Each timed result is black-boxed and, before
//!     recording, we assert it has non-empty values AND resolved styles (refuse a bogus
//!     number for an empty/unstyled grid).
//!   - **Value-only control** in the same positions (harness `read_viewport`) so findings
//!     can quote the value+style vs value-only delta alongside the Phase-1 baseline.
//!   - **p50/p99, env-stamped**, written to `results/`.

use bench_util::{Environment, GateResult, LatencyStats};
use round2_harness::engine::SpreadsheetEngine;
use round2_harness::scenario::pan_path;
use round2_harness::{targets, IronCalcEngine, Profile, Viewport};
use serde_json::json;

use styled_read::{
    count_real, new_model, read_styled_viewport, stamp_styled_band, EXCEL_MAX_COL_0,
    EXCEL_MAX_ROW_0,
};

/// One window size to benchmark: a viewport shape + a human label + the target cell count
/// it approximates (for the record).
struct Window {
    label: &'static str,
    rows: u32,
    cols: u32,
}

impl Window {
    fn cells(&self) -> u32 {
        self.rows * self.cols
    }
}

/// The two windows: the baseline-comparable viewport and the 10⁴ overscan.
const WINDOWS: [Window; 2] = [
    // 60 × 30 = 1,800 — the harness Profile::full viewport (Phase-1 baseline shape).
    Window {
        label: "viewport",
        rows: 60,
        cols: 30,
    },
    // 100 × 100 = 10,000 — the top of the spec's 10³–10⁴ band.
    Window {
        label: "overscan",
        rows: 100,
        cols: 100,
    },
];

/// How many scroll/jump pan steps to time per window. Deterministic path from the harness
/// `pan_path`; 300 steps give a stable p50/p99 (same count as the Phase-1 scrolling read).
const PAN_STEPS: usize = 300;

fn main() -> anyhow::Result<()> {
    std::fs::create_dir_all("results").ok();
    let env = Environment::detect(commit()).with_cpu(cpu_model());
    let date = report_date();

    println!(
        "SP4 bench: styled viewport read at Excel-max ({} rows x {} cols grid).",
        styled_read::EXCEL_ROWS,
        styled_read::EXCEL_COLS
    );
    println!(
        "  reading VALUE + STYLE (get_style_for_cell) per visible cell; \
         gate p99 < {} ms; Phase-1 value-only baseline p99 = 392 us.",
        targets::VIEWPORT_READ_NS / 1_000_000
    );

    let mut records = Vec::new();
    for w in &WINDOWS {
        let rec = bench_window(w, &env, &date)?;
        records.push(rec);
    }

    // Crossover sweep: pinpoint the largest window whose value+style p99 still fits the
    // 2 ms budget, so the finding states exactly where the gate is crossed (not just
    // "1,800 passes / 10,000 fails"). Uses a coarse grid; foreground; same Excel-max fixture.
    let sweep = crossover_sweep()?;

    write_results(&records, &sweep, &env, &date)?;
    println!("\nSP4 bench: wrote results/styled_read.json, results/summary.md, results/env.txt");

    let all_pass = records.iter().all(|r| r.styled_gate_pass);
    if all_pass {
        println!("SP4 GATE: PASS — value+style p99 < 2 ms at Excel-max for every window.");
        Ok(())
    } else {
        // A failed gate is a recorded FINDING, not a crash: results are written above.
        eprintln!("SP4 GATE: FAIL — a window exceeded the 2 ms p99 budget (recorded).");
        std::process::exit(1);
    }
}

/// The recorded outcome for one window.
struct WindowRecord {
    label: String,
    cells: u32,
    row0: u32,
    col0: u32,
    styled_stats: LatencyStats,
    value_only_stats: LatencyStats,
    styled_gate: GateResult,
    styled_gate_pass: bool,
    non_empty_seen: usize,
    styled_seen: usize,
}

/// Builds the Excel-max fixture for `w`, times the styled read and a value-only control
/// over the scroll path, asserts the reads are real, and gates the styled p99.
fn bench_window(w: &Window, _env: &Environment, _date: &str) -> anyhow::Result<WindowRecord> {
    // --- Build (NOT timed): a styled band anchored at Excel-max ---
    // Anchor the band so its far corner sits at the last cell of the grid; the scroll
    // path then pans within [row0, EXCEL_MAX_ROW_0] x [col0, EXCEL_MAX_COL_0].
    let band_rows = w.rows + PAN_STEP_SPAN_ROWS;
    let band_cols = w.cols + PAN_STEP_SPAN_COLS;
    let row0 = EXCEL_MAX_ROW_0 - (band_rows - 1);
    let col0 = EXCEL_MAX_COL_0 - (band_cols - 1);

    println!(
        "\n  window '{}' ({} cells): building styled band at Excel-max [rows {}..={}, cols {}..={}] ...",
        w.label,
        w.cells(),
        row0,
        EXCEL_MAX_ROW_0,
        col0,
        EXCEL_MAX_COL_0
    );

    let mut model = new_model();
    stamp_styled_band(&mut model, row0, col0, band_rows, band_cols);
    // Wrap the finished model read-only: from here the engine is exactly the UI's read
    // path (only &self is used). No recompute needed — the fixture is literals + styles.
    let engine = IronCalcEngine::from_model(model);

    // The scroll/jump path, generated by the harness over a region == the band, then
    // offset to the band's Excel-max origin. Uses the SAME pan_path the Phase-1 scrolling
    // read used, so the scenario is comparable.
    let path = scaled_pan_path(w, band_rows, band_cols, row0, col0);

    // --- CREDIBILITY GUARD: prove the band really holds values + styles before timing.
    let probe_vp = Viewport::new(path[0].0, path[0].1, w.rows, w.cols);
    let probe = read_styled_viewport(&engine, probe_vp);
    let (non_empty_seen, styled_seen) = count_real(&probe);
    assert!(
        non_empty_seen > 0 && styled_seen > 0,
        "window '{}' fixture is not real: non_empty={non_empty_seen}, styled={styled_seen} — \
         refusing to measure an empty/unstyled grid",
        w.label
    );

    // --- Measured op 1: value + STYLE read over the scroll path ---
    let mut styled_samples = Vec::with_capacity(path.len());
    for &(r, c) in &path {
        let vp = Viewport::new(r, c, w.rows, w.cols);
        let (out, dt) = bench_util::time_once(|| read_styled_viewport(&engine, vp));
        std::hint::black_box(&out);
        styled_samples.push(dt);
    }

    // --- Measured op 2: value-ONLY control (harness read_viewport), same positions ---
    let mut value_samples = Vec::with_capacity(path.len());
    for &(r, c) in &path {
        let vp = Viewport::new(r, c, w.rows, w.cols);
        let (out, dt) = bench_util::time_once(|| engine.read_viewport(vp));
        std::hint::black_box(&out);
        value_samples.push(dt);
    }

    let styled_stats =
        LatencyStats::from_durations(&styled_samples).expect("styled samples non-empty");
    let value_only_stats =
        LatencyStats::from_durations(&value_samples).expect("value samples non-empty");

    let styled_gate = GateResult::max(
        format!("styled-read/{}", w.label),
        styled_stats.p99_ns,
        targets::VIEWPORT_READ_NS,
    );
    let styled_gate_pass = styled_gate.is_pass();

    println!(
        "    value+style: p50={} p99={} max={}  [GATE p99<2ms: {}]",
        bench_util::fmt_ns(styled_stats.p50_ns),
        bench_util::fmt_ns(styled_stats.p99_ns),
        bench_util::fmt_ns(styled_stats.max_ns),
        if styled_gate_pass { "PASS" } else { "FAIL" },
    );
    println!(
        "    value-only : p50={} p99={}   (added style cost p99: {})",
        bench_util::fmt_ns(value_only_stats.p50_ns),
        bench_util::fmt_ns(value_only_stats.p99_ns),
        bench_util::fmt_ns(styled_stats.p99_ns.saturating_sub(value_only_stats.p99_ns)),
    );
    println!("    verified real reads: non_empty={non_empty_seen}, styled={styled_seen} (per window probe)");

    Ok(WindowRecord {
        label: w.label.to_string(),
        cells: w.cells(),
        row0,
        col0,
        styled_stats,
        value_only_stats,
        styled_gate,
        styled_gate_pass,
        non_empty_seen,
        styled_seen,
    })
}

/// One crossover-sweep point: a cell count and its value+style p99.
#[derive(Clone)]
struct SweepPoint {
    cells: u32,
    p99_ns: u64,
    pass: bool,
}

/// Result of the crossover sweep: every measured point plus the largest passing cell
/// count (the practical ceiling under the 2 ms budget) and the first failing count.
struct Sweep {
    points: Vec<SweepPoint>,
    largest_pass_cells: Option<u32>,
    first_fail_cells: Option<u32>,
}

/// Sweeps a coarse grid of square-ish windows at Excel-max to find where value+style p99
/// crosses the 2 ms budget. Fewer pan steps per point (the crossover only needs a stable
/// p99, and this keeps the sweep foreground-fast); same styled Excel-max fixture per point.
fn crossover_sweep() -> anyhow::Result<Sweep> {
    // Cell counts spanning the 10³–10⁴ band, dense near the expected ~2.8k crossover.
    const GRID: [(u32, u32); 8] = [
        (40, 30),  // 1,200
        (50, 40),  // 2,000
        (55, 45),  // 2,475
        (55, 50),  // 2,750
        (60, 50),  // 3,000
        (70, 50),  // 3,500
        (80, 60),  // 4,800
        (100, 70), // 7,000
    ];
    const SWEEP_STEPS: usize = 60;

    println!("\n  crossover sweep (largest window under the 2 ms p99 budget):");
    let mut points = Vec::new();
    for (rows, cols) in GRID {
        let w = Window {
            label: "sweep",
            rows,
            cols,
        };
        let band_rows = rows + PAN_STEP_SPAN_ROWS;
        let band_cols = cols + PAN_STEP_SPAN_COLS;
        let row0 = EXCEL_MAX_ROW_0 - (band_rows - 1);
        let col0 = EXCEL_MAX_COL_0 - (band_cols - 1);

        let mut model = new_model();
        stamp_styled_band(&mut model, row0, col0, band_rows, band_cols);
        let engine = IronCalcEngine::from_model(model);

        let profile = Profile {
            region_rows: band_rows,
            region_cols: band_cols,
            viewport_rows: rows,
            viewport_cols: cols,
            pan_steps: SWEEP_STEPS,
            ..Profile::full()
        };
        let path: Vec<(u32, u32)> = pan_path(&profile)
            .into_iter()
            .map(|(r, c)| (row0 + r, col0 + c))
            .collect();

        let mut samples = Vec::with_capacity(path.len());
        for &(r, c) in &path {
            let vp = Viewport::new(r, c, rows, cols);
            let (out, dt) = bench_util::time_once(|| read_styled_viewport(&engine, vp));
            std::hint::black_box(&out);
            samples.push(dt);
        }
        let stats = LatencyStats::from_durations(&samples).expect("sweep samples");
        let cells = w.cells();
        let pass = stats.p99_ns <= targets::VIEWPORT_READ_NS;
        println!(
            "    {:>6} cells: p99={} [{}]",
            cells,
            bench_util::fmt_ns(stats.p99_ns),
            if pass { "PASS" } else { "FAIL" }
        );
        points.push(SweepPoint {
            cells,
            p99_ns: stats.p99_ns,
            pass,
        });
    }

    let largest_pass_cells = points.iter().filter(|p| p.pass).map(|p| p.cells).max();
    let first_fail_cells = points.iter().filter(|p| !p.pass).map(|p| p.cells).min();
    if let Some(c) = largest_pass_cells {
        println!("    -> largest window under 2 ms p99: ~{c} cells");
    }
    Ok(Sweep {
        points,
        largest_pass_cells,
        first_fail_cells,
    })
}

/// Extra rows/cols the pan path sweeps beyond the viewport, so the scroll visits distinct
/// windows (not the same corner repeatedly). Kept modest so the band stays fully inside
/// the grid even for the overscan window.
const PAN_STEP_SPAN_ROWS: u32 = 400;
const PAN_STEP_SPAN_COLS: u32 = 200;

/// Builds the scroll/jump path for `w` by reusing the harness `pan_path` over a region ==
/// the styled band, then offsetting each step to the band's Excel-max origin. This keeps
/// the SAME deterministic scroll pattern Phase-1 used while pinning every read at the top
/// end of the grid.
fn scaled_pan_path(
    w: &Window,
    band_rows: u32,
    band_cols: u32,
    row0: u32,
    col0: u32,
) -> Vec<(u32, u32)> {
    let profile = Profile {
        region_rows: band_rows,
        region_cols: band_cols,
        viewport_rows: w.rows,
        viewport_cols: w.cols,
        pan_steps: PAN_STEPS,
        ..Profile::full()
    };
    pan_path(&profile)
        .into_iter()
        .map(|(r, c)| (row0 + r, col0 + c))
        .collect()
}

/// Writes the env-stamped JSON, a human summary, and the env stamp.
fn write_results(
    records: &[WindowRecord],
    sweep: &Sweep,
    env: &Environment,
    date: &str,
) -> anyhow::Result<()> {
    let baseline_value_only_p99_ns: u64 = 392_422; // Phase-1 ironcalc scrolling-read/D2.

    let windows: Vec<_> = records
        .iter()
        .map(|r| {
            json!({
                "window": r.label,
                "cells": r.cells,
                "excel_max_origin": { "row0": r.row0, "col0": r.col0 },
                "excel_max_far_corner": { "row": EXCEL_MAX_ROW_0, "col": EXCEL_MAX_COL_0 },
                "value_plus_style": {
                    "p50_ns": r.styled_stats.p50_ns,
                    "p99_ns": r.styled_stats.p99_ns,
                    "max_ns": r.styled_stats.max_ns,
                    "mean_ns": r.styled_stats.mean_ns,
                    "count": r.styled_stats.count,
                },
                "value_only_control": {
                    "p50_ns": r.value_only_stats.p50_ns,
                    "p99_ns": r.value_only_stats.p99_ns,
                },
                "added_style_cost_p99_ns": r.styled_stats.p99_ns.saturating_sub(r.value_only_stats.p99_ns),
                "gate_target_ns": targets::VIEWPORT_READ_NS,
                "gate_p99_under_2ms": r.styled_gate_pass,
                "gate_summary": r.styled_gate.summary(),
                "verified_real": {
                    "non_empty_values": r.non_empty_seen,
                    "styled_cells": r.styled_seen,
                },
            })
        })
        .collect();

    let summary = json!({
        "experiment": "SP4 -- styled viewport read at scale",
        "engine": "ironcalc",
        "engine_version": "0.7.1",
        "read": "value + get_style_for_cell per visible cell (per-cell loop; no native bulk style read)",
        "excel_max": { "rows": styled_read::EXCEL_ROWS, "cols": styled_read::EXCEL_COLS },
        "pan_steps": PAN_STEPS,
        "phase1_value_only_baseline_p99_ns": baseline_value_only_p99_ns,
        "gate_target_ns": targets::VIEWPORT_READ_NS,
        "all_windows_gate_pass": records.iter().all(|r| r.styled_gate_pass),
        "windows": windows,
        "crossover_sweep": {
            "note": "largest window whose value+style p99 stays under 2 ms, at Excel-max",
            "largest_pass_cells": sweep.largest_pass_cells,
            "first_fail_cells": sweep.first_fail_cells,
            "points": sweep.points.iter().map(|p| json!({
                "cells": p.cells, "p99_ns": p.p99_ns, "pass": p.pass,
            })).collect::<Vec<_>>(),
        },
        "environment": {
            "os": env.os, "arch": env.arch, "cores": env.cores,
            "cpu": env.cpu, "commit": env.commit,
        },
        "date": date,
    });
    std::fs::write(
        "results/styled_read.json",
        serde_json::to_string_pretty(&summary)?,
    )?;

    // Human-readable summary table.
    let mut md = String::new();
    md.push_str("# SP4 — styled viewport read: recorded summary\n\n");
    md.push_str(&format!(
        "IronCalc 0.7.1. Value + style (`get_style_for_cell`) per visible cell, at \
         Excel-max ({}x{}). Phase-1 value-only baseline p99 = {}. Gate: p99 < {}.\n\n",
        styled_read::EXCEL_ROWS,
        styled_read::EXCEL_COLS,
        bench_util::fmt_ns(baseline_value_only_p99_ns),
        bench_util::fmt_ns(targets::VIEWPORT_READ_NS),
    ));
    md.push_str("| window | cells | value+style p50 | value+style p99 | value-only p99 | added style p99 | GATE p99<2ms |\n");
    md.push_str("|--------|-------|-----------------|-----------------|----------------|-----------------|--------------|\n");
    for r in records {
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} |\n",
            r.label,
            r.cells,
            bench_util::fmt_ns(r.styled_stats.p50_ns),
            bench_util::fmt_ns(r.styled_stats.p99_ns),
            bench_util::fmt_ns(r.value_only_stats.p99_ns),
            bench_util::fmt_ns(
                r.styled_stats
                    .p99_ns
                    .saturating_sub(r.value_only_stats.p99_ns)
            ),
            if r.styled_gate_pass { "PASS" } else { "FAIL" },
        ));
    }
    md.push_str("\n## Crossover sweep (largest window under 2 ms p99, at Excel-max)\n\n");
    md.push_str("| cells | value+style p99 | GATE p99<2ms |\n");
    md.push_str("|-------|-----------------|--------------|\n");
    for p in &sweep.points {
        md.push_str(&format!(
            "| {} | {} | {} |\n",
            p.cells,
            bench_util::fmt_ns(p.p99_ns),
            if p.pass { "PASS" } else { "FAIL" },
        ));
    }
    if let (Some(pass), Some(fail)) = (sweep.largest_pass_cells, sweep.first_fail_cells) {
        md.push_str(&format!(
            "\n**Crossover:** largest window under 2 ms p99 ≈ **{pass} cells**; first failing ≈ **{fail} cells**.\n"
        ));
    }
    std::fs::write("results/summary.md", md)?;

    // Env stamp.
    let env_txt = format!(
        "SP4 environment\n\
         os={} arch={} cores={} cpu={}\n\
         commit={}\n\
         ironcalc=0.7.1 (pinned, same as round-2 harness)\n\
         excel_max_rows={} excel_max_cols={}\n\
         pan_steps={} windows={:?}\n",
        env.os,
        env.arch,
        env.cores,
        env.cpu,
        env.commit,
        styled_read::EXCEL_ROWS,
        styled_read::EXCEL_COLS,
        PAN_STEPS,
        WINDOWS
            .iter()
            .map(|w| (w.label, w.cells()))
            .collect::<Vec<_>>(),
    );
    std::fs::write("results/env.txt", env_txt)?;
    Ok(())
}

fn commit() -> String {
    std::env::var("SP4_COMMIT").unwrap_or_else(|_| "unknown".to_string())
}

fn report_date() -> String {
    std::env::var("SP4_DATE").unwrap_or_else(|_| "2026-07-01".to_string())
}

fn cpu_model() -> String {
    std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("model name"))
                .and_then(|l| l.split(':').nth(1))
                .map(|v| v.trim().to_string())
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaled_pan_path_stays_in_band_and_at_excel_max() {
        let w = &WINDOWS[0];
        let band_rows = w.rows + PAN_STEP_SPAN_ROWS;
        let band_cols = w.cols + PAN_STEP_SPAN_COLS;
        let row0 = EXCEL_MAX_ROW_0 - (band_rows - 1);
        let col0 = EXCEL_MAX_COL_0 - (band_cols - 1);
        let path = scaled_pan_path(w, band_rows, band_cols, row0, col0);
        assert_eq!(path.len(), PAN_STEPS);
        for (r, c) in path {
            // Each viewport lies fully inside the band ...
            assert!(r >= row0 && r + w.rows <= EXCEL_MAX_ROW_0 + 1);
            assert!(c >= col0 && c + w.cols <= EXCEL_MAX_COL_0 + 1);
            // ... and the band is anchored at the extreme top of the grid.
            assert!(r >= EXCEL_MAX_ROW_0 - band_rows);
        }
    }
}
