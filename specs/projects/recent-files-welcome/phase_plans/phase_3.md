---
status: complete
---

# Phase 3: Welcome screen redesign (`freecell-app::shell::welcome`)

## Overview

Rebuilds the welcome window from a single centered column into the two-pane launch surface of
`ui_design.md §1–3` / `functional_spec.md §2`: a LEFT pane (FreeCell wordmark, the new tagline
**"The open spreadsheet"**, and the two existing buttons — now stacked full-width) and a RIGHT
pane (a `RECENT` header over up to 5 recent-file rows, or the text-only empty state). The rows
are **pure text** (name + `"{size} · {folder}"` subtitle + right-aligned relative time), whole-row
clickable → `FreeCellApp::open_path`, with a subtle hover.

The recents *logic* already lives in `freecell_core::recent` (Phase 1) and the app owns a live
`RecentList` + a `refresh_recents_ui` seam (Phase 2, which today rebuilds the menu only). This
phase adds `WelcomeView`'s `recents` state + `set_recents`, extends `refresh_recents_ui` with the
welcome branch, seeds the welcome on open, and grows the window to 720×480. It reuses the app's
palette tokens (`ui_design.md §0`) — no new hexes — per the "Design fidelity" note (mockups are
directional; build on our design system + gpui-component `Button`s).

Boundaries: the modal overlay (`render_modal`) stays working — **both** the `Error` and `About`
arms (the About modal is removed later, in Phase 4). No changes to `freecell-core` or `menus.rs`.

## Steps

1. **`shell/welcome.rs` — constants.** Realign the local look constants to the shared `§0` token
   *values* / names (matching the titlebar/chrome pattern): `BG` → `CHROME_BG = 0xF3F3F3` (left
   pane + row hover), `CARD_BG` → `ACTIVE_TAB_BG = 0xFFFFFF` (right pane + modal card); keep
   `HAIRLINE = 0xD9D9D9`, `TEXT = 0x1F1F1F`, `MUTED_TEXT = 0x555555`. Update the `render_modal`
   references accordingly. No `uppercase`/letter-spacing helpers exist at the pinned gpui rev, so
   the `RECENT` header is the literal uppercase string with no tracking (directional; fine).

2. **`shell/welcome.rs` — state + API.** Add `recents: Vec<DisplayEntry>` to `WelcomeView`
   (import `freecell_core::recent::DisplayEntry`); initialize `Vec::new()` in `new`. Add:
   ```rust
   /// Replaces the recent-file rows the right pane shows and repaints (`functional_spec.md §2.4`).
   pub fn set_recents(&mut self, recents: Vec<DisplayEntry>, cx: &mut Context<Self>) {
       self.recents = recents;
       cx.notify();
   }
   ```
   Plus `#[cfg(test)]` accessors: `recent_row_count(&self) -> usize` and `is_empty_state(&self) ->
   bool` (`self.recents.is_empty()`).

