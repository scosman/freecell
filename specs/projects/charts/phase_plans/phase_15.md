---
status: draft
---

# Phase 15: Regression + external round-trip CI + line perf hardening

## Overview

P15 is the **final hardening phase** that proves the line-chart pipeline
(render → fidelity → robust → CI) end-to-end on one type, per `implementation_plan.md` P15
and `architecture.md §5/§7`. Three deliverables, in priority order:

1. **Diagnose & fix the failing CI `render` gate** (HIGHEST — the exit criterion needs a GREEN
   CI render run). The `render` workflow fails FAST on the runner: the shared render-all aborts
   on the FIRST case (`border_all_thin`) with `capture failed (exit Some(1))` and **empty
   stderr** — all ~81 pixel cases then report the same abort. Our LOCAL container (also
   ubuntu-24.04, same `setup_render_env.sh` + `render_tests.sh`) renders all 88 cases green, so
   this is a **runner-environment capture failure**, not chart code and not pixel drift. The
   capture harness **swallows** the failing subprocess's stderr (`xvfb-run`'s Xvfb error file
   defaults to `/dev/null`; the render binary's output is redirected to `/dev/null` inside the
   capture script), so the CI log reveals nothing. Fix = make it **diagnosable**, dispatch to
   read the real error, then fix the **root cause** in `setup_render_env.sh` / `render.yml` /
   the harness — never by weakening the gate.

2. **External round-trip CI for line (LibreOffice headless).** Excel can't run in CI but
   LibreOffice can (`soffice --headless`). Wire a CI check that takes a FreeCell-saved
   line-chart `.xlsx` and opens/converts it with headless LibreOffice, asserting it loads
   without error and the chart part survives — the "external round-trip … wired into CI for
   line" deliverable (`implementation_plan.md` P15, `architecture.md §7` real-file corpus).

3. **Regression + perf hardening (local, deterministic).** Perceptual-diff regression: confirm
   the committed line render scenes cover the slice; add a large-series / many-charts
   perceptual scene if it adds coverage. Perf: benchmark **many-line-charts** (K charts on a
   sheet) and **large-series** (a line with very many points), FOREGROUND under `timeout`,
   FORCE+ASSERT, report p50/p99 env-stamped; add any hardening the numbers motivate (large-
   series down-sample for paint while retaining full data for save, per `architecture.md §5`
   challenge 5) without regressing fidelity or save.

Exit: **a production-robust line chart** — the pipeline proven end-to-end on one type.

## Root-cause status of the CI failure (evidence)

- CI panic (run 29087414743, commit `ebfd383`): `rendering case border_all_thin: capture
  failed (exit Some(1)):` with **nothing** after the colon.
- `capture failed (exit {code}):\n{stderr}` is `capture.rs::capture_window`'s bail when
  `xvfb-run` exits non-zero. Exit `Some(1)`, empty stderr.
- Both the runner and our container are **ubuntu-24.04** with `xauth`, `xkb-data`,
  `/tmp/.X11-unix`, `lvp_icd.json`, ImageMagick 6 — so it is **not** a missing base package.
- Why stderr is empty: (a) `xvfb-run`'s `-e`/error-file defaults to `/dev/null`, so if **Xvfb
  itself** fails to start its diagnostic is discarded; (b) the capture script runs
  `{launch_cmd} >/dev/null 2>&1 &`, so if the **render binary** (Vulkan/lavapipe device
  creation) fails, its stderr is discarded; (c) the only unsuppressed script stderr is the
  no-window echo (→ rc=3, not 1) and `import`'s stderr. Exit 1 + empty stderr is therefore
  consistent with an **Xvfb-start failure** (its error → `/dev/null`) or a render-binary
  failure whose log we throw away. The harness must surface all three before we can know which.

## Steps

### Deliverable 1 — diagnosable capture + root-cause fix

