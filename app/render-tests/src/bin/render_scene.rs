//! The per-case gpui renderer:
//! - `render_scene --case <name>` opens ONE window with the real grid over the named grid case's
//!   engine-built sources, and
//! - `render_scene --chart <name>` opens ONE window with the standalone chart widget for the
//!   named chart scene.
//!
//! Either way it self-quits; the capture harness ([`render_tests::capture`]) forces presentation
//! and grabs the pixels. Isolating each fixture in its own process gives a clean gpui/Vulkan
//! lifecycle per capture, needs no window-resize API, and rules out stale-pixel races.

use std::process::ExitCode;

use render_tests::render::{run_chart_scene, run_render_scene};

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
    let exit_after_ms = arg_value(&args, "--exit-after-ms")
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_EXIT_AFTER_MS);

    // `--chart` (a chart scene) and `--case` (a grid case) are mutually exclusive; `--chart`
    // takes precedence if both are somehow given.
    let (kind, name, result) = if let Some(scene) = arg_value(&args, "--chart") {
        let r = run_chart_scene(&scene, exit_after_ms);
        ("chart scene", scene, r)
    } else if let Some(case) = arg_value(&args, "--case") {
        let r = run_render_scene(&case, exit_after_ms);
        ("case", case, r)
    } else {
        eprintln!("usage: render_scene (--case <name> | --chart <name>) [--exit-after-ms <n>]");
        return ExitCode::from(2);
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("render_scene failed for {kind} {name}: {err:#}");
            ExitCode::FAILURE
        }
    }
}
