---
status: complete
---

# Phase 13: Hardening & completion sweep

## Overview

The final MVP phase. No new features — a genuine end-to-end completeness pass that
makes "MVP complete" trustworthy. Concretely: prove every `functional_spec.md`
behavior has an automated test or an explicit documented-manual entry (a durable
**coverage matrix**); review the render suite + committed baselines for correctness and
completeness; make every README accurate; finalize `DECISIONS_TO_REVIEW.md` and ensure
every deferred/flagged item has a home; re-audit `cargo-deny`; make a conscious,
recorded **fonts** decision; and execute + record a **manual smoke checklist**, driving
what can be driven under Xvfb + lavapipe.

The invariant throughout: the full workspace suite, the render suite, and the perf gate
must stay green **foreground**.

## Steps

1. **Coverage matrix** (`coverage_matrix.md`, durable). Walk every section of
   `functional_spec.md` (§2–§9) behavior-by-behavior; for each, name the automated
   test(s) that cover it or mark it documented-manual with repro steps. Extracted the
   full test inventory (266 named unit/integration tests + 48 render cases + perf
   gates) to map against. Flag every gap; ensure none is silent.

2. **Known-limitation capture** for the genuine spec deviations the sweep surfaces —
   each gets a home (a `PROJECTS.md` entry + `projects/<name>.md` note per CLAUDE.md's
   "save for later" convention, plus a `DECISIONS_TO_REVIEW.md` finalization):
   - **Type-aware default cell alignment (§3.6)** — the grid defaults all cells to
     left; numbers/dates should default right, booleans/errors center. `PublishedCell`
     carries only a display string, no value type. Phase 6 deferred this to "Phase 11
     engine wiring", which did not land it. Not a new feature to build at the finish
     line (needs a publication-schema change + regenerating ~10 baselines); captured as
     a tracked known limitation. → `projects/type-aware-alignment.md`.
   - **`[Red]` number-format text colour (§3.6)** — `PublishedCell.text_color` is
     published as `None` (Phase 4/7). Already tracked; folded into the same alignment
     project note (both are "publish more per-cell render metadata").
   - **Bundled Inter font** — see step 5. → `projects/bundled-inter-font.md`.

3. **Render suite completeness + eyeballed baselines.** View every committed baseline
   PNG; confirm the suite covers all *implemented* render-relevant behaviors (text
   attrs, fills, engine-formatted numbers/dates/errors, alignment-when-explicit,
   clipping, variable geometry, selection layers, scrollbars, loading overlay, deep
   scroll, busy canary). Record the review + flag anything wrong. No implemented render
   behavior is missing a case (the two deviations above are *unimplemented*, so they
   cannot be baselined — tracked instead).

4. **READMEs.** Create the **root `README.md`** (the repo had none; the app is a full
   spreadsheet, not a hello-world). Fix the stale `app/README.md` "hello-world window
   (Phase 1)" run line. Verify `render-tests/README.md` (baseline process, pinned
   image, tolerance) is accurate.

5. **Fonts decision (Phase 10 → me).** DECIDE: vendor Inter, or defer. Evidence
   gathered: only Inter *variable* fonts are readily fetchable here (not the 4 static
   faces the spec names), which adds font-kit variable-axis resolution risk; the render
   harness (`render_scene`) doesn't register fonts; and the baseline-stability rationale
   the spec cites Inter for is **already delivered for the MVP** by strict runner-image
   + Mesa + font-package pinning (render suite is green + bit-stable on the default
   font). Changing the render font at the finish line means regenerating + re-eyeballing
   all 48 baselines against the load-bearing pixel gate — disproportionate to the
   marginal MVP benefit. **Decision: DEFER**, tracked in `PROJECTS.md` +
   `projects/bundled-inter-font.md` + `DECISIONS_TO_REVIEW.md`. Fix every code/doc claim
   that implies fonts are registered (`shell/fonts.rs`, `main.rs` comment,
   `grid/mod.rs GRID_FONT_FAMILY` doc) so nothing is a false "add_fonts before any
   window opens" claim; keep the default font.

6. **cargo-deny re-audit.** Re-run `cargo deny check`; re-verify each ignored advisory
   (incl. the quick-xml DoS pair via ironcalc/zed's transitive stack) still has no safe
   upgrade at the pinned rev, and the GPL `ztracing` license exception. Document the
   final posture as a security note with a home (`PROJECTS.md` +
   `projects/pre-distribution-security-audit.md`).

7. **DECISIONS_TO_REVIEW.md finalization.** Append the Phase-13 resolutions; add a
   curated "resolution index" so a reviewer can see every deferred/flagged item's
   disposition (resolved | known-limitation-with-home | MVP-scope).

8. **Manual smoke checklist** (`smoke_checklist.md`, durable). Gather the smoke items
   accumulated across phases (Phase 9 controlled-input assumptions, Phase 10 shell
   flows, Phase 11 composed window). Drive under Xvfb + lavapipe what can be driven —
   at minimum: launch the real app, open a fixture `.xlsx` via CLI argv, confirm the
   composed window renders without panic; and exercise the engine-level open→edit→save
   →reopen through the existing round-trip harness. Record observed results; for items
   that are genuinely un-driveable here (native panels, macOS menu bar / traffic-light /
   edited-dot, 100 MB open, held-drag auto-scroll), record documented-manual repro
   steps.

9. **Final green sweep (foreground).** `cargo fmt --check`, `cargo clippy -D warnings`,
   `cargo build --workspace`, `cargo test --workspace`, the render suite
   (`render_tests.sh test`), and the perf gate (`perf.sh` / `perf_harness --gate`) — all
   run synchronously with a timeout; do not finish until real pass/fail is in hand.

## Tests

Phase 13 is a completeness/hardening sweep; the "tests" are the durable coverage
artifacts + the requirement that the whole existing suite stays green, plus:

- **coverage_matrix.md** — every §2–§9 behavior mapped to a named test or a
  documented-manual entry; no silent gaps.
- **smoke_checklist.md** — recorded results (driven-here vs documented-manual).
- The existing suite unchanged and green: freecell-core (96), freecell-engine (78),
  freecell-app (92), render-tests (14 harness tests + 48 render cases), perf gate.
- Code-claim fixes (`fonts.rs`, `main.rs`, `grid/mod.rs`) are doc-only and must keep
  `cargo test --workspace` + clippy green.
</content>
