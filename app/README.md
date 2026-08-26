# FreeCell (`app/`)

A GPU-rendered (GPUI), Excel-compatible spreadsheet built to be stupid-fast on huge
sheets (Excel-max = 1,048,576 × 16,384). Engine = IronCalc; UI = GPUI (custom grid +
gpui-component chrome). See `../specs/projects/mvp/` for the specs and
`../CLAUDE.md` for project conventions.

**Platform support:** macOS (primary, Metal) + Linux (blade/Vulkan) + Windows (NSIS
installer) — all three compile and package, and all three are required `release` CI targets.
See `architecture.md §1` for the Linux MVP deltas (Ctrl for Cmd, no menu bar, GPUI
paths-prompt).

## Workspace layout

```
app/
├── crates/
│   ├── freecell-core/    # GPUI-free, IronCalc-free foundation (builds/tests anywhere)
│   ├── freecell-engine/  # IronCalc adapter + eval worker + caches + file I/O (headless)
│   └── freecell-app/     # the GPUI application (macOS + Linux + Windows)
└── render-tests/         # cell-render snapshot suite (Phase 7)
```

**Strict dependency rule** (`architecture.md §1`): `freecell-core` → std only;
`freecell-engine` → core + ironcalc; `freecell-app` → core, engine, gpui,
gpui-component; `render-tests` → freecell-app, gpui. Core and engine never import GPUI,
so they build and test headless in Linux CI.

## Prerequisites

- The pinned toolchain in `rust-toolchain.toml` (rustup installs it automatically).
- **Linux build deps:** `build-essential pkg-config cmake libx11-dev libxcb1-dev
  libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libvulkan-dev libgbm-dev
  libfontconfig1-dev libfreetype-dev libasound2-dev`. For the render path also
  `mesa-vulkan-drivers` (lavapipe software Vulkan), `xvfb`, `x11-xserver-utils`
  (`xrefresh`), `imagemagick`.
- **macOS:** the standard Xcode command-line tools (Metal stack).

## Compile cache — dev containers only

Cloud dev containers boot from an environment **image** that already carries the pinned
toolchain, the dependency sources, the system packages above, and an
[sccache](https://github.com/mozilla/sccache) disk cache holding the compiled pinned dep
tree (gpui/zed, gpui-component, ironcalc fork). A committed `SessionStart` hook
(`.claude/settings.json` → `.claude/hooks/session-start.sh`) runs
[`../.config/utils/setup_startup.sh`](../.config/utils/setup_startup.sh) in about a second
to verify the toolchain, wire `RUSTC_WRAPPER=sccache`, and `cargo fetch --locked`.

There is no bucket and no credential — the cache is a directory on the machine's own disk.
CI is deliberately untouched (it uses `Swatinem/rust-cache`). Design, the rejected warm-
`target/` alternative, and what is still paid per session:
[`../projects/build-cache.md`](../projects/build-cache.md). Cloud-UI setup steps:
[`../.config/utils/claude_cloud_setup.md`](../.config/utils/claude_cloud_setup.md).

**On your own machine**, none of that applies (the startup script refuses to run outside a
container). To install the pinned toolchain, warm `~/.cargo`, and optionally install the
system packages listed under Prerequisites:

```sh
bash ../.config/utils/setup_env.sh --human
```

## Build / run / test

```sh
cargo build --workspace          # full build (freecell-app compiles on Linux, macOS + Windows)
cargo run -p freecell-app        # launch FreeCell (opens the Welcome window)
cargo run -p freecell-app -- Book.xlsx   # open a workbook directly (CLI argv path)
cargo test --workspace           # core + engine + app logic tests (no display needed)
cargo fmt --all --check          # formatting gate
cargo clippy --workspace --all-targets -- -D warnings   # lint gate
cargo deny check                 # licenses/advisories (see deny.toml)
```

`freecell-app` is the full spreadsheet application: a Welcome window, one window per
workbook (grid + action row + formula bar + sheet tabs), open/edit/save `.xlsx`, basic
formatting, and multi-sheet management. On Linux it runs on X11/Wayland (Ctrl for Cmd, no
menu bar — `architecture.md §1`); `cargo test --workspace` is headless (the pixel render
suite is a separate step, below).

## Render tests & baselines

The cell-render snapshot suite (`render-tests/`) renders the **real** `GridView` over
scenes produced by the **real** engine, captures PNGs on Linux under **Xvfb + Mesa
lavapipe** (software Vulkan), and perceptually diffs them against the committed baselines —
one `#[test]` per feature/permutation, so a red line names the exact broken thing. Text
renders in the **bundled Inter** font (registered via `add_fonts`), so bold/italic and
metrics are identical on macOS, Linux, and CI. Install the Linux capture stack with
`render-tests/scripts/setup_render_env.sh`.

```sh
render-tests/scripts/render_tests.sh test              # run the full pixel suite (asserts baselines)
render-tests/scripts/render_tests.sh generate [--only <prefix>]   # regenerate baselines/
```

The full human baseline workflow (regenerate on the pinned runner image, **eyeball every
changed PNG**, commit together with the code change), the pinned image + Mesa version, and
the tolerance constants are documented in
[`render-tests/README.md`](render-tests/README.md). (`app/scripts/linux_render_spike.sh` is
the original Phase-1 capture-path spike, kept for reference.)

## Perf harness & gates

The perf harness drives the real grid over a 1M×100 styled fixture (the POC "Run Test"
scroll scenario) and asserts the `architecture.md §4` budgets (frame p99, cell-load p99,
zero engine calls on the scroll path):

```sh
render-tests/scripts/perf.sh            # run the perf harness + CI-buffered gates
```

Calibrated thresholds + methodology are in `render-tests/src/perf.rs` and
`DECISIONS_TO_REVIEW.md` (Phase 12).

## CI

GitHub Actions live at the repo root (`../.github/workflows/`):

- **checks** (Linux, required): fmt, clippy `-D warnings`, workspace build, workspace test,
  the render suite (Xvfb + lavapipe), cargo-deny (licenses/advisories — see `deny.toml`).
- **perf-gates** (Linux, required): the perf harness with buffered thresholds.
- **macos-verify** (manual/weekly, non-required): build + test + render smoke on macOS.
- **release** (tag `v*` / manual dispatch): package the app with `cargo-packager` for
  macOS, Linux, and Windows (all required), uploading installers as run artifacts. The
  Windows job Authenticode-signs the exe + installer via Azure Trusted Signing when the
  signing secrets are set. See [`PACKAGING.md`](PACKAGING.md).

## Packaging / releases

`cargo-packager` builds distributable bundles (macOS `.app`/`.dmg`, Linux `.deb`/`.AppImage`,
Windows NSIS `.exe`). Build them locally with the same scripts CI uses:

```sh
cargo install cargo-packager --locked --version 0.11.8   # one-time
scripts/package.sh          # macOS / Linux  -> app/target/packages/
scripts\package.ps1         # Windows (PowerShell)
```

Bare `package.*` output is **unsigned**; macOS has a signed + notarized path
(`scripts/sign_macos.sh`) and Windows Authenticode-signs via Azure Trusted Signing in CI —
though nothing is **distribution-ready** until the pre-distribution security audit clears.
Config, formats, prerequisites, the Windows target status, and the signing paths are
documented in [`PACKAGING.md`](PACKAGING.md); the app icons in
[`crates/freecell-app/packaging/icons/`](crates/freecell-app/packaging/icons/README.md).
