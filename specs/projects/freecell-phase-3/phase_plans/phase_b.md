---
status: complete
---

# Phase B: Needed-API audit (breadth — does IronCalc expose everything the build needs?)

## Overview

Phase B is the breadth investigation (functional_spec §6-B, architecture §5). It
produces a **present / absent / workaround** coverage matrix for every IronCalc 0.7.1
public API the real FreeCell app will need, each entry backed by a **runtime probe** (a
compiling assertion that calls the API) or a **source citation** (`file:line` in the
crate under `~/.cargo/registry/.../ironcalc*-0.7.1/`). The pass criterion is judgment:
**no surprise load-bearing gap buried** — the headline is display formatting.

**Headline (already established during planning, will be probe-backed):** IronCalc
**owns display formatting**. `Model::get_formatted_cell_value` /
`UserModel::get_formatted_cell_value` apply the cell's number format via
`number_format::format_number(value, format, locale) -> Formatted { text, color, error }`
and return the displayed string (`"1,234.50"`, `"100.00%"`, formatted dates). This is
**PRESENT** — FreeCell does NOT need to implement Excel number-format rendering. This is
the load-bearing answer for the renderer and must be surfaced plainly, probe-backed.

Phase B builds ON Phase A (`A-cache-sync/findings.md`), which already established (and
this phase cites, not redoes): merges have no public API; the clipboard isn't externally
chainable; the `UserModel` diff-list is opaque bitcode (`pub(crate) enum Diff`);
`UserModel<'static>` is `Send`. Structural-edit APIs (insert/delete row/col, undo/redo)
are A's domain and are only referenced here.

This is a **throwaway audit** — the deliverable is an honest, source-cited matrix +
`findings.md`, and a **compiling probe binary** that exercises the *present* APIs so
"present" claims are real. Not production code.

## Checklist (functional_spec §6-B)

Each item marked present / absent / workaround, probe- or cite-backed:

1. **Display formatting [HEADLINE]** — engine produces the display string? (who owns
   number-format rendering)
2. **Edit diff-list** (`UserModel`) shape + how FreeCell consumes it (confirm/extend A's
   opaque-bitcode finding).
3. **Sheet ops** — add / rename / delete / reorder / enumerate.
4. **Defined names / named ranges** — read + write.
5. **View/UI state in `.xlsx`** — freeze panes, hidden rows/cols, gridlines, zoom,
   selection — persisted/exposed, or FreeCell owns them?
6. **Cell extras** — comments/notes, data validation, hyperlinks.
7. **Formula-editing helpers** — function list + tokenizer/parser for the formula bar /
   reference highlighting.
