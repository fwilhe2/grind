// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Conditional formatting's one rule type — `doc/conditional-format.md`, normative.
//!
//! *This formula is true here, so draw the cell this way.* A [`Rule`] is a condition in
//! OpenFormula's own syntax, a set of ranges it applies to, the cell its relative references
//! are written from, and the [`CellStyle`] it draws **over** a cell's own while it holds. A
//! sheet keeps an ordered list of them, and the first that holds at a cell is the one drawn
//! (`doc/ods-format.md` §3.6, Q4, measured).
//!
//! **Nothing here is ever stored about a cell.** Whether a rule holds is derived as the sheet
//! is drawn ([`applied`]), the way `doc/view-modes.md` derives a role, so the answer cannot go
//! stale and no shell evaluates a formula of its own.
//!
//! The file's spelling is ODF's standard one: a `style:map` with `is-true-formula(…)` and a
//! `style:base-cell-address` on every cell style of every cell in the range, applying a named
//! style from `office:styles` (`doc/ods-format.md` §3.6). LibreOffice's `calcext:` copy is
//! never needed to read a rule, and never written.

use serde::{Deserialize, Serialize};

use crate::formula::eval::{Address, Engine};
use crate::formula::parse::{Expr, parse};
use crate::formula::shift::shift;
use crate::model::{Document, Pos};
use crate::style::CellStyle;

/// One rule: while `condition` holds at a cell of `ranges`, `style` is drawn over the cell's
/// own.
///
/// **Several ranges, one base**, which is the shape both formats give a rule: Excel's `sqref`
/// is a list of ranges with one formula, and ODF's `style:base-cell-address` is one cell
/// however many ranges carry the map. A rule over ten ranges is one rule, as it was to whoever
/// wrote it, and `condition` is written from `base` and moved to each cell from there — the
/// way a fill moves a formula ([`shift`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    /// The cells it applies to, each range top-left and bottom-right inclusive.
    pub ranges: Vec<(Pos, Pos)>,
    /// The cell `condition` is written from — its relative references are relative to this.
    pub base: Pos,
    /// OpenFormula in ODF syntax, with no `of:=` intro: `[.$E5]=TRUE()`.
    pub condition: String,
    /// What it draws. Only what this sets is drawn over the cell ([`CellStyle::over`]).
    pub style: CellStyle,
}

impl Rule {
    /// A rule over one range, written from its top-left cell — what a person means by "this
    /// range, this condition".
    pub fn over(start: Pos, end: Pos, condition: impl Into<String>, style: CellStyle) -> Self {
        Rule {
            ranges: vec![(start, end)],
            base: start,
            condition: condition.into(),
            style,
        }
    }

    /// Whether `pos` is in one of its ranges.
    pub fn covers(&self, pos: Pos) -> bool {
        self.ranges
            .iter()
            .any(|(start, end)| within(pos, *start, *end))
    }

    /// Whether any of its ranges shares a cell with `start:end`.
    pub fn touches(&self, start: Pos, end: Pos) -> bool {
        self.ranges.iter().any(|(a, b)| {
            a.row <= end.row && start.row <= b.row && a.col <= end.col && start.col <= b.col
        })
    }

    /// The condition as it reads at `pos`: parsed, and every relative reference moved by the
    /// distance from [`Rule::base`]. `None` when it does not parse — such a rule never holds.
    pub fn condition_at(&self, pos: Pos) -> Option<Expr> {
        let expr = parse(&self.condition).ok()?;
        Some(self.moved(&expr, pos))
    }

    fn moved(&self, expr: &Expr, pos: Pos) -> Expr {
        let rows = i64::from(pos.row) - i64::from(self.base.row);
        let cols = i64::from(pos.col) - i64::from(self.base.col);
        match (rows, cols) {
            (0, 0) => expr.clone(),
            _ => shift(expr, rows, cols),
        }
    }

    /// Whether the condition holds at `pos` of `sheet`, asked of `engine`. A value that is not
    /// a logical is converted the way §6.3.6 converts one, and an error — or a condition that
    /// does not parse — is false: a rule that cannot be evaluated draws nothing.
    pub fn holds(&self, engine: &mut Engine, sheet: usize, pos: Pos) -> bool {
        let Some(expr) = self.condition_at(pos) else {
            return false;
        };
        let at = Address::new(sheet, pos);
        engine.value_of(&expr, at).to_logical().unwrap_or(false)
    }
}

