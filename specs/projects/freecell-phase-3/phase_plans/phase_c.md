---
status: complete
---

# Phase C: CI snapshot rendering

## Overview

Investigation C confirms whether FreeCell's north-star **rendering tests vs known-good
PNGs** are buildable — i.e. whether we can **capture a snapshot of the GPUI grid** and
**perceptually diff** it against a baseline. Per the spec (functional_spec §6-C,
architecture §5-C/§3, overview §5-C) this has two halves, and this phase does BOTH:

- **HALF 1 — in-container, authoritative (fully completed here):**
  1. Investigate GPUI's offscreen/headless capture surface at the pinned Zed rev; document
     with file:line whether windowless capture is possible in principle.
  2. Attempt a headless in-container GPUI build, time-boxed; capture the exact failure as
     the finding (no GPU/display). Do NOT fight the GPUI build.
  3. Implement + **test** the perceptual-diff harness — GPUI-independent, pure-Rust
     (`image` crate). Prove discriminating power: identical → pass; small AA/font-like
     perturbation within tolerance → pass; genuine change → fail.
- **HALF 2 — authored for the macOS human run (code + scripts + README, human executes):**
  4. A GPUI render→PNG harness (evolving `experiments/04-ui-poc/raw-gpui`) that renders the
     grid offscreen and writes a PNG, gated so it only builds on macOS.
  5. `findings.md` (§5.2 headings) grading the GATE honestly and a precise macOS hand-off
     checklist.

**The critical framing:** GPUI needs a real GPU → it cannot render in this container. So
the GATE ("a confirmed working snapshot-in-CI mechanism, demonstrated end-to-end") is
**partially met in-container** (perceptual diff proven; render authored + capture surface
investigated) with the **end-to-end render→PNG demonstration pending the human macOS run.**

### Headless-capture investigation result (the DISCOVERY, established during planning)

At the raw-gpui PoC's pinned Zed rev (`1d217ee39d381ac101b7cf49d3d22451ac1093fe`), GPUI
DOES expose an offscreen/windowless capture surface, but it is **macOS/Metal-only**:

- `PlatformHeadlessRenderer` trait — `render_scene_to_image(&mut self, &Scene,
  Size<DevicePixels>) -> Result<RgbaImage>` (`crates/gpui/src/platform.rs`, gated
  `#[cfg(any(test, feature = "test-support"))]`).
- `HeadlessAppContext` — `open_window(size, build_root)` + `capture_screenshot(window) ->
  Result<RgbaImage>` (which calls `window.render_to_image()`), built via `with_platform(
  text_system, asset_source, renderer_factory)`
  (`crates/gpui/src/app/headless_app_context.rs`).
- The only concrete renderer factory: `gpui_platform::current_headless_renderer()`
  (`crates/gpui_platform/src/gpui_platform.rs`) returns
  `Some(MetalHeadlessRenderer::new())` **only under `#[cfg(target_os="macos")]`**;
  `#[cfg(not(target_os="macos"))]` returns **`None`**. The concrete impl is
  `MetalHeadlessRenderer` (`crates/gpui_macos/src/metal_renderer.rs`).
- Consequence: on Linux the factory yields `None` → `capture_screenshot` /
  `render_to_image` bail ("no HeadlessRenderer"). There is **no CPU / blade / Vulkan
  headless path in this rev.** => **Verdict: capture is windowless-but-Mac-only.** CI
  mechanism = a **macOS runner** (GitHub Actions `macos-*`), no display needed thanks to
  the offscreen Metal path.

This means Phase C's macOS harness does NOT need a visible window — it renders offscreen
straight to an `RgbaImage`, a cleaner CI story than a windowed screenshot.

> **Impl note (reconciling this plan with the shipped harness):** during implementation I
> confirmed Zed's own visual-test runner (`crates/zed/src/visual_test_runner.rs`) uses the
> sibling context **`VisualTestAppContext::with_asset_source(gpui_platform::current_platform(
> false), asset_source)`**, where the platform supplies the (Metal) headless renderer
> internally, rather than the `HeadlessAppContext::with_platform(..., renderer_factory)`
> form sketched above. Both are real GPUI test APIs at this rev and both reach the same
> `Window::render_to_image()` capture; the shipped `render-grid/src/main.rs` uses the
> `VisualTestAppContext` path to mirror Zed's maintained reference exactly. `findings.md` is
> written against the shipped path.

## Steps

1. **Perceptual-diff harness (in-container, real + tested).** Add a `perceptual_diff`
   library module to the `ci_rendering` crate.
   - `Cargo.toml`: add `image = "0.25"` (pure Rust, builds in-container) and `anyhow`.
     Keep GPUI OUT of the default build — gate it behind a `mac-render` feature + a
     separate `[[bin]]` so the diff harness always builds+tests cleanly on Linux.
   - `src/lib.rs` exposing:
     - `pub struct DiffOptions { pub per_channel_tolerance: u8, pub fail_fraction: f64 }`
       with a `Default` (tolerance ~ small, e.g. 12/255; fail_fraction ~ 0.5%).
     - `pub struct DiffReport { pub width, height, total_pixels, differing_pixels:
       u64, pub differing_fraction: f64, pub max_channel_delta: u8, pub passed: bool }`.
     - `pub fn diff_images(a: &RgbaImage, b: &RgbaImage, opts: &DiffOptions) ->
       anyhow::Result<DiffReport>` — errors on dimension mismatch; a pixel "differs" if any
       channel delta > `per_channel_tolerance`; PASS iff `differing_fraction <=
       fail_fraction`. This is the tolerance-based perceptual metric (architecture §3): AA
       / font sub-pixel wiggle stays under tolerance, a genuine change trips the fraction.
     - `pub fn diff_png_files(a: &Path, b: &Path, opts) -> Result<DiffReport>` — loads two
       PNGs (`image::open` → `to_rgba8`) and diffs.
   - Rationale for a two-part metric (per-channel tolerance AND an allowed differing
     fraction): a pure count of changed pixels would false-fail on AA edges; a pure
     tolerance-only test could pass a large low-amplitude shift. The pair discriminates.
