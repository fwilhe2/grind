// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! View modes: what a document *means*, derived rather than stored.
//!
//! `doc/view-modes.md` is normative for this module. Two overlays on top of the values a
//! shell already draws:
//!
//! * **Roles** (§4) — every cell has one, and the document already implies which: an input,
//!   a computed value, a label, an unnamed constant something computes with.
//! * **Name anchors** (§3) — where a named expression *lives*, so a model does not need a
//!   label cell beside every constant.
//!
//! **Nothing here is ever written to a document** (§1). Not as a cell style, not as
//! `calcext:`, not as a settings block. A role is a reading of a document rather than a
//! property of one, and the reason to derive it instead of shipping a palette of named
//! styles is that a *stored* classification goes stale and a derived one cannot.
//!
//! The precedent is `formula::display::spans`, which exposes the reference scanner in byte
//! ranges precisely so the editor's colourer and its committer cannot disagree about what a
//! reference is. This is that idea moved from the formula bar to the grid: **one classifier,
//! in the core; shells that choose colours and nothing else.**

use std::ops::Range;

use crate::formula::eval::{Address, Area, Engine, to_cell};
use crate::formula::parse::{Expr, parse};
use crate::graph::RefIndex;
use crate::model::{CellValue, Document, Pos, Sheet};

/// How many formulas must read a literal *by address* before not naming it is a problem —
/// `doc/view-modes.md` §4.2's "a lone `0.2` that **three formulas** multiply by", with the
/// bar set at two.
///
/// One reader is the ordinary shape of a spreadsheet: a column of data with a column of
/// formulas beside it, each reading its own row. Setting the bar here rather than at one is
/// what stops the rule flagging every data table in the corpus, and it errs towards a false
/// *negative* deliberately — §9's judgement is that an annoying lint gets turned off, and a
/// lint that is turned off finds nothing at all.
const MAGIC_READERS: usize = 2;

/// What a cell *is*, as the document already implies it.
///
/// **Total and disjoint**: every cell has exactly one, and that is what makes the mode
/// legible — a wash with gaps in it reads as a bug.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CellRole {
    /// Nothing there. The one role a shell draws as nothing at all.
    #[default]
    Empty,
    /// A literal with a name bound to it — the good case.
    InputNamed,
    /// A literal no formula singles out. Ordinary data — a table of numbers under a total
    /// is this, and it is not a problem.
    InputUnnamed,
    /// A literal **some formula singles out by address**, with no name bound to it: §4.2's
    /// magic number. The fix is `grind sheet name`, and the hint from Part I is what fixing
    /// one looks like. Two things do **not** count: being inside a range a `SUM` covers, and
    /// being one of a line of literals — see `MAGIC_READERS` and `in_a_run`.
    ConstantUnnamed,
    /// A formula whose references are all on its own sheet.
    ComputedLocal,
    /// A formula reading another sheet — "green" in the financial-modelling convention this
    /// borrows.
    ComputedCrossSheet,
    /// Text: the row and column headings a person writes.
    Label,
    /// A formula that evaluates to an error.
    Error,
    /// A formula whose cached value disagrees with re-evaluating it — the same disagreement
    /// [`crate::App::stale`] counts.
    Stale,
}

