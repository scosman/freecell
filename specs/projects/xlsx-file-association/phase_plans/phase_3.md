---
status: complete
---

# Phase 3: Docs & CI reconciliation

## Overview

Seam C of the project (architecture §8, functional §8): now that Phase 1 (packaging
declaration, `84785fa`) and Phase 2 (macOS Apple-Event bridge, `f2e72cc`) have landed and the
product owner has confirmed Windows compiles + builds + packages cleanly (CI green + local
build, 2026-07-24), correct the docs/CI that still assert the opposite. Two categories, no
app-code behavior change:

1. **CI gate flip** — drop `continue-on-error` from the `release.yml` Windows job so a Windows
   packaging failure gates the release like macOS/Linux.
2. **Doc reconciliation** — correct every doc that claims (a) Windows can't compile / is out of
   scope, or (b) macOS Finder open-file is unwired, and mark the tracked gap resolved with a
   pointer to this project + the two commits.

Prose + one CI-gate flip only — the no-flash bridge and the packaging block already exist. No
pixel suite (no baseline pixels move — architecture §6).

## Steps

1. **`.github/workflows/release.yml` — flip the Windows gate.** Remove `continue-on-error: true`
   from the `windows` job; rename it `package (Windows — nsis)`; rewrite the job comment
   ("first-class target … gates the release … no continue-on-error"). Rewrite the file headline
   (line 1) + header block ("macOS, Linux, AND Windows are all REQUIRED …"). Align the upload
   step with the required macOS/Linux jobs — drop `if: always()` and set
   `if-no-files-found: error` (the `always()`/`ignore` combo existed only because the job was
   allowed to fail).

2. **`app/PACKAGING.md`.** Status table Windows row `Experimental / non-blocking` → `Supported`.
   CI section: three jobs, all required. Replace the `## Windows: what a real port needs` section
   (asserted out-of-scope / not-guaranteed-to-compile / item-3 file-associations TODO) with a
   `## Windows` section stating it compiles/packages/gates CI, with file associations **wired**
   (Cargo.toml block + `open_path` pointer, Phase 1 commit); residual = hardware smoke + signing.
   Verification-status bullet: drop "experimental" from the NSIS line.

3. **`projects/windows-port.md` + `PROJECTS.md`.** Status `Future / app build not a real target`
   → `Largely done (2026-07-24)`. Reconcile "current state" (remove out-of-scope /
   not-guaranteed-to-compile / non-blocking-`continue-on-error`), mark file associations done,
   narrow "work when picked up" to genuine residuals (hardware smoke, DPI/data-path polish,
   render/perf-gate decision, signing).

4. **`GAPS.md` #4 (macOS Finder open-file).** Table row + inline detail + "still open" summary +
   the v0.5 tier-table row → **Resolved/Shipped** with the two commits and the module pointer.

5. **`specs/projects/mvp/DECISIONS_TO_REVIEW.md` Phase 10.** Preserve the original decision text
   verbatim (the file forbids editing above the append line); append a nested
   `- **[Resolved 2026-07-24] …**` bullet (the ztracing pattern). Update the end-of-file
   known-limitations summary bullet to Resolved.

6. **`specs/projects/mvp/coverage_matrix.md`.** §2 Finder-open row `Known-limitation` → `Resolved`;
   numbered known-limitation item 4 → Resolved. (Round-2 CR: also fix the CLI-argv row's stale
   `xlsx_arg` → `open_arg`.)

7. **`specs/projects/mvp/smoke_checklist.md` M-15.** `Known gap` → `Wired … pending non-blocking
   macOS hardware smoke`, with the full smoke matrix (double-click / Open-With / drag-to-Dock /
   `open -a`, cold+warm, xlsx+csv).

8. **`app/README.md`.** Platform-support line `Windows is out of scope` → all three
   compile/package/required; layout + build + CI + packaging lines de-"experimental"-ed.

9. **`specs/projects/mvp/architecture.md`.** §1 `Windows out of scope` → `was out of MVP scope
   (since made a first-class target …)`; §Structure Finder "open with" line → **wired** with the
   project pointer. (Round-2 CR: also annotate the sibling `components/app_shell.md` open-events
   line with a `[Resolved — see specs/projects/xlsx-file-association]` note so the two MVP planning
   docs agree.)

## Files changed

`.github/workflows/release.yml`, `app/PACKAGING.md`, `projects/windows-port.md`, `PROJECTS.md`,
`GAPS.md`, `specs/projects/mvp/DECISIONS_TO_REVIEW.md`, `specs/projects/mvp/coverage_matrix.md`,
`specs/projects/mvp/smoke_checklist.md`, `app/README.md`, `specs/projects/mvp/architecture.md`,
`specs/projects/mvp/components/app_shell.md` (round-2 CR addition).

## Verification notes

- No Rust source touched; a full cargo build is unnecessary per the phase scope. Ran whole-
  workspace `cargo fmt --all --check` (exit 0) to confirm no `.rs` slipped in, and validated
  `release.yml` parses as valid YAML. No pixel render suite (no baseline pixels move).
- Grounding held to the settled authority (functional §8/§11): Windows stated as a working
  target with no "reportedly"/"should" hedging; no fresh CI run gated the doc edits. Every
  resolved/wired/shipped claim is backed by the landed `84785fa` (packaging block in
  `crates/freecell-app/Cargo.toml`) + `f2e72cc` (`shell/open_files.rs` bridge + `main.rs` split),
  both verified on disk before editing.
- Round-2 CR (four line-edits) cleared: `release.yml:1` headline, `app_shell.md` resolution note,
  `coverage_matrix.md` `xlsx_arg`→`open_arg`; re-ran fmt + YAML check clean.
- The final macOS hardware smoke (M-15) is Phase 4 (user-run, NON-BLOCKING) — it does not gate
  these commits.
