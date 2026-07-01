# SP3 — Function-parity audit: findings

_IronCalc 0.7.1 vs Excel — coverage + correctness. FreeCell Phase 2 (Round-2), engine-risk cohort._

Reproduce everything from this folder (all foreground):

```
cargo run --release --bin coverage    # results/coverage_matrix.csv + coverage_summary.{json,md}
timeout 600 cargo run --release --bin golden   # results/golden_results.csv + golden_summary.json + golden_failures.md
cargo run --release --bin probe       # results/probe_vs_static.csv + probe_summary.json
cargo test                            # 22 unit/integration tests
```

Data files (committed, reproducible): `data/excel_functions_canonical.csv` (canonical
Excel list), `data/ironcalc_functions.csv` (IronCalc registered set),
`data/golden_cases.csv` (138 golden cases). Each has a `scripts/*.py` generator with
documented provenance.

---

## 1. Questions

1. **Coverage.** How much of Excel's function catalog does IronCalc 0.7.1's set of
   *registered* builtins actually cover, and which gaps are common (everyday) vs obscure
   (specialist)?
2. **Correctness.** Where IronCalc *does* implement a function, are the semantics right
   — error kinds and propagation (`#DIV/0!`/`#N/A`/`#VALUE!`/`#REF!`/`#NUM!`/`#NAME?`),
   empty-cell & type coercion, date serials, and array/spill behavior?
3. **Verdict.** Is Excel-compat **credibly achievable** on IronCalc (missing functions
   implementable/contributable, semantics mostly right), or are the gaps fundamental
   enough to **flag for the engine off-ramp** (functional_spec §4)?

---

## 2. What was done (approach + code pointers)

### 2.1 The two lists

