//! [`ChromeView`] — the action row, data row (formula bar), and sheet tab bar as one GPUI
//! entity (`components/app_shell.md`, `ui_design.md §3.1–3.4`).
//!
//! Thin plumbing over the Phase-2 pure logic: the [`DataRow`] reducer drives the content
//! field, the [`EvalIndicator`] drives the evaluating spinner, [`FILL_PALETTE`] the fill
//! swatches, and [`validate_sheet_name`] the inline rename. Every user action is a plain
//! method here (so it is unit-testable without pixel clicks); the widget handlers just call
//! those methods, and the reducers' effects are performed as [`ChromeClient`] commands and
//! [`ChromeGridRequest`]s.
//!
//! The fill popover, tab context menu, and delete-confirm modal are lightweight
//! `ChromeView`-owned panels (controlled by view state) rather than the stock
//! gpui-component `Popover`/`ContextMenu`/`Modal` — their content closures run in a foreign
//! entity context, which would force cross-entity dispatch for what is a functional-POC
//! surface (`ui_design.md`: "this is chrome — don't over-invest"). Buttons, the text inputs,
//! the color picker, and the spinner are stock gpui-component controls as specced.

use std::rc::Rc;
use std::time::Duration;

use gpui::{
    canvas, div, prelude::*, px, rgb, App, ClickEvent, Context, Entity, FocusHandle, Focusable,
    Hsla, KeyDownEvent, MouseButton, MouseDownEvent, Rgba, SharedString, Window,
};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::spinner::Spinner;
use gpui_component::{Disableable as _, Selectable as _, Sizable as _};

use freecell_core::data_row::{DataRow, DataRowEffect, DataRowEvent, FieldMode};
use freecell_core::eval_indicator::{EvalEffect, EvalEvent, EvalIndicator};
use freecell_core::format_ui::{
    adjust_decimals_cell, displayed_decimals, font_size_display, num_fmt_category, Category,
    DROPDOWN_FORMATS,
};
use freecell_core::input_cap::InputRejection;
use freecell_core::palette::FILL_PALETTE;
use freecell_core::selection::{Direction, Motion};
use freecell_core::sheet_name::validate_sheet_name;
use freecell_core::{Align, CellKind, CellRef, RenderStyle, Rgb, SelectionModel, SheetId, VAlign};

use freecell_engine::{
    BorderLine, BorderPreset, Command, EditRejectedReason, StyleAttr, StylePath, WorkerEvent,
};

use super::{
    ChromeClient, ChromeGridRequest, ChromeGridSink, EditController, EditOrigin, SheetTab,
};

/// The 250 ms no-flash delay for both the content-fetch and evaluating spinners
/// (`ui_design.md §3.1/§3.2`, mirrored from the grid's own delayed hooks).
const SPINNER_DELAY: Duration = Duration::from_millis(250);

// --- Chrome look constants (functional POC greys; `ui_design.md §3`) -----------------
const CHROME_BG: u32 = 0xF3F3F3;
const HAIRLINE: u32 = 0xD9D9D9;
const DIVIDER: u32 = 0xC8C8C8;
const ACTIVE_TAB_BG: u32 = 0xFFFFFF;
const TEXT: u32 = 0x1F1F1F;
const MUTED_TEXT: u32 = 0x555555;
/// Danger border/text for cap-rejected input + invalid rename (theme danger, `#DC2626`).
const DANGER: u32 = 0xDC2626;
/// Dark tooltip fill + text for the cap-error popover (`ui_design.md §4`).
const TOOLTIP_BG: u32 = 0x2B2B2B;
const TOOLTIP_TEXT: u32 = 0xF5F5F5;

const ACTION_ROW_H: f32 = 36.0;

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
}
const ANCHOR_COUNT: usize = 6;

impl Anchor {
    fn idx(self) -> usize {
        self as usize
    }
}
/// The action row's natural (uncompressed) width for the current control set — font family +
/// size (Phase 5), B/I/U + strikethrough/wrap, text color + fill, **borders** (Phase 6),
/// horizontal + vertical alignment, number format + decimals — with its dividers. The row never
/// wraps (`ui_design.md §2`: raise the window's min width instead), so it holds this min width; the
/// document window (1200 px) is far wider. Phase 6 added the borders button (~64 px) + a divider
/// (816 → 896); the formatting-expansion project adds strikethrough + wrap toggles and the
/// three-button vertical-align group + a divider (~180 px → 896 → 1080). Recorded in
/// DECISIONS_TO_REVIEW — regenerate the true value from a real render if it clips.
const ACTION_ROW_MIN_W: f32 = 1080.0;

