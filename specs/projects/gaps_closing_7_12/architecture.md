---
status: complete
---

# Architecture: gaps_closing_7_12

Technical design for the 8-item v0.5 batch. Each feature is an independent phase; this doc
gives the concrete build path (files, seams, engine APIs) and consolidates the open
decisions. Line numbers are from `HEAD` on `claude/gaps-closing-v0.5-roadmap-se4la1` and
may drift — treat them as anchors, re-grep before editing.

## 0. Shared plumbing (the seams every phase rides)

- **Command pipeline (mutations & queries):**
  `GridKeyCommand`/`GridEvent` (`grid/input.rs`, `grid/mod.rs`) → grid emits
  (`grid/view.rs`) → window/`ClipboardCoordinator` forwards (`shell/…`) →
  `Command` (`freecell-engine/src/worker/protocol.rs`) → `run.rs` batch split
  (`process_batch`, ~L507) → dispatch arm (~L855) → typed method on
  `WorkbookDocument` (`freecell-engine/src/document.rs`) → IronCalc `UserModel`
  (raw handle `user_model_mut()` `document.rs:1188`). Replies come back as
  `WorkerEvent` (`protocol.rs`). Engine mutations ride IronCalc's undo history, so they
  are **undoable for free** (one entry per op).
- **Chrome regions:** assembled in `ChromeView::render` (`chrome/view.rs:2764`) as fixed-
  height rows (`ACTION_ROW_H` 36, `DATA_ROW_H` 32, `TAB_BAR_H` 30) with the grid body as
  the `flex_1().min_h_0()` remainder. `render_tab_bar` (`chrome/view.rs:3650`) is the
  right place for the status-stats work (Phase 1).
- **Context menus** are **custom `div` popovers on the grid** (not gpui-component
  `Menu`): `chart_menu_elements` (`grid/view.rs:3141`) is the minimal template —
  `.absolute().left().top().occlude()` card + items with `.on_mouse_down(Left, …)` that
  `events.emit(GridEvent::…)` + a deferred full-grid backdrop; attached via an
  `extend(...)` in the grid root render (~L4167).
- **Selection:** pure `SelectionModel` + `Motion` + `apply_motion` in
  `freecell-core/src/selection.rs`; driven by `GridView::move_active`
  (`grid/view.rs:2179`) with `dims = sheet_dims()` (the **full** sheet — no occupancy).
- **Publication** (`freecell-core/src/publication.rs`): `PublishedCell` carries only
  `display_text` (formatted string, **no raw f64**) and covers only the overscanned
  viewport (`MAX_PUBLISH` 512×256). ⇒ Any feature needing values/occupancy **beyond the
  viewport** must go to the worker (Phases 1 & 4).

---

## 1. Status bar with selection stats

**Data path (worker aggregate — required for correctness on off-viewport selections):**
- New query `Command::SelectionStats { sheet, range, req_id }` (`protocol.rs`) →
  `run.rs` handler computes over `range ∩ used-range`, walking **populated cells only**
  (pattern: `find_matches` `document.rs:867-890`; used rectangle via
  `worksheet().dimension()` as `clamp_to_used` does, `document.rs:842-847`). A new
  `document.rs` method `selection_stats(sheet, range) -> SelectionStats` returns
  `{ count, numeric_count, sum, avg, min, max }` (`Option`/flags where N/A). Reply
  `WorkerEvent::SelectionStats { req_id, … }`.
- **Statistic rules:** `count` = non-empty cells; `sum/avg/min/max` over numeric cells;
  **errors counted in `count`, excluded from math (D1.1, recommended)**.
- **Debounce:** issue the query from `ChromeView::on_selection_changed`
  (`chrome/view.rs:663`) behind a short debounce (drag-select fires many changes); skip
  when the selection is a single cell or has no populated cells.

**Render (owner-decided placement — right of the tab bar, no new row):**
- Refactor `render_tab_bar` (`chrome/view.rs:3650`) into: a left group (existing sheet
  tabs) + a right-aligned **stats group** in the same `.h(px(TAB_BAR_H))` row (use
  `justify_between` / a spacer). Stats show `Sum · Average · Count` when present, and
  `Min · Max` too when the session toggle is on.
- **State on `ChromeView`:** `selection_stats: Option<SelectionStats>` +
  `stats_show_minmax: bool`. Clicking the stats group flips `stats_show_minmax` (session-
  only). Recompute display on each `SelectionStats` event.
