# raw-gpui variant — custom virtualized grid on raw gpui

A custom spreadsheet grid built directly on raw `gpui` primitives (no `gpui-component`).
Virtualization is ours: [`poc_core::Axis`](../poc-core/src/layout.rs) maps the scroll
offset to the visible row/col range over variable sizes (segment-summed prefix sums +
binary search), and each visible cell is an absolutely-positioned `div`. Only viewport +
overscan cells exist per frame, so the Excel-max grid (1,048,576 × 16,384) renders a
viewport's worth of elements.

## macOS/Metal only

This crate depends on `gpui`, which **does not build in the headless Linux CI
container** (no GPU/display, heavy system deps). Build and run it on a **Mac**. The
engine-neutral, testable logic lives in [`../poc-core`](../poc-core) and is CI-checked.

## Run it (from this directory, on a Mac)

Interactive (scroll/pan to judge feel):

```sh
cargo run --release
```

Then use the **Run Test** menu item (app menu → "Run Test") to run the scripted
scroll/jump sequence; it prints measured PASS/FAIL and writes `../results/raw-gpui-runtest.json`.

Headless one-shot (auto-runs the test and quits):

```sh
POC_DATE="$(date +%F)" POC_COMMIT="$(git rev-parse --short HEAD)" \
  cargo run --release -- --run-test
```

Or use the top-level convenience scripts: `../scripts/build_and_run.sh raw-gpui` and
`../scripts/run_test.sh raw-gpui`.

## What "Run Test" measures (functional_spec §5.4)

- **frame-p99** ≤ 8.33 ms (sustained 120 fps)
- **frame-max** ≤ 16.67 ms (never worse than 60 fps under fast scroll / jump)
- **cell-load-p99** ≤ 2 ms (pulling values + formatting for newly-visible cells)

`POC_DATE` / `POC_COMMIT` stamp the recorded JSON (recording never reads a wall clock;
architecture §3). If unset they record as `"unknown"`.

## Cargo pinning

`gpui` + `gpui_platform` are git-pinned to the same Zed rev that `gpui-component` pins
(see the header comment in `Cargo.toml`). If Cargo cannot resolve the rev, confirm the
known-good pairing per `../findings.md` ("Cargo pinning").
