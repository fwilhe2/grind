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

/// A picture anchored to its paragraph that takes no room: `style:wrap="run-through"`, drawn over
/// the text where its graphic style puts it (`doc/odt-format.md` §5c fact 63). Each axis as the
/// file spelled it: `style:horizontal-pos`/`-rel` and `svg:x`, `style:vertical-pos`/`-rel` and
/// `svg:y`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Floating {
    pub across: (String, String, Option<String>),
    pub down: (String, String, Option<String>),
}

impl Floating {
    /// Each axis as a placement in millimetres: across, then down.
    #[must_use]
    pub fn placed(&self) -> (crate::marginal::Placed, crate::marginal::Placed) {
        let placed =
            |(pos, rel, offset): &(String, String, Option<String>)| crate::marginal::Placed {
                pos: pos.clone(),
                rel: rel.clone(),
                offset: offset
                    .as_deref()
                    .and_then(grind_core::style::length_mm)
                    .unwrap_or(0.0),
            };
        (placed(&self.across), placed(&self.down))
    }
}

/// How a frame the file spelled shows its picture, besides its size: where it floats and what
/// it cuts away.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Look {
    pub floating: Option<Floating>,
    pub crop: Option<Crop>,
    pub seat: Seat,
}

/// How a picture set as a character sits on its line, by its graphic style's
/// `style:vertical-pos` and `-rel` (`doc/odt-format.md` §5c fact 66).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Seat {
    /// Its foot on the baseline: neither said, or `top` of `baseline`.
    #[default]
    Baseline,
    /// Centred on the text's height: `middle` of `text`.
    MiddleOfText,
    /// Centred on the baseline: `middle` of `baseline`.
    MiddleOfBaseline,
    /// Any other, not measured: drawn as the placeholder character.
    Unmeasured,
}

impl Seat {
    /// From `style:vertical-pos` and `style:vertical-rel`, as spelled.
    #[must_use]
    pub fn of(pos: Option<&str>, rel: Option<&str>) -> Self {
        match (pos, rel) {
            (None, None) | (Some("top"), Some("baseline")) => Seat::Baseline,
            (Some("middle"), Some("text")) => Seat::MiddleOfText,
            (Some("middle"), Some("baseline")) => Seat::MiddleOfBaseline,
            _ => Seat::Unmeasured,
        }
    }
}

/// What a frame cuts from its picture (`fo:clip`, `doc/odt-format.md` §5c fact 64): ODF lengths
/// off its top, right, bottom and left edges, as spelled. A negative one adds empty room.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Crop(pub [String; 4]);

impl Crop {
    /// `rect(top, right, bottom, left)`, commas optional; `None` for `auto`, a rect with any
    /// `auto` in it, or one cutting nothing.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        let inner = value.trim().strip_prefix("rect(")?.strip_suffix(')')?;
        let edges: Vec<&str> = inner
            .split([',', ' '])
            .filter(|edge| !edge.is_empty())
            .collect();
        let edges: [&str; 4] = edges.try_into().ok()?;
        let mm = edges.map(grind_core::style::length_mm);
        if mm.iter().any(Option::is_none) || mm.iter().all(|edge| *edge == Some(0.0)) {
            return None;
        }
        Some(Self(edges.map(str::to_owned)))
    }

    /// The cuts as fractions of the picture's height and width — top, right, bottom, left —
    /// or `None` when the picture's own size is not known here.
    #[must_use]
    pub fn fractions(&self, data: &[u8]) -> Option<[f64; 4]> {
        let (width, height) = own_mm(data)?;
        let mm = |at: usize| grind_core::style::length_mm(&self.0[at]).unwrap_or(0.0);
        Some([mm(0) / height, mm(1) / width, mm(2) / height, mm(3) / width])
    }
}