8. **Re-confirm known OPEN gaps** (record, don't design) — merges, conditional
   formatting, dynamic arrays (0/17).

## Steps

1. **Scaffold the crate** — `experiments/round-3/B-api-audit/`:
   - `Cargo.toml`: mirror A's — `name = "api_audit"`, deps on `round2_harness`
     (read-only, `../../round-2/harness`), `datagen`, `bench_util`, `ironcalc = "0.7"`,
     `ironcalc_base = "0.7"`, `anyhow`, `serde_json`. `[lib]` + `[[bin]]`.
   - Replace the stub `src/main.rs` with a runner that calls each probe and prints a
     compact present/absent line per checklist item (so `cargo run` demonstrates the
     matrix live).
   - `src/lib.rs` exposing the probe modules.

2. **`src/display_format.rs` (HEADLINE probe)** — call
   `UserModel::get_formatted_cell_value` after setting `num_fmt` via
   `update_range_style(area, "num_fmt", ...)`; assert engine returns the displayed
   string for: a thousands+decimals number (`"1,234.50"`), a percent (`"100.00%"`), and
   a date serial under a date format. Also call the low-level
   `ironcalc_base::number_format::format_number(value, format, locale)` and read the
   `Formatted { text, color, error }` (proves the display+color path is reachable
   directly for a display cache). Records: **engine owns display formatting → PRESENT.**

3. **`src/diff_list.rs`** — confirm A's finding with a fresh probe:
   `flush_send_queue()` returns non-empty `Vec<u8>` (bitcode), `apply_external_diffs`
   round-trips it into a replica; assert the replica reaches the same formatted value.
   Document: opaque bytes, replica-sync channel only, NOT field-by-field inspectable →
   FreeCell drives surgical UI updates by mirroring the op it issued (per A), and can
   use flush/apply for the collaborative/replica path.

4. **`src/sheet_ops.rs`** — probe `new_sheet`, `rename_sheet`, `delete_sheet`,
   `get_worksheets_properties` (enumerate: name/index/id/color/state) on `UserModel`;
   assert the sheet list changes accordingly, with undo/redo of a sheet add.
   **Reorder:** documented ABSENT (no `move_sheet`/reorder in source) — workaround note.

5. **`src/defined_names.rs`** — probe `new_defined_name` (workbook- and sheet-scoped),
   `get_defined_name_list`, `update_defined_name`, `delete_defined_name` on `UserModel`;
   assert a formula using the name evaluates, and that read-back lists it. Read+write
   PRESENT.

6. **`src/view_state.rs`** — probe frozen rows/cols (`set_frozen_rows/columns` +
   getters), gridlines (`set_show_grid_lines`/`get_show_grid_lines`), hidden rows/cols
   (row/col `hidden`), selection (`ui.rs`), zoom/window (`ui.rs`/window size). For each,
   record present/absent and whether xlsx import/export persists it (source-cited from
   `ironcalc/src/import|export/worksheets.rs`). Flag anything FreeCell must own.

7. **`src/cell_extras.rs`** — probe/cite comments/notes, data validation, hyperlinks:
   is there a public getter/setter or only a struct field or nothing? Cite source; mark
   present/absent/workaround; note xlsx persistence.

8. **`src/formula_helpers.rs`** — the formula-bar helpers:
   - `get_cell_content` returns the `=formula` string (formula-bar population) — probe.
   - Tokenizer: is `expressions::lexer::Lexer` public and callable externally? Construct
     it, tokenize a formula, and read `TokenType` variants (for syntax/reference
     highlighting) — probe if public; else cite `pub(crate)` and give the workaround.
   - Parser/AST reference extraction: is `Node` public + walkable? probe or cite.
   - Function list: enumerate the public `Function` enum / count variants (corroborate
     SP3's ~345). Probe or cite.

9. **`src/known_gaps.rs`** — re-confirm (record only, don't design): merges (no public
   API — cite A + `types.rs` `merge_cells`), conditional formatting (no API — search),
   dynamic arrays (0/17 — cite SP3). A short documented search each.

10. **`findings.md`** (Phase-1 §5.2 headings) — the full matrix, a plan per gap, a
    **prominent flag** on the headline (display formatting = engine-owned = PRESENT, so
    NOT a renderer scope item), and honest grading vs pass criteria. Cite `file:line`
    for every present/absent claim.

## Tests

`tests/audit.rs` — GATE assertions that make "present" claims real (each calls the API):

- `display_format_engine_owns_number_format` — `get_formatted_cell_value` yields
  `"1,234.50"` for `#,##0.00`, `"100.00%"` for `0.00%`, and a formatted date string;
  asserts FreeCell does not compute these.
- `format_number_low_level_reachable` — `number_format::format_number` returns the
  expected `Formatted.text` (and exercises `.color` on a `[Red]` negative format).
- `diff_list_opaque_but_replica_syncs` — `flush_send_queue` non-empty; `apply_external_diffs`
  makes a replica match; document opacity.
- `sheet_ops_add_rename_delete_enumerate` — round-trips a sheet lifecycle; enumerate
  reflects it; undo of add works. Reorder asserted absent (compile-time: no such method
  — documented, not a failing test).
- `defined_names_read_write` — create scoped + unscoped names; a formula referencing one
  evaluates; list + delete round-trip.
- `view_state_frozen_gridlines_present` — frozen rows/cols + gridlines set/get round-trip;
  record which persist to xlsx (cited).
- `cell_extras_status` — asserts the observed present/absent status of comments / data
  validation / hyperlinks (whatever the source shows — the test encodes the finding).
- `formula_helpers_tokenizer_and_functions` — `get_cell_content` returns the formula;
  the tokenizer path (if public) tokenizes a formula; function-list count is recorded.
- Negative/honesty control where meaningful (e.g. absent APIs documented, not silently
  passed).

## Notes / discipline

- Edit ONLY `experiments/round-3/B-api-audit/` (+ this phase plan). Read-only elsewhere;
  never touch frozen `round-2/harness`, `shared/*`, or another experiment.
- Any `cargo` build/test runs FOREGROUND with `timeout 900` (IronCalc is heavy); never
  `nohup`/`&`.
- "Present" = probe or source cite; "absent" = documented search. Grade honestly.
- Do NOT commit (manager commits).
