# Phase 6: Testing strategy & confidence

Reviewer stance: fresh eyes, assessing the test apparatus **as architecture** — what confidence
does it actually buy, and what would it fail to catch. All numbers below were measured statically
against the tree at `e4c7afa`; nothing was executed.

---

## Measured production:test LOC

Method: walk every `.rs` under `app/` (excluding `target/`, `vendor/`). For files under `src/`,
brace-balance each `#[cfg(test)]` block and attribute those lines to "inline test"; everything
else is production. Files under `tests/` are wholly test. Line counts, not statements.

| Crate | Production LOC | Inline test LOC | `tests/` LOC | Test share | Prod : Test |
|---|---:|---:|---:|---:|---:|
| `freecell-app` | 26,445 | 18,229 | 0 | 40.8% | 1 : 0.69 |
| `freecell-engine` | 15,305 | 13,122 | 4,001 | **52.8%** | 1 : 1.12 |
| `freecell-core` | 5,678 | 3,880 | 187 | 41.7% | 1 : 0.72 |
| `freecell-chart-model` | 2,755 | 1,784 | 0 | 39.3% | 1 : 0.65 |
| `render-tests` (harness) | 6,674 | 584 | 644 | 15.5% | n/a — is test infra |
| **TOTAL** | **56,857** | **37,599** | **4,832** | **42.7%** | **1 : 0.75** |

Excluding `render-tests` (whose "production" column is itself test infrastructure): **50,183
production LOC : 41,203 test LOC = 1 : 0.82.**

The headline consequence for the three files flagged as suspiciously enormous: they are **not**
mostly tests, but they are close to half.

| File | Production LOC | Inline test LOC |
|---|---:|---:|
| `app/crates/freecell-app/src/chrome/view.rs` | 8,222 | 7,878 |
| `app/crates/freecell-app/src/grid/view.rs` | 6,489 | 4,139 |
| `app/crates/freecell-engine/src/worker/run.rs` | 3,962 | **5,327** |
| `app/crates/freecell-engine/src/document.rs` | 2,119 | 1,715 |
| `app/crates/freecell-app/src/shell/app.rs` | 752 | 1,641 |

