---
status: complete
---

# Phase 2: Core foundations (Linux)

## Overview

Phase 2 builds the **headless, GPU-free, IronCalc-free foundation** the whole app grows
from. Everything here lives in `freecell-core` (std-only; the dependency-rule guard test
from Phase 1 enforces no `gpui*`/`ironcalc*` runtime deps). These are the pure logic
types the three parallel tracks (engine, grid, shell) all consume, so building them first
unblocks the fan-out (`implementation_plan.md` dependency graph).

Scope (from `architecture.md §3` + the Linux-testable halves of the grid / style_cache /
app_shell component test plans):

- **Axis** — two-level prefix-sum virtualization ported from
  `experiments/04-ui-poc/poc-core/src/layout.rs`, made `Send + Sync` so it can live in the
  worker-written, UI-read resident cache.
- **A1 / CellRef / CellRange** — zero-based cell addressing + Excel A1 conversion (both
  directions), range normalization, `to_a1()` for the ref box.
- **Rgb / Align / RenderStyle** — the engine-free resolved style the grid draws.
- **Publication / PublishedCell** — the viewport value snapshot read model.
- **SheetCaches / SheetCache / StyleId** — the geometry + resolved-style **read model**
  (the `StyleInterner` and build logic are engine-side, Phase 5), with the
  cell>row>col>default resolution order and a fixture-friendly builder.
- **input-cap validator** — length > 8192 chars OR paren-nesting depth > 64 → reject;
  includes the round-3 D abort reproducers (deep parens, long flat chain) as rejected
  cases.
- **sheet-name validator** — the xlsx rule matrix (non-empty, ≤31, illegal chars, edge
  apostrophe, case-insensitive uniqueness).
- **palette** — the 10 Office-theme fill swatches (`ui_design.md §3.1`).
- **SelectionModel + keyboard motions** — active/anchor model, `apply_motion` pure
  function for every navigation key with edge clamping and range extension.
- **data-row reducer** — the formula-bar state machine as a pure
  `reduce(state, event) -> Vec<Effect>` (`app_shell.md §Data row`).

## Steps

1. **`color.rs`** — `pub struct Rgb { r, g, b: u8 }` with `new`, `from_hex(u32)`,
   `to_hex() -> u32`. Attribution comment (shape mirrors the frozen `datagen::Rgb`, copied
   not referenced per `architecture.md §1`).
