//! `generate_baselines [--only <prefix>]` — re-render every (or a filtered) case and write the
//! results into `render-tests/baselines/`, printing a **changed / new / unchanged** summary
//! (`components/render_test_harness.md §Runner`).
//!
//! MUST run on the pinned CI runner image + Mesa/lavapipe version (or a matching container), under
//! Xvfb — use `scripts/render_tests.sh generate`. The human then **visually inspects every
//! changed PNG** before committing (see `render-tests/README.md`). Dev-machine renders are for
//! eyeballing only, never committed as baselines.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use render_tests::diff::{diff_png_files, DiffOptions};
use render_tests::{render_all, render_charts, sibling_render_scene_bin};

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn baselines_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("baselines")
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let only = arg_value(&args, "--only");

    let render_scene_bin = sibling_render_scene_bin()?;
    let baselines = baselines_dir();
    std::fs::create_dir_all(&baselines)?;

    // Render into a temp dir first so we can classify each result against the existing baseline
    // before overwriting it. Grid cases + chart scenes share `baselines/` (names never collide —
    // chart scenes are `chart_*`); the `--only <prefix>` filter selects across both (e.g.
    // `--only chart_` renders only chart scenes, `--only cell_` only grid cells).
    let staging = std::env::temp_dir().join(format!("freecell-baselines-{}", std::process::id()));
    let mut rendered = render_all(&render_scene_bin, &staging, only.as_deref())?;
    rendered.extend(render_charts(&render_scene_bin, &staging, only.as_deref())?);

    let opts = DiffOptions::default();
    let (mut new, mut changed, mut unchanged) = (Vec::new(), Vec::new(), Vec::new());
    for name in &rendered {
        let fresh = staging.join(format!("{name}.png"));
        let committed = baselines.join(format!("{name}.png"));
        if !committed.exists() {
            new.push(name.clone());
        } else {
            match diff_png_files(&committed, &fresh, &opts) {
                Ok(report) if report.passed => unchanged.push(name.clone()),
                _ => changed.push(name.clone()), // pixel change or a size change
            }
        }
        std::fs::copy(&fresh, &committed).with_context(|| format!("copying baseline {name}"))?;
    }
    let _ = std::fs::remove_dir_all(&staging);

    println!(
        "baselines: {} total ({} new, {} changed, {} unchanged) → {}",
        rendered.len(),
        new.len(),
        changed.len(),
        unchanged.len(),
        baselines.display()
    );
    for name in &new {
        println!("  NEW       {name}");
    }
    for name in &changed {
        println!("  CHANGED   {name}  (eyeball before committing)");
    }
    if !new.is_empty() || !changed.is_empty() {
        println!(
            "\nReview the NEW/CHANGED PNGs by eye, then commit them WITH the code change that \
             moved the pixels (render-tests/README.md)."
        );
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("generate_baselines failed: {err:#}");
            ExitCode::FAILURE
        }
    }
}
