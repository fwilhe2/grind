#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later

# Build the extension into a .vsix (doc/init.md I6):
#
#   editors/vscode/package.sh [out-dir]       # default: target/
#   code --install-extension target/grind-<version>.vsix
#
# Nothing is published: where the extension is offered is the author's decision. The licence
# is copied in from LICENSES/ for the package's sake and never kept here, so the tree holds one
# copy of it.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
out=$(realpath -m "${1:-$root/target}")
mkdir -p "$out"
cd "$here"
npm ci --no-audit --no-fund
cp "$root/LICENSES/AGPL-3.0-or-later.txt" LICENSE.txt
trap 'rm -f "$here/LICENSE.txt"' EXIT
# The bundle carries its dependencies' code, so it carries their licences too.
mkdir -p dist
npm ls --omit=dev --all --parseable | tail -n +2 | sort | while read -r dep; do
    printf '%s\n\n' "$(basename "$dep")"
    cat "$dep"/[Ll][Ii][Cc][Ee][Nn][SsCc][Ee]*
    printf '\n\n'
done > dist/THIRD-PARTY-NOTICES.txt
version=$(node -p "require('./package.json').version")
npx --yes @vscode/vsce@4.0.0 package --no-dependencies --out "$out/grind-$version.vsix"
