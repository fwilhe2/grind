// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! View modes: what a document means, derived rather than stored (`doc/view-modes.md`).
//!
//! * **Roles** (§4) — every cell has one: an input, a computed value, a label, an unnamed constant
//!   something computes with.
//! * **Name anchors** (§3) — where a named expression lives.
//!
//! Nothing here is ever written to a document (§1): a stored classification goes stale and a
//! derived one cannot. One classifier, in the core; shells choose colours and nothing else.

use std::ops::Range;

use crate::formula::eval::{Address, Area, Engine, to_cell};
use crate::formula::lex::{Op, Reference, SyntaxError};
use crate::formula::parse::{Expr, parse};
use crate::formula::serialize::Bare;
use crate::graph::RefIndex;
use crate::model::{CellValue, Document, Pos, Sheet};

/// How many formulas must read a literal by address before it counts as an unnamed constant
/// (§4.2). One reader is an ordinary data column beside its formulas, so the bar is two; it errs
/// towards false negatives, since an annoying lint gets turned off.
const MAGIC_READERS: usize = 2;

/// What a cell is, as the document implies it. Total and disjoint: every cell has exactly one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CellRole {
    /// Nothing there. The one role a shell draws as nothing at all.
    #[default]
    Empty,
    /// A literal with a name bound to it — the good case.
    InputNamed,
    /// A literal no formula singles out — ordinary data.
    InputUnnamed,
    /// A literal some formula singles out by address, with no name — §4.2's magic number, fixed by
    /// `grind sheet name`. Being inside a range a `SUM` covers does not count, nor being one of a
    /// line of literals (`MAGIC_READERS`, `in_a_run`).
    ConstantUnnamed,
    /// A formula whose references are all on its own sheet.
    ComputedLocal,
    /// A formula reading another sheet — "green" in the financial-modelling convention.
    ComputedCrossSheet,
    /// Text: the row and column headings a person writes.
    Label,
    /// A formula that evaluates to an error.
    Error,
    /// A formula whose cached value disagrees with re-evaluating it ([`crate::App::stale`]).
    Stale,
}

impl CellRole {
    /// The role's name as the CLI prints it and JSON spells it — stable kebab-case.
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

    /// One character standing for the role (§4.6), so the mode works without colour: `◆` a named
    /// input, `◇` an unnamed one, `!` the unnamed constant, `=` a formula, `→` one reading another
    /// sheet, `T` a label, `✕` an error, `~` a stale value. Every role's is distinct.
    pub fn marker(self) -> &'static str {
        match self {
            CellRole::Empty => "",
            CellRole::InputNamed => "◆",
            CellRole::InputUnnamed => "◇",
            CellRole::ConstantUnnamed => "!",
            CellRole::ComputedLocal => "=",
            CellRole::ComputedCrossSheet => "→",
            CellRole::Label => "T",
            CellRole::Error => "✕",
            CellRole::Stale => "~",
        }
    }

    /// The colour of this role's marker, after the financial-modelling convention: inputs blue,
    /// formulas the ordinary ink, another sheet's a third hue. A shell resolves the ink and lifts
    /// a palette colour to be legible on its ground.
    pub fn hue(self) -> Hue {
        match self {
            CellRole::Empty => Hue::None,
            // Inputs blue; named and unnamed are one hue apart.
            CellRole::InputNamed => Hue::Palette("blue"),
            CellRole::InputUnnamed => Hue::Palette("navy"),
            CellRole::ConstantUnnamed => Hue::Palette("orange"),
            // A computed cell is black by convention, which in a dark theme is the foreground.
            CellRole::ComputedLocal => Hue::Ink,
            CellRole::ComputedCrossSheet => Hue::Palette("olive"),
            CellRole::Label => Hue::Quiet,
            CellRole::Error => Hue::Palette("red"),
            CellRole::Stale => Hue::Palette("maroon"),
        }
    }

    /// Whether this role is a diagnostic — something wrong rather than something true (§4.3).
    /// Roles get a fill or text colour, diagnostics a mark, so an ordinary model does not look
    /// like a wall of warnings.
    pub fn is_diagnostic(self) -> bool {
        matches!(
            self,
            CellRole::Error | CellRole::Stale | CellRole::ConstantUnnamed
        )
    }

    /// Every role, in this module's order, for a legend.
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

