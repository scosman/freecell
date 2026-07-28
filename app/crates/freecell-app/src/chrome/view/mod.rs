//! [`ChromeView`] — the action row, data row (formula bar), and sheet tab bar as one GPUI
//! entity (`components/app_shell.md`, `ui_design.md §3.1–3.4`).
//!
//! Thin plumbing over the Phase-2 pure logic: the [`DataRow`] reducer drives the content
//! field, the [`EvalIndicator`] drives the evaluating spinner, [`FILL_PALETTE`] the fill
//! swatches, and [`freecell_core::sheet_name::validate_sheet_name`] the inline rename. Every
//! user action is a plain method here (so it is unit-testable without pixel clicks); the widget
//! handlers just call those methods, and the reducers' effects are performed as
//! [`ChromeClient`] commands and [`ChromeGridRequest`]s.
//!
//! The fill popover, tab context menu, and delete-confirm modal are lightweight
//! `ChromeView`-owned panels (controlled by view state) rather than the stock
//! gpui-component `Popover`/`ContextMenu`/`Modal` — their content closures run in a foreign
//! entity context, which would force cross-entity dispatch for what is a functional-POC
//! surface (`ui_design.md`: "this is chrome — don't over-invest"). Buttons, the text inputs,
//! the color picker, and the spinner are stock gpui-component controls as specced.
//!
//! The implementation is split across this directory by feature domain
//! (`specs/projects/chrome-view-split`). This file owns the [`ChromeView`] struct itself, its
//! constructor, and the constants shared across domains; each child module holds one domain's
//! `impl ChromeView` methods and that domain's tests. Children are *descendants* of this
//! module, so they reach every private field here without any visibility change; items they
//! need from each other are `pub(super)`, which scopes them to this subtree exactly as
//! private-to-`view.rs` did before the split.

mod cf_editor;
mod cf_sidebar;
mod charts;
mod editing;
mod find;
mod formatting;
mod shell;
mod stats;
mod tabs;
#[cfg(test)]
mod test_support;

use cf_editor::CfMenu;
pub use charts::{ChartPanel, ChartPanelSeries};
use formatting::SYSTEM_DEFAULT_FAMILY;
use tabs::{TabDrag, TabSpan};

use std::rc::Rc;
use std::time::Duration;

use gpui::{
    canvas, div, prelude::*, px, rgb, App, ClickEvent, Context, Entity, FocusHandle, Focusable,
    Hsla, KeyDownEvent, Modifiers, MouseButton, MouseDownEvent, Rgba, SharedString, Window,
};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState};
use gpui_component::input::{Input, InputEvent, InputState, Position};
use gpui_component::spinner::Spinner;
use gpui_component::{Disableable as _, Icon, Selectable as _, Sizable as _};

use freecell_core::data_row::{DataRow, DataRowEffect, DataRowEvent, FieldMode};
use freecell_core::eval_indicator::{EvalEffect, EvalEvent, EvalIndicator};
use freecell_core::format_ui::{
    adjust_decimals_cell, displayed_decimals, font_size_display, is_more_only_num_fmt,
    num_fmt_category, toggle_thousands, Category, BASIC_FORMATS, NUM_FMT_GROUPS,
};
use freecell_core::functions;
use freecell_core::input_cap::InputRejection;
use freecell_core::palette::FILL_PALETTE;
use freecell_core::selection::{Direction, Motion};
use freecell_core::{
    effective_range, region_at, regions_intersecting, Align, CellKind, CellRange, CellRef,
    CfPreview, CfRuleView, RenderStyle, Rgb, SelectionModel, SelectionStats, SheetId, VAlign,
};

use crate::grid::caret_intent_modifiers;

use freecell_chart_model::ChartId;

use freecell_engine::{
    BorderLine, BorderPreset, ChartInsertKind, Command, EditRejectedReason, StyleAttr, StylePath,
    WorkerEvent,
};

