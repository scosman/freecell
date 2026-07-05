<#
  Build a distributable FreeCell package on Windows with cargo-packager.

  Windows sibling of `package.sh`. EXPERIMENTAL / non-blocking: Windows is out of scope
  for the app today (the GPUI platform config wires only macOS/Metal + Linux x11/wayland),
  so the release build is NOT guaranteed to compile here without extra port work. See
  `../PACKAGING.md` (section "Windows: what a real port needs"). This script + the Windows
  CI job exist so the packaging path is ready the day the port lands.

  Produces an NSIS setup .exe. Builds are UNSIGNED (SmartScreen will warn); signing is
  future work (PACKAGING.md + projects/pre-distribution-security-audit.md).

  cargo-packager does not build the app itself and resolves icon paths relative to the CWD,
  so this builds the release binary first and always runs from `app/`.

  Usage:
    scripts\package.ps1
    scripts\package.ps1 --verbose                       # extra flags pass to cargo-packager
    $env:FREECELL_PACKAGE_FORMATS = 'nsis'; scripts\package.ps1
    $env:FREECELL_PACKAGE_OUT_DIR = 'C:\pkgs'; scripts\package.ps1

  Requires: the pinned Rust toolchain, an MSVC build environment, and `cargo-packager`
  (install: cargo install cargo-packager --locked --version 0.11.8). cargo-packager
  downloads the NSIS toolchain itself on first run (needs network access).
#>
$ErrorActionPreference = 'Stop'

$here = Split-Path -Parent $PSScriptRoot   # $PSScriptRoot = app\scripts  ->  app\
Set-Location $here

$formats = if ($env:FREECELL_PACKAGE_FORMATS) { $env:FREECELL_PACKAGE_FORMATS } else { 'nsis' }
$outDir  = if ($env:FREECELL_PACKAGE_OUT_DIR) { $env:FREECELL_PACKAGE_OUT_DIR } else { Join-Path $here 'target\packages' }

if (-not (Get-Command cargo-packager -ErrorAction SilentlyContinue)) {
    Write-Error "package.ps1: 'cargo-packager' not found on PATH. Install the pinned version:`n    cargo install cargo-packager --locked --version 0.11.8"
    exit 3
}

Write-Host "package.ps1: building freecell (release)…"
cargo build --release -p freecell-app --bin freecell
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

New-Item -ItemType Directory -Force -Path $outDir | Out-Null
Write-Host "package.ps1: packaging formats '$formats' -> $outDir"
cargo packager --release --packages freecell-app --formats $formats --out-dir $outDir @args
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host "`npackage.ps1: done. Packages in $outDir:"
Get-ChildItem $outDir
