#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
#
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Build a text document that uses **every feature this build supports**, through the CLI and
# nothing else. `examples/sample-sheet.sh` is this script's spreadsheet twin, and the rule is the
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

# --- editing at a caret ------------------------------------------------------------------------
#
# What a cursor does, as opposed to what a script does. The CLI has these because rule 4 has no
# exception for the operations that feel like they belong to a UI — and because a shell that had
# them and the CLI did not would be a bug.

say "type: characters at a caret — find prints an address you can hand straight back"
run insert "$doc" --text 'One keystroke is line of git diff.'
run type "$doc" "$(text find "$doc" 'line of' | cut -f1)" 'one '

# An offset composes with *any* spelling of a block, and that is the whole reason it is a
# separate part of an address rather than part of the p12 spelling: #typed+3 means the same
# place next week, and p15+3 stops meaning it the moment anything is inserted above. The
# argument that makes §2.1 worth having, one level further down.
say "a bookmark turns a caret into one that survives an edit above it"
run name "$doc" typed "$(text find "$doc" 'One keystroke' | cut -f1)"
# Typing at the anchor puts the text *before* it, so the anchor stays attached to the words it
# named rather than to a character count. #typed is now seven characters in.
run type "$doc" '#typed+0' 'Proof: '

say "split: the Return key — a block cut in two at an offset"
# Seven characters in, which is where the anchor now sits: the cut leaves "Proof: " behind and
# #typed follows its own text down into the second half.
run split "$doc" '#typed+7'

say "join: the Backspace key — and the first block's kind is the one that survives"
run join "$doc" "$(text find "$doc" 'Proof: ' | cut -f1)"

say "erase: characters rather than blocks, so the block itself stays behind"
run erase "$doc" '#typed+0:#typed+7'

say "style: a named paragraph style over a range"
run style "$doc" p2 --style 'Text_20_body'

# The other half of "styled", and a different thing: `style` names a style the *document*
# defines, `format` sets the properties directly. Only the second survives a document this
# build authored from nothing, because it needs no declaration it cannot write —
# doc/text-core.md's Styles section is the whole of that split.
say "format: direct character formatting over a span of characters"
run format "$doc" 'p2+0:p2+7' --bold
run format "$doc" 'p2+8:p2+16' --italic --color navy
run format "$doc" 'p4+0:p4+11' --font Georgia --size 13pt --underline

say "format --show: what a toolbar reads before it writes"
text format "$doc" 'p2+0:p2+7' --show

say "and over a mixed span, only what every character agrees about — here, nothing"
text format "$doc" 'p2+0:p2+17' --show

say "name: a bookmark, which is the named-range analogue"
run name "$doc" addresses '§1.2'

say "move: a whole section, addressed by its outline path"
text move "$doc" '§2' '§1' >/dev/null

say "replace: every occurrence, one undo step"
run replace "$doc" 'rather the point' 'the point'

# --- reading ---------------------------------------------------------------------------------

# --- layout ------------------------------------------------------------------------------------
#
# The line breaker is in the core (doc/text-layout.md, Path C), so the CLI can answer questions
# that are defined in terms of a line — which is exactly what a GUI's arrow keys ask. The CLI
# measures one unit per character; a GTK shell asks Pango and gets different numbers from the
# same engine.

say "view --width: the core breaks the lines, the CLI just prints them"
text view "$doc" '§2.2' --width 46

say "caret --down: the Down arrow, from a script"
text caret "$doc" 'p12+4' --down 1 --width 46

say "caret --home / --end: the visual line's ends, not the paragraph's"
text caret "$doc" 'p12+50' --home --width 46
text caret "$doc" 'p12+50' --end --width 46

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

# A bookmark contributes no characters, which makes it the one part of a text document a
# reader cannot see at all — `doc/view-modes.md` §3.6. This is what an overlay draws, in the
# only place a pipe has for one: written into the text at the offset it anchors to. It is a
# reading and not an edit; the document is byte-identical after it.

say "view --names: where each bookmark anchors, which is otherwise invisible"
text view "$doc" --names | sed -n '1,14p'

say "words: what a status bar shows"
text words "$doc"

# --- the suite level --------------------------------------------------------------------------

say "info: the kind is read out of the file, not guessed from its name"
"$GRIND" info "$doc"

say "convert: the same document as a package"
"$GRIND" convert "$doc" "$out/sample.odt" >/dev/null
"$GRIND" info "$out/sample.odt"

printf '\n%s and %s\n' "$doc" "$out/sample.odt"
