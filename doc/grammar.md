<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

# Grammar and style

**Status: G0–G7 built (2026-10-31): every client checks grammar; see §10 for what is left.** Two checkers behind one door, each optional: a **built-in**
checker, rules written here in plain Rust with no dependencies, small and always available; and
an **integration with LanguageTool**, for whoever runs a LanguageTool server and wants the best
checker there is. Either, both or neither may be on. This supersedes `doc/spelling.md` decision 7
("No grammar checker") and answers each of its three reasons (§2).

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

| Harper's objection | Built-in checker | LanguageTool |
|---|---|---|
| Its spelling is worse than ours (underlines `ODF`, ignores the personal list) | Has no spelling rules. Spelling stays Hunspell's | Its spelling category is always switched off in the request (decision 6) |
| English only, and a German paragraph drew seventeen findings | Rules are per language, and run only on a paragraph whose language is known (decision 4). Language-neutral rules run everywhere. English and German first | Covers both, and more than the seven we spell |
| 11 MB, 92 crates, an ML framework in every build | A few hundred kB of rules, no new crate in `Cargo.lock` | A separate program. What we carry is an HTTP client behind a feature (decision 8) |

## 3. Decisions

1. **The core decides where a finding is. A checker only finds it.** The shape is spelling's: a
   generic `grind_core::grammar` module holds the types (`Finding`, a `Checker` trait) with no
   document vocabulary (R8). `grind-text` hands a checker each paragraph's text with the ranges
   that must not be checked: decision 3 of spelling's skip rules (links, monospace runs, URLs,
   paths). It keeps the findings per paragraph, keyed by the paragraph's text, so an edit
   invalidates only what it touched. Findings travel to shells in the `Viewport`
   (`Viewport::flagged`, beside `misspelt`). No shell runs a rule or makes a request itself.

2. **A finding is a range, a rule, a kind, a message and replacements.** Character offsets into
   the paragraph (0-based, as everywhere in the core). A rule id stable enough to be ignored by
   name (`builtin:repeated-word`, `lt:EN_A_VS_AN`). The kind is one of **grammar**,
   **punctuation**, **typography** or **style**, and the kind decides the underline: grammar and
   punctuation in the theme's blue, typography and style dotted and quieter — the Mac keeps
   AppKit's own green dots for grammar. Spelling's red stays spelling's alone. A replacement may
   be empty, which means "delete this".

3. **A correction is an edit in the core.** `App::correct(at, expected, with)` — spelling's,
   which already replaced any span and not only a word. It is one undo step, keeps the
   formatting of the first character replaced, and is refused when the text at `at` is no
   longer `expected`. The refusal matters for LanguageTool, whose answer can arrive after the
   person typed on.

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
   which checkers are on, the server's address and the style level. Nothing about grammar is
   ever written into a document.

6. **LanguageTool never checks spelling for us.** Every request sends
   `disabledCategories=TYPOS`. Its spelling would underline `ODF` again, would not know the
   personal list, and would draw a second, different red line under what Hunspell already
   judged. LanguageTool's spelling stays off even when Hunspell is off.

7. **The built-in rules are ours, written clean.** LanguageTool's rule files are LGPL-2.1. The
   built-in checker copies none of them, the way this project copies nothing from LibreOffice.
   Each rule is written from the grammar of the language and our own examples. LanguageTool is
   a **conformance oracle** for it (§6, loop G′), never a source.

8. **The integration is in the default build and does nothing until configured** (decided
   2026-10-30). A cargo feature `languagetool` on `grind-cli` and on each shell, like `spell`
   and `pdf`, and on by default. `--no-default-features` still builds without it, and CI builds
   that too. At run time nothing is sent anywhere until the person names a server. There is
   **no default server**. What the HTTP client adds was measured (2026-10-30, a release
   `grind` on Linux): **2.3 MB** stripped (42.6 MB against 40.4 MB without the feature), and
   sixteen crates — `ureq` and its HTTP parts, `rustls`, `ring` and the certificate reading —
   with three more each on Windows and macOS for their certificate stores. All are MIT, Apache-2.0,
   ISC or BSD-3-Clause. Certificates are the **operating system's own** (`rustls-native-certs`),
   not a bundled copy of Mozilla's: the bundled lists (`webpki-roots`, `webpki-root-certs`) are
   CDLA-Permissive-2.0, which is not on the approved licences, and the system's store also
   honours an organisation's own certificate authority.