use super::cond_fmt::CondFmtPanel;
use super::h_scroller::{h_scroller, HScroller};
use super::sidebar::{docked_sidebar, section};
use super::{
    AutocompleteDisplay, AutocompleteRow, ChromeClient, ChromeGridRequest, ChromeGridSink,
    EditController, EditOrigin, SheetTab,
};

/// The 250 ms no-flash delay for both the content-fetch and evaluating spinners
/// (`ui_design.md §3.1/§3.2`, mirrored from the grid's own delayed hooks).
const SPINNER_DELAY: Duration = Duration::from_millis(250);

/// Debounce before a selection-change fires a `SelectionStats` query — a drag-select emits many
/// selection changes, so the readout waits for the drag to settle (`architecture.md §1`).
const STATS_DEBOUNCE: Duration = Duration::from_millis(120);

// --- Chrome look constants (functional POC greys; `ui_design.md §3`) -----------------
const CHROME_BG: u32 = 0xF3F3F3;
// `HAIRLINE`, `ACTIVE_TAB_BG`, `TEXT`, `MUTED_TEXT` are `pub(crate)` so the shared docked-sidebar
// container (`chrome::sidebar`) paints the identical card + section labels.
pub(crate) const HAIRLINE: u32 = 0xD9D9D9;
const DIVIDER: u32 = 0xC8C8C8;
pub(crate) const ACTIVE_TAB_BG: u32 = 0xFFFFFF;
pub(crate) const TEXT: u32 = 0x1F1F1F;
pub(crate) const MUTED_TEXT: u32 = 0x555555;
/// Danger border/text for cap-rejected input + invalid rename (theme danger, `#DC2626`).
const DANGER: u32 = 0xDC2626;
/// Dark tooltip fill + text for the cap-error popover (`ui_design.md §4`).
const TOOLTIP_BG: u32 = 0x2B2B2B;
const TOOLTIP_TEXT: u32 = 0xF5F5F5;
/// The highlighted-row tint in the function-completion list (a light accent wash,
/// `gaps_closing_7_15 §1`).
const AUTOCOMPLETE_HL_BG: u32 = 0xE8F0FE;
/// The completion list's minimum width so argument templates fit without jitter.
const AUTOCOMPLETE_MIN_W: f32 = 300.0;
/// Accent ring around the borders popover's selected color swatch (Office Accent 1 — reads over a
/// black or white swatch, unlike a grey/dark ring; `ui_design.md §2.1`).
const SWATCH_SELECTED_RING: u32 = 0x4472C4;
/// The borders target-icon 2×2 diagram: light-grey context gridlines vs. the solid-dark affected
/// edges (`ui_design.md §2.2`). Drawn from `div` rectangles, the same primitive as the grid's edges.
const TARGET_ICON_PX: f32 = 22.0;
const TARGET_ICON_GREY: u32 = 0xC8C8C8;
const TARGET_ICON_DARK: u32 = 0x1F1F1F;

// `pub(crate)` so the shared docked-sidebar container (`chrome::sidebar`) positions the card
// between the data row and the tab bar (the right-docked width is `sidebar::SIDEBAR_W`).
pub(crate) const ACTION_ROW_H: f32 = 36.0;

/// The action-row dropdown/popover triggers whose panel anchors under the button. The buttons are
/// content-sized (their labels — font family, size, number-format category — change width), so a
/// popover's x-offset can't be a fixed constant (BUG 2c); each trigger's real laid-out left edge is
/// captured into [`ChromeView::anchor_x`] by a `canvas` probe and the panel renders at that x.
/// Discriminants are the `anchor_x` indices.
#[derive(Clone, Copy)]
enum Anchor {
    FontFamily = 0,
    FontSize = 1,
    TextColor = 2,
    Fill = 3,
    Borders = 4,
    NumFmt = 5,
    Chart = 6,
}
const ANCHOR_COUNT: usize = 7;

impl Anchor {
    fn idx(self) -> usize {
        self as usize
    }
}

