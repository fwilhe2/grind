<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Spelling

**Normative for `text/src/spell.rs`, `spell/` and every shell's underline.** Built 2026-10-06:
English and German, one language per document, in the core, the CLI, `grind lint` and the GNOME
window, with every other shell's problems pane listing the words. Since 2026-10-07 the terminal
and Windows shells underline and correct as well, and so does the Mac (type-checked, not yet run);
the browser is the one client with nothing (*Where it reaches*). Since 2026-10-10 there are
seven languages: French, Spanish, Italian, Portuguese and Polish joined the first two (*The
languages*).

## The decisions

1. **Hunspell dictionaries, checked by `spellbook`.** Hunspell's format is what LibreOffice,
   Firefox and Chrome check with, so its dictionaries are the best maintained there are and it
   handles what a word list cannot: affixes, and German's compounds
   (`Textverarbeitungsprogramm`, `Rechtschreibprüfung`). `spellbook` (helix-editor, MPL-2.0) is a
   Hunspell-compatible checker in pure Rust — no system library, so the same answers on Linux,
   Windows, macOS and in a browser, and nothing for a package to depend on. Measured against the
   bundled dictionaries in a release build: 17 ms to load English, 27 ms German, 40–90 ms each
   of the Romance languages and 290 ms Polish; microseconds a word to check, and 2–280 ms a word
   to *suggest*, which is why suggestions are only ever asked for one word at a time.

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
   3. a **guess** from the first 400 checkable words, in two stages so that it never loads seven
      dictionaries to make: first whichever language's **common words** occur most often
      (`grind_spell`'s `Common` — about forty short words each, articles, pronouns, particles,
      *to be*, no word in two lists, which a test holds), and only when not one of them occurs
      (a heading, a list of nouns) whichever of the desktop's language and English knows more
      of the words. The common words are also the better judge: Spanish, Portuguese and Italian
      share a great many words and very few articles;
   4. with no words to judge by, the desktop's language, else English.

   Most documents this suite writes state no language, so the guess is what usually decides. The
   GNOME window guesses again as an empty document fills, until the guess has read all it reads.

5. **The person's own words are theirs, not the document's.** `$XDG_CONFIG_HOME/grind/words`
   (`~/.config/grind/words`, `%APPDATA%\grind\words`), one per line, `#` for comments — one list
   for every document and every language. Nothing about spelling is ever written into a document:
   checking is a reading, like `doc/view-modes.md`'s overlays, and a file's bytes after a save are
   what they would have been with spelling off.

6. **A correction is an edit in the core**: `App::correct(at, word, with)`, one undo step, the
   word's formatting kept, refused when the word is no longer where it was found.

## The languages

English and German first, because they were the author's; then the next five by the number of
people who write them, among the languages of Europe written in the Latin alphabet (Russian and
Ukrainian are the two this rule passes over): **French, Spanish, Italian, Portuguese and
Polish**. That is the Pareto line — seven languages cover most of the people who write in one of
Europe's languages, and each further one (Dutch, Romanian, Swedish, Czech, …) covers fewer than
the last for the same two files and the same review of a licence.

| Language | Tag | Dictionary | Taken under |
|---|---|---|---|
| English | `en-US` | SCOWL `en_US` 2020.12.07, size 60 | SCOWL's own notice |
| German | `de-DE` | igerman98 20161207, reformed spelling | GPL-3.0 (of GPL-2.0 or 3.0) |
| French | `fr-FR` | Grammalecte 7.5, the 1990 reform and the older spellings both | MPL-2.0 |
| Spanish | `es-ES` | RLA-ES `es_ES` 2.8 | MPL-2.0 (of GPL-3+, LGPL-3+ or MPL-1.1+) |
| Italian | `it-IT` | Italian Writing Aids 5.0 | GPL-3.0 |
| Portuguese | `pt-PT` | Universidade do Minho `pt_PT`, the 1990 agreement's spelling | LGPL-2.1 (of GPL-2.0, LGPL-2.1 or MPL-1.1) |
| Polish | `pl-PL` | Polish Native Lang Project, 2008-12-06 | GPL-3.0 (of an unversioned GPL, LGPL, MPL or CC-BY-SA) |

**Portuguese is Portugal's**, though most people who write Portuguese write Brazil's, by the
same Pareto argument applied to the cost: the Brazilian dictionary is 5.4 MB and loads in 340 ms
where Portugal's is 1.5 MB and 40 ms, and since the 1990 agreement the two spell most words
alike (`ação`, `ótimo`). A `pt-BR` document is checked as European Portuguese and says so,
which is the "Regional spellings" gap below.

Together the seven are about 12 MB of word lists, compiled into every binary that checks
spelling. A dictionary is parsed only when it is first asked for, so a binary that checks one
language pays to load one.

## In a spreadsheet

Built 2026-10-10. The same rules, the same dictionaries and the same choice of language, through
the same door: what a word is and which words are checked moved from `grind_text::spell` into
`grind_core::spell` the day a second application wanted them, beside the `Lexicon` trait and a
`Spelled` trait that each application's `App` implements — so `grind-spell` chooses, attaches and
turns off a dictionary without naming either application, and depends on neither.

1. **Only typed text is checked.** A cell holding a text value with no formula behind it — a
   heading, a label, a note (`Sheet::text_cells`). A number, a date and a boolean have no words,
   and a formula's result is what the formula computed rather than prose somebody wrote, so
   `="wrogn"` is not underlined. Inside a cell the word rules are decision 3's.
2. **The language a spreadsheet states is its locale's** — `fo:language` on the default cell
   style, which `Document::locale` already reads to decide how numbers are spelled. Then the
   guess, as for text.