fn within(pos: Pos, start: Pos, end: Pos) -> bool {
    (start.row..=end.row).contains(&pos.row) && (start.col..=end.col).contains(&pos.col)
}

/// The style the first rule holding at `pos` draws, if any rule covering it holds.
///
/// `engine` is the caller's so that a screenful of cells shares one cache; it should be
/// [`Engine::over_cached`], since a rule is about what the sheet shows.
pub fn applied<'d>(
    doc: &'d Document,
    engine: &mut Engine,
    sheet: usize,
    pos: Pos,
) -> Option<&'d CellStyle> {
    doc.sheet(sheet)?
        .rules()
        .iter()
        .filter(|rule| rule.covers(pos))
        .find(|rule| rule.holds(engine, sheet, pos))
        .map(|rule| &rule.style)
}

/// A rule's condition as ODF spells it on a `style:map` — `is-true-formula(…)`, the one
/// spelling this build writes (`doc/ods-format.md` §3.6).
pub fn odf_condition(condition: &str) -> String {
    format!("is-true-formula({})", strip_intro(condition))
}

fn strip_intro(condition: &str) -> &str {
    let c = condition.trim();
    let c = c.strip_prefix("of:").unwrap_or(c);
    c.strip_prefix('=').unwrap_or(c)
}

/// A `style:condition` read off a cell style's `style:map` → the model's condition, written
/// from `base`.
///
/// `is-true-formula(…)` is the rule itself. The value comparisons — `cell-content()>5`,
/// `cell-content-is-between(1,5)` — are each a formula about the cell, so they are read *as*
/// one, with the cell spelled as a relative reference to `base` (`doc/conditional-format.md`
/// §1: the model has no second vocabulary for what the first can say). Anything else —
/// `cell-content-is-whole-number()`, a condition this build cannot name — is `None`, and the
/// file's own map is kept where a save does not touch it.
pub fn from_odf_condition(condition: &str, base: Pos) -> Option<String> {
    let condition = condition.trim();
    if let Some(inner) = call(condition, "is-true-formula") {
        let inner = strip_intro(inner);
        parse(inner).ok()?;
        return Some(inner.to_owned());
    }
    let me = format!("[.{}]", crate::a1::format(None, base));
    for (name, negated) in [
        ("cell-content-is-between", false),
        ("cell-content-is-not-between", true),
    ] {
        if let Some(inner) = call(condition, name) {
            let (low, high) = split_pair(inner)?;
            parse(low).ok()?;
            parse(high).ok()?;
            return Some(match negated {
                false => format!("AND({me}>={low};{me}<={high})"),
                true => format!("OR({me}<{low};{me}>{high})"),
            });
        }
    }
    let rest = condition.strip_prefix("cell-content()")?.trim_start();
    // Longest operator first, so `<=` is not read as `<` and `=…`.
    let op = ["<=", ">=", "!=", "<", ">", "="]
        .into_iter()
        .find(|op| rest.starts_with(op))?;
    let value = rest[op.len()..].trim();
    parse(value).ok()?;
    let op = if op == "!=" { "<>" } else { op };
    Some(format!("{me}{op}{value}"))
}

/// An order of `n` rules honouring every `(a, b)` in `before` — `a` is tried before `b` — and
/// otherwise the order they are numbered in. What a file states is the order of the maps on each
/// cell style, a partial order, and this is the one total order a reader can derive from it. A
/// cycle (two styles disagreeing) is broken by numbering, which is all a reader can do.
pub fn priority(n: usize, before: &[(usize, usize)]) -> Vec<usize> {
    let mut pending: Vec<usize> = vec![0; n];
    for (_, b) in before {
        pending[*b] += 1;
    }
    let mut done = vec![false; n];
    let mut order = Vec::with_capacity(n);
    while order.len() < n {
        let next = (0..n)
            .find(|i| !done[*i] && pending[*i] == 0)
            .or_else(|| (0..n).find(|i| !done[*i]))
            .expect("fewer than n placed");
        done[next] = true;
        order.push(next);
        for (a, b) in before {
            if *a == next && pending[*b] > 0 {
                pending[*b] -= 1;
            }
        }
    }
    order
}

