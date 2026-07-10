//! In-grid chart presentation — the **fidelity dispatch** and the two non-plot affordances the
//! ChartLayer draws over a chart's anchor rect (charts/functional_spec §5, ui_design §2.2–2.3):
//! the Degraded corner **badge** and the Unsupported **placeholder**.
//!
//! The grid layer ([`crate::grid`]) resolves each chart's anchor to a pixel rect and hands the
//! chart + its derived [`Fidelity`] to [`in_grid_chart_element`], which decides — from the
//! fidelity alone — whether to paint the real plot ([`super::chart_element`]), the plot plus the
//! corner warning, or the placeholder box. Keeping that decision here (not in the grid) keeps the
//! grid layer geometry-only and lets the dispatch be unit-tested without a `Frame`.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, rgb, AnyElement, FontWeight, IntoElement, ParentElement, SharedString, Styled,
};

use freecell_chart_model::{Chart, Fidelity};

use super::chart_element;
use super::style::{BACKGROUND, MUTED_TEXT, TITLE_TEXT};

/// The Degraded compatibility badge text (charts/functional_spec §5, ui_design §2.2) — a small,
/// unobtrusive corner label, the whole signal (no detail list, no hover popover).
pub const COMPAT_WARNING_TEXT: &str = "⚠ May not display as intended";
/// The Unsupported placeholder's centered body line (ui_design §2.3).
pub const UNSUPPORTED_TEXT: &str = "Unsupported chart type";

/// Placeholder border colour — a quiet light grey (ui_design §2.3: "a quiet bordered rectangle").
const PLACEHOLDER_BORDER: u32 = 0xD1D5DB;

/// How the ChartLayer draws a chart at its anchor rect, derived from its [`Fidelity`]
/// (charts/functional_spec §5): the plot alone, the plot plus the corner compatibility badge, or
/// the placeholder box. A pure mapping so the dispatch is unit-tested without gpui.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderMode {
    /// Faithful → draw the plot as authored.
    Chart,
    /// Degraded → draw the plot **plus** the corner "⚠ May not display as intended" badge.
    ChartWithBadge,
    /// Unsupported → draw the bordered placeholder box instead of a chart.
    Placeholder,
}

/// The [`RenderMode`] for a [`Fidelity`] (charts/functional_spec §5). Mirrors
/// [`Fidelity::renders_as_chart`] / [`Fidelity::shows_compatibility_warning`] as a single enum the
/// element builder switches on.
pub fn render_mode(fidelity: Fidelity) -> RenderMode {
    match fidelity {
        Fidelity::Faithful => RenderMode::Chart,
        Fidelity::Degraded => RenderMode::ChartWithBadge,
        Fidelity::Unsupported => RenderMode::Placeholder,
    }
}

/// Build the element painted at a chart's anchor rect, dispatching on `fidelity`
/// (charts/architecture §4.2): the real plot for Faithful/Degraded (Degraded overlays the corner
/// badge), or the placeholder for Unsupported. If the (supposedly renderable) kind has no widget
/// yet, it falls back to the placeholder rather than a blank hole (functional_spec §1 — never a
/// crash, never a blank hole). The returned element fills its container (the layer sizes it to the
/// anchor rect).
pub fn in_grid_chart_element(chart: &Chart, fidelity: Fidelity) -> AnyElement {
    match render_mode(fidelity) {
        RenderMode::Placeholder => placeholder_element(chart.title.as_deref()),
        mode @ (RenderMode::Chart | RenderMode::ChartWithBadge) => match chart_element(chart) {
            Some(plot) => chart_with_optional_badge(plot, mode == RenderMode::ChartWithBadge),
            None => placeholder_element(chart.title.as_deref()),
        },
    }
}

/// Wrap a plot element to fill the anchor rect, overlaying the corner compatibility badge when the
/// chart is Degraded (ui_design §2.2: bottom-right, small, light grey).
fn chart_with_optional_badge(plot: AnyElement, badge: bool) -> AnyElement {
    div()
        .relative()
        .size_full()
        .overflow_hidden()
        .child(plot)
        .when(badge, |el| {
            el.child(
                div()
                    .absolute()
                    .right(px(6.0))
                    .bottom(px(4.0))
                    .text_size(px(10.0))
                    .text_color(rgb(MUTED_TEXT))
                    .child(SharedString::from(COMPAT_WARNING_TEXT)),
            )
        })
        .into_any_element()
}

/// The Unsupported placeholder (ui_design §2.3): a quiet bordered rectangle occupying the chart's
/// space, with the chart **title** (if any) at the top and a centered muted "Unsupported chart
/// type" — so the layout stays faithful and the workbook still opens.
fn placeholder_element(title: Option<&str>) -> AnyElement {
    let title = title.filter(|t| !t.is_empty()).map(str::to_string);
    div()
        .size_full()
        .flex()
        .flex_col()
        .bg(rgb(BACKGROUND))
        .border_1()
        .border_color(rgb(PLACEHOLDER_BORDER))
        .p_2()
        .when_some(title, |el, title| {
            el.child(
                div().w_full().flex().justify_center().child(
                    div()
                        .text_size(px(13.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(TITLE_TEXT))
                        .child(SharedString::from(title)),
                ),
            )
        })
        .child(
            div().flex_1().flex().items_center().justify_center().child(
                div()
                    .text_size(px(12.0))
                    .text_color(rgb(MUTED_TEXT))
                    .child(SharedString::from(UNSUPPORTED_TEXT)),
            ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_mode_maps_each_fidelity() {
        assert_eq!(render_mode(Fidelity::Faithful), RenderMode::Chart);
        assert_eq!(render_mode(Fidelity::Degraded), RenderMode::ChartWithBadge);
        assert_eq!(render_mode(Fidelity::Unsupported), RenderMode::Placeholder);
    }

    #[test]
    fn render_mode_agrees_with_fidelity_predicates() {
        for f in [
            Fidelity::Faithful,
            Fidelity::Degraded,
            Fidelity::Unsupported,
        ] {
            let mode = render_mode(f);
            // Renders as a chart iff not the placeholder.
            assert_eq!(mode != RenderMode::Placeholder, f.renders_as_chart());
            // Shows the badge iff the ChartWithBadge mode.
            assert_eq!(
                mode == RenderMode::ChartWithBadge,
                f.shows_compatibility_warning()
            );
        }
    }

    #[test]
    fn warning_and_placeholder_text_match_ui_design() {
        assert_eq!(COMPAT_WARNING_TEXT, "⚠ May not display as intended");
        assert_eq!(UNSUPPORTED_TEXT, "Unsupported chart type");
    }
}
