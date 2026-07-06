# FreeCell (`app/`)

A GPU-rendered (GPUI), Excel-compatible spreadsheet built to be stupid-fast on huge
sheets (Excel-max = 1,048,576 × 16,384). Engine = IronCalc; UI = GPUI (custom grid +
gpui-component chrome). See `../specs/projects/mvp/` for the specs and
`../CLAUDE.md` for project conventions.

**Platform support:** macOS (primary, Metal) + Linux (blade/Vulkan). Windows is out of
scope. See `architecture.md §1` for the Linux MVP deltas (Ctrl for Cmd, no menu bar,
GPUI paths-prompt).

## Workspace layout

```
app/
├── crates/
│   ├── freecell-core/    # GPUI-free, IronCalc-free foundation (builds/tests anywhere)
│   ├── freecell-engine/  # IronCalc adapter + eval worker + caches + file I/O (headless)
│   └── freecell-app/     # the GPUI application (macOS + Linux)
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

## Build / run / test

```sh
cargo build --workspace          # full build (freecell-app compiles on Linux + macOS)
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
  macOS + Linux (required) and Windows (experimental), uploading unsigned installers as run
  artifacts. See [`PACKAGING.md`](PACKAGING.md).

## Packaging / releases

`cargo-packager` builds distributable bundles (macOS `.app`/`.dmg`, Linux `.deb`/`.AppImage`,
experimental Windows NSIS `.exe`). Build them locally with the same scripts CI uses:

```sh
cargo install cargo-packager --locked --version 0.11.8   # one-time
scripts/package.sh          # macOS / Linux  -> app/target/packages/
scripts\package.ps1         # Windows (PowerShell)
```

Builds are **unsigned dev builds** (not for distribution yet). Config, formats, prerequisites,
the Windows-port requirements, and the signing deferral are documented in
[`PACKAGING.md`](PACKAGING.md); the placeholder icons in
[`crates/freecell-app/packaging/icons/`](crates/freecell-app/packaging/icons/README.md).
