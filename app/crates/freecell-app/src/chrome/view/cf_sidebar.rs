//! The conditional-formatting sidebar's **List mode** (`components/cf_sidebar.md §5`, P4/P5):
//! opening, closing, re-scoping and refreshing the right-docked panel, the rules list, and each
//! row's preview swatch and raise / lower / delete controls.
//!
//! The rule **editor** the rows open is [`super::cf_editor`] — the same panel's other mode,
//! split out because the two together exceed the 2,000-line production ceiling
//! (`architecture.md §7.3`). Moved verbatim out of the single-file `chrome/view.rs`
//! (`specs/projects/chrome-view-split`).

use super::*;

/// The conditional-formatting list-row preview swatch (`components/cf_sidebar.md §5`): a small
/// hairline chip carrying a highlight rule's fill+text, a colour scale's banded gradient, or a
/// deferred-family Badge tag.
pub(super) const CF_SWATCH_W: f32 = 22.0;
pub(super) const CF_SWATCH_H: f32 = 16.0;
/// The muted tag background behind a deferred-family Badge preview (a light grey).
const CF_BADGE_BG: u32 = 0xEDEDED;

/// Which of a CF rule row's controls are enabled (`components/cf_sidebar.md §5`). Move-up/down are
/// disabled at the ends of the priority-descending list; edit is disabled for a non-editable
/// (deferred-family/Badge) rule; delete is always enabled (even deferred-family rules are
/// deletable — `functional_spec.md §9`). Pure so the row test asserts the enablement logic without
/// pixel clicks, and [`render_cf_row`](ChromeView::render_cf_row) derives its `.disabled(...)`
/// flags from the same source.
pub(super) struct CfRowControls {
    pub(super) move_up: bool,
    pub(super) move_down: bool,
    pub(super) edit: bool,
    pub(super) delete: bool,
}

pub(super) fn cf_row_controls(row: &CfRuleView, is_first: bool, is_last: bool) -> CfRowControls {
    CfRowControls {
        move_up: !is_first,
        move_down: !is_last,
        // Every authorable row (including a concrete-RGB color scale — P7) opens the editor; a
        // deferred-family/theme-colored-scale Badge row is already `editable == false` (P1).
        edit: row.editable,
        delete: true,
    }
}

/// Whether `row`'s target range intersects the selection `sel` (BUG-3, selection-scoped list). A
/// rule's `range` is its raw sqref and may be a **multi-area** address (whitespace-separated
/// sub-areas, e.g. `"A1:A10 C1:C10"`). Each sub-area is parsed with [`CellRange::from_sqref_area`],
/// which — unlike plain A1 parsing — also understands the **whole-column** (`"A:A"`) and
/// **whole-row** (`"1:1"`) shapes Excel writes verbatim on XLSX load; the rule matches if **any**
/// sub-area parses and overlaps `sel`.
///
/// **Fail-open:** if NOT ONE sub-area parses (every sub-area is an unrecognized shape), the rule is
/// shown anyway. A rule must never silently vanish from the list just because its sqref shape isn't
/// understood — leaving it unmanageable. Only when sub-areas *did* parse but none overlap does the
/// rule hide (it genuinely doesn't cover the selection).
fn cf_rule_intersects_selection(row: &CfRuleView, sel: &CellRange) -> bool {
    let mut any_parsed = false;
    for area in row.range.split_whitespace() {
        if let Some(r) = CellRange::from_sqref_area(area) {
            any_parsed = true;
            if r.intersects(sel) {
                return true;
            }
        }
    }
    !any_parsed
}

/// A `freecell_core::Rgb` as a gpui fill/text colour (the grid maps `Rgb` the same way at draw
/// time — `color.rs`).
pub(super) fn cf_color(c: Rgb) -> Rgba {
    rgb(c.to_hex())
}

/// The List-mode preview swatch for a rule's effect (`components/cf_sidebar.md §5`): a highlight
/// rule's fill + text-colour chip (an "A" glyph so both read), a colour scale's banded gradient,
/// or a deferred-family/variant Badge tag (the first pass can't author it, only show + delete it).
fn render_cf_preview(preview: &CfPreview) -> gpui::AnyElement {
    match preview {
        CfPreview::Highlight { fill, text_color } => {
            // "No fill" reads as a white chip; "no text colour" as the default text colour — so the
            // chip always renders on the sidebar's white card.
            let bg = (*fill).map(cf_color).unwrap_or_else(|| rgb(ACTIVE_TAB_BG));
            let fg = (*text_color).map(cf_color).unwrap_or_else(|| rgb(TEXT));
            div()
                .flex_shrink_0()
                .w(px(CF_SWATCH_W))
                .h(px(CF_SWATCH_H))
                .rounded_sm()
                .border_1()
                .border_color(rgb(HAIRLINE))
                .flex()
                .items_center()
                .justify_center()
                .bg(bg)
                .child(
                    div()
                        .text_size(px(10.0))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(fg)
                        .child("A"),
                )
                .into_any_element()
        }
        CfPreview::ColorScale { colors } => {
            // Equal-width bands across the stop colours — a stepped horizontal gradient the width
            // of one swatch. `overflow_hidden` clips the bands to the rounded chip.
            let mut chip = div()
                .flex_shrink_0()
                .w(px(CF_SWATCH_W))
                .h(px(CF_SWATCH_H))
                .rounded_sm()
                .overflow_hidden()
                .border_1()
                .border_color(rgb(HAIRLINE))
                .flex();
            for c in colors {
                chip = chip.child(div().flex_1().h_full().bg(cf_color(*c)));
            }
            chip.into_any_element()
        }
        CfPreview::Badge(label) => div()
            .flex_shrink_0()
            .px(px(4.0))
            .py(px(1.0))
            .rounded_sm()
            .bg(rgb(CF_BADGE_BG))
            .border_1()
            .border_color(rgb(HAIRLINE))
            .text_size(px(9.5))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(rgb(MUTED_TEXT))
            .child(label.clone())
            .into_any_element(),
    }
}

