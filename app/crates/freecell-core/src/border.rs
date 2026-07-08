//! `BorderSpec` / `Edge` — the engine-free, fully-resolved cell border the grid draws.
//!
//! Mirrors the rest of the render model (`RenderStyle`, `style.rs`): the worker pre-resolves each
//! IronCalc `Border` into a [`BorderSpec`] and interns it into a `SheetCache.border_specs` side
//! table, so the render path does zero engine-type work (`components/style_render.md`,
//! `architecture.md §3.4`). A cell's [`RenderStyle::border`](crate::RenderStyle::border) is the
//! `u16` index into that table (`0` = [`BorderSpec::NONE`]).

use crate::color::Rgb;

/// The line pattern an [`Edge`] draws (`architecture.md §1`). The worker resolves it from the
/// IronCalc `BorderStyle`: `MediumDashed → Dashed`, `Double → Double`, everything else (thin/
/// medium/thick solid, and the deferred Dotted / dash-dot / SlantDashDot families) → `Solid`.
/// Dotted is deferred (GAPS F3) — it renders `Solid` for now, unchanged from before this landed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LinePattern {
    /// A single filled strip (the default; how every non-dashed, non-double edge draws).
    #[default]
    Solid,
    /// Evenly spaced dashes along the edge (IronCalc `MediumDashed`).
    Dashed,
    /// Two thin parallel strips separated by a gap, spanning the edge weight (IronCalc `Double`).
    Double,
}

/// One drawn cell edge: a line `weight` px wide in `color`, drawn with `pattern`. `weight` is
/// `1|2|3` px — the three visual classes IronCalc's nine `BorderStyle`s collapse to
/// (`architecture.md §1.1`; the worker owns that mapping, since it names the engine enum).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Edge {
    /// Line thickness in device px: `1` (thin/dotted), `2` (medium family), `3` (thick/double).
    pub weight: u8,
    /// Line colour (default `#000` when the engine border item carries none).
    pub color: Rgb,
    /// Line pattern (Solid / Dashed / Double — `architecture.md §7`).
    pub pattern: LinePattern,
}

impl Edge {
    /// A **solid** edge of the given px `weight` and `color` (the common case — every edge that
    /// isn't dashed or double). Kept as the primary constructor so existing callers are unchanged.
    pub const fn new(weight: u8, color: Rgb) -> Self {
        Self {
            weight,
            color,
            pattern: LinePattern::Solid,
        }
    }

    /// An edge with an explicit `pattern` (used by the cache resolver for dashed / double).
    pub const fn with_pattern(weight: u8, color: Rgb, pattern: LinePattern) -> Self {
        Self {
            weight,
            color,
            pattern,
        }
    }
}

/// A cell's four resolved edges (`None` on an edge = no border there). Interned into the owning
/// [`SheetCache`](crate::SheetCache)'s `border_specs` side table; `BorderSpec::default()` is
/// [`NONE`](BorderSpec::NONE), so a default cell interns to index `0` (like every other render
/// field's zero).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct BorderSpec {
    pub top: Option<Edge>,
    pub right: Option<Edge>,
    pub bottom: Option<Edge>,
    pub left: Option<Edge>,
}

impl BorderSpec {
    /// The "no borders" spec — the value at side-table index `0`.
    pub const NONE: Self = Self {
        top: None,
        right: None,
        bottom: None,
        left: None,
    };

    /// Whether this spec draws nothing (all four edges absent).
    pub fn is_none(&self) -> bool {
        self.top.is_none() && self.right.is_none() && self.bottom.is_none() && self.left.is_none()
    }
}

/// The effective edge to draw between a cell and its neighbour: the **heavier** of the cell's own
/// edge and the neighbour's opposing edge, ties resolving to the cell's **own** edge
/// (`components/style_render.md §Border painting`). "Heavier" = larger [`Edge::weight`].
///
/// A border between two cells is a single shared line: the grid draws it once (from the cell that
/// owns that boundary — the left/top cell), and both neighbours compute the *same* effective edge,
/// so the result is independent of which side draws it (no double-draw, consistent precedence when
/// adjacent cells disagree — the exact subtlety `architecture.md §3.4` calls out). Files loaded
/// from disk can carry disagreeing adjacent edges (they weren't written through the engine's
/// heavier-wins fix-up), so this resolution is load-bearing at render time, not just cosmetic.
pub fn effective_edge(own: Option<Edge>, neighbor: Option<Edge>) -> Option<Edge> {
    match (own, neighbor) {
        (None, None) => None,
        (Some(e), None) | (None, Some(e)) => Some(e),
        // Tie (equal weight) keeps `own` — the `>` (not `>=`) is what makes ties prefer own.
        (Some(own), Some(neighbor)) => Some(if neighbor.weight > own.weight {
            neighbor
        } else {
            own
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(weight: u8) -> Edge {
        Edge::new(weight, Rgb::new(0, 0, 0))
    }

    #[test]
    fn border_spec_none_is_default() {
        assert_eq!(BorderSpec::default(), BorderSpec::NONE);
        assert!(BorderSpec::default().is_none());
        assert!(!BorderSpec {
            top: Some(edge(1)),
            ..BorderSpec::NONE
        }
        .is_none());
    }

    #[test]
    fn effective_edge_heavier_wins_and_tie_prefers_own() {
        // Absent sides.
        assert_eq!(effective_edge(None, None), None);
        assert_eq!(effective_edge(Some(edge(1)), None), Some(edge(1)));
        assert_eq!(effective_edge(None, Some(edge(2))), Some(edge(2)));
        // Heavier neighbour wins.
        assert_eq!(effective_edge(Some(edge(1)), Some(edge(3))), Some(edge(3)));
        // Heavier own wins.
        assert_eq!(effective_edge(Some(edge(3)), Some(edge(1))), Some(edge(3)));
        // Tie prefers own — proven by distinguishing the two same-weight edges by colour.
        let own = Edge::new(2, Rgb::new(0xAA, 0, 0));
        let nbr = Edge::new(2, Rgb::new(0, 0, 0xBB));
        assert_eq!(effective_edge(Some(own), Some(nbr)), Some(own));
    }

    #[test]
    fn edge_new_is_solid_with_pattern_carries_pattern() {
        assert_eq!(Edge::new(1, Rgb::new(0, 0, 0)).pattern, LinePattern::Solid);
        assert_eq!(LinePattern::default(), LinePattern::Solid);
        let dashed = Edge::with_pattern(2, Rgb::new(0, 0, 0), LinePattern::Dashed);
        assert_eq!(dashed.pattern, LinePattern::Dashed);
        assert_eq!(dashed.weight, 2);
    }

    #[test]
    fn effective_edge_winner_carries_its_own_pattern() {
        // Weight still decides the winner; the winning edge brings its pattern along.
        let thin_dashed = Edge::with_pattern(1, Rgb::new(0, 0, 0), LinePattern::Dashed);
        let thick_double = Edge::with_pattern(3, Rgb::new(0, 0, 0), LinePattern::Double);
        assert_eq!(
            effective_edge(Some(thin_dashed), Some(thick_double)),
            Some(thick_double),
            "heavier double edge wins and keeps its Double pattern"
        );
        assert_eq!(
            effective_edge(Some(thick_double), Some(thin_dashed)),
            Some(thick_double),
        );
    }
}