2. **Synthetic-PNG test fixtures + unit tests (in-container).** `tests/perceptual_diff.rs`
   generating PNGs in a tempdir (no committed binaries needed for the in-container proof;
   the macOS baseline PNG is committed by the human):
   - identical images → PASS (0 differing).
   - a **within-tolerance AA/font-like perturbation** (add +/- a few levels of noise below
     tolerance to a fraction of pixels, plus a 1px sub-pixel-like shift on text-ish edges)
     → PASS.
   - a **genuine change** (recolor a block / move a filled rectangle — simulating a real
     render regression) → FAIL.
   - a **dimension mismatch** → `Err`.
   - boundary: fraction exactly at threshold behavior is deterministic.
   Provide a small pure-Rust image generator in the test (checkerboard / rectangles /
   text-like stripes) so fixtures are code-generated + reproducible (architecture §3).
3. **Time-boxed in-container GPUI build attempt (the finding).** Add the `mac-render`
   feature + `render_grid` bin (Step 4) FIRST so there is something to build, then run
   ONCE, foreground, `timeout 900 cargo build --features mac-render` and capture the exact
   failure (missing system libs / linker / unresolved git dep behind the proxy). Record
   verbatim in findings; do not iterate on it.
4. **macOS render→PNG harness (authored, gated).** `src/bin/render_grid.rs` behind
   `#[cfg(feature = "mac-render")]` (and the bin only compiled with the feature), evolving
   `experiments/04-ui-poc/raw-gpui`:
   - Reuse the render-ready styling + layout logic conceptually from `poc_core`
     (`RenderCell`, `Axis`), but keep this crate self-contained (copy the minimal grid
     `Render` impl; do NOT edit 04-ui-poc). Pin `gpui` + `gpui_platform` to the SAME rev as
     `04-ui-poc/raw-gpui/Cargo.toml` (`1d217ee3...`), `gpui_platform` with
     `features=["font-kit", "test-support"]` so `current_headless_renderer()` +
     `render_to_image` are available; `image` for PNG encode.
   - `main`:
     - build a `HeadlessAppContext` via `with_platform(text_system, asset_source, ||
       gpui_platform::current_headless_renderer())`,
     - `open_window(size, |_win,_cx| cx.new(|_| Grid::new(scene)))`,
     - `run_until_parked()`, `capture_screenshot(window)` → `RgbaImage`, save PNG to
       `results/<name>.png`.
     - A `--scene {baseline|changed}` flag renders either the normal grid or a
       deliberately-changed grid (e.g. a shifted/recolored cell) to prove diff power.
     - A `--diff <a.png> <b.png>` subcommand reuses the `perceptual_diff` lib so the human
       runs render + diff from one binary; exit non-zero on FAIL.
   - The `Grid` view is a minimal absolute-positioned cell grid (mirrors 04-ui-poc
     `grid.rs`) drawing a small fixed scene (a handful of rows/cols with text, a highlight
     fill, a bold cell) — deterministic, enough to be a meaningful snapshot, small enough
     to render fast.
5. **Build/run scripts + README (macOS hand-off).**
   - `scripts/render_and_diff.sh`: on macOS, `cargo run --features mac-render --bin
     render_grid -- --scene baseline` → `results/baseline.png`; re-render →
     `results/rerender.png`; `--diff baseline.png rerender.png` (must PASS); `--scene
     changed` → `results/changed.png`; `--diff baseline.png changed.png` (must FAIL).
   - README §"HUMAN RUN REQUIRED (macOS)" with the EXACT commands and the expected
     pass/fail outcomes that close the GATE.
6. **findings.md (§5.2 headings).** Questions / What was done / Results-evidence /
   Conclusion / Recommended mechanism + alternative / Risks / HUMAN RUN REQUIRED. State the
   GATE grade honestly (diff proven in-container; render+capture surface investigated +
   authored; end-to-end render→PNG pending macOS). DISCOVERY: headless works but is
   **Mac-only** (Metal), so CI = macOS runner (offscreen, no display needed).

## Tests

All in-container, on the GPUI-independent `perceptual_diff` lib (the `mac-render` bin is
macOS-gated and not built/tested here):

- `identical_images_pass` — same image twice → `passed == true`, `differing_pixels == 0`.
- `within_tolerance_perturbation_passes` — sub-tolerance per-channel noise on a fraction of
  pixels + a 1px edge wiggle (AA/font proxy) → `passed == true` (differing_fraction under
  fail_fraction); asserts `max_channel_delta <= tolerance` for the noise case.
- `genuine_change_fails` — a recolored/moved block (real regression proxy) → `passed ==
  false` and `differing_fraction` well above threshold.
- `dimension_mismatch_errors` — different sizes → `diff_images` returns `Err`.
- `threshold_is_discriminating` — a perturbation just under fail_fraction passes and one
  just over fails, proving the metric is not a rubber stamp.
- `png_roundtrip_diff` — write two PNGs to a tempdir, `diff_png_files` agrees with the
  in-memory `diff_images` (proves the file-loading path).
- `format_value`-style: NA (no engine here).
