<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

# Grammar and style

**Status: G0–G7 built (2026-10-31): every client checks grammar; see §10 for what is left.
The LanguageTool integration was built (G5) and removed on 2026-11-07 (§5).** One checker, the
**built-in** one: rules written here in plain Rust with no dependencies, small, always available,
and on or off. This supersedes `doc/spelling.md` decision 7 ("No grammar checker") and answers
each of its three reasons (§2).

## 1. What it is

A second kind of underline in the word processor, beside spelling's. Under a phrase that is
probably wrong (`could of`, `a apple`, `the the`, `seit ihr`) or that the person asked to be
told about (`in order to`, a 60-word sentence), the shell shows a message, the replacements and
a way to ignore the finding or the rule, as it does for a misspelt word. The CLI, `grind lint`
and every shell get it the way they got spelling.

Spreadsheets are **not** in scope. A cell is a label, a name or a code, and a grammar rule over
two words is noise. Spelling covers them; grammar does not.

## 2. Why now, when Harper was declined

`doc/spelling.md` decision 7 turned Harper down for three reasons. This design answers each one.

| Harper's objection | Built-in checker |
|---|---|
| Its spelling is worse than ours (underlines `ODF`, ignores the personal list) | Has no spelling rules. Spelling stays Hunspell's |
| English only, and a German paragraph drew seventeen findings | Rules are per language, and run only on a paragraph whose language is known (decision 4). Language-neutral rules run everywhere. English and German first |
| 11 MB, 92 crates, an ML framework in every build | A few hundred kB of rules, no new crate in `Cargo.lock` |

## 3. Decisions

1. **The core decides where a finding is. A checker only finds it.** The shape is spelling's: a
   generic `grind_core::grammar` module holds the types (`Finding`, a `Checker` trait) with no
   document vocabulary (R8). `grind-text` hands a checker each paragraph's text with the ranges
   that must not be checked: decision 3 of spelling's skip rules (links, monospace runs, URLs,
   paths). It keeps the findings per paragraph, keyed by the paragraph's text, so an edit
   invalidates only what it touched. Findings travel to shells in the `Viewport`
   (`Viewport::flagged`, beside `misspelt`). No shell runs a rule itself.

2. **A finding is a range, a rule, a kind, a message and replacements.** Character offsets into
   the paragraph (0-based, as everywhere in the core). A rule id stable enough to be ignored by
   name (`builtin:repeated-word`). The kind is one of **grammar**,
   **punctuation**, **typography** or **style**, and the kind decides the underline: grammar and
   punctuation in the theme's blue, typography and style dotted and quieter — the Mac keeps
   AppKit's own green dots for grammar. Spelling's red stays spelling's alone. A replacement may
   be empty, which means "delete this".

3. **A correction is an edit in the core.** `App::correct(at, expected, with)` — spelling's,
   which already replaced any span and not only a word. It is one undo step, keeps the
   formatting of the first character replaced, and is refused when the text at `at` is no
   longer `expected`.

4. **Language per paragraph, guessed cheaply.** A language-specific rule runs only where the
   paragraph's language is known: the language the document states, else a guess **per
   paragraph** from `grind_spell`'s common words (the first stage of spelling's guess, which
   loads no dictionary). A paragraph whose guess is unsure gets only the language-neutral rules.
   This is how a German quotation in an English letter avoids English rules. It also closes
   part of spelling's first named gap, so `Common` moves somewhere both can reach it: into
   `grind_core::spell`, or a small module of `grind-spell` that the grammar crate depends on
   without the dictionaries. Which one is settled at G1.

