// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The spreadsheet's half of the host API — what a script may say, and what it means.
//!
//! ## The vocabulary is the projection's
//!
//! `doc/projection-sheet.md` already names every piece of a spreadsheet in plain words:
//! `sheet`, `row`, `cell`, `style … bold=#true background=navy align=center`,
//! `format … currency decimals=2 grouping=#true symbol="EUR"`. A generator that invented a
//! second set of names for the same things would be a third vocabulary for one model, and
//! `doc/dsl.md` §3.7's argument about two scope lines applies word for word to two spellings.
//! So the nouns here are those nouns, and the adjectives are those adjectives.
//!
//! ```rhai
//! let s = sheet("Budget");
//! s.push(row(["Category", "Budgeted"]).bold());
//! s.push(row(["Housing", 1800]).format(format("currency").symbol("€")));
//! s.push(row(["Total", sum_above()]).bold());
//! s.style("A1:B1", style().background("silver").align("center"));
//! s.width("A:A", "4cm");
//! s.name("budgeted", "B2:B3");
//! s
//! ```
//!
//! ## A script has types, so nothing here guesses
//!
//! `App::enter` is the *typing rule* — a leading `=` is a formula, `1800` is a number,
//! `TRUE` is a logical — and it exists because a person typing into a cell, or into a shell,
//! has only characters to say it with. **A script does not**: `1800` is already an integer and
//! `"1800"` is already a string, and re-deriving that from their spelling would be a second
//! rule that can disagree with the first. So:
//!
//! | The script writes | The cell holds |
//! |---|---|
//! | `1800`, `1825.5` | a number |
//! | `true` | a logical |
//! | `"Housing"`, `"2091"` | text, exactly as written — a string is a string |
//! | `"=SUM([.B2:.B7])"` | a formula, verbatim, in ODF syntax |
//! | `formula("SUM(B2:B7)")` | the same formula, written the way a formula bar shows one |
//! | `"'=not a formula"` | text, the quote stripped — the core's own escape, kept for the one case a script cannot otherwise spell |
//! | `()` | nothing; the cell is left empty |
//!
//! The one guess left is the leading `=`, and it is the guess every spreadsheet in the world
//! makes.
//!
//! **Both formula spellings are the suite's, not this crate's.** A `=` string is stored
//! exactly as `grind sheet set` stores one — ODF syntax, which is what the file holds and what
//! `doc/projection-sheet.md` writes. `formula(…)` is A1 syntax put through
//! `formula::display::from_display`, the same converter behind `grind sheet fmt --from-display`
//! and behind every shell's formula bar, so a script may say `SUM(B2:B7)` and get a validated
//! formula rather than a string that happened to look right.
//!
//! **A date value has no spelling here**, and that is a named gap rather than an oversight:
//! `App::enter` will read `2026-08-16` as a date *only* in a cell already known to hold one,
//! which the cells of a document being generated are not. `=DATE(2026;8;16)` under a
//! `format("date")` is how `examples/budget.rhai` writes one, exactly as
//! `examples/sample-sheet.sh` does.

use std::cell::RefCell;
use std::rc::Rc;

use grind_core::locale::Locale;
use grind_sheet::style::CellStyle;
use grind_sheet::{App, CellValue, Pos, a1, numfmt, style};
use rhai::{Array, Dynamic, Engine, EvalAltResult};

use crate::hint::{hint, hint_get};

/// What a host function hands back — a value, or a message with the script's position on it.
type Res<T> = Result<T, Box<EvalAltResult>>;

/// The largest rectangle one `style`, `format`, `width` or `height` may cover.
///
/// `App::set_format`'s own bound, restated because the layering in [`Layered`] walks the cells
/// itself and would otherwise walk a million of them for a range nobody meant.
const MAX_CELLS: u64 = 1_000_000;

// ---------------------------------------------------------------------------
// The tree
// ---------------------------------------------------------------------------

/// A document, as a script builds it: sheets in the order they were pushed.
///
/// `Rc<RefCell<…>>` here and in [`Sheet`], and it is not an implementation detail. Rhai values
/// are copied on assignment, so a `Sheet` pushed into a book and then written to again would
/// silently update a *copy* — the script would look right and the document would be missing
/// its last few rows. A shared handle makes `d.push(s)` mean what it reads as.
#[derive(Clone, Default)]
pub struct Book(Rc<RefCell<Vec<Sheet>>>);

impl Book {
    /// The one-sheet book, for a script that returned a bare `sheet(…)`.
    pub fn of(sheet: Sheet) -> Book {
        Book(Rc::new(RefCell::new(vec![sheet])))
    }
}

/// One sheet: its name, the ops that fill it, and where the next `push` lands.
#[derive(Clone)]
pub struct Sheet(Rc<RefCell<SheetSpec>>);

struct SheetSpec {
    name: String,
    /// The next free row for [`Sheet::push`], 0-based.
    next_row: u32,
    ops: Vec<Op>,
}

