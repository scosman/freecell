#!/usr/bin/env bash
# ---------------------------------------------------------------------------------
# Build, SIGN, and NOTARIZE the macOS FreeCell release artifacts.
#
# This is the macOS *release* path. The plain `scripts/package.sh` produces UNSIGNED
# artifacts (that is what CI uploads); this script wraps it to produce the real,
# distributable pair:
#
#   target/packages/FreeCell.app        signed (Developer ID) + notarized + stapled
#   target/packages/FreeCell <ver>.dmg  signed (Developer ID) + notarized + stapled
#
# Flow:
#   1. preflight every tool/credential BEFORE the slow build, so failures cost seconds
#   2. package the .app ONLY (never the unsigned cargo-packager .dmg — see below)
#   3. prompt you to pick a "Developer ID Application" identity
#   4. codesign nested Mach-O inner-out, then the bundle (hardened runtime + timestamp)
#   5. notarize + staple the .app
#   6. build the .dmg with create-dmg (which signs it)
#   7. notarize + staple the .dmg
#   8. verify for real: stapler validate + spctl must say "Notarized Developer ID"
#
# Why the .app only in step 2: `package.sh`'s macOS default is `app,dmg`, and that .dmg is
# built from the *unsigned* bundle. Producing it here would leave an unsigned .dmg sitting
# next to the signed one in the same directory — a genuinely dangerous mixup at release
# time. So we force `FREECELL_PACKAGE_FORMATS=app` and let create-dmg make the only .dmg.
#
# Usage:
#   scripts/sign_macos.sh                 # full signed + notarized release build
#   scripts/sign_macos.sh --verbose       # extra args pass through to package.sh
#
# Requires (all checked in preflight, each with the exact fix command):
#   - macOS with Xcode command line tools (codesign, notarytool, stapler, ditto, spctl)
#   - a "Developer ID Application" certificate in your login keychain
#     (paid Apple Developer Program; Apple Development certs CANNOT be notarized)
#   - create-dmg:  npm install --global create-dmg      (Node 20+)
#   - a stored notarytool credential profile (one time):
#       xcrun notarytool store-credentials freecell-notary \
#           --apple-id <you@example.com> --team-id <TEAMID> --password <app-specific-password>
#   - network access: the secure timestamp server and two notarization round trips.
#     A round trip can be anywhere from seconds to many minutes — near-instant usually means
#     Apple had already scanned that exact CDHash and re-used the result, which is fine.
#     Proof that notarization really happened is the staple succeeding: `stapler` fetches the
#     ticket from Apple and cannot attach one that was never issued.
#
# Env overrides:
#   FREECELL_NOTARY_PROFILE   notarytool keychain profile name (default: freecell-notary)
#   FREECELL_ENTITLEMENTS     path to an entitlements .plist (default: none — a pure Rust
#                             GPUI/Metal app should not need any under hardened runtime)
#   FREECELL_PACKAGE_OUT_DIR  package output dir (default: app/target/packages)
#
# See ../PACKAGING.md for the full story, including how to verify a build the way a real
# downloader experiences it (quarantined).
# ---------------------------------------------------------------------------------
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"   # app/
cd "$here"

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
    # Print the header comment block (line 2 until the first non-comment line), minus the
    # ---- rules and the leading "# ".
    awk 'NR == 1 { next } /^#/ { sub(/^# ?/, ""); if ($0 !~ /^-+$/) print; next } { exit }' \
        "${BASH_SOURCE[0]}"
    exit 0
fi

notary_profile="${FREECELL_NOTARY_PROFILE:-freecell-notary}"
out_dir="${FREECELL_PACKAGE_OUT_DIR:-$here/target/packages}"
app_path="$out_dir/FreeCell.app"

die() { echo "sign_macos.sh: $*" >&2; exit 1; }
step() { echo; echo "==> $*"; }

# =================================================================================
# 1. Preflight — fail in seconds, not after a ten-minute release build.
# =================================================================================
step "Preflight"

[[ "$(uname -s)" == "Darwin" ]] || die "macOS only (this is $(uname -s))."

for tool in codesign ditto security spctl xcrun; do
    command -v "$tool" >/dev/null 2>&1 || die "'$tool' not found. Install the Xcode command line tools: xcode-select --install"
done
xcrun --find notarytool >/dev/null 2>&1 || die "'notarytool' not available. Update the Xcode command line tools (needs Xcode 13+)."
xcrun --find stapler >/dev/null 2>&1 || die "'stapler' not available. Update the Xcode command line tools."

if ! command -v create-dmg >/dev/null 2>&1; then
    die "'create-dmg' not found on PATH. Install it:
    npm install --global create-dmg          # needs Node 20+"
