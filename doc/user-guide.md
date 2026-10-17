# grind User Guide

Draft of 16 October 2026. grind has no release yet. This guide describes the software as it is built today, and says so where a program lacks a feature or works differently.

# 1. About this guide

## 1.1 Who this guide is for

This guide is for people who write letters and reports, keep budgets and lists in spreadsheets, and want their files in an open format. You do not need to be a programmer. Chapters 1 to 7 cover everyday work in the windows, the terminal and the browser. Chapters 8 and 9 show how to repeat work from the command line and with scripts, and you can skip them until you need them.

## 1.2 What grind is

grind is an office suite with two kinds of document: **spreadsheets** and **text documents**. Both are saved in the OpenDocument Format (ODF), the international standard that LibreOffice, Collabora Office and many other programs read and write. A document you make in grind is not locked to grind.

The suite comes as several programs that share one engine. A formula, a spelling rule or a page layout means the same thing in all of them, and a document made in one opens in all the others.

| **Program** | **What it is** | **Runs on** |
| --- | --- | --- |
| grind-sheet-gtk | The spreadsheet window | Linux (GNOME) |
| grind-text-gtk | The word processor window | Linux (GNOME) |
| grind-tui | Both kinds of document, in a terminal | Linux, macOS, Windows |
| grind web | Both kinds of document, in a browser tab | Any current browser |
| grind for Windows | Both kinds of document, in one window | Windows |
| grind for Mac | Both kinds of document, in one window | macOS |
| grind | The command line, with every function | Linux, macOS, Windows |

Which program you use is a matter of taste. Most screenshots in this guide show the Linux windows and the browser.

## 1.3 What grind does differently

Four things set grind apart from other office suites.

**You can see what is data and what is a calculation.** A spreadsheet can mark every cell as an input, a formula or a label, and show the whole document as readable text. Section 3.12 explains how.

**Formulas can be read in plain English.** `=SUM(B2:B7)` can be shown as *Sum(Number: B2:B7)*, and a long formula can be explained step by step.

**Everything works from the command line.** Every function of every program is also a command, for people, for shell scripts and for other programs.

**Repetitive documents are built by scripts, not macros.** grind has no macros, and never will. Instead, a small script can build a document from data, such as a letter for each person on a list.

## 1.4 Conventions used in this guide

Keys are written like **Ctrl+S**. A **bold** word is something you see on the screen, such as a button or a menu entry. Text in `this typeface` is something you type. A line that begins with `$` is typed in a terminal, without the `$`.

The keyboard shortcuts are those of the Linux windows. The Windows and Mac programs follow the conventions of their own platform. On a Mac, the **Command** key takes the place of **Ctrl**.

# 2. Getting started

## 2.1 Starting a program

Start a program from your desktop's application menu, or from a terminal with the name of a file. Without a file name you start with an empty document.

```
$ grind-sheet-gtk budget.fods
$ grind-text-gtk letter.fodt
$ grind-tui report.fodt
```

The terminal program, `grind-tui`, reads the file to see whether it is a spreadsheet or a text document, so one command opens both. To start an empty text document in it, type `grind-tui --text`.

The Windows and Mac programs, and the browser version, open on a welcome screen with three choices: a new spreadsheet, a new text document, or a document you already have.

## 2.2 The spreadsheet window

![The spreadsheet window with a household budget](user-guide.images/img-10b68c6f6cff5e5b.png)

The window has four parts:

- **The header bar** at the top, with the document name, undo and redo, and buttons for opening files, searching, viewing options and the main menu.
- **The format bar** below it, which changes how the selected cells look.
- **The formula bar**, which shows the address of the selected cell on the left and its content on the right.
- **The grid**, with the sheet tabs below it.

## 2.3 The word processor window

![The word processor window with a short report](user-guide.images/img-f08bab58395c7e61.png)

