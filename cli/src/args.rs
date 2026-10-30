// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later
//! The command line as clap parses it: every subcommand, its arguments and their values. Every
//! `///` here is `--help` text, written for the person running the command.

use super::*;

fn long_version() -> &'static str {
    use std::sync::OnceLock;
    static VERSION: OnceLock<String> = OnceLock::new();
    VERSION.get_or_init(|| grind_sheet::build_info::describe_version(env!("CARGO_PKG_VERSION")))
}

#[derive(Parser)]
#[command(
    name = "grind",
    version,
    long_version = long_version(),
    about = "Drive the ODF suite from the shell",
    long_about = "Drive the ODF suite from the shell.\n\n\
        Commands are grouped by the kind of document they act on — `grind sheet …` for \
        spreadsheets — with a few verbs at the top level that work on any ODF document and \
        decide what it is by reading it.\n\n\
        Each invocation loads the file, applies one command and writes it back. Pass \
        --session to carry undo history across invocations; without it every command starts \
        with empty history."
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Top,

    /// File holding undo history between invocations
    #[arg(long, global = true, value_name = "PATH")]
    pub(crate) session: Option<PathBuf>,

    /// Output format
    #[arg(long, global = true, value_enum, default_value = "text")]
    pub(crate) format: Format,

    /// Apply the command and report the result, but write nothing to disk
    #[arg(long, global = true)]
    pub(crate) dry_run: bool,
}

/// The first level: an application, or a verb that needs none — one that reads the document's kind
/// from the file, or `build`, which has no document to read.
#[derive(Subcommand)]
pub(crate) enum Top {
    /// Spreadsheets — cells, formulas, sheets, number formats
    Sheet {
        #[command(subcommand)]
        command: Command,
    },

    /// Text documents — paragraphs, headings, lists, outlines
    ///
    /// Blocks are addressed by position (p12), by an offset within one (p12+40), by a range
    /// (p12:p20), by a bookmark (#intro) or by outline path (§2.1.3, or s2.1.3). Bookmarks and
    /// outline paths keep pointing at the same place after edits elsewhere in the document; p12
    /// does not.
    Text {
        #[command(subcommand)]
        command: TextCommand,
    },

    /// What a document is, and what is in it
    ///
    /// Works on any ODF document; the kind is read from the file, not guessed from its name.
    Info { file: PathBuf },

    /// Check a document for problems, whatever kind it is
    ///
    /// Runs the spreadsheet or the text document checks, depending on the file. Exits non-zero
    /// when it finds an error.
    Lint {
        file: PathBuf,
        /// Report the house-style hints too, which are off by default
        #[arg(long)]
        hints: bool,
        /// Silence one rule, by id; repeatable
        #[arg(long, value_name = "RULE")]
        off: Vec<String>,
    },

    /// Print any document as its projection, whatever kind it is
    ///
    /// The projection is the document as plain text — what a `.grind` file holds. Useful for `git
    /// diff`: with `*.ods diff=grind` in `.gitattributes` and `git config diff.grind.textconv
    /// "grind project"`, a diff of a changed document shows the cells and blocks that changed.
    Project {
        file: PathBuf,
        /// Print what each stretch of the text is instead: one `kind<TAB>start<TAB>end` line per
        /// token
        #[arg(long, conflicts_with = "anchors")]
        tokens: bool,
        /// Print where each cell or block is instead: one `address<TAB>line<TAB>start<TAB>end`
        /// line per address
        #[arg(long)]
        anchors: bool,
        /// With --anchors, for a .grind file: positions in the file as written
        ///
        /// A .grind somebody wrote has its own comments and alignment, so its lines are not the
        /// ones `project` prints. This gives each address's place in the file itself.
        #[arg(long, requires = "anchors")]
        as_read: bool,
    },

    /// Build a document from a script
    ///
    /// Runs a Rhai script that returns a document, and writes it. The output's extension picks the
    /// file format, as for `convert`; whether it is a spreadsheet or a text document comes from
    /// the script.
    ///
    /// A script cannot touch the filesystem (beyond reading JSON data, see --data), the network,
    /// the clock or randomness, so the same script builds the same file on every machine. One that
    /// runs too long stops with an error and a line number.
    Build {
        /// The script to run
        script: PathBuf,
        /// Where to write the document: .ods/.fods, .odt/.fodt or .grind. A script that returns a
        /// map of names to documents (one letter per recipient, say) writes a series, and this is
        /// then a directory holding `<name>.<ext>` for each — `fodt` or `fods` unless --as says
        /// otherwise
        #[arg(short, long, value_name = "PATH")]
        out: PathBuf,
        /// What to write a series as: fodt, odt, fods, ods, grind — or pdf for text documents, set
        /// in the bundled fonts only so the PDFs come out the same on any machine
        #[arg(long = "as", value_name = "EXT")]
        series_as: Option<String>,
        /// Directory the script may read data from, instead of its own
        ///
        /// A script reads JSON files from inside this directory only: `..`, absolute paths and
        /// symlinks pointing out are refused. By default it is the script's own directory, so
        /// `json("prices.json")` reads the file beside it.
        #[arg(long, value_name = "DIR")]
        data: Option<PathBuf>,
        /// Skip the recalculation at the end
        ///
        /// Each formula is calculated where it is written, so this matters only for a formula that
        /// reads a cell the script fills later: it keeps the value it had then and is reported as
        /// stale.
        #[arg(long)]
        no_recalc: bool,
    },

    /// Run the tests in a script
    ///
    /// Builds the document exactly as `grind build` does, recalculates it, and calls every
    /// function in the script whose name starts with `test_` with the document it built —
    /// `fn test_total(d) { assert_eq(d.cell("B6").value, 15400); }`. Each failure is reported
    /// with the line of the assertion that failed, and the command fails if any test did, or if
    /// the script defines none. Nothing is written.
    Test {
        /// The script to test
        script: PathBuf,
        /// Directory the script may read data from, instead of its own — as for `build`
        #[arg(long, value_name = "DIR")]
        data: Option<PathBuf>,
    },

    /// Print what an editor needs to help you write a script or a projection
    ///
    /// Every function a script can call, with its parameters, types and documentation, as a Rhai
    /// definition file. Save it beside your scripts and an editor that understands Rhai offers
    /// completion and hover:
    ///
    ///   grind definitions > grind.d.rhai
    ///
    /// Rhai's own standard library is left out; editors already know it.
    Definitions {
        /// Print VS Code snippets instead of a Rhai definition file
        ///
        /// The same functions, as snippets VS Code offers without a language server:
        ///
        ///   grind definitions --snippets > .vscode/grind.code-snippets
        #[arg(long)]
        snippets: bool,
        /// Print VS Code snippets for writing a .grind file instead
        ///
        /// Every node of the spreadsheet and text projections, with its arguments as tab stops:
        ///
        ///   grind definitions --projection > .vscode/grind-projection.code-snippets
        ///
        /// Always snippets, so --snippets beside it changes nothing.
        #[arg(long)]
        projection: bool,
    },

    /// Run the language server, for an editor, over stdin and stdout
    ///
    /// For .grind files: problems on the right line, hover over a cell for its formula and what it
    /// shows, the value each formula computes shown beside it (never written into the file), go to
    /// a name's definition, and completion of nodes, functions and names. For .rhai scripts:
    /// completion and hover for every function, and syntax errors as you type; the script is
    /// checked, never run. Point your editor's language client at `grind lsp`.
    Lsp,

    /// Set up a directory for writing scripts and projections in an editor
    ///
    /// Writes a starter — by default a script that builds a spreadsheet (model.rhai, with its
    /// data.json and its tests); --template picks a hand-written spreadsheet or text document
    /// instead — and around it what an editor and git need: VS Code's recommended extensions,
    /// settings and tasks, the vocabulary as snippets and a Rhai definition file, and an
    /// .editorconfig, .gitignore and .gitattributes. In a git repository's root it also points
    /// `diff=grind` at `grind project`, so `git diff` reads an .ods as the cells that changed.
    ///
    /// Nothing that exists is overwritten: a file with other content is kept and the report
    /// says what it is missing, and --force replaces it. Running it twice writes nothing the
    /// second time. No editor is touched — extensions are recommended, never installed — and
    /// nothing written here is read back by grind, so any of it can be deleted.
    ///
    /// The vocabulary files it writes are this grind's, so a newer grind finds them `stale`;
    /// `grind init --update` rewrites those and touches nothing else.
    Init {
        /// The directory to set up, created if missing
        #[arg(default_value = ".")]
        dir: PathBuf,
        /// The starter to write: `generator` by default in a new directory, none in one set up
        /// before
        #[arg(long, value_enum)]
        template: Option<init::Template>,
        /// Rewrite only the generated vocabulary files — snippets and grind.d.rhai — when this
        /// grind's differ; write and touch nothing else
        #[arg(long, conflicts_with = "template")]
        update: bool,
        /// Replace files that exist with other content
        #[arg(long)]
        force: bool,
    },

    /// Print the third-party components grind is built from, and their licences
    ///
    /// Every library compiled into grind's programs, the fonts it ships and the Rust standard
    /// library, each with its licence text — the same list every window's About shows.
    #[command(visible_alias = "licenses")]
    Licences {
        /// One line per component instead: name, version, licence and where it comes from
        #[arg(long)]
        list: bool,
    },

