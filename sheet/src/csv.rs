// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! CSV and TSV — the one non-ODF format this suite reads and writes.
//!
//! `doc/not-doing.md` §2 makes it the single exception to "nothing but ODF, for writing", and
//! gives the reason: it carries no semantics to get wrong. That is true of the *format* and
//! not of the job, because a CSV file says nothing about what its fields mean. Everything
//! interesting here is in the two places where that gap has to be crossed.
//!
//! **There is one typing rule, and this module does not own it.** Importing does not decide
//! what a field *means*; it decides what a person would have **typed** to mean it, and
//! [`crate::App::import_csv`] hands that string to the rule a keystroke, a paste and a
//! projection all come through. So `TRUE` is a logical here because it is one when typed, and
//! a field cannot acquire a meaning in a CSV that it could not have in a cell. [`input`] is
//! that translation and is the whole of it.
//!
//! **Tolerance on the way in, strictness on the way out**, which is this project's rule for
//! ODF (`doc/ods-format.md` §8) applied to a format with far less of a specification. RFC 4180
//! describes a fraction of what real files contain, so [`parse`] never fails: an unterminated
//! quote, a stray quote inside a bare field, ragged rows, CRLF, a lone CR, a byte-order mark
//! and blank lines in the middle all have a defined reading below. [`write()`] emits the
//! conservative form every reader accepts.
//!
//! **TSV is not a second format.** The delimiter is a field of [`Dialect`], so tab-separated
//! is `Dialect::TAB` and a German export is `Dialect::SEMICOLON`. Anything else with a single
//! separator character is [`Dialect::new`].
//!
//! Dates are the one place where a field cannot become what it is without help, because the
//! typing rule reads an ISO date only into a cell already formatted as one — deliberately, so
//! that nothing acquires a date by looking like one. [`Import::dates`] supplies exactly that
//! precondition and nothing else: [`dated`] says which fields are ISO dates, the import
//! formats those cells, and the ordinary rule does the reading. **ISO only**, which is the
//! second half of the same refusal — see [`dated`].
//!
//! What this deliberately does not do, each because it would be a second rule rather than a
//! wider one:
//!
//! * **Per-column types.** LibreOffice's import dialog has a type per column; this has one
//!   rule for every field, plus [`Import::text`] for the whole file and [`Import::dates`] for
//!   the one type that cannot be reached any other way. A column that needs something else is
//!   a `grind sheet format` away *after* the import, which is where a display decision belongs.
//! * **A quoted field meaning text.** Some readers take `"007"` as text on the strength of the
//!   quotes; far more tools quote every field they write, and then a whole file of numbers
//!   arrives as strings. The leading-zero rule below covers what quoting was being used for
//!   here, without depending on how the file was written.
//! * **Encodings other than UTF-8.** [`parse`] takes `&str`, so deciding what the bytes are is
//!   the caller's, and every caller here refuses to guess: a file that is not UTF-8 is an
//!   error naming `iconv` rather than a document full of mojibake. A BOM is stripped because
//!   it is UTF-8 that Excel marked, not a different encoding.

use crate::locale::{self, Locale};

/// How a file spells its rows: which character separates fields, and which quotes them.
///
/// Two characters, because that is all a dialect is once the record separator is "whatever
/// the file uses" ([`parse`] takes all three spellings) and the escape is RFC 4180's doubled
/// quote. Anything beyond this — a backslash escape, a comment character, a fixed record
/// length — is a different format wearing a `.csv` extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dialect {
    pub delimiter: char,
    pub quote: char,
}

impl Dialect {
    /// What `.csv` means in most of the Anglophone world, and RFC 4180's own dialect.
    pub const COMMA: Dialect = Dialect {
        delimiter: ',',
        quote: '"',
    };
    /// What `.csv` means where the decimal separator is a comma — the field separator has to
    /// move out of the way, and Excel and LibreOffice both move it here.
    pub const SEMICOLON: Dialect = Dialect {
        delimiter: ';',
        quote: '"',
    };
    /// TSV — and what every clipboard between two spreadsheets carries.
    pub const TAB: Dialect = Dialect {
        delimiter: '\t',
        quote: '"',
    };