The word processor has a header bar like the spreadsheet's, a toolbar for block styles and character formatting, and the page you write on. The status bar at the bottom shows the kind of the current block, the number of words, the reading time and the spelling language. In the picture, the misspelt word *anual* has a red wavy line.

## 2.4 Opening documents

Press **Ctrl+O** to open a document. grind opens:

- OpenDocument spreadsheets and text documents (`.ods`, `.fods`, `.odt`, `.fodt`),
- the plain-text `.grind` form (section 9.4),
- Excel workbooks (`.xlsx`) and Word documents (`.docx`),
- CSV and TSV files, as spreadsheets,
- Markdown files (`.md`), as text documents.

grind decides what a file is by reading it, not by its name. An Excel workbook, a Word document, a CSV or a Markdown file is converted into a **new, unsaved document**. The original file is never changed, and a message tells you what, if anything, could not be carried over. grind reads these formats but does not write Excel or Word files.

## 2.5 Saving documents

Press **Ctrl+S** to save. A document without a name asks for one. In the word processor, **Ctrl+Shift+S** saves under a new name.

grind writes ODF in two forms:

- A **flat** file (`.fods`, `.fodt`) is one readable text file. grind offers it first, because it works well with version control and because you can open it in a text editor if you ever need to.
- A **package** (`.ods`, `.odt`) is a compressed archive, which is what most other programs save by default.

Both open in every program that reads ODF. Choose the form by the extension you give the file.

When you save a document that another program made, grind keeps everything it does not understand itself, and writes it back unchanged. Saving a document never makes it worse: if grind could not write something back faithfully, it refuses to save rather than lose it.

## 2.6 Undo and redo

**Ctrl+Z** undoes the last change and **Ctrl+Shift+Z** redoes it. In the word processor, **Ctrl+Y** also redoes. A change that touches many cells at once, such as a fill or a replace, is undone in one step.

# 3. Working with spreadsheets

## 3.1 Cells and addresses

The grid is made of numbered rows and lettered columns. The place where a row and a column meet is a **cell**, named by its column and row: the cell in column B, row 3 is **B3**. A rectangle of cells is a **range**, named by two corners: **B2:B7**. A cell on another sheet carries the sheet's name: **Prices.B2**.

## 3.2 Moving around and selecting

| **Keys** | **What they do** |
| --- | --- |
| Arrow keys | Move one cell |
| Ctrl+arrow | Jump to the edge of the data |
| Home, End | Go to the start or end of the row |
| Ctrl+Home, Ctrl+End | Go to the start or end of the sheet |
| Page Up, Page Down | Move a screenful |
| Shift with any of these | Extend the selection |
| Ctrl+A | Select the whole sheet |

You can also click a cell, drag across a range, or click a row number or a column letter to select the whole row or column. To go to a cell, type its address, or the name of a named range, in the name box at the left of the formula bar.

## 3.3 Entering data

Select a cell and type. **Enter** accepts the entry and moves down. **Tab** accepts it and moves right. **Shift** with either moves the other way. **Esc** cancels. To change what is already in a cell, press **F2** or double-click it.

grind decides what you typed by one rule:

- A leading `=` makes a formula.
- A number makes a number, read with the document's locale (section 3.10).
- `TRUE` and `FALSE` make logical values.
- Anything else is text.

To enter text that looks like a number or a formula, such as the part number `007`, start it with an apostrophe: `'007`.

## 3.4 Formulas

A formula starts with `=`. It can use numbers, cell references, ranges, the operators `+ - * / ^`, the comparison operators `= <> < > <= >=`, and functions:

```
=B2*1.19
=SUM(B2:B7)
=IF(C2>B2; "over budget"; "ok")
=VLOOKUP(A2; Prices.A2:B40; 2; FALSE())
```

Arguments are separated by a semicolon. When you press **Enter**, the cell shows the result and the formula bar keeps the formula.

