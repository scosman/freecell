# Pre-Distribution Security & License Audit

**Status: Future — MANDATORY before any binary distribution (from MVP Phase 1 / re-audited
Phase 13, 2026-07-04).**

## Why

FreeCell ships **no binaries yet** — it is experiments + a spec-driven build. `cargo-deny`
(`app/deny.toml`, pinned cargo-deny 0.19.9) is green in Linux CI, but only because a set of
**transitive** license/advisory items from the pinned `gpui`/`zed` + `ironcalc` trees are
allowed/ignored **with documented rationale**. We do not control those lockfiles, so there
is no safe upgrade at the pinned revs. Every item below **must be re-audited on any gpui/
ironcalc rev bump and resolved before distribution.**

## Current posture (re-audited Phase 13 — unchanged; still no safe upgrade at the pinned revs)

**License exception (architecture §9, tracked vs zed#55470):**
- GPL-3.0 zed tracing crates `ztracing` / `ztracing_macro` / `zlog` (transitive via
  gpui/zed). Allowed via per-crate `exceptions` in `deny.toml`. **GPL in a distributed
  binary is the load-bearing risk** — must be removed or replaced before shipping.
- `bzip2-1.0.6` license (permissive, BSD-like; via `async_zip` → zed util) added to
  `allow`.

**Advisories `ignore`d (all transitive, all "no safe upgrade"):**
- Unmaintained: `RUSTSEC-2025-0052` (async-std, via zed http_client),
  `RUSTSEC-2024-0384` (instant), `RUSTSEC-2024-0436` (paste),
  `RUSTSEC-2026-0173` (proc-macro-error2, via stacksafe→gpui),
  `RUSTSEC-2026-0192` (ttf-parser, via the gpui font stack).
- **quick-xml 0.39.4 DoS on untrusted XML** — `RUSTSEC-2026-0194` (quadratic
  attribute-dup check) + `RUSTSEC-2026-0195` (unbounded namespace-declaration alloc).
  **Provenance (verified against `Cargo.lock` Phase 13):** quick-xml enters the tree *only*
  through **desktop-protocol / build-time** parsers, all consuming **trusted,
  developer-controlled** XML — `wayland-scanner` (build/proc-macro, parses Wayland protocol
  XML), `xcb` (build, X11 protocol XML, uses quick-xml 0.30.0), and `zbus_xml` (D-Bus
  introspection, via the zbus/atspi accessibility stack). **FreeCell feeds no *untrusted*
  XML through any of these.** Notably, this is **NOT** on the `.xlsx` open path: ironcalc at
  `=0.7.1` parses user workbooks with **`roxmltree` + `zip 0.6.6`**, not quick-xml — so
  opening a hostile `.xlsx` does not exercise these advisories. Both are fixed in
  **quick-xml ≥ 0.41**, which needs a zed/wayland bump.
- **ironcalc's xlsx stack (separate line, currently no active advisory):** `roxmltree 0.19`
  + `zip 0.6.6`. `cargo deny` is clean on these today; re-check on any ironcalc bump, and
  pair a corrupt-`.xlsx` fuzz with `projects/xlsx-preservation.md`'s real-file de-risk since
  this **is** the true user-file input path.

**Non-fatal warnings left for pre-distribution:** two `no-license-field` warnings on
zed-internal crates (`gpui_shared_string`, `gpui_util`).

**Bans/sources left permissive** (`multiple-versions`/`wildcards = allow`,
`unknown-git = allow`) because zed's tree carries many duplicate versions + pinned git
forks; tightening (`allow-git` enumeration, dedupe) is pre-distribution hardening, not MVP.

## Work when picked up (before shipping any binary)

1. Re-run `cargo deny check` on the then-current gpui/ironcalc revs.
2. Resolve or replace the **GPL ztracing** dependency (the distribution blocker).
3. Bump to a **zed/wayland/zbus** rev carrying **quick-xml ≥ 0.41** (clears both DoS
   advisories — quick-xml is on the desktop-protocol/build path, not the xlsx path).
4. Re-evaluate each remaining ignored advisory for an available upgrade.
5. Tighten `[bans]` + `[sources]` (enumerate `allow-git`, dedupe versions where feasible),
   add license fields / exceptions for the two warning crates.
6. Consider fuzzing the `.xlsx` open path against a corrupt-file corpus (pairs with
   `projects/xlsx-preservation.md`'s real-file de-risk).