impl CellRole {
    /// The role's name, as the CLI prints it and as `--format json` spells it.
    ///
    /// Stable, kebab-case, and the one vocabulary every shell shares: a command id, a JSON
    /// field and a legend row all say the same word.
    pub fn name(self) -> &'static str {
        match self {
            CellRole::Empty => "empty",
            CellRole::InputNamed => "input-named",
            CellRole::InputUnnamed => "input-unnamed",
            CellRole::ConstantUnnamed => "constant-unnamed",
            CellRole::ComputedLocal => "computed-local",
            CellRole::ComputedCrossSheet => "computed-cross-sheet",
            CellRole::Label => "label",
            CellRole::Error => "error",
            CellRole::Stale => "stale",
        }
    }

    /// Whether this role is also a **diagnostic** — something wrong rather than something
    /// true (§4.3).
    ///
    /// The distinction is load-bearing for the drawing: roles get the fill or the text
    /// colour, diagnostics get a mark. If the mode painted both in one channel an ordinary
    /// model would look like a wall of warnings, and people turn that off in a week.
    pub fn is_diagnostic(self) -> bool {
        matches!(
            self,
            CellRole::Error | CellRole::Stale | CellRole::ConstantUnnamed
        )
    }

    /// Every role, in the order this module lists them — what a legend and a totality test
    /// both walk.
    pub const ALL: [CellRole; 9] = [
        CellRole::Empty,
        CellRole::InputNamed,
        CellRole::InputUnnamed,
        CellRole::ConstantUnnamed,
        CellRole::ComputedLocal,
        CellRole::ComputedCrossSheet,
        CellRole::Label,
        CellRole::Error,
        CellRole::Stale,
    ];
}

/// Where a named expression lives: a name, and the rectangle it denotes.
///
/// Only a name whose expression is a **plain reference** has one. `[.A1]*2` is a computed
/// name — it denotes no place, the formula bar can still show it, and the grid has nowhere
/// to put it (§3.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NameAnchor {
    /// The name as it was declared, not lower-cased — this is what gets drawn.
    pub name: String,
    pub sheet: usize,
    pub rows: Range<u32>,
    pub cols: Range<u32>,
}

impl NameAnchor {
    pub fn contains(&self, at: Address) -> bool {
        self.sheet == at.sheet && self.rows.contains(&at.pos.row) && self.cols.contains(&at.pos.col)
    }

    /// Whether this anchor covers more than one cell — the case a shell draws **once**, at
    /// the first visible cell, rather than forty-nine times (§3.2).
    pub fn is_range(&self) -> bool {
        self.rows.len() > 1 || self.cols.len() > 1
    }
}

/// One document, read for what it means.
///
/// Document-wide by nature — *nothing references this cell* cannot be answered inside a
/// viewport — and therefore built once and cached by [`crate::App`], which throws it away on
/// every mutation. Rule 1 is about what leaves the core, not about what the core may know.
///
/// The cost of building one is a full recalculation (for staleness) plus one walk of the
/// parsed formulas (for the reference index) — the same cost as [`crate::App::stale`], and
/// for the same reason: there is no second implementation of what counts as stale.
#[derive(Clone, Debug, Default)]
pub struct Analysis {
    refs: RefIndex,
    anchors: Vec<NameAnchor>,
    /// The role of every **formula** cell, which is the half that needs evaluating. A
    /// literal's role is answered from `refs` and `anchors` on demand, in [`Analysis::role`]
    /// — which is what keeps this from being a vector the size of the used area.
    formulas: std::collections::BTreeMap<Address, CellRole>,
}

impl Analysis {
    pub fn build(doc: &Document) -> Self {
        let refs = RefIndex::build(doc);
        let anchors = anchors(doc);
        let mut formulas = std::collections::BTreeMap::new();
        let mut engine = Engine::new(doc);
        for (index, sheet) in doc.sheets.iter().enumerate() {
            for (pos, _) in sheet.formulas() {
                let at = Address::new(index, pos);
                let value = to_cell(engine.value(at));
                let role = if crate::is_error(&value) {
                    CellRole::Error
                } else if value != sheet.get(pos) {
                    CellRole::Stale
                } else if refs.reads(at).iter().any(|area| area.sheet != index) {
                    CellRole::ComputedCrossSheet
                } else {
                    CellRole::ComputedLocal
                };
                formulas.insert(at, role);
            }
        }
        Self {
            refs,
            anchors,
            formulas,
        }
    }

