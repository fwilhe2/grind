// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `xl/workbook.xml` — the sheet list, their order, and the date system.
//!
//! Reached by relationship from `_rels/.rels`, never by its conventional path. What X0 takes
//! from it is the document's skeleton: one [`grind_sheet::model::Sheet`] per `<sheet>`, in
//! document order, and the epoch every serial date will be counted from; X1 adds where the
//! string table, the styles and the theme are; X5 the defined names, workbook protection, and
//! the sheet names ODF cannot keep.
//!
//! **A sheet name is renamed, deterministically, when LibreOffice would not keep it.** Excel
//! allows `[`, `]` and a leading apostrophe; ODF's schema says only `string`, but LibreOffice
//! refuses `[ ] * ? : / \` and an apostrophe at either end, and the pinned oracle drops such a
//! sheet with every cell on it (`doc/xlsx-format.md` §2.3). This model can address one —
//! `['Has/Slash'.A1]` evaluates — so the rename is not for grind's sake; it is so that the
//! document this import writes survives the round trip every feature is held to (rule 6). Each
//! offending character becomes `_`, which is the corpus's own suggestion, and a clash with an
//! existing name gets `unique_name`'s suffix.

use std::collections::BTreeSet;

use grind_sheet::formula::parse::Expr;
use grind_sheet::formula::{date, lex, rename};
use grind_sheet::model::{Document, Sheet};

use crate::formula::{self, Refusal};
use crate::names::{RelType, Seen};
use crate::package::Package;
use crate::report::{Dropped, Report};
use crate::xml::{Handled, Reader};
use crate::{Error, Result};

/// One `<sheet>` of `<sheets>`, before its part has been read.
#[derive(Clone, Debug)]
pub struct SheetEntry {
    pub name: String,
    /// The relationship id naming this sheet's part. `None` for a `<sheet>` with no `r:id`,
    /// which is a sheet whose content cannot be found — it still gets a name and an empty
    /// grid, because a missing part is not a reason to lose the tab.
    pub rel_id: Option<String>,
    /// `state="hidden"` or `"veryHidden"`.
    pub hidden: bool,
    /// The part this sheet's cells live in, once the relationship has been resolved.
    pub part: Option<String>,
}

/// One `<definedName>`.
#[derive(Clone, Debug)]
pub struct DefinedName {
    pub name: String,
    /// The expression, in Excel's syntax.
    pub text: String,
    /// `localSheetId` — the index of the only sheet the name is visible from.
    pub local: Option<usize>,
}

/// What `xl/workbook.xml` said.
#[derive(Debug, Default)]
pub struct Workbook {
    pub sheets: Vec<SheetEntry>,
    /// `workbookPr/@date1904`.
    pub date_1904: bool,
    /// `xl/sharedStrings.xml`, when the workbook has one — found by relationship, and optional:
    /// a workbook whose strings are all inline has none.
    pub strings_part: Option<String>,
    /// `xl/styles.xml`, likewise.
    pub styles_part: Option<String>,
    /// `xl/theme/theme1.xml`, which a `<color theme="4"/>` indexes into. Optional as well: a
    /// workbook with no theme has theme colours nothing can resolve, and the report counts them.
    pub theme_part: Option<String>,
    /// `<definedNames>`, in document order.
    pub names: Vec<DefinedName>,
    /// `<workbookProtection>` — a lock on the sheet structure, never on the data.
    pub protected: bool,
    /// The `conformance="strict"` attribute, when the workbook carries one. Corroborates the
    /// namespace evidence rather than replacing it — a file can be Strict without saying so.
    pub declares_strict: bool,
}

/// ODF's epoch for a 1904-system workbook, as days from 1970-01-01.
///
/// The 1900 system needs no constant: ODF's own default epoch is 1899-12-30, and the
/// correction that makes Excel's serials agree with it is a *value* question
/// (`dates::correct`), not a document-level one.
fn null_date_1904() -> i64 {
    date::days_from_civil(1904, 1, 1)
}

