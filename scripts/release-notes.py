#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# release-notes.py — CHANGELOG.md's `## Unreleased` section, at a release (.github/workflows/release.yml):
#
#   scripts/release-notes.py cut 1.2.0 notes.md owner/repo
#       # the release page's notes to notes.md: where to download, then the section with empty
#       # headings dropped; in CHANGELOG.md the section's heading becomes `## 1.2.0 — <date>`
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


def downloads(version: str, repo: str) -> str:
    """Which file to download on each platform, linked; the names are release.yml's."""
    base = f"https://github.com/{repo}/releases/download/v{version}"
    tap = f"{repo.split('/')[0]}/grind"

    def link(label: str, name: str) -> str:
        return f"[{label}]({base}/{name})"

    def cli(target: str, ext: str) -> str:
        return link(target.split("-")[0], f"grind-v{version}-{target}.{ext}")

    def pkg(label: str, name: str) -> str:
        debs = " · ".join(link(a, f"{name}_{version}-1_{a}.deb") for a in ("amd64", "arm64"))
        rpms = " · ".join(link(a, f"{name}-{version}-1.{a}.rpm") for a in ("x86_64", "aarch64"))
        return f"| {label} | {debs} | {rpms} |"

    pkgs = "\n".join(
        pkg(label, name)
        for label, name in [
            ("Spreadsheet", "grind-sheet-gtk"),
            ("Word processor", "grind-text-gtk"),
            ("Command line", "grind-cli"),
            ("Terminal version", "grind-tui"),
        ]
    )

    return f"""## Download

**macOS**, Apple silicon and Intel: {link(f"Grind-{version}.pkg", f"Grind-{version}.pkg")} puts Grind in your own Applications folder. It is not notarized, so macOS refuses it the first time: open System Settings ▸ Privacy & Security and choose *Open Anyway*. To drag it in yourself instead: {link(f"Grind-{version}.dmg", f"Grind-{version}.dmg")}. With Homebrew, which `brew upgrade` keeps up to date: `brew install --cask {tap}/grind-app`.

**Windows**, x64 and Arm: {link(f"grind-{version}-x86_64.msi", f"grind-{version}-x86_64.msi")} installs Grind for you alone, no administrator needed: the app in the Start menu and in *Open with*, and `grind` and `grind-tui` on your `PATH`. Running a later release's installer updates it; *Installed apps* removes it. It is not signed, so SmartScreen asks first: *More info* ▸ *Run anyway*. To install or update from PowerShell, without asking:

```powershell
iwr {base}/grind-{version}-x86_64.msi -OutFile $env:TEMP\\grind.msi; msiexec /i $env:TEMP\\grind.msi /qn
```

The app alone, nothing to install: {link(f"grind-win32-{version}-x86_64.exe", f"grind-win32-{version}-x86_64.exe")}.

**Linux**, x86_64 and Arm: one package per program, Debian and Ubuntu take the `.deb`, Fedora the `.rpm`. Download the ones you want and install them together.

| | Debian, Ubuntu | Fedora |
|---|---|---|
{pkgs}

All of them in one command:

```sh
v={version} u={base}
# Debian, Ubuntu
cd "$(mktemp -d)" && curl -fL --remote-name-all "$u"/grind{{,-cli,-tui,-sheet-gtk,-text-gtk}}_$v-1_$(dpkg --print-architecture).deb && sudo apt install ./*.deb
# Fedora
sudo dnf install "$u/grind-$v-1.noarch.rpm" "$u"/grind-{{cli,tui,sheet-gtk,text-gtk}}-$v-1.$(uname -m).rpm
```

**Only the command line**, no spreadsheet or word processor: `grind` and `grind-tui`, with Homebrew on macOS and Linux, kept up to date by `brew upgrade`:

```sh
brew install {tap}/grind
```

Or in one archive, nothing to install (the Linux ones run on any distribution):

| Linux | macOS | Windows |
|---|---|---|
| {cli("x86_64-unknown-linux-musl", "tar.gz")} · {cli("aarch64-unknown-linux-musl", "tar.gz")} | {cli("aarch64-apple-darwin", "tar.gz")} · {cli("x86_64-apple-darwin", "tar.gz")} | {cli("x86_64-pc-windows-msvc", "zip")} · {cli("aarch64-pc-windows-msvc", "zip")} |

**In the browser**, a folder of documents served at http://127.0.0.1:8080/ (Linux):

```sh
docker run --rm --network host --user "$(id -u):$(id -g)" -v ~/Documents:/documents ghcr.io/{repo}-web:{version}
```

**VS Code**: {link(f"grind-vscode-{version}.vsix", f"grind-vscode-{version}.vsix")}, for `.grind` and `.rhai` files, with `grind` on your `PATH`.

Every file is attested as built from this tag: `gh attestation verify <file> -R {repo}`.

"""


def demote(text: str) -> str:
    """Headings one level down, under the release page's own `## What changed`."""
    return "\n".join("#" + line if line.startswith("### ") else line for line in text.splitlines()) + "\n"


def cut(version: str, out: pathlib.Path, repo: str) -> None:
    lines = CHANGELOG.read_text().splitlines()
    start, end = section(lines)
    out.write_text(downloads(version, repo) + "## What changed\n\n" + demote(notes(lines[start + 1 : end])))
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
        case ["cut", version, out, repo]:
            cut(version, pathlib.Path(out), repo)
        case ["open"]:
            open_()
        case _:
            sys.exit("usage: release-notes.py cut <version> <out> <owner/repo> | release-notes.py open")
