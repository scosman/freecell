# Investigation C — CI snapshot rendering (GPUI grid → PNG → perceptual diff)

> **GATE CLOSED (2026-07-02).** The human ran `scripts/render_and_diff.sh` on
> macOS/Metal: the offscreen render→PNG→perceptual-diff pipeline works **end-to-end** —
> stable re-render PASSES (pixel-identical), deliberate change FAILS (3.47% differing vs
> the 0.5% threshold). PNGs committed under `results/`; numbers in *Results* below.
> One line of gpui API drift surfaced and was fixed (exactly the anticipated risk).
> Original honest-grade text preserved below.
>
> Phase-3 investigation C (functional_spec §6-C, architecture §3 / §5-C, overview §5-C).
> FreeCell's north star includes **rendering tests vs known-good PNGs**, which requires
> **capturing a snapshot of the GPUI grid in CI** and **perceptually diffing** it against a
> baseline (fuzzy match acceptable; a real change must still fail). GPUI needs a real GPU,
> so this investigation has two halves — an **in-container, authoritative** half (the
> headless-capture-surface investigation + a proven perceptual-diff harness) and a
> **macOS/human-run** half (the offscreen render→PNG harness, authored here, executed on a
> Mac). Every in-container claim below is backed by a runnable probe (`cargo test`, 6
> assertions) or a cited GPUI source location at the pinned Zed rev; the render half is
> authored against that same rev and mirrors Zed's own visual-test runner.

## Question(s)

1. **Can GPUI render the grid offscreen / headless to an image (texture → PNG) without a
   window/display?** Is a windowless capture even *possible* in principle at our pinned rev?
   *(DISCOVERY — does headless work, or is it Mac-CI-only?)*
2. **If not headless-anywhere, what is the confirmed CI path?** (a macOS runner with a real
   or virtual display, or offscreen Metal?) *(GATE — confirm at least ONE viable mechanism
   end-to-end: render → PNG → perceptual diff within tolerance.)*
3. **Does the perceptual diff have real discriminating power?** Identical images pass; an
   AA/font-like within-tolerance perturbation passes; a genuine change fails. *(GATE.)*

## What was done

Two crates in `experiments/round-3/C-ci-rendering/`, deliberately split so the in-container
deliverable never touches GPUI:

- **`ci_rendering` (parent, in-container)** — a pure-Rust library (`image` + `anyhow`
  only), `src/lib.rs`:
  - `DiffOptions { per_channel_tolerance: u8, fail_fraction: f64 }` (defaults 12/255,
    0.5%).
  - `diff_images(a, b, opts) -> DiffReport` — a pixel *differs* iff some channel delta
    exceeds `per_channel_tolerance`; PASS iff the **fraction** of differing pixels is
    `<= fail_fraction`; errors on a dimension mismatch. `diff_png_files(a, b, opts)` loads
    two PNGs and diffs. `DiffReport` carries width/height, differing count + fraction, max
    channel delta, and `passed`.
  - **Why two knobs, not one:** a pure changed-pixel **count** false-fails on anti-aliasing
    edges; a pure per-pixel **tolerance** can pass a large low-amplitude shift. Tolerance +
    fraction together absorb AA/font wiggle while catching a real regression — the
    discriminating power the GATE requires. **This is the same metric shape Zed's own
    visual tests use** (`compare_screenshots` / `pixels_are_similar` in
    `crates/zed/src/zed/visual_tests.rs`: per-channel threshold + a match-percentage), so we
    are not inventing a bespoke metric.
  - `tests/perceptual_diff.rs` — **6 code-generated-fixture tests** (below). No committed
    binaries in-container; the macOS baseline PNG is committed by the human.
