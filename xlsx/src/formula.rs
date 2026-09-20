// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Excel's A1 expression syntax → [`grind_sheet::formula::parse::Expr`], the AST the ODF parser
//! builds.
//!
//! **Parsed, never rewritten with string surgery.** A translated formula is one our own parser
//! could have produced — it is built as a tree and printed by the canonical serialiser — or it
//! is not translated at all. That is what makes the output a formula rather than a string that
//! resembles one, and it is why `[.A1]` is never spelled here: the brackets, the leading dot,
//! the `;` separators and §5.8's inherited sheet all come out of `Display for Expr`.
//!
//! `doc/xlsx-import.md` Part II §3 is the table this file implements. The differences that
//! matter, in the order a formula meets them:
//!
//! | Excel | OpenFormula |
//! |---|---|
//! | `A1`, `$A$1`, `Sheet1!A1`, `'My Sheet'!A1` | `[.A1]`, `[.$A$1]`, `[Sheet1.A1]`, `['My Sheet'.A1]` |
//! | `Sheet1:Sheet3!A1` | `[Sheet1.A1:Sheet3.A1]` — §4.8's cuboid |
//! | `A:A`, `1:1` | `[.A:.A]`, `[.1:.1]` |
//! | `,` between arguments | `;` (§5.6) |
//! | `TRUE`, `FALSE` | `TRUE()`, `FALSE()` (§6.15) |
//! | `_xlfn.XLOOKUP` | `XLOOKUP` — the prefix is Excel's own marker for a function older
//!   readers had not got, and the name under it is the name |
//! | `@A1`, `_xlfn.SINGLE(A1)` | `[.A1]` — the implicit-intersection marker, dropped |
//!
//! **What is deliberately not translated is semantics.** `CEILING`, `FLOOR`, `MOD` with
//! negative operands and `ROUND`'s tie rule differ between Excel and OpenFormula *under the
//! same name*. Nothing here renames or rewrites them: the name is carried and so is Excel's
//! cached value. Recalculating then applies ODF's rule, which is the correct behaviour for an
//! ODF document and a change the user chooses.
//!
//! Everything this file cannot carry is a [`Refusal`] — a **named class**, never a file name
//! and never a silent pass-through. The cell keeps Excel's cached value and loses its formula,
//! which is counted.

use grind_sheet::formula::lex::{Axis, CellRef, Op, Reference};
use grind_sheet::formula::parse::Expr;
use grind_sheet::formula::value::FormulaError;
use grind_sheet::{MAX_COLS, MAX_ROWS};

use crate::report::Dropped;

/// Why a formula was not translated.
///
/// Each variant is a *class* of expression, which is the whole point: an exclusion with a name
/// can be counted, explained and looked for, where "this one failed" cannot. [`Refusal::Syntax`]
/// is the only open-ended one, and a formula that lands there is either malformed or a
/// construct this table has not learned to name yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Refusal {
    /// `<f t="array">` — an array formula (§2.3.2). The one class [`translate`] never returns:
    /// it is a fact about the *cell*, decided before the expression is looked at, and it is
    /// here so that every formula this filter loses has a class.
    Array,
    /// `{1,2;3,4}` — an inline array constant (§5.13, excluded by §2.3.2). Both of Excel's
    /// separators change meaning in ODF at once, and nothing evaluates an array anyway.
    InlineArray,
    /// `Table1[Column]`, `Sales[[#This Row],[Amount]]` — a structured reference. It names a
    /// table's column rather than a range, and the model has no tables.
    StructuredReference,
    /// `[1]Sheet1!A1` — a reference into another workbook. Never followed: a link resolves to
    /// a path chosen by whoever sent the file.
    ExternalLink,
    /// A space between two references — Excel's intersection operator. OpenFormula spells it
    /// `!` and could hold it, so this is excluded by choice rather than by inability: §2.3.2
    /// leaves both reference operators out of the Small Group.
    Intersection,
    /// A comma between two references inside parentheses — Excel's union operator, spelled `~`
    /// in OpenFormula and excluded for the same reason.
    Union,
    /// Nothing this grammar recognises.
    Syntax,
}

