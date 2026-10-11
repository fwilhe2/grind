# A grind project

Set up by `grind init --template sheet`. `book.grind` is a spreadsheet written by hand: every
cell a line, so a change is a line of diff. This file is yours — rewrite it to say what your
project is, and keep or drop the notes below.

## Working on it

| | VS Code | Command line |
|---|---|---|
| Check it for problems | the *grind: lint* task | `grind lint book.grind` |
| Write every formula's answer into it | the *grind: recalculate* task | `grind sheet recalc book.grind` |
| See it as a grid | | `grind sheet view book.grind` |
| Hand it to somebody | | `grind convert book.grind book.ods` |

