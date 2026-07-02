//! Re-verify the committed macOS-rendered PNGs, in-container.
//!
//! The human's macOS run (2026-07-02) rendered `results/{baseline,rerender,changed}.png`
//! offscreen via Metal and committed them. This example re-runs the exact perceptual
//! diffs the GATE requires — `diff(baseline, rerender)` MUST PASS and
//! `diff(baseline, changed)` MUST FAIL — using the same `DiffOptions::default()` the
//! render-grid `--diff` subcommand uses. Runs anywhere (no gpui):
//!
//! ```sh
//! cargo run --example diff_committed
//! ```
//!
//! Exits non-zero if either expectation is violated.

use std::path::Path;

use ci_rendering::{diff_png_files, DiffOptions};

fn main() -> anyhow::Result<()> {
    let results = Path::new(env!("CARGO_MANIFEST_DIR")).join("results");
    let opts = DiffOptions::default();

    let same = diff_png_files(
        &results.join("baseline.png"),
        &results.join("rerender.png"),
        &opts,
    )?;
    println!("baseline vs rerender: {}", same.summary());

    let changed = diff_png_files(
        &results.join("baseline.png"),
        &results.join("changed.png"),
        &opts,
    )?;
    println!("baseline vs changed:  {}", changed.summary());

    anyhow::ensure!(same.passed, "stable re-render must PASS within tolerance");
    anyhow::ensure!(!changed.passed, "deliberate change must FAIL the diff");
    println!("GATE expectations hold: re-render PASS, changed FAIL.");
    Ok(())
}