    /// Print your personal dictionary, or add a word to it or remove one
    ///
    /// The words every spelling check accepts, in every document and language, one per line in
    /// $XDG_CONFIG_HOME/grind/words. They are yours and never written into a document; every
    /// window's Personal Dictionary shows the same list.
    #[cfg(feature = "spell")]
    Words {
        /// Add this word
        #[arg(long, conflicts_with = "remove")]
        add: Option<String>,
        /// Remove this word
        #[arg(long)]
        remove: Option<String>,
    },

    /// Convert between file formats — package, flat file and .grind
    ///
    /// The format comes from the output extension: .ods or .odt for the package (a zip), .fods or
    /// .fodt for one flat XML file, .grind for the projection. A spreadsheet stays a spreadsheet
    /// and a text document a text document.
    ///
    /// Converting to a .grind and back gives the same document as far as grind understands it;
    /// anything grind does not understand is left out of the .grind, as it is from any conversion.
    /// The original bytes are kept only when the format does not change.
    Convert { file: PathBuf, out: PathBuf },
}

/// The formats `sheet format` offers: `numfmt::preset`'s vocabulary, plus `general` for no format
/// and `datetime` for a date with a time.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum Style {
    General,
    Number,
    Percent,
    Currency,
    Date,
    Datetime,
    Time,
    Boolean,
    Text,
}

/// `fo:text-align` in the words a person uses; ODF's own values are `start` and `end`.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum Align {
    Left,
    Center,
    Right,
    Justify,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum VAlign {
    Top,
    Middle,
    Bottom,
}

/// `grind text`: deliberately narrower than `grind sheet` (`doc/text-core.md`).
#[derive(Subcommand)]
pub(crate) enum TextCommand {
    /// Convert a Word document into an ODF text document
    ///
    /// Reads `.docx`; grind does not write Word documents. Nothing is fetched and no macro runs.
    /// The result is an ordinary ODF document, which every other command then works on.
    #[cfg(feature = "docx")]
    Import {
        /// The Word document to read
        input: PathBuf,
        /// Where to write the ODF document: .odt for a package, anything else (.fodt by
        /// convention) for one flat XML file
        output: PathBuf,
        /// Overwrite the output if it already exists
        #[arg(long)]
        force: bool,
        /// Fail, and write nothing, if the conversion lost anything at all. The report is
        /// printed either way, so it says what.
        #[arg(long)]
        strict: bool,
    },

    /// Create an empty document
    ///
    /// The file format comes from the extension: .odt writes a zip, .grind the projection, and
    /// anything else — including a name with no extension — one flat XML file that git diff can
    /// read.
    New {
        file: PathBuf,
        /// Overwrite the file if it already exists
        #[arg(long)]
        force: bool,
    },

    /// Print the document as plain text, one block per line
    View {
        file: PathBuf,
        /// Blocks to print, e.g. p2 or p2:p9; defaults to the whole document
        range: Option<String>,
        /// Prefix each line with its address and kind
        #[arg(long)]
        marks: bool,
        /// Show where each bookmark sits, inline
        ///
        /// A bookmark adds no characters, so nothing in the text shows it otherwise. Cannot be
        /// combined with --width.
        #[arg(long, conflicts_with = "width")]
        names: bool,
        /// Wrap at this many characters
        #[arg(long, value_name = "COLUMNS")]
        width: Option<u32>,
    },

    /// Move a caret by lines, or to the ends of its line, and print where it lands
    ///
    /// Down-arrow, Up-arrow, Home and End, from a script. Lines depend on a width, counted here in
    /// characters: `--width 40` means forty.
    Caret {
        file: PathBuf,
        /// Where the caret is now, e.g. p3+12, #intro+5 or §2.1
        at: String,
        /// Move down this many lines (negative moves up)
        #[arg(long, allow_negative_numbers = true, value_name = "LINES")]
        down: Option<i32>,
        /// Move to the start of the caret's own line
        #[arg(long, conflicts_with_all = ["down", "end"])]
        home: bool,
        /// Move to the end of the caret's own line
        #[arg(long, conflicts_with_all = ["down", "home"])]
        end: bool,
        /// Wrap at this many characters; without it the block is one line
        #[arg(long, value_name = "COLUMNS", default_value = "0")]
        width: u32,
    },

    /// Print one block
    Get {
        file: PathBuf,
        /// Block address, e.g. p3, #intro or §2.1
        at: String,
    },

    /// Replace a block's text, keeping its kind and its style
    ///
    /// Bookmarks on the block are kept.
    Set {
        file: PathBuf,
        /// Block address, e.g. p3, #intro or §2.1
        at: String,
        /// The new text; "-" reads it from stdin
        #[arg(allow_hyphen_values = true)]
        text: String,
    },

    /// Insert text at a caret, without disturbing the rest of the block
    ///
    /// What typing does, where `set` replaces a whole block. The text takes the formatting of the
    /// text at the caret.
    ///
    /// With --markdown, the notation is read as it is typed: `**bold**` becomes bold and its
    /// markers go, as do `*italic*`, `__underline__`, `~~struck~~` and `` `code` ``; `# ` at the
    /// front of a block makes it a heading, and ``` fences a code paragraph. One undo step for the
    /// whole text.
    Type {
        file: PathBuf,
        /// Where it goes, e.g. p3+12 — an address with no offset means the front of its block
        at: String,
        /// The text; "-" reads it from stdin
        #[arg(allow_hyphen_values = true)]
        text: String,
        /// Read **bold**, *italic*, __underline__, ~~struck~~, `code`, "# " and ``` as they land
        #[arg(long)]
        markdown: bool,
    },

    /// Insert a table, or print the one at an address
    ///
    /// A cell of a table holds blocks — paragraphs, headings, lists — so every other command works
    /// inside a table unchanged: `p12` is the twelfth block whether it is in a table or not. A new
    /// table has an empty paragraph in every cell, inserted in one undo step.
    ///
    /// Column widths, borders and merged cells cannot be set; ones a document already has are
    /// kept.
    Table {
        file: PathBuf,
        /// Where it goes, e.g. p3 — omit to append to the end of the document
        at: Option<String>,
        /// Print the table at the address instead of inserting one
        #[arg(long, conflicts_with_all = ["rows", "columns", "name", "after"])]
        show: bool,
        /// Insert after the address rather than before it
        #[arg(long)]
        after: bool,
        /// How many rows
        #[arg(long, default_value_t = 2)]
        rows: u32,
        /// How many columns
        #[arg(long, default_value_t = 2)]
        columns: u32,
        /// The table's name, which is its identity; generated when omitted
        #[arg(long)]
        name: Option<String>,
    },

    /// Import Markdown (CommonMark) into the document
    ///
    /// Headings, paragraphs, nested lists, bold, italic, strikethrough, `code`, links, fenced
    /// code, pipe tables and pictures become what they are in the document. A picture is embedded
    /// from a `data:` URI or a relative path beside the Markdown file — never an absolute path or
    /// a URL — and becomes its alt text when it cannot be found. Rules and raw HTML are dropped,
    /// and an ordered list becomes a bulleted one. One undo step. `-` reads standard input, whose
    /// pictures can only be `data:` URIs.
    ImportMd {
        file: PathBuf,
        /// The markdown file, or - for stdin
        source: String,
        /// Insert before this block, e.g. p3 — omit to append to the end
        at: Option<String>,
        /// Insert after the address rather than before it
        #[arg(long)]
        after: bool,
    },

    /// Export the document, or a range of it, as Markdown (CommonMark)
    ///
    /// Underline, colour, size, bookmarks and picture sizes have no Markdown spelling and are left
    /// out. A table's first row becomes its header. Prints to stdout unless --out names a file.
    ///
    /// Each picture is written as a file beside the Markdown — for `notes.md`, in `notes.images/`,
    /// or in --images-dir — named for its content, so an unchanged document exports to unchanged
    /// files. --inline-images puts them in the Markdown as `data:` URIs instead, which is what
    /// stdout gets unless --images-dir is given.
    ExportMd {
        file: PathBuf,
        /// A block range, e.g. p3:p9 or §2 for a whole section — omit for everything
        range: Option<String>,
        /// Write to this file instead of stdout
        #[arg(long)]
        out: Option<PathBuf>,
        /// Where the pictures go, relative to the Markdown file, e.g. pictures
        #[arg(long, value_name = "DIR", conflicts_with = "inline_images")]
        images_dir: Option<String>,
        /// Put the pictures in the markdown as data: URIs rather than files beside it
        #[arg(long)]
        inline_images: bool,
    },

    /// Give a picture its alternative text — what a screen reader says and markdown's `![alt]`
    ///
    /// The address is the picture's own position, e.g. p4+0 for a picture at the front of p4.
    /// `--description` is the longer text (`svg:desc`); both are replaced, so a picture given
    /// no `--description` has none, and an empty title removes the short one.
    Alt {
        file: PathBuf,
        /// The picture's position, e.g. p4+0
        at: String,
        /// The short text (`svg:title`)
        title: String,
        /// The long text (`svg:desc`)
        #[arg(long)]
        description: Option<String>,
    },

