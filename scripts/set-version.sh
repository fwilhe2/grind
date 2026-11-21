#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# scripts/set-version.sh <version> — the suite's version, everywhere it is written:
#
#   * the workspace version every crate inherits, through `cargo set-version` (cargo-edit);
#   * the meta-package's pins in suite/Cargo.toml, which no tool reads from the workspace:
#     the version as artifacts.yml packages it, `-dev` as `~dev` so that it sorts before the
#     release in dpkg and rpm alike, and the release `-1` both packagers append;
#   * Cargo.lock, whose entries for the workspace's own crates carry the version too.
#
# cli/tests/packaging.rs fails when the pins and the workspace disagree.

set -euo pipefail

version="${1:?usage: scripts/set-version.sh <version>}"
cd "$(dirname "$0")/.."

cargo set-version --workspace "$version"
pinned="${version/-/\~}-1"
sed -i -E \
    -e "s/(grind-[a-z-]+ \(= )[^)]*\)/\1${pinned})/g" \
    -e "s/^(grind-[a-z-]+ = \"= )[^\"]*\"/\1${pinned}\"/" \
    suite/Cargo.toml
cargo update --workspace