impl Refusal {
    /// The [`Dropped`] kind this refusal counts as, where one exists.
    ///
    /// Only two classes name a *construct the model has no home for*, which is `Dropped`'s
    /// admission rule. The others are expressions ODF could hold and this build does not
    /// evaluate, so they are a loss of a formula and nothing more — [`crate::Report::untranslated`]
    /// is where that is recorded.
    pub fn dropped(self) -> Option<Dropped> {
        match self {
            Refusal::Array => Some(Dropped::ArrayFormula),
            Refusal::StructuredReference => Some(Dropped::StructuredReference),
            Refusal::ExternalLink => Some(Dropped::ExternalLink),
            Refusal::InlineArray | Refusal::Intersection | Refusal::Union | Refusal::Syntax => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Refusal::Array => "array formula",
            Refusal::InlineArray => "inline array",
            Refusal::StructuredReference => "structured reference",
            Refusal::ExternalLink => "external link",
            Refusal::Intersection => "intersection operator",
            Refusal::Union => "union operator",
            Refusal::Syntax => "unreadable expression",
        }
    }
}

/// `<f>`'s text → an expression, or the class that stopped it.
///
/// The leading `=` a hand-written formula may carry is accepted; `<f>` itself never has one.
pub fn translate(src: &str) -> Result<Expr, Refusal> {
    let chars: Vec<char> = src
        .trim()
        .strip_prefix('=')
        .unwrap_or(src.trim())
        .chars()
        .collect();
    if chars.is_empty() {
        return Err(Refusal::Syntax);
    }
    let mut p = Parser { src: &chars, at: 0 };
    let expr = p.expr(0)?;
    p.space();
    match p.at == p.src.len() {
        true => Ok(expr),
        false => Err(Refusal::Syntax),
    }
}

// ---- binding powers ----
//
// §5.5's Table 1, which Excel's own precedence agrees with, including both of its surprises:
// prefix `-` binds **tighter** than `^` (`-2^2` is 4) and `^` is left-associative (`2^3^2` is
// 64). They are spelled here rather than borrowed from `formula::parse` because that module's
// are `pub(crate)` — the table is ODF's, the copy is this crate's, and one test holds the two
// together by parsing the same expressions both ways.

const CMP_BP: u8 = 10;
const CONCAT_BP: u8 = 20;
const ADD_BP: u8 = 30;
const MUL_BP: u8 = 40;
const POW_BP: u8 = 50;
const PREFIX_BP: u8 = 70;
/// `:` as a binary operator, for the one shape [`Parser::reference`] cannot swallow whole:
/// a range whose far end is computed, `H2:INDIRECT(ADDRESS(ROW()-1,COLUMN()))`. ODF holds it
/// (§5.8's reference operators) and the core's own parser reads it back, so refusing it would
/// make this filter narrower than the format it writes.
const RANGE_BP: u8 = 100;

struct Parser<'a> {
    src: &'a [char],
    at: usize,
}

/// One end of a reference, before it is known whether there is a second.
#[derive(Clone, Copy)]
struct Part {
    col: Option<Axis>,
    row: Option<Axis>,
}

impl Part {
    fn is_cell(self) -> bool {
        self.col.is_some() && self.row.is_some()
    }

    /// Two ends of one range have to name the same kind of thing: `A1:B2`, `A:C`, `1:3`.
    fn same_shape(self, other: Part) -> bool {
        self.col.is_some() == other.col.is_some() && self.row.is_some() == other.row.is_some()
    }
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<char> {
        self.src.get(self.at).copied()
    }

    fn at_is(&self, offset: usize, c: char) -> bool {
        self.src.get(self.at + offset) == Some(&c)
    }

    fn eat(&mut self, c: char) -> bool {
        let found = self.peek() == Some(c);
        if found {
            self.at += 1;
        }
        found
    }

    /// Skip whitespace; say whether there was any, because a space between two references is
    /// Excel's intersection operator rather than layout.
    fn space(&mut self) -> bool {
        let start = self.at;
        while matches!(self.peek(), Some(' ' | '\t' | '\r' | '\n')) {
            self.at += 1;
        }
        self.at > start
    }

    // ---- the Pratt loop ----

