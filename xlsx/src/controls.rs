// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Form-control checkboxes, read out of a worksheet's legacy VML drawing
//! (`doc/xlsx-format.md` §4.11) onto the model's own `Checkbox`.
//!
//! Excel spells every control three times — in the worksheet's `<controls>`, in a DrawingML
//! part, and in VML — and the first two sit inside an `mc:Choice` this filter does not satisfy
//! (`mce.rs`), with no fallback. The VML is the copy outside it, and it says everything a
//! checkbox linked to a cell is: where it is anchored, the cell it is linked to, whether it is
//! ticked, and its caption.
//!
//! What a VML drawing holds that is not a checkbox — a comment's note is the common one — is
//! not read here; a comment is counted by `parts.rs`. Any other form control (a button, a
//! list, a spinner) is counted as `Dropped::FormControl`.

use grind_sheet::{Checkbox, Link, Pos, Sheet};

use crate::address;
use crate::names::{Ns, RelType, Seen};
use crate::package::Package;
use crate::report::{Dropped, Report};
use crate::xml::{Handled, Reader};

/// Millimetres in one VML pixel: an anchor's offsets are pixels at 96 to the inch.
const MM_PER_PX: f64 = 25.4 / 96.0;

/// Read every checkbox of the worksheet `sheet_part` into `sheet`.
pub fn read(
    package: &mut Package,
    sheet_part: &str,
    sheet: &mut Sheet,
    report: &mut Report,
    seen: &mut Seen,
) {
    for rel in package.rels(sheet_part, seen) {
        if rel.external || rel.kind != RelType::VmlDrawing {
            continue;
        }
        let Some(bytes) = package.part(&rel.target) else {
            continue;
        };
        for found in shapes(&bytes) {
            match found {
                Found::Checkbox(control) => place(sheet, control, report),
                Found::Other => report.drop_one(Dropped::FormControl),
            }
        }
    }
}

/// One checkbox as VML spells it.
#[derive(Debug, Default, PartialEq)]
struct Control {
    /// `x:Anchor`: from-column, offset, from-row, offset, to-column, offset, to-row, offset —
    /// columns and rows 0-based, offsets in pixels.
    anchor: [u32; 8],
    /// `x:FmlaLink`, verbatim: `$E5`, `Data!$E$5`.
    link: Option<String>,
    /// `x:Checked` is there and not `0`.
    checked: bool,
    /// The caption, the text inside the shape's text box.
    label: Option<String>,
}

#[derive(Debug, PartialEq)]
enum Found {
    Checkbox(Control),
    /// A form control of another kind.
    Other,
}

/// Every form control in a VML drawing. A comment's note is not a form control and is left out.
fn shapes(bytes: &[u8]) -> Vec<Found> {
    let mut out = Vec::new();
    let mut reader = Reader::new(bytes);
    if reader.root().ok().flatten().is_none() {
        return out;
    }
    let _ = reader.children(|reader, name, _| {
        if !(name.ns == Ns::Vml && name.local == "shape") {
            return Ok(Handled::No);
        }
        let mut control = Control::default();
        let mut kind: Option<String> = None;
        reader.children(|reader, name, attrs| {
            if name.ns == Ns::Vml && name.local == "textbox" {
                let text = reader.text()?;
                let text = text.trim();
                control.label = (!text.is_empty()).then(|| text.to_owned());
                return Ok(Handled::Yes);
            }
            if !(name.ns == Ns::VmlExcel && name.local == "ClientData") {
                return Ok(Handled::No);
            }
            kind = attrs.plain("ObjectType").map(str::to_owned);
            reader.children(|reader, name, _| {
                if name.ns != Ns::VmlExcel {
                    return Ok(Handled::No);
                }
                match name.local.as_str() {
                    "Anchor" => {
                        let numbers: Vec<u32> = reader
                            .text()?
                            .split(',')
                            .filter_map(|n| n.trim().parse().ok())
                            .collect();
                        if let Ok(anchor) = <[u32; 8]>::try_from(numbers) {
                            control.anchor = anchor;
                        }
                    }
                    "FmlaLink" => {
                        let link = reader.text()?.trim().to_owned();
                        control.link = (!link.is_empty()).then_some(link);
                    }
                    "Checked" => {
                        // An empty `<x:Checked/>` is ticked too: its presence is the flag.
                        control.checked = reader.text()?.trim() != "0";
                    }
                    _ => return Ok(Handled::No),
                }
                Ok(Handled::Yes)
            })?;
            Ok(Handled::Yes)
        })?;
        match kind.as_deref() {
            Some("Checkbox") => out.push(Found::Checkbox(control)),
            // A comment's anchor, not a control.
            Some("Note") | None => {}
            Some(_) => out.push(Found::Other),
        }
        Ok(Handled::Yes)
    });
    out
}

