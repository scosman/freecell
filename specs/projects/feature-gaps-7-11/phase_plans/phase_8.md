---
status: complete
---

# Phase 8: Render validation + closeout

## Overview

The dedicated LATE render-validation + closeout phase, run after all coding (Phases 1–7)
is committed. Two feature families moved baselined grid pixels: **text spill** (Phase 3)
and **wrap-driven row auto-grow** (Phase 7). Those intentional baselines already landed
in their coding phases (8 `spill_` + 6 `autogrow_` net-new baselines, plus 3
legitimately-changed pre-existing baselines from Phase 3). This phase:

1. Runs the WHOLE pixel suite once against the committed baselines to catch any
   **incidental** regression beyond those intentional changes.
2. Smoke-launches the app under Xvfb to confirm it builds + launches with all the batch's
   chrome features present (find bar, tab drag, quick-edit) and with the Phase-1
   font-warning fix in place.
3. Closes out docs: `GAPS.md` (mark spill/overflow + wrap/auto-grow shipped, note the
   other batch features) and `DECISIONS_TO_REVIEW.md` (resolve/accept each decision, flag
   Phase 9 as still OPEN).
4. Confirms the project checks are still green.

This phase writes **no product code** and (expected) **no baseline changes** — the
intentional baselines are already committed. If the full suite surfaces an unexpected
failure, investigate: a genuine regression STOPS and is reported; a legitimate
prior-phase miss is regenerated + eyeballed + explained.

Authority: `architecture.md §8`, `implementation_plan.md` Phase 8. CI `render` dispatch is
the manager's job (not this phase).

## Steps

1. **Full pixel suite (foreground, ~10-min watchdog).** Pre-warm the render-tests build
   with `-j 2` (guard against the intermittent `ld` bus error under full parallelism —
   environmental), then run `timeout 600 app/render-tests/scripts/render_tests.sh test`
   foreground. EXPECTED: all cases pass. If a case fails unexpectedly, investigate before
   touching any baseline.
2. **Chrome-feature smoke.** `xvfb-run -a cargo run -p freecell-app` — confirm build +
   launch + no panic + no `gpui::svg_renderer` font WARN lines (Phase 1). Headless can't
   drive the chrome features, so this is a build/launch/no-panic/no-warning confirmation.
3. **Closeout `GAPS.md`.** Mark the now-shipped gaps done: the Formatting-expansion **F1**
   row (wrap-text auto-grows row height) and the survey row "Text overflow into empty
   neighbors + wrap", matching GAPS.md's existing format. Note the batch's other shipped
   features (font-warning fix, quick-edit, find/replace, sheet reorder).
4. **Sweep `DECISIONS_TO_REVIEW.md`.** Mark each recorded decision resolved/accepted as
   appropriate; clearly flag that **Phase 9** (Replace All single-undo, the ironcalc fork
   follow-up) is NOT yet done.
5. **Project checks.** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets
   -- -D warnings`, `cargo build --workspace`, `cargo test --workspace` (2
   `charts_roundtrip_libreoffice` failures known-accepted).

## Tests

- No new tests (validation-only phase). The full pixel suite IS the test: every committed
  baseline must match, confirming no incidental regression beyond the intentional
  spill_/autogrow_ changes already landed in Phases 3 & 7.
- Xvfb smoke launch: build + launch + no panic + no font WARN.

## Outcome (2026-07-12)

**1. Full pixel suite — GREEN, all 136 committed baselines match, 0 failures, 0 baseline
changes.** A single full run under lavapipe exceeds the 10-min watchdog (software rendering
is slow — charts and grid+chart combos especially), so the suite was validated across **four
foreground, watchdog'd segments** that together assert every one of the 136 baselines exactly
once (complete, gap-free coverage):
- Segment 1 (`test`, timed out at 10 min but recorded results): 16 harness unit tests +
  `autogrow_` (6) + `border_` (10) + `cell_` (45) + 11 `chart_` — all ok.
- Segment 2 (`test chart_`): all 34 `chart_` baselines — ok (498s).
- Segment 3 (`test grid_chart_`): all 11 slow grid+chart baselines — ok (498s).
- Segment 4 (`test -- grid_ spill_ font_ header_ col_ row_ incell_ text_ titlebar_ --skip
  chart_`): remaining 41 (12 non-chart `grid_`, 8 `spill_`, 3 `font_`, 2 `header_`, + `col_`
  /`row_`/`incell_`/`text_`/`titlebar_`, and re-covered `autogrow_`/`cell_text_*`) — ok (498s).

  Coverage tally = 6+10+45+34+1+3+12+11+2+1+1+8+1+1 = **136** (all baselines). The 3 Phase-3
  legitimately-changed pre-existing baselines (`cell_text_clipped`, `grid_mixed_content`,
  `font_size_24_row_grown`) and all `spill_` (8) + `autogrow_` (6) intentional baselines PASS
  against their committed versions → **no incidental regression, no baseline regeneration
  needed.** Working tree stayed clean (test mode asserts, never writes).

**2. Chrome-feature smoke — PASS.** `xvfb-run -a cargo run -p freecell-app` built, launched,
selected the lavapipe GPU, and idled without panic (timeout-killed). **No `gpui::svg_renderer`
WARN lines** (Phase 1 fix confirmed). The only log WARN (`cosmic_text ... failed to get system
locale`) and the `no xinput mouse pointers` ERROR are headless-Xvfb environmental noise. Chrome
features (find bar, tab drag, quick-edit) are compiled into the build; headless can't drive them.

**3. Closeout docs.** `GAPS.md`: marked F1 (wrap→row auto-grow) resolved, the survey row "Text
overflow into empty neighbors + wrap" resolved (spill + wrap), the survey row "Find (⌘F)/replace"
resolved, and added a `feature-gaps-7-11` batch note (also font-warning fix, quick-edit, sheet
reorder; Replace-All single-undo = open Phase 9). `DECISIONS_TO_REVIEW.md`: added a Phase-8 sweep
banner + per-phase acceptance notes, explicitly flagging the Phase-4 Replace-All decision as
**still OPEN → Phase 9 (not started).**

**4. Project checks — GREEN** (modulo the 2 known-accepted `charts_roundtrip_libreoffice`
failures, which are environmental: soffice headless can't load a source file in this sandbox).
`cargo fmt --all --check` ✅, `cargo clippy --workspace --all-targets -- -D warnings` ✅,
`cargo build --workspace` ✅, `cargo test --workspace --no-fail-fast` = 23 test targets green,
only `charts_roundtrip_libreoffice` (2 tests) failing (environmental).

No product code or baselines changed in this phase (markdown + this plan only). CI `render`
dispatch is the manager's step.
