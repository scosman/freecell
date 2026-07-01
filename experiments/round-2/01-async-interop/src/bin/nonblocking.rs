//! `nonblocking` — the SP1 non-blocking render-loop harness + GATES (functional_spec
//! SP1 GATE, architecture §4.4).
//!
//! Drives a headless "render loop" (a driver ticking at 60 fps cadence; **NO GPUI**)
//! against an [`EvalWorker`] that owns a big IronCalc model and runs all evaluation on a
//! worker thread. While a 10⁶–10⁷-cell eval is in flight the loop must keep ticking
//! cheaply.
//!
//! Measures + asserts:
//! - **GATE 1 (render non-blocking):** per-tick synchronous work p99 < one frame
//!   (< 8.3 ms; hard-fail > 16.6 ms) even during the eval.
//! - **GATE 2 (coalescing):** a burst of N rapid edits ⇒ ≤ a small bounded number of
//!   `evaluate()` runs.
//! - **DISCOVERY:** staleness window (edit → first tick showing the fresh visible
//!   value); snapshot (`to_bytes`) cost on the big model.
//!
//! Also runs the two API probes (diff-list = edit-sites only; snapshot round-trips) and
//! records them, since the seam design rests on those findings.
//!
//! ## Usage
//! ```text
//! cargo run --release --bin nonblocking                    # default 10^6 model
//! cargo run --release --bin nonblocking -- --size 10000000 # 10^7 (run alone; heavy)
//! ```
//! Foreground with `timeout`. Exits non-zero if a hard GATE fails.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use bench_util::{fmt_ns, BenchResult, Environment, GateResult, LatencyStats};
use round2_harness::{cpu_model, Viewport};
use serde_json::json;
use sp1_async_interop::probes::{diff_list_is_edit_sites_only, snapshot_roundtrip};
use sp1_async_interop::seam::{Edit, EvalWorker};
use sp1_async_interop::shapes::{self, ReArm, Shape};

const REPORT_DATE: &str = "2026-07-01";
const FRAME_BUDGET_NS: u64 = 8_300_000; // one 120 Hz-ish frame target (< 8.3 ms)
const HARD_FAIL_NS: u64 = 16_600_000; // hard fail above ~two frames (16.6 ms)
const FRAME_PERIOD: Duration = Duration::from_micros(16_667); // ~60 fps tick cadence
const FRAMES: usize = 600; // ~10 s of simulated rendering
const BURST_EDITS: u32 = 30; // rapid edits fired mid-run

fn results_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("results")
}

fn parse_size() -> u64 {
    let mut it = std::env::args().skip(1);
    let mut size = 1_000_000u64;
    while let Some(a) = it.next() {
        if a == "--size" {
            size = it
                .next()
                .expect("--size needs a value")
                .parse()
                .expect("size");
        }
    }
    size
}

/// The model shape used for the harness. Deep-serial is the default (1 edit → long
/// cascade to a single visible tail — the motivating case), but it caps at 10⁶ (Excel
/// row limit); pass `--shape volatile` to drive a genuine 10⁷-cell eval in flight.
fn parse_shape() -> Shape {
    let mut it = std::env::args().skip(1);
    let mut shape = Shape::DeepSerial;
    while let Some(a) = it.next() {
        if a == "--shape" {
            let v = it.next().expect("--shape needs a value");
            shape = Shape::from_id(&v).unwrap_or_else(|| panic!("unknown shape {v}"));
        }
    }
    shape
}

