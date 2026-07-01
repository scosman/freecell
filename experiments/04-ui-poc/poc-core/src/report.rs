//! Turn recorded [`FrameSample`]s into §5.4 pass/fail gates and a recorded
//! `bench_util::BenchResult` JSON in `results/` (architecture §7).
//!
//! Reuses the frozen `bench_util` helpers so the UI PoC's numbers are recorded in the
//! same shape as every engine sub-project: [`LatencyStats`] (p50/p99/max), [`GateResult`]
//! (PASS/FAIL vs a target), and a `BenchResult` stamped with environment + a
//! caller-supplied date (deterministic; no wall clock here).

use std::path::Path;

use bench_util::{BenchResult, Environment, GateResult, LatencyStats};
use serde_json::json;

use crate::config::{CELL_LOAD_TARGET_NS, FRAME_TARGET_NS, FRAME_WORST_NS};
use crate::harness::FrameSample;

/// The finalized outcome of a "Run Test" run: the gates, the two latency distributions,
/// and a printable human summary. The shell prints [`RunReport::summary`], writes the
/// JSON, and exits.
#[derive(Debug, Clone)]
pub struct RunReport {
    pub variant: String,
    pub frame_stats: LatencyStats,
    pub cell_load_stats: LatencyStats,
    pub gates: Vec<GateResult>,
    pub result: BenchResult,
}

impl RunReport {
    /// `true` iff every gate passed.
    pub fn passed(&self) -> bool {
        self.result.all_gates_pass().unwrap_or(false)
    }

    /// A multi-line human summary printed to the console after a run.
    pub fn summary(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("=== Run Test: {} ===\n", self.variant));
        out.push_str(&format!(
            "frames measured: {}\n",
            self.result.input_size
        ));
        out.push_str(&format!(
            "frame render  p50={} p99={} max={}\n",
            bench_util::fmt_ns(self.frame_stats.p50_ns),
            bench_util::fmt_ns(self.frame_stats.p99_ns),
            bench_util::fmt_ns(self.frame_stats.max_ns),
        ));
        out.push_str(&format!(
            "cell load     p50={} p99={} max={}\n",
            bench_util::fmt_ns(self.cell_load_stats.p50_ns),
            bench_util::fmt_ns(self.cell_load_stats.p99_ns),
            bench_util::fmt_ns(self.cell_load_stats.max_ns),
        ));
        for g in &self.gates {
            out.push_str(&g.summary());
            out.push('\n');
        }
        out.push_str(&format!(
            "VERDICT: {}\n",
            if self.passed() { "PASS" } else { "FAIL" }
        ));
        out
    }
}

/// Builds a [`RunReport`] from measured samples and gates it against the §5.4 targets.
///
/// - `variant`: `"raw-gpui"` or `"gpui-component"`.
/// - `date`: report date string, passed in by the shell (never read from a clock here).
/// - `commit`: source commit, passed in (deterministic recording).
///
/// Gates (functional_spec §5.4):
/// - `frame-p99` ≤ 8.33 ms (sustained 120 fps),
/// - `frame-max` ≤ 16.67 ms (never worse than 60 fps under fast scroll / jump),
/// - `cell-load-p99` ≤ 2 ms.
pub fn build_report(
    variant: &str,
    date: &str,
    commit: &str,
    samples: &[FrameSample],
) -> RunReport {
    let frame_ns: Vec<u64> = samples.iter().map(|s| s.frame_render_ns).collect();
    let cell_ns: Vec<u64> = samples.iter().map(|s| s.cell_load_ns).collect();

    let frame_stats = stats_or_empty(&frame_ns);
    let cell_load_stats = stats_or_empty(&cell_ns);

    let gates = vec![
        GateResult::max("frame-p99", frame_stats.p99_ns, FRAME_TARGET_NS),
        GateResult::max("frame-max", frame_stats.max_ns, FRAME_WORST_NS),
        GateResult::max("cell-load-p99", cell_load_stats.p99_ns, CELL_LOAD_TARGET_NS),
    ];

    let mut result = BenchResult::new(
        format!("ui-poc-{variant}-runtest"),
        samples.len() as u64,
        date,
        Environment::detect(commit),
    )
    .with_stats(frame_stats.clone())
    .with_extra(json!({
        "variant": variant,
        "frame_render": stats_json(&frame_stats),
        "cell_load": stats_json(&cell_load_stats),
        "frames": samples.len(),
        "targets_ns": {
            "frame_p99": FRAME_TARGET_NS,
            "frame_max": FRAME_WORST_NS,
            "cell_load_p99": CELL_LOAD_TARGET_NS,
        },
    }));
    for g in &gates {
        result = result.with_gate(g.clone());
    }

    RunReport {
        variant: variant.to_string(),
        frame_stats,
        cell_load_stats,
        gates,
        result,
    }
}

