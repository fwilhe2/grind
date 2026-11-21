#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# release-notes.py — CHANGELOG.md's `## Unreleased` section, at a release (.github/workflows/release.yml):
#
#   scripts/release-notes.py cut 1.2.0 notes.md   # the section, empty headings dropped, to notes.md,
#                                                 # and its heading becomes `## 1.2.0 — <date>`
#   scripts/release-notes.py open                 # a fresh, empty `## Unreleased` on top

import datetime
import pathlib
import sys

CHANGELOG = pathlib.Path(__file__).resolve().parent.parent / "CHANGELOG.md"
UNRELEASED = "## Unreleased"
TEMPLATE = [UNRELEASED, "", "### Breaking changes", "", "### Changes", "", "### Fixes", ""]


def section(lines: list[str]) -> tuple[int, int]:
    """The Unreleased section's heading line and the line after its last."""
    start = lines.index(UNRELEASED)
    end = next(
        (i for i in range(start + 1, len(lines)) if lines[i].startswith("## ")), len(lines)
    )
    return start, end


def notes(body: list[str]) -> str:
    """The body with every heading that has nothing under it left out."""
    out: list[str] = []
    pending: list[str] = []
    for line in body:
        if line.startswith("### "):
            pending = ([""] if out else []) + [line, ""]
        elif line.strip():
            out += pending
            pending = []
            out.append(line)
    text = "\n".join(out).strip()
    return (text or "No changes a user would notice.") + "\n"


def cut(version: str, out: pathlib.Path) -> None:
    lines = CHANGELOG.read_text().splitlines()
    start, end = section(lines)
    out.write_text(notes(lines[start + 1 : end]))
    # The release's notes as published, empty headings and all removed.
    released = notes(lines[start + 1 : end]).rstrip("\n").splitlines()
    lines[start:end] = [f"## {version} — {datetime.date.today().isoformat()}", "", *released, ""]
    CHANGELOG.write_text("\n".join(lines) + "\n")


def open_() -> None:
    lines = CHANGELOG.read_text().splitlines()
    if UNRELEASED in lines:
        return
    first = next(i for i, line in enumerate(lines) if line.startswith("## "))
    lines[first:first] = TEMPLATE
    CHANGELOG.write_text("\n".join(lines) + "\n")


if __name__ == "__main__":
    match sys.argv[1:]:
        case ["cut", version, out]:
            cut(version, pathlib.Path(out))
        case ["open"]:
            open_()
        case _:
            sys.exit("usage: release-notes.py cut <version> <out> | release-notes.py open")