/// Rectangles of cells — in the order they were met — joined where they can be: side by side
/// on the same rows, then one above the other on the same columns. What a file spells as one
/// run of cells per row comes back as the range somebody chose.
pub fn rectangles(mut rects: Vec<(Pos, Pos)>) -> Vec<(Pos, Pos)> {
    let join = |rects: &mut Vec<(Pos, Pos)>, beside: fn(&(Pos, Pos), &(Pos, Pos)) -> bool| {
        let mut out: Vec<(Pos, Pos)> = Vec::with_capacity(rects.len());
        for rect in rects.drain(..) {
            match out.iter_mut().find(|r| beside(r, &rect)) {
                Some(r) => {
                    r.1 = Pos::new(r.1.row.max(rect.1.row), r.1.col.max(rect.1.col));
                    r.0 = Pos::new(r.0.row.min(rect.0.row), r.0.col.min(rect.0.col));
                }
                None => out.push(rect),
            }
        }
        *rects = out;
    };
    // Same rows, touching columns.
    join(&mut rects, |a, b| {
        a.0.row == b.0.row
            && a.1.row == b.1.row
            && (b.0.col <= a.1.col.saturating_add(1) && a.0.col <= b.1.col.saturating_add(1))
    });
    // Same columns, touching rows — repeated until nothing more joins, since a join can make a
    // rectangle that now touches one already passed.
    loop {
        let before = rects.len();
        join(&mut rects, |a, b| {
            a.0.col == b.0.col
                && a.1.col == b.1.col
                && (b.0.row <= a.1.row.saturating_add(1) && a.0.row <= b.1.row.saturating_add(1))
        });
        if rects.len() == before {
            break;
        }
    }
    rects
}

// ---------------------------------------------------------------------------------------------
// Editing a rule from a window — the half every shell's dialog is a face for.
//
// Five shells grew a rule editor at once, and each would otherwise have had its own idea of what
// "light red fill" writes, how `=B2>0` becomes ODF, and how a rule reads in a list. So the
// choices are data here and a shell decides only where to put them (`doc/conditional-format.md`
// §4). The CLI keeps its own flags, which can say more than any of these looks; a rule written
// by one is read by the other like any other rule.
// ---------------------------------------------------------------------------------------------

/// What a rule draws, as a window offers it: one choice from a short list rather than a dialog of
/// every [`CellStyle`] property. The three fills carry a text colour with them, chosen to read on
/// that fill, since a rule drawn over a cell must stay legible whatever the cell's own colour was.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Look {
    RedFill,
    YellowFill,
    GreenFill,
    BlueFill,
    RedText,
    GreenText,
    Bold,
    Strike,
}

impl Look {
    pub const ALL: [Look; 8] = [
        Look::RedFill,
        Look::YellowFill,
        Look::GreenFill,
        Look::BlueFill,
        Look::RedText,
        Look::GreenText,
        Look::Bold,
        Look::Strike,
    ];

    /// What a picker calls it, in sentence case.
    pub fn label(self) -> &'static str {
        match self {
            Look::RedFill => "Light red fill",
            Look::YellowFill => "Yellow fill",
            Look::GreenFill => "Green fill",
            Look::BlueFill => "Blue fill",
            Look::RedText => "Red text",
            Look::GreenText => "Green text",
            Look::Bold => "Bold",
            Look::Strike => "Strikethrough",
        }
    }

    /// One word for it, for a shell whose verbs are typed: `:rule red =B2>0`.
    pub fn word(self) -> &'static str {
        match self {
            Look::RedFill => "red",
            Look::YellowFill => "yellow",
            Look::GreenFill => "green",
            Look::BlueFill => "blue",
            Look::RedText => "red-text",
            Look::GreenText => "green-text",
            Look::Bold => "bold",
            Look::Strike => "strike",
        }
    }

    /// The look a [`Look::word`] names, case-insensitively.
    pub fn from_word(word: &str) -> Option<Look> {
        Look::ALL
            .into_iter()
            .find(|look| look.word().eq_ignore_ascii_case(word.trim()))
    }

    /// The style it draws over a cell.
    pub fn style(self) -> CellStyle {
        let fill = |background: &str, color: &str| CellStyle {
            background: Some(background.to_owned()),
            color: Some(color.to_owned()),
            ..CellStyle::default()
        };
        let ink = |color: &str| CellStyle {
            color: Some(color.to_owned()),
            ..CellStyle::default()
        };
        match self {
            Look::RedFill => fill("#ffc7ce", "#9c0006"),
            Look::YellowFill => fill("#ffeb9c", "#9c5700"),
            Look::GreenFill => fill("#c6efce", "#006100"),
            Look::BlueFill => fill("#bdd7ee", "#1f3864"),
            Look::RedText => ink("#c00000"),
            Look::GreenText => ink("#007a33"),
            Look::Bold => CellStyle {
                font_weight: Some("bold".to_owned()),
                ..CellStyle::default()
            },
            Look::Strike => CellStyle {
                line_through: Some("solid".to_owned()),
                ..CellStyle::default()
            },
        }
    }

    /// Which look `style` is, when it is exactly one of them.
    pub fn of(style: &CellStyle) -> Option<Look> {
        Look::ALL.into_iter().find(|look| look.style() == *style)
    }
}