    fn expr(&mut self, min_bp: u8) -> Result<Expr, Refusal> {
        let mut lhs = self.prefix()?;
        loop {
            let spaced = self.space();
            let Some((op, bp, len)) = self.peek_infix() else {
                // Whitespace, and then something that starts an operand: the two are one
                // expression, which means this space *is* the intersection operator.
                if spaced && self.starts_operand() {
                    return Err(Refusal::Intersection);
                }
                break;
            };
            if bp < min_bp {
                break;
            }
            self.at += len;
            // Every binary operator here is left-associative, `^` included (§5.5 Table 1).
            let rhs = self.expr(bp + 1)?;
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    /// The operator at the cursor, its binding power, and how many characters it is.
    fn peek_infix(&self) -> Option<(Op, u8, usize)> {
        Some(match self.peek()? {
            '<' if self.at_is(1, '>') => (Op::Ne, CMP_BP, 2),
            '<' if self.at_is(1, '=') => (Op::Le, CMP_BP, 2),
            '>' if self.at_is(1, '=') => (Op::Ge, CMP_BP, 2),
            '<' => (Op::Lt, CMP_BP, 1),
            '>' => (Op::Gt, CMP_BP, 1),
            '=' => (Op::Eq, CMP_BP, 1),
            '&' => (Op::Concat, CONCAT_BP, 1),
            '+' => (Op::Add, ADD_BP, 1),
            '-' => (Op::Sub, ADD_BP, 1),
            '*' => (Op::Mul, MUL_BP, 1),
            '/' => (Op::Div, MUL_BP, 1),
            '^' => (Op::Pow, POW_BP, 1),
            ':' => (Op::Range, RANGE_BP, 1),
            _ => return None,
        })
    }

    /// Could an operand begin here? Asked only to tell an intersection from trailing space.
    fn starts_operand(&self) -> bool {
        matches!(
            self.peek(),
            Some(c) if c.is_alphanumeric() || matches!(c, '$' | '\'' | '(' | '#' | '"' | '@' | '[' | '{' | '_' | '.')
        )
    }

    fn prefix(&mut self) -> Result<Expr, Refusal> {
        self.space();
        match self.peek() {
            Some('-') => {
                self.at += 1;
                Ok(Expr::Prefix(Op::Sub, Box::new(self.expr(PREFIX_BP)?)))
            }
            Some('+') => {
                self.at += 1;
                Ok(Expr::Prefix(Op::Add, Box::new(self.expr(PREFIX_BP)?)))
            }
            _ => {
                let mut expr = self.primary()?;
                // Postfix `%`, which may repeat: `A1%%` is a hundredth of a hundredth.
                //
                // The whitespace before it is looked past rather than consumed: a space this
                // operand does not turn out to want belongs to the caller, which is the one
                // that can tell an intersection from layout.
                loop {
                    let save = self.at;
                    self.space();
                    if self.peek() != Some('%') {
                        self.at = save;
                        break;
                    }
                    self.at += 1;
                    expr = Expr::Postfix(Op::Percent, Box::new(expr));
                }
                Ok(expr)
            }
        }
    }

    fn primary(&mut self) -> Result<Expr, Refusal> {
        self.space();
        match self.peek().ok_or(Refusal::Syntax)? {
            // The implicit-intersection marker Excel writes in front of a reference in a
            // pre-dynamic-array formula. It says "one cell of this", which is what a
            // non-array evaluator does anyway, so it is dropped rather than carried.
            '@' => {
                self.at += 1;
                self.primary()
            }
            '{' => Err(Refusal::InlineArray),
            '[' => Err(Refusal::ExternalLink),
            '"' => self.string(),
            '#' => self.error_literal(),
            '(' => {
                self.at += 1;
                let inner = self.expr(0)?;
                self.space();
                // `(A1:A2,C1:C2)` — a comma here is not an argument separator, so it is the
                // union operator wearing the only disguise it has.
                if self.peek() == Some(',') {
                    return Err(Refusal::Union);
                }
                match self.eat(')') {
                    true => Ok(Expr::Paren(Box::new(inner))),
                    false => Err(Refusal::Syntax),
                }
            }
            c if c.is_ascii_digit()
                || c == '.'
                || c == '$'
                || c == '\''
                || c.is_alphabetic()
                || c == '_' =>
            {
                self.operand()
            }
            _ => Err(Refusal::Syntax),
        }
    }

    /// A number, a reference, a function call or a name — the four things that start with a
    /// letter, a digit, a `$` or a quote, told apart by what follows rather than by a table.
    fn operand(&mut self) -> Result<Expr, Refusal> {
        // A digit can only begin a number or a whole-row reference (`1:1`), never a name, so
        // the word branch below is never reached for one.
        let numeric = matches!(self.peek(), Some(c) if c.is_ascii_digit() || c == '.');
        // A function name is a word with a `(` immediately after it, and nothing else is
        // (§5.14). Asking first is what keeps `SUM(` from being read as a column reference.
        let save = self.at;
        if let Some(word) = self.word() {
            match self.peek() {
                Some('(') => return self.call(&word),
                // `Sales[Amount]` — the word names a table.
                Some('[') => return Err(Refusal::StructuredReference),
                _ => self.at = save,
            }
        }
        if let Some(reference) = self.reference()? {
            return Ok(Expr::Ref(reference));
        }
        if numeric {
            return self.number();
        }
        if let Some(word) = self.word() {
            // §6.15: Excel writes the two booleans as bare words, OpenFormula as calls.
            return Ok(match word.to_ascii_uppercase().as_str() {
                "TRUE" => Expr::Call {
                    name: "TRUE".into(),
                    args: Vec::new(),
                },
                "FALSE" => Expr::Call {
                    name: "FALSE".into(),
                    args: Vec::new(),
                },
                _ => Expr::Name(word),
            });
        }
        Err(Refusal::Syntax)
    }

    fn call(&mut self, word: &str) -> Result<Expr, Refusal> {
        self.at += 1; // the `(`
        let name = function_name(word);
        let mut args = Vec::new();
        self.space();
        if !self.eat(')') {
            loop {
                self.space();
                // An omitted parameter is present and empty: `OFFSET(A1,1,,2)`.
                args.push(match matches!(self.peek(), Some(',') | Some(')')) {
                    true => Expr::Empty,
                    false => self.expr(0)?,
                });
                self.space();
                if self.eat(',') {
                    continue;
                }
                if self.eat(')') {
                    break;
                }
                return Err(Refusal::Syntax);
            }
        }
        // `_xlfn.SINGLE(x)` is the implicit-intersection marker in function clothing, and it
        // means exactly what `@x` means. The marker is required: a workbook is free to define
        // a macro function called `SINGLE`, and unwrapping *that* would silently change what
        // the cell says.
        if name == "SINGLE" && args.len() == 1 && word != name {
            return Ok(args.remove(0));
        }
        Ok(Expr::Call { name, args })
    }

    fn string(&mut self) -> Result<Expr, Refusal> {
        self.at += 1;
        let mut out = String::new();
        loop {
            match self.peek().ok_or(Refusal::Syntax)? {
                '"' if self.at_is(1, '"') => {
                    out.push('"');
                    self.at += 2;
                }
                '"' => {
                    self.at += 1;
                    return Ok(Expr::Text(out));
                }
                c => {
                    out.push(c);
                    self.at += 1;
                }
            }
        }
    }

    /// `#DIV/0!`, `#N/A`, `#NAME?` … (§5.12). Excel's set is ODF's set, plus `#GETTING_DATA`,
    /// which §5.12's own rule sends to `#NAME?` along with anything else error-shaped.
    fn error_literal(&mut self) -> Result<Expr, Refusal> {
        let start = self.at;
        self.at += 1;
        while matches!(self.peek(), Some(c) if c.is_ascii_alphanumeric() || matches!(c, '/' | '_'))
        {
            self.at += 1;
        }
        if matches!(self.peek(), Some('!') | Some('?')) {
            self.at += 1;
        }
        let name: String = self.src[start..self.at].iter().collect();
        Ok(Expr::Error(
            FormulaError::from_name(&name).unwrap_or(FormulaError::Name),
        ))
    }

    fn number(&mut self) -> Result<Expr, Refusal> {
        let start = self.at;
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.at += 1;
        }
        if self.eat('.') {
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.at += 1;
            }
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            let save = self.at;
            self.at += 1;
            if matches!(self.peek(), Some('+' | '-')) {
                self.at += 1;
            }
            match matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                true => {
                    while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                        self.at += 1;
                    }
                }
                false => self.at = save,
            }
        }
        let text: String = self.src[start..self.at].iter().collect();
        match text.parse::<f64>() {
            Ok(n) if n.is_finite() => Ok(Expr::Number(n)),
            _ => Err(Refusal::Syntax),
        }
    }

    /// An identifier: a function name, a defined name, or the letters half of an address.
    ///
    /// `.` is a word character because `_xlfn.XLOOKUP` is one word to Excel, and `_` because a
    /// defined name may start with it.
    fn word(&mut self) -> Option<String> {
        let start = self.at;
        while matches!(self.peek(), Some(c) if c.is_alphanumeric() || matches!(c, '_' | '.')) {
            self.at += 1;
        }
        match self.at > start {
            true => Some(self.src[start..self.at].iter().collect()),
            false => None,
        }
    }

    // ---- references ----

    /// A whole reference, sheet prefix and all, or `None` when what is here is not one.
    ///
    /// `Ok(None)` leaves the cursor exactly where it was, which is what lets [`Parser::operand`]
    /// try this and fall back to a name.
    fn reference(&mut self) -> Result<Option<Reference>, Refusal> {
        let save = self.at;
        let sheets = self.sheet_prefix();
        let Some((start, end)) = self.reference_body() else {
            if sheets.is_some() {
                // `Data!Name` — a sheet-qualified *name*, which has no ODF spelling this build
                // writes. Restoring here would leave the `!` to be read as an operator.
                return Err(Refusal::Syntax);
            }
            self.at = save;
            return Ok(None);
        };
        // A lone `A` or `1` is a name or a number; only a range may leave an axis out.
        if end.is_none() && !start.is_cell() {
            self.at = save;
            return Ok(None);
        }
        let (first, second) = match sheets {
            Some((a, b)) => (Some(a), b),
            None => (None, None),
        };
        let cell = |sheet: Option<String>, part: Part| CellRef {
            sheet,
            sheet_absolute: false,
            col: part.col,
            row: part.row,
        };
        Ok(Some(Reference {
            source: None,
            start: cell(first, start),
            // `Sheet1:Sheet3!A1` is §4.8's cuboid: one cell on each of a span of sheets, which
            // ODF spells as a range from the first sheet to the last.
            end: match (end, second) {
                (Some(end), sheet) => Some(cell(sheet, end)),
                (None, Some(sheet)) => Some(cell(Some(sheet), start)),
                (None, None) => None,
            },
        }))
    }

    /// `Sheet1!`, `'My Sheet'!`, `Sheet1:Sheet3!` — consumed through the `!`, or nothing.
    fn sheet_prefix(&mut self) -> Option<(String, Option<String>)> {
        let save = self.at;
        let first = self.sheet_name()?;
        let second = match self.eat(':') {
            true => match self.sheet_name() {
                Some(name) => Some(name),
                None => {
                    self.at = save;
                    return None;
                }
            },
            false => None,
        };
        if !self.eat('!') {
            self.at = save;
            return None;
        }
        Some((first, second))
    }

    /// One sheet name, quoted or bare. An inner `'` is doubled on both sides of the
    /// translation, so it is undoubled here and redoubled by the serialiser.
    fn sheet_name(&mut self) -> Option<String> {
        if self.peek() != Some('\'') {
            return self.word();
        }
        let save = self.at;
        self.at += 1;
        let mut out = String::new();
        loop {
            match self.peek() {
                Some('\'') if self.at_is(1, '\'') => {
                    out.push('\'');
                    self.at += 2;
                }
                Some('\'') => {
                    self.at += 1;
                    return Some(out);
                }
                Some(c) => {
                    out.push(c);
                    self.at += 1;
                }
                None => {
                    self.at = save;
                    return None;
                }
            }
        }
    }

    /// `A1`, `$A$1`, `A1:B2`, `A:A`, `1:1` — the part after any sheet prefix.
    fn reference_body(&mut self) -> Option<(Part, Option<Part>)> {
        let start = self.reference_part()?;
        if self.peek() == Some(':') {
            let save = self.at;
            self.at += 1;
            match self.reference_part() {
                Some(end) if start.same_shape(end) => return Some((start, Some(end))),
                // `A1:SUM(…)` and `A1:A` are both something else; leave the `:` where it is
                // and let the expression fail as syntax rather than half-translating it.
                _ => self.at = save,
            }
        }
        Some((start, None))
    }

    /// One end: an optional `$` before each axis, and at least one of the two axes.
    fn reference_part(&mut self) -> Option<Part> {
        let save = self.at;
        let col_absolute = self.eat('$');
        let letters = {
            let start = self.at;
            while matches!(self.peek(), Some(c) if c.is_ascii_alphabetic()) {
                self.at += 1;
            }
            self.src[start..self.at].iter().collect::<String>()
        };
        let mark = self.at;
        let row_absolute = self.eat('$');
        let digits = {
            let start = self.at;
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.at += 1;
            }
            self.src[start..self.at].iter().collect::<String>()
        };
        if digits.is_empty() {
            self.at = mark;
        }
        // `A1B` is neither an address nor an address followed by a name.
        if matches!(self.peek(), Some(c) if c.is_alphanumeric() || matches!(c, '_' | '.')) {
            self.at = save;
            return None;
        }
        let col = match letters.is_empty() {
            true => None,
            false => Some(Axis {
                index: column_index(&letters)?,
                absolute: col_absolute,
            }),
        };
        let row = match digits.is_empty() {
            true => None,
            false => Some(Axis {
                index: row_index(&digits)?,
                absolute: row_absolute,
            }),
        };
        if col.is_none() && row.is_none() {
            self.at = save;
            return None;
        }
        Some(Part { col, row })
    }
}

