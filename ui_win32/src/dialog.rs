// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Every modal this shell opens: the file dialogs, and every dialog it draws itself — the close
//! question, a message, a prompt, a list, the filter, the chart and print previews.
//!
//! **Windows only, and every function here runs a nested message loop.** That is the whole
//! reason they are gathered in one file: `doc/windows-shell.md`'s decision 7 is a rule about
//! *callers* — a handler that opens a modal borrows the window's state briefly on each side of
//! the dialog and **never across it** — and a rule is easier to keep when the things it applies
//! to are in one place with the warning written on them.
//!
//! What goes wrong otherwise is not a hang or a crash: while a modal is up this window still
//! receives `WM_PAINT`, the window procedure is re-entered, and a second `&mut State` is produced
//! while the first is alive. That is aliasing UB even though it appears to work, and reading the
//! code does not reveal it — in the sibling repository it was found by driving the shell under
//! Wine.
//!
//! The file dialogs are `IFileDialog` (COM, Vista and later) rather than `GetOpenFileNameW`,
//! because it is the modern dialog and needs no application manifest to be one. Its filters
//! follow `doc/flat-first.md`, the same as both GTK shells' `*_filters`/`*_save_filters`:
//! **Open offers one combined filter and prefers neither form** — a user looking for a document
//! does not know which physical form it is in, and this window shows either document kind so the
//! filter combines both kinds' extensions rather than making the user guess before they have even
//! seen the file. **Save leads with flat** — `.fods`/`.fodt` first, then the package, then
//! `.grind` — because in doubt this project writes the form that diffs, and it offers only the
//! kind of the pane that is open: a spreadsheet suggests `.fods`, a text document `.fodt`.
//! Nothing here decides what a form *is*: `grind_sheet::write_file`/`grind_text::write_file` read
//! the extension the user chose (`Form::from_path`), which is the one place in the workspace
//! where an extension decides anything.

#![cfg(windows)]

use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoTaskMemFree, CoUninitialize,
};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FileOpenDialog, FileSaveDialog, IFileOpenDialog, IFileSaveDialog, SIGDN_FILESYSPATH,
};
use windows::core::{HSTRING, PCWSTR, w};

use grind_core::{DocumentKind, Form};

use crate::gdi;

/// Start COM for this thread, for the life of the value.
///
/// `IFileDialog` is a COM object and needs an apartment; an application that never opens one
/// still pays nothing, because this is created once in `win::run` and dropped when the message
/// loop ends. Apartment-threaded because that is what the shell dialogs want.
pub struct Com;

impl Com {
    pub fn new() -> Self {
        // SAFETY: no arguments to outlive the call, and `CoUninitialize` is paired in `Drop`.
        // A failure here (COM already initialised with a different model, say) is not a reason
        // to refuse to start — the file dialogs would simply not open.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
        Self
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        // SAFETY: paired with the `CoInitializeEx` above, on the same thread.
        unsafe { CoUninitialize() }
    }
}

/// The three answers to "this document has unsaved changes".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Save,
    Discard,
    Cancel,
}

/// The plain-language name a filter uses for a document kind, matching what the GTK shells'
/// `spreadsheet_filters`/`text_filters` call theirs.
fn kind_label(kind: DocumentKind) -> &'static str {
    match kind {
        DocumentKind::Spreadsheet => "OpenDocument Spreadsheet",
        DocumentKind::Text => "OpenDocument Text",
        DocumentKind::Presentation => "OpenDocument Presentation",
    }
}

/// The Open dialog's filter: **one, matching both document kinds this window can show**, and
/// both physical forms plus the projection within each — `doc/flat-first.md`'s "one filter
/// matching both \[forms\]" extended over kinds too, since a single window here can hold either.
/// A user opening a `.fodt` should see it without switching away from whatever filter greeted
/// them, the same reason `ui_sheet_gtk`'s and `ui_text_gtk`'s own `*_filters` are one filter
/// each rather than a leading "All spreadsheets" plus a per-extension breakdown.
///
/// Held as owned UTF-16 by the caller, because `COMDLG_FILTERSPEC` is two borrowed pointers and
/// the dialog reads them after `SetFileTypes` returns.
fn open_filters() -> Vec<(Vec<u16>, Vec<u16>)> {
    let mut filters = vec![(
        gdi::wide("OpenDocument Spreadsheet or Text"),
        gdi::wide("*.fods;*.ods;*.fodt;*.odt;*.grind"),
    )];
    // A workbook is imported as a new document (`import.rs`, X6), so it is a filter of its own
    // under the name a person knows it by — second, so the ODF documents still greet the user.
    if cfg!(feature = "xlsx") {
        filters.push((gdi::wide("Excel Workbook"), gdi::wide("*.xlsx;*.xlsm")));
    }
    // A Word document, imported the same way as a text document (`doc/docx-import.md`).
    if cfg!(feature = "docx") {
        filters.push((
            gdi::wide("Word Document"),
            gdi::wide("*.docx;*.docm;*.dotx"),
        ));
    }
    // A CSV opened here is a document of its own, like a workbook; *Import CSV* on the Data
    // menu is the one that puts the fields into the sheet already open.
    filters.push((gdi::wide("CSV and TSV"), gdi::wide("*.csv;*.tsv;*.tab")));
    // And markdown, opened the same way as a text document (`grind_text::commonmark::open`).
    filters.push((gdi::wide("Markdown"), gdi::wide("*.md;*.markdown")));
    filters
}

/// The Save dialog's filters, for whichever `kind` the open pane is. **Flat first**
/// (`doc/flat-first.md`): in doubt this project writes the form that diffs, so `.fods`/`.fodt`
/// leads, then the package, then `.grind` — the same order and the same three entries as
/// `ui_sheet_gtk`'s `spreadsheet_save_filters` and `ui_text_gtk`'s `text_save_filters`, and the
/// extensions come from [`Form::extension`] so this list cannot name one the writer disagrees
/// with.
fn save_filters(kind: DocumentKind) -> Vec<(Vec<u16>, Vec<u16>)> {
    let label = kind_label(kind);
    vec![
        (
            gdi::wide(&format!("{label} (flat XML)")),
            gdi::wide(&format!("*.{}", Form::Flat.extension(kind))),
        ),
        (
            gdi::wide(&format!("{label} (package)")),
            gdi::wide(&format!("*.{}", Form::Package.extension(kind))),
        ),
        (
            gdi::wide("Grind projection"),
            gdi::wide(&format!("*.{}", Form::Projection.extension(kind))),
        ),
    ]
}

fn specs(filters: &[(Vec<u16>, Vec<u16>)]) -> Vec<COMDLG_FILTERSPEC> {
    filters
        .iter()
        .map(|(name, spec)| COMDLG_FILTERSPEC {
            pszName: PCWSTR(name.as_ptr()),
            pszSpec: PCWSTR(spec.as_ptr()),
        })
        .collect()
}

/// An `IShellItem`'s path, as a Rust string.
///
/// `GetDisplayName` allocates with the COM task allocator, so the answer has to be freed with
/// `CoTaskMemFree` — the leak this function exists to make impossible to forget.
fn item_path(item: &windows::Win32::UI::Shell::IShellItem) -> Option<PathBuf> {
    // SAFETY: the item is live; the returned pointer is owned by this call and freed below.
    unsafe {
        let wide = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = wide.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(wide.0.cast()));
        path
    }
}

/// Ask for a document to open. `None` means the user cancelled.
pub fn open_path(owner: HWND) -> Option<PathBuf> {
    let filters = open_filters();
    let specs = specs(&filters);
    // SAFETY: every buffer outlives the dialog, which is modal. **A nested message loop.**
    unsafe {
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let _ = dialog.SetFileTypes(&specs);
        let _ = dialog.SetTitle(PCWSTR(gdi::wide("Open").as_ptr()));
        dialog.Show(Some(owner)).ok()?;
        item_path(&dialog.GetResult().ok()?)
    }
}

/// Insert Picture's own filter — the same extensions `ui_text_gtk`'s `image_filters` offers,
/// since both reach the same `image.rs`/WIC (this shell) or gdk-pixbuf (that one) decoders.
fn image_filters() -> Vec<(Vec<u16>, Vec<u16>)> {
    vec![(
        gdi::wide("Images"),
        gdi::wide("*.png;*.jpg;*.jpeg;*.gif;*.webp;*.bmp;*.tif;*.tiff"),
    )]
}

/// Ask for a picture to insert. `None` means the user cancelled.
pub fn open_image_path(owner: HWND) -> Option<PathBuf> {
    let filters = image_filters();
    let specs = specs(&filters);
    // SAFETY: every buffer outlives the dialog, which is modal. **A nested message loop.**
    unsafe {
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let _ = dialog.SetFileTypes(&specs);
        let _ = dialog.SetTitle(PCWSTR(gdi::wide("Insert Picture").as_ptr()));
        dialog.Show(Some(owner)).ok()?;
        item_path(&dialog.GetResult().ok()?)
    }
}

/// What an import will open: both spellings of the one format, plus `.txt`, since that is what
/// a great many exports are called and the delimiter is read from the content rather than from
/// the name. The same list `ui_sheet_gtk`'s `csv_filters` offers.
fn csv_filters() -> Vec<(Vec<u16>, Vec<u16>)> {
    vec![(
        gdi::wide("Delimited text"),
        gdi::wide("*.csv;*.tsv;*.tab;*.txt"),
    )]
}

/// Ask for a delimited file to import. `None` means the user cancelled.
pub fn open_csv_path(owner: HWND) -> Option<PathBuf> {
    let filters = csv_filters();
    let specs = specs(&filters);
    // SAFETY: every buffer outlives the dialog, which is modal. **A nested message loop.**
    unsafe {
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let _ = dialog.SetFileTypes(&specs);
        let _ = dialog.SetTitle(PCWSTR(gdi::wide("Import CSV").as_ptr()));
        dialog.Show(Some(owner)).ok()?;
        item_path(&dialog.GetResult().ok()?)
    }
}

/// Ask where to write one out.
///
/// The **name** is what says which delimiter (`csv::Dialect::for_name`), so these two entries
/// are the whole of that choice — the same pair `ui_sheet_gtk`'s save dialog offers, and what
/// the browser shell spends two palette rows on because a download names itself.
pub fn save_csv_path(owner: HWND, suggested: &str) -> Option<PathBuf> {
    let filters = vec![
        (gdi::wide("Comma-separated values"), gdi::wide("*.csv")),
        (gdi::wide("Tab-separated values"), gdi::wide("*.tsv")),
    ];
    let specs = specs(&filters);
    let name = gdi::wide(suggested);
    let extension = gdi::wide("csv");
    // SAFETY: every buffer outlives the dialog, which is modal. **A nested message loop.**
    unsafe {
        let dialog: IFileSaveDialog =
            CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let _ = dialog.SetFileTypes(&specs);
        // 1-based, and 1 is the comma — it is what the verb is called.
        let _ = dialog.SetFileTypeIndex(1);
        let _ = dialog.SetDefaultExtension(PCWSTR(extension.as_ptr()));
        let _ = dialog.SetTitle(PCWSTR(gdi::wide("Export CSV").as_ptr()));
        let _ = dialog.SetFileName(PCWSTR(name.as_ptr()));
        dialog.Show(Some(owner)).ok()?;
        item_path(&dialog.GetResult().ok()?)
    }
}

