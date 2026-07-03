//! FreeCell application entry point.
//!
//! Phase 6 mounts the custom [`GridView`](freecell_app::grid::GridView) over hand-built
//! `freecell-core` fixtures (`grid::fixtures::demo_sources`) inside a gpui-component `Root`,
//! replacing the Phase-1 hello-world. This gives a real, capturable spreadsheet grid frame
//! (headers, gridlines, styled cells, selection) on both macOS (Metal) and Linux
//! (blade/Vulkan). The real app shell + chrome (`components/app_shell.md`) and the engine
//! wiring (`components/engine_worker.md`) land in later phases; this bin stays a thin
//! bootstrap that hosts the grid.
//!
//! The `--exit-after-ms` flag remains the Linux render-spike safety valve
//! (`architecture.md §9`, `DECISIONS_TO_REVIEW` Phase 1): open the window, let it paint,
//! and quit deterministically off an executor timer (not a paint-path deadline — headless
//! under Xvfb `render` is called only once).

use std::time::Duration;

use gpui::{prelude::*, App, AsyncApp, WindowOptions};
use gpui_component::Root;
use gpui_platform::application;

use freecell_app::grid::{fixtures, GridEventSink, GridView};

/// Parses an optional `--exit-after-ms <n>` argument (the render-spike safety valve).
fn exit_after_ms() -> Option<u64> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == "--exit-after-ms")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
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
            let grid = cx.new(|cx| {
                let mut view = GridView::new(fixtures::demo_sources(), GridEventSink::noop(), cx);
                view.set_selection(fixtures::demo_selection(), cx);
                view
            });
            // gpui-component requires the top-level window element to be a `Root`.
            cx.new(|cx| Root::new(grid, window, cx))
        })
        .expect("failed to open FreeCell window");

        // Render-spike safety valve: quit after a real executor timer, independent of
        // rendering (a render-loop deadline does NOT fire headless under Xvfb).
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
