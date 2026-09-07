#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# The arithmetic behind `.github/workflows/size.yml`, in one file because all three
# subcommands answer the same question — *how big is what we ship, and what is in it* —
# and splitting them would put the byte-formatting rule in three places.
#
#   sizes.py record   ...   one measured artifact, appended to a JSON-lines file
#   sizes.py report   ...   those lines (from every runner) as one markdown table
#   sizes.py wasm-crates    `twiggy top -f json` rolled up per crate
#
# Why a script rather than shell: the numbers cross runners. A Windows job and two Linux
# jobs each measure a different thing, and the table that compares them is assembled
# afterwards from what they uploaded — so the *format* has to be one decision, made once,
# rather than three `printf`s that agree until one of them is edited.
#
# Nothing here builds or invokes a compiler. It reads files that already exist.

from __future__ import annotations

import argparse
import collections
import gzip
import json
import os
import re
import sys
from pathlib import Path

# --------------------------------------------------------------------------------------
# Formatting
# --------------------------------------------------------------------------------------


def human(n: int | None) -> str:
    """Bytes as a person reads them.

    Binary units, because these are file sizes and every tool this script stands next to
    (`cargo bloat`, `twiggy`, `ls -lh`) uses them. An em dash for "not measured", which is
    a real state: a Windows PE has no `strip`, and only the browser's assets are served
    compressed, so most rows leave most columns empty.
    """
    if n is None:
        return "—"
    if abs(n) < 1024:
        return f"{n} B"
    v = float(n)
    for unit in ("KiB", "MiB", "GiB"):
        v /= 1024.0
        if abs(v) < 1024.0 or unit == "GiB":
            return f"{v:,.2f} {unit}"
    raise AssertionError("unreachable")


def delta(now: int | None, before: int | None) -> str:
    """One artifact's size against the same artifact on the baseline commit.

    Returns the empty-ish em dash when there is nothing to compare against, which is the
    common case rather than an error: the first run of this workflow on a branch has no
    baseline, and a newly added binary has no row in the old report.
    """
    if now is None or before is None:
        return "—"
    d = now - before
    if d == 0:
        return "no change"
    if not before:
        return f"{d:+d} B"
    return f"{'+' if d > 0 else '-'}{human(abs(d))} ({d / before * 100.0:+.1f}%)"


# --------------------------------------------------------------------------------------
# record
# --------------------------------------------------------------------------------------


def cmd_record(args: argparse.Namespace) -> int:
    """Measure one artifact and append it to a JSON-lines file.

    JSON lines rather than one JSON document because three jobs on three runners each
    write their own file and `report` concatenates them. A format where appending is
    `>>` cannot produce a merge conflict between two runners.
    """
    path = Path(args.file)
    if not path.is_file():
        print(f"sizes.py: no such artifact: {path}", file=sys.stderr)
        return 1

    record: dict[str, object] = {
        "artifact": args.artifact,
        "platform": args.platform,
        "bytes": path.stat().st_size,
    }
    if args.note:
        record["note"] = args.note

    # The symbols a default `[profile.release]` keeps. Rust does not strip on release and
    # this workspace sets no `strip =`, so every native binary here carries its symbol
    # table — worth a column, because it is the difference between what a package manager
    # ships and what the file on disk weighs, and it is invisible unless measured.
    if args.stripped:
        s = Path(args.stripped)
        if s.is_file():
            record["stripped"] = s.stat().st_size

    # Only meaningful for something served over HTTP. `ui_web/Dockerfile` puts `dist/` behind
    # static-web-server, so the number a browser actually downloads is the compressed one and
    # the raw size overstates the cost by roughly four to one.
    if args.gzip:
        record["gzip"] = len(gzip.compress(path.read_bytes(), 9))

    with open(args.out, "a", encoding="utf-8") as f:
        f.write(json.dumps(record, sort_keys=True) + "\n")
    print(f"{args.artifact} ({args.platform}): {human(record['bytes'])}")  # type: ignore[arg-type]
    return 0


# --------------------------------------------------------------------------------------
# report
# --------------------------------------------------------------------------------------


def load(paths: list[str]) -> list[dict]:
    out: list[dict] = []
    for p in paths:
        for line in Path(p).read_text(encoding="utf-8").splitlines():
            line = line.strip()
            if line:
                out.append(json.loads(line))
    return out


