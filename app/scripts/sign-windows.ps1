<#
  Sign ONE file with Azure Trusted Signing (a.k.a. Azure Artifact Signing) — but only if the
  signing environment is configured. If the required env vars are absent this is a NO-OP that
  exits 0, so unsigned local/CI builds keep working unchanged. That is what lets package.ps1
  call it unconditionally for both the app binary and the installer: signing is strictly
  opt-in, never a build gate.

  Why here and not cargo-packager's `sign_command`: signing is driven around cargo-packager
  (in package.ps1) rather than by a `sign_command` in Cargo.toml. cargo-packager copies the
  core `freecell.exe` into the NSIS installer verbatim, so signing the binary BEFORE packaging
  embeds a signature that travels with the copy; we then sign the produced `*-setup.exe`. This
  keeps one packaging entry point for local + CI and means an unconfigured environment simply
  produces an unsigned build instead of failing.

  Signer: `trusted-signing-cli` by default. Override with $env:FREECELL_WINDOWS_SIGN_TOOL —
  e.g. `artifact-signing-cli`, the renamed successor crate (same author, same -e/-a/-c flags),
  since Azure renamed "Trusted Signing" to "Artifact Signing". Both authenticate via the
  standard AZURE_* credential env vars below (Entra client-secret credential).

  Required to actually sign (ALL must be set, else signing is SKIPPED):
    AZURE_TENANT_ID / AZURE_CLIENT_ID / AZURE_CLIENT_SECRET   Entra app registration creds
    AZURE_TRUSTED_SIGNING_ACCOUNT                             Trusted Signing account name
    AZURE_TRUSTED_SIGNING_PROFILE                             certificate profile name
  Optional:
    AZURE_TRUSTED_SIGNING_ENDPOINT   default https://eus.codesigning.azure.net/
    FREECELL_WINDOWS_SIGN_TOOL       default trusted-signing-cli

  Usage:  scripts\sign-windows.ps1 -Path C:\path\to\file.exe
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string] $Path
)

$ErrorActionPreference = 'Stop'

$account     = $env:AZURE_TRUSTED_SIGNING_ACCOUNT
$certProfile = $env:AZURE_TRUSTED_SIGNING_PROFILE   # not $profile: that shadows PS's automatic $PROFILE
$endpoint    = if ($env:AZURE_TRUSTED_SIGNING_ENDPOINT) { $env:AZURE_TRUSTED_SIGNING_ENDPOINT } else { 'https://eus.codesigning.azure.net/' }
$tool        = if ($env:FREECELL_WINDOWS_SIGN_TOOL) { $env:FREECELL_WINDOWS_SIGN_TOOL } else { 'trusted-signing-cli' }

# Signing is OPTIONAL. Decide from whether the environment is fully configured.
$required = [ordered]@{
    'AZURE_TENANT_ID'               = $env:AZURE_TENANT_ID
    'AZURE_CLIENT_ID'               = $env:AZURE_CLIENT_ID
    'AZURE_CLIENT_SECRET'           = $env:AZURE_CLIENT_SECRET
    'AZURE_TRUSTED_SIGNING_ACCOUNT' = $account
    'AZURE_TRUSTED_SIGNING_PROFILE' = $certProfile
}
$missing = @($required.GetEnumerator() |
    Where-Object { [string]::IsNullOrWhiteSpace($_.Value) } |
    ForEach-Object { $_.Key })

if ($missing.Count -eq $required.Count) {
    # Nothing configured at all -> plain unsigned build. Quiet, expected path.
    Write-Host "sign-windows.ps1: Azure Trusted Signing not configured; leaving unsigned: $Path"
    return
}
if ($missing.Count -gt 0) {
    # Partially configured -> almost certainly a mistake. Still don't fail the build (signing is
    # opt-in), but shout so the misconfiguration is obvious instead of silently shipping unsigned.
    Write-Warning ("sign-windows.ps1: Azure Trusted Signing is PARTIALLY configured; skipping signing of '$Path'. Missing: " + ($missing -join ', '))
    return
}

if (-not (Test-Path -LiteralPath $Path)) {
    throw "sign-windows.ps1: file to sign not found: $Path"
}
if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
    throw "sign-windows.ps1: signing is configured but '$tool' is not on PATH. Install it (e.g. 'cargo install trusted-signing-cli --locked --version 0.11.0') or set FREECELL_WINDOWS_SIGN_TOOL."
}

Write-Host "sign-windows.ps1: signing with $tool (account '$account', profile '$certProfile'): $Path"
& $tool -e $endpoint -a $account -c $certProfile $Path
if ($LASTEXITCODE -ne 0) {
    throw "sign-windows.ps1: $tool failed (exit $LASTEXITCODE) signing: $Path"
}
Write-Host "sign-windows.ps1: signed OK: $Path"
