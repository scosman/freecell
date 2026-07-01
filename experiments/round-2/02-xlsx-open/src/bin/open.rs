//! `open` — the FRESH CHILD PROCESS that performs the real SP2 open measurement.
//!
//! Usage:  `open <path> <seed> <sheets> <rows> <cols>`
//!
//! This binary does nothing but open the file once and stamp its own peak RSS, so its
//! VmHWM high-water mark is the honest **open-only** peak memory — not the polluted
//! high-water of a long-lived harness process (architecture §3: peak RSS MUST come from
//! a separately-spawned child). It prints exactly one JSON line to stdout, which the
//! `measure` parent parses.
//!
//! The spec args (seed/sheets/rows/cols) let this process recompute the deterministic
//! **sentinel** expectation so it can force + assert the measured op (the cached value
//! at first paint and the recomputed value after first eval both equal the known number).
//!
//! Peak RSS is read from `round2_harness::peak_rss()` — the CANONICAL VmHWM helper — and
//! explicitly NOT `sysinfo::peak_rss_bytes` (which returns 0 on `/proc` failure).

use std::path::PathBuf;

use anyhow::{Context, Result};
use xlsx_open::{open_stages, sentinel, GenSpec, OpenStages};

/// The single JSON line this child emits. Field units are nanoseconds for stages and
/// bytes for sizes, matching the parent's expectations.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct ChildReport {
    pub file_bytes: u64,
    pub peak_rss_bytes: u64,
    pub stages: OpenStages,
}

fn parse<T: std::str::FromStr>(args: &mut impl Iterator<Item = String>, name: &str) -> Result<T>
where
    T::Err: std::fmt::Display,
{
    let raw = args
        .next()
        .with_context(|| format!("missing arg <{name}>"))?;
    raw.parse::<T>()
        .map_err(|e| anyhow::anyhow!("bad <{name}> '{raw}': {e}"))
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let path: PathBuf = args
        .next()
        .map(PathBuf::from)
        .context("missing arg <path>")?;
    let spec = GenSpec {
        seed: parse(&mut args, "seed")?,
        sheets: parse(&mut args, "sheets")?,
        rows: parse(&mut args, "rows")?,
        cols: parse(&mut args, "cols")?,
    };

    let file_bytes = std::fs::metadata(&path)
        .with_context(|| format!("stat {}", path.display()))?
        .len();

    // The one measured op. open_stages force+asserts the sentinel internally.
    let stages = open_stages(&path, sentinel(&spec))?;

    // Stamp the CANONICAL peak RSS from this fresh child (VmHWM high-water mark).
    let peak_rss_bytes = round2_harness::peak_rss();

    let report = ChildReport {
        file_bytes,
        peak_rss_bytes,
        stages,
    };
    // Exactly one line of JSON on stdout for the parent to parse.
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}
