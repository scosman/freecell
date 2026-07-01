# C — CI snapshot rendering (authored for a macOS / human run)

FreeCell's north star includes **rendering tests vs known-good PNGs.** That strategy
depends on being able to **capture a snapshot of the GPUI grid in CI.** C confirms at
least one working mechanism end-to-end: **render → PNG → perceptual diff within
tolerance** (fuzzy / perceptual match is acceptable — anti-aliasing / font differences
must not cause false failures; a deliberately-changed scene must still fail, proving the
diff has discriminating power).

## Why this crate is minimal right now

**No GPUI / GPU / PNG / diff dependencies are wired at scaffolding time**, and none
should be added here until Phase C. Two reasons:

1. **GPUI needs a real GPU.** The Round-3 container is headless (no GPU / no display).
   The demonstrable half of C is **run by a human on macOS** (architecture §5, §7,
   overview §7) — the code + build scripts are authored here; the human runs them and
   reports.
2. **"Is headless capture even possible?" is part of C's investigation.** Whether GPUI
   can render offscreen/windowless in CI is a finding, not an assumption — attempting
   (and likely failing) the in-container build is itself data. We do not pre-commit GPUI
   deps that would force a heavy in-container build during scaffolding.

## What Phase C adds (not now)

- **On macOS:** a GPUI dependency (git-pinned, per Phase-1 `experiments/04-ui-poc/`), a
  PNG encoder, and a perceptual-diff crate.
- A minimal harness that renders the raw-gpui grid (evolving Phase-1 `04-ui-poc`) to a
  PNG, commits a **baseline**, and runs a tolerance-based perceptual diff of a re-render
  (must pass) and a deliberately-changed scene (must fail).
- **In-container:** an investigation of GPUI's offscreen/headless capture surface and an
  attempt at it — the failure mode (if it fails) is recorded as the finding.

Until then, `src/main.rs` is a trivial stub that compiles in-container with no heavy
deps.
