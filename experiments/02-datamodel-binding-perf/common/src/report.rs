//! Results recording for the bake-off: turns scenario latencies into env-stamped
//! [`bench_util::BenchResult`]s, writes one JSON file per (engine, scenario, design)
//! under `results/<engine>/`, and appends a human-readable `results/summary.md` table
//! (architecture §3). The `scenarios` bins in each engine crate call this.

use std::io;
use std::path::Path;

use bench_util::{BenchResult, Environment, GateResult, LatencyStats};
use serde_json::json;

/// A single recorded scenario outcome, ready to serialize and to add to `summary.md`.
pub struct ScenarioResult {
    pub result: BenchResult,
    pub engine: String,
    pub scenario: String,
    pub design: Option<String>,
}

impl ScenarioResult {
    /// Builds a [`ScenarioResult`] from latency samples + an optional gate.
    ///
    /// `gate` is the pass/fail check (or `None` for discovery-only metrics like
    /// memory). `extra` carries any secondary metrics (peak RSS, single/batched
    /// ratio, ...).
    #[allow(clippy::too_many_arguments)]
    pub fn from_stats(
        engine: &str,
        scenario: &str,
        design: Option<&str>,
        input_size: u64,
        date: &str,
        env: Environment,
        stats: Option<LatencyStats>,
        gate: Option<GateResult>,
        extra: serde_json::Value,
    ) -> Self {
        let name = match design {
            Some(d) => format!("{engine}/{scenario}/{d}"),
            None => format!("{engine}/{scenario}"),
        };
        let mut result = BenchResult::new(name, input_size, date, env).with_extra(json!({
            "engine": engine,
            "scenario": scenario,
            "design": design,
            "metrics": extra,
        }));
        if let Some(s) = stats {
            result = result.with_stats(s);
        }
        if let Some(g) = gate {
            result = result.with_gate(g);
        }
        Self {
            result,
            engine: engine.to_string(),
            scenario: scenario.to_string(),
            design: design.map(str::to_string),
        }
    }

    /// The file stem for this result's JSON (`<scenario>[-<design>]`).
    fn file_stem(&self) -> String {
        match &self.design {
            Some(d) => format!("{}-{}", self.scenario, d),
            None => self.scenario.clone(),
        }
    }
}

/// Writes every result as JSON under `results_dir/<engine>/<stem>.json`, then
/// (re)builds `results_dir/summary.md` from **all** JSON on disk (both engines) so the
/// summary is complete regardless of which engine's `scenarios` bin ran last.
pub fn write_all(results_dir: impl AsRef<Path>, results: &[ScenarioResult]) -> io::Result<()> {
    let dir = results_dir.as_ref();
    for r in results {
        let path = dir.join(&r.engine).join(format!("{}.json", r.file_stem()));
        r.result.write_json(&path)?;
    }
    rebuild_summary(dir)
}

/// (Re)builds `summary.md` by scanning every `<engine>/*.json` under `results_dir`.
/// This means each engine's run contributes its rows and the table always reflects the
/// union on disk (not just the current process's results).
pub fn rebuild_summary(results_dir: impl AsRef<Path>) -> io::Result<()> {
    let dir = results_dir.as_ref();
    std::fs::create_dir_all(dir)?;

    let mut rows: Vec<SummaryRow> = Vec::new();
    for engine_entry in std::fs::read_dir(dir)? {
        let engine_entry = engine_entry?;
        if !engine_entry.file_type()?.is_dir() {
            continue;
        }
        for file in std::fs::read_dir(engine_entry.path())? {
            let path = file?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let text = std::fs::read_to_string(&path)?;
            if let Ok(result) = serde_json::from_str::<BenchResult>(&text) {
                rows.push(SummaryRow::from_result(&result));
            }
        }
    }
    // Stable ordering: engine, then scenario, then design.
    rows.sort_by(|a, b| {
        (a.engine.as_str(), a.scenario.as_str(), a.design.as_str()).cmp(&(
            b.engine.as_str(),
            b.scenario.as_str(),
            b.design.as_str(),
        ))
    });

    let mut md = String::new();
    md.push_str("# Sub-project C — recorded benchmark summary\n\n");
    md.push_str(
        "Machine-readable per-run JSON lives in `formualizer/` and `ironcalc/` \
         alongside this file. Regenerate with each engine crate's `scenarios` binary \
         (see `../findings.md`).\n\n",
    );
    md.push_str("| engine | scenario | design | p50 | p99 | max | verdict |\n");
    md.push_str("|--------|----------|--------|-----|-----|-----|---------|\n");
    for r in &rows {
        md.push_str(&r.row());
        md.push('\n');
    }
    std::fs::write(dir.join("summary.md"), md)
}

