//! Font registration hook (`components/app_shell.md §Structure`, `ui_design.md §3.3`).
//!
//! The app bundles **Inter** (SIL OFL, static RIBBI faces under `assets/fonts/inter/`) and
//! registers it via `cx.text_system().add_fonts(...)` at startup, then sets it as the app UI
//! font so the grid + gpui-component chrome render in one predictable family. This also fixes
//! a real cross-platform bug: on Linux the GPUI default UI font resolves to a single regular
//! face, so bold/italic silently render as regular — the vendored Inter has real Bold/Italic
//! faces, so styled runs render correctly and identically on macOS and Linux.
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