3. **The misspelt ranges travel in the `Viewport`** (`Viewport::misspelt`), as character ranges
   into what each cell displays, so no shell checks a word itself and every shell underlines the
   same ones. Empty when no dictionary is attached, which is what *off* is.
4. **A correction is `App::correct(sheet, cell, offset, word, with)`**: one undo step, the cell
   stays text whatever the new word looks like, and dependants recalculate.
5. **Easy to turn off, and it stays off.** A spreadsheet of part numbers, surnames and codes is
   somewhere a person may well not want it. One choice — `off`, `auto`, or a language — is
   **remembered** per kind of document in `$XDG_CONFIG_HOME/grind/spelling`
   (`grind_spell::preference`), beside the personal word list and for the same reason: it is the
   person's, never the document's. Every spreadsheet window opens with it and every toggle writes
   it; `grind sheet spelling off` sets it from a terminal, and `grind sheet lint` and `grind lint`
   follow it. The text windows do not read it yet: their Spelling menus are still the session's
   (a gap below).

## Where it reaches

| Client | What it has |
|---|---|
| `grind` | `grind text spell [range] [--language] [--suggest]`, `--add`, `grind text correct`; `misspelt` in `grind text lint` and `grind lint`. For spreadsheets `grind sheet spell [--sheet] [--language] [--suggest]`, `grind sheet correct <cell> <word> <with> [--at]`, and `grind sheet spelling [auto\|off\|<lang>]` — the remembered switch, which `grind sheet lint` and `grind lint` follow |
| `grind-text-gtk` | Wavy underlines (not under the word still being typed), suggestions, Ignore All and Add to Dictionary on right-click, a Spelling submenu (Automatic, each language in its own name, Off), the language in the status bar, and Check Document |
| `grind-sheet-gtk` | The word processor's wavy underline under misspelt words in text cells — ordinary, overflowing, wrapped and merged; not turned text — and a spelling section per word (up to three) leading the cell's right-click menu: suggestions, Ignore All, Add to Dictionary. View ▸ Check Spelling (Shift+F7, LibreOffice's key, and in the palette) turns it off and on, and View ▸ Spelling Language chooses; both are **remembered**. Check the Document lists the words |
| `grind-tui` | Words underlined in red (not the one being typed), `]`/`[` to the next or previous with suggestions on the status line, `:fix N` or `:fix <word>`, `:spell ignore` and `:spell add`, `:spell auto\|en\|de\|fr\|es\|it\|pt\|pl\|off`, the language on the status bar, and `:lint`. **In a spreadsheet** the same: a misspelt word's letters underlined in red (only the text the core checked, and only what a narrow column shows), `]`/`[` across every sheet, `:fix N`/`:fix <word>`, `:spell ignore`/`add`, and `:spell auto\|<lang>\|off` — **remembered** for every spreadsheet |
| `grind-win32` | A squiggle in Fluent's critical red, suggestions with Ignore All and Add to Dictionary at the head of the context menu (right click, or Shift+F10 with the caret in the word), F7 for the next word with the same popup, View ▸ Spelling Language…, the language on the status bar, and Check Document. **In a spreadsheet** the same squiggle under a text cell's misspelt words (wrapped and merged cells too, stopping at an ellipsis), the same popup leading a cell's right-click menu, F7 across every sheet, and View ▸ Check Spelling (Shift+F7) — **remembered** for every spreadsheet, as View ▸ Spelling Language… is there; over a text document both stay the session's |
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
  `de-AT`/`de-CH` as German German (so Swiss `ss` for `ß` is underlined), `pt-BR` as European
  Portuguese (`facto`, `receção`) and Latin American Spanish as Spain's. Each is one more pair
  of files and one more `Language`.
- **Further languages.** Dutch is next by the rule in *The languages* (OpenTaal, BSD-3-Clause),
  then Romanian. A language in another script (Russian, Ukrainian, Greek) was left out by that
  rule rather than by anything measured; whether `grind_text::spell`'s idea of a word holds for
  one is unchecked.
- **The Mac does not guess again.** Its menu is a view over `grind_spell::Setting` like every
  other shell's, but a new document's guess is not revisited as it grows (`grind_spell::reguess`,
  which the terminal and Windows call after each edit), and the language is not on a status line.
- **The text windows do not remember.** `grind_spell::preference` keeps a setting per kind of
  document and the spreadsheet windows read and write theirs; the word processor's Spelling menus
  are still the session's, and turning them to the remembered setting is a line in each.
- **The browser.** Twelve megabytes the page should fetch on first use, one language at a time, rather than carry, the way
  `grind-print`'s fonts are fetched.
- **A personal list in the projection or the document.** Deliberately not (decision 5).

## Licences

`spell/dictionaries/` is copied as released from `wooorm/dictionaries` at `8cfea40`, unmodified
and renamed by tag. English is SCOWL's `en_US` 2020.12.07 under its own permissive notice
(`LICENSES/LicenseRef-SCOWL.txt`); German is igerman98 20161207, offered under GPL-2.0 or GPL-3.0
and taken under GPL-3.0, which AGPL-3.0 §13 lets a work combine with. The other five are each
taken under the one licence *The languages* names, and `REUSE.toml` declares that one: MPL-2.0
for French and Spanish (neither carries MPL's "Incompatible With Secondary Licenses" notice, so
§3.3 lets them sit in this work), GPL-3.0 for Italian and Polish, and LGPL-2.1 for Portuguese,
since its other two offers, GPL-2.0-only and MPL-1.1, cannot be combined with AGPL-3.0 and
LGPL-2.1's §3 allows a later GPL. All seven are on the third-party list (`doc/third-party.md`),
with the licence each was offered under and the one it is used under, which `grind licences`
prints.
