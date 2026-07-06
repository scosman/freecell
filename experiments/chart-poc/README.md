# chart-poc

De-risking experiments for **FreeCell chart support**. This whole group answers a single
question — **can we build acceptable charts on gpui-component's `plot/` primitives, and how
hard is it?** — and feeds a GO / NO-GO / PARTIAL-GO decision. Spec:
`specs/projects/chart-proof-of-concept/`. Nothing here touches `/app`.

See the spec's `implementation_plan.md` for the phased build order and early-bail gates.

## Layout

```
chart-poc/
  Cargo.toml            # workspace (pinned gpui / gpui-component; MIRRORS app/Cargo.toml)
  rust-toolchain.toml   # pins 1.95.0 (gpui needs cold_path(); app's pin doesn't reach here)
  chart-model/          # the OOXML-shaped data model (functional_spec §2) — gpui-free, shared
  chart-render/         # Exp 2/3: chart widgets over gpui-component primitives + capture harness
    src/                # ticks (nice axis), palette, scenes (chart defs), bar (widget),
                        #   render (gpui window), capture (Xvfb screenshot); bins render_scene + capture
    results/            # committed example PNGs + manifest.json + review.md
    findings.md
  # load-save/          # Exp 1 (added in Phase 4): xlsx parse into the model + save re-injection
```

`chart-model` is a path dependency of `chart-render` (and, later, `load-save`) — it is the
seam: load/save parses *into* it, the renderer draws *from* it.

## Headless capture (Linux container)

gpui has **no windowless GPU capture on Linux** at the pinned zed rev (that path is
macOS/Metal-only — see `experiments/round-3/C-ci-rendering/findings.md`). So we reuse the
repo's proven **on-screen** Linux path (`app/render-tests/src/capture.rs`): render a real gpui
window under `xvfb-run` + Mesa **lavapipe** (software Vulkan), force presentation with
`xrefresh`, and screenshot the window by id with ImageMagick `import`.

**Container prerequisites** (installed via apt — see `chart-render/findings.md`):

```
apt-get install -y mesa-vulkan-drivers x11-xserver-utils x11-utils imagemagick
```

- `mesa-vulkan-drivers` → the lavapipe ICD (`/usr/share/vulkan/icd.d/lvp_icd.json`); the
  Vulkan *loader* ships in the base image but there is **no driver** without this.
- `x11-xserver-utils` → `xrefresh`; `x11-utils` → `xwininfo`; `imagemagick` → `import`.

## Run: capture + review

```bash
cd experiments/chart-poc

# 1. Build (first build is SLOW — heavy git deps; expected).
cargo build -p chart-render

# 2. Capture every scene to results/<name>.png + results/manifest.json.
cargo run -p chart-render --bin capture

# 3. Agent review: an agent judges each PNG against the functional_spec §6 rubric and
#    writes results/review.md. (Build-time agentic step; the orchestrator spawns the
#    reviewer — see chart-render/findings.md "Agentic review".)
```
