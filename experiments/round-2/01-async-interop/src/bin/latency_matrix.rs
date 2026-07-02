//! `latency_matrix` — the SP1 `evaluate()` latency matrix runner (functional_spec SP1
//! DISCOVERY).
//!
//! Times a single full `Model::evaluate()` across **sizes {10⁴,10⁵,10⁶,10⁷} × DAG
//! shapes {sparse, wide fan-out, deep-serial chain, cross-sheet, volatile}**, p50/p99,
//! env-stamped, force+asserting the tail changed each sample. Writes one
//! `results/latency_<shape>_<size>.json` per cell plus a `latency_summary.json`.
//!
//! ## Benchmark discipline
//! - Run **foreground with `timeout`**. Build time is separated from the measured op
//!   (the model is built once per cell, then only `evaluate()` is timed).
//! - **Resource discipline:** heavy scales (10⁶/10⁷) run **one at a time** — each cell
//!   builds its model, times it, then drops it before the next, so at most one big model
//!   is resident. The deep-serial 10⁷ (10M chain) may exceed the shared box's
//!   memory/time; use `--max-size`/`--shape` to cap it and record the ceiling.
//!
//! ## Usage
//! ```text
//! cargo run --release --bin latency_matrix              # full matrix (respects caps)
//! cargo run --release --bin latency_matrix -- --shape deep_serial --size 1000000
//! cargo run --release --bin latency_matrix -- --max-size 1000000   # skip 10^7
//! ```

use std::path::PathBuf;

use bench_util::{BenchResult, Environment};
use round2_harness::cpu_model;
use serde_json::json;
use sp1_async_interop::shapes::{self, Shape};
use sp1_async_interop::time_evaluate;

/// Report date (deterministic; recording never reads a wall clock — architecture §3).
const REPORT_DATE: &str = "2026-07-01";

/// The matrix sizes (target populated-cell counts).
const SIZES: [u64; 4] = [10_000, 100_000, 1_000_000, 10_000_000];

/// How many timed samples per cell, scaled down for heavy sizes so total runtime stays
/// bounded (each 10⁶ eval is ~seconds; we still get a p50/p99 over a few samples).
fn samples_for(size: u64) -> usize {
    match size {
        s if s <= 10_000 => 30,
        s if s <= 100_000 => 15,
        s if s <= 1_000_000 => 7,
        _ => 3,
    }
}

/// Per-shape/size safety cap. Deep-serial builds one IronCalc cell per chain step and
/// evaluates recursively; at 10⁷ that is a 10M-deep recursion + ~1.6 GB of cells and is
/// prone to stack overflow / OOM / multi-minute runs on the shared 4c/15 GB box. We cap
/// deep-serial at 10⁶ by default and record the ceiling as a finding (the ~2 s at 10⁶ is
/// already the known-FAIL data point the spec wants).
fn is_capped(shape: Shape, size: u64, max_size: u64) -> Option<&'static str> {
    if size > max_size {
        return Some("above --max-size");
    }
    if shape == Shape::DeepSerial && size >= 10_000_000 {
        return Some("deep-serial 10^7 capped: 10M-deep recursion risks stack overflow / OOM on the 4c/15GB box; 10^6 (~2s) is the recorded known-FAIL ceiling");
    }
    None
}

struct Args {
    only_shape: Option<Shape>,
    only_size: Option<u64>,
    max_size: u64,
}

fn parse_args() -> Args {
    let mut only_shape = None;
    let mut only_size = None;
    let mut max_size = u64::MAX;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--shape" => {
                let v = it.next().expect("--shape needs a value");
                only_shape =
                    Some(Shape::from_id(&v).unwrap_or_else(|| panic!("unknown shape {v}")));
            }
            "--size" => {
                only_size = Some(
                    it.next()
                        .expect("--size needs a value")
                        .parse()
                        .expect("size"),
                );
            }
            "--max-size" => {
                max_size = it
                    .next()
                    .expect("--max-size needs a value")
                    .parse()
                    .expect("max-size");
            }
            other => panic!("unknown arg {other}"),
        }
    }
    Args {
        only_shape,
        only_size,
        max_size,
    }
}

fn results_dir() -> PathBuf {
    // Relative to the crate root (cargo runs bins from there).
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("results")
}

