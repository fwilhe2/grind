// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A read-only, IDE-flavoured rendering of a formula: full names instead of abbreviations,
//! one argument per line once a call stops fitting on one, and each argument labelled with
//! the parameter it fills — `PresentValue(Rate: 0.05, ...)` rather than `PV(0.05, ...)`.
//!
//! This is presentation only. [`explain`] never round-trips: the string it returns is not
//! accepted back by [`super::display::from_display`] or [`super::parse::parse`], and nothing
//! written to a document is affected — the formula a `.ods` stores stays exactly the
//! abbreviated, single-line form ODF spells (CLAUDE.md's R1/R6). An editable version of this
//! view is a separate, later feature: it needs a parameter-label syntax that cannot be
//! confused with `=` used as a comparison operator inside an argument (`IF(A1=B1, ...)`),
//! which `explain`'s output does not attempt to be unambiguous about.
//!
//! Parameter names come from the catalog's `Syntax:` line, split the same way
//! `ui_gtk`'s `signature_markup` splits it: every parameter's declaration is separated by
//! exactly one `;`, however deeply its optionality brackets it, so a naive split still lines
//! up one part per parameter. Multi-line layout only ever unfolds a [`Expr::Call`] that is
//! itself the whole expression or a bare argument of another call — a call buried inside an
//! arithmetic expression (`1+PV(...)`) prints inline via the ordinary [`Bare`] printer, which
//! is the one place this module does not attempt full IDE-style reflow. ponytail: expanding
//! *that* case needs the precedence-aware bracketing `serialize::child`/`binding_power`
//! already do privately; raise the ceiling by exposing those if operator-nested calls turn
//! out to need it too.

use super::funcs;
use super::lex::SyntaxError;
use super::parse::{Expr, parse};
use super::serialize::Bare;

/// How wide a call is allowed to stay on one line before it unfolds.
const WIDTH: usize = 60;

/// The friendly name for an abbreviated function name, e.g. `PV` → `Present Value`.
///
/// Ours, not the spec's — §5.6 is under no obligation to be readable, and clean-room rules
/// don't apply to naming we invented rather than read out of LibreOffice.
pub fn alias(name: &str) -> Option<&'static str> {
    ALIASES
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, a)| *a)
}

/// Render a formula (canonical `of:=…` form, or anything [`parse`] accepts) as multi-line,
/// aliased, parameter-labelled text.
pub fn explain(formula: &str) -> Result<String, SyntaxError> {
    Ok(render(&parse(formula)?, 0, WIDTH))
}

/// The same rendering, never unfolded — for a place with one line to spend, like a formula
/// bar. Aliases and labels are the point; the layout is what does not fit there.
pub fn explain_inline(formula: &str) -> Result<String, SyntaxError> {
    Ok(render(&parse(formula)?, 0, usize::MAX))
}

/// The friendly spelling of a function's signature: its aliased name, and one label per
/// parameter in order, with `…` on the last when it repeats.
///
/// What a shell's signature hint shows while a call is being typed — the same names
/// [`explain`] labels the finished formula with, so the hint and the explanation agree.
pub fn signature(name: &str) -> Option<(String, Vec<String>)> {
    let info = funcs::catalog()
        .iter()
        .find(|info| info.name.eq_ignore_ascii_case(name))?;
    let (raw, repeating) = signature_params(info.signature);
    let mut labels: Vec<String> = raw.iter().map(|raw| param_alias(info.name, raw)).collect();
    if repeating && let Some(last) = labels.last_mut() {
        last.push('…');
    }
    Some((alias(info.name).unwrap_or(info.name).to_owned(), labels))
}

fn render(e: &Expr, indent: usize, width: usize) -> String {
    match e {
        Expr::Call { name, args } => render_call(name, args, indent, width),
        _ => Bare(e).to_string(),
    }
}

