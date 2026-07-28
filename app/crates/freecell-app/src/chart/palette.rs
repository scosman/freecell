//! The multi-series color cycle. gpui-component's theme exposes exactly `chart_1..chart_5`
//! and does **not** auto-cycle them (every stock chart defaults to a single `chart_2` —
//! `research/gpui-component-charts.md`). FreeCell owns the cycle, and must extend past 5
//! series, so we keep our own palette here (gpui-free, so it is testable and shared).
//!
//! **These are NOT gpui-component's `chart_1..chart_5`.** Those (at the pinned rev,
//! `crates/ui/src/theme/default-theme.json`) are a *monochrome blue ramp*
//! (`#93c5fd`→`#1e40af`) — fine for a single series, useless for distinguishing several,
//! and they would fail the Gate 1 "distinct colors" rubric. So `BASE` below is a deliberately
//! chosen **categorical** palette (Tableau-10 style: distinct hues) — the right choice for a
//! multi-series cycle. Do NOT "simplify" this to `cx.theme().chart_N`.
//! Beyond five series we rotate hue so additional series stay visually distinct.

use freecell_chart_model::Color;

/// The five base **categorical** colors — a Tableau-10-style palette of distinct hues (see
/// the module docs for why we do NOT use gpui-component's monochrome-blue `chart_1..chart_5`).
pub const BASE: [Color; 5] = [
    Color::from_hex(0x4E79A7), // blue
    Color::from_hex(0xF28E2B), // orange
    Color::from_hex(0x59A14F), // green
    Color::from_hex(0xE15759), // red
    Color::from_hex(0xB07AA1), // purple
];

/// The color for pie/doughnut **slice** `index`. A pie is single-series, so its slices are
/// the categories; there is no auto-palette in gpui-component (an unset slice color paints a
/// monochrome disc), so we synthesize one from the same categorical cycle the multi-series
/// charts use — and the legend keys off the same function, so slice↔swatch match by
/// construction. Alias of [`series_color`] so the intent reads clearly at the call site.
pub fn slice_color(index: usize) -> Color {
    series_color(index)
}

/// The color for series `index`, cycling the five base colors and rotating hue for a
/// second/third lap so >5 series stay distinct rather than repeating exactly.
pub fn series_color(index: usize) -> Color {
    let base = BASE[index % BASE.len()];
    let lap = index / BASE.len();
    if lap == 0 {
        return base;
    }
    // Rotate hue by a fixed offset per lap so lap-2/3 colors differ from lap-1.
    let (h, s, l) = rgb_to_hsl(base);
    let h = (h + 137.0 * lap as f64) % 360.0;
    hsl_to_rgb(h, s, l)
}

fn rgb_to_hsl(c: Color) -> (f64, f64, f64) {
    let r = c.r as f64 / 255.0;
    let g = c.g as f64 / 255.0;
    let b = c.b as f64 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < 1e-9 {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        ((g - b) / d) % 6.0
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    let h = (h * 60.0).rem_euclid(360.0);
    (h, s, l)
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> Color {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match hp as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    Color::rgb(
        (((r1 + m) * 255.0).round()) as u8,
        (((g1 + m) * 255.0).round()) as u8,
        (((b1 + m) * 255.0).round()) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_five_are_the_base_palette() {
        for (i, base) in BASE.iter().enumerate() {
            assert_eq!(series_color(i), *base);
        }
    }

    #[test]
    fn beyond_five_stays_distinct_from_first_lap() {
        // Lap 2 wraps the index but rotates hue, so it must not equal the base color.
        for i in 0..BASE.len() {
            assert_ne!(
                series_color(i),
                series_color(i + BASE.len()),
                "series {i} and {} collided",
                i + BASE.len()
            );
        }
    }

    /// **F3a duplication guard.** `freecell-chart-model::theme` carries its own `rgb_to_hsl` /
    /// `hsl_to_rgb`, and the architecture review flagged the two copies as *drifted*: this file
    /// writes `((g - b) / d) % 6.0` where chart-model writes `.rem_euclid(6.0)`, which the review
    /// read as "they disagree on negative hues".
    ///
    /// **They do not.** Both functions normalise with `.rem_euclid(360.0)` on the way out, which
    /// maps the negative intermediate onto the identical hue — verified here over the whole
    /// reachable input space (every base colour, every lap), not just the round-trip.
    ///
    /// The duplication is real even though the divergence is not, so this test pins the agreement:
    /// if either copy is edited, it fails. Actually removing the duplicate means merging the crates
    /// (unit F3) — out of scope here, and not worth risking a chart-render baseline move for a
    /// difference that produces no different pixel.
    #[test]
    fn hsl_helpers_agree_with_the_chart_model_copy() {
        for base in BASE {
            let mine = rgb_to_hsl(base);
            let theirs = freecell_chart_model::rgb_to_hsl(base);
            assert_eq!(mine, theirs, "rgb_to_hsl drifted for {base:?}");

            // Every hue this palette actually produces: the base plus each lap's rotation.
            for lap in 0..8 {
                let h = (mine.0 + 137.0 * lap as f64) % 360.0;
                assert_eq!(
                    hsl_to_rgb(h, mine.1, mine.2),
                    freecell_chart_model::hsl_to_rgb(h, mine.1, mine.2),
                    "hsl_to_rgb drifted at hue {h} (base {base:?}, lap {lap})",
                );
            }
        }
    }

    #[test]
    fn hsl_round_trip_is_close() {
        for base in BASE {
            let (h, s, l) = rgb_to_hsl(base);
            let back = hsl_to_rgb(h, s, l);
            let dr = (base.r as i32 - back.r as i32).abs();
            let dg = (base.g as i32 - back.g as i32).abs();
            let db = (base.b as i32 - back.b as i32).abs();
            assert!(dr <= 2 && dg <= 2 && db <= 2, "{base:?} -> {back:?}");
        }
    }
}