/// Find the workbook part, following `_rels/.rels`.
///
/// Falls back to the conventional path only when the package declares no relationship at
/// all — a file that broken is past the spec, and half of it still being readable is better
/// than none. The fallback is *last*, so a package that puts its workbook somewhere else and
/// says so is followed rather than second-guessed.
pub fn find_part(package: &mut Package, seen: &mut Seen) -> Result<String> {
    if let Some(rel) = package.rel_of("", RelType::OfficeDocument, seen) {
        return Ok(rel.target);
    }
    package
        .part("xl/workbook.xml")
        .map(|_| "xl/workbook.xml".to_owned())
        .ok_or(Error::NotSpreadsheet)
}

pub fn read(bytes: &[u8], report: &mut Report) -> Result<Workbook> {
    let mut reader = Reader::new(bytes);
    let mut out = Workbook::default();

    let Some((root, attrs)) = reader.root()? else {
        return Err(Error::Xml("workbook part has no elements".into()));
    };
    if !root.is("workbook") {
        return Err(Error::Xml(format!(
            "workbook part is a <{}>, not a <workbook>",
            root.local
        )));
    }
    out.declares_strict = matches!(attrs.plain("conformance"), Some("strict"));

    reader.children(|reader, name, attrs| {
        if name.is("workbookPr") {
            out.date_1904 = attrs.flag("date1904");
            return Ok(Handled::Yes);
        }
        if name.is("workbookProtection") {
            // Every lock defaults to off (§18.2.29), and LibreOffice's own `.xlsx` writes an
            // empty `<workbookProtection/>` that locks nothing — which is not protection.
            out.protected |= ["lockStructure", "lockWindows", "lockRevision"]
                .iter()
                .any(|lock| attrs.flag(lock));
            return Ok(Handled::Yes);
        }
        if name.is("definedNames") {
            reader.children(|reader, name, attrs| {
                if !name.is("definedName") {
                    return Ok(Handled::No);
                }
                let (Some(defined), local) = (
                    attrs.plain("name").map(str::to_owned),
                    attrs
                        .plain("localSheetId")
                        .and_then(|i| i.trim().parse().ok()),
                ) else {
                    return Ok(Handled::Yes);
                };
                out.names.push(DefinedName {
                    name: defined,
                    text: reader.text()?,
                    local,
                });
                Ok(Handled::Yes)
            })?;
            return Ok(Handled::Yes);
        }
        if !name.is("sheets") {
            return Ok(Handled::No);
        }
        reader.children(|_, name, attrs| {
            if !name.is("sheet") {
                return Ok(Handled::No);
            }
            let Some(sheet_name) = attrs.plain("name") else {
                // A sheet with no name is a sheet nothing can refer to. Left out of the list
                // rather than given a made-up one.
                return Ok(Handled::Yes);
            };
            out.sheets.push(SheetEntry {
                name: sheet_name.to_owned(),
                rel_id: attrs
                    .get(crate::names::Ns::Relationships, "id")
                    .map(str::to_owned),
                hidden: matches!(attrs.plain("state"), Some("hidden" | "veryHidden")),
                part: None,
            });
            Ok(Handled::Yes)
        })?;
        Ok(Handled::Yes)
    })?;

    report.must_understand.extend(reader.must_understand);
    if out.declares_strict {
        reader.seen.strict = true;
    }
    report.flavour = reader.seen.flavour();
    Ok(out)
}

/// Resolve each sheet's `r:id`, and find the string table and the styles, against the
/// workbook's own relationships.
pub fn resolve_parts(
    workbook: &mut Workbook,
    package: &mut Package,
    workbook_part: &str,
    seen: &mut Seen,
) {
    let rels = package.rels(workbook_part, seen);
    let first = |kind: RelType| {
        rels.iter()
            .find(|rel| rel.kind == kind && !rel.external)
            .map(|rel| rel.target.clone())
    };
    workbook.strings_part = first(RelType::SharedStrings);
    workbook.styles_part = first(RelType::Styles);
    workbook.theme_part = first(RelType::Theme);
    for sheet in &mut workbook.sheets {
        let Some(id) = &sheet.rel_id else { continue };
        sheet.part = rels
            .iter()
            .find(|rel| &rel.id == id && !rel.external)
            // A `<sheet>` may point at a chartsheet or a dialogsheet as well as a worksheet.
            // Only a worksheet has cells; the others keep their tab and stay empty.
            .filter(|rel| rel.kind == RelType::Worksheet)
            .map(|rel| rel.target.clone());
    }
}

