//! Cell hyperlinks — the engine-free read model (`hyperlinks/architecture.md §3.2`) plus the
//! pure decisions the UI layers over it (`§5`–`§7`).
//!
//! A link is **whole-cell metadata**: the text drawn in the cell is the cell's own content and
//! is never part of the link. IronCalc models the same shape as `ironcalc_base::types::Link`,
//! but that type may not cross into `freecell-core` (`tests/dependency_rule.rs` forbids it), so
//! [`LinkTarget`] mirrors it and `freecell-engine` converts at its boundary — exactly as
//! [`CfRuleView`](crate::CfRuleView) already does for conditional-formatting rules.
//!
//! The decisions here are deliberately gpui-free so they can be unit-tested directly (house
//! style): [`decide_open`] is the security seam an untrusted workbook's links pass through,
//! [`link_click_opens`] is the click gesture, [`parse_internal_location`] reads an in-workbook
//! target, and [`link_tooltip_text`] formats the hover text.

use std::collections::HashMap;

use crate::refs::{CellRange, CellRef};

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

/// A sheet's effective hyperlinks, keyed by 0-based `(row, col)` — what
/// [`SheetCache`](crate::SheetCache) holds and what the engine's link reads and mirror pass
/// around. Named here, beside [`CellLink`], so the cache and the engine spell the shape the same
/// way.
pub type LinkMap = HashMap<(u32, u32), CellLink>;

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

// --- Opening a link: the security seam (`hyperlinks/architecture.md §6`, F3) ------------------

/// What FreeCell is willing to do with a link target when the user asks to open it.
///
/// The engine performs **no** scheme validation at any layer — not `set_cell_link`, not xlsx
/// import, not `HYPERLINK()` — so an opened workbook is untrusted input that can carry
/// `javascript:`, `data:`, `smb:`, or an arbitrary local path straight to `App::open_url`, which
/// will launch anything. [`decide_open`] is the one gate between the two.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenDecision {
    /// Hand this exact string to the platform opener.
    Open(String),
    /// A `file:` target: ask the user first, naming the path, then open on accept (F3.2).
    ConfirmFile(String),
    /// Launch nothing and say why. `scheme` is the scheme that was refused, or **empty** when the
    /// target has no usable scheme at all (a bare path, a scheme-relative `//host/…`, or a target
    /// carrying a control character — see [`decide_open`]).
    Refuse { scheme: String },
}

/// The schemes FreeCell opens without asking (F3.1).
const OPENABLE_SCHEMES: [&str; 5] = ["http", "https", "mailto", "ftp", "ftps"];

/// Decides what opening `target` may do (F3), at **open** time only: import, render, xlsx
/// round-trip, and the ⌘K dialog all show a refused link unchanged (F3.4) — FreeCell does not
/// rewrite a user's file, it just will not launch it.
///
/// - the five [`OPENABLE_SCHEMES`] → [`Open`](OpenDecision::Open);
/// - `file` → [`ConfirmFile`](OpenDecision::ConfirmFile);
/// - everything else → [`Refuse`](OpenDecision::Refuse).
///
/// Three hardening details, each with a hostile input behind it:
/// - the target is **trimmed first**, so `"  javascript:alert(1)"` cannot hide its scheme behind
///   leading whitespace, and the trimmed string is what is returned — what is inspected is what
///   gets launched;
/// - a target containing a **control character** is refused outright, whatever its scheme looks
///   like: `"http://ok\njavascript:alert(1)"` must never reach the platform opener as two lines;
/// - an [`Internal`](LinkTarget::Internal) target refuses. Nothing launches an in-workbook
///   location (F4.4) — the caller routes it to navigation before reaching here, and this is the
///   backstop if one ever forgets.
pub fn decide_open(target: &LinkTarget) -> OpenDecision {
    let refuse_unusable = || OpenDecision::Refuse {
        scheme: String::new(),
    };
    let LinkTarget::External { target, .. } = target else {
        return refuse_unusable();
    };
    let target = target.trim();
    if target.chars().any(char::is_control) {
        return refuse_unusable();
    }
    let Some(scheme) = scheme_of(target) else {
        return refuse_unusable();
    };
    if OPENABLE_SCHEMES.contains(&scheme.as_str()) {
        OpenDecision::Open(target.to_string())
    } else if scheme == "file" {
        OpenDecision::ConfirmFile(target.to_string())
    } else {
        OpenDecision::Refuse { scheme }
    }
}

