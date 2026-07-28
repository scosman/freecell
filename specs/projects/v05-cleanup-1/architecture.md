---
status: complete
---

# Architecture: v05-cleanup-1

Seven independent units. There is no shared runtime component and no new subsystem, so this
document is organised **per unit**: for each, the confirmation experiment (how the hypothesis
is tested by *running* something), the technical design of the fix, and the test plan.

A short cross-cutting section first, because two decisions apply everywhere.

---

## 0. Cross-cutting design decisions

### 0.1 Where the reconnaissance already landed

The reconnaissance below was performed against HEAD while writing this document. It is
recorded here so each phase starts from evidence rather than repeating the search — but each
phase still owns *proving* its claim by running code, not by citing this section.

| Unit | Claim | Reconnaissance result |
|---|---|---|
| A1 | `branch =` pin | **Present.** `app/Cargo.toml` `[patch.crates-io]`: both crates `branch = "freecell-fixes"`. |
| A2 | no `--locked` | **Present.** Only `--locked` in the tree is on `cargo install` of tooling in `release.yml`. `checks.yml`, `macos-verify.yml`, `roundtrip.yml` build/test bare. |
| A4 | manifest comment stale | **Present.** Comment names E2 + E5 only and says "TEMPORARY: revert to a released crates.io pin once the fixes ship". Fork tip in the local cargo checkout is `cee2859d…`, whose log shows *many* merged `fix/*` branches. |
| C1 | closed-loop round-trip | **Present.** `tests/roundtrip.rs` (282 lines) round-trips only `fixtures::*` (FreeCell-authored) via `save_and_reopen`. |
| D1 | dispatch-only render gate | **Present.** `render.yml` is `on: workflow_dispatch:` while its header says it "MUST be a required status check". |
| F3a | numfmt divergence | **Narrower than described.** `chart-model`'s `apply_number_format` is explicitly a *bounded subset* with a `renders_faithfully` predicate that forces `Degraded` for anything outside it. The real testable invariant is agreement **inside** that subset. |
| F3a | `rgb_to_hsl` drift | **Present, but not where the review said.** Copies are in `freecell-app/src/chart/palette.rs:49` (`% 6.0`) and `freecell-chart-model/src/theme.rs:198` (`.rem_euclid(6.0)`) — not `core`. Whether the difference changes an output is unproven. |
| G1 | first group only | **Present.** `load.rs:605` `.find(…)` takes the first chart-group child; `source_fidelity` (`fidelity.rs:175`) never counts groups. |
| G1 | `is_extended_chart` | **Present.** `fidelity.rs:250`: `xml.contains("chartex")` — a bare substring test over the whole part text. |
| G5 | whole-node `dLbls` replace | **Present.** `save.rs:1317-1333` calls `upsert_child(…, &["dLbls"], Some(chrome::dlbls_element(c, l)), …)`; `upsert_child` replaces the existing node's whole byte range. `dlbls_element` builds the element from the model only. |

### 0.2 Verdict recording — the mechanism

Each phase ends by appending to its unit's section in
`projects/architecture-review-remediation.md`:

```markdown
**Outcome (v05-cleanup-1, 2026-07-28):** Confirmed | Confirmed-different | Disproved —
<one line of evidence, citing file:line or a command>. See
`specs/projects/v05-cleanup-1/phase_plans/phase_N.md`.
```

This is additive; the original text stays so the reasoning trail survives. Nothing else in
that document is rewritten except where a unit's *description* is factually wrong (e.g. the
`rgb_to_hsl` file locations), which is corrected inline with a marker.

---

## 1. A1 — Pin the IronCalc fork by SHA

### Confirmation experiment

1. `grep -n "freecell-fixes" app/Cargo.toml app/Cargo.lock` — show the branch pin and how the
   lock records it.
2. Resolve the branch to a SHA from the **remote**, not the local cargo checkout:
   `git ls-remote https://github.com/scosman/ironcalc refs/heads/freecell-fixes`. The local
   `~/.cargo/git/checkouts/ironcalc-*` checkout is a cache and can lag; it is a
   cross-check, not the source.
3. Confirm the risk is real rather than theoretical: show that other git deps in the tree are
   `rev`-pinned (`gpui`, `gpui_platform`, `gpui-component`, `gpui-component-assets` all are),
   so the fork is the sole outlier.

### Design

Edit `app/Cargo.toml`:

```toml
[patch.crates-io]
ironcalc     = { git = "https://github.com/scosman/ironcalc", rev = "<40-hex>" }
ironcalc_base = { git = "https://github.com/scosman/ironcalc", rev = "<40-hex>" }
```

