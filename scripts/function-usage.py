#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Which functions a corpus of spreadsheets calls, against the ones this build implements.

The measurement behind doc/function-coverage.md. Reads every .ods, .fods, .xlsx and .xlsm
under a directory (LibreOffice's sc/qa/unit/data is the one the document names), pulls the
function names out of every formula, and prints:

  - how many formula cells and files call only implemented functions
  - the most-used functions by number of files, implemented ones unmarked, missing ones with ✗
  - the missing functions ranked by the number of files they block

The implemented set is read out of sheet/src/formula/funcs/mod.rs's NAMES, so the script
cannot disagree with the engine. Function names are what a formula *says*: Excel's
`_xlfn.`/`_xlws.` prefixes and ODF's `COM.MICROSOFT.` are stripped, everything else is kept.

    python3 -I scripts/function-usage.py "$GRIND_LO_CORPUS/sc/qa/unit/data" --skip functions

`--skip functions` leaves out LibreOffice's one-file-per-function test directory, which
would otherwise count every function once and swamp the real documents.
"""

import argparse
import collections
import html
import os
import re
import zipfile

CALL = re.compile(r"((?:_xlfn\.|_xlws\.)?[A-Za-z][A-Za-z0-9._]*)\s*\(")
STRING = re.compile(r'"(?:[^"]|"")*"')
ODF_FORMULA = re.compile(r'table:formula="([^"]*)"')
XLSX_FORMULA = re.compile(r"<(?:\w+:)?f(?:\s[^>]*)?>([^<]*)</(?:\w+:)?f>")
PREFIX = re.compile(r"^(_XLFN\._XLWS\.|_XLFN\.|_XLWS\.|COM\.MICROSOFT\.|OF:)")


def implemented(repo):
    src = open(os.path.join(repo, "sheet/src/formula/funcs/mod.rs"), encoding="utf-8").read()
    body = src[src.index("const NAMES") :]
    return set(re.findall(r'"([A-Z0-9.]+)"', body[: body.index("];")]))


def formulas(path):
    ext = path.rsplit(".", 1)[-1].lower()
    if ext == "fods":
        return ODF_FORMULA.findall(open(path, encoding="utf-8", errors="replace").read())
    if ext == "ods":
        xml = zipfile.ZipFile(path).read("content.xml").decode("utf-8", "replace")
        return ODF_FORMULA.findall(xml)
    if ext in ("xlsx", "xlsm"):
        z = zipfile.ZipFile(path)
        sheets = [n for n in z.namelist() if n.startswith("xl/worksheets/") and n.endswith(".xml")]
        return [f for n in sheets for f in XLSX_FORMULA.findall(z.read(n).decode("utf-8", "replace"))]
    return None


def called(formula):
    text = STRING.sub('""', html.unescape(formula))
    return {PREFIX.sub("", n.upper()) for n in CALL.findall(text)}


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("corpus")
    ap.add_argument("--skip", action="append", default=[], help="directory name to leave out")
    ap.add_argument("--repo", default=os.path.join(os.path.dirname(__file__), ".."))
    ap.add_argument("--top", type=int, default=60)
    args = ap.parse_args()

    ours = implemented(args.repo)
    by_file = collections.Counter()
    blocking = collections.Counter()
    cells = covered_cells = files = covered_files = 0
    for directory, subdirs, names in os.walk(args.corpus):
        subdirs[:] = [d for d in subdirs if d not in args.skip]
        for name in names:
            try:
                found = formulas(os.path.join(directory, name))
            except Exception:
                continue  # locked, broken or not a package: loop A's business, not this
            if not found:
                continue
            files += 1
            seen, missing = set(), set()
            for formula in found:
                calls = called(formula)
                seen |= calls
                cells += 1
                if calls <= ours:
                    covered_cells += 1
                missing |= calls - ours
            by_file.update(seen)
            blocking.update(missing)
            covered_files += not missing

    print(f"formula cells calling only implemented functions: {covered_cells}/{cells}"
          f" = {covered_cells / max(cells, 1):.1%}")
    print(f"files calling only implemented functions: {covered_files}/{files}"
          f" = {covered_files / max(files, 1):.1%}")
    print(f"\nmost used, by files ({len(ours)} implemented; ✗ = missing):")
    print(" ".join(f"{n}{'' if n in ours else '✗'}:{c}" for n, c in by_file.most_common(args.top)))
    print("\nmissing, by files blocked:")
    print(" ".join(f"{n}:{c}" for n, c in blocking.most_common(args.top)))


if __name__ == "__main__":
    main()
