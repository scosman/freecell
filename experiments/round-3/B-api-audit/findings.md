# Investigation B — Needed-API audit (breadth)

> Phase-3 breadth investigation (functional_spec §6-B, architecture §5). The deliverable
> is a **present / absent / workaround** coverage matrix for every IronCalc 0.7.1 public
> API the real FreeCell app needs — each entry backed by a **runtime probe** (a compiling
> assertion that calls the API) or a **source citation** (`file:line` under
> `~/.cargo/registry/.../ironcalc*-0.7.1/`). Pass criterion is judgment: **no surprise
> load-bearing gap buried.**
>
> **HEADLINE (probe-backed): IronCalc OWNS display formatting.** FreeCell does NOT
> implement Excel number-format rendering — the engine produces the displayed string. See
> §Findings/1. Builds on Phase A (`../A-cache-sync/findings.md`): merges/clipboard/diff-list
> facts are cited, not redone.

## Questions (from the spec)

1. **Display formatting [HEADLINE]** — does IronCalc produce the *displayed string* for a
   cell (value + number-format → `"1,234.50"`, `"50%"`, a formatted date), or must
   FreeCell implement number-format rendering itself?
2. **Edit diff-list** (`UserModel`) — shape + how FreeCell consumes it (confirm A's
   "opaque bitcode" finding).
3. **Sheet ops** — add / rename / delete / reorder / enumerate.
4. **Defined names / named ranges** — read + write.
5. **View/UI state in `.xlsx`** — freeze panes, hidden rows/cols, gridlines, zoom,
   selection — persisted/exposed, or FreeCell-owned?
6. **Cell extras** — comments/notes, data validation, hyperlinks.
7. **Formula-editing helpers** — function list + tokenizer/parser for the formula bar /
   reference highlighting.
8. Re-confirm the **known OPEN gaps** — merges, conditional formatting, dynamic arrays
   (0/17).

## What was done

An independent Cargo project `api_audit` (`experiments/round-3/B-api-audit/`), depending
**read-only** on the frozen `../../round-2/harness` (only for `cpu_model` env stamping)
and directly on `ironcalc` / `ironcalc_base` 0.7.1 (the round-2 pin, `Cargo.lock` →
0.7.1). One probe module per checklist item, each exposing a `probe()` that calls the API
and an `audit()` that returns the matrix rows:

- **`src/display_format.rs`** — the HEADLINE probe: sets `num_fmt` via
  `update_range_style(area, "num_fmt", ..)` and calls `get_formatted_cell_value`, plus the
  low-level `number_format::format_number(..) -> Formatted { text, color, error }`.
- **`src/diff_list.rs`** — flushes the opaque send-queue and round-trips it into a replica.
- **`src/sheet_ops.rs`** — add/rename/delete/enumerate + undo; reorder documented absent.
- **`src/defined_names.rs`** — create (scoped/unscoped), list, use-in-formula, delete.
- **`src/view_state.rs`** — frozen rows/cols, gridlines, selection round-trip; hidden
  rows/cols + zoom documented.
- **`src/cell_extras.rs`** — comments (read-only/lossy), data validation, hyperlinks.
- **`src/formula_helpers.rs`** — `get_cell_content` + the public `Lexer` tokenizer + the
  public `Parser`/`Node` AST (reference extraction) + the function-list reachability.
- **`src/known_gaps.rs`** — merges / conditional formatting / dynamic arrays.
- **`tests/audit.rs`** — 14 GATE tests that make every "present" claim real by calling the
  API (a regression in a future IronCalc fails the matching test); the "absent" rows are
  encoded from documented source searches.
- **`src/main.rs`** — prints the full env-stamped matrix (`cargo run`), doubling as a
  liveness check of the present-claims.

`cargo test` = 14/14 pass. `cargo clippy --all-targets` + `cargo fmt --check` clean.

## Findings — the coverage matrix