/// The lowercased URI scheme `target` starts with, or `None` when it has none.
///
/// RFC 3986's grammar (`ALPHA *( ALPHA / DIGIT / "+" / "-" / "." ) ":"`), with one deliberate
/// narrowing: a **single-letter** scheme is not recognised, so a Windows path (`C:\Users\…`) reads
/// as the bare path it is rather than as a `c:` scheme. Both outcomes refuse; this only makes the
/// message honest.
fn scheme_of(target: &str) -> Option<String> {
    let mut chars = target.char_indices();
    if !chars.next()?.1.is_ascii_alphabetic() {
        return None;
    }
    for (i, c) in chars {
        if c == ':' {
            return (i >= 2).then(|| target[..i].to_ascii_lowercase());
        }
        if !(c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.') {
            return None;
        }
    }
    None
}

// --- The click gesture (`hyperlinks/architecture.md §5`, F2.1–F2.2) --------------------------

/// The gesture a mouse-up completed over a cell that had a link under the preceding mouse-down.
///
/// Deliberately free of gpui types (and of the point-mode / degraded conditions, which are
/// structural — see [`link_click_opens`]) so the decision is directly unit-testable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkClick {
    /// The release landed on the same cell as the press (merge-snapped on both ends).
    pub same_cell: bool,
    /// The pointer moved far enough between press and release to read as a drag — the user is
    /// sweeping a range, not clicking a link.
    pub dragged: bool,
    /// The primary modifier (⌘ on macOS, Ctrl elsewhere) was held: the deliberate
    /// "select this cell without following its link" affordance.
    pub primary_modifier: bool,
    /// The platform's click count for the release (`2` = a double click, which opens the in-cell
    /// editor instead).
    pub click_count: usize,
}

/// Whether `click` should follow the cell's link (F2.1) rather than being an ordinary selection
/// click (F2.2). Excel parity: a plain, single, un-dragged left click on the same cell.
///
/// Two of F2.2's conditions are **not** inputs here because they are structural rather than
/// gestural: a reference-ready formula edit (point mode) never arms a pending link click in the
/// first place, and degraded mode is refused by the window, which owns that state.
pub fn link_click_opens(click: LinkClick) -> bool {
    click.same_cell && !click.dragged && !click.primary_modifier && click.click_count == 1
}

// --- Internal links (`hyperlinks/architecture.md §7`, F4) -------------------------------------

/// Where a [`LinkTarget::Internal`] points, once parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InternalTarget {
    /// A cell on a named sheet, or on the **active** sheet when `sheet` is `None`. A range target
    /// (`Sheet1!A1:C3`) resolves to its top-left cell — the place Excel lands the selection.
    Cell {
        sheet: Option<String>,
        cell: CellRef,
    },
    /// Anything that is not an A1 reference — in practice a defined name. FreeCell models no
    /// defined names anywhere (no engine facade, no command, no read model), so navigation reports
    /// these unresolvable (F4.3) rather than guessing; the hole is tracked in `GAPS.md`.
    DefinedName(String),
}

/// Parses a `LinkTarget::Internal` location — `Sheet1!A30`, `'My Sheet'!B2`, a bare `A30`, a range
/// (whose top-left wins), or a defined name. Returns `None` only for an empty location.
///
/// A leading `#` (as written by `HYPERLINK("#Sheet1!A1")` and by xlsx) is accepted and stripped.
/// The sheet name is split on the **last** `!`, since a quoted sheet name may itself contain one,
/// and `'…'` quoting is undone (`''` → a literal `'`), matching how Excel writes names with spaces.
pub fn parse_internal_location(location: &str) -> Option<InternalTarget> {
    let raw = location.trim();
    let raw = raw.strip_prefix('#').unwrap_or(raw).trim();
    if raw.is_empty() {
        return None;
    }
    let (sheet, reference) = match raw.rfind('!') {
        Some(at) => {
            let name = unquote_sheet_name(&raw[..at]);
            let sheet = (!name.is_empty()).then_some(name);
            (sheet, raw[at + 1..].trim())
        }
        None => (None, raw),
    };
    match CellRef::from_a1(reference).or_else(|| CellRange::from_a1(reference).map(|r| r.start)) {
        Some(cell) => Some(InternalTarget::Cell { sheet, cell }),
        None => Some(InternalTarget::DefinedName(raw.to_string())),
    }
}

