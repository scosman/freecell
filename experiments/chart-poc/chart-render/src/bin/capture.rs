//! The capture harness (`capture [--scene <prefix>] [--out <dir>]`): for each scene, run
//! `render_scene` under its own Xvfb display + lavapipe, force presentation with `xrefresh`,
//! capture the window to `results/<name>.png`, and emit `results/manifest.json`.
//!
//! The manifest is the input to the agent-review step (functional_spec §6): each entry pairs a
//! PNG with the `expectation` a reviewer agent judges it against.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use chart_render::capture;

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// Default output dir: this crate's `results/` (committed alongside the code).
fn default_results_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("results")
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let out_dir = arg_value(&args, "--out")
        .map(PathBuf::from)
        .unwrap_or_else(default_results_dir);
    let only = arg_value(&args, "--scene");

    if !capture::capture_available() {
        eprintln!(
            "capture tooling unavailable — need xvfb-run + a lavapipe ICD \
             (mesa-vulkan-drivers) + xrefresh (x11-xserver-utils) + xwininfo (x11-utils) + \
             import (imagemagick). See chart-render/findings.md."
        );
        return ExitCode::from(3);
    }

    let render_scene_bin = match capture::sibling_render_scene_bin() {
        Ok(bin) => bin,
        Err(err) => {
            eprintln!("could not locate render_scene bin: {err:#}");
            return ExitCode::FAILURE;
        }
    };

    match capture::render_all(&render_scene_bin, &out_dir, only.as_deref()) {
        Ok(names) => {
            println!(
                "captured {} scene(s) into {}: {}",
                names.len(),
                out_dir.display(),
                names.join(", ")
            );
            println!("wrote {}", out_dir.join("manifest.json").display());
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("capture failed: {err:#}");
            ExitCode::FAILURE
        }
    }
}
