# Release Signing & Distribution

**Status: In progress — macOS local signing + notarization landed 2026-07-26
(`app/scripts/sign_macos.sh`, untested until first run on a Mac with a Developer ID cert).
CI, Windows, Linux, and the GitHub Release switch remain Future. Still required before
publishing any binary (packaging wired unsigned 2026-07-05).**

## Goal

Turn the current **unsigned dev artifacts** into signed, notarized, publicly distributable
releases attached to a GitHub Release.

## Current state

`cargo-packager` produces macOS `.app`/`.dmg`, Linux `.deb`/`.AppImage`, and Windows NSIS
`.exe`. `scripts/package.sh` / `package.ps1` output is **unsigned on every platform** — that
is what CI builds and uploads, and no certificate or credential is stored in the repo.

**macOS has an opt-in signed path (done, 2026-07-26).**
[`app/scripts/sign_macos.sh`](../app/scripts/sign_macos.sh) wraps `package.sh` to produce a
Developer-ID-signed, Apple-notarized, stapled `.app` + `.dmg`. It is deliberately **local
and manual**: it prompts for the signing identity, reads notary credentials from a
`notarytool` keychain profile, and is not wired into CI. Documented in
[`app/PACKAGING.md`](../app/PACKAGING.md#macos-signing--notarization).

Still unsigned:

- Windows: unsigned `.exe` triggers SmartScreen warnings.
- Linux: unsigned `.deb`/`.AppImage`, no checksums published.
- Every CI-produced artifact, including macOS.

The `release` workflow uploads packages as **run artifacts**, not GitHub Release assets,
specifically because publishing unsigned binaries as releases would be wrong.

## Work when picked up

1. ~~**macOS:** Developer ID Application certificate → sign the `.app`, then **notarize** +
   staple.~~ **Done locally** — `scripts/sign_macos.sh` signs (hardened runtime +
   timestamp), notarizes, and staples both the `.app` and the `.dmg`. Remaining: verify it
   on a real Mac, then wire cert + notary credentials as **CI secrets** so tagged releases
   are signed without a human at a keyboard.
2. **Windows:** Authenticode signing of the NSIS `.exe` (and ideally the inner binary) with
   an OV/EV code-signing certificate.
3. **Linux:** optional — GPG-sign the `.deb` / provide checksums; AppImage signing.
4. **Distribution:** switch the workflow from artifact upload to **creating/attaching a
   GitHub Release** on tag push (checksums + release notes).
5. **Only after** [`pre-distribution-security-audit.md`](pre-distribution-security-audit.md)
   is resolved — that audit (GPL `ztracing` transitive dep, quick-xml advisories, license
   exceptions) is **mandatory before shipping any binary** and is the true gate on this.
   Signing does not change that.

## Related

- `projects/pre-distribution-security-audit.md` — the hard prerequisite.
- `projects/windows-port.md` — Windows must actually compile before its installer is worth
  signing.
- `app/PACKAGING.md` — current unsigned packaging.