fn render_call(name: &str, args: &[Expr], indent: usize, width: usize) -> String {
    let head = alias(name).unwrap_or(name);
    let labels = param_labels(name, args.len());
    let items: Vec<String> = args
        .iter()
        .zip(labels)
        .map(|(arg, label)| {
            let value = render(arg, indent + 1, width);
            match label {
                Some(label) if !matches!(arg, Expr::Empty) => format!("{label}: {value}"),
                _ => value,
            }
        })
        .collect();

    let inline = format!("{head}({})", items.join(", "));
    if inline.chars().count() <= width && !inline.contains('\n') {
        return inline;
    }

    let pad = "  ".repeat(indent + 1);
    let close_pad = "  ".repeat(indent);
    format!(
        "{head}(\n{pad}{}\n{close_pad})",
        items.join(&format!(",\n{pad}"))
    )
}

/// The parameter name for each of `count` actual arguments to `name`, from the catalog's
/// signature — `None` per argument for a function the catalog does not know.
fn param_labels(name: &str, count: usize) -> Vec<Option<String>> {
    let Some(info) = funcs::catalog()
        .iter()
        .find(|info| info.name.eq_ignore_ascii_case(name))
    else {
        return vec![None; count];
    };
    let (names, repeating) = signature_params(info.signature);
    if names.is_empty() {
        return vec![None; count];
    }
    let fixed = if repeating {
        names.len() - 1
    } else {
        names.len()
    };
    (0..count)
        .map(|i| {
            if i < fixed {
                Some(param_alias(name, &names[i]))
            } else if repeating {
                let base = param_alias(name, names.last().expect("repeating implies a last name"));
                let k = i - fixed + 1;
                Some(if count - fixed == 1 {
                    base
                } else {
                    format!("{base} {k}")
                })
            } else {
                None
            }
        })
        .collect()
}

/// The friendly name for a catalog parameter token, e.g. `Pv` → `Present Value`.
///
/// A handful of single-letter tokens (`D`, `F`, `T`, …) are reused by the spec for different
/// things in different functions — `D` is a database in every `D*` function but the date in
/// `DAY`/`WEEKDAY`/`YEAR`. [`OVERRIDES`] is checked first for exactly those; everything else
/// is one global, context-free mapping, which is a deliberate simplification: this is a
/// display convenience, not a second source of truth about what a parameter means.
fn param_alias(func: &str, raw: &str) -> String {
    if let Some((_, _, alias)) = OVERRIDES
        .iter()
        .find(|(f, n, _)| f.eq_ignore_ascii_case(func) && *n == raw)
    {
        return (*alias).to_string();
    }
    PARAM_ALIASES
        .iter()
        .find(|(n, _)| *n == raw)
        .map(|(_, a)| (*a).to_string())
        .unwrap_or_else(|| raw.to_string())
}

/// A catalog `Syntax:` line, split into its ordered parameter names and whether the last one
/// repeats (`{ … } +`).
fn signature_params(signature: &str) -> (Vec<String>, bool) {
    let Some((_, rest)) = signature.split_once('(') else {
        return (Vec::new(), false);
    };
    let rest = rest.strip_suffix(')').unwrap_or(rest).trim();
    if rest.is_empty() {
        return (Vec::new(), false);
    }
    let parts: Vec<&str> = rest.split(';').collect();
    let repeating = parts
        .last()
        .is_some_and(|part| part.trim_end().ends_with('+'));
    let names = parts.iter().filter_map(|part| param_name(part)).collect();
    (names, repeating)
}

/// One parameter's name out of its declaration, e.g. `" [ Number Fv = 0 ] [ "` → `Fv`: the
/// last identifier before an `=` default (if any), with the optionality brackets and
/// repetition braces stripped.
fn param_name(part: &str) -> Option<String> {
    let cleaned: String = part.chars().filter(|c| !"[]{}+".contains(*c)).collect();
    let before_default = cleaned.split('=').next().unwrap_or(&cleaned);
    before_default.split_whitespace().last().map(str::to_owned)
}

