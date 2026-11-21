<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

# Where layout lives

**Status: decided and built — Path C.** Line layout is in the core with font metrics injected;
pages are set by `grind-print` (`doc/page-model.md`). The five answers under "The decision" are
as normative as the path. On layout this document outranks `doc/suite.md`.

## Why the core owns lines

`CLAUDE.md`'s architecture rule is that all state and logic live in the core and every shell is
a renderer. Line layout looks like rendering and is not: **it is the editing model.**

| Operation | Defined in terms of |
|---|---|
| Down and Up | *the next line* |
| Home and End | *this line's* start and end |
| Page Up and Page Down | a screenful of *lines* |
| A click at (x, y) | hitting a *line box* |
| Shift+Down | both of the above |

A line is not in the ODF document; it is an output of layout. If layout were each shell's — Pango
in GTK, the browser's in the web shell, cell counting in the terminal — each shell would decide
where Down puts the caret, and they would disagree, not about pixels, which nobody minds, but
about the program's behaviour. And the CLI, which has no width, would have no answer for Down
at all (R9).

Owning lines is a different question from owning pages, and the two were once posed as one
choice ("Path A": neither in the core; "Path B": both, with a font stack). They separate. Path C
takes the first in full and asks the shell only what the core cannot know: how wide a piece of
text is.

## Path C

```rust
pub trait Metrics {
    fn advances(&self, text: &str, style: &TextStyle, out: &mut Vec<f32>);
    fn line_height(&self, style: &TextStyle) -> f32;
    // and a defaulted baseline, for whoever places glyphs itself
}
```

`grind_core::layout` holds it. The unit is the caller's own and the core never converts: the
terminal answers in cells, Pango in its units, the browser in CSS pixels, `grind-print` in
points.

**Cumulative advances, in one call.** `advance("a") + advance("b")` is not `advance("ab")` once
kerning and ligatures are involved, and measuring prefix after prefix to find a break is slow.
The whole array in one call is what a shaping engine produces anyway, is right across kerning
because the provider sees the whole string, and leaves the resulting `Layout` metric-free:
hit-testing and caret placement are array lookups, so a `Layout` is a plain value a shell can
hold. What is lost is kerning across a fragment boundary, which is a boundary between two
character styles.

The core owns, once, for every shell: break opportunities (UAX #14 through `unicode-linebreak`),
greedy line filling, alignment, hyphenation, the `Layout` type, and every caret operation in the
table above. A shell rasterises glyphs and answers `Metrics`.

| | Path A | Path B | **Path C** |
|---|---|---|---|
| Line breaking, caret by line, hit-test, selection | each shell | core | **core** |
| Font metrics and shaping | each shell | core | **behind a trait** |
| Core dependencies added | none | a font stack | **one Unicode table crate** |
| Terminal possible | yes | only if parameterised | **by construction** |
| The CLI can answer Down | no | yes | **yes** |

**What Path A had going for it**, and why it still lost: Pango and the browser break lines better
than this core does, above all for non-Latin scripts; a document looks native in each shell; and
the core stays dependency-free. All three are about rendering quality, and the objection is
about behaviour. Shells disagreeing about glyph positions is fine; shells disagreeing about where
Down goes is what the architecture exists to prevent. Path C leaves shaping to whatever does it
best and keeps the decisions that are the program's own.

**Testing.** Line breaking is tested against a provider where every character is one unit wide,
which makes it exactly assertable with no font anywhere. The property asked for is that shells
given the same document and width agree on where lines break; agreement with LibreOffice is a
question about pages, and is `print/tests/screen_is_paper.rs`' and the PDF's.

## The decision

### 1. Bidi — out, explicitly

Layout is **left-to-right only.** `unicode-bidi` is not taken and UAX #9 is not implemented; a
document containing right-to-left text lays out as though it were LTR, which is wrong for that
document rather than merely unstyled. It is affordable because of R6: such a document is still
read, carried byte for byte and written back correctly, so only the view is wrong.
`style:writing-mode` (rng:2864) is preserved and never consulted. The gate is a real RTL
document somebody wants to edit; then `unicode-bidi` goes in at the layout engine's seam and
caret movement in mixed-direction text — logical or visual — is answered with the document in
hand (`doc/not-doing.md` §2).

### 2. The font stack stays out of `grind-core`

`unicode-linebreak` (Apache-2.0, UAX #14, a pure table crate with no fonts and no I/O) is the
one dependency layout added to the core: neutral plumbing by this project's ladder, a Unicode
algorithm rather than ODF semantics. Fonts, shaping and hyphenation patterns live in
`grind-print` (`doc/pdf-export.md`), the only crate with a font stack.

### 3. `Layout` lives in `grind-core`, and the spreadsheet uses it

R8 holds because breaking styled text names no document type's vocabulary: the input is
`(text, TextStyle)` fragments, which a paragraph's runs and a wrapped cell's text both produce.
So one engine serves both applications: a cell's wrapped height and a paragraph's lines are the
same code, and every grid measures and draws a cell at one size through its shell's `Metrics`.

### 4. The CLI measures one unit per character

`grind text view --width 72` and `grind text caret` use `layout::Fixed`, which is what makes
every line operation reachable from the CLI. The core ships it because every test uses it too.
The terminal implements the trait itself over `unicode-width`, for CJK and combining marks, so
the core stays free of that table.

### 5. Every shell implements the trait once

| Caller | Its `Metrics` |
|---|---|
| A text document in any graphical shell, and the PDF | `grind_print::Typesetter` (`doc/page-model.md`) |
| The terminal | `Cells`, terminal columns |
| GTK's grid | Pango |
| The browser's grid | an estimate per character, at the stylesheet's size |
| Windows' grid | GDI |
| The Mac's grid | Core Text |
| The CLI and the tests | `layout::Fixed` |

Pages are derived when asked for and never stored (`doc/view-modes.md` §1); setting them is
`grind-print`'s, behind this same trait.

## Sources

| | Where |
|---|---|
| The paths as first posed, and the argument between them | this file and `doc/suite.md`, in git |
| The architecture rule the decision rests on | `CLAUDE.md`, "Architecture" |
| `style:writing-mode` | rng:2864 |
| `style:page-layout`, `style:page-layout-properties` | rng:12213, rng:12248 |
| `fo:page-width`, `fo:page-height` | rng:12255, rng:12260 |
| `fo:widows`, `fo:orphans`, `fo:keep-with-next` | rng:12512, rng:12517, rng:2110 |
| `style:master-page`, `office:master-styles`, `style:master-page-name` | rng:12141, rng:7939, rng:12803 |
| `style:header`, `style:footer` | rng:11962, rng:10844 |
| `text:note`, `text:notes-configuration`, `style:footnote-sep` | rng:8465, rng:17725, rng:10871 |
