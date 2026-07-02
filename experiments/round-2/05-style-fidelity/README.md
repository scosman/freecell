# SP5 — Long-tail style-roundtrip fidelity

Extends the frozen `experiments/03-formatting/ironcalc` probe **by copy** into a
comprehensive **build → save `.xlsx` → reload → read-back** fidelity sweep over the long
tail of style attributes (exact fill/font/border colors, **all** border styles, every
number-format family, the full alignment matrix, the font long tail, `quote_prefix`),
classifying each `{survives / lossy / dropped / not-representable}` with **observed vs
expected** evidence. Every matrix row is **probe-backed** — the JSON's `observed` values
are computed by real round-trips in the same code the tests assert on.

Merges + conditional formatting are **OUT of scope** (no IronCalc public API); recorded
as a known **OPEN gap** in `findings.md`, not designed.

- `src/lib.rs` — the round-trip helpers + the `fidelity_matrix()` generator.
- `tests/probe.rs` — the probe-assertions backing every matrix row (15 tests).
- `src/bin/emit.rs` — writes `results/fidelity_matrix.json` + `results/env.txt`.
- `findings.md` — questions, the fidelity matrix, the GATE verdict, severities, OPEN gap.

Independent Cargo project (not a workspace member); depends read-only by relative path on
the frozen `../harness` (IronCalc 0.7.1 pin) and `../../shared/bench_util` (env stamp).

## Reproduce

```sh
cd experiments/round-2/05-style-fidelity
cargo test            # 15 probe-assertions back every matrix entry
cargo run --bin emit  # regenerate results/fidelity_matrix.json + results/env.txt
```

Environment: Rust 1.94.1, 4 cores / ~15 GB RAM, no GPU/display, 2026-07-01,
IronCalc 0.7.1 (pinned, same as the round-2 harness).
