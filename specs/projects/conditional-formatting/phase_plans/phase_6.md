---
status: draft
---

# Phase 6: Rule editor — highlight rules + format editor

## Overview

P5 landed the CF sidebar's **List mode** (rows + reorder/delete). Phase 6 adds the sidebar's
second mode — the **rule Editor** for the highlight families and the **differential-format
editor** — so a user can *author* and *edit* a highlight CF rule. Color scales (a separate
editor) are P7 and are explicitly out of scope here; their dropdown entries are omitted and the
editor's edit control for an existing color-scale row is disabled until P7.

Scope (`components/cf_sidebar.md §3–§7`, `ui_design.md §2.2/§3`, `functional_spec.md §2.3/§3/§4`):
editor state, `open_cf_editor`/`cancel_cf_editor`/`save_cf_editor`, the editor body (Applies-to
range, rule-type dropdown, per-kind operands, format editor, stop-if-true, Save/Cancel),
client-side validation, and inline surfacing of an engine `Err` (keeping the editor open).

Validation is gpui view tests only — the CF sidebar has **no pixel baseline**, so the render
suite is not run (per CLAUDE.md scope).

## Steps

1. **Editor state — `chrome/cond_fmt.rs`.**
   - Add `pub editor: Option<CfEditorState>` to `CondFmtPanel`.
   - `pub(crate) enum CfEditorKind { CellValue, Text, Dates, TopBottom, Average, Duplicate,
     Blanks, Errors, Formula }` — `#[derive(Clone, Copy, PartialEq, Eq, Debug)]`. (Color-scale
     variants land in P7; omitted now so no unconstructed variant trips `-D warnings`.)
   - `pub(crate) struct CfEditorState` with the §3 fields: `edit_index: Option<u32>`,
     `kind`, `value_op: CfValueOp`, `text_op: CfTextOp`, `period: CfPeriod`, `top_rank: u32`,
     `top_percent/top_bottom/average_below/duplicate_unique/blanks_no/errors_no: bool`,
     `format: CfFormat`, `stop_if_true: bool`, `errors: Vec<String>` (engine messages), plus a
     necessary `pending_save: bool` (true between Save-send and the success/refuse signal — lets
     a success `CondFmtUpdated` return to List mode while an engine `Err` keeps the editor open).
     `CfEditorState::new(edit_index)` seeds the add-defaults (CellValue / Gt / Contains / Today /
     rank 10 / empty format).

