// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `xl/styles.xml` — through X3, the **number formats**: which cell formats are dates, and
//! what each one displays.
//!
//! A cell's `s="12"` indexes `<cellXfs>`, whose twelfth `<xf>` names a `numFmtId`, which is
//! either one of ECMA-376's built-ins or a code in `<numFmts>`. Styles are read **before**
//! sheets for exactly this reason — the same order `odf/read.rs` reads `styles.xml` before
//! `content.xml` — because a serial cannot be corrected, or given its date kind, until its
//! format is known.
//!
//! Each `<xf>` is translated **once** ([`numfmt::of_code`] / [`numfmt::of_builtin`]) and every
//! cell that names it clones the result, rather than each cell parsing the same code again.
//!
//! Fonts, fills, borders and alignment are X4's and are skipped whole here by the walker's
//! ordinary tolerance; nothing in this file has to know they exist.

use std::collections::BTreeMap;

use grind_sheet::model::NumberKind;
use grind_sheet::numfmt::Format;

use crate::numfmt::{self, Translation, Unspellable};
use crate::xml::{Handled, Reader};

/// What the styles part says, as far as X3 asks.
#[derive(Debug, Default)]
pub struct Styles {
    /// Per `<cellXfs>` entry, in order: what its number format translated to.
    xfs: Vec<Translation>,
}

impl Styles {
    /// The kind a cell with `s="index"` holds — `None` for a plain number, and for an index
    /// the part does not have. An out-of-range `s` is a claim about a style nobody wrote, and
    /// reading it as "no format" is what every consumer does.
    pub fn kind(&self, index: usize) -> Option<NumberKind> {
        self.xfs.get(index).and_then(|xf| xf.kind)
    }

    /// The format a cell with `s="index"` displays through, where one could be spelled.
    pub fn format(&self, index: usize) -> Option<&Format> {
        self.xfs.get(index).and_then(|xf| xf.format.as_ref())
    }

    /// What that cell's format lost on the way — empty where nothing did.
    pub fn lost(&self, index: usize) -> &[Unspellable] {
        static NONE: &[Unspellable] = &[];
        self.xfs.get(index).map_or(NONE, |xf| &xf.lost)
    }
}

/// Read the styles part. Never fails: a styles part that will not parse leaves every cell a
/// plain number, which loses date kinds and nothing else — and losing one part is not a reason
/// to refuse a workbook (`package.rs`'s rule).
pub fn read(bytes: &[u8]) -> Styles {
    let mut reader = Reader::new(bytes);
    // A code for an id, as the file spells it. Collected first and resolved after: ECMA-376
    // puts `<numFmts>` before `<cellXfs>`, and a reader that relied on that order would be
    // relying on a producer doing what the schema says.
    let mut codes: BTreeMap<u32, String> = BTreeMap::new();
    let mut xfs: Vec<Option<u32>> = Vec::new();

    if !matches!(reader.root(), Ok(Some((ref root, _))) if root.is("styleSheet")) {
        return Styles::default();
    }
    let _ = reader.children(|reader, name, _| {
        if name.is("numFmts") {
            reader.children(|_, name, attrs| {
                if !name.is("numFmt") {
                    return Ok(Handled::No);
                }
                if let (Some(id), Some(code)) = (
                    attrs.plain("numFmtId").and_then(|id| id.parse().ok()),
                    attrs.plain("formatCode"),
                ) {
                    codes.insert(id, code.to_owned());
                }
                Ok(Handled::Yes)
            })?;
            return Ok(Handled::Yes);
        }
        if name.is("cellXfs") {
            reader.children(|_, name, attrs| {
                if !name.is("xf") {
                    return Ok(Handled::No);
                }
                // `applyNumberFormat` is deliberately not consulted. On a *cell* format it says
                // whether this xf overrides its parent cell style, and the id it carries is the
                // one in effect either way (§18.8.45). `doc/xlsx-format.md` §3.3 marks this
                // `SPEC`: the corpus has not yet been asked whether any producer disagrees.
                xfs.push(attrs.plain("numFmtId").and_then(|id| id.parse().ok()));
                Ok(Handled::Yes)
            })?;
            return Ok(Handled::Yes);
        }
        Ok(Handled::No)
    });

    let xfs = xfs
        .into_iter()
        .map(|id| {
            let Some(id) = id else {
                return Translation::default();
            };
            // A file's own code wins over the built-in table, including for an id below 164:
            // the spec reserves those, and producers redefine them anyway.
            match codes.get(&id) {
                Some(code) => numfmt::of_code(code),
                None => numfmt::of_builtin(id),
            }
        })
        .collect();
    Styles { xfs }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: &str = crate::names::MAIN_T;

    #[test]
    fn a_cell_format_is_a_date_through_its_number_format() {
        let xml = format!(
            r#"<styleSheet xmlns="{MAIN}">
                 <cellXfs count="4">
                   <xf numFmtId="0"/>
                   <xf numFmtId="14"/>
                   <xf numFmtId="164"/>
                   <xf numFmtId="165" applyNumberFormat="0"/>
                 </cellXfs>
                 <numFmts>
                   <numFmt numFmtId="164" formatCode="hh:mm:ss"/>
                   <numFmt numFmtId="165" formatCode="0.00"/>
                 </numFmts>
               </styleSheet>"#
        );
        let styles = read(xml.as_bytes());
        assert_eq!(styles.kind(0), None);
        assert_eq!(styles.kind(1), Some(NumberKind::Date));
        // `numFmts` after `cellXfs` — against the schema's order, and still read.
        assert_eq!(styles.kind(2), Some(NumberKind::Time));
        assert_eq!(styles.kind(3), None);
        assert_eq!(styles.kind(99), None, "an index past the table");
    }

    #[test]
    fn a_file_may_redefine_a_built_in_id() {
        let xml = format!(
            r#"<styleSheet xmlns="{MAIN}">
                 <numFmts><numFmt numFmtId="14" formatCode="0.00"/></numFmts>
                 <cellXfs><xf numFmtId="14"/></cellXfs>
               </styleSheet>"#
        );
        assert_eq!(read(xml.as_bytes()).kind(0), None);
    }

    #[test]
    fn a_broken_part_is_no_formats_rather_than_no_workbook() {
        assert_eq!(read(b"not xml at all <<<").kind(0), None);
        assert_eq!(read(b"").kind(0), None);
    }
}
