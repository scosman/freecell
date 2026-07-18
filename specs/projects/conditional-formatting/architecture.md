---
status: complete
---

# Architecture: Conditional Formatting

This is a **pure FreeCell-side integration**: the IronCalc CF engine (evaluation, model, save/load)
is already present on the fork's `freecell-fixes` branch. We wrap its `UserModel` CF API behind
FreeCell's engine-free seams, plumb it through the worker protocol, fold the **effective (extended)
style** into the resident render cache with **value-dependent invalidation**, and build the sidebar
UI on a reusable docked-container component.

Two component designs carry the detailed, high-risk seams:
- [`components/engine_cf.md`](components/engine_cf.md) — engine-free CF type model, `WorkbookDocument`
  methods, IronCalc↔core conversions, and the value-dependent render-cache integration.
- [`components/cf_sidebar.md`](components/cf_sidebar.md) — reusable `docked_sidebar`, `ChromeView`
  CF state, list + editor rendering, window wiring.

This doc gives the whole-system design; the component docs give the field-level detail.

---

## 1. The existing pipeline (recap)

```
UI (ChromeView, gpui)
   │  self.client.send(Command::…)                         ← mutations
   ▼
worker thread (run.rs)  →  WorkbookDocument  →  ironcalc_base::UserModel   [only place IronCalc lives]
   │                                   │
   │  publishes                        │ builds engine-free caches
   ▼                                   ▼
Shared { ArcSwap<Publication>, RwLock<SheetCaches> }   ← UI reads synchronously
   ▲
GridView paints from SheetCaches (RenderStyle) + Publication (values, resolved text_color)
```

- No IronCalc type may cross out of `freecell-engine` (enforced by `freecell-core`'s
  `dependency_rule.rs`). Engine-free shared types live in **`freecell-core`**; the worker protocol
  (`worker/protocol.rs`) is engine-free and uses them.
- Style render form: `freecell_core::style::RenderStyle` (bold/italic/…/fill/font_color/…), interned
  into `SheetCache` by `freecell_engine::cache::render_style_from(style, theme)`.
- Mutation loop template (we mirror it): button → `Command` → worker `apply_one` →
  `WorkbookDocument` → `UserModel` → cache refresh → `WorkerEvent::StyleCacheUpdated` → grid repaint.

## 2. Data model

### 2.1 Engine-free CF types (new `freecell-core::cond_fmt`)
The shared vocabulary used by the protocol and the UI. Plain data, `serde`, no gpui/ironcalc.

- **`CfFormat { fill: Option<Rgb>, text_color: Option<Rgb>, bold: bool, italic: bool }`** — the
  editable differential-format subset (first pass). Maps to/from IronCalc `Dxf` (fill.fg_color,
  font.color, font.b, font.i); all other `Dxf` fields untouched (preserved on edit — §4.3).
- **`CfRuleSpec`** — engine-free rule definition for add/update. Variants (in-scope only):
  `CellIs { op: CfValueOp, operand: String, operand2: Option<String>, format, stop_if_true }`,
  `Text { op: CfTextOp, value: String, format, stop_if_true }`,
  `TimePeriod { period: CfPeriod, format, stop_if_true }`,
  `Top { rank: u32, percent: bool, bottom: bool, format, stop_if_true }`,
  `Average { below: bool, format, stop_if_true }`,
  `DuplicateValues { unique: bool, format, stop_if_true }`,
  `Blanks { no_blanks: bool, format, stop_if_true }`,
  `Errors { no_errors: bool, format, stop_if_true }`,
  `Formula { formula: String, format, stop_if_true }`,
  `ColorScale { stops: Vec<CfColorStop> }`.
  Enums: `CfValueOp` (Gt/Lt/Ge/Le/Eq/Ne/Between/NotBetween), `CfTextOp`
  (Contains/NotContains/BeginsWith/EndsWith/Equals), `CfPeriod` (Today/Yesterday/Tomorrow/
  Last7Days/Last|This|NextWeek/Last|This|NextMonth/Last|This|NextYear),
  `CfColorStop { kind: CfThresholdKind, value: Option<f64>, color: Rgb }`,
  `CfThresholdKind` (Min/Max/Number/Percent/Percentile).
