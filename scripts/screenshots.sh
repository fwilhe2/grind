#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# screenshots.sh — the same two documents, photographed by this platform's own shell.
#
#   scripts/screenshots.sh [--desktop] [output-directory]    # default: screenshots/<platform>
#
# Run it on Linux, Windows (Git Bash or MSYS2) and macOS in turn; each run photographs only the
# shell native to the machine it is on, light and dark. The documents are built through the CLI
# from fixed data — no TODAY(), no RAND(), the document's locale set, the user's grind settings
# kept out — so a picture taken next month differs from today's only where grind changed.
#
#   Linux    grind-sheet-gtk, grind-text-gtk  --render-to on a private Broadway display, so the
#                                             desktop's scale, fonts and accent stay out of it;
#                                             --desktop uses the session's own display instead
#   Windows  grind-win32.exe                  --render-to, BMP converted to PNG
#   macOS    Grind.app                        --drive, a real window with its title bar
#
# Binaries: the newer of target/release and target/debug, else PATH; GRIND, GRIND_SHEET_GTK,
# GRIND_TEXT_GTK, GRIND_WIN32 and GRIND_MAC name one directly. Linux without --desktop needs
# gtk4-broadwayd and firefox.

set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

case "$(uname -s)" in
    Linux) platform=linux exe= ;;
    Darwin) platform=macos exe= ;;
    MINGW* | MSYS* | CYGWIN*) platform=windows exe=.exe ;;
    *) echo "screenshots: no native shell for $(uname -s)" >&2; exit 1 ;;
esac

desktop=
if [ "${1:-}" = --desktop ]; then
    desktop=1
    shift
fi
out=${1:-$root/screenshots/$platform}
mkdir -p "$out"
out=$(cd "$out" && pwd)
work=$(mktemp -d)
pids=()
cleanup() {
    for pid in "${pids[@]+"${pids[@]}"}"; do kill "$pid" 2>/dev/null || true; done
    rm -rf "$work"
}
trap cleanup EXIT

die() { echo "screenshots: $*" >&2; exit 1; }

# The newer of target/release/<name> and target/debug/<name>, else the one on PATH.
find_bin() {
    local name="$1$exe" found=
    for dir in "$root/target/release" "$root/target/debug"; do
        if [ -x "$dir/$name" ] && { [ -z "$found" ] || [ "$dir/$name" -nt "$found" ]; }; then
            found="$dir/$name"
        fi
    done
    [ -n "$found" ] && { echo "$found"; return; }
    command -v "$name" || die "no $name in target/ or on PATH; build it or set its variable"
}

# A remembered spelling language or locale lives in grind's config directory; an empty one keeps
# them out of the pictures.
if [ "$platform" = windows ]; then
    XDG_CONFIG_HOME=$(cygpath -w "$work/config")
else
    XDG_CONFIG_HOME="$work/config"
fi
export XDG_CONFIG_HOME
unset GRIND_LOCALE

GRIND=${GRIND:-$(find_bin grind)}
sheet() { "$GRIND" sheet "$@" >/dev/null; }
text() { "$GRIND" text "$@" >/dev/null; }

# --- the spreadsheet ------------------------------------------------------------------------
#
# A small design studio's invoice book: who was billed for what, when it falls due, whether it
# was paid, and what was invoiced each month. Two invoices are overdue on the fixed "as of" day.

