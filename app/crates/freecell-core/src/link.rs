//! Cell hyperlinks — the engine-free read model (`hyperlinks/architecture.md §3.2`).
//!
//! A link is **whole-cell metadata**: the text drawn in the cell is the cell's own content and
//! is never part of the link. IronCalc models the same shape as `ironcalc_base::types::Link`,
//! but that type may not cross into `freecell-core` (`tests/dependency_rule.rs` forbids it), so
//! [`LinkTarget`] mirrors it and `freecell-engine` converts at its boundary — exactly as
//! [`CfRuleView`](crate::CfRuleView) already does for conditional-formatting rules.

/// Where a hyperlink points.
///
/// [`External`](Self::External) covers URLs, `mailto:`, and file paths — a target that points
/// *inside* another document is one string, `target#location`. [`Internal`](Self::Internal) is a
/// location in **this** workbook: an A1 reference like `Sheet1!A30`, or a defined name.
///
/// `tooltip` is Excel's ScreenTip, preserved on both kinds through xlsx round-trip.
///
/// Nothing here validates the target: an opened workbook is untrusted input and can carry
/// `javascript:`, `data:`, or an arbitrary local path. FreeCell renders and round-trips such a
/// link unchanged and refuses it at **open** time only (`functional_spec.md F3.4`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LinkTarget {
    External {
        target: String,
        tooltip: Option<String>,
    },
    Internal {
        location: String,
        tooltip: Option<String>,
    },
}

impl LinkTarget {
    /// An external link (URL / `mailto:` / file path) with no ScreenTip.
    pub fn external(target: impl Into<String>) -> Self {
        Self::External {
            target: target.into(),
            tooltip: None,
        }
    }

    /// An in-workbook link (`Sheet1!A30`, or a defined name) with no ScreenTip.
    pub fn internal(location: impl Into<String>) -> Self {
        Self::Internal {
            location: location.into(),
            tooltip: None,
        }
    }

    /// The same link carrying `tooltip` as its ScreenTip (`None` clears it).
    pub fn with_tooltip(self, tooltip: Option<String>) -> Self {
        match self {
            Self::External { target, .. } => Self::External { target, tooltip },
            Self::Internal { location, .. } => Self::Internal { location, tooltip },
        }
    }

    /// The destination string: the URL/path for an external link, the workbook location for an
    /// internal one. What the hover tooltip shows and what the open policy inspects.
    pub fn destination(&self) -> &str {
        match self {
            Self::External { target, .. } => target,
            Self::Internal { location, .. } => location,
        }
    }

    /// The link's stored ScreenTip, if it has one.
    pub fn tooltip(&self) -> Option<&str> {
        match self {
            Self::External { tooltip, .. } | Self::Internal { tooltip, .. } => tooltip.as_deref(),
        }
    }

    /// Whether this points inside the workbook (never launched — no scheme is involved).
    pub fn is_internal(&self) -> bool {
        matches!(self, Self::Internal { .. })
    }
}

/// A cell's **effective** link: the link the UI acts on, plus where it came from.
///
/// A cell can be reached by two kinds of link, and the engine's precedence is that the
/// worksheet one wins:
/// - a **worksheet** link (`dynamic == false`) — user-created (⌘K, auto-linking) or imported
///   from xlsx. Persisted, undoable, editable, removable.
/// - a **dynamic** link (`dynamic == true`) — produced by a `HYPERLINK()` formula. It lives only
///   for as long as the formula produces it, is rebuilt on **every** evaluation, is never
///   persisted, and can be changed only by editing the formula
///   (`functional_spec.md F6.3` disables the Remove affordance on one).
///
/// The flag is carried in the cache (rather than re-asked of the worker) because the ⌘K dialog
/// pre-fills synchronously from the cached snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellLink {
    pub target: LinkTarget,
    pub dynamic: bool,
}

impl CellLink {
    /// A worksheet (editable, persisted) link.
    pub fn worksheet(target: LinkTarget) -> Self {
        Self {
            target,
            dynamic: false,
        }
    }

    /// A `HYPERLINK()`-produced link (not editable or removable).
    pub fn dynamic(target: LinkTarget) -> Self {
        Self {
            target,
            dynamic: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_target_accessors_read_both_variants() {
        let plain = LinkTarget::external("https://example.com");
        assert_eq!(plain.destination(), "https://example.com");
        assert_eq!(plain.tooltip(), None);
        assert!(!plain.is_internal());

        let tipped = LinkTarget::external("mailto:a@b.com").with_tooltip(Some("Email".into()));
        assert_eq!(tipped.destination(), "mailto:a@b.com");
        assert_eq!(tipped.tooltip(), Some("Email"));

        let internal = LinkTarget::internal("Sheet1!A30").with_tooltip(Some("Jump".into()));
        assert_eq!(internal.destination(), "Sheet1!A30");
        assert_eq!(internal.tooltip(), Some("Jump"));
        assert!(internal.is_internal());

        // `with_tooltip(None)` clears rather than keeps.
        assert_eq!(tipped.with_tooltip(None).tooltip(), None);
    }

    #[test]
    fn cell_link_constructors_set_the_kind() {
        assert!(!CellLink::worksheet(LinkTarget::external("https://x.y")).dynamic);
        assert!(CellLink::dynamic(LinkTarget::external("https://x.y")).dynamic);
    }
}
