#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# readme-screenshots.sh — put this build's screenshots into the README, for its author to run.
#
#   scripts/readme-screenshots.sh [--from DIR] [--dark] [--dry-run] [README.fodt]
#
# The README's pictures live inside README.fodt, one `draw:frame` each, in document order. This
# replaces each frame's picture with the screenshot of its slot (SLOTS below), gives the frame the
# new picture's height at its own width, and leaves every other byte of the file as it was. Then
# it exports the Markdown as File ▸ Save and Export Markdown does (`grind text export-md`, the same
# core), and removes the pictures in README.images that the new Markdown no longer names. The
# same screenshots make the same files, so running it twice changes nothing the second time.
#
#   --from DIR   take the pictures from a screenshot run already made (scripts/screenshots.sh's
#                output directory); without it, the script makes one
#   --dark       the dark variants
#   --dry-run    say what would change, and write nothing
#
# It refuses when README.fodt holds a different number of pictures than there are slots: a
# picture added or removed means SLOTS needs a look first. Review with `git diff` and commit.

set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

# The screenshot in each of the README's pictures, top to bottom.
SLOTS=(sheet text tape tape-sheet)

from= dark= dry=
fodt=$root/README.fodt
while [ $# -gt 0 ]; do
    case "$1" in
        --from) from=${2:?--from needs a directory}; shift 2 ;;
        --dark) dark=1; shift ;;
        --dry-run) dry=1; shift ;;
        -h | --help) sed -n '6,22p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        -*) echo "readme-screenshots: unknown option $1" >&2; exit 2 ;;
        *) fodt=$1; shift ;;
    esac
done
[ -f "$fodt" ] || { echo "readme-screenshots: no $fodt" >&2; exit 1; }
fodt=$(cd "$(dirname "$fodt")" && pwd)/$(basename "$fodt")
markdown=${fodt%.fodt}.md
images=${fodt%.fodt}.images

# The newer of target/release and target/debug, else PATH — as scripts/screenshots.sh finds it.
find_grind() {
    local found=
    for dir in "$root/target/release" "$root/target/debug"; do
        if [ -x "$dir/grind" ] && { [ -z "$found" ] || [ "$dir/grind" -nt "$found" ]; }; then
            found="$dir/grind"
        fi
    done
    [ -n "$found" ] && { echo "$found"; return; }
    command -v grind || { echo "readme-screenshots: no grind; build it or set GRIND" >&2; exit 1; }
}
GRIND=${GRIND:-$(find_grind)}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
if [ -z "$from" ]; then
    from=$work/shots
    "$root/scripts/screenshots.sh" "$from" >/dev/null
fi

pictures=()
for slot in "${SLOTS[@]}"; do
    picture=$from/$slot${dark:+-dark}.png
    [ -s "$picture" ] || { echo "readme-screenshots: no $picture" >&2; exit 1; }
    pictures+=("$picture")
done

python3 -I - "$fodt" "$work/new.fodt" "${dry:+dry}" "${pictures[@]}" <<'EOF'
import base64, re, struct, sys

source, target, dry, pictures = sys.argv[1], sys.argv[2], sys.argv[3] == "dry", sys.argv[4:]
text = open(source, encoding="utf-8").read()
frame = re.compile(r'(<draw:frame\b[^>]*>)(.*?)(</draw:frame>)', re.S)
frames = [m for m in frame.finditer(text) if "<office:binary-data>" in m.group(2)]
if len(frames) != len(pictures):
    sys.exit(f"readme-screenshots: {source} holds {len(frames)} pictures and SLOTS names "
             f"{len(pictures)}; look at SLOTS first")

def size(png):
    if png[:8] != b"\x89PNG\r\n\x1a\n":
        sys.exit("readme-screenshots: not a PNG")
    return struct.unpack(">II", png[16:24])

out, at = [], 0
for m, path in zip(frames, pictures):
    png = open(path, "rb").read()
    width_px, height_px = size(png)
    opening, body, closing = m.group(1), m.group(2), m.group(3)
    width = re.search(r'svg:width="([\d.]+)([a-z]+)"', opening)
    if width:
        height = float(width.group(1)) * height_px / width_px
        opening = re.sub(r'svg:height="[\d.]+[a-z]+"',
                         f'svg:height="{height:.3f}{width.group(2)}"', opening)
    data = base64.b64encode(png).decode("ascii")
    body = re.sub(r'(<office:binary-data>).*?(</office:binary-data>)',
                  lambda b: b.group(1) + data + b.group(2), body, count=1, flags=re.S)
    old = re.search(r'<office:binary-data>(.*?)</office:binary-data>', m.group(2), re.S).group(1)
    same = base64.b64decode(re.sub(r"\s", "", old)) == png
    print(f"  {path.rsplit('/', 1)[-1]:<22} {width_px}×{height_px}  {'unchanged' if same else 'replaced'}")
    out += [text[at:m.start()], opening, body, closing]
    at = m.end()
out.append(text[at:])
if not dry:
    open(target, "w", encoding="utf-8").write("".join(out))
EOF

if [ -n "$dry" ]; then
    echo "dry run: nothing written"
    exit 0
fi

# The new document replaces the old only once it is whole, then the Markdown is exported from it.
cat "$work/new.fodt" >"$fodt"
"$GRIND" text export-md "$fodt" --out "$markdown" >/dev/null

# Pictures are named for their content: the old screenshots' files are now named by nothing.
if [ -d "$images" ]; then
    for file in "$images"/img-*.png; do
        [ -e "$file" ] || continue
        name=$(basename "$file")
        grep -qF "$name" "$markdown" || { rm -f "$file"; echo "  removed $(basename "$images")/$name"; }
    done
fi
echo "updated $(basename "$fodt") and $(basename "$markdown"); review with git diff"
