// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! **Saving never makes an existing file worse**, held to every document LibreOffice Writer
//! wrote that is vendored in `tests/data/`, under eight ordinary edits — typing, Enter,
//! Backspace at a block's front, deleting and inserting a block, making one a heading or a list
//! item, and bold.
//!
//! Each save either writes — and then the file reads back as the document (the writer's own
//! read-back check), every byte outside `office:body` is the original's, and a package keeps
//! every entry but the thumbnail — or is refused with `Error::WouldLose`, naming what would have
//! gone. Nothing else is allowed. [`REFUSED`] is the ratchet: empty today, and a refusal that
//! appears is a regression to look at rather than a row to add.

use std::path::PathBuf;

use grind_text::loc;
use grind_text::model::BlockKind;
use grind_text::{App, CharStyle, Form};

const REFUSED: &[(&str, &str)] = &[];

type Edit = (&'static str, fn(&App));

fn caret(app: &App, address: &str) -> grind_text::Caret {
    app.resolve_caret(&loc::parse(address).expect("parses"))
        .expect("resolves")
}

const EDITS: &[Edit] = &[
    ("type", |a| a.insert_text(caret(a, "p1+0"), "x").unwrap()),
    ("enter", |a| a.split_block(caret(a, "p2+3")).unwrap()),
    ("backspace", |a| a.join_block(1).unwrap()),
    ("delete", |a| {
        a.delete(1..2).unwrap();
    }),
    ("insert", |a| {
        a.insert(0, BlockKind::Paragraph, "new first").unwrap()
    }),
    ("heading", |a| {
        a.set_kind(1, BlockKind::Heading { level: 2 }).unwrap()
    }),
    ("list", |a| {
        a.set_kind(2, BlockKind::ListItem { depth: 1 }).unwrap()
    }),
    ("bold", |a| {
        let bold = CharStyle {
            font_weight: Some("bold".into()),
            ..Default::default()
        };
        a.set_char_style(caret(a, "p1+0"), caret(a, "p1+3"), &bold)
            .unwrap();
    }),
];

fn documents() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> =
        std::fs::read_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data"))
            .expect("the fixtures")
            .map(|entry| entry.expect("an entry").path())
            .filter(|path| path.extension().is_some_and(|e| e == "fodt" || e == "odt"))
            .collect();
    files.sort();
    files
}

/// Everything before `office:body` and after it — less the namespace declarations a new
/// element may have added to the root.
fn outside_body(xml: &[u8]) -> (String, String) {
    let text = String::from_utf8_lossy(xml);
    let start = text.find("<office:body").expect("a body");
    let end = text.rfind("</office:body>").expect("a body");
    let mut head = text[..start].to_owned();
    while let Some(at) = head.find(" xmlns:") {
        let after = &head[at + 1..];
        let end = after
            .find('"')
            .and_then(|o| after[o + 1..].find('"').map(|c| o + 1 + c + 1))
            .unwrap_or(after.len());
        head.replace_range(at..at + 1 + end, "");
    }
    (head, text[end..].to_owned())
}

/// The inside of `office:automatic-styles`, or nothing.
fn automatic_styles(head: &str) -> &str {
    let Some(open) = head.find("<office:automatic-styles") else {
        return "";
    };
    let start = open + head[open..].find('>').map_or(0, |p| p + 1);
    let end = head.rfind("</office:automatic-styles>").unwrap_or(start);
    &head[start..end.max(start)]
}

/// `head` with `office:automatic-styles`' inside taken out.
fn without_automatic_styles(head: &str) -> String {
    let inside = automatic_styles(head);
    match inside.is_empty() {
        true => head.to_owned(),
        false => head.replacen(inside, "", 1),
    }
}

