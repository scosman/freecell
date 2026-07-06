//! `fixtures` — authors the example `.xlsx` fixture (functional_spec §10 #4) and runs the save
//! round-trip (§5), writing the committed artifacts under the crate:
//!
//! - `fixtures/charts_basic.xlsx` — the authored source (column + line + pie charts).
//! - `results/roundtrip_charts_basic.xlsx` — the same file after
//!   `load → IronCalc save → chart re-injection`.
//!
//! gpui-free (default features only). Run: `cargo run -p load-save --bin fixtures`.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use load_save::{authoring, load, save};

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn run() -> Result<()> {
    let root = crate_dir();
    let fixtures_dir = root.join("fixtures");
    let results_dir = root.join("results");
    std::fs::create_dir_all(&fixtures_dir)?;
    std::fs::create_dir_all(&results_dir)?;

    // 1. Author the source fixture.
    let fixture = fixtures_dir.join("charts_basic.xlsx");
    let parts = authoring::write_fixture(&fixture)?;
    println!("authored {} ({} charts)", fixture.display(), parts.len());

    // 2. Confirm we can load the charts back out (the seam the PoC proves).
    let charts = load::load_charts_from_xlsx(&fixture)?;
    println!("loaded {} charts from the fixture:", charts.len());
    for chart in &charts {
        println!(
            "  - {:?}  kind={:?}  series={}",
            chart.title.as_deref().unwrap_or("(untitled)"),
            chart.kind,
            chart.series.len()
        );
    }

    // 3. Save round-trip through IronCalc's (chart-dropping) writer + re-injection.
    let roundtrip = results_dir.join("roundtrip_charts_basic.xlsx");
    let report = save::save_with_charts(&fixture, &roundtrip)?;
    println!(
        "round-tripped to {} (charts preserved: {}, sheets patched: {:?})",
        roundtrip.display(),
        report.charts_preserved,
        report.patched_sheets
    );

    // 4. Verify the re-injected file: reopen with our loader + with IronCalc.
    let reopened = load::load_charts_from_xlsx(&roundtrip)?;
    assert_eq!(
        reopened, charts,
        "round-tripped charts must match the originals"
    );
    let out_str = roundtrip.to_str().expect("utf-8 path");
    ironcalc::import::load_from_xlsx(out_str, "en", "UTC", "en")
        .map_err(|e| anyhow::anyhow!("round-tripped file failed to reopen in IronCalc: {e:?}"))?;
    println!(
        "VERIFIED: re-injected file reopens with our loader ({} charts) AND with IronCalc.",
        reopened.len()
    );

    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("fixtures failed: {e:#}");
            ExitCode::FAILURE
        }
    }
}