    pub fn new(delimiter: char) -> Self {
        Dialect {
            delimiter,
            quote: '"',
        }
    }

    /// Guess the delimiter from the file itself.
    ///
    /// The signal that actually separates the candidates is **consistency**: a real delimiter
    /// cuts every record into the same number of fields, and a character that merely occurs in
    /// the text does not. So each candidate is tried against a sample of the file — parsed
    /// properly, quotes and all, because a comma inside a quoted field must not count — and
    /// the one that yields a rectangle wins, more columns breaking a tie.
    ///
    /// Genuinely ambiguous input (`a;b,c` is two fields either way) resolves to the earliest
    /// candidate, which is the comma. A caller who knows better passes `--delimiter`, and that
    /// is what it is for.
    pub fn sniff(text: &str) -> Dialect {
        const CANDIDATES: [char; 4] = [',', ';', '\t', '|'];
        // Enough records to see a shape, few enough to stay cheap on a large file. Cut at a
        // line boundary so the sample cannot end inside a quoted field and unbalance it.
        const SAMPLE: usize = 8192;
        let sample = match text.char_indices().nth(SAMPLE) {
            Some((at, _)) => &text[..text[..at].rfind('\n').map_or(at, |nl| nl + 1)],
            None => text,
        };

        let mut best = (0_u32, Dialect::COMMA);
        for delimiter in CANDIDATES {
            let dialect = Dialect::new(delimiter);
            let rows = parse(sample, &dialect);
            let Some(first) = rows.first().map(Vec::len) else {
                continue;
            };
            let consistent = rows.iter().all(|row| row.len() == first);
            let score = match consistent && first > 1 {
                true => 1000 + first.min(999) as u32,
                false => 0,
            };
            if score > best.0 {
                best = (score, dialect);
            }
        }
        best.1
    }
}

/// Read a delimited file into rows of fields. Never fails; see the module's tolerance rule.
///
/// The readings that are decisions rather than obligations:
///
/// * A quote **opens a quoted field only where a field starts**, so `12"` and `a"b` keep the
///   quote as a character. This is what makes a bare field holding an inch mark survive.
/// * Text after a closing quote is appended (`"a"b` is `ab`) and an unterminated quote runs to
///   the end of the input. Both are malformed; refusing them would fail a whole import over
///   one field, which is the opposite of what a tolerant reader is for.
/// * `\r\n`, `\n` and a lone `\r` all end a record, so a file written on any of the three
///   platforms reads.
/// * A blank line is a record of one empty field rather than nothing, because dropping it
///   would shift every row after it up one and silently misalign the import.
/// * A final record separator ends the file rather than starting an empty record — `a\n` is
///   one row, not two.
pub fn parse(text: &str, dialect: &Dialect) -> Vec<Vec<String>> {
    // Excel marks its UTF-8 exports with one. It is an encoding signature and never content,
    // and left in place it would make the first header cell mysteriously not match its name.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);

    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    // Whether anything at all has been read since the last record ended. This is what tells a
    // trailing newline from a trailing empty row.
    let mut started = false;
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        started = true;
        if c == dialect.quote && field.is_empty() {
            loop {
                match chars.next() {
                    Some(q) if q == dialect.quote => match chars.peek() {
                        // A doubled quote is one quote — RFC 4180's only escape.
                        Some(&next) if next == dialect.quote => {
                            field.push(dialect.quote);
                            chars.next();
                        }
                        _ => break,
                    },
                    Some(other) => field.push(other),
                    None => break,
                }
            }
            continue;
        }
        if c == dialect.delimiter {
            row.push(std::mem::take(&mut field));
            continue;
        }
        if c == '\n' || c == '\r' {
            if c == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            row.push(std::mem::take(&mut field));
            rows.push(std::mem::take(&mut row));
            started = false;
            continue;
        }
        field.push(c);
    }
    if started {
        row.push(field);
        rows.push(row);
    }
    rows
}