- **Readout formatting:** a small pure helper (`freecell-core`, testable) formats `f64` →
  compact General (thousands separators, trimmed trailing zeros, capped sig-digits).

**Tests:** unit-test `selection_stats` (numeric/text/blank/error mix, full-column range,
empty) + the formatter; a gpui view test that the tab bar renders the readout for a
multi-cell numeric selection and hides it for a single/all-text selection. No pixel
baseline (tab-bar chrome is out of pixel-suite scope). Smoke-launch under Xvfb.

**Files:** `protocol.rs`, `worker/run.rs`, `document.rs`, `chrome/view.rs`,
`chrome/client.rs` (request helper), `freecell-core` (stats struct + formatter).

---

## 2. Cell-area right-click context menu

- Add `CellMenu { x, y, paste_enabled, paste_values_enabled, insert/delete-blocked flags }`
  + `cell_menu: Option<CellMenu>` field (mirror `HeaderMenu` `grid/view.rs:113`, which
  already carries `insert_before_blocked`/`delete_blocked`).
- Build `cell_menu_elements(&self, menu, cx)` cloned from `chart_menu_elements`
  (`grid/view.rs:3141`); items emit **existing** `GridEvent`s: `Copy{cut:false}`,
  `Copy{cut:true}`, `Paste`, **`PasteValues`** (Phase 5), `ClearCells`, and the header
  menu's `InsertRows/DeleteRows/InsertColumns/DeleteColumns` scoped to the selection's
  span (reuse its **merge-displacement guard** to compute the blocked flags). `close_cell_menu`
  + an `extend(...)` line in the grid root render (~L4167).
- **Open it from the cell-body `_ =>` arm of `handle_right_mouse_down`
  (`grid/view.rs:1742-1748`)** (today it only dismisses): first adjust the selection —
  **move to the clicked cell if it's outside the current selection, keep it if inside** —
  then set `cell_menu = Some(CellMenu{ local_x, local_y, … })` (coords already in scope).