**The rev must be the SHA `freecell-fixes` currently points at**, so the resolved code is
byte-identical to what HEAD already builds. This is the load-bearing constraint of the unit:
A1 changes *how* the dependency is addressed, never *what* it resolves to.

**Verifying no drift.** `Cargo.lock` records git sources as
`git+<url>?branch=freecell-fixes#<sha>` today and will become `git+<url>?rev=<sha>#<sha>`.
The **fragment SHA must be unchanged** across the edit. That is the check: diff `Cargo.lock`
and assert the only changes are the `?branch=` → `?rev=` query strings, with every `#<sha>`
identical. If any other lock line moves, the unit stops and investigates.

**The `= "0.7.1"` version lines.** `[patch]` requires the patched crate to appear in the
dependency graph under a version requirement — `[workspace.dependencies] ironcalc = "=0.7.1"`
is what the patch attaches to. Removing it would break the patch, not tidy it. The
architecture's decision: **keep the lines, fix the comment.** The comment must say that under
the active `[patch.crates-io]` these version requirements are never satisfied from crates.io
— they exist only to give the patch a target — so nobody reads `=0.7.1` as "we ship IronCalc
0.7.1". Whether the fork's manifest still declares `0.7.1` is checked (a `[patch]` whose
replacement version does not satisfy the requirement fails to apply, so this is verifiable by
the build succeeding).

The rewritten comment also carries the maintenance rule: bumping the fork is a one-line rev
edit plus `cargo update -p ironcalc -p ironcalc_base`.

### Tests

No unit test is meaningful. Verification is:
- `cargo metadata --locked --format-version 1` resolves and shows the rev source;
- `cargo build --locked -p freecell-engine` succeeds (proves the patch still applies and the
  lock is consistent);
- the `Cargo.lock` diff shows only the source-string change.

### Risks

`cargo` will refuse `--locked` if the source string change is not reflected in the lock, so
the lock must be regenerated in the same commit. Regenerating with plain `cargo update` would
also drift unrelated crates — use `cargo build` (which minimally updates the lock) or
`cargo update -p ironcalc -p ironcalc_base --precise <sha>` semantics, then diff.

---

## 2. A2 — `--locked` in CI; `deny.toml` header

### Confirmation experiment

1. Enumerate every cargo invocation across `.github/workflows/*.yml` **and** every script
   those workflows call (`app/scripts/*`, `app/render-tests/scripts/*`), so the sweep is not
   just the YAML surface.
2. Prove the hole is real, not theoretical: at HEAD, edit a dependency version in a scratch
   copy of `Cargo.toml` without touching the lock, and show `cargo build` succeeds (silently
   rewriting the lock) while `cargo build --locked` fails. That is the exact PR scenario.

### Design

