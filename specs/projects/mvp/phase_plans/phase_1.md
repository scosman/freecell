---
status: complete
---

# Phase 1: Scaffolding & CI

## Overview

Stand up the `app/` Cargo workspace, the four crate skeletons with the strict
dependency rule, pinned toolchain + lint/format/deny config, the GitHub Actions CI
(Linux `checks` required-green; `perf-gates` defined; `macos-verify` manual/cron), a
hello-world GPUI + gpui-component window that builds on Linux and macOS, and the
**load-bearing Linux render spike** (Xvfb + Mesa lavapipe capture). This locks the
gpui-component SHA against the pinned gpui rev and records the toolchain, the spike
result, and any deviations in `DECISIONS_TO_REVIEW.md`.

Nothing downstream blocks on the spike outcome: a failed Linux capture is a decision
point (move the render suite to `macos-verify`), not a stopper (`implementation_plan.md`,
`architecture.md §9`).

## Steps

1. **Workspace root** (`app/`):
   - `Cargo.toml`: `[workspace]` (resolver 2), members = the four crates,
     `[workspace.package]` (edition 2021, license, version, rust-version),
     `[workspace.dependencies]` pinning ironcalc `=0.7.1`, the gpui git rev
     `1d217ee39d381ac101b7cf49d3d22451ac1093fe`, gpui-component pinned to a `main` SHA
     that pins that exact rev, and the utility deps; `[workspace.lints]`; profiles
     (gpui built with opt-level 3 in dev for tolerable frame times).
   - `rust-toolchain.toml`: pinned stable channel + rustfmt/clippy components.
   - `rustfmt.toml`: defaults + `max_width = 100`.
   - `deny.toml`: cargo-deny licenses/advisories/bans with the documented GPL
     `ztracing` exception (zed #55470).
   - `README.md`: build/run/test + render-baseline process skeleton.
   - `scripts/linux_render_spike.sh`: the spike runner.
2. **`crates/freecell-core`**: GPUI-free, IronCalc-free skeleton. `src/lib.rs` with the
   Excel-max constants and module scaffolding; a sanity unit test. (Axis / validators /
   selection are Phase 2.)
3. **`crates/freecell-engine`**: depends on `freecell-core` + `ironcalc(_base)`.
   `src/lib.rs` with a smoke test that constructs `UserModel::new_empty(...)` to prove
   the IronCalc dep links and is usable. (Adapter/worker are Phases 3–5.)
4. **`crates/freecell-app`**: depends on core, engine, gpui, gpui_platform,
   gpui-component, gpui-component-assets. `src/main.rs` = the hello-world window
   (gpui-component `Root` + bundled assets + `gpui_component::init`) with a
   `--exit-after-ms <n>` flag so the spike can open→draw→quit deterministically under
   Xvfb.
5. **`render-tests/`**: crate skeleton (plain lib placeholder — the gpui/grid deps and
   the ported perceptual diff land in Phase 7), `baselines/` dir, `README.md` skeleton
   documenting the human baseline process. Kept off the gpui dependency edge in P1 so
   the workspace builds regardless of the spike outcome.
6. **CI** (`.github/workflows/` at repo root, `working-directory: app`):
   - `checks.yml` (Linux, required): apt system deps (+ mesa-vulkan-drivers/lavapipe +
     xvfb), toolchain, rust-cache, `fmt --check`, `clippy -D warnings`, workspace
     build, workspace test, render/spike step under Xvfb+lavapipe, `cargo-deny check`.
   - `perf-gates.yml` (Linux, required): structure defined; harness + thresholds are
     Phase 12, so the job builds and prints a TODO placeholder for now (green).
   - `macos-verify.yml` (manual dispatch + weekly cron, non-required): build + test +
     render smoke on `macos-14`.
7. **Render spike**: build the hello-world for Linux, install lavapipe, run it under
   `xvfb-run`, capture the rendered window to PNG (preference order: GPUI capture API →
   X window capture → macOS fallback), confirm a non-blank PNG. Record the working
   variant (or the failure + fallback decision) in `DECISIONS_TO_REVIEW.md`.
8. **Record decisions**: gpui-component SHA, toolchain version, gpui_platform feature
   set, spike outcome, and any spec deviations in `DECISIONS_TO_REVIEW.md`.

## Tests

- `freecell-core::excel_max_constants_are_correct`: the row/col maxima match the Excel
  grid (1,048,576 × 16,384) — a real invariant the whole project leans on.
- `freecell-engine::ironcalc_links_and_creates_empty_model`: `UserModel::new_empty`
  succeeds — proves the pinned IronCalc dep resolves, links, and is callable (the
  engine track's foundation).
- Build-as-test: `cargo build --workspace` on Linux compiles `freecell-app` (gpui +
  gpui-component) — the SHA-compatibility check that the phase is fundamentally about.
- Spike (manual/CI, not a unit test): a non-blank PNG captured from the GPUI window
  under Xvfb + lavapipe; result recorded.

## Notes / judgment calls (also logged in DECISIONS_TO_REVIEW.md)

- gpui-component `main` at the resolved SHA already pins the exact target gpui rev, so
  it is the known-good pair (no rev-pair bisection needed).
- `render-tests` intentionally does not yet depend on gpui/`freecell-app` — that edge
  and the ported perceptual diff are Phase 7; keeping them out here means a spike
  failure never blocks the workspace build.
- `perf-gates.yml` is defined but a placeholder until Phase 12 (no perf harness yet).
