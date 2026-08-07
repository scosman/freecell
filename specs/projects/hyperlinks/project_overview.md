---
status: draft
---

# Hyperlinks

Add **hyperlink support** to FreeCell, built on the links work upstream IronCalc just
landed in [`ironcalc/IronCalc#1328`](https://github.com/ironcalc/IronCalc/pull/1328)
("Implements links", merged 2026-08-07).

Our fork (`scosman/ironcalc`) does **not** have it yet — `main` is behind upstream — so
this project starts by catching the fork up and re-pinning FreeCell, then builds the
FreeCell-side UI on top of the new engine API.

## Request (verbatim)

> add hyperlink support to freecell.
>
> See https://github.com/ironcalc/IronCalc/pull/1328 which adds it to root repo. Check
> that our fork has that (might need to be caught up).
>
> Can upgrade to a small /spec project if you feel needed.

## Notes

Escalated from `/spec task` to a project: the work spans a fork sync (upstream `main` →
rebase `freecell-fixes` and the `fix/*` branches, with a large merged-cells delta already
in flight), a new engine-facade surface, new cell rendering (link styling), new mouse
interaction (click-to-open + hover cursor), a new modal (insert/edit link), menu +
toolbar entries, and a pixel-baseline refresh. That is several coherent units of work,
not one.

Scope confirmed as **full**: view + open + create/edit/remove + xlsx round-trip + the
`HYPERLINK()` function — i.e. closing the `GAPS.md` **Hyperlinks** row completely, which
explicitly bundles `HYPERLINK()` with it ("it can compute a display value but is
near-useless until links are clickable").

## Background

`GAPS.md` carries this as an open gap:

> **Hyperlinks** (open, create/edit, preserve) **+ the `HYPERLINK()` function** — engine
> neither imports nor exports hyperlinks; `HYPERLINK` has no `Function` variant or eval
> arm in the pinned fork. Common in home sheets (link lists, indexes). Fork modelling +
> UI (click-to-open, ⌘K to add).

The upstream PR supplies the entire engine half of that gap — link storage per worksheet
with undo/redo, xlsx import/export including relationship parts, and a dynamic
`HYPERLINK()` function — so no new `fix/<slug>` branch is needed on the fork. This is an
**upstream sync**, not a fork fix. FreeCell then owns the native UI, which the upstream
PR only shipped for the web app.
