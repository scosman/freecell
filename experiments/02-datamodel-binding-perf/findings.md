# Sub-project C — Datamodel Binding & Engine Performance

> Status: **not started** — placeholder created during Phase 0 scaffolding.
> Owned and filled by Phase 2 (functional_spec §6.C, architecture §5). Do not edit
> from other phases. Recorded benchmark output goes in `results/`.

Required sections (functional_spec §5.2):

## Questions

How do we bind the engine to the UI for fast viewport reads and cascade updates?
Compare ≥2 binding designs (naive per-cell / range-bulk / cached+subscription).
Are cascades, writes, and memory within target (functional_spec §5.4)?

## What was done

_(benchmark crate(s); commands to reproduce; inputs generated via
`experiments/shared/datagen`; timings/percentiles via
`experiments/shared/bench_util`)_

## Results / evidence

_(p50/p99/max per benchmark; PASS/FAIL vs targets; results in `results/`)_

## Conclusion

## Recommended design + next-best alternative

## Risks / open questions
