//! Phase-12 perf harness binary — the POC "Run Test" scenario against the **real**
//! `GridView` over a **1M×100 styled** engine-backed fixture (`architecture.md §4, §9`).
//!
//! Run FOREGROUND with `timeout`, never backgrounded (`CLAUDE.md` benchmark convention). The
//! `render-tests/scripts/perf.sh` wrapper runs this under `xvfb-run` + lavapipe.
//!
//! What it measures (representative under lavapipe): the **CPU render-build path** — data
//! resolution + element construction, per the POC's methodology — driven by the scripted
//! scroll/jump sequence over the real engine's `Publication` + `SheetCaches`; plus the
//! **engine-call counter**, asserted to stay flat across the whole scroll sweep (the
//! zero-engine-calls-on-scroll gate) with a **negative control** proving the counter can climb.
//! GPU present + gpui layout/shaping run after each `render()` returns and, under software
//! Vulkan, are unrepresentative — not measured here (a macos-verify concern).
//!
//! Modes:
//! - default: report p50/p99 + write JSON (calibration).
//! - `--gate`: additionally exit non-zero if any committed buffered threshold or the
//!   zero-engine-calls gate is breached (the required CI gate).

use std::collections::HashSet;
use std::path::PathBuf;

use gpui::{App, AppContext as _};
use gpui_platform::application;

use freecell_app::grid::{GridEventSink, GridView};
use freecell_core::perf::{
    fmt_ns, Harness, PerfConfig, RunReport, CELL_LOAD_TARGET_NS, FRAME_TARGET_NS, FRAME_WORST_NS,
};
use freecell_engine::engine_call_count;
use render_tests::perf::{
    build_fixture, environment_json, negative_control_edit, CI_CELL_LOAD_P99_NS, CI_FRAME_MAX_NS,
    CI_FRAME_P99_NS, FIXTURE_VALUE_ROWS,
};

fn main() {
    let gate_mode = std::env::args().any(|a| a == "--gate");
    let commit = std::env::var("FREECELL_COMMIT")
        .or_else(|_| std::env::var("POC_COMMIT"))
        .unwrap_or_else(|_| "unknown".to_string());
    let date = std::env::var("FREECELL_DATE")
        .or_else(|_| std::env::var("POC_DATE"))
        .unwrap_or_else(|_| "unknown".to_string());

    // The default is the real 1M×100 styled fixture; env overrides let a quick smoke run shrink
    // it (they do NOT belong in a calibration/gate run — the committed thresholds are for the
    // default fixture).
    let mut cfg = PerfConfig::default();
    if let Some(cols) = env_u32("FREECELL_PERF_COLS") {
        cfg.cols = cols;
    }
    let value_rows = env_u32("FREECELL_PERF_VALUE_ROWS").unwrap_or(FIXTURE_VALUE_ROWS);

    // 1) Build the 1M×100 styled fixture through the REAL engine (blocking; worker kept alive).
    eprintln!(
        "building fixture: {}×{} sheet, {} valued rows …",
        cfg.rows, cfg.cols, value_rows
    );
    let fixture = match build_fixture(&cfg, value_rows) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("perf_harness: fixture build failed: {e}");
            std::process::exit(2);
        }
    };
    // Engine work happened building the fixture — the counter is already nonzero here.
    let engine_calls_after_fixture = engine_call_count();
    assert!(
        engine_calls_after_fixture > 0,
        "the fixture build must have exercised the engine (counter sanity)"
    );

    // 2) Drive the scripted sweep over the real grid inside a gpui app, then gate + report.
    let app = application();
    app.run(move |cx: &mut App| {
        let outcome = run_sweep(cx, &cfg, value_rows, &fixture, &commit, &date);
        // Report to stdout (p50/p99, environment-stamped).
        println!("{}", outcome.report_text);
        // Write the recorded JSON.
        match outcome.write_json() {
            Ok(path) => println!("results written to {}", path.display()),
            Err(e) => eprintln!("perf_harness: failed to write results JSON: {e}"),
        }

        let ok = outcome.zero_engine_calls
            && outcome.negative_control_climbed
            && outcome.ci_report.passed();
        if gate_mode && !ok {
            eprintln!("perf_harness: GATE FAILED (see report above)");
            std::process::exit(1);
        }
        std::process::exit(0);
    });
}

