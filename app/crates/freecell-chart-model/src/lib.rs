//! `freecell-chart-model` — a small Rust data model that **mirrors the OOXML `c:` chart
//! structure** (charts/functional_spec §2, architecture §3).
//!
//! It is the **stable seam** between the two chart layers: `freecell-engine::chart`
//! parses chart XML *into* this model, and `freecell-app::chart` renders *from* it. It is
//! deliberately **gpui-free and ironcalc-free** so it builds and tests anywhere with no
//! GPU/display, and neither layer can reach across it.
//!
//! Two layers of shape live here:
//! - [`Chart`] — the **render seam**: the static chart picture (kind, series, axes, legend),
//!   lifted from the chart PoC. Values come from the chart XML's **cached**
//!   `<c:numCache>` / `<c:strCache>`, so no formula evaluation is needed to render — the model
//!   only ever holds concrete numbers and strings.
//! - [`ChartSpec`] — the **production envelope** (P2): a [`Chart`] wrapped with the retained
//!   **source** XML, the live-binding **source ranges**, the in-grid **anchor**, and the
//!   chart's **origin**.
//!
//! On top of those sits the **derived** [`Fidelity`] accessor
//! ([`ChartSpec::display_fidelity`], P3): how faithfully the renderer can draw a chart
//! (`Faithful` / `Degraded` / `Unsupported`), computed on demand from the model + retained
//! source rather than stored, so it auto-clears as renderer support lands.
//!
//! The model is **OOXML-shaped but bounded, not exhaustive** (architecture §3.1): it carries
//! typed fields for what we render/edit; the rendered P1/P2 fidelity fields are added
//! additively with their phases (P6/P12/P13), and the unbounded DrawingML long tail is
//! preserved via [`ChartSpec`]'s retained source rather than modeled.

mod fidelity;
mod marker;
mod numfmt;
mod spec;
mod theme;

pub use fidelity::{normalize_3d_chart_group, source_fidelity, Fidelity};
pub use marker::{Marker, MarkerSymbol};
pub use numfmt::apply_number_format;
pub use spec::{Anchor, AnchorCell, CfRange, ChartSpec, Origin, SourcePart, SourceXml};
pub use theme::{ChartColor, ThemePalette, ThemeSlot};

/// An sRGB color, mirroring OOXML `<a:srgbClr val="RRGGBB"/>`.
///
/// This is the concrete resolved color. Theme-slot references (`<a:schemeClr>`) and their
/// `lumMod`/`lumOff` tints are modeled by [`ChartColor`] (P6), which resolves to a [`Color`]
/// against a [`ThemePalette`]; alpha is still out of scope (P13).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Build from a packed `0xRRGGBB` value (the shape OOXML stores as a hex string).
    pub const fn from_hex(hex: u32) -> Self {
        Self {
            r: ((hex >> 16) & 0xFF) as u8,
            g: ((hex >> 8) & 0xFF) as u8,
            b: (hex & 0xFF) as u8,
        }
    }

    /// Pack back into `0xRRGGBB` (inverse of [`Color::from_hex`]).
    pub const fn to_hex(self) -> u32 {
        ((self.r as u32) << 16) | ((self.g as u32) << 8) | (self.b as u32)
    }
}

/// Direction of a bar chart — vertical columns or horizontal bars. Mirrors `c:barDir`
/// (`val="col"` / `val="bar"`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarDir {
    /// Vertical bars (Excel "Column").
    Col,
    /// Horizontal bars (Excel "Bar").
    Bar,
}

/// Grouping mode shared by bar / line / area. Mirrors OOXML `c:grouping`
/// (`standard` / `clustered` / `stacked` / `percentStacked`).
///
/// Bar charts use [`Grouping::Clustered`] for the un-stacked case; line/area use
/// [`Grouping::Standard`]. Both stacked variants apply across all three.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grouping {
    /// Overlaid series on a shared value axis (line/area default).
    Standard,
    /// Side-by-side groups (bar default).
    Clustered,
    /// Cumulative stack.
    Stacked,
    /// Cumulative stack normalized to 100%.
    PercentStacked,
}

/// The chart type + its type-specific options. Mirrors the `c:<type>Chart` element
/// (functional_spec §2). Only the in-scope PoC types are represented.
#[derive(Clone, Debug, PartialEq)]
pub enum ChartKind {
    /// `c:barChart` — columns or horizontal bars.
    Bar { dir: BarDir, grouping: Grouping },
    /// `c:lineChart`.
    Line { grouping: Grouping, smooth: bool },
    /// `c:areaChart`.
    Area { grouping: Grouping },
    /// `c:pieChart` / `c:doughnutChart`. `doughnut_hole` is the hole radius as a
    /// fraction of the outer radius (`None` = solid pie).
    Pie { doughnut_hole: Option<f32> },
    /// `c:scatterChart` — uses [`SeriesData::Xy`] series.
    Scatter,
}

