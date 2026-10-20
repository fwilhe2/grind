// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The fence (`doc/formula-assistant.md` decision 3, §4.3): what a model is *able* to write,
//! as a grammar its runtime enforces token by token.
//!
//! A1 measured why it is needed: SmolLM3, told to separate arguments with `;` and given every
//! function's signature, still wrote Excel — `,` between arguments, `Sheet!A1`, `VLOOKUP` over a
//! header that is not a reference — and two thirds of its greedy answers did not parse. A fence
//! makes those unwritable rather than rejected afterwards.
//!
//! The grammar is built **per request**, from the engine and the document: the functions are
//! [`funcs::implemented`], the sheets are the document's own sheet names and the names are its
//! defined names. So a function this build lacks, a sheet that does not exist and a name nobody
//! defined cannot be produced at all. It is deliberately looser than the parser everywhere else —
//! a fence, not a second definition of display syntax — and [`crate::check`] still parses every
//! candidate.
//!
//! One grammar, two uses: [`Fence::gbnf`] is the text llama.cpp's server takes, and
//! [`Fence::accepts`] reads the same rules, so the test that every sample formula is accepted is a
//! test of exactly the text the server is sent.

use std::collections::HashMap;

use grind_sheet::assist_context::AssistContext;
use grind_sheet::formula::funcs;

/// A grammar expression.
#[derive(Clone, Debug)]
enum G {
    Lit(String),
    /// A character class: inclusive ranges, or everything outside them.
    Class(Vec<(char, char)>, bool),
    Rule(&'static str),
    Seq(Vec<G>),
    Alt(Vec<G>),
    /// At least `min`, at most `max` (`None`: unbounded).
    Rep(Box<G>, u32, Option<u32>),
}

fn lit(s: &str) -> G {
    G::Lit(s.to_owned())
}
fn rule(name: &'static str) -> G {
    G::Rule(name)
}
fn seq(items: impl IntoIterator<Item = G>) -> G {
    G::Seq(items.into_iter().collect())
}
fn alt(items: impl IntoIterator<Item = G>) -> G {
    G::Alt(items.into_iter().collect())
}
fn opt(g: G) -> G {
    G::Rep(Box::new(g), 0, Some(1))
}
fn star(g: G) -> G {
    G::Rep(Box::new(g), 0, None)
}
fn class(ranges: &[(char, char)]) -> G {
    G::Class(ranges.to_vec(), false)
}

/// The grammar for one request.
pub struct Fence {
    rules: Vec<(&'static str, G)>,
}

impl Fence {
    /// The fence for a formula in the document `context` shows. The grammar starts *after* the
    /// `=`, which the backend prefills.
    pub fn for_context(context: &AssistContext) -> Fence {
        let sheets: Vec<&str> = context
            .sheets
            .iter()
            .map(|s| s.name.as_str())
            .chain(context.sheets_left_out.iter().map(String::as_str))
            .collect();
        let names: Vec<&str> = context.names.iter().map(|(n, _)| n.as_str()).collect();
        Fence::new(&sheets, &names)
    }

    fn new(sheets: &[&str], names: &[&str]) -> Fence {
        let ws = || opt(lit(" "));
        let digit = || class(&[('0', '9')]);
        let mut functions: Vec<&str> = funcs::implemented().to_vec();
        // Longest first, which costs nothing to llama.cpp's parser and keeps the rendered text
        // readable: `SUMIFS` before `SUMIF` before `SUM`.
        functions.sort_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));

