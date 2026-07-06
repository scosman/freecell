//! The gpui side of the harness: open ONE window sized to a scene's viewport, host the chart
//! widget in a gpui-component `Root`, and self-quit after `exit_after_ms`. The capture harness
//! ([`crate::capture`]) forces presentation (`xrefresh`) and grabs the window — the proven
//! `app/render-tests` Linux path, copied here because experiments stay independent of `/app`.

use std::time::Duration;

use anyhow::{anyhow, Result};
use gpui::{
    div, px, rgb, size, App, AppContext as _, AsyncApp, Bounds, Context, IntoElement, Point,
    Render, Styled, Window, WindowBounds, WindowOptions,
};
use gpui_component::Root;
use gpui_platform::application;

use chart_model::Chart;

use crate::scenes;

/// A gpui view that renders one chart full-window. gpui-component requires the top-level
/// window element to be a `Root`, which wraps this view.
struct ChartView {
    chart: Chart,
}

impl Render for ChartView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        crate::chart_element(&self.chart)
            .unwrap_or_else(|| div().size_full().bg(rgb(0xFFFFFF)).into_any_element())
    }
}

/// Runs the gpui app for a single named scene: opens a viewport-sized window with the chart
/// widget and quits after `exit_after_ms`.
pub fn run_render_scene(scene_name: &str, exit_after_ms: u64) -> Result<()> {
    let scene = scenes::get(scene_name).ok_or_else(|| anyhow!("unknown scene: {scene_name}"))?;
    let (w, h) = scene.viewport;
    let chart = scene.chart;

    let app = application().with_assets(gpui_component_assets::Assets);
    app.run(move |cx: &mut App| {
        gpui_component::init(cx);
        cx.activate(true);

        // A window at the screen origin sized exactly to the scene viewport, so the capture
        // (which finds the window by its size, no window manager under Xvfb) grabs exactly the
        // chart with no decorations.
        let bounds = Bounds {
            origin: Point::default(),
            size: size(px(w as f32), px(h as f32)),
        };
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |window, cx| {
                let view = cx.new(|_| ChartView { chart });
                cx.new(|cx| Root::new(view, window, cx))
            },
        )
        .expect("failed to open render window");

        // Self-quit off a real executor timer (independent of rendering).
        cx.spawn(async move |cx: &mut AsyncApp| {
            cx.background_executor()
                .timer(Duration::from_millis(exit_after_ms))
                .await;
            cx.update(|cx| cx.quit());
        })
        .detach();
    });

    Ok(())
}
