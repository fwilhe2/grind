#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# mac-bundle.sh <grind-mac arm64> <grind-mac x86_64> <out>
#
# M10 (doc/macos-shell.md): Grind.app from the two slices, signed ad hoc, in a DMG — and every
# check the milestone names, run on what was built rather than on what was meant:
#
#   * the two slices joined with `lipo`, and both are there (decision 11's universal2);
#   * the Info.plist is `grind-mac --info-plist`'s, and `plutil -lint` passes it;
#   * the icon is ui_mac/data/Grind.icns, packed in the tree by make-icns.py;
#   * `codesign --verify --strict` passes the ad-hoc signature (decision 12);
#   * `otool -L` lists only /System/Library and /usr/lib — the Mac's version of the Windows
#     import-table check: nothing this app needs is missing from a clean Mac;
#   * `vtool` reports a minimum of 15.0 for both slices;
#   * LaunchServices offers Grind for a .csv, a .grind and a .fods once the app is registered,
#     and does **not** make it the default for a .csv when anything else is offered one —
#     offered, never taken. Where nothing else is, LaunchServices' answer is Grind by
#     elimination, which is not taking; a .fods's default is reported for the same reason;
#   * `spctl` **rejects** the ad-hoc build, as expected, and the check says so — the day a
#     Developer ID is added this flips, and it should be made to fail then.
#
# Reports through annotations, as mac-frames.sh does: a job's log needs admin rights to read.

set -euo pipefail

arm="$1"
intel="$2"
out="$3"
mkdir -p "$out"
progress="$out/bundle-progress.txt"
: > "$progress"

say() {
    echo "$*"
    echo "$*" >> "$progress"
}

fail() {
    echo "mac-bundle: $*" >&2
    echo "FAILED: $*" >> "$progress"
    exit 1
}

report() {
    local status=$?
    [ -n "${GITHUB_ACTIONS:-}" ] || return 0
    local level=notice
    [ "$status" -eq 0 ] || level=error
    local body
    body="$(awk 'BEGIN { ORS = "%0A" } { gsub(/%/, "%25"); gsub(/\r/, ""); print }' "$progress")"
    echo "::$level title=mac-bundle on macOS $(sw_vers -productVersion)::$body"
}
trap report EXIT

app="$out/Grind.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

lipo -create "$arm" "$intel" -output "$app/Contents/MacOS/grind-mac"
archs="$(lipo -archs "$app/Contents/MacOS/grind-mac")"
case "$archs" in
    *arm64*x86_64* | *x86_64*arm64*) say "universal: $archs" ;;
    *) fail "the binary is not universal: $archs" ;;
esac

"$arm" --info-plist > "$app/Contents/Info.plist"
plutil -lint "$app/Contents/Info.plist" > /dev/null || fail "plutil rejects the Info.plist"
say "Info.plist: plutil -lint passes"
printf 'APPL????' > "$app/Contents/PkgInfo"
cp ui_mac/data/Grind.icns "$app/Contents/Resources/Grind.icns"

codesign --force --sign - --timestamp=none "$app"
codesign --verify --strict --verbose=2 "$app" 2> "$out/codesign.txt" \
    || fail "codesign --verify --strict: $(cat "$out/codesign.txt")"
say "signed ad hoc, and codesign --verify --strict passes"

# A universal binary is listed once per architecture, each under a header line of its own path,
# so only the indented lines are libraries.
foreign="$(otool -L "$app/Contents/MacOS/grind-mac" | grep '^[[:space:]]' | awk '{ print $1 }' \
    | grep -v -e '^/System/Library/' -e '^/usr/lib/' || true)"
[ -z "$foreign" ] || fail "the binary links outside the system: $foreign"
say "otool -L: /System/Library and /usr/lib only"

for arch in arm64 x86_64; do
    minos="$(vtool -arch "$arch" -show-build "$app/Contents/MacOS/grind-mac" \
        | awk '/minos/ { print $2; exit }')"
    [ "$minos" = "15.0" ] || fail "$arch's minimum is $minos, not 15.0"
done
say "vtool: a minimum of 15.0 for both slices"

# LaunchServices: register the app where it is, then ask what is offered.
lsregister=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
"$lsregister" -f "$app"
printf 'a,b\n1,2\n' > "$out/probe.csv"
cp examples/quote.grind "$out/probe.grind"
cp "$(ls sheet/tests/data/kb/*.fods | head -n 1)" "$out/probe.fods"
for file in "$out/probe.csv" "$out/probe.grind" "$out/probe.fods"; do
    "$app/Contents/MacOS/grind-mac" --handlers "$file" > "$out/handlers.txt"
    cat "$out/handlers.txt"
    grep -q '^offered: .*Grind.app' "$out/handlers.txt" \
        || fail "LaunchServices does not offer Grind for $(basename "$file")"
done
"$app/Contents/MacOS/grind-mac" --handlers "$out/probe.csv" > "$out/handlers.txt"
# Taking the default is a failure only where something else was there to have it. Rank orders
# claims of the *same* specificity: an app that claims CSV itself outranks one that claims only
# `public.text` or `public.data` whatever either rank says, so on a runner with no spreadsheet
# installed Grind is the one exact claim and wins by elimination. The rivals that count are
# the offered applications whose own Info.plist names CSV.
others=0
while IFS= read -r other; do
    plutil -convert xml1 -o - "$other/Contents/Info.plist" 2> /dev/null \
        | grep -qiE 'comma-separated-values|text/csv|<string>csv</string>' \
        && others=$((others + 1))
done < <(sed -n 's/^offered: //p' "$out/handlers.txt" | grep -v 'Grind.app')
if grep -q '^default: .*Grind.app' "$out/handlers.txt" && [ "$others" -gt 0 ]; then
    fail "Grind took .csv's default from $others application(s) claiming CSV, when it only offers"
fi
say "LaunchServices offers Grind for .csv, .grind and .fods, and .csv's default is not taken"
say ".fods opens by default in: $("$app/Contents/MacOS/grind-mac" --handlers "$out/probe.fods" \
    | sed -n 's/^default: //p')"

if spctl --assess --type execute "$app" 2> "$out/spctl.txt"; then
    fail "spctl accepts an ad-hoc build — a Developer ID was added: make this check expect it"
fi
say "spctl rejects the ad-hoc build, as expected: $(head -n 1 "$out/spctl.txt")"

stage="$out/dmg"
rm -rf "$stage"
mkdir -p "$stage"
cp -R "$app" "$stage/"
ln -s /Applications "$stage/Applications"
hdiutil create -quiet -volname Grind -srcfolder "$stage" -ov -format UDZO "$out/Grind.dmg"
say "Grind.dmg: $(du -h "$out/Grind.dmg" | cut -f1)"
