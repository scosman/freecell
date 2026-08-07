//! Charts as the chrome sees them (P17–P20, `ui_design.md §4`): the action-row chart-insert
//! menu, and the right-docked chart **edit panel** — its type/range controls and its P20
//! chrome (title, legend, axis titles, series colours, data labels).
//!
//! This is the chart *panel*, not chart rendering; the plotted chart itself is
//! [`crate::chart`]. Moved verbatim out of the single-file `chrome/view.rs`
//! (`specs/projects/chrome-view-split`).

use super::*;

use freecell_chart_model::{Anchor as ChartAnchor, AnchorCell, LegendPosition};
use freecell_core::limits;
use freecell_engine::{ChartAxisKind, ChartChromeEdit, DataLabelToggles};

/// The default footprint (in cells) of a chart inserted from the action bar — a typical Excel
/// default chart size (~8 columns × 15 rows), anchored at the active cell (`ui_design §3.1`).
const CHART_INSERT_COLS: u32 = 8;
const CHART_INSERT_ROWS: u32 = 15;

/// The action-bar chart-insert menu entries — `(kind, icon path, label)`, in menu order
/// (`ui_design §3.1`). Every [`ChartInsertKind`] authors a near-empty chart of that type (bubble
/// landed as the final type in P26). Order matches how Excel groups the types: **Area sits right
/// after Line** (both are trend charts) before the Column/Bar pair (post-v1 Batch 2, item 13). This
/// is the single canonical order shared by the action-bar dropdown ([`render_chart_menu`]) and the
/// edit panel's Type row ([`render_chart_type_row`]).
const CHART_MENU: [(ChartInsertKind, &str, &str); 8] = [
    (ChartInsertKind::Line, "icons/chart-line.svg", "Line"),
    (ChartInsertKind::Area, "icons/chart-area.svg", "Area"),
    (ChartInsertKind::Column, "icons/chart-column.svg", "Column"),
    (ChartInsertKind::Bar, "icons/chart-bar.svg", "Bar"),
    (ChartInsertKind::Pie, "icons/chart-pie.svg", "Pie"),
    (
        ChartInsertKind::Doughnut,
        "icons/chart-doughnut.svg",
        "Doughnut",
    ),
    (
        ChartInsertKind::Scatter,
        "icons/chart-scatter.svg",
        "Scatter",
    ),
    (ChartInsertKind::Bubble, "icons/chart-bubble.svg", "Bubble"),
];

/// One series in the chart **edit panel** — its display name + current color, for the per-series
/// color swatch row (P20).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChartPanelSeries {
    pub name: String,
    /// The series' current explicit color (`None` = the renderer's palette pick), for the highlighted
    /// swatch.
    pub color: Option<Rgb>,
}

/// The right-docked chart **edit panel**'s state (P19 skeleton + P20 chrome) — the chart it is
/// shaping. `kind`/`ranges` drive the (authored-only) Type + Data-range sections; the chrome fields
/// (`title`, `legend`, axis titles, `series`, `labels`) drive the P20 controls. All are updated
/// optimistically on an edit, then reconciled by the window from the republished snapshot.
#[derive(Clone, Debug, PartialEq)]
pub struct ChartPanel {
    /// The chart's host sheet (the `SetChart*` target).
    pub sheet: SheetId,
    /// The chart's stable id.
    pub id: ChartId,
    /// Whether the chart was authored in-app — authored charts expose the Type + Data-range controls
    /// (loaded re-type/re-range is not P20; the worker ignores those for loaded ids). Both provenances
    /// expose the chrome controls.
    pub is_authored: bool,
    /// The chart's current type (for the highlighted type glyph).
    pub kind: ChartInsertKind,
    /// A short summary of the current bound data range(s), `None` if unset.
    pub ranges: Option<String>,
    /// The current chart title (`None` = no title).
    pub title: Option<String>,
    /// The current legend position (`None` = legend off).
    pub legend: Option<LegendPosition>,
    /// The current category / value axis titles (`None` = untitled).
    pub cat_axis_title: Option<String>,
    pub val_axis_title: Option<String>,
    /// One entry per series (name + current color), for the color swatch rows.
    pub series: Vec<ChartPanelSeries>,
    /// The chart's current data-label toggles (read from its first series).
    pub labels: DataLabelToggles,
}

impl ChartPanel {
    /// A panel for `chart` with the chrome fields defaulted — a convenience for tests + the
    /// near-empty authored insert case (the window fills the chrome from the snapshot).
    #[cfg(test)]
    pub fn skeleton(sheet: SheetId, id: ChartId, is_authored: bool, kind: ChartInsertKind) -> Self {
        Self {
            sheet,
            id,
            is_authored,
            kind,
            ranges: None,
            title: None,
            legend: None,
            cat_axis_title: None,
            val_axis_title: None,
            series: Vec::new(),
            labels: DataLabelToggles::default(),
        }
    }
}

impl ChromeView {
    // ---- Action row: insert chart (P17) ---------------------------------------------------

    pub(super) fn toggle_chart_menu(&mut self, cx: &mut Context<Self>) {
        self.chart_menu_open = !self.chart_menu_open;
        cx.notify();
    }

    /// Inserts a **near-empty authored chart** of `kind` onto the active sheet, anchored at the
    /// active cell (`ui_design §3.1`). This is a **mutating action-row control**, so it follows the
    /// same contract as every sibling (toggle style, fill, text color, decimals, font, borders): it
    /// closes the menu, then **commits any pending in-cell edit first and bails if the commit is
    /// blocked** (a cap-rejected edit stays editing), so the worker's subsequent publish + grid
    /// refresh can't clobber a dangling uncommitted cell edit. Degraded-guarded (a backstop to the
    /// disabled trigger). Fire-and-forget: the worker holds the authored chart + republishes it.
    pub fn insert_chart(
        &mut self,
        kind: ChartInsertKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.chart_menu_open = false;
        if self.degraded {
            return;
        }
        if !self.commit_pending_edit(window, cx) {
            return; // an invalid pending edit blocks the insert, keeping the field editing
        }
        // Post-v1 Batch 3, item 8: seed the new chart's data range from the current selection when it
        // is a **real range** (more than one cell) — the worker binds it at creation, so the chart is
        // born as a live chart of the chosen type, no follow-up "Use selection" click. A single-cell
        // (or trivial) selection passes `None`, keeping the near-empty placeholder behavior. Reuses
        // the P19 `SetChartRange` binding (`series_refs_from_block`), now on the freshly-inserted id.
        let data = (!self.selection.is_single()).then(|| self.selection.range());
        self.client.send(Command::InsertChart {
            sheet: self.active_sheet,
            kind,
            anchor: self.default_chart_anchor(),
            data,
        });
        cx.notify();
    }

    /// A default chart placement: the [`CHART_INSERT_COLS`]×[`CHART_INSERT_ROWS`] rectangle from the
    /// active cell, clamped to the sheet edge so the anchor stays on-sheet.
    fn default_chart_anchor(&self) -> ChartAnchor {
        let active = self.selection.active;
        let to_col = active
            .col
            .saturating_add(CHART_INSERT_COLS)
            .min(limits::MAX_COLS - 1);
        let to_row = active
            .row
            .saturating_add(CHART_INSERT_ROWS)
            .min(limits::MAX_ROWS - 1);
        ChartAnchor::new(
            AnchorCell::new(active.col, active.row),
            AnchorCell::new(to_col, to_row),
        )
    }

