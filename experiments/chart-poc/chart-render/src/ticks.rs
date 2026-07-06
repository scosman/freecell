//! A "nice" numeric axis generator. gpui-component's `ScaleLinear` ships raw min/max →
//! pixel mapping with **no rounding and no tick generation** (`research/`), so the value
//! axis (title + readable tick labels at sensible intervals) is a piece FreeCell must own.
//!
//! This is the classic Heckbert "nice numbers" algorithm (*Graphics Gems*, 1990): pick a
//! step that is 1, 2, or 5 × a power of ten, then snap the domain outward to whole steps.
//! It is pure, gpui-free, and unit-tested — Phase 1's numeric value axis builds directly on
//! it.

/// A numeric axis domain snapped to "nice" round bounds with an even tick step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NiceScale {
    /// Lower bound, snapped down to a whole `step`.
    pub min: f64,
    /// Upper bound, snapped up to a whole `step`.
    pub max: f64,
    /// Spacing between ticks (a 1/2/5 × 10^k value).
    pub step: f64,
}

impl NiceScale {
    /// Compute a nice scale covering `[data_min, data_max]` with about `target_ticks`
    /// intervals. Degenerate inputs (equal bounds, non-finite, `target_ticks < 1`) fall
    /// back to a sane unit scale so the axis never divides by zero or loops forever.
    pub fn new(data_min: f64, data_max: f64, target_ticks: usize) -> Self {
        let target = target_ticks.max(1) as f64;

        // Guard non-finite input.
        let (lo, hi) = if !data_min.is_finite() || !data_max.is_finite() {
            (0.0, 1.0)
        } else if data_min <= data_max {
            (data_min, data_max)
        } else {
            (data_max, data_min)
        };

        // Equal (or effectively equal) bounds: pad to a unit interval around the value.
        if (hi - lo).abs() < f64::EPSILON {
            let step = nice_num((lo.abs().max(1.0)) / target, true).max(f64::MIN_POSITIVE);
            let min = (lo / step).floor() * step;
            return Self {
                min,
                max: min + step * target,
                step,
            };
        }

        let range = nice_num(hi - lo, false);
        let step = nice_num(range / target, true);
        let min = (lo / step).floor() * step;
        let max = (hi / step).ceil() * step;
        Self { min, max, step }
    }

