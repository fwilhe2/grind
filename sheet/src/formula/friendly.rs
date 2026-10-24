// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A read-only, IDE-flavoured rendering of a formula: full names instead of abbreviations, one
//! argument per line once a call stops fitting, and each argument labelled with its parameter —
//! `Present Value(Rate: 0.05, …)` rather than `PV(0.05; …)`.
//!
//! Presentation only: nothing here parses back, and the stored formula stays ODF's (R1). An
//! editable version would need a label syntax that cannot be confused with `=` as a comparison
//! inside an argument.
//!
//! Parameter names come from the catalog's `Syntax:` line, where each declaration is separated by
//! exactly one `;`. Only a call that is the whole expression or a bare argument of another call
//! unfolds; one inside an operator (`1+PV(…)`) prints inline. ponytail: unfolding those needs
//! `serialize`'s private precedence bracketing exposed.

use super::funcs;
use super::lex::SyntaxError;
use super::parse::{Expr, parse};
use super::serialize::Bare;

/// How wide a call is allowed to stay on one line before it unfolds.
const WIDTH: usize = 60;

/// The friendly name for an abbreviated function name, e.g. `PV` → `Present Value`. Ours, not the
/// spec's.
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

/// The same rendering, never unfolded, for one line such as a formula bar.
pub fn explain_inline(formula: &str) -> Result<String, SyntaxError> {
    Ok(render(&parse(formula)?, 0, usize::MAX))
}

