//! **Data labels** — the OOXML `c:dLbls` on a chart series (charts/functional_spec §4 P2;
//! coverage-matrix §F `c:dLbls`).
//!
//! A data label is the little text drawn at each data point: its **value**, its **category** or
//! **series name**, its share as a **percent**, and/or the series' **legend key** (a color
//! swatch). This models the `show*` toggles that select those parts, the label **number format**
//! (`c:numFmt`, applied to the value via the P6 [`apply_number_format`] applier), the part
//! **separator** (`c:separator`), and the label **position** (`c:dLblPos`).
//!
//! The chart-group-level `c:dLbls` (a default for every series) and a series' own `c:dLbls` are
//! both parsed into this one type: the loader resolves the chart-level default into each series
//! that lacks its own (OOXML: a series `c:dLbls` *replaces* the chart-level default for that
//! series), so the model only carries data labels on [`Series`](crate::Series) — the render seam
//! reads them per series with no chart-level lookup.

use crate::apply_number_format;

/// The default separator between label parts when `c:separator` is absent — a comma + space,
/// matching Excel's default multi-part label joining.
const DEFAULT_SEPARATOR: &str = ", ";

/// Where a data label sits relative to its data point — the `val` of `c:dLblPos`
/// (`c:ST_DLblPos`), restricted to the positions valid on a line chart (`ctr`/`l`/`r`/`t`/`b`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataLabelPosition {
    /// `ctr` — centered on the point.
    Center,
    /// `l` — to the left of the point.
    Left,
    /// `r` — to the right of the point.
    Right,
    /// `t` — above the point (the line-chart default).
    Above,
    /// `b` — below the point.
    Below,
}

impl DataLabelPosition {
    /// Parse an OOXML `<c:dLblPos val="…">` token. Returns `None` for an unknown/inapplicable
    /// token (e.g. the pie-only `bestFit`, `inEnd`, `outEnd`), leaving the renderer's default.
    pub fn from_ooxml(val: &str) -> Option<Self> {
        Some(match val {
            "ctr" => Self::Center,
            "l" => Self::Left,
            "r" => Self::Right,
            "t" => Self::Above,
            "b" => Self::Below,
            _ => return None,
        })
    }
}

/// A series' data labels (`c:dLbls`): which parts to show, plus the label number format,
/// separator, and position. `Default` is "no labels shown" (every toggle off), matching an
/// all-off `c:dLbls` — [`is_shown`](DataLabels::is_shown) is then false and the renderer draws
/// nothing.
///
/// The value part is formatted through the P6 number-format applier
/// ([`apply_number_format`]); the legend key is a color **swatch** the renderer draws, so it is
/// not part of the composed [`label_text`](DataLabels::label_text).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DataLabels {
    /// `c:showLegendKey` — draw the series' color swatch beside the label.
    pub show_legend_key: bool,
    /// `c:showVal` — the data point's value (formatted via [`number_format`](DataLabels::number_format)).
    pub show_value: bool,
    /// `c:showCatName` — the point's category-axis label.
    pub show_category_name: bool,
    /// `c:showSerName` — the series name.
    pub show_series_name: bool,
    /// `c:showPercent` — the point's value as a percentage of the series total.
    pub show_percent: bool,
    /// `c:dLbls/c:numFmt` `formatCode` for the value part; `None` = general formatting.
    pub number_format: Option<String>,
    /// `c:separator` joining multiple label parts; `None` = the default `", "`.
    pub separator: Option<String>,
    /// `c:dLblPos` label placement; `None` = the renderer's default (above, for a line).
    pub position: Option<DataLabelPosition>,
}

impl DataLabels {
    /// All-off data labels (nothing shown) — the same as [`Default`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Show the value part (builder style).
    pub fn value(mut self) -> Self {
        self.show_value = true;
        self
    }

    /// Show the percent part (builder style).
    pub fn percent(mut self) -> Self {
        self.show_percent = true;
        self
    }

    /// Show the category-name part (builder style).
    pub fn category_name(mut self) -> Self {
        self.show_category_name = true;
        self
    }

    /// Show the series-name part (builder style).
    pub fn series_name(mut self) -> Self {
        self.show_series_name = true;
        self
    }

    /// Show the legend-key swatch (builder style).
    pub fn legend_key(mut self) -> Self {
        self.show_legend_key = true;
        self
    }

    /// Set the value-part number format (`c:numFmt` format code, builder style).
    pub fn with_number_format(mut self, code: impl Into<String>) -> Self {
        self.number_format = Some(code.into());
        self
    }

    /// Set the part separator (builder style).
    pub fn with_separator(mut self, separator: impl Into<String>) -> Self {
        self.separator = Some(separator.into());
        self
    }

    /// Set the label position (builder style).
    pub fn at(mut self, position: DataLabelPosition) -> Self {
        self.position = Some(position);
        self
    }

    /// Whether any label part is shown — the gate the renderer checks before drawing a label.
    /// An all-off `c:dLbls` (present in the XML but every toggle `0`, as Excel emits even when
    /// labels are off) is *not* shown.
    pub fn is_shown(&self) -> bool {
        self.show_legend_key
            || self.show_value
            || self.show_category_name
            || self.show_series_name
            || self.show_percent
    }

