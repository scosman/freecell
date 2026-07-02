---
status: complete
---

# Phase SP5: Long-tail style-roundtrip fidelity

## Overview

SP5 is a **check on IronCalc's `.xlsx` file I/O**: beyond the *representative* style
attributes Phase-1's `03-formatting` probed (bold/italic/size/one fill/one border/one
number-format/one alignment), does the **long tail** of style attributes survive a
**build → save `.xlsx` → reload → read-back** round-trip through IronCalc 0.7.1?

The deliverable is a **probe-backed fidelity matrix**: every attribute classified
`{survives / lossy / dropped}` with the **observed vs expected** value, each entry
backed by a *passing assertion* (never inferred). Merges + conditional formatting are
**OUT** (no IronCalc public API) — recorded as a known OPEN gap with the scope note.

This extends `experiments/03-formatting/ironcalc/` **by copy** (that crate is frozen).
It reuses the frozen `../harness` IronCalc adapter's version pin and `../../shared/*`
read-only; the roundtrip helper (`save_xlsx_to_writer` → `load_from_xlsx_bytes` →
`Model::from_workbook`) is the same native path 03-formatting proved works.

### API reconnaissance already done (de-risking probe, scratchpad — informs the matrix)

Read against the pinned `ironcalc_base` 0.7.1 `Style` type + `ironcalc` 0.7.1 xlsx
import/export source, and confirmed with a throwaway round-trip probe:

- **`Style` surface (what is representable at all).** `font.{strike,u,b,i,sz(i32),
  color(Option<hex>),name,family(i32),scheme}`, `fill.{pattern_type(String),fg_color,
  bg_color}`, `border` per-side `BorderItem{style: BorderStyle, color}` + diagonal
  flags, `alignment.{horizontal(8),vertical(5),wrap_text}`, `num_fmt(String)`,
  `quote_prefix(bool)`.
- **Colors are `Option<String>` hex only.** No theme/indexed field on `Style`. On
  **import**, IronCalc **resolves** `indexed=`/`theme=`(+tint) attributes to a concrete
  `#RRGGBB` via built-in palette/theme tables (`import/colors.rs`), and strips the alpha
  of an 8-hex `rgb="FFRRGGBB"` to `#RRGGBB`. On **export** it blindly writes
  `rgb="FF{RRGGBB}"`. So a well-formed `#RRGGBB` round-trips exactly; theme/indexed
  references are **read-then-flattened to RGB** (reference lost, resolved color kept) and
  **cannot be written back as references**.
- **`BorderStyle` enum has 9 variants** (Thin, Medium, Thick, Double, Dotted,
  SlantDashDot, MediumDashed, MediumDashDotDot, MediumDashDot). Excel's `hair`, `dashed`,
  `dashDot`, `dashDotDot` are **not representable** (no enum variant). The import parser
  (`import/styles.rs`) matches only **8** of the 9 — it has **no `"dotted"` arm**, so a
  round-tripped `Dotted` border reads back as **`Thin`** (`Some(_) => BorderStyle::Thin`).
  Confirmed by probe. This is the headline **lossy** finding.