`worker/run.rs` and `shell/app.rs` carry more test than production code. `chrome/view.rs` is a
16k-line file that is genuinely an 8.2k-line production file — the size problem is real, not a
test-inflation artifact (that's Phase 1/4 territory, noted here only to correct the premise).

**Test inventory:** 1,451 test functions parsed — `#[test]` 1,087 + `#[gpui::test]` 373
(`freecell-engine` 487, `freecell-app` 614, `freecell-core` 232, `freecell-chart-model` 93,
`render-tests` 34). ~4,880 `assert!`/`assert_eq!`/`assert_ne!` call sites ≈ **3.4 assertions per
test**. Exactly **3** tests contain no assertion at all (two delegate to a shared
`worker_cache_agrees` oracle; one — `shell/titlebar.rs::titlebar_row_builds_an_element` — is a
genuine did-it-run smoke test). Pixel suite: **165 render cases** (131 grid + 34 chart), 165
committed baselines, 1.9 MB.

---

## What's Good

This is, on the evidence, a **well-above-average test suite** and I want to be specific about why,
because the criticisms below land harder if the baseline is understood.

- **The assertion quality is real, not decorative.** I hunted specifically for assert-not-panics,
  assert-count-nonzero and did-it-run tests. Out of 1,451 tests, three lack an assertion. The
  test *names* are behavioural, not structural — `fill_drag_axis_is_sticky_then_resets_inside_seed`,
  `publish_before_bump_never_shows_a_stale_generation`,
  `drag_armed_before_editor_open_leaves_no_phantom_after_close`,
  `right_click_cell_inside_selection_keeps_it`. These describe product behaviour, which means they
  were written from a spec of behaviour rather than reverse-engineered from an implementation.

- **Negative controls are used systematically — 22 sites.** This is the single rarest good practice
  I found and it is the reason several gates are credible rather than decorative:
  - `freecell-core/tests/dependency_rule.rs::guard_detects_a_forbidden_dependency` proves the
    architectural guard trips on a synthetic violation, so green means "no gpui dep", not "the
    scanner matched nothing".
  - `render-tests/src/bin/perf_harness.rs` sends a real edit (`negative_control_edit`) and requires
    `outcome.negative_control_climbed` — so the zero-engine-calls-on-scroll gate cannot pass by
    measuring a dead counter.
  - `freecell-engine/src/cache.rs::negative_control_skipping_a_mirror_diverges` and
    `worker/run.rs::negative_control_eval_counter_detects_no_coalesce` do the same for the cache
    mirror and coalescing gates.

- **`worker_seam.rs` is a properly-designed integration surface.** It drives only the public
  `DocumentClient`/`Command`/`WorkerEvent` API — no IronCalc type is reachable — which is exactly
  what makes the seam meaningful rather than a re-assertion of internals. Every wait is bounded by
  a 10 s deadline (`wait_for`, `poll_until`) so a stuck worker fails instead of hanging CI. The
  `poll_until` doc comment explicitly reasons about opportunistic event coalescing and chooses to
  poll the converged read model rather than weaken an assertion — that is a test author who
  understood the concurrency, not one who added a sleep until it went green.

- **`dependency_rule.rs` is a real guard, not theatre.** It scans both manifests for forbidden
  dependency prefixes, and — crucially — it tests *its own parser* against the forms that would
  silently defeat a naive implementation: `[dependencies.gpui]` sub-table headers,
  `[target.'cfg(unix)'.dependencies.ironcalc]`, `gpui.workspace = true` dotted keys, and
  `[dev-dependencies]` exemption. Plus the `catch_unwind` negative control. This is how an
  architecture-rule test should be written.

- **The render harness refuses to skip silently.** `render_suite.rs::gate()` returns
  `Gate::Fail` when `FREECELL_RENDER=1` but the capture stack is missing, with the explicit
  rationale "a required gate that can't test any pixels must **fail**, never silently skip to
  green". This designs out the single most common golden-image failure mode (the gate quietly
  becoming a no-op on a runner image change). There is also a drift guard test asserting the
  `render_cases!` macro list stays in sync with `cases::all()`, so a case cannot be added to one
  and forgotten in the other.

- **The perceptual diff has a discriminating-power proof** (`tests/perceptual_diff.rs::
  threshold_is_discriminating`) that asserts pass at exactly `threshold_pixels` and fail at
  `threshold_pixels + 1`. The metric itself is correct and tested. (Whether the *threshold value*
  is right is a separate matter — see Critical 2.)

- **The perf harness forces and asserts its work.** Before measuring it asserts
  `cache.dims().0 == 1_048_576` ("the fixture is 1M rows deep"), asserts column geometry, and
  asserts a minimum bordered-cell count (`BORDERED_VIEWPORT_MIN_CELLS = 500`) so the bordered-frame
  measurement cannot be of an empty viewport. It stamps CPU/OS/rustc/lavapipe into the results JSON.
  This is exactly what `CLAUDE.md`'s benchmark convention demands, and unlike most repos the
  convention is actually honoured.

- **`roundtrip.yml` proves external interop against a genuinely foreign implementation** —
  FreeCell saves, LibreOffice headless reopens and rewrites, FreeCell reparses — with
  `FREECELL_LIBREOFFICE=1` making a missing `soffice` a hard failure rather than a skip. For a
  file-format product this is worth more than a hundred internal round-trips.

- **Coverage by file is near-total.** Only 18 files over 60 lines lack any `#[cfg(test)]` module,
  and all but four of those are themselves test files or trivial glue (`grid/mod.rs`,
  `chrome/mod.rs`, `shell/mod.rs`, `fixtures.rs`). The subsystem-untested map I was asked to
  produce is, at file granularity, nearly empty. That is not the norm.

- **The team knows about `NoopTextSystem` and engineered around it in at least one place.**
  `shell/fonts.rs` documents that `#[gpui::test]` stubs the text system and therefore drives
  `fontdb` directly — the exact crate gpui's Linux backend uses — to prove font-face resolution.
  That is a sophisticated response to a harness limitation. (It is also the *only* place that
  response was applied; see Critical 2.)