    /// The role of one cell. `sheet` must be the sheet `at` names — the caller has it in
    /// hand and this avoids a second lookup per cell of a viewport.
    ///
    /// The order of the arms *is* the disjointness rule, so it is worth reading as one:
    /// a formula's role was decided when the analysis was built; then a name wins over
    /// everything, because a named cell is the case this whole feature is arguing for;
    /// then text is a label; then a referenced literal is a magic constant; and everything
    /// left is ordinary data.
    pub fn role(&self, sheet: &Sheet, at: Address) -> CellRole {
        if let Some(role) = self.formulas.get(&at) {
            return *role;
        }
        let value = sheet.get(at.pos);
        if value.is_empty() {
            return CellRole::Empty;
        }
        if self.anchors.iter().any(|a| a.contains(at)) {
            return CellRole::InputNamed;
        }
        // A **text** literal is never a magic constant, even when a formula reads it. A sum
        // whose range overlaps its own column heading references that heading, and calling
        // it a decision somebody failed to write down is exactly the false positive
        // `doc/view-modes.md` §9 warns the rule can die of. §4.2's case is a lone `0.2` that
        // three formulas multiply by, and that is a number.
        if matches!(value, CellValue::Text(_)) {
            return CellRole::Label;
        }
        // §4.2's magic constant, and it takes **both** signals — see `MAGIC_READERS` and
        // [`in_a_run`]. Read by address from more than one place, and not one of a line of
        // literals. Either alone flags a whole data table, which `examples/sample-sheet.sh`
        // demonstrated twice.
        match self.refs.singled_out_by(at) >= MAGIC_READERS && !in_a_run(sheet, at.pos) {
            true => CellRole::ConstantUnnamed,
            false => CellRole::InputUnnamed,
        }
    }

    /// Every name anchor in the document, in declaration order.
    pub fn anchors(&self) -> &[NameAnchor] {
        &self.anchors
    }

    /// The anchors intersecting a rectangle — what a viewport carries.
    pub fn anchors_in(
        &self,
        sheet: usize,
        rows: Range<u32>,
        cols: Range<u32>,
    ) -> impl Iterator<Item = &NameAnchor> {
        self.anchors.iter().filter(move |a| {
            a.sheet == sheet
                && a.rows.start < rows.end
                && rows.start < a.rows.end
                && a.cols.start < cols.end
                && cols.start < a.cols.end
        })
    }

    /// The name bound to one cell, if any.
    pub fn name_at(&self, at: Address) -> Option<&str> {
        self.anchors
            .iter()
            .find(|a| a.contains(at))
            .map(|a| a.name.as_str())
    }

    /// The reference index this was built on — §4.4's graph, for a caller that wants the
    /// dependency answers rather than the colours.
    pub fn refs(&self) -> &RefIndex {
        &self.refs
    }
}

/// Every named expression that denotes a place, resolved.
///
/// **A name anchors only when its expression names a sheet**, and that is §3.5 rather than a
/// shortcut. `Document::names` is one flat map, so a sheet-local name is visible
/// document-wide (`model.rs`); a hint drawn from an unqualified `[.B2]` would appear on
/// *every* sheet's B2, which is worse than no hint at all. Our writer stores a named range
/// fully qualified, and so does LibreOffice, so the case this declines is the ambiguous one.
fn anchors(doc: &Document) -> Vec<NameAnchor> {
    let engine = Engine::new(doc);
    let mut out = Vec::new();
    for (name, expression) in &doc.names {
        let Ok(expr) = parse(expression) else {
            continue;
        };
        let mut expr = &expr;
        while let Expr::Paren(inner) = expr {
            expr = inner;
        }
        let Expr::Ref(reference) = expr else { continue };
        let Some(sheet_name) = reference.start.sheet.as_deref() else {
            continue;
        };
        let Some(sheet) = doc
            .sheets
            .iter()
            .position(|s| s.name.eq_ignore_ascii_case(sheet_name))
        else {
            continue;
        };
        // The far end of a range is written `[$Sheet1.$A$1:.$A$50]` — a bare `.`, meaning
        // "the sheet this is evaluated on". So it is resolved *as if from* the sheet the
        // near end named, which is the only reading that makes the range one rectangle.
        let Some(Area { sheet, rows, cols }) =
            engine.area(reference, Address::new(sheet, Pos::new(0, 0)))
        else {
            continue;
        };
        out.push(NameAnchor {
            name: name.clone(),
            sheet,
            rows,
            cols,
        });
    }
    out
}

