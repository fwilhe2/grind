// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The documents opened or saved most recently, newest first: one absolute path a line in
//! `$XDG_STATE_HOME/grind/recent` (`~/.local/state/grind/recent`). Failing to read or write it
//! is never an error the user sees.

use std::path::{Path, PathBuf};

/// How many paths the file keeps.
const KEPT: usize = 20;

fn file() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| Some(PathBuf::from(std::env::var_os("HOME")?).join(".local/state")))?;
    Some(base.join("grind").join("recent"))
}

/// The recent documents that still exist, newest first.
pub fn list() -> Vec<PathBuf> {
    file().map(|file| read(&file)).unwrap_or_default()
}

/// Put `path` at the head of the list.
pub fn note(path: &Path) {
    if let Some(file) = file() {
        noted(&file, path);
    }
}

fn read(file: &Path) -> Vec<PathBuf> {
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .collect()
}

fn noted(file: &Path, path: &Path) {
    let Ok(path) = std::path::absolute(path) else {
        return;
    };
    let mut paths = read(file);
    paths.retain(|kept| *kept != path);
    paths.insert(0, path);
    paths.truncate(KEPT);
    let mut text = String::new();
    for path in &paths {
        // A name with a line break in it cannot be one line.
        if let Some(name) = path.to_str().filter(|name| !name.contains('\n')) {
            text.push_str(name);
            text.push('\n');
        }
    }
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(file, text);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_newest_comes_first_once_and_a_missing_file_drops_out() {
        let dir = std::env::temp_dir().join("grind-tui-recent-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let list = dir.join("state/grind/recent");
        let (a, b) = (dir.join("a.fods"), dir.join("b.fodt"));
        std::fs::write(&a, "").unwrap();
        std::fs::write(&b, "").unwrap();
        noted(&list, &a);
        noted(&list, &b);
        noted(&list, &a);
        assert_eq!(read(&list), [a.clone(), b.clone()]);
        std::fs::remove_file(&b).unwrap();
        assert_eq!(read(&list), [a]);
    }
}
