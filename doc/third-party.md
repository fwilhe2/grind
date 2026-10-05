<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Third-party components and their licences

**Normative for `core/src/third_party/`, `core/tests/third_party.rs` and every shell's About.**

Grind's programs are built from other people's work — about 350 Rust crates, the Liberation fonts
the PDF falls back on, the Rust standard library, and the OpenFormula function summaries taken
from the OASIS specification — and nearly every one of those licences asks for the same thing in
return: pass the copyright notice and the licence text on with every copy. This document is how
the suite does that and how it is kept from going stale.

## The decisions

1. **One list, in the core.** `grind_core::third_party` is every component and its licence files,
   verbatim. Every client reads it, so six shells cannot keep six lists:

   | Client | Where |
   |---|---|
   | `grind` | `grind licences` (the full text), `grind licences --list` (one row each, `--format json` too) |
   | `grind-sheet-gtk`, `grind-text-gtk` | About ▸ Legal — one `add_legal_section` per component |
   | `grind-tui` | `:licences` — the read-only pane `:help` scrolls in |
   | `grind-web` | Ctrl+K ▸ *About this build* — a `<dialog>`, one collapsed row per component |
   | `grind-win32` | Help ▸ Third-Party Licences — a read-only box that scrolls |
   | `grind-mac` | About Grind — the standard panel's Credits, which is its scrolling area |

2. **Generated, never written by hand.** `core/src/third_party/data.rs` is written by
   `core/tests/third_party.rs`, from:
   - `cargo metadata`, walked from **every workspace member that builds a binary or a wasm
     module** along *normal* dependency edges, on every platform at once. A new shell is covered
     without being named. Build and dev dependencies are left out, since neither ends up in an
     artifact. Proc-macro crates stay in: what they generate does.
   - Each crate's own licence files (`LICENSE*`, `COPYING*`, `NOTICE*`, `COPYRIGHT*`, and
     `package.license-file`) out of the crate as Cargo unpacked it, verbatim apart from line
     endings and trailing blanks. Identical texts are stored once.
   - A crate that packages no licence file (about twenty do: `krilla`, the `objc2` family, …)
     gets the standard text of a licence it states, from `LICENSES/`, with its `authors` as the
     holders, **and a line saying that is what it is**.
   - `EXTRAS` in that test, for what Cargo cannot see: the fonts (their copyright read out of
     each font file's own `name` table), the Rust standard library, and the OASIS summaries.

   It is **one list for the whole suite**: the union of what every program links. The Windows
   crates show in the GTK window's About too. That is a superset, and for attribution a
   superset is the safe direction.

3. **Checked on every `cargo test`, and updated by it.** The test fails whenever the
   checked-in file differs from the one it would write, **and writes the new one as it fails**.
   Adding, removing or bumping a dependency is then one `cargo test` and one `git diff` to
   read. CI runs it in the `build` job like every other test, so a stale list cannot merge.

4. **Nothing gets past it.** Three more checks are part of the same test file:
   - **The licence allow list.** Every crate's stated licence has to be satisfiable from
     `ALLOWED`, which lists the licences checked as compatible with distributing the suite
     under AGPL-3.0-or-later. A new licence fails the build until a person has looked at it.
   - **Assets that are not crates.** Every file that REUSE says is under a licence other than
     ours must be claimed by an extra or sit under a `NOT_SHIPPED` path (the vendored specs,
     test fixtures, the container recipe). The answer counts both `REUSE.toml`'s annotations
     and each file's own SPDX tag. Since `reuse lint` (CI) already forces every file to say
     what it is under, a vendored font, icon or text cannot arrive silently. Its first run
     found `sheet/src/formula/funcs/catalog.rs`: OASIS text compiled into every binary, whose
     notice was missing from every About until then.
   - **No stale claims.** An extra that claims a path REUSE no longer knows also fails.

## What is deliberately not on it

- **System libraries linked dynamically** (GTK, libadwaita, Pango, cairo, glibc on Linux;
  AppKit and CoreText on the Mac; the Windows API). They are not part of our artifact. The
  system ships them under its own notices, and the `-sys` crates that bind them *are* on
  the list.
- **The MSVC runtime**, linked statically into `grind-win32.exe` (`.cargo/config.toml`). Its
  licence is Microsoft's Visual Studio redistribution terms, which ask for no notice.
- **Build tools**: the compiler, `cc`, `embed_resource`, `wasm-bindgen-cli`. The JavaScript glue
  that `wasm-bindgen` generates is covered by the `wasm-bindgen` crate's own entry.

## When something changes

- **A dependency added, removed or bumped**: run `cargo test -p grind-core --test third_party`,
  read the diff of `core/src/third_party/data.rs`, then commit it.
- **A crate under a licence not yet on `ALLOWED`**: decide whether it can be distributed in an
  AGPL program. If it can, add it, with the decision in the commit message.
- **A vendored asset (font, image, text) under someone else's licence**: annotate it for REUSE as
  usual, then add an `Extra` claiming the same path.

## Size

The texts are about 0.7 MB raw, which is around 12% of the browser module's uncompressed wasm.
gzip brings that down to roughly 70 KB on the wire. The texts are stored verbatim on purpose:
cutting a licence down to "the same as the one above" is how the notices come to be missing.