    /// Export the document as a PDF, ready to print
    ///
    /// Laid out on the page the document states — or on `--paper` — in the fonts it names, or
    /// their metric-compatible twins, or the bundled Liberation faces; the report says which.
    /// Paper is ISO 216 only: `a4`, `a5`, `a3-landscape`, …. The same document always writes
    /// the same bytes.
    #[cfg(feature = "pdf")]
    ExportPdf {
        file: PathBuf,
        /// Where to write the PDF
        out: PathBuf,
        /// Print on this paper instead of the document's own page, e.g. a4 or a5-landscape
        #[arg(long, value_parser = paper)]
        paper: Option<grind_core::page::PageGeometry>,
        /// The PDF's title — the file name without its extension when not given
        #[arg(long)]
        title: Option<String>,
        /// Use only the fonts built into grind, never the machine's: the same PDF anywhere
        #[arg(long)]
        bundled_fonts: bool,
    },

    /// List the pages the document prints on: where each begins and ends
    ///
    /// One line per page — its number, the first character on it and one past the last, as
    /// addresses `grind text get` takes. Typeset exactly as `export-pdf` typesets, so these are
    /// the PDF's own page breaks.
    #[cfg(feature = "pdf")]
    Pages {
        file: PathBuf,
        /// Lay out on this paper instead of the document's own page, e.g. a4 or a5-landscape
        #[arg(long, value_parser = paper)]
        paper: Option<grind_core::page::PageGeometry>,
        /// Use only the fonts built into grind, never the machine's: the same PDF anywhere
        #[arg(long)]
        bundled_fonts: bool,
    },

    /// Draw one page as a PNG — the print preview, exactly as the PDF prints it
    #[cfg(feature = "pdf")]
    Preview {
        file: PathBuf,
        /// Where to write the PNG
        out: PathBuf,
        /// Which page, from 1
        #[arg(long, default_value_t = 1)]
        page: usize,
        /// Resolution in dots per inch; 96 is a screen at 100%
        #[arg(long, default_value_t = 96.0)]
        dpi: f32,
        /// Lay out on this paper instead of the document's own page, e.g. a4 or a5-landscape
        #[arg(long, value_parser = paper)]
        paper: Option<grind_core::page::PageGeometry>,
        /// Use only the fonts built into grind, never the machine's: the same PDF anywhere
        #[arg(long)]
        bundled_fonts: bool,
    },

    /// Insert an image at a caret, from a file on disk
    ///
    /// The image counts as one character position. The MIME type is guessed from the file's
    /// extension unless --mime gives it; --width and --height are lengths such as `5cm`, and both
    /// are optional.
    Image {
        file: PathBuf,
        /// Where it goes, e.g. p3+12 — an address with no offset means the front of its block
        at: String,
        /// The picture to embed
        #[arg(long, value_name = "PATH")]
        from: PathBuf,
        /// Override the guessed MIME type, e.g. image/jpeg
        #[arg(long, value_name = "TYPE")]
        mime: Option<String>,
        /// Width, e.g. 5cm
        #[arg(long)]
        width: Option<String>,
        /// Height, e.g. 5cm
        #[arg(long)]
        height: Option<String>,
    },

    /// Erase a range of characters, leaving the blocks that hold them
    ///
    /// `delete` removes blocks; this removes text. A range that crosses a block boundary
    /// closes the boundary up, leaving one block that keeps the first one's kind and style —
    /// one undo step, however many blocks it spanned.
    Erase {
        file: PathBuf,
        /// Characters, e.g. p3+12:p3+20 — a bare address is that block's whole text
        range: String,
    },

    /// Split a block in two at a caret
    ///
    /// The Return key. The second half keeps the first's kind and style, except that a heading
    /// split at its very end leaves a body paragraph behind it.
    Split {
        file: PathBuf,
        /// Where to cut, e.g. p3+12
        at: String,
    },

    /// Join a block with the one after it
    ///
    /// The Backspace key, seen from the block above it. The first block's kind and style are
    /// the ones that survive.
    Join {
        file: PathBuf,
        /// The first of the two, e.g. p3
        at: String,
    },

    /// Print the document as its projection — the plain-text form
    ///
    /// The same text a `.grind` file holds, and what each window's source view shows. Every
    /// command here also reads a `.grind` file directly.
    Project {
        file: PathBuf,
        /// Print what each stretch of the text is instead: one `kind<TAB>start<TAB>end` line per
        /// token
        #[arg(long, conflicts_with = "anchors")]
        tokens: bool,
        /// Print where each block is instead: one `address<TAB>line<TAB>start<TAB>end` line per
        /// address
        ///
        /// A block appears once for every way it can be addressed, so one line may be several
        /// rows: p12, #bookmark and §2.1.3 can all name the same paragraph.
        #[arg(long)]
        anchors: bool,
        /// With --anchors, for a .grind file: positions in the file as written
        ///
        /// A .grind somebody wrote has its own comments and alignment, so its lines are not the
        /// ones `project` prints. This gives each address's place in the file itself.
        #[arg(long, requires = "anchors")]
        as_read: bool,
    },

    /// Insert a block before an address, or at the end
    Insert {
        file: PathBuf,
        /// Where it goes; omit to append to the end of the document
        at: Option<String>,
        /// The text; "-" reads it from stdin
        #[arg(long, allow_hyphen_values = true, default_value = "")]
        text: String,
        /// Insert after the address rather than before it
        #[arg(long)]
        after: bool,
        /// Make it a heading at this outline level
        #[arg(long, value_name = "LEVEL", conflicts_with = "list")]
        heading: Option<u32>,
        /// Make it a list item at this nesting depth
        #[arg(long, value_name = "DEPTH", num_args = 0..=1, default_missing_value = "1")]
        list: Option<u32>,
    },

    /// Delete a block or a range of them
    Delete {
        file: PathBuf,
        /// Blocks to delete, e.g. p3 or p3:p7
        range: String,
    },

    /// Move a range of blocks so that it starts at an address
    ///
    /// With outline addresses this moves whole sections: `grind text move report.fodt §3.2 §1`
    /// moves section 3.2, however long, to before section 1.
    Move {
        file: PathBuf,
        /// Blocks to move, e.g. p3:p7 — or a heading address, which moves its whole section
        range: String,
        /// Where they land, e.g. p1
        to: String,
    },

    /// Set or clear the named paragraph style of a range of blocks
    Style {
        file: PathBuf,
        /// Blocks, e.g. p3 or p3:p7
        range: String,
        /// The style name, e.g. Heading_20_1; omit to clear
        #[arg(long)]
        style: Option<String>,
    },

    /// Change what kind of block this is — paragraph, heading or list item
    Kind {
        file: PathBuf,
        /// Block address
        at: String,
        /// Outline level for a heading
        #[arg(long, value_name = "LEVEL", conflicts_with = "list")]
        heading: Option<u32>,
        /// Nesting depth for a list item
        #[arg(long, value_name = "DEPTH", num_args = 0..=1, default_missing_value = "1")]
        list: Option<u32>,
    },

    /// Print every heading, its level and the address that finds it again
    Outline {
        file: PathBuf,
        /// Only headings whose text or address contains this
        #[arg(long)]
        filter: Option<String>,
    },