/// Whether a cell is one of a **line of literals** — three or more in its column or in its
/// row — which is what a table of data looks like and what a parameter does not.
///
/// The second half of §4.2, and the reason it is needed is worth writing down, because it
/// was found rather than designed. The sample document has a column of actuals with two
/// formula columns beside it, each reading its own row's cell by address; every actual is
/// therefore a literal, unnamed, and singled out by two formulas — the whole of the first
/// rule — and the mode painted the entire column as decisions nobody wrote down. What makes
/// `0.2` a parameter is that it stands on its own. Three is the shortest line that is
/// unambiguously a table rather than a coincidence, and the scan stops at two cells each way
/// so this stays a handful of lookups per cell.
///
/// A formula cell breaks a run: a column of literals under a heading and above a total is a
/// run of literals, not a run of anything else.
fn in_a_run(sheet: &Sheet, pos: Pos) -> bool {
    let kind = |pos: Pos| match sheet.formula(pos).is_some() {
        true => None,
        false => match sheet.get(pos) {
            CellValue::Empty => None,
            CellValue::Number(_) => Some(1u8),
            CellValue::Text(_) => Some(2),
            CellValue::Bool(_) => Some(3),
        },
    };
    let Some(mine) = kind(pos) else { return false };
    // How far the run of like literals reaches from `pos` in one direction, up to two.
    let reach = |step: fn(Pos) -> Option<Pos>| {
        let mut at = pos;
        let mut n = 0;
        while n < 2 {
            let Some(next) = step(at) else { break };
            if kind(next) != Some(mine) {
                break;
            }
            at = next;
            n += 1;
        }
        n
    };
    let up = reach(|p| p.row.checked_sub(1).map(|row| Pos::new(row, p.col)));
    let down = reach(|p| Some(Pos::new(p.row + 1, p.col)));
    let left = reach(|p| p.col.checked_sub(1).map(|col| Pos::new(p.row, col)));
    let right = reach(|p| Some(Pos::new(p.row, p.col + 1)));
    1 + up + down >= 3 || 1 + left + right >= 3
}

/// Which overlays a read should compute (`doc/view-modes.md` §2).
///
/// Requested rather than always computed: a shell that draws neither pays for neither, and
/// the CLI asks for exactly what it prints.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Overlays {
    pub roles: bool,
    pub names: bool,
}

impl Overlays {
    /// What an ordinary paint asks for — the default, and what [`crate::App::get_viewport`]
    /// passes.
    pub const NONE: Overlays = Overlays {
        roles: false,
        names: false,
    };

    pub const ROLES: Overlays = Overlays {
        roles: true,
        names: false,
    };

    pub const NAMES: Overlays = Overlays {
        roles: false,
        names: true,
    };

    pub const ALL: Overlays = Overlays {
        roles: true,
        names: true,
    };