- **Clear Formatting (D2.1):** include only if a style-clear op already exists; `ClearCells`
  clears **values**. If no clear-formatting `GridEvent`/`Command` exists, **omit** it this
  batch (don't add engine surface for it here).

**Tests:** gpui view tests — right-click outside vs inside selection (selection move/keep),
menu item enable/disable (empty clipboard disables Paste/Paste-Values), a chosen item emits
the right `GridEvent`. No pixel baseline.

**Files:** `grid/view.rs` (struct/field/builder/handler/close/extend), `grid/mod.rs`
(only if a `GridEvent::PasteValues` is added — shared with Phase 5).

---

## 3. Fill down / right (⌘D / ⌘R)

- `GridKeyCommand::FillDown/FillRight` (`grid/input.rs`) bound on `secondary && !shift`
  for `D`/`R` (alongside Copy/Cut/Paste at L67-75). Emit `GridEvent::FillDown/FillRight`
  → `Command::FillDown/FillRight { sheet, range }` (`protocol.rs`) → **edit batch** arm in
  `run.rs` (with `ClearCells`/`SetColumnWidths`, not the clipboard bucket) →
  `document.rs::fill_down/fill_right(range)`.
- `document.rs` calls the fork's **existing** `UserModel::auto_fill_rows/auto_fill_columns`
  with seed = the selection's **top row / left column** and target = the rest. A single-
  row/column seed has no series → it **copies** (Excel ⌘D/⌘R semantics); undoable via
  history.
- **Single-cell case (D3.1, recommended include):** ⌘D seeds from the cell **above**
  (`range = rows (r-1..=r), col c`), ⌘R from the **left**; no-op at row 0 / col 0.
- **Fork note:** `auto_fill_*` lives in the fork (git dep, not checked out on this box).
  The implementer runs `add_repo scosman/ironcalc` to read the **exact signature**
  (source-range+target vs. range+direction) and bind the `document.rs` wrapper. **No new
  fork fix** — the API already exists (per the 2026-07-04 audit); if it turns out missing/
  different, that becomes its own `fix/<slug>` branch per `CLAUDE.md` (would push Phase 3
  to fork work — flag early).

**Tests:** engine test — ⌘D over `A1:A5` (`A1=1`) yields `1,1,1,1,1` (copy, not series);
formula relative-adjust; single-cell pull-from-above; merge-guard interaction; one undo
step.

**Files:** `grid/input.rs`, `grid/mod.rs`, `grid/view.rs`, `shell/…` (forward),
`protocol.rs`, `worker/run.rs`, `document.rs`.

---

## 4. ⌘+arrow → edge-of-data  (decision **D4.1**)

The `edge()` resolver (`selection.rs:182-189`) ignores contents; `JumpEdge`/`ExtendEdge`
route through `apply_motion` (L205-209) with full-sheet `dims`. Occupancy lives only in
the engine.

- **Option A — worker-resolved (RECOMMENDED).** New `Command::ResolveEdge { sheet, from,
  dir, extend, req_id }` → `run.rs` walks the from-cell's row/column over **populated
  cells** applying the exact Excel algorithm (empty-start → next non-empty; non-empty +
  adjacent non-empty → last of run; non-empty + adjacent empty → across the gap; else
  sheet edge), via a pure resolver fed by engine cell reads (`cell_content`
  `document.rs:337`, or a populated-cell walk). Reply `WorkerEvent::EdgeResolved
  { req_id, target }`; the grid applies the resulting `SelectionModel` (collapse for
  `JumpEdge`, keep-anchor for `ExtendEdge`). **Trade-off:** ⌘+arrow becomes an async
  round-trip — imperceptible on typical sheets, can lag only when the worker is mid-recalc
  on a huge sheet. Smallest change; exactly correct; rides existing async plumbing.
- **Option B — published occupancy index.** Publish a compact per-sheet occupancy
  structure (e.g. per-column sorted occupied rows / per-row sorted occupied cols) each
  eval; resolve **synchronously** on the UI thread in `apply_motion`. Instant + correct,
  but adds a published payload + maintenance. Larger.

**Recommendation:** ship **Option A** for this batch (one-phase, correct, low code);
upgrade to B later only if the async feel is a real problem.

- **Wiring:** in `grid/view.rs`/`input.rs`, route only `JumpEdge`/`ExtendEdge` to the new
  async query (other motions stay synchronous through `apply_motion`). Implement the pure
  edge algorithm in `freecell-core` (testable in isolation) and feed it engine reads in
  `run.rs`.

**Tests:** exhaustive unit tests of the pure algorithm (all four start/adjacent
combinations, gaps, sheet-edge fallback, empty sheet) with a mock occupancy probe.

**Files:** `freecell-core/selection.rs` (pure edge-of-data resolver), `protocol.rs`,
`worker/run.rs`, `document.rs`, `grid/view.rs`, `grid/input.rs`.

---

## 5. Paste values (⌘⇧V)  (decision **D5.2**)

`Shift+V` is reserved-but-unbound (`grid/input.rs:64-66`). No paste-special exists; only
full-fidelity `PasteInternal` (`paste_from_clipboard`, values+formulas+styles) and
`PasteTsv` (each token re-parsed as user input — so `"=A1"` becomes a formula again).

- Bind `secondary && shift` `V` → `GridKeyCommand::PasteValues` →
  `GridEvent::PasteValues` → `Command::PasteValues { sheet, target }`.
- **Mechanism — reuse the internal clipboard's computed-value TSV (RECOMMENDED, FreeCell-
  side).** At copy time, `copy_range` (`document.rs:1063-1091`) already pulls the IronCalc
  clipboard `csv` (computed values). Retain it on the `ClipboardSlot` (`run.rs:108-113`).
  `apply_paste_values` pastes that TSV at `target` via the existing `paste_tsv` path
  (`document.rs:1148-1166`) → values only (TSV carries no formats; formulas already
  collapsed to values). One undo step; reuses tiling/overflow rules.
  - **Edge case:** a computed **string** value that begins with `"="` would re-parse as a
    formula — detect and force-literal (prefix or a literal-write path). Rare; test it.
  - **Type-fidelity caveat:** dates/booleans round-trip through their formatted string
    (re-parsed on paste), so a date may land as a number/text depending on the parser.
    Acceptable for a v0.5 "minimum" paste-values; the exact-fidelity alternative is a fork
    `paste_values` op (Option B — **out of this batch's no-fork bar**).
- **Menu parity:** the same `GridEvent::PasteValues` powers the context-menu item (Phase 2).

**Tests:** paste a copied formula cell → literal value lands, target keeps its own format;
`"=x"` string edge case; size/overflow parity with normal paste; one undo step.

**Files:** `grid/input.rs`, `grid/mod.rs`, `grid/view.rs`, `protocol.rs`,
`worker/run.rs` (retain `csv` on the slot + `apply_paste_values`), `document.rs`.

---

## 6. Number-format preset breadth  (decisions **D6.1, D6.2**)

Engine renders arbitrary codes; this is UI-only. Today: flat `DROPDOWN_FORMATS`
(`freecell-core/format_ui.rs:42`) → flat `render_num_fmt_popover` (`chrome/view.rs:4083`)
→ `apply_num_fmt` → `apply_style_path(StylePath::NumFmt, code)` (`chrome/view.rs:1426`).

- **Preset model:** replace the flat const with a **grouped** model in `format_ui.rs`
  (sections/submenus: Number, Currency▸, Date▸, Time▸, More▸, Text) carrying `(label,
  code)` pairs (inventory = **D6.1**, proposal in the functional spec). Extend the reverse
  map (`num_fmt_category` / active-format highlighting) so an active cell's code selects
  the matching preset.
- **Popover:** restructure `render_num_fmt_popover` to render the groups (nested
  submenus or labeled sections). `apply_num_fmt` is unchanged.
- **Thousands-separator toggle (D6.2):** new action-bar button beside the decimals ±
  buttons (`chrome/view.rs:3096-3117`) calling a pure `toggle_thousands(code) ->
  Option<String>` helper in `format_ui.rs` (sibling of `adjust_decimals_cell`) →
  `apply_num_fmt`; enable/disable like `decimals_enabled`.

**Tests:** unit-test the preset table + `toggle_thousands` + extended reverse map; a gpui
test that selecting a preset routes the right code to `apply_style_path`. No pixel baseline
(dropdown chrome; value display is engine-rendered — add a subset render check in Phase 8
only if a committed baseline adopts a new preset).

**Files:** `freecell-core/format_ui.rs`, `chrome/view.rs`.

---

## 7. Autofit column width  (decisions **D7.1, D7.2, D7.3**)

Drag-resize exists (`resize_hotspots` `grid/view.rs:2931-2986` → `begin/commit_resize` →
`GridEvent::ResizeCommitted` → `Command::SetColumnWidths`). Hotspots bind only
`on_mouse_down` — **no double-click handler yet**.

- Add double-click detection on the **column** resize hotspots (a `.on_click` reading
  `event.down.click_count == 2`, or a manual double-click timer) → `autofit_column(index)`.
- `autofit_column`: measure the column's widest cell via **`measure_incell_text_width`**
  (`grid/view.rs:3766-3803`, exact `shape_line().width()`, honors font/bold) at each
  cell's own font; `width = max + padding`; clamp to `[floor, cap]`; emit the **existing**
  `GridEvent::ResizeCommitted` → `Command::SetColumnWidths` (reuse ⇒ one undo step +
  xlsx round-trip; **no new worker command**).
- **Measurement scope (D7.3, RECOMMENDED = published/overscan cells only):** measure the
  cells already materialized for that column (render-thread, reuses the
  `measure_and_emit_autogrow` pattern `grid/view.rs:~2825`). **Caveat:** a wide value
  scrolled beyond the overscan isn't measured. The correct-but-heavier alternative is a
  worker text query for the column's used-range strings; deferred.
- **D7.1 multi-column:** if the double-clicked column is inside a multi-column selection,
  autofit each selected column (reuse `resize_run_for`).
- **D7.2 row-height autofit:** symmetric (row hotspots + `measure_wrap_height`
  `grid/view.rs:2771`). **Recommended: defer** — keep this phase column-only (row auto-grow
  for wrap already exists).

**Tests:** unit-test the width computation (max + padding + clamps, empty column → floor);
a subset render check (`render_tests.sh test <col-resize prefix>`) since column geometry is
lightly pixel-adjacent; existing resize baselines cover the geometry.

**Files:** `grid/view.rs` (double-click on hotspots + `autofit_column` + measurement).

---

## 8. Render-fidelity polish pair  (the dedicated render phase — LAST)

**8a — fill covers interior gridlines.** `cell_element` (`grid/view.rs:3431-3468`) paints
`.bg(fill)` then always `.border_r_1().border_b_1().border_color(GRIDLINE)`. Fix: at the
cell loop (L2334-2367) resolve each visible cell's **right** and **bottom** neighbor fill
(fills resolve at L2242-2245) and pass "skip right / skip bottom gridline" flags into
`cell_element` when the neighbor shares the **same resolved fill**. Block boundary at the
viewport edge → treat the off-viewport neighbor as different (draw the gridline);
acceptable. Explicit **borders** (separate later pass, L2372+) are unaffected.

**8b — full-row header darkening: INVESTIGATE FIRST.** The committed code computes
`selected` **identically** for the column strip (L2596) and the row strip (L2633), both
feeding `header_element` (L3633-3658). So the source looks symmetric and the GAPS
observation may be render ordering (the row accent edge L2643-2651 overpainting the tint)
or a **stale baseline**. **Step 1: eyeball the current `header_full_row_selected` baseline**
(`render-tests/src/cases.rs:1262-1276`). If the row header is already darkened → no code
fix; refresh the baseline if needed + correct the GAPS entry. Else fix the real cause
(ordering) found there.

**Render validation (this phase only):** per `CLAUDE.md`, after all other coding —
regenerate + **eyeball** affected baselines (`cell_fill_covers_gridlines`,
`header_full_row_selected`, plus any block-fill / full-line-selection cases), commit them,
run the **full** suite under a ~10-min watchdog, then dispatch the CI `render` gate
(`render.yml`) and confirm green.

**Files:** `grid/view.rs` (cell loop + `cell_element` + header strips), `render-tests`
baselines.

---

## 9. Sum-section refinements + horizontal scroller control  (owner feedback; decisions **D9.1–D9.3**)

Built **last** (after Phase 8). Two parts; **all chrome → no pixel suite** (gpui view tests +
`VisualTestContext` paint tests + Xvfb smoke). Phase 1 refactored `render_tab_bar`, so
**grep for current symbols** rather than trusting pre-Phase-1 line numbers.

**9A — stats readout refinements (Phase 1 code):**
- **Adaptive decimals (D9.1).** In `freecell-core/src/stats.rs`, extend `format_stat_value`
  (the pure formatter added in Phase 1) to pick the decimal count from `|value|`: `≥100`→2,
  `≥10`→3, `≥1`→4, `<1`→5. Keep the existing thousands-grouping + trailing-zero trim; leave
  `format_stat_count` (integer) unchanged. **Pure function → exhaustive unit tests** (tier
  boundaries at 0.9999/1/9.999/10/99.99/100, negatives, zero, `1000000.6666` → `1000000.67`).
- **Vertical centering + leading divider (D-none; styling).** In `chrome/view.rs`'s
  refactored tab-bar renderer, give the stats readout `line_height`/`h_full` + centered items
  so it centers in `TAB_BAR_H`, and add a divider element **before** the stats group reusing
  the action bar's divider style (grep the action-row renderer for the existing divider
  element; factor a shared helper if cheap).
