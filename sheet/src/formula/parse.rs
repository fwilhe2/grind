// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The OpenFormula parser: tokens → AST (ODF 1.4 Part 4 §5).
//!
//! A Pratt parser, because §5.5's Table 1 *is* a table of binding powers and this is the
//! shape that transcribes it without inventing a precedence-climbing hierarchy of
//! productions. The table's two surprises are both encoded in `PREFIX_BP`: prefix `-`
//! binds **tighter** than `^` (so `-2^2` is `4`, not `-4`), and `^` is **left**-associative
//! (so `2^3^2` is `64`, not `512`) — the opposite of both in most languages.
//!
//! Not parsed, deliberately: inline arrays (§5.13) and the automatic-intersection and
//! quoted-label forms (§5.10). §2.3.2 excludes all three from the Small Group, and each
//! would need evaluator machinery that does not exist. They fail as syntax errors, which
//! is a truthful answer.

use super::lex::{Op, Reference, SyntaxError, Token, lex_spans};
use super::value::FormulaError;

/// An expression (§5.2).
///
/// [`Expr::Paren`] is a node rather than a parse artefact because §5.5 Note 3 asks
/// implementations to keep "unnecessary" parentheses: a user put them there to be read.
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Number(f64),
    Text(String),
    Error(FormulaError),
    Ref(Reference),
    /// A named expression (§5.11).
    Name(String),
    Call {
        name: String,
        args: Vec<Expr>,
    },
    /// Prefix `+` or `-` (§5.5).
    Prefix(Op, Box<Expr>),
    /// Postfix `%` (§5.5).
    Postfix(Op, Box<Expr>),
    Binary(Op, Box<Expr>, Box<Expr>),
    Paren(Box<Expr>),
    /// An omitted parameter, as in `OFFSET([.A1];1;;2)` (§5.6). Distinct from a missing
    /// one: the parameter is present and empty.
    Empty,
}

/// Binding powers, straight off §5.5 Table 1, lowest precedence first.
pub(crate) fn infix_bp(op: Op) -> u8 {
    match op {
        Op::Eq | Op::Ne | Op::Lt | Op::Le | Op::Gt | Op::Ge => 10,
        Op::Concat => 20,
        Op::Add | Op::Sub => 30,
        Op::Mul | Op::Div => 40,
        Op::Pow => 50,
        Op::Percent => POSTFIX_BP,
        Op::Union => 80,
        Op::Intersect => 90,
        Op::Range => 100,
    }
}

pub(crate) const POSTFIX_BP: u8 = 60;
pub(crate) const PREFIX_BP: u8 = 70;

/// Parse a `table:formula` attribute value, or anything a user typed.
///
/// Handles the namespace prefix documents carry (`of:=SUM(…)`, doc/ods-format.md §4) and
/// §5.2's optional `=` intro. A second `=` marks "forced recalculation", which is a
/// property of *when* to evaluate rather than of the expression, and is dropped until
/// there is a recalculation engine with an opinion about it.
pub fn parse(formula: &str) -> Result<Expr, SyntaxError> {
    let body = strip_intro(formula);
    // The intro is stripped before lexing, so every offset below is relative to `body`;
    // adding it back is what makes the reported position an offset into what was passed in.
    let intro = formula.chars().count() - body.chars().count();
    let (tokens, offsets) = lex_spans(body).map_err(|e| shift(e, intro))?;
    let mut p = Parser { tokens, pos: 0 };
    let parsed = p.expr(0).and_then(|expr| match p.pos == p.tokens.len() {
        true => Ok(expr),
        false => p.fail("unexpected trailing input"),
    });
    // The parser counts in tokens, which is the only unit it has; a caller wants a caret.
    parsed.map_err(|e| SyntaxError {
        at: offsets
            .get(e.at)
            .copied()
            .unwrap_or_else(|| body.chars().count())
            + intro,
        ..e
    })
}

fn shift(e: SyntaxError, by: usize) -> SyntaxError {
    SyntaxError { at: e.at + by, ..e }
}

/// What a formula says before its expression — `"of:="`, `"="`, `"of:=="`, or nothing.
///
/// The complement of `strip_intro`, and public because a *rewriter* has to put back what it
/// took off: `formula::rename` re-serialises an expression and a document that spells its
/// formulas `of:=` must keep spelling them that way, or renaming one sheet respells every
/// formula in the file (R6).
pub fn intro(formula: &str) -> &str {
    &formula[..formula.len() - strip_intro(formula).len()]
}

