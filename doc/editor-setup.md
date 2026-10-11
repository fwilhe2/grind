<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Setting up VS Code for `.grind` and `.rhai`

This project has two languages of its own — the **projection** (`.grind`, a document as plain
text) and the **generator** (`.rhai`, a script that returns a document). Neither is worth much
if writing one means keeping a reference document open beside the editor.

This is how to get highlighting, completion and inline documentation for both. It takes about
two minutes, and every claim in it was checked against the extensions as installed rather than
against their marketing.

* New to the two languages: **`doc/projection-guide.md`** and **`doc/generator-guide.md`**.
* Already set up and looking for the vocabulary: **`doc/projection-sheet.md`** and
  **`doc/generator-spec.md`**.

---

## 1. What you get

| | `.grind` | `.rhai` |
|---|---|---|
| Syntax highlighting | ● KDL grammar | ● Rhai grammar |
| Brace matching, auto-indent, comment toggling | ● | ● |
| Syntax errors underlined as you type | ● from the KDL language server | ○ nothing published does this — §5 |
| Completion for every node / function | ● snippets, once a letter is typed | ● snippets, once a letter is typed — nothing on `s.`, and no filtering by what `s` is |
| A description on the completion item | ◐ one line, behind Ctrl+Space | ◐ the engine's doc comment, as one plain-text detail line behind Ctrl+Space |
| Hover documentation on code already written | ○ needs a language server | ○ needs a language server |
| Types and signatures | ○ the node's example in its description | ● in the snippet's description, and in full in `grind.d.rhai` |
| Actual validation of what you wrote | `grind lint` | `grind build` |

**Measured, not assumed** (2026-10-11, VS Code with both extensions, driven headless through its
own `vscode.executeCompletionItemProvider`): typing `s.pu` offers the three `push` snippets and
`s.` alone offers none, because a snippet is matched on a word and knows nothing about `s`. A
snippet's description arrives as the item's **detail** — one plain-text line, "(Workspace
Snippet)" appended, shown when the suggestion's details are open (Ctrl+Space a second time, or
the `>` on the highlighted entry) — while its *documentation* pane holds only a preview of the
code it inserts. Completing on the dot and hover over existing code are both a language server's,
which is `doc/init.md` I5. **And none of it reaches a `.grind` in an untrusted folder** — §8's
first row.

The last row is not a consolation prize. An editor cannot tell you that a formula reads a
deleted sheet or that a cached total disagrees with its formula; `grind lint` can, and it is one
command. Set it up as a task (§6) and it is one keystroke.

---

## 2. Working in this repository

Nothing to do. Open the repository root as the workspace folder and VS Code will offer the two
extensions it recommends, then read the three files already checked in:

| File | What it does |
|---|---|
| `.vscode/extensions.json` | recommends `kdl-org.kdl` and `rhaiscript.vscode-rhai` |
| `.vscode/settings.json` | maps `*.grind` to the KDL language, and puts snippets at the top of the completion list for both |
| `.vscode/grind.code-snippets` | every function `grind build` registers — **generated**, and a test fails when it is stale |
| `.vscode/grind-projection.code-snippets` | every node of the projection's grammar, for both document types |

Install the two extensions when prompted, or:

```console
$ code --install-extension kdl-org.kdl
$ code --install-extension rhaiscript.vscode-rhai
```

Then open `examples/quote.grind` and `examples/timesheet.rhai` and type. In the first, `cell`
offers a cell node with its formula as a tab stop; in the second, `s.` then `pu` offers `push`
with what it does underneath.

---

## 3. Working in your own project

**One command does all of this section and §6:**

```console
$ grind init ~/work/pricing
```

It writes everything below — the snippets, the definition file, the settings, the tasks of §6,
the two extension recommendations — plus a starter script that builds a spreadsheet, an
`.editorconfig`, a `.gitignore` and a `.gitattributes` that makes `git diff` read an `.ods`. It
never overwrites a file that is already there: a `settings.json` of your own is kept as it is,
and the report says which line it is missing. `doc/init.md` is the design.

What follows is what it does, by hand — for an editor other than VS Code, or a project that
wants only part of it. Four commands and one setting. Say your spreadsheets and scripts live in
`~/work/pricing`:

