//! Coverage diff: IronCalc's registered builtins vs the committed canonical Excel list.
//!
//! Both sides are **committed data files** (`data/ironcalc_functions.csv` from the
//! pinned 0.7.1 source, `data/excel_functions_canonical.csv` from the Microsoft
//! catalog), so the coverage % is reproducible: re-run the diff and it is identical.
//! functional_spec SP3 DELIVERABLE ("coverage matrix committed, reproducible against
//! the cited canonical list").

use std::collections::{BTreeMap, BTreeSet};
use std::io::BufRead;

use anyhow::{Context, Result};
use serde::Serialize;

/// One canonical Excel function with its category + real-world importance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalFn {
    pub name: String,
    pub category: String,
    pub importance: String, // "common" | "obscure"
}

/// The result of diffing IronCalc's registered set against the canonical list.
#[derive(Debug, Clone)]
pub struct Coverage {
    /// Canonical functions IronCalc registers.
    pub supported: Vec<CanonicalFn>,
    /// Canonical functions IronCalc does NOT register.
    pub missing: Vec<CanonicalFn>,
    /// Names IronCalc registers that are absent from the canonical list (should be
    /// empty; a non-empty set flags a naming mismatch to investigate).
    pub extra_in_ironcalc: Vec<String>,
}

/// Overall + sliced coverage percentages for the summary.
#[derive(Debug, Clone, Serialize)]
pub struct CoverageSummary {
    pub canonical_total: usize,
    pub ironcalc_registered: usize,
    pub supported: usize,
    pub missing: usize,
    pub extra_in_ironcalc: usize,
    pub overall_pct: f64,
    /// name → (supported, total, pct)
    pub by_category: BTreeMap<String, CategorySlice>,
    /// "common"/"obscure" → (supported, total, pct)
    pub by_importance: BTreeMap<String, CategorySlice>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CategorySlice {
    pub supported: usize,
    pub total: usize,
    pub pct: f64,
}

fn pct(supported: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        (supported as f64) * 100.0 / (total as f64)
    }
}

/// Reads the canonical CSV, skipping `#`-comment header lines. Columns:
/// `name,category,importance`.
pub fn load_canonical(path: &str) -> Result<Vec<CanonicalFn>> {
    let file = std::fs::File::open(path).with_context(|| format!("open canonical {path}"))?;
    // Strip leading comment lines so `csv` sees a clean header.
    let mut body = String::new();
    for line in std::io::BufReader::new(file).lines() {
        let line = line?;
        if line.starts_with('#') {
            continue;
        }
        body.push_str(&line);
        body.push('\n');
    }
    let mut rdr = csv::Reader::from_reader(body.as_bytes());
    let mut out = Vec::new();
    for rec in rdr.records() {
        let rec = rec?;
        out.push(CanonicalFn {
            name: rec.get(0).unwrap_or_default().trim().to_string(),
            category: rec.get(1).unwrap_or_default().trim().to_string(),
            importance: rec.get(2).unwrap_or_default().trim().to_string(),
        });
    }
    Ok(out)
}

/// Reads the IronCalc registered-function CSV (single `name` column).
pub fn load_ironcalc(path: &str) -> Result<BTreeSet<String>> {
    let mut rdr = csv::Reader::from_path(path).with_context(|| format!("open ironcalc {path}"))?;
    let mut out = BTreeSet::new();
    for rec in rdr.records() {
        let rec = rec?;
        if let Some(name) = rec.get(0) {
            out.insert(name.trim().to_string());
        }
    }
    Ok(out)
}

/// Diffs the canonical list against IronCalc's registered set.
pub fn diff(canonical: &[CanonicalFn], ironcalc: &BTreeSet<String>) -> Coverage {
    let canonical_names: BTreeSet<&str> = canonical.iter().map(|f| f.name.as_str()).collect();
    let mut supported = Vec::new();
    let mut missing = Vec::new();
    for f in canonical {
        if ironcalc.contains(&f.name) {
            supported.push(f.clone());
        } else {
            missing.push(f.clone());
        }
    }
    let extra_in_ironcalc = ironcalc
        .iter()
        .filter(|n| !canonical_names.contains(n.as_str()))
        .cloned()
        .collect();
    Coverage {
        supported,
        missing,
        extra_in_ironcalc,
    }
}