- **Always-visible** is delivered by 9B (stats group is the static right section).

**9B — horizontal scroller control (new reusable widget):**
- **New module**, e.g. `chrome/h_scroller.rs` (or a `ui/` sibling) exposing a reusable render
  helper: `h_scroller(content, id, state) -> impl IntoElement` that wraps a horizontally
  scrollable content region and, **only when it overflows**, appends the static chevron
  section. Overflow + scroll offset use gpui's scroll-handle machinery (grep the codebase for
  the existing vertical grid scroll / any `ScrollHandle`/`overflow_x_scroll` usage to reuse
  the same primitive); **hide the scrollbar** (no visible track).
- **Chevron section (D9.3):** a static (non-scrolling) trailing group = divider + two buttons
  with lucide `chevron-left`/`chevron-right`, styled like the action-bar buttons/divider
  (reuse those element builders). Find the existing icon API (grep for the current lucide/icon
  usage — `IconName`/icon render in gpui-component) and use it; **add the CLAUDE.md note (9C)
  that we use lucide**.
- **Scroll step (D9.2):** each chevron click animates the scroll offset by **0.8 ×
  viewport_width** in its direction, clamped to `[0, max_scroll]`; disable a chevron at its
  limit. Use the codebase's animation/`with_animation` idiom if present, else an eased offset
  tween; a non-animated clamp is an acceptable fallback if animation plumbing is heavy (note
  it if so).