/// How a document leaves as delimited text.
///
/// [`Export::formulas`] is the odd one out and says so here rather than in a second struct:
/// the other fields decide how a field is *spelled*, and that one decides which text the
/// document hands over in the first place ([`crate::App::export_csv`] reads it, [`write()`] does
/// not).
#[derive(Clone, Debug)]
pub struct Export {
    pub dialect: Dialect,
    /// Quote every field, not only the ones that would otherwise be misread. Some readers
    /// treat a quoted field as text, so this is how a column of part numbers survives one.
    pub quote_all: bool,
    /// End records with `\r\n`. RFC 4180 asks for it and Windows tools expect it; a Unix
    /// pipeline does not, which is why it is not the default.
    pub crlf: bool,
    /// Begin with a UTF-8 byte-order mark. Excel needs it to read a UTF-8 file as UTF-8;
    /// almost nothing else wants it.
    pub bom: bool,
    /// Write the formula where a cell has one, instead of the value it computed.
    pub formulas: bool,
}

impl Default for Export {
    fn default() -> Self {
        Export {
            dialect: Dialect::COMMA,
            quote_all: false,
            crlf: false,
            bom: false,
            formulas: false,
        }
    }
}

/// Write rows as delimited text — the strict half.
///
/// A field is quoted when leaving it bare would change what a reader sees: it holds the
/// delimiter, a quote, a line break, or leading or trailing whitespace that a trimming reader
/// would eat. Everything else goes out as it is, so the file stays diffable and a human can
/// read it, which is the same argument `doc/flat-first.md` makes about documents.
pub fn write(rows: &[Vec<String>], options: &Export) -> String {
    let newline = match options.crlf {
        true => "\r\n",
        false => "\n",
    };
    let mut out = String::new();
    if options.bom {
        out.push('\u{feff}');
    }
    for row in rows {
        for (at, field) in row.iter().enumerate() {
            if at > 0 {
                out.push(options.dialect.delimiter);
            }
            match options.quote_all || needs_quoting(field, &options.dialect) {
                true => {
                    out.push(options.dialect.quote);
                    for c in field.chars() {
                        if c == options.dialect.quote {
                            out.push(c);
                        }
                        out.push(c);
                    }
                    out.push(options.dialect.quote);
                }
                false => out.push_str(field),
            }
        }
        out.push_str(newline);
    }
    out
}

fn needs_quoting(field: &str, dialect: &Dialect) -> bool {
    field.contains([dialect.delimiter, dialect.quote, '\n', '\r'])
        || field.starts_with(char::is_whitespace)
        || field.ends_with(char::is_whitespace)
}

/// How a delimited file arrives as cells.
#[derive(Clone, Debug)]
pub struct Import {
    pub dialect: Dialect,
    /// Which characters spell a number. `1.234,50` is a number in `de-DE` and three fields'
    /// worth of punctuation without it — and it is the reason a semicolon file exists at all.
    pub locale: Option<Locale>,
    /// Store every field as text, guessing nothing. The escape hatch for a file whose columns
    /// are identifiers rather than quantities.
    pub text: bool,
    /// Let a field starting with `=` become a formula.
    ///
    /// **Off by default**, which is LibreOffice's default too. A CSV has no formulas, so a
    /// leading `=` is text somebody exported — and treating it as a formula is how a
    /// spreadsheet ends up evaluating a string that arrived from outside.
    pub formulas: bool,
    /// Read an **ISO** date, datetime or clock time as one, and format the cell it lands in
    /// so it stays one. See [`dated`] for what counts and what deliberately does not.
    pub dates: bool,
    /// Drop leading and trailing whitespace from every field. RFC 4180 says the space in
    /// `a, b` is part of the field, and hand-written files mean it as padding.
    pub trim: bool,
}

