#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later

# Run the tests that ask Euro-Office (loop D's second oracle) against the image pinned in
# ci/euro-office-image. Needs Docker; arguments go to `cargo test` after the test selection.
#
#   scripts/euro-office-tests.sh
#   scripts/euro-office-tests.sh -- --nocapture
set -euo pipefail
cd "$(dirname "$0")/.."

docker pull -q "$(cat ci/euro-office-image)"
export PATH="$PWD/scripts/euro-office-docker:$PATH"
exec cargo test -p grind-xlsx --test loop_d_euro_office "$@"
