#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later

# Run the soffice-dependent tests (loop C, loop E) against the exact LibreOffice CI pins
# (ci/libreoffice-version, an Ubuntu 24.04 package version) rather than whatever a
# developer's own machine has installed — see "Pinning LibreOffice" in
# doc/differential-fuzz.md for why the version matters here. Needs Docker; a named volume
# keeps the Rust toolchain and target/ across runs so only the first one is slow.
#
#   scripts/soffice-tests.sh                              # loop C + loop E, pinned version
#   scripts/soffice-tests.sh --test loop_e -- --nocapture  # one test, extra args passed on
set -euo pipefail
cd "$(dirname "$0")/.."

version=$(cat ci/libreoffice-version)
args=("$@")
if [ ${#args[@]} -eq 0 ]; then
    args=(--test roundtrip --test loop_e)
fi

exec docker run --rm \
    -v "$PWD:/repo" \
    -v "sheet-soffice-tests-cargo:/usr/local/cargo" \
    -v "sheet-soffice-tests-rustup:/usr/local/rustup" \
    -v "sheet-soffice-tests-target:/repo/target" \
    -w /repo \
    -e DEBIAN_FRONTEND=noninteractive \
    -e CARGO_HOME=/usr/local/cargo \
    -e RUSTUP_HOME=/usr/local/rustup \
    ubuntu:24.04 \
    bash -c "
        set -e
        apt-get update -qq
        apt-get install -y --no-install-recommends curl build-essential ca-certificates \
            'libreoffice-calc=$version' >/dev/null
        export PATH=\$CARGO_HOME/bin:\$PATH
        [ -x \$CARGO_HOME/bin/cargo ] || \
            curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable
        cargo test -p sheet-core ${args[*]}
    "
