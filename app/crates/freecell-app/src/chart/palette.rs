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

// The HSL helpers are `freecell-chart-model`'s — this file used to carry a byte-identical second
// copy (unit F3a found them provably equivalent), and architecture §6 prescribed deleting the copy
// rather than pinning it with a guard test. Importing costs nothing: `freecell-app` already depends
// on `freecell-chart-model`.
use freecell_chart_model::{hsl_to_rgb, rgb_to_hsl, Color};

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

    /// **F3a — the colour the cycle actually hands the renderer, pinned.**
    ///
    /// This file used to carry its own `rgb_to_hsl` / `hsl_to_rgb`, and the architecture review
    /// flagged the two copies as *drifted*: this one wrote `((g - b) / d) % 6.0` where chart-model
    /// writes `.rem_euclid(6.0)`, read as "they disagree on negative hues". **They did not**, and
    /// not merely on the inputs a test happened to sweep: `rgb_to_hsl` normalises its result with
    /// `.rem_euclid(360.0)`, so it can only ever return `h ∈ [0, 360)`; `hsl_to_rgb`'s `hp = h/60`
    /// is therefore never negative, and `%` and `rem_euclid` are identical on non-negative
    /// dividends. The equivalence held **by construction**, for every input, not just for the eight
    /// laps a sweep could enumerate.
    ///
    /// So the copy is gone (architecture §6: export from `chart-model`, delete the app copy) and
    /// there is nothing left to pin an agreement *between*. What is worth pinning is the thing this
    /// module owes the renderer: the concrete colour per series index. If the shared helpers, the
    /// lap rotation, or `BASE` change, these values change and this test says so — which is
    /// strictly more than the old equivalence test caught, since that one passed happily however
    /// both copies moved as long as they moved together.
    #[test]
    fn series_color_is_pinned_for_the_first_three_laps() {
        // Lap 0 is `BASE` verbatim; laps 1 and 2 rotate hue by 137° and 274°.
        const EXPECTED: [(usize, u32); 15] = [
            (0, 0x4E79A7),
            (1, 0xF28E2B),
            (2, 0x59A14F),
            (3, 0xE15759),
            (4, 0xB07AA1),
            (5, 0xA74E60),
            (6, 0x2BF2C6),
            (7, 0x5C4FA1),
            (8, 0x57E17C),
            (9, 0x92B07A),
            (10, 0x4EA755),
            (11, 0xF22BE5),
            (12, 0xA1734F),
            (13, 0xA357E1),
            (14, 0x7A82B0),
        ];
        for (index, hex) in EXPECTED {
            assert_eq!(
                series_color(index),
                Color::from_hex(hex),
                "series {index} moved to #{:06X} — the multi-series cycle changed colour, which \
                 moves chart pixels",
                series_color(index).to_hex(),
            );
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