9. **Three kinds of server, chosen by name** (decided 2026-10-30):
   - **Self-hosted**: any URL, typically a container on this computer (§5, *Setting one up*).
   - **Public**: LanguageTool's free API, `https://api.languagetool.org`.
   - **Premium**: `https://api.languagetoolplus.com` with a username and API key. The key comes
     from the environment (`GRIND_LANGUAGETOOL_KEY`, with `GRIND_LANGUAGETOOL_USER`), never
     from a file we write.

   **Where the text goes is said in words, every time it changes.** A server on `localhost` or
   `127.0.0.1` is "checked on this computer". Any other host is "your text is sent to *host*",
   shown when it is chosen and in the status bar while it is on.

   **The public API comes with its terms, and the client keeps them.** LanguageTool's public
   API page (read 2026-10-30) sets 20 requests a minute, 75 KB of text a minute and 20 KB a
   request per IP address. It asks for a clearly visible link back to languagetool.org and that
   users are told how their text is handled. It also says automated requests are not allowed.
   So with the public server:
   - the client throttles itself below those limits and batches paragraphs, rather than letting
     the server refuse it;
   - the status chip reads *Checked by LanguageTool* and links to `https://languagetool.org`;
   - choosing it shows a one-time notice that the text goes to LanguageTool, with their privacy
     policy's address;
   - checking runs only on what the person edits and on an explicit *Check Document*. The CLI
     refuses to batch a whole file to the public server unless `--public-ok` is given.

   Whether an editor checking as one types counts as "automated" under their terms is theirs
   to say. These measures keep the traffic within what one person typing produces.

10. **The core does no I/O.** A checker that answers at once (built-in) is called from the
    core's worker. A checker that answers later (LanguageTool) is driven from outside the core:
    `grind-languagetool` on a native thread, the browser's `fetch` in `grind-web`. It hands
    results back through `App::grammar_found(block, text_hash, findings)`, which drops any
    answer for text that has since changed. The core pushes the change to observers (rule 3).
    So the browser and a native window share every line of the core, and only the transport
    differs.

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

## 5. The LanguageTool integration

A new crate, **`grind-languagetool`**: the client, native only (the browser uses `fetch`
through the same request and response code, §3.10).

- **The request.** `POST {server}/v2/check`, form-encoded: `data` (below), `language`,
  `disabledCategories=TYPOS` (decision 6), `disabledRules` (the person's ignored `lt:` rules),
  `level=picky` when style is on. The language is the paragraph's (decision 4) when known.
  Otherwise it is `auto` with `preferredVariants` from spelling's choice (`en-US,de-DE`;
  LanguageTool wants a variant for English, German and Portuguese).
- **Markup, not plain text.** The `data` form takes an `annotation` array of `text` and `markup`
  items, and markup is not checked. The ranges decision 1 skips (links, code, URLs) go as
  markup, so LanguageTool never sees them as prose and its offsets still count them.
- **Batching.** Paragraphs that changed are sent together, separated by a markup `\n\n`, up to
  the server's per-request size (20 KB on the free tier, unlimited on one's own server), and
  the findings are split back by offset. A paragraph being typed in is not sent until typing
  pauses (about 800 ms), and never more than one request is in flight per document.