- **No `indent` field** on `Alignment` → Excel indent is **not representable / dropped**.
- **No rich-text API.** Cell content is a single string / shared-string; a cell holds one
  font via its `Style`. Mixed runs in one cell are **not representable** (dropped to the
  cell's single style). Recorded as a representability gap.

## Steps

1. **Scaffold** `experiments/round-2/05-style-fidelity/` as an independent Cargo project
   (NOT a workspace member), mirroring the sibling `02-xlsx-open/` layout:
   - `Cargo.toml` — `name = "style_fidelity"`, edition 2021, `publish = false`,
     `[lib]` + `[[bin]] emit`. Deps (all read-only relative or pinned):
     `round2_harness = { path = "../harness" }` (version-pin provenance / same engine
     seam), `bench_util = { path = "../../shared/bench_util" }` (env stamp),
     `ironcalc = "0.7"`, `ironcalc_base = "0.7"`, `serde` + `serde_json`. (No `datagen`
     dependency needed — SP5's fixture is a hand-built styled `Model`, generated from
     committed code, not synthetic bulk data. Spec allows datagen read-only but does not
     require it.)
   - `.gitignore` → `/target` (and no generated binaries to ignore — the fixture is
     built in-memory and round-tripped; nothing large is written to disk).
   - `results/.gitkeep`.

2. **`src/lib.rs` — the fidelity engine.** Copy the *shape* of 03-formatting's ironcalc
   lib (roundtrip helper, matrix types), then extend to the long tail:
   - `roundtrip_via_xlsx(&Model) -> Model` — verbatim native path (in-memory cursor:
     `save_xlsx_to_writer` → `load_from_xlsx_bytes` → `Model::from_workbook`).
   - A **`Fidelity` enum**: `Survives`, `Lossy`, `Dropped`, `NotRepresentable` — the
     classification axis. (`NotRepresentable` = the attribute cannot even be *set* via the
     public `Style`, distinct from set-but-lost.)
   - A **`FidelityRow`** { `attribute: &str`, `family: &str`, `expected: String`,
     `observed: String`, `fidelity: Fidelity`, `probe: &str` (the test fn that backs it),
     `note: &str` }.
   - A **`FidelityMatrix`** { engine, engine_version, rustc, date, `rows` } — env-stamped,
     serde-serializable to JSON (env via `bench_util::Environment::detect` where useful;
     version/date stamped like 03-formatting for direct comparability).
   - **Probe builders + readers** grouped by family, each returning `(expected, observed)`
     pairs computed by *actually round-tripping* — so the matrix is generated from the
     same code the tests assert on (single source of truth, no hand-typed observed
     values). Families:
     - **Fill colors**: several exact `#RRGGBB` values (incl. lowercase-input,
       black/white edge) via `fill.fg_color` (+ `pattern_type="solid"`); `bg_color` with a
       non-solid pattern (e.g. `"gray125"`).
     - **Font colors**: `font.color` several `#RRGGBB`.
     - **Theme/indexed colors**: build a file whose XML carries a `theme=`/`indexed=`
       color is **not reachable via the write API** (Style has no such field) — so this
       row is documented from the *import-side* source behavior (read-then-flatten) and
       marked accordingly; we probe the **reachable** proxy: that a resolved `#RRGGBB`
       survives, and record theme/indexed *references* as Dropped-to-RGB with the source
       citation. (Honest: we cannot synthesize a theme-colored file purely via the public
       write API, so this row is API-reasoned + partially probed, marked as such — not a
       false "survives".)
     - **Border styles**: **all 9** `BorderStyle` enum variants, each set on a side +
       round-tripped + read back. Expect 8 survive, **`Dotted` → `Thin` (Lossy)**. Also a
       row documenting the 4 Excel styles with **no enum variant** (`NotRepresentable`).
     - **Border colors + diagonal**: a colored border; `diagonal_up`/`diagonal_down` flags
       (probe whether they survive — the exporter has a `TODO: diagonal_up/down?`, so this
       is a real unknown to *measure*, not assume).
     - **Number formats**: one per family — currency `$#,##0.00`, percent `0.00%`,
       thousands `#,##0`, scientific `0.00E+00`, date `yyyy-mm-dd`, time `hh:mm:ss`,
       date-time, fraction `# ?/?`, text `@`, and a **custom conditional-color**
       `[Red]-0.00;[Blue]0.00`. All expected to survive (confirmed by probe).
     - **Alignment**: every `HorizontalAlignment` (8) × representative `VerticalAlignment`
       (5) + `wrap_text` true/false. Probe each survives.
     - **Font attributes long tail**: `strike`, `u` (underline — note: bool only, no
       double/accounting underline → representability note), `name`, `family`, large
       `sz`. Probe survival.
     - **`quote_prefix`**: probe survival.
     - **Rich text (mixed runs)**: `NotRepresentable` — no API; documented with the
       one-font-per-cell citation.
   - `fidelity_matrix() -> FidelityMatrix` assembling every row, each `observed` computed
     by round-tripping (not literals), `probe` naming the backing test.

3. **`src/bin/emit.rs`** — writes `fidelity_matrix()` to
   `results/fidelity_matrix.json` (pretty), mirroring 03-formatting's emit. Also writes a
   small `results/env.txt` stamp (rustc/os/cores/date) for provenance.

4. **`tests/probe.rs`** — the probe-assertions that *back every matrix entry*. One test
   per family, asserting the observed round-tripped value equals the expected for
   `Survives` rows and equals the documented degraded value for `Lossy`/`Dropped` rows
   (e.g. assert `Dotted` reads back as `Thin`; assert theme/indexed unreachable via write
   API by construction). Plus a **matrix-integrity test**: every row's `fidelity` is
   consistent with a re-computed round-trip, and every `Survives`/`Lossy` row names a real
   probe fn. Plus a **guard test** that merges/CF have no public API (copy 03-formatting's
   documented-absence pattern — a call would not compile).

5. **`findings.md`** — functional_spec §5.2 headings (Questions / What was done /
   Results-evidence / Conclusion / Recommended… / Risks-open). Contains the full fidelity
   matrix table (attribute × family × {survives/lossy/dropped/not-representable} ×
   expected/observed × backing probe), the **GATE verdict** (common long tail faithful?),
   the itemized lossy/dropped list **with severity**, and the **merges + CF OPEN gap**
   with the "would force taking over `.xlsx` writing" scope note. Reproduce-command block.

6. **`README.md`** — one-paragraph what-it-is + reproduce command (mirror siblings).

## Tests

- `fill_colors_survive_roundtrip` — several exact `#RRGGBB` fills (incl. lowercase, edge
  black/white) read back identical after round-trip.
- `font_colors_survive_roundtrip` — `font.color` `#RRGGBB` values survive.
- `bg_color_with_pattern_survives` — `bg_color` + non-solid pattern round-trips.
- `all_border_styles_roundtrip_classified` — all 9 `BorderStyle` variants: asserts 8
  survive and **`Dotted` reads back as `Thin`** (the locked lossy fact).
- `border_color_and_diagonal_roundtrip` — colored border survives; measures + asserts the
  actual observed behavior of `diagonal_up/down` + diagonal `BorderItem` (record whatever
  IronCalc really does — survive or drop — and lock the assertion to it).
- `number_formats_all_families_survive` — 10 format-code families each read back exactly.
- `alignment_full_matrix_survives` — every horizontal (8) + vertical (5) + wrap combo
  survives.
- `font_longtail_survives` — strike, underline(bool), name, family, large size survive.
- `quote_prefix_survives` — `quote_prefix` survives.
- `theme_indexed_colors_flatten_to_rgb` — documents (probe-backed where reachable) that a
  resolved `#RRGGBB` survives and that theme/indexed *references* are not writable via the
  public `Style` (compile-time absence of a theme field on `Style`).
- `merges_and_cf_absent_from_public_api` — copied documented-absence guard.
- `matrix_is_probe_consistent` — every matrix row's classification matches a fresh
  round-trip recomputation; every Survives/Lossy row names a real probe.
- `matrix_serializes` — JSON contains the engine, a Lossy entry, and a NotRepresentable
  entry (guards the matrix keeps reporting the honest degraded cases).

## Verification discipline

- Foreground `cargo test` + `cargo run --bin emit` with `timeout` (Phase-2 §3). No
  background runs.
- Every matrix entry is **probe-backed**: `observed` is computed by round-tripping in the
  same code the tests assert on; no inferred/hand-typed survivals.
- Honest about loss: `Dotted→Thin` (Lossy), theme/indexed reference (Dropped-to-RGB),
  indent / rich-text / extra border styles / double-underline (NotRepresentable) are all
  reported with severity, not hidden.
- Isolation: operate only inside `05-style-fidelity/` (+ read-only harness/shared/03).
  Git-scope to this folder. No commit (manager commits).

## GATE self-check (functional_spec SP5)

- DELIVERABLE: fidelity matrix committed, each entry probe-backed → yes (`results/` +
  `tests/probe.rs`).
- Judgment GATE: common long tail (colors, standard borders, number formats, alignment)
  round-trips faithfully → **expected PASS** (probe already confirms colors, 8/9 borders,
  all 10 number-format families, full alignment matrix survive). Lossy/dropped documented
  with severity: `Dotted→Thin` (low — one uncommon style, workaround = medium/dashed),
  theme/indexed→RGB (low — visually identical, reference-only loss), indent / rich text /
  4 extra border styles / double-underline (low-medium — uncommon, representability gaps).
  Merges/CF recorded OPEN with scope note.