```console
$ cd ~/work/pricing
$ mkdir -p .vscode
$ grind definitions --snippets   > .vscode/grind.code-snippets
$ grind definitions --projection > .vscode/grind-projection.code-snippets
$ grind definitions              > grind.d.rhai
```

and `.vscode/settings.json`:

```json
{
    "files.associations": {
        "*.grind": "kdl"
    },
    "[kdl]": { "editor.snippetSuggestions": "top" },
    "[rhai]": { "editor.snippetSuggestions": "top" }
}
```

The second file is the projection's nodes. It is written by hand rather than generated, and
the binary carries a copy, so it needs no checkout of this repository.

**`files.associations` is the load-bearing line.** Without it `.grind` is plain text: no
highlighting, no snippets, nothing. With it, VS Code treats the file as KDL, which is what it
is.

**Regenerate the snippets when you upgrade `grind`.** The first file is the engine's own answer to
*what can a script say*, so a new version of the binary may have new functions in it. There is
no harm in running the command in a `Makefile`.

---

## 4. Why snippets rather than a language server

Rhai has a mechanism for exactly this problem. `grind definitions` writes a **Rhai definition
file** — the language server's own format — holding every function `grind build` registers,
with its parameter names, its types and its documentation:

```rhai
/// Append a row, and answer the index of the row it landed on (counted from 0).
///
/// `s.at(s.push(…), 2)` is an address in the row just written.
fn push(sheet: Sheet, row: Row) -> int;
```

That is the right answer, and it needs a language server to read it. **Measured against the
published extension, there is not one available.** What was found, in
`rhaiscript.vscode-rhai` 0.6.9 as installed:

* The extension ships a TextMate grammar and **no server**. Its `package.json` contributes a
  `rhai.useLanguageServer` setting, and its `extension.js` starts a `LanguageClient` whose
  command is the string `rhai-lsp` — a binary expected on `PATH`, not bundled.
* It offers to install that binary with `cargo install rhai-lsp`. There is no crate published
  under that name.
* And the client would not start in any case: `activate()` reads the setting out of the
  configuration section `notedown` rather than `rhai`, so `useLanguageServer` is always
  `undefined`, which is falsy, and the branch that starts the client is never taken.

So `doc/generator-spec.md` §8's sentence about that extension bundling a language server is
wrong, and has been corrected to point here.

Snippets need no server. VS Code reads `*.code-snippets` out of the workspace itself; all it
needs from an extension is the **language id** — `rhai`, `kdl` — which the two syntax extensions
in §5 register and which is what you wanted them for anyway. A snippet then carries a name, a
body with tab stops, and a description that shows in the completion item's documentation pane.
That is completion and inline documentation for the whole vocabulary, with one moving part fewer
than the correct answer has.

**Both files come from the same place**, which is the property worth having:
`grind definitions` and `grind definitions --snippets` are each the *engine's* answer, so
neither can describe a function that is not there. And a function cannot be registered without
its documentation — `build/src/hint.rs` takes the doc comment as an argument and there is no
shorter way to register one — so an undocumented function cannot be added by forgetting.
`cli/tests/editor.rs` fails the build when either shipped file goes stale.

Keep `grind.d.rhai` anyway. It costs one command, an editor that reads it may appear, and it is
the most readable single-file reference to the vocabulary there is — `examples/grind.d.rhai` is
a copy kept in this repository for exactly that reading.

---

## 5. What each extension actually does

Both were read as installed, rather than taken from a marketplace description.

### `kdl-org.kdl` — for `.grind`

Version 2.1.3. A KDL grammar, `language-configuration.json` for brackets and comments, and a
**bundled language server** that reports syntax diagnostics (`kdl.enable`, `kdl.command`,
`kdl.argv`, `kdl.loglevel`).

It supports **KDL v2 only**, which is the version the projection is written in — `#true`,
`#false` and `#null` are v2 spellings, and `kdl-rs` 6.x is the parser underneath `grind`. If
something highlights a `.grind` as though every `#` began a comment, an extension for KDL v1
has claimed the file.

What its diagnostics cover is *KDL*: an unclosed brace, a bad escape, a malformed node. They
know nothing about `sheet`, `cell` or a range, so a `format … percent` that should be
`percentage` looks fine to the editor and is caught by `grind lint`.

### `rhaiscript.vscode-rhai` — for `.rhai`

Version 0.6.9. A Rhai grammar, and the language server situation in §4. Highlighting,
bracket matching and comment toggling all work; nothing else in the extension does.