/// The fixed font-size dropdown list in points (`functional_spec.md §3.2`).
const FONT_SIZES: [f64; 12] = [8., 9., 10., 11., 12., 14., 16., 18., 20., 24., 28., 36.];
/// The top "clear the family override" entry in the font-family dropdown (`ui_design.md §2`).
const SYSTEM_DEFAULT_FAMILY: &str = "Default (Inter)";
const DATA_ROW_H: f32 = 32.0;
/// The formula-bar content entry's height: [`DATA_ROW_H`] minus 2 px breathing room above **and**
/// below (BUG C), so the row's `items_center` insets the entry within the bar without changing the
/// bar height. gpui-component's single-line `Input` otherwise renders at its fixed control height
/// (`Size::Medium` → 32 px) and fills the row edge-to-edge, which reads as cramped.
const DATA_ROW_FIELD_H: f32 = DATA_ROW_H - 4.0;
const TAB_BAR_H: f32 = 30.0;
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
    /// The installed font-family names for the family dropdown, fetched once at build
    /// (`cx.text_system().all_font_names()`), sorted-unique with "Default (Inter)" prepended
    /// (`components/action_bar.md`). `Rc` so the render closure can clone it cheaply.
    font_names: Rc<Vec<SharedString>>,
    /// The font-family dropdown's open state (a `ChromeView`-owned scrolling menu panel).
    font_family_open: bool,
    /// The font-size dropdown's open state.
    font_size_open: bool,
    /// The borders-preset popover's open state (a 4×2 preset grid — presets are actions, not
    /// toggles, so it carries no active-state derivation; `components/action_bar.md`).
    borders_open: bool,
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
        let color_picker = cx.new(|cx| ColorPickerState::new(window, cx));
        let text_color_picker = cx.new(|cx| ColorPickerState::new(window, cx));

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

        let subscriptions = vec![
            cx.subscribe_in(&content_input, window, Self::on_content_event),
            cx.subscribe_in(&in_cell_input, window, Self::on_incell_event),
            cx.subscribe_in(&rename_input, window, Self::on_rename_event),
            cx.subscribe_in(&color_picker, window, Self::on_color_picker_event),
            cx.subscribe_in(&text_color_picker, window, Self::on_text_color_picker_event),
        ];

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
            committed_cell: None,
            cap_error_external: None,
            eval: EvalIndicator::default(),
            fill_open: false,
            color_picker,
            text_color_open: false,
            text_color_picker,
            num_fmt_open: false,
            anchor_x: [0.0; ANCHOR_COUNT],
            font_names,
            font_family_open: false,
            font_size_open: false,
            borders_open: false,
            sheets,
            rename_target: None,
            rename_input,
            rename_error: false,
            context_menu: None,
            confirm_delete: None,
            body: None,
            _subscriptions: subscriptions,
        }
    }

    /// Installs the grid as the chrome's body (the Phase-11 window calls this once), so the
    /// chrome renders action-row → data-row → grid (flex-fill) → tab-bar in one stack.
    pub fn set_grid_body(&mut self, body: gpui::AnyView, cx: &mut Context<Self>) {
        self.body = Some(body);
        cx.notify();
    }

    /// Re-reads the active cell's resolved style (the action-row toggle pressed states) without
    /// disturbing the data row — for a `StyleCacheUpdated` after a formatting edit that didn't
    /// move the selection (`components/app_shell.md §Action row`).
    pub fn refresh_active_style(&mut self, cx: &mut Context<Self>) {
        if self.selection.is_single() {
            let cell = self.selection.active;
            self.active_style = self.client.render_style(self.active_sheet, cell);
            self.active_num_fmt = self.client.num_fmt_code(self.active_sheet, cell);
            self.active_font_family = self.client.font_family_name(self.active_sheet, cell);
            self.active_published = self.client.published_cell(self.active_sheet, cell);
        } else {
            self.active_style = None;
            self.active_num_fmt = None;
            self.active_font_family = None;
            self.active_published = None;
        }
        // The workbook default size is selection-independent (used to label a default cell).
        self.default_font_size_pt = self.client.default_font_size_pt(self.active_sheet);
        cx.notify();
    }

    // ---- Selection + data-row plumbing ----------------------------------------------------

    /// The grid's selection changed: refresh the ref box + toggle states, and drive the
    /// content field's fetch/disable via the reducer.
    pub fn on_selection_changed(
        &mut self,
        selection: SelectionModel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selection = selection;
        self.cap_error_external = None;
        // A multi-cell selection clears the reducer's `committed` (data_row multi arm), so the
        // seed tag it named is no longer valid — reset it (else a later collapse-to-single +
        // in-cell open would seed the just-cleared empty content; data-corruption guard).
        if !selection.is_single() {
            self.committed_cell = None;
        }
        if selection.is_single() {
            self.active_style = self
                .client
                .render_style(self.active_sheet, selection.active);
            self.active_num_fmt = self
                .client
                .num_fmt_code(self.active_sheet, selection.active);
            self.active_font_family = self
                .client
                .font_family_name(self.active_sheet, selection.active);
            self.active_published = self
                .client
                .published_cell(self.active_sheet, selection.active);
        } else {
            self.active_style = None;
            self.active_num_fmt = None;
            self.active_font_family = None;
            self.active_published = None;
        }
        // The workbook default size is selection-independent (used to label a default cell).
        self.default_font_size_pt = self.client.default_font_size_pt(self.active_sheet);
        let effects = self.data_row.reduce(DataRowEvent::SelectionChanged {
            single: selection.is_single(),
        });
        // begin_fetch / disable cleared the field; mirror the reducer's text into the widget.
        self.sync_input_from_reducer(window, cx);
        self.apply_data_effects(effects, window, cx);
        // A selection change ends any pending edit — close the in-cell overlay + clear the mirror.
        self.edit.close();
        self.refresh_edit_grid_state(window, cx);
        cx.notify();
    }

    /// The grid asks the field to commit a pending edit before a click-away selection change
    /// (`components/grid.md`). Returns whether the field is now committable (a cap-rejected
    /// edit blocks — the caller keeps the field editing and cancels the pending move).
    pub fn on_edit_commit_requested(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let was_editing = self.data_row.mode() == FieldMode::Editing;
        let effects = self.data_row.reduce(DataRowEvent::EditCommitRequested);
        self.apply_data_effects(effects, window, cx);
        let committed = self.data_row.mode() != FieldMode::Editing;
        self.note_commit(was_editing);
        // A committed (or absent) edit closes the overlay; a cap-rejected one stays open + editing.
        if committed {
            self.edit.close();
        }
        self.refresh_edit_grid_state(window, cx);
        cx.notify();
        committed
    }

    /// Escape while editing: revert the field to the last-fetched content, close any in-cell
    /// overlay, and hand focus back to the grid.
    pub fn escape_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.data_row.mode() != FieldMode::Editing {
            return;
        }
        let effects = self.data_row.reduce(DataRowEvent::Escape);
        self.sync_input_from_reducer(window, cx);
        self.mirror_to_in_cell(window, cx);
        self.apply_data_effects(effects, window, cx);
        self.edit.close();
        self.refresh_edit_grid_state(window, cx);
        cx.notify();
    }

    // ---- Pending edit: type-to-replace, in-cell editor, Tab, mirror -----------------------
    // (`components/edit_controller.md`; the single pending edit lives in `content_input` + the
    // `DataRow` reducer, with `edit` adding the in-cell overlay + two-editor sync.)

    /// The reused in-cell editor input — the window hands a clone to the grid so it can render the
    /// overlay (`components/edit_controller.md §4.4`).
    pub fn in_cell_input(&self) -> Entity<InputState> {
        self.edit.in_cell_input()
    }

    /// Type-to-replace (`functional_spec.md §1.1`): a printable keystroke on the focused grid
    /// starts an edit of the active cell whose content is **replaced** by `text`, caret at end, in
    /// the data row (never the in-cell overlay). Works from Idle **or** a multi-cell selection
    /// (targets the active cell — the grid collapses the range first).
    pub fn begin_typed(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.edit.close();
        self.edit.set_origin(EditOrigin::DataRow);
        self.cap_error_external = None;
        // Force Editing with the typed char (supersedes any pending fetch / disabled multi state).
        let effects = self.data_row.reduce(DataRowEvent::Edited {
            text: text.to_string(),
        });
        self.content_input.update(cx, |input, cx| {
            input.set_value(text.to_string(), window, cx);
            input.focus(window, cx);
        });
        self.apply_data_effects(effects, window, cx);
        self.refresh_edit_grid_state(window, cx);
        cx.notify();
    }

    /// Open the in-cell editor over `cell` (`functional_spec.md §1.3`). Double-click / F2 route
    /// here. Seeds from the reducer's **committed** content (the last fetched raw), so it shows the
    /// real content even if a redundant re-select cleared the live field but the reply already
    /// landed. If a first content fetch is still in flight the overlay opens empty and
    /// [`on_worker_event`](Self::on_worker_event) promotes it once the reply arrives
    /// (empty-with-spinner, `§Grid integration`).
    pub fn begin_in_cell(&mut self, cell: CellRef, window: &mut Window, cx: &mut Context<Self>) {
        // Don't relocate the overlay onto a different cell while another cell's edit is still
        // pending (e.g. a cap-rejected click-away, whose selection revert is deferred) — the
        // reducer + selection remain on the old cell, so opening here would diverge (review #2).
        if self.data_row.mode() == FieldMode::Editing && cell != self.selection.active {
            return;
        }
        self.cap_error_external = None;
        // Enter Editing seeded with the committed raw content, unless already editing this cell
        // (F2 mid-edit keeps the pending text) or the fetch for THIS cell hasn't landed yet. The
        // reducer keeps a previous cell's `committed` across a single→single selection change, so
        // seed only when `committed` is known to belong to `cell`; otherwise open empty and let the
        // in-flight reply promote it (guards a cross-cell stale-content commit, review New Critical).
        // Only seed when not already editing this cell AND `committed` is known to hold THIS
        // sheet+cell's fetched content; otherwise leave the reducer Idle-awaiting and let the
        // in-flight reply promote the overlay.
        if self.data_row.mode() != FieldMode::Editing
            && self.committed_cell == Some((self.active_sheet, cell))
        {
            let committed = self.data_row.committed().to_string();
            self.content_input.update(cx, |input, cx| {
                input.set_value(committed.clone(), window, cx);
            });
            let effects = self
                .data_row
                .reduce(DataRowEvent::Edited { text: committed });
            self.apply_data_effects(effects, window, cx);
        }
        let text = self.content_input.read(cx).value().to_string();
        self.edit.set_syncing(true);
        self.edit.in_cell().update(cx, |input, cx| {
            input.set_value(text, window, cx);
            input.focus(window, cx);
        });
        self.edit.set_syncing(false);
        self.edit.open_on(cell);
        self.refresh_edit_grid_state(window, cx);
        cx.notify();
    }

    /// Tab / Shift+Tab from the in-cell overlay (routed via the grid): commit + move
    /// right / left (`functional_spec.md §1.4`).
    pub fn commit_incell_move(
        &mut self,
        dir: Direction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit_and_move(dir, window, cx);
    }

    /// Escape from the in-cell overlay (routed via the grid): cancel the edit, revert, close. When
    /// the overlay is open but no edit has started yet (a first fetch is still in flight), there is
    /// nothing to revert — just close the overlay and return focus to the grid.
    pub fn cancel_incell(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.data_row.mode() == FieldMode::Editing {
            self.escape_edit(window, cx);
        } else if self.edit.is_open() {
            self.edit.close();
            self.grid.emit(ChromeGridRequest::FocusGrid, window, cx);
            self.refresh_edit_grid_state(window, cx);
            cx.notify();
        }
    }

    /// Commit the pending edit and move the active cell in `dir` (Enter → Down, Shift+Enter → Up,
    /// Tab → Right, Shift+Tab → Left). A cap-rejected commit keeps the edit (no move). Shared by
    /// both editors' Enter/Tab paths.
    fn commit_and_move(&mut self, dir: Direction, window: &mut Window, cx: &mut Context<Self>) {
        let was_editing = self.data_row.mode() == FieldMode::Editing;
        let mut effects = self.data_row.reduce(DataRowEvent::Commit);
        // The reducer's Commit hardcodes a Down move; retarget it to `dir`.
        for effect in &mut effects {
            if matches!(
                effect,
                DataRowEffect::MoveActive(Motion::Move(Direction::Down))
            ) {
                *effect = DataRowEffect::MoveActive(Motion::Move(dir));
            }
        }
        self.apply_data_effects(effects, window, cx);
        self.note_commit(was_editing);
        // A successful commit ends the edit → close the overlay; a cap-rejected one stays open.
        if self.data_row.mode() != FieldMode::Editing {
            self.edit.close();
        }
        self.refresh_edit_grid_state(window, cx);
        cx.notify();
    }

    /// After a `Commit`/`EditCommitRequested` reduce, keep the [`committed_cell`](Self::committed_cell)
    /// tag consistent with the reducer's `committed`. When an edit that was in progress
    /// (`was_editing`) just committed (now no longer Editing), the reducer set `committed` to the
    /// **active cell's** just-committed content — so re-tag it to `(active_sheet, active)`. In the
    /// click-away path `selection.active` is still the edited cell here (the selection moves only
    /// afterwards), so the tag names the right cell (data-corruption guard).
    fn note_commit(&mut self, was_editing: bool) {
        if was_editing && self.data_row.mode() != FieldMode::Editing {
            self.committed_cell = Some((self.active_sheet, self.selection.active));
        }
    }

    /// The in-cell overlay input emitted an event: `Change` drives the shared edit (mirrored to the
    /// data row); `PressEnter` commits + moves; `Focus` makes the in-cell editor the driver.
    fn on_incell_event(
        &mut self,
        _input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                if self.edit.is_syncing() {
                    return; // the echo of our own push into this editor — ignore (guard the loop)
                }
                self.cap_error_external = None;
                let text = self.edit.in_cell().read(cx).value().to_string();
                // Push into the data-row editor (events suppressed) and drive the shared reducer.
                self.edit.set_syncing(true);
                self.content_input.update(cx, |input, cx| {
                    input.set_value(text.clone(), window, cx);
                });
                self.edit.set_syncing(false);
                let effects = self.data_row.reduce(DataRowEvent::Edited { text });
                self.apply_data_effects(effects, window, cx);
                self.refresh_edit_grid_state(window, cx);
                cx.notify();
            }
            InputEvent::PressEnter { shift, .. } => {
                self.commit_and_move(
                    if *shift {
                        Direction::Up
                    } else {
                        Direction::Down
                    },
                    window,
                    cx,
                );
            }
            InputEvent::Focus => {
                self.edit.set_origin(EditOrigin::InCell);
                // The active editor drives which side shows the cap popover — re-push so the grid
                // reflects the flip (avoids a transient double popover, review #4).
                self.refresh_edit_grid_state(window, cx);
                cx.notify();
            }
            InputEvent::Blur => {}
        }
    }

    /// Mirrors the data-row editor's current text into the in-cell editor (events suppressed) when
    /// the overlay is open — the other half of the two-editor sync.
    fn mirror_to_in_cell(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.edit.is_open() || self.edit.is_syncing() {
            return;
        }
        let text = self.content_input.read(cx).value().to_string();
        self.edit.set_syncing(true);
        self.edit.in_cell().update(cx, |input, cx| {
            input.set_value(text, window, cx);
        });
        self.edit.set_syncing(false);
    }

    /// Pushes the current edit's grid-facing state (live mirror, in-cell overlay cell, in-cell cap
    /// message) to the grid. Called after every edit transition
    /// (`components/edit_controller.md §4.3–4.4`). The overlay is opened/closed explicitly by the
    /// edit entry/exit methods (not auto-closed here), so the in-cell editor can stay open while an
    /// initial content fetch is still in flight (empty-with-spinner, `§Grid integration`).
    fn refresh_edit_grid_state(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let editing = self.data_row.mode() == FieldMode::Editing;
        let mirror = editing.then(|| {
            let text: SharedString = self.content_input.read(cx).value().to_string().into();
            (self.active_sheet, self.selection.active, text)
        });
        let in_cell = self.edit.open_cell();
        let cap = (self.edit.origin() == EditOrigin::InCell)
            .then(|| self.cap_error_message())
            .flatten()
            .map(SharedString::from);
        let nonempty = mirror.is_some() || in_cell.is_some();
        // Skip an all-`None` clear when nothing was shown (idle selection moves would otherwise
        // re-push every keystroke); always push when something is/was shown so the clear lands.
        if !nonempty && !self.edit_state_shown {
            return;
        }
        self.edit_state_shown = nonempty;
        self.grid.emit(
            ChromeGridRequest::EditState {
                mirror,
                in_cell,
                cap,
            },
            window,
            cx,
        );
    }

    /// Folds a worker event into the chrome (Phase 11 calls this from the event task; tests
    /// call it directly).
    pub fn on_worker_event(
        &mut self,
        event: WorkerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            WorkerEvent::CellContent { req_id, raw } => {
                let was_awaiting = self.data_row.is_awaiting();
                self.data_row
                    .reduce(DataRowEvent::ContentFetched { req_id, raw });
                // Sync the widget only when the reducer populated the field (fresh reply,
                // still Idle) — never mid-edit, so a late reply can't reset the caret.
                if self.data_row.mode() == FieldMode::Idle {
                    self.sync_input_from_reducer(window, cx);
                    // A reply that actually landed (cleared `awaiting`) is the current active
                    // cell's content — record which cell `committed` now belongs to, and, if the
                    // in-cell editor opened before its content arrived (empty-with-spinner),
                    // promote it to an edit seeded with it (`§Grid integration`; review #3).
                    let landed = was_awaiting && !self.data_row.is_awaiting();
                    if landed {
                        self.committed_cell = Some((self.active_sheet, self.selection.active));
                        if self.edit.is_open() {
                            let text = self.content_input.read(cx).value().to_string();
                            let effects = self.data_row.reduce(DataRowEvent::Edited { text });
                            self.apply_data_effects(effects, window, cx);
                            self.mirror_to_in_cell(window, cx);
                            self.refresh_edit_grid_state(window, cx);
                        }
                    }
                }
                cx.notify();
            }
            WorkerEvent::EvalStarted => {
                let effects = self.eval.reduce(EvalEvent::Started);
                self.apply_eval_effects(effects, cx);
            }
            WorkerEvent::EvalFinished => {
                self.eval.reduce(EvalEvent::Finished);
                cx.notify();
            }
            WorkerEvent::Loaded { sheets } | WorkerEvent::SheetsChanged { sheets } => {
                self.merge_sheet_metas(&sheets);
                cx.notify();
            }
            WorkerEvent::EditRejected {
                reason: EditRejectedReason::InputCap(rejection),
            } => {
                self.cap_error_external = Some(rejection);
                cx.notify();
            }
            // Published/Saved/SaveFailed/StyleCacheUpdated/other EditRejected reasons /
            // degraded are the window's concern (Phase 11 dirty state + modals).
            _ => {}
        }
    }

    /// Mirrors the reducer's current text into the content widget (suppressing the widget's
    /// change event — `InputState::set_value` sets `emit_events = false`).
    fn sync_input_from_reducer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.data_row.text().to_string();
        self.content_input
            .update(cx, |input, cx| input.set_value(text, window, cx));
    }

    /// The content input emitted an event: typing enters Editing; Enter commits (+ moves the
    /// active cell); Shift+Enter commits + moves up.
    fn on_content_event(
        &mut self,
        _input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                if self.edit.is_syncing() {
                    return; // the echo of an in-cell → data-row push — ignore (guard the loop)
                }
                // A keystroke dismisses the cap-error popover (`functional_spec.md §4.2`): the
                // reducer clears its own rejection in `Edited`; the worker backstop is cleared
                // here so both sources dismiss on the next keystroke.
                self.cap_error_external = None;
                let text = self.content_input.read(cx).value().to_string();
                let effects = self.data_row.reduce(DataRowEvent::Edited { text });
                self.apply_data_effects(effects, window, cx);
                self.mirror_to_in_cell(window, cx);
                self.refresh_edit_grid_state(window, cx);
                cx.notify();
            }
            InputEvent::PressEnter { shift, .. } => {
                // Enter commits + moves down, Shift+Enter up (the reducer's Commit hardcodes Down).
                self.commit_and_move(
                    if *shift {
                        Direction::Up
                    } else {
                        Direction::Down
                    },
                    window,
                    cx,
                );
            }
            InputEvent::Blur => {
                // Focus leaving the field dismisses the cap-error popover
                // (`functional_spec.md §4.2`). The reducer clears its own rejection on the
                // next edit/escape; the worker backstop is cleared here.
                if self.cap_error_external.take().is_some() {
                    cx.notify();
                }
            }
            InputEvent::Focus => {
                self.edit.set_origin(EditOrigin::DataRow);
                // Re-push so the in-cell cap popover (grid-side) clears when focus flips to the data
                // row and the data-row popover takes over (avoids a transient double, review #4).
                self.refresh_edit_grid_state(window, cx);
            }
        }
    }

    /// Performs the reducer's data-row effects: fetch/commit as client commands, move/focus as
    /// grid requests, and arm the 250 ms fetch-spinner timer.
    fn apply_data_effects(
        &mut self,
        effects: Vec<DataRowEffect>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for effect in effects {
            match effect {
                DataRowEffect::Fetch { req_id } => {
                    self.client.send(Command::GetCellContent {
                        sheet: self.active_sheet,
                        cell: self.selection.active,
                        req_id,
                    });
                    self.arm_fetch_timer(req_id, cx);
                }
                DataRowEffect::Commit { input } => {
                    self.client.send(Command::SetCellInput {
                        sheet: self.active_sheet,
                        cell: self.selection.active,
                        input,
                    });
                }
                DataRowEffect::MoveActive(motion) => {
                    self.grid
                        .emit(ChromeGridRequest::MoveActive(motion), window, cx);
                }
                DataRowEffect::FocusGrid => {
                    self.grid.emit(ChromeGridRequest::FocusGrid, window, cx);
                }
                // The danger border + fetch spinner render directly from the reducer's state.
                DataRowEffect::ShowCapError | DataRowEffect::SetSpinner(_) => {}
            }
        }
    }

    /// Arms the 250 ms content-fetch spinner timer for `req_id` (`ui_design.md §3.2`).
    fn arm_fetch_timer(&mut self, req_id: u64, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SPINNER_DELAY).await;
            this.update(cx, |this, cx| {
                this.data_row.reduce(DataRowEvent::FetchTimeout { req_id });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Performs the evaluating-spinner effects, arming the 250 ms timer when asked.
    fn apply_eval_effects(&mut self, effects: Vec<EvalEffect>, cx: &mut Context<Self>) {
        for effect in effects {
            if let EvalEffect::ArmTimer { epoch } = effect {
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(SPINNER_DELAY).await;
                    this.update(cx, |this, cx| {
                        this.eval.reduce(EvalEvent::Timeout { epoch });
                        cx.notify();
                    })
                    .ok();
                })
                .detach();
            }
        }
        cx.notify();
    }

    // ---- Action row: formatting -----------------------------------------------------------

    /// Toggles a character style over the selection; commits any pending edit first (the same
    /// rule as clicking another cell). A cap-rejected pending edit blocks the toggle.
    pub fn toggle_style(&mut self, attr: StyleAttr, window: &mut Window, cx: &mut Context<Self>) {
        if !self.commit_pending_edit(window, cx) {
            return; // an invalid pending edit blocks the format, keeping the field editing
        }
        self.client.send(Command::SetStyleAttr {
            sheet: self.active_sheet,
            range: self.selection.range(),
            attr,
        });
    }

    /// Applies a fill colour (`Some`) or clears it (`None`) over the selection; commits any
    /// pending edit first, and closes the fill popover.
    pub fn apply_fill(&mut self, fill: Option<Rgb>, window: &mut Window, cx: &mut Context<Self>) {
        self.fill_open = false;
        if !self.commit_pending_edit(window, cx) {
            return;
        }
        self.client.send(Command::SetStyleAttr {
            sheet: self.active_sheet,
            range: self.selection.range(),
            attr: StyleAttr::Fill(fill),
        });
        cx.notify();
    }

    /// Commits a pending data-row edit if any. Returns whether the field is now committable
    /// (`false` = a cap-rejected edit is still open).
    fn commit_pending_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.data_row.mode() == FieldMode::Editing {
            let effects = self.data_row.reduce(DataRowEvent::EditCommitRequested);
            self.apply_data_effects(effects, window, cx);
            self.note_commit(true);
            if self.data_row.mode() != FieldMode::Editing {
                self.edit.close();
            }
            self.refresh_edit_grid_state(window, cx);
        }
        self.data_row.mode() != FieldMode::Editing
    }

    fn toggle_fill_popover(&mut self, cx: &mut Context<Self>) {
        self.fill_open = !self.fill_open;
        cx.notify();
    }

    fn on_color_picker_event(
        &mut self,
        _picker: &Entity<ColorPickerState>,
        event: &ColorPickerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ColorPickerEvent::Change(color) = event;
        if let Some(hsla) = color {
            self.apply_fill(Some(hsla_to_rgb(*hsla)), window, cx);
        }
    }

    // ---- Action row: SetStylePath (text color, alignment, number format) ------------------

    /// Sends one `SetStylePath` over the selection after committing any pending edit (the same
    /// rule as clicking another cell). Fire-and-forget: a cap-rejected pending edit blocks it, and
    /// the worker logs any engine rejection (the UI only ever sends valid paths/values).
    fn apply_style_path(
        &mut self,
        path: StylePath,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // No mutating control may dispatch while degraded/read-only (`functional_spec.md §6`) — a
        // backstop to the disabled buttons, covering a swatch/entry clicked in a popover that was
        // open at the instant of degradation (also closed by `set_degraded`).
        if self.degraded {
            return;
        }
        if !self.commit_pending_edit(window, cx) {
            return;
        }
        self.client.send(Command::SetStylePath {
            sheet: self.active_sheet,
            range: self.selection.range(),
            path,
            value,
        });
        cx.notify();
    }

    /// Applies a text colour (`Some`) or clears it to Automatic (`None`, value `""`), closing the
    /// text-color popover.
    pub fn apply_text_color(
        &mut self,
        color: Option<Rgb>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.text_color_open = false;
        let value = match color {
            Some(rgb) => format!("#{:06X}", rgb.to_hex()),
            None => String::new(),
        };
        self.apply_style_path(StylePath::FontColor, value, window, cx);
    }

    /// Applies a horizontal alignment; re-pressing the active one clears the explicit alignment
    /// back to the type default (value `"general"` — clears horizontal only, never wrap/vertical).
    pub fn apply_alignment(&mut self, align: Align, window: &mut Window, cx: &mut Context<Self>) {
        let value = if self.align_active(align) {
            "general".to_string()
        } else {
            match align {
                Align::Left => "left",
                Align::Center => "center",
                Align::Right => "right",
            }
            .to_string()
        };
        self.apply_style_path(StylePath::AlignHorizontal, value, window, cx);
    }

    /// Applies a vertical alignment (top/center/bottom) over the selection — a plain radio-style
    /// set (`functional_spec.md §1.3`, `architecture.md §2`). Unlike horizontal align there is no
    /// re-press-to-clear: IronCalc's vertical default is `bottom` and the grid's default placement
    /// is also bottom (decision C — Excel-faithful), so there is no independent "unset" value to
    /// clear back to; the group is purely one-of-N (top / center / bottom).
    pub fn apply_valign(&mut self, valign: VAlign, window: &mut Window, cx: &mut Context<Self>) {
        let value = match valign {
            VAlign::Top => "top",
            VAlign::Center => "center",
            VAlign::Bottom => "bottom",
        }
        .to_string();
        self.apply_style_path(StylePath::AlignVertical, value, window, cx);
    }

    /// Applies a number-format code over the selection, closing the number-format dropdown.
    pub fn apply_num_fmt(&mut self, code: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.num_fmt_open = false;
        self.apply_style_path(StylePath::NumFmt, code.to_string(), window, cx);
    }

    /// Adjusts the active cell's number of decimal places by `delta` (`+1` / `-1`). Computed
    /// UI-side from the cached format string and the active cell's kind/display: a real numeric
    /// format is rewritten directly, and a *numeric* General cell (`200000`) is switched to a
    /// `0.0…` format (BUG 3); a no-op (`adjust_decimals_cell` → `None`) does nothing (the buttons
    /// also render disabled in that case).
    pub fn bump_decimals(&mut self, delta: i8, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.active_num_fmt.clone();
        let (numeric, displayed) = self.active_numeric_decimals();
        if let Some(new_code) = current
            .as_deref()
            .and_then(|c| adjust_decimals_cell(c, delta, numeric, displayed))
        {
            self.apply_num_fmt(&new_code, window, cx);
        }
    }

    fn toggle_text_color_popover(&mut self, cx: &mut Context<Self>) {
        self.text_color_open = !self.text_color_open;
        cx.notify();
    }

    fn toggle_num_fmt_popover(&mut self, cx: &mut Context<Self>) {
        self.num_fmt_open = !self.num_fmt_open;
        cx.notify();
    }

    // ---- Action row: SetFont (family + size) ----------------------------------------------

    /// Sends one `SetFont` over the selection after committing any pending edit (fire-and-forget,
    /// degraded-guarded — the same rule as the `SetStylePath` controls).
    fn apply_set_font(
        &mut self,
        family: Option<String>,
        size_pt: Option<f64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.degraded {
            return;
        }
        if !self.commit_pending_edit(window, cx) {
            return;
        }
        self.client.send(Command::SetFont {
            sheet: self.active_sheet,
            range: self.selection.range(),
            family,
            size_pt,
        });
        cx.notify();
    }

    /// Applies a font family over the selection, closing the family dropdown. "Default (Inter)"
    /// clears the override (sent as `Some("")`); any other name sets it.
    pub fn apply_font_family(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.font_family_open = false;
        let family = if name == SYSTEM_DEFAULT_FAMILY {
            String::new()
        } else {
            name.to_string()
        };
        self.apply_set_font(Some(family), None, window, cx);
    }

    /// Applies a font size (points) over the selection, closing the size dropdown.
    pub fn apply_font_size(&mut self, pt: f64, window: &mut Window, cx: &mut Context<Self>) {
        self.font_size_open = false;
        self.apply_set_font(None, Some(pt), window, cx);
    }

    fn toggle_font_family_popover(&mut self, cx: &mut Context<Self>) {
        self.font_family_open = !self.font_family_open;
        cx.notify();
    }

    fn toggle_font_size_popover(&mut self, cx: &mut Context<Self>) {
        self.font_size_open = !self.font_size_open;
        cx.notify();
    }

    // ---- Action row: SetBorders (preset popover) ------------------------------------------

    /// Applies a border `preset` over the selection, closing the borders popover. Commits any
    /// pending edit first and degraded-guards, the same rule as the other action-row controls
    /// (`components/action_bar.md`). Fire-and-forget: the worker logs any engine rejection.
    pub fn apply_borders(
        &mut self,
        preset: BorderPreset,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.borders_open = false;
        if self.degraded {
            return;
        }
        if !self.commit_pending_edit(window, cx) {
            return;
        }
        // Phase 2: the preset buttons keep applying the default pen (thin solid black); the pen
        // popover redesign (line/color selection) is Phase 3.
        self.client.send(Command::SetBorders {
            sheet: self.active_sheet,
            range: self.selection.range(),
            preset,
            line: BorderLine::ThinSolid,
            color: None,
        });
        cx.notify();
    }

    fn toggle_borders_popover(&mut self, cx: &mut Context<Self>) {
        self.borders_open = !self.borders_open;
        cx.notify();
    }

    /// Whether the borders popover is open (test/render introspection).
    pub fn borders_open(&self) -> bool {
        self.borders_open
    }

    /// The font-family dropdown's active label: the active cell's family, or "Default (Inter)" for a
    /// default-font (or multi-cell) selection (`components/action_bar.md`).
    pub fn font_family_label(&self) -> &str {
        match self.active_font_family.as_deref() {
            Some(name) if !name.is_empty() => name,
            _ => SYSTEM_DEFAULT_FAMILY,
        }
    }

    /// The font-size dropdown's active label. An explicit size (`font_size_q != 0`) shows `q/4` pt;
    /// a **default** cell shows the workbook's real default size (13pt for a new workbook, the file's
    /// default otherwise) — never a hardcoded value that would mismatch the cell. Re-picking that
    /// shown default from the list is a visual no-op (the engine maps size == the workbook default
    /// back to the sentinel), so no surprising size jump. `13` is the fallback before a cache loads
    /// (IronCalc's default; `DECISIONS_TO_REVIEW` records the residual pt↔px seam).
    pub fn font_size_label(&self) -> String {
        let q = self.active_style.map(|s| s.font_size_q).unwrap_or(0);
        if q != 0 {
            font_size_display(q)
        } else {
            format_size_pt(self.default_font_size_pt.unwrap_or(13.0))
        }
    }

    fn on_text_color_picker_event(
        &mut self,
        _picker: &Entity<ColorPickerState>,
        event: &ColorPickerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ColorPickerEvent::Change(color) = event;
        if let Some(hsla) = color {
            self.apply_text_color(Some(hsla_to_rgb(*hsla)), window, cx);
        }
    }

    /// Marks the worker degraded/read-only (or clears it) — disables every mutating action-bar
    /// control (`functional_spec.md §6`). Called by the window on `WorkerDegraded`. Closes any open
    /// formatting popover so a swatch/entry can't be clicked after the controls lock.
    pub fn set_degraded(&mut self, degraded: bool, cx: &mut Context<Self>) {
        if self.degraded != degraded {
            self.degraded = degraded;
            if degraded {
                self.fill_open = false;
                self.text_color_open = false;
                self.num_fmt_open = false;
                self.font_family_open = false;
                self.font_size_open = false;
                self.borders_open = false;
            }
            cx.notify();
        }
    }

    // ---- Sheet tab bar --------------------------------------------------------------------

    /// Replaces the tab list + active sheet (fixtures / Phase-11 init).
    pub fn set_sheets(&mut self, sheets: Vec<SheetTab>, active: SheetId, cx: &mut Context<Self>) {
        self.sheets = sheets;
        self.active_sheet = active;
        cx.notify();
    }

    /// Merges a worker sheet-meta list into the tab mirror. `has_content` is now sourced
    /// directly from the worker's `SheetMeta` (Phase 11 populated it), so the delete-confirm
    /// gate is correct against the real workbook.
    fn merge_sheet_metas(&mut self, metas: &[freecell_engine::SheetMeta]) {
        self.sheets = metas
            .iter()
            .map(|meta| SheetTab {
                id: meta.id,
                name: meta.name.clone(),
                has_content: meta.has_content,
            })
            .collect();
        if !self.sheets.iter().any(|t| t.id == self.active_sheet) {
            if let Some(first) = self.sheets.first() {
                self.active_sheet = first.id;
            }
        }
    }

    /// Adopts `id` as the active sheet because the *window* (not a tab click) switched it — the
    /// worker added a sheet, a sheet was deleted, or the initial load resolved. Unlike
    /// [`select_sheet`](Self::select_sheet) this does **not** re-emit a `SetActiveSheet` grid
    /// request (that would re-enter the window's `defer` loop); it only re-points the chrome's
    /// active sheet so every subsequent command/fetch and the tab highlight target the right
    /// sheet, and refreshes the action-row toggle state. Load-bearing: without this, adding a
    /// sheet left the chrome pointing at the OLD sheet and routed edits there (`functional_spec.md
    /// §3.7`).
    pub fn adopt_active_sheet(&mut self, id: SheetId, cx: &mut Context<Self>) {
        if id == self.active_sheet {
            return;
        }
        self.active_sheet = id;
        // The committed content belongs to the old sheet — invalidate its seed tag (the tag is also
        // sheet-qualified, so this is belt-and-braces against a cross-sheet stale seed).
        self.committed_cell = None;
        self.context_menu = None;
        self.refresh_active_style(cx);
    }

    /// Switches the active sheet (tab click) and asks the grid to follow.
    pub fn select_sheet(&mut self, id: SheetId, window: &mut Window, cx: &mut Context<Self>) {
        if id == self.active_sheet {
            return;
        }
        self.active_sheet = id;
        self.committed_cell = None;
        self.context_menu = None;
        self.grid
            .emit(ChromeGridRequest::SetActiveSheet(id), window, cx);
        cx.notify();
    }

    /// Adds a sheet (the worker names it and republishes; the UI switches on `SheetsChanged`).
    pub fn add_sheet(&self) {
        self.client.send(Command::AddSheet);
    }

    /// Starts an inline rename of `id`, seeding + focusing the rename input.
    pub fn rename_start(&mut self, id: SheetId, window: &mut Window, cx: &mut Context<Self>) {
        let name = self
            .sheets
            .iter()
            .find(|t| t.id == id)
            .map(|t| t.name.clone())
            .unwrap_or_default();
        self.rename_target = Some(id);
        self.rename_error = false;
        self.context_menu = None;
        self.rename_input.update(cx, |input, cx| {
            input.set_value(name, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    /// Commits the pending rename (Enter): validates against the other sheet names; invalid
    /// keeps editing with a danger border.
    pub fn commit_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.validated_rename(cx) {
            Some((id, name)) => {
                self.client.send(Command::RenameSheet { sheet: id, name });
                self.rename_target = None;
                self.rename_error = false;
                self.grid.emit(ChromeGridRequest::FocusGrid, window, cx);
            }
            None => {
                if self.rename_target.is_some() {
                    self.rename_error = true;
                }
            }
        }
        cx.notify();
    }

    /// Cancels the pending rename (Escape / blur-when-invalid), reverting to the tab label.
    pub fn cancel_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.rename_target.is_none() {
            return;
        }
        self.rename_target = None;
        self.rename_error = false;
        self.grid.emit(ChromeGridRequest::FocusGrid, window, cx);
        cx.notify();
    }

    /// The pending rename resolved to `(id, name)` iff it validates, else `None`.
    fn validated_rename(&self, cx: &Context<Self>) -> Option<(SheetId, String)> {
        let id = self.rename_target?;
        let name = self.rename_input.read(cx).value().trim().to_string();
        let others: Vec<&str> = self
            .sheets
            .iter()
            .filter(|t| t.id != id)
            .map(|t| t.name.as_str())
            .collect();
        validate_sheet_name(&name, &others)
            .ok()
            .map(|()| (id, name))
    }

    fn on_rename_event(
        &mut self,
        _input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::PressEnter { .. } => self.commit_rename(window, cx),
            InputEvent::Blur => {
                // Blur commits if valid, otherwise reverts (never traps focus in a bad name).
                if self.validated_rename(cx).is_some() {
                    self.commit_rename(window, cx);
                } else {
                    self.cancel_rename(window, cx);
                }
            }
            _ => {}
        }
    }

    fn open_context_menu(&mut self, id: SheetId, cx: &mut Context<Self>) {
        self.context_menu = Some(id);
        cx.notify();
    }

    fn close_context_menu(&mut self, cx: &mut Context<Self>) {
        self.context_menu = None;
        cx.notify();
    }

    /// Whether a sheet can be deleted (not the last one).
    pub fn delete_enabled(&self) -> bool {
        self.sheets.len() > 1
    }

    /// Requests deletion of `id`: a non-empty sheet opens the confirm modal; an empty one is
    /// deleted immediately. The last sheet cannot be deleted.
    pub fn request_delete(&mut self, id: SheetId, cx: &mut Context<Self>) {
        self.context_menu = None;
        if !self.delete_enabled() {
            cx.notify();
            return;
        }
        let has_content = self
            .sheets
            .iter()
            .find(|t| t.id == id)
            .map(|t| t.has_content)
            .unwrap_or(false);
        if has_content {
            self.confirm_delete = Some(id);
        } else {
            self.client.send(Command::DeleteSheet { sheet: id });
        }
        cx.notify();
    }

    /// Confirms the pending delete.
    pub fn confirm_delete(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.confirm_delete.take() {
            self.client.send(Command::DeleteSheet { sheet: id });
            cx.notify();
        }
    }

    /// Cancels the pending delete.
    pub fn cancel_delete(&mut self, cx: &mut Context<Self>) {
        self.confirm_delete = None;
        cx.notify();
    }

    // ---- Read accessors (tests + render) --------------------------------------------------

    /// The ref box text: `B7` / `B2:D9` for cells, and the band forms `C:C` / `3:7` / `A:XFD`
    /// for header selections (`components/grid_structure.md §5.2`).
    pub fn ref_box_text(&self) -> String {
        freecell_core::format_selection_ref(&self.selection)
    }

    /// The content field's current text.
    pub fn content_text(&self, cx: &App) -> String {
        self.content_input.read(cx).value().to_string()
    }

    /// The formula-bar mode.
    pub fn data_mode(&self) -> FieldMode {
        self.data_row.mode()
    }

    /// Whether the bold toggle is pressed (active cell is bold).
    pub fn bold_active(&self) -> bool {
        self.active_style.map(|s| s.bold).unwrap_or(false)
    }

    /// Whether the italic toggle is pressed.
    pub fn italic_active(&self) -> bool {
        self.active_style.map(|s| s.italic).unwrap_or(false)
    }

    /// Whether the underline toggle is pressed.
    pub fn underline_active(&self) -> bool {
        self.active_style.map(|s| s.underline).unwrap_or(false)
    }

    /// Whether the strikethrough toggle is pressed.
    pub fn strikethrough_active(&self) -> bool {
        self.active_style.map(|s| s.strikethrough).unwrap_or(false)
    }

    /// Whether the wrap-text toggle is pressed.
    pub fn wrap_active(&self) -> bool {
        self.active_style.map(|s| s.wrap).unwrap_or(false)
    }

    /// Whether an alignment button is pressed — the **explicit** alignment only (a number aligned
    /// right by type default shows no pressed button, matching Excel; `components/action_bar.md`).
    pub fn align_active(&self, align: Align) -> bool {
        self.active_style.and_then(|s| s.h_align) == Some(align)
    }

    /// Whether a vertical-alignment button is pressed — the active cell's resolved vertical
    /// alignment (`functional_spec.md §1.3`). Under decision C the resolver reports a defaulted
    /// bottom as `Some(Bottom)`, so a cell whose vertical is merely defaulted (e.g. only horizontal
    /// set, or loaded from `.xlsx`) lights **Align bottom**; a truly-clean cell (no alignment
    /// record at all) lights nothing but still renders bottom. Accepted Excel-ish behavior.
    pub fn valign_active(&self, valign: VAlign) -> bool {
        self.active_style.and_then(|s| s.v_align) == Some(valign)
    }

    /// The active cell's number-format [`Category`] (General on a multi-cell selection / no cache).
    pub fn num_fmt_category(&self) -> Category {
        num_fmt_category(self.active_num_fmt.as_deref().unwrap_or("general"))
    }

    /// The number-format dropdown's button label (the active cell's category name).
    pub fn num_fmt_category_label(&self) -> &'static str {
        self.num_fmt_category().label()
    }

    /// Whether the "increase decimals" button is enabled (not degraded, single cell, and the
    /// active format has an adjustable decimal group).
    pub fn increase_decimals_enabled(&self) -> bool {
        self.decimals_enabled(1)
    }

    /// Whether the "decrease decimals" button is enabled.
    pub fn decrease_decimals_enabled(&self) -> bool {
        self.decimals_enabled(-1)
    }

    fn decimals_enabled(&self, delta: i8) -> bool {
        if self.degraded {
            return false;
        }
        let (numeric, displayed) = self.active_numeric_decimals();
        self.active_num_fmt
            .as_deref()
            .and_then(|c| adjust_decimals_cell(c, delta, numeric, displayed))
            .is_some()
    }

    /// Whether the active cell is a *number* (not text/date/bool/error/empty) and, if so, how many
    /// decimals its value currently displays — the inputs the decimals ± need to enable/adjust a
    /// General-formatted number (BUG 3). Both come from the cached publication of the active cell.
    fn active_numeric_decimals(&self) -> (bool, Option<u8>) {
        match &self.active_published {
            Some((CellKind::Number, display)) => (true, displayed_decimals(display)),
            _ => (false, None),
        }
    }

    /// Whether the worker is degraded (read-only) — all mutating action-bar controls disable.
    pub fn is_degraded(&self) -> bool {
        self.degraded
    }

    /// Whether the text-color popover is open.
    pub fn text_color_open(&self) -> bool {
        self.text_color_open
    }

    /// Whether the number-format dropdown is open.
    pub fn num_fmt_open(&self) -> bool {
        self.num_fmt_open
    }

    /// Whether the evaluating spinner is shown.
    pub fn eval_spinner_visible(&self) -> bool {
        self.eval.spinner()
    }

    /// Whether the content-fetch spinner is shown.
    pub fn fetch_spinner_visible(&self) -> bool {
        self.data_row.spinner()
    }

    /// Whether the content field shows the cap-rejection danger state.
    pub fn cap_error_visible(&self) -> bool {
        self.data_row.cap_error() || self.cap_error_external.is_some()
    }

    /// The cap-error popover message (`functional_spec.md §4.2`), if a cap rejection is
    /// active. A local reject (the reducer's) takes precedence over the worker backstop.
    pub fn cap_error_message(&self) -> Option<String> {
        self.data_row
            .cap_rejection()
            .or(self.cap_error_external)
            .map(|r| r.message())
    }

    /// The active sheet id.
    pub fn active_sheet(&self) -> SheetId {
        self.active_sheet
    }

    /// The current tab list.
    pub fn sheets(&self) -> &[SheetTab] {
        &self.sheets
    }

    /// The sheet being renamed, if any.
    pub fn rename_target(&self) -> Option<SheetId> {
        self.rename_target
    }

    /// Whether the pending rename is showing the invalid-name state.
    pub fn rename_error(&self) -> bool {
        self.rename_error
    }

    /// The sheet awaiting delete confirmation, if any.
    pub fn confirm_delete_target(&self) -> Option<SheetId> {
        self.confirm_delete
    }

    /// The tab whose context menu is open, if any.
    pub fn context_menu_target(&self) -> Option<SheetId> {
        self.context_menu
    }

    /// Whether the fill popover is open.
    pub fn fill_open(&self) -> bool {
        self.fill_open
    }
}

