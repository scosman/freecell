//! The application's combined [`AssetSource`] (`ui_design.md §1.1` action-bar icons).
//!
//! gpui takes exactly **one** [`AssetSource`] via `application().with_assets(...)`, and the app
//! needs assets from two places:
//!
//! - **`gpui_component_assets::Assets`** — the pinned gpui-component icon bundle (a curated ~99
//!   icon Lucide subset). gpui-component's own [`IconName`](gpui_component::IconName) (e.g.
//!   `IconName::Loader`, used by the grid loading overlay, and `IconName::ChevronDown`, used by
//!   Button's dropdown caret) resolves out of this bundle, so it MUST keep resolving.
//! - **FreeCell's own vendored icons** (`assets/icons/*.svg`) — a small set of Lucide typography
//!   / formatting glyphs (bold, italic, alignment, decimals, …) that the bundle does **not**
//!   ship. Rather than repin gpui-component, we vendor exactly the icons the action bar needs
//!   and compose them here at the asset layer.
//!
//! [`AppAssets`] is that composition: it resolves a FreeCell-owned icon first, then falls back to
//! the gpui-component bundle. FreeCell icon paths (`icons/<name>.svg`) are disjoint from the
//! bundle's names, so no bundle asset is ever shadowed. The action bar renders each icon with
//! gpui-component's `Icon` component via `Icon::empty().path("icons/<name>.svg")`.

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

/// The FreeCell-vendored Lucide icons, each embedded by its bundle-relative asset path — the same
/// `icons/<name>.svg` string passed to `Icon::path(...)`. Vendored under `assets/icons/` (ISC /
/// Feather-MIT — see `assets/icons/LICENSE`) in the same tintable `stroke="currentColor"` form the
/// gpui-component bundle uses, so `Icon` tints them to each button's foreground exactly like the
/// bundled icons. Keep this list in sync with the files in `assets/icons/`.
const FREECELL_ICONS: &[(&str, &[u8])] = &[
    (
        "icons/arrow-down-from-line.svg",
        include_bytes!("../../assets/icons/arrow-down-from-line.svg"),
    ),
    (
        "icons/arrow-up-to-line.svg",
        include_bytes!("../../assets/icons/arrow-up-to-line.svg"),
    ),
    (
        "icons/baseline.svg",
        include_bytes!("../../assets/icons/baseline.svg"),
    ),
    (
        "icons/bold.svg",
        include_bytes!("../../assets/icons/bold.svg"),
    ),
    // Chart-type glyphs for the action-bar insert menu (P17). Lucide chart icons in the same
    // tintable form; `chart-doughnut` is a hand-authored ring (Lucide ships no doughnut glyph).
    (
        "icons/chart-area.svg",
        include_bytes!("../../assets/icons/chart-area.svg"),
    ),
    (
        "icons/chart-bar.svg",
        include_bytes!("../../assets/icons/chart-bar.svg"),
    ),
    (
        "icons/chart-column.svg",
        include_bytes!("../../assets/icons/chart-column.svg"),
    ),
    (
        "icons/chart-doughnut.svg",
        include_bytes!("../../assets/icons/chart-doughnut.svg"),
    ),
    (
        "icons/chart-line.svg",
        include_bytes!("../../assets/icons/chart-line.svg"),
    ),
    (
        "icons/chart-pie.svg",
        include_bytes!("../../assets/icons/chart-pie.svg"),
    ),
    (
        "icons/chart-scatter.svg",
        include_bytes!("../../assets/icons/chart-scatter.svg"),
    ),
    (
        "icons/decimals-arrow-left.svg",
        include_bytes!("../../assets/icons/decimals-arrow-left.svg"),
    ),
    (
        "icons/decimals-arrow-right.svg",
        include_bytes!("../../assets/icons/decimals-arrow-right.svg"),
    ),
    (
        "icons/grid-2x2.svg",
        include_bytes!("../../assets/icons/grid-2x2.svg"),
    ),
    (
        "icons/italic.svg",
        include_bytes!("../../assets/icons/italic.svg"),
    ),
    (
        "icons/paint-bucket.svg",
        include_bytes!("../../assets/icons/paint-bucket.svg"),
    ),
    (
        "icons/separator-horizontal.svg",
        include_bytes!("../../assets/icons/separator-horizontal.svg"),
    ),
    (
        "icons/strikethrough.svg",
        include_bytes!("../../assets/icons/strikethrough.svg"),
    ),
    (
        "icons/text-align-center.svg",
        include_bytes!("../../assets/icons/text-align-center.svg"),
    ),
    (
        "icons/text-align-end.svg",
        include_bytes!("../../assets/icons/text-align-end.svg"),
    ),
    (
        "icons/text-align-start.svg",
        include_bytes!("../../assets/icons/text-align-start.svg"),
    ),
    (
        "icons/text-wrap.svg",
        include_bytes!("../../assets/icons/text-wrap.svg"),
    ),
    (
        "icons/underline.svg",
        include_bytes!("../../assets/icons/underline.svg"),
    ),
];

