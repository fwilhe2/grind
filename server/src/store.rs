// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The store: a directory of ODF files and nothing else (`doc/server.md` §3, Tier 1), so every
//! other client opens the same files. Ids live in `.grind/index.json`, because a name is not a
//! key; a version tag is a hash of the bytes, so a file another program changed is simply a new
//! version.

use std::collections::BTreeMap;
use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use grind_core::DocumentKind;
use serde::{Deserialize, Serialize};

use crate::git::{self, Git, Version};

/// The four extensions a document is listed under: the kind and the form are read off them.
const EXTENSIONS: [(&str, DocumentKind, bool); 4] = [
    ("fods", DocumentKind::Spreadsheet, false),
    ("ods", DocumentKind::Spreadsheet, true),
    ("fodt", DocumentKind::Text, false),
    ("odt", DocumentKind::Text, true),
];

const META: &str = ".grind";

pub struct Store {
    root: PathBuf,
    git: Option<Git>,
}

/// A document as the library lists it.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    /// The path below the store, `/`-separated.
    pub name: String,
    pub kind: &'static str,
    pub size: u64,
    /// Seconds since the epoch.
    pub modified: u64,
    pub etag: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    NotFound,
    /// Somebody saved first; this is the version on disk now.
    Stale(String),
    Exists(String),
    BadName(String),
    Unreadable(String),
    /// The store has no history: not a git work tree.
    NoHistory,
    Failed(String),
}

