# Sub-project E — UI Technical Test (GPUI Proof-of-Concept, macOS)

> Status: **not started** — placeholder created during Phase 0 scaffolding.
> Owned and filled by Phase 3 (functional_spec §6.E, architecture §7). Builds/runs
> on **macOS/Metal**, run by the human. Do not edit from other phases.
> `raw-gpui/` and `gpui-component/` hold the two PoC variants; `scripts/` holds the
> one-command macOS build/run; `results/` holds the logged "Run Test" pass/fail.

Required sections (functional_spec §5.2):

## Questions

Can GPUI render an Excel-max grid at the perf bar (functional_spec §5.4)? Raw `gpui`
vs `gpui-component`?

## What was done

_(both variants over the static datamodel provider from
`experiments/shared/datagen::CellSource`; in-app "Run Test" harness)_

## Results / evidence

_(measured frame p50/p99/max and cell-load latency on macOS/Metal; PASS/FAIL;
logs in `results/`; optional known-good PNGs)_

## Conclusion

## Recommended design + next-best alternative

_(raw-gpui vs gpui-component)_

## Risks / open questions
