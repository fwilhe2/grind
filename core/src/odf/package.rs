// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Getting at `content.xml`, whichever physical form the document takes. **\[GENERIC\]**
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
    // **Not `?`.** Decompressing a zip entry reports a corrupt one — a bad CRC-32, a truncated
    // deflate stream — as an `io::Error`, and letting that convert into [`Error::Io`] would say
    // "the filesystem failed" about a file that was read from disk perfectly and is simply
    // damaged inside. Loop A found this: a fuzzer's corrupt `.odt` came back as `io: Invalid
    // checksum`, which is true of no filesystem anywhere.
    file.read_to_end(&mut out)
        .map_err(|e| Error::Package(format!("content.xml will not decompress: {e}")))?;
    Ok(out)
}

/// `styles.xml`, if the package has one.
///
/// Separate from [`content_xml`] because it is optional in a way content is not: the flat
/// form has no such part at all (its `office:styles` sits in the one file), and a package
/// written minimally — ours is — leaves it out. A document referencing a style that lived
/// only there loses the style, never the document, so every failure here is `None`.
pub fn styles_xml(bytes: &[u8]) -> Option<Vec<u8>> {
    part(bytes, "styles.xml")
}

/// Any other part of a package, by its path inside the zip — `Pictures/foo.jpg`, the target of
/// a `draw:image`'s `xlink:href` (rng:1626), being the reason this exists rather than only
/// [`content_xml`] and [`styles_xml`]. `None` for the flat form (there is no separate part to
/// fetch — everything is already in the one file a caller has) and for anything the zip does
/// not actually hold, on the same "lose the picture, not the document" principle as
/// [`styles_xml`].
pub fn part(bytes: &[u8], path: &str) -> Option<Vec<u8>> {
    if !is_package(bytes) {
        return None;
    }
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).ok()?;
    let mut file = archive.by_name(path).ok()?;
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

/// The ODF version this build writes. One constant for every document type, because a package
/// declares it in the manifest and each content part declares it again on its root.
pub const VERSION: &str = "1.4";

/// A document inside the package — an embedded object's own document: a directory of its own,
/// holding a `content.xml` rooted at `office:document-content`, and declared in the manifest by
/// a directory entry that carries its media type. **\[GENERIC\]** — what kind of document it
/// is, and what it says, are the caller's business; a spreadsheet's charts are the one caller.
///
/// This is how LibreOffice writes an embedded chart in a package (`Object 1/content.xml`,
/// `doc/chart-format.md`), and the only way it *reads* one there: an inline `office:document`
/// inside a package's `draw:object` is valid ODF, and LibreOffice drops it on load.
pub struct SubDocument {
    /// The directory, with no leading `./` and no trailing `/` — `Object 1`.
    pub directory: String,
    /// The sub-document's own media type, which is what its manifest entry declares.
    pub mimetype: &'static str,
    /// Its `content.xml`, complete.
    pub content: String,
}

/// `META-INF/manifest.xml` for a minimal package: the document itself and `content.xml`.
///
/// Minimal by intent (§1.4) — a manifest lists what the package *holds*, and this writer holds
/// two entries. Listing a `styles.xml` that is not there would be a lie a reader acts on.
pub fn manifest(mimetype: &str) -> String {
    manifest_with(mimetype, false, &[])
}

/// [`manifest`], plus `styles.xml` when the package holds one, and two entries per
/// [`SubDocument`]: its directory, carrying its media type, and the `content.xml` inside it.
fn manifest_with(mimetype: &str, styles: bool, subdocuments: &[SubDocument]) -> String {
    let mut entries = String::new();
    if styles {
        entries.push_str(
            "\x20<manifest:file-entry manifest:full-path=\"styles.xml\" \
             manifest:media-type=\"text/xml\"/>\n",
        );
    }
    for sub in subdocuments {
        entries.push_str(&format!(
            "\x20<manifest:file-entry manifest:full-path=\"{dir}/\" manifest:version=\"{VERSION}\" \
             manifest:media-type=\"{mimetype}\"/>\n\
             \x20<manifest:file-entry manifest:full-path=\"{dir}/content.xml\" \
             manifest:media-type=\"text/xml\"/>\n",
            dir = crate::odf::xml::esc(&sub.directory),
            mimetype = sub.mimetype,
        ));
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <manifest:manifest \
         xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" \
         manifest:version=\"{VERSION}\">\n\
         \x20<manifest:file-entry manifest:full-path=\"/\" manifest:version=\"{VERSION}\" \
         manifest:media-type=\"{mimetype}\"/>\n\
         \x20<manifest:file-entry manifest:full-path=\"content.xml\" \
         manifest:media-type=\"text/xml\"/>\n\
         {entries}\
         </manifest:manifest>\n"
    )
}

