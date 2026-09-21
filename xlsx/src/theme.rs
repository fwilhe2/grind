// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `xl/theme/theme1.xml` — the colour scheme a `<color theme="4"/>` indexes into.
//!
//! A theme is DrawingML, not SpreadsheetML: `<a:theme><a:themeElements><a:clrScheme>` holds
//! twelve named slots, each an `<a:srgbClr val="4472C4"/>` or an `<a:sysClr val="window"
//! lastClr="FFFFFF"/>` — a system colour together with the value it had when the file was
//! saved, which is the only value a converter can use.
//!
//! **The slot order is the trap.** The scheme lists `dk1, lt1, dk2, lt2, accent1…accent6,
//! hlink, folHlink`, and a `theme` attribute counts `lt1, dk1, lt2, dk2, …`: the light and the
//! dark of each of the first two pairs are swapped. A reader that indexes by document order
//! gets white and black backwards and every accent exactly right, which is the worst way to be
//! wrong. Measured against the oracle — theme 0 is `#ffffff` and theme 1 `#000000` in
//! `styles/colors.xlsx` — and recorded in `doc/xlsx-format.md` §4.2. Only the colours are read;
//! the font scheme (`+mn-lt`, `scheme="minor"`) names a *family*, and the model carries none.

use crate::color;
use crate::names::Ns;
use crate::xml::{Handled, Reader};

/// The scheme's slots in **`theme` attribute order**.
const ORDER: [&str; 12] = [
    "lt1", "dk1", "lt2", "dk2", "accent1", "accent2", "accent3", "accent4", "accent5", "accent6",
    "hlink", "folHlink",
];

/// Read the colour scheme, in `theme` attribute order. Empty when the part is not a theme this
/// build can read — including a Strict one, whose DrawingML namespace is unmeasured — so every
/// theme colour then resolves to [`color::Resolved::Unknown`] and is counted rather than
/// guessed. A slot the scheme leaves out ends the table there, for the same reason: the
/// indices after it would otherwise shift onto the wrong colours.
pub fn read(bytes: &[u8]) -> Vec<[u8; 3]> {
    let mut slots: [Option<[u8; 3]>; 12] = [None; 12];
    let mut reader = Reader::new(bytes);
    let is = |name: &crate::xml::Name, local: &str| name.ns == Ns::Drawing && name.local == local;
    if !matches!(reader.root(), Ok(Some((ref root, _))) if is(root, "theme")) {
        return Vec::new();
    }
    let _ = reader.children(|reader, name, _| {
        if !is(name, "themeElements") {
            return Ok(Handled::No);
        }
        reader.children(|reader, name, _| {
            if !is(name, "clrScheme") {
                return Ok(Handled::No);
            }
            reader.children(|reader, name, _| {
                if name.ns != Ns::Drawing {
                    return Ok(Handled::No);
                }
                let Some(slot) = ORDER.iter().position(|s| *s == name.local) else {
                    return Ok(Handled::No);
                };
                reader.children(|_, name, attrs| {
                    let value = match (name.ns, name.local.as_str()) {
                        (Ns::Drawing, "srgbClr") => attrs.plain("val"),
                        (Ns::Drawing, "sysClr") => attrs.plain("lastClr"),
                        _ => return Ok(Handled::No),
                    };
                    slots[slot] = value.and_then(color::argb);
                    Ok(Handled::Yes)
                })?;
                Ok(Handled::Yes)
            })?;
            Ok(Handled::Yes)
        })?;
        Ok(Handled::Yes)
    });
    slots.iter().map_while(|slot| *slot).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = crate::names::DRAWING_T;

    fn scheme(body: &str) -> String {
        format!(
            r#"<a:theme xmlns:a="{A}" name="Office"><a:themeElements><a:clrScheme name="Office">{body}</a:clrScheme></a:themeElements></a:theme>"#
        )
    }

    #[test]
    fn light_and_dark_swap_places_and_the_accents_do_not() {
        let xml = scheme(
            r#"<a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1>
               <a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1>
               <a:dk2><a:srgbClr val="44546A"/></a:dk2>
               <a:lt2><a:srgbClr val="E7E6E6"/></a:lt2>
               <a:accent1><a:srgbClr val="4472C4"/></a:accent1>
               <a:accent2><a:srgbClr val="ED7D31"/></a:accent2>
               <a:accent3><a:srgbClr val="A5A5A5"/></a:accent3>
               <a:accent4><a:srgbClr val="FFC000"/></a:accent4>
               <a:accent5><a:srgbClr val="5B9BD5"/></a:accent5>
               <a:accent6><a:srgbClr val="70AD47"/></a:accent6>
               <a:hlink><a:srgbClr val="0563C1"/></a:hlink>
               <a:folHlink><a:srgbClr val="954F72"/></a:folHlink>"#,
        );
        let theme = read(xml.as_bytes());
        assert_eq!(theme.len(), 12);
        assert_eq!(theme[0], [0xFF, 0xFF, 0xFF], "theme 0 is lt1");
        assert_eq!(theme[1], [0, 0, 0], "theme 1 is dk1");
        assert_eq!(theme[2], [0xE7, 0xE6, 0xE6], "theme 2 is lt2");
        assert_eq!(theme[3], [0x44, 0x54, 0x6A], "theme 3 is dk2");
        assert_eq!(theme[4], [0x44, 0x72, 0xC4], "theme 4 is accent1");
        assert_eq!(theme[11], [0x95, 0x4F, 0x72]);
    }

    #[test]
    fn a_missing_slot_ends_the_table_rather_than_shifting_it() {
        // No `lt2`, which is theme 2: the accents after it must not slide down into its place.
        let xml = scheme(
            r#"<a:dk1><a:srgbClr val="000000"/></a:dk1>
               <a:lt1><a:srgbClr val="FFFFFF"/></a:lt1>
               <a:dk2><a:srgbClr val="44546A"/></a:dk2>
               <a:accent1><a:srgbClr val="4472C4"/></a:accent1>"#,
        );
        assert_eq!(read(xml.as_bytes()).len(), 2);
    }

    #[test]
    fn not_a_theme_is_no_colours() {
        assert!(read(b"<nonsense/>").is_empty());
        assert!(read(b"").is_empty());
        // The right shape in a namespace this build has not measured.
        let strict = scheme("").replace(A, "http://example.invalid/drawingml");
        assert!(read(strict.as_bytes()).is_empty());
    }
}
