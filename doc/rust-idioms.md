<!--
SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>

SPDX-License-Identifier: CC-BY-4.0
-->

# Rust in this repository

Which spelling the code uses where the language offers several, and why. A habit listed here was
found in the tree and replaced everywhere, so new code that brings it back is the odd one out.
The toolchain is the current stable (§5).

## 1. A byte offset is not a character

Two panics came out of one assumption, that an offset into a `&str` lands between two
characters. It does when the offset came from `find` on the same string, and nowhere else:

- **from a file**: the Excel import sliced an eight-byte `rgb` attribute at byte 2, and
  `rgb="aé12345"` has a character across that byte;
- **from a shell**: a caret converted from UTF-16 code units can land inside `é`, and the
  signature hint sliced the formula there.

So `text[a..b]` is for offsets the same function has just searched for. Everything else has a
spelling that cannot panic:

| The offset is | Write |
|---|---|
| a prefix or suffix to take off | `strip_prefix`, `strip_suffix`, `split_once` — never `starts_with` and a counted `[n..]` |
| the file's, or another crate's | `text.get(a..b)?` |
| a caret | `&text[..text.floor_char_boundary(caret)]` |
| a count of characters | `char_indices().nth(n)`, as `text/src/paint.rs` does |

`cargo clippy -- -W clippy::string_slice` lists every slice. CI does not hold it, since most of
what it finds is the first kind, but it is the list to read when a reader or an editor gains a
new input.

## 2. Newer spellings, by what they replace

Each row is used throughout the tree.

| Instead of | Write | Since |
|---|---|---|
| `_ if cmd.starts_with("open ") => f(&cmd[5..])` | `_ if let Some(rest) = cmd.strip_prefix("open ") => f(rest)` | 1.95 |
| `s.strip_prefix(p).unwrap_or(s)` | `s.trim_prefix(p)`, and `trim_suffix` | 1.100 |
| `s.strip_prefix(a).and_then(\|s\| s.strip_suffix(b))` | `s.strip_circumfix(a, b)` | 1.98 |
| `v.windows(2)` read as `w[0]`, `w[1]` | `v.array_windows()` with `\|[prev, next]\|` | 1.94 |
| `assert!(matches!(value, Pattern))` | `assert_matches!(value, Pattern)` | 1.96 |
| `String::from_utf8_lossy(&bytes).into_owned()`, `bytes` owned | `String::from_utf8_lossy_owned(bytes)` | 1.99 |
| `&mut x as *mut T` handed to a C function | `&raw mut x`, and `&raw const x` | 1.82 |
| `let x = match y { Ok(x) => x, Err(_) => return … };` | `let Ok(x) = y else { return … };` | 1.65 |
| `row as u64`, widening | `u64::from(row)` | — |
| `match flag { true => a, false => b }` | `if flag { a } else { b }` | — |
| `assert!(hits.is_empty())` | `assert!(hits.is_empty(), "{hits:?}")` | — |

What each buys:

- **An `if let` guard binds what the guard tested.** The terminal's two `:` command tables were
  sixty arms of `starts_with` and a hand-counted offset; now an arm cannot disagree with its
  literal. The guard's pattern can do the filtering too:
  `_ if let Some((which @ ("width" | "height"), length)) = word.split_once('=')`.
- **`array_windows` yields `&[T; N]`**, so a pair is taken apart by a pattern rather than
  indexed, and `N` comes from the pattern. Bind by value, `|&[a, b]|`, when `T` is `Copy` and
  the body compares against a value.
- **`assert_matches!` prints the value** that did not match, where `assert!(matches!(…))` prints
  only its own source. It is not in the prelude: `use std::assert_matches;` in the test module.
  The value needs `Debug`, one more reason for a type to derive it.
- **`&raw mut`** makes the pointer without making a reference first, so it stays correct for a
  struct the callee is about to initialise.
- **`From` for a widening conversion** stops compiling the day the source type grows; `as`
  truncates instead. A narrowing `as` stays, with the reason it cannot truncate beside it.

