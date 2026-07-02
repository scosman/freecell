//! Runs the bake-off scenario suite against **IronCalc** and writes recorded results to
//! `results/ironcalc/` plus the shared `results/summary.md`, printing each gate's
//! PASS/FAIL to stdout (functional_spec §5.3, §5.4). Identical driving logic to the
//! Formualizer bin (both call `binding_common::run_suite`), so numbers are comparable.
//!
//! Reproduce (from `experiments/02-datamodel-binding-perf/ironcalc/`):
//! ```sh
//! cargo run --release --bin scenarios -- full 783a515   # full suite, stamped commit
//! cargo run --release --bin scenarios -- dev            # tiny smoke profile
//! cargo run --release --bin scenarios -- mem 783a515    # ONLY the memory scenario, in
//!                                                        # a fresh process → clean peak RSS
//! ```
//! Run `mem` **separately** (its own process) so `VmHWM` reflects only the memory load.

use bench_util::Environment;
use binding_common::sysinfo::{cpu_model, peak_rss_bytes};
use binding_common::{run_memory_only, run_suite, write_all, Profile, SpreadsheetEngine};
use ironcalc_bench::IronCalcEngine;

/// The report date for this phase (passed in — recording never reads a clock).
const REPORT_DATE: &str = "2026-07-01";

fn main() {
    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap_or_else(|| "full".to_string());
    let commit = args.next().unwrap_or_else(|| "unknown".to_string());
    let (profile, memory_only) = match mode.as_str() {
        "dev" => (Profile::dev(), false),
        "mem" => (Profile::full(), true),
        "mem-dev" => (Profile::dev(), true),
        _ => (Profile::full(), false),
    };

    let env = Environment::detect(commit).with_cpu(cpu_model());
    println!(
        "IronCalc scenarios ({mode}) — os={} arch={} cores={} cpu={:?}",
        env.os, env.arch, env.cores, env.cpu
    );

    let results = if memory_only {
        vec![run_memory_only(
            IronCalcEngine::new_blank,
            &profile,
            &env,
            REPORT_DATE,
            peak_rss_bytes,
        )]
    } else {
        run_suite(
            IronCalcEngine::new_blank,
            &profile,
            &env,
            REPORT_DATE,
            peak_rss_bytes,
        )
    };

    for r in &results {
        for g in &r.result.gates {
            g.print();
        }
    }

    let results_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../results");
    write_all(results_dir, &results).expect("write results");
    println!("Wrote {} records to {results_dir}", results.len());
}
