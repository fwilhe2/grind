<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# grind for VS Code

The client for `grind lsp`, the language server of [grind](https://github.com/fwilhe2/grind) —
an ODF-native office suite whose spreadsheets can be written by hand as a **projection**
(`.grind`) or built by a **script** (`.rhai`).

## What it does

| | `.grind` | `.rhai` |
|---|---|---|
| Errors and warnings | a read error where it says; every `grind lint` finding on its cell's line, spelling included | syntax errors as you type — the script is compiled, **never run** |
| Completion | the projection's nodes; inside a formula, the functions and the document's defined names | after `s.`, a sheet's methods; elsewhere, the free functions — each with its documentation |
| Hover | a cell's formula and what it shows; a name's definition | a function's signature and documentation |
| Inline values | what every formula computes, shown after it — never written into the file | — |
| Go to definition | a defined name's or a sheet's declaration | — |

Every answer comes from the `grind` binary, so it is the same one `grind lint`, `grind build`
and the other editors give.

## Setup

1. **Install grind**, so `grind` is on your `PATH` — or set `grind.path` to where it is.
2. Install this extension (`code --install-extension grind-<version>.vsix`, from
   `editors/vscode/package.sh`).
3. Optionally the *Rhai* extension (`rhaiscript.vscode-rhai`) for highlighting in scripts; a
   `.grind` is highlighted by this extension.

`grind.path` is honoured from your user settings everywhere, but from a folder's own settings
only once you trust the folder: in Restricted Mode a folder cannot choose the program VS Code
runs. *grind: Restart the language server* picks up a new binary.

## Licence

AGPL-3.0-or-later, like grind. `vscode-languageclient` and what it depends on are MIT, ISC and
BlueOak-1.0.0.