fn main() -> anyhow::Result<()> {
    let args = parse_args();
    let env = Environment::detect("HEAD").with_cpu(cpu_model());
    let dir = results_dir();
    std::fs::create_dir_all(&dir)?;

    let shapes_to_run: Vec<Shape> = match args.only_shape {
        Some(s) => vec![s],
        None => Shape::all().to_vec(),
    };
    let sizes_to_run: Vec<u64> = match args.only_size {
        Some(s) => vec![s],
        None => SIZES.to_vec(),
    };

    // Iterate size-outer so heavy sizes are grouped; each cell builds+drops its own
    // model (only one big model resident at a time — resource discipline).
    for &size in &sizes_to_run {
        for &shape in &shapes_to_run {
            if let Some(reason) = is_capped(shape, size, args.max_size) {
                println!("SKIP  {:<12} size={:>10}  ({reason})", shape.id(), size);
                // Persist a per-cell marker for structural caps (not --max-size skips) so
                // the aggregated summary records the ceiling as a finding.
                if !reason.contains("above --max-size") {
                    let marker = json!({
                        "shape": shape.id(),
                        "requested_size": size,
                        "status": "capped",
                        "reason": reason,
                    });
                    std::fs::write(
                        dir.join(format!("latency_{}_{}.json", shape.id(), size)),
                        serde_json::to_string_pretty(&marker)?,
                    )?;
                }
                continue;
            }

            print!("BUILD {:<12} size={:>10} ... ", shape.id(), size);
            use std::io::Write;
            std::io::stdout().flush().ok();
            let build_start = std::time::Instant::now();
            let mut built = shapes::build(shape, size);
            let build_secs = build_start.elapsed().as_secs_f64();
            println!(
                "built {} cells in {:.2}s; timing evaluate()...",
                built.populated_cells, build_secs
            );

            let samples = samples_for(size);
            let cell = time_evaluate(&mut built, size, samples);

            // Drop the big model ASAP (before writing/next cell).
            drop(built);

            let result = BenchResult::new(
                format!("evaluate_{}_{}", shape.id(), size),
                cell.populated_cells,
                REPORT_DATE,
                env.clone(),
            )
            .with_stats(cell.stats)
            .with_extra(json!({
                "shape": shape.id(),
                "requested_size": size,
                "populated_cells": cell.populated_cells,
                "samples": samples,
                "build_secs": build_secs,
                "tail_a1": cell.tail_a1,
                "tail_before": cell.tail_before,
                "tail_after": cell.tail_after,
            }));

            let path = dir.join(format!("latency_{}_{}.json", shape.id(), size));
            result.write_json(&path)?;

            println!(
                "  {:<12} size={:>10} pop={:>10}  p50={:>10}  p99={:>10}  (tail {} {} -> {})",
                shape.id(),
                size,
                cell.populated_cells,
                bench_util::fmt_ns(cell.stats.p50_ns),
                bench_util::fmt_ns(cell.stats.p99_ns),
                cell.tail_a1,
                cell.tail_before,
                cell.tail_after,
            );
        }
    }

    // The committed summary aggregates EVERY per-cell file on disk so it's complete
    // regardless of how the matrix was split across `--shape`/`--size` invocations
    // (resource discipline runs heavy scales one at a time).
    let all_cells = aggregate_cells_from_disk(&dir)?;
    let summary_doc = json!({
        "experiment": "SP1 evaluate() latency matrix",
        "date": REPORT_DATE,
        "environment": {
            "cpu": env.cpu,
            "os": env.os,
            "arch": env.arch,
            "cores": env.cores,
        },
        "note": "evaluate() is full-workbook (O(all cells)), non-incremental, non-interruptible. Deep-serial 10^6 (~1.2s here) is the expected known-FAIL vs the <100ms target. Deep-serial 10^7 is capped (10M-deep recursion / Excel 1,048,576-row limit).",
        "cells": all_cells,
    });
    std::fs::write(
        dir.join("latency_summary.json"),
        serde_json::to_string_pretty(&summary_doc)?,
    )?;

    println!("\nWrote results to {}", dir.display());
    Ok(())
}

/// Reads every `latency_<shape>_<size>.json` per-cell file in `dir` and returns a
/// compact summary row per cell, sorted by (shape, size), so the committed
/// `latency_summary.json` reflects the whole matrix even when it was run in pieces.
fn aggregate_cells_from_disk(dir: &std::path::Path) -> anyhow::Result<Vec<serde_json::Value>> {
    let mut cells: Vec<(String, u64, serde_json::Value)> = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if !name.starts_with("latency_") || name == "latency_summary.json" {
            continue;
        }
        let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path)?)?;
        // Two file shapes: a full BenchResult (measured) or a small capped marker.
        if doc.get("status").and_then(|v| v.as_str()) == Some("capped") {
            let shape = doc["shape"].as_str().unwrap_or("").to_string();
            let size = doc["requested_size"].as_u64().unwrap_or(0);
            cells.push((shape.clone(), size, doc));
        } else {
            let extra = &doc["extra"];
            let shape = extra["shape"].as_str().unwrap_or("").to_string();
            let size = extra["requested_size"].as_u64().unwrap_or(0);
            let stats = &doc["stats"];
            let row = json!({
                "shape": shape,
                "requested_size": size,
                "populated_cells": doc["input_size"],
                "status": "measured",
                "p50_ns": stats["p50_ns"],
                "p99_ns": stats["p99_ns"],
                "p50": bench_util::fmt_ns(stats["p50_ns"].as_u64().unwrap_or(0)),
                "p99": bench_util::fmt_ns(stats["p99_ns"].as_u64().unwrap_or(0)),
                "tail_before": extra["tail_before"],
                "tail_after": extra["tail_after"],
            });
            cells.push((shape, size, row));
        }
    }
    cells.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    Ok(cells.into_iter().map(|(_, _, v)| v).collect())
}
