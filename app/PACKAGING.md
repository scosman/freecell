# Packaging & releases

FreeCell is packaged with [`cargo-packager`](https://crates.io/crates/cargo-packager)
(pinned **0.11.8**). One config drives every platform; one script builds them.

| Platform | Formats | Status |
|---|---|---|
| macOS | `.app` bundle + `.dmg` | **Supported** (primary target) |
| Linux | `.deb` + `.AppImage` | **Supported** (native **x64 + arm64**) |
| Windows | NSIS setup `.exe` | **Supported** (see below) |

> **All builds are UNSIGNED dev builds.** They are **not** for public distribution yet.
> macOS Gatekeeper will block an unsigned `.app`/`.dmg` (right-click → **Open** to run
> anyway); Windows SmartScreen will warn. Code signing + notarization, and publishing to a
> GitHub Release, are deliberately **out of scope** here and gated behind
> [`projects/pre-distribution-security-audit.md`](../projects/pre-distribution-security-audit.md)
> and [`projects/release-signing-and-distribution.md`](../projects/release-signing-and-distribution.md).
> That is why the CI workflow uploads packages as **run artifacts**, not Release assets.

## Config

The packager config is `[package.metadata.packager]` in
[`crates/freecell-app/Cargo.toml`](crates/freecell-app/Cargo.toml). cargo-packager reads it
via `cargo metadata`, so it auto-fills the version (workspace `0.1.0`) and auto-detects the
`freecell` binary — the config sets product name, bundle identifier
(`com.scosman.freecell`), category, short + long description, authors (the deb
`Maintainer:`), homepage, and the icon list.

Package **formats are chosen per-OS by the scripts** (`--formats`), not pinned in the
config, so the same config serves all platforms.

**Linux App Center / GNOME Software metadata.** The `[package.metadata.packager.deb]`
sub-table sets `package-name = "freecell"` (the `Package:` control field — cargo-packager's
default kebab-cased it to `free-cell`) and ships an **AppStream metainfo** file,
[`packaging/linux/com.scosman.freecell.metainfo.xml`](crates/freecell-app/packaging/linux/com.scosman.freecell.metainfo.xml),
into `/usr/share/metainfo/`. App Center reads that metainfo in preference to the bare
Debian control fields, which is what gives the store view the proper name (**FreeCell**),
license (**MIT OR Apache-2.0** — a Debian control file has no license field), and rich
description. Keep the metainfo `<summary>` in sync with the packager `description`; keep the
description ASCII (a Unicode em-dash rendered as `??` in App Center's synopsis).

**Gotcha worth knowing:** cargo-packager `cd`s into the crate manifest directory
(`crates/freecell-app/`) before packaging, so the `icons` paths in the config are relative
to *that* directory (`packaging/icons/...`), not the workspace root or your shell's CWD.

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

## Signing (deferred)

No signing is done here, by design — outputs are plain unsigned artifacts. macOS
notarization and Windows Authenticode, and the switch to published GitHub Releases, are
future work. See
[`projects/release-signing-and-distribution.md`](../projects/release-signing-and-distribution.md),
and note the **mandatory**
[`projects/pre-distribution-security-audit.md`](../projects/pre-distribution-security-audit.md)
(license/advisory re-audit) must be resolved before shipping any binary. The GPL `ztracing`
distribution blocker is already handled — replaced by permissively-licensed no-op stubs via
`[patch]` (`app/vendor/`), so no GPL code is compiled or linked.

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

So the first `v*` tag is the first time `.dmg` / `.AppImage` / `.exe` are actually
assembled. The macOS + Linux jobs run under `set -euo pipefail`, so a format-tool failure
would fail the (required) job; the `libfuse2t64` install is there specifically to de-risk
the AppImage step. If you want to smoke it before tagging, trigger the workflow via manual
dispatch first.