/// A picture's own size in millimetres: its pixels at the density it states, or at 96 to the
/// inch when it states none or only an aspect ratio; a WMF's placeable bounds at its units to the
/// inch, an EMF's frame (`doc/odt-format.md` §5c fact 64). `None` for a JPEG without a JFIF
/// segment and any other format, whose size LibreOffice was not measured on.
#[must_use]
pub fn own_mm(data: &[u8]) -> Option<(f64, f64)> {
    const DEFAULT: f64 = 96.0 / 25.4;
    if let Some(size) = metafile_mm(data) {
        return Some(size);
    }
    let (width, height) = pixels(data)?;
    let per_mm = if data.starts_with(b"\x89PNG") {
        png_density(data).unwrap_or((DEFAULT, DEFAULT))
    } else if data.starts_with(&[0xFF, 0xD8]) {
        if data.get(6..11) != Some(b"JFIF\0") {
            return None;
        }
        let at = |i: usize| f64::from(u16::from_be_bytes([data[i], data[i + 1]]));
        match data.get(13..18).map(|_| data[13]) {
            Some(1) => (at(14) / 25.4, at(16) / 25.4),
            Some(2) => (at(14) / 10.0, at(16) / 10.0),
            _ => (DEFAULT, DEFAULT),
        }
    } else {
        (DEFAULT, DEFAULT)
    };
    let (x, y) = (
        per_mm.0.max(f64::MIN_POSITIVE),
        per_mm.1.max(f64::MIN_POSITIVE),
    );
    Some((f64::from(width) / x, f64::from(height) / y))
}

/// A placeable WMF's or an EMF's own size in millimetres.
fn metafile_mm(data: &[u8]) -> Option<(f64, f64)> {
    let i16_at = |at: usize| {
        Some(f64::from(i16::from_le_bytes(
            data.get(at..at + 2)?.try_into().ok()?,
        )))
    };
    let i32_at = |at: usize| {
        Some(f64::from(i32::from_le_bytes(
            data.get(at..at + 4)?.try_into().ok()?,
        )))
    };
    if data.starts_with(b"\xd7\xcd\xc6\x9a") {
        let inch = f64::from(u16::from_le_bytes(data.get(14..16)?.try_into().ok()?));
        let (width, height) = (
            (i16_at(10)? - i16_at(6)?).abs(),
            (i16_at(12)? - i16_at(8)?).abs(),
        );
        return (inch > 0.0 && width > 0.0 && height > 0.0)
            .then(|| (width / inch * 25.4, height / inch * 25.4));
    }
    if data.starts_with(&[1, 0, 0, 0]) && data.get(40..44) == Some(b" EMF") {
        let (width, height) = (i32_at(32)? - i32_at(24)?, i32_at(36)? - i32_at(28)?);
        return (width > 0.0 && height > 0.0).then(|| (width / 100.0, height / 100.0));
    }
    None
}

/// A PNG's `pHYs` density in pixels per millimetre, when it is in metres.
fn png_density(data: &[u8]) -> Option<(f64, f64)> {
    let mut at = 8;
    while let Some(head) = data.get(at..at + 8) {
        let length = u32::from_be_bytes(head[..4].try_into().ok()?) as usize;
        match &head[4..] {
            b"pHYs" => {
                let body = data.get(at + 8..at + 17)?;
                let per_metre = |i: usize| {
                    f64::from(u32::from_be_bytes(
                        body[i..i + 4].try_into().unwrap_or_default(),
                    ))
                };
                return (body[8] == 1).then(|| (per_metre(0) / 1000.0, per_metre(4) / 1000.0));
            }
            b"IDAT" | b"IEND" => return None,
            _ => at = at.checked_add(12 + length)?,
        }
    }
    None
}