5. **Ignoring is the person's, never the document's** (spelling's decision 5 again). *Ignore
   once* is for the session. *Ignore this rule* is remembered in
   `$XDG_CONFIG_HOME/grind/grammar`, beside `words` and `spelling`. The same file remembers
   whether the checker is on and the style level; a `languagetool` line left in it from before
   §5 is read past. Nothing about grammar is ever written into a document.

6. *Withdrawn with §5* (LanguageTool's spelling category was always switched off).

7. **The built-in rules are ours, written clean.** LanguageTool's rule files are LGPL-2.1. The
   built-in checker copies none of them, the way this project copies nothing from LibreOffice.
   Each rule is written from the grammar of the language and our own examples. LanguageTool was
   a **conformance oracle** for it (§6, loop G′), never a source, until §5.

8. *Withdrawn with §5* (the `languagetool` feature, in the default build).

9. *Withdrawn with §5* (self-hosted, public and Premium servers, and saying where the text goes).

10. **The core does no I/O.** The checker answers at once and is called from the core. The
    cache that kept a checker's late answers by paragraph text (`App::grammar_found`,
    `App::grammar_pending`) went with §5: nothing answers late any more.

## 4. The built-in checker

A new crate, **`grind-grammar`**: pure Rust, no dependencies beyond `grind-core`, built for
wasm too. It works on the tokens `grind_core::spell` already produces (words, spaces,
punctuation) plus sentence boundaries. A rule is a function over a sentence's tokens with
fixtures beside it, not a pattern language. A pattern language is what LanguageTool has, and
building a second one is the cost this checker exists to avoid.

**The bar for a rule: precision first.** A rule ships on by default only when at least 9 of 10
of its findings over the corpus (§6) are right. A rule that cannot meet that is opt-in
("picky"), or is not written. A checker that cries wolf gets switched off, and then it catches
nothing.

### First rules

**Language-neutral** (every paragraph):

| Rule | Catches | Does not fire on |
|---|---|---|
| `repeated-word` | `the the`, `und und` | A per-language list of real doubles (`that that`, `had had`, `die die`, `das das`); across a line break inside a heading |
| `space-before-punctuation` | `word ,` `word .` | French `;:!?` (a narrow no-break space is right there); `...` |
| `missing-space-after` | `end.Next` `a,b` | Numbers (`1,5`, `3.14`), URLs and paths (spelling's skip rules), abbreviations (`e.g.`, `z.B.`), file names |
| `double-space` | two spaces between words | Spaces before a tab or at a line's end |
| `repeated-punctuation` | `!!`, `,,`, `??` | `...` and `…` |
| `sentence-start-lowercase` | `. then` | After an abbreviation in a per-language list; after a number (`3. Oktober`); after `…` |
| `unpaired-bracket` | `(` with no `)` in the paragraph | `1)`, `a)` list labels |

**English** (`en`):

| Rule | Catches |
|---|---|
| `a-an` | `a apple`, `an test`, with lists of exceptions by sound (`an hour`, `a user`, `a one-off`, `an MBA`) |
| `could-of` | `could of`, `should of`, `would of`, `must of` → `… have` |
| `than-then` | `better then`, `more then`, `rather then` → `than` |
| `its-it's` | `its a`, `its the` → `it's`; `it's own` → `its own` |
| `your-you're` | `your welcome`, `your right` (sentence-final) |
| `there-their` | `their is`, `their are` → `there` |
| `lowercase-i` | ` i ` as a word → `I` |
| `alot`, `irregardless` | single words, as typography/style |

**German** (`de`):

| Rule | Catches |
|---|---|
| `seid-seit` | `seit ihr`, `ihr seit` → `seid`; `seid gestern` → `seit` |
| `comma-before-conjunction` | a word followed by `dass`, `weil`, `obwohl` with no comma, except after `so`, `als`, `ohne`, `statt`, `anstatt` |
| `das-dass-after-comma` | `, das` followed by a word that cannot start a relative clause (a narrow list), as punctuation, picky |
| `wieder-wider` | a few fixed phrases (`wiedersprechen`, `wiederrum`), which are really spelling slips Hunspell may accept |

**Style** (all opt-in, "picky", English first): `in order to` → `to`, `at this point in time`,
`very`/`really` density, a sentence over 40 words, the same word starting three sentences in a
row. The style list is where a writer's taste differs most. None of it is on by default, and the
first list is short on purpose.

French, Spanish, Italian, Portuguese and Polish get the language-neutral rules from the start.
They get their own rules only when someone who writes them asks, as spelling's languages were
chosen.

## 5. LanguageTool: built, then removed

**Decided 2026-11-07: grind talks to no other computer** (`doc/local-only.md`, R12), and the
LanguageTool integration was the one thing that did. It was removed whole: the
`grind-languagetool` crate and its `ureq`/`rustls` stack, the `languagetool` cargo feature,
`grind grammar <url|public|premium>`, `--check-server`, `grind text grammar --languagetool`, each
window's *LanguageTool Server…*, and the three transports beside the crate's own client — WinHTTP
on Windows, the system's `curl` on the Mac, `fetch` in the browser. A self-hosted server on this
computer was an option, but the same code reached the public and Premium servers, and "local
unless configured otherwise" is not the promise R12 makes.

It **may come back in another shape**, and if it does, R12 decides the shape: a server on the
loopback address only, or a separate program the person installs on purpose. What was learned
building it holds for any return, and is the reason this section is kept:

- **The request.** `POST {server}/v2/check`, form-encoded: `data`, `language`,
  `disabledCategories=TYPOS` (LanguageTool's spelling would underline `ODF` again and not know
  the personal list), `disabledRules` for ignored `lt:` rules, `level=picky` for style. English,
  German and Portuguese want a variant (`en-US`, `de-DE`).
- **Markup, not plain text.** `data` is an `annotation` array of `text` and `markup` items; what
  spelling skips (links, code, URLs) goes as markup, unchecked but counted.
- **Offsets** — `MEASURED` (2026-10-30, LanguageTool 6.8, `erikvl87/languagetool` image). The API
  calls them characters; they are **UTF-16 code units**, counted over everything sent, markup
  included.
- **A late answer.** A checker over HTTP answers after the person typed on, so the core kept its
  answers by a hash of the paragraph's text and language and dropped any for text since changed,
  and `App::correct` refused a replacement whose text was no longer what was expected. The second
  half is still there.
- **Cost.** The HTTP client and TLS added 2.3 MB to a stripped release `grind` and sixteen crates,
  three more each on Windows and macOS for their certificate stores. `ring`'s C would not
  cross-check from Linux, which is why Windows and the Mac had transports of their own.
- **The public API's terms** (read 2026-10-30): 20 requests and 75 KB a minute, 20 KB a request,
  a visible link back, users told where their text goes, and no automated requests.

The code is in git history before the removal commit, for reading rather than restoring.

## 6. Proving it

- **Each built-in rule** has fixtures it must flag and fixtures it must not, beside the rule
  (`grammar/tests/<lang>.rs`). A false positive found later becomes a must-not fixture,
  ratchet-style, never a deleted rule test.
- **Precision over a corpus.** The gov.uk ODT corpus, our own `doc/*.md` prose and the docx
  samples, run through `grind text grammar --builtin`. Findings are counted per rule, and a
  sample of each rule's findings is judged by hand before it is on by default (§4's 9 in 10).
  The counts go in this document, as spelling's measurements did.
- **What the corpus measured** (2026-10-30, the 257 gov.uk documents, English, with a few
  Welsh). Each rule's findings were read in context (all of them where there were few, a sample
  of 20–25 otherwise), before and after the fixes they prompted. Each fix is now a test.

  | Rule | Findings | Right | What changed |
  |---|---|---|---|
  | `double-space` | 907 → 86 | 19 of 20 sampled | Runs of three or more lay out forms (`Day   Month   Year`), and two after a sentence is a typing convention: now exactly two between words, two after a sentence only as style |
  | `space-before-punctuation` | 20 | 20 | — (`Location :`, `Article 119 .`, a letter's `Dear ,`) |
  | `repeated-word` | 4 → 3 | 3 | A form's `does         does not` choice: at most two spaces between |
  | `repeated-punctuation` | 3 | 3 | — (`::`, `??`) |
  | `missing-space-after` | 63 → 2 | 2 | Every full stop was `GOV.UK`; `LEP,d` is a symbol |
  | `sentence-start-lowercase` | 14 → 5 | 5 | `inc.`, `ref.`, `per cent.` are abbreviations; `? e.g.` is not a sentence |
  | `unpaired-bracket` | 73 → 62 | about 6 of 25 | Labels like `d)` after a line break are fine; most of the rest close in a later paragraph or cell, which one paragraph cannot see. **Opt-in** (picky) |

- **Our own `doc/*.md`**, imported as Markdown: 725 findings at first, nearly all code. A
  fenced block was checked as prose (now skipped whole, for spelling too) and the punctuation
  inside a code run was read beside its prose (now one opaque token). After those and the fixes
  they showed (`::` in a path, `chart:axis`, `.xlsx`, `an rng`, `a Unicode`, `ᵖ` footnote keys,
  `1825 : 443`), the rest is 21 lowercase sentence starts, most of them this project writing
  *grind* in lowercase on purpose, and a few `in in`s after a phrasal verb (`moved in in July`),
  which stays a finding because the gov.uk corpus's `set out in in Part VI` was a real one.
  None of the English rules fired on the gov.uk corpus at all: it shows they keep quiet on
  edited English, not what they catch, which is loop G′'s to show.

- **German**: thirty random German Wikipedia articles (22,000 words, fetched as plain text,
  never committed). The German rules found nothing in them, which is right for edited prose. The
  language-neutral `sentence-start-lowercase` drew 15, all after an abbreviation German writes
  freely (`hrsg.`, `kais.`, `zus.`, `Rs.`, `Jakob VI.`) or before a domain (`forum.de`). A word of
  four letters or fewer before a full stop is now taken as an abbreviation unless it is a common
  last word (`ist`, `war`, `it`, `now`), and so is a Roman numeral. That dropped all 15, at the
  price of one real gov.uk finding (`and so on. being`).

- **Loop G′** runs the built-in checker over `grammar/tests/data/samples.txt` with a `FLOOR` on
  the slips caught and no finding allowed in a correct sentence. Until §5 it also compared
  against a LanguageTool server: a finding of ours LanguageTool did not make was a **suspected
  false positive**, one of LanguageTool's a built-in rule could make the **backlog**. That half
  went with §5; the first run's results below are what it found.
- **Loop G′, first run** (2026-10-30, `grammar/tests/loop_g_prime.rs`, LanguageTool 6.8 in the
  container). Forty-seven sentences written for it (`grammar/tests/data/samples.txt`, CC0):
  31 with one slip each, 16 correct but tempting. The built-in checker found 17 of the 31 and
  nothing in the 16. Both found 12. The built-in checker alone found 5 that LanguageTool missed
  (`better then`, `its a`, `it's own`, a lone `i`, `ended.Then`), all real slips, so
  LanguageTool is not a superset and running both is worth it. LanguageTool alone found 9, the
  backlog: verb agreement (`He go`, `They was`, `She don't`, `has went`), `informations`,
  `discuss about`, `more taller`, `if I would have known`, and German `die die Bücher`, which the
  built-in checker leaves alone on purpose because `Frauen, die die Bücher lasen` is right.
  Floors: 17 caught, and no finding at all in a correct sentence.

## 7. Where it reaches (R9)

| Client | What it gets |
|---|---|
| `grind` | `grind text grammar [range] [--picky]` lists findings, with the remembered style level and ignored rules; `grind text correct` takes a finding's address and text; `grind grammar [builtin\|off] [--style on\|off] [--ignore RULE] [--unignore RULE]` sets and prints the setting. `grind lint` and `grind text lint` get rules `grammar` and `style` |
| `grind-text-gtk` | The underlines (blue, dotted for style); the finding leads the right-click menu with its message, replacements and *Ignore Rule*; a Grammar submenu beside Spelling with *Check Grammar* and *Style Suggestions*, remembered |
| `grind-tui` | The underline (blue, grey for style; spelling's red kept), `}`/`{` to the next or previous finding with the message and numbered replacements on the status line, `:fix N`, `:grammar [on\|off\|style on\|off\|ignore]` remembered |
| `grind-win32` | The squiggle in the accent (dotted for style); a finding leads the context menu with its message, replacements and *Ignore Rule*; Ctrl+F7 to the next finding; View ▸ *Check Grammar* and *Grammar Style Suggestions*, remembered |
| `grind-mac` | Green dots under a finding — AppKit's own mark for grammar, so the Mac keeps it rather than the accent (muted for style); the finding leads the context menu with its message, replacements and *Ignore Grammar Rule*; Edit ▸ Spelling ▸ *Check Grammar With Spelling* (AppKit's name), *Grammar Style Suggestions* and *Next Grammar Finding* (⌥⌘;), remembered. Type-checked; the drawing is tested on Linux |
| `grind-web` | The built-in checker in the wasm module: the underline, the palette's rows for the finding at the caret (replacements, *Ignore this rule*), *Go to the next grammar finding*, *Grammar: check / off / style suggestions*, remembered in `localStorage` (`grind.grammar`) |
| `grind lsp` | Nothing at first: projection files are code, not prose |

`doc/feature-matrix.md` gets a *Grammar* row and `doc/cli-parity-text.md` the verbs, so
`cli/tests/parity.rs` holds them.

## 8. Milestones

| | What | Done when |
|---|---|---|
| **G0** | This document; spelling's decision 7 points here | done, 2026-10-30: §9's questions answered |
| **G1** | Core: `Finding`, `Checker`, findings cached per paragraph, `Viewport::flagged`, `App::replace_span`, `App::grammar_found`; `Common` reachable (decision 4) | A fake checker's findings reach the viewport, survive an unrelated edit, vanish with an edit inside them, and a stale answer is dropped |
| **G2** | `grind-grammar` with the language-neutral rules and English; `grind text grammar`; lint rules | Fixtures per rule; corpus precision measured and recorded; `sample-text.sh` gains a line |
| **G3** | German rules | As G2, on German text — which needs a German corpus first, the gov.uk one being English |
| **G4** | GTK text shell | Rendered by `--render-to`; the popover's actions walked |
| **G5** | `grind-languagetool`, the default feature, `--languagetool`, `grind grammar --check-server`, the public server's throttle and notice | Built 2026-10-30, **removed 2026-11-07** (§5) |
| **G6** | Loop G′ | The report over the corpus, a first `FLOOR`, and the backlog in §10 |
| **G7** | TUI, Windows, Mac, browser | A filled feature-matrix row each |

G1–G4 make a useful product without LanguageTool, which is what is left after §5.

## 9. Open questions

1. ~~Which LanguageTool servers~~ — **decided 2026-10-30**: self-hosted, public and Premium;
   **superseded 2026-11-07** by §5: none.
2. ~~Default build~~ — **decided**: in the default build, doing nothing until configured;
   **superseded 2026-11-07** by §5.
3. ~~Built-in languages~~ — **decided**: English and German. The other five get the
   language-neutral rules, and rules of their own when someone asks (§4).
4. ~~Style rules~~ — **decided**: all off by default, behind *Style suggestions* / `--picky`
   (§4).

None is open.

## 10. Gaps, named

- **The Mac has never run it**: type-checked and its portable half tested, like the rest of
  that shell, until a run on GitHub's macOS runners (`scripts/mac-remote.sh`).
- **Recall is measured on a small written set only** (loop G′'s 31 slips). The backlog
  LanguageTool showed there before §5 — agreement above all — is the next set of rules worth
  writing.
