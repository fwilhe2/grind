# A grind project

Set up by `grind init`. `model.rhai` is a script that builds a spreadsheet out of `data.json`
beside it; change the numbers there, rebuild, and every formula and total follows. This file is
yours — rewrite it to say what your project is, and keep or drop the notes below.

## Working on it

| | VS Code | Command line |
|---|---|---|
| Build `model.fods` from the script | **Ctrl+Shift+B** on `model.rhai` | `grind build model.rhai -o model.fods` |
| Run the `test_…` functions at its end | *Tasks: Run Test Task* | `grind test model.rhai` |
| Check a document for problems | the *grind: lint* task | `grind lint model.fods` |
| Write a `.grind`'s answers into it | the *grind: recalculate* task | `grind sheet recalc book.grind` |
| Open what was built | the *grind: open the built spreadsheet* task | `grind-sheet-gtk model.fods` |

Errors from the build, a failing test and lint findings in a `.grind` land on their line in the
*Problems* panel. The language is described in grind's `doc/generator-guide.md`, and a
hand-written spreadsheet (`.grind`) in `doc/projection-guide.md`:
<https://github.com/fwilhe2/grind/tree/main/doc>.

## First time in VS Code

1. **Install the two extensions it offers** — *KDL* (`kdl-org.kdl`) and *Rhai*
   (`rhaiscript.vscode-rhai`), recommended in `.vscode/extensions.json`.
2. **Trust this folder.** VS Code opens a folder it has not been told to trust in *Restricted
   Mode*, and there it ignores `.vscode/settings.json`'s line saying a `.grind` is KDL — so a
   `.grind` stays plain text with no highlighting and no completion — and it runs no tasks, so
   Ctrl+Shift+B does nothing. Click *Restricted Mode* in the status bar, or run *Workspaces:
   Manage Workspace Trust*. If you would rather not trust it, put
   `"files.associations": { "*.grind": "kdl" }` in your *user* settings instead.
3. **If `grind` is not on your `PATH`**, set `"grind.path"` in `.vscode/settings.json` to where it
   is; every task runs it from there.

## What completion does, and does not, do

Completion comes from **snippets**: `.vscode/grind.code-snippets` for scripts and
`.vscode/grind-projection.code-snippets` for `.grind` files, both written by `grind init`. That
works without a language server, and it has three limits worth knowing before they surprise you:

- **It waits for a letter.** Nothing is offered right after `s.`; type `s.pu` and `push` appears.
  Snippets do not know that `s` is a sheet, so after the dot you are offered every function
  that starts with what you typed, not only a sheet's.
- **The description is one press away.** Each entry's explanation is in its detail line: press
  **Ctrl+Space again** while the list is open, or click the `>` on the highlighted entry, to show
  it. It reads as plain text.
- **There is no hover.** Pointing at `push` in your code shows nothing, and nothing underlines a
  mistake while you type — `grind build` and `grind test` are the check, and their errors carry a
  line and a column.

All three are what a **language server** fixes, and grind has one: `grind lsp` offers a sheet's
methods right after `s.`, shows each one's documentation properly, hovers, underlines a syntax
error as you type, and in a `.grind` shows what every formula computes beside it. VS Code starts
it through grind's own extension, which is not in the marketplace yet: build it from grind's
`editors/vscode/` (`package.sh`) and `code --install-extension` the `.vsix` it writes — it also
makes a `.grind` work in an untrusted folder. Helix, Neovim and other editors that start a
language server themselves need only a few lines of configuration; grind's
`doc/editor-setup.md` §7.1 has both.

`grind.d.rhai` is the same vocabulary as a Rhai definition file: every function with its
parameters, types and documentation, in one file you can read.

## Keeping it current

- **After upgrading grind, rewrite the generated files** — the two snippet files and
  `grind.d.rhai` — so completion describes the functions this version has. Each one's first
  lines name the command that writes it.
- **`git diff` reads an `.ods` as the cells that changed** because of `.gitattributes` and one
  setting that git keeps per clone rather than in the repository. In a fresh clone, run once:

  ```console
  git config diff.grind.textconv "grind project"
  ```

- `model.fods` is in `.gitignore` because it is built from the script. A spreadsheet you write
  or edit by hand is source: keep it as `.fods` or `.grind`, which diff, and commit it.