1. **`app/render-tests/src/capture.rs` — surface the swallowed failure (keep local success
   behavior identical).**
   - `capture_window`: pass `xvfb-run` an explicit **error file** via `-e <path>` (a unique
     temp file) so Xvfb's own startup/`xauth` errors are captured instead of going to the
     default `/dev/null`. On a non-zero exit, include in the bail: the **exit code**, the
     **failing command context**, `xvfb-run` **stdout** (currently ignored) + **stderr**, and
     the **Xvfb error-file** contents. Read + delete the error file; on success it is unused
     (local behavior unchanged). The same enrichment covers both `render_all` (grid) and
     `render_charts` (chart) because both flow through `capture_window`.
   - `capture_script`: redirect the launched render binary to a **script-local log**
     (`APP_LOG=$(mktemp)`), not `/dev/null`; capture `import`'s stderr; and on ANY non-zero
     `rc`, echo a **diagnostic block to stderr** — which step failed, `DISPLAY`, whether the
     render process is still alive, the render binary's captured log, and `xwininfo -root
     -tree` (so a "no window" failure shows what DID present). On success, print nothing extra
     and exit 0 exactly as before. Clean up `APP_LOG` before exit.
   - This is diagnostics only — no behavior change on the passing (local) path. A unit test
     asserts the assembled failure message includes the exit code + the diagnostic sections
     (pure string assembly, factored into a helper so it is testable without a display).

2. **Dispatch `render.yml` on the branch, poll (~5-8 min fast-fail), read the real error.**
   Use `mcp__github__actions_run_trigger` (`run_workflow`, `render.yml`, ref
   `claude/charts-spec-implement-dcdypq`); poll `actions_list`/`actions_get`; on failure read
   `get_job_logs failed_only`. The improved harness now prints WHY the capture fails on the
   runner.

3. **Fix the root cause** in the right layer (NOT by weakening the gate):
   - missing runtime lib on ubuntu-24.04 → add to `setup_render_env.sh`;
   - Xvfb display-size/timing/cold-start → harden the harness (poll for the window instead of a
     single fixed sleep; raise settle budget) and/or `render.yml`;
   - lavapipe/Mesa ICD path or Vulkan device creation → fix the ICD/env in
     `setup_render_env.sh`/`render.yml`/the launch env.
   Re-dispatch + re-poll to confirm the capture now **proceeds past** `border_all_thin`
   (renders instead of aborting at ~40s). A FULL green run is ~45 min — confirm the capture
   SUCCEEDS (proceeds), report, and hand the final full-green confirmation to the manager.
   - If, after a couple of diagnostic+fix attempts, the failure is clearly a GitHub-runner
     environment incompatibility outside our code, **STOP**, document it precisely as a GAP in
     `GAPS.md` (Charts render-fidelity / CI section) with the observed error + what would
     unblock it, and surface it in the summary — do **not** weaken/disable the required gate.

### Deliverable 2 — LibreOffice external round-trip (local test + CI wiring)

4. **`app/crates/freecell-engine/tests/charts_roundtrip_libreoffice.rs`** — a headless
   `soffice`-driven external round-trip, gated on `soffice` being present (skips with a clear
   note when absent, so `cargo test --workspace` on a box without LibreOffice stays green):
   - produce a **FreeCell-saved** line-chart `.xlsx` via the real engine save path
     (`save::save_with_charts` on the committed real Excel line workbook — the byte-preserve
     path the app's Save rides), into a temp dir;
   - run `soffice --headless --convert-to xlsx --outdir <tmp> <freecell_saved.xlsx>` with an
     isolated `-env:UserInstallation` profile (so CI has no stale lock), asserting **exit 0**
     and that the converted file exists;
   - assert the **chart part survives** the external round-trip: the LibreOffice-written
     `.xlsx` still contains a chart part that our own `discover_and_parse` reads back as a
     **line** chart (proves LibreOffice both READ our chart and re-EMITTED it).
   - A helper `soffice_bin()` finds `soffice`/`libreoffice`; the run uses a unique temp profile
     dir; the test is `#[ignore]`-free but self-skips (returns early with an `eprintln!`) when
     no binary is found, matching the render-suite gate idiom.

