---
status: complete
---

# Phase 1: Quick wins & publication

## Overview

Phase 1 is the dependency-free foundation of MVP-Gaps (architecture §10: "§1.2/§1.3
publication has no dependents — first"). It ships four independent quick wins plus the
publication data-model change every later formatting phase reads:

1. **`PublishedCell.kind` + populated `text_color`** (§1.2) — the worker now classifies
   each published cell (Number/Date/Text/Bool/Error, with a date-format heuristic) and
   resolves its text colour (explicit font colour → number-format `[Red]`-style colour →
   none).
2. **Type-aware default alignment** (§1.3, GAPS #1) — the grid aligns cells with no
   explicit alignment by evaluated type: numbers/dates right, booleans/errors center,
   text left. Explicit alignment still wins.
3. **`[Red]` number-format colour** (§3.5, GAPS #2) — carried by (1); the grid already
   consumes `PublishedCell.text_color`.
4. **Cap-error popover, data-row only** (§7.2) — the existing no-op
   `DataRowEffect::ShowCapError` now drives an anchored tooltip under the data row
   ("Formula too long (max 8,192 characters)" / "Formula nested too deeply (max 64
   levels)").
5. **`.back` backup before first save** (§7.3) — a write-once copy of a disk-opened
   file's original bytes before the first save-in-place; copy failure aborts the save
   with a dialog.

Also: update the GAPS.md rows for #1/#2, and record judgment calls in
`DECISIONS_TO_REVIEW.md`.

Out of scope for Phase 1 (later phases): the in-cell editor cap popover (Phase 2), font/
border/num-fmt cache side tables (Phases 4–6), the SetStylePath/SetBorders/SetFont
commands.

## Steps

### A. Publication data model + derivation (§1.2)

1. **`freecell-core/src/publication.rs`** — add `CellKind`:
   ```rust
   #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
   #[repr(u8)]
   pub enum CellKind {
       Number, Date, #[default] Text, Bool, Error,
   }
   impl CellKind {
       /// Type-aware default horizontal alignment when a cell has no explicit alignment
       /// (`architecture.md §1.3`): numbers/dates right, booleans/errors center, text left.
       pub fn default_align(self) -> Align { … }
   }
   ```
   Add `pub kind: CellKind` to `PublishedCell` (default `Text`). Update the two in-file
   test constructions. Import `crate::style::Align`.

2. **`freecell-core/src/format_color.rs`** (NEW module) — pure, engine-free, unit-tested:
   ```rust
   /// True if `fmt` is a date/time number format: strip `[...]` sections and
   /// `"quoted"` / `\`-escaped literals, then look for any of `y m d h s`.
   pub fn is_date_format(fmt: &str) -> bool { … }

   /// Map an IronCalc number-format colour index (from `format_number().color`) to an RGB.
   /// Named colours 0–6 (black, white, red, green, blue, yellow, magenta) — covers `[Red]`
   /// and all named format colours (GAPS #2). `[Color N]` for N>6 → None (default text).
   pub fn format_color_rgb(index: i32) -> Option<Rgb> { … }
   ```
   Register `pub mod format_color;` in `lib.rs`; re-export `CellKind`.

3. **`freecell-engine/src/document.rs`** — add:
   ```rust
   pub(crate) fn published_style(&self, sheet: u32, cell: CellRef)
       -> Result<(CellKind, Option<Rgb>), CellQueryError>
   ```
   - `kind`: `self.model.get_cell_type(sheet,row,col)` → map `CellType::{Number,Text,
     LogicalValue,ErrorValue}` → `CellKind::{Number,Text,Bool,Error}` (Array/CompoundData
     → Text). Reclassify Number → Date when `format_color::is_date_format(&style.num_fmt)`.
   - `text_color` (fully resolved): explicit `style.font.color` via existing
     `parse_color`, black-filtered → that; else if `style.num_fmt.contains('[')` and the
     cell value is `CellValue::Number(v)`, call
     `ironcalc_base::formatter::format::format_number(v, &num_fmt, locale)` and map
     `.color` via `format_color::format_color_rgb`; else None. Locale via
     `get_locale(&model.workbook.settings.locale)`.
   - Style read: `get_style_for_cell`; value: `get_cell_value_by_index`.

4. **`freecell-engine/src/worker/run.rs`** `build_publication` (~:632) — for each non-empty
   cell, call `self.doc.published_style(idx, CellRef::new(row,col))` and populate `kind` +
   `text_color` (fall back to `(Text, None)` on the rare `Err`). Remove the stale
   "text_color = None" comment.

### B. Grid type-aware alignment (§1.3)

5. **`freecell-app/src/grid/view.rs`** — plumb `kind` into `cell_element`:
   - Import `freecell_core::publication::CellKind`.
   - At the cell build site (~:913) extract `pc.kind` (default `CellKind::Text` for the
     empty-cell branch) and pass it to `cell_element`.
   - `cell_element` gains a `kind: CellKind` param; alignment becomes
     `style.and_then(|s| s.h_align).unwrap_or_else(|| kind.default_align())`. The text-colour
     line (`pc.text_color.or(style.font_color)`) is unchanged (pc.text_color is already
     fully resolved).

6. **`freecell-app/src/grid/fixtures.rs`** — add `kind: CellKind::Text` to the `cell()`
   fixture helper (demo only).

### C. Cap-error popover, data row only (§7.2)

7. **`freecell-core/src/input_cap.rs`** — add `InputRejection::message(&self) -> String`
   ("Formula too long (max 8,192 characters)" / "Formula nested too deeply (max 64
   levels)") with a small `group_thousands` helper (8192 → "8,192"); unit-test both.

8. **`freecell-core/src/data_row.rs`** — replace the `cap_error: bool` field with
   `cap_rejection: Option<InputRejection>`; keep `cap_error()` (now
   `self.cap_rejection.is_some()`) and add `cap_rejection() -> Option<InputRejection>`. The
   `Commit`/`EditCommitRequested` Err arms store `Some(rej)`; all reset points set `None`.
   `DataRowEffect::ShowCapError` stays payload-free (popover renders from state, like the
   danger border). Existing reducer tests unchanged (they assert `cap_error()` bool +
   `ShowCapError`).

9. **`freecell-app/src/chrome/view.rs`**:
   - Change `cap_error_external: bool` → `Option<InputRejection>`; bind the rejection in
     the `EditRejected { InputCap(rej) }` arm; resets set `None`; clear on `InputEvent::Blur`.
   - `cap_error_visible()` → `data_row.cap_error() || cap_error_external.is_some()`.
   - Add `cap_error_message() -> Option<String>` = `data_row.cap_rejection().or(external).map(|r| r.message())`.
   - `render_overlays`: when a message exists, push an anchored dark tooltip div
     (`top ACTION_ROW_H+DATA_ROW_H`, `left ~97`, `bg #333`, white 11px text, rounded,
     shadow) under the data-row content field.

### D. `.back` backup before first save (§7.3)

10. **`freecell-app/src/shell/lifecycle.rs`** — pure, unit-tested:
    ```rust
    /// `<path>` + ".back" (e.g. Budget.xlsx → Budget.xlsx.back).
    pub fn backup_path(path: &Path) -> PathBuf { … }
    /// The backup to create, or None. Some(back) iff the doc was opened from disk, the save
    /// target is that same path, and `<path>.back` does not already exist (write-once).
    pub fn backup_target(opened_from: Option<&Path>, save_target: &Path) -> Option<PathBuf> { … }
    ```

11. **`freecell-app/src/shell/window.rs`**:
    - Capture `opened_from: Option<PathBuf>` in `build` (= the `path` arg at construction,
      before any save-as mutates `self.path`). Store as a field.
    - In `send_save(path)`, before dispatching `Command::Save`: if
      `backup_target(opened_from, &path)` is `Some(back)`, `std::fs::copy(&path, &back)`; on
      `Err`, show `ActiveModal::Error { title: "Couldn't create backup", detail: "File not
      saved.", close_window_on_dismiss: false }`, abort the save (clear pending, cancel the
      close/quit follow-up), and return — do not send `Command::Save`.

### E. Docs / test suite

12. **`render-tests/src/cases.rs`** — fix the stale `cell_number_negative_red` comment;
    add `cell_number_align_left` (number + explicit `Align::Left`) to guard "explicit
    alignment beats the new numeric type-default". Add its name to
    **`render-tests/tests/render_suite.rs`**' `render_cases!` list.
13. **`GAPS.md`** — mark #1 (type-aware alignment) and #2 (`[Red]` colour) resolved.
14. **`DECISIONS_TO_REVIEW.md`** — create + record the judgment calls (see below).

## Tests

Unit (freecell-core, headless):
- `cell_kind_default_align` — Number/Date→Right, Bool/Error→Center, Text→Left.
- `is_date_format` — `m/d/yyyy`✓, `h:mm AM/PM`✓, `yyyy\-mm`✓, `[Red]0.00`✗ (bracket
  section stripped), `"months"@`✗ (quoted literal stripped), `#,##0.00`✗, `@`✗, `general`✗.
- `format_color_rgb` — 2→red `0xFF0000`, 0→black, 6→magenta, 7+→None, negative→None.
- `input_rejection_message` — TooLong → "Formula too long (max 8,192 characters)";
  TooDeeplyNested → "Formula nested too deeply (max 64 levels)".
- `group_thousands` — 8192→"8,192", 64→"64", 0→"0", 1000000→"1,000,000".
- data_row: existing cap tests stay green; add `cap_rejection_exposes_kind` (Commit of an
  over-long formula sets `cap_rejection()` to `TooLong`, cleared by a subsequent `Edited`).

Unit (freecell-app shell, headless):
- `backup_path_appends_back` — `/d/Budget.xlsx` → `/d/Budget.xlsx.back`.
- `backup_target_first_save_in_place` — opened-from P, save to P, no `.back` → `Some`.
- `backup_target_writeonce` — `.back` already exists → `None` (tempdir).
- `backup_target_save_as_new_path` — save target ≠ opened-from → `None`.
- `backup_target_new_document` — opened_from `None` → `None`.

Engine integration (freecell-engine, real UserModel):
- `published_kind_maps_cell_types` — number, text, `TRUE`, `=1/0`, and a date-formatted
  cell publish Number/Text/Bool/Error/Date.
- `published_red_color_for_bracket_format` — a cell with a `$#,##0.00;[Red]$#,##0.00`
  number format + a negative value publishes red `text_color`; a positive value publishes
  None; an explicit non-black font colour wins over the format colour.

Render suite (pinned-runner regen required — see DECISIONS): existing number/date/bool/
error/`cell_number_negative_red`/`grid_mixed_content` baselines change (alignment + red);
new `cell_number_align_left`.