/// Turn the skeleton into a `Document`.
///
/// Built through the model's **public API only** — `Sheet::new`, and the `Document` fields
/// that are public. No core change is needed to construct one, and none may be added that
/// hands out mutable internals.
pub fn document(workbook: &Workbook, report: &mut Report) -> Document {
    let mut sheets: Vec<Sheet> = Vec::with_capacity(workbook.sheets.len());
    for entry in &workbook.sheets {
        if entry.hidden {
            report.drop_one(Dropped::HiddenSheet);
        }
        let name = unique_name(&keepable(&entry.name), &sheets);
        if name != entry.name {
            report.renamed.push((entry.name.clone(), name.clone()));
        }
        sheets.push(Sheet::new(name));
    }
    if workbook.protected {
        report.drop_one(Dropped::Protection);
    }
    // `Document::default()` is one sheet called `Sheet1`; a workbook with no sheets at all is
    // not a document anybody wants, and an empty grid to type into is the better answer than
    // a document with no sheet for a caret to be on. Same split as
    // `grind_text::Document::default` vs `empty`.
    if sheets.is_empty() {
        sheets.push(Sheet::new("Sheet1"));
    }
    report.sheets = sheets.len();
    Document {
        sheets,
        null_date: if workbook.date_1904 {
            null_date_1904()
        } else {
            date::DEFAULT_NULL_DATE
        },
        ..Document::default()
    }
}

/// The names a formula may use, and the ones it may not.
#[derive(Debug, Default)]
pub struct Names {
    /// Per sheet index, the names that are sheet-local there — lower-case, since names match
    /// case-insensitively (ODF §5.11, and Excel). A formula on that sheet naming one is refused.
    pub local: Vec<BTreeSet<String>>,
}

impl Names {
    pub fn is_local(&self, sheet: usize, name: &str) -> bool {
        self.local
            .get(sheet)
            .is_some_and(|names| names.contains(&name.to_lowercase()))
    }
}

