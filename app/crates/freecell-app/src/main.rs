//! FreeCell application entry point.
//!
//! Phase 1 (scaffolding) ships a **hello-world GPUI + gpui-component window** that must
//! build and run on both macOS (Metal) and Linux (blade/Vulkan). Its second job is the
//! **load-bearing Linux render spike** (`architecture.md §9`,
//! `components/render_test_harness.md`): run under Xvfb + Mesa lavapipe and prove pixels
//! can be captured off the GPUI window. The `--exit-after-ms` flag lets the spike script
//! open the window, let it paint, and quit deterministically.
//!
//! The real app shell, chrome, and grid (`components/app_shell.md`, `components/grid.md`)
//! land in later phases; this file is deliberately a thin bootstrap.
//!
//! Bootstrap shape (gpui-component `Root` + bundled assets) mirrors the validated POC
//! (`experiments/04-ui-poc/gpui-component/src/main.rs`) at the pinned gpui rev.

use std::time::Duration;

// `prelude::*` brings the chainable UI traits (Styled, ParentElement, InteractiveElement,
// IntoElement, Render); concrete types are named explicitly.
use gpui::{
    prelude::*, px, rgb, App, AsyncApp, Context, FontWeight, Pixels, Window, WindowOptions,
};
use gpui_component::Root;
use gpui_platform::application;

/// Parses an optional `--exit-after-ms <n>` argument (the render-spike safety valve).
fn exit_after_ms() -> Option<u64> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == "--exit-after-ms")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
}

/// The Phase-1 hello-world root view: a titled card over a tiny spreadsheet-like grid,
/// so a captured frame exercises fills, borders, and text — the three things the render
/// suite cares about. Replaced by the real `WorkbookWindow` in later phases.
struct HelloView;

const HEADER_W: f32 = 36.0;
const HEADER_H: f32 = 22.0;
const CELL_W: f32 = 72.0;
const CELL_H: f32 = 26.0;
const GRID_ROWS: usize = 3;
const GRID_COLS: usize = 3;

impl Render for HelloView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        gpui::div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_4()
            .bg(rgb(0xF5F5F5))
            .child(
                gpui::div()
                    .text_2xl()
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(0x1A1A1A))
                    .child("FreeCell"),
            )
            .child(
                gpui::div()
                    .text_sm()
                    .text_color(rgb(0x666666))
                    .child("GPUI + gpui-component hello world"),
            )
            .child(mini_grid())
    }
}

/// A small absolute-positioned grid (headers + cells, one filled), mirroring the
/// raw-gpui POC render style — a non-blank, deterministic scene for the capture spike.
fn mini_grid() -> impl IntoElement {
    let mut children: Vec<gpui::AnyElement> = Vec::new();

    for c in 0..GRID_COLS {
        let left = HEADER_W + c as f32 * CELL_W;
        let label = ((b'A' + c as u8) as char).to_string();
        children.push(
            cell(
                label,
                px(left),
                px(0.0),
                px(CELL_W),
                px(HEADER_H),
                0xF2F2F2,
                false,
            )
            .into_any_element(),
        );
    }
    for r in 0..GRID_ROWS {
        let top = HEADER_H + r as f32 * CELL_H;
        children.push(
            cell(
                (r + 1).to_string(),
                px(0.0),
                px(top),
                px(HEADER_W),
                px(CELL_H),
                0xF2F2F2,
                false,
            )
            .into_any_element(),
        );
    }
    for r in 0..GRID_ROWS {
        for c in 0..GRID_COLS {
            let left = HEADER_W + c as f32 * CELL_W;
            let top = HEADER_H + r as f32 * CELL_H;
            let filled = r == 1 && c == 1;
            let fill = if filled { 0xFFF9C4 } else { 0xFFFFFF };
            let text = format!("{}{}", (b'A' + c as u8) as char, r + 1);
            children.push(
                cell(
                    text,
                    px(left),
                    px(top),
                    px(CELL_W),
                    px(CELL_H),
                    fill,
                    filled,
                )
                .into_any_element(),
            );
        }
    }

    gpui::div()
        .relative()
        .w(px(HEADER_W + GRID_COLS as f32 * CELL_W))
        .h(px(HEADER_H + GRID_ROWS as f32 * CELL_H))
        .children(children)
}

#[allow(clippy::too_many_arguments)]
fn cell(
    text: String,
    left: Pixels,
    top: Pixels,
    w: Pixels,
    h: Pixels,
    fill: u32,
    bold: bool,
) -> impl IntoElement {
    let mut el = gpui::div()
        .absolute()
        .left(left)
        .top(top)
        .w(w)
        .h(h)
        .flex()
        .items_center()
        .justify_center()
        .bg(rgb(fill))
        .border_1()
        .border_color(rgb(0xD0D0D0))
        .text_color(rgb(0x1A1A1A))
        .text_xs()
        .child(text);
    if bold {
        el = el.font_weight(FontWeight::BOLD);
    }
    el
}

fn main() {
    // Logging setup per architecture.md §8 (tracing + env-filter). Best-effort: a second
    // init in tests/embedded use would fail, which we ignore.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();

    let exit_after = exit_after_ms();

    let app = application().with_assets(gpui_component_assets::Assets);
    app.run(move |cx: &mut App| {
        gpui_component::init(cx);
        cx.activate(true);

        cx.open_window(WindowOptions::default(), |window, cx| {
            let view = cx.new(|_| HelloView);
            // gpui-component requires the top-level window element to be a `Root`.
            cx.new(|cx| Root::new(view, window, cx))
        })
        .expect("failed to open FreeCell window");

        // Render-spike safety valve: quit after a real timer, independent of rendering.
        // (A render-loop deadline does NOT work headless under Xvfb — with no compositor
        // driving frame callbacks, `render` is called only once, so the timer must live on
        // the executor, not in the paint path.)
        if let Some(ms) = exit_after {
            cx.spawn(async move |cx: &mut AsyncApp| {
                cx.background_executor()
                    .timer(Duration::from_millis(ms))
                    .await;
                cx.update(|cx| cx.quit());
            })
            .detach();
        }
    });
}