/// One thing a script asked for, in the order it asked.
///
/// Ranges are kept as the strings they were written as and resolved against the *finished*
/// sheet — see [`materialise`], which is where the two-pass order is argued.
enum Op {
    Cell {
        pos: Pos,
        cell: Cell,
    },
    Format {
        range: String,
        format: numfmt::Format,
    },
    Style {
        range: String,
        style: CellStyle,
    },
    Width {
        range: String,
        length: String,
    },
    Height {
        range: String,
        length: String,
    },
    Name {
        name: String,
        target: String,
    },
}

/// What one cell of a pushed row is. Everything but the last is decided when the script says
/// it; `SumAbove` cannot be, which is the whole reason this is a tree and not a document.
#[derive(Clone, Debug, PartialEq)]
pub enum Cell {
    Empty,
    Value(CellValue),
    /// ODF syntax, stored verbatim, with its leading `=`.
    Formula(String),
    /// `sum_above()` — resolved against the rows above the cell it lands in.
    SumAbove,
}

/// A row of cells, with the formatting the whole row carries.
#[derive(Clone, Default)]
pub struct Row {
    cells: Vec<Cell>,
    style: CellStyle,
    format: Option<numfmt::Format>,
}

/// How cells look — `doc/projection-sheet.md`'s `style` node, one method per attribute.
#[derive(Clone, Default)]
pub struct Style(CellStyle);