impl ChromeView {
    // ---- Conditional-formatting sidebar (P4, `components/cf_sidebar.md`) -------------------

    /// Whether the CF sidebar is open (the action-bar `split` button's `selected` state + the render
    /// gate + tests).
    pub fn cond_fmt_open(&self) -> bool {
        self.cond_fmt.is_some()
    }

    /// The sheet the open CF sidebar targets, if any (window introspection: the `CondFmtUpdated`
    /// refresh gate).
    pub fn cond_fmt_sheet(&self) -> Option<SheetId> {
        self.cond_fmt.as_ref().map(|p| p.sheet)
    }

    /// Toggle the right-docked CF sidebar (the action-bar `split` button): open it in List mode if
    /// closed, else close it.
    pub fn toggle_cond_fmt_sidebar(&mut self, cx: &mut Context<Self>) {
        if self.cond_fmt.is_some() {
            self.close_cond_fmt(cx);
        } else {
            self.open_cond_fmt(cx);
        }
    }

    /// Open the CF sidebar in List mode for the active sheet, building its rows from the published
    /// rules. Closes the chart panel first (they share the right dock — `components/cf_sidebar.md §2`).
    fn open_cond_fmt(&mut self, cx: &mut Context<Self>) {
        self.close_chart_panel(cx);
        let sheet = self.active_sheet;
        let rows = self.client.cond_fmt_rules(sheet);
        self.cond_fmt = Some(CondFmtPanel {
            sheet,
            rows,
            editor: None,
        });
        cx.notify();
    }

    /// Close the CF sidebar (its ×, the action-bar toggle, or a degrade).
    pub fn close_cond_fmt(&mut self, cx: &mut Context<Self>) {
        self.cf_menu_open = None;
        if self.cond_fmt.take().is_some() {
            cx.notify();
        }
    }

    /// Rebuild the open CF sidebar's rows from the latest published rules for its own sheet — the
    /// `WorkerEvent::CondFmtUpdated` refresh (and, from P5, after a CF command). A no-op when the
    /// sidebar is closed.
    pub fn refresh_cond_fmt(&mut self, cx: &mut Context<Self>) {
        let Some(sheet) = self.cond_fmt.as_ref().map(|p| p.sheet) else {
            return;
        };
        let rows = self.client.cond_fmt_rules(sheet);
        // A refresh that lands while a Save is in flight is that save's success signal — the rule
        // was accepted (an engine `Err` would instead route to `show_cf_editor_error`), so return
        // to List mode (`components/cf_sidebar.md §4`).
        let save_landed = self
            .cond_fmt
            .as_ref()
            .and_then(|p| p.editor.as_ref())
            .is_some_and(|e| e.pending_save);
        if let Some(panel) = self.cond_fmt.as_mut() {
            panel.rows = rows;
            if save_landed {
                panel.editor = None;
            }
            cx.notify();
        }
        if save_landed {
            self.cf_menu_open = None;
        }
    }

    /// Re-scope the open CF sidebar to the newly active sheet (rebuild its rows), if open — the
    /// sheet-switch path (`components/cf_sidebar.md §4/§9`: the sidebar does **not** close on a sheet
    /// change, it re-scopes). A no-op when the sidebar is closed. (P6 also cancels any open editor here.)
    pub(super) fn rescope_cond_fmt_if_open(&mut self, cx: &mut Context<Self>) {
        if self.cond_fmt.is_none() {
            return;
        }
        let sheet = self.active_sheet;
        let rows = self.client.cond_fmt_rules(sheet);
        self.cf_menu_open = None;
        if let Some(panel) = self.cond_fmt.as_mut() {
            panel.sheet = sheet;
            panel.rows = rows;
            // A sheet switch cancels any open editor (it was scoped to the old sheet) —
            // `components/cf_sidebar.md §4`.
            panel.editor = None;
            cx.notify();
        }
    }

    /// Raise the priority of the CF rule at storage `index` — the row's ▲ control
    /// (`components/cf_sidebar.md §5`). Fire-and-forget; the worker republishes the reordered list
    /// and the sidebar refreshes via `CondFmtUpdated` → [`refresh_cond_fmt`](Self::refresh_cond_fmt).
    /// A no-op when the sidebar is closed (no target sheet). `index` is the rule's stable storage
    /// index, not its display position.
    pub fn raise_cf_rule(&mut self, index: u32) {
        if let Some(sheet) = self.cond_fmt_sheet() {
            self.client
                .send(Command::RaiseCondFmtPriority { sheet, index });
        }
    }

    /// Lower the priority of the CF rule at storage `index` — the row's ▼ control (the mirror of
    /// [`raise_cf_rule`](Self::raise_cf_rule)).
    pub fn lower_cf_rule(&mut self, index: u32) {
        if let Some(sheet) = self.cond_fmt_sheet() {
            self.client
                .send(Command::LowerCondFmtPriority { sheet, index });
        }
    }

    /// Delete the CF rule at storage `index` — the row's 🗑 control. Enabled for every row,
    /// including deferred-family/Badge rows (which are non-editable but deletable —
    /// `functional_spec.md §9`). A no-op when the sidebar is closed.
    pub fn delete_cf_rule(&mut self, index: u32) {
        if let Some(sheet) = self.cond_fmt_sheet() {
            self.client.send(Command::DeleteCondFmt { sheet, index });
        }
    }

