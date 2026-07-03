# DECISIONS_TO_REVIEW — implementation-phase log

**Purpose (part of the autonomy contract — see `implementation_plan.md`):**
implementation agents work autonomously and never stop to ask a human. When you make
a judgment call the specs don't cover, deviate from a spec because reality forced it
(e.g., an API missing at the pinned rev), or resolve a placeholder (pinned SHA,
calibrated threshold), **append an entry here and keep building**. A human reviews
this file at their leisure — it is a log, not a request queue.

Entry format: `- [Phase N] <decision> — <one-line rationale> (<files/spec section
affected>)`.

Known placeholders the build will resolve (append the resolution here):

- gpui-component pinned SHA + Rust toolchain version (Phase 1).
- Linux render-capture variant proven by the Phase-1 spike (or fallback-to-macOS).
- Perf-gate CI thresholds calibrated on the pinned runner image (Phase 12).
- Perceptual-diff thresholds after first real baselines (Phase 7).

---
*Append entries below this line. Do not edit above it.*

- [Phase 1] gpui-component pinned to SHA `a9a7341c35b62f27ff512371c62419342264710c`
  (longbridge/gpui-component `main`) — its workspace `Cargo.toml` at this SHA pins the
  exact target zed rev `1d217ee39d381ac101b7cf49d3d22451ac1093fe`, so it is the
  known-good pair with no rev-pair bisection needed. (`app/Cargo.toml`)
- [Phase 1] Rust toolchain pinned to stable `1.95.0` (`app/rust-toolchain.toml`).
  Resolved empirically: gpui at the pinned rev calls `std::hint::cold_path()` with no
  `#![feature(...)]` gate, so it requires the stable where `cold_path` is stabilized.
  Building on 1.94.1 fails with E0658 (`crates/gpui/src/profiler.rs`). zed's own
  `rust-toolchain.toml` at the pinned rev pins stable `1.95.0`, which is exactly the
  version that stabilized `cold_path` — so FreeCell matches zed's pin. Any future gpui
  rev bump must re-check zed's toolchain pin. (`app/rust-toolchain.toml`)
- [Phase 1] `gpui_platform` features set to `["font-kit", "x11", "wayland",
  "runtime_shaders"]` rather than architecture §1's `["font-kit"]` — the extra features
  are the Linux backends (x11/wayland) + shader path that gpui-component's own workspace
  enables, required for the cross-platform (macOS + Linux) build the phase mandates. The
  §1 list read as the macOS-focused subset. (`app/Cargo.toml`)
- [Phase 1] Workspace crates use edition 2021 (per architecture §1) even though the gpui
  and gpui-component crates themselves are edition 2024 — per-crate editions are
  independent; the pinned 1.95.0 toolchain supports both. (`app/*/Cargo.toml`)
- [Phase 1] `render-tests` is a bare skeleton crate that does NOT yet depend on
  `gpui`/`freecell-app` — those deps + the ported round-3 C perceptual diff + the case
  suite are Phase 7. Keeping it off the gpui edge in P1 means a render-spike failure can
  never block the workspace build/test. (`app/render-tests/`)
- [Phase 1] `deny.toml` is lenient on `[bans]` (multiple-versions/wildcards allowed) and
  `[sources] unknown-git = "allow"` because zed's tree carries many duplicate versions
  and pinned git forks; the load-bearing gate is licenses (the documented GPL `ztracing`
  exception, all three GPL-3.0 SPDX spellings, tracked vs zed#55470). P13 hardening may
  tighten bans/sources. (`app/deny.toml`)
- [Phase 1] `perf-gates.yml` is DEFINED but a placeholder (builds the workspace, prints a
  TODO) — the perf harness + committed buffered thresholds are Phase 12. The Phase-1
  `checks.yml` render step runs the spike as `continue-on-error` (informational); Phase 7
  flips it to a required `cargo test -p render-tests`. (`.github/workflows/`)
- [Phase 1] **LINUX RENDER SPIKE: PASSED** — the primary capture path works, so the
  render suite stays on Linux CI and the macOS offscreen-Metal fallback is NOT needed.
  The hello-world GPUI + gpui-component window renders under Xvfb + Mesa lavapipe
  (software Vulkan: "llvmpipe (LLVM 20.1.2)") and its pixels capture to a non-blank PNG
  (1566 colors — the FreeCell title, subtitle, 3×3 grid, and yellow B2 fill all render;
  text, fills, and borders confirmed). Capture variant = **option 2** from
  `render_test_harness.md §Mechanism`: render to an X window under Xvfb, capture the root
  via ImageMagick `import`. (`app/scripts/linux_render_spike.sh`)
- [Phase 1] **Load-bearing spike detail (MUST carry into Phase 7):** gpui's X11 backend
  only *presents* a rendered frame when it receives an **Expose** event
  (`crates/gpui_linux/.../x11/client.rs`: `require_presentation` is gated on
  `expose_event_received`). Under Xvfb there is no compositor to emit one, so the frame
  renders but never reaches the framebuffer → blank capture. Fix: run **`xrefresh`**
  (x11-xserver-utils) after the window settles — it repaints the root, forcing an Expose
  on every window so gpui presents. Phase 7's capture step must do the same (or drive an
  equivalent redraw). Related: the spike app quits via a real executor timer
  (`App::spawn` + `background_executor().timer`), NOT a render-loop deadline — with no
  compositor `render` runs only once, so a paint-path deadline never fires.
  (`app/scripts/linux_render_spike.sh`, `crates/freecell-app/src/main.rs`)
