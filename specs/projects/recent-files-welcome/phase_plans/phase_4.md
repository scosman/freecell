---
status: complete
---

# Phase 4: Standalone About window

## Overview

Turns the About screen from a modal overlay (hosted on the welcome / document windows) into a
standalone, single-instance **window** in the `freecell-app` shell, mirroring the existing
welcome-window plumbing. Implements `architecture.md §9`, `functional_spec.md §4`,
`ui_design.md §6`. A new `AboutView` renders the wordmark / tagline / version / links (opening
URLs in the browser via gpui `cx.open_url`); `FreeCellApp` opens/activates/tracks/closes it;
`registry` gains `about_open` accounting; the old About modal is removed from both the welcome
and document windows.

Design-time risk (`architecture.md §9.1`): `cx.open_url`. **Resolved** — `App::open_url(&str)`
is present at the pinned gpui rev (`crates/gpui/src/app.rs`) and the headless test platform
stubs it (records the URL, no panic), so no `open`-crate/`xdg-open` fallback is needed.

## Steps

1. **New `crates/freecell-app/src/shell/about.rs`.** `AboutView` — a `Render` + `Focusable`
   entity like `WelcomeView`, no state (just a `FocusHandle`). Module constants:
   - URLs: `HOMEPAGE_URL = "https://github.com/scosman/freecell"`,
     `IRONCALC_URL = "https://www.ironcalc.com"`, `GPUI_URL = "https://gpui.rs"`.
   - Homepage display label `"github.com/scosman/freecell"`.
   - `VERSION_LINE = concat!("Version ", env!("CARGO_PKG_VERSION"))`.
   - Palette tokens reused from the shared values: `CHROME_BG 0xF3F3F3` (window bg),
     `HAIRLINE 0xD9D9D9`, `TEXT 0x1F1F1F`, `MUTED_TEXT 0x555555`, plus the one new
     `LINK 0x2563EB` accent token (gpui-component's theme exposes no link/accent color at the
     pinned rev, so a local constant per `ui_design.md §6`).
   - `render`: `CHROME_BG` background, optional macOS titlebar row (`titlebar::MACOS_TITLEBAR
     .then(|| titlebar::titlebar_row("About FreeCell"))`), `track_focus` + `key_context("About")`
     + `CloseWindow` → `window.remove_window()` (mirrors welcome), then a padded vertical body:
     centered identity block (wordmark 28px bold `TEXT`, tagline 13px `MUTED_TEXT`, version 12px
     `MUTED_TEXT`), a 1px `HAIRLINE` rule, then two label→value rows (Homepage → homepage link;
     Built with → `IronCalc` · `GPUI` links with a `MUTED_TEXT` "·").
   - Each link: a `div().id(..).text_color(rgb(LINK)).cursor_pointer().hover(|s| s.underline())
     .on_click(move |_, _window, cx| cx.open_url(URL))`.
   - `#[cfg(test)]` accessors: `homepage_url`, `ironcalc_url`, `gpui_url`, `version`.

2. **`shell/mod.rs`.** Add `mod about;` (private, like `welcome`).

3. **`shell/app.rs`.**
   - Import `use super::about::AboutView;`.
   - Add fields `about: Option<Entity<AboutView>>`, `about_id: Option<WindowId>` (mirror
     `welcome`/`welcome_id`); initialize both to `None` in `init`.
   - `show_about(cx)` — drop the active-window lookup; just
     `cx.update_global(|app, cx| app.do_show_about(cx))`.
   - Rewrite `do_show_about(&mut self, cx)` to **open or activate** the About window (mirror
     `do_show_welcome`: activate `about_id` if set; else `open_window(about_window_options(cx), …)`
     hosting `AboutView` in a `Root`, store `about`/`about_id`, `registry.set_about_open(true)`).
     Remove the old modal routing (`ww.show_about` / `welcome.show_about`).
   - `on_window_closed`: add an `else if self.about_id == Some(window_id)` branch clearing
     `about`/`about_id` + `registry.set_about_open(false)`, falling through to the
     quit-when-empty check.
   - New `about_window_options(cx)`: `WindowBounds::centered(460×340)`, `titlebar_options()`,
     `is_resizable: false`, `is_minimizable: false`.
   - `#[cfg(test)]` accessors: `about_open(cx)`, `about_view(cx)`.

4. **`shell/registry.rs`.** Add `about_open: bool` field; `set_about_open`/`about_open`; include
   in `open_count` (`windows.len() + welcome + about`). Doc-comment the field like `welcome_open`.

5. **Remove the old About modal.**
   - `welcome.rs`: drop `WelcomeModal::About`, `WelcomeView::show_about`, and the About arm of
     `render_modal` (KEEP `Error`). Update the module doc to drop the "About dialog" mention.
   - `window.rs`: drop `ActiveModal::About`, `WorkbookWindow::show_about`, and the About arm of
     `render_modal` (KEEP UnsavedChanges + Error). Fix the "(or Error/About dismiss)" doc on
     `dismiss_modal` → "(or Error dismiss)". The "About is handled globally" comment stays
     accurate.

## Tests

- **`about.rs`** `about_view_exposes_link_urls_and_version` — build an `AboutView` in a test
  window; assert `homepage_url`/`ironcalc_url`/`gpui_url` equal the three spec URLs and
  `version() == env!("CARGO_PKG_VERSION")`.
- **`app.rs`** `about_action_opens_a_single_about_window` — `boot` + `show_about` ⇒ `about_open`
  true and `cx.windows().len() == 1`; a second `show_about` keeps it at one window
  (single-instance activate, no duplicate).
- **`app.rs`** `closing_the_last_about_window_quits` — open only the About window, then close it
  (drive `on_window_closed` via `remove_window` on the About window handle) ⇒ `about_open` false
  and the registry is empty (quit-accounting). Use `cx.test_quit`/observe as the harness allows;
  assert `open_count`/`is_empty` reached zero.
- **`registry.rs`** `about_counts_toward_open_count` — mirrors `welcome_counts_toward_open_count`.

## Render / smoke

Not part of the pixel render suite (About window is out of its scope, per `CLAUDE.md` + `§7`);
covered by the gpui view tests above + a headless `xvfb-run cargo run -p freecell-app` smoke
launch (confirm no panic; trigger About if practical). Do not touch baselines.
