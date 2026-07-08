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
//! three **non-RIBBI** static faces used by the About/Welcome identity block: **Inter Medium**
//! (tagline), **Inter SemiBold** (links), and the tighter, higher-contrast **Inter Display
//! ExtraBold** (wordmark).
//!
//! These three files were **rewritten at asset-prep time** (a one-off fontTools pass, not part of
//! the build) so each is a clean **single-face family**: its legacy family (name ID 1) and
//! typographic family (name ID 16) are the SAME string — [`WORDMARK_FAMILY`] /
//! [`TAGLINE_FAMILY`] / [`LINK_FAMILY`] — with subfamily "Regular". That makes each face resolve
//! by one unambiguous `.font_family(...)` name on BOTH gpui backends (font-kit/CoreText on macOS
//! reads name ID 1; cosmic-text/fontdb on Linux prefers name ID 16), so there is no per-platform
//! `cfg!` and no fragile weight-matching. `OS/2.usWeightClass` is left authentic (Medium 500,
//! SemiBold 600, ExtraBold 800). See [`WORDMARK_FAMILY`] and the fontdb test below.
//!
//! Registration is **best-effort**: if `add_fonts` fails (unexpected on the bundled bytes) the
//! function logs a warning and returns without setting the UI font, so the app falls back to
//! GPUI's default font rather than panicking. See `projects/bundled-inter-font.md`.

use std::borrow::Cow;

use gpui::App;

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

// ---- About/Welcome identity-block face families -------------------------------------------------
//
// The three non-RIBBI faces were rewritten (asset-prep) into clean single-face families whose
// legacy (name ID 1) and typographic (name ID 16) family names are IDENTICAL to the strings below.
// Because both gpui backends read one of those two IDs — CoreText/font-kit reads ID 1 on macOS,
// fontdb prefers ID 16 on Linux — and here they are the same string, a single `.font_family(...)`
// name resolves the exact face on every platform. No `cfg!`, no weight-matching: the family has one
// face, so it resolves regardless of the requested weight. The fontdb (Linux) backend is verified
// empirically by the test below; the macOS backend matches the same string because ID 1 == ID 16.

/// Family name of the bundled **Inter Display ExtraBold** face — the About/Welcome wordmark. A
/// genuinely heavier & tighter cut than the RIBBI Bold.
pub(crate) const WORDMARK_FAMILY: &str = "Inter Display ExtraBold";

/// Family name of the bundled **Inter Medium** face — the About tagline.
pub(crate) const TAGLINE_FAMILY: &str = "Inter Medium";

/// Family name of the bundled **Inter SemiBold** face — the About links.
pub(crate) const LINK_FAMILY: &str = "Inter SemiBold";

#[cfg(test)]
mod tests {
    //! Empirical verification of the About-window face resolution.
    //!
    //! gpui's `#[gpui::test]` harness installs a `NoopTextSystem` — `add_fonts` is a no-op,
    //! `all_font_names` is empty, and every `resolve_font` returns the same stub `FontId` — so a
    //! gpui test cannot observe real registration. Instead we drive **fontdb** directly, the exact
    //! crate gpui's Linux (cosmic-text) text system loads fonts into and queries, with the bundled
    //! bytes. This proves each identity-block face resolves by its single-face family name
    //! ([`WORDMARK_FAMILY`] / [`TAGLINE_FAMILY`] / [`LINK_FAMILY`]) on the Linux backend. The macOS
    //! backend matches the SAME string because each face's name ID 1 == name ID 16 (asset-prep).
    use super::*;
    use fontdb::{Database, Family, Query, Weight};

    /// A fontdb database loaded with the RIBBI faces *and* the three rewritten single-face faces, so
    /// a family query must genuinely discriminate them from the RIBBI "Inter" family, not just find
    /// the only face present.
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
    fn fontdb_backend_resolves_each_about_face_by_family() {
        let db = database();

        // Each rewritten face is now its own single-face family registered under its target name.
        let families: Vec<&str> = db
            .faces()
            .flat_map(|f| f.families.iter().map(|(name, _)| name.as_str()))
            .collect();
        for fam in [WORDMARK_FAMILY, TAGLINE_FAMILY, LINK_FAMILY] {
            assert!(
                families.contains(&fam),
                "single-face family `{fam}` present: {families:?}"
            );
        }

        // The exact face each identity element renders — resolved by family name alone. Querying at
        // the default NORMAL weight proves resolution no longer depends on weight-matching (the
        // family has one face); this is the same string CoreText reads from name ID 1 on macOS.
        assert_eq!(
            resolved_postscript(&db, WORDMARK_FAMILY, Weight::NORMAL),
            "InterDisplayExtraBold",
            "wordmark resolves to the Display ExtraBold cut by family name alone"
        );
        assert_eq!(
            resolved_postscript(&db, TAGLINE_FAMILY, Weight::NORMAL),
            "InterMedium",
            "tagline resolves to Inter Medium by family name alone"
        );
        assert_eq!(
            resolved_postscript(&db, LINK_FAMILY, Weight::NORMAL),
            "InterSemiBold",
            "links resolve to Inter SemiBold by family name alone"
        );

        // The RIBBI "Inter" family is untouched: the three non-RIBBI faces are their own families
        // now, so "Inter" still resolves the genuine Regular/Bold RIBBI faces.
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
