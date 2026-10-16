#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# A series of letters as files: one PDF per recipient.
#
#   GRIND=target/debug/grind examples/letters/pdfs.sh nebenkosten /tmp/letters
#   GRIND=target/debug/grind examples/letters/pdfs.sh mahnung     /tmp/letters
#
# `grind build` makes one document per run, so the script is run once per recipient, each time
# with a `select.json` beside the data saying which letter to write (`{ "only": "w3a" }`). Every
# run still does the whole calculation and its check; only the output differs. The recipients
# are the `"id"`s in `<name>.json`; a script that also writes a page for the office says so by
# mentioning `uebersicht`. The PDFs are the same bytes for the same data (`--bundled-fonts`: the
# machine's fonts are not an input).
set -euo pipefail

GRIND="${GRIND:-grind}"
here="$(cd "$(dirname "$0")" && pwd)"
name="${1:?usage: pdfs.sh SCRIPT_NAME OUT_DIR   (e.g. nebenkosten)}"
out="${2:?usage: pdfs.sh SCRIPT_NAME OUT_DIR}"
mkdir -p "$out"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cp "$here/$name.json" "$here/brief-din5008.fodt" "$work/"

ids=$(grep -o '"id": "[^"]*"' "$here/$name.json" | cut -d'"' -f4)
if grep -q uebersicht "$here/$name.rhai"; then ids="$ids uebersicht"; fi
for id in $ids; do
    printf '{ "only": "%s" }\n' "$id" > "$work/select.json"
    "$GRIND" build "$here/$name.rhai" --data "$work" -o "$work/$id.fodt" >/dev/null
    "$GRIND" text export-pdf "$work/$id.fodt" "$out/$name-$id.pdf" --bundled-fonts \
        --title "$name $id" >/dev/null
    echo "$out/$name-$id.pdf"
done
