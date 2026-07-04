//! Font registration hook (`components/app_shell.md §Structure`, `ui_design.md §3.3`).
//!
//! The design *intends* to bundle **Inter** and register it via `cx.text_system().add_fonts(...)`
//! before any window opens, so the grid + chrome render in one predictable family. **The MVP
//! does not do this** — bundling Inter was a conscious Phase-13 deferral: the render-baseline
//! stability the spec cites Inter for is already delivered by pinning the render-suite runner
//! image, and changing the render font at the finish line would mean regenerating + re-eyeballing
//! all 48 pixel baselines. The app runs on GPUI's **default UI font**.
//!
//! This function is therefore a **no-op** at present (it registers nothing). It is kept as the
//! single seam a future font pass flips on — see `PROJECTS.md` → `projects/bundled-inter-font.md`
//! and `DECISIONS_TO_REVIEW.md` (Phase 13). It intentionally makes no false "fonts registered"
//! claim: nothing is added, so callers get the default font.

use gpui::App;

/// No-op today: the MVP ships on GPUI's default UI font (bundled Inter deferred — see the module
/// doc and `projects/bundled-inter-font.md`). Kept as the startup seam a future font pass flips to
/// `cx.text_system().add_fonts(...)`; call once before the first window opens.
pub fn register_fonts(_cx: &mut App) {
    // A future font pass vendors the Inter faces under `assets/fonts/` and replaces this body with
    // `cx.text_system().add_fonts(...)`. Doing so requires regenerating + eyeballing all render
    // baselines (they were captured on the default font), so it is deliberately NOT done in MVP.
    tracing::debug!(
        "register_fonts: no-op (MVP ships on the default UI font; bundled Inter deferred — \
         see projects/bundled-inter-font.md)"
    );
}