/// How a value is spelled — `doc/projection-sheet.md`'s `format` node.
///
/// A *request* for a format rather than one: `numfmt::preset` builds the real thing at
/// [`Fmt::format`], so the generator cannot produce a format the format picker in a shell
/// could not also produce. A format outside that vocabulary is a named gap — the projection
/// spells one part by part, and a generator that could too would be inventing Excel's format
/// codes (`doc/dsl.md` §3.8).
#[derive(Clone)]
pub struct Fmt {
    kind: Option<numfmt::Kind>,
    /// `datetime` is not a [`numfmt::Kind`] — §4.3.4's DateTime is a Date whose value carries
    /// a fraction — so it is its own flag, as it is in the CLI.
    datetime: bool,
    decimals: u8,
    grouping: bool,
    symbol: String,
    locale: Option<Locale>,
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// Everything a script may say about a spreadsheet.
///
/// **Every registration carries its own documentation**, and that is not decoration: `grind
/// definitions` turns what is written here into a `.d.rhai` file, which is what an editor reads
/// to complete a call and to show what it does while somebody is typing it
/// (`doc/generator-spec.md` §9). A function registered without a comment is a function whose
/// hover says nothing, so [`hint`] takes both and there is no shorter way to add one.
pub fn register(engine: &mut Engine) {
    engine
        .register_type_with_name::<Book>("Spreadsheet")
        .register_type_with_name::<Sheet>("Sheet")
        .register_type_with_name::<Row>("Row")
        .register_type_with_name::<Cell>("Cell")
        .register_type_with_name::<Style>("Style")
        .register_type_with_name::<Fmt>("Format");

    // --- constructors ---
    hint(
        engine,
        "spreadsheet",
        ["Spreadsheet"],
        ["/// An empty document. Push a sheet into it, and return it at the end of the script."],
        Book::default,
    );
    hint(
        engine,
        "sheet",
        ["name: string", "Sheet"],
        [
            "/// A sheet, not yet in any document.",
            "///",
            "/// Returning one *is* a one-sheet spreadsheet, so a script that builds one sheet",
            "/// may end with it rather than with a `spreadsheet()` around it.",
        ],
        |name: &str| Sheet::named(name),
    );
    hint(
        engine,
        "row",
        ["cells: array", "Row"],
        [
            "/// A row of cells, ready to push.",
            "///",
            "/// A number is a number, a string is text, a string starting with `=` is a formula",
            "/// in ODF syntax, `()` leaves the cell empty. `.bold()`, `.italic()`, `.style(…)`",
            "/// and `.format(…)` cover the whole row.",
        ],
        |cells: Array| -> Res<Row> { Row::of(cells) },
    );
    hint(
        engine,
        "style",
        ["Style"],
        [
            "/// Styling, with nothing set yet: `style().bold().background(\"silver\")`.",
            "///",
            "/// In a script styles *layer* — a later one adds to what is underneath.",
        ],
        Style::default,
    );
    hint(
        engine,
        "format",
        ["kind: string", "Format"],
        [
            "/// A number format by name: `general`, `number`, `percent`, `currency`, `date`,",
            "/// `datetime`, `time`, `boolean` or `text`.",
            "///",
            "/// Refine it with `.decimals(2)`, `.grouping()`, `.symbol(\"€\")`, `.locale(\"de-DE\")`.",
        ],
        |kind: &str| -> Res<Fmt> { Fmt::named(kind) },
    );
    hint(
        engine,
        "formula",
        ["source: string", "Cell"],
        [
            "/// A formula written the way a formula bar shows one — `formula(\"SUM(B2:B7)\")`.",
            "///",
            "/// Converted to ODF syntax by the core's own converter, so a formula that does not",
            "/// parse is an error here. The other spelling is a plain string starting with `=`,",
            "/// which is stored exactly as written.",
        ],
        |source: &str| -> Res<Cell> { Cell::formula(source) },
    );
    hint(
        engine,
        "sum_above",
        ["Cell"],
        [
            "/// The sum of the cells directly above this one.",
            "///",
            "/// Resolved where the cell lands, over the contiguous run of numbers or formulas",
            "/// above it — an AutoSum's rule, so a header of text is not part of it.",
        ],
        || Cell::SumAbove,
    );

    // --- the book ---
    hint(
        engine,
        "push",
        ["document: Spreadsheet", "sheet: Sheet", "Spreadsheet"],
        ["/// Append a sheet. Sheets appear in the document in the order pushed."],
        |book: &mut Book, sheet: Sheet| {
            book.0.borrow_mut().push(sheet);
            book.clone()
        },
    );

    // Every method that *asks for something* hands its receiver back, so a script may chain.
    // It is a shared handle, so the one that comes back is the one that went in — see [`Book`].
    // `push` is the exception and returns where the row landed, which is the answer a script
    // has no other way to get.
    hint(
        engine,
        "push",
        ["sheet: Sheet", "row: Row", "int"],
        [
            "/// Append a row, and answer the index of the row it landed on (counted from 0).",
            "///",
            "/// `s.at(s.push(…), 2)` is an address in the row just written.",
        ],
        |sheet: &mut Sheet, row: Row| -> Res<i64> { sheet.push(row) },
    );
    hint(
        engine,
        "push",
        ["sheet: Sheet", "cells: array", "int"],
        ["/// Append a row from a bare array — `s.push([\"Housing\", 1800])`."],
        |sheet: &mut Sheet, cells: Array| -> Res<i64> {
            let row = Row::of(cells)?;
            sheet.push(row)
        },
    );
    hint(
        engine,
        "set",
        ["sheet: Sheet", "at: string", "value: ?", "Sheet"],
        [
            "/// One cell at one address — `s.set(\"B7\", 1800)`. A range is an error.",
            "///",
            "/// A `set` below the rows pushed so far moves the next `push` under it, so mixing",
            "/// the two cannot silently overwrite.",
        ],
        |sheet: &mut Sheet, at: &str, value: Dynamic| -> Res<Sheet> {
            sheet.set(at, value)?;
            Ok(sheet.clone())
        },
    );
    hint(
        engine,
        "format",
        ["sheet: Sheet", "range: string", "format: Format", "Sheet"],
        ["/// A number format over a range — `s.format(\"B2:B8\", format(\"currency\"))`."],
        |sheet: &mut Sheet, range: &str, f: Fmt| -> Res<Sheet> {
            sheet.format(range, &f)?;
            Ok(sheet.clone())
        },
    );
    hint(
        engine,
        "format",
        ["sheet: Sheet", "range: string", "kind: string", "Sheet"],
        ["/// A number format by name over a range — `s.format(\"B17\", \"date\")`."],
        |sheet: &mut Sheet, range: &str, kind: &str| -> Res<Sheet> {
            sheet.format(range, &Fmt::named(kind)?)?;
            Ok(sheet.clone())
        },
    );
    hint(
        engine,
        "style",
        ["sheet: Sheet", "range: string", "style: Style", "Sheet"],
        [
            "/// Styling over a range, laid over whatever is already there.",
            "///",
            "/// A whole column (`\"A:A\"`) means the column the script actually filled.",
        ],
        |sheet: &mut Sheet, range: &str, s: Style| {
            sheet.style(range, &s);
            sheet.clone()
        },
    );
    hint(
        engine,
        "width",
        ["sheet: Sheet", "columns: string", "length: string", "Sheet"],
        [
            "/// Column widths — `s.width(\"A:A\", \"4cm\")`. An ODF length: cm, mm, pt, in.",
            "///",
            "/// `\"A\"` and `\"A:A\"` mean the same run of one.",
        ],
        |sheet: &mut Sheet, cols: &str, length: &str| {
            sheet.track(true, cols, length);
            sheet.clone()
        },
    );
    hint(
        engine,
        "height",
        ["sheet: Sheet", "rows: string", "length: string", "Sheet"],
        ["/// Row heights — `s.height(\"1:1\", \"8mm\")`. The twin of `width`."],
        |sheet: &mut Sheet, rows: &str, length: &str| {
            sheet.track(false, rows, length);
            sheet.clone()
        },
    );
    hint(
        engine,
        "name",
        ["sheet: Sheet", "name: string", "target: string", "Sheet"],
        [
            "/// A named range (`\"B2:B7\"`) or a named expression (`\"=MAX(budgeted)\"`).",
            "///",
            "/// Written out qualified with this sheet, so the name means the same range read",
            "/// from anywhere in the document.",
        ],
        |sheet: &mut Sheet, name: &str, target: &str| {
            sheet.name(name, target);
            sheet.clone()
        },
    );
    // Both spellings, because a script reaches for `s.rows` as often as `s.rows()` and being
    // told which one this build wants is not a lesson worth teaching.
    hint(
        engine,
        "rows",
        ["sheet: Sheet", "int"],
        ["/// How many rows have been written — where the next `push` will land."],
        |sheet: &mut Sheet| sheet.0.borrow().next_row as i64,
    );
    hint_get(
        engine,
        "rows",
        ["/// How many rows have been written — where the next `push` will land."],
        |sheet: &mut Sheet| sheet.0.borrow().next_row as i64,
    );
    hint(
        engine,
        "at",
        ["sheet: Sheet", "row: int", "column: int", "string"],
        [
            "/// The address of a cell, from indices counted from 0 — `s.at(0, 1)` is `\"B1\"`.",
            "///",
            "/// The one place a script converts between an index and an address; the `+ 1` a",
            "/// spreadsheet's rows carry is the core's, not the script's.",
        ],
        |sheet: &mut Sheet, row: i64, col: i64| -> Res<String> {
            let _ = sheet;
            address(row, col)
        },
    );

    // --- a row ---
    hint(
        engine,
        "bold",
        ["row: Row", "Row"],
        ["/// The whole row bold."],
        |row: &mut Row| row.with(|s| s.font_weight = weight()),
    );
    hint(
        engine,
        "italic",
        ["row: Row", "Row"],
        ["/// The whole row italic."],
        |row: &mut Row| row.with(|s| s.font_style = slant()),
    );
    hint(
        engine,
        "style",
        ["row: Row", "style: Style", "Row"],
        ["/// Styling over the row's own cells — as many as the row holds, and no more."],
        |row: &mut Row, style: Style| {
            let mut out = row.clone();
            out.style = style.0;
            out
        },
    );
    hint(
        engine,
        "format",
        ["row: Row", "format: Format", "Row"],
        ["/// A number format over the row's own cells."],
        |row: &mut Row, format: Fmt| -> Res<Row> {
            let mut out = row.clone();
            out.format = Some(format.format()?);
            Ok(out)
        },
    );
    hint(
        engine,
        "format",
        ["row: Row", "kind: string", "Row"],
        ["/// A number format by name over the row's own cells — `.format(\"percent\")`."],
        |row: &mut Row, kind: &str| -> Res<Row> {
            let mut out = row.clone();
            out.format = Some(Fmt::named(kind)?.format()?);
            Ok(out)
        },
    );

    // --- a style ---
    hint(
        engine,
        "bold",
        ["style: Style", "Style"],
        ["/// `fo:font-weight` — bold."],
        |s: &mut Style| s.with(|c| c.font_weight = weight()),
    );
    hint(
        engine,
        "italic",
        ["style: Style", "Style"],
        ["/// `fo:font-style` — italic."],
        |s: &mut Style| s.with(|c| c.font_style = slant()),
    );
    hint(
        engine,
        "wrap",
        ["style: Style", "Style"],
        ["/// `fo:wrap-option` — wrap the text inside the cell."],
        |s: &mut Style| s.with(|c| c.wrap = Some("wrap".to_owned())),
    );
    hint(
        engine,
        "size",
        ["style: Style", "size: string", "Style"],
        ["/// `fo:font-size` — an ODF length, `\"9pt\"`."],
        |s: &mut Style, size: &str| {
            let size = size.to_owned();
            s.with(move |c| c.font_size = Some(size))
        },
    );
    hint(
        engine,
        "color",
        ["style: Style", "color: string", "Style"],
        [
            "/// `fo:color` — a palette name (`navy`, `red`, `silver`, …), `#rrggbb`, or",
            "/// `transparent`.",
        ],
        |s: &mut Style, color: &str| -> Res<Style> {
            let color = style::color(color).map_err(bad)?;
            Ok(s.with(move |c| c.color = Some(color)))
        },
    );
    hint(
        engine,
        "background",
        ["style: Style", "color: string", "Style"],
        ["/// `fo:background-color`, in the same colours `color` takes."],
        |s: &mut Style, color: &str| -> Res<Style> {
            let color = style::color(color).map_err(bad)?;
            Ok(s.with(move |c| c.background = Some(color)))
        },
    );
    hint(
        engine,
        "align",
        ["style: Style", "align: string", "Style"],
        ["/// `fo:text-align` — `left`, `center`, `right` or `justify`."],
        |s: &mut Style, align: &str| -> Res<Style> {
            let align = self::align(align)?;
            Ok(s.with(move |c| c.align = Some(align)))
        },
    );
    hint(
        engine,
        "valign",
        ["style: Style", "align: string", "Style"],
        ["/// `style:vertical-align` — `top`, `middle` or `bottom`."],
        |s: &mut Style, align: &str| -> Res<Style> {
            let align = self::valign(align)?;
            Ok(s.with(move |c| c.vertical_align = Some(align)))
        },
    );
    hint(
        engine,
        "border",
        ["style: Style", "border: string", "Style"],
        ["/// All four edges — a width, a line and a colour: `\"0.5pt solid navy\"`."],
        |s: &mut Style, border: &str| -> Res<Style> {
            let border = style::border(border).map_err(bad)?;
            Ok(s.with(move |c| c.set_border(Some(border))))
        },
    );

    // --- a format ---
    hint(
        engine,
        "decimals",
        ["format: Format", "decimals: int", "Format"],
        ["/// Fraction digits, 0 to 255. Exactly this many, never \"up to\"."],
        |f: &mut Fmt, decimals: i64| -> Res<Fmt> {
            let mut out = f.clone();
            out.decimals = u8::try_from(decimals)
                .map_err(|_| bad(format!("{decimals}: decimal places run from 0 to 255")))?;
            Ok(out)
        },
    );
    hint(
        engine,
        "grouping",
        ["format: Format", "Format"],
        ["/// Thousands separators."],
        |f: &mut Fmt| {
            let mut out = f.clone();
            out.grouping = true;
            out
        },
    );
    hint(
        engine,
        "symbol",
        ["format: Format", "symbol: string", "Format"],
        ["/// The currency symbol — `\"€\"`, `\"EUR\"`."],
        |f: &mut Fmt, symbol: &str| {
            let mut out = f.clone();
            out.symbol = symbol.to_owned();
            out
        },
    );
    hint(
        engine,
        "locale",
        ["format: Format", "tag: string", "Format"],
        ["/// The locale whose separators the format uses — `\"de-DE\"`."],
        |f: &mut Fmt, tag: &str| -> Res<Fmt> {
            let mut out = f.clone();
            out.locale = Some(
                Locale::parse(tag)
                    .ok_or_else(|| bad(format!("{tag}: expected a language tag like de-DE")))?,
            );
            Ok(out)
        },
    );
}

/// A message with no position of its own — Rhai adds the call site's.
fn bad(message: impl Into<String>) -> Box<EvalAltResult> {
    message.into().into()
}

fn weight() -> Option<String> {
    Some("bold".to_owned())
}

fn slant() -> Option<String> {
    Some("italic".to_owned())
}

/// §16.5's writing-direction-relative values, in the spelling a person uses — the same
/// translation `grind sheet style --align` makes, and the same words the projection writes.
fn align(align: &str) -> Res<String> {
    Ok(match align {
        "left" => "start",
        "center" => "center",
        "right" => "end",
        "justify" => "justify",
        other => {
            return Err(bad(format!(
                "{other}: expected left, center, right or justify"
            )));
        }
    }
    .to_owned())
}

fn valign(align: &str) -> Res<String> {
    match align {
        "top" | "middle" | "bottom" => Ok(align.to_owned()),
        other => Err(bad(format!("{other}: expected top, middle or bottom"))),
    }
}

/// A 0-based row and column as the address a person reads — `at(0, 1)` is `B1`.
///
/// **The indices are the script's own**: a row is where it sits in the sheet and a column is
/// where a cell sits in a `row([…])` array, both counted from zero like every other index in
/// the language. The 1-based spelling belongs to the address, and turning one into the other
/// is `a1::format`'s job and nobody else's — the workspace's one `+ 1`.
/// Whether a cell is on the grid at all — `grind_sheet`'s own extent, not a second opinion.
fn within(row: u32, col: u32) -> Res<()> {
    match (row < grind_sheet::MAX_ROWS, col < grind_sheet::MAX_COLS) {
        (true, true) => Ok(()),
        _ => Err(bad(format!(
            "row {} column {} is past the end of the sheet",
            row as u64 + 1,
            col as u64 + 1
        ))),
    }
}

fn address(row: i64, col: i64) -> Res<String> {
    let axis = |n: i64, what: &str| {
        u32::try_from(n).map_err(|_| bad(format!("{n}: a {what} index starts at 0")))
    };
    Ok(a1::format(
        None,
        Pos::new(axis(row, "row")?, axis(col, "column")?),
    ))
}

// ---------------------------------------------------------------------------
// The builders
// ---------------------------------------------------------------------------

impl Sheet {
    fn named(name: &str) -> Sheet {
        Sheet(Rc::new(RefCell::new(SheetSpec {
            name: name.to_owned(),
            next_row: 0,
            ops: Vec::new(),
        })))
    }

