//! `measure` — the SP2 orchestrator (parent process).
//!
//! Usage:  `cargo run --release --bin measure -- [target_mb] [runs] [out_path]`
//!
//! Steps (all foreground):
//! 1. Generate (or grow to) a ≥ `target_mb` styled `.xlsx` from committed code, recording
//!    the exact final [`GenSpec`] so the child knows the sentinel.
//! 2. Spawn the `open` binary `runs` times as **fresh child processes** (default 3), each
//!    stamping its own peak RSS (canonical VmHWM). Fresh child per run = cold peak RSS.
//! 3. Aggregate per-stage timings (min/median/max), take peak RSS = max across runs,
//!    compute the RSS multiple vs file size, apply the judgment GATE ("seconds not
//!    minutes" + "sane RSS multiple"), and write env-stamped `results/`.
//!
//! Peak RSS never comes from THIS process — only from the children (architecture §3).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use bench_util::{BenchResult, Environment};
use serde_json::json;
use xlsx_open::{generate_until_target, GenSpec, OpenStages};

const BYTES_PER_MB: u64 = 1024 * 1024;
/// Judgment thresholds (functional_spec SP2). "Seconds not minutes": open (to
/// recompute-ready) under this many seconds passes the time GATE.
const OPEN_SECONDS_CEILING: f64 = 60.0;
/// "Sane multiple of file size": the judgment GATE reads peak RSS as a multiple of the
/// **uncompressed** OOXML payload (what IronCalc actually parses in memory). At or under
/// this multiple passes. The louder OFF-RAMP flag fires at the spec's ≫10× uncompressed.
const RSS_UNCOMPRESSED_MULTIPLE_CEILING: f64 = 8.0;
/// Off-ramp trigger from functional_spec SP2: peak RSS ≫10× uncompressed.
const OFF_RAMP_UNCOMPRESSED_MULTIPLE: f64 = 10.0;

/// Mirror of the child's stdout JSON. Kept in sync with `bin/open.rs::ChildReport`.
#[derive(serde::Serialize, serde::Deserialize)]
struct ChildReport {
    file_bytes: u64,
    uncompressed_bytes: u64,
    peak_rss_bytes: u64,
    stages: OpenStages,
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let target_mb: u64 = args
        .next()
        .map(|s| s.parse())
        .transpose()
        .context("target_mb must be an integer")?
        .unwrap_or(100);
    let runs: u32 = args
        .next()
        .map(|s| s.parse())
        .transpose()
        .context("runs must be an integer")?
        .unwrap_or(3);
    let out_path: PathBuf = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data/large.xlsx"));

    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::create_dir_all("results").ok();

    // --- Step 1: generate the file (build time kept OUT of the open measurement) ---
    // Reuse fast-path: `GenSpec::large()` lands >= 100 MB in a single attempt (245k rows),
    // so when SP2_REUSE_FILE=1 and an existing file at `out_path` already meets target, we
    // measure it directly with `GenSpec::large()` (its exact generating spec, hence its
    // exact sentinel). This keeps `measure` the one canonical command while avoiding a
    // costly ~100s regeneration in the timing-constrained container. Without the flag it
    // always regenerates from committed code (fully reproducible).
    let target_bytes = target_mb * BYTES_PER_MB;
    let reuse = std::env::var("SP2_REUSE_FILE").is_ok_and(|v| v == "1");
    let (spec, file_bytes) = if reuse
        && std::fs::metadata(&out_path)
            .map(|m| m.len() >= target_bytes)
            .unwrap_or(false)
    {
        let bytes = std::fs::metadata(&out_path)?.len();
        println!(
            "SP2 measure: REUSING existing {:.1} MB file at {} (SP2_REUSE_FILE=1).",
            bytes as f64 / BYTES_PER_MB as f64,
            out_path.display()
        );
        (GenSpec::large(), bytes)
    } else {
        println!("SP2 measure: generating >= {target_mb} MB styled .xlsx ...");
        let (spec, gen_report) = generate_until_target(
            GenSpec::large(),
            target_bytes,
            &out_path,
            |attempt, spec, report| {
                println!(
                    "  gen attempt {attempt}: rows={} -> {:.1} MB (build {:.2}s, write {:.2}s)",
                    spec.rows,
                    report.file_bytes as f64 / BYTES_PER_MB as f64,
                    report.build.as_secs_f64(),
                    report.write.as_secs_f64(),
                );
            },
        )?;
        (spec, gen_report.file_bytes)
    };
    println!(
        "SP2 measure: file ready: {:.1} MB, {} cells across {} sheets.",
        file_bytes as f64 / BYTES_PER_MB as f64,
        spec.total_cells(),
        spec.sheets,
    );

