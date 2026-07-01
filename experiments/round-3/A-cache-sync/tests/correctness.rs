//! GATE correctness tests (functional_spec §6-A):
//!   - structural insert/delete of a row and a column shift formula references, row/column
//!     band styles, and sizes correctly (+ an `.xlsx` round-trip),
//!   - undo/redo covers value, style, and structural edits (structural undo fully un-shifts),
//!   - the resident-cache prototype **agrees with IronCalc** after every edit and every
//!     undo/redo (architecture §4.4 — the load-bearing test),
//!   - copy/paste relative-reference translation (via the public `extend_copied_value`),
//!   - the style interner dedups.

use cache_sync::cache::{Axis, ResidentCache, StyleId};
use cache_sync::harness::{
    self, assert_cache_agrees, build_sheet, cell_content, cell_display, engine_col, engine_row,
    hydrate_cache, SHEET,
};
use ironcalc_base::expressions::types::CellReferenceIndex;
use ironcalc_base::types::Style;
use ironcalc_base::UserModel;

const ROWS: i32 = 30;
const COLS: i32 = 6;

fn fresh() -> UserModel<'static> {
    let mut model = UserModel::new_empty("test", "en", "UTC", "en").unwrap();
    build_sheet(&mut model, ROWS).unwrap();
    model
}

/// Indices spanning an edit point for the agreement contract. Crucially includes the
/// **banded rows and their shifted positions** (±1 around each of 3/10/25) so a sample
/// actually lands where a band/size moved — otherwise the check has no discriminating
/// power (a negative control caught exactly this).
fn row_samples(at: i32) -> Vec<i32> {
    let mut s = vec![1, at - 1, at, at + 1, at + 5, ROWS];
    for &b in harness::BANDED_ROWS.iter() {
        s.extend([b - 1, b, b + 1]);
    }
    s.retain(|&r| r >= 1 && r <= ROWS + 2);
    s.sort_unstable();
    s.dedup();
    s
}
fn col_samples(at: i32) -> Vec<i32> {
    let mut s = vec![1, at - 1, at, at + 1, COLS];
    for &b in harness::BANDED_COLS.iter() {
        s.extend([b - 1, b, b + 1]);
    }
    s.retain(|&c| c >= 1 && c <= COLS + 2);
    s.sort_unstable();
    s.dedup();
    s
}

#[test]
fn send_probe_links() {
    cache_sync::probe::assert_usermodel_send();
    // Runtime seam probe returns the shifted, still-correct value.
    assert_eq!(cache_sync::probe::probe_worker_seam().unwrap(), "20");
}

#[test]
fn insert_row_shifts_refs_styles_sizes() {
    let mut model = fresh();
    // Before: B10 = A10 + A9 -> content references A10,A9; row 10 is banded + tall.
    let b10_before = cell_content(&model, 10, 2);
    assert!(b10_before.contains("A10") && b10_before.contains("A9"), "pre: {b10_before}");
    assert!(engine_row(&model, 10).unwrap().band_style.is_some(), "row 10 banded pre-insert");
    assert_eq!(engine_row(&model, 10).unwrap().size, harness::CUSTOM_ROW_HEIGHT);

    // Insert 1 row at index 4 (above the banded/formula rows at 10,25).
    model.insert_rows(SHEET, 4, 1).unwrap();

    // (a) formula reference shifted: what was B10 is now B11 and references A11,A10.
    let b11 = cell_content(&model, 11, 2);
    assert!(b11.contains("A11") && b11.contains("A10"), "post insert B11: {b11}");
    // (b) band style shifted from row 10 to row 11.
    assert!(engine_row(&model, 11).unwrap().band_style.is_some(), "band moved to row 11");
    assert!(engine_row(&model, 10).unwrap().band_style.is_none(), "row 10 no longer banded");
    // (c) custom height shifted from row 10 to row 11.
    assert_eq!(engine_row(&model, 11).unwrap().size, harness::CUSTOM_ROW_HEIGHT);

    // The SUM total (C1) still covers all A values: originally SUM(A1:A30)=465; after
    // inserting a blank row the range expands to A1:A31 and the sum is unchanged (blank=0).
    assert_eq!(cell_display(&model, 1, 3), "465", "SUM re-targets across the insert");
}

