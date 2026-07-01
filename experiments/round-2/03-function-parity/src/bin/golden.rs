//! Golden-file correctness binary (functional_spec SP3 GATE).
//!
//! Runs every case in `data/golden_cases.csv` through the frozen IronCalc adapter,
//! compares (errors as **typed** errors), and writes the pass rate + itemized failures
//! to `results/`.
//!
//! Run (foreground, timeout-wrapped): `timeout 600 cargo run --release --bin golden`

use std::collections::BTreeMap;

use anyhow::Result;
use function_parity::cases_io::load_cases;
use function_parity::golden::run_all;
use function_parity::util::{git_commit, iso_date};

const CASES: &str = "data/golden_cases.csv";
const RESULTS: &str = "results";

fn main() -> Result<()> {
    let cases = load_cases(CASES)?;
    let results = run_all(&cases);
    std::fs::create_dir_all(RESULTS)?;

    let total = results.len();
    let passed = results.iter().filter(|r| r.pass).count();
    let pass_rate = if total == 0 {
        0.0
    } else {
        passed as f64 * 100.0 / total as f64
    };

    // Per-category breakdown.
    let mut cat_total: BTreeMap<String, usize> = BTreeMap::new();
    let mut cat_pass: BTreeMap<String, usize> = BTreeMap::new();
    for r in &results {
        *cat_total.entry(r.category.clone()).or_default() += 1;
        if r.pass {
            *cat_pass.entry(r.category.clone()).or_default() += 1;
        }
    }

    // 1) Per-case results CSV.
    {
        let mut wtr = csv::Writer::from_path(format!("{RESULTS}/golden_results.csv"))?;
        wtr.write_record([
            "id", "category", "formula", "expected", "actual", "pass", "reason",
        ])?;
        for r in &results {
            wtr.write_record([
                r.id.as_str(),
                r.category.as_str(),
                r.formula.as_str(),
                r.expected.as_str(),
                r.actual.as_str(),
                if r.pass { "true" } else { "false" },
                r.reason.as_str(),
            ])?;
        }
        wtr.flush()?;
    }

    // 2) Summary JSON, env-stamped.
    let per_category: BTreeMap<String, serde_json::Value> = cat_total
        .iter()
        .map(|(k, &t)| {
            let p = cat_pass.get(k).copied().unwrap_or(0);
            (
                k.clone(),
                serde_json::json!({ "passed": p, "total": t, "pass_rate": p as f64 * 100.0 / t as f64 }),
            )
        })
        .collect();
    let env = bench_util::Environment::detect(git_commit());
    let summary = serde_json::json!({
        "generated_utc": iso_date(),
        "environment": env,
        "engine": "ironcalc 0.7.1 (frozen round-2 harness adapter)",
        "total_cases": total,
        "passed": passed,
        "failed": total - passed,
        "pass_rate": pass_rate,
        "gate_min_cases": 100,
        "gate_met": total >= 100,
        "per_category": per_category,
    });
    std::fs::write(
        format!("{RESULTS}/golden_summary.json"),
        serde_json::to_string_pretty(&summary)?,
    )?;

    // 3) Itemized failures markdown.
    write_failures_md(&results, total, passed, pass_rate, &cat_total, &cat_pass)?;

    println!(
        "golden: {passed}/{total} passed = {pass_rate:.1}%  (GATE >=100 cases: {})",
        if total >= 100 { "MET" } else { "NOT MET" }
    );
    for (cat, &t) in &cat_total {
        let p = cat_pass.get(cat).copied().unwrap_or(0);
        println!("  {cat:<20} {p}/{t}");
    }
    println!("wrote results/golden_results.csv, golden_summary.json, golden_failures.md");
    Ok(())
}

fn write_failures_md(
    results: &[function_parity::golden::CaseResult],
    total: usize,
    passed: usize,
    pass_rate: f64,
    cat_total: &BTreeMap<String, usize>,
    cat_pass: &BTreeMap<String, usize>,
) -> Result<()> {
    let mut md = String::new();
    md.push_str("# SP3 golden-file results — IronCalc 0.7.1 vs known-correct Excel\n\n");
    md.push_str(&format!(
        "**{passed}/{total} passed = {pass_rate:.1}%.** GATE (>=~100 cases): {}.\n\n",
        if total >= 100 { "MET" } else { "NOT MET" }
    ));
    md.push_str("## Per-category pass rate\n\n| category | passed | total | rate |\n");
    md.push_str("|----------|--------|-------|------|\n");
    for (cat, &t) in cat_total {
        let p = cat_pass.get(cat).copied().unwrap_or(0);
        md.push_str(&format!(
            "| {cat} | {p} | {t} | {:.1}% |\n",
            p as f64 * 100.0 / t as f64
        ));
    }

    let failures: Vec<_> = results.iter().filter(|r| !r.pass).collect();
    md.push_str(&format!("\n## Itemized failures ({})\n\n", failures.len()));
    if failures.is_empty() {
        md.push_str("_No failures._\n");
    } else {
        md.push_str("| id | formula | expected | actual | reason |\n");
        md.push_str("|----|---------|----------|--------|--------|\n");
        for r in &failures {
            md.push_str(&format!(
                "| `{}` | `{}` | {} | {} | {} |\n",
                r.id,
                r.formula.replace('|', "\\|"),
                r.expected.replace('|', "\\|"),
                r.actual.replace('|', "\\|"),
                r.reason.replace('|', "\\|"),
            ));
        }
    }
    std::fs::write(format!("{RESULTS}/golden_failures.md"), md)?;
    Ok(())
}
