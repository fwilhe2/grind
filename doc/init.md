<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# `grind init` — a project for the two languages, and what comes after it

**Status: I0 done (2026-10-11); I1 onwards planned, not built.** Written 2026-10-11 from the request *"the CLI should have an
`init` command that sets up a workspace for working with a `.rhai` or `.grind` in VS Code or
other editors — environment, extensions, snippets, run commands, as complete an experience as
possible; later, perhaps a VS Code extension that calculates values inline."* §3 is the part
the request asked for most directly: the assumptions in it, challenged one at a time, each
with what this plan does about it.

Milestones are **I0–I7** (§7). I0 is three small things the CLI owes *before* an `init` is
worth writing; I1 is `init` itself; I5 onwards is the inline-values half, and it changes shape
under §3's challenges more than anything else here.

---

## 1. What exists today

Setting up an editor is already documented and already works — by hand.
`doc/editor-setup.md` §3 is the recipe for "your own project", and it is three commands, one
copied file and one hand-written settings file:

| Piece | Where it comes from today | Kept current by |
|---|---|---|
| `.vscode/grind.code-snippets` — the generator's vocabulary | `grind definitions --snippets` | regenerating after an upgrade (by hand) |
| `grind.d.rhai` — the same, as a Rhai definition file | `grind definitions` | the same |
| `.vscode/grind-projection.code-snippets` — the projection's nodes | **copied out of this repository**; written by hand, held to the grammar by `cli/tests/editor.rs` | nothing, once copied |
| `.vscode/settings.json` — `*.grind` → `kdl`, snippets on top | typed by hand from §3 | — |
| `.vscode/extensions.json` — `kdl-org.kdl`, `rhaiscript.vscode-rhai` | this repository's | — |
| `.vscode/tasks.json` — lint and build on a key | typed by hand from §6, with an approximate problem matcher | — |

What the editors are, measured (`doc/editor-setup.md` §4–§5): `kdl-org.kdl` 2.1.3 is a KDL v2
grammar with a language server for *syntax*; `rhaiscript.vscode-rhai` 0.6.9 is a grammar and
nothing else — its language client can never start. Snippets were chosen over a language
server because no usable Rhai server exists, and `doc/dsl.md` §9 declines to write one.

So the honest one-line description of `grind init` is: **`doc/editor-setup.md` §3 and §6,
executed** — plus the things §3 below argues a project wants and that document never covered.

---

## 2. What `init` had to know that it did not

Three facts surfaced while reading the CLI for this plan. Each blocked part of a good `init`,
each was small, and **all three are I0 and done** — what was built follows each one:

1. **`grind lint` cannot say a line.** Its location is an address in the document's own
   spelling (`Quote.C5`, `#intro`), in both the text and the JSON form. An editor's problem
   matcher wants `file:line:column`, so `doc/editor-setup.md` §6's matcher is admittedly
   approximate and a lint finding never lands on its line. For a `.grind` the answer already
   exists — the span map (`grind sheet project --anchors`) is address → line and byte range —
   it is just not consulted.
   *Built:* `grind_core::projection::Source::locate` turns an address into a line and a
   character column in the file **as read**, `lint::Report::locate_in` applies it after the
   rules, and both applications' `App::lint` call it when the document came from a `.grind`.
   The CLI prints `file:line:col: address: severity: …` and the JSON gains a `location`
   object; a finding with no spelled address prints as before. `doc/editor-setup.md` §6's
   matcher now takes the line, the column, and the address as the problem's code.
