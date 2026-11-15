#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later

# Install the pinned WordCraft and GridCraft packages, each checked against its published
# SHA-256 before apt sees it. The one place the versions are pinned: the Dockerfile and CI both
# run this. Needs curl and apt; uses sudo when not root.
set -euo pipefail

wordcraft_version=0.18.1
wordcraft_sha256=9b557161ded7b5973112c8c21f600fe0b0110c1a4d3151184485733c96f87e19
gridcraft_version=0.19.1
gridcraft_sha256=3e9b093ee3bed8b1f008ea56c8d60e6a96890e24317be2bbb93e431f3c9d8112

sudo=""
[ "$(id -u)" = 0 ] || sudo=sudo
dir=$(mktemp -d)
trap 'rm -rf "$dir"' EXIT
cd "$dir"

fetch() { # name version sha256
    local deb="$1-$2-linux-x86_64.deb"
    curl -fsSLO "https://github.com/storytold/$1/releases/download/v$2/$deb"
    echo "$3  $deb" | sha256sum --check --status
}
fetch wordcraft "$wordcraft_version" "$wordcraft_sha256"
fetch gridcraft "$gridcraft_version" "$gridcraft_sha256"
$sudo apt-get update -q
$sudo apt-get install -y -q --no-install-recommends ./*.deb
