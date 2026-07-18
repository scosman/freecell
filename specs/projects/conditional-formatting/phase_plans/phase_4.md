---
status: complete
---

# Phase 4: Reusable sidebar container + action-bar button + empty CF sidebar shell

## Overview

First UI phase. Extract the chart edit panel's hand-rolled docked card into a reusable
`docked_sidebar` container (+ shared `section`/`section_label`/`close_button` helpers), refactor the
chart panel onto it with no visual change, then add the conditional-formatting action-bar button,
`ChromeView` CF state (open/close/toggle/refresh + chart↔CF mutual exclusion + selection-change
exemption), a minimal List-mode sidebar shell, and the `CondFmtUpdated` window refresh. No rule rows
(P5) and no rule editor (P6) yet — this phase only proves the sidebar opens/closes and docks.

Per `CLAUDE.md` render-test scope: the chart panel, the CF sidebar, and the action row have **no
pixel baselines**, so this phase does **not** run the pixel suite. Validation = gpui view tests +
an Xvfb smoke launch.

## Design decisions

- **Editor state deferred to P6.** `CondFmtPanel { sheet, rows }` only — no `editor` field and no
  seeded input entities this phase. The phase prompt explicitly permits deferring the editor input
  entities "to P6 if cleaner"; carrying an `Option<CfEditorState>` whose `CfEditorState` is never
  constructed in P4 would be dead code (fails `clippy -D warnings`). List mode is the only mode.
  P6 introduces `editor` + `CfEditorState` + the seeded inputs + the editor body.
- **Sheet-change refresh lives in the sheet-switch methods, not `on_selection_changed`.** In this
  codebase `on_selection_changed` never changes `active_sheet` (it only closes the chart panel as a
  click-away). Sheet switches funnel through `select_sheet` (tab click) and `adopt_active_sheet`
  (window-driven), which set `active_sheet`; the CF re-scope hooks there (`cf_sidebar.md §9`:
  "wherever the active sheet changes"). `on_selection_changed` is left as the selection-change
  exemption (CF stays open, untouched).

## Steps

1. **New `chrome/sidebar.rs`** (`pub(crate)` helpers):
   - `const SIDEBAR_W: f32 = 268.0` (the old `CHART_PANEL_W`).
   - `fn close_button(id, on_click) -> Button` (moved verbatim from `view.rs`).
   - `fn section_label(text: impl Into<SharedString>) -> impl IntoElement` and
     `fn section(label: impl Into<SharedString>, body: impl IntoElement) -> impl IntoElement`
     (promoted from the chart-panel local closures; `section` calls `section_label`).
   - `fn docked_sidebar(id: &'static str, title: impl Into<SharedString>, on_close: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static, body: impl IntoElement) -> impl IntoElement`
     — renders exactly today's card: `absolute` / `top(ACTION_ROW_H+DATA_ROW_H)` / `right_0` /
     `bottom(TAB_BAR_H)` / `w(SIDEBAR_W)` / `occlude` / `flex_col` / `overflow_hidden` /
     `bg(ACTIVE_TAB_BG)` / `border_l_1` / `border_color(HAIRLINE)` / `shadow_md`; pinned header
     (`flex_shrink_0`, `px_3 pt_3 pb_2`, `justify_between` title + `close_button`); scrolling body
     (`id="{id}-body"`, `flex_1 min_h_0 overflow_y_scroll px_3 pb_3`) wrapping `body`.
     Debug selectors `{id}-card` / `{id}-body` / `{id}-close` (so `chart-panel` reproduces the
     current selectors exactly).
   - Imports the layout/color consts from `super::view`.
2. **`chrome/view.rs` consts:** make `HAIRLINE`, `ACTIVE_TAB_BG`, `TEXT`, `MUTED_TEXT`,
   `ACTION_ROW_H`, `DATA_ROW_H`, `TAB_BAR_H` `pub(crate)`; delete `CHART_PANEL_W` and the local
   `close_button` fn; add `use super::sidebar::{close_button, docked_sidebar, section};` and
   `use super::cond_fmt::CondFmtPanel;`.