/// Summarizes coverage overall, per category, and per importance.
pub fn summarize(
    canonical: &[CanonicalFn],
    ironcalc: &BTreeSet<String>,
    cov: &Coverage,
) -> CoverageSummary {
    let mut cat_total: BTreeMap<String, usize> = BTreeMap::new();
    let mut cat_supported: BTreeMap<String, usize> = BTreeMap::new();
    let mut imp_total: BTreeMap<String, usize> = BTreeMap::new();
    let mut imp_supported: BTreeMap<String, usize> = BTreeMap::new();

    let supported_names: BTreeSet<&str> = cov.supported.iter().map(|f| f.name.as_str()).collect();
    for f in canonical {
        *cat_total.entry(f.category.clone()).or_default() += 1;
        *imp_total.entry(f.importance.clone()).or_default() += 1;
        if supported_names.contains(f.name.as_str()) {
            *cat_supported.entry(f.category.clone()).or_default() += 1;
            *imp_supported.entry(f.importance.clone()).or_default() += 1;
        }
    }

    let slice = |sup: &BTreeMap<String, usize>, tot: &BTreeMap<String, usize>| {
        tot.iter()
            .map(|(k, &total)| {
                let supported = sup.get(k).copied().unwrap_or(0);
                (
                    k.clone(),
                    CategorySlice {
                        supported,
                        total,
                        pct: pct(supported, total),
                    },
                )
            })
            .collect::<BTreeMap<_, _>>()
    };

    CoverageSummary {
        canonical_total: canonical.len(),
        ironcalc_registered: ironcalc.len(),
        supported: cov.supported.len(),
        missing: cov.missing.len(),
        extra_in_ironcalc: cov.extra_in_ironcalc.len(),
        overall_pct: pct(cov.supported.len(), canonical.len()),
        by_category: slice(&cat_supported, &cat_total),
        by_importance: slice(&imp_supported, &imp_total),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<CanonicalFn> {
        vec![
            CanonicalFn {
                name: "SUM".into(),
                category: "math".into(),
                importance: "common".into(),
            },
            CanonicalFn {
                name: "IF".into(),
                category: "logical".into(),
                importance: "common".into(),
            },
            CanonicalFn {
                name: "BESSELK".into(),
                category: "engineering".into(),
                importance: "obscure".into(),
            },
        ]
    }

    #[test]
    fn diff_partitions_cleanly() {
        let canonical = fixture();
        let ironcalc: BTreeSet<String> =
            ["SUM".to_string(), "IF".to_string(), "ZZZ_ONLY".to_string()]
                .into_iter()
                .collect();
        let cov = diff(&canonical, &ironcalc);
        assert_eq!(cov.supported.len() + cov.missing.len(), canonical.len());
        let supported: BTreeSet<_> = cov.supported.iter().map(|f| f.name.clone()).collect();
        assert!(supported.contains("SUM"));
        assert!(supported.contains("IF"));
        let missing: BTreeSet<_> = cov.missing.iter().map(|f| f.name.clone()).collect();
        assert!(missing.contains("BESSELK"));
        // IronCalc has a name the canonical list lacks -> flagged.
        assert_eq!(cov.extra_in_ironcalc, vec!["ZZZ_ONLY".to_string()]);
    }

    #[test]
    fn summary_slices_by_importance() {
        let canonical = fixture();
        let ironcalc: BTreeSet<String> =
            ["SUM".to_string(), "IF".to_string()].into_iter().collect();
        let cov = diff(&canonical, &ironcalc);
        let s = summarize(&canonical, &ironcalc, &cov);
        // 2 common supported of 2 -> 100%; 0 obscure of 1 -> 0%.
        assert_eq!(s.by_importance["common"].supported, 2);
        assert_eq!(s.by_importance["common"].total, 2);
        assert!((s.by_importance["common"].pct - 100.0).abs() < 1e-9);
        assert_eq!(s.by_importance["obscure"].supported, 0);
        assert!((s.by_importance["obscure"].pct - 0.0).abs() < 1e-9);
    }

    /// The committed IronCalc list must hold exactly the 345 registered functions the
    /// pinned 0.7.1 source declares. Guards against source-extraction drift.
    #[test]
    fn committed_ironcalc_list_has_345() {
        let ic = load_ironcalc("data/ironcalc_functions.csv").expect("load ironcalc csv");
        assert_eq!(ic.len(), 345, "expected 345 registered IronCalc functions");
    }

    /// The committed canonical list must contain the everyday core; a truncated or
    /// corrupt file fails loudly here rather than silently inflating coverage.
    #[test]
    fn committed_canonical_covers_common_core() {
        let canon =
            load_canonical("data/excel_functions_canonical.csv").expect("load canonical csv");
        let names: BTreeSet<&str> = canon.iter().map(|f| f.name.as_str()).collect();
        for core in [
            "SUM", "IF", "VLOOKUP", "INDEX", "MATCH", "TEXT", "ROUND", "DATE", "LEFT", "MID",
            "COUNTIF", "SUMIF", "AVERAGE", "CONCAT", "IFERROR", "XLOOKUP",
        ] {
            assert!(
                names.contains(core),
                "canonical list missing core fn {core}"
            );
        }
        assert!(
            canon.len() > 480,
            "canonical list unexpectedly small: {}",
            canon.len()
        );
    }

    /// Against the real committed files, every IronCalc name resolves in the canonical
    /// list (no stray/misspelled names) — the extra set is empty.
    #[test]
    fn committed_ironcalc_names_all_canonical() {
        let canon =
            load_canonical("data/excel_functions_canonical.csv").expect("load canonical csv");
        let ic = load_ironcalc("data/ironcalc_functions.csv").expect("load ironcalc csv");
        let cov = diff(&canon, &ic);
        assert!(
            cov.extra_in_ironcalc.is_empty(),
            "IronCalc names not in canonical list: {:?}",
            cov.extra_in_ironcalc
        );
    }
}
