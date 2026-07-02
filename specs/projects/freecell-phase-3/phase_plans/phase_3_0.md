---
status: complete
---

# Phase 3.0: Scaffolding

## Overview

Create the `experiments/round-3/` directory holding the four Round-3 investigations as
**independent Cargo projects** (NOT a workspace — matching the Phase-1/2 isolation
pattern where each experiment has its own `Cargo.toml`, `Cargo.lock`, and `target/`).
This phase only produces buildable skeletons + orienting docs; the experiments
themselves are implemented in Phases A–D.

Per architecture §1: **no new frozen harness.** A/B/D depend **read-only, by relative
path** on the frozen `../../round-2/harness` and `../../shared/*` crates and pin
`ironcalc`/`ironcalc_base` to the same `0.7` line (Cargo.lock resolves to **0.7.1**, the
round-2 pin) so numbers stay comparable. A and B additionally pull `ironcalc`/
`ironcalc_base` directly for the `UserModel` API; D reuses the harness `Model` adapter.
**C stays minimal** — no GPUI / GPU deps at scaffolding time (building GPUI in-container
is out of scope and is itself part of C's investigation); C's crate is authored for a
macOS/human run per architecture §5 and overview §7.

Relative-path depth check (each crate sits at `experiments/round-3/<X>-<name>/`):
`../../round-2/harness` and `../../shared/{datagen,bench_util}` — the same `../../`
depth round-2 crates use, because round-3 and round-2 are sibling dirs under
`experiments/`.

## Steps

1. **`experiments/round-3/README.md`** — orient the four investigations (A crux, B
   breadth, C rendering/human-run, D robustness), mirror `round-2/README.md` tone: a
   subtree index table + the independent-Cargo-project / read-only-reuse note.

2. **`A-cache-sync/`** — `Cargo.toml` (name `cache_sync`, edition 2021, `publish =
   false`, MIT/Apache) depending read-only on `round2_harness = { path =
   "../../round-2/harness" }`, `datagen`/`bench_util` at `../../shared/*`, plus
   `ironcalc = "0.7"` + `ironcalc_base = "0.7"` (A will drive `UserModel` directly) and
   `anyhow = "1"`. `src/main.rs` prints a TODO pointing at the investigation. `cargo
   check`.

3. **`B-api-audit/`** — same shape (name `api_audit`); deps mirror A (harness read-only
   + `ironcalc`/`ironcalc_base` 0.7 for the `UserModel`/`Model` public-API probe +
   `anyhow`). `src/main.rs` TODO stub. `cargo check`.

4. **`D-robustness/`** — same shape (name `robustness`); D reuses the harness `Model`
   adapter, so it depends on `round2_harness` read-only + `ironcalc_base = "0.7"` (typed
   errors / direct probes, matching SP1/SP3) + `anyhow`. `src/main.rs` TODO stub. `cargo
   check`.

5. **`C-ci-rendering/`** — MINIMAL. `Cargo.toml` (name `ci_rendering`, edition 2021,
   `publish = false`) with **NO GPUI / GPU deps**; a trivial `src/main.rs` TODO stub that
   compiles in-container. A `README.md` stub stating the crate is authored for a
   macOS/human run (architecture §5, §7) and that GPUI + PNG-encoder + perceptual-diff
   deps are added during Phase C itself. `cargo check` (trivial — no heavy deps).

6. **Verify builds.** Run `cargo check` FOREGROUND with a generous `timeout` for each of
   A/B/C/D (IronCalc is large; first compile can take minutes → `timeout 900 cargo
   check`). If still compiling near the cap, note it. NEVER background.

## Tests

Scaffolding phase — no functional tests. The bar is:

- **A/B/D `cargo check` clean** with IronCalc 0.7.1 resolved (verify each `Cargo.lock`
  pins `ironcalc`/`ironcalc_base` to `0.7.1`).
- **C `cargo check` clean** with no GPUI dep present.
- Each crate contains a trivial placeholder (`main` printing a TODO) that compiles.
- No files created or edited outside `experiments/round-3/`; frozen `round-2/harness`
  and `shared/*` untouched (depended on read-only by relative path only).