/// `A` → 0, `XFD` → 16383. `None` for letters that name no column this grid has.
fn column_index(letters: &str) -> Option<u32> {
    if letters.len() > 3 {
        return None;
    }
    let mut col: u32 = 0;
    for c in letters.bytes() {
        col = col * 26 + u32::from(c.to_ascii_uppercase() - b'A' + 1);
    }
    (col > 0 && col <= MAX_COLS).then(|| col - 1)
}

fn row_index(digits: &str) -> Option<u32> {
    if digits.starts_with('0') {
        return None;
    }
    let row: u32 = digits.parse().ok()?;
    (row > 0 && row <= MAX_ROWS).then(|| row - 1)
}

/// The name under Excel's markers, uppercased.
///
/// `_xlfn.` is how a workbook names a function readers older than it had not got; `_xlws.` is
/// the same marker for one that is worksheet-only. Neither is part of the name, and a document
/// that kept them would name functions nothing has ever implemented.
fn function_name(word: &str) -> String {
    let mut name = word;
    for marker in ["_xlfn.", "_xlws."] {
        name = name.strip_prefix(marker).unwrap_or(name);
    }
    name.to_ascii_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[track_caller]
    fn odf(excel: &str) -> String {
        translate(excel)
            .unwrap_or_else(|e| panic!("{excel:?} refused as {}", e.label()))
            .to_string()
    }

    #[track_caller]
    fn refused(excel: &str) -> Refusal {
        translate(excel).expect_err(excel)
    }

    #[test]
    fn references_in_every_spelling() {
        assert_eq!(odf("A1"), "[.A1]");
        assert_eq!(odf("$A$1"), "[.$A$1]");
        assert_eq!(odf("$A1"), "[.$A1]");
        assert_eq!(odf("A$1"), "[.A$1]");
        assert_eq!(odf("A1:B3"), "[.A1:.B3]");
        assert_eq!(odf("XFD1048576"), "[.XFD1048576]");
    }

    #[test]
    fn a_sheet_name_changes_its_punctuation_and_nothing_else() {
        assert_eq!(odf("Data!A1"), "[Data.A1]");
        assert_eq!(odf("SUM(Data!A1:B3)"), "SUM([Data.A1:.B3])");
        assert_eq!(odf("'My Sheet'!A2"), "['My Sheet'.A2]");
        assert_eq!(odf("'O''Brien'!A3"), "['O''Brien'.A3]");
    }

    /// §4.8's cuboid: a span of sheets is a range from the first to the last.
    #[test]
    fn a_three_dimensional_reference_becomes_a_range_across_sheets() {
        assert_eq!(odf("SUM(Data:Sheet3!A1)"), "SUM([Data.A1:Sheet3.A1])");
        assert_eq!(odf("SUM(Data:Sheet3!A1:A2)"), "SUM([Data.A1:Sheet3.A2])");
    }

    #[test]
    fn a_whole_track_keeps_its_missing_axis() {
        assert_eq!(odf("SUM(Data!A:A)"), "SUM([Data.A:.A])");
        assert_eq!(odf("SUM(Data!1:1)"), "SUM([Data.1:.1])");
        assert_eq!(odf("SUM(A:A)"), "SUM([.A:.A])");
        assert_eq!(odf("SUM(1:1)"), "SUM([.1:.1])");
    }

    /// The precedence table's two surprises, which Excel shares with §5.5.
    #[test]
    fn precedence_is_the_tables_and_parentheses_a_person_wrote_survive() {
        assert_eq!(odf("-2^2"), "-2^2");
        assert_eq!(odf("(-2)^2"), "(-2)^2");
        assert_eq!(odf("2^3^2"), "2^3^2");
        assert_eq!(odf("1+2*3"), "1+2*3");
        assert_eq!(odf("(1+2)*3"), "(1+2)*3");
        assert_eq!(odf("A1%"), "[.A1]%");
        assert_eq!(odf("+A1"), "+[.A1]");
    }

    #[test]
    fn every_operator() {
        assert_eq!(odf("A1+A2"), "[.A1]+[.A2]");
        assert_eq!(odf("A1-A2"), "[.A1]-[.A2]");
        assert_eq!(odf("A1*A2"), "[.A1]*[.A2]");
        assert_eq!(odf("A1/A2"), "[.A1]/[.A2]");
        assert_eq!(odf("A1^A2"), "[.A1]^[.A2]");
        assert_eq!(odf("A3&A4"), "[.A3]&[.A4]");
        assert_eq!(odf("A1=A2"), "[.A1]=[.A2]");
        assert_eq!(odf("A1<>A2"), "[.A1]<>[.A2]");
        assert_eq!(odf("A1<=A2"), "[.A1]<=[.A2]");
        assert_eq!(odf("A1>=A2"), "[.A1]>=[.A2]");
        assert_eq!(odf("A1<A2"), "[.A1]<[.A2]");
        assert_eq!(odf("A1>A2"), "[.A1]>[.A2]");
    }

    #[test]
    fn separators_and_booleans() {
        assert_eq!(odf("ROUND(A1/3,2)"), "ROUND([.A1]/3;2)");
        assert_eq!(
            odf("VLOOKUP(4,A1:A5,1,FALSE)"),
            "VLOOKUP(4;[.A1:.A5];1;FALSE())"
        );
        assert_eq!(odf("TRUE"), "TRUE()");
        assert_eq!(odf("NA()"), "NA()");
        assert_eq!(odf("SUMIF(A1:A5,\">4\")"), "SUMIF([.A1:.A5];\">4\")");
    }

    #[test]
    fn strings_errors_and_names() {
        assert_eq!(odf("\"a\"&\" b\""), "\"a\"&\" b\"");
        assert_eq!(odf("\"say \"\"hi\"\"\""), "\"say \"\"hi\"\"\"");
        assert_eq!(odf("#DIV/0!"), "#DIV/0!");
        assert_eq!(odf("#N/A"), "#N/A");
        assert_eq!(odf("#REF!"), "#REF!");
        // §5.12: an error name nothing knows is still an error.
        assert_eq!(odf("#GETTING_DATA"), "#NAME?");
        assert_eq!(odf("SUM(Range)"), "SUM(Range)");
        assert_eq!(odf("Constant*2"), "Constant*2");
    }

    /// A name that looks like an address *is* an address — Excel refuses to define one, so
    /// there is no ambiguity to resolve.
    #[test]
    fn a_word_is_a_function_an_address_or_a_name_in_that_order() {
        assert_eq!(odf("LOG10"), "[.LOG10]");
        assert_eq!(odf("SUM(A1)"), "SUM([.A1])");
        assert_eq!(odf("Rate"), "Rate");
        assert_eq!(odf("_underscore"), "_underscore");
    }

    #[test]
    fn excels_markers_are_not_part_of_the_name() {
        assert_eq!(
            odf("_xlfn.XLOOKUP(3,A1:A4,B1:B4)"),
            "XLOOKUP(3;[.A1:.A4];[.B1:.B4])"
        );
        assert_eq!(odf("_xlfn.CONCAT(B1,B2)"), "CONCAT([.B1];[.B2])");
        assert_eq!(odf("SUM(_xlfn.SINGLE(A1:A4))"), "SUM([.A1:.A4])");
        assert_eq!(odf("@A1"), "[.A1]");
        // Without the marker it is somebody's own function, and unwrapping it would change
        // what the cell says.
        assert_eq!(odf("SINGLE(A1)"), "SINGLE([.A1])");
    }

    #[test]
    fn numbers_in_every_spelling() {
        assert_eq!(odf("0.2"), "0.2");
        assert_eq!(odf("1E+5"), "100000");
        assert_eq!(odf(".5"), "0.5");
        assert_eq!(odf("-2.5"), "-2.5");
    }

    /// A plain `A1:B2` is one reference, scanned whole; a range whose far end is computed is
    /// the operator, which ODF has and this build therefore carries.
    #[test]
    fn a_range_is_a_reference_where_it_can_be_and_an_operator_where_it_cannot() {
        assert_eq!(odf("A1:B2"), "[.A1:.B2]");
        assert_eq!(
            odf("SUM(H2:INDIRECT(ADDRESS(ROW()-1,COLUMN())))"),
            "SUM([.H2]:INDIRECT(ADDRESS(ROW()-1;COLUMN())))"
        );
    }

    #[test]
    fn an_omitted_parameter_is_present_and_empty() {
        assert_eq!(odf("OFFSET(A1,1,,2)"), "OFFSET([.A1];1;;2)");
    }

    #[test]
    fn every_refusal_is_a_class_with_a_name() {
        assert_eq!(refused("SUM({1,2;3,4})"), Refusal::InlineArray);
        assert_eq!(refused("SUM(Sales[Amount])"), Refusal::StructuredReference);
        assert_eq!(
            refused("Sales[[#This Row],[Amount]]*0.2"),
            Refusal::StructuredReference
        );
        assert_eq!(refused("[1]Sheet1!$A$1"), Refusal::ExternalLink);
        assert_eq!(refused("SUM([1]Sheet1!$A$1:$A$3)"), Refusal::ExternalLink);
        assert_eq!(refused("[1]!ExternalName"), Refusal::ExternalLink);
        assert_eq!(refused("SUM(A1:C2 B1:B3)"), Refusal::Intersection);
        assert_eq!(refused("SUM((A1:A2,C1:C2))"), Refusal::Union);
        assert_eq!(refused(""), Refusal::Syntax);
        assert_eq!(refused("A1+"), Refusal::Syntax);
        assert_eq!(refused("SUM(A1"), Refusal::Syntax);
    }

    /// Only two classes are a `Dropped` kind: the rest are expressions ODF could hold.
    #[test]
    fn a_refusal_counts_as_a_dropped_construct_only_where_one_applies() {
        assert_eq!(
            Refusal::StructuredReference.dropped(),
            Some(Dropped::StructuredReference)
        );
        assert_eq!(Refusal::ExternalLink.dropped(), Some(Dropped::ExternalLink));
        assert_eq!(Refusal::Intersection.dropped(), None);
        assert_eq!(Refusal::Union.dropped(), None);
        assert_eq!(Refusal::InlineArray.dropped(), None);
        assert_eq!(Refusal::Syntax.dropped(), None);
    }

    /// The property the whole file is under: what comes out is something the ODF parser
    /// accepts, and re-parsing it changes nothing.
    #[test]
    fn everything_translated_is_a_formula_our_own_parser_produces() {
        for excel in [
            "A1+A2",
            "SUM(Data!A1:B3)",
            "-2^2",
            "(-2)^2",
            "2^3^2",
            "IF(A1>1,\"yes\",\"no\")",
            "'O''Brien'!A3",
            "SUM(Data:Sheet3!A1)",
            "A1%",
            "SUM(A:A)",
            "TRUE",
            "#DIV/0!",
            "ROUND(A1/3,2)",
            "OFFSET(A1,1,,2)",
        ] {
            let ours = odf(excel);
            let reparsed = grind_sheet::formula::parse::parse(&ours)
                .unwrap_or_else(|e| panic!("{excel:?} → {ours:?}, which will not parse: {e:?}"));
            assert_eq!(reparsed.to_string(), ours, "{excel}");
        }
    }

    /// Which functions a translated formula names is `funcs::used`'s question, asked of the
    /// text this file produced — one walker over the AST rather than a second one here.
    #[test]
    fn the_functions_a_translated_formula_names_are_the_cores_answer() {
        let ours = odf("SUM(ABS(A1),MAX(B1:B2))");
        assert_eq!(
            grind_sheet::formula::funcs::used(&ours),
            Some(vec!["SUM".to_owned(), "ABS".to_owned(), "MAX".to_owned()])
        );
    }
}