---

## Critical (must fix)

### C1. The `render` gate is documented as required but the merge history proves it is not enforced

`render.yml`'s own header states: *"because this is the pixel gate, it MUST be a required status
check. Add it to branch protection under the EXACT context name `render (Xvfb + lavapipe)`."*
`CLAUDE.md` builds an entire process on top of that assumption.

The Actions history says otherwise. Over the repo's life (2026-06-30 → 2026-07-27):

- **29 total `render` runs**, every one `workflow_dispatch`, spanning **13 distinct branches**.
- **44 merged pull requests** on `main`; **46 commits** touching `app/crates/freecell-app/src/grid/`
  or `.../src/chart/` — i.e. squarely in-scope render code.
- **Zero `render` runs on `main`, ever.**

If `render` were genuinely a required status check, every one of those 44 PRs would have been
blocked until dispatched — `render.yml` is `workflow_dispatch`-only and carries no `paths` filter,
so GitHub would demand the context on *all* PRs regardless of what they touched. Thirty-one PRs
merged without it. Branch protection is not enforcing this check.

What remains, therefore, is exactly what the phase brief suspected: **"the agent must remember" is
the entire safety net.** And the gate is not decorative — 2 of the 29 runs failed, a ~7% hit rate.
Extrapolated across the ~30 in-scope merges that skipped it, the expected number of rendering
regressions that merged unexamined is not zero.

The realistic failure mode is not a forgetful agent on a normal day. It is the *diffuse* change:
a shared layout constant, a font fallback, a padding value in `gpui-component`, a dependency bump.
Those don't announce themselves as "grid rendering changes", so the scope test in `CLAUDE.md`
("did I touch grid/cell/sheet or titlebar?") returns *no*, the agent correctly follows the
documented process, and the regression ships anyway. A gate whose trigger condition requires
correctly predicting the blast radius of your own change is not a gate.

Concretely: either make `render` a genuine required check (and accept the manual-dispatch friction),
or make it automatic on a `paths` filter covering `grid/`, `chart/`, `chrome/view.rs`, `assets/`,
and `Cargo.lock`. The current state — a process document asserting an enforcement that does not
exist — is worse than either, because it manufactures confidence nobody has earned.

### C2. The pixel diff tolerance is larger than the content it is supposed to validate

`render-tests/src/diff.rs` sets `per_channel_tolerance: 12` and `fail_fraction: 0.005`. I measured
what 0.5% actually buys on the committed baselines:

| Baseline | Dimensions | Non-background pixels (>12 delta from modal bg) | Allowed differing pixels |
|---|---|---:|---:|
| `cell_valign_middle.png` | 480×160 | 7,337 | **384** |
| `autogrow_wrap_grows.png` | 480×220 | 9,157 | **528** |
| `grid_empty_origin.png` | 640×320 | 14,643 | **1,024** |
| `merge_basic_box.png` | 640×320 | 14,792 | **1,024** |
| `grid_chart_line.png` | 760×420 | 24,262 | **1,596** |

The budget is consistently **5–10% of the scene's entire inked content**, and that inked content is
dominated by gridlines and fills, not glyphs. A separate measurement: the total number of *dark*
(sub-120 luminance) pixels in `cell_align_left_text.png` — essentially the glyph ink — is **74
pixels**, against a 384-pixel budget.

Therefore, concretely, the pixel suite **cannot detect**:

- a wrong character or digit in a cell (~30–90 pixels change);
- a one-or-two-pixel glyph baseline shift;
- a small font-weight or hinting change;
- a subtly wrong text colour (if the per-channel delta stays under 12/255, it is invisible *at any
  scale*, because such pixels never even count as differing);
- a wrong number format rendering `1234.5` as `1,234.50`.

This matters far more than it would in a generic UI app, for one reason: **`#[gpui::test]` installs
a `NoopTextSystem`** — the repo documents this itself in `crates/freecell-app/Cargo.toml` and
`shell/fonts.rs`: *"add_fonts is a no-op, every font resolves to one stub id"*. So all 373 gpui view
tests measure text against a stub. Real font metrics, real shaping, real glyph rasterisation are
exercised **only** under lavapipe in the pixel suite — whose tolerance, as measured, exceeds the
size of the text being validated.