/// Ask for a markdown file to read in. `None` means the user cancelled.
pub fn open_markdown_path(owner: HWND) -> Option<PathBuf> {
    let filters = vec![(
        gdi::wide("Markdown"),
        gdi::wide("*.md;*.markdown;*.mdown;*.txt"),
    )];
    let specs = specs(&filters);
    // SAFETY: every buffer outlives the dialog, which is modal. **A nested message loop.**
    unsafe {
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let _ = dialog.SetFileTypes(&specs);
        let _ = dialog.SetTitle(PCWSTR(gdi::wide("Import Markdown").as_ptr()));
        dialog.Show(Some(owner)).ok()?;
        item_path(&dialog.GetResult().ok()?)
    }
}

/// Ask where to write markdown out.
pub fn save_markdown_path(owner: HWND, suggested: &str) -> Option<PathBuf> {
    save_one_type(
        owner,
        suggested,
        ("Markdown", "*.md", "md"),
        "Export Markdown",
    )
}

/// Ask where to write a PDF (`doc/pdf-export.md`).
pub fn save_pdf_path(owner: HWND, suggested: &str) -> Option<PathBuf> {
    save_one_type(owner, suggested, ("PDF", "*.pdf", "pdf"), "Export PDF")
}

/// A save dialog for one file type: its name, its pattern and the extension it defaults to.
fn save_one_type(
    owner: HWND,
    suggested: &str,
    (kind, pattern, extension): (&str, &str, &str),
    title: &str,
) -> Option<PathBuf> {
    let filters = vec![(gdi::wide(kind), gdi::wide(pattern))];
    let specs = specs(&filters);
    let name = gdi::wide(suggested);
    let extension = gdi::wide(extension);
    let title = gdi::wide(title);
    // SAFETY: every buffer outlives the dialog, which is modal. **A nested message loop.**
    unsafe {
        let dialog: IFileSaveDialog =
            CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let _ = dialog.SetFileTypes(&specs);
        let _ = dialog.SetDefaultExtension(PCWSTR(extension.as_ptr()));
        let _ = dialog.SetTitle(PCWSTR(title.as_ptr()));
        let _ = dialog.SetFileName(PCWSTR(name.as_ptr()));
        dialog.Show(Some(owner)).ok()?;
        item_path(&dialog.GetResult().ok()?)
    }
}

/// Ask where to save. `suggested` seeds the name and the folder; `kind` is which document type
/// the open pane holds, so a text document suggests `.fodt` rather than always `.fods`.
pub fn save_path(owner: HWND, suggested: Option<&Path>, kind: DocumentKind) -> Option<PathBuf> {
    let filters = save_filters(kind);
    let specs = specs(&filters);
    let flat_extension = Form::Flat.extension(kind);
    let name = suggested
        .and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| format!("Untitled.{flat_extension}"));
    let name = gdi::wide(&name);
    let extension = gdi::wide(flat_extension);
    // SAFETY: every buffer outlives the dialog, which is modal. **A nested message loop.**
    unsafe {
        let dialog: IFileSaveDialog =
            CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let _ = dialog.SetFileTypes(&specs);
        // 1-based, and 1 is the flat form: `doc/flat-first.md`'s default made the dialog's.
        let _ = dialog.SetFileTypeIndex(1);
        let _ = dialog.SetDefaultExtension(PCWSTR(extension.as_ptr()));
        let _ = dialog.SetFileName(PCWSTR(name.as_ptr()));
        dialog.Show(Some(owner)).ok()?;
        item_path(&dialog.GetResult().ok()?)
    }
}

// ---------------------------------------------------------------------------
// The modal — every dialog this shell draws itself
// ---------------------------------------------------------------------------

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

use windows::Win32::Foundation::{HANDLE, RECT};
use windows::Win32::Graphics::Dwm::{
    DWMWA_BORDER_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE,
    DWMWCP_ROUND, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    BLACK_BRUSH, BeginPaint, EndPaint, GetMonitorInfoW, GetStockObject, HBRUSH, HDC,
    InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow, PAINTSTRUCT,
    SetBkColor, SetBkMode, SetTextColor, SetViewportOrgEx, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{
    DRAWITEMSTRUCT, EM_SETLIMITTEXT, EM_SETSEL, MEASUREITEMSTRUCT, ODS_DISABLED, ODS_FOCUS,
    ODS_NOFOCUSRECT, ODS_SELECTED, ODT_BUTTON, ODT_LISTBOX, SetWindowTheme, WM_MOUSELEAVE,
};
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetFocus, IsWindowEnabled, SetActiveWindow, SetFocus, TME_LEAVE, TRACKMOUSEEVENT,
    TrackMouseEvent, VK_END, VK_HOME, VK_LEFT, VK_NEXT, VK_PRIOR, VK_RIGHT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BS_OWNERDRAW, CREATESTRUCTW, CS_DROPSHADOW, CallWindowProcW, CreateWindowExW, DLGC_BUTTON,
    DLGC_DEFPUSHBUTTON, DLGC_UNDEFPUSHBUTTON, DefWindowProcW, DestroyWindow, DispatchMessageW,
    EN_KILLFOCUS, EN_SETFOCUS, ES_AUTOHSCROLL, ES_AUTOVSCROLL, ES_MULTILINE, ES_READONLY,
    GWLP_USERDATA, GWLP_WNDPROC, GetClientRect, GetMessageW, GetPropW, GetWindowLongPtrW,
    GetWindowRect, GetWindowTextLengthW, GetWindowTextW, HMENU, HTCLIENT, IDC_ARROW, IDCANCEL,
    IDNO, IDOK, IDYES, IsDialogMessageW, LAYERED_WINDOW_ATTRIBUTES_FLAGS, LB_ADDSTRING,
    LB_GETCOUNT, LB_GETCURSEL, LB_GETITEMRECT, LB_GETSEL, LB_ITEMFROMPOINT, LB_SETCURSEL,
    LB_SETSEL, LBN_DBLCLK, LBS_HASSTRINGS, LBS_MULTIPLESEL, LBS_NOINTEGRALHEIGHT, LBS_NOTIFY,
    LBS_OWNERDRAWFIXED, LWA_ALPHA, LoadCursorW, MSG, PostQuitMessage, RegisterClassW, RemovePropW,
    SM_CXVSCROLL, SW_SHOW, SW_SHOWNOACTIVATE, SendMessageW, SetLayeredWindowAttributes, SetPropW,
    SetWindowLongPtrW, SetWindowTextW, ShowWindow, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_CLOSE, WM_COMMAND, WM_CTLCOLOREDIT, WM_CTLCOLORLISTBOX, WM_CTLCOLORSTATIC, WM_DRAWITEM,
    WM_ERASEBKGND, WM_GETDLGCODE, WM_KEYDOWN, WM_MEASUREITEM, WM_MOUSEMOVE, WM_NCACTIVATE,
    WM_NCCALCSIZE, WM_NCCREATE, WM_NCDESTROY, WM_NCHITTEST, WM_PAINT, WM_SETFONT, WM_USER,
    WNDCLASSW, WS_CAPTION, WS_CHILD, WS_CLIPCHILDREN, WS_DISABLED, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_POPUP, WS_TABSTOP, WS_THICKFRAME, WS_VISIBLE, WS_VSCROLL,
};

use crate::gdi::{Across, Brush, Font, Measure, Selected};
use crate::modal::{self, Edges, End};
use crate::sheet::geom::{Rect, scale};
use crate::theme::{Interaction, Rgb, Theme};

/// The palette every modal in this file paints itself in.
///
/// **A static, and the one in this crate.** Everywhere else the rule `theme.rs` states holds —
/// the drawing code is handed a palette and cannot reach past it — and this is the exception the
/// rule's own reason allows: a modal here is opened at a point where the caller has deliberately
/// *released* its borrow of the pane (decision 7 forbids holding one across a nested message
/// loop), so there is no `&Pane` to read a theme out of at the call site, and threading one
/// through every caller would put a parameter on each of them purely to work around that. The
/// window sets it when it learns the theme and again on `WM_SETTINGCHANGE`; there is one UI
/// thread, and a modal that opened before the first call gets the light palette, which is also
/// what a machine that has said nothing gets.
static THEME: std::sync::Mutex<Option<Theme>> = std::sync::Mutex::new(None);

/// Tell this module which palette to paint in — called by `win.rs` whenever the theme changes.
pub fn use_theme(theme: Theme) {
    if let Ok(mut slot) = THEME.lock() {
        *slot = Some(theme);
    }
}

fn theme() -> Theme {
    THEME
        .lock()
        .ok()
        .and_then(|slot| *slot)
        .unwrap_or_else(|| Theme::of(crate::theme::Mode::Light))
}

/// `DM_GETDEFID`, and the marker its answer carries — the dialog manager's own question,
/// "which button does Enter mean?", which `IsDialogMessageW` asks of any window it is pumping for
/// and not only of a dialog.
const DM_GETDEFID: u32 = WM_USER;
const DC_HASDEFID: isize = 0x534b;

/// The ids of the answers that are not the dialog manager's own two.
const ID_CLEAR: i32 = 11;
const ID_PREVIOUS: i32 = 201;
const ID_NEXT: i32 = 202;
/// The first of a chart preview's kind buttons; the rest follow in the order they were given.
const ID_KIND: i32 = 300;

const ID_EDIT: usize = 10;
const ID_LIST: usize = 12;

/// What a button in the footer is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    /// The answer Enter gives — drawn in the accent, which is ContentDialog's *primary* button.
    /// One per modal, and **always the safe one** where an answer destroys something: a dialog
    /// whose accent button deletes is a dialog that deletes when somebody presses Enter twice.
    Default,
    /// Any other answer.
    Answer,
    /// Not an answer, at the footer's leading end — *Previous*, *Next*, *Clear*.
    Leading,
    /// One of a group of which one is chosen — a chart's kinds. Leading, and drawn checked.
    Choice,
}

impl Role {
    fn end(self) -> End {
        match self {
            Self::Default | Self::Answer => End::Trailing,
            Self::Leading | Self::Choice => End::Leading,
        }
    }
}

struct Button {
    id: i32,
    label: String,
    role: Role,
    hwnd: HWND,
}

/// One page of the print preview, rasterised at a scale: its width and height in pixels, and
/// premultiplied RGBA.
pub type Rendered = (u32, u32, Vec<u8>);

/// What is between a modal's title and its footer.
enum Body {
    /// A sentence or two — what `MessageBoxW` used to say.
    Message(String),
    /// A label and one line of text to type.
    Field { label: String, initial: String },
    /// A label and a long text to read, which scrolls — the licences.
    Reader { label: String, text: String },
    /// Rows to pick one of, or — `multi` — to tick any of.
    List {
        rows: Vec<String>,
        initial: usize,
        multi: Vec<bool>,
        is_multi: bool,
    },
    /// A chart, drawn as whichever of its kinds is chosen.
    Chart {
        pictures: Vec<(
            grind_sheet::ChartKind,
            grind_sheet::Chart,
            grind_sheet::ChartData,
        )>,
        shown: usize,
        dpi: u32,
    },
    /// A document's pages as they print.
    Pages(Pages),
}

