#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later

"""Import the large Craft corpus with Grind; write a compatibility matrix and import timings.

For every probe in ``manifest.json`` the matrix answers three questions: did Craft write the
feature (a pattern over the Office file's XML), did Grind's ODF output keep it (a pattern over
the ODF package's XML), and when it did not, did Grind's import report say so? A loss Grind
reports is honest; a loss it does not is the bug worth chasing. A probe may give a second, exact
pattern: content that matches the first but not the second survived in an altered form.

Losses every file of a kind reports (the baseline probe's) are not charged to each feature.
"""

import argparse
import json
import os
import re
import resource
import shutil
import statistics
import subprocess
import sys
import time
import zipfile
from pathlib import Path

LIMIT_BYTES = 12 * 1024 ** 3


def xml_text(path):
    """Every XML part of a zip package, concatenated, or the file itself when it is flat XML."""
    try:
        with zipfile.ZipFile(path) as package:
            return "\n".join(package.read(n).decode("utf-8", "replace") for n in sorted(package.namelist())
                             if n.endswith((".xml", ".rels", ".vml")))
    except zipfile.BadZipFile:
        return Path(path).read_text("utf-8", "replace")


def measure(argv, stdout=None):
    """Run argv; return (exit status, seconds, peak RSS in MiB)."""
    def cap():
        resource.setrlimit(resource.RLIMIT_AS, (LIMIT_BYTES, LIMIT_BYTES))

    start = time.perf_counter()
    child = subprocess.Popen(argv, stdout=stdout or subprocess.DEVNULL, stderr=subprocess.PIPE, preexec_fn=cap)
    _, status, usage = os.wait4(child.pid, 0)
    seconds = time.perf_counter() - start
    child.returncode = os.waitstatus_to_exitcode(status)
    err = child.stderr.read().decode("utf-8", "replace")
    child.stderr.close()
    return child.returncode, seconds, usage.ru_maxrss / 1024, err


def losses(report):
    """What a Grind import report says was lost, as {what: count}."""
    found = {}
    for key in ("dropped", "formats_lost", "appearance_lost", "names_lost", "must_understand", "untranslated", "unknown_functions", "renamed"):
        for item in report.get(key) or []:
            if isinstance(item, dict):
                what = item.get("what") or item.get("name") or item.get("function") or json.dumps(item, sort_keys=True)
                found[f"{key}: {what}"] = item.get("count", 1)
            else:
                found[f"{key}: {item}"] = 1
    return found


def grind_import(grind, kind, source, target):
    verb = ["sheet", "import"] if kind == "xlsx" else ["text", "import"]
    report_path = target.with_suffix(".report.json")
    with report_path.open("w") as out:
        status, seconds, rss, err = measure([grind, *verb, str(source), str(target), "--force", "--format", "json"], stdout=out)
    try:
        report = json.loads(report_path.read_text())
    except json.JSONDecodeError:
        report = {}
    return status, seconds, rss, err, report


def run_probes(grind, root, manifest, out):
    rows = []
    baseline = {}
    for probe in sorted(manifest["probes"], key=lambda p: (p["kind"], p["id"].split("/")[1] != "values", p["id"])):
        kind = probe["kind"]
        row = {"id": probe["id"], "kind": kind, "feature": probe["feature"]}
        source = root / probe["path"]
        if probe.get("craft_error") or not source.exists():
            row.update(status="not-built", note="Craft refused the script")
            rows.append(row)
            continue
        row["craft_wrote"] = bool(re.search(probe["src"], xml_text(source), re.S))
        target = out / "probes" / kind / (source.stem + (".ods" if kind == "xlsx" else ".odt"))
        target.parent.mkdir(parents=True, exist_ok=True)
        status, seconds, rss, err, report = grind_import(grind, kind, source, target)
        row.update(seconds=round(seconds, 3), rss_mib=round(rss, 1))
        if status != 0:
            row.update(status="failed", note=err.strip().splitlines()[-1] if err.strip() else f"exit {status}")
            rows.append(row)
            continue
        lost = losses(report)
        if probe["id"].endswith("/values"):
            baseline[kind] = set(lost)
        extra = {k: v for k, v in lost.items() if k not in baseline.get(kind, set())}
        row["reported"] = extra
        odf = xml_text(target)
        kept = bool(re.search(probe["odf"], odf, re.S))
        exact = kept and ("exact" not in probe or bool(re.search(probe["exact"], odf, re.S)))
        if not row["craft_wrote"]:
            row["status"] = "craft-absent"
        elif exact:
            row["status"] = "carried"
        elif kept:
            row["status"] = "altered"
        elif extra:
            row["status"] = "reported-loss"
        else:
            row["status"] = "silent-loss"
        rows.append(row)
    return rows


