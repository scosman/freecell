# chart-render — findings

Phase 0 (M0 — Enablement). Goal: prove the whole harness end-to-end — the pinned
gpui / gpui-component stack builds and paints, a chart renders headless to a non-blank PNG,
and an agent reviewer confirms it. Scope kept trivial: **one single-series column chart**.

## Result: ENABLEMENT PASS

The harness works end-to-end in this Linux container:
pinned gpui/gpui-component build → chart widget over the `plot/` primitives paints →
headless capture to `results/bar_single.png` (640×440, 1447 unique colors, non-blank) →
reviewer agent verdict **PASS** ("a recognizable, non-blank bar chart"; see `results/review.md`).
The make-or-break render-quality question (multi-series line + legend a user would accept) is
Gate 1 (Phase 1), not this phase.

## What worked

- **Pinned dependency pair builds unchanged.** The exact `app/Cargo.toml` pins
  (gpui/gpui_platform @ zed `1d217ee3…`, gpui-component/-assets @ `a9a7341c…`, `image` 0.25,
  `png` 0.17) build here with the 1.95.0 toolchain and `profile.dev.package.gpui* opt-level = 3`.
  First build is slow (heavy git deps), as expected. A `rust-toolchain.toml` pinning `1.95.0`
  under `chart-poc/` was required — the app's pin does not reach `experiments/`.
- **Building over the `plot/` primitives is viable.** `bar.rs::BarPlot` is a
  `#[derive(IntoPlot)] + impl Plot` over the raw `Bar` + `ScaleBand` + `ScaleLinear` +
  `PlotAxis` + `Grid` primitives — the pattern the library's own hand-rolled `stacked_bar_chart`
  story uses ("You can draw any chart you want by using the `Plot`"). So Phase 0 already
  exercises the Gate 1 approach rather than leaning on the stock `BarChart` struct.
- **We own the value axis.** `ticks.rs::NiceScale` (Heckbert "nice numbers") generates the
  rounded domain + tick step that the library's `ScaleLinear` does NOT provide. Feeding that
  domain into `ScaleLinear::new(vec![nice.min, nice.max], vec![bottom, top])` makes the bars
  and the tick labels share one scale, so they line up — the fix for the research-flagged
  "each stock chart normalizes its own domain, no nice ticks" trap. The captured chart shows
  0/50/100/150/200 at even intervals with gridlines, bars landing correctly against them.
- **We own title / axis titles / legend.** These live in a plain gpui `div` layout around the
  plot element (the stock structs have none). Legend swatch colors come from the same
  `palette.rs` cycle the bars use, so series→color mapping is correct by construction.
- **Theme-independent colors.** The widget passes explicit colors to the plot primitives
  instead of reading `cx.theme()`, so the capture is deterministic and high-contrast regardless
  of the ambient (possibly dark) gpui-component theme.

## Headless capture — the key risk, retired

gpui has **no windowless GPU capture on Linux** at the pinned rev (that path is macOS/Metal
only — see `round-3/C-ci-rendering/findings.md`). So we reuse the repo's proven **on-screen**
path (`app/render-tests/src/capture.rs`), copied into `capture.rs`: a real gpui window under
`xvfb-run` + Mesa **lavapipe** (software Vulkan), `xrefresh` to force presentation, screenshot
the window by id with ImageMagick `import`.

**Container setup needed (was missing; installed via apt):**

    apt-get install -y mesa-vulkan-drivers x11-xserver-utils x11-utils imagemagick
    # and, for the gpui link step (below):
    apt-get install -y libxkbcommon-dev libwayland-dev libxcb1-dev libx11-dev

- The Vulkan **loader** (`libvulkan.so.1`) ships in the base image, but there was **no ICD /
  driver** — `/usr/share/vulkan/icd.d/` did not exist. `mesa-vulkan-drivers` installs the
  lavapipe ICD (`lvp_icd.json`); `vulkaninfo` under Xvfb then reports an `llvmpipe` device.
- `xrefresh` (x11-xserver-utils), `xwininfo` (x11-utils), and `import` (imagemagick) were all
  missing from the base image and are all load-bearing for the capture path.
- **Linking gpui failed until `libxkbcommon-dev` was installed** (`rust-lld: unable to find
  library -lxkbcommon`); gpui_platform's x11/wayland backends link against it. Installed the
  wayland/xcb/x11 dev libs alongside it.

**Load-bearing details inherited from the proven path:** each scene renders under its **own**
Xvfb display sized to the viewport (+8px) — lavapipe only *presents* when the window nearly
fills the screen; `xrefresh` forces the Expose gpui needs to present (Xvfb has no compositor);
a blank-guard rejects any capture with ≤1 unique color.

## What was hard / notable

- **Discovering the container gaps.** The base image had the Vulkan loader but no driver, none
  of `xrefresh` / `xwininfo` / `import`, and no `libxkbcommon-dev` for the gpui link. These are
  the things a fresh container needs before capture works; now documented in `README.md`.
- **`ScaleBand::tick` ignores its range start (a real gotcha, found and fixed).** It positions
  bands from `0`, not from `min(range)`. Passing `[plot_left, plot_right]` therefore slid the
  bars left into the value-axis gutter, and the bars (painted last) covered the 50/100 tick
  labels. Fix: give the band scale the available *width* (`[0, plot_right-plot_left]`) and add
  `plot_left` to every band position (bar cross + category label x). After the fix all five
  value labels render and the bars sit correctly right of the axis. (This is exactly how the
  library's own `BarChart` uses a 0-based band range — the value gutter is our addition.)
- **Primitive API is closure/`'static`-heavy.** `Bar`'s accessor closures are `'static`, so the
  widget must move owned clones (`ScaleBand`/`ScaleLinear` are `Clone`) into them rather than
  borrow `self` — mirrors how the library's own `BarChart::paint` does it.
- **Value type locked to `f64`.** `ScaleLinear`'s value type is a sealed trait (`f64` or
  `Decimal` only). Our data model already stores values as `f64`, so a non-issue for us.
- **Paint order matters.** Grid → axis+labels → bars. Bars paint last, so anything a bar
  overlaps is covered (this is what surfaced the band-range bug above).

## Agentic review

`capture` writes `results/manifest.json` (`{name, png, description, expectation}` per image).
A fresh reviewer sub-agent is given each PNG + its `expectation`, judges it against the §6
rubric, and the verdict is recorded in `results/review.md`. Phase 0 = single verdict ("a bar
chart, non-blank"); Gate 1 upgrades this to a 3-agent majority panel on the make-or-break image.

## Tests (light, per relaxed rigor)

- `chart-model`: color hex round-trip, category labels, series length, whole-chart round-trip
  through the public API (the seam's in-memory shape). 4 tests.
- `ticks`: nice-scale covers the data with a 1/2/5×10^k step; ticks span the domain evenly;
  value axis includes the zero baseline; degenerate inputs don't panic/loop; fraction mapping;
  tick formatting.
- `palette`: first five = the base cycle; >5 stays distinct; HSL round-trip.
- `scenes`: every scene is name-lookupable with non-empty metadata; the Phase-0 scene is a
  single-series column.
- The **PNG + agent review** in `results/` is the real evidence, not test coverage.
