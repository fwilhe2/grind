#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Build a shell and run it on the sample document — the "see it working" loop.
#
#   scripts/run.sh sheet-gtk|text-gtk|tui|web [document]
#
# `sheet-gtk` is the spreadsheet's window and `text-gtk` the word processor's — two
# binaries because a `.desktop` file's MimeType= is per application (doc/suite.md).
# `tui` and `web` are one shell each and open whichever document type they are given.
#
# With no document it builds one with `examples/sample-sheet.sh` — or `examples/sample-text.sh`
# for the word processor — which uses every feature this build supports, so whatever was
# just changed is on screen somewhere. The
# document goes to $GRIND_DEMO (default /tmp/grind-demo) and is rebuilt every run:
# it is disposable, and a stale one is worse than no demo at all.
#
# The web shell has no filesystem, so its copy is served next to the page and opened
# with `?doc=` — the browser's own way of being told which document to load.

set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
shell="${1:-sheet-gtk}"
demo="${GRIND_DEMO:-/tmp/grind-demo}"
port="${PORT:-8000}"

document="${2:-}"
if [ -z "$document" ]; then
    cargo build -p grind-cli
    # Quietly: the sample script narrates every feature it uses, which is worth
    # reading when it is the subject and noise when it is the fixture.
    case "$shell" in
        text-gtk)
            GRIND="$root/target/debug/grind" "$root/examples/sample-text.sh" "$demo" >/dev/null
            document="$demo/sample.fodt"
            ;;
        *)
            GRIND="$root/target/debug/grind" "$root/examples/sample-sheet.sh" "$demo" >/dev/null
            document="$demo/sample.fods"
            ;;
    esac
fi

case "$shell" in
    sheet-gtk)
        exec cargo run -p grind-sheet-gtk -- "$document"
        ;;
    text-gtk)
        exec cargo run -p grind-text-gtk -- "$document"
        ;;
    tui)
        exec cargo run -p grind-tui -- "$document"
        ;;
    web)
        "$root/ui_web/build.sh" debug
        cp "$document" "$root/ui_web/dist/"
        echo "  http://localhost:$port/?doc=$(basename "$document")"
        exec python3 -m http.server --directory "$root/ui_web/dist" "$port"
        ;;
    *)
        echo "usage: $(basename "$0") sheet-gtk|text-gtk|tui|web [document]" >&2
        exit 2
        ;;
esac
