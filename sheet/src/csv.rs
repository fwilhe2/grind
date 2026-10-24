// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! CSV and TSV — the one non-ODF format this suite reads and writes (`doc/not-doing.md` §2).
//!
//! This module does not own the typing rule. Importing decides what a person would have typed to
//! mean each field ([`input`]), and [`crate::App::import_csv`] hands that to the rule a keystroke
//! and a paste use, so a field means nothing in a CSV it could not mean in a cell.
//!
//! [`parse`] never fails — an unterminated quote, a stray quote, ragged rows, CRLF, a lone CR, a
//! byte-order mark and blank lines all have a defined reading — and [`write()`] emits the
//! conservative form. TSV is `Dialect::TAB`, not a second format.
//!
//! Dates need help: the typing rule reads an ISO date only into a cell formatted as one, so
//! [`Import::dates`] formats the cells [`dated`] picks out, ISO only. Not done, each being a
//! second rule: per-column types (format after importing), quotes meaning text (the leading-zero
//! rule covers it), and encodings other than UTF-8 (refused, naming `iconv`).

use crate::locale::{self, Locale};

/// How a file spells its rows: which character separates fields and which quotes them. Records end
/// however the file ends them, and the escape is RFC 4180's doubled quote.
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
    /// What `.csv` means where the decimal separator is a comma.
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

    /// Guess the delimiter from the file: each candidate is parsed against a sample, quotes and
    /// all, and the one that cuts every record into the same number of fields wins, more columns
    /// breaking a tie. Ambiguous input resolves to the comma; `--delimiter` overrides.
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

    /// The dialect a file name asks for, for writing, where there is nothing to sniff: `.tsv` and
    /// `.tab` are tab-separated, everything else comma. Importing never uses this — a German
    /// Excel's `.csv` is semicolon-separated whatever it is called.
    pub fn for_name(name: &str) -> Dialect {
        let extension = name
            .rsplit_once('.')
            .map(|(_, extension)| extension.to_ascii_lowercase());
        match extension.as_deref() {
            Some("tsv" | "tab") => Dialect::TAB,
            _ => Dialect::COMMA,
        }
    }
}

/// What [`decode`] says when the bytes are not UTF-8; the caller puts the file's name in front.
pub const NOT_UTF8: &str = "not UTF-8 — convert it first, e.g. iconv -f windows-1252 -t utf-8";

/// A file's bytes as text, or [`NOT_UTF8`] — never a guessed encoding (`doc/not-doing.md`, "CSV
/// encodings"). A byte-order mark is left for [`parse`] to strip.
pub fn decode(bytes: Vec<u8>) -> Result<String, &'static str> {
    String::from_utf8(bytes).map_err(|_| NOT_UTF8)
}

/// Whether a file name is a delimited file — `.csv`, `.tsv` or `.tab`, in any case. Plain text has
/// no signature, so a caller asks this only after the bytes have said they are neither ODF nor a
/// workbook.
pub fn is_delimited_name(name: &str) -> bool {
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
    matches!(
        file.rsplit_once('.').map(|(stem, ext)| (stem.is_empty(), ext.to_ascii_lowercase())),
        Some((false, ext)) if matches!(ext.as_str(), "csv" | "tsv" | "tab")
    )
}

/// A delimited file opened as a document of its own: what [`open`] hands a shell.
#[derive(Debug)]
pub struct Opened {
    /// The name it now goes by — `data.csv` becomes `data.fods`. A shell opens it with no path, so
    /// Save can never write it over the CSV.
    pub name: String,
    /// Flat ODF, for [`crate::App::open_bytes`].
    pub odf: Vec<u8>,
    /// One sentence for a notice bar, the same in every shell.
    pub summary: String,
}

/// Open a delimited file as a new spreadsheet — what a double-click on `data.csv` means, as
/// opposed to importing into an open document. `None` when `name` is not a delimited file. The
/// fields land at `A1` of one sheet named after the file, read with [`Import::sniffed`], and come
/// back as flat ODF under an ODF name, as `grind_xlsx::open` returns a workbook. Not UTF-8 is
/// [`NOT_UTF8`].
pub fn open(name: &str, bytes: &[u8]) -> Option<Result<Opened, String>> {
    if !is_delimited_name(name) {
        return None;
    }
    Some(open_delimited(name, bytes))
}