- **Call site 1 — action bar:** wrap the action-row button groups in `h_scroller` so they
  scroll on small windows. Fits → byte-identical to today.
- **Call site 2 — sheet-tab strip:** wrap the tabs in `h_scroller`; render the **stats group +
  its leading divider as the static right content OUTSIDE the scroller** (so it never
  scrolls / never gets pushed off). This is what implements 9A.4.
- **Tests:** pure/gpui tests for overflow-detection (fits → no chevron section; overflows →
  chevron section present), chevron-click moves the offset by 0.8×width and clamps at both
  ends, disabled-at-limit, and both call sites host their content; a `VisualTestContext` paint
  of the overflow state. No pixel suite.

**Files:** `freecell-core/src/stats.rs`, `chrome/view.rs` (tab-bar + action-row renderers),
new `chrome/h_scroller.rs` (or `ui/`), `CLAUDE.md` (lucide note).

---

## 10. Feedback tweaks  (owner feedback; decision **D10.1**)

Built after Phase 9. Chrome-only → no pixel suite (gpui view tests + `VisualTestContext` paint
+ Xvfb smoke). Open-ended phase; 10.1 is the first tweak.

**10.1 — number-format dropdown: basics-first + "More ▸" submenu.** Undo Phase 6's
scroll-to-reach-basics regression. In `chrome/view.rs`'s `render_num_fmt_popover` (grep — Phase
6 restructured it):
- **Recover the original 7-preset list** from git history — the pre-Phase-6 `DROPDOWN_FORMATS`
  const removed in commit `382f075` (`git show 382f075^:app/crates/freecell-core/src/format_ui.rs`).
  Reintroduce it in `freecell-core/src/format_ui.rs` as the **basic set** (e.g. `BASIC_FORMATS`),
  keeping the Phase-6 `NUM_FMT_GROUPS` intact as the full/"More" inventory (single source of
  truth for breadth). Both feed the same reverse map (`num_fmt_category`) and the same
  `apply_num_fmt` path.
