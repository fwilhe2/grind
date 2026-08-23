// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The OpenFormula tokenizer (ODF 1.4 Part 4 §5).
//!
//! Everything the grammar calls a *terminating rule* — one whose definition mentions no
//! other rule (§5.14) — is resolved here, including whole references. A reference is
//! lexical, not syntactic: it always begins with `[`, which §5.8 says exists precisely so
//! that a cell address can never be confused with a function name, and it ends at the
//! matching `]`. So the parser deals in [`Token`]s and never looks at a character.
//!
//! Whitespace is dropped (§5.14) with one exception that has to survive to the parser:
//! whitespace may precede a function name but **shall not** separate that name from its
//! opening parenthesis. That is why an identifier followed *immediately* by `(` lexes as
//! [`Token::Func`] and any other identifier as [`Token::Name`] — the distinction is
//! lexical, and reconstructing it later would need the character offsets we just threw
//! away.

use std::fmt;

use super::value::FormulaError;

/// A formula that is not a formula. Carries where, because the CLI will want to say so.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxError {
    pub message: String,
    /// Offset in `char`s from the start of the expression.
    pub at: usize,
}

impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at character {}", self.message, self.at)
    }
}

impl std::error::Error for SyntaxError {}

fn err<T>(message: impl Into<String>, at: usize) -> Result<T, SyntaxError> {
    Err(SyntaxError {
        message: message.into(),
        at,
    })
}

/// §5.5 Operators. `~` (reference union) is lexed but rejected by the evaluator: §2.3.2 G
/// excludes it from the Small Group, and a document that uses it deserves a clear error
/// rather than a parse failure that blames the wrong thing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Concat,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    /// Postfix `%` — divide by 100 (§5.5).
    Percent,
    /// `:` range.
    Range,
    /// `!` reference intersection.
    Intersect,
    /// `~` reference union.
    Union,
}

impl Op {
    pub fn text(self) -> &'static str {
        match self {
            Op::Add => "+",
            Op::Sub => "-",
            Op::Mul => "*",
            Op::Div => "/",
            Op::Pow => "^",
            Op::Concat => "&",
            Op::Eq => "=",
            Op::Ne => "<>",
            Op::Lt => "<",
            Op::Le => "<=",
            Op::Gt => ">",
            Op::Ge => ">=",
            Op::Percent => "%",
            Op::Range => ":",
            Op::Intersect => "!",
            Op::Union => "~",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    Number(f64),
    Text(String),
    /// A constant error, `#N/A` and friends (§5.12). `[#REF!]` lands here too (§5.8).
    Error(FormulaError),
    Ref(Reference),
    /// An identifier: a named expression (§5.11).
    Name(String),
    /// An identifier immediately followed by `(` — a function name, and only here (§5.14).
    Func(String),
    Op(Op),
    LParen,
    RParen,
    /// `;` — the parameter separator (§5.6). Never `,`.
    Semi,
}

/// One axis of a cell address: `A`, `$A`, `1`, `$1`. Stored **0-based**, like every other
/// position in the core; only the text form is 1-based.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Axis {
    pub index: u32,
    pub absolute: bool,
}

/// One end of a reference (§5.8).
///
/// `col` and `row` are optional because the grammar has whole-column (`[.A:.C]`) and
/// whole-row (`[.1:.3]`) forms, where the missing axis means "everything the evaluator
/// supports". Absent `sheet` means the sheet the formula lives on.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct CellRef {
    pub sheet: Option<String>,
    pub sheet_absolute: bool,
    pub col: Option<Axis>,
    pub row: Option<Axis>,
}

/// A constant reference: one cell, or a range when `end` is present (§5.8).
///
/// `source` is the optional external-document IRI. Nothing evaluates it — external
/// documents are out of scope — but it is kept so that parsing and re-serialising a
/// formula cannot quietly turn a reference into a different one.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Reference {
    pub source: Option<String>,
    pub start: CellRef,
    pub end: Option<CellRef>,
}

pub fn lex(src: &str) -> Result<Vec<Token>, SyntaxError> {
    Ok(lex_spans(src)?.0)
}

