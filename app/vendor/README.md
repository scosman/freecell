# `app/vendor/` — permissively-licensed GPL-shim replacements

These are tiny, self-contained crates FreeCell owns (MIT OR Apache-2.0) that stand in for
zed's **GPL-3.0-or-later** tracing shims via `[patch]` in `app/Cargo.toml`.

## Why

gpui (git dep) pulls `sum_tree`, which depends on `ztracing`, which depends on `zlog` and
`ztracing_macro`. All three declare **GPL-3.0-or-later**. GPL in a distributed binary is a
hard shipping blocker (tracked upstream as zed#55470), previously papered over by a
documented license *exception* in `app/deny.toml`.

The GPL code is **functionally unused** in FreeCell: `ztracing` only does real work when the
`ZTRACING` env var is set (a Tracy-profiling build), which FreeCell never sets. In every
normal build it already compiles to a no-op shim, and `sum_tree` uses only its pass-through
`#[instrument]` attribute. So the fix is not to work around a bug — it is to **stop compiling
and linking the GPL crates at all** by substituting equivalent no-op crates we own.

## How it works

`app/Cargo.toml` contains:

```toml
[patch."https://github.com/zed-industries/zed"]
ztracing = { path = "vendor/ztracing" }
ztracing_macro = { path = "vendor/ztracing_macro" }
```

- `vendor/ztracing` reproduces upstream's no-op public API (`instrument`, the span/event
  macros, `Span`, `Level`/`field` re-exports, `init()`). Crucially it does **not** depend on
  `zlog`, so `zlog` — whose only dependent was `ztracing` — falls out of the graph entirely.
- `vendor/ztracing_macro` is a one-attribute proc-macro whose `instrument` returns the item
  unchanged (matching upstream's no-op).

Result: `ztracing`, `ztracing_macro`, and `zlog` no longer appear in `Cargo.lock` or the
compiled binary, so **no GPL-3.0 code is compiled or linked**. Verify with:

```sh
cd app && cargo tree -i ztracing   # -> "package ID specification ... did not match any packages"
```

These crates are **excluded** from the workspace (see `[workspace].exclude` in
`app/Cargo.toml`) and are not real tracing implementations — they are deliberately inert.

## Maintenance

This is a stand-in until zed relicenses the tracing shims (zed#55470) or the dependency is
otherwise removed upstream. On any **gpui/zed rev bump**, re-check that the patched crates
still match (name + `0.1.x` version) and that `ztracing`'s public API hasn't grown a surface
`sum_tree`/gpui now uses; run `cargo tree -i ztracing` and `cargo deny check licenses` after
bumping. See `projects/pre-distribution-security-audit.md`.