def run_scale(grind, root, manifest, out, repeat, extra_inputs):
    rows = []
    entries = [dict(e, source=root / e["path"]) for e in manifest["scale"] if not e.get("craft_error")]
    entries += [{"id": f"{Path(p).suffix[1:]}/reference:{Path(p).stem}", "kind": Path(p).suffix[1:], "size": None, "source": Path(p)} for p in extra_inputs]
    for entry in entries:
        kind, source = entry["kind"], entry["source"]
        print(f"scale {entry['id']}", file=sys.stderr)
        row = {"id": entry["id"], "kind": kind, "size": entry.get("size"), "input_bytes": source.stat().st_size}
        target = out / "scale" / kind / (source.stem + (".ods" if kind == "xlsx" else ".odt"))
        target.parent.mkdir(parents=True, exist_ok=True)
        times, rss = [], 0
        for _ in range(repeat):
            status, seconds, peak, err, report = grind_import(grind, kind, source, target)
            if status != 0:
                break
            times.append(seconds)
            rss = max(rss, peak)
        if status != 0:
            row.update(status="failed", note=(err.strip().splitlines() or [f"exit {status}"])[-1])
            rows.append(row)
            continue
        row.update(status="ok", import_s=round(statistics.median(times), 3), import_rss_mib=round(rss, 1), output_bytes=target.stat().st_size,
                   reported=losses(report), report={k: report.get(k) for k in ("sheets", "cells", "formulas", "styled", "blocks", "paragraphs", "headings", "tables", "pictures", "notes")
                                                   if report.get(k) is not None})
        status, seconds, peak, err = measure([grind, "info", str(target), "--format", "json"])
        row.update(reopen_s=round(seconds, 3), reopen_rss_mib=round(peak, 1)) if status == 0 else row.update(reopen_error=err.strip()[-200:])
        if kind == "xlsx":
            copy = target.with_name(target.stem + ".recalc.ods")
            shutil.copyfile(target, copy)
            status, seconds, peak, err = measure([grind, "sheet", "recalc", str(copy)])
            row.update(recalc_s=round(seconds, 3), recalc_rss_mib=round(peak, 1)) if status == 0 else row.update(recalc_error=err.strip()[-200:])
            copy.unlink(missing_ok=True)
        else:
            pdf = target.with_suffix(".pdf")
            pdf.unlink(missing_ok=True)
            status, seconds, peak, err = measure([grind, "text", "export-pdf", str(target), str(pdf)])
            if status == 0:
                pages = len(re.findall(rb"/Type\s*/Page[^s]", pdf.read_bytes()))
                row.update(pdf_s=round(seconds, 3), pdf_rss_mib=round(peak, 1), pdf_pages=pages)
            else:
                row.update(pdf_error=err.strip()[-200:])
        rows.append(row)
    return rows


ICON = {"carried": "✅ carried", "altered": "🟡 kept, altered", "reported-loss": "⚠️ dropped, reported", "silent-loss": "❌ silent loss", "failed": "💥 import failed",
        "craft-absent": "➖ Craft did not write it", "not-built": "➖ Craft refused"}


def size(n):
    return f"{n / 1e6:.1f} MB" if n >= 1e6 else f"{n / 1e3:.0f} kB"


