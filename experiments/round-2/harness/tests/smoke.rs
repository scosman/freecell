//! Smoke tests proving the COPIED IronCalc adapter works standalone through the copied
//! `SpreadsheetEngine` trait, and that the new fresh-process `peak_rss()` helper
//! returns a plausible non-zero figure. These are the harness's acceptance checks: if
//! the verbatim copy or the version pin drifted, they fail here rather than in a
//! downstream Round-2 experiment.

use round2_harness::{peak_rss, EngineValue, IronCalcEngine, SpreadsheetEngine, Viewport};

/// Construct the IronCalc adapter via the copied trait, set two literals + a formula,
/// recompute, and read the evaluated result back — the trivial end-to-end scenario the
/// phase requires, proving the adapter compiles and evaluates.
#[test]
fn smoke_ironcalc_adapter_roundtrip() {
    let mut engine = IronCalcEngine::new_blank();
    assert_eq!(engine.name(), "ironcalc");

    // A1 = 2, B1 = 3, C1 = =A1+B1. (0-based datagen coordinates; the adapter maps +1.)
    engine.set_value(0, 0, EngineValue::Number(2.0));
    engine.set_value(0, 1, EngineValue::Number(3.0));
    engine.set_formula(0, 2, "=A1+B1");

    // IronCalc is non-incremental: the adapter defers evaluation to recompute().
    engine.recompute();

    // Read the literals and the evaluated formula result back.
    assert_eq!(engine.get_value(0, 0), EngineValue::Number(2.0));
    assert_eq!(engine.get_value(0, 1), EngineValue::Number(3.0));
    assert_eq!(
        engine.get_value(0, 2),
        EngineValue::Number(5.0),
        "=A1+B1 should evaluate to 5 after recompute()"
    );
}

/// The neutral viewport read path through IronCalc returns the seeded cells in
/// row-major order.
#[test]
fn smoke_read_viewport() {
    let mut engine = IronCalcEngine::new_blank();
    engine.set_value(0, 0, EngineValue::Number(10.0));
    engine.set_value(0, 1, EngineValue::Number(20.0));
    engine.recompute();

    let vals = engine.read_viewport(Viewport::new(0, 0, 1, 2));
    assert_eq!(
        vals,
        vec![EngineValue::Number(10.0), EngineValue::Number(20.0)]
    );
}

/// The fresh-process peak-RSS helper returns a plausible non-zero byte count.
#[test]
fn peak_rss_is_plausible_nonzero() {
    let rss = peak_rss();
    assert!(
        rss > 1024 * 1024,
        "peak_rss too small to be real: {rss} bytes"
    );
    assert!(
        rss < 64u64 * 1024 * 1024 * 1024,
        "peak_rss implausibly large (unit bug?): {rss} bytes"
    );
}