Net: for a spreadsheet, whose single most important rendering property is *"the number in the cell
is the right number, drawn correctly"*, **nothing in the suite validates that at character
granularity.** The engine tests assert `display_text` at the data layer (good, and it partly
covers this), but the path from correct string to correct pixels is unguarded.

The fix is not to abandon the metric — the two-part tolerance/fraction shape is right. It is to
either (a) tighten `fail_fraction` dramatically now that lavapipe has proven stable across 29 runs
(the diff.rs comment already says *"if lavapipe proves bit-exact, tighten rather than loosen"* —
that condition appears to have been met and never acted on), or (b) add per-case options so
text-centric scenes (`cell_*`, `spill_*`, `autogrow_*`) run at a far tighter fraction than
chart/gradient scenes.

### C3. Nothing tests the product's headline claim at scale

The stated goal is *"stupid-fast on huge sheets (Excel-max = 1,048,576 rows × 16,384 cols)"*.
The entire committed fixture corpus is:

| Fixture | Size |
|---|---:|
| `personal_monthly_budget.xlsx` | 35 KB |
| `libreoffice_custom_height_wrap.xlsx` | 27 KB |
| `charts/excel_line_chart_workbook.xlsx` | 16 KB |
| `numbers_table.xlsx` | 7 KB |
| `FONTS.xlsx` | 6 KB |
| `dates.xlsx` | 2 KB |

Six files, largest 35 KB. The 1M×100 perf fixture is **synthesised in-process through the engine**
(`render_tests::perf::build_fixture`), never serialised and reloaded. Consequently there is **zero
coverage** of:

- **the xlsx load path at scale** — parse time, peak memory, shared-string table blowup, a 200 MB
  workbook, a sheet with 500k populated rows. This is the first thing a user does and the first
  thing that will fall over.
- **the save path at scale** — atomic-save temp-file space, serialisation time, memory during write.
- **any memory ceiling assertion anywhere.** Not one test asserts a bound on RSS or allocation.
- **the sparse/dense boundary** — behaviour when a 1M-row sheet has data at row 1 and row 1,048,575.

The perf gate proves the *render* path handles a 1M-row sheet. The product claim is about the whole
pipeline. For a project whose entire differentiator is scale, having zero scale tests on I/O is the
gap most likely to produce a humiliating first-contact-with-a-real-file failure.

---

## Moderate (should fix)

### M1. No property-based testing or fuzzing on the two subsystems that most demand it

There is **no `proptest`, `quickcheck`, `arbitrary`, `loom`, `cargo-fuzz`, `criterion`, or `insta`
anywhere in the workspace** — I checked every `Cargo.toml`. Every one of the 1,451 tests is a
hand-written example.

Two places where that is a genuine architectural gap:

- **The file parser.** Hostile-input coverage is four hand-written cases in
  `worker_seam.rs::spawn_open_bad_files_emit_typed_load_failed` (`empty.xlsx`, plain text, a
  truncated zip header, one more) plus four broken-drawing fixtures in `charts_corpus.rs`. That is
  a reasonable smoke set and a poor security posture. An xlsx is a zip of attacker-controlled XML
  going into a third-party parser (IronCalc) that this project *forked* — precisely the shape that
  fuzzing exists for. A `cargo-fuzz` target over `WorkbookDocument::open` would be a few hours'
  work and is the highest-value single addition available.
- **Formula/reference math.** `freecell-core/src/refs.rs` (612 lines) and `selection.rs`
  (1,203 lines) implement reference parsing, range algebra, and clamping against Excel-max bounds
  — closed algebraic domains with obvious round-trip and invariant properties (`parse(render(r)) ==
  r`; a range's `contains` agrees with its enumeration; clamping is idempotent; union/intersect
  are commutative). These are tested by example only. Property tests would be cheap and would
  find the off-by-one at `col == 16_383` that examples reliably miss.

### M2. Self-oracle tests in the measurement paths

`grid/view.rs::autofit_column_fits_published_cell` computes its expected value by calling the same
production helper the code under test calls:

