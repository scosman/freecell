//! Investigation D — the GATE assertions (functional_spec §6-D pass criteria).
//!
//! - Circular refs → typed `#CIRC!` error, and **return at all** (no hang): each cycle
//!   probe runs under a bounded-thread deadline; a hang would surface as `TimedOut` and
//!   fail the assertion instead of wedging the test process.
//! - Malformed / giant input → typed error, **no panic**.
//! - Deeply-nested parens → the honest finding: a bounded ceiling beyond which the parser
//!   stack-overflows (an ABORT). Asserted via child-process isolation so the test process
//!   itself is never killed.
//! - Worker-panic recovery → the SP1-shaped worker survives a bad eval and still serves a
//!   subsequent good edit.

use std::time::Duration;

use std::path::PathBuf;

use robustness::{
    cycle_probe, error_probe, run_in_bounded_thread, run_recursion_in_subprocess_with, wide_add,
    worker_recovery_probe, CycleKind, Isolated, RecursionShape,
};

const STACK: usize = 1024 * 1024; // 1 MiB — ample for cycles/malformed (they don't recurse deep)
const DEADLINE: Duration = Duration::from_secs(20);

/// The built `robustness` binary (which carries the `--nested-parens|--wide-flat`
/// subcommand dispatch). Cargo injects `CARGO_BIN_EXE_robustness` for integration tests.
/// The subprocess-isolation probes MUST target this, NOT the test harness's `current_exe`.
fn robustness_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_robustness"))
}

/// Helper: run a probe under the bounded-thread deadline and require it Completed (proving
/// no hang), returning the `CellOutcome`.
fn completed(iso: Isolated<robustness::CellOutcome>) -> robustness::CellOutcome {
    match iso {
        Isolated::Completed(out) => out,
        Isolated::TimedOut => panic!("probe HUNG (TimedOut) — GATE violation"),
        Isolated::Panicked(msg) => panic!("probe PANICKED: {msg} — GATE violation"),
    }
}

#[test]
fn circular_self_ref_returns_error_not_hang() {
    let out = completed(run_in_bounded_thread(STACK, DEADLINE, || {
        cycle_probe(CycleKind::SelfRef)
    }));
    assert!(out.is_error, "A1=A1 must be an error cell, got {out:?}");
    assert_eq!(out.value_string, "#CIRC!", "expected #CIRC!, got {out:?}");
}

#[test]
fn circular_mutual_ref_returns_error() {
    let out = completed(run_in_bounded_thread(STACK, DEADLINE, || {
        cycle_probe(CycleKind::Mutual)
    }));
    assert!(
        out.is_error,
        "mutual cycle head must be an error, got {out:?}"
    );
    assert_eq!(out.value_string, "#CIRC!");
}

#[test]
fn circular_long_ring_returns_error_no_stack_overflow() {
    // A 1000-cell ring: the marker-based cycle guard must still terminate with #CIRC!
    // rather than recursing the whole ring (which would risk a stack overflow).
    let out = completed(run_in_bounded_thread(STACK, DEADLINE, || {
        cycle_probe(CycleKind::Ring(1000))
    }));
    assert!(out.is_error, "ring head must be an error, got {out:?}");
    assert_eq!(out.value_string, "#CIRC!");
}

#[test]
fn invalid_formulas_are_errors_not_panics() {
    // A corpus of syntactically-invalid / pathological formulas. Each must evaluate to a
    // typed error (ErrorValue) WITHOUT panicking. The bounded thread would surface a panic
    // as `Panicked`; `completed()` fails on that.
    let corpus = [
        "=1+",
        "=SUM(",
        "=@#$%",
        "=)(",
        "=(",
        "=A1:",
        "=IF(",
        "=1/0",
        "=0/0",
        "=1E308*1E308",
        "=SQRT(-1)",
    ];
    for f in corpus {
        let out = completed(run_in_bounded_thread(STACK, DEADLINE, move || {
            error_probe(f)
        }));
        assert!(
            out.is_error,
            "malformed/pathological input {f:?} should be a typed error, got {out:?}"
        );
    }
}

