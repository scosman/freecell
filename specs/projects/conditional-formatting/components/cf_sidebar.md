---
status: complete
---

# Component: CF sidebar (reusable container + ChromeView state + rendering + window wiring)

The app-side half, in `freecell-app`. All references are to the current `chrome/view.rs`
(`ChromeView`) and `shell/window.rs` (`WorkbookWindow`). Line numbers are indicative — locate by
symbol.

## 1. Reusable docked-sidebar container — `chrome/sidebar.rs` (new)

Extract the chart panel's card shell (`render_chart_panel`, ~`chrome/view.rs:5024–5167`) into:
```rust
pub(crate) fn docked_sidebar(
    id: &'static str,
    title: impl Into<SharedString>,
    on_close: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    body: impl IntoElement,
) -> impl IntoElement
```
Renders exactly today's card (ui_design §0): `absolute` / `top(ACTION_ROW_H + DATA_ROW_H)` /
`right_0` / `bottom(TAB_BAR_H)` / `w(SIDEBAR_W)` / `occlude` / `flex_col` / `overflow_hidden` /
`bg(ACTIVE_TAB_BG)` / `border_l_1` / `border_color(HAIRLINE)` / `shadow_md`, a **pinned header**
(`flex_shrink_0`, `px_3 pt_3 pb_2`, `justify_between` title + `close_button(<id>-close, on_close)`),
and a **scrolling body** (`div().id(<id>-body).flex_1().min_h_0().overflow_y_scroll().px_3().pb_3()`
wrapping `body`).
- Move/keep the layout consts (`ACTION_ROW_H`, `DATA_ROW_H`, `TAB_BAR_H`, `SIDEBAR_W` = the old
  `CHART_PANEL_W` 268px) accessible to this module.
- Also promote the `section` / `section_label` closures (chart panel local closures) to
  `pub(crate)` helpers here (`section(label, body)`, `section_label(text)`) so both panels share
  them.
- Refactor `render_chart_panel` to build its `sections` `Vec<AnyElement>` and call
  `docked_sidebar("chart-panel", "Edit chart", close→close_chart_panel, body)`. **No visual change.**
  Guard: the chart panel has no pixel baseline, so validate via the existing chart-panel view tests +
  an Xvfb smoke launch (not the pixel suite).

## 2. Mutual exclusion with the chart panel

Both dock right. First pass: opening one **closes the other**. In `open_cond_fmt`/`toggle_cond_fmt`
call `self.close_chart_panel(cx)` first; in `open_chart_panel` call `self.close_cond_fmt(cx)` first.

## 3. `ChromeView` state (new)

Add fields (near `chart_panel`, ~`chrome/view.rs:408`):
```rust
cond_fmt: Option<CondFmtPanel>,          // Some ⇒ sidebar open
// editor inputs (seeded on editor open, like chart_title_input):
cf_range_input: Entity<InputState>,
cf_operand1_input: Entity<InputState>,
cf_operand2_input: Entity<InputState>,
cf_formula_input: Entity<InputState>,
cf_stop_value_inputs: Vec<Entity<InputState>>,   // color-scale stop values (≤3)
```
Model types (in `chrome/cond_fmt.rs`, a new submodule of `chrome`):
```rust
pub struct CondFmtPanel { pub sheet: SheetId, pub rows: Vec<CfRuleView>, pub editor: Option<CfEditorState> }
pub struct CfEditorState {
    pub edit_index: Option<u32>,          // None = add, Some = edit
    pub kind: CfEditorKind,               // which rule family/variant is selected
    pub value_op: CfValueOp, pub text_op: CfTextOp, pub period: CfPeriod,
    pub top_rank: u32, pub top_percent: bool, pub top_bottom: bool, pub average_below: bool,
    pub duplicate_unique: bool, pub blanks_no: bool, pub errors_no: bool,
    pub format: CfFormat,
    pub scale: Vec<CfColorStop>,          // 2–3 stops for ColorScale
    pub stop_if_true: bool,
    pub errors: Vec<String>,              // engine/validation messages
}
pub enum CfEditorKind { CellValue, Text, Dates, TopBottom, Average, Duplicate, Blanks, Errors, Formula, ColorScale2, ColorScale3 }
```
`CfRuleView`, `CfFormat`, `CfColorStop`, and the op/period enums come from `freecell_core::cond_fmt`.

