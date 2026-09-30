#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# mac-frames.sh <grind-mac> <grind> <out>
#
# M2's exit criterion (doc/macos-shell.md), run by `artifacts.yml`'s `macos` job:
#
#   1. every R7 document and the sample spreadsheet is drawn with `--render-to` **twice in each
#      appearance**, and the two frames must be the same bytes — decision 9's reproducibility,
#      with the PNG signature checked so two empty files cannot agree their way past it — and
#      the dark frame must differ from the light one, or `--dark` reached nothing;
#   2. every one of them is **opened in a real window through a drive** that waits for it to
#      draw and snaps it, so a document that cannot be opened by the application, rather than
#      only by the renderer, fails here.
#
# Every step is bounded: a GUI process that manages to put an alert up on a runner would
# otherwise wait for a click until the job's own limit.
#
# Written for the bash 3.2 macOS ships as /bin/bash: no empty arrays under `set -u`, no
# associative arrays.

set -euo pipefail

mac="$1"
grind="$2"
out="$3"
mkdir -p "$out"

fail() {
    echo "mac-frames: $*" >&2
    exit 1
}

# Run a command with a time limit, since `timeout` is not part of macOS.
bounded() {
    local seconds="$1"
    shift
    "$@" &
    local pid=$!
    ( sleep "$seconds" && kill -9 "$pid" 2>/dev/null ) &
    local watchdog=$!
    local status=0
    wait "$pid" || status=$?
    kill "$watchdog" 2>/dev/null || true
    return "$status"
}

is_png() {
    [ "$(head -c 8 "$1" | xxd -p)" = "89504e470d0a1a0a" ]
}

# The sample spreadsheet, built through the CLI out of every feature this build has.
GRIND="$grind" examples/sample-sheet.sh "$out/sample" > /dev/null

drive="$out/open.drive"
cat > "$drive" <<'EOF'
# Let the window draw, then keep a picture of it.
wait 1
snap window
EOF

for doc in sheet/tests/data/kb/*.fods "$out/sample/sample.fods"; do
    name="$(basename "$doc" .fods)"
    echo "== $name"
    for look in light dark; do
        for n in 1 2; do
            frame="$out/$name-$look-$n.png"
            if [ "$look" = dark ]; then
                bounded 60 "$mac" "$doc" --render-to "$frame" --dark
            else
                bounded 60 "$mac" "$doc" --render-to "$frame"
            fi
            is_png "$frame" || fail "$frame is not a PNG"
        done
        cmp -s "$out/$name-$look-1.png" "$out/$name-$look-2.png" \
            || fail "$name: two $look renders of the same document differ"
        rm "$out/$name-$look-2.png"
    done
    if cmp -s "$out/$name-light-1.png" "$out/$name-dark-1.png"; then
        fail "$name: the dark frame is the light one — --dark reached nothing"
    fi
    echo "   reproducible in both appearances, and the two differ"

    bounded 120 "$mac" "$doc" --drive "$drive" --out "$out/$name-window" \
        || fail "$name: the drive that opens it failed"
    is_png "$out/$name-window/window.png" || fail "$name: no snapshot of its window"
    echo "   opened in a window, snapshot kept"
done