/// The MIME type of a picture's bytes, from their signature rather than a name, or `None` for
/// bytes that are no picture known here — the formats `NSImage` reads and an ODF consumer can be
/// expected to, and Windows' metafiles.
#[must_use]
pub fn mime(bytes: &[u8]) -> Option<&'static str> {
    let at = |offset: usize, magic: &[u8]| bytes.get(offset..offset + magic.len()) == Some(magic);
    Some(match () {
        () if at(0, b"\x89PNG\r\n\x1a\n") => "image/png",
        () if at(0, b"\xff\xd8\xff") => "image/jpeg",
        () if at(0, b"GIF87a") || at(0, b"GIF89a") => "image/gif",
        () if at(0, b"II*\0") || at(0, b"MM\0*") => "image/tiff",
        () if at(0, b"RIFF") && at(8, b"WEBP") => "image/webp",
        () if at(0, b"BM") => "image/bmp",
        () if at(4, b"ftypheic") || at(4, b"ftypheix") || at(4, b"ftypmif1") => "image/heic",
        () if svg(bytes) => "image/svg+xml",
        () if at(0, b"\xd7\xcd\xc6\x9a") || at(0, b"\x01\x00\x09\x00\x00\x03") => "image/x-wmf",
        () if at(0, b"\x02\x00\x09\x00\x00\x03") => "image/x-wmf",
        () if at(0, b"\x01\x00\x00\x00") && at(40, b" EMF") => "image/x-emf",
        () => return None,
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

/// A PNG's, a JPEG's or a GIF's own width and height in pixels, read from its header — `None`
/// for anything else, which is then not drawn rather than drawn at a guessed size.
#[must_use]
pub fn pixels(data: &[u8]) -> Option<(u32, u32)> {
    let be32 = |at: usize| Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?));
    let be16 = |at: usize| {
        Some(u32::from(u16::from_be_bytes(
            data.get(at..at + 2)?.try_into().ok()?,
        )))
    };
    let le16 = |at: usize| {
        Some(u32::from(u16::from_le_bytes(
            data.get(at..at + 2)?.try_into().ok()?,
        )))
    };
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some((be32(16)?, be32(20)?));
    }
    if data.starts_with(b"GIF8") {
        return Some((le16(6)?, le16(8)?));
    }
    if data.starts_with(&[0xFF, 0xD8]) {
        // Walk the segments to the first start-of-frame, which carries the size.
        let mut at = 2;
        while at + 9 < data.len() {
            if data[at] != 0xFF {
                return None;
            }
            let marker = data[at + 1];
            let length = be16(at + 2)? as usize;
            if matches!(marker, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF) {
                return Some((be16(at + 7)?, be16(at + 5)?));
            }
            at += 2 + length;
        }
    }
    None
}

/// The size a picture of `data` is given when it is inserted without one: its own pixels at 96
/// to the inch, made to fit `measure` millimetres across with its shape kept. A frame stating no
/// size is a 16 pt square to LibreOffice (`doc/odt-format.md` §5c fact 50).
#[must_use]
pub(crate) fn own_size(data: &[u8], measure: f64) -> Option<(String, String)> {
    const MM_PER_PIXEL: f64 = 25.4 / 96.0;
    let (width, height) = pixels(data)?;
    let (width, height) = (f64::from(width.max(1)), f64::from(height.max(1)));
    let across = (width * MM_PER_PIXEL).min(measure.max(1.0));
    let down = height * across / width;
    Some((
        grind_core::style::mm_length(across),
        grind_core::style::mm_length(down),
    ))
}

