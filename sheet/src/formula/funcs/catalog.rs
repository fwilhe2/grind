// SPDX-FileCopyrightText: 2025 OASIS Open
// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: LicenseRef-OASIS-IPR AND AGPL-3.0-or-later

//! What each function is, for a shell that has to offer it: extracted from ODF 1.4 Part 4 rather
//! than written about it. `signature` is the section's `Syntax:` line and `brief` its `Summary:`
//! line, verbatim, with the section number. Verbatim extraction is why this file carries the OASIS
//! copyright alongside ours, as `doc/small-group.md` does.
//!
//! Four of the spec's `Syntax:` lines and one `Summary:` are wrong and are corrected, with the erratum noted at the
//! entry:
//!
//! * §6.13.20 `ISNA` is written `ISERR( Scalar X )`
//! * §6.20.18 `REPT` is written `T ( Text T ; Integer Count )`
//! * §6.12.45 `SLN` is written `DDB( Number Cost ; Number Salvage ; Number LifeTime )`
//! * §6.10.10 `EOMONTH` is written `EOMONTH( DateParam StartDate ; Integer MonthAdd`
//! * §6.13.23 `ISODD`'s `Summary:` says "even"
//!
//! The `COM.MICROSOFT.` entries are not the spec's: their lines are written in its style, under
//! their ODF namesake's chapter. Tests hold [`catalog`] to [`super::implemented`] and every
//! section number to `doc/small-group.md`. Arity checking stays in each function.

/// One function, as a user interface needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FuncInfo {
    /// `SUM`.
    pub name: &'static str,
    /// The spec's `Syntax:` line, `;`-separated the way a formula writes its arguments —
    /// which is what a hint UI splits on.
    pub signature: &'static str,
    /// The spec's `Summary:` line: one sentence, and the whole tooltip.
    pub brief: &'static str,
    /// Where its normative definition lives in Part 4, e.g. `6.16.61`.
    pub section: &'static str,
}

/// Every function this build implements, alphabetically.
#[must_use]
pub fn catalog() -> &'static [FuncInfo] {
    CATALOG
}

/// A `Syntax:` line's parameters in order, each as its declared type and name — `("Number",
/// "Fv")` out of `[ Number Fv = 0 ]` — and how many of them, at the end, repeat as a group:
/// 1 for `{ … } +`, the members of the last bracket for the `[ ; Reference R2 ; Criterion C2
/// ]...` the several-criteria functions are written with, and 0 when nothing repeats.
pub fn parameters(signature: &str) -> (Vec<(String, String)>, usize) {
    let Some((_, rest)) = signature.split_once('(') else {
        return (Vec::new(), 0);
    };
    let rest = rest.strip_suffix(')').unwrap_or(rest).trim();
    if rest.is_empty() {
        return (Vec::new(), 0);
    }
    let params: Vec<(String, String)> = rest.split(';').filter_map(parameter).collect();
    let group = if rest.ends_with('+') {
        1
    } else if let Some(tail) = rest.strip_suffix("...") {
        let last_bracket = tail.rfind('[').unwrap_or(0);
        tail[last_bracket..]
            .split(';')
            .filter_map(parameter)
            .count()
    } else {
        0
    };
    (params, group)
}

/// One parameter's type and name out of its declaration, e.g. `" [ Number Fv = 0 ] [ "` →
/// `("Number", "Fv")`: the first and last identifiers before an `=` default (if any), with
/// the optionality brackets, repetition braces and the `...` of a repeated group stripped.
fn parameter(part: &str) -> Option<(String, String)> {
    let cleaned: String = part.chars().filter(|c| !"[]{}+.".contains(*c)).collect();
    let before_default = cleaned.split('=').next().unwrap_or(&cleaned);
    let mut words = before_default.split_whitespace();
    let kind = words.next()?;
    Some((kind.to_owned(), words.last().unwrap_or(kind).to_owned()))
}

/// Whether the `index`th argument of a call to `name` is one value rather than a range — which
/// decides whether an array handed to a `ForceArray` parameter is iterated over (§3.3 2.2) or taken
/// whole. One value when every alternative type is: `Number`, `Text|Number`, `Any`; not
/// `NumberSequence`, `Reference|Array` or `Criteria`.
#[must_use]
pub(crate) fn takes_one_value(name: &str, index: usize) -> bool {
    const ONE: &[&str] = &[
        "Any",
        "Criterion",
        "DateParam",
        "Field",
        "Integer",
        "Logical",
        "Number",
        "Scalar",
        "Text",
        "TimeParam",
    ];
    let Some(info) = CATALOG.iter().find(|info| info.name == name) else {
        return false;
    };
    let (params, group) = parameters(info.signature);
    let index = match index.checked_sub(params.len()) {
        None => index,
        Some(_) if group == 0 => return false,
        Some(past) => params.len() - group + past % group,
    };
    params[index].0.split('|').all(|kind| ONE.contains(&kind))
}

/// The function group a help browser files `info` under, derived from the section number's second
/// component (`6.16.61` is `6.16`, Mathematical) — `doc/small-group.md`'s own headings.
#[must_use]
pub fn category(info: &FuncInfo) -> &'static str {
    match info.section.split('.').nth(1) {
        Some("9") => "Database",
        Some("10") => "Date & Time",
        Some("11") => "External Access",
        Some("12") => "Financial",
        Some("13") => "Information",
        Some("14") => "Lookup & Reference",
        Some("15") => "Logical",
        Some("16") => "Math & Trigonometry",
        Some("17") => "Rounding",
        Some("18") => "Statistical",
        Some("19") => "Number Conversion",
        Some("20") => "Text",
        _ => "Other",
    }
}