    /// Append a row, and answer where it landed — 0-based, so `s.at(s.push(…), 2)` is an
    /// address in the row just written.
    ///
    /// The sheet's own limits are checked here rather than left to the writer: a script that
    /// walks off the end of the grid should be told which line did it, and a document with a
    /// cell past `XFD1048576` in it is not a document.
    fn push(&mut self, row: Row) -> Res<i64> {
        let mut spec = self.0.borrow_mut();
        let at = spec.next_row;
        let width = row.cells.len() as u32;
        self::within(at, width.saturating_sub(1))?;
        spec.next_row += 1;
        for (col, cell) in row.cells.into_iter().enumerate() {
            let pos = Pos::new(at, col as u32);
            spec.ops.push(Op::Cell { pos, cell });
        }
        // An empty row still advances: `push([])` is how a script leaves a gap.
        if width > 0 {
            let range = format!(
                "{}:{}",
                a1::format(None, Pos::new(at, 0)),
                a1::format(None, Pos::new(at, width - 1))
            );
            if !row.style.is_plain() {
                spec.ops.push(Op::Style {
                    range: range.clone(),
                    style: row.style,
                });
            }
            if let Some(format) = row.format {
                spec.ops.push(Op::Format { range, format });
            }
        }
        Ok(at as i64)
    }