        let mut primary = vec![
            rule("number"),
            rule("string"),
            rule("call"),
            rule("ref"),
            lit("TRUE"),
            lit("FALSE"),
            seq([lit("("), ws(), rule("expr"), ws(), lit(")")]),
        ];
        if !names.is_empty() {
            primary.push(rule("name"));
        }
        let mut rules = vec![
            ("root", rule("expr")),
            (
                "expr",
                seq([
                    rule("concat"),
                    opt(seq([ws(), rule("cmpop"), ws(), rule("concat")])),
                ]),
            ),
            ("cmpop", alt(["<=", ">=", "<>", "=", "<", ">"].map(lit))),
            (
                "concat",
                seq([rule("add"), star(seq([ws(), lit("&"), ws(), rule("add")]))]),
            ),
            (
                "add",
                seq([
                    rule("mul"),
                    star(seq([
                        ws(),
                        class(&[('+', '+'), ('-', '-')]),
                        ws(),
                        rule("mul"),
                    ])),
                ]),
            ),
            (
                "mul",
                seq([
                    rule("pow"),
                    star(seq([
                        ws(),
                        class(&[('*', '*'), ('/', '/')]),
                        ws(),
                        rule("pow"),
                    ])),
                ]),
            ),
            (
                "pow",
                seq([
                    rule("unary"),
                    star(seq([ws(), lit("^"), ws(), rule("unary")])),
                ]),
            ),
            (
                "unary",
                seq([opt(lit("-")), rule("primary"), opt(lit("%"))]),
            ),
            ("primary", alt(primary)),
            (
                "number",
                seq([
                    G::Rep(Box::new(digit()), 1, Some(15)),
                    opt(seq([lit("."), G::Rep(Box::new(digit()), 1, Some(15))])),
                    opt(seq([
                        class(&[('E', 'E'), ('e', 'e')]),
                        opt(class(&[('+', '+'), ('-', '-')])),
                        G::Rep(Box::new(digit()), 1, Some(3)),
                    ])),
                ]),
            ),
            (
                "string",
                seq([
                    lit("\""),
                    G::Rep(
                        Box::new(alt([
                            G::Class(vec![('"', '"'), ('\n', '\n')], true),
                            lit("\"\""),
                        ])),
                        0,
                        Some(200),
                    ),
                    lit("\""),
                ]),
            ),
            (
                "call",
                seq([
                    rule("function"),
                    lit("("),
                    ws(),
                    opt(seq([
                        rule("expr"),
                        star(seq([ws(), lit(";"), ws(), rule("expr")])),
                    ])),
                    ws(),
                    lit(")"),
                ]),
            ),
            ("function", alt(functions.iter().map(|f| lit(f)))),
            (
                "ref",
                seq([
                    opt(seq([rule("sheet"), lit(".")])),
                    rule("cell"),
                    opt(seq([
                        lit(":"),
                        opt(seq([rule("sheet"), lit(".")])),
                        rule("cell"),
                    ])),
                ]),
            ),
            (
                "cell",
                seq([
                    opt(lit("$")),
                    G::Rep(Box::new(class(&[('A', 'Z')])), 1, Some(3)),
                    opt(lit("$")),
                    class(&[('1', '9')]),
                    G::Rep(Box::new(digit()), 0, Some(6)),
                ]),
            ),
            ("sheet", alt(sheets.iter().map(|s| lit(&sheet_spelling(s))))),
        ];
        if !names.is_empty() {
            rules.push(("name", alt(names.iter().map(|n| lit(n)))));
        }
        Fence { rules }
    }

    /// The grammar in llama.cpp's GBNF, one rule a line.
    pub fn gbnf(&self) -> String {
        self.rules
            .iter()
            .map(|(name, g)| format!("{name} ::= {}\n", render(g)))
            .collect()
    }

    /// Whether the grammar accepts `text` whole — what a runtime would let a model write. A
    /// small recogniser over the same rules [`Fence::gbnf`] renders, for tests.
    pub fn accepts(&self, text: &str) -> bool {
        let chars: Vec<char> = text.chars().collect();
        let rules: HashMap<&str, &G> = self.rules.iter().map(|(n, g)| (*n, g)).collect();
        let mut memo = HashMap::new();
        ends(&rules, &mut memo, &rule("root"), &chars, 0).contains(&chars.len())
    }
}

/// How a sheet is named in a reference: bare when it is a plain word, quoted otherwise.
fn sheet_spelling(name: &str) -> String {
    let plain = name
        .chars()
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_alphanumeric() || c == '_');
    match plain {
        true => name.to_owned(),
        false => format!("'{}'", name.replace('\'', "''")),
    }
}

fn render(g: &G) -> String {
    match g {
        G::Lit(s) => {
            let escaped: String = s
                .chars()
                .map(|c| match c {
                    '"' => "\\\"".to_owned(),
                    '\\' => "\\\\".to_owned(),
                    '\n' => "\\n".to_owned(),
                    c => c.to_string(),
                })
                .collect();
            format!("\"{escaped}\"")
        }
        G::Class(ranges, negated) => {
            let mut out = String::from("[");
            if *negated {
                out.push('^');
            }
            for &(from, to) in ranges {
                out += &class_char(from);
                if from != to {
                    out.push('-');
                    out += &class_char(to);
                }
            }
            out.push(']');
            out
        }
        G::Rule(name) => (*name).to_owned(),
        G::Seq(items) => items
            .iter()
            .map(|i| match i {
                G::Alt(_) => format!("({})", render(i)),
                _ => render(i),
            })
            .collect::<Vec<_>>()
            .join(" "),
        G::Alt(items) => items.iter().map(render).collect::<Vec<_>>().join(" | "),
        G::Rep(inner, min, max) => {
            let inner = match **inner {
                G::Seq(_) | G::Alt(_) => format!("({})", render(inner)),
                _ => render(inner),
            };
            match (min, max) {
                (0, Some(1)) => format!("{inner}?"),
                (0, None) => format!("{inner}*"),
                (1, None) => format!("{inner}+"),
                (min, Some(max)) => format!("{inner}{{{min},{max}}}"),
                (min, None) => format!("{inner}{{{min},}}"),
            }
        }
    }
}

fn class_char(c: char) -> String {
    match c {
        '\n' => "\\n".into(),
        ']' | '\\' | '^' | '-' => format!("\\{c}"),
        c => c.to_string(),
    }
}