/// Insert `data` (of type `mime`) below the caret's block, or in it when it is empty, returning
/// the block it landed in. The caret belongs at offset 1 afterwards, so the next thing typed is a
/// caption.
pub fn insert_below(app: &App, block: usize, mime: &str, data: Vec<u8>) -> Result<usize> {
    let empty = app.input_text(block).is_ok_and(|text| text.is_empty());
    let at = if empty {
        block
    } else {
        app.insert(block + 1, BlockKind::Paragraph, "")?;
        block + 1
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
#[must_use]
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
#[must_use]
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
        assert_eq!(mime(b"\xd7\xcd\xc6\x9a\0\0"), Some("image/x-wmf"));
        let mut emf = vec![1, 0, 0, 0];
        emf.resize(40, 0);
        emf.extend(b" EMF");
        assert_eq!(mime(&emf), Some("image/x-emf"));
        assert_eq!(mime(b"<html>"), None);
        assert_eq!(mime(b"PK\x03\x04"), None, "a document is not a picture");
        assert_eq!(mime(b""), None);
    }

    /// A PNG's header, 400 × 200 px, with a `pHYs` chunk when `phys` gives one.
    fn png(phys: Option<(u32, u8)>) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        bytes.extend(400u32.to_be_bytes());
        bytes.extend(200u32.to_be_bytes());
        bytes.extend([8, 2, 0, 0, 0, 0, 0, 0, 0]);
        if let Some((per_metre, unit)) = phys {
            bytes.extend(b"\0\0\0\x09pHYs");
            bytes.extend(per_metre.to_be_bytes());
            bytes.extend(per_metre.to_be_bytes());
            bytes.extend([unit, 0, 0, 0, 0]);
        }
        bytes.extend(b"\0\0\0\0IDAT");
        bytes
    }

    /// A JPEG's header, 400 × 200 px, with a JFIF segment when `jfif` gives one.
    fn jpeg(jfif: Option<(u8, u16)>) -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xD8];
        if let Some((unit, density)) = jfif {
            bytes.extend([0xFF, 0xE0, 0, 16]);
            bytes.extend(b"JFIF\0\x01\x01");
            bytes.push(unit);
            bytes.extend(density.to_be_bytes());
            bytes.extend(density.to_be_bytes());
            bytes.extend([0, 0]);
        }
        bytes.extend([0xFF, 0xC0, 0, 17, 8, 0, 200, 1, 144, 3, 0, 0, 0, 0, 0, 0]);
        bytes
    }

    #[test]
    fn a_crop_is_four_lengths_with_or_without_commas_and_never_auto() {
        let crop = |edges: [&str; 4]| Some(Crop(edges.map(str::to_owned)));
        assert_eq!(
            Crop::parse("rect(0.3967in 0.95623in 0.56605in 0.92727in)"),
            crop(["0.3967in", "0.95623in", "0.56605in", "0.92727in"])
        );
        assert_eq!(
            Crop::parse("rect(0cm, -1cm,0cm , 2.54cm)"),
            crop(["0cm", "-1cm", "0cm", "2.54cm"])
        );
        assert_eq!(Crop::parse("auto"), None);
        assert_eq!(Crop::parse("rect(auto, 1in, auto, 1in)"), None);
        assert_eq!(
            Crop::parse("rect(0in, 0in, 0in, 0in)"),
            None,
            "it cuts nothing"
        );
        assert_eq!(Crop::parse("rect(1in, 1in, 1in)"), None);
    }

    /// `doc/odt-format.md` §5c fact 64: a picture's own size is its pixels at the density it
    /// states, at 96 to the inch when it states none or only an aspect ratio.
    #[test]
    fn a_pictures_own_size_is_its_pixels_at_its_density() {
        let close = |got: Option<(f64, f64)>, inches: (f64, f64)| {
            let (w, h) = got.unwrap();
            assert!((w - inches.0 * 25.4).abs() < 0.01, "{w}");
            assert!((h - inches.1 * 25.4).abs() < 0.01, "{h}");
        };
        close(own_mm(&png(Some((3937, 1)))), (4.0, 2.0));
        close(own_mm(&png(None)), (400.0 / 96.0, 200.0 / 96.0));
        close(own_mm(&png(Some((1, 0)))), (400.0 / 96.0, 200.0 / 96.0));
        close(own_mm(&jpeg(Some((1, 100)))), (4.0, 2.0));
        close(own_mm(&jpeg(Some((2, 40)))), (400.0 / 101.6, 200.0 / 101.6));
        close(own_mm(&jpeg(Some((0, 1)))), (400.0 / 96.0, 200.0 / 96.0));
        assert_eq!(own_mm(&jpeg(None)), None, "not measured");
        // A placeable WMF two inches by one at 500 units to the inch, and an EMF 76.49 mm wide.
        let mut wmf = b"\xd7\xcd\xc6\x9a\0\0".to_vec();
        for value in [0i16, 0, 1000, 500] {
            wmf.extend(value.to_le_bytes());
        }
        wmf.extend(500u16.to_le_bytes());
        close(own_mm(&wmf), (2.0, 1.0));
        let mut emf = vec![1, 0, 0, 0];
        emf.resize(24, 0);
        for value in [0i32, 0, 7649, 580] {
            emf.extend(value.to_le_bytes());
        }
        emf.extend(b" EMF");
        close(own_mm(&emf), (76.49 / 25.4, 5.8 / 25.4));
        let cut = Crop::parse("rect(0.5in, 1in, 0in, 25.4mm)").unwrap();
        let [top, right, bottom, left] = cut.fractions(&png(Some((3937, 1)))).unwrap();
        assert!((top - 0.25).abs() < 0.001 && (right - 0.25).abs() < 0.001);
        assert!(bottom.abs() < 0.001 && (left - 0.25).abs() < 0.001);
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