#[test]
fn delete_row_shifts_refs_styles_sizes() {
    let mut model = fresh();
    // Delete row 4 (below A3, above the banded row 10). Row 10 band should move to row 9.
    model.delete_rows(SHEET, 4, 1).unwrap();

    let b9 = cell_content(&model, 9, 2);
    assert!(b9.contains("A9") && b9.contains("A8"), "post delete B9: {b9}");
    assert!(engine_row(&model, 9).unwrap().band_style.is_some(), "band moved to row 9");
    assert_eq!(engine_row(&model, 9).unwrap().size, harness::CUSTOM_ROW_HEIGHT);
    // Deleting a blank data row (row 4 held A4=4); SUM loses 4 -> 465-4 = 461.
    assert_eq!(cell_display(&model, 1, 3), "461", "SUM reflects the deleted value");
}

#[test]
fn insert_delete_column_shifts_band_and_width() {
    let mut model = fresh();
    // Column 5 is banded (blue) + custom width. Insert a column at index 3 -> band moves
    // to column 6.
    assert!(engine_col(&model, 5).unwrap().band_style.is_some(), "col 5 banded pre");
    assert_eq!(engine_col(&model, 5).unwrap().size, harness::CUSTOM_COL_WIDTH);

    model.insert_columns(SHEET, 3, 1).unwrap();
    assert!(engine_col(&model, 6).unwrap().band_style.is_some(), "band moved to col 6");
    assert_eq!(engine_col(&model, 6).unwrap().size, harness::CUSTOM_COL_WIDTH);

    // Delete it again -> band returns to column 5.
    model.delete_columns(SHEET, 3, 1).unwrap();
    assert!(engine_col(&model, 5).unwrap().band_style.is_some(), "band back to col 5");
    assert_eq!(engine_col(&model, 5).unwrap().size, harness::CUSTOM_COL_WIDTH);
}

