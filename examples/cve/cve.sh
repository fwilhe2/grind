#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: MIT
#
# A spreadsheet from a directory of grype scans (`grype -o json`, one file per image).
#
#   examples/cve/cve.sh ~/scans/cve scan.ods
#   examples/cve/cve.sh -t "Nightly, amd64" ~/scans/cve scan.fods
#   GRIND=target/debug/grind examples/cve/cve.sh …     # a binary other than the one on PATH
#
# Each `*.json` in the directory is one image, named for its file. They are copied into a
# scratch directory beside a `scans.json` naming them, because `cve.rhai` reads only the files
# a manifest names. After the build, every table gets filter buttons and banding
# (`sheet format-table`), its severities their colours back as conditional rules (the banding
# replaces a cell's own background, a rule draws over it), and its `Shown` column is hidden.
#
# Needs `jq`.

set -euo pipefail

GRIND=${GRIND:-grind}
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

title=
while getopts t: opt; do
    case $opt in
    t) title=$OPTARG ;;
    *) exit 2 ;;
    esac
done
shift $((OPTIND - 1))
if [[ $# -ne 2 ]]; then
    echo "usage: $0 [-t TITLE] SCAN_DIR OUT.ods|OUT.fods" >&2
    exit 2
fi
scans=$1 out=$2
if [[ -z $title ]]; then
    dir=$(cd "$scans" && pwd)
    title="Vulnerability scan: $(basename "$(dirname "$dir")")/$(basename "$dir")"
fi

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT

names=()
for f in "$scans"/*.json; do
    [[ -e $f ]] || continue
    name=$(basename "$f" .json)
    [[ $name != scans ]] || continue
    jq -e '.matches | type == "array"' "$f" >/dev/null 2>&1 || {
        echo "skipping $f: not a grype JSON report" >&2
        continue
    }
    cp "$f" "$stage/$name.json"
    names+=("$name")
done
if [[ ${#names[@]} -eq 0 ]]; then
    echo "$scans holds no grype JSON reports" >&2
    exit 1
fi
jq -n --arg title "$title" '{title: $title, scans: [$ARGS.positional[] | {name: ., file: (. + ".json")}]}' \
    --args "${names[@]}" >"$stage/scans.json"

"$GRIND" build "$here/cve.rhai" --data "$stage" -o "$out" >/dev/null

# A column's letters from its 1-based number.
letters() {
    local n=$1 s=
    while ((n > 0)); do
        s=$(printf "\\$(printf %o $((65 + (n - 1) % 26)))")$s
        n=$(((n - 1) / 26))
    done
    printf %s "$s"
}

# Severity colours as conditional rules, which draw over the table's banding. The script
# colours the same cells directly, which banding replaces; these are its colours.
colour() {
    local range=$1 at=$2
    "$GRIND" sheet rule "$out" "$range" "[.$at]=\"Critical\"" --background '#b71c1c' --color white --bold >/dev/null
    "$GRIND" sheet rule "$out" "$range" "[.$at]=\"High\"" --background '#e65100' --color white --bold >/dev/null
    "$GRIND" sheet rule "$out" "$range" "[.$at]=\"Medium\"" --background '#f9a825' --color black --bold >/dev/null
    "$GRIND" sheet rule "$out" "$range" "[.$at]=\"Low\"" --background '#90caf9' --color black --bold >/dev/null
    "$GRIND" sheet rule "$out" "$range" "[.$at]=\"Negligible\"" --background '#e0e0e0' --color black --bold >/dev/null
    "$GRIND" sheet rule "$out" "$range" "[.$at]=\"Unknown\"" --background '#bdbdbd' --color black --bold >/dev/null
}

info=$("$GRIND" --format json info "$out")
for sheet in Findings Vulnerabilities Packages; do
    rows=$(jq --arg s "$sheet" '.sheets[] | select(.name == $s) | .rows' <<<"$info")
    cols=$(jq --arg s "$sheet" '.sheets[] | select(.name == $s) | .cols' <<<"$info")
    shown=$("$GRIND" --format json sheet find "$out" Shown --whole-cell --match-case |
        jq -r --arg s "$sheet" '.lines[] | split("\t")[0] | select(test("^" + $s + "\\.[A-Z]+1$"))' | head -1)
    shown=${shown#"$sheet."}
    "$GRIND" sheet format-table "$out" "$sheet.A1:$(letters "$cols")$rows" --name "${sheet,,}" \
        >/dev/null 2> >(grep -v '^grind: named' >&2)
    "$GRIND" sheet hide "$out" "$sheet.${shown%1}" >/dev/null
    case $sheet in
    Findings) colour "Findings.B2:B$rows" B2 ;;
    Vulnerabilities) colour "Vulnerabilities.C2:C$rows" C2 ;;
    Packages) colour "Packages.E1:J1" E1 ;;
    esac
done
echo "$out: ${#names[@]} images"
