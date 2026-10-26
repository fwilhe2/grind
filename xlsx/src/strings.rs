// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `xl/sharedStrings.xml` — the table a `t="s"` cell indexes into. Found by relationship, and
//! **optional**: SheetJS writes every string inline (`values/inline-strings.xlsx`). `count` and
//! `uniqueCount` are claims and never read; `realworld/lying-counts.xlsx` gets both wrong.

use crate::xml::{Handled, Reader, Spreadsheet as _};

/// One `<si>`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Item {
    /// The runs, flattened. Whitespace is content and is never trimmed.
    pub text: String,
    /// Whether flattening lost something: more than one run, so formatting within the cell. One
    /// run with an `rPr` loses nothing the cell's own style cannot carry (`values/rich-text.xlsx`
    /// B2).
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

/// The children of one `<si>` or `<is>` — the same content model (`CT_Rst`, §18.4.13). **`<rPh>`
/// is not text**: a phonetic reading (furigana) with a `<t>` of its own, which concatenating every
/// text node would splice into the string.
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
        text: unescape(&text),
        rich: runs > 1,
    })
}

/// `ST_Xstring`'s `_xHHHH_` escapes decoded, as the oracle does (`doc/xlsx-format.md` §2.6): a
/// UTF-16 code unit each, pairs joined, `_x0000_` dropped, CR LF one line break, and anything
/// malformed left as written.
pub fn unescape(text: &str) -> String {
    if !text.contains("_x") {
        return text.to_owned();
    }
    let bytes = text.as_bytes();
    let unit = |at: usize| -> Option<u16> {
        let candidate = bytes.get(at..at + 7)?;
        if !(candidate.starts_with(b"_x") && candidate[6] == b'_') {
            return None;
        }
        let hex = std::str::from_utf8(&candidate[2..6]).ok()?;
        hex.bytes()
            .all(|b| b.is_ascii_hexdigit())
            .then(|| u16::from_str_radix(hex, 16).ok())
            .flatten()
    };
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while at < text.len() {
        if let Some(first) = unit(at) {
            let (decoded, used) = match (first, unit(at + 7)) {
                (0xD800..=0xDBFF, Some(second @ 0xDC00..=0xDFFF)) => (
                    char::decode_utf16([first, second])
                        .next()
                        .and_then(Result::ok),
                    14,
                ),
                _ => (char::from_u32(u32::from(first)), 7),
            };
            if let Some(c) = decoded {
                if c != '\0' && !(c == '\r' && bytes.get(at + used) == Some(&b'\n')) {
                    out.push(c);
                }
                at += used;
                continue;
            }
        }
        let c = text[at..].chars().next().unwrap();
        out.push(c);
        at += c.len_utf8();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: &str = crate::names::MAIN_T;

    fn table(body: &str) -> Vec<Item> {
        read(format!(r#"<sst xmlns="{MAIN}" count="999" uniqueCount="1">{body}</sst>"#).as_bytes())
    }

    #[test]
    fn escapes_decode_the_way_the_oracle_reads_them() {
        for (written, read) in [
            ("a_x0041_b", "aAb"),
            ("e_x00E9_e", "eée"),
            ("p_x000d_q", "p\rq"),
            ("s_xD83D__xDE00_s", "s😀s"),
            ("_x005F_x0041_", "_x0041_"),
            ("_x005f_x0041_", "_x0041_"),
            ("x_x000D_\ny", "x\ny"),
            ("lf_x000A_lf", "lf\nlf"),
            ("nul_x0000_end", "nulend"),
            ("short_x41_", "short_x41_"),
            ("bad_xZZZZ_", "bad_xZZZZ_"),
            ("lone_xD83D_", "lone_xD83D_"),
            ("_x", "_x"),
        ] {
            assert_eq!(unescape(written), read, "{written}");
        }
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
