//! P10 Step 4 — headless conditional-formatting **edit-path** perf bin (`phase_plans/phase_10.md`
//! Step 4, `CLAUDE.md` benchmark conventions).
//!
//! Measures the **edit → cache-rebuild → repaint-ready** worker round-trip latency on a populated
//! 256×8 range (`A1:H256`), **CF-on vs CF-off**, through the **real** [`DocumentClient`] — the same
//! worker path the product drives. **Headless** (worker thread, no GPU / no Xvfb), so it runs
//! directly under a `timeout`:
//!
//! ```text
//! cargo run -p render-tests --release --bin cf_perf
//! ```
//!
//! Two fixtures carry the **same** populated values over `A1:H256`:
//! - **CF-on**: a `CellIs > 100` highlight rule (a fill format) over the whole range.
//! - **CF-off**: the identical values, no rule.
//!
//! Per measured iteration one **in-range** cell's value is flipped across the threshold
//! (alternating `50 ↔ 150`, so it is a **real change every iteration** — never a measured no-op),
//! sent as a real `SetCellInput`, and timed from the send until the worker goes quiet: send →
//! **last** publish/cache event, excluding the trailing idle wait. On the CF-on sheet the last
//! event is the CF fold's `StyleCacheUpdated` (the bounded full-range rebuild); on the CF-off sheet
//! it is the cheap touched-cell mirror. Warm-up iterations are discarded; p50 / p99 / max are taken
//! over a solid measured sample.
//!
//! Every measured edit is **FORCE + ASSERTED** (per `CLAUDE.md` — the measured work provably
//! happened): a static matching (`>100`) probe cell carries the CF fill in CF-on and **no** fill in
//! CF-off (the CF fold happened / didn't), the flipped cell's fill tracks its value each iteration
//! in CF-on (and never fills in CF-off), and every edit emitted at least one publish/cache event (no
//! silent no-op). Any violation panics — a benchmark that measured nothing must fail loudly. Run
//! FOREGROUND; never background it. Writes `results/cf-perf.json`.

use std::time::{Duration, Instant};

use freecell_core::perf::{fmt_ns, LatencyStats};
use freecell_core::{CellRef, CfFormat, CfRuleSpec, CfValueOp, Rgb, SheetId};
use freecell_engine::{Command, DocumentClient, DocumentSource, WorkerEvent, WorkerEventReceiver};

// --- Fixture shape --------------------------------------------------------------------------

/// Rows of the populated range (`A1:H256`).
const ROWS: u32 = 256;
/// Columns of the populated range (`A1:H256`).
const COLS: u32 = 8;
/// The A1 range the CF rule and the viewport cover.
const RANGE_A1: &str = "A1:H256";

/// The in-range cell flipped every measured iteration (mid-sheet, mid-column).
const FLIP: CellRef = CellRef::new(128, 4);
/// A static, always-matching (`>100`) probe cell — the standalone "did the CF fold fill a matching
/// cell?" guard (filled in CF-on, unfilled in CF-off), independent of the measurement loop.
const PROBE: CellRef = CellRef::new(0, 0);

/// The high value (`> 100` → matches the rule → fills) the flip cell alternates onto.
const HIGH: &str = "150";
/// The low value (`<= 100` → no match → no fill) the flip cell alternates onto.
const LOW: &str = "50";
/// The `CellIs` threshold operand.
const THRESHOLD: &str = "100";
/// The highlight fill (Excel's classic light-red CF fill) — the exact `Rgb` the folded resident
/// cache carries on a matching cell (the engine folds `CfFormat.fill` straight through).
const FILL: Rgb = Rgb::from_hex(0xFFC7CE);

// --- Sampling -------------------------------------------------------------------------------

/// Discarded warm-up iterations (JIT/alloc/cache warm-up before recording).
const WARMUP: usize = 50;
/// Recorded measured iterations (a solid sample per `CLAUDE.md` — well over the 200 floor).
const MEASURED: usize = 250;

// --- Worker timing knobs --------------------------------------------------------------------

