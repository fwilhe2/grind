module static;

/// `fo:text-align` — `left`, `center`, `right` or `justify`.
fn align(style: Style, align: string) -> Style;

/// The address of a cell, from indices counted from 0 — `s.at(0, 1)` is `"B1"`.
///
/// The one place a script converts between an index and an address; the `+ 1` a
/// spreadsheet's rows carry is the core's, not the script's.
fn at(sheet: Sheet, row: int, column: int) -> string;

/// `fo:background-color`, in the same colours `color` takes.
fn background(style: Style, color: string) -> Style;

/// How many blocks have been said.
fn blocks(document: Text) -> int;

/// The whole row bold.
fn bold(row: Row) -> Row;

/// `fo:font-weight` — bold.
fn bold(style: Style) -> Style;

/// Anchor a bookmark at the start of the block last said.
///
/// `#name` then addresses it — an address that survives editing above it, which
/// `p12` does not.
fn bookmark(document: Text, name: string) -> Text;

/// All four edges — a width, a line and a colour: `"0.5pt solid navy"`.
fn border(style: Style, border: string) -> Style;

/// `fo:color` — a palette name (`navy`, `red`, `silver`, …), `#rrggbb`, or
/// `transparent`.
fn color(style: Style, color: string) -> Style;

/// Fraction digits, 0 to 255. Exactly this many, never "up to".
fn decimals(format: Format, decimals: int) -> Format;

/// A number format by name: `general`, `number`, `percent`, `currency`, `date`,
/// `datetime`, `time`, `boolean` or `text`.
///
/// Refine it with `.decimals(2)`, `.grouping()`, `.symbol("€")`, `.locale("de-DE")`.
fn format(kind: string) -> Format;

/// A number format over the row's own cells.
fn format(row: Row, format: Format) -> Row;

/// A number format by name over the row's own cells — `.format("percent")`.
fn format(row: Row, kind: string) -> Row;

/// A number format over a range — `s.format("B2:B8", format("currency"))`.
fn format(sheet: Sheet, range: string, format: Format) -> Sheet;

/// A number format by name over a range — `s.format("B17", "date")`.
fn format(sheet: Sheet, range: string, kind: string) -> Sheet;

/// A formula written the way a formula bar shows one — `formula("SUM(B2:B7)")`.
///
/// Converted to ODF syntax by the core's own converter, so a formula that does not
/// parse is an error here. The other spelling is a plain string starting with `=`,
/// which is stored exactly as written.
fn formula(source: string) -> Cell;

/// How many blocks have been said.
fn get blocks(_: Text) -> int;

/// How many rows have been written — where the next `push` will land.
fn get rows(_: Sheet) -> int;

/// Thousands separators.
fn grouping(format: Format) -> Format;

/// A heading, level 1 to 6.
///
/// The outline is implied by the levels and by nothing else — a heading does not
/// contain what follows it.
fn heading(document: Text, level: int, text: string) -> Text;

/// Row heights — `s.height("1:1", "8mm")`. The twin of `width`.
fn height(sheet: Sheet, rows: string, length: string) -> Sheet;

/// The whole row italic.
fn italic(row: Row) -> Row;

/// `fo:font-style` — italic.
fn italic(style: Style) -> Style;

/// A list item at depth 1.
fn item(document: Text, text: string) -> Text;

/// A list item, nested 1 to 9 deep.
fn item(document: Text, depth: int, text: string) -> Text;

/// The data in one JSON file, as values a script can use.
///
/// An object is a map, an array is an array, `null` is `()`, and object keys come
/// back sorted. The file is read from the data directory — the script's own, or
/// the one `--data` names — and `..`, an absolute path and a symlink pointing out
/// are all refused. Reading the same file twice is free.
fn json(name: string) -> ?;

/// The locale whose separators the format uses — `"de-DE"`.
fn locale(format: Format, tag: string) -> Format;

/// A named range (`"B2:B7"`) or a named expression (`"=MAX(budgeted)"`).
///
/// Written out qualified with this sheet, so the name means the same range read
/// from anywhere in the document.
fn name(sheet: Sheet, name: string, target: string) -> Sheet;

/// A paragraph.
///
/// The text is read in the notation the whole suite types in: `**bold**`,
/// `_italic_`, `` `code` ``.
fn para(document: Text, text: string) -> Text;

/// Append a sheet. Sheets appear in the document in the order pushed.
fn push(document: Spreadsheet, sheet: Sheet) -> Spreadsheet;

/// Append a row from a bare array — `s.push(["Housing", 1800])`.
fn push(sheet: Sheet, cells: array) -> int;

/// Append a row, and answer the index of the row it landed on (counted from 0).
///
/// `s.at(s.push(…), 2)` is an address in the row just written.
fn push(sheet: Sheet, row: Row) -> int;

/// A row of cells, ready to push.
///
/// A number is a number, a string is text, a string starting with `=` is a formula
/// in ODF syntax, `()` leaves the cell empty. `.bold()`, `.italic()`, `.style(…)`
/// and `.format(…)` cover the whole row.
fn row(cells: array) -> Row;

/// How many rows have been written — where the next `push` will land.
fn rows(sheet: Sheet) -> int;

/// One cell at one address — `s.set("B7", 1800)`. A range is an error.
///
/// A `set` below the rows pushed so far moves the next `push` under it, so mixing
/// the two cannot silently overwrite.
fn set(sheet: Sheet, at: string, value: ?) -> Sheet;

/// A sheet, not yet in any document.
///
/// Returning one *is* a one-sheet spreadsheet, so a script that builds one sheet
/// may end with it rather than with a `spreadsheet()` around it.
fn sheet(name: string) -> Sheet;

/// `fo:font-size` — an ODF length, `"9pt"`.
fn size(style: Style, size: string) -> Style;

/// An empty document. Push a sheet into it, and return it at the end of the script.
fn spreadsheet() -> Spreadsheet;

/// Styling, with nothing set yet: `style().bold().background("silver")`.
///
/// In a script styles *layer* — a later one adds to what is underneath.
fn style() -> Style;

/// Styling over the row's own cells — as many as the row holds, and no more.
fn style(row: Row, style: Style) -> Row;

/// Styling over a range, laid over whatever is already there.
///
/// A whole column (`"A:A"`) means the column the script actually filled.
fn style(sheet: Sheet, range: string, style: Style) -> Sheet;

/// The sum of the cells directly above this one.
///
/// Resolved where the cell lands, over the contiguous run of numbers or formulas
/// above it — an AutoSum's rule, so a header of text is not part of it.
fn sum_above() -> Cell;

/// The currency symbol — `"€"`, `"EUR"`.
fn symbol(format: Format, symbol: string) -> Format;

/// An empty text document. Say headings, paragraphs and list items into it, and
/// return it at the end of the script.
fn text() -> Text;

/// `style:vertical-align` — `top`, `middle` or `bottom`.
fn valign(style: Style, align: string) -> Style;

/// Column widths — `s.width("A:A", "4cm")`. An ODF length: cm, mm, pt, in.
///
/// `"A"` and `"A:A"` mean the same run of one.
fn width(sheet: Sheet, columns: string, length: string) -> Sheet;

/// `fo:wrap-option` — wrap the text inside the cell.
fn wrap(style: Style) -> Style;


