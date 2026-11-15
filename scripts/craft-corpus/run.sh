#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later

# Build the Craft corpus into OUT (default target/craft-corpus) and import it with grind
# (GRIND_BIN, default `grind`), holding each probe to expected.json. Any further arguments go
# to compat_matrix.py — `--update-expect` records a new baseline.
set -euo pipefail
here=$(dirname "$(readlink -f "$0")")
out=${1:-target/craft-corpus}
shift || true
python3 -I "$here/large_corpus.py" "$out"
python3 -I "$here/compat_matrix.py" "$out" --expect "$here/expected.json" "$@"
