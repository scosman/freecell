//! `BorderSpec` / `Edge` — the engine-free, fully-resolved cell border the grid draws.
//!
//! Mirrors the rest of the render model (`RenderStyle`, `style.rs`): the worker pre-resolves each
//! IronCalc `Border` into a [`BorderSpec`] and interns it into a `SheetCache.border_specs` side
//! table, so the render path does zero engine-type work (`components/style_render.md`,
//! `architecture.md §3.4`). A cell's [`RenderStyle::border`](crate::RenderStyle::border) is the
//! `u16` index into that table (`0` = [`BorderSpec::NONE`]).

use crate::color::Rgb;

/// One drawn cell edge: a solid line `weight` px wide in `color`. `weight` is `1|2|3` px — the
/// three visual classes IronCalc's nine `BorderStyle`s collapse to (`architecture.md §1.1`; the
/// worker owns that mapping, since it names the engine enum). Dotted/dashed families are drawn
/// solid (SP5-accepted fidelity).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Edge {
    /// Line thickness in device px: `1` (thin/dotted), `2` (medium family), `3` (thick/double).
    pub weight: u8,
    /// Line colour (default `#000` when the engine border item carries none).
    pub color: Rgb,
}

impl Edge {
    /// A solid edge of the given px `weight` and `color`.
    pub const fn new(weight: u8, color: Rgb) -> Self {
        Self { weight, color }
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
}
