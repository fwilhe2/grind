# The GitHub Action

<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
SPDX-License-Identifier: CC-BY-4.0
-->

`action.yml` at the root of this repository puts the released `grind` and `grind-tui` on the
`PATH` of a GitHub Actions job:

```yaml
- uses: fwilhe2/grind@main
- run: grind --version
```

It downloads the release's static command-line archive for the runner (Linux, macOS or Windows,
x64 or Arm), checks it against the release's `.sha256` file, and adds it to the `PATH`. It
compiles nothing, and it needs no token or permissions.

## Inputs and outputs

| Input | Default | |
|---|---|---|
| `version` | `latest` | A release tag, `v0.2.0` or `0.2.0`, or `latest` |
| `cache` | `true` | Keep the binaries in the Actions cache, so later runs skip the download |

| Output | |
|---|---|
| `version` | The tag installed, such as `v0.2.0`; with `latest`, the one that was latest |
| `path` | The directory holding `grind` and `grind-tui` |
| `cache-hit` | Whether the binaries came from the Actions cache |

Calling the action a second time in the same job, from a second step or a composite action of
your own, finds the binaries already unpacked and downloads nothing. With `cache: true`, later
runs and other jobs restore them from the Actions cache, keyed by release and platform, so a
pinned version is downloaded once per platform until the cache expires. `latest` is resolved
from the release page's redirect on every run, so a new release is picked up without changing
the workflow.

## Pinning

There are two versions to pin: the action's own code (`@…`) and the grind release it installs
(`version:`). For a build that does not change under you, pin both: the action to a commit SHA,
which is the safest way to use any third-party action, and `version:` to a release:

```yaml
- uses: fwilhe2/grind@<commit sha> # main on 2026-11-27
  with:
    version: v0.2.0
```

Dependabot's `github-actions` ecosystem keeps a SHA-pinned `uses:` up to date. `version:` is
plain text to it, so a new grind release reaches the workflow only when you change it.

## Examples

### PDFs of every text document, on every push

```yaml
name: pdf
on: [push]
permissions:
  contents: read
jobs:
  pdf:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
      - uses: fwilhe2/grind@main
      - run: |
          mkdir -p pdf
          for f in docs/*.odt docs/*.fodt; do
            [ -e "$f" ] || continue
            name="$(basename "${f%.*}")"
            grind text export-pdf "$f" "pdf/$name.pdf" --paper a4
          done
      - uses: actions/upload-artifact@v7
        with:
          name: pdf
          path: pdf/
```

The same document always gives the same PDF, byte for byte, so the PDFs can be committed or
compared without spurious changes.

### Spreadsheets built from scripts, with tests

A spreadsheet written as a Rhai script (`grind init` starts one) is source code: its tests run
on every pull request, and the built document is the artifact.

```yaml
- uses: fwilhe2/grind@main
- run: grind test model/budget.rhai
- run: grind build model/budget.rhai -o budget.ods
- uses: actions/upload-artifact@v7
  with:
    name: budget
    path: budget.ods
```

`grind test` exits with status 4 when an assertion fails, which fails the step.

### Check the documents in a pull request

`grind lint` reads either kind of document and exits with status 4 on an error. A spreadsheet
whose saved values disagree with its formulas is reported as a warning.

```yaml
on: [pull_request]
jobs:
  lint:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
      - uses: fwilhe2/grind@main
      - run: |
          status=0
          for f in $(git ls-files '*.ods' '*.fods' '*.odt' '*.fodt'); do
            grind lint "$f" || status=1
          done
          exit $status
```

### Show what changed in a document

`grind diff` compares what two versions of a document say, cell by cell or block by block,
rather than their XML. In a pull request, against the base branch:

```yaml
- uses: actions/checkout@v7
  with:
    fetch-depth: 0
- uses: fwilhe2/grind@main
- env:
    BASE: ${{ github.event.pull_request.base.sha }}
  run: |
    for f in $(git diff --name-only "$BASE" -- '*.ods' '*.fods' '*.odt' '*.fodt'); do
      git cat-file -e "$BASE:$f" 2>/dev/null || continue
      git show "$BASE:$f" > "/tmp/before.${f##*.}"
      echo "## $f" >> "$GITHUB_STEP_SUMMARY"
      { echo '```'; grind diff "/tmp/before.${f##*.}" "$f"; echo '```'; } >> "$GITHUB_STEP_SUMMARY"
    done
```

The base SHA is passed through `env:`, never written into the script, which is how a workflow
keeps event data from becoming shell code.

### Turn Office files into ODF

```yaml
- uses: fwilhe2/grind@main
- run: |
    grind sheet import inbox/figures.xlsx figures.ods
    grind sheet recalc figures.ods
    grind text import inbox/letter.docx letter.odt
```

Nothing is fetched and no macro runs.

### Markdown to a printed document

```yaml
- uses: fwilhe2/grind@main
- run: |
    grind text new handbook.fodt
    grind text import-md handbook.fodt HANDBOOK.md
    grind text export-pdf handbook.fodt handbook.pdf
```

### A spreadsheet's numbers for another tool

```yaml
- uses: fwilhe2/grind@main
- run: |
    grind sheet recalc forecast.ods
    grind sheet export-csv forecast.ods --out forecast.csv
    grind --format json info forecast.ods > forecast.json
```

`export-csv` writes what each cell shows, so dates and percentages leave as they are displayed;
`--formulas` writes the formulas instead.

### Every platform

```yaml
jobs:
  check:
    strategy:
      matrix:
        os: [ubuntu-latest, ubuntu-24.04-arm, macos-latest, windows-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v7
      - uses: fwilhe2/grind@main
      - run: grind test model/budget.rhai
```

On Windows the binaries are `grind.exe` and `grind-tui.exe`; a `run:` step finds them as
`grind` either way.

## How it is tested

`.github/workflows/setup-action.yml` uses the action from this checkout on all six runner kinds
the release has a binary for: the latest release, a second call in the same job, a pinned
version, and a version that is not a release, which must fail.