/// The finalized outcome of a sweep — everything the driver prints, records, and gates on.
struct Outcome {
    report_text: String,
    json: serde_json::Value,
    zero_engine_calls: bool,
    negative_control_climbed: bool,
    ci_report: RunReport,
}

impl Outcome {
    fn write_json(&self) -> std::io::Result<PathBuf> {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("results");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("perf-runtest.json");
        std::fs::write(&path, serde_json::to_vec_pretty(&self.json)?)?;
        Ok(path)
    }
}

fn env_u32(key: &str) -> Option<u32> {
    std::env::var(key).ok().and_then(|v| v.parse().ok())
}

/// Drive the scripted harness through the real `GridView::measure_frame`, run the
/// zero-engine-calls gate + negative control, and reduce the samples into a report.
fn run_sweep(
    cx: &mut App,
    cfg: &PerfConfig,
    value_rows: u32,
    fixture: &render_tests::perf::Fixture,
    commit: &str,
    date: &str,
) -> Outcome {
    let sources = fixture.sources();
    let grid = cx.new(|cx| GridView::new(sources, GridEventSink::noop(), cx));

    let vw = cfg.viewport_width as f64;
    let vh = cfg.viewport_height as f64;

    // Snapshot the engine-call counter immediately BEFORE the sweep. Everything the sweep does
    // reads only the shared publication + caches — never the worker/engine — so this must not
    // move across the whole sweep.
    let calls_before_sweep = engine_call_count();

    let mut harness = Harness::scripted(cfg);
    let mut prev: Option<(std::ops::Range<u32>, std::ops::Range<u32>)> = None;
    let mut distinct: HashSet<(u32, u32)> = HashSet::new();

    while let Some(vp) = harness.next_viewport() {
        let prev_for_call = prev.clone();
        let (sample, ranges) = grid.update(cx, |g, _cx| {
            g.measure_frame(vp.scroll_x, vp.scroll_y, vw, vh, prev_for_call)
        });
        harness.record(sample);
        distinct.insert((ranges.0.start, ranges.1.start));
        prev = Some(ranges);
    }

    let samples = harness.samples().to_vec();
    let calls_after_sweep = engine_call_count();

    // FORCE + ASSERT: the sweep actually scrolled across many distinct viewports (not the same
    // frame 348 times), and it built real frames.
    assert!(
        distinct.len() > 20,
        "the scripted sweep must visit many distinct viewports (got {})",
        distinct.len()
    );
    assert!(
        samples.iter().all(|s| s.elements > 0),
        "every measured frame must have built real content cells"
    );

    // The zero-engine-calls-on-scroll gate: the counter did not move across the whole sweep.
    let zero_engine_calls = calls_after_sweep == calls_before_sweep;

    // Negative control: one real edit MUST make the counter climb, proving the gate is not
    // vacuous. Sent to the still-alive worker; drain until it goes idle again.
    fixture.client().send(negative_control_edit(fixture.sheet));
    let _ = fixture.drain_to_idle();
    let calls_after_edit = engine_call_count();
    let negative_control_climbed = calls_after_edit > calls_after_sweep;

    // Two reports: the product-truth verdict (§4 real-hardware budgets) and the hard CI gate
    // (committed buffered thresholds = 2× the calibrated p99).
    let truth = RunReport::build(
        &samples,
        FRAME_TARGET_NS,
        FRAME_WORST_NS,
        CELL_LOAD_TARGET_NS,
    );
    let ci = RunReport::build(
        &samples,
        CI_FRAME_P99_NS,
        CI_FRAME_MAX_NS,
        CI_CELL_LOAD_P99_NS,
    );

    let report_text = format_report(
        &truth,
        &ci,
        zero_engine_calls,
        calls_after_sweep - calls_before_sweep,
        negative_control_climbed,
        calls_after_edit - calls_after_sweep,
        distinct.len(),
    );

    let json = report_json(
        &truth,
        &ci,
        zero_engine_calls,
        negative_control_climbed,
        calls_after_edit.saturating_sub(calls_after_sweep),
        cfg,
        value_rows,
        commit,
        date,
    );

    Outcome {
        report_text,
        json,
        zero_engine_calls,
        negative_control_climbed,
        ci_report: ci,
    }
}

