// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `json("prices.json")` — data a script reads, and the four walls around it. The one exception to
//! "no I/O": a script reads **data**, never code, and only from one directory a person named.
//!
//! 1. **JSON, nothing that can execute** — parsed by `serde_json` into Rhai data. The function's
//!    name says which parser ran, so a `csv(…)` would be a peer.
//! 2. **One directory.** `..` and absolute paths are refused before the filesystem is touched, and
//!    the canonical result must lie inside the root, which is what stops a symlink. `grind build`
//!    roots it at the script's directory unless `--data` says otherwise.
//! 3. **Bounded**: [`MAX_BYTES`] per file, [`MAX_FILES`] per script.
//! 4. **Read only, and cached**, so a `json(…)` inside a loop is one read.
//!
//! Determinism becomes: the same source *and the same data* produce the same bytes. The crate
//! still has no filesystem of its own: [`Data`] is a trait, [`Directory`] one implementation, and
//! [`NoData`] the default (rule 5).

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

use rhai::{Dynamic, Engine, EvalAltResult, Map};

/// The largest data file a script may read, in bytes.
pub const MAX_BYTES: usize = 8 * 1024 * 1024;

/// How many distinct files one script may read. Reading the same one again is free (it is
/// cached) and does not count twice.
pub const MAX_FILES: usize = 64;

/// Where a script's data comes from: a [`Directory`] from `grind-cli`, a map in memory from a
/// test, [`NoData`] from a host with no filesystem.
pub trait Data {
    /// The text of one data file, by the name the script used. The name is untrusted: **an
    /// implementation must refuse anything outside its own root**, as [`Directory`] does.
    fn read(&self, name: &str) -> Result<String, String>;

    /// What to say when there is nothing to read from. Overridden by [`NoData`] so that the
    /// error names the flag rather than the file.
    fn unavailable(&self) -> Option<String> {
        None
    }
}

/// No data at all: every read is an error that explains how to get some — loudly, at the line that
/// asked, rather than an empty directory.
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

/// One directory, and nothing outside it. The root is canonicalised once; every read is
/// canonicalised and checked against it, so a symlink pointing out is refused.
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

    /// The files this script read, in the order it first read them — a build's inputs.
    pub fn inputs(&self) -> Vec<PathBuf> {
        self.read.borrow().clone()
    }

    /// `name` as a path inside the root, or an error saying which wall it hit. The name is checked
    /// before the filesystem is touched (`../../etc/hosts`), and the canonical result after, since
    /// an innocent name can still be a symlink pointing out.
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

/// The [`Data`] a script runs against, plus what it has already read — the cache that makes a
/// `json(…)` in a loop one read, and where [`MAX_FILES`] is counted.
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

    /// One file's text, through the same walls whatever it is parsed as afterwards.
    fn text(&self, name: &str) -> Result<String, Box<EvalAltResult>> {
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
        Ok(text)
    }

    /// A flat ODF text document to start a `Text` from. Counted with the data files, and
    /// cached under a key no JSON name can have.
    fn template(&self, name: &str) -> Result<String, Box<EvalAltResult>> {
        let key = format!("template:{name}");
        if let Some(value) = self.cache.borrow().get(&key) {
            return Ok(value.clone().cast::<String>());
        }
        let text = self.text(name)?;
        self.cache
            .borrow_mut()
            .insert(key, Dynamic::from(text.clone()));
        Ok(text)
    }

    fn json(&self, name: &str) -> Result<Dynamic, Box<EvalAltResult>> {
        if let Some(value) = self.cache.borrow().get(name) {
            return Ok(value.clone());
        }
        let text = self.text(name)?;
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

/// JSON as Rhai values: an object is a map, an array an array, `null` is `()` — the empty a cell
/// takes. A number is an integer when it is one, since a script comparing `1800` with `1800.0` can
/// tell them apart.
///
/// **Object keys come back sorted**, as both Rhai's and `serde_json`'s maps are ordered by key; a
/// script that wants the author's order wants an array.
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
    let loaded = Rc::new(Loaded::new(data));
    let templates = Rc::clone(&loaded);
    crate::hint::hint(
        engine,
        "text_from",
        ["template: string", "Text"],
        [
            "/// An empty text document that starts from a template: a flat ODF text document",
            "/// (`.fodt`) in the data directory, whose page, master pages — letterhead,",
            "/// footer — and named paragraph styles the document keeps. Its own body is",
            "/// dropped; say new blocks into it, naming a style with `para(style, text)`.",
            "///",
            "/// The template is read through the same walls as `json`.",
        ],
        move |name: &str| -> Result<crate::text::Doc, Box<EvalAltResult>> {
            Ok(crate::text::Doc::from_template(templates.template(name)?))
        },
    );
    crate::hint::hint(
        engine,
        "json",
        ["name: string", "?"],
        [
            "/// The data in one JSON file, as values a script can use.",
            "///",
            "/// An object is a map, an array is an array, `null` is `()`, and object keys come",
            "/// back sorted. The file is read from the data directory — the script\'s own, or",
            "/// the one `--data` names — and `..`, an absolute path and a symlink pointing out",
            "/// are all refused. Reading the same file twice is free.",
        ],
        move |name: &str| loaded.json(name),
    );
}
