//! Font registration hook (`components/app_shell.md §Structure`, `ui_design.md §3.3`).
//!
//! The design bundles **Inter** (regular / bold / italic / bold-italic) and registers it via
//! `cx.text_system().add_fonts(...)` **before any window opens**, so the grid + chrome render
//! in one predictable family across platforms.
//!
//! The Inter `.ttf` files are **not vendored yet** (see DECISIONS_TO_REVIEW — the Phase-6/7
//! render baselines were captured on gpui's default UI font, and Phase 13 owns the final font
//! pass). Until the files land under `assets/fonts/`, this is the seam that will load them:
//! it looks for the bundled bytes and, when absent, leaves gpui's default UI font in place
//! (a graceful, non-fatal fallback — the app still runs and renders).

use gpui::App;

/// Registers the app's bundled fonts. Best-effort: if the Inter bundle isn't vendored, the
/// default UI font is kept and a debug line is logged. Call once at startup before the first
/// window opens.
pub fn register_fonts(_cx: &mut App) {
    // Placeholder for the Inter bundle (`include_bytes!("../../assets/fonts/Inter-*.ttf")`).
    // Vendoring the four TTFs + flipping this to `cx.text_system().add_fonts(...)` is the
    // Phase-13 font pass; wiring it here now would change the committed render baselines,
    // which were generated on the default font.
    tracing::debug!(
        "register_fonts: bundled Inter not vendored yet; using the default UI font \
         (see DECISIONS_TO_REVIEW, Phase 10)"
    );
}
