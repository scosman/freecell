//! `gen` — generate the reproducible styled `.xlsx` from committed code.
//!
//! Usage:  `cargo run --release --bin gen -- [target_mb] [out_path]`
//!
//! - `target_mb` (default 100): grow the row count until the on-disk file crosses this
//!   size, so the generated file is genuinely ≥ the SP2 target from one command.
//! - `out_path` (default `data/large.xlsx`): where to write.
//!
//! Generation wall-clock is printed but kept **separate** from open time (benchmark
//! discipline). Run foreground with `timeout`; if a size risks the ~15 GB box, cap
//! `target_mb` and record the ceiling — do not OOM.

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use xlsx_open::{generate_until_target, GenSpec};

const BYTES_PER_MB: u64 = 1024 * 1024;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let target_mb: u64 = args
        .next()
        .map(|s| s.parse())
        .transpose()
        .context("target_mb must be an integer")?
        .unwrap_or(100);
    let out_path: PathBuf = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data/large.xlsx"));

    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }

    let target_bytes = target_mb * BYTES_PER_MB;
    println!(
        "SP2 gen: target >= {target_mb} MB ({target_bytes} bytes) -> {}",
        out_path.display()
    );

    let overall = Instant::now();
    let (spec, report) = generate_until_target(
        GenSpec::large(),
        target_bytes,
        &out_path,
        |attempt, spec, report| {
            println!(
                "  attempt {attempt}: sheets={} rows={} cols={} -> {:.1} MB \
                 (build {:.2}s, write {:.2}s, {} cells)",
                spec.sheets,
                spec.rows,
                spec.cols,
                report.file_bytes as f64 / BYTES_PER_MB as f64,
                report.build.as_secs_f64(),
                report.write.as_secs_f64(),
                spec.total_cells(),
            );
        },
    )?;

    println!(
        "SP2 gen: DONE in {:.2}s total. Final file {:.1} MB at {} \
         (seed={}, sheets={}, rows={}, cols={}).",
        overall.elapsed().as_secs_f64(),
        report.file_bytes as f64 / BYTES_PER_MB as f64,
        out_path.display(),
        spec.seed,
        spec.sheets,
        spec.rows,
        spec.cols,
    );
    Ok(())
}
