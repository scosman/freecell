//! The chrome shell: what holds the domains together rather than any one of them.
//!
//! The [`Render`]/[`Focusable`] impls and the top-level layout (action row → data row → find
//! bar → grid body → tab bar), the action row itself and its horizontal scroller, the overlay
//! stack and its click-away backdrop, the dropdown-anchor probe, the worker-event router that
//! fans events out to every domain, the grid-body installation and selection plumbing, and the
//! degraded (read-only) state.
//!
//! Moved verbatim out of the single-file `chrome/view.rs`
//! (`specs/projects/chrome-view-split`).

use super::*;

impl ChromeView {
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
        // Click-away closes the chart edit panel (post-v1 Batch 2, item 12): a grid click on a
        // cell/header/empty area (or a paste / sheet switch) routes here and dismisses the panel.
        // A click on *another chart* does NOT route here — the grid emits `ChartSelected` instead,
        // which re-points the panel (a switch, not a close) — and the panel's own controls never
        // change the grid selection, so they can't dismiss it either.
        //
        // The CF sidebar is deliberately EXEMPT from this click-away close (`components/cf_sidebar.md
        // §4`): it stays open across selection changes so the user can pick the "Applies to" range by
        // selecting cells. It re-scopes to a new sheet via the sheet-switch path, not here.
        self.close_chart_panel(cx);
        // Refresh the tab-bar selection-stats readout for the new selection (debounced).
        self.request_selection_stats(cx);
        cx.notify();
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
            // Only honor results while the bar is open (a late reply after close is dropped).
            WorkerEvent::FindResults { matches } if self.find_open => {
                self.matches = matches;
                self.match_idx = self.first_match_from_selection();
                self.select_current_match(window, cx);
                cx.notify();
            }
            WorkerEvent::ReplacedCount { n } => {
                if self.pending_replace_all {
                    self.pending_replace_all = false;
                    self.replaced_notice = Some(n);
                }
                // Re-scan so the match set + counter reflect the post-replace state and the cursor
                // advances past a (now-changed) cell (`functional_spec.md §4.4`).
                if self.find_open {
                    self.recompute_matches(cx);
                }
                cx.notify();
            }
            // Keep only the reply for the latest request — a superseded selection bumped
            // `stats_seq`, so an older reply (or one after a collapse to a single cell) falls
            // through the guard to the `_` arm and is dropped.
            WorkerEvent::SelectionStats { req_id, stats } if req_id == self.stats_seq => {
                self.selection_stats = Some(stats);
                cx.notify();
            }
            // Published/Saved/SaveFailed/StyleCacheUpdated/other EditRejected reasons /
            // degraded are the window's concern (Phase 11 dirty state + modals).
            _ => {}
        }
    }

    // ---- Read accessors (tests + render) --------------------------------------------------

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
                self.num_fmt_more_open = false;
                self.font_family_open = false;
                self.font_size_open = false;
                self.borders_open = false;
                self.chart_menu_open = false;
                self.chart_panel = None;
                self.cond_fmt = None;
            }
            cx.notify();
        }
    }

    /// Whether the worker is degraded (read-only) — all mutating action-bar controls disable.
    pub fn is_degraded(&self) -> bool {
        self.degraded
    }

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

    fn render_action_row(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Every mutating control disables in degraded/read-only mode (`functional_spec.md §6`).
        let disabled = self.degraded;

        // Each button renders a FreeCell-vendored Lucide icon (`shell::assets`) via
        // gpui-component's `Icon` (`icons/<name>.svg`); `Icon` tints it to the button's
        // foreground so the pressed/disabled states read the same as the former text glyphs.
        let toggle = |id: &'static str,
                      icon_path: &'static str,
                      tooltip: &'static str,
                      pressed: bool,
                      attr: StyleAttr,
                      cx: &mut Context<Self>| {
            Button::new(id)
                .icon(Icon::empty().path(icon_path))
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
                         icon_path: &'static str,
                         cx: &mut Context<Self>| {
            Button::new(id)
                .icon(Icon::empty().path(icon_path))
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
                          icon_path: &'static str,
                          cx: &mut Context<Self>| {
            Button::new(id)
                .icon(Icon::empty().path(icon_path))
                .tooltip(tooltip)
                .ghost()
                .small()
                .disabled(disabled)
                .selected(self.valign_active(valign))
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.apply_valign(valign, window, cx);
                }))
        };

        // The button groups are the horizontal scroller's *content*: they sit at their exact
        // natural width so a small window makes them overflow + scroll (chevrons) rather than
        // compressing the controls (`functional_spec.md §9B`, call site 1). `flex_shrink_0` is what
        // holds that natural width — flexbox's default shrink=1 would otherwise squish the buttons
        // to fit; it (not a hand-estimated `min_w`) is the "scroll, don't squish" guarantee, so the
        // chevrons appear ONLY when the controls genuinely don't fit (`functional_spec.md §10.2`).
        let groups = div()
            .flex()
            .items_center()
            .gap_1()
            .debug_selector(|| "action-row-groups".to_string())
            // Never wrap or shrink; the scroller scrolls the groups when they don't fit.
            .flex_shrink_0()
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
                "icons/bold.svg",
                "Bold ⌘B",
                self.bold_active(),
                StyleAttr::Bold,
                cx,
            ))
            .child(toggle(
                "italic",
                "icons/italic.svg",
                "Italic ⌘I",
                self.italic_active(),
                StyleAttr::Italic,
                cx,
            ))
            .child(toggle(
                "underline",
                "icons/underline.svg",
                "Underline ⌘U",
                self.underline_active(),
                StyleAttr::Underline,
                cx,
            ))
            // Strikethrough, appended to the B/I/U toggle group
            // (`ui_design.md §1.1`, `functional_spec.md §1`).
            .child(toggle(
                "strikethrough",
                "icons/strikethrough.svg",
                "Strikethrough",
                self.strikethrough_active(),
                StyleAttr::Strikethrough,
                cx,
            ))
            .child(action_divider())
            // Text color · Fill:
            .child(
                self.anchored_trigger(
                    Anchor::TextColor,
                    Button::new("text-color")
                        .icon(Icon::empty().path("icons/baseline.svg"))
                        .label("▾")
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
                        .icon(Icon::empty().path("icons/paint-bucket.svg"))
                        .label("▾")
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
                        .icon(Icon::empty().path("icons/grid-2x2.svg"))
                        .label("▾")
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
            .child(align_btn(
                "align-left",
                "Align left",
                Align::Left,
                "icons/text-align-start.svg",
                cx,
            ))
            .child(align_btn(
                "align-center",
                "Align center",
                Align::Center,
                "icons/text-align-center.svg",
                cx,
            ))
            .child(align_btn(
                "align-right",
                "Align right",
                Align::Right,
                "icons/text-align-end.svg",
                cx,
            ))
            .child(action_divider())
            // Vertical alignment — its own group after horizontal align (`ui_design.md §1.1`):
            .child(valign_btn(
                "valign-top",
                "Align top",
                VAlign::Top,
                "icons/arrow-up-to-line.svg",
                cx,
            ))
            .child(valign_btn(
                "valign-middle",
                "Align middle",
                VAlign::Center,
                "icons/separator-horizontal.svg",
                cx,
            ))
            .child(valign_btn(
                "valign-bottom",
                "Align bottom",
                VAlign::Bottom,
                "icons/arrow-down-from-line.svg",
                cx,
            ))
            // Wrap text — grouped with vertical alignment, right of Align bottom.
            .child(toggle(
                "wrap",
                "icons/text-wrap.svg",
                "Wrap text",
                self.wrap_active(),
                StyleAttr::WrapText,
                cx,
            ))
            .child(action_divider())
            // Merge / Unmerge toggle — a cell-layout concern grouped after wrap (`ui_design.md §1`).
            // Built directly (not the shared `toggle` closure, which drives character styles): it has
            // its own `on_click`, and a distinct disabled rule (a lone 1×1 not in any merge is
            // disabled — nothing to toggle — as well as the degraded case). One icon both directions;
            // the pressed state + tooltip swap convey mode.
            .child(
                Button::new("merge")
                    .icon(Icon::empty().path("icons/table-cells-merge.svg"))
                    .tooltip(if self.merge_active() {
                        "Unmerge cells"
                    } else {
                        "Merge cells"
                    })
                    .ghost()
                    .small()
                    .disabled(self.merge_disabled())
                    .selected(self.merge_active())
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.toggle_merge(window, cx);
                    })),
            )
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
                    .icon(Icon::empty().path("icons/decimals-arrow-right.svg"))
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
                    .icon(Icon::empty().path("icons/decimals-arrow-left.svg"))
                    .tooltip("Decrease decimals")
                    .ghost()
                    .small()
                    .disabled(!self.decrease_decimals_enabled())
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.bump_decimals(-1, window, cx);
                    })),
            )
            // Thousands-separator toggle (Phase 6, D6.2): adds/removes the `,` grouping on the
            // active cell's format; `selected` when grouping is on, disabled when it can't be
            // safely toggled (General/Text/Date/Time/multi-section).
            .child(
                Button::new("thousands-sep")
                    .icon(Icon::empty().path("icons/thousands-separator.svg"))
                    .tooltip("Thousands separator")
                    .ghost()
                    .small()
                    .disabled(!self.toggle_thousands_enabled())
                    .selected(self.thousands_active())
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.toggle_thousands_separator(window, cx);
                    })),
            )
            .child(action_divider())
            // Insert-chart menu — the action-bar chart-type glyph menu (`ui_design.md §3.1`, P17).
            .child(
                self.anchored_trigger(
                    Anchor::Chart,
                    Button::new("insert-chart")
                        .icon(Icon::empty().path("icons/chart-column.svg"))
                        .label("▾")
                        .tooltip("Insert chart")
                        .ghost()
                        .small()
                        .disabled(disabled)
                        .selected(self.chart_menu_open)
                        .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                            this.toggle_chart_menu(cx);
                        })),
                    cx,
                ),
            )
            // Conditional formatting — toggles the right-docked CF sidebar directly (no menu, unlike
            // the chart-insert glyph). `selected` (accent) while the sidebar is open; disabled in
            // degraded/read-only mode (`components/cf_sidebar.md §5`, `ui_design.md §1`).
            .child(
                Button::new("cond-fmt")
                    .icon(Icon::empty().path("icons/split.svg"))
                    .tooltip("Conditional formatting")
                    .ghost()
                    .small()
                    .disabled(disabled)
                    .selected(self.cond_fmt_open())
                    .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                        this.toggle_cond_fmt_sidebar(cx);
                    })),
            )
            .child(action_divider())
            // Find & Replace trigger (`ui_design.md §2`): toggles the find bar; `selected` (accent)
            // while it is open, so it reads as a toggle. `icons/search.svg` resolves from the
            // gpui-component bundle (the magnifier the bundle already ships + tints).
            .child(
                // Find is a *read* — it stays available in degraded/read-only mode (only the bar's
                // Replace / Replace All are gated on `degraded`).
                Button::new("find")
                    .icon(Icon::empty().path("icons/search.svg"))
                    .tooltip("Find & Replace (⌘F)")
                    .ghost()
                    .small()
                    .selected(self.find_open)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.toggle_find(window, cx);
                    })),
            );

        // Frame: the fixed-height, full-width action bar hosting the scrollable groups + the
        // right-docked evaluating spinner (`ui_design.md §3.1`). The scroller is `flex_1`, so it
        // fills up to the spinner — the spinner stays docked right exactly as the old `flex_1`
        // spacer did — and shows chevrons only when the groups overflow (`functional_spec.md §9B`).
        div()
            .flex()
            .items_center()
            .w_full()
            .h(px(ACTION_ROW_H))
            .px_2()
            .bg(rgb(CHROME_BG))
            .border_b_1()
            .border_color(rgb(HAIRLINE))
            .child(h_scroller(
                "action-row",
                &self.action_scroller,
                window,
                groups,
            ))
            .when(self.eval.spinner(), |row| row.child(Spinner::new().small()))
    }

    /// The floating overlays (fill popover, tab context menu, delete-confirm modal), each a
    /// `ChromeView`-owned panel over a dismiss backdrop.
    fn render_overlays(&self, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
        let mut overlays: Vec<gpui::AnyElement> = Vec::new();

        // The right-docked chart edit panel is pushed FIRST so it is the **bottom-most** overlay:
        // gpui paints sibling overlays in vector order (later = on top), so every action-bar
        // dropdown/popover below — the new-chart menu in particular (post-v1 Batch 3, item 10) —
        // floats ABOVE the docked panel instead of dropping behind it. The panel is a persistent
        // docked surface; the transient popovers layer on top of it.
        if self.chart_panel.is_some() {
            overlays.push(self.render_chart_panel(cx));
        }
        // The CF sidebar shares the right dock and is mutually exclusive with the chart panel, so at
        // most one of these ever renders; like the chart panel it is a persistent docked surface, so
        // it is pushed early to stay below the transient action-bar popovers.
        if self.cond_fmt.is_some() {
            overlays.push(self.render_cond_fmt_sidebar(cx));
        }

        // The data-row cap popover anchors under the data row only when it is the active editor;
        // an in-cell cap error is shown under the overlay by the grid (`edit_controller.md §4.2`).
        // The completion list + signature hint anchor at the same spot but yield to the cap error
        // (it means the edit can't commit — `functional_spec.md §1`).
        if self.edit.origin() == EditOrigin::DataRow {
            if let Some(message) = self.cap_error_message() {
                overlays.push(self.render_cap_error_popover(message));
            } else if let Some(list) = self.autocomplete_display() {
                overlays.push(self.render_autocomplete_popover(&list, cx));
            } else if let Some(template) = self.edit.sig_hint() {
                overlays.push(self.render_sig_hint_popover(template));
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
        if self.chart_menu_open {
            overlays.push(self.render_chart_menu(cx));
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

    pub(super) fn backdrop(
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
}

impl Focusable for ChromeView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ChromeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("freecell-chrome")
            .track_focus(&self.focus_handle)
            .relative()
            .flex()
            .flex_col()
            .w_full()
            // Fill the available height when hosting the grid, so the grid slot can flex.
            .when(self.body.is_some(), |d| d.flex_1().min_h_0())
            // `window` threads to the two `h_scroller` call sites (action row + tab bar) so a chevron
            // click can drive an animated slide via `request_animation_frame` (D10.2).
            .child(self.render_action_row(window, cx))
            .child(self.render_data_row(cx))
            // The find/replace bar sits directly below the data row and above the grid, pushing the
            // grid down when open (`functional_spec.md §4.1`, `ui_design.md §1`).
            .children(self.find_open.then(|| self.render_find_bar(cx)))
            // The grid body fills the space between the data row and the tab bar
            // (`ui_design.md §3`: action → data → grid → tabs).
            .when_some(self.body.clone(), |d, body| {
                d.child(div().flex_1().min_h_0().w_full().child(body))
            })
            .child(self.render_tab_bar(window, cx))
            .children(self.render_overlays(cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chrome::view::test_support::*;
    use freecell_core::SelectionModel;
    use freecell_engine::WorkerEvent;
    use gpui::{Modifiers, TestAppContext};

    // ---- Horizontal scroller (action bar + tab strip, `functional_spec.md §9B`) ------------

    /// A tab list long enough to overflow a narrow window.
    fn many_sheets(n: u32) -> Vec<SheetTab> {
        (0..n)
            .map(|i| SheetTab::new(SheetId(i), format!("Sheet{i}")))
            .collect()
    }

    /// Paint the window twice: the scroll handle measures overflow on the first paint, so the
    /// chevron affordance only appears on the second (the one-frame gpui scroll-handle lag noted in
    /// `h_scroller`).
    fn paint_twice(vcx: &mut gpui::VisualTestContext) {
        vcx.run_until_parked();
        vcx.update(|window, _| window.refresh());
        vcx.run_until_parked();
    }

    /// Advance the chevron slide by `n` animation frames. `request_animation_frame`'s `on_next_frame`
    /// callback only fires from the platform's frame loop, which the test window stubs out — so a
    /// test drives each frame manually with a `refresh` + `run_until_parked` (each redraw runs
    /// `h_scroller`'s one-frame `anim_step`).
    fn pump_frames(vcx: &mut gpui::VisualTestContext, n: usize) {
        for _ in 0..n {
            vcx.update(|window, _| window.refresh());
            vcx.run_until_parked();
        }
    }

    /// Drive a numeric multi-cell selection so the tab-bar stats readout is shown (mirrors the
    /// Phase-1 reply plumbing).
    fn show_stats(h: &Harness, cx: &mut TestAppContext) {
        upd(h, cx, |c, window, cx| {
            c.on_selection_changed(multi_a1_a3(), window, cx)
        });
        tick(cx, 150);
        upd(h, cx, |c, window, cx| {
            c.on_worker_event(
                WorkerEvent::SelectionStats {
                    req_id: 1,
                    stats: numeric_stats(),
                },
                window,
                cx,
            )
        });
    }

    #[gpui::test]
    fn action_row_fits_has_no_chevrons(cx: &mut TestAppContext) {
        // A wide window fits the whole action row → the scroller is invisible (no chevrons, no
        // behaviour change vs. today — `functional_spec.md §9B` "fits horizontally").
        let h = build_sized(
            cx,
            vec![SheetTab::new(SheetId(0), "Sheet1")],
            SheetId(0),
            1400.0,
            200.0,
        );
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        paint_twice(&mut vcx);
        assert!(
            vcx.debug_bounds("action-row-chevrons").is_none(),
            "a window wider than the action row shows no scroll chevrons"
        );
    }

    #[gpui::test]
    fn action_row_overflow_shows_chevrons(cx: &mut TestAppContext) {
        // A window narrower than the ~1152 px action row → the scroller shows its chevron section.
        let h = build_sized(
            cx,
            vec![SheetTab::new(SheetId(0), "Sheet1")],
            SheetId(0),
            480.0,
            200.0,
        );
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        paint_twice(&mut vcx);
        assert!(
            vcx.debug_bounds("action-row-chevrons").is_some(),
            "a narrow window overflows the action row → scroll chevrons appear"
        );
    }

    #[gpui::test]
    fn action_row_no_chevrons_when_the_controls_actually_fit(cx: &mut TestAppContext) {
        // Regression for `functional_spec.md §10.2`: the button group's natural width is well under
        // the old hand-estimated `ACTION_ROW_MIN_W = 1152`, so at a viewport that comfortably fits
        // the real controls (but is narrower than 1152) the scroller must report NO overflow — the
        // pre-fix `min_w(1152)` padded the scroll content with trailing empty space and tripped the
        // chevrons here.
        let h = build_sized(
            cx,
            vec![SheetTab::new(SheetId(0), "Sheet1")],
            SheetId(0),
            1400.0,
            200.0,
        );
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        paint_twice(&mut vcx);

        // The painted natural width of the button group (`flex_shrink_0`, so it never compresses).
        let natural = vcx
            .debug_bounds("action-row-groups")
            .expect("the action-row button group paints")
            .size
            .width;
        let natural = f32::from(natural);
        assert!(
            natural < 1152.0,
            "the old min_w=1152 over-estimated the controls; true natural width is {natural}"
        );
        // A window comfortably wider than the true controls but still below the old 1152 estimate —
        // the band the removed `min_w = 1152` over-padded (its viewport would fall between the true
        // width and 1152 → false chevrons). Derived as the middle of that band rather than a fixed
        // offset, so it stays valid as the action row grows: the frame's `px_2` (16px total) is the
        // gap between the window width and the scroller's viewport, so the band is
        // `(natural + FRAME_PAD, 1152)`.
        const FRAME_PAD: f32 = 16.0;
        let fits = (natural + FRAME_PAD + 1152.0) / 2.0;
        assert!(
            fits > natural + FRAME_PAD && fits < 1152.0,
            "the fit window {fits} sits in the (controls, 1152) no-overflow band (natural {natural})"
        );
        let h2 = build_sized(
            cx,
            vec![SheetTab::new(SheetId(0), "Sheet1")],
            SheetId(0),
            fits,
            200.0,
        );
        let mut vcx2 = gpui::VisualTestContext::from_window(h2.window.into(), cx);
        paint_twice(&mut vcx2);
        assert!(
            vcx2.debug_bounds("action-row-chevrons").is_none(),
            "a viewport ({fits}) wide enough for the real controls shows no chevrons (§10.2 fix)"
        );
    }

    #[gpui::test]
    fn tab_bar_overflow_shows_chevrons_and_keeps_stats_static(cx: &mut TestAppContext) {
        // Many tabs in a narrow window overflow the tab strip → chevrons appear, AND the stats
        // group stays pinned static to the RIGHT of the scroller (never pushed off — §9A.4).
        let h = build_sized(cx, many_sheets(40), SheetId(0), 560.0, 200.0);
        show_stats(&h, cx);
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        paint_twice(&mut vcx);
        let chevrons = vcx
            .debug_bounds("tab-bar-chevrons")
            .expect("a long tab strip overflows → tab-bar scroll chevrons appear");
        let stats = vcx
            .debug_bounds("selection-stats")
            .expect("the stats group is still painted, never scrolled away");
        assert!(
            f32::from(stats.origin.x) > f32::from(chevrons.origin.x),
            "the static stats group sits to the right of the (scrolling) tab chevrons"
        );
    }

    #[gpui::test]
    fn tab_bar_leading_divider_gated_on_stats(cx: &mut TestAppContext) {
        // The leading divider (§9A.3) renders only when the stats readout is shown, so it never
        // floats alone — the render gates it on the same `stats_readout_parts().is_some()` this test
        // exercises directly.
        let h = one_sheet(cx);
        assert!(
            upd(&h, cx, |c, _w, _cx| c.stats_readout_parts().is_none()),
            "a single-cell selection hides the readout → no leading divider"
        );
        show_stats(&h, cx);
        assert!(
            upd(&h, cx, |c, _w, _cx| c.stats_readout_parts().is_some()),
            "a numeric multi-cell selection shows the readout → leading divider renders"
        );
    }

    #[gpui::test]
    fn chevron_click_animates_to_target(cx: &mut TestAppContext) {
        // The tab strip starts scrolled to the left: the left chevron is a no-op there, and the
        // right chevron ANIMATES the content toward the end (offset slides negative over frames —
        // D10.2, replacing the D9.2 instant jump).
        let h = build_sized(cx, many_sheets(40), SheetId(0), 560.0, 200.0);
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        paint_twice(&mut vcx);

        let at_start = vcx.update(|_w, app| h.chrome.read(app).tab_scroller.offset_x());
        assert!(
            at_start.abs() < 1.0,
            "a fresh scroller starts at offset 0, got {at_start}"
        );

        // Left chevron at the start is disabled → clicking it arms no slide and does not scroll.
        let left = vcx
            .debug_bounds("tab-bar-chevron-left")
            .expect("left chevron painted");
        vcx.simulate_click(left.center(), Modifiers::default());
        vcx.run_until_parked();
        assert!(
            !vcx.update(|_w, app| h.chrome.read(app).tab_scroller.is_animating()),
            "the disabled left chevron at the start arms no animation"
        );
        let after_left = vcx.update(|_w, app| h.chrome.read(app).tab_scroller.offset_x());
        assert!(
            after_left.abs() < 1.0,
            "the left chevron at the start is a no-op, got {after_left}"
        );

        // Right chevron arms an animated slide (`target` set, `is_animating`) rather than an instant
        // jump: even after the first redraw it's still mid-flight (one 60%-step is not arrival), so
        // the reader sees a slide, not a teleport to the destination.
        let right = vcx
            .debug_bounds("tab-bar-chevron-right")
            .expect("right chevron painted");
        vcx.simulate_click(right.center(), Modifiers::default());
        vcx.run_until_parked();
        assert!(
            vcx.update(|_w, app| h.chrome.read(app).tab_scroller.is_animating()),
            "clicking the right chevron arms an in-flight slide"
        );

        // Each frame steps the offset monotonically toward the (negative) clamped target, then the
        // slide settles and clears `target`.
        let mut prev = vcx.update(|_w, app| h.chrome.read(app).tab_scroller.offset_x());
        let mut moved_negative = false;
        for _ in 0..20 {
            pump_frames(&mut vcx, 1);
            let now = vcx.update(|_w, app| h.chrome.read(app).tab_scroller.offset_x());
            assert!(
                now <= prev + 0.01,
                "the slide only moves toward the end (never backward): {now} > {prev}"
            );
            if now < -1.0 {
                moved_negative = true;
            }
            prev = now;
            if !vcx.update(|_w, app| h.chrome.read(app).tab_scroller.is_animating()) {
                break;
            }
        }
        assert!(
            moved_negative,
            "the animated slide moved the content toward the end, got final {prev}"
        );
        assert!(
            !vcx.update(|_w, app| h.chrome.read(app).tab_scroller.is_animating()),
            "the slide self-terminates once it reaches the target"
        );

        // It lands at the clamped `scroll_step` destination (0.8 × viewport from the start), within
        // the scroll range.
        let landed = vcx.update(|_w, app| h.chrome.read(app).tab_scroller.offset_x());
        assert!(landed < -1.0, "settled toward the end, got {landed}");
    }

    #[gpui::test]
    fn chevron_animation_clamps_at_end(cx: &mut TestAppContext) {
        // Repeated right-chevron clicks (each fully animated) drive the tab scroller to the end and
        // no further; the right chevron then disables (`at_end`) and arms no more slides.
        let h = build_sized(cx, many_sheets(40), SheetId(0), 560.0, 200.0);
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        paint_twice(&mut vcx);

        // Click + fully settle several times; each click resolves before the next.
        for _ in 0..12 {
            if let Some(right) = vcx.debug_bounds("tab-bar-chevron-right") {
                vcx.simulate_click(right.center(), Modifiers::default());
                vcx.run_until_parked();
                // Settle this slide before the next click.
                for _ in 0..20 {
                    pump_frames(&mut vcx, 1);
                    if !vcx.update(|_w, app| h.chrome.read(app).tab_scroller.is_animating()) {
                        break;
                    }
                }
            }
        }

        // At the end, the right chevron is disabled: clicking it arms no further slide and the
        // offset does not move past the limit.
        let at_limit = vcx.update(|_w, app| h.chrome.read(app).tab_scroller.offset_x());
        if let Some(right) = vcx.debug_bounds("tab-bar-chevron-right") {
            vcx.simulate_click(right.center(), Modifiers::default());
            vcx.run_until_parked();
            pump_frames(&mut vcx, 3);
        }
        let after = vcx.update(|_w, app| h.chrome.read(app).tab_scroller.offset_x());
        assert!(
            (after - at_limit).abs() < 1.0,
            "at the end the content stays pinned at the limit ({at_limit} → {after})"
        );
        assert!(
            at_limit < -1.0,
            "the strip did scroll to a non-trivial end, got {at_limit}"
        );
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
}