    /// Set how a span of characters looks — bold, italic, a font, a size
    ///
    /// Direct formatting, as a toolbar's B and I buttons apply it. It replaces rather than adds,
    /// so `grind text format p3` with no options makes that paragraph plain again; use --show
    /// first to read the current formatting. Named character styles are left alone (see `grind
    /// text style`).
    Format {
        file: PathBuf,
        /// Characters, e.g. p3+12:p3+20 — or a bare address for a whole block
        range: String,
        /// Print the formatting of the range instead of setting any
        #[arg(long, conflicts_with_all = [
            "bold", "italic", "underline", "strike", "font", "size", "color", "background",
        ])]
        show: bool,
        #[arg(long)]
        bold: bool,
        #[arg(long)]
        italic: bool,
        #[arg(long)]
        underline: bool,
        /// Strike the text through
        #[arg(long)]
        strike: bool,
        /// Font family, e.g. Georgia
        #[arg(long, value_name = "FAMILY")]
        font: Option<String>,
        /// Font size, e.g. 14pt
        #[arg(long)]
        size: Option<String>,
        /// Text colour: a palette name (navy, red, silver, …) or #rrggbb
        #[arg(long, value_parser = style::color)]
        color: Option<String>,
        /// Highlight behind the text: a palette name, #rrggbb, or "transparent"
        #[arg(long, value_parser = style::color)]
        background: Option<String>,
    },

    /// Link text to a URL or a bookmark, unlink it, or show the link that is there
    ///
    /// With a target, every character in the range links to it — formatting untouched, one
    /// undo step. `--remove` unlinks; at a bare caret (`p3+5`) it unlinks the whole link the
    /// caret touches. With neither, prints the link as `<range>\t<target>` — at a caret the
    /// whole link around it, over a range the one target every character agrees on.
    Link {
        file: PathBuf,
        /// Characters, e.g. p3+12:p3+20, a bare address for a whole block, or a caret
        range: String,
        /// A URL, or #name for a bookmark in this document
        #[arg(conflicts_with = "remove")]
        target: Option<String>,
        /// Unlink the range, or the link at a caret
        #[arg(long)]
        remove: bool,
    },

    /// Check the document for problems
    ///
    /// A skipped heading level, a link to a bookmark that does not exist, a style name the
    /// document never declares, and anything a `.grind` of it would leave out. Nothing is written.
    /// Exits non-zero when it finds an error.
    Lint {
        file: PathBuf,
        /// Report the house-style hints too, which are off by default
        #[arg(long)]
        hints: bool,
        /// Silence one rule, by id; repeatable
        #[arg(long, value_name = "RULE")]
        off: Vec<String>,
        /// List the rules instead of running them
        #[arg(long)]
        rules: bool,
    },

    /// Print every block carrying a style of its own — why one paragraph looks different from the
    /// others
    Formatting { file: PathBuf },

    /// Print every occurrence of some text, with the address of each
    Find {
        file: PathBuf,
        #[arg(allow_hyphen_values = true)]
        needle: String,
        /// Match `Appendix` when asked for `appendix` — what a find bar does by default
        #[arg(short, long)]
        ignore_case: bool,
    },

    /// Replace every occurrence of some text
    Replace {
        file: PathBuf,
        #[arg(allow_hyphen_values = true)]
        needle: String,
        #[arg(allow_hyphen_values = true)]
        replacement: String,
    },

    /// Print every misspelt word, with its address
    ///
    /// A word any of the dictionaries knows is accepted, unless --language names one, which is
    /// then the only one checked. Suggestions come from the document's own language — the one it
    /// states, else the one whose dictionary knows most of its words — which is printed to stderr.
    /// Words in your personal dictionary ($XDG_CONFIG_HOME/grind/words, one per line) are
    /// accepted; --add puts one there. Nothing is written to the document.
    #[cfg(feature = "spell")]
    Spell {
        #[arg(required_unless_present = "add")]
        file: Option<PathBuf>,
        /// Blocks to check, e.g. p3 or p3:p9 — the whole document when left out
        range: Option<String>,
        /// The language to check in: en, de, fr, es, it, pt or pl, or any tag of one (de-AT,
        /// en_GB.UTF-8)
        #[arg(long, value_name = "LANG")]
        language: Option<String>,
        /// Print what each word might have been, best first
        #[arg(long)]
        suggest: bool,
        /// Add a word to your own list instead of checking anything
        #[arg(long, value_name = "WORD", conflicts_with_all = ["range", "language", "suggest"])]
        add: Option<String>,
    },

    /// Print what the grammar checker finds, one finding a line
    ///
    /// Each line is where the finding starts, its rule, its kind (grammar, punctuation,
    /// typography or style), the text it covers, what is wrong, and what it might be instead.
    /// `grind text correct` takes the address, the covered text and a replacement. Each
    /// paragraph is checked in the language the document states, else the one its common words
    /// suggest. Nothing is written to the document.
    #[cfg(feature = "grammar")]
    Grammar {
        file: PathBuf,
        /// Blocks to check, e.g. p3 or p3:p9 — the whole document when left out
        range: Option<String>,
        /// Add the style suggestions, which are off by default
        #[arg(long)]
        picky: bool,
    },

    /// Print or set whether text documents are spell-checked, and in what
    ///
    /// `off` turns spelling off in every text window and for `grind lint`, and keeps it off
    /// until `auto` (the default: the document's own language, else a guess) or a language
    /// turns it back on. Remembered in $XDG_CONFIG_HOME/grind/spelling — yours, not any
    /// document's — beside the spreadsheets' own setting (`grind sheet spelling`). With no
    /// setting, prints the remembered one.
    #[cfg(feature = "spell")]
    Spelling {
        /// auto, off, or a language: en, de, fr, es, it, pt or pl
        setting: Option<String>,
    },

    /// Replace one misspelt word — what picking a suggestion does
    ///
    /// The word is named as well as placed, so a correction aimed at text that has since
    /// changed is refused rather than overwriting whatever is there now. Keeps the word's
    /// formatting; one undo step.
    Correct {
        file: PathBuf,
        /// Where the word starts, e.g. p3+12 — as `grind text spell` prints it
        at: String,
        /// The word that is there now
        word: String,
        /// What it should be
        with: String,
    },

    /// Put a bookmark on a block, list them, or delete one
    ///
    /// A bookmark moves with the text, so #intro keeps meaning the same place after an edit above
    /// it.
    Name {
        file: PathBuf,
        /// The bookmark name; omit to list them all
        name: Option<String>,
        /// Where it goes
        at: Option<String>,
        /// Remove it instead
        #[arg(long)]
        delete: bool,
    },

    /// Count blocks, headings, words and characters
    Words { file: PathBuf },
}

#[derive(Subcommand)]
pub(crate) enum Command {
    /// Convert an Excel workbook into an ODF spreadsheet
    ///
    /// Reads `.xlsx` and `.xlsm`; grind does not write Excel files. Nothing is calculated: Excel's
    /// cached values are kept as they are, and `grind sheet recalc` recalculates when you want it
    /// to. The result is an ordinary ODF document, which every other command then works on.
    #[cfg(feature = "xlsx")]
    Import {
        /// The workbook to read
        input: PathBuf,
        /// Where to write the ODF document: .ods for a package, anything else (.fods by
        /// convention) for one flat XML file
        output: PathBuf,
        /// Overwrite the output if it already exists
        #[arg(long)]
        force: bool,
        /// Fail, and write nothing, if the conversion lost anything at all — a construct, a
        /// formula, a piece of a format or a look, a defined name, a sheet's spelling. The
        /// report is printed either way, so it says what.
        #[arg(long)]
        strict: bool,
    },

    /// Create an empty document
    ///
    /// The file format comes from the extension: .ods writes a zip, .grind the projection, and
    /// anything else — including a name with no extension — one flat XML file that git diff can
    /// read.
    New {
        file: PathBuf,
        /// Overwrite the file if it already exists
        #[arg(long)]
        force: bool,
        /// The document's own locale, e.g. de-DE — how it spells its numbers (`sheet locale`).
        /// A new document states none unless told
        #[arg(long, value_parser = locale)]
        locale: Option<grind_sheet::locale::Locale>,
    },

    /// Print or set the document's own locale — how it spells its numbers
    ///
    /// `sheet locale book.fods de-DE` makes every number with no format, and every format that
    /// names no locale of its own, read `1.234,5`; a number typed afterwards is read the same
    /// way (`1,5` is one and a half). A format that names its own locale keeps it. `--clear`
    /// takes the document's away again, and with neither it prints the one it has, or `none`,
    /// and beside it a number spelled that way. A number typed in the other spelling where this
    /// one cannot be meant — `1.5` in a German document — is read that way, and `set` says so.
    Locale {
        file: PathBuf,
        /// A language, and optionally a country: de-DE, fr, pt-BR
        #[arg(value_parser = locale)]
        tag: Option<grind_sheet::locale::Locale>,
        /// State no locale at all
        #[arg(long, conflicts_with = "tag")]
        clear: bool,
    },

    /// Print a cell or a range
    ///
    /// Prints what the cell displays — its number format applied, so a date prints as a
    /// date. `--raw` prints the stored value instead.
    Get {
        file: PathBuf,
        /// Cell address, e.g. A1 or Data.B2
        address: String,
        /// Print the formula source instead of the value
        #[arg(long)]
        formula: bool,
        /// Print what an editor would show: the text that, set again, changes nothing
        #[arg(long, conflicts_with = "formula")]
        input: bool,
        /// Print a formula's calculated, formatted result rather than its source
        #[arg(long, conflicts_with_all = ["formula", "input"])]
        value: bool,
        /// Print the stored value rather than the formatted display text
        #[arg(long)]
        raw: bool,
    },

    /// Print a rectangle of values, tab-separated
    ///
    /// --roles, --names and --formulas print something else about each cell instead: what kind of
    /// cell it is, which name refers to it, and the formula that computes it. None of them changes
    /// the file. `--format json` carries every column at once.
    View {
        file: PathBuf,
        /// Range to print; defaults to everything the sheet uses
        address: Option<String>,
        /// Stop after this many rows
        #[arg(long, default_value_t = 40)]
        max_rows: u32,
        /// Print stored values rather than formatted display text
        #[arg(long)]
        raw: bool,
        /// Print each cell's role: input-named, constant-unnamed, computed-local, …
        #[arg(long, conflicts_with_all = ["raw", "names", "formulas"])]
        roles: bool,
        /// Print the named expression bound to each cell, where one is
        #[arg(long, conflicts_with_all = ["raw", "roles"])]
        names: bool,
        /// Print each cell's formula source rather than its value
        ///
        /// With --names as well, the formula is printed the way a formula bar reads it:
        /// display form with every reference that a name stands for spelled as that name.
        #[arg(long, conflicts_with_all = ["raw", "roles"])]
        formulas: bool,
    },

    /// Print the document as its projection — the plain-text form
    ///
    /// The same text a `.grind` file holds, and what each window's source view shows. Every
    /// command here also reads a `.grind` file directly.
    Project {
        file: PathBuf,
        /// Print what each stretch of the text is instead: one `kind<TAB>start<TAB>end` line per
        /// token
        ///
        /// What an editor would colour the text by.
        #[arg(long, conflicts_with = "anchors")]
        tokens: bool,
        /// Print where each cell is instead: one `address<TAB>line<TAB>start<TAB>end` line per
        /// address
        #[arg(long)]
        anchors: bool,
        /// With --anchors, for a .grind file: positions in the file as written
        ///
        /// A .grind somebody wrote has its own comments and alignment, so its lines are not the
        /// ones `project` prints. This gives each address's place in the file itself.
        #[arg(long, requires = "anchors")]
        as_read: bool,
    },

