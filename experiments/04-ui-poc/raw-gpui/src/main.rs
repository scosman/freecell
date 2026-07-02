//! raw-gpui PoC — a custom virtualized spreadsheet grid on raw gpui primitives.
//!
//! One of the two Sub-project E variants (functional_spec §6.E, architecture §7). Serves
//! two purposes in one macOS/Metal app:
//! 1. **Interactive** — scroll/pan to judge feel.
//! 2. **"Run Test"** — a scripted scroll/jump sequence that measures per-frame render
//!    time + newly-visible-cell load latency, prints measured PASS/FAIL vs §5.4, and
//!    writes JSON to `../results/`. Triggered by the "Run Test" menu item or by the
//!    `--run-test` CLI flag (which also auto-quits on completion).
//!
//! macOS/Metal only. Do NOT build on Linux (see `Cargo.toml`). Run via `../scripts/`.

mod grid;

use gpui::{App, Menu, MenuItem, WindowOptions, actions, prelude::*};
use gpui_platform::application;

use grid::Grid;
use poc_core::PocConfig;

actions!(raw_gpui_poc, [RunTest, Quit]);

/// Resolves the `results/` directory next to this crate (../results relative to the
/// crate manifest), so a run from anywhere writes to the committed results folder.
fn results_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.join("results"))
        .unwrap_or_else(|| std::path::PathBuf::from("results"))
}

/// Best-effort short commit for stamping results (env override, else "unknown"). We do
/// not shell out to git from render code (determinism, architecture §3).
fn commit() -> String {
    std::env::var("POC_COMMIT").unwrap_or_else(|_| "unknown".to_string())
}

/// The report date, overridable for reproducible runs; defaults to a fixed placeholder
/// the human can replace when reporting (we never read a wall clock in recording code).
fn report_date() -> String {
    std::env::var("POC_DATE").unwrap_or_else(|_| "unknown".to_string())
}

fn main() {
    let auto_run = std::env::args().any(|a| a == "--run-test");

    application().run(move |cx: &mut App| {
        cx.activate(true);

        // Menu with the "Run Test" item + Quit. The action is dispatched to the focused
        // view (the grid), which flips into scripted mode.
        cx.set_menus([Menu::new("raw-gpui-poc").items([
            MenuItem::action("Run Test", RunTest),
            MenuItem::separator(),
            MenuItem::action("Quit", Quit),
        ])]);
        cx.on_action(|_: &Quit, cx: &mut App| cx.quit());

        let cfg = PocConfig::default();
        let out_dir = results_dir();
        let date = report_date();
        let commit = commit();

        // Create the grid entity up front so we can both mount it as the window root AND
        // keep a handle to drive the "Run Test" action / `--run-test` flag. `Entity` is
        // cheap to clone (a reference into the App's entity map).
        let grid = cx.new(|_| Grid::new(cfg, out_dir, date, commit));

        let root = grid.clone();
        cx.open_window(WindowOptions::default(), move |_window, _cx| root.clone())
            .expect("failed to open window");

        // Route the menu "Run Test" action into the grid entity.
        let action_grid = grid.clone();
        cx.on_action(move |_: &RunTest, cx: &mut App| {
            action_grid.update(cx, |grid, cx| grid.start_run_test(cx));
        });

        // CLI `--run-test`: kick off the scripted run immediately after launch.
        if auto_run {
            cx.defer(move |cx| {
                grid.update(cx, |grid, cx| grid.start_run_test(cx));
            });
        }
    });
}