/// Every position a match of `g` starting at `at` can end at.
fn ends(
    rules: &HashMap<&str, &G>,
    memo: &mut HashMap<(&'static str, usize), Vec<usize>>,
    g: &G,
    text: &[char],
    at: usize,
) -> Vec<usize> {
    let mut out = match g {
        G::Lit(s) => {
            let s: Vec<char> = s.chars().collect();
            match text.get(at..at + s.len()) {
                Some(slice) if slice == s.as_slice() => vec![at + s.len()],
                _ => vec![],
            }
        }
        G::Class(ranges, negated) => match text.get(at) {
            Some(&c) if ranges.iter().any(|&(f, t)| f <= c && c <= t) != *negated => vec![at + 1],
            _ => vec![],
        },
        G::Rule(name) => {
            if let Some(known) = memo.get(&(*name, at)) {
                return known.clone();
            }
            // No rule here is left-recursive, so a rule never re-enters itself at one position;
            // the empty entry only guards against that assumption breaking silently.
            memo.insert((*name, at), vec![]);
            let found = ends(rules, memo, rules[name], text, at);
            memo.insert((*name, at), found.clone());
            found
        }
        G::Seq(items) => {
            let mut here = vec![at];
            for item in items {
                let mut next = Vec::new();
                for p in here {
                    next.extend(ends(rules, memo, item, text, p));
                }
                next.sort_unstable();
                next.dedup();
                here = next;
            }
            here
        }
        G::Alt(items) => items
            .iter()
            .flat_map(|i| ends(rules, memo, i, text, at))
            .collect(),
        G::Rep(inner, min, max) => {
            let mut all = Vec::new();
            let mut here = vec![at];
            let mut count = 0;
            loop {
                if count >= *min {
                    all.extend(&here);
                }
                if max.is_some_and(|m| count >= m) || here.is_empty() {
                    break;
                }
                let mut next = Vec::new();
                for p in &here {
                    next.extend(
                        ends(rules, memo, inner, text, *p)
                            .into_iter()
                            .filter(|e| e > p),
                    );
                }
                next.sort_unstable();
                next.dedup();
                here = next;
                count += 1;
            }
            all
        }
    };
    out.sort_unstable();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fence() -> Fence {
        Fence::new(&["Ledger", "Garden", "My Sheet"], &["tax_rate"])
    }

    #[test]
    fn display_syntax_is_let_through() {
        let fence = fence();
        for ok in [
            "SUM(Ledger.D2:D14)",
            "SUMIFS(Ledger.D2:D14;Ledger.A2:A14;\">=\"&DATE(2026;9;15);Ledger.D2:D14;\"<0\")",
            "-LOG10(Garden.B15)",
            "Garden.B13*POWER(1+Garden.B14;6)",
            "SQRT(Garden.B5^2+Garden.B6^2)",
            "COUNTIF(Ledger.D2:D6;TRUE())",
            "PI()*Garden.B2^2",
            "'My Sheet'.A1*tax_rate",
            "IF(A1 > 0; \"say \"\"hi\"\"\"; 1.5E-3)",
            "$A$1:B$2",
            "COM.MICROSOFT.CEILING(Ledger.C2*1.03;5)",
        ] {
            assert!(fence.accepts(ok), "{ok}");
        }
    }

    #[test]
    fn excel_and_invention_are_not() {
        let fence = fence();
        for refused in [
            "SUM(A1,A2)",
            "Ledger!D2",
            "SUM(Sheet1.A1)",
            "VLOOKUP(\"x\";Shipping.From kg;2)",
            "SUMMATION(A1:A3)",
            "SUM(A1:A3) ",
            "IF(A1;...;0)",
            "net_income*2",
            "",
        ] {
            assert!(!fence.accepts(refused), "{refused}");
        }
    }

    #[test]
    fn every_rule_named_is_defined_and_the_text_is_one_rule_a_line() {
        let fence = fence();
        let gbnf = fence.gbnf();
        assert_eq!(gbnf.lines().count(), fence.rules.len());
        let defined: Vec<&str> = fence.rules.iter().map(|(n, _)| *n).collect();
        assert!(defined.contains(&"root"));
        // Outside quotes and classes, every lowercase word on a right-hand side is a rule.
        for line in gbnf.lines() {
            let body = line.split_once(" ::= ").unwrap().1;
            let mut bare = String::new();
            let (mut quoted, mut classed, mut escaped) = (false, false, false);
            for c in body.chars() {
                match (quoted, classed, escaped, c) {
                    (_, _, true, _) => escaped = false,
                    (true, _, _, '\\') | (_, true, _, '\\') => escaped = true,
                    (false, false, _, '"') => quoted = true,
                    (true, _, _, '"') => quoted = false,
                    (false, false, _, '[') => classed = true,
                    (_, true, _, ']') => classed = false,
                    (false, false, _, c) => bare.push(c),
                    _ => {}
                }
            }
            for word in bare.split(|c: char| !c.is_ascii_lowercase()) {
                assert!(
                    word.is_empty() || defined.contains(&word),
                    "{word} in {line}"
                );
            }
        }
        assert!(gbnf.contains("function ::= \"COM.MICROSOFT.CEILING\" | "));
        assert!(gbnf.contains("sheet ::= \"Ledger\" | \"Garden\" | \"'My Sheet'\""));
        assert!(gbnf.contains("string ::= \"\\\"\" ([^\"\\n] | \"\\\"\\\"\"){0,200} \"\\\"\""));
    }
}
