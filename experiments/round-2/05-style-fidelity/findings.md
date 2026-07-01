# SP5 — Long-tail style-roundtrip fidelity: Findings

> Status: **complete** — Round-2 (FreeCell Phase 2). A check on IronCalc's `.xlsx` file
> I/O, per functional_spec **SP5** and architecture **§5** (SP5 bullet). Extends the
> frozen `experiments/03-formatting/ironcalc` probe **by copy** into a comprehensive
> attribute-fidelity sweep. Runnable: `cargo test` (15 probe-assertions) +
> `cargo run --bin emit` (regenerates `results/`). Environment: Rust 1.94.1, 4 cores /
> ~15 GB RAM, no GPU/display, 2026-07-01, **IronCalc 0.7.1** (pinned, same as the round-2
> harness). Machine-readable matrix: `results/fidelity_matrix.json`; env stamp:
> `results/env.txt`.

## Questions

Beyond the *representative* attributes Phase-1's `03-formatting` proved survive (bold /
italic / size / one fill / one border / one number-format / one alignment), does the
**long tail** of style attributes survive a **build → save `.xlsx` → reload → read-back**
round-trip through IronCalc's native file I/O?

1. **Exact colors** — do precise `#RRGGBB` fill/font/border colors survive? What about
   indexed / theme colors?
2. **All border styles** — thin/medium/thick/double/dotted/dashed variants: which survive?
3. **Every number-format family** — currency, percent, thousands, scientific, date, time,
   date-time, fraction, text, custom conditional-color.
4. **Alignment** — the full horizontal × vertical matrix + wrap (+ indent).
5. **Font long tail** — strike, underline, name, family, size, `quote_prefix`.
6. **Rich text** — mixed formatting runs inside one cell.
7. **Merges + conditional formatting** — explicitly **OUT** (no IronCalc public API);
   recorded as a known OPEN gap, not designed.

Each answer must be **probe-backed** (a passing assertion that reads the attribute back),
classified `{survives / lossy / dropped / not-representable}` with observed vs expected.

## What was done

One independent probe crate (`style_fidelity`) that builds a styled IronCalc `Model` from
committed code, round-trips it through **IronCalc's native `.xlsx` byte path**
(`save_xlsx_to_writer` → `load_from_xlsx_bytes` → `Model::from_workbook` — the same path
03-formatting proved works), and reads each attribute back. Discipline (overview §7):
fixtures generated from committed code; **force + assert** the measured op; every matrix
entry probe-backed, not inferred.

- **`src/lib.rs`** — per-family round-trip helpers (`fill_color_roundtrip`,
  `border_style_roundtrip`, `number_format_roundtrip`, `alignment_roundtrip`, …), each
  returning the `(expected, observed)` pair from a *real* round-trip, and
  `fidelity_matrix()` which assembles all 59 rows by calling those helpers. The matrix's
  `observed` values are therefore **generated from the same code the tests assert on** —
  single source of truth, no hand-typed survivals.
- **`tests/probe.rs`** (15 tests) — one per family, asserting `Survives` rows read back
  exactly and `Lossy`/`Dropped` rows read back the *documented degraded* value (e.g.
  `dotted` → `thin`). Plus a matrix-integrity test (every row's classification agrees with
  a fresh independent round-trip; every row names a real probe), a
  `representative_phase1_attributes_still_survive` anchor (SP5 is a strict extension of
  03-formatting, not a regression), and the merges/CF documented-absence guard.
- **`src/bin/emit.rs`** — writes `results/fidelity_matrix.json` + `results/env.txt`.

The findings below were also cross-checked against the pinned IronCalc 0.7.1 **source**
(`ironcalc::import::styles`, `ironcalc::import::colors`, `ironcalc::export::styles`,
`ironcalc_base::types`) so the *mechanism* of each loss is cited, not guessed
(adversarial-review discipline, overview §7).

Reproduce:

```sh
( cd experiments/round-2/05-style-fidelity && cargo test && cargo run --bin emit )
```

