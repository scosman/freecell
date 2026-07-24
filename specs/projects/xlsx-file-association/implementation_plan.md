---
status: complete
---

# Implementation Plan: xlsx File Association

Ordered build. Phases 1–3 each **commit and push**; phase 4 is user-run and **non-blocking**
(does not gate the earlier commits). Details live in `functional_spec.md` + `architecture.md`;
this is the checklist. No pixel render suite (no baseline pixels move — architecture §6).

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
  optional Windows hardware smoke. Phases 1–3 are already committed/pushed before this runs.
