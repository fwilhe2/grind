// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Putting a picture in a document — the half of Insert Picture that is not a file dialog. A
//! picture goes in a paragraph of its own below the caret's block, since an image mid-sentence
//! draws only as the placeholder character; an already empty block is used as it is.
//!
//! The other half is alternative text — what a screen reader says and Markdown writes as `![alt]`:
//! [`at`] is which picture a caret means, [`set_alt`] writes it, and [`advice`] is the one note
//! every dialog shows under the field.

use crate::{App, BlockKind, BlockView, Caret, ImageView, Result};

/// The MIME type of a picture's bytes, from their signature rather than a name, or `None` for
/// bytes that are no picture known here — the formats `NSImage` reads and an ODF consumer can be
/// expected to.
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

/// Whether some bytes are an SVG document: text whose first element, after any XML declaration,
/// comment or doctype, is `svg`.
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

/// Insert `data` (of type `mime`) below the caret's block, or in it when it is empty, returning
/// the block it landed in. The caret belongs at offset 1 afterwards, so the next thing typed is a
/// caption.
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

/// The picture a caret in `block` means, and its offset — what Alt Text… edits: the one just after
/// the caret, else just before it, else, when the block is a picture ([`crate::picture_of`]), that
/// one. A picture elsewhere in a long paragraph is not guessed at.
pub fn at(block: &BlockView, offset: usize) -> Option<(usize, &ImageView)> {
    let image = |start: usize| {
        block
            .runs
            .iter()
            .find(|run| run.start == start)
            .and_then(|run| run.image.as_ref())
            .map(|image| (start, image))
    };
    image(offset)
        .or_else(|| offset.checked_sub(1).and_then(image))
        .or_else(|| crate::picture_of(block).and_then(|_| image(0)))
}

/// Give the picture at `at` its alternative text, each half trimmed and a blank one removed.
pub fn set_alt(app: &App, at: Caret, title: &str, description: &str) -> Result<()> {
    let keep = |s: &str| Some(s.trim().to_owned()).filter(|s| !s.is_empty());
    app.set_image_alt(at, keep(title), keep(description))
}

/// About how long a short text can be before [`advice`] suggests the description (WebAIM's
/// convention); nothing longer is refused.
pub const SHORT: usize = 150;

/// One sentence of advice about a short alternative text as it is typed, or `None`.
pub fn advice(title: &str) -> Option<&'static str> {
    let title = title.trim();
    let lower = title.to_lowercase();
    if title.is_empty() {
        return Some("Say what the picture shows, as you would to someone who cannot see it.");
    }
    if [
        "image of",
        "picture of",
        "photo of",
        "graphic of",
        "bild von",
        "foto von",
    ]
    .iter()
    .any(|lead| lower.starts_with(lead))
    {
        return Some("No need to say it is a picture — a screen reader already does.");
    }
    if title.chars().count() > SHORT {
        return Some("That is long to hear — put the detail in the description.");
    }
    None
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

    fn block(app: &App, index: usize) -> BlockView {
        app.get_viewport(index..index + 1)
            .iter()
            .next()
            .cloned()
            .unwrap()
    }

    #[test]
    fn the_picture_a_caret_means() {
        let app = App::new();
        app.set_text(0, "ab").unwrap();
        let png = vec![0x89, b'P', b'N', b'G'];
        app.insert_image(
            Caret {
                block: 0,
                offset: 1,
            },
            "image/png".into(),
            png,
            None,
            None,
        )
        .unwrap();
        // a ￼ b: in front of it, just past it, and not from across the paragraph.
        let b = block(&app, 0);
        assert_eq!(at(&b, 1).map(|(o, _)| o), Some(1));
        assert_eq!(at(&b, 2).map(|(o, _)| o), Some(1));
        assert_eq!(at(&b, 0), None);
        assert_eq!(at(&b, 3), None);
        // A block that is a picture and a caption: anywhere in the caption.
        let p = insert_below(&app, 0, "image/png", vec![0x89, b'P', b'N', b'G']).unwrap();
        app.insert_text(
            Caret {
                block: p,
                offset: 1,
            },
            "A long caption",
        )
        .unwrap();
        assert_eq!(at(&block(&app, p), 9).map(|(o, _)| o), Some(0));
    }

    #[test]
    fn alt_text_is_trimmed_and_blank_is_none() {
        let app = App::new();
        app.set_text(0, "words").unwrap();
        let p = insert_below(&app, 0, "image/png", vec![0x89, b'P', b'N', b'G']).unwrap();
        let caret = Caret {
            block: p,
            offset: 0,
        };
        set_alt(&app, caret, "  A heron, standing  ", " ").unwrap();
        let b = block(&app, p);
        let (_, image) = at(&b, 0).unwrap();
        assert_eq!(image.title.as_deref(), Some("A heron, standing"));
        assert_eq!(image.description, None);
        set_alt(&app, caret, "", "").unwrap();
        assert_eq!(at(&block(&app, p), 0).unwrap().1.title, None);
        // Not in front of a picture is an error, not a search.
        assert!(
            set_alt(
                &app,
                Caret {
                    block: 0,
                    offset: 0
                },
                "x",
                ""
            )
            .is_err()
        );
    }

    #[test]
    fn advice_is_one_sentence_or_none() {
        assert!(advice("").is_some());
        assert!(
            advice("Image of a heron")
                .unwrap()
                .contains("screen reader")
        );
        assert_eq!(advice("A grey heron standing in shallow water"), None);
        assert!(advice(&"word ".repeat(40)).unwrap().contains("description"));
    }
}
