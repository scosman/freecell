# Phase 2 — A2: `--locked` on every cargo invocation in CI

**Verdict: CONFIRMED (the `--locked` half) · DISPROVED (the `deny.toml` header half).**

## Part 1 — `--locked`: confirmed, and demonstrated

### The confirmation

Enumerating every cargo invocation reachable from CI — the workflows *and* the scripts they
call — found `--locked` on exactly four lines, all of them `cargo install` of external
tooling in `release.yml` (where the flag means the *tool's* lockfile, not ours). Nothing that
builds or tests this workspace passed it.

The hole was then demonstrated rather than argued. Pinning `anyhow` to a version other than
the locked one, without touching `Cargo.lock`:

```
$ cargo metadata --locked   →  exit 101
    error: cannot update the lock file … because --locked was passed to prevent this

$ cargo metadata            →  exit 0
$ grep -A1 'name = "anyhow"' Cargo.lock
    version = "1.0.99"        # was 1.0.103 — silently rewritten
```

That is the exact PR scenario the unit describes: a dependency edit with no lock update
builds green, against a graph `cargo deny` never audited, and the rewritten lock never
reaches the repository. The signed-binary path (`release.yml` → `package.sh`) had the same
hole.

A first attempt at this experiment was inconclusive and worth recording: tightening `anyhow =
"1"` to `"1.0.100"` passed `--locked`, because the locked 1.0.103 already satisfied the new
requirement, so no lock change was needed. `--locked` only fires when resolution would
actually move — which is the right semantics, and means the flag is not a tripwire on
harmless manifest edits.

### What changed

| File | Change |
|---|---|
| `checks.yml` | `--locked` on `clippy`, `build`, `test`; `cargo deny --locked check` |
| `macos-verify.yml` | `--locked` on `build`, `test` |
| `roundtrip.yml` | `--locked` on the LibreOffice round-trip `test` |
| `app/scripts/package.sh`, `package.ps1` | `--locked` on the release build — this is what `release.yml` actually invokes, and it is the one the unit called out as producing signed binaries |
| `render-tests/scripts/render_tests.sh`, `perf.sh` | forward `${CARGO_LOCKED:-}` to their cargo calls |
| `render.yml`, `perf-gates.yml` | set `CARGO_LOCKED: "--locked"` |

`cargo fmt` is deliberately left bare: it resolves nothing.

### Why the scripts take an env var rather than a hard `--locked`

`render.yml` and `perf-gates.yml` do not call cargo directly — they call
`render_tests.sh` / `perf.sh`, which are also the everyday local iteration tools (CLAUDE.md
tells agents to run the render subset constantly). The unit's scope is *CI*, so CI sets
`CARGO_LOCKED=--locked` and gets the strict behaviour, while a local run mid-dependency-edit
is not made to fail. The two packaging scripts are the deliberate exception and get an
unconditional `--locked`: a release artifact must be built from the committed lock whether it
is produced by CI or by hand.

### The precondition — is the committed lock current?

`--locked` turns a stale lock into a red CI. Verified at HEAD (post-Phase-1 lock):

- `cargo metadata --locked --format-version 1` — exit 0. This resolves the **entire**
  workspace graph including dev-dependencies, which is precisely the property `--locked`
  guards; a full `cargo build --workspace` would compile more but could not disprove anything
  `cargo metadata` already proved about resolution.
- `cargo deny --locked check` (pinned 0.19.9, the CI version) — `advisories ok, bans ok,
  licenses ok, sources ok`. This also confirms the `cargo deny --locked check` invocation is
  valid CLI: `--locked` is a cargo-deny *global* option and must precede the subcommand.
- `cargo build --locked -p freecell-engine` — clean (Phase 1).

### Verification

- All six workflow files re-parse as YAML.
- `bash -n` clean on all three edited shell scripts.

## Part 2 — the `deny.toml` header: disproved

The unit says:

> While in `deny.toml`: its header still references a GPL exception that was deliberately
> replaced with `exceptions = []`. Fix the comment.

**It does not.** `app/deny.toml` at HEAD:

- header, lines 4–10: *"The former load-bearing item — the GPL-3.0 zed tracing crates … — is
  now RESOLVED: they are replaced at build time by permissively-licensed no-op stubs … **There
  is therefore NO license exception here**; if a gpui rev bump ever reintroduces a GPL crate,
  this gate fails loudly rather than silently allowing it."*
- lines 69–76: *"**No per-crate license exceptions.** The zed GPL tracing family … is no
  longer in the graph … Keep this empty: a GPL crate reappearing after a gpui bump should FAIL
  the gate"*, followed by `exceptions = []`.

The header and the config agree, and both describe the current mechanism correctly. Commit
`19195b2` ("Replace GPL ztracing shims with permissive no-op stubs") updated the comment and
the `exceptions` list in the same change — the reviewer appears to have matched on the word
"exception" appearing in the header without reading that the sentence containing it says the
exception is *gone*.

**No change made.** Editing a correct comment to satisfy a mistaken finding would be worse
than leaving it.

## Scope held

No `cargo tree -i zlog` guard — the unit explicitly drops it, and `cargo deny --locked check`
(now with the lock provably current) already fails on a reintroduced GPL crate.
