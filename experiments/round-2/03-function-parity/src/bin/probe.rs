//! Runtime-probe binary: empirically confirm which canonical functions IronCalc
//! recognizes, and cross-check against the source-extracted static list.
//!
//! Run (foreground): `cargo run --release --bin probe`
//! Writes `results/probe_vs_static.csv` + a short `results/probe_summary.json`.

use anyhow::Result;
use function_parity::coverage::{diff, load_canonical, load_ironcalc};
use function_parity::probe::probe_all;
use function_parity::util::{git_commit, iso_date};

const CANONICAL: &str = "data/excel_functions_canonical.csv";
const IRONCALC: &str = "data/ironcalc_functions.csv";
const RESULTS: &str = "results";

fn main() -> Result<()> {
    let canonical = load_canonical(CANONICAL)?;
    let ironcalc = load_ironcalc(IRONCALC)?;
    let cov = diff(&canonical, &ironcalc);
    let static_supported: std::collections::BTreeSet<String> =
        cov.supported.iter().map(|f| f.name.clone()).collect();

    let names: Vec<String> = canonical.iter().map(|f| f.name.clone()).collect();
    let rows = probe_all(&names, &static_supported);

    std::fs::create_dir_all(RESULTS)?;

    let mut wtr = csv::Writer::from_path(format!("{RESULTS}/probe_vs_static.csv"))?;
    wtr.write_record(["name", "static_supported", "probe_recognized", "agree"])?;
    for r in &rows {
        wtr.write_record([
            r.name.as_str(),
            if r.static_supported { "true" } else { "false" },
            if r.probe_recognized { "true" } else { "false" },
            if r.agree { "true" } else { "false" },
        ])?;
    }
    wtr.flush()?;

    let disagreements: Vec<&function_parity::probe::ProbeRow> =
        rows.iter().filter(|r| !r.agree).collect();
    let probe_recognized = rows.iter().filter(|r| r.probe_recognized).count();

    let env = bench_util::Environment::detect(git_commit());
    let summary = serde_json::json!({
        "generated_utc": iso_date(),
        "environment": env,
        "canonical_total": rows.len(),
        "static_supported": static_supported.len(),
        "probe_recognized": probe_recognized,
        "disagreements": disagreements.len(),
        "disagreement_names": disagreements.iter().map(|r| serde_json::json!({
            "name": r.name,
            "static_supported": r.static_supported,
            "probe_recognized": r.probe_recognized,
        })).collect::<Vec<_>>(),
    });
    std::fs::write(
        format!("{RESULTS}/probe_summary.json"),
        serde_json::to_string_pretty(&summary)?,
    )?;

    println!(
        "probe: static-supported={} probe-recognized={} disagreements={}",
        static_supported.len(),
        probe_recognized,
        disagreements.len()
    );
    for d in &disagreements {
        println!(
            "  DISAGREE {:<20} static={} probe={}",
            d.name, d.static_supported, d.probe_recognized
        );
    }
    println!("wrote results/probe_vs_static.csv, probe_summary.json");
    Ok(())
}