// `DATA_ROW_H` / `TAB_BAR_H` are `pub(crate)` so the shared docked-sidebar container
// (`chrome::sidebar`) can dock the card between the data row and the tab bar.
pub(crate) const DATA_ROW_H: f32 = 32.0;
/// The formula-bar content entry's height: [`DATA_ROW_H`] minus 2 px breathing room above **and**
/// below (BUG C), so the row's `items_center` insets the entry within the bar without changing the
/// bar height. gpui-component's single-line `Input` otherwise renders at its fixed control height
/// (`Size::Medium` → 32 px) and fills the row edge-to-edge, which reads as cramped.
const DATA_ROW_FIELD_H: f32 = DATA_ROW_H - 4.0;
pub(crate) const TAB_BAR_H: f32 = 30.0;
const REF_BOX_W: f32 = 72.0;
/// The content field's left edge inside the data row = padding + ref box + gap + divider +
/// gap (`render_data_row` layout); the cap-error popover anchors here.
const DATA_ROW_CONTENT_LEFT: f32 = 8.0 + REF_BOX_W + 8.0 + 1.0 + 8.0;

/// The chrome around the grid: action row + data row + sheet tab bar.
pub struct ChromeView {
    client: Rc<dyn ChromeClient>,
    grid: ChromeGridSink,
    focus_handle: FocusHandle,

    /// The active sheet (mirrors the grid); commands + fetches are scoped to it.
    active_sheet: SheetId,
    /// The current selection (mirrored from the grid) — drives the ref box, toggle states,
    /// and the content fetch.
    selection: SelectionModel,
    /// The active cell's resolved style, cached at selection-change time for the toggles.
    active_style: Option<RenderStyle>,
    /// The active cell's number-format code, cached alongside `active_style` — drives the
    /// number-format dropdown's category label + the decimals ± enabled/computed state
    /// (`components/action_bar.md`). `None` on a multi-cell selection (matches `active_style`).
    active_num_fmt: Option<String>,
    /// The active cell's font-family name (`""` = the workbook default = "Default (Inter)"), cached
    /// alongside `active_style` for the family dropdown's label. `None` on a multi-cell selection.
    active_font_family: Option<String>,
    /// The active cell's evaluated kind + displayed value from the latest publication, cached
    /// alongside `active_num_fmt` — lets the decimals ± buttons enable on a *numeric* General cell
    /// (`200000`) while staying disabled on a text/date General cell (BUG 3). `None` on a multi-cell
    /// selection or an empty/off-viewport active cell.
    active_published: Option<(CellKind, String)>,
    /// The workbook's default font size in points, cached from the resident cache — the size box
    /// labels a **default** cell (`font_size_q == 0`) with this instead of a hardcoded value, so the
    /// label reflects the real default (13pt for a new workbook, the file's default otherwise —
    /// `components/action_bar.md`). `None` until a cache is resident. Workbook-global, so it is
    /// refreshed unconditionally (not gated on a single-cell selection).
    default_font_size_pt: Option<f64>,
    /// Whether the worker is degraded (read-only): every mutating action-bar control disables
    /// (`functional_spec.md §6`). Set by the window on `WorkerDegraded`.
    degraded: bool,