/// How a [`CellRole`]'s marker is coloured ([`CellRole::hue`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hue {
    /// Drawn as nothing at all — an empty cell.
    None,
    /// A `grind_core::style::PALETTE` colour, by its name.
    Palette(&'static str),
    /// The theme's own ink.
    Ink,
    /// The theme's ink, quieter: present, but not what you are reading — a label.
    Quiet,
}

/// Where a named expression lives: a name and the rectangle it denotes. Only a plain reference has
/// one; `[.A1]*2` denotes no place (§3.1).
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

    /// Whether this anchor covers more than one cell, drawn once at the first visible cell (§3.2).
    pub fn is_range(&self) -> bool {
        self.rows.len() > 1 || self.cols.len() > 1
    }
}

/// One document, read for what it means. Document-wide by nature, so built once and cached by
/// [`crate::App`], which drops it on every mutation. Building one costs what [`crate::App::stale`]
/// costs, a full recalculation, plus a walk of the parsed formulas.
#[derive(Clone, Debug, Default)]
pub struct Analysis {
    refs: RefIndex,
    names: Names,
    /// The role of every formula cell; a literal's role is answered on demand in
    /// [`Analysis::role`], so this is not the size of the used area.
    formulas: std::collections::BTreeMap<Address, CellRole>,
}

impl Analysis {
    pub fn build(doc: &Document) -> Self {
        let refs = RefIndex::build(doc);
        let names = Names::build(doc);
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
            names,
            formulas,
        }
    }

    /// The role of one cell; `sheet` must be the sheet `at` names. The order of the arms is the
    /// disjointness rule: a formula's role is decided at build time; then a name wins; then text
    /// is a label; then a referenced literal is a magic constant; anything left is data.
    pub fn role(&self, sheet: &Sheet, at: Address) -> CellRole {
        if let Some(role) = self.formulas.get(&at) {
            return *role;
        }
        let value = sheet.get(at.pos);
        if value.is_empty() {
            return CellRole::Empty;
        }
        if self.names.at(at).is_some() {
            return CellRole::InputNamed;
        }
        // A text literal is never a magic constant: a sum whose range overlaps its own heading
        // references the heading, and flagging that is the false positive §9 warns of.
        if matches!(value, CellValue::Text(_)) {
            return CellRole::Label;
        }
        // A magic constant takes both signals: read by address from more than one place
        // (`MAGIC_READERS`) and not one of a line of literals ([`in_a_run`]). Either alone flags
        // whole data tables.
        match self.refs.singled_out_by(at) >= MAGIC_READERS && !in_a_run(sheet, at.pos) {
            true => CellRole::ConstantUnnamed,
            false => CellRole::InputUnnamed,
        }
    }

    /// Every name anchor in the document, in declaration order.
    pub fn anchors(&self) -> &[NameAnchor] {
        self.names.anchors()
    }

    /// The anchors intersecting a rectangle — what a viewport carries.
    pub fn anchors_in(
        &self,
        sheet: usize,
        rows: Range<u32>,
        cols: Range<u32>,
    ) -> impl Iterator<Item = &NameAnchor> {
        self.names.anchors_in(sheet, rows, cols)
    }

    /// The name bound to one cell, if any.
    pub fn name_at(&self, at: Address) -> Option<&str> {
        self.names.at(at)
    }

    /// The names this was built on, for [`Names::display`] and the rest of Part I.
    pub fn names(&self) -> &Names {
        &self.names
    }

    /// The reference index this was built on (§4.4).
    pub fn refs(&self) -> &RefIndex {
        &self.refs
    }
}

/// Every named expression that denotes a place, resolved — what inline names need. One parse per
/// name, far cheaper than an [`Analysis`], for the formula bar.
#[derive(Clone, Debug, Default)]
pub struct Names {
    anchors: Vec<NameAnchor>,
}