5. **`.github/workflows/roundtrip.yml`** — a new workflow that runs the LibreOffice round-trip
   in CI (LibreOffice can't share the fast `checks` gate without slowing every push, and the
   pixel gate is a separate slow manual job). `workflow_dispatch` + `pull_request` (paths
   `app/**`) so it runs on the PR and is manually dispatchable; installs `libreoffice-calc`
   (+ the build deps via `setup_render_env.sh`), builds + runs **only** the round-trip test
   with a `FREECELL_LIBREOFFICE=1` opt-in so the test **hard-fails if `soffice` is absent**
   (a required external-round-trip gate must not silently skip — mirrors the render gate's
   `FREECELL_RENDER` policy). Same disk-prune + rust-cache posture as `render.yml`.
   - The test reads `FREECELL_LIBREOFFICE`: set → `soffice` MUST be present (fail if not);
     unset → self-skip when absent. So the workflow enforces the gate; local `cargo test`
     stays green whether or not LibreOffice is installed.

### Deliverable 3 — regression + perf hardening

6. **Perf — extend `app/render-tests/src/bin/chart_perf.rs`** with two new FOREGROUND,
   FORCE+ASSERTED ops (release; p50/p99 env-stamped; written to `results/chart-perf.json`):
   - **many-line-charts (first-paint at scale)** — discover+parse+bind+snapshot a workbook
     bearing **K line charts** (generate via a new `authoring::write_many_line_charts_fixture(path,
     k)`), asserting all K bound with their cached values. Reports the per-open cost for a
     many-chart sheet (the "many-line-charts" perf the plan calls for).
   - **large-series** — build a line `Chart` with **N points** (e.g. N = 100 000) and measure
     the **paint-prep down-sample** (`chart::downsample_for_paint`, new — see step 7) plus the
     model clone the snapshot does; assert the down-sample keeps ≤ a bounded number of vertices
     while the **full series is retained** for save (assert the source `Chart` still has N
     points). Confirms large-series paint stays cheap without touching save fidelity.
   - Keep the existing three ops; adversarially sanity-check (a no-op would trip the asserts).

7. **Large-series down-sample for paint (`architecture.md §5` challenge 5), only if the numbers
   motivate it** — add `freecell-chart-model` `downsample_for_paint(points, max_vertices)` (a
   pure, min/max-preserving bucketed decimation that keeps first/last + per-bucket extremes so
   the line's shape and peaks are preserved) and call it in the line renderer's polyline build
   (`freecell-app::chart`) **for paint only** — the retained `Chart`/source used by save is
   untouched, so save keeps every point (no fidelity/save regression). Gated behind a vertex
   threshold so small series are byte-identical (no baseline move). If the large-series bench
   shows paint is already comfortably under budget at realistic N, record that and **do not**
   add the down-sample (avoid unmotivated complexity) — decide from the measured numbers and
   write the decision into this plan + the summary.

8. **Regression scenes** — audit the committed `chart_line_*` scenes for slice coverage. Add a
   **large-series** perceptual scene (`chart_line_dense`, a many-point line) **iff** it adds
   coverage the existing scenes lack (it exercises the down-sample paint path visibly). If
   added: register it in `chart_scene::all()` + the `chart_render_cases!` table, regenerate its
   baseline with `render_tests.sh generate --only chart_line_dense`, **EYEBALL** the PNG, and
   commit it. Any intentional pixel change to existing chart scenes (only if step 7's down-
   sample is enabled at a threshold that touches a committed scene — it should not) is
   regenerated + eyeballed the same way.

### Render validation (own late sub-phase, per CLAUDE.md)

9. While coding, run only the **relevant subset** FOREGROUND under a `timeout`, ONE blocking
   command, NEVER backgrounded: `render_tests.sh test chart_line` (and `test grid_chart` if the
   in-grid path changed). If step 7's down-sample lands, regenerate + eyeball the affected chart
   baselines. The **full** suite + the CI `render` gate is deliverable 1's job (already the
   dedicated late gate for this phase).

