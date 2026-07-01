//! Coverage-diff binary: writes the committed coverage matrix + summary to `results/`.
//!
//! Run (foreground): `cargo run --release --bin coverage`
//! Reproducible: both inputs are committed data files; output is deterministic.

use anyhow::{Context, Result};
use function_parity::coverage::{diff, load_canonical, load_ironcalc, summarize, CanonicalFn};
use function_parity::util::{git_commit, iso_date};

const CANONICAL: &str = "data/excel_functions_canonical.csv";
const IRONCALC: &str = "data/ironcalc_functions.csv";
const RESULTS: &str = "results";

fn main() -> Result<()> {
    let canonical = load_canonical(CANONICAL)?;
    let ironcalc = load_ironcalc(IRONCALC)?;
    let cov = diff(&canonical, &ironcalc);
    let summary = summarize(&canonical, &ironcalc, &cov);

    std::fs::create_dir_all(RESULTS)?;

    // 1) Per-function matrix CSV: name, category, importance, supported.
    write_matrix_csv(&canonical, &ironcalc)?;

    // 2) Machine-readable summary JSON, env-stamped.
    let env = bench_util::Environment::detect(git_commit());
    let json = serde_json::json!({
        "generated_utc": iso_date(),
        "environment": env,
        "canonical_source": "Microsoft 'Excel functions (alphabetical)' catalog (Excel for Microsoft 365); committed data/excel_functions_canonical.csv",
        "ironcalc_source": "ironcalc_base 0.7.1 src/functions/mod.rs Function enum (345 registered); committed data/ironcalc_functions.csv",
        "summary": summary,
    });
    std::fs::write(
        format!("{RESULTS}/coverage_summary.json"),
        serde_json::to_string_pretty(&json)?,
    )?;

    // 3) Human-readable markdown summary.
    write_summary_md(&cov, &summary)?;

    println!(
        "coverage: {}/{} = {:.1}% overall  |  common {:.1}%  |  obscure {:.1}%",
        summary.supported,
        summary.canonical_total,
        summary.overall_pct,
        summary
            .by_importance
            .get("common")
            .map(|s| s.pct)
            .unwrap_or(0.0),
        summary
            .by_importance
            .get("obscure")
            .map(|s| s.pct)
            .unwrap_or(0.0),
    );
    if !cov.extra_in_ironcalc.is_empty() {
        println!(
            "WARNING: {} IronCalc names not in canonical list: {:?}",
            cov.extra_in_ironcalc.len(),
            cov.extra_in_ironcalc
        );
    }
    println!("wrote results/coverage_matrix.csv, coverage_summary.json, coverage_summary.md");
    Ok(())
}

fn write_matrix_csv(
    canonical: &[CanonicalFn],
    ironcalc: &std::collections::BTreeSet<String>,
) -> Result<()> {
    let mut wtr = csv::Writer::from_path(format!("{RESULTS}/coverage_matrix.csv"))
        .context("open coverage_matrix.csv")?;
    wtr.write_record(["name", "category", "importance", "supported"])?;
    let mut sorted = canonical.to_vec();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));
    for f in &sorted {
        let supported = ironcalc.contains(&f.name);
        wtr.write_record([
            f.name.as_str(),
            f.category.as_str(),
            f.importance.as_str(),
            if supported { "true" } else { "false" },
        ])?;
    }
    wtr.flush()?;
    Ok(())
}

fn write_summary_md(
    cov: &function_parity::coverage::Coverage,
    s: &function_parity::coverage::CoverageSummary,
) -> Result<()> {
    let mut md = String::new();
    md.push_str("# SP3 coverage matrix — IronCalc 0.7.1 vs canonical Excel list\n\n");
    md.push_str(&format!(
        "Overall: **{}/{} = {:.1}%** of the canonical Excel catalog is registered in \
         IronCalc 0.7.1.\n\n",
        s.supported, s.canonical_total, s.overall_pct
    ));
    md.push_str("## By importance\n\n| importance | supported | total | coverage |\n");
    md.push_str("|-----------|-----------|-------|----------|\n");
    for (k, slice) in &s.by_importance {
        md.push_str(&format!(
            "| {k} | {} | {} | {:.1}% |\n",
            slice.supported, slice.total, slice.pct
        ));
    }
    md.push_str("\n## By category\n\n| category | supported | total | coverage |\n");
    md.push_str("|----------|-----------|-------|----------|\n");
    for (k, slice) in &s.by_category {
        md.push_str(&format!(
            "| {k} | {} | {} | {:.1}% |\n",
            slice.supported, slice.total, slice.pct
        ));
    }

    // Missing common functions — the decision-driving list.
    let mut missing_common: Vec<&CanonicalFn> = cov
        .missing
        .iter()
        .filter(|f| f.importance == "common")
        .collect();
    missing_common.sort_by(|a, b| a.name.cmp(&b.name));
    md.push_str(&format!(
        "\n## Missing COMMON functions ({}) — the off-ramp set\n\n",
        missing_common.len()
    ));
    if missing_common.is_empty() {
        md.push_str("_None — every function tagged `common` is registered._\n");
    } else {
        for f in &missing_common {
            md.push_str(&format!("- `{}` ({})\n", f.name, f.category));
        }
    }
    std::fs::write(format!("{RESULTS}/coverage_summary.md"), md)?;
    Ok(())
}
