---
status: complete
---

# Component: Engine CF integration (types, wrapper, conversions, value-dependent cache)

The engine-side half: engine-free CF vocabulary (`freecell-core`), the `WorkbookDocument` wrapper +
IronCalc↔core conversions (`freecell-engine`), the worker protocol/publish, and the value-dependent
render-cache change. Field names below are verified against the pinned fork
(`scosman/ironcalc#freecell-fixes`); still, **read the pinned source when implementing** — do not
trust these verbatim if the checkout differs.

## 1. Engine-free types — `freecell-core/src/cond_fmt.rs`

Plain `serde` data; `Rgb` is the existing `freecell_core` color. Register the module in
`freecell-core/src/lib.rs`. These types must **not** reference gpui or ironcalc (keeps
`dependency_rule.rs` green) and are re-used by the worker protocol.

```rust
pub struct CfFormat { pub fill: Option<Rgb>, pub text_color: Option<Rgb>, pub bold: bool, pub italic: bool }

pub enum CfValueOp { Gt, Lt, Ge, Le, Eq, Ne, Between, NotBetween }
pub enum CfTextOp  { Contains, NotContains, BeginsWith, EndsWith, Equals }
pub enum CfPeriod  { Today, Yesterday, Tomorrow, Last7Days, LastWeek, ThisWeek, NextWeek,
                     LastMonth, ThisMonth, NextMonth, LastYear, ThisYear, NextYear }
pub enum CfThresholdKind { Min, Max, Number, Percent, Percentile }
pub struct CfColorStop { pub kind: CfThresholdKind, pub value: Option<f64>, pub color: Rgb }

pub enum CfRuleSpec {
    CellIs { op: CfValueOp, operand: String, operand2: Option<String>, format: CfFormat, stop_if_true: bool },
    Text { op: CfTextOp, value: String, format: CfFormat, stop_if_true: bool },
    TimePeriod { period: CfPeriod, format: CfFormat, stop_if_true: bool },
    Top { rank: u32, percent: bool, bottom: bool, format: CfFormat, stop_if_true: bool },
    Average { below: bool, format: CfFormat, stop_if_true: bool },
    DuplicateValues { unique: bool, format: CfFormat, stop_if_true: bool },
    Blanks { no_blanks: bool, format: CfFormat, stop_if_true: bool },
    Errors { no_errors: bool, format: CfFormat, stop_if_true: bool },
    Formula { formula: String, format: CfFormat, stop_if_true: bool },
    ColorScale { stops: Vec<CfColorStop> },
}

pub enum CfPreview { Highlight { fill: Option<Rgb>, text_color: Option<Rgb> }, ColorScale { colors: Vec<Rgb> }, Badge(String) }
pub struct CfRuleView { pub index: u32, pub range: String, pub priority: u32, pub editable: bool,
                        pub summary: String, pub preview: CfPreview, pub spec: Option<CfRuleSpec> }
```
- Add small pure helpers here if convenient (e.g. `CfRuleSpec::format_mut()` for the editor;
  `summary_for(...)` may live engine-side where the `CfRule` is). Keep them pure + unit-tested.

## 2. IronCalc types (pinned fork — reference)

- `cf_types::CfRuleInput` — input enum; dxf variants carry `format: Dxf` (not a dxf_id).
  Variants used: `CellIs { operator: ValueOperator, formula: String, formula2: Option<String>,
  format: Dxf, stop_if_true }`, `Text { operator: TextOperator, value, format, stop_if_true }`,
  `TimePeriod { time_period: PeriodType, date1: Option<String>, date2: Option<String>, format,
  stop_if_true }`, `Top10 { rank, percent, format, stop_if_true }`, `Bottom10 {…}`,
  `AboveAverage {…}`, `BelowAverage {…}`, `DuplicateValues {…}`, `UniqueValues {…}`,
  `Blanks {…}`, `NotBlanks {…}`, `Errors {…}`, `NoErrors {…}`, `Formula { formula, format,
  stop_if_true }`, `ColorScale { thresholds: Vec<ColorScaleThreshold> }`.