fn main() -> anyhow::Result<()> {
    let size = parse_size();
    let shape = parse_shape();
    let env = Environment::detect("HEAD").with_cpu(cpu_model());
    let dir = results_dir();
    std::fs::create_dir_all(&dir)?;

    // --- API probes the seam design rests on (recorded up front). ---
    let diff_short = diff_list_is_edit_sites_only(10);
    let diff_long = diff_list_is_edit_sites_only(1000);
    let snapshot_ok = snapshot_roundtrip();
    println!(
        "PROBE diff-list edit-sites-only: chain 10 -> {} cascaded, {} diff bytes; chain 1000 -> {} cascaded, {} diff bytes (no evaluated-cell stream)",
        diff_short.cascaded_cells,
        diff_short.diff_bytes_for_one_edit,
        diff_long.cascaded_cells,
        diff_long.diff_bytes_for_one_edit,
    );
    println!("PROBE snapshot to_bytes/from_bytes round-trips: {snapshot_ok}");

    // --- Build the expensive-eval model. Deep-serial (default) is the motivating case:
    // one edit → a long cascade to a single visible tail cell, few cells on screen. Its
    // 10⁶ eval (~1.2 s) is the known-FAIL recompute we must keep OFF the render loop.
    // For a genuine 10⁷-cell eval in flight, pass `--shape volatile`. ---
    println!(
        "BUILD {} size={size} (this is the expensive-eval model)...",
        shape.id()
    );
    let build_start = Instant::now();
    let built = shapes::build(shape, size);
    let populated = built.populated_cells;
    let (tail_sheet, tail_row, tail_col) = built.tail;
    // The cell the edit burst writes so the visible tail actually changes: the tail's
    // own re-arm precedent (head of the chain / referenced literal). Volatile has no
    // precedent (re-rolls every eval), so we just edit the tail's own cell.
    let (edit_sheet, edit_row, edit_col) = match built.rearm {
        ReArm::BumpSeed { sheet, row, col } => (sheet, row, col),
        ReArm::None => (tail_sheet, tail_row, tail_col),
    };
    let build_secs = build_start.elapsed().as_secs_f64();
    println!("  built {populated} cells in {build_secs:.2}s");

    // Visible viewport = a small window ending at the tail cell (the on-screen cells).
    let vp = Viewport::new(tail_row.saturating_sub(4), tail_col, 5, 1);

    let worker = EvalWorker::spawn(built.model, vp);

    // Wait for the initial eval so we have last-known values to paint.
    wait_until(|| worker.eval_count() >= 1, Duration::from_secs(120));
    let evals_before_burst = worker.eval_count();
    println!("  initial eval done (eval_count={evals_before_burst})");

    // --- Run the render loop for FRAMES ticks, firing a burst of rapid edits partway
    // so an eval is in flight while the loop keeps ticking. ---
    let mut tick_durations: Vec<Duration> = Vec::with_capacity(FRAMES);
    let mut ticks_during_eval: Vec<Duration> = Vec::new();
    let mut burst_fired = false;
    let mut burst_at: Option<Instant> = None;
    let mut staleness: Option<Duration> = None;
    let mut scroll_row = tail_row.saturating_sub(4);

    for frame in 0..FRAMES {
        let frame_start = Instant::now();

        // Fire the edit burst around frame 60 (~1 s in), while the initial state is
        // painted; these rapid edits must coalesce.
        if frame == 60 && !burst_fired {
            let now = Instant::now();
            for i in 0..BURST_EDITS {
                worker.enqueue_edit(Edit {
                    sheet: edit_sheet,
                    row: edit_row,
                    col: edit_col,
                    input: format!("{}", i + 1),
                    enqueued_at: now,
                });
            }
            burst_fired = true;
            burst_at = Some(now);
        }

        // --- The tick's SYNCHRONOUS work (this is what GATE 1 measures). ---
        let tick_start = Instant::now();
        // 1. Read the latest published viewport (cheap: O(viewport), short-held lock).
        let published = worker.latest_published();
        // 2. Show "recalculating..." if an eval is in flight (never blocks).
        let _recalculating = worker.is_evaluating();
        // 3. Advance a synthetic scroll and update the visible viewport (cheap message).
        scroll_row = scroll_row.wrapping_add(1) % (tail_row + 1).max(1);
        if frame % 30 == 0 {
            worker.set_viewport(Viewport::new(
                scroll_row.min(tail_row.saturating_sub(4)),
                tail_col,
                5,
                1,
            ));
        }
        // 4. "Paint": touch the published values so the read isn't optimized away.
        let _painted = published.values.len();
        let tick_cost = tick_start.elapsed();
        tick_durations.push(tick_cost);
        if worker.is_evaluating() {
            ticks_during_eval.push(tick_cost);
        }

        // Staleness: first tick after the burst whose published generation reflects the
        // edit (>= the generation stamped when the edit became visible).
        if staleness.is_none() {
            if let (Some(bat), gen) = (burst_at, worker.last_edit_visible_gen()) {
                if gen != 0 && published.generation >= gen {
                    staleness = Some(bat.elapsed());
                }
            }
        }

        // Sleep to the next frame boundary (simulating vsync). If the tick already blew
        // the budget this would not sleep — but the whole point is it never does.
        let elapsed = frame_start.elapsed();
        if elapsed < FRAME_PERIOD {
            std::thread::sleep(FRAME_PERIOD - elapsed);
        }
    }

    // If the eval is still running or staleness wasn't captured within the loop, wait a
    // bounded time for it to settle and capture the final staleness.
    if staleness.is_none() {
        if let Some(bat) = burst_at {
            wait_until(
                || {
                    let gen = worker.last_edit_visible_gen();
                    gen != 0 && worker.latest_published().generation >= gen
                },
                Duration::from_secs(120),
            );
            staleness = Some(bat.elapsed());
        }
    }

    let evals_for_burst = worker.eval_count() - evals_before_burst;

    // --- Snapshot cost on the big model (the clone-cost discovery). Shut the worker
    // down to reclaim the model, then time to_bytes(). ---
    let model = worker.shutdown();
    let (snapshot_bytes, snapshot_time) = {
        let start = Instant::now();
        let bytes = model.to_bytes();
        (bytes.len(), start.elapsed())
    };

    // --- Compute stats + gates. ---
    let all_stats = LatencyStats::from_durations(&tick_durations).expect("ticks");
    let eval_tick_stats = LatencyStats::from_durations(&ticks_during_eval);

    // GATE 1: the strict test is per-tick work WHILE an eval is in flight. If (on a fast
    // box) the eval finished before any tick sampled it, fall back to the all-ticks p99
    // (which is an upper bound on the during-eval cost anyway).
    let gate1_measured = match eval_tick_stats {
        Some(s) => s.p99_ns,
        None => all_stats.p99_ns,
    };
    let gate1 = GateResult::max(
        "render_tick_p99_during_eval",
        gate1_measured,
        FRAME_BUDGET_NS,
    );
    let gate1_hardfail = gate1_measured > HARD_FAIL_NS;

    // GATE 2: N rapid edits => <= a small bounded number of evals. Bound = 2 (at most
    // one already in-flight + one coalesced settle).
    let coalesce_bound = 2u64;
    let gate2 = GateResult::max("coalesce_eval_count", evals_for_burst, coalesce_bound);

    // --- Report. ---
    let tag = format!("{}_{}", shape.id(), size);
    println!(
        "\n=== SP1 non-blocking harness ({}, {populated} cells) ===",
        shape.id()
    );
    println!(
        "render ticks: n={} p50={} p99={} max={}",
        all_stats.count,
        fmt_ns(all_stats.p50_ns),
        fmt_ns(all_stats.p99_ns),
        fmt_ns(all_stats.max_ns)
    );
    match eval_tick_stats {
        Some(s) => println!(
            "ticks DURING eval: n={} p50={} p99={} max={}",
            s.count,
            fmt_ns(s.p50_ns),
            fmt_ns(s.p99_ns),
            fmt_ns(s.max_ns)
        ),
        None => println!("ticks DURING eval: none sampled (eval finished between ticks); using all-ticks p99 as upper bound"),
    }
    println!(
        "GATE1 {} (hardfail>{}): {}",
        gate1.summary(),
        fmt_ns(HARD_FAIL_NS),
        if gate1_hardfail { "HARD FAIL" } else { "ok" }
    );
    println!(
        "GATE2 {} ({} rapid edits -> {} evals)",
        gate2.summary(),
        BURST_EDITS,
        evals_for_burst
    );
    if let Some(s) = staleness {
        println!("staleness window (edit -> visible fresh): {:.2?}", s);
    }
    println!(
        "snapshot to_bytes: {} bytes in {} (the 'clone cost' if snapshot-publish were used)",
        snapshot_bytes,
        fmt_ns(snapshot_time.as_nanos() as u64)
    );

    // --- Persist results. ---
    let render_result = BenchResult::new(
        "render_loop_nonblocking",
        populated,
        REPORT_DATE,
        env.clone(),
    )
    .with_stats(all_stats)
    .with_gate(gate1.clone())
    .with_extra(json!({
        "model_shape": shape.id(),
        "populated_cells": populated,
        "frames": FRAMES,
        "frame_budget_ns": FRAME_BUDGET_NS,
        "hard_fail_ns": HARD_FAIL_NS,
        "ticks_during_eval": eval_tick_stats.map(|s| json!({
            "count": s.count, "p50_ns": s.p50_ns, "p99_ns": s.p99_ns, "max_ns": s.max_ns
        })),
        "gate1_hardfail": gate1_hardfail,
        "build_secs": build_secs,
    }));
    render_result.write_json(dir.join(format!("gate_render_loop_{tag}.json")))?;

    let coalesce_result = BenchResult::new("coalesce", populated, REPORT_DATE, env.clone())
        .with_gate(gate2.clone())
        .with_extra(json!({
            "burst_edits": BURST_EDITS,
            "evals_for_burst": evals_for_burst,
            "coalesce_bound": coalesce_bound,
        }));
    coalesce_result.write_json(dir.join(format!("gate_coalesce_{tag}.json")))?;

    let staleness_doc = json!({
        "experiment": "SP1 staleness window + snapshot cost",
        "date": REPORT_DATE,
        "model_shape": shape.id(),
        "populated_cells": populated,
        "staleness_window_ns": staleness.map(|s| s.as_nanos() as u64),
        "staleness_window": staleness.map(|s| format!("{s:.2?}")),
        "snapshot_to_bytes_bytes": snapshot_bytes,
        "snapshot_to_bytes_ns": snapshot_time.as_nanos() as u64,
        "snapshot_to_bytes": fmt_ns(snapshot_time.as_nanos() as u64),
    });
    std::fs::write(
        dir.join(format!("staleness_{tag}.json")),
        serde_json::to_string_pretty(&staleness_doc)?,
    )?;

    let probes_doc = json!({
        "experiment": "SP1 API probes",
        "date": REPORT_DATE,
        "model_is_send": "asserted at compile time (seam::assert_model_send)",
        "diff_list_edit_sites_only": {
            "chain_10": { "cascaded_cells": diff_short.cascaded_cells, "one_edit_diff_bytes": diff_short.diff_bytes_for_one_edit },
            "chain_1000": { "cascaded_cells": diff_long.cascaded_cells, "one_edit_diff_bytes": diff_long.diff_bytes_for_one_edit },
            "finding": "UserModel diff-list records only edit-sites; one-edit diff size does NOT scale with cascade -> IronCalc exposes no evaluated-cell change stream."
        },
        "snapshot_roundtrips": snapshot_ok,
    });
    std::fs::write(
        dir.join(format!("api_probes_{tag}.json")),
        serde_json::to_string_pretty(&probes_doc)?,
    )?;

    println!("\nWrote results to {}", dir.display());

    // Self-checking: exit non-zero on a hard gate failure.
    let mut failed = false;
    if gate1_hardfail {
        eprintln!(
            "HARD FAIL: render tick p99 exceeded {}",
            fmt_ns(HARD_FAIL_NS)
        );
        failed = true;
    }
    if !gate1.is_pass() {
        eprintln!("GATE1 FAIL: render tick p99 exceeded frame budget");
        failed = true;
    }
    if !gate2.is_pass() {
        eprintln!("GATE2 FAIL: rapid edits did not coalesce");
        failed = true;
    }
    if !snapshot_ok {
        eprintln!("PROBE FAIL: snapshot did not round-trip");
        failed = true;
    }
    if failed {
        std::process::exit(1);
    }
    Ok(())
}

/// Busy-waits (yielding) until `cond` holds or `timeout` elapses. Returns whether it
/// held. Used only for orchestration/teardown, never on the measured render path.
fn wait_until<F: Fn() -> bool>(cond: F, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    cond()
}
