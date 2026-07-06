//! The per-scene gpui renderer (`render_scene --scene <name> [--exit-after-ms N]`). Opens ONE
//! window with the chart widget for the named scene and self-quits; the capture harness
//! ([`chart_render::capture`]) forces presentation and grabs the pixels.
//!
//! Isolating each scene in its own process gives a clean gpui/Vulkan lifecycle per capture and
//! rules out stale-pixel races between scenes.

use std::process::ExitCode;

use chart_render::render::run_render_scene;

/// Default window lifetime — long enough for the harness to settle, xrefresh, and capture.
const DEFAULT_EXIT_AFTER_MS: u64 = 9000;

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn main() -> ExitCode {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .try_init();

    let args: Vec<String> = std::env::args().collect();
    let Some(scene) = arg_value(&args, "--scene") else {
        eprintln!("usage: render_scene --scene <name> [--exit-after-ms <n>]");
        return ExitCode::from(2);
    };
    let exit_after_ms = arg_value(&args, "--exit-after-ms")
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_EXIT_AFTER_MS);

    match run_render_scene(&scene, exit_after_ms) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("render_scene failed for scene {scene}: {err:#}");
            ExitCode::FAILURE
        }
    }
}