    /// Whether the chart-insert menu is open (test/render introspection).
    pub fn chart_menu_open(&self) -> bool {
        self.chart_menu_open
    }

    // ---- Chart edit panel (P19 skeleton + P20 chrome) -------------------------------------

    /// Open (or re-point / reconcile) the right-docked chart **edit panel** on `panel`'s chart
    /// (`ui_design §4`). The window calls this when a chart is selected or freshly inserted, and again
    /// on each republish to reconcile the shown state with the worker's snapshot. The text inputs
    /// (title + axis titles) are seeded **only when the panel's chart id changes** — never on a live
    /// republish of the same chart — so a republish (e.g. a source-cell edit re-resolving the chart)
    /// never clobbers an in-progress panel edit.
    pub fn open_chart_panel(
        &mut self,
        panel: ChartPanel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The chart panel and the CF sidebar share the right dock — opening one closes the other
        // (`components/cf_sidebar.md §2`). A no-op when the CF sidebar is already closed (the common
        // case: this also runs to reconcile an already-open chart panel on republish).
        self.close_cond_fmt(cx);
        let new_chart = self.chart_panel.as_ref().map(|p| p.id) != Some(panel.id);
        if new_chart {
            self.seed_chart_input(
                &self.chart_title_input.clone(),
                panel.title.clone(),
                window,
                cx,
            );
            self.seed_chart_input(
                &self.chart_cat_axis_input.clone(),
                panel.cat_axis_title.clone(),
                window,
                cx,
            );
            self.seed_chart_input(
                &self.chart_val_axis_input.clone(),
                panel.val_axis_title.clone(),
                window,
                cx,
            );
        }
        self.chart_panel = Some(panel);
        cx.notify();
    }

    /// Seed a chart-panel text input with the chart's current value (`set_value` suppresses the
    /// widget's change event, so seeding never triggers a spurious commit).
    fn seed_chart_input(
        &self,
        input: &Entity<InputState>,
        value: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        input.update(cx, |i, cx| {
            i.set_value(value.unwrap_or_default(), window, cx)
        });
    }

    /// Close the chart edit panel (its × button, the chart's deletion, or a degrade).
    pub fn close_chart_panel(&mut self, cx: &mut Context<Self>) {
        if self.chart_panel.take().is_some() {
            cx.notify();
        }
    }

    /// The chart the edit panel is currently shaping, if any (window introspection: refresh / close).
    pub fn chart_panel_target(&self) -> Option<ChartId> {
        self.chart_panel.as_ref().map(|p| p.id)
    }

    /// Switch the panel's chart to `kind` (P19). A mutating chart control — like `insert_chart` it
    /// commits any pending in-cell edit first (bailing if blocked) and degrade-guards. Updates the
    /// panel's shown `kind` optimistically; the worker republishes the reshaped chart and the window
    /// reconciles.
    pub fn set_chart_type_from_panel(
        &mut self,
        kind: ChartInsertKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(panel) = self.chart_panel.as_ref() else {
            return;
        };
        let (sheet, id) = (panel.sheet, panel.id);
        if self.degraded {
            return;
        }
        if !self.commit_pending_edit(window, cx) {
            return;
        }
        self.client.send(Command::SetChartType { sheet, id, kind });
        if let Some(panel) = self.chart_panel.as_mut() {
            panel.kind = kind;
        }
        cx.notify();
    }

    /// Bind the panel's chart to the **current grid selection** as its data range (P19). The skeleton
    /// range-picker: the user selects the data block in the grid, then applies it here. Commits any
    /// pending edit first + degrade-guards, then sends `SetChartRange` for the current selection
    /// rectangle; the worker re-resolves the chart live + republishes.
    ///
    /// The command's `sheet` is the **active** sheet — the one the selection's coordinates live in —
    /// **not** the chart's host sheet: the worker finds the chart by `id` and qualifies the emitted
    /// `c:f` with `sheet`, so pairing the selection with its own sheet is what keeps the binding
    /// honest (and enables valid cross-sheet data, e.g. a chart on Sheet1 bound to Sheet2's cells).
    /// Every other range command in the chrome pairs `self.selection.range()` with `self.active_sheet`.
    pub fn apply_chart_range_from_selection(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(panel) = self.chart_panel.as_ref() else {
            return;
        };
        let id = panel.id;
        if self.degraded {
            return;
        }
        if !self.commit_pending_edit(window, cx) {
            return;
        }
        self.client.send(Command::SetChartRange {
            sheet: self.active_sheet,
            id,
            data: self.selection.range(),
        });
        cx.notify();
    }

    // ---- Chart edit panel: chrome (P20) ---------------------------------------------------

    /// Send one chrome edit for the panel's chart, on either provenance (`Command::SetChartChrome`).
    /// A mutating chart control — like every panel/action-row control it degrade-guards and commits
    /// any pending in-cell edit first (bailing if blocked), then hands the panel to `update_panel` to
    /// reflect the change optimistically (the window reconciles from the republished snapshot).
    fn send_chart_chrome(
        &mut self,
        edit: ChartChromeEdit,
        update_panel: impl FnOnce(&mut ChartPanel),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(panel) = self.chart_panel.as_ref() else {
            return;
        };
        let (sheet, id) = (panel.sheet, panel.id);
        if self.degraded {
            return;
        }
        if !self.commit_pending_edit(window, cx) {
            return;
        }
        self.client
            .send(Command::SetChartChrome { sheet, id, edit });
        if let Some(panel) = self.chart_panel.as_mut() {
            update_panel(panel);
        }
        cx.notify();
    }

    /// Set (or clear, `None`) the chart title (P20).
    pub fn set_chart_title_from_panel(
        &mut self,
        title: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let edit = ChartChromeEdit::Title(title.clone());
        self.send_chart_chrome(edit, |p| p.title = title, window, cx);
    }

    /// Turn the legend on at `position`, or off (`None`) (P20).
    pub fn set_chart_legend_from_panel(
        &mut self,
        position: Option<LegendPosition>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let edit = ChartChromeEdit::Legend(position);
        self.send_chart_chrome(edit, |p| p.legend = position, window, cx);
    }

    /// Set (or clear, `None`) an axis title (P20).
    pub fn set_chart_axis_title_from_panel(
        &mut self,
        axis: ChartAxisKind,
        title: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let edit = ChartChromeEdit::AxisTitle {
            axis,
            title: title.clone(),
        };
        self.send_chart_chrome(
            edit,
            |p| match axis {
                ChartAxisKind::Category => p.cat_axis_title = title,
                ChartAxisKind::Value => p.val_axis_title = title,
            },
            window,
            cx,
        );
    }

    /// Set (or clear back to the palette, `None`) one series' color (P20).
    pub fn set_chart_series_color_from_panel(
        &mut self,
        series: usize,
        color: Option<Rgb>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let edit = ChartChromeEdit::SeriesColor { series, color };
        self.send_chart_chrome(
            edit,
            |p| {
                if let Some(s) = p.series.get_mut(series) {
                    s.color = color;
                }
            },
            window,
            cx,
        );
    }