/// A single category-axis label from `c:cat` — a cached string (`c:strCache`) or a
/// cached number (`c:numCache`). Kept as an enum so numeric categories round-trip
/// without being coerced to text.
#[derive(Clone, Debug, PartialEq)]
pub enum Category {
    Text(String),
    Number(f64),
}

impl Category {
    /// The label to draw on the category axis.
    pub fn label(&self) -> String {
        match self {
            Category::Text(s) => s.clone(),
            Category::Number(n) => format_number(*n),
        }
    }
}

impl std::fmt::Display for Category {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label())
    }
}

/// The data of one series: either category/value (bar/line/area/pie, mirroring
/// `c:cat` + `c:val`) or xy (scatter, mirroring `c:xVal` + `c:yVal`).
#[derive(Clone, Debug, PartialEq)]
pub enum SeriesData {
    /// Cached `c:cat` categories paired with cached `c:val` values.
    CategoryValue {
        categories: Vec<Category>,
        values: Vec<f64>,
    },
    /// Cached `c:xVal` / `c:yVal` numeric pairs (scatter).
    Xy { x: Vec<f64>, y: Vec<f64> },
}

/// One data series — a `c:ser` element. `color` mirrors the series' `c:spPr` fill — an explicit
/// sRGB color or a theme reference ([`ChartColor`], P6); `None` means "let the renderer pick from
/// the palette cycle". `marker` mirrors `c:marker` (the point symbol for a line/scatter series,
/// P6); `None` leaves the marker to the renderer's default.
#[derive(Clone, Debug, PartialEq)]
pub struct Series {
    pub name: Option<String>,
    pub data: SeriesData,
    pub color: Option<ChartColor>,
    pub marker: Option<Marker>,
}

impl Series {
    /// A category/value series (bar/line/area/pie).
    pub fn category_value(
        name: Option<impl Into<String>>,
        categories: Vec<Category>,
        values: Vec<f64>,
    ) -> Self {
        Self {
            name: name.map(Into::into),
            data: SeriesData::CategoryValue { categories, values },
            color: None,
            marker: None,
        }
    }

    /// An xy series (scatter).
    pub fn xy(name: Option<impl Into<String>>, x: Vec<f64>, y: Vec<f64>) -> Self {
        Self {
            name: name.map(Into::into),
            data: SeriesData::Xy { x, y },
            color: None,
            marker: None,
        }
    }

    /// Set an explicit series color (builder style) — an sRGB [`Color`] or a [`ChartColor`]
    /// (theme reference); a plain `Color` converts via [`From<Color>`](ChartColor).
    pub fn with_color(mut self, color: impl Into<ChartColor>) -> Self {
        self.color = Some(color.into());
        self
    }

    /// Set the series marker (builder style).
    pub fn with_marker(mut self, marker: Marker) -> Self {
        self.marker = Some(marker);
        self
    }

    /// Number of data points in this series.
    pub fn len(&self) -> usize {
        match &self.data {
            SeriesData::CategoryValue { values, .. } => values.len(),
            SeriesData::Xy { x, .. } => x.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// An axis — its title (`c:valAx` / `c:catAx` `c:title`) and its number format (`c:numFmt`
/// `formatCode`, applied to tick labels, P6). The scale/tick generation is the renderer's business
/// (functional_spec §2); `number_format` only governs how each tick number is *rendered*
/// ([`apply_number_format`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Axis {
    pub title: Option<String>,
    /// The `c:numFmt` format code (e.g. `"0%"`, `"$#,##0"`); `None` = general number formatting.
    pub number_format: Option<String>,
}

impl Axis {
    pub fn untitled() -> Self {
        Self {
            title: None,
            number_format: None,
        }
    }

    pub fn titled(title: impl Into<String>) -> Self {
        Self {
            title: Some(title.into()),
            number_format: None,
        }
    }

    /// Set the axis tick number format (`c:numFmt` format code, builder style).
    pub fn with_number_format(mut self, format_code: impl Into<String>) -> Self {
        self.number_format = Some(format_code.into());
        self
    }
}

/// Legend placement (`c:legendPos`). Presence is what matters for the PoC; the
/// renderer may treat the position as advisory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LegendPosition {
    Right,
    Bottom,
    Left,
    Top,
    TopRight,
}

/// A chart legend (`c:legend`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Legend {
    pub position: LegendPosition,
}

impl Default for Legend {
    fn default() -> Self {
        Self {
            position: LegendPosition::Right,
        }
    }
}

/// A whole chart — the root `c:chart` (functional_spec §2).
#[derive(Clone, Debug, PartialEq)]
pub struct Chart {
    pub title: Option<String>,
    pub kind: ChartKind,
    pub series: Vec<Series>,
    pub cat_axis: Axis,
    pub val_axis: Axis,
    pub legend: Option<Legend>,
}

