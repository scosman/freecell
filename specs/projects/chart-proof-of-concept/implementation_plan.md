---
status: draft
---

# Implementation Plan: Chart Proof of Concept

Per the PoC framing, architecture is folded in here as slim **design notes**, followed by
an ordered, early-bail **phase checklist**. Details live in `functional_spec.md`; this doc
is the build order.

---

## Design notes (slim architecture)

### Location & crate layout
All work under a new **`experiments/chart-poc/`** group (independent of `/app`, matching
the `experiments/` convention: independent Cargo projects + `findings.md` + committed
`results/`). Precedent for wiring gpui-component in an experiment: `experiments/04-ui-poc/`.

```
experiments/chart-poc/
  README.md                 # what this group is; how to run capture + review
  SYNTHESIS.md              # FINAL go/no-go assessment (written in the last phase)
  chart-model/              # shared lib crate — the OOXML-shaped data model (no gpui, no ironcalc)
  chart-render/             # Exp 2 + 3: chart widgets on gpui-component primitives,
                            #   example scenes, headless capture binary
    findings.md
    results/                # committed example PNGs
  load-save/                # Exp 1: zip second-pass parse + save re-injection
    findings.md
    results/                # sample xlsx round-trips (+ small fixtures)
```
`chart-model` is a path dependency of both `chart-render` and `load-save` — it is the seam
(Exp 1 parses into it; Exp 2/3 render from it). Keep each experiment its own Cargo project;
`chart-model` may be a tiny shared crate they both path-depend on.

### Dependency wiring (mirror `app/Cargo.toml`)
The render crate must use the **known-good pinned pair** (never bump one alone):
- `gpui` + `gpui_platform` at zed rev `1d217ee39d381ac101b7cf49d3d22451ac1093fe`
  (`gpui_platform` features `font-kit`, `x11`, `wayland`, `runtime_shaders`).
- `gpui-component` + `gpui-component-assets` at rev `a9a7341c35b62f27ff512371c62419342264710c`.
- `image = { version = "0.25", features = ["png"] }` + `png = "0.17"` for capture.
- `rust-version = "1.95"` (gpui needs `cold_path()`); set `profile.dev.package.gpui*
  opt-level = 3` so builds aren't glacial.
`chart-model` and `load-save` stay gpui-free. `load-save` uses the same libs `open_fixups.rs`
already pulls in — **`zip 0.6` + `roxmltree 0.19`** — plus `ironcalc =0.7.1` to exercise the
save path. Expect long first builds (heavy git deps) — call this out, don't fight it.

### Rendering approach (Exp 2/3)
Build our chart widgets over gpui-component's `plot/` **primitives** (`Line`/`Bar`/`Area`/
`Arc`/`Pie`/`Stack` + `ScaleLinear`/`ScaleBand` + axis/grid/label helpers) — *not* the
stock chart structs. We **own** the wrapper: layout box (plot area + margins), **chart
title**, **axis titles**, **numeric value-axis with a "nice" tick generator** (the linear
scale ships none — research), **legend** (swatch + series name, correct series→color
mapping), and **multi-series color cycle** over `chart_1..chart_5` (extend past 5).
Known traps to hit head-on (all from `research/`): multi-line needs one **shared**
`ScaleLinear` across series; **stacked area** needs an `Area` fork (scalar baseline) or
hand-rolled polygons + a normalize pass for percent; **pie** has no auto-palette so we
synthesize per-slice colors.

### Headless capture (Exp 2/3 validation)
Reuse the repo's proven Linux path rather than invent one: `app/render-tests/src/capture.rs`
(gpui window under `xvfb-run` + lavapipe, **`xrefresh`** to force presentation, capture by
window id) and/or the `round-3/C-ci-rendering` render→PNG pipeline. Because experiments stay
independent of `/app`, **copy/adapt** the capture logic into `chart-render` (a `capture` bin
+ a small module). Each example scene renders to `results/<name>.png` with a sidecar
manifest line describing what it should show.

### Agentic image review
The capture bin emits PNGs + a `manifest.json` (`{name, description, expectation}` per
image). A review step feeds each PNG + its expectation to a **Claude subagent** that returns
`{name, verdict: PASS|MARGINAL|FAIL, notes}` against the §6 rubric; results are written to
`results/review.md`. **Single verdict per image; a 3-agent majority panel on the Gate 1
image.** This is a build-time agentic step (the orchestrator spawns the reviewer agents),
not app code.

### Relaxed rigor (explicit)
Light tests only (a parse round-trips; a scene renders to a non-blank PNG of expected size;
tick generator produces sane ticks). Structural review only. No perf/robustness/style bar.
The **PNGs + agent review + `findings.md`** are the real evidence, not test coverage.

---

## Phases

Ordered so the worst dealbreaker is hit first (functional_spec §7). **Each phase ends with a
STOP/REASSESS checkpoint; a failed core gate can end the PoC with a NO-GO / PARTIAL-GO — do
not grind through later phases once a core capability is shown unachievable.**

- [ ] **Phase 0 — Enablement (M0).** Scaffold `chart-poc/`; `chart-model` crate (the §2
  data model); `chart-render` wired to the pinned gpui/gpui-component pair; a working
  headless **capture** bin; the **agent-review** step. Prove end-to-end on ONE trivial
  single-series bar → non-blank PNG, reviewer agent confirms "a bar chart." *Exit: the
  harness works; if capture can't be made to work headless, that itself is an early finding.*

- [ ] **Phase 1 — Gate 1: MAKE-OR-BREAK.** Multi-series **line** (2–4 series) with title +
  axis titles + numeric value-axis (nice ticks) + category axis + **legend**. Capture +
  **3-agent panel** review. **STOP: if panel FAIL → write NO-GO `SYNTHESIS.md` and end.**

- [ ] **Phase 2 — Gate 2: harder layouts.** Single column + horizontal bar; **grouped
  (clustered)** column; **stacked** + **100%-stacked** column; **stacked area** (Area fork);
  **pie** + **doughnut** with synthesized palette. Capture + single-agent review each.
  *Checkpoint: wholesale grouped/stacked FAIL → lean PARTIAL-GO (e.g. "single-series only").*

- [ ] **Phase 3 — Gate 3: scatter.** Single- and multi-series **scatter** (two numeric axes
  + dots), reusing the Phase 1/2 title/axis/legend scaffolding. Capture + review.
  *Checkpoint: FAIL → scatter recorded out-of-scope for the follow-on (not a whole NO-GO).*

- [ ] **Phase 4 — Gate 4: load/save stitching.** `load-save` crate: parse a real
  agent-authored `.xlsx` (a couple of in-scope chart types) into `chart-model` and render it
  via `chart-render`; **byte-preservation re-injection** on save + `open→save→reopen`
  round-trip. May run in parallel with Phases 1–3 (render leads). *Checkpoint: save FAIL →
  display-only recommendation; load FAIL is serious.*

- [ ] **Phase 5 — Synthesis.** Aggregate PNGs + review tables + per-experiment `findings.md`
  into `experiments/chart-poc/SYNTHESIS.md`: **GO / NO-GO / PARTIAL-GO**, recommended scope
  (types in/out, scatter in/out, display-only vs save-preservation), known risks, and a
  rough shape for the follow-on ship-quality project.
