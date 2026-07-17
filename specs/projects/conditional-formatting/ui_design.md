---
status: complete
---

# UI Design: Conditional Formatting

The CF UI reuses FreeCell's existing chrome (`chrome/view.rs` — the `ChromeView` action row +
right-docked panel + popovers). It adds one action-bar button and one right-docked sidebar, built
on a **reusable sidebar-container** extracted from the chart edit panel. All new surfaces live in
the `freecell-app` crate.

## 0. Reusable sidebar container (extraction)

**Source:** `render_chart_panel` (`chrome/view.rs` ~5024–5167) currently hand-rolls the docked card
(header with title + `close_button`, pinned header, scrolling body). Extract that shell into a
reusable helper so both the chart panel and the CF sidebar share it.

**Shape:** a free function in `chrome/` (e.g. `chrome/sidebar.rs`):
```
fn docked_sidebar(
    id: &'static str,
    title: impl Into<SharedString>,
    on_close: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    body: impl IntoElement,          // caller supplies the scrollable content
) -> impl IntoElement
```
It renders the identical card the chart panel uses today:
- `div().absolute().top(ACTION_ROW_H + DATA_ROW_H).right_0().bottom(TAB_BAR_H).w(SIDEBAR_W)
  .occlude().flex().flex_col().overflow_hidden().bg(ACTIVE_TAB_BG).border_l_1()
  .border_color(HAIRLINE).shadow_md()`
- **pinned header** (`flex_shrink_0`, `px_3 pt_3 pb_2`): `justify_between` title (`SEMIBOLD`,
  `text_size 12`, `TEXT`) + the shared `close_button`.
- **scrolling body** (`id`, `flex_1`, `min_h_0`, `overflow_y_scroll`, `px_3 pb_3`) hosting `body`.

`render_chart_panel` is refactored to call it (title `"Edit chart"`, its sections as the body) — a
**pure refactor, no pixel change** to the chart panel (verify: the `chart_*` baselines are not
grid/cell baselines, but the panel itself has no baseline; a gpui view test + smoke launch covers
it). Width constant: keep `CHART_PANEL_W` (268px) as the shared `SIDEBAR_W` (the CF sidebar uses
the same width for a consistent dock).

> Note: the chart panel and CF sidebar are **mutually exclusive-ish** overlays in the same dock
> position. First pass: opening one while the other is open **closes the other** (they share the
> right dock). Implemented by each `open_*` closing the sibling panel.

## 1. Action-bar button

- A new **ghost, small icon button** in the action row (`render_action_row`, `chrome/view.rs`),
  placed in the formatting cluster (after the number-format / borders group, near the chart insert
  button). Icon: lucide **`split`** (`Icon::empty().path("icons/split.svg")`), tooltip
  **"Conditional formatting"**, `selected(self.cond_fmt_open())`, `disabled(self.degraded)`.
- `on_click` calls a `ChromeView` method `toggle_cond_fmt_sidebar(cx)` that opens/closes the
  sidebar directly (no menu, unlike the chart insert button which opens a type menu).
- **Icon asset:** if lucide `split` is already in the gpui-component bundle, use its path for free;
  otherwise vendor `assets/icons/split.svg` (tintable `stroke="currentColor"`) and register it in
  `shell/assets.rs` (per CLAUDE.md icon convention). Confirm during implementation.

## 2. The Conditional Formatting sidebar

Built with `docked_sidebar("cond-fmt", "Conditional formatting", …, body)`. Two internal modes in
the body, driven by sidebar state (`CondFmtPanel`): **List mode** (default) and **Editor mode**.

### 2.1 List mode (default)
- A short intro line: the active sheet name + "rules".
- A **"+ Add rule"** primary button.
- A vertical list of **rule rows**, priority order (highest first). Each row:
  - **preview swatch** (left): for a highlight rule, a small chip painted with the rule's fill +
    a glyph/"A" in the text color (so fill and text color both read); for a color scale, a small
    horizontal **gradient** chip.
  - **summary** (middle, two lines): line 1 = human summary (e.g. *"Cell value > 100"*,
    *"Text contains 'foo'"*, *"3-color scale"*, *"Top 10 items"*); line 2 = the **range** in muted
    text (e.g. `B2:B20`).
  - **controls** (right): move-up / move-down (lucide `chevron-up` / `chevron-down`; disabled at
    the ends), **edit** (lucide `pencil`; disabled for a deferred-family rule — §functional_spec
    §9), **delete** (lucide `trash-2`).
- Empty state: a muted "No rules on this sheet yet." above the Add-rule button.
- Section-label + row styling reuse the chart panel's `section`/`section_label` closures (muted
  `SEMIBOLD` mini-labels), promoted to shared helpers if convenient.