Status legend: **PRESENT** (public API + probe), **WORKAROUND** (partial / caveated),
**ABSENT** (documented search found no public API). Totals: **14 present, 4 workaround,
9 absent (27 rows).** Every citation is `file:line` in `ironcalc_base-0.7.1/src` unless
noted `ironcalc-0.7.1/src`.

### 1. Display formatting [HEADLINE] — PRESENT (engine-owned). FreeCell does NOT own number-format rendering.

This is the load-bearing answer for the renderer, and it is **not a surprise gap** — the
engine owns it.

- **`Model::get_formatted_cell_value` (model.rs:1800) / `UserModel::get_formatted_cell_value`
  (common.rs:475)** read the cell's number format (`get_style_for_cell(..).num_fmt`,
  model.rs:1808) and run `number_format::format_number(value, &format, locale).text`
  (model.rs:1810-1812) to return the **display string**. The upstream doc example asserts
  `=1/3` → `"0.333333333"` (model.rs:1795).
- **Probe results (the renderer's exact per-cell call):**
  `1234.5` under `#,##0.00` → **`"1,234.50"`**; raw `1.0` under `0.00%` → **`"100.00%"`**;
  serial `44197` under `yyyy-mm-dd` → **`"2021-01-01"`** (not the raw serial). Upstream
  format tests corroborate: `formatter/test/test_en_examples.rs:56` (`"1,234.50"`), `:93`
  (`"100.00%"`), `formatter/test/test_dates.rs` (dates).
- **Color too.** `pub fn format_number(value, format_code, locale) -> Formatted` is
  publicly reachable (number_format.rs:152; `pub mod number_format`, lib.rs:37), returning
  `Formatted { color: Option<i32>, text: String, error: Option<String> }`
  (formatter/format.rs:10). A `[Red]` negative format yields a color the renderer can use
  directly (probe: `red_negative_has_color = true`). So a FreeCell **display-cache** can
  call `format_number(value, format_string, locale)` directly with the value + the cell's
  format string.
- **Setting a format is public + undoable:** `UserModel::update_range_style(area,
  "num_fmt", fmt)` (common.rs:1253; the `"num_fmt"` style path is common.rs:150); read
  back via `get_cell_style(..).num_fmt` (common.rs:1402).

**Renderer scope conclusion:** FreeCell asks IronCalc for `get_formatted_cell_value` per
visible cell (or calls `format_number` from a display-cache) and renders that text +
optional color. **No Excel number-format engine on the FreeCell side.** (This is the
prime suspect in functional_spec §7 / architecture §8 — surfaced and cleared.)

### 2. Edit diff-list (`UserModel`) — opaque bitcode (ABSENT for structured inspection; PRESENT as replica-sync). Confirms + extends Phase A.

- `UserModel::flush_send_queue() -> Vec<u8>` (common.rs:376) returns
  `bitcode::encode(&self.send_queue)` — **opaque bytes**. The `Diff` enum is `pub(crate)`
  (history.rs:20), so an external crate **cannot match diff variants** field-by-field.
- `UserModel::apply_external_diffs(&[u8])` (common.rs:389) decodes those bytes
  (`bitcode::decode::<Vec<QueueDiffs>>`) and replays redo/undo lists into a **replica** —
  the collaborative / multi-replica sync channel. Probe: flush → 24 opaque bytes → apply
  into a fresh replica → replica matches origin.
- **Consequence (matches A's locked cache-sync design):** the diff-list is a
  **replica-sync transport, NOT a surgical-UI-update channel.** FreeCell drives surgical
  updates by **mirroring the op it issued** (it originates every edit, so it knows
  `kind/at/count`/the edited cell). Edit-sites only, no downstream-dirty (per SP1).

### 3. Sheet ops — add / rename / delete / enumerate PRESENT (undoable); **reorder ABSENT**.

- add `new_sheet` (common.rs:496), rename `rename_sheet(sheet, new_name)` (common.rs:531),
  delete `delete_sheet(sheet)` (common.rs:507) — all on the `UserModel` undo stack (probe:
  add 1→2, rename→"Renamed", undo-rename→"Sheet2", undo-add→1, delete 3→2).
- enumerate `get_worksheets_properties() -> Vec<SheetProperties{name,state,sheet_id,color}>`
  (common.rs:1682; types.rs:653); also `Workbook::get_worksheet_names()` (workbook.rs:6).
- hide/unhide `hide_sheet`/`unhide_sheet` (common.rs:550/576), tab color `set_sheet_color`
  (common.rs:594).
- **reorder / move a sheet — ABSENT.** No `move_sheet`/`reorder`/`set_worksheet_index`/
  `swap_worksheets` anywhere in `ironcalc_base/src` (documented search).
  **Plan:** FreeCell reorders the workbook's worksheet vector itself (it owns `.xlsx`
  writing on export) or upstreams a reorder API. Low-risk (view-order only).

### 4. Defined names / named ranges — PRESENT (read + write, workbook & sheet scope).

`UserModel`: `new_defined_name(name, scope, formula)` (common.rs:1996; `scope = None`
workbook / `Some(idx)` sheet), `get_defined_name_list() -> Vec<(name, scope, formula)>`
(common.rs:1977), `update_defined_name(..)` (common.rs:2014), `delete_defined_name(name,
scope)` (common.rs:1982); all undoable + auto-evaluating. Probe: created `MyVal`→`$A$1`
(=42), `=MyVal*2` → **`"84"`**, listed, deleted.

### 5. View/UI state — MIXED. Freeze panes / gridlines / selection PRESENT + round-trip; hidden rows WORKAROUND; hidden columns / zoom ABSENT.

- **Freeze panes — PRESENT + `.xlsx` round-trip.** `set_frozen_rows_count`/
  `set_frozen_columns_count` + getters (common.rs:1143/1157/1126/1135), undoable; xlsx
  `<pane>` import + export (`ironcalc-0.7.1/src/import|export/worksheets.rs`);
  `Worksheet.frozen_rows/columns` (types.rs:115/116). Probe: rows=2, cols=1.
- **Gridlines — PRESENT + xlsx export.** `set_show_grid_lines`/`get_show_grid_lines`
  (common.rs:1687/1700); exported as `showGridLines`.
- **Selection / active cell — PRESENT + round-trip.** `set_selected_cell`/
  `set_selected_range`/`get_selected_cell` (ui.rs:92/118/35). Probe: (5,3) round-trips.
  (Also window size + scroll `top_row`/`left_column` exist as UI state but are **not
  persisted to `.xlsx`** — view-only.)
- **Hide a ROW — WORKAROUND.** `Row.hidden` is a public field (types.rs:135), read on xlsx
  import + written on export, but there is **no public `UserModel` setter** to hide a row
  (only `set_rows_height`, common.rs:1081). **Plan:** FreeCell sets visibility via the
  `Model`/field path or upstreams a setter; it *does* xlsx round-trip.
- **Hide a COLUMN — ABSENT.** `Col` (types.rs:140) has **no `hidden` field at all** —
  columns cannot be hidden through IronCalc, and it won't xlsx round-trip via IronCalc.
  **Plan:** FreeCell owns column visibility in its own view model.
- **Zoom — ABSENT.** No zoom field/method/xlsx handling. **Plan:** FreeCell owns zoom
  (view-only, not persisted through IronCalc).

### 6. Cell extras — comments WORKAROUND (read-only + lossy on save); data validation + hyperlinks ABSENT.

- **Comments / notes — WORKAROUND.** `Worksheet.comments: Vec<Comment>` is a public field
  (types.rs:114; `Comment { text, author_name, author_id, cell_ref }`, types.rs:229),
  **loaded on xlsx import** (`ironcalc-0.7.1/src/import/worksheets.rs`) but **NOT written
  on export** (a search for "comment" in `ironcalc-0.7.1/src/export/worksheets.rs` finds
  nothing) — **comments are dropped on save.** No public getter/setter to create/edit a
  comment. **Plan:** if FreeCell needs comments, it owns comment storage + `.xlsx` writing.
- **Data validation — ABSENT.** No validation field/type/method in `ironcalc_base/src`;
  xlsx import ignores `<dataValidation>`. **Plan:** FreeCell-owned (needs xlsx writing).
- **Hyperlinks — ABSENT.** No hyperlink field on `Cell`/`Worksheet`; xlsx import/export do
  not handle them. **Plan:** FreeCell-owned (needs xlsx writing).

### 7. Formula-editing helpers — formula string + tokenizer + parser PRESENT; function list WORKAROUND.

- **Formula-bar string — PRESENT.** `UserModel::get_cell_content` (common.rs:466) returns
  the `=formula` string (probe: `=A2+B3*2`).
- **Tokenizer — PRESENT.** `expressions::lexer::Lexer` is a public struct in a `pub mod`
  (expressions/mod.rs:2): `Lexer::new(formula, LexerMode::A1, &Locale, &Language)`
  (lexer/mod.rs:94) + `next_token() -> TokenType` (lexer/mod.rs:185) + `peek_token`/
  `get_position` (for cursor-aware highlighting). `TokenType` (token.rs:219) is public with
  `Reference{row,column,absolute_*}`, `Range{..}`, `Ident` (function/name), `String`,
  `Number`, operators, `EOF` — enough to color tokens and highlight references.
  Locale/Language via public `ironcalc_base::locale::get_locale` / `language::get_language`
  (locale/mod.rs:105, language/mod.rs:399). Probe: `A2+B3*2` → 5 tokens, 2 references.
- **Parser / reference extraction — PRESENT.** `expressions::parser::Parser` (parser/mod.rs:229)
  + `new_parser_english(worksheets, defined_names, tables)` (parser/mod.rs:219);
  `parse(formula, &CellReferenceRC) -> Node` (parser/mod.rs:276). `Node` (parser/mod.rs:110)
  is public with `ReferenceKind{row,column,absolute_*}` + `RangeKind{..}`, walkable for
  precedent highlighting. Probe: `A2+B3*2` → 2 reference leaves.
- **Function list — WORKAROUND.** The `Function` enum (functions/mod.rs:32) has **exactly
  345 variants** (matches SP3), BUT `mod functions;` is **PRIVATE** (lib.rs:45, not
  `pub mod`) → the enum is **not reachable from an external crate**, and there is **no
  iterator/`strum`** (IronCalc's own test parses the source file to enumerate,
  functions/mod.rs:2022). **Plan:** FreeCell maintains its **own function-name list** for
  autocomplete (a small static table; SP3 already lists the 345). It can validate a typed
  name by parsing — an unknown function parses to `Node::InvalidFunctionKind`
  (parser/mod.rs:178). Note: `Node::FunctionKind { kind: Function, .. }` embeds the
  private `Function` type, so external code can match `FunctionKind` but cannot name
  `Function` — reference extraction still works (it doesn't need the function identity).

### 8. Known OPEN gaps (re-confirmed, NOT designed) — merges / conditional formatting / dynamic arrays ABSENT.

- **Merges — ABSENT.** `Worksheet.merge_cells: Vec<String>` exists (types.rs:113) but there
  is **no public `Model`/`UserModel` setter or getter** (search of `common.rs` for
  `merge`). Confirms Phase A §2(d) + overview §2.
- **Conditional formatting — ABSENT.** No conditional-formatting type/field/method in
  `ironcalc_base/src` (the only "conditional" hits are number-format `[cond]` parsing in
  the formatter, unrelated). Confirms overview §2.
- **Dynamic arrays / spilling — ABSENT (0/17, SP3).** A pending **product** decision
  (accept v1 / build spill / upstream), not a technical unknown.

Merges + conditional formatting + (if pursued) comments/validation/hyperlinks share one
consequence: they would force FreeCell to **own `.xlsx` writing** for those features
(~10× scope per overview §2) — recorded, not designed here.

## Load-bearing-gap assessment (the GATE judgment)

| Gap | Load-bearing for MVP renderer/editor? | Verdict |
|---|---|---|
| **Display formatting** | Would be the biggest — it is **PRESENT (engine-owned)** | **cleared, no scope** |
| Function-list enumeration | Autocomplete nicety; FreeCell keeps its own 345-list | not load-bearing |
| Sheet reorder | View-order; FreeCell reorders on export | not load-bearing |
| Hidden columns / zoom | View-only; FreeCell owns them | not load-bearing |
| Comments (lossy) / validation / hyperlinks | Fidelity features; force owning xlsx writing | pre-known product-scope |
| Merges / conditional formatting / dynamic arrays | Pre-known OPEN (overview §2); force owning xlsx writing | pre-known, unchanged |
| Diff-list opacity | Surgical updates via mirror-the-op (A's design) | not load-bearing (A settled) |

**No surprise load-bearing gap.** The single highest-stakes item — display formatting —
is engine-owned and probe-confirmed. Everything else is either a small FreeCell-side
view-model responsibility or a **pre-known** product-scope item (merges/CF/arrays and the
xlsx-writing-forcing extras), all already on the record.

## Grade against pass criteria

| Criterion | Type | Result |
|---|---|---|
| The coverage matrix, reproducible | DELIVERABLE | **PASS** — 27 rows, probe- or source-cited; `cargo run` prints it, `cargo test` (14) verifies present-claims |
| No *surprise* load-bearing gap (esp. display formatting) | GATE (judgment) | **PASS** — display formatting is **engine-owned/PRESENT**; every other gap is small view-model work or pre-known product-scope, surfaced not buried |

**Verdict for B:** clear. IronCalc's public API covers the interactive build's needs.
The one renderer-critical question (who owns display formatting) resolves in the engine's
favor. The remaining gaps are known and planned (function list = own static list; reorder
/ hidden-cols / zoom = FreeCell view model; comments/validation/hyperlinks + merges/CF =
pre-known xlsx-writing product-scope).

## Carry-forward

- **Renderer (build):** call `get_formatted_cell_value` per visible cell, or
  `number_format::format_number(value, format_string, locale)` from a display-cache; use
  the returned `Formatted.color` for `[Red]`-style formats. **No number-format engine on
  the FreeCell side.**
- **Formula bar (build):** `get_cell_content` for the string; the public `Lexer` for
  tokenizing/highlighting; the public `Parser`/`Node` for precedent (reference) extraction.
  Ship FreeCell's **own** function-name list for autocomplete (validate via parse).
- **View model (build):** FreeCell owns sheet order, column visibility, and zoom (not all
  round-trip through IronCalc); freeze panes / gridlines / selection / row-visibility use
  IronCalc and do round-trip.
- **Collaboration (later):** `flush_send_queue` / `apply_external_diffs` is the
  replica-sync transport; surgical UI updates come from mirroring the issued op (A).
- **Product-scope (pre-known):** merges, conditional formatting, comments, data
  validation, hyperlinks, dynamic arrays → owning `.xlsx` writing if pursued.

## Reproduce

```
cd experiments/round-3/B-api-audit
cargo test                 # 14 GATE tests: every "present" claim calls the real API
cargo run                  # prints the env-stamped present/absent/workaround matrix
cargo clippy --all-targets # clean
```

All API claims are backed by a runtime probe (above) or a cited `file:line` in
`~/.cargo/registry/.../ironcalc*-0.7.1/`.

> Note: this crate intentionally drops the scaffolded `datagen` / `bench_util` /
> `serde_json` dependencies — a pure API audit needs none of them; it keeps only
> `round2_harness` (for `cpu_model` env-stamping) plus `ironcalc`/`ironcalc_base` and
> `anyhow`.