- `cf_types::CfRule` — stored enum (dxf variants carry `dxf_id: u32`, `stop_if_true`). Read-back
  source for the list; deferred families are `DataBar`, `IconSet`, `IconRating`.
- `cf_types::{ValueOperator, TextOperator, PeriodType, Cfvo, ColorScaleThreshold, Color? }`.
  `Cfvo::{Min, Max, Number(f64), Percent(f64), Percentile(f64), Formula(String)}`.
  `ColorScaleThreshold { cfvo: Cfvo, color: Color }`.
- `types::Dxf { font: Option<DxfFont>, fill: Option<Fill>, border: Option<Border>,
  num_fmt: Option<NumFmt>, alignment: Option<Alignment> }`, with `Dxf::apply_to(&self, &Style) ->
  Style` (folds itself over a base style — used internally by `get_extended_style_for_cell`).
- `types::DxfFont { strike: Option<bool>, u: Option<bool>, b: Option<bool>, i: Option<bool>,
  sz: Option<i32>, color: Color }`.
- `types::Fill { color: Color }`; `types::Color::{ Rgb(String "#RRGGBB"), Theme(i32,f64), None }`.
- `UserModel` CF methods (all `Result<_, String>`): `add_conditional_formatting(sheet, range: &str,
  rule: CfRuleInput)`, `update_conditional_formatting(sheet, index: u32, new_range: &str, new_rule:
  CfRuleInput)`, `delete_conditional_formatting(sheet, index: u32)`,
  `raise_/lower_conditional_formatting_priority(sheet, index: u32)`,
  `get_conditional_formatting_list(sheet) -> Vec<ConditionalFormattingView { index: usize, range:
  String, cf_rule: CfRule, priority: u32 }>`, `get_dxf_for_conditional_formatting(sheet, index:
  u32) -> Option<Dxf>`, `get_extended_cell_style(sheet, row, col) -> ExtendedStyle { style: Style,
  icon, data_bar, rating }`.

## 3. Conversions — `freecell-engine/src/cond_fmt_convert.rs`

Isolated so no IronCalc type leaks. Functions (all pure except where noted):

- `rgb_to_color(Rgb) -> ironcalc Color::Rgb("#RRGGBB")`; `color_to_rgb(&Color) -> Option<Rgb>`
  (reuse `cache::parse_color` for `#RRGGBB`/`#AARRGGBB`; `Theme`/`None` → `None`).
- `cf_format_to_dxf(&CfFormat) -> Dxf`:
  - `fill = fill.map(|rgb| Fill { color: rgb_to_color(rgb) })`.
  - `font`: if `bold || italic || text_color.is_some()` → `Some(DxfFont { b: bold.then_some(true),
    i: italic.then_some(true), color: text_color.map(rgb_to_color).unwrap_or(Color::None),
    strike: None, u: None, sz: None })` else `None`.
  - `border/num_fmt/alignment = None`.
- `merge_cf_format_into_dxf(&CfFormat, existing: Dxf) -> Dxf` (update path): start from `existing`;
  set `fill` from the new format (or `None` to clear); on the font, **preserve** existing
  `strike/u/sz`, set `b/i/color` from the format (clear the font entirely if all three unset and no
  preserved attrs remain). Keeps `border/num_fmt/alignment` intact.
- `dxf_to_cf_format(&Dxf) -> CfFormat`: `fill = dxf.fill.as_ref().and_then(|f| color_to_rgb(&f.color))`;
  `text_color = dxf.font.as_ref().and_then(|f| color_to_rgb(&f.color))`;
  `bold = dxf.font…b == Some(true)`; `italic = dxf.font…i == Some(true)`.