build_sheet() {
    local book="$1"
    local ink='#1f3a5f' bar='#3d72b4' muted='#6b7280' late='#b3261e'
    local usd=(currency --symbol '$' --grouping)

    sheet new "$book" --force
    sheet locale "$book" en-US
    sheet rename "$book" Sheet1 Invoices
    sheet add "$book" Clients

    # Clients first: the invoices look their payment terms up here.
    cat >"$work/clients.tsv" <<'EOF'
Client	City	Terms (days)	Client since
Willow Street Bakery	Portland	14	2023-03-01
Riverside Library	Madison	30	2024-09-15
Lantern Playhouse	Chicago	30	2022-11-08
North Hill Cycling Club	Boulder	30	2025-04-22
Copper Kettle Tea	Seattle	30	2024-01-10
Bluebell Books	Austin	45	2026-05-19
EOF
    sheet import-csv "$book" "$work/clients.tsv" --at Clients.A1 --delimiter tab --dates
    sheet name "$book" Terms 'Clients.$A$2:$C$7'
    sheet style "$book" Clients.A1:D1 --bold --color white --background "$ink"
    sheet width "$book" Clients.A 5cm
    sheet width "$book" Clients.B:D 3cm

    sheet set "$book" A1 'Invoices 2026'
    sheet style "$book" A1 --bold --size 16pt --color "$ink"
    sheet set "$book" A2 'Kestrel Studio · design and illustration'
    sheet style "$book" A2 --italic --color "$muted"
    sheet merge "$book" A1:D1
    sheet merge "$book" A2:D2
    sheet height "$book" 1 0.9cm
    sheet set "$book" E2 'As of'
    sheet style "$book" E2 --align right --color "$muted"
    sheet set "$book" F2 '=DATE(2026;10;31)'
    sheet format "$book" F2 date
    sheet style "$book" F2 --bold --align right --color "$ink"

    cat >"$work/invoices.tsv" <<'EOF'
Invoice	Client	Project	Issued	Due	Amount	Paid
INV-0418	Willow Street Bakery	Menu boards	2026-06-03		1450
INV-0419	Riverside Library	Reading posters	2026-06-17		2800
INV-0420	Lantern Playhouse	Season brochure	2026-07-01		3250
INV-0421	North Hill Cycling Club	Jersey design	2026-07-15		980
INV-0422	Copper Kettle Tea	Tin labels	2026-07-29		1720
INV-0423	Willow Street Bakery	Window decals	2026-08-06		640
INV-0424	Bluebell Books	Shop signage	2026-08-20		2150
INV-0425	Riverside Library	Annual report	2026-09-02		4400
INV-0426	Lantern Playhouse	Ticket design	2026-09-16		760
INV-0427	Copper Kettle Tea	Gift box	2026-09-30		2380
INV-0428	North Hill Cycling Club	Race poster	2026-10-14		1150
INV-0429	Bluebell Books	Catalog layout	2026-10-27		3600
EOF
    sheet import-csv "$book" "$work/invoices.tsv" --at A3 --delimiter tab --dates
    sheet set "$book" E4 '=[.D4]+VLOOKUP([.B4];Terms;3;0)'
    sheet fill "$book" E4 E5:E15
    sheet format "$book" E4:E15 date
    sheet format "$book" F4:F17 "${usd[@]}"

    # Column G draws the checkboxes; the TRUE or FALSE each sets lives in Z, hidden and far out
    # of view, because a hidden column's text still overflows into the one beside it.
    printf '%s\n' TRUE TRUE TRUE TRUE TRUE TRUE TRUE TRUE FALSE FALSE FALSE FALSE |
        sheet paste "$book" Z4 -
    sheet hide "$book" Z
    for row in $(seq 4 15); do
        sheet checkbox "$book" "G$row" --link "Z$row"
    done

    sheet set "$book" A16 'Total'
    sheet set "$book" F16 '=SUM([.F4:.F15])'
    sheet set "$book" A17 'Outstanding'
    sheet set "$book" F17 '=SUMIF([.Z4:.Z15];FALSE();[.F4:.F15])'
    sheet style "$book" A16:F16 --bold --color "$ink"
    sheet style "$book" A17:F17 --color "$muted"

    sheet format-table "$book" A3:G15 --name Ledger 2>/dev/null
    sheet style "$book" A3:G3 --bold --color white --background "$ink"
    sheet rule "$book" E4:F15 'AND(NOT([.$Z4]);[.$E4]<[.$F$2])' --bold --color "$late"

    # What was invoiced per month, with the chart drawn from it underneath. That way round, a
    # shell whose rows are shorter than GTK's leaves a wider gap rather than an overlap, since a
    # chart is placed in centimetres and not on cells.
    printf '%s\n' $'Month\tInvoiced' Jun Jul Aug Sep Oct | sheet paste "$book" I3 -
    local row month
    for month in 6 7 8 9 10; do
        row=$((month - 2))
        sheet set "$book" "J$row" "=SUMPRODUCT(MONTH([.\$D\$4:.\$D\$15])=$month;[.\$F\$4:.\$F\$15])"
    done
    sheet format "$book" J4:J8 "${usd[@]}" --decimals 0
    sheet style "$book" I3:J3 --bold --color white --background "$ink"

    sheet width "$book" A 2.1cm
    sheet width "$book" B 3.9cm
    sheet width "$book" C 3.1cm
    sheet width "$book" D:E 2.2cm
    sheet width "$book" F 2.4cm
    sheet width "$book" G 1.7cm
    sheet width "$book" H 0.3cm
    sheet width "$book" I:J 2.4cm

    sheet chart-add "$book" --type bar --categories I4:I8 --series J4:J8=J3:J3 \
        --title 'Invoiced per month' --legend none \
        --x 17.9cm --y 6.7cm --width 7cm --height 5cm
    sheet chart-style "$book" 0 --y-gridlines true --series-color "0=$bar"

    sheet recalc "$book"
}

# --- the text document ----------------------------------------------------------------------
#
# A one-page project brief from the same studio: headings, a list, a table, a link.

