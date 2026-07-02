# SP3 — Function-parity audit (coverage + correctness vs Excel)

Measures how close IronCalc 0.7.1's formula engine is to Excel, in **coverage** and
**correctness**, and judges whether Excel-compat is credibly achievable (or an
engine-off-ramp trigger). See **`findings.md`** for the verdict and **`results/`** for
the committed artifacts.

Independent Cargo project depending read-only by relative path on the frozen
`../harness` (IronCalc adapter, pinned IronCalc 0.7.1) and `../../shared/*`.

## Reproduce (all foreground)

```
# Coverage diff -> results/coverage_matrix.csv, coverage_summary.{json,md}
cargo run --release --bin coverage

# Golden-file correctness -> results/golden_results.csv, golden_summary.json, golden_failures.md
timeout 600 cargo run --release --bin golden

# Runtime probe cross-check -> results/probe_vs_static.csv, probe_summary.json
cargo run --release --bin probe

# Tests
cargo test
```

## Data (committed, reproducible)

- `data/excel_functions_canonical.csv` — canonical Excel function list (~506), from the
  Microsoft catalog; provenance in the file header and in
  `scripts/build_canonical_excel_list.py`.
- `data/ironcalc_functions.csv` — IronCalc 0.7.1's 345 registered builtins, extracted
  from the pinned source by `scripts/extract_ironcalc_functions.py`.
- `data/golden_cases.csv` — 138 golden cases (`formula + inputs -> expected value OR
  typed error`) spanning error propagation, coercion, dates, and array/spill. Generated
  from `scripts/build_golden_cases.py` (edit the structured list there and re-run to
  grow the suite — formulas contain commas, so the CSV must be machine-emitted).

Regenerate any data file: `python3 scripts/extract_ironcalc_functions.py`,
`python3 scripts/build_canonical_excel_list.py`, `python3 scripts/build_golden_cases.py`.