## 4. Open / close / toggle (methods on `ChromeView`)

Mirror `open_chart_panel`/`close_chart_panel` (~2049–2099):
- `toggle_cond_fmt_sidebar(cx)` — if open, close; else open in **List mode**. On open:
  `close_chart_panel(cx)`, build `CondFmtPanel { sheet: active_sheet, rows:
  client.cond_fmt_rules(sheet), editor: None }`, `cx.notify()`.
- `close_cond_fmt(cx)` — `self.cond_fmt.take(); cx.notify()`.
- `cond_fmt_open() -> bool`.
- `refresh_cond_fmt(cx)` — rebuild `rows` from `client.cond_fmt_rules(sheet)` for the panel's sheet;
  keep any open editor; called on `CondFmtUpdated` + after a CF command.
- `open_cf_editor(edit_index, cx)` / `cancel_cf_editor(cx)` — set/clear `editor`; on open, seed the
  text inputs + `CfEditorState` from the row's `spec` (edit) or defaults (add; `range` seeded from
  the current selection A1).
- Editor commit `save_cf_editor(cx)` — validate (§6), assemble a `CfRuleSpec` from `CfEditorState`,
  send `Command::AddCondFmt`/`UpdateCondFmt`, return to List mode (rows refresh on `CondFmtUpdated`).

**Selection-change exemption:** in `on_selection_changed` (~723–776) the chart panel is closed
(line ~772). Do **not** close the CF sidebar there — only refresh nothing (leave it open). If the
active **sheet** changed, call `refresh_cond_fmt` + `cancel_cf_editor`.

## 5. Rendering

- **Action-bar button:** in `render_action_row`, add (formatting cluster, near the chart button
  ~3669) a ghost/small `Button::new("cond-fmt").icon(Icon::empty().path("icons/split.svg"))
  .tooltip("Conditional formatting").selected(self.cond_fmt_open()).disabled(disabled)
  .on_click(→ toggle_cond_fmt_sidebar)`. Verify `icons/split.svg` resolves from the gpui-component
  bundle; if not, vendor it (`assets/icons/split.svg`, tintable) + register in `shell/assets.rs`.
- **Overlay:** push `render_cond_fmt_sidebar` in `render_overlays` (~4420–4473) next to the chart
  panel, `when(self.cond_fmt.is_some())`.
- **`render_cond_fmt_sidebar(cx) -> AnyElement`:** build `body` = list mode or editor mode by
  `panel.editor`, then `docked_sidebar("cond-fmt", "Conditional formatting",
  cx.listener(close_cond_fmt), body)`.
  - **List body:** intro line (sheet name), a `+ Add rule` button (→ `open_cf_editor(None)`), then a
    `children(rows.map(render_cf_row))`. `render_cf_row(row)`: a `flex items_center gap_2` with the
    **preview swatch** (a small `div` filled per `row.preview`: Highlight → fill bg + an "A" in
    text_color; ColorScale → a `flex` of thin color bands; Badge → a muted tag), a two-line
    **summary/range**, and right-aligned controls: move-up (`chevron-up`, disabled if first),
    move-down (`chevron-down`, disabled if last), edit (`pencil`, disabled if `!row.editable`),
    delete (`trash-2`). Wire each to the matching `Command` (Raise/Lower/Delete) or
    `open_cf_editor(Some(row.index))`.
  - **Editor body:** a back-row (`chevron-left` + "New rule"/"Edit rule"), then per ui_design §2.2:
    Applies-to input (+ inline error), rule-type dropdown (drives `CfEditorKind`), operand controls
    (per kind), the **format editor** (§7) or **color-scale editor** (§8), stop-if-true checkbox,
    and Save/Cancel. Any `panel.editor.errors` render inline (red, small) above Save.