    /// A nice scale for a value axis that starts at zero (Excel bars/columns start at 0):
    /// the domain always includes 0, extended to the data's max (or min, if all negative).
    pub fn for_values(values: impl IntoIterator<Item = f64>, target_ticks: usize) -> Self {
        let mut lo = 0.0_f64;
        let mut hi = 0.0_f64;
        let mut any = false;
        for v in values {
            if !v.is_finite() {
                continue;
            }
            if !any {
                lo = v.min(0.0);
                hi = v.max(0.0);
                any = true;
            } else {
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
        if !any {
            return Self::new(0.0, 1.0, target_ticks);
        }
        Self::new(lo, hi, target_ticks)
    }

    /// The tick positions from `min` to `max` inclusive, spaced by `step`.
    pub fn ticks(&self) -> Vec<f64> {
        let mut ticks = Vec::new();
        if self.step <= 0.0 || !self.step.is_finite() {
            return ticks;
        }
        let count = ((self.max - self.min) / self.step).round() as i64;
        for i in 0..=count.max(0) {
            // Multiply from `min` (not repeated addition) to avoid float drift, then snap
            // values that are within rounding distance of zero to exactly 0.0.
            let mut t = self.min + self.step * i as f64;
            if t.abs() < self.step * 1e-9 {
                t = 0.0;
            }
            ticks.push(t);
        }
        ticks
    }

    /// Fraction (0..=1) of the way from `min` to `max` for `value` — the position a
    /// renderer maps onto the pixel range of the axis.
    pub fn fraction(&self, value: f64) -> f64 {
        if (self.max - self.min).abs() < f64::EPSILON {
            0.0
        } else {
            (value - self.min) / (self.max - self.min)
        }
    }
}

/// Round `x` to a "nice" number. When `round`, pick the nearest of {1,2,5,10}×10^k;
/// otherwise pick the smallest such number ≥ `x` (used to nice-ify the raw range).
fn nice_num(x: f64, round: bool) -> f64 {
    if x <= 0.0 || !x.is_finite() {
        return 1.0;
    }
    let exp = x.log10().floor();
    let frac = x / 10f64.powf(exp);
    let nice_frac = if round {
        if frac < 1.5 {
            1.0
        } else if frac < 3.0 {
            2.0
        } else if frac < 7.0 {
            5.0
        } else {
            10.0
        }
    } else if frac <= 1.0 {
        1.0
    } else if frac <= 2.0 {
        2.0
    } else if frac <= 5.0 {
        5.0
    } else {
        10.0
    };
    nice_frac * 10f64.powf(exp)
}

/// Format a tick value for an axis label: integers without a decimal point, fractional
/// values trimmed of trailing zeros.
pub fn format_tick(value: f64) -> String {
    if value == 0.0 {
        return "0".to_string();
    }
    if value.fract() == 0.0 && value.abs() < 1e15 {
        return format!("{}", value as i64);
    }
    let mut s = format!("{value:.4}");
    while s.contains('.') && (s.ends_with('0') || s.ends_with('.')) {
        s.pop();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covers_data_with_round_step() {
        let s = NiceScale::new(0.0, 97.0, 5);
        assert_eq!(s.min, 0.0);
        assert_eq!(s.max, 100.0);
        assert_eq!(s.step, 20.0);
        assert_eq!(s.ticks(), vec![0.0, 20.0, 40.0, 60.0, 80.0, 100.0]);
    }

    #[test]
    fn ticks_span_the_domain_and_are_evenly_spaced() {
        let s = NiceScale::new(3.0, 11.0, 4);
        let ticks = s.ticks();
        assert!(ticks.len() >= 2, "expected several ticks, got {ticks:?}");
        assert_eq!(*ticks.first().unwrap(), s.min);
        assert_eq!(*ticks.last().unwrap(), s.max);
        for pair in ticks.windows(2) {
            assert!((pair[1] - pair[0] - s.step).abs() < 1e-9);
        }
        assert!(s.min <= 3.0 && s.max >= 11.0);
    }

    #[test]
    fn value_axis_includes_zero_baseline() {
        let s = NiceScale::for_values([120.0, 90.0, 150.0], 5);
        assert_eq!(s.min, 0.0);
        assert!(s.max >= 150.0);
        assert!(s.ticks().contains(&0.0));
    }

    #[test]
    fn all_negative_values_still_include_zero() {
        let s = NiceScale::for_values([-5.0, -20.0, -12.0], 5);
        assert_eq!(s.max, 0.0);
        assert!(s.min <= -20.0);
    }

    #[test]
    fn degenerate_inputs_do_not_panic_or_loop() {
        // Equal bounds.
        let s = NiceScale::new(42.0, 42.0, 5);
        assert!(s.step > 0.0);
        assert!(s.max > s.min);
        assert!(!s.ticks().is_empty());
        // Empty value set.
        let e = NiceScale::for_values([], 5);
        assert!(!e.ticks().is_empty());
        // Non-finite.
        let n = NiceScale::new(f64::NAN, f64::INFINITY, 5);
        assert!(n.step.is_finite() && n.step > 0.0);
    }

    #[test]
    fn fraction_maps_endpoints() {
        let s = NiceScale::new(0.0, 100.0, 5);
        assert!((s.fraction(0.0) - 0.0).abs() < 1e-9);
        assert!((s.fraction(100.0) - 1.0).abs() < 1e-9);
        assert!((s.fraction(50.0) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn tick_formatting_trims_zeros() {
        assert_eq!(format_tick(0.0), "0");
        assert_eq!(format_tick(20.0), "20");
        assert_eq!(format_tick(2.5), "2.5");
        assert_eq!(format_tick(-1000.0), "-1000");
    }
}
