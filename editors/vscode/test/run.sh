#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later

# The extension in a real VS Code, headless, against the real `grind lsp` (doc/init.md I6).
#
#   editors/vscode/test/run.sh                     # VS Code from /usr/share/code, grind from target/
#   VSCODE=/path/to/code GRIND=/path/to/grind editors/vscode/test/run.sh
#
# VSCODE is the Electron binary, not the `code` launcher: the launcher hands off and returns at
# once, so its exit status says nothing about the tests. Nothing of yours is used or touched — a
# fresh profile, no other extension, a throwaway folder — and the folder is left *untrusted*,
# because that is how a folder somebody just opened is, and it is where `.grind` used to fail.
# Chromium's headless backend needs no display.
set -euo pipefail

here=$(cd "$(dirname "$0")/.." && pwd)
root=$(cd "$here/../.." && pwd)
VSCODE=${VSCODE:-/usr/share/code/code}
GRIND=${GRIND:-$root/target/debug/grind}

[ -x "$GRIND" ] || (cd "$root" && cargo build -q -p grind-cli)
[ -d "$here/node_modules" ] || (cd "$here" && npm ci --no-audit --no-fund)

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/folder" "$work/profile/User" "$work/extensions"
cp "$root/examples/quote.grind" "$work/folder/"
printf 'let s = sheet("A");\ns.\n' > "$work/folder/model.rhai"
printf 'let s = sheet("A");\ns.push(row(1, 2)\n' > "$work/folder/broken.rhai"
# In the *user* settings, which an untrusted folder cannot override — the folder's own would be
# ignored, by design (package.json's restrictedConfigurations).
printf '{ "grind.path": "%s" }\n' "$GRIND" > "$work/profile/User/settings.json"

status=0
"$VSCODE" --ozone-platform=headless --disable-gpu \
    --user-data-dir="$work/profile" --extensions-dir="$work/extensions" \
    --skip-welcome --skip-release-notes \
    --extensionDevelopmentPath="$here" --extensionTestsPath="$here/test/suite.js" \
    "$work/folder" > "$work/log" 2>&1 || status=$?
# VS Code's own chatter is long; the runner's lines and any failure are what matter.
grep -E '^(ok|[0-9]+ checks|Error|AssertionError|    at )' "$work/log" || cat "$work/log"
exit "$status"