```rust
let m = measure_incell_text_width(text, CELL_FONT_PX, None, false, false, window);
grid.autofit_column(2, window, cx);
…
assert!((px - autofit_width(measured)).abs() < 0.5, …);
```

This asserts `f(x) == f(x)`. If `measure_incell_text_width` or `autofit_width` is wrong, the test
still passes. Under `NoopTextSystem` both sides receive the same stub metric, so it does not even
verify that a real font was consulted. The *surrounding* assertions (one resize emitted, range is
`(2,2)`, result exceeds the floor) are genuine plumbing checks and worth keeping — but the width
assertion, which is the one that reads like it validates measurement, validates nothing.

The same shape recurs across the autogrow/wrap family (`measure_wrap_height` used to build the
expectation). The fix is a small table of hand-computed expected widths for a pinned font at a
pinned size, run against the real text system — which today means these must live in the pixel
suite, not in `#[gpui::test]`.

### M3. Perf gates are set at 2× and single-shot — they catch catastrophes, not regressions

`render-tests/src/perf.rs`:

```
CI_FRAME_P99_NS      = 11_500_000   // ≈ 2.07× the calibrated 5.56 ms p99
CI_FRAME_MAX_NS      = 13_000_000
CI_CELL_LOAD_P99_NS  =    500_000   // vs a calibrated 93.6 µs → ~5.3×
```

Two problems:

1. **A 2× (and for cell-load, 5×) buffer means a 90% performance regression merges green.** The
   header justifies the buffer as *"for runner-to-runner variance, not because the budget is at
   risk"* — but the effect is the same regardless of the motive: the gate only fires on a
   catastrophe. Given the product is *about* performance, that is the wrong sensitivity.
2. **No trend tracking.** Each run compares against a fixed constant and uploads `perf-runtest.json`
   as a throwaway artifact. Nothing stores or compares across runs. A creep of 1.9 ms → 3 ms → 5 ms
   → 9 ms passes every individual gate and is never noticed. For a gate at 2×, trend tracking isn't
   a nicety — it's the only thing that would make it work.

Separately, the gate explicitly excludes GPU present and gpui layout/shaping as unrepresentative
under lavapipe, deferring them to `macos-verify` — which is **non-required, weekly-cron, and does
not implement any render or perf capture at all** (its final step is an `echo` explaining that the
macOS pixel path is a "documented, unimplemented fallback"). So the real-hardware budgets
(8.33 ms / 2 ms) that the docs call "the product truth" are, in practice, measured by nothing.

Credibility verdict: the harness *methodology* is sound and non-vacuous (forced work, asserted
fixture, negative control — genuinely better than most). The *gate* built on it is too loose to
protect anything short of a 2× cliff. Run history bears out that it is not merely noise: 131 runs,
19 success / 5 failure / 6 cancelled in the last 30 — it runs and it is mostly stable.

### M4. The chart corpus is largely generated by the code under test

`charts_corpus.rs` asserts breadth across chart types — but the corpus is
`freecell_engine::chart::authoring::write_corpus_fixture()`, i.e. **FreeCell writes the fixture and
FreeCell reads it back**. A systematic misunderstanding of the OOXML chart schema would be written
and read consistently and pass every assertion. The same applies to `write_bad_aux_rels_fixture`,
`write_dangling_chart_rel_fixture`, and the other resilience fixtures.

The counterweights are real but thin: **one** genuinely Excel-authored workbook
(`excel_line_chart_workbook.xlsx`, 4 line charts) and the LibreOffice round-trip gate (line charts
only). So for a product whose banner is "Excel-compatible", cross-implementation fidelity is
established for **one chart type from one real file**, and everything else is self-consistency.
Given the ~30 `chart_*` and `retyped_to_*` tests in `worker_seam.rs`, the volume here badly
overstates the fidelity confidence.

### M5. Concurrency coverage is two tests, both about publication ordering

The engine has a worker thread, a shared `RwLock` over the caches, an `arc_swap` publication slot,
a command channel, and a `recv() + try_iter()` coalescing loop. What is tested:

