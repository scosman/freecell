---
status: complete
---

# Phase SP3: Function-parity audit (coverage + correctness vs Excel)

## Overview

SP3 answers the headline-but-least-proven promise: **is IronCalc's formula engine
close enough to Excel to credibly ship an Excel-compatible spreadsheet?** Two
measurements, then a verdict:

1. **Coverage diff** — IronCalc's *actually registered* builtins vs a **committed,
   provenance-documented canonical Excel function list**, categorized by real-world
   importance (common vs obscure). Coverage % must be reproducible.
2. **Golden-file correctness harness** — a committed `cases` **data table** (≥ ~100
   cases) of `formula + input cells → expected value OR expected typed error`, with
   known-correct Excel outputs, run through the frozen `round-2/harness` IronCalc
   adapter. Errors compared as **typed errors, not strings**. Reports a pass rate +
   itemized failures.

Then `findings.md` (functional_spec §5.2 headings) carries the coverage matrix, the
golden-file pass rate + failure list, the categorized gap list, and an **honest
verdict** on whether Excel-compat is credibly achievable on IronCalc — or whether the
gaps are fundamental enough to **flag for the engine off-ramp** (functional_spec §4,
SP3 "Judgment / off-ramp").

This is a **measurement** phase (functional_spec §1–2): it does not build product
features. It builds two reproducible measurement tools + a findings doc.

### Discovered API facts (verified against the pinned IronCalc 0.7.1 source)

These shape the design and are documented here so reviewers can check them:

- **345 registered builtins.** `ironcalc_base` 0.7.1 `src/functions/mod.rs` defines
  `pub enum Function` with **345 variants** (confirmed: `Function::into_iter()` returns
  `IntoIter<Function, 345>`; the enum body has exactly 345 variants). The spec's
  "~345" is exact for this pin. Some Excel functions appear *commented out* in the enum
  (e.g. `Forecast`, `Linest`, `Logest`, `ModeMult`, `ModeSingl`, `Percentile*`,
  `Percentrank*`, `Permut`, `Permutationa`, `Prob`, `Quartile*`, `Trend`, `Trimmean`)
  — declared-but-not-registered, so they count as **missing**.
- **`mod functions` is PRIVATE** (`mod functions;`, not `pub`). The `Function` enum and
  its `into_iter()` / `to_localized_name()` are **not reachable** from a downstream
  crate. So the IronCalc-side list cannot be enumerated at runtime via the enum; it is
  extracted **deterministically from the pinned source** (the `impl_function_lookup!`
  macro's `field => Variant` pairs ∩ `language/language.json`'s `en.functions`
  field→ExcelName map). This is fully reproducible against the frozen 0.7.1 crate. A
  runtime probe (evaluate `=FUNC(...)`, check for `#NAME?`) **cross-checks** the static
  list empirically.