impl Default for Import {
    fn default() -> Self {
        Import {
            dialect: Dialect::COMMA,
            locale: None,
            text: false,
            formulas: false,
            dates: false,
            trim: false,
        }
    }
}

/// The format a field needs to be read as a date, a datetime or a time — or `None`, which is
/// every other field.
///
/// **ISO 8601 only, with a four-digit year.** `15/03/2026` and `03/15/2026` are the same eight
/// characters meaning two different days, and nothing in the file says which; a spreadsheet
/// that resolves that by guessing is the reason "date column" and "corrupted" go together in
/// so many bug reports. This one does not resolve it, so those fields stay text and stay
/// exactly what the file said.
///
/// The parsers are [`crate::formula::date`]'s — the evaluator's own, and the ones the typing
/// rule's date branch uses — so this decides *which cells are dates* and never what a date
/// means. The four-digit year is the one extra condition: `1-2-3` is a valid ISO date and is
/// far more often a part number.
pub fn dated(field: &str, null_date: i64) -> Option<crate::numfmt::Format> {
    use crate::numfmt::{Kind, datetime_preset, preset};

    let field = field.trim();
    let year = field
        .split_once('-')
        .is_some_and(|(year, _)| year.len() == 4 && year.chars().all(|c| c.is_ascii_digit()));
    if year && crate::formula::date::parse_date(field, null_date).is_some() {
        return Some(match field.contains(['T', ' ']) {
            true => datetime_preset(),
            false => preset(Kind::Date, 0, false, ""),
        });
    }
    // A clock, and only a clock: `parse_time` also reads the `PT17H20M00S` duration ODF writes
    // into an attribute, which is not something anybody exports into a column.
    if field.contains(':') && crate::formula::date::parse_time(field).is_some() {
        return Some(preset(Kind::Time, 0, false, ""));
    }
    None
}

/// One field, as the string a person would have **typed** to put it in a cell.
///
/// The whole of this module's contribution to meaning, and it is deliberately a translation
/// into an existing vocabulary rather than a second one: what comes back goes to
/// [`crate::App::enter`]'s rule, where a leading `'` forces text, a leading `=` is a formula
/// and everything else is a number, a logical or text, in that order.
///
/// Four things get forced to text that the typing rule alone would read as numbers, and each
/// is a way a real file loses data:
///
/// * **`007`** — a postcode, an order number, a product code. A leading zero that survives to
///   the file was put there on purpose, and a number cannot carry one.
/// * **`NaN`, `inf`, `Infinity`** — Rust reads all three as floating-point values, and a
///   column of names containing *Nan* is far more common in a CSV than a column of
///   quantities containing a quiet NaN.
/// * **`1,5`** where the locale says otherwise, and **`1.234,50`** where it says exactly this
///   — number recognition runs on the locale's own separators rather than on Rust's, so a
///   German file is read as German rather than as broken English.
/// * **A field beginning with `'`** — which the typing rule would strip. It comes back
///   doubled, so the apostrophe stays in the cell.
pub fn input(field: &str, options: &Import) -> String {
    let field = match options.trim {
        true => field.trim(),
        false => field,
    };
    // An empty field clears its cell. Not `'`-forced text, which would leave an empty string
    // sitting in a cell that reads as blank and sorts and counts as neither.
    if field.is_empty() {
        return String::new();
    }
    let as_text = || format!("'{field}");
    if options.text || field.starts_with('\'') {
        return as_text();
    }
    if field.starts_with('=') {
        // A CSV carries **display** form — it came out of somebody's formula bar, and that is
        // what this exports too — while the typing rule takes the canonical spelling the file
        // format holds. One conversion, by the existing parser and printer rather than by
        // rewriting text that looks right.
        return match options
            .formulas
            .then(|| crate::formula::display::from_display(field))
        {
            Some(Ok(canonical)) => canonical,
            // Asked for, and not a formula this build can read. Keeping the characters as text
            // loses nothing and says so on the screen; storing it would make a cell whose only
            // possible value is an error.
            _ => as_text(),
        };
    }
    let (decimal, group) = locale::separators(options.locale.as_ref());
    if let Some(number) = number(field, decimal, group) {
        return number;
    }
    // Anything Rust would read as a number and this would not is text, and the gap between the
    // two is the point rather than an oversight — see this function's own list.
    match field.parse::<f64>().is_ok() {
        true => as_text(),
        false => field.to_owned(),
    }
}

