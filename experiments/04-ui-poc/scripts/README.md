# Sub-project E scripts — macOS build / run / measure

One-command helpers to build, run, and measure the two UI PoC variants **on a Mac**
(macOS/Metal). The GPUI crates cannot build in the headless Linux CI container, so these
scripts are for the human's Mac run (functional_spec §6.E, architecture §7).

## Prerequisites (Mac)

- macOS with a Metal-capable GPU (any modern Mac).
- Rust toolchain (edition 2024 — a recent stable, e.g. ≥ 1.85).
- Xcode command-line tools (`xcode-select --install`).
- Network access (Cargo fetches the git-pinned `gpui` / `gpui-component`).

## Lock the gpui-component pin first (one time)

The `gpui-component` variant floats `main`; lock it so the run is reproducible:

```sh
git ls-remote https://github.com/longbridge/gpui-component refs/heads/main
# add rev = "<that sha>" to the gpui-component lines in ../gpui-component/Cargo.toml
```

(raw-gpui's `gpui`/`gpui_platform` are already pinned to a fixed Zed rev.)

## Interactive (feel test)

```sh
./build_and_run.sh raw-gpui          # or: gpui-component
```

Scroll and pan around; jump with the scrollbars. Judge whether it feels smooth. Then use
the app menu's **Run Test** item to run the measured perf test in-place.

## Measured "Run Test" (numbers)

```sh
./run_test.sh raw-gpui               # one variant
./run_test.sh both                   # both, back to back
```

Each run prints, and writes to `../results/<variant>-runtest.json`:

- `frame render p50/p99/max`
- `cell load p50/p99/max`
- three gates vs functional_spec §5.4:
  - `frame-p99` ≤ 8.33 ms (120 fps),
  - `frame-max` ≤ 16.67 ms (60 fps worst case),
  - `cell-load-p99` ≤ 2 ms,
- a final `VERDICT: PASS|FAIL`.

The app auto-quits when the scripted run finishes.

## What to report back

See the **"HUMAN RUN REQUIRED"** section of [`../findings.md`](../findings.md) for the
exact checklist: did each variant build, does each scroll smoothly, and the printed
PASS/FAIL + the `results/*.json` contents for both variants — so the raw-vs-gpui-component
comparison and the §5.4 verdict can be completed.
