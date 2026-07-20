# Release Signing & Distribution

**Status: Future — required before publishing any binary (packaging wired unsigned
2026-07-05).**

## Goal

Turn the current **unsigned dev artifacts** into signed, notarized, publicly distributable
releases attached to a GitHub Release.

## Current state

`cargo-packager` produces macOS `.app`/`.dmg`, Linux `.deb`/`.AppImage`, and (experimental)
Windows NSIS `.exe`. Signing status by platform:

- **macOS: unsigned** — `.app`/`.dmg` trip Gatekeeper (right-click → **Open** to run). No
  signing/notarization plumbing yet.
- **Windows: optional Authenticode wired (2026-07-20)** — `package.ps1` can sign the core
  exe + installer via Azure Trusted Signing when env vars are set; otherwise unsigned and
  SmartScreen warns. See item 2 below + `app/PACKAGING.md` §Signing.
- **Linux: unsigned** — no GPG/AppImage signing yet.

The `release` workflow uploads packages as **run artifacts**, not GitHub Release assets,
specifically because publishing unsigned binaries as releases would be wrong.

## Work when picked up

1. **macOS:** Developer ID Application certificate → sign the `.app`, then **notarize** +
   staple the `.dmg` (Apple notary service). Wire cert + credentials as CI secrets.
2. **Windows:** ✅ **Wired (2026-07-20).** Authenticode signing of **both** the inner
   `freecell.exe` and the NSIS installer `.exe` via **Azure Trusted Signing** — `package.ps1`
   signs the core binary before packaging (cargo-packager embeds the signed copy) and the
   installer after, using `trusted-signing-cli` (`scripts/sign-windows.ps1`). Opt-in: a no-op
   unless the `AZURE_*` signing env vars are set (see `app/PACKAGING.md` §Signing), so unsigned
   builds still work. The `release` Windows job maps the creds/config from repo secrets +
   variables. Not yet exercised end-to-end because the Windows app build itself is still
   experimental (`projects/windows-port.md`).
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