/// A condition as somebody typed it — display syntax, `=B2>0` or just `B2>0`, written from the
/// range's top-left cell — as the ODF a [`Rule`] stores, `[.B2]>0`. The same conversion a cell's
/// formula goes through ([`crate::formula::display::from_display`]), so a condition that would not
/// parse as a cell's formula is refused with the same message and the same offset.
pub fn condition_from_display(text: &str) -> Result<String, crate::formula::display::DisplayError> {
    let text = text.trim();
    let body = text.strip_prefix('=').unwrap_or(text);
    let odf = crate::formula::display::from_display(&format!("={body}"))?;
    Ok(odf.strip_prefix('=').unwrap_or(&odf).to_owned())
}

/// A rule's condition as a window shows it: display syntax with its `=`. A condition that does
/// not parse — read from a file this build cannot read all of — is shown as it is stored.
pub fn condition_to_display(rule: &Rule) -> String {
    crate::formula::display::to_display(strip_intro(&rule.condition))
        .unwrap_or_else(|_| rule.condition.clone())
}

/// A rule's ranges as A1 text: `B2:B9 D2`.
pub fn ranges_text(rule: &Rule) -> String {
    rule.ranges
        .iter()
        .map(|&(a, b)| range_hint(a, b))
        .collect::<Vec<_>>()
        .join(" ")
}

/// What a rule draws, in words: a [`Look`]'s label, or the properties it sets.
pub fn look_text(style: &CellStyle) -> String {
    if let Some(look) = Look::of(style) {
        return look.label().to_owned();
    }
    let mut out = Vec::new();
    if style.font_weight.as_deref() == Some("bold") {
        out.push("bold".to_owned());
    }
    if style.font_style.as_deref() == Some("italic") {
        out.push("italic".to_owned());
    }
    if style.is_underlined() {
        out.push("underline".to_owned());
    }
    if style.is_struck() {
        out.push("strikethrough".to_owned());
    }
    if let Some(color) = &style.color {
        out.push(format!("text {color}"));
    }
    if let Some(background) = &style.background {
        out.push(format!("fill {background}"));
    }
    if let Some(size) = &style.font_size {
        out.push(size.clone());
    }
    match out.is_empty() {
        true => "other formatting".to_owned(),
        false => out.join(", "),
    }
}

/// One rule as one line of a list: `B2:B9   =B2>0   Light red fill`, tab-separated so a shell
/// that lays it out in columns can split it and one that cannot shows it as it is.
pub fn summary(rule: &Rule) -> String {
    format!(
        "{}\t{}\t{}",
        ranges_text(rule),
        condition_to_display(rule),
        look_text(&rule.style)
    )
}