impl From<std::io::Error> for Refusal {
    fn from(error: std::io::Error) -> Self {
        Refusal::Failed(error.to_string())
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Index {
    docs: BTreeMap<String, String>,
}

impl Store {
    /// `root` must exist. Its `.grind/` is made, and ignored by git from inside.
    pub fn open(root: &Path) -> Result<Store, String> {
        let root = root
            .canonicalize()
            .map_err(|e| format!("{}: {e}", root.display()))?;
        if !root.is_dir() {
            return Err(format!("{} is not a directory", root.display()));
        }
        let meta = root.join(META);
        std::fs::create_dir_all(&meta).map_err(|e| format!("{}: {e}", meta.display()))?;
        let ignore = meta.join(".gitignore");
        if !ignore.exists() {
            std::fs::write(&ignore, "*\n").map_err(|e| format!("{}: {e}", ignore.display()))?;
        }
        let git = Git::find(&root);
        Ok(Store { root, git })
    }

    pub fn has_history(&self) -> bool {
        self.git.is_some()
    }

    pub fn list(&self) -> Result<Vec<Entry>, Refusal> {
        let index = self.scan()?;
        let mut entries: Vec<Entry> = index
            .docs
            .iter()
            .filter_map(|(id, name)| self.entry(id, name).ok())
            .collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    }

    pub fn get(&self, id: &str) -> Result<(Entry, Vec<u8>), Refusal> {
        let name = self.name_of(id)?;
        let bytes = std::fs::read(self.root.join(&name))?;
        Ok((self.entry(id, &name)?, bytes))
    }

    /// Replace a document, if `if_match` is still the version on disk (`*` is any).
    pub fn put(&self, id: &str, if_match: &str, bytes: &[u8]) -> Result<Entry, Refusal> {
        let name = self.name_of(id)?;
        self.check_version(&name, Some(if_match))?;
        check_bytes(&name, bytes)?;
        grind_core::atomic::write(self.root.join(&name), bytes)?;
        self.commit(&[&name], &format!("Save {name}"));
        self.entry(id, &name)
    }

    /// A new document, in the store's top directory.
    pub fn create(&self, name: &str, bytes: &[u8]) -> Result<Entry, Refusal> {
        check_name(name)?;
        check_bytes(name, bytes)?;
        let path = self.root.join(name);
        if path.exists() {
            return Err(Refusal::Exists(name.to_owned()));
        }
        grind_core::atomic::write(&path, bytes)?;
        let mut index = self.scan()?;
        let id = match index.docs.iter().find(|(_, n)| *n == name) {
            Some((id, _)) => id.clone(),
            None => {
                let id = fresh_id(&index);
                index.docs.insert(id.clone(), name.to_owned());
                self.save_index(&index)?;
                id
            }
        };
        self.commit(&[name], &format!("Add {name}"));
        self.entry(&id, name)
    }

    /// A new name in the same directory, under the same extension: the form is in the bytes.
    pub fn rename(&self, id: &str, if_match: Option<&str>, new: &str) -> Result<Entry, Refusal> {
        check_name(new)?;
        let old = self.name_of(id)?;
        self.check_version(&old, if_match)?;
        let new = match old.rsplit_once('/') {
            Some((dir, _)) => format!("{dir}/{new}"),
            None => new.to_owned(),
        };
        if new == old {
            return self.entry(id, &old);
        }
        if extension(&old).map(|(e, ..)| e) != extension(&new).map(|(e, ..)| e) {
            return Err(Refusal::BadName(format!(
                "{new} would change what kind of file {old} is"
            )));
        }
        let to = self.root.join(&new);
        if to.exists() {
            return Err(Refusal::Exists(new));
        }
        std::fs::rename(self.root.join(&old), &to)?;
        let mut index = self.scan_from(self.load_index())?;
        index.docs.insert(id.to_owned(), new.clone());
        index.docs.retain(|other, name| other == id || *name != new);
        self.save_index(&index)?;
        self.commit(&[&old, &new], &format!("Rename {old} to {new}"));
        self.entry(id, &new)
    }

    /// Move a document into `.grind/trash/`, which keeps it when there is no history to.
    pub fn delete(&self, id: &str, if_match: Option<&str>) -> Result<(), Refusal> {
        let name = self.name_of(id)?;
        self.check_version(&name, if_match)?;
        let trash = self.root.join(META).join("trash");
        std::fs::create_dir_all(&trash)?;
        let file = name.rsplit('/').next().unwrap_or(&name);
        std::fs::rename(self.root.join(&name), trash.join(format!("{id}-{file}")))?;
        let mut index = self.load_index();
        index.docs.remove(id);
        self.save_index(&index)?;
        self.commit(&[&name], &format!("Delete {name}"));
        Ok(())
    }

    pub fn history(&self, id: &str) -> Result<Vec<Version>, Refusal> {
        let git = self.git.as_ref().ok_or(Refusal::NoHistory)?;
        let name = self.name_of(id)?;
        git.log(&name).map_err(Refusal::Failed)
    }

    /// The document's bytes at `rev`, one of its [`Store::history`]'s.
    pub fn at(&self, id: &str, rev: &str) -> Result<Vec<u8>, Refusal> {
        let git = self.git.as_ref().ok_or(Refusal::NoHistory)?;
        let version = self
            .history(id)?
            .into_iter()
            .find(|v| git::is_rev(rev) && v.rev == rev)
            .ok_or(Refusal::NotFound)?;
        git.show(&version.rev, &version.path)
            .map_err(Refusal::Failed)
    }

    /// Put the bytes of `rev` back as the current version — a save like any other.
    pub fn restore(&self, id: &str, if_match: &str, rev: &str) -> Result<Entry, Refusal> {
        let bytes = self.at(id, rev)?;
        let name = self.name_of(id)?;
        self.check_version(&name, Some(if_match))?;
        check_bytes(&name, &bytes)?;
        grind_core::atomic::write(self.root.join(&name), &bytes)?;
        self.commit(&[&name], &format!("Restore {name} to {}", &rev[..7]));
        self.entry(id, &name)
    }

    // --- the index ---

    fn name_of(&self, id: &str) -> Result<String, Refusal> {
        let index = self.load_index();
        match index.docs.get(id) {
            Some(name) if self.holds(name) => Ok(name.clone()),
            _ => match self.scan()?.docs.get(id) {
                Some(name) if self.holds(name) => Ok(name.clone()),
                _ => Err(Refusal::NotFound),
            },
        }
    }

    /// A regular file below the root, reached through no symbolic link: a save writes through a
    /// link, so a document swapped for one must not be written as the place it points to.
    fn holds(&self, name: &str) -> bool {
        let path = self.root.join(name);
        path.symlink_metadata().is_ok_and(|meta| meta.is_file())
            && path.canonicalize().is_ok_and(|real| real == path)
    }

    fn load_index(&self) -> Index {
        std::fs::read(self.index_path())
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn save_index(&self, index: &Index) -> Result<(), Refusal> {
        let bytes = serde_json::to_vec_pretty(index).map_err(|e| Refusal::Failed(e.to_string()))?;
        grind_core::atomic::write(self.index_path(), &bytes).map_err(Refusal::from)
    }

    fn index_path(&self) -> PathBuf {
        self.root.join(META).join("index.json")
    }

    /// The index made to match the directory: an id for every new document, none for a gone one.
    fn scan(&self) -> Result<Index, Refusal> {
        self.scan_from(self.load_index())
    }

    fn scan_from(&self, mut index: Index) -> Result<Index, Refusal> {
        let mut found = Vec::new();
        walk(&self.root, "", &mut found);
        let before = index.docs.len();
        let mut changed = false;
        index.docs.retain(|_, name| found.contains(name));
        changed |= index.docs.len() != before;
        for name in found {
            if !index.docs.values().any(|n| *n == name) {
                let id = fresh_id(&index);
                index.docs.insert(id, name);
                changed = true;
            }
        }
        if changed {
            self.save_index(&index)?;
        }
        Ok(index)
    }

    fn entry(&self, id: &str, name: &str) -> Result<Entry, Refusal> {
        let path = self.root.join(name);
        let meta = std::fs::metadata(&path).map_err(|_| Refusal::NotFound)?;
        let bytes = std::fs::read(&path)?;
        let kind = extension(name).ok_or(Refusal::NotFound)?.1;
        Ok(Entry {
            id: id.to_owned(),
            name: name.to_owned(),
            kind: kind_key(kind),
            size: meta.len(),
            modified: meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs()),
            etag: etag(&bytes),
        })
    }

    fn check_version(&self, name: &str, if_match: Option<&str>) -> Result<(), Refusal> {
        let Some(wanted) = if_match else {
            return Ok(());
        };
        let now = etag(&std::fs::read(self.root.join(name))?);
        match wanted == "*" || unquote(wanted) == unquote(&now) {
            true => Ok(()),
            false => Err(Refusal::Stale(now)),
        }
    }

    fn commit(&self, paths: &[&str], message: &str) {
        if let Some(git) = &self.git
            && let Err(error) = git.commit(paths, message)
        {
            // The file is written either way; a history that missed one save is not a reason to
            // refuse it.
            eprintln!("grind serve: {error}");
        }
    }
}

fn kind_key(kind: DocumentKind) -> &'static str {
    match kind {
        DocumentKind::Spreadsheet => "spreadsheet",
        DocumentKind::Text => "text",
        DocumentKind::Presentation => "presentation",
    }
}

/// The documents below `dir`, by extension. Dot entries and symbolic links are not followed, so
/// nothing written here lands outside the store.
fn walk(dir: &Path, prefix: &str, out: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if name.starts_with('.') || kind.is_symlink() {
            continue;
        }
        let relative = format!("{prefix}{name}");
        if kind.is_dir() {
            walk(&entry.path(), &format!("{relative}/"), out);
        } else if kind.is_file() && extension(&name).is_some() {
            out.push(relative);
        }
    }
}

