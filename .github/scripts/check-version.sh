#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later

# check-version.sh <version> <command…>
#
# Fails unless `<command…> --version` says it is a release build of <version>, from the commit
# checked out here, with nothing changed in the tree: what a binary built from a release's tag
# must say about itself.
set -euo pipefail

version="${1:?usage: check-version.sh <version> <command…>}"
shift
commit="$(git rev-parse HEAD)"

said="$("$@" --version | tr -d '\r')"
echo "$said"

first="$(sed -n 1p <<< "$said")"
case "$first" in
    *" $version (release build)") ;;
    *) echo "::error::$1 says \"$first\", not $version (release build)"; exit 1 ;;
esac

stamp="$(sed -n 's/^commit: \([0-9a-f]*\) (\(.*\) git tree)$/\1 \2/p' <<< "$said")"
read -r short tree <<< "$stamp"
if [ -z "${short:-}" ] || [ "${commit#"$short"}" = "$commit" ]; then
    echo "::error::$1 was built from commit ${short:-unknown}, not $commit"
    exit 1
fi
if [ "$tree" != clean ]; then
    echo "::error::$1 was built from a $tree tree"
    exit 1
fi
