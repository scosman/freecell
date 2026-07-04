//! The per-case gpui renderer (`render_scene --case <name> [--exit-after-ms N]`). Opens ONE
//! window with the real grid over the named case's engine-built sources and self-quits; the
//! capture harness ([`render_tests::capture`]) forces presentation and grabs the pixels.
//!
//! Isolating each case in its own process gives a clean gpui/Vulkan lifecycle per capture, needs
//! no window-resize API, and rules out stale-pixel races between cases.

use std::process::ExitCode;

use render_tests::render::run_render_scene;

/// Default window lifetime — long enough for the harness to settle, xrefresh, and capture.
const DEFAULT_EXIT_AFTER_MS: u64 = 9000;

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn main() -> ExitCode {
    // Best-effort logging (mirrors the app bin); a second init in embedded use is ignored.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .try_init();

    let args: Vec<String> = std::env::args().collect();
    let Some(case) = arg_value(&args, "--case") else {
        eprintln!("usage: render_scene --case <name> [--exit-after-ms <n>]");
        return ExitCode::from(2);
    };
    let exit_after_ms = arg_value(&args, "--exit-after-ms")
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_EXIT_AFTER_MS);

    match run_render_scene(&case, exit_after_ms) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("render_scene failed for case {case}: {err:#}");
            ExitCode::FAILURE
        }
    }
}
