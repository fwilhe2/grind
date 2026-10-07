<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Spelling

**Normative for `text/src/spell.rs`, `spell/` and every shell's underline.** Built 2026-10-06:
English and German, one language per document, in the core, the CLI, `grind lint` and the GNOME
window, with every other shell's problems pane listing the words. Since 2026-10-07 the terminal
and Windows shells underline and correct as well, and so does the Mac (type-checked, not yet run);
the browser is the one client with nothing (*Where it reaches*).

## The decisions

1. **Hunspell dictionaries, checked by `spellbook`.** Hunspell's format is what LibreOffice,
   Firefox and Chrome check with, so its dictionaries are the best maintained there are and it
   handles what a word list cannot: affixes, and German's compounds
   (`Textverarbeitungsprogramm`, `Rechtschreibprüfung`). `spellbook` (helix-editor, MPL-2.0) is a
   Hunspell-compatible checker in pure Rust — no system library, so the same answers on Linux,
   Windows, macOS and in a browser, and nothing for a package to depend on. Measured against the
   bundled dictionaries in a release build: 17 ms to load English and 27 ms German, microseconds
   a word to check, and 20–250 ms a word to *suggest*, which is why suggestions are only ever
   asked for one word at a time.

   **Not cspell.** It is a fine tool for a repository's own prose and code, but it is a Node
   program built for source code — splitting `camelCase`, knowing programming vocabularies — and
   none of a native binary, a wasm module or a terminal can carry a Node runtime.

2. **The core decides what a word is; a dictionary only answers whether it is right.**
   `grind_text::spell` holds the rules — what a word is, which words are checked at all, where a
   misspelling is — and a `Lexicon` trait the dictionary is handed in through, the way `Metrics`
   is for layout. So every shell underlines exactly the same words, the CLI can answer the
   question, and the two megabytes of word lists live in one optional crate (`grind-spell`)
   rather than in every build that only reads documents.

3. **What is checked is narrower than "every word"**, because a checker stops being believed the
   third time it underlines `ODF`. Skipped: anything with a digit, anything all in capitals, a
   capital inside a word (`OpenDocument`, `iPhone`), one letter, a link's text, a run in a
   monospace face (what `` `code` `` types), and a chunk carrying `://`, `@`, `/`, `\`, `_`, `=`
   or an inner `.` — a URL, an address, a path, an identifier, `e.g.`. A word joins across one
   apostrophe or hyphen between letters; a hyphenated word is right when the whole is or every
   part is; `’` is checked as `'`.

4. **One language per document**, chosen in this order (`grind_spell::choose`):
   1. the one somebody named — `--language`, or a shell's Spelling menu;
   2. the one the document **states**: `fo:language`/`fo:country` on its `Standard` paragraph
      style, over the default paragraph style, which is where LibreOffice keeps it. A language
      stated and not carried (`fr`) is **not checked** rather than checked in English, and `zxx`
      (ODF's "no language") is not checked either;
   3. a **guess**: whichever dictionary knows most of the first 400 checkable words;
   4. with no words to judge by, the desktop's language, else English.

   Most documents this suite writes state no language, so the guess is what usually decides. The
   GNOME window guesses again as an empty document fills, until the guess has read all it reads.

5. **The person's own words are theirs, not the document's.** `$XDG_CONFIG_HOME/grind/words`
   (`~/.config/grind/words`, `%APPDATA%\grind\words`), one per line, `#` for comments — one list
   for every document and both languages. Nothing about spelling is ever written into a document:
   checking is a reading, like `doc/view-modes.md`'s overlays, and a file's bytes after a save are
   what they would have been with spelling off.

6. **A correction is an edit in the core**: `App::correct(at, word, with)`, one undo step, the
   word's formatting kept, refused when the word is no longer where it was found.

## Where it reaches

| Client | What it has |
|---|---|
| `grind` | `grind text spell [range] [--language] [--suggest]`, `--add`, `grind text correct`; `misspelt` in `grind text lint` and `grind lint` |
| `grind-text-gtk` | Wavy underlines (not under the word still being typed), suggestions, Ignore All and Add to Dictionary on right-click, a Spelling submenu (Automatic, English (US), Deutsch, Off), the language in the status bar, and Check Document |
| `grind-tui` | Words underlined in red (not the one being typed), `]`/`[` to the next or previous with suggestions on the status line, `:fix N` or `:fix <word>`, `:spell ignore` and `:spell add`, `:spell auto\|en\|de\|off`, the language on the status bar, and `:lint` |
| `grind-win32` | A squiggle in Fluent's critical red, suggestions with Ignore All and Add to Dictionary at the head of the context menu (right click, or Shift+F10 with the caret in the word), F7 for the next word with the same popup, View ▸ Spelling Language…, the language on the status bar, and Check Document |
| `grind-mac` | The dotted red underline, drawn by the portable `text/paint.rs`; suggestions, *Ignore Spelling* and *Learn Spelling* leading the page's context menu over a misspelt word, and Edit ▸ Spelling ▸ *Check Document Now* (⌘;, the next word selected) and *Spelling Language…* (`ui_mac/src/spelling.rs`, 2026-10-07, type-checked only); the misspelt words in the sidebar's Problems |
| `grind-web` | Nothing yet |

## The gaps, named

- **Several languages in one document.** The next step, and a requirement rather than a nicety: a
  language per paragraph and per run (`fo:language` on a paragraph style, an automatic text
  style), which needs the text model to *read* both and the writer to carry them.
- **Setting the document's language.** Read and never written: stating one means writing
  `office:styles`, which the text writer does not own (`doc/text-core.md`). The menu's choice is
  the session's.
- **Regional spellings.** One dictionary per language: `en-GB` is checked as American English,
  and `de-AT`/`de-CH` as German German (so Swiss `ss` for `ß` is underlined). Each is one more
  pair of files and one more `Language`.
- **The Mac does not guess again.** Its menu is a view over `grind_spell::Setting` like every
  other shell's, but a new document's guess is not revisited as it grows (`grind_spell::reguess`,
  which the terminal and Windows call after each edit), and the language is not on a status line.
- **The browser.** Two megabytes the page should fetch on first use rather than carry, the way
  `grind-print`'s fonts are fetched.
- **A personal list in the projection or the document.** Deliberately not (decision 5).

## Licences

`spell/dictionaries/` is copied as released from `wooorm/dictionaries` at `8cfea40`. English is
SCOWL's `en_US` 2020.12.07 under its own permissive notice (`LICENSES/LicenseRef-SCOWL.txt`);
German is igerman98 20161207, offered under GPL-2.0 or GPL-3.0 and taken under GPL-3.0, which
AGPL-3.0 §13 lets a work combine with. Both are on the third-party list (`doc/third-party.md`),
which `grind licences` prints.
