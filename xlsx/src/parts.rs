// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What hangs off a worksheet in parts of its own — drawings, charts, comments, pivot tables —
//! **counted and never read** (X5).
//!
//! Each is a construct the model has no home for, or one whose home would cost a second
//! vocabulary: a chart is DrawingML, a comment is a part plus an author table plus a VML anchor
//! from 2007. Recognising them costs a relationship type and buys an honest report
//! (`doc/xlsx-import.md` Part II §6). They are found **by relationship from the worksheet
//! part**, never by path, and each is counted once per construct:
//!
//! | relationship | counted as | unit |
//! |---|---|---|
//! | `drawing` | `Dropped::Drawing` | per drawing part |
//! | the drawing's `chart` | `Dropped::Chart` | per chart part |
//! | `comments` | `Dropped::Comment` | per `<comment>` in it |
//! | `pivotTable` | `Dropped::PivotTable` | per table, however many cache parts it took |
//!
//! A comment's `vmlDrawing` is its anchor, not a drawing, and is not counted a second time.
//! An external target is never opened, whatever its type.

use crate::names::{RelType, Seen};
use crate::package::Package;
use crate::report::{Dropped, Report};
use crate::xml::{Handled, Reader};

pub fn count(package: &mut Package, sheet_part: &str, report: &mut Report, seen: &mut Seen) {
    for rel in package.rels(sheet_part, seen) {
        if rel.external {
            continue;
        }
        match rel.kind {
            RelType::Drawing => {
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