    /// The right-docked **conditional-formatting sidebar** (`components/cf_sidebar.md §5`): the
    /// **List mode** (intro + rows + "+ Add rule") or, when `panel.editor` is `Some`, the rule
    /// **Editor mode** (`render_cf_editor`) — both inside the same [`docked_sidebar`] container.
    pub(super) fn render_cond_fmt_sidebar(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let panel = self
            .cond_fmt
            .as_ref()
            .expect("render_cond_fmt_sidebar only runs while the sidebar is open");

        let body = match panel.editor.as_ref() {
            Some(editor) => self.render_cf_editor(editor, cx),
            None => self.render_cf_list(panel, cx),
        };

        docked_sidebar(
            "cond-fmt",
            "Conditional formatting",
            cx.listener(|this, _: &ClickEvent, _window, cx| {
                this.close_cond_fmt(cx);
            }),
            body,
        )
        .into_any_element()
    }

    /// The CF sidebar's **List mode** body (`ui_design.md §2.1`, BUG-3): an intro line naming the
    /// current selection, the rule rows **whose target range intersects the selection** (or one of
    /// two empty states), and the primary "+ Add rule".
    ///
    /// The list is **selection-scoped**: only rules intersecting the current selection are shown
    /// (a sheet can carry hundreds of rules), re-filtered live as the selection changes. Filtering
    /// is display-only — each surviving row keeps its GLOBAL priority position (for the first/last
    /// reorder-disable) and its true engine `index` (the handle the mutators target), so a filtered
    /// view never mis-targets a Raise/Lower/Delete.
    fn render_cf_list(&self, panel: &CondFmtPanel, cx: &mut Context<Self>) -> gpui::AnyElement {
        let selection_ref = freecell_core::format_selection_ref(&self.selection);

        let intro = div()
            .text_size(px(12.0))
            .text_color(rgb(MUTED_TEXT))
            .child(format!("Rules for {selection_ref}"));

        // The sidebar tracks the active sheet, so the (active-sheet) selection is the right scope.
        // If the panel is somehow scoped to a different sheet, fall back to showing every rule
        // (defensive — never hide rules against a selection from another sheet).
        let sel = self.selection.range();
        let filter_to_selection = panel.sheet == self.active_sheet;

        // Keep the ORIGINAL enumerate index alongside each surviving row: it is the rule's global
        // priority position (drives the first/last reorder-disable), preserved through the filter.
        let global_last = panel.rows.len().saturating_sub(1);
        let visible: Vec<(usize, &CfRuleView)> = panel
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| !filter_to_selection || cf_rule_intersects_selection(row, &sel))
            .collect();

        // Two distinct empty states (both above the Add button, `ui_design.md §2.1`): the sheet
        // truly has no rules, vs. it has rules but none apply to the current selection.
        let content = if panel.rows.is_empty() {
            div()
                .debug_selector(|| "cf-empty".to_string())
                .text_size(px(12.0))
                .text_color(rgb(MUTED_TEXT))
                .child("No rules on this sheet yet.")
                .into_any_element()
        } else if visible.is_empty() {
            div()
                .debug_selector(|| "cf-empty-selection".to_string())
                .text_size(px(12.0))
                .text_color(rgb(MUTED_TEXT))
                .child("No rules apply to the selected cells.")
                .into_any_element()
        } else {
            let mut list = div().flex().flex_col().gap_1();
            for (orig_i, row) in visible {
                list = list.child(self.render_cf_row(row, orig_i == 0, orig_i == global_last, cx));
            }
            list.into_any_element()
        };

