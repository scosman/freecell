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
cargo run -p freecell-app        # the hello-world window (Phase 1)
cargo test --workspace           # core + engine + app logic tests
cargo fmt --all --check          # formatting gate
cargo clippy --workspace --all-targets -- -D warnings   # lint gate
cargo deny check                 # licenses/advisories (see deny.toml)
```

## Render tests & baselines

The cell-render snapshot suite (`render-tests/`, Phase 7) renders the real grid, captures
PNGs, and perceptually diffs them against committed baselines. On Linux CI it runs under
**Xvfb + Mesa lavapipe** (software Vulkan). Phase 1 ships the **capture spike** that
validates that path:

```sh
app/scripts/linux_render_spike.sh   # renders the hello-world under Xvfb+lavapipe, captures a PNG
```

The full baseline workflow (regenerate on the pinned runner image, eyeball every changed
PNG, commit) is documented in `render-tests/README.md`.

## CI

GitHub Actions live at the repo root (`../.github/workflows/`):

- **checks** (Linux, required): fmt, clippy `-D warnings`, build, test, render (spike in
  Phase 1 → render suite in Phase 7), cargo-deny.
- **perf-gates** (Linux, required): the perf harness with buffered thresholds (Phase 12).
- **macos-verify** (manual/weekly, non-required): build + test + render smoke on macOS.