    // --- Step 2: spawn `open` as fresh child processes ---
    let open_bin = locate_open_bin()?;
    let mut reports = Vec::with_capacity(runs as usize);
    for run in 1..=runs {
        println!("  open run {run}/{runs} (fresh child) ...");
        let report = run_open_child(&open_bin, &out_path, &spec)?;
        println!(
            "    read {:.3}s | parse+build {:.3}s | first-paint {:.3}s | first-eval {:.3}s \
             | total {:.3}s | peak RSS {:.1} MB",
            ns_s(report.stages.read_ns),
            ns_s(report.stages.parse_build_ns),
            ns_s(report.stages.first_paint_ns),
            ns_s(report.stages.first_eval_ns),
            ns_s(report.stages.total_ns),
            report.peak_rss_bytes as f64 / BYTES_PER_MB as f64,
        );
        reports.push(report);
    }

    // --- Step 3: aggregate, gate, record ---
    let agg = aggregate(&reports);
    let peak_rss = reports.iter().map(|r| r.peak_rss_bytes).max().unwrap_or(0);
    let uncompressed_bytes = reports
        .iter()
        .map(|r| r.uncompressed_bytes)
        .max()
        .unwrap_or(0);
    let rss_multiple_file = peak_rss as f64 / file_bytes as f64;
    let rss_multiple_uncompressed = peak_rss as f64 / uncompressed_bytes as f64;
    let open_total_s = ns_s(agg.total_ns_med);
    let first_paint_s = ns_s(agg.first_paint_ns_med);

    let time_pass = open_total_s <= OPEN_SECONDS_CEILING;
    let rss_pass = rss_multiple_uncompressed <= RSS_UNCOMPRESSED_MULTIPLE_CEILING;
    // Off-ramp fires on minutes-scale open OR RSS ≫10× UNCOMPRESSED (functional_spec SP2).
    let offramp = open_total_s > OPEN_SECONDS_CEILING
        || rss_multiple_uncompressed > OFF_RAMP_UNCOMPRESSED_MULTIPLE;

    // Dominant stage (median-of-medians): which of read / parse+build / eval costs most.
    let dominant = dominant_stage(&agg);

    println!("\nSP2 measure: RESULTS");
    println!(
        "  open->recompute-ready (median): {:.3}s  [GATE seconds-not-minutes: {}]",
        open_total_s,
        pass_str(time_pass)
    );
    println!(
        "  peak RSS: {:.1} MB = {:.2}x compressed file / {:.2}x uncompressed payload \
         ({:.1} MB)  [GATE <= {}x uncompressed: {}]",
        peak_rss as f64 / BYTES_PER_MB as f64,
        rss_multiple_file,
        rss_multiple_uncompressed,
        uncompressed_bytes as f64 / BYTES_PER_MB as f64,
        RSS_UNCOMPRESSED_MULTIPLE_CEILING,
        pass_str(rss_pass)
    );
    println!("  dominant stage: {dominant}");
    println!("  time-to-first-paint (median): {first_paint_s:.3}s");
    println!(
        "  OFF-RAMP flag (open minutes-scale OR RSS >{OFF_RAMP_UNCOMPRESSED_MULTIPLE}x uncompressed): {}",
        if offramp {
            "TRIGGERED (see findings)"
        } else {
            "clear"
        }
    );

