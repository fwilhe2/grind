#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Build a text document that uses **every feature this build supports**, through the CLI and
# nothing else. `examples/sample.sh` is this script's spreadsheet twin, and the rule is the
# same one: a feature that lands without a line here is a feature nobody can see.
#
#   examples/sample-text.sh [output-directory]        # default: ./sample
#   GRIND=target/debug/grind examples/sample-text.sh  # a binary other than the one on PATH
#
# `cli/tests/cli.rs` runs it and fails the build if it stops working.
#
# What it deliberately cannot show, because nothing can do it yet: undo and redo (no session
# for text — see doc/cli-parity-text.md), tables, footnotes and fields.
#
# What it cannot show because a script is the wrong medium: R6. Editing one paragraph of this
# document changes one line of `git diff`, and opening it to read changes nothing at all —
# `text/tests/diffable.rs` is where that is held.

set -euo pipefail

GRIND=${GRIND:-grind}
out=${1:-sample}
mkdir -p "$out"
doc="$out/sample.fodt"
rm -f "$doc" "$out/sample.odt"

say() { printf '\n=== %s\n' "$1"; }

# The suite is one binary with an app under it, so every verb is `grind text <verb>`.
text() { "$GRIND" text "$@"; }
run() { text "$@" >/dev/null; }

# --- the document ---------------------------------------------------------------------------

say "a new document"
run new "$doc" --force

say "headings and paragraphs — the outline is implied by the levels, and nothing else"
run insert "$doc" --heading 1 --text 'Field Notes'
run insert "$doc" --text 'Written entirely from a shell, which is the point.'
run insert "$doc" --heading 2 --text 'What a block is'
run insert "$doc" --text 'A paragraph, a heading or a list item. The body is a flat sequence.'
run insert "$doc" --heading 2 --text 'Addresses'
run insert "$doc" --text 'p12 is a position. #name and §2.1 survive edits above them.'

say "list items, nested — the model flattens them and the writer folds them back"
run insert "$doc" --list --text 'by position'
run insert "$doc" --list 2 --text 'invalidated by an insert above'
run insert "$doc" --list --text 'by bookmark'
run insert "$doc" --list --text 'by outline path'

say "a second top-level section, so there is something to move"
run insert "$doc" --heading 1 --text 'Appendix'
run insert "$doc" --text 'Nothing to see here yet.'

say "spaces that XML would otherwise collapse, kept by re-encoding them as text:s"
run insert "$doc" --text 'columns:    one    two    three'

say "and the other two: a tab and a newline become text:tab and text:line-break"
run insert "$doc" --text "$(printf 'name\tvalue\nsecond line')"

say "text read from stdin, for the case where it is longer than a shell argument wants"
printf 'A paragraph piped in, so a script can write prose without quoting it.' \
    | run insert "$doc" --text -

# --- editing --------------------------------------------------------------------------------

say "set: replace a block's text by address"
run set "$doc" p2 'Written entirely from a shell, which is rather the point.'

say "kind: turn a paragraph into a heading, and back"
# Addressed relative to a *heading* rather than by number, and deliberately: the first draft
# of this script said p14, which was the position that block would have had if the two inserts
# above it had not happened. That is the failure mode `#name` and `§2.1` exist to avoid, and
# this script fell into it before the reader did.
run insert "$doc" '§1' --after --text 'Temporarily a heading'
run kind "$doc" p2 --heading 3
run kind "$doc" p2

say "delete: by range"
run delete "$doc" p2

say "style: a named paragraph style over a range"
run style "$doc" p2 --style 'Text_20_body'

say "name: a bookmark, which is the named-range analogue"
run name "$doc" addresses '§1.2'

say "move: a whole section, addressed by its outline path"
text move "$doc" '§2' '§1' >/dev/null

say "replace: every occurrence, one undo step"
run replace "$doc" 'rather the point' 'the point'

# --- reading ---------------------------------------------------------------------------------

say "outline: every heading, its level, and the address that finds it again"
text outline "$doc"

say "outline --filter"
text outline "$doc" --filter Address

say "view --marks: address and kind per block"
text view "$doc" --marks

say "view of one section, by outline address"
text view "$doc" '§2.2'

say "get: one block, by bookmark"
text get "$doc" '#addresses'

say "find: every occurrence, with an address per hit"
text find "$doc" 'flat'

say "formatting: every block carrying a style of its own"
text formatting "$doc"

say "name with no name: every bookmark"
text name "$doc"

say "words: what a status bar shows"
text words "$doc"

# --- the suite level --------------------------------------------------------------------------

say "info: the kind is read out of the file, not guessed from its name"
"$GRIND" info "$doc"

say "convert: the same document as a package"
"$GRIND" convert "$doc" "$out/sample.odt" >/dev/null
"$GRIND" info "$out/sample.odt"

printf '\n%s and %s\n' "$doc" "$out/sample.odt"
