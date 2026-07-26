---
status: complete
---

# Implementation Plan: xlsx File Association

Ordered build. Phases 1–3 and 5 each **commit and push**; phase 4 is the user-run **non-blocking**
release smoke (does not gate the coding commits). Details live in `functional_spec.md` +
`architecture.md`; this is the checklist. No pixel render suite for phases 1–4 (no baseline pixels
move — architecture §6); phase 5 touches only the welcome window, which is also out of the pixel
suite's scope (validate with gpui view tests + a smoke launch).

## Phases

- [x] **Phase 1 — Declaration (packaging config).** Add the two
  `[[package.metadata.packager.file-associations]]` entries (`xlsx`, `csv`) to
  `crates/freecell-app/Cargo.toml` (architecture §2). Verify by building the `.deb` and
  asserting the generated `freecell.desktop` has `MimeType=…spreadsheetml.sheet;text/csv;` and
  `Exec=… %F`. Enables Windows/Linux end-to-end (argv already wired). Small.

- [x] **Phase 2 — macOS entry-point bridge (the substantive phase + spike).** New
  `shell/open_files.rs`: cfg-agnostic `file_url_to_path` (+ unit tests, architecture §3.3) and
  the `cfg(macos)` `install_finder_open` channel bridge; promote `url` + `async-channel` to
  direct deps; split the `main.rs` startup welcome-vs-open decision (architecture §3.2). Honor
  the no-flash requirement (functional §5.5) and the spike guardrail — if `on_open_urls` doesn't
  fire at the pinned rev, stop and raise it (do not add an `NSApplicationDelegate`). Crate-scoped
  `cargo build`/`test -p freecell-app` + `cargo fmt --all --check`.

- [x] **Phase 3 — Docs & CI reconciliation.** After 1+2 land: drop `continue-on-error` from the
  `release.yml` Windows job and correct the docs that assert Windows can't compile / macOS
  Finder-open is unwired (architecture §8, functional §8) — `PACKAGING.md`, `windows-port.md`,
  `GAPS.md` #4, DECISIONS Phase 10, `coverage_matrix.md`, `smoke_checklist.md` M-15, READMEs,
  mvp `architecture.md`. Prose + one CI-gate flip; no app-code behavior change.

- [ ] **Phase 4 — Release smoke (user-run, NON-BLOCKING).** Build `.app`/`.dmg` (+ NSIS) via
  `scripts/package.sh`; user installs and smokes on real macOS: double-click / Open-With /
  drag-to-Dock / `open -a`, cold + warm start, for `.xlsx` and `.csv` (smoke item **M-15**);
  optional Windows hardware smoke. Phases 1–3 are already committed/pushed before this runs. When
  run after phase 5, also exercise the welcome "Set as default app for xlsx files" link (macOS sets
  it silently; Windows opens Settings).

- [x] **Phase 5 — "Set as default" (cross-platform default-handler integration).** Reverses the
  original "becoming the default handler = non-goal" (project_overview / functional_spec §9 — flip
  those notes as part of this phase). A new, **well-isolated** `shell/default_app` module exposes a
  clean platform-agnostic API — *detect* whether FreeCell is the current default handler for an
  extension (returns `Some(true)`/`Some(false)`/`None`-unknown), and *request* becoming it — with
  all the messy per-OS FFI contained behind `cfg`. Detection compares against our **own** runtime
  identity (not a hardcoded string): macOS LaunchServices `LSCopyDefaultRoleHandlerForContentType`
  vs `CFBundleGetIdentifier` (prefer `core-foundation` FFI already in the tree over adding
  `objc2-app-kit`); Linux `xdg-mime query default <mime>` vs our `.desktop` id; Windows
  `IApplicationAssociationRegistration::QueryCurrentDefault` vs our NSIS ProgId (confirm the exact
  ProgId cargo-packager 0.11.8 emits). Make-default: macOS `LSSetDefaultRoleHandlerForContentType`
  and Linux `xdg-mime default …` are **silent**; Windows cannot set silently (UserChoice hash) so it
  opens `ms-settings:defaultapps` as a guided hand-off. Welcome screen (`shell/welcome.rs`): when
  `xlsx` status is `Some(false)`, render a **"Set as default app for xlsx files"** text-link under
  "Open Demo Spreadsheet" in the same tertiary style (id/debug_selector `welcome-set-default-link`);
  click sets default + re-checks/hides (mac/Linux) or opens Settings (Windows); cache the queried
  status in the view (one query at build), with a `#[cfg(test)]` hook to force it for a view test.
  xlsx-only on the button (module stays generic so `.csv` is a trivial follow-on). Validate: Linux
  crate-scoped build + `cargo test -p freecell-app` (a gpui view test that the link paints when
  status is forced `No`, plus cfg-agnostic unit tests for the pure ext→UTI/mime + id-compare
  helpers) + `cargo fmt --all --check` + an Xvfb smoke launch; the mac/Windows FFI can't compile on
  the Linux host, so confirm those arms via the compile gates — dispatch **`macos-verify`** (macOS)
  and the **`release`** Windows build. No pixel render suite (welcome window is out of scope).