fi

# Confirm the notary credential profile exists before building. `notarytool history` is the
# cheapest call that actually exercises the stored credentials.
if ! xcrun notarytool history --keychain-profile "$notary_profile" >/dev/null 2>&1; then
    die "notarytool credential profile '$notary_profile' is missing or invalid. Store it once:
    xcrun notarytool store-credentials $notary_profile \\
        --apple-id <you@example.com> --team-id <TEAMID> --password <app-specific-password>
(Create the app-specific password at https://account.apple.com -> Sign-In and Security.)
Override the profile name with FREECELL_NOTARY_PROFILE."
fi

entitlements="${FREECELL_ENTITLEMENTS:-}"
if [[ -n "$entitlements" ]]; then
    [[ -f "$entitlements" ]] || die "FREECELL_ENTITLEMENTS='$entitlements' does not exist."
    echo "    entitlements: $entitlements"
fi

# Any .dmg already sitting in the output dir is NOT ours (we only ever create one, below).
# Remember them so the final summary can warn instead of leaving an unsigned lookalike
# next to the real artifact.
pre_existing_dmgs=()
while IFS= read -r -d '' f; do pre_existing_dmgs+=("$f"); done \
    < <(find "$out_dir" -maxdepth 1 -type f -name '*.dmg' -print0 2>/dev/null || true)

echo "    tools ok; notary profile '$notary_profile' ok"

# =================================================================================
# 2. Build + package the .app bundle ONLY.
# =================================================================================
step "Packaging the .app bundle (unsigned)"
FREECELL_PACKAGE_FORMATS=app scripts/package.sh "$@"

if [[ ! -d "$app_path" ]]; then
    # product-name in Cargo.toml decides the bundle name; fall back to whatever .app landed.
    found_app="$(find "$out_dir" -maxdepth 1 -type d -name '*.app' | head -1 || true)"
    [[ -n "$found_app" ]] || die "no .app bundle found in $out_dir after packaging"
    app_path="$found_app"
fi
echo "    bundle: $app_path"

# =================================================================================
# 3. Pick a Developer ID Application identity.
# =================================================================================
step "Select a signing identity"

# `security find-identity -v -p codesigning` lists only identities whose private key is
# present and whose cert is valid, one per line:
#   1) A1B2C3D4... "Developer ID Application: Some Name (TEAM123456)"
identity_lines=()
while IFS= read -r line; do
    [[ -n "$line" ]] && identity_lines+=("$line")
done < <(security find-identity -v -p codesigning | grep 'Developer ID Application' || true)

if [[ ${#identity_lines[@]} -eq 0 ]]; then
    die "no \"Developer ID Application\" identity found in your keychains.

Notarization REQUIRES a Developer ID Application certificate (paid Apple Developer
Program). An \"Apple Development\" cert cannot be notarized or distributed.
Create/download one at https://developer.apple.com/account/resources/certificates and
double-click it to install into your login keychain, then re-run.

All valid codesigning identities currently visible:
$(security find-identity -v -p codesigning)"
fi

echo
for i in "${!identity_lines[@]}"; do
    printf '  %2d) %s\n' "$((i + 1))" "$(printf '%s' "${identity_lines[$i]}" | sed 's/^[[:space:]]*[0-9]*)[[:space:]]*//')"
done
echo

choice=""
while true; do
    # `read` fails at EOF; without this guard `set -e` would abort with no explanation when
    # the script is run with stdin closed or piped.
    read -r -p "Identity to sign with [1-${#identity_lines[@]}]: " choice \
        || die "no input available — this script is interactive and needs a terminal."
    [[ "$choice" =~ ^[0-9]+$ ]] && (( choice >= 1 && choice <= ${#identity_lines[@]} )) && break
    echo "  Enter a number between 1 and ${#identity_lines[@]}."
done

selected="${identity_lines[$((choice - 1))]}"
# Sign with the SHA-1 hash, not the common name: names are frequently duplicated across
# expired/renewed certs, and a hash is unambiguous. create-dmg passes it straight to
# `codesign --sign`, which accepts either form.
identity_hash="$(printf '%s' "$selected" | awk '{print $2}')"
identity_name="$(printf '%s' "$selected" | sed -n 's/.*"\(.*\)".*/\1/p')"
# "Developer ID Application: Some Name (TEAM123456)" -> TEAM123456
team_id="$(printf '%s' "$identity_name" | sed -n 's/.*(\([A-Z0-9]*\))$/\1/p')"

[[ -n "$identity_hash" ]] || die "could not parse the identity hash from: $selected"
[[ -n "$team_id" ]] || die "could not parse a Team ID from identity name: $identity_name"

echo "    signing with: $identity_name"
echo "    sha-1:        $identity_hash"
echo "    team id:      $team_id"

# =================================================================================
# 4. Codesign the bundle.
#
#    --force     REQUIRED: every arm64 Mach-O carries an ad-hoc signature applied by the
#                linker, so signing without --force fails on Apple silicon.
#    --timestamp REQUIRED for notarization (needs network). An untimestamped signature is
#                rejected by the notary service.
#    --options runtime  enables the hardened runtime, also required for notarization.
#
#    NOT --deep: Apple deprecated it and it silently mis-signs nested code. We sign nested
#    Mach-O inner-out ourselves, then the bundle last.
# =================================================================================
step "Codesigning $app_path"

codesign_args=(--force --timestamp --options runtime --sign "$identity_hash")
[[ -n "$entitlements" ]] && codesign_args+=(--entitlements "$entitlements")

main_exe_name="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$app_path/Contents/Info.plist" 2>/dev/null || echo freecell)"
main_exe="$app_path/Contents/MacOS/$main_exe_name"

# Nested code first (inner-out). For a pure-Rust bundle this usually finds nothing; the
# loop is cheap insurance for the day a dylib or helper binary shows up.
nested_count=0
while IFS= read -r -d '' f; do
    [[ "$f" == "$main_exe" ]] && continue
    # Only Mach-O files are code; skip resources that merely have the execute bit set.
    file -b "$f" | grep -q 'Mach-O' || continue
    echo "    nested: ${f#"$app_path/"}"
    codesign "${codesign_args[@]}" "$f"
    nested_count=$((nested_count + 1))
done < <(find "$app_path/Contents" -type f \( -name '*.dylib' -o -name '*.so' -o -perm -111 \) -print0)

while IFS= read -r -d '' d; do
    echo "    nested framework: ${d#"$app_path/"}"
    codesign "${codesign_args[@]}" "$d"
    nested_count=$((nested_count + 1))
done < <(find "$app_path/Contents" -type d -name '*.framework' -print0)

echo "    nested code signed: $nested_count"

codesign "${codesign_args[@]}" "$app_path"

# =================================================================================
# Signature assertion.
#
# `codesign --verify --strict` is NOT sufficient on its own: an ad-hoc / linker-signed
# binary — which is what EVERY arm64 Mach-O is before we touch it — verifies perfectly
# cleanly. Only the `codesign -d` display output distinguishes "has a valid signature" from
# "is signed by us, with the flags notarization needs". An unsigned bundle looks like this,
# and every one of these lines is a failure we assert against below:
#
#   Identifier=freecell-b6e2e888c757f89d          <- linker-generated, not the bundle id
#   CodeDirectory ... flags=0x20002(adhoc,linker-signed)
#   Signature=adhoc
#   Info.plist=not bound
#   TeamIdentifier=not set
#   Sealed Resources=none
#
# Reads $team_id and $bundle_id from the enclosing script.
# =================================================================================
assert_developer_id_signed() {
    local target="$1" kind="$2"
    local info

    # codesign -d writes its report to stderr.
    info="$(codesign --display --verbose=2 "$target" 2>&1)" \
        || die "codesign could not read a signature from $target"
    printf '%s\n' "$info" | sed 's/^/      /'

    if printf '%s\n' "$info" | grep -qE 'Signature=adhoc|adhoc,linker-signed|linker-signed'; then
        die "$target is AD-HOC signed, not Developer ID signed — 'codesign --force' did not take.
(An ad-hoc signature passes 'codesign --verify' cleanly, which is why we check the display
output. See the report above.)"
    fi
    if printf '%s\n' "$info" | grep -q 'Signature=none'; then
        die "$target is NOT SIGNED at all. See the report above."
    fi
    if ! printf '%s\n' "$info" | grep -q 'Authority=Developer ID Application'; then
        die "$target has no 'Developer ID Application' authority — it is signed by the wrong
kind of certificate and cannot be notarized. See the report above."
    fi
    if ! printf '%s\n' "$info" | grep -q "TeamIdentifier=$team_id"; then
        die "$target does not carry TeamIdentifier=$team_id (an unsigned bundle reports
'TeamIdentifier=not set'). See the report above."
    fi
    # With --timestamp, codesign reports 'Timestamp=<date>'. WITHOUT a secure timestamp it
    # reports 'Signed Time=<date>' instead, and the notary service rejects that.
    if ! printf '%s\n' "$info" | grep -q '^Timestamp='; then
        die "$target has no secure timestamp (look for 'Signed Time=' instead of
'Timestamp=' in the report above). The Apple notary service rejects untimestamped
signatures; this usually means the timestamp server was unreachable."
    fi

    # Bundle-only checks. A .dmg signature has no hardened runtime, no Info.plist, and no
    # sealed resources, so these apply to the .app only.
    if [[ "$kind" == "app" ]]; then
        if ! printf '%s\n' "$info" | grep -qE 'flags=[^(]*\([^)]*runtime'; then
            die "$target is signed WITHOUT the hardened runtime (no 'runtime' in the
CodeDirectory flags above). Notarization requires it."
        fi
        if ! printf '%s\n' "$info" | grep -q "^Identifier=$bundle_id"; then
            die "$target reports the wrong code signing identifier — expected
'Identifier=$bundle_id' (an unsigned bundle shows a linker-generated id like
'freecell-b6e2e888c757f89d'). See the report above."
        fi
        # Asserted as negatives on purpose. codesign spells these two inconsistently — the
        # unsigned forms are 'Sealed Resources=none' / 'Info.plist=not bound', while the
        # signed forms drop the '=' ('Sealed Resources version=2 rules=…', 'Info.plist
        # entries=24'). Matching the failure text is exact; matching the success text would
        # be guesswork.
        if printf '%s\n' "$info" | grep -q 'Sealed Resources=none'; then
            die "$target has no sealed resources — the BUNDLE was not signed, only its
executable. See the report above."
        fi
        if printf '%s\n' "$info" | grep -q 'Info.plist=not bound'; then
            die "$target's Info.plist is not bound into the signature — the BUNDLE was not
signed, only its executable. See the report above."
        fi
    fi

    echo "      ^ ok: Developer ID, team $team_id, timestamped$([[ "$kind" == "app" ]] && echo ", hardened runtime")"
}

bundle_id="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$app_path/Contents/Info.plist" 2>/dev/null || echo '')"
[[ -n "$bundle_id" ]] || die "could not read CFBundleIdentifier from $app_path/Contents/Info.plist"

echo "    verifying the signature…"
codesign --verify --deep --strict --verbose=2 "$app_path"
assert_developer_id_signed "$app_path" app

# =================================================================================
# Notarization helper — SUBMIT ONLY. Stapling is the caller's job, because what you submit
# and what you staple are not always the same file: an .app is submitted as a .zip but the
# ticket is stapled to the BUNDLE (`stapler` refuses a zip outright — "Stapler is incapable
# of working with ZIP archive files"), while a .dmg is both submitted and stapled directly.
#
# `notarytool submit --wait` returns a terse status; when it is "Invalid" the summary tells
# you nothing useful, so we always pull the full log for the submission — the real reason is
# almost always a missing hardened-runtime flag or an unsigned nested binary.
# =================================================================================
notarize_submit() {
    local target="$1"

    # Timing varies wildly: seconds when Apple has already scanned this exact CDHash (tickets
    # are keyed by code directory hash, which excludes the timestamped signature blob, so a
    # rebuild of identical code re-uses the earlier scan), up to many minutes otherwise.
    echo "    submitting $(basename "$target") to the Apple notary service (--wait; anywhere from seconds to many minutes)…"

    local json status submission_id
    # Do not let a non-zero exit abort before we can fetch the log.
    json="$(xcrun notarytool submit "$target" \
        --keychain-profile "$notary_profile" \
        --wait --output-format json 2>&1)" || true

    submission_id="$(printf '%s' "$json" | grep -o '"id"[[:space:]]*:[[:space:]]*"[^"]*"' | head -1 | sed 's/.*"\([^"]*\)"$/\1/')"
    status="$(printf '%s' "$json" | grep -o '"status"[[:space:]]*:[[:space:]]*"[^"]*"' | head -1 | sed 's/.*"\([^"]*\)"$/\1/')"

    if [[ "$status" != "Accepted" ]]; then
        echo "$json" >&2
        if [[ -n "$submission_id" ]]; then
            echo >&2
            echo "--- notary log for submission $submission_id ---" >&2
            xcrun notarytool log "$submission_id" --keychain-profile "$notary_profile" >&2 2>&1 || true
        fi
        die "notarization failed for $(basename "$target") (status: ${status:-unknown})."
    fi

    echo "    accepted (submission $submission_id)"
}