- `publish_before_bump_never_shows_a_stale_generation` — a real spawned reader thread spins on
  `(generation, publication)` while 200 edits fire, counting violations, and asserts the sample
  count is non-zero so the reader isn't dead. **This is a good test.**
- `edit_reflected_after_publish_and_reads_are_wait_free` — 10,000 reads during an in-flight
  recompute, bounded by elapsed time.

What is not tested at all:

- **Cancellation.** `cancel` appears in `worker/protocol.rs` and across five app-side files; no
  test drives a cancel racing an in-flight evaluation.
- **Viewport churn under concurrent edits** — rapid `SetViewport` interleaved with `SetCellInput`,
  the actual scroll-while-typing scenario.
- **`RwLock` contention or lock ordering.** No test acquires the cache lock from a reader while the
  worker holds it for write. No deadlock guard, no lock-ordering assertion.
- **Worker death.** No test kills or panics the worker thread and asserts the UI degrades rather
  than hangs or deadlocks on a channel.
- **Shutdown ordering** beyond the one `drop(client)` comment.

Both existing tests are also wall-clock dependent (`thread::sleep(50ms)`, 2 ms poll intervals),
which under a loaded CI runner is a latent flake source. `loom` on the publication slot would
convert the strongest of these from "we ran it 200 times and saw no violation" to a proof.

### M6. One mass baseline regeneration is not credibly reviewable

Baseline commit hygiene is mostly good: 25 commits touching `render-tests/baselines`, typically
2–7 PNGs each and tied to a named feature phase (`freeze_*`, `merge_*`, `cf_*`, `incell_editor_*`).
That pattern is consistent with targeted, eyeballed regeneration, and it argues *against* the
rubber-stamp hypothesis. Two exceptions:

- **`245a6cd` — "Phase 6: render validation — regenerate baselines for fill handle + new cases"
  touched 107 baseline PNGs in one commit.** No human eyeballs 107 software-rendered screenshots.
  Whatever else changed in that regeneration is now the baseline, permanently.
- Merge commits `fc84ed8` (12 files) and `a6800ed` (18 files) fold baseline updates in during a
  merge, where they receive the least scrutiny.

A mass regeneration is exactly where a golden-image suite launders a regression into a baseline.
It should require its own commit, its own justification, and ideally a before/after contact sheet.

### M7. No coverage instrumentation exists

No `llvm-cov`, `tarpaulin`, or `grcov` anywhere in the workspace or CI. The 42.7% test-LOC figure I
measured is a *proxy* for effort, not a measurement of coverage. Nobody in this project knows which
lines of the 26k-line app crate are actually executed by the 614 tests that target it. Given the
size of `chrome/view.rs` (8.2k production LOC) and `grid/view.rs` (6.5k), the difference between
"broad coverage of the happy path" and "genuine branch coverage" is exactly what nobody can
currently distinguish — and it is a one-line CI addition to find out.

### M8. Trigger-level `paths` filters can leave required contexts permanently pending

`checks.yml` and `perf-gates.yml` both filter on `paths: ['app/**', …]` at the trigger level. The
`checks.yml` header documents the consequence honestly: a PR touching no `app/**` file skips the
whole workflow, the `checks` context is never posted, and under strict branch protection that PR
sits *"Expected — waiting for status"* and cannot merge. The header calls this "the intended
budget trade-off". It is a footgun that will be rediscovered painfully by whoever owns this next;
the `dorny/paths-filter` pattern the comment itself suggests is the correct fix and costs almost
nothing.

---

## Mild (consider fixing)

- **`dependency_rule.rs` checks only direct manifest text.** It scans `Cargo.toml` strings, so it
  catches `gpui` added directly to `freecell-core` but not a *transitive* pull-in via some new
  dependency, and not a `use gpui::…` reaching core through a re-export. A `cargo tree --invert`
  assertion or a source-level grep would close both. The existing test is good; it just guards a
  narrower surface than its doc comment implies.