### 2.2 Editor mode (add / edit)
A back-chevron + "New rule" / "Edit rule" title row, then the form (all fields stacked, the body
scrolls):
1. **Applies to** — a small text `Input` (A1 range), defaulting to the current selection when
   adding. Inline red error text under it when invalid.
2. **Rule type** — a dropdown (gpui-component popover/menu) grouped per functional_spec §2.3. The
   selection drives which operand + format controls show.
3. **Operands** — rendered per type:
   - *Cell value*: an operator dropdown + one value `Input` (two inputs, "and", for
     Between/Not between).
   - *Text*: an operator dropdown + a text `Input`.
   - *Dates*: a period dropdown.
   - *Top/Bottom*: a rank `Input` (number) + a **"% of range"** toggle.
   - *Above/Below average, Duplicate/Unique, Blanks/Errors*: no operand.
   - *Formula*: a formula `Input` (monospace, `=`-prefixed hint).
4. **Format** (non-color-scale types): the **format editor** (§3) with live preview.
5. **Color-scale editor** (color-scale types): the **color-scale editor** (§4) with gradient
   preview.
6. **Stop if true** — a checkbox (row with label).
7. **Save** (primary) + **Cancel** (ghost). Save is disabled while the form is invalid.

### 2.3 Interaction rules
- The sidebar **does not close on grid selection change** (opt out of the chart panel's
  `on_selection_changed → close` behavior). This lets the user pick the "Applies to" range by
  selecting cells: an **"Use selection"** affordance next to the range field sets the field to the
  current selection on demand (first pass keeps it explicit rather than auto-syncing every click).
- Closes on **×**, on the action-bar toggle, or on degrade.
- Switching sheets returns the sidebar to **List mode** for the new sheet (cancels an open editor).
- Save/Cancel in the editor returns to List mode.

## 3. Format editor (differential format)

A compact block (reused by every non-color-scale rule type):
- **Presets row:** a handful of one-click Excel-style presets rendered as small chips showing the
  look (e.g. *Light red fill / dark red text*, *Yellow fill / dark yellow text*, *Green fill /
  dark green text*, *Red text*, *Bold*). Clicking a preset fills the controls below.
- **Fill color:** a color control (reuse the existing fill-color picker pattern from the action
  row's fill popover — the same swatch grid + custom entry). "No fill" allowed.
- **Text color:** the same color control for font color.
- **Bold / Italic:** two toggle buttons (reuse the `B`/`I` icon buttons style).
- **Preview:** a sample cell ("123" / "Abc") painted with the current fill + text color + weight,
  updated live.

(Underline, strikethrough, border, number format, alignment are intentionally absent — GAPS.)

## 4. Color-scale editor

- **Scale type:** 2-color vs 3-color (a segmented control or two preset chips), with Excel default
  color presets offered as clickable chips (e.g. green-white-red, white-blue, …).
- **Stops:** 2 or 3 rows, each: a **threshold-type** dropdown (Min/Max for endpoints;
  Number/Percent/Percentile) + (for Number/Percent/Percentile) a value `Input` + a **color**
  control. The midpoint (3-color) defaults to 50th percentile.
- **Preview:** a horizontal **gradient** bar across the chosen stop colors.

## 5. State (owned by `ChromeView`)

- `cond_fmt: Option<CondFmtPanel>` — `Some` ⇒ sidebar open. Mirrors the chart panel's
  `Option<ChartPanel>`. `CondFmtPanel` holds: the active `SheetId`, the list of rule view-models
  (`Vec<CfRuleRow>`), and the editor sub-state (`None` in list mode; `Some(CfEditorState)` in
  editor mode). `CfEditorState` holds the working range, chosen type, operands, working `Dxf`/scale,
  stop-if-true, an optional edit target `index`, and validation error strings.
- Text inputs for the editor (range, operand values, formula) are `Entity<InputState>` on
  `ChromeView`, seeded when the editor opens (mirroring the chart title/axis inputs pattern).
- The window (`WorkbookWindow`) builds `CondFmtPanel`'s rule rows from the engine snapshot
  (mirroring `chart_panel_info`) and refreshes them after CF mutations + on sheet switch +
  (for value-dependent previews) not required — previews are format-only, not value-driven.

## 6. UX principles

- **Discoverable, low-chrome:** one action-bar button, one familiar right-docked sidebar. List-first
  so managing existing rules is the default; adding is one click in.
- **Convention reuse:** the docked sidebar, the color picker, the `B`/`I` toggles, the
  `close_button`, and the section labels are all the existing chrome vocabulary — no new mental
  models.
- **Honest about scope:** deferred-family rules are visible + deletable but not miseditable; the
  editor only offers what the first pass can faithfully author.