3. **New `chrome/cond_fmt.rs`:** `pub(crate) struct CondFmtPanel { pub sheet: SheetId, pub rows: Vec<CfRuleView> }`.
4. **`chrome/mod.rs`:** add `mod cond_fmt;` and `mod sidebar;`.
5. **Refactor `render_chart_panel`** to build `sections: Vec<AnyElement>` (unchanged content), wrap
   them in `div().flex().flex_col().gap_3().children(sections)` as `body`, and return
   `docked_sidebar("chart-panel", "Edit chart", close→close_chart_panel, body)`. Behavior-preserving.
6. **`ChromeView` CF state + methods:** field `cond_fmt: Option<CondFmtPanel>` (init `None`); methods
   `cond_fmt_open()`, `cond_fmt_sheet() -> Option<SheetId>`, `toggle_cond_fmt_sidebar(cx)`,
   `open_cond_fmt(cx)` (closes chart panel, builds panel from `client.cond_fmt_rules(active_sheet)`),
   `close_cond_fmt(cx)`, `refresh_cond_fmt(cx)` (rebuild rows for the panel's sheet),
   `rescope_cond_fmt_if_open(cx)` (re-point to `active_sheet` + rebuild). Mutual exclusion:
   `open_chart_panel` closes CF first. Degrade block clears `cond_fmt`. `select_sheet` +
   `adopt_active_sheet` call `rescope_cond_fmt_if_open`. `on_selection_changed` gets an exemption
   comment (CF deliberately NOT closed).
7. **Action-bar button** in `render_action_row` next to the chart button: ghost/small
   `Button::new("cond-fmt").icon(Icon::empty().path("icons/split.svg")).tooltip("Conditional formatting").disabled(disabled).selected(self.cond_fmt_open()).on_click(→ toggle_cond_fmt_sidebar)`.
8. **`render_cond_fmt_sidebar(cx)`** — List-mode shell: intro line naming the sheet + a primary
   `+ Add rule` button (no-op click, TODO(P6)); wrapped in `docked_sidebar("cond-fmt", "Conditional
   formatting", close→close_cond_fmt, body)`. Pushed in `render_overlays` right after the chart panel.
9. **`chrome/client.rs`:** add `fn cond_fmt_rules(&self, sheet) -> Vec<CfRuleView>` to `ChromeClient`;
   impl on `DocumentClient` (delegates) + `RecordingClient` (injected map + `set_cond_fmt_rules`).
10. **`shell/window.rs`:** `WorkerEvent::CondFmtUpdated { sheet }` → if `chrome.cond_fmt_sheet() ==
    Some(sheet)`, `chrome.update(refresh_cond_fmt)`.
11. **Icon:** lucide `split` is not in the gpui-component bundle → vendor
    `assets/icons/split.svg` (24×24, `fill="none"`, `stroke="currentColor"`, sw 2, round caps) and
    register in `shell/assets.rs` `FREECELL_ICONS`.

## Tests (gpui view tests, `chrome/view.rs`)

- `cond_fmt_button_toggles_sidebar` — toggle opens (`cond_fmt_open()` true), toggle again closes.
- `opening_cond_fmt_closes_chart_panel` — open chart panel, toggle CF → chart panel closed, CF open.
- `opening_chart_panel_closes_cond_fmt` — open CF, open chart panel → CF closed, chart panel open.
- `selection_change_does_not_close_cond_fmt` — open CF, `on_selection_changed` → CF still open.
- `sheet_switch_rescopes_cond_fmt` — inject rules for sheet 1, open CF on sheet 0,
  `adopt_active_sheet(1)` → panel re-points to sheet 1 with sheet 1's rows.
- `cond_fmt_updated_refreshes_rows` — open CF (empty), inject rules, `refresh_cond_fmt` → rows appear.
- `degrade_closes_cond_fmt` — open CF, `set_degraded(true)` → CF closed.
- Existing chart-panel view tests must still pass (container extraction is behavior-preserving).
- `shell/assets.rs` `vendored_icons_load_and_are_tintable` automatically covers `split.svg`.

The window `CondFmtUpdated` handler is a trivial gated call to the view-tested `refresh_cond_fmt`
(no `test-support` cond_fmt injection exists on `DocumentClient`, so it is covered by the view-level
refresh test + review rather than a window test).
