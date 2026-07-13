//! Selection-statistics aggregate + its compact readout formatting (`functional_spec.md §1`,
//! `architecture.md §1`).
//!
//! Pure and engine-free so the aggregate crosses the worker seam (`WorkerEvent::SelectionStats`)
//! and both the aggregate and its formatter are unit-testable in isolation. The worker computes a
//! [`SelectionStats`] over the selection ∩ the sheet's populated cells (so a full-column selection
//! is correct past the published viewport); the chrome formats it with [`format_stat_value`] /
//! [`format_stat_count`].

/// Aggregate statistics over a selection ∩ the sheet's used/populated range.
///
/// Excel semantics (`functional_spec.md §1`): [`count`](Self::count) is every **non-empty** cell
/// (text, numbers, booleans, errors — blanks excluded); the sum / average / min / max population is
/// the **numeric** cells only ([`numeric_count`](Self::numeric_count)). Text, blanks, booleans, and
/// errors are excluded from the math — D1.1: an error still counts toward `count` (text and errors
/// are treated identically here, both merely "non-numeric, non-empty").
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SelectionStats {
    /// Non-empty cells in the selection — the "Count" readout.
    pub count: u64,
    /// Numeric cells — the population summed/averaged and the min/max domain.
    pub numeric_count: u64,
    /// Sum of the numeric cells (`0.0` when there are none).
    pub sum: f64,
    /// Smallest numeric value, or `None` when no numeric cell is selected.
    pub min: Option<f64>,
    /// Largest numeric value, or `None` when no numeric cell is selected.
    pub max: Option<f64>,
}

impl SelectionStats {
    /// The empty aggregate — the starting accumulator and the "nothing populated" reply.
    pub const EMPTY: Self = Self {
        count: 0,
        numeric_count: 0,
        sum: 0.0,
        min: None,
        max: None,
    };

    /// Fold one numeric value into the aggregate: it counts, joins the numeric population, and
    /// updates the running sum / min / max.
    pub fn push_number(&mut self, n: f64) {
        self.count += 1;
        self.numeric_count += 1;
        self.sum += n;
        self.min = Some(self.min.map_or(n, |m| m.min(n)));
        self.max = Some(self.max.map_or(n, |m| m.max(n)));
    }

    /// Fold one non-empty, non-numeric value (text / boolean / error): it counts toward `count`
    /// only, and is excluded from the math (D1.1).
    pub fn push_non_numeric(&mut self) {
        self.count += 1;
    }

    /// The arithmetic mean of the numeric cells, or `None` when there are none.
    pub fn average(&self) -> Option<f64> {
        (self.numeric_count > 0).then(|| self.sum / self.numeric_count as f64)
    }

    /// Whether the readout should show the numeric stats (Sum / Average / Min / Max) — i.e. at
    /// least one numeric cell is selected (`functional_spec.md §1` "when shown").
    pub fn has_numeric(&self) -> bool {
        self.numeric_count > 0
    }
}

/// Decimal places the readout shows for a value of the given magnitude (D9.1, `functional_spec.md
/// §9A`): the larger the integer part, the fewer decimals, so a big Sum reads cleanly (`1,000,000.67`,
/// not `1000000.666667`) while a sub-1 value keeps resolution. Scaled by `|value|`, so negatives
/// scale identically. The fixed per-tier decimal count also caps float noise (a `.5`-decimal render
/// of `0.1 + 0.2` trims back to `0.3`).
fn decimals_for_magnitude(abs: f64) -> usize {
    if abs >= 100.0 {
        2
    } else if abs >= 10.0 {
        3
    } else if abs >= 1.0 {
        4
    } else {
        5
    }
}

/// Format an aggregate value (Sum / Average / Min / Max) as a compact, General-style string,
/// independent of any cell's own number format (`functional_spec.md §1` "Readout number
/// formatting"): thousands separators on the integer part, trailing zeros trimmed, and the decimal
/// precision scaled to the value's magnitude via [`decimals_for_magnitude`] (D9.1). A non-finite
/// input (never produced by a real sum, but guarded) renders its plain form rather than a grouped
/// one.
pub fn format_stat_value(value: f64) -> String {
    if !value.is_finite() {
        return value.to_string();
    }
    if value == 0.0 {
        return "0".to_string();
    }
    let negative = value < 0.0;
    let abs = value.abs();
    let decimals = decimals_for_magnitude(abs);
    let rendered = format!("{abs:.decimals$}");
    let trimmed = trim_trailing_zeros(&rendered);
    let grouped = group_thousands(trimmed);
    // A magnitude that rounds down to zero (e.g. a sub-ULP sum) must never render as "-0".
    if grouped == "0" || !negative {
        grouped
    } else {
        format!("-{grouped}")
    }
}

/// Format the Count readout — a non-negative integer with thousands separators (`1000000` →
/// `"1,000,000"`).
pub fn format_stat_count(count: u64) -> String {
    group_thousands(&count.to_string())
}

/// Trim a fixed-decimal rendering's trailing zeros (and a now-bare decimal point). Only touches
/// strings that carry a `.`, so an integer rendering is returned unchanged.
fn trim_trailing_zeros(s: &str) -> &str {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.')
    } else {
        s
    }
}

