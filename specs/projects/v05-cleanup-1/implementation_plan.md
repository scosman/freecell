---
status: complete
---

# Implementation Plan: v05-cleanup-1

One unit per phase. Every phase starts by **confirming the root cause at HEAD by running
something**, and ends with a verdict recorded in
`projects/architecture-review-remediation.md` (see `architecture.md` §0.2). A phase that
closes its unit as **Disproved** is complete and successful.

Details live in [`functional_spec.md`](functional_spec.md) and
[`architecture.md`](architecture.md); this file is the ordered checklist.

## Phases

- [x] **Phase 1 — A1: pin the IronCalc fork by SHA.** `branch` → `rev` on both patch entries,
      lock regenerated with the resolved SHA unchanged, `=0.7.1` comment corrected.
      (`architecture.md` §1)
- [ ] **Phase 2 — A2: `--locked` across CI + `deny.toml` header.** Sweep workflows *and*
      scripts; verify the committed lock is current first. Runs after Phase 1 so the
      precondition is checked against the post-A1 lock. (§2)
- [ ] **Phase 3 — A4: fork inventory, doc corrections, permanent-fork strategy.** Docs only;
      edits the manifest comment Phase 1 rewrote. (§3)
- [ ] **Phase 4 — D1: render gate weekly on `main` + at release.** Triggers, concurrency key,
      plus `render.yml` header, `checks.yml` header, `app/README.md` §CI and `CLAUDE.md`
      render section. Dispatch the workflow on the branch to prove it still runs. (§4)
- [ ] **Phase 5 — C1 (keystone): part-inventory round-trip test.** New
      `tests/part_inventory.rs` over the real Excel fixtures, both production save paths,
      committed drop baselines, `GAPS.md` updated with what it measures. (§5)
- [ ] **Phase 6 — F3a: chart/cell number-format + colour agreement.** Differential corpus in
      `freecell-engine`; fix or honestly-degrade any disagreement inside the faithful subset;
      resolve the `rgb_to_hsl` and Office-palette duplication claims. (§6)
- [ ] **Phase 7 — G1: detect multi-group (combo) charts.** Group counting in
      `source_fidelity`, tightened `is_extended_chart`, corrected `GAPS.md` combo row + G1b
      entry. (§7)
- [ ] **Phase 8 — G5: `dLbls` per-point overrides.** In-place patch of an existing `c:dLbls`
      instead of whole-node replacement; chart-insert collision confirmed and filed, not
      fixed. (§8)

## Constraints that apply to every phase

- Crate-scoped `cargo build -p` / `cargo test -p`; `cargo fmt --all --check` always.
- No full render suite. `chart_` subset only, and only if Phase 6 moves label text.
- Do not touch `app/crates/freecell-app/src/chrome/view.rs` or
  `app/crates/freecell-engine/src/worker/*` — parallel projects own them.
- Commit + push after each phase.
