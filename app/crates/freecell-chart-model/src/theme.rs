//! **Theme colors** — the OOXML `a:schemeClr` color model (charts/functional_spec §4 P1,
//! architecture §3.1; coverage-matrix §C `a:schemeClr`).
//!
//! A chart series (or, later, data point) can reference a **theme slot** rather than an explicit
//! sRGB value — `<a:schemeClr val="accent1"><a:lumMod val="60000"/><a:lumOff val="40000"/></a:schemeClr>`.
//! Resolving it needs a [`ThemePalette`] (the workbook's `theme1.xml` slot → RGB map). The model
//! carries the **reference** ([`ChartColor::Theme`]) rather than a pre-resolved color so the seam
//! stays workbook-agnostic; the resolution happens where a palette is available — against
//! [`ThemePalette::office_default`] for the isolated render component (this phase, P6), and against
//! the real workbook theme once the engine threads it (P8).
//!
//! `lumMod`/`lumOff` (the "+tint" of functional_spec §4 P1) are applied as an HSL-luminance
//! transform (`L' = clamp(L·lumMod + lumOff)`) — a widely-used approximation of the OOXML tint,
//! not a gamma-exact match; a visual stand-in in the same spirit as the coverage matrix's other
//! "E-OK" color items.

use crate::Color;

/// A theme color slot — the `val` of `<a:schemeClr>` (and the workbook's `clrScheme`).
///
/// `dk1`/`lt1`/`dk2`/`lt2` are the scheme names; `tx1`/`bg1`/`tx2`/`bg2` are the chart/drawing
/// aliases for the same four slots (dark-1/light-1/dark-2/light-2), so [`ThemeSlot::from_ooxml`]
/// accepts either spelling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeSlot {
    /// `dk1` / `tx1` — the primary dark (text) color.
    Dark1,
    /// `lt1` / `bg1` — the primary light (background) color.
    Light1,
    /// `dk2` / `tx2` — the secondary dark color.
    Dark2,
    /// `lt2` / `bg2` — the secondary light color.
    Light2,
    Accent1,
    Accent2,
    Accent3,
    Accent4,
    Accent5,
    Accent6,
    /// `hlink` — hyperlink color.
    Hyperlink,
    /// `folHlink` — followed-hyperlink color.
    FollowedHyperlink,
}

impl ThemeSlot {
    /// Parse an OOXML `schemeClr`/`clrScheme` slot name (namespace-agnostic `val` text), accepting
    /// both the scheme spelling (`dk1`) and the drawing alias (`tx1`). Returns `None` for an
    /// unknown token (e.g. `phClr`, which is only meaningful inside a style part).
    pub fn from_ooxml(name: &str) -> Option<Self> {
        Some(match name {
            "dk1" | "tx1" => Self::Dark1,
            "lt1" | "bg1" => Self::Light1,
            "dk2" | "tx2" => Self::Dark2,
            "lt2" | "bg2" => Self::Light2,
            "accent1" => Self::Accent1,
            "accent2" => Self::Accent2,
            "accent3" => Self::Accent3,
            "accent4" => Self::Accent4,
            "accent5" => Self::Accent5,
            "accent6" => Self::Accent6,
            "hlink" => Self::Hyperlink,
            "folHlink" => Self::FollowedHyperlink,
            _ => return None,
        })
    }
}

/// A theme's twelve color slots resolved to concrete RGB — the workbook `theme1.xml` `clrScheme`,
/// or [`office_default`](ThemePalette::office_default) for a chart rendered without a workbook.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemePalette {
    pub dk1: Color,
    pub lt1: Color,
    pub dk2: Color,
    pub lt2: Color,
    pub accent1: Color,
    pub accent2: Color,
    pub accent3: Color,
    pub accent4: Color,
    pub accent5: Color,
    pub accent6: Color,
    pub hlink: Color,
    pub fol_hlink: Color,
}