fn open_delimited(name: &str, bytes: &[u8]) -> Result<Opened, String> {
    let fail = |e: &dyn std::fmt::Display| format!("{name}: {e}");
    let text = decode(bytes.to_vec()).map_err(|e| fail(&e))?;
    let options = Import::sniffed(&text);
    let app = crate::App::new();
    app.import_csv(
        0,
        crate::model::Pos::new(0, 0),
        &text,
        &options,
        crate::RecalcMode::Document,
    )
    .map_err(|e| fail(&e))?;

    let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let stem = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
    // LibreOffice names the sheet after the file too; a stem that is no valid sheet name keeps
    // `Sheet1`.
    let _ = app.rename_sheet(0, stem);
    let odf = app.save_bytes(crate::Form::Flat).map_err(|e| fail(&e))?;

    let rows = parse(&text, &options.dialect);
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    let plural = |n: usize, one: &str| match n {
        1 => format!("1 {one}"),
        n => format!("{n} {one}s"),
    };
    let separated = match options.dialect.delimiter {
        ',' => "comma".to_owned(),
        ';' => "semicolon".to_owned(),
        '\t' => "tab".to_owned(),
        '|' => "bar".to_owned(),
        other => format!("{other:?}"),
    };
    Ok(Opened {
        name: format!("{}{stem}.fods", &name[..name.len() - file.len()]),
        odf,
        summary: format!(
            "Imported from CSV: {} × {}, {separated}-separated.",
            plural(rows.len(), "row"),
            plural(columns, "column")
        ),
    })
}