**Help while typing.** When you start a function name, grind lists the functions that fit. Press **Tab** to accept one. A hint below the formula bar shows the function's arguments and which one you are typing. **Enter** always finishes the cell, even while the list is shown.

**Pointing at cells.** While you edit a formula, you can click a cell or use the arrow keys to put its address into the formula.

**Recalculation.** A formula recalculates when a cell it depends on changes. Press **F9** to recalculate the whole document.

## 3.5 Functions

grind implements the functions of the OpenFormula standard's core set, plus many common ones beyond it: 139 functions in all, among them `SUM`, `AVERAGE`, `COUNTIF`, `SUMIF`, `IF`, `VLOOKUP`, `ROUND`, `DATE`, `TODAY`, `PMT`, `TEXT` functions such as `LEFT` and `CONCATENATE`, and many more.

To browse them, choose **Functions…** from the main menu or the command palette. The list gives each function a plain-English name, a category and a short description. In a terminal, `grind sheet functions` prints the same list.

If a document uses a function that grind does not implement, the cell keeps the value it was saved with, and grind tells you which cells are affected.

## 3.6 Reading a formula in plain English

The formula bar can show a formula as a sentence. `=ROUND(A1; 2)` reads as *Round(Value: A1, Digits: 2)*, and `=SUM(B2:B7)` as *Sum(Number: B2:B7)*. This only changes how the formula is shown, never the formula itself. Turn it on and off with **Read Formulas as Sentences** in the View menu.

For a long, nested formula, choose **Explain This Formula**. It shows the formula taken apart, one function at a time, with the value of each part.

## 3.7 Filling, copying and clearing

To repeat a formula down a column, select the cell with the formula and the cells below it and press **Ctrl+D**. **Ctrl+R** fills to the right. You can also drag the small square at the corner of the selection, the **fill handle**. References in the formula move along: `=B2*2` filled down becomes `=B3*2`.

**Ctrl+C**, **Ctrl+X** and **Ctrl+V** copy, cut and paste. A copied range is placed on the clipboard as tab-separated text, so it pastes into any other spreadsheet or a text editor. **Delete** clears the selected cells.

## 3.8 Formatting cells

The format bar acts on the selected cells. From left to right it has bold, italic, underline and strikethrough; left, centre and right alignment; less and more indent; text wrapping; borders; text colour and fill colour; the number format; and **Clear**, which removes the formatting.

The number format decides how a number is shown. Choose from the list:

| **Format** | **Shows 1234.5 as** |
| --- | --- |
| General | 1234.5 |
| Number | 1,234.50 |
| Percent | 123450.00% |
| Currency | 1,234.50 € |
| Date, Time, Date and time | the number as a date or a time |
| Boolean | TRUE or FALSE |
| Text | the value as it was typed |

The separators depend on the document's locale (section 3.10). Formatting never changes the value itself: a cell showing 1,235 may hold 1234.5, and calculations use 1234.5.

**Column widths and row heights.** Drag the line between two column letters to change a column's width. Double-click it to fit the column to its widest entry. Rows work the same way. Rows grow by themselves to fit wrapped text.

**Zoom.** Hold **Ctrl** and turn the mouse wheel, or press **Ctrl++**, **Ctrl+-** and **Ctrl+0**.

## 3.9 Sheets

A document can have several sheets, shown as tabs at the bottom of the window. Click **+** to add a sheet. Right-click a tab to rename or delete it. When you rename a sheet, grind updates every formula, name and chart that refers to it, and one **Ctrl+Z** undoes the whole rename.

## 3.10 The document's locale

Every spreadsheet can state a **locale**, which decides how numbers are written and read. In a German document you type `1.234,50`; in an English one, `1,234.50`. Because the locale is stored in the document, everybody who opens it sees the same numbers. Set it in **Document Settings…** in the main menu, which also shows a sample of how a number will look.

## 3.11 Charts

