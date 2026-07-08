//! Font registration hook (`components/app_shell.md §Structure`, `ui_design.md §3.3`).
//!
//! The app bundles **Inter** (SIL OFL, static faces under `assets/fonts/inter/`) and registers it
//! via `cx.text_system().add_fonts(...)` at startup, then sets it as the app UI font so the grid +
//! gpui-component chrome render in one predictable family. This also fixes a real cross-platform
//! bug: on Linux the GPUI default UI font resolves to a single regular face, so bold/italic
//! silently render as regular — the vendored Inter has real Bold/Italic faces, so styled runs
//! render correctly and identically on macOS and Linux.
//!
//! Beyond the four RIBBI faces (Regular / Bold / Italic / Bold Italic) the bundle now also carries
//! three **non-RIBBI** static faces used by the About window's identity block: **Inter Medium**
//! (tagline) and **Inter SemiBold** (links) from the "Inter" typographic family, plus **Inter
//! Display ExtraBold** from the tighter, higher-contrast **"Inter Display"** family (the wordmark).
//! These non-RIBBI faces register under platform-specific family names — see
//! [`wordmark_font`]/[`medium_font`]/[`semibold_font`] for the resolution trap and how it is
//! handled.
//!
//! Registration is **best-effort**: if `add_fonts` fails (unexpected on the bundled bytes) the
//! function logs a warning and returns without setting the UI font, so the app falls back to
//! GPUI's default font rather than panicking. See `projects/bundled-inter-font.md`.

use std::borrow::Cow;

use gpui::{App, FontWeight};

use crate::grid::GRID_FONT_FAMILY;

/// The bundled Inter faces (family name "Inter"), embedded in the binary so no external font
/// package is required. Static RIBBI faces (not the variable font) for deterministic
/// weight/italic resolution, and **TrueType (`glyf`) outlines, not OpenType/CFF (`.otf`)** —
/// macOS registers embedded fonts via `CGFont::from_data_provider`, which fails on CFF, so an
/// `.otf` here would make `add_fonts` error and the whole UI fall back to the system font
/// (Linux's loader tolerates CFF, so it would look fine there and break only on macOS).
const INTER_REGULAR: &[u8] = include_bytes!("../../assets/fonts/inter/Inter-Regular.ttf");
const INTER_BOLD: &[u8] = include_bytes!("../../assets/fonts/inter/Inter-Bold.ttf");
const INTER_ITALIC: &[u8] = include_bytes!("../../assets/fonts/inter/Inter-Italic.ttf");
const INTER_BOLD_ITALIC: &[u8] = include_bytes!("../../assets/fonts/inter/Inter-BoldItalic.ttf");
/// Non-RIBBI static Inter weights (Medium 500, SemiBold 600) + the Display cut's ExtraBold (800),
/// bundled for the About window's identity block. Same **TrueType (`glyf`)** constraint as above
/// — all three are `glyf` outlines, so `CGFont::from_data_provider` accepts them on macOS.
const INTER_MEDIUM: &[u8] = include_bytes!("../../assets/fonts/inter/Inter-Medium.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../../assets/fonts/inter/Inter-SemiBold.ttf");
const INTER_DISPLAY_EXTRA_BOLD: &[u8] =
    include_bytes!("../../assets/fonts/inter/InterDisplay-ExtraBold.ttf");

/// Registers the bundled Inter faces with the text system and sets Inter as the app UI font
/// (both the grid and the gpui-component chrome), so rendering is one predictable family across
/// platforms. Call once before the first window opens, **after** `gpui_component::init(cx)`.
///
/// Best-effort: if registration fails, logs a warning and returns without changing the UI font,
/// leaving the app on GPUI's default font (the documented "bundle absent → default font"
/// fallback). Never panics.
pub fn register_fonts(cx: &mut App) {
    if let Err(err) = cx.text_system().add_fonts(vec![
        Cow::Borrowed(INTER_REGULAR),
        Cow::Borrowed(INTER_BOLD),
        Cow::Borrowed(INTER_ITALIC),
        Cow::Borrowed(INTER_BOLD_ITALIC),
        Cow::Borrowed(INTER_MEDIUM),
        Cow::Borrowed(INTER_SEMIBOLD),
        Cow::Borrowed(INTER_DISPLAY_EXTRA_BOLD),
    ]) {
        tracing::warn!(
            error = %err,
            "register_fonts: failed to register bundled Inter faces; falling back to the default UI font"
        );
        return;
    }

    // Point the gpui-component theme (which drives the chrome, and which the grid also names
    // explicitly at its text sites) at Inter so everything renders in one family.
    gpui_component::Theme::global_mut(cx).font_family = GRID_FONT_FAMILY.into();

    tracing::debug!("register_fonts: registered bundled Inter faces; UI font set to Inter");
}