/// Patience for the worker's initial `Loaded` event when spinning up a fixture.
const LOAD_TIMEOUT: Duration = Duration::from_secs(10);
/// Build-time idle gap: once no event arrives for this long, the fixture's big populate batch has
/// settled. Generous (the populate ships thousands of cells).
const BUILD_IDLE_GAP: Duration = Duration::from_millis(150);
/// Build-time drain cap (a misbehaving worker fails the build instead of hanging).
const BUILD_DRAIN_CAP: Duration = Duration::from_secs(30);
/// Per-edit patience for the **first** event after a `SetCellInput` (the worker wakes, applies the
/// edit, folds CF). Generous so a slow CI CPU's first event is never mistaken for "no event".
const FIRST_EVENT_TIMEOUT: Duration = Duration::from_secs(2);
/// Per-edit idle gap draining the **tail** of one edit's event burst. The worker emits a single
/// edit's events synchronously back-to-back (no timed debounce — `run()` is recv → drain → process),
/// so a short gap reliably detects the burst end without splitting it.
const MEASURE_IDLE_GAP: Duration = Duration::from_millis(50);
/// Per-edit drain cap: a single edit that churns events this long is a worker fault (panic).
const MEASURE_DRAIN_CAP: Duration = Duration::from_secs(5);

/// A live headless fixture: the running worker client + its event receiver + the active sheet.
struct Fixture {
    client: DocumentClient,
    events: WorkerEventReceiver,
    sheet: SheetId,
}

fn main() {
    let commit = std::env::var("FREECELL_COMMIT")
        .or_else(|_| std::env::var("POC_COMMIT"))
        .unwrap_or_else(|_| "unknown".to_string());

    // CF-off baseline first, then CF-on (same values; the only difference is the rule).
    let off = build_fixture(false);
    assert_eq!(
        probe_fill(&off),
        None,
        "FORCE+ASSERT: the CF-off resident cache carries NO CF fill on the matching probe cell",
    );
    let cf_off = measure(&off, false);
    off.client.send(Command::Shutdown);

    let on = build_fixture(true);
    assert_eq!(
        probe_fill(&on),
        Some(FILL),
        "FORCE+ASSERT: the CF-on resident cache carries the CF fill on the matching (>100) probe \
         cell (the CF fold provably happened)",
    );
    let cf_on = measure(&on, true);
    on.client.send(Command::Shutdown);

    let env = render_tests::perf::environment_json(&commit);
    report(&env, &cf_on, &cf_off);

    if let Err(e) = write_json(&env, &cf_on, &cf_off) {
        eprintln!("cf_perf: failed to write results JSON: {e}");
    }
}

// --- Fixture build --------------------------------------------------------------------------

/// Builds a live headless fixture over `A1:H256`: populate the whole range with a deterministic
/// spread of numbers straddling the `100` threshold, pin the probe cell high, (optionally) add the
/// `CellIs > 100` fill rule over the whole range, publish the range, and drain to idle. The worker
/// is left **running** so `measure` can send edits + read the live resident cache.
fn build_fixture(with_cf: bool) -> Fixture {
    let (client, events) = DocumentClient::spawn(DocumentSource::NewWorkbook);

    let sheet = loop {
        match events.recv_timeout(LOAD_TIMEOUT) {
            Some(WorkerEvent::Loaded { sheets }) => {
                break sheets.first().map(|m| m.id).unwrap_or(SheetId(0));
            }
            Some(WorkerEvent::LoadFailed { error }) => panic!("worker load failed: {error}"),
            Some(_) => continue,
            None => panic!("worker never emitted Loaded within {LOAD_TIMEOUT:?}"),
        }
    };

    // Populate every cell of A1:H256 with a deterministic spread (0..196 → ~half match `>100`), so
    // the CF fold has a real, populated range to evaluate over.
    for row in 0..ROWS {
        for col in 0..COLS {
            client.send(Command::SetCellInput {
                sheet,
                cell: CellRef::new(row, col),
                input: populate_value(row, col),
            });
        }
    }
    // Pin the probe cell high (>100) so it always matches — the standalone CF-fold guard.
    client.send(Command::SetCellInput {
        sheet,
        cell: PROBE,
        input: HIGH.to_string(),
    });

    // The CF rule (a real `Command::AddCondFmt`) over the whole range — sent after the values so the
    // first fold already sees them; a fill-only `CellIs > 100` highlight.
    if with_cf {
        client.send(Command::AddCondFmt {
            sheet,
            range: RANGE_A1.to_string(),
            spec: gt_fill_rule(THRESHOLD, FILL),
        });
    }

    // Publish the whole range last so the settled cache the grid reads covers every value.
    client.send(Command::SetViewport {
        sheet,
        rows: 0..ROWS,
        cols: 0..COLS,
    });

    drain_to_idle(&events, BUILD_IDLE_GAP, BUILD_DRAIN_CAP)
        .unwrap_or_else(|e| panic!("fixture build never went idle: {e}"));

    Fixture {
        client,
        events,
        sheet,
    }
}

// --- Measurement ----------------------------------------------------------------------------