def cmd_report(args: argparse.Namespace) -> int:
    """Every artifact from every runner as one markdown table.

    This is the whole point of the workflow: the six things this suite ships are built by
    three different jobs on two operating systems, and until they are in one table nobody
    can see that the CLI is two and a half times the size of a GTK window.
    """
    rows = load(args.input)
    rows.sort(key=lambda r: (-r["bytes"], r["artifact"]))

    baseline: dict[tuple[str, str], dict] = {}
    if args.baseline:
        try:
            for r in load([args.baseline]):
                baseline[(r["artifact"], r["platform"])] = r
        except FileNotFoundError:
            # Not an error. A branch whose first run this is has no baseline to fetch, and
            # a report with no delta column filled in is still the report.
            print(f"sizes.py: no baseline at {args.baseline}, reporting absolute sizes only",
                  file=sys.stderr)

    lines = ["| Artifact | Platform | Ships | Stripped | gzip | vs baseline |",
             "|---|---|--:|--:|--:|--:|"]
    for r in rows:
        base = baseline.get((r["artifact"], r["platform"]))
        name = f"`{r['artifact']}`"
        if r.get("note"):
            name += f"<br><sub>{r['note']}</sub>"
        lines.append(
            f"| {name} | {r['platform']} | **{human(r['bytes'])}** "
            f"| {human(r.get('stripped'))} | {human(r.get('gzip'))} "
            f"| {delta(r['bytes'], base['bytes'] if base else None)} |"
        )

    total = sum(r["bytes"] for r in rows)
    lines.append(f"\n{len(rows)} artifacts, {human(total)} in total.\n")

    out = "\n".join(lines)
    print(out)
    if summary := os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(summary, "a", encoding="utf-8") as f:
            f.write(out + "\n")
    return 0


# --------------------------------------------------------------------------------------
# wasm-crates
# --------------------------------------------------------------------------------------

# A monomorphised Rust symbol in a wasm module carries its crate and that crate's
# disambiguator hash: `grind_sheet[6806b85635c70cf5]::odf::write::content`. The first such
# name in a symbol is the crate the code *belongs to* — for a generic instantiated
# elsewhere the instantiating crate comes first, which is the attribution `cargo bloat`
# makes too, so the two halves of this workflow agree about whose bytes these are.
SYMBOL = re.compile(r"([A-Za-z_][A-Za-z0-9_]*)\[[0-9a-f]{8,}\]")


def bucket(name: str) -> str:
    """Which crate — or which non-code part of the module — an item belongs to."""
    if m := SYMBOL.search(name):
        return m.group(1)
    # twiggy reports the module's non-function content as items too, and they are a large
    # share of it: the name section alone is usually a fifth to a third of a wasm-bindgen
    # module. Lumping them into "unattributed" would hide exactly the thing worth seeing.
    if "subsection" in name:
        return "(name section)"
    if name.startswith("custom section"):
        return "(custom sections)"
    if name.startswith("data segment"):
        return "(data segments)"
    return "(wasm structure)"


def cmd_wasm_crates(args: argparse.Namespace) -> int:
    """`twiggy top -f json` rolled up per crate.

    `cargo bloat --crates` answers this for a native binary and has no wasm backend;
    `twiggy` reads wasm and has no notion of a crate. This is the missing half — so the
    browser bundle can be read with the same question as the five executables.
    """
    items = json.loads(Path(args.input).read_text(encoding="utf-8"))
    if not items:
        print("sizes.py: twiggy produced no items", file=sys.stderr)
        return 1

    total = sum(i["shallow_size"] for i in items)
    by: collections.Counter[str] = collections.Counter()
    for i in items:
        by[bucket(i["name"])] += i["shallow_size"]

    lines = ["| Crate / section | Bytes | Share |", "|---|--:|--:|"]
    shown = by.most_common(args.top)
    for k, v in shown:
        lines.append(f"| `{k}` | {human(v)} | {100.0 * v / total:.1f}% |")
    rest = total - sum(v for _, v in shown)
    if rest > 0:
        lines.append(f"| _{len(by) - len(shown)} more_ | {human(rest)} | {100.0 * rest / total:.1f}% |")
    lines.append(f"| **total** | **{human(total)}** | 100.0% |")

    out = "\n".join(lines)
    print(out)
    if summary := os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(summary, "a", encoding="utf-8") as f:
            f.write(out + "\n")
    return 0


# --------------------------------------------------------------------------------------


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    sub = ap.add_subparsers(dest="cmd", required=True)

    r = sub.add_parser("record", help="measure one artifact into a JSON-lines file")
    r.add_argument("--artifact", required=True, help="the name it ships under")
    r.add_argument("--platform", required=True, help="e.g. linux-x86_64, windows-x86_64, wasm32")
    r.add_argument("--file", required=True, help="the file to measure")
    r.add_argument("--stripped", help="a stripped copy of it, if one was made")
    r.add_argument("--gzip", action="store_true", help="also record the gzipped size")
    r.add_argument("--note", help="a short parenthetical for the table")
    r.add_argument("--out", required=True, help="the .jsonl file to append to")
    r.set_defaults(func=cmd_record)

    p = sub.add_parser("report", help="merge JSON-lines files into one markdown table")
    p.add_argument("input", nargs="+", help="the .jsonl files to merge")
    p.add_argument("--baseline", help="a .jsonl from an earlier run, to diff against")
    p.set_defaults(func=cmd_report)

    w = sub.add_parser("wasm-crates", help="roll `twiggy top -f json` up per crate")
    w.add_argument("input", help="twiggy's JSON output")
    w.add_argument("--top", type=int, default=25, help="rows before the remainder is summed")
    w.set_defaults(func=cmd_wasm_crates)

    args = ap.parse_args()
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
