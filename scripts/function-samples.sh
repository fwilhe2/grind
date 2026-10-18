#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later

# Regenerate the function samples and vendor them, both halves:
#
#   sheet/tests/data/functions/            odsgen's documents, recalculated by LibreOffice
#   xlsx/tests/data/corpus/functions/      ooxmlgen's `functions` family, built from them
#
# The samples live in odsgen's `function_samples.py`; add one there when adding a function,
# then run this. Needs `soffice` (odsgen recalculates with it) and `dotnet` (ooxmlgen).
#
#   scripts/function-samples.sh [ODSGEN_DIR] [OOXMLGEN_DIR]     # default: ../odsgen ../ooxmlgen
#
# ooxmlgen writes its whole corpus, and every other fixture in it is vendored too; the script
# refuses to vendor if any of those changed, since that is a different change to review.

set -euo pipefail

here=$(cd "$(dirname "$0")/.." && pwd)
odsgen=$(cd "${1:-$here/../odsgen}" && pwd)
ooxmlgen=$(cd "${2:-$here/../ooxmlgen}" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

(cd "$odsgen" && python3 generate_function_samples.py)
rm -f "$here"/sheet/tests/data/functions/*.fods
cp "$odsgen"/functions/*.fods "$odsgen"/functions/README.md "$here/sheet/tests/data/functions/"

(cd "$ooxmlgen" && dotnet run -- --out "$work/corpus" --function-samples "$odsgen/functions/samples.json")
python3 -I - "$work/corpus/manifest.json" "$here/xlsx/tests/data/corpus/manifest.json" <<'EOF'
import json, sys
new = {f["file"]: f.get("sha256") for f in json.load(open(sys.argv[1]))["fixtures"]}
old = {f["file"]: f.get("sha256") for f in json.load(open(sys.argv[2]))["fixtures"]}
changed = [f for f in new if not f.startswith("functions/") and f in old and new[f] != old[f]]
if changed:
    sys.exit(f"ooxmlgen changed fixtures outside functions/, vendor those on purpose: {changed}")
EOF
rm -rf "$here/xlsx/tests/data/corpus/functions"
cp -r "$work/corpus/functions" "$here/xlsx/tests/data/corpus/"
cp "$work/corpus/manifest.json" "$work/corpus/README.md" "$here/xlsx/tests/data/corpus/"

echo "vendored; now: cargo test -p grind-sheet --test function_samples && cargo test -p grind-xlsx --test function_samples --test ooxmlgen"
