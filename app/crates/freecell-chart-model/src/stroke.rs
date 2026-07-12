//! **Line stroke** — the OOXML `a:ln` on a series' `c:spPr` (charts/functional_spec §4 P2;
//! coverage-matrix §C `a:ln`, P13).
//!
//! A line series' visible line is styled by `<a:ln w="…"><a:solidFill>…</a:solidFill></a:ln>`: a
//! **width** (`@w`, in EMUs — 12700 per point), a **color** (its own `a:solidFill`, which may carry
//! an `a:alpha`), independent of the series' marker/fill `a:solidFill`. The PoC drew every line at a
//! single hard-coded width in the series/palette color; this models the real stroke so the renderer
//! can honor Excel's heavier default and any authored width/color/alpha.
//!
//! It is deliberately bounded to the three rendered fields (width, color, alpha) — a **plain solid**
//! line. The DrawingML line long tail — dash patterns, caps, joins, compound lines, gradient line
//! fills — is out of scope and preserved via the retained source, not modeled (architecture §3.1).
//! A non-solid variant we don't render (a preset/custom **dash**, a **compound** line) is therefore
//! not represented here; it is instead caught by the fidelity accessor (`unsupported_line_stroke`)
//! so the chart is honestly [`Degraded`](crate::Fidelity::Degraded) rather than drawn solid and
//! silently mislabeled (P13; functional_spec §5).

use crate::ChartColor;

/// EMUs per point — the unit `a:ln@w` is written in. 914400 EMU = 1 inch = 72 pt ⇒ 12700 EMU/pt.
const EMU_PER_POINT: f32 = 12_700.0;

/// A series line's stroke (`a:ln`): an optional **width** in points, an optional **color**
/// ([`ChartColor`] — explicit sRGB or a theme reference), and an optional **alpha** (opacity
/// fraction in `0..=1`). Every field is optional so the renderer falls back to its Excel-like
/// default width, the series/palette color, and full opacity when a field is absent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineStroke {
    /// `a:ln@w` converted to **points** (EMU ÷ 12700); `None` = the renderer's default weight.
    pub width_pt: Option<f32>,
    /// `a:ln/a:solidFill` color; `None` = fall back to the series color / palette cycle.
    pub color: Option<ChartColor>,
    /// `a:ln/a:solidFill/*/a:alpha` as a fraction in `0..=1` (the OOXML per-mille `val` ÷ 100000);
    /// `None` = fully opaque.
    pub alpha: Option<f32>,
}

impl LineStroke {
    /// An empty stroke — every field defaulted (the renderer picks its defaults).
    pub const fn new() -> Self {
        Self {
            width_pt: None,
            color: None,
            alpha: None,
        }
    }

    /// Convert an `a:ln@w` EMU width to points (12700 EMU per point).
    pub fn width_pt_from_emu(emu: i64) -> f32 {
        emu as f32 / EMU_PER_POINT
    }

    /// Set the stroke width in points (builder style).
    pub fn with_width_pt(mut self, width_pt: f32) -> Self {
        self.width_pt = Some(width_pt);
        self
    }

    /// Set the stroke width from an `a:ln@w` EMU value (builder style).
    pub fn with_width_emu(mut self, emu: i64) -> Self {
        self.width_pt = Some(Self::width_pt_from_emu(emu));
        self
    }

    /// Set the stroke color (builder style).
    pub fn with_color(mut self, color: impl Into<ChartColor>) -> Self {
        self.color = Some(color.into());
        self
    }

    /// Set the stroke alpha (opacity fraction in `0..=1`, builder style).
    pub fn with_alpha(mut self, alpha: f32) -> Self {
        self.alpha = Some(alpha);
        self
    }
}

impl Default for LineStroke {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Color, ThemeSlot};

    #[test]
    fn width_pt_from_emu_matches_excel_default() {
        // Excel's default line-series weight is `a:ln w="28440"` ≈ 2.24 pt.
        let pt = LineStroke::width_pt_from_emu(28_440);
        assert!((pt - 2.24).abs() < 0.01, "expected ~2.24pt, got {pt}");
        // A thin `w="9360"` (axis/gridline) ≈ 0.74 pt.
        assert!((LineStroke::width_pt_from_emu(9_360) - 0.737).abs() < 0.01);
        assert_eq!(LineStroke::width_pt_from_emu(0), 0.0);
    }

    #[test]
    fn builders_round_trip() {
        let s = LineStroke::new()
            .with_width_emu(28_440)
            .with_color(Color::from_hex(0x4A7EBB))
            .with_alpha(0.5);
        assert!((s.width_pt.unwrap() - 2.24).abs() < 0.01);
        assert_eq!(s.color, Some(ChartColor::Rgb(Color::from_hex(0x4A7EBB))));
        assert_eq!(s.alpha, Some(0.5));

        // A theme-referenced stroke color rides the same builder.
        let themed = LineStroke::new().with_color(ChartColor::theme(ThemeSlot::Accent2));
        assert_eq!(themed.color, Some(ChartColor::theme(ThemeSlot::Accent2)));
        assert_eq!(themed.width_pt, None, "absent width stays None");
        assert_eq!(themed.alpha, None, "absent alpha stays None");
    }

    #[test]
    fn default_is_empty() {
        assert_eq!(LineStroke::default(), LineStroke::new());
        let d = LineStroke::default();
        assert!(d.width_pt.is_none() && d.color.is_none() && d.alpha.is_none());
    }
}