**Scope rule.** `--locked` goes on every cargo subcommand that *resolves this workspace's
dependency graph*: `build`, `test`, `clippy`, `run`, `metadata`, `deny`. It does **not** go on
`cargo fmt` (no resolution) and it stays as-is on `cargo install` (that flag already means the
installed tool's own lock).

Files touched, per the enumeration: `.github/workflows/checks.yml`,
`macos-verify.yml`, `render.yml`, `roundtrip.yml`, `perf-gates.yml`, `release.yml`, plus any
in-repo script that invokes cargo against this workspace.

**Precondition that must be checked first.** `--locked` makes CI fail if `Cargo.lock` is stale
at HEAD. So the phase runs `cargo build --locked --workspace` (or at minimum
`cargo metadata --locked`) locally *before* landing. If the lock is stale, it is regenerated
and committed in the same change — otherwise this unit turns `main` red.

Interaction with A1: A1 rewrites the lock's git source strings. **A2 runs after A1**, so the
`--locked` precondition is checked against the post-A1 lock.

**`deny.toml` header.** Read the current `[licenses]` block, confirm `exceptions = []`, and
rewrite the header comment to describe the real mechanism (GPL crates removed from the graph
by the `vendor/` no-op stubs + `[patch]`, not by a license exception).

### Tests

CI-config change; no unit test. Verification: local `cargo build --locked --workspace`
succeeds; `cargo deny check --locked` succeeds; YAML parses (`python3 -c "import
yaml,sys;[yaml.safe_load(open(f)) for f in sys.argv[1:]]"` over the workflow files).

---

## 3. A4 — Fork docs and the real strategy

Docs only. No manifest *semantics* change (A1 owns the pin); A4 owns the *comment*, so A4
runs after A1 and edits the comment A1 rewrote.

### Establishing the true state

The fork is a separate repo. Two evidence sources, in preference order:

1. **The fork itself.** A local cargo checkout already exists at
   `~/.cargo/git/checkouts/ironcalc-*/` — a real git repo with the fork's history. It gives
   the merge commits on `freecell-fixes` (each `fix/<slug>` merge is a titled merge commit),
   which is exactly the branch inventory. If a fuller clone is needed the container's git
   proxy routes `scosman/ironcalc` (per `CLAUDE.md` §Engine).
2. **Upstream status.** Determining merged-upstream status requires comparing against
   `ironcalc/IronCalc` `main`. Where that is reachable, use
   `git merge-base --is-ancestor <fix-sha> upstream/main` per fix. Where it is not, the
   status table records the fix with status **`unknown (not verifiable from this container)`**
   rather than a guess.

The overview states as fact that fixes *have* been merged upstream. The inventory must
therefore not reproduce the review's "nothing upstreamed" claim; where per-fix status cannot
be verified, it says so explicitly.

### Documents edited

| Document | Change |
|---|---|
| `app/Cargo.toml` `[patch.crates-io]` comment | Replace the two-fix list + "TEMPORARY … revert to a released crates.io pin" with the real inventory summary and the permanent-fork framing. Point at the upstreaming spec for the full table rather than duplicating it. |
| `specs/projects/ironcalc-upstreaming/…` status table | Rewrite rows from the inventory: fix slug, what it does, upstream status. |
| `CLAUDE.md` §Engine | Reframe: the fork is a permanent operating position. Keep every existing operating rule (one fix = one branch = one PR; sync `main` periodically) — those are correct and load-bearing. Remove only implications of an exit. |
| `projects/ironcalc-upgrade.md` | Same reframe. If the document's entire premise is "get off the fork", it is rewritten to be about *keeping the fork current* (re-syncing from upstream `main`, bumping the A1 rev pin), not about eliminating it. |

### Tests

None (docs). Verification is internal consistency: no document left claiming the fork is
temporary, no unverified merge status asserted as fact, and the manifest comment's inventory
matches the upstreaming table.

---

## 4. D1 — Render gate weekly on `main` + at release

### Confirmation experiment

1. `render.yml` trigger block — read it (already confirmed: `workflow_dispatch` only).
2. The review's run statistics (29 dispatched runs, zero on `main`, 7% failure) are checkable
   via the Actions API. Worth one query, because the "it fails 7% of the time" figure is the
   argument for *not* putting it on releases naively — if the suite is that flaky, a hard
   release gate blocks releases on noise. This is the one number in D1 that changes the
   design, so it is confirmed rather than assumed.

### Design

**Triggers.** `render.yml` gains:

```yaml
on:
  workflow_dispatch:
  schedule:
    - cron: "<weekly, off-peak UTC>"
  workflow_call:        # invoked by release.yml
```

- `schedule` runs on the default branch by definition — that is precisely the "`main` is
  never render-verified" hole, closed.
- `workflow_call` is the mechanism chosen for "run at release", over duplicating the job or
  adding `on: release`. Rationale: `release.yml` already owns the release sequence; a
  `workflow_call` job in it makes the render result a **visible dependency of the release
  run** rather than a separate workflow that may or may not have finished. The alternative
  (`on: release: types: [published]`) runs *after* the release is published, which cannot
  gate anything — it would be observation, not a gate.

**How hard a gate at release.** Given the measured flake rate, the release job calls render
and **fails the release on a red suite**, because that is what "nothing ships without a green
pixel suite" means. Flake is handled by re-running the release workflow, not by making the
gate advisory. If confirmation shows the flake rate is high enough that this is untenable,
the phase records that and makes the render call a non-blocking job with a loud summary —
that decision belongs to the evidence, and the phase writes down which branch it took and
why.

**Concurrency.** The existing `concurrency: render-${{ github.ref }}` with
`cancel-in-progress: true` is a hazard for the new callers: a scheduled run and a release run
on the same ref would cancel each other. The group key must incorporate the event or the
calling run so a release-time run is never cancelled by a schedule tick.

**Header + docs.** In the same phase:
- `render.yml` header: drop "MUST be a required status check" and the bootstrap note's
  now-stale framing; describe schedule + release + dispatch.
- `app/README.md` §CI and `checks.yml` header: correct the claim that `checks` runs the render
  suite.
- `CLAUDE.md` "Render tests — agent-driven (no automatic every-push gate)": rewritten. The
  *scope* and *cost* guidance stays (it is accurate and useful); the "there is no safety net"
  framing is replaced by the real mechanism — weekly on `main`, blocking at release, dispatch
  for branch confirmation — and the agent's responsibility narrows to "run the subset while
  iterating; dispatch the full suite on your branch when you changed in-scope pixels".

### Tests

Workflow YAML cannot be executed here. Verification:
- YAML parse of every edited workflow;
- `workflow_call` contract checked by reading GitHub's requirements (the called workflow needs
  `on: workflow_call:`; the caller uses `uses: ./.github/workflows/render.yml`);