- **Popover** renders the basic set flat (no scroll) + a trailing **"More ▸"** row.
- **D10.1 — submenu mechanism:** prefer a **flyout** (a second popover card anchored to the
  "More ▸" row, reusing the existing custom-`div` popover/backdrop pattern — like the
  chart/cell menus). **Fallback: drill-in** (clicking "More ▸" swaps the popover's content to
  the grouped `NUM_FMT_GROUPS` list — the current Phase-6 scrollable grouped view — with a
  "◂ Back" row restoring the basic set); use drill-in if a hover/click flyout is awkward with
  the popover's occlude/dismiss machinery. Note the choice in the phase plan.
- **Highlighting:** the basic menu highlights the active preset when it's a basic one; when the
  active format is a "More"-only preset, mark "More ▸" active (and, for drill-in, have it open
  onto the matched group) so the user can still see/reach the current format. The
  thousands-separator action-bar button is untouched.
- **Tests:** basic menu shows the 7 without scroll; "More ▸" opens the full grouped list;
  selecting from either level applies the right code; reverse-map highlight for a basic vs. a
  More-only active format; a `VisualTestContext` paint of both levels.

**Files:** `freecell-core/src/format_ui.rs` (reintroduce basic set), `chrome/view.rs`
(`render_num_fmt_popover` + More/back state).

**10.2 — action-bar scroller triggers too early (bug fix).** Root cause: `render_action_row`'s
button-group `div` carries `.min_w(px(ACTION_ROW_MIN_W))` with `ACTION_ROW_MIN_W = 1152.0`
(`chrome/view.rs` — a hand-estimated, self-documented-as-drift-prone constant). When 1152 >
the true natural button width, the slack is trailing empty space inside the scroll content, so
`h_scroller`'s `max_offset().x > 0` overflow check fires early. **Fix:** drop the `min_w` +
`ACTION_ROW_MIN_W` and put `.flex_shrink_0()` on the button-group content (default flex shrink
= 1 was the only reason min_w was needed against compression). Content then sits at exact
natural width → chevrons only when it genuinely overflows. Fixed-width children (font-family
`w(140)`, font-size `w(56)`) lay out unchanged. Test: no overflow at a wide-enough viewport.

**10.3 — animate the chevron scroll (D10.2).** `ScrollHandle::set_offset` is instantaneous;
gpui's `window.request_animation_frame()` notifies the current view each frame and needs only
`&mut Window` (the chevron `on_click` has it). Approach: add `target: Rc<Cell<Option<f32>>>` to
`HScroller`; chevron click computes the clamped destination via the existing `scroll_step` and
stores it in `target` + requests a frame; `h_scroller()` takes a `&mut Window` param and, while
`target` is `Some`, lerps `offset` toward it each frame (e.g. `cur + (t-cur)*STEP`, snap within
~0.5px), `set_offset`s, re-requests a frame until arrived, then clears `target` (drives frames
only while animating → never fights wheel/trackpad). The per-frame step is a pure, unit-tested
fn like `scroll_step`. Thread `&mut Window` into `render_action_row`/`render_tab_bar` and their
two `h_scroller` calls — the sole caller `render()` already has `window` (currently `_window`).