To draw a chart, select the table, including its headings, and choose **Insert a Chart** from the command palette or the cell's context menu. The chart dialog shows a live preview. grind guesses which row or column holds the labels and whether the data runs down or across, and you can change the guess. Bar, line and pie charts are available.

A chart sits on the sheet. Click it to select it, drag it to move it, and drag one of its eight handles to resize it. **Shift** on a corner handle keeps the proportions. **Delete** removes the selected chart, the arrow keys nudge it, and **Esc** lets go.

## 3.12 Seeing what a spreadsheet is made of

A table of numbers does not show which numbers somebody typed and which a formula calculated. grind can show it.

![Cell roles: inputs in blue, calculations and labels marked](user-guide.images/img-4e49585c801b138b.png)

Turn on **Show What Each Cell Is** in the View menu. Every cell gets a small mark:

| **Mark** | **Meaning** |
| --- | --- |
| Filled diamond, blue | An input with a name, used by formulas |
| Hollow diamond | An input, such as a number in a table |
| ! | A number that a formula uses by its address, without a name. Consider naming it |
| = | A calculation on the same sheet |
| Arrow | A calculation that reads another sheet |
| T | A label |
| Cross | A formula that gives an error |
| \~ | A formula whose saved value is out of date |

**Show Where Names Live** outlines every named range. **Show Formulas** shows formulas instead of their results. **Show the Source** (**Ctrl+Shift+U**) shows the whole document as plain text, one line per cell, and selecting a line selects its cell. None of these views changes the document.

## 3.13 Finding and replacing

Press **Ctrl+F** to search. grind searches what the cells show, and moves to each match in turn. The replace field changes text and formulas in every matching cell, as one change that **Ctrl+Z** undoes.

## 3.14 Checking a spreadsheet

Press **F8** for **Check the Document**. grind lists problems such as a formula that gives an error, a reference to an empty cell, a saved value that is out of date, or a misspelt word. Click an entry to go to the cell.

## 3.15 CSV files

Opening a CSV or TSV file makes a new spreadsheet from it. To read a CSV file into the spreadsheet you have, use **Import CSV**; to write a range out, use **Export CSV**. grind finds the delimiter from the file's content. Fields that look like numbers become numbers, and a field such as `007` stays text.

# 4. Working with text documents

## 4.1 Blocks

A text document is a sequence of **blocks**. A block is a paragraph, a heading or a list item. **Enter** ends a block and starts a new one, and **Backspace** at the start of a block joins it to the one before.

Choose a block's kind from the box at the left of the toolbar, or with the keyboard:

| **Keys** | **Block kind** |
| --- | --- |
| Ctrl+0 | Paragraph |
| Ctrl+1, Ctrl+2, Ctrl+3 | Heading 1, 2, 3 |
| Ctrl+L | List item |

The box also offers Title, Subtitle and Headings 4 to 6. Press **Tab** at the start of a list item to indent it one level, and **Shift+Tab** to bring it back.

Headings make up the document's outline. **Ctrl+Shift+O** opens it, and choosing a heading takes you there.

## 4.2 Formatting text

Select text and use the toolbar or the keys:

| **Keys** | **Format** |
| --- | --- |
| Ctrl+B | Bold |
| Ctrl+I | Italic |
| Ctrl+U | Underline |

The toolbar also has strikethrough, code, the font and the size, the text colour and the highlight colour, and **Clear**, which removes direct formatting. A button that looks pressed tells you the selection already has that format.

**Formatting as you type.** You can type formatting the way many people do in plain text. When you type `**bold**`, the asterisks disappear and the word is bold. The same works for:

| **You type** | **You get** |
| --- | --- |
| `**word**` | **bold** |
| `*word*` | *italic* |
| `__word__` | underline |
| `~~word~~` | strikethrough |
| `` `word` `` | code |
| `#`  at the start of a line | a heading |
| `-`  at the start of a line | a list item |