    /// Apply the data-label toggles across the chart's series (P20).
    pub fn set_chart_data_labels_from_panel(
        &mut self,
        labels: DataLabelToggles,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let edit = ChartChromeEdit::DataLabels(labels);
        self.send_chart_chrome(edit, |p| p.labels = labels, window, cx);
    }

    /// The title-input's current value as a chart title (`None` for empty/blank).
    fn chart_input_title(input: &Entity<InputState>, cx: &Context<Self>) -> Option<String> {
        let text = input.read(cx).value().to_string();
        (!text.trim().is_empty()).then_some(text)
    }

    /// The panel's current target `(sheet, id)` — the key the input-focus staleness guard compares.
    fn chart_panel_key(&self) -> Option<(SheetId, ChartId)> {
        self.chart_panel.as_ref().map(|p| (p.sheet, p.id))
    }

    /// Classify a chart-input event into the shared handling: capture the target on `Focus`, or
    /// return `true` to **commit** the field's current text as a live chrome edit.
    ///
    /// The title + axis-title fields commit **live, per keystroke** (`Change`) so the chart updates
    /// as the user types (post-v1 Batch 2, item 6) — not only on Enter/blur. `Change` reads the panel
    /// *synchronously* with the keystroke, so it always targets the chart currently shown. Enter/blur
    /// remain commit points too (a redundant safety net once live commits have already synced).
    ///
    /// Every commit is guarded against the **cross-chart clobber**: it fires only if the panel still
    /// points at the chart the field was focused for. A `Change` keeps the captured focus (typing
    /// continues); Enter/blur consume it (`take`). Seeding uses `set_value`, which suppresses `Change`
    /// (`InputState::emit_events`), so re-seeding a field on a chart switch never emits a spurious
    /// live edit — and a live republish of the *same* chart never re-seeds (only an id change does,
    /// [`open_chart_panel`]), so it can't clobber in-progress typing.
    fn chart_input_should_commit(&mut self, event: &InputEvent) -> bool {
        match event {
            InputEvent::Focus => {
                self.chart_input_focus = self.chart_panel_key();
                false
            }
            // Live per-keystroke commit: keep the focus capture (more keystrokes will follow) and
            // commit only while the panel still points at the focused chart.
            InputEvent::Change => match self.chart_input_focus {
                Some(focused) => self.chart_panel_key() == Some(focused),
                // No captured focus (e.g. a direct test call) → allow if a panel is open.
                None => self.chart_panel_key().is_some(),
            },
            InputEvent::PressEnter { .. } | InputEvent::Blur => match self.chart_input_focus.take()
            {
                // Commit only if the panel still points at the chart the field was focused for; a
                // re-point (or a closed panel) since focus drops the stale text.
                Some(focused) => self.chart_panel_key() == Some(focused),
                // No captured focus (e.g. a direct call) → allow (send_chart_chrome still guards).
                None => true,
            },
        }
    }

    /// Commit the chart title input live per keystroke (`Change`), and on Enter / blur — only when it
    /// differs from the panel's current title (so a redundant event doesn't re-send the same value)
    /// and the panel hasn't been re-pointed since focus (the staleness guard).
    pub(super) fn on_chart_title_event(
        &mut self,
        _input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.chart_input_should_commit(event) {
            return;
        }
        let title = Self::chart_input_title(&self.chart_title_input.clone(), cx);
        if self.chart_panel.as_ref().map(|p| p.title.clone()) != Some(title.clone()) {
            self.set_chart_title_from_panel(title, window, cx);
        }
    }

    pub(super) fn on_chart_cat_axis_event(
        &mut self,
        _input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.chart_input_should_commit(event) {
            return;
        }
        let title = Self::chart_input_title(&self.chart_cat_axis_input.clone(), cx);
        if self.chart_panel.as_ref().map(|p| p.cat_axis_title.clone()) != Some(title.clone()) {
            self.set_chart_axis_title_from_panel(ChartAxisKind::Category, title, window, cx);
        }
    }

    pub(super) fn on_chart_val_axis_event(
        &mut self,
        _input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.chart_input_should_commit(event) {
            return;
        }
        let title = Self::chart_input_title(&self.chart_val_axis_input.clone(), cx);
        if self.chart_panel.as_ref().map(|p| p.val_axis_title.clone()) != Some(title.clone()) {
            self.set_chart_axis_title_from_panel(ChartAxisKind::Value, title, window, cx);
        }
    }

    /// The chart-insert menu (P17, `ui_design.md §3.1`): a small panel of chart-type glyphs; picking
    /// one inserts a near-empty authored chart of that type. Same backdrop/occlude/anchor pattern as
    /// the number-format dropdown.
    pub(super) fn render_chart_menu(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        // `items_start` keeps each entry content-sized (not stretched to the widest row), so its
        // icon + label pack at the LEFT edge instead of being centered in a full-width button
        // (post-v1 Batch 3, item 14: left-align the dropdown items like a normal menu). Without it
        // the flex column's default `stretch` widens every button to "Doughnut" and the inner
        // label flex (which gpui-component hardcodes to `justify_center`) centers the glyph + text.
        let mut menu = div().flex().flex_col().items_start().gap(px(2.0));
        for (kind, icon_path, label) in CHART_MENU {
            menu = menu.child(
                Button::new(gpui::ElementId::Name(format!("chart-{label}").into()))
                    .icon(Icon::empty().path(icon_path))
                    .label(label)
                    .debug_selector(move || format!("chart-menu-{label}"))
                    .ghost()
                    .small()
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.insert_chart(kind, window, cx);
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
                        this.chart_menu_open = false;
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
                    .left(px(self.anchor_x[Anchor::Chart.idx()]))
                    // Occlude the card so item clicks don't trip the backdrop dismiss (BUG A/B).
                    .occlude()
                    .debug_selector(|| "chart-menu-card".into())
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

    /// The right-docked chart **edit panel** (P19, `ui_design §4`): a floating card on the right side
    /// of the sheet with a **Type** row (the `CHART_MENU` glyphs, current kind highlighted) and a
    /// **Data range** section ("use the current selection" applies it as the chart's range). It is a
    /// chrome overlay (no pixel baseline), not a popover on the chart. It closes on its × button, on
    /// **click-away** (a click on a cell / header / empty grid, routed via `on_selection_changed` —
    /// post-v1 Batch 2, item 12), on the chart's deletion, or on a degrade; clicking **another chart**
    /// re-points the panel to it (a switch, not a close). Its body scrolls + clips to its own bounds
    /// (item 7).
    pub(super) fn render_chart_panel(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let panel = self
            .chart_panel
            .as_ref()
            .expect("render_chart_panel only runs while the panel is open");

        // The scrollable body sections (the docked-sidebar header stays pinned above them, so the ×
        // is always reachable — post-v1 Batch 2, item 7).
        let mut sections: Vec<gpui::AnyElement> = Vec::new();

        // Type + Data range — authored charts only (loaded re-type/re-range is not P20).
        if panel.is_authored {
            sections.push(
                section("Type", self.render_chart_type_row(panel.kind, cx)).into_any_element(),
            );
            sections.push(
                section("Data range", self.render_chart_range_body(panel, cx)).into_any_element(),
            );
        }

        // Title (a committed-on-Enter/blur text input).
        sections.push(
            section(
                "Title",
                Input::new(&self.chart_title_input).small().w_full(),
            )
            .into_any_element(),
        );

        // Legend on/off + position.
        sections.push(
            section("Legend", self.render_chart_legend_row(panel.legend, cx)).into_any_element(),
        );

        // Axis titles.
        sections.push(
            section(
                "Axis titles",
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(Input::new(&self.chart_cat_axis_input).small().w_full())
                    .child(Input::new(&self.chart_val_axis_input).small().w_full()),
            )
            .into_any_element(),
        );

        // Series colors.
        if !panel.series.is_empty() {
            sections.push(
                section("Series colors", self.render_chart_series_colors(panel, cx))
                    .into_any_element(),
            );
        }

        // Data-label toggles.
        sections.push(
            section(
                "Data labels",
                self.render_chart_data_labels(panel.labels, cx),
            )
            .into_any_element(),
        );

        let body = div().flex().flex_col().gap_3().children(sections);

        docked_sidebar(
            "chart-panel",
            "Edit chart",
            cx.listener(|this, _: &ClickEvent, _window, cx| {
                this.close_chart_panel(cx);
            }),
            body,
        )
        .into_any_element()
    }

    /// The Type row: one glyph button per authorable kind, the current one selected (authored only).
    fn render_chart_type_row(
        &self,
        current: ChartInsertKind,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let mut type_row = div().flex().flex_wrap().gap(px(2.0));
        for (kind, icon_path, label) in CHART_MENU {
            type_row = type_row.child(
                Button::new(gpui::ElementId::Name(
                    format!("chart-panel-type-{label}").into(),
                ))
                .icon(Icon::empty().path(icon_path))
                .tooltip(label)
                .debug_selector(move || format!("chart-panel-type-{label}"))
                .ghost()
                .small()
                .selected(kind == current)
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.set_chart_type_from_panel(kind, window, cx);
                })),
            );
        }
        type_row.into_any_element()
    }