    fn set(&mut self, at: &str, value: Dynamic) -> Res<()> {
        let pos = self::cell_address(at)?;
        self::within(pos.row, pos.col)?;
        let cell = Cell::of(value)?;
        let mut spec = self.0.borrow_mut();
        // A `set` below everything pushed so far moves `push` out of its way, so mixing the
        // two cannot silently overwrite: the next `push` lands under the lowest row written.
        spec.next_row = spec.next_row.max(pos.row + 1);
        spec.ops.push(Op::Cell { pos, cell });
        Ok(())
    }

    fn format(&mut self, range: &str, format: &Fmt) -> Res<()> {
        self.0.borrow_mut().ops.push(Op::Format {
            range: range.to_owned(),
            format: format.format()?,
        });
        Ok(())
    }

    fn style(&mut self, range: &str, style: &Style) {
        self.0.borrow_mut().ops.push(Op::Style {
            range: range.to_owned(),
            style: style.0.clone(),
        });
    }

    fn track(&mut self, columns: bool, range: &str, length: &str) {
        // `A` and `A:A` mean the same run of one, as they do at the command line.
        let range = match range.contains(':') {
            true => range.to_owned(),
            false => format!("{range}:{range}"),
        };
        let length = length.to_owned();
        let mut spec = self.0.borrow_mut();
        spec.ops.push(match columns {
            true => Op::Width { range, length },
            false => Op::Height { range, length },
        });
    }