/// Format a cached number for a label: integers print without a decimal point, other
/// values keep up to a few significant fractional digits, trimmed of trailing zeros. Shared with
/// [`numfmt`] as the `General` / fall-back formatting.
pub(crate) fn format_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        return format!("{}", n as i64);
    }
    let mut s = format!("{n:.3}");
    while s.contains('.') && (s.ends_with('0') || s.ends_with('.')) {
        s.pop();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_hex_round_trips() {
        for hex in [0x000000, 0xFFFFFF, 0x1F77B4, 0xFFEB3B, 0x2E4053] {
            let c = Color::from_hex(hex);
            assert_eq!(c.to_hex(), hex, "hex {hex:#08x} did not round-trip");
        }
        assert_eq!(Color::from_hex(0x1F77B4), Color::rgb(0x1F, 0x77, 0xB4));
    }

    #[test]
    fn category_labels_render_text_and_numbers() {
        assert_eq!(Category::Text("Q1".into()).label(), "Q1");
        assert_eq!(Category::Number(2024.0).label(), "2024");
        assert_eq!(Category::Number(3.5).label(), "3.5");
        assert_eq!(Category::Number(3.250).label(), "3.25");
    }

    #[test]
    fn series_len_reflects_underlying_data() {
        let cv = Series::category_value(
            Some("Revenue"),
            vec![Category::Text("Q1".into()), Category::Text("Q2".into())],
            vec![10.0, 20.0],
        );
        assert_eq!(cv.len(), 2);
        assert!(!cv.is_empty());

        let xy = Series::xy(None::<String>, vec![], vec![]);
        assert!(xy.is_empty());
    }

    #[test]
    fn series_color_accepts_rgb_and_theme() {
        let base = Series::category_value(Some("s"), vec![], vec![]);
        assert_eq!(base.color, None, "constructors default color to None");
        assert_eq!(base.marker, None, "constructors default marker to None");

        // A plain `Color` converts to `ChartColor::Rgb` via `From`.
        let explicit = base.clone().with_color(Color::from_hex(0x123456));
        assert_eq!(
            explicit.color,
            Some(ChartColor::Rgb(Color::from_hex(0x123456)))
        );

        // A theme reference is stored as-is.
        let themed = base
            .clone()
            .with_color(ChartColor::theme(ThemeSlot::Accent2));
        assert_eq!(themed.color, Some(ChartColor::theme(ThemeSlot::Accent2)));

        // Markers ride the same builder pattern.
        let marked = base.with_marker(Marker::new(MarkerSymbol::Diamond));
        assert_eq!(marked.marker, Some(Marker::new(MarkerSymbol::Diamond)));
    }

    #[test]
    fn axis_number_format_builder() {
        assert_eq!(Axis::untitled().number_format, None);
        assert_eq!(Axis::titled("Units").number_format, None);
        let ax = Axis::titled("Revenue").with_number_format("$#,##0");
        assert_eq!(ax.title.as_deref(), Some("Revenue"));
        assert_eq!(ax.number_format.as_deref(), Some("$#,##0"));
    }

    /// The model "round-trips" in the sense that a chart built through the public API
    /// reads back exactly the values it was given (there is no serialization yet —
    /// that is Experiment 1's job; this guards the in-memory shape / seam).
    #[test]
    fn chart_round_trips_through_accessors() {
        let chart = Chart {
            title: Some("Quarterly Revenue".into()),
            kind: ChartKind::Bar {
                dir: BarDir::Col,
                grouping: Grouping::Clustered,
            },
            series: vec![Series::category_value(
                Some("2024"),
                vec![
                    Category::Text("Q1".into()),
                    Category::Text("Q2".into()),
                    Category::Text("Q3".into()),
                ],
                vec![120.0, 90.0, 150.0],
            )
            .with_color(Color::from_hex(0x1F77B4))],
            cat_axis: Axis::titled("Quarter"),
            val_axis: Axis::titled("USD (thousands)"),
            legend: Some(Legend::default()),
        };

        assert_eq!(chart.title.as_deref(), Some("Quarterly Revenue"));
        assert_eq!(
            chart.kind,
            ChartKind::Bar {
                dir: BarDir::Col,
                grouping: Grouping::Clustered
            }
        );
        assert_eq!(chart.series.len(), 1);
        let s = &chart.series[0];
        assert_eq!(s.name.as_deref(), Some("2024"));
        assert_eq!(s.color, Some(ChartColor::Rgb(Color::rgb(0x1F, 0x77, 0xB4))));
        assert_eq!(s.len(), 3);
        match &s.data {
            SeriesData::CategoryValue { categories, values } => {
                assert_eq!(categories.len(), 3);
                assert_eq!(values, &vec![120.0, 90.0, 150.0]);
                assert_eq!(categories[1].label(), "Q2");
            }
            other => panic!("expected CategoryValue, got {other:?}"),
        }
        assert_eq!(chart.cat_axis.title.as_deref(), Some("Quarter"));
        assert_eq!(chart.val_axis.title.as_deref(), Some("USD (thousands)"));
        assert_eq!(
            chart.legend.map(|l| l.position),
            Some(LegendPosition::Right)
        );
    }
}