/// The tokens, and the `char` offset each one starts at.
///
/// The offsets exist for one reason: the parser reports failures by *token*, and a caller
/// that wants to put a caret on the problem needs a position in the text. Keeping them here
/// rather than in a `Token` field means nothing that merely reads tokens pays for them.
pub fn lex_spans(src: &str) -> Result<(Vec<Token>, Vec<usize>), SyntaxError> {
    // Char-indexed: the grammar allows non-ASCII in identifiers, sheet names and strings,
    // and byte offsets in a diagnostic would point at the wrong place.
    let src: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut at = Vec::new();
    let mut i = 0;
    while i < src.len() {
        let c = src[i];
        at.truncate(out.len());
        at.resize(out.len() + 1, i);
        match c {
            // §5.14 Whitespace, exactly these four.
            ' ' | '\t' | '\n' | '\r' => i += 1,
            '(' => {
                out.push(Token::LParen);
                i += 1;
            }
            ')' => {
                out.push(Token::RParen);
                i += 1;
            }
            ';' => {
                out.push(Token::Semi);
                i += 1;
            }
            '"' => {
                let (text, next) = string(&src, i)?;
                out.push(Token::Text(text));
                i = next;
            }
            '#' => {
                let (e, next) = error_name(&src, i)?;
                out.push(Token::Error(e));
                i = next;
            }
            '[' => {
                let (token, next) = reference(&src, i)?;
                out.push(token);
                i = next;
            }
            '0'..='9' => {
                let (n, next) = number(&src, i)?;
                out.push(Token::Number(n));
                i = next;
            }
            // §5.3: a leading `.` is readable but never written. Anything else starting
            // with `.` is not a number, and `.` alone is not a token at all.
            '.' if matches!(src.get(i + 1), Some('0'..='9')) => {
                let (n, next) = number(&src, i)?;
                out.push(Token::Number(n));
                i = next;
            }
            '<' => {
                let (op, next) = match src.get(i + 1) {
                    Some('=') => (Op::Le, i + 2),
                    Some('>') => (Op::Ne, i + 2),
                    _ => (Op::Lt, i + 1),
                };
                out.push(Token::Op(op));
                i = next;
            }
            '>' => {
                let (op, next) = match src.get(i + 1) {
                    Some('=') => (Op::Ge, i + 2),
                    _ => (Op::Gt, i + 1),
                };
                out.push(Token::Op(op));
                i = next;
            }
            '+' | '-' | '*' | '/' | '^' | '&' | '=' | '%' | ':' | '!' | '~' => {
                let op = match c {
                    '+' => Op::Add,
                    '-' => Op::Sub,
                    '*' => Op::Mul,
                    '/' => Op::Div,
                    '^' => Op::Pow,
                    '&' => Op::Concat,
                    '=' => Op::Eq,
                    '%' => Op::Percent,
                    ':' => Op::Range,
                    '!' => Op::Intersect,
                    _ => Op::Union,
                };
                out.push(Token::Op(op));
                i += 1;
            }
            // `$$Name` and `$$'Name'` — an explicitly marked named expression (§5.11).
            '$' if src.get(i + 1) == Some(&'$') => {
                let (name, next) = if src.get(i + 2) == Some(&'\'') {
                    single_quoted(&src, i + 2)?
                } else {
                    let (name, next) = identifier(&src, i + 2);
                    if name.is_empty() {
                        return err("expected a name after `$$`", i);
                    }
                    (name, next)
                };
                out.push(Token::Name(name));
                i = next;
            }
            c if is_name_start(c) => {
                let (name, next) = identifier(&src, i);
                // §5.14: whitespace may come *before* a function name but never between it
                // and its `(`. So this test is the whole distinction, and it is lexical.
                out.push(if src.get(next) == Some(&'(') {
                    Token::Func(name)
                } else {
                    Token::Name(name)
                });
                i = next;
            }
            c => return err(format!("unexpected character {c:?}"), i),
        }
    }
    at.truncate(out.len());
    Ok((out, at))
}

