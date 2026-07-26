# Windows Port

**Status: Largely done (2026-07-24) — Windows compiles, packages, and gates CI; `.xlsx`/`.csv`
file associations are wired; optional Authenticode signing is wired (2026-07-20). Remaining: a
hardware smoke.**

## Goal

Make FreeCell build and run on Windows as a first-class platform, and promote the
already-wired Windows packaging (NSIS installer) to supported. **Both are now done**; this
note is retained to track the residual polish.

## Current state (what exists today)

Windows is a **first-class, working target**: it compiles and packages (NSIS `.exe` via
`scripts/package.ps1`), and the `release` workflow's Windows job is **required** — a packaging
failure gates the release like macOS and Linux (`continue-on-error` was removed 2026-07-24;
CI green + local build confirmed by the owner the same day). The platform-support statements
in `app/README.md` and `specs/projects/mvp/architecture.md §1` were reconciled to match.

`.xlsx`/`.csv` **file associations are wired** — the shared cargo-packager
`file-associations` block emits the Windows NSIS registry ProgId + `shell\open\command`, and
the delivered path reaches the app through `main.rs::open_arg` → `FreeCellApp::open_path`
(process argv: the Explorer double-click / Open-With / shell-arg flows). See
`specs/projects/xlsx-file-association/` (Phase 1, commit `84785fa`) and `app/PACKAGING.md`
("Windows").

## Work when picked up (residual)

1. **Hardware smoke.** A Windows smoke of the installed NSIS build — open/edit/save plus the
   file-association double-click / Open-With — complementary to the CI green build. This is
   the non-blocking optional Windows leg of `specs/projects/xlsx-file-association` Phase 4.
2. **Polish surfaced by a real run.** Per-monitor DPI and installed-app data paths (NSIS
   `appdata-paths`) want a look once someone runs the app on Windows hardware.
3. **Render/perf gates.** Decide whether the render-test + perf harness run on Windows or
   stay Linux/macOS-only.

## Signing — wired (optional)

Authenticode signing of the core exe + installer via **Azure Trusted Signing** is wired
(2026-07-20): `scripts/package.ps1` signs both when the signing env vars are set, otherwise a
no-op unsigned build. It has not been run against a live Azure account yet (no CI secrets
provisioned). See `app/PACKAGING.md` §Signing and
`projects/release-signing-and-distribution.md`.