One **Ctrl+Z** takes back the whole formatted word.

## 4.3 Tables and pictures

**Ctrl+Shift+T** inserts a table. Click a cell to type in it; a cell can hold several paragraphs.

**Ctrl+Shift+I** inserts a picture from a file. Give each picture **alternative text** with **Ctrl+Alt+A**. Screen readers speak it, and it stands in for the picture where the picture cannot be shown.

## 4.4 Going to a place

**Ctrl+G** goes to an address. **Ctrl+F** searches. A place in a document can be named three ways:

| **Address** | **Means** |
| --- | --- |
| `p12` | The twelfth block |
| `#intro` | The block with the bookmark *intro* |
| `§2.1` | The first subsection of section 2 |

A bookmark or a section address keeps pointing at the same place when you edit above it. A block number does not.

## 4.5 Pages, print preview and PDF

While you write, a document is one continuous column. grind lays it out on pages when you print or export it.

![Print preview of a letter on A4](user-guide.images/img-d2c972610d028941.png)

| **Keys** | **Command** |
| --- | --- |
| Ctrl+Shift+P | Print preview |
| Ctrl+Shift+E | Export as PDF |
| Ctrl+P | Print |

The pages are those the document states: the paper size, the margins, and the header and footer with their page numbers. A document that states no page is printed on A4. grind prints only on the international ISO paper sizes, such as A4 and A5.

The same document always produces the same PDF. The PDF has an outline of the headings, and it is tagged, so screen readers can read it.

## 4.6 Headers, footers and letterheads

A document's header and footer are printed on every page. A page number or a page count in one shows the right number on each page.

A document can use different pages for different parts. A letter template, for example, can put a full letterhead on the first page and a short header on the pages after it, and draw fold marks at the edge of the paper. grind prints all of these as the template describes them. Section 9.2 shows how to fill such a template automatically.

## 4.7 Markdown

Markdown is a plain-text notation for headings, lists and emphasis. A `.md` file opens as a new, unsaved document. **Import Markdown** inserts a Markdown file into the document you have, and **Export Markdown** (**Ctrl+Alt+S**) saves the document, or the selection, as a Markdown file. Pictures are written as files in a folder next to it.

## 4.8 Word documents

A Word document (`.docx`) opens as a new, unsaved text document. Headings, lists, tables, pictures, headers, footers and the page come across. A message names anything that does not. Save the result as an ODF document.

# 5. Spelling

grind checks spelling as you type, in text documents and in the text of spreadsheet cells. A word grind does not know gets a red wavy line. The word you are still typing is left alone until you finish it.

## 5.1 Languages

grind checks English (US), German, French, Spanish, Italian, Portuguese (Portugal) and Polish. A document is checked in one language at a time. The status bar of the word processor shows which.

The language is **Automatic** unless you choose one. grind then uses the language the document states, if it states one, and otherwise judges from the words in it.

To choose, open **Spelling Language**:

- In the spreadsheet window, it is in the View menu (the eye button), next to **Check Spelling**, which turns spelling on and off (**Shift+F7**). The command palette has **Spelling Language…** as well.
- In the word processor window, it is the **Spelling** submenu of the main menu.
- In the browser, the command palette lists **Spelling: automatic**, **Spelling: off** and one entry for each language.
- In the terminal, type `:spell de`, `:spell auto` or `:spell off`.

Your choice is remembered, separately for spreadsheets and for text documents, and it holds for every document you open until you change it. It is your setting, and it is never written into a document. If you turn spelling off, it stays off.

## 5.2 Correcting a word

Right-click a misspelt word. The menu lists suggestions; choose one to replace the word. **Ignore All** accepts the word until you close the program. **Add to Dictionary** keeps it in your own word list for good.

![The command palette in the browser, listing the spelling commands](user-guide.images/img-f08d259f77e81f6e.png)