/// Carry `<definedNames>` into `document.names`, and say which names are sheet-local where.
///
/// - `_xlnm.` names are Excel's own — `Print_Area`, `Print_Titles`, `_FilterDatabase` (an
///   autofilter's range, which `<autoFilter>` states again) — print settings and bookkeeping
///   wearing a name's clothes. They are not the author's names and do not arrive as if they
///   were; print is not a feature here, and a filter is read from the sheet.
/// - A name with `localSheetId` that is **the only definition of its name** is carried like a
///   global one. The model's names are document-wide, and `odf/read.rs` already flattens
///   ODF's own sheet-local names into the one map (the ponytail on `Document::names`); with
///   one definition there is nothing to pick between. The cost is that the name becomes
///   visible from other sheets too, where Excel would have said `#NAME?`. Loop A′ is why this
///   is not the stricter rule: 40 of the corpus's 54 sheet-local names are unique, and
///   dropping them refused 2,506 formulas that meant exactly one thing.
/// - A sheet-local name that **collides** — two sheets' `Rate`, or a local `Rate` beside a
///   global one — is dropped and counted: flattening would silently pick one. A global
///   definition of the same name is kept, and a formula on a sheet where the local one
///   applied is refused rather than pointed at the global.
/// - Everything else is translated by [`crate::formula`] like a cell's formula, with renamed
///   sheets rewritten, and stored the way `odf/read.rs` stores one: lower-case key, OpenFormula
///   text with no `=`. A name whose expression will not translate, or whose spelling is not
///   one identifier to the core's lexer (§5.11), is lost and listed in `Report::names_lost`.
pub fn names(workbook: &Workbook, document: &mut Document, report: &mut Report) -> Names {
    let mut out = Names {
        local: vec![BTreeSet::new(); document.sheets.len()],
    };
    let mut definitions: std::collections::HashMap<String, usize> = Default::default();
    for defined in &workbook.names {
        *definitions.entry(defined.name.to_lowercase()).or_default() += 1;
    }
    for defined in &workbook.names {
        if defined.name.starts_with("_xlnm.") {
            continue;
        }
        let collides = definitions[&defined.name.to_lowercase()] > 1;
        if let Some(sheet) = defined.local.filter(|_| collides) {
            report.drop_one(Dropped::SheetLocalName);
            if let Some(set) = out.local.get_mut(sheet) {
                set.insert(defined.name.to_lowercase());
            }
            continue;
        }
        let spelled = matches!(
            lex::lex(&defined.name).as_deref(),
            Ok([lex::Token::Name(n)]) if *n == defined.name
        );
        let translated = match spelled {
            false => Err(Refusal::Syntax),
            // Parenthesised, then unwrapped: in a cell a comma belongs to a function call, but
            // at the top of a name it is Excel's union operator (`A1:A2,B4:B5`), and inside
            // parentheses is where the translator reads it as one — refused by *class*, not as
            // unreadable. `(expr)` is otherwise the same expression.
            true => formula::translate(&format!("({})", defined.text)).map(|expr| match expr {
                Expr::Paren(inner) => *inner,
                other => other,
            }),
        };
        match translated {
            Ok(expr) => {
                let expr = renamed(&expr, &report.renamed);
                document
                    .names
                    .insert(defined.name.to_lowercase(), expr.to_string());
                report.names += 1;
            }
            Err(refusal) => report.names_lost.push((defined.name.clone(), refusal)),
        }
    }
    out
}

/// `expr` with every renamed sheet's references following it — `formula::rename`, the core's
/// AST substitution that a sheet rename already uses, so no string surgery happens here.
pub fn renamed(expr: &Expr, renames: &[(String, String)]) -> Expr {
    renames.iter().fold(expr.clone(), |expr, (from, to)| {
        rename::rename(&expr, from, to)
    })
}

/// A sheet name LibreOffice will keep: each of `[ ] * ? : / \` becomes `_`, and so does an
/// apostrophe at either end. Everything else — spaces, CJK, 31 characters — is left alone.
pub fn keepable(name: &str) -> String {
    let last = name.chars().count().saturating_sub(1);
    name.chars()
        .enumerate()
        .map(|(i, c)| match c {
            '[' | ']' | '*' | '?' | ':' | '/' | '\\' => '_',
            '\'' if i == 0 || i == last => '_',
            c => c,
        })
        .collect()
}