/// A field as a number, in the canonical spelling the typing rule reads, or `None` if it is
/// not one.
///
/// Stricter than [`f64::from_str`] in the ways that matter for a file somebody exported, and
/// looser in the one way that matters for a file somebody exported *in another language*.
/// The grouping check is the fussy part and has to be: `1,234` is a number and `1,2,3` is
/// three fields that ended up in one, so the positions are checked rather than the separator
/// merely being deleted.
fn number(field: &str, decimal: char, group: char) -> Option<String> {
    // A no-break space groups in French and Russian, and a file written by hand — or by a tool
    // that normalised its whitespace — has a plain one. Both mean the same digit group.
    let owned;
    let field = match group == '\u{a0}' && field.contains([' ', '\u{202f}']) {
        true => {
            owned = field.replace([' ', '\u{202f}'], "\u{a0}");
            owned.as_str()
        }
        false => field,
    };

    let (sign, rest) = match field.starts_with(['+', '-']) {
        true => field.split_at(1),
        false => ("", field),
    };
    // Split the exponent off first, so the separators are looked for in the mantissa only:
    // `1e5` is a number and `1e5,000` is not.
    let (mantissa, exponent) = match rest.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, Some(exponent)),
        None => (rest, None),
    };

    if mantissa.matches(decimal).count() > 1 {
        return None;
    }
    let (whole, fraction) = match mantissa.split_once(decimal) {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (mantissa, None),
    };

    let digits = |s: &str| s.chars().all(|c| c.is_ascii_digit());
    let whole = match whole.contains(group) {
        true => {
            let groups: Vec<&str> = whole.split(group).collect();
            let grouped = (1..=3).contains(&groups[0].len())
                && groups[1..].iter().all(|g| g.len() == 3)
                && groups.iter().all(|g| digits(g));
            if !grouped {
                return None;
            }
            groups.concat()
        }
        false => whole.to_owned(),
    };

    if !digits(&whole) || !fraction.is_none_or(digits) {
        return None;
    }
    // `.` on its own, or a sign with nothing after it.
    if whole.is_empty() && fraction.is_none_or(str::is_empty) {
        return None;
    }
    // The leading-zero rule, and the reason this function exists at all: `007` is text, `0`
    // and `0.5` are numbers.
    if whole.len() > 1 && whole.starts_with('0') {
        return None;
    }
    if let Some(exponent) = exponent {
        let digits_of = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
        if digits_of.is_empty() || !digits(digits_of) {
            return None;
        }
    }

    let mut out = String::from(sign);
    out.push_str(match whole.is_empty() {
        true => "0",
        false => &whole,
    });
    if let Some(fraction) = fraction.filter(|f| !f.is_empty()) {
        out.push('.');
        out.push_str(fraction);
    }
    if let Some(exponent) = exponent {
        out.push('e');
        out.push_str(exponent);
    }
    // It exists to be read back by the typing rule, so it has to parse — a subnormal or an
    // overflow spelled out in digits would not, and that is a text field.
    out.parse::<f64>().ok().filter(|n| n.is_finite())?;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(text: &str, dialect: &Dialect) -> Vec<Vec<String>> {
        parse(text, dialect)
    }

    #[test]
    fn the_plain_shape_reads() {
        assert_eq!(
            rows("a,b,c\n1,2,3\n", &Dialect::COMMA),
            vec![
                vec!["a".to_owned(), "b".to_owned(), "c".to_owned()],
                vec!["1".to_owned(), "2".to_owned(), "3".to_owned()],
            ]
        );
    }

    #[test]
    fn a_quoted_field_carries_the_delimiter_a_quote_and_a_newline() {
        let text = "name,note\n\"Smith, J\",\"said \"\"hi\"\"\"\n\"two\nlines\",x\n";
        assert_eq!(
            rows(text, &Dialect::COMMA),
            vec![
                vec!["name".to_owned(), "note".to_owned()],
                vec!["Smith, J".to_owned(), "said \"hi\"".to_owned()],
                vec!["two\nlines".to_owned(), "x".to_owned()],
            ]
        );
    }

    #[test]
    fn the_malformed_shapes_have_a_reading_rather_than_an_error() {
        // A quote that does not start a field is a character.
        assert_eq!(rows("12\",a", &Dialect::COMMA)[0][0], "12\"");
        assert_eq!(rows("a\"b,c", &Dialect::COMMA)[0][0], "a\"b");
        // Text after the closing quote joins the field.
        assert_eq!(rows("\"a\"b,c", &Dialect::COMMA)[0][0], "ab");
        // An unterminated quote runs to the end.
        assert_eq!(rows("\"a,b", &Dialect::COMMA)[0][0], "a,b");
        // Ragged rows stay ragged; the caller lands each row where it falls.
        let ragged = rows("a,b,c\nd\ne,f\n", &Dialect::COMMA);
        assert_eq!(ragged.iter().map(Vec::len).collect::<Vec<_>>(), [3, 1, 2]);
    }

    #[test]
    fn every_line_ending_ends_a_record_and_a_trailing_one_does_not_add_a_row() {
        for text in ["a\nb\n", "a\r\nb\r\n", "a\rb\r"] {
            assert_eq!(rows(text, &Dialect::COMMA).len(), 2, "{text:?}");
        }
        assert_eq!(rows("a\nb", &Dialect::COMMA).len(), 2);
        assert_eq!(rows("", &Dialect::COMMA).len(), 0);
        // A blank line is an empty row, because dropping it would move every row after it.
        assert_eq!(
            rows("a\n\nb\n", &Dialect::COMMA),
            vec![
                vec!["a".to_owned()],
                vec![String::new()],
                vec!["b".to_owned()],
            ]
        );
    }

    #[test]
    fn a_byte_order_mark_is_not_content() {
        assert_eq!(rows("\u{feff}id,name\n", &Dialect::COMMA)[0][0], "id");
    }

    #[test]
    fn the_delimiter_is_the_only_difference_between_csv_and_tsv() {
        let expected = vec![vec!["a".to_owned(), "b".to_owned()]];
        assert_eq!(rows("a\tb\n", &Dialect::TAB), expected);
        assert_eq!(rows("a;b\n", &Dialect::SEMICOLON), expected);
        assert_eq!(rows("a|b\n", &Dialect::new('|')), expected);
    }

    #[test]
    fn sniffing_prefers_the_delimiter_that_makes_a_rectangle() {
        assert_eq!(Dialect::sniff("a,b\n1,2\n"), Dialect::COMMA);
        assert_eq!(Dialect::sniff("a;b\n1;2\n"), Dialect::SEMICOLON);
        assert_eq!(Dialect::sniff("a\tb\n1\t2\n"), Dialect::TAB);
        // A comma inside a quoted field is not a delimiter, so the semicolon still wins.
        assert_eq!(
            Dialect::sniff("name;note\n\"Smith, J\";ok\n"),
            Dialect::SEMICOLON
        );
        // Prose with commas in it and one column: nothing separates, so the default stands.
        assert_eq!(Dialect::sniff("hello\nworld\n"), Dialect::COMMA);
        assert_eq!(Dialect::sniff(""), Dialect::COMMA);
    }

    #[test]
    fn writing_quotes_exactly_what_would_be_misread() {
        let out = write(
            &[vec![
                "plain".to_owned(),
                "a,b".to_owned(),
                "say \"hi\"".to_owned(),
                "two\nlines".to_owned(),
                " padded ".to_owned(),
            ]],
            &Export::default(),
        );
        assert_eq!(
            out,
            "plain,\"a,b\",\"say \"\"hi\"\"\",\"two\nlines\",\" padded \"\n"
        );
    }

    #[test]
    fn the_write_options_do_what_they_say() {
        let row = [vec!["a".to_owned(), "b".to_owned()]];
        let quoted = Export {
            quote_all: true,
            ..Export::default()
        };
        assert_eq!(write(&row, &quoted), "\"a\",\"b\"\n");
        let crlf = Export {
            crlf: true,
            ..Export::default()
        };
        assert_eq!(write(&row, &crlf), "a,b\r\n");
        let bom = Export {
            bom: true,
            ..Export::default()
        };
        assert_eq!(write(&row, &bom), "\u{feff}a,b\n");
        let semi = Export {
            dialect: Dialect::SEMICOLON,
            ..Export::default()
        };
        assert_eq!(write(&row, &semi), "a;b\n");
    }

    #[test]
    fn what_is_written_reads_back_as_itself() {
        let rows = vec![vec![
            "plain".to_owned(),
            "a,b".to_owned(),
            "say \"hi\"".to_owned(),
            "two\nlines".to_owned(),
            " padded ".to_owned(),
            String::new(),
        ]];
        for options in [
            Export::default(),
            Export {
                quote_all: true,
                ..Export::default()
            },
            Export {
                crlf: true,
                ..Export::default()
            },
            Export {
                dialect: Dialect::SEMICOLON,
                ..Export::default()
            },
        ] {
            let text = write(&rows, &options);
            assert_eq!(parse(&text, &options.dialect), rows, "{options:?}");
        }
    }

    fn typed(field: &str) -> String {
        input(field, &Import::default())
    }

    #[test]
    fn a_field_becomes_what_a_person_would_have_typed() {
        assert_eq!(typed("42"), "42");
        assert_eq!(typed("-3.5"), "-3.5");
        assert_eq!(typed("+3.5"), "+3.5");
        assert_eq!(typed(".5"), "0.5");
        assert_eq!(typed("1e5"), "1e5");
        assert_eq!(typed("1E-5"), "1e-5");
        assert_eq!(typed("TRUE"), "TRUE");
        assert_eq!(typed("hello"), "hello");
        assert_eq!(typed(""), "");
    }

    #[test]
    fn the_four_shapes_a_number_would_eat_come_back_as_text() {
        // A postcode, an order number, a product code.
        assert_eq!(typed("007"), "'007");
        assert_eq!(typed("-007"), "'-007");
        assert_eq!(typed("00"), "'00");
        // A person's name, an abbreviation. Rust reads every one of these as a float.
        for word in ["NaN", "nan", "inf", "-inf", "Infinity", "infinity"] {
            assert_eq!(typed(word), format!("'{word}"), "{word}");
        }
        // The apostrophe the typing rule would otherwise strip.
        assert_eq!(typed("'quoted"), "''quoted");
        // A formula from outside is text unless it is asked for.
        assert_eq!(typed("=SUM(A1:B1)"), "'=SUM(A1:B1)");
        let formulas = Import {
            formulas: true,
            ..Import::default()
        };
        // And when it is asked for it arrives canonical, because a file carries display form.
        assert_eq!(input("=SUM(A1:B1)", &formulas), "=SUM([.A1:.B1])");
        assert_eq!(input("=1+1", &formulas), "=1+1");
        // A formula this build cannot read keeps its characters rather than becoming a cell
        // that can only hold an error.
        assert_eq!(input("=NOSUCH(", &formulas), "'=NOSUCH(");
    }

    #[test]
    fn a_date_is_iso_and_never_a_guess() {
        let iso = |field: &str| dated(field, 0).map(|format| format.kind);
        assert_eq!(iso("2026-03-15"), Some(crate::numfmt::Kind::Date));
        assert_eq!(iso("2026-03-15T10:30:00"), Some(crate::numfmt::Kind::Date));
        assert_eq!(iso("10:30"), Some(crate::numfmt::Kind::Time));
        assert_eq!(iso("10:30:15"), Some(crate::numfmt::Kind::Time));
        // The ambiguous spellings, which mean two different days and say which nowhere.
        assert_eq!(iso("15/03/2026"), None);
        assert_eq!(iso("03/15/2026"), None);
        assert_eq!(iso("15.03.2026"), None);
        // A part number is a valid ISO date without the four-digit-year rule.
        assert_eq!(iso("1-2-3"), None);
        // Not dates at all.
        assert_eq!(iso("2026-13-01"), None);
        assert_eq!(iso("March"), None);
        assert_eq!(iso("42"), None);
        // A duration is an attribute's spelling, not a column's.
        assert_eq!(iso("PT17H20M00S"), None);
        // The datetime form keeps the clock, which the plain date format would round away.
        assert!(dated("2026-03-15T10:30:00", 0).unwrap().parts.len() > 5);
    }

    #[test]
    fn a_number_is_read_in_the_locale_that_wrote_it() {
        let de = Import {
            dialect: Dialect::SEMICOLON,
            locale: Locale::parse("de-DE"),
            ..Import::default()
        };
        assert_eq!(input("1.234,50", &de), "1234.50");
        assert_eq!(input("0,5", &de), "0.5");
        assert_eq!(input("-1.000", &de), "-1000");
        // In German that stop is a grouping separator in the wrong place, so it is not a
        // number at all. It comes back as *text*, which is the whole argument for reading a
        // file in one convention: the English number 1.5 silently becoming the German 15 is
        // the corruption this refuses, and a visibly text cell is how a person finds out that
        // `--locale` was the wrong call.
        assert_eq!(input("1.5", &de), "'1.5");
        assert_eq!(input("12.34", &de), "'12.34");
        // English keeps its own answer, and its own grouping.
        assert_eq!(typed("1,234.50"), "1234.50");
        assert_eq!(typed("1234.50"), "1234.50");
        // French groups with a space; a file written with a plain one still reads.
        let fr = Import {
            locale: Locale::parse("fr-FR"),
            ..Import::default()
        };
        assert_eq!(input("1\u{a0}234,5", &fr), "1234.5");
        assert_eq!(input("1 234,5", &fr), "1234.5");
    }

    #[test]
    fn grouping_is_checked_by_position_rather_than_deleted() {
        // Three fields somebody put in one, not one hundred and twenty-three.
        assert_eq!(typed("1,2,3"), "1,2,3");
        assert_eq!(typed("12,34"), "12,34");
        assert_eq!(typed("1,2345"), "1,2345");
        // And the shapes that really are grouped.
        assert_eq!(typed("1,234"), "1234");
        assert_eq!(typed("12,345,678"), "12345678");
        assert_eq!(typed("123,456.75"), "123456.75");
    }

    #[test]
    fn the_flags_cover_the_rest() {
        let text_only = Import {
            text: true,
            ..Import::default()
        };
        assert_eq!(input("42", &text_only), "'42");
        assert_eq!(input("TRUE", &text_only), "'TRUE");
        // Even here an empty field clears rather than storing an empty string.
        assert_eq!(input("", &text_only), "");

        let trimmed = Import {
            trim: true,
            ..Import::default()
        };
        assert_eq!(input(" 42 ", &trimmed), "42");
        assert_eq!(input("  ", &trimmed), "");
        // Without it the space is content, which is what RFC 4180 says it is.
        assert_eq!(typed(" 42"), " 42");
    }
}