In the browser, put the cursor in the word and press **Ctrl+K**: the palette lists the suggestions, **Ignore** and **Add to your dictionary**, and **Go to the next misspelling**. The browser fetches a language's dictionary the first time it needs it, so the first check in a language takes a moment.

In a spreadsheet only typed text is checked: never a number, a date or the result of a formula.

# 6. Using grind in a terminal

`grind-tui` runs in any terminal. It works like the editor vi: the keyboard is in one of a few **modes**.

| **Mode** | **For** | **Enter it with** |
| --- | --- | --- |
| Normal | Moving around | **Esc** |
| Insert | Typing | `i` |
| Visual | Selecting | `v` |
| Command | Commands | `:` |

The title bar at the top shows the document and its sheets or headings. The status bar at the bottom shows the mode, your position and whether there are unsaved changes.

## 6.1 Normal and Visual mode

| **Key** | **What it does** |
| --- | --- |
| `h` `j` `k` `l` or the arrows | Move |
| `i` | Start typing |
| `v` | Start selecting |
| `y` | Copy the selection |
| `p` | Paste |
| `d` or `x` | Delete |
| `u` | Undo |
| `n`, `N` | Next and previous search match |
| `]`, `[` | Next and previous misspelt word |
| `*` `/` `_` `~` (Visual mode) | Bold, italic, underline, strikethrough |

## 6.2 Commands

Type `:` and a command, then **Enter**.

| **Command** | **What it does** |
| --- | --- |
| `:w` | Save |
| `:q` | Quit; `:q!` quits without saving |
| `:wq` | Save and quit |
| `:help` | Show help |
| `:find word` | Search |
| `:s/old/new/` | Replace (spreadsheet) |
| `:recalc` | Recalculate (spreadsheet) |
| `:roles` | Mark what each cell is (spreadsheet) |
| `:outline` | List the headings (text) |
| `:table` | Insert a table (text) |
| `:mark name` | Set a bookmark (text) |
| `:pdf` | Export as PDF (text) |
| `:spell` | Show the spelling language; `:spell de` chooses one |
| `:fix 1` | Replace a misspelt word with the first suggestion |
| `:lint` | Check the document |
| `:source` | Show the document as plain text |

# 7. Using grind in the browser

![A spreadsheet in the browser](user-guide.images/img-faf768ac6516e543.png)

The browser version is a web page with no menus. It has one bar with **Open…**, **Save**, undo and redo, and **Commands**, which opens the command palette. A second bar holds the formatting tools for the kind of document that is open. A spreadsheet adds the formula bar and the sheet tabs.

## 7.1 The command palette

Press **Ctrl+K**, or click **Commands**, type a few letters of what you want, and press **Enter**. The palette holds every command that has no button. It is also the way to go somewhere: type an address such as `B12` or `p12`, the name of a sheet, a named range, a heading or a bookmark.

## 7.2 Opening and saving

Click **Open…**, or drop a file anywhere on the page. **Save** downloads a copy to your downloads folder. It cannot write over the file you opened, because a web page has no access to your files.

Your documents never leave your computer: the page reads and writes them itself. Your spelling choices and your own word list are kept by the browser for this page only, and clearing the page's data clears them.

# 8. The command line

Every function of grind is also a command. This is useful for repeating a job, for working on a server, and for checking a file without opening it.

## 8.1 The shape of a command

```
$ grind <area> <verb> <file> [arguments]
```

The area is `sheet` for spreadsheets and `text` for text documents. A few verbs work on any document and find out what it is by reading it: `info`, `lint`, `convert` and `project`.

Each command reads the file, does one thing and writes the file back. Two options work with every command: `--dry-run` shows what a command would do without writing anything, and `--format json` gives the answer in a form other programs can read. Add `--help` to any command for a full description.

## 8.2 A spreadsheet from the command line

This example makes a small spreadsheet, adds a formula and shows the result:

