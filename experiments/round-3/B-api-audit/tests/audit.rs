//! GATE tests for Investigation B — each makes a "present" claim REAL by calling the
//! API, and encodes the observed status of the gaps. If a present-API claim regresses in
//! a future IronCalc, the matching test fails.

use api_audit::{
    cell_extras, defined_names, diff_list, display_format, formula_helpers, known_gaps, sheet_ops,
    view_state, Status,
};

// ---------------------------------------------------------------------------
// HEADLINE: display formatting is engine-owned.
// ---------------------------------------------------------------------------

#[test]
fn display_format_engine_owns_number_format() {
    let o = display_format::probe();
    // The renderer's exact per-cell call yields the DISPLAY string, not the raw value.
    assert_eq!(
        o.number, "1,234.50",
        "thousands+decimals: #,##0.00 on 1234.5"
    );
    assert_eq!(o.percent, "100.00%", "percent: 0.00% on raw 1.0");
    assert_eq!(o.date, "2021-01-01", "date: yyyy-mm-dd on serial 44197");
}

#[test]
fn display_format_exposes_color() {
    // A [Red] negative format returns a color too — the renderer gets it from the engine.
    let o = display_format::probe();
    assert_eq!(o.red_negative_text, "1,234.50");
    assert!(
        o.red_negative_has_color,
        "format_number should surface the [Red] color"
    );
}

#[test]
fn display_format_audit_is_all_present() {
    for row in display_format::audit() {
        assert_eq!(
            row.status,
            Status::Present,
            "display-format capability should be PRESENT: {}",
            row.capability
        );
    }
}

// ---------------------------------------------------------------------------
// Diff-list: opaque bytes, replica-sync only.
// ---------------------------------------------------------------------------

#[test]
fn diff_list_opaque_but_replica_syncs() {
    let o = diff_list::probe();
    assert!(
        o.diff_byte_len > 0,
        "flush_send_queue should carry the edits"
    );
    assert!(
        o.replica_matches_origin,
        "apply_external_diffs should replay into a matching replica"
    );
}

// ---------------------------------------------------------------------------
// Sheet ops: add/rename/delete/enumerate present + undoable; reorder absent.
// ---------------------------------------------------------------------------

#[test]
fn sheet_ops_add_rename_delete_enumerate() {
    let o = sheet_ops::probe();
    assert_eq!(o.after_add, o.initial + 1, "new_sheet adds one");
    assert_eq!(o.renamed, "Renamed", "rename_sheet takes effect");
    assert_eq!(
        o.name_after_undo, "Sheet2",
        "undo of rename reverts to the sheet's prior (auto-assigned) name"
    );
    assert_eq!(o.after_undo_add, o.initial, "undo of add removes the sheet");
    assert_eq!(
        o.after_delete,
        o.before_delete - 1,
        "delete_sheet removes one"
    );
}

#[test]
fn sheet_reorder_is_absent() {
    // Encodes the documented gap: no reorder API. (There is no method to call — this
    // asserts the audit records it as ABSENT.)
    let reorder = sheet_ops::audit()
        .into_iter()
        .find(|r| r.capability.contains("reorder"))
        .expect("reorder row present in matrix");
    assert_eq!(reorder.status, Status::Absent);
}

// ---------------------------------------------------------------------------
// Defined names: read + write.
// ---------------------------------------------------------------------------

#[test]
fn defined_names_read_write() {
    let o = defined_names::probe();
    assert!(o.has_name, "created name appears in the list");
    assert_eq!(o.via_name, "84", "=MyVal*2 with A1=42 evaluates to 84");
    assert!(o.gone_after_delete, "delete removes the name");
}

// ---------------------------------------------------------------------------
// View state: freeze panes / gridlines / selection present.
// ---------------------------------------------------------------------------

#[test]
fn view_state_frozen_gridlines_selection_present() {
    let o = view_state::probe();
    assert_eq!(o.frozen_rows, 2);
    assert_eq!(o.frozen_cols, 1);
    assert!(o.grid_off, "gridlines can be turned off + read back");
    assert_eq!((o.sel_row, o.sel_col), (5, 3), "selection round-trips");
}

#[test]
fn view_state_hidden_columns_and_zoom_absent() {
    let audit = view_state::audit();
    let hidden_col = audit
        .iter()
        .find(|r| r.capability.contains("hide a column"))
        .unwrap();
    let zoom = audit
        .iter()
        .find(|r| r.capability.contains("zoom"))
        .unwrap();
    assert_eq!(hidden_col.status, Status::Absent);
    assert_eq!(zoom.status, Status::Absent);
}

// ---------------------------------------------------------------------------
// Cell extras: comments read-only/lossy; validation + hyperlinks absent.
// ---------------------------------------------------------------------------

#[test]
fn cell_extras_status_recorded() {
    let o = cell_extras::probe();
    assert_eq!(
        o.comment_count, 0,
        "fresh sheet has no comments (field reachable)"
    );
    let audit = cell_extras::audit();
    let comments = audit
        .iter()
        .find(|r| r.capability.contains("comments"))
        .unwrap();
    let validation = audit
        .iter()
        .find(|r| r.capability.contains("data validation"))
        .unwrap();
    let hyperlinks = audit
        .iter()
        .find(|r| r.capability.contains("hyperlinks"))
        .unwrap();
    assert_eq!(
        comments.status,
        Status::Workaround,
        "comments: read-only, lossy on save"
    );
    assert_eq!(validation.status, Status::Absent);
    assert_eq!(hyperlinks.status, Status::Absent);
}

// ---------------------------------------------------------------------------
// Formula helpers: formula string + tokenizer + parser present; function list workaround.
// ---------------------------------------------------------------------------

#[test]
fn formula_helpers_tokenizer_and_parser_extract_references() {
    let o = formula_helpers::probe();
    assert_eq!(
        o.content, "=A2+B3*2",
        "get_cell_content returns the formula"
    );
    assert_eq!(o.token_refs, 2, "lexer sees A2 and B3 as references");
    assert_eq!(o.parsed_refs, 2, "parser AST has 2 reference leaves");
    assert!(
        o.token_total > o.token_refs,
        "lexer also emits operators/literals"
    );
}

#[test]
fn function_list_is_not_externally_enumerable() {
    let o = formula_helpers::probe();
    assert_eq!(
        o.function_enum_variants, 345,
        "matches SP3's function count"
    );
    assert!(
        !o.function_enum_public,
        "the Function enum is in a private module — FreeCell owns its own list"
    );
    let row = formula_helpers::audit()
        .into_iter()
        .find(|r| r.capability.contains("function list"))
        .unwrap();
    assert_eq!(row.status, Status::Workaround);
}

// ---------------------------------------------------------------------------
// Known gaps re-confirmed.
// ---------------------------------------------------------------------------

#[test]
fn known_gaps_all_absent() {
    for row in known_gaps::audit() {
        assert_eq!(
            row.status,
            Status::Absent,
            "known gap should be ABSENT: {}",
            row.capability
        );
    }
}

// ---------------------------------------------------------------------------
// The whole matrix assembles without panicking (every present-claim probe ran).
// ---------------------------------------------------------------------------

#[test]
fn full_audit_runs_and_covers_all_areas() {
    let rows = api_audit::run_full_audit();
    assert!(rows.len() >= 20, "matrix should cover every checklist item");
    // At least one PRESENT (headline) and at least one ABSENT (a real gap) — a sanity
    // check that the audit discriminates, not a rubber stamp.
    assert!(rows.iter().any(|r| r.status == Status::Present));
    assert!(rows.iter().any(|r| r.status == Status::Absent));
}