2. **`ChromeView` editor inputs + menu state — `chrome/view.rs`.**
   - New fields: `cf_range_input`, `cf_operand1_input`, `cf_operand2_input`, `cf_formula_input:
     Entity<InputState>` (mirror the chart-title inputs); `cf_menu_open: Option<CfMenu>` where
     `enum CfMenu { RuleType, ValueOp, TextOp, Period }` drives which inline dropdown is expanded.
   - Construct the four inputs in `new()` and subscribe each to `on_cf_input_event` (a re-render
     hook so Save's disabled state + inline validation track the field text live).

3. **Open / cancel / save (methods on `ChromeView`).**
   - Replace the `open_cf_editor` stub with `open_cf_editor(&mut self, edit_index, window, cx)`:
     `None` → `CfEditorState::new(None)`, range input seeded from `self.selection.to_a1()`, the
     other inputs cleared. `Some(index)` → find the row; if its `spec` is an authorable highlight
     spec, build the state + seed inputs from it (`cf_state_from_spec`); a color-scale/absent spec
     is a no-op (edit is disabled for those in P6). Update the two render call sites to pass
     `window`.
   - `cancel_cf_editor(&mut self, cx)` → clear `editor` + `cf_menu_open`, back to List mode.
   - `save_cf_editor(&mut self, cx)` → degrade-guard; read the four input texts; run `cf_validate`
     (no-op if invalid — Save is disabled anyway); assemble a `CfRuleSpec` via `cf_build_spec`;
     send `AddCondFmt { sheet, range, spec }` (add) or `UpdateCondFmt { sheet, index, range, spec }`
     (edit); set `editor.pending_save = true` + clear `editor.errors` (stay open until the signal).
   - `refresh_cond_fmt`: after rebuilding rows, if the open editor was `pending_save`, close it
     (success returned us to List mode).
   - `rescope_cond_fmt_if_open`: also clear `editor` + `cf_menu_open` (sheet switch cancels the
     editor, `cf_sidebar.md §4`).
   - `show_cf_editor_error(&mut self, msg, cx)` (public): push `msg` into `editor.errors`, clear
     `pending_save`, keep the editor open + notify. `cf_editor_open(&self) -> bool` introspection.
   - Small mutators used by the editor controls: `toggle_cf_menu`, `select_cf_kind(kind, window,
     cx)` (reseeds operand inputs on a kind switch), `select_cf_value_op/text_op/period`,
     `set_cf_format` (preset), `set_cf_fill/text_color`, `toggle_cf_bold/italic`,
     `set_cf_stop_if_true`, and the per-kind sub-toggles (`set_cf_top_bottom`, `toggle_cf_top_percent`,
     `set_cf_average_below`, `set_cf_duplicate_unique`, `set_cf_blanks_no`, `set_cf_errors_no`) —
     all through a `with_cf_editor(cx, f)` helper that mutates the editor + clears stale errors +
     notifies.

4. **Editor body — `render_cond_fmt_sidebar` + `render_cf_editor`.** When `panel.editor.is_some()`
   render the editor instead of the list: a back-row (`chevron-left` + "New rule"/"Edit rule" →
   `cancel_cf_editor`); the **Applies to** range `Input` (+ inline range error); the **rule-type**
   inline dropdown (`CF_KIND_MENU`) driving `CfEditorKind`; the **operands** per kind (operator
   dropdown + 1–2 value inputs / text input / period dropdown / rank input + "% of range" toggle +
   Top/Bottom, Above/Below, Duplicate/Unique, Blank/No-blanks, Error/No-errors segmented toggles /
   formula input / none); the **format editor** (step 5); a **Stop if true** `Checkbox`; the live
   validation + engine errors (red); and **Save** (primary, disabled while invalid/degraded/pending)
   + **Cancel**. Inline dropdowns render their option list directly below the trigger (no anchored
   popover — the sidebar body scrolls), keyed by `debug_selector` for tests.

5. **Format editor — `render_cf_format_editor`.** A `const CF_PRESETS: [(&str, CfFormat); 5]`
   (light-red/dark-red, yellow/dark-yellow, green/dark-green, red text, bold) rendered as clickable
   chips painted with their own look (each sets `editor.format` wholesale); a compact **fill** +
   **text color** swatch palette (`FILL_PALETTE`) with "No fill"/"Automatic" (→ `None`) and a
   selected ring; **Bold**/**Italic** icon toggle buttons (`icons/bold.svg`/`italic.svg`); and a
   live **preview** cell ("123 Abc") painted with the current fill/text-color/weight/italic.

6. **Engine-error wiring — `shell/window.rs`.** In `on_edit_rejected`, route an
   `EditRejectedReason::Engine(msg)` to `chrome.show_cf_editor_error(msg)` (instead of the modal)
   **when the CF editor is open** — the inline "keep the editor open" path (`functional_spec.md §8`).

7. **Color-scale edit gate — `cf_row_controls`.** Disable a row's edit control when its `spec` is a
   `ColorScale` (P6 can't author it; P7 re-enables). Delete stays enabled.

## Tests (gpui view tests, `RecordingClient`)

- `cf_add_editor_seeds_range_from_selection` — selection B2:B20 → open add editor → `cf_range_input`
  == "B2:B20", `cf_editor_open()`.
- `cf_add_cell_value_rule_saves_add_command` — add editor, operand1 "100", apply preset 0, Save →
  `AddCondFmt { range: "B2:B20", spec: CellIs { Gt, "100", None, CF_PRESETS[0].1, false } }`.
- `cf_save_success_returns_to_list` — after Save, a `refresh_cond_fmt` (the `CondFmtUpdated` hook)
  closes the editor (back to List).
- `cf_edit_row_seeds_form_and_saves_update` — a row with `spec CellIs{Ge,"50"}` → open edit → inputs
  + state seeded → Save → `UpdateCondFmt { index, range, spec }`.
- `cf_empty_operand_blocks_save` — add editor, operand empty → `save_cf_editor` sends nothing +
  editor stays open.
- `cf_rule_type_dropdown_selects_kind` — click `cf-type-trigger` → `cf-type-text` → `editor.kind`
  == Text.
- `cf_preset_sets_format` — click `cf-preset-0` → `editor.format` == CF_PRESETS[0].1.
- `cf_preview_reflects_format` — set a fill → `cf-format-preview` painted + `editor.format.fill`
  set.
- `cf_cancel_returns_to_list` — Cancel → editor `None`, sidebar still open.
- `cf_engine_error_keeps_editor_open` — `show_cf_editor_error("bad range")` → editor open +
  message in `editor.errors`.
- `cf_color_scale_row_edit_disabled` — a `ColorScale`-spec row's edit control is disabled.
