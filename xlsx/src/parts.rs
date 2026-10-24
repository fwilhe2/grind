// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What hangs off a worksheet in parts of its own — drawings, charts, comments, pivot tables —
//! **counted and never read**: each has no home in the model, or one that would cost a second
//! vocabulary. Found by relationship from the worksheet part, counted once per construct:
//!
//! | relationship | counted as | unit |
//! |---|---|---|
//! | `drawing` | `Dropped::Drawing` | per drawing part |
//! | the drawing's `chart` | `Dropped::Chart` | per chart part |
//! | `comments` | `Dropped::Comment` | per `<comment>` in it |
//! | `pivotTable` | `Dropped::PivotTable` | per table, however many cache parts it took |
//!
//! A comment's `vmlDrawing` is its anchor and is not counted again. An external target is never
//! opened.

use crate::names::{RelType, Seen};
use crate::package::Package;
use crate::report::{Dropped, Report};
use crate::xml::{Handled, Reader, Spreadsheet as _};

pub fn count(package: &mut Package, sheet_part: &str, report: &mut Report, seen: &mut Seen) {
    for rel in package.rels(sheet_part, seen) {
        if rel.external {
            continue;
        }
        match rel.kind {
            RelType::Drawing => {
                // A drawing whose every shape is a form control's DrawingML twin is not a
                // drawing anybody made: the controls are read from VML (`controls.rs`).
                if package
                    .part(&rel.target)
                    .is_some_and(|bytes| only_control_twins(&bytes))
                {
                    continue;
                }
                report.drop_one(Dropped::Drawing);
                let charts = package
                    .rels(&rel.target, seen)
                    .iter()
                    .filter(|r| r.kind == RelType::Chart && !r.external)
                    .count();
                report.drop_many(Dropped::Chart, charts);
            }
            RelType::Comments => {
                let comments = package
                    .part(&rel.target)
                    .map_or(0, |bytes| comments(&bytes));
                report.drop_many(Dropped::Comment, comments);
            }
            RelType::PivotTable => report.drop_one(Dropped::PivotTable),
            _ => {}
        }
    }
}

/// Whether every anchored shape in a DrawingML drawing is the twin of a VML form control —
/// marked by an `a14:compatExt` naming the VML shape (`doc/xlsx-format.md` §4.11). Counted on
/// the bytes: one anchor element per shape, one `compatExt` per twin, and a drawing with no
/// anchors at all is still a drawing.
fn only_control_twins(bytes: &[u8]) -> bool {
    let count = |needle: &[u8]| bytes.windows(needle.len()).filter(|w| *w == needle).count();
    let anchors = count(b"twoCellAnchor") + count(b"oneCellAnchor") + count(b"absoluteAnchor");
    // Each anchor element's name appears twice, in its start tag and its end tag.
    let anchors = anchors / 2;
    anchors > 0 && count(b"compatExt") >= anchors
}

/// How many `<comment>`s a comments part holds. A part that will not parse holds none that
/// anybody could have read.
fn comments(bytes: &[u8]) -> usize {
    let mut reader = Reader::new(bytes);
    let mut count = 0;
    if !matches!(reader.root(), Ok(Some((ref root, _))) if root.is("comments")) {
        return 0;
    }
    let _ = reader.children(|reader, name, _| {
        if !name.is("commentList") {
            return Ok(Handled::No);
        }
        reader.children(|_, name, _| {
            count += usize::from(name.is("comment"));
            Ok(Handled::No)
        })?;
        Ok(Handled::Yes)
    });
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_comment_is_counted_once_however_rich() {
        let xml = format!(
            r#"<comments xmlns="{}"><authors><author>a</author></authors><commentList>
                 <comment ref="A1" authorId="0"><text><t>one</t></text></comment>
                 <comment ref="B2" authorId="0"><text><r><t>two</t></r><r><t>runs</t></r></text></comment>
               </commentList></comments>"#,
            crate::names::MAIN_T
        );
        assert_eq!(comments(xml.as_bytes()), 2);
        assert_eq!(comments(b"<nonsense/>"), 0);
    }
}