    /// The Data-range body: the current bound range summary + a "use selection" apply button.
    fn render_chart_range_body(
        &self,
        panel: &ChartPanel,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let range_status = match &panel.ranges {
            Some(r) => r.clone(),
            None => "No data range set".to_string(),
        };
        let selection_a1 = self.selection.range().to_a1();
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(TEXT))
                    .debug_selector(|| "chart-panel-range-status".into())
                    .child(range_status),
            )
            .child(
                Button::new("chart-panel-apply-range")
                    .label(format!("Use selection ({selection_a1})"))
                    .debug_selector(|| "chart-panel-apply-range".into())
                    .small()
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.apply_chart_range_from_selection(window, cx);
                    })),
            )
            .into_any_element()
    }

    /// The Legend row: one lucide icon per position (`panel-top` / `panel-right` / `panel-left` /
    /// `panel-bottom`, showing where the legend docks) + `square-x` for Off, the current one selected
    /// (post-v1 Batch 2, item 11). Same behavior as before — each button sets the legend position or
    /// turns it off — just iconized. `panel-top` + `square-x` are FreeCell-vendored; the other three
    /// resolve from the gpui-component bundle (`shell::assets`).
    fn render_chart_legend_row(
        &self,
        current: Option<LegendPosition>,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        // `(target position, icon path, id/selector tag, tooltip)`. `None` = Off. Debug-selector
        // tags stay stable (`off` / `R`/`T`/`B`/`L`) so existing selectors keep resolving.
        let entries: [(Option<LegendPosition>, &str, &str, &str); 5] = [
            (Some(LegendPosition::Top), "icons/panel-top.svg", "T", "Top"),
            (
                Some(LegendPosition::Right),
                "icons/panel-right.svg",
                "R",
                "Right",
            ),
            (
                Some(LegendPosition::Left),
                "icons/panel-left.svg",
                "L",
                "Left",
            ),
            (
                Some(LegendPosition::Bottom),
                "icons/panel-bottom.svg",
                "B",
                "Bottom",
            ),
            (None, "icons/square-x.svg", "off", "Off"),
        ];
        let mut row = div().flex().flex_wrap().gap(px(2.0));
        for (pos, icon_path, tag, tooltip) in entries {
            row = row.child(
                Button::new(gpui::ElementId::Name(
                    format!("chart-panel-legend-{tag}").into(),
                ))
                .icon(Icon::empty().path(icon_path))
                .tooltip(tooltip)
                .debug_selector(move || format!("chart-panel-legend-{tag}"))
                .ghost()
                .small()
                .selected(current == pos)
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.set_chart_legend_from_panel(pos, window, cx);
                })),
            );
        }
        row.into_any_element()
    }

    /// The per-series color rows: each series' name + a palette of swatches (the current one ringed)
    /// + an Automatic (clear) button.
    fn render_chart_series_colors(
        &self,
        panel: &ChartPanel,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let mut col = div().flex().flex_col().gap_2();
        for (i, series) in panel.series.iter().enumerate() {
            let mut swatches = div().flex().flex_wrap().items_center().gap(px(3.0));
            for sw in FILL_PALETTE {
                let selected = series.color == Some(sw.rgb);
                let color = sw.rgb;
                swatches = swatches.child(
                    div()
                        .id(gpui::ElementId::NamedInteger(
                            format!("chart-series-{i}-swatch-{:06X}", sw.rgb.to_hex()).into(),
                            i as u64,
                        ))
                        .w(px(16.0))
                        .h(px(16.0))
                        .rounded(px(2.0))
                        .bg(rgb(sw.rgb.to_hex()))
                        .border_1()
                        .border_color(rgb(if selected {
                            SWATCH_SELECTED_RING
                        } else {
                            HAIRLINE
                        }))
                        .cursor_pointer()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                                this.set_chart_series_color_from_panel(i, Some(color), window, cx);
                            }),
                        ),
                );
            }
            swatches = swatches.child(
                Button::new(gpui::ElementId::NamedInteger(
                    "chart-series-auto".into(),
                    i as u64,
                ))
                .label("Auto")
                .debug_selector(move || format!("chart-series-{i}-auto"))
                .ghost()
                .small()
                .selected(series.color.is_none())
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.set_chart_series_color_from_panel(i, None, window, cx);
                })),
            );
            col = col.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(rgb(TEXT))
                            .child(series.name.clone()),
                    )
                    .child(swatches),
            );
        }
        col.into_any_element()
    }

    /// The data-label toggle row: Value / Category / Percent, each reflecting the chart's current
    /// state; clicking flips that flag and applies the whole toggle set.
    fn render_chart_data_labels(
        &self,
        labels: DataLabelToggles,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let toggle = |id: &'static str,
                      label: &'static str,
                      on: bool,
                      next: DataLabelToggles,
                      cx: &mut Context<Self>| {
            Button::new(id)
                .label(label)
                .debug_selector(move || id.into())
                .small()
                .selected(on)
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.set_chart_data_labels_from_panel(next, window, cx);
                }))
        };
        div()
            .flex()
            .flex_wrap()
            .gap(px(2.0))
            .child(toggle(
                "chart-panel-label-value",
                "Value",
                labels.show_value,
                DataLabelToggles {
                    show_value: !labels.show_value,
                    ..labels
                },
                cx,
            ))
            .child(toggle(
                "chart-panel-label-category",
                "Category",
                labels.show_category_name,
                DataLabelToggles {
                    show_category_name: !labels.show_category_name,
                    ..labels
                },
                cx,
            ))
            .child(toggle(
                "chart-panel-label-percent",
                "Percent",
                labels.show_percent,
                DataLabelToggles {
                    show_percent: !labels.show_percent,
                    ..labels
                },
                cx,
            ))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chrome::view::test_support::*;
    use freecell_core::input_cap::MAX_INPUT_LEN;
    use gpui::TestAppContext;

    // ---- Action row: insert chart (P17) ---------------------------------------------------

    /// Criterion #1: inserting a chart is a mutating action-row control — it commits any pending
    /// in-cell edit FIRST (the same rule as every sibling), so the worker's subsequent publish +
    /// grid refresh can't clobber a dangling edit.
    #[gpui::test]
    fn insert_chart_commits_pending_edit_first(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.test_type("=A1", window, cx);
        });
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.insert_chart(ChartInsertKind::Line, window, cx)
        });
        let cmds = h.client.take_commands();
        // The pending edit commits FIRST, then the chart insert.
        assert!(
            matches!(cmds.first(), Some(Command::SetCellInput { input, .. }) if input == "=A1"),
            "pending edit committed first, got {cmds:?}"
        );
        assert!(cmds.iter().any(|c| matches!(
            c,
            Command::InsertChart {
                kind: ChartInsertKind::Line,
                ..
            }
        )));
    }

    /// Criterion #1 (blocking half): a cap-rejected pending edit blocks the insert — no
    /// `InsertChart` is sent and the field stays editing, so the invalid edit isn't clobbered.
    #[gpui::test]
    fn cap_rejected_edit_blocks_insert_chart(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        let huge = format!("={}", "1".repeat(MAX_INPUT_LEN));
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(0, 0)), window, cx);
            c.test_type(&huge, window, cx);
        });
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.insert_chart(ChartInsertKind::Line, window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(
            !cmds
                .iter()
                .any(|c| matches!(c, Command::InsertChart { .. })),
            "a cap-rejected pending edit blocks the insert, got {cmds:?}"
        );
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.data_mode()), FieldMode::Editing);
    }

    /// Inserting anchors the chart at the active cell (8×15 cells) on the active sheet.
    #[gpui::test]
    fn insert_chart_sends_command_for_active_sheet_from_active_cell(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(3, 2)), window, cx)
        });
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.insert_chart(ChartInsertKind::Column, window, cx)
        });
        let cmds = h.client.take_commands();
        let inserted = cmds.iter().find_map(|cmd| match cmd {
            Command::InsertChart {
                sheet,
                kind,
                anchor,
                data,
            } => Some((*sheet, *kind, *anchor, *data)),
            _ => None,
        });
        let (sheet, kind, anchor, data) = inserted.expect("an InsertChart command was sent");
        assert_eq!(sheet, SheetId(0));
        assert_eq!(kind, ChartInsertKind::Column);
        // From the active cell (col 2, row 3), spanning 8 cols × 15 rows.
        assert_eq!((anchor.from.col, anchor.from.row), (2, 3));
        assert_eq!((anchor.to.col, anchor.to.row), (2 + 8, 3 + 15));
        // A single-cell selection carries no data range (the chart stays near-empty).
        assert_eq!(data, None, "a single-cell selection seeds no data range");
    }

    /// Batch 3 item 8: inserting a chart with a **range** selection (more than one cell) threads that
    /// range into `InsertChart` as its data, so the worker binds it at creation (no "Use selection"
    /// click). A single-cell selection threads `None` (covered above), keeping the near-empty chart.
    #[gpui::test]
    fn insert_chart_with_range_selection_seeds_data_range(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        // Select a real B2:D9 block, then insert.
        let sel = SelectionModel {
            anchor: cell(1, 1),
            active: cell(8, 3),
        };
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(sel, window, cx)
        });
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.insert_chart(ChartInsertKind::Line, window, cx)
        });
        let cmds = h.client.take_commands();
        let data = cmds.iter().find_map(|cmd| match cmd {
            Command::InsertChart { data, .. } => Some(*data),
            _ => None,
        });
        assert_eq!(
            data,
            Some(Some(sel.range())),
            "a range selection is threaded into InsertChart as its data, got {cmds:?}"
        );
    }

    /// Criterion #2 (disabled-in-degraded parity): OPEN the chart menu, THEN degrade — the menu
    /// must close (so a type glyph can't be clicked after the trigger disables), mirroring how the
    /// other formatting popovers close on degrade.
    #[gpui::test]
    fn degrade_closes_open_chart_menu(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, _w, cx| c.toggle_chart_menu(cx));
        assert!(
            upd(&h, cx, |c, _w, _cx| c.chart_menu_open()),
            "the chart menu opened"
        );
        upd(&h, cx, |c, _w, cx| c.set_degraded(true, cx));
        assert!(
            !upd(&h, cx, |c, _w, _cx| c.chart_menu_open()),
            "degrading closes the open chart menu"
        );
        assert!(upd(&h, cx, |c, _w, _cx| c.is_degraded()));
    }

    // ---- Chart edit panel (P19) -----------------------------------------------------------

    #[gpui::test]
    fn chart_panel_opens_and_closes(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.chart_panel_target()), None);
        upd(&h, cx, |c, window, cx| {
            c.open_chart_panel(
                ChartPanel::skeleton(SheetId(0), ChartId(7), true, ChartInsertKind::Line),
                window,
                cx,
            )
        });
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.chart_panel_target()),
            Some(ChartId(7)),
            "the panel opens on the given chart"
        );
        upd(&h, cx, |c, _w, cx| c.close_chart_panel(cx));
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.chart_panel_target()), None);
    }

    /// A type glyph in the panel sends `SetChartType` for the panel's chart and updates the shown
    /// kind optimistically.
    #[gpui::test]
    fn chart_panel_type_glyph_sends_set_chart_type(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.open_chart_panel(
                ChartPanel::skeleton(SheetId(0), ChartId(7), true, ChartInsertKind::Line),
                window,
                cx,
            )
        });
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.set_chart_type_from_panel(ChartInsertKind::Column, window, cx)
        });
        assert!(matches!(
            h.client.take_commands().as_slice(),
            [Command::SetChartType {
                id: ChartId(7),
                kind: ChartInsertKind::Column,
                ..
            }]
        ));
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.chart_panel.as_ref().map(|p| p.kind)),
            Some(ChartInsertKind::Column),
            "the shown type updates optimistically"
        );
    }

    /// The "use selection" button binds the chart to the current grid selection as its data range.
    #[gpui::test]
    fn chart_panel_apply_range_uses_current_selection(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(
                SelectionModel {
                    anchor: cell(0, 0),
                    active: cell(4, 3),
                },
                window,
                cx,
            )
        });
        upd(&h, cx, |c, window, cx| {
            c.open_chart_panel(
                ChartPanel::skeleton(SheetId(0), ChartId(7), true, ChartInsertKind::Line),
                window,
                cx,
            )
        });
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.apply_chart_range_from_selection(window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::SetChartRange { id: ChartId(7), data, .. }]
                    if *data == freecell_core::CellRange::new(cell(0, 0), cell(4, 3))
            ),
            "the current selection is applied as the chart's range, got {cmds:?}"
        );
    }

    /// Moderate-fix regression: "Use selection" binds to the sheet the SELECTION is on (the active
    /// sheet), not the chart's host sheet — so a chart hosted on one sheet can be bound to another
    /// sheet's data (valid cross-sheet), and a stale host sheet never silently mis-qualifies the c:f.
    #[gpui::test]
    fn chart_panel_apply_range_binds_the_active_sheet_not_the_host(cx: &mut TestAppContext) {
        // The user is on sheet 1 ("Data"); the panel edits a chart HOSTED on sheet 0 ("Host").
        let h = build(
            cx,
            vec![
                SheetTab::new(SheetId(0), "Host"),
                SheetTab::new(SheetId(1), "Data"),
            ],
            SheetId(1),
        );
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(
                SelectionModel {
                    anchor: cell(0, 0),
                    active: cell(4, 1),
                },
                window,
                cx,
            )
        });
        upd(&h, cx, |c, window, cx| {
            c.open_chart_panel(
                ChartPanel::skeleton(SheetId(0), ChartId(7), true, ChartInsertKind::Line),
                window,
                cx,
            )
        });
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            c.apply_chart_range_from_selection(window, cx)
        });
        let cmds = h.client.take_commands();
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::SetChartRange { sheet: SheetId(1), id: ChartId(7), data }]
                    if *data == freecell_core::CellRange::new(cell(0, 0), cell(4, 1))
            ),
            "the range binds the active/data sheet (1), not the chart's host sheet (0): {cmds:?}"
        );
    }

    /// Degrading closes the panel and makes its controls inert (like the action-bar popovers).
    #[gpui::test]
    fn degrade_closes_chart_panel(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.open_chart_panel(
                ChartPanel::skeleton(SheetId(0), ChartId(7), true, ChartInsertKind::Line),
                window,
                cx,
            )
        });
        assert!(upd(&h, cx, |c, _w, _cx| c.chart_panel_target().is_some()));
        upd(&h, cx, |c, _w, cx| c.set_degraded(true, cx));
        assert!(
            upd(&h, cx, |c, _w, _cx| c.chart_panel_target().is_none()),
            "degrading closes the edit panel"
        );
        upd(&h, cx, |c, window, cx| {
            c.set_chart_type_from_panel(ChartInsertKind::Bar, window, cx)
        });
        assert!(
            h.client.take_commands().is_empty(),
            "a closed/degraded panel sends no command"
        );
    }

    // ---- Chart edit panel: chrome (P20) ---------------------------------------------------

    /// The canonical chrome-editing [`ChartPanel`] over chart 7 (host sheet 0) with one series —
    /// authored, seeded title "Chart". Spread with `..chart_7_panel()` to vary a field.
    fn chart_7_panel() -> ChartPanel {
        ChartPanel {
            sheet: SheetId(0),
            id: ChartId(7),
            is_authored: true,
            kind: ChartInsertKind::Line,
            ranges: None,
            title: Some("Chart".into()),
            legend: Some(LegendPosition::Right),
            cat_axis_title: None,
            val_axis_title: None,
            series: vec![ChartPanelSeries {
                name: "Widgets".into(),
                color: Some(Rgb::from_hex(0x4472C4)),
            }],
            labels: DataLabelToggles::default(),
        }
    }

    /// Open a chrome-editing panel over chart 7 (host sheet 0) with one series.
    fn open_chrome_panel(h: &Harness, cx: &mut TestAppContext, is_authored: bool) {
        upd(h, cx, |c, window, cx| {
            c.open_chart_panel(
                ChartPanel {
                    is_authored,
                    ..chart_7_panel()
                },
                window,
                cx,
            )
        });
        h.client.take_commands();
    }

    /// The single `SetChartChrome` edit sent for chart 7, or a panic.
    fn last_chrome_edit(h: &Harness) -> ChartChromeEdit {
        match h.client.take_commands().as_slice() {
            [Command::SetChartChrome {
                id: ChartId(7),
                edit,
                ..
            }] => edit.clone(),
            other => panic!("expected one SetChartChrome for chart 7, got {other:?}"),
        }
    }

    #[gpui::test]
    fn chart_panel_title_sends_chrome_and_updates_optimistically(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        open_chrome_panel(&h, cx, true);
        upd(&h, cx, |c, window, cx| {
            c.set_chart_title_from_panel(Some("Sales".into()), window, cx)
        });
        assert_eq!(
            last_chrome_edit(&h),
            ChartChromeEdit::Title(Some("Sales".into()))
        );
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c
                .chart_panel
                .as_ref()
                .and_then(|p| p.title.clone())),
            Some("Sales".into()),
            "the shown title updates optimistically",
        );
    }

    /// Typing in the title field + pressing Enter commits the title as a chrome edit.
    #[gpui::test]
    fn chart_panel_title_input_commits_on_enter(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        open_chrome_panel(&h, cx, true);
        upd(&h, cx, |c, window, cx| {
            let handle = c.chart_title_input.clone();
            handle.update(cx, |i, cx| i.set_value("Renamed", window, cx));
            c.on_chart_title_event(
                &handle,
                &InputEvent::PressEnter {
                    secondary: false,
                    shift: false,
                },
                window,
                cx,
            );
        });
        assert_eq!(
            last_chrome_edit(&h),
            ChartChromeEdit::Title(Some("Renamed".into())),
        );
    }

    /// Clearing the title field to empty commits `Title(None)` (remove the title).
    #[gpui::test]
    fn chart_panel_empty_title_input_clears_the_title(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        open_chrome_panel(&h, cx, true); // seeded title = "Chart"
        upd(&h, cx, |c, window, cx| {
            let handle = c.chart_title_input.clone();
            handle.update(cx, |i, cx| i.set_value("", window, cx));
            c.on_chart_title_event(&handle, &InputEvent::Blur, window, cx);
        });
        assert_eq!(last_chrome_edit(&h), ChartChromeEdit::Title(None));
    }

    #[gpui::test]
    fn chart_panel_legend_off_and_position(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        open_chrome_panel(&h, cx, true);
        upd(&h, cx, |c, window, cx| {
            c.set_chart_legend_from_panel(None, window, cx)
        });
        assert_eq!(last_chrome_edit(&h), ChartChromeEdit::Legend(None));
        upd(&h, cx, |c, window, cx| {
            c.set_chart_legend_from_panel(Some(LegendPosition::Bottom), window, cx)
        });
        assert_eq!(
            last_chrome_edit(&h),
            ChartChromeEdit::Legend(Some(LegendPosition::Bottom))
        );
    }

    #[gpui::test]
    fn chart_panel_axis_title_sends_chrome(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        open_chrome_panel(&h, cx, true);
        upd(&h, cx, |c, window, cx| {
            c.set_chart_axis_title_from_panel(
                ChartAxisKind::Value,
                Some("Units".into()),
                window,
                cx,
            )
        });
        assert_eq!(
            last_chrome_edit(&h),
            ChartChromeEdit::AxisTitle {
                axis: ChartAxisKind::Value,
                title: Some("Units".into()),
            },
        );
    }

    #[gpui::test]
    fn chart_panel_series_color_sends_chrome(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        open_chrome_panel(&h, cx, true);
        upd(&h, cx, |c, window, cx| {
            c.set_chart_series_color_from_panel(0, Some(Rgb::from_hex(0x70AD47)), window, cx)
        });
        assert_eq!(
            last_chrome_edit(&h),
            ChartChromeEdit::SeriesColor {
                series: 0,
                color: Some(Rgb::from_hex(0x70AD47)),
            },
        );
        // Clearing back to Auto sends None.
        upd(&h, cx, |c, window, cx| {
            c.set_chart_series_color_from_panel(0, None, window, cx)
        });
        assert_eq!(
            last_chrome_edit(&h),
            ChartChromeEdit::SeriesColor {
                series: 0,
                color: None
            },
        );
    }

    #[gpui::test]
    fn chart_panel_data_labels_send_chrome(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        open_chrome_panel(&h, cx, true);
        let toggles = DataLabelToggles {
            show_value: true,
            show_category_name: false,
            show_percent: true,
        };
        upd(&h, cx, |c, window, cx| {
            c.set_chart_data_labels_from_panel(toggles, window, cx)
        });
        assert_eq!(last_chrome_edit(&h), ChartChromeEdit::DataLabels(toggles));
    }

    /// A **loaded** chart's panel still edits chrome (the same commands), while its provenance is
    /// recorded so the render hides the authored-only Type + Data-range sections.
    #[gpui::test]
    fn loaded_chart_panel_edits_chrome(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        open_chrome_panel(&h, cx, false);
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c
                .chart_panel
                .as_ref()
                .map(|p| p.is_authored)),
            Some(false),
            "the panel records the loaded provenance",
        );
        upd(&h, cx, |c, window, cx| {
            c.set_chart_title_from_panel(Some("Reviewed".into()), window, cx)
        });
        assert_eq!(
            last_chrome_edit(&h),
            ChartChromeEdit::Title(Some("Reviewed".into())),
        );
    }

    /// The full chrome panel actually **paints** without panicking — both the authored variant (Type +
    /// Data range + every chrome section incl. the per-series swatches) and the loaded variant
    /// (chrome-only). Forces a real draw through the test renderer (the chrome is out of pixel scope,
    /// so this + the Xvfb smoke launch are its render validation).
    #[gpui::test]
    fn chart_panel_paints_for_both_provenances(cx: &mut TestAppContext) {
        let h = tall_sheet(cx);
        open_chrome_panel(&h, cx, true);
        {
            let vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
            vcx.run_until_parked();
        }
        assert!(
            upd(&h, cx, |c, _w, _cx| c.chart_panel_target().is_some()),
            "the authored panel painted and stayed open"
        );

        upd(&h, cx, |c, _w, cx| c.close_chart_panel(cx));
        open_chrome_panel(&h, cx, false);
        {
            let vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
            vcx.run_until_parked();
        }
        assert!(
            upd(&h, cx, |c, _w, _cx| c.chart_panel_target().is_some()),
            "the loaded (chrome-only) panel painted and stayed open"
        );
    }

    /// Batch 3 item 10: the action-bar new-chart dropdown and the right-docked edit panel can be open
    /// **at the same time** and the chrome paints without panicking. The panel is pushed as the
    /// bottom-most overlay so the dropdown (pushed later) paints ABOVE it; this forces a real draw of
    /// that coexistence path (chrome is out of pixel scope — z-order itself is verified in the Xvfb
    /// smoke launch, this guards the both-open render path).
    #[gpui::test]
    fn chart_menu_and_panel_coexist_and_paint(cx: &mut TestAppContext) {
        let h = tall_sheet(cx);
        open_chrome_panel(&h, cx, true); // right-docked edit panel open (authored chart 7)
        upd(&h, cx, |c, _w, cx| c.toggle_chart_menu(cx)); // action-bar new-chart dropdown open
        assert!(
            upd(&h, cx, |c, _w, _cx| c.chart_menu_open())
                && upd(&h, cx, |c, _w, _cx| c.chart_panel_target().is_some()),
            "the dropdown and the edit panel are both open"
        );
        {
            let vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
            vcx.run_until_parked();
        }
        // Both survived the paint (the overlay-ordering path drew cleanly, no panic).
        assert!(
            upd(&h, cx, |c, _w, _cx| c.chart_menu_open())
                && upd(&h, cx, |c, _w, _cx| c.chart_panel_target().is_some()),
            "the dropdown-over-panel overlay stack painted and both stayed open"
        );
    }

    /// Degrading makes the chrome setters inert (a closed panel sends nothing).
    #[gpui::test]
    fn degrade_makes_chrome_setters_inert(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        open_chrome_panel(&h, cx, true);
        upd(&h, cx, |c, _w, cx| c.set_degraded(true, cx));
        upd(&h, cx, |c, window, cx| {
            c.set_chart_legend_from_panel(None, window, cx)
        });
        assert!(
            h.client.take_commands().is_empty(),
            "a degraded panel sends no chrome edit"
        );
    }

    /// CR guard: a text field that gained focus for chart A must NOT commit its (stale) text to a
    /// DIFFERENT chart if the panel re-points before the field's Blur is processed (rapid selection
    /// switch). The captured focus target (A) no longer matches the panel (B), so the commit is dropped.
    #[gpui::test]
    fn chart_panel_stale_field_commit_is_dropped_after_repoint(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        open_chrome_panel(&h, cx, true); // panel on chart 7
        upd(&h, cx, |c, window, cx| {
            let handle = c.chart_title_input.clone();
            // Focus captures the target (sheet 0, chart 7); then the field holds stale text for it.
            c.on_chart_title_event(&handle, &InputEvent::Focus, window, cx);
            handle.update(cx, |i, cx| i.set_value("Stale for chart 7", window, cx));
        });
        // The panel re-points to a DIFFERENT chart under the still-focused field (the event-ordering
        // race: a direct re-point that does NOT re-seed, so the field keeps its stale text).
        upd(&h, cx, |c, _w, _cx| {
            c.chart_panel = Some(ChartPanel::skeleton(
                SheetId(0),
                ChartId(9),
                true,
                ChartInsertKind::Line,
            ));
        });
        h.client.take_commands();
        upd(&h, cx, |c, window, cx| {
            let handle = c.chart_title_input.clone();
            c.on_chart_title_event(&handle, &InputEvent::Blur, window, cx);
        });
        assert!(
            h.client.take_commands().is_empty(),
            "stale field text must not be sent to the re-pointed chart",
        );
    }

    /// The counterpart: a focused field whose panel is UNCHANGED commits normally (proving the guard
    /// drops only stale, re-pointed commits — not every focused edit).
    #[gpui::test]
    fn chart_panel_focused_field_commits_when_panel_unchanged(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        open_chrome_panel(&h, cx, true);
        upd(&h, cx, |c, window, cx| {
            let handle = c.chart_title_input.clone();
            c.on_chart_title_event(&handle, &InputEvent::Focus, window, cx);
            handle.update(cx, |i, cx| i.set_value("Kept", window, cx));
            c.on_chart_title_event(&handle, &InputEvent::Blur, window, cx);
        });
        assert_eq!(
            last_chrome_edit(&h),
            ChartChromeEdit::Title(Some("Kept".into())),
            "a same-chart focus→blur commits the field",
        );
    }

    // ---- Chart edit panel: post-v1 Batch 2 (live titles / click-away / scroll / order) ----

    /// Item 6: typing in the Title field commits the chart title **live, per keystroke** (`Change`) —
    /// no Enter/blur needed. Each keystroke sends the current text as a chrome edit.
    #[gpui::test]
    fn chart_panel_title_input_commits_live_on_each_keystroke(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        open_chrome_panel(&h, cx, true); // seeded title = "Chart"
        upd(&h, cx, |c, window, cx| {
            let handle = c.chart_title_input.clone();
            c.on_chart_title_event(&handle, &InputEvent::Focus, window, cx);
            handle.update(cx, |i, cx| i.set_value("S", window, cx));
            c.on_chart_title_event(&handle, &InputEvent::Change, window, cx);
        });
        assert_eq!(
            last_chrome_edit(&h),
            ChartChromeEdit::Title(Some("S".into())),
            "the first keystroke commits live",
        );
        upd(&h, cx, |c, window, cx| {
            let handle = c.chart_title_input.clone();
            handle.update(cx, |i, cx| i.set_value("Sa", window, cx));
            c.on_chart_title_event(&handle, &InputEvent::Change, window, cx);
        });
        assert_eq!(
            last_chrome_edit(&h),
            ChartChromeEdit::Title(Some("Sa".into())),
            "the next keystroke commits live too",
        );
    }

    /// Item 6: the axis-title fields also commit live per keystroke.
    #[gpui::test]
    fn chart_panel_axis_title_input_commits_live_on_change(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        open_chrome_panel(&h, cx, true); // cat/val axis titles seeded empty
        upd(&h, cx, |c, window, cx| {
            let handle = c.chart_cat_axis_input.clone();
            c.on_chart_cat_axis_event(&handle, &InputEvent::Focus, window, cx);
            handle.update(cx, |i, cx| i.set_value("Month", window, cx));
            c.on_chart_cat_axis_event(&handle, &InputEvent::Change, window, cx);
        });
        assert_eq!(
            last_chrome_edit(&h),
            ChartChromeEdit::AxisTitle {
                axis: ChartAxisKind::Category,
                title: Some("Month".into()),
            },
        );
    }

    /// Item 6 anti-clobber: a live **republish of the same chart** (a same-id reconcile) must NOT
    /// re-seed the field, so it never overwrites what the user is actively typing.
    #[gpui::test]
    fn chart_panel_same_chart_republish_does_not_clobber_typing(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        open_chrome_panel(&h, cx, true); // chart 7, title "Chart" seeded
                                         // The user is mid-type: the field holds unsaved text (set_value stands in for typing).
        upd(&h, cx, |c, window, cx| {
            let handle = c.chart_title_input.clone();
            c.on_chart_title_event(&handle, &InputEvent::Focus, window, cx);
            handle.update(cx, |i, cx| i.set_value("My draft title", window, cx));
        });
        // A worker republish reconciles the SAME chart (id 7) with the stale snapshot title.
        upd(&h, cx, |c, window, cx| {
            c.open_chart_panel(
                ChartPanel {
                    title: Some("Chart".into()),
                    ..chart_7_panel()
                },
                window,
                cx,
            )
        });
        assert_eq!(
            upd(&h, cx, |c, _w, cx| c
                .chart_title_input
                .read(cx)
                .value()
                .to_string()),
            "My draft title",
            "a same-chart republish must not re-seed / clobber the in-progress edit",
        );
    }

    /// Item 6 anti-clobber counterpart: **switching** the selected chart (a new id) DOES re-seed the
    /// fields to the new chart's values.
    #[gpui::test]
    fn chart_panel_switch_reseeds_the_title_field(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        open_chrome_panel(&h, cx, true); // chart 7, title "Chart"
        upd(&h, cx, |c, window, cx| {
            c.open_chart_panel(
                ChartPanel {
                    id: ChartId(9),
                    title: Some("Nine".into()),
                    ..chart_7_panel()
                },
                window,
                cx,
            )
        });
        assert_eq!(
            upd(&h, cx, |c, _w, cx| c
                .chart_title_input
                .read(cx)
                .value()
                .to_string()),
            "Nine",
            "switching to another chart re-seeds the field to its value",
        );
    }

    /// Item 12: a grid selection change (a click on a cell / empty grid) closes the edit panel.
    #[gpui::test]
    fn chart_panel_closes_on_grid_click_away(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        open_chrome_panel(&h, cx, true);
        assert!(upd(&h, cx, |c, _w, _cx| c.chart_panel_target().is_some()));
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(2, 2)), window, cx)
        });
        assert!(
            upd(&h, cx, |c, _w, _cx| c.chart_panel_target().is_none()),
            "a grid click (selection change) closes the edit panel",
        );
    }

    /// Item 12: selecting **another chart** re-points the panel (the window routes a chart click
    /// through `open_chart_panel`, not `on_selection_changed`), so it switches rather than closing.
    #[gpui::test]
    fn chart_panel_another_chart_switches_not_closes(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        open_chrome_panel(&h, cx, true); // chart 7
        upd(&h, cx, |c, window, cx| {
            c.open_chart_panel(
                ChartPanel::skeleton(SheetId(0), ChartId(9), true, ChartInsertKind::Line),
                window,
                cx,
            )
        });
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.chart_panel_target()),
            Some(ChartId(9)),
            "clicking another chart switches the panel to it, not close",
        );
    }

    /// Item 11: the legend icon buttons set the right position / off (behavior unchanged, just
    /// iconized). Exercises the same setter the icon `on_click`s call.
    #[gpui::test]
    fn chart_panel_legend_icons_set_position_and_off(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        open_chrome_panel(&h, cx, true);
        for pos in [
            Some(LegendPosition::Top),
            Some(LegendPosition::Right),
            Some(LegendPosition::Left),
            Some(LegendPosition::Bottom),
            None,
        ] {
            upd(&h, cx, |c, window, cx| {
                c.set_chart_legend_from_panel(pos, window, cx)
            });
            assert_eq!(last_chrome_edit(&h), ChartChromeEdit::Legend(pos));
        }
    }

    /// Item 13: the shared chart-type order places **Area right after Line** (Excel grouping), then
    /// the Column/Bar pair — the single canonical order used by both the panel Type row and the
    /// action-bar dropdown.
    #[test]
    fn chart_type_order_places_area_after_line() {
        let kinds: Vec<ChartInsertKind> = CHART_MENU.iter().map(|(k, _, _)| *k).collect();
        assert_eq!(
            &kinds[..4],
            &[
                ChartInsertKind::Line,
                ChartInsertKind::Area,
                ChartInsertKind::Column,
                ChartInsertKind::Bar,
            ],
            "Line → Area → Column → Bar",
        );
    }

    /// Item 7: the panel paints (scrollable body + clipped to its bounds) on a **short** window where
    /// its control stack overflows — without panicking and while staying open.
    #[gpui::test]
    fn chart_panel_paints_scrollable_on_a_short_window(cx: &mut TestAppContext) {
        let h = build_win(
            cx,
            vec![SheetTab::new(SheetId(0), "Sheet1")],
            SheetId(0),
            160.0,
        );
        upd(&h, cx, |c, _w, cx| {
            let body: gpui::AnyView = cx.new(|_| ShortBodyStub).into();
            c.set_grid_body(body, cx);
        });
        open_chrome_panel(&h, cx, true);
        {
            let vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
            vcx.run_until_parked();
        }
        assert!(
            upd(&h, cx, |c, _w, _cx| c.chart_panel_target().is_some()),
            "the panel paints (scrollable + clipped) on a short window and stays open",
        );
    }
}