    /// The formula-bar state machine (`freecell-core`).
    data_row: DataRow,
    /// The content field's text buffer (stock gpui-component input).
    content_input: Entity<InputState>,
    /// The in-cell editor + cross-editor sync (`components/edit_controller.md`). Owns the reused
    /// in-cell overlay `InputState`; the data-row half is `content_input` + the `DataRow` reducer.
    edit: EditController,
    /// Whether the last edit-state push to the grid was non-empty (a mirror / overlay was shown),
    /// so an idle selection move doesn't re-push an all-`None` clear on every keystroke.
    edit_state_shown: bool,
    /// Whether the current pending edit is in **quick-edit** mode (`functional_spec.md §5`). Set by
    /// `begin_typed` (type-to-replace entry); cleared by `begin_in_cell`, by any caret-intent signal
    /// (mouse-down in the field, Home/End, a modified arrow — see [`leave_quick_edit`](Self::leave_quick_edit)),
    /// and on commit/cancel. While set + editing, an unmodified arrow commits + moves the active cell
    /// instead of the caret.
    quick_edit: bool,
    /// The `(sheet, cell)` whose fetched content currently lives in the reducer's `committed`
    /// field. The in-cell editor seeds from `committed` **only** for this exact sheet+cell — the
    /// single shared reducer keeps a previous cell's `committed` across a single→single selection
    /// change, and its content is not sheet-scoped, so seeding by `(sheet, cell)` prevents opening
    /// the editor with another cell's/sheet's stale content while the target's fetch is in flight
    /// (`components/edit_controller.md §Grid integration`; data-corruption guard). Reset to `None`
    /// whenever `committed` is cleared or invalidated (multi-select, sheet switch); `None` until the
    /// first reply lands.
    committed_cell: Option<(SheetId, CellRef)>,
    /// A worker `EditRejected{InputCap}` backstop (the UI validates first, so this is rare);
    /// carries the rejection so the popover shows the same message as a local cap reject.
    cap_error_external: Option<InputRejection>,

    /// The evaluating-spinner state machine (`freecell-core`).
    eval: EvalIndicator,

