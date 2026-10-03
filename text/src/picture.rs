// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Putting a picture in a document — the half of *Insert Picture* that is not a file dialog.
//!
//! Hoisted out of the macOS shell when the browser wanted the same and the GNOME and Windows
//! windows each already had a copy. A picture goes in a **paragraph of its own**, below the
//! caret's block rather than at the caret: an image sitting mid-sentence still draws as the
//! placeholder character everywhere in this suite (nothing lays inline content out around one
//! yet), and an Insert Picture whose result is `\u{fffc}` would be a bug report. A block that is
//! *already* empty is used as it stands, so pressing Return and then inserting does what it looks
//! like.

use crate::{App, BlockKind, Caret, Result};

/// The MIME type of a picture's bytes, read from their signature rather than a file's name —
/// the way `grind_core::kind` reads a document's — or `None` for bytes that are no picture this
/// build knows. The list is what `NSImage` reads and an ODF consumer can be expected to.
pub fn mime(bytes: &[u8]) -> Option<&'static str> {
    let at = |offset: usize, magic: &[u8]| bytes.get(offset..offset + magic.len()) == Some(magic);
    Some(match () {
        _ if at(0, b"\x89PNG\r\n\x1a\n") => "image/png",
        _ if at(0, b"\xff\xd8\xff") => "image/jpeg",
        _ if at(0, b"GIF87a") || at(0, b"GIF89a") => "image/gif",
        _ if at(0, b"II*\0") || at(0, b"MM\0*") => "image/tiff",
        _ if at(0, b"RIFF") && at(8, b"WEBP") => "image/webp",
        _ if at(0, b"BM") => "image/bmp",
        _ if at(4, b"ftypheic") || at(4, b"ftypheix") || at(4, b"ftypmif1") => "image/heic",
        _ if svg(bytes) => "image/svg+xml",
        _ => return None,
    })
}

/// Whether some bytes are an SVG document: text whose first element is `svg`, after any XML
/// declaration, comment or doctype.
fn svg(bytes: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(1024)]);
    let mut rest = head.trim_start_matches('\u{feff}').trim_start();
    while let Some(after) = rest.strip_prefix("<?").or_else(|| rest.strip_prefix("<!")) {
        let Some(end) = after.find('>') else {
            return false;
        };
        rest = after[end + 1..].trim_start();
    }
    rest.starts_with("<svg")
}

/// Insert `data` (of type `mime`) below the caret's block — or in it, when it is empty — and
/// answer the block it landed in. The caret belongs at offset 1 of that block afterwards, past
/// the picture, so the next thing typed is a caption.
pub fn insert_below(app: &App, block: usize, mime: &str, data: Vec<u8>) -> Result<usize> {
    let empty = app.input_text(block).is_ok_and(|text| text.is_empty());
    let at = match empty {
        true => block,
        false => {
            app.insert(block + 1, BlockKind::Paragraph, "")?;
            block + 1
        }
    };
    app.insert_image(
        Caret {
            block: at,
            offset: 0,
        },
        mime.to_owned(),
        data,
        None,
        None,
    )?;
    Ok(at)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_is_known_by_its_signature() {
        assert_eq!(mime(b"\x89PNG\r\n\x1a\n...."), Some("image/png"));
        assert_eq!(mime(b"\xff\xd8\xff\xe0"), Some("image/jpeg"));
        assert_eq!(mime(b"GIF89a"), Some("image/gif"));
        assert_eq!(mime(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(mime(b"\0\0\0\x18ftypheic"), Some("image/heic"));
        assert_eq!(
            mime(b"<?xml version=\"1.0\"?>\n<!-- drawn -->\n<svg xmlns=\"...\">"),
            Some("image/svg+xml")
        );
        assert_eq!(mime(b"<html>"), None);
        assert_eq!(mime(b"PK\x03\x04"), None, "a document is not a picture");
        assert_eq!(mime(b""), None);
    }

    #[test]
    fn a_picture_takes_the_empty_block_or_a_new_one_below() {
        let app = App::new();
        app.set_text(0, "caption me").unwrap();
        let png = || vec![0x89, b'P', b'N', b'G'];
        assert_eq!(insert_below(&app, 0, "image/png", png()).unwrap(), 1);
        assert_eq!(app.block_count(), 2);
        // The picture's own block is not empty text-wise in the model, but a blank one is used.
        app.insert(2, BlockKind::Paragraph, "").unwrap();
        assert_eq!(insert_below(&app, 2, "image/png", png()).unwrap(), 2);
    }
}