fn extension(name: &str) -> Option<(&'static str, DocumentKind, bool)> {
    let (_, ext) = name.rsplit_once('.')?;
    EXTENSIONS
        .into_iter()
        .find(|(e, ..)| e.eq_ignore_ascii_case(ext))
}

/// A plain file name a document may be created or renamed to.
fn check_name(name: &str) -> Result<(), Refusal> {
    let bad = |why: &str| Err(Refusal::BadName(format!("{name}: {why}")));
    if name.is_empty() || name.starts_with('.') {
        return bad("a name may not be empty or start with a dot");
    }
    if name.contains(['/', '\\', '\0']) || name.chars().any(char::is_control) {
        return bad("a name may not contain a slash");
    }
    if extension(name).is_none() {
        return bad("a document is .fods, .ods, .fodt or .odt");
    }
    Ok(())
}

/// The bytes are a document of the kind and form the name says, and the core reads them.
fn check_bytes(name: &str, bytes: &[u8]) -> Result<(), Refusal> {
    let (_, kind, package) = extension(name).ok_or(Refusal::NotFound)?;
    let unreadable = |why: String| Err(Refusal::Unreadable(format!("{name}: {why}")));
    if bytes.starts_with(b"PK") != package {
        return unreadable(match package {
            true => "not a package, which the name says it is".to_owned(),
            false => "not a flat document, which the name says it is".to_owned(),
        });
    }
    if grind_core::kind::kind(bytes) != Some(kind) {
        return unreadable(format!("not a {}", kind.label()));
    }
    let read = match kind {
        DocumentKind::Spreadsheet => grind_sheet::read_bytes(name, bytes)
            .map(drop)
            .map_err(|e| e.to_string()),
        _ => grind_text::read_bytes(name, bytes)
            .map(drop)
            .map_err(|e| e.to_string()),
    };
    read.or_else(unreadable)
}

/// FNV-1a over the bytes, quoted as HTTP wants a tag: the same file is the same tag across
/// restarts and builds.
pub fn etag(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("\"{hash:016x}\"")
}

fn unquote(tag: &str) -> &str {
    tag.trim().trim_start_matches("W/").trim_matches('"')
}

/// Twelve hex digits from the standard library's per-process random keys.
fn fresh_id(index: &Index) -> String {
    loop {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_usize(index.docs.len());
        let id = format!("{:012x}", hasher.finish() >> 16);
        if !index.docs.contains_key(&id) {
            return id;
        }
    }
}
