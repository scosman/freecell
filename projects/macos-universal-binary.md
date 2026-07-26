# macOS Universal Binary (Intel + Apple Silicon)

**Status: Future — a distribution decision, not a build blocker. Spotted 2026-07-26 while
validating macOS signing.**

## Goal

Ship a macOS binary that runs on **both** Apple Silicon and Intel Macs — or make an explicit,
recorded decision to be Apple-Silicon-only.

## Current state: arm64 only, by accident rather than by choice

The macOS `release` job runs on `macos-14` (Apple Silicon) and calls `scripts/package.sh`,
which does a plain `cargo build --release`. That produces an **arm64-only** binary, so the
published `.dmg` **cannot launch on an Intel Mac at all** — not slowly, not under Rosetta;
Rosetta translates x86_64 *to* arm64, never the reverse.

This is confirmed rather than inferred: the notarization log for the first signed build
reports `"arch": "arm64"` for both the bundle and its executable, with no second slice.

Nothing in the repo chose this — it is just what the runner architecture produced. Note the
contrast with Linux, which **deliberately** builds x64 and arm64 as separate native jobs
(see `PACKAGING.md`), and with Windows, which is x64. macOS is the one platform silently
covering half its hardware.

## The decision

Apple Silicon has been the only Mac sold since 2023, but Intel Macs remain in use and Apple
still ships OS updates for some of them. Whether that tail is worth supporting is a product
call, not a technical one. Either answer is fine — the point is to make it deliberately and
write it down, because right now the release quietly excludes those machines.

## Work when picked up

1. Add the `x86_64-apple-darwin` target and build both slices on the same runner (an Apple
   Silicon runner cross-compiles to x86_64 fine; the reverse is not true).
2. `lipo -create` the two binaries into one universal executable **before** packaging —
   check whether cargo-packager 0.11.8 can be pointed at a pre-built universal binary, or
   whether the script needs to lipo into place after `cargo build` and before
   `cargo packager`. Untested either way.
3. Verify the assembled bundle: `lipo -archs FreeCell.app/Contents/MacOS/freecell` should
   list both.
4. Signing and notarization need **no** changes: `codesign` signs a universal binary once,
   and the notary ticket covers every slice. `scripts/sign_macos.sh` should work unmodified
   — the notarization log will simply list both architectures instead of one.
5. Decide whether the GPUI/Metal stack and the pinned dependency tree actually build clean
   for x86_64; that is the main unknown and should be checked before committing to this.

## Related

- `app/PACKAGING.md` — packaging + the macOS signing path.
- `projects/release-signing-and-distribution.md` — the wider release gate.