- cron expression verified by hand against the 5-field UTC semantics;
- **the changes are dispatched on the branch** to prove `render.yml` still runs after the
  edit — a `workflow_dispatch` run is the one part of this that *is* executable pre-merge, and
  D1 must not be the change that silently breaks the manual path. This is the only CI
  dispatch in the project; it does not require the full-suite validation phase the repo
  conventions describe, because D1 changes no pixels.

---

## 5. C1 — Part-inventory round-trip test (keystone)

### The production save paths — which one the test must drive

This is the central design question of the unit, and getting it wrong makes the test measure
the wrong thing.

`freecell-engine` has **two** save paths:

1. `WorkbookDocument::save(path)` (`document.rs:348`) — IronCalc's writer only. This is what a
   workbook with **no charts and no authored drawings** takes in production (`worker/run.rs`
   falls back to it when `reinject_source` is `None` and `!has_authored`).
2. `chart::save_with_charts(original, out)` (`save.rs:55`) and `chart::reinject_live_charts`
   (`save.rs:137`) — the **byte-preserve** path: load the original, run IronCalc's writer,
   then splice the original zip's chart/drawing parts + content-type overrides back in.
   `save_with_charts` is the no-edit form and is `pub`.

`reinject_live_charts` is driven from `worker/run.rs`, which this project may not touch — but
it does not need to: `save_with_charts` is the same `reinject` core with an empty patch map,
and it is a public path-in/path-out function. **The test drives `save_with_charts` for
chart-bearing fixtures and `WorkbookDocument::open` + `save` for the rest**, and says in a
comment that these are the two production shapes.

Driving both matters: path 1 is where the loss is largest and path 2 is where the
carry-forward machinery already exists, so the inventory difference between them is itself
information — it shows exactly how much `reinject` is buying.

### Test design

New file: `app/crates/freecell-engine/tests/part_inventory.rs`.

```rust
/// The set of part names in an .xlsx (an OPC zip), sorted.
fn part_names(bytes_or_path) -> BTreeSet<String>
```

Read with the `zip` crate (already a workspace dependency, already used by
`chart/xlsx.rs`).

**Per fixture, per path:**

```
original_parts = part_names(fixture)
saved_parts    = part_names(save(fixture))
dropped        = original_parts - saved_parts        // the loss
added          = saved_parts - original_parts        // writer-introduced parts
```

**The assertion.** `dropped` is compared against a **committed, per-fixture expected set**
declared in the test file as a literal, each entry annotated with what that part is:

```rust
/// personal_monthly_budget.xlsx, plain save path. Every name here is a part IronCalc's
/// writer does not reproduce — i.e. CONTENT WE LOSE. This list is a baseline of a known
/// defect (C3, v1.0), not an approval of it. Adding to it requires a GAPS.md update.
const PMB_PLAIN_DROPPED: &[&str] = &[ … ];
```

Test fails if `dropped != expected`, in **either** direction — a newly dropped part is a
regression, and a part that stops being dropped is a (welcome) change that must update the
baseline so the record stays true.

**Diagnostics are the point.** Failure output prints the symmetric difference, grouped, with
the fixture name — not a raw `assert_eq!` of two sets. A human reading a CI failure must be
able to see "we now also drop `xl/pivotTables/pivotTable1.xml`" without rerunning anything.

**Exclusions.** Deliberately minimal, and applied to `added`, not `dropped`:
- Nothing is excluded from `dropped`. If IronCalc renames a part (e.g. writes
  `xl/sharedStrings.xml` where the original had none), that shows up as an add, not a drop.
- `added` is not asserted at all in the first cut — it is *reported* on failure for context.
  Asserting it would pin IronCalc writer internals, which is not this unit's business.

This design avoids the trap in the functional spec: no exclusion list to argue about, and the
committed baseline carries the honesty (the loss is written out, named, in the test file, not
hidden behind a filter).

**Reopen assertion.** Independently of the inventory, each saved file is reopened via
`WorkbookDocument::open` and must succeed — cheap, and it catches the "we produced a zip that
is not a valid workbook" failure the inventory alone would miss.

