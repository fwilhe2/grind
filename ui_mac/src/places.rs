// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The sidebar's rows — *where you can go* (decision 3) — for either document, portable.
//!
//! A spreadsheet's are its sheets, its defined names and its **Problems**; a page's are its
//! outline, its bookmarks and its Problems (D6, M8). Every row but a heading is a jump, and a
//! jump is an address the pane's own go-to machinery already reads — `a1` for the grid, `loc`
//! for the page — so a Problems row goes where `grind lint` says the problem is, by the same
//! words. `sidebar.rs` is the one AppKit list over these rows; this file decides what they are.

use grind_core::lint::{Diagnostic, Report, Severity};
use grind_sheet::nav::Selection;

/// Where choosing a row goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Go {
    /// Show this sheet.
    Sheet(usize),
    /// An address, in the document's own spelling: `Data.B2:C9` on a grid, `§2.1` or `#intro`
    /// or `p12+4` on a page.
    Address(String),
}

/// One row: a section's heading, or a place — which goes somewhere unless it is a finding about
/// the whole document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub title: String,
    pub heading: bool,
    pub go: Option<Go>,
    /// A tooltip — a problem's rule, so a reader can look it up.
    pub tip: Option<String>,
}

impl Row {
    fn heading(title: &str) -> Row {
        Row {
            title: title.to_uppercase(),
            heading: true,
            go: None,
            tip: None,
        }
    }

    fn place(title: impl Into<String>, go: Go) -> Row {
        Row {
            title: title.into(),
            heading: false,
            go: Some(go),
            tip: None,
        }
    }

    pub fn is_heading(&self) -> bool {
        self.heading
    }
}

/// The mark a problem's row starts with, by severity — drawn in the row's own text, since a
/// source list's row is a line of text.
pub fn mark(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "\u{2715}",
        Severity::Warning => "\u{25b2}",
        Severity::Hint => "\u{00b7}",
    }
}

/// The Problems section: a heading, then a row a finding, worst first as the report sorts them;
/// nothing at all for a clean document, since an empty section is a heading saying nothing.
fn problems(report: &Report) -> Vec<Row> {
    if report.is_empty() {
        return Vec::new();
    }
    let mut rows = vec![Row::heading("Problems")];
    rows.extend(report.diagnostics.iter().map(problem));
    rows
}

fn problem(diagnostic: &Diagnostic) -> Row {
    let title = match diagnostic.at.is_empty() {
        true => format!("{} {}", mark(diagnostic.severity), diagnostic.message),
        false => format!(
            "{} {}: {}",
            mark(diagnostic.severity),
            diagnostic.at,
            diagnostic.message
        ),
    };
    Row {
        title,
        heading: false,
        // A finding about the whole document goes nowhere; the row still says it.
        go: (!diagnostic.at.is_empty()).then(|| Go::Address(diagnostic.at.clone())),
        tip: Some(diagnostic.rule.to_owned()),
    }
}

/// A spreadsheet's rows: its sheets, its defined names, and what `grind lint` finds.
pub fn sheet(app: &grind_sheet::App, report: &Report) -> Vec<Row> {
    let mut rows = vec![Row::heading("Sheets")];
    rows.extend(
        (0..app.sheet_count())
            .map(|sheet| Row::place(app.sheet_name(sheet).unwrap_or_default(), Go::Sheet(sheet))),
    );
    let names = app.names();
    if !names.is_empty() {
        rows.push(Row::heading("Names"));
        rows.extend(
            names
                .into_iter()
                .map(|(name, _)| Row::place(name.clone(), Go::Address(name))),
        );
    }
    rows.extend(problems(report));
    rows
}

/// A page's rows: its outline, indented by level, its bookmarks, and what `grind lint` finds.
pub fn text(app: &grind_text::App, report: &Report) -> Vec<Row> {
    let mut rows = Vec::new();
    let outline = app.outline();
    if !outline.is_empty() {
        rows.push(Row::heading("Outline"));
        rows.extend(outline.iter().map(|heading| {
            let indent = "   ".repeat(heading.level.saturating_sub(1) as usize);
            Row::place(
                format!("{indent}{}", heading.text),
                Go::Address(heading.address()),
            )
        }));
    }
    let bookmarks = app.bookmarks();
    if !bookmarks.is_empty() {
        rows.push(Row::heading("Bookmarks"));
        rows.extend(
            bookmarks
                .into_iter()
                .map(|(name, _)| Row::place(name.clone(), Go::Address(format!("#{name}")))),
        );
    }
    rows.extend(problems(report));
    rows
}

