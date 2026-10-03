// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Writing a file so that a crash leaves either the old one or the new one, never half of one.
//!
//! `std::fs::write` truncates the target and then fills it, so a crash, a full disk or a killed
//! process between the two destroys the user's document — the one thing an editor must not do.
//! [`write()`] writes a sibling temporary file, flushes it to disk and renames it over the target;
//! a rename within one directory is atomic on every platform this suite runs on.
//!
//! What it deliberately keeps of the file it replaces: its **permissions**, and its *identity*
//! when the path is a symlink (the link's target is replaced, not the link). What it cannot keep
//! is ownership, extended attributes, and a hard link's sharing — the new file is a new inode.
//! Where a rename is impossible — the directory is not writable by this user but the file is, or
//! the path names something that is not a regular file (`/dev/stdout`, a pipe) — it falls back
//! to writing in place, which is exactly what every save did before this existed.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Replace `path` with `bytes`, atomically where the file system allows it.
pub fn write(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> io::Result<()> {
    write_bytes(path.as_ref(), bytes.as_ref())
}

fn write_bytes(path: &Path, bytes: &[u8]) -> io::Result<()> {
    // A symlink is replaced through, so saving never turns somebody's link into a file.
    let target = match fs::canonicalize(path) {
        Ok(real) => real,
        Err(_) => path.to_path_buf(),
    };
    let existing = fs::metadata(&target).ok();
    if existing.as_ref().is_some_and(|meta| !meta.is_file()) {
        return fs::write(path, bytes);
    }

    let temp = temp_name(&target);
    match write_then_rename(&temp, &target, bytes, existing.as_ref()) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(&temp);
            if error.kind() == io::ErrorKind::PermissionDenied && existing.is_some() {
                // The file is ours to write and its directory is not.
                fs::write(path, bytes)
            } else {
                Err(error)
            }
        }
    }
}

fn write_then_rename(
    temp: &Path,
    target: &Path,
    bytes: &[u8],
    existing: Option<&fs::Metadata>,
) -> io::Result<()> {
    let mut file: File = OpenOptions::new().write(true).create_new(true).open(temp)?;
    file.write_all(bytes)?;
    if let Some(meta) = existing {
        file.set_permissions(meta.permissions())?;
    }
    // Data must be on disk *before* the rename, or a crash can leave the new name on an empty
    // file — the failure this whole module exists to rule out.
    file.sync_all()?;
    drop(file);
    fs::rename(temp, target)?;
    sync_directory(target);
    Ok(())
}

/// Make the rename itself durable. Best effort: a directory cannot be opened for this on
/// Windows, and a failure here costs durability across a power cut, not correctness.
fn sync_directory(target: &Path) {
    #[cfg(unix)]
    if let Some(dir) = target.parent() {
        let dir = if dir.as_os_str().is_empty() {
            Path::new(".")
        } else {
            dir
        };
        if let Ok(handle) = File::open(dir) {
            let _ = handle.sync_all();
        }
    }
    #[cfg(not(unix))]
    let _ = target;
}

/// A name beside the target, unique per process and call, and hidden on Unix so a file
/// manager does not show one for the instant it exists.
fn temp_name(target: &Path) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let unique = format!(
        ".{name}.grind-{}-{}.tmp",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    target.with_file_name(unique)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("grind-atomic-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn leftovers(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn creates_and_replaces_without_leaving_a_temporary_behind() {
        let dir = scratch("replace");
        let file = dir.join("book.fods");
        write(&file, b"one").unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"one");
        write(&file, b"two, longer").unwrap();
        write(&file, b"3").unwrap();
        assert_eq!(
            fs::read(&file).unwrap(),
            b"3",
            "a shorter file leaves no tail of the old one"
        );
        assert_eq!(leftovers(&dir), ["book.fods"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failed_write_leaves_the_old_file_alone() {
        let dir = scratch("failed");
        let file = dir.join("book.fods");
        write(&file, b"precious").unwrap();
        // A target whose directory does not exist cannot be written, and must not be created.
        assert!(write(dir.join("missing/book.fods"), b"x").is_err());
        assert_eq!(fs::read(&file).unwrap(), b"precious");
        assert_eq!(leftovers(&dir), ["book.fods"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn permissions_survive_and_a_symlink_stays_a_symlink() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = scratch("unix");
        let real = dir.join("real.fods");
        write(&real, b"a").unwrap();
        fs::set_permissions(&real, fs::Permissions::from_mode(0o640)).unwrap();
        let link = dir.join("link.fods");
        symlink(&real, &link).unwrap();

        write(&link, b"b").unwrap();
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(&real).unwrap(), b"b");
        assert_eq!(
            fs::metadata(&real).unwrap().permissions().mode() & 0o777,
            0o640
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn something_that_is_not_a_regular_file_is_written_in_place() {
        // `/dev/null` accepts anything and a rename over it would be a disaster.
        write(Path::new("/dev/null"), b"discarded").unwrap();
        assert!(Path::new("/dev/null").exists());
    }
}