# =================================================================================
# 5. Notarize + staple the .app.
#
#    Stapling the app as well as the dmg is what Apple's docs describe: the app then carries
#    its own ticket, so it launches even offline after being dragged out of the disk image.
#
#    The notary service takes an ARCHIVE, not a bundle, and that archive MUST be made with
#    `ditto -c -k --keepParent`: `zip` mangles the symlinks and extended attributes inside a
#    bundle and the submission is rejected.
#
#    But the TICKET goes on the bundle, not on the archive — `stapler` rejects a zip
#    outright ("Stapler is incapable of working with ZIP archive files"). So: submit the
#    zip, staple the .app, throw the zip away.
# =================================================================================
step "Notarizing the .app"

zip_path="$out_dir/FreeCell-notarize.zip"
rm -f "$zip_path"
/usr/bin/ditto -c -k --keepParent "$app_path" "$zip_path"
notarize_submit "$zip_path"
echo "    stapling the ticket to the bundle…"
xcrun stapler staple "$app_path"
rm -f "$zip_path"

# =================================================================================
# 6. Build the .dmg (create-dmg signs it with the same identity).
# =================================================================================
step "Building the .dmg"
create-dmg --overwrite --identity="$identity_hash" "$app_path" "$out_dir"

# create-dmg names the output "<CFBundleDisplayName or CFBundleName> <version>.dmg" — note
# the space. Derive that name rather than guessing, falling back to the newest .dmg.
app_display_name="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleDisplayName' "$app_path/Contents/Info.plist" 2>/dev/null \
    || /usr/libexec/PlistBuddy -c 'Print :CFBundleName' "$app_path/Contents/Info.plist" 2>/dev/null \
    || echo FreeCell)"
