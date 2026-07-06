//! `render_loaded` — the gpui side of the load render-proof: parse a chart OUT of an `.xlsx`
//! fixture (via [`load_save::load`]) and render it through `chart-render`'s widgets in ONE
//! window, mirroring chart-render's `render_scene`. The [`capture_loaded`] harness forces
//! presentation and grabs the pixels.
//!
//! `render_loaded --fixture <path> --chart-index <n> --width <w> --height <h> [--exit-after-ms <n>]`
//!
//! This binary is the whole point of Gate 4's load half: it exercises the full seam
//! **parse → chart-model → render** on a real file, not a hand-built in-memory `Chart`.

use std::process::ExitCode;

use anyhow::{anyhow, Context, Result};

fn arg(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let fixture = arg(&args, "--fixture").ok_or_else(|| anyhow!("--fixture <path> required"))?;
    let index: usize = arg(&args, "--chart-index")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let width: u32 = arg(&args, "--width")
        .and_then(|s| s.parse().ok())
        .unwrap_or(720);
    let height: u32 = arg(&args, "--height")
        .and_then(|s| s.parse().ok())
        .unwrap_or(460);
    let exit_after_ms: u64 = arg(&args, "--exit-after-ms")
        .and_then(|s| s.parse().ok())
        .unwrap_or(9000);

    let charts = load_save::load::load_charts_from_xlsx(std::path::Path::new(&fixture))
        .with_context(|| format!("loading charts from {fixture}"))?;
    let chart = charts
        .get(index)
        .cloned()
        .ok_or_else(|| anyhow!("chart index {index} out of range ({} charts)", charts.len()))?;

    chart_render::render::run_render_chart(chart, (width, height), exit_after_ms)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("render_loaded failed: {e:#}");
            ExitCode::FAILURE
        }
    }
}
