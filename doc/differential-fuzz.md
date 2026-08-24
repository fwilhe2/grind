<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Loop E — the generated differential

The plan for checking our function implementations against LibreOffice on inputs **nobody
wrote a fixture for**, and the document that holds it to the rules. Normative for this work
the way `doc/sheet-shell.md` is for phase 9.

## What it adds that loop B does not

Loop B recalculates LibreOffice's own `functions/` corpus and compares against the cached
values in it — 52213 formula cells, categorised by function, already written by people who
knew what to test. It is the best conformance corpus available and it has one shape of blind
spot: **it only contains the arguments somebody thought to write down.** `LEFT("abc"; -1)`,
`SUM` over a range holding an error, `MID` with a fractional start, a date function at the
1900 boundary, text-that-looks-like-a-number where a number is wanted — §6.3's coercion
rules are exactly where our implementation and LO's can differ silently, and exactly where a
hand-written fixture is least likely to go.

Loop E generates those arguments instead of waiting for them: formulas built from the
catalog's own signatures, filled from a pool of deliberately awkward values, evaluated by us
and by LibreOffice, compared cell for cell.

## Two measured facts the design rests on

Both established by probing `soffice` directly, not by reading anything:

1. **A formula cell shipped with no cached value comes back computed.** LibreOffice does not
   recalculate on load in general — that is a setting, and loop C depends on it *not*
   happening — but a `table:formula` cell carrying no `office:value` has nothing to load, so
   the conversion produces LO's own answer. That is the oracle: write formulas with no
   values, convert, read the values back.
2. **An error's identity survives only in its display text.** `=1/0` returns as
   `office:value-type="string" office:string-value="" calcext:value-type="error"` with
   `<text:p>#DIV/0!</text:p>` — the value attribute is empty and the name is in the
   paragraph, which the reader already keeps (`sheet/src/odf/read.rs`, the `cached_error`
   branch). Some errors come back as LO's internal code instead (`Err:502`), which §5.12 does
   not name, so any error agrees with those — loop B's rule, unchanged. An `=ERROR.TYPE(f)`
   companion cell was planned to recover the kind and is **not** needed: it doubles the
   document for a name the paragraph already carries.

   One thing this rests on, learned the hard way: the `of:` namespace must be declared in the
   emitted file. Without it LibreOffice reads `of:=1/0` as an unprefixed formula and writes
   back `of:=of:=1/0`, every cell `Err:510`.

## Why not a fuzzer

`cargo-fuzz`/AFL are coverage-guided: they need a feedback signal per input, cheaply. The
oracle here is a 400 MB office suite whose startup dominates everything, and there is no
coverage signal from it at all — so what pays is not a mutation loop but a **seeded grammar
generator** plus batching: thousands of formulas in one document, one `soffice` invocation.
Deterministic from `GRIND_FUZZ_SEED`, so a disagreement replays exactly.

Coverage-guided fuzzing *is* the right tool for a different target, and it needs no oracle at
all: the lexer/parser/serializer, where the property is "never panics, and
`parse → serialize → parse` is stable". That is part II below, not built yet.

## Clean room

Unchanged, and this is oracle use, which `CONTRIBUTING.md` already permits: LibreOffice is
run as a program and its *output* is compared. No LO source is read for this, and nothing
learned from a disagreement reaches code without going through `doc/ods-format.md` first,
cited. A mismatch is not automatically our bug — LO has its own — so each one is triaged into
either a fix or a documented divergence.

## The design

**One document per run.** Layout, on a single sheet:

- Rows 1–8, columns A–F: the **data block** — the values the generated formulas point at.
  Numbers, negatives, zero, a huge magnitude, text, text that looks like a number, a logical,
  a date serial, an error, and *empty cells*, which are their own semantics in §6.3 and the
  single most common source of divergence.
- Row 12 down: one generated formula per row, in column A.

Both sides then read **the same file**: we read it and evaluate with `Engine`, LibreOffice
converts it and we read the values out of its output. Nothing is written by our writer, so a
writer bug cannot show up here as an evaluator bug — the harness emits the flat XML itself,
which is a template of about forty lines.

**The generator** walks `funcs::catalog()` and reads each entry's `signature` — the spec's own
`Syntax:` line, already in the binary — for the argument count and each argument's type word
(`Number`, `Text`, `Logical`, `Reference`, `NumberSequence`, `Any`, …). Optional arguments are
the `[ ; … ]` groups; a repeated one is `}+`. Each argument is then drawn from the pool for
its type, which is where the awkwardness lives: `-1`, `0`, `""`, `"12"`, `1e308`, a reference
into the data block, a range spanning empty and text cells.

