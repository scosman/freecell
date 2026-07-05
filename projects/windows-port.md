# Windows Port

**Status: Future (packaging wired 2026-07-05; app build not a real target yet).**

## Goal

Make FreeCell build and run on Windows as a first-class platform, then promote the
already-wired Windows packaging (NSIS installer) from experimental to supported.

## Current state (what exists today)

Windows is **out of scope** for the app (`README.md`, `app/README.md`,
`architecture.md §1`). The GPUI platform config in `app/Cargo.toml` wires only macOS/Metal
and Linux (`x11`/`wayland`); there is no Windows GPUI backend configured, so a Windows build
is **not guaranteed to compile**.

The `cargo-packager` work (2026-07-05) added the *packaging* half so it's ready when the
port lands: `scripts/package.ps1` builds an NSIS `.exe`, and the `release` workflow has a
Windows job — kept **non-blocking** (`continue-on-error`) precisely because the build may
fail. See `app/PACKAGING.md` ("Windows: what a real port needs").

## Work when picked up

1. **GPUI DirectX backend.** Split `gpui`/`gpui_platform` deps with a
   `[target.'cfg(windows)']` block selecting the DirectX backend + Windows features;
   confirm the pinned known-good gpui / gpui-component rev pair supports it (bump the pair
   together if not — never one alone, per `architecture.md §10`).
2. **Platform code arms.** Add Windows branches to everything `#[cfg]`-gated to macOS/Linux
   today: menus vs. no-menu-bar, `Cmd` vs `Ctrl`, native file dialogs, font registration,
   the `--exit-after-ms` render valve. Expect to flush these out via compile errors.
3. **System integration.** `.xlsx` file associations, per-monitor DPI, installed-app data
   paths (NSIS `appdata-paths`), and a Windows smoke of open/edit/save.
4. **Render/perf gates.** Decide whether the render-test + perf harness run on Windows or
   stay Linux/macOS-only.
5. **Promote CI.** Once it compiles + smokes, drop `continue-on-error` from the Windows job
   and update the platform-support statements in the READMEs + architecture.

## Not needed for this

Signing (Authenticode) is a separate deferral — see
`projects/release-signing-and-distribution.md`.
