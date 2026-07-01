# gpui-component variant — virtualized DataTable

A spreadsheet grid built on [`gpui-component`](https://github.com/longbridge/gpui-component)'s
virtualized `DataTable` + `TableDelegate`, over the same static provider as the
raw-gpui variant, so the numbers compare head-to-head.

Two findings surface from using the component (detailed in [`../findings.md`](../findings.md)):

- **Variable column widths: supported** — each `Column` carries its own width (from the
  provider's `col_width`), plus min/max + interactive resize.
- **Variable row heights: not supported** — `DataTable`'s vertical virtualization is
  `uniform_list`-based (fixed row height), so this variant renders a *uniform* row height
  while raw-gpui honours per-row heights. That difference is a comparison result, not a
  bug.

## macOS/Metal only

Depends on `gpui` + `gpui-component`, which **do not build in the headless Linux CI
container**. Build/run on a **Mac**. Testable logic lives in
[`../poc-core`](../poc-core) (CI-checked).

## Run it (from this directory, on a Mac)

Interactive:

```sh
cargo run --release
```

Use the **Run Test** menu item to run the scripted sequence; it prints measured
PASS/FAIL and writes `../results/gpui-component-runtest.json`.

Headless one-shot:

```sh
POC_DATE="$(date +%F)" POC_COMMIT="$(git rev-parse --short HEAD)" \
  cargo run --release -- --run-test
```

Or use `../scripts/build_and_run.sh gpui-component` and `../scripts/run_test.sh gpui-component`.

## Cargo pinning (IMPORTANT — lock before recording numbers)

`gpui` + `gpui_platform` are pinned to the Zed rev that `gpui-component`'s workspace
pins (`1d217ee3…`). The `gpui-component` dependency itself floats `main`; **before you
record numbers, lock it** to a known-good SHA so the run is reproducible:

```sh
git ls-remote https://github.com/longbridge/gpui-component refs/heads/main
# then add rev = "<that sha>" to the gpui-component lines in Cargo.toml
```

Keep the gpui rev and the gpui-component SHA a known-good pair (see `../findings.md`).

## Measurement note

The component scrolls by row/column index (`scroll_to_row` / `scroll_to_col`), so the
scripted pixel viewport is converted to the nearest index. Per-frame render time and
newly-visible-cell load latency are measured the same way as raw-gpui (same `poc_core`
harness + gates), so the two `results/*.json` are directly comparable.
