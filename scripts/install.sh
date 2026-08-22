#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Install the packages CI just built, over whatever is installed now.
#
#   scripts/install.sh [deb|rpm] [package ...]
#
# There is no release process yet: the .deb and .rpm files are artifacts of the
# `packaging` workflow's last green run on main (.github/workflows/packaging.yml),
# and every build carries the same version number. So this forces the install —
# same version over same version is the normal case here, not an accident.
#
# Needs the GitHub CLI, logged in: the artifacts are only reachable with a token.
# With no arguments it installs every package for this system's format.

set -euo pipefail

repo="${SHEET_REPO:-fwilhe2/sheet}"
format="${1:-}"
if [ -z "$format" ]; then
    if command -v dpkg >/dev/null; then format=deb; else format=rpm; fi
fi
shift || true

case "$format" in
deb | rpm) ;;
*)
    echo "usage: $0 [deb|rpm] [package ...]" >&2
    exit 2
    ;;
esac

run=$(gh run list -R "$repo" --workflow packaging.yml --branch main \
    --status success --limit 1 --json databaseId -q '.[0].databaseId')
if [ -z "$run" ]; then
    echo "no green packaging run on main to install from" >&2
    exit 1
fi

dir=$(mktemp -d)
# `mktemp -d` is 0700, which apt's unprivileged `_apt` user cannot read through —
# it drops privileges even for a local file and warns loudly when it cannot.
chmod 755 "$dir"
trap 'rm -rf "$dir"' EXIT
echo "downloading $format packages from run $run"
gh run download -R "$repo" "$run" --name "$format-packages" --dir "$dir"
chmod a+r "$dir"/*."$format"

# No package named: everything the run built.
files=()
if [ "$#" -eq 0 ]; then
    files=("$dir"/*."$format")
else
    for package in "$@"; do
        # One package per name, whatever version string CI stamped on it.
        found=("$dir/$package"[-_]*."$format")
        if [ ! -e "${found[0]}" ]; then
            echo "no $package package in that run" >&2
            exit 1
        fi
        files+=("${found[@]}")
    done
fi

for file in "${files[@]}"; do echo "installing $(basename "$file")"; done
case "$format" in
# `--reinstall` is the force: apt refuses a version it already has, and every
# build here has the same one. `--allow-downgrades` covers a run older than what
# is installed, which is what testing a branch's package looks like.
deb) sudo apt-get install -y --reinstall --allow-downgrades "${files[@]}" ;;
rpm) sudo rpm -Uvh --force "${files[@]}" ;;
esac
