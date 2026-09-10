#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Screenshot `grind-tui` opened on one document. There is no `--render-to` here —
# unlike the two GTK shells and the Windows shell, `grind-tui` draws with ratatui's
# `TestBackend` in its own tests (a cell buffer, no terminal involved) and has no
# equivalent "draw one frame and exit" path of its own. So this drives a *real*
# terminal instead: `xterm` under a virtual X display, screenshotted from the
# outside with ImageMagick — the same two tools `doc/windows-shell.md` already
# names for driving the Wine window under Xvfb.
#
#   tui-screenshot.sh <grind-tui-binary> <document> <output.png> [columns] [rows]
#
# $DISPLAY must already point at a running X server (Xvfb or otherwise).

set -euo pipefail

bin=$1
document=$2
output=$3
cols=${4:-120}
rows=${5:-40}

if [ -z "${DISPLAY:-}" ]; then
    echo "tui-screenshot.sh: \$DISPLAY is not set — start Xvfb first" >&2
    exit 1
fi

xterm -geometry "${cols}x${rows}+0+0" -fa Monospace -fs 14 -bg black -fg white \
    -e "$bin" "$document" &
pid=$!

# Give ratatui a moment to draw its first frame — there is no readiness signal to
# poll for from outside a terminal, so this is a fixed wait rather than a loop.
sleep 2

import -window root "$output"

# `grind-tui` is left running (it is a normal interactive TUI with no `--once`
# flag); the screenshot is all this needs from it.
kill "$pid" 2>/dev/null || true
wait "$pid" 2>/dev/null || true