/// The app's asset source: FreeCell-vendored icons composed over the gpui-component bundle.
///
/// A zero-sized handle (like `gpui_component_assets::Assets`) suitable for
/// `application().with_assets(AppAssets)`.
#[derive(Clone, Copy, Debug, Default)]
pub struct AppAssets;

impl AppAssets {
    /// The embedded bytes for a FreeCell-vendored icon at `path`, if one exists.
    fn freecell_icon(path: &str) -> Option<&'static [u8]> {
        FREECELL_ICONS
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(_, bytes)| *bytes)
    }
}

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        // FreeCell-owned icons win, then fall back to the gpui-component bundle (which owns
        // everything the bundled `IconName`s resolve, e.g. `Loader` / `ChevronDown`). The two
        // namespaces are disjoint, so this order never shadows a bundle asset.
        if let Some(bytes) = Self::freecell_icon(path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        gpui_component_assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut items = gpui_component_assets::Assets.list(path)?;
        items.extend(
            FREECELL_ICONS
                .iter()
                .filter(|(p, _)| p.starts_with(path))
                .map(|(p, _)| SharedString::from(*p)),
        );
        Ok(items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every vendored icon path resolves to non-empty SVG bytes that are `currentColor`-tintable:
    /// `stroke="currentColor"` is present AND there is no hardcoded `fill="#…"` / `fill="rgb…"`
    /// that would override the theme tint (so `Icon` tints them like the bundled icons).
    #[test]
    fn vendored_icons_load_and_are_tintable() {
        for (path, _) in FREECELL_ICONS {
            let bytes = AppAssets
                .load(path)
                .expect("load ok")
                .unwrap_or_else(|| panic!("missing vendored icon {path}"));
            assert!(!bytes.is_empty(), "empty icon {path}");
            let svg = std::str::from_utf8(&bytes).expect("utf8 svg");
            assert!(
                svg.contains("stroke=\"currentColor\""),
                "{path} is not currentColor-tintable"
            );
            let lower = svg.to_ascii_lowercase();
            assert!(
                !lower.contains("fill=\"#") && !lower.contains("fill=\"rgb"),
                "{path} has a hardcoded fill that would override the theme tint"
            );
        }
    }

    /// The bundle still resolves through the combined source (regression guard for
    /// `IconName::Loader` / `ChevronDown`, which the grid overlay + dropdown carets need).
    #[test]
    fn bundle_still_resolves_through_combined_source() {
        let loader = AppAssets.load("icons/loader.svg").expect("load ok");
        assert!(
            loader.is_some(),
            "gpui-component bundle icon must still resolve"
        );
    }

    /// A path in neither FreeCell's icons nor the bundle delegates straight through to the
    /// bundle, which reports a missing asset as `Err` — the pre-existing `gpui_component_assets`
    /// behavior the app already relied on (the combined source doesn't change it).
    #[test]
    fn unknown_path_delegates_to_bundle_err() {
        assert!(AppAssets.load("icons/does-not-exist.svg").is_err());
    }

    /// `list` surfaces both the bundle's icons and the vendored ones under `icons/`.
    #[test]
    fn list_merges_bundle_and_vendored() {
        let listed = AppAssets.list("icons/").expect("list ok");
        assert!(
            listed.iter().any(|s| s.as_ref() == "icons/bold.svg"),
            "vendored icon missing from list"
        );
        assert!(
            listed.iter().any(|s| s.as_ref() == "icons/loader.svg"),
            "bundle icon missing from list"
        );
    }
}