    fn name(&mut self, name: &str, target: &str) {
        self.0.borrow_mut().ops.push(Op::Name {
            name: name.to_owned(),
            target: target.to_owned(),
        });
    }
}

impl Row {
    fn of(cells: Array) -> Res<Row> {
        Ok(Row {
            cells: cells
                .into_iter()
                .map(Cell::of)
                .collect::<Res<Vec<Cell>>>()?,
            ..Row::default()
        })
    }

    fn with(&mut self, f: impl FnOnce(&mut CellStyle)) -> Row {
        let mut out = self.clone();
        f(&mut out.style);
        out
    }
}

impl Style {
    fn with(&mut self, f: impl FnOnce(&mut CellStyle)) -> Style {
        let mut out = self.clone();
        f(&mut out.0);
        out
    }
}

impl Cell {
    /// One Rhai value as one cell — the table in this module's comment, executable.
    fn of(value: Dynamic) -> Res<Cell> {
        if value.is_unit() {
            return Ok(Cell::Empty);
        }
        if let Some(cell) = value.clone().try_cast::<Cell>() {
            return Ok(cell);
        }
        if value.is_int() {
            return Ok(Cell::Value(CellValue::Number(
                value.as_int().unwrap() as f64
            )));
        }
        if value.is_float() {
            return Ok(Cell::Value(CellValue::Number(value.as_float().unwrap())));
        }
        if value.is_bool() {
            return Ok(Cell::Value(CellValue::Bool(value.as_bool().unwrap())));
        }
        if value.is_string() {
            let text = value.into_immutable_string().unwrap();
            return Ok(Cell::of_str(&text));
        }
        Err(bad(format!(
            "{} is not something a cell can hold — a number, a string, a logical, () or a \
             formula(…)",
            value.type_name()
        )))
    }

    fn of_str(text: &str) -> Cell {
        if let Some(text) = text.strip_prefix('\'') {
            return Cell::Value(CellValue::Text(text.to_owned()));
        }
        match text.starts_with('=') {
            true => Cell::Formula(text.to_owned()),
            false => Cell::Value(CellValue::Text(text.to_owned())),
        }
    }

    /// A formula written the way a formula bar shows one — `SUM(B2:B7)` — converted by the
    /// core's own converter rather than by a rule invented here. The leading `=` is optional
    /// because a script is not typing into a cell and has nothing to disambiguate.
    fn formula(source: &str) -> Res<Cell> {
        let display = match source.starts_with('=') {
            true => source.to_owned(),
            false => format!("={source}"),
        };
        let canonical = grind_sheet::formula::display::from_display(&display)
            .map_err(|e| bad(format!("{source}: {e}")))?;
        Ok(Cell::Formula(canonical))
    }
}

impl Fmt {
    /// The vocabulary `grind sheet format` takes and `doc/projection-sheet.md` writes, plus
    /// `general` for no format at all.
    fn named(kind: &str) -> Res<Fmt> {
        let (kind, datetime) = match kind {
            "general" => (None, false),
            "number" => (Some(numfmt::Kind::Number), false),
            "percent" => (Some(numfmt::Kind::Percentage), false),
            "currency" => (Some(numfmt::Kind::Currency), false),
            "date" => (Some(numfmt::Kind::Date), false),
            "datetime" => (None, true),
            "time" => (Some(numfmt::Kind::Time), false),
            "boolean" => (Some(numfmt::Kind::Boolean), false),
            "text" => (Some(numfmt::Kind::Text), false),
            other => {
                return Err(bad(format!(
                    "{other}: expected general, number, percent, currency, date, datetime, \
                     time, boolean or text"
                )));
            }
        };
        Ok(Fmt {
            kind,
            datetime,
            decimals: 0,
            grouping: false,
            symbol: String::new(),
            locale: None,
        })
    }

