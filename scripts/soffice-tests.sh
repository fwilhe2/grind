#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later

# Run the soffice-dependent tests (loop C, loop E) against the exact LibreOffice CI pins
# (ci/libreoffice-image, a container image by digest) rather than whatever a developer's own
# machine has installed — see "Pinning LibreOffice" in doc/differential-fuzz.md for why the
# version matters here. Needs Docker.
#
# The cargo build is the host's, so the usual target/ cache applies and only `soffice` comes
# out of the container (scripts/soffice-docker/soffice does that part).
#
#   scripts/soffice-tests.sh                               # loop C + loop E, pinned version
#   scripts/soffice-tests.sh --test loop_e -- --nocapture  # one test, extra args passed on
#
# Note: with no arguments this runs *both* applications' loop C, because both `grind-sheet` and
# `grind-text` have a `roundtrip` target. The pinned image is currently Calc-only, so the text
# one skips its soffice-backed half with a notice — see `oracle_ready` in
# text/tests/roundtrip.rs.
set -euo pipefail
cd "$(dirname "$0")/.."

args=("$@")
if [ ${#args[@]} -eq 0 ]; then
    # No `-p`: loop C belongs to both applications, and loop E only exists in `grind-sheet`.
    args=(--test roundtrip --test loop_e)
fi

docker pull -q "$(cat ci/libreoffice-image)"
export PATH="$PWD/scripts/soffice-docker:$PATH"
exec cargo test "${args[@]}"