/// The print preview's own state (`doc/pdf-export.md` §4).
struct Pages {
    count: usize,
    page: usize,
    /// The page's size in points, for fitting it.
    size: (f32, f32),
    render: Box<dyn Fn(usize, f32) -> Option<Rendered>>,
    label: Box<dyn Fn(usize) -> String>,
    dpi: u32,
    /// The page last drawn, at the scale it was drawn at, already in GDI's channel order — so a
    /// repaint that changed nothing rasterises nothing.
    cache: Option<(usize, u32, u32, u32, Vec<u8>)>,
}

/// What a modal owns while it is up.
struct Modal {
    title: String,
    body: Body,
    buttons: Vec<Button>,
    theme: Theme,
    dpi: u32,
    body_font: Font,
    title_font: Font,
    /// The edit or the list the body holds, if it holds one.
    control: HWND,
    /// Where the body's field, reader or list is framed — the control sits inside it.
    well: RECT,
    /// Whether the field has the keyboard — its frame wears the accent when it does.
    focused: bool,
    /// The two grounds a control in the body can stand on, kept alive for `WM_CTLCOLOR…`: a
    /// brush handed to Windows has to outlive the repaint it is handed for.
    rest: Brush,
    active: Brush,
    /// The id of the button that ended the modal, or `IDCANCEL` for Escape and closing.
    answer: Option<i32>,
    finished: bool,
}

impl Modal {
    fn px(&self, value: f64) -> i32 {
        scale(value, self.dpi).round() as i32
    }

    fn default_id(&self) -> i32 {
        self.buttons
            .iter()
            .find(|button| button.role == Role::Default)
            .map_or(IDCANCEL.0, |button| button.id)
    }

    /// Whether the button with this id ends the modal rather than changing what it shows.
    fn ends(&self, id: i32) -> bool {
        id == IDCANCEL.0
            || self.buttons.iter().any(|button| {
                button.id == id && matches!(button.role, Role::Default | Role::Answer)
            })
            || id == ID_CLEAR
    }
}

/// Everything a caller says about a modal before it opens.
struct Spec<'a> {
    owner: HWND,
    title: &'a str,
    body: Body,
    /// Id, label, role — in the order they read, leading ones first.
    buttons: Vec<(i32, &'a str, Role)>,
}

const MODAL_CLASS: &str = "GrindModalClass";
const SMOKE_CLASS: &str = "GrindSmokeClass";
static MODAL_REGISTERED: AtomicBool = AtomicBool::new(false);
static SMOKE_REGISTERED: AtomicBool = AtomicBool::new(false);

/// The window procedures the two subclassed control classes had, kept once: every `BUTTON` in a
/// process shares one, and so does every `LISTBOX`.
static BUTTON_PROC: AtomicIsize = AtomicIsize::new(0);
static LIST_PROC: AtomicIsize = AtomicIsize::new(0);

/// The property a subclassed control keeps its pointer state in: 1 + the hovered row for a list,
/// 1 for a hovered button, absent for neither.
const HOVER: &str = "GrindHover";

fn register(
    name: &str,
    proc: windows::Win32::UI::WindowsAndMessaging::WNDPROC,
    flag: &AtomicBool,
    smoke: bool,
) -> bool {
    if flag.swap(true, Ordering::SeqCst) {
        return true;
    }
    let class = gdi::wide(name);
    // SAFETY: the class name outlives the call; the stock brush is never freed.
    unsafe {
        let Ok(instance) = GetModuleHandleW(None) else {
            flag.store(false, Ordering::SeqCst);
            return false;
        };
        let wc = WNDCLASSW {
            // The drop shadow is the one a menu has, and it is what lets a caption-less surface
            // read as lifted off the window behind it where the compositor draws no shadow of its
            // own (Windows 10, a remote session).
            style: match smoke {
                true => Default::default(),
                false => CS_DROPSHADOW,
            },
            lpfnWndProc: proc,
            hInstance: instance.into(),
            lpszClassName: PCWSTR(class.as_ptr()),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            // The smoke is nothing but its ground, and black is a stock object — not created, so
            // never freed. The modal paints itself, so a class brush would only flash.
            hbrBackground: match smoke {
                true => HBRUSH(GetStockObject(BLACK_BRUSH).0),
                false => HBRUSH::default(),
            },
            ..Default::default()
        };
        if RegisterClassW(&wc) == 0 {
            flag.store(false, Ordering::SeqCst);
            return false;
        }
    }
    true
}

/// The work area of the monitor `hwnd` is on, in screen pixels.
fn work_area(hwnd: HWND) -> Edges {
    // SAFETY: `info` is a live local sized as the call requires.
    unsafe {
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = GetMonitorInfoW(monitor, &mut info);
        edges(info.rcWork)
    }
}

fn edges(rect: RECT) -> Edges {
    Edges {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    }
}

fn rect_of(rect: Rect) -> RECT {
    let (left, top, right, bottom) = rect.edges();
    RECT {
        left,
        top,
        right,
        bottom,
    }
}

/// The smoke over the owner while a modal is up — Fluent's *SmokeFillColorDefault*, black at
/// 30%, which is what says "the window behind this is waiting" more plainly than a disabled
/// window does on its own. Destroyed when the guard is, so no path out of a modal leaves it.
struct Smoke(HWND);

impl Smoke {
    fn over(owner: HWND) -> Self {
        // Wine draws a layered window's alpha only where a compositor is running, and opaque
        // black where none is — which would hide the window rather than dim it.
        if under_wine() || !register(SMOKE_CLASS, Some(smoke_proc), &SMOKE_REGISTERED, true) {
            return Self(HWND::default());
        }
        let class = gdi::wide(SMOKE_CLASS);
        // SAFETY: the class name outlives the call; the window is destroyed in `Drop`.
        unsafe {
            let mut client = RECT::default();
            let _ = GetClientRect(owner, &mut client);
            let mut origin = windows::Win32::Foundation::POINT::default();
            let _ = windows::Win32::Graphics::Gdi::ClientToScreen(owner, &mut origin);
            let Ok(instance) = GetModuleHandleW(None) else {
                return Self(HWND::default());
            };
            let Ok(smoke) = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                PCWSTR(class.as_ptr()),
                PCWSTR::null(),
                WS_POPUP | WS_DISABLED,
                origin.x,
                origin.y,
                client.right - client.left,
                client.bottom - client.top,
                Some(owner),
                None::<HMENU>,
                Some(instance.into()),
                None,
            ) else {
                return Self(HWND::default());
            };
            let _ = SetLayeredWindowAttributes(
                smoke,
                COLORREF(0),
                0x4d,
                LAYERED_WINDOW_ATTRIBUTES_FLAGS(LWA_ALPHA.0),
            );
            let _ = ShowWindow(smoke, SW_SHOWNOACTIVATE);
            Self(smoke)
        }
    }
}

impl Drop for Smoke {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: the window is this guard's own.
            unsafe {
                let _ = DestroyWindow(self.0);
            }
        }
    }
}

/// Whether this process is running under Wine rather than Windows — `ntdll` exports
/// `wine_get_version` there and nowhere else, which is the check Wine itself documents for this.
fn under_wine() -> bool {
    use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    // SAFETY: looking a symbol up in a module every process has loaded.
    unsafe {
        GetModuleHandleW(w!("ntdll.dll"))
            .ok()
            .and_then(|ntdll| GetProcAddress(ntdll, windows::core::s!("wine_get_version")))
            .is_some()
    }
}

extern "system" fn smoke_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        // Never the target of a click — the owner is disabled anyway, and the modal is above.
        WM_NCHITTEST => LRESULT(-1), // HTTRANSPARENT
        // SAFETY: the default handler with the arguments it was given.
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