3. **`shell/welcome.rs` — row click action (deferred).** A whole-row click routes to
   `FreeCellApp::open_path`, but `open_path` re-enters the app global and can push fresh rows back
   into *this* view via `set_recents` (`refresh_recents_ui`) — a re-entrant self-update that panics.
   So defer it (mirrors `app.rs on_window_closed`'s `cx.defer`):
   ```rust
   fn open_recent(&self, index: usize, cx: &mut App) {
       if let Some(row) = self.recents.get(index) {
           let path = row.path.clone();
           cx.defer(move |cx| FreeCellApp::open_path(&path, cx));
       }
   }
   ```

4. **`shell/welcome.rs` — two-pane `render`.** Root: the optional titlebar row (unchanged), then a
   horizontal `flex_1` split — LEFT pane `w(px(264.))`, `bg(CHROME_BG)`, `border_r_1` `HAIRLINE`,
   ~32 px padding, vertical flex: wordmark (`28 px` bold `TEXT`) → tagline **"The open
   spreadsheet"** (`13 px` `MUTED_TEXT`) → gap → two `w_full` buttons stacked ~12 px apart
   (**New Spreadsheet** `.primary()` → `FreeCellApp::new_workbook`; **Open…** default →
   `FreeCellApp::open_via_panel`; wiring unchanged). RIGHT pane `flex_1`, `bg(ACTIVE_TAB_BG)`,
   ~24 px padding, vertical flex: `RECENT` header (`11 px` `MUTED_TEXT`, bottom margin) then the
   body. Keep `.children(self.render_modal(cx))` as the last child.
   - **Non-empty body:** a card (`border_1` `HAIRLINE`, `rounded`) of up to 5 rows; each row is a
     `div().id(("welcome-recent-row", i))` horizontal flex, ~56 px tall, ~14 px padding,
     `border_b_1` `HAIRLINE` except the last, `.hover(|s| s.bg(CHROME_BG))`, `.cursor_pointer()`,
     `.on_click(cx.listener(move |this, _, _w, cx| this.open_recent(i, cx)))`. Left column
     `flex_1().min_w_0()`: name (`14 px` semibold `TEXT`, `.truncate()`) over subtitle (`12 px`
     `MUTED_TEXT`, `.truncate()`); right column relative time (`12 px` `MUTED_TEXT`,
     `.flex_shrink_0()`). **No icons/glyphs.**
   - **Empty body:** centered, no glyph — "No recent spreadsheets" (`15 px` semibold `TEXT`) +
     "Create a new spreadsheet or open a file to get started." (`13 px` `MUTED_TEXT`, centered).

5. **`shell/app.rs` — window size.** `welcome_window_options`: `size(px(420.), px(300.))` →
   `size(px(720.), px(480.))`. Stays `is_resizable: false`, `is_minimizable: false`, centered,
   same `titlebar_options()`.

6. **`shell/app.rs` — live-update seam.** Extend `refresh_recents_ui` to push rows to the welcome
   before rebuilding the menu:
   ```rust
   if let Some(welcome) = self.welcome.clone() {
       let rows = self.recents.display_entries(recents::now_unix_secs(), WELCOME_LIMIT);
       welcome.update(cx, |w, cx| w.set_recents(rows, cx));
   }
   menus::install_menus_with(&self.recents, cx);
   ```
   Import `WELCOME_LIMIT` from `freecell_core::recent`. In `do_show_welcome`, after setting
   `self.welcome`/`welcome_id`/`welcome_open`, call `self.refresh_recents_ui(cx)` to seed the
   freshly-opened welcome with the current rows.

7. **`shell/app.rs` — test accessor.** `#[cfg(test)] fn welcome_view(cx: &App) ->
   Option<Entity<WelcomeView>>` returning `self.welcome.clone()`.

8. Run the gates from `/home/user/freecell/app` (`cargo fmt --all --check`, `cargo clippy
   --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo build
   --workspace`); iterate until clean. Run the render suite (`render_tests.sh test`) to confirm
   the grid baselines are unaffected (welcome isn't in it). Xvfb smoke-launch to confirm the
   redesigned welcome opens without panicking.

## Tests

**`welcome.rs` `#[cfg(test)]` (worker-less `TestAppContext`, `WelcomeView` in a test window):**

- `set_recents_reports_the_row_count` — `set_recents` with 2 `DisplayEntry`s ⇒ `recent_row_count()
  == 2` and `is_empty_state()` false.
- `no_recents_is_the_empty_state` — a fresh view (and `set_recents(vec![])`) ⇒ `recent_row_count()
  == 0` and `is_empty_state()` true.

**`app.rs` `#[cfg(test)]` (extends the existing recents tests):**

- `showing_welcome_seeds_current_recents` — record a temp `.xlsx` (`open_path_detached`), then
  `show_welcome` ⇒ `welcome_view().recent_row_count() == 1` (`do_show_welcome` seeding).
- `recording_updates_the_open_welcome` — `show_welcome` (empty), then `open_path_detached` a temp
  `.xlsx` ⇒ the open welcome's `recent_row_count()` becomes 1 (§2.4 live update via the
  `refresh_recents_ui` welcome branch).
- `clicking_a_recent_row_routes_to_open_path` — seed the welcome with a row for a real temp file,
  delete the file, `open_recent(0)`, `run_until_parked` ⇒ the welcome shows a modal
  (`has_modal()`), i.e. the click reached `open_path` whose canonicalize-failure surfaced the
  "Couldn't open the file" error on the welcome — proving the routing without spawning a real
  worker.

**Render suite:** the welcome window is not in the pixel suite (which targets `GridView`); this
change touches no grid/chrome/titlebar render code, so baselines are unaffected and must still
pass (`render_tests.sh test`) — do not regenerate. Welcome correctness is covered by the view
tests above + the Xvfb smoke launch.
