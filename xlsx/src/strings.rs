// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `xl/sharedStrings.xml` — the string table a `t="s"` cell indexes into.
//!
//! Found by relationship, and **optional**: SheetJS writes every string inline by default,
//! and a reader that reaches for this part before asking whether the workbook has one fails
//! on all of them (`values/inline-strings.xlsx`).
//!
//! `count` and `uniqueCount` are claims and are never read (`doc/xlsx-import.md`,
//! "Transitional, and the real world" §3): the table is exactly as long as the `<si>`
//! elements it holds, and `realworld/lying-counts.xlsx` gets both numbers wrong at once.

use crate::xml::{Handled, Reader};

/// One `<si>`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Item {
    /// The runs, flattened. Whitespace is content and is never trimmed.
    pub text: String,
    /// Whether flattening lost something: more than one run, so formatting *within* the cell
    /// that the model has no home for. One run with an `rPr` is not counted — it carries
    /// nothing the flattened string loses that the cell's own style cannot, and the corpus's
    /// `values/rich-text.xlsx` makes exactly that argument with its B2.
    pub rich: bool,
}

/// Read the table. A part that stops parsing partway keeps what it had: an index past the end
/// then reads as an empty string rather than failing the workbook.
pub fn read(bytes: &[u8]) -> Vec<Item> {
    let mut reader = Reader::new(bytes);
    let mut out = Vec::new();
    if !matches!(reader.root(), Ok(Some((ref root, _))) if root.is("sst")) {
        return out;
    }
    let _ = reader.children(|reader, name, _| {
        if !name.is("si") {
            return Ok(Handled::No);
        }
        out.push(item(reader)?);
        Ok(Handled::Yes)
    });
    out
}

/// The children of one `<si>` or `<is>` — the same content model (`CT_Rst`, §18.4.13), so an
/// inline string is read by this function too.
///
/// `<t>` is the plain case and `<r><t>` the rich one. **`<rPh>` is not text**: it is a
/// phonetic reading — furigana over Japanese — and it has a `<t>` of its own, which a reader
/// that concatenated every text node would splice into the middle of the string.
pub fn item(reader: &mut Reader<'_>) -> crate::Result<Item> {
    let mut text = String::new();
    let mut runs = 0usize;
    reader.children(|reader, name, _| {
        if name.is("t") {
            text.push_str(&reader.text()?);
            return Ok(Handled::Yes);
        }
        if name.is("r") {
            runs += 1;
            reader.children(|reader, name, _| {
                if !name.is("t") {
                    return Ok(Handled::No);
                }
                text.push_str(&reader.text()?);
                Ok(Handled::Yes)
            })?;
            return Ok(Handled::Yes);
        }
        Ok(Handled::No)
    })?;
    Ok(Item {
        text,
        rich: runs > 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: &str = crate::names::MAIN_T;

    fn table(body: &str) -> Vec<Item> {
        read(format!(r#"<sst xmlns="{MAIN}" count="999" uniqueCount="1">{body}</sst>"#).as_bytes())
    }

    #[test]
    fn the_table_is_as_long_as_its_items_not_its_claims() {
        let items = table("<si><t>a</t></si><si><t>b</t></si><si><t/></si>");
        let texts: Vec<_> = items.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(texts, ["a", "b", ""]);
    }

    #[test]
    fn runs_flatten_and_only_several_of_them_count_as_a_loss() {
        let items = table(
            r#"<si><r><rPr><b/></rPr><t>all bold</t></r></si>
               <si><r><t xml:space="preserve">a </t></r><r><rPr><i/></rPr><t xml:space="preserve"> b</t></r></si>"#,
        );
        assert_eq!(
            items[0],
            Item {
                text: "all bold".into(),
                rich: false
            }
        );
        assert_eq!(
            items[1],
            Item {
                text: "a  b".into(),
                rich: true
            }
        );
    }

    #[test]
    fn a_phonetic_reading_is_not_part_of_the_text() {
        let items = table(
            r#"<si><t>漢字</t><rPh sb="0" eb="2"><t>かんじ</t></rPh><phoneticPr fontId="1"/></si>"#,
        );
        assert_eq!(items[0].text, "漢字");
    }

    #[test]
    fn no_table_at_all_is_an_empty_one() {
        assert!(read(b"").is_empty());
        assert!(read(b"<nonsense/>").is_empty());
    }
}