/// Read a delimited file into rows of fields. Never fails. The readings that are decisions:
///
/// * A quote opens a quoted field only where a field starts, so `12"` keeps its inch mark.
/// * Text after a closing quote is appended (`"a"b` is `ab`), and an unterminated quote runs to
///   the end.
/// * `\r\n`, `\n` and a lone `\r` all end a record.
/// * A blank line is a record of one empty field, so later rows do not shift up.
/// * A final record separator ends the file: `a\n` is one row.
pub fn parse(text: &str, dialect: &Dialect) -> Vec<Vec<String>> {
    // Excel's UTF-8 signature; left in, the first header would not match its name.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);

    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    // Whether anything was read since the last record ended — a trailing newline versus a trailing
    // empty row.
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

/// How a document leaves as delimited text. [`Export::formulas`] decides which text is handed over
/// ([`crate::App::export_csv`] reads it); the rest decide how a field is spelled ([`write()`]).
#[derive(Clone, Debug)]
pub struct Export {
    pub dialect: Dialect,
    /// Quote every field, not only the ones that need it — some readers take a quoted field as
    /// text.
    pub quote_all: bool,
    /// End records with `\r\n`, as RFC 4180 asks and Windows tools expect.
    pub crlf: bool,
    /// Begin with a UTF-8 byte-order mark, which Excel needs.
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

/// Write rows as delimited text. A field is quoted only when leaving it bare would change what a
/// reader sees: it holds the delimiter, a quote, a line break, or leading or trailing whitespace.
/// Everything else stays readable and diffable.
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
    /// Which characters spell a number: `1.234,50` is a number in `de-DE`.
    pub locale: Option<Locale>,
    /// Store every field as text, guessing nothing.
    pub text: bool,
    /// Let a field starting with `=` become a formula — off by default, as in LibreOffice, since a
    /// CSV has no formulas and evaluating text from outside is a risk.
    pub formulas: bool,
    /// Read an ISO date, datetime or time as one, formatting its cell so it stays one ([`dated`]).
    pub dates: bool,
    /// Drop leading and trailing whitespace from every field; RFC 4180 counts the space in `a, b`
    /// as part of the field.
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

impl Import {
    /// The options a window imports with, having asked nothing: the delimiter sniffed from the
    /// content ([`Dialect::sniff`]), and [`Import::dates`] on, since [`dated`] reads only ISO
    /// spellings and cannot misread `15/03/2026`. `text`, `formulas`, `locale` and `trim` stay
    /// off; each changes what a field means, and `grind sheet import-csv` offers all four.
    pub fn sniffed(text: &str) -> Import {
        Import {
            dialect: Dialect::sniff(text),
            dates: true,
            ..Import::default()
        }
    }
}

impl Import {
    /// These options with a person's words laid over them: `delimiter=semicolon locale=de-DE text
    /// trim`. A bare word turns an option on, `no-dates` turns dates off, and `delimiter=` takes
    /// `comma`, `semicolon`, `tab`, `pipe`, `space` or one character. `Err` names the word it did
    /// not understand.
    pub fn amended(mut self, words: &str) -> std::result::Result<Import, String> {
        for word in words.split_whitespace() {
            match word.split_once('=') {
                Some(("delimiter" | "d", value)) => {
                    self.dialect.delimiter = match value {
                        "comma" => ',',
                        "semicolon" => ';',
                        "tab" | "\\t" => '\t',
                        "pipe" => '|',
                        "space" => ' ',
                        one if one.chars().count() == 1 => one.chars().next().unwrap_or(','),
                        _ => {
                            return Err(format!(
                                "delimiter {value:?} is not comma, semicolon, tab, pipe, space or one character"
                            ));
                        }
                    }
                }
                Some(("locale" | "l", value)) => {
                    self.locale = Some(
                        Locale::parse(value)
                            .ok_or_else(|| format!("{value:?} is not a locale (try de-DE)"))?,
                    )
                }
                None if word == "text" => self.text = true,
                None if word == "formulas" => self.formulas = true,
                None if word == "dates" => self.dates = true,
                None if word == "no-dates" => self.dates = false,
                None if word == "trim" => self.trim = true,
                _ => {
                    return Err(format!(
                        "{word:?} is not an import option — delimiter=, locale=, text, formulas, dates, no-dates, trim"
                    ));
                }
            }
        }
        Ok(self)
    }
}

/// The format a field needs to be read as a date, datetime or time, or `None`. ISO 8601 only, with
/// a four-digit year: `15/03/2026` and `03/15/2026` mean different days and the file does not say
/// which, so those stay text; `1-2-3` is more often a part number. The parsers are
/// [`crate::formula::date`]'s.
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
    // A clock only: `parse_time` also reads ODF's `PT17H20M00S`, which nobody exports into a
    // column.
    if field.contains(':') && crate::formula::date::parse_time(field).is_some() {
        return Some(preset(Kind::Time, 0, false, ""));
    }
    None
}

/// One field, as the string a person would have typed to put it in a cell, for
/// [`crate::App::enter`]'s rule (`'` forces text, `=` is a formula, else a number, logical or
/// text). Forced to text, since each is a way a file loses data:
///
/// * `007` — a leading zero someone meant.
/// * `NaN`, `inf`, `Infinity` — names far more often than numbers.
/// * A number in the wrong locale: numbers are recognised by the locale's own separators.
/// * A leading `'`, doubled so it stays in the cell.
pub fn input(field: &str, options: &Import) -> String {
    let field = match options.trim {
        true => field.trim(),
        false => field,
    };
    // An empty field clears its cell rather than storing an empty string.
    if field.is_empty() {
        return String::new();
    }
    let as_text = || format!("'{field}");
    if options.text || field.starts_with('\'') {
        return as_text();
    }
    if field.starts_with('=') {
        // A CSV carries display form; the typing rule takes the canonical spelling, converted by
        // the parser and printer.
        return match options
            .formulas
            .then(|| crate::formula::display::from_display(field))
        {
            Some(Ok(canonical)) => canonical,
            // A formula this build cannot read stays text rather than a cell that can only be an
            // error.
            _ => as_text(),
        };
    }
    let (decimal, group) = locale::separators(options.locale.as_ref());
    if let Some(number) = number(field, decimal, group) {
        return number;
    }
    // Anything Rust reads as a number and this does not is text, deliberately.
    match field.parse::<f64>().is_ok() {
        true => as_text(),
        false => field.to_owned(),
    }
}

/// A field as a number in the canonical spelling the typing rule reads, or `None`. Separators are
/// checked by position: `1,234` is a number and `1,2,3` is not.
pub(crate) fn number(field: &str, decimal: char, group: char) -> Option<String> {
    // A no-break space groups in French and Russian; a plain one means the same.
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
    // Split off the exponent first: `1e5` is a number, `1e5,000` is not.
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
    // `007` is text; `0` and `0.5` are numbers.
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
    // It must read back as a finite number, or it is text.
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
        // In German the stop is a misplaced grouping separator, so this is text: a visible text
        // cell rather than 1.5 silently read as 15.
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

    /// Bytes in, and the one sentence that says why not.
    #[test]
    fn decoding_takes_utf8_and_names_the_fix_for_everything_else() {
        assert_eq!(decode(b"a,b\n".to_vec()).as_deref(), Ok("a,b\n"));
        // A BOM survives decoding and is `parse`'s to strip, so a file Excel marked is not
        // refused for carrying the mark.
        assert_eq!(
            decode("\u{feff}a,b\n".as_bytes().to_vec()).as_deref(),
            Ok("\u{feff}a,b\n")
        );
        // Windows-1252 "Müller" — the commonest way a real export is not UTF-8.
        assert_eq!(
            decode(vec![b'M', 0xfc, b'l', b'l', b'e', b'r']),
            Err(NOT_UTF8)
        );
        assert!(NOT_UTF8.contains("iconv"), "the sentence names the fix");
    }

    /// What a window imports with, having asked nothing: the file's own delimiter, and the one
    /// flag that cannot misread a field.
    #[test]
    fn the_sniffed_options_read_the_delimiter_and_nothing_else_a_window_did_not_ask_about() {
        let german = "id;name;when\n1;a;2026-03-15\n2;b;2026-03-16\n";
        let options = Import::sniffed(german);
        assert_eq!(options.dialect, Dialect::SEMICOLON);
        assert!(options.dates, "a date column is why anybody imports a CSV");
        assert!(!options.text);
        assert!(
            !options.formulas,
            "a leading = arriving from outside stays text"
        );
        assert!(!options.trim);
        assert!(options.locale.is_none());
        // The date flag is a precondition and not a guess: only ISO spellings are dated, so a
        // file full of `15/03/2026` is unaffected by it (`dated`).
        assert!(dated("2026-03-15", 0).is_some());
        assert!(dated("15/03/2026", 0).is_none());
    }

    /// Writing has no content to sniff, so the *name* carries the dialect — the one thing a
    /// save dialog can say.
    #[test]
    fn the_export_dialect_follows_the_name_it_is_saved_under() {
        assert_eq!(Dialect::for_name("data.tsv"), Dialect::TAB);
        assert_eq!(Dialect::for_name("DATA.TSV"), Dialect::TAB);
        assert_eq!(Dialect::for_name("data.tab"), Dialect::TAB);
        assert_eq!(Dialect::for_name("data.csv"), Dialect::COMMA);
        // Anything else, including no extension, is a comma.
        assert_eq!(Dialect::for_name("data"), Dialect::COMMA);
        assert_eq!(Dialect::for_name("notes.txt"), Dialect::COMMA);
    }

    #[test]
    fn a_delimited_name_is_known_by_its_extension_alone() {
        for name in ["data.csv", "DATA.CSV", "a/b/list.tsv", r"C:\\x\\t.tab"] {
            assert!(is_delimited_name(name), "{name}");
        }
        for name in ["data.fods", "notes.txt", ".csv", "csv", "data.csv.fods"] {
            assert!(!is_delimited_name(name), "{name}");
        }
    }

    /// What a double-click on a CSV means: a document of its own, named after the file, read
    /// the way a window's Import CSV reads one — delimiter sniffed, ISO dates dated.
    #[test]
    fn a_delimited_file_opens_as_a_document_of_its_own() {
        let opened = open(
            "/tmp/in/prices.csv",
            b"item;price;when\nnut;0,5;2026-03-15\n",
        )
        .expect("a delimited name")
        .unwrap();
        assert_eq!(opened.name, "/tmp/in/prices.fods");
        assert_eq!(
            opened.summary,
            "Imported from CSV: 2 rows × 3 columns, semicolon-separated."
        );
        let app = crate::App::new();
        app.open_bytes(&opened.name, &opened.odf).unwrap();
        assert_eq!(app.sheet_count(), 1);
        assert_eq!(app.sheet_name(0).unwrap(), "prices");
        let view = app.get_viewport(0, 0..2, 0..3).unwrap();
        assert_eq!(view.text(0, 0), Some("item"));
        assert_eq!(view.text(1, 2), Some("2026-03-15"));

        assert!(open("book.fods", b"").is_none(), "not ours to answer");
        let refused = open("latin1.csv", b"caf\xe9\n").unwrap().unwrap_err();
        assert!(
            refused.contains("latin1.csv") && refused.contains("UTF-8"),
            "{refused}"
        );
    }

    #[test]
    fn a_persons_words_lay_over_the_defaults() {
        let base = Import::sniffed("a,b\n1,2\n");
        let amended = base
            .clone()
            .amended("delimiter=semicolon locale=de-DE text trim no-dates")
            .unwrap();
        assert_eq!(amended.dialect.delimiter, ';');
        assert!(amended.text && amended.trim && !amended.dates);
        assert!(amended.locale.is_some());
        assert!(!amended.formulas);
        assert_eq!(base.clone().amended("d=|").unwrap().dialect.delimiter, '|');
        assert!(base.clone().amended("delimiter=ab").is_err());
        assert!(base.clone().amended("locale=nowhere-at-all").is_err());
        assert!(base.amended("shout").unwrap_err().contains("shout"));
    }
}