```
$ grind sheet new book.ods
$ grind sheet set book.ods A1 1
$ grind sheet set book.ods A2 '=[.A1]*2'
$ grind sheet recalc book.ods
$ grind sheet view book.ods A1:A2
```

On the command line, formulas are written in ODF's own form, as they are stored in the file: `[.A1]` is the cell A1. The `eval` command tries out a formula without storing it. Give it the cell the formula would sit in, because relative references depend on that; `--trace` shows the value of every part:

```
$ grind sheet eval book.ods A3 '=SUM([.A1:.A2])'
$ grind sheet eval book.ods A3 '=SUM([.A1:.A2])' --trace
```

Other useful commands:

```
$ grind sheet import-csv book.ods data.csv
$ grind sheet export-csv book.ods --delimiter tab
$ grind sheet format book.ods B2:B9 currency
$ grind sheet locale book.ods de-DE
$ grind sheet functions
```

## 8.3 A text document from the command line

```
$ grind text new letter.fodt
$ grind text import-md letter.fodt notes.md
$ grind text view letter.fodt
$ grind text export-md letter.fodt
$ grind text export-pdf letter.fodt letter.pdf --paper a4
$ grind text preview letter.fodt page1.png --page 1
```

A block is addressed as in the window: `p12`, `#intro` or `§2.1`.

## 8.4 Any document

```
$ grind info report.fodt
$ grind lint report.fodt
$ grind convert book.fods book.ods
$ grind convert book.fods book.grind
$ grind sheet import workbook.xlsx workbook.fods
$ grind text import letter.docx letter.fodt
```

`grind info` tells you what a document is and what is in it. `grind lint` checks it and ends with an error when it finds one, so you can use it in a script. `grind convert` changes between the package, the flat file and the `.grind` form. The two `import` commands convert an Excel workbook or a Word document into ODF.

## 8.5 Spelling from the command line

```
$ grind text spell report.fodt --suggest
$ grind text spelling de
$ grind sheet spelling off
```

The first command lists the misspelt words in a document, with suggestions. `grind text spelling` and `grind sheet spelling` set the remembered language for each kind of document, the same setting the windows use. Without a value they print the current one.

# 9. Automating work

## 9.1 Scripts that build documents

A script is a small text file that describes a document. It is written in a simple language called Rhai. A script cannot read your files, use the network or start programs. It can read one data file in JSON format from its own folder. Because of these limits, the same script and data always produce exactly the same document.

```
$ grind build budget.rhai -o budget.fods
$ grind test budget.rhai
```

`grind build` builds the document. `grind test` runs the checks written into the script, such as "the total is the sum of the rows", and reports any that fail.

Scripts suit documents you make again and again from changing data: a monthly report, a loan schedule, a grade book, or a letter to each person on a list. The `examples` folder has a working script for each of these.

## 9.2 A letter to each person on a list

A script can return several documents at once. grind then writes them into a folder, one file each. The letter examples write one business letter per recipient, laid out on a DIN 5008 template, as PDF files:

```
$ grind build examples/letters/nebenkosten.rhai -o letters --as pdf
```

The template is an ordinary text document with the page, the letterhead and the paragraph styles. The script fills in the recipient, the date and the figures, and calculates the figures itself. You design the template in any word processor and save it as a flat text document (`.fodt`).

## 9.3 Checking documents automatically

`grind lint` looks for the mistakes people make in spreadsheets and documents: a formula that gives an error, a saved value that is out of date, a style that does not exist, a misspelt word. It never changes the document, and it ends with an error when it finds one, so it can check documents automatically.

## 9.4 The plain-text form

The `.grind` form writes a document as readable text. A spreadsheet becomes a list of sheets, rows and cells:

```
$ grind sheet project book.ods
```

You can edit the `.grind` file in any text editor and open it again in grind. A change to one cell changes one line of the file, which makes the form useful for reviewing a spreadsheet somebody else made and for keeping documents in a version-control system such as Git.