**Edit variant.** For one fixture, a single `set_user_input`-equivalent cell edit before save,
proving the inventory is the same whether or not the model was touched. Kept to one fixture:
the loss is a property of the writer, not of the edit, and running every fixture twice
doubles a slow test for little information.

### Fixtures

`app/crates/freecell-engine/tests/fixtures/`: `personal_monthly_budget.xlsx`, `dates.xlsx`,
`numbers_table.xlsx`, `FONTS.xlsx`, `libreoffice_custom_height_wrap.xlsx`, and
`charts/excel_line_chart_workbook.xlsx` (the chart path). FreeCell-authored `fixtures::*`
workbooks are excluded by design — they are the closed loop.

### `GAPS.md`

The measured drop lists are summarised into the existing save-fidelity gap entry: what
classes of content are lost (as opposed to individual part names), at the tier the
remediation doc assigns — the fix is C3 (v1.0), the warning is C2 (next round). If the
existing entry already says this accurately, it is annotated with the now-measured evidence
rather than rewritten.

### Expected outcome and the "it goes red" instruction

The overview says to expect it red and loud. The architecture's position: **it will be red on
first run, and the deliverable is the baseline that captures that redness in a form CI can
keep honest.** A permanently-red required check is not a deliverable; a named, committed,
`GAPS.md`-linked inventory of exactly what is lost is. If the loss turns out to be *empty* on
some fixture, that is recorded too — it would mean the review's "unbounded data loss" framing
is narrower than claimed for that file, which is a finding.

---

## 6. F3a — Chart/cell number-format and colour agreement

### Reframing (from reconnaissance)

`chart-model::numfmt` is not an unconstrained reimplementation racing IronCalc. It is an
explicitly *bounded subset* with a companion predicate `renders_faithfully(code)`, and
`source_fidelity` degrades any chart whose codes fall outside the subset — so out-of-subset
divergence is already **disclosed to the user by the ⚠ badge**.

That makes the sharp, falsifiable invariant:

> For every format code where `renders_faithfully(code)` is `true`, `apply_number_format(code,
> v)` must equal IronCalc's formatting of `v` under `code` — because the chart claims to be
> faithful, and the user compares the axis label against the cell.

A disagreement inside that set is a real bug (a chart labelled Faithful that shows a different
string than its cells). A disagreement outside it is already flagged and is F3/G3 territory.

### Where the test lives

`freecell-chart-model` must stay ironcalc-free — a **hard constraint**, enforced elsewhere in
the tree by a crate-boundary guard test. So the differential test lives in
`freecell-engine`, which depends on both:
`app/crates/freecell-engine/tests/numfmt_agreement.rs`.

IronCalc side of the comparison: `ironcalc_base::formatter::format::format_number(value,
format, locale) -> Formatted` (public; `formatter` and `locale` are `pub mod` in
`ironcalc_base`). The locale is the same one the engine uses (`"en"`), matching how
`WorkbookDocument` loads workbooks — comparing against a different locale would manufacture
disagreements that no user can see.

If for any reason that API is not reachable from the engine crate, the fallback is to compare
through the **engine's own** surface: build a one-cell workbook, set the format code, and read
`formatted_value` — which is exactly the string the user sees in the cell, and therefore an
even better reference. The phase picks whichever is reachable and says so; the second is
strictly more faithful to "what the cell shows", so it is preferred if both work.

### Corpus

Cross-product of codes × values, with the agreement assertion applied **only** where
`renders_faithfully(code)`, and the out-of-subset pairs *recorded* (printed as an informational
table) rather than asserted:

- **Codes:** `""`, `General`, `0`, `0.00`, `#,##0`, `#,##0.00`, `0%`, `0.00%`, `$#,##0.00`,
  `#,##0.00 "kg"`, plus every distinct `formatCode` reachable from the in-tree chart fixtures
  (extracted from the fixture XML so the corpus is grounded in real files, not invented), plus
  representative out-of-subset codes (`0.00E+00`, `yyyy-mm-dd`, `#,##0.00;[Red](#,##0.00)`,
  `[<100]0;0.0`) to populate the informational table.
- **Values:** `0`, `1`, `-1`, `0.5`, `-0.5`, `1234.5`, `-1234.5`, `1e-7`, `1e15`, `0.005`,
  `-0.005` (rounding-boundary), `f64::MIN_POSITIVE`-ish smallness excluded as not
  user-reachable.