/// Open a modal and pump messages until it is answered; then ask `read` what it says, while its
/// controls still exist, and close it.
///
/// **A nested message loop**, like everything else in this file — the caller has released its
/// borrow. The owner is disabled for exactly as long as the modal is up and re-enabled *before*
/// the modal is destroyed, or Windows activates some other application instead.
fn run<T>(spec: Spec<'_>, read: impl FnOnce(&mut Modal) -> T) -> Option<T> {
    if !register(MODAL_CLASS, Some(modal_proc), &MODAL_REGISTERED, false) {
        return None;
    }
    let owner = spec.owner;
    // SAFETY: `owner` is a live window; everything below is handed only buffers that outlive it.
    let dpi = unsafe { GetDpiForWindow(owner) }.max(96);
    let px = |value: f64| scale(value, dpi).round() as i32;
    let theme = theme();
    let body_font = Font::new(gdi::ui_face(), px(crate::theme::text::BODY), false);
    let title_font = Font::weighted(gdi::display_face(), px(modal::size::TITLE), 600);
    let titled = !spec.title.is_empty();
    let work = work_area(owner);

    // How big the content is — measured in the fonts it will be drawn in, before there is a
    // window, since the window's size is decided from it.
    let content = {
        let measure = Measure::new(&body_font)?;
        let s = |v: f64| scale(v, dpi);
        let label_line = s(20.0) + s(modal::size::LABEL_GAP);
        match &spec.body {
            Body::Message(text) => {
                let widest = text
                    .lines()
                    .map(|line| f64::from(measure.width(line)))
                    .fold(0.0, f64::max);
                let w = widest.clamp(
                    s(modal::size::MIN_W - modal::size::PAD * 2.0),
                    s(modal::size::MAX_W - modal::size::PAD * 2.0),
                );
                (w, f64::from(measure.height(text, w.round() as i32)))
            }
            Body::Field { label, .. } => (
                (f64::from(measure.width(label))).max(s(352.0)),
                label_line + s(modal::size::CONTROL_H),
            ),
            Body::Reader { .. } => (s(600.0), s(440.0)),
            Body::List { rows, is_multi, .. } => {
                let widest = rows
                    .iter()
                    .map(|row| match row.split_once('\t') {
                        Some((left, right)) => {
                            f64::from(measure.width(left) + measure.width(right)) + s(48.0)
                        }
                        None => f64::from(measure.width(row)),
                    })
                    .fold(0.0, f64::max);
                // SAFETY: a metric lookup.
                let bar = f64::from(unsafe { GetSystemMetricsForDpi(SM_CXVSCROLL, dpi) });
                let chrome = s(12.0) * 2.0 + s(8.0) + bar + if *is_multi { s(28.0) } else { 0.0 };
                modal::list_body(rows.len(), widest, chrome, dpi)
            }
            Body::Chart { .. } => (s(520.0), s(320.0)),
            Body::Pages(pages) => {
                // As tall as the screen allows, and as wide as the page is at that height.
                let chrome = f64::from(modal::wanted((0.0, 0.0), dpi, titled).1);
                let tall = (f64::from(work.height()) - s(modal::size::MARGIN) * 2.0 - chrome)
                    .max(s(240.0));
                let inset = s(12.0) * 2.0;
                let (pw, ph) = (f64::from(pages.size.0), f64::from(pages.size.1.max(1.0)));
                (((tall - inset) * pw / ph + inset).max(s(420.0)), tall)
            }
        }
    };
    let (natural, between): (Vec<f64>, f64) = {
        let measure = Measure::new(&body_font)?;
        let natural = spec
            .buttons
            .iter()
            .map(|(_, label, _)| f64::from(measure.width(label)))
            .collect();
        // The print preview's *Page 2 of 5* sits between the footer's two ends, and the footer
        // has to be wide enough to hold it — the last page's label is the longest.
        let between = match &spec.body {
            Body::Pages(pages) => {
                f64::from(measure.width(&(pages.label)(pages.count.max(1) - 1)))
                    + scale(modal::size::PAD, dpi)
            }
            _ => 0.0,
        };
        (natural, between)
    };
    let want = modal::wanted(content, dpi, titled);
    // Wide enough for its buttons too, or the footer falls back to columns for no reason.
    let footer_want = {
        let s = |v: f64| scale(v, dpi);
        let sum: f64 = natural
            .iter()
            .zip(&spec.buttons)
            .map(|(w, (_, _, role))| match role.end() {
                End::Trailing => {
                    (w + s(modal::size::BUTTON_PAD) * 2.0).max(s(modal::size::BUTTON_MIN))
                }
                End::Leading => w + s(modal::size::BUTTON_PAD) * 2.0,
            })
            .sum();
        (sum + s(modal::size::BUTTON_GAP) * (spec.buttons.len() as f64 + 1.0)
            + s(modal::size::PAD) * 2.0
            + between)
            .ceil() as i32
    };
    let want = (want.0.max(footer_want), want.1);
    let mut owner_rect = RECT::default();
    // SAFETY: a live local.
    unsafe {
        let _ = GetWindowRect(owner, &mut owner_rect);
    }
    let placed = modal::place(want, edges(owner_rect), work, dpi);

    let state = Box::new(Modal {
        title: spec.title.to_owned(),
        body: spec.body,
        buttons: spec
            .buttons
            .iter()
            .map(|(id, label, role)| Button {
                id: *id,
                label: (*label).to_owned(),
                role: *role,
                hwnd: HWND::default(),
            })
            .collect(),
        theme,
        dpi,
        body_font,
        title_font,
        control: HWND::default(),
        well: RECT::default(),
        focused: false,
        rest: Brush::solid(theme.card),
        active: Brush::solid(theme.background),
        answer: None,
        finished: false,
    });

    let smoke = Smoke::over(owner);
    let class = gdi::wide(MODAL_CLASS);
    let title = gdi::wide(spec.title);
    // SAFETY: the class name and title outlive the call; the boxed state is handed to the
    // window and taken back in `WM_NCDESTROY`.
    let popup = unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        // A caption and a thick frame that are never drawn — `WM_NCCALCSIZE` gives the whole
        // window to the client — because they are what makes the compositor treat the surface
        // as a window: its shadow, and Windows 11's rounded corner and border.
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            PCWSTR(class.as_ptr()),
            PCWSTR(title.as_ptr()),
            WS_POPUP | WS_CAPTION | WS_THICKFRAME | WS_CLIPCHILDREN,
            placed.left,
            placed.top,
            placed.width(),
            placed.height(),
            Some(owner),
            None::<HMENU>,
            Some(instance.into()),
            Some(Box::into_raw(state).cast()),
        )
        .ok()?
    };
    chrome(popup, theme);
    create_children(popup, &natural, titled);

    // SAFETY: the modal loop — see the function's comment for the ordering it keeps.
    unsafe {
        let _ = EnableWindow(owner, false);
        let _ = ShowWindow(popup, SW_SHOW);
        let focus = with_modal(popup, |modal| match modal.control.is_invalid() {
            false => modal.control,
            true => modal
                .buttons
                .iter()
                .find(|button| button.role == Role::Default)
                .map_or(HWND::default(), |button| button.hwnd),
        });
        if let Some(focus) = focus.filter(|hwnd| !hwnd.is_invalid()) {
            let _ = SetFocus(Some(focus));
        }

        let mut message = MSG::default();
        loop {
            if with_modal(popup, |modal| modal.finished).unwrap_or(true) {
                break;
            }
            let got = GetMessageW(&mut message, None, 0, 0).0;
            if got <= 0 {
                // `WM_QUIT` arrived while a modal was up — the application is closing. Put it
                // back so the outer loop sees it too, and stop.
                PostQuitMessage(0);
                break;
            }
            // The print preview turns pages from the keyboard wherever the focus is, which is
            // what a person pressing Page Down at a page expects; the dialog manager would
            // otherwise spend the arrows moving between buttons.
            if message.message == WM_KEYDOWN && turn_page(popup, message.wParam.0 as u16) {
                continue;
            }
            if IsDialogMessageW(popup, &message).as_bool() {
                continue;
            }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }

        let answer = with_modal(popup, read);
        let _ = EnableWindow(owner, true);
        drop(smoke);
        let _ = SetActiveWindow(owner);
        let _ = DestroyWindow(popup);
        answer
    }
}

/// Ask the compositor for a modal's corner, border and title-bar mode — Windows 11's, and inert
/// where they are not supported.
fn chrome(popup: HWND, theme: Theme) {
    let dark = windows::core::BOOL::from(theme.mode == crate::theme::Mode::Dark);
    let corner = DWMWCP_ROUND;
    let border = COLORREF(theme.stroke.colorref());
    // SAFETY: every buffer is a live local of the size given.
    unsafe {
        let _ = DwmSetWindowAttribute(
            popup,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            std::ptr::from_ref(&dark).cast(),
            std::mem::size_of_val(&dark) as u32,
        );
        let _ = DwmSetWindowAttribute(
            popup,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            std::ptr::from_ref(&corner).cast(),
            std::mem::size_of_val(&corner) as u32,
        );
        let _ = DwmSetWindowAttribute(
            popup,
            DWMWA_BORDER_COLOR,
            std::ptr::from_ref(&border).cast(),
            std::mem::size_of_val(&border) as u32,
        );
    }
}

/// The controls a modal holds: its buttons along the footer, and the edit or list in its body.
fn create_children(popup: HWND, natural: &[f64], titled: bool) {
    // SAFETY: every child is created on the popup and lives as long as it; the borrow of the
    // state is released around every call that sends a message to the popup.
    unsafe {
        let Ok(instance) = GetModuleHandleW(None) else {
            return;
        };
        let mut client = RECT::default();
        let _ = GetClientRect(popup, &mut client);
        let Some((dpi, specs, body_kind)) = with_modal(popup, |modal| {
            let specs: Vec<(f64, End)> = modal
                .buttons
                .iter()
                .zip(natural)
                .map(|(button, w)| (*w, button.role.end()))
                .collect();
            let kind = match &modal.body {
                Body::Field { .. } => 1,
                Body::Reader { .. } => 2,
                Body::List { is_multi, .. } => 3 + u8::from(*is_multi),
                _ => 0,
            };
            (modal.dpi, specs, kind)
        }) else {
            return;
        };
        let frame = modal::frame(
            f64::from(client.right),
            f64::from(client.bottom),
            dpi,
            titled,
        );
        let placed = modal::buttons(frame.footer, dpi, &specs);
        let px = |value: f64| scale(value, dpi).round() as i32;
        let make = |class: &str, text: &str, style: WINDOW_STYLE, id: usize, rect: RECT| -> HWND {
            let class = gdi::wide(class);
            let text = gdi::wide(text);
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                PCWSTR(class.as_ptr()),
                PCWSTR(text.as_ptr()),
                WS_CHILD | WS_VISIBLE | style,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                Some(popup),
                Some(HMENU(id as *mut std::ffi::c_void)),
                Some(instance.into()),
                None,
            )
            .unwrap_or_default()
        };
        let font = with_modal(popup, |modal| modal.body_font.handle()).unwrap_or_default();
        let set_font = |control: HWND| {
            SendMessageW(
                control,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(0)),
            );
        };
        let dark = theme().mode == crate::theme::Mode::Dark;

        // The body's control first, so Tab reaches it before the buttons.
        let body = frame.body;
        let label_h = f64::from(px(20.0) + px(modal::size::LABEL_GAP));
        let (control, well) = match body_kind {
            1 | 2 => {
                let reading = body_kind == 2;
                let well = Rect {
                    x: body.x,
                    y: body.y + label_h,
                    w: body.w,
                    h: match reading {
                        true => (body.h - label_h).max(0.0),
                        false => scale(modal::size::CONTROL_H, dpi),
                    },
                };
                // Inside the frame: a single line is centred on it, the reader fills it less a
                // margin. The EDIT has no border of its own — the frame is drawn.
                let line = px(crate::theme::text::BODY * 1.4);
                let inner = match reading {
                    true => RECT {
                        left: well.x as i32 + px(12.0),
                        top: well.y as i32 + px(8.0),
                        right: (well.x + well.w) as i32 - px(4.0),
                        bottom: (well.y + well.h) as i32 - px(8.0),
                    },
                    false => {
                        let top = well.y as i32 + (well.h as i32 - line) / 2;
                        RECT {
                            left: well.x as i32 + px(11.0),
                            top,
                            right: (well.x + well.w) as i32 - px(11.0),
                            bottom: top + line,
                        }
                    }
                };
                let style = match reading {
                    true => {
                        WINDOW_STYLE((ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL) as u32)
                            | WS_VSCROLL
                            | WS_TABSTOP
                    }
                    false => WINDOW_STYLE(ES_AUTOHSCROLL as u32) | WS_TABSTOP,
                };
                let edit = make("EDIT", "", style, ID_EDIT, inner);
                set_font(edit);
                if dark {
                    let _ = SetWindowTheme(edit, w!("DarkMode_Explorer"), PCWSTR::null());
                }
                let text = with_modal(popup, |modal| match &modal.body {
                    Body::Field { initial, .. } => initial.clone(),
                    Body::Reader { text, .. } => text.clone(),
                    _ => String::new(),
                })
                .unwrap_or_default();
                // A multi-line edit holds 32K characters until told otherwise, and the
                // licences are longer than that; zero is "as much as the control can".
                SendMessageW(edit, EM_SETLIMITTEXT, Some(WPARAM(0)), Some(LPARAM(0)));
                let wide = gdi::wide(&text);
                let _ = SetWindowTextW(edit, PCWSTR(wide.as_ptr()));
                if !reading {
                    SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
                }
                (edit, rect_of(well))
            }
            3 | 4 => {
                let multi = body_kind == 4;
                let style = WINDOW_STYLE(
                    (LBS_NOTIFY | LBS_OWNERDRAWFIXED | LBS_HASSTRINGS | LBS_NOINTEGRALHEIGHT)
                        as u32
                        | if multi { LBS_MULTIPLESEL as u32 } else { 0 },
                ) | WS_VSCROLL
                    | WS_TABSTOP;
                // Inside a one-pixel frame that is drawn, with the corners' radius inset so the
                // list's square edge never cuts across a rounded one.
                let well = rect_of(body);
                let inset = px(modal::size::BUTTON_GAP / 2.0).max(2);
                let list = make(
                    "LISTBOX",
                    "",
                    style,
                    ID_LIST,
                    RECT {
                        left: well.left + 1,
                        top: well.top + inset,
                        right: well.right - 1,
                        bottom: well.bottom - inset,
                    },
                );
                set_font(list);
                if dark {
                    let _ = SetWindowTheme(list, w!("DarkMode_Explorer"), PCWSTR::null());
                }
                let original =
                    SetWindowLongPtrW(list, GWLP_WNDPROC, list_proc as *const () as isize);
                LIST_PROC.store(original, Ordering::SeqCst);
                let rows = with_modal(popup, |modal| match &modal.body {
                    Body::List {
                        rows,
                        initial,
                        multi,
                        ..
                    } => (rows.clone(), *initial, multi.clone()),
                    _ => (Vec::new(), 0, Vec::new()),
                })
                .unwrap_or_default();
                for row in &rows.0 {
                    let row = gdi::wide(row);
                    SendMessageW(
                        list,
                        LB_ADDSTRING,
                        Some(WPARAM(0)),
                        Some(LPARAM(row.as_ptr() as isize)),
                    );
                }
                match multi {
                    true => {
                        for (i, ticked) in rows.2.iter().enumerate() {
                            if *ticked {
                                SendMessageW(
                                    list,
                                    LB_SETSEL,
                                    Some(WPARAM(1)),
                                    Some(LPARAM(i as isize)),
                                );
                            }
                        }
                    }
                    false => {
                        // Windows scrolls a listbox to the row a program selects, the same as
                        // it does for one a person clicks.
                        let initial = rows.1.min(rows.0.len().saturating_sub(1));
                        SendMessageW(list, LB_SETCURSEL, Some(WPARAM(initial)), Some(LPARAM(0)));
                    }
                }
                (list, well)
            }
            _ => (HWND::default(), RECT::default()),
        };

        let mut handles = Vec::new();
        let ids: Vec<(i32, String)> = with_modal(popup, |modal| {
            modal
                .buttons
                .iter()
                .map(|button| (button.id, button.label.clone()))
                .collect()
        })
        .unwrap_or_default();
        for ((id, label), rect) in ids.iter().zip(&placed) {
            let button = make(
                "BUTTON",
                label,
                WINDOW_STYLE(BS_OWNERDRAW as u32) | WS_TABSTOP,
                *id as usize,
                rect_of(*rect),
            );
            set_font(button);
            let original =
                SetWindowLongPtrW(button, GWLP_WNDPROC, button_proc as *const () as isize);
            BUTTON_PROC.store(original, Ordering::SeqCst);
            handles.push(button);
        }
        with_modal(popup, |modal| {
            modal.control = control;
            modal.well = well;
            for (button, hwnd) in modal.buttons.iter_mut().zip(&handles) {
                button.hwnd = *hwnd;
            }
        });
        enable_page_buttons(popup);
    }
}