#[test]
fn xlsx_roundtrip_preserves_structural_edit() {
    let mut model = fresh();
    model.insert_rows(SHEET, 4, 1).unwrap();
    model.delete_columns(SHEET, 3, 1).unwrap();

    let dir = std::env::temp_dir().join(format!("cache_sync_xlsx_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("edited.xlsx");
    ironcalc::export::save_to_xlsx(model.get_model(), path.to_str().unwrap()).unwrap();

    let reloaded_model =
        ironcalc::import::load_from_xlsx(path.to_str().unwrap(), "en", "UTC", "en").unwrap();
    let reloaded = UserModel::from_model(reloaded_model);

    // Band + height survived the insert shift (row 10 -> row 11).
    assert!(engine_row(&reloaded, 11).unwrap().band_style.is_some(), "band survived xlsx");
    assert_eq!(engine_row(&reloaded, 11).unwrap().size, harness::CUSTOM_ROW_HEIGHT);
    // Formula reference survived.
    let b11 = cell_content(&reloaded, 11, 2);
    assert!(b11.contains("A11") && b11.contains("A10"), "formula survived xlsx: {b11}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn undo_redo_value_edit() {
    let mut model = UserModel::new_empty("uv", "en", "UTC", "en").unwrap();
    model.set_user_input(SHEET, 1, 1, "=6*7").unwrap();
    assert_eq!(cell_display(&model, 1, 1), "42");
    model.undo().unwrap();
    assert_eq!(cell_display(&model, 1, 1), "", "value undo reverts");
    model.redo().unwrap();
    assert_eq!(cell_display(&model, 1, 1), "42", "value redo reapplies");
}

#[test]
fn undo_redo_style_edit() {
    let mut model = fresh();
    // Bold cell A1 (a per-cell style edit) via a single-cell range.
    use ironcalc_base::expressions::types::Area;
    model.set_selected_cell(1, 1).unwrap();
    let a1 = Area { sheet: SHEET, row: 1, column: 1, width: 1, height: 1 };
    let before = model.get_model().get_style_for_cell(SHEET, 1, 1).unwrap().font.b;
    model.update_range_style(&a1, "font.b", "true").unwrap();
    assert!(model.get_model().get_style_for_cell(SHEET, 1, 1).unwrap().font.b, "bold applied");
    model.undo().unwrap();
    assert_eq!(model.get_model().get_style_for_cell(SHEET, 1, 1).unwrap().font.b, before, "style undo");
    model.redo().unwrap();
    assert!(model.get_model().get_style_for_cell(SHEET, 1, 1).unwrap().font.b, "style redo");
}

#[test]
fn undo_redo_structural_edit_fully_unshifts() {
    let mut model = fresh();
    let b10_before = cell_content(&model, 10, 2);
    let row10_size_before = engine_row(&model, 10).unwrap().size;

    model.insert_rows(SHEET, 4, 1).unwrap();
    assert!(cell_content(&model, 11, 2).contains("A11"), "insert shifted formula");

    model.undo().unwrap();
    // Fully un-shifted: row 10 formula, band, and size are back where they started.
    assert_eq!(cell_content(&model, 10, 2), b10_before, "structural undo restores formula");
    assert!(engine_row(&model, 10).unwrap().band_style.is_some(), "band restored to row 10");
    assert_eq!(engine_row(&model, 10).unwrap().size, row10_size_before, "size restored");

    model.redo().unwrap();
    assert!(cell_content(&model, 11, 2).contains("A11"), "structural redo re-shifts");
}

#[test]
fn cache_agrees_after_insert_row() {
    let mut model = fresh();
    let mut cache = hydrate_cache(&model, ROWS, COLS).unwrap();
    assert_cache_agrees(&model, &mut cache, &row_samples(4), &col_samples(3)).unwrap();

    model.insert_rows(SHEET, 4, 1).unwrap();
    cache.shift_rows(4, 1);
    assert_cache_agrees(&model, &mut cache, &row_samples(4), &col_samples(3))
        .expect("cache must agree with IronCalc after insert row");
}

#[test]
fn cache_agrees_after_delete_row() {
    let mut model = fresh();
    let mut cache = hydrate_cache(&model, ROWS, COLS).unwrap();

    model.delete_rows(SHEET, 4, 1).unwrap();
    cache.shift_rows(4, -1);
    assert_cache_agrees(&model, &mut cache, &row_samples(4), &col_samples(3))
        .expect("cache must agree with IronCalc after delete row");
}

#[test]
fn cache_agrees_after_insert_and_delete_col() {
    let mut model = fresh();
    let mut cache = hydrate_cache(&model, ROWS, COLS).unwrap();

    model.insert_columns(SHEET, 3, 1).unwrap();
    cache.shift_cols(3, 1);
    // COLS grew by one in the engine's active band; sample the shifted band col (6).
    assert_cache_agrees(&model, &mut cache, &row_samples(4), &[1, 2, 3, 6])
        .expect("cache agrees after insert col");

    model.delete_columns(SHEET, 3, 1).unwrap();
    cache.shift_cols(3, -1);
    assert_cache_agrees(&model, &mut cache, &row_samples(4), &col_samples(3))
        .expect("cache agrees after delete col");
}

#[test]
fn cache_agrees_after_undo_and_redo() {
    let mut model = fresh();
    let mut cache = hydrate_cache(&model, ROWS, COLS).unwrap();

    // Insert on both, then undo on both (mirror-the-primitive: undo-insert = delete).
    model.insert_rows(SHEET, 4, 1).unwrap();
    let (rm, rmc) = cache.shift_rows(4, 1);
    assert_cache_agrees(&model, &mut cache, &row_samples(4), &col_samples(3)).unwrap();

    model.undo().unwrap();
    // Undo of an insert is a delete at the same index; the mirror restores nothing removed
    // (insert removed nothing), so a plain delete-shift is the inverse.
    cache.shift_rows(4, -1);
    let _ = (rm, rmc);
    assert_cache_agrees(&model, &mut cache, &row_samples(4), &col_samples(3))
        .expect("cache agrees after undo");

    model.redo().unwrap();
    cache.shift_rows(4, 1);
    assert_cache_agrees(&model, &mut cache, &row_samples(4), &col_samples(3))
        .expect("cache agrees after redo");
}

#[test]
fn cache_agrees_after_delete_undo_restores_removed_overrides() {
    // Delete a *banded* row so the cache must restore the removed band override on undo.
    let mut model = fresh();
    let mut cache = hydrate_cache(&model, ROWS, COLS).unwrap();
    // Row 10 is banded + tall. Delete row 10 on both, then undo.
    assert!(cache.rows.band_style(10) != StyleId(0), "row 10 has a band override");

    model.delete_rows(SHEET, 10, 1).unwrap();
    let (removed, removed_cells) = cache.shift_rows(10, -1);
    assert_cache_agrees(&model, &mut cache, &row_samples(10), &col_samples(3))
        .expect("agrees after delete of banded row");

    model.undo().unwrap();
    // Undo of a delete = insert + restore the removed overrides (mirror-the-primitive).
    cache.shift_rows(10, 1);
    cache.restore_row_shift(&removed, &removed_cells);
    assert_cache_agrees(&model, &mut cache, &row_samples(10), &col_samples(3))
        .expect("agrees after undo of delete (restored band override)");
}

#[test]
fn negative_control_wrong_shift_is_detected() {
    // The agreement contract must have discriminating power: if we shift IronCalc but do
    // NOT shift the cache (or shift it wrong), assert_cache_agrees must FAIL. A rubber
    // stamp would be worthless.
    let mut model = fresh();
    let mut cache = hydrate_cache(&model, ROWS, COLS).unwrap();
    model.insert_rows(SHEET, 4, 1).unwrap();
    // Deliberately forget to shift the cache.
    let result = assert_cache_agrees(&model, &mut cache, &row_samples(4), &col_samples(3));
    assert!(
        result.is_err(),
        "agreement check must detect a cache that was NOT shifted with the engine"
    );
    // And a wrong-direction shift is also caught.
    cache.shift_rows(4, -1);
    let result2 = assert_cache_agrees(&model, &mut cache, &row_samples(4), &col_samples(3));
    assert!(result2.is_err(), "agreement check must detect a wrong-direction shift");
}

#[test]
fn cumulative_offset_matches_sizes() {
    // A small hand-built axis: sizes [_, 10, 20, 30, 40] (1-based).
    let mut axis = Axis::new(4, 10.0);
    axis.set_size(2, 20.0);
    axis.set_size(3, 30.0);
    axis.set_size(4, 40.0);
    assert_eq!(axis.offset(1), 0.0);
    assert_eq!(axis.offset(2), 10.0);
    assert_eq!(axis.offset(3), 30.0);
    assert_eq!(axis.offset(4), 60.0);
    // index_at: pixel 0 -> line 1; pixel 15 -> line 2 (10..30); pixel 65 -> line 4.
    assert_eq!(axis.index_at(0.0), 1);
    assert_eq!(axis.index_at(15.0), 2);
    assert_eq!(axis.index_at(65.0), 4);

    // After inserting a default(10) row at index 2: sizes [_, 10, 10, 20, 30, 40].
    axis.shift(2, 1);
    assert_eq!(axis.offset(3), 20.0, "offset consistent after insert");
    assert_eq!(axis.size(3), 20.0, "override shifted from index 2 to 3");
}

#[test]
fn copy_paste_translates_relative_refs() {
    // The public copy->paste clipboard cannot be chained externally (Clipboard.data is
    // pub(crate)), but the relative-reference translator IS public on Model. Probe it:
    // "=A1+B1" copied from C1 to C2 should become "=A2+B2".
    let mut model = UserModel::new_empty("cp", "en", "UTC", "en").unwrap();
    model.set_user_input(SHEET, 1, 1, "1").unwrap(); // A1
    model.set_user_input(SHEET, 1, 2, "2").unwrap(); // B1
    model.set_user_input(SHEET, 1, 3, "=A1+B1").unwrap(); // C1

    // Reach the translator via get_model()... but it needs &mut Model, which UserModel
    // doesn't hand out. Instead reconstruct a Model to exercise the pure translation.
    let mut m = ironcalc_base::Model::new_empty("cp2", "en", "UTC", "en").unwrap();
    m.set_user_input(SHEET, 1, 1, "1".into()).unwrap();
    m.set_user_input(SHEET, 1, 2, "2".into()).unwrap();
    let source = CellReferenceIndex { sheet: SHEET, row: 1, column: 3 };
    let target = CellReferenceIndex { sheet: SHEET, row: 2, column: 3 };
    let translated = m.extend_copied_value("=A1+B1", &source, &target).unwrap();
    assert_eq!(translated, "=A2+B2", "relative refs translate on paste");
}

#[test]
fn cache_interns_styles() {
    let mut cache = ResidentCache::new(10, 10, 21.0, 100.0);
    let mut s = Style::default();
    s.font.b = true;
    let id1 = harness::intern(&mut cache, &s);
    let id2 = harness::intern(&mut cache, &s.clone());
    assert_eq!(id1, id2, "equal styles share one id");
    assert_ne!(id1, StyleId(0), "a non-default style is not id 0");
    let mut s2 = Style::default();
    s2.font.i = true;
    let id3 = harness::intern(&mut cache, &s2);
    assert_ne!(id1, id3, "different styles get different ids");
    // default + bold + italic = 3 distinct.
    assert_eq!(cache.interner.len(), 3);
}
