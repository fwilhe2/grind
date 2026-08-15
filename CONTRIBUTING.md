<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Contributing

## The clean-room rule

This project implements ODF from the OASIS specifications. It is **not** a port of
LibreOffice, and no LibreOffice code is present in it.

LibreOffice's source may be **read**, and facts learned from it may be **cited by file and
line**. Nothing may be copied — not a function, not a table, not a constant list, not a
reworded line-by-line transcription. "I retyped it" is copying.

Every fact derived from reading LO goes through a **spec document first**
(`doc/ods-format.md` is the template), and only then into code. If a behaviour is worth
implementing, it is worth one sentence in the spec saying what it is and where it was
observed. This is what makes the work auditable by someone who wasn't there — and it is why
the project can be licensed freely at all. A port would have been bound to LibreOffice's
MPL-2.0 forever; a clean-room implementation is wholly ours to license.

## Licensing and REUSE

The project is **AGPL-3.0-or-later** — network use counts as distribution (§13), so a
modified version offered as a hosted service must publish its source. Contributions are
accepted under the same terms.

**[REUSE](https://reuse.software) compliance is enforced in CI.** Every file — source, docs,
config, generated — carries machine-readable copyright and license information. Before you
push:

```sh
reuse lint
```

Adding a file means annotating it:

```sh
reuse annotate --copyright "Your Name <you@example.com>" \
               --license AGPL-3.0-or-later path/to/file
```

Files that cannot carry a header (generated output such as `Cargo.lock`) are declared in
`REUSE.toml` instead.

### The two files you must not annotate in place

`doc/OpenDocument-v1.4-schema.rng` and `doc/OpenDocument-v1.4-os-part4-formula.html` are
OASIS specifications, redistributed verbatim. They are **not** open source. The OASIS IPR
Policy permits copying and redistribution, but states that the document *"may not be modified
in any way, including by removing the copyright notice or references to OASIS"* — and adding
an SPDX header is a modification.

They are therefore marked with `.license` **sidecar** files. If you ever re-download them,
re-add the sidecars; never run `reuse annotate` against them without `--force-dot-license`.

Anything you derive from those documents — `doc/small-group.md` is the worked example —
carries the OASIS copyright line alongside ours, because the OASIS grant for derivative works
is conditional on passing the notice along.

### Third-party code

There is none yet, and every addition is a decision (see `doc/plan.md`). When one arrives:
it must be AGPL-3.0-compatible — permissive (MIT / Apache-2.0 / BSD) and GPL-3.0 are fine
(GPLv3 §13 explicitly permits combining with AGPLv3), GPL-2.0-**only** is not — it gets its
own `LICENSES/` entry and correct
`SPDX-FileCopyrightText` naming the upstream author — not us — and vendored files keep their
original headers untouched.

Sources of truth, in order:

1. **ODF 1.4 Part 3** — `doc/OpenDocument-v1.4-schema.rng`, the content schema.
2. **ODF 1.4 Part 4** — `doc/OpenDocument-v1.4-os-part4-formula.html`, the formula language:
   per-function semantics, argument types, implicit conversions, error model.
3. **`doc/ods-format.md`** — clean-room notes on what LibreOffice actually does, where the
   specs leave room and real files disagree. Every entry cites `file:line`.
4. **LibreOffice's test corpus** — as an oracle, never as a source. See below.

Do not use the LibreOffice name or branding in anything user-facing.

## The three loops

LibreOffice is how correctness is checked. `soffice` must be on `PATH` for loop C.

| Loop | What | Where |
|---|---|---|
| **A** — read tolerance | every `.ods`/`.fods` in LO's corpus loads without error | `core/tests/corpus_read.rs` |
| **B** — formula conformance | recalculate each of LO's 509 per-function fixtures, compare against the cached value in the file | phase 4 |
| **C** — round-trip differential | write a document, convert it with `soffice --headless --convert-to`, read it back, assert semantic identity — and the reverse | phase 3 |

Point them at a LibreOffice checkout:

```sh
SHEET_LO_CORPUS=/path/to/libreoffice/core/sc/qa/unit/data cargo test
```

Without it the corpus tests skip with a notice rather than failing, so `cargo test` still
works on a machine that has no checkout.

## The rules that carry the weight

Carried over from [fwilhe2/editor](https://github.com/fwilhe2/editor)'s
`doc/shared-core-native-shell.md`. Breaking one is cheap today and expensive later.

1. **Reads go through a windowed API** — `get_viewport(sheet, rows, cols)`. Never a getter
   that hands a caller the whole document.
2. **Undo/redo lives in the core**, as a command pattern whose inverse falls out of the
   action itself.
3. **The core pushes, shells never poll.** Drop the write lock *before* notifying observers,
   or an observer that reads the document deadlocks. There is a test.
4. **Whatever any GUI can do, the CLI can do.** A UI-only feature is a bug.
5. **Do not assume a filesystem.** Every `*_file` function has a `*_bytes` twin.
6. **The core's public API is shaped for Rust.** FFI annotations live in a facade crate.
7. **Every feature must survive a round-trip through LibreOffice unchanged.** Loop C
   enforces this, which is what makes the feature line a gate rather than a preference.

Rule 7 is also the anti-bloat mechanism: if it doesn't round-trip, it isn't done, and if it
isn't on the "in" list in `doc/plan.md`, it doesn't get built.
