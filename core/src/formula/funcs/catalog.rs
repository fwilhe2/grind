// SPDX-FileCopyrightText: 2025 OASIS Open
// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: LicenseRef-OASIS-IPR AND AGPL-3.0-or-later

//! What each function *is*, for a shell that has to offer it to a user.
//!
//! Autocomplete needs a list, a signature hint needs the parameter names, and both need one
//! line saying what the function does. All three are in ODF 1.4 Part 4 already, so this is
//! **extracted from the spec, not written about it**: `signature` is the section's `Syntax:`
//! line and `brief` its `Summary:` line, verbatim, with the section number that carries the
//! normative definition. Reading the spec is the only way to be right about `MOD`'s sign or
//! `VLOOKUP`'s fourth argument, and quoting it means a shell's tooltip cannot drift from
//! what the evaluator implements.
//!
//! Verbatim extraction is why this file carries the OASIS copyright alongside ours, exactly
//! as `doc/small-group.md` does: the grant for derivative works is conditional on passing
//! the notice along (`CLAUDE.md`, REUSE).
//!
//! **Three of the spec's own `Syntax:` lines name the wrong function**, and are corrected
//! here with the erratum noted at the entry — a signature that says `ISERR` under `ISNA`
//! would be a defect in the tooltip, not fidelity to the source:
//!
//! * §6.13.20 `ISNA` is written `ISERR( Scalar X )`
//! * §6.20.18 `REPT` is written `T ( Text T ; Integer Count )`
//! * §6.12.45 `SLN` is written `DDB( Number Cost ; Number Salvage ; Number LifeTime )`
//!
//! Each was found by the test below rather than by reading, which is the argument for
//! having it. Nothing else is altered: the parameter names, the optional-argument brackets
//! and the summaries are the spec's.
//!
//! Two tests hold it in place: [`catalog`] names exactly what
//! [`super::implemented`] does, and every section number matches `doc/small-group.md`. The
//! catalog **documents** — arity checking stays where it is, inside each function.

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

/// Every function this build implements, alphabetically — the order an autocomplete list
/// wants, and the order a person reads.
pub fn catalog() -> &'static [FuncInfo] {
    CATALOG
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
        name: "AVERAGE",
        signature: "AVERAGE( { NumberSequence N } + )",
        brief: "Average the set of numbers",
        section: "6.18.3",
    },
    FuncInfo {
        name: "AVERAGEIF",
        signature: "AVERAGEIF( Reference R ; Criterion C [ ; Reference A ] )",
        brief: "Average the values of cells in a range that meet a criteria.",
        section: "6.18.5",
    },
    FuncInfo {
        name: "CHOOSE",
        signature: "CHOOSE( Integer Index ; { Any Value } + )",
        brief: "Uses an index to return a value from a list of values.",
        section: "6.14.3",
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
        name: "DATE",
        signature: "DATE( Integer Year ; Integer Month ; Integer Day )",
        brief: "Constructs a date from year, month, and day of month.",
        section: "6.10.2",
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
        name: "DEGREES",
        signature: "DEGREES( Number N )",
        brief: "Convert radians to degrees.",
        section: "6.16.25",
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
        name: "FV",
        signature: "FV( Number Rate ; Number Nper ; Number Payment [ ; [ Number Pv = 0 ] [ ; Number PayType = 0 ] ] )",
        brief: "Compute the future value (FV) of an investment.",
        section: "6.12.20",
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
        name: "IF",
        signature: "IF( Logical Condition [ ; [ Any IfTrue ] [ ; [ Any IfFalse ] ] ] )",
        brief: "Return one of two values, depending on a condition.",
        section: "6.15.4",
    },
    FuncInfo {
        name: "INDEX",
        signature: "INDEX( ReferenceList | Array DataSource ; [ Integer Row ] [ ; [ Integer Column ] ] [ ; Integer AreaNumber = 1 ] )",
        brief: "Returns a value using a row and column index value (and optionally an area index).",
        section: "6.14.6",
    },
    FuncInfo {
        name: "INT",
        signature: "INT( Number N )",
        brief: "Rounds a number down to the nearest integer.",
        section: "6.17.2",
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
        name: "ISTEXT",
        signature: "ISTEXT( Scalar X )",
        brief: "Return TRUE if the parameter has type Text, else return FALSE. ISTEXT( X ) is equivalent to NOT(ISNONTEXT( X )).",
        section: "6.13.25",
    },
    FuncInfo {
        name: "LEFT",
        signature: "LEFT( Text T [ ; Integer Length ] )",
        brief: "Return a selected number of text characters from the left.",
        section: "6.20.12",
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
        name: "MONTH",
        signature: "MONTH( DateParam Date )",
        brief: "Extracts the month from a date.",
        section: "6.10.14",
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
        name: "ODD",
        signature: "ODD( Number N )",
        brief: "Rounds a number up to the nearest odd integer, where \"up\" means \"away from 0\".",
        section: "6.16.44",
    },
    FuncInfo {
        name: "OR",
        signature: "OR( { Logical|NumberSequenceList L } + )",
        brief: "Compute logical OR of all parameters.",
        section: "6.15.8",
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
        name: "RADIANS",
        signature: "RADIANS( Number N )",
        brief: "Convert degrees to radians.",
        section: "6.16.49",
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
        name: "ROUND",
        signature: "ROUND( Number X [ ; Number Digits = 0 ] )",
        brief: "Rounds the value X to the nearest multiple of the power of 10 specified by Digits .",
        section: "6.17.5",
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
        name: "SECOND",
        signature: "SECOND( TimeParam T )",
        brief: "Extracts the second (the integer 0 through 59) from a time. This function presumes that leap seconds never exist.",
        section: "6.10.17",
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
        name: "SQRT",
        signature: "SQRT( Number N )",
        brief: "Return the square root of a number.",
        section: "6.16.58",
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
        name: "YEAR",
        signature: "YEAR( DateParam D )",
        brief: "Extracts the year from a date given in the current locale of the evaluator .",
        section: "6.10.24",
    },
];