/// Builds the report and writes its JSON to `out_dir/<variant>-runtest.json`, returning
/// the report (already printable) and the path written.
pub fn finalize(
    variant: &str,
    date: &str,
    commit: &str,
    samples: &[FrameSample],
    out_dir: impl AsRef<Path>,
) -> std::io::Result<(RunReport, std::path::PathBuf)> {
    let report = build_report(variant, date, commit, samples);
    let path = out_dir
        .as_ref()
        .join(format!("{variant}-runtest.json"));
    report.result.write_json(&path)?;
    Ok((report, path))
}

fn stats_or_empty(samples_ns: &[u64]) -> LatencyStats {
    let mut sorted = samples_ns.to_vec();
    sorted.sort_unstable();
    LatencyStats::from_sorted_ns(&sorted).unwrap_or(LatencyStats {
        count: 0,
        min_ns: 0,
        max_ns: 0,
        mean_ns: 0,
        p50_ns: 0,
        p99_ns: 0,
    })
}

fn stats_json(s: &LatencyStats) -> serde_json::Value {
    json!({
        "min_ns": s.min_ns,
        "max_ns": s.max_ns,
        "mean_ns": s.mean_ns,
        "p50_ns": s.p50_ns,
        "p99_ns": s.p99_ns,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples_at(frame_ns: u64, cell_ns: u64, n: usize) -> Vec<FrameSample> {
        (0..n)
            .map(|_| FrameSample {
                frame_render_ns: frame_ns,
                cell_load_ns: cell_ns,
                newly_visible: 5,
            })
            .collect()
    }

    #[test]
    fn all_gates_pass_under_target() {
        // 5 ms frame, 0.5 ms cell-load: comfortably under every target.
        let samples = samples_at(5_000_000, 500_000, 200);
        let report = build_report("raw-gpui", "2026-07-01", "deadbeef", &samples);
        assert!(report.passed(), "under-target run should PASS: {}", report.summary());
        assert!(report.gates.iter().all(|g| g.is_pass()));
    }

    #[test]
    fn frame_gate_fails_over_120fps_budget_but_within_60fps() {
        // 12 ms frame: over the 8.33 ms (120 fps) p99 gate, but under the 16.67 ms
        // (60 fps) worst-case gate. So frame-p99 FAILs, frame-max PASSes.
        let samples = samples_at(12_000_000, 500_000, 200);
        let report = build_report("raw-gpui", "2026-07-01", "deadbeef", &samples);
        let p99 = report.gates.iter().find(|g| g.name == "frame-p99").unwrap();
        let max = report.gates.iter().find(|g| g.name == "frame-max").unwrap();
        assert!(!p99.is_pass(), "frame-p99 should fail at 12 ms");
        assert!(max.is_pass(), "frame-max should pass at 12 ms (< 16.67 ms)");
        assert!(!report.passed());
    }

    #[test]
    fn cell_load_gate_fails_over_2ms() {
        let samples = samples_at(5_000_000, 3_000_000, 200);
        let report = build_report("gpui-component", "2026-07-01", "deadbeef", &samples);
        let cell = report
            .gates
            .iter()
            .find(|g| g.name == "cell-load-p99")
            .unwrap();
        assert!(!cell.is_pass(), "cell-load-p99 should fail at 3 ms");
        assert!(!report.passed());
    }

    #[test]
    fn finalize_writes_wellformed_json() {
        let dir = std::env::temp_dir().join(format!(
            "poc-core-report-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let samples = samples_at(4_000_000, 400_000, 50);
        let (report, path) =
            finalize("raw-gpui", "2026-07-01", "cafef00d", &samples, &dir).unwrap();
        assert!(path.exists());
        let text = std::fs::read_to_string(&path).unwrap();
        // Round-trips through bench_util's serde and carries the gates + variant.
        let parsed: BenchResult = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed.name, "ui-poc-raw-gpui-runtest");
        assert_eq!(parsed.gates.len(), 3);
        assert!(text.contains("PASS"));
        assert_eq!(parsed.input_size, 50);
        assert!(report.passed());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn empty_samples_do_not_panic() {
        let report = build_report("raw-gpui", "2026-07-01", "deadbeef", &[]);
        // No frames → zeroed stats → gates trivially pass (0 <= target); documents that
        // an empty run is not a meaningful PASS. The shell always runs the full script.
        assert_eq!(report.frame_stats.p99_ns, 0);
        assert_eq!(report.result.input_size, 0);
    }
}
