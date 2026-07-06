//! `capture_loaded` — headless PNG proof for the LOAD half of Gate 4. For each chart parsed out
//! of the authored fixture, it launches [`render_loaded`] under its own Xvfb + lavapipe display
//! and captures the rendered chart to `results/loaded_<kind>.png`, reusing `chart-render`'s
//! proven `capture_window` path. Also writes `results/manifest.json` for the agent review.
//!
//! Run: `cargo run -p load-save --features render --bin capture_loaded`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{anyhow, bail, Context, Result};
use chart_model::{Chart, ChartKind};
use chart_render::capture::{
    capture_available, default_exit_after_ms, lavapipe_icd, sibling_bin, ManifestEntry,
};
use load_save::{authoring, load};

const VIEWPORT: (u32, u32) = (720, 460);

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A short, filename-safe name for a loaded chart, e.g. `loaded_column` / `loaded_line`.
fn chart_slug(chart: &Chart) -> &'static str {
    match chart.kind {
        ChartKind::Bar { .. } => "loaded_column",
        ChartKind::Line { .. } => "loaded_line",
        ChartKind::Area { .. } => "loaded_area",
        ChartKind::Pie {
            doughnut_hole: None,
        } => "loaded_pie",
        ChartKind::Pie { .. } => "loaded_doughnut",
        ChartKind::Scatter => "loaded_scatter",
    }
}

/// The per-kind expectation the reviewer agent judges against.
fn expectation(chart: &Chart) -> String {
    let title = chart.title.as_deref().unwrap_or("(untitled)");
    let series = chart.series.len();
    match chart.kind {
        ChartKind::Bar { .. } => format!(
            "A clustered column chart titled '{title}' with {series} series drawn side-by-side \
             within each of four quarters (Q1-Q4), distinct colors matching a legend, a readable \
             numeric value axis, and axis titles. Rendered FROM data parsed out of an .xlsx. \
             Non-blank, no clipping."
        ),
        ChartKind::Line { .. } => format!(
            "A multi-series line chart titled '{title}' with {series} straight-segment lines over \
             four quarters on one shared numeric value axis, dot markers, distinct colors matching \
             a legend, and axis titles. Rendered FROM data parsed out of an .xlsx. Non-blank."
        ),
        ChartKind::Pie { .. } => format!(
            "A pie chart titled '{title}' divided into four wedges in DISTINCT colors (one per \
             quarter), on-slice percentage labels, and a legend mapping each quarter to its slice. \
             Rendered FROM data parsed out of an .xlsx. Non-blank, no clipping."
        ),
        _ => format!(
            "A chart titled '{title}' with {series} series, rendered from parsed .xlsx data."
        ),
    }
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn run() -> Result<()> {
    if !capture_available() {
        bail!(
            "capture tooling unavailable (needs xvfb-run + lavapipe ICD + ImageMagick import + \
             xwininfo + xrefresh)"
        );
    }
    let icd = lavapipe_icd().ok_or_else(|| anyhow!("no lavapipe ICD found"))?;
    let render_bin = sibling_bin("render_loaded")?;

    let root = crate_dir();
    let fixtures_dir = root.join("fixtures");
    let results_dir = root.join("results");
    std::fs::create_dir_all(&fixtures_dir)?;
    std::fs::create_dir_all(&results_dir)?;

    // Ensure the fixture exists (author it if the `fixtures` bin hasn't been run yet).
    let fixture = fixtures_dir.join("charts_basic.xlsx");
    if !fixture.exists() {
        authoring::write_fixture(&fixture)?;
    }

    let charts = load::load_charts_from_xlsx(&fixture)
        .with_context(|| format!("loading charts from {}", fixture.display()))?;
    if charts.is_empty() {
        bail!("no charts loaded from {}", fixture.display());
    }

    let fixture_arg = shell_quote(fixture.to_str().context("fixture path utf-8")?);
    let render_arg = shell_quote(render_bin.to_str().context("render_loaded path utf-8")?);
    let exit_after_ms = default_exit_after_ms();
    let (w, h) = VIEWPORT;

    let mut manifest: Vec<ManifestEntry> = Vec::new();
    for (i, chart) in charts.iter().enumerate() {
        let slug = chart_slug(chart);
        let out = results_dir.join(format!("{slug}.png"));
        let launch = format!(
            "{render_arg} --fixture {fixture_arg} --chart-index {i} --width {w} --height {h} \
             --exit-after-ms {exit_after_ms}"
        );
        capture_window_wrapped(&launch, &icd, &out)
            .with_context(|| format!("capturing loaded chart {i} ({slug})"))?;
        println!("captured {}", out.display());

        manifest.push(ManifestEntry {
            name: slug.to_string(),
            png: format!("{slug}.png"),
            description: format!(
                "Loaded from charts_basic.xlsx and rendered through chart-render: {} ({} series).",
                chart.title.as_deref().unwrap_or("(untitled)"),
                chart.series.len()
            ),
            expectation: expectation(chart),
        });
    }

    let manifest_path = results_dir.join("manifest.json");
    std::fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).context("serializing manifest")?,
    )
    .with_context(|| format!("writing {}", manifest_path.display()))?;
    println!("wrote {}", manifest_path.display());

    Ok(())
}

/// Thin wrapper so a capture failure names the output file.
fn capture_window_wrapped(launch: &str, icd: &Path, out: &Path) -> Result<()> {
    chart_render::capture::capture_window(launch, VIEWPORT, icd, out)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("capture_loaded failed: {e:#}");
            ExitCode::FAILURE
        }
    }
}