    /// Check the document for problems
    ///
    /// A cached value that disagrees with its formula, a formula naming a sheet that is gone or
    /// reading a cell that is empty, and anything a `.grind` of the document would leave out, each
    /// with its address. Nothing is written. Exits non-zero when it finds an error; warnings and
    /// hints do not.
    Lint {
        file: PathBuf,
        /// Report the house-style hints too, which are off by default
        #[arg(long)]
        hints: bool,
        /// Silence one rule, by id; repeatable
        #[arg(long, value_name = "RULE")]
        off: Vec<String>,
        /// List the rules instead of running them
        #[arg(long)]
        rules: bool,
    },

    /// Set a cell's value or formula
    ///
    /// A leading '=' makes it a formula and a leading apostrophe forces text; otherwise the
    /// value is taken as a number, TRUE or FALSE, or text. "-" reads the value from stdin.
    Set {
        file: PathBuf,
        /// Cell address, e.g. A1 or Data.B2
        address: String,
        /// The value; hyphen-leading values are taken literally, so -1 is a number
        #[arg(allow_hyphen_values = true)]
        value: String,
        /// Store the value as text, whatever it looks like
        #[arg(long)]
        text: bool,
        /// Recalculate the document in the same undo step, unless that would spoil a cell
        #[arg(long)]
        recalc: bool,
    },

    /// Fill a rectangle from tab-separated rows
    ///
    /// Each cell is read the way `set` reads its value. "-" reads the rows from stdin.
    Paste {
        file: PathBuf,
        /// Where the first cell goes, e.g. B2
        anchor: String,
        /// Tab-separated rows, one per line; "-" reads them from stdin
        #[arg(allow_hyphen_values = true)]
        rows: String,
        /// Recalculate the document in the same undo step, unless that would spoil a cell
        #[arg(long)]
        recalc: bool,
    },

    /// Read a CSV or TSV file into the sheet
    ///
    /// The delimiter is sniffed from the file unless you name one, so a comma file, a
    /// semicolon file and a tab file all just open. Quoted fields, doubled quotes, embedded
    /// newlines, CRLF and a byte-order mark are all read; ragged rows land where they fall.
    ///
    /// Each field arrives through the same typing rule as `set`, with four exceptions that
    /// keep a file's own data: `007` stays text rather than becoming 7, `NaN` and `inf` stay
    /// text, a leading `=` stays text unless you pass --formulas, and numbers are read in
    /// --locale's convention when you give one.
    ///
    /// --dates reads ISO dates and times (2026-03-15, 10:30) as dates and times, and formats
    /// the cells they land in. Only ISO: 15/03/2026 and 03/15/2026 are the same characters
    /// meaning two different days, and guessing which is how a date column gets corrupted.
    ImportCsv {
        file: PathBuf,
        /// The CSV file to read; "-" reads it from standard input
        source: String,
        /// Where the first field goes, e.g. B2 or Data.A1
        #[arg(long, default_value = "A1")]
        at: String,
        /// Field separator: comma, semicolon, tab, pipe, or a single character. Sniffed from
        /// the file when not given
        #[arg(long, value_parser = delimiter)]
        delimiter: Option<char>,
        /// The character that quotes a field
        #[arg(long, default_value_t = '"')]
        quote: char,
        /// Read numbers the way this locale writes them: 1.234,50 is a number in de-DE
        #[arg(long, value_parser = locale)]
        locale: Option<grind_sheet::locale::Locale>,
        /// Store every field as text, guessing nothing
        #[arg(long)]
        text: bool,
        /// Let a field starting with = become a formula rather than text
        #[arg(long)]
        formulas: bool,
        /// Read ISO dates and times as dates and times, formatting the cells they land in
        #[arg(long)]
        dates: bool,
        /// Drop leading and trailing whitespace from every field
        #[arg(long)]
        trim: bool,
        /// Recalculate the document in the same undo step, unless that would spoil a cell
        #[arg(long)]
        recalc: bool,
    },

    /// Write a range out as CSV or TSV
    ///
    /// What each cell *shows*, so a date leaves as a date and a percentage as a percentage;
    /// --formulas writes the formula instead, in the spelling a formula bar shows. With no
    /// range, everything the sheet uses. Writes to standard output unless --out names a file.
    ///
    /// A field is quoted only when leaving it bare would change what a reader sees — it holds
    /// the delimiter, a quote, a line break, or padding whitespace.
    ExportCsv {
        file: PathBuf,
        /// Cell range, e.g. A1:D20; everything the sheet uses when left out
        address: Option<String>,
        /// Write to this file instead of standard output
        #[arg(long, short)]
        out: Option<PathBuf>,
        /// Field separator: comma, semicolon, tab, pipe, or a single character
        #[arg(long, value_parser = delimiter, default_value = "comma")]
        delimiter: char,
        /// The character that quotes a field
        #[arg(long, default_value_t = '"')]
        quote: char,
        /// Quote every field, not only the ones that need it
        #[arg(long)]
        quote_all: bool,
        /// End every record with CRLF, as RFC 4180 asks
        #[arg(long)]
        crlf: bool,
        /// Start the file with a UTF-8 byte-order mark, which is what Excel looks for
        #[arg(long)]
        bom: bool,
        /// Write formulas where a cell has one, instead of what they computed
        #[arg(long)]
        formulas: bool,
    },

    /// Replicate one cell across a rectangle — extend a calculation the way a drag handle
    /// or Ctrl+D/Ctrl+R does
    ///
    /// A formula's relative references shift by each target's offset from `source`; its
    /// absolute ones (`$`) do not move. A plain value is copied as is.
    Fill {
        file: PathBuf,
        /// The cell whose content is replicated, e.g. A1
        source: String,
        /// Where it lands, e.g. A2:A10 or B1:D1
        address: String,
        /// Recalculate the document in the same undo step, unless that would spoil a cell
        #[arg(long)]
        recalc: bool,
    },

