//! Adapter faithfulness tests: the IronCalc [`IronCalcEngine`] must map the shared
//! binding surface correctly so the shared scenarios measure the real engine. Note the
//! deferred-eval contract: `set_value`/`set_formula` don't evaluate; the caller (or a
//! `set_batch`) triggers the single full recompute — mirroring how the scenarios drive
//! IronCalc's non-incremental engine.

use binding_common::binding::{read_under, Design};
use binding_common::{CellInput, EngineValue, SpreadsheetEngine, Viewport};
use ironcalc_bench::IronCalcEngine;

#[test]
fn sets_and_reads_value() {
    let mut e = IronCalcEngine::new_blank();
    e.set_value(0, 0, EngineValue::Number(42.0));
    e.recompute();
    assert_eq!(e.get_value(0, 0), EngineValue::Number(42.0));
    e.set_value(3, 5, EngineValue::Text("hi".into()));
    e.recompute();
    assert_eq!(e.get_value(3, 5), EngineValue::Text("hi".into()));
}

#[test]
fn formula_cascades_after_recompute() {
    // A1=1, A2=2, A3==A1+A2 -> 3 via a batch (one full evaluate).
    let mut e = IronCalcEngine::new_blank();
    e.set_batch(&[
        (0, 0, CellInput::Value(EngineValue::Number(1.0))),
        (1, 0, CellInput::Value(EngineValue::Number(2.0))),
        (2, 0, CellInput::Formula("=A1+A2".into())),
    ]);
    assert_eq!(e.get_value(2, 0), EngineValue::Number(3.0));

    // Edit head, recompute (full), tail follows.
    e.set_value(0, 0, EngineValue::Number(10.0));
    e.recompute();
    assert_eq!(e.get_value(2, 0), EngineValue::Number(12.0));
}

#[test]
fn linear_chain_cascades() {
    let mut e = IronCalcEngine::new_blank();
    e.set_batch(&[
        (0, 0, CellInput::Formula("=1".into())),
        (1, 0, CellInput::Formula("=A1+1".into())),
        (2, 0, CellInput::Formula("=A2+1".into())),
        (3, 0, CellInput::Formula("=A3+1".into())),
    ]);
    assert_eq!(e.get_value(3, 0), EngineValue::Number(4.0));
    e.set_value(0, 0, EngineValue::Number(10.0));
    e.recompute();
    assert_eq!(e.get_value(3, 0), EngineValue::Number(13.0));
}

#[test]
fn read_viewport_matches_get_value() {
    let mut e = IronCalcEngine::new_blank();
    let batch: Vec<(u32, u32, CellInput)> = (0..5)
        .flat_map(|r| {
            (0..5).map(move |c| {
                (
                    r,
                    c,
                    CellInput::Value(EngineValue::Number((r * 10 + c) as f64)),
                )
            })
        })
        .collect();
    e.set_batch(&batch);
    let vp = Viewport::new(1, 1, 3, 3);
    let bulk = e.read_viewport(vp);
    let per_cell: Vec<EngineValue> = vp.addresses().map(|(r, c)| e.get_value(r, c)).collect();
    assert_eq!(bulk, per_cell);
}

#[test]
fn dirty_drain_reports_edited_cell() {
    let mut e = IronCalcEngine::new_blank();
    e.enable_change_tracking();
    e.set_value(2, 3, EngineValue::Number(1.0));
    e.set_value(7, 1, EngineValue::Number(2.0));
    let dirty = e.drain_dirty();
    assert!(dirty.contains(&(2, 3)), "dirty was {dirty:?}");
    assert!(dirty.contains(&(7, 1)), "dirty was {dirty:?}");
    assert!(e.drain_dirty().is_empty());
}

#[test]
fn caps_flags_are_correct() {
    let e = IronCalcEngine::new_blank();
    let caps = e.caps();
    assert!(!caps.native_range_read); // per-cell only
    assert!(!caps.incremental_recalc); // full evaluate()
    assert!(!caps.parallel_eval); // single-threaded
    assert!(caps.change_log); // UserModel diff-list
    assert!(caps.styles_on_read); // get_style_for_cell
}

#[test]
fn d1_d2_d3_agree_on_real_engine() {
    let mut e = IronCalcEngine::new_blank();
    let batch: Vec<(u32, u32, CellInput)> = (0..8)
        .flat_map(|r| {
            (0..8).map(move |c| {
                (
                    r,
                    c,
                    CellInput::Value(EngineValue::Number((r * 100 + c) as f64)),
                )
            })
        })
        .collect();
    e.set_batch(&batch);
    let vp = Viewport::new(2, 2, 4, 4);
    let d1 = read_under(Design::NaivePerCell, &e, vp);
    let d2 = read_under(Design::BulkRange, &e, vp);
    let d3 = read_under(Design::CachedChangelog, &e, vp);
    assert_eq!(d1, d2);
    assert_eq!(d2, d3);
}