- **Offsets** — `MEASURED` (2026-10-30, LanguageTool 6.8 in the `erikvl87/languagetool` image).
  The API documents them as "characters"; they are **UTF-16 code units** (`go` after an emoji
  came back at 6, where it is the fifth character), counted over **everything sent, markup
  included** (a match after a markup item counted the markup's length). The client converts
  them to characters per paragraph, and drops a match that crosses a paragraph's end or falls
  inside markup.
- **Mapping a match.** `rule.id` → `lt:<id>`. `rule.category.id` and `rule.issueType` → our four
  kinds (`TYPOGRAPHY` → typography, `STYLE`/`REDUNDANCY`/`PLAIN_ENGLISH` → style,
  `PUNCTUATION` → punctuation, everything else → grammar). `message`, the first five
  `replacements`, and `rule.urls[0]` as a "why" link where a shell can open one.
- **Failure is quiet and visible.** A refused connection, a timeout (5 s) or a 4xx/429 turns the
  checker's chip to *LanguageTool unavailable*, keeps the built-in findings, and retries with
  backoff. It never blocks typing, and never shows a dialog more than once.
- **Both at once.** With both checkers on, the built-in's findings show at once and
  LanguageTool's arrive later. Where both flag the same range, LanguageTool's wins and the
  built-in one is dropped, so one phrase never carries two underlines.
- **Setting one up.** The guide offers three routes, simplest first:
  1. **A container.** `docker run --rm -p 8010:8010 erikvl87/languagetool` (or `podman run`),
     then `grind grammar http://localhost:8010`. This is a community image
     (Erik van Leeuwen, LGPL-2.1), not one LanguageTool publishes. It takes `Java_Xmx` for
     memory and an n-gram directory mounted at `/ngrams` for LanguageTool's better
     confusion-word rules. The guide names the image and its version as tested, and says what
     changes if it goes away: any server that speaks `/v2/check` will do.
  2. **The `languagetool` system package**, where a distribution has one:
     `languagetool --http --port 8081 --allow-origin "*"`.
  3. **LanguageTool's server jar** with Java 8 or later.

  `--allow-origin` is what the browser shell needs (the container sets it). `grind grammar
  --check-server` checks one sentence and reports the version and where the text goes, the
  first thing to try when it does not work.
- **Proving it against a real server.** G5's offset check and loop G′ run against the
  container, started by `scripts/` the way `soffice-docker` starts the LibreOffice oracle, so
  "a server is available" means "podman or docker is".

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

- **Loop G′: the built-in against LanguageTool**, where a server is available
  (`GRIND_LANGUAGETOOL_URL`, skipped with a notice without one, as loops A and B skip). Over the
  same corpus: a finding of ours that LanguageTool does not make is a **suspected false
  positive**, and is reviewed. A finding of LanguageTool's that a built-in rule could make is
  the **backlog**. The loop is a report and a `FLOOR` on agreement, not an equality.
- **Loop G′, first run** (2026-10-30, `grammar/tests/loop_g_prime.rs`, LanguageTool 6.8 in the
  container). Forty-seven sentences written for it (`grammar/tests/data/samples.txt`, CC0):
  31 with one slip each, 16 correct but tempting. The built-in checker found 17 of the 31 and
  nothing in the 16. Both found 12. The built-in checker alone found 5 that LanguageTool missed
  (`better then`, `its a`, `it's own`, a lone `i`, `ended.Then`), all real slips, so
  LanguageTool is not a superset and running both is worth it. LanguageTool alone found 9, the
  backlog: verb agreement (`He go`, `They was`, `She don't`, `has went`), `informations`,
  `discuss about`, `more taller`, `if I would have known`, and German `die die Bücher`, which the
  built-in checker leaves alone on purpose because `Frauen, die die Bücher lasen` is right.
  Floors: 17 caught, 12 agreed, and no finding at all in a correct sentence.

- **The client** is tested against a local mock (`std::net::TcpListener` answering canned JSON):
  offsets past a non-BMP character, a batch split back into paragraphs, a stale answer dropped,
  429 and timeouts. No test in CI reaches the network.

## 7. Where it reaches (R9)

| Client | What it gets |
|---|---|
| `grind` | `grind text grammar [range] [--picky] [--builtin \| --languagetool <url\|public\|premium>] [--public-ok]` lists findings, following the remembered setting unless told otherwise; `grind text correct` takes a finding's address and text; `grind grammar [builtin\|off\|<url>\|public\|premium] [--with-builtin] [--style on\|off] [--ignore RULE] [--unignore RULE] [--check-server]` sets and prints the setting. `grind lint` and `grind text lint` get rules `grammar` and `style`, **built-in only**, because a lint must answer the same twice and must not need the network. `--languagetool` opts in |
| `grind-text-gtk` | The underlines (blue, dotted for style); the finding leads the right-click menu with its message, replacements and *Ignore Rule*; a Grammar submenu beside Spelling with *Check Grammar*, *Style Suggestions* and *LanguageTool Server…* (an address, `public`, `premium`, or empty for off), remembered; a toast says where the text goes, and whether the server answered |
| `grind-tui` | The underline (blue, grey for style; spelling's red kept), `}`/`{` to the next or previous finding with the message and numbered replacements on the status line, `:fix N`, `:grammar [on\|off\|style on\|off\|ignore\|<url>\|public\|premium]` remembered; LanguageTool asked on a thread once typing pauses, the event loop woken for its answer only while one is wanted |
| `grind-win32` | The squiggle in the accent (dotted for style); a finding leads the context menu with its message, replacements and *Ignore Rule*; Ctrl+F7 to the next finding; View ▸ *Check Grammar*, *Grammar Style Suggestions* and *LanguageTool Server…*, remembered. LanguageTool is asked through **WinHTTP**, not the crate's own client: `ring`'s C would not cross-check from Linux, as `win32.yml` does, and WinHTTP brings the system's certificates and proxy |
| `grind-mac` | Green dots under a finding — AppKit's own mark for grammar, so the Mac keeps it rather than the accent (muted for style); the finding leads the context menu with its message, replacements and *Ignore Grammar Rule*; Edit ▸ Spelling ▸ *Check Grammar With Spelling* (AppKit's name), *Grammar Style Suggestions*, *Next Grammar Finding* (⌥⌘;) and *LanguageTool Server…*, remembered. LanguageTool through the system's `curl`, on a thread, the answer taken on the caret's blink — `ring` would not type-check for this target from Linux either. Type-checked; the drawing and `curl` are tested on Linux |
| `grind-web` | The built-in checker in the wasm module: the underline, the palette's rows for the finding at the caret (replacements, *Ignore this rule*), *Go to the next grammar finding*, *Grammar: check / off / style suggestions*, remembered in `localStorage` (`grind.grammar`); *Grammar: LanguageTool server…* (an address, or `public`) asked through `fetch` once typing pauses — Premium is not offered, since the page keeps no key |
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
| **G5** | `grind-languagetool`, the default feature, `--languagetool`, `grind grammar --check-server`, the public server's throttle and notice | Mock-server tests; UTF-16 offsets verified against the container; binary size, crates and licences measured and recorded; the `--no-default-features` build still passing |
| **G6** | Loop G′ | The report over the corpus, a first `FLOOR`, and the backlog in §10 |
| **G7** | TUI, Windows, Mac, browser | A filled feature-matrix row each |

G1–G4 make a useful product without LanguageTool, and G5 is independent of G3, so the order can
bend.

## 9. Open questions

1. ~~Which LanguageTool servers~~ — **decided 2026-10-30**: self-hosted (the container first),
   public and Premium, all three (decision 9).
2. ~~Default build~~ — **decided**: in the default build, doing nothing until configured
   (decision 8).
3. ~~Built-in languages~~ — **decided**: English and German. The other five get the
   language-neutral rules, and rules of their own when someone asks (§4).
4. ~~Style rules~~ — **decided**: all off by default, behind *Style suggestions* / `--picky`
   (§4).

None is open.

## 10. Gaps, named

- **Choosing a server in a window.** The GNOME window follows `grind grammar`'s setting and
  switches the built-in checker and style, but has no dialog to name a server; and while the
  public server is in use it says where the text goes in a toast, not in the clearly visible
  link back its terms ask for.
- **The Mac has never run it**: type-checked and its portable half tested, like the rest of
  that shell, until a run on GitHub's macOS runners (`scripts/mac-remote.sh`).
- **Premium in the browser**: not offered, since the page keeps no key.
- **Recall is measured on a small written set only** (loop G′'s 31 slips). The backlog there —
  agreement above all — is the next set of rules worth writing.