    /// The format itself. `general` has none — and asking for `format("general")` where a
    /// format is required is a script saying nothing at all, which is an error rather than a
    /// silent no-op.
    fn format(&self) -> Res<numfmt::Format> {
        let format = match (self.kind, self.datetime) {
            (_, true) => numfmt::datetime_preset(),
            (Some(kind), _) => numfmt::preset(kind, self.decimals, self.grouping, &self.symbol),
            (None, _) => {
                return Err(bad(
                    "format(\"general\") is the absence of a format; leave the cells alone \
                     instead",
                ));
            }
        };
        Ok(format.in_locale(self.locale.clone()))
    }
}

/// One address, for `set` — a range there is a script meaning something it cannot have.
fn cell_address(at: &str) -> Res<Pos> {
    let reference = a1::parse(at).map_err(|e| bad(e.to_string()))?;
    if !a1::is_single(&reference) {
        return Err(bad(format!("{at}: set takes one cell, not a range")));
    }
    let (row, col) = (reference.start.row, reference.start.col);
    match (row, col) {
        (Some(row), Some(col)) => Ok(Pos::new(row.index, col.index)),
        _ => Err(bad(format!("{at}: expected a cell, like B7"))),
    }
}

// ---------------------------------------------------------------------------
// The tree as a document
// ---------------------------------------------------------------------------

/// Build the document a script described.
///
/// **Cells first, then everything that decorates them**, which is not the order the script
/// said them in and is the order it meant. A range is resolved against the sheet's used
/// extent, so `style("A:A", …)` means the column the script actually filled — and it would
/// mean an empty column if it were applied when the script said it. Within each pass the
/// script's order is kept, so a second `format` over the same cells wins, as the last word
/// should.
pub fn materialise(book: &Book) -> Result<App, String> {
    let app = App::new();
    let sheets = book.0.borrow();
    if sheets.is_empty() {
        return Err("a spreadsheet needs at least one sheet".to_owned());
    }
    for (nth, sheet) in sheets.iter().enumerate() {
        let spec = sheet.0.borrow();
        // The document arrives with one sheet already; the first one named takes it over
        // rather than leaving an empty `Sheet1` in front of everything.
        let index = match nth {
            0 => {
                app.rename_sheet(0, &spec.name).map_err(say)?;
                0
            }
            _ => app.add_sheet(&spec.name).map_err(say)?,
        };
        for op in &spec.ops {
            if let Op::Cell { pos, cell } = op {
                self::cell(&app, index, *pos, cell)?;
            }
        }
        let mut styles = Layered::default();
        for op in &spec.ops {
            self::decoration(&app, index, op, &mut styles)?;
        }
        styles.apply(&app, index)?;
    }
    Ok(app)
}

/// The styling a script asked for, cell by cell, before any of it is applied.
///
/// **Styles layer here, and `App::set_style` replaces.** Both are right for who they serve: a
/// toolbar's Bold button sets what a cell *is*, and a shell that wants "bold as well" reads
/// first, which is one call and keeps a merge policy out of the core. A script is not a
/// toolbar — it says `row(…).bold()` and then `style("A1:H1", style().background("silver"))`,
/// meaning both, in the order a stylesheet means both. Replacing there would silently drop the
/// bold, which is the kind of quiet loss a generated document is least likely to be checked
/// for.
///
/// So the layering is this crate's rule, applied to what the script said, and the *result* is
/// one ordinary `set_style` per run of cells that agree — nothing the core has to know about.
#[derive(Default)]
struct Layered(std::collections::BTreeMap<(u32, u32), CellStyle>);

impl Layered {
    /// A style over a rectangle, laid over whatever is already there. Fields the new style
    /// leaves unset keep the value underneath, edge by edge for the borders.
    fn add(&mut self, start: Pos, end: Pos, style: &CellStyle) {
        for row in start.row..=end.row {
            for col in start.col..=end.col {
                let under = self.0.entry((row, col)).or_default();
                let over = |a: &mut Option<String>, b: &Option<String>| {
                    if b.is_some() {
                        a.clone_from(b);
                    }
                };
                over(&mut under.font_weight, &style.font_weight);
                over(&mut under.font_style, &style.font_style);
                over(&mut under.font_size, &style.font_size);
                over(&mut under.color, &style.color);
                over(&mut under.background, &style.background);
                over(&mut under.align, &style.align);
                over(&mut under.vertical_align, &style.vertical_align);
                over(&mut under.wrap, &style.wrap);
                for edge in 0..4 {
                    let (a, b) = (&mut under.borders[edge], &style.borders[edge]);
                    over(a, b);
                }
            }
        }
    }