## 3. Considered, and not adopted

Written down so the next reader of a release note does not have to decide again.

- **`f64::mul_add` and the `algebraic_*` methods**: each changes the last bit of a result. A drawing refactor here is proved by a byte-identical `--render-to` frame, and loop E
  compares evaluated numbers with LibreOffice's, so none of these goes in as a tidy-up.
- **`core::range::Range`**, which is `Copy` (1.96). A span type made today should hold one; the
  existing `std::ops::Range` fields and their `.clone()`s are not worth a sweep until `a..b`
  itself makes the new type.
- **`mem::DropGuard`** (1.100) is for a rollback one function needs on an early return. A
  resource with a name keeps its own type and `Drop` (`ui_win32/src/gdi.rs`), and `App::mutate`
  needs neither: its closure leaves the state untouched on `Err`.
- **`cfg_select!`** (1.95) is for three or more alternatives of one item. A platform half here
  is a whole module behind one `#[cfg]`, which reads better as it is.
- **The never type** (1.100): nothing here returns `Result<T, Infallible>`.
- **Clippy's own fix for `assert_is_empty`**, `assert_eq!(hits, [] as [Hit; 0])`. The assertion
  prints the value instead (§2's last row), which is what the lint is after.
- **`Vec::push_mut`** (1.95) replaces `push` followed by `last_mut().unwrap()`. The tree's
  `last_mut().expect("never empty")` sites append to a row started earlier, which it does not
  fit.

## 4. Locks

`App` hands out no guard. Inside it, two habits keep the core pushing with no lock held:

- **Clone the `Arc` out in its own statement, then use it**:
  `let lexicon = self.lexicon.read().unwrap().clone();`. A guard made in a `match` scrutinee
  lives to the end of the `match`. In `if let` it ends before the `else`, since the 2024
  edition; in `match` it does not.
- **Never call out under a lock.** `notify` reads the observer, lets the guard go, then calls.

## 5. What CI holds, and a new toolchain

`cargo clippy` runs with warnings as errors for the Linux, Windows and macOS targets, and the
root `Cargo.toml` turns on clippy's whole **pedantic** group for every crate
(`[workspace.lints]`, which a crate takes with `[lints] workspace = true`). New code is held to
all of it.

Under the group is a list of lints allowed by name: the ones the existing tree is not yet clean
of. It only shrinks. To retire one, delete its line, run `cargo clippy --fix` for the three
targets, fix what is left by hand, and commit that lint alone. Three of them are a decision
rather than a chore, and are still open:

- the four `cast_*` lints, about eleven hundred sites, most of them pixel arithmetic where
  `as`'s saturation is what is meant;
- `missing_errors_doc` and `missing_panics_doc`, which ask for a section on some four hundred
  functions, against the rule that a comment says only what the code cannot;
- `too_many_lines`, over a hundred functions, `cli/src/sheet.rs`'s `run_sheet` longest.

`clippy.toml` lists the product names `doc_markdown` leaves as prose. An `#[allow]` anywhere
else carries its `reason`, as `cli/src/args.rs` does: its doc comments are `--help` text.

CI installs stable unpinned, so a release can turn the tree red without a commit: 1.100 did,
through a lint that had grown stricter. When one arrives:

```sh
rustup update stable
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p grind-win32 --target x86_64-pc-windows-msvc --all-targets -- -D warnings
cargo clippy -p grind-mac --target aarch64-apple-darwin --all-targets -- -D warnings
```

Then read the release notes for what replaces a habit of ours. A feature earns a row in §2 when
it removes a way to be wrong (an offset, an index, an `unwrap`), and it goes in everywhere at
once or not at all: two spellings of one thing are worse than the older one.

Cargo warns about a dependency nothing uses (1.100). A crate only a platform's code uses goes in
that platform's table, `[target.'cfg(windows)'.dependencies]`, or Linux builds it for nothing.