## 6. Validation (client-side, cheap)

Before enabling Save (and re-checked on Save): range non-empty; CellValue operands non-empty
(operand2 required for Between/NotBetween); Text value non-empty; Formula non-empty; Top rank ≥ 1;
color-scale Number/Percent/Percentile stops have a value. The engine's `Err(String)` (returned via
the command result path) is authoritative and shown in `editor.errors` — the editor **stays open**
on an engine error (no return to List mode).

## 7. Format editor (`render_cf_format_editor`)

Reuse existing chrome building blocks:
- **Presets row:** a few chips (label + look) that set `format` in one click (e.g. light-red/dark-red,
  yellow/dark-yellow, green/dark-green, red text, bold). Define the preset table as a `const`.
- **Fill + Text color:** reuse the action-row **fill color picker** pattern (the swatch grid +
  custom hex used by the fill popover — factor a small shared color-swatch-grid helper if the
  existing one isn't reusable as-is; otherwise inline the same grid). "No fill"/"Automatic" allowed
  → `None`.
- **Bold / Italic:** two small toggle icon buttons (`bold.svg`/`italic.svg`) bound to
  `format.bold`/`format.italic`.
- **Preview:** a sample `div` ("123 Abc") painted with `format.fill` bg + `format.text_color` +
  weight, updated from `CfEditorState.format`.

## 8. Color-scale editor (`render_cf_scale_editor`)

- **2 vs 3 color** segmented control (sets `CfEditorKind::ColorScale2/3` and resizes `scale`).
- **Default presets** chips (e.g. green-yellow-red, white-blue) that fill `scale` colors + kinds.
- **Per-stop row:** threshold-kind dropdown (Min/Max only on the endpoints; Number/Percent/
  Percentile everywhere), a value input (when not Min/Max) bound to `cf_stop_value_inputs[i]`, and a
  color swatch. Endpoints default Min/Max; midpoint defaults 50th percentile.
- **Preview:** a horizontal gradient `div` across the stop colors.

## 9. Window wiring — `shell/window.rs`

- Handle `WorkerEvent::CondFmtUpdated { sheet }` (near the `StyleCacheUpdated` handler ~497–509):
  if the chrome's CF panel targets `sheet`, `chrome.update(cx, |c, cx| c.refresh_cond_fmt(cx))`.
- On **sheet switch** (wherever the active sheet changes / chart panel is reconciled), if the CF
  sidebar is open, `refresh_cond_fmt` for the new sheet + cancel any open editor (handled inside
  `on_selection_changed`/sheet-change path — §4).
- The rule rows are read straight from `DocumentClient::cond_fmt_rules(sheet)` (engine-free), so the
  window needs no new builder beyond calling `refresh_cond_fmt`.

## 10. Tests (gpui view tests — no pixels)

- Button toggles the sidebar open/closed; `selected` tracks state; disabled when degraded.
- Opening the CF sidebar closes an open chart panel and vice-versa.
- List renders one row per `CfRuleView` with correct summary/range; a `Badge`/non-editable row's
  edit control is disabled but delete is enabled.
- Move-up/down send `Raise/LowerCondFmtPriority`; delete sends `DeleteCondFmt`.
- Editor: selecting CellValue + entering an operand + a format → Save sends `AddCondFmt` with the
  expected `CfRuleSpec`; editing a row seeds the form from `spec` and Save sends `UpdateCondFmt`;
  empty operand disables Save; an injected engine `Err` keeps the editor open with the message.
- ColorScale editor: 3-color → Save sends a `ColorScale` spec with 3 stops.
- Selection change does **not** close the sidebar; a sheet change refreshes it.
- Chart-panel refactor: existing chart-panel view tests still pass (container extraction is
  behavior-preserving).
