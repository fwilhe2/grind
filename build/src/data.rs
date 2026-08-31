// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `json("prices.json")` — **data a script reads, and the four walls around it.**
//!
//! Separating data from code is the reason a generator is worth having: a script that says how
//! a document is *shaped* and a file that says what is *in it* are two different things, edited
//! by different people at different times. Without this, the data ends up as a `const` at the
//! top of the script, which works and is what `examples/budget.rhai` does — until somebody who
//! does not read Rhai has to change a price.
//!
//! **This is the one place `doc/dsl.md` §2's "no I/O" is not literally true, and the promise it
//! replaces is written down rather than quietly dropped:**
//!
//! > A script reads **data**, never code, and only from one directory a person named.
//!
//! Four walls, and each is here rather than in a caller:
//!
//! 1. **JSON, and nothing that can execute.** JSON has no references, no includes, no
//!    functions and no side effects — it is a value, parsed by a real parser (`serde_json`),
//!    and what comes back is Rhai data. The name of the function says which parser ran, so a
//!    `csv(…)` later is a peer rather than a surprise.
//! 2. **One directory, named by a person.** Paths are relative to it, `..` is refused before
//!    the filesystem is touched, an absolute path is refused, and the result is canonicalised
//!    and checked to be *inside* it, which is what stops a symlink. `grind build` roots it at
//!    the script's own directory unless `--data` says otherwise — the same shape as
//!    `doc/dsl.md` §9's rule that a script may not import from anywhere but the project.
//! 3. **Bounded**, like everything else the engine does: a file may be [`MAX_BYTES`] and a
//!    script may read [`MAX_FILES`] of them, so "read a file" cannot become "read a disk".
//! 4. **Read only, and cached.** There is no writing, and reading the same file twice returns
//!    the same value without touching the filesystem again — so a `json(…)` inside a loop is a
//!    mistake with no consequences rather than a performance trap.
//!
//! **Determinism survives, restated.** §2 promised that the same source produces the same
//! bytes; with data it is the same source *and the same data*. That is a build system's
//! contract rather than a weakening of it — inputs to outputs, with the inputs named in the
//! script and living beside it.
//!
//! **The crate still has no filesystem of its own.** [`Data`] is a trait and [`Directory`] is
//! one implementation of it; `build` takes whichever the caller supplies, and the default is
//! [`NoData`], where every read is an error that says so. That is architecture rule 5 —
//! everything with a path has a twin without one — and it is what keeps a host that has no
//! filesystem (a browser, one day) able to run a script that does not ask for a file.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

use rhai::{Dynamic, Engine, EvalAltResult, Map};

/// The largest data file a script may read, in bytes.
///
/// Eight megabytes of JSON is a document nobody would generate and everybody would regret
/// opening; a spreadsheet's worth of prices is kilobytes. The point of the bound is that
/// "reading data" cannot become "reading whatever is there" by degree.
pub const MAX_BYTES: usize = 8 * 1024 * 1024;

/// How many distinct files one script may read. Reading the same one again is free (it is
/// cached) and does not count twice.
pub const MAX_FILES: usize = 64;

/// Where a script's data comes from.
///
/// A trait rather than a path, because this crate has no filesystem: `grind-cli` hands over a
/// [`Directory`], a test hands over a map in memory, and a host with no filesystem hands over
/// [`NoData`] and gets a clear error instead of a panic.
pub trait Data {
    /// The text of one data file, by the name the script used.
    ///
    /// The name is the script's, unresolved and untrusted: **an implementation is responsible
    /// for refusing anything outside whatever it considers its own**. [`Directory`] is the
    /// worked example.
    fn read(&self, name: &str) -> Result<String, String>;

    /// What to say when there is nothing to read from. Overridden by [`NoData`] so that the
    /// error names the flag rather than the file.
    fn unavailable(&self) -> Option<String> {
        None
    }
}

/// No data at all: every read is an error that explains how to get one.
///
/// The default, and deliberately not "an empty directory": a script asking for data it cannot
/// have should say so loudly, at the line that asked.
pub struct NoData;

impl Data for NoData {
    fn read(&self, name: &str) -> Result<String, String> {
        Err(format!(
            "{name}: this build has no data directory. `grind build` reads data from the \
             script's own directory, or from the one `--data` names"
        ))
    }

    fn unavailable(&self) -> Option<String> {
        Some("no data directory".to_owned())
    }
}

/// One directory, and nothing outside it.
///
/// The root is canonicalised once, when the directory is made; every read is canonicalised and
/// checked against it, so a symlink pointing out is refused at the point it would escape rather
/// than trusted because its name looked innocent.
pub struct Directory {
    root: PathBuf,
    /// Every file actually read, in order, without repeats — what a build's inputs were.
    read: RefCell<Vec<PathBuf>>,
}

impl Directory {
    /// A directory to read data from. Fails if it is not one.
    pub fn new(root: &Path) -> Result<Directory, String> {
        let root = root
            .canonicalize()
            .map_err(|e| format!("{}: {e}", root.display()))?;
        if !root.is_dir() {
            return Err(format!("{}: not a directory", root.display()));
        }
        Ok(Directory {
            root,
            read: RefCell::new(Vec::new()),
        })
    }