/// Put one checkbox into the cell it is drawn in: the anchor's column, and the row its middle
/// falls in — a control placed a little too high still belongs to the row it is drawn over
/// (`Sheet::row_holding`). Two in one cell is a file that drew them on top of each other: the
/// first stays and the second is counted.
fn place(sheet: &mut Sheet, control: Control, report: &mut Report) {
    let [col, _, row, dy, _, _, row2, dy2] = control.anchor;
    let height = |r: u32| {
        sheet
            .row_height(r)
            .and_then(grind_sheet::style::length_mm)
            .unwrap_or(grind_sheet::model::DEFAULT_ROW_MM)
    };
    let top = f64::from(dy) * MM_PER_PX;
    let bottom = (row..row2.max(row)).map(height).sum::<f64>() + f64::from(dy2) * MM_PER_PX;
    let at = Pos::new(sheet.row_holding(row, (top + bottom) / 2.0), col);
    let link = control.link.as_deref().and_then(|l| link(l, &sheet.name));
    if sheet.checkbox(at).is_some() || (control.link.is_some() && link.is_none()) {
        report.drop_one(Dropped::FormControl);
        return;
    }
    sheet.set_checkbox(
        at,
        Some(Checkbox {
            link,
            checked: control.checked,
            name: None,
            label: control.label,
        }),
    );
}

/// `$E5`, `E5`, `Data!$E$5` or `'Q3 Actuals'!E5` as a link — the sheet folded away when it is
/// the checkbox's own.
fn link(formula: &str, own: &str) -> Option<Link> {
    let (sheet, cell) = match formula.rsplit_once('!') {
        Some((sheet, cell)) => {
            let sheet = match sheet.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
                Some(quoted) => quoted.replace("''", "'"),
                None => sheet.to_owned(),
            };
            (Some(sheet).filter(|s| s != own), cell)
        }
        None => (None, formula),
    };
    let pos = address::cell(&cell.replace('$', ""))?;
    Some(Link { sheet, pos })
}

#[cfg(test)]
mod tests {
    use super::*;

    const VML: &str = r##"<xml xmlns:v="urn:schemas-microsoft-com:vml"
 xmlns:o="urn:schemas-microsoft-com:office:office"
 xmlns:x="urn:schemas-microsoft-com:office:excel">
 <v:shape id="_x0000_s1025" type="#_x0000_t201" style='position:absolute'>
  <v:textbox style='mso-direction-alt:auto'><div style='text-align:left'></div></v:textbox>
  <x:ClientData ObjectType="Checkbox">
   <x:Anchor>
    1, 0, 4, 0, 2, 0, 4, 28</x:Anchor>
   <x:FmlaLink>$E5</x:FmlaLink>
   <x:NoThreeD/>
  </x:ClientData>
 </v:shape>
 <v:shape id="_x0000_s1026"><v:textbox><div>Done</div></v:textbox>
  <x:ClientData ObjectType="Checkbox"><x:Anchor>1, 0, 6, 0, 2, 0, 6, 20</x:Anchor><x:Checked>1</x:Checked></x:ClientData>
 </v:shape>
 <v:shape id="_x0000_s1027"><x:ClientData ObjectType="Note"><x:Row>0</x:Row></x:ClientData></v:shape>
 <v:shape id="_x0000_s1028"><x:ClientData ObjectType="Button"/></v:shape>
</xml>"##;

    #[test]
    fn checkboxes_are_read_and_a_note_is_not_a_control() {
        let found = shapes(VML.as_bytes());
        assert_eq!(found.len(), 3, "{found:?}");
        assert_eq!(
            found[0],
            Found::Checkbox(Control {
                anchor: [1, 0, 4, 0, 2, 0, 4, 28],
                link: Some("$E5".into()),
                checked: false,
                label: None,
            })
        );
        assert_eq!(
            found[1],
            Found::Checkbox(Control {
                anchor: [1, 0, 6, 0, 2, 0, 6, 20],
                link: None,
                checked: true,
                label: Some("Done".into()),
            })
        );
        assert_eq!(found[2], Found::Other);
    }

    #[test]
    fn a_link_is_a_cell_with_its_sheet_folded_away_when_it_is_its_own() {
        assert_eq!(
            link("$E5", "Checkliste"),
            Some(Link {
                sheet: None,
                pos: Pos::new(4, 4)
            })
        );
        assert_eq!(link("Checkliste!$E$5", "Checkliste").unwrap().sheet, None);
        assert_eq!(
            link("'Q3 Actuals'!B2", "Checkliste")
                .unwrap()
                .sheet
                .as_deref(),
            Some("Q3 Actuals")
        );
        assert_eq!(link("nonsense", "x"), None);
    }

    /// A control anchored in one row with an offset carrying it into the next is the next
    /// row's — measured on a real checklist, where one box sat 48 pixels into its row.
    #[test]
    fn a_control_belongs_to_the_row_its_middle_is_in() {
        let mut sheet = Sheet::new("Checkliste");
        for row in 34..37 {
            sheet.set_row_height(row, Some("0.794cm".into()));
        }
        let mut report = Report::default();
        let control = |dy: u32, row2: u32, link: &str| Control {
            anchor: [1, 0, 34, dy, 2, 0, row2, 0],
            link: Some(link.into()),
            ..Control::default()
        };
        place(&mut sheet, control(0, 35, "$E35"), &mut report);
        place(&mut sheet, control(48, 36, "$E36"), &mut report);
        let at: Vec<Pos> = sheet.checkboxes().map(|(pos, _)| pos).collect();
        assert_eq!(at, [Pos::new(34, 1), Pos::new(35, 1)]);
        assert!(report.dropped.is_empty());
    }
}
