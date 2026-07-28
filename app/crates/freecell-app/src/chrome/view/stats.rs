//! The selection-stats readout pinned to the right of the sheet-tab bar
//! (`functional_spec.md §1`): the debounced worker query behind it, the session-only Min/Max
//! expand toggle, and the readout's own rendering.
//!
//! The worker reply that feeds it (`WorkerEvent::SelectionStats`) is routed by
//! [`super::shell`]'s `on_worker_event`, and the readout is placed by [`super::tabs`]'s
//! `render_tab_bar` — this module owns the query, the state and the readout itself.
//!
//! Moved verbatim out of the single-file `chrome/view.rs`
//! (`specs/projects/chrome-view-split`).

use super::*;

use freecell_core::{format_stat_count, format_stat_value};

impl ChromeView {
    /// Re-request the selection-stats readout — the window calls this on `WorkerEvent::Published`
    /// so an edit that changes a value **inside** a still-active multi-cell selection re-aggregates
    /// (`functional_spec.md §1` live-update). Debounced + deduped like the selection-change path.
    pub fn refresh_selection_stats(&mut self, cx: &mut Context<Self>) {
        self.request_selection_stats(cx);
    }

    /// Issue the debounced `SelectionStats` query for the current selection (`functional_spec.md
    /// §1`). Bumps [`stats_seq`](Self::stats_seq) (which invalidates any in-flight reply); a
    /// single-cell / empty selection shows nothing, so it clears the readout and sends no query.
    /// A multi-cell selection arms a [`STATS_DEBOUNCE`] timer that fires the query only if no newer
    /// selection change has superseded it, tagging the request with `seq` so a stale reply is
    /// dropped on arrival.
    pub(super) fn request_selection_stats(&mut self, cx: &mut Context<Self>) {
        self.stats_seq = self.stats_seq.wrapping_add(1);
        let seq = self.stats_seq;
        if self.selection.is_single() {
            if self.selection_stats.take().is_some() {
                cx.notify();
            }
            return;
        }
        let sheet = self.active_sheet;
        let range = self.selection.range();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(STATS_DEBOUNCE).await;
            this.update(cx, |this, _cx| {
                // Only the most-recently armed timer sends — an intervening selection change bumped
                // `stats_seq`, superseding this one.
                if this.stats_seq == seq {
                    this.client.send(Command::SelectionStats {
                        sheet,
                        range,
                        req_id: seq,
                    });
                }
            })
            .ok();
        })
        .detach();
    }

    /// Flip the session-only Min / Max expansion of the stats readout (`functional_spec.md §1`).
    pub fn toggle_stats_minmax(&mut self, cx: &mut Context<Self>) {
        self.stats_show_minmax = !self.stats_show_minmax;
        cx.notify();
    }

    /// The labeled parts of the selection-stats readout, or `None` when nothing should show — a
    /// single-cell/empty selection (no stats), or a selection with no numeric cell. Default form is
    /// `Sum · Average · Count`; the session toggle appends `Min · Max` (`functional_spec.md §1`).
    /// Pure — the render + the tests read the same source.
    pub fn stats_readout_parts(&self) -> Option<Vec<String>> {
        let stats = self.selection_stats?;
        if !stats.has_numeric() {
            return None;
        }
        let mut parts = vec![
            format!("Sum: {}", format_stat_value(stats.sum)),
            format!(
                "Average: {}",
                format_stat_value(stats.average().unwrap_or_default())
            ),
            format!("Count: {}", format_stat_count(stats.count)),
        ];
        if self.stats_show_minmax {
            parts.push(format!(
                "Min: {}",
                format_stat_value(stats.min.unwrap_or_default())
            ));
            parts.push(format!(
                "Max: {}",
                format_stat_value(stats.max.unwrap_or_default())
            ));
        }
        Some(parts)
    }

    /// The full selection-stats readout as one string (`"Sum: … Average: … Count: …"`), or `None`
    /// when hidden — a test accessor mirroring what the tab bar renders.
    pub fn selection_stats_text(&self) -> Option<String> {
        self.stats_readout_parts().map(|parts| parts.join("   "))
    }

    /// The right-aligned selection-stats readout in the tab bar (`functional_spec.md §1`). Empty
    /// when hidden (single-cell / all-text / empty selection) so the row's height stays stable;
    /// when shown, the whole group is clickable to toggle the Min / Max expansion.
    pub(super) fn render_selection_stats(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut group = div()
            .id("selection-stats")
            .debug_selector(|| "selection-stats".into())
            .flex()
            .items_center()
            // Fill the bar height + track the line-height to it, so the readout sits vertically
            // centered in `TAB_BAR_H` rather than hugging the text box (`functional_spec.md §9A.2`).
            .h_full()
            .line_height(px(TAB_BAR_H))
            .gap_4()
            .pr_1()
            .text_size(px(12.0))
            .text_color(rgb(MUTED_TEXT));
        if let Some(parts) = self.stats_readout_parts() {
            group = group.cursor_pointer().on_click(cx.listener(
                |this, _: &ClickEvent, _window, cx| {
                    this.toggle_stats_minmax(cx);
                },
            ));
            for part in parts {
                group = group.child(div().child(part));
            }
        }
        group
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chrome::view::test_support::*;
    use gpui::TestAppContext;

    // ---- Selection stats (tab-bar status readout, `functional_spec.md §1`) -----------------

    #[gpui::test]
    fn multi_cell_selection_requests_debounced_stats(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(multi_a1_a3(), window, cx)
        });
        // Debounced: nothing is sent until the timer fires (a drag-select would otherwise spam).
        assert!(
            h.client.take_commands().is_empty(),
            "the stats query is debounced, not sent synchronously"
        );
        tick(cx, 150);
        let cmds = h.client.take_commands();
        assert!(
            matches!(
                cmds.as_slice(),
                [Command::SelectionStats { range, req_id: 1, .. }]
                    if *range == CellRange::new(cell(0, 0), cell(2, 0))
            ),
            "expected one debounced SelectionStats for A1:A3, got {cmds:?}"
        );
    }

    #[gpui::test]
    fn single_cell_selection_issues_no_stats(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(SelectionModel::single(cell(1, 1)), window, cx)
        });
        tick(cx, 150);
        let cmds = h.client.take_commands();
        assert!(
            cmds.iter()
                .all(|c| !matches!(c, Command::SelectionStats { .. })),
            "a single-cell selection issues no stats query, got {cmds:?}"
        );
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.selection_stats_text()), None);
    }

    #[gpui::test]
    fn stats_reply_renders_readout_with_minmax_toggle(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(multi_a1_a3(), window, cx)
        });
        tick(cx, 150);
        h.client.take_commands(); // drain the SelectionStats query (req_id 1)
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(
                WorkerEvent::SelectionStats {
                    req_id: 1,
                    stats: numeric_stats(),
                },
                window,
                cx,
            )
        });
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.selection_stats_text()),
            Some("Sum: 30   Average: 15   Count: 5".to_string())
        );
        // Clicking the readout expands it to also show Min / Max (session-only toggle).
        upd(&h, cx, |c, _w, cx| c.toggle_stats_minmax(cx));
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.selection_stats_text()),
            Some("Sum: 30   Average: 15   Count: 5   Min: 10   Max: 20".to_string())
        );
    }

    #[gpui::test]
    fn stale_stats_reply_dropped(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        // Two multi-cell selections back-to-back → the latest request is req_id 2.
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(multi_a1_a3(), window, cx);
            c.on_selection_changed(
                SelectionModel {
                    anchor: cell(0, 0),
                    active: cell(3, 0),
                },
                window,
                cx,
            );
        });
        tick(cx, 150);
        h.client.take_commands();
        // A superseded (req_id 1) reply is dropped.
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(
                WorkerEvent::SelectionStats {
                    req_id: 1,
                    stats: numeric_stats(),
                },
                window,
                cx,
            )
        });
        assert_eq!(
            upd(&h, cx, |c, _w, _cx| c.selection_stats_text()),
            None,
            "a stale reply for a superseded selection is ignored"
        );
        // The current (req_id 2) reply lands.
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(
                WorkerEvent::SelectionStats {
                    req_id: 2,
                    stats: numeric_stats(),
                },
                window,
                cx,
            )
        });
        assert!(upd(&h, cx, |c, _w, _cx| c.selection_stats_text()).is_some());
    }

    #[gpui::test]
    fn all_text_reply_hides_readout(cx: &mut TestAppContext) {
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(multi_a1_a3(), window, cx)
        });
        tick(cx, 150);
        h.client.take_commands();
        // A selection with content but no numeric cell shows no readout.
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(
                WorkerEvent::SelectionStats {
                    req_id: 1,
                    stats: SelectionStats {
                        count: 3,
                        numeric_count: 0,
                        sum: 0.0,
                        min: None,
                        max: None,
                    },
                },
                window,
                cx,
            )
        });
        assert_eq!(upd(&h, cx, |c, _w, _cx| c.selection_stats_text()), None);
    }

    #[gpui::test]
    fn tab_bar_paints_stats_readout_when_present(cx: &mut TestAppContext) {
        // Real render coverage for the tab-bar refactor: with a numeric multi-cell selection the
        // right-aligned readout element paints (its Sum/Average/Count text gives it real width).
        let h = one_sheet(cx);
        upd(&h, cx, |c, window, cx| {
            c.on_selection_changed(multi_a1_a3(), window, cx)
        });
        tick(cx, 150);
        upd(&h, cx, |c, window, cx| {
            c.on_worker_event(
                WorkerEvent::SelectionStats {
                    req_id: 1,
                    stats: numeric_stats(),
                },
                window,
                cx,
            )
        });
        let mut vcx = gpui::VisualTestContext::from_window(h.window.into(), cx);
        vcx.run_until_parked();
        let bounds = vcx
            .debug_bounds("selection-stats")
            .expect("the selection-stats readout paints in the tab bar");
        assert!(
            f32::from(bounds.size.width) > 20.0,
            "the readout should paint its Sum/Average/Count text, got width {}",
            f32::from(bounds.size.width)
        );
    }
}