#[allow(clippy::too_many_arguments)]
fn format_report(
    truth: &RunReport,
    ci: &RunReport,
    zero_engine_calls: bool,
    sweep_delta: u64,
    neg_control_climbed: bool,
    neg_control_delta: u64,
    distinct_viewports: usize,
) -> String {
    let mut out = String::new();
    out.push_str(&truth.summary("real-grid / 1M×100 styled (CPU render-build path)"));
    out.push_str("  ^ verdict above is vs the §4 REAL-HARDWARE budgets (product truth; lavapipe CPU-build only — informational)\n\n");
    out.push_str("--- committed CI gate (buffered = 2× calibrated p99) ---\n");
    for g in &ci.gates {
        out.push_str(&g.summary());
        out.push('\n');
    }
    out.push_str(&format!(
        "zero-engine-calls-on-scroll: {} (sweep delta = {} engine calls over {} distinct viewports)\n",
        if zero_engine_calls { "PASS" } else { "FAIL" },
        sweep_delta,
        distinct_viewports,
    ));
    out.push_str(&format!(
        "negative control: {} (one edit moved the counter by {} calls — proves the gate is discriminating)\n",
        if neg_control_climbed { "PASS" } else { "FAIL" },
        neg_control_delta,
    ));
    let ci_ok = ci.passed() && zero_engine_calls && neg_control_climbed;
    out.push_str(&format!(
        "CI VERDICT: {}\n",
        if ci_ok { "PASS" } else { "FAIL" }
    ));
    out
}

#[allow(clippy::too_many_arguments)]
fn report_json(
    truth: &RunReport,
    ci: &RunReport,
    zero_engine_calls: bool,
    neg_control_climbed: bool,
    neg_control_delta: u64,
    cfg: &PerfConfig,
    value_rows: u32,
    commit: &str,
    date: &str,
) -> serde_json::Value {
    serde_json::json!({
        "name": "freecell-perf-runtest",
        "date": date,
        "environment": environment_json(commit),
        "fixture": {
            "rows": cfg.rows,
            "cols": cfg.cols,
            "valued_rows": value_rows,
            "styled": true,
            "note": "1M×100 styled: dense col-band styles across all cols, variable col widths + row heights, engine-produced publication over the valued band",
        },
        "frames": truth.frame_stats.count,
        "frame_render": stats_json(&truth.frame_stats),
        "cell_load": stats_json(&truth.cell_load_stats),
        "true_budgets_ns": {
            "frame_p99": FRAME_TARGET_NS,
            "frame_max": FRAME_WORST_NS,
            "cell_load_p99": CELL_LOAD_TARGET_NS,
            "verdict": if truth.passed() { "PASS" } else { "FAIL" },
            "note": "real-hardware product truth; under lavapipe only the CPU render-build path is measured (informational)",
        },
        "ci_thresholds_ns": {
            "frame_p99": CI_FRAME_P99_NS,
            "frame_max": CI_FRAME_MAX_NS,
            "cell_load_p99": CI_CELL_LOAD_P99_NS,
            "verdict": if ci.passed() { "PASS" } else { "FAIL" },
            "note": "committed buffered gate = 2× the calibrated p99 on the pinned runner image",
        },
        "zero_engine_calls_on_scroll": {
            "verdict": if zero_engine_calls { "PASS" } else { "FAIL" },
            "negative_control_climbed": neg_control_climbed,
            "negative_control_delta": neg_control_delta,
        },
    })
}

fn stats_json(s: &freecell_core::perf::LatencyStats) -> serde_json::Value {
    serde_json::json!({
        "count": s.count,
        "min_ns": s.min_ns,
        "max_ns": s.max_ns,
        "mean_ns": s.mean_ns,
        "p50_ns": s.p50_ns,
        "p99_ns": s.p99_ns,
        "p50": fmt_ns(s.p50_ns),
        "p99": fmt_ns(s.p99_ns),
        "max": fmt_ns(s.max_ns),
    })
}