- `cf_rule_spec_to_input(&CfRuleSpec, dxf: Dxf) -> CfRuleInput`: 1:1 by variant using the mappings in
  architecture §4.3 (Top{bottom}→Top10/Bottom10, Average{below}→Above/BelowAverage,
  DuplicateValues{unique}→Unique/Duplicate, Blanks{no_blanks}→NotBlanks/Blanks,
  Errors{no_errors}→NoErrors/Errors; TimePeriod date1/date2 = None; ColorScale stops →
  `ColorScaleThreshold { cfvo: kind+value → Cfvo, color }` where `Min→Cfvo::Min`, `Max→Cfvo::Max`,
  `Number(v)/Percent(v)/Percentile(v)` from `kind`+`value`). The `dxf` arg is the converted format
  (from `cf_format_to_dxf` on add, or `merge_…` on update); ColorScale ignores it.
- `cf_rule_to_view(index, range, priority, &CfRule, dxf: Option<Dxf>) -> CfRuleView`:
  - authorable variants → `editable:true`, `summary` (human string, e.g. `"Cell value > {formula}"`,
    `"Text contains \"{value}\""`, `"Top {rank}{%}"`, `"Duplicate values"`, `"3-color scale"`),
    `preview` (`Highlight{fill,text_color}` from `dxf_to_cf_format`, or `ColorScale{colors}` from the
    thresholds), and `spec: Some(reconstructed CfRuleSpec)` (operands from the `CfRule` fields +
    `format = dxf_to_cf_format(dxf)`).
  - deferred families (`DataBar/IconSet/IconRating`) and deferred variants (TimePeriod
    Between/NotBetween, ColorScale with a `Cfvo::Formula` stop) → `editable:false`,
    `preview: Badge("Data bar" | "Icon set" | "Rating" | …)`, `spec: None`.

Unit-test every arm (round-trip `spec → input → (add) → list → view.spec == spec` for each variant;
`cf_format ↔ dxf`; deferred → Badge/non-editable).

## 4. `WorkbookDocument` methods — `freecell-engine/src/document.rs` (+ `cond_fmt.rs` submodule)

Signatures per architecture §4.1. Notes:
- `add_cond_fmt`: `self.user_model_mut().add_conditional_formatting(sheet, range,
  cf_rule_spec_to_input(spec, cf_format_to_dxf(spec.format())))`. (`spec.format()` = the spec's
  `CfFormat`; ColorScale has none.)
- `update_cond_fmt`: fetch `get_dxf_for_conditional_formatting(sheet, index)` (may be `None` for
  ColorScale) → `merge_cf_format_into_dxf(new_format, existing.unwrap_or_default())` →
  `update_conditional_formatting(sheet, index, new_range, cf_rule_spec_to_input(spec, merged))`.
- `cond_fmt_rules`: for each `ConditionalFormattingView`, fetch its dxf (only for dxf variants; skip
  for ColorScale/deferred) and build `cf_rule_to_view`. Return sorted by priority desc (the list is
  already priority-sorted).
- `has_cond_fmt(sheet)`: `self.model().workbook.worksheet(sheet).map(|ws|
  !ws.conditional_formatting.is_empty()).unwrap_or(false)` (or via the list len). Cheap; used to gate
  the cache path.
- `extended_render_style(sheet, row, col, theme) -> RenderStyle`:
  `render_style_from(&self.user_model().get_extended_cell_style(sheet,row,col)?.style, theme)`.
  (Coordinates are IronCalc 1-based — reuse `to_engine_coords`. `render_style_from` is `cache.rs`.)

## 5. Worker protocol + publish — `worker/protocol.rs`, `worker/run.rs`, `worker/client.rs`

- **Commands** (protocol §4.2): `AddCondFmt/UpdateCondFmt/DeleteCondFmt/RaiseCondFmtPriority/
  LowerCondFmtPriority`. Bucketed in `process_batch` alongside style edits (own bucket, applied
  in-order with them).
- **Dispatch** (`apply_one`): call the matching `WorkbookDocument` method; propagate `Err(String)`
  back on the command-result channel (so the UI can show it). On success:
  - map to `AppliedOp::Cells { sheet, range }` for the rule's range → the existing
    `apply_cache_refresh` rebuilds/mirrors those cells (which now go through the extended path,
    §6) → `StyleCacheUpdated { sheet }`.
  - refresh the published CF map for `sheet` and emit `WorkerEvent::CondFmtUpdated { sheet }`.
