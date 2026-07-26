# Packaging & releases

FreeCell is packaged with [`cargo-packager`](https://crates.io/crates/cargo-packager)
(pinned **0.11.8**). One config drives every platform; one script builds them.

| Platform | Formats | Signing | Status |
|---|---|---|---|
| macOS | `.app` bundle + `.dmg` | **Developer ID + notarized** (local, opt-in — see below) | **Supported** (primary target) |
| Linux | `.deb` + `.AppImage` | unsigned | **Supported** (native **x64 + arm64**) |
| Windows | NSIS setup `.exe` | unsigned | **Supported** (see below) |

> **`package.sh` / `package.ps1` output is UNSIGNED on every platform** — that is what CI
> builds and uploads. macOS additionally has an **opt-in signed + notarized release path**,
> [`scripts/sign_macos.sh`](#macos-signing--notarization), which you run locally with a
> Developer ID certificate; it is **not** wired into CI (no secrets are stored in this
> repo). Windows Authenticode and Linux signing remain future work.
>
> **None of this makes a build publicly distributable yet.** Publishing any binary is still
> gated on
> [`projects/pre-distribution-security-audit.md`](../projects/pre-distribution-security-audit.md),
> with the remaining release plumbing in
> [`projects/release-signing-and-distribution.md`](../projects/release-signing-and-distribution.md).
> That is why the CI workflow uploads packages as **run artifacts**, not Release assets.

## Config

The packager config is `[package.metadata.packager]` in
[`crates/freecell-app/Cargo.toml`](crates/freecell-app/Cargo.toml). cargo-packager reads it
via `cargo metadata`, so it auto-fills the version (workspace `0.1.0`) and auto-detects the
`freecell` binary — the config sets product name, bundle identifier
(`net.scosman.freecell`), category, short + long description, authors (the deb
`Maintainer:`), homepage, and the icon list.

Package **formats are chosen per-OS by the scripts** (`--formats`), not pinned in the
config, so the same config serves all platforms.

**Linux App Center / GNOME Software metadata.** The `[package.metadata.packager.deb]`
sub-table sets `package-name = "freecell"` (the `Package:` control field — cargo-packager's
default kebab-cased it to `free-cell`) and ships an **AppStream metainfo** file,
[`packaging/linux/net.scosman.freecell.metainfo.xml`](crates/freecell-app/packaging/linux/net.scosman.freecell.metainfo.xml),
into `/usr/share/metainfo/`. App Center reads that metainfo in preference to the bare
Debian control fields, which is what gives the store view the proper name (**FreeCell**),
license (**MIT OR Apache-2.0** — a Debian control file has no license field), and rich
description. Keep the metainfo `<summary>` in sync with the packager `description`; keep the
description ASCII (a Unicode em-dash rendered as `??` in App Center's synopsis).

**Gotcha worth knowing:** cargo-packager `cd`s into the crate manifest directory
(`crates/freecell-app/`) before packaging, so the `icons` paths in the config are relative
to *that* directory (`packaging/icons/...`), not the workspace root or your shell's CWD.

**Linux window-icon gotcha.** cargo-packager derives the installed Linux **desktop-entry id
and icon name from the *binary name*** — it ships `usr/share/applications/freecell.desktop`
with `Icon=freecell` and hicolor icons named `freecell.png`, *not* from the reverse-DNS
`identifier`. GNOME maps a running window to its launcher (and thus its dock icon + app name)
by the window's `app_id` (Wayland) / `WM_CLASS` (X11). So the app sets its Linux `app_id` to
**`freecell`** (`window_app_id()` in `shell/app.rs`) to match that desktop-entry id — *not*
`net.scosman.freecell`. Pointing `app_id` at the reverse-DNS identifier (which has no matching
installed `.desktop`) makes GNOME show a **generic icon labelled with the raw app_id** — even
for a correctly-installed `.deb`. Keep `window_app_id()` equal to the binary name. (The
reverse-DNS `identifier` is still the macOS/Windows bundle id, where that form is required.)

Icons are final — see
[`crates/freecell-app/packaging/icons/README.md`](crates/freecell-app/packaging/icons/README.md)
for how they're generated from the two source PNGs.

## Build locally

From `app/`:

```sh
# One-time: install the pinned packager (and your platform's build deps — see README.md).
cargo install cargo-packager --locked --version 0.11.8

# macOS / Linux:
scripts/package.sh                 # platform defaults (mac: app,dmg | linux: deb,appimage)

# Windows (PowerShell):
scripts\package.ps1                # nsis
```

The scripts build the release binary first (cargo-packager does **not** build for you, and
the binary profile must match), then package. Output lands in **`app/target/packages/`**
(git-ignored). Extra flags pass through (e.g. `scripts/package.sh --verbose`).

These produce **unsigned** artifacts. For a real macOS release build, use
[`scripts/sign_macos.sh`](#macos-signing--notarization) instead — it wraps `package.sh` and
adds signing, notarization, and stapling.

Overrides (both scripts honor these env vars):

```sh
FREECELL_PACKAGE_FORMATS=dmg  scripts/package.sh    # build just one format (comma list)
FREECELL_PACKAGE_OUT_DIR=/tmp/pkgs scripts/package.sh
```

### Platform prerequisites

- **All:** the pinned Rust toolchain + `cargo-packager` + your platform's normal FreeCell
  build deps (see [`README.md`](README.md)). Packaging also needs **network access** the
  first time per format — cargo-packager downloads its own helper tools (create-dmg on
  macOS, linuxdeploy/AppRun for AppImage, the NSIS toolchain on Windows).
- **Linux `.deb`:** pure Rust, no extra tools.
- **Linux `.AppImage`:** needs `file` and `patchelf` on `PATH` (used by linuxdeploy).
  cargo-packager runs linuxdeploy with `--appimage-extract-and-run`, so FUSE is normally
  not required; if a minimal runner ever fails to assemble the AppImage, install FUSE
  (`libfuse2t64` on Ubuntu 24.04 — the CI job does this defensively; `libfuse2` on older
  distros).
- **macOS `.dmg`:** uses the system `hdiutil` (present on macOS).

## CI: the `release` workflow

[`.github/workflows/release.yml`](../.github/workflows/release.yml) triggers on:

- a **version tag push** matching `v*` (e.g. `git tag v0.1.0 && git push --tags`), or
- **manual dispatch** (Actions → *release* → *Run workflow*).

It has three job definitions — **macOS**, **Linux**, and **Windows** (all required); Linux
fans out to two runners, so a run shows four job instances. The Linux job is a **matrix over two native runners** —
`ubuntu-24.04` (x64) and `ubuntu-24.04-arm` (arm64) — so each architecture is a true native
build, not a cross-compile. Each leg installs the pinned toolchain + cargo-packager, then
calls the **same** `scripts/package.*` used locally, and uploads the result as a workflow
**artifact** (`freecell-macos` / `freecell-linux-x64` / `freecell-linux-arm64` /
`freecell-windows`), downloadable from the run page. No GitHub Release object is created or
attached.

## Windows

Windows is a **first-class, working target**: it compiles, packages the NSIS `.exe` (via
`scripts/package.ps1`), and its `release` CI job is **required** — a packaging failure gates
the release exactly like macOS and Linux (CI green + local build confirmed 2026-07-24).

**File associations are wired.** The shared
[`[[package.metadata.packager.file-associations]]`](crates/freecell-app/Cargo.toml) block
(two entries — `.xlsx`, `.csv`) makes cargo-packager emit the Windows NSIS registry ProgId +
`shell\open\command`, and the delivered path reaches the app through `main.rs::open_arg` →
`FreeCellApp::open_path` (process **argv** — the Explorer double-click / Open-With / shell-arg
flows). See [`specs/projects/xlsx-file-association/`](../specs/projects/xlsx-file-association)
(Phase 1, commit `84785fa`).

What remains is not build work: a Windows **hardware smoke** of the installed NSIS build
(double-click + Open-With), complementary to the CI green build, and Authenticode **signing**
(deferred with the rest of signing — see below). Any residual per-monitor-DPI / installed-app
data-path polish is tracked in
[`projects/windows-port.md`](../projects/windows-port.md).

## macOS signing & notarization

[`scripts/sign_macos.sh`](scripts/sign_macos.sh) is the macOS **release** path. It wraps
`package.sh` and produces the distributable pair — both signed with your Developer ID,
notarized by Apple, and stapled:

```
app/target/packages/FreeCell.app          signed + notarized + stapled
app/target/packages/FreeCell 0.1.0.dmg    signed + notarized + stapled
```

It is **local and manual** by design: it prompts you to choose a signing identity, and no
certificate or notary credential is stored in this repo or in CI.

### One-time setup

1. **Developer ID Application certificate** in your login keychain. This needs a paid Apple
   Developer Program membership. An *Apple Development* certificate is **not** sufficient —
   it cannot be notarized or distributed. Create/download one from
   [developer.apple.com → Certificates](https://developer.apple.com/account/resources/certificates)
   and double-click to install. Verify with:
   ```sh
   security find-identity -v -p codesigning | grep "Developer ID Application"
   ```
2. **create-dmg** — [`sindresorhus/create-dmg`](https://github.com/sindresorhus/create-dmg),
   needs Node 20+:
   ```sh
   npm install --global create-dmg
   ```
3. **Notary credentials**, stored once in your keychain (the script never takes a password):
   ```sh
   xcrun notarytool store-credentials freecell-notary \
       --apple-id <you@example.com> --team-id <TEAMID> --password <app-specific-password>
   ```
   Generate the app-specific password at [account.apple.com](https://account.apple.com) →
   *Sign-In and Security* → *App-Specific Passwords*. Override the profile name with
   `FREECELL_NOTARY_PROFILE`.
4. Xcode command line tools (`xcode-select --install`) and network access.

The script preflights all four **before** the slow release build, so a missing prerequisite
costs seconds and prints the exact command to fix it.

### Run it

From `app/`:

```sh
scripts/sign_macos.sh              # extra args pass through to package.sh
```

It will prompt you to pick from your *Developer ID Application* identities, then run
unattended. Expect it to take a while: two notarization round trips, typically a few
minutes each, occasionally much longer.

### What it does, and why

1. **Packages the `.app` only** (`FREECELL_PACKAGE_FORMATS=app`). `package.sh`'s macOS
   default is `app,dmg`, and that `.dmg` is built from the *unsigned* bundle — producing it
   here would leave an unsigned `.dmg` sitting next to the signed one in the same directory.
   The signed `.dmg` comes from create-dmg instead. (The script warns about any other `.dmg`
   it finds in the output directory for exactly this reason.)
2. **Signs nested Mach-O inner-out, then the bundle**, with
   `--force --timestamp --options runtime`:
   - `--force` is **required** — every arm64 Mach-O carries an ad-hoc signature applied by
     the linker, so signing without it fails on Apple silicon.
   - `--timestamp` and `--options runtime` (hardened runtime) are both **required for
     notarization**; the notary service rejects a signature lacking either.
   - **Not `--deep`** — Apple deprecated it and it silently mis-signs nested code. For a
     pure-Rust bundle the nested loop usually finds nothing; it is insurance for the day a
     dylib or helper binary appears.
   - No entitlements by default; a GPUI/Metal app should not need any under the hardened
     runtime. If a *signed* build crashes on launch where the unsigned one did not, that is
     the tell — point `FREECELL_ENTITLEMENTS` at a plist.
3. **Notarizes and staples both the `.app` and the `.dmg`.** Stapling the app as well as the
   disk image is what Apple's documentation describes: the app then carries its own ticket,
   so it launches even offline after being dragged out of the `.dmg`. The app is submitted as
   a `ditto -c -k --keepParent` archive — `zip` mangles symlinks and extended attributes in a
   bundle and the notary service rejects the result.
4. **Verifies**: `stapler validate` on both, then `spctl --assess`, which must report
   `accepted … source=Notarized Developer ID`. Anything else fails the run.

If notarization comes back `Invalid`, the summary status alone is useless, so the script
automatically fetches and prints `xcrun notarytool log <submission-id>`. The reason is
almost always a missing hardened-runtime flag or an unsigned nested binary.

### Verifying like a real downloader

A locally-built app is never quarantined, so it launches even unsigned — passing `spctl`
locally is necessary but not the whole story. To reproduce what someone downloading the
`.dmg` actually experiences, copy it elsewhere and set the quarantine attribute by hand:

```sh
xattr -w com.apple.quarantine '0081;00000000;Safari;' "FreeCell 0.1.0.dmg"
```

Then mount it, drag the app out, and launch. A correctly notarized build opens with no
Gatekeeper prompt at all.

> Signing **without** notarization is not enough for distribution: a signed-but-unnotarized
> download still gets *"Apple could not verify FreeCell is free of malware."* That is why
> this script always notarizes rather than offering a sign-only mode.

## Signing — what's still deferred

- **CI.** The `release` workflow builds **unsigned** artifacts. Wiring the certificate and
  notary credentials in as GitHub secrets is future work.
- **Windows Authenticode** for the NSIS `.exe` (and the inner binary) — needs an OV/EV
  code-signing certificate.
- **Linux** — optional GPG-signed `.deb` / checksums / AppImage signing.
- **Publishing.** Switching the workflow from artifact upload to attaching assets to a
  GitHub Release.

See
[`projects/release-signing-and-distribution.md`](../projects/release-signing-and-distribution.md).
Note the **mandatory**
[`projects/pre-distribution-security-audit.md`](../projects/pre-distribution-security-audit.md)
(license/advisory re-audit) must be resolved before shipping any binary — signing does not
change that. The GPL `ztracing` distribution blocker is already handled — replaced by
permissively-licensed no-op stubs via `[patch]` (`app/vendor/`), so no GPL code is compiled
or linked.

## Verification status

**Verified locally (cargo-packager 0.11.8, built on Linux x64):**

- `.deb` (x64) — installs the binary, desktop entry, and all hicolor icon sizes (16→512 +
  `256x256@2`), with a correct control file.
- macOS `.app` bundle — gets the `.icns` in `Contents/Resources` and a correct `Info.plist`
  (identifier, product name, `public.app-category.productivity`). *Built* on Linux; not yet
  run on macOS.

**Not yet produced — driven by the same validated config, but first built when the
`release` workflow runs on a `v*` tag (or when you run the scripts on each OS):**

- `.dmg` (needs macOS `hdiutil` / create-dmg — macOS-only, not runnable in the Linux
  validation env).
- `.AppImage` (needs linuxdeploy + network; the Linux job installs `file`, `patchelf`, and
  `libfuse2t64` as FUSE insurance — see the caveat below).
- The **arm64** `.deb` + `.AppImage` — same config, first built natively on the
  `ubuntu-24.04-arm` matrix leg when the `release` workflow runs (never validated locally in
  the x64 env above).
- NSIS `.exe` (Windows — see the Windows section; first assembled when the `release` workflow
  runs on Windows, or when you run `scripts/package.ps1` on Windows).

**Not yet run at all:** `scripts/sign_macos.sh`. It was written against Apple's documented
`codesign`/`notarytool`/`stapler` behavior and `create-dmg`'s CLI, but signing and
notarization cannot be exercised anywhere except a macOS machine holding a Developer ID
certificate — so its first real run is its first validation. Update this note once it has
produced a notarized build.

So the first `v*` tag is the first time `.dmg` / `.AppImage` / `.exe` are actually
assembled. The macOS + Linux jobs run under `set -euo pipefail`, so a format-tool failure
would fail the (required) job; the `libfuse2t64` install is there specifically to de-risk
the AppImage step. If you want to smoke it before tagging, trigger the workflow via manual
dispatch first.
