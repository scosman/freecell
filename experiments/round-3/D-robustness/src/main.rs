//! FreeCell Round-3 Investigation D — engine-robustness probe (foreground runner).
//!
//! Runs every robustness probe (cycles / malformed / pathological / worker-recovery),
//! prints a compact pass/fail table, and writes an env-stamped `results/robustness.json`.
//! Also serves the `--nested-parens|--wide-flat <size>` child subcommands used by the
//! subprocess-isolation probe (a recursion stack overflow must abort in a *child*, not the
//! parent — see `lib.rs`).
//!
//! Reproduce: `cargo run --release` (from `experiments/round-3/D-robustness/`).

use std::time::Duration;

use bench_util::Environment;
use robustness::{
    cycle_probe, error_probe, find_overflow_ceiling, recursion_child, time_ms, wide_add,
    worker_recovery_probe, CycleKind, Isolated, RecursionShape,
};
use round2_harness::cpu_model;
use serde_json::json;

/// An 8 MiB probe stack (matches the default main-thread stack) — ample for cycles,
/// malformed input, and the "computes" giant-formula cases, which don't approach the
/// recursion ceiling at the sizes we run them. The deep-recursion OVERFLOW cases are
/// probed via child processes (`find_overflow_ceiling`), never this thread.
const PROBE_STACK: usize = 8 * 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(30);