def markdown(probes, scale, grind_version):
    lines = ["# Grind import compatibility matrix (Craft corpus)", "", f"Grind: `{grind_version}`. Generated by `scripts/compat_matrix.py`.", ""]
    for kind, title in (("xlsx", "Spreadsheet (.xlsx → .ods)"), ("docx", "Text (.docx → .odt)")):
        rows = [r for r in probes if r["kind"] == kind]
        counts = {s: sum(1 for r in rows if r["status"] == s) for s in ICON}
        lines += [f"## {title}", "", " · ".join(f"{ICON[s]}: {n}" for s, n in counts.items() if n), "",
                  "| Feature | Probe | Result | Grind reported |", "|---|---|---|---|"]
        for r in rows:
            reported = "; ".join(f"{k.split(': ', 1)[1]} ×{v}" for k, v in r.get("reported", {}).items()) or r.get("note", "")
            lines.append(f"| {r['feature']} | `{r['id'].split('/')[1]}` | {ICON[r['status']]} | {reported} |")
        lines.append("")
    lines += ["## Import performance", "", "| File | Input | Output | Import | Peak RSS | Reopen | Recalc / PDF | Notes |", "|---|---:|---:|---:|---:|---:|---:|---|"]
    for r in scale:
        if r["status"] != "ok":
            lines.append(f"| `{r['id']}` | {size(r['input_bytes'])} | | 💥 | | | | {r.get('note', '')} |")
            continue
        extra = f"{r['recalc_s']:.2f} s" if "recalc_s" in r else f"{r['pdf_s']:.2f} s, {r['pdf_pages']} pp" if "pdf_s" in r else "error"
        facts = ", ".join(f"{k} {v:,}" for k, v in r["report"].items())
        lines.append(f"| `{r['id']}` | {size(r['input_bytes'])} | {size(r['output_bytes'])} | {r['import_s']:.2f} s | {r['import_rss_mib']:.0f} MiB | "
                     f"{r.get('reopen_s', float('nan')):.2f} s | {extra} | {facts} |")
    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("corpus", nargs="?", default="target/craft-corpus")
    parser.add_argument("--grind", default=os.environ.get("GRIND_BIN", "grind"))
    parser.add_argument("--out", help="where imports and results go (default: CORPUS/grind)")
    parser.add_argument("--repeat", type=int, default=3, help="imports per scale file; the median is reported")
    parser.add_argument("--reference", action="append", default=[], help="another .xlsx/.docx to time alongside the tiers")
    parser.add_argument("--only", choices=["probes", "scale"])
    parser.add_argument("--expect", help="a JSON file of each probe's status; any probe that differs fails the run")
    parser.add_argument("--update-expect", action="store_true", help="rewrite --expect from this run instead")
    args = parser.parse_args()
    grind = shutil.which(args.grind) or args.grind
    root = Path(args.corpus)
    out = Path(args.out) if args.out else root / "grind"
    manifest = json.loads((root / "manifest.json").read_text())
    version = subprocess.run([grind, "--version"], capture_output=True, text=True).stdout.strip()
    results_path = out / "matrix.json"
    previous = json.loads(results_path.read_text()) if results_path.exists() else {}
    probes = run_probes(grind, root, manifest, out) if args.only in (None, "probes") else previous.get("probes", [])
    scale = run_scale(grind, root, manifest, out, args.repeat, args.reference) if args.only in (None, "scale") else previous.get("scale", [])
    out.mkdir(parents=True, exist_ok=True)
    results_path.write_text(json.dumps({"grind": version, "probes": probes, "scale": scale}, indent=1, ensure_ascii=False))
    (out / "matrix.md").write_text(markdown(probes, scale, version))
    print(out / "matrix.md")
    if args.expect and args.only in (None, "probes"):
        sys.exit(check_expectations(probes, Path(args.expect), args.update_expect))


RANK = ["failed", "silent-loss", "reported-loss", "altered", "carried"]


def check_expectations(probes, path, update):
    """Hold each probe to its recorded status, both ways: a worse one is a regression, a better
    one is a promise to keep — record it with --update-expect. Craft's own refusals and
    absences are recorded too, so a Craft upgrade that changes them is noticed."""
    found = {r["id"]: r["status"] for r in probes}
    if update:
        path.write_text(json.dumps(dict(sorted(found.items())), indent=1) + "\n")
        print(f"recorded {len(found)} probes in {path}")
        return 0
    expected = json.loads(path.read_text())
    problems = []
    for pid in sorted(set(found) | set(expected)):
        was, now = expected.get(pid), found.get(pid)
        if was == now:
            continue
        if was in RANK and now in RANK and RANK.index(now) > RANK.index(was):
            problems.append(f"better: {pid} is {now}, recorded {was} — run with --update-expect to keep it")
        else:
            problems.append(f"changed: {pid} is {now}, recorded {was}")
    for line in problems:
        print(line, file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    main()