// ---- About-window face resolution (the static-font naming trap) ---------------------------------
//
// Inter's static, non-RIBBI faces carry TWO family names in their `name` table: the **typographic
// family** (name ID 16 — "Inter" / "Inter Display", shared with the RIBBI faces, with the real
// weight in ID 17) *and* a **legacy per-weight family** (name ID 1 — e.g. "Inter Medium", "Inter
// SemiBold", "Inter Display ExtraBold", each carrying subfamily "Regular"). Which name a face is
// reachable under depends on how the platform's font backend reads that table, so there is NO
// single `.font_family(...)` string that resolves the target face on both platforms:
//
//   * **macOS** (Core Text / font-kit): `CTFontCopyFamilyName` on a `CGFont` built from our bytes
//     returns name **ID 1**, so each non-RIBBI face lands in its own legacy family as a lone
//     "Regular" — you reach it by that legacy family name (weight is then almost moot, the family
//     has one face). `.font_family("Inter").font_weight(EXTRA_BOLD)` would silently fall back to
//     the RIBBI Bold, so we must name the legacy family explicitly.
//   * **Linux** (cosmic-text / fontdb): fontdb prefers name **ID 16**, so these faces join the
//     "Inter" / "Inter Display" typographic families as genuine extra weights — you reach them by
//     family + weight (`"Inter"` @ 500/600, `"Inter Display"` @ 800).
//
// So each helper picks the family string per platform via `cfg!` (both arms are compiled and
// type-checked on every target — no `#[cfg]` dead-code trap for the macOS strings we can't build
// here). The **non-macOS** (fontdb) branch is verified empirically by the test below
// (`fontdb_backend_resolves_each_about_face_by_family_and_weight`), which loads the bundled bytes
// into a `fontdb::Database` and asserts each family + weight resolves to the exact face by
// PostScript name (not a Regular/Bold fallback). The macOS mapping is fixed by the ID-1 legacy
// names dumped from the same `name` tables at bundling time.

/// The `(family, weight)` that resolves to the bundled **Inter Display ExtraBold** face — the
/// About wordmark. This is a genuinely heavier & tighter cut than the RIBBI Bold.
pub(crate) fn wordmark_font() -> (&'static str, FontWeight) {
    if cfg!(target_os = "macos") {
        ("Inter Display ExtraBold", FontWeight::EXTRA_BOLD)
    } else {
        ("Inter Display", FontWeight::EXTRA_BOLD)
    }
}

/// The `(family, weight)` that resolves to the bundled **Inter Medium** face — the About tagline.
pub(crate) fn medium_font() -> (&'static str, FontWeight) {
    if cfg!(target_os = "macos") {
        ("Inter Medium", FontWeight::MEDIUM)
    } else {
        ("Inter", FontWeight::MEDIUM)
    }
}

/// The `(family, weight)` that resolves to the bundled **Inter SemiBold** face — the About links.
pub(crate) fn semibold_font() -> (&'static str, FontWeight) {
    if cfg!(target_os = "macos") {
        ("Inter SemiBold", FontWeight::SEMIBOLD)
    } else {
        ("Inter", FontWeight::SEMIBOLD)
    }
}

#[cfg(test)]
mod tests {
    //! Empirical verification of the About-window face resolution.
    //!
    //! gpui's `#[gpui::test]` harness installs a `NoopTextSystem` — `add_fonts` is a no-op,
    //! `all_font_names` is empty, and every `resolve_font` returns the same stub `FontId` — so a
    //! gpui test cannot observe real registration. Instead we drive **fontdb** directly, the exact
    //! crate gpui's Linux (cosmic-text) text system loads fonts into and queries, with the bundled
    //! bytes. This proves each target face resolves under the family + weight the **non-macOS**
    //! branch of [`wordmark_font`]/[`medium_font`]/[`semibold_font`] uses. The macOS branch is
    //! fixed by the ID-1 legacy family names carried in the same `name` tables.
    use super::*;
    use fontdb::{Database, Family, Query, Weight};

    /// A fontdb database loaded with the RIBBI faces *and* the three non-RIBBI faces, so "Inter"
    /// models the real registered family (a weight query must genuinely discriminate Medium /
    /// SemiBold from Regular / Bold, not just find the only face).
    fn database() -> Database {
        let mut db = Database::new();
        for bytes in [
            INTER_REGULAR,
            INTER_BOLD,
            INTER_MEDIUM,
            INTER_SEMIBOLD,
            INTER_DISPLAY_EXTRA_BOLD,
        ] {
            db.load_font_data(bytes.to_vec());
        }
        db
    }

    /// The unambiguous PostScript name of the face fontdb resolves for `family` @ `weight`.
    fn resolved_postscript(db: &Database, family: &str, weight: Weight) -> String {
        let id = db
            .query(&Query {
                families: &[Family::Name(family)],
                weight,
                ..Query::default()
            })
            .unwrap_or_else(|| panic!("fontdb found no face for `{family}` @ {}", weight.0));
        db.face(id).unwrap().post_script_name.clone()
    }

    #[test]
    fn fontdb_backend_resolves_each_about_face_by_family_and_weight() {
        let db = database();

        // fontdb prefers the typographic family (name ID 16), so the non-RIBBI faces join the
        // "Inter" / "Inter Display" families rather than legacy per-weight families.
        let families: Vec<&str> = db
            .faces()
            .flat_map(|f| f.families.iter().map(|(name, _)| name.as_str()))
            .collect();
        assert!(
            families.contains(&"Inter"),
            "Inter typographic family present: {families:?}"
        );
        assert!(
            families.contains(&"Inter Display"),
            "Inter Display family present: {families:?}"
        );

        // The exact face each About element renders on the Linux backend.
        assert_eq!(
            resolved_postscript(&db, "Inter Display", Weight::EXTRA_BOLD),
            "InterDisplay-ExtraBold",
            "wordmark resolves to the Display ExtraBold cut"
        );
        assert_eq!(
            resolved_postscript(&db, "Inter", Weight::MEDIUM),
            "Inter-Medium",
            "tagline resolves to Inter Medium"
        );
        assert_eq!(
            resolved_postscript(&db, "Inter", Weight::SEMIBOLD),
            "Inter-SemiBold",
            "links resolve to Inter SemiBold"
        );
        // Guard against a silent weight fallback: the base weights still map to the RIBBI faces,
        // proving Medium/SemiBold above are genuine distinct faces, not the nearest RIBBI weight.
        assert_eq!(
            resolved_postscript(&db, "Inter", Weight::NORMAL),
            "Inter-Regular"
        );
        assert_eq!(
            resolved_postscript(&db, "Inter", Weight::BOLD),
            "Inter-Bold"
        );
    }
}