app_version="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$app_path/Contents/Info.plist" 2>/dev/null || echo '')"
dmg_path="$out_dir/$app_display_name $app_version.dmg"

if [[ ! -f "$dmg_path" ]]; then
    dmg_path="$(find "$out_dir" -maxdepth 1 -type f -name '*.dmg' -print0 2>/dev/null \
        | xargs -0 ls -t 2>/dev/null | head -1 || true)"
fi
[[ -f "$dmg_path" ]] || die "could not locate the .dmg create-dmg produced in $out_dir"
echo "    built: $dmg_path"

# create-dmg treats a signing failure as a WARNING and still exits successfully (that is
# deliberate, so CI without a cert keeps working). Left unchecked that would hand us an
# unsigned .dmg. Assert the signature ourselves.
echo "    verifying the .dmg signature…"
codesign --verify --strict --verbose=2 "$dmg_path" \
    || die "create-dmg produced an UNSIGNED (or badly signed) .dmg — check its output above."
assert_developer_id_signed "$dmg_path" dmg

# =================================================================================
# 7. Notarize + staple the .dmg. Unlike the .app, a disk image is submitted and stapled
#    directly — no archive step.
# =================================================================================
step "Notarizing the .dmg"
notarize_submit "$dmg_path"
echo "    stapling the ticket to the .dmg…"
xcrun stapler staple "$dmg_path"