    /// The fill popover's open state (a `ChromeView`-owned panel).
    fill_open: bool,
    /// The stock color picker for the fill popover's "Custom…" entry.
    color_picker: Entity<ColorPickerState>,
    /// The text-color popover's open state (mirrors the fill popover, with "Automatic" in place
    /// of "No fill" — `components/action_bar.md`).
    text_color_open: bool,
    /// The stock color picker for the text-color popover's "Custom…" entry.
    text_color_picker: Entity<ColorPickerState>,
    /// The number-format dropdown's open state (a `ChromeView`-owned menu panel).
    num_fmt_open: bool,
    /// The number-format dropdown's **drill-in** state (`functional_spec.md §10.1`, D10.1). `false`
    /// = the basics-first view (the 7 [`BASIC_FORMATS`] flat + a trailing "More ▸" row); `true` =
    /// the full grouped [`NUM_FMT_GROUPS`] view (with a "◂ Back" row). Reset to `false` at every
    /// popover-close so the dropdown always reopens basics-first (except when it opens directly onto
    /// a More-only active format — see [`toggle_num_fmt_popover`](Self::toggle_num_fmt_popover)).
    num_fmt_more_open: bool,
    /// The chart-insert menu's open state (the action-bar chart-type glyph menu, P17). Like the
    /// other formatting popovers it closes on click-away / a type pick / degrade.
    chart_menu_open: bool,
    /// The right-docked **chart edit panel** (P19, `ui_design §4`), open while a chart is being
    /// shaped. It closes on its × button, on **click-away** (a grid click on a cell/empty area,
    /// routed through [`on_selection_changed`](Self::on_selection_changed) — post-v1 Batch 2, item
    /// 12), on the chart's deletion, or on degrade. Clicking *another* chart re-points it (a switch).
    /// The window drives open/close/refresh (`shell::window`); the panel's controls send
    /// `SetChartType` / `SetChartRange` / `SetChartChrome` for its `(sheet, id)`.
    chart_panel: Option<ChartPanel>,
    /// The chart edit-panel's text inputs (P20 chrome): title + category/value axis titles. Seeded
    /// when the panel opens for a NEW chart id (never on a live republish — so an in-progress edit
    /// isn't clobbered), committed **live per keystroke** (`Change`), with Enter/blur as redundant
    /// commit points (post-v1 Batch 2, item 6).
    chart_title_input: Entity<InputState>,
    chart_cat_axis_input: Entity<InputState>,
    chart_val_axis_input: Entity<InputState>,
    /// The panel target `(sheet, id)` captured when a chart text input **gained focus** — the
    /// staleness guard for a deferred `Blur`. If the panel re-points to a different chart between
    /// focus and the field's commit (a rapid selection switch while a field holds unsaved text), the
    /// captured key no longer matches the panel and the stale commit is dropped, so a field's text can
    /// never be sent to the wrong chart. `None` when no chart input is focused.
    chart_input_focus: Option<(SheetId, ChartId)>,
    /// The right-docked **conditional-formatting sidebar** (`components/cf_sidebar.md`), open while
    /// managing a sheet's CF rules. `Some` ⇒ open (mirrors [`chart_panel`](Self::chart_panel)); the
    /// two share the right dock and are **mutually exclusive** (opening one closes the other). Closes
    /// on its × / the action-bar toggle / degrade; does **not** close on grid selection change, and
    /// re-scopes to the new sheet on a sheet switch. P4 renders List mode only (rows P5, editor P6).
    cond_fmt: Option<CondFmtPanel>,
    /// The CF rule-editor's seeded text inputs (`components/cf_sidebar.md §3`), seeded when the
    /// editor opens (mirrors the chart title/axis inputs). Read at Save time — not live-committed:
    /// the Applies-to range, the value operand(s) (operand-1 also carries the Text value + the
    /// Top/Bottom rank), and the custom formula.
    cf_range_input: Entity<InputState>,
    cf_operand1_input: Entity<InputState>,
    cf_operand2_input: Entity<InputState>,
    cf_formula_input: Entity<InputState>,
    /// The color-scale editor's per-stop value inputs (`components/cf_sidebar.md §8`) — a fixed 3
    /// (the max stop count), of which only the active scale's stops are shown. Seeded from
    /// `CfEditorState.scale` on editor open / kind change and synced back into it on edit.
    cf_stop_value_inputs: Vec<Entity<InputState>>,
    /// Which of the editor's inline dropdowns (rule-type / operator / period) is currently expanded,
    /// or `None`. Only one opens at a time; opening the editor / saving / cancelling clears it.
    cf_menu_open: Option<CfMenu>,
    /// The installed font-family names for the family dropdown, fetched once at build
    /// (`cx.text_system().all_font_names()`), sorted-unique with "Default (Inter)" prepended
    /// (`components/action_bar.md`). `Rc` so the render closure can clone it cheaply.
    font_names: Rc<Vec<SharedString>>,
    /// The font-family dropdown's open state (a `ChromeView`-owned scrolling menu panel).
    font_family_open: bool,
    /// The font-size dropdown's open state.
    font_size_open: bool,
    /// The borders popover's open state (the pen-model card — target icons + line gallery +
    /// color; `ui_design.md §2`). Only click-away / Esc closes it; a target click paints and
    /// keeps it open.
    borders_open: bool,
    /// The pen's **selected target** — which set of edges the line/color controls paint right now
    /// (`functional_spec.md §2.1`). `None` on open (and after `None`/click-away); reset every open.
    border_target: Option<BorderPreset>,
    /// The pen's **line style**, default thin solid, reset every open (`ui_design.md §2.4`).
    border_line: BorderLine,
    /// The pen's **color**, default black, reset every open.
    border_color: Rgb,
    /// The stock color picker for the borders popover's "Custom…" entry (reused pattern, like the
    /// fill/text-color pickers).
    border_color_picker: Entity<ColorPickerState>,
    /// The captured chrome-local left-x (device px) of each action-row dropdown trigger, so its
    /// popover anchors under the real (content-sized) button rather than a hardcoded offset (BUG
    /// 2c). Written by a per-button `canvas` bounds probe during paint; indexed by [`Anchor`].
    anchor_x: [f32; ANCHOR_COUNT],

    /// The sheet tabs (the chrome's mirror of the worker's sheet list).
    sheets: Vec<SheetTab>,
    /// The sheet being inline-renamed, if any.
    rename_target: Option<SheetId>,
    /// The inline-rename text input (reused across renames).
    rename_input: Entity<InputState>,
    /// Whether the pending rename failed validation (danger border, stays editing).
    rename_error: bool,
    /// The tab whose right-click context menu is open, if any.
    context_menu: Option<SheetId>,
    /// The sheet pending a delete confirmation (non-empty sheet), if any.
    confirm_delete: Option<SheetId>,
    /// A potential or in-flight tab reorder drag (`functional_spec.md §6`, `ui_design.md §3`).
    tab_drag: Option<TabDrag>,
    /// Each tab's captured window-space horizontal span, refreshed by a per-tab `canvas` probe on
    /// every paint — the geometry the pure insertion-index computation reads (a `Window`-free
    /// snapshot). Keyed by [`SheetId`]; read back in `self.sheets` order.
    tab_spans: Vec<TabSpan>,

