---
status: complete
---

# Phase 1: Font-warning fix

## Overview

At startup, gpui core's `svg_renderer` logs two benign `WARN`s because it hard-codes Zed's
own bundled font asset paths (`fonts/ibm-plex-sans/…`, `fonts/lilex/…`) and tries to load
them through FreeCell's `AssetSource` to build a font DB for `<text>` inside SVGs. FreeCell
does not serve those paths and its icon SVGs contain no `<text>`, so the loads fail
harmlessly and the font DB is never used — nothing renders wrong.

This phase suppresses exactly the `gpui::svg_renderer` target in the **default** `tracing`
`EnvFilter` (used when `RUST_LOG` is unset). An explicit `RUST_LOG` still wins, so the
warning can be re-enabled for debugging. We do **not** ship Zed's fonts and do **not** alias
our fonts under `fonts/*` (functional_spec §1, architecture §1). No pixel impact.

## Steps

1. `app/crates/freecell-app/src/main.rs`: introduce a `DEFAULT_LOG_FILTER` constant
   (`"info,gpui::svg_renderer=error"`) with a doc comment explaining the suppression and
   that `RUST_LOG` overrides it. Use it as the `unwrap_or_else` fallback for
   `EnvFilter::try_from_default_env()` (which reads `RUST_LOG`), replacing the current
   `EnvFilter::new("info")`. The `try_from_default_env` path is unchanged, so an explicit
   `RUST_LOG` continues to win.

## Tests

- `default_log_filter_silences_svg_renderer_warning` (unit test in `main.rs`): asserts the
  constructed default filter directive contains `gpui::svg_renderer=error`, and that it is a
  well-formed `EnvFilter` directive (strict `EnvFilter::builder().parse` succeeds).