impl Names {
    /// A name anchors only when its expression names a sheet (§3.5): `Document::names` is one flat
    /// map, so an unqualified `[.B2]` would hint on every sheet's B2. Both writers store named
    /// ranges fully qualified.
    pub fn build(doc: &Document) -> Self {
        let engine = Engine::new(doc);
        let mut anchors = Vec::new();
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
            // The far end of `[$Sheet1.$A$1:.$A$50]` is resolved from the sheet the near end
            // named.
            let Some(Area { sheet, rows, cols }) =
                engine.area(reference, Address::new(sheet, Pos::new(0, 0)))
            else {
                continue;
            };
            anchors.push(NameAnchor {
                name: name.clone(),
                sheet,
                rows,
                cols,
            });
        }
        Self { anchors }
    }

    pub fn anchors(&self) -> &[NameAnchor] {
        &self.anchors
    }

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
    pub fn at(&self, at: Address) -> Option<&str> {
        self.anchors
            .iter()
            .find(|a| a.contains(at))
            .map(|a| a.name.as_str())
    }

    /// One formula in display form with its names substituted (§3.3): `=[.B2]*[.B7]` reads
    /// `=tax_rate*subtotal`. Substitutes into the parsed expression and prints with the canonical
    /// printer, so there is no second grammar, and `from_display` already reads a bare non-cell
    /// identifier as a name. `at` is the cell the formula is in: relative references resolve from
    /// there.
    pub fn display(
        &self,
        doc: &Document,
        at: Address,
        canonical: &str,
    ) -> Result<String, SyntaxError> {
        let expr = parse(canonical)?;
        Ok(format!("={}", Bare(&self.substitute(doc, at, &expr))))
    }

    /// The same substitution, stopping at the expression; exposed because loop B checks that
    /// display form with names re-parses to this.
    pub fn substitute(&self, doc: &Document, at: Address, expr: &Expr) -> Expr {
        self.rewrite(&Engine::new(doc), at, expr)
    }

    /// The name denoting exactly this reference, resolved from `at`: by area, through
    /// [`Engine::area`], since `[$Rates.$A$1]` and `[Rates.A1]` are different text for the same
    /// cell.
    pub fn of(&self, doc: &Document, at: Address, reference: &Reference) -> Option<&str> {
        self.resolve(&Engine::new(doc), at, reference)
    }

    fn rewrite(&self, engine: &Engine, at: Address, expr: &Expr) -> Expr {
        let recur = |e: &Expr| Box::new(self.rewrite(engine, at, e));
        match expr {
            Expr::Ref(reference) => match self.resolve(engine, at, reference) {
                Some(name) => Expr::Name(name.to_owned()),
                None => expr.clone(),
            },
            Expr::Call { name, args } => Expr::Call {
                name: name.clone(),
                args: args.iter().map(|a| self.rewrite(engine, at, a)).collect(),
            },
            Expr::Prefix(op, operand) => Expr::Prefix(*op, recur(operand)),
            Expr::Postfix(op, operand) => Expr::Postfix(*op, recur(operand)),
            // The range operator is left alone: display form keeps its operands bracketed, and a
            // bare word beside `:` would read back as a reference.
            Expr::Binary(Op::Range, lhs, rhs) => Expr::Binary(Op::Range, lhs.clone(), rhs.clone()),
            Expr::Binary(op, lhs, rhs) => Expr::Binary(*op, recur(lhs), recur(rhs)),
            Expr::Paren(inner) => Expr::Paren(recur(inner)),
            other => other.clone(),
        }
    }

    fn resolve(&self, engine: &Engine, at: Address, reference: &Reference) -> Option<&str> {
        // An external reference denotes a document nothing here has open.
        if reference.source.is_some() {
            return None;
        }
        let Area { sheet, rows, cols } = engine.area(reference, at)?;
        self.anchors
            .iter()
            .find(|a| {
                a.sheet == sheet
                    && a.rows == rows
                    && a.cols == cols
                    // A cell-shaped name is never substituted: `date1` printed bare reads back as
                    // cell DATE1. The grid hint still draws it, since nothing re-parses a hint.
                    && !crate::formula::display::reads_as_reference(&a.name)
            })
            .map(|a| a.name.as_str())
    }
}

/// Whether a cell is one of a line of literals — three or more in its column or row — which is
/// what a data table looks like and a parameter does not. Without this, a column of actuals read
/// by two formula columns beside it was painted as unnamed constants. The scan stops two cells
/// each way, and a formula cell breaks a run.
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

