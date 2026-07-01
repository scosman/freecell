//! Emits the SP5 fidelity matrix (`results/fidelity_matrix.json`) + an env stamp
//! (`results/env.txt`). Run from the crate directory:
//!
//! ```sh
//! cargo run --bin emit
//! ```
//!
//! Every row in the emitted matrix is computed by a real `.xlsx` round-trip inside
//! [`style_fidelity::fidelity_matrix`] and backed by a passing test in `tests/probe.rs`.

use bench_util::Environment;
use style_fidelity::{fidelity_matrix, Fidelity, DATE, ENGINE_VERSION};

fn main() -> std::io::Result<()> {
    std::fs::create_dir_all("results")?;

    let matrix = fidelity_matrix();
    let json = serde_json::to_string_pretty(&matrix).expect("serialize matrix");
    std::fs::write("results/fidelity_matrix.json", json + "\n")?;

    // A small human-readable tally + env stamp for provenance.
    let mut survives = 0;
    let mut lossy = 0;
    let mut dropped = 0;
    let mut not_repr = 0;
    for r in &matrix.rows {
        match r.fidelity {
            Fidelity::Survives => survives += 1,
            Fidelity::Lossy => lossy += 1,
            Fidelity::Dropped => dropped += 1,
            Fidelity::NotRepresentable => not_repr += 1,
        }
    }

    let env = Environment::detect(commit());
    let env_txt = format!(
        "SP5 environment\n\
         os={} arch={} cores={} date={}\n\
         commit={}\n\
         ironcalc={} (pinned, same as round-2 harness)\n\
         matrix rows={} survives={} lossy={} dropped={} not_representable={}\n",
        env.os,
        env.arch,
        env.cores,
        DATE,
        env.commit,
        ENGINE_VERSION,
        matrix.rows.len(),
        survives,
        lossy,
        dropped,
        not_repr,
    );
    std::fs::write("results/env.txt", env_txt)?;

    println!(
        "SP5 emit: wrote results/fidelity_matrix.json ({} rows: {} survives, {} lossy, {} dropped, {} not-representable), results/env.txt",
        matrix.rows.len(),
        survives,
        lossy,
        dropped,
        not_repr,
    );
    Ok(())
}

/// Best-effort short commit hash for provenance; `"unknown"` if git is unavailable
/// (matches the sibling SP experiments' convention).
fn commit() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}