## Results / evidence — the fidelity matrix

**Tally: 59 rows — 50 Survives, 1 Lossy, 2 Dropped, 6 NotRepresentable.** Fidelity axes:
**Survives** = exact value read back; **Lossy** = writable but degraded (observed records
the degraded value); **Dropped** = writable-then-lost, or reference-form discarded while a
derived value is kept; **NotRepresentable** = cannot be expressed through the public API at
all. Each row is backed by the named probe in `tests/probe.rs`.

| Family | Attribute | Expected | Observed | Fidelity | Probe |
|---|---|---|---|---|---|
| colors | fill fg_color (#RRGGBB) | `#FF0000` | `#FF0000` | Survives | fill_colors_survive_roundtrip |
| colors | fill fg_color (#RRGGBB) | `#00ff00` | `#00ff00` | Survives | fill_colors_survive_roundtrip |
| colors | fill fg_color (#RRGGBB) | `#0000FF` | `#0000FF` | Survives | fill_colors_survive_roundtrip |
| colors | fill fg_color (#RRGGBB) | `#000000` | `#000000` | Survives | fill_colors_survive_roundtrip |
| colors | fill fg_color (#RRGGBB) | `#FFFFFF` | `#FFFFFF` | Survives | fill_colors_survive_roundtrip |
| colors | fill fg_color (#RRGGBB) | `#1A2B3C` | `#1A2B3C` | Survives | fill_colors_survive_roundtrip |
| colors | fill bg_color (gray125 pattern) | `#ABCDEF` | `#ABCDEF` | Survives | bg_color_with_pattern_survives |
| colors | font color (#RRGGBB) | `#123456` | `#123456` | Survives | font_colors_survive_roundtrip |
| colors | font color (#RRGGBB) | `#abcdef` | `#abcdef` | Survives | font_colors_survive_roundtrip |
| colors | font color (#RRGGBB) | `#000000` | `#000000` | Survives | font_colors_survive_roundtrip |
| colors | theme / indexed color reference | `theme=n / indexed=n reference` | `resolved #RRGGBB (reference discarded)` | **Dropped** | theme_indexed_colors_flatten_to_rgb |
| borders | border style | `thin` | `thin` | Survives | all_border_styles_roundtrip_classified |
| borders | border style | `medium` | `medium` | Survives | all_border_styles_roundtrip_classified |
| borders | border style | `thick` | `thick` | Survives | all_border_styles_roundtrip_classified |
| borders | border style | `double` | `double` | Survives | all_border_styles_roundtrip_classified |
| borders | border style | `dotted` | `thin` | **Lossy** | all_border_styles_roundtrip_classified |
| borders | border style | `slantdashdot` | `slantdashdot` | Survives | all_border_styles_roundtrip_classified |
| borders | border style | `mediumdashed` | `mediumdashed` | Survives | all_border_styles_roundtrip_classified |
| borders | border style | `mediumdashdotdot` | `mediumdashdotdot` | Survives | all_border_styles_roundtrip_classified |
| borders | border style | `mediumdashdot` | `mediumdashdot` | Survives | all_border_styles_roundtrip_classified |
| borders | hair / dashed / dashDot / dashDotDot | `Excel line styles` | `no BorderStyle variant` | **NotRepresentable** | all_border_styles_roundtrip_classified |
| borders | border color (#RRGGBB) | `#1A2B3C` | `#1A2B3C` | Survives | border_color_and_diagonal_classified |
| borders | diagonal_up / diagonal_down flags | `up=true, down=true` | `up=false, down=false` | **Dropped** | border_color_and_diagonal_classified |
| borders | diagonal line (BorderItem) | `diagonal thin #445566` | `present` | Survives | border_color_and_diagonal_classified |
| number_formats | currency | `$#,##0.00` | `$#,##0.00` | Survives | number_formats_all_families_survive |
| number_formats | percent | `0.00%` | `0.00%` | Survives | number_formats_all_families_survive |
| number_formats | thousands | `#,##0` | `#,##0` | Survives | number_formats_all_families_survive |
| number_formats | scientific | `0.00E+00` | `0.00E+00` | Survives | number_formats_all_families_survive |
| number_formats | date | `yyyy-mm-dd` | `yyyy-mm-dd` | Survives | number_formats_all_families_survive |
| number_formats | time | `hh:mm:ss` | `hh:mm:ss` | Survives | number_formats_all_families_survive |
| number_formats | datetime | `yyyy-mm-dd hh:mm` | `yyyy-mm-dd hh:mm` | Survives | number_formats_all_families_survive |
| number_formats | fraction | `# ?/?` | `# ?/?` | Survives | number_formats_all_families_survive |
| number_formats | text | `@` | `@` | Survives | number_formats_all_families_survive |
| number_formats | custom conditional-color | `[Red]-0.00;[Blue]0.00` | `[Red]-0.00;[Blue]0.00` | Survives | number_formats_all_families_survive |
| alignment | horizontal (all 8) | `general/left/center/right/fill/justify/centerContinuous/distributed` | *same* | Survives ×8 | alignment_full_matrix_survives |
| alignment | vertical (all 5) | `top/center/bottom/justify/distributed` | *same* | Survives ×5 | alignment_full_matrix_survives |
| alignment | wrap_text | `true` | `true` | Survives | alignment_full_matrix_survives |
| alignment | indent | `cell indent level` | `no Alignment.indent field` | **NotRepresentable** | alignment_full_matrix_survives |
| font | strike | `true` | `true` | Survives | font_longtail_survives |
| font | underline (bool) | `true` | `true` | Survives | font_longtail_survives |
| font | name | `Times New Roman` | `Times New Roman` | Survives | font_longtail_survives |
| font | family | `1` | `1` | Survives | font_longtail_survives |
| font | size | `22` | `22` | Survives | font_longtail_survives |
| font | underline: double / accounting | `distinct underline kinds` | `single bool (on/off only)` | **NotRepresentable** | font_longtail_survives |
| font | quote_prefix | `true` | `true` | Survives | quote_prefix_survives |
| rich_text | mixed runs in one cell | `per-run fonts within a cell` | `one Style/font per cell` | **NotRepresentable** | merges_and_cf_absent_from_public_api |
| open_gap | merged cells | `merge ranges` | `no public API` | **NotRepresentable** | merges_and_cf_absent_from_public_api |
| open_gap | conditional formatting | `CF rules` | `no public API` | **NotRepresentable** | merges_and_cf_absent_from_public_api |

*(The horizontal/vertical alignment rows are collapsed above for readability; the JSON
carries all 8 + 5 as individual probe-backed rows. Full detail in
`results/fidelity_matrix.json`.)*

### The common long tail round-trips faithfully (the GATE)

- **Colors — faithful.** Every exact `#RRGGBB` fill, background (under a non-solid
  pattern), font, and border color survives **byte-for-byte, case preserved** (upper and
  lower). *Mechanism:* `Style` models color as an `Option<String>` hex; on export IronCalc
  writes `rgb="FF{RRGGBB}"` (`ironcalc::export::styles::get_color_xml`), on import it
  strips the two leading alpha hex back to `#RRGGBB` (`ironcalc::import::util::get_color`).
  So a well-formed `#RRGGBB` is a fixed point of the round-trip.
- **Standard borders — 8 of 9 faithful.** thin, medium, thick, double, slantdashdot,
  mediumdashed, mediumdashdot, mediumdashdotdot all round-trip exactly. Border **colors**
  round-trip. (The one exception, `dotted`, is the Lossy finding below.)
- **Number formats — every family faithful.** All 10 format-code families (currency,
  percent, thousands, scientific, date, time, date-time, fraction, text, and a custom
  conditional-color `[Red]-0.00;[Blue]0.00`) survive verbatim. *Mechanism:* `num_fmt` is a
  raw format-code string carried through unchanged.
- **Alignment — the full matrix faithful.** All 8 horizontal variants, all 5 vertical
  variants, and `wrap_text` survive.
- **Font long tail — faithful.** strike, underline (on/off), name, family, size, and
  `quote_prefix` all survive.

**→ GATE (judgment) PASS.** The common long tail — colors, standard borders, number
formats, alignment — round-trips faithfully.

### Lossy / dropped / not-representable attributes (documented with severity)

1. **`dotted` border → `thin` — LOSSY. Severity: LOW.** A `Dotted` border is *written*
   correctly to the `.xlsx`, but on **import** IronCalc's border-style parser
   (`ironcalc::import::styles::get_border`) has arms for only **8** of the 9 `BorderStyle`
   enum variants — it has no `Some("dotted")` arm — and falls back
   `Some(_) => BorderStyle::Thin`. So `dotted` reads back as `thin`. Uncommon style; the
   nearest faithful alternatives (`medium`, `mediumdashed`) survive. *(This is an IronCalc
   import bug — a one-line fix upstream — worth a contribution, not a blocker.)*
2. **Theme / indexed color *reference* → resolved RGB — DROPPED. Severity: LOW.** The
   public `Style` has **no** theme/indexed color field, so a theme/indexed *reference*
   cannot be **written** at all. On **import**, IronCalc resolves `theme=`/`indexed=`
   (+`tint`) attributes to a concrete `#RRGGBB` via built-in palette/theme tables
   (`ironcalc::import::colors::{get_themed_color, get_indexed_color}`). Net: the *resolved
   color is kept* (visually identical), the *reference is lost* — so re-saving flattens
   theme colors to literal RGB. Cosmetically invisible; only matters if an app wants to
   preserve "follows the theme" semantics.
3. **`diagonal_up` / `diagonal_down` flags → dropped — DROPPED. Severity: LOW.** *Measured,
   not assumed:* the exporter (`ironcalc::export::styles::get_borders_xml`) carries a
   `TODO: diagonal_up/down?` and does **not** emit the direction flags, so they come back
   `false`. Note the **diagonal border line itself** (the `BorderItem`) *does* survive —
   only the up/down *direction* is lost. Rare attribute.
4. **Excel `hair` / `dashed` / `dashDot` / `dashDotDot` borders — NOT REPRESENTABLE.
   Severity: LOW-MEDIUM.** The `BorderStyle` enum has 9 variants; these four Excel line
   styles have no variant, so they cannot be *expressed* at all. `dashed` is somewhat
   common; the nearest representable style is `mediumdashed`.
5. **Cell `indent` — NOT REPRESENTABLE. Severity: LOW-MEDIUM.** `Alignment` is
   `{horizontal, vertical, wrap_text}` only — no `indent` field — so Excel indent levels
   cannot be expressed.
6. **Double / accounting underline — NOT REPRESENTABLE. Severity: LOW.** `Font.u` is a
   single `bool`; the distinction between single / double / accounting underline collapses
   to on/off.
7. **Rich text (mixed runs in one cell) — NOT REPRESENTABLE. Severity: MEDIUM (for
   import).** IronCalc models one `Style` (hence one font) per cell; cell content is a
   single string / shared string. Mixed-run formatting inside one cell has no API, so a
   rich-text cell loaded from a real-world file collapses to a single style.

### OPEN gap — merges + conditional formatting (out of scope, recorded)

Per functional_spec SP5 / project-overview §2, **merges and conditional formatting are
OUT of scope** and were **not designed** here — only recorded as a known **OPEN** gap:

- **No public merged-cells API** on `Model` in IronCalc 0.7 (the internal
  `Worksheet.merge_cells` field has no getter/setter — a call would not compile; the
  `merges_and_cf_absent_from_public_api` probe documents the absence).
- **No conditional-formatting API** in the public crate interface.

**Scope note (the trap to avoid).** Supporting either would force **FreeCell to take over
`.xlsx` writing** for those features: even if FreeCell kept merges/CF in its own
side-store, IronCalc's native `save_xlsx_to_writer` knows nothing about them and would
emit a file *without* them — so FreeCell would have to either post-process IronCalc's
`.xlsx` output (inject `<mergeCells>` / `<conditionalFormatting>` into the OOXML) or own
the writer entirely. That is a real, separate engineering effort, deliberately **not**
undertaken in Phase 2. This does **not** block the SP5 GATE (which is about attribute
round-trip fidelity of what IronCalc *does* model).

## Conclusion

**IronCalc's `.xlsx` file I/O is high-fidelity for the common long tail.** 50 of 59
probed attribute-values survive a full build → save → reload round-trip *exactly*; the
losses are narrow, well-understood, and low-severity:

- **Faithful (probe-backed):** all exact `#RRGGBB` colors (fill/bg/font/border), 8 of 9
  border styles + border colors + the diagonal line, all 10 number-format families, the
  full 8×5 alignment matrix + wrap, and the font long tail (strike/underline-bool/name/
  family/size/quote_prefix).
- **One genuine LOSSY case:** `dotted` border → `thin` (an IronCalc import parser gap, a
  one-line upstream fix; workaround = use a surviving style).
- **Dropped (reference-only / rare):** theme/indexed color *references* flatten to
  resolved RGB (color kept, reference lost); border `diagonal_up`/`down` direction flags.
- **Not representable (API gaps):** the 4 extra Excel border styles, cell indent,
  double/accounting underline, and rich-text runs — each a `Style`/API limitation, not a
  round-trip corruption.
- **OPEN (out of scope):** merges + conditional formatting have no IronCalc API at all;
  supporting them would force FreeCell to take over `.xlsx` writing — recorded, not
  designed.

**→ SP5 GATE: PASS.** The DELIVERABLE (a probe-backed fidelity matrix, committed to
`results/`) is met, and the judgment GATE (common long tail round-trips faithfully; any
lossy/dropped documented with severity; merges/CF recorded OPEN) holds.

## Recommended design + next-best alternative

- **Native IronCalc styles remain a sound source of truth for the modeled long tail.** For
  colors, standard borders, number formats, alignment, and the common font attributes,
  IronCalc's native `Style` + `.xlsx` I/O is faithful — FreeCell does **not** need a
  side-store for these. This confirms the overview §2 direction (native IronCalc styles)
  for the bulk of formatting.
- **A small FreeCell-side compensation is warranted for the narrow gaps** IF fidelity for
  power-user files matters at build time: (a) map `dotted`→a surviving style (or carry an
  upstream fix), (b) keep indent / rich-text / the 4 extra border styles / double-underline
  in a thin side-store if needed, and (c) merges + CF **must** live in a side-store *and*
  require FreeCell to own or post-process the `.xlsx` write path (the scope trap above).
  None of this is required to *ship*; it is the roadmap for full-fidelity import/export.
- **Next-best alternative** (if any of the not-representable attributes becomes a hard
  requirement): FreeCell takes over `.xlsx` **writing** (post-process IronCalc's output or
  own the writer) so its side-store attributes are emitted. Higher effort; deferred until a
  real requirement forces it.

## Risks / open questions

- **`dotted` import bug** is IronCalc-version-specific (0.7.1); a version bump could fix it
  (re-run this suite to re-confirm) — the `all_border_styles_roundtrip_classified` test
  will flip if it does, forcing a conscious matrix update. Keep the pin; note any forced
  bump as a finding (architecture §8).
- **Theme/indexed flattening** means round-tripping a themed file through IronCalc
  *silently converts* theme colors to literal RGB. Fine for display; a fidelity loss for
  users who expect theme-relative colors to re-theme. Low priority.
- **Rich-text-heavy imports** (mixed runs per cell) collapse to one style — a **medium**
  concern for faithful *import* of real-world formatted spreadsheets; flag for the Stage-3
  decision if rich text is a product requirement.
- **Merges + CF remain OPEN** (overview §2) — no API, and closing them forces the `.xlsx`
  write-path takeover described above. Carried forward, not designed here.
- **Diagonal direction flags** dropped by the exporter TODO; trivial upstream fix if ever
  needed.