## Tests

- **capture.rs unit** — the failure-message assembler includes the exit code, the failing-
  command context, and each diagnostic section header (render log / xwininfo / display), given
  representative inputs. No display needed.
- **charts_roundtrip_libreoffice.rs** —
  - `libreoffice_reopens_freecell_saved_line_chart` — FreeCell saves the real line workbook;
    `soffice --headless --convert-to xlsx` exits 0 and writes the output; our
    `discover_and_parse` on the LibreOffice output finds a **line** chart (chart part survived
    the external round-trip). Self-skips (or hard-fails under `FREECELL_LIBREOFFICE=1`) when
    `soffice` is absent.
- **chart_perf.rs** (bench asserts, not `#[test]`) — many-line-charts binds all K with cached
  values; large-series retains N points for save while the paint vertex count is bounded.
- **authoring unit** — `write_many_line_charts_fixture` writes K discoverable line charts (a
  small K in a unit test; the bench uses a larger K).
- **downsample unit** (if step 7 lands) — `downsample_for_paint` on a small series is identity
  (≤ threshold), on a large series keeps ≤ max_vertices, always preserves the first + last
  point and the global min/max index (shape/peaks preserved), and never reorders.
- **chart_scene unit** — if `chart_line_dense` is added, the drift guard
  (`chart_scene_names_match_table`) stays green and the new scene is a dense line.

## Render tests

Grid/cell/titlebar pixels are untouched by this phase. Chart scenes only move **if** step 7's
down-sample is enabled at a threshold that a committed scene crosses (it should not — committed
scenes are ≤ a dozen points). The **new** `chart_line_dense` scene (if added) gets a fresh
eyeballed baseline. Subset runs (`render_tests.sh test chart_line`) guard the existing chart
baselines during coding; the CI `render` gate (deliverable 1) is the full-suite truth.

## CI render gate — root cause & outcome (deliverable 1)

The improved diagnostics did their job: they turned the opaque `capture failed (exit Some(1))`
into the real cause. The failure is **intermittent flaky software rendering**, not a
deterministic bug:

- Run on `6f0426e` (diagnostics commit): **full green** — `102 passed; 0 failed` in 471 s (all
  ~88 grid + 15 chart cases rendered + diffed).
- Re-dispatch on the **render-identical** `93041ab`: aborted at ~10 s. The now-visible cause
  (from the added Xvfb `-e` error file + blank-guard message):
  `capture for border_all_thin is blank (1 unique colour(s))` — the **first, coldest** lavapipe
  render presented a **blank frame** within the fixed settle window (the Xvfb error file held
  only benign `xkbcomp` keysym warnings, "not fatal to the X server"). P11's earlier
  `exit Some(1)` was a **different** transient mode of the same flakiness. One flaky case aborts
  the whole *shared* render-all on the first case.

