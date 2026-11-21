#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# mac-pkg.sh <Grind.app> <version> <out.pkg>
#
# The installer: a flat package whose only domain is the current user's home, so Installer
# puts Grind.app in ~/Applications without asking for an administrator's password, and
# Spotlight and Launchpad find it there. Then the package is installed on this runner the same
# way, and what landed is checked: the bundle, its icon, its signature, and that it starts.
#
# The package is unsigned: Installer asks once, through Privacy & Security, before it runs it.

set -euo pipefail

app="$1"
version="$2"
out="$3"
id="io.github.fwilhe2.Grind"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

mkdir -p "$work/root"
cp -R "$app" "$work/root/"
pkgbuild --analyze --root "$work/root" "$work/component.plist"
# Not relocatable: Installer would otherwise "upgrade" whichever Grind.app LaunchServices
# knows of, wherever that is. Not version-checked: the package installs what it carries.
plutil -replace 0.BundleIsRelocatable -bool NO "$work/component.plist"
plutil -replace 0.BundleIsVersionChecked -bool NO "$work/component.plist"

mkdir -p "$work/packages"
pkgbuild --root "$work/root" --component-plist "$work/component.plist" \
    --install-location /Applications --identifier "$id" --version "$version" \
    "$work/packages/Grind-component.pkg"

# In the current-user-home domain, every install location is under the user's home.
cat > "$work/distribution.xml" <<EOF
<?xml version="1.0" encoding="utf-8"?>
<installer-gui-script minSpecVersion="2">
    <title>Grind</title>
    <options customize="never" require-scripts="false" hostArchitectures="arm64,x86_64"/>
    <domains enable_anywhere="false" enable_currentUserHome="true" enable_localSystem="false"/>
    <volume-check>
        <allowed-os-versions><os-version min="15.0"/></allowed-os-versions>
    </volume-check>
    <choices-outline><line choice="default"><line choice="$id"/></line></choices-outline>
    <choice id="default"/>
    <choice id="$id" visible="false"><pkg-ref id="$id"/></choice>
    <pkg-ref id="$id" version="$version" onConclusion="none">Grind-component.pkg</pkg-ref>
</installer-gui-script>
EOF
productbuild --distribution "$work/distribution.xml" --package-path "$work/packages" "$out"
echo "$(basename "$out"): $(du -h "$out" | cut -f1)"

# What a person double-clicking it gets, without the clicking: no sudo, the home domain.
installed="$HOME/Applications/Grind.app"
rm -rf "$installed"
installer -pkg "$out" -target CurrentUserHomeDirectory
[ -x "$installed/Contents/MacOS/grind-mac" ] || { echo "not installed at $installed" >&2; exit 1; }
icon="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIconFile' "$installed/Contents/Info.plist")"
[ -s "$installed/Contents/Resources/$icon.icns" ] \
    || { echo "the installed app has no $icon.icns" >&2; exit 1; }
codesign --verify --strict "$installed"
"$installed/Contents/MacOS/grind-mac" --version
echo "installed in ~/Applications with its icon, and it starts"
rm -rf "$installed"