    /// Evaluate a formula against the document without storing anything
    ///
    /// The address is where the formula would sit, which is what its relative references
    /// are relative to.
    Eval {
        file: PathBuf,
        /// Cell address the formula would live at, e.g. B5
        address: String,
        /// The formula, e.g. '=SUM([.B2:.B4])'
        #[arg(allow_hyphen_values = true)]
        formula: String,
        /// Take the formula apart: every sub-expression on its own line, indented by depth,
        /// with what it comes to in place ("not evaluated" for a branch not taken)
        #[arg(long)]
        trace: bool,
        /// What if a cell held something else: ADDRESS=VALUE, read as typing VALUE into that
        /// cell would read it, for this evaluation only (repeatable; an empty VALUE is a blank)
        #[arg(
            long = "with",
            value_name = "ADDRESS=VALUE",
            allow_hyphen_values = true
        )]
        with: Vec<String>,
        /// Lay the formula out over several lines, nesting indented, in display syntax; with
        /// --trace, each line's answer beside it. Read-only: nothing stored is laid out
        #[arg(long)]
        pretty: bool,
        /// The width --pretty lays out to, in characters
        #[arg(long, default_value_t = 60, requires = "pretty")]
        width: usize,
    },

    /// Clear a cell or a range, leaving it empty
    Clear {
        file: PathBuf,
        /// Cell address or range, e.g. A1 or B2:D40
        address: String,
        /// Remove only the formula, keeping the value it last computed (one cell)
        #[arg(long)]
        formula_only: bool,
    },

    /// Set how a cell or range is displayed
    ///
    /// The value never changes — a number format is display only. `general` removes the
    /// format, leaving the plain spelling of the value.
    Format {
        file: PathBuf,
        /// Cell address or range, e.g. B2:B40 or Data.C:C
        address: String,
        /// How to display it
        #[arg(value_enum, required_unless_present = "show", conflicts_with = "show")]
        style: Option<Style>,
        /// Digits after the decimal point, for number, percent and currency
        #[arg(long, default_value_t = 2)]
        decimals: u8,
        /// Group thousands, e.g. 1,234,567
        #[arg(long)]
        grouping: bool,
        /// Currency symbol — written before the amount for $ and £, after it for € and the rest
        #[arg(long, default_value = grind_sheet::numfmt::DEFAULT_CURRENCY)]
        symbol: String,
        /// Locale for the decimal and grouping characters, e.g. de-DE. Defaults to the
        /// document's own (`sheet locale`), then $GRIND_LOCALE, then
        /// $XDG_CONFIG_HOME/grind/locale, then none.
        #[arg(long, value_parser = locale)]
        locale: Option<grind_sheet::locale::Locale>,
        /// Print the format of one cell instead of setting one
        #[arg(long, conflicts_with_all = ["decimals", "grouping", "symbol", "locale"])]
        show: bool,
    },

    /// Set how a cell or range looks
    ///
    /// Replaces the cell's styling rather than adding to it, so `sheet style A1` with no options
    /// makes A1 plain again; use --show first to read the current styling. The font family cannot
    /// be set.
    Style {
        file: PathBuf,
        /// Cell address or range, e.g. A1:D1 or Data.B:B
        address: String,
        /// Print the styling of one cell instead of setting any
        #[arg(long, conflicts_with_all = [
            "bold", "italic", "underline", "strike", "color", "background", "align", "valign",
            "wrap", "size", "border", "indent", "rotate",
        ])]
        show: bool,
        #[arg(long)]
        bold: bool,
        #[arg(long)]
        italic: bool,
        /// Underline the text
        #[arg(long)]
        underline: bool,
        /// Strike the text through
        #[arg(long)]
        strike: bool,
        /// Text colour: a palette name (navy, red, silver, …) or #rrggbb
        #[arg(long, value_parser = style::color)]
        color: Option<String>,
        /// Cell background: a palette name, #rrggbb, or "transparent"
        #[arg(long, value_parser = style::color)]
        background: Option<String>,
        /// Horizontal alignment
        #[arg(long, value_enum)]
        align: Option<Align>,
        /// Vertical alignment
        #[arg(long, value_enum)]
        valign: Option<VAlign>,
        /// Wrap text at the cell edge
        #[arg(long)]
        wrap: bool,
        /// Font size, e.g. 14pt
        #[arg(long)]
        size: Option<String>,
        /// Border on every edge, as width, line and colour: "0.5pt solid navy"
        #[arg(long, value_parser = style::border)]
        border: Option<String>,
        /// Indent the text from the cell's left edge: an ODF length, e.g. 0.5cm
        #[arg(long)]
        indent: Option<String>,
        /// Turn the text anticlockwise by this many degrees, e.g. 45 or 90
        #[arg(long)]
        rotate: Option<String>,
    },

    /// Set the width of a column or a run of columns
    ///
    /// With no length, prints the width of every column in the range that has one of its own —
    /// nothing when all are at the default.
    Width {
        file: PathBuf,
        /// A column or a run of them: B, B:D, or Data.B:D
        columns: String,
        /// An ODF length: 2.5cm, 64pt, 0.9in. Omit to print.
        length: Option<String>,
        /// Back to the default width
        #[arg(long, conflicts_with = "length")]
        clear: bool,
    },

    /// Set the height of a row or a run of rows
    ///
    /// Like `width`, addressed by row number: 3, 3:7, or Data.3:7.
    Height {
        file: PathBuf,
        /// A row or a run of them: 3, 3:7, or Data.3:7
        rows: String,
        /// An ODF length: 0.5cm, 14pt. Omit to print.
        length: Option<String>,
        /// Back to the default height
        #[arg(long, conflicts_with = "length")]
        clear: bool,
    },

    /// Fit columns to their widest text and give rows back to their content
    ///
    /// `sheet fit book.ods B:D` makes each column as wide as the widest thing it shows; `sheet fit
    /// book.ods 3:7` removes those rows' fixed heights, so each grows to hold what is in it. With
    /// no columns or rows named, fits every sheet whole — every column in use and every row — like
    /// the windows' Fit Content to Cells. One undo step per sheet. Text is measured in the
    /// Liberation fonts grind ships, which match the widths of the usual spreadsheet fonts.
    Fit {
        file: PathBuf,
        /// A column or a run of them (B, B:D, Data.B:D), or a row or a run (3, 3:7, Data.3:7)
        tracks: Option<String>,
    },

    /// Hide — or with --unhide, show — a column or a row, or a run of them
    ///
    /// `sheet hide book.ods B:D` hides columns B through D; `sheet hide book.ods 3:7` hides rows 3
    /// through 7. With nothing named, prints every column and row hidden by hand, on every sheet.
    Hide {
        file: PathBuf,
        /// A column or a run of them (B, B:D, Data.B:D), or a row or a run (3, 3:7, Data.3:7)
        tracks: Option<String>,
        /// Show it again instead of hiding it
        #[arg(long)]
        unhide: bool,
    },

    /// Merge a range into one cell — or with `--unmerge`, take every merge in it away
    ///
    /// `sheet merge book.ods B2:D2` draws B2's value across B2 to D2; whatever C2 and D2 held
    /// stays, out of sight, and comes back on unmerge. A merge it overlaps is replaced. With no
    /// range, prints every merged range, across every sheet.
    Merge {
        file: PathBuf,
        /// A range: B2:D2, or Data.A4:A6
        range: Option<String>,
        /// Take away every merge the range overlaps instead
        #[arg(long)]
        unmerge: bool,
    },

    /// Put a checkbox in a cell, linked to the cell whose TRUE or FALSE it shows and sets —
    /// or tick it, or take it away
    ///
    /// `sheet checkbox book.ods B5 --link E5` draws a checkbox in B5 that is ticked while E5 is
    /// TRUE; `--toggle` ticks or unticks it, which writes E5. With no cell, prints every
    /// checkbox, across every sheet.
    Checkbox {
        file: PathBuf,
        /// The cell it is drawn in: B5, or Data.B5
        cell: Option<String>,
        /// The cell it is linked to: E5, or Data.E5. Without one, it holds its own state
        #[arg(long, conflicts_with_all = ["remove", "toggle"])]
        link: Option<String>,
        /// The text drawn beside the box
        #[arg(long, conflicts_with_all = ["remove", "toggle"])]
        label: Option<String>,
        /// Tick it if it is not ticked, untick it if it is
        #[arg(long, conflicts_with = "remove")]
        toggle: bool,
        /// Take it away
        #[arg(long)]
        remove: bool,
    },

    /// Draw a range differently while a formula is true — a conditional-format rule — or list
    /// or take away a sheet's rules
    ///
    /// `sheet rule book.ods B5:D9 '[.$E5]=TRUE()' --background green` fills B5:D9's rows green
    /// wherever column E is TRUE: the condition is ODF syntax, written from the range's top-left
    /// cell, and moved to every other cell the way a fill moves a formula. A new rule goes
    /// after every rule the sheet has, and the first that holds at a cell is the one drawn.
    /// `--remove 2` takes the sheet's second rule away; `--clear` every rule that touches the
    /// range. With no condition, prints every rule, across every sheet, numbered per sheet.
    Rule {
        file: PathBuf,
        /// A range: B5:D9, or Data.B5:D9 — or with `--remove`, a sheet name
        range: Option<String>,
        /// The condition, in ODF syntax: '[.$E5]=TRUE()' (a leading = is allowed)
        condition: Option<String>,
        /// Take the rule with this number (as listed) off the range's sheet
        #[arg(long, conflicts_with_all = ["condition", "clear"])]
        remove: Option<usize>,
        /// Take away every rule on the range's sheet that touches the range
        #[arg(long, conflicts_with = "condition")]
        clear: bool,
        #[arg(long)]
        bold: bool,
        #[arg(long)]
        italic: bool,
        /// Underline the text
        #[arg(long)]
        underline: bool,
        /// Strike the text through
        #[arg(long)]
        strike: bool,
        /// Text colour: a palette name (navy, red, silver, …) or #rrggbb
        #[arg(long, value_parser = style::color)]
        color: Option<String>,
        /// Cell background: a palette name, #rrggbb, or "transparent"
        #[arg(long, value_parser = style::color)]
        background: Option<String>,
        /// Font size, e.g. 14pt
        #[arg(long)]
        size: Option<String>,
        /// Border on every edge, as width, line and colour: "0.5pt solid navy"
        #[arg(long, value_parser = style::border)]
        border: Option<String>,
    },

    /// Define, redefine, rename, inline or delete a named range or expression
    ///
    /// With no target, prints what the name stands for. `sheet info` lists them all.
    Name {
        file: PathBuf,
        /// The name, as a formula would spell it: a letter or _, then letters, digits or _
        name: String,
        /// An address (`Data.B2:C9`), or an expression when it starts with `=`
        /// (`=SUM([$Data.$B$2:.$B$9])`). Omit to print the current definition.
        target: Option<String>,
        /// Delete the name instead
        #[arg(long, conflicts_with = "target")]
        delete: bool,
        /// Rename it, and every formula and every other name that uses it — one undo step
        #[arg(long, value_name = "NEW", conflicts_with_all = ["target", "delete"])]
        rename: Option<String>,
        /// Write what it stands for into every formula and name that uses it, then delete it —
        /// one undo step
        #[arg(long, conflicts_with_all = ["target", "delete", "rename"])]
        inline: bool,
    },

    /// Filter rows by a set of values per column
    ///
    /// `sheet filter book.ods A1:F12 C=Desk C=Lamp` keeps the rows whose column C holds one of
    /// those values and hides the rest. A value is matched exactly against what the cell displays;
    /// `C=` keeps the empty ones. With no range, prints each sheet's filter and the rows it hides.
    Filter {
        file: PathBuf,
        /// The filtered range, header row included: A1:F12, or Data.A1:F12. With --clear,
        /// any address on the sheet whose filter is to go.
        range: Option<String>,
        /// A column and one value it keeps: C=Desk. Repeat for more values or more columns.
        keep: Vec<String>,
        /// Remove the filter instead
        #[arg(long)]
        clear: bool,
        /// The range's first row is data rather than a heading
        #[arg(long)]
        no_header: bool,
    },

    /// Format a range as a table: autofilter, alternating row shading, an optional totals row, and
    /// a named range, in one undo step
    ///
    /// `sheet format-table book.ods A1:D12` bands the range, adds filter buttons on row 1, and
    /// names it `Table1` (or the next unused `TableN`), printing the name. The result is an
    /// ordinary filter, styling and name, so there is no un-format: `sheet filter --clear`, `sheet
    /// style` and `sheet name --delete` undo the parts, or undo the step.
    FormatTable {
        file: PathBuf,
        /// The table's range, header row included: A1:D12, or Data.A1:D12
        range: String,
        /// The range's first row is data rather than a heading
        #[arg(long)]
        no_header: bool,
        /// Append a totals row: the aggregate under every column it can answer for, its own
        /// word in the first. Bare --totals is a sum; name one of sum, average, count,
        /// count-numbers, min, max, stdev, var for the rest
        #[arg(long, num_args = 0..=1, default_missing_value = "sum")]
        totals: Option<grind_sheet::TotalsFunction>,
        /// The named range to create; default is the next unused Table1, Table2, …
        #[arg(long)]
        name: Option<String>,
    },

    /// Append an empty sheet
    Add {
        file: PathBuf,
        /// The new sheet's name, unique in the document; default is the first unused Sheet1,
        /// Sheet2, … — what every window's Add Sheet calls it
        name: Option<String>,
    },

    /// Rename a sheet
    ///
    /// Every formula, named expression and chart range naming the old sheet follows it, in
    /// one undo step. `sheet info` lists the sheets.
    Rename {
        file: PathBuf,
        /// The sheet to rename
        sheet: String,
        /// Its new name
        name: String,
    },

    /// Delete a sheet and everything on it
    ///
    /// The document's last sheet cannot be deleted. Undo brings the cells back.
    Remove {
        file: PathBuf,
        /// The sheet to delete
        sheet: String,
    },

    /// Recalculate every formula in the document
    Recalc { file: PathBuf },

    /// Undo the last change recorded in the session
    Undo { file: PathBuf },

    /// Redo the last undone change recorded in the session
    Redo { file: PathBuf },

    /// Parse a formula and print it back in normalised form
    ///
    /// Stored form brackets its references — =SUM([.B2:.B4]) — and display form, what a
    /// formula bar shows, does not: =SUM(B2:B4). The two convert losslessly.
    Fmt {
        /// The formula, e.g. '=SUM([.A1:.A2])'
        #[arg(allow_hyphen_values = true)]
        formula: String,
        /// Print it in display form, without the brackets
        #[arg(long, conflicts_with_all = ["from_display", "friendly"])]
        display: bool,
        /// Read display form and print the stored form
        #[arg(long, conflicts_with = "friendly")]
        from_display: bool,
        /// Print it for reading: full function names, one argument per line once it is too wide,
        /// each argument labelled with its parameter. For reading only; it cannot be typed back in
        #[arg(long)]
        friendly: bool,
        /// With --friendly, keep it on one line however wide it gets — what a formula bar
        /// shows, as against the multi-line explanation
        #[arg(long, requires = "friendly")]
        inline: bool,
    },

    /// Print every misspelt word in a text cell, with its cell and where in it the word starts
    ///
    /// Only typed text is checked — never a number, a date or a formula's result. A word any of
    /// the dictionaries knows is accepted, unless --language names one, which is then the only one
    /// checked. Suggestions come from the language the document states, else the one whose
    /// dictionary knows most of its words, which is printed to stderr. Words in your personal
    /// dictionary ($XDG_CONFIG_HOME/grind/words) are accepted; --add puts one there. Nothing is
    /// written to the document.
    #[cfg(feature = "spell")]
    Spell {
        #[arg(required_unless_present = "add")]
        file: Option<PathBuf>,
        /// Cells to check, e.g. B2:D9 or Sheet2.A1 — on every sheet checked when unqualified,
        /// everywhere when left out
        range: Option<String>,
        /// Check only this sheet, by name
        #[arg(long, value_name = "NAME")]
        sheet: Option<String>,
        /// The language to check in: en, de, fr, es, it, pt or pl, or any tag of one (de-AT,
        /// en_GB.UTF-8)
        #[arg(long, value_name = "LANG")]
        language: Option<String>,
        /// Print what each word might have been, best first
        #[arg(long)]
        suggest: bool,
        /// Add a word to your own list instead of checking anything
        #[arg(long, value_name = "WORD", conflicts_with_all = ["range", "sheet", "language", "suggest"])]
        add: Option<String>,
    },

    /// Replace one misspelt word in a text cell — what picking a suggestion does
    ///
    /// The word is named as well as placed, so a correction aimed at a cell that has since
    /// changed is refused rather than overwriting whatever is there now. The cell stays text
    /// and keeps its look; one undo step.
    Correct {
        file: PathBuf,
        /// The cell, e.g. B4 or Sheet2.B4
        address: String,
        /// The word that is there now
        word: String,
        /// What it should be
        with: String,
        /// Where the word starts, in characters, as `sheet spell` prints it — the first time
        /// the word occurs in the cell when left out
        #[arg(long, value_name = "N")]
        at: Option<usize>,
    },

    /// Print or set whether spreadsheets are spell-checked, and in what
    ///
    /// `off` turns spelling off in every spreadsheet window and for `grind lint`, and keeps it
    /// off until `auto` (the default: the document's own language, else a guess) or a language
    /// turns it back on. Remembered in $XDG_CONFIG_HOME/grind/spelling — yours, not any
    /// document's. With no setting, prints the remembered one.
    #[cfg(feature = "spell")]
    Spelling {
        /// auto, off, or a language: en, de, fr, es, it, pt or pl
        setting: Option<String>,
    },

    /// Print every cell holding some text, with its address
    ///
    /// What is searched is what a formula bar shows: a formula in display syntax
    /// (=SUM(B2:B4)), a date in ISO, a number in the document's own spelling. Case is ignored
    /// unless --match-case says otherwise.
    Find {
        file: PathBuf,
        #[arg(allow_hyphen_values = true)]
        needle: String,
        #[command(flatten)]
        search: SearchArgs,
    },

    /// Replace some text in every cell holding it, as one undo step
    ///
    /// Each changed cell is re-read as if the replaced text had been typed into it, so a
    /// formula stays a formula and a number stays a number. A formula the replace would
    /// break is left alone and named on stderr; the rest is still replaced.
    Replace {
        file: PathBuf,
        #[arg(allow_hyphen_values = true)]
        needle: String,
        #[arg(allow_hyphen_values = true)]
        replacement: String,
        #[command(flatten)]
        search: SearchArgs,
        /// Recalculate the document in the same undo step, unless that would spoil a cell
        #[arg(long)]
        recalc: bool,
    },

    /// List every calculated cell in a document — its formula, its result, what it calls
    ///
    /// Plain arithmetic counts: =A1/2 calls no function and is still a calculation.
    Calculations {
        file: PathBuf,
        /// Only cells whose sheet, address, formula or function names contain this
        #[arg(long, value_name = "TEXT")]
        filter: Option<String>,
    },

    /// List the OpenFormula functions grind implements
    Functions {
        /// Print each one's signature, summary and specification section
        #[arg(long)]
        long: bool,
        /// Only functions whose name contains this
        #[arg(long, value_name = "TEXT")]
        filter: Option<String>,
    },

    /// Add a chart — bar, line or pie
    ///
    /// `sheet chart-add book.ods --from A1:C5` charts a table the way the windows' Insert Chart
    /// does: a first row of text names the series, a first column of text, dates or a counting
    /// sequence names the categories, and the series run down the columns unless the figures are
    /// wider than they are tall. Any flag below overrides what that reading gave.
    ///
    /// Or spell it out: `sheet chart-add book.ods --type bar --categories B3:B9 --series C3:C9`
    /// adds a bar chart of column C, labelled by column B. Repeat --series for more than one
    /// series (a pie draws the first); each may carry its own name as `range=label-range`, e.g.
    /// `C3:C9=C2:C2`.
    ChartAdd {
        file: PathBuf,
        /// A table to chart, read the way Insert Chart reads one; `--series` is then optional
        #[arg(long, value_name = "RANGE")]
        from: Option<String>,
        /// Bar, line or pie; with `--from`, defaults to what the table suggests
        #[arg(value_enum, long)]
        r#type: Option<ChartType>,
        /// The range naming each category (the x axis), e.g. B3:B9
        #[arg(long)]
        categories: Option<String>,
        /// A series' values, optionally followed by `=label-range` — repeat for more than
        /// one series. Required unless `--from` is given; given, these replace its series
        #[arg(long = "series", value_name = "RANGE[=LABEL]")]
        series: Vec<String>,
        /// Which way a pie runs from twelve o'clock (default true)
        #[arg(long, value_name = "BOOL")]
        clockwise: Option<bool>,
        /// The chart's own title; with `--from`, a single series' name is the default
        #[arg(long, value_name = "TEXT")]
        title: Option<String>,
        /// Where the legend sits, or none; defaults to the end when there is more than one
        /// series to tell apart, or a pie's slices
        #[arg(value_enum, long)]
        legend: Option<LegendArg>,
        /// Which sheet the ranges (and the chart itself) are on; defaults to the first
        #[arg(long)]
        sheet: Option<String>,
        #[arg(long, default_value = "1cm")]
        x: String,
        #[arg(long, default_value = "1cm")]
        y: String,
        #[arg(long, default_value = "10cm")]
        width: String,
        #[arg(long, default_value = "8cm")]
        height: String,
        /// The x axis' own title
        #[arg(long, value_name = "TEXT")]
        x_axis_label: Option<String>,
        /// The y axis' own title
        #[arg(long, value_name = "TEXT")]
        y_axis_label: Option<String>,
        /// Draw the category names along the x axis (default true)
        #[arg(long, value_name = "BOOL")]
        x_tick_labels: Option<bool>,
        /// Draw the value scale along the y axis (default true)
        #[arg(long, value_name = "BOOL")]
        y_tick_labels: Option<bool>,
        /// Rule gridlines up from the x axis (default false)
        #[arg(long, value_name = "BOOL")]
        x_gridlines: Option<bool>,
        /// Rule gridlines across from the y axis (default false)
        #[arg(long, value_name = "BOOL")]
        y_gridlines: Option<bool>,
    },

    /// Change what a chart is: its type, its ranges and its axes
    ///
    /// Every flag left off keeps what the chart already has, so
    /// `sheet chart-edit book.ods 0 --type line` changes only the shape. `--series` replaces
    /// the whole list when given at all (repeat it, `RANGE[=LABEL]` each, exactly as
    /// `chart-add` takes them); a colour picked by hand survives for a series still pointing
    /// at the same range. `--categories ''` clears the categories, and the chart keeps its
    /// position and size — that is `chart-reshape`'s.
    ChartEdit {
        file: PathBuf,
        /// The chart's position in `chart-list`, 0-based
        index: usize,
        /// Defaults to the first
        #[arg(long)]
        sheet: Option<String>,
        #[arg(value_enum, long)]
        r#type: Option<ChartType>,
        /// The range naming each category (the x axis); pass an empty string to clear it
        #[arg(long)]
        categories: Option<String>,
        /// A series' values, optionally followed by `=label-range` — repeat for more than
        /// one series. Given at all, these replace every series the chart had.
        #[arg(long = "series", value_name = "RANGE[=LABEL]")]
        series: Vec<String>,
        /// Which way a pie runs from twelve o'clock
        #[arg(long, value_name = "BOOL")]
        clockwise: Option<bool>,
        /// The chart's own title; pass an empty string to clear it
        #[arg(long, value_name = "TEXT")]
        title: Option<String>,
        /// Where the legend sits, or none
        #[arg(value_enum, long)]
        legend: Option<LegendArg>,
        /// The x axis' own title; pass an empty string to clear it
        #[arg(long, value_name = "TEXT")]
        x_axis_label: Option<String>,
        /// The y axis' own title; pass an empty string to clear it
        #[arg(long, value_name = "TEXT")]
        y_axis_label: Option<String>,
        /// Draw the category names along the x axis
        #[arg(long, value_name = "BOOL")]
        x_tick_labels: Option<bool>,
        /// Draw the value scale along the y axis
        #[arg(long, value_name = "BOOL")]
        y_tick_labels: Option<bool>,
        /// Rule gridlines up from the x axis
        #[arg(long, value_name = "BOOL")]
        x_gridlines: Option<bool>,
        /// Rule gridlines across from the y axis
        #[arg(long, value_name = "BOOL")]
        y_gridlines: Option<bool>,
    },

    /// List a sheet's charts
    ChartList {
        file: PathBuf,
        /// Defaults to the first
        sheet: Option<String>,
    },

    /// Remove a chart
    ChartRemove {
        file: PathBuf,
        /// The chart's position in `chart-list`, 0-based
        index: usize,
        /// Defaults to the first
        sheet: Option<String>,
    },

    /// Move or resize a chart
    ChartReshape {
        file: PathBuf,
        /// The chart's position in `chart-list`, 0-based
        index: usize,
        #[arg(long)]
        x: String,
        #[arg(long)]
        y: String,
        #[arg(long)]
        width: String,
        #[arg(long)]
        height: String,
        /// Defaults to the first
        #[arg(long)]
        sheet: Option<String>,
    },

    /// Set a chart's axes or a mark's colour
    ///
    /// `sheet chart-style book.ods 0 --x-axis-label Party --y-axis-label Votes` titles a
    /// chart's axes, and `--y-gridlines true` rules gridlines across it.
    /// `--series-color 0=navy` sets a whole bar or line series' colour, and `--point-color
    /// 0.2=red` one bar of it or one slice of a pie — the third of series 0. Either repeatable
    /// flag with nothing after `=` — `--series-color 0=` — goes back to the default cycle.
    ChartStyle {
        file: PathBuf,
        /// The chart's position in `chart-list`, 0-based
        index: usize,
        /// Defaults to the first
        #[arg(long)]
        sheet: Option<String>,
        /// The x axis' own title; pass an empty string to clear it
        #[arg(long, value_name = "TEXT")]
        x_axis_label: Option<String>,
        /// The y axis' own title; pass an empty string to clear it
        #[arg(long, value_name = "TEXT")]
        y_axis_label: Option<String>,
        /// Draw the category names along the x axis
        #[arg(long, value_name = "BOOL")]
        x_tick_labels: Option<bool>,
        /// Draw the value scale along the y axis
        #[arg(long, value_name = "BOOL")]
        y_tick_labels: Option<bool>,
        /// Rule gridlines up from the x axis
        #[arg(long, value_name = "BOOL")]
        x_gridlines: Option<bool>,
        /// Rule gridlines across from the y axis
        #[arg(long, value_name = "BOOL")]
        y_gridlines: Option<bool>,
        /// A bar or line series' whole colour, as `SERIES=COLOR` (a palette name or
        /// `#rrggbb`) — `SERIES=` clears it. Repeat for more than one series.
        #[arg(long = "series-color", value_name = "SERIES=COLOR")]
        series_color: Vec<String>,
        /// One bar's or slice's colour, as `SERIES.POINT=COLOR` — `SERIES.POINT=` clears it.
        /// Repeat for more than one.
        #[arg(long = "point-color", value_name = "SERIES.POINT=COLOR")]
        point_color: Vec<String>,
    },
}