impl ThemePalette {
    /// The default **Office** theme (Excel 2013+), the palette real files use unless they carry a
    /// custom `clrScheme`. These are the RGBs Excel maps `accent1..6` to for its default series
    /// colors, so a `schemeClr`-driven series resolves to the same color Excel shows.
    pub const fn office_default() -> Self {
        Self {
            dk1: Color::from_hex(0x000000),
            lt1: Color::from_hex(0xFFFFFF),
            dk2: Color::from_hex(0x44546A),
            lt2: Color::from_hex(0xE7E6E6),
            accent1: Color::from_hex(0x4472C4),
            accent2: Color::from_hex(0xED7D31),
            accent3: Color::from_hex(0xA5A5A5),
            accent4: Color::from_hex(0xFFC000),
            accent5: Color::from_hex(0x5B9BD5),
            accent6: Color::from_hex(0x70AD47),
            hlink: Color::from_hex(0x0563C1),
            fol_hlink: Color::from_hex(0x954F72),
        }
    }

    /// The concrete RGB for a theme slot.
    pub const fn color(&self, slot: ThemeSlot) -> Color {
        match slot {
            ThemeSlot::Dark1 => self.dk1,
            ThemeSlot::Light1 => self.lt1,
            ThemeSlot::Dark2 => self.dk2,
            ThemeSlot::Light2 => self.lt2,
            ThemeSlot::Accent1 => self.accent1,
            ThemeSlot::Accent2 => self.accent2,
            ThemeSlot::Accent3 => self.accent3,
            ThemeSlot::Accent4 => self.accent4,
            ThemeSlot::Accent5 => self.accent5,
            ThemeSlot::Accent6 => self.accent6,
            ThemeSlot::Hyperlink => self.hlink,
            ThemeSlot::FollowedHyperlink => self.fol_hlink,
        }
    }
}

/// A chart color reference — an explicit sRGB value (`a:srgbClr`) or a **theme** color
/// (`a:schemeClr`) with optional `lumMod`/`lumOff` tint, [resolved](ChartColor::resolve) against a
/// [`ThemePalette`]. This is what a [`Series`](crate::Series) carries for its color; the renderer
/// resolves it at paint time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ChartColor {
    /// An explicit sRGB color (`<a:srgbClr val="RRGGBB"/>`).
    Rgb(Color),
    /// A theme-slot reference (`<a:schemeClr val="accentN"/>`) with optional luminance tint.
    ///
    /// `lum_mod`/`lum_off` are the `<a:lumMod>`/`<a:lumOff>` values as **fractions** in `0..=1`
    /// (the OOXML per-mille `val` divided by 100000 — the engine parser does the division). `None`
    /// means the modifier is absent.
    Theme {
        slot: ThemeSlot,
        lum_mod: Option<f32>,
        lum_off: Option<f32>,
    },
}

impl ChartColor {
    /// A plain theme-slot color with no tint.
    pub const fn theme(slot: ThemeSlot) -> Self {
        Self::Theme {
            slot,
            lum_mod: None,
            lum_off: None,
        }
    }

    /// A tinted theme-slot color (`lumMod`/`lumOff` as fractions in `0..=1`).
    pub const fn theme_tinted(slot: ThemeSlot, lum_mod: f32, lum_off: f32) -> Self {
        Self::Theme {
            slot,
            lum_mod: Some(lum_mod),
            lum_off: Some(lum_off),
        }
    }

    /// Resolve this reference to a concrete RGB against `palette`. An [`Rgb`](ChartColor::Rgb) is
    /// itself; a [`Theme`](ChartColor::Theme) looks the slot up and applies the `lumMod`/`lumOff`
    /// tint as an HSL-luminance transform (see the module docs — a documented approximation).
    pub fn resolve(&self, palette: &ThemePalette) -> Color {
        match self {
            ChartColor::Rgb(c) => *c,
            ChartColor::Theme {
                slot,
                lum_mod,
                lum_off,
            } => {
                let base = palette.color(*slot);
                if lum_mod.is_none() && lum_off.is_none() {
                    return base;
                }
                let (h, s, l) = rgb_to_hsl(base);
                let l = (l * lum_mod.unwrap_or(1.0) as f64 + lum_off.unwrap_or(0.0) as f64)
                    .clamp(0.0, 1.0);
                hsl_to_rgb(h, s, l)
            }
        }
    }
}

