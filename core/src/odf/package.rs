// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Getting at `content.xml`, whichever physical form the document takes. **[GENERIC]**
//!
//! An ODF document is either a zip (`.ods`) or a single flat XML file (`.fods`); both
//! carry the same logical content model, and only the packaging differs
//! (doc/ods-format.md §1). Nothing below is spreadsheet-specific.

use std::io::{Cursor, Read};

use crate::{Error, Result};

/// Zip local file header. The package form always starts with it, because `mimetype` must
/// be the first entry (§1.1).
const ZIP_MAGIC: &[u8] = b"PK\x03\x04";

/// Is this the package form rather than the flat form?
///
/// Sniffed from the bytes, not the file extension: `.fods` content turns up under `.xml`,
/// and an extension is a hint from a filesystem rather than a fact about the data.
pub fn is_package(bytes: &[u8]) -> bool {
    bytes.starts_with(ZIP_MAGIC)
}

/// Extract `content.xml` from a document in either form.
///
/// Known gap: §9 describes LibreOffice falling back to a brute-force scan for local file
/// headers when a zip's central directory is unusable. Not implemented — the `zip` crate
/// handles well-formed archives, and a corrupt-archive recovery path belongs with the
/// explicit repair mode of §8.2 rather than being smeared into the normal read. If loop A
/// ever fails on a corpus file for this reason, that is the trigger to build it.
pub fn content_xml(bytes: &[u8]) -> Result<Vec<u8>> {
    if !is_package(bytes) {
        return Ok(bytes.to_vec());
    }

    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| Error::Package(format!("not a readable ODF package: {e}")))?;

    if is_encrypted(&mut archive) {
        return Err(Error::Encrypted);
    }

    let mut file = archive
        .by_name("content.xml")
        .map_err(|e| Error::Package(format!("no content.xml in package: {e}")))?;
    let mut out = Vec::with_capacity(file.size() as usize);
    file.read_to_end(&mut out)?;
    Ok(out)
}

/// `styles.xml`, if the package has one.
///
/// Separate from [`content_xml`] because it is optional in a way content is not: the flat
/// form has no such part at all (its `office:styles` sits in the one file), and a package
/// written minimally — ours is — leaves it out. A document referencing a style that lived
/// only there loses the style, never the document, so every failure here is `None`.
pub fn styles_xml(bytes: &[u8]) -> Option<Vec<u8>> {
    if !is_package(bytes) {
        return None;
    }
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).ok()?;
    let mut file = archive.by_name("styles.xml").ok()?;
    let mut out = Vec::with_capacity(file.size() as usize);
    file.read_to_end(&mut out).ok()?;
    Some(out)
}

/// Does the manifest declare any part as encrypted?
///
/// Worth detecting up front. A password-protected document's `content.xml` is ciphertext,
/// so without this the reader reports a bogus XML syntax error for a file that is
/// perfectly well formed and simply not ours to open. Absence of a key is not corruption.
fn is_encrypted<R: std::io::Read + std::io::Seek>(archive: &mut zip::ZipArchive<R>) -> bool {
    let Ok(mut manifest) = archive.by_name("META-INF/manifest.xml") else {
        return false;
    };
    let mut buf = Vec::new();
    if manifest.read_to_end(&mut buf).is_err() {
        return false;
    }
    // `manifest:encryption-data` appears only on entries that are actually encrypted, so
    // its presence anywhere is sufficient — no parse needed for a yes/no question.
    buf.windows(b"encryption-data".len())
        .any(|w| w == b"encryption-data")
}
