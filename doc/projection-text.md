<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# The projection's grammar — `grind text`

> **1:1 with the subset this build supports — and the subset is still evolving.**
> — `doc/dsl.md` §3.7

The twin of `doc/projection-sheet.md`, one per application by **R9**, and the one where §3.7's
promise is kept *literally*. That section says the projection's vocabulary is checked "against
the same two lists"; the spreadsheet turned out to have neither, and is checked against its
model instead. A text document has one: `doc/text-core.md`'s element table is a real scope line,
already parsed by `text/tests/scope.rs` and already held against `grind_text::implemented()`.
So the rule here is the one §3.7 actually wrote down —

> An element that enters the scope line without a projection spelling fails the build, and a
> projection spelling with no element behind it fails it too.

`text/tests/projection_scope.rs` is that test, four ways:

1. **Every element `grind_text::implemented()` returns has a spelling here** — a node, a piece
   of inline notation, or a `gap:` with a reason. Nothing may be absent.
2. **Every node this document names is one the writer emits**, read out of
   `text/src/projection/write.rs`, and every node the writer emits is named here.
3. **Every example parses**, with a `grind text` header on it.
4. **Every example still holds its spelling after a round trip** — read it, project the model it
   produced, and the spelling has to come back. This is what separates *accepted* from
   *carried*: something the reader parses and then throws away passes (3) and fails here.

## The blocks

One node per block, and the body is **flat** — `model.rs`'s founding schema fact (rng:16938): a
`text:h` does not contain the paragraphs under it, so a heading's level and a list item's depth
are numbers rather than nesting.

| Node | Carries | Example |
|---|---|---|
| `p` | `BlockKind::Paragraph`, with the block's runs as its string | `p "an ordinary paragraph"` |
| `h` | `BlockKind::Heading`, its `text:outline-level` as the first argument | `h 2 "Addresses"` |
| `li` | `BlockKind::ListItem`, its nesting depth as the first argument | `li 2 "a nested item"` |
| `list` | **read only** — the authoring spelling for a run of `li`, which supplies their depth by nesting. The writer never emits one, because the model is flat | `list { li "in a list" }` |

`style=` on any of the three is `Block::style`, the block's `text:style-name` kept as a *name*:
`p "code" style="Preformatted Text"`.

## The inline notation

Inside the string. `text/src/projection/inline.rs` is normative for it, and its marker table is
`markdown.rs`'s — `**` means bold in this file format because it already means bold while you
type, and a fourth reading of `**` is exactly what `doc/text-core.md` argues against.

| Element | Spelling | Example |
|---|---|---|
| character data | the characters | `p "words"` |
| `text:span` | one emphasis as markers, anything else as `[text]{…}` | `p "a **bold** word"` |
| `text:s` | interior spaces, kept — KDL does not collapse whitespace and XML does | `p "two  spaces"` |
| `text:tab` | `\t` | `p "name\tvalue"` |
| `text:line-break` | `\n` — visibly different from a new *block*, which is a new node | `p "one\ntwo"` |
| `text:a` | `[text](url)`, or `href=` in the attribute form when the run is formatted too | `p "the [site](https://example.org)"` |
| `text:bookmark` | `{#name}` — an anchor, zero characters wide | `p "{#intro}the introduction"` |
| `draw:frame` | gap: **images**, and the section below is why | |

The attribute form spells every property `CharStyle` carries. The four switch-shaped ones are
`#true` when they hold the value the markers would have produced and a string otherwise, so an
unusual weight is still expressible rather than unreachable:

```
[x]{bold=#true italic=#true underline=#true strike=#true}
[x]{weight=600 slant=oblique underline=dotted}
[x]{family="Liberation Serif" size=14pt color=#ff4136 background=#ffff00}
[x]{style=Emphasis href=https://example.org}
```

A **raw** KDL string turns the whole notation off — `p #"a literal ** stays literal"#` — which
is how a paragraph *about* markdown is written (`doc/dsl.md` §3.5). It is the one place in this
project where how a value was *spelled* carries meaning and not only how it reads.

## Images — the one named gap

`doc/dsl.md` §3.8 answered this one before it was built: write `image "figures/plot.png"` and
keep the bytes in a **sidecar directory** beside the file, because base64 in a format whose
selling point is that a human reads it is absurd. That answer was correct when it was written
and is not available now, for a reason that arrived after it:

**D4 made the projection a `Form`.** A form is reached through `write_bytes` and `read_bytes` —
**bytes, and no path**. Rule 5 is that every `*_file` has a `*_bytes` twin, and `grind-web` is
that rule's honest test: a browser tab has no filesystem, so it cannot write a sidecar and
cannot read one. A physical form that only works when there is a directory to put things next
to is not a form this project can have.

So the choices are all real ones — a `data:` URI (honest and unreadable, and §3.8 rejected it),
a sidecar (needs a path), or a form that is bytes-only and drops images (this one) — and none of
them is obviously right. Choosing between them is a design question **D2 reopens rather than
answers**, and until it is answered a paragraph's image is dropped and the prose around it is
not. `text/tests/loop_f.rs`'s `images_are_the_one_named_gap` fails the day that changes.

## What a projection does not carry, and why that is not a gap

`Document::styles` — the set of style names the file this document was read from *declares* —
has no node here and needs none. It is read and never written, by anything: this build carries
style names and not style definitions (`doc/text-core.md`), so a `.grind` has nothing to declare
and an `.fodt` this build regenerates declares nothing either. The set exists so that
`grind text lint`'s `undeclared-style` rule can ask whether a name points at anything
(`doc/dsl.md` §4.3), and a document read back from its own projection reporting every style as
undeclared is that known loss stated out loud rather than a round-trip failure — loop F compares
blocks, runs and bookmarks, which is what the projection is bijective with.

## What this document is not

It is not the *design* — `doc/dsl.md` is, and it outranks this file on every question of why.
It is not the scope line either: `doc/text-core.md` is, and this document is checked against it
rather than the other way round. And it is not a place to record what the projection *will*
spell: a row here is a spelling that exists, and a gap is a row with a reason on it.