- **Published CF map:** add `cond_fmt: Arc<RwLock<HashMap<SheetId, Vec<CfRuleView>>>>` to `Shared`
  (`client.rs`). The worker writes `document.cond_fmt_rules(sheet)` into it after any CF mutation and
  once on open (populate for all sheets that have rules). `DocumentClient::cond_fmt_rules(sheet) ->
  Vec<CfRuleView>` reads it (clone under the read lock). Undo/redo of a CF op also refreshes the map
  + emits `CondFmtUpdated` (the undo/redo path already re-derives caches — extend it to CF sheets).

## 6. Value-dependent render cache — `freecell-engine/src/cache.rs` + `worker/run.rs`

The one genuinely new behavior. Keep every added cost behind `has_cond_fmt(sheet)`.

- **Build/refresh path.** `build_sheet_cache` and `refresh_cell` currently read the cell's own style.
  Thread a `cf: bool` (from `document.has_cond_fmt(sheet)`, computed once per build) into them:
  - `cf == false` → unchanged fast path (`cell_own_style` → `render_style_from`). Zero overhead for
    non-CF workbooks.
  - `cf == true` → for each populated cell, `RenderStyle = document.extended_render_style(sheet,
    row, col, theme)`. (`get_extended_cell_style` returns the base style when no rule matches, so
    every cell is correct; the extra cost is bounded to populated cells.) The build already iterates
    `ws.sheet_data` populated cells — apply the extended read there. Band (row/col) styles keep the
    base path (CF is per-cell). Note the border-adjacency in `get_cell_style`/`get_extended_cell_style`
    is not needed here (borders come from the existing side-table path); use
    `get_extended_cell_style(...).style` directly for fill/font/color, matching how `refresh_cell`
    reads `cell_own_style` today (own style, not neighbor-merged).
- **Value-change invalidation (new coupling).** Today the style cache refreshes only on style edits.
  After a recompute **publishes new values**, CF results can change. In the worker's publish path
  (where `WorkerEvent::Published` is emitted), for each sheet whose values changed **and**
  `has_cond_fmt(sheet)`, call `build_and_store_cache(sheet)` (full rebuild of that sheet's style
  cache via the extended path) and emit `StyleCacheUpdated { sheet }`. Non-CF sheets are skipped
  (unchanged behavior). A full rebuild per CF sheet per recompute is the simple correct choice
  (rules can be global — Top-N/average/color-scale depend on the whole range); optimize to
  dirty-range only if perf requires (tracked as a follow-up, not first pass).
- **Grid paint unchanged.** The overlay lives in `RenderStyle.fill/font_color/bold/italic`; `grid/
  view.rs` already paints those. Data-bar/icon/rating decorations are ignored this pass
  (`ExtendedStyle.icon/data_bar/rating` dropped in `extended_render_style`).

## 7. Tests (engine)

- Conversions: every `cf_rule_spec_to_input` arm; `cf_format ↔ dxf` (incl. merge preserves
  strike/u/sz/border); `cf_rule_to_view` (authorable → editable+spec; deferred → Badge).
- `WorkbookDocument`: add→list; update merges dxf (unmodeled fields survive); delete; raise/lower
  reorders priority; `has_cond_fmt`; `extended_render_style` — a "> 100" `CellIs` rule with a fill
  yields that fill on a 150 cell and the base style on a 50 cell; a 2-color scale interpolates;
  **value change** — set the cell to 150 then to 50, extended style flips.
- Worker seam: `AddCondFmt` → `cond_fmt_rules(sheet)` reflects it + `CondFmtUpdated` +
  `StyleCacheUpdated`; `Update/Delete/Raise/Lower`; an `Err` (bad range) surfaces on the result
  channel; undo/redo restores rules + repaints; a value edit on a CF sheet refreshes the style
  cache (a Top-N cell gains/loses its fill without any CF command).
- xlsx round-trip: author highlight + color-scale rules → `save` → reopen (fresh `WorkbookDocument`)
  → `cond_fmt_rules` + `extended_render_style` match.
