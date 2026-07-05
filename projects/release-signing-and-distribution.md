# Release Signing & Distribution

**Status: Future — required before publishing any binary (packaging wired unsigned
2026-07-05).**

## Goal

Turn the current **unsigned dev artifacts** into signed, notarized, publicly distributable
releases attached to a GitHub Release.

## Current state

`cargo-packager` produces macOS `.app`/`.dmg`, Linux `.deb`/`.AppImage`, and (experimental)
Windows NSIS `.exe`. Everything is **unsigned by design** — no signing config, no signing
hooks, no secret plumbing exist in the repo (a deliberate scope decision, 2026-07-05):

- macOS: unsigned `.app`/`.dmg` trip Gatekeeper (right-click → **Open** to run).
- Windows: unsigned `.exe` triggers SmartScreen warnings.

The `release` workflow uploads packages as **run artifacts**, not GitHub Release assets,
specifically because publishing unsigned binaries as releases would be wrong.

## Work when picked up

1. **macOS:** Developer ID Application certificate → sign the `.app`, then **notarize** +
   staple the `.dmg` (Apple notary service). Wire cert + credentials as CI secrets.
2. **Windows:** Authenticode signing of the NSIS `.exe` (and ideally the inner binary) with
   an OV/EV code-signing certificate.
3. **Linux:** optional — GPG-sign the `.deb` / provide checksums; AppImage signing.
4. **Distribution:** switch the workflow from artifact upload to **creating/attaching a
   GitHub Release** on tag push (checksums + release notes).
5. **Only after** [`pre-distribution-security-audit.md`](pre-distribution-security-audit.md)
   is resolved — that audit (GPL `ztracing` transitive dep, quick-xml advisories, license
   exceptions) is **mandatory before shipping any binary** and is the true gate on this.

## Related

- `projects/pre-distribution-security-audit.md` — the hard prerequisite.
- `projects/windows-port.md` — Windows must actually compile before its installer is worth
  signing.
- `app/PACKAGING.md` — current unsigned packaging.
