---
status: complete
---

# Scalar Functions Batch (+ TRIM fix)

An engine-coverage batch that closes everyday spreadsheet-function gaps in the calculation
engine. FreeCell's IronCalc fork implements 345 of ~506 common Excel functions today; when
a formula calls one of the missing everyday ones, the cell errors instead of computing.
This batch fills the common scalar (non-array) gaps and fixes one TRIM correctness bug.

No UI, no product design — pure engine coverage/correctness — but it gets the **full spec
treatment** (functional spec + architecture + phased implementation plan) so another agent
can implement it cold, and so each function's exact Excel-compatible contract is written
down and testable. Per the fork policy (`CLAUDE.md`): **one fix = one `fix/<name>` branch =
one clean upstream PR**, all integrated onto `freecell-fixes`; FreeCell picks them up via
the existing pin. The agent prepares each upstream PR (compare link + title + body) for the
owner to open — the owner shepherds the PRs.

## Scope — 11 functions + 1 bug fix

| Function | What it does |
|---|---|
| **SUMPRODUCT** | Sum of element-wise products of arrays (weighted sums, multi-condition counts) — the highest-value item |
| **PROPER** | Title-cases text (`john smith` → `John Smith`) |
| **REPLACE** | Replaces part of a string by position |
| **CHAR** | Number → character (`CHAR(65)` → `A`) |
| **CODE** | Character → number (inverse of CHAR) |
| **CLEAN** | Strips non-printable characters |
| **DOLLAR** | Number → currency text (`"$1,234.50"`) |
| **ADDRESS** | Row/col numbers → reference string (`ADDRESS(1,1)` → `"$A$1"`) |
| **PERCENTILE.INC** | k-th percentile (modern name for PERCENTILE) |
| **QUARTILE.INC** | Quartile (modern name for QUARTILE) |
| **XMATCH** | Modern MATCH — position of a lookup value (returns a scalar) |

**TRIM bug fix:** Excel's TRIM removes leading/trailing spaces **and** collapses internal
runs of multiple spaces to a single space; the fork's TRIM only trims the ends. Fix it to
collapse internal runs.

## Explicitly deferred to v1

- **TRANSPOSE** — returns an array; needs the spill / dynamic-array capability (a v1.0
  project). Can't ship as a standalone scalar function.
- **HYPERLINK (function)** — would compute/display but not be clickable until the v1.0
  clickable-hyperlinks feature lands; deferred to travel with it.

## Source

GAPS.md v0.5 tier row "Missing everyday scalar functions + TRIM bug" (round-2 SP3
findings). Fork process: `specs/projects/ironcalc-upstreaming/implementation_plan.md`
§Operating model.