#[test]
fn every_edit_to_a_writer_document_saves_or_is_refused() {
    let mut refused = Vec::new();
    for path in documents() {
        let bytes = std::fs::read(&path).expect("reads");
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let form = Form::from_path(&path);
        for (label, edit) in EDITS {
            let app = App::new();
            app.open_bytes(&name, &bytes).expect("opens");
            edit(&app);
            match app.save_bytes(form) {
                Ok(out) => {
                    App::new()
                        .open_bytes(&name, &out)
                        .unwrap_or_else(|e| panic!("{name}, {label}: does not open: {e}"));
                    let (before, after) = match form {
                        Form::Package => {
                            for entry in grind_core::odf::package::entry_names(&bytes) {
                                if entry == "content.xml"
                                    || entry == "META-INF/manifest.xml"
                                    || entry.starts_with("Thumbnails/")
                                {
                                    continue;
                                }
                                let part = |b: &[u8]| grind_core::odf::package::part(b, &entry);
                                assert!(part(&bytes) == part(&out), "{name}, {label}: {entry}");
                            }
                            (
                                grind_core::odf::package::content_xml(&bytes).unwrap(),
                                grind_core::odf::package::content_xml(&out).unwrap(),
                            )
                        }
                        _ => (bytes.clone(), out.clone()),
                    };
                    let (old, new) = (outside_body(&before), outside_body(&after));
                    assert_eq!(
                        without_automatic_styles(&old.0),
                        without_automatic_styles(&new.0),
                        "{name}, {label}: the head changed"
                    );
                    // A style for new formatting may join the automatic styles; nothing goes.
                    let (mine, theirs) = (automatic_styles(&old.0), automatic_styles(&new.0));
                    assert!(
                        theirs.starts_with(mine.trim_end()),
                        "{name}, {label}: the file's own automatic styles changed"
                    );
                    assert_eq!(old.1, new.1, "{name}, {label}: the tail changed");
                }
                Err(grind_core::Error::WouldLose(what)) => {
                    assert!(
                        !what.is_empty(),
                        "{name}, {label}: refused without saying why"
                    );
                    refused.push((name.clone(), label.to_string()));
                }
                Err(other) => panic!("{name}, {label}: neither saved nor refused: {other}"),
            }
        }
    }
    let expected: Vec<(String, String)> = REFUSED
        .iter()
        .map(|(n, l)| (n.to_string(), l.to_string()))
        .collect();
    assert_eq!(refused, expected, "the refusals moved");
}

/// Every element and attribute outside `office:body`, counted — the page layout, the named
/// styles, the fonts, the metadata, the settings.
fn envelope_vocabulary(flat: &[u8]) -> std::collections::BTreeMap<String, usize> {
    use grind_core::odf::envelope;
    let mut whole = envelope::document_vocabulary(flat).expect("reads");
    for (key, count) in envelope::body_vocabulary(flat).expect("reads") {
        if let Some(total) = whole.get_mut(&key) {
            *total = total.saturating_sub(count);
        }
    }
    whole.retain(|_, count| *count > 0);
    whole
}

/// **Saving into the other form is not a way round any of this.** A `.fodt` saved as an `.odt`
/// — or an `.ods` as a `.fods` — used to start from nothing and drop everything the model does
/// not own, with no error. Untouched, the document in the other form is every element and
/// attribute of the original, counted the same; after an edit, nothing outside the body has
/// gone, and an edit a same-form save refuses is refused here too.
#[test]
fn saving_into_the_other_form_carries_everything() {
    use grind_core::odf::{envelope, forms};
    let flat = |bytes: &[u8]| forms::convert(bytes, Form::Flat).expect("converts to flat");
    for path in documents() {
        let bytes = std::fs::read(&path).expect("reads");
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let other = match Form::from_path(&path) {
            Form::Package => Form::Flat,
            _ => Form::Package,
        };
        let original = flat(&bytes);

        let app = App::new();
        app.open_bytes(&name, &bytes).expect("opens");
        let out = app
            .save_bytes(other)
            .unwrap_or_else(|e| panic!("{name}: an untouched save into the other form: {e}"));
        assert_eq!(
            envelope::document_vocabulary(&flat(&out)),
            envelope::document_vocabulary(&original),
            "{name}: not the same document in the other form"
        );

        let before = envelope_vocabulary(&original);
        for (label, edit) in EDITS {
            let app = App::new();
            app.open_bytes(&name, &bytes).expect("opens");
            edit(&app);
            match app.save_bytes(other) {
                Ok(out) => {
                    let after = envelope_vocabulary(&flat(&out));
                    for (key, count) in &before {
                        let left = after.get(key).copied().unwrap_or(0);
                        assert!(
                            left >= *count,
                            "{name}, {label}: {key} went from {count} to {left} in the other form"
                        );
                    }
                }
                Err(grind_core::Error::WouldLose(_)) => assert!(
                    REFUSED.iter().any(|(n, l)| *n == name && *l == *label),
                    "{name}, {label}: refused in the other form only"
                ),
                Err(other) => panic!("{name}, {label}: neither saved nor refused: {other}"),
            }
        }
    }
}