/// Run `f` with the modal's state. The same arrangement `win.rs` uses, and the same rule: the
/// borrow lives for the call and nothing inside it dispatches a message.
unsafe fn with_modal<T>(hwnd: HWND, f: impl FnOnce(&mut Modal) -> T) -> Option<T> {
    // SAFETY: the slot holds either null or the pointer stored in `WM_NCCREATE`.
    let raw = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut Modal;
    if raw.is_null() {
        return None;
    }
    // SAFETY: exclusive for the duration of this call.
    Some(f(unsafe { &mut *raw }))
}

/// The keys the print preview answers wherever the focus is. `true` when one was taken.
fn turn_page(popup: HWND, key: u16) -> bool {
    let id = match key {
        k if k == VK_LEFT.0 || k == VK_PRIOR.0 => ID_PREVIOUS,
        k if k == VK_RIGHT.0 || k == VK_NEXT.0 => ID_NEXT,
        k if k == VK_HOME.0 => ID_PREVIOUS - 100,
        k if k == VK_END.0 => ID_NEXT - 100,
        _ => return false,
    };
    // SAFETY: one borrow, no dispatch; the repaint is queued after it.
    let turned = unsafe {
        with_modal(popup, |modal| match &mut modal.body {
            Body::Pages(pages) => {
                pages.page = match id {
                    ID_PREVIOUS => pages.page.saturating_sub(1),
                    ID_NEXT => (pages.page + 1).min(pages.count - 1),
                    i if i == ID_PREVIOUS - 100 => 0,
                    _ => pages.count - 1,
                };
                true
            }
            _ => false,
        })
    } == Some(true);
    if turned {
        enable_page_buttons(popup);
        // SAFETY: the borrow is released.
        unsafe {
            let _ = InvalidateRect(Some(popup), None, false);
        }
    }
    turned
}

/// *Previous* on the first page and *Next* on the last mean nothing, and say so.
fn enable_page_buttons(popup: HWND) {
    // SAFETY: one borrow, then calls with it released.
    unsafe {
        let Some(Some((page, count, previous, next))) = with_modal(popup, |modal| {
            let Body::Pages(pages) = &modal.body else {
                return None;
            };
            let find = |id| {
                modal
                    .buttons
                    .iter()
                    .find(|button| button.id == id)
                    .map_or(HWND::default(), |button| button.hwnd)
            };
            Some((pages.page, pages.count, find(ID_PREVIOUS), find(ID_NEXT)))
        }) else {
            return;
        };
        let focus = GetFocus();
        let _ = EnableWindow(previous, page > 0);
        let _ = EnableWindow(next, page + 1 < count);
        // A disabled button cannot keep the focus, and Windows will not move it on its own.
        if (focus == previous && page == 0) || (focus == next && page + 1 >= count) {
            let _ = SetFocus(Some(popup));
        }
    }
}

