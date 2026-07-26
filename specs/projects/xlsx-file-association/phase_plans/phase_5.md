---
status: complete
---

# Phase 5: "Set as default" (cross-platform default-handler integration)

## Overview

Reverses the original "becoming the default handler = non-goal" (project_overview / functional_spec
§9). Candidate registration (the packaging default from Phase 1) stays; this phase adds an explicit,
**user-initiated** "make FreeCell the default for xlsx" action, surfaced as a tertiary text-link on
the welcome screen.

All the messy per-OS FFI lives behind `cfg` in one new **well-isolated** module
`shell/default_app.rs`. It exposes a small, platform-agnostic API — *detect* whether FreeCell is the
current default handler for an extension (`Option<bool>`: `Some(true)`/`Some(false)`/`None`-unknown),
and *request* becoming it (`MakeDefaultOutcome`). `welcome.rs` calls that tidy API and knows nothing
about LaunchServices / COM / `xdg-mime`.

Detection compares against our **own** runtime identity (not a hardcoded string):
- **macOS** LaunchServices `LSCopyDefaultRoleHandlerForContentType(uti, kLSRolesAll)` vs
  `CFBundleGetIdentifier(CFBundleGetMainBundle())` — pure `core-foundation`/`CoreServices` FFI (no
  new crate). Set: `LSSetDefaultRoleHandlerForContentType` (silent, no prompt/admin).
- **Linux** `xdg-mime query default <mime>` vs our `.desktop` id (`freecell.desktop`). Set:
  `xdg-mime default freecell.desktop <mime>` (silent). `xdg-mime` absent → `None`.
- **Windows** COM `IApplicationAssociationRegistration::QueryCurrentDefault(".xlsx",
  AT_FILEEXTENSION, AL_EFFECTIVE)` vs our NSIS ProgId. Cannot set silently (UserChoice hash) →
  open `ms-settings:defaultapps` (guided hand-off).

## Steps

1. **`crates/freecell-app/Cargo.toml`** — add a `#[cfg(windows)]` direct dep on `windows` (already
   resolved at 0.61.3 in the lockfile via gpui — no new crate), features `Win32_UI_Shell` +
   `Win32_System_Com`, under a new `[target.'cfg(windows)'.dependencies]` table. macOS needs no new
   dep (pure framework FFI); Linux shells out.

2. **`crates/freecell-app/src/shell/default_app.rs`** (NEW):
   - `pub enum MakeDefaultOutcome { SetSilently, OpenedSettings, Unavailable }` (`#[allow(dead_code)]`
     — each variant is produced on a subset of platforms).
   - Pure, cfg-agnostic core (unit-tested on Linux):
     - `struct FileType { extension, uti, mime, windows_progid }` (`#[allow(dead_code)]` — each field
       is read by exactly one platform arm) + `fn file_type(&str) -> Option<FileType>` mapping
       `xlsx`/`csv` (normalizes: strips a leading `.`, lowercases).
     - `fn identifiers_match(ours, theirs) -> bool` — trim + `eq_ignore_ascii_case`, false if either
       empty.
   - `pub fn default_status(ext) -> Option<bool>` / `pub fn make_default(ext) -> MakeDefaultOutcome`
     dispatch to a per-OS `platform` submodule; `pub fn is_default_for_xlsx()` /
     `pub fn make_default_for_xlsx()` convenience wrappers.
   - `#[cfg(target_os = "macos")] mod platform` — raw `#[link(name = "CoreFoundation"/"CoreServices",
     kind = "framework")]` extern blocks; small `CfString` RAII wrapper (CFRelease on drop),
     `cfstring_to_string`, `our_bundle_id`. No `unwrap`/`expect`; every failure → `None` /
     `Unavailable`.
   - `#[cfg(target_os = "linux")] mod platform` — `std::process::Command` for `xdg-mime query
     default` / `xdg-mime default`; `OUR_DESKTOP_ID = "freecell.desktop"`.
   - `#[cfg(windows)] mod platform` — COM detection via the `windows` crate; make-default opens
     `ms-settings:defaultapps` via a process spawn (documented: ShellExecuteW's 0.61 signature can't
     be compile-checked on this Linux host; a URI spawn is equivalent and certain).
   - `#[cfg(not(any(target_os="macos", target_os="linux", windows)))] mod platform` — `None`/no-op
     fallback so the crate still builds anywhere.
   - `#[cfg(test)] mod tests` — cfg-agnostic unit tests for `file_type` (xlsx/csv/normalization/
     unknown) and `identifiers_match`.

3. **`crates/freecell-app/src/shell/mod.rs`** — `mod default_app;` (crate-internal; welcome.rs uses
   `super::default_app::…`).

4. **`crates/freecell-app/src/shell/welcome.rs`**:
   - Add field `xlsx_default_status: Option<bool>`, set in `new()` from
     `initial_xlsx_default_status()` — a `#[cfg(not(test))]` fn that calls the module (one cheap OS
     query at build) and a `#[cfg(test)]` fn returning `None` (hermetic tests; injected instead).
   - Render a `welcome-set-default-link` tertiary text-link (same style as `welcome-demo-link`)
     directly under the demo link, wrapped with it in an 8px column, **only** when
     `xlsx_default_status == Some(false)`.
   - Click → `set_as_default_for_xlsx`: on `SetSilently` re-query + `cx.notify()` (link disappears
     once default); on `OpenedSettings`/`Unavailable` leave as-is.
   - `#[cfg(test)] fn set_xlsx_default_status_for_test(&mut self, Option<bool>, cx)` to inject status.

5. **Spec flips** — `project_overview.md` Non-goals first bullet + `functional_spec.md §9` first
   bullet (and §1 handler-rank note): candidate registration is still the packaging default; an
   explicit user-initiated "Set as default" is now IN scope; automatic seizing stays a non-goal.

## Tests

- `file_type_maps_known_extensions` — `xlsx` / `csv` map to the right uti/mime/progid; leading-dot
  and uppercase normalize; unknown → `None`.
- `identifiers_match_is_case_insensitive_and_trimmed` — `"freecell.desktop"` vs `" FreeCell.Desktop
  "` true; different ids false; empty either side false.
- `render_paints_the_set_default_link_when_not_default` (gpui view test, mirrors
  `render_paints_the_demo_link`) — a fresh view (status `None`) paints no link; injecting `Some(false)`
  paints `welcome-set-default-link`; injecting `Some(true)` hides it again.

## Render / build validation

No pixel suite — the welcome window is out of the pixel suite's scope (implementation_plan header;
functional §10). Validate: crate-scoped `cargo build -p freecell-app` + `cargo test -p freecell-app
--lib` (Linux arm) + `cargo clippy -p freecell-app --all-targets -- -D warnings` + whole-workspace
`cargo fmt --all --check` + an Xvfb smoke launch. The macOS/Windows FFI arms can't compile on this
Linux host; their correctness rests on the source-verified OS APIs and the CI compile gates
(`macos-verify`, the `release` Windows build) the manager dispatches after commit.