Rounding-boundary values are the highest-yield part of the corpus: two independent
implementations most often disagree on half-way rounding, not on shape.

### Fix policy

- Disagreement **inside** the faithful subset → fix `chart-model` to match IronCalc. IronCalc
  is the reference because the cell is the thing the user compares against.
- If a code cannot be made to agree without pulling real format machinery into `chart-model`,
  the correct fix is to make `renders_faithfully` return `false` for it — the chart then
  degrades honestly instead of lying. This is a legitimate outcome and is cheaper and safer
  than growing the reimplementation.
- Disagreement **outside** the subset → no change; recorded.

### Colour helpers

Two separate claims, each with its own experiment:

1. **`rgb_to_hsl` drift** (`freecell-app/src/chart/palette.rs:49` vs
   `freecell-chart-model/src/theme.rs:198`; `% 6.0` vs `.rem_euclid(6.0)`). The experiment: a
   test that runs both over the inputs the app actually feeds them — the five `BASE` colours
   and the theme accents — and compares. `%` and `rem_euclid` differ only for negative
   dividends, which arise when `max == r && g < b`. If no reachable input produces a different
   final `Color`, the divergence is latent, not live: the honest outcome is "not a live bug",
   and the fix is **deduplication** (export the `chart-model` implementation, delete the app
   copy) so it cannot become live — a strictly-smaller change than the review implied, and
   free of crate-graph impact since `freecell-app` already depends on `freecell-chart-model`.
2. **Two Office palettes.** Reconnaissance suggests these are not duplicates:
   `chart_model::ThemePalette::office_default()` is the theme **accent-slot** palette, while
   `freecell-core::palette::FILL_PALETTE` is the cell-fill **swatch list** that happens to
   contain the same accent hexes. If that holds, the claim is **overstated** and the
   deliverable is a guard test asserting the shared accent values agree, plus a correction to
   the remediation doc. If a genuine second `ThemePalette`-equivalent exists, it is merged.

### Tests

- `numfmt_agreement.rs` — the differential corpus (engine crate).
- A colour-helper equivalence test wherever the dedupe lands.
- Existing `chart-model` and `engine` unit tests must stay green; any `apply_number_format`
  change is a behaviour change to axis labels, so `chart-model`'s own tests are the regression
  net.

