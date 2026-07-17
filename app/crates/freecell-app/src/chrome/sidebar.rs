//! The reusable right-docked **sidebar container** (`ui_design.md §0`, `components/cf_sidebar.md
//! §1`), extracted from the chart edit panel's hand-rolled card so both the chart panel and the
//! conditional-formatting sidebar share one docked shell + section helpers.
//!
//! The card is an absolutely-positioned surface between the data row and the tab bar on the right
//! edge of the sheet, with a pinned header (title + close `×`) above a scrolling body. `docked_sidebar`
//! renders it; the caller supplies the scrollable `body`. The `section` / `section_label` helpers are
//! the chart panel's former local closures, promoted here so the CF sidebar reuses them.

use gpui::{div, prelude::*, px, rgb, App, ClickEvent, ElementId, SharedString, Window};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::{IconName, Sizable as _};

use super::view::{ACTION_ROW_H, ACTIVE_TAB_BG, DATA_ROW_H, HAIRLINE, MUTED_TEXT, TAB_BAR_H, TEXT};

/// The shared width of a right-docked sidebar (the old chart-panel `CHART_PANEL_W`). The chart panel
/// and the CF sidebar dock at the same width for a consistent dock (`ui_design.md §0`).
pub(crate) const SIDEBAR_W: f32 = 268.0;

/// The shared dismiss `×` button used by a docked sidebar's header (and the find bar): a ghost, small
/// Lucide-"x" icon button (gpui-component's `IconName::Close`).
pub(crate) fn close_button(
    id: impl Into<ElementId>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    Button::new(id)
        .icon(IconName::Close)
        .ghost()
        .small()
        .on_click(on_click)
}

/// A muted, semibold mini section label (`ui_design.md §0`), shared by the chart panel + CF sidebar.
pub(crate) fn section_label(text: impl Into<SharedString>) -> impl IntoElement {
    div()
        .text_size(px(10.5))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(rgb(MUTED_TEXT))
        .child(text.into())
}

/// A labeled section: a [`section_label`] above `body` (`ui_design.md §0`).
pub(crate) fn section(label: impl Into<SharedString>, body: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(section_label(label))
        .child(body)
}

/// The right-docked card shell shared by the chart edit panel + the CF sidebar (`ui_design.md §0`,
/// `components/cf_sidebar.md §1`): an absolutely-positioned card on the right edge between the data
/// row and the tab bar, with a **pinned header** (title + close `×`, always reachable) above a
/// **scrolling body** that clips to the card's own bounds. The caller supplies `body` (its own inner
/// layout — e.g. a `flex_col gap_3` of sections).
///
/// `id` scopes the body's element id and the `{id}-card` / `{id}-body` / `{id}-close` debug
/// selectors, so `"chart-panel"` reproduces the chart panel's existing selectors exactly.
pub(crate) fn docked_sidebar(
    id: &'static str,
    title: impl Into<SharedString>,
    on_close: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    body: impl IntoElement,
) -> impl IntoElement {
    let header = div()
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .text_size(px(12.0))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgb(TEXT))
                .child(title.into()),
        )
        .child(
            close_button(SharedString::from(format!("{id}-close")), on_close)
                .debug_selector(move || format!("{id}-close")),
        );

    div()
        .absolute()
        .top(px(ACTION_ROW_H + DATA_ROW_H))
        .right_0()
        .bottom(px(TAB_BAR_H))
        .w(px(SIDEBAR_W))
        // Occlude the card so clicks on its controls don't trip a backdrop dismiss / move the grid
        // selection behind it.
        .occlude()
        .debug_selector(move || format!("{id}-card"))
        .flex()
        .flex_col()
        // Clip to the card's own bounds so overflowing controls never paint over the tab bar / grid
        // on a short window.
        .overflow_hidden()
        .bg(rgb(ACTIVE_TAB_BG))
        .border_l_1()
        .border_color(rgb(HAIRLINE))
        .shadow_md()
        // Pinned header (never scrolls, so the close × is always reachable).
        .child(div().flex_shrink_0().px_3().pt_3().pb_2().child(header))
        // Scrollable body: fills the remaining height and scrolls when the content overflows.
        // `min_h_0` lets the flex child shrink below its content so `overflow_y_scroll` engages; the
        // `id` gives it a tracked scroll offset.
        .child(
            div()
                .id(SharedString::from(format!("{id}-body")))
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px_3()
                .pb_3()
                .child(body),
        )
}
