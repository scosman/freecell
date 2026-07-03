# DECISIONS_TO_REVIEW — implementation-phase log

**Purpose (part of the autonomy contract — see `implementation_plan.md`):**
implementation agents work autonomously and never stop to ask a human. When you make
a judgment call the specs don't cover, deviate from a spec because reality forced it
(e.g., an API missing at the pinned rev), or resolve a placeholder (pinned SHA,
calibrated threshold), **append an entry here and keep building**. A human reviews
this file at their leisure — it is a log, not a request queue.

Entry format: `- [Phase N] <decision> — <one-line rationale> (<files/spec section
affected>)`.

Known placeholders the build will resolve (append the resolution here):

- gpui-component pinned SHA + Rust toolchain version (Phase 1).
- Linux render-capture variant proven by the Phase-1 spike (or fallback-to-macOS).
- Perf-gate CI thresholds calibrated on the pinned runner image (Phase 12).
- Perceptual-diff thresholds after first real baselines (Phase 7).

---
*Append entries below this line. Do not edit above it.*
