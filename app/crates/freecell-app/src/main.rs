//! FreeCell application entry point (`components/app_shell.md §Structure`,
//! `functional_spec.md §2`).
//!
//! Phase 10 replaces the Phase-6 demo grid window with the real app shell: initialize
//! gpui-component + fonts, install the [`FreeCellApp`] global (window registry, menus, key
//! bindings, action handlers), then show the welcome window — or open a `.xlsx` passed on the
//! command line. The document window's grid + chrome composition is wired in Phase 11; this
//! bin stays a thin bootstrap.
//!
//! The `--exit-after-ms` flag remains the Linux render-spike safety valve
//! (`architecture.md §9`, `DECISIONS_TO_REVIEW` Phase 1): quit deterministically off an
//! executor timer (not a paint-path deadline — headless under Xvfb `render` runs once).

use std::path::PathBuf;
use std::time::Duration;

use gpui::{App, AsyncApp};
use gpui_platform::application;

use freecell_app::shell::{register_fonts, FreeCellApp};

/// Parses an optional `--exit-after-ms <n>` argument (the render-spike safety valve).
fn exit_after_ms() -> Option<u64> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == "--exit-after-ms")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
}

/// The first non-flag `.xlsx` path argument, if any (best-effort CLI open — Finder open-file
/// events are a separate, deferred path; see DECISIONS_TO_REVIEW Phase 10).
fn xlsx_arg() -> Option<PathBuf> {
    std::env::args()
        .skip(1)
        .find(|a| !a.starts_with('-') && a.to_ascii_lowercase().ends_with(".xlsx"))
        .map(PathBuf::from)
}

fn main() {
    // Logging setup per architecture.md §8 (tracing + env-filter). Best-effort init.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();

    let exit_after = exit_after_ms();
    let open_path = xlsx_arg();

    let app = application().with_assets(gpui_component_assets::Assets);
    app.run(move |cx: &mut App| {
        gpui_component::init(cx);
        register_fonts(cx); // font-registration seam, called before any window opens; a no-op in
                            // MVP — ships on the default font (bundled Inter deferred, see fonts.rs)
        cx.activate(true);

        FreeCellApp::init(cx);
        match open_path {
            Some(path) => FreeCellApp::open_path(&path, cx),
            None => FreeCellApp::show_welcome(cx),
        }

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