/// Measures the edit → cache-rebuild → repaint-ready round-trip on `fixture`: each iteration flips
/// `FLIP` across the threshold, sends a real `SetCellInput`, and times send → **last** event
/// (excluding the trailing idle gap). FORCE+ASSERTS every measured edit (see the module docs).
fn measure(fixture: &Fixture, with_cf: bool) -> LatencyStats {
    let sheet = fixture.sheet;
    let mut samples: Vec<u64> = Vec::with_capacity(MEASURED);

    for i in 0..(WARMUP + MEASURED) {
        let high = i % 2 == 0;
        let value = if high { HIGH } else { LOW };

        let start = Instant::now();
        fixture.client.send(Command::SetCellInput {
            sheet,
            cell: FLIP,
            input: value.to_string(),
        });

        // Wait (generously) for the worker to react, then drain the rest of this edit's burst with a
        // short idle gap. `last` is the moment of the final event → `last - start` is send → last
        // event, which excludes the trailing `MEASURE_IDLE_GAP` idle wait.
        let mut events_seen = 0u64;
        let mut last = match fixture.events.recv_timeout(FIRST_EVENT_TIMEOUT) {
            Some(_) => {
                events_seen += 1;
                Instant::now()
            }
            None => panic!(
                "iter {i}: worker emitted no event within {FIRST_EVENT_TIMEOUT:?} for a real value \
                 edit — the measured edit was a silent no-op"
            ),
        };
        let deadline = Instant::now() + MEASURE_DRAIN_CAP;
        while let Some(_ev) = fixture.events.recv_timeout(MEASURE_IDLE_GAP) {
            last = Instant::now();
            events_seen += 1;
            if Instant::now() >= deadline {
                panic!(
                    "iter {i}: worker still emitting events after {MEASURE_DRAIN_CAP:?} — fault"
                );
            }
        }
        let elapsed = last.duration_since(start).as_nanos() as u64;

        // FORCE + ASSERT: the edit emitted at least one publish/cache event (not a silent no-op).
        assert!(
            events_seen > 0,
            "iter {i}: measured edit emitted no publish/cache event"
        );

        // FORCE + ASSERT: after the worker settled, the resident cache reflects the edit. In CF-on
        // the flipped cell's fill tracks its value (Some(FILL) when >100, None when <=100) — proof
        // the CF fold responded to *this* measured edit; in CF-off it never carries a CF fill.
        let flip_fill = cell_fill(fixture, FLIP);
        if with_cf {
            let expect = if high { Some(FILL) } else { None };
            assert_eq!(
                flip_fill, expect,
                "iter {i}: CF-on flipped-cell fill must track its value (high={high})"
            );
        } else {
            assert_eq!(
                flip_fill, None,
                "iter {i}: CF-off flipped cell must never carry a CF fill"
            );
        }

        if i >= WARMUP {
            samples.push(elapsed);
        }
    }

    LatencyStats::from_samples(&samples)
}

// --- Reporting ------------------------------------------------------------------------------

/// Print the environment stamp + CF-on vs CF-off p50/p99/max side by side, and the delta note.
fn report(env: &serde_json::Value, cf_on: &LatencyStats, cf_off: &LatencyStats) {
    println!("=== conditional-formatting edit-path perf (headless, real DocumentClient) ===");
    println!(
        "environment: os={} arch={} cores={} rustc={} profile={} commit={}",
        env["os"].as_str().unwrap_or("?"),
        env["arch"].as_str().unwrap_or("?"),
        env["cores"],
        env["rustc"].as_str().unwrap_or("?"),
        env["profile"].as_str().unwrap_or("?"),
        env["commit"].as_str().unwrap_or("?"),
    );
    println!("  cpu: {}", env["cpu"].as_str().unwrap_or("?"));
    println!(
        "fixture: {ROWS}×{COLS} populated range {RANGE_A1} ({} cells); edit = flip one in-range \
         cell 50↔150 across the >100 threshold",
        ROWS * COLS
    );
    print_op("edit, CF-on  (add full-range CF rebuild)", cf_on);
    print_op("edit, CF-off (touched-cell mirror)      ", cf_off);

    let d50 = delta(cf_on.p50_ns, cf_off.p50_ns);
    let d99 = delta(cf_on.p99_ns, cf_off.p99_ns);
    println!(
        "delta (CF-on − CF-off): p50 {}{}  p99 {}{}  — CF-on adds the bounded full-range CF \
         rebuild ({} cells) that the CF-off touched-cell mirror skips.",
        d50.0,
        d50.1,
        d99.0,
        d99.1,
        ROWS * COLS,
    );
}