    write_results(
        &spec,
        file_bytes,
        uncompressed_bytes,
        peak_rss,
        rss_multiple_file,
        rss_multiple_uncompressed,
        &agg,
        &reports,
        time_pass,
        rss_pass,
        offramp,
        &dominant,
    )?;
    println!("\nSP2 measure: wrote results/open_stage_timings.json, results/open_summary.json, results/env.txt");
    Ok(())
}

/// Runs the `open` child once and parses its single JSON stdout line.
fn run_open_child(open_bin: &Path, file: &Path, spec: &GenSpec) -> Result<ChildReport> {
    let output = Command::new(open_bin)
        .arg(file)
        .arg(spec.seed.to_string())
        .arg(spec.sheets.to_string())
        .arg(spec.rows.to_string())
        .arg(spec.cols.to_string())
        .output()
        .with_context(|| format!("spawning {}", open_bin.display()))?;
    if !output.status.success() {
        return Err(anyhow!(
            "open child failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let stdout = String::from_utf8(output.stdout).context("child stdout not UTF-8")?;
    let line = stdout
        .lines()
        .last()
        .ok_or_else(|| anyhow!("child produced no output"))?;
    parse_child_line(line)
}

/// Parses one child JSON line into a [`ChildReport`]. Split out so it is unit-testable
/// without spawning a process (covers the parent↔child contract).
fn parse_child_line(line: &str) -> Result<ChildReport> {
    serde_json::from_str(line).with_context(|| format!("parsing child JSON: {line}"))
}

/// Finds the compiled `open` binary next to the current executable (same target dir).
fn locate_open_bin() -> Result<PathBuf> {
    let me = std::env::current_exe().context("current_exe")?;
    let dir = me
        .parent()
        .ok_or_else(|| anyhow!("current exe has no parent dir"))?;
    let candidate = dir.join(if cfg!(windows) { "open.exe" } else { "open" });
    if candidate.exists() {
        Ok(candidate)
    } else {
        Err(anyhow!(
            "could not find `open` binary at {} — build it first (cargo build --release)",
            candidate.display()
        ))
    }
}

/// Per-stage aggregates (median, min, max) across runs, in nanoseconds.
struct Aggregate {
    read_ns_med: u64,
    parse_build_ns_med: u64,
    first_paint_ns_med: u64,
    first_eval_ns_med: u64,
    total_ns_med: u64,
    total_ns_min: u64,
    total_ns_max: u64,
}

fn aggregate(reports: &[ChildReport]) -> Aggregate {
    let field = |f: fn(&OpenStages) -> u64| -> Vec<u64> {
        let mut v: Vec<u64> = reports.iter().map(|r| f(&r.stages)).collect();
        v.sort_unstable();
        v
    };
    let med = |v: &[u64]| v[v.len() / 2];
    let totals = field(|s| s.total_ns);
    Aggregate {
        read_ns_med: med(&field(|s| s.read_ns)),
        parse_build_ns_med: med(&field(|s| s.parse_build_ns)),
        first_paint_ns_med: med(&field(|s| s.first_paint_ns)),
        first_eval_ns_med: med(&field(|s| s.first_eval_ns)),
        total_ns_med: med(&totals),
        total_ns_min: *totals.first().unwrap(),
        total_ns_max: *totals.last().unwrap(),
    }
}

/// Names the dominant open stage by median cost.
fn dominant_stage(agg: &Aggregate) -> String {
    let stages = [
        ("file read", agg.read_ns_med),
        (
            "parse+build (unzip/XML/shared-strings/styles/graph)",
            agg.parse_build_ns_med,
        ),
        ("first eval (full recompute)", agg.first_eval_ns_med),
    ];
    stages
        .iter()
        .max_by_key(|(_, ns)| *ns)
        .map(|(name, ns)| format!("{name} ({:.3}s)", ns_s(*ns)))
        .unwrap_or_else(|| "unknown".to_string())
}

#[allow(clippy::too_many_arguments)]
fn write_results(
    spec: &GenSpec,
    file_bytes: u64,
    uncompressed_bytes: u64,
    peak_rss: u64,
    rss_multiple_file: f64,
    rss_multiple_uncompressed: f64,
    agg: &Aggregate,
    reports: &[ChildReport],
    time_pass: bool,
    rss_pass: bool,
    offramp: bool,
    dominant: &str,
) -> Result<()> {
    let env = Environment::detect(commit()).with_cpu(cpu_model());

    // Per-run + aggregate stage timings as an env-stamped BenchResult.
    let per_run: Vec<_> = reports
        .iter()
        .map(|r| {
            json!({
                "read_s": ns_s(r.stages.read_ns),
                "parse_build_s": ns_s(r.stages.parse_build_ns),
                "first_paint_s": ns_s(r.stages.first_paint_ns),
                "first_eval_s": ns_s(r.stages.first_eval_ns),
                "total_s": ns_s(r.stages.total_ns),
                "peak_rss_bytes": r.peak_rss_bytes,
            })
        })
        .collect();

    let timings = BenchResult::new(
        "sp2-xlsx-open",
        spec.total_cells(),
        report_date(),
        env.clone(),
    )
    .with_extra(json!({
        "file_bytes": file_bytes,
        "uncompressed_bytes": uncompressed_bytes,
        "sheets": spec.sheets,
        "rows": spec.rows,
        "cols": spec.cols,
        "seed": spec.seed,
        "median": {
            "read_s": ns_s(agg.read_ns_med),
            "parse_build_s": ns_s(agg.parse_build_ns_med),
            "first_paint_s": ns_s(agg.first_paint_ns_med),
            "first_eval_s": ns_s(agg.first_eval_ns_med),
            "total_s": ns_s(agg.total_ns_med),
            "total_min_s": ns_s(agg.total_ns_min),
            "total_max_s": ns_s(agg.total_ns_max),
        },
        "per_run": per_run,
    }));
    timings.write_json("results/open_stage_timings.json")?;

    // The summary: gate verdicts, TTFP, RSS multiple, dominant stage, off-ramp flag.
    let summary = json!({
        "experiment": "SP2 -- large styled .xlsx open",
        "file_mb": file_bytes as f64 / BYTES_PER_MB as f64,
        "uncompressed_mb": uncompressed_bytes as f64 / BYTES_PER_MB as f64,
        "peak_rss_mb": peak_rss as f64 / BYTES_PER_MB as f64,
        "rss_multiple_of_compressed_file": rss_multiple_file,
        "rss_multiple_of_uncompressed_payload": rss_multiple_uncompressed,
        "open_to_recompute_ready_median_s": ns_s(agg.total_ns_med),
        "time_to_first_paint_median_s": ns_s(agg.first_paint_ns_med),
        "first_eval_median_s": ns_s(agg.first_eval_ns_med),
        "dominant_stage": dominant,
        "gate_time_seconds_not_minutes": time_pass,
        "gate_rss_sane_multiple": rss_pass,
        "off_ramp_triggered": offramp,
        "thresholds": {
            "open_seconds_ceiling": OPEN_SECONDS_CEILING,
            "rss_uncompressed_multiple_ceiling": RSS_UNCOMPRESSED_MULTIPLE_CEILING,
            "off_ramp_uncompressed_multiple": OFF_RAMP_UNCOMPRESSED_MULTIPLE,
        },
        "environment": {
            "os": env.os,
            "arch": env.arch,
            "cores": env.cores,
            "cpu": env.cpu,
            "commit": env.commit,
        },
    });
    std::fs::write(
        "results/open_summary.json",
        serde_json::to_string_pretty(&summary)?,
    )?;

    // Human-readable env stamp.
    let env_txt = format!(
        "SP2 environment\n\
         os={} arch={} cores={} cpu={}\n\
         commit={}\n\
         ironcalc=0.7.1 (pinned, same as round-2 harness)\n\
         file_bytes={} ({:.1} MB compressed)\n\
         uncompressed_bytes={} ({:.1} MB)\n\
         spec: seed={} sheets={} rows={} cols={} total_cells={}\n",
        env.os,
        env.arch,
        env.cores,
        env.cpu,
        env.commit,
        file_bytes,
        file_bytes as f64 / BYTES_PER_MB as f64,
        uncompressed_bytes,
        uncompressed_bytes as f64 / BYTES_PER_MB as f64,
        spec.seed,
        spec.sheets,
        spec.rows,
        spec.cols,
        spec.total_cells(),
    );
    std::fs::write("results/env.txt", env_txt)?;
    Ok(())
}

fn ns_s(ns: u64) -> f64 {
    ns as f64 / 1e9
}

fn pass_str(b: bool) -> &'static str {
    if b {
        "PASS"
    } else {
        "FAIL"
    }
}

/// Best-effort commit hash (env override, else "unknown" — recording must not shell out
/// to git in deterministic code; the CI-of-record can pass SP2_COMMIT).
fn commit() -> String {
    std::env::var("SP2_COMMIT").unwrap_or_else(|_| "unknown".to_string())
}

/// A relative report date (env override, else the fixed spec date). Recording must not
/// read a wall clock (bench_util determinism boundary).
fn report_date() -> String {
    std::env::var("SP2_DATE").unwrap_or_else(|_| "2026-07-01".to_string())
}

/// Best-effort CPU model from `/proc/cpuinfo` (Linux); empty otherwise.
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
    fn parse_child_line_roundtrips() {
        // Prove the parent↔child JSON contract without spawning a process.
        let stages = OpenStages {
            read_ns: 1_000_000,
            parse_build_ns: 2_000_000,
            first_paint_ns: 3_000_000,
            first_eval_ns: 4_000_000,
            total_ns: 7_000_000,
        };
        let child = ChildReport {
            file_bytes: 123_456,
            uncompressed_bytes: 654_321,
            peak_rss_bytes: 789_000,
            stages,
        };
        let line = serde_json::to_string(&child).unwrap();
        let back = parse_child_line(&line).unwrap();
        assert_eq!(back.file_bytes, 123_456);
        assert_eq!(back.uncompressed_bytes, 654_321);
        assert_eq!(back.peak_rss_bytes, 789_000);
        assert_eq!(back.stages.total_ns, 7_000_000);
        assert_eq!(back.stages.first_paint_ns, 3_000_000);
    }

    #[test]
    fn parse_child_line_rejects_garbage() {
        assert!(parse_child_line("not json").is_err());
    }

    #[test]
    fn aggregate_medians_and_dominant_stage() {
        let mk = |read, build, eval| ChildReport {
            file_bytes: 1000,
            uncompressed_bytes: 4000,
            peak_rss_bytes: 2000,
            stages: OpenStages {
                read_ns: read,
                parse_build_ns: build,
                first_paint_ns: read + build,
                first_eval_ns: eval,
                total_ns: read + build + eval,
            },
        };
        // parse+build dominates.
        let reports = vec![mk(10, 100, 30), mk(12, 120, 33), mk(11, 110, 31)];
        let agg = aggregate(&reports);
        assert_eq!(agg.read_ns_med, 11);
        assert_eq!(agg.parse_build_ns_med, 110);
        assert_eq!(agg.first_eval_ns_med, 31);
        assert!(dominant_stage(&agg).starts_with("parse+build"));
    }
}