/// One entry per catalog function (checked by a test below), a plain-English name for its
/// abbreviation.
static ALIASES: &[(&str, &str)] = &[
    ("ABS", "Absolute Value"),
    ("ACOS", "Arc Cosine"),
    ("AND", "And"),
    ("ASIN", "Arc Sine"),
    ("ATAN", "Arc Tangent"),
    ("ATAN2", "Arc Tangent (2 Argument)"),
    ("AVERAGE", "Average"),
    ("AVERAGEIF", "Average If"),
    ("CHOOSE", "Choose"),
    ("COLUMN", "Column"),
    ("COLUMNS", "Column Count"),
    ("COS", "Cosine"),
    ("COUNT", "Count Numbers"),
    ("COUNTA", "Count All"),
    ("COUNTBLANK", "Count Blank"),
    ("COUNTIF", "Count If"),
    ("DATE", "Date"),
    ("DAVERAGE", "Database Average"),
    ("DAY", "Day"),
    ("DCOUNT", "Database Count"),
    ("DCOUNTA", "Database Count All"),
    ("DDB", "Declining Balance Depreciation"),
    ("DEGREES", "Radians To Degrees"),
    ("DGET", "Database Get"),
    ("DMAX", "Database Maximum"),
    ("DMIN", "Database Minimum"),
    ("DPRODUCT", "Database Product"),
    ("DSTDEV", "Database Standard Deviation"),
    ("DSTDEVP", "Database Standard Deviation (Population)"),
    ("DSUM", "Database Sum"),
    ("DVAR", "Database Variance"),
    ("DVARP", "Database Variance (Population)"),
    ("EVEN", "Round Up To Even"),
    ("EXACT", "Text Equals (Case-Sensitive)"),
    ("EXP", "Exponential"),
    ("FACT", "Factorial"),
    ("FALSE", "False"),
    ("FIND", "Find Text"),
    ("FV", "Future Value"),
    ("HLOOKUP", "Horizontal Lookup"),
    ("HOUR", "Hour"),
    ("IF", "If"),
    ("INDEX", "Index"),
    ("INT", "Integer Part"),
    ("IRR", "Internal Rate Of Return"),
    ("ISBLANK", "Is Blank"),
    ("ISERR", "Is Error (Not #N/A)"),
    ("ISERROR", "Is Error"),
    ("ISLOGICAL", "Is Logical"),
    ("ISNA", "Is #N/A"),
    ("ISNONTEXT", "Is Not Text"),
    ("ISNUMBER", "Is Number"),
    ("ISTEXT", "Is Text"),
    ("LEFT", "Left Characters"),
    ("LEN", "Length"),
    ("LN", "Natural Logarithm"),
    ("LOG", "Logarithm"),
    ("LOG10", "Logarithm (Base 10)"),
    ("LOWER", "Lowercase"),
    ("MATCH", "Match Position"),
    ("MAX", "Maximum"),
    ("MID", "Middle Characters"),
    ("MIN", "Minimum"),
    ("MINUTE", "Minute"),
    ("MOD", "Modulo"),
    ("MONTH", "Month"),
    ("N", "To Number"),
    ("NA", "Not Available"),
    ("NOT", "Not"),
    ("NOW", "Now"),
    ("NPER", "Number Of Periods"),
    ("NPV", "Net Present Value"),
    ("ODD", "Round Up To Odd"),
    ("OR", "Or"),
    ("PI", "Pi"),
    ("PMT", "Payment"),
    ("POWER", "Power"),
    ("PRODUCT", "Product"),
    ("PROPER", "Title Case"),
    ("PV", "Present Value"),
    ("RADIANS", "Degrees To Radians"),
    ("RATE", "Interest Rate"),
    ("REPLACE", "Replace Characters"),
    ("REPT", "Repeat Text"),
    ("RIGHT", "Right Characters"),
    ("ROUND", "Round"),
    ("ROW", "Row"),
    ("ROWS", "Row Count"),
    ("SECOND", "Second"),
    ("SIN", "Sine"),
    ("SLN", "Straight-Line Depreciation"),
    ("SQRT", "Square Root"),
    ("STDEV", "Standard Deviation"),
    ("STDEVP", "Standard Deviation (Population)"),
    ("SUBSTITUTE", "Substitute Text"),
    ("SUM", "Sum"),
    ("SUMIF", "Sum If"),
    ("SYD", "Sum-Of-Years-Digits Depreciation"),
    ("T", "To Text"),
    ("TAN", "Tangent"),
    ("TIME", "Time"),
    ("TODAY", "Today"),
    ("TRIM", "Trim Spaces"),
    ("TRUE", "True"),
    ("TRUNC", "Truncate"),
    ("UPPER", "Uppercase"),
    ("VALUE", "Text To Number"),
    ("VAR", "Variance"),
    ("VARP", "Variance (Population)"),
    ("VLOOKUP", "Vertical Lookup"),
    ("WEEKDAY", "Weekday"),
    ("YEAR", "Year"),
];