- [Phase 1] Linux system deps needed beyond architecture §1's list, found while making
  the app link + render: `libxkbcommon-x11-dev` (link fails on `-lxkbcommon-x11` without
  it), `libfreetype-dev` (the `libfreetype6-dev` name is obsolete on Ubuntu 24.04), and
  `x11-xserver-utils` (xrefresh). All added to `checks.yml` / `perf-gates.yml` /
  `app/README.md`. (`.github/workflows/`, `app/README.md`)
- [Phase 1] Verified end-to-end on the container image (Ubuntu 24.04, Rust 1.95.0):
  `cargo build --workspace`, `cargo fmt --all --check`, `cargo clippy --workspace
  --all-targets -- -D warnings`, `cargo test --workspace` (freecell-core: 2 unit + 5
  integration dependency-rule-guard tests; freecell-engine: 1 unit — 8 total), the render
  spike, AND `cargo deny check` (cargo-deny 0.19.9) all pass. (`app/`)
- [Phase 1] **cargo-deny: needs human review.** Making the required gate pass surfaced,
  beyond the specced GPL `ztracing` license exception, a set of TRANSITIVE issues from
  gpui/zed's pinned tree with **no safe upgrade** (we do not control zed's lockfile),
  all now documented (not silently skipped) in `app/deny.toml`:
  - Licenses: `bzip2-1.0.6` (permissive, via async_zip) added to `allow`; the GPL
    exception widened to the real crate names `zlog` + `ztracing_macro` (+ `ztracing`),
    which are the ones declaring `GPL-3.0-or-later`.
  - Advisories `ignore`d w/ rationale: `RUSTSEC-2025-0052` (async-std discontinued),
    `-2024-0384` (instant), `-2024-0436` (paste), `-2026-0173` (proc-macro-error2),
    `-2026-0192` (ttf-parser) — all *unmaintained*; and **two quick-xml 0.39.4
    vulnerabilities** `-2026-0194`/`-2026-0195` (DoS on *untrusted* XML, via
    wayland-scanner build dep + zbus/atspi accessibility). FreeCell feeds no untrusted
    XML through those paths; both are fixed in quick-xml ≥0.41, which needs a zed bump.
  - **All of the above (plus the GPL exception) must be re-audited on any gpui rev bump
    and resolved before any binary distribution.** Two non-fatal `no-license-field`
    warnings remain (zed-internal `gpui_shared_string`, `gpui_util`); left for P13.
    (`app/deny.toml`)
- [Phase 1] (post-CR) The strict dependency rule (`architecture §1`) is now
  CI-enforced by a guard test rather than only the hand-written graph:
  `freecell-core/tests/dependency_rule.rs` scans the core + engine manifests and fails
  if `freecell-core` gains a `gpui*`/`ironcalc*` runtime dependency or `freecell-engine`
  gains a `gpui*` one (dev-deps exempt; includes a negative-control test). A `deny.toml`
  ban was considered but rejected: cargo-deny can't scope a ban to a subgraph, and gpui
  is legitimately in the tree via `freecell-app`. (`app/crates/freecell-core/tests/`)
- [Phase 1] (post-CR) Trimmed forward-declared, not-yet-used crate deps to keep the
  manifests honest: dropped `anyhow`/`tracing` from `freecell-app` and
  `ironcalc`/`thiserror`/`tracing` from `freecell-engine` (only `ironcalc_base` is used
  in P1). The version pins stay in the workspace dependency table; each crate re-adds the
  line in the phase that first uses it (noted inline in the manifests). MSRV corrected to
  `rust-version = "1.95"` to match the pin. (`app/Cargo.toml`, `app/crates/*/Cargo.toml`)
