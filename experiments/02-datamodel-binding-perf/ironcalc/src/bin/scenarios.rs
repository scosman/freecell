//! Runs the full bake-off scenario suite against **IronCalc** and writes recorded
//! results to `results/ironcalc/` plus the shared `results/summary.md`, printing each
//! gate's PASS/FAIL to stdout (functional_spec §5.3, §5.4). Identical driving logic to
//! the Formualizer bin (both call `binding_common::run_suite`), so the numbers are
//! directly comparable.
//!
//! Reproduce (from `experiments/02-datamodel-binding-perf/ironcalc/`):
//! ```sh
//! cargo run --release --bin scenarios            # spec-scale ("full") profile
//! cargo run --release --bin scenarios -- dev     # tiny profile for a smoke run
//! cargo run --release --bin scenarios -- full 783a515   # stamp the commit
//! ```

use bench_util::Environment;
use binding_common::sysinfo::{cpu_model, peak_rss_bytes};
use binding_common::{run_suite, write_all, Profile, SpreadsheetEngine};
use ironcalc_bench::IronCalcEngine;

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
        "IronCalc scenarios — os={} arch={} cores={} cpu={:?}",
        env.os, env.arch, env.cores, env.cpu
    );

    let results = run_suite(
        IronCalcEngine::new_blank,
        &profile,
        &env,
        REPORT_DATE,
        peak_rss_bytes,
    );

    for r in &results {
        for g in &r.result.gates {
            g.print();
        }
    }

    let results_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../results");
    write_all(results_dir, &results).expect("write results");
    println!("Wrote {} records to {results_dir}", results.len());
}
