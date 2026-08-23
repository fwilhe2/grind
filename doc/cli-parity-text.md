<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# CLI parity — `grind text`

> **Whatever any GUI can do, the CLI can do. A UI-only feature is a bug.**
> — `doc/plan.md` rule 4, `CONTRIBUTING.md` §4, and `doc/suite.md`'s **R9** once there is more
> than one application

This file is that rule made mechanical, **for the word processor**. Every public method of
`grind_text::App` appears below exactly once, with the command that reaches it or the reason it
is not reachable. `cli/tests/parity.rs` reads `text/src/lib.rs` and this file and fails the
build when they disagree.

One of these per application, never a second section in the spreadsheet's: two apps sharing one
document would pass every check while covering half as much, and `parity.rs` asserts the
separation directly.

Format: one bullet per method, `` - `method` — how ``. "not exposed:" must be followed by a
reason; the test checks that too, because an unexplained exemption is how a ratchet quietly
stops ratcheting.

**A note on addresses.** Every command taking a block takes `p12`, `p12+40`, `p12:p20`,
`#bookmark` or `§2.1.3` — `grind_text::loc`, the crate's only 0↔1 conversion. Where a command
takes a *range* and is given a bare heading address, it means that heading's **whole section**,
computed from outline levels; that is what makes `grind text move report.fodt §3.2 §1` mean
what a person expects.

## Editing

- `set_text` — `grind text set <at> <text>`, or `-` to read it from stdin. Keeps the block's
  kind, its style and any bookmark on it: an anchor is a position, not content, and losing one
  because its sentence was rewritten would make `#intro` useless the first time anyone edited
  the paragraph it names
- `insert` — `grind text insert [<at>] --text <text>`, with `--after`, `--heading <level>` and
  `--list [depth]`. No address appends, which is what building a document from a script does
  most of the time
- `delete` — `grind text delete <range>`
- `set_kind` — `grind text kind <at> --heading <level>` / `--list [depth]`, and neither flag
  makes it a paragraph. This is how a document gets its structure: the outline is implied by
  `text:outline-level` and nothing else
- `set_style` — `grind text style <range> --style <name>` (and no `--style` to clear one)
- `move_blocks` — `grind text move <range> <to>`, which is the verb `§2.1.3` addressing exists
  for
- `replace` — `grind text replace <needle> <replacement>`, one undo step for the whole document
- `set_bookmark` — `grind text name <name> <at>`, and `--delete` to remove one

## History

- `undo` — not exposed: **no session yet.** Each invocation loads the file, applies one command
  and writes it back, so history dies with the process unless it is carried in a file — which
  is what `grind sheet`'s `--session` does through `grind_sheet::Session`. Doing the same here
  means making `grind_text::Action` serialisable, and that is a decision about the model rather
  than about the CLI. No shell can undo a text document either, so this is not a parity gap
- `redo` — not exposed: the same reason, and it arrives with `undo` or not at all
- `can_undo` — `grind info`, and the `can_undo` field of every JSON report
- `can_redo` — `grind info`, and the `can_redo` field of every JSON report

## Documents

- `open_file` — every subcommand that takes a file
- `open_bytes` — not exposed: the CLI has a filesystem, and this is its twin for shells that do
  not (`doc/plan.md` rule 5). `grind convert` covers the same ground for a user
- `save_file` — `grind text new`, and every command that changes something
- `save_bytes` — not exposed: the `*_bytes` twin of `save_file`, for shells without a
  filesystem. Nothing a user can ask for is missing while `save_file` is here

## Reading

- `get_viewport` — `grind text view [<range>]`, one block per line; `--marks` prefixes each with
  its address and kind
- `block_count` — `grind text words`, and `grind info`
- `input_text` — `grind text get <at>`, which prints what an editor would show for the block:
  the text that, given back to `grind text set`, leaves it exactly as it is
- `resolve` — every command that takes an address; it is what turns `#intro` or `§2.1` into a
  block, against the document as it now is
- `resolve_range` — every command that takes a range
- `section` — the same commands, for a bare heading address: `grind text delete §3.2` removes
  the section rather than the heading line. Computed from outline levels, because the body has
  no such container
- `outline` — `grind text outline`, one line per heading with its `§` address, level and text.
  `--filter` narrows it. The spreadsheet's `calculations` for prose
- `formatting` — `grind text formatting`, every block carrying a style of its own. "Why is this
  paragraph different?", answered in one place
- `find` — `grind text find <needle>`, with a `p12+40` address per hit
- `counts` — `grind text words` — blocks, headings, words, characters
- `bookmarks` — `grind text name` with no name, which lists them all; and `grind text name
  <name>` prints where one is

## Not reachable, and why

- `new` — not exposed: constructing an `App` is what every subcommand does before anything
  else. `grind text new` writes an empty document, which is the user-visible half
- `set_observer` — not exposed: the core pushes changes to shells that stay running
  (`doc/plan.md` rule 3). A CLI process exits before a notification could matter, and there is
  no user-facing behaviour behind it

## Beyond `App`

Reachable from the CLI, but not `App` methods, so the test does not track them:

- `grind info` and `grind convert` — suite-level verbs, which read the document's kind out of
  the file (`grind_core::kind`) rather than trusting its name, and route accordingly
- `grind_text::loc` — the addressing module, this crate's only 0↔1 conversion. Free functions,
  used by every command that takes an address and by the outline's own `§` spellings