    /// The files this script actually read, in the order it first read them.
    ///
    /// A build's inputs, which is what a person wants when the output changed and the script
    /// did not.
    pub fn inputs(&self) -> Vec<PathBuf> {
        self.read.borrow().clone()
    }

    /// `name` as a path inside the root, or an error saying which wall it hit.
    ///
    /// Two checks, and both are needed. The first is on the *name*, before the filesystem is
    /// touched, so `../../etc/hosts` is refused as what it plainly is. The second is on the
    /// canonical *result*, because a name with nothing suspicious in it can still be a symlink
    /// pointing anywhere, and only the filesystem knows.
    fn resolve(&self, name: &str) -> Result<PathBuf, String> {
        let path = Path::new(name);
        if name.is_empty() {
            return Err("a data file needs a name".to_owned());
        }
        if path.is_absolute() {
            return Err(format!(
                "{name}: a data file is named relative to the data directory, never absolutely"
            ));
        }
        for part in path.components() {
            match part {
                Component::Normal(_) | Component::CurDir => {}
                _ => {
                    return Err(format!(
                        "{name}: a data file is inside the data directory, and `..` leaves it"
                    ));
                }
            }
        }
        let full = self
            .root
            .join(path)
            .canonicalize()
            .map_err(|e| format!("{name}: {e}"))?;
        if !full.starts_with(&self.root) {
            return Err(format!(
                "{name}: resolves to {}, which is outside the data directory",
                full.display()
            ));
        }
        Ok(full)
    }
}

impl Data for Directory {
    fn read(&self, name: &str) -> Result<String, String> {
        let path = self.resolve(name)?;
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{name}: {e}"))?;
        let mut read = self.read.borrow_mut();
        if !read.contains(&path) {
            read.push(path);
        }
        Ok(text)
    }
}

/// The [`Data`] a script is running against, plus what it has already read.
///
/// The cache is not an optimisation first: it is what makes `json("prices.json")` inside a loop
/// mean one file rather than a thousand reads, and it is where [`MAX_FILES`] is counted.
pub(crate) struct Loaded {
    data: Rc<dyn Data>,
    cache: RefCell<HashMap<String, Dynamic>>,
}

impl Loaded {
    pub(crate) fn new(data: Rc<dyn Data>) -> Loaded {
        Loaded {
            data,
            cache: RefCell::new(HashMap::new()),
        }
    }

    fn json(&self, name: &str) -> Result<Dynamic, Box<EvalAltResult>> {
        if let Some(value) = self.cache.borrow().get(name) {
            return Ok(value.clone());
        }
        if self.cache.borrow().len() >= MAX_FILES {
            return Err(bad(format!(
                "{name}: a script may read {MAX_FILES} data files, which is more than a \
                 document needs and fewer than a directory holds"
            )));
        }
        let text = self.data.read(name).map_err(bad)?;
        if text.len() > MAX_BYTES {
            return Err(bad(format!(
                "{name}: {} bytes is larger than the {MAX_BYTES} a data file may be",
                text.len()
            )));
        }
        let parsed: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| bad(format!("{name}: {e}")))?;
        let value = self::dynamic(parsed);
        self.cache
            .borrow_mut()
            .insert(name.to_owned(), value.clone());
        Ok(value)
    }
}

fn bad(message: impl Into<String>) -> Box<EvalAltResult> {
    message.into().into()
}

/// JSON as Rhai's own values, which is the whole of what "pure data" means here.
///
/// One mapping, no surprises: an object is a map, an array is an array, `null` is `()` — the
/// same empty a cell takes. A number is an integer when it is one and a float otherwise, which
/// matters because a spreadsheet writes `1800` and `1800.0` the same way but a script comparing
/// them does not.
///
/// **Object keys come back sorted**, because Rhai's map and `serde_json`'s are both ordered by
/// key. JSON says an object is unordered, so nothing is lost — but a script that wants the
/// author's order wants an array, and this comment is where that is said.
fn dynamic(value: serde_json::Value) -> Dynamic {
    use serde_json::Value;
    match value {
        Value::Null => Dynamic::UNIT,
        Value::Bool(b) => b.into(),
        Value::Number(n) => match n.as_i64() {
            Some(i) => i.into(),
            None => n.as_f64().unwrap_or(f64::NAN).into(),
        },
        Value::String(s) => s.into(),
        Value::Array(items) => items
            .into_iter()
            .map(self::dynamic)
            .collect::<Vec<_>>()
            .into(),
        Value::Object(fields) => fields
            .into_iter()
            .map(|(key, value)| (key.into(), self::dynamic(value)))
            .collect::<Map>()
            .into(),
    }
}

/// `json(name)`, the only door a script has to anything outside itself.
pub(crate) fn register(engine: &mut Engine, data: Rc<dyn Data>) {
    let loaded = Loaded::new(data);
    engine.register_fn("json", move |name: &str| loaded.json(name));
}