/// `(function, raw token)` → alias, for the handful of single-letter tokens the spec reuses
/// with a different meaning per function. Checked before [`PARAM_ALIASES`].
static OVERRIDES: &[(&str, &str, &str)] = &[
    ("DAY", "D", "Date"),
    ("WEEKDAY", "D", "Date"),
    ("YEAR", "D", "Date"),
    ("FACT", "F", "Number"),
    ("HOUR", "T", "Time"),
    ("MINUTE", "T", "Time"),
    ("SECOND", "T", "Time"),
];

/// One entry per raw parameter token the catalog's signatures produce (checked by a test
/// below), a plain-English name for it. Single-letter tokens get the meaning they carry in
/// the majority of the functions that use them — [`OVERRIDES`] covers the rest.
static PARAM_ALIASES: &[(&str, &str)] = &[
    ("A", "Number"),
    ("AnyValue", "Value"),
    ("AreaNumber", "Area Number"),
    ("B", "Number"),
    ("Base", "Base"),
    ("C", "Criterion"),
    ("Column", "Column"),
    ("Condition", "Condition"),
    ("Cost", "Cost"),
    ("Count", "Count"),
    ("D", "Database"),
    ("DataSource", "Data Source"),
    ("Date", "Date"),
    ("Day", "Day"),
    ("DeclinationFactor", "Declination Factor"),
    ("Digits", "Digits"),
    ("F", "Field"),
    ("Fv", "Future Value"),
    ("Guess", "Guess"),
    ("Hours", "Hours"),
    ("IfFalse", "If False"),
    ("IfTrue", "If True"),
    ("Index", "Index"),
    ("L", "Logical Value"),
    ("Length", "Length"),
    ("LifeTime", "Life Time"),
    ("Lookup", "Lookup Value"),
    ("MatchType", "Match Type"),
    ("Minutes", "Minutes"),
    ("Month", "Month"),
    ("N", "Number"),
    ("New", "New Text"),
    ("Nper", "Number Of Periods"),
    ("Old", "Old Text"),
    ("PayType", "Payment Type"),
    ("Payment", "Payment"),
    ("Period", "Period"),
    ("Pv", "Present Value"),
    ("R", "Reference"),
    ("RangeLookup", "Range Lookup"),
    ("Rate", "Rate"),
    ("Row", "Row"),
    ("S", "Reference"),
    ("Salvage", "Salvage Value"),
    ("Search", "Search"),
    ("SearchRegion", "Search Region"),
    ("Seconds", "Seconds"),
    ("Start", "Start"),
    ("T", "Text"),
    ("T1", "Text 1"),
    ("T2", "Text 2"),
    ("Type", "Type"),
    ("Value", "Value"),
    ("Values", "Values"),
    ("Which", "Occurrence"),
    ("X", "Value"),
    ("Year", "Year"),
    ("x", "X"),
    ("y", "Y"),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// A missing alias would fall back to the abbreviated name silently, so the thing worth
    /// checking is that every catalogued function has one.
    #[test]
    fn every_catalogued_function_has_an_alias() {
        for info in funcs::catalog() {
            assert!(alias(info.name).is_some(), "{} has no alias", info.name);
        }
    }

    #[test]
    fn simple_calls_stay_on_one_line_with_labelled_parameters() {
        assert_eq!(
            explain("of:=ROUND([.A1];2)").unwrap(),
            "Round(Value: A1, Digits: 2)"
        );
    }

    #[test]
    fn a_wide_call_unfolds_one_argument_per_line() {
        let out = explain("of:=RATE(12;-100;1000;0;0;0.05)").unwrap();
        assert_eq!(
            out,
            "Interest Rate(\n  Number Of Periods: 12,\n  Payment: -100,\n  Present Value: 1000,\n  Future Value: 0,\n  Payment Type: 0,\n  Guess: 0.05\n)"
        );
    }

    #[test]
    fn variadic_arguments_are_numbered_past_the_first() {
        assert_eq!(
            explain("of:=SUM(1;2;3)").unwrap(),
            "Sum(Number 1: 1, Number 2: 2, Number 3: 3)"
        );
        assert_eq!(explain("of:=SUM(1)").unwrap(), "Sum(Number: 1)");
    }

    #[test]
    fn nested_calls_that_are_call_arguments_unfold_too() {
        let out = explain("of:=IF([.A1]>0;ROUND([.A1];2);0)").unwrap();
        assert_eq!(
            out,
            "If(\n  Condition: A1>0,\n  If True: Round(Value: A1, Digits: 2),\n  If False: 0\n)"
        );
    }

    #[test]
    fn a_call_nested_inside_an_arithmetic_expression_stays_inline() {
        // The documented ceiling: only a Call that is the whole expression or a bare call
        // argument unfolds. One buried inside `+` prints through the ordinary Bare printer.
        assert_eq!(explain("of:=1+PV(1;2;3)").unwrap(), "1+PV(1;2;3)");
    }

    #[test]
    fn an_omitted_argument_is_not_mislabelled_with_a_stray_colon() {
        assert_eq!(
            explain("of:=IF([.A1];;2)").unwrap(),
            "If(Condition: A1, , If False: 2)"
        );
    }

    /// A missing entry would fall back silently to the raw catalog token, so the thing worth
    /// checking is that every token [`signature_params`] can produce across the whole catalog
    /// is a known key of [`PARAM_ALIASES`] — [`param_alias`]'s own fallback can't tell "no
    /// entry" apart from "aliased to itself" the way this test can.
    #[test]
    fn every_catalogued_parameter_token_has_an_alias() {
        for info in funcs::catalog() {
            let (names, _) = signature_params(info.signature);
            for raw in names {
                assert!(
                    PARAM_ALIASES.iter().any(|(n, _)| *n == raw),
                    "{} has no alias for parameter {raw:?} of {}",
                    raw,
                    info.name
                );
            }
        }
    }

    #[test]
    fn the_inline_rendering_never_unfolds_however_wide_it_gets() {
        let out = explain_inline("of:=RATE(12;-100;1000;0;0;0.05)").unwrap();
        assert!(!out.contains('\n'), "{out}");
        assert!(
            out.starts_with("Interest Rate(Number Of Periods: 12, "),
            "{out}"
        );
    }

    #[test]
    fn a_signature_reads_in_the_same_names_the_explanation_labels_with() {
        let (head, params) = signature("RATE").expect("RATE is in the catalog");
        assert_eq!(head, "Interest Rate");
        assert_eq!(params[0], "Number Of Periods");
        assert_eq!(params[1], "Payment");

        // A repeating tail says so, and case does not matter.
        let (head, params) = signature("sum").expect("SUM is in the catalog");
        assert_eq!(head, "Sum");
        assert_eq!(params, vec!["Number…".to_owned()]);
        assert_eq!(signature("NOSUCHFUNCTION"), None);
    }

    #[test]
    fn unknown_functions_render_positionally() {
        assert_eq!(
            explain("of:=COM.MICROSOFT.X(1;2)").unwrap(),
            "COM.MICROSOFT.X(1, 2)"
        );
    }
}