    /// Whether anything is being asked for — whether the analysis has to exist at all.
    pub fn any(self) -> bool {
        self.roles || self.names
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{App, RecalcMode};

    /// The document `doc/view-modes.md` argues about: an input with a name, one without, a
    /// magic constant, a label, a local formula and a cross-sheet one.
    fn app() -> App {
        let app = App::new();
        app.add_sheet("Rates").unwrap();
        // The other sheet first, so nothing reading it is stale before it is written.
        app.enter(1, Pos::new(0, 0), "7", RecalcMode::Document)
            .unwrap();
        for (cell, text) in [
            ("A1", "Region"),      // a label
            ("A2", "North"),       // a label
            ("B1", "Revenue"),     // a label
            ("B2", "4200"),        // data: a literal nothing reads
            ("C2", "0.2"),         // the magic constant: read by D2, unnamed
            ("D2", "=[.C2]*100"),  // local
            ("F2", "=[.C2]/2"),    // a second reader: C2 is a parameter, not data
            ("E2", "=[Rates.A1]"), // cross-sheet
        ] {
            app.enter(0, cell_of(cell), text, RecalcMode::Document)
                .unwrap();
        }
        app.set_name("tax", "[$Rates.$A$1]").unwrap();
        app
    }

    fn cell_of(cell: &str) -> Pos {
        let split = cell.find(|c: char| c.is_ascii_digit()).unwrap();
        let col = cell[..split]
            .bytes()
            .fold(0u32, |n, b| n * 26 + u32::from(b - b'A') + 1);
        Pos::new(cell[split..].parse::<u32>().unwrap() - 1, col - 1)
    }

    fn roles(app: &App, sheet: usize, cells: &[&str]) -> Vec<&'static str> {
        let view = app
            .get_viewport_with(sheet, 0..8, 0..8, Overlays::ALL)
            .unwrap();
        cells
            .iter()
            .map(|c| {
                let pos = cell_of(c);
                view.role(pos.row, pos.col).unwrap().name()
            })
            .collect()
    }

    #[test]
    fn every_kind_of_cell_gets_the_role_the_document_implies() {
        let app = app();
        assert_eq!(
            roles(&app, 0, &["A1", "B2", "C2", "D2", "E2", "H8"]),
            [
                "label",
                "input-unnamed",
                "constant-unnamed",
                "computed-local",
                "computed-cross-sheet",
                "empty",
            ]
        );
        // The named cell is on the other sheet, which is what `tax` points at.
        assert_eq!(roles(&app, 1, &["A1"]), ["input-named"]);
    }

    #[test]
    fn naming_a_magic_constant_is_what_fixing_one_looks_like() {
        // The two features in one document: Part II finds it, Part I is the fix.
        let app = app();
        assert_eq!(roles(&app, 0, &["C2"]), ["constant-unnamed"]);
        app.set_name("tax_rate", "[$Sheet1.$C$2]").unwrap();
        assert_eq!(roles(&app, 0, &["C2"]), ["input-named"]);
        let view = app
            .get_viewport_with(0, 0..8, 0..8, Overlays::NAMES)
            .unwrap();
        assert_eq!(view.name_at(1, 2), Some("tax_rate"));
    }

    #[test]
    fn a_stale_cell_and_an_error_outrank_being_computed() {
        let app = App::new();
        app.enter(0, cell_of("A1"), "2", RecalcMode::Document)
            .unwrap();
        app.enter(0, cell_of("A2"), "=[.A1]*3", RecalcMode::Document)
            .unwrap();
        app.enter(0, cell_of("A3"), "=1/0", RecalcMode::Document)
            .unwrap();
        assert_eq!(roles(&app, 0, &["A2", "A3"]), ["computed-local", "error"]);
        // Editing what A2 reads makes its cached value a second, disagreeing claim.
        app.enter(0, cell_of("A1"), "5", RecalcMode::No).unwrap();
        assert_eq!(roles(&app, 0, &["A2"]), ["stale"]);
        app.recalc().unwrap();
        assert_eq!(roles(&app, 0, &["A2"]), ["computed-local"]);
    }

    #[test]
    fn a_computed_name_denotes_no_place_and_anchors_nothing() {
        let app = App::new();
        app.enter(0, cell_of("A1"), "1", RecalcMode::No).unwrap();
        app.set_name("doubled", "[$Sheet1.$A$1]*2").unwrap();
        let view = app.get_viewport_with(0, 0..4, 0..4, Overlays::ALL).unwrap();
        assert!(view.names().is_empty());
        assert_eq!(view.role(0, 0).unwrap(), CellRole::InputUnnamed);
    }

