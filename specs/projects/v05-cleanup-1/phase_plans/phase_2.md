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

> **Superseded — see §"Review remediation" below.** The `CARGO_LOCKED` opt-in described here
> shipped in `64310b6` and was replaced during review remediation by an unconditional
> `--locked` plus an explicit opt-out. The reasoning is kept for the record.

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

---

## Review remediation (2026-07-28)

A code review of `64310b6` raised two Moderate findings against the `CARGO_LOCKED` indirection
and four Mild ones. It independently re-enumerated every cargo invocation reachable from CI and
**confirmed the sweep itself is complete**, and confirmed `cargo deny --locked check` is the
correct argument order, that the committed lock is current, and that the Part-2 DISPROVED call
was right. None of that changed. What changed is the *shape* of the two wrapper scripts.

### The two Moderates, and why they are one fix

1. **The value was spliced into argv, so a wrong value failed GREEN.** `CARGO_LOCKED` reads
   like a boolean but its value became cargo arguments. `CARGO_LOCKED=1` — the obvious guess
   for a flag-named variable — expands to `cargo test 1 --manifest-path …`, where `1` is a
   *test-name filter*. libtest exits 0 on a filter matching nothing. That is precisely the
   "required gate that renders zero pixels" outcome `render_tests.sh`'s `require_tools` block
   exists to prevent, except silent instead of loud.
2. **The indirection was fail-OPEN.** For two of the six workflows the `--locked` guarantee
   depended on a workflow author remembering an `env:` line. A new workflow calling
   `render_tests.sh`, or an edit dropping the `env:`, would silently revert A2 for that path
   with no test, lint or assertion to notice — the same class of silent hole A2 exists to
   close.

**Resolution taken: invert the polarity.** Both scripts now pass `--locked`
**unconditionally**, with an explicit named opt-out, `FREECELL_CARGO_UNLOCKED=1`, for the one
legitimate local case (iterating mid-dependency-edit, before the lock is regenerated). This
closes both findings with one change:

- *Fail-safe instead of fail-open.* Strict is what you get by default, from CI and from a new
  workflow that has never heard of A2. The two `env:` blocks in `render.yml` / `perf-gates.yml`
  are no longer load-bearing and were **removed** (replaced by a one-line comment pointing at
  the scripts, so a reader who expects to find the plumbing there learns where it went).
- *No value can become a cargo argument.* The env var is **compared, never interpolated**: the
  scripts branch on it in a `case` and build an argv **array** (`cargo_locked=(--locked)` or
  `()`), expanded `"${cargo_locked[@]}"`. Unset/empty → strict. Exactly `1` → unlocked, with a
  warning on stderr. **Anything else → hard exit 2**, rather than guessing.
- *Consistency.* `package.sh` / `package.ps1` were already unconditional, justified in the
  original commit by "a release artifact must come from the committed lock however it is
  built". That reasoning applies just as well to a CI gate, and the inconsistency is now gone.
- The local-ergonomics rationale from the original commit is legitimate and is preserved
  intact — it just argues for the opposite *default*. An opt-out costs the one developer who
  is mid-edit a single env var, once; the opt-in cost every future workflow author a silent
  regression.

Documented in both script headers (a `# Env:` block in the usage comment plus the rationale at
the branch).

### Mild fixes

| Finding | Fix |
|---|---|
| `app/deny.toml:3` said CI runs `cargo deny check` — stale as of *this phase*, which made it `cargo deny --locked check`. Part 2 disproved the original header claim and then introduced fresh drift in the same header. | One-word fix: header now says `cargo deny --locked check`. |
| `architecture.md` §2 Tests said "`cargo deny check --locked` succeeds" — invalid CLI (wrong argument order), the reverse of what shipped. | The doc is `status: complete`, so it is **annotated** with a dated correction note rather than silently rewritten. The same note records that §2's env-var design was superseded here. |
| `cargo packager` in `package.sh` / `package.ps1` was neither swept nor exempted. | No actual hole — it runs immediately after a successful `cargo build --locked` in the same script, and cargo-packager does not accept `--locked` — but functional_spec §A2's edge-case rule asks for an inline "why" comment on anything without the flag. Added to both. |
| `perf.sh` inlined `${CARGO_LOCKED:-}` with no comment while `render_tests.sh` got a named variable and a rationale. | The two are now **symmetric**: same `case` block, same array, same header `# Env:` note, cross-referencing each other. |

### Enumeration methodology — the category the sweep's *method* missed

The sweep's result was right, but its method has a blind spot worth recording so the next one
does not re-derive it:

- **Marketplace actions that shell out to cargo.** Six workflows (`checks`, `render`,
  `perf-gates`, `roundtrip`, `macos-verify`, and `release` three times) use
  `Swatinem/rust-cache@v2`, which runs cargo **but not from a `run:` line** — so a grep of
  `run:` blocks misses it entirely. It is **not** a hole: its main step calls
  `cargo metadata … --no-deps`, which does not rewrite `Cargo.lock` (verified empirically by
  the reviewer); the full-deps call lives in the **post** step, after the build has already
  happened. Recorded so the category is not missed again when a new action is added — the
  enumeration must cover `uses:` as well as `run:`.

### Considered and excluded

- **`app/scripts/linux_render_spike.sh:53`** — `cargo build -p freecell-app --bin freecell`,
  the one remaining in-repo cargo build against this workspace without `--locked`. It is a
  manual GPU/lavapipe spike script, **not reachable from any workflow**, so it is outside
  §A2's CI scope and was deliberately left alone. Named here because a reader currently cannot
  distinguish "excluded" from "missed".

### Verification of the remediation

- All six workflow files re-parse as YAML.
- `bash -n` clean on `render_tests.sh`, `perf.sh`, `package.sh`.
- **Behaviour exercised**, not just read — both scripts run against a stub `cargo` that prints
  its argv (and stub capture tooling, so `require_tools` passes):

  | Env | Resulting cargo argv | Exit |
  |---|---|---|
  | unset | `test --locked --manifest-path … -p render-tests cell_` | 0 |
  | `""` | `test --locked …` (empty ≡ unset ≡ strict) | 0 |
  | `1` | `test --manifest-path …` — no `--locked`, warning on stderr | 0 |
  | `--locked` | *refused*: "must be unset or exactly '1'" | 2 |
  | `1 --manifest-path /evil` | *refused* — the value never reaches argv | 2 |

  `generate` (both its `build` and `run` calls) and `perf.sh` (`run` and `gate`) behave
  identically. **No value of the variable can produce a cargo positional argument**, so the
  `CARGO_LOCKED=1`-becomes-a-test-filter failure mode is structurally impossible.
- `cargo metadata --locked` from `app/` still exits 0 — the strict default passes against the
  committed lock.