- **`CfRuleView { index: u32, range: String, priority: u32, editable: bool, summary: String,
  preview: CfPreview, spec: Option<CfRuleSpec> }`** — read model for the list. `preview` =
  `Highlight { fill: Option<Rgb>, text_color: Option<Rgb> } | ColorScale { colors: Vec<Rgb> } |
  Badge(String)`. `spec` is `Some` for authorable types (seeds the editor); `None` +
  `editable=false` for deferred-family / deferred-variant rules (list shows + can delete, can't
  edit — functional_spec §9).

### 2.2 UI state (`ChromeView`)
- `cond_fmt: Option<CondFmtPanel>` — `Some` ⇒ sidebar open.
- `CondFmtPanel { sheet: SheetId, rows: Vec<CfRuleView>, editor: Option<CfEditorState> }`.
- `CfEditorState { edit_index: Option<u32>, range: String, kind: CfEditorKind, operands…, format:
  CfFormat, scale: Vec<CfColorStop>, stop_if_true: bool, errors: Vec<String> }` (detail in
  `components/cf_sidebar.md`).
- Editor text inputs (`Entity<InputState>`): range, operand1, operand2, formula, per-stop values —
  seeded on editor open (mirrors chart panel inputs).

## 3. Component breakdown

| Component | Home | Responsibility |
|---|---|---|
| Engine-free CF types | `freecell-core/src/cond_fmt.rs` | Shared vocabulary (§2.1). |
| CF engine wrapper | `freecell-engine/src/document.rs` (+ `cond_fmt.rs` submodule) | Wrap `UserModel` CF API; convert core↔IronCalc; extended-style read. |
| CF ↔ IronCalc conversions | `freecell-engine/src/cond_fmt_convert.rs` | `CfRuleSpec`↔`CfRuleInput`, `CfRule`→`CfRuleView`, `CfFormat`↔`Dxf`, `Rgb`↔`Color`. |
| Value-dependent cache | `freecell-engine/src/cache.rs` + `worker/run.rs` | Fold extended style into `SheetCache`; invalidate on value publish for CF sheets. |
| Worker protocol | `freecell-engine/src/worker/protocol.rs` | `Command` CF variants; `WorkerEvent::CondFmtUpdated`. |
| CF rule publish + client | `worker/run.rs`, `worker/client.rs` | Shared `HashMap<SheetId, Vec<CfRuleView>>`; `DocumentClient::cond_fmt_rules`. |
| Reusable sidebar | `freecell-app/src/chrome/sidebar.rs` | `docked_sidebar(id, title, on_close, body)`. |
| CF sidebar UI | `freecell-app/src/chrome/view.rs` (+ `cond_fmt.rs` submodule) | Button, state, list + editor rendering. |
| Window wiring | `freecell-app/src/shell/window.rs` | Build `CfRuleView`s → panel; refresh on `CondFmtUpdated` + sheet switch. |
| Grid rendering | `freecell-app/src/grid/view.rs` | **No change** — CF overlay for in-scope families is already a `RenderStyle` (fill/font/color). |

## 4. Public interfaces (the hard problems, solved)

### 4.1 `WorkbookDocument` (engine-free in/out)
```
// mutate (each returns Result<(), String>; all undoable via UserModel diffs)
fn add_cond_fmt(&mut self, sheet, range: &str, spec: &CfRuleSpec) -> Result<(), String>;
fn update_cond_fmt(&mut self, sheet, index: u32, range: &str, spec: &CfRuleSpec) -> Result<(), String>;
fn delete_cond_fmt(&mut self, sheet, index: u32) -> Result<(), String>;
fn raise_cond_fmt(&mut self, sheet, index: u32) -> Result<(), String>;
fn lower_cond_fmt(&mut self, sheet, index: u32) -> Result<(), String>;
// read
fn cond_fmt_rules(&self, sheet) -> Result<Vec<CfRuleView>, String>;   // list → view models
fn has_cond_fmt(&self, sheet) -> bool;                                 // fast gate for the cache
fn extended_render_style(&self, sheet, row, col, theme) -> RenderStyle // effective style incl. CF
```
- `add/update/delete/raise/lower` delegate to the identically-named `UserModel` methods after
  converting `CfRuleSpec → CfRuleInput` (§4.3). `update` first fetches the existing `Dxf`
  (`get_dxf_for_conditional_formatting`) and **merges** the edited `CfFormat` onto it so unmodeled
  `Dxf` fields survive an edit.
- `cond_fmt_rules` maps `get_conditional_formatting_list(sheet)` → `Vec<CfRuleView>`: builds the
  human `summary`, the `preview`, sets `editable`/`spec` (Some for in-scope variants; for
  authorable rules it reconstructs `CfRuleSpec` from the stored `CfRule` + its fetched `Dxf`).