/// The friendly spelling of a function's signature: its aliased name and one label per parameter,
/// `…` on the last when it repeats — the names [`explain`] uses, so a hint and an explanation
/// agree.
pub fn signature(name: &str) -> Option<(String, Vec<String>)> {
    let info = funcs::catalog()
        .iter()
        .find(|info| info.name.eq_ignore_ascii_case(name))?;
    let (raw, repeating) = signature_params(info.signature);
    let mut labels: Vec<String> = raw.iter().map(|raw| param_alias(info.name, raw)).collect();
    if repeating > 0
        && let Some(last) = labels.last_mut()
    {
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
    let (names, group) = signature_params(info.signature);
    if names.is_empty() {
        return vec![None; count];
    }
    let fixed = names.len() - group;
    (0..count)
        .map(|i| {
            if i < fixed {
                Some(param_alias(name, &names[i]))
            } else if group == 1 {
                let base = param_alias(name, names.last().expect("a group has a last name"));
                let k = i - fixed + 1;
                Some(if count - fixed == 1 {
                    base
                } else {
                    format!("{base} {k}")
                })
            } else if group > 1 {
                // A repeated *group* — `[ ; Reference R2 ; Criterion C2 ]...` — numbers its
                // members by pass: the second time round, `R2` labels the third range.
                let (j, pass) = ((i - fixed) % group, (i - fixed) / group);
                Some(renumber(&param_alias(name, &names[fixed + j]), pass))
            } else {
                None
            }
        })
        .collect()
}

/// `Range 2` on its `pass`th repetition past the first: `Range 2`, `Range 3`, … A label
/// with no number of its own is left as it is.
fn renumber(label: &str, pass: usize) -> String {
    let digits = label.len() - label.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    match label[label.len() - digits..].parse::<usize>() {
        Ok(first) => format!("{}{}", &label[..label.len() - digits], first + pass),
        Err(_) => label.to_owned(),
    }
}

/// How many parameters `name`'s signature lists, and how many at its end repeat as a group. `None`
/// for a function the catalog does not know.
pub fn shape(name: &str) -> Option<(usize, usize)> {
    let info = funcs::catalog()
        .iter()
        .find(|info| info.name.eq_ignore_ascii_case(name))?;
    let (names, group) = signature_params(info.signature);
    Some((names.len(), group))
}

/// The friendly name for a catalog parameter token, e.g. `Pv` → `Present Value`. The spec reuses a
/// few single letters per function (`D` is a database in `D*` functions and a date in `DAY`), so
/// [`OVERRIDES`] is checked first; everything else is one global mapping.
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

/// A catalog `Syntax:` line's ordered parameter names, and how many at the end repeat as a
/// group ([`funcs::parameters`]).
fn signature_params(signature: &str) -> (Vec<String>, usize) {
    let (params, group) = funcs::parameters(signature);
    (params.into_iter().map(|(_, name)| name).collect(), group)
}

/// One entry per catalog function (checked by a test below), a plain-English name for its
/// abbreviation.
static ALIASES: &[(&str, &str)] = &[
    ("ABS", "Absolute Value"),
    ("ACOS", "Arc Cosine"),
    ("ADDRESS", "Address"),
    ("AND", "And"),
    ("ASIN", "Arc Sine"),
    ("ATAN", "Arc Tangent"),
    ("ATAN2", "Arc Tangent (2 Argument)"),
    ("AVERAGE", "Average"),
    ("AVERAGEIF", "Average If"),
    ("AVERAGEIFS", "Average If All"),
    ("CEILING", "Round Up To Multiple"),
    ("CHOOSE", "Choose"),
    ("COLUMN", "Column"),
    ("COLUMNS", "Column Count"),
    ("COM.MICROSOFT.CEILING", "Round Up To Multiple (Excel)"),
    ("COM.MICROSOFT.FLOOR", "Round Down To Multiple (Excel)"),
    ("CONCATENATE", "Concatenate"),
    ("COS", "Cosine"),
    ("COUNT", "Count Numbers"),
    ("COUNTA", "Count All"),
    ("COUNTBLANK", "Count Blank"),
    ("COUNTIF", "Count If"),
    ("COUNTIFS", "Count If All"),
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
    ("EDATE", "Add Months"),
    ("EOMONTH", "End Of Month"),
    ("EVEN", "Round Up To Even"),
    ("EXACT", "Text Equals (Case-Sensitive)"),
    ("EXP", "Exponential"),
    ("FACT", "Factorial"),
    ("FALSE", "False"),
    ("FIND", "Find Text"),
    ("FLOOR", "Round Down To Multiple"),
    ("FV", "Future Value"),
    ("HLOOKUP", "Horizontal Lookup"),
    ("HOUR", "Hour"),
    ("HYPERLINK", "Hyperlink"),
    ("IF", "If"),
    ("IFERROR", "If Error"),
    ("IFNA", "If Not Available"),
    ("INDEX", "Index"),
    ("INT", "Integer Part"),
    ("IPMT", "Interest Payment"),
    ("IRR", "Internal Rate Of Return"),
    ("ISBLANK", "Is Blank"),
    ("ISERR", "Is Error (Not #N/A)"),
    ("ISERROR", "Is Error"),
    ("ISLOGICAL", "Is Logical"),
    ("ISNA", "Is #N/A"),
    ("ISNONTEXT", "Is Not Text"),
    ("ISNUMBER", "Is Number"),
    ("ISOWEEKNUM", "ISO Week Number"),
    ("ISTEXT", "Is Text"),
    ("LARGE", "Nth Largest"),
    ("LEFT", "Left Characters"),
    ("LEN", "Length"),
    ("LN", "Natural Logarithm"),
    ("LOG", "Logarithm"),
    ("LOG10", "Logarithm (Base 10)"),
    ("LOWER", "Lowercase"),
    ("MATCH", "Match Position"),
    ("MAX", "Maximum"),
    ("MEDIAN", "Median"),
    ("MID", "Middle Characters"),
    ("MIN", "Minimum"),
    ("MINUTE", "Minute"),
    ("MOD", "Modulo"),
    ("MONTH", "Month"),
    ("MROUND", "Round To Multiple"),
    ("N", "To Number"),
    ("NA", "Not Available"),
    ("NETWORKDAYS", "Working Days Between"),
    ("NOT", "Not"),
    ("NOW", "Now"),
    ("NPER", "Number Of Periods"),
    ("NPV", "Net Present Value"),
    ("ODD", "Round Up To Odd"),
    ("OR", "Or"),
    ("PI", "Pi"),
    ("PMT", "Payment"),
    ("POWER", "Power"),
    ("PPMT", "Principal Payment"),
    ("PRODUCT", "Product"),
    ("PROPER", "Title Case"),
    ("PV", "Present Value"),
    ("QUOTIENT", "Integer Quotient"),
    ("RADIANS", "Degrees To Radians"),
    ("RANK", "Rank"),
    ("RATE", "Interest Rate"),
    ("REPLACE", "Replace Characters"),
    ("REPT", "Repeat Text"),
    ("RIGHT", "Right Characters"),
    ("ROUND", "Round"),
    ("ROUNDDOWN", "Round Down"),
    ("ROUNDUP", "Round Up"),
    ("ROW", "Row"),
    ("ROWS", "Row Count"),
    ("SEARCH", "Search Text"),
    ("SECOND", "Second"),
    ("SIGN", "Sign"),
    ("SIN", "Sine"),
    ("SLN", "Straight-Line Depreciation"),
    ("SMALL", "Nth Smallest"),
    ("SQRT", "Square Root"),
    ("STDEV", "Standard Deviation"),
    ("STDEVP", "Standard Deviation (Population)"),
    ("SUBSTITUTE", "Substitute Text"),
    ("SUBTOTAL", "Subtotal"),
    ("SUM", "Sum"),
    ("SUMIF", "Sum If"),
    ("SUMIFS", "Sum If All"),
    ("SUMPRODUCT", "Sum Of Products"),
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
    ("WEEKNUM", "Week Number"),
    ("WORKDAY", "Working Day"),
    ("YEAR", "Year"),
];

/// `(function, raw token)` → alias, for the handful of single-letter tokens the spec reuses
/// with a different meaning per function. Checked before [`PARAM_ALIASES`].
static OVERRIDES: &[(&str, &str, &str)] = &[
    ("AVERAGEIFS", "A", "Average Range"),
    ("SUMIFS", "R", "Sum Range"),
    ("DAY", "D", "Date"),
    ("ISOWEEKNUM", "D", "Date"),
    ("WEEKNUM", "D", "Date"),
    ("WEEKDAY", "D", "Date"),
    ("YEAR", "D", "Date"),
    ("FACT", "F", "Number"),
    ("HOUR", "T", "Time"),
    ("IPMT", "Type", "Payment Type"),
    ("PPMT", "Type", "Payment Type"),
    ("MINUTE", "T", "Time"),
    ("SECOND", "T", "Time"),
];

/// One entry per raw parameter token the catalog's signatures produce (checked by a test
/// below), a plain-English name for it. Single-letter tokens get the meaning they carry in
/// the majority of the functions that use them — [`OVERRIDES`] covers the rest.
static PARAM_ALIASES: &[(&str, &str)] = &[
    ("A", "Number"),
    ("A1Style", "A1 Style"),
    ("Abs", "Absolute"),
    ("Alternative", "Alternative"),
    ("AnyValue", "Value"),
    ("AreaNumber", "Area Number"),
    ("B", "Number"),
    ("Base", "Base"),
    ("C", "Criterion"),
    ("C1", "Criterion 1"),
    ("C2", "Criterion 2"),
    ("Column", "Column"),
    ("Condition", "Condition"),
    ("Cost", "Cost"),
    ("Count", "Count"),
    ("D", "Database"),
    ("Data", "Data"),
    ("DataSource", "Data Source"),
    ("Date", "Date"),
    ("Date1", "Start Date"),
    ("Date2", "End Date"),
    ("Day", "Day"),
    ("DeclinationFactor", "Declination Factor"),
    ("Digits", "Digits"),
    ("F", "Field"),
    ("FV", "Future Value"),
    ("FunctionResult", "Shown Value"),
    ("Future", "Future Value"),
    ("Function", "Function Number"),
    ("Fv", "Future Value"),
    ("Guess", "Guess"),
    ("Holidays", "Holidays"),
    ("Hours", "Hours"),
    ("IRI", "Link Address"),
    ("IfFalse", "If False"),
    ("IfTrue", "If True"),
    ("Index", "Index"),
    ("L", "Logical Value"),
    ("Length", "Length"),
    ("LifeTime", "Life Time"),
    ("List", "List"),
    ("Lookup", "Lookup Value"),
    ("MatchType", "Match Type"),
    ("Minutes", "Minutes"),
    ("Mode", "Mode"),
    ("Month", "Month"),
    ("MonthAdd", "Months To Add"),
    ("N", "Number"),
    ("New", "New Text"),
    ("Nper", "Number Of Periods"),
    ("Offset", "Working Days"),
    ("Old", "Old Text"),
    ("Order", "Order"),
    ("PV", "Present Value"),
    ("PayType", "Payment Type"),
    ("Payment", "Payment"),
    ("Period", "Period"),
    ("Present", "Present Value"),
    ("Pv", "Present Value"),
    ("R", "Reference"),
    ("R1", "Range 1"),
    ("R2", "Range 2"),
    ("RangeLookup", "Range Lookup"),
    ("Rate", "Rate"),
    ("Row", "Row"),
    ("S", "Reference"),
    ("Salvage", "Salvage Value"),
    ("Search", "Search"),
    ("SearchRegion", "Search Region"),
    ("Seconds", "Seconds"),
    ("Sequence", "Range"),
    ("Sheet", "Sheet"),
    ("Significance", "Multiple Of"),
    ("Start", "Start"),
    ("StartDate", "Start Date"),
    ("T", "Text"),
    ("T1", "Text 1"),
    ("T2", "Text 2"),
    ("Type", "Type"),
    ("Value", "Value"),
    ("Values", "Values"),
    ("Which", "Occurrence"),
    ("Workdays", "Work Week"),
    ("X", "Value"),
    ("Year", "Year"),
    ("x", "X"),
    ("y", "Y"),
];

#[cfg(test)]
mod tests {
    use super::*;

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
    fn a_repeated_pair_of_parameters_is_numbered_by_pair() {
        // `[ ; Reference R2 ; Criterion C2 ]...` repeats two parameters together, and the
        // third pair is the third range and the third criterion.
        assert_eq!(
            explain_inline("of:=COUNTIFS([.A1:.A2];1;[.B1:.B2];2;[.C1:.C2];3)").unwrap(),
            "Count If All(Range 1: A1:A2, Criterion 1: 1, Range 2: B1:B2, Criterion 2: 2, \
             Range 3: C1:C2, Criterion 3: 3)"
        );
        assert_eq!(
            explain_inline("of:=SUMIFS([.B1:.B2];[.A1:.A2];1)").unwrap(),
            "Sum If All(Sum Range: B1:B2, Range 1: A1:A2, Criterion 1: 1)"
        );
        let (_, params) = signature("COUNTIFS").expect("COUNTIFS is in the catalog");
        assert_eq!(
            params,
            ["Range 1", "Criterion 1", "Range 2", "Criterion 2…"]
        );
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

    /// Every token [`signature_params`] can produce is a key of [`PARAM_ALIASES`];
    /// [`param_alias`]'s fallback cannot tell a missing entry from one aliased to itself.
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
