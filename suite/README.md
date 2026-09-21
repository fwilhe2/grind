<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# grind — the suite

This package installs nothing itself. It depends on the four applications:

| Package | Installs |
|---|---|
| `grind-cli` | `grind`, the command line — every capability of both applications, scriptable |
| `grind-tui` | `grind-tui`, the terminal shell — spreadsheets and text documents in one binary |
| `grind-sheet-gtk` | `grind-sheet-gtk`, the spreadsheet window, and its desktop entry |
| `grind-text-gtk` | `grind-text-gtk`, the word processor window, and its desktop entry |

Removing this package leaves the four in place; remove them by name.