- `extended_render_style` calls `UserModel::get_extended_cell_style(sheet,row,col)` and runs its
  `.style` through the existing `render_style_from(style, theme)` — i.e. the effective (base+CF)
  style becomes an ordinary `RenderStyle`. (`.icon`/`.data_bar`/`.rating` are ignored this pass.)

### 4.2 Worker protocol (`worker/protocol.rs`)
```
enum Command {                       // new variants
  AddCondFmt   { sheet, range: String, spec: CfRuleSpec },
  UpdateCondFmt{ sheet, index: u32, range: String, spec: CfRuleSpec },
  DeleteCondFmt{ sheet, index: u32 },
  RaiseCondFmtPriority { sheet, index: u32 },
  LowerCondFmtPriority { sheet, index: u32 },
}
enum WorkerEvent { CondFmtUpdated { sheet: SheetId } }   // new; prompts the window to rebuild rows
```
- Dispatch in `apply_one` (`run.rs`): call the matching `WorkbookDocument` method. On success the op
  maps to `AppliedOp::Cells{ sheet, range }` covering the rule's range (so the **style cache
  refreshes for that range**), plus refresh the **published CF rule map** for the sheet and emit
  `CondFmtUpdated { sheet }` and `StyleCacheUpdated { sheet }`.
- CF commands go in their own bucket in `process_batch`, ordered with the other style edits.

### 4.3 CF ↔ IronCalc conversions (`cond_fmt_convert.rs`)
- `CfRuleSpec → ironcalc CfRuleInput`: 1:1 by variant (CellIs→CellIs with `ValueOperator`, operand
  strings; Text→Text with `TextOperator`; TimePeriod→TimePeriod (date1/date2 = None for the
  parameterless periods); Top{bottom:false}→Top10, Top{bottom:true}→Bottom10; Average{below}→
  AboveAverage/BelowAverage; DuplicateValues{unique}→UniqueValues/DuplicateValues;
  Blanks{no_blanks}→NotBlanks/Blanks; Errors{no_errors}→NoErrors/Errors; Formula→Formula;
  ColorScale→ColorScale with `ColorScaleThreshold{ cfvo, color }`).
- `CfFormat → Dxf`: set `fill = Some(Fill{ fg_color: Some(color)… })` when fill set;
  `font = Some(DxfFont{ b, i, color, … })` when any font attr set; else `None`.
- `Dxf → CfFormat` (readback): pull fill.fg_color, font.color/b/i.
- `Rgb ↔ ironcalc Color`: `Rgb → Color::Rgb("#RRGGBB")`; `Color → Rgb` parsing `#RRGGBB`/`#AARRGGBB`
  (reuse `cache::parse_color`).
- `ironcalc CfRule → CfRuleView`: match variant → `summary` string + `preview` + `spec`
  reconstruction; deferred families (DataBar/IconSet/IconRating) and deferred variants →
  `editable:false, spec:None, preview:Badge(<label>)`.

### 4.4 Value-dependent render cache (the crux)
Today `build_sheet_cache`/`refresh_cell` read the cell's stored style. Change, gated on
`document.has_cond_fmt(sheet)`:
- **Build/refresh:** for a CF sheet, a populated cell's `RenderStyle` comes from
  `extended_render_style(sheet,row,col,theme)` instead of the base-style path. (`get_extended_cell_style`
  returns the base style when no rule matches, so this is always correct — just does the CF
  evaluation read. Non-CF sheets keep the existing fast path untouched → zero overhead when CF is
  unused.)
- **Value-change invalidation:** after a recompute publishes new values, the worker's publish path
  checks each sheet touched by the recompute; **for CF sheets it rebuilds that sheet's style cache**
  (extended read) and emits `StyleCacheUpdated { sheet }`. This is the new coupling
  (value publish → style refresh) that today does not exist — CF is the first value-dependent style.
  Scope it to CF sheets so non-CF workbooks are unaffected. Detail + the exact hook in
  `components/engine_cf.md`.
- **Grid paint: unchanged.** The overlay is already inside `RenderStyle` (fill/font/color), so
  `grid/view.rs` paints it with today's code. (Data-bar/icon/rating decorations — future — would add
  `RenderStyle`/cache decoration fields + new paint code; not this pass.)