**Determinism** is a SplitMix64 in the harness — twenty lines, no new dependency for what a
`u64` and two shifts do.

## What is excluded, and why

Named classes, the way loop B names its exclusions — not a tolerance:

- **Volatile functions** — `RAND`, `RANDBETWEEN`, `NOW`, `TODAY`. Not a function of the
  document, so no two evaluators can agree by construction. Loop B excludes the same two it
  meets.
- **Locale-dependent output** — `TEXT` with a format code renders through LO's *session*
  locale, which is the developer's desktop rather than a property of the document: on this
  machine `=TEXT(1/3;"0.00000000000000000E+00")` came back as
  `333.333.333.333.333.000E-18`. The run pins a private `UserInstallation` profile (loop C
  already does, for the profile lock) and the harness excludes format-code arguments until a
  locale can be pinned as deliberately as the profile is.

## Pinning LibreOffice

`FLOOR` is a fact about one specific `soffice` binary, not about the code: a LibreOffice
point release can move a borderline formula (a stats function dividing by a hole-sized
denominator, say) by exactly one, and the test has no way to tell that apart from a
regression. So the oracle is pinned rather than left to whatever a machine has installed.

**The pin.** `ci/libreoffice-image` holds one line: a container image referenced by digest,
currently `ghcr.io/fwilhe2/libreoffice@sha256:adb88646…` (LibreOffice 26.2.5.2 on Debian 13,
Calc **and** Writer).
A digest is the strongest pin available — it names bytes, not a version string whose
contents a distribution can rebuild underneath it — and, unlike the Ubuntu package version
this used to be, it means the same binary answers on a laptop, in CI, and on any OS with
Docker.

**How the pin reaches the tests.** Not through the Rust: `scripts/soffice-docker/soffice` is
a shim that runs the pinned image, and anything with that directory first on `PATH` gets the
pinned `soffice` from the plain `Command::new("soffice")` the tests already use. The shim passes
`--security-opt label=disable`, because on an SELinux-enforcing host (Fedora, RHEL) a container
may read the bind mount below but not write to it, so `soffice` cannot create its own
`UserInstallation` profile and every loop C and loop E test fails for a reason unconnected to
the code. The
container sees the host's temp directory at the same path, which is where every input is
staged and every output collected, and nothing else — a converter that could reach the
repository would be a worse oracle, not a better one. `.github/workflows/ci.yml`'s
`roundtrip`, `loop_e` and `corpus` jobs pull the image and prepend that directory; the build
itself stays on the runner, so the usual cargo caching still applies.

**Running it locally, at the same pin.** `scripts/soffice-tests.sh` does the pull and the
`PATH` prepend and then runs the soffice-backed tests, so a disagreement between "works for
me" and a red CI run is reproducible rather than argued about:

```sh
scripts/soffice-tests.sh                               # loop C (out) + loop E
scripts/soffice-tests.sh --test loop_e -- --nocapture  # one test, extra args passed through
```

Without Docker, loop E still runs against whatever `soffice` is on `PATH` — useful for
iterating on the generator or the parser, not for reading `FLOOR` as a verdict, since a
local LibreOffice can legitimately disagree with the pin by one or two formulas.

**The pin serves both applications, and did not always.** The first image was Calc-only: its
`share/registry/` held `calc.xcd` and no `writer.xcd`, so it imported a `.fodt` *as a
spreadsheet* and had no `fodt` export filter, and `grind-text`'s loop C could not use it
(`doc/odt-format.md` §5b). Rather than drop the pin or hard-code a skip, `oracle_ready` in
`text/tests/roundtrip.rs` probed the capability by converting a one-paragraph document and
skipped with a notice when nothing came out. The image was then rebuilt with Writer in it —
`sha256:adb88646…`, same LibreOffice 26.2.5.2 on the same Debian 13 — and **loop C for text
started gating CI with no file changed**, which is what detecting a capability buys over
hard-coding a skip. The probe stays for the developer whose own `soffice` has no Writer.

That bump was run through the four steps below, and it is the worked example of why step 3
exists: nothing about Calc was meant to change, and loop E was re-read anyway. It came back at
**913, unchanged to the formula**, so `FLOOR` did not move — which is the evidence that the
rebuild added Writer and disturbed nothing else, and is not something anyone could have
asserted without running it.

