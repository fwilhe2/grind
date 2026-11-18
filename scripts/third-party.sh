#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later

# Bring every open Dependabot pull request back to green as far as the third-party list goes.
# Run it after merging one of them, or when dependabot-third-party.yml failed. Needs `gh`.
#
#   scripts/third-party.sh          # every open Dependabot PR
#   scripts/third-party.sh 88 91    # just these
#
# Per pull request:
# - Behind main: asks Dependabot to `recreate` it. Dependabot will not rebase a branch that
#   carries the workflow's commit, and resolving Cargo.lock by hand is what recreating avoids.
#   The fresh branch triggers the workflow again, which commits the list.
# - On top of main: regenerates `core/src/third_party/data.rs` in a scratch worktree (with this
#   checkout's target/ as the cache), and pushes it as a commit if it changed.
#
# Merging several: merge one, run this, wait for the recreated PRs to go green, merge the next.
set -euo pipefail

root=$(git rev-parse --show-toplevel)
cd "$root"
repo=$(gh repo view --json nameWithOwner -q .nameWithOwner)
data=core/src/third_party/data.rs

if (($#)); then
    prs=("$@")
else
    mapfile -t prs < <(gh pr list --repo "$repo" --author app/dependabot --state open \
        --json number -q '.[].number')
fi
if ((${#prs[@]} == 0)); then
    echo "no open Dependabot pull requests"
    exit 0
fi

git fetch --quiet origin main

for pr in "${prs[@]}"; do
    branch=$(gh pr view "$pr" --repo "$repo" --json headRefName -q .headRefName)
    case "$branch" in
        dependabot/*) ;;
        *) echo "#$pr: $branch is not a Dependabot branch, skipped"; continue ;;
    esac
    git fetch --quiet origin "+refs/heads/$branch:refs/remotes/origin/$branch"

    if ! git merge-base --is-ancestor origin/main "origin/$branch"; then
        echo "#$pr: behind main, asking Dependabot to recreate it"
        gh pr comment "$pr" --repo "$repo" --body "@dependabot recreate" >/dev/null
        continue
    fi

    work=$(mktemp -d)
    git worktree add --quiet --detach "$work" "origin/$branch"
    (
        cd "$work"
        log=$(mktemp)
        if CARGO_TARGET_DIR="$root/target" cargo test --locked -p grind-core \
            --test third_party the_third_party_list_is_current >"$log" 2>&1; then
            echo "#$pr: list already current"
        elif git diff --quiet -- "$data"; then
            tail -n 30 "$log"
            echo "#$pr: the test failed without rewriting the list (full log: $log)"
        else
            git add -- "$data"
            git commit --quiet -m "Regenerate the third-party list"
            git push --quiet origin "HEAD:refs/heads/$branch"
            echo "#$pr: list regenerated and pushed"
        fi
    )
    git worktree remove --force "$work"
done
