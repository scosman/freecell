# Release Signing & Distribution

**Status: Signing done. macOS `.app`/`.dmg` are Developer-ID-signed + Apple-notarized
(`app/scripts/sign_macos.sh`); Windows `.exe`s are Authenticode-signed via Azure Trusted
Signing in the CI `release` job (`app/scripts/sign-windows.ps1`, run green). Linux is
intentionally unsigned. What remains before *publishing* is distribution (the GitHub Release
switch) and the mandatory pre-distribution security audit — not signing.**

## Goal

Turn `cargo-packager`'s dev artifacts into signed, notarized, publicly distributable releases
attached to a GitHub Release. **Signing is done** (below); distribution is the open half.

## Signing — done

`cargo-packager` produces macOS `.app`/`.dmg`, Linux `.deb`/`.AppImage`, and Windows NSIS
`.exe`. Bare `scripts/package.sh` / `package.ps1` output is unsigned; the signed paths are:

- **macOS — signed + notarized.**
  [`app/scripts/sign_macos.sh`](../app/scripts/sign_macos.sh) wraps `package.sh` to produce a
  Developer-ID-signed (hardened runtime + timestamp), Apple-notarized, stapled `.app` + `.dmg`.
  It is **local/manual** by design — it prompts for the signing identity and reads notary
  credentials from a `notarytool` keychain profile (no secrets in the repo, not in CI). See
  [`app/PACKAGING.md`](../app/PACKAGING.md#macos-signing--notarization).
- **Windows — signed (Authenticode).** `package.ps1` Authenticode-signs **both** the core
  `freecell.exe` (before packaging, so the installer embeds the signed binary) and the NSIS
  installer `.exe` via **Azure Trusted Signing** (`scripts/sign-windows.ps1`). Wired into the
  CI `release` Windows job — it maps the `AZURE_*` creds/config from repo secrets/variables and
  has run green. Opt-in: a no-op when the env is unset, so plain unsigned builds still work.
  See [`app/PACKAGING.md`](../app/PACKAGING.md#windows-signing).
- **Linux — unsigned by design.** GPG-signing the `.deb` / publishing checksums / AppImage
  signing are out of scope; the `.deb`/`.AppImage` ship unsigned.

## Distribution — the open half

The `release` workflow uploads packages as **run artifacts**, not GitHub Release assets. To
actually publish:

1. **Switch to Releases.** Change the workflow from artifact upload to **creating/attaching a
   GitHub Release** on tag push (checksums + release notes).
2. **Gate:** nothing ships until
   [`pre-distribution-security-audit.md`](pre-distribution-security-audit.md) is resolved (GPL
   `ztracing` transitive dep, quick-xml advisories, license exceptions) — **mandatory before
   shipping any binary**. Signing does not change that.

Optional automation (not blocking): `sign_macos.sh` is local/manual; wiring the Developer ID
cert + notary credentials as CI secrets would sign tagged macOS releases hands-free, matching
how Windows already signs in CI.

## Related

- `projects/pre-distribution-security-audit.md` — the hard prerequisite for publishing.
- `projects/windows-port.md` — the Windows port (compiles, packages, signed installer).
- `app/PACKAGING.md` — packaging + the macOS/Windows signing paths.