/// A rule from what a dialog's fields hold, added after every rule the sheet has: `ranges` as
/// A1 text (`B2:B9`, or several split by spaces, commas or semicolons — on `sheet` unless one
/// names its own), a condition in display syntax written from the first range's top-left cell,
/// and a look. Returns the sheet the rule went on and its index there; the `Err` is a sentence
/// a dialog can show as it stands.
pub fn add_from_input(
    app: &crate::App,
    sheet: usize,
    ranges: &str,
    condition: &str,
    look: Look,
) -> Result<(usize, usize), String> {
    let mut on = None;
    let mut rects = Vec::new();
    for piece in ranges
        .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
        .filter(|p| !p.is_empty())
    {
        let reference = crate::a1::parse(piece).map_err(|e| format!("{piece}: {e}"))?;
        let (s, start, end) =
            crate::a1::resolve_in(app, sheet, &reference).map_err(|e| format!("{piece}: {e}"))?;
        if on.is_some_and(|first| first != s) {
            return Err("A rule's ranges must all be on one sheet".to_owned());
        }
        on = Some(s);
        rects.push((start, end));
    }
    let (Some(on), Some(&(base, _))) = (on, rects.first()) else {
        return Err("A rule needs a range to apply to".to_owned());
    };
    if condition.trim().trim_start_matches('=').trim().is_empty() {
        return Err("A rule needs a condition: a formula that is true where it applies".to_owned());
    }
    let condition = condition_from_display(condition)
        .map_err(|e| format!("The condition is not a formula: {}", e.message))?;
    let rule = Rule {
        ranges: rects,
        base,
        condition,
        style: look.style(),
    };
    app.add_rule(on, rule)
        .map(|index| (on, index))
        .map_err(|e| e.to_string())
}

/// What a new rule's condition field starts with: the selection's top-left cell compared with
/// nothing yet, so the one thing left to type is what it is compared with.
pub fn condition_hint(start: Pos) -> String {
    format!("={}>", crate::a1::format(None, start))
}

/// What a new rule's range field starts with: the selection.
pub fn range_hint(start: Pos, end: Pos) -> String {
    match start == end {
        true => crate::a1::format(None, start),
        false => format!(
            "{}:{}",
            crate::a1::format(None, start),
            crate::a1::format(None, end)
        ),
    }
}

/// The argument text of `name(…)`, when `text` is exactly one call of it.
fn call<'t>(text: &'t str, name: &str) -> Option<&'t str> {
    text.strip_prefix(name)?
        .trim_start()
        .strip_prefix('(')?
        .strip_suffix(')')
}