2. **`axis.rs`** — port `poc-core/src/layout.rs` verbatim, with two adaptations:
   (a) sizer bound becomes `Fn(u32) -> f32 + Send + Sync + 'static` and the box is
   `Box<dyn Fn(u32) -> f32 + Send + Sync>` so `Arc<Axis>` is `Send + Sync` for the shared
   cache; (b) add `Axis::from_overrides(count, default_px, BTreeMap<u32,f32>)` (captures an
   `Arc` of the overrides — the cache's geometry constructor). Port all 5 POC tests,
   swapping `datagen::EXCEL_MAX_ROWS` → `crate::limits::MAX_ROWS`.
3. **`refs.rs`** — `SheetId(u32)`; `CellRef { row, col: u32 }` (zero-based) with
   `to_a1()`, `from_a1(&str) -> Option<CellRef>`; `column_label(u32) -> String` /
   `column_from_label(&str) -> Option<u32>` (bijective base-26, ported from
   `datagen::column_label`); `CellRange { start, end: CellRef }` **normalized** (min/max
   corners) with `contains`, `rows()`, `cols()`, `to_a1()` (single-cell collapses to one
   ref).
4. **`style.rs`** — `pub enum Align { Left, Center, Right }`; `RenderStyle { bold, italic,
   underline: bool, fill, font_color: Option<Rgb>, h_align: Option<Align>,
   num_format_is_default: bool }` + `Default` (all clear, `num_format_is_default: true`).
5. **`publication.rs`** — `PublishedCell { row, col: u32, display_text: String, text_color:
   Option<Rgb> }`; `Publication { sheet: SheetId, rows, cols: Range<u32>, generation: u64,
   cells: Vec<PublishedCell> }` + `empty(sheet, generation)` + `covers(row, col) -> bool`.
6. **`cache.rs`** — `StyleId(u32)`; geometry defaults `DEFAULT_ROW_HEIGHT_PX = 24.0`,
   `DEFAULT_COL_WIDTH_PX = 100.0` (`ui_design.md §3.3`); `SheetCache` holding the geometry
   (defaults + override maps + `Arc<Axis>` pair) and styling (cell/row/col `StyleId` maps +
   `resolved: Vec<RenderStyle>`); `render_style(row,col) -> Option<&RenderStyle>` resolving
   **cell > row-band > col-band > default(None)**; `axes()`, `row_height`, `col_width`,
   `total_width/height`. `SheetCacheBuilder` (fixture + engine friendly): dedup-interns
   `RenderStyle`s into `resolved`, sets cell/row/col styles + geometry overrides, `build()`
   constructs the axes. `SheetCaches { sheets: HashMap<SheetId, SheetCache> }` with
   `get/insert/remove/get_or_default`.
7. **`input_cap.rs`** — `MAX_INPUT_LEN`/`MAX_NESTING_DEPTH` re-exported from `limits`;
   `InputRejection { TooLong{len,max}, TooDeeplyNested{depth,max} }`;
   `validate_input(&str) -> Result<(), InputRejection>` scoped to **formulas** (leading
   `=`): char-length cap + max paren-nesting depth via a scan that **skips string
   literals** (`"..."`, `""` escape) so quoted parens don't count. Non-formula inputs pass
   (they never reach the recursive parser — round-3 D).
8. **`sheet_name.rs`** — `SheetNameError { Empty, TooLong{len}, IllegalChar(char),
   EdgeApostrophe, Duplicate }`; `validate_sheet_name(name, existing: &[&str]) ->
   Result<(), SheetNameError>` enforcing the `functional_spec §3.7` xlsx rules
   (case-insensitive uniqueness; caller passes the *other* sheets).
9. **`palette.rs`** — `Swatch { name: &'static str, rgb: Rgb }`; `FILL_PALETTE: [Swatch;
   10]` in canonical Office theme order (`ui_design.md §3.1` hexes).
10. **`selection.rs`** — `SheetDims { rows, cols: u32 }`; `Direction { Up,Down,Left,Right }`;
    `Motion { Move, Extend, JumpEdge, ExtendEdge, Page{dir,rows}, ExtendPage{dir,rows},
    RowStart, ExtendRowStart }`; `SelectionModel { anchor, active: CellRef }` with `single`,
    `range() -> CellRange`, `is_single`, `to_a1`; `apply_motion(sel, motion, dims) ->
    SelectionModel` (all clamped to `[0,dims)`; Move/Jump collapse, Extend/ExtendEdge keep
    anchor). Tab/Enter/Shift variants documented as `Move(Right/Down/Left/Up)` mappings
    (bound at the window/keymap layer).
11. **`data_row.rs`** — `DataRow` state (`FieldMode { Idle, Editing, Disabled }`, text,
    committed, `latest_req`, awaiting, spinner, cap_error); `DataRowEvent`
    (`SelectionChanged{single}`, `ContentFetched{req_id,raw}`, `Edited{text}`, `Commit`,
    `EditCommitRequested`, `Escape`, `FetchTimeout{req_id}`); `DataRowEffect` (`Fetch{req_id}`,
    `Commit{input}`, `MoveActive(Motion)`, `FocusGrid`, `ShowCapError`, `SetSpinner(bool)`);
    `reduce(&mut self, DataRowEvent) -> Vec<DataRowEffect>` per the `app_shell.md` state
    machine (stale-reply drop, cap-reject-keeps-editing, escape-reverts,
    commit-on-click-away, multiselect-disables, 250 ms spinner).
12. **`lib.rs`** — declare the modules, re-export the load-bearing types at the crate root,
    keep the `limits` module (Phase 1) and reference `MAX_INPUT_LEN`/`MAX_NESTING_DEPTH`
    from `input_cap`.
13. Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
    `cargo test --workspace` in the FOREGROUND until green.

## Tests

- **axis**: `axis_total_matches_naive_sum`, `offset_and_index_roundtrip`,
  `visible_range_covers_viewport_and_clamps`, `visible_range_clamps_at_edges`,
  `empty_axis_is_well_behaved`, `handles_excel_max_rows_without_oom` (ported);
  `from_overrides_matches_default_and_override`.
- **refs**: `column_label_*` (A/Z/AA/XFD), `column_label_roundtrip`, `a1_roundtrip`,
  `from_a1_rejects_junk`, `cell_range_normalizes_corners`, `cell_range_contains`,
  `range_to_a1_single_vs_rect`.
- **style**: `render_style_default_is_plain`.
- **color**: `rgb_hex_roundtrip`.
- **publication**: `covers_reports_membership`, `empty_publication_has_no_cells`.
- **cache**: `render_style_resolution_order` (cell>row>col>default), `builder_interns_dedups`,
  `geometry_defaults_and_overrides`, `axes_total_matches_geometry`,
  `excel_max_geometry_totals` (1M rows via defaults).
- **input_cap**: `accepts_normal_formula`, `accepts_non_formula_text`,
  `rejects_over_length`, `rejects_over_nesting_depth`,
  `rejects_round3_d_deep_parens_reproducer`, `rejects_round3_d_flat_chain_reproducer`,
  `paren_in_string_literal_not_counted`, `boundary_at_exactly_the_caps`.
- **sheet_name**: `accepts_valid`, `rejects_empty`, `rejects_blank`, `rejects_too_long`,
  `rejects_each_illegal_char`, `rejects_edge_apostrophe`,
  `rejects_case_insensitive_duplicate`, `allows_rename_to_same_name`.
- **palette**: `palette_has_ten_office_swatches`, `palette_hexes_match_spec`.
- **selection**: `move_each_direction_collapses`, `move_clamps_at_edges`,
  `extend_keeps_anchor`, `jump_edge_goes_to_sheet_bound`, `extend_edge_keeps_anchor`,
  `page_moves_by_rows_clamped`, `row_start_goes_to_col_zero`,
  `selection_to_a1_single_and_range`, `single_selection_is_single`.
- **data_row**: `selection_single_fetches`, `multiselect_disables`,
  `stale_content_reply_dropped`, `fresh_content_reply_shown`, `edit_enters_editing`,
  `escape_reverts_to_committed`, `cap_reject_keeps_editing`, `commit_valid_moves_down`,
  `edit_commit_on_cell_click`, `fetch_timeout_shows_spinner`,
  `spinner_hidden_when_reply_arrives`.

## Untestable-on-Linux items (deferred, per component plans)

None in this phase — Phase 2 is entirely pure logic. The GPUI integration tests
(`welcome_to_workbook_lifecycle`, etc.) and render snapshots named in the component plans
belong to Phases 6–10; the pure-logic extractions those plans call out are exactly what
this phase delivers.