build_text() {
    local doc="$1"
    text new "$doc" --force
    cat >"$work/brief.md" <<'EOF'
# Riverside Library: Reading Posters

A short brief for the autumn campaign, written after the kickoff meeting on *October 6*.

## What we are making

Six posters for the branch libraries, each one pairing a **single book** with a place in town
where someone might read it. The series runs from November to January and should feel warm,
quiet and a little playful.

- Two sizes: A2 for the branches, A4 for notice boards
- One shared typeface, one accent color per poster
- A short line of text, never more than twelve words

## Schedule

| Step | Who | Due |
|:--|:--|:--|
| Sketches | Kestrel Studio | Oct 20 |
| Feedback | Library staff | Oct 27 |
| Final art | Kestrel Studio | Nov 10 |
| Printing | Harbor Press | Nov 17 |

## Open questions

Do the branches want space for their own opening hours, or should every poster point to the
[library website](https://example.org/library) instead? We will settle this at the next meeting.
EOF
    text import-md "$doc" "$work/brief.md"
    # import-md appends, so the new document's own empty first paragraph goes.
    text delete "$doc" p1
}

build_sheet "$work/invoices.fods"
build_text "$work/brief.fodt"

# --- the pictures ---------------------------------------------------------------------------

# A private Broadway display with a headless browser attached: GTK paces its frames by the
# browser, and a display with none never draws.
broadway() {
    command -v gtk4-broadwayd >/dev/null || die "no gtk4-broadwayd; install it or pass --desktop"
    command -v firefox >/dev/null || die "no firefox to drive Broadway; install it or pass --desktop"
    local display=$((20 + RANDOM % 50))
    gtk4-broadwayd ":$display" >/dev/null 2>&1 &
    pids+=($!)
    sleep 1
    firefox --headless --no-remote --profile "$work/firefox" \
        "http://127.0.0.1:$((8080 + display))/" >/dev/null 2>&1 &
    pids+=($!)
    sleep 6
    unset WAYLAND_DISPLAY DISPLAY
    export GDK_BACKEND=broadway BROADWAY_DISPLAY=":$display"
}

shoot_linux() {
    local sheet_gtk text_gtk scheme suffix
    sheet_gtk=${GRIND_SHEET_GTK:-$(find_bin grind-sheet-gtk)}
    text_gtk=${GRIND_TEXT_GTK:-$(find_bin grind-text-gtk)}
    if [ -n "$desktop" ]; then
        [ -n "${WAYLAND_DISPLAY:-}${DISPLAY:-}" ] || die "--desktop needs a desktop session"
    else
        mkdir -p "$work/firefox"
        broadway
    fi
    for scheme in light dark; do
        suffix=${scheme/light/}
        suffix=${suffix:+-$suffix}
        export ADW_DEBUG_COLOR_SCHEME="prefer-$scheme"
        "$sheet_gtk" "$work/invoices.fods" --render-to "$out/sheet$suffix.png" >/dev/null 2>&1
        "$text_gtk" "$work/brief.fodt" --render-to "$out/text$suffix.png" >/dev/null 2>&1
    done
}

# grind-win32 writes a BMP; .NET makes it a PNG, so nothing needs installing.
shoot_windows() {
    local win32 doc name file
    win32=${GRIND_WIN32:-$(find_bin grind-win32)}
    for doc in sheet:invoices.fods text:brief.fodt; do
        name=${doc%%:*} file=$(cygpath -w "$work/${doc#*:}")
        "$win32" "$file" --render-to "$(cygpath -w "$work/$name.bmp")"
        "$win32" "$file" --render-to "$(cygpath -w "$work/$name-dark.bmp")" --dark
    done
    powershell.exe -NoProfile -Command "
        Add-Type -AssemblyName System.Drawing
        foreach (\$n in 'sheet', 'sheet-dark', 'text', 'text-dark') {
            \$b = [System.Drawing.Bitmap]::new('$(cygpath -w "$work")\\' + \$n + '.bmp')
            \$b.Save('$(cygpath -w "$out")\\' + \$n + '.png', [System.Drawing.Imaging.ImageFormat]::Png)
            \$b.Dispose()
        }"
}

# A drive opens a real window, so the picture has the title bar a person sees. The appearance is
# asked of AppKit through its argument domain, which outranks the system setting.
shoot_macos() {
    local mac doc name file style
    if [ -n "${GRIND_MAC:-}" ]; then
        mac=$GRIND_MAC
    elif [ -x /Applications/Grind.app/Contents/MacOS/grind-mac ]; then
        mac=/Applications/Grind.app/Contents/MacOS/grind-mac
    else
        mac=$(find_bin grind-mac)
    fi
    printf 'wait 1.5\nsnap shot\n' >"$work/snap.drive"
    for doc in sheet:invoices.fods text:brief.fodt; do
        name=${doc%%:*} file="$work/${doc#*:}"
        for style in Light Dark; do
            "$mac" "$file" --drive "$work/snap.drive" --out "$work" \
                -AppleInterfaceStyle "$style" >/dev/null
            if [ "$style" = Dark ]; then
                mv "$work/shot.png" "$out/$name-dark.png"
            else
                mv "$work/shot.png" "$out/$name.png"
            fi
        done
    done
}

"shoot_$platform"
cp "$work/invoices.fods" "$work/brief.fodt" "$out/"
"$GRIND" --version >"$out/version.txt"
for picture in sheet sheet-dark text text-dark; do
    [ -s "$out/$picture.png" ] || die "$picture.png was not drawn"
done
echo "$out"
