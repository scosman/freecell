// Windows: a *release* build must open only the app window — not a console/terminal window
// alongside it. Linking the GUI ("windows") subsystem instead of the default console subsystem
// stops the OS from allocating a console for the process (the reported Windows launch issue).
// Debug builds keep the console subsystem so `cargo run` still shows the tracing/log output on a
// terminal. The attribute is Windows-only and a no-op on macOS/Linux, so it is safe to leave in.
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

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

use freecell_app::shell::{register_fonts, AppAssets, FreeCellApp};

/// Parses an optional `--exit-after-ms <n>` argument (the render-spike safety valve).
fn exit_after_ms() -> Option<u64> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == "--exit-after-ms")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
}

/// The first non-flag `.xlsx` / `.csv` path argument, if any (best-effort CLI open — Finder
/// open-file events are a separate, deferred path; see DECISIONS_TO_REVIEW Phase 10). A `.csv`
/// routes to CSV import (`open_path` branches on the extension, `functional_spec.md §2`).
fn open_arg() -> Option<PathBuf> {
    std::env::args().skip(1).find_map(|a| {
        if a.starts_with('-') {
            return None;
        }
        let lower = a.to_ascii_lowercase();
        (lower.ends_with(".xlsx") || lower.ends_with(".csv")).then(|| PathBuf::from(a))
    })
}

/// Default `tracing` filter, used only when `RUST_LOG` is unset.
///
/// `info` for the app, but the `gpui::svg_renderer` target is raised to `error` to silence two
/// benign startup `WARN`s: gpui core hard-codes Zed's own bundled font asset paths
/// (`fonts/ibm-plex-sans/…`, `fonts/lilex/…`) and tries to load them through our `AssetSource`
/// to build a font DB for `<text>` inside SVGs. FreeCell doesn't serve those paths and its icon
/// SVGs contain no `<text>`, so the loads fail harmlessly and that font DB is never used —
/// nothing renders wrong (functional_spec.md §1). An explicit `RUST_LOG` still overrides this,
/// so the warning can be re-enabled for debugging.
const DEFAULT_LOG_FILTER: &str = "info,gpui::svg_renderer=error";

fn main() {
    // Logging setup per architecture.md §8 (tracing + env-filter). Best-effort init.
    // `try_from_default_env` reads `RUST_LOG`, so an explicit `RUST_LOG` still wins.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(DEFAULT_LOG_FILTER)),
        )
        .try_init();

    let exit_after = exit_after_ms();
    let open_path = open_arg();

    // The combined asset source: FreeCell's vendored action-bar icons composed over the
    // gpui-component bundle (`shell::assets`). The bundle still resolves `IconName::Loader`,
    // `ChevronDown`, etc. — see `AppAssets`.
    let app = application().with_assets(AppAssets);
    app.run(move |cx: &mut App| {
        gpui_component::init(cx);
        register_fonts(cx); // registers the bundled Inter faces + sets Inter as the UI font,
                            // before any window opens (best-effort; falls back to the default
                            // font on failure — see fonts.rs)
        cx.activate(true);

        FreeCellApp::init(cx);
        // Load the persisted recent-files list once, at startup (kept out of `init` so gpui
        // tests never read the real per-user data dir — architecture.md §3).
        FreeCellApp::load_recents(cx);
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

#[cfg(test)]
mod tests {
    use super::DEFAULT_LOG_FILTER;
    use tracing_subscriber::EnvFilter;

    /// The default filter (used when `RUST_LOG` is unset) must raise the `gpui::svg_renderer`
    /// target above `WARN` so the two benign bundled-font load warnings never appear
    /// (functional_spec.md §1, architecture.md §1), and it must be a well-formed directive.
    #[test]
    fn default_log_filter_silences_svg_renderer_warning() {
        assert!(
            DEFAULT_LOG_FILTER.contains("gpui::svg_renderer=error"),
            "default filter must silence the gpui::svg_renderer WARN, got: {DEFAULT_LOG_FILTER}",
        );
        EnvFilter::builder()
            .parse(DEFAULT_LOG_FILTER)
            .expect("default log filter must be a valid EnvFilter directive");
    }
}