- **Four permanently-`#[ignore]`d tests with no expiry mechanism.** One in `worker/run.rs`
  (*"blocked on fork CF-undo fix on freecell-fixes (push 403-denied)"*) and three in
  `personal_monthly_budget_fixture.rs` (*"IronCalc does not apply Excel table styles"*). All four
  document *why*, which is better than most repos — but nothing will ever surface them again. They
  will still be ignored in a year. A dated TODO plus a periodic "try un-ignoring" job, or an issue
  link, would keep them alive.

- **No `#[should_panic]` anywhere in the workspace.** Not wrong — `catch_unwind` is used where a
  panic must be proven — but combined with only 28 `.is_ok()` assertions across ~4,880 total, the
  error-path assertion density is noticeably thinner than the happy-path density. Failure modes
  (save-to-full-disk, permission denied, file locked, network path) are represented mostly by the
  four bad-file cases.

- **`thread::sleep(Duration::from_millis(50))` at `worker_seam.rs:561`** is a fixed wall-clock
  settle in an otherwise deadline-disciplined file. Under a contended shared runner this is the
  most likely first flake. The `poll_until` helper already in the file is the right tool.

- **The capture harness's own retry/settle logic is untested.** `render.yml` sets
  `RENDER_TESTS_CAPTURE_ATTEMPTS: 4`, `RENDER_TESTS_SETTLE_S: 6.0` to work around lavapipe
  presenting blank first frames. The retry path — the thing standing between a transient blank frame
  and a false red — has no unit test. `capture.rs` is 515 lines; the `gate()` policy was correctly
  factored out for testability, the retry policy was not.

- **`chrome/edit.rs` (284 lines, the `EditController` state machine) and `chrome/client.rs`
  (268 lines) carry no inline test module.** They are exercised indirectly through `chrome/view.rs`
  tests, which is defensible for a controller that only makes sense in a hosted window — but
  `EditController` owns the two-way editor sync, the `syncing` re-entrancy guard, and autocomplete
  state, which is a real state machine worth testing at its own altitude.

---

## Phase Summary

**Counts:** Critical 3 · Moderate 8 · Mild 6.

This suite is materially better than its size alone would suggest, and better than most codebases
of comparable age. 1,451 tests at 3.4 assertions each, three of which lack an assertion; behavioural
test names; negative controls at 22 sites including on the perf gate and the architecture guard; a
public-API-only worker integration surface with deadline-bounded waits; a render harness that fails
loudly rather than skipping to green. The measured **production : test ratio is 1 : 0.75 across the
workspace (1 : 0.82 excluding the render harness), with `freecell-engine` at 1 : 1.12** — this is a
team that writes tests, and writes them with an unusual amount of thought about what a green result
actually proves. I found essentially none of the pathologies I was sent to hunt for.

The problems are all at the *strategy* altitude rather than the test-writing altitude, and they
cluster into three holes. **First**, the pixel gate's enforcement is fictional: `render.yml` and
`CLAUDE.md` both assert it is a required status check, but 29 dispatched runs across 13 branches
against 44 merged PRs and zero runs on `main` prove branch protection is not enforcing it — the
gate ran on roughly a third of merges and caught real failures on 7% of the runs it did get.
**Second**, and worse, even when it runs it cannot see what it claims to: I measured the 0.5%
`fail_fraction` against the committed baselines and it permits 384–1,656 differing pixels in scenes
whose entire non-background content is 8k–25k pixels, and whose glyph ink is on the order of 70
pixels — so a wrong digit in a cell passes. Because `#[gpui::test]` installs a `NoopTextSystem`,
the pixel suite is the *only* thing that sees real font metrics, which means **nothing in this
project validates that the right number is drawn correctly.** **Third**, the product's entire
premise — Excel-max scale — has no test: six fixtures, largest 35 KB, and the 1M-row perf fixture
is synthesised in-memory so the load and save paths at scale are wholly unexercised.

If I owned this tomorrow I would do three things in this order: make the render gate automatic on a
`paths` filter (C1), tighten `fail_fraction` for text-centric cases now that lavapipe has 29 runs
of demonstrated stability (C2), and commit one large real-world workbook plus a `cargo-fuzz` target
over `WorkbookDocument::open` (C3, M1). None of these is expensive. The foundation underneath them
is good enough that they would pay off immediately.
