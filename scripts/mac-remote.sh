#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# mac-remote.sh — the remote loop (doc/macos-shell.md, M11): a Mac, from a machine that is not
# one, compiling nothing. Needs `gh`, signed in to an account that may run this repository's
# workflows.
#
#   scripts/mac-remote.sh renders [branch]
#       The frames the latest artifacts.yml run drew on that branch — every R7 document, the
#       samples and the welcome window, in both appearances, on macOS 15 and 26 — into
#       ./mac-frames/<run>/, and what mac-frames.sh and mac-bundle.sh said about them.
#
#   scripts/mac-remote.sh drive <script> [document] [branch]
#       The script run against that branch's latest Grind.app by mac-drive.yml, waited for, and
#       its transcript printed and its snapshots put in ./mac-drive/<run>/. The document is a
#       path in the repository, or empty for a new spreadsheet, or `text` for a new page.
#
#   scripts/mac-remote.sh session [minutes]
#       M12's screen-sharing session (mac-session.yml), up for at most an hour; connect with any
#       VNC client to vnc://grind-mac-session over the tailnet once it says so.
#
# The branch defaults to the one checked out here.

set -euo pipefail

here="$(git rev-parse --abbrev-ref HEAD)"
repo=""

# The repository on GitHub — asked of `gh` only by a command that needs it, so the usage text
# needs nothing signed in.
need_repo() {
    repo="$(gh repo view --json nameWithOwner -q .nameWithOwner)" \
        || die "gh cannot see this repository; is it signed in?"
}

die() {
    echo "mac-remote: $*" >&2
    exit 1
}

# The newest run of `workflow` on `branch` — after `since` (an ISO time) when given.
newest() {
    local workflow="$1" branch="$2" since="${3:-}"
    gh run list -R "$repo" -w "$workflow" -b "$branch" -L 10 \
        --json databaseId,createdAt \
        -q "[.[] | select(.createdAt > \"$since\")][0].databaseId // empty"
}

# Every annotation a run's jobs left — what a job says when its log needs credentials to read.
annotations() {
    local run="$1"
    for job in $(gh api "repos/$repo/actions/runs/$run/jobs" -q '.jobs[].id'); do
        gh api "repos/$repo/check-runs/$job/annotations" \
            -q '.[] | "== \(.title // "")\n\(.message)\n"'
    done
}

case "${1:-}" in
    renders)
        need_repo
        branch="${2:-$here}"
        run="$(newest artifacts.yml "$branch")"
        [ -n "$run" ] || die "no artifacts.yml run on $branch"
        out="mac-frames/$run"
        mkdir -p "$out"
        for os in macos-15 macos-26; do
            gh run download "$run" -R "$repo" -n "grind-mac-frames-$os" -D "$out/$os" \
                || echo "mac-remote: run $run left no frames for $os"
        done
        annotations "$run"
        echo "frames: $out"
        ;;
    drive)
        script="${2:-}"
        [ -f "$script" ] || die "usage: mac-remote.sh drive <script> [document] [branch]"
        need_repo
        document="${3:-}"
        branch="${4:-$here}"
        since="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
        gh workflow run mac-drive.yml -R "$repo" --ref "$branch" \
            -f script="$(cat "$script")" -f document="$document" -f branch="$branch"
        run=""
        for _ in $(seq 1 30); do
            run="$(newest mac-drive.yml "$branch" "$since")"
            [ -n "$run" ] && break
            sleep 2
        done
        [ -n "$run" ] || die "the drive did not start"
        gh run watch "$run" -R "$repo" --exit-status > /dev/null || true
        out="mac-drive/$run"
        mkdir -p "$out"
        gh run download "$run" -R "$repo" -n mac-drive -D "$out" || true
        if [ -f "$out/transcript.txt" ]; then
            cat "$out/transcript.txt"
        else
            annotations "$run"
        fi
        echo "snapshots: $out"
        ;;
    session)
        need_repo
        minutes="${2:-45}"
        gh workflow run mac-session.yml -R "$repo" --ref "$here" -f minutes="$minutes"
        echo "requested; when its job says so, connect to vnc://grind-mac-session over the tailnet"
        ;;
    *)
        sed -n '6,24p' "$0" | sed 's/^# \{0,1\}//'
        exit 2
        ;;
esac
