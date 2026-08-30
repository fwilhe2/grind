<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Flat first — the default physical form

> **In doubt, write the form that diffs.**

One decision, stated once and implemented everywhere. Normative for every place this project
chooses between the physical forms of an ODF document without being told which to use.

## The two forms, and why there is a choice at all

ODF defines a document twice over (`doc/ods-format.md` §1):

| | Extensions | What it is |
|---|---|---|
| **Package** | `.ods`, `.odt`, `.odp` | A zip holding `content.xml`, `styles.xml`, `meta.xml`, a manifest, a thumbnail |
| **Flat** | `.fods`, `.fodt`, `.fodp` | One XML file containing all of it |

They are the same document. Nothing in the model, the reader, the formula engine or any shell
distinguishes them — reading sniffs the form from the bytes and never from the name, so the
choice only ever arises when *writing*.

## The decision

**A package is written when a name asks for one. Everything else gets flat XML.**

- `report.odt`, `book.ods` → package. Naming the extension is a decision, and this build does
  not overrule it. A document opened from a `.ods` and saved is still a `.ods`; **no document is
  ever converted behind somebody's back.**
- `report.fodt`, `book.fods`, `notes.xml` → flat. Also a decision, also honoured.
- `report`, `book.backup`, no name at all → **flat**. This is the decision: the default is not
  "whatever is most conventional", it is the form that a reviewer, a `git diff`, a `grep` and a
  code-review tool can read.

And in a UI, where "in doubt" is the normal case rather than the edge one:

- **Save dialogs lead with flat.** The name field is pre-filled with a flat extension, and the
  file-type list puts flat first, so the default action writes a diffable file and choosing the
  package is one deliberate click.
- **Open dialogs prefer neither.** One filter matching both, because a user looking for a
  document does not know which form it is in and should not have to. Flat is listed first there
  too, but only as ordering — both are accepted equally, and so is any file whose *bytes* say
  it is a document whatever it is called.

## Why

`README.md`'s pitch and `doc/suite.md` both lean on it, and `text/tests/diffable.rs` and R6
exist to serve it: **office documents that live in git.** R6 makes an edited document change as
little XML as possible — one keystroke is one line of `git diff` — and that property is worth
exactly nothing if the file it applies to is a zip. A package in a repository is one opaque blob
per commit: no diff, no blame, no review, no merge, and a version-control system reduced to a
backup with a changelog.

The flat form is the whole of the difference between this and every other office suite's
relationship with source control, and it is one of the few genuine differentiators this project
has. A differentiator that a user has to know about and go looking for is not a differentiator;
it is a preference page. So it is the default, and the conventional form is the option.

**What it costs, stated honestly.** A `.fods` is larger than a `.ods` — the zip is doing real
compression, and prose and XML both compress well. That is the trade: bytes on disk against a
history that can be read. Git compresses its own objects, so the cost in a repository is far
smaller than the cost on a filesystem, which is exactly the case this is optimising for.

**And one place the package is genuinely worse than merely opaque.** `grind-text`'s R6 splicing
works on the flat form only, because a zip has no diff to preserve — so saving a `.odt` this
build did not author regenerates it and drops `styles.xml`, `settings.xml`, `meta.xml` and the
thumbnail (`text/src/odf/source.rs`, measured in `text/tests/libreoffice.rs`). The same document
in flat form loses nothing at all. So the default is not only better for review; for text
documents it is currently better for *fidelity*.

## Where it is implemented

One rule, one function, and everything else reaches it:

- **`grind_core::odf::Form::from_path`** — the rule. Names the package extensions exhaustively
  — and `.grind`, the projection, for the same reason: naming one is a decision and not doubt —
  then returns `Form::Flat` for everything else. Nothing anywhere may spell a second extension
  list; `grind-web` reaches this through a `Path` built from a download's name rather than
  restating it, which is the shape of that rule made mechanical.
- **`grind_core::odf::Form::extension`** — the inverse, so a shell naming a new document and the
  CLI creating one cannot disagree.
- **`grind-sheet-gtk` / `grind-text-gtk`** — `save_name` pre-fills `Untitled.fods` /
  `Untitled.fodt`; `*_save_filters` lists flat first as its own entry; `*_filters` stays one
  combined filter for opening.
- **`grind-web`** — a document nobody opened downloads as `untitled.fods` / `untitled.fodt`,
  and the file input accepts both with flat listed first.
- **`grind-cli`** — `grind sheet new book` and `grind text new report` write flat XML, through
  `Form::from_path` like everything else. `grind convert` is the explicit escape hatch in both
  directions and is what a user reaches for when they want the other form on purpose.

## The third form, which diffs better still

`doc/dsl.md`'s projection — `.grind` — is a physical form like these two, and by the standard
this document is named after it wins outright: it is the form a diff reads *best*. It is
nevertheless not the default, and the reason is that "the form that diffs" was never the whole
rule. **The default is the best-diffing form that other software opens**, and nothing but this
build reads a projection.

So the rule stands unchanged and gains one clause: `.grind` is named exhaustively like the
package extensions, because asking for it is a decision; doubt still resolves to flat XML.

## What this decision is not

- **Not a claim that flat is more compatible.** Both forms are ODF and LibreOffice opens both;
  loop C converts both directions through it on every CI run. The package is more *familiar*,
  which is a reason to keep it one click away and not a reason to default to it.
- **Not a conversion policy.** Nothing here ever changes an existing document's form. A `.ods`
  stays a `.ods` until a user types a different name — see `grind convert`.
- **Not about `.xml`.** A bare `.xml` extension has always meant flat and still does; it is a
  spelling LibreOffice writes and this build reads, not part of this decision.