/// Undoes Excel's `'…'` sheet-name quoting (`''` is an escaped apostrophe).
fn unquote_sheet_name(raw: &str) -> String {
    let name = raw.trim();
    match name
        .strip_prefix('\'')
        .and_then(|inner| inner.strip_suffix('\''))
    {
        Some(inner) => inner.replace("''", "'"),
        None => name.to_string(),
    }
}

/// The hover text for a link cell (F2.4): the link's stored ScreenTip on its own first line when it
/// has one, then the destination — so the target is always visible, tooltip or not.
pub fn link_tooltip_text(target: &LinkTarget) -> String {
    match target.tooltip().map(str::trim).filter(|t| !t.is_empty()) {
        Some(tip) => format!("{tip}\n{}", target.destination()),
        None => target.destination().to_string(),
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

    // ---- `decide_open` (F3) ------------------------------------------------------------------

    fn decide(target: &str) -> OpenDecision {
        decide_open(&LinkTarget::external(target))
    }

    /// The refused scheme, or the empty string for "no usable scheme"; panics on a non-refusal, so
    /// a test that expects a refusal can never silently pass on an `Open`.
    fn refused_scheme(target: &str) -> String {
        match decide(target) {
            OpenDecision::Refuse { scheme } => scheme,
            other => panic!("{target:?} must be refused, got {other:?}"),
        }
    }

    #[test]
    fn decide_open_allows_the_five_safe_schemes() {
        for target in [
            "http://example.com/a?b=1#c",
            "https://example.com",
            "mailto:someone@example.com",
            "ftp://files.example.com/x.zip",
            "ftps://files.example.com/x.zip",
        ] {
            assert_eq!(
                decide(target),
                OpenDecision::Open(target.to_string()),
                "{target} is an allowed scheme"
            );
        }
        // Scheme matching is case-insensitive, and the target opens in its ORIGINAL case.
        assert_eq!(
            decide("HTTPS://Example.COM/Path"),
            OpenDecision::Open("HTTPS://Example.COM/Path".to_string())
        );
        // Surrounding whitespace is trimmed, and the trimmed form is what would be launched.
        assert_eq!(
            decide("  https://example.com \n"),
            OpenDecision::Open("https://example.com".to_string())
        );
    }

    #[test]
    fn decide_open_confirms_file_urls() {
        // A `file:` link reaches the local filesystem on the say-so of a downloaded document, so it
        // asks first (F3.2) — never a bare `Open`.
        assert_eq!(
            decide("file:///etc/passwd"),
            OpenDecision::ConfirmFile("file:///etc/passwd".to_string())
        );
        assert_eq!(
            decide("FILE:///C:/secrets.txt"),
            OpenDecision::ConfirmFile("FILE:///C:/secrets.txt".to_string())
        );
    }

    #[test]
    fn decide_open_refuses_hostile_targets() {
        // The `architecture.md §6` table: every one of these is reachable from an opened .xlsx.
        assert_eq!(refused_scheme("javascript:alert(1)"), "javascript");
        assert_eq!(refused_scheme("JaVaScRiPt:alert(1)"), "javascript");
        assert_eq!(refused_scheme("  javascript:alert(1)"), "javascript");
        assert_eq!(refused_scheme("data:text/html,<script>x</script>"), "data");
        assert_eq!(refused_scheme("vbscript:msgbox(1)"), "vbscript");
        assert_eq!(refused_scheme("smb://host/share"), "smb");
        // No usable scheme: scheme-relative, a bare path, and a Windows drive letter (one letter is
        // not read as a scheme, so the message says "no scheme" rather than naming `c`).
        assert_eq!(refused_scheme("//evil.com/payload"), "");
        assert_eq!(refused_scheme("/usr/bin/whatever"), "");
        assert_eq!(refused_scheme("C:\\Windows\\notepad.exe"), "");
        assert_eq!(refused_scheme(""), "");
    }

    #[test]
    fn decide_open_refuses_a_target_with_a_control_character() {
        // A smuggled newline must not reach the platform opener as a second line, even though the
        // target's visible prefix is an allowed scheme.
        assert_eq!(refused_scheme("http://ok.example\njavascript:alert(1)"), "");
        assert_eq!(refused_scheme("https://ok.example\r\nx"), "");
        assert_eq!(refused_scheme("https://ok.example\u{0}x"), "");
    }

    #[test]
    fn decide_open_is_never_reached_for_internal_targets() {
        // Nothing launches an in-workbook location (F4.4). The caller routes it to navigation; this
        // is the backstop if one ever forgets.
        assert_eq!(
            decide_open(&LinkTarget::internal("Sheet1!A30")),
            OpenDecision::Refuse {
                scheme: String::new()
            }
        );
    }

    // ---- The click gesture (F2.1–F2.2) -------------------------------------------------------

    #[test]
    fn link_click_opens_only_on_a_plain_single_click() {
        let plain = LinkClick {
            same_cell: true,
            dragged: false,
            primary_modifier: false,
            click_count: 1,
        };
        assert!(link_click_opens(plain), "a plain single click opens (F2.1)");
        assert!(
            !link_click_opens(LinkClick {
                same_cell: false,
                ..plain
            }),
            "releasing on another cell is a selection, not an open"
        );
        assert!(
            !link_click_opens(LinkClick {
                dragged: true,
                ..plain
            }),
            "a drag is a range sweep, not an open"
        );
        assert!(
            !link_click_opens(LinkClick {
                primary_modifier: true,
                ..plain
            }),
            "the primary modifier is the select-without-opening affordance"
        );
        assert!(
            !link_click_opens(LinkClick {
                click_count: 2,
                ..plain
            }),
            "a double click opens the in-cell editor"
        );
    }

    // ---- Internal locations (F4) -------------------------------------------------------------

    fn cell_target(sheet: Option<&str>, a1: &str) -> InternalTarget {
        InternalTarget::Cell {
            sheet: sheet.map(str::to_string),
            cell: CellRef::from_a1(a1).expect("test ref parses"),
        }
    }

    #[test]
    fn parse_internal_location_reads_the_shapes_excel_writes() {
        let parse = |s: &str| parse_internal_location(s).expect("non-empty location parses");
        assert_eq!(parse("Sheet1!A30"), cell_target(Some("Sheet1"), "A30"));
        assert_eq!(parse("'My Sheet'!B2"), cell_target(Some("My Sheet"), "B2"));
        // `''` is Excel's escaped apostrophe inside a quoted sheet name.
        assert_eq!(parse("'It''s'!A1"), cell_target(Some("It's"), "A1"));
        // No sheet part → the active sheet.
        assert_eq!(parse("A30"), cell_target(None, "A30"));
        // A `#` prefix (what `HYPERLINK("#…")` and xlsx write) is accepted and stripped.
        assert_eq!(parse("#Sheet1!A30"), cell_target(Some("Sheet1"), "A30"));
        // A range lands on its top-left, however the corners were ordered.
        assert_eq!(parse("Sheet1!A1:C3"), cell_target(Some("Sheet1"), "A1"));
        assert_eq!(parse("Sheet1!C3:A1"), cell_target(Some("Sheet1"), "A1"));
        // Absolute markers are ignored, as everywhere else in FreeCell.
        assert_eq!(parse("Sheet1!$A$1"), cell_target(Some("Sheet1"), "A1"));
        // An empty location is the one `None`.
        assert_eq!(parse_internal_location("   "), None);
        assert_eq!(parse_internal_location("#"), None);
    }

    #[test]
    fn parse_internal_location_classifies_a_non_reference_as_a_defined_name() {
        assert_eq!(
            parse_internal_location("MyRange"),
            Some(InternalTarget::DefinedName("MyRange".into()))
        );
        // A sheet-qualified non-reference is not a cell either; the whole location is reported so
        // the "can't resolve this" message can name what the file actually said.
        assert_eq!(
            parse_internal_location("Sheet1!zzz"),
            Some(InternalTarget::DefinedName("Sheet1!zzz".into()))
        );
    }

    #[test]
    fn link_tooltip_text_shows_the_screentip_above_the_target() {
        assert_eq!(
            link_tooltip_text(&LinkTarget::external("https://example.com")),
            "https://example.com"
        );
        assert_eq!(
            link_tooltip_text(
                &LinkTarget::external("https://example.com").with_tooltip(Some("Our docs".into()))
            ),
            "Our docs\nhttps://example.com"
        );
        // A blank ScreenTip is not a line of its own.
        assert_eq!(
            link_tooltip_text(
                &LinkTarget::external("https://example.com").with_tooltip(Some("  ".into()))
            ),
            "https://example.com"
        );
        assert_eq!(
            link_tooltip_text(
                &LinkTarget::internal("Sheet1!A30").with_tooltip(Some("Jump".into()))
            ),
            "Jump\nSheet1!A30"
        );
    }
}