/// `a,b` split at its one top-level comma — outside quotes and brackets.
fn split_pair(text: &str) -> Option<(&str, &str)> {
    let mut depth = 0i32;
    let mut quoted = false;
    for (i, c) in text.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '(' | '[' | '{' if !quoted => depth += 1,
            ')' | ']' | '}' if !quoted => depth -= 1,
            ',' if !quoted && depth == 0 => {
                return Some((text[..i].trim(), text[i + 1..].trim()));
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CellValue, Sheet};

    fn green() -> CellStyle {
        CellStyle {
            background: Some("#97e8ca".into()),
            ..CellStyle::default()
        }
    }

    #[test]
    fn a_relative_reference_moves_from_the_base_and_an_absolute_one_does_not() {
        let rule = Rule::over(Pos::new(4, 1), Pos::new(6, 3), "[.$E5]=TRUE()", green());
        let at = rule.condition_at(Pos::new(6, 3)).unwrap();
        assert_eq!(at.to_string(), "[.$E7]=TRUE()");
        assert!(rule.covers(Pos::new(5, 2)));
        assert!(!rule.covers(Pos::new(7, 2)));
    }

    #[test]
    fn the_first_rule_that_holds_is_the_one_drawn() {
        let mut doc = Document::default();
        let mut sheet = Sheet::new("Sheet1");
        sheet.set(Pos::new(0, 0), CellValue::Number(5.0));
        let red = CellStyle {
            background: Some("#ff0000".into()),
            ..CellStyle::default()
        };
        sheet.set_rules(vec![
            Rule::over(Pos::new(0, 0), Pos::new(0, 0), "[.A1]>0", red.clone()),
            Rule::over(Pos::new(0, 0), Pos::new(0, 0), "[.A1]>1", green()),
        ]);
        doc.sheets = vec![sheet];
        let mut engine = Engine::over_cached(&doc);
        assert_eq!(applied(&doc, &mut engine, 0, Pos::new(0, 0)), Some(&red));
        assert_eq!(applied(&doc, &mut engine, 0, Pos::new(1, 0)), None);
    }

    #[test]
    fn an_error_or_a_condition_that_does_not_parse_draws_nothing() {
        let mut doc = Document::default();
        let mut sheet = Sheet::new("Sheet1");
        sheet.set_rules(vec![
            Rule::over(Pos::new(0, 0), Pos::new(0, 0), "1/0", green()),
            Rule::over(Pos::new(0, 0), Pos::new(0, 0), "((", green()),
        ]);
        doc.sheets = vec![sheet];
        let mut engine = Engine::over_cached(&doc);
        assert_eq!(applied(&doc, &mut engine, 0, Pos::new(0, 0)), None);
    }

    #[test]
    fn the_value_comparisons_are_read_as_formulas_about_the_cell() {
        let b2 = Pos::new(1, 1);
        assert_eq!(
            from_odf_condition("is-true-formula([.$A2]>0)", b2).as_deref(),
            Some("[.$A2]>0")
        );
        assert_eq!(
            from_odf_condition("cell-content()>=5", b2).as_deref(),
            Some("[.B2]>=5")
        );
        assert_eq!(
            from_odf_condition("cell-content()!=\"x\"", b2).as_deref(),
            Some("[.B2]<>\"x\"")
        );
        assert_eq!(
            from_odf_condition("cell-content-is-between(1,[.$C$1])", b2).as_deref(),
            Some("AND([.B2]>=1;[.B2]<=[.$C$1])")
        );
        assert_eq!(
            from_odf_condition("cell-content-is-not-between(1,5)", b2).as_deref(),
            Some("OR([.B2]<1;[.B2]>5)")
        );
        assert_eq!(
            from_odf_condition("cell-content-is-whole-number()", b2),
            None
        );
        assert_eq!(odf_condition("of:=[.A1]>0"), "is-true-formula([.A1]>0)");
    }

    #[test]
    fn a_condition_typed_in_display_syntax_is_stored_in_odf_and_shown_back() {
        assert_eq!(
            condition_from_display("=B2>0").unwrap(),
            condition_from_display(" B2>0").unwrap()
        );
        let odf = condition_from_display("=AND($E5; b2<>\"x\")").unwrap();
        assert_eq!(odf, "AND([.$E5];[.B2]<>\"x\")");
        assert!(condition_from_display("=B2>").is_err());
        let rule = Rule::over(Pos::new(1, 1), Pos::new(8, 1), odf, Look::RedFill.style());
        assert_eq!(condition_to_display(&rule), "=AND($E5;B2<>\"x\")");
        assert_eq!(summary(&rule), "B2:B9\t=AND($E5;B2<>\"x\")\tLight red fill");
    }

    #[test]
    fn a_rule_is_added_from_a_dialogs_fields_or_refused_in_a_sentence() {
        let app = crate::App::new();
        assert_eq!(
            add_from_input(&app, 0, "B2:B9, D2", "=B2>0", Look::GreenFill),
            Ok((0, 0))
        );
        let rules = app.rules(0).unwrap();
        assert_eq!(ranges_text(&rules[0]), "B2:B9 D2");
        assert_eq!(rules[0].base, Pos::new(1, 1));
        assert_eq!(rules[0].condition, "[.B2]>0");
        assert_eq!(Look::of(&rules[0].style), Some(Look::GreenFill));
        assert!(add_from_input(&app, 0, "", "=B2>0", Look::Bold).is_err());
        assert!(add_from_input(&app, 0, "B2", "=", Look::Bold).is_err());
        assert!(
            add_from_input(&app, 0, "B2", &condition_hint(Pos::new(1, 1)), Look::Bold).is_err()
        );
        assert!(add_from_input(&app, 0, "B2:", "=B2>0", Look::Bold).is_err());
        assert_eq!(app.rules(0).unwrap().len(), 1);
    }

    #[test]
    fn every_look_draws_something_and_is_recognised_as_itself() {
        for look in Look::ALL {
            assert!(!look.style().is_plain(), "{look:?}");
            assert_eq!(Look::of(&look.style()), Some(look));
            assert_eq!(Look::from_word(&look.word().to_uppercase()), Some(look));
        }
        assert_eq!(look_text(&green()), "fill #97e8ca");
    }
}
