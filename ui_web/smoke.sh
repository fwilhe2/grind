#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Run the browser-boundary smoke test (ui_web/smoke.js).
#
# Builds the module for node rather than the web — the same `.wasm`, different glue
# — and drives it against index.html in jsdom. No browser involved, so this runs
# anywhere, which is what makes it usable as a CI gate.
#
#   ui_web/smoke.sh [debug|release]

set -euo pipefail

profile="${1:-release}"
root="$(cd "$(dirname "$0")/.." && pwd)"
work="$root/ui_web/.smoke"
module="$root/target/wasm32-unknown-unknown/$profile/grind_web.wasm"

if [ ! -f "$module" ]; then
    echo "missing $module — run: ui_web/build.sh $profile" >&2
    exit 1
fi

mkdir -p "$work"
wasm-bindgen --target nodejs --no-typescript --out-dir "$work" --out-name grind_web "$module"

# Pinned, and installed next to the shell rather than in the repo root: this is
# test scaffolding, not something the app ships.
if [ ! -d "$root/ui_web/node_modules/jsdom" ]; then
    npm install --silent --no-save --no-fund --no-audit --prefix "$root/ui_web" jsdom@24
fi

node "$root/ui_web/smoke.js"