/// One entry per name in `funcs::implemented()`, checked by a test in that module.
static CATALOG: &[FuncInfo] = &[
    FuncInfo {
        name: "ABS",
        signature: "ABS( Number N )",
        brief: "Return the absolute (nonnegative) value.",
        section: "6.16.2",
    },
    FuncInfo {
        name: "ACOS",
        signature: "ACOS( Number N )",
        brief: "Returns the principal value of the arc cosine of a number. The angle is returned in radians.",
        section: "6.16.3",
    },
    FuncInfo {
        name: "ADDRESS",
        signature: "ADDRESS( Integer Row ; Integer Column [ ; Integer Abs = 1 [ ; Logical A1Style = TRUE [ ; Text Sheet ] ] ] )",
        brief: "Returns a cell address (reference) as text.",
        section: "6.14.2",
    },
    FuncInfo {
        name: "AND",
        signature: "AND( { Logical|NumberSequenceList L } + )",
        brief: "Compute logical AND of all parameters.",
        section: "6.15.2",
    },
    FuncInfo {
        name: "ASIN",
        signature: "ASIN( Number N )",
        brief: "Return the principal value of the arc sine of a number. The angle is returned in radians.",
        section: "6.16.7",
    },
    FuncInfo {
        name: "ATAN",
        signature: "ATAN( Number N )",
        brief: "Return the principal value of the arc tangent of a number. The angle is returned in radians.",
        section: "6.16.9",
    },
    FuncInfo {
        name: "ATAN2",
        signature: "ATAN2( Number x ; Number y )",
        brief: "Returns the principal value of the arc tangent given a coordinate of two numbers. The angle is returned in radians.",
        section: "6.16.10",
    },
    FuncInfo {
        name: "AVEDEV",
        signature: "AVEDEV( { NumberSequenceList N } + )",
        brief: "Calculates the average of the absolute deviations of the values in list.",
        section: "6.18.2",
    },
    FuncInfo {
        name: "AVERAGE",
        signature: "AVERAGE( { NumberSequence N } + )",
        brief: "Average the set of numbers",
        section: "6.18.3",
    },
    FuncInfo {
        name: "AVERAGEA",
        signature: "AVERAGEA( { Any N } + )",
        brief: "Average values, including values of type Text and Logical.",
        section: "6.18.4",
    },
    FuncInfo {
        name: "AVERAGEIF",
        signature: "AVERAGEIF( Reference R ; Criterion C [ ; Reference A ] )",
        brief: "Average the values of cells in a range that meet a criteria.",
        section: "6.18.5",
    },
    FuncInfo {
        name: "AVERAGEIFS",
        signature: "AVERAGEIFS( Reference A ; Reference R1 ; Criterion C1 [ ; Reference R2 ; Criterion C2 ]... )",
        brief: "Average the values of cells that meet multiple criteria in multiple ranges.",
        section: "6.18.6",
    },
    FuncInfo {
        name: "BASE",
        signature: "BASE( Integer X ; Integer Radix [ ; Integer MinimumLength ] )",
        brief: "Converts a number into a text representation with the given base.",
        section: "6.19.3",
    },
    FuncInfo {
        name: "BIN2DEC",
        signature: "BIN2DEC( TextOrNumber X )",
        brief: "Converts a binary (base 2) number (up to 10 digits) to its decimal equivalent",
        section: "6.19.4",
    },
    FuncInfo {
        name: "BIN2HEX",
        signature: "BIN2HEX( TextOrNumber X [ ; Number Digits ] )",
        brief: "Converts a binary (base 2) number (10 th bit is sign) to its hexadecimal equivalent",
        section: "6.19.5",
    },
    FuncInfo {
        name: "BIN2OCT",
        signature: "BIN2OCT( TextOrNumber X [ ; Number Digits ] )",
        brief: "Converts a binary (base 2) number (10 th bit is sign) to its octal (base 8) equivalent",
        section: "6.19.6",
    },
    FuncInfo {
        name: "CEILING",
        signature: "CEILING( Number N [ ; [ Number Significance ] [ ; Number Mode ] ] )",
        brief: "Round a number N up to the nearest multiple of the second parameter, significance.",
        section: "6.17.1",
    },
    FuncInfo {
        name: "CHAR",
        signature: "CHAR( Number N )",
        brief: "Return character represented by the given numeric value",
        section: "6.20.3",
    },
    FuncInfo {
        name: "CHOOSE",
        signature: "CHOOSE( Integer Index ; { Any Value } + )",
        brief: "Uses an index to return a value from a list of values.",
        section: "6.14.3",
    },
    FuncInfo {
        name: "CLEAN",
        signature: "CLEAN( Text T )",
        brief: "Remove all non-printable characters from the string and return the result.",
        section: "6.20.4",
    },
    FuncInfo {
        name: "CODE",
        signature: "CODE( Text T )",
        brief: "Return numeric value corresponding to the first character of the text value.",
        section: "6.20.5",
    },
    FuncInfo {
        name: "COLUMN",
        signature: "COLUMN( [ Reference R ] )",
        brief: "Returns the column number(s) of a reference.",
        section: "6.13.4",
    },
    FuncInfo {
        name: "COLUMNS",
        signature: "COLUMNS( Reference|Array R )",
        brief: "Returns the number of columns in a given range.",
        section: "6.13.5",
    },
    FuncInfo {
        name: "COM.MICROSOFT.CEILING",
        signature: "COM.MICROSOFT.CEILING( Number N ; Number Significance )",
        brief: "Excel's CEILING: rounds N up to a multiple of Significance on the number line.",
        section: "6.17",
    },
    FuncInfo {
        name: "COM.MICROSOFT.CEILING.MATH",
        signature: "COM.MICROSOFT.CEILING.MATH( Number N [ ; Number Significance = 1 [ ; Number Mode = 0 ] ] )",
        brief: "Excel's CEILING.MATH: CEILING.PRECISE, or away from zero for a negative N when Mode is not 0.",
        section: "6.17",
    },
    FuncInfo {
        name: "COM.MICROSOFT.CEILING.PRECISE",
        signature: "COM.MICROSOFT.CEILING.PRECISE( Number N [ ; Number Significance = 1 ] )",
        brief: "Excel's CEILING.PRECISE: rounds N up to a multiple of the magnitude of Significance.",
        section: "6.17",
    },
    FuncInfo {
        name: "COM.MICROSOFT.CONCAT",
        signature: "COM.MICROSOFT.CONCAT( { Any T } + )",
        brief: "Excel's CONCAT: the text of every argument, ranges cell by cell, joined.",
        section: "6.20",
    },
    FuncInfo {
        name: "COM.MICROSOFT.FLOOR",
        signature: "COM.MICROSOFT.FLOOR( Number N ; Number Significance )",
        brief: "Excel's FLOOR: rounds N down to a multiple of Significance on the number line.",
        section: "6.17",
    },
    FuncInfo {
        name: "COM.MICROSOFT.FLOOR.MATH",
        signature: "COM.MICROSOFT.FLOOR.MATH( Number N [ ; Number Significance = 1 [ ; Number Mode = 0 ] ] )",
        brief: "Excel's FLOOR.MATH: FLOOR.PRECISE, or towards zero for a negative N when Mode is not 0.",
        section: "6.17",
    },
    FuncInfo {
        name: "COM.MICROSOFT.FLOOR.PRECISE",
        signature: "COM.MICROSOFT.FLOOR.PRECISE( Number N [ ; Number Significance = 1 ] )",
        brief: "Excel's FLOOR.PRECISE: rounds N down to a multiple of the magnitude of Significance.",
        section: "6.17",
    },
    FuncInfo {
        name: "COM.MICROSOFT.FORECAST.LINEAR",
        signature: "COM.MICROSOFT.FORECAST.LINEAR( Number Value ; ForceArray Array Data_Y ; ForceArray Array Data_X )",
        brief: "Excel's FORECAST.LINEAR: FORECAST by its new name.",
        section: "6.18",
    },
    FuncInfo {
        name: "COM.MICROSOFT.GAMMA.DIST",
        signature: "COM.MICROSOFT.GAMMA.DIST( Number X ; Number Alpha ; Number Beta ; Logical Cumulative )",
        brief: "Excel's GAMMA.DIST: the gamma distribution's density at X, or its cumulative distribution when Cumulative is TRUE.",
        section: "6.18",
    },
    FuncInfo {
        name: "COM.MICROSOFT.IFS",
        signature: "COM.MICROSOFT.IFS( Logical Condition1 ; Any Value1 [ ; Logical Condition2 ; Any Value2 ]... )",
        brief: "Excel's IFS: the value beside the first condition that is true, #N/A if none is.",
        section: "6.15",
    },
    FuncInfo {
        name: "COM.MICROSOFT.ISO.CEILING",
        signature: "COM.MICROSOFT.ISO.CEILING( Number N [ ; Number Significance = 1 ] )",
        brief: "Excel's ISO.CEILING: CEILING.PRECISE by its other name.",
        section: "6.17",
    },
    FuncInfo {
        name: "COM.MICROSOFT.MAXIFS",
        signature: "COM.MICROSOFT.MAXIFS( Reference M ; Reference R1 ; Criterion C1 [ ; Reference R2 ; Criterion C2 ]... )",
        brief: "Excel's MAXIFS: the largest number in M where every range meets its criterion, 0 if none does.",
        section: "6.18",
    },
    FuncInfo {
        name: "COM.MICROSOFT.MINIFS",
        signature: "COM.MICROSOFT.MINIFS( Reference M ; Reference R1 ; Criterion C1 [ ; Reference R2 ; Criterion C2 ]... )",
        brief: "Excel's MINIFS: the smallest number in M where every range meets its criterion, 0 if none does.",
        section: "6.18",
    },
    FuncInfo {
        name: "COM.MICROSOFT.MODE.SNGL",
        signature: "COM.MICROSOFT.MODE.SNGL( { ForceArray NumberSequence N } + )",
        brief: "Excel's MODE.SNGL: MODE by its new name.",
        section: "6.18",
    },
    FuncInfo {
        name: "COM.MICROSOFT.NETWORKDAYS.INTL",
        signature: "COM.MICROSOFT.NETWORKDAYS.INTL( DateParam StartDate ; DateParam EndDate [ ; Scalar Weekend = 1 [ ; DateSequence Holidays ] ] )",
        brief: "Excel's NETWORKDAYS.INTL: the working days between two dates, with Weekend a code from 1 to 7 or 11 to 17, or seven 0s and 1s from Monday.",
        section: "6.10",
    },
    FuncInfo {
        name: "COM.MICROSOFT.NORM.DIST",
        signature: "COM.MICROSOFT.NORM.DIST( Number X ; Number Mean ; Number StandardDeviation ; Logical Cumulative )",
        brief: "Excel's NORM.DIST: NORMDIST with Cumulative required.",
        section: "6.18",
    },
    FuncInfo {
        name: "COM.MICROSOFT.NORM.INV",
        signature: "COM.MICROSOFT.NORM.INV( Number P ; Number Mean ; Number StandardDeviation )",
        brief: "Excel's NORM.INV: NORMINV by its new name.",
        section: "6.18",
    },
    FuncInfo {
        name: "COM.MICROSOFT.NORM.S.DIST",
        signature: "COM.MICROSOFT.NORM.S.DIST( Number Z ; Logical Cumulative )",
        brief: "Excel's NORM.S.DIST: the standard normal distribution's density at Z, or its cumulative distribution when Cumulative is TRUE.",
        section: "6.18",
    },
    FuncInfo {
        name: "COM.MICROSOFT.NORM.S.INV",
        signature: "COM.MICROSOFT.NORM.S.INV( Number P )",
        brief: "Excel's NORM.S.INV: LEGACY.NORMSINV by its new name.",
        section: "6.18",
    },
    FuncInfo {
        name: "COM.MICROSOFT.PERCENTILE.EXC",
        signature: "COM.MICROSOFT.PERCENTILE.EXC( NumberSequenceList Data ; Number X )",
        brief: "Excel's PERCENTILE.EXC: the X-th percentile, interpolated at rank X·(n+1), 0 < X < 1.",
        section: "6.18",
    },
    FuncInfo {
        name: "COM.MICROSOFT.PERCENTILE.INC",
        signature: "COM.MICROSOFT.PERCENTILE.INC( NumberSequenceList Data ; Number X )",
        brief: "Excel's PERCENTILE.INC: PERCENTILE by its new name.",
        section: "6.18",
    },
    FuncInfo {
        name: "COM.MICROSOFT.PERCENTRANK.EXC",
        signature: "COM.MICROSOFT.PERCENTRANK.EXC( NumberSequenceList Data ; Number X [ ; Integer Significance = 3 ] )",
        brief: "Excel's PERCENTRANK.EXC: X's rank in the data, from above 0 to below 1.",
        section: "6.18",
    },
    FuncInfo {
        name: "COM.MICROSOFT.PERCENTRANK.INC",
        signature: "COM.MICROSOFT.PERCENTRANK.INC( NumberSequenceList Data ; Number X [ ; Integer Significance = 3 ] )",
        brief: "Excel's PERCENTRANK.INC: PERCENTRANK by its new name.",
        section: "6.18",
    },
    FuncInfo {
        name: "COM.MICROSOFT.QUARTILE.EXC",
        signature: "COM.MICROSOFT.QUARTILE.EXC( NumberSequence Data ; Integer Quart )",
        brief: "Excel's QUARTILE.EXC: the quartile by PERCENTILE.EXC, Quart from 1 to 3.",
        section: "6.18",
    },
    FuncInfo {
        name: "COM.MICROSOFT.QUARTILE.INC",
        signature: "COM.MICROSOFT.QUARTILE.INC( NumberSequence Data ; Integer Quart )",
        brief: "Excel's QUARTILE.INC: QUARTILE by its new name.",
        section: "6.18",
    },
    FuncInfo {
        name: "COM.MICROSOFT.RANK.AVG",
        signature: "COM.MICROSOFT.RANK.AVG( Number Value ; NumberSequenceList Data [ ; Number Order = 0 ] )",
        brief: "Excel's RANK.AVG: the rank of Value, ties sharing the mean of the ranks they span.",
        section: "6.18",
    },
    FuncInfo {
        name: "COM.MICROSOFT.RANK.EQ",
        signature: "COM.MICROSOFT.RANK.EQ( Number Value ; NumberSequenceList Data [ ; Number Order = 0 ] )",
        brief: "Excel's RANK.EQ: RANK by its new name.",
        section: "6.18",
    },
    FuncInfo {
        name: "COM.MICROSOFT.STDEV.P",
        signature: "COM.MICROSOFT.STDEV.P( { NumberSequenceList N } + )",
        brief: "Excel's STDEV.P: STDEVP, the population standard deviation.",
        section: "6.18",
    },
    FuncInfo {
        name: "COM.MICROSOFT.STDEV.S",
        signature: "COM.MICROSOFT.STDEV.S( { NumberSequenceList N } + )",
        brief: "Excel's STDEV.S: STDEV, the sample standard deviation.",
        section: "6.18",
    },
    FuncInfo {
        name: "COM.MICROSOFT.SWITCH",
        signature: "COM.MICROSOFT.SWITCH( Any Expression ; Any Value1 ; Any Result1 [ ; Any Value2 ; Any Result2 ]... )",
        brief: "Excel's SWITCH: the result beside the first value equal to Expression; a last odd argument is the default, else #N/A.",
        section: "6.15",
    },
    FuncInfo {
        name: "COM.MICROSOFT.TEXTJOIN",
        signature: "COM.MICROSOFT.TEXTJOIN( Text Delimiter ; Logical IgnoreEmpty ; { Any T } + )",
        brief: "Excel's TEXTJOIN: the text of every argument joined by Delimiter, empty ones skipped when IgnoreEmpty is TRUE.",
        section: "6.20",
    },
    FuncInfo {
        name: "COM.MICROSOFT.VAR.P",
        signature: "COM.MICROSOFT.VAR.P( { NumberSequenceList N } + )",
        brief: "Excel's VAR.P: VARP, the population variance.",
        section: "6.18",
    },
    FuncInfo {
        name: "COM.MICROSOFT.VAR.S",
        signature: "COM.MICROSOFT.VAR.S( { NumberSequenceList N } + )",
        brief: "Excel's VAR.S: VAR, the sample variance.",
        section: "6.18",
    },
    FuncInfo {
        name: "COM.MICROSOFT.WORKDAY.INTL",
        signature: "COM.MICROSOFT.WORKDAY.INTL( DateParam StartDate ; Integer Days [ ; Scalar Weekend = 1 [ ; DateSequence Holidays ] ] )",
        brief: "Excel's WORKDAY.INTL: the date Days working days on from StartDate, with NETWORKDAYS.INTL's Weekend.",
        section: "6.10",
    },
    FuncInfo {
        name: "COM.MICROSOFT.XLOOKUP",
        signature: "COM.MICROSOFT.XLOOKUP( Any Key ; Reference|Array LookupArray ; Reference|Array ReturnArray [ ; Any IfNotFound [ ; Integer MatchMode = 0 [ ; Integer SearchMode = 1 ] ] ] )",
        brief: "Excel's XLOOKUP: the cell of ReturnArray beside Key's place in LookupArray.",
        section: "6.14",
    },
    FuncInfo {
        name: "COM.MICROSOFT.XMATCH",
        signature: "COM.MICROSOFT.XMATCH( Any Key ; Reference|Array LookupArray [ ; Integer MatchMode = 0 [ ; Integer SearchMode = 1 ] ] )",
        brief: "Excel's XMATCH: Key's position in LookupArray, starting from 1.",
        section: "6.14",
    },
    FuncInfo {
        name: "COMBIN",
        signature: "COMBIN( Integer N ; Integer R )",
        brief: "Returns the number of different R -length sets that can be selected from N items.",
        section: "6.16.16",
    },
    FuncInfo {
        name: "CONCATENATE",
        signature: "CONCATENATE( { Text T } + )",
        brief: "Concatenate the text strings",
        section: "6.20.6",
    },
    FuncInfo {
        name: "CORREL",
        signature: "CORREL( ForceArray Array N1 ; ForceArray Array N2 )",
        brief: "Calculates the correlation coefficient of values in N1 and N2 .",
        section: "6.18.17",
    },
    FuncInfo {
        name: "COS",
        signature: "COS( Number N )",
        brief: "Return the cosine of an angle specified in radians.",
        section: "6.16.19",
    },
    FuncInfo {
        name: "COUNT",
        signature: "COUNT( { NumberSequenceList N } + )",
        brief: "Count the number of Numbers provided.",
        section: "6.13.6",
    },
    FuncInfo {
        name: "COUNTA",
        signature: "COUNTA( { Any AnyValue } + )",
        brief: "Count the number of non-empty values.",
        section: "6.13.7",
    },
    FuncInfo {
        name: "COUNTBLANK",
        signature: "COUNTBLANK( ReferenceList R )",
        brief: "Count the number of blank cells.",
        section: "6.13.8",
    },
    FuncInfo {
        name: "COUNTIF",
        signature: "COUNTIF( ReferenceList R ; Criterion C )",
        brief: "Count the number of cells in a range that meet a criteria.",
        section: "6.13.9",
    },
    FuncInfo {
        name: "COUNTIFS",
        signature: "COUNTIFS( Reference R1 ; Criterion C1 [ ; Reference R2 ; Criterion C2 ]... )",
        brief: "Count the number of cells that meet multiple criteria in multiple ranges.",
        section: "6.13.10",
    },
    FuncInfo {
        name: "DATE",
        signature: "DATE( Integer Year ; Integer Month ; Integer Day )",
        brief: "Constructs a date from year, month, and day of month.",
        section: "6.10.2",
    },
    FuncInfo {
        name: "DATEDIF",
        signature: "DATEDIF( DateParam StartDate ; DateParam EndDate ; Text Format )",
        brief: "Returns the difference in years, months, or days of two date numbers.",
        section: "6.10.3",
    },
    FuncInfo {
        name: "DAVERAGE",
        signature: "DAVERAGE( Database D ; Field F ; Criteria C )",
        brief: "Finds the average of values in a given field from the records (rows) in a database that match a search criteria.",
        section: "6.9.2",
    },
    FuncInfo {
        name: "DAY",
        signature: "DAY( DateParam D )",
        brief: "Returns the day from a date.",
        section: "6.10.5",
    },
    FuncInfo {
        name: "DAYS",
        signature: "DAYS( DateParam EndDate ; DateParam StartDate )",
        brief: "Returns the number of days between two dates",
        section: "6.10.6",
    },
    FuncInfo {
        name: "DAYS360",
        signature: "DAYS360( DateParam StartDate ; DateParam EndDate [ ; Logical Method = FALSE ] )",
        brief: "Returns the number of days between two dates using the 360-day year",
        section: "6.10.7",
    },
    FuncInfo {
        name: "DB",
        signature: "DB( Number Cost ; Number Salvage ; Integer LifeTime ; Number Period [ ; Number Month = 12 ] )",
        brief: "Compute the depreciation allowance of an asset.",
        section: "6.12.13",
    },
    FuncInfo {
        name: "DCOUNT",
        signature: "DCOUNT( Database D ; [ Field F ] ; Criteria C )",
        brief: "Counts the number of records (rows) in a database that match a search criteria and contain numerical values.",
        section: "6.9.3",
    },
    FuncInfo {
        name: "DCOUNTA",
        signature: "DCOUNTA( Database D ; [ Field F ] ; Criteria C )",
        brief: "Counts the number of records (rows) in a database that match a search criteria and contain values (as COUNTA).",
        section: "6.9.4",
    },
    FuncInfo {
        name: "DDB",
        signature: "DDB( Number Cost ; Number Salvage ; Number LifeTime ; Number Period [ ; Number DeclinationFactor = 2 ] )",
        brief: "Compute the amount of depreciation at a given period of time.",
        section: "6.12.14",
    },
    FuncInfo {
        name: "DEC2BIN",
        signature: "DEC2BIN( TextOrNumber X [ ; Number Digits ] )",
        brief: "Converts a decimal number to base 2 (whose 10 th bit is sign)",
        section: "6.19.7",
    },
    FuncInfo {
        name: "DEC2HEX",
        signature: "DEC2HEX( TextOrNumber X [ ; Number Digits ] )",
        brief: "Converts a decimal number to base 16 (whose 40 th bit is sign)",
        section: "6.19.8",
    },
    FuncInfo {
        name: "DEC2OCT",
        signature: "DEC2OCT( TextOrNumber X [ ; Number Digits ] )",
        brief: "Converts a decimal number to base 8 (whose 30 th bit is sign)",
        section: "6.19.9",
    },
    FuncInfo {
        name: "DECIMAL",
        signature: "DECIMAL( Text X ; Integer Radix )",
        brief: "Converts text representing a number in a given base into a base 10 number.",
        section: "6.19.10",
    },
    FuncInfo {
        name: "DEGREES",
        signature: "DEGREES( Number N )",
        brief: "Convert radians to degrees.",
        section: "6.16.25",
    },
    FuncInfo {
        name: "DEVSQ",
        signature: "DEVSQ( { NumberSequence N } + )",
        brief: "Calculates sum of squares of deviations.",
        section: "6.18.20",
    },
    FuncInfo {
        name: "DGET",
        signature: "DGET( Database D ; Field F ; Criteria C )",
        brief: "Gets the single value in the field from the single record (row) in a database that matches a search criteria.",
        section: "6.9.5",
    },
    FuncInfo {
        name: "DMAX",
        signature: "DMAX( Database D ; Field F ; Criteria C )",
        brief: "Finds the maximum value in a given field from the records (rows) in a database that match a search criteria.",
        section: "6.9.6",
    },
    FuncInfo {
        name: "DMIN",
        signature: "DMIN( Database D ; Field F ; Criteria C )",
        brief: "Finds the minimum value in a given field from the records (rows) in a database that match a search criteria.",
        section: "6.9.7",
    },
    FuncInfo {
        name: "DPRODUCT",
        signature: "DPRODUCT( Database D ; Field F ; Criteria C )",
        brief: "Finds the product of values in a given field from the records (rows) in a database that match a search criteria.",
        section: "6.9.8",
    },
    FuncInfo {
        name: "DSTDEV",
        signature: "DSTDEV( Database D ; Field F ; Criteria C )",
        brief: "Finds the sample standard deviation in a given field from the records (rows) in a database that match a search criteria.",
        section: "6.9.9",
    },
    FuncInfo {
        name: "DSTDEVP",
        signature: "DSTDEVP( Database D ; Field F ; Criteria C )",
        brief: "Finds the population standard deviation in a given field from the records (rows) in a database that match a search criteria.",
        section: "6.9.10",
    },
    FuncInfo {
        name: "DSUM",
        signature: "DSUM( Database D ; Field F ; Criteria C )",
        brief: "Finds the sum of values in a given field from the records (rows) in a database that match a search criteria.",
        section: "6.9.11",
    },
    FuncInfo {
        name: "DVAR",
        signature: "DVAR( Database D ; Field F ; Criteria C )",
        brief: "Finds the sample variance in a given field from the records (rows) in a database that match a search criteria.",
        section: "6.9.12",
    },
    FuncInfo {
        name: "DVARP",
        signature: "DVARP( Database D ; Field F ; Criteria C )",
        brief: "Finds the population variance in a given field from the records (rows) in a database that match a search criteria.",
        section: "6.9.13",
    },
    FuncInfo {
        name: "EDATE",
        signature: "EDATE( DateParam StartDate ; Number MonthAdd )",
        brief: "Returns the serial number of a given date when MonthAdd months is added",
        section: "6.10.9",
    },
    FuncInfo {
        name: "EOMONTH",
        // Erratum: §6.10.10 never closes the parenthesis.
        signature: "EOMONTH( DateParam StartDate ; Integer MonthAdd )",
        brief: "Returns the serial number of the end of a month, given date plus MonthAdd months",
        section: "6.10.10",
    },
    FuncInfo {
        name: "ERROR.TYPE",
        signature: "ERROR.TYPE( Error E )",
        brief: "Returns Number representing the specific Error type.",
        section: "6.13.11",
    },
    FuncInfo {
        name: "EVEN",
        signature: "EVEN( Number N )",
        brief: "Rounds a number up to the nearest even integer. Rounding is away from zero.",
        section: "6.16.30",
    },
    FuncInfo {
        name: "EXACT",
        signature: "EXACT( Text T1 ; Text T2 )",
        brief: "Report if two text values are equal using a case-sensitive comparison .",
        section: "6.20.8",
    },
    FuncInfo {
        name: "EXP",
        signature: "EXP( Number X )",
        brief: "Returns e raised by the given number.",
        section: "6.16.31",
    },
    FuncInfo {
        name: "FACT",
        signature: "FACT( Integer F )",
        brief: "Return factorial (!).",
        section: "6.16.32",
    },
    FuncInfo {
        name: "FALSE",
        signature: "FALSE()",
        brief: "Returns constant FALSE.",
        section: "6.15.3",
    },
    FuncInfo {
        name: "FIND",
        signature: "FIND( Text Search ; Text T [ ; Integer Start = 1 ] )",
        brief: "Return the starting position of a given text.",
        section: "6.20.9",
    },
    FuncInfo {
        name: "FIXED",
        signature: "FIXED( Number N [ ; Integer D = 2 [ ; Logical OmitSeparators = FALSE ] ] )",
        brief: "Round the number to a specified number of decimals and format the result as a text.",
        section: "6.20.10",
    },
    FuncInfo {
        name: "FLOOR",
        signature: "FLOOR( Number N [ ; [ Number Significance ] [ ; Number Mode ] ] )",
        brief: "Round a number N down to the nearest multiple of the second parameter, significance.",
        section: "6.17.3",
    },
    FuncInfo {
        name: "FORECAST",
        signature: "FORECAST( Number Value ; ForceArray Array Data_Y ; ForceArray Array Data_X )",
        brief: "Extrapolates future values based on existing x and y values.",
        section: "6.18.28",
    },
    FuncInfo {
        name: "FORMULA",
        signature: "FORMULA( Reference X )",
        brief: "Returns formula at given reference as text.",
        section: "6.13.12",
    },
    FuncInfo {
        name: "FV",
        signature: "FV( Number Rate ; Number Nper ; Number Payment [ ; [ Number Pv = 0 ] [ ; Number PayType = 0 ] ] )",
        brief: "Compute the future value (FV) of an investment.",
        section: "6.12.20",
    },
    FuncInfo {
        name: "GEOMEAN",
        signature: "GEOMEAN( { NumberSequenceList N } + )",
        brief: "returns the geometric mean of a sequence",
        section: "6.18.34",
    },
    FuncInfo {
        name: "HARMEAN",
        signature: "HARMEAN( { NumberSequenceList N } + )",
        brief: "returns the harmonic mean of a sequence",
        section: "6.18.36",
    },
    FuncInfo {
        name: "HEX2BIN",
        signature: "HEX2BIN( TextOrNumber X [ ; Number Digits ] )",
        brief: "Converts a hexadecimal number (40 th bit is sign) to base 2 (whose 10 th bit is sign)",
        section: "6.19.11",
    },
    FuncInfo {
        name: "HEX2DEC",
        signature: "HEX2DEC( TextOrNumber X )",
        brief: "Converts a hexadecimal number (40 th bit is sign) to decimal",
        section: "6.19.12",
    },
    FuncInfo {
        name: "HEX2OCT",
        signature: "HEX2OCT( TextOrNumber X [ ; Number Digits ] )",
        brief: "Converts a hexadecimal number (40 th bit is sign) to base 8 (whose 30 th bit is sign)",
        section: "6.19.13",
    },
    FuncInfo {
        name: "HLOOKUP",
        signature: "HLOOKUP( Any Lookup ; ForceArray Reference|Array DataSource ; Integer Row [ ; Logical RangeLookup = TRUE ] )",
        brief: "Look for a matching value in the first row of the given table, and return the value of the indicated row.",
        section: "6.14.5",
    },
    FuncInfo {
        name: "HOUR",
        signature: "HOUR( TimeParam T )",
        brief: "Extracts the hour (0 through 23) from a time.",
        section: "6.10.11",
    },
    FuncInfo {
        name: "HYPERLINK",
        signature: "HYPERLINK( Text IRI [ ; Text|Number FunctionResult ] )",
        brief: "Creation of a hyperlink involving an evaluated expression.",
        section: "6.11.3",
    },
    FuncInfo {
        name: "IF",
        signature: "IF( Logical Condition [ ; [ Any IfTrue ] [ ; [ Any IfFalse ] ] ] )",
        brief: "Return one of two values, depending on a condition.",
        section: "6.15.4",
    },
    FuncInfo {
        name: "IFERROR",
        signature: "IFERROR( Any X ; Any Alternative )",
        brief: "Return X unless it is an Error, in which case return an alternative value.",
        section: "6.15.5",
    },
    FuncInfo {
        name: "IFNA",
        signature: "IFNA( Any X ; Any Alternative )",
        brief: "Return X unless it is #N/A, in which case return an alternative value.",
        section: "6.15.6",
    },
    FuncInfo {
        name: "INDEX",
        signature: "INDEX( ReferenceList | Array DataSource ; [ Integer Row ] [ ; [ Integer Column ] ] [ ; Integer AreaNumber = 1 ] )",
        brief: "Returns a value using a row and column index value (and optionally an area index).",
        section: "6.14.6",
    },
    FuncInfo {
        name: "INDIRECT",
        signature: "INDIRECT( Text Ref [ ; Logical A1 = TRUE ] )",
        brief: "Return a reference given a string representation of a reference.",
        section: "6.14.7",
    },
    FuncInfo {
        name: "INT",
        signature: "INT( Number N )",
        brief: "Rounds a number down to the nearest integer.",
        section: "6.17.2",
    },
    FuncInfo {
        name: "INTERCEPT",
        signature: "INTERCEPT( ForceArray Array Data_Y ; ForceArray Array Data_X )",
        brief: "Returns the y-intercept of the linear regression line for the given data.",
        section: "6.18.38",
    },
    FuncInfo {
        name: "IPMT",
        signature: "IPMT( Number Rate ; Number Period ; Number Nper ; Number PV [ ; Number FV = 0 [ ; Number Type = 0 ] ] )",
        brief: "Returns the amount of an annuity payment going towards interest.",
        section: "6.12.23",
    },
    FuncInfo {
        name: "IRR",
        signature: "IRR( NumberSequence Values [ ; Number Guess = 0.1 ] )",
        brief: "Compute the internal rate of return for a series of cash flows.",
        section: "6.12.24",
    },
    FuncInfo {
        name: "ISBLANK",
        signature: "ISBLANK( Scalar X )",
        brief: "Return TRUE if the referenced cell is blank, else return FALSE.",
        section: "6.13.14",
    },
    FuncInfo {
        name: "ISERR",
        signature: "ISERR( Scalar X )",
        brief: "Return TRUE if the parameter has type Error and is not #N/A, else return FALSE.",
        section: "6.13.15",
    },
    FuncInfo {
        name: "ISERROR",
        signature: "ISERROR( Scalar X )",
        brief: "Return TRUE if the parameter has type Error, else return FALSE.",
        section: "6.13.16",
    },
    FuncInfo {
        name: "ISEVEN",
        signature: "ISEVEN( Number X )",
        brief: "Return TRUE if the value is even, else return FALSE.",
        section: "6.13.17",
    },
    FuncInfo {
        name: "ISFORMULA",
        signature: "ISFORMULA( Reference X )",
        brief: "Return TRUE if the reference refers to a formula, else return FALSE.",
        section: "6.13.18",
    },
    FuncInfo {
        name: "ISLOGICAL",
        signature: "ISLOGICAL( Scalar X )",
        brief: "Return TRUE if the parameter has type Logical, else return FALSE.",
        section: "6.13.19",
    },
    FuncInfo {
        name: "ISNA",
        // Erratum: §6.13.20's Syntax line reads `ISERR( Scalar X )`. See the module docs.
        signature: "ISNA( Scalar X )",
        brief: "Return TRUE if the parameter has type Error and is #N/A, else return FALSE.",
        section: "6.13.20",
    },
    FuncInfo {
        name: "ISNONTEXT",
        signature: "ISNONTEXT( Scalar X )",
        brief: "Return TRUE if the parameter does not have type Text, else return FALSE.",
        section: "6.13.21",
    },
    FuncInfo {
        name: "ISNUMBER",
        signature: "ISNUMBER( Scalar X )",
        brief: "Return TRUE if the parameter has type Number, else return FALSE.",
        section: "6.13.22",
    },
    FuncInfo {
        name: "ISODD",
        signature: "ISODD( Number X )",
        brief: "Return TRUE if the value is odd, else return FALSE.",
        section: "6.13.23",
    },
    FuncInfo {
        name: "ISOWEEKNUM",
        signature: "ISOWEEKNUM( DateParam D )",
        brief: "Determines the ISO week number of the year for a given date.",
        section: "6.10.12",
    },
    FuncInfo {
        name: "ISREF",
        signature: "ISREF( Any X )",
        brief: "Return TRUE if the parameter is of type reference, else return FALSE.",
        section: "6.13.24",
    },
    FuncInfo {
        name: "ISTEXT",
        signature: "ISTEXT( Scalar X )",
        brief: "Return TRUE if the parameter has type Text, else return FALSE. ISTEXT( X ) is equivalent to NOT(ISNONTEXT( X )).",
        section: "6.13.25",
    },
    FuncInfo {
        name: "LARGE",
        signature: "LARGE( NumberSequenceList List ; Number|Array N )",
        brief: "Finds the nth largest value in a list.",
        section: "6.18.40",
    },
    FuncInfo {
        name: "LEFT",
        signature: "LEFT( Text T [ ; Integer Length ] )",
        brief: "Return a selected number of text characters from the left.",
        section: "6.20.12",
    },
    FuncInfo {
        name: "LEGACY.NORMSDIST",
        signature: "LEGACY.NORMSDIST( Number X )",
        brief: "returns the value of the cumulative distribution function for the standard normal distribution.",
        section: "6.18.54",
    },
    FuncInfo {
        name: "LEGACY.NORMSINV",
        signature: "LEGACY.NORMSINV( Number P )",
        brief: "returns the inverse of LEGACY.NORMSDIST(X).",
        section: "6.18.55",
    },
    FuncInfo {
        name: "LEN",
        signature: "LEN( Text T )",
        brief: "Return the length, in characters, of given text",
        section: "6.20.13",
    },
    FuncInfo {
        name: "LN",
        signature: "LN( Number X )",
        brief: "Return the natural logarithm of a number.",
        section: "6.16.39",
    },
    FuncInfo {
        name: "LOG",
        signature: "LOG( Number N [ ; Number Base = 10 ] )",
        brief: "Return the logarithm of a number in a specified base.",
        section: "6.16.40",
    },
    FuncInfo {
        name: "LOG10",
        signature: "LOG10( Number N )",
        brief: "Return the base 10 logarithm of a number.",
        section: "6.16.41",
    },
    FuncInfo {
        name: "LOOKUP",
        signature: "LOOKUP( Any Find ; ForceArray Reference|Array Searched [ ; ForceArray Reference|Array Results ] )",
        brief: "Look for criterion in an already-sorted array, and return a corresponding result.",
        section: "6.14.8",
    },
    FuncInfo {
        name: "LOWER",
        signature: "LOWER( Text T )",
        brief: "Return input string, but with all uppercase letters converted to lowercase letters.",
        section: "6.20.14",
    },
    FuncInfo {
        name: "MATCH",
        signature: "MATCH( Scalar Search ; ForceArray Reference|Array SearchRegion [ ; Integer MatchType = 1 ] )",
        brief: "Finds a Search item in a sequence, and returns its position (starting from 1).",
        section: "6.14.9",
    },
    FuncInfo {
        name: "MAX",
        signature: "MAX( { NumberSequenceList N } + )",
        brief: "Return the maximum from a set of numbers.",
        section: "6.18.45",
    },
    FuncInfo {
        name: "MAXA",
        signature: "MAXA( { Any N } + )",
        brief: "Return the maximum from a set of values, including values of type Text and Logical.",
        section: "6.18.46",
    },
    FuncInfo {
        name: "MEDIAN",
        signature: "MEDIAN( { NumberSequenceList X } + )",
        brief: "Returns the median (middle) value in the list.",
        section: "6.18.47",
    },
    FuncInfo {
        name: "MID",
        signature: "MID( Text T ; Integer Start ; Integer Length )",
        brief: "Returns extracted text, given an original text, starting position, and length.",
        section: "6.20.15",
    },
    FuncInfo {
        name: "MIN",
        signature: "MIN( { NumberSequenceList N } + )",
        brief: "Return the minimum from a set of numbers.",
        section: "6.18.48",
    },
    FuncInfo {
        name: "MINA",
        signature: "MINA( { Any N } + )",
        brief: "Return the minimum from a set of values, including values of type Text and Logical.",
        section: "6.18.49",
    },
    FuncInfo {
        name: "MINUTE",
        signature: "MINUTE( TimeParam T )",
        brief: "Extracts the minute (0 through 59) from a time.",
        section: "6.10.13",
    },
    FuncInfo {
        name: "MOD",
        signature: "MOD( Number A ; Number B )",
        brief: "Return the remainder when one number is divided by another number.",
        section: "6.16.42",
    },
    FuncInfo {
        name: "MODE",
        signature: "MODE( { ForceArray NumberSequence N } + )",
        brief: "Returns the most common value in a data set.",
        section: "6.18.50",
    },
    FuncInfo {
        name: "MONTH",
        signature: "MONTH( DateParam Date )",
        brief: "Extracts the month from a date.",
        section: "6.10.14",
    },
    FuncInfo {
        name: "MROUND",
        signature: "MROUND( Number A ; Number B )",
        brief: "Rounds the number to given multiple.",
        section: "6.17.4",
    },
    FuncInfo {
        name: "N",
        signature: "N( Any X )",
        brief: "Return the number of a value.",
        section: "6.13.26",
    },
    FuncInfo {
        name: "NA",
        signature: "NA()",
        brief: "Return the constant Error value #N/A.",
        section: "6.13.27",
    },
    FuncInfo {
        name: "NETWORKDAYS",
        signature: "NETWORKDAYS( DateParam Date1 ; DateParam Date2 [ ; [ DateSequence Holidays ] [ ; LogicalSequence Workdays ] ] )",
        brief: "Returns the whole number of work days between two dates.",
        section: "6.10.15",
    },
    FuncInfo {
        name: "NORMDIST",
        signature: "NORMDIST( Number X ; Number Mean ; Number StandardDeviation [ ; Logical Cumulative = TRUE() ] )",
        brief: "returns the value of the probability density function or the cumulative distribution function for the normal distribution with the mean and standard deviation given.",
        section: "6.18.52",
    },
    FuncInfo {
        name: "NORMINV",
        signature: "NORMINV( Number P ; Number Mean ; Number StandardDeviation )",
        brief: "returns the inverse of NORMDIST(x; Mean ; StandardDeviation ,TRUE()).",
        section: "6.18.53",
    },
    FuncInfo {
        name: "NOT",
        signature: "NOT( Logical L )",
        brief: "Compute logical NOT.",
        section: "6.15.7",
    },
    FuncInfo {
        name: "NOW",
        signature: "NOW()",
        brief: "Returns the serial number of the current date and time.",
        section: "6.10.16",
    },
    FuncInfo {
        name: "NPER",
        signature: "NPER( Number Rate ; Number Payment ; Number Pv [ ; [ Number Fv = 0] [ ; Number PayType = 0] ] )",
        brief: "Compute the number of payment periods for an investment.",
        section: "6.12.29",
    },
    FuncInfo {
        name: "NPV",
        signature: "NPV( Number Rate ; { NumberSequenceList Values } + )",
        brief: "Compute the net present value (NPV) for a series of periodic cash flows.",
        section: "6.12.30",
    },
    FuncInfo {
        name: "NUMBERVALUE",
        signature: "NUMBERVALUE( Text X [ ; Text DecimalSeparator [ ; Text GroupSeparator ] ] )",
        brief: "Convert text to number, in a locale-independent way.",
        section: "6.13.28",
    },
    FuncInfo {
        name: "OCT2BIN",
        signature: "OCT2BIN( TextOrNumber X [ ; Number Digits ] )",
        brief: "Converts an octal number (30 th bit is sign) to base 2 (whose 10 th bit is sign)",
        section: "6.19.14",
    },
    FuncInfo {
        name: "OCT2DEC",
        signature: "OCT2DEC( TextOrNumber X )",
        brief: "Converts an octal number (30 th bit is sign) to decimal",
        section: "6.19.15",
    },
    FuncInfo {
        name: "OCT2HEX",
        signature: "OCT2HEX( TextOrNumber X [ ; Number Digits ] )",
        brief: "Converts an octal number (30 th bit is sign) to hexadecimal (whose 40 th bit is sign)",
        section: "6.19.16",
    },
    FuncInfo {
        name: "ODD",
        signature: "ODD( Number N )",
        brief: "Rounds a number up to the nearest odd integer, where \"up\" means \"away from 0\".",
        section: "6.16.44",
    },
    FuncInfo {
        name: "OFFSET",
        signature: "OFFSET( Reference R ; Integer RowOffset ; Integer ColumnOffset [ ; [ Integer NewHeight ] [ ; [ Integer NewWidth ] ] ] )",
        brief: "Modifies a reference's position and dimension.",
        section: "6.14.11",
    },
    FuncInfo {
        name: "OR",
        signature: "OR( { Logical|NumberSequenceList L } + )",
        brief: "Compute logical OR of all parameters.",
        section: "6.15.8",
    },
    FuncInfo {
        name: "PEARSON",
        signature: "PEARSON( ForceArray Array IndependentValues ; ForceArray Array DependentValues )",
        brief: "PEARSON returns the Pearson correlation coefficient of two data sets",
        section: "6.18.56",
    },
    FuncInfo {
        name: "PERCENTILE",
        signature: "PERCENTILE( NumberSequenceList Data ; Number X )",
        brief: "Calculates the x-th sample percentile among the values in range.",
        section: "6.18.57",
    },
    FuncInfo {
        name: "PERCENTRANK",
        signature: "PERCENTRANK( NumberSequenceList Data ; Number X [ ; Integer Significance = 3 ] )",
        brief: "Returns the percentage rank of a value in a sample.",
        section: "6.18.58",
    },
    FuncInfo {
        name: "PERMUT",
        signature: "PERMUT( Integer N ; Integer K )",
        brief: "returns the number of permutations of k objects taken from n objects.",
        section: "6.18.59",
    },
    FuncInfo {
        name: "PI",
        signature: "PI()",
        brief: "Return the approximate value of π .",
        section: "6.16.45",
    },
    FuncInfo {
        name: "PMT",
        signature: "PMT( Number Rate ; Integer Nper ; Number Pv [ ; [ Number Fv = 0 ] [ ; Number PayType = 0 ] ] )",
        brief: "Compute the payment made each period for an investment.",
        section: "6.12.36",
    },
    FuncInfo {
        name: "POWER",
        signature: "POWER( Number A ; Number B )",
        brief: "Return the value of one number raised to the power of another number.",
        section: "6.16.46",
    },
    FuncInfo {
        name: "PPMT",
        signature: "PPMT( Number Rate ; Integer Period ; Integer Nper ; Number Present [ ; Number Future = 0 [ ; Number Type = 0 ] ] )",
        brief: "Calculate the payment for a given period on the principal for an investment at a given interest rate and constant payments.",
        section: "6.12.37",
    },
    FuncInfo {
        name: "PRODUCT",
        signature: "PRODUCT( { NumberSequenceList N } + )",
        brief: "Multiply the set of numbers, including all numbers inside ranges.",
        section: "6.16.47",
    },
    FuncInfo {
        name: "PROPER",
        signature: "PROPER( Text T )",
        brief: "Return the input string with the first letter of each word converted to an uppercase letter and the rest of the letters in the word converted to lowercase.",
        section: "6.20.16",
    },
    FuncInfo {
        name: "PV",
        signature: "PV( Number Rate ; Number Nper ; Number Payment [ ; [ Number Fv = 0 ] [ ; Number PayType = 0 ] ] )",
        brief: "Compute the present value (PV) of an investment.",
        section: "6.12.41",
    },
    FuncInfo {
        name: "QUARTILE",
        signature: "QUARTILE( NumberSequence Data ; Integer Quart )",
        brief: "Returns a quartile of a set of data points.",
        section: "6.18.64",
    },
    FuncInfo {
        name: "QUOTIENT",
        signature: "QUOTIENT( Number A ; Number B )",
        brief: "Return the integer portion of a division.",
        section: "6.16.48",
    },
    FuncInfo {
        name: "RADIANS",
        signature: "RADIANS( Number N )",
        brief: "Convert degrees to radians.",
        section: "6.16.49",
    },
    FuncInfo {
        name: "RAND",
        signature: "RAND()",
        brief: "Return a random number between 0 (inclusive) and 1 (exclusive).",
        section: "6.16.50",
    },
    FuncInfo {
        name: "RANDBETWEEN",
        signature: "RANDBETWEEN( Integer A ; Integer B )",
        brief: "Return a random integer number between A and B .",
        section: "6.16.51",
    },
    FuncInfo {
        name: "RANK",
        signature: "RANK( Number Value ; NumberSequenceList Data [ ; Number Order = 0 ] )",
        brief: "Returns the rank of a number in a list of numbers.",
        section: "6.18.65",
    },
    FuncInfo {
        name: "RATE",
        signature: "RATE( Number Nper ; Number Payment ; Number Pv [ ; [ Number Fv = 0 ] [ ; [ Number PayType = 0 ] [ ; Number Guess = 0.1 ] ] ] )",
        brief: "Compute the interest rate per period of an investment.",
        section: "6.12.42",
    },
    FuncInfo {
        name: "REPLACE",
        signature: "REPLACE( Text T ; Number Start ; Number Count ; Text New )",
        brief: "Returns text where an old text is substituted with a new text.",
        section: "6.20.17",
    },
    FuncInfo {
        name: "REPT",
        // Erratum: §6.20.18's Syntax line reads `T ( Text T ; Integer Count )`.
        signature: "REPT( Text T ; Integer Count )",
        brief: "Return text repeated Count times.",
        section: "6.20.18",
    },
    FuncInfo {
        name: "RIGHT",
        signature: "RIGHT( Text T [ ; Integer Length ] )",
        brief: "Return a selected number of text characters from the right.",
        section: "6.20.19",
    },
    FuncInfo {
        name: "ROMAN",
        signature: "ROMAN( Integer N [ ; Integer Format = 0 ] )",
        brief: "Convert to Roman numerals",
        section: "6.19.17",
    },
    FuncInfo {
        name: "ROUND",
        signature: "ROUND( Number X [ ; Number Digits = 0 ] )",
        brief: "Rounds the value X to the nearest multiple of the power of 10 specified by Digits .",
        section: "6.17.5",
    },
    FuncInfo {
        name: "ROUNDDOWN",
        signature: "ROUNDDOWN( Number X [ ; Integer Digits = 0 ] )",
        brief: "Rounds the value X towards zero to the number of digits specified by Digits.",
        section: "6.17.6",
    },
    FuncInfo {
        name: "ROUNDUP",
        signature: "ROUNDUP( Number X [ ; Integer Digits = 0 ] )",
        brief: "Rounds the value X away from zero to the number of digits specified by Digits",
        section: "6.17.7",
    },
    FuncInfo {
        name: "ROW",
        signature: "ROW( [ Reference R ] )",
        brief: "Returns the row number(s) of a reference.",
        section: "6.13.29",
    },
    FuncInfo {
        name: "ROWS",
        signature: "ROWS( Reference|Array R )",
        brief: "Returns the number of rows in a given range.",
        section: "6.13.30",
    },
    FuncInfo {
        name: "RSQ",
        signature: "RSQ( ForceArray Array ArrayY ; ForceArray Array ArrayX )",
        brief: "Returns the square of the Pearson product moment correlation coefficient.",
        section: "6.18.66",
    },
    FuncInfo {
        name: "SEARCH",
        signature: "SEARCH( Text Search ; Text T [ ; Integer Start = 1 ] )",
        brief: "Return the starting position of a given text.",
        section: "6.20.20",
    },
    FuncInfo {
        name: "SECOND",
        signature: "SECOND( TimeParam T )",
        brief: "Extracts the second (the integer 0 through 59) from a time. This function presumes that leap seconds never exist.",
        section: "6.10.17",
    },
    FuncInfo {
        name: "SHEET",
        signature: "SHEET( [ Text|Reference R ] )",
        brief: "Returns the sheet number of the reference or the string representing a sheet name.",
        section: "6.13.31",
    },
    FuncInfo {
        name: "SHEETS",
        signature: "SHEETS( [ Reference R ] )",
        brief: "Returns the number of sheets in a reference or current document.",
        section: "6.13.32",
    },
    FuncInfo {
        name: "SIGN",
        signature: "SIGN( Number N )",
        brief: "Return the sign of a number.",
        section: "6.16.54",
    },
    FuncInfo {
        name: "SIN",
        signature: "SIN( Number N )",
        brief: "Return the sine of an angle specified in radians.",
        section: "6.16.55",
    },
    FuncInfo {
        name: "SLN",
        // Erratum: §6.12.45's Syntax line reads `DDB( Number Cost ; … )`.
        signature: "SLN( Number Cost ; Number Salvage ; Number LifeTime )",
        brief: "Compute the amount of depreciation at a given period of time using the straight-line depreciation method.",
        section: "6.12.45",
    },
    FuncInfo {
        name: "SLOPE",
        signature: "SLOPE( ForceArray Array Y ; ForceArray Array X )",
        brief: "Calculates the slope of the linear regression line.",
        section: "6.18.69",
    },
    FuncInfo {
        name: "SMALL",
        signature: "SMALL( NumberSequenceList List ; Integer|Array N )",
        brief: "Finds the nth smallest value in a list.",
        section: "6.18.70",
    },
    FuncInfo {
        name: "SQRT",
        signature: "SQRT( Number N )",
        brief: "Return the square root of a number.",
        section: "6.16.58",
    },
    FuncInfo {
        name: "STANDARDIZE",
        signature: "STANDARDIZE( Number Value ; Number Mean ; Number Sigma )",
        brief: "Calculates a normalized value of a random variable.",
        section: "6.18.71",
    },
    FuncInfo {
        name: "STDEV",
        signature: "STDEV( { NumberSequenceList N } + )",
        brief: "Compute the sample standard deviation of a set of numbers.",
        section: "6.18.72",
    },
    FuncInfo {
        name: "STDEVP",
        signature: "STDEVP( { NumberSequence N } + )",
        brief: "Calculates the standard deviation using the population of a random variable, including values of type Text and Logical.",
        section: "6.18.74",
    },
    FuncInfo {
        name: "SUBSTITUTE",
        signature: "SUBSTITUTE( Text T ; Text Old ; Text New [ ; Integer Which ] )",
        brief: "Returns text where an old text is substituted with a new text.",
        section: "6.20.21",
    },
    FuncInfo {
        name: "SUBTOTAL",
        signature: "SUBTOTAL( Integer Function ; NumberSequence Sequence )",
        brief: "Evaluates a function on a range.",
        section: "6.16.60",
    },
    FuncInfo {
        name: "SUM",
        signature: "SUM( { NumberSequenceList N } + )",
        brief: "Sum (add) the set of numbers, including all numbers in ranges.",
        section: "6.16.61",
    },
    FuncInfo {
        name: "SUMIF",
        signature: "SUMIF( ReferenceList|Reference R ; Criterion C [ ; Reference S ] )",
        brief: "Sum the values of cells in a range that meet a criteria.",
        section: "6.16.62",
    },
    FuncInfo {
        name: "SUMIFS",
        signature: "SUMIFS( Reference R ; Reference R1 ; Criterion C1 [ ; Reference R2 ; Criterion C2 ]... )",
        brief: "Sum the values of cells in a range that meet multiple criteria in multiple ranges.",
        section: "6.16.63",
    },
    FuncInfo {
        name: "SUMPRODUCT",
        signature: "SUMPRODUCT( { ForceArray Array A }+ )",
        brief: "Returns the sum of the products of the matrix elements.",
        section: "6.16.64",
    },
    FuncInfo {
        name: "SUMSQ",
        signature: "SUMSQ( { NumberSequence N } + )",
        brief: "Sum (add) the set of squares of numbers, including all numbers in ranges",
        section: "6.16.65",
    },
    FuncInfo {
        name: "SYD",
        signature: "SYD( Number Cost ; Number Salvage ; Number LifeTime ; Number Period )",
        brief: "Compute the amount of depreciation at a given period of time using the Sum-of-the-Years'-Digits method.",
        section: "6.12.46",
    },
    FuncInfo {
        name: "T",
        signature: "T( Any X )",
        brief: "Return the text (if Text), else return 0-length Text value",
        section: "6.20.22",
    },
    FuncInfo {
        name: "TAN",
        signature: "TAN( Number N )",
        brief: "Return the tangent of an angle specified in radians",
        section: "6.16.69",
    },
    FuncInfo {
        name: "TEXT",
        signature: "TEXT( Scalar X ; Text FormatCode )",
        brief: "Return the value converted to a text.",
        section: "6.20.23",
    },
    FuncInfo {
        name: "TIME",
        signature: "TIME( Number Hours ; Number Minutes ; Number Seconds )",
        brief: "Constructs a time value from hours, minutes, and seconds.",
        section: "6.10.18",
    },
    FuncInfo {
        name: "TODAY",
        signature: "TODAY()",
        brief: "Returns the serial number of today.",
        section: "6.10.20",
    },
    FuncInfo {
        name: "TRIM",
        signature: "TRIM( Text T )",
        brief: "Remove leading and trailing spaces, and replace all internal multiple spaces with a single space.",
        section: "6.20.24",
    },
    FuncInfo {
        name: "TRUE",
        signature: "TRUE()",
        brief: "Returns constant TRUE",
        section: "6.15.9",
    },
    FuncInfo {
        name: "TRUNC",
        signature: "TRUNC( Number A ; Integer B )",
        brief: "Truncate a number to a specified number of digits.",
        section: "6.17.8",
    },
    FuncInfo {
        name: "UNICHAR",
        signature: "UNICHAR( Integer N )",
        brief: "Return the character represented by the given numeric value according to the [UNICODE] Standard.",
        section: "6.20.25",
    },
    FuncInfo {
        name: "UNICODE",
        signature: "UNICODE( Text T )",
        brief: "Return the [UNICODE] code point corresponding to the first character of the text value.",
        section: "6.20.26",
    },
    FuncInfo {
        name: "UPPER",
        signature: "UPPER( Text T )",
        brief: "Return input string, but with all lowercase letters converted to uppercase letters.",
        section: "6.20.27",
    },
    FuncInfo {
        name: "VALUE",
        signature: "VALUE( Text X )",
        brief: "Convert text to number.",
        section: "6.13.34",
    },
    FuncInfo {
        name: "VAR",
        signature: "VAR( { NumberSequence N } + )",
        brief: "Compute the sample variance of a set of numbers.",
        section: "6.18.82",
    },
    FuncInfo {
        name: "VARP",
        signature: "VARP( { NumberSequence N } + )",
        brief: "Compute the variance of the set for a set of numbers.",
        section: "6.18.84",
    },
    FuncInfo {
        name: "VLOOKUP",
        signature: "VLOOKUP( Any Lookup ; ForceArray Reference|Array DataSource ; Integer Column [ ; Logical RangeLookup = TRUE() ] )",
        brief: "Look for a matching value in the first column of the given table, and return the value of the indicated column.",
        section: "6.14.12",
    },
    FuncInfo {
        name: "WEEKDAY",
        signature: "WEEKDAY( DateParam D [ ; Integer Type = 1 ] )",
        brief: "Extracts the day of the week from a date; if text, uses current locale to convert to a date.",
        section: "6.10.21",
    },
    FuncInfo {
        name: "WEEKNUM",
        signature: "WEEKNUM( DateParam D [ ; Number Mode = 1 ] )",
        brief: "Determines the week number of the year for a given date.",
        section: "6.10.22",
    },
    FuncInfo {
        name: "WORKDAY",
        signature: "WORKDAY( DateParam Date ; Number Offset [ ; [ DateSequence Holidays ] [ ; LogicalSequence Workdays ] ] )",
        brief: "Returns the date serial number which is a specified number of work days before or after an input date.",
        section: "6.10.23",
    },
    FuncInfo {
        name: "XIRR",
        signature: "XIRR( NumberSequence Values ; DateSequence Dates [ ; Number Guess = 0.1 ] )",
        brief: "Compute the internal rate of return for a non-periodic series of cash flows.",
        section: "6.12.51",
    },
    FuncInfo {
        name: "XNPV",
        signature: "XNPV( Number Rate ; Reference | Array Values ; Reference | Array Dates )",
        brief: "Compute the net present value of a series of cash flows.",
        section: "6.12.52",
    },
    FuncInfo {
        name: "YEAR",
        signature: "YEAR( DateParam D )",
        brief: "Extracts the year from a date given in the current locale of the evaluator .",
        section: "6.10.24",
    },
    FuncInfo {
        name: "YEARFRAC",
        signature: "YEARFRAC( DateParam StartDate ; DateParam EndDate [ ; Basis B = 0 ] )",
        brief: "Extracts the number of years (including fractional part) between two dates",
        section: "6.10.25",
    },
];