- **`render-grid/` (nested, macOS-only)** — a separate Cargo package (`src/main.rs`)
  depending on the parent `ci_rendering` + the pinned `gpui`/`gpui_platform`. It renders a
  minimal 5×4 FreeCell grid (white bg, grey gridlines, per-cell fill + bold, header strip —
  the raw-gpui PoC's render style) **offscreen** via GPUI's headless capture surface and
  saves a PNG; `--scene {baseline|changed}` picks the normal or a deliberately-changed grid,
  and `--diff a b` reuses the parent diff (exit 1 on FAIL) so render + diff live in one
  binary. It mirrors Zed's own `crates/zed/src/visual_test_runner.rs` on the
  construction/capture path.
- **`scripts/render_and_diff.sh`** — the macOS run that closes the GATE end-to-end
  (baseline vs re-render MUST PASS; baseline vs changed MUST FAIL).

**Reproduce (in-container):** from `experiments/round-3/C-ci-rendering/`: `cargo test`
(6 pass), `cargo clippy --all-targets -- -D warnings` (clean), `cargo fmt --check` (clean).
**Reproduce (macOS, human):** `./scripts/render_and_diff.sh`.

### GPUI headless-capture surface at the pinned rev (source-cited)

The raw-gpui PoC pins Zed rev **`1d217ee39d381ac101b7cf49d3d22451ac1093fe`**
(`experiments/04-ui-poc/raw-gpui/Cargo.toml`). At that rev, GPUI **does** expose a real
offscreen/windowless capture surface — but it is **Metal-backed and macOS-only**:

- **The trait.** `PlatformHeadlessRenderer` with
  `render_scene_to_image(&mut self, &Scene, Size<DevicePixels>) -> Result<RgbaImage>` and
  `render_scene(...)` — gated `#[cfg(any(test, feature = "test-support"))]`
  (`crates/gpui/src/platform.rs`). The `render_scene` doc says it does "the same CPU-side
  scene encoding and GPU submission as drawing to a real window" — i.e. real GPU work, not a
  stub.
- **The capture entry point.** `VisualTestAppContext::capture_screenshot(&mut self,
  AnyWindowHandle) -> Result<RgbaImage>`, which calls `Window::render_to_image()`
  (`crates/gpui/src/app/visual_test_context.rs`). `render_to_image` delegates to the
  window's `renderer.render_scene_to_image(...)` and **bails if no headless renderer is
  configured** (`crates/gpui/src/platform/test/window.rs`).
- **The only concrete factory.** `gpui_platform::current_headless_renderer()` returns
  `Some(Box::new(gpui_macos::metal_renderer::MetalHeadlessRenderer::new()))` **only** under
  `#[cfg(target_os = "macos")]`, and `#[cfg(not(target_os = "macos"))] { None }`
  (`crates/gpui_platform/src/gpui_platform.rs`). The concrete impl is `MetalHeadlessRenderer`
  (`crates/gpui_macos/src/metal_renderer.rs`, `impl PlatformHeadlessRenderer`).
- **Consequence.** On **Linux/Windows the factory yields `None`** → `render_to_image`
  bails ("no HeadlessRenderer"). There is **no CPU / blade / Vulkan headless path** wired in
  this rev. So the answer to Q1 is: **windowless capture is possible, but macOS/Metal-only.**
- **Precedent.** Zed ships exactly this: `crates/zed/src/visual_test_runner.rs` builds a
  `VisualTestAppContext::with_asset_source(gpui_platform::current_platform(false), ...)`,
  opens a `show: false` window, `run_until_parked()` → `refresh()` → `run_until_parked()` →
  `capture_screenshot(window.into())` → `RgbaImage::save(png)`. Our `render-grid` follows
  this precisely, so it tracks a maintained reference API, not a speculative one. The
  runner is itself `#[cfg(target_os = "macos")]`.

## Results / evidence

### In-container (authoritative)

- **Perceptual-diff harness — `cargo test` → 6 passed** (`tests/perceptual_diff.rs`):
  - `identical_images_pass` — same image → PASS, 0 differing, max delta 0.
  - `within_tolerance_perturbation_passes` — sub-tolerance per-channel jitter on
    edge/gridline pixels (AA/font proxy) → PASS, 0 counted differing, `max_channel_delta <=
    tolerance` and `> 0` (perturbation is real).
  - `genuine_change_fails` — a recolored interior block (real regression proxy) → FAIL,
    differing fraction > 5× the threshold, max delta > tolerance.
  - `dimension_mismatch_errors` — different sizes → `Err` (a size change is a hard failure,
    not fuzzy).
  - `threshold_is_discriminating` — a change touching **exactly** `fail_fraction` of pixels
    PASSES and **one pixel more** FAILS — proving the metric is not a rubber stamp.
  - `png_roundtrip_diff` — write two PNGs to a tempdir; `diff_png_files` equals the
    in-memory `diff_images` (the file-loading CI path is exercised).
  - `clippy -D warnings` clean; `fmt --check` clean.
- **In-container GPUI build attempt (the finding).** One time-boxed `timeout 900 cargo
  build` of `render-grid/` in the container **fails at dependency resolution**, before any
  compilation or linking:

  ```
  error: failed to get `gpui` as a dependency of package `render_grid` ...
    Unable to update https://github.com/zed-industries/zed?rev=1d217ee3...
    revision 1d217ee3... not found
    failed to receive HTTP 200 response: got 403; class=Net
  ```

  `git ls-remote https://github.com/zed-industries/zed` → `HTTP 403` via the container
  proxy. The session's GitHub access is **scoped to `scosman/freecell`** (overview §7), so
  the Zed source is unreachable — Cargo cannot even *fetch* GPUI, let alone hit the (also
  real) no-GPU/no-display wall. **We did not fight this** (spec: don't fight the GPUI build);
  the failure mode is the recorded finding. Two independent reasons make an in-container GPUI
  render impossible here: (1) source fetch is proxy-denied; (2) no GPU/display for Metal
  (and no Linux headless renderer exists in this rev anyway).

### macOS (human-run) — COMPLETED 2026-07-02

The human ran `scripts/render_and_diff.sh` on a macOS/Metal machine. Build required one
fix first: gpui API drift at the pinned rev — `cx.new()` needs the `AppContext` trait in
scope (`use gpui::AppContext as _`, `render-grid/src/main.rs`) — exactly the "small API
drift on first macOS build" failure mode anticipated under *Risks*. After the fix the
script completed and reported **"GATE CLOSED: offscreen render → PNG → perceptual diff
works end-to-end on macOS; stable re-render PASSES within tolerance and a deliberate
change FAILS."** The offscreen render needed no visible window.

All three PNGs are committed under `results/` (`baseline.png`, `rerender.png`,
`changed.png`). The diff lines, reproduced **in-container** from those committed PNGs via
`cargo run --example diff_committed` (same `DiffOptions::default()` as the macOS `--diff`
path — the diff half is GPUI-free, so the committed artifacts are re-verifiable anywhere):

```
baseline vs rerender: 800x328 px: 0 differing (0.0000%), max channel delta 0 -> PASS
baseline vs changed:  800x328 px: 9093 differing (3.4653%), max channel delta 229 -> FAIL
```

Two observations worth recording:

- **Same-machine Metal offscreen rendering is fully deterministic** — the re-render is
  **pixel-identical** (0 differing pixels, max channel delta 0), stronger than
  "within tolerance." Corollary: the tolerance defaults (12/255, 0.5%) absorbed zero
  same-machine noise, so their headroom is **untested against cross-machine variation**
  (different Mac/GPU/font versions) — the "capture baselines on the same CI runner class"
  risk below is now the *only* tolerance risk, and the first cross-machine CI run should
  tune from real deltas.
- The deliberate change lands at **3.47% differing / max delta 229** — ~7× over the 0.5%
  fail threshold, a decisive margin confirming discriminating power on real Metal output,
  not just the synthetic test images.

## Conclusion (GATE grade — honest)

- **DISCOVERY (Q1) — ANSWERED.** GPUI **can** capture windowless/offscreen, but the headless
  renderer is **Metal-only / macOS-only** at our pinned rev (`current_headless_renderer()`
  is `None` off-macOS; no blade/Vulkan/CPU headless path). So headless capture does **not**
  work in the Linux container — the CI path is a **macOS runner**, but a *good* one:
  rendering is **offscreen (`show: false`)**, so **no display/virtual framebuffer is
  needed**, just a `macos-*` runner with the Metal stack (standard on GitHub Actions macOS
  runners).
- **GATE (Q2 + Q3) — PARTIALLY MET in-container; end-to-end demonstration PENDING the macOS
  run.**
  - **Q3 (perceptual diff has discriminating power): MET, in-container, proven.** 6 tests;
    identical → PASS, within-tolerance AA/font wiggle → PASS, genuine change → FAIL,
    at-threshold vs over-threshold discriminates. Same metric shape as Zed's own visual
    tests.
  - **Q2 (a confirmed working snapshot-in-CI mechanism): INVESTIGATED + AUTHORED, not yet
    demonstrated end-to-end.** The offscreen render→PNG path is source-confirmed to exist at
    the pinned rev, authored against it (mirroring Zed's runner), and wired to the proven
    diff — but the **actual render→PNG** step cannot run in this container and is **pending
    the human macOS run.** No off-ramp fired: a viable mechanism is identified and the code
    is in place; what remains is execution, not a redesign.
- **Net:** the GATE ("a confirmed, working snapshot-in-CI mechanism, demonstrated
  end-to-end") is **met on its provable in-container portion (the diff) and the
  investigation/authoring portion (the render), with the end-to-end render→PNG→diff
  demonstration outstanding on macOS.** State plainly at the checkpoint: **diff proven;
  render authored + capture-surface confirmed; end-to-end pending the Mac run.**

> **Update 2026-07-02 — GATE (Q2) now MET end-to-end.** The macOS run executed the full
> render→PNG→diff pipeline: re-render PASS (pixel-identical), changed-scene FAIL (3.47%
> vs 0.5% threshold). See *Results §macOS (human-run)*. The GATE is closed in full; the
> Phase-3 synthesis's "not yet demonstrated" caveat is superseded. Residual caveat: this
> demonstrated the mechanism against the minimal fixed scene, not the real FreeCell grid
> (which doesn't exist yet) — baselines for the real grid land during the build, on the
> CI runner class that validates them.

## Recommended CI mechanism + alternative

- **Recommended: a macOS CI runner doing OFFSCREEN Metal capture** via
  `VisualTestAppContext` + `current_headless_renderer()` → `capture_screenshot` → PNG →
  the `ci_rendering` perceptual diff (tolerance + fraction). No visible window / no display
  server required (`show: false`), so it fits an unattended CI job. This is the path Zed
  itself uses for visual tests at this rev, so it is maintained and battle-tested upstream.
- **Alternative (if a non-Mac runner is ever mandated):** there is **no** headless GPU
  capture in this GPUI rev off-macOS. Options, in rough order: (a) a Linux runner with a
  real/virtual GPU once GPUI grows a blade/Vulkan `PlatformHeadlessRenderer` (not present
  now — would need an upstream contribution or a rev bump); (b) render the grid content
  through a **non-GPUI** rasterizer for CI (a second render path — real scope, and it would
  test a different renderer than production, so weaker); (c) drive a visible window under a
  virtual display and screen-capture (heavier, and still macOS for Metal). **Recommend (a)-
  later / stick with the macOS runner now.**

## Risks / open questions

- **GPUI API drift on the render half.** `render-grid` is authored against the pinned rev
  from source reading (the container can't compile it). Small drift is possible when the
  human first builds on macOS (e.g. `VisualTestAppContext` constructor name,
  `WindowOptions` fields, `open_window` closure shape). It mirrors Zed's own
  `visual_test_runner.rs` at this rev to minimize that; the README says to paste the first
  Cargo error if it drifts.
- **Font resolution on the Mac.** The harness relies on font-kit resolving a system font on
  macOS for text shaping; if a machine renders no glyphs, the human can embed a `.ttf` and
  `cx.text_system().add_fonts(vec![Cow::Borrowed(include_bytes!(...))])` (noted in code).
  Cell geometry/fills still render regardless, so the snapshot is meaningful even in that
  edge case.
- **Tolerance tuning.** Defaults (12/255 per channel, 0.5% fraction) are a starting point
  chosen without real Metal AA in hand; the first macOS re-render-vs-baseline PASS confirms
  they are not too tight, and the changed-scene FAIL confirms they are not too loose. Tune
  from the printed `max channel delta` / `differing %` if needed.
- **Baseline portability across machines.** A baseline captured on one macOS/GPU may differ
  from another's by more than tolerance (different Metal AA / font versions). The build
  should capture baselines on the **same CI runner class** it validates against — a standard
  visual-testing caveat, flagged for the build.

## HUMAN RUN REQUIRED (macOS)

> **COMPLETED 2026-07-02** — see *Results §macOS (human-run)*. Build succeeded after the
> one-line `AppContext` import fix; both diff expectations held; PNGs committed; no
> visible window needed. Original ask follows.

See `README.md` "HUMAN RUN REQUIRED (macOS)". In short, on a macOS/Metal machine from
`experiments/round-3/C-ci-rendering/`:

```sh
./scripts/render_and_diff.sh
```

Report: did `render-grid` build (paste the first error if not); the two printed diff lines
(the PASS for baseline-vs-rerender and the FAIL for baseline-vs-changed); and confirm the
render needed no visible window. Commit `results/baseline.png`. That closes the C GATE
end-to-end and lets the Phase-3 synthesis mark C: **mechanism = macOS-runner offscreen
Metal capture + tolerance-based perceptual diff, demonstrated end-to-end.**