impl From<Color> for ChartColor {
    fn from(c: Color) -> Self {
        ChartColor::Rgb(c)
    }
}

/// Convert an sRGB [`Color`] to `(hue°, saturation, luminance)` in HSL, all in `0..=1` except hue
/// in `0..360`.
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
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    ((h * 60.0).rem_euclid(360.0), s, l)
}

/// Inverse of [`rgb_to_hsl`].
fn hsl_to_rgb(h: f64, s: f64, l: f64) -> Color {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h / 60.0;
    let x = c * (1.0 - (hp.rem_euclid(2.0) - 1.0).abs());
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
        ((r1 + m) * 255.0).round() as u8,
        ((g1 + m) * 255.0).round() as u8,
        ((b1 + m) * 255.0).round() as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn office_default_has_the_known_accents() {
        let p = ThemePalette::office_default();
        assert_eq!(p.accent1, Color::from_hex(0x4472C4));
        assert_eq!(p.accent2, Color::from_hex(0xED7D31));
        assert_eq!(p.accent6, Color::from_hex(0x70AD47));
        assert_eq!(p.dk1, Color::from_hex(0x000000));
        assert_eq!(p.lt1, Color::from_hex(0xFFFFFF));
    }

    #[test]
    fn rgb_resolves_to_itself() {
        let c = ChartColor::Rgb(Color::from_hex(0x123456));
        assert_eq!(
            c.resolve(&ThemePalette::office_default()),
            Color::from_hex(0x123456)
        );
    }

    #[test]
    fn theme_slot_resolves_to_palette_color() {
        let p = ThemePalette::office_default();
        assert_eq!(ChartColor::theme(ThemeSlot::Accent1).resolve(&p), p.accent1);
        assert_eq!(ChartColor::theme(ThemeSlot::Accent3).resolve(&p), p.accent3);
    }

    #[test]
    fn from_color_wraps_as_rgb() {
        let c: ChartColor = Color::from_hex(0xABCDEF).into();
        assert_eq!(c, ChartColor::Rgb(Color::from_hex(0xABCDEF)));
    }

    #[test]
    fn lum_off_lightens_and_lum_mod_darkens() {
        let p = ThemePalette::office_default();
        let base = p.accent1;
        let (_, _, base_l) = rgb_to_hsl(base);

        // lumMod 0.5 with no offset halves luminance → darker.
        let darker = ChartColor::theme_tinted(ThemeSlot::Accent1, 0.5, 0.0).resolve(&p);
        let (_, _, dl) = rgb_to_hsl(darker);
        assert!(dl < base_l, "lumMod 0.5 should darken ({dl} !< {base_l})");
        assert!(
            (dl - base_l * 0.5).abs() < 0.02,
            "≈ half luminance, got {dl}"
        );

        // The Excel "lighter" tint (lumMod 0.6 + lumOff 0.4) → lighter.
        let lighter = ChartColor::theme_tinted(ThemeSlot::Accent1, 0.6, 0.4).resolve(&p);
        let (_, _, ll) = rgb_to_hsl(lighter);
        assert!(
            ll > base_l,
            "lumMod .6/lumOff .4 should lighten ({ll} !> {base_l})"
        );
    }

    #[test]
    fn from_ooxml_maps_slot_names_and_aliases() {
        assert_eq!(ThemeSlot::from_ooxml("dk1"), Some(ThemeSlot::Dark1));
        assert_eq!(ThemeSlot::from_ooxml("tx1"), Some(ThemeSlot::Dark1));
        assert_eq!(ThemeSlot::from_ooxml("bg2"), Some(ThemeSlot::Light2));
        assert_eq!(ThemeSlot::from_ooxml("accent3"), Some(ThemeSlot::Accent3));
        assert_eq!(
            ThemeSlot::from_ooxml("folHlink"),
            Some(ThemeSlot::FollowedHyperlink)
        );
        assert_eq!(ThemeSlot::from_ooxml("phClr"), None);
        assert_eq!(ThemeSlot::from_ooxml("accent7"), None);
    }
}
