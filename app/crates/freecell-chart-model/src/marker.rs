//! **Series markers** — the OOXML `c:marker` on a line/scatter series (charts/functional_spec §4
//! P2; coverage-matrix §C `c:marker`).
//!
//! A marker is the small symbol drawn at each data point. The PoC only ever drew a round dot; this
//! models the full OOXML symbol set so the renderer can paint the actual shape a file asks for.

/// A marker shape — the `val` of `<c:symbol>` (`c:ST_MarkerStyle`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkerSymbol {
    /// No marker (`none`) — draw the line with no point symbols.
    None,
    /// `auto` — let the renderer choose (treated as a filled circle).
    Auto,
    Circle,
    Square,
    Diamond,
    Triangle,
    Star,
    /// `x` — a diagonal cross.
    X,
    /// `plus` — an upright cross.
    Plus,
    /// `dash` — a short horizontal tick.
    Dash,
    /// `dot` — a small filled circle.
    Dot,
}

impl MarkerSymbol {
    /// Parse an OOXML `<c:symbol val="…">` token. Returns `None` for an unknown token.
    pub fn from_ooxml(name: &str) -> Option<Self> {
        Some(match name {
            "none" => Self::None,
            "auto" => Self::Auto,
            "circle" => Self::Circle,
            "square" => Self::Square,
            "diamond" => Self::Diamond,
            "triangle" => Self::Triangle,
            "star" => Self::Star,
            "x" => Self::X,
            "plus" => Self::Plus,
            "dash" => Self::Dash,
            "dot" => Self::Dot,
            _ => return None,
        })
    }

    /// Whether this symbol draws nothing.
    pub fn is_none(self) -> bool {
        matches!(self, MarkerSymbol::None)
    }
}

/// A series marker (`c:marker`): its [symbol](MarkerSymbol) and an optional size in points
/// (`<c:size>`, roughly the marker diameter). Fill/line color follow the series color, so they are
/// not modeled separately at this phase (per-point `dPt` styling is P12).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Marker {
    pub symbol: MarkerSymbol,
    pub size: Option<f32>,
}

impl Marker {
    /// A marker with the given symbol and the renderer's default size.
    pub const fn new(symbol: MarkerSymbol) -> Self {
        Self { symbol, size: None }
    }

    /// Set an explicit marker size (builder style).
    pub const fn with_size(mut self, size: f32) -> Self {
        self.size = Some(size);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_ooxml_maps_known_symbols() {
        assert_eq!(
            MarkerSymbol::from_ooxml("circle"),
            Some(MarkerSymbol::Circle)
        );
        assert_eq!(
            MarkerSymbol::from_ooxml("square"),
            Some(MarkerSymbol::Square)
        );
        assert_eq!(
            MarkerSymbol::from_ooxml("diamond"),
            Some(MarkerSymbol::Diamond)
        );
        assert_eq!(MarkerSymbol::from_ooxml("none"), Some(MarkerSymbol::None));
        assert_eq!(MarkerSymbol::from_ooxml("auto"), Some(MarkerSymbol::Auto));
        assert_eq!(MarkerSymbol::from_ooxml("nope"), None);
    }

    #[test]
    fn none_symbol_reports_empty() {
        assert!(MarkerSymbol::None.is_none());
        assert!(!MarkerSymbol::Circle.is_none());
    }

    #[test]
    fn marker_builder_sets_symbol_and_size() {
        let m = Marker::new(MarkerSymbol::Diamond);
        assert_eq!(m.symbol, MarkerSymbol::Diamond);
        assert_eq!(m.size, None);
        assert_eq!(m.with_size(9.0).size, Some(9.0));
    }
}
