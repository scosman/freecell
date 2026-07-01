//! Pass/fail gating of a measured latency against a target (architecture §3,
//! functional_spec §5.4). Each benchmark asserts a measured number against its
//! target and prints `PASS`/`FAIL` with the number.

use serde::{Deserialize, Serialize};

/// The outcome of comparing a measured value against a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Verdict {
    Pass,
    Fail,
}

impl Verdict {
    /// The `"PASS"` / `"FAIL"` label used in printed output.
    pub fn label(self) -> &'static str {
        match self {
            Verdict::Pass => "PASS",
            Verdict::Fail => "FAIL",
        }
    }

    /// True only for [`Verdict::Pass`].
    pub fn is_pass(self) -> bool {
        matches!(self, Verdict::Pass)
    }
}

/// A single gated measurement: a name, the measured and target values (in
/// nanoseconds), and the resulting [`Verdict`]. Serializable for recording.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateResult {
    pub name: String,
    pub measured_ns: u64,
    pub target_ns: u64,
    pub verdict: Verdict,
}

impl GateResult {
    /// Gates a "must not exceed" latency target: `measured_ns <= target_ns` passes.
    /// This is the common case for the §5.4 latency budgets (frame time, cell-load,
    /// recalc).
    pub fn max(name: impl Into<String>, measured_ns: u64, target_ns: u64) -> Self {
        let verdict = if measured_ns <= target_ns {
            Verdict::Pass
        } else {
            Verdict::Fail
        };
        Self {
            name: name.into(),
            measured_ns,
            target_ns,
            verdict,
        }
    }

    /// Convenience: `true` if this gate passed.
    pub fn is_pass(&self) -> bool {
        self.verdict.is_pass()
    }

    /// A one-line human-readable summary, e.g.
    /// `PASS scrolling-read: measured 1.20 ms <= target 2.00 ms`.
    pub fn summary(&self) -> String {
        let cmp = if self.verdict.is_pass() { "<=" } else { ">" };
        format!(
            "{} {}: measured {} {} target {}",
            self.verdict.label(),
            self.name,
            fmt_ns(self.measured_ns),
            cmp,
            fmt_ns(self.target_ns),
        )
    }

    /// Prints [`GateResult::summary`] to stdout.
    pub fn print(&self) {
        println!("{}", self.summary());
    }
}

/// Formats a nanosecond count with a human-friendly unit (ns / µs / ms / s).
pub fn fmt_ns(ns: u64) -> String {
    const US: u64 = 1_000;
    const MS: u64 = 1_000_000;
    const S: u64 = 1_000_000_000;
    if ns < US {
        format!("{ns} ns")
    } else if ns < MS {
        format!("{:.2} µs", ns as f64 / US as f64)
    } else if ns < S {
        format!("{:.2} ms", ns as f64 / MS as f64)
    } else {
        format!("{:.2} s", ns as f64 / S as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_pass_and_fail() {
        let pass = GateResult::max("read", 1_500_000, 2_000_000);
        assert!(pass.is_pass());
        assert_eq!(pass.verdict, Verdict::Pass);
        assert_eq!(pass.measured_ns, 1_500_000);
        assert_eq!(pass.target_ns, 2_000_000);

        let fail = GateResult::max("read", 3_000_000, 2_000_000);
        assert!(!fail.is_pass());
        assert_eq!(fail.verdict, Verdict::Fail);
    }

    #[test]
    fn gate_boundary_is_pass() {
        // Exactly at target passes (<=).
        let g = GateResult::max("edge", 100, 100);
        assert!(g.is_pass());
    }

    #[test]
    fn summary_contains_verdict_and_units() {
        let g = GateResult::max("scrolling-read", 1_200_000, 2_000_000);
        let s = g.summary();
        assert!(s.contains("PASS"));
        assert!(s.contains("scrolling-read"));
        assert!(s.contains("ms"));

        let f = GateResult::max("recalc", 150_000_000, 100_000_000);
        assert!(f.summary().contains("FAIL"));
    }

    #[test]
    fn fmt_ns_units() {
        assert_eq!(fmt_ns(500), "500 ns");
        assert_eq!(fmt_ns(1_500), "1.50 µs");
        assert_eq!(fmt_ns(2_000_000), "2.00 ms");
        assert_eq!(fmt_ns(1_500_000_000), "1.50 s");
    }
}
