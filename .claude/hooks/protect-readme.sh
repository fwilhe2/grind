#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later

# PreToolUse hook: no agent writes the root README. It is authored by hand in README.fodt and
# exported to README.md from grind-text-gtk (File ▸ Save and Export Markdown, Ctrl+Alt+S).
# An agent may *read* both and *tell* the author what is wrong or out of date; it may not
# change either. Exit 2 blocks the tool call and hands stderr back to the agent.
#
# This is a tripwire, not a sandbox: `.claude/settings.json` denies Edit/Write outright, and
# this catches the shell spellings of a write. A script that opens the file by a computed name
# gets past it — CLAUDE.md's rule is what covers that.

input=$(cat)
tool=$(printf '%s' "$input" | sed -n 's/.*"tool_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')
root=${CLAUDE_PROJECT_DIR:-$(pwd)}

block() {
    echo "README.md and README.fodt are the author's, written by hand in grind-text." >&2
    echo "Do not edit, regenerate, restore, move or delete them. If something in them is" >&2
    echo "wrong or out of date, say so in your reply and leave the change to the author." >&2
    exit 2
}

case "$tool" in
Edit | Write | MultiEdit | NotebookEdit)
    path=$(printf '%s' "$input" | sed -n 's/.*"\(file_path\|notebook_path\)"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\2/p')
    case "$path" in
    "$root/README.md" | "$root/README.fodt" | README.md | README.fodt | ./README.md | ./README.fodt) block ;;
    esac
    ;;
Bash)
    # The root README by name: bare, `./`, or under the project root. `suite/README.md` and
    # the corpus's are other files and stay writable.
    names="(^|[^/[:alnum:]_.-])(\./)?README\.(md|fodt)|${root//./\\.}/README\.(md|fodt)"
    printf '%s' "$input" | grep -Eq "$names" || exit 0
    writes='>[[:space:]]*[^[:space:]&]*README|\btee\b|\bsed\b[^|;&]*[[:space:]]-[[:alpha:]]*i|\bperl\b[^|;&]*[[:space:]]-[[:alpha:]]*i|\b(mv|cp|rm|ln|truncate|dd|touch|install|sponge|patch)\b|\bgit[[:space:]]+(checkout|restore|rm|mv|stash|reset|apply|am|cherry-pick|revert)\b|export-md|\bconvert\b|\btext[[:space:]]+(type|erase|split|join|set-kind|format|import-md)\b'
    printf '%s' "$input" | grep -Eq "$writes" && block
    ;;
esac
exit 0