/// Converts a gpui `Hsla` to a 24-bit [`Rgb`] (the color picker's "Custom…" pick).
fn hsla_to_rgb(hsla: Hsla) -> Rgb {
    let rgba: Rgba = hsla.into();
    Rgb::new(
        (rgba.r * 255.0).round() as u8,
        (rgba.g * 255.0).round() as u8,
        (rgba.b * 255.0).round() as u8,
    )
}

/// Formats a font size in points for the size box, trimming a trailing `.0` (`13.0` → `"13"`,
/// `10.5` → `"10.5"`) — the same look as [`font_size_display`] for explicit sizes.
fn format_size_pt(pt: f64) -> String {
    format!("{pt}")
}

/// A vertical divider between action-row control groups (`ui_design.md §2`, existing styling).
fn action_divider() -> gpui::Div {
    div().w(px(1.0)).h(px(20.0)).mx_1().bg(rgb(DIVIDER))
}

impl Focusable for ChromeView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ChromeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("freecell-chrome")
            .track_focus(&self.focus_handle)
            .relative()
            .flex()
            .flex_col()
            .w_full()
            // Fill the available height when hosting the grid, so the grid slot can flex.
            .when(self.body.is_some(), |d| d.flex_1().min_h_0())
            .child(self.render_action_row(cx))
            .child(self.render_data_row(cx))
            // The grid body fills the space between the data row and the tab bar
            // (`ui_design.md §3`: action → data → grid → tabs).
            .when_some(self.body.clone(), |d, body| {
                d.child(div().flex_1().min_h_0().w_full().child(body))
            })
            .child(self.render_tab_bar(cx))
            .children(self.render_overlays(cx))
    }
}