/// A `summary.md` row derived from a recorded [`BenchResult`] on disk.
struct SummaryRow {
    engine: String,
    scenario: String,
    design: String,
    p50: String,
    p99: String,
    max: String,
    verdict: &'static str,
}

impl SummaryRow {
    fn from_result(result: &BenchResult) -> Self {
        // engine/scenario/design are stored in the extra bag by ScenarioResult.
        let extra = &result.extra;
        let engine = extra
            .get("engine")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .to_string();
        let scenario = extra
            .get("scenario")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .to_string();
        let design = extra
            .get("design")
            .and_then(|v| v.as_str())
            .unwrap_or("—")
            .to_string();
        let (p50, p99, max) = match &result.stats {
            Some(s) => (
                bench_util::fmt_ns(s.p50_ns),
                bench_util::fmt_ns(s.p99_ns),
                bench_util::fmt_ns(s.max_ns),
            ),
            None => ("—".into(), "—".into(), "—".into()),
        };
        let verdict = match result.all_gates_pass() {
            Some(true) => "PASS",
            Some(false) => "FAIL",
            None => "—",
        };
        Self {
            engine,
            scenario,
            design,
            p50,
            p99,
            max,
            verdict,
        }
    }

    fn row(&self) -> String {
        format!(
            "| {} | {} | {} | {} | {} | {} | {} |",
            self.engine, self.scenario, self.design, self.p50, self.p99, self.max, self.verdict,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bench_util::Verdict;
    use std::time::Duration;

    fn stats() -> LatencyStats {
        let ds: Vec<Duration> = (1..=10).map(Duration::from_micros).collect();
        LatencyStats::from_durations(&ds).unwrap()
    }

    #[test]
    fn scenario_result_builds_name_and_gate() {
        let env = Environment::detect("abc");
        let gate = GateResult::max("read", 5_000, 2_000_000);
        let sr = ScenarioResult::from_stats(
            "formualizer",
            "scrolling-read",
            Some("D2"),
            1_000,
            "2026-07-01",
            env,
            Some(stats()),
            Some(gate.clone()),
            json!({}),
        );
        assert_eq!(sr.result.name, "formualizer/scrolling-read/D2");
        assert_eq!(sr.result.gates[0].verdict, Verdict::Pass);
        assert!(sr.result.to_json_pretty().contains("formualizer"));
    }

    #[test]
    fn write_all_creates_json_and_summary() {
        let dir = std::env::temp_dir().join(format!("bind_report_{}", std::process::id()));
        let env = Environment::detect("c");
        let results = vec![
            ScenarioResult::from_stats(
                "formualizer",
                "scrolling-read",
                Some("D1"),
                1,
                "d",
                env.clone(),
                Some(stats()),
                Some(GateResult::max("r", 5_000, 2_000_000)),
                json!({}),
            ),
            ScenarioResult::from_stats(
                "ironcalc",
                "memory",
                None,
                1,
                "d",
                env,
                None,
                None,
                json!({ "peak_rss_bytes": 123 }),
            ),
        ];
        write_all(&dir, &results).unwrap();

        let j = dir.join("formualizer/scrolling-read-D1.json");
        assert!(j.exists());
        let summary = std::fs::read_to_string(dir.join("summary.md")).unwrap();
        assert!(summary.contains("formualizer"));
        assert!(summary.contains("scrolling-read"));
        assert!(summary.contains("ironcalc"));
        assert!(summary.contains("PASS"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
