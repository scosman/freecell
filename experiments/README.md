# FreeCell — Phase 1 Experiments

This tree holds all **Phase 1 (technical de-risking)** work: research findings,
runnable Rust benchmarks, a throwaway GPUI proof-of-concept, and recorded results.
Phase 1 answers one question with reproducible evidence: **can we build FreeCell on
Formualizer + GPUI (or a better-ranked alternative)?** See
`specs/projects/freecell-phase-1/` for the functional spec, architecture, and plan.

Phase 1 does **not** build any part of the real FreeCell app. It stops at
validation + a go / go-with-changes / no-go / pivot recommendation.

## Not a Cargo workspace

Each sub-project (and each `shared/` crate) is a **self-contained, independent Cargo
project** — there is **no** root `Cargo.toml`, no root `Cargo.lock`, and no shared
`target/`. This is deliberate (architecture §1): parallel sub-projects must never
contend on shared build files. Build/test each crate from **its own directory**.

## Environment (grounding facts, architecture §0)

- Rust **1.94.1**; container is **4 cores / ~15 GB RAM**, **no GPU, no display**.
- crates.io fetch + build works in-container.
- In-container is authoritative for everything **except the UI** (Sub-project E),
  which targets **macOS/Metal** and is run by the human.

## Layout / sub-project index

| Folder | Sub-project | What it is | Runs |
|--------|-------------|------------|------|
| `shared/datagen/` | — | Deterministic synthetic-sheet + CSV generators (lib crate). **Frozen; read-only.** | in-container |
| `shared/bench_util/` | — | Timing / percentile / gating / results-recording helpers (lib crate). **Frozen; read-only.** | in-container |
| `00-stack-decision/` | A (GATE) | Formualizer smoke test + engine/UI research → ranked stack recommendation. Human sign-off gates the rest. | in-container |
| `01-file-support/` | B | `.xlsx`/CSV load→edit→save round-trip + recommendation. | in-container |
| `02-datamodel-binding-perf/` | C | ≥2 engine↔UI binding designs; scrolling-read, cascade→visible, 1M-cell cascade, write, memory benchmarks. `results/` holds recorded output. | in-container |
| `03-formatting/` | D | Formatting/metadata exposure + storage design. `results/` holds recorded output. | in-container |
| `04-ui-poc/` | E | GPUI PoC: `raw-gpui/` and `gpui-component/` variants over the static datamodel provider; in-app "Run Test" harness; `scripts/` (one-command macOS build/run); `results/` (logged pass/fail). | **macOS/Metal (human runs)** |
| `05-round-2-proposal/` | F | Ranked Round-2 exploration list. | — |
| `SYNTHESIS.md` | G | Roll-up go/no-go + Round-2 pointer feeding the Stage 3 decision. | — |

> Only `shared/` and this README exist as real code/docs after Phase 0
> (scaffolding). Every sub-project folder currently holds a placeholder
> `findings.md` (or `round_2_explorations.md`) stub; later phases fill them in.

## Parallel-editor isolation

After the gate, sub-projects B–F run in parallel on one branch/tree. Every parallel
agent operates **only** inside its own `NN-<name>/` folder (plus read-only use of
`shared/` and `specs/`), never edits the repo root / another sub-project / `shared/`,
and scopes every git operation to its folder (architecture §2.2).

## Shared crates (`shared/`)

Created and **frozen** at scaffolding (Phase 0); later phases consume them by
**relative path** and never edit them. If a phase needs a change here, it escalates.

- **`datagen`** — engine-neutral, deterministic generators: a `SyntheticSheet`
  (`CellSource`) proxy for a big, difficult sheet (varied text/numbers, ~10–20%
  highlighted, scattered bold/italic, variable row/col sizes), `=PREV+1` linear
  chains and wide-fan-out formula shapes, and CSV output. `.xlsx` generation is
  deferred to Sub-project B (the writer choice is gated).
- **`bench_util`** — `Stopwatch`/timing helpers, `LatencyStats` (min/max/mean/
  p50/p99), `GateResult`/`Verdict` PASS/FAIL gating vs §5.4 targets, and a
  serializable `BenchResult` (env-stamped; **date passed in**, no wall-clock in
  recording code) written as JSON to a phase's `results/`.

Depend on them from a sub-project's `Cargo.toml` by relative path. The example
below is for a crate **one level** under `experiments/` (e.g.
`01-file-support/`); **adjust the number of `..` segments to your crate's depth**
under `experiments/`. For a crate two levels down — e.g. Sub-project A's smoke
crate at `experiments/00-stack-decision/smoke/` — use `../../shared/datagen`.

```toml
[dependencies]
# from experiments/01-file-support/ (one level down):
datagen = { path = "../shared/datagen" }
bench_util = { path = "../shared/bench_util" }

# from experiments/00-stack-decision/smoke/ (two levels down):
# datagen    = { path = "../../shared/datagen" }
# bench_util = { path = "../../shared/bench_util" }
```

## How to run everything

There is no top-level build. Run the standard per-crate checks **inside each crate
directory**:

```sh
cargo fmt --all -- --check          # formatting
cargo clippy --all-targets -- -D warnings   # lints (warnings = errors)
cargo build                          # compile
cargo test                           # unit + doc tests
cargo bench                          # benchmarks (crates that add Criterion)
```

For example, to check the shared crates:

```sh
( cd shared/datagen    && cargo fmt --all -- --check && cargo clippy --all-targets -- -D warnings && cargo test )
( cd shared/bench_util && cargo fmt --all -- --check && cargo clippy --all-targets -- -D warnings && cargo test )
```

Engine sub-projects (B–D) record machine-readable results into their `results/`
directory plus a human-readable summary. The UI PoC (E) is built and run on macOS
via `04-ui-poc/scripts/`; its "Run Test" menu item logs measured pass/fail to
`04-ui-poc/results/`.