**Upgrading the pin, safely.** An image bump is a deliberate, reviewable change, not
something that happens by CI drifting under it:

1. `docker pull ghcr.io/fwilhe2/libreoffice:latest`, take the digest it prints, and write
   the full `name@sha256:…` into `ci/libreoffice-image`. A tag is not a pin; only the digest
   goes in the file.
2. Run `scripts/soffice-tests.sh` locally — it now uses the *new* pin — and note where
   loop C and loop E's scoreboard land.
3. If loop E's `matched` count changed, update `FLOOR` in `sheet/tests/loop_e.rs` and the
   figure in this document's "The first run" section together with the image bump, in the
   same commit. A `FLOOR` change with no image bump beside it is a regression, not routine
   maintenance — that is the whole point of pinning.
4. Push and let the `roundtrip`/`loop_e`/`corpus` CI jobs confirm the same numbers away from
   this machine before merging.

## The ceiling, named

LibreOffice writes doubles at **15 significant digits**, so this loop cannot compare tighter
than loop C's rule and never will — ULP-level divergence is invisible to it, permanently.
That is a property of the oracle's serialiser, not a loosening we chose, and it is the same
one already written down in `doc/ods-format.md` §3.4.

## Milestones

| # | Milestone | Contents | Exit criterion |
|---|---|---|---|
| F0 | **The harness** — *done* | flat-XML emitter, data block, `soffice` driver reused from loop C, both-sides read, `agrees` from loop B, scoreboard + `FLOOR` | one seeded run, green, with a per-function scoreboard |
| F1 | **The generator** — *done* | signature parsing (arity, optionality, type words), the awkward-value pool, references and ranges into the data block | every catalog function appears in a run |
| F2 | **Triage** | each disagreement either fixed or written into `doc/ods-format.md` as a divergence with LO's behaviour cited by probe | `wrong` reaches zero and `FLOOR` rises to the run's total |
| F3 | **Nesting** | arguments that are themselves generated calls, one level deep — where an error propagates through, which is §4.6 territory | the scoreboard holds at one level of nesting |

## The first run, and what F2 inherits

Seed `0x5EED`, 1000 formulas, on the pinned image: **913 match, 87 disagree**, which is
`FLOOR` in `sheet/tests/loop_e.rs`. (It was 911 against the Ubuntu 24.04 package this used to
pin — a borderline `AVERAGEIF`/`NPV`/`SUMIF` case moves by exactly one across a point
release, which is why the pin is a digest and why the number is only meaningful beside one.
Since the pin is now an image rather than a distribution package, a local run and a CI run
are the same binary and should print the same figure.) The disagreements are not yet
triaged, and they are not evenly spread — the recurring classes, for whoever picks up F2:

- **A logical cell used as text.** `TRIM([.C2])` on a `office:value-type="boolean"` cell is
  `"1"` in LO and `"TRUE"` here; `LOWER`, `REPT`, `SUBSTITUTE`, `MID` all repeat it. This is a
  *reader* question before it is an evaluator one — LO holds a boolean cell as the number 1
  and only the format says otherwise — so it belongs in `doc/ods-format.md` first.
- **A logical inside a numeric range.** `MAX([.C2:.C8])` is 1 in LO and 0 here, `COUNT` over
  the same range counts them; §6.3's rule for logicals *in a reference* is the section to read.
- **Empty and error cells in the statistical family.** `STDEV`, `VAR`, `AVERAGE` and `NPV`
  over ranges spanning a hole disagree on the denominator.
- **Genuine numeric drift.** `FV` at a rate of `1e-15` differs in the 15th digit — that is the
  ceiling above, not a bug.

Part II, not built: `cargo-fuzz` targets for `lex`, `parse` and `display`, whose property
needs no oracle — no panic on any input, and `parse → serialize → parse` stable. It wants
nightly and a dev-dependency, so it lands on its own rather than inside this.

## Running it

```sh
cargo test -p grind-sheet --test loop_e                       # needs soffice; skips with a notice without it
GRIND_LOOP_E_DUMP=1 cargo test -p grind-sheet --test loop_e -- --nocapture   # print every disagreement
GRIND_FUZZ_SEED=12345 GRIND_LOOP_E_FORMULAS=5000 cargo test -p grind-sheet --test loop_e
```