        // "+ Add rule" opens the editor in add mode (`open_cf_editor(None)`).
        let add_rule = Button::new("cond-fmt-add-rule")
            .label("+ Add rule")
            .primary()
            .small()
            .disabled(self.degraded)
            .debug_selector(|| "cond-fmt-add-rule".to_string())
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.open_cf_editor(None, window, cx);
            }));

        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(intro)
            .child(content)
            .child(add_rule)
            .into_any_element()
    }

    /// One conditional-formatting rule as a List-mode row (`ui_design.md §2.1`,
    /// `components/cf_sidebar.md §5`): the preview swatch, a two-line summary/range, and the
    /// reorder/edit/delete controls. `is_first`/`is_last` are the row's ends in the
    /// priority-descending list (they gate the ▲/▼ reorder buttons); `row.index` is the rule's
    /// stable storage index (the handle the index-based mutators take).
    fn render_cf_row(
        &self,
        row: &CfRuleView,
        is_first: bool,
        is_last: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let index = row.index;
        let controls = cf_row_controls(row, is_first, is_last);

        let summary = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(px(1.0))
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(TEXT))
                    .child(row.summary.clone()),
            )
            .child(
                div()
                    .text_size(px(10.5))
                    .text_color(rgb(MUTED_TEXT))
                    .child(row.range.clone()),
            );

        let controls_row = div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap(px(1.0))
            .child(
                Button::new(gpui::ElementId::Name(format!("cf-row-{index}-up").into()))
                    .icon(Icon::empty().path("icons/chevron-up.svg"))
                    .tooltip("Move up")
                    .ghost()
                    .small()
                    .disabled(!controls.move_up)
                    .debug_selector(move || format!("cf-row-{index}-up"))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _window, _cx| {
                        this.raise_cf_rule(index);
                    })),
            )
            .child(
                Button::new(gpui::ElementId::Name(format!("cf-row-{index}-down").into()))
                    .icon(Icon::empty().path("icons/chevron-down.svg"))
                    .tooltip("Move down")
                    .ghost()
                    .small()
                    .disabled(!controls.move_down)
                    .debug_selector(move || format!("cf-row-{index}-down"))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _window, _cx| {
                        this.lower_cf_rule(index);
                    })),
            )
            .child(
                Button::new(gpui::ElementId::Name(format!("cf-row-{index}-edit").into()))
                    .icon(Icon::empty().path("icons/pencil.svg"))
                    .tooltip("Edit rule")
                    .ghost()
                    .small()
                    .disabled(!controls.edit)
                    .debug_selector(move || format!("cf-row-{index}-edit"))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.open_cf_editor(Some(index), window, cx);
                    })),
            )
            .child(
                Button::new(gpui::ElementId::Name(
                    format!("cf-row-{index}-delete").into(),
                ))
                .icon(Icon::empty().path("icons/trash-2.svg"))
                .tooltip("Delete rule")
                .ghost()
                .small()
                // Always enabled — even a deferred-family/Badge rule is deletable
                // (`functional_spec.md §9`); driven off the helper so every control has one source.
                .disabled(!controls.delete)
                .debug_selector(move || format!("cf-row-{index}-delete"))
                .on_click(cx.listener(
                    move |this, _: &ClickEvent, _window, _cx| {
                        this.delete_cf_rule(index);
                    },
                )),
            );

        div()
            .debug_selector(move || format!("cf-row-{index}"))
            .flex()
            .items_center()
            .gap_2()
            .py(px(4.0))
            .child(render_cf_preview(&row.preview))
            .child(summary)
            .child(controls_row)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chrome::view::test_support::*;
    use gpui::TestAppContext;

    // ---- Conditional-formatting sidebar (P4, `components/cf_sidebar.md`) -------------------

    /// A minimal published rule row for a given range (P4 doesn't render rows, but the sidebar
    /// carries them, so the re-scope / refresh tests assert on `rows`).
    fn cf_rule(range: &str) -> freecell_core::CfRuleView {
        freecell_core::CfRuleView {
            index: 0,
            range: range.to_string(),
            priority: 1,
            editable: true,
            summary: format!("Cell value > 100 ({range})"),
            preview: freecell_core::CfPreview::Highlight {
                fill: None,
                text_color: None,
            },
            spec: None,
        }
    }

    fn cf_rows_len(h: &Harness, cx: &mut TestAppContext) -> Option<usize> {
        upd(h, cx, |c, _w, _cx| {
            c.cond_fmt.as_ref().map(|p| p.rows.len())
        })
    }

    #[gpui::test]
    fn cond_fmt_button_toggles_sidebar(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        assert!(!upd(&h, cx, |c, _w, _cx| c.cond_fmt_open()));
        upd(&h, cx, |c, _w, cx| c.toggle_cond_fmt_sidebar(cx));
        assert!(
            upd(&h, cx, |c, _w, _cx| c.cond_fmt_open()),
            "the toggle opens the sidebar"
        );
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.cond_fmt_sheet()),
            Some(SheetId(0)),
            "it opens on the active sheet"
        );
        upd(&h, cx, |c, _w, cx| c.toggle_cond_fmt_sidebar(cx));
        assert!(
            !upd(&h, cx, |c, _w, _cx| c.cond_fmt_open()),
            "toggling again closes it"
        );
    }

    #[gpui::test]
    fn opening_cond_fmt_closes_chart_panel(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.open_chart_panel(
                ChartPanel::skeleton(SheetId(0), ChartId(7), true, ChartInsertKind::Line),
                window,
                cx,
            )
        });
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.chart_panel_target()),
            Some(ChartId(7))
        );
        upd(&h, cx, |c, _w, cx| c.toggle_cond_fmt_sidebar(cx));
        assert!(upd(&h, cx, |c, _w, _cx| c.cond_fmt_open()));
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.chart_panel_target()),
            None,
            "opening the CF sidebar closes the chart panel (shared right dock)"
        );
    }

    #[gpui::test]
    fn opening_chart_panel_closes_cond_fmt(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, _w, cx| c.toggle_cond_fmt_sidebar(cx));
        assert!(upd(&h, cx, |c, _w, _cx| c.cond_fmt_open()));
        upd(&h, cx, |c, window, cx| {
            c.open_chart_panel(
                ChartPanel::skeleton(SheetId(0), ChartId(7), true, ChartInsertKind::Line),
                window,
                cx,
            )
        });
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.chart_panel_target()),
            Some(ChartId(7))
        );
        assert!(
            !upd(&h, cx, |c, _w, _cx| c.cond_fmt_open()),
            "opening the chart panel closes the CF sidebar (shared right dock)"
        );
    }

    #[gpui::test]
    fn selection_change_does_not_close_cond_fmt(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, _w, cx| c.toggle_cond_fmt_sidebar(cx));
        assert!(upd(&h, cx, |c, _w, _cx| c.cond_fmt_open()));
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(3, 2)), window, cx)
        });
        assert!(
            upd(&h, cx, |c, _w, _cx| c.cond_fmt_open()),
            "a grid selection change must NOT close the CF sidebar (the range-pick exemption)"
        );
    }

    #[gpui::test]
    fn sheet_switch_rescopes_cond_fmt(cx: &mut TestAppContext) {
        let h = build(
            cx,
            vec![
                SheetTab::new(SheetId(0), "Sheet1"),
                SheetTab::new(SheetId(1), "Sheet2"),
            ],
            SheetId(0),
        );
        h.client
            .set_cond_fmt_rules(SheetId(1), vec![cf_rule("B2:B20")]);
        upd(&h, cx, |c, _w, cx| c.toggle_cond_fmt_sidebar(cx));
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.cond_fmt_sheet()),
            Some(SheetId(0))
        );
        assert_eq!(cf_rows_len(&h, cx), Some(0), "sheet 0 has no CF rules");
        // A window-driven sheet switch re-scopes the open sidebar to the new sheet + rebuilds rows.
        upd(&h, cx, |c, _w, cx| c.adopt_active_sheet(SheetId(1), cx));
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.cond_fmt_sheet()),
            Some(SheetId(1)),
            "the sidebar re-scopes to the new sheet"
        );
        assert_eq!(
            cf_rows_len(&h, cx),
            Some(1),
            "and rebuilds its rows from the new sheet's published rules"
        );
    }

    #[gpui::test]
    fn cond_fmt_updated_refreshes_rows(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, _w, cx| c.toggle_cond_fmt_sidebar(cx));
        assert_eq!(cf_rows_len(&h, cx), Some(0), "opens with no rules");
        // A CF mutation republishes the rule list; `refresh_cond_fmt` (the `CondFmtUpdated` handler)
        // rebuilds the sidebar's rows from the published map.
        h.client
            .set_cond_fmt_rules(SheetId(0), vec![cf_rule("A1:A10")]);
        upd(&h, cx, |c, _w, cx| c.refresh_cond_fmt(cx));
        assert_eq!(
            cf_rows_len(&h, cx),
            Some(1),
            "the sidebar rebuilt its rows from the published map"
        );
    }

    #[gpui::test]
    fn degrade_closes_cond_fmt(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, _w, cx| c.toggle_cond_fmt_sidebar(cx));
        assert!(upd(&h, cx, |c, _w, _cx| c.cond_fmt_open()));
        upd(&h, cx, |c, _w, cx| c.set_degraded(true, cx));
        assert!(
            !upd(&h, cx, |c, _w, _cx| c.cond_fmt_open()),
            "degrade closes the CF sidebar (like the chart panel)"
        );
    }

    // ---- Conditional-formatting rules list (P5, `components/cf_sidebar.md §5`) --------------

    /// A no-fill/no-text highlight preview — the common shape for the list-rendering tests.
    fn cf_highlight() -> CfPreview {
        CfPreview::Highlight {
            fill: None,
            text_color: None,
        }
    }

    /// The open sidebar's row summaries, top-to-bottom (priority order).
    fn cf_row_summaries(h: &Harness, cx: &mut TestAppContext) -> Vec<String> {
        upd(h, cx, |c, _w, _cx| {
            c.cond_fmt
                .as_ref()
                .expect("sidebar open")
                .rows
                .iter()
                .map(|r| r.summary.clone())
                .collect()
        })
    }

    #[test]
    fn cf_row_controls_reflect_position_and_editability() {
        let editable = cf_view(1, "A1:A10", "Cell value > 100", true, cf_highlight());
        let badge = cf_view(
            4,
            "C1:C9",
            "Data bar",
            false,
            CfPreview::Badge("Data bar".to_string()),
        );

        let first = cf_row_controls(&editable, true, false);
        assert!(
            !first.move_up,
            "the first (highest-priority) row can't move up"
        );
        assert!(first.move_down);
        let last = cf_row_controls(&editable, false, true);
        assert!(last.move_up);
        assert!(
            !last.move_down,
            "the last (lowest-priority) row can't move down"
        );
        let middle = cf_row_controls(&editable, false, false);
        assert!(middle.move_up && middle.move_down);
        assert!(
            first.edit && first.delete,
            "an editable highlight rule can be edited AND deleted"
        );

        let badge_controls = cf_row_controls(&badge, false, false);
        assert!(
            !badge_controls.edit,
            "a deferred-family Badge rule can't be edited"
        );
        assert!(
            badge_controls.delete,
            "but a deferred-family Badge rule can still be deleted"
        );
    }

    #[gpui::test]
    fn cf_list_renders_one_row_per_rule(cx: &mut TestAppContext) {
        let h = tall_sheet(cx);
        h.client.set_cond_fmt_rules(
            SheetId(0),
            vec![
                cf_view(5, "A1:A10", "Cell value > 100", true, cf_highlight()),
                cf_view(
                    2,
                    "B2:B20",
                    "3-color scale",
                    true,
                    CfPreview::ColorScale {
                        colors: vec![Rgb::from_hex(0x63BE7B), Rgb::from_hex(0xF8696B)],
                    },
                ),
                cf_view(
                    9,
                    "C1:C9",
                    "Data bar",
                    false,
                    CfPreview::Badge("Data bar".to_string()),
                ),
            ],
        );
        upd(&h, cx, |c, _w, cx| {
            c.toggle_cond_fmt_sidebar(cx);
            // The list is selection-scoped (BUG-3): widen the selection (A1:C20) to cover all
            // three rule ranges so every published row is in scope.
            c.selection = SelectionModel {
                anchor: cell(0, 0),
                active: cell(19, 2),
            };
        });
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        // One row painted per published rule, keyed by its stable storage index (5/2/9).
        for sel in ["cf-row-5", "cf-row-2", "cf-row-9"] {
            assert!(
                vcx.debug_bounds(sel).is_some(),
                "{sel} must render one row per rule"
            );
        }
        assert!(
            vcx.debug_bounds("cf-empty").is_none(),
            "no empty state renders while rules exist"
        );
    }

    #[gpui::test]
    fn cf_empty_state_shown_when_no_rules(cx: &mut TestAppContext) {
        let h = tall_sheet(cx);
        upd(&h, cx, |c, _w, cx| c.toggle_cond_fmt_sidebar(cx));
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        assert!(
            vcx.debug_bounds("cf-empty").is_some(),
            "the empty state shows when the sheet carries no rules"
        );
        assert!(vcx.debug_bounds("cf-row-0").is_none(), "no rows render");
        assert!(
            vcx.debug_bounds("cf-empty-selection").is_none(),
            "the zero-rules state is the sheet-empty message, not the no-rules-in-selection one"
        );
        assert!(
            vcx.debug_bounds("cond-fmt-add-rule").is_some(),
            "the '+ Add rule' button stays available in the empty state"
        );
    }

    #[gpui::test]
    fn cf_list_filters_rows_to_selection(cx: &mut TestAppContext) {
        // BUG-3: the list shows only rules whose target range intersects the current selection,
        // and re-filters live as the selection moves. Two rules with disjoint ranges (A over
        // A1:A5, B over C1:C5); the selection picks exactly one at a time.
        let h = tall_sheet(cx);
        h.client.set_cond_fmt_rules(
            SheetId(0),
            vec![
                cf_view(7, "A1:A5", "Rule A", true, cf_highlight()),
                cf_view(3, "C1:C5", "Rule B", true, cf_highlight()),
            ],
        );
        upd(&h, cx, |c, window, cx| {
            c.toggle_cond_fmt_sidebar(cx);
            // Selection inside Rule A's range (A2).
            c.on_selection_changed(SelectionModel::single(cell(1, 0)), window, cx);
        });
        {
            let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
            vcx.run_until_parked();
            assert!(
                vcx.debug_bounds("cf-row-7").is_some(),
                "Rule A (A1:A5) intersects the A2 selection → its row shows"
            );
            assert!(
                vcx.debug_bounds("cf-row-3").is_none(),
                "Rule B (C1:C5) does not intersect the A2 selection → its row is hidden"
            );
            assert!(
                vcx.debug_bounds("cf-empty-selection").is_none(),
                "a matching rule is shown, so no empty state"
            );
        }
        // Move the selection into Rule B's range (C3): the list re-filters to the other row.
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(2, 2)), window, cx);
        });
        {
            let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
            vcx.run_until_parked();
            assert!(
                vcx.debug_bounds("cf-row-3").is_some(),
                "Rule B (C1:C5) now intersects the C3 selection → its row shows"
            );
            assert!(
                vcx.debug_bounds("cf-row-7").is_none(),
                "Rule A (A1:A5) no longer intersects → its row is hidden"
            );
        }
    }

    #[gpui::test]
    fn cf_whole_column_rule_scopes_by_column(cx: &mut TestAppContext) {
        // BUG-3 regression: Excel writes whole-column CF as sqref "A:A" (verbatim on load).
        // The rule must show whenever the selection touches column A and hide otherwise — it
        // used to vanish for EVERY selection because "A:A" doesn't parse as a plain A1 range.
        let h = tall_sheet(cx);
        h.client.set_cond_fmt_rules(
            SheetId(0),
            vec![cf_view(7, "A:A", "Whole-column A", true, cf_highlight())],
        );
        upd(&h, cx, |c, window, cx| {
            c.toggle_cond_fmt_sidebar(cx);
            // Selection deep inside column A (A501).
            c.on_selection_changed(SelectionModel::single(cell(500, 0)), window, cx);
        });
        {
            let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
            vcx.run_until_parked();
            assert!(
                vcx.debug_bounds("cf-row-7").is_some(),
                "the whole-column rule \"A:A\" intersects a column-A selection → its row shows"
            );
        }
        // Move to a selection confined to a different column (B2): the rule hides.
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(1, 1)), window, cx);
        });
        {
            let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
            vcx.run_until_parked();
            assert!(
                vcx.debug_bounds("cf-row-7").is_none(),
                "the whole-column rule \"A:A\" does not cover column B → its row is hidden"
            );
        }
    }

    #[gpui::test]
    fn cf_whole_row_rule_scopes_by_row(cx: &mut TestAppContext) {
        // BUG-3 regression: Excel writes whole-row CF as sqref "1:1" (verbatim on load). The
        // rule must show for a row-1 selection and hide for a different row.
        let h = tall_sheet(cx);
        h.client.set_cond_fmt_rules(
            SheetId(0),
            vec![cf_view(4, "1:1", "Whole-row 1", true, cf_highlight())],
        );
        upd(&h, cx, |c, window, cx| {
            c.toggle_cond_fmt_sidebar(cx);
            // Selection in row 1 (C1) but a non-A column, to prove it's the ROW that matches.
            c.on_selection_changed(SelectionModel::single(cell(0, 2)), window, cx);
        });
        {
            let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
            vcx.run_until_parked();
            assert!(
                vcx.debug_bounds("cf-row-4").is_some(),
                "the whole-row rule \"1:1\" intersects a row-1 selection → its row shows"
            );
        }
        // Move to row 5 (A5): the rule hides.
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(4, 0)), window, cx);
        });
        {
            let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
            vcx.run_until_parked();
            assert!(
                vcx.debug_bounds("cf-row-4").is_none(),
                "the whole-row rule \"1:1\" does not cover row 5 → its row is hidden"
            );
        }
    }

    #[gpui::test]
    fn cf_unparseable_rule_fails_open(cx: &mut TestAppContext) {
        // BUG-3 regression / fail-open: a rule whose sqref shape is unrecognized must never
        // silently vanish (that would make it unmanageable). It shows for ANY selection.
        let h = tall_sheet(cx);
        h.client.set_cond_fmt_rules(
            SheetId(0),
            vec![cf_view(2, "###", "Mystery range", true, cf_highlight())],
        );
        upd(&h, cx, |c, window, cx| {
            c.toggle_cond_fmt_sidebar(cx);
            c.on_selection_changed(SelectionModel::single(cell(1, 0)), window, cx);
        });
        {
            let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
            vcx.run_until_parked();
            assert!(
                vcx.debug_bounds("cf-row-2").is_some(),
                "an unparseable range fails open at A2 → the row still shows"
            );
        }
        // A totally different, distant selection: still shown (fail-open, not selection-scoped).
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(900, 20)), window, cx);
        });
        {
            let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
            vcx.run_until_parked();
            assert!(
                vcx.debug_bounds("cf-row-2").is_some(),
                "an unparseable range stays visible for any selection (fail-open)"
            );
        }
    }

    #[gpui::test]
    fn cf_no_rules_apply_shows_selection_empty_state(cx: &mut TestAppContext) {
        // BUG-3: rules exist but none intersect the selection → the distinct "no rules apply"
        // empty state (NOT the sheet-empty one), and the "+ Add rule" button stays available.
        let h = tall_sheet(cx);
        h.client.set_cond_fmt_rules(
            SheetId(0),
            vec![
                cf_view(7, "A1:A5", "Rule A", true, cf_highlight()),
                cf_view(3, "C1:C5", "Rule B", true, cf_highlight()),
            ],
        );
        upd(&h, cx, |c, window, cx| {
            c.toggle_cond_fmt_sidebar(cx);
            // E1 intersects neither A1:A5 nor C1:C5.
            c.on_selection_changed(SelectionModel::single(cell(0, 4)), window, cx);
        });
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        assert!(
            vcx.debug_bounds("cf-empty-selection").is_some(),
            "no rule intersects the selection → the selection empty state shows"
        );
        assert!(
            vcx.debug_bounds("cf-empty").is_none(),
            "the sheet DOES have rules, so it is not the sheet-empty state"
        );
        assert!(
            vcx.debug_bounds("cf-row-7").is_none() && vcx.debug_bounds("cf-row-3").is_none(),
            "no rule rows render"
        );
        assert!(
            vcx.debug_bounds("cond-fmt-add-rule").is_some(),
            "the '+ Add rule' button stays available in the selection empty state"
        );
    }

    #[gpui::test]
    fn cf_filtered_row_keeps_global_index_and_priority(cx: &mut TestAppContext) {
        // BUG-3: filtering is display-only. Two rules — A (idx5, A1:A10, GLOBAL first/highest
        // priority) and B (idx2, C1:C10, GLOBAL last/lowest). The selection (C3) shows only B.
        // B is the GLOBAL last, so its move-down stays disabled even though it is the only row;
        // its move-up and delete still target its ORIGINAL engine index (2), not a filtered slot.
        let h = tall_sheet(cx);
        h.client.set_cond_fmt_rules(
            SheetId(0),
            vec![
                cf_view(5, "A1:A10", "Rule A", true, cf_highlight()),
                cf_view(2, "C1:C10", "Rule B", true, cf_highlight()),
            ],
        );
        upd(&h, cx, |c, window, cx| {
            c.toggle_cond_fmt_sidebar(cx);
            c.on_selection_changed(SelectionModel::single(cell(2, 2)), window, cx);
            // C3 → only B
        });
        h.client.take_commands();
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        assert!(
            vcx.debug_bounds("cf-row-2").is_some(),
            "the intersecting rule (idx2) is the only row shown"
        );
        assert!(
            vcx.debug_bounds("cf-row-5").is_none(),
            "the non-intersecting rule (idx5) is filtered out"
        );
        // Move-down is disabled (idx2 is the GLOBAL last row) — a click is inert.
        let down = vcx
            .debug_bounds("cf-row-2-down")
            .expect("the shown row's move-down is painted");
        vcx.simulate_click(down.center(), Modifiers::default());
        assert!(
            h.client.take_commands().is_empty(),
            "the shown row is GLOBAL-last, so its move-down stays disabled even when it is the \
             only visible row"
        );
        // Move-up is enabled (idx2 is not the GLOBAL first) and raises by the ORIGINAL index.
        let up = vcx
            .debug_bounds("cf-row-2-up")
            .expect("the shown row's move-up is painted");
        vcx.simulate_click(up.center(), Modifiers::default());
        assert!(
            matches!(
                h.client.take_commands().as_slice(),
                [Command::RaiseCondFmtPriority {
                    sheet: SheetId(0),
                    index: 2
                }]
            ),
            "move-up targets the rule's ORIGINAL engine index (2), not a filtered position"
        );
        // Delete likewise targets the original engine index.
        let del = vcx
            .debug_bounds("cf-row-2-delete")
            .expect("the shown row's delete is painted");
        vcx.simulate_click(del.center(), Modifiers::default());
        assert!(
            matches!(
                h.client.take_commands().as_slice(),
                [Command::DeleteCondFmt {
                    sheet: SheetId(0),
                    index: 2
                }]
            ),
            "delete targets the rule's ORIGINAL engine index (2) from a filtered view"
        );
    }

    #[gpui::test]
    fn cf_delete_sends_delete_command(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        h.client.set_cond_fmt_rules(
            SheetId(0),
            vec![cf_view(3, "A1:A10", "Rule", true, cf_highlight())],
        );
        upd(&h, cx, |c, _w, cx| c.toggle_cond_fmt_sidebar(cx));
        h.client.take_commands();
        upd(&h, cx, |c, _w, _cx| c.delete_cf_rule(3));
        assert!(
            matches!(
                h.client.take_commands().as_slice(),
                [Command::DeleteCondFmt {
                    sheet: SheetId(0),
                    index: 3
                }]
            ),
            "delete sends DeleteCondFmt for the rule's storage index"
        );
    }

    #[gpui::test]
    fn cf_move_up_sends_raise(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        h.client.set_cond_fmt_rules(
            SheetId(0),
            vec![cf_view(2, "A1:A10", "Rule", true, cf_highlight())],
        );
        upd(&h, cx, |c, _w, cx| c.toggle_cond_fmt_sidebar(cx));
        h.client.take_commands();
        upd(&h, cx, |c, _w, _cx| c.raise_cf_rule(2));
        assert!(
            matches!(
                h.client.take_commands().as_slice(),
                [Command::RaiseCondFmtPriority {
                    sheet: SheetId(0),
                    index: 2
                }]
            ),
            "move-up sends RaiseCondFmtPriority for the rule's storage index"
        );
    }

    #[gpui::test]
    fn cf_move_down_sends_lower(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        h.client.set_cond_fmt_rules(
            SheetId(0),
            vec![cf_view(2, "A1:A10", "Rule", true, cf_highlight())],
        );
        upd(&h, cx, |c, _w, cx| c.toggle_cond_fmt_sidebar(cx));
        h.client.take_commands();
        upd(&h, cx, |c, _w, _cx| c.lower_cf_rule(2));
        assert!(
            matches!(
                h.client.take_commands().as_slice(),
                [Command::LowerCondFmtPriority {
                    sheet: SheetId(0),
                    index: 2
                }]
            ),
            "move-down sends LowerCondFmtPriority for the rule's storage index"
        );
    }

    #[gpui::test]
    fn cf_commands_noop_when_sidebar_closed(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        // The sidebar is closed → no target sheet → the mutators are no-ops (they never target the
        // active sheet blindly).
        upd(&h, cx, |c, _w, _cx| {
            c.raise_cf_rule(0);
            c.lower_cf_rule(0);
            c.delete_cf_rule(0);
        });
        assert!(
            h.client.take_commands().is_empty(),
            "a closed CF sidebar sends no CF command"
        );
    }

    #[gpui::test]
    fn cf_delete_button_click_sends_delete(cx: &mut TestAppContext) {
        let h = tall_sheet(cx);
        h.client.set_cond_fmt_rules(
            SheetId(0),
            vec![
                cf_view(5, "A1:A10", "Rule A", true, cf_highlight()),
                cf_view(2, "B2:B20", "Rule B", true, cf_highlight()),
            ],
        );
        upd(&h, cx, |c, _w, cx| c.toggle_cond_fmt_sidebar(cx));
        h.client.take_commands();
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        let del = vcx
            .debug_bounds("cf-row-5-delete")
            .expect("the row's delete control is painted");
        vcx.simulate_click(del.center(), Modifiers::default());
        assert!(
            matches!(
                h.client.take_commands().as_slice(),
                [Command::DeleteCondFmt {
                    sheet: SheetId(0),
                    index: 5
                }]
            ),
            "clicking a row's delete button sends DeleteCondFmt for that row (button → method wiring)"
        );
    }

    #[gpui::test]
    fn cf_first_row_move_up_disabled(cx: &mut TestAppContext) {
        let h = tall_sheet(cx);
        h.client.set_cond_fmt_rules(
            SheetId(0),
            vec![
                cf_view(5, "A1:A10", "Rule A", true, cf_highlight()),
                cf_view(2, "B2:B20", "Rule B", true, cf_highlight()),
            ],
        );
        upd(&h, cx, |c, _w, cx| {
            c.toggle_cond_fmt_sidebar(cx);
            // Selection-scoped list (BUG-3): widen to A1:B20 so both rules are shown.
            c.selection = SelectionModel {
                anchor: cell(0, 0),
                active: cell(19, 1),
            };
        });
        h.client.take_commands();
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        // First row (highest priority) — its move-up is disabled, so a click sends nothing.
        let up_first = vcx
            .debug_bounds("cf-row-5-up")
            .expect("first row move-up painted");
        vcx.simulate_click(up_first.center(), Modifiers::default());
        assert!(
            h.client.take_commands().is_empty(),
            "the first row's move-up is disabled (a click is inert)"
        );
        // A lower row's move-up is enabled and raises its priority.
        let up_second = vcx
            .debug_bounds("cf-row-2-up")
            .expect("second row move-up painted");
        vcx.simulate_click(up_second.center(), Modifiers::default());
        assert!(
            matches!(
                h.client.take_commands().as_slice(),
                [Command::RaiseCondFmtPriority {
                    sheet: SheetId(0),
                    index: 2
                }]
            ),
            "a non-first row's move-up raises its priority"
        );
    }

    #[gpui::test]
    fn cf_last_row_move_down_disabled(cx: &mut TestAppContext) {
        let h = tall_sheet(cx);
        h.client.set_cond_fmt_rules(
            SheetId(0),
            vec![
                cf_view(5, "A1:A10", "Rule A", true, cf_highlight()),
                cf_view(2, "B2:B20", "Rule B", true, cf_highlight()),
            ],
        );
        upd(&h, cx, |c, _w, cx| {
            c.toggle_cond_fmt_sidebar(cx);
            // Selection-scoped list (BUG-3): widen to A1:B20 so both rules are shown.
            c.selection = SelectionModel {
                anchor: cell(0, 0),
                active: cell(19, 1),
            };
        });
        h.client.take_commands();
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        // Last row (lowest priority) — its move-down is disabled, so a click sends nothing.
        let down_last = vcx
            .debug_bounds("cf-row-2-down")
            .expect("last row move-down painted");
        vcx.simulate_click(down_last.center(), Modifiers::default());
        assert!(
            h.client.take_commands().is_empty(),
            "the last row's move-down is disabled (a click is inert)"
        );
        // A higher row's move-down is enabled and lowers its priority.
        let down_first = vcx
            .debug_bounds("cf-row-5-down")
            .expect("first row move-down painted");
        vcx.simulate_click(down_first.center(), Modifiers::default());
        assert!(
            matches!(
                h.client.take_commands().as_slice(),
                [Command::LowerCondFmtPriority {
                    sheet: SheetId(0),
                    index: 5
                }]
            ),
            "a non-last row's move-down lowers its priority"
        );
    }

    #[gpui::test]
    fn cf_list_reorders_after_cond_fmt_updated(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        h.client.set_cond_fmt_rules(
            SheetId(0),
            vec![
                cf_view(0, "A1:A10", "Rule A", true, cf_highlight()),
                cf_view(1, "B1:B10", "Rule B", true, cf_highlight()),
            ],
        );
        upd(&h, cx, |c, _w, cx| c.toggle_cond_fmt_sidebar(cx));
        assert_eq!(
            cf_row_summaries(&h, cx).join(","),
            "Rule A,Rule B",
            "rows render in the published (priority) order"
        );
        // A raise/lower swapped their priority; the worker republishes the reordered list. The
        // `CondFmtUpdated` refresh rebuilds the rows in the new order.
        h.client.set_cond_fmt_rules(
            SheetId(0),
            vec![
                cf_view(1, "B1:B10", "Rule B", true, cf_highlight()),
                cf_view(0, "A1:A10", "Rule A", true, cf_highlight()),
            ],
        );
        upd(&h, cx, |c, _w, cx| c.refresh_cond_fmt(cx));
        assert_eq!(
            cf_row_summaries(&h, cx).join(","),
            "Rule B,Rule A",
            "the list reflects the republished order after CondFmtUpdated"
        );
    }
}