# 10. Troubleshooting

**A file does not open.** Check what the file is with `grind info` followed by the file name. Password-protected files cannot be opened. A flat file is plain text, so a damaged one can be read in a text editor.

**A cell shows an error such as** **`#DIV/0!`** **or** **`#NAME?`****.** Select the cell and read its formula, or let grind explain it (section 3.6). **Check the Document** (**F8**) lists every cell with an error.

**A cell shows** **`###`****.** The column is too narrow for the number. Widen it, or double-click the line at the right of the column letter.

**Numbers look wrong.** Check the document's locale (section 3.10). `1.234` is one thousand two hundred thirty-four in German and one and a bit in English.

**Spelling underlines almost every word.** The document is being checked in the wrong language. Choose the right one, or **Automatic** (section 5.1).

**A document looks different in another program.** Programs can draw the same ODF slightly differently, and a font that is not installed is replaced. To share exactly how a page looks, export a PDF.

**A command fails.** Add `--help` to the command to see what it expects. `grind --help` lists all commands.

# Appendix A: Keyboard shortcuts

These are the shortcuts of the Linux windows. In the word processor, **Ctrl+?** shows the full list. In the spreadsheet window, anything without a shortcut is in the command palette (**Ctrl+K**); in the word processor, it is in the main menu.

| **Keys** | **Spreadsheet** | **Text document** |
| --- | --- | --- |
| Ctrl+N | New document | New document |
| Ctrl+O | Open | Open |
| Ctrl+S | Save | Save |
| Ctrl+Shift+S |  | Save as |
| Ctrl+Z | Undo | Undo |
| Ctrl+Shift+Z | Redo | Redo |
| Ctrl+F | Find | Find |
| Ctrl+G |  | Go to an address |
| Ctrl+K | Command palette | Insert a link |
| Ctrl+L |  | List item |
| Ctrl+C, Ctrl+X, Ctrl+V | Copy, cut, paste | Copy, cut, paste |
| Ctrl+D, Ctrl+R | Fill down, fill right |  |
| Ctrl+B, Ctrl+I, Ctrl+U | Bold, italic, underline | Bold, italic, underline |
| Ctrl+0 | Reset zoom | Paragraph |
| Ctrl+1 to Ctrl+3 |  | Headings 1 to 3 |
| Ctrl+Shift+T |  | Insert a table |
| Ctrl+Shift+I |  | Insert a picture |
| Ctrl+Shift+O |  | Outline |
| Ctrl+Shift+U | Show the source | Show the source |
| Ctrl+Alt+S |  | Save as Markdown |
| Ctrl+Shift+E |  | Export as PDF |
| Ctrl+Shift+P |  | Print preview |
| Ctrl+P |  | Print |
| F2 | Edit the cell |  |
| F8 | Check the document | Check the document |
| F9 | Recalculate |  |
| Shift+F7 | Spelling on and off |  |
| Ctrl+? |  | List of shortcuts |

# Appendix B: Glossary

**Block.** A paragraph, heading or list item in a text document.

**Bookmark.** A name for a place in a text document, which keeps pointing at the same place when you edit above it.

**Cell.** One box of a spreadsheet, named by its column and row, such as B3.

**Command palette.** The searchable list of every command, opened with Ctrl+K.

**Flat file.** An ODF document saved as one plain text file: `.fods` or `.fodt`.

**Formula.** A cell entry that starts with `=` and calculates a value.

**Lint.** An automatic check of a document for mistakes.

**Locale.** The conventions a document follows for writing numbers, such as which character is the decimal point.

**ODF.** The OpenDocument Format, the open standard grind uses for its files.

**Package.** An ODF document saved as a compressed archive: `.ods` or `.odt`.

**Range.** A rectangle of cells, named by two corners, such as B2:B7.

**Template.** A document that provides the page, the letterhead and the styles for other documents.
