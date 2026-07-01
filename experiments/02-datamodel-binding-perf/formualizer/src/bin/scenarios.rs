//! Runs the full bake-off scenario suite against **Formualizer** and writes recorded
//! results to `results/formualizer/` plus the shared `results/summary.md`, printing
//! each gate's PASS/FAIL to stdout (functional_spec §5.3, §5.4).
//!
//! Reproduce (from `experiments/02-datamodel-binding-perf/formualizer/`):
//! ```sh
//! cargo run --release --bin scenarios            # spec-scale ("full") profile
//! cargo run --release --bin scenarios -- dev     # tiny profile for a smoke run
//! ```
//! The commit stamp defaults to `unknown`; pass it as the 2nd arg for a clean record:
//! `cargo run --release --bin scenarios -- full 783a515`.

use bench_util::Environment;
use binding_common::sysinfo::{cpu_model, peak_rss_bytes};
use binding_common::{run_suite, write_all, Profile, SpreadsheetEngine};
use formualizer_bench::FormualizerEngine;

/// The report date for this phase (passed in — recording never reads a clock).
const REPORT_DATE: &str = "2026-07-01";

fn main() {
    let mut args = std::env::args().skip(1);
    let profile = match args.next().as_deref() {
        Some("dev") => Profile::dev(),
        _ => Profile::full(),
    };
    let commit = args.next().unwrap_or_else(|| "unknown".to_string());

    let env = Environment::detect(commit).with_cpu(cpu_model());
    println!(
        "Formualizer scenarios — os={} arch={} cores={} cpu={:?}",
        env.os, env.arch, env.cores, env.cpu
    );

    let results = run_suite(
        FormualizerEngine::new_blank,
        &profile,
        &env,
        REPORT_DATE,
        peak_rss_bytes,
    );

    // Print every gate verdict.
    for r in &results {
        for g in &r.result.gates {
            g.print();
        }
    }

    // The results/ dir lives one level up from this crate.
    let results_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../results");
    write_all(results_dir, &results).expect("write results");
    println!("Wrote {} records to {results_dir}", results.len());
}