### 4.5 Reusable sidebar + client read
- `chrome/sidebar.rs::docked_sidebar(id, title, on_close, body) -> impl IntoElement` (ui_design §0);
  `render_chart_panel` refactored onto it (no behavior change).
- `DocumentClient::cond_fmt_rules(sheet) -> Vec<CfRuleView>` reads the shared published map
  (`Arc<RwLock<HashMap<SheetId, Vec<CfRuleView>>>>` added to `Shared`), refreshed by the worker on
  CF mutations + on open. The window builds `CondFmtPanel.rows` from it after `CondFmtUpdated` and on
  sheet switch (mirrors `chart_panel_info`).

## 5. Design patterns & rationale

- **Engine-free seam preserved.** All CF crosses the worker boundary as `freecell-core` types; the
  IronCalc↔core conversion is isolated in `freecell-engine`. Keeps `dependency_rule.rs` green and
  the UI testable without a real engine.
- **Overlay-as-style.** Reading `get_extended_cell_style().style` reuses the entire existing
  fill/font/color render path — the first pass needs **zero** new grid draw code. This is why
  highlight + color scales are the low-risk core; data-bar/icon/rating (decorations) are the phases
  that add primitives.
- **CF-sheet gating.** Every added cost (extended reads, value-publish→style-refresh) is behind
  `has_cond_fmt(sheet)`, so a workbook with no CF pays nothing — protects FreeCell's huge-sheet perf.
- **List-published, not queried.** CF rules ride a shared map refreshed on change (like the style
  cache), so the window reads them synchronously to build the panel — no request/response round-trip.

## 6. Error handling

- Engine `Err(String)` from add/update (bad range/formula/operand) propagates via the `Command`
  result path to the sidebar, shown inline; the editor stays open, nothing partially applied.
- The worker validates nothing itself; the engine is the source of truth. The UI does cheap
  client-side guards (non-empty operands, numeric rank) to disable Save early, but the engine's
  error is authoritative.
- A CF read failure (`Result::Err`) for a sheet degrades to "no CF" (empty list, base-style cache),
  never a crash — logged.

## 7. Testing strategy

- **freecell-core:** `cond_fmt` type unit tests (serde round-trip; `CfPreview`/summary helpers if any
  live here).
- **freecell-engine (headless, the bulk):**
  - conversions: `CfRuleSpec↔CfRuleInput`, `CfFormat↔Dxf`, `Rgb↔Color`, `CfRule→CfRuleView`
    (incl. deferred-family → Badge/non-editable).
  - `WorkbookDocument`: add→list reflects it (index/range/priority/summary); update merges dxf;
    delete; raise/lower reorders; `extended_render_style` reflects a matching rule (e.g. a
    "greater than 100" rule yields the fill on a >100 cell, base style on a ≤100 cell); a color scale
    interpolates; **value change re-evaluates** (edit a cell so it crosses the threshold → extended
    style flips); `has_cond_fmt` gate.
  - worker seam: `AddCondFmt`→`cond_fmt_rules` reflects it + `StyleCacheUpdated`/`CondFmtUpdated`
    emitted; undo/redo restores; a value edit on a CF sheet refreshes the style cache.
  - xlsx round-trip: author a highlight + a color-scale rule → save → reopen → rules + effective
    styles survive (Excel-modeled subset).
- **freecell-app (gpui view tests, no pixels):** button toggles sidebar; list renders rows
  (summary/range/preview/controls); delete + reorder send the right `Command`; editor validation
  blocks bad input and Save sends `AddCondFmt`/`UpdateCondFmt`; deferred-family row is non-editable;
  sheet switch refreshes; sidebar does **not** close on selection change; chart-panel refactor
  unchanged.
- **Render suite (in-scope — dedicated late phase):** new baselines for a CF **highlight** scene and
  a **color-scale** scene rendered over the real GridView; subset (`render_tests.sh test cond_` /
  `cf_`) while building, full suite + CI `render` gate once at the end; eyeball + commit baselines.
- **Perf:** benchmark edit→repaint + scroll-with-CF on a CF-heavy sheet vs a no-CF baseline (repo
  benchmark conventions: foreground, force+assert, p50/p99).

## 8. Two-phase decision

This is a **large** project (engine wrapper + protocol + value-dependent cache + a multi-mode
sidebar). It uses **architecture.md + two component docs** (`engine_cf.md`, `cf_sidebar.md`). Both
carry the same depth bar; the split is organizational — the coding agents execute a fully-specified
plan, they do not design.