- **Errors surface as strings through the public read API.** `CellValue` (returned by
  `get_cell_value_by_index`, and thus the harness's `EngineValue`) has **no Error
  variant** — an error cell comes back as `CellValue::String("#DIV/0!")` etc. The
  adapter maps that to `EngineValue::Text("#DIV/0!")`. To compare **typed** errors
  (spec requirement), SP3 parses both the IronCalc output string and the case's
  expected-error into a `TypedError` enum (12 kinds, mirroring
  `expressions::token::Error` + Excel's `#GETTING_DATA`), then compares enum variants.
  `Model::new_empty(.., "en", .., "en")` fixes the locale so the strings are the
  canonical English error tokens (`#DIV/0!`, `#N/A`, `#VALUE!`, `#REF!`, `#NUM!`,
  `#NAME?`, `#NULL!`, `#SPILL!`, `#CALC!`, `#REF!`, ...).
- **Canonical Excel list source.** Live-fetching Microsoft's page is **blocked in this
  container** (egress policy 403 on support.microsoft.com and every mirror/alt tried;
  GitHub MCP is scoped to `scosman/freecell` only). Per functional_spec SP3 ("you may
  WebFetch it, **or** construct it from a well-known published list and cite exactly
  what you used") and architecture §8 ("'Excel's ~500' needs a committed canonical
  source; document which"), the canonical list is **committed as a data file built from
  the Microsoft 'Excel functions (alphabetical)' catalog (Excel for Microsoft 365)**,
  with provenance (source URL, retrieval method = manual construction from the MS
  catalog, date, and the fact that live fetch was blocked) documented in the file
  header and in `findings.md`. Coverage % is reproducible: re-diff the committed
  canonical CSV against the source-extracted IronCalc CSV.

## Steps

### 1. Scaffold the independent Cargo project

`experiments/round-2/03-function-parity/` — an independent binary+lib crate
`function_parity` (NOT a workspace member; own `Cargo.toml`/`Cargo.lock`).

- `[dependencies]`: `round2_harness = { path = "../harness" }`,
  `ironcalc_base = "0.7"` (same pin, for the typed-error enum + direct probes),
  `serde` + `serde_json`, `csv`, `anyhow`.
- `[dev-dependencies]`: none beyond std (tests live in the lib).
- `.gitignore`: `/target`.
- Add a top-of-folder `README.md` pointing at `findings.md` and `results/`.

### 2. Commit the canonical Excel function list (`data/excel_functions_canonical.csv`)

Columns: `name,category,importance,provenance_note`.
- `name`: uppercase Excel function name.
- `category`: MS category (Math/Trig, Statistical, Text, Logical, Lookup/Reference,
  Date/Time, Financial, Engineering, Information, Database, Cube, Web, Compatibility,
  Dynamic-array).
- `importance`: `common` | `obscure` — hand-tagged by real-world usage (SUM/IF/VLOOKUP
  = common; BESSELK/IMSECH/CUBEKPIMEMBER = obscure). Documented rubric in the file
  header + findings.
- File header (comment lines): **provenance** — "Source: Microsoft 'Excel functions
  (alphabetical)' reference for Excel for Microsoft 365; constructed manually on
  2026-07-01 because live fetch (support.microsoft.com + mirrors) was blocked by the
  container egress policy; ~505 functions." Cite the exact MS URL.

The list targets the full MS catalog (~505 incl. Cube/Web/Compatibility/dynamic-array),
so coverage is measured against the honest superset, not a convenient subset.

### 3. Extract the IronCalc registered list deterministically (`data/ironcalc_functions.csv` + generator)

- `scripts/extract_ironcalc_functions.py` (committed): parses the **pinned 0.7.1
  source** — `~/.cargo/.../ironcalc_base-0.7.1/src/functions/mod.rs`
  (`impl_function_lookup!` macro) ∩ `src/language/language.json` (`en.functions`) — and
  writes `data/ironcalc_functions.csv` (`name`). Deterministic + reproducible; the
  script header documents the exact source paths + version pin. (This is a
  *provenance/reproducibility* tool, run once; the committed CSV is the artifact.)
- Emits exactly 345 distinct names (asserted in the script).

### 4. Coverage-diff library + binary (`src/coverage.rs`, `src/bin/coverage.rs`)

- `load_canonical()` / `load_ironcalc()` read the two CSVs.
- `diff()` → `{ supported: Vec<Fn>, missing: Vec<Fn>, extra_in_ironcalc: Vec<String> }`
  where `Fn` carries name/category/importance. `extra_in_ironcalc` = names IronCalc has
  that aren't in the canonical list (sanity check; expected ~0 — flags typos/aliases).
- `summary()` → overall coverage %, plus **per-category** and **per-importance**
  (common vs obscure) coverage %. The common-function coverage % is the number the
  off-ramp judgment hangs on.
- Binary writes `results/coverage_matrix.csv` (per-function: name, category,
  importance, `supported`(bool)) + `results/coverage_summary.json` (the %s,
  env-stamped) + a human-readable `results/coverage_summary.md`.

### 5. Runtime probe cross-check (`src/probe.rs`)

For each canonical function, build a **minimal valid call** (arity-aware table for the
common ones; a generic `=FUNC()` / `=FUNC(1)` fallback), evaluate through a fresh
`IronCalcEngine`, and classify: `#NAME?` (or `#ERROR!` parse failure) ⇒ **unsupported**;
anything else (value or a *different* typed error) ⇒ **recognized**. This **empirically
confirms** the static list against the real engine (the honest "registered ≠ working"
check, functional_spec §7). Discrepancies (static-says-supported but probe-says-NAME,
or vice-versa) are reported in `results/probe_vs_static.csv` and discussed in findings.
The static source-extracted list remains the authoritative coverage number; the probe
is corroboration.

### 6. Typed-error model (`src/typed_error.rs`)

- `enum TypedError { Ref, Name, Value, Div0, Na, Num, Null, Spill, Calc, Circ, Error,
  GettingData }` (mirrors `ironcalc_base::expressions::token::Error` + Excel's
  `#GETTING_DATA`).
- `TypedError::parse(&str) -> Option<TypedError>` — canonicalizes IronCalc *and* Excel
  spellings (`#DIV/0!`, `#N/A`, `#NAME?`, `#N/IMPL!`→map to the engine's not-impl,
  etc.). This is what makes error comparison **typed, not string**: both expected and
  actual are parsed to the enum and the *enum variants* are compared.

### 7. Golden-file cases as data (`data/golden_cases.csv`)

Columns: `id,category,formula,inputs,expected_kind,expected` where:
- `formula`: the formula in the target cell (e.g. `=A1/A2`), always references cells so
  coercion/empty-cell behavior is exercised through real cell reads.
- `inputs`: `;`-separated `Cell=literal` seeds (e.g. `A1=10;A2=0`); a literal may be a
  number, quoted text, TRUE/FALSE, an ISO date (seeded via a `=DATE(y,m,d)` helper or a
  serial), or empty (cell omitted).
- `expected_kind`: `number` | `text` | `bool` | `error`.
- `expected`: the known-correct Excel result (number: exact or with a documented
  tolerance column; error: the Excel error token).

**≥ ~100 cases** (target ~120) spanning the required edge categories:
- **Error semantics / propagation**: `#DIV/0!` (`=1/0`, `=A1/A2` with A2=0),
  `#N/A` (`=VLOOKUP(...)` miss, `=NA()`, `=MATCH` miss), `#VALUE!` (`="a"+1`,
  `=SQRT("x")`), `#REF!` (`=INDEX(A1:A3,9)`, deleted-ref surrogate via `=OFFSET`
  out of range), `#NUM!` (`=SQRT(-1)`, `=1E308*10`), `#NAME?` (typo'd name),
  `#DIV/0!` **propagation** through `=IFERROR`, `=SUM(errcell, 1)`, `=A1+errcell`.
- **Empty-cell & type coercion**: empty cell as 0 in `+`, empty as "" in `&`,
  `=COUNT`/`=COUNTA` on blanks, `="5"+2`→7 (text-number coercion), `=TRUE+1`→2,
  `=1&2`→"12", `=SUM(TRUE, "3", 4)`, `=N("x")`, `=ISBLANK` on empty vs "".
- **Date serials / locale**: `=DATE(2020,1,1)`→43831, `=DATEVALUE("2020-01-01")`,
  `=A1-A2` day-diff, `=YEAR/MONTH/DAY`, `=EOMONTH`, `=WEEKDAY`, `=1900 leap-year
  bug` serials (Excel's 1900-02-29 quirk: serial 60), `=TODAY()`-type deterministic
  substitutes avoided (use fixed dates).
- **Array / spill**: `=SUM(A1:A3*B1:B3)` (implicit array), `=SEQUENCE`/`=UNIQUE`/
  `=SORT`/`=FILTER` (dynamic-array — likely unsupported → `#NAME?`/`#SPILL!`; the
  point is to *document* the gap as a typed result), `=TRANSPOSE`.
- Plus a broad **common-function correctness** sweep (SUM/AVERAGE/IF/VLOOKUP/INDEX/
  MATCH/LEFT/MID/CONCAT/ROUND/MOD/TEXT/etc.) so the pass rate reflects everyday use,
  not just edge cases.

Cases are **data**, so the suite grows by appending rows (spec: "so the suite grows
cheaply").

### 8. Golden-file harness (`src/golden.rs`, `src/bin/golden.rs`)

- `load_cases()` parses the CSV into `Case { id, category, formula, inputs, expected }`
  where `expected: Expected { Number(f64, tol), Text(String), Bool(bool),
  Error(TypedError) }`.
- `run_case(&Case) -> Outcome`: fresh `IronCalcEngine::new_blank()`, seed `inputs`
  (parse each literal to `EngineValue`; dates via a helper), set the formula in the
  target cell, `recompute()`, read it back, classify the returned `EngineValue`:
  - `Text(s)` where `TypedError::parse(s).is_some()` ⇒ actual is an **error** →
    compare typed.
  - else compare by `expected_kind` (number within tolerance; text exact; bool).
- `Outcome::{Pass, Fail{expected, actual, reason}}`.
- Binary runs all cases, prints pass rate, writes `results/golden_results.csv`
  (`id,category,formula,expected,actual,pass`) + `results/golden_summary.json`
  (total, passed, pass_rate, per-category breakdown, env stamp) +
  `results/golden_failures.md` (itemized failures with expected vs actual).
- **Foreground, `timeout`-wrapped.** No background monitors.

### 9. `findings.md` (functional_spec §5.2 headings)

Headings per Phase-1 §5.2 (Question / Method / Results / Interpretation / Threats /
Recommendation), containing:
- **Coverage matrix**: overall %, per-category %, common-vs-obscure %, the categorized
  gap list (which common functions are missing — the decision-driving set), and the
  commented-out-in-enum note.
- **Golden-file**: total cases, pass rate, per-category pass rate, the itemized failure
  list, and what each failure means (semantic bug vs unsupported function vs expected
  gap like dynamic arrays).
- **Verdict**: an honest judgment on whether Excel-compat is **credibly achievable**
  (missing functions implementable/contributable, semantics mostly right) or whether
  gaps are **fundamental → off-ramp** (functional_spec §4). Explicitly state the
  off-ramp position. Include reproducibility instructions (the one command per tool).
- **Threats to validity**: canonical-list construction (fetch blocked; documented),
  probe arity heuristics, float tolerance choices, the small array/spill sample.
- Commit `results/`.

## Tests

Unit + integration tests in the lib (`cargo test`, foreground):

- **`typed_error::parses_all_ironcalc_tokens`** — every `#...` token IronCalc's
  `Display for Error` can emit parses to the matching `TypedError`; unknown strings →
  `None`.
- **`typed_error::excel_and_ironcalc_aliases_agree`** — `#DIV/0!`, `#N/A`, `#NAME?`,
  `#NUM!`, `#REF!`, `#VALUE!` from either spelling map to one variant.
- **`coverage::diff_partitions_cleanly`** — on a tiny fixture, supported ∪ missing =
  canonical, and counts add up; `extra_in_ironcalc` catches an IronCalc-only name.
- **`coverage::ironcalc_list_has_345`** — the committed `ironcalc_functions.csv` has
  exactly 345 distinct names (guards the source-extraction pin).
- **`coverage::canonical_covers_common_core`** — the canonical list contains a
  hard-coded set of ~30 everyday functions (SUM, IF, VLOOKUP, INDEX, MATCH, TEXT, ...),
  so a truncated/corrupt canonical file fails loudly.
- **`golden::known_cases_pass`** — a curated subset of cases whose IronCalc behavior is
  *verified correct* pass (locks the harness's own correctness; not the same as the
  full pass rate).
- **`golden::typed_error_comparison_is_not_string`** — a case expecting `#DIV/0!`
  passes when IronCalc returns the `#DIV/0!` string, and a *different* error
  (`#VALUE!`) is reported as a Fail with both typed errors named — proving the
  comparison is on parsed variants.
- **`golden::at_least_100_cases`** — `load_cases()` returns ≥ 100 rows (guards the GATE).
- **`golden::every_case_runs`** — every case produces an `Outcome` without panicking
  (no case crashes the adapter).
- **`probe::probe_agrees_with_static_on_core`** — probing SUM/IF/VLOOKUP returns
  recognized; probing a deliberately fake name returns unsupported.

## Reproducibility (one command each)

- Coverage: `cargo run --release --bin coverage` → writes `results/coverage_*`.
- Golden: `timeout 600 cargo run --release --bin golden` → writes `results/golden_*`.
- Probe: `cargo run --release --bin probe` → `results/probe_vs_static.csv`.
- Re-extract IronCalc list: `python3 scripts/extract_ironcalc_functions.py` (against the
  pinned 0.7.1 source).
