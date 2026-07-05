---
status: draft
---

# Charts

A project to (consider) adding charts to FreeCell.

## Motivation / understanding

- The Excel Open format (OOXML / `.xlsx`) has ~17 basic chart types, plus more in an
  extended set.
- `gpui-component` already has chart support for common chart types (line, bar, etc.).

## Scope

The scope of this project would be **at most** to support the chart types
`gpui-component` already supports — **not** getting into the chart-rendering business
ourselves. We render with what `gpui-component` gives us; we don't build a charting
engine.

## Approach

This is a **consideration / de-risking** project first. Before committing to a
functional spec, we do:

1. **Project overview** — this note, refined.
2. **Research phase** — many subagents (serial or parallel), saving findings to the
   `research/` folder under this spec folder. Research questions:
   - Which charts are in the Excel format? Where are the data-model specs?
   - Which chart types does `gpui-component` support?
   - What is the potential scope of this project: bar, line, pie, etc.? What important
     chart types are we leaving out?
   - For each in-scope chart type: compare the Excel data model vs. `gpui-component`.
     Can we make it work? Multiple datasets? Coloring. 3D options. We do **not** expect
     perfect support for every style field — but we want an assessment for each of
     whether we can render it well or will have major gaps. Each chart type gets its
     own doc.
   - IronCalc: does it expose chart data? Or can we roll our own without too much
     trouble?
3. **Discussion phase** — regroup on findings before writing the functional spec.
4. **Functional spec** — only after discussion.