2. **There is no suite-level `project`.** `grind sheet project` and `grind text project` print
   a projection; a caller that does not know the kind first (git's `textconv`, §3.8) has no
   verb. `grind info` and `grind lint` already read the kind out of the file, so this is the
   same shape.
   *Built:* `grind project <file> [--tokens|--anchors]`, the application verb's own function
   behind a kind check. Checked with a real repository: with §3.8's two lines, `git diff`
   shows an edited `.ods` as `-row 2` / `+row 5` and an `.odt` as the paragraph that changed.
3. **The projection's snippets are not in the binary.** They are a file in this repository,
   `include_str!`'d only by a test. An `init` run against an installed `grind` has nowhere to
   get them from.
   *Built:* `grind definitions --projection` prints the file, compiled in from `.vscode/` so
   there is still one copy; `cli/tests/editor.rs` asserts the binary's output is that file
   byte for byte.

Two smaller things noticed on the way, both fixed with I0: `grind convert --help` said *"A
text document has no projection yet and says so"*, which D3 had made untrue; and `grind lint
examples/quote.grind` reported eight `misspelt` warnings, because the quote was written in
British English and is checked as `en-US` — it and the timesheet are now spelt the American way
(*Labor*, *meters*, *edging*). That matters to §4.4: whatever template `init` writes must lint
clean.

---

## 3. The assumptions, challenged

### 3.1 "init creates a workspace/project" — does grind need a *project* at all?

Everything in this suite so far is **a document is a file**; `doc/dsl.md` §10 names the image
sidecar directory as a risk *because* it breaks that. A `grind.toml` that lists scripts and
their outputs would be the first project-level concept, and it would grow: targets, data
directories, a default locale, then dependencies between outputs — which is a build system.

**Decision: no manifest.** `init` writes editor and repository configuration around ordinary
files and nothing that `grind` itself reads back. A task builds `${file}`, not a target. Every
file `init` writes is one a person could delete without any `grind` command behaving
differently. If a manifest ever earns its place it will be by the same rule as anything else in
`doc/not-doing.md` — evidence first — and the likeliest evidence is §3.7's CI job wanting to
know which scripts to build.

### 3.2 "set up the right environment/extensions" — a CLI should not install into your editor

`code --install-extension` changes the user's editor for every project they have, needs
`code` on `PATH`, and does something different in VSCodium, Cursor and a remote container.
That is not a side effect a command named `init` should have.

**Decision: recommend, never install.** `init` writes `.vscode/extensions.json`, and VS Code
offers the two extensions itself the first time the folder opens — the same mechanism this
repository uses. `init` prints the two `code --install-extension` lines for anybody who wants
them now. Nothing outside the target directory is written, ever (I1's test asserts it).

### 3.3 "VS Code or other editors" — the cost is a matrix, so pick the cells

Every editor has its own spelling of the same four facts: *`.grind` is KDL*, *`.rhai` is
Rhai*, *here is the vocabulary*, *here are the commands*. Support is cheap where the editor
keeps project configuration in a file and expensive where it does not:

| Editor | Project file | `.grind` → KDL | Vocabulary | Commands | Verdict |
|---|---|---|---|---|---|
| VS Code (and VSCodium, Cursor) | `.vscode/` | `files.associations` | snippets (§1) | `tasks.json` | **I1** |
| Zed | `.zed/settings.json`, `.zed/tasks.json` | `file_types` | none without a server | `tasks.json` | **I3** — association and tasks only |
| Helix | `.helix/languages.toml` | `file-types` on the `kdl` language | none without a server | none | **I3** — association only |
| Neovim, Emacs | no project convention (`exrc` is opt-in and off by default) | — | — | — | **not written**; `doc/editor-setup.md` §7 says what to add |
| Any | `.editorconfig` | — | — | — | **I1** — indent and final newline, which every editor above reads |

**Decision:** `grind init --editor vscode` is the default and writes the VS Code files;
`--editor zed`, `--editor helix` and `--editor none` are I3; several may be named. The real
cross-editor answer is a language server (§3.9), and the matrix above is the argument for
building one rather than five sets of configuration.

### 3.4 "as complete an experience as possible" — complete against *what*?

The suite's stated character is *a feature list that ends*. "As complete as possible" has no
end. The useful reading is **complete against a list**: everything `doc/editor-setup.md` tells a
person to do by hand, plus what §3.6–§3.8 argue a repository of documents wants. The scope
line for `init` is therefore:

> `init` writes a file only if a person following this repository's own documentation would
> otherwise have had to write it by hand.

That admits snippets, settings, tasks, recommendations, a starter document, `.gitignore`,
`.gitattributes`, `.editorconfig` — and keeps out `launch.json` (there is no debugger, and a
launch configuration that only runs a command is a task wearing a costume), a `README` (the
author's, not a generator's), and a `Makefile` (a second, worse copy of `tasks.json`).

### 3.5 "init" once — but the generated half goes stale

Snippets and `grind.d.rhai` are the *installed binary's* answer to "what can a script say".
After `grind` is upgraded they describe the old vocabulary, and nothing says so.
`doc/editor-setup.md` §8 lists that as a troubleshooting row, which is a symptom of the
design.

**Decision:** the files split into two kinds and `init` treats them differently.

| Kind | Examples | First `init` | Second `init` | `init --update` |
|---|---|---|---|---|
| **Generated** — the binary's own answer, owned by `grind` | `grind.code-snippets`, `grind-projection.code-snippets`, `grind.d.rhai` | written | left alone, reported stale if the version differs | **rewritten** |
| **Seeded** — a starting point, owned by the user from then on | `settings.json`, `tasks.json`, `extensions.json`, the starter document, `.gitignore`, `.gitattributes`, `.editorconfig` | written | **never touched** | **never touched** |

A generated file carries a first-line comment — `// Generated by grind 0.x.y; regenerate with
grind init --update` — which `.code-snippets` (JSONC) and `.d.rhai` both allow. That comment is
how a second `init` knows a file is stale, and how a person knows not to edit it.

### 3.6 Overwriting the user's settings is the failure that loses trust

A user who already has `.vscode/settings.json` with their own formatter, theme or Python path
does not want it rewritten, and JSONC with comments cannot be merged by a JSON library without
losing them. This is `CLAUDE.md`'s rule 7 — *saving never makes an existing file worse* —
applied to configuration.

**Decision: never clobber, never merge silently.** A seeded file that exists is left exactly as
it is, and `init` prints the keys it would have added, so a person can paste them. `--force`
overwrites seeded files and says which. There is no merge in I1; if it proves wanted, it
merges only top-level keys `init` owns, via a JSONC parser that keeps comments, and is its own
milestone.

### 3.7 Who is this for? Probably not who a "workspace" suggests

A person writing a `.grind` or `.rhai` by hand already lives in a text editor and in git; a
person who wants a spreadsheet opens the GTK window and never meets `init`. That changes what
"complete" means for them: **what they lack is not editor chrome but the repository side** —
diffs that read, outputs kept out of history, and a check that runs without them.

So `init` is also, and perhaps first, a *repository* setup:

- **`.gitignore`** — built outputs (`*.fods`/`*.fodt` produced from a `.rhai`) are derived and
  stay out of history; `doc/flat-first.md` is why a *hand-edited* `.fods` is still welcome in
  it. Seeded with the starter's output name and a comment, never with a blanket `*.fods`.
- **A readable `git diff` for packages** — §3.8.
- **CI, opt-in** (`--ci github`): one workflow running `grind test` on every script and `grind
  lint` on every document, in the published `ghcr.io/fwilhe2/grind` image, so the check runs
  where the user's collaborators push. Opt-in because a workflow file is the one thing `init`
  could write that runs on somebody else's machines.

### 3.8 The single most useful file may be `.gitattributes`

A `.ods` in git is a binary blob; its diff is "Binary files differ". The projection is exactly
the text a diff wants, and git has the hook for it: a `diff` attribute with a `textconv`
command. Two lines make `git diff`, `git log -p` and every forge's local review show a changed
`.ods` as the cells that changed:

```gitattributes
*.ods  diff=grind
*.odt  diff=grind
```

```sh
git config diff.grind.textconv "grind project"
```

The catch, and why this is a design decision rather than a template line: **`textconv` lives in
`.git/config`, which is not committed**, so a clone does not get it. `init` can set it in the
repository it is run in (and does, when that is a git repository), and the README line it prints
tells collaborators the one command. This needs I0's suite-level `grind project`.

### 3.9 "Later, a VS Code extension that calculates values inline" — three challenges

**(a) For a `.grind`, the values are already inline.** A cached value is part of the
projection; `grind sheet recalc quote.grind` fills every answer in, one line of diff per cell
(`doc/projection-guide.md` §4). So "calculate inline" for the projection splits into two
different features:

- **write the answers** — exists; it is a task bound to a key (I1 ships it);
- **show the answers without writing them** — new, and the right analogue is
  `doc/view-modes.md`: what a document *means*, drawn, never stored. That is what an editor's
  *inlay hints* are for.

**(b) It should not be a VS Code extension; it should be a language server.** Inlay hints,
hover, diagnostics on the right line, go-to-definition for a name, completion — all of it is
in the Language Server Protocol, which VS Code, Zed, Helix, Neovim and Emacs all speak. A VS
Code extension would answer one cell of §3.3's matrix; `grind lsp` answers the row. The
extension then shrinks to what VS Code requires of any server: a few dozen lines of client,
a language contribution for `.grind`, and the grammar it already gets from `kdl-org.kdl`.

That collides with `doc/dsl.md` §9's *"Tooling for the generator — an LSP … Writing a second
one is how a document project becomes a language project"*, and the collision should be
resolved rather than ignored:

- for **`.grind`** it does not apply: the projection *is* this project's language, the span map
  was built as "the thing every later IDE feature is built on" (`grind sheet project --help`),
  and every answer a server gives comes from the core — the server holds no knowledge of its
  own;
- for **`.rhai`** it applies as written to *language* analysis — types, scopes, completion of
  local variables — and this plan keeps out of that. What a `grind lsp` offers a `.rhai` is
  only what `grind build` and `grind test` already compute: errors at their line, test
  results at their `fn`, and the values of §(c). It never parses Rhai itself.

§9's row should be amended to say that, by the author, once I5 is decided.

**(c) For a `.rhai`, "inline" needs something that does not exist: provenance.** A script's
line `s.push(row("Rent", rent))` produces a cell, but nothing records which line produced which
cell, so there is nowhere to draw the answer. Rhai 1.26 gives a host function its call site
(`NativeCallContext::call_position`), so `grind-build` can record, for every cell and block it
materialises, the line and column of the call that wrote it. Two honest consequences:

- a loop produces many cells from one line, so a hint reads `→ C5:C94, Σ 41 280` or `→ 90
  rows`, not ninety values;
- a helper function's `push` is the call site, not the line that called the helper — the
  hint lands inside the helper. Walking up Rhai's call stack is possible and is a later
  refinement, not I6.

### 3.10 Starting from the CLI keeps the core honest

A tempting shortcut for inline values is a TypeScript extension that shells out to `grind sheet
view --format json` per keystroke and maps addresses to lines itself. It would work for small
files, and it would put a second copy of the address-to-line logic in TypeScript where nothing
tests it against loop F. The plan's order — span map consulted by the CLI first (I0), then a
server in Rust that owns the mapping (I5) — keeps the mapping in one place, which is the same
reason `grind_sheet::formula::assist` was hoisted out of a shell.

---

## 4. `grind init` — the command

### 4.1 Synopsis

```console
$ grind init [DIR] [--template sheet|text|generator|none] [--editor vscode|zed|helix|none]...
             [--ci github] [--force] [--update]
```

`DIR` defaults to the current directory and is created if missing — `cargo init` and `cargo new`
in one verb, because grind has no package to name. `--update` rewrites the **generated** files
only (§3.5) and implies nothing else. Everything is reported, one line per file, with what was
done to it: `wrote`, `kept`, `stale`, `updated`, `would add` (§3.6).

### 4.2 What it writes (VS Code, the default)

| File | Kind | Content |
|---|---|---|
| `.vscode/extensions.json` | seeded | `kdl-org.kdl`, `rhaiscript.vscode-rhai` |
| `.vscode/settings.json` | seeded | `*.grind` → `kdl`, snippets on top, four-space indent — `doc/editor-setup.md` §3, verbatim |
| `.vscode/tasks.json` | seeded | §4.3 |
| `.vscode/grind.code-snippets` | generated | `grind definitions --snippets` |
| `.vscode/grind-projection.code-snippets` | generated | the projection's nodes, from the binary (I0) |
| `grind.d.rhai` | generated | `grind definitions` — kept for the reading, and for a Rhai server if one appears |
| `.editorconfig` | seeded | indent and newline for `*.grind` and `*.rhai` |
| `.gitignore` | seeded | the starter's built output, with a comment saying why |
| `.gitattributes` | seeded | §3.8 |
| the starter (§4.4) | seeded | one document or script, which lints and tests clean |

### 4.3 The tasks

One task per verb a person runs while writing, each on `${file}` so no target list is needed
(§3.1), each with a problem matcher that lands on a line once I0 is done:

| Task | Command | Group |
|---|---|---|
| Build this script | `grind build ${file} -o ${fileDirname}/${fileBasenameNoExtension}.fods` | build (default) |
| Test this script | `grind test ${file}` | test (default) |
| Lint this document | `grind lint ${file}` | test |
| Recalculate this document | `grind sheet recalc ${file}` — writes the answers (§3.9 a) | none |
| Open in grind | `grind-sheet-gtk` / `grind-text-gtk` / `grind-tui` on the built output | none |

`grind` is called as `${config:grind.path}`, defaulting to `grind`, so a user with the binary
somewhere odd sets one key rather than editing five tasks. (VS Code warns that the key is
unknown until an extension declares it; I6's client declares it.)

### 4.4 The starter

`--template` picks it; `generator` is the default, since a script is where an editor helps
most:

| Template | Writes | Must |
|---|---|---|
| `generator` | `model.rhai` with two `fn test_…`, and `data.json` beside it | `grind test` passes, `grind build` output lints clean |
| `sheet` | `book.grind` — a small hand-written projection with names and formulas and no cached values | `grind lint` clean, including spelling (§2), and `recalc` fills it |
| `text` | `notes.grind` — the text projection's headings, a list, a table | `grind lint` clean |
| `none` | nothing | — |

Templates are compiled into the binary (`include_str!`), checked by the same test that runs
them, and are deliberately *small*: `doc/generator-guide.md` and `doc/projection-guide.md` are
the tutorials, and the starter's first comment points at the right one.

---

## 5. The language server — the later half

`grind lsp`, a subcommand speaking LSP over stdio, in its own crate (`grind-lsp`) behind a
cargo feature like `xlsx`, since it brings an LSP library. It holds open documents as `App`s
with their `projection::Source`, and answers every request from the core:

| Request | `.grind` | `.rhai` |
|---|---|---|
| diagnostics | `lint` findings at their span; KDL syntax errors at their byte | `build`'s error at its line; failing `test_…` at its `fn` |
| inlay hints | each formula's computed value at the end of its line, **never written** | provenance hints (§3.9 c) |
| hover | a cell's value, display, formula in friendly form (`formula::friendly`) | the cells a line produced |
| go to definition | a name's `name` node; a sheet's `sheet` node | — |
| completion | the projection's vocabulary; function names from `funcs::catalog` with `assist`'s signatures | the host vocabulary — what the snippets give today, now context-free of a snippet file |
| code actions | *Recalculate*, *Fix spelling* (the `lint` rule's own suggestions) | *Build*, *Run tests* |

The **VS Code extension** is then a client: it registers `grind` as a language (so `.grind`
no longer needs `files.associations`), contributes `grind.path`, starts `grind lsp`, and
inherits the grammar from `kdl-org.kdl`. Packaging, a publisher id and marketplace
publication are the author's decisions (§8), as is whether it lives in this repository —
which would bring Node into a Rust workspace and a new job into CI.

A **wasm** build of the server — so the extension runs in `vscode.dev` with no binary — is
plausible because `ui_web` already proves the core runs in a browser, and is not planned.

---

## 6. What this plan does not do

| Not doing | Because |
|---|---|
| Install editor extensions | §3.2 — recommendations only |
| A `grind.toml` / project manifest | §3.1 — no evidence yet |
| `launch.json` | there is no debugger; §3.4 |
| Merge into existing settings in I1 | §3.6 — never clobber, print instead |
| Configuration for Neovim or Emacs | no project-file convention to write into; documented instead |
| Rhai language analysis | `doc/dsl.md` §9, unchanged — §3.9 b |
| A TypeScript address-to-line mapping | §3.10 |
| Write computed values into a file from the server | inlay hints are shown, never stored; *Recalculate* is the explicit, writing verb |

---

## 7. Milestones

| | What | Exit criterion |
|---|---|---|
| **I0** — done | The three prerequisites (§2), as built there: `grind lint` reports `file:line:col` for a `.grind` through the span map (and keeps the address); a suite-level `grind project <file>` printing either kind's projection to stdout; the projection snippets compiled into the binary, printed by `grind definitions --projection`. Plus the stale `convert --help` sentence | `doc/editor-setup.md` §6's matcher lands every finding on its line; `git diff` with the §3.8 textconv shows a changed `.ods` as changed cells; `cli/tests/editor.rs` holds the embedded file to the shipped one |
| **I1** | `grind init` for VS Code: §4.2's files, the `generator` template, never-clobber, the report | in a temp dir: the template builds, tests and lints clean; a second run writes nothing; a pre-existing `settings.json` survives byte for byte; nothing is written outside `DIR` |
| **I2** | `--update` and the generated-file header; the `sheet` and `text` templates | upgrading the vocabulary (a test that adds a function) makes `init` report `stale` and `--update` fix it, touching no seeded file |
| **I3** | `--editor zed`, `--editor helix`, several at once | each editor's files written and parsed back (TOML, JSON) by a test |
| **I4** | `--ci github` | the workflow it writes runs green in this repository's own CI on the starter |
| **I5** | `grind lsp` for `.grind`: diagnostics, hover, inlay hints, go-to-definition, completion | an LSP conformance test driving the server over stdio on `examples/quote.grind`; and the decision recorded in `doc/dsl.md` §9 |
| **I6** | The VS Code client extension, and Helix/Zed configuration pointing at `grind lsp` | the extension opens `examples/quote.grind` in a VS Code test host and shows `G14`'s value as a hint |
| **I7** | `.rhai` provenance in `grind-build` and its hints through the server | `examples/timesheet.rhai`'s loop line shows the range it produced; `grind build` output is byte-identical with provenance on and off |

I0 is worth doing regardless of the rest: the line-accurate lint and `grind project` help
every editor and every repository today.

---

## 8. Open questions for the author

1. **Is §3.1 right** — no manifest — or is there a workflow already in mind (several scripts,
   shared data) that wants one now?
2. **Default template**: `generator` (where the editor helps most) or `sheet` (where a newcomer
   is likeliest to start)?
3. **The extension's home**: in this repository (Node in CI, one version number) or its own
   (a separate release cadence)? And publication — VS Code Marketplace, Open VSX, both?
4. **`doc/dsl.md` §9**: amend the LSP row as §3.9 b proposes, or keep the server to `.grind`
   only and leave `.rhai` with snippets?
5. **`--ci github`**: worth carrying a forge-specific file at all, or document the two commands
   and let people write their own?

---

## 9. Risks

**Configuration drift across editors.** Every editor added in I3 is configuration that a new
release of that editor can break, tested only by parsing it back. The language server is the
mitigation, which is why it is the later half rather than an afterthought.

**The generated files are the binary's answer, and a team has several binaries.** Two
collaborators on different `grind` versions will `--update` the snippet file back and forth.
The header makes it visible; nothing makes it go away short of not committing generated files,
which loses offline completion for whoever clones. Leaning: commit them, and let the header say
which version wrote them.

**The server invites becoming an editor.** `doc/dsl.md` §10 already warns that the read-only
code view invites the editable one. A server that answers questions is safe; one that starts
rewriting the user's buffer on save (formatting, recalculating) is that same slope. Code
actions stay explicit.

**`textconv` needs `grind` on every machine that diffs.** A collaborator without it sees a
failed diff command rather than "Binary files differ". `init` sets it only locally, and the
printed line says so.