impl ChromeView {
    /// Wraps a dropdown/popover trigger `button` so its panel can anchor under the real, laid-out
    /// button position instead of a guessed pixel offset (BUG 2c). A zero-size `canvas` probe
    /// fills the wrapper and records the button's window-x into `anchor_x[which]` on each paint —
    /// chrome-local x equals window x (the chrome fills the window width from x = 0), and only the
    /// x is needed (the panel's y is the fixed action-row height). It notifies only on a real
    /// change, so a stable layout captures once and never render-loops.
    fn anchored_trigger(
        &self,
        which: Anchor,
        button: impl IntoElement,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let probe = cx.entity().downgrade();
        let idx = which.idx();
        div().relative().child(button).child(
            canvas(
                move |bounds, _window, app| {
                    probe
                        .update(app, |this, cx| {
                            let x = f32::from(bounds.origin.x);
                            if (this.anchor_x[idx] - x).abs() > 0.5 {
                                this.anchor_x[idx] = x;
                                cx.notify();
                            }
                        })
                        .ok();
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_full(),
        )
    }

    fn render_action_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // Every mutating control disables in degraded/read-only mode (`functional_spec.md §6`).
        let disabled = self.degraded;

        let toggle = |id: &'static str,
                      label: &'static str,
                      tooltip: &'static str,
                      pressed: bool,
                      attr: StyleAttr,
                      cx: &mut Context<Self>| {
            Button::new(id)
                .label(label)
                .tooltip(tooltip)
                .ghost()
                .small()
                .disabled(disabled)
                .selected(pressed)
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.toggle_style(attr, window, cx);
                }))
        };

        // An alignment toggle (pressed = the cell's *explicit* alignment).
        let align_btn = |id: &'static str,
                         tooltip: &'static str,
                         align: Align,
                         glyph: &'static str,
                         cx: &mut Context<Self>| {
            Button::new(id)
                .label(glyph)
                .tooltip(tooltip)
                .ghost()
                .small()
                .disabled(disabled)
                .selected(self.align_active(align))
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.apply_alignment(align, window, cx);
                }))
        };

        // A vertical-alignment button (pressed = the cell's explicit vertical alignment). Mirrors
        // `align_btn` but drives the vertical group (`ui_design.md §1.1`).
        let valign_btn = |id: &'static str,
                          tooltip: &'static str,
                          valign: VAlign,
                          glyph: &'static str,
                          cx: &mut Context<Self>| {
            Button::new(id)
                .label(glyph)
                .tooltip(tooltip)
                .ghost()
                .small()
                .disabled(disabled)
                .selected(self.valign_active(valign))
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.apply_valign(valign, window, cx);
                }))
        };

        div()
            .flex()
            .items_center()
            .gap_1()
            .w_full()
            .h(px(ACTION_ROW_H))
            // The row's groups don't wrap; the window's min width holds them (`ui_design.md §2`).
            .min_w(px(ACTION_ROW_MIN_W))
            .px_2()
            .bg(rgb(CHROME_BG))
            .border_b_1()
            .border_color(rgb(HAIRLINE))
            // Font family · size (`ui_design.md §2`):
            .child(
                self.anchored_trigger(
                    Anchor::FontFamily,
                    Button::new("font-family")
                        .label(format!("{} ▾", self.font_family_label()))
                        .tooltip("Font")
                        .ghost()
                        .small()
                        .w(px(140.0))
                        .disabled(disabled)
                        .selected(self.font_family_open)
                        .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                            this.toggle_font_family_popover(cx);
                        })),
                    cx,
                ),
            )
            .child(
                self.anchored_trigger(
                    Anchor::FontSize,
                    Button::new("font-size")
                        .label(format!("{} ▾", self.font_size_label()))
                        .tooltip("Font size")
                        .ghost()
                        .small()
                        .w(px(56.0))
                        .disabled(disabled)
                        .selected(self.font_size_open)
                        .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                            this.toggle_font_size_popover(cx);
                        })),
                    cx,
                ),
            )
            .child(action_divider())
            // B I U:
            .child(toggle(
                "bold",
                "B",
                "Bold ⌘B",
                self.bold_active(),
                StyleAttr::Bold,
                cx,
            ))
            .child(toggle(
                "italic",
                "I",
                "Italic ⌘I",
                self.italic_active(),
                StyleAttr::Italic,
                cx,
            ))
            .child(toggle(
                "underline",
                "U",
                "Underline ⌘U",
                self.underline_active(),
                StyleAttr::Underline,
                cx,
            ))
            // Strikethrough (S with a combining long-stroke overlay) + Wrap text (⤶),
            // appended to the B/I/U toggle group (`ui_design.md §1.1`, `functional_spec.md §1`).
            .child(toggle(
                "strikethrough",
                "S\u{0336}",
                "Strikethrough",
                self.strikethrough_active(),
                StyleAttr::Strikethrough,
                cx,
            ))
            .child(toggle(
                "wrap",
                "\u{2936}",
                "Wrap text",
                self.wrap_active(),
                StyleAttr::WrapText,
                cx,
            ))
            .child(action_divider())
            // Text color · Fill:
            .child(
                self.anchored_trigger(
                    Anchor::TextColor,
                    Button::new("text-color")
                        .label("A ▾")
                        .tooltip("Text color")
                        .ghost()
                        .small()
                        .disabled(disabled)
                        .selected(self.text_color_open)
                        .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                            this.toggle_text_color_popover(cx);
                        })),
                    cx,
                ),
            )
            .child(
                self.anchored_trigger(
                    Anchor::Fill,
                    Button::new("fill")
                        .label("Fill ▾")
                        .tooltip("Fill color")
                        .ghost()
                        .small()
                        .disabled(disabled)
                        .selected(self.fill_open)
                        .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                            this.toggle_fill_popover(cx);
                        })),
                    cx,
                ),
            )
            .child(action_divider())
            // Borders preset popover:
            .child(
                self.anchored_trigger(
                    Anchor::Borders,
                    Button::new("borders")
                        .label("⊞ ▾")
                        .tooltip("Borders")
                        .ghost()
                        .small()
                        .disabled(disabled)
                        .selected(self.borders_open)
                        .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                            this.toggle_borders_popover(cx);
                        })),
                    cx,
                ),
            )
            .child(action_divider())
            // Alignment L / C / R:
            .child(align_btn("align-left", "Align left", Align::Left, "⇤", cx))
            .child(align_btn(
                "align-center",
                "Align center",
                Align::Center,
                "≡",
                cx,
            ))
            .child(align_btn(
                "align-right",
                "Align right",
                Align::Right,
                "⇥",
                cx,
            ))
            .child(action_divider())
            // Vertical alignment — its own group after horizontal align (`ui_design.md §1.1`):
            .child(valign_btn(
                "valign-top",
                "Align top",
                VAlign::Top,
                "\u{2912}",
                cx,
            ))
            .child(valign_btn(
                "valign-middle",
                "Align middle",
                VAlign::Center,
                "\u{2015}",
                cx,
            ))
            .child(valign_btn(
                "valign-bottom",
                "Align bottom",
                VAlign::Bottom,
                "\u{2913}",
                cx,
            ))
            .child(action_divider())
            // Number format dropdown + decimals ±:
            .child(
                self.anchored_trigger(
                    Anchor::NumFmt,
                    Button::new("num-fmt")
                        .label(format!("{} ▾", self.num_fmt_category_label()))
                        .tooltip("Number format")
                        .ghost()
                        .small()
                        .disabled(disabled)
                        .selected(self.num_fmt_open)
                        .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                            this.toggle_num_fmt_popover(cx);
                        })),
                    cx,
                ),
            )
            .child(
                Button::new("decimals-inc")
                    .label(".00→")
                    .tooltip("Increase decimals")
                    .ghost()
                    .small()
                    .disabled(!self.increase_decimals_enabled())
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.bump_decimals(1, window, cx);
                    })),
            )
            .child(
                Button::new("decimals-dec")
                    .label("→.00")
                    .tooltip("Decrease decimals")
                    .ghost()
                    .small()
                    .disabled(!self.decrease_decimals_enabled())
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.bump_decimals(-1, window, cx);
                    })),
            )
            // Right-aligned evaluating spinner (`ui_design.md §3.1`).
            .child(div().flex_1())
            .when(self.eval.spinner(), |row| row.child(Spinner::new().small()))
    }

    fn render_data_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let disabled = self.data_row.mode() == FieldMode::Disabled;
        let cap_error = self.cap_error_visible();

        // Inset the entry to `DATA_ROW_H - 4` so the row's `items_center` leaves 2 px above and
        // below it (BUG C), without shrinking the 32 px bar. gpui-component's single-line `Input`
        // pins a fixed 32 px control height (`Size::Medium` → `h_8`) that otherwise fills the row
        // edge-to-edge; `Input::h()` is multi-line-only, so pin the single-line control via
        // `min_h`/`max_h` (applied after `input_h` through `refine_style`). The 20 px line box fits
        // the 28 px control, so the normal-size text stays centered and un-clipped.
        let mut content = Input::new(&self.content_input)
            .disabled(disabled)
            .w_full()
            .min_h(px(DATA_ROW_FIELD_H))
            .max_h(px(DATA_ROW_FIELD_H));
        if self.fetch_spinner_visible() {
            content = content.suffix(Spinner::new().small());
        }

        div()
            .flex()
            .items_center()
            .gap_2()
            .w_full()
            .h(px(DATA_ROW_H))
            .px_2()
            .bg(rgb(CHROME_BG))
            .border_b_1()
            .border_color(rgb(HAIRLINE))
            // Escape reverts the edit (the InputState propagates Escape up to here).
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    this.escape_edit(window, cx);
                }
            }))
            // Tab / Shift+Tab commit + move right/left (`functional_spec.md §1.4`). Captured
            // **before** the input consumes the key (the bare gpui-component Input emits no commit
            // on Tab — `components/edit_controller.md §Tab interception`).
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "tab" && this.data_mode() == FieldMode::Editing {
                    cx.stop_propagation();
                    let dir = if event.keystroke.modifiers.shift {
                        Direction::Left
                    } else {
                        Direction::Right
                    };
                    this.commit_and_move(dir, window, cx);
                }
            }))
            // Ref box: read-only A1 address.
            .child(
                div()
                    .w(px(REF_BOX_W))
                    .h(px(22.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_sm()
                    .bg(rgb(ACTIVE_TAB_BG))
                    .border_1()
                    .border_color(rgb(HAIRLINE))
                    .text_size(px(12.0))
                    .text_color(rgb(TEXT))
                    .child(self.ref_box_text()),
            )
            .child(div().w(px(1.0)).h(px(20.0)).bg(rgb(DIVIDER)))
            // Content field (danger border on cap reject). The row's `items_center` centers this
            // (input-height) field so the 28 px entry sits 2 px inside the 32 px bar (BUG C).
            .child(
                div()
                    .flex_1()
                    .debug_selector(|| "data-content-field".into())
                    .when(cap_error, |d| {
                        d.border_1().border_color(rgb(DANGER)).rounded_md()
                    })
                    .child(content),
            )
    }

    fn render_tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut row = div()
            .flex()
            .items_center()
            .gap_1()
            .w_full()
            .h(px(TAB_BAR_H))
            .px_2()
            .bg(rgb(CHROME_BG))
            .border_t_1()
            .border_color(rgb(HAIRLINE));

        for tab in &self.sheets {
            row = row.child(self.render_tab(tab, cx));
        }

        row.child(
            Button::new("add-sheet")
                .label("+")
                .tooltip("New sheet")
                .ghost()
                .small()
                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                    this.add_sheet();
                    cx.notify();
                })),
        )
    }

    fn render_tab(&self, tab: &SheetTab, cx: &mut Context<Self>) -> gpui::AnyElement {
        let id = tab.id;
        let is_active = id == self.active_sheet;

        if self.rename_target == Some(id) {
            // Inline rename input in the tab's footprint.
            return div()
                .w(px(100.0))
                .when(self.rename_error, |d| {
                    d.border_1().border_color(rgb(DANGER)).rounded_md()
                })
                .child(Input::new(&self.rename_input).small())
                .into_any_element();
        }

        div()
            .id(gpui::ElementId::Name(format!("tab-{}", id.0).into()))
            .px_3()
            .h(px(24.0))
            .flex()
            .items_center()
            .rounded_t_md()
            .bg(rgb(if is_active { ACTIVE_TAB_BG } else { CHROME_BG }))
            .text_size(px(13.0))
            .text_color(rgb(if is_active { TEXT } else { MUTED_TEXT }))
            .when(is_active, |d| {
                d.border_t_1()
                    .border_l_1()
                    .border_r_1()
                    .border_color(rgb(HAIRLINE))
            })
            .child(tab.name.clone())
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if event.click_count() >= 2 {
                    this.rename_start(id, window, cx);
                } else {
                    this.select_sheet(id, window, cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _: &MouseDownEvent, _window, cx| {
                    this.open_context_menu(id, cx);
                }),
            )
            .into_any_element()
    }

    /// The floating overlays (fill popover, tab context menu, delete-confirm modal), each a
    /// `ChromeView`-owned panel over a dismiss backdrop.
    fn render_overlays(&self, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
        let mut overlays: Vec<gpui::AnyElement> = Vec::new();

        // The data-row cap popover anchors under the data row only when it is the active editor;
        // an in-cell cap error is shown under the overlay by the grid (`edit_controller.md §4.2`).
        if self.edit.origin() == EditOrigin::DataRow {
            if let Some(message) = self.cap_error_message() {
                overlays.push(self.render_cap_error_popover(message));
            }
        }
        if self.fill_open {
            overlays.push(self.render_fill_popover(cx));
        }
        if self.text_color_open {
            overlays.push(self.render_text_color_popover(cx));
        }
        if self.num_fmt_open {
            overlays.push(self.render_num_fmt_popover(cx));
        }
        if self.font_family_open {
            overlays.push(self.render_font_family_popover(cx));
        }
        if self.font_size_open {
            overlays.push(self.render_font_size_popover(cx));
        }
        if self.borders_open {
            overlays.push(self.render_borders_popover(cx));
        }
        if let Some(id) = self.context_menu {
            overlays.push(self.render_context_menu(id, cx));
        }
        if let Some(id) = self.confirm_delete {
            overlays.push(self.render_delete_confirm(id, cx));
        }
        overlays
    }

    fn backdrop(
        &self,
        on_dismiss: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            // Occlude the grid behind the popover: `BlockMouse` makes every hitbox behind this one
            // (the grid) un-hovered and un-scrollable, so a click on the overlay no longer also
            // moves the grid selection (BUG 2a) and scrolling anywhere over it no longer scrolls the
            // grid underneath (BUG 2b). The popover card, painted *after* this backdrop, still gets
            // its own clicks/scroll (it is in front, not behind).
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                    on_dismiss(this, window, cx);
                }),
            )
    }

    /// The cap-error popover (`functional_spec.md §4.2`, `ui_design.md §4`): a small dark
    /// tooltip anchored just below the data-row content field's left edge. No backdrop — it
    /// auto-dismisses on the next keystroke (reducer clears its rejection) or focus change.
    fn render_cap_error_popover(&self, message: String) -> gpui::AnyElement {
        div()
            .absolute()
            .top(px(ACTION_ROW_H + DATA_ROW_H + 2.0))
            .left(px(DATA_ROW_CONTENT_LEFT))
            .px_2()
            .py_1()
            .bg(rgb(TOOLTIP_BG))
            .text_color(rgb(TOOLTIP_TEXT))
            .text_size(px(11.0))
            .rounded_md()
            .shadow_md()
            .whitespace_nowrap()
            .child(message)
            .into_any_element()
    }

    fn render_fill_popover(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        // 5×2 swatch grid.
        let mut grid = div().flex().flex_col().gap_1();
        for chunk in FILL_PALETTE.chunks(5) {
            let mut r = div().flex().gap_1();
            for swatch in chunk {
                let color = swatch.rgb;
                r = r.child(
                    div()
                        .id(gpui::ElementId::Name(
                            format!("swatch-{}", swatch.name).into(),
                        ))
                        .debug_selector(|| format!("fill-swatch-{}", swatch.name))
                        .w(px(20.0))
                        .h(px(20.0))
                        .rounded_sm()
                        .bg(rgb(color.to_hex()))
                        .border_1()
                        .border_color(rgb(HAIRLINE))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                                this.apply_fill(Some(color), window, cx);
                            }),
                        ),
                );
            }
            grid = grid.child(r);
        }

        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(
                self.backdrop(
                    |this, _w, cx| {
                        this.fill_open = false;
                        cx.notify();
                    },
                    cx,
                )
                .child(div()),
            )
            .child(
                div()
                    .absolute()
                    .top(px(ACTION_ROW_H))
                    .left(px(self.anchor_x[Anchor::Fill.idx()]))
                    // Occlude the card so a mouse-down on it can't reach the backdrop's dismiss
                    // listener painted behind it (BUG A/B): the card's `BlockMouse` hitbox drops
                    // the backdrop out of the hit-test under the pointer, so `is_hovered` is false
                    // there and the backdrop's `on_mouse_down` never fires. Without this, clicking
                    // an item dismissed the popover on mouse-DOWN, tearing it down before the item's
                    // `on_click` (mouse-UP) could apply. Items inside paint above the card, so their
                    // own clicks are unaffected; a click OUTSIDE the card still dismisses.
                    .occlude()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_2()
                    .bg(rgb(ACTIVE_TAB_BG))
                    .border_1()
                    .border_color(rgb(HAIRLINE))
                    .rounded_md()
                    .shadow_md()
                    .child(grid)
                    .child(
                        Button::new("no-fill")
                            .label("No fill")
                            .debug_selector(|| "fill-no-fill".into())
                            .ghost()
                            .small()
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.apply_fill(None, window, cx);
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(rgb(MUTED_TEXT))
                                    .child("Custom…"),
                            )
                            .child(ColorPicker::new(&self.color_picker).small()),
                    ),
            )
            .into_any_element()
    }

    /// The text-color popover: the same palette as Fill, with **Automatic** (clear) in place of
    /// "No fill" (`components/action_bar.md`, `ui_design.md §2`).
    fn render_text_color_popover(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let mut grid = div().flex().flex_col().gap_1();
        for chunk in FILL_PALETTE.chunks(5) {
            let mut r = div().flex().gap_1();
            for swatch in chunk {
                let color = swatch.rgb;
                r = r.child(
                    div()
                        .id(gpui::ElementId::Name(
                            format!("text-swatch-{}", swatch.name).into(),
                        ))
                        .w(px(20.0))
                        .h(px(20.0))
                        .rounded_sm()
                        .bg(rgb(color.to_hex()))
                        .border_1()
                        .border_color(rgb(HAIRLINE))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                                this.apply_text_color(Some(color), window, cx);
                            }),
                        ),
                );
            }
            grid = grid.child(r);
        }

        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(
                self.backdrop(
                    |this, _w, cx| {
                        this.text_color_open = false;
                        cx.notify();
                    },
                    cx,
                )
                .child(div()),
            )
            .child(
                div()
                    .absolute()
                    .top(px(ACTION_ROW_H))
                    .left(px(self.anchor_x[Anchor::TextColor.idx()]))
                    // Occlude the card so item clicks don't trip the backdrop dismiss (BUG A/B).
                    .occlude()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_2()
                    .bg(rgb(ACTIVE_TAB_BG))
                    .border_1()
                    .border_color(rgb(HAIRLINE))
                    .rounded_md()
                    .shadow_md()
                    .child(grid)
                    .child(
                        Button::new("text-automatic")
                            .label("Automatic")
                            .debug_selector(|| "text-automatic".into())
                            .ghost()
                            .small()
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.apply_text_color(None, window, cx);
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(rgb(MUTED_TEXT))
                                    .child("Custom…"),
                            )
                            .child(ColorPicker::new(&self.text_color_picker).small()),
                    ),
            )
            .into_any_element()
    }

    /// The number-format dropdown: a plain scrolling menu of the seven categories, the active
    /// cell's category highlighted (`components/action_bar.md`, `architecture.md §3.1`).
    fn render_num_fmt_popover(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let active = self.num_fmt_category();
        let mut menu = div().flex().flex_col().gap(px(2.0));
        for (category, code) in DROPDOWN_FORMATS {
            let code = code.to_string();
            menu = menu.child(
                Button::new(gpui::ElementId::Name(
                    format!("numfmt-{}", category.label()).into(),
                ))
                .label(category.label())
                .debug_selector(move || format!("numfmt-{}", category.label()))
                .ghost()
                .small()
                .selected(category == active)
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.apply_num_fmt(&code, window, cx);
                })),
            );
        }

        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(
                self.backdrop(
                    |this, _w, cx| {
                        this.num_fmt_open = false;
                        cx.notify();
                    },
                    cx,
                )
                .child(div()),
            )
            .child(
                div()
                    .absolute()
                    .top(px(ACTION_ROW_H))
                    .left(px(self.anchor_x[Anchor::NumFmt.idx()]))
                    // Occlude the card so item clicks don't trip the backdrop dismiss (BUG A/B).
                    .occlude()
                    .debug_selector(|| "numfmt-card".into())
                    .flex()
                    .flex_col()
                    .p_1()
                    .bg(rgb(ACTIVE_TAB_BG))
                    .border_1()
                    .border_color(rgb(HAIRLINE))
                    .rounded_md()
                    .shadow_md()
                    .child(menu),
            )
            .into_any_element()
    }

    /// The font-family dropdown: a scrolling menu of the installed families (fetched once at build),
    /// "Default (Inter)" first, the active cell's family highlighted (`components/action_bar.md`).
    fn render_font_family_popover(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let active = self.font_family_label().to_string();
        let names = Rc::clone(&self.font_names);
        let mut menu = div().flex().flex_col().gap(px(1.0));
        for (i, name) in names.iter().enumerate() {
            let pick = name.to_string();
            menu = menu.child(
                Button::new(gpui::ElementId::NamedInteger(
                    "font-family".into(),
                    i as u64,
                ))
                .label(name.clone())
                .debug_selector(move || format!("font-family-{i}"))
                .ghost()
                .small()
                .selected(name.as_ref() == active)
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.apply_font_family(&pick, window, cx);
                })),
            );
        }

        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(
                self.backdrop(
                    |this, _w, cx| {
                        this.font_family_open = false;
                        cx.notify();
                    },
                    cx,
                )
                .child(div()),
            )
            .child(
                div()
                    .id("font-family-menu")
                    .absolute()
                    .top(px(ACTION_ROW_H))
                    .left(px(self.anchor_x[Anchor::FontFamily.idx()]))
                    // Occlude the card so item clicks don't trip the backdrop dismiss (BUG A/B).
                    .occlude()
                    .flex()
                    .flex_col()
                    .p_1()
                    // The installed-font list is long — cap the height and scroll it.
                    .max_h(px(320.0))
                    .overflow_y_scroll()
                    .bg(rgb(ACTIVE_TAB_BG))
                    .border_1()
                    .border_color(rgb(HAIRLINE))
                    .rounded_md()
                    .shadow_md()
                    .child(menu),
            )
            .into_any_element()
    }

    /// The font-size dropdown: the fixed point list, the active cell's size highlighted.
    fn render_font_size_popover(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let active = self.font_size_label();
        let mut menu = div().flex().flex_col().gap(px(1.0));
        for pt in FONT_SIZES {
            let label = format!("{pt}");
            menu = menu.child(
                Button::new(gpui::ElementId::NamedInteger("font-size".into(), pt as u64))
                    .label(label.clone())
                    .debug_selector(move || format!("font-size-{pt}"))
                    .ghost()
                    .small()
                    .selected(label == active)
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.apply_font_size(pt, window, cx);
                    })),
            );
        }

        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(
                self.backdrop(
                    |this, _w, cx| {
                        this.font_size_open = false;
                        cx.notify();
                    },
                    cx,
                )
                .child(div()),
            )
            .child(
                div()
                    .id("font-size-menu")
                    .absolute()
                    .top(px(ACTION_ROW_H))
                    .left(px(self.anchor_x[Anchor::FontSize.idx()]))
                    // Occlude the card so item clicks don't trip the backdrop dismiss (BUG A/B).
                    .occlude()
                    .flex()
                    .flex_col()
                    .p_1()
                    .max_h(px(320.0))
                    .overflow_y_scroll()
                    .bg(rgb(ACTIVE_TAB_BG))
                    .border_1()
                    .border_color(rgb(HAIRLINE))
                    .rounded_md()
                    .shadow_md()
                    .child(menu),
            )
            .into_any_element()
    }

    /// The borders popover: a 4×2 grid of preset actions (All/Inner/Outer/None over
    /// Top/Bottom/Left/Right — `ui_design.md §2`). Presets are actions, not toggles, so no button
    /// carries pressed state.
    fn render_borders_popover(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let preset_btn = |id: &'static str,
                          label: &'static str,
                          preset: BorderPreset,
                          cx: &mut Context<Self>| {
            Button::new(id)
                .label(label)
                .debug_selector(move || id.to_string())
                .ghost()
                .small()
                .w(px(64.0))
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.apply_borders(preset, window, cx);
                }))
        };
        let row1 = div()
            .flex()
            .gap_1()
            .child(preset_btn("border-all", "All", BorderPreset::All, cx))
            .child(preset_btn("border-inner", "Inner", BorderPreset::Inner, cx))
            .child(preset_btn("border-outer", "Outer", BorderPreset::Outer, cx))
            .child(preset_btn("border-none", "None", BorderPreset::None, cx));
        let row2 = div()
            .flex()
            .gap_1()
            .child(preset_btn("border-top", "Top", BorderPreset::Top, cx))
            .child(preset_btn(
                "border-bottom",
                "Bottom",
                BorderPreset::Bottom,
                cx,
            ))
            .child(preset_btn("border-left", "Left", BorderPreset::Left, cx))
            .child(preset_btn("border-right", "Right", BorderPreset::Right, cx));

        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(
                self.backdrop(
                    |this, _w, cx| {
                        this.borders_open = false;
                        cx.notify();
                    },
                    cx,
                )
                .child(div()),
            )
            .child(
                div()
                    .absolute()
                    .top(px(ACTION_ROW_H))
                    .left(px(self.anchor_x[Anchor::Borders.idx()]))
                    // Occlude the card so item clicks don't trip the backdrop dismiss (BUG A/B).
                    .occlude()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .p_2()
                    .bg(rgb(ACTIVE_TAB_BG))
                    .border_1()
                    .border_color(rgb(HAIRLINE))
                    .rounded_md()
                    .shadow_md()
                    .child(row1)
                    .child(row2),
            )
            .into_any_element()
    }

    fn render_context_menu(&self, id: SheetId, cx: &mut Context<Self>) -> gpui::AnyElement {
        let delete_enabled = self.delete_enabled();
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(self.backdrop(|this, _w, cx| this.close_context_menu(cx), cx))
            .child(
                div()
                    .absolute()
                    .bottom(px(TAB_BAR_H))
                    .left(px(16.0))
                    // Occlude the card so Rename/Delete clicks don't trip the backdrop dismiss on
                    // mouse-down before their `on_click` (mouse-up) fires (BUG A/B, same root cause
                    // as the action-bar popovers).
                    .occlude()
                    .flex()
                    .flex_col()
                    .p_1()
                    .bg(rgb(ACTIVE_TAB_BG))
                    .border_1()
                    .border_color(rgb(HAIRLINE))
                    .rounded_md()
                    .shadow_md()
                    .child(
                        Button::new("ctx-rename")
                            .label("Rename")
                            .ghost()
                            .small()
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                this.rename_start(id, window, cx);
                            })),
                    )
                    .child(
                        Button::new("ctx-delete")
                            .label("Delete")
                            .ghost()
                            .small()
                            .disabled(!delete_enabled)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                                this.request_delete(id, cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_delete_confirm(&self, id: SheetId, cx: &mut Context<Self>) -> gpui::AnyElement {
        let name = self
            .sheets
            .iter()
            .find(|t| t.id == id)
            .map(|t| t.name.clone())
            .unwrap_or_default();
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(0x000000).opacity(0.3))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_4()
                    .w(px(320.0))
                    .bg(rgb(ACTIVE_TAB_BG))
                    .border_1()
                    .border_color(rgb(HAIRLINE))
                    .rounded_lg()
                    .shadow_lg()
                    .child(
                        div()
                            .text_size(px(14.0))
                            .text_color(rgb(TEXT))
                            .child(format!("Delete sheet “{name}”? This can't be undone.")),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                Button::new("delete-cancel")
                                    .label("Cancel")
                                    .ghost()
                                    .small()
                                    .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                                        this.cancel_delete(cx);
                                    })),
                            )
                            .child(
                                Button::new("delete-confirm")
                                    .label("Delete")
                                    .danger()
                                    .small()
                                    .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                                        this.confirm_delete(cx);
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
impl ChromeView {
    /// Test seam: simulate the user typing `text` into the content field (sets the widget
    /// text, then delivers the `Change` event the subscription would).
    fn test_type(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.content_input
            .update(cx, |input, cx| input.set_value(text, window, cx));
        let handle = self.content_input.clone();
        self.on_content_event(&handle, &InputEvent::Change, window, cx);
    }

    /// Test seam: simulate pressing Enter (optionally with Shift) in the content field.
    fn test_press_enter(&mut self, shift: bool, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.content_input.clone();
        self.on_content_event(
            &handle,
            &InputEvent::PressEnter {
                secondary: false,
                shift,
            },
            window,
            cx,
        );
    }

    /// Test seam: set the rename input's text.
    fn test_rename_type(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.rename_input
            .update(cx, |input, cx| input.set_value(text, window, cx));
    }

    /// Test seam: simulate typing `text` into the in-cell editor (sets the widget text, then
    /// delivers the `Change` event the subscription would).
    fn test_incell_type(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.edit.in_cell().clone();
        handle.update(cx, |input, cx| input.set_value(text, window, cx));
        self.on_incell_event(&handle, &InputEvent::Change, window, cx);
    }

    /// Test seam: simulate pressing Enter (optionally with Shift) in the in-cell editor.
    fn test_incell_press_enter(
        &mut self,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let handle = self.edit.in_cell().clone();
        self.on_incell_event(
            &handle,
            &InputEvent::PressEnter {
                secondary: false,
                shift,
            },
            window,
            cx,
        );
    }

    /// Test seam: replicate the data-row Tab handler (commit + move right/left) without the
    /// widget-level `capture_key_down`.
    fn test_data_row_tab(&mut self, shift: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.data_mode() == FieldMode::Editing {
            let dir = if shift {
                Direction::Left
            } else {
                Direction::Right
            };
            self.commit_and_move(dir, window, cx);
        }
    }

    /// Test seam: the in-cell editor's current text.
    fn incell_text(&self, cx: &App) -> String {
        self.edit.in_cell().read(cx).value().to_string()
    }

    /// Test seam: the open in-cell overlay cell, if any.
    fn incell_open(&self) -> Option<CellRef> {
        self.edit.open_cell()
    }

    /// Test seam: which editor currently drives the edit.
    fn edit_origin(&self) -> EditOrigin {
        self.edit.origin()
    }

    /// Test seam: the captured chrome-local left-x of a dropdown trigger (BUG 2c anchoring).
    fn anchor_x_of(&self, which: Anchor) -> f32 {
        self.anchor_x[which.idx()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chrome::{ChromeClient, RecordingClient};
    use freecell_core::input_cap::MAX_INPUT_LEN;
    use freecell_core::{CellRef, SelectionModel};
    use freecell_engine::{BorderPreset, Command, SheetMeta, StyleAttr, WorkerEvent};
    use gpui::{px, size, TestAppContext};
    use gpui_component::Root;
    use std::cell::RefCell;

    /// A window hosting a `ChromeView` over a `RecordingClient`, plus a recording grid sink.
    struct Harness {
        chrome: Entity<ChromeView>,
        client: Rc<RecordingClient>,
        grid_requests: Rc<RefCell<Vec<ChromeGridRequest>>>,
        window: gpui::WindowHandle<Root>,
    }

    fn cell(row: u32, col: u32) -> CellRef {
        CellRef::new(row, col)
    }

    fn build(cx: &mut TestAppContext, sheets: Vec<SheetTab>, active: SheetId) -> Harness {
        build_win(cx, sheets, active, 200.0)
    }

    /// [`build`] with a caller-chosen window height — the popover-click tests want a tall enough
    /// window that every dropdown item lays out on-screen and can be hit by a simulated click.
    fn build_win(
        cx: &mut TestAppContext,
        sheets: Vec<SheetTab>,
        active: SheetId,
        height: f32,
    ) -> Harness {
        let client = Rc::new(RecordingClient::new());
        let grid_requests: Rc<RefCell<Vec<ChromeGridRequest>>> = Rc::new(RefCell::new(Vec::new()));

        cx.update(gpui_component::init);

        let client_for_window = client.clone();
        let reqs_for_window = grid_requests.clone();
        let mut chrome_out: Option<Entity<ChromeView>> = None;
        let chrome_slot = &mut chrome_out;

        // The test window matches the real document window width (1200 px) so the full action row
        // — including the number-format popover trigger past the vertical-align group — is on-screen
        // for the popover-hit tests (the row's natural width is ~1080 px, `ACTION_ROW_MIN_W`).
        let window = cx.open_window(size(px(1200.0), px(height)), |window, cx| {
            let client_dyn: Rc<dyn ChromeClient> = client_for_window;
            let reqs = reqs_for_window;
            let sink = ChromeGridSink::new(move |req, _w, _cx| reqs.borrow_mut().push(req.clone()));
            let chrome = cx.new(|cx| ChromeView::new(client_dyn, sink, active, sheets, window, cx));
            *chrome_slot = Some(chrome.clone());
            Root::new(chrome, window, cx)
        });

        Harness {
            chrome: chrome_out.expect("chrome built"),
            client,
            grid_requests,
            window,
        }
    }

    fn one_sheet(cx: &mut TestAppContext) -> Harness {
        build(cx, vec![SheetTab::new(SheetId(0), "Sheet1")], SheetId(0))
    }

    /// A stand-in for the hosted grid: an empty full-size body. Its only job is to make the chrome
    /// **fill the window** (`render` flexes only when a body is present), so a popover's full-window
    /// backdrop really spans the window height — the condition under which BUG A/B bites. With a
    /// bodyless chrome the backdrop is only ~3 rows tall and the dropdown items lay out *below* it,
    /// never overlapping it, so the regression would hide.
    struct BodyStub;
    impl gpui::Render for BodyStub {
        fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            // A concrete-height body so the chrome content — and thus the popover backdrop, which is
            // `size_full` of the chrome — spans well past the dropdown items. (`flex_1` alone won't
            // stretch it: the test Root sizes the chrome to its content, not the window.)
            div().h(px(500.0)).w_full()
        }
    }

    /// One sheet in a tall window with a (stub) grid body, for the popover-click tests: every item
    /// lays out on-screen over a full-height backdrop.
    fn tall_sheet(cx: &mut TestAppContext) -> Harness {
        let h = build_win(
            cx,
            vec![SheetTab::new(SheetId(0), "Sheet1")],
            SheetId(0),
            600.0,
        );
        upd(&h, cx, |c, _w, cx| {
            let body: gpui::AnyView = cx.new(|_| BodyStub).into();
            c.set_grid_body(body, cx);
        });
        h
    }

    /// Runs `f` against the chrome with a live `Window`.
    fn upd<R>(
        h: &Harness,
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut ChromeView, &mut Window, &mut Context<ChromeView>) -> R,
    ) -> R {
        h.window
            .update(cx, |_root, window, cx| {
                h.chrome.update(cx, |c, cx| f(c, window, cx))
            })
            .unwrap()
    }

    fn tick(cx: &mut TestAppContext, ms: u64) {
        cx.executor().advance_clock(Duration::from_millis(ms));
        cx.run_until_parked();
    }

    // ---- Data row: fetch / reply / disable -------------------------------------------------

    #[gpui::test]
    fn data_row_content_field_is_inset_within_bar(cx: &mut TestAppContext) {
        // BUG C: the formula-bar content entry must sit 2 px inside the 32 px bar (top and bottom)
        // — i.e. render at `DATA_ROW_H - 4` = 28 px — without changing the bar height. The field
        // wrapper hugs the hosted `Input`'s height (the bar is `items_center`, not stretch), so its
        // painted height is the control height. Without the `min_h`/`max_h` inset on the `Input`
        // the control renders at gpui-component's fixed 32 px and fills the bar edge-to-edge; this
        // asserts 28 px and fails if the inset is removed (verified fail-without / pass-with).
        let h = one_sheet(cx);
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        let field = vcx
            .debug_bounds("data-content-field")
            .expect("the data-row content field was painted");
        let field_h = f32::from(field.size.height);
        assert!(
            (field_h - DATA_ROW_FIELD_H).abs() < 0.5,
            "content field must render at DATA_ROW_H - 4 = {DATA_ROW_FIELD_H}px, got {field_h}"
        );
        // The inset must not have changed the bar height.
        assert_eq!(DATA_ROW_H, 32.0, "the data-row bar height must stay 32px");
        assert!(
            field_h + 3.5 < DATA_ROW_H,
            "the field must be shorter than the bar so items_center leaves breathing room"
        );
    }

    #[gpui::test]
    fn selection_single_fetches_content(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(1, 1)), window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(
            matches!(cmds.as_slice(), [Command::GetCellContent { cell: cc, req_id: 1, .. }] if *cc == cell(1, 1)),
            "expected one GetCellContent for B2, got {cmds:?}"
        );
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Idle);
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.ref_box_text()), "B2");
    }

    #[gpui::test]
    fn content_reply_populates_field(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(1, 1)), window, cx)
        });
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(
                WorkerEvent::CellContent {
                    req_id: 1,
                    raw: "=SUM(A1:A2)".into(),
                },
                window,
                cx,
            )
        });
        assert_eq!(upd(&h, cx, |c, _w, cx| c.content_text(cx)), "=SUM(A1:A2)");
    }

    #[gpui::test]
    fn stale_content_reply_dropped(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx); // req 1
            c.on_selection_changed(SelectionModel::single(cell(1, 0)), window, cx);
            // req 2
        });
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(
                WorkerEvent::CellContent {
                    req_id: 1,
                    raw: "stale".into(),
                },
                window,
                cx,
            )
        });
        assert_eq!(upd(&h, cx, |c, _w, cx| c.content_text(cx)), "");
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(
                WorkerEvent::CellContent {
                    req_id: 2,
                    raw: "fresh".into(),
                },
                window,
                cx,
            )
        });
        assert_eq!(upd(&h, cx, |c, _w, cx| c.content_text(cx)), "fresh");
    }

    #[gpui::test]
    fn multiselect_disables_field(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(1, 1)), window, cx);
            c.on_worker_event(
                WorkerEvent::CellContent {
                    req_id: 1,
                    raw: "42".into(),
                },
                window,
                cx,
            );
            c.on_selection_changed(
                SelectionModel {
                    anchor: cell(1, 1),
                    active: cell(3, 3),
                },
                window,
                cx,
            );
        });
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Disabled);
        assert_eq!(upd(&h, cx, |c, _w, cx| c.content_text(cx)), "");
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.ref_box_text()), "B2:D4");
    }

    // ---- Data row: edit / commit / escape / cap ------------------------------------------

    #[gpui::test]
    fn enter_commits_and_moves_down(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx)
        });
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.test_type("=1+1", window, cx);
            c.test_press_enter(false, window, cx);
        });
        let cmds = h.client.take_commands();
        assert!(
            matches!(cmds.as_slice(), [Command::SetCellInput { input, .. }] if input == "=1+1"),
            "expected SetCellInput, got {cmds:?}"
        );
        let reqs = h.grid_requests.borrow();
        assert!(reqs.iter().any(|r| matches!(
            r,
            ChromeGridRequest::MoveActive(Motion::Move(Direction::Down))
        )));
        assert!(reqs
            .iter()
            .any(|r| matches!(r, ChromeGridRequest::FocusGrid)));
    }

    #[gpui::test]
    fn shift_enter_moves_up(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(5, 0)), window, cx);
            c.test_type("v", window, cx);
            c.test_press_enter(true, window, cx);
        });
        let reqs = h.grid_requests.borrow();
        assert!(reqs.iter().any(|r| matches!(
            r,
            ChromeGridRequest::MoveActive(Motion::Move(Direction::Up))
        )));
    }

    #[gpui::test]
    fn escape_reverts_field(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.on_worker_event(
                WorkerEvent::CellContent {
                    req_id: 1,
                    raw: "42".into(),
                },
                window,
                cx,
            );
            c.test_type("999", window, cx);
        });
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Editing);
        upd(&h, cx, |c, window, cx| c.escape_edit(window, cx));
        assert_eq!(upd(&h, cx, |c, _w, cx| c.content_text(cx)), "42");
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Idle);
        assert!(h
            .grid_requests
            .borrow()
            .iter()
            .any(|r| matches!(r, ChromeGridRequest::FocusGrid)));
    }

    #[gpui::test]
    fn cap_reject_keeps_editing_and_flags_error(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        let huge = format!("={}", "1".repeat(MAX_INPUT_LEN));
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx)
        });
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.test_type(&huge, window, cx);
            c.test_press_enter(false, window, cx);
        });
        let cmds = h.client.take_commands();
        assert!(
            !cmds
                .iter()
                .any(|cmd| matches!(cmd, Command::SetCellInput { .. })),
            "a cap-rejected formula must not be committed"
        );
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Editing);
        assert!(upd(&h, cx, |c, _w, _cx| c.cap_error_visible()));
        // The popover shows the length-specific message (`functional_spec.md §4.2`).
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.cap_error_message()),
            Some("Formula too long (max 8,192 characters)".to_string())
        );
        // The next keystroke clears the danger state + popover.
        upd(&h, cx, |c, window, cx| c.test_type("=1", window, cx));
        assert!(!upd(&h, cx, |c, _w, _cx| c.cap_error_visible()));
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.cap_error_message()), None);
    }

    #[gpui::test]
    fn edit_commit_requested_commits_without_moving(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.test_type("=A1", window, cx);
        });
        h.client.take_commands();
        let committed = upd(&h, cx, |c, window, cx| {
            c.on_edit_commit_requested(window, cx)
        });
        assert!(committed);
        let cmds = h.client.take_commands();
        assert!(matches!(cmds.as_slice(), [Command::SetCellInput { input, .. }] if input == "=A1"));
        assert!(
            !h.grid_requests
                .borrow()
                .iter()
                .any(|r| matches!(r, ChromeGridRequest::MoveActive(_))),
            "click-away commit does not move the active cell itself"
        );
    }

    // ---- Action row: toggles + fill --------------------------------------------------------

    #[gpui::test]
    fn toggle_bold_sends_setstyleattr(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(1, 1)), window, cx)
        });
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.toggle_style(StyleAttr::Bold, window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(matches!(
            cmds.as_slice(),
            [Command::SetStyleAttr {
                attr: StyleAttr::Bold,
                ..
            }]
        ));
    }

    #[gpui::test]
    fn toggles_reflect_active_style(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        h.client.set_style(
            SheetId(0),
            cell(1, 1),
            RenderStyle {
                bold: true,
                italic: false,
                underline: true,
                ..Default::default()
            },
        );
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(1, 1)), window, cx)
        });
        assert!(upd(&h, cx, |c, _w, _cx| c.bold_active()));
        assert!(!upd(&h, cx, |c, _w, _cx| c.italic_active()));
        assert!(upd(&h, cx, |c, _w, _cx| c.underline_active()));
    }

    #[gpui::test]
    fn strikethrough_and_wrap_toggles_send_setstyleattr(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(1, 1)), window, cx)
        });
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.toggle_style(StyleAttr::Strikethrough, window, cx)
        });
        assert!(matches!(
            h.client.take_commands().as_slice(),
            [Command::SetStyleAttr {
                attr: StyleAttr::Strikethrough,
                ..
            }]
        ));
        upd(&h, cx, |c, window, cx| {
            c.toggle_style(StyleAttr::WrapText, window, cx)
        });
        assert!(matches!(
            h.client.take_commands().as_slice(),
            [Command::SetStyleAttr {
                attr: StyleAttr::WrapText,
                ..
            }]
        ));
    }

    #[gpui::test]
    fn strikethrough_and_wrap_reflect_active_style(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        h.client.set_style(
            SheetId(0),
            cell(1, 1),
            RenderStyle {
                strikethrough: true,
                wrap: false,
                ..Default::default()
            },
        );
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(1, 1)), window, cx)
        });
        assert!(upd(&h, cx, |c, _w, _cx| c.strikethrough_active()));
        assert!(!upd(&h, cx, |c, _w, _cx| c.wrap_active()));
    }

    #[gpui::test]
    fn fill_swatch_and_no_fill(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(1, 1)), window, cx)
        });
        h.client.take_commands();
        let accent = FILL_PALETTE[4].rgb; // Accent 1
        upd(&h, cx, |c, window, cx| {
            c.apply_fill(Some(accent), window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(matches!(
            cmds.as_slice(),
            [Command::SetStyleAttr { attr: StyleAttr::Fill(Some(rgb)), .. }] if *rgb == accent
        ));
        upd(&h, cx, |c, window, cx| c.apply_fill(None, window, cx));
        let cmds = h.client.take_commands();
        assert!(matches!(
            cmds.as_slice(),
            [Command::SetStyleAttr {
                attr: StyleAttr::Fill(None),
                ..
            }]
        ));
    }

    #[gpui::test]
    fn formatting_commits_pending_edit_first(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.test_type("=A1", window, cx);
        });
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.toggle_style(StyleAttr::Italic, window, cx)
        });
        let cmds = h.client.take_commands();
        // Commit first, then the style.
        assert!(
            matches!(cmds.first(), Some(Command::SetCellInput { input, .. }) if input == "=A1"),
            "pending edit committed first, got {cmds:?}"
        );
        assert!(cmds.iter().any(|c| matches!(
            c,
            Command::SetStyleAttr {
                attr: StyleAttr::Italic,
                ..
            }
        )));
    }

    // ---- Action row: SetStylePath (text color, alignment, number format) ------------------

    /// Select `cell` as a single-cell selection and drain the resulting fetch command.
    fn select_single(h: &Harness, cx: &mut TestAppContext, r: u32, c: u32) {
        upd(h, cx, |chrome, window, cx| {
            chrome.on_selection_changed(SelectionModel::single(cell(r, c)), window, cx)
        });
        h.client.take_commands();
    }

    #[gpui::test]
    fn alignment_toggle_emits_clear_on_repress(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        // The active cell is explicitly right-aligned.
        h.client.set_style(
            SheetId(0),
            cell(1, 1),
            RenderStyle {
                h_align: Some(Align::Right),
                ..Default::default()
            },
        );
        select_single(&h, cx, 1, 1);
        assert!(upd(&h, cx, |c, _w, _cx| c.align_active(Align::Right)));

        // Re-pressing the pressed alignment clears horizontal only (value "general").
        upd(&h, cx, |c, window, cx| {
            c.apply_alignment(Align::Right, window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::SetStylePath { path: StylePath::AlignHorizontal, value, .. }] if value == "general"
            ),
            "re-press clears with general, got {cmds:?}"
        );

        // Pressing a different (unpressed) alignment sets it directly.
        upd(&h, cx, |c, window, cx| {
            c.apply_alignment(Align::Left, window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(matches!(
            cmds.as_slice(),
            [Command::SetStylePath { path: StylePath::AlignHorizontal, value, .. }] if value == "left"
        ));
    }

    #[gpui::test]
    fn vertical_alignment_sets_and_reflects(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        // The active cell is explicitly top-aligned → the Top button reads pressed, others not.
        h.client.set_style(
            SheetId(0),
            cell(1, 1),
            RenderStyle {
                v_align: Some(VAlign::Top),
                ..Default::default()
            },
        );
        select_single(&h, cx, 1, 1);
        assert!(upd(&h, cx, |c, _w, _cx| c.valign_active(VAlign::Top)));
        assert!(!upd(&h, cx, |c, _w, _cx| c.valign_active(VAlign::Bottom)));

        // Pressing a vertical-align button is a plain set (no re-press-to-clear).
        upd(&h, cx, |c, window, cx| {
            c.apply_valign(VAlign::Bottom, window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(matches!(
            cmds.as_slice(),
            [Command::SetStylePath { path: StylePath::AlignVertical, value, .. }] if value == "bottom"
        ));

        // Re-pressing the already-active alignment re-applies it (no clear value).
        h.client.set_style(
            SheetId(0),
            cell(1, 1),
            RenderStyle {
                v_align: Some(VAlign::Center),
                ..Default::default()
            },
        );
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, window, cx| {
            c.apply_valign(VAlign::Center, window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(matches!(
            cmds.as_slice(),
            [Command::SetStylePath { path: StylePath::AlignVertical, value, .. }] if value == "center"
        ));
    }

    #[gpui::test]
    fn text_color_automatic_and_swatch(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        select_single(&h, cx, 1, 1);

        // Automatic clears the color (empty value).
        upd(&h, cx, |c, window, cx| c.apply_text_color(None, window, cx));
        let cmds = h.client.take_commands();
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::SetStylePath { path: StylePath::FontColor, value, .. }] if value.is_empty()
            ),
            "Automatic clears color, got {cmds:?}"
        );

        // A swatch sends its #RRGGBB hex.
        upd(&h, cx, |c, window, cx| {
            c.apply_text_color(Some(Rgb::from_hex(0xFF0000)), window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(matches!(
            cmds.as_slice(),
            [Command::SetStylePath { path: StylePath::FontColor, value, .. }] if value == "#FF0000"
        ));
    }

    #[gpui::test]
    fn num_fmt_pick_emits_code_and_category_reflects_active_cell(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        h.client.set_num_fmt(SheetId(0), cell(1, 1), "0.00%");
        select_single(&h, cx, 1, 1);
        // The dropdown label reflects the active cell's format category.
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.num_fmt_category_label()),
            "Percent"
        );

        upd(&h, cx, |c, window, cx| {
            c.apply_num_fmt("$#,##0.00", window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(matches!(
            cmds.as_slice(),
            [Command::SetStylePath { path: StylePath::NumFmt, value, .. }] if value == "$#,##0.00"
        ));
    }

    #[gpui::test]
    fn decimals_buttons_emit_adjusted_code(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        h.client.set_num_fmt(SheetId(0), cell(1, 1), "#,##0.00");
        select_single(&h, cx, 1, 1);
        assert!(upd(&h, cx, |c, _w, _cx| c.increase_decimals_enabled()));
        assert!(upd(&h, cx, |c, _w, _cx| c.decrease_decimals_enabled()));

        upd(&h, cx, |c, window, cx| c.bump_decimals(1, window, cx));
        let cmds = h.client.take_commands();
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::SetStylePath { path: StylePath::NumFmt, value, .. }] if value == "#,##0.000"
            ),
            "increase decimals rewrites the code, got {cmds:?}"
        );
    }

    #[gpui::test]
    fn decimals_disabled_for_date_format(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        h.client.set_num_fmt(SheetId(0), cell(1, 1), "m/d/yyyy");
        select_single(&h, cx, 1, 1);
        // A date format has no adjustable decimal group → both buttons disabled + no-op.
        assert!(!upd(&h, cx, |c, _w, _cx| c.increase_decimals_enabled()));
        assert!(!upd(&h, cx, |c, _w, _cx| c.decrease_decimals_enabled()));
        upd(&h, cx, |c, window, cx| c.bump_decimals(1, window, cx));
        assert!(
            h.client.take_commands().is_empty(),
            "a no-op decimals adjust sends nothing"
        );
    }

    #[gpui::test]
    fn dropdown_anchors_capture_button_positions_left_to_right(cx: &mut TestAppContext) {
        // BUG 2c: each dropdown popover anchors under its real (content-sized) trigger button, not
        // a hardcoded x. After a paint, the `canvas` probes capture each button's laid-out left
        // edge; they must land in left-to-right action-row order and be strictly increasing.
        let h = one_sheet(cx);
        // Force the window to paint so the canvas probes capture each button's laid-out x.
        {
            let vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
            vcx.run_until_parked();
        }

        let xs = upd(&h, cx, |c, _w, _cx| {
            [
                c.anchor_x_of(Anchor::FontFamily),
                c.anchor_x_of(Anchor::FontSize),
                c.anchor_x_of(Anchor::TextColor),
                c.anchor_x_of(Anchor::Fill),
                c.anchor_x_of(Anchor::Borders),
                c.anchor_x_of(Anchor::NumFmt),
            ]
        });
        assert!(
            xs[0] >= 0.0 && xs.windows(2).all(|w| w[1] > w[0]),
            "trigger anchors must be captured in strictly increasing left-to-right order, got {xs:?}"
        );
    }

    #[gpui::test]
    fn decimals_enabled_on_general_numeric_cell(cx: &mut TestAppContext) {
        // BUG 3: a plain number like `200000` is stored with the General format. The ± must still
        // be adjustable — increase applies `0.0`; decrease is a no-op at zero decimals (disabled).
        let h = one_sheet(cx);
        h.client.set_num_fmt(SheetId(0), cell(1, 1), "general");
        h.client
            .set_published_cell(SheetId(0), cell(1, 1), CellKind::Number, "200000");
        select_single(&h, cx, 1, 1);

        assert!(
            upd(&h, cx, |c, _w, _cx| c.increase_decimals_enabled()),
            "increase must be enabled on a General-formatted number"
        );
        assert!(
            !upd(&h, cx, |c, _w, _cx| c.decrease_decimals_enabled()),
            "decrease is a no-op on a General integer (0 decimals)"
        );

        upd(&h, cx, |c, window, cx| c.bump_decimals(1, window, cx));
        let cmds = h.client.take_commands();
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::SetStylePath { path: StylePath::NumFmt, value, .. }] if value == "0.0"
            ),
            "increase on a General number applies a 0.0 format, got {cmds:?}"
        );
    }

    #[gpui::test]
    fn decimals_disabled_on_general_text_cell(cx: &mut TestAppContext) {
        // A text cell under General is not numeric → the ± stay disabled and no-op.
        let h = one_sheet(cx);
        h.client.set_num_fmt(SheetId(0), cell(1, 1), "general");
        h.client
            .set_published_cell(SheetId(0), cell(1, 1), CellKind::Text, "hello");
        select_single(&h, cx, 1, 1);

        assert!(!upd(&h, cx, |c, _w, _cx| c.increase_decimals_enabled()));
        assert!(!upd(&h, cx, |c, _w, _cx| c.decrease_decimals_enabled()));
        upd(&h, cx, |c, window, cx| c.bump_decimals(1, window, cx));
        assert!(
            h.client.take_commands().is_empty(),
            "a text General cell must not emit a number-format change"
        );
    }

    #[gpui::test]
    fn decimals_gating_for_custom_formats_matches_spec(cx: &mut TestAppContext) {
        // BUG C audit: for a cell with an explicit *custom* number format, ± must be enabled iff the
        // format is safely adjustable — single-section, no exponent (`E`/`e`), no quoted/escaped
        // literal (`functional_spec.md §3.4`, the deliberate Phase-4 gate). This locks the exact
        // enable/disable set so it can be reconciled against what the owner observed.
        let h = one_sheet(cx);
        fn gate(h: &Harness, cx: &mut TestAppContext, code: &str) -> (bool, bool) {
            h.client.set_num_fmt(SheetId(0), cell(1, 1), code);
            select_single(h, cx, 1, 1);
            (
                upd(h, cx, |c, _w, _cx| c.increase_decimals_enabled()),
                upd(h, cx, |c, _w, _cx| c.decrease_decimals_enabled()),
            )
        }
        // Safe single-section customs ARE enabled: increase always, decrease when ≥1 decimal.
        assert_eq!(gate(&h, cx, "0.00"), (true, true), "0.00");
        assert_eq!(gate(&h, cx, "#,##0.00"), (true, true), "#,##0.00");
        assert_eq!(gate(&h, cx, "0.00%"), (true, true), "0.00%");
        // `#,##0` has zero decimals → increase enabled, decrease a correct no-op (Excel: can't go
        // below 0). This is NOT a bug: the format IS adjustable, there is just nothing to remove.
        assert_eq!(gate(&h, cx, "#,##0"), (true, false), "#,##0");
        // Only exponent / quoted / multi-section customs are (correctly) disabled both ways.
        assert_eq!(gate(&h, cx, "0.00E+00"), (false, false), "0.00E+00");
        assert_eq!(gate(&h, cx, "0.0\"x\""), (false, false), "0.0\"x\"");
        assert_eq!(
            gate(&h, cx, "0.00;[Red]0.00"),
            (false, false),
            "0.00;[Red]0.00"
        );
    }

    #[gpui::test]
    fn controls_disabled_in_degraded_mode(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        h.client.set_num_fmt(SheetId(0), cell(1, 1), "#,##0.00");
        select_single(&h, cx, 1, 1);
        // Enabled before degradation.
        assert!(!upd(&h, cx, |c, _w, _cx| c.is_degraded()));
        assert!(upd(&h, cx, |c, _w, _cx| c.increase_decimals_enabled()));

        upd(&h, cx, |c, _w, cx| c.set_degraded(true, cx));
        assert!(upd(&h, cx, |c, _w, _cx| c.is_degraded()));
        // The decimals gate folds in the degraded flag (the other controls disable via
        // `.disabled(self.is_degraded())` in the render path).
        assert!(!upd(&h, cx, |c, _w, _cx| c.increase_decimals_enabled()));
        assert!(!upd(&h, cx, |c, _w, _cx| c.decrease_decimals_enabled()));
    }

    #[gpui::test]
    fn degraded_closes_popovers_and_blocks_dispatch(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        select_single(&h, cx, 1, 1);
        // Open the text-color popover, then degrade.
        upd(&h, cx, |c, _w, cx| c.toggle_text_color_popover(cx));
        assert!(upd(&h, cx, |c, _w, _cx| c.text_color_open()));
        upd(&h, cx, |c, _w, cx| c.set_degraded(true, cx));
        // The popover is force-closed and a swatch click can no longer dispatch a command.
        assert!(!upd(&h, cx, |c, _w, _cx| c.text_color_open()));
        upd(&h, cx, |c, window, cx| {
            c.apply_text_color(Some(Rgb::from_hex(0xFF0000)), window, cx)
        });
        assert!(
            h.client.take_commands().is_empty(),
            "no SetStylePath dispatches while degraded"
        );
    }

    // ---- BUG A/B: popover item clicks APPLY (real mouse dispatch, not direct `apply_*`) -----
    //
    // These drive real mouse events through the rendered popover with a `VisualTestContext` over a
    // full-height backdrop (`tall_sheet` mounts a tall body stub so the backdrop — `size_full` of
    // the chrome — actually spans the dropdown items) — the path the part-1 anchor test and the
    // `apply_*` unit tests never exercised. Pre-fix, EVERY mouse-down inside the card reached the
    // backdrop: the menu `Button`s insert a plain (Normal) hitbox and only `prevent_default()` on
    // down (never `.occlude()`/`stop_propagation`), and the backdrop's `on_mouse_down` is not gated
    // on `default_prevented`, so a down directly on an item — as well as on the p_1/p_2 padding and
    // the gaps between rows — fired the backdrop's dismiss, tearing the popover down before the
    // item's `on_click` (mouse-UP) could dispatch. Wrapping the card in `.occlude()` inserts a
    // BlockMouse hitbox that breaks the hit-test before the backdrop for ALL in-card presses, so no
    // in-popover press can dismiss it. The mouse-DOWN is the discriminating signal (a full
    // `simulate_click` would not catch the regression: it sends down+up with no intervening repaint,
    // so the doomed button's `on_click` still fires); each per-item test below asserts the down
    // keeps the popover open — and fails without the card `.occlude()`.

    /// Opens a popover via `open`, paints, presses mouse **down** on the item registered under
    /// debug-selector `item`, asserts `open_flag` still holds (the down did not reach the backdrop
    /// dismiss — the BUG A/B guard), then releases and returns the dispatched commands.
    fn press_popover_button(
        h: &Harness,
        cx: &mut TestAppContext,
        open: impl FnOnce(&mut ChromeView, &mut Window, &mut Context<ChromeView>),
        item: &'static str,
        open_flag: impl Fn(&ChromeView) -> bool,
    ) -> Vec<Command> {
        upd(h, cx, |c, w, cx| open(c, w, cx));
        h.client.take_commands(); // drop anything incidental to opening; isolate the click
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        let center = vcx
            .debug_bounds(item)
            .unwrap_or_else(|| panic!("popover item {item:?} was not painted"))
            .center();
        let mods = gpui::Modifiers::default();
        vcx.simulate_mouse_down(center, MouseButton::Left, mods);
        let alive = vcx.update(|_w, cx| open_flag(h.chrome.read(cx)));
        assert!(
            alive,
            "popover item {item:?}: a mouse-DOWN must not dismiss the popover"
        );
        vcx.simulate_mouse_up(center, MouseButton::Left, mods);
        h.client.take_commands()
    }

    #[gpui::test]
    fn card_padding_click_keeps_popover_open(cx: &mut TestAppContext) {
        // Covers the card region a press can land on that ISN'T an item — the p_1 padding ring and
        // the gaps between rows. Like the buttons, pre-fix this reached the backdrop's dismiss
        // listener and closed the popover; the card `.occlude()` shields it too. Verified to fail
        // without the fix.
        let h = tall_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_num_fmt_popover(cx));
        h.client.take_commands();
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        let card = vcx
            .debug_bounds("numfmt-card")
            .expect("the number-format card was painted");
        // The card's top-left padding corner (inside the p_1 border, above the first menu button).
        let pad = gpui::point(card.origin.x + px(1.0), card.origin.y + px(1.0));
        vcx.simulate_mouse_down(pad, MouseButton::Left, gpui::Modifiers::default());
        assert!(
            vcx.update(|_w, cx| h.chrome.read(cx).num_fmt_open),
            "a press on the card's padding must not dismiss the popover"
        );
        assert!(
            h.client.take_commands().is_empty(),
            "a press on the card padding dispatches no command"
        );
    }

    #[gpui::test]
    fn numfmt_currency_click_applies_and_closes(cx: &mut TestAppContext) {
        let h = tall_sheet(cx);
        select_single(&h, cx, 1, 1);
        let cmds = press_popover_button(
            &h,
            cx,
            |c, _w, cx| c.toggle_num_fmt_popover(cx),
            "numfmt-Currency",
            |c| c.num_fmt_open,
        );
        assert!(
            matches!(cmds.as_slice(), [Command::SetStylePath { path: StylePath::NumFmt, value, .. }] if value == "$#,##0.00"),
            "clicking Currency must dispatch the Currency num-fmt, got {cmds:?}"
        );
        assert!(
            !upd(&h, cx, |c, _w, _cx| c.num_fmt_open),
            "the popover must close after applying"
        );
    }

    #[gpui::test]
    fn text_color_automatic_click_applies_and_closes(cx: &mut TestAppContext) {
        let h = tall_sheet(cx);
        select_single(&h, cx, 1, 1);
        let cmds = press_popover_button(
            &h,
            cx,
            |c, _w, cx| c.toggle_text_color_popover(cx),
            "text-automatic",
            |c| c.text_color_open,
        );
        assert!(
            matches!(cmds.as_slice(), [Command::SetStylePath { path: StylePath::FontColor, value, .. }] if value.is_empty()),
            "Automatic must clear the font colour (empty value), got {cmds:?}"
        );
        assert!(!upd(&h, cx, |c, _w, _cx| c.text_color_open));
    }

    #[gpui::test]
    fn fill_no_fill_click_applies_and_closes(cx: &mut TestAppContext) {
        let h = tall_sheet(cx);
        select_single(&h, cx, 1, 1);
        let cmds = press_popover_button(
            &h,
            cx,
            |c, _w, cx| c.toggle_fill_popover(cx),
            "fill-no-fill",
            |c| c.fill_open,
        );
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::SetStyleAttr {
                    attr: StyleAttr::Fill(None),
                    ..
                }]
            ),
            "No fill must clear the fill, got {cmds:?}"
        );
        assert!(!upd(&h, cx, |c, _w, _cx| c.fill_open));
    }

    #[gpui::test]
    fn fill_swatch_click_applies_and_closes(cx: &mut TestAppContext) {
        // A swatch applies on `on_mouse_down` (the backdrop also dismissed on that same down pre-fix,
        // but the swatch's own listener still ran, so the command went out either way). This is
        // positive coverage that the card `.occlude()` doesn't break a swatch's own down-to-apply. A
        // single down suffices to dispatch its command.
        let h = tall_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_fill_popover(cx));
        h.client.take_commands();
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        let center = vcx
            .debug_bounds("fill-swatch-Background 1")
            .expect("the first fill swatch was painted")
            .center();
        vcx.simulate_mouse_down(center, MouseButton::Left, gpui::Modifiers::default());
        let cmds = h.client.take_commands();
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::SetStyleAttr { attr: StyleAttr::Fill(Some(rgb)), .. }] if rgb.to_hex() == 0xFFFFFF
            ),
            "the first swatch must apply its colour, got {cmds:?}"
        );
        assert!(!upd(&h, cx, |c, _w, _cx| c.fill_open));
    }

    #[gpui::test]
    fn font_family_click_applies_and_closes(cx: &mut TestAppContext) {
        let h = tall_sheet(cx);
        select_single(&h, cx, 1, 1);
        // Item 0 is always "Default (Inter)" → clears the family override (sent as `Some("")`).
        let cmds = press_popover_button(
            &h,
            cx,
            |c, _w, cx| c.toggle_font_family_popover(cx),
            "font-family-0",
            |c| c.font_family_open,
        );
        assert!(
            matches!(cmds.as_slice(), [Command::SetFont { family: Some(f), size_pt: None, .. }] if f.is_empty()),
            "Default (Inter) must clear the font family, got {cmds:?}"
        );
        assert!(!upd(&h, cx, |c, _w, _cx| c.font_family_open));
    }

    #[gpui::test]
    fn font_size_click_applies_and_closes(cx: &mut TestAppContext) {
        let h = tall_sheet(cx);
        select_single(&h, cx, 1, 1);
        let cmds = press_popover_button(
            &h,
            cx,
            |c, _w, cx| c.toggle_font_size_popover(cx),
            "font-size-14",
            |c| c.font_size_open,
        );
        assert!(
            matches!(cmds.as_slice(), [Command::SetFont { family: None, size_pt: Some(pt), .. }] if (*pt - 14.0).abs() < 1e-6),
            "clicking 14 must set the font size to 14 pt, got {cmds:?}"
        );
        assert!(!upd(&h, cx, |c, _w, _cx| c.font_size_open));
    }

    #[gpui::test]
    fn borders_all_click_applies_and_closes(cx: &mut TestAppContext) {
        let h = tall_sheet(cx);
        select_single(&h, cx, 1, 1);
        let cmds = press_popover_button(
            &h,
            cx,
            |c, _w, cx| c.toggle_borders_popover(cx),
            "border-all",
            |c| c.borders_open,
        );
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::SetBorders {
                    preset: BorderPreset::All,
                    ..
                }]
            ),
            "clicking All must dispatch the All border preset, got {cmds:?}"
        );
        assert!(!upd(&h, cx, |c, _w, _cx| c.borders_open));
    }

    #[gpui::test]
    fn popover_backdrop_outside_click_dismisses_without_dispatch(cx: &mut TestAppContext) {
        // The occluded card must still let a click OUTSIDE it hit the backdrop → dismiss (and never
        // dispatch a command).
        let h = tall_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_num_fmt_popover(cx));
        h.client.take_commands();

        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        let card = vcx
            .debug_bounds("numfmt-card")
            .expect("the number-format card was painted");
        // A point on the backdrop but clear of the card: same top strip as the card (so it is within
        // the backdrop, which only spans the chrome height when no grid body is hosted) but far to
        // its left (the number-format trigger anchors the card on the right).
        let outside = gpui::point(px(10.0), card.origin.y + px(4.0));
        assert!(
            !card.contains(&outside),
            "test point must be outside the card, card = {card:?}"
        );
        vcx.simulate_click(outside, gpui::Modifiers::default());

        assert!(
            !upd(&h, cx, |c, _w, _cx| c.num_fmt_open),
            "a click outside the card dismisses the popover"
        );
        assert!(
            h.client.take_commands().is_empty(),
            "dismissing via the backdrop dispatches no command"
        );
    }

    #[gpui::test]
    fn popover_outside_click_removes_card_on_next_render_without_hover(cx: &mut TestAppContext) {
        // BUG B: the backdrop's dismiss closure must `cx.notify()` so the view repaints on the
        // very next frame. Without the notify the open-flag flips false but the view is never
        // marked dirty, so the popover card stays painted until some *unrelated* later event (a
        // hover/mouse-move) happens to repaint it — exactly the reported "won't close until the
        // mouse moves" symptom.
        //
        // The element-level discriminator: `debug_bounds` reads `window.rendered_frame`, which
        // only changes on an actual draw, and `simulate_event` ends in `run_until_parked`, which
        // redraws a window ONLY if something marked it dirty. So a single outside mouse-DOWN that
        // clears the flag but does not notify leaves the *previous* frame — card still present —
        // standing, with no intervening mouse-move. This asserts the card is GONE on that next
        // frame. Reverting the `cx.notify()` in `render_num_fmt_popover`'s backdrop closure makes
        // this fail (card still painted on the next render). Verified fail-without / pass-with.
        let h = tall_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_num_fmt_popover(cx));
        h.client.take_commands();

        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        let card = vcx
            .debug_bounds("numfmt-card")
            .expect("the number-format card was painted while open");
        // A point on the backdrop but clear of the card (top strip, far left of the right-anchored
        // card) — same geometry the sibling outside-click test uses.
        let outside = gpui::point(px(10.0), card.origin.y + px(4.0));
        assert!(
            !card.contains(&outside),
            "test point must be outside the card, card = {card:?}"
        );

        // A single mouse-DOWN on the backdrop, and crucially NO following mouse-move / hover.
        vcx.simulate_mouse_down(outside, MouseButton::Left, gpui::Modifiers::default());

        // The flag flipped false...
        assert!(
            !vcx.update(|_w, cx| h.chrome.read(cx).num_fmt_open),
            "the outside press must clear the open flag"
        );
        // ...AND the dismiss notified, so the view repainted on the very next frame and the card
        // element is gone — no intervening hover needed. This is the assertion that fails without
        // the `cx.notify()`.
        assert!(
            vcx.debug_bounds("numfmt-card").is_none(),
            "the popover card must be gone on the very next render (the dismiss must cx.notify)"
        );
    }

    // ---- Action row: SetBorders (preset popover) ------------------------------------------

    #[gpui::test]
    fn borders_popover_toggles(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        assert!(!upd(&h, cx, |c, _w, _cx| c.borders_open()));
        upd(&h, cx, |c, _w, cx| c.toggle_borders_popover(cx));
        assert!(upd(&h, cx, |c, _w, _cx| c.borders_open()));
        upd(&h, cx, |c, _w, cx| c.toggle_borders_popover(cx));
        assert!(!upd(&h, cx, |c, _w, _cx| c.borders_open()));
    }

    #[gpui::test]
    fn apply_borders_sends_command_over_selection(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, _w, cx| c.toggle_borders_popover(cx));
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, window, cx| {
            c.apply_borders(BorderPreset::All, window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::SetBorders {
                    preset: BorderPreset::All,
                    range,
                    line: BorderLine::ThinSolid,
                    color: None,
                    ..
                }]
                    if *range == freecell_core::CellRange::single(cell(1, 1))
            ),
            "picking a preset dispatches one SetBorders over the selection with the default \
             thin-solid-black pen, got {cmds:?}"
        );
        // Picking closes the popover.
        assert!(!upd(&h, cx, |c, _w, _cx| c.borders_open()));
    }

    #[gpui::test]
    fn borders_disabled_in_degraded_mode(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.toggle_borders_popover(cx));
        upd(&h, cx, |c, _w, cx| c.set_degraded(true, cx));
        // The popover is force-closed and a preset click can no longer dispatch.
        assert!(!upd(&h, cx, |c, _w, _cx| c.borders_open()));
        upd(&h, cx, |c, window, cx| {
            c.apply_borders(BorderPreset::Outer, window, cx)
        });
        assert!(
            h.client.take_commands().is_empty(),
            "no SetBorders dispatches while degraded"
        );
    }

    // ---- Action row: SetFont (family + size) ----------------------------------------------

    #[gpui::test]
    fn font_dropdowns_reflect_active_cell(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        h.client.set_font_family(SheetId(0), cell(1, 1), "Arial");
        h.client.set_style(
            SheetId(0),
            cell(1, 1),
            RenderStyle {
                font_size_q: 48, // 12pt
                ..Default::default()
            },
        );
        select_single(&h, cx, 1, 1);
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.font_family_label().to_string()),
            "Arial"
        );
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.font_size_label()), "12");
    }

    #[gpui::test]
    fn font_size_box_shows_workbook_default_for_default_cell(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        // A default cell (no explicit font_size_q) shows the WORKBOOK default (13pt for a new
        // workbook) — not a hardcoded "11" that would mismatch the cell (CR Moderate).
        h.client.set_default_font_size_pt(13.0);
        select_single(&h, cx, 1, 1);
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.font_size_label()), "13");

        // An opened file whose default is 10pt shows "10" for its default cells (and re-picking 10
        // is a no-op in the engine, so no size jump).
        h.client.set_default_font_size_pt(10.0);
        select_single(&h, cx, 2, 2);
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.font_size_label()), "10");
    }

    #[gpui::test]
    fn font_family_pick_and_system_default(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        select_single(&h, cx, 1, 1);

        upd(&h, cx, |c, window, cx| {
            c.apply_font_family("Times New Roman", window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::SetFont { family: Some(f), size_pt: None, .. }] if f == "Times New Roman"
            ),
            "family pick emits SetFont, got {cmds:?}"
        );

        // "Default (Inter)" clears the override (family = Some("")).
        upd(&h, cx, |c, window, cx| {
            c.apply_font_family(SYSTEM_DEFAULT_FAMILY, window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(matches!(
            cmds.as_slice(),
            [Command::SetFont { family: Some(f), size_pt: None, .. }] if f.is_empty()
        ));
    }

    #[gpui::test]
    fn font_size_pick_emits_setfont(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, window, cx| c.apply_font_size(18.0, window, cx));
        let cmds = h.client.take_commands();
        assert!(matches!(
            cmds.as_slice(),
            [Command::SetFont { family: None, size_pt: Some(pt), .. }] if (*pt - 18.0).abs() < 1e-9
        ));
    }

    #[gpui::test]
    fn font_controls_disabled_in_degraded_mode(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        select_single(&h, cx, 1, 1);
        upd(&h, cx, |c, _w, cx| c.set_degraded(true, cx));
        // A pick made while degraded dispatches nothing.
        upd(&h, cx, |c, window, cx| c.apply_font_size(24.0, window, cx));
        upd(&h, cx, |c, window, cx| {
            c.apply_font_family("Arial", window, cx)
        });
        assert!(
            h.client.take_commands().is_empty(),
            "no SetFont dispatches while degraded"
        );
    }

    // ---- Action row / data row: the two 250 ms spinners -----------------------------------

    #[gpui::test]
    fn eval_spinner_hidden_for_short_eval(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(WorkerEvent::EvalStarted, window, cx)
        });
        tick(cx, 100);
        assert!(!upd(&h, cx, |c, _w, _cx| c.eval_spinner_visible()));
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(WorkerEvent::EvalFinished, window, cx)
        });
        tick(cx, 300);
        assert!(
            !upd(&h, cx, |c, _w, _cx| c.eval_spinner_visible()),
            "a fast eval never flashes the spinner"
        );
    }

    #[gpui::test]
    fn eval_spinner_shown_for_long_eval(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(WorkerEvent::EvalStarted, window, cx)
        });
        tick(cx, 300);
        assert!(upd(&h, cx, |c, _w, _cx| c.eval_spinner_visible()));
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(WorkerEvent::EvalFinished, window, cx)
        });
        assert!(!upd(&h, cx, |c, _w, _cx| c.eval_spinner_visible()));
    }

    #[gpui::test]
    fn formula_field_spinner_only_after_250ms(cx: &mut TestAppContext) {
        // Long fetch: no reply → after 250 ms the field spinner shows, then a reply hides it.
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(1, 1)), window, cx)
        });
        assert!(!upd(&h, cx, |c, _w, _cx| c.fetch_spinner_visible()));
        tick(cx, 300);
        assert!(upd(&h, cx, |c, _w, _cx| c.fetch_spinner_visible()));
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(
                WorkerEvent::CellContent {
                    req_id: 1,
                    raw: "x".into(),
                },
                window,
                cx,
            )
        });
        assert!(!upd(&h, cx, |c, _w, _cx| c.fetch_spinner_visible()));
    }

    #[gpui::test]
    fn formula_field_spinner_never_flashes_on_fast_reply(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(1, 1)), window, cx);
            c.on_worker_event(
                WorkerEvent::CellContent {
                    req_id: 1,
                    raw: "fast".into(),
                },
                window,
                cx,
            );
        });
        tick(cx, 300);
        assert!(!upd(&h, cx, |c, _w, _cx| c.fetch_spinner_visible()));
    }

    // ---- Sheet tab bar --------------------------------------------------------------------

    fn two_sheets(cx: &mut TestAppContext) -> Harness {
        build(
            cx,
            vec![
                SheetTab::new(SheetId(0), "Sheet1"),
                SheetTab::new(SheetId(1), "Sheet2"),
            ],
            SheetId(0),
        )
    }

    #[gpui::test]
    fn add_sheet_sends_command(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, _w, _cx| c.add_sheet());
        assert!(matches!(
            h.client.take_commands().as_slice(),
            [Command::AddSheet]
        ));
    }

    #[gpui::test]
    fn select_sheet_switches_and_notifies_grid(cx: &mut TestAppContext) {
        let h = two_sheets(cx);
        upd(&h, cx, |c, window, cx| {
            c.select_sheet(SheetId(1), window, cx)
        });
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.active_sheet()), SheetId(1));
        assert!(h
            .grid_requests
            .borrow()
            .iter()
            .any(|r| matches!(r, ChromeGridRequest::SetActiveSheet(SheetId(1)))));
    }

    #[gpui::test]
    fn rename_valid_sends_command(cx: &mut TestAppContext) {
        let h = two_sheets(cx);
        upd(&h, cx, |c, window, cx| {
            c.rename_start(SheetId(0), window, cx);
            c.test_rename_type("Revenue", window, cx);
            c.commit_rename(window, cx);
        });
        let cmds = h.client.take_commands();
        assert!(
            matches!(cmds.as_slice(), [Command::RenameSheet { sheet: SheetId(0), name }] if name == "Revenue"),
            "got {cmds:?}"
        );
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.rename_target()), None);
    }

    #[gpui::test]
    fn rename_invalid_stays_editing(cx: &mut TestAppContext) {
        let h = two_sheets(cx);
        upd(&h, cx, |c, window, cx| {
            c.rename_start(SheetId(0), window, cx);
            c.test_rename_type("Sheet2", window, cx); // duplicate (case-insensitive)
            c.commit_rename(window, cx);
        });
        assert!(!h
            .client
            .take_commands()
            .iter()
            .any(|cmd| matches!(cmd, Command::RenameSheet { .. })));
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.rename_target()),
            Some(SheetId(0))
        );
        assert!(upd(&h, cx, |c, _w, _cx| c.rename_error()));
    }

    #[gpui::test]
    fn rename_escape_reverts(cx: &mut TestAppContext) {
        let h = two_sheets(cx);
        upd(&h, cx, |c, window, cx| {
            c.rename_start(SheetId(0), window, cx);
            c.test_rename_type("whatever", window, cx);
            c.cancel_rename(window, cx);
        });
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.rename_target()), None);
        assert!(!h
            .client
            .take_commands()
            .iter()
            .any(|cmd| matches!(cmd, Command::RenameSheet { .. })));
    }

    #[gpui::test]
    fn delete_last_sheet_disabled(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        assert!(!upd(&h, cx, |c, _w, _cx| c.delete_enabled()));
        upd(&h, cx, |c, _w, cx| c.request_delete(SheetId(0), cx));
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.confirm_delete_target()), None);
        assert!(h.client.take_commands().is_empty());
    }

    #[gpui::test]
    fn delete_empty_sheet_no_confirm(cx: &mut TestAppContext) {
        let h = two_sheets(cx);
        upd(&h, cx, |c, _w, cx| c.request_delete(SheetId(1), cx));
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.confirm_delete_target()), None);
        assert!(matches!(
            h.client.take_commands().as_slice(),
            [Command::DeleteSheet { sheet: SheetId(1) }]
        ));
    }

    #[gpui::test]
    fn delete_with_content_confirms_then_deletes(cx: &mut TestAppContext) {
        let h = build(
            cx,
            vec![
                SheetTab::new(SheetId(0), "Sheet1"),
                SheetTab::new(SheetId(1), "Data").with_content(true),
            ],
            SheetId(0),
        );
        upd(&h, cx, |c, _w, cx| c.request_delete(SheetId(1), cx));
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.confirm_delete_target()),
            Some(SheetId(1))
        );
        assert!(
            h.client.take_commands().is_empty(),
            "no delete before confirm"
        );
        upd(&h, cx, |c, _w, cx| c.confirm_delete(cx));
        assert!(matches!(
            h.client.take_commands().as_slice(),
            [Command::DeleteSheet { sheet: SheetId(1) }]
        ));
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.confirm_delete_target()), None);
    }

    #[gpui::test]
    fn sheets_changed_event_updates_tabs(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(
                WorkerEvent::SheetsChanged {
                    sheets: vec![
                        SheetMeta {
                            id: SheetId(0),
                            name: "Sheet1".into(),
                            has_content: false,
                        },
                        SheetMeta {
                            id: SheetId(7),
                            name: "Sheet2".into(),
                            has_content: false,
                        },
                    ],
                },
                window,
                cx,
            )
        });
        let names: Vec<String> = upd(&h, cx, |c, _w, _cx| {
            c.sheets().iter().map(|t| t.name.clone()).collect()
        });
        assert_eq!(names, vec!["Sheet1".to_string(), "Sheet2".to_string()]);
    }

    #[gpui::test]
    fn worker_input_cap_reject_flags_error(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.on_worker_event(
                WorkerEvent::EditRejected {
                    reason: freecell_engine::EditRejectedReason::InputCap(
                        freecell_core::input_cap::InputRejection::TooLong {
                            len: 9000,
                            max: MAX_INPUT_LEN,
                        },
                    ),
                },
                window,
                cx,
            );
        });
        assert!(upd(&h, cx, |c, _w, _cx| c.cap_error_visible()));
        // The worker backstop carries the rejection, so the popover message matches too.
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.cap_error_message()),
            Some("Formula too long (max 8,192 characters)".to_string())
        );
        // The next keystroke dismisses the backstop popover (`functional_spec.md §4.2`).
        upd(&h, cx, |c, window, cx| c.test_type("=1", window, cx));
        assert!(!upd(&h, cx, |c, _w, _cx| c.cap_error_visible()));
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.cap_error_message()), None);
    }

    // ---- Editing feel: type-to-replace, in-cell editor, sync, Tab, mirror ----------------

    /// The most recent edit-state push the chrome sent to the grid (mirror / in-cell / cap).
    type EditStatePush = (
        Option<(SheetId, CellRef, gpui::SharedString)>,
        Option<CellRef>,
        Option<gpui::SharedString>,
    );
    fn last_edit_state(reqs: &[ChromeGridRequest]) -> Option<EditStatePush> {
        reqs.iter().rev().find_map(|r| match r {
            ChromeGridRequest::EditState {
                mirror,
                in_cell,
                cap,
            } => Some((mirror.clone(), *in_cell, cap.clone())),
            _ => None,
        })
    }

    /// A chrome whose active cell A1 has fetched `content` (single selection, reply applied).
    fn idle_on_a1(cx: &mut TestAppContext, content: &str) -> Harness {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.on_worker_event(
                WorkerEvent::CellContent {
                    req_id: 1,
                    raw: content.into(),
                },
                window,
                cx,
            );
        });
        h.client.take_commands();
        h.grid_requests.borrow_mut().clear();
        h
    }

    #[gpui::test]
    fn type_to_replace_starts_edit_with_char(cx: &mut TestAppContext) {
        let h = idle_on_a1(cx, "old");
        upd(&h, cx, |c, window, cx| c.begin_typed("x", window, cx));
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Editing);
        assert_eq!(upd(&h, cx, |c, _w, cx| c.content_text(cx)), "x");
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.edit_origin()),
            EditOrigin::DataRow
        );
        // A live mirror of the typed char was pushed to the grid for the active cell.
        let mirror = last_edit_state(&h.grid_requests.borrow())
            .and_then(|(m, _, _)| m)
            .expect("mirror pushed while editing");
        assert_eq!(mirror.1, cell(0, 0));
        assert_eq!(mirror.2.as_ref(), "x");
    }

    #[gpui::test]
    fn type_to_replace_on_multiselect_targets_active(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(
                SelectionModel {
                    anchor: cell(1, 1),
                    active: cell(3, 3),
                },
                window,
                cx,
            );
        });
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Disabled);
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.begin_typed("5", window, cx);
            c.test_press_enter(false, window, cx);
        });
        // The commit targets the active cell of the (multi) selection.
        let cmds = h.client.take_commands();
        assert!(
            matches!(cmds.first(), Some(Command::SetCellInput { cell: cc, input, .. }) if *cc == cell(3, 3) && input == "5"),
            "expected SetCellInput at D4 with \"5\", got {cmds:?}"
        );
    }

    #[gpui::test]
    fn f2_opens_in_cell_keeping_content(cx: &mut TestAppContext) {
        let h = idle_on_a1(cx, "42");
        upd(&h, cx, |c, window, cx| {
            c.begin_in_cell(cell(0, 0), window, cx)
        });
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.incell_open()), Some(cell(0, 0)));
        assert_eq!(upd(&h, cx, |c, _w, cx| c.incell_text(cx)), "42");
        assert_eq!(upd(&h, cx, |c, _w, cx| c.content_text(cx)), "42");
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.edit_origin()),
            EditOrigin::InCell
        );
        // The grid got the in-cell overlay open on A1.
        assert_eq!(
            last_edit_state(&h.grid_requests.borrow()).and_then(|(_, ic, _)| ic),
            Some(cell(0, 0))
        );
    }

    #[gpui::test]
    fn begin_in_cell_focuses_the_in_cell_input(cx: &mut TestAppContext) {
        // BUG D (seam-level): opening the in-cell editor must focus its input so it shows a caret
        // and accepts typing. The grid-side focus-transfer *race* — where the grid re-steals focus
        // after `begin_in_cell` focuses the input — needs a real grid and is covered by the grid
        // harness test `double_click_keeps_focus_on_in_cell_input`.
        let h = idle_on_a1(cx, "42");
        let focused = upd(&h, cx, |c, window, cx| {
            c.begin_in_cell(cell(0, 0), window, cx);
            c.edit
                .in_cell()
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        });
        assert!(focused, "the in-cell input must be focused on open");
    }

    #[gpui::test]
    fn in_cell_and_data_row_stay_in_sync(cx: &mut TestAppContext) {
        let h = idle_on_a1(cx, "");
        upd(&h, cx, |c, window, cx| {
            c.begin_in_cell(cell(0, 0), window, cx);
            // Typing in the in-cell editor updates the data row.
            c.test_incell_type("=A1", window, cx);
        });
        assert_eq!(upd(&h, cx, |c, _w, cx| c.content_text(cx)), "=A1");
        assert_eq!(upd(&h, cx, |c, _w, cx| c.incell_text(cx)), "=A1");
        // Typing in the data row updates the in-cell editor (both directions, no echo loop).
        upd(&h, cx, |c, window, cx| c.test_type("=B2", window, cx));
        assert_eq!(upd(&h, cx, |c, _w, cx| c.incell_text(cx)), "=B2");
        assert_eq!(upd(&h, cx, |c, _w, cx| c.content_text(cx)), "=B2");
    }

    #[gpui::test]
    fn in_cell_enter_commits_and_moves_down(cx: &mut TestAppContext) {
        let h = idle_on_a1(cx, "");
        upd(&h, cx, |c, window, cx| {
            c.begin_in_cell(cell(0, 0), window, cx);
            c.test_incell_type("99", window, cx);
            c.test_incell_press_enter(false, window, cx);
        });
        let cmds = h.client.take_commands();
        assert!(
            matches!(cmds.as_slice(), [Command::SetCellInput { input, .. }] if input == "99"),
            "expected SetCellInput \"99\", got {cmds:?}"
        );
        assert!(h.grid_requests.borrow().iter().any(|r| matches!(
            r,
            ChromeGridRequest::MoveActive(Motion::Move(Direction::Down))
        )));
        // The overlay closed on commit.
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.incell_open()), None);
        assert_eq!(
            last_edit_state(&h.grid_requests.borrow()).and_then(|(_, ic, _)| ic),
            None
        );
    }

    #[gpui::test]
    fn in_cell_tab_commits_and_moves_right(cx: &mut TestAppContext) {
        let h = idle_on_a1(cx, "");
        upd(&h, cx, |c, window, cx| {
            c.begin_in_cell(cell(0, 0), window, cx);
            c.test_incell_type("7", window, cx);
            c.commit_incell_move(Direction::Right, window, cx);
        });
        assert!(h.grid_requests.borrow().iter().any(|r| matches!(
            r,
            ChromeGridRequest::MoveActive(Motion::Move(Direction::Right))
        )));
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.incell_open()), None);
    }

    #[gpui::test]
    fn in_cell_escape_cancels_and_reverts(cx: &mut TestAppContext) {
        let h = idle_on_a1(cx, "42");
        upd(&h, cx, |c, window, cx| {
            c.begin_in_cell(cell(0, 0), window, cx);
            c.test_incell_type("999", window, cx);
            c.cancel_incell(window, cx);
        });
        assert_eq!(upd(&h, cx, |c, _w, cx| c.content_text(cx)), "42");
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.incell_open()), None);
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Idle);
        assert!(h
            .grid_requests
            .borrow()
            .iter()
            .any(|r| matches!(r, ChromeGridRequest::FocusGrid)));
    }

    #[gpui::test]
    fn in_cell_cap_reject_keeps_editing_and_flags(cx: &mut TestAppContext) {
        let h = idle_on_a1(cx, "");
        let huge = format!("={}", "1".repeat(MAX_INPUT_LEN));
        upd(&h, cx, |c, window, cx| {
            c.begin_in_cell(cell(0, 0), window, cx);
            c.test_incell_type(&huge, window, cx);
            c.test_incell_press_enter(false, window, cx);
        });
        // No commit, still editing, overlay still open.
        assert!(!h
            .client
            .take_commands()
            .iter()
            .any(|cmd| matches!(cmd, Command::SetCellInput { .. })));
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Editing);
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.incell_open()), Some(cell(0, 0)));
        // The cap message is pushed for the in-cell popover (origin == InCell).
        let cap = last_edit_state(&h.grid_requests.borrow()).and_then(|(_, _, cap)| cap);
        assert_eq!(
            cap.as_deref(),
            Some("Formula too long (max 8,192 characters)")
        );
    }

    #[gpui::test]
    fn begin_in_cell_mid_edit_keeps_pending_text(cx: &mut TestAppContext) {
        // Type-to-replace in the data row, then F2 → the in-cell editor keeps the pending text.
        let h = idle_on_a1(cx, "old");
        upd(&h, cx, |c, window, cx| {
            c.begin_typed("x", window, cx);
            c.begin_in_cell(cell(0, 0), window, cx);
        });
        assert_eq!(upd(&h, cx, |c, _w, cx| c.incell_text(cx)), "x");
        assert_eq!(upd(&h, cx, |c, _w, cx| c.content_text(cx)), "x");
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.edit_origin()),
            EditOrigin::InCell
        );
    }

    #[gpui::test]
    fn data_row_tab_commits_and_moves_right(cx: &mut TestAppContext) {
        let h = idle_on_a1(cx, "");
        upd(&h, cx, |c, window, cx| {
            c.test_type("=1", window, cx);
            c.test_data_row_tab(false, window, cx);
        });
        let cmds = h.client.take_commands();
        assert!(matches!(cmds.as_slice(), [Command::SetCellInput { input, .. }] if input == "=1"));
        assert!(h.grid_requests.borrow().iter().any(|r| matches!(
            r,
            ChromeGridRequest::MoveActive(Motion::Move(Direction::Right))
        )));
    }

    #[gpui::test]
    fn data_row_shift_tab_moves_left(cx: &mut TestAppContext) {
        let h = idle_on_a1(cx, "");
        upd(&h, cx, |c, window, cx| {
            c.test_type("=1", window, cx);
            c.test_data_row_tab(true, window, cx);
        });
        assert!(h.grid_requests.borrow().iter().any(|r| matches!(
            r,
            ChromeGridRequest::MoveActive(Motion::Move(Direction::Left))
        )));
    }

    #[gpui::test]
    fn mirror_cleared_on_commit(cx: &mut TestAppContext) {
        let h = idle_on_a1(cx, "");
        upd(&h, cx, |c, window, cx| {
            c.test_type("=1", window, cx);
        });
        // Mirror present while editing.
        assert!(last_edit_state(&h.grid_requests.borrow())
            .and_then(|(m, _, _)| m)
            .is_some());
        h.grid_requests.borrow_mut().clear();
        upd(&h, cx, |c, window, cx| {
            c.test_press_enter(false, window, cx)
        });
        // Cleared on commit.
        assert_eq!(
            last_edit_state(&h.grid_requests.borrow()).and_then(|(m, _, _)| m),
            None
        );
    }

    #[gpui::test]
    fn double_click_reselect_keeps_content(cx: &mut TestAppContext) {
        // Replays the real double-click chrome-level order: the second mousedown re-emits
        // SelectionChanged for the already-selected cell (restarting the fetch + clearing the
        // field) BEFORE OpenInCellEditor. The in-cell editor must still show the cell's real
        // content ("42"), not the just-cleared field (review Critical #1 — data-loss guard).
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.on_worker_event(
                WorkerEvent::CellContent {
                    req_id: 1,
                    raw: "42".into(),
                },
                window,
                cx,
            );
            // Redundant re-select (the grid also elides this now, but the chrome must be robust).
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.begin_in_cell(cell(0, 0), window, cx);
        });
        assert_eq!(upd(&h, cx, |c, _w, cx| c.incell_text(cx)), "42");
        assert_eq!(upd(&h, cx, |c, _w, cx| c.content_text(cx)), "42");
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Editing);
    }

    #[gpui::test]
    fn begin_in_cell_ignored_while_other_cell_editing(cx: &mut TestAppContext) {
        // A cap-rejected/deferred-revert click-away leaves the reducer + selection on the OLD cell;
        // opening the in-cell editor on a DIFFERENT cell must no-op (review Moderate #2).
        let h = idle_on_a1(cx, "");
        upd(&h, cx, |c, window, cx| {
            c.begin_typed("x", window, cx); // editing A1 (the active cell)
            c.begin_in_cell(cell(1, 1), window, cx); // a divergent cell
        });
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.incell_open()),
            None,
            "overlay must not relocate onto a cell the edit isn't on"
        );
        assert_eq!(upd(&h, cx, |c, _w, cx| c.content_text(cx)), "x");
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Editing);
    }

    #[gpui::test]
    fn in_cell_opens_empty_while_fetch_pending_then_populates(cx: &mut TestAppContext) {
        // F2 before the content reply arrives: the overlay opens empty (no forced empty edit), and
        // the in-flight reply promotes it once it lands (empty-with-spinner intent, review #3).
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.begin_in_cell(cell(0, 0), window, cx); // reply not yet delivered
        });
        assert_eq!(upd(&h, cx, |c, _w, cx| c.incell_text(cx)), "");
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.incell_open()), Some(cell(0, 0)));
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.data_mode()),
            FieldMode::Idle,
            "no empty edit forced while the fetch is still in flight"
        );
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(
                WorkerEvent::CellContent {
                    req_id: 1,
                    raw: "hello".into(),
                },
                window,
                cx,
            );
        });
        assert_eq!(upd(&h, cx, |c, _w, cx| c.incell_text(cx)), "hello");
        assert_eq!(upd(&h, cx, |c, _w, cx| c.content_text(cx)), "hello");
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Editing);
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.incell_open()), Some(cell(0, 0)));
    }

    #[gpui::test]
    fn double_click_cross_cell_pending_fetch_opens_empty(cx: &mut TestAppContext) {
        // Select non-empty A1 (reply lands), then B2 whose fetch is still in flight, then open the
        // in-cell editor on B2. It must NOT seed A1's stale committed "42" (the reducer keeps A1's
        // `committed` across the single→single switch) — it opens empty, and B2's reply populates
        // it (review New Critical — cross-cell data-corruption guard).
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.on_worker_event(
                WorkerEvent::CellContent {
                    req_id: 1,
                    raw: "42".into(),
                },
                window,
                cx,
            );
            c.on_selection_changed(SelectionModel::single(cell(1, 1)), window, cx); // B2, no reply
            c.begin_in_cell(cell(1, 1), window, cx);
        });
        assert_eq!(
            upd(&h, cx, |c, _w, cx| c.incell_text(cx)),
            "",
            "must not seed the previous cell's stale content"
        );
        assert_eq!(upd(&h, cx, |c, _w, cx| c.content_text(cx)), "");
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Idle);
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.incell_open()), Some(cell(1, 1)));
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(
                WorkerEvent::CellContent {
                    req_id: 2,
                    raw: "world".into(),
                },
                window,
                cx,
            );
        });
        assert_eq!(upd(&h, cx, |c, _w, cx| c.incell_text(cx)), "world");
        assert_eq!(upd(&h, cx, |c, _w, cx| c.content_text(cx)), "world");
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Editing);
    }

    #[gpui::test]
    fn multiselect_collapse_open_does_not_seed_stale(cx: &mut TestAppContext) {
        // A1 reply "42" tags the seed. A range multi-select clears `committed` and resets the tag.
        // Collapsing back to A1 (fetch in flight) and opening the in-cell editor must NOT seed the
        // just-cleared empty content — it opens empty, and A1's reply repopulates it (New Critical
        // path #1: multi-select clears committed but the bare tag used to survive).
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.on_worker_event(
                WorkerEvent::CellContent {
                    req_id: 1,
                    raw: "42".into(),
                },
                window,
                cx,
            );
            // A range selection → multi → the reducer clears `committed`.
            c.on_selection_changed(
                SelectionModel {
                    anchor: cell(0, 0),
                    active: cell(2, 2),
                },
                window,
                cx,
            );
            // Collapse back to A1 → a fresh fetch (req 2) is in flight, `committed` still "".
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.begin_in_cell(cell(0, 0), window, cx);
        });
        assert_eq!(
            upd(&h, cx, |c, _w, cx| c.incell_text(cx)),
            "",
            "must not seed the just-cleared committed content"
        );
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Idle);
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.incell_open()), Some(cell(0, 0)));
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(
                WorkerEvent::CellContent {
                    req_id: 2,
                    raw: "42".into(),
                },
                window,
                cx,
            );
        });
        assert_eq!(upd(&h, cx, |c, _w, cx| c.incell_text(cx)), "42");
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Editing);
    }

    #[gpui::test]
    fn sheet_switch_open_does_not_seed_other_sheet(cx: &mut TestAppContext) {
        // Sheet1!A1 reply lands (tag = (Sheet1, A1)). Switch to Sheet2 and open the in-cell editor
        // on Sheet2!A1 (the default landing cell, same CellRef) before its fetch replies — it must
        // NOT seed Sheet1's content across sheets (New Critical path #2: the bare tag ignored the
        // sheet). Opens empty; Sheet2's reply promotes with the right content.
        let h = two_sheets(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.on_worker_event(
                WorkerEvent::CellContent {
                    req_id: 1,
                    raw: "sheet1A1".into(),
                },
                window,
                cx,
            );
            // Switch to Sheet2 (window-driven adopt), then select its A1 (fetch req 2 in flight).
            c.adopt_active_sheet(SheetId(1), cx);
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.begin_in_cell(cell(0, 0), window, cx);
        });
        assert_eq!(
            upd(&h, cx, |c, _w, cx| c.incell_text(cx)),
            "",
            "must not seed another sheet's content"
        );
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Idle);
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(
                WorkerEvent::CellContent {
                    req_id: 2,
                    raw: "sheet2A1".into(),
                },
                window,
                cx,
            );
        });
        assert_eq!(upd(&h, cx, |c, _w, cx| c.incell_text(cx)), "sheet2A1");
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Editing);
    }

    #[gpui::test]
    fn commit_retags_so_reopen_other_cell_does_not_seed_committed(cx: &mut TestAppContext) {
        // The commit paths overwrite the reducer's `committed` with the EDITED cell's content; the
        // seed tag must move with it (New Critical — commit-path stale seed). Repro: land A1="Zval",
        // type-to-replace B1="x", click-away commit of B1, then reopen A1 before its re-fetch reply.
        // The A1 editor must NOT show B1's "x"; it opens empty and A1's reply repopulates "Zval".
        let h = one_sheet(cx);
        let b1 = cell(0, 1);
        upd(&h, cx, |c, window, cx| {
            // 1. A1 reply lands → tag = (s, A1).
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.on_worker_event(
                WorkerEvent::CellContent {
                    req_id: 1,
                    raw: "Zval".into(),
                },
                window,
                cx,
            );
            // 2. Move to B1, type-to-replace "x"; B1's reply arrives mid-edit and is dropped.
            c.on_selection_changed(SelectionModel::single(b1), window, cx);
            c.begin_typed("x", window, cx);
            c.on_worker_event(
                WorkerEvent::CellContent {
                    req_id: 2,
                    raw: "Bval".into(),
                },
                window,
                cx,
            );
        });
        // 3. Click-away commit of B1 (the tag must move to B1 here — selection.active is still B1).
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.on_edit_commit_requested(window, cx);
        });
        let cmds = h.client.take_commands();
        assert!(
            matches!(cmds.as_slice(), [Command::SetCellInput { cell: cc, input, .. }] if *cc == b1 && input == "x"),
            "B1 must receive the committed \"x\", got {cmds:?}"
        );
        // Select A1 (its re-fetch req 3 is in flight), then reopen the in-cell editor on A1.
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.begin_in_cell(cell(0, 0), window, cx);
        });
        assert_ne!(
            upd(&h, cx, |c, _w, cx| c.incell_text(cx)),
            "x",
            "A1 must not seed B1's just-committed content"
        );
        assert_eq!(upd(&h, cx, |c, _w, cx| c.incell_text(cx)), "");
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Idle);
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.incell_open()), Some(cell(0, 0)));
        // 4. A1's real reply (req 3) promotes the overlay with A1's content.
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(
                WorkerEvent::CellContent {
                    req_id: 3,
                    raw: "Zval".into(),
                },
                window,
                cx,
            );
        });
        assert_eq!(upd(&h, cx, |c, _w, cx| c.incell_text(cx)), "Zval");
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Editing);
    }

    #[gpui::test]
    fn focus_flip_clears_incell_cap_push(cx: &mut TestAppContext) {
        // After an in-cell cap reject (grid shows the popover), flipping focus to the data row must
        // clear the in-cell cap push so only one popover shows (review Mild #4).
        let h = idle_on_a1(cx, "");
        let huge = format!("={}", "1".repeat(MAX_INPUT_LEN));
        upd(&h, cx, |c, window, cx| {
            c.begin_in_cell(cell(0, 0), window, cx);
            c.test_incell_type(&huge, window, cx);
            c.test_incell_press_enter(false, window, cx);
        });
        assert!(last_edit_state(&h.grid_requests.borrow())
            .and_then(|(_, _, cap)| cap)
            .is_some());
        h.grid_requests.borrow_mut().clear();
        upd(&h, cx, |c, window, cx| {
            let handle = c.content_input.clone();
            c.on_content_event(&handle, &InputEvent::Focus, window, cx);
        });
        assert_eq!(
            last_edit_state(&h.grid_requests.borrow()).and_then(|(_, _, cap)| cap),
            None,
            "the in-cell cap popover clears when focus flips to the data row"
        );
    }
}
