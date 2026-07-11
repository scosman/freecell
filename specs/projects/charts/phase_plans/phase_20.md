---
status: complete
---

# Phase 20: Chrome editing

## Overview

P20 completes the line-chart MVP: the right-docked edit panel (P19 skeleton) gains **chrome
editing** — set the chart **title**, toggle the **legend** on/off + position, set **axis titles**,
pick **series colors**, and toggle **data labels** (value / category name / percent). These apply
to **both** provenances and to a **single** chart addressed by `ChartId`:

- **Authored** chart → the edit mutates the in-memory model; on save the write-from-model
  serializer (`write::serialize_chart_xml`) re-serializes it (now emitting `c:dLbls` too).
- **Loaded** chart → the edit mutates the retained render model so the in-grid chart re-renders
  **live**; on save the **source-patch** path (`save::patch_chart_source`) splices **only the
  changed chrome sub-elements** into the retained `chartN.xml`, preserving all unmodeled OOXML
  styling **byte-for-byte** (the **edit contract**, functional_spec §6).

The one genuinely new engine machinery is the **chrome patcher** (targeted-XML splicing of
title/legend/axis-title/series-color/data-labels into a loaded chart's retained source) — the P20
extension of the P10/P16 source-patch path.

## Steps

### Engine — protocol (`freecell-engine::worker::protocol`)
1. Add the chrome-edit seam types (engine-free; model + core types only):
   - `enum ChartAxisKind { Category, Value }`
   - `struct DataLabelToggles { show_value, show_category_name, show_percent }`
   - `enum ChartChromeEdit { Title(Option<String>), Legend(Option<LegendPosition>),
     AxisTitle { axis: ChartAxisKind, title: Option<String> }, SeriesColor { series: usize,
     color: Option<Rgb> }, DataLabels(DataLabelToggles) }`
   - `Command::SetChartChrome { sheet: SheetId, id: ChartId, edit: ChartChromeEdit }`.
   Re-export the new types through `worker::mod` + `lib.rs`.

### Engine — chrome serializers (`freecell-engine::chart::chrome`, new)
2. New `chart/chrome.rs`: **prefix-aware** pure serializers (single source of truth for both the
   loaded patch and the authored `dLbls`): `title_tx`, `title_element`, `axis_title_element`,
   `legend_element`, `series_sppr_element`, `sppr_solid_fill`, `dlbls_element`. Each takes the
   file's `c:`/`a:` namespace prefixes so a loaded patch keeps the file's exact prefixes; the
   authored path passes `"c:"`/`"a:"`. Follow the loader's read shapes so every emitted element
   round-trips through `load::parse_*`.

### Engine — source-patch chrome patcher (`freecell-engine::chart::save`)
3. Extend `patch_chart_source(chart_xml, chart)`: after the existing cache reflow, re-parse the
   file XML (`parse_chart_xml`) as `cached` and, for each chrome field that **differs** from
   `chart`, add a targeted splice. Unchanged fields are never touched (preserve-unmodeled).
   - A generic **upsert-before-first-following-sibling** helper: replace an existing child's byte
     range, remove it (empty replacement), or insert a new one before the first schema-later
     sibling (else before the parent's close tag). Same-offset inserts (a series' new `spPr` +
     `dLbls`) are merged in schema order into one insert so ordering is exact.
   - Title: replace the `c:tx` of an existing title (preserving title styling), insert a fresh
     `c:title` (flipping `c:autoTitleDeleted` to `0`) when absent, or remove it (setting
     `autoTitleDeleted=1`) when cleared.
   - Legend / axis titles / series `spPr` solidFill / `c:dLbls`: upsert. Series color splices only
     the `solidFill` inside `spPr` (a line series' `a:ln` stroke survives).
   - Prefix detection: `c:` from the chart node's own tag; `a:` via `root.lookup_prefix(NS_A)`.

### Engine — authored `dLbls` (`freecell-engine::chart::write`)
4. `series_element`: emit `chrome::dlbls_element("c:", labels)` between `spPr` and the data roles
   when the series carries data labels (schema-correct for every `CT_*Ser`). Round-trips via
   `read_data_labels`.

### Engine — loaded-chart chrome mutation (`freecell-engine::chart::binding`)
5. `ChartBindings::edit_chart_by_id(id, impl FnOnce(&mut Chart)) -> bool` — mutate a bound chart's
   render `Chart` in place (only a `Parsed` body; an Unsupported chart has no chrome to edit).

### Engine — worker (`freecell-engine::worker::run`)
6. Route `SetChartChrome` in the `chart_ops` bucket + one-by-one after the edit batch.
7. `set_chart_chrome(sheet, id, edit)` (degrade-guarded): apply `apply_chrome_edit` to the authored
   entry's chart (found by id) **or** the loaded chart via `edit_chart_by_id`; then `commit_chart_op`
   (bump ops/version, re-store snapshot, publish). Unknown id logged.
8. `apply_chrome_edit(&mut Chart, &ChartChromeEdit)`: title/legend/axis-title/series-color set the
   model field; `DataLabels` applies the toggles to **every** series, preserving each series'
   existing numFmt/separator/position (and legend-key/series-name when already on) — clearing to
   `None` only when nothing would show.

### App — panel state + info (`freecell-app::chrome::view`, `freecell-app::shell::window`)
9. `ChartPanel` carries the full chrome state: `is_authored`, `title`, `legend`, `cat_axis_title`,
   `val_axis_title`, `series: Vec<(name, Option<Rgb>)>`, `labels: DataLabelToggles`. `ChartPanelInfo`
   (window) resolves all of it from the snapshot spec for **both** provenances; rename
   `authored_chart_panel_info` → `chart_panel_info`. `ChartSelected` opens the panel for loaded
   charts too; `refresh_chart_panel` reconciles either. Seed the three title `InputState`s only when
   the panel's chart **id changes** (never on every refresh — don't clobber in-progress typing).

### App — panel controls (`freecell-app::chrome::view`)
10. Add title/cat-axis/val-axis `InputState`s (committed on Enter/blur), a legend on/off + R/T/B/L
    position row, per-series color swatch popover (reusing `FILL_PALETTE` + "Automatic"), and three
    data-label toggle buttons. For a **loaded** chart hide the Type + Data-range sections (loaded
    re-type/re-range stays authored-only); show the chrome sections for both.
11. Panel methods (test seams, each: degrade-guard + commit-pending-edit + send `SetChartChrome` +
    optimistic panel update): `set_chart_title_from_panel`, `set_chart_legend_from_panel`,
    `set_chart_axis_title_from_panel`, `set_chart_series_color_from_panel`,
    `set_chart_data_labels_from_panel`. `set_degraded(true)` still closes the panel.

## Tests

- **chrome serializers (engine)**: each emitted element round-trips through the loader's parse;
  prefix-parameterization (a `d:`/`x:` prefixed doc) keeps prefixes.
- **patch_chart_source chrome (engine, save.rs)** — the headline edit-contract tests:
  - editing the **title** of a loaded chart with an **unmodeled** `<c:roundedCorners>` +
    plotArea `<c:spPr>` gradient: (a) the title changed on re-parse, (b) the unmodeled elements are
    **byte-identical** in the patched XML.
  - each field (legend add/remove, axis title, series color into an `a:ln`-bearing `spPr`,
    data-labels) patches only its element; a series `a:ln` stroke survives a color edit.
  - an **unchanged** chart re-patches byte-for-byte (no-op reflow + no chrome edit).
- **authored dLbls (write.rs)**: a series with data labels round-trips serialize→parse.
- **worker unit (run.rs)**: `set_chart_chrome` mutates an authored chart's title/legend/color/labels
  + republishes; mutates a **loaded** chart's title (via a fixture) + bumps the version;
  degrade-guarded.
- **worker seam round-trip (worker_seam.rs, via `discover_and_parse`)** — the headline exit proof:
  open a loaded line-chart fixture (with an unmodeled styling element), `SetChartChrome(Title)`,
  Save → reopen: the title changed **and** the unmodeled element is byte-identical; an authored
  chart's title/legend/color/labels round-trip.
- **chrome view (gpui)**: each panel method sends the right `SetChartChrome`; the panel opens for a
  loaded chart (no Type/range sections); degrade closes it.

## Render validation

The edit panel is **chrome with no pixel baseline** (like the P19 skeleton) — out of pixel scope,
validated by the chrome gpui view tests + an Xvfb smoke launch. Chrome edits re-render the chart
through the **existing** `freecell_app::chart` renderer over runtime model state; **no chart render
code changes**, so **no `chart_*` / `grid_chart_*` baseline can move**. The full suite stays deferred
to P21.