/// §5.6 `FunctionName` / §5.11 `Identifier`, loosened to any Unicode alphabetic since the
/// grammar's `LetterXML` is [XML1.0]'s Letter production.
fn is_name_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_name_char(c: char) -> bool {
    // `.` belongs to FunctionName only, which is what makes `COM.MICROSOFT.CUBEMEMBER` one
    // token (§5.7). A named expression containing a `.` cannot exist, so accepting it here
    // costs nothing and keeps one scanner instead of two.
    c.is_alphanumeric() || c == '_' || c == '.'
}

fn identifier(src: &[char], start: usize) -> (String, usize) {
    let mut i = start;
    while i < src.len() && is_name_char(src[i]) {
        i += 1;
    }
    (src[start..i].iter().collect(), i)
}

/// §5.4: `"` … `"`, a literal `"` doubled.
fn string(src: &[char], start: usize) -> Result<(String, usize), SyntaxError> {
    let mut out = String::new();
    let mut i = start + 1;
    while i < src.len() {
        match src[i] {
            '"' if src.get(i + 1) == Some(&'"') => {
                out.push('"');
                i += 2;
            }
            '"' => return Ok((out, i + 1)),
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    err("unterminated string", start)
}

/// `'` … `'`, a literal `'` doubled (§5.2 `SingleQuoted`). Used for sheet names and for
/// `$$'named expressions'`.
fn single_quoted(src: &[char], start: usize) -> Result<(String, usize), SyntaxError> {
    let mut out = String::new();
    let mut i = start + 1;
    while i < src.len() {
        match src[i] {
            '\'' if src.get(i + 1) == Some(&'\'') => {
                out.push('\'');
                i += 2;
            }
            '\'' => return Ok((out, i + 1)),
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    err("unterminated quoted name", start)
}

/// §5.3 Constant Numbers. Locale-free by definition: `.` is the decimal separator and there
/// are no group separators anywhere in the grammar.
fn number(src: &[char], start: usize) -> Result<(f64, usize), SyntaxError> {
    let mut i = start;
    while i < src.len() && src[i].is_ascii_digit() {
        i += 1;
    }
    if src.get(i) == Some(&'.') {
        i += 1;
        while i < src.len() && src[i].is_ascii_digit() {
            i += 1;
        }
    }
    if matches!(src.get(i), Some('e' | 'E')) {
        let mut j = i + 1;
        if matches!(src.get(j), Some('+' | '-')) {
            j += 1;
        }
        // A trailing `e` is not part of the number: `1e` is `1` followed by the named
        // expression `e`, which the parser will then reject on its own terms.
        if matches!(src.get(j), Some(c) if c.is_ascii_digit()) {
            i = j;
            while i < src.len() && src[i].is_ascii_digit() {
                i += 1;
            }
        }
    }
    let text: String = src[start..i].iter().collect();
    match text.parse::<f64>() {
        Ok(n) if n.is_finite() => Ok((n, i)),
        // Overflow to infinity: syntactically a Number, but §4.3.1 has no value for it.
        _ => err(format!("{text} is not a number"), start),
    }
}

/// §5.12: `Error ::= '#' [A-Z0-9]+ ([!?] | ('/' ([A-Z] | ([0-9] [!?]))))`.
///
/// Lower case and `_` are accepted beyond that production because implementations write
/// them (`#getting_data` appears in LO's own fixtures) and §5.12 already says an error name
/// we do not know becomes one we do. Rejecting the *spelling* of an error would turn a
/// document that says "this cell is broken" into a document that will not open.
fn error_name(src: &[char], start: usize) -> Result<(FormulaError, usize), SyntaxError> {
    let mut i = start + 1;
    while i < src.len() && (src[i].is_ascii_alphanumeric() || src[i] == '/' || src[i] == '_') {
        i += 1;
    }
    if matches!(src.get(i), Some('!' | '?')) {
        i += 1;
    }
    let text: String = src[start..i].iter().collect();
    // from_name maps anything error-shaped we do not know onto #NAME? (§5.12), so this
    // fails only when there was no error name at all.
    match FormulaError::from_name(&text) {
        Some(e) if text.len() > 1 => Ok((e, i)),
        _ => err("expected an error name after `#`", start),
    }
}

/// §5.8 References. `start` points at the `[`.
fn reference(src: &[char], start: usize) -> Result<(Token, usize), SyntaxError> {
    let mut i = start + 1;
    let mut inside = String::new();
    // A `]` inside a quoted sheet name or IRI does not close the reference, so the scan has
    // to track quoting rather than search for the first `]`.
    while i < src.len() {
        match src[i] {
            ']' => {
                let token = if inside.trim() == "#REF!" {
                    Token::Error(FormulaError::Ref)
                } else {
                    Token::Ref(parse_reference(&inside, start)?)
                };
                return Ok((token, i + 1));
            }
            '\'' => {
                let (text, next) = single_quoted(src, i)?;
                inside.push('\'');
                inside.push_str(&text.replace('\'', "''"));
                inside.push('\'');
                i = next;
            }
            c => {
                inside.push(c);
                i += 1;
            }
        }
    }
    err("unterminated reference", start)
}

/// The inside of a `[…]`, minus the brackets.
fn parse_reference(inside: &str, at: usize) -> Result<Reference, SyntaxError> {
    let chars: Vec<char> = inside.chars().collect();
    let mut i = 0;

    // §5.11: a leading `'…'` is a Source if `#` follows the closing quote, and a sheet name
    // if `.` does. This is the one place the two are ambiguous.
    let source = if chars.first() == Some(&'\'') {
        let (text, next) = single_quoted(&chars, 0).map_err(|e| shift(e, at))?;
        if chars.get(next) == Some(&'#') {
            i = next + 1;
            Some(text)
        } else {
            None
        }
    } else {
        None
    };

    // Split at the `:` that separates the two ends, ignoring any inside a quoted name.
    let rest = &chars[i..];
    let mut split = None;
    let mut j = 0;
    while j < rest.len() {
        match rest[j] {
            '\'' => {
                let (_, next) = single_quoted(rest, j).map_err(|e| shift(e, at + i))?;
                j = next;
            }
            ':' => {
                split = Some(j);
                break;
            }
            _ => j += 1,
        }
    }
    let (first, second) = match split {
        Some(j) => (&rest[..j], Some(&rest[j + 1..])),
        None => (rest, None),
    };

    let start = parse_cell_ref(first, at)?;
    let mut end = second.map(|s| parse_cell_ref(s, at)).transpose()?;
    // §5.8: "if the first part contains a SheetLocator and the second part does not, the
    // second part inherits the SheetLocator from the first part."
    if let Some(end) = &mut end
        && end.sheet.is_none()
    {
        end.sheet.clone_from(&start.sheet);
        end.sheet_absolute = start.sheet_absolute;
    }
    Ok(Reference { source, start, end })
}

/// `SheetLocatorOrEmpty '.' Column? Row?` — one end of a reference.
fn parse_cell_ref(src: &[char], at: usize) -> Result<CellRef, SyntaxError> {
    let mut i = 0;
    let mut sheet = None;

    // A leading `$` can only be the sheet-absolute marker: the grammar puts a `.` before
    // every cell address, so a sheetless reference starts with that `.` and never with a
    // `$`. Absolute *column* markers live after the dot.
    let mut sheet_absolute = src.first() == Some(&'$');
    if sheet_absolute {
        i += 1;
    }
    if src.get(i) == Some(&'\'') {
        let (name, next) = single_quoted(src, i).map_err(|e| shift(e, at))?;
        sheet = Some(name);
        i = next;
    } else if let Some(dot) = src[i..].iter().position(|c| *c == '.') {
        if dot > 0 {
            sheet = Some(src[i..i + dot].iter().collect());
        }
        i += dot;
    }
    sheet_absolute &= sheet.is_some();
    if src.get(i) != Some(&'.') {
        return err("a reference needs a `.` before its cell address", at);
    }
    i += 1;

    let col_absolute = src.get(i) == Some(&'$');
    if col_absolute {
        i += 1;
    }
    let letters_start = i;
    while matches!(src.get(i), Some(c) if c.is_ascii_uppercase()) {
        i += 1;
    }
    let col = if i > letters_start {
        Some(Axis {
            index: column_index(&src[letters_start..i]).ok_or_else(|| SyntaxError {
                message: "column out of range".into(),
                at,
            })?,
            absolute: col_absolute,
        })
    } else {
        None
    };

    let row_absolute = src.get(i) == Some(&'$');
    if row_absolute {
        i += 1;
    }
    let digits_start = i;
    while matches!(src.get(i), Some(c) if c.is_ascii_digit()) {
        i += 1;
    }
    let row = if i > digits_start {
        let text: String = src[digits_start..i].iter().collect();
        let n: u32 = text.parse().map_err(|_| SyntaxError {
            message: "row out of range".into(),
            at,
        })?;
        if n == 0 {
            return err("rows are 1-based", at);
        }
        Some(Axis {
            index: n - 1,
            absolute: row_absolute,
        })
    } else {
        None
    };

    if i != src.len() || (col.is_none() && row.is_none()) {
        return err("not a cell address", at);
    }
    Ok(CellRef {
        sheet,
        sheet_absolute,
        col,
        row,
    })
}

/// `A` → 0, `Z` → 25, `AA` → 26 (§5.8). Bijective base-26, not base-26.
fn column_index(letters: &[char]) -> Option<u32> {
    let mut index: u32 = 0;
    for c in letters {
        index = index
            .checked_mul(26)?
            .checked_add(*c as u32 - 'A' as u32 + 1)?;
    }
    index.checked_sub(1)
}

/// Column index back to letters, for serialisation.
pub fn column_name(mut index: u32) -> String {
    let mut out = Vec::new();
    loop {
        out.push(b'A' + (index % 26) as u8);
        match index / 26 {
            0 => break,
            next => index = next - 1,
        }
    }
    out.reverse();
    String::from_utf8(out).expect("ASCII")
}

fn shift(e: SyntaxError, by: usize) -> SyntaxError {
    SyntaxError { at: e.at + by, ..e }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(src: &str) -> Vec<Token> {
        lex(src).expect(src)
    }

    fn cell(src: &str) -> Reference {
        match &tokens(src)[..] {
            [Token::Ref(r)] => r.clone(),
            other => panic!("{src} lexed as {other:?}"),
        }
    }

    #[test]
    fn a_name_is_a_function_only_when_the_paren_touches_it() {
        // §5.14. `SUM (1)` is a named expression applied to nothing, and shall not lex as a
        // call; the parser is the one that then complains.
        assert_eq!(tokens("SUM(")[0], Token::Func("SUM".into()));
        assert_eq!(tokens("SUM (")[0], Token::Name("SUM".into()));
        assert_eq!(tokens(" SUM(")[0], Token::Func("SUM".into()));
    }

    #[test]
    fn vendor_function_names_are_one_token() {
        // §5.7: the dots are part of the name, not operators.
        assert_eq!(
            tokens("COM.MICROSOFT.CUBEMEMBER()")[0],
            Token::Func("COM.MICROSOFT.CUBEMEMBER".into())
        );
    }

    #[test]
    fn numbers_follow_section_5_3() {
        assert_eq!(tokens("1"), [Token::Number(1.0)]);
        assert_eq!(tokens("1.5e-3"), [Token::Number(0.0015)]);
        assert_eq!(tokens(".5"), [Token::Number(0.5)]); // readable, never written
        // A trailing `e` is not an exponent, and the number ends before it.
        assert_eq!(tokens("1e"), [Token::Number(1.0), Token::Name("e".into())]);
        assert!(lex("1e999").is_err()); // syntactically fine, not a Number (§4.3.1)
    }

    #[test]
    fn a_doubled_quote_is_one_quote() {
        assert_eq!(tokens(r#""a""b""#), [Token::Text(r#"a"b"#.into())]);
        assert_eq!(tokens(r#""""#), [Token::Text(String::new())]);
        assert!(lex(r#""unterminated"#).is_err());
    }

    #[test]
    fn constant_errors_lex_by_name() {
        assert_eq!(tokens("#N/A"), [Token::Error(FormulaError::NA)]);
        assert_eq!(tokens("#DIV/0!"), [Token::Error(FormulaError::DivZero)]);
        // §5.8: a reference can be a reference error instead of an address.
        assert_eq!(tokens("[#REF!]"), [Token::Error(FormulaError::Ref)]);
        assert!(lex("#").is_err());
    }

    #[test]
    fn comparison_operators_are_greedy() {
        assert_eq!(
            tokens("<=<><"),
            [Token::Op(Op::Le), Token::Op(Op::Ne), Token::Op(Op::Lt)]
        );
    }

    #[test]
    fn a_plain_reference_is_relative_and_sheet_local() {
        let r = cell("[.A1]");
        assert_eq!(r.start.sheet, None);
        assert_eq!(
            r.start.col,
            Some(Axis {
                index: 0,
                absolute: false
            })
        );
        assert_eq!(
            r.start.row,
            Some(Axis {
                index: 0,
                absolute: false
            })
        );
        assert_eq!(r.end, None);
    }

    #[test]
    fn dollars_mark_each_axis_independently() {
        let r = cell("[.$B$10]");
        assert_eq!(
            r.start.col,
            Some(Axis {
                index: 1,
                absolute: true
            })
        );
        assert_eq!(
            r.start.row,
            Some(Axis {
                index: 9,
                absolute: true
            })
        );
        let r = cell("[.$C7]");
        assert!(r.start.col.unwrap().absolute);
        assert!(!r.start.row.unwrap().absolute);
    }

    #[test]
    fn the_second_end_of_a_range_inherits_the_first_ends_sheet() {
        // §5.8, and getting this wrong silently reads the wrong sheet rather than failing.
        let r = cell("[Sheet2.A1:.B2]");
        assert_eq!(r.start.sheet.as_deref(), Some("Sheet2"));
        assert_eq!(r.end.as_ref().unwrap().sheet.as_deref(), Some("Sheet2"));
        // Two explicit sheets stay two sheets (a cuboid, §4.8).
        let r = cell("[Sheet1.A1:Sheet3.B2]");
        assert_eq!(r.end.unwrap().sheet.as_deref(), Some("Sheet3"));
    }

    #[test]
    fn a_quoted_sheet_name_may_contain_anything_including_a_doubled_quote() {
        let r = cell("['It''s a sheet.1'.A1]");
        assert_eq!(r.start.sheet.as_deref(), Some("It's a sheet.1"));
        assert_eq!(r.start.col.unwrap().index, 0);
    }

    #[test]
    fn whole_columns_and_whole_rows_leave_the_other_axis_open() {
        let r = cell("[.A:.C]");
        assert_eq!(r.start.row, None);
        assert_eq!(r.end.unwrap().col.unwrap().index, 2);
        let r = cell("[.1:.3]");
        assert_eq!(r.start.col, None);
        assert_eq!(r.start.row.unwrap().index, 0);
    }

    #[test]
    fn an_external_source_is_kept_rather_than_confused_with_a_sheet_name() {
        // §5.11: `'…'#` is a Source, `'…'.` is a sheet name. Nothing evaluates the IRI, but
        // dropping it would silently retarget the reference at the current document.
        let r = cell("['file:///tmp/other.ods'#$Sheet1.A1]");
        assert_eq!(r.source.as_deref(), Some("file:///tmp/other.ods"));
        assert_eq!(r.start.sheet.as_deref(), Some("Sheet1"));
        assert!(r.start.sheet_absolute);
    }

    #[test]
    fn a_bracket_inside_a_quoted_name_does_not_end_the_reference() {
        let r = cell("['a]b'.A1]");
        assert_eq!(r.start.sheet.as_deref(), Some("a]b"));
    }

    #[test]
    fn malformed_references_are_errors_not_guesses() {
        for src in ["[A1]", "[.]", "[.a1]", "[.A0]", "[.A1", "[.A1:]"] {
            assert!(lex(src).is_err(), "{src} should not lex");
        }
    }

    #[test]
    fn column_names_are_bijective_base_26() {
        for (name, index) in [("A", 0u32), ("Z", 25), ("AA", 26), ("AMJ", 1023)] {
            let chars: Vec<char> = name.chars().collect();
            assert_eq!(column_index(&chars), Some(index), "{name}");
            assert_eq!(column_name(index), name);
        }
    }
}