- **IronCalc registered set — extracted from the pinned source, not guessed.**
  `ironcalc_base`'s `functions` module is **private** (`mod functions;`), so the
  `Function` enum and its `into_iter()` / `to_localized_name()` are **unreachable from a
  downstream crate** — a real API constraint (documented in the phase plan). Instead,
  `scripts/extract_ironcalc_functions.py` reads the pinned 0.7.1 source directly: it
  intersects the `impl_function_lookup! { field => Variant, … }` macro in
  `src/functions/mod.rs` (the parser's own registration table) with the `en.functions`
  name map in `src/language/language.json`. Result: **exactly 345 distinct registered
  Excel-named functions** — matching the enum's `into_iter()` signature
  `IntoIter<Function, 345>` and the spec's "~345". Committed as `data/ironcalc_functions.csv`.
  Functions that are *commented out* in the enum (FORECAST, LINEST, LOGEST, MODE.MULT,
  MODE.SNGL, PERCENTILE.*, PERCENTRANK.*, PERMUT, PROB, QUARTILE.*, TREND, TRIMMEAN) are
  declared-but-not-registered and correctly counted as **missing**.

- **Canonical Excel list — committed, with documented provenance.** The source is
  Microsoft's "Excel functions (alphabetical)"/"(by category)" catalog for **Excel for
  Microsoft 365**. Live fetching was attempted first and is **blocked in this container**
  (HTTP 403 from support.microsoft.com and every mirror tried; the GitHub MCP is scoped
  to a single private repo — so no external fetch route exists). functional_spec SP3
  explicitly permits the fallback: "you may WebFetch it, **or** construct it from a
  well-known published list and cite exactly what you used." `scripts/build_canonical_excel_list.py`
  therefore holds the full MS catalog as structured data (name, category, common/obscure
  importance) with provenance + the tagging rubric in its header, and emits
  `data/excel_functions_canonical.csv` — **506 distinct functions** (the honest superset:
  math, stat, text, logical, lookup, date/time, financial, engineering, information,
  database, cube, web, compatibility aliases, and dynamic-array). Coverage % is
  reproducible: re-diff the two committed CSVs.

- **Cross-check:** all 345 IronCalc names resolve inside the canonical 506 (0 stray /
  misspelled — see `coverage::committed_ironcalc_names_all_canonical` test), so the diff
  is name-clean.

### 2.2 Coverage diff (`src/coverage.rs`, `src/bin/coverage.rs`)

Partitions the canonical list into supported/missing against the registered set, and
slices coverage overall, per-category, and by importance (common vs obscure). Writes a
per-function matrix CSV + an env-stamped summary JSON + a human-readable MD.

### 2.3 Golden-file correctness harness (`src/golden.rs`, `src/cases_io.rs`, `src/bin/golden.rs`)

- **Cases are data.** `data/golden_cases.csv` — 138 cases, each `formula + input cells →
  expected value OR expected typed error`, with the **known-correct Excel output** as the
  oracle. The suite grows by editing `scripts/build_golden_cases.py` and regenerating
  (formulas contain commas, so the CSV is machine-emitted to stay well-quoted).
- **Run path.** Each case gets a fresh `IronCalcEngine` (the frozen `round2_harness`
  adapter, pinned IronCalc 0.7.1): seed inputs (a `=`-prefixed literal seeds a formula,
  used for dates and precedent errors), set the formula, `recompute()`, read back.
- **Errors compared as typed errors, not strings** (`src/typed_error.rs`). IronCalc's
  public read API has **no error value type** — an error cell returns the *string*
  `"#DIV/0!"`. Both the engine output and the case's expected error are parsed into a
  `TypedError` enum (mirroring `ironcalc_base::expressions::token::Error`) and the
  **variants** are compared. Test `typed_error_comparison_is_not_string` proves a
  `#DIV/0!` string satisfies `Div0` while a different error is a named-both-ways failure.
- **Categories covered:** error semantics/propagation (31), empty-cell & type coercion
  (28), date serials/locale (20), array/spill (8), plus a common-function correctness
  sweep (51).

### 2.4 Runtime probe cross-check (`src/probe.rs`, `src/bin/probe.rs`)

Empirically confirms the static list against the real engine (functional_spec §7:
"registered ≠ working"). For every canonical function it evaluates `=FUNC(1,1,1)` and
classifies by the returned error: verified against IronCalc 0.7.1, an **unknown** name
returns `#NAME?`, while a **known** function with wrong arity returns `#ERROR!` (a
parse/arg error, not `#NAME?`). So `#NAME?` ⇒ unknown, anything else ⇒ recognized.

---

## 3. Results / evidence

Environment: Linux x86_64, 4 cores (container floor), IronCalc 0.7.1 (frozen harness
adapter). Full env stamp in each `results/*.json`. Date: 2026-07-01.

### 3.1 Coverage

**Overall: 345 / 506 = 68.2%** of the canonical Excel catalog is registered in IronCalc 0.7.1.

| importance | supported | total | coverage |
|-----------|-----------|-------|----------|
| **common** | 154 | 189 | **81.5%** |
| obscure | 191 | 317 | 60.3% |

| category | supported / total | coverage | note |
|----------|------------------|----------|------|
| datetime | 25 / 25 | **100%** | full |
| engineering | 54 / 54 | **100%** | full (Bessel, complex, bit, number-systems) |
| database | 12 / 12 | **100%** | full D-functions |
| information | 20 / 21 | 95.2% | only ISOMITTED missing |
| math | 71 / 79 | 89.9% | missing SUMPRODUCT, matrix (MMULT/MINVERSE/MDETERM), AGGREGATE |
| stat | 88 / 111 | 79.3% | modern `.` dists strong; missing FORECAST/percentile/quartile family |
| lookup | 14 / 22 | 63.6% | missing XMATCH, TRANSPOSE, ADDRESS, HYPERLINK, GETPIVOTDATA |
| logical | 11 / 19 | 57.9% | missing LET/LAMBDA + the BYROW/MAP/REDUCE lambda-helpers |
| text | 22 / 43 | 51.2% | missing PROPER, REPLACE, CHAR, CODE, CLEAN, the *B byte variants |
| financial | 28 / 55 | 50.9% | core TVM present; bond/coupon/yield family missing |
| **dynamic-array** | 0 / 17 | **0%** | none: FILTER/SORT/UNIQUE/SEQUENCE/XLOOKUP-spill/VSTACK/… |
| **compatibility** | 0 / 38 | **0%** | pre-2010 aliases (NORMDIST, …); modern `.` forms mostly exist |
| **cube** | 0 / 7 | **0%** | none (needs OLAP/PowerPivot; irrelevant to a desktop clone) |
| **web** | 0 / 3 | **0%** | none (ENCODEURL/FILTERXML/WEBSERVICE) |

**Missing "common" functions: 35** (full list in `results/coverage_summary.md`).
Breaking them down by *why* they're missing changes the picture materially:

- **Dynamic-array (11):** RANDARRAY, FILTER, SORT, SORTBY, UNIQUE, TAKE, DROP, HSTACK,
  VSTACK, CHOOSECOLS, CHOOSEROWS, TEXTSPLIT. This is a whole **capability** IronCalc
  lacks (spilling), not 11 independent bugs — see §4.
- **Pre-2010 compatibility aliases (7):** MODE, PERCENTILE, QUARTILE, RANK, STDEV,
  STDEVP, VAR, VARP. Their **modern `.`-suffixed equivalents are registered** (STDEV.S/.P,
  VAR.S/.P, RANK.EQ; PERCENTILE.INC and QUARTILE.INC are *not*). So most of these
  are aliasing, not true absence — a thin shim (map old name → new impl) closes them.
- **Genuinely-missing everyday scalar functions (~14):** SUMPRODUCT, TRANSPOSE, XMATCH,
  LET, PROPER, REPLACE, CHAR, CODE, CLEAN, DOLLAR, ADDRESS, HYPERLINK, PERCENTILE.INC,
  QUARTILE.INC. **These are the real, user-facing common gaps** — and they are all
  ordinary, independently-implementable scalar functions.

### 3.2 Golden-file correctness — **133 / 138 passed = 96.4%** (GATE: ≥~100 cases — **MET**)

| category | passed / total | rate |
|----------|---------------|------|
| error-propagation | 31 / 31 | **100%** |
| common | 50 / 51 | 98.0% |
| coercion | 27 / 28 | 96.4% |
| dates | 18 / 20 | 90.0% |
| array-spill | 7 / 8 | 87.5% |

**Error semantics are excellent: 31/31.** IronCalc returns the *correct typed error* for
`#DIV/0!` (literal, cell-ref, MOD-by-0, empty divisor), `#N/A` (NA(), VLOOKUP/HLOOKUP/
MATCH miss), `#VALUE!` (text+number, SQRT of text, bad DATEVALUE), `#NUM!` (SQRT(-1),
overflow, FACT(200)), `#NAME?` (typo), `#REF!` (INDEX out of range), and **propagates
errors correctly** through SUM/`+`/`*`, catches them with IFERROR/IFNA, and reports the
right ERROR.TYPE codes. Coercion (empty→0, text↔number, bool arithmetic, `&`
concatenation, N()/ISBLANK/COUNT/COUNTA/COUNTBLANK) is right except one nuance below.

**The 5 failures (all genuine IronCalc-vs-Excel differences; the oracle is Excel):**

| id | formula | Excel | IronCalc | classification |
|----|---------|-------|----------|----------------|
| `coerce-sumproduct-bools` | `=SUMPRODUCT((A1:A3>1)*1)` | 2 | `#NAME?` | **missing common fn** (SUMPRODUCT) |
| `array-transpose-index` | `=INDEX(TRANSPOSE(A1:C1),2)` | 20 | `#NAME?` | **missing common fn** (TRANSPOSE) |
| `text-trim` | `=TRIM("  a  b  ")` | `"a b"` | `"a  b"` | **semantic bug**: TRIM doesn't collapse internal runs |
| `date-serial-1900-01-01` | `=DATE(1900,1,1)` | 1 | 2 | **narrow epoch diff**: Jan–Feb 1900 off-by-one |
| `date-serial-epoch-leapbug` | `=DATE(1900,2,28)` | 59 | 60 | **narrow epoch diff**: same 1900 leap-bug handling |

Notes: (a) the two `#NAME?` failures are the *coverage* gap surfacing as *correctness*
failures — SUMPRODUCT and TRANSPOSE simply aren't registered. (b) The 1900 date
difference is confined to **Jan–Feb 1900**: `DATE(1900,3,1)` onward matches Excel
exactly (61), and modern dates match to the day (`DATE(2020,1,1)`=43831). IronCalc
appears to apply the phantom-1900-leap-day offset to pre-March-1900 dates where Excel
doesn't — a real but essentially zero-impact divergence. (c) TRIM not collapsing internal
whitespace is a small, isolated implementation bug, not a design limit.

During construction one case (`date-edate-forward`) initially failed because **my oracle
was wrong** (I expected 44300; EDATE(2021-01-15,+2)=2021-03-15=44270, which is what
IronCalc correctly returned). Fixed the oracle — a reminder that grading requires a
correct oracle, and that IronCalc's date math is otherwise sound.

### 3.3 Runtime probe — **0 disagreements** with the static list

The probe recognized **345/345** statically-registered functions and returned `#NAME?`
for **all 161** missing ones — perfect agreement (`results/probe_summary.json`,
`probe_vs_static.csv`). This is strong, independent corroboration that (a) the
source-extracted registered list is accurate, and (b) "345 registered" really does mean
"345 recognized by the evaluator", not a stale enum. It directly answers the §7 risk
("a count is not an audit"): here the count and the empirical behavior coincide.

---

## 4. Conclusion — direct answers

- **Coverage:** IronCalc registers **68.2%** of the full Excel catalog and **81.5% of
  common functions**. The four wholly-absent categories (dynamic-array, cube, web,
  compatibility aliases) account for most of the raw gap; two of those (cube, web) are
  irrelevant to a desktop spreadsheet clone, and compatibility is largely aliasing over
  already-implemented modern functions. The everyday scalar core is strong: date/time,
  engineering, database are 100%, math ~90%, information ~95%, logical/lookup/text hold
  the common operations (IF/SUM/VLOOKUP/INDEX/MATCH/LEFT/MID/CONCAT/COUNTIF/…).

- **Correctness:** where IronCalc implements a function, semantics are **mostly right** —
  a **96.4%** golden pass rate with **flawless (31/31) error semantics and propagation**,
  the single hardest and most compat-sensitive area. The failures are two coverage gaps
  surfacing, one small isolated bug (TRIM), and one negligible pre-1900 date edge.

- **The real common gap is small and ordinary.** Stripping out the dynamic-array
  *capability* and the aliasable compatibility names, the genuinely-missing everyday
  functions number roughly **~14 plain scalar functions** (SUMPRODUCT, TRANSPOSE, XMATCH,
  LET, PROPER, REPLACE, CHAR, CODE, CLEAN, DOLLAR, ADDRESS, HYPERLINK, PERCENTILE.INC,
  QUARTILE.INC). Every one is a standard, self-contained function that is
  straightforward to implement and contribute upstream (IronCalc is open source, MIT/
  Apache, Rust). None require engine-architecture changes.

- **The one structural gap: dynamic arrays / spilling.** The 0/17 dynamic-array score
  (FILTER/SORT/UNIQUE/SEQUENCE/spill-range references, plus XLOOKUP's array return and
  the LAMBDA helper family) is **not 17 missing functions — it's a missing engine
  capability** (a formula returning a range that "spills" into neighbouring cells, with
  `#SPILL!` semantics). Adding these is materially harder than adding a scalar function:
  it touches the calc engine's value model and the grid's cell-ownership model. This is
  the one gap that is a *feature-scope decision*, not a quick contribution.

### Verdict: **Excel-compat is credibly achievable on IronCalc — NOT an off-ramp trigger.**

Against the functional_spec §4 off-ramp criterion ("a large fraction of *common*
functions missing, or systematically wrong semantics → flag for the engine off-ramp"),
**neither condition holds**:

- **Common functions are NOT largely missing** — 81.5% are present, and the true
  user-facing scalar gap is ~14 ordinary, independently-implementable functions.
- **Semantics are NOT systematically wrong** — 96.4% golden pass, perfect error-typing
  and propagation, correct coercion and (modern) date math. The failures are localized
  and explainable, not a pervasive semantic mismatch.

IronCalc is a credible Excel-compat base. Recommend **proceed** past the off-ramp
checkpoint on the SP3 axis, with two carry-forward items (below).

---

## 5. Recommended follow-through + risks carried forward

**Recommended (for the real build, not Phase 2):**
1. **Contribute the ~14 missing common scalar functions upstream** (SUMPRODUCT and
   TRANSPOSE first — both showed up as concrete correctness failures and are common in
   real workbooks), plus the thin compatibility-alias shim (old stat names → existing
   `.` impls). Low risk, high compat payoff.
2. **File/fix the TRIM internal-whitespace bug** upstream (one-line semantics fix).

**Risks / open questions carried forward:**
- **Dynamic arrays / spilling (structural).** Decide explicitly whether FreeCell v1
  needs spill semantics. If yes, this is a real engine investment (or an upstream
  collaboration), materially larger than scalar functions — it should be its own scoped
  decision, not assumed. Modern Excel users increasingly rely on FILTER/SORT/UNIQUE, so
  this is the item most likely to affect perceived compatibility. **Flag for Round-3
  scoping**, not for the engine off-ramp.
- **Pre-1900 date edge + TRIM.** Both minor; itemized here so they aren't rediscovered
  as surprises. Neither blocks.
- **Canonical-list construction (threat to validity).** The canonical 506 was
  hand-constructed from the MS catalog because live fetch was blocked; category/
  importance tags are judgment calls (the rubric errs toward `common`, making the common
  bar conservative). The **coverage direction and magnitude are robust** — every IronCalc
  name resolves cleanly and the probe corroborates the registered set — but the exact
  denominator could shift a few functions with a different published snapshot. The list +
  generator + provenance are committed so any reviewer can adjust and re-diff.
- **Version pin.** All numbers are IronCalc 0.7.1 (Phase-1 pin). A version bump changes
  the registered set; re-run the three binaries to refresh — the whole audit is one
  command each.

---

## 6. Deliverables index (committed)

- `data/excel_functions_canonical.csv` — canonical Excel list (506), provenance-documented.
- `data/ironcalc_functions.csv` — IronCalc 0.7.1 registered set (345), source-extracted.
- `data/golden_cases.csv` — 138 golden cases (data; grows cheaply).
- `results/coverage_matrix.csv`, `results/coverage_summary.{json,md}` — coverage matrix.
- `results/golden_results.csv`, `results/golden_summary.json`, `results/golden_failures.md` — golden run.
- `results/probe_vs_static.csv`, `results/probe_summary.json` — empirical cross-check.
- `scripts/*.py` — the three data generators (each with provenance in its header).