/// The three shapes `sheet chart-add --type` takes (`doc/chart-format.md`).
#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum ChartType {
    Bar,
    Line,
    Pie,
}

/// Where `--legend` puts a chart's legend, or `none` — ODF's own words (`chart:legend-position`),
/// `end` being the right-hand side of a left-to-right chart.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum LegendArg {
    None,
    End,
    Bottom,
    Top,
    Start,
}

impl From<LegendArg> for Option<grind_sheet::ChartLegend> {
    fn from(legend: LegendArg) -> Self {
        match legend {
            LegendArg::None => None,
            LegendArg::End => Some(grind_sheet::ChartLegend::End),
            LegendArg::Bottom => Some(grind_sheet::ChartLegend::Bottom),
            LegendArg::Top => Some(grind_sheet::ChartLegend::Top),
            LegendArg::Start => Some(grind_sheet::ChartLegend::Start),
        }
    }
}

impl From<ChartType> for grind_sheet::ChartKind {
    fn from(t: ChartType) -> Self {
        match t {
            ChartType::Bar => grind_sheet::ChartKind::Bar,
            ChartType::Line => grind_sheet::ChartKind::Line,
            ChartType::Pie => grind_sheet::ChartKind::Pie,
        }
    }
}

/// `chart-add`/`chart-list`/`chart-remove`'s own `--sheet`/positional sheet argument,
/// defaulting to the first — the same default `a1::as_definition` gives a named range.
/// What `sheet find` and `sheet replace` share: how a needle matches, and where.
#[derive(clap::Args)]
pub(crate) struct SearchArgs {
    /// Match `Total` only, not `total` as well
    #[arg(long)]
    pub(crate) match_case: bool,
    /// The needle must be the cell's whole content, not a piece of it
    #[arg(long)]
    pub(crate) whole_cell: bool,
    /// Only this sheet; defaults to every sheet
    #[arg(long)]
    pub(crate) sheet: Option<String>,
    /// Only the cells in this range, e.g. B2:D40 or Notes.A1:A9 — a sheet named in it
    /// narrows the search to that sheet as well
    #[arg(long = "in", value_name = "RANGE")]
    pub(crate) within: Option<String>,
}

impl SearchArgs {
    pub(crate) fn resolve(
        &self,
        app: &App,
        needle: &str,
    ) -> Result<grind_sheet::find::Search, String> {
        let mut sheet = match &self.sheet {
            Some(name) => Some(a1::sheet(app, name).say()?),
            None => None,
        };
        let mut range = None;
        if let Some(within) = &self.within {
            let reference = a1::parse(within).say()?;
            let (on, start, end) = a1::resolve(app, &reference).say()?;
            // An unqualified range is on every sheet searched; a qualified one names its own.
            if reference.start.sheet.is_some() {
                if sheet.is_some_and(|sheet| sheet != on) {
                    return Err(format!("{within} is not on the sheet --sheet names"));
                }
                sheet = Some(on);
            }
            range = Some((start, end));
        }
        Ok(grind_sheet::find::Search {
            needle: needle.to_owned(),
            match_case: self.match_case,
            whole_cell: self.whole_cell,
            sheet,
            range,
        })
    }
}
