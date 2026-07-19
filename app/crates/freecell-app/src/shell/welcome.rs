//! [`WelcomeView`] — the fixed launch window (`functional_spec.md §2`, `ui_design.md §1–3`).
//!
//! A two-pane layout: a LEFT pane with the FreeCell wordmark, tagline, and the **New
//! Spreadsheet** / **Open…** buttons; a RIGHT pane listing up to [`WELCOME_LIMIT`] recent files
//! (or a text-only empty state). It registers **no document actions**, so Save / Undo / etc. are
//! disabled while it is frontmost (`components/app_shell.md §Menus & actions`). It can also host
//! an app-level error dialog when no document window is around to own it (`render_modal`); the
//! About screen is its own window now (`shell::about`), not a modal.
//!
//! The recent rows are driven entirely by [`set_recents`](WelcomeView::set_recents): the app hands
//! this view ready-to-render [`DisplayEntry`]s (built by the pure `freecell_core::recent`), so the
//! view itself does no disk access or formatting.

use gpui::{div, prelude::*, px, rgb, App, ClickEvent, Context, FocusHandle, Focusable, Window};
use gpui_component::button::{Button, ButtonVariants as _};

use freecell_core::recent::DisplayEntry;

use super::fonts::WORDMARK_FAMILY;
use super::{titlebar, CloseWindow, FreeCellApp};

// Shared chrome/titlebar palette tokens (`ui_design.md §0`) — mirrored here as the established
// pattern (titlebar.rs / chrome/view.rs carry the same values); no new welcome-only hexes.
const CHROME_BG: u32 = 0xF3F3F3; // left pane bg + row hover
const ACTIVE_TAB_BG: u32 = 0xFFFFFF; // right pane / card surface + modal card
const HAIRLINE: u32 = 0xD9D9D9;
const TEXT: u32 = 0x1F1F1F;
const MUTED_TEXT: u32 = 0x555555;

/// The left pane's fixed width (`ui_design.md §1`).
const LEFT_PANE_WIDTH: f32 = 264.0;

/// A dialog the welcome window can host when there's no document window to own it. Only the
/// app-level error dialog remains — the About screen is a standalone window now (`shell::about`).
#[derive(Debug, Clone)]
enum WelcomeModal {
    Error { title: String, detail: String },
}

/// The launch window.
pub struct WelcomeView {
    focus_handle: FocusHandle,
    modal: Option<WelcomeModal>,
    /// The recent-file rows the right pane renders, most-recent-first, already capped +
    /// formatted by the app (`set_recents`). Empty ⇒ the right pane shows the empty state.
    recents: Vec<DisplayEntry>,
}

impl WelcomeView {
    /// Builds the welcome view, focused so its global-action key bindings resolve. Recent rows
    /// start empty; the app seeds + refreshes them via [`set_recents`](Self::set_recents).
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        window.set_window_title("FreeCell");
        Self {
            focus_handle,
            modal: None,
            recents: Vec::new(),
        }
    }

    /// Replaces the recent-file rows the right pane shows and repaints. The app pushes fresh rows
    /// here when it seeds a newly-opened welcome and whenever the store changes while the welcome
    /// is open (`functional_spec.md §2.4`, `architecture.md §4`).
    pub fn set_recents(&mut self, recents: Vec<DisplayEntry>, cx: &mut Context<Self>) {
        self.recents = recents;
        cx.notify();
    }

    /// Shows an app-level error dialog on the welcome window.
    pub fn show_error(
        &mut self,
        title: impl Into<String>,
        detail: impl Into<String>,
        cx: &mut Context<Self>,
    ) {
        self.modal = Some(WelcomeModal::Error {
            title: title.into(),
            detail: detail.into(),
        });
        cx.notify();
    }

    fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.modal = None;
        cx.notify();
    }

    /// Whether a dialog is currently showing (tests).
    pub fn has_modal(&self) -> bool {
        self.modal.is_some()
    }

    /// Opens the recent file at `index` — the whole-row click target (`functional_spec.md §2.2`).
    ///
    /// Deferred: [`FreeCellApp::open_path`] re-enters the app global and can push fresh rows back
    /// into *this* view (`refresh_recents_ui` → `set_recents`), which — run inline from the row's
    /// click listener while this entity is still leased — would be a re-entrant self-update.
    /// Deferring runs the open after this update returns (the same reason `app.rs on_window_closed`
    /// defers). A missing file is a no-op here; a vanished-between-render-and-click file surfaces
    /// `open_path`'s standard "Couldn't open the file" dialog (`functional_spec.md §2.2`).
    pub(crate) fn open_recent(&self, index: usize, cx: &mut App) {
        if let Some(row) = self.recents.get(index) {
            let path = row.path.clone();
            cx.defer(move |cx| FreeCellApp::open_path(&path, cx));
        }
    }

    /// The number of recent rows currently shown (tests).
    #[cfg(test)]
    pub(crate) fn recent_row_count(&self) -> usize {
        self.recents.len()
    }

    /// Whether the right pane is showing the empty state (no recent rows) (tests).
    #[cfg(test)]
    pub(crate) fn is_empty_state(&self) -> bool {
        self.recents.is_empty()
    }
}