#[test]
fn unterminated_string_is_recovered_gracefully_not_a_panic() {
    // Adversarial control: `="unterminated` is NOT an error — IronCalc's lexer recovers it
    // as the plain string `unterminated`. The point is GRACEFUL handling (no panic), which
    // is what the GATE requires; it just happens to be a value rather than an error cell.
    let out = completed(run_in_bounded_thread(STACK, DEADLINE, || {
        error_probe("=\"unterminated")
    }));
    assert!(
        !out.is_error,
        "expected graceful string recovery, got {out:?}"
    );
    assert_eq!(out.value_string, "unterminated");
}

#[test]
fn giant_flat_formula_under_ceiling_computes_without_panic() {
    // An 8k-term flat sum computes (no panic, no error) on an 8 MiB stack — under the
    // ~11.8k-term flat overflow ceiling measured for that stack. (A LONGER flat chain is
    // itself a recursion vector — see `deep_recursion_overflow_is_contained_in_subprocess`
    // for the WideFlat overflow case; that is why the size is bounded here.)
    let out = completed(run_in_bounded_thread(8 * 1024 * 1024, DEADLINE, || {
        error_probe(&wide_add(8_000))
    }));
    assert!(
        !out.is_error,
        "giant flat formula should compute, got {out:?}"
    );
    assert_eq!(out.value_string, "8000");
}

#[test]
fn shallow_recursion_is_fine() {
    // Nesting/width well under the ceiling parses & evaluates cleanly (control for the
    // overflow finding). Child-isolated so it's uniform with the ceiling probe.
    let bin = robustness_bin();
    for (shape, size) in [
        (RecursionShape::NestedParens, 200usize),
        (RecursionShape::WideFlat, 500usize),
    ] {
        let out = run_recursion_in_subprocess_with(&bin, shape, size);
        assert!(
            out.survived,
            "{shape:?} size {size} should survive, got {out:?}"
        );
    }
}

#[test]
fn deep_recursion_overflow_is_contained_in_subprocess() {
    // THE finding: deep recursion (BOTH nested parens AND a long flat operator chain)
    // overflows IronCalc's recursive-descent parser and ABORTS. This is uncatchable by
    // catch_unwind — but child-process isolation contains it: the child exits
    // non-successfully and OUR process lives. Sizes chosen well past the default-stack
    // ceilings (~2637 nesting depth / ~11.8k flat terms on 8 MiB).
    let bin = robustness_bin();
    for (shape, size) in [
        (RecursionShape::NestedParens, 20_000usize),
        (RecursionShape::WideFlat, 60_000usize),
    ] {
        let out = run_recursion_in_subprocess_with(&bin, shape, size);
        assert!(
            !out.survived,
            "{shape:?} size {size} is expected to overflow/abort in the child; if it \
             survived, the parser gained a depth cap — update the finding. Got {out:?}"
        );
    }
    // Crucially, we (the parent/test process) are still running to make these assertions.
}

#[test]
fn worker_survives_bad_eval_and_recovers() {
    // The SP1-shaped worker owns the Model; feed it an adversarial input, then a good edit.
    // Empirically no user input unwind-panics evaluate(), so the worker never even needs to
    // catch — but the catch_unwind wrapper + a surviving worker are what we assert. A good
    // `=2+3` must still come back as 5 afterward (the worker was not poisoned).
    for adversarial in [
        "=A1",   /*cycle*/
        "=1+",   /*malformed*/
        "=SUM(", /*malformed*/
    ] {
        let rec = worker_recovery_probe(adversarial);
        assert!(
            rec.recovered,
            "worker must survive adversarial {adversarial:?} and still evaluate =2+3; got {rec:?}"
        );
        assert_eq!(rec.post_recovery_value, "5");
        // Document the observed fact: none of these unwind-panic the eval.
        assert!(
            !rec.adversarial_panicked,
            "no user input is expected to unwind-panic evaluate(); {adversarial:?} did — \
             that would strengthen the case for catch_unwind. Got {rec:?}"
        );
    }
}
