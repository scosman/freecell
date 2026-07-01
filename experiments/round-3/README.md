# Round-3 experiments (FreeCell Phase 3 — pre-build de-risking)

Round-3 closes the **last architectural unknowns before committing to the real build.**
Phase 2 (Round-2) validated *reading, recomputing, rendering* and returned **BUILD**
(`../round-2/SYNTHESIS.md`); Round-3 validates the parts those didn't touch — the
**interactive editing model** and a few targeted robustness / tooling checks. After this
it is **building, not de-risking.**

**A, B, D run in-container** (4 cores / ~15 GB RAM, no GPU / no display) and their
numbers are authoritative. **C is the one cross-environment piece:** GPUI needs a real
GPU, so C's demonstrable render→PNG→perceptual-diff harness is authored here and **run
by a human on macOS** (its in-container half is the "is headless even possible?"
finding). Phase-1/2 folders (`../shared/`, `../round-2/`) are **frozen** — Round-3
reads/copies from them but never edits them (architecture §1).

## Subtree index

| Path | What it is |
|------|------------|
| [`A-cache-sync/`](A-cache-sync/) | **A (the crux)** — style/geometry cache sync + structural editing. Probes IronCalc's interactive `UserModel` (insert/delete rows/cols, undo/redo, copy/paste, diff-list, `Send`-ness — does the SP1 seam hold?); asserts structural edits shift references + band styles + sizes; builds a resident-cache-shift prototype that **provably agrees with IronCalc**. Locks the cache-sync design + the `Model`-vs-`UserModel` recommendation. |
| [`B-api-audit/`](B-api-audit/) | **B (breadth)** — needed-API audit. Walks a checklist against IronCalc 0.7.1's public API (headline: **display formatting** — who owns number-format rendering; plus diff-list shape, sheet ops, defined names, view state, cell extras, tokenizer) and marks each **present / absent / workaround**, probe- or source-cited, with a plan per gap. |
| [`C-ci-rendering/`](C-ci-rendering/) | **C (rendering-test strategy; human-run)** — confirm a "snapshot the GPUI grid in CI" mechanism end-to-end (render → PNG → perceptual diff within tolerance). In-container: investigate GPUI's offscreen/headless capture (expected to fail with no GPU — the failure mode is the finding). **macOS/human-run:** the demonstrable render→PNG→diff harness. **No GPUI deps are wired at scaffolding** — added during Phase C itself. |
| [`D-robustness/`](D-robustness/) | **D (cheap robustness)** — feed circular refs (`A1=A1`; `A1=B1,B1=A1`) + malformed/pathological formulas; assert IronCalc returns typed errors and does **not** hang (foreground `timeout` catches a hang) or panic; test a worker-panic-recovery path for the SP1-style worker. Reuses the harness `Model` adapter. |
| `SYNTHESIS.md` | Phase-3 "clear to build" verdict → Stage 3 (written last, after the build-readiness checkpoint). |

Each investigation is an **independent Cargo project** (NOT a workspace member) that
depends by **relative path, read-only** on `../../round-2/harness` and `../../shared/*`,
pinning `ironcalc`/`ironcalc_base` to `0.7` (Cargo.lock resolves to **0.7.1**, the
round-2 pin, so numbers stay comparable). A/B additionally drive IronCalc's `UserModel`
directly; D reuses the harness `Model` adapter; C carries no engine at all. `target/` is
gitignored repo-wide; repeated IronCalc compiles are accepted (Phase-1 isolation
rationale). **No new frozen harness** — A/B/D probe the engine in their own crates
(architecture §1).
