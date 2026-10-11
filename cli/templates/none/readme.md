# A grind project

Set up by `grind init --template none`: the editor and repository files, and no starter. This
file is yours — rewrite it to say what your project is, and keep or drop the notes below.

## Working on it

| | VS Code | Command line |
|---|---|---|
| Build a script into the spreadsheet beside it | **Ctrl+Shift+B** on the `.rhai` | `grind build model.rhai -o model.fods` |
| Run a script's `test_…` functions | *Tasks: Run Test Task* | `grind test model.rhai` |
| Check a document for problems | the *grind: lint* task | `grind lint book.grind` |
| Write a `.grind`'s answers into it | the *grind: recalculate* task | `grind sheet recalc book.grind` |