# =================================================================================
# 8. Verify for real.
#
#    This is the assertion that the whole chain worked. Before this script, spctl on the
#    bundle reports "rejected". It must now report "accepted ... source=Notarized Developer
#    ID" — anything else is a failed release build.
# =================================================================================
step "Verification"

# Stapling writes the notarization ticket into the bundle. That is designed not to disturb
# the seal, but re-verify rather than assume.
codesign --verify --deep --strict --verbose=2 "$app_path"

xcrun stapler validate "$app_path"
xcrun stapler validate "$dmg_path"

spctl_out="$(spctl --assess --type exec --verbose=4 "$app_path" 2>&1 || true)"
echo "$spctl_out"
printf '%s' "$spctl_out" | grep -q 'accepted' \
    || die "spctl REJECTED $app_path — the app is not distributable. See the output above."
printf '%s' "$spctl_out" | grep -q 'Notarized Developer ID' \
    || die "spctl accepted $app_path but not as \"Notarized Developer ID\". See the output above."

echo
echo "==> Done. Signed + notarized + stapled:"
echo "      $app_path"
echo "      $dmg_path"
echo
echo "    Signed with: $identity_name"

if [[ ${#pre_existing_dmgs[@]} -gt 0 ]]; then
    echo
    echo "    WARNING: these .dmg files were already in $out_dir and are NOT signed or"
    echo "    notarized by this run (most likely leftovers from a plain package.sh run)."
    echo "    Do not ship them:"
    for f in "${pre_existing_dmgs[@]}"; do
        [[ "$f" == "$dmg_path" ]] && continue
        echo "      $f"
    done
fi

echo
echo "    A locally-built app is never quarantined, so it would launch even unsigned. To"
echo "    test what a real downloader gets, copy the .dmg elsewhere and quarantine it:"
echo "      xattr -w com.apple.quarantine '0081;00000000;Safari;' \"$dmg_path\""