/// Excel permits sheet names this model would then have two of. Deterministic rather than
/// clever: the second `Sheet1` becomes `Sheet1 (2)`, and [`Report::renamed`] says so.
fn unique_name(name: &str, taken: &[Sheet]) -> String {
    let clashes = |candidate: &str| {
        taken
            .iter()
            .any(|sheet| sheet.name.eq_ignore_ascii_case(candidate))
    };
    let base = if name.is_empty() { "Sheet" } else { name };
    if !clashes(base) {
        return base.to_owned();
    }
    (2..)
        .map(|n| format!("{base} ({n})"))
        .find(|candidate| !clashes(candidate))
        .expect("the integers do not run out")
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: &str = crate::names::MAIN_T;
    const REL: &str = crate::names::REL_T;

    fn parse(xml: &str) -> (Workbook, Report) {
        let mut report = Report::default();
        let workbook = read(xml.as_bytes(), &mut report).expect("a workbook");
        (workbook, report)
    }

    #[test]
    fn sheets_keep_their_order_and_their_relationship() {
        let xml = format!(
            r#"<workbook xmlns="{MAIN}" xmlns:r="{REL}">
                 <sheets>
                   <sheet name="Budget" sheetId="1" r:id="rId1"/>
                   <sheet name="Notes" sheetId="7" r:id="rId4"/>
                 </sheets>
               </workbook>"#
        );
        let (workbook, _) = parse(&xml);
        let names: Vec<_> = workbook.sheets.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Budget", "Notes"]);
        // Order is document order, not `sheetId` order — `sheetId` is an identifier, and
        // reading it as a position is a classic way to shuffle somebody's workbook.
        assert_eq!(workbook.sheets[1].rel_id.as_deref(), Some("rId4"));
    }

    #[test]
    fn the_1904_system_moves_the_epoch() {
        let plain = format!(r#"<workbook xmlns="{MAIN}"><sheets/></workbook>"#);
        let (workbook, mut report) = parse(&plain);
        assert_eq!(
            document(&workbook, &mut report).null_date,
            date::DEFAULT_NULL_DATE
        );

        let xml =
            format!(r#"<workbook xmlns="{MAIN}"><workbookPr date1904="1"/><sheets/></workbook>"#);
        let (workbook, mut report) = parse(&xml);
        assert_eq!(
            document(&workbook, &mut report).null_date,
            date::days_from_civil(1904, 1, 1)
        );
    }

    #[test]
    fn a_hidden_sheet_keeps_its_data_and_loses_its_hiddenness() {
        let xml = format!(
            r#"<workbook xmlns="{MAIN}">
                 <sheets><sheet name="Secret" state="hidden"/></sheets>
               </workbook>"#
        );
        let (workbook, mut report) = parse(&xml);
        let document = document(&workbook, &mut report);
        assert_eq!(document.sheets[0].name, "Secret");
        assert_eq!(report.dropped[&Dropped::HiddenSheet], 1);
    }

    /// The whole reason the walker exists, exercised on the part X0 actually reads.
    #[test]
    fn an_alternation_around_the_sheet_list_is_seen_through() {
        let xml = format!(
            r#"<workbook xmlns="{MAIN}" xmlns:mc="{mce}" xmlns:x15="http://example.invalid/x15">
                 <mc:AlternateContent>
                   <mc:Choice Requires="x15"><x15:nonsense/></mc:Choice>
                   <mc:Fallback><workbookPr date1904="1"/></mc:Fallback>
                 </mc:AlternateContent>
                 <sheets><sheet name="One"/></sheets>
               </workbook>"#,
            mce = crate::names::MCE
        );
        let (workbook, _) = parse(&xml);
        assert!(workbook.date_1904, "the fallback's workbookPr was read");
        assert_eq!(workbook.sheets.len(), 1);
    }

    #[test]
    fn a_strict_workbook_is_read_by_the_same_code() {
        let xml = format!(
            r#"<workbook xmlns="{}" conformance="strict"><sheets><sheet name="S"/></sheets></workbook>"#,
            crate::names::MAIN_S
        );
        let (workbook, report) = parse(&xml);
        assert_eq!(workbook.sheets.len(), 1);
        assert!(workbook.declares_strict);
        assert_eq!(report.flavour, crate::names::Flavour::Strict);
    }

    #[test]
    fn a_part_that_is_not_a_workbook_is_an_error_rather_than_an_empty_document() {
        let xml = format!(r#"<worksheet xmlns="{MAIN}"/>"#);
        let mut report = Report::default();
        assert!(read(xml.as_bytes(), &mut report).is_err());
    }

    #[test]
    fn a_workbook_with_no_sheets_still_has_somewhere_to_type() {
        let xml = format!(r#"<workbook xmlns="{MAIN}"><sheets/></workbook>"#);
        let (workbook, mut report) = parse(&xml);
        let document = document(&workbook, &mut report);
        assert_eq!(document.sheets.len(), 1);
        assert_eq!(report.sheets, 1);
    }

    #[test]
    fn two_sheets_of_one_name_are_told_apart() {
        let xml = format!(
            r#"<workbook xmlns="{MAIN}">
                 <sheets><sheet name="Data"/><sheet name="data"/><sheet name=""/></sheets>
               </workbook>"#
        );
        let (workbook, mut report) = parse(&xml);
        let names: Vec<_> = document(&workbook, &mut report)
            .sheets
            .iter()
            .map(|s| s.name.clone())
            .collect();
        assert_eq!(names, ["Data", "data (2)", "Sheet"]);
    }

    #[test]
    fn a_name_libreoffice_would_not_keep_is_renamed_and_nothing_else_is() {
        assert_eq!(keepable("Has[Brackets]"), "Has_Brackets_");
        assert_eq!(keepable("Has/Slash"), "Has_Slash");
        assert_eq!(keepable("a*b?c:d\\e"), "a_b_c_d_e");
        assert_eq!(keepable("'quoted'"), "_quoted_");
        assert_eq!(
            keepable("O'Brien"),
            "O'Brien",
            "an inner apostrophe is fine"
        );
        assert_eq!(keepable("Ünïcødé Ω 日本"), "Ünïcødé Ω 日本");
        // A rename that lands on an existing name takes the usual suffix.
        let xml = format!(
            r#"<workbook xmlns="{MAIN}"><sheets><sheet name="A_B"/><sheet name="A/B"/></sheets></workbook>"#
        );
        let (workbook, mut report) = parse(&xml);
        let document = document(&workbook, &mut report);
        assert_eq!(document.sheets[1].name, "A_B (2)");
        assert_eq!(report.renamed, [("A/B".to_owned(), "A_B (2)".to_owned())]);
    }

    /// Global names carry, translated and following a renamed sheet; `_xlnm.` names are not
    /// the author's; a sheet-local one carries when it is the only one of its name, and is
    /// dropped and remembered where it is local when it collides.
    #[test]
    fn defined_names_are_carried_dropped_or_skipped() {
        let xml = format!(
            r#"<workbook xmlns="{MAIN}"><sheets><sheet name="Data"/><sheet name="Odd/One"/></sheets>
                 <definedNames>
                   <definedName name="Total">SUM(Data!$A$1:$A$9)</definedName>
                   <definedName name="Elsewhere">'Odd/One'!$B$2</definedName>
                   <definedName name="Rate" localSheetId="1">Data!$C$1</definedName>
                   <definedName name="Rate">Data!$C$2</definedName>
                   <definedName name="Alone" localSheetId="0">Data!$D$1</definedName>
                   <definedName name="_xlnm.Print_Area" localSheetId="0">Data!$A$1:$B$5</definedName>
                   <definedName name="_xlnm._FilterDatabase" hidden="1">Data!$A$1:$C$9</definedName>
                   <definedName name="Two">Data!$A$1,Data!$B$2</definedName>
                 </definedNames></workbook>"#
        );
        let (workbook, mut report) = parse(&xml);
        let mut doc = document(&workbook, &mut report);
        let names = names(&workbook, &mut doc, &mut report);
        assert_eq!(doc.names["total"], "SUM([Data.$A$1:.$A$9])");
        assert_eq!(doc.names["elsewhere"], "[Odd_One.$B$2]");
        // The global `Rate` stays; the local one beside it cannot, and on its sheet a formula
        // naming `Rate` means the one that is gone.
        assert_eq!(doc.names["rate"], "[Data.$C$2]");
        assert_eq!(
            doc.names["alone"], "[Data.$D$1]",
            "the only `Alone`: nothing to pick"
        );
        assert_eq!(doc.names.len(), 4);
        assert_eq!(report.names, 4);
        assert_eq!(report.dropped[&Dropped::SheetLocalName], 1);
        assert!(names.is_local(1, "RATE") && !names.is_local(0, "rate"));
        assert_eq!(report.names_lost, [("Two".to_owned(), Refusal::Union)]);
    }

    #[test]
    fn protection_that_locks_nothing_is_not_protection() {
        let empty =
            format!(r#"<workbook xmlns="{MAIN}"><workbookProtection/><sheets/></workbook>"#);
        assert!(!parse(&empty).0.protected);
        let locked = format!(
            r#"<workbook xmlns="{MAIN}"><workbookProtection lockStructure="1"/><sheets/></workbook>"#
        );
        assert!(parse(&locked).0.protected);
    }
}