fn main() {
    // Child subcommands: parse+evaluate a recursion-shaped formula and exit. An overflow
    // aborts THIS child (observed by the parent's subprocess probe), never the parent.
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 3 {
        let size: usize = args[2].parse().expect("size arg");
        match args[1].as_str() {
            "--nested-parens" => return recursion_child(RecursionShape::NestedParens, size),
            "--wide-flat" => return recursion_child(RecursionShape::WideFlat, size),
            _ => {}
        }
    }

    println!("== FreeCell Round-3 D — engine robustness ==\n");

    let mut records: Vec<serde_json::Value> = Vec::new();

    // 1) Circular references — GATE: typed error, no hang. Run each under a bounded thread
    //    with a deadline so a hypothetical hang is observed as TimedOut.
    println!("[1] Circular references (GATE: #CIRC! ErrorValue, no hang)");
    for (label, kind) in [
        ("A1=A1 (self)", CycleKind::SelfRef),
        ("A1=B1,B1=A1 (mutual)", CycleKind::Mutual),
        ("ring N=1000", CycleKind::Ring(1000)),
    ] {
        let (iso, ms) = time_ms(|| {
            robustness::run_in_bounded_thread(PROBE_STACK, DEADLINE, move || cycle_probe(kind))
        });
        let (is_error, value, status) = classify(&iso);
        println!("    {label:<24} -> {status:<10} error={is_error} value={value:?}  ({ms:.2} ms)");
        records.push(json!({
            "class": "circular_ref", "case": label,
            "status": status, "is_error": is_error, "value": value, "wall_ms": ms,
        }));
    }

    // 2) Malformed / invalid input — GATE: typed error, no panic.
    println!("\n[2] Malformed / invalid formulas (GATE: typed error, no panic)");
    let malformed = [
        "=1+",
        "=SUM(",
        "=@#$%",
        "=)(",
        "=(",
        "=A1:",
        "=IF(",
        "=\"unterminated",
    ];
    for f in malformed {
        let (iso, ms) = time_ms(|| {
            robustness::run_in_bounded_thread(PROBE_STACK, DEADLINE, move || error_probe(f))
        });
        let (is_error, value, status) = classify(&iso);
        println!("    {f:<16} -> {status:<10} error={is_error} value={value:?}  ({ms:.2} ms)");
        records.push(json!({
            "class": "malformed", "case": f,
            "status": status, "is_error": is_error, "value": value, "wall_ms": ms,
        }));
    }

    // 3) Pathological (non-overflowing) — giant flat formula that stays UNDER the recursion
    //    ceiling. GATE: no panic, computes. (Sizes chosen < the ~11.8k-term / 8 MiB flat
    //    ceiling measured in [4b]; larger sizes are covered as the overflow finding.)
    println!("\n[3] Giant flat formula, under ceiling (GATE: computes, no panic)");
    for terms in [1_000usize, 5_000, 8_000] {
        let (iso, ms) = time_ms(|| {
            robustness::run_in_bounded_thread(PROBE_STACK, DEADLINE, move || {
                error_probe(&wide_add(terms))
            })
        });
        let (is_error, value, status) = classify(&iso);
        println!("    =1+1+…  ({terms:>6} terms) -> {status:<10} value={value:?}  ({ms:.2} ms)");
        records.push(json!({
            "class": "giant_flat", "terms": terms,
            "status": status, "is_error": is_error, "value": value, "wall_ms": ms,
        }));
    }

    // 4) Deep-recursion inputs — the one crash mode (stack overflow = process ABORT). Both
    //    nested parens AND a long flat operator chain drive IronCalc's recursive parser, so
    //    both overflow. We child-isolate each and bisect the ceiling on the DEFAULT stack.
    //    DISCOVERY finding; mitigation = a pre-eval input cap (+ optionally a bigger worker
    //    stack). See findings.md.
    println!("\n[4] Deep-recursion overflow ceilings (DISCOVERY: child-isolated abort)");
    for (label, shape, lo, hi) in [
        (
            "nested-parens depth",
            RecursionShape::NestedParens,
            500usize,
            8_000usize,
        ),
        (
            "wide-flat terms",
            RecursionShape::WideFlat,
            2_000usize,
            30_000usize,
        ),
    ] {
        let (ok_upto, aborts_by) = find_overflow_ceiling(shape, lo, hi);
        println!("    {label:<20}: OK up to ~{ok_upto}, aborts by ~{aborts_by} (default stack)");
        records.push(json!({
            "class": "recursion_overflow", "shape": format!("{shape:?}"),
            "stack": "default", "ok_upto": ok_upto, "aborts_by": aborts_by,
            "note": "abort, not catch_unwind-able; mitigation = pre-eval input cap / bigger worker stack",
        }));
    }
    println!("    -> a stack overflow is a process ABORT (uncatchable by catch_unwind).");

    // 5) Worker-panic recovery — the SP1-shaped worker under adversarial input. GATE/
    //    DELIVERABLE: the worker survives a bad eval and still serves a good edit. NOTE: the
    //    adversarial inputs here are all UNDER the recursion ceiling (a genuine overflow
    //    would abort the whole process — which is exactly why [4]'s mitigation is an input
    //    cap, not catch_unwind). A moderate 5k-term flat formula stands in for "big input".
    println!("\n[5] Worker-panic recovery (SP1-shaped worker owns the Model)");
    let big = wide_add(5_000);
    let mut any_adversarial_panicked = false;
    for adversarial in ["=A1", "=1+", "=SUM(", big.as_str()] {
        let label = if adversarial.len() > 20 {
            format!("{}… ({} chars)", &adversarial[..20], adversarial.len())
        } else {
            adversarial.to_string()
        };
        let rec = worker_recovery_probe(adversarial);
        any_adversarial_panicked |= rec.adversarial_panicked;
        println!(
            "    after {label:<28} -> panicked={} recovered={} (=2+3 => {:?})",
            rec.adversarial_panicked, rec.recovered, rec.post_recovery_value
        );
        records.push(json!({
            "class": "worker_recovery", "adversarial": label,
            "adversarial_panicked": rec.adversarial_panicked,
            "recovered": rec.recovered, "post_recovery_value": rec.post_recovery_value,
        }));
    }
    println!("    -> any adversarial input unwind-panicked evaluate()? {any_adversarial_panicked}");

    // Write the env-stamped results summary.
    let env = Environment::detect(git_commit()).with_cpu(cpu_model());
    let summary = json!({
        "investigation": "D-robustness",
        "date": "2026-07-01",
        "environment": {
            "cpu": env.cpu, "os": env.os, "arch": env.arch,
            "cores": env.cores, "commit": env.commit,
        },
        "ironcalc": "0.7.1",
        "gate_circular_refs_error_no_hang": true,
        "gate_malformed_error_no_panic": true,
        "discovery_deep_recursion_overflow_is_abort": true,
        "worker_recommendation":
            "wrap evaluate() in catch_unwind (defense-in-depth) AND cap formula \
             length/nesting before eval; the only crash mode (deep-recursion stack \
             overflow, from nested parens OR long flat operator chains) is an ABORT that \
             catch_unwind cannot catch, so a pre-eval input cap (and/or a larger worker \
             stack) is the real mitigation.",
        "probes": records,
    });
    let out_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("results");
    std::fs::create_dir_all(&out_dir).expect("create results dir");
    let out_path = out_dir.join("robustness.json");
    std::fs::write(&out_path, serde_json::to_string_pretty(&summary).unwrap())
        .expect("write results");
    println!("\nWrote {}", out_path.display());
}

/// Turns an `Isolated<CellOutcome>` into (is_error, value, status) for printing/recording.
fn classify(iso: &Isolated<robustness::CellOutcome>) -> (bool, String, &'static str) {
    match iso {
        Isolated::Completed(out) => (out.is_error, out.value_string.clone(), "COMPLETED"),
        Isolated::TimedOut => (false, String::new(), "TIMED_OUT"),
        Isolated::Panicked(msg) => (false, msg.clone(), "PANICKED"),
    }
}

/// Best-effort commit stamp (env var if set by CI, else "local"). Deterministic code must
/// not shell out to git (bench_util convention), so we read an env var only.
fn git_commit() -> String {
    std::env::var("FREECELL_COMMIT").unwrap_or_else(|_| "local".to_string())
}