/// Wrap a `content.xml` as a package (§1.1). **\[GENERIC\]** — the only thing that varies
/// between document types is the media type string.
///
/// `mimetype` goes first, **stored uncompressed**, raw bytes, no trailing newline: readers
/// sniff it at a fixed offset before parsing any XML, so this is not somewhere to be creative.
pub fn write_package(mimetype: &str, content: &str) -> Result<Vec<u8>> {
    write_package_with(mimetype, content, None, &[])
}

/// [`write_package`], with a `styles.xml` when there are common styles to put in one — the
/// part `office:styles` lives in, since `content.xml`'s root does not allow it — and
/// [`SubDocument`]s beside the content.
pub fn write_package_with(
    mimetype: &str,
    content: &str,
    styles: Option<&str>,
    subdocuments: &[SubDocument],
) -> Result<Vec<u8>> {
    use std::io::Write as _;

    let zip = |e: zip::result::ZipError| Error::Package(e.to_string());
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));

    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    w.start_file("mimetype", stored).map_err(zip)?;
    w.write_all(mimetype.as_bytes())?;

    let deflated = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    w.start_file("META-INF/manifest.xml", deflated)
        .map_err(zip)?;
    w.write_all(manifest_with(mimetype, styles.is_some(), subdocuments).as_bytes())?;

    w.start_file("content.xml", deflated).map_err(zip)?;
    w.write_all(content.as_bytes())?;

    if let Some(styles) = styles {
        w.start_file("styles.xml", deflated).map_err(zip)?;
        w.write_all(styles.as_bytes())?;
    }

    for sub in subdocuments {
        w.start_file(format!("{}/content.xml", sub.directory), deflated)
            .map_err(zip)?;
        w.write_all(sub.content.as_bytes())?;
    }

    Ok(w.finish().map_err(zip)?.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §1.1's fixed-offset contract, which is the whole reason `mimetype` is written the way
    /// it is: a reader identifies the document *without unzipping anything*, so the entry has
    /// to be first, stored, and carry no extra field. Get any of the three wrong and the file
    /// is still a valid zip that nothing recognises.
    #[test]
    fn the_package_starts_with_an_uncompressed_mimetype_entry() {
        const MEDIA: &str = "application/vnd.oasis.opendocument.text";
        let bytes = write_package(MEDIA, "<x/>").expect("writes");

        assert_eq!(&bytes[..4], b"PK\x03\x04");
        // Local header is 30 bytes, then the name, then the raw media type.
        assert_eq!(
            &bytes[8..10],
            &[0, 0],
            "mimetype must be stored, not deflated"
        );
        assert_eq!(
            &bytes[28..30],
            &[0, 0],
            "mimetype entry must carry no extra field"
        );
        assert_eq!(&bytes[30..38], b"mimetype");
        assert_eq!(&bytes[38..38 + MEDIA.len()], MEDIA.as_bytes());
    }

    /// A sub-document is a directory the manifest declares with its own media type, and a
    /// `content.xml` inside it a reader can reach by path.
    #[test]
    fn a_sub_document_is_declared_and_readable() {
        const CHART: &str = "application/vnd.oasis.opendocument.chart";
        let bytes = write_package_with(
            "application/vnd.oasis.opendocument.spreadsheet",
            "<x/>",
            None,
            &[SubDocument {
                directory: "Object 1".to_owned(),
                mimetype: CHART,
                content: "<y/>".to_owned(),
            }],
        )
        .expect("writes");
        assert_eq!(
            part(&bytes, "Object 1/content.xml").as_deref(),
            Some(&b"<y/>"[..])
        );
        let manifest = String::from_utf8(part(&bytes, "META-INF/manifest.xml").unwrap()).unwrap();
        assert!(manifest.contains(&format!(
            "manifest:full-path=\"Object 1/\" manifest:version=\"{VERSION}\" \
             manifest:media-type=\"{CHART}\""
        )));
        assert!(manifest.contains("manifest:full-path=\"Object 1/content.xml\""));
    }

    #[test]
    fn a_written_package_reads_its_own_content_back() {
        let bytes =
            write_package("application/vnd.oasis.opendocument.text", "<x/>").expect("writes");
        assert!(is_package(&bytes));
        assert_eq!(content_xml(&bytes).expect("has content.xml"), b"<x/>");
    }
}