There is a second extension, `mikai233.rhai-analyzer`, which claims completion, hover and
diagnostics from an analyzer of its own. It was **not** tested here and is not recommended in
`.vscode/extensions.json`; if you try it, note that its formatter configuration is a
`rhai.toml`, which is a different file from the `.d.rhai` definitions above.

---

## 6. One more keystroke: lint as a task

The check an editor cannot do, bound to a key. `.vscode/tasks.json`:

```json
{
    "version": "2.0.0",
    "tasks": [
        {
            "label": "grind lint",
            "type": "shell",
            "command": "grind lint ${file}",
            "problemMatcher": {
                "owner": "grind",
                "fileLocation": ["autoDetected", "${workspaceFolder}"],
                "pattern": {
                    "regexp": "^(.*):(\\d+):(\\d+): (.+?): (error|warning): (.*)$",
                    "file": 1,
                    "line": 2,
                    "column": 3,
                    "code": 4,
                    "severity": 5,
                    "message": 6
                }
            },
            "group": "test"
        },
        {
            "label": "grind build",
            "type": "shell",
            "command": "grind build ${file} -o ${fileDirname}/${fileBasenameNoExtension}.fods",
            "group": "build"
        }
    ]
}
```

For a `.grind`, every finding whose address the file spells comes out in the compiler's shape
— `quote.grind:58:34: Quote.B11: warning: … [misspelt]` — with the line and column read from
the file as written, comments and alignment included, through the projection's span map. The
matcher above takes the address as the problem's *code*, so it is still there to type back at
`grind`. Two kinds of finding have no line and the matcher skips them: one about a document in
another form (a `.fods` has no line a person wrote), and one whose address the file never
spelled on its own, such as a cell covered only by a `style` range. Both still print, by
address. `--format json` carries the same thing as a `location` object, with an end as well as
a start, for anything that wants to underline a range.

---

## 7. Other editors

Nothing above is VS Code's except the file names.

| | `.grind` | `.rhai` |
|---|---|---|
| **Zed, Helix, Neovim** | a KDL tree-sitter grammar exists and covers the file once the extension is mapped | a Rhai tree-sitter grammar exists; `grind definitions` output feeds anything that speaks the Rhai language server protocol |
| **Anything with LSP** | — | point it at a `rhai-lsp` if one becomes available, with `grind.d.rhai` beside your scripts |
| **Anything at all** | `grind lint`, `grind sheet project`, `grind info` | `grind build` |

The one setting worth carrying across is the association: `.grind` is KDL, and no editor guesses
that from the extension.

---

## 8. Troubleshooting

| | |
|---|---|
| **A `.grind` is plain text, and no task runs** | the folder is in **Restricted Mode** — VS Code's state for a folder it has not been told to trust — and there it ignores the workspace's `files.associations`, so the snippets, scoped to `kdl`, never apply; it also refuses to run tasks. Trust the folder (*Restricted Mode* in the status bar, or *Workspaces: Manage Workspace Trust*), or put the association in your **user** settings, which apply either way. Measured: untrusted, `book.grind` opened as `plaintext`; trusted, as `kdl` with its `cell` snippets. A `.rhai` is unaffected, because the Rhai extension claims the file type itself |
| **The description is missing from a completion** | it is the item's detail line, shown only when the details are open: press Ctrl+Space again while the list is up |
| **Nothing is offered after `s.`** | expected: a snippet needs a letter (`s.pu`), and nothing knows what `s` is until there is a language server (`doc/init.md` I5) |
| **No highlighting in a `.grind`** | the `files.associations` entry is missing, or the window needs reloading after adding it. The language shown in the status bar should read *KDL* |
| **No completions** | snippets are suppressed when `editor.snippetSuggestions` is `"none"`; the settings above set it to `"top"` for these two languages. Check that the `.code-snippets` file is in the *workspace folder's* `.vscode/`, not a parent's |
| **Completions for functions that do not exist** | the snippet file is from an older `grind` — regenerate it (§3) |
| **`#true` highlighted as a comment** | a KDL **v1** extension has claimed the file; the projection is v2 (§5) |
| **The Rhai extension offers to install a language server** | decline. §4 is why |
| **A script fails and the editor said nothing** | it would: nothing published type-checks Rhai. `grind build` is the check, and its errors carry a line and a column |