    // ---- Find / replace bar (`functional_spec.md §4`, `ui_design.md §1`) -------------------
    /// Whether the find/replace bar is open (rendered below the data row, pushing the grid down).
    find_open: bool,
    /// The Find field's text buffer.
    find_input: Entity<InputState>,
    /// The Replace field's text buffer.
    replace_input: Entity<InputState>,
    /// The **match-case** toggle (`Aa`): off = case-insensitive (default), on = exact case.
    match_case: bool,
    /// The **match-entire-cell** toggle: off = substring (default), on = whole-cell equality.
    whole_cell: bool,
    /// The current match set (row-major `CellRef`s from the worker's `FindResults`); empty = no
    /// matches / empty find field.
    matches: Vec<CellRef>,
    /// The index into [`matches`](Self::matches) of the current match, or `None` when there are no
    /// matches. Drives the "N of M" counter + which cell is selected/revealed.
    match_idx: Option<usize>,
    /// Set while a `ReplaceAll` reply is awaited, so its `ReplacedCount` shows the "Replaced N"
    /// notice (a single `ReplaceOne`'s count is not surfaced — `functional_spec.md §4.4`).
    pending_replace_all: bool,
    /// A transient "Replaced N" notice shown in the counter after a Replace All until the user next
    /// edits the find field / steps matches (`functional_spec.md §4.4`).
    replaced_notice: Option<usize>,

    // ---- Selection stats (the tab-bar status readout, `functional_spec.md §1`) --------------
    /// The latest worker-computed aggregate for the current selection, or `None` when there is
    /// nothing to show (a single-cell/empty selection, or no reply yet). Rendered right-aligned in
    /// the tab bar; only shown when it has ≥1 numeric cell (`SelectionStats::has_numeric`).
    selection_stats: Option<SelectionStats>,
    /// Whether the readout is expanded to also show Min / Max (a **session-only** toggle, flipped by
    /// clicking the readout — `functional_spec.md §1`).
    stats_show_minmax: bool,
    /// Monotonic tag for the debounced stats query: it both debounces (only the most-recently armed
    /// timer fires the send) and stamps the request's `req_id`, so a reply for a superseded
    /// selection is dropped.
    stats_seq: u64,

    /// Horizontal-scroller state for the **action row** — its button groups scroll (with chevrons)
    /// when the window is too small to fit them (`functional_spec.md §9B`, call site 1).
    action_scroller: HScroller,
    /// Horizontal-scroller state for the **sheet-tab strip** — the tabs scroll while the
    /// selection-stats group stays pinned static to the right (`functional_spec.md §9B`, call site 2
    /// → §9A.4 always-visible).
    tab_scroller: HScroller,

    /// The grid, hosted as the chrome's body so the layout is action-row → data-row → **grid**
    /// → tab-bar (`ui_design.md §3`). `None` in the standalone Phase-9 demo/tests; the Phase-11
    /// window installs the real `GridView` via [`set_grid_body`](Self::set_grid_body).
    body: Option<gpui::AnyView>,

    _subscriptions: Vec<gpui::Subscription>,
}