**Render impact:** changing `apply_number_format` output *would* move chart-render pixels if a
`chart_*` baseline exercises an affected code. The phase checks the `chart_*` baselines' format
codes; if any is affected, it runs `render_tests.sh test chart_` (the subset only, per the
project's working agreement) and refreshes baselines if the change is intentional.

---

## 7. G1 — Detect multi-group (combo) charts

### Confirmation experiment

Construct a real two-group plot area (`c:barChart` + `c:lineChart` in one `c:plotArea`, each
with its own `c:ser`), then assert the *current* behaviour by running:

- `parse_chart_xml(xml)` → the returned `Chart` has only the bar series (the line series is
  gone);
- `source_fidelity(xml)` → `Fidelity::Faithful`.

That pair — data missing, badge says faithful — is the defect, demonstrated rather than read.

For `is_extended_chart`, the experiment is a chart part with the literal string `chartex`
appearing in **content** (a category label, series name, or a cached string value) and no
`cx:` namespace: current code returns `Unsupported`, hiding a perfectly renderable chart
behind a placeholder. If such a false positive can be produced, the bug is confirmed; if the
string genuinely cannot appear outside the namespace declaration in any realistic part, the
claim is overstated and the honest fix is still to tighten the predicate (cheap) while
recording that no live failure was demonstrated.

### Design

**Group counting.** The count belongs where the classifier already lives.
`source_fidelity(chart_xml)` is a pure text classifier over the part; adding a
`multiple_chart_groups(xml)` detector keeps the design coherent with the existing
`has_3d_chart_group` / `is_unsupported_chart` structure and requires no new plumbing between
crates.

Placement in `source_fidelity`'s precedence chain: **after** `is_unsupported_chart` and
alongside `has_3d_chart_group` (both are `Degraded`). This satisfies the functional spec's
edge case that an already-`Unsupported` chart is not upgraded — `Unsupported` returns first.

The detector counts elements whose local name is a chart-group name, using the existing
prefix-agnostic `contains_element`-family helpers, and returns true when the count exceeds
one. It counts *chart-group* names specifically, so axes / `c:layout` / `c:spPr` / `c:dTable`
siblings cannot trigger it.

**Careful case — 3-D normalisation.** A `bar3DChart` + `barChart` combo is two groups *and*
3-D. Both are `Degraded`, so precedence between them does not change the verdict; the *reason*
string should mention both. Which leads to:

**Reason strings.** `Fidelity` today is a bare enum classified from text. The functional spec
asks for a reason that names the dropped groups. If `Fidelity` carries no reason field, adding
one is a wider change than this unit should make (it touches every classifier call site and
the badge UI, and G3 is the unit that reworks the classifier). **Decision: do not widen
`Fidelity`.** Instead:
- the parser (`load.rs`) gains a `tracing::warn!` naming the retained group and the dropped
  ones, so the information exists at runtime;
- the badge's existing Degraded tooltip/text is checked — if it already surfaces a reason
  sourced from a detector, the combo detector supplies one through the same channel; if it
  does not, the phase does not invent that channel and records the limitation.

This keeps G1 the "make it honest" unit it is scoped as, and leaves the reason-plumbing to G3
which owns the classifier rework.

**`is_extended_chart`.** Replace `xml.contains("chartex")` with a test of the real condition:
the extended-chart namespace URI (`…/2014/chartex`) being **declared**, i.e. matched as a
namespace-declaration value rather than as free text, or the root element being `cx:chartSpace`.
The existing helpers in `fidelity.rs` already do prefix-agnostic element matching, so the
tightening is local and testable.

**`GAPS.md`.** The combo row is rewritten to state actual behaviour: only the first chart
group is parsed and rendered; the remaining groups' series are absent; the chart is flagged
Degraded (after this unit). A v2.0 entry for real combo/dual-axis support (G1b) is added or
updated to match.

### Tests

In `fidelity.rs`'s existing test module:
- bar+line combo → `Degraded`;
- single group with axes/`layout`/`spPr`/`dTable` siblings → `Faithful` (the false-positive
  guard);
- `surfaceChart` + `lineChart` → `Unsupported` (precedence);
- `bar3DChart` + `lineChart` → `Degraded`;
- `chartex` as literal content with no `cx:` namespace → not `Unsupported`;
- a genuine `cx:` extended part → `Unsupported`.

In `load.rs`'s tests: the combo fixture parses to the first group's series only (locking the
documented behaviour rather than pretending otherwise).

---

## 8. G5 — `dLbls` per-point overrides

### Confirmation experiment

Take the existing `line_fixture_with_unmodeled()` test fixture, inject a `c:dLbls` carrying a
per-point `c:dLbl` override and a `c:txPr`, parse, change a modelled field
(`data_labels.show_value`), run `patch_chart_source`, and assert on the output. If the
`c:dLbl` and `c:txPr` are gone, the data loss is demonstrated — from the real save path, with
the real patcher.

### Design

The existing code (`save.rs:1317`) does:

```rust
upsert_child(src, ser, &["dLbls"], Some(chrome::dlbls_element(c, l)), …)
```

`upsert_child` replaces the existing node's whole byte range with a model-built element. Every
child the model does not carry — `c:dLbl` per-point overrides, `c:spPr`, `c:txPr`, `c:delete`,
`c:showLeaderLines`, `c:leaderLines`, `c:extLst` — is destroyed.

**The fix follows the pattern already in the file.** `patch_series_color` (`save.rs:1432`) is
the precedent: when the parent element exists and is not self-closing, patch *inside* it with
per-child `upsert_child` calls; only when it is absent or self-closing does it build a whole
element. G5 introduces `patch_data_labels(src, ser, c, new: Option<&DataLabels>, …)` with the
same three arms:

| Existing `c:dLbls` | Action |
|---|---|
| absent | insert a whole `chrome::dlbls_element` (unchanged behaviour — nothing to preserve) |
| self-closing `<c:dLbls/>` | replace the whole element with a built one (lossless: it held nothing) |
| present, with content | **upsert each modelled child inside it**; leave every other child untouched |

**Setting to `None` (clearing labels).** Today this removes the whole `c:dLbls`. That remains
correct — the user asked for no labels, and per-point overrides of removed labels are moot.
Preserving a `c:dLbls` husk to keep unknown children would produce a node that means
"labels on, with defaults" to Excel. So: clearing still removes the node, and this is stated
in a comment so it does not read as an oversight.

**Schema order for the inner upserts.** `CT_DLbls` orders its children: the `c:dLbl` group
first, then either `c:delete` or the settings group (`numFmt`, `spPr`, `txPr`, `dLblPos`,
`showLegendKey`, `showVal`, `showCatName`, `showSerName`, `showPercent`, `showBubbleSize`,
`separator`, `showLeaderLines`, `leaderLines`), then `c:extLst`. Each modelled child therefore
needs its own `FOLLOWING` list — the set of names that may legally follow it — exactly as
`SER_SPPR_FOLLOWING` / `SPPR_FILL_FOLLOWING` already do. These are added as new `const`s beside
the existing ones. Getting this right is what keeps Excel (a strict reader) accepting the
output, and it is checked by re-parsing the patched XML in every test.

Modelled children, and hence the upserts: `numFmt` (from `labels.number_format`), `dLblPos`
(from `labels.position`), the five `show*` flags, `separator`. Each uses the same builder
fragments `chrome::dlbls_element` already composes — so `dlbls_element` is refactored into
per-child fragment functions that both the whole-element builder and the in-place patcher
call, keeping one source of truth for the emitted spelling.

**`c:delete`.** If the existing `c:dLbls` carries `<c:delete val="1"/>` and the model now
turns a label on, leaving `c:delete` would silently suppress the labels. It is a modelled
concept in effect (labels off), so the patcher removes a truthy `c:delete` when it sets any
`show*` flag on. This is an edge case the whole-node replace accidentally got right and an
in-place patch can get wrong — hence an explicit rule and a test.

### Tests

In `save.rs`'s test module:
1. per-point `c:dLbl` overrides survive a `show_value` edit, and the edit lands;
2. `c:txPr` / `c:spPr` label typography survives;
3. an unknown/unmodelled child of `c:dLbls` (e.g. `c:showLeaderLines`) survives;
4. inserting labels where there were none is unchanged (existing
   `patch_adds_data_labels_to_a_series` must still pass);
5. self-closing `<c:dLbls/>` → whole-element replace, round-trips;
6. clearing labels removes the node;
7. `c:delete val="1"` + turning labels on → `c:delete` removed, labels visible on re-parse;
8. every case: `roxmltree::Document::parse(&patched).is_ok()` and a `parse_chart_xml`
   round-trip of the modelled fields.

### Chart-insert collisions — explicitly deferred

`reinject`/`write_authored_charts` cannot compose on a sheet that already carries a drawing,
so inserting a chart onto such a sheet is a hard `SaveError`. This phase **confirms** that
behaviour with a test-or-read and ensures `GAPS.md` describes it accurately at the v1.0 tier.
No fix. If `GAPS.md` already covers it correctly, nothing is added.

---

## 9. Error handling, logging, and conventions

- **No new error types.** Every unit either edits configuration/docs, adds a test, or patches
  within an existing `Result<…>` flow.
- **Logging.** Only G1 adds runtime logging (a `tracing::warn!` on dropped chart groups), at
  parse time — off the render path, so it cannot cost frame time.
- **The `instrument` engine-call counter.** New tests in `freecell-engine` run headless and do
  not touch the render path, so the process-global zero-engine-calls-on-render guard is
  unaffected.
- **Crate boundaries.** No crate gains a dependency. The `chart-model` must-stay-ironcalc-free
  rule is the binding constraint on F3a and is satisfied by locating the differential test in
  `freecell-engine`.

## 10. Test strategy summary

| Phase | Crate(s) built | Test command |
|---|---|---|
| A1 | workspace resolve only | `cargo build --locked -p freecell-engine`; `Cargo.lock` diff review |
| A2 | workspace | `cargo build --locked --workspace`, `cargo deny check --locked`, YAML parse |
| A4 | — | docs consistency read-through |
| D1 | — | YAML parse + a `workflow_dispatch` run of `render.yml` on the branch |
| C1 | `freecell-engine` | `cargo test -p freecell-engine --test part_inventory` |
| F3a | `freecell-engine`, `freecell-chart-model`, `freecell-app` | `cargo test -p freecell-engine --test numfmt_agreement`; `cargo test -p freecell-chart-model --lib`; `chart_` render subset only if labels move |
| G1 | `freecell-chart-model`, `freecell-engine` | `cargo test -p freecell-chart-model --lib`; `cargo test -p freecell-engine --lib` |
| G5 | `freecell-engine` | `cargo test -p freecell-engine --lib` |

`cargo fmt --all --check` on every phase without exception. No full render suite in this
project.

## 11. Phase ordering and why

A1 → A2 → A4 is forced: A1 rewrites the manifest pin and the lock, A2's `--locked` precondition
must be checked against the post-A1 lock, and A4 rewrites the comment A1 touched. The
remaining four (D1, C1, F3a, G1, G5) are genuinely independent and are ordered
cheapest-confirmation-first, with C1 early because it is the keystone and the most likely to
surface something that changes later judgement.
