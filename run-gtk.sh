#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later
#

cargo build
SHEET=target/debug/sheet examples/sample.sh /tmp/demo
cargo run -p sheet-gtk -- /tmp/demo/sample.fods