/// Insert thousands separators into the integer part of a **non-negative** numeric string (any
/// sign is handled by the caller). A fractional part after `.` is passed through untouched.
fn group_thousands(s: &str) -> String {
    let (int_part, frac_part) = match s.split_once('.') {
        Some((int, frac)) => (int, Some(frac)),
        None => (s, None),
    };
    let bytes = int_part.as_bytes();
    let len = bytes.len();
    let mut grouped = String::with_capacity(len + len / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(*b as char);
    }
    match frac_part {
        Some(frac) => format!("{grouped}.{frac}"),
        None => grouped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_number_tracks_count_sum_min_max() {
        let mut stats = SelectionStats::EMPTY;
        stats.push_number(3.0);
        stats.push_number(-1.0);
        stats.push_number(10.0);
        assert_eq!(stats.count, 3);
        assert_eq!(stats.numeric_count, 3);
        assert_eq!(stats.sum, 12.0);
        assert_eq!(stats.min, Some(-1.0));
        assert_eq!(stats.max, Some(10.0));
        assert_eq!(stats.average(), Some(4.0));
        assert!(stats.has_numeric());
    }

    #[test]
    fn non_numeric_cells_count_but_not_math() {
        let mut stats = SelectionStats::EMPTY;
        stats.push_number(5.0);
        stats.push_non_numeric(); // text
        stats.push_non_numeric(); // error / boolean
        assert_eq!(stats.count, 3, "every non-empty cell counts");
        assert_eq!(stats.numeric_count, 1, "only the number is numeric");
        assert_eq!(stats.sum, 5.0);
        assert_eq!(stats.average(), Some(5.0));
    }

    #[test]
    fn empty_and_all_text_have_no_numeric() {
        assert!(!SelectionStats::EMPTY.has_numeric());
        assert_eq!(SelectionStats::EMPTY.average(), None);
        let mut all_text = SelectionStats::EMPTY;
        all_text.push_non_numeric();
        all_text.push_non_numeric();
        assert_eq!(all_text.count, 2);
        assert!(!all_text.has_numeric(), "no numeric cell → stats hidden");
        assert_eq!(all_text.min, None);
        assert_eq!(all_text.max, None);
    }

    #[test]
    fn format_value_groups_and_trims() {
        assert_eq!(format_stat_value(1234.5), "1,234.5");
        assert_eq!(format_stat_value(246.9), "246.9");
        assert_eq!(format_stat_value(1_000_000.0), "1,000,000");
        // |v| >= 100 → 2 decimals (D9.1), so the fractional tail rounds to 2 places.
        assert_eq!(format_stat_value(1_234_567.891), "1,234,567.89");
        // 1234.50 → trailing zero trimmed.
        assert_eq!(format_stat_value(1234.50), "1,234.5");
    }

    #[test]
    fn format_value_adaptive_decimals_by_tier() {
        // Tier boundaries (D9.1): |v| >= 100 → 2 dp, >= 10 → 3 dp, >= 1 → 4 dp, < 1 → 5 dp.
        // Just below each boundary keeps the finer precision; at/above it drops one place.
        assert_eq!(format_stat_value(0.9999), "0.9999"); // < 1 → 5 dp
        assert_eq!(format_stat_value(1.0), "1"); // >= 1 → 4 dp, trimmed
        assert_eq!(format_stat_value(9.999), "9.999"); // >= 1 → 4 dp
        assert_eq!(format_stat_value(10.0), "10"); // >= 10 → 3 dp, trimmed
        assert_eq!(format_stat_value(99.99), "99.99"); // >= 10 → 3 dp
        assert_eq!(format_stat_value(100.0), "100"); // >= 100 → 2 dp, trimmed
                                                     // The owner's motivating example: a big sum no longer renders absurd precision.
        assert_eq!(format_stat_value(1_000_000.6666), "1,000,000.67");
        // A sub-1 magnitude retains 5 decimals of resolution.
        assert_eq!(format_stat_value(0.123456), "0.12346");
        // Negatives scale by |value| identically.
        assert_eq!(format_stat_value(-1234.567), "-1,234.57"); // |v| >= 100 → 2 dp
        assert_eq!(format_stat_value(-0.5), "-0.5"); // < 1 → 5 dp, trimmed
    }

    #[test]
    fn format_value_caps_float_noise() {
        // The classic float-representation surprise must not leak into the readout.
        assert_eq!(format_stat_value(0.1 + 0.2), "0.3");
        // A repeating decimal is capped, not expanded to the full f64 width.
        let third = format_stat_value(1.0 / 3.0);
        assert!(
            third.starts_with("0.333") && third.len() <= 12,
            "1/3 should render a bounded, capped decimal, got {third}"
        );
    }

    #[test]
    fn format_value_handles_sign_and_zero() {
        assert_eq!(format_stat_value(0.0), "0");
        // |v| >= 100 → 2 decimals (D9.1).
        assert_eq!(format_stat_value(-1234.567), "-1,234.57");
        assert_eq!(format_stat_value(-5.0), "-5");
        // A sub-ULP magnitude that rounds to zero must not render "-0".
        assert_eq!(format_stat_value(-1e-15), "0");
    }

    #[test]
    fn format_count_groups_thousands() {
        assert_eq!(format_stat_count(0), "0");
        assert_eq!(format_stat_count(5), "5");
        assert_eq!(format_stat_count(1234), "1,234");
        assert_eq!(format_stat_count(1_000_000), "1,000,000");
        assert_eq!(format_stat_count(12_345_678), "12,345,678");
    }
}