/// A signed delta as (`sign`, `fmt_ns(abs)`), so a negative delta reads correctly.
fn delta(on_ns: u64, off_ns: u64) -> (&'static str, String) {
    if on_ns >= off_ns {
        ("+", fmt_ns(on_ns - off_ns))
    } else {
        ("-", fmt_ns(off_ns - on_ns))
    }
}

fn print_op(name: &str, stats: &LatencyStats) {
    println!(
        "  {name}: p50 {}  p99 {}  (max {}, n={})",
        fmt_ns(stats.p50_ns),
        fmt_ns(stats.p99_ns),
        fmt_ns(stats.max_ns),
        stats.count,
    );
}

// --- Helpers --------------------------------------------------------------------------------

/// A "Cell value > operand" highlight rule filling matches with `fill` (fill-only differential).
fn gt_fill_rule(operand: &str, fill: Rgb) -> CfRuleSpec {
    CfRuleSpec::CellIs {
        op: CfValueOp::Gt,
        operand: operand.to_string(),
        operand2: None,
        format: CfFormat {
            fill: Some(fill),
            ..Default::default()
        },
        stop_if_true: false,
    }
}

/// The populate value for a cell: a deterministic spread 0..196 (roughly half match `>100`).
fn populate_value(row: u32, col: u32) -> String {
    ((row * COLS + col) % 197).to_string()
}

/// The resident cache's folded fill on the fixture's probe cell (`None` when unstored/unfilled).
fn probe_fill(fixture: &Fixture) -> Option<Rgb> {
    cell_fill(fixture, PROBE)
}

/// The resident cache's folded render-style fill for `cell` (the same read the grid paints from).
fn cell_fill(fixture: &Fixture, cell: CellRef) -> Option<Rgb> {
    let caches = fixture.client.caches();
    let guard = caches.read();
    guard
        .get(fixture.sheet)
        .and_then(|c| c.render_style(cell.row, cell.col))
        .and_then(|rs| rs.fill)
}

/// Drain worker events until the queue stays empty for `idle_gap`. A `cap` overrun is a hard error
/// (the worker never went idle; the data may be incomplete).
fn drain_to_idle(
    events: &WorkerEventReceiver,
    idle_gap: Duration,
    cap: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + cap;
    loop {
        match events.recv_timeout(idle_gap) {
            Some(_) => {
                if Instant::now() >= deadline {
                    return Err(format!("worker still emitting events after {cap:?}"));
                }
            }
            None => return Ok(()),
        }
    }
}

// --- JSON output ----------------------------------------------------------------------------

fn write_json(
    env: &serde_json::Value,
    cf_on: &LatencyStats,
    cf_off: &LatencyStats,
) -> std::io::Result<()> {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("results");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("cf-perf.json");
    let json = serde_json::json!({
        "name": "freecell-cf-perf",
        "environment": env,
        "note": "Conditional-formatting edit-path perf (P10 Step 4): headless send → last publish/cache event round-trip through the real DocumentClient, on a populated 256×8 range (A1:H256), CF-on vs CF-off. Each iteration flips one in-range cell 50↔150 across the >100 threshold (a real change every iteration). CF-on adds the bounded full-range CF rebuild (2048-cell fold); CF-off stays on the cheap touched-cell mirror. Force+asserted: the CF fold fills a matching cell in CF-on / not in CF-off, the flipped cell's fill tracks its value each iteration, and every edit emitted an event.",
        "fixture": {
            "rows": ROWS,
            "cols": COLS,
            "cells": ROWS * COLS,
            "range": RANGE_A1,
            "rule": "CellIs > 100 (fill highlight)",
            "edit": "flip one in-range cell 50<->150 across the >100 threshold",
            "warmup": WARMUP,
            "measured": MEASURED,
        },
        "ops": {
            "edit_cf_on": stats_json(cf_on),
            "edit_cf_off": stats_json(cf_off),
        },
        "delta_ns": {
            "p50": cf_on.p50_ns as i128 - cf_off.p50_ns as i128,
            "p99": cf_on.p99_ns as i128 - cf_off.p99_ns as i128,
        },
    });
    std::fs::write(&path, serde_json::to_vec_pretty(&json)?)?;
    println!("results written to {}", path.display());
    Ok(())
}

fn stats_json(s: &LatencyStats) -> serde_json::Value {
    serde_json::json!({
        "count": s.count,
        "p50_ns": s.p50_ns,
        "p99_ns": s.p99_ns,
        "max_ns": s.max_ns,
        "p50": fmt_ns(s.p50_ns),
        "p99": fmt_ns(s.p99_ns),
        "max": fmt_ns(s.max_ns),
    })
}