**Fix (gate-preserving, `bf4978b`)** — a retry, not a weakening: `capture_window` now retries
each case's capture up to `RENDER_TESTS_CAPTURE_ATTEMPTS` (default 3; CI 4) fully-independent
`xvfb-run` attempts, so a transient cold-start blank / `xvfb` failure clears on a fresh attempt
(the failed attempt also warms Mesa's on-disk shader cache for the retry). A retry still must
produce a **non-blank frame that diffs against the committed baseline** — nothing is skipped or
softened. `render.yml` also gives the cold first frame more margin
(`RENDER_TESTS_SETTLE_S=6.0`, `PRESENT_S=2.0`) — these change only *when* a frame is captured
(after it has painted), never *what*, so baselines are unaffected. Happy path is unchanged
(succeeds on attempt 1; verified locally). The confirming full CI render run on `bf4978b` is
dispatched; a green run there closes the exit criterion (hand final green confirmation to the
manager if it is still rendering at hand-off — the fix is validated locally and the mechanism is
sound).

## Measured p50/p99

Environment (stamped into `results/chart-perf.json`): x86_64 linux, **release**; headless — no
GPU; all ops are CPU engine/render-path work. Bench: `render-tests/src/bin/chart_perf.rs`
(FORCE+ASSERTED each op — a no-op would trip the asserts). Run FOREGROUND under `timeout`.

| Op | p50 | p99 | max | Reference budget | Notes |
|---|---|---|---|---|---|
| first-paint (discover+parse+bind+snapshot, 1 line chart) | 248 µs | 428 µs | 461 µs | (off critical path) | P11 op; consistent with prior. |
| edit-rerender (dirty-set → reresolve → snapshot) | 1.29 µs | 1.52 µs | 22.7 µs | (off critical path) | P11 op. |
| scroll-with-K (K=1000, per-frame rect+cull scan) | 4.67 µs | 8.90 µs | 111 µs | 8.33 ms / 16.67 ms | P11 op; ~1800× under frame budget. |
| **many-line-charts (open: discover+parse+bind 200 charts on one sheet)** | **7.61 ms** | **8.70 ms** | 8.76 ms | (lazy, off critical path) | ~38 µs/chart; a one-time per-sheet open (P11 lazy discovery), never per-frame. Comfortable. |
| **large-series down-sample (N=100 000 → ≤ 2048 vertices)** | **241 µs** | **272 µs** | 279 µs | 8.33 ms / 16.67 ms | the decimation cost; ~3% of one frame, negligible vs a 100k-primitive paint. |
| **large-series paint-prep, FULL N=100 000 (pre-hardening)** | **177 µs** | **221 µs** | 269 µs | 8.33 ms / 16.67 ms | per-frame point→pixel mapping at full resolution (proxy). |
| **large-series paint-prep, DOWN-SAMPLED to 2048 (post-hardening)** | **4.14 µs** | **11.8 µs** | 23.6 µs | 8.33 ms / 16.67 ms | ~43× less prep; and the gpui side now draws **2048** markers + path vertices, not 100 000 (~49× fewer primitives — the dominant, un-measurable-headlessly win). |

**Down-sample decision — ADOPTED.** The numbers motivate the large-series paint down-sample
(architecture §5 challenge 5). The measurable proxy (per-frame point→pixel prep) drops **43×**
(177 µs → 4.14 µs); the real, larger win is the **gpui primitive count**: a 100 000-point line
would paint 100 000 marker discs + a 100 000-vertex tessellated stroke **per frame** (far over
the 8.33 ms budget), which the decimation bounds to **2048** (~49× fewer) — shape-preserving
(first/last + per-bucket min/max, so peaks survive). The decimation's own 241 µs is a rounding
error against a 100k-primitive paint. Crucially it is **paint-only**: the retained `Chart`/source
keeps all 100 000 points, so **save/live fidelity is untouched** (bench FORCE-ASSERTS the full N
is retained). Gated at `MAX_PAINT_VERTICES = 2048`, far above every committed render scene
(≤ ~12 points) → identity for them → **no baseline moves**.

**Adversarial review.** None is a measured no-op: first-paint asserts the chart bound with the
file's values; edit-rerender asserts the value changed + republished; scroll asserts all K
scanned with only ~2 on-screen; many-line-charts asserts all 200 parsed as line charts with
their cached values; large-series asserts the full 100 000-point series is retained while paint
touches ≤ 2048. The many-line-charts open (7.6 ms/200 charts) is a lazy per-sheet open, never a
per-frame cost, so it needs no further hardening.