impl Focusable for WelcomeView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for WelcomeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("welcome")
            .track_focus(&self.focus_handle)
            .key_context("Welcome")
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(CHROME_BG))
            // Cmd/Ctrl+W closes the welcome window (matches the platform convention). Closing
            // the last window quits the app via the registry (`app.rs on_window_closed`).
            .on_action(cx.listener(|_this, _: &CloseWindow, window, _cx| window.remove_window()))
            // macOS custom titlebar (§7.1): "FreeCell", drawn only when the master switch is on.
            // On Linux it is omitted and the two panes below fill the window.
            .children(titlebar::MACOS_TITLEBAR.then(|| titlebar::titlebar_row("")))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_row()
                    .min_h_0()
                    .child(self.render_left_pane(cx))
                    .child(self.render_right_pane(cx)),
            )
            .children(self.render_modal(cx))
    }
}

impl WelcomeView {
    /// The LEFT pane: wordmark, tagline, and the two stacked full-width actions at the top
    /// (`ui_design.md §2`), plus the "Open Demo Spreadsheet" link anchored at the pane bottom.
    fn render_left_pane(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(LEFT_PANE_WIDTH))
            .h_full()
            .flex()
            .flex_col()
            .gap(px(24.0))
            .p(px(32.0))
            .bg(rgb(CHROME_BG))
            .border_r_1()
            .border_color(rgb(HAIRLINE))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(0.0))
                    .child(
                        // Same Inter Display ExtraBold single-face family as the About wordmark —
                        // one family name resolves it on every platform (brand consistency).
                        div()
                            .font_family(WORDMARK_FAMILY)
                            .text_size(px(28.0))
                            .text_color(rgb(TEXT))
                            .child("FreeCell"),
                    )
                    .child(
                        div()
                            .text_size(px(16.0))
                            .text_color(rgb(MUTED_TEXT))
                            .child("The open spreadsheet"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.0))
                    .child(
                        Button::new("new-spreadsheet")
                            .label("New Spreadsheet")
                            .primary()
                            .w_full()
                            .on_click(cx.listener(|_this, _: &ClickEvent, _window, cx| {
                                FreeCellApp::new_workbook(cx);
                            })),
                    )
                    .child(
                        Button::new("open-file")
                            .label("Open…")
                            .w_full()
                            .on_click(cx.listener(|_this, _: &ClickEvent, _window, cx| {
                                FreeCellApp::open_via_panel(cx);
                            })),
                    ),
            )
            // A flex grower pushes the demo link down so it anchors at the BOTTOM of the pane
            // while the wordmark + buttons stay at the top. The pane's `p(32)` keeps the link off
            // the bottom/left edges (not flush against the border).
            .child(div().flex_1())
            // A subtle tertiary text link (not a third button): opens the bundled demo workbook as
            // a fresh untitled sheet (`FreeCellApp::open_demo`). Styled from the pane's muted-text
            // token, darkening + underlining on hover for a link affordance.
            .child(
                div()
                    .id("welcome-demo-link")
                    // Registers the painted bounds under this name for the render test's
                    // `debug_bounds("welcome-demo-link")` lookup.
                    .debug_selector(|| "welcome-demo-link".to_string())
                    .text_center()
                    .cursor_pointer()
                    .text_size(px(13.0))
                    .text_color(rgb(MUTED_TEXT))
                    .hover(|s| s.text_color(rgb(TEXT)).underline())
                    .on_click(cx.listener(|_this, _: &ClickEvent, _window, cx| {
                        FreeCellApp::open_demo(cx);
                    }))
                    .child("Open Demo Spreadsheet"),
            )
    }

    /// The RIGHT pane: a `RECENT` header over the recent list or the empty state (`ui_design.md §3`).
    fn render_right_pane(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex_1()
            .h_full()
            .min_w_0()
            .flex()
            .flex_col()
            .p(px(26.0))
            .bg(rgb(ACTIVE_TAB_BG))
            .child(
                div()
                    .pb(px(12.0))
                    .text_size(px(11.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(MUTED_TEXT))
                    .child("RECENT"),
            )
            .child(if self.recents.is_empty() {
                self.render_empty_state().into_any_element()
            } else {
                self.render_recent_list(cx).into_any_element()
            })
    }

    /// The recent list: a bordered card of up to [`WELCOME_LIMIT`] pure-text rows (`ui_design.md
    /// §3.1`). No icons/glyphs.
    fn render_recent_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let last = self.recents.len().saturating_sub(1);
        div()
            .flex()
            .flex_col()
            .border_1()
            .border_color(rgb(HAIRLINE))
            .rounded(px(8.0))
            .overflow_hidden()
            .children(
                self.recents
                    .iter()
                    .enumerate()
                    .map(|(index, row)| self.render_recent_row(index, row, index != last, cx)),
            )
    }

    /// One recent-file row: name + `"{size} · {folder}"` subtitle + right-aligned relative time,
    /// whole-row clickable with a subtle hover (`ui_design.md §3.1`).
    fn render_recent_row(
        &self,
        index: usize,
        row: &DisplayEntry,
        separator: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(("welcome-recent-row", index))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(12.0))
            .h(px(56.0))
            .px(px(14.0))
            .when(separator, |d| d.border_b_1().border_color(rgb(HAIRLINE)))
            .cursor_pointer()
            .hover(|s| s.bg(rgb(CHROME_BG)))
            .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.open_recent(index, cx);
            }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .text_size(px(14.0))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(TEXT))
                            .truncate()
                            .child(row.name.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(MUTED_TEXT))
                            .truncate()
                            .child(row.subtitle.clone()),
                    ),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(12.0))
                    .text_color(rgb(MUTED_TEXT))
                    .child(row.relative_time.clone()),
            )
    }

    /// The text-only empty state, centered under the `RECENT` header (`ui_design.md §3.2`). No glyph.
    fn render_empty_state(&self) -> impl IntoElement {
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(6.0))
            .child(
                div()
                    .text_size(px(15.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(TEXT))
                    .child("No recent spreadsheets"),
            )
            .child(
                div()
                    .max_w(px(240.0))
                    .text_center()
                    .text_size(px(13.0))
                    .text_color(rgb(MUTED_TEXT))
                    .child("Create a new spreadsheet or open a file to get started."),
            )
    }

    fn render_modal(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let WelcomeModal::Error { title, detail } = self.modal.as_ref()?;
        let (title, body) = (title.clone(), detail.clone());
        Some(
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
                                .font_weight(gpui::FontWeight::BOLD)
                                .text_color(rgb(TEXT))
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(px(13.0))
                                .text_color(rgb(MUTED_TEXT))
                                .child(body),
                        )
                        .child(div().flex().justify_end().child(
                            Button::new("welcome-ok").label("OK").primary().on_click(
                                cx.listener(|this, _: &ClickEvent, _window, cx| this.dismiss(cx)),
                            ),
                        )),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{size, TestAppContext};
    use gpui_component::Root;
    use std::path::PathBuf;

    /// A ready-to-render row for a fictitious file (the view does no disk access itself).
    fn row(name: &str) -> DisplayEntry {
        DisplayEntry {
            path: PathBuf::from(format!("/tmp/{name}")),
            name: name.to_string(),
            subtitle: "12 KB · Downloads".to_string(),
            relative_time: "2h ago".to_string(),
        }
    }

    /// Builds a `WelcomeView` in a test window (no worker, no `FreeCellApp` needed for the
    /// state-only assertions).
    fn welcome(cx: &mut TestAppContext) -> gpui::Entity<WelcomeView> {
        cx.update(gpui_component::init);
        let mut out = None;
        let slot = &mut out;
        cx.open_window(size(px(720.0), px(480.0)), |window, cx| {
            let view = cx.new(|cx| WelcomeView::new(window, cx));
            *slot = Some(view.clone());
            Root::new(view, window, cx)
        });
        out.expect("welcome view built")
    }

    #[gpui::test]
    fn set_recents_reports_the_row_count(cx: &mut TestAppContext) {
        let view = welcome(cx);
        cx.update(|cx| {
            view.update(cx, |w, cx| {
                w.set_recents(vec![row("A.xlsx"), row("B.xlsx")], cx)
            })
        });
        assert_eq!(cx.update(|cx| view.read(cx).recent_row_count()), 2);
        assert!(
            !cx.update(|cx| view.read(cx).is_empty_state()),
            "a populated list is not the empty state"
        );
    }

    #[gpui::test]
    fn render_paints_the_demo_link(cx: &mut TestAppContext) {
        // The left pane paints the "Open Demo Spreadsheet" text link, anchored at the bottom of
        // the pane — the link renders as part of the tree, not just as a constant string.
        // `debug_bounds` resolves the element's `.debug_selector()` (the `.id()` is there for
        // `.on_click` interactivity, not this lookup — see `grid/view.rs`). Its click routing to
        // `FreeCellApp::open_demo` is exercised worker-free by the app-level
        // `demo_opens_as_an_untitled_window` test.
        cx.update(gpui_component::init);
        let handle = cx.open_window(size(px(720.0), px(480.0)), |window, cx| {
            let view = cx.new(|cx| WelcomeView::new(window, cx));
            Root::new(view, window, cx)
        });
        let mut vcx = gpui::VisualTestContext::from_window(handle.into(), cx);
        vcx.run_until_parked();
        assert!(
            vcx.debug_bounds("welcome-demo-link").is_some(),
            "the welcome left pane paints the Open Demo Spreadsheet link"
        );
    }

    #[gpui::test]
    fn no_recents_is_the_empty_state(cx: &mut TestAppContext) {
        let view = welcome(cx);
        // A fresh view starts empty.
        assert!(cx.update(|cx| view.read(cx).is_empty_state()));
        assert_eq!(cx.update(|cx| view.read(cx).recent_row_count()), 0);
        // Clearing back to empty stays the empty state.
        cx.update(|cx| view.update(cx, |w, cx| w.set_recents(vec![], cx)));
        assert!(cx.update(|cx| view.read(cx).is_empty_state()));
    }
}
