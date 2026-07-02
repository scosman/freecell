# Round-2 experiments (FreeCell Phase 2 — technical de-risking)

Round-2 continues the Phase-1 engine bake-off, now focused on **de-risking the
IronCalc-based direction** ahead of a Stage-3 build decision. Everything here runs
**in-container** (4 cores / ~15 GB RAM, no GPU / no display); those numbers are
authoritative. Phase-1 folders (`experiments/shared/`, `experiments/02-*`,
`experiments/03-*`) are **frozen** — Round-2 reads/copies from them but never edits
them (architecture §1).

## Subtree index

| Path | What it is |
|------|------------|
| [`harness/`](harness/) | **FROZEN shared harness** (created at scaffolding). An independent Cargo **lib** crate: the `SpreadsheetEngine` trait + benchmark scenarios copied verbatim from `02/common`, the IronCalc adapter copied verbatim from `02/ironcalc` (same IronCalc 0.7.x pin), and a fresh-process `peak_rss()` helper. **Read-only downstream** — SP1–SP5 consume it, never edit it; needed changes → escalate. |
| `01-async-interop/` | **SP1** — non-blocking recompute & the engine↔render interop seam (the key experiment). |
| `02-xlsx-open/` | **SP2** — large (≥100 MB) styled `.xlsx` open: fresh-process time, peak RSS, stage breakdown. |
| `03-function-parity/` | **SP3** — function-parity coverage diff + golden-file correctness harness. |
| `04-styled-read/` | **SP4** — styled viewport read at scale + style-API coverage probe. |
| `05-style-fidelity/` | **SP5** — long-tail style round-trip fidelity over an `.xlsx`. |
| `SYNTHESIS.md` | Phase-2 synthesis → the Stage-3 recommendation. |

Each experiment (SP1–SP5) is an **independent Cargo project** (not a workspace member)
that depends by **relative path, read-only** on `../harness` and `../../shared/*`.
`target/` is gitignored repo-wide; repeated IronCalc compiles are accepted (Phase-1
isolation rationale).

The `harness/` crate is the only shared code created in Round-2. It is created and
frozen during **Phase 2.0 (Scaffolding)**; see
`specs/projects/freecell-phase-2/phase_plans/phase_2_0.md`.