/// `of:=` / `=` / `==` → the expression after it (§5.2).
fn strip_intro(formula: &str) -> &str {
    let rest = match formula.find('=') {
        // A namespace prefix is an NCName followed by `:`, and it can only sit before the
        // very first `=`. Anything else before an `=` means there was no intro at all —
        // `[.A1]="x"` is a whole formula whose first `=` is an operator.
        Some(eq) => {
            let head = formula[..eq].trim_start();
            match head.strip_suffix(':') {
                _ if head.is_empty() => &formula[eq + 1..],
                Some(prefix) if !prefix.is_empty() && prefix.chars().all(is_prefix_char) => {
                    &formula[eq + 1..]
                }
                _ => return formula,
            }
        }
        None => return formula,
    };
    // The forced-recalculate marker.
    rest.strip_prefix('=').unwrap_or(rest)
}

fn is_prefix_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-' || c == '.'
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.pos).cloned();
        self.pos += 1;
        token
    }

    /// `at` is a **token index** here; [`parse`] maps it back to a character offset through
    /// the spans the lexer kept, so what a caller sees is always a position in the text.
    fn fail<T>(&self, message: &str) -> Result<T, SyntaxError> {
        Err(SyntaxError {
            message: message.to_owned(),
            at: self.pos,
        })
    }

    fn expect(&mut self, token: Token, message: &str) -> Result<(), SyntaxError> {
        if self.peek() == Some(&token) {
            self.pos += 1;
            Ok(())
        } else {
            self.fail(message)
        }
    }

    /// Pratt: parse a prefix, then absorb every operator that binds tighter than `min_bp`.
    fn expr(&mut self, min_bp: u8) -> Result<Expr, SyntaxError> {
        let mut lhs = self.prefix()?;
        while let Some(&Token::Op(op)) = self.peek() {
            let bp = infix_bp(op);
            // `<=` rather than `<`: every operator in Table 1 is left-associative, and
            // prefix `+`/`-` — the only right-associative ones — never reach this loop.
            if bp <= min_bp {
                break;
            }
            self.pos += 1;
            if op == Op::Percent {
                lhs = Expr::Postfix(op, Box::new(lhs));
                continue;
            }
            let rhs = self.expr(bp)?;
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn prefix(&mut self) -> Result<Expr, SyntaxError> {
        match self.next() {
            Some(Token::Number(n)) => Ok(Expr::Number(n)),
            Some(Token::Text(s)) => Ok(Expr::Text(s)),
            Some(Token::Error(e)) => Ok(Expr::Error(e)),
            Some(Token::Ref(r)) => Ok(Expr::Ref(r)),
            Some(Token::Name(name)) => Ok(Expr::Name(name)),
            Some(Token::Func(name)) => {
                self.expect(Token::LParen, "expected `(` after a function name")?;
                Ok(Expr::Call {
                    name,
                    args: self.args()?,
                })
            }
            Some(Token::LParen) => {
                let inner = self.expr(0)?;
                self.expect(Token::RParen, "expected `)`")?;
                Ok(Expr::Paren(Box::new(inner)))
            }
            Some(Token::Op(op @ (Op::Add | Op::Sub))) => {
                Ok(Expr::Prefix(op, Box::new(self.expr(PREFIX_BP)?)))
            }
            _ => {
                self.pos -= 1;
                self.fail("expected a value")
            }
        }
    }

    /// §5.6 `ParameterList`, the `(` already consumed.
    fn args(&mut self) -> Result<Vec<Expr>, SyntaxError> {
        // "An empty list of parameters is considered a call with 0 parameters, not a call
        // with one parameter that happens to be empty" — TRUE() takes none.
        if self.peek() == Some(&Token::RParen) {
            self.pos += 1;
            return Ok(Vec::new());
        }
        let mut args = Vec::new();
        loop {
            args.push(match self.peek() {
                Some(Token::Semi | Token::RParen) => Expr::Empty,
                _ => self.expr(0)?,
            });
            match self.next() {
                Some(Token::Semi) => {}
                Some(Token::RParen) => return Ok(args),
                _ => {
                    self.pos -= 1;
                    return self.fail("expected `;` or `)`");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse and print again — the shortest way to assert a *shape*, since the serialiser
    /// parenthesises by precedence rather than copying the input's brackets.
    fn shape(formula: &str) -> String {
        parse(formula).expect(formula).to_string()
    }

    #[test]
    fn the_intro_and_its_namespace_prefix_are_stripped() {
        assert_eq!(shape("of:=1+1"), "1+1");
        assert_eq!(shape("=1+1"), "1+1");
        assert_eq!(shape("==1+1"), "1+1"); // forced recalculation (§5.2)
        assert_eq!(shape("1+1"), "1+1"); // Intro is optional
    }

    #[test]
    fn an_equals_that_is_an_operator_is_not_an_intro() {
        assert_eq!(shape("[.A1]=\"x\""), "[.A1]=\"x\"");
    }

    #[test]
    fn precedence_follows_table_1() {
        assert_eq!(shape("=1+2*3"), "1+2*3");
        assert_eq!(shape("=(1+2)*3"), "(1+2)*3"); // §5.5 Note 3: parentheses are kept
        assert_eq!(shape("=1&2=3"), "1&2=3");
        assert_eq!(shape("=1-2-3"), "1-2-3"); // left-associative
    }

    #[test]
    fn prefix_minus_binds_tighter_than_power_and_power_is_left_associative() {
        // Both are §5.5 Note 1, and both are the opposite of most languages: `-2^2` is 4
        // and `2^3^2` is 64. Getting either wrong is silent and wrong by a lot.
        assert_eq!(
            parse("=-2^2").unwrap(),
            Expr::Binary(
                Op::Pow,
                Box::new(Expr::Prefix(Op::Sub, Box::new(Expr::Number(2.0)))),
                Box::new(Expr::Number(2.0))
            )
        );
        // Left-associative, so the *first* `^` is the inner one — and the text needs no
        // brackets to say so, which is why this asserts the tree and not the printout.
        assert_eq!(
            parse("=2^3^2").unwrap(),
            Expr::Binary(
                Op::Pow,
                Box::new(Expr::Binary(
                    Op::Pow,
                    Box::new(Expr::Number(2.0)),
                    Box::new(Expr::Number(3.0))
                )),
                Box::new(Expr::Number(2.0))
            )
        );
    }

    #[test]
    fn percent_is_postfix_and_applies_to_expressions() {
        assert_eq!(shape("=[.B1]%"), "[.B1]%");
        assert_eq!(shape("=1+2%"), "1+2%");
    }

    #[test]
    fn a_call_may_take_no_parameters_or_empty_ones() {
        assert_eq!(
            parse("=TRUE()").unwrap(),
            Expr::Call {
                name: "TRUE".into(),
                args: vec![]
            }
        );
        // §5.6: an empty parameter is a parameter. Dropping it renumbers every one after it.
        assert_eq!(shape("=OFFSET([.A1];1;;2)"), "OFFSET([.A1];1;;2)");
        assert_eq!(shape("=F(;)"), "F(;)");
    }

    #[test]
    fn ranges_and_intersections_bind_tighter_than_arithmetic() {
        assert_eq!(shape("=SUM([.A1]:[.B2])"), "SUM([.A1]:[.B2])");
        assert_eq!(shape("=[.A1:.C4]![.B1:.B5]"), "[.A1:.C4]![.B1:.B5]");
        assert_eq!(shape("=1+[.A1]:[.B2]"), "1+[.A1]:[.B2]");
    }

    #[test]
    fn syntax_errors_are_errors_rather_than_a_guess() {
        for formula in [
            "=1+", "=(1", "=SUM(1;", "=)",
            "=SUM 1)", // §5.14: whitespace may not separate a name from its `(`
            "={1;2}",  // inline arrays are out of scope (§2.3.2)
            "=1 2",
        ] {
            assert!(parse(formula).is_err(), "{formula} should not parse");
        }
    }

    #[test]
    fn parse_serialize_parse_is_a_fixed_point_for_random_expressions() {
        // The plan's property test (doc/plan.md, phase 4 step 2). Whitespace and number
        // spelling are normalised by the first pass, so the *second* pass is where the
        // fixed point starts: parse(s) == parse(print(parse(s))) for everything below.
        let mut seed = 0x853C_49E6_748F_EA9Bu64;
        let mut rand = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let leaves = [
            "1",
            "0.5",
            "1.5E+20",
            "\"a\"\"b\"",
            "#N/A",
            "[.A1]",
            "[$'Sheet 2'.$B$7]",
            "[.A1:.C9]",
            "MyName",
            "SUM([.A1:.B2];3)",
            "TRUE()",
            "COM.MICROSOFT.X(1)",
        ];
        let ops = [
            "+", "-", "*", "/", "^", "&", "=", "<>", "<=", ":", "!", "%", "-",
        ];
        for _ in 0..500 {
            let mut formula = leaves[(rand() % leaves.len() as u64) as usize].to_owned();
            for _ in 0..(rand() % 4) {
                let op = ops[(rand() % ops.len() as u64) as usize];
                formula = match op {
                    "%" => format!("({formula})%"),
                    "-" if rand() % 2 == 0 => format!("-({formula})"),
                    op => format!(
                        "({formula}){op}{}",
                        leaves[(rand() % leaves.len() as u64) as usize]
                    ),
                };
            }
            let Ok(first) = parse(&formula) else { continue };
            let printed = first.to_string();
            let second = parse(&printed).unwrap_or_else(|e| panic!("{printed}: {e}"));
            assert_eq!(first, second, "{formula}");
            assert_eq!(printed, second.to_string(), "{formula}");
        }
    }
}
