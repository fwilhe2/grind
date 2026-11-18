// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Footnotes at the foot of a page and endnotes after the body (`doc/odt-format.md` §5c facts 67
//! and 69).

use super::*;

/// The endnotes, on pages of their own after the body: on its last page's paper, with no header
/// or footer, one after another from the top margin, a note that does not fit going on to the
/// next page (`doc/odt-format.md` §5c fact 67). With `after` — the body's last page, where its
/// last line ends and where its body does — they start on that page instead, under a rule, as
/// Word sets them (fact 69). Answers the pages added.
pub(super) fn endnote_pages(
    app: &App,
    setter: &Typesetter,
    endnotes: &[grind_text::marginal::Note],
    geometry: PageGeometry,
    after: Option<(&mut Page, f64, f64)>,
) -> Vec<Page> {
    /// Fact 69's distances: from the last line to the rule, the rule's length, from the rule to
    /// the first note, in millimetres.
    const ABOVE: f64 = 2.89;
    const LONG: f64 = 50.8;
    const BELOW: f64 = 1.8;
    if endnotes.is_empty() {
        return Vec::new();
    }
    let look = app.note_look();
    let citations = look.endnote;
    let (left, top) = (pt(geometry.left), pt(geometry.top));
    let width = pt(geometry.width - geometry.left - geometry.right);
    let bottom = pt(geometry.height - geometry.bottom);
    let boxes = Boxes {
        paper: (pt(geometry.width), pt(geometry.height)),
        body: (top, bottom - top),
    };
    let blank = || Page {
        width: pt(geometry.width) as f32,
        height: pt(geometry.height) as f32,
        ops: Vec::new(),
        lines: Vec::new(),
        start: None,
        end: None,
    };
    let mut pages = Vec::new();
    // The page being filled — the body's last, or the latest added — how far down it is, where
    // its room ends, and whether anything is on it yet.
    let (mut on_body, mut y, mut end, mut fresh) = (None, top, bottom, true);
    if let Some((page, foot, body_bottom)) = after {
        let rule_y = foot + pt(ABOVE);
        if look.rule.drawn {
            page.ops.push(Op::Rect {
                x: left as f32,
                y: rule_y as f32,
                width: pt(LONG).min(width) as f32,
                height: pt(look.rule.width) as f32,
                color: Rgb::parse(&look.rule.color).unwrap_or(Rgb::BLACK),
            });
        }
        y = rule_y + pt(look.rule.width) + pt(BELOW);
        end = body_bottom;
        fresh = false;
        on_body = Some(page);
    } else {
        pages.push(blank());
    }
    for note in endnotes {
        let band = note_band(app, setter, note, &citations, width);
        if y + band.height > end && !fresh {
            on_body = None;
            pages.push(blank());
            (y, end) = (top, bottom);
        }
        let ops = match on_body.as_deref_mut() {
            Some(page) => &mut page.ops,
            None => &mut pages.last_mut().expect("there is a page").ops,
        };
        band.draw(ops, (left, y), (1, 1, &[]), &boxes);
        y += band.height;
        fresh = false;
    }
    pages
}

/// A footnote set as its own paragraphs: its citation in front of the first, in the class's
/// note style, and the text after it at the paragraph's indent when the number fits the hang
/// its first line has, straight after it when there is none (`doc/odt-format.md` §5c fact 67).
pub(super) fn note_band<'a>(
    app: &App,
    setter: &'a Typesetter,
    note: &grind_text::marginal::Note,
    citations: &grind_text::marginal::Citations,
    width: f64,
) -> Marginal<'a> {
    use grind_text::marginal::{Look, Part};
    let mut paragraphs = note.paragraphs.clone();
    let hangs = paragraphs.first().is_some_and(|first| {
        app.resolve_style(first.style.as_deref())
            .props
            .text_indent
            .as_deref()
            .and_then(length_mm)
            .is_some_and(|indent| indent < 0.0)
    });
    if let Some(first) = paragraphs.first_mut() {
        let plain = Look {
            names: None,
            props: grind_text::CharStyle::default(),
        };
        let mut looks: Vec<Look> = vec![Look {
            names: citations.note.clone(),
            props: grind_text::CharStyle::default(),
        }];
        let mut parts = vec![Part::Text(note.citation.clone())];
        if hangs {
            parts.push(Part::Text("\t".into()));
            looks.push(plain.clone());
        }
        // Looks run alongside parts, and a part past the last has none.
        let count = first.parts.len();
        looks.extend((0..count).map(|at| first.looks.get(at).cloned().unwrap_or(plain.clone())));
        parts.append(&mut first.parts);
        first.parts = parts;
        first.looks = looks;
    }
    let marginal = grind_text::marginal::Marginal {
        paragraphs,
        min_height: 0.0,
        spacing: 0.0,
    };
    let mut band = Marginal::lay_out(app, setter, &marginal, width);
    if hangs && let Some(first) = band.paragraphs.first_mut() {
        first.tabs.stops.insert(
            0,
            grind_core::layout::TabStop {
                position: 0.0,
                align: grind_core::layout::TabAlign::Left,
                leader: None,
            },
        );
    }
    band
}