/// Where a grid's address is: its sheet, and the selection it makes — a defined name through
/// the name box's own reading, any other address through `a1`, **allowed to land on another
/// sheet** than the one showing, since a Problems row is about the whole document.
pub fn cells(app: &grind_sheet::App, showing: usize, address: &str) -> Option<(usize, Selection)> {
    if let Some(selection) = grind_sheet::place::locate(app, showing, address) {
        return Some((showing, selection));
    }
    // A name the name box refused for living on another sheet: its own definition is the
    // address, less the brackets a stored reference carries.
    let defined = app
        .names()
        .into_iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(address))
        .map(|(_, expression)| expression);
    let address = defined.as_deref().map_or(address, |expression| {
        expression.trim_start_matches('[').trim_end_matches(']')
    });
    let reference = grind_sheet::a1::parse(address).ok()?;
    let (sheet, start, end) = grind_sheet::a1::resolve(app, &reference).ok()?;
    Some((
        sheet,
        Selection {
            anchor: end,
            active: start,
        },
    ))
}

/// Where a page's address is.
pub fn caret(app: &grind_text::App, address: &str) -> Option<grind_text::Caret> {
    let loc = grind_text::loc::parse(address).ok()?;
    app.resolve_caret(&loc).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use grind_core::lint::Options;
    use grind_sheet::{Pos, RecalcMode};
    use grind_text::BlockKind;

    #[test]
    fn a_spreadsheet_lists_its_sheets_names_and_problems() {
        let app = grind_sheet::App::new();
        app.add_sheet("Data").unwrap();
        app.enter(1, Pos::new(1, 1), "5", RecalcMode::Document)
            .unwrap();
        app.set_name("rate", "[$Data.$B$2]").unwrap();
        app.enter(0, Pos::new(0, 0), "=[.Z99]*rate", RecalcMode::Document)
            .unwrap();
        let report = app.lint(&Options::default());
        let rows = sheet(&app, &report);
        let titles: Vec<&str> = rows.iter().map(|row| row.title.as_str()).collect();
        assert_eq!(&titles[..5], ["SHEETS", "Sheet1", "Data", "NAMES", "rate"]);
        assert!(rows[0].is_heading());
        assert_eq!(rows[2].go, Some(Go::Sheet(1)));
        assert_eq!(rows[4].go, Some(Go::Address("rate".into())));
        // The name goes to its own sheet, and a Problems row to where lint says.
        let b2 = Pos::new(1, 1);
        assert_eq!(
            cells(&app, 0, "rate"),
            Some((
                1,
                Selection {
                    anchor: b2,
                    active: b2
                }
            ))
        );
        assert!(!report.is_empty(), "A1 reads the empty Z99");
        {
            let problem = rows
                .iter()
                .skip_while(|row| row.title != "PROBLEMS")
                .nth(1)
                .expect("a row per finding");
            let Some(Go::Address(at)) = &problem.go else {
                panic!("{problem:?} goes nowhere")
            };
            assert!(cells(&app, 0, at).is_some(), "{at} is a place");
            assert!(problem.tip.is_some());
        }
    }

    #[test]
    fn a_clean_document_has_no_problems_heading() {
        let app = grind_sheet::App::new();
        let rows = sheet(&app, &Report::default());
        assert!(rows.iter().all(|row| row.title != "PROBLEMS"));
        assert!(
            rows.iter().all(|row| row.title != "NAMES"),
            "no names, no heading"
        );
    }

    #[test]
    fn a_page_lists_its_outline_and_bookmarks_and_each_is_a_caret() {
        let app = grind_text::App::new();
        app.set_text(0, "Intro").unwrap();
        app.set_kind(0, BlockKind::Heading { level: 1 }).unwrap();
        app.insert(1, BlockKind::Paragraph, "some prose").unwrap();
        app.insert(2, BlockKind::Heading { level: 2 }, "Detail")
            .unwrap();
        app.set_bookmark("here", Some(1)).unwrap();
        let rows = text(&app, &Report::default());
        let titles: Vec<&str> = rows.iter().map(|row| row.title.as_str()).collect();
        assert_eq!(
            titles,
            ["OUTLINE", "Intro", "   Detail", "BOOKMARKS", "here"]
        );
        for row in &rows {
            if let Some(Go::Address(at)) = &row.go {
                assert!(caret(&app, at).is_some(), "{at}");
            }
        }
        assert_eq!(caret(&app, "#here").map(|c| c.block), Some(1));
    }

    #[test]
    fn a_finding_about_the_whole_document_goes_nowhere_and_still_says_so() {
        use grind_core::lint::Rule;
        const RULE: Rule = Rule {
            id: "whole",
            severity: Severity::Warning,
            what: "made up",
        };
        let mut report = Report::default();
        report.push(Diagnostic::new(&RULE, "", "is odd"));
        let rows = problems(&report);
        assert_eq!(rows[1].go, None);
        assert!(!rows[1].is_heading());
        assert!(rows[1].title.contains("is odd"));
    }
}