extern "system" fn modal_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_NCCREATE => {
            // SAFETY: `lparam` is this message's `CREATESTRUCTW`.
            unsafe {
                let create = &*(lparam.0 as *const CREATESTRUCTW);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        // The whole window is client: the caption and frame exist only for the compositor.
        // Both forms: the one `CreateWindowExW` sends first (`wparam` false) decides the client
        // rectangle the children are laid out in, and answering only the other left a caption's
        // height missing from the bottom of every modal.
        WM_NCCALCSIZE => LRESULT(0),
        WM_NCHITTEST => LRESULT(HTCLIENT as isize),
        // Without `-1` the default repaints a frame this window does not have.
        // SAFETY: the default handler, told not to paint.
        WM_NCACTIVATE => unsafe { DefWindowProcW(hwnd, message, wparam, LPARAM(-1)) },
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_MEASUREITEM => {
            // SAFETY: `lparam` is this message's `MEASUREITEMSTRUCT`.
            unsafe {
                let item = &mut *(lparam.0 as *mut MEASUREITEMSTRUCT);
                // Sent while the list is being created, which is after `WM_NCCREATE` stored the
                // state — so the DPI is already known here.
                let dpi = with_modal(hwnd, |modal| modal.dpi).unwrap_or(96);
                item.itemHeight = scale(modal::size::ROW_H, dpi).round() as u32;
            }
            LRESULT(1)
        }
        WM_DRAWITEM => {
            // SAFETY: `lparam` is this message's `DRAWITEMSTRUCT`, and its DC is live for it.
            unsafe {
                let item = &*(lparam.0 as *const DRAWITEMSTRUCT);
                with_modal(hwnd, |modal| match item.CtlType {
                    ODT_BUTTON => draw_button(modal, item),
                    ODT_LISTBOX => draw_row(modal, item),
                    _ => {}
                });
            }
            LRESULT(1)
        }
        // The field and the reader stand on the frame's ground; a read-only edit asks for a
        // *static*'s colours, which is the only reason that message is answered here.
        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC | WM_CTLCOLORLISTBOX => {
            // SAFETY: one borrow; only the message's own `HDC` is written.
            let answered = unsafe {
                with_modal(hwnd, |modal| {
                    let dc = HDC(wparam.0 as *mut std::ffi::c_void);
                    let (ground, brush) = match (message, modal.focused) {
                        (WM_CTLCOLORLISTBOX, _) => (modal.theme.background, &modal.active),
                        (WM_CTLCOLOREDIT, true) => (modal.theme.background, &modal.active),
                        _ => (modal.theme.card, &modal.rest),
                    };
                    SetBkMode(dc, TRANSPARENT);
                    SetBkColor(dc, COLORREF(ground.colorref()));
                    SetTextColor(dc, COLORREF(modal.theme.text.colorref()));
                    LRESULT(brush.handle().0 as isize)
                })
            };
            // SAFETY: the default answer, with the borrow released.
            answered.unwrap_or_else(|| unsafe { DefWindowProcW(hwnd, message, wparam, lparam) })
        }
        message if message == DM_GETDEFID => {
            // SAFETY: one borrow, no dispatch.
            let id = unsafe { with_modal(hwnd, |modal| modal.default_id()) }.unwrap_or(IDOK.0);
            LRESULT((DC_HASDEFID << 16) | id as isize)
        }
        WM_COMMAND => {
            command(
                hwnd,
                (wparam.0 & 0xffff) as i32,
                ((wparam.0 >> 16) & 0xffff) as u32,
            );
            LRESULT(0)
        }
        WM_CLOSE => {
            command(hwnd, IDCANCEL.0, 0);
            LRESULT(0)
        }
        WM_NCDESTROY => {
            // SAFETY: the pointer came from `Box::into_raw`; reconstituting it once frees it.
            unsafe {
                let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Modal;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                if !raw.is_null() {
                    drop(Box::from_raw(raw));
                }
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        // SAFETY: the default handler with the arguments it was given.
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

/// A button, a double click on a row, or the field gaining or losing the keyboard.
fn command(hwnd: HWND, id: i32, code: u32) {
    // SAFETY: one borrow per step, with repaints queued after it is released.
    unsafe {
        if id as usize == ID_EDIT && (code == EN_SETFOCUS || code == EN_KILLFOCUS) {
            let Some((well, edit)) = with_modal(hwnd, |modal| {
                modal.focused = code == EN_SETFOCUS;
                (modal.well, modal.control)
            }) else {
                return;
            };
            let _ = InvalidateRect(Some(hwnd), Some(&well), false);
            let _ = InvalidateRect(Some(edit), None, true);
            return;
        }
        if id as usize == ID_LIST {
            if code == LBN_DBLCLK {
                with_modal(hwnd, |modal| {
                    if let Body::List {
                        is_multi: false, ..
                    } = modal.body
                    {
                        modal.answer = Some(IDOK.0);
                        modal.finished = true;
                    }
                });
            }
            return;
        }
        let ends = with_modal(hwnd, |modal| {
            if !modal.ends(id) {
                return false;
            }
            // Enter on a modal whose only answer is Close means Close, not an `IDOK` nobody
            // offered.
            let id = match modal.buttons.iter().any(|button| button.id == id) || id == IDCANCEL.0 {
                true => id,
                false => modal.default_id(),
            };
            modal.answer = Some(id);
            modal.finished = true;
            true
        });
        if ends != Some(false) {
            return;
        }
        if id == IDOK.0 {
            // `IsDialogMessageW`'s Enter with no default button to send — treat it as the
            // default, which is what a person pressing Enter meant.
            let default = with_modal(hwnd, |modal| modal.default_id()).unwrap_or(IDCANCEL.0);
            if default != IDOK.0 {
                return command(hwnd, default, 0);
            }
        }
        let changed = with_modal(hwnd, |modal| match &mut modal.body {
            Body::Chart {
                pictures, shown, ..
            } if id >= ID_KIND => {
                let i = (id - ID_KIND) as usize;
                if i < pictures.len() {
                    *shown = i;
                }
                true
            }
            Body::Pages(pages) if id == ID_PREVIOUS || id == ID_NEXT => {
                pages.page = match id {
                    ID_PREVIOUS => pages.page.saturating_sub(1),
                    _ => (pages.page + 1).min(pages.count - 1),
                };
                true
            }
            _ => false,
        });
        if changed == Some(true) {
            enable_page_buttons(hwnd);
            let _ = InvalidateRect(Some(hwnd), None, false);
            // The kind buttons draw their own checked state.
            if let Some(buttons) = with_modal(hwnd, |modal| {
                modal
                    .buttons
                    .iter()
                    .map(|button| button.hwnd)
                    .collect::<Vec<_>>()
            }) {
                for button in buttons {
                    let _ = InvalidateRect(Some(button), None, false);
                }
            }
        }
    }
}

/// The modal's own surface: the content's ground, the footer's, the title, and whatever the body
/// draws rather than holds.
fn paint(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    // SAFETY: `BeginPaint`/`EndPaint` are paired, and the borrow does not dispatch.
    unsafe {
        let dc = BeginPaint(hwnd, &mut ps);
        let client = gdi::client_rect(hwnd);
        let (w, h) = (client.right, client.bottom);
        with_modal(hwnd, |modal| {
            let Some(buffer) = gdi::BackBuffer::new(dc, w, h) else {
                return;
            };
            let out = buffer.dc();
            let theme = modal.theme;
            let frame = modal::frame(
                f64::from(w),
                f64::from(h),
                modal.dpi,
                !modal.title.is_empty(),
            );
            // ContentDialog's two layers: the content on the lighter one, the buttons on the
            // window's own ground, with a hairline where they meet.
            buffer.clear(theme.background);
            let (_, footer_top, _, _) = frame.footer.edges();
            gdi::fill(out, 0, footer_top, w, h, theme.backdrop);
            gdi::fill(out, 0, footer_top, w, footer_top + 1, theme.divider);
            if let Some(title) = frame.title {
                let _font = Selected::font(out, &modal.title_font);
                gdi::line(out, &modal.title, rect_of(title), Across::Left, theme.text);
            }
            paint_body(modal, out, frame.body);
            if let Body::Pages(pages) = &modal.body {
                // *Page 2 of 5*, between the two ends of the footer.
                let specs: Vec<(f64, End)> = modal
                    .buttons
                    .iter()
                    .map(|button| (0.0, button.role.end()))
                    .collect();
                let placed: Vec<Rect> = modal
                    .buttons
                    .iter()
                    .map(|button| {
                        let mut r = RECT::default();
                        let _ = GetWindowRect(button.hwnd, &mut r);
                        let mut origin = windows::Win32::Foundation::POINT {
                            x: r.left,
                            y: r.top,
                        };
                        let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut origin);
                        Rect {
                            x: f64::from(origin.x),
                            y: f64::from(origin.y),
                            w: f64::from(r.right - r.left),
                            h: f64::from(r.bottom - r.top),
                        }
                    })
                    .collect();
                let (from, to) = modal::between(frame.footer, &placed, &specs, modal.dpi);
                let _font = Selected::font(out, &modal.body_font);
                gdi::line(
                    out,
                    &(pages.label)(pages.page),
                    rect_of(Rect {
                        x: from,
                        y: frame.footer.y,
                        w: to - from,
                        h: frame.footer.h,
                    }),
                    Across::Left,
                    theme.text_secondary,
                );
            }
            buffer.present(dc);
        });
        let _ = EndPaint(hwnd, &ps);
    }
}

fn paint_body(modal: &mut Modal, out: HDC, body: Rect) {
    let theme = modal.theme;
    let radius = modal.px(crate::theme::space::RADIUS);
    let label = |modal: &Modal, text: &str| {
        let _font = Selected::font(out, &modal.body_font);
        let (left, top, right, _) = body.edges();
        gdi::line(
            out,
            text,
            RECT {
                left,
                top,
                right,
                bottom: top + modal.px(20.0),
            },
            Across::Left,
            theme.text,
        );
    };
    match &mut modal.body {
        Body::Message(text) => {
            let text = text.clone();
            let _font = Selected::font(out, &modal.body_font);
            gdi::paragraph(out, &text, rect_of(body), theme.text, false);
        }
        Body::Field { label: text, .. } => {
            let text = text.clone();
            label(modal, &text);
            // Fluent's TextBox: a control's ground and stroke, a stronger line along the bottom,
            // and that line in the accent, two pixels thick, while it has the keyboard.
            let well = modal.well;
            let (ground, line, thick) = match modal.focused {
                true => (theme.background, theme.accent, modal.px(2.0).max(2)),
                false => (theme.card, theme.text_tertiary, 1),
            };
            gdi::round_rect(out, well, radius, ground, theme.stroke);
            gdi::fill(
                out,
                well.left + radius / 2,
                well.bottom - thick,
                well.right - radius / 2,
                well.bottom,
                line,
            );
        }
        Body::Reader { label: text, .. } => {
            let text = text.clone();
            label(modal, &text);
            gdi::round_rect(out, modal.well, radius, theme.card, theme.stroke);
        }
        Body::List { .. } => {
            gdi::round_rect(out, modal.well, radius, theme.background, theme.stroke);
        }
        Body::Chart {
            pictures,
            shown,
            dpi,
        } => {
            let (left, top, right, bottom) = body.edges();
            // SAFETY: the DC is this paint's; the origin is put back below.
            unsafe {
                let _ = SetViewportOrgEx(out, left, top, None);
            }
            let (_, chart, data) = &pictures[*shown];
            crate::sheet::chart::paint_in(
                out,
                theme,
                gdi::ui_face(),
                *dpi,
                ((right - left).max(1), (bottom - top).max(1)),
                chart,
                data,
            );
            // SAFETY: as above.
            unsafe {
                let _ = SetViewportOrgEx(out, 0, 0, None);
            }
        }
        Body::Pages(pages) => {
            // The page stands on the window's own ground, the way the text pane's does.
            let well = rect_of(body);
            gdi::round_rect(
                out,
                well,
                modal_surface_radius(modal.dpi),
                theme.backdrop,
                theme.backdrop,
            );
            let inset = scale(12.0, modal.dpi);
            let area = (
                (body.w - inset * 2.0).max(1.0).round() as i32,
                (body.h - inset * 2.0).max(1.0).round() as i32,
            );
            let (fit, (x, y, pw, ph)) = crate::text::paper::fit(pages.size, area, pages.dpi);
            let key = fit.to_bits();
            let fresh =
                !matches!(&pages.cache, Some((page, at, ..)) if *page == pages.page && *at == key);
            if fresh {
                pages.cache = (pages.render)(pages.page, fit).map(|(rw, rh, rgba)| {
                    (pages.page, key, rw, rh, crate::text::paper::bgra(&rgba))
                });
            }
            let dest = Rect {
                x: body.x + inset + f64::from(x),
                y: body.y + inset + f64::from(y),
                w: f64::from(pw),
                h: f64::from(ph),
            };
            let (l, t, r, b) = dest.edges();
            gdi::fill(out, l - 1, t - 1, r + 1, b + 1, theme.stroke);
            if let Some((_, _, rw, rh, pixels)) = &pages.cache {
                gdi::blit_image(out, dest, (*rw, *rh), pixels);
            }
        }
    }
}

fn modal_surface_radius(dpi: u32) -> i32 {
    scale(crate::theme::space::RADIUS_SURFACE, dpi).round() as i32
}

/// A footer button, Fluent's way: the default in the accent, the rest on a control's ground with
/// its stroke and a slightly darker bottom edge (the "elevation" border), each answering the
/// pointer, and the focus ring only when the keyboard put the focus there.
fn draw_button(modal: &Modal, item: &DRAWITEMSTRUCT) {
    let theme = modal.theme;
    let Some(button) = modal.buttons.iter().find(|b| b.hwnd == item.hwndItem) else {
        return;
    };
    let dc = item.hDC;
    let rect = item.rcItem;
    let pressed = item.itemState.0 & ODS_SELECTED.0 != 0;
    let disabled = item.itemState.0 & ODS_DISABLED.0 != 0;
    let focused = item.itemState.0 & ODS_FOCUS.0 != 0 && item.itemState.0 & ODS_NOFOCUSRECT.0 == 0;
    // SAFETY: reading a property of a live control.
    let hover = unsafe { !GetPropW(button.hwnd, &HSTRING::from(HOVER)).is_invalid() };
    let state = match (pressed, hover) {
        (true, _) => Interaction::Pressed,
        (false, true) => Interaction::Hover,
        _ => Interaction::Rest,
    };
    let radius = modal.px(crate::theme::space::RADIUS);
    // The ground behind the rounded corners is the footer's.
    gdi::fill(
        dc,
        rect.left,
        rect.top,
        rect.right,
        rect.bottom,
        theme.backdrop,
    );
    let checked = match (&modal.body, button.role) {
        (Body::Chart { shown, .. }, Role::Choice) => button.id - ID_KIND == *shown as i32,
        _ => false,
    };
    let (fill, border, ink) = match (button.role, disabled) {
        (_, true) => (theme.card, theme.stroke, theme.text_tertiary),
        (Role::Default, false) => {
            let fill = match state {
                Interaction::Rest => theme.accent,
                Interaction::Hover => theme.accent.blend(theme.background, 0.1),
                Interaction::Pressed => theme.accent.blend(theme.background, 0.2),
            };
            (fill, fill, theme.on_accent)
        }
        (Role::Choice, false) if checked => {
            let fill = crate::theme::control_fill(theme, state, true, true).unwrap_or(theme.card);
            (fill, theme.accent.blend(fill, 0.55), theme.accent)
        }
        _ => {
            let fill = crate::theme::control_fill(theme, state, false, true).unwrap_or(theme.card);
            let ink = match state {
                Interaction::Pressed => theme.text_secondary,
                _ => theme.text,
            };
            (fill, theme.stroke, ink)
        }
    };
    gdi::round_rect(dc, rect, radius, fill, border);
    // The elevation edge: a control at rest is lit from above.
    if !disabled && state != Interaction::Pressed && button.role != Role::Default {
        gdi::fill(
            dc,
            rect.left + radius,
            rect.bottom - 1,
            rect.right - radius,
            rect.bottom,
            theme.stroke.blend(theme.text, 0.12),
        );
    }
    {
        let _font = Selected::font(dc, &modal.body_font);
        gdi::line(dc, &button.label, rect, Across::Centre, ink);
    }
    if focused {
        focus_ring(dc, rect, radius, theme);
    }
}

/// Fluent's focus visual, drawn inside the control since a child's DC is clipped to it: the outer
/// stroke in the ink that reads on any ground, and an inner one in its opposite.
fn focus_ring(dc: HDC, rect: RECT, radius: i32, theme: Theme) {
    let outline = |inset: i32, colour: Rgb| {
        let r = RECT {
            left: rect.left + inset,
            top: rect.top + inset,
            right: rect.right - inset,
            bottom: rect.bottom - inset,
        };
        let pen = crate::gdi::Pen::solid(colour, 1);
        let _pen = Selected::pen(dc, &pen);
        // SAFETY: the DC is the caller's; a hollow brush keeps what is inside.
        unsafe {
            let hollow = GetStockObject(windows::Win32::Graphics::Gdi::NULL_BRUSH);
            let previous = windows::Win32::Graphics::Gdi::SelectObject(dc, hollow);
            let _ = windows::Win32::Graphics::Gdi::RoundRect(
                dc,
                r.left,
                r.top,
                r.right,
                r.bottom,
                (radius - inset).max(0) * 2,
                (radius - inset).max(0) * 2,
            );
            windows::Win32::Graphics::Gdi::SelectObject(dc, previous);
        }
    };
    outline(0, theme.focus);
    outline(1, theme.focus);
    outline(2, theme.focus_inner);
}

/// A list row: Fluent's ListView item — a rounded subtle fill under the pointer and under the
/// selection, a short accent pill at the leading edge of the selected one, and for a list of
/// choices a check box instead of the pill. A row with a tab in it is two columns, the second in
/// the secondary ink and against the trailing edge — the shortcut list's key, say.
fn draw_row(modal: &Modal, item: &DRAWITEMSTRUCT) {
    let theme = modal.theme;
    let dc = item.hDC;
    let rect = item.rcItem;
    gdi::fill(
        dc,
        rect.left,
        rect.top,
        rect.right,
        rect.bottom,
        theme.background,
    );
    let Body::List { rows, is_multi, .. } = &modal.body else {
        return;
    };
    let Some(row) = rows.get(item.itemID as usize) else {
        return;
    };
    let selected = item.itemState.0 & ODS_SELECTED.0 != 0;
    let focused = item.itemState.0 & ODS_FOCUS.0 != 0 && item.itemState.0 & ODS_NOFOCUSRECT.0 == 0;
    // SAFETY: reading a property of a live control.
    let hovered = unsafe { GetPropW(item.hwndItem, &HSTRING::from(HOVER)) }.0 as usize;
    let hovered = hovered == item.itemID as usize + 1;
    let px = |v: f64| modal.px(v);
    let radius = px(crate::theme::space::RADIUS);
    let inner = RECT {
        left: rect.left + px(4.0),
        top: rect.top + px(2.0),
        right: rect.right - px(4.0),
        bottom: rect.bottom - px(2.0),
    };
    let ground = match (selected && !is_multi, hovered) {
        (true, true) => Some(theme.subtle_pressed.blend(theme.subtle_hover, 0.5)),
        (true, false) => Some(theme.subtle_hover),
        (false, true) => Some(theme.background.blend(theme.subtle_hover, 0.6)),
        (false, false) => None,
    };
    if let Some(ground) = ground {
        gdi::round_rect(dc, inner, radius, ground, ground);
    }
    let mut text_left = inner.left + px(12.0);
    if *is_multi {
        // A check box, 20 pixels, Fluent's: the accent filled with a white tick when on, a
        // control's ground with a strong stroke when off.
        let size = px(20.0);
        let top = (rect.top + rect.bottom - size) / 2;
        let check = RECT {
            left: inner.left + px(8.0),
            top,
            right: inner.left + px(8.0) + size,
            bottom: top + size,
        };
        match selected {
            true => {
                gdi::round_rect(dc, check, radius, theme.accent, theme.accent);
                gdi::check_mark(dc, check, theme.on_accent, px(1.5).max(2));
            }
            false => gdi::round_rect(dc, check, radius, theme.card, theme.text_tertiary),
        }
        text_left = check.right + px(12.0);
    } else if selected {
        let pill_h = px(16.0);
        let top = (rect.top + rect.bottom - pill_h) / 2;
        gdi::round_rect(
            dc,
            RECT {
                left: inner.left,
                top,
                right: inner.left + px(3.0),
                bottom: top + pill_h,
            },
            px(1.5),
            theme.accent,
            theme.accent,
        );
    }
    let _font = Selected::font(dc, &modal.body_font);
    let text = RECT {
        left: text_left,
        top: rect.top,
        right: inner.right - px(12.0),
        bottom: rect.bottom,
    };
    match row.split_once('\t') {
        Some((left, right)) => {
            let key_w = gdi::text_width(dc, right);
            gdi::line(dc, right, text, Across::Right, theme.text_secondary);
            gdi::line(
                dc,
                left,
                RECT {
                    right: text.right - key_w - px(24.0),
                    ..text
                },
                Across::Left,
                theme.text,
            );
        }
        None => gdi::line(dc, row, text, Across::Left, theme.text),
    }
    if focused {
        focus_ring(dc, inner, radius, theme);
    }
}

/// The footer buttons' own procedure, wrapped round `BUTTON`'s: it tracks the pointer — a button
/// that does not respond to it is what most gives a custom-drawn window away — and tells the dialog
/// manager the button is a push button, which an owner-drawn one otherwise does not say, so that
/// Enter on a focused *Cancel* means Cancel.
extern "system" fn button_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let original = BUTTON_PROC.load(Ordering::SeqCst);
    // SAFETY: `original` is `BUTTON`'s own procedure, stored before this one was installed.
    let call = || unsafe {
        CallWindowProcW(
            std::mem::transmute::<isize, windows::Win32::UI::WindowsAndMessaging::WNDPROC>(
                original,
            ),
            hwnd,
            message,
            wparam,
            lparam,
        )
    };
    match message {
        WM_MOUSEMOVE => {
            // SAFETY: properties of this control, and a leave notification for it.
            unsafe {
                let key = HSTRING::from(HOVER);
                if GetPropW(hwnd, &key).is_invalid() {
                    let _ = SetPropW(
                        hwnd,
                        &key,
                        Some(HANDLE(std::ptr::without_provenance_mut(1))),
                    );
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    let _ = TrackMouseEvent(&mut track);
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
            }
            call()
        }
        WM_MOUSELEAVE => {
            // SAFETY: this control's own property.
            unsafe {
                let _ = RemovePropW(hwnd, &HSTRING::from(HOVER));
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            call()
        }
        WM_GETDLGCODE => {
            // SAFETY: asking the focus, which does not dispatch.
            let focused = unsafe { GetFocus() } == hwnd;
            let enabled = unsafe { IsWindowEnabled(hwnd) }.as_bool();
            LRESULT(
                (DLGC_BUTTON
                    | match focused && enabled {
                        true => DLGC_DEFPUSHBUTTON,
                        false => DLGC_UNDEFPUSHBUTTON,
                    }) as isize,
            )
        }
        WM_NCDESTROY => {
            // SAFETY: this control's own property.
            unsafe {
                let _ = RemovePropW(hwnd, &HSTRING::from(HOVER));
            }
            call()
        }
        _ => call(),
    }
}

/// The list's own procedure, wrapped round `LISTBOX`'s: the row under the pointer, so it can be
/// drawn as the one a click would choose.
extern "system" fn list_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let original = LIST_PROC.load(Ordering::SeqCst);
    // SAFETY: `original` is `LISTBOX`'s own procedure, stored before this one was installed.
    let call = |message: u32, wparam: WPARAM, lparam: LPARAM| unsafe {
        CallWindowProcW(
            std::mem::transmute::<isize, windows::Win32::UI::WindowsAndMessaging::WNDPROC>(
                original,
            ),
            hwnd,
            message,
            wparam,
            lparam,
        )
    };
    let key = HSTRING::from(HOVER);
    // SAFETY: properties and item rectangles of this control.
    let redraw = |row: usize| unsafe {
        if row == 0 {
            return;
        }
        let mut rect = RECT::default();
        SendMessageW(
            hwnd,
            LB_GETITEMRECT,
            Some(WPARAM(row - 1)),
            Some(LPARAM(std::ptr::from_mut(&mut rect) as isize)),
        );
        let _ = InvalidateRect(Some(hwnd), Some(&rect), false);
    };
    match message {
        WM_MOUSEMOVE => {
            let at = call(LB_ITEMFROMPOINT, WPARAM(0), lparam).0 as u32;
            // The high word says the point is past the last row.
            let row = match at >> 16 {
                0 => (at & 0xffff) as usize + 1,
                _ => 0,
            };
            // SAFETY: as above.
            unsafe {
                let before = GetPropW(hwnd, &key).0 as usize;
                if before != row {
                    match row {
                        0 => {
                            let _ = RemovePropW(hwnd, &key);
                        }
                        _ => {
                            let _ =
                                SetPropW(hwnd, &key, Some(HANDLE(row as *mut std::ffi::c_void)));
                        }
                    }
                    if before == 0 {
                        let mut track = TRACKMOUSEEVENT {
                            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags: TME_LEAVE,
                            hwndTrack: hwnd,
                            dwHoverTime: 0,
                        };
                        let _ = TrackMouseEvent(&mut track);
                    }
                    redraw(before);
                    redraw(row);
                }
            }
            call(message, wparam, lparam)
        }
        WM_MOUSELEAVE => {
            // SAFETY: as above.
            let before = unsafe { RemovePropW(hwnd, &key) }.map_or(0, |h| h.0 as usize);
            redraw(before);
            call(message, wparam, lparam)
        }
        WM_NCDESTROY => {
            // SAFETY: as above.
            unsafe {
                let _ = RemovePropW(hwnd, &key);
            }
            call(message, wparam, lparam)
        }
        _ => call(message, wparam, lparam),
    }
}

/// A control's text, as a Rust string.
fn window_text(hwnd: HWND) -> String {
    // SAFETY: the length is asked for first and the buffer sized from it, with room for the
    // terminator `GetWindowTextW` always writes.
    unsafe {
        let length = GetWindowTextLengthW(hwnd);
        if length <= 0 {
            return String::new();
        }
        let mut buffer = vec![0u16; length as usize + 1];
        let written = GetWindowTextW(hwnd, &mut buffer);
        String::from_utf16_lossy(&buffer[..written.max(0) as usize])
    }
}

// ---------------------------------------------------------------------------
// The modals themselves
// ---------------------------------------------------------------------------

/// Say something and wait to be dismissed — `MessageBoxW`'s job, in this shell's own surface so
/// that it follows the theme and reads like every other dialog here.
fn tell(owner: HWND, title: &str, text: &str) {
    run(
        Spec {
            owner,
            title,
            body: Body::Message(text.to_owned()),
            buttons: vec![(IDCANCEL.0, "Close", Role::Default)],
        },
        |_| (),
    );
}

pub fn error(owner: HWND, text: &str) {
    tell(owner, "Something went wrong", text);
}

/// The Help menu's About: what build this is.
///
/// `grind_core::build_info::describe` is the one place this fact is formatted, so this window's
/// About reads the same commit/tree/date every other shell's does.
pub fn about(owner: HWND) {
    let text = format!(
        "An ODF-native office suite.\n\n{}\n\nhttps://github.com/fwilhe2/grind\n\n\
         Free software under the GNU Affero General Public License, version 3 or later. \
         Help ▸ Third-Party Licences lists the components it is built from.",
        grind_core::build_info::describe("grind-win32", env!("CARGO_PKG_VERSION"))
    );
    tell(owner, "About Grind", &text);
}

/// Help ▸ Third-Party Licences: every component this program is built from and its licence,
/// the list every window's About shows (`grind_core::third_party`, doc/third-party.md), in a
/// read-only box that scrolls.
pub fn licences(owner: HWND) {
    // An `EDIT` breaks lines at CRLF and draws a bare LF as nothing at all.
    let text = grind_core::third_party::notices().replace('\n', "\r\n");
    run(
        Spec {
            owner,
            title: "Third-Party Licences",
            body: Body::Reader {
                label: "Grind is built from these components, each under its own licence:".into(),
                text,
            },
            buttons: vec![(IDCANCEL.0, "Close", Role::Default)],
        },
        |_| (),
    );
}

/// The three-button close question, in the words Windows' own applications use: the verbs on
/// the buttons rather than *Yes* and *No*, and **Save** the default, which is the safe answer.
pub fn confirm_close(owner: HWND, name: &str) -> Answer {
    let answer = run(
        Spec {
            owner,
            title: "Save changes?",
            body: Body::Message(format!(
                "Do you want to save the changes you made to {name}? \
                 Your changes will be lost if you don't save them."
            )),
            buttons: vec![
                (IDYES.0, "Save", Role::Default),
                (IDNO.0, "Don't Save", Role::Answer),
                (IDCANCEL.0, "Cancel", Role::Answer),
            ],
        },
        |modal| modal.answer,
    )
    .flatten();
    match answer {
        Some(id) if id == IDYES.0 => Answer::Save,
        Some(id) if id == IDNO.0 => Answer::Discard,
        _ => Answer::Cancel,
    }
}

/// A destructive question — *delete this sheet?* — with the verb on its button and **Cancel** the
/// default: a dialog whose default answer is the destructive one is a dialog that deletes things
/// when somebody presses Enter twice.
pub fn confirm(owner: HWND, title: &str, text: &str, verb: &str) -> bool {
    run(
        Spec {
            owner,
            title,
            body: Body::Message(text.to_owned()),
            buttons: vec![
                (IDOK.0, verb, Role::Answer),
                (IDCANCEL.0, "Cancel", Role::Default),
            ],
        },
        |modal| modal.answer,
    )
    .flatten()
        == Some(IDOK.0)
}

/// Ask for one line of text. `None` means the user cancelled or typed nothing.
///
/// `title` heads the dialog and `label` sits over the field — Fluent's TextBox *header* — so a
/// caller's question reads as a question rather than as a caption.
pub fn prompt(owner: HWND, title: &str, label: &str, initial: &str) -> Option<String> {
    run(
        Spec {
            owner,
            title,
            body: Body::Field {
                label: label.to_owned(),
                initial: initial.to_owned(),
            },
            buttons: vec![
                (IDOK.0, "OK", Role::Default),
                (IDCANCEL.0, "Cancel", Role::Answer),
            ],
        },
        |modal| (modal.answer == Some(IDOK.0)).then(|| window_text(modal.control)),
    )
    .flatten()
    .filter(|text| !text.trim().is_empty())
}

/// Ask the user to pick one of a list of lines. `None` means cancelled, or nothing to pick from.
///
/// This is `grind text`'s outline dialog (`App::outline`, `doc/windows-shell.md`'s W5b) and W6's
/// "Show Source" and "Check Document", built generically over strings rather than over `Heading`
/// or a `Diagnostic` so this module stays ignorant of the document types (R8's rule for the core
/// applies just as well to a shell file with no reason to know one). A double-click accepts the
/// same as OK, because a list a user has to click twice — once to select, once on a separate
/// button — is slower than the box it replaces. A row with a tab in it is drawn as two columns.
///
/// `initial` is the row already selected when the list opens — "the line the selection is on
/// marked" (D9's own wording), clamped rather than refused so a caller need not check its own
/// bound first.
pub fn choose(owner: HWND, title: &str, items: &[String], initial: usize) -> Option<usize> {
    if items.is_empty() {
        return None;
    }
    run(
        Spec {
            owner,
            title,
            body: Body::List {
                rows: items.to_vec(),
                initial,
                multi: Vec::new(),
                is_multi: false,
            },
            buttons: vec![
                (IDOK.0, "OK", Role::Default),
                (IDCANCEL.0, "Cancel", Role::Answer),
            ],
        },
        |modal| {
            if modal.answer != Some(IDOK.0) {
                return None;
            }
            // SAFETY: reading the list's selection does not dispatch.
            let selection =
                unsafe { SendMessageW(modal.control, LB_GETCURSEL, Some(WPARAM(0)), None) }.0;
            (selection >= 0).then_some(selection as usize)
        },
    )
    .flatten()
}

/// A list that is there to be read rather than chosen from — the keyboard shortcuts, a formula
/// explained. One button, *Close*.
pub fn show_list(owner: HWND, title: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    run(
        Spec {
            owner,
            title,
            body: Body::List {
                rows: items.to_vec(),
                initial: 0,
                multi: Vec::new(),
                is_multi: false,
            },
            buttons: vec![(IDCANCEL.0, "Close", Role::Default)],
        },
        |_| (),
    );
}

/// What a filter dropdown decided — [`choose_multi`]'s answer, `ui_sheet_gtk`'s own `Chosen`
/// mirrored: *Clear* is a distinct button from *OK* rather than "every row ticked", because
/// `grind_sheet::Filter::keep` naming every value is not the same document as no condition on
/// that field at all, even though the two hide the same rows.
pub enum FilterChoice {
    /// Keep exactly these values, in the order `items` were given.
    Keep(Vec<bool>),
    /// Drop this field's condition — every value shows again.
    Clear,
}

/// Ask which of a column's values stay visible — the autofilter dropdown, `App::set_filter`'s
/// `keep` on the field a header cell's button was clicked for. `checked` is one bool per
/// `items`, the field's current `keep` set (or every value, when the field carries no
/// condition yet); `None` means cancelled.
///
/// A list of check boxes over `LBS_MULTIPLESEL`: a click toggles a row, and so does Space.
pub fn choose_multi(
    owner: HWND,
    title: &str,
    items: &[String],
    checked: &[bool],
) -> Option<FilterChoice> {
    if items.is_empty() {
        return None;
    }
    run(
        Spec {
            owner,
            title,
            body: Body::List {
                rows: items.to_vec(),
                initial: 0,
                multi: checked.to_vec(),
                is_multi: true,
            },
            // *Clear* at the leading end: clearing the condition is a different kind of answer
            // from picking values, the same separation `ui_sheet_gtk`'s dropdown draws.
            buttons: vec![
                (ID_CLEAR, "Clear Filter", Role::Leading),
                (IDOK.0, "OK", Role::Default),
                (IDCANCEL.0, "Cancel", Role::Answer),
            ],
        },
        |modal| match modal.answer {
            Some(ID_CLEAR) => Some(FilterChoice::Clear),
            Some(id) if id == IDOK.0 => {
                // SAFETY: reading the list's state does not dispatch.
                let count = unsafe { SendMessageW(modal.control, LB_GETCOUNT, None, None) }
                    .0
                    .max(0);
                Some(FilterChoice::Keep(
                    (0..count as usize)
                        .map(|i| {
                            // SAFETY: as above.
                            unsafe { SendMessageW(modal.control, LB_GETSEL, Some(WPARAM(i)), None) }
                                .0
                                > 0
                        })
                        .collect(),
                ))
            }
            _ => None,
        },
    )
    .flatten()
}

/// Show the chart *Insert Chart* would make and ask whether to insert it, with the GNOME dialog's
/// three kind buttons — Bar, Line, Pie — beside Insert, the one `shown` names pressed: the kind
/// pressed for **Insert**, `None` for Cancel, Escape or closing it. `pictures` is the chart as
/// each kind. The picture is `sheet/chart.rs`'s own marks over `grind_sheet::chart_paint`, so
/// what is previewed is what the grid then draws. Nothing is written here; the caller inserts.
pub fn chart_preview(
    owner: HWND,
    pictures: Vec<(
        grind_sheet::ChartKind,
        grind_sheet::Chart,
        grind_sheet::ChartData,
    )>,
    shown: usize,
    dpi: u32,
) -> Option<grind_sheet::ChartKind> {
    if pictures.is_empty() {
        return None;
    }
    let names: Vec<&'static str> = pictures.iter().map(|(kind, _, _)| kind.name()).collect();
    let mut buttons: Vec<(i32, &str, Role)> = names
        .iter()
        .enumerate()
        .map(|(i, name)| (ID_KIND + i as i32, *name, Role::Choice))
        .collect();
    buttons.push((IDOK.0, "Insert", Role::Default));
    buttons.push((IDCANCEL.0, "Cancel", Role::Answer));
    let shown = shown.min(pictures.len() - 1);
    run(
        Spec {
            owner,
            title: "Insert Chart",
            body: Body::Chart {
                pictures,
                shown,
                dpi,
            },
            buttons,
        },
        |modal| match (&modal.body, modal.answer) {
            (
                Body::Chart {
                    pictures, shown, ..
                },
                Some(id),
            ) if id == IDOK.0 => Some(pictures[*shown].0),
            _ => None,
        },
    )
    .flatten()
}

/// Every page as it prints, one at a time, with Previous, Next, Print… and Close: `true` when
/// Print was chosen. `render(page, scale)` is `grind_print::raster` over the display list the PDF
/// is written from, so the preview is the paper; nothing here knows what a page holds. `label`
/// says which page is showing — *Page 2 of 5*.
///
/// The modal is as tall as the screen it opens on allows and as wide as the page is at that
/// height, so the page is as large as it can be and the window never runs off the screen.
pub fn page_preview(
    owner: HWND,
    count: usize,
    size: (f32, f32),
    render: Box<dyn Fn(usize, f32) -> Option<Rendered>>,
    label: Box<dyn Fn(usize) -> String>,
) -> bool {
    // SAFETY: a live window.
    let dpi = unsafe { GetDpiForWindow(owner) }.max(96);
    run(
        Spec {
            owner,
            title: "Print Preview",
            body: Body::Pages(Pages {
                count: count.max(1),
                page: 0,
                size,
                render,
                label,
                dpi,
                cache: None,
            }),
            buttons: vec![
                (ID_PREVIOUS, "Previous", Role::Leading),
                (ID_NEXT, "Next", Role::Leading),
                (IDOK.0, "Print…", Role::Default),
                (IDCANCEL.0, "Close", Role::Answer),
            ],
        },
        |modal| modal.answer == Some(IDOK.0),
    )
    .unwrap_or(false)
}