    /// One `set_style` per run of adjacent cells in a row that ended up agreeing — which is
    /// what a script's row-at-a-time styling produces, so this is one call per row rather than
    /// one per cell.
    fn apply(self, app: &App, sheet: usize) -> Result<(), String> {
        let mut run: Option<(u32, u32, u32, CellStyle)> = None;
        for ((row, col), style) in self.0 {
            match run.take() {
                Some((at, first, last, held)) if at == row && col == last + 1 && held == style => {
                    run = Some((at, first, col, held));
                }
                held => {
                    self::flush(app, sheet, held)?;
                    run = Some((row, col, col, style));
                }
            }
        }
        self::flush(app, sheet, run)
    }
}

fn flush(app: &App, sheet: usize, run: Option<(u32, u32, u32, CellStyle)>) -> Result<(), String> {
    let Some((row, first, last, style)) = run else {
        return Ok(());
    };
    app.set_style(
        sheet,
        Pos::new(row, first),
        Pos::new(row, last),
        Some(style),
    )
    .map_err(say)
    .map(drop)
}

fn say(error: grind_sheet::Error) -> String {
    error.to_string()
}

fn cell(app: &App, sheet: usize, pos: Pos, cell: &Cell) -> Result<(), String> {
    match cell {
        Cell::Empty => Ok(()),
        Cell::Value(value) => app.set_cell(sheet, pos, value.clone()).map_err(say),
        Cell::Formula(formula) => app.set_formula(sheet, pos, formula).map_err(say),
        Cell::SumAbove => {
            let formula = sum_above(app, sheet, pos)?;
            app.set_formula(sheet, pos, &formula).map_err(say)
        }
    }
}

/// `sum_above()` as a formula, resolved where the cell landed.
///
/// **The rule is the one a spreadsheet's AutoSum uses**: the contiguous run of cells directly
/// above that hold a number or a formula. Not "everything above", which would reach up into
/// the header row and sum a heading; not "the whole column", which cannot be written before
/// the sheet exists. A script that wants a different range writes the formula, which is one
/// string and needs nothing from this.
///
/// `doc/dsl.md` §4.2's sketch spelled it `sum_above(i + 1)`, passing the column. Building it
/// took the argument away: the cell knows which column it landed in, and a helper that can be
/// told a *different* one is a helper that can disagree with where its answer sits.
fn sum_above(app: &App, sheet: usize, pos: Pos) -> Result<String, String> {
    let filled = |row: u32| -> Result<bool, String> {
        let at = Pos::new(row, pos.col);
        let value = app.get(sheet, at).map_err(say)?;
        let formula = app.formula(sheet, at).map_err(say)?;
        Ok(matches!(value, CellValue::Number(_)) || formula.is_some())
    };
    let mut top = None;
    let mut row = pos.row;
    while row > 0 && filled(row - 1)? {
        row -= 1;
        top = Some(row);
    }
    let Some(top) = top else {
        return Err(format!(
            "sum_above() at {}: there is no run of numbers above it to sum",
            a1::format(None, pos)
        ));
    };
    Ok(format!(
        "=SUM([.{}:.{}])",
        a1::format(None, Pos::new(top, pos.col)),
        a1::format(None, Pos::new(pos.row - 1, pos.col))
    ))
}

fn decoration(app: &App, sheet: usize, op: &Op, styles: &mut Layered) -> Result<(), String> {
    // Resolved *in* this sheet: an unqualified range in a script means the sheet it was
    // written on, not the first one (`a1::resolve_in`).
    let place = |range: &str| -> Result<(usize, Pos, Pos), String> {
        let reference = a1::parse(range).map_err(say)?;
        let (sheet, start, end) = a1::resolve_in(app, sheet, &reference).map_err(say)?;
        // The same bound `App::set_format` puts on a rectangle, checked here because the
        // layering walks the cells itself: `A1:XFD1048576` is a memory-exhaustion request
        // rather than an intent, whoever wrote it.
        let cells = (u64::from(end.row - start.row) + 1) * (u64::from(end.col - start.col) + 1);
        match cells > MAX_CELLS {
            true => Err(format!(
                "{range}: {cells} cells is more than one script may decorate"
            )),
            false => Ok((sheet, start, end)),
        }
    };
    match op {
        Op::Cell { .. } => Ok(()),
        Op::Format { range, format } => {
            let (sheet, start, end) = place(range)?;
            app.set_format(sheet, start, end, Some(format.clone()))
                .map_err(say)
                .map(drop)
        }
        Op::Style { range, style } => {
            let (_, start, end) = place(range)?;
            styles.add(start, end, style);
            Ok(())
        }
        Op::Width { range, length } => {
            let (sheet, start, end) = place(range)?;
            app.set_col_width(sheet, start.col..end.col + 1, Some(length.clone()))
                .map_err(say)
                .map(drop)
        }
        Op::Height { range, length } => {
            let (sheet, start, end) = place(range)?;
            app.set_row_height(sheet, start.row..end.row + 1, Some(length.clone()))
                .map_err(say)
                .map(drop)
        }
        // A name is document-level in ODF (§5.11) and is written here anyway: a script says
        // it on the sheet whose cells it names, and the definition is qualified with that
        // sheet, which is what makes `budgeted` mean the same range read from anywhere.
        Op::Name { name, target } => {
            let definition = match target.strip_prefix('=') {
                Some(formula) => formula.to_owned(),
                None => {
                    let (sheet, start, end) = place(target)?;
                    let named = app.sheet_name(sheet).map_err(say)?;
                    let reference = a1::reference(Some(&named), start, end);
                    a1::as_definition(app, &reference).map_err(say)?
                }
            };
            app.set_name(name, &definition).map_err(say)
        }
    }
}