    #[test]
    fn an_unqualified_name_anchors_nowhere_rather_than_on_every_sheet() {
        // §3.5: `Document::names` is one flat map, so `[.B2]` would hint on Sheet2's B2 as
        // well as Sheet1's. Declining is the only answer that is never wrong.
        let app = App::new();
        app.add_sheet("Sheet2").unwrap();
        app.enter(0, cell_of("B2"), "1", RecalcMode::No).unwrap();
        app.enter(1, cell_of("B2"), "2", RecalcMode::No).unwrap();
        app.set_name("loose", "[.B2]").unwrap();
        for sheet in 0..2 {
            let view = app
                .get_viewport_with(sheet, 0..4, 0..4, Overlays::NAMES)
                .unwrap();
            assert!(view.names().is_empty(), "sheet {sheet}");
        }
    }

    #[test]
    fn a_named_range_is_one_anchor_over_the_whole_rectangle() {
        let app = App::new();
        for row in 0..5 {
            app.enter(0, Pos::new(row, 0), "1", RecalcMode::No).unwrap();
        }
        app.set_name("sales", "[$Sheet1.$A$1:.$A$5]").unwrap();
        let view = app.get_viewport_with(0, 0..8, 0..4, Overlays::ALL).unwrap();
        assert_eq!(view.names().len(), 1);
        assert!(view.names()[0].is_range());
        assert_eq!(view.names()[0].rows, 0..5);
        // Every cell in it is a named input, and the hint is one anchor rather than five.
        assert_eq!(
            roles(&app, 0, &["A1", "A5"]),
            ["input-named", "input-named"]
        );
    }

    #[test]
    fn an_anchor_outside_the_viewport_is_not_carried() {
        let app = App::new();
        app.enter(0, cell_of("H8"), "1", RecalcMode::No).unwrap();
        app.set_name("far", "[$Sheet1.$H$8]").unwrap();
        let near = app
            .get_viewport_with(0, 0..4, 0..4, Overlays::NAMES)
            .unwrap();
        assert!(near.names().is_empty());
        let over = app
            .get_viewport_with(0, 4..10, 4..10, Overlays::NAMES)
            .unwrap();
        assert_eq!(over.names().len(), 1);
    }

    #[test]
    fn a_viewport_without_the_overlay_carries_neither() {
        let app = app();
        let view = app.get_viewport(0, 0..8, 0..8).unwrap();
        assert_eq!(view.role(0, 0), None);
        assert!(view.names().is_empty());
    }

    #[test]
    fn the_analysis_is_rebuilt_after_a_mutation_rather_than_remembered() {
        // §9's one real risk: a cache with an invalidation rule. The rule is trivial —
        // invalidate in `mutate`, where observers are already notified — and this is it.
        let app = App::new();
        app.enter(0, cell_of("A1"), "0.2", RecalcMode::No).unwrap();
        assert_eq!(roles(&app, 0, &["A1"]), ["input-unnamed"]);
        app.enter(0, cell_of("B1"), "=[.A1]*2", RecalcMode::No)
            .unwrap();
        // One reader is a column of data with a formula beside it; two make it a parameter.
        assert_eq!(roles(&app, 0, &["A1"]), ["input-unnamed"]);
        app.enter(0, cell_of("C1"), "=[.A1]+1", RecalcMode::No)
            .unwrap();
        assert_eq!(roles(&app, 0, &["A1"]), ["constant-unnamed"]);
    }

    #[test]
    fn every_cell_of_the_corpus_document_gets_exactly_one_role() {
        // Totality, on the shape the corpus tests assert it over: a role is never absent,
        // and `role` is one function so it can never return two.
        let app = app();
        let view = app
            .get_viewport_with(0, 0..16, 0..16, Overlays::ROLES)
            .unwrap();
        for row in 0..16 {
            for col in 0..16 {
                let role = view.role(row, col).expect("a role for every cell");
                assert!(CellRole::ALL.contains(&role));
            }
        }
    }
}
