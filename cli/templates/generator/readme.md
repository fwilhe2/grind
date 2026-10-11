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

