//! Machine-readable results recording (architecture §3): each engine phase writes a
//! JSON [`BenchResult`] to its `results/`, **stamped** with environment (CPU/OS/
//! commit), input size, and a **relative date passed in** — deterministic code
//! never calls a wall clock (architecture §3).

use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::gate::GateResult;
use crate::stats::LatencyStats;

/// The environment a benchmark ran in. Everything except `commit` is auto-detected
/// from `std`; `commit` is passed in because deterministic code must not shell out
/// to `git`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Environment {
    /// A CPU description (best-effort; empty if unknown). Callers may override with
    /// a richer string via [`Environment::with_cpu`].
    pub cpu: String,
    /// OS identifier, e.g. `"linux"`, `"macos"` (from `std::env::consts::OS`).
    pub os: String,
    /// Architecture, e.g. `"x86_64"`, `"aarch64"` (from `std::env::consts::ARCH`).
    pub arch: String,
    /// Logical core count (from `std::thread::available_parallelism`; `0` if
    /// unavailable).
    pub cores: u32,
    /// The source commit the benchmark ran against — **passed in** by the caller.
    pub commit: String,
}

impl Environment {
    /// Detects OS, arch, and core count from `std`, and stamps the provided
    /// `commit`. Deliberately takes **no date**: the report date lives on
    /// [`BenchResult`] and is passed in separately (architecture §3).
    pub fn detect(commit: impl Into<String>) -> Self {
        let cores = std::thread::available_parallelism()
            .map(|n| n.get() as u32)
            .unwrap_or(0);
        Self {
            cpu: String::new(),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            cores,
            commit: commit.into(),
        }
    }

    /// Sets a human-readable CPU description (e.g. parsed from `/proc/cpuinfo` or
    /// `sysctl` by the caller — kept out of this crate to avoid platform code).
    pub fn with_cpu(mut self, cpu: impl Into<String>) -> Self {
        self.cpu = cpu.into();
        self
    }
}

/// A recorded benchmark result: identity, input size, the report date, the
/// environment, optional latency [`LatencyStats`], any pass/fail [`GateResult`]s,
/// and an open `extra` bag for benchmark-specific fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchResult {
    /// Benchmark name/identifier.
    pub name: String,
    /// Input size (e.g. cell count) this run measured.
    pub input_size: u64,
    /// A relative date string **passed in** by the caller (e.g. `"2026-06-30"`).
    /// This crate never reads a wall clock, keeping recording deterministic.
    pub date: String,
    pub environment: Environment,
    /// Latency distribution, if the benchmark produced one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<LatencyStats>,
    /// Any pass/fail gates evaluated for this run.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gates: Vec<GateResult>,
    /// Arbitrary extra fields (design name, secondary metrics, memory RSS, ...).
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub extra: Value,
}

impl BenchResult {
    /// Creates a result with no stats/gates/extra yet.
    pub fn new(
        name: impl Into<String>,
        input_size: u64,
        date: impl Into<String>,
        environment: Environment,
    ) -> Self {
        Self {
            name: name.into(),
            input_size,
            date: date.into(),
            environment,
            stats: None,
            gates: Vec::new(),
            extra: Value::Null,
        }
    }

    /// Attaches latency stats (builder style).
    pub fn with_stats(mut self, stats: LatencyStats) -> Self {
        self.stats = Some(stats);
        self
    }

    /// Adds a single gate result (builder style).
    pub fn with_gate(mut self, gate: GateResult) -> Self {
        self.gates.push(gate);
        self
    }

    /// Attaches an arbitrary `extra` JSON payload (builder style).
    pub fn with_extra(mut self, extra: Value) -> Self {
        self.extra = extra;
        self
    }

    /// `true` if there is at least one gate and **every** gate passed. `false` if
    /// any gate failed. `None` if there are no gates (nothing to judge).
    pub fn all_gates_pass(&self) -> Option<bool> {
        if self.gates.is_empty() {
            None
        } else {
            Some(self.gates.iter().all(GateResult::is_pass))
        }
    }

    /// Serializes to pretty JSON.
    pub fn to_json_pretty(&self) -> String {
        // Our types are plain data and always serialize; unwrap is safe.
        serde_json::to_string_pretty(self).expect("BenchResult serializes")
    }

    /// Writes pretty JSON to `path`, creating parent directories if needed.
    pub fn write_json(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, self.to_json_pretty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gate::Verdict;

    fn sample_stats() -> LatencyStats {
        LatencyStats {
            count: 3,
            min_ns: 1,
            max_ns: 3,
            mean_ns: 2,
            p50_ns: 2,
            p99_ns: 3,
        }
    }

    #[test]
    fn environment_detect_fills_os_and_cores_and_no_date() {
        let env = Environment::detect("abc123");
        assert_eq!(env.commit, "abc123");
        assert!(!env.os.is_empty());
        assert!(!env.arch.is_empty());
        // Serialized environment carries no `date` field (date is on BenchResult).
        let json = serde_json::to_string(&env).unwrap();
        assert!(!json.contains("date"));
        assert!(json.contains("commit"));
    }

    #[test]
    fn environment_with_cpu() {
        let env = Environment::detect("c").with_cpu("Test CPU @ 3.0GHz");
        assert_eq!(env.cpu, "Test CPU @ 3.0GHz");
    }

    #[test]
    fn bench_result_json_roundtrip() {
        let env = Environment::detect("deadbeef").with_cpu("CPU X");
        let result = BenchResult::new("scrolling-read", 1_000_000, "2026-06-30", env)
            .with_stats(sample_stats())
            .with_gate(GateResult::max("read", 1_500_000, 2_000_000))
            .with_gate(GateResult::max("recalc", 150_000_000, 100_000_000))
            .with_extra(serde_json::json!({ "design": "D2-bulk" }));

        let json = result.to_json_pretty();
        assert!(json.contains("scrolling-read"));
        assert!(json.contains("2026-06-30"));
        assert!(json.contains("1000000"));
        assert!(json.contains("PASS"));
        assert!(json.contains("FAIL"));
        assert!(json.contains("D2-bulk"));

        let back: BenchResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back, result);
    }

    #[test]
    fn all_gates_pass_logic() {
        let env = Environment::detect("c");
        let none = BenchResult::new("n", 1, "d", env.clone());
        assert_eq!(none.all_gates_pass(), None);

        let passing = BenchResult::new("p", 1, "d", env.clone())
            .with_gate(GateResult::max("a", 1, 2))
            .with_gate(GateResult::max("b", 1, 2));
        assert_eq!(passing.all_gates_pass(), Some(true));

        let failing = BenchResult::new("f", 1, "d", env)
            .with_gate(GateResult::max("a", 1, 2))
            .with_gate(GateResult::max("b", 5, 2));
        assert_eq!(failing.all_gates_pass(), Some(false));
        assert_eq!(failing.gates[1].verdict, Verdict::Fail);
    }

    #[test]
    fn write_json_creates_file_and_parents() {
        let dir = std::env::temp_dir().join(format!("bench_util_test_{}", std::process::id()));
        let path = dir.join("nested/result.json");
        let env = Environment::detect("c");
        let result = BenchResult::new("write-test", 10, "2026-06-30", env);

        result.write_json(&path).unwrap();
        let read_back = std::fs::read_to_string(&path).unwrap();
        let parsed: BenchResult = serde_json::from_str(&read_back).unwrap();
        assert_eq!(parsed.name, "write-test");

        // Cleanup.
        let _ = std::fs::remove_dir_all(&dir);
    }
}
