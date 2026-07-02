//! Adapter faithfulness tests: the Formualizer [`FormualizerEngine`] must map the
//! shared binding surface correctly (values, formulas/cascade, native range read,
//! changelog dirty drain, capability flags) so the shared scenarios measure the real
//! engine, not an adapter bug.

use binding_common::binding::{read_under, Design};
use binding_common::{CellInput, EngineValue, SpreadsheetEngine, Viewport};
use formualizer_bench::FormualizerEngine;

#[test]
fn sets_and_reads_value() {
    let mut e = FormualizerEngine::new_blank();
    e.set_value(0, 0, EngineValue::Number(42.0));
    assert_eq!(e.get_value(0, 0), EngineValue::Number(42.0));
    e.set_value(3, 5, EngineValue::Text("hi".into()));
    assert_eq!(e.get_value(3, 5), EngineValue::Text("hi".into()));
}

#[test]
fn evaluate_cell_reflects_precedent_edit() {
    // A1=1, A2=2, A3==A1+A2 -> 3; edit A1:=10 -> A3 == 12.
    let mut e = FormualizerEngine::new_blank();
    e.set_batch(&[
        (0, 0, CellInput::Value(EngineValue::Number(1.0))),
        (1, 0, CellInput::Value(EngineValue::Number(2.0))),
        (2, 0, CellInput::Formula("=A1+A2".into())),
    ]);
    assert_eq!(e.evaluate_cell(2, 0), EngineValue::Number(3.0));

    e.set_value(0, 0, EngineValue::Number(10.0));
    e.recompute();
    assert_eq!(e.get_value(2, 0), EngineValue::Number(12.0));
}

#[test]
fn linear_chain_cascades() {
    let mut e = FormualizerEngine::new_blank();
    e.set_batch(&[
        (0, 0, CellInput::Formula("=1".into())),
        (1, 0, CellInput::Formula("=A1+1".into())),
        (2, 0, CellInput::Formula("=A2+1".into())),
        (3, 0, CellInput::Formula("=A3+1".into())),
    ]);
    assert_eq!(e.get_value(3, 0), EngineValue::Number(4.0));
    // Edit the head, recompute, tail follows.
    e.set_value(0, 0, EngineValue::Number(10.0));
    e.recompute();
    assert_eq!(e.get_value(3, 0), EngineValue::Number(13.0));
}

#[test]
fn read_viewport_matches_get_value() {
    let mut e = FormualizerEngine::new_blank();
    for r in 0..5 {
        for c in 0..5 {
            e.set_value(r, c, EngineValue::Number((r * 10 + c) as f64));
        }
    }
    let vp = Viewport::new(1, 1, 3, 3);
    let bulk = e.read_viewport(vp);
    let per_cell: Vec<EngineValue> = vp.addresses().map(|(r, c)| e.get_value(r, c)).collect();
    assert_eq!(bulk, per_cell);
}

#[test]
fn changelog_drain_reports_edited_cell() {
    let mut e = FormualizerEngine::new_blank();
    e.enable_change_tracking();
    e.set_value(2, 3, EngineValue::Number(1.0));
    e.set_value(7, 1, EngineValue::Number(2.0));
    let dirty = e.drain_dirty();
    assert!(dirty.contains(&(2, 3)), "dirty was {dirty:?}");
    assert!(dirty.contains(&(7, 1)), "dirty was {dirty:?}");
    // A second drain with no edits is empty.
    assert!(e.drain_dirty().is_empty());
}

#[test]
fn caps_flags_are_correct() {
    let e = FormualizerEngine::new_blank();
    let caps = e.caps();
    assert!(caps.native_range_read);
    assert!(caps.incremental_recalc);
    assert!(caps.parallel_eval);
    assert!(caps.change_log);
    assert!(!caps.styles_on_read); // 0.7 hard-codes style:None on read
}

#[test]
fn d1_d2_d3_agree_on_real_engine() {
    let mut e = FormualizerEngine::new_blank();
    for r in 0..8 {
        for c in 0..8 {
            e.set_value(r, c, EngineValue::Number((r * 100 + c) as f64));
        }
    }
    let vp = Viewport::new(2, 2, 4, 4);
    let d1 = read_under(Design::NaivePerCell, &e, vp);
    let d2 = read_under(Design::BulkRange, &e, vp);
    let d3 = read_under(Design::CachedChangelog, &e, vp);
    assert_eq!(d1, d2);
    assert_eq!(d2, d3);
}