    /// Compose the label **text** for one data point from its enabled parts, in Excel's order —
    /// series name, category, value, percent — joined by [`separator`](DataLabels::separator)
    /// (default `", "`). The **value** part is formatted through [`apply_number_format`] under
    /// [`number_format`](DataLabels::number_format).
    ///
    /// `percent` is a pre-computed fraction (or `None` when it does not apply / the total is 0),
    /// rendered as `NN%`. **Note:** the percent part uses a fixed `0%` format — it does **not**
    /// honor an authored label `c:numFmt` like `0.0%` (only the value part does); that is the
    /// bounded P12 behavior. The caller computes the fraction as value ÷ series total, which is the
    /// pie/doughnut semantic (coverage-matrix §F scopes `c:showPercent` to those types); on a
    /// **line** chart `showPercent` has no canonical Excel meaning, so "share of the series total"
    /// is a reasonable non-panicking stand-in, not a spec guarantee.
    ///
    /// The **legend key** is a color swatch drawn by the renderer, so it is not part of this
    /// string (a legend-key-only label composes to `""` — the renderer then draws just the
    /// swatch).
    pub fn label_text(
        &self,
        series_name: Option<&str>,
        category: Option<&str>,
        value: f64,
        percent: Option<f64>,
    ) -> String {
        let separator = self.separator.as_deref().unwrap_or(DEFAULT_SEPARATOR);
        let mut parts: Vec<String> = Vec::new();
        if self.show_series_name {
            if let Some(name) = series_name.filter(|n| !n.is_empty()) {
                parts.push(name.to_string());
            }
        }
        if self.show_category_name {
            if let Some(cat) = category.filter(|c| !c.is_empty()) {
                parts.push(cat.to_string());
            }
        }
        if self.show_value {
            parts.push(apply_number_format(
                self.number_format.as_deref().unwrap_or("General"),
                value,
            ));
        }
        if self.show_percent {
            if let Some(fraction) = percent {
                // Fixed `0%` — the percent part does not honor the label `number_format` (which
                // formats the value part); see the fn docs.
                parts.push(apply_number_format("0%", fraction));
            }
        }
        parts.join(separator)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_parses_line_positions() {
        assert_eq!(
            DataLabelPosition::from_ooxml("t"),
            Some(DataLabelPosition::Above)
        );
        assert_eq!(
            DataLabelPosition::from_ooxml("b"),
            Some(DataLabelPosition::Below)
        );
        assert_eq!(
            DataLabelPosition::from_ooxml("ctr"),
            Some(DataLabelPosition::Center)
        );
        assert_eq!(
            DataLabelPosition::from_ooxml("l"),
            Some(DataLabelPosition::Left)
        );
        assert_eq!(
            DataLabelPosition::from_ooxml("r"),
            Some(DataLabelPosition::Right)
        );
        // A pie-only / unknown position is left to the renderer default.
        assert_eq!(DataLabelPosition::from_ooxml("bestFit"), None);
        assert_eq!(DataLabelPosition::from_ooxml("outEnd"), None);
    }

    #[test]
    fn default_is_all_off_and_not_shown() {
        let dl = DataLabels::default();
        assert!(!dl.is_shown());
        assert_eq!(dl, DataLabels::new());
        // An all-off label composes to nothing.
        assert_eq!(dl.label_text(Some("S"), Some("C"), 5.0, Some(0.5)), "");
    }

    #[test]
    fn value_label_uses_number_format() {
        let dl = DataLabels::new().value();
        assert_eq!(dl.label_text(None, None, 1234.0, None), "1234");

        let currency = DataLabels::new().value().with_number_format("$#,##0");
        assert_eq!(currency.label_text(None, None, 1234.0, None), "$1,234");
    }

    #[test]
    fn percent_label_renders_share() {
        let dl = DataLabels::new().percent();
        assert_eq!(dl.label_text(None, None, 0.0, Some(0.153)), "15%");
        // No total → no percent part (and nothing else shown → empty).
        assert_eq!(dl.label_text(None, None, 10.0, None), "");
    }

    #[test]
    fn parts_compose_in_order_with_separator() {
        let dl = DataLabels::new()
            .series_name()
            .category_name()
            .value()
            .percent();
        // Order is series, category, value, percent — joined by the default ", ".
        assert_eq!(
            dl.label_text(Some("North"), Some("Jan"), 1200.0, Some(0.4)),
            "North, Jan, 1200, 40%"
        );

        // A custom separator (a single space, as Excel sometimes emits).
        let spaced = dl.clone().with_separator(" ");
        assert_eq!(
            spaced.label_text(Some("North"), Some("Jan"), 1200.0, Some(0.4)),
            "North Jan 1200 40%"
        );
    }

    #[test]
    fn legend_key_is_not_part_of_the_text() {
        // Legend key alone is a swatch (renderer-drawn) — the text is empty.
        let dl = DataLabels::new().legend_key();
        assert!(dl.is_shown());
        assert_eq!(
            dl.label_text(Some("North"), Some("Jan"), 1200.0, Some(0.4)),
            ""
        );

        // With a value too, only the value shows as text (the swatch rides alongside).
        let dl = dl.value();
        assert_eq!(
            dl.label_text(Some("North"), Some("Jan"), 1200.0, Some(0.4)),
            "1200"
        );
    }

    #[test]
    fn empty_series_or_category_parts_are_skipped() {
        let dl = DataLabels::new().series_name().category_name().value();
        // A blank/absent series name and category drop out, leaving just the value.
        assert_eq!(dl.label_text(Some(""), None, 7.0, None), "7");
    }
}