impl ChromeView {
    /// Builds the chrome over `client`, delivering grid requests to `grid`. Starts on
    /// `active_sheet` with an A1 selection and the given tabs; the content field begins Idle
    /// and fetches on the first `on_selection_changed`.
    pub fn new(
        client: Rc<dyn ChromeClient>,
        grid: ChromeGridSink,
        active_sheet: SheetId,
        sheets: Vec<SheetTab>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let content_input = cx.new(|cx| InputState::new(window, cx).placeholder(""));
        let in_cell_input = cx.new(|cx| InputState::new(window, cx).placeholder(""));
        let rename_input = cx.new(|cx| InputState::new(window, cx));
        let chart_title_input = cx.new(|cx| InputState::new(window, cx).placeholder("Chart title"));
        let chart_cat_axis_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Category axis"));
        let chart_val_axis_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Value axis"));
        let color_picker = cx.new(|cx| ColorPickerState::new(window, cx));
        let text_color_picker = cx.new(|cx| ColorPickerState::new(window, cx));
        let border_color_picker = cx.new(|cx| ColorPickerState::new(window, cx));
        let find_input = cx.new(|cx| InputState::new(window, cx).placeholder("Find"));
        let replace_input = cx.new(|cx| InputState::new(window, cx).placeholder("Replace with"));
        let cf_range_input = cx.new(|cx| InputState::new(window, cx).placeholder("e.g. B2:B20"));
        let cf_operand1_input = cx.new(|cx| InputState::new(window, cx).placeholder("Value"));
        let cf_operand2_input = cx.new(|cx| InputState::new(window, cx).placeholder("Value"));
        let cf_formula_input = cx.new(|cx| InputState::new(window, cx).placeholder("=A1>0"));
        let cf_stop_value_inputs: Vec<Entity<InputState>> = (0..3)
            .map(|_| cx.new(|cx| InputState::new(window, cx).placeholder("Value")))
            .collect();

        // Installed font families for the dropdown, fetched once (`all_font_names` is verified
        // available). "Default (Inter)" is prepended as the clear-the-override entry.
        let mut names: Vec<SharedString> =
            std::iter::once(SharedString::from(SYSTEM_DEFAULT_FAMILY))
                .chain(
                    cx.text_system()
                        .all_font_names()
                        .into_iter()
                        .map(SharedString::from),
                )
                .collect();
        names.dedup();
        let font_names = Rc::new(names);

        // The data-row edit keys (Tab and — in quick-edit — the unmodified arrows) must be seen
        // *before* the gpui-component single-line `Input` acts on them. That `Input` binds
        // Left/Right to caret actions (`MoveLeft`/`MoveRight`) via the keymap; in this gpui build,
        // action bindings dispatch *before* any `capture_key_down`/`on_key_down` listener and stop
        // propagation once handled, so an ancestor capture listener can never preempt the input's
        // Left/Right (Up/Down happen to be unbound in single-line mode, which is the only reason
        // they used to work). A keystroke *interceptor* is the one phase that runs before the
        // input's action bindings, and `stop_propagation` inside it prevents that action dispatch
        // (`feature-gaps-7-11/DECISIONS_TO_REVIEW.md`). It is guarded to this view's focused
        // data-row input, so it never touches other inputs or the in-cell overlay, and it delegates
        // to the same [`handle_data_row_edit_key`](Self::handle_data_row_edit_key) the direct-call
        // unit tests exercise.
        let weak = cx.weak_entity();
        let mut subscriptions = vec![
            cx.subscribe_in(&content_input, window, Self::on_content_event),
            cx.subscribe_in(&in_cell_input, window, Self::on_incell_event),
            cx.subscribe_in(&rename_input, window, Self::on_rename_event),
            cx.subscribe_in(&color_picker, window, Self::on_color_picker_event),
            cx.subscribe_in(&text_color_picker, window, Self::on_text_color_picker_event),
            cx.subscribe_in(
                &border_color_picker,
                window,
                Self::on_border_color_picker_event,
            ),
            cx.subscribe_in(&chart_title_input, window, Self::on_chart_title_event),
            cx.subscribe_in(&chart_cat_axis_input, window, Self::on_chart_cat_axis_event),
            cx.subscribe_in(&chart_val_axis_input, window, Self::on_chart_val_axis_event),
            cx.subscribe_in(&find_input, window, Self::on_find_input_event),
            cx.subscribe_in(&replace_input, window, Self::on_replace_input_event),
            cx.subscribe_in(&cf_range_input, window, Self::on_cf_input_event),
            cx.subscribe_in(&cf_operand1_input, window, Self::on_cf_input_event),
            cx.subscribe_in(&cf_operand2_input, window, Self::on_cf_input_event),
            cx.subscribe_in(&cf_formula_input, window, Self::on_cf_input_event),
            cx.intercept_keystrokes(move |event, window, cx| {
                let Some(view) = weak.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    // Only when this view's data-row input is the focused editor — never the
                    // in-cell overlay (its own input) or an unrelated field.
                    let focused = this
                        .content_input
                        .read(cx)
                        .focus_handle(cx)
                        .is_focused(window);
                    if !focused {
                        return;
                    }
                    let keystroke = &event.keystroke;
                    let key = keystroke.key.as_str();
                    if this.handle_data_row_edit_key(key, keystroke.modifiers, window, cx) {
                        // Suppress the input's competing caret action for this keystroke.
                        cx.stop_propagation();
                    } else if matches!(key, "left" | "right" | "home" | "end")
                        && this.data_mode() == FieldMode::Editing
                    {
                        // A caret-only key falls through to the input; recompute the list/hint once
                        // the input has moved the caret (`functional_spec.md §1`).
                        this.schedule_autocomplete_recompute(window, cx);
                    }
                });
            }),
        ];
        for input in &cf_stop_value_inputs {
            subscriptions.push(cx.subscribe_in(input, window, Self::on_cf_input_event));
        }

        Self {
            client,
            grid,
            focus_handle: cx.focus_handle(),
            active_sheet,
            selection: SelectionModel::default(),
            active_style: None,
            active_num_fmt: None,
            active_font_family: None,
            active_published: None,
            default_font_size_pt: None,
            degraded: false,
            data_row: DataRow::default(),
            content_input,
            edit: EditController::new(in_cell_input),
            edit_state_shown: false,
            quick_edit: false,
            committed_cell: None,
            cap_error_external: None,
            eval: EvalIndicator::default(),
            fill_open: false,
            color_picker,
            text_color_open: false,
            text_color_picker,
            num_fmt_open: false,
            num_fmt_more_open: false,
            chart_menu_open: false,
            chart_panel: None,
            chart_title_input,
            chart_cat_axis_input,
            chart_val_axis_input,
            chart_input_focus: None,
            cond_fmt: None,
            cf_range_input,
            cf_operand1_input,
            cf_operand2_input,
            cf_formula_input,
            cf_stop_value_inputs,
            cf_menu_open: None,
            anchor_x: [0.0; ANCHOR_COUNT],
            font_names,
            font_family_open: false,
            font_size_open: false,
            borders_open: false,
            border_target: None,
            border_line: BorderLine::ThinSolid,
            border_color: Rgb::new(0, 0, 0),
            border_color_picker,
            sheets,
            rename_target: None,
            rename_input,
            rename_error: false,
            context_menu: None,
            confirm_delete: None,
            tab_drag: None,
            tab_spans: Vec::new(),
            find_open: false,
            find_input,
            replace_input,
            match_case: false,
            whole_cell: false,
            matches: Vec::new(),
            match_idx: None,
            pending_replace_all: false,
            replaced_notice: None,
            selection_stats: None,
            stats_show_minmax: false,
            stats_seq: 0,
            action_scroller: HScroller::new(),
            tab_scroller: HScroller::new(),
            body: None,
            _subscriptions: subscriptions,
        }
    }

    // ---- Selection + data-row plumbing ----------------------------------------------------
}

/// A vertical divider between action-row control groups (`ui_design.md §2`, existing styling).
/// `pub(super)` so the sibling [`super::h_scroller`] reuses the exact same divider for the
/// horizontal scroller's chevron section (`functional_spec.md §9B`, D9.3) and the tab bar's
/// leading stats divider (§9A.3).
pub(super) fn action_divider() -> gpui::Div {
    div().w(px(1.0)).h(px(20.0)).mx_1().bg(rgb(DIVIDER))
}