/// Which overlays a read should compute (`doc/view-modes.md` §2), so a shell drawing neither pays
/// for neither.
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

    /// §4.6 and §9: every visible role has its own glyph, so none disappears for a reader without
    /// colour.
    #[test]
    fn every_role_a_reader_can_see_has_a_marker_of_its_own() {
        let mut seen: Vec<&str> = Vec::new();
        for role in CellRole::ALL {
            let marker = role.marker();
            if role == CellRole::Empty {
                assert!(marker.is_empty(), "the empty cell is drawn as nothing");
                continue;
            }
            assert_eq!(
                marker.chars().count(),
                1,
                "{} is not one glyph",
                role.name()
            );
            assert!(!seen.contains(&marker), "{} repeats {marker}", role.name());
            seen.push(marker);
        }
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
        // The cache is dropped in `mutate`.
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

    /// §3.3's example, end to end: the reading, and that it reads *back*.
    #[test]
    fn a_formula_reads_through_the_names_it_uses() {
        let app = App::new();
        app.enter(0, cell_of("B2"), "0.2", RecalcMode::No).unwrap();
        app.enter(0, cell_of("B7"), "500", RecalcMode::No).unwrap();
        app.enter(0, cell_of("C2"), "=[.B2]*[.B7]", RecalcMode::Document)
            .unwrap();
        app.set_name("tax_rate", "[$Sheet1.$B$2]").unwrap();
        app.set_name("subtotal", "[$Sheet1.$B$7]").unwrap();
        assert_eq!(
            app.named_formula(0, cell_of("C2")).unwrap().as_deref(),
            Some("=tax_rate*subtotal")
        );
        // And it is not a one-way rendering: the reverse direction already exists, because
        // `from_display` resolves a bare identifier that is not cell-shaped as a name.
        assert_eq!(
            crate::formula::display::from_display("=tax_rate*subtotal").unwrap(),
            "=tax_rate*subtotal"
        );
    }

    /// The name is stored `[$Sheet1.$B$2]` and the formula says `[.B2]`. Comparing the two
    /// spellings finds nothing; comparing the rectangles they resolve to finds this.
    #[test]
    fn substitution_matches_on_the_area_rather_than_on_the_spelling() {
        let app = App::new();
        app.enter(0, cell_of("B2"), "1", RecalcMode::No).unwrap();
        app.enter(0, cell_of("B3"), "1", RecalcMode::No).unwrap();
        app.enter(0, cell_of("D1"), "=[.B2]+[.B3]", RecalcMode::Document)
            .unwrap();
        app.set_name("rate", "[$Sheet1.$B$2]").unwrap();
        // Only the reference that denotes exactly what the name denotes is replaced.
        assert_eq!(
            app.named_formula(0, cell_of("D1")).unwrap().as_deref(),
            Some("=rate+B3")
        );
    }

    /// A name over a range names the *whole* rectangle and nothing inside it — `SUM(sales)`
    /// where the range matches, `SUM(A1:A3)` where it does not.
    #[test]
    fn a_range_name_stands_for_the_whole_rectangle_and_not_for_a_cell_in_it() {
        let app = App::new();
        for row in 0..5 {
            app.enter(0, Pos::new(row, 0), "1", RecalcMode::No).unwrap();
        }
        app.enter(0, cell_of("C1"), "=SUM([.A1:.A5])", RecalcMode::Document)
            .unwrap();
        app.enter(0, cell_of("C2"), "=SUM([.A1:.A3])", RecalcMode::Document)
            .unwrap();
        app.enter(0, cell_of("C3"), "=[.A1]", RecalcMode::Document)
            .unwrap();
        app.set_name("sales", "[$Sheet1.$A$1:.$A$5]").unwrap();
        let named = |cell: &str| app.named_formula(0, cell_of(cell)).unwrap().unwrap();
        assert_eq!(named("C1"), "=SUM(sales)");
        assert_eq!(named("C2"), "=SUM(A1:A3)");
        assert_eq!(named("C3"), "=A1");
    }

    /// A name is anchored, so the same *text* in two rows is two different cells and only
    /// one of them is the name — which is why the substitution takes an address.
    #[test]
    fn a_relative_reference_is_resolved_from_where_it_sits() {
        let app = App::new();
        app.enter(0, cell_of("A1"), "1", RecalcMode::No).unwrap();
        app.enter(0, cell_of("A2"), "2", RecalcMode::No).unwrap();
        app.enter(0, cell_of("B1"), "=[.A1]", RecalcMode::Document)
            .unwrap();
        app.enter(0, cell_of("B2"), "=[.A2]", RecalcMode::Document)
            .unwrap();
        app.set_name("first", "[$Sheet1.$A$1]").unwrap();
        assert_eq!(
            app.named_formula(0, cell_of("B1")).unwrap().as_deref(),
            Some("=first")
        );
        assert_eq!(
            app.named_formula(0, cell_of("B2")).unwrap().as_deref(),
            Some("=A2")
        );
    }

    /// §3.1: a computed name denotes no place, so nothing is ever printed as it.
    #[test]
    fn a_computed_name_is_never_substituted_into_a_formula() {
        let app = App::new();
        app.enter(0, cell_of("A1"), "1", RecalcMode::No).unwrap();
        app.enter(0, cell_of("B1"), "=[.A1]", RecalcMode::Document)
            .unwrap();
        app.set_name("doubled", "[$Sheet1.$A$1]*2").unwrap();
        assert_eq!(
            app.named_formula(0, cell_of("B1")).unwrap().as_deref(),
            Some("=A1")
        );
    }

    /// The range **operator** keeps its operands, because unbracketed a bare word either
    /// side of a `:` scans back as a reference rather than as a name.
    #[test]
    fn the_range_operator_keeps_its_references() {
        let app = App::new();
        app.add_sheet("Sheet2").unwrap();
        app.enter(1, cell_of("C22"), "1", RecalcMode::No).unwrap();
        app.enter(0, cell_of("C33"), "1", RecalcMode::No).unwrap();
        app.enter(
            0,
            cell_of("A1"),
            "=SUM([Sheet2.C22]:[.C33])",
            RecalcMode::Document,
        )
        .unwrap();
        app.set_name("corner", "[$Sheet1.$C$33]").unwrap();
        let shown = app.named_formula(0, cell_of("A1")).unwrap().unwrap();
        assert_eq!(shown, "=SUM([Sheet2.C22]:[.C33])");
        // Which is the point: what it prints reads back unchanged.
        assert_eq!(
            crate::formula::display::from_display(&shown).unwrap(),
            "=SUM([Sheet2.C22]:[.C33])"
        );
    }

    /// A **cell-shaped** name is anchored but never substituted — the corpus found this,
    /// and it is the `LOG10` collision from the other side.
    #[test]
    fn a_cell_shaped_name_is_hinted_but_never_printed_into_a_formula() {
        // Built by hand because `App::set_name` refuses to create such a name; a document
        // written elsewhere can and does.
        let mut doc = Document::default();
        doc.names
            .insert("date1".to_owned(), "[$Sheet1.$G$1]".to_owned());
        let names = Names::build(&doc);
        let at = Address::new(0, Pos::new(4, 0));
        // The anchor exists, so the grid draws the hint: nothing re-parses a hint.
        assert_eq!(names.at(Address::new(0, Pos::new(0, 6))), Some("date1"));
        // The formula reading does not, because `DATE1` reads back as a cell address.
        assert_eq!(
            names
                .display(&doc, at, "of:=DAYS360([.G$1];[.G$2])")
                .unwrap(),
            "=DAYS360(G$1;G$2)"
        );
    }

    /// A cell that holds no formula has no reading, and a document with no names reads the
    /// same as `input_text` — the overlay is off by being empty rather than by a flag.
    #[test]
    fn a_document_with_no_names_reads_exactly_as_display_form_does() {
        let app = App::new();
        app.enter(0, cell_of("A1"), "1", RecalcMode::No).unwrap();
        app.enter(0, cell_of("B1"), "=[.A1]*2", RecalcMode::Document)
            .unwrap();
        assert_eq!(
            app.named_formula(0, cell_of("B1")).unwrap(),
            Some(app.input_text(0, cell_of("B1")).unwrap())
        );
        assert_eq!(app.named_formula(0, cell_of("A1")).unwrap(), None);
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

    /// Every role that is drawn has a colour, and every palette colour named is one.
    #[test]
    fn every_drawn_role_has_a_hue_the_palette_knows() {
        for role in CellRole::ALL {
            match role.hue() {
                Hue::None => assert_eq!(role, CellRole::Empty),
                Hue::Palette(name) => {
                    assert!(grind_core::style::palette(name).is_some(), "{name}")
                }
                Hue::Ink | Hue::Quiet => {}
            }
        }
    }
}