/// A run that is superscript, German and bold — two of which `CharStyle` does not read — and a
/// run that is only bold, as LibreOffice writes them.
const SUPERSCRIPT: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<office:document xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:version="1.4" office:mimetype="application/vnd.oasis.opendocument.text">
 <office:automatic-styles>
  <style:style style:name="T1" style:family="text"><style:text-properties style:text-position="super 58%" fo:language="de" fo:country="DE" fo:font-weight="bold"/></style:style>
  <style:style style:name="T2" style:family="text"><style:text-properties fo:font-weight="bold"/></style:style>
 </office:automatic-styles>
 <office:body>
  <office:text>
   <text:p>E = mc<text:span text:style-name="T1">2</text:span> and <text:span text:style-name="T2">bold</text:span> words.</text:p>
  </office:text>
 </office:body>
</office:document>
"##;

/// The `style:text-properties` of the style a span names, in a flat document.
fn span_properties(xml: &str, text: &str) -> String {
    let span = xml
        .match_indices("<text:span text:style-name=\"")
        .map(|(i, _)| &xml[i..])
        .find(|rest| rest[rest.find('>').unwrap() + 1..].starts_with(text))
        .unwrap_or_else(|| panic!("no span around {text:?} in\n{xml}"));
    let name = span.split('"').nth(1).unwrap();
    let start = xml
        .find(&format!("<style:style style:name=\"{name}\""))
        .unwrap();
    let end = start + xml[start..].find("</style:style>").unwrap();
    xml[start..end].to_owned()
}

/// **Restyling a run keeps everything about its style the model does not read**, the text half
/// of the restyle hole: a superscript's new style is its old one with only what changed
/// rewritten — and a style that says more than the model reads is never borrowed for a run
/// that merely looks the same to it, which used to *add* the superscript to somebody's bold.
#[test]
fn restyling_a_run_keeps_what_the_model_does_not_read() {
    let restyle = |at: &str, end: usize, bold: bool| {
        let app = App::new();
        app.open_bytes("superscript.fodt", SUPERSCRIPT.as_bytes())
            .unwrap();
        let start = caret(&app, at);
        let mut style = CharStyle::default();
        if bold {
            style.font_weight = Some("bold".into());
        }
        app.set_char_style(
            start,
            grind_text::Caret {
                offset: end,
                ..start
            },
            &style,
        )
        .unwrap();
        String::from_utf8(app.save_bytes(Form::Flat).unwrap()).unwrap()
    };

    // Unbold the superscript: it stays superscript and German.
    let out = restyle("p1+6", 7, false);
    let two = span_properties(&out, "2<");
    assert!(two.contains("style:text-position=\"super 58%\""), "{two}");
    assert!(two.contains("fo:language=\"de\""), "{two}");
    assert!(!two.contains("font-weight"), "{two}");

    // Bold "words": the plain bold style, never the superscript one.
    let out = restyle("p1+17", 22, true);
    let words = span_properties(&out, "words");
    assert!(!words.contains("text-position"), "{words}");
    // And the superscript nobody touched keeps the file's own name for its style.
    assert!(
        out.contains("<text:span text:style-name=\"T1\">2</text:span>"),
        "{out}"
    );
}

/// **A byte-order mark does not shift the file under a save** — the text half of the
/// spreadsheet's test of the same name: every range a reader records is measured from the
/// file's first byte, a BOM included.
#[test]
fn a_byte_order_mark_does_not_shift_a_save() {
    for path in documents()
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "fodt"))
    {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let bytes = [b"\xEF\xBB\xBF".as_slice(), &std::fs::read(&path).unwrap()].concat();
        let app = App::new();
        app.open_bytes(&name, &bytes).expect("opens");
        app.insert_text(caret(&app, "p1+0"), "x").unwrap();
        let out = app
            .save_bytes(Form::Flat)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let back = App::new();
        back.open_bytes(&name, &out).expect("reopens");
        assert!(
            back.get_viewport(0..1)
                .get(0)
                .unwrap()
                .text
                .starts_with('x'),
            "{name}"
        );
    }
}