**10.4 — flyout for "More ▸": DEFERRED to a GAP (D10.3).** gpui-component ships flyout submenus
(`crates/ui/src/menu/popup_menu.rs` `PopupMenu::submenu`), but (a) its `scrollable(true)` note
says submenus "cannot be support[ed]" while scrollable — and the num-fmt card is deliberately
scrollable; (b) the app uses **zero** gpui-component menus — all seven toolbar popovers are
hand-rolled `div().absolute()…occlude()` cards over a `backdrop()`; a single `PopupMenu` would
own separate anchoring/dismiss and diverge visually from its siblings. So a flyout done *well*
is an app-wide migration. Keep the 10.1 drill-in; file a GAP (`PROJECTS.md` +
`projects/gpui-component-menus.md`, status Future) to adopt gpui-component menus app-wide.

**Files (10.2/10.3):** `chrome/h_scroller.rs`, `chrome/view.rs` (`render_action_row`,
`render_tab_bar`, `render()` window thread). **10.4:** `PROJECTS.md`,
`projects/gpui-component-menus.md` (no code).

---

## Consolidated decisions

| ID | Decision | Recommendation |
|----|----------|----------------|
| D1.1 | Error cells in stats | Count in `count`, exclude from Sum/Avg/Min/Max |
| D1.2 | Status-bar placement | **RESOLVED (owner):** right of the sheet-tab bar, no new row |
| D2.1 | Context-menu inventory / Clear Formatting | Standard set; include Clear Formatting only if a style-clear op already exists, else omit |
| D3.1 | Single-cell ⌘D/⌘R pull-from-neighbor | Include (cheap, Excel-expected) |
| D4.1 | Edge-of-data mechanism | **Worker-resolved (Option A)**; occupancy-index (B) deferred |
| D5.1 | Paste-values keeps formats? | No — values only |
| D5.2 | Paste-values mechanism | **Computed-value TSV reuse (FreeCell-side)**; fork op deferred |
| D6.1 | Preset inventory | Proposal in functional spec §6 |
| D6.2 | Thousands-separator toggle button | Include |
| D7.1 | Multi-column autofit | Include |
| D7.2 | Row-height autofit | Defer (column-only this phase) |
| D7.3 | Autofit measurement scope | Published/overscan cells only (render-thread) |
| D9.1 | Stats adaptive decimals | **RESOLVED (owner):** by \|value\| — ≥100→2, ≥10→3, ≥1→4, <1→5 dp |
| D9.2 | H-scroller chevron step | **RESOLVED (owner):** animated, 0.8 × viewport width per click, clamped |
| D9.3 | H-scroller overflow affordance | **RESOLVED (owner):** static divider + lucide chevron-left/right (action-bar style); no visible scrollbar; unchanged when it fits |
| D10.1 | Num-fmt "More ▸" submenu mechanism | **RESOLVED:** drill-in shipped (10.1); a true flyout deferred to a GAP (see D10.3) |
| D10.2 | H-scroller animated scroll | **RESOLVED (owner):** animate — fast lerp via `request_animation_frame` (needs only `&mut Window`); replaces the D9.2 non-animated fallback |
| D10.3 | Flyout via gpui-component menus | **RESOLVED (owner):** not cheap for one popover (scrollable⊗submenu; app's only gpui-component menu) → **GAP** to adopt gpui-component menus app-wide; keep drill-in for now |

## Component designs

Not needed — each phase is small and self-contained; this doc + the functional spec are
sufficient for a coding agent. No `/components/*.md`.

## Cross-cutting testing & checks

- Per phase: crate-scoped `cargo build -p <crate>` + `cargo test -p <crate> --lib` (add
  `-p freecell-engine` when the engine is touched); **always** `cargo fmt --all --check`.
- Pixel suite: only Phase 7 (subset) and Phase 8 (full + CI gate) per the functional
  spec's render-scope table; Phases 1–6 verify with unit/gpui tests + an Xvfb smoke launch.
