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
#      only by the renderer, fails here;
#   3. M3's: a drive moves the selection with the arrows, grows it with Shift, collapses it with
#      Esc and sends it to G20 through Go To and the name box, and the transcript has to say so
#      after each step — keys that took the path a real key takes, through the user's own
#      bindings;
#   4. M4's: an untouched open writes nothing, bytes and modification time; a driven edit —
#      a number and a formula typed in display syntax — is saved in place, lints clean and
#      reads back as typed; and an edited import leaves its workbook, and the folder beside it,
#      alone;
#   5. M5's: a copy in the application is on the system pasteboard as the cells' tab-separated
#      input text, and text another application put there pastes as a rectangle of cells.
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

echo "== selection"
cat > "$out/select.drive" <<'EOF'
key right
key down
key shift+down
key escape
key cmd+l
type g20
key return
wait 0.5
snap g20
EOF
bounded 120 "$mac" "$out/sample/sample.fods" --drive "$out/select.drive" --out "$out/select" \
    > "$out/select.txt" || fail "the selection drive failed: $(cat "$out/select.txt")"
cat "$out/select.txt"
expect() {
    grep -q "^step $1: .*selection $2\$" "$out/select.txt" \
        || fail "after step $1 the selection is not $2"
}
expect 1 'B1'
expect 2 'B2'
expect 3 'B2:B3 (active B3)'
expect 4 'B3'
expect 7 'G20'
echo "   arrows, Shift, Esc and a typed place all land where they should"

echo "== editing and saving"
# An untouched open writes nothing: the bytes and the modification time both stay.
cp "$out/sample/sample.fods" "$out/untouched.fods"
before_sum="$(shasum "$out/untouched.fods" | cut -d' ' -f1)"
before_time="$(stat -f %m "$out/untouched.fods")"
printf 'wait 2\n' > "$out/wait.drive"
bounded 120 "$mac" "$out/untouched.fods" --drive "$out/wait.drive" --out "$out/untouched" \
    > /dev/null || fail "the drive that only opens a document failed"
[ "$(shasum "$out/untouched.fods" | cut -d' ' -f1)" = "$before_sum" ] \
    || fail "opening a document and touching nothing changed its bytes"
[ "$(stat -f %m "$out/untouched.fods")" = "$before_time" ] \
    || fail "opening a document and touching nothing rewrote it"
echo "   an untouched open writes nothing"

# A driven edit is stored the way a typed one is — display syntax in, ODF's out — and saved in
# place: the file lints clean and reads back as typed.
cp "$out/sample/sample.fods" "$out/edited.fods"
cat > "$out/edit.drive" <<'EOF'
key cmd+l
type Z90
key return
type 42
key return
key cmd+l
type Z91
key return
type =Z90*2
key return
key cmd+s
wait 2
EOF
bounded 120 "$mac" "$out/edited.fods" --drive "$out/edit.drive" --out "$out/edited" \
    > "$out/edit.txt" || fail "the editing drive failed: $(cat "$out/edit.txt")"
"$grind" lint "$out/edited.fods" || fail "the saved document does not lint clean"
[ "$("$grind" sheet view "$out/edited.fods" Z90:Z91 | tr -d ' \r' | paste -sd, -)" = "42,84" ] \
    || fail "the saved document does not hold what was typed: $("$grind" sheet view "$out/edited.fods" Z90:Z91)"
# Typed in display syntax, stored in ODF's.
"$grind" sheet project "$out/edited.fods" | grep -F 'formula="=[.Z90]*2"' | grep -q 'cell Z91' \
    || fail "the formula was not stored as a formula"
echo "   a typed number and formula are saved in place, lint clean, and project as typed"

# An imported workbook is untitled: editing it writes nothing beside it and nothing into it,
# autosave included.
workbook="xlsx/tests/data/sample.xlsx"
if [ -f "$workbook" ]; then
    cp "$workbook" "$out/imported.xlsx"
    source_sum="$(shasum "$out/imported.xlsx" | cut -d' ' -f1)"
    printf 'type 7\nkey return\nwait 8\n' > "$out/import.drive"
    bounded 120 "$mac" "$out/imported.xlsx" --drive "$out/import.drive" --out "$out/imported" \
        > /dev/null || fail "the drive that edits an imported workbook failed"
    [ "$(shasum "$out/imported.xlsx" | cut -d' ' -f1)" = "$source_sum" ] \
        || fail "an imported workbook was written to"
    [ ! -e "$out/imported.fods" ] || fail "an import was saved beside its workbook"
    echo "   an edited import leaves its workbook alone"
fi

echo "== the pasteboard"
# Copy in the application, and the job's own `pbpaste` shows the rectangle's input text, tab- and
# line-separated — cross-application interop, which `ui_win32` could not check under Wine.
cp "$out/sample/sample.fods" "$out/pasteboard.fods"
printf 'key shift+right\nkey shift+down\nkey cmd+c\nwait 0.5\n' > "$out/copy.drive"
bounded 120 "$mac" "$out/pasteboard.fods" --drive "$out/copy.drive" --out "$out/copy" \
    > /dev/null || fail "the copy drive failed"
input() { "$grind" sheet get --input "$out/pasteboard.fods" "$1"; }
expected="$(printf '%s\t%s\n%s\t%s' "$(input A1)" "$(input B1)" "$(input A2)" "$(input B2)")"
[ "$(pbpaste)" = "$expected" ] || fail "pbpaste shows $(pbpaste | head -c 200), not the copied cells"
echo "   a copy reaches the system pasteboard as the cells' tab-separated input text"

# `pbcopy` in the job, Paste in the application, and the cells hold it once saved.
printf 'pasted\tthere' | pbcopy
printf 'key cmd+l\ntype Z97\nkey return\nkey cmd+v\nkey cmd+s\nwait 2\n' > "$out/paste.drive"
bounded 120 "$mac" "$out/pasteboard.fods" --drive "$out/paste.drive" --out "$out/paste" \
    > /dev/null || fail "the paste drive failed"
[ "$(input Z97)" = "pasted" ] && [ "$(input AA97)" = "there" ] \
    || fail "the pasted text did not land in Z97:AA97: $(input Z97) / $(input AA97)"
echo "   text from another application pastes as a rectangle of cells"
